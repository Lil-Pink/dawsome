//! The external input channel through the command line: each render of
//! tests/fixtures/gen_p16_expected.py (p16_expected.json), fed tests/fixtures/p16_input.wav with
//! `--input-feed`, gives exactly the frames the generator computed (their SHA-256 and probes), the
//! same bytes at blocks of 1, 7, 64 and 4096 frames, and silence with no feed. Every run gets
//! per-user folders of its own under the test's temp dir.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use sha2::{Digest, Sha256};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> PathBuf {
    repo_root().join("tests/fixtures")
}

/// Fresh empty directory under cargo's per-target test directory.
fn temp_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("input")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Runs `dawsome render <project> <out> <extra>` with every per-user folder under `home`.
fn run_render(project: &Path, out: &Path, extra: &[&str], home: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dawsome"))
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
        .expect("run dawsome")
}

/// [`run_render`], requiring success.
fn render(project: &Path, out: &Path, extra: &[&str], home: &Path) {
    let output = run_render(project, out, extra, home);
    assert!(
        output.status.success(),
        "dawsome render {} {extra:?} failed: {}",
        project.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A render's frames, left and right.
fn read_stereo(path: &Path) -> Vec<[f32; 2]> {
    let mut reader = hound::WavReader::open(path).expect("open rendered wav");
    let spec = reader.spec();
    assert_eq!(spec.channels, 2);
    assert_eq!(spec.sample_format, hound::SampleFormat::Float);
    let values: Vec<f32> = reader
        .samples::<f32>()
        .map(|v| v.expect("decodes"))
        .collect();
    values
        .chunks_exact(2)
        .map(|pair| [pair[0], pair[1]])
        .collect()
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p16_expected.json")).expect("p16_expected");
    serde_json::from_str(&text).expect("parses")
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

/// Asserts `frames` is render `name` of the generator: its length, its probes to the bit, how
/// many frames sound, and the hash of every frame.
fn assert_render(name: &str, frames: &[[f32; 2]], want: &Value) {
    let length = want["frames"].as_u64().expect("frames");
    assert_eq!(frames.len() as u64, length, "{name}: frames");
    for probe in want["probes"].as_array().expect("probes") {
        let at = probe[0].as_u64().expect("a frame") as usize;
        let side = |index: usize| probe[index].as_f64().expect("a value") as f32;
        let wanted = [side(1), side(2)];
        assert_eq!(
            frames[at].map(f32::to_bits),
            wanted.map(f32::to_bits),
            "{name}, frame {at}: {:?} != {wanted:?}",
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
        frames_hash(frames),
        want["sha256"].as_str().expect("a hash"),
        "{name}: the frames differ from the generator's"
    );
}

fn input_wav() -> String {
    fixtures().join("p16_input.wav").display().to_string()
}

/// Known answers 1 to 4 (unity, volume and pan by both laws, mute beside a sampler, an insert,
/// an underrun past the feed's end), through the real entry point.
#[test]
fn p16_input_channels_render_their_known_answers() {
    let want = expected();
    assert_eq!(want["feed_delay"].as_u64(), Some(0));
    let dir = temp_dir("known");
    let feed = input_wav();
    for (name, answer) in want["renders"].as_object().expect("renders") {
        let project = fixtures().join(answer["fixture"].as_str().expect("fixture"));
        let bars = answer["bars"].as_u64().expect("bars").to_string();
        let out = dir.join(format!("{name}.wav"));
        render(
            &project,
            &out,
            &["--input-feed", &feed, "--bars", &bars],
            &dir,
        );
        assert_render(name, &read_stereo(&out), answer);
    }
}

/// Known answer 5: blocks of 1, 7, 64 and 4096 frames give the bytes of the default 512.
#[test]
fn p16_input_renders_do_not_depend_on_the_block_size() {
    let want = expected();
    let dir = temp_dir("blocks");
    let feed = input_wav();
    for (name, answer) in want["renders"].as_object().expect("renders") {
        let project = fixtures().join(answer["fixture"].as_str().expect("fixture"));
        let bars = answer["bars"].as_u64().expect("bars").to_string();
        let whole_path = dir.join(format!("{name}.wav"));
        render(
            &project,
            &whole_path,
            &["--input-feed", &feed, "--bars", &bars],
            &dir,
        );
        let whole = fs::read(&whole_path).expect("the render");
        for block in ["1", "7", "64", "4096"] {
            let split = dir.join(format!("{name}-{block}.wav"));
            render(
                &project,
                &split,
                &["--input-feed", &feed, "--bars", &bars, "--block", block],
                &dir,
            );
            assert!(
                fs::read(&split).expect("the render") == whole,
                "{name}: --block {block} renders other bytes"
            );
        }
    }
}

/// An offline render of an input channel with no feed is silence: a render must not depend on
/// what a device happens to deliver. The muted case's sampler still plays its four hits.
#[test]
fn an_input_channel_without_a_feed_renders_silence() {
    let dir = temp_dir("no_feed");
    let out = dir.join("unity.wav");
    render(&fixtures().join("p16_unity.json"), &out, &[], &dir);
    let frames = read_stereo(&out);
    assert_eq!(frames.len(), 48_000);
    assert!(frames.iter().all(|frame| *frame == [0.0; 2]), "silence");
    let out = dir.join("muted.wav");
    render(&fixtures().join("p16_muted.json"), &out, &[], &dir);
    let sounding: Vec<usize> = read_stereo(&out)
        .iter()
        .enumerate()
        .filter(|(_, frame)| **frame != [0.0; 2])
        .map(|(at, _)| at)
        .collect();
    assert_eq!(sounding, [0, 12_000, 24_000, 36_000]);
}

/// A feed at another rate than the project's is refused by name (no resampling in this
/// slice), as is a feed file that cannot be read; neither writes the output.
#[test]
fn a_feed_at_another_rate_or_unreadable_is_refused() {
    let dir = temp_dir("refused");
    let other_rate = fixtures().join("impulse.wav").display().to_string();
    let out = dir.join("out.wav");
    let output = run_render(
        &fixtures().join("p16_unity.json"),
        &out,
        &["--input-feed", &other_rate],
        &dir,
    );
    assert!(!output.status.success(), "a 44.1 kHz feed is refused");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("44100") && stderr.contains("48000"),
        "names both rates: {stderr}"
    );
    assert!(!out.exists(), "no output written");
    let missing = dir.join("missing.wav").display().to_string();
    let output = run_render(
        &fixtures().join("p16_unity.json"),
        &out,
        &["--input-feed", &missing],
        &dir,
    );
    assert!(!output.status.success(), "a missing feed is refused");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("missing.wav"),
        "names the file"
    );
    assert!(!out.exists(), "no output written");
}
