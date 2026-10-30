//! Where Dawsome's own test plugins are: the folders scripts/build-test-plugins.sh fills
//! (`<target dir>/test-plugins`, `<target dir>/test-plugin-variants` and
//! `<target dir>/test-plugin-gui` for CLAP,
//! `<target dir>/test-plugins-vst3` for VST3), and the real plugins fetched for tests. Included
//! as a module by the tests that load them, unit tests as well as integration tests.

use std::path::PathBuf;

/// The test plugins' folder, holding `dawsome_test_plugin.clap` (sine and gain) and
/// `dawsome_test_crash.clap` (aborts as it loads). Panics with the fix when they are not built.
// Not every test crate that includes this module plays CLAP.
#[allow(dead_code)]
pub fn test_plugins() -> PathBuf {
    built("test-plugins", "dawsome_test_plugin.clap")
}

/// The variants' folder, holding `dawsome_test_variants.clap` (the big-state, failing and timer
/// sines).
// Not every test crate that includes this module plays the variants.
#[allow(dead_code)]
pub fn test_plugin_variants() -> PathBuf {
    built("test-plugin-variants", "dawsome_test_variants.clap")
}

/// The interface's folder, holding `dawsome_test_gui.clap` (the sine with an interface).
// Not every test crate that includes this module opens plugin windows.
#[allow(dead_code)]
pub fn test_plugin_gui() -> PathBuf {
    built("test-plugin-gui", "dawsome_test_gui.clap")
}

/// The VST3 test plugins' folder, holding the bundles `Dawsome Test VST3.vst3` (the sine, gain,
/// fail, tick, latency and view classes) and `Dawsome Test VST3 Crash.vst3` (its `ModuleEntry`
/// aborts).
// Not every test crate that includes this module plays VST3.
#[allow(dead_code)]
pub fn test_plugins_vst3() -> PathBuf {
    let binary = if cfg!(windows) {
        "Dawsome Test VST3.vst3/Contents/x86_64-win/Dawsome Test VST3.vst3"
    } else {
        "Dawsome Test VST3.vst3/Contents/x86_64-linux/Dawsome Test VST3.so"
    };
    built("test-plugins-vst3", binary)
}

/// Real plugins a script fetched for the tests, never shipped: `<target dir>/<folder>` when it
/// holds `file` (Surge XT's VST3 folder, `surge/linux-x86_64-vst3`, from
/// `scripts/fetch-surge.sh --vst3`; the VST3 test plugins' `vst3-test/<name>`, from
/// `scripts/fetch-vst3-test-plugins.sh`). `None` when it is not fetched: the test that wants it
/// says so and passes, as the install check does for the bundled plugin.
// Not every test crate that includes this module plays real plugins.
#[allow(dead_code)]
pub fn fetched(folder: &str, file: &str) -> Option<PathBuf> {
    let dir = target_dir().join(folder);
    dir.join(file).is_file().then_some(dir)
}

/// `<target dir>/<folder>`, which must hold `file`.
fn built(folder: &str, file: &str) -> PathBuf {
    let dir = target_dir().join(folder);
    assert!(
        dir.join(file).is_file(),
        "no {file} in {}: run scripts/build-test-plugins.sh (scripts/check.sh does)",
        dir.display()
    );
    dir
}

/// The target dir, found from the test binary's own place, `<target dir>/<profile>/deps/
/// <binary>`. Why not `CARGO_TARGET_TMPDIR`: Cargo sets it for integration tests only, and unit
/// tests load the plugins too.
fn target_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("the test binary has a path");
    exe.ancestors()
        .nth(3)
        .expect("the test binary is inside the target dir")
        .to_owned()
}
