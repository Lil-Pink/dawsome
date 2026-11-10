//! Time signatures other than 4/4 through the real `dawsome` binary: the pattern's length and
//! the render's (`render --bars`), the MIDI files (`smf-export`, `export-midi`) and the
//! DAWproject (`export-dawproject`) of projects in 3/4, 6/8, 7/8 and 15/16. Every expected value
//! comes from tests/fixtures/gen_p34_expected.py, never from the engine.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(name: &str) -> PathBuf {
    repo_root().join("tests/fixtures").join(name)
}

/// A fresh empty folder under cargo's per-target test folder (on the build drive).
fn temp_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("time_signature")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn expected() -> Value {
    let text = fs::read_to_string(fixture("p34_expected.json")).expect("p34_expected.json");
    serde_json::from_str(&text).expect("p34_expected.json parses")
}

fn dawsome(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .args(args)
        .output()
        .expect("run dawsome binary")
}

/// Runs `dawsome` with `args`, requiring success; returns its stderr.
fn dawsome_ok(args: &[&std::ffi::OsStr]) -> String {
    let output = dawsome(args);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "dawsome {args:?} failed ({}): {stderr}",
        output.status
    );
    stderr
}

/// A 32-bit float stereo WAV's frames, both sides checked equal.
fn read_mono_of_stereo(path: &Path) -> Vec<f32> {
    let mut reader = hound::WavReader::open(path).expect("open rendered wav");
    let spec = reader.spec();
    assert_eq!(spec.channels, 2, "stereo");
    assert_eq!(spec.sample_format, hound::SampleFormat::Float);
    let samples: Vec<f32> = reader
        .samples::<f32>()
        .collect::<Result<_, _>>()
        .expect("decode samples");
    samples
        .chunks_exact(2)
        .map(|pair| {
            assert_eq!(pair[0].to_bits(), pair[1].to_bits(), "both sides alike");
            pair[0]
        })
        .collect()
}

// Known answer 4: a one-bar pattern with a hit at tick 0 (impulse.wav, unit gain) at 130 BPM:
// `--bars 1` is one bar of the signature, s(288) = 61061 frames in 3/4 and s(336) = 71238 in
// 7/8, the hit alone; `--bars 2` loops the pattern, its second pass from s(288) = 61061 and
// s(336) = 71238 (not s(384) = 81415, a 4/4 bar), in 122123 and 142476 frames.
#[test]
fn the_pattern_and_the_render_are_bars_of_the_signature() {
    let dir = temp_dir("onsets");
    let want = expected();
    for (case, answer) in want["onsets"].as_object().expect("cases") {
        let project = fixture(answer["fixture"].as_str().expect("fixture"));
        for (bars, render) in answer["bars"].as_object().expect("bars") {
            let out = dir.join(format!("{case}_{bars}.wav"));
            dawsome_ok(&[
                "render".as_ref(),
                project.as_os_str(),
                out.as_os_str(),
                "--bars".as_ref(),
                bars.as_ref(),
            ]);
            let frames = read_mono_of_stereo(&out);
            let length = render["frames"].as_u64().expect("frames") as usize;
            assert_eq!(frames.len(), length, "{case} --bars {bars}");
            let onsets: Vec<usize> = render["onsets"]
                .as_array()
                .expect("onsets")
                .iter()
                .map(|at| at.as_u64().expect("a frame") as usize)
                .collect();
            for (at, &value) in frames.iter().enumerate() {
                let want = if onsets.contains(&at) { 1.0 } else { 0.0 };
                assert_eq!(value, want, "{case} --bars {bars}, frame {at}");
            }
        }
    }
}

// Known answer 7: the pattern file (smf-export) and the song file (export-midi) of projects in
// 3/4, 6/8 and 15/16 are the hand-written bytes: the time signature FF 58 04 03 02 18 08,
// FF 58 04 06 03 0C 08 and FF 58 04 0F 04 06 08 at tick 0, the pattern's end and the song's
// repeats at the signature's bar.
#[test]
fn the_midi_files_carry_the_signature_and_its_bar() {
    let dir = temp_dir("midi");
    let want = expected();
    for (case, answer) in want["midi"].as_object().expect("cases") {
        let project = fixture(answer["project"].as_str().expect("project"));
        let meta: Vec<u8> = answer["time_signature"]
            .as_array()
            .expect("bytes")
            .iter()
            .map(|byte| byte.as_u64().expect("a byte") as u8)
            .collect();
        for (command, key) in [("smf-export", "pattern"), ("export-midi", "song")] {
            let out = dir.join(format!("{case}_{key}.mid"));
            dawsome_ok(&[command.as_ref(), project.as_os_str(), out.as_os_str()]);
            let bytes = fs::read(&out).expect("the MIDI file");
            let hand = fs::read(fixture(answer[key].as_str().expect("a file"))).expect(".mid");
            assert_eq!(bytes, hand, "{case} {command}");
            assert!(
                bytes.windows(meta.len()).any(|window| window == meta),
                "{case} {command}: the time signature event"
            );
        }
    }
}

/// The entries of a zip whose entries are all stored (not compressed), read from its local
/// headers: (name, bytes), in file order.
fn stored_entries(zip: &[u8]) -> Vec<(String, Vec<u8>)> {
    let u16_at = |at: usize| u16::from_le_bytes([zip[at], zip[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes([zip[at], zip[at + 1], zip[at + 2], zip[at + 3]]);
    let mut entries = Vec::new();
    let mut at = 0;
    while at + 30 <= zip.len() && u32_at(at) == 0x0403_4b50 {
        assert_eq!(u16_at(at + 8), 0, "entry {} is stored", entries.len());
        let size = u32_at(at + 18) as usize;
        let name_len = usize::from(u16_at(at + 26));
        let extra_len = usize::from(u16_at(at + 28));
        let name = String::from_utf8(zip[at + 30..at + 30 + name_len].to_vec()).expect("utf-8");
        let data = at + 30 + name_len + extra_len;
        entries.push((name, zip[data..data + size].to_vec()));
        at = data + size;
    }
    entries
}

// Known answer 8: gen_p18's pattern project in 6/8 exports as the hand-written project.xml:
// <TimeSignature numerator="6" denominator="8"/>, and each clip's notes unrolled over the
// pattern's 288 ticks (beats 0, 2, 3, 5, 6 in the first clip; 0, 2, 3 in the muted one).
#[test]
fn the_dawproject_carries_the_signature_and_its_bar() {
    let dir = temp_dir("dawproject");
    let want = &expected()["dawproject"];
    let project = fixture(want["project"].as_str().expect("project"));
    let out = dir.join("p34_daw_68.dawproject");
    let stderr = dawsome_ok(&[
        "export-dawproject".as_ref(),
        project.as_os_str(),
        out.as_os_str(),
    ]);
    let entries = stored_entries(&fs::read(&out).expect("the zip"));
    let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["project.xml", "metadata.xml"]);
    let xml = fs::read(fixture("p34_daw_68.project.xml")).expect("the hand-written document");
    assert_eq!(
        entries[0].1,
        xml,
        "project.xml:\n{}",
        String::from_utf8_lossy(&entries[0].1)
    );
    assert_eq!(
        String::from_utf8_lossy(&entries[1].1),
        want["metadata"].as_str().expect("metadata")
    );
    let line = want["report"][0].as_str().expect("a line");
    assert!(stderr.contains(line), "the report: {stderr}");
}
