//! Cut itself through the real door: `dawsome-app --play-to --midi-events` plays a device's
//! notes and its sustain pedal as a connected port delivers them, on a channel that cuts
//! itself, and its render must be what tests/fixtures/gen_p26_expected.py computed from the rules
//! it states (p26_expected.json), compared to the bit by its SHA-256 (32-bit floats,
//! little-endian, left then right), its probes and its count of sounding frames.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha256};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

/// A fresh directory for one test's files.
fn out_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("p26_{test}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn expected() -> Value {
    let text = fs::read_to_string(Path::new(FIXTURES).join("p26_expected.json"))
        .expect("p26_expected.json");
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

/// Plays live case `name` of p26_expected.json (its project and events, one bar) into `dir`;
/// returns the frames.
fn play(dir: &Path, name: &str) -> Vec<[f32; 2]> {
    let case = &expected()["live"][name];
    let fixtures = Path::new(FIXTURES);
    let wav = dir.join(format!("{name}.wav"));
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome-app"))
        .arg("--load")
        .arg(fixtures.join(case["fixture"].as_str().expect("fixture")))
        .arg("--play-to")
        .arg(&wav)
        .args(["--bars", "1", "--midi-events"])
        .arg(fixtures.join(case["events"].as_str().expect("events")))
        .output()
        .expect("binary starts");
    assert!(
        output.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    frames_of(&wav)
}

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

// Pedal down, note on, its note-off held by the pedal, another note: through the door, the held
// note falls over the release from the second note's start, as the generator says (and, without
// the flag, sounds to the lift).
#[test]
fn p26_a_note_the_pedal_holds_is_cut_through_the_door() {
    let dir = out_dir("pedal");
    for name in ["pedal", "pedal_nocut"] {
        let want = &expected()["live"][name];
        let frames = play(&dir, name);
        assert_eq!(
            frames.len() as u64,
            want["frames"].as_u64().expect("frames")
        );
        for probe in want["probes"].as_array().expect("probes") {
            let at = probe[0].as_u64().expect("a frame") as usize;
            let side = |index: usize| probe[index].as_f64().expect("a value") as f32;
            assert_eq!(
                frames[at].map(f32::to_bits),
                [side(1), side(2)].map(f32::to_bits),
                "{name}, frame {at}"
            );
        }
        let sounding = frames.iter().filter(|frame| **frame != [0.0; 2]).count();
        assert_eq!(sounding as u64, want["sounding"].as_u64().expect("a count"));
        assert_eq!(
            frames_hash(&frames),
            want["sha256"].as_str().expect("a hash"),
            "{name}"
        );
    }
}
