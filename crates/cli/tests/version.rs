//! `dawsome --version` prints the program's name and the workspace's version (Cargo.toml: 0.1.0)
//! and exits.

use std::process::Command;

#[test]
fn version_names_the_program_and_its_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .arg("--version")
        .output()
        .expect("binary starts");
    assert!(output.status.success(), "--version failed");
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf-8"),
        "dawsome 0.1.0\n"
    );
    assert!(output.stderr.is_empty(), "--version wrote to stderr");
}
