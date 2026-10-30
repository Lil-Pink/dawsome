//! Patterns made, renamed and deleted: each one edit, one undo step, whose inverse gives the
//! document back byte for byte. Expected documents are the fixtures with the change written out
//! by hand, never the engine's own output.

use std::collections::BTreeSet;

use dawsome_engine::edit::{self, Edit, EditError, Touched};
use dawsome_engine::project::{
    ClipId, LoadError, Pattern, PatternId, Project, ValidationError, MAX_PATTERNS,
};
use dawsome_engine::time::MilliBpm;

const F1: &str = include_str!("../../../tests/fixtures/f1_four_on_floor.json");
const P2_THREE: &str = include_str!("../../../tests/fixtures/p2_three_patterns.json");
const P4_TWO_CLIPS: &str = include_str!("../../../tests/fixtures/p4_two_clips.json");

/// Pattern 3 of p2_three_patterns.json, the last in its list, as the file writes it.
const P2_PATTERN_3: &str = r#"{"id":3,"name":"Pattern 3","length_bars":1,"notes":[]}"#;
/// Pattern 2 of p2_three_patterns.json, as the file writes it.
const P2_PATTERN_2: &str = concat!(
    r#"{"id":2,"name":"Pattern 2","length_bars":1,"notes":["#,
    r#"{"channel":1,"tick":48,"length":0,"velocity":1.0},"#,
    r#"{"channel":1,"tick":144,"length":0,"velocity":1.0}]}"#
);

fn load(text: &str) -> Project {
    Project::load_json(text).expect("fixture loads")
}

fn json(project: &Project) -> String {
    project.to_json().expect("serializes")
}

fn add(id: u32, name: &str, bars: u32) -> Edit {
    Edit::AddPattern {
        id: PatternId(id),
        name: name.to_owned(),
        bars,
        copied_from: None,
    }
}

fn touched_pattern(id: u32) -> Touched {
    Touched {
        patterns: BTreeSet::from([PatternId(id)]),
        ..Touched::default()
    }
}

/// Applies `edit`, checks the document against `expected`, then that its inverse gives `before`
/// back byte for byte and that the inverse of that (the redo) gives `expected` again.
fn walks(project: &mut Project, edit: Edit, expected: &str) -> Touched {
    let before = json(project);
    let applied = edit::apply(project, edit).expect("applies");
    assert_eq!(json(project), expected);
    let undone = edit::apply(project, applied.inverse).expect("undoes");
    assert_eq!(json(project), before, "undo restores the bytes");
    edit::apply(project, undone.inverse).expect("redoes");
    assert_eq!(json(project), expected, "redo makes it again");
    applied.touched
}

// "Next" past the last pattern: one above the highest ID, empty, after the others.
#[test]
fn a_new_pattern_takes_the_next_id_and_its_undo_gives_the_id_back() {
    let mut project = load(P2_THREE);
    assert_eq!(project.next_pattern_id(), Some(PatternId(4)));
    let with_four = json(&project).replace(
        P2_PATTERN_3,
        &format!(r#"{P2_PATTERN_3},{{"id":4,"name":"Pattern 4","length_bars":1,"notes":[]}}"#),
    );
    let touched = walks(&mut project, add(4, "Pattern 4", 1), &with_four);
    assert_eq!(touched, touched_pattern(4));
    assert_eq!(project.next_pattern_id(), Some(PatternId(5)));
    let undo = edit::apply(&mut project, Edit::UnaddPattern { id: PatternId(4) })
        .expect("the newest, as added");
    assert_eq!(undo.inverse, add(4, "Pattern 4", 1));
    assert_eq!(project.next_pattern_id(), Some(PatternId(4)), "free again");

    let mut none = Project::new(44_100, MilliBpm(120_000), 96);
    assert_eq!(none.next_pattern_id(), Some(PatternId(1)), "the first is 1");
    edit::apply(&mut none, add(1, "Pattern 1", 2)).expect("adds the first");
    assert_eq!(
        json(&none),
        concat!(
            r#"{"format_version":2,"sample_rate":44100,"tempo_mbpm":120000,"ppq":96,"#,
            r#""next_channel_id":1,"channels":[],"#,
            r#""patterns":[{"id":1,"name":"Pattern 1","length_bars":2,"notes":[]}]}"#
        )
    );
}

// A deleted pattern's ID is never given out again. Deleting the newest pattern writes the next
// ID down, before the patterns, so "next" past the last makes pattern 4, not a second pattern
// 3; once pattern 4 exists the patterns say it again and it is no longer written.
#[test]
fn a_deleted_patterns_id_is_not_given_out_again() {
    let mut project = load(P2_THREE);
    let without_three = json(&project)
        .replace(&format!(",{P2_PATTERN_3}"), "")
        .replace(r#""patterns":["#, r#""next_pattern_id":4,"patterns":["#);
    walks(
        &mut project,
        Edit::DeletePattern { id: PatternId(3) },
        &without_three,
    );
    assert_eq!(project.next_pattern_id(), Some(PatternId(4)));
    let before = json(&project);
    assert_eq!(
        edit::apply(&mut project, add(3, "Pattern 3", 1)),
        Err(EditError::NotNextPatternId {
            got: 3,
            expected: 4
        })
    );
    assert_eq!(json(&project), before);
    let with_four = before.replace(r#""next_pattern_id":4,"#, "").replace(
        P2_PATTERN_2,
        &format!(r#"{P2_PATTERN_2},{{"id":4,"name":"Pattern 4","length_bars":1,"notes":[]}}"#),
    );
    walks(&mut project, add(4, "Pattern 4", 1), &with_four);
    let reopened = load(&without_three);
    assert_eq!(
        json(&reopened),
        without_three,
        "the written ID loads and saves"
    );
    assert_eq!(reopened.next_pattern_id(), Some(PatternId(4)));
}

// Only the pattern added last can be taken out again: the ID it gives back is then the one the
// counter stood at before its add. After adding 4 and 5 and deleting 4, taking out 5 writes
// the counter down at 5 and its add puts the document back byte for byte. Taking out 3 there
// is refused: it would give back 3 and make 4, a deleted pattern's ID, the next but one, and
// the add that undid it would leave the counter at 4, not the 5 that was written.
#[test]
fn only_the_pattern_added_last_can_be_taken_out() {
    let mut project = load(P2_THREE);
    edit::apply(&mut project, add(4, "Pattern 4", 1)).expect("adds 4");
    edit::apply(&mut project, add(5, "Pattern 5", 1)).expect("adds 5");
    edit::apply(&mut project, Edit::DeletePattern { id: PatternId(4) }).expect("deletes 4");
    let counted_to_five =
        json(&load(P2_THREE)).replace(r#""patterns":["#, r#""next_pattern_id":5,"patterns":["#);
    walks(
        &mut project,
        Edit::UnaddPattern { id: PatternId(5) },
        &counted_to_five,
    );
    assert_eq!(project.next_pattern_id(), Some(PatternId(5)));
    assert_eq!(
        edit::apply(&mut project, Edit::UnaddPattern { id: PatternId(3) }),
        Err(EditError::PatternNotRemovable(3))
    );
    assert_eq!(json(&project), counted_to_five);
}

// A written next_pattern_id must be above every pattern ID, or a new pattern could reuse one.
#[test]
fn a_written_next_pattern_id_not_above_the_patterns_is_refused() {
    let text = P2_THREE
        .trim_end()
        .replace(r#""patterns":["#, r#""next_pattern_id":3,"patterns":["#);
    assert!(matches!(
        Project::load_json(&text),
        Err(LoadError::Invalid(ValidationError::UnallocatedPatternId {
            id: 3,
            next: 3
        }))
    ));
}

// ID u32::MAX is never given out, so the next ID can always be written down: with the highest
// pattern at u32::MAX − 1 no pattern can be added, and deleting that pattern keeps it so.
#[test]
fn the_last_pattern_id_is_never_given_out() {
    let high = u32::MAX - 1;
    let text = P2_THREE.replace(
        P2_PATTERN_3,
        &P2_PATTERN_3.replace(r#""id":3"#, &format!(r#""id":{high}"#)),
    );
    let mut project = load(&text);
    assert_eq!(project.next_pattern_id(), None);
    assert_eq!(
        edit::apply(&mut project, add(u32::MAX, "Pattern", 1)),
        Err(EditError::PatternIdsExhausted)
    );
    edit::apply(
        &mut project,
        Edit::DeletePattern {
            id: PatternId(high),
        },
    )
    .expect("deletes");
    assert_eq!(project.next_pattern_id(), None, "still used up");
    let written = format!(r#""next_pattern_id":{},"patterns""#, u32::MAX);
    assert!(json(&project).contains(&written), "{}", json(&project));
}

#[test]
fn a_renamed_pattern_keeps_its_place_and_compiles_nothing() {
    let mut project = load(P2_THREE);
    let renamed = json(&project).replace(r#""name":"Pattern 2""#, r#""name":"Chorus""#);
    let edit = Edit::RenamePattern {
        id: PatternId(2),
        name: "Chorus".to_owned(),
    };
    let touched = walks(&mut project, edit, &renamed);
    assert!(touched.is_empty(), "a name plays no differently");
}

// A pattern deleted with its notes comes back under its own ID, between its neighbors.
#[test]
fn a_deleted_pattern_and_its_notes_come_back_with_undo() {
    let mut project = load(P2_THREE);
    let without_two = json(&project).replace(&format!("{P2_PATTERN_2},"), "");
    let touched = walks(
        &mut project,
        Edit::DeletePattern { id: PatternId(2) },
        &without_two,
    );
    assert_eq!(touched, touched_pattern(2));
}

// The clips of a pattern go in the same transaction, before it: one undo step for both.
#[test]
fn a_pattern_clips_play_is_deleted_with_its_clips_in_one_transaction() {
    let mut project = load(P4_TWO_CLIPS);
    let alone = edit::apply(&mut project, Edit::DeletePattern { id: PatternId(1) });
    assert_eq!(
        alone,
        Err(EditError::PatternInUse(1)),
        "clips 1 and 2 play it"
    );
    let pattern_1 = concat!(
        r#"{"id":1,"name":"Pattern 1","length_bars":1,"notes":["#,
        r#"{"channel":1,"tick":0,"length":0,"velocity":1.0},"#,
        r#"{"channel":1,"tick":96,"length":0,"velocity":1.0},"#,
        r#"{"channel":1,"tick":192,"length":0,"velocity":1.0},"#,
        r#"{"channel":1,"tick":288,"length":0,"velocity":1.0}]},"#
    );
    let clips = concat!(
        r#"{"id":1,"track":1,"start":0,"length":384,"kind":"pattern","pattern":1},"#,
        r#"{"id":2,"track":1,"start":768,"length":384,"kind":"pattern","pattern":1}"#
    );
    let expected = json(&project).replace(pattern_1, "").replace(clips, "");
    let both = Edit::Transaction(vec![
        Edit::DeleteClips {
            ids: vec![ClipId(1), ClipId(2)],
        },
        Edit::DeletePattern { id: PatternId(1) },
    ]);
    let touched = walks(&mut project, both, &expected);
    assert!(touched.playlist && touched.patterns == BTreeSet::from([PatternId(1)]));
}

// Every refusal leaves the document as it was.
#[test]
fn every_refused_pattern_edit_changes_nothing() {
    let cases = [
        (
            P2_THREE,
            add(3, "Pattern 3", 1),
            EditError::NotNextPatternId {
                got: 3,
                expected: 4,
            },
        ),
        (
            P2_THREE,
            add(5, "Pattern 5", 1),
            EditError::NotNextPatternId {
                got: 5,
                expected: 4,
            },
        ),
        (P2_THREE, add(4, "Pattern 4", 0), EditError::NoBars),
        // Not added last, and not empty.
        (
            P2_THREE,
            Edit::UnaddPattern { id: PatternId(2) },
            EditError::PatternNotRemovable(2),
        ),
        (
            F1,
            Edit::UnaddPattern { id: PatternId(1) },
            EditError::PatternNotRemovable(1),
        ),
        (
            P2_THREE,
            Edit::UnaddPattern { id: PatternId(9) },
            EditError::UnknownPattern(9),
        ),
        (
            F1,
            Edit::DeletePattern { id: PatternId(1) },
            EditError::LastPattern,
        ),
        (
            P2_THREE,
            Edit::DeletePattern { id: PatternId(9) },
            EditError::UnknownPattern(9),
        ),
        (
            P2_THREE,
            Edit::RestorePattern {
                id: PatternId(2),
                pattern: Pattern {
                    name: "Again".to_owned(),
                    length_bars: 1,
                    copied_from: None,
                    notes: Vec::new(),
                },
            },
            EditError::PatternExists(2),
        ),
        (
            P2_THREE,
            Edit::RenamePattern {
                id: PatternId(9),
                name: "Nowhere".to_owned(),
            },
            EditError::UnknownPattern(9),
        ),
    ];
    for (fixture, edit, error) in cases {
        let mut project = load(fixture);
        let before = json(&project);
        assert_eq!(
            edit::apply(&mut project, edit.clone()),
            Err(error),
            "{edit:?}"
        );
        assert_eq!(json(&project), before, "{edit:?} changed the project");
    }
}

// A restored pattern is checked like one loaded from a file: its notes must play channels that
// exist.
#[test]
fn a_restored_pattern_must_be_valid() {
    let mut project = load(P2_THREE);
    edit::apply(&mut project, Edit::DeletePattern { id: PatternId(2) }).expect("deletes");
    let before = json(&project);
    let mut pattern: Pattern = serde_json::from_str(
        r#"{"name":"Pattern 2","length_bars":1,"notes":[{"channel":7,"tick":0,"length":0,"velocity":1.0}]}"#,
    )
    .expect("parses");
    let restore = |pattern: Pattern| Edit::RestorePattern {
        id: PatternId(2),
        pattern,
    };
    assert!(matches!(
        edit::apply(&mut project, restore(pattern.clone())),
        Err(EditError::Invalid(_))
    ));
    assert_eq!(json(&project), before);
    pattern.notes.clear();
    edit::apply(&mut project, restore(pattern)).expect("an empty one is fine");
}

// A project gets at most 999 patterns by adding them. The cap counts patterns, not IDs: with
// one of the 999 deleted, the next pattern added is pattern 1000.
#[test]
fn a_project_holds_at_most_999_patterns() {
    assert_eq!(MAX_PATTERNS, 999);
    let mut project = load(F1);
    for id in 2..=999 {
        edit::apply(&mut project, add(id, &format!("Pattern {id}"), 1)).expect("adds");
    }
    let before = json(&project);
    assert_eq!(
        edit::apply(&mut project, add(1000, "Pattern 1000", 1)),
        Err(EditError::TooManyPatterns)
    );
    assert_eq!(json(&project), before);
    edit::apply(&mut project, Edit::DeletePattern { id: PatternId(2) }).expect("deletes");
    edit::apply(&mut project, add(1000, "Pattern 1000", 1)).expect("adds pattern 1000");
    assert_eq!(project.patterns.len(), 999);
}
