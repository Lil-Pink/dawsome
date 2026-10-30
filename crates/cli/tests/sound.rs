//! A channel's sound settings through the command line: each case of
//! tests/fixtures/gen_p13_expected.py renders the frames it states (p13_expected.json), and the
//! same bytes at any block size. Every run gets per-user folders of its own under the test's
//! temp dir.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> PathBuf {
    repo_root().join("tests/fixtures")
}

/// Fresh empty directory under cargo's per-target test directory.
fn temp_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("sound")
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

/// Asserts `frames` is the expected render: every sounding frame to the bit and all else
/// silence; for the filter, its probes to the case's tolerance and its sounding count.
fn assert_render(name: &str, frames: &[[f32; 2]], want: &Value) {
    assert_eq!(
        frames.len() as u64,
        want["frames"].as_u64().expect("frames"),
        "{name}"
    );
    if let Some(tolerance) = want["tolerance"].as_f64() {
        for probe in want["probes"].as_array().expect("probes") {
            let at = probe[0].as_u64().expect("a frame") as usize;
            for side in 0..2 {
                let wanted = probe[side + 1].as_f64().expect("a value");
                let got = f64::from(frames[at][side]);
                assert!(
                    (got - wanted).abs() <= tolerance,
                    "{name}, {at}: {got} != {wanted}"
                );
            }
        }
        let sounding = frames.iter().filter(|frame| frame[0] != 0.0).count();
        assert_eq!(
            sounding as u64,
            want["sounding"].as_u64().expect("count"),
            "{name}"
        );
        return;
    }
    let mut sounding = std::collections::BTreeMap::new();
    for entry in want["nonzero"].as_array().expect("a list") {
        let at = entry[0].as_u64().expect("a frame") as usize;
        let side = |index: usize| entry[index].as_f64().expect("a value") as f32;
        sounding.insert(at, [side(1), side(2)]);
    }
    for (at, got) in frames.iter().enumerate() {
        let want = sounding.get(&at).copied().unwrap_or([0.0; 2]);
        assert_eq!(
            got.map(f32::to_bits),
            want.map(f32::to_bits),
            "{name}, frame {at}: {got:?} != {want:?}"
        );
    }
}

/// Known answers (gen_p13_expected.py) through the real entry point, and the same bytes with
/// blocks of 1 and 7 frames as with the default 512.
#[test]
fn p13_sound_settings_render_their_known_answers() {
    let text = fs::read_to_string(fixtures().join("p13_expected.json")).expect("p13_expected");
    let want: Value = serde_json::from_str(&text).expect("parses");
    let dir = temp_dir("known");
    for (name, answer) in want["renders"].as_object().expect("renders") {
        let project = fixtures().join(answer["fixture"].as_str().expect("fixture"));
        let out = dir.join(format!("{name}.wav"));
        render(&project, &out, &[], &dir);
        assert_render(name, &read_stereo(&out), answer);
        let whole = fs::read(&out).expect("the render");
        for block in ["1", "7"] {
            let split = dir.join(format!("{name}-{block}.wav"));
            render(&project, &split, &["--block", block], &dir);
            assert!(
                fs::read(&split).expect("the render") == whole,
                "{name}: --block {block} renders other bytes"
            );
        }
    }
}
