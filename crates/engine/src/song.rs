//! The song as the notes it plays: each pattern clip's notes on the song ticks it plays them,
//! a clip longer than its pattern repeating it, each note ending after its length (a step hit
//! after one step) or at its clip's end, whichever comes first. These are the rules playback
//! follows (`render`'s clip window and gate); the exports read the song through here, playback
//! never does.

use crate::project::{ChannelId, Clip, Project};
use crate::time::{self, Ticks};

/// A note as a clip plays it: its channel, the song ticks it starts and ends on, its velocity
/// and its key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayedNote {
    pub channel: ChannelId,
    pub start: Ticks,
    /// After `start`: the note's length (one step for a step hit), cut at the clip's end.
    pub end: Ticks,
    pub velocity: f32,
    pub key: u8,
}

/// The notes `clip` plays, by (start, channel, key): song tick T in the clip plays the
/// pattern's tick (T − start + offset) mod L, L the pattern's length. Notes at or past the
/// pattern's end are silent and left out. A clip that is not a pattern clip plays none. Whether
/// the clip plays at all (muted, or its track) is [`Project::clip_plays`]'s to say.
pub fn clip_notes(project: &Project, clip: &Clip) -> Vec<PlayedNote> {
    let Some(pattern) = clip.pattern().and_then(|id| project.patterns.get(&id)) else {
        return Vec::new();
    };
    let length = time::bars_to_ticks(pattern.length_bars, project.ppq)
        .0
        .max(1);
    let step = time::step_to_ticks(1, project.ppq).0;
    let offset = clip.offset.0.rem_euclid(length);
    let end = clip.end().0;
    let mut notes = Vec::new();
    for note in pattern
        .notes
        .iter()
        .filter(|note| (0..length).contains(&note.start.0))
    {
        let gate = if note.length.0 > 0 {
            note.length.0
        } else {
            step
        };
        // The first song tick that plays the note's tick, then every pattern length on.
        let mut start = Some(clip.start.0 + (note.start.0 - offset).rem_euclid(length));
        while let Some(at) = start.filter(|&at| at < end) {
            notes.push(PlayedNote {
                channel: note.channel,
                start: Ticks(at),
                end: Ticks(at.saturating_add(gate).min(end)),
                velocity: note.velocity,
                key: note.key,
            });
            start = at.checked_add(length);
        }
    }
    notes.sort_by_key(|note| (note.start, note.channel, note.key));
    notes
}
