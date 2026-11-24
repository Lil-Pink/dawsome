//! Warp markers through `dawsome render`: every project gen_p39_expected.py writes renders as its
//! rules give (the resample read through the markers, at 48 and 44.1 kHz; no markers the uniform
//! stretch's bits; a split warped clip the uncut clip's bits, with fades too; a seam between two
//! lists declicked; 256 markers on a short sound), and a time-stretched warped clip's cached file
//! is exactly its stretch's length.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha256};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Fresh empty directory under cargo's per-target test directory (on the build drive).
fn temp_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// One bar of `project` (a path from the repository's root) in song mode, stretching into
/// `cache`: left and right.
fn render_bar(project: &str, out: &Path, cache: &Path) -> (Vec<f32>, Vec<f32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .arg("render")
        .arg(project)
        .arg(out)
        .args(["--song", "--bars", "1", "--cache-dir"])
        .arg(cache)
        .output()
        .expect("run dawsome binary");
    assert!(
        output.status.success(),
        "dawsome render {project} failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let mut reader = hound::WavReader::open(out).expect("open rendered wav");
    assert_eq!(reader.spec().channels, 2, "output must be stereo");
    let interleaved: Vec<f32> = reader
        .samples::<f32>()
        .collect::<Result<_, _>>()
        .expect("decode samples");
    let left = interleaved.iter().copied().step_by(2).collect();
    let right = interleaved.iter().copied().skip(1).step_by(2).collect();
    (left, right)
}

fn json_usize(value: &Value) -> usize {
    value
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .expect("a frame count")
}

fn json_f32(value: &Value) -> f32 {
    // The generator writes each f32 as the shortest f64 that names it, so `as f32` is exact.
    value.as_f64().expect("a number") as f32
}

fn expected() -> Value {
    let path = repo_root().join("tests/fixtures/p39_expected.json");
    let text = fs::read_to_string(&path).expect("p39_expected.json (run gen_p39_expected.py)");
    serde_json::from_str(&text).expect("parses")
}

// (2), (3), (4) and the messy case: by SHA-256 of the interleaved floats, with probes to say
// where when they differ.
#[test]
fn p39_warped_clips_render_as_their_rules_give() {
    for (name, want) in expected()["renders"].as_object().expect("renders") {
        let dir = temp_dir(&format!("p39_{name}"));
        let fixture = format!(
            "tests/fixtures/{}",
            want["fixture"].as_str().expect("fixture")
        );
        let (left, right) = render_bar(&fixture, &dir.join("out.wav"), &dir.join("cache"));
        assert_eq!(left.len(), json_usize(&want["frames"]), "{name}");
        for probe in want["probes"].as_array().expect("probes") {
            let at = json_usize(&probe[0]);
            let side = |index: usize| json_f32(&probe[index]);
            assert_eq!((left[at], right[at]), (side(1), side(2)), "{name} at {at}");
        }
        let mut hash = Sha256::new();
        for (l, r) in left.iter().zip(&right) {
            hash.update(l.to_le_bytes());
            hash.update(r.to_le_bytes());
        }
        let hex: String = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(hex, want["sha256"].as_str().expect("sha256"), "{name}");
    }
}

// (8) p39_warp3 in the mode that keeps the pitch: its one cached file, made through the markers,
// holds exactly the stretch's 30000 frames (no partial left), and a second render reads it back
// to the same bits.
#[test]
fn p39_a_time_stretched_warped_clip_is_exactly_its_length() {
    let dir = temp_dir("p39_stretch_mode");
    let text = fs::read_to_string(repo_root().join("tests/fixtures/p39_warp3.json")).expect("p39");
    let mut document: Value = serde_json::from_str(&text).expect("parses");
    document["playlist"]["clips"][0]["stretch"]["mode"] = "stretch".into();
    // The sound by its whole path, as the project now lies outside the fixtures.
    let sound = repo_root().join("tests/fixtures/p39_sound/tone.wav");
    document["channels"][0]["sample"] = sound.to_str().expect("utf-8 path").into();
    let project = dir.join("p39_warp3_stretch.json");
    fs::write(&project, document.to_string()).expect("writes");
    let cache = dir.join("cache");
    let path = project.to_str().expect("utf-8 path");
    let first = render_bar(path, &dir.join("one.wav"), &cache);
    let files: Vec<PathBuf> = fs::read_dir(&cache)
        .expect("the cache folder exists")
        .map(|entry| entry.expect("entry").path())
        .collect();
    assert_eq!(
        files.len(),
        1,
        "one cached file, no partial left: {files:?}"
    );
    let reader = hound::WavReader::open(&files[0]).expect("a WAV");
    assert_eq!(reader.duration(), 30000, "exactly T frames");
    let second = render_bar(path, &dir.join("two.wav"), &cache);
    assert_eq!(first, second, "read back from the cache");
    assert!(first.0.iter().any(|&value| value != 0.0), "it sounds");
}
