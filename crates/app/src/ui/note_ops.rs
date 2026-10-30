//! The piano roll's note operations: transpose, nudge, quantize,
//! chop, glue, legato, reverse, invert, humanize, velocity ramps and paste. Each is a pure
//! function from the notes it acts on to the [`Change`] it makes, so every rule is tested on
//! hand-written notes, and the roll commits it as the one `ReplaceNotes` edit every gesture is
//! (one undo step). No iced types.
//!
//! Two notes of a channel never share a tick and key (the document refuses it). An operation
//! that would land two there keeps one ([`settled`]): the longest, then the loudest, then the
//! first given. A note of the pattern an operation lands on is replaced, as a moved note
//! replaces one ([`Change::new`]).

use std::collections::BTreeSet;

use dawsome_engine::project::Note;
use dawsome_engine::time::Ticks;

use crate::ui::note_gesture::{
    key_of, moved, snap_nearest, velocity_of_steps, velocity_steps, Change, Target, VELOCITY_STEPS,
};
use crate::ui::piano_layout::drawn_length;

/// What humanize moves a note by at most: ticks either way, and velocity steps (of 128) either
/// way.
pub const HUMANIZE_TICKS: i64 = 5;
pub const HUMANIZE_STEPS: i64 = 8;

/// `notes` with no two on one channel, tick and key: of those that would share, the longest
/// stays, then the loudest, then the first given. Sorted as the pattern keeps notes (tick,
/// channel, key).
pub fn settled(notes: Vec<Note>) -> Vec<Note> {
    let mut indexed: Vec<(usize, Note)> = notes.into_iter().enumerate().collect();
    indexed.sort_by(|(a_index, a), (b_index, b)| {
        let place = |note: &Note| (note.start, note.channel, note.key);
        place(a)
            .cmp(&place(b))
            .then(b.length.cmp(&a.length))
            .then(b.velocity.total_cmp(&a.velocity))
            .then(a_index.cmp(b_index))
    });
    let mut kept: Vec<Note> = Vec::with_capacity(indexed.len());
    for (_, note) in indexed {
        if kept.last().map(key_of) != Some(key_of(&note)) {
            kept.push(note);
        }
    }
    kept
}

/// The change that puts `add` in place of `notes`, kept apart by [`settled`].
fn replaced(target: &Target, notes: &[Note], add: Vec<Note>) -> Change {
    Change::new(target, notes.to_vec(), settled(add))
}

/// `notes` up (or down) by `keys` semitones, together: never past key 0 or 127, so a group
/// whose highest note is 127 goes no higher.
pub fn transposed(target: &Target, notes: &[Note], keys: i32) -> Change {
    moved(target, notes, Ticks(0), keys)
}

/// `notes` later (or earlier) by `ticks`, together: never before tick 0. Past the pattern's
/// end the pattern grows, as a moved note makes it.
pub fn nudged(target: &Target, notes: &[Note], ticks: Ticks) -> Change {
    moved(target, notes, ticks, 0)
}

/// How a quantize moves notes: to the nearest line of `grid`, `strength` per cent of the way
/// (0 to 100), the starts only or the ends too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quantize {
    pub grid: Ticks,
    pub strength: u8,
    pub ends: bool,
}

impl Quantize {
    /// The grid at full strength, starts only.
    #[cfg(test)]
    pub fn full(grid: Ticks) -> Self {
        Self {
            grid,
            strength: 100,
            ends: false,
        }
    }

    /// `strength` per cent of the way from `tick` to the grid line nearest it (half way rounds
    /// up), the ticks rounded half away from zero.
    fn toward_grid(self, tick: i64) -> i64 {
        let line = snap_nearest(Ticks(tick), self.grid).0;
        tick + share(line - tick, i64::from(self.strength.min(100)))
    }
}

/// `percent` per cent of `ticks`, rounded half away from zero: 50 % of −5 is −3.
fn share(ticks: i64, percent: i64) -> i64 {
    let scaled = ticks * percent;
    let rounded = (scaled.abs() * 2 + 100) / 200;
    rounded * scaled.signum()
}

/// `notes` quantized by `how`. A start is moved toward its nearest grid line; with `ends`, a
/// sounding note's end is moved toward its own, and a note whose end would land on or before
/// its start is one grid line long. A step hit (no length) keeps none. A start moved onto or
/// past the pattern's end wraps to its start, as a recorded note's does: the loop's next
/// downbeat.
pub fn quantized(target: &Target, notes: &[Note], how: Quantize) -> Change {
    let pattern_end = target.end().0;
    let add = notes
        .iter()
        .map(|note| {
            let mut start = how.toward_grid(note.start.0);
            let length = if how.ends && note.length.0 > 0 {
                let end = how.toward_grid(note.start.0 + note.length.0);
                if end > start {
                    end - start
                } else {
                    how.grid.0.max(1)
                }
            } else {
                note.length.0
            };
            if note.start.0 < pattern_end && start >= pattern_end {
                start -= pattern_end;
            }
            Note {
                start: Ticks(start),
                length: Ticks(length),
                ..note.clone()
            }
        })
        .collect();
    replaced(target, notes, add)
}

/// `notes` cut at every multiple of `unit` strictly inside them, each piece a note with the
/// same key and velocity. A step hit, and a note no line crosses, stay as they are.
pub fn chopped(target: &Target, notes: &[Note], unit: Ticks) -> Change {
    let unit = unit.0.max(1);
    let mut add = Vec::new();
    for note in notes {
        let (start, end) = (note.start.0, note.start.0 + note.length.0);
        let mut from = start;
        let mut line = (start.div_euclid(unit) + 1) * unit;
        while line < end {
            add.push(Note {
                start: Ticks(from),
                length: Ticks(line - from),
                ..note.clone()
            });
            from = line;
            line += unit;
        }
        add.push(Note {
            start: Ticks(from),
            length: Ticks(end - from),
            ..note.clone()
        });
    }
    replaced(target, notes, add)
}

/// `notes` of one channel and key that touch or overlap joined into one note from the first's
/// start to the last end, at the first's velocity. A step hit ends where it starts.
pub fn glued(target: &Target, notes: &[Note]) -> Change {
    let mut sorted = notes.to_vec();
    sorted.sort_by_key(|note| (note.channel, note.key, note.start));
    let mut add: Vec<Note> = Vec::with_capacity(sorted.len());
    for note in sorted {
        let end = note.start.0 + note.length.0;
        match add.last_mut() {
            Some(run)
                if run.channel == note.channel
                    && run.key == note.key
                    && note.start.0 <= run.start.0 + run.length.0 =>
            {
                run.length = Ticks(run.length.0.max(end - run.start.0));
            }
            _ => add.push(note),
        }
    }
    replaced(target, notes, add)
}

/// `notes` each made to end where the next of them starts, on any key; the notes starting last
/// keep their length.
pub fn legato(target: &Target, notes: &[Note]) -> Change {
    let starts: BTreeSet<i64> = notes.iter().map(|note| note.start.0).collect();
    let add = notes
        .iter()
        .map(|note| {
            let next = starts.range(note.start.0 + 1..).next();
            Note {
                length: next.map_or(note.length, |&next| Ticks(next - note.start.0)),
                ..note.clone()
            }
        })
        .collect();
    replaced(target, notes, add)
}

/// `notes` mirrored in time within their span (the first start to the last end as drawn, a
/// step hit a step long): a note that ended last starts first.
pub fn reversed(target: &Target, notes: &[Note], step: Ticks) -> Change {
    let first = notes.iter().map(|note| note.start.0).min();
    let last = notes
        .iter()
        .map(|note| note.start.0 + drawn_length(note, step).0)
        .max();
    let (Some(first), Some(last)) = (first, last) else {
        return Change::default();
    };
    let add = notes
        .iter()
        .map(|note| Note {
            start: Ticks(first + last - (note.start.0 + drawn_length(note, step).0)),
            ..note.clone()
        })
        .collect();
    replaced(target, notes, add)
}

/// `notes` mirrored in pitch within their span of keys: the highest becomes the lowest.
pub fn inverted(target: &Target, notes: &[Note]) -> Change {
    let (Some(lowest), Some(highest)) = (
        notes.iter().map(|note| note.key).min(),
        notes.iter().map(|note| note.key).max(),
    ) else {
        return Change::default();
    };
    let add = notes
        .iter()
        .map(|note| Note {
            // Within lowest..=highest, so it stays a key.
            key: lowest + highest - note.key,
            ..note.clone()
        })
        .collect();
    replaced(target, notes, add)
}

/// SplitMix64's mixing of `x`: the same value for the same `x` on every machine.
fn mix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A whole number from −`most` to `most`, drawn from `seed` for note `index`, the time's
/// (`which` 0) or the velocity's (1).
fn wobble(seed: u64, index: usize, which: u64, most: i64) -> i64 {
    let drawn = mix((seed << 32) ^ (2 * index as u64 + which));
    let span = (2 * most + 1) as u64;
    (drawn % span) as i64 - most
}

/// `notes` moved by up to [`HUMANIZE_TICKS`] either way and made louder or softer by up to
/// [`HUMANIZE_STEPS`], each by numbers drawn from `seed` and its place in `notes`, so the same
/// seed humanizes the same notes the same way. A start stays at 0 or later, and a note that
/// ended inside the pattern still does; velocities stay within 0 to 128 steps.
pub fn humanized(target: &Target, notes: &[Note], seed: u64, step: Ticks) -> Change {
    let pattern_end = target.end().0;
    let add = notes
        .iter()
        .enumerate()
        .map(|(index, note)| {
            let drawn = drawn_length(note, step).0;
            let mut start = (note.start.0 + wobble(seed, index, 0, HUMANIZE_TICKS)).max(0);
            if note.start.0 + drawn <= pattern_end {
                start = start.min(pattern_end - drawn).max(0);
            }
            let steps =
                i64::from(velocity_steps(note.velocity)) + wobble(seed, index, 1, HUMANIZE_STEPS);
            // Clamped into 0..=128 first, so the conversion is exact.
            let steps = steps.clamp(0, i64::from(VELOCITY_STEPS)) as u8;
            Note {
                start: Ticks(start),
                velocity: velocity_of_steps(steps),
                ..note.clone()
            }
        })
        .collect();
    replaced(target, notes, add)
}

/// `notes` that start from tick `from.0` to tick `to.0` (either way round) given velocities on
/// the line from `from.1` to `to.1` steps, by where each starts, rounded half away from zero;
/// the rest stay. A line no wider than a tick gives its notes the end's value.
pub fn ramped(target: &Target, notes: &[Note], from: (Ticks, u8), to: (Ticks, u8)) -> Change {
    let (low, high) = (from.0 .0.min(to.0 .0), from.0 .0.max(to.0 .0));
    let reached: Vec<Note> = notes
        .iter()
        .filter(|note| (low..=high).contains(&note.start.0))
        .cloned()
        .collect();
    let width = to.0 .0 - from.0 .0;
    let rise = i64::from(to.1) - i64::from(from.1);
    let add = reached
        .iter()
        .map(|note| {
            let steps = if width == 0 {
                i64::from(to.1)
            } else {
                let scaled = rise * (note.start.0 - from.0 .0);
                let rounded = (scaled.abs() * 2 + width.abs()) / (2 * width.abs());
                i64::from(from.1) + rounded * scaled.signum() * width.signum()
            };
            // On the line between two values of 0..=128, so within it.
            let steps = steps.clamp(0, i64::from(VELOCITY_STEPS)) as u8;
            Note {
                velocity: velocity_of_steps(steps),
                ..note.clone()
            }
        })
        .collect();
    replaced(target, &reached, add)
}

/// Each of `painted`'s notes at its own velocity in steps: what a drag across the velocity
/// lane's stems sets.
pub fn with_velocities(target: &Target, painted: &[(Note, u8)]) -> Change {
    let notes: Vec<Note> = painted.iter().map(|(note, _)| note.clone()).collect();
    let add = painted
        .iter()
        .map(|(note, steps)| Note {
            velocity: velocity_of_steps(*steps),
            ..note.clone()
        })
        .collect();
    replaced(target, &notes, add)
}

/// Notes copied for pasting: their starts counted from the first, their keys, lengths and
/// velocities as they were. The clipboard holds one, whatever pattern and channel it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    notes: Vec<Note>,
    /// From the first start to the last end as drawn (a step hit a step long).
    pub span: Ticks,
}

impl Clip {
    /// `notes` as a clip; none for no notes.
    pub fn of(notes: &[Note], step: Ticks) -> Option<Self> {
        let first = notes.iter().map(|note| note.start.0).min()?;
        let last = notes
            .iter()
            .map(|note| note.start.0 + drawn_length(note, step).0)
            .max()?;
        let mut copied: Vec<Note> = notes
            .iter()
            .map(|note| Note {
                start: Ticks(note.start.0 - first),
                ..note.clone()
            })
            .collect();
        copied.sort_by_key(|note| (note.start, note.key));
        Some(Self {
            notes: copied,
            span: Ticks(last - first),
        })
    }

    /// How many notes it holds.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.notes.len()
    }
}

/// `clip`'s notes on the target channel from tick `at`, on their own keys. A note of the
/// channel where one lands is replaced; past the pattern's end the pattern grows.
pub fn pasted(target: &Target, clip: &Clip, at: Ticks) -> Change {
    let add = clip
        .notes
        .iter()
        .map(|note| Note {
            channel: target.channel,
            start: Ticks(note.start.0 + at.0.max(0)),
            ..note.clone()
        })
        .collect();
    Change::new(target, Vec::new(), settled(add))
}

#[cfg(test)]
mod tests {
    use dawsome_engine::edit::{Edit, History};
    use dawsome_engine::project::{ChannelId, PatternId, Project};

    use super::*;

    const STEP: Ticks = Ticks(24);
    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    /// A note of channel 1 at `tick`, `length` long on `key`, at velocity 100 of 128.
    fn n(tick: i64, length: i64, key: u8) -> Note {
        nv(tick, length, key, 100)
    }

    /// The same at velocity `steps` of 128.
    fn nv(tick: i64, length: i64, key: u8, steps: u8) -> Note {
        Note {
            channel: ChannelId(1),
            start: Ticks(tick),
            length: Ticks(length),
            velocity: f32::from(steps) / 128.0,
            key,
        }
    }

    /// `notes` in the pattern's order.
    fn sorted(mut notes: Vec<Note>) -> Vec<Note> {
        notes.sort_by_key(|note| (note.start, note.channel, note.key));
        notes
    }

    /// A one-bar pattern (384 ticks at ppq 96) holding `notes`, channel 1 shown.
    fn on(notes: &[Note]) -> Target<'_> {
        Target {
            pattern: PatternId(1),
            length_bars: 1,
            ppq: 96,
            channel: ChannelId(1),
            notes,
        }
    }

    /// What `change` leaves of `notes`: the pattern's notes after it, in the pattern's order.
    fn after(notes: &[Note], change: &Change) -> Vec<Note> {
        let gone: BTreeSet<_> = change.remove.iter().map(key_of).collect();
        let kept = notes.iter().filter(|note| !gone.contains(&key_of(note)));
        sorted(kept.cloned().chain(change.add.iter().cloned()).collect())
    }

    /// p3_roll with its pattern holding `notes` instead.
    fn project_with(notes: &[Note]) -> Project {
        let text = std::fs::read_to_string(format!("{FIXTURES}/p3_roll.json")).expect("reads");
        let mut project = Project::load_json(&text).expect("loads");
        project
            .patterns
            .get_mut(&PatternId(1))
            .expect("pattern 1")
            .notes = sorted(notes.to_vec());
        project
    }

    /// `operation` on `notes` applied through the history: the document holds what `after`
    /// says, one undo gives the bytes from before back and one redo the bytes from after.
    fn round_trip(notes: &[Note], operation: impl Fn(&Target, &[Note]) -> Change) {
        let mut project = project_with(notes);
        let before = project.to_json().expect("writes");
        let shown = sorted(notes.to_vec());
        let target = Target::of(&project, Some(PatternId(1)), Some(ChannelId(1))).expect("shown");
        let change = operation(&target, &shown);
        let expected = after(&shown, &change);
        let edit = change.into_edit(&target).expect("changes something");
        let mut history = History::new();
        history.apply(&mut project, edit).expect("applies");
        let pattern = &project.patterns[&PatternId(1)];
        assert_eq!(pattern.notes, expected);
        let done = project.to_json().expect("writes");
        history.undo(&mut project).expect("undoes").expect("a step");
        assert_eq!(project.to_json().expect("writes"), before, "undo");
        history.redo(&mut project).expect("redoes").expect("a step");
        assert_eq!(project.to_json().expect("writes"), done, "redo");
        assert_eq!(history.undo_len(), 1, "one undo step");
    }

    #[test]
    fn notes_landing_together_keep_the_longest_then_the_loudest_then_the_first() {
        let kept = settled(vec![n(0, 24, 60), n(0, 48, 60), nv(0, 48, 60, 64)]);
        assert_eq!(kept, [n(0, 48, 60)]);
        let kept = settled(vec![nv(24, 24, 61, 64), nv(24, 24, 61, 90), n(0, 0, 62)]);
        assert_eq!(kept, [n(0, 0, 62), nv(24, 24, 61, 90)]);
        let first = settled(vec![nv(5, 10, 60, 50), nv(5, 10, 60, 50)]);
        assert_eq!(first, [nv(5, 10, 60, 50)]);
        assert_eq!(settled(Vec::new()), []);
    }

    // Up a semitone and an octave, down an octave; at the top (127) the group goes no higher,
    // and near the bottom it stops at key 0 together.
    #[test]
    fn transposing_moves_keys_together_inside_the_keyboard() {
        let notes = [n(0, 24, 60), n(24, 24, 64)];
        let target = on(&notes);
        assert_eq!(
            transposed(&target, &notes, 1).add,
            [n(0, 24, 61), n(24, 24, 65)]
        );
        assert_eq!(
            transposed(&target, &notes, 12).add,
            [n(0, 24, 72), n(24, 24, 76)]
        );
        assert_eq!(
            transposed(&target, &notes, -12).add,
            [n(0, 24, 48), n(24, 24, 52)]
        );
        let top = [n(0, 24, 120), n(24, 24, 127)];
        assert!(transposed(&on(&top), &top, 12).is_empty());
        let low = [n(0, 24, 5), n(24, 24, 60)];
        assert_eq!(
            transposed(&on(&low), &low, -12).add,
            [n(0, 24, 0), n(24, 24, 55)]
        );
        assert!(transposed(&target, &[], 1).into_edit(&target).is_none());
        round_trip(&notes, |target, notes| transposed(target, notes, 12));
    }

    // A step later; a step earlier stops at tick 0; a note at the pattern's last step nudged on
    // grows the pattern to 2 bars in the same edit.
    #[test]
    fn nudging_moves_by_the_snap_and_stops_at_tick_0() {
        let notes = [n(0, 24, 60)];
        let target = on(&notes);
        assert_eq!(nudged(&target, &notes, STEP).add, [n(24, 24, 60)]);
        let late = [n(10, 24, 60), n(30, 24, 62)];
        assert_eq!(
            nudged(&on(&late), &late, Ticks(-24)).add,
            [n(0, 24, 60), n(20, 24, 62)]
        );
        let end = [n(360, 24, 60)];
        let edit = nudged(&on(&end), &end, STEP).into_edit(&on(&end));
        assert_eq!(
            edit,
            Some(Edit::Transaction(vec![
                Edit::ReplaceNotes {
                    pattern: PatternId(1),
                    remove: vec![n(360, 24, 60)],
                    add: vec![n(384, 24, 60)],
                },
                Edit::SetPatternBars {
                    pattern: PatternId(1),
                    bars: 2,
                },
            ]))
        );
        round_trip(&late, |target, notes| nudged(target, notes, STEP));
    }

    // Grid 24. At 100 %: 5 → 0, 40 → 48, 100 → 96. At 50 %: 5 goes 2.5 back, rounded away
    // from zero to 3 (→ 2), 40 goes 4 on (→ 44), 100 goes 2 back (→ 98).
    #[test]
    fn quantizing_moves_starts_toward_the_grid_by_the_strength() {
        let notes = [n(5, 30, 60), n(40, 20, 62), n(100, 10, 64)];
        let target = on(&notes);
        let full = quantized(&target, &notes, Quantize::full(STEP));
        assert_eq!(full.add, [n(0, 30, 60), n(48, 20, 62), n(96, 10, 64)]);
        let half = Quantize {
            strength: 50,
            ..Quantize::full(STEP)
        };
        assert_eq!(
            quantized(&target, &notes, half).add,
            [n(2, 30, 60), n(44, 20, 62), n(98, 10, 64)]
        );
        let none = Quantize {
            strength: 0,
            ..Quantize::full(STEP)
        };
        assert!(quantized(&target, &notes, none).is_empty());
        round_trip(&notes, |target, notes| {
            quantized(target, notes, Quantize::full(STEP))
        });
    }

    // Starts and ends, grid 24: 5..35 → 0..24; 40..60 → 48..72; 100..110 → 96..120; 13..15
    // would end where it starts (24..24), so it is a grid line long; a step hit at 50 moves to
    // 48 and keeps no length.
    #[test]
    fn quantizing_ends_too_keeps_every_sounding_note_at_least_a_line_long() {
        let notes = [
            n(5, 30, 60),
            n(13, 2, 65),
            n(40, 20, 62),
            n(50, 0, 66),
            n(100, 10, 64),
        ];
        let target = on(&notes);
        let both = Quantize {
            ends: true,
            ..Quantize::full(STEP)
        };
        assert_eq!(
            quantized(&target, &notes, both).add,
            [
                n(0, 24, 60),
                n(24, 24, 65),
                n(48, 24, 62),
                n(48, 0, 66),
                n(96, 24, 64)
            ]
        );
        round_trip(&notes, |target, notes| quantized(target, notes, both));
    }

    // Saturated: 380 snaps onto the pattern's end (384) and wraps to 0, where a note from 2
    // lands too: the longer (10 ticks) stays. 370 → 360. Nothing grows the pattern.
    #[test]
    fn quantizing_wraps_the_end_onto_the_start_and_keeps_one_of_two_landing_together() {
        let notes = [n(2, 10, 60), n(370, 20, 61), n(380, 4, 60)];
        let target = on(&notes);
        let change = quantized(&target, &notes, Quantize::full(STEP));
        assert_eq!(change.add, [n(0, 10, 60), n(360, 20, 61)]);
        assert!(matches!(
            change.into_edit(&target),
            Some(Edit::ReplaceNotes { .. })
        ));
        assert_eq!(
            quantized(&target, &[], Quantize::full(STEP)),
            Change::default()
        );
        round_trip(&notes, |target, notes| {
            quantized(target, notes, Quantize::full(STEP))
        });
    }

    // Unit 24: 0..48 → 0..24, 24..48; 30..60 → 30..48, 48..60. A step hit and a note one unit
    // long stay. Saturated: a note ending on the pattern's end, and two overlapping notes of
    // one key whose pieces meet at 24: the louder piece (100) stays.
    #[test]
    fn chopping_cuts_notes_at_every_snap_line_inside_them() {
        let notes = [n(0, 48, 60), n(30, 30, 62)];
        let target = on(&notes);
        assert_eq!(
            chopped(&target, &notes, STEP).add,
            [n(0, 24, 60), n(24, 24, 60), n(30, 18, 62), n(48, 12, 62)]
        );
        let whole = [n(24, 0, 60), n(48, 24, 61)];
        assert!(chopped(&on(&whole), &whole, STEP).is_empty());
        let end = [n(336, 48, 60)];
        assert_eq!(
            chopped(&on(&end), &end, STEP).add,
            [n(336, 24, 60), n(360, 24, 60)]
        );
        let overlapping = [nv(0, 48, 64, 100), nv(24, 48, 64, 64)];
        assert_eq!(
            chopped(&on(&overlapping), &overlapping, STEP).add,
            [nv(0, 24, 64, 100), nv(24, 24, 64, 100), nv(48, 24, 64, 64)]
        );
        round_trip(&notes, |target, notes| chopped(target, notes, STEP));
        round_trip(&overlapping, |target, notes| chopped(target, notes, STEP));
    }

    // Touching (0..24, 24..48) → 0..48; overlapping 0..48 and 24..72 → 0..72 at the first's
    // velocity; other keys and gaps stay apart. Saturated: a step hit at a note's end, two
    // notes ending on the pattern's end.
    #[test]
    fn gluing_joins_touching_and_overlapping_notes_of_one_key() {
        let notes = [n(0, 24, 60), n(24, 24, 60), n(48, 24, 62)];
        let target = on(&notes);
        assert_eq!(glued(&target, &notes).add, [n(0, 48, 60), n(48, 24, 62)]);
        let overlapping = [nv(0, 48, 60, 90), nv(24, 48, 60, 40)];
        assert_eq!(
            glued(&on(&overlapping), &overlapping).add,
            [nv(0, 72, 60, 90)]
        );
        let apart = [n(0, 24, 60), n(24, 24, 61), n(60, 24, 60)];
        assert!(glued(&on(&apart), &apart).is_empty());
        let hit = [n(0, 24, 60), n(24, 0, 60)];
        assert_eq!(glued(&on(&hit), &hit).add, [n(0, 24, 60)]);
        let end = [n(336, 24, 60), n(360, 24, 60)];
        assert_eq!(glued(&on(&end), &end).add, [n(336, 48, 60)]);
        round_trip(&overlapping, glued);
        round_trip(&hit, glued);
    }

    // Each note ends where the next starts: 0 → 96, 96 → 192, the last keeps its 24. A chord
    // shares the next start. Saturated: step hits get lengths, an overlapping note is
    // shortened, one note alone changes nothing.
    #[test]
    fn legato_ends_each_note_where_the_next_starts() {
        let notes = [n(0, 24, 60), n(96, 24, 62), n(192, 24, 64)];
        let target = on(&notes);
        assert_eq!(
            legato(&target, &notes).add,
            [n(0, 96, 60), n(96, 96, 62), n(192, 24, 64)]
        );
        let chord = [n(0, 24, 60), n(0, 24, 64), n(48, 10, 62)];
        assert_eq!(
            legato(&on(&chord), &chord).add,
            [n(0, 48, 60), n(0, 48, 64), n(48, 10, 62)]
        );
        let hits = [n(0, 0, 60), n(24, 0, 60), n(0, 96, 61), n(48, 24, 62)];
        assert_eq!(
            legato(&on(&hits), &hits).add,
            [n(0, 24, 60), n(0, 24, 61), n(24, 24, 60), n(48, 24, 62)]
        );
        let alone = [n(360, 24, 64)];
        assert!(legato(&on(&alone), &alone).is_empty());
        round_trip(&notes, legato);
        round_trip(&hits, legato);
    }

    // Span 0..192: A (0..48) → 144..192, B (96..192) → 0..96. A span from 100: 100..120 and
    // 130..140 → 120..140 and 100..110. Saturated: a step hit (a step long as drawn) and a
    // note ending on the pattern's end; two notes of one key ending together land together
    // and the longer stays.
    #[test]
    fn reversing_mirrors_notes_in_time_within_their_span() {
        let notes = [n(0, 48, 60), n(96, 96, 64)];
        let target = on(&notes);
        assert_eq!(
            reversed(&target, &notes, STEP).add,
            [n(0, 96, 64), n(144, 48, 60)]
        );
        let later = [n(100, 20, 60), n(130, 10, 62)];
        assert_eq!(
            reversed(&on(&later), &later, STEP).add,
            [n(100, 10, 62), n(120, 20, 60)]
        );
        let end = [n(300, 0, 62), n(360, 24, 60)];
        assert_eq!(
            reversed(&on(&end), &end, STEP).add,
            [n(300, 24, 60), n(360, 0, 62)]
        );
        let together = [n(0, 48, 60), n(24, 24, 60)];
        assert_eq!(
            reversed(&on(&together), &together, STEP).add,
            [n(0, 48, 60)]
        );
        assert_eq!(reversed(&target, &[], STEP), Change::default());
        round_trip(&notes, |target, notes| reversed(target, notes, STEP));
        round_trip(&end, |target, notes| reversed(target, notes, STEP));
    }

    // Keys 60, 64, 67 → 67, 63, 60. Two keys swap; one key alone stays. Saturated: keys 0 and
    // 127 swap.
    #[test]
    fn inverting_mirrors_keys_within_their_span() {
        let notes = [n(0, 24, 60), n(24, 24, 64), n(48, 24, 67)];
        let target = on(&notes);
        assert_eq!(
            inverted(&target, &notes).add,
            [n(0, 24, 67), n(24, 24, 63), n(48, 24, 60)]
        );
        let two = [n(0, 24, 60), n(24, 24, 62)];
        assert_eq!(inverted(&on(&two), &two).add, [n(0, 24, 62), n(24, 24, 60)]);
        let one = [n(0, 24, 60), n(48, 24, 60)];
        assert!(inverted(&on(&one), &one).is_empty());
        let extremes = [nv(0, 24, 0, 100), nv(0, 24, 127, 64)];
        assert_eq!(
            inverted(&on(&extremes), &extremes).add,
            [nv(0, 24, 0, 64), nv(0, 24, 127, 100)]
        );
        round_trip(&notes, inverted);
        round_trip(&extremes, inverted);
    }

    // The offsets come from tests/fixtures-independent arithmetic: SplitMix64 in Python
    // (seed 0: (−4, +2), (+1, −4), (+5, +2), (−4, −8); seed 1: (+4, +1), (−3, −5)), the
    // first of each pair ticks, the second velocity steps. Seed 0: A at 0 stays at 0 (never
    // before), 100 → 102; B 96 → 97, 64 → 60. Seed 1: A → 4, 101; B → 93, 59.
    #[test]
    fn humanizing_moves_and_shades_notes_by_the_seed() {
        let notes = [nv(0, 48, 60, 100), nv(96, 96, 64, 64)];
        let target = on(&notes);
        assert_eq!(
            humanized(&target, &notes, 0, STEP).add,
            [nv(0, 48, 60, 102), nv(97, 96, 64, 60)]
        );
        assert_eq!(
            humanized(&target, &notes, 1, STEP).add,
            [nv(4, 48, 60, 101), nv(93, 96, 64, 59)]
        );
        assert_eq!(
            humanized(&target, &notes, 1, STEP),
            humanized(&target, &notes, 1, STEP),
            "the same seed, the same notes"
        );
        round_trip(&notes, |target, notes| humanized(target, notes, 1, STEP));
    }

    // Saturated, seed 0 over four notes: 2 − 4 stops at 0 and 128 + 2 at 128; a step hit at 24
    // goes to 25 and 0 − 4 stops at 0; a note ending on the pattern's end (378..384) may not
    // pass it (+5 refused) and 100 + 2 = 102; 100 − 4 = 96 and 4 − 8 stops at 0.
    #[test]
    fn humanizing_stays_inside_the_pattern_and_the_velocity_scale() {
        let notes = [
            nv(2, 24, 60, 128),
            nv(24, 0, 61, 0),
            nv(378, 6, 62, 100),
            nv(100, 24, 63, 4),
        ];
        let target = on(&notes);
        let change = humanized(&target, &notes, 0, STEP);
        assert_eq!(
            change.add,
            [
                nv(0, 24, 60, 128),
                nv(25, 0, 61, 0),
                nv(96, 24, 63, 0),
                nv(378, 6, 62, 102)
            ]
        );
        assert!(matches!(
            change.into_edit(&target),
            Some(Edit::ReplaceNotes { .. })
        ));
        round_trip(&notes, |target, notes| humanized(target, notes, 0, STEP));
    }

    // From (0, 32) to (192, 128): 0 → 32, 96 → 80, 192 → 128; drawn the other way the same
    // line gives the reverse. A line one tick wide sets its notes to its end's value; notes
    // outside the line stay. Rounding: 128·1/100 = 1.28 → 1, 128·99/100 = 126.72 → 127.
    #[test]
    fn a_ramp_sets_velocities_along_its_line() {
        let notes = [
            nv(0, 24, 60, 100),
            nv(96, 24, 62, 100),
            nv(192, 24, 64, 100),
        ];
        let target = on(&notes);
        assert_eq!(
            ramped(&target, &notes, (Ticks(0), 32), (Ticks(192), 128)).add,
            [nv(0, 24, 60, 32), nv(96, 24, 62, 80), nv(192, 24, 64, 128)]
        );
        assert_eq!(
            ramped(&target, &notes, (Ticks(192), 32), (Ticks(0), 128)).add,
            [nv(0, 24, 60, 128), nv(96, 24, 62, 80), nv(192, 24, 64, 32)]
        );
        let point = ramped(&target, &notes, (Ticks(96), 50), (Ticks(96), 120));
        assert_eq!(point.add, [nv(96, 24, 62, 120)]);
        assert_eq!(point.remove, [nv(96, 24, 62, 100)]);
        let fine = [nv(1, 0, 60, 7), nv(50, 0, 60, 7), nv(99, 0, 60, 7)];
        assert_eq!(
            ramped(&on(&fine), &fine, (Ticks(0), 0), (Ticks(100), 128)).add,
            [nv(1, 0, 60, 1), nv(50, 0, 60, 64), nv(99, 0, 60, 127)]
        );
        let missed = ramped(&target, &notes, (Ticks(200), 0), (Ticks(300), 128));
        assert!(missed.is_empty());
        round_trip(&notes, |target, notes| {
            ramped(target, notes, (Ticks(0), 32), (Ticks(192), 128))
        });
    }

    #[test]
    fn painted_stems_take_each_its_own_velocity() {
        let notes = [nv(0, 24, 60, 100), nv(96, 24, 62, 100)];
        let target = on(&notes);
        let painted = [(notes[0].clone(), 20), (notes[1].clone(), 128)];
        assert_eq!(
            with_velocities(&target, &painted).add,
            [nv(0, 24, 60, 20), nv(96, 24, 62, 128)]
        );
        assert!(with_velocities(&target, &[(notes[0].clone(), 100)]).is_empty());
        assert!(with_velocities(&target, &[]).is_empty());
        round_trip(&notes, |target, notes| {
            with_velocities(target, &[(notes[1].clone(), 0)])
        });
    }

    // A and B copied span 0..192 and paste at 192 as 192..240 and 288..384. The kicks (channel
    // 2, step hits at 192 and 288) copied span 0..120 and paste onto channel 1 from 24 at their
    // own key. Saturated: pasted at 336 they reach past the end, so the pattern grows; a paste
    // onto a note of the same tick and key replaces it.
    #[test]
    fn a_paste_puts_the_copied_notes_on_the_channel_from_a_tick() {
        let notes = [n(0, 48, 60), nv(96, 96, 64, 64)];
        let target = on(&notes);
        let clip = Clip::of(&notes, STEP).expect("notes");
        assert_eq!((clip.span, clip.len()), (Ticks(192), 2));
        assert_eq!(
            pasted(&target, &clip, Ticks(192)).add,
            [n(192, 48, 60), nv(288, 96, 64, 64)]
        );
        let kicks = [
            Note {
                channel: ChannelId(2),
                ..nv(192, 0, 60, 128)
            },
            Note {
                channel: ChannelId(2),
                ..nv(288, 0, 60, 128)
            },
        ];
        let clip_of_kicks = Clip::of(&kicks, STEP).expect("notes");
        assert_eq!(clip_of_kicks.span, Ticks(120));
        assert_eq!(
            pasted(&target, &clip_of_kicks, Ticks(24)).add,
            [nv(24, 0, 60, 128), nv(120, 0, 60, 128)]
        );
        let grown = pasted(&target, &clip, Ticks(336)).into_edit(&target);
        assert!(matches!(grown, Some(Edit::Transaction(_))), "{grown:?}");
        let onto = pasted(&target, &clip, Ticks(0));
        assert_eq!(onto.remove, notes, "each lands on itself and replaces it");
        assert!(onto.is_empty(), "pasted over themselves, nothing changes");
        let shifted = pasted(&target, &clip, Ticks(96));
        assert!(
            shifted.remove.is_empty(),
            "A's copy at 96 is on key 60, B on 64"
        );
        assert_eq!(Clip::of(&[], STEP), None);
        round_trip(&notes, |target, _| pasted(target, &clip, Ticks(192)));
        round_trip(&notes, |target, _| pasted(target, &clip, Ticks(336)));
    }
}
