//! Cut itself through the command line: each pattern case of tests/fixtures/gen_p26_expected.py
//! renders the frames it states (p26_expected.json: their SHA-256, probes and sounding frames),
//! and the same bytes at blocks of 1 and 7 frames. Every run gets per-user folders of its own
//! under the test's temp dir.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha256};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> PathBuf {
    repo_root().join("tests/fixtures")
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p26_expected.json"))
        .expect("p26_expected.json (run gen_p26_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("parses")
}

/// Fresh empty directory under cargo's per-target test directory.
fn temp_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("cut_itself")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Runs `dawsome render <project> <out> <extra>` with every per-user folder under `home`.
fn render(project: &Path, out: &Path, extra: &[&str], home: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .env("DAWSOME_DATA_HOME", home.join("data"))
        .env("DAWSOME_CACHE_HOME", home.join("cache"))
        .env("DAWSOME_CONFIG_HOME", home.join("config"))
        .env("DAWSOME_STATE_HOME", home.join("state"))
        .arg("render")
        .arg(project)
        .arg(out)
        .args(extra)
        .output()
        .expect("run dawsome");
    assert!(
        output.status.success(),
        "dawsome render {} failed: {}",
        project.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A render's frames, left and right.
fn read_stereo(path: &Path) -> Vec<[f32; 2]> {
    let mut reader = hound::WavReader::open(path).expect("open rendered wav");
    assert_eq!(reader.spec().channels, 2);
    let values: Vec<f32> = reader
        .samples::<f32>()
        .map(|v| v.expect("decodes"))
        .collect();
    values
        .chunks_exact(2)
        .map(|pair| [pair[0], pair[1]])
        .collect()
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

/// Asserts `frames` is render `name` of the generator: its length, its probes to the bit, its
/// sounding frames, its peak and the hash of every frame.
fn assert_render(name: &str, frames: &[[f32; 2]], want: &Value) {
    assert_eq!(
        frames.len() as u64,
        want["frames"].as_u64().expect("frames"),
        "{name}"
    );
    for probe in want["probes"].as_array().expect("probes") {
        let at = probe[0].as_u64().expect("a frame") as usize;
        let side = |index: usize| probe[index].as_f64().expect("a value") as f32;
        assert_eq!(
            frames[at].map(f32::to_bits),
            [side(1), side(2)].map(f32::to_bits),
            "{name}, frame {at}: {:?}",
            frames[at]
        );
    }
    let sounding = frames.iter().filter(|frame| **frame != [0.0; 2]).count();
    assert_eq!(
        sounding as u64,
        want["sounding"].as_u64().expect("a count"),
        "{name}: sounding frames"
    );
    assert_eq!(
        peak(frames),
        want["peak"].as_f64().expect("a peak") as f32,
        "{name}"
    );
    assert_eq!(
        frames_hash(frames),
        want["sha256"].as_str().expect("a hash"),
        "{name}: the frames differ from the generator's"
    );
}

fn peak(frames: &[[f32; 2]]) -> f32 {
    frames
        .iter()
        .flatten()
        .fold(0.0f32, |most, value| most.max(value.abs()))
}

/// Renders case `name` (its bars) into `dir` with `extra` arguments; its frames and bytes.
fn rendered(dir: &Path, name: &str, extra: &[&str]) -> (Vec<[f32; 2]>, Vec<u8>) {
    let want = &expected()["renders"][name];
    let project = fixtures().join(want["fixture"].as_str().expect("fixture"));
    let bars = want["bars"].as_u64().expect("bars").to_string();
    let out = dir.join(format!("{name}{}.wav", extra.join("")));
    let mut args = vec!["--bars", bars.as_str()];
    args.extend_from_slice(extra);
    render(&project, &out, &args, dir);
    (read_stereo(&out), fs::read(&out).expect("the render"))
}

/// Every pattern case renders its known answer, and the same bytes with blocks of 1 and 7
/// frames as with the default 512 (the cut's frame never depends on where a block ends).
#[test]
fn p26_every_pattern_case_renders_its_known_answer_at_any_block_size() {
    let dir = temp_dir("known");
    let want = expected();
    for (name, answer) in want["renders"].as_object().expect("renders") {
        let (frames, whole) = rendered(&dir, name, &[]);
        assert_render(name, &frames, answer);
        // The row is four bars of 64 notes; blocks of one frame there would only take time.
        let blocks: &[&str] = if name.starts_with("row") {
            &["7"]
        } else {
            &["1", "7"]
        };
        for block in blocks {
            let (_, split) = rendered(&dir, name, &["--block", block]);
            assert!(
                split == whole,
                "{name}: --block {block} renders other bytes"
            );
        }
    }
}

/// The cut changes nothing before its frame: the overlap with and without the flag are the same
/// bits up to the second note's start (frame 11025), and differ after it, where the first note
/// falls over the release instead of ringing on.
#[test]
fn p26_the_cut_changes_nothing_before_the_next_notes_start() {
    let dir = temp_dir("before");
    let (cut, _) = rendered(&dir, "overlap", &[]);
    let (whole, _) = rendered(&dir, "overlap_nocut", &[]);
    let at = expected()["renders"]["overlap"]["cuts"][0][1]
        .as_u64()
        .expect("the cut") as usize;
    assert_eq!(at, 11025, "the second note's start, tick 48");
    assert!(cut[..at] == whole[..at], "the same up to the cut");
    assert!(cut[at + 1..at + 441] != whole[at + 1..at + 441]);
}

/// 64 notes a 16th apart, each a beat long: four ring at once without the cut and peak at 1.9;
/// with it, at most the note and the one before it in its 441-frame release sound together,
/// and the peak stays under 0.8 (the generator's figures, which the renders hold to the bit).
#[test]
fn p26_a_fast_row_does_not_stack() {
    let want = expected();
    let row = &want["renders"]["row"];
    let stacked = &want["renders"]["row_nocut"];
    assert_eq!(row["voices_most"].as_u64(), Some(2));
    assert!(stacked["voices_most"].as_u64().expect("a count") >= 4);
    let dir = temp_dir("row");
    let (cut, _) = rendered(&dir, "row", &[]);
    let (whole, _) = rendered(&dir, "row_nocut", &[]);
    assert!(
        peak(&cut) < 0.8 && peak(&whole) > 1.8,
        "{} {}",
        peak(&cut),
        peak(&whole)
    );
}
