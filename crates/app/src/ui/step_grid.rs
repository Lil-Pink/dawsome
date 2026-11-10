//! The step grid: which steps are lit for each channel of one pattern. It is a view of the
//! pattern's notes, never a model of its own: a step is lit when a note starts on its tick.

use std::collections::BTreeSet;

use dawsome_engine::edit::Edit;
use dawsome_engine::project::{ChannelId, Note, PatternId, Project, DEFAULT_KEY};
use dawsome_engine::time::{self, Ticks, TimeSignature};

use crate::ui::layout;

/// The most steps a rack row holds: 512, the longest fixed length a step sequencer row offers.
pub const MAX_RACK_STEPS: u32 = 512;

/// The longest pattern the rack header's "+" makes in `signature`: the whole bars inside
/// [`MAX_RACK_STEPS`] (32 bars of 4/4, 42 of 3/4 or 6/8). The piano roll can still grow a
/// pattern past it.
pub fn max_step_bars(signature: TimeSignature) -> u32 {
    MAX_RACK_STEPS / signature.steps_per_bar().max(1)
}

/// Lit steps of one pattern, per channel, and the groups (beats) its row is drawn in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepGrid {
    steps: u32,
    group: u32,
    lit: BTreeSet<(ChannelId, u32)>,
}

impl StepGrid {
    /// The lit steps of `pattern`. The project must be valid: its ppq is then a positive
    /// multiple of the steps in a beat.
    pub fn new(project: &Project, pattern: Option<PatternId>) -> Self {
        let steps = step_count(project, pattern);
        let ticks_per_step = time::step_to_ticks(1, project.ppq).0;
        let lit = pattern
            .and_then(|id| project.patterns.get(&id))
            .into_iter()
            .flat_map(|shown| &shown.notes)
            .filter(|note| note.start.0 % ticks_per_step == 0)
            .filter_map(|note| {
                let step = u32::try_from(note.start.0 / ticks_per_step).ok()?;
                (step < steps).then_some((note.channel, step))
            })
            .collect();
        let group = layout::steps_per_group(signature_of(project, pattern));
        Self { steps, group, lit }
    }

    /// Steps in each row.
    pub fn steps(&self) -> u32 {
        self.steps
    }

    /// Steps in each group of a row: a beat of the time signature.
    pub fn group(&self) -> u32 {
        self.group
    }

    pub fn is_lit(&self, channel: ChannelId, step: u32) -> bool {
        self.lit.contains(&(channel, step))
    }
}

/// What starts on a step of a channel's row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepContent {
    Empty,
    /// One note, at any key: a step as the row writes it (at the default key, [`DEFAULT_KEY`]),
    /// a hit played in at another key, or one note of a melody.
    Plain,
    /// A chord of a melody: two notes or more, as the piano roll writes them; `default` when
    /// one of them is at the default key.
    Melody {
        default: bool,
    },
}

/// The notes `channel` starts on `step` of `pattern`.
fn notes_on(project: &Project, pattern: PatternId, channel: ChannelId, step: u32) -> Vec<&Note> {
    let tick = Ticks(i64::from(step) * time::step_to_ticks(1, project.ppq).0);
    project
        .patterns
        .get(&pattern)
        .into_iter()
        .flat_map(|shown| &shown.notes)
        .filter(|note| note.channel == channel && note.start == tick)
        .collect()
}

/// What `channel` starts on `step` of `pattern`.
pub fn step_content(
    project: &Project,
    pattern: PatternId,
    channel: ChannelId,
    step: u32,
) -> StepContent {
    let notes = notes_on(project, pattern, channel, step);
    match notes.len() {
        0 => StepContent::Empty,
        1 => StepContent::Plain,
        _ => StepContent::Melody {
            default: notes.iter().any(|note| note.key == DEFAULT_KEY),
        },
    }
}

/// What a press on `step` of `channel`'s row does to `pattern`: an empty step or a plain one
/// (one note, whatever its key) switches ([`Edit::ToggleStep`]); on a chord only its note at
/// the row's default key goes, never the whole chord; `None` for a chord without that key (the
/// piano roll changes it).
pub fn step_press(
    project: &Project,
    pattern: PatternId,
    channel: ChannelId,
    step: u32,
) -> Option<Edit> {
    match step_content(project, pattern, channel, step) {
        StepContent::Empty | StepContent::Plain => Some(Edit::ToggleStep {
            pattern,
            channel,
            step,
        }),
        StepContent::Melody { default: true } => {
            let remove = notes_on(project, pattern, channel, step)
                .into_iter()
                .filter(|note| note.key == DEFAULT_KEY)
                .cloned()
                .collect();
            Some(Edit::ReplaceNotes {
                pattern,
                remove,
                add: Vec::new(),
            })
        }
        StepContent::Melody { default: false } => None,
    }
}

/// The time signature `pattern`'s row counts in: its own, else the project's (the project's
/// without a pattern).
pub fn signature_of(project: &Project, pattern: Option<PatternId>) -> TimeSignature {
    pattern.map_or(project.time_signature, |id| project.pattern_signature(id))
}

/// Steps in a row for `pattern`: its time signature's steps a bar (16 in 4/4, 12 in 3/4 and
/// 6/8) times its bars, none without a pattern.
pub fn step_count(project: &Project, pattern: Option<PatternId>) -> u32 {
    let per_bar = signature_of(project, pattern).steps_per_bar();
    pattern
        .and_then(|id| project.patterns.get(&id))
        .map_or(0, |shown| shown.length_bars.saturating_mul(per_bar))
}

/// The step under `position`, wrapping at the pattern's `steps`; `None` when there are none.
pub fn step_at(position: Ticks, ppq: u16, steps: u32) -> Option<u32> {
    let step = u64::try_from(position.0 / time::step_to_ticks(1, ppq).0).ok()?;
    let wrapped = step.checked_rem(u64::from(steps))?;
    u32::try_from(wrapped).ok()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::project_file;

    const F1: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/f1_four_on_floor.json"
    );

    /// F1 with its only pattern and channel.
    fn f1() -> (Project, PatternId, ChannelId) {
        let project = project_file::open(Path::new(F1)).expect("F1 opens").project;
        let pattern = *project.patterns.keys().next().expect("a pattern");
        let channel = *project.channels.keys().next().expect("a channel");
        (project, pattern, channel)
    }

    fn lit_steps(grid: &StepGrid, channel: ChannelId) -> Vec<u32> {
        (0..grid.steps())
            .filter(|&step| grid.is_lit(channel, step))
            .collect()
    }

    // Known answers 11 of gen_p34_expected.py: a pattern's row has the time signature's steps a
    // bar (12 in 6/8 and 3/4, 14 in 7/8, 15 in 15/16), a note lights the step its tick is on
    // (144 in 6/8: step 6), the groups are a beat wide (2 in 6/8, 4 in 3/4, 1 in 15/16), and the
    // rack's "+" stops at the whole bars inside 512 steps (42 in 6/8, 32 in 4/4).
    #[test]
    fn the_rows_steps_and_groups_follow_the_time_signature() {
        let p34: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/p34_expected.json"
        )))
        .expect("p34_expected.json");
        let signature = |value: &serde_json::Value| {
            TimeSignature::new(
                value[0].as_u64().expect("beats") as u8,
                value[1].as_u64().expect("unit") as u8,
            )
            .expect("a signature")
        };
        for case in p34["step_grids"].as_array().expect("step grids") {
            let (mut project, pattern, channel) = f1();
            project.time_signature = signature(&case["signature"]);
            let shown = project.patterns.get_mut(&pattern).expect("pattern");
            shown.length_bars = case["bars"].as_u64().expect("bars") as u32;
            shown.notes.truncate(1);
            shown.notes[0].start = Ticks(case["tick"].as_i64().expect("a tick"));
            let grid = StepGrid::new(&project, Some(pattern));
            let want = |key: &str| case[key].as_u64().expect(key) as u32;
            assert_eq!(grid.steps(), want("steps"), "{}", project.time_signature);
            assert_eq!(lit_steps(&grid, channel), [want("lit")]);
            let groups = case["groups"].as_array().expect("groups");
            assert_eq!(grid.group(), groups[0].as_u64().expect("a group") as u32);
            assert_eq!(grid.steps() / grid.group(), groups.len() as u32);
        }
        for row in p34["table"].as_array().expect("table") {
            let sig = signature(&row["signature"]);
            let most = row["max_step_bars"].as_u64().expect("most") as u32;
            assert_eq!(max_step_bars(sig), most, "{sig}");
        }
    }

    // Known answers B 4 of gen_p35_expected.py: p35_pattern_34's pattern 2 has its own 3/4, so
    // its row has 3 * 16 / 4 = 12 steps in groups of 4; pattern 1 (the project's 4/4) 16. The
    // rack's "+" stops at 512 // 12 = 42 bars for it.
    #[test]
    fn a_patterns_own_signature_sets_its_steps() {
        let p35: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/p35_expected.json"
        )))
        .expect("p35_expected.json");
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/p35_pattern_34.json"
        ))
        .expect("the fixture");
        let project = Project::load_json(&text).expect("loads");
        for case in p35["b"]["steps"].as_array().expect("cases") {
            let pattern = PatternId(case["pattern"].as_u64().expect("a pattern") as u32);
            let grid = StepGrid::new(&project, Some(pattern));
            assert_eq!(
                u64::from(grid.steps()),
                case["steps"].as_u64().expect("steps")
            );
            assert_eq!(
                u64::from(grid.group()),
                case["group"].as_u64().expect("group")
            );
        }
        assert_eq!(
            max_step_bars(signature_of(&project, Some(PatternId(2)))),
            42
        );
    }

    // F1 lights steps 0, 4, 8 and 12 (ticks 0, 96, 192, 288 at 24 ticks per step).
    #[test]
    fn f1_lights_every_fourth_step() {
        let (project, pattern, channel) = f1();
        let grid = StepGrid::new(&project, Some(pattern));
        assert_eq!(grid.steps(), 16);
        assert_eq!(lit_steps(&grid, channel), [0, 4, 8, 12]);
    }

    #[test]
    fn notes_off_the_step_grid_or_past_the_pattern_are_not_steps() {
        let (mut project, pattern, channel) = f1();
        let notes = &mut project.patterns.get_mut(&pattern).expect("pattern").notes;
        notes[1].start = Ticks(97); // between steps 4 and 5
        notes[2].start = Ticks(384); // the first tick after a one-bar pattern
        let grid = StepGrid::new(&project, Some(pattern));
        assert_eq!(lit_steps(&grid, channel), [0, 12]);
    }

    #[test]
    fn rows_have_sixteen_steps_per_bar_and_none_without_a_pattern() {
        let (mut project, pattern, _) = f1();
        project
            .patterns
            .get_mut(&pattern)
            .expect("pattern")
            .length_bars = 3;
        assert_eq!(step_count(&project, Some(pattern)), 48);
        assert_eq!(step_count(&project, None), 0);
        assert_eq!(StepGrid::new(&project, None).steps(), 0);
    }

    // At ppq 96 a step is 24 ticks: tick 120 is step 5, tick 504 is step 21, which wraps to 5
    // in a 16-step pattern.
    #[test]
    fn the_playhead_step_wraps_at_the_pattern_length() {
        assert_eq!(step_at(Ticks(0), 96, 16), Some(0));
        assert_eq!(step_at(Ticks(120), 96, 16), Some(5));
        assert_eq!(step_at(Ticks(143), 96, 16), Some(5));
        assert_eq!(step_at(Ticks(504), 96, 16), Some(5));
        assert_eq!(step_at(Ticks(504), 96, 32), Some(21));
        assert_eq!(step_at(Ticks(0), 96, 0), None);
    }

    /// F1 with notes added on channel 1: (tick, key, length) each, velocity 0.5.
    fn f1_with(notes: &[(i64, u8, i64)]) -> (Project, PatternId, ChannelId) {
        let (mut project, pattern, channel) = f1();
        let shown = project.patterns.get_mut(&pattern).expect("pattern");
        for &(tick, key, length) in notes {
            shown.notes.push(Note {
                channel,
                start: Ticks(tick),
                length: Ticks(length),
                velocity: 0.5,
                key,
            });
        }
        shown
            .notes
            .sort_by_key(|note| (note.start, note.channel, note.key));
        (project, pattern, channel)
    }

    // C4: a press on an empty step switches it on and one on a plain step (F1's at tick 0)
    // off, as before; on a chord with the row's default key (60, C5) it removes that note only;
    // on a chord without it, nothing: the piano roll changes it. One note at another key is a
    // plain step and switches off.
    #[test]
    fn a_step_press_on_a_chord_removes_only_its_default_key() {
        let (project, pattern, channel) = f1();
        let toggle = |step| {
            Some(Edit::ToggleStep {
                pattern,
                channel,
                step,
            })
        };
        assert_eq!(step_press(&project, pattern, channel, 1), toggle(1));
        assert_eq!(step_press(&project, pattern, channel, 0), toggle(0));
        // A chord on step 0 (tick 0): F1's C5 step with E5 and G5 a beat long.
        let (chord, pattern, channel) = f1_with(&[(0, 64, 96), (0, 67, 96)]);
        let c5 = chord.patterns[&pattern].notes[0].clone();
        assert_eq!((c5.start, c5.key), (Ticks(0), 60));
        assert_eq!(
            step_press(&chord, pattern, channel, 0),
            Some(Edit::ReplaceNotes {
                pattern,
                remove: vec![c5],
                add: vec![],
            })
        );
        // A chord without C5 on step 2 (tick 48), and one E5 note on step 3 (tick 72).
        let (melody, pattern, channel) = f1_with(&[(48, 64, 24), (48, 67, 24), (72, 64, 0)]);
        assert_eq!(step_press(&melody, pattern, channel, 2), None);
        assert_eq!(
            step_press(&melody, pattern, channel, 3),
            Some(Edit::ToggleStep {
                pattern,
                channel,
                step: 3,
            })
        );
        assert_eq!(
            step_content(&melody, pattern, channel, 2),
            StepContent::Melody { default: false }
        );
        assert_eq!(
            step_content(&chord, pattern, channel, 0),
            StepContent::Melody { default: true }
        );
        assert_eq!(
            step_content(&project, pattern, channel, 0),
            StepContent::Plain
        );
        assert_eq!(
            step_content(&project, pattern, channel, 1),
            StepContent::Empty
        );
    }

    // A step holding exactly one note starting there is plain whatever
    // its key, so a press switches it off as before C4: a hit recorded at C4 (key 48, the
    // typing keyboard's Z) on step 1, one at key 36 (a pad's) on step 2. A note starting
    // between steps (tick 30, inside step 1's ticks 24 to 47) starts on no step: step 1 is
    // empty, and a press switches it on beside that note.
    #[test]
    fn a_step_holding_one_note_at_any_key_switches_off() {
        let (hits, pattern, channel) = f1_with(&[(24, 48, 24), (48, 36, 0)]);
        for step in [1, 2] {
            assert_eq!(
                step_content(&hits, pattern, channel, step),
                StepContent::Plain
            );
            assert_eq!(
                step_press(&hits, pattern, channel, step),
                Some(Edit::ToggleStep {
                    pattern,
                    channel,
                    step,
                })
            );
        }
        let (between, pattern, channel) = f1_with(&[(30, 48, 24)]);
        assert_eq!(
            step_content(&between, pattern, channel, 1),
            StepContent::Empty
        );
        assert_eq!(
            step_press(&between, pattern, channel, 1),
            Some(Edit::ToggleStep {
                pattern,
                channel,
                step: 1,
            })
        );
    }
}
