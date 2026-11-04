//! The native effects through the command line: each p32 project (one sampler channel playing an
//! input file into insert 1, which holds the effect) renders what
//! tests/fixtures/gen_p32_expected.py computes from the formulas it restates (p32_expected.json),
//! checked as each case's "asserts" list says. Every run gets per-user folders of its own under
//! the test's temp dir.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha256};

/// The cases this test renders, by name in p32_expected.json: the render path with no effect
/// (the input's bits), then each effect's. Every case of the answers is here
/// (`p32_every_case_of_the_answers_is_rendered`).
const CASES: [&str; 47] = [
    "dry_impulse",
    "dry_sine440",
    "dry_two_tone",
    "dry_stereo",
    "dry_bursts",
    "chorus_default",
    "chorus_extreme",
    "chorus_mix0",
    "chorus_still",
    "chorus_still_half",
    "flanger_default",
    "flanger_comb_pos",
    "flanger_comb_neg",
    "flanger_mix0",
    "flanger_feedback70",
    "flanger_feedback70_still",
    "flanger_spread0",
    "flanger_lead",
    "phaser_default",
    "phaser_still",
    "phaser_feedback",
    "phaser_mix0",
    "phaser_spread0",
    "phaser_lead",
    "distortion_default",
    "distortion_auto_off",
    "distortion_hot",
    "distortion_mix0",
    "filter_default",
    "filter_lp_q20",
    "filter_hp_q20",
    "filter_bp_q20",
    "filter_notch_q20",
    "filter_lp24_default",
    "filter_lp24_q20",
    "filter_lp24_far",
    "gate_default",
    "gate_range0",
    "gate_hold0",
    "gate_release_1ms",
    "gate_rms_closed",
    "gate_rms_open",
    "width_200",
    "width_0",
    "width_100",
    "width_bass_mono",
    "width_bass_mono_mono",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> PathBuf {
    repo_root().join("tests/fixtures")
}

/// Fresh empty directory under cargo's per-target test directory.
fn temp_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("effects")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p32_expected.json"))
        .expect("p32_expected.json (run gen_p32_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("parses")
}

/// `dawsome render <project> <dir>/<name>.wav --bits 32f`, every per-user folder under `dir`;
/// the frames it wrote, left and right.
fn render(project: &Path, dir: &Path, name: &str) -> Vec<[f32; 2]> {
    render_with(project, dir, name, &[])
}

/// As [`render`], with `args` added.
fn render_with(project: &Path, dir: &Path, name: &str, args: &[&str]) -> Vec<[f32; 2]> {
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
        .arg(project)
        .arg(&out)
        .args(["--bits", "32f"])
        .args(args)
        .output()
        .expect("run dawsome");
    assert!(
        output.status.success(),
        "dawsome render {} failed: {}",
        project.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut reader = hound::WavReader::open(&out).expect("open rendered wav");
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

/// An input file's frames, a mono file's on both sides.
fn input_frames(name: &str) -> Vec<[f32; 2]> {
    let mut reader = hound::WavReader::open(fixtures().join(name)).expect("an input file");
    let channels = usize::from(reader.spec().channels);
    let values: Vec<f32> = reader
        .samples::<f32>()
        .map(|v| v.expect("decodes"))
        .collect();
    values
        .chunks_exact(channels)
        .map(|frame| [frame[0], frame[channels - 1]])
        .collect()
}

/// The SHA-256 of `frames` as 32-bit floats, little-endian, left then right (the generator's).
fn frames_hash(frames: &[[f32; 2]]) -> String {
    let mut hasher = Sha256::new();
    for frame in frames {
        hasher.update(frame[0].to_le_bytes());
        hasher.update(frame[1].to_le_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn number(value: &Value) -> f64 {
    value.as_f64().expect("a number")
}

fn index(value: &Value) -> usize {
    usize::try_from(value.as_u64().expect("a frame")).expect("fits")
}

/// An f32 as the generator writes it: the shortest digits that read back as that f32.
fn sample(value: &Value) -> f32 {
    number(value) as f32
}

/// The amplitude of the `hz` component of `side` over `[lo, hi)`, a whole number of periods at
/// `rate`: the projection onto sine and cosine (gen_p5's `amplitude`).
fn amplitude(side: &[f32], hz: f64, rate: f64, lo: usize, hi: usize) -> f64 {
    let (mut s, mut c) = (0.0, 0.0);
    for (n, &y) in side.iter().enumerate().take(hi).skip(lo) {
        let phase = std::f64::consts::TAU * hz * n as f64 / rate;
        s += f64::from(y) * phase.sin();
        c += f64::from(y) * phase.cos();
    }
    2.0 * s.hypot(c) / (hi - lo) as f64
}

/// Asserts `frames`, the render of `case`, holds check `check` of the case's answer.
fn assert_check(case: &str, check: &str, frames: &[[f32; 2]], answer: &Value, want: &Value) {
    match check {
        "sha256" => assert_eq!(
            frames_hash(frames),
            answer["sha256"].as_str().expect("sha256"),
            "{case}: the hash of every frame"
        ),
        "same_as" => {
            let dry = answer["same_as"].as_str().expect("a case");
            assert_eq!(
                frames_hash(frames),
                want["cases"][dry]["sha256"].as_str().expect("sha256"),
                "{case}: the bits of {dry}"
            );
        }
        "probes" => {
            let tolerance = number(&want["tolerances"]["close"]);
            let probes = &answer["probes"];
            let at = want["probe_frames"].as_array().expect("probe frames");
            for (side, key) in ["left", "right"].into_iter().enumerate() {
                let values = probes[key].as_array().expect("probes");
                assert_eq!(values.len(), at.len(), "{case}: {key} probes");
                for (frame, value) in at.iter().map(index).zip(values) {
                    let (got, ideal) = (frames[frame][side], sample(value));
                    assert!(
                        (f64::from(got) - f64::from(ideal)).abs() <= tolerance,
                        "{case}, frame {frame} {key}: {got} is not {ideal}"
                    );
                }
            }
        }
        "amplitude" => {
            let amp = &answer["amplitude"];
            let (lo, hi) = (index(&amp["window"][0]), index(&amp["window"][1]));
            let tolerance = number(&amp["tolerance"]);
            let rate = number(&want["rate"]);
            for side in 0..2 {
                // One value for both sides, or [left, right].
                let ideal = match amp["expected"].as_array() {
                    Some(sides) => number(&sides[side]),
                    None => number(&amp["expected"]),
                };
                let values: Vec<f32> = frames.iter().map(|frame| frame[side]).collect();
                let got = amplitude(&values, number(&amp["hz"]), rate, lo, hi);
                assert!(
                    (got - ideal).abs() <= tolerance,
                    "{case}, side {side}: amplitude {got}, expected {ideal}"
                );
            }
        }
        "peak_at_most" => {
            let most = number(&answer["peak_at_most"]) as f32;
            let peak = frames
                .iter()
                .flat_map(|frame| frame.map(f32::abs))
                .fold(0.0f32, f32::max);
            assert!(peak <= most, "{case}: peak {peak} past {most}");
        }
        "zero_before" => {
            let before = index(&answer["zero_before"]);
            assert!(before <= frames.len(), "{case}: frames to check");
            let first = frames[..before].iter().position(|frame| *frame != [0.0; 2]);
            assert_eq!(first, None, "{case}: a frame before {before} is not 0");
        }
        "sounding" => {
            let rows = answer["sounding"].as_array().expect("sounding frames");
            assert!(!rows.is_empty(), "{case}: no sounding frames to check");
            let mut sounding = vec![None; frames.len()];
            for row in rows {
                sounding[index(&row[0])] = Some(sample(&row[1]));
            }
            for (frame, (got, want)) in frames.iter().zip(sounding).enumerate() {
                let want = want.map_or([0.0; 2], |value| [value; 2]);
                assert_eq!(
                    got.map(f32::to_bits),
                    want.map(f32::to_bits),
                    "{case}, frame {frame}"
                );
            }
        }
        "zero_from" => {
            let from = index(&answer["zero_from"]);
            assert!(from < frames.len(), "{case}: a tail to check");
            let first = frames[from..].iter().position(|frame| *frame != [0.0; 2]);
            assert_eq!(first, None, "{case}: a frame from {from} is not 0");
        }
        "stretch.at_floor_from" => {
            let stretch = &answer["stretch"];
            let (from, end) = (
                index(&stretch["at_floor_from"]),
                index(&stretch["frames"][1]),
            );
            let floor = number(&stretch["floor"]);
            let input = input_frames(answer["input"].as_str().expect("input"));
            for frame in from..end {
                let at_floor = input[frame].map(|x| (f64::from(x) * floor) as f32);
                assert_eq!(
                    frames[frame].map(f32::to_bits),
                    at_floor.map(f32::to_bits),
                    "{case}, frame {frame}: not the input at the floor"
                );
            }
        }
        "hand_frames" => {
            let input = input_frames(answer["input"].as_str().expect("input"));
            for row in answer["hand_frames"].as_array().expect("hand frames") {
                let frame = index(&row[0]);
                let given = if frame < input.len() {
                    input[frame]
                } else {
                    [0.0; 2]
                };
                assert_eq!(
                    given.map(f32::to_bits),
                    [sample(&row[1]), sample(&row[2])].map(f32::to_bits),
                    "{case}, frame {frame}: the input"
                );
                assert_eq!(
                    frames[frame].map(f32::to_bits),
                    [sample(&row[3]), sample(&row[4])].map(f32::to_bits),
                    "{case}, frame {frame}: the output"
                );
            }
        }
        "lead" => {
            let lead = &answer["lead"];
            let shift = index(&lead["frames"]);
            let (from, to) = (index(&lead["from"]), index(&lead["to"]));
            let tolerance = number(&lead["tolerance"]);
            assert!(to + shift <= frames.len(), "{case}: frames to check");
            for n in from..to {
                let (right, later) = (frames[n][1], frames[n + shift][0]);
                assert!(
                    (f64::from(right) - f64::from(later)).abs() <= tolerance,
                    "{case}, frame {n}: right {right} is not left {later} {shift} frames on"
                );
            }
        }
        "sides_differ" => {
            let by = number(&answer["sides_differ"]["by_more_than"]);
            let most = frames
                .iter()
                .map(|[left, right]| (f64::from(*left) - f64::from(*right)).abs())
                .fold(0.0, f64::max);
            assert!(most > by, "{case}: the sides differ by at most {most}");
        }
        "scaled" => {
            let scaled = &answer["scaled"];
            let other = scaled["of"].as_str().expect("a case");
            let (factor, tolerance) = (number(&scaled["factor"]), number(&scaled["tolerance"]));
            let probes = &want["cases"][other]["probes"];
            let at = want["probe_frames"].as_array().expect("probe frames");
            for (side, key) in ["left", "right"].into_iter().enumerate() {
                let values = probes[key].as_array().expect("probes");
                for (frame, value) in at.iter().map(index).zip(values) {
                    let (got, base) = (frames[frame][side], sample(value));
                    assert!(
                        (f64::from(got) - factor * f64::from(base)).abs() <= tolerance,
                        "{case}, frame {frame} {key}: {got} is not {factor} x {other}'s {base}"
                    );
                }
            }
        }
        "left_equals_right" => {
            let differ = frames
                .iter()
                .position(|[left, right]| left.to_bits() != right.to_bits());
            assert_eq!(differ, None, "{case}: left and right differ");
        }
        other => panic!("{case}: no check for \"{other}\""),
    }
}

/// Known answers (gen_p32_expected.py): every listed case rendered through the command line to
/// 32-bit float holds every check its "asserts" list names: the exact tier's hash, the close
/// tier's probes within 1e-6, the closed forms' amplitudes within 1e-4, and each case's own
/// claims (the chorus's silence before its shortest delay, the flanger combs' echoes, the gate's
/// −60 dB stretch at its floor, the width's hand frames, the distortion's peak and silent
/// tail, the flanger's and phaser's right side leading the left by a
/// quarter of the sweep, and the distortion's auto gain as a scale of its render without it).
/// A listed case missing from the answers, or a check this test does not know, fails it.
#[test]
fn p32_the_native_effects_render_their_known_answers() {
    let want = expected();
    let dir = temp_dir("renders");
    let frames_each = index(&want["frames"]);
    for case in CASES {
        let answer = &want["cases"][case];
        assert!(
            answer.is_object(),
            "{case} is not a case of p32_expected.json"
        );
        let fixture = fixtures().join(answer["fixture"].as_str().expect("fixture"));
        let frames = render(&fixture, &dir, case);
        assert_eq!(frames.len(), frames_each, "{case}: frames");
        let checks = answer["asserts"].as_array().expect("asserts");
        assert!(!checks.is_empty(), "{case}: nothing to check");
        for check in checks {
            assert_check(
                case,
                check.as_str().expect("a check"),
                &frames,
                answer,
                &want,
            );
        }
    }
}

/// Known answers (gen_p32_expected.py): the cases this test renders are every case of
/// p32_expected.json, by count and by name, so none can be left out unseen.
#[test]
fn p32_every_case_of_the_answers_is_rendered() {
    let want = expected();
    let mut answers: Vec<&str> = want["cases"]
        .as_object()
        .expect("cases")
        .keys()
        .map(String::as_str)
        .collect();
    let mut listed = CASES.to_vec();
    assert_eq!(listed.len(), answers.len(), "how many cases");
    answers.sort_unstable();
    listed.sort_unstable();
    assert_eq!(listed, answers, "the cases' names");
}

/// Known answers (gen_p32_expected.py): an effect's light. filter_lp_q20's project with its
/// filter marked `"on":false` renders the bits of dry_sine440 (the input passed by, as the mix-0
/// cases render it through the same insert), and marked `"on":true` holds every check of
/// filter_lp_q20 (its probes and its amplitude; that case's tier is "close", so not its hash)
/// and renders the fixture's own bits, as the fixture says nothing of it and so is on. The
/// copies are written beside a copy of its input, so its relative sample path finds it.
#[test]
fn p32_an_effect_switched_off_renders_the_dry_input() {
    let want = expected();
    let dir = temp_dir("light");
    let filter = &want["cases"]["filter_lp_q20"];
    let dry = &want["cases"]["dry_sine440"];
    let input = filter["input"].as_str().expect("input");
    assert_eq!(dry["input"], filter["input"], "the same input");
    fs::copy(fixtures().join(input), dir.join(input)).expect("copy the input");
    let fixture = filter["fixture"].as_str().expect("fixture");
    let text = fs::read_to_string(fixtures().join(fixture)).expect("the fixture");
    let mut project: Value = serde_json::from_str(&text).expect("parses");
    let effect = &mut project["mixer"]["inserts"][1]["effects"][0];
    assert_eq!(effect["processor"]["id"], "dawsome.filter");
    assert!(effect.get("on").is_none(), "the fixture says nothing of it");
    for (on, name) in [(false, "off"), (true, "on")] {
        project["mixer"]["inserts"][1]["effects"][0]["on"] = Value::Bool(on);
        let path = dir.join(format!("filter_lp_q20_{name}.json"));
        fs::write(&path, serde_json::to_string(&project).expect("writes")).expect("save");
        let frames = render(&path, &dir, name);
        assert_eq!(frames.len(), index(&want["frames"]), "{name}: frames");
        if on {
            for check in filter["asserts"].as_array().expect("asserts") {
                let check = check.as_str().expect("a check");
                assert_check("filter_lp_q20 on", check, &frames, filter, &want);
            }
            let as_saved = render(&fixtures().join(fixture), &dir, "as_saved");
            assert_eq!(
                frames_hash(&frames),
                frames_hash(&as_saved),
                "on: the bits of the fixture, which says nothing of it"
            );
        } else {
            assert_eq!(
                frames_hash(&frames),
                dry["sha256"].as_str().expect("sha256"),
                "off: the bits of dry_sine440"
            );
        }
    }
}

/// The Delay's Time moved while it sounds fades between the old echo and the new instead of
/// jumping (turning Time used to make static). p32_chorus_default's
/// project with a Delay in the chorus's place (100 ms, no feedback, all wet) on the 0.5 s
/// 440 Hz sine, its Time automated from 100 ms on tick 24 to 200 ms on tick 72 and held there
/// (the playlist: the pattern, and the automation on a track of its own), rendered as the song:
/// the automation sets Time every 32 frames, the tap moving about 13 frames each time.
///
/// The echo is (1 − g)·a + g·b of two taps of the input, g rising by 1/F a frame (F = 30 ms =
/// 1323 frames), and the mixer after it a constant gain, so no step of the render is more than
/// the steady delay's largest step plus 2·peak/F (the weights move against taps at most twice
/// the peak apart). The steady render is the same project at a constant 200 ms. Once the last
/// fade is over (two fades after tick 72) the render is that one's bits, and any block size
/// renders the same bits.
#[test]
fn p32_a_delay_time_automated_while_it_sounds_has_no_click() {
    let dir = temp_dir("delay_time");
    let input = "p32_sine440.wav";
    fs::copy(fixtures().join(input), dir.join(input)).expect("copy the input");
    let text = fs::read_to_string(fixtures().join("p32_chorus_default.json")).expect("fixture");
    let mut project: Value = serde_json::from_str(&text).expect("parses");
    assert_eq!(project["channels"][0]["sample"], input);
    let effect = &mut project["mixer"]["inserts"][1]["effects"][0];
    assert_eq!(effect["processor"]["id"], "dawsome.chorus");
    effect["processor"] = serde_json::json!({
        "id": "dawsome.delay",
        "params": {"time_ms": 100.0, "feedback": 0.0, "mix": 1.0}
    });
    let tracks = serde_json::json!([
        {"id": 1, "name": "Track 1", "mute": false},
        {"id": 2, "name": "Track 2", "mute": false}
    ]);
    let pattern = serde_json::json!(
        {"id": 1, "track": 1, "start": 0, "length": 384, "kind": "pattern", "pattern": 1}
    );
    let automation = serde_json::json!(
        {"id": 2, "track": 2, "start": 0, "length": 384, "kind": "automation", "automation": 1}
    );
    // Time's range is 1 to 4000 ms, so t ms is (t − 1)/3999 of it.
    let at = |ms: f64| (ms - 1.0) / 3999.0;
    let save = |project: &Value, name: &str| {
        let path = dir.join(format!("{name}.json"));
        fs::write(&path, serde_json::to_string(project).expect("writes")).expect("save");
        path
    };
    project["playlist"] = serde_json::json!(
        {"next_track_id": 3, "next_clip_id": 3, "tracks": tracks, "clips": [pattern]}
    );
    project["mixer"]["inserts"][1]["effects"][0]["processor"]["params"]["time_ms"] =
        serde_json::json!(200.0);
    let steady = render_with(&save(&project, "steady"), &dir, "steady", &["--song"]);
    project["mixer"]["inserts"][1]["effects"][0]["processor"]["params"]["time_ms"] =
        serde_json::json!(100.0);
    project["playlist"]["clips"] = serde_json::json!([pattern, automation]);
    project["automation"] = serde_json::json!({"next_id": 2, "list": [{
        "id": 1,
        "target": {"effect": {"id": 1, "param": "time_ms"}},
        "points": [{"tick": 24, "value": at(100.0)}, {"tick": 72, "value": at(200.0)}]
    }]});
    let path = save(&project, "automated");
    let moved = render_with(&path, &dir, "moved", &["--song"]);
    assert_eq!(moved.len(), 88_200, "one bar");
    assert_eq!(steady.len(), moved.len(), "the steady render's length");

    let step = |frames: &[[f32; 2]]| {
        frames
            .windows(2)
            .flat_map(|pair| [0, 1].map(|side| (pair[1][side] - pair[0][side]).abs()))
            .fold(0.0f32, f32::max)
    };
    let peak = steady
        .iter()
        .flat_map(|frame| frame.map(f32::abs))
        .fold(0.0f32, f32::max);
    assert!(peak > 0.1, "the steady delay sounds: peak {peak}");
    let bound = step(&steady) + 2.0 * peak / 1323.0 + 1e-6;
    let worst = step(&moved);
    assert!(worst <= bound, "a step of {worst}, more than {bound}");
    // Tick 72 at 120 BPM, 44.1 kHz is frame 16537.5, so 16538 is the first at or past it.
    let settled = 16_538 + 2 * 1323;
    let differ =
        (settled..moved.len()).find(|&n| moved[n].map(f32::to_bits) != steady[n].map(f32::to_bits));
    assert_eq!(
        differ, None,
        "a frame after the fades is not the steady delay's"
    );
    assert!(
        steady[settled..].iter().any(|frame| *frame != [0.0; 2]),
        "the settled part sounds"
    );
    let small = render_with(&path, &dir, "moved_block7", &["--song", "--block", "7"]);
    assert_eq!(frames_hash(&small), frames_hash(&moved), "7-frame blocks");
}
