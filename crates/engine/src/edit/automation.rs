//! Automation edits: automation clips added with their automation, an automation taken out with
//! every clip of it, and the points of one automation added, moved, reshaped and deleted. Each
//! checks everything before it changes anything and returns its exact inverse.

use std::collections::BTreeSet;
use std::mem;

use super::{Applied, Edit, EditError, Touched};
use crate::project::automation::check_points;
use crate::project::{
    Automation, AutomationId, AutomationPoint, AutomationTarget, Clip, ClipId, Curve, Project,
    TrackId, MAX_AUTOMATIONS, TENSION_RANGE, VALUE_RANGE,
};
use crate::time::Ticks;

/// What an automation edit touched: the automation, and the playlist when clips changed.
fn touched(id: AutomationId, playlist: bool) -> Touched {
    Touched {
        automation: BTreeSet::from([id]),
        playlist,
        ..Touched::default()
    }
}

fn automation_mut(project: &mut Project, id: AutomationId) -> Result<&mut Automation, EditError> {
    project
        .automation
        .list
        .get_mut(&id)
        .ok_or(EditError::UnknownAutomation(id.0))
}

#[expect(
    clippy::too_many_arguments,
    reason = "the fields of `Edit::AddAutomationClip`, one by one, as every edit's applier takes \
              its edit's fields"
)]
pub(super) fn add_clip(
    project: &mut Project,
    id: AutomationId,
    clip: ClipId,
    target: AutomationTarget,
    track: TrackId,
    start: Ticks,
    length: Ticks,
    points: Vec<AutomationPoint>,
) -> Result<Applied, EditError> {
    let automations = &project.automation;
    if id != automations.next_id || automations.list.contains_key(&id) {
        return Err(EditError::NotNextAutomationId {
            got: id.0,
            expected: automations.next_id.0,
        });
    }
    let next_automation =
        id.0.checked_add(1)
            .ok_or(EditError::AutomationIdsExhausted)?;
    if automations.list.len() >= MAX_AUTOMATIONS {
        return Err(EditError::TooManyAutomations);
    }
    if !project.has_target(&target) {
        return Err(EditError::UnknownTarget);
    }
    if automations.of_target(&target).is_some() {
        return Err(EditError::AlreadyAutomated);
    }
    check_points(id, &points)?;
    let playlist = &project.playlist;
    if clip != playlist.next_clip_id || playlist.clips.contains_key(&clip) {
        return Err(EditError::NotNextClipId {
            got: clip.0,
            expected: playlist.next_clip_id.0,
        });
    }
    let next_clip = clip.0.checked_add(1).ok_or(EditError::ClipIdsExhausted)?;
    let placed = Clip::of_automation(id, track, start, length);
    // The automation goes in first so the clip check finds it; a refused clip takes it out.
    project
        .automation
        .list
        .insert(id, Automation { target, points });
    if let Err(error) = project.check_clip(clip, &placed) {
        project.automation.list.remove(&id);
        return Err(error.into());
    }
    project.automation.next_id = AutomationId(next_automation);
    project.playlist.clips.insert(clip, placed);
    project.playlist.next_clip_id = ClipId(next_clip);
    Ok(Applied {
        inverse: Edit::UnaddAutomationClip {
            automation: id,
            clip,
        },
        touched: touched(id, true),
    })
}

pub(super) fn unadd_clip(
    project: &mut Project,
    id: AutomationId,
    clip: ClipId,
) -> Result<Applied, EditError> {
    let newest_automation = id.0.checked_add(1) == Some(project.automation.next_id.0);
    let newest_clip = clip.0.checked_add(1) == Some(project.playlist.next_clip_id.0);
    let shows_it = project
        .playlist
        .clips
        .get(&clip)
        .is_some_and(|placed| placed.automation() == Some(id));
    let others = project
        .playlist
        .clips
        .iter()
        .any(|(&other, placed)| other != clip && placed.automation() == Some(id));
    if !newest_automation || !newest_clip || !shows_it || others {
        return Err(EditError::NotNewestAutomationClip {
            automation: id.0,
            clip: clip.0,
        });
    }
    let (Some(automation), Some(placed)) = (
        project.automation.list.remove(&id),
        project.playlist.clips.remove(&clip),
    ) else {
        return Err(EditError::UnknownAutomation(id.0));
    };
    // Giving both IDs back is safe: they were the newest, and nothing refers to them any more.
    project.automation.next_id = id;
    project.playlist.next_clip_id = clip;
    let inverse = Edit::AddAutomationClip {
        automation: id,
        clip,
        target: automation.target,
        track: placed.track,
        start: placed.start,
        length: placed.length,
        points: automation.points,
    };
    Ok(Applied {
        inverse,
        touched: touched(id, true),
    })
}

pub(super) fn delete(project: &mut Project, id: AutomationId) -> Result<Applied, EditError> {
    let Some(data) = project.automation.list.remove(&id) else {
        return Err(EditError::UnknownAutomation(id.0));
    };
    let ids: Vec<ClipId> = project
        .playlist
        .clips
        .iter()
        .filter(|(_, clip)| clip.automation() == Some(id))
        .map(|(&clip, _)| clip)
        .collect();
    let clips = ids
        .into_iter()
        .filter_map(|clip| Some((clip, project.playlist.clips.remove(&clip)?)))
        .collect();
    let inverse = Edit::RestoreAutomationClip {
        automation: id,
        data,
        clips,
    };
    Ok(Applied {
        inverse,
        touched: touched(id, true),
    })
}

pub(super) fn restore(
    project: &mut Project,
    id: AutomationId,
    data: Automation,
    clips: Vec<(ClipId, Clip)>,
) -> Result<Applied, EditError> {
    let automations = &project.automation;
    if automations.list.contains_key(&id) {
        return Err(EditError::AutomationExists(id.0));
    }
    if id >= automations.next_id {
        return Err(crate::project::ValidationError::UnallocatedAutomationId {
            id: id.0,
            next: automations.next_id.0,
        }
        .into());
    }
    if automations.list.len() >= MAX_AUTOMATIONS {
        return Err(EditError::TooManyAutomations);
    }
    if automations.of_target(&data.target).is_some() {
        return Err(EditError::AlreadyAutomated);
    }
    check_points(id, &data.points)?;
    let mut seen = BTreeSet::new();
    for (clip, placed) in &clips {
        if !seen.insert(*clip) {
            return Err(EditError::ClipListedTwice(clip.0));
        }
        if project.playlist.clips.contains_key(clip) {
            return Err(EditError::ClipExists(clip.0));
        }
        if *clip >= project.playlist.next_clip_id || placed.automation() != Some(id) {
            return Err(EditError::NotNewestAutomationClip {
                automation: id.0,
                clip: clip.0,
            });
        }
    }
    project.automation.list.insert(id, data);
    if let Some(error) = clips
        .iter()
        .find_map(|(clip, placed)| project.check_clip(*clip, placed).err())
    {
        project.automation.list.remove(&id);
        return Err(error.into());
    }
    project.playlist.clips.extend(clips);
    Ok(Applied {
        inverse: Edit::DeleteAutomationClip { automation: id },
        touched: touched(id, true),
    })
}

/// A point's value or a curve's tension the rules allow.
fn check_point(point: &AutomationPoint) -> Result<(), EditError> {
    let tension_ok = match point.curve {
        Curve::Exp(tension) => TENSION_RANGE.contains(&tension),
        Curve::Linear | Curve::Hold | Curve::Smooth => true,
    };
    // `contains` is false for NaN and infinity.
    if point.tick.0 < 0 || !VALUE_RANGE.contains(&point.value) || !tension_ok {
        return Err(EditError::InvalidPoint);
    }
    Ok(())
}

pub(super) fn add_point(
    project: &mut Project,
    id: AutomationId,
    point: AutomationPoint,
) -> Result<Applied, EditError> {
    check_point(&point)?;
    let automation = automation_mut(project, id)?;
    let at = match automation
        .points
        .binary_search_by_key(&point.tick, |existing| existing.tick)
    {
        Ok(_) => return Err(EditError::PointExists { tick: point.tick.0 }),
        Err(at) => at,
    };
    automation.points.insert(at, point);
    Ok(Applied {
        inverse: Edit::DeleteAutomationPoint {
            automation: id,
            tick: point.tick,
        },
        touched: touched(id, false),
    })
}

pub(super) fn move_point(
    project: &mut Project,
    id: AutomationId,
    from: Ticks,
    tick: Ticks,
    value: f32,
) -> Result<Applied, EditError> {
    let automation = automation_mut(project, id)?;
    let (index, &point) = automation
        .point_at(from)
        .ok_or(EditError::NoPointAt { tick: from.0 })?;
    let moved = AutomationPoint {
        tick,
        value,
        ..point
    };
    check_point(&moved)?;
    // Strictly between its neighbors, so the points keep their order and one per tick.
    let after_previous = index == 0 || automation.points[index - 1].tick < tick;
    let before_next = automation
        .points
        .get(index + 1)
        .is_none_or(|next| tick < next.tick);
    if !after_previous || !before_next {
        return Err(EditError::PointOutOfPlace { tick: tick.0 });
    }
    automation.points[index] = moved;
    let inverse = Edit::MoveAutomationPoint {
        automation: id,
        from_tick: tick,
        tick: from,
        value: point.value,
    };
    Ok(Applied {
        inverse,
        touched: touched(id, false),
    })
}

pub(super) fn set_curve(
    project: &mut Project,
    id: AutomationId,
    tick: Ticks,
    curve: Curve,
) -> Result<Applied, EditError> {
    let automation = automation_mut(project, id)?;
    let (index, &point) = automation
        .point_at(tick)
        .ok_or(EditError::NoPointAt { tick: tick.0 })?;
    check_point(&AutomationPoint { curve, ..point })?;
    let old = mem::replace(&mut automation.points[index].curve, curve);
    let inverse = Edit::SetSegmentCurve {
        automation: id,
        tick,
        curve: old,
    };
    Ok(Applied {
        inverse,
        touched: touched(id, false),
    })
}

pub(super) fn delete_point(
    project: &mut Project,
    id: AutomationId,
    tick: Ticks,
) -> Result<Applied, EditError> {
    let automation = automation_mut(project, id)?;
    let (index, _) = automation
        .point_at(tick)
        .ok_or(EditError::NoPointAt { tick: tick.0 })?;
    if automation.points.len() == 1 {
        return Err(EditError::LastPoint);
    }
    let point = automation.points.remove(index);
    Ok(Applied {
        inverse: Edit::AddAutomationPoint {
            automation: id,
            point,
        },
        touched: touched(id, false),
    })
}
