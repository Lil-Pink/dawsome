//! Instrument channels through the command line: the test sets of tests/fixtures/instruments
//! render the frames tests/fixtures/gen_p11_expected.py states (p11_expected.json), whatever
//! the block size; a set's load report names what the reader left out; and a project naming a
//! shipped set or sound by its `dawsome:` reference renders the same bytes from any user's data
//! folder that holds it. Every run gets per-user folders of its own under the test's temp dir.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> PathBuf {
    repo_root().join("tests/fixtures")
}

/// Fresh empty directory under cargo's per-target test directory.
fn temp_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("instruments")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p11_expected.json")).expect("p11_expected.json");
    serde_json::from_str(&text).expect("parses")
}

/// Runs `dawsome render <project> <out> <extra>` from the repository root with every per-user
/// folder under `home` and the data folder at `data`.
fn render(project: &Path, out: &Path, extra: &[&str], home: &Path, data: &Path) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .env("DAWSOME_DATA_HOME", data)
        .env("DAWSOME_CACHE_HOME", home.join("cache"))
        .env("DAWSOME_CONFIG_HOME", home.join("config"))
        .env("DAWSOME_STATE_HOME", home.join("state"))
        .env("XDG_DATA_HOME", home.join("xdg-data"))
        .env("XDG_CACHE_HOME", home.join("xdg-cache"))
        .arg("render")
        .arg(project)
        .arg(out)
        .args(extra)
        .output()
        .expect("run dawsome");
    assert!(
        output.status.success(),
        "dawsome render {} failed: {}",
        project.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// A render's frames, left and right.
fn read_stereo(path: &Path) -> Vec<[f32; 2]> {
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

/// Asserts `frames` is exactly the expected render: its sounding frames, all else silence.
fn assert_render(name: &str, frames: &[[f32; 2]], want: &Value) {
    assert_eq!(
        frames.len() as u64,
        want["frames"].as_u64().expect("frames"),
        "{name}"
    );
    let mut sounding = std::collections::BTreeMap::new();
    for entry in want["nonzero"].as_array().expect("a list") {
        let at = entry[0].as_u64().expect("a frame") as usize;
        let side = |index: usize| entry[index].as_f64().expect("a value") as f32;
        sounding.insert(at, [side(1), side(2)]);
    }
    for (at, got) in frames.iter().enumerate() {
        let want = sounding.get(&at).copied().unwrap_or([0.0; 2]);
        assert_eq!(
            got.map(f32::to_bits),
            want.map(f32::to_bits),
            "{name}, frame {at}: {got:?} != {want:?}"
        );
    }
}

/// Known answers (gen_p11_expected.py): layers picks a zone by key and velocity layer and
/// layers the click over the top range; pad loops continuously and while held and releases in
/// decibels after its attack; rr takes turns at the velocity curve's gains; macro reads
/// #define, #include, default_path, note names, volume, tune, transpose, pan, offset and end.
#[test]
fn p11_sets_render_their_known_answers() {
    let want = expected();
    let dir = temp_dir("known");
    for name in ["layers", "pad", "rr", "macro"] {
        let answer = &want["renders"][name];
        let project = fixtures().join(answer["fixture"].as_str().expect("fixture"));
        let out = dir.join(format!("{name}.wav"));
        render(&project, &out, &[], &dir, &dir.join("data"));
        assert_render(name, &read_stereo(&out), answer);
    }
}

/// Known answers (gen_p11_expected.py's set "formats"): what real libraries write that the other
/// sets leave out (sound files in AIFF and AIFC or whose fmt chunk is a size the spec does not
/// give, a volume past SFZ's range) plays each sound's frames as written, and the load report
/// counts the problems the generator states and names the first.
#[test]
fn p11_what_real_libraries_write_renders_its_known_answer() {
    let want = expected();
    let dir = temp_dir("formats");
    let answer = &want["renders"]["formats"];
    let project = fixtures().join(answer["fixture"].as_str().expect("fixture"));
    let out = dir.join("formats.wav");
    let output = render(&project, &out, &[], &dir, &dir.join("data"));
    assert_render("formats", &read_stereo(&out), answer);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let problems = want["formats"]["problems"].as_array().expect("problems");
    let first = problems.first().map(|p| p.as_str().expect("a problem"));
    match (problems.len(), first) {
        (_, None) => assert!(stderr.is_empty(), "{stderr}"),
        (1, Some(first)) => assert!(stderr.contains(&format!("1 problem: {first}")), "{stderr}"),
        (count, Some(first)) => assert!(
            stderr.contains(&format!("{count} problems, the first: {first}")),
            "{stderr}"
        ),
    }
}

/// Known answers (gen_p11_expected.py): a velocity that is no whole number of 128ths plays the
/// nearest byte, not the one below. layers at 0.505 (64.64) plays byte 65, its loud layer;
/// pad at 0.7 (89.6) plays byte 90, at gain (90/127)^2 (found 2026-10-22: a
/// mutant rounding down passed every test).
#[test]
fn p11_a_velocity_between_bytes_plays_the_nearest() {
    let want = expected();
    let dir = temp_dir("velocity");
    for name in ["velocity_layers", "velocity_pad"] {
        let answer = &want["renders"][name];
        let project = fixtures().join(answer["fixture"].as_str().expect("fixture"));
        let out = dir.join(format!("{name}.wav"));
        render(&project, &out, &[], &dir, &dir.join("data"));
        assert_render(name, &read_stereo(&out), answer);
    }
}

/// The pad's loops, attack and releases cross block edges: block sizes 1, 7, 64 and 4096 give
/// the bytes of the default 512.
#[test]
fn p11_pad_renders_the_same_bytes_at_any_block_size() {
    let dir = temp_dir("blocks");
    let project = fixtures().join("p11_pad.json");
    let reference = dir.join("512.wav");
    render(&project, &reference, &[], &dir, &dir.join("data"));
    let bytes = fs::read(&reference).expect("rendered");
    for block in ["1", "7", "64", "4096"] {
        let out = dir.join(format!("{block}.wav"));
        render(
            &project,
            &out,
            &["--block", block, "--bars", "1"],
            &dir,
            &dir.join("data"),
        );
        assert!(
            fs::read(&out).expect("rendered") == bytes,
            "--block {block} differs"
        );
    }
}

/// The load report: macro's set names what it leaves out, once each with how often; a set of
/// supported opcodes says nothing.
#[test]
fn a_sets_load_report_names_what_it_leaves_out() {
    let dir = temp_dir("report");
    let output = render(
        &fixtures().join("p11_macro.json"),
        &dir.join("m.wav"),
        &[],
        &dir,
        &dir.join("data"),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    let line = stderr
        .lines()
        .find(|line| line.contains("macro.sfz"))
        .unwrap_or_else(|| panic!("no report for macro.sfz in: {stderr}"));
    for ignored in expected()["ignored"]["macro"].as_array().expect("a list") {
        let name = ignored["name"].as_str().expect("a name");
        assert!(
            line.contains(&format!("{name} (\u{d7}1)")),
            "{name} in {line}"
        );
    }
    let quiet = render(
        &fixtures().join("p11_layers.json"),
        &dir.join("l.wav"),
        &[],
        &dir,
        &dir.join("data"),
    );
    assert!(
        quiet.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&quiet.stderr)
    );
}

/// Copies the folder `from` into `to`, files and folders.
fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("dir");
    for entry in fs::read_dir(from).expect("lists") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copies");
        }
    }
}

/// A data folder holding the layers set and the starter kit's Kick 808, as an install or the
/// Library puts them there.
fn data_folder(root: &Path) -> PathBuf {
    let shipped = root.join("dawsome");
    copy_dir(
        &fixtures().join("instruments/layers"),
        &shipped.join("instruments/layers"),
    );
    fs::create_dir_all(shipped.join("starter-kit")).expect("dir");
    fs::copy(
        repo_root().join("assets/sounds/Kick 808.wav"),
        shipped.join("starter-kit/Kick 808.wav"),
    )
    .expect("copies");
    root.to_path_buf()
}

/// Known answer and portability: p11_shipped names the layers set as
/// `dawsome:instruments/layers/layers.sfz`; rendered with two users' data folders (A and B)
/// that both hold it, it gives the layers' known frames, the same bytes each time. A data
/// folder without the set plays it silent, with a warning naming where it looked.
#[test]
fn a_shipped_set_renders_the_same_from_any_data_folder_that_holds_it() {
    let dir = temp_dir("shipped");
    let (a, b) = (data_folder(&dir.join("a")), data_folder(&dir.join("b")));
    let project = fixtures().join("p11_shipped.json");
    render(&project, &dir.join("a.wav"), &[], &dir, &a);
    render(&project, &dir.join("b.wav"), &[], &dir, &b);
    let bytes = fs::read(dir.join("a.wav")).expect("rendered");
    assert!(
        bytes == fs::read(dir.join("b.wav")).expect("rendered"),
        "A and B differ"
    );
    assert_render(
        "shipped",
        &read_stereo(&dir.join("a.wav")),
        &expected()["renders"]["layers"],
    );
    let empty = dir.join("empty");
    fs::create_dir_all(&empty).expect("dir");
    let output = render(&project, &dir.join("none.wav"), &[], &dir, &empty);
    assert!(read_stereo(&dir.join("none.wav"))
        .iter()
        .all(|frame| *frame == [0.0; 2]));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let looked = empty.join("dawsome/instruments/layers/layers.sfz");
    assert!(stderr.contains(&looked.display().to_string()), "{stderr}");
}

/// A starter-kit sound named `dawsome:starter-kit/Kick 808.wav` renders from data folders A and B
/// the bytes the same sound renders from beside the project.
#[test]
fn a_shipped_kit_sound_renders_as_the_file_beside_the_project_does() {
    let dir = temp_dir("kit");
    let (a, b) = (data_folder(&dir.join("a")), data_folder(&dir.join("b")));
    let document = |sample: &str| {
        format!(
            concat!(
                r#"{{"format_version":2,"sample_rate":44100,"tempo_mbpm":120000,"ppq":96,"#,
                r#""next_channel_id":2,"channels":[{{"id":1,"name":"Kick","kind":"sampler","#,
                r#""sample":"{}","volume":1.0,"pan":0.0,"mute":false}}],"patterns":[{{"id":1,"#,
                r#""name":"Pattern 1","length_bars":1,"notes":[{{"channel":1,"tick":0,"length":0,"#,
                r#""velocity":1.0}}]}}]}}"#
            ),
            sample
        )
    };
    let shipped = dir.join("shipped.json");
    fs::write(&shipped, document("dawsome:starter-kit/Kick 808.wav")).expect("written");
    let local = dir.join("local.json");
    fs::write(&local, document("Kick 808.wav")).expect("written");
    fs::copy(
        repo_root().join("assets/sounds/Kick 808.wav"),
        dir.join("Kick 808.wav"),
    )
    .expect("copies");
    render(&shipped, &dir.join("a.wav"), &[], &dir, &a);
    render(&shipped, &dir.join("b.wav"), &[], &dir, &b);
    render(&local, &dir.join("local.wav"), &[], &dir, &dir.join("data"));
    let local = fs::read(dir.join("local.wav")).expect("rendered");
    assert!(read_stereo(&dir.join("local.wav"))
        .iter()
        .any(|frame| *frame != [0.0; 2]));
    assert!(
        fs::read(dir.join("a.wav")).expect("rendered") == local,
        "A differs"
    );
    assert!(
        fs::read(dir.join("b.wav")).expect("rendered") == local,
        "B differs"
    );
}

/// A project named by a relative path (from the working directory) finds its set's samples
/// beside the set, not under the project's folder twice over: macro renders its known answer.
#[test]
fn a_project_named_by_a_relative_path_finds_its_sets_samples() {
    let dir = temp_dir("relative");
    let out = dir.join("macro.wav");
    let output = render(
        Path::new("tests/fixtures/p11_macro.json"),
        &out,
        &[],
        &dir,
        &dir.join("data"),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("cannot be read"), "{stderr}");
    assert_render("macro", &read_stereo(&out), &expected()["renders"]["macro"]);
}

/// A project of one instrument channel playing the set at `instrument` (as the project writes
/// it), one full-velocity step on key 60 at tick 0, at 44.1 kHz.
fn one_note_project(instrument: &str) -> String {
    let path = serde_json::to_string(instrument).expect("a path");
    format!(
        concat!(
            r#"{{"format_version":4,"sample_rate":44100,"tempo_mbpm":120000,"ppq":96,"#,
            r#""next_channel_id":2,"channels":[{{"id":1,"name":"Set","kind":"instrument","#,
            r#""instrument":{},"volume":1.0,"pan":0.0,"mute":false}}],"patterns":[{{"id":1,"#,
            r#""name":"Pattern 1","length_bars":1,"notes":[{{"channel":1,"tick":0,"length":0,"#,
            r#""velocity":1.0}}]}}]}}"#
        ),
        path
    )
}

/// A set's includes and sounds stay inside its folder (the main file's): an include climbing
/// out with `..`, one naming an absolute path and a sound outside are each left out, and
/// reported. Nothing of the files outside reaches the warning, and the region of the outside
/// sound stays silent (found 2026-10-22).
#[test]
fn a_set_reads_nothing_outside_its_folder() {
    let dir = temp_dir("confined");
    let (set, outside) = (dir.join("set"), dir.join("outside"));
    fs::create_dir_all(&set).expect("dir");
    fs::create_dir_all(&outside).expect("dir");
    fs::copy(fixtures().join("impulse.wav"), outside.join("secret.wav")).expect("copies");
    fs::write(outside.join("leak.sfz"), "<global> leaked_opcode=1\n").expect("written");
    let absolute = outside.join("absolute.sfz");
    fs::write(&absolute, "<global> absolute_opcode=1\n").expect("written");
    let sfz = format!(
        "<region> sample=../outside/secret.wav key=60\n#include \"../outside/leak.sfz\"\n#include \"{}\"\n",
        absolute.display()
    );
    fs::write(set.join("set.sfz"), sfz).expect("written");
    let project = dir.join("confined.json");
    fs::write(&project, one_note_project("set/set.sfz")).expect("written");
    let out = dir.join("confined.wav");
    let output = render(&project, &out, &[], &dir, &dir.join("data"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        read_stereo(&out).iter().all(|frame| *frame == [0.0; 2]),
        "the outside sound played"
    );
    assert!(
        !stderr.contains("leaked_opcode") && !stderr.contains("absolute_opcode"),
        "{stderr}"
    );
    assert!(
        stderr.contains("3 problems, the first: set.sfz:2: #include \"../outside/leak.sfz\": outside the set's folder"),
        "{stderr}"
    );
}

/// An include naming a named pipe is refused at once rather than read: before, the render
/// waited on the pipe for ever. The render is given 60 s.
#[cfg(unix)]
#[test]
fn an_include_naming_a_pipe_is_refused_without_waiting() {
    let dir = temp_dir("pipe");
    let set = dir.join("set");
    fs::create_dir_all(&set).expect("dir");
    let made = Command::new("mkfifo")
        .arg(set.join("pipe.sfz"))
        .status()
        .expect("mkfifo runs");
    assert!(made.success());
    fs::copy(fixtures().join("impulse.wav"), set.join("tone.wav")).expect("copies");
    fs::write(
        set.join("set.sfz"),
        "<region> sample=tone.wav key=60\n#include \"pipe.sfz\"\n",
    )
    .expect("written");
    let project = dir.join("pipe.json");
    fs::write(&project, one_note_project("set/set.sfz")).expect("written");
    let out = dir.join("pipe.wav");
    let mut child = Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .env("DAWSOME_DATA_HOME", dir.join("data"))
        .env("DAWSOME_CACHE_HOME", dir.join("cache"))
        .env("XDG_DATA_HOME", dir.join("xdg-data"))
        .env("XDG_CACHE_HOME", dir.join("xdg-cache"))
        .args(["render".as_ref(), project.as_os_str(), out.as_os_str()])
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("run dawsome");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().expect("waits") {
            break status;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the render waited on the pipe");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!(status.success());
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().expect("piped"), &mut stderr)
        .expect("reads");
    assert!(
        stderr.contains("#include \"pipe.sfz\": not a regular file"),
        "{stderr}"
    );
    assert!(read_stereo(&out).iter().any(|frame| *frame != [0.0; 2]));
}

/// A `dawsome:` reference names a file inside the data folder's `dawsome/` only: one climbing
/// out with `..`, and one naming an absolute path, play nothing and say they cannot be read
/// (found 2026-10-22). Before, both played the file they named.
#[test]
fn a_shipped_reference_names_nothing_outside_the_data_folder() {
    let dir = temp_dir("reference_escape");
    let data = dir.join("data");
    fs::create_dir_all(data.join("dawsome")).expect("dir");
    let secret = data.join("secret.wav");
    fs::copy(fixtures().join("impulse.wav"), &secret).expect("copies");
    for (name, sample) in [
        ("climbs", "dawsome:../secret.wav".to_owned()),
        ("absolute", format!("dawsome:{}", secret.display())),
    ] {
        let document = format!(
            concat!(
                r#"{{"format_version":2,"sample_rate":44100,"tempo_mbpm":120000,"ppq":96,"#,
                r#""next_channel_id":2,"channels":[{{"id":1,"name":"Kick","kind":"sampler","#,
                r#""sample":{},"volume":1.0,"pan":0.0,"mute":false}}],"patterns":[{{"id":1,"#,
                r#""name":"Pattern 1","length_bars":1,"notes":[{{"channel":1,"tick":0,"length":0,"#,
                r#""velocity":1.0}}]}}]}}"#
            ),
            serde_json::to_string(&sample).expect("a path")
        );
        let project = dir.join(format!("{name}.json"));
        fs::write(&project, document).expect("written");
        let out = dir.join(format!("{name}.wav"));
        let output = render(&project, &out, &[], &dir, &data);
        assert!(
            read_stereo(&out).iter().all(|frame| *frame == [0.0; 2]),
            "{name} played the file outside"
        );
        assert!(!output.stderr.is_empty(), "{name}: no warning");
    }
}

/// Runs `dawsome render <project> <out> <extra>` with the data folder at `data` and the
/// installed instrument sets at `installed` (`DAWSOME_SHIPPED_INSTRUMENTS`), every other
/// per-user folder under `home`.
fn render_installed(
    project: &Path,
    out: &Path,
    extra: &[&str],
    home: &Path,
    data: &Path,
    installed: &Path,
) {
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .env("DAWSOME_DATA_HOME", data)
        .env("DAWSOME_SHIPPED_INSTRUMENTS", installed)
        .env("DAWSOME_CACHE_HOME", home.join("cache"))
        .env("DAWSOME_CONFIG_HOME", home.join("config"))
        .env("DAWSOME_STATE_HOME", home.join("state"))
        .env("XDG_DATA_HOME", home.join("xdg-data"))
        .env("XDG_CACHE_HOME", home.join("xdg-cache"))
        .arg("render")
        .arg(project)
        .arg(out)
        .args(extra)
        .output()
        .expect("run dawsome");
    assert!(
        output.status.success(),
        "dawsome render {} failed: {}",
        project.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The test set `set` copied as the set folder `piano` into `root`, its main file renamed
/// piano.sfz, as `dawsome:instruments/piano/piano.sfz` names it.
fn piano_from(set: &str, root: &Path) {
    let folder = root.join("piano");
    copy_dir(&fixtures().join("instruments").join(set), &folder);
    let main = expected()["sets"][set]["sfz"]
        .as_str()
        .expect("its main file")
        .to_owned();
    fs::rename(folder.join(main), folder.join("piano.sfz")).expect("renames");
}

/// `fixture` (a p11 case project) with its set named `dawsome:instruments/piano/piano.sfz`.
fn as_piano(fixture: &str, set: &str, dir: &Path) -> PathBuf {
    let text = fs::read_to_string(fixtures().join(fixture)).expect("fixture");
    let path = format!("\"instruments/{set}/{set}.sfz\"");
    assert!(text.contains(&path), "{fixture} names {path}");
    let project = dir.join(fixture);
    fs::write(
        &project,
        text.replace(&path, "\"dawsome:instruments/piano/piano.sfz\""),
    )
    .expect("written");
    project
}

/// Known answers (gen_p11_expected.py) and the two roots of shipped sets: a project naming
/// `dawsome:instruments/piano/piano.sfz` renders the same bytes, the layers' known answer, with
/// the set in the user's data folder and with it only among the installed sets; and a set of
/// the user's of the same folder name (rr's impulses) shadows the installed one whole, so the
/// rr notes give rr's known answer, not the layers'.
#[test]
fn a_set_plays_the_same_from_the_users_folder_and_the_installed_sets_and_the_users_wins() {
    let dir = temp_dir("two_roots");
    let (empty, installed) = (dir.join("empty"), dir.join("installed"));
    let user = dir.join("user");
    fs::create_dir_all(&empty).expect("dir");
    piano_from("layers", &installed);
    piano_from("layers", &user.join("dawsome/instruments"));
    let layers = as_piano("p11_layers.json", "layers", &dir);
    let (from_user, from_installed) = (dir.join("user.wav"), dir.join("installed.wav"));
    render_installed(&layers, &from_user, &[], &dir, &user, &empty);
    render_installed(&layers, &from_installed, &[], &dir, &empty, &installed);
    let want = expected();
    assert_render(
        "installed",
        &read_stereo(&from_installed),
        &want["renders"]["layers"],
    );
    assert!(
        fs::read(&from_user).expect("rendered") == fs::read(&from_installed).expect("rendered"),
        "the user's folder and the installed sets differ"
    );
    let shadow = dir.join("shadow");
    piano_from("rr", &shadow.join("dawsome/instruments"));
    let rr = as_piano("p11_rr.json", "rr", &dir);
    let shadowed = dir.join("shadowed.wav");
    render_installed(&rr, &shadowed, &[], &dir, &shadow, &installed);
    assert_render("shadowed", &read_stereo(&shadowed), &want["renders"]["rr"]);
}

/// Known answers (gen_p12_expected.py) for the electronic instrument sets the program ships,
/// named by their references and read with assets/instruments as the installed sets: each
/// render is the frames the generator states (their SHA-256), so a note stops with its release
/// after note-off, silent from where the release ends, and a key away from the root plays the
/// sound pitched; the probes say where a difference first shows.
#[test]
fn p12_the_shipped_electronic_sets_render_their_known_answers() {
    use sha2::{Digest, Sha256};
    let text = fs::read_to_string(fixtures().join("p12_expected.json")).expect("p12_expected");
    let want: Value = serde_json::from_str(&text).expect("parses");
    let dir = temp_dir("electronic");
    let installed = repo_root().join("assets/instruments");
    let empty = dir.join("data");
    let renders = want["renders"].as_object().expect("renders");
    assert_eq!(renders.len(), 3);
    for (case, answer) in renders {
        let project = fixtures().join(answer["fixture"].as_str().expect("fixture"));
        let out = dir.join(format!("{case}.wav"));
        let bars = answer["bars"].to_string();
        render_installed(&project, &out, &["--bars", &bars], &dir, &empty, &installed);
        let frames = read_stereo(&out);
        assert_eq!(
            frames.len() as u64,
            answer["frames"].as_u64().expect("frames"),
            "{case}"
        );
        for note in answer["notes"].as_array().expect("notes") {
            for probe in note["probes"].as_array().expect("probes") {
                let at = probe[0].as_u64().expect("a frame") as usize;
                let side = |index: usize| probe[index].as_f64().expect("a value") as f32;
                assert_eq!(
                    frames[at].map(f32::to_bits),
                    [side(1), side(2)].map(f32::to_bits),
                    "{case}, frame {at}"
                );
            }
        }
        let bytes: Vec<u8> = frames
            .iter()
            .flat_map(|frame| frame.iter().flat_map(|value| value.to_le_bytes()))
            .collect();
        let hex: String = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(hex, answer["sha256"].as_str().expect("sha256"), "{case}");
    }
}
