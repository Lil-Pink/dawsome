//! A sound's edits as the sample editor makes them: [`Edit::SetChannelEdits`] itself, and the
//! edits that one step makes of an op appended to a channel's list ([`append`]) or of the list
//! taken back to the file ([`revert`]). An op that changes the sound's length (a crop, a
//! delete) moves every audio clip of the channel by its frame map in the same transaction, so
//! each keeps playing the audio it played; a stretched clip keeps its ratio. Pure, as
//! `takes.rs` is: the caller says how many frames the sound has, which the document does not
//! hold (media knows it).

use std::mem;

use super::{channel_mut, Applied, Edit, EditError};
use crate::project::{ChannelId, ClipItem, ClipStretch, EditsSource, Project, SoundEdits, SoundOp};

/// Gives `channel` the edit list `edits`; the inverse gives it the list it had. Refused for a
/// channel that plays no file of its own and for a list no sound can hold.
pub(super) fn set(
    project: &mut Project,
    channel: ChannelId,
    edits: SoundEdits,
) -> Result<Applied, EditError> {
    let shown = channel_mut(project, channel)?;
    if !shown.plays_a_file() {
        return Err(EditError::NoSoundFile(channel.0));
    }
    let old = mem::replace(&mut shown.edits, edits);
    if let Some(problem) = shown.edits_problem() {
        shown.edits = old;
        return Err(problem.into());
    }
    Ok(Applied::channel(
        Edit::SetChannelEdits {
            channel,
            edits: old,
        },
        channel,
    ))
}

/// The one step that appends `op` to `channel`'s list, its sound `frames` frames long as the
/// list leaves it now (a first op keeping that length and the project's rate as the list's
/// source): [`Edit::SetChannelEdits`] with the longer list, and when the op changes
/// the length, a transaction of it first (so it names the step) and then a
/// [`Edit::TrimAudioClip`] or [`Edit::SetClipStretch`] for every clip of the channel whose
/// window moves. Refused for a channel that plays no file of its own and for a delete that
/// would leave no frame.
pub fn append(
    project: &Project,
    channel: ChannelId,
    op: SoundOp,
    frames: u64,
) -> Result<Edit, EditError> {
    let shown = project
        .channels
        .get(&channel)
        .ok_or(EditError::UnknownChannel(channel.0))?;
    if !shown.plays_a_file() {
        return Err(EditError::NoSoundFile(channel.0));
    }
    let after = op.frames_after(frames);
    if after == 0 && frames > 0 {
        return Err(EditError::NoFramesLeft(channel.0));
    }
    // The first op keeps what it found: the sound's length and the project's rate then, which
    // tell a file changed on disk since.
    let edits = if shown.edits.is_empty() {
        SoundEdits {
            ops: vec![op],
            source: Some(EditsSource {
                frames,
                rate: project.sample_rate,
            }),
        }
    } else {
        shown.edits.with(op)
    };
    let set = Edit::SetChannelEdits { channel, edits };
    let moved = moved_clips(project, channel, |clip| {
        clip.step(frames, after, |frame| op.map_frame(frames, frame))
    });
    Ok(one_step(set, moved))
}

/// The one step that takes `channel`'s list back to none, the file as it is, `frames` frames
/// long: every clip of the channel moved back through the list's crops and deletes, last first,
/// so it plays the audio it played where the file holds it (a frame a delete closed up on going
/// back to the first frame after the ones it took out).
pub fn revert(project: &Project, channel: ChannelId, frames: u64) -> Result<Edit, EditError> {
    let shown = project
        .channels
        .get(&channel)
        .ok_or(EditError::UnknownChannel(channel.0))?;
    if !shown.plays_a_file() {
        return Err(EditError::NoSoundFile(channel.0));
    }
    // The sound's length before each op: lengths[i] before op i, the last after them all.
    let mut lengths = vec![frames];
    for op in &shown.edits.ops {
        let last = lengths[lengths.len() - 1];
        lengths.push(op.frames_after(last));
    }
    let set = Edit::SetChannelEdits {
        channel,
        edits: SoundEdits::default(),
    };
    let moved = moved_clips(project, channel, |mut clip| {
        for (index, op) in shown.edits.ops.iter().enumerate().rev() {
            let (before, after) = (lengths[index], lengths[index + 1]);
            clip = clip.step(after, before, |frame| op.unmap_frame(before, frame));
        }
        clip
    });
    Ok(one_step(set, moved))
}

/// `set` alone, or a transaction of it and then `moved`.
fn one_step(set: Edit, moved: Vec<Edit>) -> Edit {
    if moved.is_empty() {
        set
    } else {
        Edit::Transaction([set].into_iter().chain(moved).collect())
    }
}

/// An audio clip's window onto its channel's sound: where it starts and how it is stretched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Window {
    frame_offset: u64,
    stretch: Option<ClipStretch>,
}

impl Window {
    /// The window on a sound of `before` frames that becomes one of `after` frames, a frame of
    /// it moving as `map` says: the offset mapped; a stretched clip's offset (stretched frames)
    /// taken to its sound's frame first (floor) and back after (floor), its length made
    /// `frames × after / before`, half up, at least 1, so its ratio stays.
    fn step(self, before: u64, after: u64, map: impl Fn(u64) -> u64) -> Self {
        if before == after {
            return Self {
                frame_offset: map(self.frame_offset),
                ..self
            };
        }
        let Some(stretch) = self.stretch else {
            return Self {
                frame_offset: map(self.frame_offset),
                ..self
            };
        };
        let (total, before, after) = (
            u128::from(stretch.frames),
            u128::from(before),
            u128::from(after),
        );
        let frames = ((2 * total * after + before) / (2 * before)).max(1);
        let source = u128::from(self.frame_offset) * before / total;
        let mapped = u128::from(map(u64::try_from(source).unwrap_or(u64::MAX)));
        let offset = mapped * frames / after.max(1);
        Self {
            frame_offset: u64::try_from(offset).unwrap_or(u64::MAX),
            stretch: Some(ClipStretch {
                frames: u64::try_from(frames).unwrap_or(u64::MAX),
                ..stretch
            }),
        }
    }
}

/// The edits that move every audio clip of `channel` whose window `step` changes, in clip
/// order: a [`Edit::TrimAudioClip`] for a clip not stretched, a [`Edit::SetClipStretch`] for one
/// that is. Each clip keeps its start and its length in ticks.
fn moved_clips(
    project: &Project,
    channel: ChannelId,
    step: impl Fn(Window) -> Window,
) -> Vec<Edit> {
    let mut moved = Vec::new();
    for (&id, clip) in &project.playlist.clips {
        let ClipItem::Audio {
            channel: played,
            frame_offset,
            stretch,
            ..
        } = clip.item
        else {
            continue;
        };
        if played != channel {
            continue;
        }
        let window = Window {
            frame_offset,
            stretch,
        };
        let new = step(window);
        if new == window {
            continue;
        }
        moved.push(match new.stretch {
            None => Edit::TrimAudioClip {
                clip: id,
                start: clip.start,
                length: clip.length,
                frame_offset: new.frame_offset,
            },
            Some(_) => Edit::SetClipStretch {
                clip: id,
                stretch: new.stretch,
                frame_offset: new.frame_offset,
                length: clip.length,
            },
        });
    }
    moved
}
