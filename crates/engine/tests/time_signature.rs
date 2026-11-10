//! Time signatures other than 4/4, one a project: the beat and the bar a signature makes, the
//! metronome's and the count-in's clicks, the project file that keeps it and the edit that sets
//! it. Expected values come from tests/fixtures/gen_p34_expected.py (p34_expected.json and its
//! projects), never from the engine.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, Edit, History, Touched};
use dawsome_engine::plan::PlayMode;
use dawsome_engine::project::{LoadError, PatternId, Project};
use dawsome_engine::realtime::{self, Player, RealtimeLink};
use dawsome_engine::time::{Ticks, TimeSignature};
use dawsome_engine::transport::Control;
use serde_json::Value;

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p34_expected.json"))
        .expect("p34_expected.json (run gen_p34_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("parses")
}

fn text(name: &str) -> String {
    fs::read_to_string(fixtures().join(name)).expect("a p34 fixture")
}

fn load(name: &str) -> Project {
    Project::load_json(&text(name)).expect("loads")
}

fn signature(value: &Value) -> TimeSignature {
    let beats = value[0].as_u64().expect("beats") as u8;
    let unit = value[1].as_u64().expect("unit") as u8;
    TimeSignature::new(beats, unit).expect("a signature the generator lists")
}

// Known answer 1: each signature's beat, bar, steps a bar and steps a beat at 96 ppq, and its
// text, from the generator's table (4/4 to 24/4 and 1/16).
#[test]
fn each_signature_makes_its_beat_and_bar() {
    let want = expected();
    for row in want["table"].as_array().expect("a table") {
        let sig = signature(&row["signature"]);
        let number = |key: &str| row[key].as_i64().expect(key);
        assert_eq!(sig.beat_ticks(96), Ticks(number("beat")), "{sig} beat");
        assert_eq!(sig.bar_ticks(96), Ticks(number("bar")), "{sig} bar");
        assert_eq!(
            i64::from(sig.steps_per_bar()),
            number("steps"),
            "{sig} steps"
        );
        assert_eq!(
            i64::from(sig.steps_per_beat()),
            number("group"),
            "{sig} group"
        );
        assert_eq!(
            sig.bars_to_ticks(3, 96),
            Ticks(3 * number("bar")),
            "{sig} 3 bars"
        );
        let shown = format!("{}/{}", row["signature"][0], row["signature"][1]);
        assert_eq!(sig.to_string(), shown);
    }
    assert_eq!(TimeSignature::COMMON, signature(&serde_json::json!([4, 4])));
    assert_eq!(TimeSignature::default(), TimeSignature::COMMON);
}

// Known answer 1, the typed texts: each reads as the generator says, or is refused with the one
// message naming the ranges; beats 0 and 25 and the units 2, 3 and 32 are no signature at all.
#[test]
fn a_typed_signature_reads_or_is_refused_with_the_ranges() {
    let want = expected();
    let message = want["message"].as_str().expect("the message");
    for case in want["typed"].as_array().expect("typed") {
        let typed = case[0].as_str().expect("a text");
        let got = typed.parse::<TimeSignature>();
        match &case[1] {
            Value::Null => assert_eq!(
                got.map_err(|error| error.to_string()),
                Err(message.to_owned()),
                "{typed:?}"
            ),
            pair => assert_eq!(got, Ok(signature(pair)), "{typed:?}"),
        }
    }
    for (beats, unit) in [(0, 4), (25, 4), (3, 2), (3, 3), (3, 32)] {
        assert_eq!(TimeSignature::new(beats, unit), None, "{beats}/{unit}");
    }
}

/// A player of `project`'s pattern 1, its files one frame of silence, the metronome on as
/// `metronome` says.
fn player(project: &Project, metronome: bool) -> (RealtimeLink, Player) {
    let silence = |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(vec![0.0]));
    let update = Compiler::new()
        .compile(
            project,
            &Touched::default(),
            PlayMode::Pattern(PatternId(1)),
            silence,
        )
        .expect("compiles");
    let (mut link, player) = realtime::link(update, project.tempo);
    link.send(Control::Metronome(metronome)).expect("room");
    (link, player)
}

/// `frames` frames of `player`, a call per size of `sizes` (the last repeated).
fn render(player: &mut Player, frames: usize, sizes: &[usize]) -> Vec<[f32; 2]> {
    let mut out = vec![[f32::NAN; 2]; frames];
    let mut done = 0;
    let mut sizes = sizes.iter().cycle();
    while done < frames {
        let end = (done + sizes.next().copied().unwrap_or(512)).min(frames);
        player.process(&mut out[done..end]);
        done = end;
    }
    out
}

/// The frames `want` says sound, by frame, both sides the value.
fn sounding(want: &Value) -> BTreeMap<usize, f32> {
    want["nonzero"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|entry| {
            let at = entry[0].as_u64().expect("a frame") as usize;
            (at, entry[1].as_f64().expect("a value") as f32)
        })
        .collect()
}

// Known answer 2: an empty pattern in 3/4 at 120 BPM clicks every quarter (22050 frames apart),
// accented on ticks 0 and 288 (every third beat); in 6/8 every eighth (11025 frames apart),
// accented every sixth (ticks 0, 288, 576, 864); to the bit, whole and split.
#[test]
fn the_metronome_clicks_the_beats_and_bars_of_the_signature() {
    let want = expected();
    for (name, answer) in want["metronome"].as_object().expect("renders") {
        let project = load(answer["fixture"].as_str().expect("fixture"));
        let frames = answer["frames"].as_u64().expect("frames") as usize;
        let clicks = sounding(answer);
        for sizes in [&[frames][..], &[1], &[64], &[4096], &[13, 1, 997, 2, 64]] {
            let (mut link, mut player) = player(&project, true);
            link.send(Control::Play).expect("room");
            let out = render(&mut player, frames, sizes);
            for (at, got) in out.iter().enumerate() {
                let value = clicks.get(&at).copied().unwrap_or(0.0);
                assert_eq!(
                    got.map(f32::to_bits),
                    [value.to_bits(); 2],
                    "{name}, blocks {sizes:?}, frame {at}"
                );
            }
        }
    }
}

/// The count-in's clicks the generator lists: the burst at each, accented on the first.
fn count_in_clicks(answer: &Value, frames: usize, click_frames: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; frames];
    for (index, at) in answer["clicks"]
        .as_array()
        .expect("clicks")
        .iter()
        .enumerate()
    {
        let at = at.as_u64().expect("a frame") as usize;
        let amplitude = if index == 0 { 0.25 } else { 0.125 };
        for n in 0..click_frames {
            let sign = if (n / 8) % 2 == 0 { 1.0 } else { -1.0 };
            out[at + n] += amplitude * (1.0 - n as f32 / click_frames as f32) * sign;
        }
    }
    out
}

// Known answer 3: Record with a count-in clicks one bar of the signature's beats: three
// quarters in 3/4, six eighths in 6/8, seven in 7/8, the first accented; the song, and the
// recording, start where the bar ends (s(288), s(288), s(336)). The metronome is off, so the
// song after it is silent.
#[test]
fn the_count_in_is_one_bar_of_the_signatures_beats() {
    let want = expected();
    let click_frames = want["click_frames"].as_u64().expect("frames") as usize;
    for (name, answer) in want["count_in"].as_object().expect("count-ins") {
        let project = load(answer["fixture"].as_str().expect("fixture"));
        let start = answer["start"].as_u64().expect("a start") as usize;
        let (mut link, mut player) = player(&project, false);
        link.send(Control::Record {
            count_in: true,
            tap_master: false,
        })
        .expect("room");
        let out = render(&mut player, start + 1000, &[512]);
        let clicks = count_in_clicks(answer, start, click_frames);
        for (at, &value) in clicks.iter().enumerate() {
            assert_eq!(out[at], [value; 2], "{name}: count-in frame {at}");
        }
        assert!(
            out[start..].iter().all(|frame| *frame == [0.0; 2]),
            "{name}: the song after it"
        );
        assert_eq!(
            link.record_clock().recording_from(),
            Some(start as u64),
            "{name}: the recording starts where the count-in's bar ends"
        );
    }
}

// The audio thread's rule holds for a count-in of the most beats a bar holds: 24 clicks, no heap
// call.
#[test]
fn a_count_in_of_24_beats_never_touches_the_heap() {
    let mut project = load("p34_click_34.json");
    project.time_signature = TimeSignature::new(24, 16).expect("24/16");
    let (mut link, mut player) = player(&project, true);
    link.send(Control::Record {
        count_in: true,
        tap_master: false,
    })
    .expect("room");
    let mut out = vec![[0.0; 2]; 512];
    let calls: usize = (0..200)
        .map(|_| heap_calls_during(|| player.process(&mut out)))
        .sum();
    assert_eq!(calls, 0, "Player::process made {calls} heap calls");
}

// Known answer 5: the 4/4 project's bytes are what they were (version 2, no field); the 3/4
// one's are version 10 with the field after "ppq"; each loads and saves as its own bytes, and
// the 4/4 one given 3/4 saves as the 3/4 file.
#[test]
fn the_file_keeps_the_signature_and_leaves_4_4_as_it_was() {
    let want = expected();
    for key in ["common", "three_four"] {
        let name = want["file"][key].as_str().expect("a file");
        let bytes = text(name);
        let saved = load(name).to_json().expect("saves");
        assert_eq!(saved, bytes.trim_end(), "{name}");
    }
    let common = text(want["file"]["common"].as_str().expect("a file"));
    assert!(
        common.contains(r#""format_version":2,"#),
        "4/4 stays version 2"
    );
    assert!(!common.contains("time_signature"), "4/4 writes no field");
    let mut project = Project::load_json(&common).expect("loads");
    assert_eq!(project.time_signature, TimeSignature::COMMON);
    project.time_signature = TimeSignature::new(3, 4).expect("3/4");
    let three_four = text(want["file"]["three_four"].as_str().expect("a file"));
    assert_eq!(project.to_json().expect("saves"), three_four.trim_end());
    assert!(three_four.contains(r#""format_version":10,"#));
    assert!(three_four.contains(r#""ppq":96,"time_signature":{"beats":3,"unit":4},"#));
}

// A file holding a signature this build cannot play is refused, naming it; a version past 10
// is refused naming every version this build reads, 10 the last.
#[test]
fn a_file_with_a_signature_out_of_range_or_a_later_version_is_refused() {
    let three_four = text("p34_file_34.json");
    for (bad, shown) in [
        (r#"{"beats":0,"unit":4}"#, "0/4"),
        (r#"{"beats":25,"unit":4}"#, "25/4"),
        (r#"{"beats":3,"unit":2}"#, "3/2"),
        (r#"{"beats":3,"unit":32}"#, "3/32"),
    ] {
        let file = three_four.replace(r#"{"beats":3,"unit":4}"#, bad);
        let error = Project::load_json(&file).expect_err("refused").to_string();
        assert!(
            error.contains(shown) && error.contains("1 to 24 beats over 4, 8 or 16"),
            "{bad}: {error}"
        );
    }
    let later = three_four.replace(r#""format_version":10"#, r#""format_version":11"#);
    match Project::load_json(&later) {
        Err(error @ LoadError::UnsupportedVersion(11)) => assert_eq!(
            error.to_string(),
            "format_version 11 is not supported (this build reads versions 1, 2, 4, 5, 6, 7, \
             8, 9 and 10)"
        ),
        other => panic!("refused by its version: {other:?}"),
    }
}

// Known answer 6: setting 3/4 grows pattern 1 (a hit at 300) and pattern 3 (a note ending at
// 400) to 2 bars, keeps pattern 2 at its 2 bars (never shorter) and the clip at 384 ticks: the
// generator's document byte for byte; undo gives the first file's bytes back, redo the second's;
// 3/4 again changes nothing and touches nothing.
#[test]
fn setting_the_signature_grows_patterns_never_shortens_and_undoes() {
    let want = expected();
    let before = text(want["edit"]["before"].as_str().expect("a file"));
    let after = text(want["edit"]["after"].as_str().expect("a file"));
    let sig = signature(&want["edit"]["signature"]);
    let mut project = Project::load_json(&before).expect("loads");
    let mut history = History::new();
    let touched = history
        .apply(&mut project, Edit::SetTimeSignature { signature: sig })
        .expect("applies");
    assert_eq!(project.to_json().expect("saves"), after.trim_end());
    assert_eq!(
        touched.patterns,
        project.patterns.keys().copied().collect(),
        "every pattern is compiled again: its bar changed"
    );
    let clip_length = want["edit"]["clip_length"].as_i64().expect("a length");
    assert!(project
        .playlist
        .clips
        .values()
        .all(|clip| clip.length == Ticks(clip_length)));
    history.undo(&mut project).expect("undoes");
    assert_eq!(project.to_json().expect("saves"), before.trim_end());
    history.redo(&mut project).expect("redoes");
    assert_eq!(project.to_json().expect("saves"), after.trim_end());
    let again =
        edit::apply(&mut project, Edit::SetTimeSignature { signature: sig }).expect("applies");
    assert!(
        again.touched.is_empty(),
        "the same signature touches nothing"
    );
    assert_eq!(project.to_json().expect("saves"), after.trim_end());
}

// The plan plays the pattern's length in the signature: 6/8's empty one-bar pattern given a hit
// on tick 0 (velocity 1, a one-frame sound of 1.0 at unit gain) loops every 288 ticks, so at 120
// BPM and 44.1 kHz its second pass starts at s(288) = 288 · 229.6875 = 66150 (a 4/4 bar would put
// it at s(384) = 88200): two hits in 2 · 66150 frames, nothing else.
#[test]
fn the_plan_loops_the_pattern_at_the_signatures_bar() {
    let mut project = load("p34_click_68.json");
    project
        .patterns
        .get_mut(&PatternId(1))
        .expect("pattern 1")
        .notes
        .push(dawsome_engine::project::Note {
            channel: dawsome_engine::project::ChannelId(1),
            start: Ticks(0),
            length: Ticks(0),
            velocity: 1.0,
            key: dawsome_engine::project::DEFAULT_KEY,
        });
    project.validate().expect("valid");
    let update = Compiler::new()
        .compile(
            &project,
            &Touched::default(),
            PlayMode::Pattern(PatternId(1)),
            |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(vec![1.0])),
        )
        .expect("compiles");
    let (mut link, mut player) = realtime::link(update, project.tempo);
    link.send(Control::Play).expect("room");
    let out = render(&mut player, 2 * 66150, &[512]);
    for (at, frame) in out.iter().enumerate() {
        let want = if at == 0 || at == 66150 { 1.0 } else { 0.0 };
        assert_eq!(*frame, [want; 2], "frame {at}");
    }
}

// RestoreTimeSignature is exact whatever it is given: 4/4 (the signature p34_edit has) with
// pattern 1 at 3 bars changes pattern 1 alone (1 -> 3), touches it alone, and its inverse puts
// back 1 bar, the first file's bytes; that inverse's own inverse gives 3 again. A pattern named
// twice, (1, 3) then (1, 5), ends at 5, and the inverse brings it back to 1.
#[test]
fn restoring_a_signature_is_exact_with_or_without_a_change() {
    let before = text("p34_edit.json");
    let mut project = Project::load_json(&before).expect("loads");
    let common = TimeSignature::COMMON;
    let bars = |project: &Project| project.patterns[&PatternId(1)].length_bars;
    let applied = edit::apply(
        &mut project,
        Edit::RestoreTimeSignature {
            signature: common,
            lengths: vec![(PatternId(1), 3)],
            // p34_edit has no changes along the song: the list it puts back is empty.
            changes: Vec::new(),
        },
    )
    .expect("applies");
    assert_eq!(bars(&project), 3);
    assert_eq!(
        applied.touched.patterns,
        std::collections::BTreeSet::from([PatternId(1)])
    );
    let back = edit::apply(&mut project, applied.inverse).expect("the inverse applies");
    assert_eq!(project.to_json().expect("saves"), before.trim_end());
    edit::apply(&mut project, back.inverse).expect("and its inverse");
    assert_eq!(bars(&project), 3);
    let mut project = Project::load_json(&before).expect("loads");
    let twice = edit::apply(
        &mut project,
        Edit::RestoreTimeSignature {
            signature: common,
            lengths: vec![(PatternId(1), 3), (PatternId(1), 5)],
            changes: Vec::new(),
        },
    )
    .expect("applies");
    assert_eq!(bars(&project), 5);
    edit::apply(&mut project, twice.inverse).expect("the inverse applies");
    assert_eq!(project.to_json().expect("saves"), before.trim_end());
}

// A hand-made note of an absurd length (i64::MAX / 2 ticks, from tick 300 of p34_edit's one-bar
// pattern 1) grows the pattern only to the longest song, MAX_SONG_BARS = 100,000 bars, as a MIDI
// file's notes are held (not to u32::MAX bars); the project stays valid and undo gives 1 bar back.
#[test]
fn a_note_of_an_absurd_length_grows_its_pattern_to_the_longest_song() {
    let mut project = load("p34_edit.json");
    let pattern = project.patterns.get_mut(&PatternId(1)).expect("pattern 1");
    pattern.notes[0].length = Ticks(i64::MAX / 2);
    project.validate().expect("valid");
    let applied = edit::apply(
        &mut project,
        Edit::SetTimeSignature {
            signature: TimeSignature::new(3, 4).expect("3/4"),
        },
    )
    .expect("applies");
    assert_eq!(project.patterns[&PatternId(1)].length_bars, 100_000);
    project.validate().expect("still valid");
    edit::apply(&mut project, applied.inverse).expect("undoes");
    assert_eq!(project.patterns[&PatternId(1)].length_bars, 1);
}
