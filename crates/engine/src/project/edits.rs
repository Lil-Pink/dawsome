//! A sound's edits: the ordered list of operations a sampler's or an audio channel's sound file
//! plays through (crop, delete, silence, fades, gain, normalize, reverse), so a sound is shaped
//! while its file stays as it is. Positions are frames at the project's rate (as a clip's
//! `frame_offset` is) in the sound as the ops before leave it; gains are whole hundredths of a
//! dB, so a list is exact data that orders and hashes, a cache key as it is.
//! [`crate::dsp::edits`] applies a list to a sound's frames.

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

/// The most ops one sound's list holds.
pub const MAX_SOUND_EDITS: usize = 1000;

/// A gain op's range, in hundredths of a dB: −48 dB to +24 dB.
pub const EDIT_GAIN_RANGE: RangeInclusive<i32> = -4800..=2400;

/// Where a normalize puts the loudest value, in hundredths of a dB: −48 dB to full scale.
pub const NORMALIZE_RANGE: RangeInclusive<i32> = -4800..=0;

/// A sound's edits, applied in order as an undo history replays. Written
/// `"edits":{"ops":[...],"source":{"frames":..,"rate":..}}` after the channel's sound settings,
/// only when it holds an op. An object, so what a later build keeps beside the ops (markers)
/// has a place without changing this shape; fields it does not know are read past, since none
/// of them plays (a build before `source` reads past it too: it is what the list saw, not what
/// it does).
///
/// Two lists are one (equal, ordered and hashed alike, one cache key) when their ops are:
/// `source` says what the file was when the list began, which changes nothing that plays.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SoundEdits {
    #[serde(default)]
    pub ops: Vec<SoundOp>,
    /// The sound as the list's first op found it, written with that op: what tells a file
    /// changed on disk since (the sample editor says so; the ops still apply, by frame). None
    /// in a list from before it was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<EditsSource>,
}

/// What a sound's edit list found when its first op was made: the sound's length in `frames`
/// at `rate`, the project's rate then (the rate its frames, and the ops' positions, count in).
/// A file that is now another length at the project's rate, or a project at another rate, is
/// not the sound the ops were made on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditsSource {
    pub frames: u64,
    pub rate: u32,
}

impl PartialEq for SoundEdits {
    fn eq(&self, other: &Self) -> bool {
        self.ops == other.ops
    }
}

impl Eq for SoundEdits {}

impl Hash for SoundEdits {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.ops.hash(state);
    }
}

impl PartialOrd for SoundEdits {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SoundEdits {
    fn cmp(&self, other: &Self) -> Ordering {
        self.ops.cmp(&other.ops)
    }
}

/// One edit of a sound, over the frames `[from, to)` of the sound as the ops before it leave
/// it. A range reaching past the sound's end is cut to it, and an op wholly past it does
/// nothing, so a file that changed on disk still plays sensibly. An op or a field this build
/// does not know is refused by its name: read past, it would play the sound otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum SoundOp {
    /// Keeps the range alone.
    Crop { from: u64, to: u64 },
    /// Takes the range out; the rest closes up.
    Delete { from: u64, to: u64 },
    /// Sets the range to silence, its length kept.
    Silence { from: u64, to: u64 },
    /// Rises from silence over the range: frame k of N is scaled by k/N.
    FadeIn { from: u64, to: u64 },
    /// Falls to silence over the range: frame k of N is scaled by (N − 1 − k)/N.
    FadeOut { from: u64, to: u64 },
    /// Scales the range by `centi_db` hundredths of a dB, in [`EDIT_GAIN_RANGE`].
    Gain { from: u64, to: u64, centi_db: i32 },
    /// Scales the range so its loudest value (on either side) lands on `centi_db` hundredths of
    /// a dB, in [`NORMALIZE_RANGE`]: full scale when not written. A silent range stays as it is.
    Normalize {
        from: u64,
        to: u64,
        #[serde(default, skip_serializing_if = "is_zero")]
        centi_db: i32,
    },
    /// Plays the range backwards.
    Reverse { from: u64, to: u64 },
}

fn is_zero(value: &i32) -> bool {
    *value == 0
}

/// What makes a list one no sound can hold, by the op it is in (counted from 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditsProblem {
    /// More ops than [`MAX_SOUND_EDITS`].
    TooMany(usize),
    /// An op over no frames: `to` is not past `from`.
    EmptyRange { index: usize, from: u64, to: u64 },
    /// A gain or a normalize's target outside its range.
    Gain { index: usize, centi_db: i32 },
}

impl SoundOp {
    /// The frames the op names, `[from, to)`, as written.
    pub fn span(&self) -> (u64, u64) {
        match *self {
            SoundOp::Crop { from, to }
            | SoundOp::Delete { from, to }
            | SoundOp::Silence { from, to }
            | SoundOp::FadeIn { from, to }
            | SoundOp::FadeOut { from, to }
            | SoundOp::Gain { from, to, .. }
            | SoundOp::Normalize { from, to, .. }
            | SoundOp::Reverse { from, to } => (from, to),
        }
    }

    /// What an undo step of this op is called.
    pub fn name(&self) -> &'static str {
        match self {
            SoundOp::Crop { .. } => "Crop",
            SoundOp::Delete { .. } => "Delete part",
            SoundOp::Silence { .. } => "Silence",
            SoundOp::FadeIn { .. } => "Fade in",
            SoundOp::FadeOut { .. } => "Fade out",
            SoundOp::Gain { .. } => "Gain",
            SoundOp::Normalize { .. } => "Normalize",
            SoundOp::Reverse { .. } => "Reverse",
        }
    }

    /// The frames the op works on in a sound of `frames` frames: its range cut to the sound's
    /// end, empty (`lo == hi`) when the op does nothing there.
    pub fn cut(&self, frames: u64) -> (u64, u64) {
        let (from, to) = self.span();
        let (lo, hi) = (from.min(frames), to.min(frames));
        (lo, hi.max(lo))
    }

    /// The frames a sound of `frames` frames has after the op.
    pub fn frames_after(&self, frames: u64) -> u64 {
        let (lo, hi) = self.cut(frames);
        match self {
            SoundOp::Crop { .. } if lo < hi => hi - lo,
            SoundOp::Delete { .. } => frames - (hi - lo),
            _ => frames,
        }
    }

    /// Where frame `frame` of a sound of `frames` frames is after the op, so what played from
    /// there plays on from where it went: a crop moves the frames it keeps to its start (those
    /// before it to 0, those after it to its end), a delete closes the gap (the frames it takes
    /// out to where it was); every other op moves nothing.
    pub fn map_frame(&self, frames: u64, frame: u64) -> u64 {
        let (lo, hi) = self.cut(frames);
        if lo == hi {
            return frame;
        }
        match self {
            SoundOp::Crop { .. } => frame.clamp(lo, hi) - lo,
            SoundOp::Delete { .. } if frame < lo => frame,
            SoundOp::Delete { .. } => frame.max(hi) - (hi - lo),
            _ => frame,
        }
    }

    /// Where frame `frame` of the sound after the op (of `frames` frames before it) was before
    /// it: the frame [`map_frame`](Self::map_frame) takes there; for the frame a delete closed
    /// up on, the first frame after the ones it took out, whose audio plays there now.
    pub fn unmap_frame(&self, frames: u64, frame: u64) -> u64 {
        let (lo, hi) = self.cut(frames);
        if lo == hi {
            return frame;
        }
        match self {
            SoundOp::Crop { .. } => frame.saturating_add(lo),
            SoundOp::Delete { .. } if frame < lo => frame,
            SoundOp::Delete { .. } => frame.saturating_add(hi - lo),
            _ => frame,
        }
    }

    fn problem(&self, index: usize) -> Option<EditsProblem> {
        let (from, to) = self.span();
        if to <= from {
            return Some(EditsProblem::EmptyRange { index, from, to });
        }
        match *self {
            SoundOp::Gain { centi_db, .. } if !EDIT_GAIN_RANGE.contains(&centi_db) => {
                Some(EditsProblem::Gain { index, centi_db })
            }
            SoundOp::Normalize { centi_db, .. } if !NORMALIZE_RANGE.contains(&centi_db) => {
                Some(EditsProblem::Gain { index, centi_db })
            }
            _ => None,
        }
    }
}

impl SoundEdits {
    /// Whether the list holds no op: the sound plays its file as it is.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// The list with `op` after its own (what it found of the sound kept).
    pub fn with(&self, op: SoundOp) -> Self {
        let mut ops = self.ops.clone();
        ops.push(op);
        Self { ops, ..*self }
    }

    /// Whether the sound it plays through is not the one its first op was made on: `frames`
    /// long at `rate` now, where the list found another length or rate. A list that kept
    /// nothing of it (one from before it was kept) never says so.
    pub fn file_changed(&self, frames: u64, rate: u32) -> bool {
        self.source
            .is_some_and(|source| source != EditsSource { frames, rate })
    }

    /// The first thing that makes the list one no sound can hold: too many ops, an op over no
    /// frames, a gain out of its range.
    pub fn problem(&self) -> Option<EditsProblem> {
        if self.ops.len() > MAX_SOUND_EDITS {
            return Some(EditsProblem::TooMany(self.ops.len()));
        }
        self.ops
            .iter()
            .enumerate()
            .find_map(|(index, op)| op.problem(index))
    }

    /// The list's ops as the project file writes them, `{"ops":[...]}`: the text a cache of
    /// what it makes is keyed by (what it found of the file is not how it plays).
    pub fn key_text(&self) -> String {
        #[derive(Serialize)]
        struct Ops<'a> {
            ops: &'a [SoundOp],
        }
        // A list of integers and names always serializes.
        serde_json::to_string(&Ops { ops: &self.ops }).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The file's shape: an object of ops, each named, its fields in order, a normalize's target
    // only when set; ops of another build refused by their names, fields beside the ops read
    // past.
    #[test]
    fn a_list_writes_its_ops_by_name_and_refuses_ones_it_does_not_know() {
        let edits = SoundEdits {
            ops: vec![
                SoundOp::Crop {
                    from: 12000,
                    to: 36000,
                },
                SoundOp::Gain {
                    from: 0,
                    to: 24000,
                    centi_db: -300,
                },
                SoundOp::Normalize {
                    from: 0,
                    to: 8,
                    centi_db: 0,
                },
                SoundOp::FadeIn { from: 0, to: 480 },
            ],
            source: None,
        };
        let text = edits.key_text();
        assert_eq!(
            text,
            r#"{"ops":[{"crop":{"from":12000,"to":36000}},{"gain":{"from":0,"to":24000,"centi_db":-300}},{"normalize":{"from":0,"to":8}},{"fade_in":{"from":0,"to":480}}]}"#
        );
        assert_eq!(
            serde_json::from_str::<SoundEdits>(&text).expect("reads"),
            edits
        );
        let error = serde_json::from_str::<SoundEdits>(r#"{"ops":[{"warble":{"from":0,"to":8}}]}"#)
            .expect_err("an op of another build");
        assert!(error.to_string().contains("warble"), "{error}");
        let error =
            serde_json::from_str::<SoundEdits>(r#"{"ops":[{"crop":{"from":0,"to":8,"curve":1}}]}"#)
                .expect_err("a field of another build");
        assert!(error.to_string().contains("curve"), "{error}");
        let marked: SoundEdits =
            serde_json::from_str(r#"{"ops":[{"reverse":{"from":0,"to":8}}],"markers":[4]}"#)
                .expect("markers are read past");
        assert_eq!(marked.ops, [SoundOp::Reverse { from: 0, to: 8 }]);
    }

    // What a list's first op found of the sound: written after the ops, read back; a list
    // from before it reads with none. Two lists of the same ops are one (equal, hashed alike)
    // whatever they found, and the cache's text is the ops alone, as it was. A sound of another
    // length, or a project at another rate, is a changed file; a list that kept nothing never
    // says so; an op more keeps what the list found.
    #[test]
    fn a_lists_source_is_what_its_first_op_found_of_the_sound() {
        use std::collections::hash_map::DefaultHasher;
        let ops = vec![SoundOp::Crop {
            from: 1000,
            to: 2000,
        }];
        let found = SoundEdits {
            ops: ops.clone(),
            source: Some(EditsSource {
                frames: 72_000,
                rate: 44_100,
            }),
        };
        let text = serde_json::to_string(&found).expect("writes");
        assert_eq!(
            text,
            r#"{"ops":[{"crop":{"from":1000,"to":2000}}],"source":{"frames":72000,"rate":44100}}"#
        );
        let read: SoundEdits = serde_json::from_str(&text).expect("reads");
        assert_eq!(read.source, found.source);
        let before: SoundEdits =
            serde_json::from_str(r#"{"ops":[{"crop":{"from":1000,"to":2000}}]}"#).expect("reads");
        assert_eq!(before.source, None);
        assert_eq!(before, found, "one list");
        let hash = |edits: &SoundEdits| {
            let mut hasher = DefaultHasher::new();
            edits.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash(&before), hash(&found));
        assert_eq!(
            found.key_text(),
            r#"{"ops":[{"crop":{"from":1000,"to":2000}}]}"#
        );
        assert_eq!(found.key_text(), before.key_text());
        assert!(!found.file_changed(72_000, 44_100));
        assert!(found.file_changed(48_000, 44_100), "another length");
        assert!(found.file_changed(72_000, 48_000), "another rate");
        assert!(
            !before.file_changed(48_000, 44_100),
            "a list that kept nothing"
        );
        let more = found.with(SoundOp::Reverse { from: 0, to: 10 });
        assert_eq!((more.ops.len(), more.source), (2, found.source));
    }

    // The frame maps of the spec, by hand: crop [a, b) moves f < a to 0, a <= f < b to f - a and
    // f >= b to b - a; delete [a, b) keeps f < a, moves a <= f < b to a and f >= b back by
    // b - a. A range past the end is cut to it.
    #[test]
    fn a_crop_and_a_delete_move_frames_as_the_spec_says() {
        let crop = SoundOp::Crop { from: 10, to: 20 };
        let moved: Vec<u64> = [0, 9, 10, 15, 19, 20, 30]
            .iter()
            .map(|&f| crop.map_frame(100, f))
            .collect();
        assert_eq!(moved, [0, 0, 0, 5, 9, 10, 10]);
        assert_eq!(crop.frames_after(100), 10);
        let delete = SoundOp::Delete { from: 10, to: 20 };
        let moved: Vec<u64> = [0, 9, 10, 15, 19, 20, 30]
            .iter()
            .map(|&f| delete.map_frame(100, f))
            .collect();
        assert_eq!(moved, [0, 9, 10, 10, 10, 10, 20]);
        assert_eq!(delete.frames_after(100), 90);
        assert_eq!(delete.unmap_frame(100, 10), 20);
        assert_eq!(crop.unmap_frame(100, 5), 15);
        // Past the end: cut to the sound's 15 frames; wholly past it, nothing.
        assert_eq!(crop.frames_after(15), 5);
        assert_eq!(SoundOp::Crop { from: 20, to: 30 }.frames_after(15), 15);
        assert_eq!(SoundOp::Delete { from: 20, to: 30 }.map_frame(15, 14), 14);
        assert_eq!(SoundOp::Silence { from: 0, to: 9 }.map_frame(15, 3), 3);
    }

    #[test]
    fn a_list_out_of_bounds_is_named() {
        let edits = |ops: Vec<SoundOp>| SoundEdits { ops, source: None };
        assert_eq!(edits(Vec::new()).problem(), None);
        assert_eq!(
            edits(vec![SoundOp::Silence { from: 4, to: 4 }]).problem(),
            Some(EditsProblem::EmptyRange {
                index: 0,
                from: 4,
                to: 4
            })
        );
        let loud = SoundOp::Gain {
            from: 0,
            to: 4,
            centi_db: 2401,
        };
        assert_eq!(
            edits(vec![SoundOp::Reverse { from: 0, to: 4 }, loud]).problem(),
            Some(EditsProblem::Gain {
                index: 1,
                centi_db: 2401
            })
        );
        let over = SoundOp::Normalize {
            from: 0,
            to: 4,
            centi_db: 1,
        };
        assert!(edits(vec![over]).problem().is_some());
        let quiet = SoundOp::Gain {
            from: 0,
            to: 4,
            centi_db: -4800,
        };
        assert_eq!(edits(vec![quiet]).problem(), None);
        let many = edits(vec![
            SoundOp::Reverse { from: 0, to: 4 };
            MAX_SOUND_EDITS + 1
        ]);
        assert_eq!(many.problem(), Some(EditsProblem::TooMany(1001)));
        assert_eq!(
            edits(vec![SoundOp::Reverse { from: 0, to: 4 }; MAX_SOUND_EDITS]).problem(),
            None
        );
    }
}
