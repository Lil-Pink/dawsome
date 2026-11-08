//! The piano roll's player through the real `dawsome` binary: `render --pattern P --channel C`
//! (Channel mode) writes the bytes `render --pattern P` writes of the same project with every
//! other channel muted, for a pattern of two channels, a channel on an insert it shares (with an
//! effect), a channel with no notes and an input channel beside a sampler; the frames the
//! generator works out by hand; `--once` one pass, exactly the first pass of the looped render,
//! whose two passes are the same bytes; and played once, a MIDI out sends nothing past the
//! pattern's end.
//! Expected values come from tests/fixtures/gen_roll_player.py, never from the engine.

use std::ffi::OsStr;
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
        .join("roll_player")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn expected() -> Value {
    let text = fs::read_to_string(fixture("rp_expected.json"))
        .expect("rp_expected.json (run gen_roll_player.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("rp_expected.json parses")
}

fn text(value: &Value) -> &str {
    value.as_str().expect("a string")
}

fn number(value: &Value) -> usize {
    usize::try_from(value.as_u64().expect("a number")).expect("fits")
}

/// Runs `dawsome` with `args`, its cache in `dir`.
fn dawsome(dir: &Path, args: &[&OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .env("DAWSOME_CACHE_HOME", dir.join("cache"))
        .args(args)
        .output()
        .expect("run dawsome binary")
}

/// `dawsome render <project> <out> <flags>`, requiring success; returns the WAV's bytes.
fn render(dir: &Path, project: &str, out: &str, flags: &[&str]) -> Vec<u8> {
    let project = fixture(project);
    let out = dir.join(out);
    let mut args: Vec<&OsStr> = vec!["render".as_ref(), project.as_os_str(), out.as_os_str()];
    args.extend(flags.iter().map(OsStr::new));
    let output = dawsome(dir, &args);
    assert!(
        output.status.success(),
        "dawsome {args:?} failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    fs::read(&out).expect("the rendered WAV")
}

/// A rendered WAV's frames, requiring 32-bit float stereo.
fn frames(wav: &[u8]) -> Vec<[f32; 2]> {
    let mut reader = hound::WavReader::new(wav).expect("a WAV");
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

fn bits(frames: &[[f32; 2]]) -> Vec<[u32; 2]> {
    frames.iter().map(|frame| frame.map(f32::to_bits)).collect()
}

/// Each frame of `audio` against the generator's runs ([first frame, frames, value on both
/// sides]); every other frame silent.
fn assert_runs(case: &str, audio: &[[f32; 2]], runs: &Value) {
    let mut want = vec![[0.0f32; 2]; audio.len()];
    for run in runs.as_array().expect("runs") {
        let (first, count) = (number(&run[0]), number(&run[1]));
        let value = run[2].as_f64().expect("a value") as f32;
        for frame in &mut want[first..first + count] {
            *frame = [value; 2];
        }
    }
    for (at, (got, want)) in audio.iter().zip(&want).enumerate() {
        assert_eq!(
            got.map(f32::to_bits),
            want.map(f32::to_bits),
            "{case}, frame {at}"
        );
    }
}

// Known answers 3.1: each project's channel in Channel mode is the byte-identical WAV of the
// pattern played with every other channel muted (the reference project), its length the
// pattern's; the keys, the empty channel and both input cases frame for frame as worked out by
// hand (an input channel beside the keys is left out though it has no notes; the roll on the
// input channel hears its input); where the plain pattern sounds a drum alone, the keys'
// render is silent; the shared insert's channel is heard through the insert's effect, and the
// pad on that insert is not.
#[test]
fn a_channel_renders_as_its_pattern_with_the_other_channels_muted() {
    let dir = temp_dir("pairs");
    let want = expected();
    for (case, answer) in want["renders"].as_object().expect("cases") {
        let (pattern, channel) = (text(&answer["pattern"]), text(&answer["channel"]));
        let bars = answer["bars"].to_string();
        // The input channels' cases play the generator's feed file for every input channel.
        let feed = answer
            .get("feed")
            .map(|feed| fixture(text(feed)).to_string_lossy().into_owned());
        let feeds: Vec<&str> = feed
            .iter()
            .flat_map(|feed| ["--input-feed", feed])
            .collect();
        let pattern_flags = [&["--pattern", pattern, "--bars", &bars][..], &feeds].concat();
        let channel_flags = [&["--channel", channel][..], &pattern_flags].concat();
        let alone = render(
            &dir,
            text(&answer["project"]),
            &format!("{case}_channel.wav"),
            &channel_flags,
        );
        let muted = render(
            &dir,
            text(&answer["reference"]),
            &format!("{case}_muted.wav"),
            &pattern_flags,
        );
        assert!(alone == muted, "{case}: the two renders differ");
        let audio = frames(&alone);
        assert_eq!(audio.len(), number(&answer["frames"]), "{case}: frames");
        if let Some(runs) = answer.get("runs") {
            assert_runs(case, &audio, runs);
        }
        let plain = frames(&render(
            &dir,
            text(&answer["project"]),
            &format!("{case}_plain.wav"),
            &pattern_flags,
        ));
        for silenced in answer["silenced"].as_array().into_iter().flatten() {
            let (at, value) = (number(&silenced[0]), silenced[1].as_f64().expect("a value"));
            assert_eq!(
                plain[at], [value as f32; 2],
                "{case}: the drum plays at {at}"
            );
            assert_eq!(audio[at], [0.0; 2], "{case}: no drum at {at}");
        }
        if case == "shared_insert" {
            assert!(
                audio.iter().any(|frame| *frame != [0.0; 2]),
                "the lead is heard through the shared insert"
            );
            assert_ne!(bits(&audio), bits(&plain), "the pad is not heard");
        }
    }
}

// Known answers 3.2: `--once` renders the pattern a single time, the generator's frames, the
// bytes of the looped render's first pass; looped over two passes the halves are the same
// bytes. Why this holds the engine's once and not only the command's arithmetic (the command
// sets a once render's length from the pattern's bars): the command refuses a once render whose
// player has not stopped by itself on its last frame, so a once that played on fails here, and
// one that stopped early renders silence where the looped pass has its last keys.
#[test]
fn once_renders_one_pass_and_a_loop_renders_copies_of_it() {
    let dir = temp_dir("once_loop");
    let want = expected();
    let once = &want["once"];
    let (pattern, channel) = (text(&once["pattern"]), text(&once["channel"]));
    let one = frames(&render(
        &dir,
        text(&once["project"]),
        "once.wav",
        &["--pattern", pattern, "--channel", channel, "--once"],
    ));
    assert_eq!(one.len(), number(&once["frames"]), "once: one pass");

    let looped = &want["loop"];
    let bars = looped["bars"].to_string();
    let two = frames(&render(
        &dir,
        text(&looped["project"]),
        "loop.wav",
        &["--pattern", pattern, "--channel", channel, "--bars", &bars],
    ));
    assert_eq!(two.len(), number(&looped["frames"]), "loop: two passes");
    let (first, second) = two.split_at(number(&looped["pass_frames"]));
    assert_eq!(
        bits(first),
        bits(second),
        "the two passes are the same bits"
    );
    assert_eq!(bits(&one), bits(first), "once is the first pass");
}

// The flags' rules: a channel or a pattern the project does not name, `--once` without
// `--channel` or with `--bars`, and `--song` with either are refused as usage errors (exit 2,
// the usage printed); nothing is written.
#[test]
fn the_render_refuses_what_its_flags_cannot_mean() {
    let dir = temp_dir("refused");
    let project = fixture("rp_keys_drums.json");
    let out = dir.join("refused.wav");
    for (flags, says) in [
        (
            &["--pattern", "Chorus"][..],
            "no pattern is named \"Chorus\"",
        ),
        (&["--channel", "bass"][..], "no channel is named \"bass\""),
        (&["--once"][..], "--once plays a channel"),
        (
            &["--channel", "keys", "--once", "--bars", "2"][..],
            "--once renders",
        ),
        (&["--song", "--channel", "keys"][..], "--song"),
        (&["--song", "--pattern", "Verse"][..], "--song"),
    ] {
        let mut args: Vec<&OsStr> = vec!["render".as_ref(), project.as_os_str(), out.as_os_str()];
        args.extend(flags.iter().map(OsStr::new));
        let output = dawsome(&dir, &args);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{flags:?}: {stderr}");
        assert!(stderr.contains(says), "{flags:?}: {stderr}");
        assert!(!out.exists(), "{flags:?}: nothing written");
    }
}

// A channel with a MIDI out played once sends nothing from past the pattern's end: the next
// pass's first note would be sent its latency (480 frames) ahead, inside the render, and cut by
// the stop as it began. Looped, it is sent there, and the render's Stop at its end ends it. The
// messages on their frames as gen_roll_player.py works them out by gen_p19's rules, none dropped.
#[test]
fn played_once_a_midi_out_sends_nothing_past_the_patterns_end() {
    let dir = temp_dir("midi_out");
    let want = expected();
    let case = &want["midi_out"];
    let project = fixture(text(&case["project"]));
    for (name, flags) in [("once", &["--once"][..]), ("looped", &["--bars", "1"][..])] {
        let out = dir.join(format!("{name}.wav"));
        let log = dir.join(format!("{name}.json"));
        let named = [
            "--pattern",
            text(&case["pattern"]),
            "--channel",
            text(&case["channel"]),
        ];
        let mut args: Vec<&OsStr> = vec!["render".as_ref(), project.as_os_str(), out.as_os_str()];
        args.extend(named.iter().chain(flags).map(OsStr::new));
        args.extend([OsStr::new("--midi-out-log"), log.as_os_str()]);
        let output = dawsome(&dir, &args);
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let sent: Value =
            serde_json::from_str(&fs::read_to_string(&log).expect("the log")).expect("parses");
        assert_eq!(sent["events"], case[name], "{name}");
        assert_eq!(sent["dropped"], 0, "{name}");
        assert_eq!(
            frames(&fs::read(&out).expect("the WAV")).len(),
            number(&case["frames"])
        );
    }
}
