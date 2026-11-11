//! Jam through the real `dawsome` binary: every request tests/fixtures/gen_p36_expected.py
//! writes, run as `dawsome jam ... --print`, prints exactly the pattern the generator works out
//! and saves the project with it (the key, version 11, the tempo with --auto-tempo, the "Jam 1"
//! pattern note for note); the pattern renders sound on every channel given a role; and what the
//! command refuses. Expected values come from the generator, never from the engine.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{json, Value};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(name: &str) -> PathBuf {
    repo_root().join("tests/fixtures").join(name)
}

/// A fresh empty folder under cargo's per-target test folder (on the build drive).
fn temp_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("jam")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn expected() -> Value {
    let text = fs::read_to_string(fixture("p36_expected.json"))
        .expect("p36_expected.json (run gen_p36_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("p36_expected.json parses")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("reads")).expect("parses")
}

/// Runs `dawsome` with `args`, its cache in `dir`.
fn dawsome(dir: &Path, args: &[OsString]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .env("DAWSOME_CACHE_HOME", dir.join("cache"))
        .args(args)
        .output()
        .expect("run dawsome binary")
}

fn dawsome_ok(dir: &Path, args: &[OsString]) -> String {
    let output = dawsome(dir, args);
    assert!(
        output.status.success(),
        "dawsome {args:?} failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8")
}

/// The `dawsome jam` command line a request file describes, writing to `out`.
fn jam_args(request: &Value, out: &Path) -> Vec<OsString> {
    let text = |key: &str| request[key].as_str().expect(key).to_owned();
    let mut args: Vec<OsString> = vec![
        "jam".into(),
        fixture(&text("project")).into(),
        out.into(),
        "--genre".into(),
        text("genre").into(),
        "--bars".into(),
        request["bars"].to_string().into(),
        "--seed".into(),
        request["seed"].to_string().into(),
        "--print".into(),
    ];
    if request.get("key").is_some() {
        args.extend([
            "--key".into(),
            text("key").into(),
            "--scale".into(),
            text("scale").into(),
        ]);
    }
    for role in request["roles"].as_array().expect("roles") {
        args.extend(["--role".into(), role.as_str().expect("a role").into()]);
    }
    for range in request
        .get("ranges")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        args.extend(["--range".into(), range.as_str().expect("a range").into()]);
    }
    for channel in request
        .get("sustained")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        args.extend(["--sustained".into(), channel.to_string().into()]);
    }
    if request["auto_tempo"] == json!(true) {
        args.push("--auto-tempo".into());
    }
    args
}

/// The saved project's parts the generator states: version, key, tempo and the Jam pattern.
fn saved_parts(file: &Value) -> Value {
    let pattern = file["patterns"]
        .as_array()
        .expect("patterns")
        .iter()
        .find(|p| p["name"] == "Jam 1")
        .cloned()
        .expect("the Jam pattern");
    json!({"format_version": file["format_version"], "key": file["key"],
           "tempo_mbpm": file["tempo_mbpm"], "pattern": pattern})
}

// Known answers: each of the 20 requests prints the generator's pattern (genre, key, bars, seed,
// progression, tempo, swing and every channel's notes) and saves its project.
#[test]
fn every_request_prints_and_saves_the_oracles_jam() {
    let dir = temp_dir("cases");
    let want = expected();
    let cases = want["cases"].as_object().expect("cases");
    assert_eq!(cases.len(), 20);
    for (name, case) in cases {
        let request = read_json(&fixture(case["request"].as_str().expect("a file")));
        let out = dir.join(format!("{name}.dawsome"));
        let printed: Value =
            serde_json::from_str(&dawsome_ok(&dir, &jam_args(&request, &out))).expect("JSON");
        assert_eq!(printed, case["print"], "{name}: --print");
        assert_eq!(
            saved_parts(&read_json(&out)),
            case["saved"],
            "{name}: saved"
        );
    }
}

/// Whether a rendered WAV holds any sample other than 0.
fn sounds(path: &Path) -> bool {
    let mut reader = hound::WavReader::open(path).expect("open rendered wav");
    reader
        .samples::<f32>()
        .any(|sample| sample.expect("a sample") != 0.0)
}

// The smoke pin: the Jam pattern rendered one channel at a time (`--pattern "Jam 1" --channel
// NAME --once`) sounds on every channel given a role, the band's four and the kit's three.
#[test]
fn every_channel_with_a_role_sounds() {
    let dir = temp_dir("renders");
    let want = expected();
    for (case, names) in [
        ("lofi_1", &["Drums", "Bass", "Keys", "Lead"][..]),
        ("lofi_6_kit", &["Kick", "Snare", "Hat closed"][..]),
    ] {
        let request = read_json(&fixture(
            want["cases"][case]["request"].as_str().expect("a file"),
        ));
        let out = dir.join(format!("{case}.dawsome"));
        dawsome_ok(&dir, &jam_args(&request, &out));
        for name in names {
            let wav = dir.join(format!("{case} {name}.wav"));
            let args: Vec<OsString> = vec![
                "render".into(),
                out.clone().into(),
                wav.clone().into(),
                "--pattern".into(),
                "Jam 1".into(),
                "--channel".into(),
                (*name).into(),
                "--once".into(),
            ];
            dawsome_ok(&dir, &args);
            assert!(sounds(&wav), "{case}: {name} renders silence");
        }
    }
}

/// Runs `dawsome jam` on `project` with `flags`, requiring it to fail with `code`; returns its
/// stderr.
fn refused(dir: &Path, project: &str, flags: &[&str], code: i32) -> String {
    let mut args: Vec<OsString> = vec![
        "jam".into(),
        fixture(project).into(),
        dir.join("out.dawsome").into(),
    ];
    args.extend(flags.iter().map(OsString::from));
    let output = dawsome(dir, &args);
    assert_eq!(output.status.code(), Some(code), "{flags:?}");
    assert!(
        !dir.join("out.dawsome").exists(),
        "{flags:?}: nothing written"
    );
    String::from_utf8(output.stderr).expect("utf-8")
}

// What the command refuses, before it writes anything: a project not in 4/4, no key given or
// held, a channel the project lacks, a word it does not know, half a key, bars other than 4 or
// 8, one channel given two roles, no role at all, a sustained channel with no role or no number,
// a range upside down, past key 127, without its
// high key, for a channel with no role, or given twice.
#[test]
fn the_command_refuses_what_it_cannot_write() {
    let dir = temp_dir("refusals");
    let band = "p36_band.json";
    let roles = ["--role", "1=drums"];
    let with = |extra: &[&'static str]| [&["--genre", "lofi"][..], &roles[..], extra].concat();
    let error = refused(
        &dir,
        "p36_waltz.json",
        &with(&["--key", "C", "--scale", "minor"]),
        1,
    );
    assert!(
        error.contains("a Jam writes 4/4 patterns and this project is in 3/4"),
        "{error}"
    );
    let error = refused(&dir, band, &with(&[]), 1);
    assert!(
        error.contains("has no key: give --key and --scale"),
        "{error}"
    );
    let error = refused(
        &dir,
        band,
        &[
            "--genre", "lofi", "--key", "C", "--scale", "minor", "--role", "9=bass",
        ],
        1,
    );
    assert!(error.contains("has no channel 9"), "{error}");
    for (flags, says) in [
        (
            with(&["--key", "H", "--scale", "minor"]),
            "'H' is not a key",
        ),
        (
            with(&["--key", "C", "--scale", "lydian"]),
            "'lydian' is not a scale",
        ),
        (with(&["--key", "C"]), "--key and --scale go together"),
        (
            with(&["--key", "C", "--scale", "minor", "--bars", "5"]),
            "--bars is 4 or 8",
        ),
        (
            with(&["--key", "C", "--scale", "minor", "--role", "1=bass"]),
            "channel 1 was given two roles",
        ),
        (
            with(&["--key", "C", "--scale", "minor", "--role", "2=tuba"]),
            "--role 2=tuba",
        ),
        (
            with(&[
                "--key",
                "C",
                "--scale",
                "minor",
                "--role",
                "2=drums:kick=200",
            ]),
            "--role 2=drums:kick=200",
        ),
        (
            with(&["--key", "C", "--scale", "minor", "--range", "1=50-40"]),
            "--range 1=50-40",
        ),
        (
            with(&["--key", "C", "--scale", "minor", "--range", "1=20-128"]),
            "--range 1=20-128",
        ),
        (
            with(&["--key", "C", "--scale", "minor", "--range", "1=20"]),
            "--range 1=20",
        ),
        (
            with(&["--key", "C", "--scale", "minor", "--range", "2=26-45"]),
            "--range names channel 2, which has no --role",
        ),
        (
            with(&[
                "--key", "C", "--scale", "minor", "--range", "1=0-60", "--range", "1=0-70",
            ]),
            "channel 1 was given two ranges",
        ),
        (
            with(&["--key", "C", "--scale", "minor", "--sustained", "4"]),
            "--sustained names channel 4, which has no --role",
        ),
        (
            with(&["--key", "C", "--scale", "minor", "--sustained", "x"]),
            "--sustained expects a whole number above 0",
        ),
        (
            vec![
                "--genre", "polka", "--key", "C", "--scale", "minor", "--role", "1=drums",
            ],
            "'polka' is not a genre",
        ),
        (
            vec!["--genre", "lofi", "--key", "C", "--scale", "minor"],
            "at least one --role",
        ),
    ] {
        let error = refused(&dir, band, &flags, 2);
        assert!(error.contains(says), "{flags:?}: {error}");
    }
}

// A project at PPQ 4 (one a project may have; a copy of the band made here, since no fixture
// is at a PPQ a Jam refuses) is refused before anything is written.
#[test]
fn the_command_refuses_a_ppq_that_is_no_multiple_of_8() {
    let dir = temp_dir("ppq");
    let band = fs::read_to_string(fixture("p36_band.json")).expect("reads");
    let coarse = band.replacen(r#""ppq":96"#, r#""ppq":4"#, 1);
    assert_ne!(coarse, band);
    let project = dir.join("coarse.json");
    fs::write(&project, coarse).expect("writes");
    let out = dir.join("out.dawsome");
    let args: Vec<OsString> = [
        "jam", "--genre", "trap", "--key", "C", "--scale", "minor", "--role", "1=drums",
    ]
    .iter()
    .map(OsString::from)
    .chain([project.into(), out.clone().into()])
    .collect();
    let output = dawsome(&dir, &args);
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8(output.stderr).expect("utf-8");
    assert!(
        error.contains("a Jam needs a PPQ that is a multiple of 8, and this project's is 4"),
        "{error}"
    );
    assert!(!out.exists(), "nothing written");
}
