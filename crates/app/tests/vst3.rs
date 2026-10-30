//! The app's side of the VST3 host through its own binary: its scan child (`--scan-child`, the
//! child the app's background scan starts) prints what tests/fixtures/gen_p28_expected.py says a
//! scan child prints for the VST3 test module, byte for byte, as the command line's
//! `scan --child` does (crates/cli/tests/vst3.rs holds that one to the same text).

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

#[path = "../../../tests/support/test_plugins.rs"]
mod test_plugins;

#[test]
fn the_apps_scan_child_prints_the_generators_text() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let text = fs::read_to_string(fixtures.join("p28_expected.json")).expect("p28_expected.json");
    let expected: Value = serde_json::from_str(&text).expect("parses");
    let home = Path::new(env!("CARGO_TARGET_TMPDIR")).join("vst3-scan-child");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).expect("temp dir");
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome-app"))
        .arg("--scan-child")
        .arg(test_plugins::test_plugins_vst3().join("Dawsome Test VST3.vst3"))
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("XDG_STATE_HOME", home.join("state"))
        .output()
        .expect("the app starts");
    assert!(output.status.success(), "{}", output.status);
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8"),
        expected["scan_child"].as_str().expect("a text")
    );
}

// A render that plays MIDI events refuses a project naming plugins of any format, VST3 ones too
// (a plugin project renders on a thread of its own, which the messages could not be timed
// against): before VST3 reached the app, the VST3 sine's project passed the check as a project
// without plugins and played it silent.
#[test]
fn midi_events_refuse_a_vst3_project_as_a_clap_one() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("vst3-midi-events");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome-app"))
        .arg("--load")
        .arg(fixtures.join("p27_vst3_sine.json"))
        .arg("--play-to")
        .arg(dir.join("out.wav"))
        .args(["--bars", "1", "--midi-events"])
        .arg(fixtures.join("p10_link_events.json"))
        .arg("--plugin-path")
        .arg(test_plugins::test_plugins_vst3())
        .arg("--cache")
        .arg(dir.join("catalogue.json"))
        .env("HOME", &dir)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_STATE_HOME", dir.join("state"))
        .output()
        .expect("the app starts");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("play projects without plugins only"),
        "{stderr}"
    );
}
