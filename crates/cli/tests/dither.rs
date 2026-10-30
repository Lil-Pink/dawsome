//! The export's dither through the command line: a 16-bit WAV is written with TPDF dither from
//! one LCG run per file, giving the values tests/fixtures/gen_p25_expected.py computes from the
//! rule it states (p25_expected.json); `--no-dither` writes the plain rounding of before, and
//! 24-bit files are never dithered. Every run gets per-user folders of its own under the test's
//! temp dir.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use sha2::{Digest, Sha256};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> PathBuf {
    repo_root().join("tests/fixtures")
}

/// Fresh empty directory under cargo's per-target test directory.
fn temp_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("dither")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p25_expected.json"))
        .expect("p25_expected.json (run gen_p25_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("parses")
}

/// `dawsome <args>` with every per-user folder under `dir`.
fn dawsome(dir: &Path, args: &[&OsStr]) -> Output {
    let home = dir.join("home");
    Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .env("DAWSOME_DATA_HOME", home.join("data"))
        .env("DAWSOME_CACHE_HOME", home.join("cache"))
        .env("DAWSOME_CONFIG_HOME", home.join("config"))
        .env("DAWSOME_STATE_HOME", home.join("state"))
        .env("XDG_DATA_HOME", home.join("xdg-data"))
        .env("XDG_CACHE_HOME", home.join("xdg-cache"))
        .env("XDG_CONFIG_HOME", home.join("xdg-config"))
        .env("XDG_STATE_HOME", home.join("xdg-state"))
        .args(args)
        .output()
        .expect("run dawsome")
}

/// `dawsome render <fixture> <dir>/<name>.wav <extra>`: the integer samples it wrote, left then
/// right, and their sample size.
fn render_ints(fixture: &str, dir: &Path, name: &str, extra: &[&str]) -> (Vec<i32>, u16) {
    let out = dir.join(format!("{name}.wav"));
    let project = fixtures().join(fixture);
    let mut args = vec!["render".as_ref(), project.as_os_str(), out.as_os_str()];
    args.extend(extra.iter().map(OsStr::new));
    let output = dawsome(dir, &args);
    assert!(
        output.status.success(),
        "dawsome render {fixture} {extra:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut reader = hound::WavReader::open(&out).expect("open rendered wav");
    let spec = reader.spec();
    assert_eq!(spec.channels, 2);
    assert_eq!(spec.sample_format, hound::SampleFormat::Int);
    let values = reader
        .samples::<i32>()
        .collect::<Result<_, _>>()
        .expect("decodes");
    (values, spec.bits_per_sample)
}

/// The generator's hash of integer samples: each a 32-bit little-endian integer, in file order.
fn ints_hash(values: &[i32]) -> String {
    let mut hasher = Sha256::new();
    for value in values {
        hasher.update(value.to_le_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Asserts `values` (of `bits` bits) is answer `name` of the generator: the sample size, the
/// frame count, the first 32 values, how many are not 0, and the hash of every value.
fn assert_ints(name: &str, (values, bits): &(Vec<i32>, u16), want: &Value) {
    assert_eq!(
        u64::from(*bits),
        want["bits"].as_u64().expect("bits"),
        "{name}: bits"
    );
    assert_eq!(
        values.len() as u64,
        2 * want["frames"].as_u64().expect("frames"),
        "{name}: values"
    );
    let first: Vec<i64> = want["first"]
        .as_array()
        .expect("first")
        .iter()
        .map(|v| v.as_i64().expect("an integer"))
        .collect();
    let got: Vec<i64> = values[..first.len()]
        .iter()
        .map(|&v| i64::from(v))
        .collect();
    assert_eq!(got, first, "{name}: the first values");
    let nonzero = values.iter().filter(|&&v| v != 0).count() as u64;
    assert_eq!(
        nonzero,
        want["nonzero"].as_u64().expect("nonzero"),
        "{name}"
    );
    assert_eq!(
        ints_hash(values),
        want["sha256"].as_str().expect("sha256"),
        "{name}: the hash of every value"
    );
}

/// Known answer (gen_p25_expected.py): a -60 dBFS sine after eight saturated frames, written at
/// 16 bits, is each value plus the TPDF dither's next two LCG draws, rounded and clamped; the
/// same at --block 1 and 4096 (the generator runs per value written, not per block).
#[test]
fn p25_sixteen_bits_are_dithered_by_the_tpdf_rule() {
    let want = &expected()["dither"]["quiet16"];
    let dir = temp_dir("quiet16");
    let fixture = want["fixture"].as_str().expect("fixture");
    let written = render_ints(fixture, &dir, "quiet", &["--bits", "16"]);
    assert_ints("quiet16", &written, want);
    for block in ["1", "4096"] {
        let split = render_ints(fixture, &dir, block, &["--bits", "16", "--block", block]);
        assert!(split.0 == written.0, "quiet16 at --block {block}");
    }
}

/// Known answer: `--no-dither` writes 16 bits by the plain rounding the build before this change
/// wrote, round(v * 2^15) halves away from zero, clamped.
#[test]
fn p25_no_dither_rounds_as_before() {
    let want = &expected()["dither"]["quiet16_plain"];
    let dir = temp_dir("quiet16_plain");
    let fixture = want["fixture"].as_str().expect("fixture");
    let written = render_ints(fixture, &dir, "plain", &["--bits", "16", "--no-dither"]);
    assert_ints("quiet16_plain", &written, want);
}

/// Known answer: 24 bits are never dithered, with `--no-dither` or without.
#[test]
fn p25_twenty_four_bits_are_never_dithered() {
    let want = &expected()["dither"]["quiet24"];
    let dir = temp_dir("quiet24");
    let fixture = want["fixture"].as_str().expect("fixture");
    let written = render_ints(fixture, &dir, "deep", &["--bits", "24"]);
    assert_ints("quiet24", &written, want);
}

/// Known answer: `--no-dither` changes nothing at 24 bits.
#[test]
fn p25_no_dither_leaves_twenty_four_bits_alone() {
    let want = &expected()["dither"]["quiet24"];
    let dir = temp_dir("quiet24_plain");
    let fixture = want["fixture"].as_str().expect("fixture");
    let written = render_ints(fixture, &dir, "deep", &["--bits", "24", "--no-dither"]);
    assert_ints("quiet24", &written, want);
}

/// Known answer (the design's decision: silence is dithered, as most DAWs do): a silent render at
/// 16 bits is the dither alone, -1, 0 or 1 at every value (about a quarter not 0); with
/// `--no-dither` it is digital silence.
#[test]
fn p25_silence_dithers_to_one_lsb_of_noise() {
    let want = &expected()["dither"];
    let dir = temp_dir("silence16");
    let fixture = want["silence16"]["fixture"].as_str().expect("fixture");
    let hiss = render_ints(fixture, &dir, "hiss", &["--bits", "16"]);
    assert_ints("silence16", &hiss, &want["silence16"]);
    assert!(hiss.0.iter().all(|v| (-1..=1).contains(v)));
    for (level, count) in want["silence16"]["counts"].as_object().expect("counts") {
        let level: i32 = level.parse().expect("a level");
        let found = hiss.0.iter().filter(|&&v| v == level).count() as u64;
        assert_eq!(found, count.as_u64().expect("a count"), "values at {level}");
    }
    let quiet = render_ints(fixture, &dir, "quiet", &["--bits", "16", "--no-dither"]);
    assert_ints("silence16_plain", &quiet, &want["silence16_plain"]);
}
