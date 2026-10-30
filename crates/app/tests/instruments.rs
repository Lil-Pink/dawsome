//! Instrument channels through the app's own doors: a live note from the headless
//! `--midi-events` door holds a set's loop and releases it at note-off, as
//! tests/fixtures/gen_p11_expected.py states; and a project naming shipped files by their place
//! in one user's data folder saves them as `dawsome:` references, so it renders the same bytes
//! from another user's data folder that holds them; a project naming a set no longer shipped
//! keeps its channel. Every run gets per-user folders of its own.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

fn out_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("instruments")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Runs `dawsome-app` with `args`, its per-user folders under `home` and its data folder at
/// `data`.
fn app(args: &[&std::ffi::OsStr], home: &Path, data: &Path) {
    app_installed(args, home, data, None);
}

/// As [`app`], with `installed` (when given) as the sets installed with the program; returns
/// what the run printed.
fn app_installed(
    args: &[&std::ffi::OsStr],
    home: &Path,
    data: &Path,
    installed: Option<&Path>,
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dawsome-app"));
    if let Some(installed) = installed {
        command.env("DAWSOME_SHIPPED_INSTRUMENTS", installed);
    }
    let output = command
        .env("DAWSOME_DATA_HOME", data)
        .env("DAWSOME_CACHE_HOME", home.join("cache"))
        .env("DAWSOME_CONFIG_HOME", home.join("config"))
        .env("DAWSOME_STATE_HOME", home.join("state"))
        .env("XDG_DATA_HOME", home.join("xdg-data"))
        .env("XDG_CACHE_HOME", home.join("xdg-cache"))
        .env("XDG_CONFIG_HOME", home.join("xdg-config"))
        .env("XDG_STATE_HOME", home.join("xdg-state"))
        .args(args)
        .output()
        .expect("the app starts");
    assert!(
        output.status.success(),
        "dawsome-app {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn read_stereo(path: &Path) -> Vec<[f32; 2]> {
    let mut reader = hound::WavReader::open(path).expect("open rendered wav");
    let values: Vec<f32> = reader
        .samples::<f32>()
        .map(|value| value.expect("decodes"))
        .collect();
    values
        .chunks_exact(2)
        .map(|pair| [pair[0], pair[1]])
        .collect()
}

fn expected() -> serde_json::Value {
    let text = fs::read_to_string(fixture("p11_expected.json")).expect("p11_expected.json");
    serde_json::from_str(&text).expect("parses")
}

/// Asserts `frames` is exactly the expected render: its sounding frames, all else silence.
fn assert_render(name: &str, frames: &[[f32; 2]], want: &serde_json::Value) {
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

/// Known answer: C4 held from MIDI call 0 to call 1 on p11_live's pad (no notes of its own)
/// loops while held and releases over 176 frames after note-off, through the door a device's
/// messages come in by.
#[test]
fn a_live_note_from_the_midi_door_holds_the_loop_and_releases() {
    let dir = out_dir("live");
    let out = dir.join("live.wav");
    app(
        &[
            "--load".as_ref(),
            fixture("p11_live.json").as_os_str(),
            "--play-to".as_ref(),
            out.as_os_str(),
            "--midi-events".as_ref(),
            fixture("p11_live_events.json").as_os_str(),
            "--bars".as_ref(),
            "1".as_ref(),
            "--cache-dir".as_ref(),
            dir.join("stretch").as_os_str(),
        ],
        &dir,
        &dir.join("data"),
    );
    assert_render("live", &read_stereo(&out), &expected()["renders"]["live"]);
}

/// v2 addition, known answer: the Library previewing the pad set (`--library-preview`) plays
/// the notes of the `--midi-events` door that no link routes, as the pad's channel plays them
/// once added (at the volume an add gives a channel): C4 held from call 0 to call 1 renders
/// exactly `renders.live_added` of p11_expected.json (gen_p11_expected.py; `renders.live` at
/// that volume), on a project of one silent sampler that has no set at all. Without
/// the preview the same notes play that silent sampler and the render is silence; and the
/// project the render saves (`--save-to`) holds no channel of the preview.
#[test]
fn a_set_previewed_from_the_library_plays_the_midi_doors_notes() {
    let dir = out_dir("library_preview");
    let base = dir.join("silent.json");
    fs::write(
        &base,
        r#"{"format_version":2,"sample_rate":44100,"tempo_mbpm":120000,"ppq":96,"next_channel_id":2,"channels":[{"id":1,"name":"Sampler","kind":"sampler","sample":"","volume":1.0,"pan":0.0,"mute":false}],"patterns":[{"id":1,"name":"Pattern 1","length_bars":1,"notes":[]}]}"#,
    )
    .expect("writes");
    let render = |out: &Path, preview: bool| {
        let mut args: Vec<&std::ffi::OsStr> = vec![
            "--load".as_ref(),
            base.as_os_str(),
            "--play-to".as_ref(),
            out.as_os_str(),
            "--midi-events".as_ref(),
        ];
        let events = fixture("p11_live_events.json");
        let pad = fixture("instruments/pad/pad.sfz");
        let saved = dir.join("saved.json");
        let cache = dir.join("stretch");
        args.extend([
            events.as_os_str(),
            "--bars".as_ref(),
            "1".as_ref(),
            "--cache-dir".as_ref(),
            cache.as_os_str(),
        ]);
        if preview {
            args.extend([
                "--library-preview".as_ref(),
                pad.as_os_str(),
                "--save-to".as_ref(),
                saved.as_os_str(),
            ]);
        }
        app(&args, &dir, &dir.join("data"));
        read_stereo(out)
    };
    let previewed = render(&dir.join("previewed.wav"), true);
    assert_render(
        "previewed",
        &previewed,
        &expected()["renders"]["live_added"],
    );
    let silent = render(&dir.join("silent.wav"), false);
    assert!(
        silent.iter().all(|frame| *frame == [0.0, 0.0]),
        "without the preview the notes play the silent sampler"
    );
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("saved.json")).expect("saved"))
            .expect("JSON");
    let channels = saved["channels"].as_array().expect("channels");
    assert_eq!(channels.len(), 1, "{channels:?}");
    assert_eq!(channels[0]["name"], "Sampler");
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

/// A data folder holding the layers set and the starter kit's Kick 808.
fn data_folder(root: &Path) -> PathBuf {
    let shipped = root.join("dawsome");
    copy_dir(
        &fixture("instruments/layers"),
        &shipped.join("instruments/layers"),
    );
    fs::create_dir_all(shipped.join("starter-kit")).expect("dir");
    fs::copy(
        Path::new(FIXTURES).join("../../assets/sounds/Kick 808.wav"),
        shipped.join("starter-kit/Kick 808.wav"),
    )
    .expect("copies");
    root.to_path_buf()
}

/// Portability: a project naming the layers set and the kit's Kick by their absolute paths in
/// data folder A (as a drop from the Library did before references existed) is played and
/// saved with A as the data folder: the saved file names both by their `dawsome:` references.
/// Played with data folder B, which holds the same files, it renders the bytes A rendered, and
/// its layers sound as the known answer states.
#[test]
fn a_project_saved_with_one_data_folder_renders_the_same_with_another() {
    let dir = out_dir("portable");
    let (a, b) = (data_folder(&dir.join("a")), data_folder(&dir.join("b")));
    let set = a.join("dawsome/instruments/layers/layers.sfz");
    let kick = a.join("dawsome/starter-kit/Kick 808.wav");
    let layers = fs::read_to_string(fixture("p11_layers.json")).expect("fixture");
    let channel = r#"{"id":1,"name":"Layers","kind":"instrument","instrument":"instruments/layers/layers.sfz","volume":1.0,"pan":0.0,"mute":false}"#;
    assert!(layers.contains(channel));
    let both = format!(
        r#"{},{{"id":2,"name":"Kick","kind":"sampler","sample":{},"volume":1.0,"pan":0.0,"mute":false}}"#,
        channel.replace(
            r#""instruments/layers/layers.sfz""#,
            &serde_json::to_string(&set).expect("a path")
        ),
        serde_json::to_string(&kick).expect("a path"),
    );
    let project = layers
        .replace(channel, &both)
        .replace(r#""next_channel_id":2"#, r#""next_channel_id":3"#);
    // The project's folder holds neither data folder, so nothing is relative to it.
    let song = dir.join("song");
    fs::create_dir_all(&song).expect("dir");
    let original = song.join("absolute.json");
    fs::write(&original, project).expect("written");
    let saved = song.join("saved.json");
    let (with_a, with_b) = (dir.join("a.wav"), dir.join("b.wav"));
    app(
        &[
            "--load".as_ref(),
            original.as_os_str(),
            "--play-to".as_ref(),
            with_a.as_os_str(),
            "--save-to".as_ref(),
            saved.as_os_str(),
        ],
        &dir,
        &a,
    );
    let text = fs::read_to_string(&saved).expect("saved");
    assert!(
        text.contains(r#""instrument":"dawsome:instruments/layers/layers.sfz""#),
        "{text}"
    );
    assert!(
        text.contains(r#""sample":"dawsome:starter-kit/Kick 808.wav""#),
        "{text}"
    );
    app(
        &[
            "--load".as_ref(),
            saved.as_os_str(),
            "--play-to".as_ref(),
            with_b.as_os_str(),
        ],
        &dir,
        &b,
    );
    assert!(
        fs::read(&with_a).expect("A") == fs::read(&with_b).expect("B"),
        "A and B differ"
    );
    // The kick plays from frame 0 and the layers' first impulse is at frame 36: the kick's
    // sound runs under them, so only the layers-only project gives the known answer itself.
    let only_layers = song.join("layers.json");
    fs::write(
        &only_layers,
        layers.replace(
            r#""instruments/layers/layers.sfz""#,
            r#""dawsome:instruments/layers/layers.sfz""#,
        ),
    )
    .expect("written");
    let layers_out = dir.join("layers.wav");
    app(
        &[
            "--load".as_ref(),
            only_layers.as_os_str(),
            "--play-to".as_ref(),
            layers_out.as_os_str(),
        ],
        &dir,
        &b,
    );
    assert_render(
        "layers from B",
        &read_stereo(&layers_out),
        &expected()["renders"]["layers"],
    );
}

/// A project naming a set that is no longer shipped (`ep-c4`, the electric piano made for
/// Dawsome until FreePats' `ep` replaced it on 2026-10-27) opens as a project naming any set
/// that is not there does: it plays its bar (88,200 frames: 384 ticks at 120 BPM and 44.1 kHz)
/// in silence, a warning names the set's main file where the data folder would hold it (the
/// installed sets hold no folder of that name), and the project saved keeps the channel, its
/// name and its reference.
#[test]
fn a_project_naming_a_set_no_longer_shipped_opens_with_its_channel_kept() {
    let dir = out_dir("retired");
    let (data, installed) = (dir.join("data"), dir.join("installed"));
    fs::create_dir_all(&installed).expect("dir");
    let layers = fs::read_to_string(fixture("p11_shipped.json")).expect("reads");
    let named =
        r#""name":"EP","kind":"instrument","instrument":"dawsome:instruments/ep-c4/ep-c4.sfz""#;
    let project = layers.replace(
        r#""name":"Layers","kind":"instrument","instrument":"dawsome:instruments/layers/layers.sfz""#,
        named,
    );
    assert_ne!(project, layers);
    let (original, out, saved) = (
        dir.join("old.json"),
        dir.join("old.wav"),
        dir.join("saved.json"),
    );
    fs::write(&original, project).expect("written");
    let output = app_installed(
        &[
            "--load".as_ref(),
            original.as_os_str(),
            "--play-to".as_ref(),
            out.as_os_str(),
            "--save-to".as_ref(),
            saved.as_os_str(),
        ],
        &dir,
        &data,
        Some(&installed),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    let looked = data.join("dawsome/instruments/ep-c4/ep-c4.sfz");
    let warning = format!(
        "dawsome-app: warning: instrument {}: 1 problem: it cannot be read",
        looked.display()
    );
    assert!(stderr.contains(&warning), "{stderr}");
    let frames = read_stereo(&out);
    assert_eq!(frames.len(), 88_200);
    assert!(frames.iter().all(|frame| *frame == [0.0; 2]));
    let text = fs::read_to_string(&saved).expect("saved");
    assert!(text.contains(named), "{text}");
}
