//! Playlist edits: clips placed, copied, deleted, moved and resized, a pattern clip given
//! another pattern, audio clips added with their channel, trimmed, gained and muted, the loop range, and the tracks. Each one checks everything before it changes anything, so a refused edit leaves the
//! project as it was, and each returns its exact inverse. Clips and loop ranges are checked by
//! the same rules [`Project::validate`] applies, so no edit can make a project it would refuse.

use std::collections::BTreeSet;
use std::mem;
use std::path::PathBuf;

use super::{Applied, Edit, EditError, Touched};
use crate::project::{
    Channel, ChannelId, Clip, ClipId, ClipItem, ClipStretch, LoopRange, PatternId, Project, Track,
    TrackId, ValidationError, CLIP_GAIN_RANGE, MAX_TRACKS,
};
use crate::time::Ticks;

pub(super) fn place(
    project: &mut Project,
    first: ClipId,
    clips: Vec<Clip>,
) -> Result<Applied, EditError> {
    if clips.is_empty() {
        return Err(EditError::NothingToChange);
    }
    let expected = project.playlist.next_clip_id;
    if first != expected {
        return Err(EditError::NotNextClipId {
            got: first.0,
            expected: expected.0,
        });
    }
    let count = u32::try_from(clips.len()).map_err(|_| EditError::ClipIdsExhausted)?;
    let next = first
        .0
        .checked_add(count)
        .ok_or(EditError::ClipIdsExhausted)?;
    let ids = (first.0..next).map(ClipId);
    for (id, clip) in ids.clone().zip(&clips) {
        // Only a project that never passed `validate` can hold an ID at or above the next one.
        if project.playlist.clips.contains_key(&id) {
            return Err(EditError::ClipExists(id.0));
        }
        project.check_clip(id, clip)?;
    }
    project.playlist.clips.extend(ids.zip(clips));
    project.playlist.next_clip_id = ClipId(next);
    Ok(Applied::playlist(Edit::UnplaceClips { first, count }))
}

pub(super) fn unplace(
    project: &mut Project,
    first: ClipId,
    count: u32,
) -> Result<Applied, EditError> {
    if count == 0 {
        return Err(EditError::NothingToChange);
    }
    let playlist = &mut project.playlist;
    let end = u64::from(first.0) + u64::from(count);
    let not_newest = EditError::NotNewestClips {
        first: first.0,
        end,
    };
    if end != u64::from(playlist.next_clip_id.0) {
        return Err(not_newest);
    }
    let ids = (first.0..playlist.next_clip_id.0).map(ClipId);
    if !ids.clone().all(|id| playlist.clips.contains_key(&id)) {
        return Err(not_newest);
    }
    let clips = ids.filter_map(|id| playlist.clips.remove(&id)).collect();
    // Giving the IDs back is safe: they were the newest, and nothing refers to a clip.
    playlist.next_clip_id = first;
    Ok(Applied::playlist(Edit::PlaceClips { first, clips }))
}

pub(super) fn duplicate(
    project: &mut Project,
    ids: &[ClipId],
    first: ClipId,
) -> Result<Applied, EditError> {
    let mut originals = existing(project, ids)?;
    let earliest = originals.iter().map(|(_, clip)| clip.start).min();
    let latest = originals.iter().map(|(_, clip)| clip.end()).max();
    let (Some(earliest), Some(latest)) = (earliest, latest) else {
        return Err(EditError::NothingToChange);
    };
    let shift = latest.0 - earliest.0;
    originals.sort_by_key(|&(id, clip)| (clip.start, id));
    let copies = originals
        .into_iter()
        .map(|(_, clip)| Clip {
            // Too far for the song saturates, and the clip check then refuses it.
            start: Ticks(clip.start.0.saturating_add(shift)),
            ..clip.clone()
        })
        .collect();
    place(project, first, copies)
}

pub(super) fn delete(project: &mut Project, ids: &[ClipId]) -> Result<Applied, EditError> {
    existing(project, ids)?;
    let clips = ids
        .iter()
        .filter_map(|&id| Some((id, project.playlist.clips.remove(&id)?)))
        .collect();
    Ok(Applied::playlist(Edit::RestoreClips { clips }))
}

pub(super) fn restore(
    project: &mut Project,
    clips: Vec<(ClipId, Clip)>,
) -> Result<Applied, EditError> {
    if clips.is_empty() {
        return Err(EditError::NothingToChange);
    }
    let mut seen = BTreeSet::new();
    for (id, clip) in &clips {
        if !seen.insert(*id) {
            return Err(EditError::ClipListedTwice(id.0));
        }
        if project.playlist.clips.contains_key(id) {
            return Err(EditError::ClipExists(id.0));
        }
        let next = project.playlist.next_clip_id;
        if *id >= next {
            return Err(EditError::Invalid(ValidationError::UnallocatedClipId {
                id: id.0,
                next: next.0,
            }));
        }
        project.check_clip(*id, clip)?;
    }
    let ids = clips.iter().map(|&(id, _)| id).collect();
    project.playlist.clips.extend(clips);
    Ok(Applied::playlist(Edit::DeleteClips { ids }))
}

pub(super) fn move_clips(
    project: &mut Project,
    moves: &[(ClipId, Ticks, TrackId)],
) -> Result<Applied, EditError> {
    let ids: Vec<ClipId> = moves.iter().map(|&(id, _, _)| id).collect();
    let named = existing(project, &ids)?;
    for (&(id, start, track), &(_, clip)) in moves.iter().zip(&named) {
        let target = Clip {
            start,
            track,
            ..clip.clone()
        };
        project.check_clip(id, &target)?;
    }
    let before = named
        .iter()
        .map(|&(id, clip)| (id, clip.start, clip.track))
        .collect();
    for &(id, start, track) in moves {
        if let Some(clip) = project.playlist.clips.get_mut(&id) {
            clip.start = start;
            clip.track = track;
        }
    }
    Ok(Applied::playlist(Edit::MoveClips { moves: before }))
}

pub(super) fn resize(
    project: &mut Project,
    id: ClipId,
    length: Ticks,
) -> Result<Applied, EditError> {
    let clip = project
        .playlist
        .clips
        .get(&id)
        .ok_or(EditError::UnknownClip(id.0))?;
    let target = Clip {
        length,
        ..clip.clone()
    };
    project.check_clip(id, &target)?;
    let clip = project
        .playlist
        .clips
        .get_mut(&id)
        .ok_or(EditError::UnknownClip(id.0))?;
    let old = mem::replace(&mut clip.length, length);
    Ok(Applied::playlist(Edit::ResizeClip { id, length: old }))
}

pub(super) fn set_loop(
    project: &mut Project,
    value: Option<LoopRange>,
) -> Result<Applied, EditError> {
    if let Some(range) = &value {
        project.check_loop(range)?;
    }
    let old = mem::replace(&mut project.playlist.loop_range, value);
    Ok(Applied::playlist(Edit::SetLoop { value: old }))
}

pub(super) fn add_track(
    project: &mut Project,
    id: TrackId,
    name: String,
) -> Result<Applied, EditError> {
    let playlist = &mut project.playlist;
    if id != playlist.next_track_id || playlist.tracks.contains_key(&id) {
        return Err(EditError::NotNextTrackId {
            got: id.0,
            expected: playlist.next_track_id.0,
        });
    }
    if playlist.tracks.len() >= MAX_TRACKS {
        return Err(EditError::TooManyTracks);
    }
    let next = id.0.checked_add(1).ok_or(EditError::TrackIdsExhausted)?;
    playlist.tracks.insert(id, Track::named(name));
    playlist.next_track_id = TrackId(next);
    Ok(Applied::playlist(Edit::RemoveTrack { id }))
}

pub(super) fn remove_track(project: &mut Project, id: TrackId) -> Result<Applied, EditError> {
    let playlist = &mut project.playlist;
    let track = playlist
        .tracks
        .get(&id)
        .ok_or(EditError::UnknownTrack(id.0))?;
    let newest = id.0.checked_add(1) == Some(playlist.next_track_id.0);
    let has_clips = playlist.clips.values().any(|clip| clip.track == id);
    // A playlist always keeps one track (`validate` needs at least one).
    if !newest || track.mute || has_clips || playlist.tracks.len() == 1 {
        return Err(EditError::TrackNotRemovable(id.0));
    }
    let Some(track) = playlist.tracks.remove(&id) else {
        return Err(EditError::UnknownTrack(id.0));
    };
    // Giving the ID back is safe: it was the newest, and no clip is on it.
    playlist.next_track_id = id;
    Ok(Applied::playlist(Edit::AddTrack {
        id,
        name: track.name,
    }))
}

pub(super) fn rename_track(
    project: &mut Project,
    id: TrackId,
    name: String,
) -> Result<Applied, EditError> {
    let track = track_mut(project, id)?;
    let old = mem::replace(&mut track.name, name);
    Ok(Applied::playlist(Edit::RenameTrack { id, name: old }))
}

pub(super) fn set_track_mute(
    project: &mut Project,
    id: TrackId,
    mute: bool,
) -> Result<Applied, EditError> {
    let track = track_mut(project, id)?;
    let old = mem::replace(&mut track.mute, mute);
    Ok(Applied::playlist(Edit::SetTrackMute { id, mute: old }))
}

fn track_mut(project: &mut Project, id: TrackId) -> Result<&mut Track, EditError> {
    project
        .playlist
        .tracks
        .get_mut(&id)
        .ok_or(EditError::UnknownTrack(id.0))
}

/// The clips `ids` names, in that order: at least one, each existing and named once.
fn existing<'a>(
    project: &'a Project,
    ids: &[ClipId],
) -> Result<Vec<(ClipId, &'a Clip)>, EditError> {
    if ids.is_empty() {
        return Err(EditError::NothingToChange);
    }
    let mut seen = BTreeSet::new();
    ids.iter()
        .map(|&id| {
            if !seen.insert(id) {
                return Err(EditError::ClipListedTwice(id.0));
            }
            let clip = project
                .playlist
                .clips
                .get(&id)
                .ok_or(EditError::UnknownClip(id.0))?;
            Ok((id, clip))
        })
        .collect()
}

/// An audio clip's item, field by field.
struct AudioItem {
    channel: ChannelId,
    frame_offset: u64,
    gain: f32,
    stretch: Option<ClipStretch>,
}

impl AudioItem {
    fn item(&self) -> ClipItem {
        ClipItem::Audio {
            channel: self.channel,
            frame_offset: self.frame_offset,
            gain: self.gain,
            stretch: self.stretch,
        }
    }
}

/// The audio clip `id` and its item.
fn audio_item(project: &Project, id: ClipId) -> Result<(&Clip, AudioItem), EditError> {
    let clip = project
        .playlist
        .clips
        .get(&id)
        .ok_or(EditError::UnknownClip(id.0))?;
    match clip.item {
        ClipItem::Audio {
            channel,
            frame_offset,
            gain,
            stretch,
        } => Ok((
            clip,
            AudioItem {
                channel,
                frame_offset,
                gain,
                stretch,
            },
        )),
        ClipItem::Pattern { .. } | ClipItem::Automation { .. } => {
            Err(EditError::NotAnAudioClip(id.0))
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the fields of `Edit::AddAudioClip`, one by one, as every edit's applier takes its \
              edit's fields"
)]
pub(super) fn add_audio_clip(
    project: &mut Project,
    channel: ChannelId,
    name: String,
    sample: PathBuf,
    clip: ClipId,
    track: TrackId,
    start: Ticks,
    length: Ticks,
) -> Result<Applied, EditError> {
    if channel != project.next_channel_id || project.channels.contains_key(&channel) {
        return Err(EditError::NotNextChannelId {
            got: channel.0,
            expected: project.next_channel_id.0,
        });
    }
    let next_channel = channel
        .0
        .checked_add(1)
        .ok_or(EditError::ChannelIdsExhausted)?;
    let playlist = &project.playlist;
    if clip != playlist.next_clip_id || playlist.clips.contains_key(&clip) {
        return Err(EditError::NotNextClipId {
            got: clip.0,
            expected: playlist.next_clip_id.0,
        });
    }
    let next_clip = clip.0.checked_add(1).ok_or(EditError::ClipIdsExhausted)?;
    let placed = Clip::of_audio(channel, track, start, length);
    // The channel goes in first so the clip check finds it; a refused clip takes it out.
    project
        .channels
        .insert(channel, Channel::audio(name, sample));
    if let Err(error) = project.check_clip(clip, &placed) {
        project.channels.remove(&channel);
        return Err(error.into());
    }
    project.next_channel_id = ChannelId(next_channel);
    project.playlist.clips.insert(clip, placed);
    project.playlist.next_clip_id = ClipId(next_clip);
    let touched = Touched {
        channels: BTreeSet::from([channel]),
        playlist: true,
        ..Touched::default()
    };
    Ok(Applied {
        inverse: Edit::UnaddAudioClip { channel, clip },
        touched,
    })
}

pub(super) fn unadd_audio_clip(
    project: &mut Project,
    channel: ChannelId,
    clip: ClipId,
) -> Result<Applied, EditError> {
    let refused = EditError::NotNewestAudioClip {
        channel: channel.0,
        clip: clip.0,
    };
    let newest = channel.0.checked_add(1) == Some(project.next_channel_id.0)
        && clip.0.checked_add(1) == Some(project.playlist.next_clip_id.0);
    let (Some(added), Some(placed)) = (
        project.channels.get(&channel),
        project.playlist.clips.get(&clip),
    ) else {
        return Err(refused);
    };
    // Only as the add left them, so the add this gives back restores them exactly.
    let as_added = *added == Channel::audio(added.name.clone(), added.sample.clone())
        && *placed == Clip::of_audio(channel, placed.track, placed.start, placed.length);
    let others = project
        .playlist
        .clips
        .iter()
        .any(|(&other, placed)| other != clip && placed.audio_channel() == Some(channel));
    let has_notes = project
        .patterns
        .values()
        .any(|pattern| pattern.notes.iter().any(|note| note.channel == channel));
    if !newest || !as_added || others || has_notes {
        return Err(refused);
    }
    let (Some(removed), Some(placed)) = (
        project.channels.remove(&channel),
        project.playlist.clips.remove(&clip),
    ) else {
        return Err(refused);
    };
    // Giving both IDs back is safe: they were the newest, and nothing refers to them any more.
    project.next_channel_id = channel;
    project.playlist.next_clip_id = clip;
    let inverse = Edit::AddAudioClip {
        channel,
        name: removed.name,
        sample: removed.sample,
        clip,
        track: placed.track,
        start: placed.start,
        length: placed.length,
    };
    let touched = Touched {
        channels: BTreeSet::from([channel]),
        playlist: true,
        ..Touched::default()
    };
    Ok(Applied { inverse, touched })
}

pub(super) fn trim_audio_clip(
    project: &mut Project,
    id: ClipId,
    start: Ticks,
    length: Ticks,
    frame_offset: u64,
) -> Result<Applied, EditError> {
    let (clip, audio) = audio_item(project, id)?;
    let old_offset = audio.frame_offset;
    let target = Clip {
        start,
        length,
        item: AudioItem {
            frame_offset,
            ..audio
        }
        .item(),
        ..clip.clone()
    };
    project.check_clip(id, &target)?;
    let inverse = Edit::TrimAudioClip {
        clip: id,
        start: clip.start,
        length: clip.length,
        frame_offset: old_offset,
    };
    project.playlist.clips.insert(id, target);
    Ok(Applied::playlist(inverse))
}

pub(super) fn set_clip_gain(
    project: &mut Project,
    id: ClipId,
    gain: f32,
) -> Result<Applied, EditError> {
    // `contains` is false for NaN and infinity.
    if !CLIP_GAIN_RANGE.contains(&gain) {
        return Err(EditError::InvalidGain(gain));
    }
    let (_, audio) = audio_item(project, id)?;
    let old = audio.gain;
    let item = AudioItem { gain, ..audio }.item();
    if let Some(clip) = project.playlist.clips.get_mut(&id) {
        clip.item = item;
    }
    Ok(Applied::playlist(Edit::SetClipGain {
        clip: id,
        gain: old,
    }))
}

pub(super) fn set_clip_stretch(
    project: &mut Project,
    id: ClipId,
    stretch: Option<ClipStretch>,
    frame_offset: u64,
    length: Ticks,
) -> Result<Applied, EditError> {
    let (clip, audio) = audio_item(project, id)?;
    let inverse = Edit::SetClipStretch {
        clip: id,
        stretch: audio.stretch,
        frame_offset: audio.frame_offset,
        length: clip.length,
    };
    let target = Clip {
        length,
        item: AudioItem {
            stretch,
            frame_offset,
            ..audio
        }
        .item(),
        ..clip.clone()
    };
    project.check_clip(id, &target)?;
    project.playlist.clips.insert(id, target);
    Ok(Applied::playlist(inverse))
}

/// Makes pattern clip `id` play `pattern`; the inverse gives it back the pattern it played.
pub(super) fn set_clip_pattern(
    project: &mut Project,
    id: ClipId,
    pattern: PatternId,
) -> Result<Applied, EditError> {
    let clip = project
        .playlist
        .clips
        .get(&id)
        .ok_or(EditError::UnknownClip(id.0))?;
    let old = clip.pattern().ok_or(EditError::NotAPatternClip(id.0))?;
    if !project.patterns.contains_key(&pattern) {
        return Err(EditError::UnknownPattern(pattern.0));
    }
    if old == pattern {
        return Err(EditError::NothingToChange);
    }
    if let Some(clip) = project.playlist.clips.get_mut(&id) {
        clip.item = ClipItem::Pattern { pattern };
    }
    // The song plays other events from here on; the patterns' own events are as they were.
    Ok(Applied::playlist(Edit::SetClipPattern { id, pattern: old }))
}

/// Mutes or unmutes `ids`. The inverse puts each clip back as it was: one edit for the clips
/// that were muted and one for those that were not, in one transaction when both exist.
pub(super) fn set_clip_mute(
    project: &mut Project,
    ids: &[ClipId],
    muted: bool,
) -> Result<Applied, EditError> {
    let named = existing(project, ids)?;
    let (mut was_muted, mut was_unmuted) = (Vec::new(), Vec::new());
    for &(id, clip) in &named {
        if clip.muted {
            was_muted.push(id);
        } else {
            was_unmuted.push(id);
        }
    }
    for &id in ids {
        if let Some(clip) = project.playlist.clips.get_mut(&id) {
            clip.muted = muted;
        }
    }
    let mut undo: Vec<Edit> = [(was_muted, true), (was_unmuted, false)]
        .into_iter()
        .filter(|(ids, _)| !ids.is_empty())
        .map(|(ids, muted)| Edit::SetClipMute { ids, muted })
        .collect();
    let inverse = if undo.len() == 1 {
        undo.remove(0)
    } else {
        Edit::Transaction(undo)
    };
    Ok(Applied::playlist(inverse))
}
