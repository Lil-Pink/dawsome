//! MIDI out through the command line: each case of tests/fixtures/gen_p19_expected.py
//! (p19_expected.json), rendered with `--midi-out-log`, sends exactly the messages the generator
//! computed, on the frames it computed, for the rack channels it names, and drops none, at blocks
//! of 1, 7, 64 and 4096 frames. The log is the player's MIDI-out ring drained after every call,
//! then once more after the Stop the render sends at its end. Every run gets per-user folders of
//! its own under the test's temp dir, and no MIDI device is opened.

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
        .join("midi_out")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p19_expected.json")).expect("p19_expected");
    serde_json::from_str(&text).expect("parses")
}

/// Renders `case` at `block` frames per call and returns the MIDI-out log it wrote.
fn midi_out_log(name: &str, case: &Value, block: usize, dir: &Path) -> Value {
    let fixture = case["fixture"].as_str().expect("fixture");
    let bars = case["bars"].as_u64().expect("bars").to_string();
    let block = block.to_string();
    let out = dir.join(format!("{name}_{block}.wav"));
    let log = dir.join(format!("{name}_{block}.json"));
    let mut command = Command::new(env!("CARGO_BIN_EXE_dawsome"));
    command
        .current_dir(repo_root())
        .env("DAWSOME_DATA_HOME", dir.join("data"))
        .env("DAWSOME_CACHE_HOME", dir.join("cache"))
        .env("DAWSOME_CONFIG_HOME", dir.join("config"))
        .env("DAWSOME_STATE_HOME", dir.join("state"))
        .arg("render")
        .arg(fixtures().join(fixture))
        .arg(&out)
        .args(["--bars", &bars, "--block", &block, "--midi-out-log"])
        .arg(&log);
    if case["song"].as_bool().expect("song") {
        command.arg("--song");
    }
    let output = command.output().expect("run dawsome");
    assert!(
        output.status.success(),
        "dawsome render {fixture} --block {block} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = fs::read_to_string(&log).expect("the log was written");
    serde_json::from_str(&text).expect("the log parses")
}

// Pattern A, song B (latency, a repeated clip cut at its end, the wrap sent ahead) and rack C
// (MIDI channels, mute, rack order, a retrigger, a note cut by Stop): every message on its frame
// in order, the same at every block size, none dropped.
#[test]
fn each_case_sends_the_known_messages_at_every_block_size() {
    let expected = expected();
    let cases = expected["cases"].as_object().expect("cases");
    assert_eq!(cases.len(), 3);
    let dir = temp_dir("cases");
    for (name, case) in cases {
        let want = &case["events"];
        assert!(!want.as_array().expect("events").is_empty(), "{name}");
        for block in [1, 7, 64, 4096] {
            let log = midi_out_log(name, case, block, &dir);
            assert_eq!(&log["events"], want, "{name} at block {block}");
            assert_eq!(log["dropped"], 0, "{name} at block {block}");
        }
    }
}
