//! `dawsome-app --play-to` plays a project through the app's engine path (session, compiler,
//! rings, player) into a WAV file. It must equal `dawsome render` of the same project byte for
//! byte, which proves the app drives the engine exactly as the command line does.
//!
//! The command line is another package's binary, so Cargo defines no `CARGO_BIN_EXE_dawsome`
//! here; it is found next to this package's binary, where a workspace build puts it.
//! `cargo test --workspace` (scripts/check.sh) builds both before running any test.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

fn app_binary() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_dawsome-app"))
}

fn cli_binary() -> PathBuf {
    let path = app_binary().with_file_name(format!("dawsome{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.is_file(),
        "{} is missing: run the workspace tests (scripts/check.sh) so the command line is built",
        path.display()
    );
    path
}

/// A fresh directory for one test's files.
fn out_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(test);
    // A leftover from an earlier run may or may not exist; either way it is replaced.
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn run(command: &mut Command) {
    let output = command.output().expect("binary starts");
    assert!(
        output.status.success(),
        "{command:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Renders `bars` bars of `fixture` with both binaries; returns the app's file once the two are
/// identical.
fn app_matches_cli(fixture: &str, bars: u32) -> PathBuf {
    app_matches_cli_with(fixture, bars, &[])
}

/// As [`app_matches_cli`], with `extra` arguments given to both binaries.
fn app_matches_cli_with(fixture: &str, bars: u32, extra: &[&str]) -> PathBuf {
    let dir = out_dir(&format!(
        "{}{}",
        fixture.trim_end_matches(".json"),
        extra.concat()
    ));
    let project = Path::new(FIXTURES).join(fixture);
    let (app_wav, cli_wav) = (dir.join("app.wav"), dir.join("cli.wav"));
    let bars = bars.to_string();
    run(Command::new(app_binary())
        .arg("--load")
        .arg(&project)
        .arg("--play-to")
        .arg(&app_wav)
        .args(["--bars", &bars])
        .args(extra));
    run(Command::new(cli_binary())
        .arg("render")
        .arg(&project)
        .arg(&cli_wav)
        .args(["--bars", &bars])
        .args(extra));
    let (app, cli) = (
        fs::read(&app_wav).expect("app wav"),
        fs::read(&cli_wav).expect("cli wav"),
    );
    assert!(app == cli, "{fixture}: app and command line renders differ");
    app_wav
}

/// Frames in a WAV file, and the frames whose left sample is not silent.
fn frames_and_impulses(path: &Path) -> (u32, Vec<usize>) {
    let mut reader = hound::WavReader::open(path).expect("readable wav");
    let frames = reader.duration();
    let left: Vec<f32> = reader
        .samples::<f32>()
        .step_by(2)
        .collect::<Result<_, _>>()
        .expect("float samples");
    let impulses = left
        .iter()
        .enumerate()
        .filter(|(_, &sample)| sample != 0.0)
        .map(|(frame, _)| frame)
        .collect();
    (frames, impulses)
}

// F1: 44.1 kHz, 120 BPM, steps 0, 4, 8, 12. A bar is 384·60·44100·1000 / (120000·96) = 88200
// frames and a beat 22050.
#[test]
fn f1_plays_to_the_same_bytes_as_the_command_line() {
    let wav = app_matches_cli("f1_four_on_floor.json", 1);
    assert_eq!(
        frames_and_impulses(&wav),
        (88_200, vec![0, 22_050, 44_100, 66_150])
    );
}

// F2: 48 kHz, 140 BPM. A bar is floor(384·60·48000·1000 / (140000·96)) = 82285 frames.
#[test]
fn f2_plays_to_the_same_bytes_as_the_command_line() {
    let wav = app_matches_cli("f2_tempo_140_48k.json", 1);
    assert_eq!(frames_and_impulses(&wav).0, 82_285);
}

#[test]
fn play_to_refuses_to_overwrite_its_project() {
    let dir = out_dir("play_to_refuses_to_overwrite_its_project");
    let project = dir.join("f1.json");
    fs::copy(Path::new(FIXTURES).join("f1_four_on_floor.json"), &project).expect("copy");
    fs::copy(
        Path::new(FIXTURES).join("impulse.wav"),
        dir.join("impulse.wav"),
    )
    .expect("copy");
    let before = fs::read(&project).expect("project");
    let output = Command::new(app_binary())
        .arg("--load")
        .arg(&project)
        .arg("--play-to")
        .arg(&project)
        .output()
        .expect("binary starts");
    assert!(!output.status.success());
    assert_eq!(fs::read(&project).expect("project"), before);
}

// The piano-roll fixture (chords of keys, note lengths, two channels) plays through the app exactly
// as the command line renders it. Expected frames from tests/fixtures/gen_p3_expected.py: A at 0
// (100/128), B at 22050 (0.5), kicks at 44100 and 66150.
#[test]
fn p3_roll_plays_to_the_same_bytes_as_the_command_line() {
    let wav = app_matches_cli("p3_roll.json", 1);
    assert_eq!(
        frames_and_impulses(&wav),
        (88_200, vec![0, 22_050, 44_100, 66_150])
    );
}

// Song mode plays through the app exactly as the command line renders it. Expected frames from
// tests/fixtures/gen_p4_expected.py: p4_loop repeats bar 2 (the loop 384..768); p4_messy (48 kHz,
// 140 BPM) cuts a clip at tick 287 and wraps from sample 107142 to 28500. Its 44.1 kHz impulse
// loads resampled to the project's rate: a click of two frames on each onset.
#[test]
fn song_mode_plays_to_the_same_bytes_as_the_command_line() {
    let wav = app_matches_cli_with("p4_loop.json", 3, &["--song"]);
    let onsets = vec![0, 22_050, 44_100, 66_150, 99_225, 121_275, 187_425, 209_475];
    assert_eq!(frames_and_impulses(&wav), (264_600, onsets));
    let wav = app_matches_cli_with("p4_messy.json", 2, &["--song"]);
    let onsets = vec![
        7928, 7929, 28_500, 28_501, 49_071, 49_072, 96_214, 96_215, 107_142, 107_143, 127_713,
        127_714,
    ];
    assert_eq!(frames_and_impulses(&wav), (164_571, onsets));
}

// The mixer plays through the app exactly as the command line renders it. Expected frames from
// tests/fixtures/gen_p5_expected.py: p5_gain's five clicks through the fader law and the post-fader
// sends; p5_delay_sync's echoes every 16537 frames (3 steps).
#[test]
fn the_mixer_plays_to_the_same_bytes_as_the_command_line() {
    let wav = app_matches_cli("p5_gain.json", 1);
    assert_eq!(
        frames_and_impulses(&wav),
        (88_200, vec![0, 11_025, 22_050, 44_100, 66_150])
    );
    let wav = app_matches_cli("p5_delay_sync.json", 1);
    assert_eq!(
        frames_and_impulses(&wav),
        (88_200, vec![16_537, 33_074, 49_611, 66_148, 82_685])
    );
}

// The app's export path writes each bit depth exactly as the command line does.
#[test]
fn every_bit_depth_plays_to_the_same_bytes_as_the_command_line() {
    for bits in ["16", "24", "32f"] {
        app_matches_cli_with("f1_four_on_floor.json", 1, &["--bits", bits]);
    }
}

#[path = "../../../tests/support/test_plugins.rs"]
mod test_plugins;

// gen_p7_expected.py's known answer 6: the app plays a plugin project exactly as the command line
// does, the test sine on frame 6000 at its state's gain of 0.5. Both find the test plugins through
// a catalogue of their own, never the user's.
#[test]
fn a_plugin_project_plays_to_the_same_bytes_as_the_command_line() {
    let dir = out_dir("p7_sine_play_to");
    let project = Path::new(FIXTURES).join("p7_sine.json");
    let plugins = test_plugins::test_plugins();
    let (app_wav, cli_wav) = (dir.join("app.wav"), dir.join("cli.wav"));
    let with_plugins = |command: &mut Command, cache: &str| {
        command
            .arg("--plugin-path")
            .arg(&plugins)
            .arg("--cache")
            .arg(dir.join(cache))
            .args(["--bars", "1"]);
    };
    let mut app = Command::new(app_binary());
    app.arg("--load")
        .arg(&project)
        .arg("--play-to")
        .arg(&app_wav);
    with_plugins(&mut app, "app.json");
    run(&mut app);
    let mut cli = Command::new(cli_binary());
    cli.arg("render").arg(&project).arg(&cli_wav);
    with_plugins(&mut cli, "cli.json");
    run(&mut cli);
    let app_bytes = fs::read(&app_wav).expect("app wav");
    assert!(
        app_bytes == fs::read(&cli_wav).expect("cli wav"),
        "renders differ"
    );
    let mut reader = hound::WavReader::open(&app_wav).expect("reads");
    let samples: Vec<f32> = reader
        .samples::<f32>()
        .collect::<Result<_, _>>()
        .expect("decodes");
    assert_eq!(samples[2 * 5999], 0.0);
    assert!((samples[2 * 6000] - 0.5).abs() < 1e-6, "the sine sounds");
}

// A plugin that fails part way through a --play-to (the fail variant, from its fifth block on)
// is named once on stderr, as the command line names it.
#[test]
fn a_plugin_failing_mid_play_to_is_named_once() {
    let dir = out_dir("p7_fail_play_to");
    let project = dir.join("p7_fail.json");
    let text = fs::read_to_string(Path::new(FIXTURES).join("p7_sine.json"))
        .expect("reads")
        .replace(
            r#""id":"clap:org.dawsome.test.sine","name":"Dawsome Test Sine""#,
            r#""id":"clap:org.dawsome.test.fail","name":"Dawsome Test Fail""#,
        );
    fs::write(&project, text).expect("writes");
    let output = Command::new(app_binary())
        .arg("--load")
        .arg(&project)
        .arg("--play-to")
        .arg(dir.join("out.wav"))
        .arg("--plugin-path")
        .arg(test_plugins::test_plugin_variants())
        .arg("--cache")
        .arg(dir.join("catalogue.json"))
        .args(["--bars", "20"])
        .output()
        .expect("binary starts");
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{}: {message}", output.status);
    let named =
        "dawsome-app: warning: Plugin failed: Dawsome Test Fail: it failed while processing\n";
    assert_eq!(message.matches(named).count(), 1, "{message}");
}

// Audio clips and automation play through the app exactly as the command line renders them
// (p8_trim: trimmed, gained and muted clips over a song-end wrap; p8_auto_curves: hold, smooth and
// exponential segments).
#[test]
fn audio_clips_and_automation_play_to_the_same_bytes_as_the_command_line() {
    app_matches_cli_with("p8_trim.json", 1, &["--song"]);
    app_matches_cli_with("p8_auto_curves.json", 3, &["--song"]);
}

/// The audio-clip and automation known answers tests/fixtures/gen_p8_expected.py wrote.
fn p8_expected() -> serde_json::Value {
    let text =
        fs::read_to_string(Path::new(FIXTURES).join("p8_expected.json")).expect("p8_expected.json");
    serde_json::from_str(&text).expect("parses")
}

fn json_usize(value: &serde_json::Value) -> usize {
    value
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .expect("a frame count")
}

/// The interleaved float samples of a WAV file.
fn samples_of(path: &Path) -> Vec<f32> {
    hound::WavReader::open(path)
        .expect("readable wav")
        .samples::<f32>()
        .collect::<Result<_, _>>()
        .expect("float samples")
}

// gen_p8_expected.py's known answer (8), the master loopback through the offline path: one bar of
// count-in bursts (not recorded), then the song, which is recorded into samples/rec-0001.wav, the
// bytes `dawsome render --song --bars 2` writes; the saved project holds exactly one new audio
// channel and one new clip on the first free track.
#[test]
fn recording_the_master_through_the_offline_path_writes_the_song_and_adds_its_clip() {
    let expected = &p8_expected()["rec"];
    let dir = out_dir("p8_rec_master_loopback");
    for file in ["p8_rec.json", "impulse.wav"] {
        fs::copy(Path::new(FIXTURES).join(file), dir.join(file)).expect("copy fixture");
    }
    let project = dir.join("p8_rec.json");
    let (wav, saved, cli) = (dir.join("a.wav"), dir.join("out.json"), dir.join("cli.wav"));
    run(Command::new(app_binary())
        .arg("--load")
        .arg(&project)
        .args(["--song", "--bars", "2", "--record", "master", "--play-to"])
        .arg(&wav)
        .arg("--save-to")
        .arg(&saved));
    run(Command::new(cli_binary())
        .arg("render")
        .arg(&project)
        .arg(&cli)
        .args(["--song", "--bars", "2"]));

    let played = samples_of(&wav);
    assert_eq!(
        played.len(),
        2 * json_usize(&expected["frames"]),
        "a.wav frames"
    );
    let count_in = json_usize(&expected["count_in"]);
    let mut clicks = vec![0.0f32; count_in];
    for burst in expected["bursts"].as_array().expect("bursts") {
        let at = json_usize(&burst[0]);
        for (k, value) in burst[1].as_array().expect("burst").iter().enumerate() {
            clicks[at + k] = value.as_f64().expect("a number") as f32;
        }
    }
    for (frame, &want) in clicks.iter().enumerate() {
        assert_eq!(
            [played[2 * frame], played[2 * frame + 1]],
            [want; 2],
            "count-in frame {frame}"
        );
    }
    let song = samples_of(&cli);
    assert!(
        played[2 * count_in..] == song[..],
        "the song after the count-in is the render"
    );
    let impulses: Vec<usize> = (0..song.len() / 2)
        .filter(|&frame| song[2 * frame] != 0.0)
        .map(|frame| frame + count_in)
        .collect();
    let want: Vec<usize> = expected["impulses"]
        .as_array()
        .expect("impulses")
        .iter()
        .map(json_usize)
        .collect();
    assert_eq!(impulses, want);

    let recorded = dir.join("samples/rec-0001.wav");
    assert!(
        fs::read(&recorded).expect("the recording") == fs::read(&cli).expect("the render"),
        "the recording is the render's bytes"
    );
    let text = fs::read_to_string(&saved).expect("the saved project");
    let before = fs::read_to_string(&project).expect("the fixture");
    for part in [&expected["channel"], &expected["clip"]] {
        let part = part.as_str().expect("JSON text");
        assert!(text.contains(part), "{part} missing from {text}");
        assert!(!before.contains(part));
    }
    let doc: serde_json::Value = serde_json::from_str(&text).expect("parses");
    assert_eq!(doc["channels"].as_array().map(Vec::len), Some(2));
    assert_eq!(doc["playlist"]["clips"].as_array().map(Vec::len), Some(2));
}

// The master loopback through the offline path with a plugin, whose render runs on a thread of
// its own: p7_sine as a song (one clip of its pattern, 60 bars: two minutes at 48 kHz) recorded
// after the count-in is the bytes `dawsome render --song --bars 60` writes. The record tap holds
// 4 s, so it must be drained as the render goes, not on the main thread's timer, which the render
// thread outruns.
#[test]
fn recording_a_plugin_project_offline_drops_no_frame() {
    let dir = out_dir("p7_sine_rec_master");
    let project = dir.join("p7_song.json");
    let text = fs::read_to_string(Path::new(FIXTURES).join("p7_sine.json")).expect("reads");
    let mut doc: serde_json::Value = serde_json::from_str(&text).expect("parses");
    doc["playlist"] = serde_json::json!({
        "next_track_id": 2,
        "next_clip_id": 2,
        "tracks": [{"id": 1, "name": "Track 1", "mute": false}],
        "clips": [{"id": 1, "track": 1, "start": 0, "length": 60 * 384, "kind": "pattern", "pattern": 1}],
    });
    fs::write(&project, doc.to_string()).expect("writes");
    let plugins = test_plugins::test_plugins();
    let (wav, cli) = (dir.join("a.wav"), dir.join("cli.wav"));
    let mut app = Command::new(app_binary());
    app.arg("--load")
        .arg(&project)
        .arg("--play-to")
        .arg(&wav)
        .args(["--song", "--bars", "60", "--record", "master"])
        .arg("--plugin-path")
        .arg(&plugins)
        .arg("--cache")
        .arg(dir.join("app.json"));
    run(&mut app);
    let mut render = Command::new(cli_binary());
    render
        .arg("render")
        .arg(&project)
        .arg(&cli)
        .args(["--song", "--bars", "60"])
        .arg("--plugin-path")
        .arg(&plugins)
        .arg("--cache")
        .arg(dir.join("cli.json"));
    run(&mut render);
    let recorded = fs::read(dir.join("samples/rec-0001.wav")).expect("the recording");
    let rendered = fs::read(&cli).expect("the render");
    assert_eq!(recorded.len(), rendered.len(), "the recording's length");
    let first = recorded.iter().zip(&rendered).position(|(a, b)| a != b);
    assert_eq!(first, None, "first byte that differs");
    assert!(
        samples_of(&cli).iter().any(|&v| v != 0.0),
        "the render holds the sine"
    );
}

// ---------------------------------------------------------------------------------------------
// MIDI events through the headless door, and stretched clips and the ported effects through the
// app's offline path. Expected values: tests/fixtures/p9_expected.json
// (tests/fixtures/gen_p9_expected.py).
// ---------------------------------------------------------------------------------------------

fn p9_expected() -> serde_json::Value {
    let text =
        fs::read_to_string(Path::new(FIXTURES).join("p9_expected.json")).expect("p9_expected.json");
    serde_json::from_str(&text).expect("parses")
}

fn json_f32(value: &serde_json::Value) -> f32 {
    // The generator writes each f32 as the shortest decimal that names it, so `as f32` is exact.
    value.as_f64().expect("a number") as f32
}

/// Plays `fixture` with `--play-to` and `extra`, saving the document after it into the test's
/// directory; returns the render's interleaved samples and the saved document.
fn play_midi(test: &str, fixture: &str, extra: &[&str]) -> (Vec<f32>, serde_json::Value) {
    let dir = out_dir(test);
    for file in [fixture, "impulse.wav"] {
        fs::copy(Path::new(FIXTURES).join(file), dir.join(file)).expect("copy fixture");
    }
    let (wav, saved) = (dir.join("out.wav"), dir.join("saved.json"));
    run(Command::new(app_binary())
        .arg("--load")
        .arg(dir.join(fixture))
        .arg("--play-to")
        .arg(&wav)
        .arg("--save-to")
        .arg(&saved)
        .args(extra));
    let text = fs::read_to_string(&saved).expect("saved document");
    (
        samples_of(&wav),
        serde_json::from_str(&text).expect("parses"),
    )
}

// (3) Note recording from MIDI events in pattern mode (--record notes, no count-in): each note
// is heard live where it is played and, once finished, applied to the pattern and heard from
// the next call on; quantize moves only the start, onto the nearest step, wrapping at the
// pattern's end. Every other frame is silent.
#[test]
fn notes_from_midi_events_are_heard_recorded_and_quantized() {
    let expected = p9_expected();
    let events = Path::new(FIXTURES).join("p9_record_events.json");
    let events = events.to_str().expect("utf-8 path");
    for (quantize, key) in [("on", "record_on"), ("off", "record_off")] {
        let want = &expected[key];
        let (samples, saved) = play_midi(
            &format!("p9_record_{quantize}"),
            "p9_record.json",
            &[
                "--bars",
                "2",
                "--midi-events",
                events,
                "--record",
                "notes",
                "--count-in",
                "0",
                "--quantize",
                quantize,
            ],
        );
        let frames = json_usize(&want["frames"]);
        assert_eq!(samples.len(), 2 * frames, "quantize {quantize}");
        let mut sounding = vec![0.0f32; frames];
        for pair in want["nonzero"].as_array().expect("nonzero") {
            sounding[json_usize(&pair[0])] = json_f32(&pair[1]);
        }
        for (frame, pair) in samples.chunks_exact(2).enumerate() {
            assert_eq!(
                [pair[0], pair[1]],
                [sounding[frame]; 2],
                "quantize {quantize}, frame {frame}"
            );
        }
        let notes = saved["patterns"][0]["notes"]
            .as_array()
            .expect("notes")
            .clone();
        let want_notes = want["notes"].as_array().expect("notes");
        assert_eq!(
            notes.len(),
            want_notes.len(),
            "quantize {quantize}: {notes:?}"
        );
        for (got, want) in notes.iter().zip(want_notes) {
            for field in ["channel", "tick", "length", "key"] {
                assert_eq!(got.get(field), want.get(field), "{field}: {got}");
            }
            assert_eq!(
                json_f32(&got["velocity"]),
                json_f32(&want["velocity"]),
                "{got}"
            );
        }
    }
}

// (4) Controller links: cc 7 = 100 on any channel moves insert 1's fader to 100/127; cc 10 on
// MIDI channel 2 is not channel 1's link; cc 10 = 127 then 64 on channel 1 leaves the pan at
// −1 + 2·64/127. The saved document holds the last values.
#[test]
fn controller_moves_from_midi_events_set_the_linked_controls() {
    let want = &p9_expected()["cc"];
    let events = Path::new(FIXTURES).join("p9_cc_events.json");
    let (_, saved) = play_midi(
        "p9_cc",
        "p9_cc.json",
        &[
            "--bars",
            "1",
            "--midi-events",
            events.to_str().expect("utf-8 path"),
        ],
    );
    let fader = &saved["mixer"]["inserts"][1]["fader"];
    assert_eq!(json_f32(fader), json_f32(&want["fader"]));
    let pan = &saved["channels"][0]["pan"];
    assert_eq!(json_f32(pan), json_f32(&want["pan"]));
}

/// The variable this test moves the per-user cache folder with: the platform's own on Linux,
/// which must keep working, and Dawsome's elsewhere.
const CACHE_HOME: &str = if cfg!(target_os = "linux") {
    "XDG_CACHE_HOME"
} else {
    "DAWSOME_CACHE_HOME"
};

// From ratio 2 on, each new time-stretch of a clip comes out a little different, so a render reads
// the one cached file. With no --cache-dir, `--play-to` keeps it in the cache folder `dawsome
// render` keeps it in (dawsome/stretch in the per-user cache folder, here inside the test's
// folder), so at ratio 3 the two renders are the same bytes, with the cache folder given or not.
#[test]
fn at_ratio_3_play_to_and_render_read_one_cached_stretch() {
    let dir = out_dir("p9_ratio3");
    fs::copy(
        Path::new(FIXTURES).join("p9_tone440.wav"),
        dir.join("p9_tone440.wav"),
    )
    .expect("copy fixture");
    // p9_stretch's 44100-frame tone stretched to 132300 frames.
    let text = fs::read_to_string(Path::new(FIXTURES).join("p9_stretch.json"))
        .expect("fixture")
        .replace(r#""frames":66150"#, r#""frames":132300"#);
    assert!(
        text.contains(r#""frames":132300"#),
        "the stretch is ratio 3"
    );
    let project = dir.join("p9_ratio3.json");
    fs::write(&project, text).expect("project written");
    // Absolute, as the variable must be: a test directory named from another platform's build
    // (a cross-built test run under a compatibility layer) is only rooted, not absolute.
    let user_cache = std::path::absolute(dir.join("xdg-cache")).expect("an absolute path");
    let given = dir.join("given-cache");
    let given = given.to_str().expect("utf-8 path");
    for (name, extra) in [
        ("default", Vec::new()),
        ("given", vec!["--cache-dir", given]),
    ] {
        let (app_wav, cli_wav) = (
            dir.join(format!("{name}-app.wav")),
            dir.join(format!("{name}-cli.wav")),
        );
        run(Command::new(app_binary())
            .env_remove("DAWSOME_CACHE_HOME")
            .env(CACHE_HOME, &user_cache)
            .arg("--load")
            .arg(&project)
            .arg("--play-to")
            .arg(&app_wav)
            .args(["--song", "--bars", "1"])
            .args(&extra));
        run(Command::new(cli_binary())
            .env_remove("DAWSOME_CACHE_HOME")
            .env(CACHE_HOME, &user_cache)
            .arg("render")
            .arg(&project)
            .arg(&cli_wav)
            .args(["--song", "--bars", "1"])
            .args(&extra));
        assert!(
            fs::read(&app_wav).expect("app wav") == fs::read(&cli_wav).expect("cli wav"),
            "{name} cache folder: app and command line renders differ"
        );
    }
    let cached = fs::read_dir(user_cache.join("dawsome").join("stretch"))
        .expect("the default cache folder")
        .count();
    assert_eq!(cached, 1, "one stretched file in the default cache folder");
}

// A stretched clip (resample mode) and the ported saturator play to the bytes the command line
// renders.
#[test]
fn stretched_clips_and_ported_effects_play_to_the_same_bytes_as_the_command_line() {
    let dir = out_dir("p9_cache");
    let cache = dir.to_str().expect("utf-8 path");
    for (fixture, song) in [("p9_resample.json", true), ("p9_density_cli.json", false)] {
        let out = out_dir(&format!("p9_matches_{}", fixture.trim_end_matches(".json")));
        let (app_wav, cli_wav) = (out.join("app.wav"), out.join("cli.wav"));
        let mut extra = vec!["--bars", "1", "--cache-dir", cache];
        if song {
            extra.push("--song");
        }
        run(Command::new(app_binary())
            .arg("--load")
            .arg(Path::new(FIXTURES).join(fixture))
            .arg("--play-to")
            .arg(&app_wav)
            .args(&extra));
        run(Command::new(cli_binary())
            .arg("render")
            .arg(Path::new(FIXTURES).join(fixture))
            .arg(&cli_wav)
            .args(&extra));
        assert!(
            fs::read(&app_wav).expect("app wav") == fs::read(&cli_wav).expect("cli wav"),
            "{fixture}: app and command line renders differ"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// The Library's MIDI devices: known answers from tests/fixtures/p10_expected.json
// (tests/fixtures/gen_p10_expected.py).
// ---------------------------------------------------------------------------------------------

fn p10_expected() -> serde_json::Value {
    let text = fs::read_to_string(Path::new(FIXTURES).join("p10_expected.json"))
        .expect("p10_expected.json");
    serde_json::from_str(&text).expect("parses")
}

// A MIDI device linked to a channel plays it whatever is selected. In p10_link "kick" is the
// first channel, so the selected one, and "keys" plays the notes of the linked port. One note-on
// from that port before call 0 renders exactly as `dawsome render` renders the same note on
// "keys" at tick 0; from any other port, exactly as it renders the note on "kick". The two
// expected renders differ, so each comparison can fail.
#[test]
fn a_linked_device_plays_its_channel_and_any_other_port_the_selected_one() {
    let want = &p10_expected()["link"];
    let dir = out_dir("p10_link");
    let fixture = |name: &str| Path::new(FIXTURES).join(name);
    let render = |project: &str, out: &Path| {
        run(Command::new(cli_binary())
            .arg("render")
            .arg(fixture(project))
            .arg(out)
            .args(["--bars", "1"]));
        fs::read(out).expect("cli wav")
    };
    let play = |events: &str, out: &Path| {
        run(Command::new(app_binary())
            .arg("--load")
            .arg(fixture("p10_link.json"))
            .arg("--play-to")
            .arg(out)
            .args(["--bars", "1", "--midi-events"])
            .arg(fixture(events)));
        fs::read(out).expect("app wav")
    };
    let on_keys = render(
        want["linked"].as_str().expect("a file"),
        &dir.join("on_keys.wav"),
    );
    let on_kick = render(
        want["unlinked"].as_str().expect("a file"),
        &dir.join("on_kick.wav"),
    );
    assert!(on_keys != on_kick, "the two channels sound different");
    let frames = hound::WavReader::open(dir.join("on_keys.wav"))
        .expect("readable wav")
        .duration();
    assert_eq!(u64::from(frames), want["frames"].as_u64().expect("frames"));
    assert!(
        play("p10_link_events.json", &dir.join("linked.wav")) == on_keys,
        "a note from the linked port plays keys"
    );
    assert!(
        play("p10_unlinked_events.json", &dir.join("unlinked.wav")) == on_kick,
        "a note from another port plays the selected kick"
    );
}

// While a channel's instrument window is open (`--open-instrument` with
// `--play-to`), the notes of a port linked to no channel, and of a message naming no port,
// play the window's channel, and a linked port's still play its own. In p10_link "kick"
// (channel 1) is selected and "keys" (channel 2) linked: with the window on keys, a note from
// another port renders as the note on keys does (without the window, as on kick: the test
// above), as does one naming no port; with the window on kick, the linked port's note still
// renders as on keys.
#[test]
fn an_open_instrument_window_plays_the_notes_no_link_routes() {
    let want = &p10_expected()["link"];
    let dir = out_dir("p10_window");
    let fixture = |name: &str| Path::new(FIXTURES).join(name);
    let render = |project: &str, out: &Path| {
        run(Command::new(cli_binary())
            .arg("render")
            .arg(fixture(project))
            .arg(out)
            .args(["--bars", "1"]));
        fs::read(out).expect("cli wav")
    };
    let play = |events: &Path, window: u32, out: &Path| {
        run(Command::new(app_binary())
            .arg("--load")
            .arg(fixture("p10_link.json"))
            .arg("--play-to")
            .arg(out)
            .args(["--bars", "1", "--midi-events"])
            .arg(events)
            .arg("--open-instrument")
            .arg(format!("channel:{window}")));
        fs::read(out).expect("app wav")
    };
    let on_keys = render(
        want["linked"].as_str().expect("a file"),
        &dir.join("on_keys.wav"),
    );
    let on_kick = render(
        want["unlinked"].as_str().expect("a file"),
        &dir.join("on_kick.wav"),
    );
    assert!(on_keys != on_kick, "the two channels sound different");
    let portless = dir.join("portless_events.json");
    fs::write(
        &portless,
        r#"{"buffer":512,"events":[{"call":0,"bytes":[144,60,100]}]}"#,
    )
    .expect("writes");
    let unlinked = fixture("p10_unlinked_events.json");
    let linked = fixture("p10_link_events.json");
    assert!(
        play(&unlinked, 2, &dir.join("unlinked_on_keys.wav")) == on_keys,
        "another port's note plays the window's keys"
    );
    assert!(
        play(&portless, 2, &dir.join("portless_on_keys.wav")) == on_keys,
        "a note naming no port plays the window's keys"
    );
    assert!(
        play(&linked, 1, &dir.join("linked_on_keys.wav")) == on_keys,
        "the linked port's note plays keys with the window on kick"
    );
}

// On Linux a port's name ends in the system's client and port numbers, which change when
// another device is found first. A link saved under one numbering still plays its device's notes
// under another: a note-on from the renumbered port before call 0 renders exactly as the same
// note placed on "keys" does (gen_p10_expected.py).
#[test]
fn a_link_saved_under_former_system_numbers_still_plays_its_device() {
    let want = &p10_expected()["link"]["alsa"];
    let dir = out_dir("p10_link_alsa");
    let fixture = |key: &str| Path::new(FIXTURES).join(want[key].as_str().expect("a file"));
    let expected = dir.join("on_keys.wav");
    run(Command::new(cli_binary())
        .arg("render")
        .arg(fixture("renders_as"))
        .arg(&expected)
        .args(["--bars", "1"]));
    let played = dir.join("played.wav");
    run(Command::new(app_binary())
        .arg("--load")
        .arg(fixture("project"))
        .arg("--play-to")
        .arg(&played)
        .args(["--bars", "1", "--midi-events"])
        .arg(fixture("events")));
    assert!(
        fs::read(&played).expect("app wav") == fs::read(&expected).expect("cli wav"),
        "the renumbered port plays keys"
    );
}

// Two devices holding one key, each on its own channel: the linked port's C4 plays "keys", the
// other port's C4 the selected "kick", and each port's note-off ends only its own port's note.
// Recorded through the MIDI door (--record notes, no count-in, no quantize), the take holds both
// notes, each as long as its own port held it, exactly as gen_p10_expected.py works them out.
// Before, the second note-on ended the first and the linked port's note-off ended the other's.
#[test]
fn two_devices_holding_one_key_on_two_channels_both_play_to_their_own_release() {
    let want = &p10_expected()["two_ports"];
    let dir = out_dir("p10_two_ports");
    let (wav, saved) = (dir.join("out.wav"), dir.join("saved.json"));
    let events = Path::new(FIXTURES).join(want["events"].as_str().expect("a file"));
    let bars = want["bars"].as_u64().expect("bars").to_string();
    run(Command::new(app_binary())
        .arg("--load")
        .arg(Path::new(FIXTURES).join("p10_link.json"))
        .arg("--play-to")
        .arg(&wav)
        .arg("--save-to")
        .arg(&saved)
        .args(["--bars", &bars, "--midi-events"])
        .arg(&events)
        .args(["--record", "notes", "--count-in", "0", "--quantize", "off"]));
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&saved).expect("saved document")).expect("parses");
    let notes = saved["patterns"][0]["notes"].as_array().expect("notes");
    let want_notes = want["notes"].as_array().expect("notes");
    assert_eq!(notes.len(), want_notes.len(), "{notes:?}");
    for (got, want) in notes.iter().zip(want_notes) {
        for field in ["channel", "tick", "length", "key"] {
            assert_eq!(got.get(field), want.get(field), "{field}: {got}");
        }
        assert_eq!(
            json_f32(&got["velocity"]),
            json_f32(&want["velocity"]),
            "{got}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Recording v2 through the headless door. Expected values: tests/fixtures/p17_expected.json
// (tests/fixtures/gen_p17_expected.py).
// ---------------------------------------------------------------------------------------------

fn p17_expected() -> serde_json::Value {
    let text = fs::read_to_string(Path::new(FIXTURES).join("p17_expected.json"))
        .expect("p17_expected.json");
    serde_json::from_str(&text).expect("parses")
}

// Known answer (1): a take of notes in pattern mode from MIDI events, two passes over the
// one-bar pattern, quantized to the step. `--dump-notes-at 210` prints the pattern's notes once
// 210 calls have played: the three that ended by then, each written the moment its end was
// heard. At the end the saved document holds those three unchanged, the second pass's note
// added (overdub) and the note held to the end ending at the render's last tick (768): the
// generator's document, byte for byte.
#[test]
fn p17_a_pattern_takes_notes_are_there_mid_take_and_unchanged_at_the_end() {
    let want = &p17_expected()["pattern"];
    let dir = out_dir("p17_pattern");
    let project = want["project"].as_str().expect("a file");
    for file in [project, "impulse.wav"] {
        fs::copy(Path::new(FIXTURES).join(file), dir.join(file)).expect("copy fixture");
    }
    let events = Path::new(FIXTURES).join(want["events"].as_str().expect("a file"));
    let (wav, saved) = (dir.join("out.wav"), dir.join("saved.json"));
    let bars = want["bars"].as_u64().expect("bars").to_string();
    let dump = want["dump_call"].as_u64().expect("a call").to_string();
    let output = Command::new(app_binary())
        .arg("--load")
        .arg(dir.join(project))
        .arg("--play-to")
        .arg(&wav)
        .arg("--save-to")
        .arg(&saved)
        .args(["--bars", &bars, "--midi-events"])
        .arg(&events)
        .args(["--record", "notes", "--count-in", "0", "--quantize", "on"])
        .args(["--dump-notes-at", &dump])
        .output()
        .expect("binary starts");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    let line = stdout
        .lines()
        .find(|line| line.starts_with("{\"call\""))
        .unwrap_or_else(|| panic!("no dump in {stdout:?}"));
    let dumped: serde_json::Value = serde_json::from_str(line).expect("parses");
    assert_eq!(dumped["call"], want["dump_call"]);
    assert_eq!(dumped["notes"], want["dump_notes"], "notes mid-take");
    let document =
        fs::read_to_string(Path::new(FIXTURES).join(want["document"].as_str().expect("a file")))
            .expect("the expected document");
    assert_eq!(
        fs::read_to_string(&saved).expect("saved").trim_end(),
        document.trim_end(),
        "the document at the end"
    );
}

// Found in use: a pattern take's key held past the loop's end. The note is split at the
// pattern's end, one from tick 0 for the rest, no longer than its own start, so a key held a
// pass and a half covers the pattern once; none runs past the pattern's 384 ticks. Quantize
// off, then on (starts to the step). p17_expected.json's "pattern_wrap" (gen_p17_expected.py).
#[test]
fn p17_a_note_held_across_the_loops_end_is_split_at_the_pattern_end() {
    let want = &p17_expected()["pattern_wrap"];
    let events = Path::new(FIXTURES).join(want["events"].as_str().expect("a file"));
    let bars = want["bars"].as_u64().expect("bars").to_string();
    for (quantize, key) in [("off", "notes_off"), ("on", "notes_on")] {
        let dir = out_dir(&format!("p17_wrap_{quantize}"));
        let project = want["project"].as_str().expect("a file");
        for file in [project, "impulse.wav"] {
            fs::copy(Path::new(FIXTURES).join(file), dir.join(file)).expect("copy fixture");
        }
        let (wav, saved) = (dir.join("out.wav"), dir.join("saved.json"));
        run(Command::new(app_binary())
            .arg("--load")
            .arg(dir.join(project))
            .arg("--play-to")
            .arg(&wav)
            .arg("--save-to")
            .arg(&saved)
            .args(["--bars", &bars, "--midi-events"])
            .arg(&events)
            .args([
                "--record",
                "notes",
                "--count-in",
                "0",
                "--quantize",
                quantize,
            ]));
        let saved: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&saved).expect("saved")).expect("parses");
        let notes = &saved["patterns"][0]["notes"];
        assert_eq!(notes, &want[key], "quantize {quantize}");
        for note in notes.as_array().expect("notes") {
            let end =
                note["tick"].as_i64().expect("a tick") + note["length"].as_i64().expect("a length");
            assert!(
                end <= 384,
                "quantize {quantize}: {note} runs past the pattern"
            );
        }
    }
}

// MIDI out's echo door (tests/fixtures/gen_p19_expected.py, "echo"): the MIDI messages of
// p19_echo_events.json, played through --play-to's MIDI path as the window's connected ports
// deliver them, send out exactly the messages the generator computed: a keyboard linked to
// nothing plays the selected Synth's MIDI out, the Synth's own port linked to nothing and a port
// linked to its channel with echo off play quietly (nothing sent), the port linked to the
// channel with echo on sends, and its note sent back while it holds the key is left out. No
// MIDI port is opened.
#[test]
fn notes_from_ports_go_out_as_the_echo_rules_say() {
    let fixtures = Path::new(FIXTURES);
    let expected: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(fixtures.join("p19_expected.json")).expect("p19_expected"),
    )
    .expect("parses");
    let case = &expected["echo"];
    let dir = out_dir("midi_out_echo");
    let log = dir.join("midi_out.json");
    let bars = case["bars"].as_u64().expect("bars").to_string();
    run(Command::new(app_binary())
        .arg("--load")
        .arg(fixtures.join(case["fixture"].as_str().expect("fixture")))
        .arg("--play-to")
        .arg(dir.join("echo.wav"))
        .args(["--bars", &bars, "--midi-events"])
        .arg(fixtures.join(case["events"].as_str().expect("events")))
        .arg("--midi-out-log")
        .arg(&log));
    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&log).expect("the log")).expect("parses");
    assert!(!case["messages"].as_array().expect("messages").is_empty());
    assert_eq!(written["events"], case["messages"]);
    assert_eq!(
        (&written["dropped"], &written["lost_offs"]),
        (&0.into(), &0.into())
    );
}
