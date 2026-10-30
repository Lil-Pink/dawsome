//! The edits a recording makes (`edit::takes`): a song take's pattern and clip and their growth by
//! whole bars, an audio take's channel and clip, and Capture. Each built from the plain edits and
//! applied as the window applies it, live, its inverses kept newest first as one undo step that
//! gives the document back byte for byte. Expected documents come from
//! tests/fixtures/gen_p17_expected.py; the others are written out by hand in each test.

use std::convert::Infallible;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::takes::{self, SongTake};
use dawsome_engine::edit::{self, Edit, History, Touched};
use dawsome_engine::plan::PlayMode;
use dawsome_engine::project::{ChannelId, ClipId, Note, PatternId, Project, TrackId};
use dawsome_engine::realtime;
use dawsome_engine::time::Ticks;
use dawsome_engine::transport::Control;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

fn text(name: &str) -> String {
    let text = std::fs::read_to_string(Path::new(FIXTURES).join(name)).expect("the fixture");
    text.trim_end().to_owned()
}

fn fixture(name: &str) -> Project {
    Project::load_json(&text(name)).expect("loads")
}

fn json(project: &Project) -> String {
    project.to_json().expect("serializes")
}

fn expected() -> serde_json::Value {
    serde_json::from_str(&text("p17_expected.json")).expect("parses")
}

fn number(value: &serde_json::Value) -> i64 {
    value.as_i64().expect("a number")
}

/// The notes of pattern `id` in the document `name` the generator wrote.
fn notes_of(name: &str, id: u32) -> Vec<Note> {
    let doc = fixture(name);
    doc.patterns[&PatternId(id)].notes.clone()
}

/// A recording's edits applied live, as the window applies them: each one's inverse kept, the
/// whole one undo step at the end.
#[derive(Default)]
struct Live {
    undo: Vec<Edit>,
}

impl Live {
    fn apply(&mut self, project: &mut Project, edit: Edit) {
        let applied = edit::apply(project, edit).expect("applies");
        self.undo.push(applied.inverse);
    }

    /// One undo step, newest first.
    fn step(mut self) -> Edit {
        self.undo.reverse();
        Edit::Transaction(self.undo)
    }
}

/// `history`'s one step undone gives `before` back byte for byte, and redone `after`.
fn round_trip(project: &mut Project, history: &mut History, before: &str, after: &str) {
    assert_eq!(json(project), after, "after");
    assert!(history.undo(project).expect("undoes").is_some());
    assert_eq!(json(project), before, "undone");
    assert!(history.redo(project).expect("redoes").is_some());
    assert_eq!(json(project), after, "redone");
    project.validate().expect("valid");
}

/// A song take from `marker` as the window makes it: the pattern and its clip one bar long,
/// grown a bar at a time to `bars`, then `notes` written one by one as they end.
fn song_take(project: &mut Project, marker: i64, bars: u32, notes: &[Note]) -> (SongTake, Live) {
    let name = takes::take_name(project, &project.channels[&ChannelId(1)].name.clone());
    let take = takes::song_take(project, name, Ticks(marker), None, 1).expect("builds");
    let mut live = Live::default();
    live.apply(project, take.edit.clone());
    for held in 2..=bars {
        live.apply(
            project,
            takes::take_length(take.pattern, take.clip, held, project.ppq),
        );
    }
    for note in notes {
        let write = Edit::ReplaceNotes {
            pattern: take.pattern,
            remove: Vec::new(),
            add: vec![note.clone()],
        };
        live.apply(project, write);
    }
    (take, live)
}

// Known answer (2) in the engine: pattern "Kick take 1" and its clip from tick 384 on track 2
// (track 1 holds a clip over the take's first bar), grown to 6 bars, the four notes written as
// they ended: the generator's document byte for byte. The take is one undo step that gives the
// project back byte for byte (the pattern, clip and counters gone); its redo makes it again.
#[test]
fn p17_a_song_take_from_its_edits_is_one_undo_step() {
    let want = &expected()["song"];
    let mut project = fixture("p17_song.json");
    let before = json(&project);
    let after = text(want["document"].as_str().expect("a file"));
    let notes = notes_of("p17_song_after.json", 2);
    let bars = u32::try_from(number(&want["bars"])).expect("fits");
    let (take, live) = song_take(&mut project, number(&want["marker"]), bars, &notes);
    assert_eq!(
        (take.pattern, take.clip, take.track),
        (
            PatternId(2),
            ClipId(2),
            TrackId(u32::try_from(number(&want["track"])).expect("fits"))
        )
    );
    assert_eq!(project.patterns[&PatternId(2)].name, "Kick take 1");
    assert!(!take.added_track, "track 2 had room");
    assert_eq!(project.playlist.end(), Ticks(number(&want["song_end"])));
    let mut history = History::default();
    history.record_as(live.step(), "Record notes");
    round_trip(&mut project, &mut history, &before, &after);
}

// Known answer (5): a take from tick 3072, past the song's end, lands on track 1, free there.
#[test]
fn p17_a_song_take_past_the_end_lands_on_the_first_free_track() {
    let want = &expected()["song_past_end"];
    let mut project = fixture("p17_song.json");
    let before = json(&project);
    let notes = notes_of("p17_song_past_end_after.json", 2);
    let bars = u32::try_from(number(&want["bars"])).expect("fits");
    let (take, live) = song_take(&mut project, number(&want["marker"]), bars, &notes);
    assert_eq!(take.track, TrackId(1));
    let mut history = History::default();
    history.record_as(live.step(), "Record notes");
    let after = text(want["document"].as_str().expect("a file"));
    round_trip(&mut project, &mut history, &before, &after);
}

// Known answer (5): a take in which no note was played, grown to three bars, is taken back whole
// by its own inverses, leaving the project byte for byte as it was; nothing is recorded.
#[test]
fn a_song_take_with_no_note_is_taken_back_whole() {
    let mut project = fixture("p17_song.json");
    let before = json(&project);
    let (_, live) = song_take(&mut project, 384, 3, &[]);
    assert_eq!(project.patterns.len(), 2);
    edit::apply(&mut project, live.step()).expect("takes it back");
    assert_eq!(json(&project), before);
}

// With every track taken over the take's first bar, a track is added in the same step and taken
// out by its undo; the armed track wins over a free one.
#[test]
fn a_song_take_with_no_free_track_adds_one_and_the_armed_track_wins() {
    let mut project = fixture("p17_song.json");
    for track in 2..=8 {
        let clip = dawsome_engine::project::Clip::of_pattern(
            PatternId(1),
            TrackId(track),
            Ticks(0),
            Ticks(1536),
        );
        let first = project.playlist.next_clip_id;
        edit::apply(
            &mut project,
            Edit::PlaceClips {
                first,
                clips: vec![clip],
            },
        )
        .expect("places");
    }
    let before = json(&project);
    let take =
        takes::song_take(&project, "Kick take 1".into(), Ticks(384), None, 1).expect("builds");
    assert_eq!((take.track, take.added_track), (TrackId(9), true));
    let mut history = History::default();
    history.apply(&mut project, take.edit).expect("applies");
    assert_eq!(project.playlist.tracks.len(), 9);
    assert!(history.undo(&mut project).expect("undoes").is_some());
    assert_eq!(json(&project), before);
    let armed = takes::song_take(
        &project,
        "Kick take 1".into(),
        Ticks(384),
        Some(TrackId(3)),
        1,
    )
    .expect("builds");
    assert_eq!(armed.track, TrackId(3), "the armed track, taken or not");
    let gone = takes::song_take(
        &project,
        "Kick take 1".into(),
        Ticks(384),
        Some(TrackId(40)),
        1,
    )
    .expect("builds");
    assert_eq!(
        gone.track,
        TrackId(9),
        "an armed track the project lacks counts for nothing"
    );
}

// By hand: whole bars of 384 ticks holding a span, at least one; "<channel> take N" for the
// first N no pattern is named.
#[test]
fn bars_holding_a_span_and_take_names() {
    for (span, bars) in [
        (-5, 1),
        (0, 1),
        (1, 1),
        (384, 1),
        (385, 2),
        (2113, 6),
        (2304, 6),
    ] {
        assert_eq!(takes::bars_holding(Ticks(span), 96), bars, "{span}");
    }
    let mut project = fixture("p17_song.json");
    assert_eq!(takes::take_name(&project, "Kick"), "Kick take 1");
    edit::apply(
        &mut project,
        Edit::AddPattern {
            id: PatternId(2),
            name: "Kick take 1".into(),
            bars: 1,
            copied_from: None,
        },
    )
    .expect("adds");
    assert_eq!(takes::take_name(&project, "Kick"), "Kick take 2");
}

// Known answer (3) in the engine: two audio takes from tick 192, 1000 frames each at 48 kHz:
// channels "Take 1" and "Take 2" with their files, clips 4 ticks long on tracks 2 and 3. Each one
// undo step; undoing both gives the project back.
#[test]
fn p17_audio_takes_from_their_edit() {
    let want = &expected()["audio_song"];
    let mut project = fixture("p17_audio48.json");
    let before = json(&project);
    let mut history = History::default();
    for take in want["takes"].as_array().expect("takes") {
        let number = number(&take["channel"]) - 1;
        let edit = takes::audio_take(
            &project,
            format!("Take {number}"),
            PathBuf::from(take["file"].as_str().expect("a file")),
            Ticks(take["start"].as_i64().expect("a tick")),
            take["frames"].as_u64().expect("frames"),
            None,
        )
        .expect("an edit");
        history.apply(&mut project, edit).expect("applies");
    }
    let after = text(want["document"].as_str().expect("a file"));
    assert_eq!(json(&project), after);
    assert!(history.undo(&mut project).expect("undoes").is_some());
    assert!(history.undo(&mut project).expect("undoes").is_some());
    assert_eq!(json(&project), before);
    assert!(
        takes::audio_take(&project, "Take 1".into(), "x.wav".into(), Ticks(0), 0, None).is_none(),
        "an empty take adds nothing"
    );
    let want = &expected()["audio_pattern"];
    let take = &want["take"];
    let edit = takes::audio_take(
        &project,
        "Take 1".into(),
        PathBuf::from(take["file"].as_str().expect("a file")),
        Ticks(0),
        take["frames"].as_u64().expect("frames"),
        None,
    )
    .expect("an edit");
    history.apply(&mut project, edit).expect("applies");
    let after = text(want["document"].as_str().expect("a file"));
    round_trip(&mut project, &mut history, &before, &after);
}

// Known answer (4) in the engine: the captured phrase laid into pattern 1, which grows to two
// bars to hold it; and the same phrase as pattern "Keys take 1" and its clip from tick 384. Each
// one undo step.
#[test]
fn p17_capture_from_its_edits() {
    let want = &expected()["capture"];
    let notes = notes_of("p17_capture_after.json", 1);
    let mut project = fixture("p17_capture.json");
    let before = json(&project);
    let mut history = History::default();
    let edit = takes::capture_into(&project, PatternId(1), notes.clone()).expect("an edit");
    history.apply(&mut project, edit).expect("applies");
    let after = text(want["document"].as_str().expect("a file"));
    round_trip(&mut project, &mut history, &before, &after);
    let mut project = fixture("p17_capture.json");
    let mut history = History::default();
    let name = takes::take_name(&project, "Keys");
    let start = Ticks(number(&want["song_start"]));
    let edit = takes::capture_song(&project, name, start, None, notes)
        .expect("builds")
        .expect("an edit");
    history.apply(&mut project, edit).expect("applies");
    let after = text(want["song_document"].as_str().expect("a file"));
    round_trip(&mut project, &mut history, &before, &after);
}

// By hand: captured notes on one channel, tick and key keep the later one; a captured note on a
// note of the pattern's slot replaces it; notes reaching past the pattern's end grow it by
// whole bars (a 1-bar pattern, a note 380..400 → 2 bars); nothing to capture is no edit.
#[test]
fn captured_notes_replace_one_per_slot_and_grow_the_pattern() {
    let note = |start, length, velocity, key| Note {
        channel: ChannelId(1),
        start: Ticks(start),
        length: Ticks(length),
        velocity,
        key,
    };
    let mut project = fixture("p17_capture.json");
    edit::apply(
        &mut project,
        Edit::ReplaceNotes {
            pattern: PatternId(1),
            remove: Vec::new(),
            add: vec![note(0, 10, 0.25, 60), note(96, 0, 0.25, 62)],
        },
    )
    .expect("adds");
    let before = json(&project);
    let captured = vec![
        note(0, 24, 0.5, 60),
        note(0, 48, 0.75, 60),
        note(380, 20, 1.0, 64),
    ];
    let edit = takes::capture_into(&project, PatternId(1), captured).expect("an edit");
    let applied = edit::apply(&mut project, edit).expect("applies");
    let pattern = &project.patterns[&PatternId(1)];
    assert_eq!(pattern.length_bars, 2);
    assert_eq!(
        pattern.notes,
        [
            note(0, 48, 0.75, 60),
            note(96, 0, 0.25, 62),
            note(380, 20, 1.0, 64)
        ]
    );
    edit::apply(&mut project, applied.inverse).expect("undoes");
    assert_eq!(json(&project), before);
    assert!(takes::capture_into(&project, PatternId(1), Vec::new()).is_none());
    assert!(takes::capture_into(&project, PatternId(9), vec![note(0, 1, 1.0, 60)]).is_none());
    assert!(matches!(
        takes::capture_song(&project, "x".into(), Ticks(0), None, Vec::new()),
        Ok(None)
    ));
}

// EndRecording: a recording plays straight through the song's end (one bar of p17_song's
// pattern clip ends at 1536); once ended while the playhead is before the end, the song wraps
// there again, as it does when played. In the count-in it ends the count-in and nothing plays.
#[test]
fn ending_a_recording_brings_the_song_end_wrap_back() {
    let project = fixture("p17_song.json");
    let same = |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(vec![1.0]));
    let plan = Compiler::new()
        .compile(&project, &Touched::default(), PlayMode::Song, same)
        .expect("compiles");
    let (mut link, mut player) = realtime::link(plan, project.tempo);
    let mut out = vec![[0.0; 2]; 512];
    // 1536 ticks at 44.1 kHz and 120 BPM are 352800 frames: 689.06 calls.
    link.send(Control::Record {
        count_in: false,
        tap_master: false,
    })
    .expect("room");
    for _ in 0..700 {
        player.process(&mut out);
    }
    assert!(
        link.position().ticks > Ticks(1536),
        "straight through while recording"
    );
    link.send(Control::Stop).expect("room");
    player.process(&mut out);
    link.send(Control::Record {
        count_in: false,
        tap_master: false,
    })
    .expect("room");
    for _ in 0..100 {
        player.process(&mut out);
    }
    link.send(Control::EndRecording).expect("room");
    for _ in 0..600 {
        player.process(&mut out);
    }
    assert!(link.playing());
    assert!(
        link.position().ticks < Ticks(1536),
        "wrapped at the song's end"
    );
    link.send(Control::Stop).expect("room");
    player.process(&mut out);
    link.send(Control::Record {
        count_in: true,
        tap_master: false,
    })
    .expect("room");
    player.process(&mut out);
    assert!(link.counting_in());
    link.send(Control::EndRecording).expect("room");
    for _ in 0..400 {
        player.process(&mut out);
    }
    assert!(
        !link.counting_in() && !link.playing(),
        "the count-in ended, nothing plays"
    );
}
