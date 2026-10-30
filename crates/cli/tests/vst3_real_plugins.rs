//! The VST3 host through the command line with real plugins, fetched for testing and never
//! shipped: Surge XT 1.3.4's VST3 bundles (`scripts/fetch-surge.sh --vst3`, into
//! `<target dir>/surge/linux-x86_64-vst3`), the first JUCE plugins through the host; Dexed 1.0.1,
//! JS80P 4.1.1 and DPF-Plugins 1.7's 3-Band EQ (`scripts/fetch-vst3-test-plugins.sh`, into
//! `<target dir>/vst3-test/<name>`): a second JUCE synth, one on the SDK's own classes, and one
//! DSP in both formats on DPF's own VST3 code; and LSP Plugins' module of 198 classes where the
//! system has it installed. Dawsome's own
//! test plugins and their formulas stay the gate; a real plugin is held only to what is true by
//! construction: the names, kinds and class ids its own source or moduleinfo.json gives, a note
//! that renders finite and not silent, and one DSP's CLAP and VST3 builds agreeing. A test whose plugin is not fetched prints a line saying so and passes.
//! Every run passes --plugin-path and a fresh --cache, and the plugin's code runs with a home and
//! XDG folders of its own (Surge reads and makes `~/Documents/Surge XT`, or `~/.Surge XT`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

#[path = "../../../tests/support/test_plugins.rs"]
mod test_plugins;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Fresh empty directory under cargo's per-target test directory.
fn temp_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("vst3-real")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// `dawsome` with every per-user folder under `dir`, and no plugin folders from the
/// environment.
fn dawsome(dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dawsome"));
    command
        .current_dir(dir)
        .env_remove("CLAP_PATH")
        .env_remove("VST3_PATH")
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

/// `dawsome scan` of `plugins` into `dir/catalogue.json`, which must exit 0: the catalogue.
fn scan(dir: &Path, plugins: &[&Path]) -> Value {
    let cache = dir.join("catalogue.json");
    let mut command = dawsome(dir);
    command.arg("scan").arg("--cache").arg(&cache);
    for folder in plugins {
        command.arg("--plugin-path").arg(folder);
    }
    let output = command.output().expect("run dawsome");
    assert!(
        output.status.success(),
        "dawsome scan failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_str(&fs::read_to_string(&cache).expect("written")).expect("JSON")
}

/// Every plugin of the catalogue's files of `format`: (id, name, kind), sorted by id.
fn listed(catalogue: &Value, format: &str) -> Vec<(String, String, String)> {
    let mut plugins: Vec<(String, String, String)> = catalogue["files"]
        .as_array()
        .expect("files")
        .iter()
        .filter(|file| file["format"].as_str().unwrap_or("clap") == format)
        .inspect(|file| assert_eq!(file["status"], "ok", "{file}"))
        .flat_map(|file| file["plugins"].as_array().expect("plugins").clone())
        .map(|plugin| {
            let text = |key: &str| plugin[key].as_str().expect(key).to_owned();
            (text("id"), text("name"), text("kind"))
        })
        .collect();
    plugins.sort();
    plugins
}

/// `dawsome render` of `project` with `plugins`, which must exit 0 with every plugin found and
/// none failed (a missing effect would pass the audio through, and two renders agree then):
/// the frames, left and right.
fn render(dir: &Path, plugins: &Path, project: &Value, name: &str) -> Vec<[f32; 2]> {
    let file = dir.join(format!("{name}.json"));
    fs::write(&file, project.to_string()).expect("project written");
    let out = dir.join(format!("{name}.wav"));
    let output = dawsome(dir)
        .arg("render")
        .arg(&file)
        .arg(&out)
        .arg("--plugin-path")
        .arg(plugins)
        .arg("--cache")
        .arg(dir.join("catalogue.json"))
        .output()
        .expect("run dawsome");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "dawsome render {name} failed ({}): {stderr}",
        output.status
    );
    assert!(
        !stderr.contains("Missing plugin") && !stderr.contains("Plugin failed"),
        "{name}: {stderr}"
    );
    let mut reader = hound::WavReader::open(&out).expect("open rendered wav");
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

/// The p27 sine fixture (48 kHz, 120 BPM, one bar of pattern 1) with its channel playing
/// `id`, with no state, and one note: `key` at velocity 1.0 for the whole bar.
fn one_note_bar(id: &str, name: &str, key: u8) -> Value {
    let text = fs::read_to_string(repo_root().join("tests/fixtures/p27_vst3_sine.json"))
        .expect("the p27 sine fixture");
    let mut project: Value = serde_json::from_str(&text).expect("JSON");
    let channel = &mut project["channels"][0];
    channel["name"] = name.into();
    channel["processor"] = serde_json::json!({ "id": format!("vst3:{id}"), "name": name });
    project["patterns"][0]["notes"] = serde_json::json!([
        { "channel": 1, "tick": 0, "length": 384, "velocity": 1.0, "key": key }
    ]);
    project
}

/// Holds `frames` to what a played note is by construction: every sample finite, and sound.
fn assert_sounds(frames: &[[f32; 2]], name: &str) {
    assert!(
        frames.iter().flatten().all(|value| value.is_finite()),
        "{name}: a sample is not finite"
    );
    let peak = frames
        .iter()
        .flatten()
        .fold(0.0f32, |peak, value| peak.max(value.abs()));
    assert!(peak > 1e-3, "{name}: silent (peak {peak})");
}

/// A JUCE plugin's VST3 class id, as its source makes it: `INLINE_UID (0xABCDEF01, 0x9182FAEB,
/// JucePlugin_ManufacturerCode, JucePlugin_PluginCode)` (JUCE 7's
/// juce_audio_plugin_client_VST3.cpp, the component's `iid`), each code its four characters as
/// a big-endian word, printed as the class id text.
fn juce_class_id(manufacturer: &str, plugin: &str) -> String {
    let word = |code: &str| {
        assert_eq!(code.len(), 4, "{code}");
        code.bytes()
            .fold(0u32, |word, byte| word << 8 | u32::from(byte))
    };
    format!(
        "ABCDEF019182FAEB{:08X}{:08X}",
        word(manufacturer),
        word(plugin)
    )
}

// Surge XT's VST3 bundles: the scan lists the instrument and the effects with the class ids its
// source gives (manufacturer code VmbA, plugin codes SgXT and SFXT: src/surge-xt/CMakeLists.txt
// and src/surge-fx/CMakeLists.txt of release_xt_1.3.4, VST3 categories "Instrument Synth" and
// "Fx"), and one C4 through the instrument for a bar renders finite and not silent.
#[test]
fn surge_xt_is_listed_and_one_c4_bar_sounds() {
    let binary = "Surge XT.vst3/Contents/x86_64-linux/Surge XT.so";
    let Some(surge) = test_plugins::fetched("surge/linux-x86_64-vst3", binary) else {
        println!("skipped: Surge XT's VST3 is not fetched (scripts/fetch-surge.sh --vst3)");
        return;
    };
    let dir = temp_dir("surge");
    let catalogue = scan(&dir, &[&surge]);
    let instrument = juce_class_id("VmbA", "SgXT");
    assert_eq!(instrument, "ABCDEF019182FAEB566D624153675854");
    let effects = juce_class_id("VmbA", "SFXT");
    let want = |id: &str, name: &str, kind: &str| (id.to_owned(), name.to_owned(), kind.to_owned());
    assert_eq!(
        listed(&catalogue, "vst3"),
        [
            want(&effects, "Surge XT Effects", "effect"),
            want(&instrument, "Surge XT", "instrument"),
        ]
    );
    let frames = render(
        &dir,
        &surge,
        &one_note_bar(&instrument, "Surge XT", 60),
        "surge-c4",
    );
    assert_eq!(frames.len(), 96_000, "one bar at 120 BPM, 48 kHz");
    assert_sounds(&frames, "Surge XT, C4");
}

/// `text` with every comma that closes an object or a list dropped, commas inside strings
/// kept: the SDK's moduleinfotool writes such commas (Dexed's and JS80P's moduleinfo.json have
/// them), which JSON, and so serde_json, refuses.
fn without_closing_commas(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let (mut in_string, mut escaped) = (false, false);
    for (at, &c) in chars.iter().enumerate() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
        } else if c == '"' {
            in_string = true;
        } else if c == ',' {
            let next = chars[at + 1..].iter().find(|c| !c.is_whitespace());
            if matches!(next, Some('}' | ']')) {
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// The audio classes a module's `Contents/Resources/moduleinfo.json` lists (the SDK's
/// moduleinfotool writes it from the module's factory): (id, name, kind), the kind from its sub
/// categories, sorted by id.
fn moduleinfo_classes(module: &Path) -> Vec<(String, String, String)> {
    let text = fs::read_to_string(module.join("Contents/Resources/moduleinfo.json"))
        .expect("the module's moduleinfo.json");
    let info: Value = serde_json::from_str(&without_closing_commas(&text)).expect("JSON");
    let mut classes: Vec<(String, String, String)> = info["Classes"]
        .as_array()
        .expect("classes")
        .iter()
        .filter(|class| class["Category"] == "Audio Module Class")
        .map(|class| {
            let has = |name: &str| {
                class["Sub Categories"]
                    .as_array()
                    .expect("sub categories")
                    .iter()
                    .any(|sub| sub == name)
            };
            let kind = if has("Instrument") {
                "instrument"
            } else if has("Fx") {
                "effect"
            } else {
                panic!("{class}: neither an instrument nor an effect")
            };
            let text = |key: &str| class[key].as_str().expect(key).to_owned();
            (text("CID"), text("Name"), kind.to_owned())
        })
        .collect();
    classes.sort();
    classes
}

// LSP Plugins' VST3 module, the system's own (`/usr/lib/vst3/lsp-plugins.vst3`, skipped when it
// is not installed), scanned through a temp folder holding a link to it, never a copy: every
// audio class of its moduleinfo.json (198) is listed with its name and kind. Its child takes
// close to one CHILD_TIMEOUT in all (7.7 to 8.3 s on the debug build, 2026-10-26), some 40 ms a
// class: its progress lines, one a class, are what keep a module like it inside the limits as it
// grows (CHILD_CAP, 60 s, in all). The wall time is printed.
#[cfg(target_os = "linux")]
#[test]
fn lsp_plugins_lists_every_audio_class_of_its_moduleinfo() {
    let module = Path::new("/usr/lib/vst3/lsp-plugins.vst3");
    if !module
        .join("Contents/x86_64-linux/lsp-plugins.so")
        .is_file()
    {
        println!("skipped: LSP Plugins' VST3 is not installed in /usr/lib/vst3");
        return;
    }
    let want = moduleinfo_classes(module);
    let dir = temp_dir("lsp");
    let plugins = dir.join("plugins");
    fs::create_dir_all(&plugins).expect("folder");
    std::os::unix::fs::symlink(module, plugins.join("lsp-plugins.vst3")).expect("link");
    let started = std::time::Instant::now();
    let catalogue = scan(&dir, &[&plugins]);
    let took = started.elapsed();
    println!(
        "LSP Plugins: {} audio classes scanned in {:.1} s",
        want.len(),
        took.as_secs_f64()
    );
    assert_eq!(listed(&catalogue, "vst3"), want);
}

/// A VST3 test plugin's folder (`scripts/fetch-vst3-test-plugins.sh`), when it is fetched and
/// holds `file`.
fn vst3_test(name: &str, file: &str) -> Option<PathBuf> {
    let folder = test_plugins::fetched(&format!("vst3-test/{name}"), file);
    if folder.is_none() {
        println!("skipped: {name} is not fetched (scripts/fetch-vst3-test-plugins.sh)");
    }
    folder
}

// Dexed 1.0.1 (JUCE) and JS80P 4.1.1 (on the SDK's own classes): the scan lists each synth's
// audio class as its moduleinfo.json names it, with the class id the brief pinned from that
// file, and one C4 bar through each renders finite and not silent.
#[test]
fn dexed_and_js80p_are_listed_as_their_moduleinfo_says_and_sound() {
    let synths = [
        (
            "dexed",
            "Dexed.vst3",
            "x86_64-linux/Dexed.so",
            "ABCDEF019182FAEB4447534244657864",
        ),
        (
            "js80p",
            "js80p.vst3",
            "x86_64-linux/js80p.so",
            "00565354414D4A386A73383070000000",
        ),
    ];
    for (name, bundle, binary, id) in synths {
        let Some(folder) = vst3_test(name, &format!("{bundle}/Contents/{binary}")) else {
            continue;
        };
        let want = moduleinfo_classes(&folder.join(bundle));
        assert_eq!(want.len(), 1, "{name}: {want:?}");
        assert_eq!((want[0].0.as_str(), want[0].2.as_str()), (id, "instrument"));
        let dir = temp_dir(name);
        let catalogue = scan(&dir, &[&folder]);
        assert_eq!(listed(&catalogue, "vst3"), want, "{name}");
        let frames = render(&dir, &folder, &one_note_bar(id, &want[0].1, 60), name);
        assert_sounds(&frames, name);
    }
}

// DPF-Plugins 1.7's 3-Band EQ, one DSP built as a CLAP and as a VST3 (DPF's own VST3 code, not
// the SDK's): the scan lists it once in each format, and the impulse fixture (p27_vst3_fx: one
// sample of 1.0 at 44.1 kHz through an insert) renders through either at its defaults. The two
// renders were expected bit-identical (a guess before the first run, recorded in the spec's
// Decisions either way).
#[test]
fn dpf_3band_eq_renders_alike_as_clap_and_as_vst3() {
    let Some(folder) = vst3_test("dpf", "3BandEQ.vst3/Contents/x86_64-linux/3BandEQ.so") else {
        return;
    };
    let dir = temp_dir("dpf");
    let catalogue = scan(&dir, &[&folder]);
    let id_of = |format: &str| {
        let plugins = listed(&catalogue, format);
        assert_eq!(plugins.len(), 1, "{format}: {plugins:?}");
        let (id, name, kind) = &plugins[0];
        assert_eq!((name.as_str(), kind.as_str()), ("3 Band EQ", "effect"));
        id.clone()
    };
    let text = fs::read_to_string(repo_root().join("tests/fixtures/p27_vst3_fx.json"))
        .expect("the p27 effect fixture");
    fs::copy(
        repo_root().join("tests/fixtures/impulse.wav"),
        dir.join("impulse.wav"),
    )
    .expect("the impulse beside the projects");
    let through = |format: &str| {
        let mut project: Value = serde_json::from_str(&text).expect("JSON");
        project["mixer"]["inserts"][1]["effects"][0]["processor"] = serde_json::json!({
            "id": format!("{format}:{}", id_of(format)),
            "name": "3 Band EQ",
        });
        render(&dir, &folder, &project, &format!("eq-{format}"))
    };
    let clap = through("clap");
    let vst3 = through("vst3");
    assert_sounds(&vst3, "3 Band EQ, VST3");
    let differ = clap.iter().zip(&vst3).filter(|(a, b)| a != b).count();
    println!(
        "3 Band EQ: {} frames, {differ} differ between CLAP and VST3",
        clap.len()
    );
    assert_eq!(clap.len(), vst3.len());
    assert!(clap == vst3, "{differ} frames differ");
}
