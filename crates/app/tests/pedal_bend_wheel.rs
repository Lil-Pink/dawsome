//! The sustain pedal, pitch bend and the mod wheel through the real door: `dawsome-app --play-to
//! --midi-events` plays a device's CC 64, pitch bend and CC 1 messages as a connected port
//! delivers them, and its render, the notes a take records and the MIDI-out log must be what
//! tests/fixtures/gen_p22_expected.py computed from the rules it states (p22_expected.json).
//!
//! The renders are compared to the bit by their SHA-256 (32-bit floats, little-endian, left then
//! right), their probes and their count of sounding frames; the filter's and the vibrato's,
//! whose values come from sin and cos, to 1e-6 at every 32nd frame.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha256};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

fn app_binary() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_dawsome-app"))
}

/// A fresh directory for one test's files.
fn out_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("p22_{test}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn run(command: &mut Command) {
    let output = command.output().expect("binary starts");
    assert!(
        output.status.success(),
        "{command:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn expected() -> Value {
    let text = fs::read_to_string(Path::new(FIXTURES).join("p22_expected.json"))
        .expect("p22_expected.json");
    serde_json::from_str(&text).expect("parses")
}

/// The frames of a WAV file, left and right.
fn frames_of(path: &Path) -> Vec<[f32; 2]> {
    let samples: Vec<f32> = hound::WavReader::open(path)
        .expect("readable wav")
        .samples::<f32>()
        .collect::<Result<_, _>>()
        .expect("float samples");
    samples
        .chunks_exact(2)
        .map(|pair| [pair[0], pair[1]])
        .collect()
}

/// Plays render `name` of p22_expected.json (its project and events, one bar) with `extra`
/// arguments, into `dir`; returns the frames.
fn play(dir: &Path, case: &Value, file: &str, extra: &[&str]) -> Vec<[f32; 2]> {
    let fixtures = Path::new(FIXTURES);
    let wav = dir.join(file);
    run(Command::new(app_binary())
        .arg("--load")
        .arg(fixtures.join(case["fixture"].as_str().expect("fixture")))
        .arg("--play-to")
        .arg(&wav)
        .args(["--bars", "1", "--midi-events"])
        .arg(fixtures.join(case["events"].as_str().expect("events")))
        .args(extra));
    frames_of(&wav)
}

/// The SHA-256 of `frames` as 32-bit floats, little-endian, left then right.
fn frames_hash(frames: &[[f32; 2]]) -> String {
    let mut hasher = Sha256::new();
    for frame in frames {
        hasher.update(frame[0].to_le_bytes());
        hasher.update(frame[1].to_le_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn json_f32(value: &Value) -> f32 {
    // The generator writes each f32 as the shortest decimal that names it, so `as f32` is exact.
    value.as_f64().expect("a number") as f32
}

/// Asserts `frames` is render `name` of the generator: its length, how many frames sound, its
/// probes (to the bit, or to its tolerance) and, without a tolerance, the hash of every frame.
fn assert_render(name: &str, frames: &[[f32; 2]], want: &Value) {
    let length = want["frames"].as_u64().expect("frames");
    assert_eq!(frames.len() as u64, length, "{name}: frames");
    let tolerance = want["tolerance"].as_f64();
    for probe in want["probes"].as_array().expect("probes") {
        let at = probe[0].as_u64().expect("a frame") as usize;
        let wanted = [json_f32(&probe[1]), json_f32(&probe[2])];
        match tolerance {
            Some(tolerance) => {
                for side in 0..2 {
                    let off = (f64::from(frames[at][side]) - f64::from(wanted[side])).abs();
                    assert!(
                        off <= tolerance,
                        "{name}, frame {at}: {:?} is not {wanted:?} within {tolerance}",
                        frames[at]
                    );
                }
            }
            None => assert_eq!(
                frames[at].map(f32::to_bits),
                wanted.map(f32::to_bits),
                "{name}, frame {at}: {:?} != {wanted:?}",
                frames[at]
            ),
        }
    }
    let sounding = frames.iter().filter(|frame| **frame != [0.0; 2]).count();
    assert_eq!(
        sounding as u64,
        want["sounding"].as_u64().expect("a count"),
        "{name}: sounding frames"
    );
    if tolerance.is_none() {
        assert_eq!(
            frames_hash(frames),
            want["sha256"].as_str().expect("a hash"),
            "{name}: the frames differ from the generator's"
        );
    }
}

/// Plays render `name` and checks it against the generator's; returns its frames.
fn checked(name: &str) -> Vec<[f32; 2]> {
    let want = &expected()["renders"][name];
    let dir = out_dir(name);
    let frames = play(&dir, want, "out.wav", &[]);
    assert_render(name, &frames, want);
    frames
}

/// Plays render `name` recording its notes (no quantize, no count-in) and returns the notes the
/// saved document's pattern holds.
fn taken(name: &str) -> Vec<Value> {
    let want = &expected()["renders"][name];
    let dir = out_dir(&format!("{name}_take"));
    let saved = dir.join("saved.json");
    let frames = play(
        &dir,
        want,
        "take.wav",
        &[
            "--record",
            "notes",
            "--count-in",
            "0",
            "--quantize",
            "off",
            "--save-to",
            saved.to_str().expect("utf-8 path"),
        ],
    );
    assert_render(&format!("{name} (recorded)"), &frames, want);
    let document: Value =
        serde_json::from_str(&fs::read_to_string(&saved).expect("saved")).expect("parses");
    document["patterns"][0]["notes"]
        .as_array()
        .expect("notes")
        .clone()
}

/// Asserts the notes a take saved are the generator's: channel, tick, length, key, velocity.
fn assert_notes(name: &str, got: &[Value], want: &Value) {
    let want = want.as_array().expect("notes");
    assert_eq!(got.len(), want.len(), "{name}: {got:?}");
    for (got, want) in got.iter().zip(want) {
        for field in ["channel", "tick", "length", "key"] {
            assert_eq!(got.get(field), want.get(field), "{name}, {field}: {got}");
        }
        assert_eq!(
            json_f32(&got["velocity"]),
            json_f32(&want["velocity"]),
            "{name}: {got}"
        );
    }
}

// (1) Pedal down, note on, note off, pedal up: the note sounds to the lift, as the same note held
// to that call does, byte for byte, and a take records it that long.
#[test]
fn p22_a_pedal_holds_a_note_to_its_lift_and_the_take_records_it() {
    let pedal = checked("pedal");
    let held = checked("held");
    assert!(pedal == held, "the pedal's note is the held note");
    let want = &expected()["renders"]["pedal"]["take"];
    assert_notes("pedal", &taken("pedal"), want);
}

// (2) A key struck again under the pedal ends its first voice there and starts anew; the lift
// ends the second. The take holds both notes.
#[test]
fn p22_a_key_struck_again_under_the_pedal_starts_anew() {
    checked("repress");
    let want = &expected()["renders"]["repress"]["take"];
    assert_notes("repress", &taken("repress"), want);
}

// (3) Pitch bend under a held note: +8191 plays 2^(2/12) from its call on, -8192 2^(-2/12), 0
// the zone's own speed, each reading on from where the voice was; at a range of 12, +8191 plays
// twice the speed.
#[test]
fn p22_a_bend_plays_from_its_call_at_the_bent_rate() {
    checked("bend");
    checked("range");
}

// (4) The mod wheel at 127 before a note on a filtered channel opens the filter 4 octaves
// (1 kHz to 16 kHz): the note plays as the same note with its filter set to 16 kHz by hand, to
// the bit, up to where the wheel goes back to 0; on an unfiltered channel it plays vibrato.
#[test]
fn p22_the_wheel_opens_the_filter_as_set_by_hand_or_plays_vibrato() {
    let wheel = checked("wheel");
    let hand = checked("wheel_hand");
    let until = expected()["wheel_same_until"].as_u64().expect("a frame") as usize;
    assert!(
        wheel[..until] == hand[..until],
        "the wheel's filter is the hand-set filter"
    );
    assert!(
        wheel[until..] != hand[until..],
        "back at 0 it is the channel's own filter"
    );
    checked("vibrato");
}

// (5) Pedal up with no note does nothing, a bend with no voice waits for the next note, CC 64 at
// 63 is up and at 64 down.
#[test]
fn p22_saturated_pedal_and_bend_messages() {
    checked("saturated");
}

// (6) A channel with a MIDI out sends the pedal, the bend and the wheel on as the same messages on
// its MIDI channel at their call's first frame, only when they change something; the pedal's lift
// goes out before the note off of the key it held; a quiet port's pedal sends nothing.
#[test]
fn p22_a_midi_out_channel_sends_pedal_bend_and_wheel_on() {
    let case = &expected()["out"];
    let fixtures = Path::new(FIXTURES);
    let dir = out_dir("out");
    let log = dir.join("midi_out.json");
    run(Command::new(app_binary())
        .arg("--load")
        .arg(fixtures.join(case["fixture"].as_str().expect("fixture")))
        .arg("--play-to")
        .arg(dir.join("out.wav"))
        .args(["--bars", "1", "--midi-events"])
        .arg(fixtures.join(case["events"].as_str().expect("events")))
        .arg("--midi-out-log")
        .arg(&log));
    let written: Value =
        serde_json::from_str(&fs::read_to_string(&log).expect("the log")).expect("parses");
    assert_eq!(written["events"], case["messages"]);
    assert_eq!(
        (&written["dropped"], &written["lost_offs"]),
        (&0.into(), &0.into())
    );
}

// (7) Controller links on CC 64 and CC 1 win: the pedal moves the channel's volume (to 4) and
// holds nothing, the wheel moves its pan (hard right) and plays no vibrato; the saved document
// holds both values. (Before the pedal and the wheel played anything every controller only moved
// links, so this holds from the start; it guards the parser's routing.)
#[test]
fn p22_a_link_on_cc_1_or_64_wins_over_the_wheel_and_the_pedal() {
    let want = expected();
    let case = &want["renders"]["link"];
    let dir = out_dir("link");
    let saved = dir.join("saved.json");
    let frames = play(
        &dir,
        case,
        "out.wav",
        &["--save-to", saved.to_str().expect("utf-8 path")],
    );
    assert_render("link", &frames, case);
    let document: Value =
        serde_json::from_str(&fs::read_to_string(&saved).expect("saved")).expect("parses");
    let channel = &document["channels"][0];
    assert_eq!(
        json_f32(&channel["volume"]),
        json_f32(&want["link"]["volume"])
    );
    assert_eq!(json_f32(&channel["pan"]), json_f32(&want["link"]["pan"]));
}
