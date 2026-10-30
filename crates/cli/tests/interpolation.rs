//! The sampler's interpolation through the command line: a voice reading between frames reads
//! four of them by cubic Hermite interpolation (the sampler, a channel's sound settings and an
//! instrument set's zones, their loops included), giving the frames
//! tests/fixtures/gen_p25_expected.py computes from the rule it states (p25_expected.json), to
//! the bit and whatever the block size; a voice at speed 1 copies its sound's frames as before.
//! Every run gets per-user folders of its own under the test's temp dir.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
        .join("interpolation")
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

/// `dawsome render <fixture> <dir>/<name>.wav <extra>`, every per-user folder under `dir`;
/// the file it wrote.
fn render_file(fixture: &str, dir: &Path, name: &str, extra: &[&str]) -> PathBuf {
    let out = dir.join(format!("{name}.wav"));
    let home = dir.join("home");
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .env("DAWSOME_DATA_HOME", home.join("data"))
        .env("DAWSOME_CACHE_HOME", home.join("cache"))
        .env("DAWSOME_CONFIG_HOME", home.join("config"))
        .env("DAWSOME_STATE_HOME", home.join("state"))
        .env("XDG_DATA_HOME", home.join("xdg-data"))
        .env("XDG_CACHE_HOME", home.join("xdg-cache"))
        .env("XDG_CONFIG_HOME", home.join("xdg-config"))
        .env("XDG_STATE_HOME", home.join("xdg-state"))
        .arg("render")
        .arg(fixtures().join(fixture))
        .arg(&out)
        .args(extra)
        .output()
        .expect("run dawsome");
    assert!(
        output.status.success(),
        "dawsome render {fixture} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    out
}

/// The frames of a 32-bit float stereo WAV.
fn frames_of(path: &Path) -> Vec<[f32; 2]> {
    let mut reader = hound::WavReader::open(path).expect("open rendered wav");
    assert_eq!(reader.spec().channels, 2);
    let values: Vec<f32> = reader
        .samples::<f32>()
        .map(|v| v.expect("decodes"))
        .collect();
    values
        .chunks_exact(2)
        .map(|pair| [pair[0], pair[1]])
        .collect()
}

/// The 32-bit float render of `fixture`, frame by frame.
fn render(fixture: &str, dir: &Path, name: &str, extra: &[&str]) -> Vec<[f32; 2]> {
    let mut args = vec!["--bits", "32f"];
    args.extend_from_slice(extra);
    frames_of(&render_file(fixture, dir, name, &args))
}

/// The SHA-256 of `frames` as 32-bit floats, little-endian, left then right (the generator's).
fn frames_hash(frames: &[[f32; 2]]) -> String {
    let mut hasher = Sha256::new();
    for frame in frames {
        hasher.update(frame[0].to_le_bytes());
        hasher.update(frame[1].to_le_bytes());
    }
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The first 16 hex digits of the SHA-256 of a file.
fn file_hash16(path: &Path) -> String {
    hex(&Sha256::digest(fs::read(path).expect("read rendered wav"))[..8])
}

fn number(value: &Value) -> f32 {
    value.as_f64().expect("a number") as f32
}

/// Asserts `frames` is render `name` of the generator: length, probes and sounding frames to the
/// bit, and the hash of every frame.
fn assert_render(name: &str, frames: &[[f32; 2]], want: &Value) {
    assert_eq!(
        frames.len() as u64,
        want["frames"].as_u64().expect("frames"),
        "{name}: frames"
    );
    for probe in want["probes"].as_array().expect("probes") {
        let at = probe[0].as_u64().expect("a frame") as usize;
        let wanted = [number(&probe[1]), number(&probe[2])];
        assert_eq!(
            frames[at].map(f32::to_bits),
            wanted.map(f32::to_bits),
            "{name}, frame {at}: {:?} != {wanted:?}",
            frames[at]
        );
    }
    let sounding = frames.iter().filter(|f| **f != [0.0; 2]).count() as u64;
    assert_eq!(
        sounding,
        want["sounding"].as_u64().expect("sounding"),
        "{name}"
    );
    assert_eq!(
        frames_hash(frames),
        want["sha256"].as_str().expect("sha256"),
        "{name}: the hash of every frame"
    );
}

/// Known answers (gen_p25_expected.py): a sine set read a semitone up and seven down, a loop's
/// seams (a continuous loop, and a sustain loop with its offset inside the loop, released after
/// wrapping), a 4-frame stereo sound at half speed and a 1-frame one (the points before the
/// first frame and past the last count 0), a reversed sampler from a start frame: each frame
/// by the 4-point Hermite rule, to the bit.
#[test]
fn p25_pitched_reads_are_the_hermite_rules_frames() {
    let want = expected();
    let dir = temp_dir("renders");
    for (name, answer) in want["renders"].as_object().expect("renders") {
        let fixture = answer["fixture"].as_str().expect("fixture");
        let frames = render(fixture, &dir, name, &[]);
        assert_render(name, &frames, answer);
    }
}

/// Known answer: at speed 1 a voice copies its sound's frames, so the sine set at its root is
/// the file's frames then silence; and the renders of fixtures that play at speed 1 keep the
/// bytes the build before this change wrote (7caab68, `dawsome render <fixture> out.wav`): a
/// regression pin, the known answers for them being their generators' own.
#[test]
fn p25_speed_1_copies_the_sound() {
    let want = &expected()["root"];
    let dir = temp_dir("root");
    let frames = render(
        want["fixture"].as_str().expect("fixture"),
        &dir,
        "root",
        &[],
    );
    assert_render("root", &frames, want);
    let mut file = hound::WavReader::open(fixtures().join(want["file"].as_str().expect("file")))
        .expect("the sound");
    let sound: Vec<f32> = file
        .samples::<f32>()
        .collect::<Result<_, _>>()
        .expect("reads");
    assert_eq!(sound.len() as u64, want["file_frames"].as_u64().expect("n"));
    for (at, frame) in frames.iter().enumerate() {
        let value = sound.get(at).copied().unwrap_or(0.0);
        assert_eq!(frame.map(f32::to_bits), [value.to_bits(); 2], "frame {at}");
    }
    let pinned = [
        ("p11_rr.json", "731e85bdac5e99bc"),
        ("p24_under.json", "2a247439aef2dd2b"),
        ("p13_reverse.json", "e8e4e866565a5bb1"),
    ];
    for (fixture, hash) in pinned {
        let out = render_file(fixture, &dir, fixture, &[]);
        assert_eq!(
            file_hash16(&out),
            hash,
            "{fixture}: its speed-1 render moved"
        );
    }
}

/// The read point is a function of the frame alone, so block sizes 1, 7, 64 and 4096 give the
/// 512 render's bytes, across a loop's wraps and a sound's edges too.
#[test]
fn p25_pitched_reads_render_the_same_bytes_at_any_block_size() {
    let want = expected();
    let dir = temp_dir("blocks");
    for name in want["blocks"]["cases"].as_array().expect("cases") {
        let name = name.as_str().expect("a case");
        let fixture = want["renders"][name]["fixture"].as_str().expect("fixture");
        let reference = frames_hash(&render(fixture, &dir, name, &[]));
        for size in want["blocks"]["sizes"].as_array().expect("sizes") {
            let size = size.as_u64().expect("a size").to_string();
            let frames = render(
                fixture,
                &dir,
                &format!("{name}-{size}"),
                &["--block", &size],
            );
            assert_eq!(frames_hash(&frames), reference, "{name} at --block {size}");
        }
    }
}
