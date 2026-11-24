//! Playlist gestures as edits: pure functions from the playlist and a gesture's ticks and track
//! rows to the [`Edit`] that commits it, and to the targets a drag shows while it is held. No
//! iced types, so every rule is unit-tested directly, and a drag draws the very targets its
//! release commits.

use std::collections::BTreeSet;
use std::fmt;

use dawsome_engine::edit::Edit;
use dawsome_engine::project::{
    Clip, ClipId, ClipItem, ClipStretch, LoopRange, PatternId, Project, StretchMode, TrackId,
    MAX_AUDIO_CLIPS, STRETCH_PITCH_RANGE,
};
use dawsome_engine::time::{self, MeterMap, MeterUnit, Ticks};
use dawsome_engine::warp::{self, ClipWarp};

use crate::ui::note_gesture::{snap_down, snap_nearest};
use crate::ui::playlist_layout::LoopEdge;

/// What playlist positions snap to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ClipSnap {
    #[default]
    Bar,
    Beat,
    Step,
    /// Single ticks.
    None,
}

impl ClipSnap {
    /// Every choice, in the order the header shows them.
    pub const ALL: [ClipSnap; 4] = [
        ClipSnap::Bar,
        ClipSnap::Beat,
        ClipSnap::Step,
        ClipSnap::None,
    ];

    /// The lines this snaps to in the song's `meter`: its bars or its beats (each change's
    /// own: a quarter, an eighth or a sixteenth), its steps (sixteenths) or single ticks.
    pub fn grid(self, meter: &MeterMap) -> SnapGrid<'_> {
        match self {
            ClipSnap::Bar => SnapGrid::Meter(meter, MeterUnit::Bar),
            ClipSnap::Beat => SnapGrid::Meter(meter, MeterUnit::Beat),
            ClipSnap::Step => SnapGrid::Every(time::step_to_ticks(1, meter.ppq())),
            ClipSnap::None => SnapGrid::Every(Ticks(1)),
        }
    }
}

/// The lines a playlist position snaps to: every so many ticks (a step, a tick), or the bars or
/// beats of the song's meter map, whose length changes where the signature does.
#[derive(Debug, Clone, Copy)]
pub enum SnapGrid<'a> {
    Every(Ticks),
    Meter(&'a MeterMap, MeterUnit),
}

impl SnapGrid<'_> {
    /// The line at or before `tick`.
    pub fn down(self, tick: Ticks) -> Ticks {
        match self {
            SnapGrid::Every(unit) => snap_down(tick, unit),
            SnapGrid::Meter(meter, unit) => meter.snap_down(tick, unit),
        }
    }

    /// The line after the one at or before `tick`.
    pub fn next(self, tick: Ticks) -> Ticks {
        match self {
            SnapGrid::Every(unit) => Ticks(snap_down(tick, unit).0 + unit.0.max(1)),
            SnapGrid::Meter(meter, unit) => meter.snap_next(tick, unit),
        }
    }

    /// The nearer of the lines either side of `tick`, half way going on.
    pub fn nearest(self, tick: Ticks) -> Ticks {
        match self {
            SnapGrid::Every(unit) => snap_nearest(tick, unit),
            SnapGrid::Meter(meter, unit) => meter.snap_nearest(tick, unit),
        }
    }
}

impl fmt::Display for ClipSnap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ClipSnap::Bar => "Bar",
            ClipSnap::Beat => "Beat",
            ClipSnap::Step => "Step",
            ClipSnap::None => "None",
        })
    }
}

/// The track at `index` in track order.
pub fn track_at(project: &Project, index: usize) -> Option<TrackId> {
    project.playlist.tracks.keys().nth(index).copied()
}

/// Where `track` is in track order.
pub fn index_of(project: &Project, track: TrackId) -> Option<usize> {
    project.playlist.tracks.keys().position(|&id| id == track)
}

/// A press on empty lane space: `pattern` placed on the track at `index`, at `tick` snapped
/// down to the grid, as long as the pattern. A new clip of a 1-bar pattern is one bar long.
pub fn place(
    project: &Project,
    pattern: PatternId,
    index: usize,
    tick: Ticks,
    grid: SnapGrid,
) -> Option<Edit> {
    let track = track_at(project, index)?;
    project.patterns.get(&pattern)?;
    let start = grid.down(tick);
    if start.0 < 0 {
        return None;
    }
    let length = project.pattern_ticks(pattern);
    Some(Edit::PlaceClips {
        first: project.playlist.next_clip_id,
        clips: vec![Clip::of_pattern(pattern, track, start, length)],
    })
}

/// Where a drag of `ticks` and `rows` puts each of `ids`, as (clip, start, track index) in
/// `ids` order: the grid line the earliest clip's line moves to is the one nearest that line
/// plus the drag (a whole number of units on an even grid), and both offsets kept so no clip
/// lands before tick 0 or off the tracks. The group moves as one, so an off-grid clip stays as
/// far off the grid as it was.
pub fn move_targets(
    project: &Project,
    ids: &[ClipId],
    ticks: Ticks,
    rows: i64,
    grid: SnapGrid,
) -> Vec<(ClipId, Ticks, usize)> {
    let clips: Vec<(ClipId, &Clip, usize)> = ids
        .iter()
        .filter_map(|&id| {
            let clip = project.playlist.clips.get(&id)?;
            Some((id, clip, index_of(project, clip.track)?))
        })
        .collect();
    let (Some(earliest), Some(top), Some(bottom)) = (
        clips.iter().map(|(_, clip, _)| clip.start.0).min(),
        clips.iter().map(|&(_, _, index)| index).min(),
        clips.iter().map(|&(_, _, index)| index).max(),
    ) else {
        return Vec::new();
    };
    // The line the earliest clip starts on (or after, off the grid) moves to the line nearest
    // it plus the drag; the furthest left is that line at tick 0.
    let line = grid.down(Ticks(earliest)).0;
    let ticks = (grid.nearest(Ticks(line.saturating_add(ticks.0))).0 - line).max(-line);
    let last = project.playlist.tracks.len().saturating_sub(1);
    let rows = rows.clamp(-(top as i64), last.saturating_sub(bottom) as i64);
    clips
        .into_iter()
        .map(|(id, clip, index)| {
            // Clamped above to stay within 0..=last, so the row fits a usize.
            let index = (index as i64 + rows) as usize;
            (id, Ticks(clip.start.0 + ticks), index)
        })
        .collect()
}

/// The one edit that moves clips to `targets`, or `None` when none of them moves.
pub fn moved(project: &Project, targets: &[(ClipId, Ticks, usize)]) -> Option<Edit> {
    let moves: Vec<(ClipId, Ticks, TrackId)> = targets
        .iter()
        .map(|&(id, start, index)| Some((id, start, track_at(project, index)?)))
        .collect::<Option<_>>()?;
    let changed = moves.iter().any(|&(id, start, track)| {
        project
            .playlist
            .clips
            .get(&id)
            .is_some_and(|clip| (clip.start, clip.track) != (start, track))
    });
    changed.then_some(Edit::MoveClips { moves })
}

/// The length a right-edge drag of `ticks` gives `clip`: its end moved by the drag and floored
/// to the grid, and at least one grid line past the cell the clip starts in, so the end stays
/// on the grid even for a clip that starts off it (an on-grid clip stays a unit long). A press
/// without a drag keeps the length, even for an end off the grid.
pub fn resized_length(clip: &Clip, ticks: Ticks, grid: SnapGrid) -> Ticks {
    if ticks.0 == 0 {
        return clip.length;
    }
    let end = grid.down(Ticks(clip.end().0.saturating_add(ticks.0)));
    let shortest = grid.next(clip.start).0;
    Ticks(end.0.max(shortest) - clip.start.0)
}

/// The edit for a right-edge drag of `ticks` on clip `id`, or `None` when its length stays.
pub fn resized(project: &Project, id: ClipId, ticks: Ticks, grid: SnapGrid) -> Option<Edit> {
    let clip = project.playlist.clips.get(&id)?;
    let length = resized_length(clip, ticks, grid);
    (length != clip.length).then_some(Edit::ResizeClip { id, length })
}

/// Where an Alt drag of `ticks` on a clip's right edge puts its end: on the snap line nearest
/// the dragged end, at least a tick after the start.
pub fn stretched_end(clip: &Clip, ticks: Ticks, grid: SnapGrid) -> Ticks {
    let end = grid.nearest(Ticks(clip.end().0.saturating_add(ticks.0)));
    Ticks(end.0.max(clip.start.0 + 1))
}

/// Why a stretch gesture changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StretchRefused {
    /// The file would be stretched beyond a quarter or four times its length.
    Ratio,
    /// The stretched file would be shorter than the `shortest` frames a time-stretch can make.
    TooShort { shortest: u64 },
    /// The clip's warp markers, scaled to the new length, would break a rule (a part between
    /// them stretched past a quarter or four times, or two closer than the least spacing).
    Warp,
}

/// `value · num / den` rounded half up, in exact integers; `None` past `u64::MAX` or for a
/// `den` of 0.
fn scaled_half_up(value: u64, num: u64, den: u64) -> Option<u64> {
    let den = u128::from(den);
    if den == 0 {
        return None;
    }
    // Why u128 with checks: two u64 factors always fit, and only their doubling can overflow,
    // which is refused rather than wrapped.
    let twice = u128::from(value)
        .checked_mul(u128::from(num))?
        .checked_mul(2)?;
    u64::try_from((twice + den) / (den * 2)).ok()
}

/// The edit an Alt drag of audio clip `id`'s right edge to `end` commits, stretch to fit: the
/// clip ends there and the audio it plays now fills it. In samples by the one tick rule, with
/// T_old the stretched length, or the file's `source_frames` when the clip is not stretched yet
/// (which makes it a time-stretch at its own pitch): the clip plays
/// played = min(len_old, T_old − offset) frames of it (a clip longer than its audio plays
/// silence after it; one trimmed shorter shows only a part), and
/// T_new = round-half-up(T_old · len_new / played), the frame offset scaled the same way, so
/// those frames take len_new: the silence after the audio goes, a part trimmed to keeps its
/// window. `Ok(None)` when the length stays or the clip plays none of its audio. A ratio past a
/// quarter or four times the file, or a time-stretch shorter than `shortest` frames, is refused.
/// Why fit and not the clip's length scaled: a clip dragged longer first (its edge trims, so
/// that adds silence) kept that silence in every stretch after.
pub fn stretched(
    project: &Project,
    id: ClipId,
    end: Ticks,
    source_frames: u64,
    shortest: u64,
) -> Result<Option<Edit>, StretchRefused> {
    let Some(clip) = project.playlist.clips.get(&id) else {
        return Ok(None);
    };
    let ClipItem::Audio {
        frame_offset,
        stretch,
        ..
    } = clip.item
    else {
        return Ok(None);
    };
    let sample = |tick: Ticks| {
        time::tick_to_sample(
            Ticks(tick.0.max(0)),
            project.sample_rate,
            project.tempo,
            project.ppq,
        )
    };
    let lengths = sample(clip.start).and_then(|start| {
        let old = sample(clip.end())?.checked_sub(start)?;
        let new = sample(end)?.checked_sub(start)?;
        Some((old, new))
    });
    let Some((old, new)) = lengths else {
        return Ok(None);
    };
    if old == new || end <= clip.start {
        return Ok(None);
    }
    let from = stretch.map_or(source_frames, |stretch| stretch.frames);
    let played = old.min(from.saturating_sub(frame_offset));
    if played == 0 {
        return Ok(None);
    }
    let (Some(frames), Some(offset)) = (
        scaled_half_up(from, new, played),
        scaled_half_up(frame_offset, new, played),
    ) else {
        return Err(StretchRefused::Ratio);
    };
    let stretch = ClipStretch {
        mode: stretch.map_or(StretchMode::Stretch, |stretch| stretch.mode),
        frames,
        pitch: stretch.map_or(0, |stretch| stretch.pitch),
    };
    if frames == 0 || !stretch.fits(source_frames) {
        return Err(StretchRefused::Ratio);
    }
    if stretch.mode == StretchMode::Stretch && frames < shortest {
        return Err(StretchRefused::TooShort { shortest });
    }
    // The stretch edit scales the markers by new / old and refuses a list that then breaks a
    // rule; the gesture says so rather than leave the drag unexplained.
    if let Some(warp) = project.playlist.warps.get(&id) {
        let scaled = ClipWarp {
            markers: warp::scaled(&warp.markers, from, frames),
            ..warp.clone()
        };
        if scaled.problem(frames).is_some() {
            return Err(StretchRefused::Warp);
        }
    }
    Ok(Some(Edit::SetClipStretch {
        clip: id,
        stretch: Some(stretch),
        frame_offset: offset,
        length: Ticks(end.0 - clip.start.0),
    }))
}

/// The properties strip's Stretch choice: audio clip `id` played as its file (`None`),
/// resampled or time-stretched, its stretched length kept (a clip not stretched yet starts at
/// its file's `source_frames`). Ending a stretch puts the frame offset back in the file's
/// frames. Resampling has no pitch of its own. `Ok(None)` when the mode stays; a time-stretch
/// shorter than `shortest` frames is refused.
pub fn with_mode(
    project: &Project,
    id: ClipId,
    mode: Option<StretchMode>,
    source_frames: u64,
    shortest: u64,
) -> Result<Option<Edit>, StretchRefused> {
    let Some((clip, frame_offset, stretch)) = audio_clip(project, id) else {
        return Ok(None);
    };
    if stretch.map(|stretch| stretch.mode) == mode {
        return Ok(None);
    }
    let edit = |stretch, frame_offset| {
        Ok(Some(Edit::SetClipStretch {
            clip: id,
            stretch,
            frame_offset,
            length: clip.length,
        }))
    };
    let Some(mode) = mode else {
        let frames = stretch.map_or(source_frames, |stretch| stretch.frames);
        let offset =
            scaled_half_up(frame_offset, source_frames, frames).ok_or(StretchRefused::Ratio)?;
        return edit(None, offset);
    };
    let new = ClipStretch {
        mode,
        frames: stretch.map_or(source_frames, |stretch| stretch.frames),
        pitch: match mode {
            StretchMode::Stretch => stretch.map_or(0, |stretch| stretch.pitch),
            StretchMode::Resample => 0,
        },
    };
    if !new.fits(source_frames) {
        return Err(StretchRefused::Ratio);
    }
    if mode == StretchMode::Stretch && new.frames < shortest {
        return Err(StretchRefused::TooShort { shortest });
    }
    edit(Some(new), frame_offset)
}

/// The properties strip's pitch stepper: a time-stretched clip `semitones` higher, inside
/// [`STRETCH_PITCH_RANGE`]; `None` for a clip that is not time-stretched or a pitch that stays.
pub fn with_pitch(project: &Project, id: ClipId, semitones: i8) -> Option<Edit> {
    let (clip, frame_offset, stretch) = audio_clip(project, id)?;
    let stretch = stretch.filter(|stretch| stretch.mode == StretchMode::Stretch)?;
    let pitch = stretch
        .pitch
        .saturating_add(semitones)
        .clamp(*STRETCH_PITCH_RANGE.start(), *STRETCH_PITCH_RANGE.end());
    (pitch != stretch.pitch).then_some(Edit::SetClipStretch {
        clip: id,
        stretch: Some(ClipStretch { pitch, ..stretch }),
        frame_offset,
        length: clip.length,
    })
}

/// Audio clip `id`, its frame offset and its stretch.
fn audio_clip(project: &Project, id: ClipId) -> Option<(&Clip, u64, Option<ClipStretch>)> {
    let clip = project.playlist.clips.get(&id)?;
    match clip.item {
        ClipItem::Audio {
            frame_offset,
            stretch,
            ..
        } => Some((clip, frame_offset, stretch)),
        ClipItem::Pattern { .. } | ClipItem::Automation { .. } => None,
    }
}

/// Deletes `ids`, or `None` when there are none.
pub fn deleted(ids: &BTreeSet<ClipId>) -> Option<Edit> {
    (!ids.is_empty()).then(|| Edit::DeleteClips {
        ids: ids.iter().copied().collect(),
    })
}

/// Copies `ids` one selection length later, and the IDs the copies will have.
pub fn duplicated(project: &Project, ids: &BTreeSet<ClipId>) -> Option<(Edit, BTreeSet<ClipId>)> {
    if ids.is_empty() {
        return None;
    }
    let first = project.playlist.next_clip_id;
    let count = u32::try_from(ids.len()).ok()?;
    let copies = (first.0..first.0.checked_add(count)?).map(ClipId).collect();
    let edit = Edit::DuplicateClips {
        ids: ids.iter().copied().collect(),
        first,
    };
    Some((edit, copies))
}

/// Why a split changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitRefused {
    /// The song would hold more than [`MAX_AUDIO_CLIPS`] audio clips.
    TooManyAudioClips,
}

/// The edit Split commits on the clips `ids` at song tick `tick`, and the IDs of the parts it
/// adds. Each clip `tick` falls strictly inside is cut there: the left part keeps its start and
/// ends at `tick`; the right part is a new clip from `tick` to the old end that plays on from
/// where the left stops, its audio frame offset moved by s(tick) − s(start) (stretched frames on
/// a stretched clip), a pattern's or an automation's offset by the ticks. Why the offset is not
/// reduced by the pattern's length: the right part then plays what the whole clip did after
/// `tick` even once the pattern is made longer or shorter. One transaction, so one undo step: the
/// lefts resized, then the rights placed from `next_clip_id` in the order of the cut clips' IDs.
/// `Ok(None)` when `tick` falls inside none of them.
/// Why Crossfade did nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossfadeRefused {
    /// The selection is not two audio clips.
    NotTwoAudioClips,
    /// The two are on different tracks.
    OtherTracks,
    /// The later starts where the earlier has ended.
    NoOverlap,
    /// The later ends before the earlier does: the earlier's fade out, which counts back from
    /// its own end, would miss the overlap.
    Contained,
}

/// Crossfade on the clips `ids`: exactly two audio clips on one track that overlap. The earlier
/// (by start, then ID) fades out and the later fades in over the frames they share, s(the
/// earlier end of the two) − s(the later start), each fade edit clamped to its clip; one step.
/// `None` when both already fade so.
pub fn crossfade(
    project: &Project,
    ids: &BTreeSet<ClipId>,
) -> Result<Option<Edit>, CrossfadeRefused> {
    let clips: Vec<(ClipId, &Clip)> = ids
        .iter()
        .filter_map(|id| Some((*id, project.playlist.clips.get(id)?)))
        .filter(|(_, clip)| clip.audio_channel().is_some())
        .collect();
    let [mut earlier, mut later] = clips[..] else {
        return Err(CrossfadeRefused::NotTwoAudioClips);
    };
    if ids.len() != 2 {
        return Err(CrossfadeRefused::NotTwoAudioClips);
    }
    if (later.1.start, later.0) < (earlier.1.start, earlier.0) {
        std::mem::swap(&mut earlier, &mut later);
    }
    if earlier.1.track != later.1.track {
        return Err(CrossfadeRefused::OtherTracks);
    }
    let shared_end = earlier.1.end().min(later.1.end());
    if shared_end <= later.1.start {
        return Err(CrossfadeRefused::NoOverlap);
    }
    if later.1.end() < earlier.1.end() {
        return Err(CrossfadeRefused::Contained);
    }
    let sample =
        |at: Ticks| time::tick_to_sample(at, project.sample_rate, project.tempo, project.ppq);
    let overlap = sample(shared_end)
        .zip(sample(later.1.start))
        .and_then(|(end, start)| end.checked_sub(start))
        .ok_or(CrossfadeRefused::NoOverlap)?;
    let (
        ClipItem::Audio {
            fade_in: earlier_in,
            fade_out: earlier_out,
            ..
        },
        ClipItem::Audio {
            fade_in: later_in,
            fade_out: later_out,
            ..
        },
    ) = (earlier.1.item, later.1.item)
    else {
        return Err(CrossfadeRefused::NotTwoAudioClips);
    };
    // What the two fade edits leave, clamped as they clamp: when that is what the clips hold,
    // the command changes nothing and is no edit (no undo step).
    let frames = |clip: &Clip| project.clip_frames(clip).unwrap_or(0);
    let (earlier_frames, later_frames) = (frames(earlier.1), frames(later.1));
    let earlier_kept = earlier_in.min(earlier_frames);
    let earlier_new = (earlier_kept, overlap.min(earlier_frames - earlier_kept));
    let later_kept = overlap.min(later_frames);
    let later_new = (later_kept, later_out.min(later_frames - later_kept));
    if earlier_new == (earlier_in, earlier_out) && later_new == (later_in, later_out) {
        return Ok(None);
    }
    Ok(Some(Edit::Transaction(vec![
        Edit::SetClipFades {
            clip: earlier.0,
            fade_in: earlier_in,
            fade_out: overlap,
        },
        Edit::SetClipFades {
            clip: later.0,
            fade_in: overlap,
            fade_out: later_out,
        },
    ])))
}

pub fn split(
    project: &Project,
    ids: &BTreeSet<ClipId>,
    tick: Ticks,
) -> Result<Option<(Edit, BTreeSet<ClipId>)>, SplitRefused> {
    let sample =
        |at: Ticks| time::tick_to_sample(at, project.sample_rate, project.tempo, project.ppq);
    let mut edits = Vec::new();
    let mut rights = Vec::new();
    let mut marked = Vec::new();
    for &id in ids {
        let Some(clip) = project.playlist.clips.get(&id) else {
            continue;
        };
        if !(clip.start < tick && tick < clip.end()) {
            continue;
        }
        let mut right = Clip {
            start: tick,
            length: Ticks(clip.end().0 - tick.0),
            ..clip.clone()
        };
        match &mut right.item {
            ClipItem::Audio {
                frame_offset,
                fade_in,
                ..
            } => {
                let moved = sample(tick)
                    .zip(sample(clip.start))
                    .and_then(|(at, start)| at.checked_sub(start))
                    .and_then(|moved| frame_offset.checked_add(moved));
                let Some(moved) = moved else {
                    continue;
                };
                *frame_offset = moved;
                // The fade in stays on the left part's start, the fade out on the right part's
                // end: the cut itself is no fade.
                *fade_in = 0;
            }
            ClipItem::Pattern { .. } | ClipItem::Automation { .. } => {
                right.offset = Ticks(clip.offset.0.saturating_add(tick.0 - clip.start.0));
            }
        }
        edits.push(Edit::ResizeClip {
            id,
            length: Ticks(tick.0 - clip.start.0),
        });
        if let ClipItem::Audio {
            fade_in, fade_out, ..
        } = clip.item
        {
            // The stored fade in stays whole, even past the left part's end (the render's
            // min(1, k / N) holds it), so an undo and the generator's rule agree.
            if fade_out > 0 {
                edits.push(Edit::RestoreClipFades {
                    clip: id,
                    fade_in,
                    fade_out: 0,
                });
            }
        }
        rights.push(right);
        // Markers describe the whole sound's map and a clip is a window on it, so the right
        // part plays through the very list the clip had.
        marked.push(project.playlist.warps.get(&id).cloned());
    }
    if rights.is_empty() {
        return Ok(None);
    }
    let audio = |clips: &mut dyn Iterator<Item = &Clip>| {
        clips.filter(|clip| clip.audio_channel().is_some()).count()
    };
    let held = audio(&mut project.playlist.clips.values());
    if held + audio(&mut rights.iter()) > MAX_AUDIO_CLIPS {
        return Err(SplitRefused::TooManyAudioClips);
    }
    let first = project.playlist.next_clip_id;
    // IDs past u32's end are left out here; the placing edit refuses them.
    let new: BTreeSet<ClipId> = (0..rights.len())
        .filter_map(|k| first.0.checked_add(u32::try_from(k).ok()?))
        .map(ClipId)
        .collect();
    edits.push(Edit::PlaceClips {
        first,
        clips: rights,
    });
    let warps = new
        .iter()
        .zip(marked)
        .filter_map(|(&clip, warp)| Some((clip, warp?)));
    edits.extend(warps.map(|(clip, warp)| Edit::SetClipWarp {
        clip,
        warp: Some(warp),
    }));
    Ok(Some((Edit::Transaction(edits), new)))
}

/// The ticks a slip's drag of `dragged` moves the audio of a clip starting at `start`, snapped
/// as a move snaps: whole units of `grid` from the line the clip starts on. Past the song's start
/// (where a grid has no lines) a drag to the left is the mirror of the same drag to the right.
pub fn slip_ticks(start: Ticks, dragged: Ticks, grid: SnapGrid) -> Ticks {
    let line = grid.down(start).0;
    let to = line.saturating_add(dragged.0);
    if to >= 0 {
        Ticks(grid.nearest(Ticks(to)).0 - line)
    } else {
        let mirrored = line.saturating_sub(dragged.0);
        Ticks(line - grid.nearest(Ticks(mirrored)).0)
    }
}

/// The frame offset audio clip `clip` has after a slip of `ticks` (already snapped): the frames
/// F the ticks span from its start (s(start + d) − s(start), and the mirror for a drag to the
/// left, so a slip and its opposite cancel), taken off its offset, which stays from frame 0 to
/// the last frame: the stretched file's on a stretched clip, else the sound's of `sound_frames`
/// frames when known, else with no upper stop. Dragging right plays earlier audio, so the
/// picture follows the hand. `None` for a clip that is not audio.
pub fn slipped_offset(
    project: &Project,
    clip: &Clip,
    ticks: Ticks,
    sound_frames: Option<u64>,
) -> Option<u64> {
    let ClipItem::Audio {
        frame_offset,
        stretch,
        ..
    } = clip.item
    else {
        return None;
    };
    let sample =
        |at: Ticks| time::tick_to_sample(at, project.sample_rate, project.tempo, project.ppq);
    let reach = Ticks(clip.start.0.saturating_add(ticks.0.saturating_abs()));
    let frames = i128::from(sample(reach)?.checked_sub(sample(clip.start)?)?);
    let frames = if ticks.0 < 0 { -frames } else { frames };
    let last = match stretch {
        Some(stretch) => Some(stretch.frames.saturating_sub(1)),
        None => sound_frames.map(|frames| frames.saturating_sub(1)),
    };
    let offset = (i128::from(frame_offset) - frames).max(0);
    let offset = last.map_or(offset, |last| offset.min(i128::from(last)));
    u64::try_from(offset).ok()
}

/// The edit a slip of `ticks` on audio clip `id` commits: the clip's start and length kept, its
/// frame offset moved ([`slipped_offset`]). `None` when the offset stays or the clip is not audio.
pub fn slipped(
    project: &Project,
    id: ClipId,
    ticks: Ticks,
    sound_frames: Option<u64>,
) -> Option<Edit> {
    let clip = project.playlist.clips.get(&id)?;
    let (_, frame_offset, _) = audio_clip(project, id)?;
    let offset = slipped_offset(project, clip, ticks, sound_frames)?;
    (offset != frame_offset).then_some(Edit::TrimAudioClip {
        clip: id,
        start: clip.start,
        length: clip.length,
        frame_offset: offset,
    })
}

/// A loop range drawn from tick `press` to tick `cursor`: every grid cell from the one pressed
/// to the one under the cursor, switched on, and at least `step` long so a pass is never empty.
pub fn drawn_loop(press: Ticks, cursor: Ticks, grid: SnapGrid, step: Ticks) -> LoopRange {
    let start = grid.down(Ticks(press.0.min(cursor.0))).0.max(0);
    let end = grid.next(Ticks(press.0.max(cursor.0))).0;
    LoopRange {
        start: Ticks(start),
        end: Ticks(end.max(start + step.0)),
        on: true,
    }
}

/// `range` with `edge` moved to the grid line nearest tick `tick`, never closer than `step` to
/// the other edge nor before tick 0. The range stays on or off as it was.
pub fn moved_loop_edge(
    range: LoopRange,
    edge: LoopEdge,
    tick: Ticks,
    grid: SnapGrid,
    step: Ticks,
) -> LoopRange {
    let at = grid.nearest(tick).0.max(0);
    match edge {
        LoopEdge::Start => LoopRange {
            start: Ticks(at.min(range.end.0 - step.0).max(0)),
            ..range
        },
        LoopEdge::End => LoopRange {
            end: Ticks(at.max(range.start.0 + step.0)),
            ..range
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P4_TWO_CLIPS: &str = include_str!("../../../../tests/fixtures/p4_two_clips.json");
    const P4_MESSY: &str = include_str!("../../../../tests/fixtures/p4_messy.json");
    const P9_STRETCH_UI: &str = include_str!("../../../../tests/fixtures/p9_stretch_ui.json");
    const P39_WARP1: &str = include_str!("../../../../tests/fixtures/p39_warp1.json");

    /// p9_tone440.wav is one second at 44.1 kHz.
    const TONE: u64 = 44_100;
    /// Twice the stretcher's latency at 44.1 kHz, mono.
    const SHORTEST: u64 = 2646;

    // Stretch to fit, by hand at 120 BPM, 44.1 kHz: s(t) = floor(t · 229.6875), so a bar is
    // 88200 samples and the clips' four bars 352800; both clips are longer than their audio.
    // Clip 2 (unstretched: N = the file's 44100 frames, then silence) dragged to 288 (L = 66150
    // samples): ratio L/N = 1.5, T = 44100 · 66150 / 44100 = 66150, p9_stretch's own stretch
    // (whose render is gen_p9's 440 Hz tone over 66150 frames); to 768 (176400): T = 176400,
    // four times, the most. Clip 1 (a stretch to 66150, then silence) to 576 (132300): T =
    // 66150 · 132300 / 66150 = 132300. A clip trimmed to the file's first half (clip 2 at 96
    // ticks, 22050 samples) dragged to 192 (44100): that half fills it, T = 44100 · 44100 /
    // 22050 = 88200. From an offset of 11025 (33075 frames played) to 288: T = 44100 · 66150 /
    // 33075 = 88200, the offset 11025 · 66150 / 33075 = 22050, and 88200 − 22050 = 66150.
    #[test]
    fn an_alt_drag_stretches_the_clips_audio_to_fill_its_new_length() {
        let mut project = load(P9_STRETCH_UI);
        let stretch = |frames| {
            Some(ClipStretch {
                mode: StretchMode::Stretch,
                frames,
                pitch: 0,
            })
        };
        let set = |clip, frames, frame_offset, length| {
            Ok(Some(Edit::SetClipStretch {
                clip: ClipId(clip),
                stretch: stretch(frames),
                frame_offset,
                length: Ticks(length),
            }))
        };
        let p9: serde_json::Value =
            serde_json::from_str(include_str!("../../../../tests/fixtures/p9_expected.json"))
                .expect("p9_expected.json parses");
        assert_eq!(p9["stretch"]["stretched"], 66_150);
        assert_eq!(
            stretched(&project, ClipId(2), Ticks(288), TONE, SHORTEST),
            set(2, 66_150, 0, 288)
        );
        assert_eq!(
            stretched(&project, ClipId(2), Ticks(768), TONE, SHORTEST),
            set(2, 176_400, 0, 768)
        );
        assert_eq!(
            stretched(&project, ClipId(1), Ticks(576), TONE, SHORTEST),
            set(1, 132_300, 0, 576)
        );
        assert_eq!(
            stretched(&project, ClipId(2), Ticks(1536), TONE, SHORTEST),
            Ok(None),
            "the same length"
        );
        let clip = project.playlist.clips.get_mut(&ClipId(2)).expect("clip 2");
        clip.length = Ticks(96);
        assert_eq!(
            stretched(&project, ClipId(2), Ticks(192), TONE, SHORTEST),
            set(2, 88_200, 0, 192),
            "a part trimmed to keeps its window"
        );
        let clip = project.playlist.clips.get_mut(&ClipId(2)).expect("clip 2");
        clip.length = Ticks(1536);
        if let ClipItem::Audio { frame_offset, .. } = &mut clip.item {
            *frame_offset = 11_025;
        }
        assert_eq!(
            stretched(&project, ClipId(2), Ticks(288), TONE, SHORTEST),
            set(2, 88_200, 22_050, 288)
        );
        if let Some(ClipItem::Audio { frame_offset, .. }) = project
            .playlist
            .clips
            .get_mut(&ClipId(2))
            .map(|clip| &mut clip.item)
        {
            *frame_offset = 44_100;
        }
        assert_eq!(
            stretched(&project, ClipId(2), Ticks(288), TONE, SHORTEST),
            Ok(None),
            "it plays none of its audio"
        );
    }

    // gen_p39's "last_segment_ratio": p39_warp1 (S 24000, T 36000, marker (12000, 24000), the
    // clip 240..384 at 250 frames a tick) Alt-dragged to end at 269, 29 ticks: T 7250, a uniform
    // ratio of 0.30 that fits, but the marker scales to (12000, 4833) and its last part, 12000
    // frames of sound made 2417, is under a quarter. Refused, with nothing changed; to 300 (60
    // ticks, T 15000, the marker (12000, 10000), the last part 12000 made 5000) it stretches.
    #[test]
    fn p39_an_alt_drag_that_breaks_the_warp_markers_is_refused() {
        let project = load(P39_WARP1);
        assert_eq!(
            stretched(&project, ClipId(1), Ticks(269), 24_000, SHORTEST),
            Err(StretchRefused::Warp)
        );
        let Ok(Some(Edit::SetClipStretch { stretch, .. })) =
            stretched(&project, ClipId(1), Ticks(300), 24_000, SHORTEST)
        else {
            panic!("a stretch");
        };
        assert_eq!(stretch.map(|stretch| stretch.frames), Some(15_000));
    }

    // Both lengths scale half up: from an offset of 4100 (40000 frames played) to a bar (88200):
    // T = 44100 · 88200 / 40000 = 97240.5 → 97241 and the offset 4100 · 88200 / 40000 = 9040.5
    // → 9041. Past four times the file (to 20 bars: 1764000, T = 1944810 of 44100) is refused,
    // as is a time-stretch below the shortest (a 4000-frame file to 11 ticks, 2526 samples:
    // T = 2526 < 2646). Clip 1 to 3072 (705600 samples) would be sixteen times its file:
    // refused. An end lands on the nearest bar line.
    // The strip's choices by hand: clip 1 (a time-stretch to 66150) resampled keeps its
    // length and loses its pitch; its stretch ended puts an offset of 1500 stretched frames
    // back at 1500 · 44100 / 66150 = 1000 file frames; clip 2 time-stretched starts at its
    // file's 44100. A 2000-frame file is too short for the stretcher (2646).
    #[test]
    fn the_strip_switches_modes_keeping_the_stretched_length() {
        let mut project = load(P9_STRETCH_UI);
        let clip_1 = project.playlist.clips.get_mut(&ClipId(1)).expect("clip 1");
        if let ClipItem::Audio {
            frame_offset,
            stretch: Some(stretch),
            ..
        } = &mut clip_1.item
        {
            *frame_offset = 1500;
            stretch.pitch = 7;
        }
        let set = |clip, stretch, frame_offset| {
            Ok(Some(Edit::SetClipStretch {
                clip: ClipId(clip),
                stretch,
                frame_offset,
                length: Ticks(1536),
            }))
        };
        let of = |mode, frames, pitch| {
            Some(ClipStretch {
                mode,
                frames,
                pitch,
            })
        };
        let mode =
            |project: &Project, clip, mode| with_mode(project, ClipId(clip), mode, TONE, SHORTEST);
        assert_eq!(
            mode(&project, 1, Some(StretchMode::Resample)),
            set(1, of(StretchMode::Resample, 66_150, 0), 1500)
        );
        assert_eq!(mode(&project, 1, None), set(1, None, 1000));
        assert_eq!(mode(&project, 1, Some(StretchMode::Stretch)), Ok(None));
        assert_eq!(
            mode(&project, 2, Some(StretchMode::Stretch)),
            set(2, of(StretchMode::Stretch, 44_100, 0), 0)
        );
        assert_eq!(mode(&project, 2, None), Ok(None));
        assert_eq!(
            with_mode(
                &project,
                ClipId(2),
                Some(StretchMode::Stretch),
                2000,
                SHORTEST
            ),
            Err(StretchRefused::TooShort { shortest: SHORTEST })
        );
    }

    // Time-stretched clips step their pitch inside ±24 semitones; other clips have none.
    #[test]
    fn the_pitch_steps_only_a_time_stretch_and_stays_in_range() {
        let mut project = load(P9_STRETCH_UI);
        let Some(Edit::SetClipStretch {
            stretch: Some(stretch),
            ..
        }) = with_pitch(&project, ClipId(1), 1)
        else {
            panic!("a pitch step");
        };
        assert_eq!(stretch.pitch, 1);
        assert_eq!(with_pitch(&project, ClipId(2), 1), None, "not stretched");
        if let Some(clip) = project.playlist.clips.get_mut(&ClipId(1)) {
            if let ClipItem::Audio {
                stretch: Some(stretch),
                ..
            } = &mut clip.item
            {
                stretch.pitch = 24;
            }
        }
        assert_eq!(with_pitch(&project, ClipId(1), 1), None, "at the top");
        let Some(Edit::SetClipStretch {
            stretch: Some(stretch),
            ..
        }) = with_pitch(&project, ClipId(1), -1)
        else {
            panic!("a pitch step");
        };
        assert_eq!(stretch.pitch, 23);
    }

    #[test]
    fn a_stretch_rounds_half_up_and_refuses_what_it_cannot_make() {
        let mut project = load(P9_STRETCH_UI);
        let clip = project.playlist.clips.get_mut(&ClipId(2)).expect("clip 2");
        if let ClipItem::Audio { frame_offset, .. } = &mut clip.item {
            *frame_offset = 4100;
        }
        let Ok(Some(Edit::SetClipStretch {
            stretch: Some(stretch),
            frame_offset,
            ..
        })) = stretched(&project, ClipId(2), Ticks(384), TONE, SHORTEST)
        else {
            panic!("a stretch");
        };
        assert_eq!((stretch.frames, frame_offset), (97_241, 9041));
        assert_eq!(
            stretched(&project, ClipId(2), Ticks(7680), TONE, SHORTEST),
            Err(StretchRefused::Ratio)
        );
        assert_eq!(
            stretched(&project, ClipId(1), Ticks(3072), TONE, SHORTEST),
            Err(StretchRefused::Ratio)
        );
        let clip = project.playlist.clips.get_mut(&ClipId(2)).expect("clip 2");
        if let ClipItem::Audio { frame_offset, .. } = &mut clip.item {
            *frame_offset = 0;
        }
        assert_eq!(
            stretched(&project, ClipId(2), Ticks(11), 4000, SHORTEST),
            Err(StretchRefused::TooShort { shortest: SHORTEST })
        );
        let clip = project.playlist.clips[&ClipId(2)].clone();
        assert_eq!(
            stretched_end(&clip, Ticks(-2000), BAR),
            Ticks(1),
            "a tick at least"
        );
        assert_eq!(stretched_end(&clip, Ticks(200), BAR), Ticks(1920));
        assert_eq!(stretched_end(&clip, Ticks(150), BAR), Ticks(1536));
    }

    fn load(text: &str) -> Project {
        Project::load_json(text).expect("fixture loads")
    }

    /// A fixture's text, as the generator wrote it.
    fn fixture(name: &str) -> String {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/");
        std::fs::read_to_string(format!("{path}{name}")).expect("a p37 fixture")
    }

    fn p37() -> serde_json::Value {
        serde_json::from_str(&fixture("p37_expected.json")).expect("p37_expected.json parses")
    }

    fn ids_of(value: &serde_json::Value) -> BTreeSet<ClipId> {
        let ids = value.as_array().expect("ids");
        ids.iter()
            .map(|id| ClipId(id.as_u64().expect("an id") as u32))
            .collect()
    }

    // gen_p37's four splits: each is one transaction whose document is the generator's (the
    // lefts shortened, the rights from next_clip_id in the order of the cut clips' ids, their
    // offsets by s(t) - s(start) or in ticks) and whose undo gives back the file's bytes.
    #[test]
    fn p37_a_split_is_one_step_to_the_generators_document_and_back() {
        let want = p37();
        for (name, case) in want["cuts"].as_object().expect("cuts") {
            let before = fixture(case["fixture"].as_str().expect("fixture"));
            let mut project = load(&before);
            let tick = Ticks(case["tick"].as_i64().expect("tick"));
            let Ok(Some((change, new))) = split(&project, &ids_of(&case["ids"]), tick) else {
                panic!("{name}: a split");
            };
            assert!(matches!(change, Edit::Transaction(_)), "{name}: one step");
            assert_eq!(new, ids_of(&case["new"]), "{name}");
            let applied = dawsome_engine::edit::apply(&mut project, change).expect("applies");
            let after = project.to_json().expect("saves");
            assert_eq!(after, case["after"].as_str().expect("after"), "{name}");
            assert_eq!(
                after + "\n",
                fixture(case["cut"].as_str().expect("cut")),
                "{name}: the file the command line renders"
            );
            dawsome_engine::edit::apply(&mut project, applied.inverse).expect("undoes");
            assert_eq!(project.to_json().expect("saves") + "\n", before, "{name}");
        }
    }

    // gen_p38's faded clip split: the left part keeps its fade in and loses its fade out, the
    // right part the reverse (0 at the cut), in one step whose undo gives back the file's bytes.
    #[test]
    fn p38_a_split_keeps_each_fade_on_its_outer_edge() {
        let want: serde_json::Value =
            serde_json::from_str(&fixture("p38_expected.json")).expect("p38_expected.json parses");
        let case = &want["split"];
        let before = fixture(case["fixture"].as_str().expect("fixture"));
        let mut project = load(&before);
        let tick = Ticks(case["tick"].as_i64().expect("tick"));
        let ids = BTreeSet::from([ClipId(case["clip"].as_u64().expect("clip") as u32)]);
        let Ok(Some((change, _))) = split(&project, &ids, tick) else {
            panic!("a split");
        };
        assert!(matches!(change, Edit::Transaction(_)), "one step");
        let applied = dawsome_engine::edit::apply(&mut project, change).expect("applies");
        let after = project.to_json().expect("saves");
        assert_eq!(after, case["after"].as_str().expect("after"));
        dawsome_engine::edit::apply(&mut project, applied.inverse).expect("undoes");
        assert_eq!(project.to_json().expect("saves") + "\n", before);
    }

    // gen_p39's warped clip split at 300, without fades and with them: both parts hold the whole
    // marker list (the generator's p39_warp3_split and p39_warp3_faded_split, whose renders are
    // the uncut clip's bits); one step, undone to the file's bytes and redone to the split's.
    #[test]
    fn p39_a_split_warped_clip_gives_both_parts_the_whole_list_and_undoes_exactly() {
        for (whole, cut) in [
            ("p39_warp3.json", "p39_warp3_split.json"),
            ("p39_warp3_faded.json", "p39_warp3_faded_split.json"),
        ] {
            let before = fixture(whole);
            let mut project = load(&before);
            let Ok(Some((change, new))) = split(&project, &BTreeSet::from([ClipId(1)]), Ticks(300))
            else {
                panic!("{whole}: a split");
            };
            assert_eq!(new, BTreeSet::from([ClipId(2)]));
            let applied = dawsome_engine::edit::apply(&mut project, change).expect("applies");
            let after = project.to_json().expect("saves");
            assert_eq!(after + "\n", fixture(cut), "{whole}");
            let undone =
                dawsome_engine::edit::apply(&mut project, applied.inverse).expect("undoes");
            assert_eq!(
                project.to_json().expect("saves") + "\n",
                before,
                "{whole}: undone"
            );
            dawsome_engine::edit::apply(&mut project, undone.inverse).expect("redoes");
            assert_eq!(
                project.to_json().expect("saves") + "\n",
                fixture(cut),
                "{whole}: redone"
            );
        }
    }

    // gen_p38's splits inside a fade in (p38_split at 205, with a fade out; p38_fade_in at 305,
    // without): the left part keeps its stored fade in whole and no fade out, the right part no
    // fade in and the stored fade out, the generator's documents; one step, undone to the
    // file's bytes and redone to the split's.
    #[test]
    fn p38_a_split_inside_a_fade_keeps_the_stored_fades_and_undoes_exactly() {
        let want: serde_json::Value =
            serde_json::from_str(&fixture("p38_expected.json")).expect("p38_expected.json parses");
        for case in want["split_inside"].as_array().expect("split_inside") {
            let name = case["fixture"].as_str().expect("fixture");
            let before = fixture(name);
            let mut project = load(&before);
            let tick = Ticks(case["tick"].as_i64().expect("tick"));
            let ids = BTreeSet::from([ClipId(1)]);
            let Ok(Some((change, _))) = split(&project, &ids, tick) else {
                panic!("{name}: a split");
            };
            let applied = dawsome_engine::edit::apply(&mut project, change).expect("applies");
            let after = project.to_json().expect("saves");
            assert_eq!(after, case["after"].as_str().expect("after"), "{name}");
            let undone =
                dawsome_engine::edit::apply(&mut project, applied.inverse).expect("undoes");
            assert_eq!(
                project.to_json().expect("saves") + "\n",
                before,
                "{name}: undone"
            );
            dawsome_engine::edit::apply(&mut project, undone.inverse).expect("redoes");
            assert_eq!(project.to_json().expect("saves"), after, "{name}: redone");
        }
    }

    // Crossfade on p38_crossfade with clip 1 given a fade in nearly its whole length (25000 of
    // 26000 frames, so its fade out can only take the 1000 left of the 6000 overlap) and its
    // fades taken off clip 2: one step, undone to the bytes and redone; pressed again on the
    // pair it left, it changes nothing, so it is no edit at all.
    #[test]
    fn crossfade_undoes_to_the_bytes_and_again_is_no_edit() {
        let mut project = load(&fixture("p38_crossfade.json"));
        for (clip, fade_in) in [(ClipId(1), 25000), (ClipId(2), 0)] {
            let set = Edit::SetClipFades {
                clip,
                fade_in,
                fade_out: 0,
            };
            dawsome_engine::edit::apply(&mut project, set).expect("sets");
        }
        let ids = BTreeSet::from([ClipId(1), ClipId(2)]);
        let before = project.to_json().expect("saves");
        let Ok(Some(change)) = crossfade(&project, &ids) else {
            panic!("a crossfade");
        };
        let applied = dawsome_engine::edit::apply(&mut project, change).expect("applies");
        let after = project.to_json().expect("saves");
        assert_eq!(
            crossfade(&project, &ids),
            Ok(None),
            "again: nothing to change"
        );
        let undone = dawsome_engine::edit::apply(&mut project, applied.inverse).expect("undoes");
        assert_eq!(project.to_json().expect("saves"), before);
        dawsome_engine::edit::apply(&mut project, undone.inverse).expect("redoes");
        assert_eq!(project.to_json().expect("saves"), after);
    }

    // Crossfade on a clip that holds the other whole (clip 2 moved to 250..300, inside clip 1's
    // 200..304): the earlier clip's fade out would count back from its own end, past the
    // overlap, so the pair is refused.
    #[test]
    fn crossfade_refuses_a_clip_inside_the_other() {
        let mut project = load(&fixture("p38_crossfade.json"));
        let inside = Edit::TrimAudioClip {
            clip: ClipId(2),
            start: Ticks(250),
            length: Ticks(50),
            frame_offset: 30000,
        };
        dawsome_engine::edit::apply(&mut project, inside).expect("moves");
        let ids = BTreeSet::from([ClipId(1), ClipId(2)]);
        assert_eq!(crossfade(&project, &ids), Err(CrossfadeRefused::Contained));
    }

    // Crossfade's refusals: one clip, two on different tracks (p38_short_fade), two on one track
    // with a gap between (p38_seam_gap), a pattern clip with an audio clip (p37_cut_two).
    #[test]
    fn crossfade_needs_two_overlapping_audio_clips_on_one_track() {
        let ids = |list: &[u32]| list.iter().copied().map(ClipId).collect::<BTreeSet<_>>();
        let gap = load(&fixture("p38_seam_gap.json"));
        assert_eq!(
            crossfade(&gap, &ids(&[1])),
            Err(CrossfadeRefused::NotTwoAudioClips)
        );
        assert_eq!(
            crossfade(&gap, &ids(&[1, 2])),
            Err(CrossfadeRefused::NoOverlap)
        );
        let tracks = load(&fixture("p38_short_fade.json"));
        assert_eq!(
            crossfade(&tracks, &ids(&[1, 2])),
            Err(CrossfadeRefused::OtherTracks)
        );
        let mixed = load(&fixture("p37_cut_two.json"));
        assert_eq!(
            crossfade(&mixed, &ids(&[1, 2])),
            Err(CrossfadeRefused::NotTwoAudioClips)
        );
        let done = load(&fixture("p38_crossfade.json"));
        assert_eq!(crossfade(&done, &ids(&[1, 2])), Ok(None), "already so");
    }

    // p37_cut_two at 150: clip 3 (0..96) ends before it and clip 4 starts on it, so neither is
    // cut; at a clip's own start or end nothing is either.
    #[test]
    fn a_split_cuts_only_the_clips_the_tick_falls_inside() {
        let project = load(&fixture("p37_cut_two.json"));
        let ids = |list: &[u32]| list.iter().copied().map(ClipId).collect::<BTreeSet<_>>();
        assert_eq!(split(&project, &ids(&[3, 4]), Ticks(150)), Ok(None));
        assert_eq!(split(&project, &ids(&[2]), Ticks(100)), Ok(None));
        assert_eq!(split(&project, &ids(&[2]), Ticks(384)), Ok(None));
        assert_eq!(split(&project, &BTreeSet::new(), Ticks(150)), Ok(None));
    }

    // A song already holding 4096 audio clips refuses a split that makes another; a pattern
    // clip in it still cuts.
    #[test]
    fn a_split_past_the_audio_clip_limit_is_refused() {
        let mut project = load(&fixture("p37_cut_two.json"));
        let model = project.playlist.clips[&ClipId(4)].clone();
        let mut next = project.playlist.next_clip_id.0;
        let audio = |project: &Project| {
            let clips = project.playlist.clips.values();
            clips.filter(|clip| clip.audio_channel().is_some()).count()
        };
        while audio(&project) < MAX_AUDIO_CLIPS {
            project.playlist.clips.insert(ClipId(next), model.clone());
            next += 1;
        }
        project.playlist.next_clip_id = ClipId(next);
        let one = |id| BTreeSet::from([ClipId(id)]);
        assert_eq!(
            split(&project, &one(2), Ticks(150)),
            Err(SplitRefused::TooManyAudioClips)
        );
        assert!(matches!(split(&project, &one(5), Ticks(150)), Ok(Some(_))));
    }

    // gen_p37's slips on p37_slip: each document after, or no edit for an offset that stays.
    #[test]
    fn p37_a_slip_moves_the_audio_and_keeps_the_clip() {
        let want = p37();
        let before = fixture("p37_slip.json");
        for step in want["slip"]["steps"].as_array().expect("steps") {
            let mut project = load(&before);
            let clip = ClipId(step["clip"].as_u64().expect("clip") as u32);
            let ticks = Ticks(step["ticks"].as_i64().expect("ticks"));
            let frames = step["sound_frames"].as_u64();
            let slip = slipped(&project, clip, ticks, frames);
            let Some(after) = step["after"].as_str() else {
                assert_eq!(slip, None, "{step}");
                continue;
            };
            let Some(Edit::TrimAudioClip { start, length, .. }) = slip.clone() else {
                panic!("a trim: {slip:?}");
            };
            let shown = &project.playlist.clips[&clip];
            assert_eq!((start, length), (shown.start, shown.length), "{step}");
            dawsome_engine::edit::apply(&mut project, slip.expect("an edit")).expect("applies");
            assert_eq!(project.to_json().expect("saves"), after, "{step}");
        }
        let project = load(&fixture("p37_cut_pattern.json"));
        assert_eq!(
            slipped(&project, ClipId(1), Ticks(96), None),
            None,
            "a pattern clip"
        );
    }

    // A slip's drag snaps as a move's: whole units from the line the clip starts on. A clip at
    // 40 on bars: 200 ticks rounds to a bar, 150 to none, -250 to a bar back; at 100 on beats
    // (line 96) 50 ticks goes to 192, one beat; with no snap the drag as it is.
    #[test]
    fn a_slip_snaps_its_drag_as_a_move_does() {
        let beat = SnapGrid::Every(Ticks(96));
        let tick = SnapGrid::Every(Ticks(1));
        assert_eq!(slip_ticks(Ticks(40), Ticks(200), BAR), Ticks(384));
        assert_eq!(slip_ticks(Ticks(40), Ticks(150), BAR), Ticks(0));
        assert_eq!(slip_ticks(Ticks(40), Ticks(-250), BAR), Ticks(-384));
        assert_eq!(slip_ticks(Ticks(100), Ticks(50), beat), Ticks(96));
        assert_eq!(slip_ticks(Ticks(100), Ticks(-37), tick), Ticks(-37));
    }

    const BAR_TICKS: Ticks = Ticks(384);
    /// A bar of 4/4 at 96 ppq as an even grid: what Bar snaps to in a song of one 4/4.
    const BAR: SnapGrid<'static> = SnapGrid::Every(BAR_TICKS);

    // The line after tick 0 of each snap in a 4/4 song at 96 ppq: a bar, a beat, a step, a tick.
    #[test]
    fn snap_units_follow_the_ppq() {
        let meter = MeterMap::single(dawsome_engine::time::TimeSignature::COMMON, 96);
        let units: Vec<i64> = ClipSnap::ALL
            .iter()
            .map(|snap| snap.grid(&meter).next(Ticks(0)).0)
            .collect();
        assert_eq!(units, [384, 96, 24, 1]);
        assert_eq!(ClipSnap::default(), ClipSnap::Bar);
        assert_eq!(ClipSnap::Step.to_string(), "Step");
    }

    // p4_two_clips has clips 1 and 2 (next id 3). Pattern 2 at tick 1300 on track index 1
    // snaps down to bar 4 (1152) on track 2, one bar long.
    #[test]
    fn a_placed_clip_starts_on_the_grid_below_the_cursor() {
        let project = load(P4_TWO_CLIPS);
        let placed = place(&project, PatternId(2), 1, Ticks(1300), BAR);
        let clip = Clip::of_pattern(PatternId(2), TrackId(2), Ticks(1152), BAR_TICKS);
        assert_eq!(
            placed,
            Some(Edit::PlaceClips {
                first: ClipId(3),
                clips: vec![clip]
            })
        );
        assert_eq!(place(&project, PatternId(9), 1, Ticks(0), BAR), None);
        assert_eq!(
            place(&project, PatternId(1), 8, Ticks(0), BAR),
            None,
            "no 9th track"
        );
        assert_eq!(place(&project, PatternId(1), 0, Ticks(-5), BAR), None);
    }

    // Clip 1 (0..384, track index 0) and clip 2 (768.., index 0). A drag of 500 ticks rounds to
    // one bar; of 200 to none (nearest); left of tick 0 the group stops at 0; rows stay on the
    // 8 tracks.
    #[test]
    fn a_move_rounds_to_whole_units_and_stays_on_the_playlist() {
        let project = load(P4_TWO_CLIPS);
        let both = [ClipId(1), ClipId(2)];
        assert_eq!(
            move_targets(&project, &both, Ticks(500), 1, BAR),
            [(ClipId(1), Ticks(384), 1), (ClipId(2), Ticks(1152), 1)]
        );
        assert_eq!(
            move_targets(&project, &both, Ticks(200), 0, BAR),
            [(ClipId(1), Ticks(384), 0), (ClipId(2), Ticks(1152), 0)]
        );
        assert_eq!(
            move_targets(&project, &both, Ticks(-1000), -3, BAR),
            [(ClipId(1), Ticks(0), 0), (ClipId(2), Ticks(768), 0)]
        );
        assert_eq!(
            move_targets(&project, &[ClipId(2)], Ticks(0), 20, BAR),
            [(ClipId(2), Ticks(768), 7)]
        );
        let targets = move_targets(&project, &[ClipId(1)], Ticks(150), 0, BAR);
        assert_eq!(
            moved(&project, &targets),
            None,
            "150 ticks rounds to no move"
        );
        let targets = move_targets(&project, &[ClipId(1)], Ticks(0), 2, BAR);
        assert_eq!(
            moved(&project, &targets),
            Some(Edit::MoveClips {
                moves: vec![(ClipId(1), Ticks(0), TrackId(3))]
            })
        );
    }

    // p4_messy's clip 1 starts off the grid at 37: a bar move keeps it 37 past the line, and
    // the furthest left it can go is to 37 (0 whole bars back).
    #[test]
    fn an_off_grid_clip_moves_by_whole_units() {
        let project = load(P4_MESSY);
        assert_eq!(
            move_targets(&project, &[ClipId(1)], Ticks(400), 0, BAR),
            [(ClipId(1), Ticks(421), 0)]
        );
        assert_eq!(
            move_targets(&project, &[ClipId(1)], Ticks(-400), 0, BAR),
            [(ClipId(1), Ticks(37), 0)]
        );
        let meter = MeterMap::single(dawsome_engine::time::TimeSignature::COMMON, 96);
        let step = ClipSnap::Step.grid(&meter);
        assert_eq!(
            move_targets(&project, &[ClipId(1)], Ticks(-30), 0, step),
            [(ClipId(1), Ticks(13), 0)],
            "-30 rounds to one step back"
        );
    }

    // A right-edge drag floors the new end to the grid, at least one grid line past the start's
    // cell: clip 1 (0..384) dragged 768 ticks ends at 1152; 300 ticks floors back to 384 (no
    // change); far left keeps one bar. p4_messy's clip 1 (37..287) dragged 100 ticks floors
    // 387 to 384 (length 347), and dragged far left still ends on 384.
    #[test]
    fn a_resize_floors_the_end_to_the_grid() {
        let project = load(P4_TWO_CLIPS);
        assert_eq!(
            resized(&project, ClipId(1), Ticks(768), BAR),
            Some(Edit::ResizeClip {
                id: ClipId(1),
                length: Ticks(1152)
            })
        );
        assert_eq!(resized(&project, ClipId(1), Ticks(300), BAR), None);
        let clip = &project.playlist.clips[&ClipId(1)];
        assert_eq!(resized_length(clip, Ticks(-5000), BAR), BAR_TICKS);
        let messy = load(P4_MESSY);
        let off_grid = &messy.playlist.clips[&ClipId(1)];
        assert_eq!(resized_length(off_grid, Ticks(100), BAR), Ticks(347));
        assert_eq!(
            resized_length(off_grid, Ticks(0), BAR),
            Ticks(250),
            "a press alone"
        );
        assert_eq!(
            resized_length(off_grid, Ticks(-100), BAR),
            Ticks(347),
            "ends on bar 2"
        );
    }

    #[test]
    fn delete_and_duplicate_name_the_selection_and_the_new_ids() {
        let project = load(P4_TWO_CLIPS);
        let both = BTreeSet::from([ClipId(2), ClipId(1)]);
        assert_eq!(
            deleted(&both),
            Some(Edit::DeleteClips {
                ids: vec![ClipId(1), ClipId(2)]
            })
        );
        assert_eq!(deleted(&BTreeSet::new()), None);
        let (edit, copies) = duplicated(&project, &both).expect("a selection");
        assert_eq!(
            edit,
            Edit::DuplicateClips {
                ids: vec![ClipId(1), ClipId(2)],
                first: ClipId(3)
            }
        );
        assert_eq!(copies, BTreeSet::from([ClipId(3), ClipId(4)]));
        assert_eq!(duplicated(&project, &BTreeSet::new()), None);
    }

    // A drawn range covers the cells from the one pressed to the one under the cursor, either
    // way round; with no snap it is at least a step long.
    #[test]
    fn a_drawn_loop_covers_the_cells_between_press_and_cursor() {
        let step = Ticks(24);
        let range = |start, end| LoopRange {
            start: Ticks(start),
            end: Ticks(end),
            on: true,
        };
        assert_eq!(
            drawn_loop(Ticks(444), Ticks(828), BAR, step),
            range(384, 1152)
        );
        assert_eq!(
            drawn_loop(Ticks(828), Ticks(444), BAR, step),
            range(384, 1152)
        );
        assert_eq!(
            drawn_loop(Ticks(444), Ticks(444), BAR, step),
            range(384, 768)
        );
        assert_eq!(
            drawn_loop(Ticks(100), Ticks(105), SnapGrid::Every(Ticks(1)), step),
            range(100, 124)
        );
        assert_eq!(drawn_loop(Ticks(-50), Ticks(10), BAR, step), range(0, 384));
    }

    // Loop 384..768: the end dragged to tick 1500 lands on the nearest bar line, 1536; the start
    // dragged past the end stops a step before it; the on/off state is kept.
    #[test]
    fn a_loop_edge_moves_to_the_nearest_line_and_keeps_a_step() {
        let range = LoopRange {
            start: Ticks(384),
            end: Ticks(768),
            on: false,
        };
        let step = Ticks(24);
        let end = moved_loop_edge(range, LoopEdge::End, Ticks(1500), BAR, step);
        assert_eq!(
            (end.start, end.end, end.on),
            (Ticks(384), Ticks(1536), false)
        );
        let start = moved_loop_edge(range, LoopEdge::Start, Ticks(2000), BAR, step);
        assert_eq!((start.start, start.end), (Ticks(744), Ticks(768)));
        let end = moved_loop_edge(range, LoopEdge::End, Ticks(0), BAR, step);
        assert_eq!(end.end, Ticks(408));
        let start = moved_loop_edge(range, LoopEdge::Start, Ticks(-900), BAR, step);
        assert_eq!(start.start, Ticks(0));
    }
}
