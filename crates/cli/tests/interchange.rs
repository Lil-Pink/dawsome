//! Interchange export through the real `dawsome` binary: the song as one MIDI file
//! (`export-midi`), stems (`export-stems`) and DAWproject (`export-dawproject`), each with the
//! report it writes beside its file. Every expected value comes from
//! tests/fixtures/gen_p18_expected.py (the MIDI bytes and project.xml files written out by hand
//! there), never from the engine.

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
        .join("interchange")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn expected() -> Value {
    let text = fs::read_to_string(fixture("p18_expected.json")).expect("p18_expected.json");
    serde_json::from_str(&text).expect("p18_expected.json parses")
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

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("a list")
        .iter()
        .map(|line| line.as_str().expect("a string").to_owned())
        .collect()
}

/// The report written beside `export` (`<export>.report.txt`): "Exported <export>", then
/// "Not carried over:" and `lines`, two spaces in. The command prints the same text.
fn assert_report(export: &Path, lines: &[String], stderr: &str) {
    let mut name = export.as_os_str().to_owned();
    name.push(".report.txt");
    let text = fs::read_to_string(&name).expect("the report beside the export");
    let mut want = vec![format!("Exported {}", export.display())];
    if lines.is_empty() {
        want.push("Everything carried over.".to_owned());
    } else {
        want.push("Not carried over:".to_owned());
        want.extend(lines.iter().map(|line| format!("  {line}")));
    }
    assert_eq!(text.lines().collect::<Vec<_>>(), want, "{}", name.display());
    assert!(text.ends_with('\n'));
    assert!(
        stderr.contains(&text),
        "the command prints its report: {stderr}"
    );
}

// ---------- E1: the song as one MIDI file ----------

// Known answers E1: p18_song_repeat (a clip repeating its pattern, one played from an offset,
// a muted clip, an overlapping same-key note cut and a same-tick one left out, an audio channel's
// empty track) and p18_song_tempo (140 BPM, a chord, a velocity of 0 and of 1, a note cut at its
// clip's end, a channel with no notes) export as the bytes written out by hand.
#[test]
fn the_song_exports_as_the_hand_written_midi_bytes() {
    let dir = temp_dir("song_midi");
    let expected = expected();
    for (case, want) in expected["midi"].as_object().expect("cases") {
        let project = fixture(want["project"].as_str().expect("project"));
        let out = dir.join(format!("{case}.mid"));
        let stderr = dawsome_ok(&["export-midi".as_ref(), project.as_os_str(), out.as_os_str()]);
        let bytes = fs::read(&out).expect("the MIDI file");
        let hand = fs::read(fixture(want["mid"].as_str().expect("mid"))).expect("expected .mid");
        assert_eq!(bytes, hand, "{case}");
        assert_report(&out, &strings(&want["report"]), &stderr);
    }
}

// The song's file reads back through the existing smf-import: every track merged into pattern
// 1's channel 1, the notes as the hand-written events give them, the pattern grown to 3 bars.
#[test]
fn the_song_midi_reads_back_through_smf_import() {
    let dir = temp_dir("song_midi_reimport");
    let want = &expected()["midi"]["repeat"];
    let project = fixture("p18_song_repeat.json");
    let mid = dir.join("song.mid");
    dawsome_ok(&["export-midi".as_ref(), project.as_os_str(), mid.as_os_str()]);
    let back = dir.join("back.json");
    dawsome_ok(&[
        "smf-import".as_ref(),
        project.as_os_str(),
        mid.as_os_str(),
        back.as_os_str(),
    ]);
    let doc: Value = serde_json::from_str(&fs::read_to_string(&back).expect("written"))
        .expect("the imported project parses");
    let pattern = &doc["patterns"][0];
    assert_eq!(pattern["length_bars"], want["reimport"]["length_bars"]);
    let notes: Vec<(i64, i64, f64, i64)> = pattern["notes"]
        .as_array()
        .expect("notes")
        .iter()
        .map(|note| {
            (
                note["tick"].as_i64().expect("tick"),
                note["length"].as_i64().expect("length"),
                note["velocity"].as_f64().expect("velocity"),
                note.get("key").map_or(60, |key| key.as_i64().expect("key")),
            )
        })
        .collect();
    let hand: Vec<(i64, i64, f64, i64)> = want["reimport"]["notes"]
        .as_array()
        .expect("notes")
        .iter()
        .map(|note| {
            (
                note[0].as_i64().expect("tick"),
                note[1].as_i64().expect("length"),
                note[2].as_f64().expect("velocity"),
                note[3].as_i64().expect("key"),
            )
        })
        .collect();
    assert_eq!(notes, hand);
}

// Known answer from gen_p18 (p18_pattern_fine.mid): the pattern export of a project at PPQ 960
// makes every note at least one file tick long, so a note whose two ends round to one tick is
// written note-on then note-off, never the other way round.
#[test]
fn a_pattern_at_a_finer_ppq_exports_every_note_a_tick_long() {
    let dir = temp_dir("pattern_fine");
    let want = &expected()["pattern_export"];
    let project = fixture(want["project"].as_str().expect("project"));
    let out = dir.join("pattern.mid");
    dawsome_ok(&["smf-export".as_ref(), project.as_os_str(), out.as_os_str()]);
    assert_eq!(
        fs::read(&out).expect("the MIDI file"),
        fs::read(fixture(want["mid"].as_str().expect("mid"))).expect("expected .mid")
    );
}

// An export that names the project as its output is refused, and the project keeps its bytes.
#[test]
fn an_export_over_its_own_project_is_refused() {
    let dir = temp_dir("export_over_project");
    let project = dir.join("song.json");
    fs::copy(fixture("p18_song_repeat.json"), &project).expect("copy");
    let before = fs::read(&project).expect("read");
    let output = dawsome(&[
        "export-midi".as_ref(),
        project.as_os_str(),
        project.as_os_str(),
    ]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(fs::read(&project).expect("read"), before);
}

// ---------- E2: stems ----------

/// Reads a WAV back as its two sides, requiring 32-bit float stereo.
fn read_float_stereo(path: &Path) -> (Vec<f32>, Vec<f32>) {
    let mut reader = hound::WavReader::open(path).expect("open wav");
    let spec = reader.spec();
    assert_eq!(spec.channels, 2, "{}: stereo", path.display());
    assert_eq!(spec.sample_format, hound::SampleFormat::Float);
    assert_eq!(spec.bits_per_sample, 32);
    let samples: Vec<f32> = reader
        .samples::<f32>()
        .collect::<Result<_, _>>()
        .expect("decode samples");
    (
        samples.iter().copied().step_by(2).collect(),
        samples.iter().copied().skip(1).step_by(2).collect(),
    )
}

/// The files in `dir`, by name.
fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

fn export_stems(dir: &Path, extra: &[&str]) -> (PathBuf, String) {
    let want = &expected()["stems"];
    let project = fixture(want["project"].as_str().expect("project"));
    let cache = dir.join("stretch-cache");
    let mut args: Vec<&std::ffi::OsStr> = vec![
        "export-stems".as_ref(),
        project.as_os_str(),
        dir.as_os_str(),
        "--cache-dir".as_ref(),
        cache.as_os_str(),
    ];
    args.extend(extra.iter().map(|arg| std::ffi::OsStr::new(*arg)));
    let stderr = dawsome_ok(&args);
    (dir.join(want["folder"].as_str().expect("folder")), stderr)
}

// Known answers E2: one file per playlist track (01 Drums, 02 Hats_Top with the slash made safe,
// 03..08 silent: automation alone, muted, empty) and master.wav, every one the song's 2 bars
// (176400 frames), each holding exactly the impulses worked out by hand: the automation on track
// 3 plays in Drums' file, the muted track 4 stays silent, and the project's loop range (0..384)
// is left out, so every file plays the song through.
#[test]
fn stems_are_a_file_a_track_and_the_master_of_one_length() {
    let dir = temp_dir("stems");
    let (folder, stderr) = export_stems(&dir, &[]);
    let want = &expected()["stems"];
    let frames = usize::try_from(want["frames"].as_u64().expect("frames")).expect("fits");
    let files = want["files"].as_array().expect("files");
    let mut names: Vec<String> = files
        .iter()
        .map(|file| file[0].as_str().expect("name").to_owned())
        .collect();
    names.sort();
    assert_eq!(names_in(&folder), names);
    for file in files {
        let name = file[0].as_str().expect("name");
        let (left, right) = read_float_stereo(&folder.join(name));
        assert_eq!(left.len(), frames, "{name}: frames");
        let levels: Vec<(usize, f32)> = file[1]
            .as_array()
            .expect("levels")
            .iter()
            .map(|level| {
                let frame = usize::try_from(level[0].as_u64().expect("frame")).expect("fits");
                (frame, level[1].as_f64().expect("level") as f32)
            })
            .collect();
        for (side, samples) in [("left", &left), ("right", &right)] {
            let heard: Vec<(usize, f32)> = samples
                .iter()
                .enumerate()
                .filter(|(_, &value)| value != 0.0)
                .map(|(frame, &value)| (frame, value))
                .collect();
            assert_eq!(heard, levels, "{name}, {side}");
        }
    }
    assert_report(&folder, &strings(&want["report"]), &stderr);
}

// A stem is the solo render through the existing render path (`render --song` of the project
// with the other tracks' clips muted and no loop, as gen_p18 writes it), byte for byte, and
// master.wav is the render of the song.
#[test]
fn a_stem_is_the_solo_render_and_the_master_the_song() {
    stems_are_their_solo_renders("stems_solo", &[]);
}

// At 16 bits too: each stem is dithered as a render is (its own generator from the one seed, so
// a stem's bytes are `render --song --bits 16` of its project), and with --no-dither rounded as a
// render with --no-dither is.
#[test]
fn a_sixteen_bit_stem_is_dithered_as_its_solo_render() {
    stems_are_their_solo_renders("stems_solo16", &["--bits", "16"]);
    stems_are_their_solo_renders("stems_solo16_plain", &["--bits", "16", "--no-dither"]);
}

/// Exports the stems with `extra` and holds each file to `render --song <extra>` of its solo
/// project, and master.wav to that of the song, byte for byte.
fn stems_are_their_solo_renders(test: &str, extra: &[&str]) {
    let dir = temp_dir(test);
    let (folder, _) = export_stems(&dir, extra);
    let want = &expected()["stems"];
    let bars = want["bars"].as_u64().expect("bars").to_string();
    let mut pairs: Vec<(String, String)> = want["solo"]
        .as_object()
        .expect("solo")
        .iter()
        .map(|(file, project)| (file.clone(), project.as_str().expect("project").to_owned()))
        .collect();
    pairs.push((
        "master.wav".to_owned(),
        want["master"].as_str().expect("master").to_owned(),
    ));
    for (file, project) in pairs {
        let rendered = dir.join(format!("render-{project}.wav"));
        let project_path = fixture(&project);
        let mut args: Vec<&std::ffi::OsStr> = vec![
            "render".as_ref(),
            project_path.as_os_str(),
            rendered.as_os_str(),
            "--song".as_ref(),
            "--bars".as_ref(),
            bars.as_ref(),
        ];
        args.extend(extra.iter().map(|arg| std::ffi::OsStr::new(*arg)));
        dawsome_ok(&args);
        assert_eq!(
            fs::read(folder.join(&file)).expect("stem"),
            fs::read(&rendered).expect("render"),
            "{file} against render --song of {project}"
        );
    }
}

// `--bits` sets the stems' sample format as it sets a render's: 16-bit integer files, of the
// same length.
#[test]
fn stems_take_the_bit_depth_a_render_takes() {
    let dir = temp_dir("stems_bits");
    let (folder, _) = export_stems(&dir, &["--bits", "16"]);
    let want = &expected()["stems"];
    let frames = u32::try_from(want["frames"].as_u64().expect("frames")).expect("fits");
    for name in names_in(&folder) {
        let reader = hound::WavReader::open(folder.join(&name)).expect("open");
        let spec = reader.spec();
        assert_eq!(spec.sample_format, hound::SampleFormat::Int, "{name}");
        assert_eq!(spec.bits_per_sample, 16, "{name}");
        assert_eq!(reader.duration(), frames, "{name}");
    }
}

// ---------- E3: DAWproject ----------

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
        assert_eq!(u32_at(at + 22) as usize, size, "stored: both sizes equal");
        let name_len = usize::from(u16_at(at + 26));
        let extra_len = usize::from(u16_at(at + 28));
        let name = String::from_utf8(zip[at + 30..at + 30 + name_len].to_vec()).expect("utf-8");
        let data = at + 30 + name_len + extra_len;
        entries.push((name, zip[data..data + size].to_vec()));
        at = data + size;
    }
    assert!(!entries.is_empty(), "a zip with entries");
    entries
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// Known answers E3: p18_daw_pattern (a pattern clip repeating, a muted clip), p18_daw_audio (an
// audio clip from an offset, a resampled one with a gain) and p18_daw_mix (volume, pan, mute, a
// CLAP instrument with its state) export as zips holding the hand-written project.xml, the
// metadata, the audio file and the plugin's state, in that order, and the report's lines.
#[test]
fn dawproject_zips_hold_the_hand_written_documents() {
    let dir = temp_dir("dawproject");
    let expected = expected();
    for (case, want) in expected["dawproject"].as_object().expect("cases") {
        let project_name = want["project"].as_str().expect("project");
        let project = fixture(project_name);
        let stem = project_name.trim_end_matches(".json");
        let out = dir.join(format!("{stem}.dawproject"));
        let stderr = dawsome_ok(&[
            "export-dawproject".as_ref(),
            project.as_os_str(),
            out.as_os_str(),
        ]);
        let entries = stored_entries(&fs::read(&out).expect("the zip"));
        let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
        let want_names: Vec<&str> = want["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .map(|entry| entry[0].as_str().expect("name"))
            .collect();
        assert_eq!(names, want_names, "{case}");
        for ((name, bytes), entry) in entries.iter().zip(want["entries"].as_array().expect("e")) {
            match (name.as_str(), entry[1].as_str()) {
                (_, Some(file)) => assert_eq!(
                    bytes,
                    &fs::read(fixture(file)).expect("fixture"),
                    "{case}: {name} against {file}:\n{}",
                    String::from_utf8_lossy(bytes)
                ),
                ("metadata.xml", None) => assert_eq!(
                    String::from_utf8_lossy(bytes),
                    want["metadata"].as_str().expect("metadata"),
                    "{case}"
                ),
                (_, None) => assert_eq!(
                    hex(bytes),
                    want["state_hex"].as_str().expect("state"),
                    "{case}: {name}"
                ),
            }
        }
        assert_report(&out, &strings(&want["report"]), &stderr);
    }
}

/// Runs tests/scripts/dawproject_check.py over `files`: the documents against the format's
/// published schema, the zip's CRCs, every packed path present.
fn schema_check(files: &[PathBuf]) {
    // Windows installs Python 3 as `python`; its `python3` may be a store shortcut that runs
    // nothing.
    let python = if cfg!(windows) { "python" } else { "python3" };
    let output = Command::new(python)
        .arg("-I")
        .arg("-B")
        .arg(repo_root().join("tests/scripts/dawproject_check.py"))
        .args(files)
        .output()
        .expect("python runs");
    assert!(
        output.status.success(),
        "dawproject_check: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

// What the command writes follows the DAWproject 1.0 schema (Project.xsd, MetaData.xsd,
// vendored in tests/fixtures/dawproject/), checked by the schema checker over the zip itself: the
// three known-answer projects and the demo song (samplers, inserts, automation, repeats).
#[test]
fn dawproject_exports_follow_the_published_schema() {
    let dir = temp_dir("dawproject_schema");
    let projects = [
        fixture("p18_daw_pattern.json"),
        fixture("p18_daw_audio.json"),
        fixture("p18_daw_mix.json"),
        repo_root().join("examples/demo.json"),
    ];
    let mut written = Vec::new();
    for project in projects {
        let stem = project.file_stem().expect("a name").to_owned();
        let out = dir.join(stem).with_extension("dawproject");
        dawsome_ok(&[
            "export-dawproject".as_ref(),
            project.as_os_str(),
            out.as_os_str(),
        ]);
        written.push(out);
    }
    schema_check(&written);
}
