//! `dawsome render --midi-out-log` refuses a block longer than the MIDI-out ring (4096 messages,
//! read after every call): a longer call could fill the ring and drop notes, so the log would
//! depend on the block size. Without the log the block size is free, as before (found
//! 2026-10-25). Every run gets per-user folders of its own under the
//! test's temp dir, and no MIDI device is opened.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Fresh empty directory under cargo's per-target test directory.
fn temp_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("midi_out_block")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// `dawsome render` of the p19_pattern fixture's 2 bars (165375 frames) at `block` frames per
/// call, writing the MIDI-out log to `log` when given.
fn render(dir: &Path, block: &str, log: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dawsome"));
    command
        .current_dir(repo_root())
        .env("DAWSOME_DATA_HOME", dir.join("data"))
        .env("DAWSOME_CACHE_HOME", dir.join("cache"))
        .env("DAWSOME_CONFIG_HOME", dir.join("config"))
        .env("DAWSOME_STATE_HOME", dir.join("state"))
        .arg("render")
        .arg(repo_root().join("tests/fixtures/p19_pattern.json"))
        .arg(dir.join(format!("out_{block}.wav")))
        .args(["--bars", "2", "--block", block]);
    if let Some(log) = log {
        command.arg("--midi-out-log").arg(log);
    }
    command.output().expect("run dawsome")
}

// One frame past the ring, and the whole render in one call: refused, saying why, nothing
// written. The ring's own size renders, and its log drops nothing.
#[test]
fn a_block_longer_than_the_midi_out_ring_is_refused_with_the_log() {
    let dir = temp_dir("refused");
    for block in ["4097", "165375"] {
        let log = dir.join(format!("log_{block}.json"));
        let output = render(&dir, block, Some(&log));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "--block {block} rendered");
        assert!(
            stderr.contains("--block is at most 4096 with --midi-out-log"),
            "{stderr}"
        );
        assert!(!log.exists(), "--block {block} wrote a log");
        assert!(!dir.join(format!("out_{block}.wav")).exists());
    }
    let log = dir.join("log_4096.json");
    let output = render(&dir, "4096", Some(&log));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = fs::read_to_string(&log).expect("the log was written");
    let log: serde_json::Value = serde_json::from_str(&text).expect("the log parses");
    assert_eq!(log["dropped"], 0);
}

#[test]
fn without_the_log_a_long_block_still_renders() {
    let dir = temp_dir("no_log");
    let output = render(&dir, "165375", None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(dir.join("out_165375.wav").exists());
}
