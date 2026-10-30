//! The VST3 host with real plugins, fetched for testing and never shipped: Surge XT 1.3.4's VST3
//! bundles (`scripts/fetch-surge.sh --vst3`, into `<target dir>/surge/linux-x86_64-vst3`), the
//! first JUCE plugins through the host. Each plugin's state round-trips: an instance made with no
//! state is captured, a second instance is made from that capture, and its capture is the same
//! text. (The scan and a render through the command line are `crates/cli/tests/
//! vst3_real_plugins.rs`.) A test whose plugin is not fetched prints a line saying so and passes.
//!
//! The plugins run in this process, and they read and make user folders (Surge:
//! `~/Documents/Surge XT`, or `~/.Surge XT`), so each test runs again in a child process of this
//! test binary whose home and XDG folders are temp folders ([`in_temp_home`]): the environment is
//! set before the child starts, not changed under threads that may read it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use dawsome_engine::processor::ProcessorHost;
use dawsome_engine::project::{ChannelId, EffectId, ProcessorOwner, ProcessorRef};
use dawsome_host_common::catalogue::{self, Catalogue, FileRecord, FileStatus, PluginFormat};
use dawsome_host_vst3::{scan, Vst3Host};

#[path = "../../../tests/support/test_plugins.rs"]
mod test_plugins;

/// Set in the child [`in_temp_home`] starts.
const CHILD: &str = "DAWSOME_REAL_PLUGIN_CHILD";

/// Whether this is the child that does `test`'s work. In the test binary's own run it is not:
/// `test` runs again in a child of this binary, alone, with HOME and the XDG folders under a temp
/// folder, and must pass there.
fn in_temp_home(test: &str) -> bool {
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("vst3-real")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let output = Command::new(std::env::current_exe().expect("the test binary"))
        .args([test, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .env_remove("CLAP_PATH")
        .env_remove("VST3_PATH")
        .env("HOME", dir.join("home"))
        .env("XDG_CONFIG_HOME", dir.join("home/config"))
        .env("XDG_DATA_HOME", dir.join("home/data"))
        .env("XDG_CACHE_HOME", dir.join("home/cache"))
        .env("XDG_STATE_HOME", dir.join("home/state"))
        .output()
        .expect("run the test again");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{test} failed in its child ({}):\n{stdout}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("1 passed"),
        "{test} did not run in its child:\n{stdout}"
    );
    print!("{stdout}");
    false
}

/// A catalogue of the module `module`, described in this process (the scan child's work).
fn catalogue_of(module: PathBuf) -> Catalogue {
    let plugins = scan::describe_module(&module, || {}).expect("the module loads");
    let (size, mtime) =
        catalogue::module_stamp(PluginFormat::Vst3, &module).expect("the module is there");
    Catalogue {
        files: vec![FileRecord {
            file: module,
            format: PluginFormat::Vst3,
            size,
            mtime,
            status: FileStatus::Ok,
            reason: None,
            plugins,
        }],
        ..Catalogue::default()
    }
}

/// `module`'s one plugin, made with no state for `fresh`, captured; made again from that capture
/// for `again`, captured: the two captures.
fn captured_twice(module: &Path, fresh: ProcessorOwner, again: ProcessorOwner) -> (String, String) {
    let catalogue = catalogue_of(module.to_owned());
    let plugin = catalogue.files[0].plugins[0].clone();
    let host = Vst3Host::new(catalogue);
    let mut reference = ProcessorRef::plugin(format!("vst3:{}", plugin.id), &plugin.name);
    let _first = host.create(fresh, &reference, 48_000).expect("creates");
    let first = host.capture_state(fresh).expect("captures").state;
    reference.state = Some(first.clone());
    let _second = host
        .create(again, &reference, 48_000)
        .expect("creates from its own state");
    let second = host.capture_state(again).expect("captures").state;
    let bytes = BASE64.decode(&first).expect("base64");
    assert!(bytes.starts_with(b"VST3"), "{}: a .vstpreset", plugin.name);
    println!("{}: a state of {} bytes", plugin.name, bytes.len());
    (first, second)
}

// Surge XT and Surge XT Effects: capture, create from the capture, capture again: the same text.
#[test]
fn surge_xt_state_round_trips() {
    let binary = "Surge XT.vst3/Contents/x86_64-linux/Surge XT.so";
    let Some(surge) = test_plugins::fetched("surge/linux-x86_64-vst3", binary) else {
        println!("skipped: Surge XT's VST3 is not fetched (scripts/fetch-surge.sh --vst3)");
        return;
    };
    if !in_temp_home("surge_xt_state_round_trips") {
        return;
    }
    let (first, second) = captured_twice(
        &surge.join("Surge XT.vst3"),
        ProcessorOwner::Channel(ChannelId(1)),
        ProcessorOwner::Channel(ChannelId(2)),
    );
    assert_eq!(first, second, "Surge XT");
    let (first, second) = captured_twice(
        &surge.join("Surge XT Effects.vst3"),
        ProcessorOwner::Effect(EffectId(1)),
        ProcessorOwner::Effect(EffectId(2)),
    );
    assert_eq!(first, second, "Surge XT Effects");
}
