//! The VST3 host through the command line, with Dawsome's own VST3 test plugins
//! (crates/test-plugin-vst3, built into <target dir>/test-plugins-vst3 by
//! scripts/build-test-plugins.sh): `dawsome scan` lists the module's classes and survives the one
//! that aborts as it loads, and `dawsome render` plays the classes as a channel and as an effect.
//! Every expected value is read from tests/fixtures/p27_expected.json, which
//! tests/fixtures/gen_p27_expected.py writes from the test plugins' formulas. Every run passes
//! --plugin-path, a fresh --cache and per-user folders of its own, so no test reads or writes the
//! user's catalogue.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use sha2::{Digest, Sha256};

#[path = "../../../tests/support/test_plugins.rs"]
mod test_plugins;

use test_plugins::test_plugins_vst3;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> PathBuf {
    repo_root().join("tests/fixtures")
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p27_expected.json"))
        .expect("p27_expected.json (run gen_p27_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("parses")
}

/// Fresh empty directory under cargo's per-target test directory.
fn temp_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("vst3")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// `dawsome` with every per-user folder under `dir` (a plugin's code runs in the scan child and
/// the render, so it gets a home of its own too).
fn dawsome(dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dawsome"));
    command
        .current_dir(repo_root())
        .env("HOME", dir.join("home"))
        .env("XDG_CONFIG_HOME", dir.join("home/config"))
        .env("XDG_DATA_HOME", dir.join("home/data"))
        .env("XDG_CACHE_HOME", dir.join("home/cache"))
        .env("XDG_STATE_HOME", dir.join("home/state"))
        .env("DAWSOME_DATA_HOME", dir.join("data"))
        .env("DAWSOME_CACHE_HOME", dir.join("cache"))
        .env("DAWSOME_CONFIG_HOME", dir.join("config"))
        .env("DAWSOME_STATE_HOME", dir.join("state"));
    command
}

/// A copy of the VST3 test plugins' folder in `dir`, so a test can change it.
fn copied_plugins(dir: &Path) -> PathBuf {
    let plugins = dir.join("plugins");
    copy_tree(&test_plugins_vst3(), &plugins);
    plugins
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("folder");
    for entry in fs::read_dir(from).expect("reads") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copies");
        }
    }
}

/// Runs `dawsome scan` over `plugins` into `cache`, requiring exit 0; returns stdout.
fn scan(dir: &Path, plugins: &Path, cache: &Path) -> String {
    let output = dawsome(dir)
        .arg("scan")
        .arg("--plugin-path")
        .arg(plugins)
        .arg("--cache")
        .arg(cache)
        .output()
        .expect("run dawsome");
    assert!(
        output.status.success(),
        "dawsome scan failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Runs `dawsome render <fixture> <out> <extra>` with the VST3 test plugins (or `plugins`).
fn render_in(dir: &Path, plugins: &Path, fixture: &str, out: &Path, extra: &[&str]) -> Output {
    dawsome(dir)
        .arg("render")
        .arg(fixtures().join(fixture))
        .arg(out)
        .arg("--plugin-path")
        .arg(plugins)
        .arg("--cache")
        .arg(dir.join("catalogue.json"))
        .args(extra)
        .output()
        .expect("run dawsome")
}

/// [`render_in`] with the built test plugins, requiring success: the frames and stderr.
fn rendered(dir: &Path, fixture: &str, extra: &[&str]) -> (Vec<[f32; 2]>, String) {
    let out = dir.join(format!("{fixture}{}.wav", extra.concat()));
    let output = render_in(dir, &test_plugins_vst3(), fixture, &out, extra);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "dawsome render {fixture} {extra:?} failed ({}): {stderr}",
        output.status
    );
    (read_stereo(&out), stderr)
}

/// A render's frames, left and right.
fn read_stereo(path: &Path) -> Vec<[f32; 2]> {
    let mut reader = hound::WavReader::open(path).expect("open rendered wav");
    assert_eq!(reader.spec().channels, 2);
    assert_eq!(reader.spec().sample_format, hound::SampleFormat::Float);
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

/// Holds `frames` to the generator's answer for a render: its length, every probe within 1e-6
/// (the generator's values are f32 written as f64), how many frames sound, both sides alike, and
/// the frames' SHA-256, which pins every other frame to the bit.
fn assert_render(frames: &[[f32; 2]], answer: &Value) {
    let name = answer["fixture"].as_str().expect("fixture");
    assert_eq!(
        frames.len() as u64,
        answer["frames"].as_u64().expect("frames"),
        "{name}"
    );
    for probe in answer["probes"].as_array().expect("probes") {
        let at = probe[0].as_u64().expect("frame") as usize;
        let want = probe[1].as_f64().expect("value");
        for (side, &value) in frames[at].iter().enumerate() {
            let got = f64::from(value);
            assert!(
                (got - want).abs() < 1e-6,
                "{name} side {side} frame {at}: {got} instead of {want}"
            );
        }
    }
    assert!(
        frames.iter().all(|frame| frame[0] == frame[1]),
        "{name}: both sides alike"
    );
    let sounding = frames.iter().filter(|frame| frame[0] != 0.0).count() as u64;
    assert_eq!(
        sounding,
        answer["nonzero"].as_u64().expect("nonzero"),
        "{name}"
    );
    assert_eq!(
        frames_hash(frames),
        answer["sha256"].as_str().expect("sha256"),
        "{name}: the frames are the formula's, every one"
    );
}

/// How a scan records the crash module, whose entry aborts: by the signal on Linux, by the
/// exception status an aborted process exits with on Windows.
fn aborted(expected: &Value) -> String {
    if cfg!(windows) {
        "crashed: exception 0xC0000409".to_owned()
    } else {
        expected["scan"]["crash_reason"]
            .as_str()
            .expect("reason")
            .to_owned()
    }
}

// The generator writes its projects as Dawsome writes them: each loads and saves back to its own
// bytes, so the references a render keeps (the missing plugins' ids and states) round-trip.
#[test]
fn p27_projects_are_canonical_documents() {
    for name in [
        "p27_vst3_sine",
        "p27_vst3_fx",
        "p27_vst3_fx_unity",
        "p27_vst3_missing",
        "p27_vst3_auto",
        "p27_vst3_notes",
        "p27_vst3_fail",
        "p27_vst3_latency",
    ] {
        let text = fs::read_to_string(fixtures().join(format!("{name}.json"))).expect("reads");
        let project = dawsome_engine::project::Project::load_json(&text).expect("loads");
        assert_eq!(project.to_json().expect("writes"), text, "{name}");
    }
}

// 1. The scan loads each module in a child process: the module whose ModuleEntry aborts kills its
// child only and is recorded failed with the signal; the test module lists its audio classes (six
// since the View class; the controller classes are no plugins) with the generator's ids, names,
// kinds and parameter counts, as a VST3 record. Unchanged modules are not loaded again; one whose binary was touched is.
#[test]
fn p27_scan_lists_the_test_module_and_survives_the_crashing_one() {
    let expected = expected();
    let dir = temp_dir("scan");
    let plugins = copied_plugins(&dir);
    let cache = dir.join("catalogue.json");
    let first = scan(&dir, &plugins, &cache);
    let want = |key: &str| expected["scan"][key].as_str().expect(key).to_owned();
    assert!(first.contains(&want("first")), "{first}");
    let classes = expected["scan"]["plugins"]
        .as_array()
        .expect("plugins")
        .len();
    assert!(
        first.contains(&format!("Dawsome Test VST3.vst3: {classes} plugins")),
        "{first}"
    );
    let catalogue: Value =
        serde_json::from_str(&fs::read_to_string(&cache).expect("written")).expect("JSON");
    let files = catalogue["files"].as_array().expect("files");
    assert_eq!(files.len(), 2);
    let crash = &files[0];
    assert!(
        crash["file"]
            .as_str()
            .expect("path")
            .ends_with(&want("crash")),
        "{crash}"
    );
    assert_eq!(crash["format"], "vst3");
    assert_eq!(crash["status"], "failed");
    assert_eq!(crash["reason"], aborted(&expected));
    let module = &files[1];
    assert!(module["file"]
        .as_str()
        .expect("path")
        .ends_with(&want("module")));
    assert_eq!(module["format"], "vst3");
    assert_eq!(module["status"], "ok");
    assert_eq!(module["plugins"], expected["scan"]["plugins"]);
    let second = scan(&dir, &plugins, &cache);
    assert!(second.contains(&want("second")), "{second}");
    let binary = if cfg!(windows) {
        plugins.join("Dawsome Test VST3.vst3/Contents/x86_64-win/Dawsome Test VST3.vst3")
    } else {
        plugins.join("Dawsome Test VST3.vst3/Contents/x86_64-linux/Dawsome Test VST3.so")
    };
    fs::File::options()
        .write(true)
        .open(&binary)
        .expect("open the binary")
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
        .expect("touch the binary");
    let third = scan(&dir, &plugins, &cache);
    assert!(third.contains(&want("touched")), "{third}");
    assert!(third.contains("Dawsome Test VST3.vst3"), "{third}");
}

// The scan child of either format prints a progress line as it finishes each plugin it
// describes ("dawsome-scan-progress 1", "2", ...), which keeps a module of many plugins from its
// parent's timeout, then its result line, unchanged: the VST3 test module's audio classes as the
// generator gives them (its controller classes get no line), the CLAP test file's two plugins.
#[test]
fn the_scan_child_reports_each_plugin_as_it_goes() {
    let expected = expected();
    let dir = temp_dir("child");
    let child = |file: &Path| {
        let output = dawsome(&dir)
            .args(["scan", "--child"])
            .arg(file)
            .output()
            .expect("run dawsome");
        assert!(output.status.success(), "{}", output.status);
        String::from_utf8(output.stdout).expect("UTF-8")
    };
    let progress = |count: usize| -> Vec<String> {
        (1..=count)
            .map(|n| format!("dawsome-scan-progress {n}"))
            .collect()
    };
    let vst3 = child(&test_plugins_vst3().join("Dawsome Test VST3.vst3"));
    let lines: Vec<&str> = vst3.lines().collect();
    let want = &expected["scan"]["plugins"];
    let classes = want.as_array().expect("plugins").len();
    assert_eq!(lines[..lines.len() - 1], progress(classes), "{vst3}");
    let result: Value = serde_json::from_str(
        lines[classes]
            .strip_prefix("dawsome-scan-result ")
            .expect("the result line last"),
    )
    .expect("JSON");
    assert_eq!(result["plugins"], *want);
    let clap = child(&test_plugins::test_plugins().join("dawsome_test_plugin.clap"));
    let lines: Vec<&str> = clap.lines().collect();
    assert_eq!(lines.len(), 3, "{clap}");
    assert_eq!(lines[..2], progress(2), "{clap}");
    assert!(lines[2].starts_with("dawsome-scan-result {\"plugins\":["));
}

// The scan child prints, byte for byte, what gen_p28_expected.py says it prints for the test
// module (the app's `--scan-child` is held to the same text): a progress line per audio class,
// then the records of the grown oracle (the sine's five parameters, the gain's three).
#[test]
fn p28_the_scan_child_prints_the_generators_text() {
    let text = fs::read_to_string(fixtures().join("p28_expected.json")).expect("p28_expected.json");
    let p28: Value = serde_json::from_str(&text).expect("parses");
    let dir = temp_dir("p28-child");
    let output = dawsome(&dir)
        .args(["scan", "--child"])
        .arg(test_plugins_vst3().join("Dawsome Test VST3.vst3"))
        .output()
        .expect("run dawsome");
    assert!(output.status.success(), "{}", output.status);
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8"),
        p28["scan_child"].as_str().expect("a text")
    );
}

// The View class is the sine with an editor: its project (p27's sine with the View's
// class and preset) is a canonical document and renders the sine's frames, every one.
#[test]
fn p29_the_view_class_sounds_as_the_sine() {
    let text = fs::read_to_string(fixtures().join("p29_expected.json")).expect("p29_expected.json");
    let p29: Value = serde_json::from_str(&text).expect("parses");
    let answer = &p29["render"];
    let fixture = answer["fixture"].as_str().expect("fixture");
    let document = fs::read_to_string(fixtures().join(fixture)).expect("reads");
    let project = dawsome_engine::project::Project::load_json(&document).expect("loads");
    assert_eq!(project.to_json().expect("writes"), document);
    let dir = temp_dir("p29-view");
    let (frames, _) = rendered(&dir, fixture, &[]);
    assert_render(&frames, answer);
    assert_eq!(
        answer["sha256"],
        expected()["renders"]["sine"]["sha256"],
        "the sine's frames"
    );
}

// 2. The sine as a channel, at gain 0.5 from its preset: one A4 from frame 6000 to 30000 at
// 48 kHz, every frame the formula's.
#[test]
fn p27_the_sine_plays_its_formula_as_a_channel() {
    let expected = expected();
    let dir = temp_dir("sine");
    let (frames, _) = rendered(&dir, "p27_vst3_sine.json", &[]);
    let answer = &expected["renders"]["sine"];
    assert_render(&frames, answer);
    let rms = &answer["rms"];
    let (from, to) = (
        rms[0].as_u64().expect("from") as usize,
        rms[1].as_u64().expect("to") as usize,
    );
    let level = (frames[from..to]
        .iter()
        .map(|frame| f64::from(frame[0]).powi(2))
        .sum::<f64>()
        / (to - from) as f64)
        .sqrt();
    assert!(
        (level - rms[2].as_f64().expect("rms")).abs() < 1e-7,
        "{level}"
    );
}

// 3. The gain as an effect in insert 1 halves the kick's impulse; at 1.0 it renders the bytes of
// the same project with no effect. Missing plugins: the synth is silent, the effect passes its
// insert through, both are named, and the render succeeds.
#[test]
fn p27_the_gain_processes_its_insert_and_missing_plugins_are_named() {
    let expected = expected();
    let dir = temp_dir("fx");
    let (halved, _) = rendered(&dir, "p27_vst3_fx.json", &[]);
    assert_render(&halved, &expected["renders"]["fx"]);
    let (unity, _) = rendered(&dir, "p27_vst3_fx_unity.json", &[]);
    assert_render(&unity, &expected["renders"]["fx_unity"]);
    let (none, _) = rendered(&dir, "p7_fx_none.json", &[]);
    assert!(unity == none, "gain 1.0 changes the render");
    let (missing, stderr) = rendered(&dir, "p27_vst3_missing.json", &[]);
    for named in expected["missing"].as_array().expect("names") {
        let named = named.as_str().expect("name");
        assert!(stderr.contains(named), "{named:?} not in {stderr}");
    }
    let impulses: Vec<usize> = expected["missing_impulses"]
        .as_array()
        .expect("impulses")
        .iter()
        .map(|frame| frame.as_u64().expect("frame") as usize)
        .collect();
    for (at, frame) in missing.iter().enumerate() {
        let want = if impulses.contains(&at) { 1.0 } else { 0.0 };
        assert_eq!(*frame, [want, want], "frame {at}");
    }
}

// 6. An automation point changes the gain on its own frame, whatever the block: the engine cuts
// the block there and the plugin takes the value at the block's start.
#[test]
fn p27_an_automation_point_lands_on_its_frame_at_every_block_size() {
    let expected = expected();
    let dir = temp_dir("auto");
    for block in ["1", "64", "512", "4096"] {
        let (frames, _) = rendered(&dir, "p27_vst3_auto.json", &["--song", "--block", block]);
        assert_render(&frames, &expected["renders"]["auto"]);
    }
}

// 7. Notes with pitch and velocity: the A4's note-off goes before the A5's note-on on frame 12000,
// and the A5 plays at half the level, an octave up.
#[test]
fn p27_notes_play_with_their_pitch_and_velocity() {
    let expected = expected();
    let dir = temp_dir("notes");
    let (frames, _) = rendered(&dir, "p27_vst3_notes.json", &[]);
    assert_render(&frames, &expected["renders"]["notes"]);
}

// 8. The Fail class sounds for its four calls of 512 frames, then is silent, and is named once
// however many calls it fails.
#[test]
fn p27_a_plugin_failing_mid_render_is_named_once() {
    let expected = expected();
    let dir = temp_dir("fail");
    let answer = &expected["renders"]["fail"];
    let (frames, stderr) = rendered(&dir, "p27_vst3_fail.json", &[]);
    assert_render(&frames, answer);
    let named = format!(
        "dawsome: warning: {}\n",
        answer["notice"].as_str().expect("notice")
    );
    assert_eq!(stderr.matches(&named).count(), 1, "{stderr}");
}

// The latency a plugin reports is said, uncompensated: the Latency class plays the impulse 64
// frames late and the render names its delay.
#[test]
fn p27_a_plugins_latency_is_reported() {
    let expected = expected();
    let dir = temp_dir("latency");
    let answer = &expected["renders"]["latency"];
    let (frames, stderr) = rendered(&dir, "p27_vst3_latency.json", &[]);
    assert_render(&frames, answer);
    let latency = answer["latency"].as_u64().expect("latency");
    let said = format!(
        "Dawsome Test Latency (vst3:{}) delays its sound by {latency} frames",
        expected["ids"]["latency"].as_str().expect("id")
    );
    assert!(stderr.contains(&said), "{said:?} not in {stderr}");
}

// 9. The renders the engine controls are the same bits for every block size (the test plugins
// are split-invariant, so these are; a third-party plugin need not be): the sine, the effect,
// the automation and the notes at blocks of 1, 7, 64, 512 and 4096 frames.
#[test]
fn p27_renders_do_not_depend_on_the_block_size() {
    let expected = expected();
    let dir = temp_dir("blocks");
    for (name, extra) in [
        ("sine", &[][..]),
        ("fx", &[][..]),
        ("auto", &["--song"][..]),
        ("notes", &[][..]),
    ] {
        let answer = &expected["renders"][name];
        let fixture = answer["fixture"].as_str().expect("fixture");
        for block in ["1", "7", "64", "512", "4096"] {
            let mut args = extra.to_vec();
            args.extend(["--block", block]);
            let (frames, stderr) = rendered(&dir, fixture, &args);
            assert_eq!(
                frames_hash(&frames),
                answer["sha256"].as_str().expect("sha256"),
                "{name} at --block {block}"
            );
            if block != "512" {
                assert!(
                    stderr.contains("plugin output may depend on the block size"),
                    "{name} --block {block}: {stderr}"
                );
            }
        }
    }
}

// A module replaced after its scan (here by the module that aborts as it loads) is never loaded
// into the render's own process: its record no longer matches its binary, so a child scans it
// again and finds it broken, and the render plays without the plugin and says so.
#[test]
fn p27_a_module_changed_after_its_scan_is_not_loaded_in_process() {
    let expected = expected();
    let dir = temp_dir("stale");
    let plugins = copied_plugins(&dir);
    let cache = dir.join("catalogue.json");
    scan(&dir, &plugins, &cache);
    let (good, crash) = if cfg!(windows) {
        (
            "Dawsome Test VST3.vst3/Contents/x86_64-win/Dawsome Test VST3.vst3",
            "Dawsome Test VST3 Crash.vst3/Contents/x86_64-win/Dawsome Test VST3 Crash.vst3",
        )
    } else {
        (
            "Dawsome Test VST3.vst3/Contents/x86_64-linux/Dawsome Test VST3.so",
            "Dawsome Test VST3 Crash.vst3/Contents/x86_64-linux/Dawsome Test VST3 Crash.so",
        )
    };
    fs::copy(plugins.join(crash), plugins.join(good)).expect("replace it");
    let out = dir.join("out.wav");
    let output = render_in(&dir, &plugins, "p27_vst3_sine.json", &out, &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{}: {stderr}", output.status);
    let named = format!(
        "Missing plugin: Dawsome Test Sine (vst3:{})",
        expected["ids"]["sine"].as_str().expect("id")
    );
    assert!(stderr.contains(&named), "{stderr}");
    assert!(read_stereo(&out).iter().all(|frame| *frame == [0.0, 0.0]));
}
