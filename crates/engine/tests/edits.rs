//! Known answers (a) to (c) of tests/fixtures/gen_p2_expected.py: edits, undo and redo checked byte
//! for byte on the canonical JSON, and the incremental compiler sharing the event lists of
//! untouched patterns. Expected values come from tests/fixtures/gen_p2_expected.py, never from the
//! engine.
//!
//! Note edits as `ReplaceNotes` and `Transaction`, checked against the documents
//! tests/fixtures/gen_p3_expected.py wrote.

use std::collections::BTreeSet;
use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, Edit, EditError, History, Touched};
use dawsome_engine::plan::{NoteEvent, PlanUpdate, PlayMode};
use dawsome_engine::project::{
    ChannelId, Clip, ClipId, ClipItem, EffectId, InsertId, LoopRange, Note, PatternId,
    ProcessorRef, Project, TrackId, ValidationError,
};
use dawsome_engine::time::{self, MilliBpm, Ticks};
use sha2::{Digest, Sha256};

const F1: &str = include_str!("../../../tests/fixtures/f1_four_on_floor.json");
const F2: &str = include_str!("../../../tests/fixtures/f2_tempo_140_48k.json");
const F3: &str = include_str!("../../../tests/fixtures/f3_velocity_ramp.json");
const F4: &str = include_str!("../../../tests/fixtures/f4_saturated.json");
const P2_EMPTY: &str = include_str!("../../../tests/fixtures/p2_empty.json");
const P2_THREE: &str = include_str!("../../../tests/fixtures/p2_three_patterns.json");
const P3_ROLL: &str = include_str!("../../../tests/fixtures/p3_roll.json");
const P3_D_DUPLICATE: &str = include_str!("../../../tests/fixtures/p3_d_duplicate.json");
const P3_E_GROW: &str = include_str!("../../../tests/fixtures/p3_e_grow.json");
const P3_F1_REPLACE: &str = include_str!("../../../tests/fixtures/p3_f1_replace.json");
const P3_F1_OVERLAP: &str = include_str!("../../../tests/fixtures/p3_f1_overlap.json");

fn load(text: &str) -> Project {
    Project::load_json(text).expect("fixture loads")
}

fn json(project: &Project) -> String {
    project.to_json().expect("serializes")
}

fn toggle(channel: u32, step: u32) -> Edit {
    Edit::ToggleStep {
        pattern: PatternId(1),
        channel: ChannelId(channel),
        step,
    }
}

fn notes_json(project: &Project) -> String {
    serde_json::to_string(&project.patterns[&PatternId(1)].notes).expect("serializes")
}

#[test]
fn saving_and_loading_round_trips_every_fixture() {
    for text in [F1, F2, F3, F4, P2_EMPTY, P2_THREE] {
        let project = load(text);
        let saved = json(&project);
        let again = load(&saved);
        assert_eq!(again, project, "{saved}");
        assert_eq!(json(&again), saved);
    }
}

#[test]
fn current_format_fixtures_are_already_canonical() {
    for text in [P2_EMPTY, P2_THREE] {
        assert_eq!(json(&load(text)), text.trim_end());
    }
}

#[test]
fn a_toggling_step_5_on_an_empty_pattern_adds_one_note_at_tick_120() {
    let mut project = load(P2_EMPTY);
    edit::apply(&mut project, toggle(1, 5)).expect("toggles");
    assert_eq!(
        notes_json(&project),
        r#"[{"channel":1,"tick":120,"length":0,"velocity":0.78125}]"#
    );
}

#[test]
fn a_toggling_a_step_off_and_back_restores_identical_json() {
    let mut project = load(F1);
    let before = json(&project);
    let applied = edit::apply(&mut project, toggle(1, 4)).expect("toggles");
    assert_eq!(
        notes_json(&project),
        concat!(
            r#"[{"channel":1,"tick":0,"length":0,"velocity":1.0},"#,
            r#"{"channel":1,"tick":192,"length":0,"velocity":1.0},"#,
            r#"{"channel":1,"tick":288,"length":0,"velocity":1.0}]"#
        )
    );
    edit::apply(&mut project, applied.inverse).expect("inverse applies");
    assert_eq!(json(&project), before);
}

/// The first 16 hex digits of the SHA-256 of `text`.
fn hash16(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

// Regenerated when a fresh step's velocity became 100/128, and when a channel that plays notes
// came to be added at 9/16 (states 5 to 7, from gen_p2_expected.py (b)).
const STATE_HASHES: [&str; 8] = [
    "26b38dd281bb1913",
    "996eb4a36d13a176",
    "9105712418e3894a",
    "9f03fac4f9e779cc",
    "7d19445d412515f1",
    "7be66f0594d625a8",
    "8b79fc8b4d995b41",
    "ab487e7b526d8a8f",
];

const FINAL_DOC: &str = concat!(
    r#"{"format_version":2,"sample_rate":44100,"tempo_mbpm":140000,"ppq":96,"next_channel_id":3,"#,
    r#""channels":[{"id":1,"name":"kick","kind":"sampler","sample":"tests/fixtures/impulse.wav","volume":0.5,"pan":0.0,"mute":false},"#,
    r#"{"id":2,"name":"snare","kind":"sampler","sample":"tests/fixtures/impulse.wav","volume":0.5625,"pan":0.0,"mute":false}],"#,
    r#""patterns":[{"id":1,"name":"Pattern 1","length_bars":2,"notes":[{"channel":1,"tick":0,"length":0,"velocity":1.0},"#,
    r#"{"channel":1,"tick":48,"length":0,"velocity":0.78125},{"channel":2,"tick":96,"length":0,"velocity":0.78125},"#,
    r#"{"channel":1,"tick":192,"length":0,"velocity":1.0},{"channel":1,"tick":288,"length":0,"velocity":1.0}]}]}"#
);

#[test]
fn b_seven_edits_seven_undos_and_seven_redos_walk_the_same_eight_states() {
    let mut project = load(F1);
    // The generator writes sample paths relative to the repository root; F1 stores them
    // relative to its own folder. Same file, so align the text before hashing.
    project
        .channels
        .get_mut(&ChannelId(1))
        .expect("kick")
        .sample = "tests/fixtures/impulse.wav".into();
    let edits = [
        toggle(1, 2),
        toggle(1, 4),
        Edit::SetChannelVolume {
            channel: ChannelId(1),
            value: 0.5,
        },
        Edit::SetTempo {
            mbpm: MilliBpm(140_000),
        },
        Edit::AddSamplerChannel {
            id: ChannelId(2),
            name: "snare".into(),
            sample: "tests/fixtures/impulse.wav".into(),
        },
        toggle(2, 4),
        Edit::SetPatternBars {
            pattern: PatternId(1),
            bars: 2,
        },
    ];
    let state = |project: &Project| hash16(&json(project));
    let mut history = History::new();
    assert_eq!(state(&project), STATE_HASHES[0], "state 0");
    for (number, edit) in (1..).zip(edits) {
        history.apply(&mut project, edit).expect("edit applies");
        assert_eq!(state(&project), STATE_HASHES[number], "after edit {number}");
    }
    assert_eq!(json(&project), FINAL_DOC);
    for number in (0..7).rev() {
        let undone = history.undo(&mut project).expect("undo applies");
        assert!(undone.is_some(), "an edit to undo");
        assert_eq!(
            state(&project),
            STATE_HASHES[number],
            "undo to state {number}"
        );
    }
    assert_eq!(history.undo(&mut project), Ok(None));
    for (number, hash) in STATE_HASHES.iter().enumerate().skip(1) {
        let redone = history.redo(&mut project).expect("redo applies");
        assert!(redone.is_some(), "an edit to redo");
        assert_eq!(state(&project), *hash, "redo to state {number}");
    }
    assert_eq!(json(&project), FINAL_DOC);
}

#[test]
fn b_history_keeps_the_last_500_edits_and_a_new_edit_clears_redo() {
    let mut project = load(F1);
    let mut history = History::new();
    for _ in 0..501 {
        history.apply(&mut project, toggle(1, 1)).expect("toggles");
    }
    assert_eq!(history.undo_len(), 500);
    for _ in 0..500 {
        let undone = history.undo(&mut project).expect("undo applies");
        assert!(undone.is_some());
    }
    // The first edit fell off the end of the history, so the project stays as it left it.
    let mut after_first_edit = load(F1);
    edit::apply(&mut after_first_edit, toggle(1, 1)).expect("toggles");
    assert_eq!(project, after_first_edit);
    assert_eq!(
        history.undo(&mut project),
        Ok(None),
        "the 501st undo does nothing"
    );
    assert_eq!(project, after_first_edit);

    assert_eq!(history.redo_len(), 500);
    history.apply(&mut project, toggle(1, 2)).expect("toggles");
    assert_eq!(history.redo_len(), 0, "a new edit clears redo");
    assert_eq!(history.redo(&mut project), Ok(None));
}

// Each step keeps the name it was recorded with, whichever stack it is on: the undo side names
// the newest step to undo, the redo side the newest undone; a step recorded without a name has
// none; a new step clears the redo side's names with its edits.
#[test]
fn each_step_keeps_its_name_through_undo_and_redo() {
    let mut project = load(F1);
    let mut history = History::new();
    assert_eq!((history.undo_name(), history.redo_name()), (None, None));
    let inverse = edit::apply(&mut project, toggle(1, 1))
        .expect("toggles")
        .inverse;
    history.record_as(inverse, "Step");
    let inverse = edit::apply(&mut project, toggle(1, 2))
        .expect("toggles")
        .inverse;
    history.record_as(inverse, "Second step");
    assert_eq!(history.undo_name(), Some("Second step"));
    history.undo(&mut project).expect("undoes");
    assert_eq!(
        (history.undo_name(), history.redo_name()),
        (Some("Step"), Some("Second step"))
    );
    history.undo(&mut project).expect("undoes");
    assert_eq!(
        (history.undo_name(), history.redo_name()),
        (None, Some("Step"))
    );
    history.redo(&mut project).expect("redoes");
    assert_eq!(
        (history.undo_name(), history.redo_name()),
        (Some("Step"), Some("Second step"))
    );
    history.apply(&mut project, toggle(1, 3)).expect("toggles");
    assert_eq!((history.undo_name(), history.redo_name()), (None, None));
    history.undo(&mut project).expect("undoes");
    assert_eq!(
        (history.undo_name(), history.redo_name()),
        (Some("Step"), None)
    );
}

// A change folded into the newest step is taken back with it: two toggles of F1, the second
// folded into the first's step, are one step under the first's name; one undo gives F1 back
// exactly, one redo both toggles. A document saved between the two is dirty after the fold (a
// state that never was), and stays dirty undone and redone: the saved state is gone. With no step
// to fold into, the change is a step of its own.
#[test]
fn a_change_folded_into_the_newest_step_is_undone_with_it() {
    let mut project = load(F1);
    let mut both = load(F1);
    for step in [1, 2] {
        edit::apply(&mut both, toggle(1, step)).expect("toggles");
    }
    assert_ne!(json(&both), json(&load(F1)));
    let mut history = History::new();
    let first = edit::apply(&mut project, toggle(1, 1)).expect("toggles");
    history.record_as(first.inverse, "Drop");
    history.mark_saved();
    let second = edit::apply(&mut project, toggle(1, 2)).expect("toggles");
    history.fold_into_newest(second.inverse);
    assert_eq!(json(&project), json(&both));
    assert_eq!((history.undo_len(), history.undo_name()), (1, Some("Drop")));
    assert!(history.is_dirty(), "the fold is a new state");
    history.undo(&mut project).expect("undo applies");
    assert_eq!(json(&project), json(&load(F1)), "both taken back");
    assert_eq!(history.undo_len(), 0);
    assert!(history.is_dirty());
    history.redo(&mut project).expect("redo applies");
    assert_eq!(json(&project), json(&both), "both done again");
    assert!(history.is_dirty(), "the saved state is gone");

    let mut empty = History::new();
    let alone = edit::apply(&mut project, toggle(1, 3)).expect("toggles");
    empty.fold_into_newest(alone.inverse);
    assert_eq!((empty.undo_len(), empty.undo_name()), (1, None));
    empty.undo(&mut project).expect("undo applies");
    assert_eq!(json(&project), json(&both));
}

// The save marker. Dirty means the document is not in the state last saved; undoing back to that
// state makes it clean again, and a new edit after undoing past it leaves no way back.
#[test]
fn the_save_marker_follows_undo_and_redo() {
    let mut project = load(F1);
    let mut history = History::new();
    assert!(!history.is_dirty(), "a document as opened is clean");
    history.apply(&mut project, toggle(1, 1)).expect("toggles");
    assert!(history.is_dirty());
    history.undo(&mut project).expect("undo applies");
    assert!(!history.is_dirty(), "undone back to the opened state");
    history.redo(&mut project).expect("redo applies");
    assert!(history.is_dirty());

    history.mark_saved();
    assert!(!history.is_dirty());
    history.apply(&mut project, toggle(1, 2)).expect("toggles");
    assert!(history.is_dirty());
    history.undo(&mut project).expect("undo applies");
    assert!(!history.is_dirty(), "back at the saved state");
    history.undo(&mut project).expect("undo applies");
    assert!(history.is_dirty(), "before the saved state");
    history.redo(&mut project).expect("redo applies");
    assert!(!history.is_dirty(), "redo returns to the saved state");

    // A new edit here clears redo; the saved state stays reachable only by undo.
    history.undo(&mut project).expect("undo applies");
    history.apply(&mut project, toggle(1, 3)).expect("toggles");
    assert!(history.is_dirty());
    history.undo(&mut project).expect("undo applies");
    assert!(
        history.is_dirty(),
        "the saved state was the redo that the edit cleared"
    );
}

#[test]
fn a_document_marked_unsaved_stays_dirty_until_saved() {
    let mut project = load(F1);
    let mut history = History::new();
    history.mark_unsaved();
    assert!(history.is_dirty(), "a recovered document starts dirty");
    history.apply(&mut project, toggle(1, 1)).expect("toggles");
    history.undo(&mut project).expect("undo applies");
    assert!(history.is_dirty(), "undo never reaches a state on file");
    history.mark_saved();
    assert!(!history.is_dirty());
}

#[test]
fn a_save_point_the_history_forgot_is_never_reached_again() {
    let mut project = load(F1);
    let mut history = History::new();
    // Saved with an empty history; 501 edits push the first one out.
    for _ in 0..501 {
        history.apply(&mut project, toggle(1, 1)).expect("toggles");
    }
    for _ in 0..500 {
        history.undo(&mut project).expect("undo applies");
    }
    assert_eq!(history.undo_len(), 0);
    assert!(
        history.is_dirty(),
        "the empty history now means the state after the forgotten edit"
    );
}

#[test]
fn a_state_id_changes_with_every_edit_and_returns_with_undo() {
    let mut project = load(F1);
    let mut history = History::new();
    let start = history.state();
    history.apply(&mut project, toggle(1, 1)).expect("toggles");
    let first = history.state();
    assert_ne!(first, start);
    history.undo(&mut project).expect("undo applies");
    assert_eq!(history.state(), start);
    history.redo(&mut project).expect("redo applies");
    assert_eq!(history.state(), first);
}

// Two documents' histories never share a state id, so an id from one (a late autosave's) can
// never name a state of the other: neither at the start nor after the same edit.
#[test]
fn two_histories_never_share_a_state_id() {
    let (mut one, mut two) = (load(F1), load(F1));
    let (mut first, mut second) = (History::new(), History::new());
    assert_ne!(first.state(), second.state());
    first.apply(&mut one, toggle(1, 1)).expect("toggles");
    second.apply(&mut two, toggle(1, 1)).expect("toggles");
    assert_ne!(first.state(), second.state());
}

// An audio clip plays, muted or not, and validates when its channel exists and is an audio channel;
// one whose channel is gone, or is a sampler, is refused.
#[test]
fn an_audio_clip_names_an_existing_audio_channel() {
    let clip = |channel| Clip::of_audio(ChannelId(channel), TrackId(1), Ticks(0), Ticks(10));
    let mut project = load(F1);
    project.channels.insert(
        ChannelId(2),
        dawsome_engine::project::Channel::audio("click", "impulse.wav"),
    );
    project.next_channel_id = ChannelId(3);
    project.playlist.clips.insert(ClipId(1), clip(2));
    project.playlist.next_clip_id = ClipId(2);
    assert_eq!(project.validate(), Ok(()));
    project
        .playlist
        .clips
        .get_mut(&ClipId(1))
        .expect("clip 1")
        .muted = true;
    assert_eq!(project.validate(), Ok(()), "muted too");
    project.playlist.clips.insert(ClipId(1), clip(9));
    assert_eq!(
        project.validate(),
        Err(ValidationError::UnknownClipChannel {
            clip: 1,
            channel: 9
        })
    );
    project.playlist.clips.insert(ClipId(1), clip(1));
    assert_eq!(
        project.validate(),
        Err(ValidationError::NotAnAudioChannel {
            clip: 1,
            channel: 1
        })
    );
}

#[test]
fn a_channel_an_audio_clip_plays_cannot_be_removed() {
    let mut project = load(F1);
    let added = Edit::AddAudioClip {
        channel: ChannelId(2),
        name: "click".into(),
        sample: "click.wav".into(),
        clip: ClipId(1),
        track: TrackId(1),
        start: Ticks(0),
        length: Ticks(10),
    };
    edit::apply(&mut project, added).expect("channel 2 and clip 1 are next");
    let second = Clip::of_audio(ChannelId(2), TrackId(2), Ticks(0), Ticks(10));
    edit::apply(
        &mut project,
        Edit::PlaceClips {
            first: ClipId(2),
            clips: vec![second],
        },
    )
    .expect("a second clip of the channel places");
    let before = project.clone();
    assert_eq!(
        edit::apply(&mut project, Edit::RemoveChannel { id: ChannelId(2) }),
        Err(EditError::NotRemovable(2))
    );
    assert_eq!(project, before);
}

fn events(update: &PlanUpdate, pattern: u32) -> &Arc<[NoteEvent]> {
    update
        .plan()
        .pattern_events(PatternId(pattern))
        .expect("pattern compiled")
}

fn ticks(events: &[NoteEvent]) -> Vec<i64> {
    events.iter().map(|event| event.tick().0).collect()
}

#[test]
fn c_recompiling_after_an_edit_rebuilds_only_the_touched_pattern() {
    let mut project = load(P2_THREE);
    let mut compiler = Compiler::new();
    let mut compile = |project: &Project, touched: &Touched| {
        let silent = |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(vec![0.0]));
        compiler
            .compile(project, touched, PlayMode::Pattern(PatternId(1)), silent)
            .expect("in-memory samples load")
    };
    let before = compile(&project, &Touched::default());
    assert_eq!(ticks(events(&before, 1)), [0, 96, 192, 288]);

    let applied = edit::apply(&mut project, toggle(1, 2)).expect("toggles");
    assert_eq!(applied.touched.patterns, BTreeSet::from([PatternId(1)]));
    let after = compile(&project, &applied.touched);
    let pattern_1 = ticks(events(&after, 1));
    assert_eq!(pattern_1, [0, 48, 96, 192, 288]);
    let samples: Vec<u64> = pattern_1
        .iter()
        .map(|&tick| time::tick_to_sample(Ticks(tick), 44100, project.tempo, 96).expect("in range"))
        .collect();
    assert_eq!(samples, [0, 11025, 22050, 44100, 66150]);
    assert_eq!(ticks(events(&after, 2)), [48, 144]);
    assert!(events(&after, 3).is_empty());
    assert!(!Arc::ptr_eq(events(&before, 1), events(&after, 1)));
    assert!(Arc::ptr_eq(events(&before, 2), events(&after, 2)));
    assert!(Arc::ptr_eq(events(&before, 3), events(&after, 3)));

    // Channel and tempo edits recompile no pattern.
    let mut previous = after;
    let edits = [
        Edit::SetChannelVolume {
            channel: ChannelId(1),
            value: 0.5,
        },
        Edit::SetTempo {
            mbpm: MilliBpm(140_000),
        },
    ];
    for edit in edits {
        let applied = edit::apply(&mut project, edit.clone()).expect("applies");
        let next = compile(&project, &applied.touched);
        for pattern in 1..=3 {
            assert!(
                Arc::ptr_eq(events(&previous, pattern), events(&next, pattern)),
                "{edit:?} recompiled pattern {pattern}"
            );
        }
        previous = next;
    }
}

fn note(channel: u32, tick: i64, length: i64, velocity: f32, key: u8) -> Note {
    Note {
        channel: ChannelId(channel),
        start: Ticks(tick),
        length: Ticks(length),
        velocity,
        key,
    }
}

/// p3_roll's notes: lead A and B, the kicks on ticks 192 and 288.
fn lead_a() -> Note {
    note(1, 0, 48, 0.78125, 60)
}

fn lead_b() -> Note {
    note(1, 96, 96, 0.5, 64)
}

fn kick(tick: i64) -> Note {
    note(2, tick, 0, 1.0, 60)
}

fn replace(remove: Vec<Note>, add: Vec<Note>) -> Edit {
    Edit::ReplaceNotes {
        pattern: PatternId(1),
        remove,
        add,
    }
}

/// The text of a fixture file as `to_json` writes it (the files end with a newline).
fn canonical(text: &str) -> &str {
    text.trim_end()
}

#[test]
fn p3_fixtures_are_already_canonical() {
    for text in [
        P3_ROLL,
        P3_D_DUPLICATE,
        P3_E_GROW,
        P3_F1_REPLACE,
        P3_F1_OVERLAP,
    ] {
        assert_eq!(json(&load(text)), canonical(text));
    }
}

// (f1): B dragged onto A's tick and key. The gesture removes B and A (A is where B lands) and
// adds B there; one undo puts both back.
#[test]
fn f1_a_note_moved_onto_another_replaces_it_and_undo_restores_both() {
    let mut project = load(P3_ROLL);
    let mut history = History::new();
    let moved = note(1, 0, 96, 0.5, 60);
    history
        .apply(&mut project, replace(vec![lead_b(), lead_a()], vec![moved]))
        .expect("move applies");
    assert_eq!(json(&project), canonical(P3_F1_REPLACE));
    history.undo(&mut project).expect("undo applies");
    assert_eq!(json(&project), canonical(P3_ROLL));
    history.redo(&mut project).expect("redo applies");
    assert_eq!(json(&project), canonical(P3_F1_REPLACE));
}

#[test]
fn replace_notes_inverse_is_byte_identical_for_a_three_note_move() {
    let mut project = load(P3_ROLL);
    let originals = vec![lead_a(), lead_b(), kick(192)];
    let moved: Vec<Note> = originals
        .iter()
        .map(|note| Note {
            start: Ticks(note.start.0 + 24),
            key: note.key + 2,
            ..note.clone()
        })
        .collect();
    let applied = edit::apply(&mut project, replace(originals, moved)).expect("move applies");
    assert_eq!(applied.touched.patterns, BTreeSet::from([PatternId(1)]));
    project.validate().expect("moved notes stay sorted");
    let notes = &project.patterns[&PatternId(1)].notes;
    let keys: Vec<(i64, u32, u8)> = notes
        .iter()
        .map(|n| (n.start.0, n.channel.0, n.key))
        .collect();
    assert_eq!(
        keys,
        [(24, 1, 62), (120, 1, 66), (216, 2, 62), (288, 2, 60)]
    );
    edit::apply(&mut project, applied.inverse).expect("inverse applies");
    assert_eq!(json(&project), canonical(P3_ROLL));
}

// A note may overlap another on the same key (two voices); only the same start is refused.
#[test]
fn a_partial_overlap_on_one_key_is_valid_and_an_exact_duplicate_is_refused() {
    let mut project = load(P3_ROLL);
    edit::apply(
        &mut project,
        replace(vec![], vec![note(1, 24, 48, 0.78125, 60)]),
    )
    .expect("overlap applies");
    assert_eq!(json(&project), canonical(P3_F1_OVERLAP));

    let mut project = load(P3_ROLL);
    let refused = edit::apply(&mut project, replace(vec![], vec![lead_a()]));
    assert_eq!(
        refused,
        Err(EditError::NoteExists {
            tick: 0,
            channel: 1,
            key: 60
        })
    );
    assert_eq!(json(&project), canonical(P3_ROLL));
}

#[test]
fn every_refused_replace_notes_changes_nothing() {
    let quieter_a = note(1, 0, 48, 0.5, 60);
    let missing_a = EditError::NoteMissing {
        tick: 0,
        channel: 1,
        key: 60,
    };
    let cases = [
        (replace(vec![quieter_a], vec![]), missing_a.clone()),
        (replace(vec![lead_a(), lead_a()], vec![]), missing_a),
        (
            replace(vec![], vec![note(2, 192, 24, 1.0, 60)]),
            EditError::NoteExists {
                tick: 192,
                channel: 2,
                key: 60,
            },
        ),
        (
            replace(vec![], vec![note(1, 24, 0, 1.5, 60)]),
            EditError::InvalidNote,
        ),
        (
            replace(vec![], vec![note(1, 24, 0, f32::NAN, 60)]),
            EditError::InvalidNote,
        ),
        (
            replace(vec![], vec![note(1, 24, 0, 1.0, 128)]),
            EditError::InvalidNote,
        ),
        (
            replace(vec![], vec![note(1, 24, -1, 1.0, 60)]),
            EditError::InvalidNote,
        ),
        (
            replace(vec![], vec![note(1, -24, 0, 1.0, 60)]),
            EditError::InvalidNote,
        ),
        (
            replace(vec![], vec![note(9, 24, 0, 1.0, 60)]),
            EditError::UnknownChannel(9),
        ),
        (
            Edit::ReplaceNotes {
                pattern: PatternId(9),
                remove: vec![],
                add: vec![note(1, 24, 0, 1.0, 60)],
            },
            EditError::UnknownPattern(9),
        ),
        (replace(vec![], vec![]), EditError::NothingToChange),
        (
            // The removal is fine; the second added note collides with the first.
            replace(
                vec![lead_a()],
                vec![note(1, 24, 0, 1.0, 60), note(1, 24, 48, 0.5, 60)],
            ),
            EditError::NoteExists {
                tick: 24,
                channel: 1,
                key: 60,
            },
        ),
    ];
    for (edit, error) in cases {
        let mut project = load(P3_ROLL);
        assert_eq!(
            edit::apply(&mut project, edit.clone()),
            Err(error),
            "{edit:?}"
        );
        assert_eq!(json(&project), canonical(P3_ROLL), "{edit:?}");
    }
}

#[test]
fn a_transaction_whose_second_edit_fails_changes_nothing() {
    let mut project = load(P3_ROLL);
    let mut history = History::new();
    let transaction = Edit::Transaction(vec![
        replace(vec![], vec![note(1, 408, 24, 0.78125, 60)]),
        Edit::SetPatternBars {
            pattern: PatternId(1),
            bars: 0,
        },
    ]);
    assert_eq!(
        history.apply(&mut project, transaction),
        Err(EditError::NoBars)
    );
    assert_eq!(json(&project), canonical(P3_ROLL));
    assert_eq!(history.undo_len(), 0);
    assert_eq!(
        edit::apply(&mut project, Edit::Transaction(Vec::new())),
        Err(EditError::NothingToChange)
    );
}

// (e): drawing past the pattern's end adds the note and grows the pattern in one undo step.
#[test]
fn e_a_note_and_the_growth_it_needs_are_one_undo_step() {
    let mut project = load(P3_ROLL);
    let mut history = History::new();
    let grow = Edit::Transaction(vec![
        replace(vec![], vec![note(1, 408, 24, 0.78125, 60)]),
        Edit::SetPatternBars {
            pattern: PatternId(1),
            bars: 2,
        },
    ]);
    let touched = history.apply(&mut project, grow).expect("growth applies");
    assert_eq!(touched.patterns, BTreeSet::from([PatternId(1)]));
    assert_eq!(json(&project), canonical(P3_E_GROW));
    assert_eq!(history.undo_len(), 1);
    history.undo(&mut project).expect("undo applies");
    assert_eq!(json(&project), canonical(P3_ROLL));
    history.redo(&mut project).expect("redo applies");
    assert_eq!(json(&project), canonical(P3_E_GROW));
}

// (d): copies on channel 1 share ticks 192 and 288 with the kicks of channel 2.
#[test]
fn d_copies_may_share_a_tick_with_another_channel() {
    let mut project = load(P3_ROLL);
    let copies = vec![note(1, 192, 48, 0.78125, 60), note(1, 288, 96, 0.5, 64)];
    edit::apply(&mut project, replace(vec![], copies)).expect("copies apply");
    assert_eq!(json(&project), canonical(P3_D_DUPLICATE));
}

// The step grid lights a step for any note starting on it, so switching the step off removes
// every note of that channel on that tick, whatever its key; undo puts each one back.
#[test]
fn toggling_a_step_off_removes_every_key_on_it_and_undo_restores_them() {
    let mut project = load(P3_ROLL);
    edit::apply(
        &mut project,
        replace(vec![], vec![note(2, 192, 12, 0.25, 67)]),
    )
    .expect("second key applies");
    let before = json(&project);
    let mut history = History::new();
    history
        .apply(&mut project, toggle(2, 8))
        .expect("step 8 toggles off");
    let kicks: Vec<i64> = project.patterns[&PatternId(1)]
        .notes
        .iter()
        .filter(|n| n.channel == ChannelId(2))
        .map(|n| n.start.0)
        .collect();
    assert_eq!(kicks, [288]);
    history.undo(&mut project).expect("undo applies");
    assert_eq!(json(&project), before);
}

// ---------------------------------------------------------------------------------------------
// Playlist edits, each with its exact inverse, checked byte for byte on the canonical JSON of the
// p4 fixtures that tests/fixtures/gen_p4_expected.py wrote.
// ---------------------------------------------------------------------------------------------

const P4_FIXTURES: [&str; 7] = [
    include_str!("../../../tests/fixtures/p4_two_clips.json"),
    include_str!("../../../tests/fixtures/p4_resize.json"),
    include_str!("../../../tests/fixtures/p4_loop.json"),
    include_str!("../../../tests/fixtures/p4_loop_off.json"),
    include_str!("../../../tests/fixtures/p4_two_tracks.json"),
    include_str!("../../../tests/fixtures/p4_overlap.json"),
    include_str!("../../../tests/fixtures/p4_messy.json"),
];

/// p4_loop: pattern 1 at 0, pattern 2 at 384, pattern 1 at 768 (clips 1 to 3, track 1, 384
/// ticks each), 8 empty-but-for-track-1 tracks, loop 384..768 on.
const P4_LOOP: &str = P4_FIXTURES[2];

#[test]
fn p4_fixtures_round_trip_byte_for_byte() {
    for text in P4_FIXTURES {
        assert_eq!(json(&load(text)), canonical(text));
    }
}

fn pattern_clip(pattern: u32, track: u32, start: i64, length: i64) -> Clip {
    Clip::of_pattern(
        PatternId(pattern),
        TrackId(track),
        Ticks(start),
        Ticks(length),
    )
}

fn range(start: i64, end: i64, on: bool) -> LoopRange {
    LoopRange {
        start: Ticks(start),
        end: Ticks(end),
        on,
    }
}

/// One of every playlist edit that p4_loop accepts.
fn every_playlist_edit() -> Vec<Edit> {
    vec![
        Edit::PlaceClips {
            first: ClipId(4),
            clips: vec![pattern_clip(3, 2, 0, 384), pattern_clip(1, 8, 37, 250)],
        },
        Edit::UnplaceClips {
            first: ClipId(3),
            count: 1,
        },
        Edit::DuplicateClips {
            ids: vec![ClipId(2), ClipId(1)],
            first: ClipId(4),
        },
        Edit::DeleteClips {
            ids: vec![ClipId(3), ClipId(1)],
        },
        Edit::MoveClips {
            moves: vec![
                (ClipId(1), Ticks(1152), TrackId(2)),
                (ClipId(3), Ticks(0), TrackId(1)),
            ],
        },
        Edit::ResizeClip {
            id: ClipId(3),
            length: Ticks(96),
        },
        Edit::SetLoop { value: None },
        Edit::SetLoop {
            value: Some(range(0, 96, false)),
        },
        Edit::AddTrack {
            id: TrackId(9),
            name: "Track 9".into(),
        },
        Edit::RemoveTrack { id: TrackId(8) },
        Edit::RenameTrack {
            id: TrackId(1),
            name: "Drums".into(),
        },
        Edit::SetTrackMute {
            id: TrackId(2),
            mute: true,
        },
    ]
}

#[test]
fn every_playlist_edit_is_undone_byte_for_byte_and_touches_only_the_playlist() {
    let playlist_only = Touched {
        playlist: true,
        ..Touched::default()
    };
    for edit in every_playlist_edit() {
        let mut project = load(P4_LOOP);
        let applied = edit::apply(&mut project, edit.clone()).expect("applies");
        assert_ne!(
            json(&project),
            canonical(P4_LOOP),
            "{edit:?} changed nothing"
        );
        assert_eq!(applied.touched, playlist_only, "{edit:?}");
        project.validate().expect("the edited project is valid");
        let changed = json(&project);
        let undone = edit::apply(&mut project, applied.inverse).expect("inverse applies");
        assert_eq!(json(&project), canonical(P4_LOOP), "{edit:?} undone");
        // The inverse's own inverse redoes the edit exactly.
        edit::apply(&mut project, undone.inverse).expect("redo applies");
        assert_eq!(json(&project), changed, "{edit:?} redone");
    }
}

// Copies land one selection length later. Clips 1 (0..384) and 2 (384..768) span 0..768, so the
// copies start at 768 and 1152, on track 1, ids 4 and 5 in start order.
#[test]
fn duplicated_clips_land_one_selection_length_later() {
    let mut project = load(P4_LOOP);
    edit::apply(
        &mut project,
        Edit::DuplicateClips {
            ids: vec![ClipId(2), ClipId(1)],
            first: ClipId(4),
        },
    )
    .expect("duplicates");
    let copies = concat!(
        r#"{"id":4,"track":1,"start":768,"length":384,"kind":"pattern","pattern":1},"#,
        r#"{"id":5,"track":1,"start":1152,"length":384,"kind":"pattern","pattern":2}]"#
    );
    let saved = json(&project);
    assert!(saved.contains(copies), "{saved}");
    assert!(saved.contains(r#""next_clip_id":6"#), "{saved}");
}

#[test]
fn undoing_a_placement_or_a_new_track_gives_the_id_back() {
    let mut project = load(P4_LOOP);
    let mut history = History::new();
    let place = Edit::PlaceClips {
        first: ClipId(4),
        clips: vec![pattern_clip(2, 3, 1152, 384)],
    };
    history.apply(&mut project, place).expect("places");
    assert_eq!(project.playlist.next_clip_id, ClipId(5));
    history.undo(&mut project).expect("undo applies");
    assert_eq!(project.playlist.next_clip_id, ClipId(4));
    assert_eq!(json(&project), canonical(P4_LOOP));
    history.redo(&mut project).expect("redo applies");
    assert_eq!(
        project
            .playlist
            .clips
            .get(&ClipId(4))
            .map(|clip| clip.start),
        Some(Ticks(1152)),
        "the redone clip has the same id"
    );

    let mut project = load(P4_LOOP);
    let add = Edit::AddTrack {
        id: TrackId(9),
        name: "Track 9".into(),
    };
    history.apply(&mut project, add).expect("adds");
    assert_eq!(project.playlist.next_track_id, TrackId(10));
    history.undo(&mut project).expect("undo applies");
    assert_eq!(project.playlist.next_track_id, TrackId(9));
    assert_eq!(json(&project), canonical(P4_LOOP));
}

#[test]
fn every_refused_playlist_edit_changes_nothing() {
    use dawsome_engine::project::ValidationError as V;
    let max = 38_400_000;
    let audio = Clip {
        item: ClipItem::Audio {
            channel: ChannelId(1),
            frame_offset: 0,
            gain: 1.0,
            stretch: None,
        },
        ..pattern_clip(1, 1, 0, 384)
    };
    let cases = [
        (
            // The first move is fine, the second names a track that does not exist.
            Edit::MoveClips {
                moves: vec![
                    (ClipId(1), Ticks(384), TrackId(2)),
                    (ClipId(2), Ticks(0), TrackId(99)),
                ],
            },
            EditError::Invalid(V::UnknownTrack { clip: 2, track: 99 }),
        ),
        (
            Edit::MoveClips {
                moves: vec![
                    (ClipId(1), Ticks(0), TrackId(2)),
                    (ClipId(1), Ticks(9), TrackId(3)),
                ],
            },
            EditError::ClipListedTwice(1),
        ),
        (
            Edit::MoveClips {
                moves: vec![(ClipId(1), Ticks(-1), TrackId(1))],
            },
            EditError::Invalid(V::InvalidClipSpan {
                clip: 1,
                start: -1,
                length: 384,
                offset: 0,
                max,
            }),
        ),
        (
            Edit::PlaceClips {
                first: ClipId(3),
                clips: vec![pattern_clip(1, 1, 0, 384)],
            },
            EditError::NotNextClipId {
                got: 3,
                expected: 4,
            },
        ),
        (
            Edit::PlaceClips {
                first: ClipId(4),
                clips: vec![audio],
            },
            // Channel 1 is a sampler: an audio clip plays an audio channel's file.
            EditError::Invalid(V::NotAnAudioChannel {
                clip: 4,
                channel: 1,
            }),
        ),
        (
            Edit::PlaceClips {
                first: ClipId(4),
                clips: vec![pattern_clip(9, 1, 0, 384)],
            },
            EditError::Invalid(V::UnknownClipPattern {
                clip: 4,
                pattern: 9,
            }),
        ),
        (
            Edit::PlaceClips {
                first: ClipId(4),
                clips: vec![pattern_clip(1, 1, max - 383, 384)],
            },
            EditError::Invalid(V::InvalidClipSpan {
                clip: 4,
                start: max - 383,
                length: 384,
                offset: 0,
                max,
            }),
        ),
        (
            Edit::PlaceClips {
                first: ClipId(4),
                clips: vec![],
            },
            EditError::NothingToChange,
        ),
        (
            Edit::UnplaceClips {
                first: ClipId(1),
                count: 1,
            },
            EditError::NotNewestClips { first: 1, end: 2 },
        ),
        (
            Edit::DuplicateClips {
                ids: vec![ClipId(1), ClipId(1)],
                first: ClipId(4),
            },
            EditError::ClipListedTwice(1),
        ),
        (
            Edit::DuplicateClips {
                ids: vec![ClipId(9)],
                first: ClipId(4),
            },
            EditError::UnknownClip(9),
        ),
        (
            Edit::DuplicateClips {
                ids: vec![ClipId(1)],
                first: ClipId(5),
            },
            EditError::NotNextClipId {
                got: 5,
                expected: 4,
            },
        ),
        (
            Edit::DeleteClips {
                ids: vec![ClipId(2), ClipId(9)],
            },
            EditError::UnknownClip(9),
        ),
        (
            Edit::DeleteClips { ids: vec![] },
            EditError::NothingToChange,
        ),
        (
            Edit::RestoreClips {
                clips: vec![(ClipId(2), pattern_clip(1, 1, 0, 384))],
            },
            EditError::ClipExists(2),
        ),
        (
            Edit::ResizeClip {
                id: ClipId(1),
                length: Ticks(0),
            },
            EditError::Invalid(V::InvalidClipSpan {
                clip: 1,
                start: 0,
                length: 0,
                offset: 0,
                max,
            }),
        ),
        (
            Edit::ResizeClip {
                id: ClipId(9),
                length: Ticks(96),
            },
            EditError::UnknownClip(9),
        ),
        (
            Edit::SetLoop {
                value: Some(range(384, 407, true)),
            },
            EditError::Invalid(V::InvalidLoop {
                start: 384,
                end: 407,
                step: 24,
                max,
            }),
        ),
        (
            Edit::AddTrack {
                id: TrackId(10),
                name: "x".into(),
            },
            EditError::NotNextTrackId {
                got: 10,
                expected: 9,
            },
        ),
        (
            // Track 1 holds clips; track 2 is not the newest.
            Edit::RemoveTrack { id: TrackId(1) },
            EditError::TrackNotRemovable(1),
        ),
        (
            Edit::RemoveTrack { id: TrackId(2) },
            EditError::TrackNotRemovable(2),
        ),
        (
            Edit::RenameTrack {
                id: TrackId(99),
                name: "x".into(),
            },
            EditError::UnknownTrack(99),
        ),
        (
            Edit::SetTrackMute {
                id: TrackId(99),
                mute: true,
            },
            EditError::UnknownTrack(99),
        ),
    ];
    for (edit, error) in cases {
        let mut project = load(P4_LOOP);
        assert_eq!(
            edit::apply(&mut project, edit.clone()),
            Err(error),
            "{edit:?}"
        );
        assert_eq!(json(&project), canonical(P4_LOOP), "{edit:?}");
    }
}

#[test]
fn a_track_is_removable_only_while_unmuted_and_the_playlist_keeps_one() {
    let mut project = load(P4_LOOP);
    let mute = Edit::SetTrackMute {
        id: TrackId(8),
        mute: true,
    };
    edit::apply(&mut project, mute).expect("mutes");
    let before = json(&project);
    assert_eq!(
        edit::apply(&mut project, Edit::RemoveTrack { id: TrackId(8) }),
        Err(EditError::TrackNotRemovable(8))
    );
    assert_eq!(json(&project), before);

    let mut lone = load(P2_THREE);
    lone.playlist.tracks.retain(|&id, _| id == TrackId(1));
    lone.playlist.next_track_id = TrackId(2);
    lone.validate().expect("one track is valid");
    assert_eq!(
        edit::apply(&mut lone, Edit::RemoveTrack { id: TrackId(1) }),
        Err(EditError::TrackNotRemovable(1))
    );
}

#[test]
fn the_playlist_holds_at_most_500_tracks() {
    let mut project = load(P4_LOOP);
    for id in 9..=500 {
        let add = Edit::AddTrack {
            id: TrackId(id),
            name: format!("Track {id}"),
        };
        edit::apply(&mut project, add).expect("room for another track");
    }
    let before = json(&project);
    let one_more = Edit::AddTrack {
        id: TrackId(501),
        name: "Track 501".into(),
    };
    assert_eq!(
        edit::apply(&mut project, one_more),
        Err(EditError::TooManyTracks)
    );
    assert_eq!(json(&project), before);
}

// `Touched.playlist` never recompiles a pattern list.
#[test]
fn playlist_edits_recompile_no_pattern() {
    let silent = |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(vec![0.0]));
    for edit in every_playlist_edit() {
        let mut project = load(P4_LOOP);
        let mut compiler = Compiler::new();
        let before = compiler
            .compile(&project, &Touched::default(), PlayMode::Song, silent)
            .expect("in-memory samples load");
        let applied = edit::apply(&mut project, edit.clone()).expect("applies");
        let after = compiler
            .compile(&project, &applied.touched, PlayMode::Song, silent)
            .expect("in-memory samples load");
        for pattern in 1..=3 {
            assert!(
                Arc::ptr_eq(events(&before, pattern), events(&after, pattern)),
                "{edit:?} recompiled pattern {pattern}"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Mixer edits, each undone byte for byte, and the p5 fixtures tests/fixtures/gen_p5_expected.py
// wrote.
// ---------------------------------------------------------------------------------------------

/// Every p5 fixture that is a valid project (p5_cycle is not: it holds a routing loop).
const P5_FIXTURES: [&str; 13] = [
    include_str!("../../../tests/fixtures/p5_pan.json"),
    include_str!("../../../tests/fixtures/p5_gain.json"),
    include_str!("../../../tests/fixtures/p5_mute.json"),
    include_str!("../../../tests/fixtures/p5_solo.json"),
    include_str!("../../../tests/fixtures/p5_eq_f0.json"),
    include_str!("../../../tests/fixtures/p5_eq_3k.json"),
    include_str!("../../../tests/fixtures/p5_delay_ms.json"),
    include_str!("../../../tests/fixtures/p5_delay_sync.json"),
    include_str!("../../../tests/fixtures/p5_reverb.json"),
    include_str!("../../../tests/fixtures/p5_reverb_dry.json"),
    include_str!("../../../tests/fixtures/p5_reverb_none.json"),
    include_str!("../../../tests/fixtures/p5_chain.json"),
    include_str!("../../../tests/fixtures/p5_saturated.json"),
];

/// p5_delay_sync: channel 1 plays into insert 1, whose slot 0 holds effect 1 (a delay synced
/// to 3 steps, feedback 0.5, mix 1); next effect id 2; the other 15 inserts as new.
const P5_DELAY_SYNC: &str = P5_FIXTURES[7];

#[test]
fn p5_fixtures_round_trip_byte_for_byte() {
    for text in P5_FIXTURES {
        assert_eq!(json(&load(text)), canonical(text));
    }
}

// A default mixer and channels on the master are left out of the file, so every earlier fixture
// keeps its bytes (the state hashes above check the same from the other side).
#[test]
fn a_default_mixer_and_master_routing_are_not_written() {
    for text in [F1, F2, F3, F4, P2_EMPTY, P2_THREE, P3_ROLL, P4_LOOP] {
        let saved = json(&load(text));
        assert!(!saved.contains("mixer"), "{saved}");
        assert!(!saved.contains(r#""insert""#), "{saved}");
    }
}

// An effect this build does not know keeps its id and every parameter, unknown keys too.
#[test]
fn an_unknown_processor_round_trips_untouched() {
    let delay = r#""processor":{"id":"dawsome.delay","params":{"feedback":0.5"#;
    assert!(P5_DELAY_SYNC.contains(delay));
    let unknown = P5_DELAY_SYNC.replacen(
        delay,
        r#""processor":{"id":"vendor.missing","params":{"color":7.5,"feedback":0.5"#,
        1,
    );
    assert_eq!(json(&load(&unknown)), canonical(&unknown));
}

fn insert(id: u16) -> InsertId {
    InsertId(id)
}

fn mixer_only() -> Touched {
    Touched {
        mixer: true,
        ..Touched::default()
    }
}

fn effects_touched(id: u32, mixer: bool) -> Touched {
    Touched {
        mixer,
        effects: BTreeSet::from([EffectId(id)]),
        ..Touched::default()
    }
}

/// One of every mixer edit p5_delay_sync accepts, with what it touches.
fn every_mixer_edit() -> Vec<(Edit, Touched)> {
    let param = |key: &str, value| Edit::SetEffectParam {
        effect: EffectId(1),
        key: key.into(),
        value,
    };
    vec![
        (
            Edit::SetInsertVolume {
                insert: insert(2),
                fader: 0.5,
            },
            mixer_only(),
        ),
        (
            Edit::SetInsertPan {
                insert: insert(3),
                pan: -0.25,
            },
            mixer_only(),
        ),
        (
            Edit::MuteInsert {
                insert: insert(4),
                mute: true,
            },
            mixer_only(),
        ),
        (
            Edit::RenameInsert {
                insert: insert(1),
                name: "Echo".into(),
            },
            mixer_only(),
        ),
        (
            Edit::SoloInsert {
                set: BTreeSet::from([insert(1), insert(5)]),
            },
            mixer_only(),
        ),
        (
            Edit::RouteChannel {
                channel: ChannelId(1),
                insert: insert(7),
            },
            Touched {
                channels: BTreeSet::from([ChannelId(1)]),
                ..Touched::default()
            },
        ),
        (
            Edit::SetSend {
                from: insert(2),
                to: insert(3),
                amount: Some(0.5),
            },
            mixer_only(),
        ),
        (
            Edit::SetSend {
                from: insert(1),
                to: InsertId::MASTER,
                amount: Some(0.25),
            },
            mixer_only(),
        ),
        (
            Edit::SetSend {
                from: insert(1),
                to: InsertId::MASTER,
                amount: None,
            },
            mixer_only(),
        ),
        (
            Edit::AddEffect {
                insert: insert(1),
                slot: 3,
                id: EffectId(2),
                processor: ProcessorRef::new("dawsome.reverb"),
            },
            effects_touched(2, true),
        ),
        (
            Edit::RemoveEffect { id: EffectId(1) },
            effects_touched(1, true),
        ),
        (param("feedback", Some(0.75)), effects_touched(1, false)),
        (param("time_ms", Some(20.0)), effects_touched(1, false)),
        (param("mix", None), effects_touched(1, false)),
        // A value outside the processor's range is the processor's to clamp.
        (param("feedback", Some(5.0)), effects_touched(1, false)),
        (
            Edit::MoveEffect {
                effect: EffectId(1),
                slot: 4,
            },
            mixer_only(),
        ),
    ]
}

#[test]
fn every_mixer_edit_is_undone_byte_for_byte() {
    for (edit, touched) in every_mixer_edit() {
        let mut project = load(P5_DELAY_SYNC);
        let applied = edit::apply(&mut project, edit.clone()).expect("applies");
        assert_ne!(
            json(&project),
            canonical(P5_DELAY_SYNC),
            "{edit:?} changed nothing"
        );
        assert_eq!(applied.touched, touched, "{edit:?}");
        project.validate().expect("the edited project is valid");
        let changed = json(&project);
        let undone = edit::apply(&mut project, applied.inverse).expect("inverse applies");
        assert_eq!(json(&project), canonical(P5_DELAY_SYNC), "{edit:?} undone");
        edit::apply(&mut project, undone.inverse).expect("redo applies");
        assert_eq!(json(&project), changed, "{edit:?} redone");
    }
}

// Moving an effect onto a taken slot swaps the two; undo swaps them back.
#[test]
fn moving_onto_a_taken_slot_swaps_the_two_effects() {
    let mut project = load(P5_DELAY_SYNC);
    let add = Edit::AddEffect {
        insert: insert(1),
        slot: 6,
        id: EffectId(2),
        processor: ProcessorRef::new("dawsome.eq3"),
    };
    edit::apply(&mut project, add).expect("slot 6 is free");
    let before = json(&project);
    let slots = |project: &Project| -> Vec<(u32, u8)> {
        let strip = project.mixer.get(insert(1)).expect("insert 1");
        strip.effects.iter().map(|e| (e.id.0, e.slot)).collect()
    };
    assert_eq!(slots(&project), [(1, 0), (2, 6)]);
    let moved = edit::apply(
        &mut project,
        Edit::MoveEffect {
            effect: EffectId(1),
            slot: 6,
        },
    )
    .expect("moves");
    assert_eq!(slots(&project), [(2, 0), (1, 6)]);
    edit::apply(&mut project, moved.inverse).expect("undo applies");
    assert_eq!(json(&project), before);
}

// Undoing an added effect gives its id back, so redoing it gives the same id.
#[test]
fn undoing_an_added_effect_gives_its_id_back() {
    let mut project = load(P5_DELAY_SYNC);
    let mut history = History::new();
    let add = Edit::AddEffect {
        insert: insert(2),
        slot: 0,
        id: EffectId(2),
        processor: ProcessorRef::new("dawsome.reverb"),
    };
    history.apply(&mut project, add).expect("adds");
    assert_eq!(project.mixer.next_effect_id, EffectId(3));
    history.undo(&mut project).expect("undo").expect("an edit");
    assert_eq!(json(&project), canonical(P5_DELAY_SYNC));
    history.redo(&mut project).expect("redo").expect("an edit");
    assert_eq!(project.mixer.next_effect_id, EffectId(3));
    assert!(project.mixer.effect(EffectId(2)).is_some());
}

/// An edit that must be refused, and a check that it was refused for the right reason.
type RefusedEdit = (Edit, fn(&EditError) -> bool);

// A send that would close a loop, a send to itself and a send from the master are refused and
// change nothing; so is every other edit that breaks a mixer rule.
#[test]
fn every_refused_mixer_edit_changes_nothing() {
    let send = |from, to, amount| Edit::SetSend {
        from: insert(from),
        to: insert(to),
        amount,
    };
    let reverb_at = |slot, id| Edit::AddEffect {
        insert: insert(1),
        slot,
        id: EffectId(id),
        processor: ProcessorRef::new("dawsome.reverb"),
    };
    let mut non_finite = ProcessorRef::new("dawsome.reverb");
    non_finite.params.insert("room".into(), f32::INFINITY);
    let restore = |slot, id| Edit::RestoreEffect {
        insert: insert(1),
        slot,
        id: EffectId(id),
        processor: ProcessorRef::new("dawsome.delay"),
    };
    let refused: Vec<RefusedEdit> = vec![
        (send(3, 3, Some(1.0)), |e| *e == EditError::SendToItself(3)),
        (send(0, 1, Some(1.0)), |e| *e == EditError::SendFromMaster),
        (send(1, 17, Some(1.0)), |e| {
            *e == EditError::UnknownInsert(17)
        }),
        (send(1, 2, Some(1.5)), |e| {
            *e == EditError::InvalidSendAmount(1.5)
        }),
        (
            send(1, 2, Some(f32::NAN)),
            |e| matches!(e, EditError::InvalidSendAmount(v) if v.is_nan()),
        ),
        (
            Edit::SetInsertVolume {
                insert: insert(1),
                fader: 1.5,
            },
            |e| *e == EditError::InvalidFader(1.5),
        ),
        (
            Edit::SetInsertVolume {
                insert: insert(17),
                fader: 0.5,
            },
            |e| *e == EditError::UnknownInsert(17),
        ),
        (
            Edit::SetInsertPan {
                insert: insert(1),
                pan: 2.0,
            },
            |e| *e == EditError::InvalidPan(2.0),
        ),
        (
            Edit::SoloInsert {
                set: BTreeSet::from([insert(17)]),
            },
            |e| *e == EditError::UnknownInsert(17),
        ),
        (
            Edit::RouteChannel {
                channel: ChannelId(1),
                insert: insert(17),
            },
            |e| *e == EditError::UnknownInsert(17),
        ),
        (
            Edit::RouteChannel {
                channel: ChannelId(9),
                insert: insert(2),
            },
            |e| *e == EditError::UnknownChannel(9),
        ),
        (reverb_at(3, 5), |e| {
            *e == EditError::NotNextEffectId {
                got: 5,
                expected: 2,
            }
        }),
        (reverb_at(0, 2), |e| {
            *e == EditError::SlotTaken { insert: 1, slot: 0 }
        }),
        (reverb_at(10, 2), |e| *e == EditError::InvalidEffectSlot(10)),
        (
            Edit::AddEffect {
                insert: insert(1),
                slot: 3,
                id: EffectId(2),
                processor: non_finite,
            },
            |e| *e == EditError::InvalidParamValue(f32::INFINITY),
        ),
        (Edit::UnaddEffect { id: EffectId(0) }, |e| {
            *e == EditError::NotNewestEffect(0)
        }),
        (Edit::RemoveEffect { id: EffectId(9) }, |e| {
            *e == EditError::UnknownEffect(9)
        }),
        (restore(5, 2), |e| {
            *e == EditError::Invalid(ValidationError::UnallocatedEffectId { id: 2, next: 2 })
        }),
        (restore(5, 1), |e| *e == EditError::EffectExists(1)),
        (
            Edit::SetEffectParam {
                effect: EffectId(1),
                key: "mix".into(),
                value: Some(f32::NEG_INFINITY),
            },
            |e| *e == EditError::InvalidParamValue(f32::NEG_INFINITY),
        ),
        (
            Edit::SetEffectParam {
                effect: EffectId(9),
                key: "mix".into(),
                value: Some(0.5),
            },
            |e| *e == EditError::UnknownEffect(9),
        ),
        (
            Edit::MoveEffect {
                effect: EffectId(1),
                slot: 10,
            },
            |e| *e == EditError::InvalidEffectSlot(10),
        ),
        (
            Edit::MoveEffect {
                effect: EffectId(9),
                slot: 1,
            },
            |e| *e == EditError::UnknownEffect(9),
        ),
    ];
    for (edit, is_expected) in refused {
        let mut project = load(P5_DELAY_SYNC);
        let before = json(&project);
        match edit::apply(&mut project, edit.clone()) {
            Err(error) => assert!(is_expected(&error), "{edit:?}: wrong error {error:?}"),
            Ok(_) => panic!("{edit:?} was applied, but should have been refused"),
        }
        assert_eq!(json(&project), before, "{edit:?} changed the project");
    }
}

// After 1 → 2, a send 2 → 1 would loop. It is refused, the header's message names the loop, and the
// document is unchanged.
#[test]
fn a_send_that_would_close_a_loop_is_refused_with_the_loop_named() {
    let mut project = load(P5_DELAY_SYNC);
    let send = |from, to| Edit::SetSend {
        from: insert(from),
        to: insert(to),
        amount: Some(1.0),
    };
    edit::apply(&mut project, send(1, 2)).expect("1 → 2 is fine");
    let before = json(&project);
    let refused = edit::apply(&mut project, send(2, 1)).expect_err("2 → 1 would loop");
    assert_eq!(
        refused,
        EditError::RoutingCycle {
            from: insert(2),
            to: insert(1)
        }
    );
    assert_eq!(
        refused.to_string(),
        "routing loop: Insert 1 already feeds Insert 2"
    );
    assert_eq!(json(&project), before);
    // Through a longer path too: 1 → 2 → 3, then 3 → 1.
    edit::apply(&mut project, send(2, 3)).expect("2 → 3 is fine");
    assert!(matches!(
        edit::apply(&mut project, send(3, 1)),
        Err(EditError::RoutingCycle { .. })
    ));
}

// Mixer and effect edits never recompile a pattern's event list.
#[test]
fn mixer_edits_recompile_no_pattern() {
    let silent = |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(vec![0.0]));
    for (edit, _) in every_mixer_edit() {
        let mut project = load(P5_DELAY_SYNC);
        let mut compiler = Compiler::new();
        let mode = PlayMode::Pattern(PatternId(1));
        let before = compiler
            .compile(&project, &Touched::default(), mode, silent)
            .expect("compiles");
        let applied = edit::apply(&mut project, edit.clone()).expect("applies");
        let after = compiler
            .compile(&project, &applied.touched, mode, silent)
            .expect("compiles");
        assert!(
            Arc::ptr_eq(events(&before, 1), events(&after, 1)),
            "{edit:?} recompiled pattern 1"
        );
    }
}
