//! Time signatures per pattern and along the song through the real `dawsome` binary: a
//! pattern's own bars in the song and alone (`render`), its MIDI file (`smf-export`), the song's
//! MIDI file with each change (`export-midi`), its DAWproject (`export-dawproject`), `--bars`
//! across the changes and the stems' end. Every expected value comes from
//! tests/fixtures/gen_p35_expected.py, never from the engine.

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
        .join("meter")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn expected() -> Value {
    let text = fs::read_to_string(fixture("p35_expected.json")).expect("p35_expected.json");
    serde_json::from_str(&text).expect("p35_expected.json parses")
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

/// `dawsome render project out <args>`: the file is `frames` long and 1.0 exactly on `onsets`,
/// silent elsewhere.
fn assert_render(project: &str, render: &Value, out: &Path) {
    let project = fixture(project);
    let mut args: Vec<&std::ffi::OsStr> =
        vec!["render".as_ref(), project.as_os_str(), out.as_os_str()];
    args.extend(
        render["args"]
            .as_array()
            .expect("args")
            .iter()
            .map(|arg| std::ffi::OsStr::new(arg.as_str().expect("an arg"))),
    );
    dawsome_ok(&args);
    let frames = read_mono_of_stereo(out);
    let shown = &render["args"];
    assert_eq!(
        frames.len() as u64,
        render["frames"].as_u64().expect("frames"),
        "{shown}"
    );
    let onsets: Vec<usize> = render["onsets"]
        .as_array()
        .expect("onsets")
        .iter()
        .map(|at| at.as_u64().expect("a frame") as usize)
        .collect();
    for (at, &value) in frames.iter().enumerate() {
        let want = if onsets.contains(&at) { 1.0 } else { 0.0 };
        assert_eq!(value, want, "{shown}, frame {at}");
    }
}

// Known answers B 4: pattern 2's own 3/4 makes it 288 ticks wherever it plays: its 384-tick
// clip plays a second pass at s(288) = 66150 before pattern 1's clip at s(384) = 88200 (two song
// bars of 4/4, 176400 frames); alone, `--bars 2` is two of its own bars, 132300 frames, hits at 0
// and 66150.
#[test]
fn a_pattern_plays_its_own_bars_in_the_song_and_alone() {
    let dir = temp_dir("pattern");
    let want = expected();
    let project = want["b"]["project"].as_str().expect("a file");
    for (index, render) in want["b"]["renders"]
        .as_array()
        .expect("renders")
        .iter()
        .enumerate()
    {
        assert_render(project, render, &dir.join(format!("b{index}.wav")));
    }
}

// Known answers C 6: `--bars 6` ends where bar 7 of the map starts, s(1920) = 441000 frames, the
// pattern's hits every 384 ticks before it.
#[test]
fn bars_count_across_the_changes() {
    let dir = temp_dir("bars");
    let want = expected();
    assert_render(
        want["c"]["project"].as_str().expect("a file"),
        &want["c"]["render"],
        &dir.join("c.wav"),
    );
}

// Known answers B 4 and C 6: pattern 2's MIDI file carries its own 3/4 (FF 58 04 03 02 18 08),
// the song's keeps 4/4 and plays pattern 2 at its 288 ticks; the map's song file has FF 58 at 0,
// 768, 1344 and 1920 in its conductor: each the hand-written bytes.
#[test]
fn the_midi_files_carry_the_patterns_signature_and_the_songs_changes() {
    let dir = temp_dir("midi");
    let want = expected();
    let b = &want["b"];
    let p34 = fixture(b["project"].as_str().expect("a file"));
    let pattern = &b["midi"]["pattern"];
    let out = dir.join("pattern.mid");
    dawsome_ok(&[
        "smf-export".as_ref(),
        p34.as_os_str(),
        out.as_os_str(),
        pattern[0].as_str().expect("--pattern").as_ref(),
        pattern[1].as_str().expect("2").as_ref(),
    ]);
    let hand = fs::read(fixture(pattern[2].as_str().expect("a file"))).expect(".mid");
    assert_eq!(fs::read(&out).expect("written"), hand, "pattern 2's file");
    for (project, file) in [
        (p34.clone(), b["midi"]["song"].as_str().expect("a file")),
        (
            fixture(want["c"]["project"].as_str().expect("a file")),
            want["c"]["midi"]["song"].as_str().expect("a file"),
        ),
    ] {
        let out = dir.join(file);
        dawsome_ok(&["export-midi".as_ref(), project.as_os_str(), out.as_os_str()]);
        let hand = fs::read(fixture(file)).expect(".mid");
        assert_eq!(fs::read(&out).expect("written"), hand, "{file}");
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

// Known answers C 6: the map's DAWproject is the hand-written project.xml: the Transport's 4/4
// named "time-signature" and a TimeSignatureAutomation of points at beats 0, 8, 14 and 20.
#[test]
fn the_dawproject_carries_the_changes() {
    let dir = temp_dir("dawproject");
    let want = expected();
    let daw = &want["c"]["dawproject"];
    let project = fixture(want["c"]["project"].as_str().expect("a file"));
    let out = dir.join("p35_meter.dawproject");
    let stderr = dawsome_ok(&[
        "export-dawproject".as_ref(),
        project.as_os_str(),
        out.as_os_str(),
    ]);
    let entries = stored_entries(&fs::read(&out).expect("the zip"));
    let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["project.xml", "metadata.xml"]);
    let xml = fs::read(fixture("p35_meter.project.xml")).expect("the hand-written document");
    assert_eq!(
        entries[0].1,
        xml,
        "project.xml:\n{}",
        String::from_utf8_lossy(&entries[0].1)
    );
    assert_eq!(
        String::from_utf8_lossy(&entries[1].1),
        daw["metadata"].as_str().expect("metadata")
    );
    let line = daw["report"][0].as_str().expect("a line");
    assert!(stderr.contains(line), "the report: {stderr}");
}

// Known answers C 6: a song ending at tick 1700, inside bar 6 of the map (1632..1920), gives
// stems that end where bar 7 starts, s(1920) = 441000 frames each (not a 4/4 bar's 2304).
#[test]
fn the_stems_end_on_the_maps_bar() {
    let dir = temp_dir("stems");
    let want = &expected()["c"]["stems"];
    let project = fixture(want["project"].as_str().expect("a file"));
    let cache = dir.join("stretch-cache");
    dawsome_ok(&[
        "export-stems".as_ref(),
        project.as_os_str(),
        dir.as_os_str(),
        "--cache-dir".as_ref(),
        cache.as_os_str(),
    ]);
    let folder = dir.join("p35_meter_stems stems");
    let mut names: Vec<String> = fs::read_dir(&folder)
        .expect("the stems folder")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    let files: Vec<&str> = want["files"]
        .as_array()
        .expect("files")
        .iter()
        .map(|file| file.as_str().expect("a name"))
        .collect();
    assert_eq!(names, files);
    for file in files {
        let frames = read_mono_of_stereo(&folder.join(file)).len() as u64;
        assert_eq!(frames, want["frames"].as_u64().expect("frames"), "{file}");
    }
}
