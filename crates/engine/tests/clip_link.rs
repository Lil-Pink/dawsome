//! Clip link and unlink: a clip given its own copy of the
//! pattern it shares, an edit in the copy leaving the others alone, and the clip
//! linked back, its copy gone when no other clip plays it. Each is one undo step that gives the
//! document back byte for byte, through `History` as the window keeps it. Every expected
//! document is written out here by hand, never taken from the engine's output.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::path::Path;

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::copies::{self, Link, LinkBack, Unlink};
use dawsome_engine::edit::{self, Edit, EditError, History, Touched};
use dawsome_engine::plan::PlayMode;
use dawsome_engine::project::{
    Clip, ClipId, LoadError, Note, PatternId, Project, TrackId, ValidationError,
};
use dawsome_engine::realtime;
use dawsome_engine::time::Ticks;
use dawsome_engine::transport::Control;

/// One sampler, "Kick", and "Pattern 1" (one bar: a step on tick 0, a note on tick 96, key 64,
/// half velocity, 48 ticks long) played by five one-bar clips side by side on Track 1, clip N
/// from bar N.
const FIVE: &str = concat!(
    r#"{"format_version":2,"sample_rate":44100,"tempo_mbpm":120000,"ppq":96,"next_channel_id":2,"#,
    r#""channels":[{"id":1,"name":"Kick","kind":"sampler","sample":"impulse.wav","volume":1.0,"pan":0.0,"mute":false}],"#,
    r#""patterns":[{"id":1,"name":"Pattern 1","length_bars":1,"notes":["#,
    r#"{"channel":1,"tick":0,"length":0,"velocity":1.0},"#,
    r#"{"channel":1,"tick":96,"length":48,"velocity":0.5,"key":64}]}],"#,
    r#""playlist":{"next_track_id":2,"next_clip_id":6,"tracks":[{"id":1,"name":"Track 1","mute":false}],"#,
    r#""clips":[{"id":1,"track":1,"start":0,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":2,"track":1,"start":384,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":3,"track":1,"start":768,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":4,"track":1,"start":1152,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":5,"track":1,"start":1536,"length":384,"kind":"pattern","pattern":1}]}}"#
);

/// FIVE with clip 3 unlinked: pattern 2, "Pattern 1 (copy)", made from pattern 1, with its
/// length and both notes; clip 3 plays it, the others pattern 1.
const UNLINKED: &str = concat!(
    r#"{"format_version":2,"sample_rate":44100,"tempo_mbpm":120000,"ppq":96,"next_channel_id":2,"#,
    r#""channels":[{"id":1,"name":"Kick","kind":"sampler","sample":"impulse.wav","volume":1.0,"pan":0.0,"mute":false}],"#,
    r#""patterns":[{"id":1,"name":"Pattern 1","length_bars":1,"notes":["#,
    r#"{"channel":1,"tick":0,"length":0,"velocity":1.0},"#,
    r#"{"channel":1,"tick":96,"length":48,"velocity":0.5,"key":64}]},"#,
    r#"{"id":2,"name":"Pattern 1 (copy)","length_bars":1,"copied_from":1,"notes":["#,
    r#"{"channel":1,"tick":0,"length":0,"velocity":1.0},"#,
    r#"{"channel":1,"tick":96,"length":48,"velocity":0.5,"key":64}]}],"#,
    r#""playlist":{"next_track_id":2,"next_clip_id":6,"tracks":[{"id":1,"name":"Track 1","mute":false}],"#,
    r#""clips":[{"id":1,"track":1,"start":0,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":2,"track":1,"start":384,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":3,"track":1,"start":768,"length":384,"kind":"pattern","pattern":2},"#,
    r#"{"id":4,"track":1,"start":1152,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":5,"track":1,"start":1536,"length":384,"kind":"pattern","pattern":1}]}}"#
);

/// UNLINKED with the copy's note on tick 96 raised from key 64 to 67; pattern 1 as it was.
const EDITED: &str = concat!(
    r#"{"format_version":2,"sample_rate":44100,"tempo_mbpm":120000,"ppq":96,"next_channel_id":2,"#,
    r#""channels":[{"id":1,"name":"Kick","kind":"sampler","sample":"impulse.wav","volume":1.0,"pan":0.0,"mute":false}],"#,
    r#""patterns":[{"id":1,"name":"Pattern 1","length_bars":1,"notes":["#,
    r#"{"channel":1,"tick":0,"length":0,"velocity":1.0},"#,
    r#"{"channel":1,"tick":96,"length":48,"velocity":0.5,"key":64}]},"#,
    r#"{"id":2,"name":"Pattern 1 (copy)","length_bars":1,"copied_from":1,"notes":["#,
    r#"{"channel":1,"tick":0,"length":0,"velocity":1.0},"#,
    r#"{"channel":1,"tick":96,"length":48,"velocity":0.5,"key":67}]}],"#,
    r#""playlist":{"next_track_id":2,"next_clip_id":6,"tracks":[{"id":1,"name":"Track 1","mute":false}],"#,
    r#""clips":[{"id":1,"track":1,"start":0,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":2,"track":1,"start":384,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":3,"track":1,"start":768,"length":384,"kind":"pattern","pattern":2},"#,
    r#"{"id":4,"track":1,"start":1152,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":5,"track":1,"start":1536,"length":384,"kind":"pattern","pattern":1}]}}"#
);

/// EDITED with clip 3 linked back: the copy gone with its edit, every clip on pattern 1: FIVE,
/// but the pattern counter is written (3: the copy's ID is never given out again).
const LINKED_BACK: &str = concat!(
    r#"{"format_version":2,"sample_rate":44100,"tempo_mbpm":120000,"ppq":96,"next_channel_id":2,"#,
    r#""channels":[{"id":1,"name":"Kick","kind":"sampler","sample":"impulse.wav","volume":1.0,"pan":0.0,"mute":false}],"#,
    r#""next_pattern_id":3,"#,
    r#""patterns":[{"id":1,"name":"Pattern 1","length_bars":1,"notes":["#,
    r#"{"channel":1,"tick":0,"length":0,"velocity":1.0},"#,
    r#"{"channel":1,"tick":96,"length":48,"velocity":0.5,"key":64}]}],"#,
    r#""playlist":{"next_track_id":2,"next_clip_id":6,"tracks":[{"id":1,"name":"Track 1","mute":false}],"#,
    r#""clips":[{"id":1,"track":1,"start":0,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":2,"track":1,"start":384,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":3,"track":1,"start":768,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":4,"track":1,"start":1152,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":5,"track":1,"start":1536,"length":384,"kind":"pattern","pattern":1}]}}"#
);

/// FIVE with clip 3, then clip 4 unlinked: "Pattern 1 (copy)" (2) for clip 3 and
/// "Pattern 1 (copy 2)" (3) for clip 4, both made from pattern 1.
const TWO_COPIES: &str = concat!(
    r#"{"format_version":2,"sample_rate":44100,"tempo_mbpm":120000,"ppq":96,"next_channel_id":2,"#,
    r#""channels":[{"id":1,"name":"Kick","kind":"sampler","sample":"impulse.wav","volume":1.0,"pan":0.0,"mute":false}],"#,
    r#""patterns":[{"id":1,"name":"Pattern 1","length_bars":1,"notes":["#,
    r#"{"channel":1,"tick":0,"length":0,"velocity":1.0},"#,
    r#"{"channel":1,"tick":96,"length":48,"velocity":0.5,"key":64}]},"#,
    r#"{"id":2,"name":"Pattern 1 (copy)","length_bars":1,"copied_from":1,"notes":["#,
    r#"{"channel":1,"tick":0,"length":0,"velocity":1.0},"#,
    r#"{"channel":1,"tick":96,"length":48,"velocity":0.5,"key":64}]},"#,
    r#"{"id":3,"name":"Pattern 1 (copy 2)","length_bars":1,"copied_from":1,"notes":["#,
    r#"{"channel":1,"tick":0,"length":0,"velocity":1.0},"#,
    r#"{"channel":1,"tick":96,"length":48,"velocity":0.5,"key":64}]}],"#,
    r#""playlist":{"next_track_id":2,"next_clip_id":6,"tracks":[{"id":1,"name":"Track 1","mute":false}],"#,
    r#""clips":[{"id":1,"track":1,"start":0,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":2,"track":1,"start":384,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":3,"track":1,"start":768,"length":384,"kind":"pattern","pattern":2},"#,
    r#"{"id":4,"track":1,"start":1152,"length":384,"kind":"pattern","pattern":3},"#,
    r#"{"id":5,"track":1,"start":1536,"length":384,"kind":"pattern","pattern":1}]}}"#
);

/// UNLINKED with a sixth clip of the copy from bar 6, then clip 3 linked back: the copy stays
/// (clip 6 plays it), clip 3 plays pattern 1 again.
const KEPT_COPY: &str = concat!(
    r#"{"format_version":2,"sample_rate":44100,"tempo_mbpm":120000,"ppq":96,"next_channel_id":2,"#,
    r#""channels":[{"id":1,"name":"Kick","kind":"sampler","sample":"impulse.wav","volume":1.0,"pan":0.0,"mute":false}],"#,
    r#""patterns":[{"id":1,"name":"Pattern 1","length_bars":1,"notes":["#,
    r#"{"channel":1,"tick":0,"length":0,"velocity":1.0},"#,
    r#"{"channel":1,"tick":96,"length":48,"velocity":0.5,"key":64}]},"#,
    r#"{"id":2,"name":"Pattern 1 (copy)","length_bars":1,"copied_from":1,"notes":["#,
    r#"{"channel":1,"tick":0,"length":0,"velocity":1.0},"#,
    r#"{"channel":1,"tick":96,"length":48,"velocity":0.5,"key":64}]}],"#,
    r#""playlist":{"next_track_id":2,"next_clip_id":7,"tracks":[{"id":1,"name":"Track 1","mute":false}],"#,
    r#""clips":[{"id":1,"track":1,"start":0,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":2,"track":1,"start":384,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":3,"track":1,"start":768,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":4,"track":1,"start":1152,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":5,"track":1,"start":1536,"length":384,"kind":"pattern","pattern":1},"#,
    r#"{"id":6,"track":1,"start":1920,"length":384,"kind":"pattern","pattern":2}]}}"#
);

fn load(text: &str) -> Project {
    Project::load_json(text).expect("loads")
}

fn json(project: &Project) -> String {
    project.to_json().expect("serializes")
}

/// Applies `edit` as the window does, one step of `history`, and checks the document is
/// `after`; that the step undone gives `before` back byte for byte, and redone `after` again.
fn step(project: &mut Project, history: &mut History, edit: Edit, after: &str) {
    let before = json(project);
    history.apply(project, edit).expect("applies");
    assert_eq!(json(project), after, "after");
    assert!(history.undo(project).expect("undoes").is_some());
    assert_eq!(json(project), before, "undone");
    assert!(history.redo(project).expect("redoes").is_some());
    assert_eq!(json(project), after, "redone");
    project.validate().expect("valid");
}

/// The copy's note on tick 96 moved from key 64 to 67, as a drag in the piano roll does.
fn raise_note(pattern: PatternId, key: u8) -> Edit {
    let note = |key| Note {
        channel: dawsome_engine::project::ChannelId(1),
        start: Ticks(96),
        length: Ticks(48),
        velocity: 0.5,
        key,
    };
    Edit::ReplaceNotes {
        pattern,
        remove: vec![note(64)],
        add: vec![note(key)],
    }
}

// The base document is written in the file's own form: it reads and writes back unchanged. The
// window's tests open it as tests/fixtures/p20_five_clips.json.
#[test]
fn the_known_answers_are_in_the_files_own_form() {
    for text in [FIVE, LINKED_BACK] {
        assert_eq!(json(&load(text)), text);
    }
    let fixture = include_str!("../../../tests/fixtures/p20_five_clips.json");
    assert_eq!(fixture.trim_end(), FIVE);
}

// Known answer 1: Pattern 1 on 5 clips; clip 3 unlinked gives patterns {1, "Pattern 1 (copy)"}
// with identical notes, clip 3 on the copy, the others on 1. Before, every clip is linked (5
// places); after, clip 3 is unlinked from pattern 1 and the other four are linked (4 places).
#[test]
fn unlinking_clip_3_of_five_gives_it_a_copy() {
    let mut project = load(FIVE);
    let mut history = History::default();
    assert_eq!(
        copies::link_of(&project, ClipId(3)),
        Some(Link::Linked { clips: 5 })
    );
    assert_eq!(
        copies::clip_counts(&project),
        BTreeMap::from([(PatternId(1), 5)])
    );
    let Unlink { edit, copy } = copies::unlink(&project, ClipId(3)).expect("builds");
    assert_eq!(copy, PatternId(2));
    step(&mut project, &mut history, edit, UNLINKED);
    assert_eq!(
        copies::link_of(&project, ClipId(3)),
        Some(Link::Unlinked {
            original: PatternId(1)
        })
    );
    assert_eq!(
        copies::link_of(&project, ClipId(1)),
        Some(Link::Linked { clips: 4 })
    );
    assert_eq!(
        copies::clip_counts(&project),
        BTreeMap::from([(PatternId(1), 4), (PatternId(2), 1)])
    );
    assert_eq!(copies::original_of(&project, ClipId(3)), Some(PatternId(1)));
    assert_eq!(copies::original_of(&project, ClipId(1)), None);
}

// Known answer 2: a note edited in the copy leaves pattern 1 unchanged; clip 3 linked back
// takes the copy out (clip 3 on pattern 1); one undo brings the copy back with its edit.
#[test]
fn an_edit_in_the_copy_stays_there_and_link_back_takes_the_copy_out() {
    let mut project = load(FIVE);
    let mut history = History::default();
    let unlink = copies::unlink(&project, ClipId(3)).expect("builds");
    history.apply(&mut project, unlink.edit).expect("unlinks");
    step(
        &mut project,
        &mut history,
        raise_note(PatternId(2), 67),
        EDITED,
    );
    let LinkBack { edit, original } = copies::link_back(&project, ClipId(3)).expect("builds");
    assert_eq!(original, PatternId(1));
    step(&mut project, &mut history, edit, LINKED_BACK);
    assert!(history.undo(&mut project).expect("undoes").is_some());
    assert_eq!(json(&project), EDITED, "one undo: the copy and its edit");
    assert!(history.undo(&mut project).expect("undoes").is_some());
    assert!(history.undo(&mut project).expect("undoes").is_some());
    assert_eq!(json(&project), FIVE, "three undos: as it was");
    assert_eq!(
        copies::link_of(&project, ClipId(3)),
        Some(Link::Linked { clips: 5 })
    );
}

// Known answer 3: two clips unlinked in turn get two copies, "(copy)" and "(copy 2)".
#[test]
fn two_clips_unlinked_in_turn_get_copy_and_copy_2() {
    let mut project = load(FIVE);
    let mut history = History::default();
    let first = copies::unlink(&project, ClipId(3)).expect("builds");
    step(&mut project, &mut history, first.edit, UNLINKED);
    let second = copies::unlink(&project, ClipId(4)).expect("builds");
    assert_eq!(second.copy, PatternId(3));
    step(&mut project, &mut history, second.edit, TWO_COPIES);
    assert_eq!(
        copies::link_of(&project, ClipId(4)),
        Some(Link::Unlinked {
            original: PatternId(1)
        })
    );
    assert_eq!(
        copies::link_of(&project, ClipId(1)),
        Some(Link::Linked { clips: 3 })
    );
}

// The copy stays when another clip plays it: linked back, clip 3 plays pattern 1 again and
// clip 6 keeps the copy, which it now plays alone (unlinked from pattern 1).
#[test]
fn link_back_keeps_a_copy_another_clip_plays() {
    let mut project = load(FIVE);
    let mut history = History::default();
    let unlink = copies::unlink(&project, ClipId(3)).expect("builds");
    history.apply(&mut project, unlink.edit).expect("unlinks");
    let sixth = Clip::of_pattern(PatternId(2), TrackId(1), Ticks(1920), Ticks(384));
    let place = Edit::PlaceClips {
        first: ClipId(6),
        clips: vec![sixth],
    };
    history.apply(&mut project, place).expect("places");
    assert_eq!(
        copies::link_of(&project, ClipId(3)),
        Some(Link::Linked { clips: 2 }),
        "linked to clip 6 wins over unlinked from pattern 1"
    );
    assert_eq!(copies::original_of(&project, ClipId(3)), Some(PatternId(1)));
    let back = copies::link_back(&project, ClipId(3)).expect("builds");
    step(&mut project, &mut history, back.edit, KEPT_COPY);
    assert_eq!(
        copies::link_of(&project, ClipId(6)),
        Some(Link::Unlinked {
            original: PatternId(1)
        })
    );
}

// A copy's name: "(copy)", then "(copy 2)", "(copy 3)", skipping names patterns already have
// (a pattern renamed by hand counts).
#[test]
fn a_copy_takes_the_first_free_copy_name() {
    let mut project = load(FIVE);
    assert_eq!(copies::copy_name(&project, "Pattern 1"), "Pattern 1 (copy)");
    project.patterns.get_mut(&PatternId(1)).expect("1").name = "Pattern 1 (copy)".to_owned();
    assert_eq!(
        copies::copy_name(&project, "Pattern 1"),
        "Pattern 1 (copy 2)"
    );
    assert_eq!(
        copies::copy_name(&project, "Pattern 1 (copy)"),
        "Pattern 1 (copy) (copy)"
    );
}

// What is refused, changing nothing: a clip that is not there, an automation clip, a link back
// of a clip that plays no copy, and of one whose original was deleted. A clip alone on a
// pattern that is no copy shows no link control.
#[test]
fn unlink_and_link_back_refuse_what_they_cannot_do() {
    let project = load(FIVE);
    assert_eq!(
        copies::unlink(&project, ClipId(9)),
        Err(EditError::UnknownClip(9))
    );
    assert_eq!(
        copies::link_back(&project, ClipId(3)),
        Err(EditError::NotACopy(3))
    );
    let automated = load(include_str!("../../../tests/fixtures/p8_auto_linear.json"));
    let (&auto, _) = automated
        .playlist
        .clips
        .iter()
        .find(|(_, clip)| clip.automation().is_some())
        .expect("an automation clip");
    assert_eq!(
        copies::unlink(&automated, auto),
        Err(EditError::NotAPatternClip(auto.0))
    );
    assert_eq!(copies::link_of(&automated, auto), None);
    let alone = load(include_str!("../../../tests/fixtures/p17_song.json"));
    assert_eq!(
        copies::link_of(&alone, ClipId(1)),
        None,
        "one clip, no copy"
    );

    // Pattern 1 deleted after clip 3 was unlinked (its four clips first): the copy stays, but
    // there is nothing to link it back to.
    let mut project = load(FIVE);
    let mut history = History::default();
    let unlink = copies::unlink(&project, ClipId(3)).expect("builds");
    history.apply(&mut project, unlink.edit).expect("unlinks");
    let delete = Edit::Transaction(vec![
        Edit::DeleteClips {
            ids: vec![ClipId(1), ClipId(2), ClipId(4), ClipId(5)],
        },
        Edit::DeletePattern { id: PatternId(1) },
    ]);
    history.apply(&mut project, delete).expect("deletes");
    assert_eq!(copies::link_of(&project, ClipId(3)), None);
    assert_eq!(
        copies::link_back(&project, ClipId(3)),
        Err(EditError::NotACopy(3))
    );
}

// A copy reads and writes back unchanged (its original kept in `copied_from`), and a copy
// naming a pattern made after it (one it cannot have been copied from, or a later one would
// come to be named by it) is refused on load.
#[test]
fn a_copy_round_trips_through_the_file() {
    assert_eq!(json(&load(UNLINKED)), UNLINKED);
    let later = UNLINKED.replace(r#""copied_from":1"#, r#""copied_from":2"#);
    assert!(matches!(
        Project::load_json(&later),
        Err(LoadError::Invalid(
            ValidationError::CopyOfLaterPattern { .. }
        ))
    ));
}

// Playback plays what the document says: FIVE with clip 3 unlinked and the copy's note moved
// from tick 96 to 192, compiled after each edit as the window compiles (only what each touched),
// in song mode with every sound a one-frame click. By hand, s(t) = floor(t·229.6875) at 120 BPM,
// 44.1 kHz: clips at 0, 384, 768, 1152 and 1536 play ticks 0 and 96 of pattern 1, clip 3 ticks 0
// and 192 of the copy.
#[test]
fn the_unlinked_clip_plays_its_copy() {
    let mut project = load(FIVE);
    let click = |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(vec![1.0]));
    let mut compiler = Compiler::new();
    compiler
        .compile(&project, &Touched::default(), PlayMode::Song, click)
        .expect("compiles");
    let mut history = History::default();
    let unlink = copies::unlink(&project, ClipId(3)).expect("builds");
    let touched = history.apply(&mut project, unlink.edit).expect("unlinks");
    compiler
        .compile(&project, &touched, PlayMode::Song, click)
        .expect("compiles");
    let note = |start| Note {
        channel: dawsome_engine::project::ChannelId(1),
        start: Ticks(start),
        length: Ticks(48),
        velocity: 0.5,
        key: 64,
    };
    let moved = Edit::ReplaceNotes {
        pattern: PatternId(2),
        remove: vec![note(96)],
        add: vec![note(192)],
    };
    let touched = history.apply(&mut project, moved).expect("moves");
    let plan = compiler
        .compile(&project, &touched, PlayMode::Song, click)
        .expect("compiles");
    let (mut link, mut player) = realtime::link(plan, project.tempo);
    link.send(Control::Play).expect("room");
    let mut out = vec![[0.0f32; 2]; 441_000];
    for block in out.chunks_mut(512) {
        player.process(block);
    }
    let onsets: Vec<usize> = (0..out.len())
        .filter(|&frame| out[frame] != [0.0; 2] && (frame == 0 || out[frame - 1] == [0.0; 2]))
        .collect();
    assert_eq!(
        onsets,
        [0, 22050, 88200, 110250, 176400, 220500, 264600, 286650, 352800, 374850]
    );
}

// The edit under both transactions: a pattern clip given another pattern touches the playlist
// alone (the song is laid out again, no pattern's events change) and its inverse gives the old
// pattern back; refused, changing nothing, for a clip that is not there, an automation clip, a
// pattern that does not exist and the pattern the clip already plays. A copy added naming a
// pattern that is not one made before it is refused as the file is.
#[test]
fn a_clip_given_another_pattern_and_what_is_refused() {
    let mut project = load(FIVE);
    let verse = Edit::AddPattern {
        id: PatternId(2),
        name: "Verse".to_owned(),
        bars: 1,
        copied_from: None,
    };
    edit::apply(&mut project, verse).expect("adds");
    let applied = edit::apply(
        &mut project,
        Edit::SetClipPattern {
            id: ClipId(3),
            pattern: PatternId(2),
        },
    )
    .expect("points");
    assert_eq!(
        applied.touched,
        Touched {
            playlist: true,
            ..Touched::default()
        }
    );
    assert_eq!(
        applied.inverse,
        Edit::SetClipPattern {
            id: ClipId(3),
            pattern: PatternId(1)
        }
    );
    let before = json(&project);
    let refused = [
        (ClipId(9), PatternId(1), EditError::UnknownClip(9)),
        (ClipId(3), PatternId(7), EditError::UnknownPattern(7)),
        (ClipId(3), PatternId(2), EditError::NothingToChange),
    ];
    for (id, pattern, error) in refused {
        let edit = Edit::SetClipPattern { id, pattern };
        assert_eq!(edit::apply(&mut project, edit), Err(error));
        assert_eq!(json(&project), before, "unchanged");
    }
    let mut automated = load(include_str!("../../../tests/fixtures/p8_auto_linear.json"));
    let (&auto, _) = automated
        .playlist
        .clips
        .iter()
        .find(|(_, clip)| clip.automation().is_some())
        .expect("an automation clip");
    let edit = Edit::SetClipPattern {
        id: auto,
        pattern: PatternId(1),
    };
    assert_eq!(
        edit::apply(&mut automated, edit),
        Err(EditError::NotAPatternClip(auto.0))
    );
    for from in [3, 4] {
        let copy = Edit::AddPattern {
            id: PatternId(3),
            name: "Copy".to_owned(),
            bars: 1,
            copied_from: Some(PatternId(from)),
        };
        assert_eq!(
            edit::apply(&mut project, copy),
            Err(EditError::Invalid(ValidationError::CopyOfLaterPattern {
                pattern: "Copy".to_owned(),
                from,
            }))
        );
        assert_eq!(json(&project), before, "unchanged");
    }
}
