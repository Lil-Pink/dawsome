//! Piano-roll gestures as edits: pure functions from the pattern's notes and a gesture's ticks
//! and keys to the [`Change`] it makes, and from that to the one [`Edit`] that commits it. No
//! iced types, so every rule is unit-tested directly, and a drag is drawn from the same
//! `Change` its release commits.

use std::collections::BTreeSet;
use std::fmt;

use dawsome_engine::edit::{Edit, STEP_VELOCITY};
use dawsome_engine::project::{ChannelId, Note, PatternId, Project, MAX_KEY};
use dawsome_engine::time::{self, Ticks};

use crate::ui::piano_layout::{drawn_length, View};

/// A note's identity in a pattern: no two notes share a channel, tick and key.
pub type NoteKey = (ChannelId, Ticks, u8);

pub fn key_of(note: &Note) -> NoteKey {
    (note.channel, note.start, note.key)
}

/// The velocity scale the piano roll shows and edits: whole numbers 0..=128, velocity n/128,
/// the scale of the project-file note record.
pub const VELOCITY_STEPS: u8 = 128;

/// `velocity` on the 0..=128 scale, rounded.
pub fn velocity_steps(velocity: f32) -> u8 {
    let steps = (velocity * f32::from(VELOCITY_STEPS)).round();
    // Clamped into 0..=128 first, so the conversion is exact.
    steps.clamp(0.0, f32::from(VELOCITY_STEPS)) as u8
}

/// The velocity of `steps` on the 0..=128 scale.
pub fn velocity_of_steps(steps: u8) -> f32 {
    f32::from(steps.min(VELOCITY_STEPS)) / f32::from(VELOCITY_STEPS)
}

/// What positions snap to. Alt held while dragging snaps to single ticks whatever is chosen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Snap {
    /// The finest grid line drawn, which follows the zoom.
    Line,
    #[default]
    Step,
    Beat,
    Bar,
    /// Single ticks.
    None,
}

impl Snap {
    /// Every choice, in the order the pick list shows them.
    pub const ALL: [Snap; 5] = [Snap::Line, Snap::Step, Snap::Beat, Snap::Bar, Snap::None];

    /// The snap unit in ticks at `view`'s zoom.
    pub fn unit(self, view: &View) -> Ticks {
        match self {
            Snap::Line => view.line_unit(),
            Snap::Step => view.step,
            Snap::Beat => view.beat(),
            Snap::Bar => view.bar(),
            Snap::None => Ticks(1),
        }
    }
}

impl fmt::Display for Snap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Snap::Line => "Line",
            Snap::Step => "Step",
            Snap::Beat => "Beat",
            Snap::Bar => "Bar",
            Snap::None => "None",
        })
    }
}

/// `tick` moved down to a multiple of `unit` (at least 1).
pub fn snap_down(tick: Ticks, unit: Ticks) -> Ticks {
    let unit = unit.0.max(1);
    Ticks(tick.0.div_euclid(unit) * unit)
}

/// `tick` moved to the nearest multiple of `unit` (at least 1), half way rounding up.
pub fn snap_nearest(tick: Ticks, unit: Ticks) -> Ticks {
    let unit = unit.0.max(1);
    Ticks((tick.0 + unit / 2).div_euclid(unit) * unit)
}

/// The pattern a gesture edits, and the channel whose notes it draws and moves.
#[derive(Debug, Clone, Copy)]
pub struct Target<'a> {
    pub pattern: PatternId,
    pub length_bars: u32,
    pub ppq: u16,
    pub channel: ChannelId,
    /// Every note of the pattern, all channels, in the pattern's order.
    pub notes: &'a [Note],
}

impl<'a> Target<'a> {
    /// `channel`'s notes in `pattern` of `project`; `None` unless both exist.
    pub fn of(
        project: &'a Project,
        pattern: Option<PatternId>,
        channel: Option<ChannelId>,
    ) -> Option<Self> {
        let pattern = pattern?;
        let shown = project.patterns.get(&pattern)?;
        let channel = channel.filter(|id| project.channels.contains_key(id))?;
        Some(Self {
            pattern,
            length_bars: shown.length_bars,
            ppq: project.ppq,
            channel,
            notes: &shown.notes,
        })
    }

    /// The channel's notes, in the pattern's order (by tick, then key).
    pub fn channel_notes(&self) -> impl DoubleEndedIterator<Item = &'a Note> + Clone + 'a {
        let channel = self.channel;
        self.notes
            .iter()
            .filter(move |note| note.channel == channel)
    }

    /// The pattern's note with `key`, if there is one.
    pub fn note(&self, key: NoteKey) -> Option<&'a Note> {
        let (channel, start, key) = key;
        let index = self
            .notes
            .binary_search_by_key(&(start, channel, key), |note| {
                (note.start, note.channel, note.key)
            })
            .ok()?;
        self.notes.get(index)
    }

    /// The notes `keys` name that the pattern holds, in the pattern's order.
    pub fn notes_with(&self, keys: &BTreeSet<NoteKey>) -> Vec<Note> {
        self.notes
            .iter()
            .filter(|note| keys.contains(&key_of(note)))
            .cloned()
            .collect()
    }

    /// The pattern's length in ticks.
    pub fn end(&self) -> Ticks {
        time::bars_to_ticks(self.length_bars, self.ppq)
    }
}

/// What a gesture changes: the notes it takes out and the notes it puts in.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Change {
    pub remove: Vec<Note>,
    pub add: Vec<Note>,
}

impl Change {
    /// Takes out `remove` and puts in `add`. A note of the pattern on the channel, tick and key
    /// where an added note lands, and that is not taken out already, is taken out too: the
    /// added note replaces it.
    pub(crate) fn new(target: &Target, mut remove: Vec<Note>, add: Vec<Note>) -> Self {
        let mut taken: BTreeSet<NoteKey> = remove.iter().map(key_of).collect();
        for note in &add {
            if let Some(landed_on) = target.note(key_of(note)) {
                if taken.insert(key_of(landed_on)) {
                    remove.push(landed_on.clone());
                }
            }
        }
        Self { remove, add }
    }

    /// Whether applying it would leave the pattern as it is.
    pub fn is_empty(&self) -> bool {
        let sorted = |notes: &[Note]| {
            let mut notes = notes.to_vec();
            notes.sort_by_key(key_of);
            notes
        };
        sorted(&self.remove) == sorted(&self.add)
    }

    /// The keys of the notes it puts in.
    pub fn added_keys(&self) -> BTreeSet<NoteKey> {
        self.add.iter().map(key_of).collect()
    }

    /// The one edit that commits it, or `None` when it changes nothing. When an added note
    /// ends past the pattern (a zero-length note ends a tick after it starts), the pattern
    /// grows to the whole bars that hold it, in the same edit and so the same undo step. It
    /// never shrinks.
    pub fn into_edit(self, target: &Target) -> Option<Edit> {
        if self.is_empty() {
            return None;
        }
        let bar = time::bars_to_ticks(1, target.ppq).0.max(1);
        let needed = self
            .add
            .iter()
            .map(|note| note.start.0 + note.length.0.max(1))
            .max()
            .map(|end| (end + bar - 1).div_euclid(bar));
        let replace = Edit::ReplaceNotes {
            pattern: target.pattern,
            remove: self.remove,
            add: self.add,
        };
        match needed.and_then(|bars| u32::try_from(bars).ok()) {
            Some(bars) if bars > target.length_bars => Some(Edit::Transaction(vec![
                replace,
                Edit::SetPatternBars {
                    pattern: target.pattern,
                    bars,
                },
            ])),
            _ => Some(replace),
        }
    }
}

/// A new note of `length` at `tick` and `key` on the target channel, with the velocity a step
/// switched on gets (100 on the 0..=128 scale).
pub fn draw(target: &Target, tick: Ticks, key: u8, length: Ticks) -> Change {
    let note = Note {
        channel: target.channel,
        start: tick,
        length,
        velocity: STEP_VELOCITY,
        key,
    };
    Change::new(target, Vec::new(), vec![note])
}

/// How far a move drags its notes in time: the grabbed note's start, moved by the ticks the
/// cursor traveled, lands on the nearest multiple of `unit`.
pub fn move_offset(grabbed: &Note, dragged: Ticks, unit: Ticks) -> Ticks {
    let start = grabbed.start.0;
    Ticks(snap_nearest(Ticks(start + dragged.0), unit).0 - start)
}

/// `notes` moved by `ticks` and `keys`, held together: the offsets are clamped so that no note
/// starts before tick 0 or leaves keys 0..=127.
pub fn moved(target: &Target, notes: &[Note], ticks: Ticks, keys: i32) -> Change {
    let (Some(first), Some(lowest), Some(highest)) = (
        notes.iter().map(|note| note.start.0).min(),
        notes.iter().map(|note| i32::from(note.key)).min(),
        notes.iter().map(|note| i32::from(note.key)).max(),
    ) else {
        return Change::default();
    };
    let ticks = ticks.0.max(-first);
    let keys = keys.clamp(-lowest, i32::from(MAX_KEY) - highest);
    let add = notes
        .iter()
        .map(|note| Note {
            start: Ticks(note.start.0 + ticks),
            // The clamp keeps every moved key inside 0..=127.
            key: u8::try_from(i32::from(note.key) + keys).unwrap_or(note.key),
            ..note.clone()
        })
        .collect();
    Change::new(target, notes.to_vec(), add)
}

/// How far a resize moves the grabbed note's drawn end: by the ticks the cursor traveled, to
/// the nearest multiple of `unit`.
pub fn resize_offset(grabbed: &Note, dragged: Ticks, unit: Ticks, step: Ticks) -> Ticks {
    let end = grabbed.start.0 + drawn_length(grabbed, step).0;
    Ticks(snap_nearest(Ticks(end + dragged.0), unit).0 - end)
}

/// `notes` with `delta` added to each one's drawn length, each at least `unit` (and 1) long.
pub fn resized(target: &Target, notes: &[Note], delta: Ticks, unit: Ticks, step: Ticks) -> Change {
    if delta.0 == 0 {
        return Change::default();
    }
    let shortest = unit.0.max(1);
    let add = notes
        .iter()
        .map(|note| Note {
            length: Ticks((drawn_length(note, step).0 + delta.0).max(shortest)),
            ..note.clone()
        })
        .collect();
    Change::new(target, notes.to_vec(), add)
}

/// `notes` at velocity `steps`/128.
pub fn with_velocity(target: &Target, notes: &[Note], steps: u8) -> Change {
    let velocity = velocity_of_steps(steps);
    let add = notes
        .iter()
        .map(|note| Note {
            velocity,
            ..note.clone()
        })
        .collect();
    Change::new(target, notes.to_vec(), add)
}

/// `notes` taken out.
pub fn deleted(target: &Target, notes: &[Note]) -> Change {
    Change::new(target, notes.to_vec(), Vec::new())
}

/// Copies of `notes` one extent later: the span from the first start to the last drawn end,
/// rounded up to a multiple of `unit`. With bar snap and bar-aligned notes that is the
/// playlist's own rule, the exact extent.
pub fn duplicated(target: &Target, notes: &[Note], unit: Ticks, step: Ticks) -> Change {
    let first = notes.iter().map(|note| note.start.0).min();
    let last_end = notes
        .iter()
        .map(|note| note.start.0 + drawn_length(note, step).0)
        .max();
    let (Some(first), Some(last_end)) = (first, last_end) else {
        return Change::default();
    };
    let unit = unit.0.max(1);
    let offset = (last_end - first + unit - 1).div_euclid(unit) * unit;
    let add = notes
        .iter()
        .map(|note| Note {
            start: Ticks(note.start.0 + offset),
            ..note.clone()
        })
        .collect();
    Change::new(target, Vec::new(), add)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::piano_layout::DEFAULT_ZOOM;

    fn note(channel: u32, tick: i64, length: i64, velocity: f32, key: u8) -> Note {
        Note {
            channel: ChannelId(channel),
            start: Ticks(tick),
            length: Ticks(length),
            velocity,
            key,
        }
    }

    /// p3_roll's notes, in its order.
    fn p3_notes() -> Vec<Note> {
        vec![
            note(1, 0, 48, 0.78125, 60),
            note(1, 96, 96, 0.5, 64),
            note(2, 192, 0, 1.0, 60),
            note(2, 288, 0, 1.0, 60),
        ]
    }

    fn target(notes: &[Note]) -> Target<'_> {
        Target {
            pattern: PatternId(1),
            length_bars: 1,
            ppq: 96,
            channel: ChannelId(1),
            notes,
        }
    }

    fn view() -> View {
        View {
            top: 69,
            scroll: Ticks(0),
            zoom: DEFAULT_ZOOM,
            step: Ticks(24),
            rows: crate::ui::piano_layout::Rows::ALL,
        }
    }

    const STEP: Ticks = Ticks(24);

    // From gen_p3_expected.py: nearest rounds half up, down floors.
    #[test]
    fn snapping_rounds_half_up_or_floors() {
        assert_eq!(snap_nearest(Ticks(12), STEP), Ticks(24));
        assert_eq!(snap_nearest(Ticks(11), STEP), Ticks(0));
        assert_eq!(snap_nearest(Ticks(30), STEP), Ticks(24));
        assert_eq!(snap_down(Ticks(423), STEP), Ticks(408));
        assert_eq!(snap_down(Ticks(-5), STEP), Ticks(-24));
        assert_eq!(snap_nearest(Ticks(-13), STEP), Ticks(-24));
        assert_eq!(snap_nearest(Ticks(7), Ticks(1)), Ticks(7));
        assert_eq!(
            snap_down(Ticks(7), Ticks(0)),
            Ticks(7),
            "a unit of 0 counts as 1"
        );
    }

    #[test]
    fn snap_units_at_16_px_per_step() {
        let units = Snap::ALL.map(|snap| snap.unit(&view()).0);
        assert_eq!(units, [24, 24, 96, 384, 1]);
        assert_eq!(
            Snap::ALL.map(|snap| snap.to_string()),
            ["Line", "Step", "Beat", "Bar", "None"]
        );
    }

    #[test]
    fn velocity_shows_as_whole_steps_of_128() {
        assert_eq!(velocity_steps(0.78125), 100);
        assert_eq!(velocity_steps(0.5), 64);
        assert_eq!(velocity_steps(1.0), 128);
        assert_eq!(velocity_steps(0.003), 0);
        assert_eq!(velocity_of_steps(64), 0.5);
        assert_eq!(velocity_of_steps(100), STEP_VELOCITY);
        assert_eq!(velocity_of_steps(200), 1.0);
    }

    // (e): a note drawn at 408 ends at 432, past the 1-bar pattern, so the pattern grows to 2
    // bars in the same edit.
    #[test]
    fn drawing_past_the_end_grows_the_pattern_in_the_same_edit() {
        let notes = p3_notes();
        let target = target(&notes);
        let change = draw(&target, Ticks(408), 60, STEP);
        let drawn = note(1, 408, 24, 0.78125, 60);
        assert_eq!(
            change.into_edit(&target),
            Some(Edit::Transaction(vec![
                Edit::ReplaceNotes {
                    pattern: PatternId(1),
                    remove: vec![],
                    add: vec![drawn],
                },
                Edit::SetPatternBars {
                    pattern: PatternId(1),
                    bars: 2,
                },
            ]))
        );
        // Ending exactly on the pattern's end needs no growth.
        let inside = draw(&target, Ticks(360), 60, STEP).into_edit(&target);
        assert!(
            matches!(inside, Some(Edit::ReplaceNotes { .. })),
            "{inside:?}"
        );
    }

    // (a): grabbed at tick_at 12, released at 42: raw start 30, nearest step 24.
    #[test]
    fn a_move_snaps_the_grabbed_note_to_the_nearest_line() {
        let notes = p3_notes();
        assert_eq!(move_offset(&notes[0], Ticks(30), STEP), Ticks(24));
        // 40 is nearer 48 than 24: nearest, not floor.
        assert_eq!(move_offset(&notes[0], Ticks(40), STEP), Ticks(48));
        assert_eq!(move_offset(&notes[1], Ticks(-64), STEP), Ticks(-72));
        assert_eq!(move_offset(&notes[1], Ticks(-64), Ticks(1)), Ticks(-64));
    }

    #[test]
    fn moved_notes_stay_together_inside_the_timeline_and_keyboard() {
        let notes = p3_notes();
        let target = target(&notes);
        let lead = &notes[..2];
        let change = moved(&target, lead, Ticks(-500), 100);
        let placed: Vec<(i64, u8)> = change.add.iter().map(|n| (n.start.0, n.key)).collect();
        // The earliest note stops at tick 0; the highest key (64) stops at 127.
        assert_eq!(placed, [(0, 123), (96, 127)]);
        assert!(moved(&target, lead, Ticks(0), 0)
            .into_edit(&target)
            .is_none());
    }

    // (f1): B lands on A's tick and key, so A is taken out too.
    #[test]
    fn a_note_moved_onto_another_takes_its_place() {
        let notes = p3_notes();
        let target = target(&notes);
        let change = moved(&target, &notes[1..2], Ticks(-96), -4);
        assert_eq!(change.remove, [notes[1].clone(), notes[0].clone()]);
        assert_eq!(change.add, [note(1, 0, 96, 0.5, 60)]);
    }

    // (b): A's end at 48 dragged by 51 ticks: 99, nearest step 96, so 48 more ticks.
    #[test]
    fn a_resize_moves_every_end_by_the_grabbed_notes_snapped_change() {
        let notes = p3_notes();
        let target = target(&notes);
        let delta = resize_offset(&notes[0], Ticks(51), STEP, STEP);
        assert_eq!(delta, Ticks(48));
        // An end dragged to 111 is nearer 120 than 96: nearest, not floor.
        assert_eq!(resize_offset(&notes[0], Ticks(63), STEP, STEP), Ticks(72));
        let change = resized(&target, &notes[..2], delta, STEP, STEP);
        let lengths: Vec<i64> = change.add.iter().map(|n| n.length.0).collect();
        assert_eq!(lengths, [96, 144]);
        // Shortened past nothing, each note keeps one snap unit.
        let short = resized(&target, &notes[..2], Ticks(-200), STEP, STEP);
        let lengths: Vec<i64> = short.add.iter().map(|n| n.length.0).collect();
        assert_eq!(lengths, [24, 24]);
        // A step note is drawn 24 long; resizing it by a step makes it 48.
        let kick = resized(&target, &notes[2..3], STEP, STEP, STEP);
        assert_eq!(kick.add[0].length, Ticks(48));
        assert_eq!(
            resized(&target, &notes[..1], Ticks(0), STEP, STEP),
            Change::default()
        );
    }

    #[test]
    fn velocity_and_delete_change_only_the_notes_given() {
        let notes = p3_notes();
        let target = target(&notes);
        let quieter = with_velocity(&target, &notes[..1], 64);
        assert_eq!(quieter.add, [note(1, 0, 48, 0.5, 60)]);
        assert_eq!(quieter.remove, [notes[0].clone()]);
        assert!(with_velocity(&target, &notes[..1], 100).is_empty());
        let gone = deleted(&target, &notes[..2]);
        assert_eq!((gone.remove.len(), gone.add.len()), (2, 0));
    }

    // (d): A and B span 0..192, a whole number of steps: copies 192 later. Other units round
    // the span up: a beat keeps 192, a bar makes it 384; a lone step note moves one step.
    #[test]
    fn duplicates_land_one_extent_later_rounded_up_to_the_snap() {
        let notes = p3_notes();
        let target = target(&notes);
        let starts = |change: Change| change.add.iter().map(|n| n.start.0).collect::<Vec<_>>();
        assert_eq!(
            starts(duplicated(&target, &notes[..2], STEP, STEP)),
            [192, 288]
        );
        assert_eq!(
            starts(duplicated(&target, &notes[..2], Ticks(96), STEP)),
            [192, 288]
        );
        assert_eq!(
            starts(duplicated(&target, &notes[..2], Ticks(384), STEP)),
            [384, 480]
        );
        assert_eq!(
            starts(duplicated(&target, &notes[..2], Ticks(1), STEP)),
            [192, 288]
        );
        assert_eq!(starts(duplicated(&target, &notes[2..3], STEP, STEP)), [216]);
        assert_eq!(duplicated(&target, &[], STEP, STEP), Change::default());
        // (d2): the copies again, 384 and 480: past the end, so the pattern grows.
        let copies = duplicated(&target, &notes[..2], STEP, STEP).add;
        let again = duplicated(&target, &copies, STEP, STEP);
        assert!(matches!(
            again.into_edit(&target),
            Some(Edit::Transaction(_))
        ));
    }

    #[test]
    fn the_target_finds_notes_by_key() {
        let notes = p3_notes();
        let target = target(&notes);
        assert_eq!(target.note((ChannelId(2), Ticks(192), 60)), Some(&notes[2]));
        assert_eq!(target.note((ChannelId(1), Ticks(192), 60)), None);
        let lead: Vec<Ticks> = target.channel_notes().map(|n| n.start).collect();
        assert_eq!(lead, [Ticks(0), Ticks(96)]);
        assert_eq!(target.end(), Ticks(384));
        let keys = BTreeSet::from([key_of(&notes[1]), (ChannelId(1), Ticks(5), 60)]);
        assert_eq!(target.notes_with(&keys), [notes[1].clone()]);
    }
}
