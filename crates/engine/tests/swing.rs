//! Swing: a pattern's `swing_tenths` plays every note on the second 16th of an 8th-note pair
//! late, at compile time, the note's end with it, while the stored notes stay straight. The
//! player renders each case of tests/fixtures/gen_p31_expected.py to the frames it states
//! (p31_expected.json), however a render is split and without touching the heap; the song's
//! notes the exports read are the swung ones; the swing round-trips through the file, the
//! default (500, straight) is written nowhere, and a swing outside 50 to 75 % is refused.
//! Expected values come from the generator, never from the engine.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::Touched;
use dawsome_engine::plan::PlayMode;
use dawsome_engine::project::{ClipId, LoadError, PatternId, Project, ValidationError};
use dawsome_engine::realtime::{self, Player, RealtimeLink};
use dawsome_engine::song;
use dawsome_engine::time::{Ticks, STRAIGHT_SWING};
use dawsome_engine::transport::Control;
use serde_json::Value;

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p31_expected.json"))
        .expect("p31_expected.json (run gen_p31_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("parses")
}

fn text(name: &str) -> String {
    fs::read_to_string(fixtures().join(name)).expect("a p31 fixture")
}

fn load(name: &str) -> Project {
    Project::load_json(&text(name)).expect("loads")
}

/// A fixture's sound, read from its WAV (mono, at the projects' rate, so played as it is).
fn sound(path: &Path) -> Result<SampleBuffer, Infallible> {
    let mut reader = hound::WavReader::open(fixtures().join(path)).expect("a p31 sound");
    let values: Vec<f32> = reader
        .samples::<f32>()
        .map(|value| value.expect("reads"))
        .collect();
    Ok(SampleBuffer::from(values))
}

/// A player of `project`'s pattern 1 (its song when `song`), playing.
fn playing(project: &Project, song: bool) -> (RealtimeLink, Player) {
    let mode = if song {
        PlayMode::Song
    } else {
        PlayMode::Pattern(PatternId(1))
    };
    let update = Compiler::new()
        .compile(project, &Touched::default(), mode, sound)
        .expect("compiles");
    let (mut link, player) = realtime::link(update, project.tempo);
    link.send(Control::Play).expect("room");
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

/// Block sizes from 1 to 4096, drawn by a fixed xorshift, so every run splits alike.
fn seeded_sizes(count: usize) -> Vec<usize> {
    let mut state = 0x9e37_79b9_u32;
    (0..count)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as usize % 4096 + 1
        })
        .collect()
}

/// The frames `want` says sound, by frame, both sides the value: each run [first, frames,
/// value] spread out.
fn sounding(want: &Value) -> BTreeMap<usize, f32> {
    let mut frames = BTreeMap::new();
    for run in want["runs"].as_array().expect("a list") {
        let first = run[0].as_u64().expect("a frame") as usize;
        let count = run[1].as_u64().expect("a count") as usize;
        let value = run[2].as_f64().expect("a value") as f32;
        frames.extend((first..first + count).map(|at| (at, value)));
    }
    frames
}

// Known answers (gen_p31_expected.py): the 16 steps straight (with and without the field), at
// 66.7 % (ticks 48p + 32) and at 75 % and 130 BPM (each floored from the absolute tick, the
// second bar's second hit at s(420) = 96923); a swung hit passing a hand-placed one and a held
// note whose end moved with its start; a song whose clips' windows play the swung ticks. To the
// bit, whole and split into blocks of 1, of primes, of 64, of 4096 and of seeded sizes.
#[test]
fn every_swing_case_plays_its_known_answer_however_split() {
    let want = expected();
    let seeded = seeded_sizes(997);
    for (name, answer) in want["renders"].as_object().expect("renders") {
        let project = load(answer["fixture"].as_str().expect("fixture"));
        let frames = answer["frames"].as_u64().expect("frames") as usize;
        let song = answer["song"].as_bool().expect("a mode");
        let heard = sounding(answer);
        for sizes in [
            &[frames][..],
            &[1],
            &[7, 13, 61, 4093],
            &[64],
            &[4096],
            &seeded,
        ] {
            let (_link, mut player) = playing(&project, song);
            let out = render(&mut player, frames, sizes);
            for (at, got) in out.iter().enumerate() {
                let value = heard.get(&at).copied().unwrap_or(0.0);
                assert_eq!(
                    got.map(f32::to_bits),
                    [value.to_bits(); 2],
                    "{name}, blocks {:?}, frame {at}",
                    &sizes[..sizes.len().min(4)]
                );
            }
        }
    }
}

// The audio thread's rule: playing a swung pattern and a swung song makes no heap call.
#[test]
fn swing_never_touches_the_heap() {
    for (fixture, song) in [("p31_triplet_120.json", false), ("p31_song.json", true)] {
        let project = load(fixture);
        let (_link, mut player) = playing(&project, song);
        let mut out = vec![[0.0; 2]; 512];
        let mut calls = 0;
        // 600 blocks of 512 frames: past the end of both renders, a wrap included.
        for _ in 0..600 {
            calls += heap_calls_during(|| player.process(&mut out));
        }
        assert_eq!(
            calls, 0,
            "{fixture}: Player::process made {calls} heap calls"
        );
    }
}

// Known answer (e), as the exports read it (song::clip_notes): clip A from offset 28 plays the
// swung 32 and 80 at 4 and 52, each a moved step hit lasting 16, and 0 at 356 for a step; clip
// B plays the pattern twice from 384.
#[test]
fn the_songs_notes_are_the_swung_ticks_its_clips_play() {
    let project = load("p31_song.json");
    let mut notes: Vec<[i64; 2]> = project
        .playlist
        .clips
        .values()
        .flat_map(|clip| song::clip_notes(&project, clip))
        .map(|note| [note.start.0, note.end.0])
        .collect();
    notes.sort_unstable();
    let want: Vec<[i64; 2]> =
        serde_json::from_value(expected()["song_notes"].clone()).expect("(start, end) pairs");
    assert_eq!(notes, want);
}

// Known answer (a): a pattern at 500 is straight, the default. A file that writes it loads
// equal to one that does not, and saves as that one's bytes: 500 is written nowhere, so every
// project made before swing keeps its bytes.
#[test]
fn straight_swing_is_the_default_and_written_nowhere() {
    let plain = load("p31_plain.json");
    let straight = load("p31_straight.json");
    assert_eq!(plain.patterns[&PatternId(1)].swing_tenths, STRAIGHT_SWING);
    assert_eq!(straight, plain);
    assert_eq!(
        straight.to_json().expect("serializes"),
        text("p31_plain.json").trim_end_matches('\n')
    );
}

// A swung pattern saves its swing back where the file had it, after the length and before
// the notes; the song's clip at offset 28 loads (validation takes any offset from 0).
#[test]
fn a_swung_pattern_round_trips_through_the_file() {
    for fixture in ["p31_triplet_120.json", "p31_offgrid.json", "p31_song.json"] {
        let project = load(fixture);
        assert_eq!(
            project.to_json().expect("serializes"),
            text(fixture).trim_end_matches('\n'),
            "{fixture}"
        );
    }
    let song = load("p31_song.json");
    assert_eq!(song.patterns[&PatternId(1)].swing_tenths, 667);
    assert_eq!(song.playlist.clips[&ClipId(1)].offset, Ticks(28));
}

// The knob's range is 50 to 75 %: 499 and 751 are refused when a file is read, 500 and 750
// load.
#[test]
fn a_swing_outside_fifty_to_seventy_five_percent_is_refused() {
    let triplet = text("p31_triplet_120.json");
    let with = |swing: u16| {
        triplet.replace(
            r#""swing_tenths":667"#,
            &format!(r#""swing_tenths":{swing}"#),
        )
    };
    for swing in [499, 751, 0, 1000] {
        match Project::load_json(&with(swing)) {
            Err(LoadError::Invalid(ValidationError::InvalidSwing {
                pattern,
                swing_tenths,
            })) => {
                assert_eq!(pattern, "Pattern 1");
                assert_eq!(swing_tenths, swing);
            }
            other => panic!("swing {swing}: {other:?}"),
        }
    }
    for swing in [500, 750] {
        let project = Project::load_json(&with(swing)).expect("in range");
        assert_eq!(project.patterns[&PatternId(1)].swing_tenths, swing);
    }
}
