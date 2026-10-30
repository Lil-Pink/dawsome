//! `dawsome-app --version` prints the program's name and version, and `--help` (or `-h`) the
//! usage, and each exits without a window.

use std::process::Command;

#[test]
fn version_names_the_program_and_its_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome-app"))
        .arg("--version")
        .output()
        .expect("binary starts");
    assert!(output.status.success(), "--version failed");
    let line = String::from_utf8(output.stdout).expect("utf-8");
    assert!(
        line.starts_with("dawsome-app 0.1.0") && line.ends_with('\n') && line.lines().count() == 1,
        "unexpected --version output: {line:?}"
    );
}

// With no display to open a window on, a run that tried to would fail: success and the usage on
// stdout show the flag is answered before any window, as `dawsome --help` answers it.
#[test]
fn help_prints_the_usage_and_opens_no_window() {
    for flag in ["--help", "-h"] {
        let output = Command::new(env!("CARGO_BIN_EXE_dawsome-app"))
            .arg(flag)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .expect("binary starts");
        assert!(output.status.success(), "{flag} failed: {output:?}");
        let text = String::from_utf8(output.stdout).expect("utf-8");
        assert!(
            text.starts_with("usage: dawsome-app ") && text.contains("  --plugin-path "),
            "{flag}: {text:?}"
        );
        assert!(output.stderr.is_empty(), "{flag}: {:?}", output.stderr);
    }
}
