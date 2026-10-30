//! Takes: the edits a recording makes, built from the plain edits so each has its exact inverse.
//! A take of notes in song mode is a new pattern and one clip of it ([`song_take`]), which grow
//! together by whole bars as the take plays ([`take_length`]), and the notes written into it as
//! they end; an audio take is an audio channel and one clip of its file ([`audio_take`]); a
//! Capture is notes laid into a pattern ([`capture_into`]) or a new pattern and its clip
//! ([`capture_song`]); a take of the line in is placed where it was played when its input
//! records late ([`input_take`]). The caller applies each live and keeps the inverses, newest
//! first, as one
//! [`Edit::Transaction`]: one undo step that gives the document back byte for byte, since every
//! inverse here gives back the IDs and counters its edit took.

use std::path::PathBuf;

use super::{Edit, EditError};
use crate::project::{Clip, Note, PatternId, Project, TrackId};
use crate::time::{self, Ticks};

/// The first track, top to bottom, that holds no clip over ticks `start..end`.
pub fn free_track(project: &Project, start: Ticks, end: Ticks) -> Option<TrackId> {
    project.playlist.tracks.keys().copied().find(|&track| {
        !project
            .playlist
            .clips
            .values()
            .any(|clip| clip.track == track && clip.start < end && clip.end() > start)
    })
}

/// Where a take's clip over `start..end` lands: on `wanted` when the project has it, else on
/// the first track free there, else on a new track, which the returned edit adds.
fn place(
    project: &Project,
    wanted: Option<TrackId>,
    start: Ticks,
    end: Ticks,
) -> (TrackId, Option<Edit>) {
    let chosen = wanted
        .filter(|track| project.playlist.tracks.contains_key(track))
        .or_else(|| free_track(project, start, end));
    match chosen {
        Some(track) => (track, None),
        None => {
            let id = project.playlist.next_track_id;
            let add = Edit::AddTrack {
                id,
                name: format!("Track {}", id.0),
            };
            (id, Some(add))
        }
    }
}

/// Whole bars that hold a span of `span` ticks: at least 1. A take in song mode is as long as
/// the bars that hold what it played, and while it plays, the bar the playhead is in.
pub fn bars_holding(span: Ticks, ppq: u16) -> u32 {
    let bar = time::bars_to_ticks(1, ppq).0.max(1);
    // Rounded up; `span` is not negative here and a bar is at least 1.
    let span = span.0.max(0);
    let bars = (span / bar + i64::from(span % bar != 0)).max(1);
    u32::try_from(bars).unwrap_or(u32::MAX)
}

/// "<channel> take N" for the first N from 1 that no pattern of `project` is named.
pub fn take_name(project: &Project, channel: &str) -> String {
    (1u32..)
        .map(|number| format!("{channel} take {number}"))
        .find(|name| {
            !project
                .patterns
                .values()
                .any(|pattern| pattern.name == *name)
        })
        .unwrap_or_else(|| format!("{channel} take"))
}

/// A take of notes in song mode as its first edit: what it adds and under which IDs.
#[derive(Debug, Clone, PartialEq)]
pub struct SongTake {
    pub edit: Edit,
    pub pattern: PatternId,
    pub clip: crate::project::ClipId,
    pub track: TrackId,
    /// The edit adds `track` (no track had room), which the take's undo takes out again.
    pub added_track: bool,
}

/// The pattern `name` of `bars` bars, added after the others, and one clip of it from `start`
/// as long as it, on `track` (the armed one) when the project has it, else on the first track
/// with no clip over its first bar, else on a new track: one transaction, whose inverse takes
/// them out again and gives their IDs back.
pub fn song_take(
    project: &Project,
    name: String,
    start: Ticks,
    track: Option<TrackId>,
    bars: u32,
) -> Result<SongTake, EditError> {
    let pattern = project
        .next_pattern_id()
        .ok_or(EditError::PatternIdsExhausted)?;
    let bar = time::bars_to_ticks(1, project.ppq);
    let first_bar_end = Ticks(start.0.saturating_add(bar.0));
    let (track, add_track) = place(project, track, start, first_bar_end);
    let added_track = add_track.is_some();
    let clip = project.playlist.next_clip_id;
    let length = time::bars_to_ticks(bars, project.ppq);
    let mut edits = vec![Edit::AddPattern {
        id: pattern,
        name,
        bars,
        copied_from: None,
    }];
    edits.extend(add_track);
    edits.push(Edit::PlaceClips {
        first: clip,
        clips: vec![Clip::of_pattern(pattern, track, start, length)],
    });
    Ok(SongTake {
        edit: Edit::Transaction(edits),
        pattern,
        clip,
        track,
        added_track,
    })
}

/// A take's pattern and its clip made `bars` bars long together: the pattern's bars, then the
/// clip's length. Its inverse sets both back.
pub fn take_length(pattern: PatternId, clip: crate::project::ClipId, bars: u32, ppq: u16) -> Edit {
    Edit::Transaction(vec![
        Edit::SetPatternBars { pattern, bars },
        Edit::ResizeClip {
            id: clip,
            length: time::bars_to_ticks(bars, ppq),
        },
    ])
}

/// A new file of `frames` frames (an audio take, or a file dropped), named `sample` as the
/// project names it, as a new audio channel `name` and one clip of it from `start`, as long as
/// the file: on `track` when the project has it, else on the first track free over the clip,
/// else on a new track added in the same step. `None` for an empty file (a take stopped in the
/// count-in), which adds nothing, or one too long for the song.
pub fn audio_take(
    project: &Project,
    name: String,
    sample: PathBuf,
    start: Ticks,
    frames: u64,
    track: Option<TrackId>,
) -> Option<Edit> {
    if frames == 0 {
        return None;
    }
    let length = time::ticks_holding(
        start,
        frames,
        project.sample_rate,
        project.tempo,
        project.ppq,
    )?;
    let end = Ticks(start.0.checked_add(length.0)?);
    let (track, add_track) = place(project, track, start, end);
    let add = Edit::AddAudioClip {
        channel: project.next_channel_id,
        name,
        sample,
        clip: project.playlist.next_clip_id,
        track,
        start,
        length,
    };
    Some(match add_track {
        Some(add_track) => Edit::Transaction(vec![add_track, add]),
        None => add,
    })
}

/// How a take of the line in was placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shift {
    /// From where it started, as recorded: no latency kept, or 0.
    None,
    /// `frames` (at the project's rate) earlier than recorded; later when negative.
    By(i64),
    /// From where it started, as recorded: every frame of it lies within the latency before
    /// the song's start, where no clip can go.
    TooShort,
}

/// Where a take that started at tick `start`, its file `frames` long, its input recording
/// `latency` frames late (at the project's rate; negative: early), must start for its sound to
/// land where it was played, and the frames its clip's offset skips: with p = s(start) −
/// latency, at p ≤ 0 tick 0 and −p frames (the start of the song cuts the take's first frames,
/// played before it); else the first tick t with s(t) ≥ p and s(t) − p frames (less than a
/// tick). File frame j then plays at song sample p + j. `None` when the offset holds every frame
/// of the file, or past what the time base holds.
pub fn shifted_start(
    project: &Project,
    start: Ticks,
    frames: u64,
    latency: i64,
) -> Option<(Ticks, u64)> {
    let (rate, tempo, ppq) = (project.sample_rate, project.tempo, project.ppq);
    let played = i128::from(time::tick_to_sample(start, rate, tempo, ppq)?) - i128::from(latency);
    let (tick, offset) = if played <= 0 {
        (Ticks(0), u64::try_from(-played).ok()?)
    } else {
        let sample = u64::try_from(played).ok()?;
        let tick = time::first_tick_at_or_after(sample, rate, tempo, ppq)?;
        (
            tick,
            time::tick_to_sample(tick, rate, tempo, ppq)?.checked_sub(sample)?,
        )
    };
    (offset < frames).then_some((tick, offset))
}

/// A take of the line in as [`audio_take`] adds it, placed where it was played when its input
/// records `latency` frames late ([`shifted_start`]): the clip from the shifted tick, as long as
/// the frames after its offset, the offset set in the same transaction (one undo step); the
/// file is the whole take. A take with every frame inside the offset is placed as recorded
/// ([`Shift::TooShort`]). `None` as for [`audio_take`].
pub fn input_take(
    project: &Project,
    name: String,
    sample: PathBuf,
    start: Ticks,
    frames: u64,
    track: Option<TrackId>,
    latency: i64,
) -> Option<(Edit, Shift)> {
    if latency == 0 {
        return audio_take(project, name, sample, start, frames, track).map(|e| (e, Shift::None));
    }
    let Some((tick, offset)) = shifted_start(project, start, frames, latency) else {
        let edit = audio_take(project, name, sample, start, frames, track)?;
        return Some((edit, Shift::TooShort));
    };
    let edit = audio_take(project, name, sample, tick, frames - offset, track)?;
    if offset == 0 {
        return Some((edit, Shift::By(latency)));
    }
    let mut edits = match edit {
        Edit::Transaction(edits) => edits,
        add => vec![add],
    };
    let Some(&Edit::AddAudioClip { clip, length, .. }) = edits.last() else {
        unreachable!("an audio take ends with its clip");
    };
    edits.push(Edit::TrimAudioClip {
        clip,
        start: tick,
        length,
        frame_offset: offset,
    });
    Some((Edit::Transaction(edits), Shift::By(latency)))
}

/// `notes` with no two on one channel, tick and key: of two, the later in the list stays, as a
/// note recorded over another replaces it.
fn one_per_slot(notes: Vec<Note>) -> Vec<Note> {
    let mut kept: Vec<Note> = Vec::with_capacity(notes.len());
    for note in notes {
        kept.retain(|old| {
            (old.channel, old.start, old.key) != (note.channel, note.start, note.key)
        });
        kept.push(note);
    }
    kept
}

/// The bars that hold every note's end (a step hit its start), at least `least`.
fn bars_for(notes: &[Note], least: u32, ppq: u16) -> u32 {
    let reach = notes
        .iter()
        .map(|note| note.start.0.saturating_add(note.length.0))
        .max()
        .unwrap_or(0);
    bars_holding(Ticks(reach), ppq).max(least)
}

/// Captured `notes` laid into `pattern` (their ticks from its start), each replacing a note of
/// its channel on its tick and key, the pattern growing by whole bars when they reach past its
/// end: one transaction. `None` without notes or without the pattern.
pub fn capture_into(project: &Project, pattern: PatternId, notes: Vec<Note>) -> Option<Edit> {
    let held = project.patterns.get(&pattern)?;
    let notes = one_per_slot(notes);
    if notes.is_empty() {
        return None;
    }
    let bars = bars_for(&notes, held.length_bars, project.ppq);
    let remove = held
        .notes
        .iter()
        .filter(|old| {
            notes
                .iter()
                .any(|new| (new.channel, new.start, new.key) == (old.channel, old.start, old.key))
        })
        .cloned()
        .collect();
    let mut edits = Vec::new();
    if bars != held.length_bars {
        edits.push(Edit::SetPatternBars { pattern, bars });
    }
    edits.push(Edit::ReplaceNotes {
        pattern,
        remove,
        add: notes,
    });
    Some(Edit::Transaction(edits))
}

/// Captured `notes` as a new pattern `name` of the bars that hold them, and its clip from
/// `start`, placed as [`song_take`] places it: one transaction. `None` without notes.
pub fn capture_song(
    project: &Project,
    name: String,
    start: Ticks,
    track: Option<TrackId>,
    notes: Vec<Note>,
) -> Result<Option<Edit>, EditError> {
    let notes = one_per_slot(notes);
    if notes.is_empty() {
        return Ok(None);
    }
    let bars = bars_for(&notes, 1, project.ppq);
    let take = song_take(project, name, start, track, bars)?;
    let Edit::Transaction(mut edits) = take.edit else {
        unreachable!("a song take is a transaction");
    };
    edits.push(Edit::ReplaceNotes {
        pattern: take.pattern,
        remove: Vec::new(),
        add: notes,
    });
    Ok(Some(Edit::Transaction(edits)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // By hand at 48 kHz and 120 BPM, where s(t) = 250·t (gen_p23's cases): from tick 192 (sample
    // 48000) 1234 frames late, p = 46766: tick 188 (47000), 234 frames skipped; from tick 0,
    // tick 0 and 1234 skipped; 960 early, p = 48960: tick 196, 40 skipped; exactly a tick, 250
    // late, tick 191 and none skipped; a latency holding the whole take (30000 of 24000, 1234 of
    // 1234 at tick 0) leaves nothing.
    #[test]
    fn a_late_input_starts_its_take_earlier() {
        let project = Project::new(48_000, time::MilliBpm(120_000), 96);
        let at = |start, frames, latency| shifted_start(&project, Ticks(start), frames, latency);
        assert_eq!(at(192, 24_000, 1234), Some((Ticks(188), 234)));
        assert_eq!(at(0, 24_000, 1234), Some((Ticks(0), 1234)));
        assert_eq!(at(192, 24_000, -960), Some((Ticks(196), 40)));
        assert_eq!(at(192, 24_000, 250), Some((Ticks(191), 0)));
        assert_eq!(at(0, 24_000, 30_000), None);
        assert_eq!(at(0, 1234, 1234), None);
        assert_eq!(at(0, 1235, 1234), Some((Ticks(0), 1234)));
    }
}
