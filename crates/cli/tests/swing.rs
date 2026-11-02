//! Swing through the real `dawsome` binary: a swung pattern and a swung song render to the
//! frames tests/fixtures/gen_p31_expected.py states, and both MIDI exports (`smf-export`, the
//! pattern's, and `export-midi`, the song's) write the notes where the swing plays them, as the
//! bytes written out by hand there: a step hit swing moved lasts one step less its delay, so it
//! ends where the next step starts and no hit of a key overlaps the next. Expected values come
//! from the generator, never from the engine.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
        .join("swing")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn expected() -> Value {
    let text = fs::read_to_string(fixture("p31_expected.json"))
        .expect("p31_expected.json (run gen_p31_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("p31_expected.json parses")
}

fn text(value: &Value) -> &str {
    value.as_str().expect("a string")
}

/// Runs `dawsome` with `args`, its cache in `dir`, requiring success; returns its stderr.
fn dawsome_ok(dir: &Path, args: &[&OsStr]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .env("DAWSOME_CACHE_HOME", dir.join("cache"))
        .args(args)
        .output()
        .expect("run dawsome binary");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "dawsome {args:?} failed ({}): {stderr}",
        output.status
    );
    stderr
}

/// The frames that sound in a render the generator states (its runs of [first frame, frames,
/// value]), by frame.
fn sounding(answer: &Value) -> BTreeMap<usize, f32> {
    let mut frames = BTreeMap::new();
    for run in answer["runs"].as_array().expect("a list") {
        let first = run[0].as_u64().expect("a frame") as usize;
        let count = run[1].as_u64().expect("a count") as usize;
        let value = run[2].as_f64().expect("a value") as f32;
        frames.extend((first..first + count).map(|at| (at, value)));
    }
    frames
}

/// A rendered WAV read back as its frames, requiring 32-bit float stereo.
fn read_stereo(path: &Path) -> Vec<[f32; 2]> {
    let mut reader = hound::WavReader::open(path).expect("open rendered wav");
    let spec = reader.spec();
    assert_eq!(spec.channels, 2, "output must be stereo");
    assert_eq!(spec.sample_format, hound::SampleFormat::Float);
    assert_eq!(spec.bits_per_sample, 32);
    let samples: Vec<f32> = reader
        .samples::<f32>()
        .collect::<Result<_, _>>()
        .expect("decode samples");
    samples
        .chunks_exact(2)
        .map(|frame| [frame[0], frame[1]])
        .collect()
}

// Known answers (b) and (e): the 16 steps at 66.7 % (ticks 48p and 48p + 32, two passes) and
// the swung song (clip A from offset 28 and clip B, the pattern twice, playing the swung ticks;
// three bars), through `dawsome render`, every frame to the bit on both sides.
#[test]
fn swung_renders_land_on_their_known_frames() {
    let dir = temp_dir("render");
    let want = expected();
    for case in ["triplet_120", "song"] {
        let answer = &want["renders"][case];
        let project = fixture(text(&answer["fixture"]));
        let bars = answer["bars"].as_u64().expect("bars").to_string();
        let out = dir.join(format!("{case}.wav"));
        let mut args: Vec<&OsStr> = vec![
            "render".as_ref(),
            project.as_os_str(),
            out.as_os_str(),
            "--bars".as_ref(),
            bars.as_ref(),
        ];
        if answer["song"].as_bool().expect("a mode") {
            args.push("--song".as_ref());
        }
        dawsome_ok(&dir, &args);
        let frames = answer["frames"].as_u64().expect("frames") as usize;
        let heard = sounding(answer);
        let audio = read_stereo(&out);
        assert_eq!(audio.len(), frames, "{case}: frames");
        for (at, got) in audio.iter().enumerate() {
            let value = heard.get(&at).copied().unwrap_or(0.0);
            assert_eq!(
                got.map(f32::to_bits),
                [value.to_bits(); 2],
                "{case}, frame {at}"
            );
        }
    }
}

// Known answers (f), the pattern's export: p31_export (a step on a pair's start, a note of
// length 12 on the second 16th, which plays at 32 for its 12, and a step on the second 16th,
// which plays at 176 for 16) and the 16 steps at 66.7 % (each moved hit ending where the next
// pair starts) are the hand-written bytes.
#[test]
fn the_pattern_export_writes_the_notes_where_swing_plays_them() {
    let dir = temp_dir("pattern_midi");
    let want = expected();
    for (case, files) in want["exports"]["pattern"].as_object().expect("cases") {
        let project = fixture(text(&files["project"]));
        let out = dir.join(format!("{case}.mid"));
        dawsome_ok(
            &dir,
            &["smf-export".as_ref(), project.as_os_str(), out.as_os_str()],
        );
        assert_eq!(
            fs::read(&out).expect("the MIDI file"),
            fs::read(fixture(text(&files["mid"]))).expect("expected .mid"),
            "{case}"
        );
    }
}

// Known answers (f), the song's export: the same two patterns in one clip each are the
// hand-written bytes, and the report names nothing but what a MIDI file never holds: no note
// was shortened, since a moved step hit already ends where the same key starts again.
#[test]
fn the_song_export_writes_the_notes_where_swing_plays_them() {
    let dir = temp_dir("song_midi");
    let want = expected();
    for (case, files) in want["exports"]["song"].as_object().expect("cases") {
        let project = fixture(text(&files["project"]));
        let out = dir.join(format!("{case}.mid"));
        let stderr = dawsome_ok(
            &dir,
            &["export-midi".as_ref(), project.as_os_str(), out.as_os_str()],
        );
        assert_eq!(
            fs::read(&out).expect("the MIDI file"),
            fs::read(fixture(text(&files["mid"]))).expect("expected .mid"),
            "{case}"
        );
        let mut report = out.clone().into_os_string();
        report.push(".report.txt");
        let report = fs::read_to_string(&report).expect("the report beside the export");
        let mut lines = vec![
            format!("Exported {}", out.display()),
            "Not carried over:".to_owned(),
        ];
        lines.extend(
            files["report"]
                .as_array()
                .expect("report lines")
                .iter()
                .map(|line| format!("  {}", text(line))),
        );
        assert_eq!(report.lines().collect::<Vec<_>>(), lines, "{case}");
        assert!(
            stderr.contains(&report),
            "the command prints its report: {stderr}"
        );
    }
}
