//! Known-answer tests: run the real `dawsome` binary on fixture projects and check the rendered WAV
//! against numbers computed by hand or printed by the Python generators in tests/fixtures. Expected
//! values never come from the engine itself.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use dawsome_engine::project::{FORMER_NATIVE_ID_PREFIX, NATIVE_ID_PREFIX};

/// Rendered audio: one `Vec<f32>` per channel.
struct Rendered {
    channels: Vec<Vec<f32>>,
}

impl Rendered {
    fn frames(&self) -> usize {
        self.channels[0].len()
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Fresh empty directory under cargo's per-target test directory (on the build drive).
fn temp_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Runs `dawsome render <fixture> <out> <extra args>` and requires success.
fn render(fixture: &str, out: &Path, extra: &[&str]) {
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .arg("render")
        .arg(format!("tests/fixtures/{fixture}.json"))
        .arg(out)
        .args(extra)
        .output()
        .expect("run dawsome binary");
    assert!(
        output.status.success(),
        "dawsome render {fixture} {extra:?} failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Reads a WAV back, requiring 32-bit float stereo, and splits the channels.
fn read_stereo(path: &Path) -> Rendered {
    let mut reader = hound::WavReader::open(path).expect("open rendered wav");
    let spec = reader.spec();
    assert_eq!(spec.channels, 2, "output must be stereo");
    assert_eq!(spec.sample_format, hound::SampleFormat::Float);
    assert_eq!(spec.bits_per_sample, 32);
    let interleaved: Vec<f32> = reader
        .samples::<f32>()
        .collect::<Result<_, _>>()
        .expect("decode samples");
    let left = interleaved.iter().copied().step_by(2).collect();
    let right = interleaved.iter().copied().skip(1).step_by(2).collect();
    Rendered {
        channels: vec![left, right],
    }
}

/// Asserts both channels match `expected` (frame index -> value) exactly and
/// every other frame is exactly 0.0.
fn assert_exact(audio: &Rendered, total_frames: usize, expected: &[(usize, f32)]) {
    assert_eq!(audio.frames(), total_frames, "frame count");
    for (ch, samples) in audio.channels.iter().enumerate() {
        assert_eq!(samples.len(), total_frames, "channel {ch} length");
        for (i, &got) in samples.iter().enumerate() {
            let want = expected
                .iter()
                .find(|(pos, _)| *pos == i)
                .map_or(0.0, |(_, v)| *v);
            assert_eq!(got, want, "channel {ch}, frame {i}");
        }
    }
}

/// Expected list of `(position, 1.0)` impulses.
fn unit_impulses(positions: &[usize]) -> Vec<(usize, f32)> {
    positions.iter().map(|&p| (p, 1.0)).collect()
}

// Proves: step grid + tick->sample at 44.1k/120 BPM land impulses on exact frames.
#[test]
fn f1_four_on_floor() {
    let out = temp_dir("f1_four_on_floor").join("out.wav");
    render("f1_four_on_floor", &out, &[]);
    let audio = read_stereo(&out);
    assert_exact(&audio, 88200, &unit_impulses(&[0, 22050, 44100, 66150]));
}

// Proves: flooring of fractional samples-per-tick at 48k/140 BPM (bar = 82285 frames). A sound file
// loads resampled to the project's rate, so the 44.1 kHz impulse plays as the two-frame click
// tests/fixtures/gen_p8_expected.py prints (from resample_oracle.py), on the same four onsets;
// every other frame is exactly silent.
#[test]
fn f2_tempo_140_48k() {
    let out = temp_dir("f2_tempo_140_48k").join("out.wav");
    render("f2_tempo_140_48k", &out, &[]);
    let audio = read_stereo(&out);
    assert_close(&audio, 82285, &resampled_clicks(&[0, 20571, 41142, 61714]));
}

/// The two frames impulse.wav (one 1.0 at 44.1 kHz) loads as in a 48 kHz project, printed by
/// tests/fixtures/gen_p8_expected.py from resample_oracle.py (f64; the engine's resampler computes
/// in f32).
const CLICK_48K: [f64; 2] = [0.953_838_531_317_332_2, 0.131_178_001_559_130_76];

/// Largest difference between a resampled frame and the oracle's f64 value: the FFT resampler
/// computes in f32, whose rounding over 588- and 640-point transforms stays below 2e-6.
const RESAMPLE_TOLERANCE: f64 = 2e-6;

/// The resampled click on each of `onsets`.
fn resampled_clicks(onsets: &[usize]) -> Vec<(usize, f64)> {
    onsets
        .iter()
        .flat_map(|&onset| {
            CLICK_48K
                .iter()
                .enumerate()
                .map(move |(k, &v)| (onset + k, v))
        })
        .collect()
}

/// Asserts both channels hold `expected` (frame index -> value) within
/// [`RESAMPLE_TOLERANCE`] and every other frame is exactly 0.0.
fn assert_close(audio: &Rendered, total_frames: usize, expected: &[(usize, f64)]) {
    assert_eq!(audio.frames(), total_frames, "frame count");
    for (ch, samples) in audio.channels.iter().enumerate() {
        for (i, &got) in samples.iter().enumerate() {
            match expected.iter().find(|(pos, _)| *pos == i) {
                Some(&(_, want)) => assert!(
                    (f64::from(got) - want).abs() <= RESAMPLE_TOLERANCE,
                    "channel {ch}, frame {i}: {got} != {want}"
                ),
                None => assert_eq!(got, 0.0, "channel {ch}, frame {i}"),
            }
        }
    }
}

// Proves: velocity scales the whole sample, and a multi-frame sample plays out in order.
#[test]
fn f3_velocity_ramp() {
    let out = temp_dir("f3_velocity_ramp").join("out.wav");
    render("f3_velocity_ramp", &out, &[]);
    let audio = read_stereo(&out);
    let expected = [(0, 0.5), (1, 0.25), (2, 0.125), (3, 0.0625)];
    assert_exact(&audio, 88200, &expected);
}

// Proves: two simultaneous channels sum in float (0.8 + 0.8 = 1.6) with no clipping.
#[test]
fn f4_saturated() {
    let out = temp_dir("f4_saturated").join("out.wav");
    render("f4_saturated", &out, &[]);
    let audio = read_stereo(&out);
    assert_eq!(audio.frames(), 88200, "frame count");
    for (ch, samples) in audio.channels.iter().enumerate() {
        assert!(
            (samples[0] - 1.6).abs() < 1e-6,
            "channel {ch} frame 0 = {}",
            samples[0]
        );
        assert!(
            samples[1..].iter().all(|&s| s == 0.0),
            "channel {ch} must be silent after frame 0"
        );
    }
}

// Proves: output bytes do not depend on the render block size (64, 4096, odd 1000).
#[test]
fn f5_block_size_invariance() {
    let dir = temp_dir("f5_block_size_invariance");
    let mut files = Vec::new();
    for block in ["64", "4096", "1000"] {
        let out = dir.join(format!("block{block}.wav"));
        render("f1_four_on_floor", &out, &["--block", block]);
        files.push(fs::read(&out).expect("read rendered wav"));
    }
    assert!(!files[0].is_empty(), "rendered file is empty");
    assert!(files[0] == files[1], "--block 64 differs from --block 4096");
    assert!(files[0] == files[2], "--block 64 differs from --block 1000");
}

// Proves: --bars 2 loops the pattern, with bar 2 starting at the bar sample (88200).
#[test]
fn f6_two_bars() {
    let out = temp_dir("f6_two_bars").join("out.wav");
    render("f1_four_on_floor", &out, &["--bars", "2"]);
    let audio = read_stereo(&out);
    let positions = [0, 22050, 44100, 66150, 88200, 110250, 132300, 154350];
    assert_exact(&audio, 176400, &unit_impulses(&positions));
}

// Proves: sample paths resolve against the project file, not the current directory.
#[test]
fn f1_renders_from_any_current_directory() {
    let out = temp_dir("f1_any_cwd").join("out.wav");
    let project = repo_root().join("tests/fixtures/f1_four_on_floor.json");
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir("/")
        .arg("render")
        .arg(&project)
        .arg(&out)
        .output()
        .expect("run dawsome binary");
    assert!(
        output.status.success(),
        "render from / failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let audio = read_stereo(&out);
    assert_exact(&audio, 88200, &unit_impulses(&[0, 22050, 44100, 66150]));
}

// Proves: the output may not be the project file or one of its samples, and the refused
// file is left untouched.
#[test]
fn output_that_is_an_input_is_refused() {
    let dir = temp_dir("output_is_input");
    let fixtures = repo_root().join("tests/fixtures");
    for file in ["f1_four_on_floor.json", "impulse.wav"] {
        fs::copy(fixtures.join(file), dir.join(file)).expect("copy fixture");
    }
    for target in ["f1_four_on_floor.json", "impulse.wav"] {
        let before = fs::read(dir.join(target)).expect("read input");
        // A path through `..` names the same file differently: the check compares real paths.
        let out = dir
            .join("..")
            .join(dir.file_name().expect("dir name"))
            .join(target);
        let output = Command::new(env!("CARGO_BIN_EXE_dawsome"))
            .arg("render")
            .arg(dir.join("f1_four_on_floor.json"))
            .arg(&out)
            .output()
            .expect("run dawsome binary");
        assert_eq!(output.status.code(), Some(1), "{target}");
        let message = String::from_utf8_lossy(&output.stderr);
        assert!(
            message.contains("also an input file"),
            "{target}: {message}"
        );
        assert_eq!(
            fs::read(dir.join(target)).expect("read input"),
            before,
            "{target} changed"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Piano roll and pitch from key: every expected document and every expected frame below is printed
// by tests/fixtures/gen_p3_expected.py, which also wrote the p3_* fixtures. The documents are the
// results of the generator's piano-roll gestures (a) to (f1); the app tests check that the gestures
// produce exactly these bytes, and these tests check what they sound like.
// ---------------------------------------------------------------------------------------------

/// Renders `fixture` for `bars` bars and checks it against `expected` exactly.
fn renders_exactly(fixture: &str, bars: u32, expected: &[(usize, f32)]) {
    let out = temp_dir(fixture).join("out.wav");
    render(fixture, &out, &["--bars", &bars.to_string()]);
    assert_exact(&read_stereo(&out), 88200 * bars as usize, expected);
}

// The starting document: lead A (tick 0, velocity 100/128), lead B (tick 96, key 64, velocity
// 0.5), kick on ticks 192 and 288. Key 64 plays the one-frame impulse faster, still one frame.
#[test]
fn p3_roll_plays_its_four_notes() {
    let expected = [(0, 0.78125), (22050, 0.5), (44100, 1.0), (66150, 1.0)];
    renders_exactly("p3_roll", 1, &expected);
}

// (a) A moved to tick 24: floor(24·229.6875) = floor(5512.5) = 5512.
#[test]
fn p3_a_moved_note_starts_on_its_new_tick() {
    let expected = [(5512, 0.78125), (22050, 0.5), (44100, 1.0), (66150, 1.0)];
    renders_exactly("p3_a_move", 1, &expected);
}

// (b) Length never ends a voice: A resized to 96 ticks renders the same bytes as before, and a
// 2000-frame sample held for 1 tick (229.6875 samples) or 96 ticks plays all 2000 frames.
#[test]
fn p3_b_note_length_does_not_change_the_sound() {
    let dir = temp_dir("p3_b_note_length");
    let bytes = |fixture: &str| {
        let out = dir.join(format!("{fixture}.wav"));
        render(fixture, &out, &[]);
        fs::read(&out).expect("read rendered wav")
    };
    assert!(
        bytes("p3_b_resize") == bytes("p3_roll"),
        "resizing changed the render"
    );
    let held: Vec<(usize, f32)> = (0..2000).map(|frame| (frame, 0.25)).collect();
    for fixture in ["p3_hold", "p3_hold_96"] {
        let out = dir.join(format!("{fixture}.wav"));
        render(fixture, &out, &[]);
        assert_exact(&read_stereo(&out), 88200, &held);
    }
    assert!(bytes("p3_hold") == bytes("p3_hold_96"));
}

// (c) A's velocity dragged to 64/128.
#[test]
fn p3_c_velocity_scales_the_note() {
    let expected = [(0, 0.5), (22050, 0.5), (44100, 1.0), (66150, 1.0)];
    renders_exactly("p3_c_velocity", 1, &expected);
}

// (d) Ctrl+B copies A and B by their extent (0..192) rounded up to a step: copies at 192 and
// 288 sound with the kicks (1.0 + 0.78125, 1.0 + 0.5). (d2) a second Ctrl+B copies the copies
// to 384 and 480 and grows the pattern to 2 bars, so the kick does not repeat in bar 2.
#[test]
fn p3_d_duplicates_land_one_extent_later() {
    let one = [(0, 0.78125), (22050, 0.5), (44100, 1.78125), (66150, 1.5)];
    renders_exactly("p3_d_duplicate", 1, &one);
    let two = [
        (0, 0.78125),
        (22050, 0.5),
        (44100, 1.78125),
        (66150, 1.5),
        (88200, 0.78125),
        (110250, 0.5),
    ];
    renders_exactly("p3_d2_duplicate_again", 2, &two);
}

// (e) A note drawn at tick 408 grows the pattern to 2 bars: floor(408·229.6875) = 93712.
#[test]
fn p3_e_a_note_past_the_end_plays_in_the_grown_pattern() {
    let expected = [
        (0, 0.78125),
        (22050, 0.5),
        (44100, 1.0),
        (66150, 1.0),
        (93712, 0.78125),
    ];
    renders_exactly("p3_e_grow", 2, &expected);
}

// (f1) B moved onto A's tick and key replaces A; a partial overlap on one key is two voices.
#[test]
fn p3_f1_replaced_and_overlapping_notes() {
    renders_exactly("p3_f1_replace", 1, &[(0, 0.5), (44100, 1.0), (66150, 1.0)]);
    let overlap = [
        (0, 0.78125),
        (5512, 0.78125),
        (22050, 0.5),
        (44100, 1.0),
        (66150, 1.0),
    ];
    renders_exactly("p3_f1_overlap", 1, &overlap);
}

// (f2) 65 notes at tick 0, keys 0..64, velocity (k+1)/128, on 64 voices: the 65th takes the
// voice of key 0 (the first to start, lowest slot), so frame 0 = Σ(k+1)/128 for k = 1..64
// = 16.75 exactly (dropping the new note gives 16.25, taking the newest voice 16.2578125).
// Keys below 60 slow the impulse into a falling curve (the 4-point Hermite's, the points around
// the one frame 0: 1 - 2.5f^2 + 1.5f^3 at f = p), so frames 1..30 sound too; those sums of
// irrational ratios are checked to 1e-5 (gen_p3_expected.py prints them).
#[test]
fn p3_f2_a_full_voice_pool_takes_over_the_first_voice() {
    const TAIL: [f32; 30] = [
        9.409_697,
        5.467_255_6,
        3.678_768_2,
        2.639_012,
        1.962_304_5,
        1.491_069_6,
        1.150_100_4,
        0.894_445_3,
        0.700_204_73,
        0.550_043_2,
        0.432_774_28,
        0.339_641_63,
        0.266_695_08,
        0.208_313_42,
        0.161_732_09,
        0.124_688_97,
        0.095_477_15,
        0.072_300_956,
        0.054_056_276,
        0.039_910_063,
        0.028_958_18,
        0.020_540_766,
        0.014_121_387,
        0.009_269_281,
        0.005_972_079,
        0.003_596_990_8,
        0.001_898_664_4,
        0.000_951_221_5,
        0.000_359_588_56,
        0.000_054_178_563,
    ];
    let out = temp_dir("p3_f2_chord").join("out.wav");
    render("p3_chord", &out, &[]);
    let audio = read_stereo(&out);
    assert_eq!(audio.frames(), 88200, "frame count");
    for (ch, samples) in audio.channels.iter().enumerate() {
        assert_eq!(samples[0], 16.75, "channel {ch}, frame 0");
        for (frame, (&got, want)) in samples[1..=TAIL.len()].iter().zip(TAIL).enumerate() {
            assert!(
                (got - want).abs() < 1e-5,
                "channel {ch}, frame {}: {got} != {want}",
                frame + 1
            );
        }
        assert!(
            samples[TAIL.len() + 1..].iter().all(|&s| s == 0.0),
            "channel {ch} must be silent after frame {}",
            TAIL.len()
        );
    }
}

// (g) Pitch from key on ramp4 (1, 0.5, 0.25, 0.125): key 72 plays at twice the speed and reads
// frames 0 and 2 as they are; key 48 at half speed reads every half frame, between frames by the
// 4-point Hermite at f = 1/2, (-y0 + 9 y1 + 9 y2 - y3)/16, a point before the first frame or
// past the last reading 0: (0 + 9 + 4.5 - 0.25)/16 = 0.828125, (-1 + 4.5 + 2.25 - 0.125)/16 =
// 0.3515625, (-0.5 + 2.25 + 1.125)/16 = 0.1796875, (-0.25 + 1.125)/16 = 0.0546875.
#[test]
fn p3_g_key_sets_the_playback_speed() {
    renders_exactly("p3_g_key72", 1, &[(0, 1.0), (1, 0.25)]);
    let octave_down = [
        (0, 1.0),
        (1, 0.828_125),
        (2, 0.5),
        (3, 0.351_562_5),
        (4, 0.25),
        (5, 0.179_687_5),
        (6, 0.125),
        (7, 0.054_687_5),
    ];
    renders_exactly("p3_g_key48", 1, &octave_down);
}

/// The first 16 hex digits of the SHA-256 of a file.
fn file_hash16(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let bytes = fs::read(path).expect("read rendered wav");
    Sha256::digest(&bytes)
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

// The byte-identical proof that playing pitch from key changed nothing at key 60: these are the
// hashes of the WAV files the last build before pitch existed rendered from the same fixtures.
// Every note in them is at key 60. A regression pin, not a known answer: the known answers for
// these files are the exact-value tests above. F2 is not on the list: its 44.1 kHz sample loads
// resampled to 48 kHz, which changed its render by design (its known answer is `f2_tempo_140_48k`).
#[test]
fn key_60_renders_are_byte_identical_to_the_build_before_pitch() {
    let pinned = [
        ("f1_four_on_floor", 1, "216f37a90a7842dd"),
        ("f1_four_on_floor", 2, "654f06eef92f0648"),
        ("f3_velocity_ramp", 1, "2665940ee443d0ce"),
        ("f4_saturated", 1, "6ecab19923b19a9b"),
        ("p2_three_patterns", 1, "216f37a90a7842dd"),
        ("p3_hold", 1, "e103a4f15b69836a"),
    ];
    let dir = temp_dir("key_60_byte_identical");
    for (fixture, bars, hash) in pinned {
        let out = dir.join(format!("{fixture}_{bars}.wav"));
        render(fixture, &out, &["--bars", &bars.to_string()]);
        assert_eq!(file_hash16(&out), hash, "{fixture}, {bars} bar(s)");
    }
}

// ---------------------------------------------------------------------------------------------
// Playlist and song mode: every expected frame below is printed by
// tests/fixtures/gen_p4_expected.py, which also wrote the p4_* fixtures. `--song` plays the
// playlist from tick 0; `--bars N` counts played bars, so the file is s(N bars) frames long however
// many times playback wraps (the loop range, or the song end with the loop off).
// ---------------------------------------------------------------------------------------------

/// Renders `fixture` in song mode for `bars` played bars with `extra` arguments.
fn render_song(fixture: &str, bars: u32, out: &Path, extra: &[&str]) {
    let bars = bars.to_string();
    let mut args = vec!["--song", "--bars", bars.as_str()];
    args.extend_from_slice(extra);
    render(fixture, out, &args);
}

/// A song-mode run: fixture, played bars, frames, and the frames that sound with their values.
type SongRun = (&'static str, u32, usize, &'static [(usize, f32)]);

/// Every song-mode run of the playlist known answers at 44.1 kHz, where the impulse loads as it is
/// (p4_messy, at 48 kHz, is [`P4_MESSY`]).
const P4_RUNS: [SongRun; 7] = [
    // Two clips of pattern 1 on track 1, bars 1 and 3: bar 2 is silent.
    (
        "p4_two_clips",
        3,
        264_600,
        &[
            (0, 1.0),
            (22050, 1.0),
            (44100, 1.0),
            (66150, 1.0),
            (176_400, 1.0),
            (198_450, 1.0),
            (220_500, 1.0),
            (242_550, 1.0),
        ],
    ),
    // The song ends at tick 1152 (sample 264600) and wraps to bar 1.
    (
        "p4_two_clips",
        4,
        352_800,
        &[
            (0, 1.0),
            (22050, 1.0),
            (44100, 1.0),
            (66150, 1.0),
            (176_400, 1.0),
            (198_450, 1.0),
            (220_500, 1.0),
            (242_550, 1.0),
            (264_600, 1.0),
            (286_650, 1.0),
            (308_700, 1.0),
            (330_750, 1.0),
        ],
    ),
    // A clip half its pattern's length plays the first half (192 and 288 are cut); a clip
    // twice its pattern's length repeats it (432 and 528).
    (
        "p4_resize",
        2,
        176_400,
        &[
            (0, 1.0),
            (11025, 1.0),
            (22050, 1.0),
            (33075, 1.0),
            (99225, 1.0),
            (121_275, 1.0),
        ],
    ),
    // Loop 384..768 on: bar 2 (the offbeats) plays twice, bar 3 never.
    (
        "p4_loop",
        3,
        264_600,
        &[
            (0, 1.0),
            (22050, 1.0),
            (44100, 1.0),
            (66150, 1.0),
            (99225, 1.0),
            (121_275, 1.0),
            (187_425, 1.0),
            (209_475, 1.0),
        ],
    ),
    // The same range switched off: bars 1, 2, 3 in order.
    (
        "p4_loop_off",
        3,
        264_600,
        &[
            (0, 1.0),
            (22050, 1.0),
            (44100, 1.0),
            (66150, 1.0),
            (99225, 1.0),
            (121_275, 1.0),
            (176_400, 1.0),
            (198_450, 1.0),
            (220_500, 1.0),
            (242_550, 1.0),
        ],
    ),
    // Two tracks sum: pattern 3 adds velocity 0.5 on ticks 0 and 192.
    (
        "p4_two_tracks",
        1,
        88200,
        &[(0, 1.5), (22050, 1.0), (44100, 1.5), (66150, 1.0)],
    ),
    // Saturated: two identical clips at 0 and a third at 192 all sound; the song ends at tick 576
    // (sample 132300) and wraps there.
    (
        "p4_overlap",
        2,
        176_400,
        &[
            (0, 2.0),
            (22050, 2.0),
            (44100, 3.0),
            (66150, 3.0),
            (88200, 1.0),
            (110_250, 1.0),
            (132_300, 2.0),
            (154_350, 2.0),
        ],
    ),
];

/// 48 kHz, 140 BPM, a clip at tick 37 cut at 287 (tick 325 is past its end), loop 133..500: the
/// frame at song sample 107142 plays song sample 28500. Its 44.1 kHz impulse loads resampled to the
/// project's rate, so each onset is the two-frame click gen_p4_expected.py prints.
const P4_MESSY: (&str, u32, usize, [usize; 6]) = (
    "p4_messy",
    2,
    164_571,
    [7928, 28500, 49071, 96214, 107_142, 127_713],
);

#[test]
fn p4_song_mode_plays_the_playlist() {
    for (fixture, bars, frames, expected) in P4_RUNS {
        let out = temp_dir(&format!("{fixture}_song_{bars}")).join("out.wav");
        render_song(fixture, bars, &out, &[]);
        assert_exact(&read_stereo(&out), frames, expected);
    }
    let (fixture, bars, frames, onsets) = P4_MESSY;
    let out = temp_dir(&format!("{fixture}_song_{bars}")).join("out.wav");
    render_song(fixture, bars, &out, &[]);
    assert_close(&read_stereo(&out), frames, &resampled_clicks(&onsets));
}

// Without --song the command line renders pattern 1, as before: F1's four beats.
#[test]
fn p4_without_song_renders_pattern_1() {
    let out = temp_dir("p4_two_clips_pattern").join("out.wav");
    render("p4_two_clips", &out, &[]);
    assert_exact(
        &read_stereo(&out),
        88200,
        &unit_impulses(&[0, 22050, 44100, 66150]),
    );
}

#[test]
fn p4_song_renders_do_not_depend_on_the_block_size() {
    let dir = temp_dir("p4_block_size_invariance");
    let messy = (P4_MESSY.0, P4_MESSY.1);
    let runs = P4_RUNS.iter().map(|&(fixture, bars, _, _)| (fixture, bars));
    for (fixture, bars) in runs.chain([messy]) {
        let default = dir.join(format!("{fixture}_{bars}.wav"));
        render_song(fixture, bars, &default, &[]);
        let expected = fs::read(&default).expect("read rendered wav");
        for block in ["1", "7", "64", "1000", "4096"] {
            let out = dir.join(format!("{fixture}_{bars}_block{block}.wav"));
            render_song(fixture, bars, &out, &["--block", block]);
            assert!(
                fs::read(&out).expect("read rendered wav") == expected,
                "{fixture}, {bars} bars: --block {block} differs from the default block"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Mixer: every expected value below is printed by tests/fixtures/gen_p5_expected.py, which also
// wrote the p5_* fixtures and the two sine files. Voices play into their channel's insert; inserts
// run in routing order (each after every insert that sends to it, the master last): effects,
// cube-law fader, pan, mute and solo, then each send. Channel pan is the circular law scaled by √2
// (center unity, the sides +3 dB).
// ---------------------------------------------------------------------------------------------

/// Renders `fixture` for `bars` bars with `extra` arguments and reads it back.
fn rendered(fixture: &str, bars: u32, extra: &[&str]) -> Rendered {
    let out = temp_dir(&format!("{fixture}_{bars}{}", extra.concat())).join("out.wav");
    let bars = bars.to_string();
    let mut args = vec!["--bars", bars.as_str()];
    args.extend_from_slice(extra);
    render(fixture, &out, &args);
    read_stereo(&out)
}

fn rms(samples: &[f32]) -> f64 {
    let sum: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (sum / samples.len() as f64).sqrt()
}

// 1. Pan law at 48 kHz, one channel per bar on the 24000-frame 1 kHz sine (amplitude 0.5):
// channel pan −1 gives gains (√2, 0), center (1, 1), +1 (0, √2); bar 4 plays centered into
// insert 1, panned hard left, which folds the right side into the left: (L + R, 0).
#[test]
fn p5_pan_follows_the_scaled_circular_law() {
    let audio = rendered("p5_pan", 4, &[]);
    assert_eq!(audio.frames(), 384_000);
    let expected = [
        (0.500_000_0, 0.0),
        (0.353_553_4, 0.353_553_4),
        (0.0, 0.500_000_0),
        // The generator's 0.7071068: the unit-amplitude sum of both sides, RMS 1/√2.
        (std::f64::consts::FRAC_1_SQRT_2, 0.0),
    ];
    for (bar, (left, right)) in expected.into_iter().enumerate() {
        let start = bar * 96_000;
        let sounding = start..start + 24_000;
        for (side, want) in [left, right].into_iter().enumerate() {
            let got = rms(&audio.channels[side][sounding.clone()]);
            assert!(
                (got - want).abs() < 1e-6,
                "bar {}, side {side}: RMS {got}, expected {want}",
                bar + 1
            );
        }
        for side in 0..2 {
            let rest = &audio.channels[side][start + 24_000..start + 96_000];
            assert!(
                rest.iter().all(|&s| s == 0.0),
                "bar {}, side {side}: silent after the sine",
                bar + 1
            );
        }
    }
}

// 2. Cube-law fader, unity at 0.8: fader 1.0 is 1.953125 (+5.81 dB), 0.63496 is
// 0.4999989867210388. Sends are linear and post-fader: insert 6 (fader 0.63496) sends 50% to
// insert 7, so the master gets 0.49999899 + 0.24999949 = 0.7499984502792358.
#[test]
fn p5_gain_applies_the_fader_law_and_post_fader_sends() {
    // The generator prints each f32 as the shortest f64 that names it; `as f32` is exact.
    let half = 0.499_998_986_721_038_8_f64 as f32;
    let post_fader = 0.749_998_450_279_235_8_f64 as f32;
    let expected = [
        (0, 1.953_125),
        (11025, post_fader),
        (22050, 1.0),
        (44100, half),
        (66150, 1.5),
    ];
    assert_exact(&rendered("p5_gain", 1, &[]), 88200, &expected);
}

// 3. Mute and solo: insert 2 is muted; insert 5 sends only to 3, which sends to the master and
// to 4. Soloing 3 keeps 5 (upstream) and 4 (downstream) audible and silences insert 1.
#[test]
fn p5_mute_and_solo_silence_the_right_inserts() {
    let muted = [(0, 1.0), (11025, 2.0), (44100, 2.0), (66150, 1.0)];
    assert_exact(&rendered("p5_mute", 1, &[]), 88200, &muted);
    let soloed = [(11025, 2.0), (44100, 2.0), (66150, 1.0)];
    assert_exact(&rendered("p5_solo", 1, &[]), 88200, &soloed);
}

/// The amplitude of the `freq` Hz component of `samples[lo..hi]` at 48 kHz: the projection
/// onto sine and cosine over a whole number of periods.
fn amplitude(samples: &[f32], freq: f64, lo: usize, hi: usize) -> f64 {
    let (mut s, mut c) = (0.0, 0.0);
    for (n, &y) in samples.iter().enumerate().take(hi).skip(lo) {
        let phase = 2.0 * std::f64::consts::PI * freq * n as f64 / 48_000.0;
        s += f64::from(y) * phase.sin();
        c += f64::from(y) * phase.cos();
    }
    2.0 * s.hypot(c) / (hi - lo) as f64
}

// 4. A peaking band at 1 kHz, +6 dB, Q 1 (the cookbook biquad): |H| = 1.9952623 at 1 kHz and
// 1.0913575 at 3 kHz, so the amplitude-0.5 probes come out at 0.9976312 and 0.5456788.
#[test]
fn p5_eq_peaking_band_has_the_cookbook_response() {
    for (fixture, freq, want) in [
        ("p5_eq_f0", 1000.0, 0.997_631_2),
        ("p5_eq_3k", 3000.0, 0.545_678_8),
    ] {
        let audio = rendered(fixture, 1, &[]);
        for (side, samples) in audio.channels.iter().enumerate() {
            let got = amplitude(samples, freq, 6000, 18_000);
            assert!(
                (got - want).abs() < 1e-4,
                "{fixture}, side {side}: amplitude {got}, expected {want}"
            );
        }
        assert!(
            audio.channels[0] == audio.channels[1],
            "{fixture}: both sides equal"
        );
    }
}

// 5. Delay: 10 ms at 44.1 kHz is 441 frames; feedback 0.5, mix 0.5 puts 0.5 on frame 0 and
// 0.5^k on frame 441k. Synced to 3 steps (tick 72) the delay is 16537 frames, so echo 2 lands
// on 33074, not on tick 144's 33075.
#[test]
fn p5_delay_echoes_land_on_exact_frames() {
    let audio = rendered("p5_delay_ms", 1, &[]);
    assert_eq!(audio.frames(), 88200);
    for (side, samples) in audio.channels.iter().enumerate() {
        for (frame, &got) in samples.iter().enumerate().take(9261) {
            let want = match frame {
                0 => 0.5,
                f if f % 441 == 0 => 0.5f32.powi(i32::try_from(f / 441).expect("small")),
                _ => 0.0,
            };
            assert_eq!(got, want, "p5_delay_ms side {side}, frame {frame}");
        }
    }
    assert_exact(&rendered("p5_delay_sync", 1, &[]), 88200, &SYNCED_ECHOES);
}

/// p5_delay_sync's echoes: a 16537-frame delay with feedback 0.5 at mix 1.
const SYNCED_ECHOES: [(usize, f32); 5] = [
    (16537, 1.0),
    (33074, 0.5),
    (49611, 0.25),
    (66148, 0.125),
    (82685, 0.0625),
];

// A project saved with the former native-id prefix keeps its effects: p5_delay_sync saved that
// way echoes on the same frames.
#[test]
fn a_native_effect_saved_under_the_former_prefix_still_plays() {
    let dir = temp_dir("former_native_prefix");
    let fixtures = repo_root().join("tests/fixtures");
    let text = fs::read_to_string(fixtures.join("p5_delay_sync.json")).expect("fixture");
    assert!(
        text.contains(NATIVE_ID_PREFIX),
        "the fixture has a native effect"
    );
    let project = dir.join("former.json");
    fs::write(
        &project,
        text.replace(NATIVE_ID_PREFIX, FORMER_NATIVE_ID_PREFIX),
    )
    .expect("written");
    fs::copy(fixtures.join("impulse.wav"), dir.join("impulse.wav")).expect("sample copied");
    let out = dir.join("out.wav");
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .arg("render")
        .arg(&project)
        .arg(&out)
        .args(["--bars", "1"])
        .output()
        .expect("run dawsome binary");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_exact(&read_stereo(&out), 88200, &SYNCED_ECHOES);
}

// 6. Reverb: nothing comes out before the shortest comb (at least 25 ms, 1103 frames), the tail
// is still there half a second later and decays, and the sides differ; at mix 0 the insert
// passes its input through bit for bit.
#[test]
fn p5_reverb_tail_is_delayed_decays_and_spreads() {
    let audio = rendered("p5_reverb", 1, &[]);
    assert_eq!(audio.frames(), 88200);
    for (side, samples) in audio.channels.iter().enumerate() {
        assert!(
            samples[..1102].iter().all(|&s| s == 0.0),
            "side {side}: silent before the shortest comb"
        );
        assert!(
            samples[22050..].iter().any(|s| s.abs() > 1e-4),
            "side {side}: the tail reaches past half a second"
        );
        let (early, late) = (rms(&samples[4410..22050]), rms(&samples[44100..66150]));
        assert!(
            late < early,
            "side {side}: RMS {late} later, {early} earlier"
        );
    }
    assert!(audio.channels[0] != audio.channels[1], "the sides differ");
    let dir = temp_dir("p5_reverb_dry");
    let bytes = |fixture: &str| {
        let out = dir.join(format!("{fixture}.wav"));
        render(fixture, &out, &[]);
        fs::read(&out).expect("read rendered wav")
    };
    assert!(
        bytes("p5_reverb_dry") == bytes("p5_reverb_none"),
        "mix 0 changes the sound"
    );
}

// 7. Routing order: 3 → 2 → 1 → master plays through all three in one block, so the click is
// on frame 0 (mixing in id order would leave it in insert 3's bus until the next block).
#[test]
fn p5_chain_mixes_in_routing_order() {
    assert_exact(&rendered("p5_chain", 1, &[]), 88200, &[(0, 1.0)]);
}

// 7. A routing cycle is refused at load: exit 1, the cycle named, no file written.
#[test]
fn p5_cycle_is_refused_and_named() {
    let out = temp_dir("p5_cycle").join("out.wav");
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .args(["render", "tests/fixtures/p5_cycle.json"])
        .arg(&out)
        .output()
        .expect("run dawsome binary");
    assert_eq!(output.status.code(), Some(1));
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(message.contains("routing cycle"), "{message}");
    assert!(message.contains("1 → 2 → 3 → 1"), "{message}");
    assert!(!out.exists(), "a refused project writes no file");
}

// 8. Saturated: 16 channels at volume 4.0 through faders at 1.0 into the master at 1.0:
// 4 × 1.953125 = 7.8125 per insert, × 16 = 125.0, × 1.953125 = 244.140625, written unclamped.
#[test]
fn p5_saturated_mix_is_written_unclamped() {
    assert_exact(
        &rendered("p5_saturated", 1, &[]),
        88200,
        &[(0, 244.140_625_f64 as f32)],
    );
}

/// Every mixer render as (fixture, bars).
const P5_RUNS: [(&str, u32); 13] = [
    ("p5_pan", 4),
    ("p5_gain", 1),
    ("p5_mute", 1),
    ("p5_solo", 1),
    ("p5_eq_f0", 1),
    ("p5_eq_3k", 1),
    ("p5_delay_ms", 1),
    ("p5_delay_sync", 1),
    ("p5_reverb", 1),
    ("p5_reverb_dry", 1),
    ("p5_reverb_none", 1),
    ("p5_chain", 1),
    ("p5_saturated", 1),
];

#[test]
fn p5_renders_do_not_depend_on_the_block_size() {
    let dir = temp_dir("p5_block_size_invariance");
    for (fixture, bars) in P5_RUNS {
        let bars = bars.to_string();
        let default = dir.join(format!("{fixture}.wav"));
        render(fixture, &default, &["--bars", &bars]);
        let expected = fs::read(&default).expect("read rendered wav");
        for block in ["1", "7", "64", "1000", "4096"] {
            let out = dir.join(format!("{fixture}_block{block}.wav"));
            render(fixture, &out, &["--bars", &bars, "--block", block]);
            assert!(
                fs::read(&out).expect("read rendered wav") == expected,
                "{fixture}: --block {block} differs from the default block"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Files: export bit depths and MP3, and a missing sample played silent.
// ---------------------------------------------------------------------------------------------

/// Runs `dawsome <args>` from the repository root.
fn dawsome(args: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .args(args)
        .output()
        .expect("run dawsome binary")
}

/// Reads an integer stereo WAV of `bits` bits back as left and right samples.
fn read_int_stereo(path: &Path, bits: u16) -> (Vec<i32>, Vec<i32>) {
    let mut reader = hound::WavReader::open(path).expect("open rendered wav");
    let spec = reader.spec();
    assert_eq!(
        (spec.channels, spec.sample_format, spec.bits_per_sample),
        (2, hound::SampleFormat::Int, bits)
    );
    let samples: Vec<i32> = reader
        .samples::<i32>()
        .collect::<Result<_, _>>()
        .expect("decode samples");
    let left = samples.iter().copied().step_by(2).collect();
    let right = samples.iter().copied().skip(1).step_by(2).collect();
    (left, right)
}

// (g) F1 one bar: 88200 frames, 1.0 at 0, 22050, 44100, 66150 -> the integer top at 16 (plain
// rounding, --no-dither) and 24 bits, every other value 0; dithered (16 bits by default) the tops
// stay and the rest is gen_p25_expected.py's dither alone, value for value; 32-bit float is
// today's render byte for byte; p3_chord's frame 0 (16.75) clips.
#[test]
fn export_bit_depths_quantize_the_render() {
    use sha2::Digest;
    let dir = temp_dir("export_bits");
    for (bits, name, top, plain) in [
        (16, "16", 32_767, &["--no-dither"][..]),
        (24, "24", 8_388_607, &[][..]),
    ] {
        let out = dir.join(format!("f1_{name}.wav"));
        let mut args = vec!["--bits", name];
        args.extend_from_slice(plain);
        render("f1_four_on_floor", &out, &args);
        let (left, right) = read_int_stereo(&out, bits);
        assert_eq!(left.len(), 88_200);
        assert_eq!(left, right);
        let lit: Vec<(usize, i32)> = left
            .iter()
            .enumerate()
            .filter(|(_, &v)| v != 0)
            .map(|(at, &v)| (at, v))
            .collect();
        assert_eq!(lit, [(0, top), (22_050, top), (44_100, top), (66_150, top)]);
    }
    let want = &p25_expected()["dither"]["f1_16"];
    let dithered = dir.join("f1_16_dithered.wav");
    render("f1_four_on_floor", &dithered, &["--bits", "16"]);
    let (left, right) = read_int_stereo(&dithered, 16);
    for beat in want["beats"].as_array().expect("beats") {
        let at = json_usize(beat);
        assert_eq!((left[at], right[at]), (32_767, 32_767), "the beat at {at}");
    }
    let values: Vec<i32> = left
        .iter()
        .zip(&right)
        .flat_map(|(&l, &r)| [l, r])
        .collect();
    let nonzero = values.iter().filter(|&&v| v != 0).count() as u64;
    assert_eq!(nonzero, want["nonzero"].as_u64().expect("nonzero"));
    let hash: String = sha2::Sha256::digest(
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<u8>>(),
    )
    .iter()
    .map(|byte| format!("{byte:02x}"))
    .collect();
    assert_eq!(
        hash,
        want["sha256"].as_str().expect("sha256"),
        "f1 dithered"
    );
    let (default, float) = (dir.join("default.wav"), dir.join("float.wav"));
    render("f1_four_on_floor", &default, &[]);
    render("f1_four_on_floor", &float, &["--bits", "32f"]);
    assert!(fs::read(&default).expect("read") == fs::read(&float).expect("read"));
    let chord = dir.join("chord.wav");
    render("p3_chord", &chord, &["--bits", "16"]);
    let (left, _) = read_int_stereo(&chord, 16);
    assert_eq!(left[0], 32_767);
    let output = dawsome(&[
        "render".as_ref(),
        "tests/fixtures/f1_four_on_floor.json".as_ref(),
        dir.join("x.wav").as_os_str(),
        "--bits".as_ref(),
        "20".as_ref(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "an unknown depth is a usage error"
    );
}

// MP3 has no exact length without a gapless tag: the decoder sees the encoder delay and the padding
// to whole 1152-frame packets on top of the 88200 rendered frames.
#[test]
fn mp3_export_decodes_to_about_the_rendered_length() {
    let dir = temp_dir("export_mp3");
    let out = dir.join("f1.mp3");
    render("f1_four_on_floor", &out, &[]);
    let decoded = dawsome_media::decode::decode(&out).expect("decodes");
    assert_eq!((decoded.sample_rate, decoded.channels), (44_100, 2));
    let frames = decoded.frames();
    assert!(
        (88_200..88_200 + 3_456).contains(&frames),
        "{frames} frames"
    );
}

// A sample that is not there plays silence, the render goes on, and stderr says so.
#[test]
fn a_missing_sample_plays_silence_with_a_notice() {
    let dir = temp_dir("missing_sample");
    let project = dir.join("f1.json");
    fs::copy(
        repo_root().join("tests/fixtures/f1_four_on_floor.json"),
        &project,
    )
    .expect("copy");
    let out = dir.join("out.wav");
    let output = dawsome(&["render".as_ref(), project.as_os_str(), out.as_os_str()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(
        message.contains("impulse.wav") && message.contains("plays silence"),
        "{message}"
    );
    assert_exact(&read_stereo(&out), 88_200, &[]);
}

// ---------------------------------------------------------------------------------------------
// Plugins: every expected frame below is printed by tests/fixtures/gen_p7_expected.py, which also
// wrote the p7_* fixtures. The plugins are Dawsome's own test plugins (crates/test-plugin), whose
// output formulas are exact, built into <target dir>/test-plugins by scripts/build-test-plugins.sh.
// Every run passes --plugin-path and a fresh --cache, so no test reads or writes the user's
// catalogue.
// ---------------------------------------------------------------------------------------------

#[path = "../../../tests/support/test_plugins.rs"]
mod test_plugins;

use test_plugins::{test_plugin_variants, test_plugins};

/// Runs `dawsome render <fixture> <out>` with the test plugins and a catalogue in `dir`, plus
/// `extra`; returns the run for its status and stderr.
fn render_with_plugins(
    fixture: &str,
    dir: &Path,
    out: &Path,
    extra: &[&str],
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .arg("render")
        .arg(format!("tests/fixtures/{fixture}.json"))
        .arg(out)
        .arg("--plugin-path")
        .arg(test_plugins())
        .arg("--cache")
        .arg(dir.join("catalogue.json"))
        .args(extra)
        .output()
        .expect("run dawsome binary")
}

/// As [`render_with_plugins`], requiring success, and reads the render back.
fn rendered_with_plugins(fixture: &str, dir: &Path, extra: &[&str]) -> Rendered {
    let out = dir.join(format!("{fixture}{}.wav", extra.concat()));
    let output = render_with_plugins(fixture, dir, &out, extra);
    assert!(
        output.status.success(),
        "dawsome render {fixture} {extra:?} failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    read_stereo(&out)
}

// 1. Test sine at gain 0.5 (its state), one note at tick 24 (frame 6000) for 96 ticks (to frame
// 30000), key 69 (440 Hz), velocity 1.0, at 48 kHz: 0.5·cos(2π·440·n/48000) from 6000 on.
#[test]
fn p7_sine_starts_on_its_frame_with_the_formula_amplitude() {
    let dir = temp_dir("p7_sine");
    let audio = rendered_with_plugins("p7_sine", &dir, &["--bars", "1"]);
    assert_eq!(audio.frames(), 96_000);
    // The generator's nine significant digits, kept as written (f64), compared within 1e-6.
    let expected: [(usize, f64); 6] = [
        (6000, 0.5),
        (6001, 0.499_170_899),
        (6002, 0.496_686_369),
        (6003, 0.492_554_665),
        (7200, 0.5),
        (29_999, 0.499_170_899),
    ];
    for (side, samples) in audio.channels.iter().enumerate() {
        for &(frame, value) in &expected {
            assert!(
                (f64::from(samples[frame]) - value).abs() < 1e-6,
                "side {side}, frame {frame}: {} instead of {value}",
                samples[frame]
            );
        }
        assert!(
            samples[..6000].iter().all(|&s| s == 0.0),
            "side {side}: sound before the note"
        );
        assert!(
            samples[30_000..].iter().all(|&s| s == 0.0),
            "side {side}: sound after the note-off"
        );
        let level = rms(&samples[6000..30_000]);
        assert!(
            (level - 0.353_553_4).abs() < 1e-7,
            "side {side}: RMS {level}"
        );
    }
}

// 4. A missing instrument plays nothing and a missing effect passes its insert through: only the
// kick's four impulses (ticks 0, 96, 192, 288 at 44.1 kHz) sound, both plugins are named on
// stderr, and the render succeeds.
#[test]
fn p7_missing_plugins_are_silent_or_pass_through_and_named() {
    let dir = temp_dir("p7_missing");
    let out = dir.join("out.wav");
    let output = render_with_plugins("p7_missing", &dir, &out, &[]);
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{message}");
    for named in [
        "Missing plugin: Missing Synth (clap:org.example.missing)",
        "Missing plugin: Missing FX (clap:org.example.missing-fx)",
    ] {
        assert!(message.contains(named), "{named:?} not in {message}");
    }
    assert_exact(
        &read_stereo(&out),
        88_200,
        &unit_impulses(&[0, 22050, 44100, 66150]),
    );
}

/// The catalogue `dawsome scan` wrote, as JSON.
fn read_catalogue(path: &Path) -> serde_json::Value {
    let text = fs::read_to_string(path).expect("the scan writes the catalogue");
    serde_json::from_str(&text).expect("the catalogue is JSON")
}

/// How a scan records the test crash plugin, whose entry aborts: by the signal on Linux, by the
/// exception status an aborted process exits with on Windows.
const ABORTED: &str = if cfg!(windows) {
    "crashed: exception 0xC0000409"
} else {
    "crashed: signal 6"
};

/// Runs `dawsome scan` over `plugins` into `cache`, requiring exit 0, and returns stdout.
fn scan(plugins: &Path, cache: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .arg("scan")
        .arg("--plugin-path")
        .arg(plugins)
        .arg("--cache")
        .arg(cache)
        .output()
        .expect("run dawsome binary");
    assert!(
        output.status.success(),
        "dawsome scan failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

// 5. The scan loads each file in a child process: the aborting file kills its child only, and is
// recorded as failed with the signal; the good file lists both plugins with their kinds. An
// unchanged file is not loaded again; a touched one is.
#[test]
fn p7_scan_survives_a_crashing_plugin_and_rescans_only_changed_files() {
    let dir = temp_dir("p7_scan");
    let plugins = dir.join("plugins");
    fs::create_dir_all(&plugins).expect("plugin dir");
    for file in ["dawsome_test_plugin.clap", "dawsome_test_crash.clap"] {
        fs::copy(test_plugins().join(file), plugins.join(file)).expect("copy test plugin");
    }
    let cache = dir.join("catalogue.json");
    let first = scan(&plugins, &cache);
    assert!(first.contains("2 files: 2 scanned, 0 unchanged"), "{first}");
    let catalogue = read_catalogue(&cache);
    assert_eq!(catalogue["version"], 1);
    let files = catalogue["files"].as_array().expect("files");
    let names: Vec<&str> = files
        .iter()
        .map(|file| file["file"].as_str().expect("path"))
        .collect();
    assert_eq!(names.len(), 2);
    assert!(names[0].ends_with("dawsome_test_crash.clap"), "{names:?}");
    assert!(names[1].ends_with("dawsome_test_plugin.clap"), "{names:?}");
    assert_eq!(files[0]["status"], "failed");
    assert_eq!(files[0]["reason"], ABORTED);
    assert_eq!(files[1]["status"], "ok");
    let plugins_found: Vec<(&str, &str)> = files[1]["plugins"]
        .as_array()
        .expect("plugins")
        .iter()
        .map(|plugin| {
            (
                plugin["id"].as_str().expect("id"),
                plugin["kind"].as_str().expect("kind"),
            )
        })
        .collect();
    assert_eq!(
        plugins_found,
        [
            ("org.dawsome.test.gain", "effect"),
            ("org.dawsome.test.sine", "instrument")
        ]
    );
    let second = scan(&plugins, &cache);
    assert!(
        second.contains("2 files: 0 scanned, 2 unchanged"),
        "{second}"
    );
    let good = fs::File::options()
        .write(true)
        .open(plugins.join("dawsome_test_plugin.clap"))
        .expect("open the good file");
    good.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
        .expect("touch the good file");
    let third = scan(&plugins, &cache);
    assert!(third.contains("2 files: 1 scanned, 1 unchanged"), "{third}");
    assert!(third.contains("dawsome_test_plugin.clap"), "{third}");
}

// A plugin file replaced after its scan (here by the file that aborts as it loads) is never
// loaded into the render's own process: its record no longer matches the file, so a child scans
// it again and finds it broken, and the render plays without the plugin and says so.
#[test]
fn p7_a_plugin_file_changed_after_its_scan_is_not_loaded_in_process() {
    let dir = temp_dir("p7_stale");
    let plugins = dir.join("plugins");
    fs::create_dir_all(&plugins).expect("plugin dir");
    let file = plugins.join("dawsome_test_plugin.clap");
    fs::copy(test_plugins().join("dawsome_test_plugin.clap"), &file).expect("copy test plugin");
    let cache = dir.join("catalogue.json");
    scan(&plugins, &cache);
    fs::copy(test_plugins().join("dawsome_test_crash.clap"), &file).expect("replace it");
    let out = dir.join("out.wav");
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .args(["render", "tests/fixtures/p7_sine.json"])
        .arg(&out)
        .arg("--plugin-path")
        .arg(&plugins)
        .arg("--cache")
        .arg(&cache)
        .output()
        .expect("run dawsome binary");
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{}: {message}", output.status);
    assert!(
        message.contains("Missing plugin: Dawsome Test Sine (clap:org.dawsome.test.sine)"),
        "{message}"
    );
    assert_exact(&read_stereo(&out), 96_000, &[]);
}

// A plugin that fails part way through a render (the fail variant, from its fifth block on) is
// named once, however many blocks it fails, and the render goes on without it: the instrument
// stays silent for all 20 bars (1920000 frames at 48 kHz and 120 BPM).
#[test]
fn p7_a_plugin_failing_mid_render_is_named_once() {
    let dir = temp_dir("p7_fail");
    let project = dir.join("p7_fail.json");
    let text = fs::read_to_string(repo_root().join("tests/fixtures/p7_sine.json"))
        .expect("reads")
        .replace(
            r#""id":"clap:org.dawsome.test.sine","name":"Dawsome Test Sine""#,
            r#""id":"clap:org.dawsome.test.fail","name":"Dawsome Test Fail""#,
        );
    fs::write(&project, text).expect("writes");
    let out = dir.join("out.wav");
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .arg("render")
        .arg(&project)
        .arg(&out)
        .arg("--plugin-path")
        .arg(test_plugin_variants())
        .arg("--cache")
        .arg(dir.join("catalogue.json"))
        .args(["--bars", "20"])
        .output()
        .expect("run dawsome binary");
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{}: {message}", output.status);
    let named = "dawsome: warning: Plugin failed: Dawsome Test Fail: it failed while processing\n";
    assert_eq!(message.matches(named).count(), 1, "{message}");
    assert!(!message.contains("a plugin it"), "{message}");
    assert_exact(&read_stereo(&out), 1_920_000, &[]);
}

// 6. The test plugin is split-invariant, so any --block gives the bytes of the default one: the
// host places every note and parameter on its exact frame. A block other than 512 is allowed with a
// warning.
#[test]
fn p7_renders_are_deterministic_and_do_not_depend_on_the_block_size() {
    let dir = temp_dir("p7_determinism");
    let render_bytes = |name: &str, extra: &[&str]| {
        let out = dir.join(format!("{name}.wav"));
        let output = render_with_plugins("p7_sine", &dir, &out, extra);
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        (
            fs::read(&out).expect("read render"),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    let (first, quiet) = render_bytes("first", &[]);
    let (second, _) = render_bytes("second", &[]);
    assert!(first == second, "two default renders differ");
    assert!(!quiet.contains("block size"), "{quiet}");
    for block in ["1", "7", "64", "4096"] {
        let (bytes, warning) = render_bytes(&format!("block{block}"), &["--block", block]);
        assert!(bytes == first, "--block {block} differs from the default");
        assert!(
            warning.contains("plugin output may depend on the block size"),
            "--block {block}: {warning}"
        );
    }
}

// 7. The test gain at 0.5 in insert 1's first slot halves the impulse; at 1.0 it renders the
// bytes of the same project with no effect.
#[test]
fn p7_effect_plugin_processes_its_insert() {
    let dir = temp_dir("p7_fx");
    let halved = rendered_with_plugins("p7_fx", &dir, &[]);
    assert_exact(&halved, 88_200, &[(0, 0.5)]);
    let bytes = |fixture: &str| {
        let out = dir.join(format!("{fixture}.wav"));
        let output = render_with_plugins(fixture, &dir, &out, &[]);
        assert!(
            output.status.success(),
            "{fixture}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        fs::read(&out).expect("read render")
    };
    assert!(
        bytes("p7_fx_unity") == bytes("p7_fx_none"),
        "gain 1.0 changes the render"
    );
}

// ---------------------------------------------------------------------------------------------
// Audio clips and automation: every expected value below is read from
// tests/fixtures/p8_expected.json, which tests/fixtures/gen_p8_expected.py writes with the p8_*
// fixtures and their sound files. Audio clips play their file from `frame_offset` on, in song mode;
// automation sets a channel's volume per sample through the same parameter the knob sets; a file at
// another rate loads resampled; long files stream from disk.
// ---------------------------------------------------------------------------------------------

/// The audio-clip and automation known answers.
fn p25_expected() -> serde_json::Value {
    let path = repo_root().join("tests/fixtures/p25_expected.json");
    let text = fs::read_to_string(&path).expect("p25_expected.json (run gen_p25_expected.py)");
    serde_json::from_str(&text).expect("p25_expected.json parses")
}

fn p8_expected() -> serde_json::Value {
    let path = repo_root().join("tests/fixtures/p8_expected.json");
    let text = fs::read_to_string(&path).expect("p8_expected.json (run gen_p8_expected.py)");
    serde_json::from_str(&text).expect("p8_expected.json parses")
}

fn json_usize(value: &serde_json::Value) -> usize {
    value
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .expect("a frame count")
}

fn json_f32(value: &serde_json::Value) -> f32 {
    // The generator writes each f32 as the shortest f64 that names it, so `as f32` is exact.
    value.as_f64().expect("a number") as f32
}

/// Renders `fixture` in song mode for `bars` bars with `extra` arguments and reads it back.
fn rendered_song(fixture: &str, bars: u32, extra: &[&str]) -> Rendered {
    let out = temp_dir(&format!("{fixture}_song{bars}{}", extra.concat())).join("out.wav");
    render_song(fixture, bars, &out, extra);
    read_stereo(&out)
}

/// The mono frames of a fixture WAV written by gen_p8_expected.py, as f32.
fn fixture_frames(name: &str) -> Vec<f32> {
    let mut reader =
        hound::WavReader::open(repo_root().join("tests/fixtures").join(name)).expect("opens");
    let spec = reader.spec();
    assert_eq!(spec.channels, 1, "{name} is mono");
    match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .expect("reads"),
        // 16-bit PCM is v / 32768, exact in f32.
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|v| v.map(|v| f32::from(v) / 32768.0))
            .collect::<Result<_, _>>()
            .expect("reads"),
    }
}

/// Asserts both channels are `expected` frame for frame, bit for bit.
fn assert_frames(audio: &Rendered, expected: &[f32], what: &str) {
    assert_eq!(audio.frames(), expected.len(), "{what}: frame count");
    for (ch, samples) in audio.channels.iter().enumerate() {
        for (i, (&got, &want)) in samples.iter().zip(expected).enumerate() {
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "{what}: channel {ch}, frame {i}: {got} != {want}"
            );
        }
    }
}

// (1) Placement, 48 kHz at 140 BPM: the 1000-frame ramp clip at bar 2 starts exactly at
// s(384) = 82285 and ends after its last frame; nothing else sounds.
#[test]
fn p8_place_starts_an_audio_clip_on_its_bar() {
    let expected = &p8_expected()["place"];
    let mut frames = vec![0.0f32; json_usize(&expected["frames"])];
    let first = json_usize(&expected["first"]);
    for (k, value) in expected["values"]
        .as_array()
        .expect("values")
        .iter()
        .enumerate()
    {
        frames[first + k] = json_f32(value);
    }
    assert_frames(&rendered_song("p8_place", 2, &[]), &frames, "p8_place");
}

// (2) A trimmed clip (100 frames into the file, one beat long), a clip at gain 0.5 and a muted
// clip, with the song-end wrap at 66150 reading by position again.
#[test]
fn p8_trim_offsets_gains_and_mutes_audio_clips() {
    let expected = &p8_expected()["trim"];
    let saw = fixture_frames("p8_saw.wav");
    let mut frames = vec![0.0f32; json_usize(&expected["frames"])];
    for run in expected["runs"].as_array().expect("runs") {
        let (from, to) = (json_usize(&run[0]), json_usize(&run[1]));
        let (file_from, gain) = (json_usize(&run[2]), json_f32(&run[3]));
        for (n, frame) in frames.iter_mut().enumerate().take(to).skip(from) {
            let value = saw.get(file_from + n - from).copied().unwrap_or(0.0);
            *frame = value * gain;
        }
    }
    for spot in expected["spots"].as_array().expect("spots") {
        assert_eq!(
            frames[json_usize(&spot[0])],
            json_f32(&spot[1]),
            "spot {spot}"
        );
    }
    assert_frames(&rendered_song("p8_trim", 1, &[]), &frames, "p8_trim");
}

// (3) A 48 kHz file in a 44.1 kHz project loads resampled to 11025 frames; the clip (48 ticks,
// the song) repeats eight times in a bar. Each pass has its peaks within a frame of 918.75,
// 4410 and 8820, in the files' 2 : 1 ratio, and nothing farther than 256 frames from them. A
// resampler whose delay was left in would move every peak.
#[test]
fn p8_resample_loads_a_48k_file_at_the_project_rate() {
    let expected = &p8_expected()["resample"];
    let audio = rendered_song("p8_resample", 1, &[]);
    assert_eq!(audio.frames(), json_usize(&expected["frames"]));
    assert!(audio.channels[0] == audio.channels[1], "both sides equal");
    let samples = &audio.channels[0];
    let pass = json_usize(&expected["pass"]);
    let peaks: Vec<usize> = expected["peaks"]
        .as_array()
        .expect("peaks")
        .iter()
        .map(json_usize)
        .collect();
    for start in (0..samples.len()).step_by(pass) {
        let at = |frame: usize| f64::from(samples[start + frame].abs());
        let found: Vec<usize> = peaks
            .iter()
            .map(|&peak| {
                let window = peak - 64..=peak + 64;
                let loudest = window
                    .max_by(|&a, &b| at(a).total_cmp(&at(b)))
                    .expect("a window");
                assert!(
                    loudest.abs_diff(peak) <= 1,
                    "pass at {start}: peak {loudest}, expected {peak}"
                );
                loudest
            })
            .collect();
        let ratio = at(found[1]) / at(found[2]);
        assert!((ratio - 2.0).abs() < 1e-5, "pass at {start}: ratio {ratio}");
        for frame in 0..pass {
            if peaks.iter().all(|&peak| frame.abs_diff(peak) > 256) {
                assert!(
                    at(frame) < 0.01,
                    "pass at {start}, frame {frame}: {}",
                    at(frame)
                );
            }
        }
    }
}

/// A render of `fixture` that sounds only on the steps `expected` lists, each at its value
/// (exactly, or within 1e-6 where an exponential segment gave it), and is silent elsewhere.
fn assert_steps(fixture: &str, expected: &serde_json::Value) {
    let audio = rendered_song(fixture, 3, &[]);
    assert_eq!(audio.frames(), json_usize(&expected["frames"]));
    let steps = expected["steps"].as_array().expect("steps");
    for (ch, samples) in audio.channels.iter().enumerate() {
        let mut silent = vec![true; samples.len()];
        for step in steps {
            let (frame, want) = (json_usize(&step[0]), json_f32(&step[1]));
            let exact = step[2].as_bool().expect("exactness");
            let got = samples[frame];
            if exact {
                assert_eq!(got, want, "{fixture} channel {ch}, frame {frame}");
            } else {
                assert!(
                    (got - want).abs() <= 1e-6,
                    "{fixture} channel {ch}, frame {frame}: {got} != {want}"
                );
            }
            silent[frame] = false;
        }
        for (frame, &got) in samples.iter().enumerate() {
            if silent[frame] {
                assert_eq!(got, 0.0, "{fixture} channel {ch}, frame {frame}");
            }
        }
    }
}

// (4) Linear channel-volume automation (range 0..4): the stored volume 0.5 rules bar 1, before the
// clip; the line runs 1.0 to 0.0 over bar 2; bar 3 holds the end value 0.0.
#[test]
fn p8_linear_automation_rules_its_window_and_holds_after_it() {
    assert_steps("p8_auto_linear", &p8_expected()["auto_linear"]);
}

// (5) Hold, smooth and exp(0.5) over three bars: smooth is exactly 0.75 at its midpoint, and
// exp at frame 220500 is 1 − 1/(e^1.5 + 1).
#[test]
fn p8_hold_smooth_and_exponential_segments_follow_their_formulas() {
    assert_steps("p8_auto_curves", &p8_expected()["auto_curves"]);
}

// (7) Every clip streamed from disk (--stream-threshold 0, offline renders wait for the disk)
// renders the bytes of the in-memory render, also at --block 7: overlapping clips of one file,
// the file's end at 30000, and a loop wrap that seeks.
#[test]
fn p8_streamed_clips_render_the_bytes_of_memory() {
    let expected = &p8_expected()["stream"];
    let dir = temp_dir("p8_stream_vs_memory");
    let bytes = |name: &str, extra: &[&str]| {
        let out = dir.join(format!("{name}.wav"));
        render_song("p8_stream", 4, &out, extra);
        fs::read(&out).expect("read rendered wav")
    };
    let memory = bytes("memory", &[]);
    assert!(
        bytes("stream", &["--stream-threshold", "0"]) == memory,
        "streaming differs"
    );
    assert!(
        bytes("stream7", &["--stream-threshold", "0", "--block", "7"]) == memory,
        "streaming at --block 7 differs"
    );
    let audio = read_stereo(&dir.join("memory.wav"));
    assert_eq!(audio.frames(), json_usize(&expected["frames"]));
    for spot in expected["spots"].as_array().expect("spots") {
        let frame = json_usize(&spot[0]);
        for (ch, samples) in audio.channels.iter().enumerate() {
            assert_eq!(
                samples[frame],
                json_f32(&spot[1]),
                "channel {ch}, frame {frame}"
            );
        }
    }
}

/// Every audio-clip and automation song render as (fixture, bars).
const P8_RUNS: [(&str, u32); 6] = [
    ("p8_place", 2),
    ("p8_trim", 1),
    ("p8_resample", 1),
    ("p8_auto_linear", 3),
    ("p8_auto_curves", 3),
    ("p8_stream", 4),
];

#[test]
fn p8_renders_do_not_depend_on_the_block_size() {
    let dir = temp_dir("p8_block_size_invariance");
    for (fixture, bars) in P8_RUNS {
        let default = dir.join(format!("{fixture}.wav"));
        render_song(fixture, bars, &default, &[]);
        let expected = fs::read(&default).expect("read rendered wav");
        for block in ["1", "7", "64", "1000", "4096"] {
            let out = dir.join(format!("{fixture}_block{block}.wav"));
            render_song(fixture, bars, &out, &["--block", block]);
            assert!(
                fs::read(&out).expect("read rendered wav") == expected,
                "{fixture}: --block {block} differs from the default block"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// MIDI files and stretch: SMF files through the command line, clip stretch rendered into the cache,
// and the ported saturator on an insert. Expected values: tests/fixtures/p9_expected.json
// (tests/fixtures/gen_p9_expected.py) and tests/fixtures/ref.
// ---------------------------------------------------------------------------------------------

fn p9_expected() -> serde_json::Value {
    let path = repo_root().join("tests/fixtures/p9_expected.json");
    let text = fs::read_to_string(&path).expect("p9_expected.json (run gen_p9_expected.py)");
    serde_json::from_str(&text).expect("p9_expected.json parses")
}

/// Runs `dawsome <args>` and requires success; returns its stderr.
fn dawsome_ok(args: &[&std::ffi::OsStr]) -> String {
    let output = dawsome(args);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "dawsome {args:?} failed: {stderr}");
    stderr
}

/// Pattern 1's notes of a project file as JSON values, keys as the file writes them.
fn pattern_1_notes(path: &Path) -> serde_json::Value {
    let text = fs::read_to_string(path).expect("project reads");
    let project: serde_json::Value = serde_json::from_str(&text).expect("project parses");
    project["patterns"][0]["notes"].clone()
}

// (1) and (2): `smf-import` puts the file's notes on pattern 1 / channel 1 (ticks rescaled from
// PPQ 480 to 96, the pattern grown to 2 bars) and reports what it left out; `smf-export` of the
// result writes the known 85 bytes.
#[test]
fn p9_smf_import_and_export_run_through_the_command_line() {
    let expected = p9_expected();
    let dir = temp_dir("p9_smf");
    let (imported, mid) = (dir.join("imported.json"), dir.join("exported.mid"));
    let stderr = dawsome_ok(&[
        "smf-import".as_ref(),
        "tests/fixtures/p9_import_base.json".as_ref(),
        "tests/fixtures/p9_import.mid".as_ref(),
        imported.as_os_str(),
        "--pattern".as_ref(),
        "1".as_ref(),
        "--channel".as_ref(),
        "1".as_ref(),
    ]);
    assert!(stderr.contains("1 overlapping note dropped"), "{stderr}");
    assert!(stderr.contains("tempo 120 BPM not applied"), "{stderr}");
    let notes = pattern_1_notes(&imported);
    let want = &expected["import"]["notes"];
    assert_eq!(
        notes.as_array().map(Vec::len),
        want.as_array().map(Vec::len)
    );
    for (got, want) in notes
        .as_array()
        .expect("notes")
        .iter()
        .zip(want.as_array().expect("notes"))
    {
        for key in ["channel", "tick", "length", "key"] {
            assert_eq!(got.get(key), want.get(key), "{key} of {got}");
        }
        assert_eq!(
            got["velocity"].as_f64().map(|v| v as f32),
            want["velocity"].as_f64().map(|v| v as f32)
        );
    }
    let text = fs::read_to_string(&imported).expect("reads");
    assert!(text.contains(r#""length_bars":2"#), "{text}");
    dawsome_ok(&["smf-export".as_ref(), imported.as_os_str(), mid.as_os_str()]);
    let bytes = fs::read(&mid).expect("the export");
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(hex, expected["export"]["hex"].as_str().expect("hex"));
}

/// Renders `fixture` in song mode for one bar with the stretch cache in `cache`.
fn rendered_stretch(fixture: &str, dir: &Path, cache: &Path, extra: &[&str]) -> Rendered {
    let out = dir.join(format!("{fixture}{}.wav", extra.concat()));
    let mut args = vec!["--cache-dir", cache.to_str().expect("utf-8 path")];
    args.extend_from_slice(extra);
    render_song(fixture, 1, &out, &args);
    read_stereo(&out)
}

/// The one stretched file in `cache`, as its frames (left channel) and its bytes.
fn cached_file(cache: &Path) -> (Vec<f32>, Vec<u8>) {
    let files: Vec<PathBuf> = fs::read_dir(cache)
        .expect("the cache folder exists")
        .map(|entry| entry.expect("entry").path())
        .collect();
    assert_eq!(
        files.len(),
        1,
        "one cached file, no partial left: {files:?}"
    );
    let mut reader = hound::WavReader::open(&files[0]).expect("a WAV");
    let channels = usize::from(reader.spec().channels);
    let values: Vec<f32> = reader
        .samples::<f32>()
        .collect::<Result<_, _>>()
        .expect("reads");
    let left = values.iter().copied().step_by(channels).collect();
    (left, fs::read(&files[0]).expect("bytes"))
}

// (5) The resample stretch mode on the clicks (1.0 every 441 frames), read as a pitched voice
// reads (the 4-point Hermite): to 44100 frames they fall every 882 frames with 9/16 either side
// and -1/16 a frame further; to 33075 frames by thirds (gen_p9_expected.py's values). Every other
// frame of the bar is exactly 0, the same at --block 1, 7 and 4096.
#[test]
fn p9_resample_mode_reads_the_exact_positions() {
    let expected = p9_expected();
    let dir = temp_dir("p9_resample");
    for fixture in ["p9_resample", "p9_resample15"] {
        let want = &expected[fixture.trim_start_matches("p9_")];
        let mut frames = vec![0.0f32; json_usize(&want["frames"])];
        for pair in want["nonzero"].as_array().expect("nonzero") {
            frames[json_usize(&pair[0])] = json_f32(&pair[1]);
        }
        let cache = dir.join(format!("{fixture}-cache"));
        let audio = rendered_stretch(fixture, &dir, &cache, &[]);
        assert_frames(&audio, &frames, fixture);
        let whole = fs::read(dir.join(format!("{fixture}.wav"))).expect("render");
        for block in ["1", "7", "4096"] {
            rendered_stretch(fixture, &dir, &cache, &["--block", block]);
            let split = fs::read(dir.join(format!("{fixture}--block{block}.wav"))).expect("render");
            assert!(split == whole, "{fixture}: --block {block} differs");
        }
    }
}

/// The frequency of `wav`'s left channel over frames `from..to`, measured by the generator
/// (rising zero crossings, linearly interpolated).
fn measured_hz(wav: &Path, from: usize, to: usize) -> f64 {
    // Windows installs Python 3 as `python`; its `python3` may be a store shortcut that runs
    // nothing.
    let python = if cfg!(windows) { "python" } else { "python3" };
    let output = Command::new(python)
        .current_dir(repo_root())
        .args(["-I", "tests/fixtures/gen_p9_expected.py", "--frequency"])
        .arg(wav)
        .args([from.to_string(), to.to_string()])
        .output()
        .expect("python runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .expect("a frequency")
}

// (6) Stretch at ratio 1.5, pitch 0: the cached file holds exactly 66150 frames, the render is
// silent from there, 440 Hz stays 440 ± 1 Hz over the middle half, and a second fresh cache is
// byte-identical (below ratio 2 the library is deterministic).
#[test]
fn p9_stretch_keeps_the_pitch_and_rebuilds_its_cache_byte_for_byte() {
    let want = &p9_expected()["stretch"];
    let dir = temp_dir("p9_stretch");
    let (first, second) = (dir.join("cache-a"), dir.join("cache-b"));
    let audio = rendered_stretch("p9_stretch", &dir, &first, &[]);
    let stretched = json_usize(&want["stretched"]);
    assert_eq!(audio.frames(), json_usize(&want["frames"]));
    let (cached, bytes) = cached_file(&first);
    assert_eq!(cached.len(), stretched, "the cached file's frames");
    for (ch, samples) in audio.channels.iter().enumerate() {
        assert!(
            samples[stretched..].iter().all(|&value| value == 0.0),
            "channel {ch} sounds past the stretched file"
        );
        assert!(samples[..stretched].iter().any(|&value| value != 0.0));
    }
    let hz = measured_hz(
        &dir.join("p9_stretch.wav"),
        stretched / 4,
        3 * stretched / 4,
    );
    let (target, tolerance) = (
        want["hz"].as_f64().expect("hz"),
        want["hz_tolerance"].as_f64().expect("tolerance"),
    );
    assert!((hz - target).abs() <= tolerance, "{hz} Hz");
    rendered_stretch("p9_stretch", &dir, &second, &[]);
    assert!(
        cached_file(&second).1 == bytes,
        "a second cache build differs"
    );
}

// (6) Stretch with pitch +7 at ratio 1: 440 Hz becomes 659.255 Hz, within 10 cents.
#[test]
fn p9_stretch_with_pitch_shifts_up_a_fifth() {
    let want = &p9_expected()["stretch_pitch"];
    let dir = temp_dir("p9_stretch_pitch");
    rendered_stretch("p9_stretch_pitch", &dir, &dir.join("cache"), &[]);
    let stretched = json_usize(&want["stretched"]);
    let hz = measured_hz(
        &dir.join("p9_stretch_pitch.wav"),
        stretched / 4,
        3 * stretched / 4,
    );
    let target = want["hz"].as_f64().expect("hz");
    let cents = 1200.0 * (hz / target).log2();
    assert!(
        cents.abs() <= want["cents_tolerance"].as_f64().expect("tolerance"),
        "{hz} Hz is {cents} cents from {target}"
    );
}

// (7) The mono input on a channel into insert 1's Density {0.6, 0.3, 0.8, 0.7}, rendered at 480
// BPM for one bar (22050 frames), nulls against the original's render: max |diff| ≤ 2^-20 and
// no sample differs.
#[test]
fn p9_density_on_an_insert_nulls_against_the_original() {
    let dir = temp_dir("p9_density_cli");
    let out = dir.join("out.wav");
    render("p9_density_cli", &out, &["--bars", "1"]);
    let audio = read_stereo(&out);
    let reference = read_stereo(&repo_root().join("tests/fixtures/ref/density_cli_44k.wav"));
    assert_eq!(
        audio.frames(),
        json_usize(&p9_expected()["density_cli"]["frames"])
    );
    assert_eq!(audio.frames(), reference.frames());
    let (mut largest, mut differing) = (0.0f64, 0usize);
    for (ours, theirs) in audio.channels.iter().zip(&reference.channels) {
        for (a, b) in ours.iter().zip(theirs) {
            largest = largest.max((f64::from(*a) - f64::from(*b)).abs());
            differing += usize::from(a != b);
        }
    }
    assert!(
        largest <= 2f64.powi(-20) && differing == 0,
        "max |diff| {largest:e}, {differing} samples differ"
    );
}

// ---------------------------------------------------------------------------------------------
// The demo project (examples/demo.json, made by examples/gen_demo.py with its own synthesized
// sounds). Its song's first bar plays pattern 1 alone: kick, snare and hat, each on an insert of
// its own. Every sound starts with a step far taller than any sample-to-sample step inside the
// sounds, so a hit is where the output steps by more than the generator's threshold, and those
// places are the onsets the generator computed from the note ticks by the tick-to-sample rule.

fn demo_expected() -> serde_json::Value {
    let path = repo_root().join("examples/demo_expected.json");
    let text = fs::read_to_string(&path).expect("demo_expected.json (run examples/gen_demo.py)");
    serde_json::from_str(&text).expect("demo_expected.json parses")
}

#[test]
fn the_demo_songs_first_bar_hits_where_its_notes_are() {
    let want = demo_expected();
    let out = temp_dir("demo_first_bar").join("out.wav");
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .args(["render", "examples/demo.json"])
        .arg(&out)
        .args(["--song", "--bars", "1"])
        .output()
        .expect("run dawsome binary");
    assert!(
        output.status.success(),
        "dawsome render examples/demo.json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let audio = read_stereo(&out);
    assert_eq!(audio.frames(), json_usize(&want["bar_frames"]));
    let threshold = want["threshold"].as_f64().expect("threshold");
    let stepped = |frame: usize| {
        audio.channels.iter().any(|samples| {
            let before = frame
                .checked_sub(1)
                .map_or(0.0, |previous| samples[previous]);
            (f64::from(samples[frame]) - f64::from(before)).abs() > threshold
        })
    };
    let onsets: Vec<usize> = (0..audio.frames())
        .filter(|&frame| stepped(frame))
        .collect();
    let expected: Vec<usize> = want["onsets"]
        .as_array()
        .expect("onsets")
        .iter()
        .map(json_usize)
        .collect();
    assert_eq!(onsets, expected);
}

// ---------------------------------------------------------------------------------------------
// The per-user cache folder. Without --cache and --cache-dir, the plugin catalogue and stretched
// clips go to dawsome/ in it: $DAWSOME_CACHE_HOME on every platform, ahead of the platform's own
// folder; on Linux, $XDG_CACHE_HOME still moves it when ours is not set.
// ---------------------------------------------------------------------------------------------

/// `dawsome scan` of an empty plugin folder and `dawsome render` of p9_stretch, with neither
/// --cache nor --cache-dir, and only the cache variables of `env` set.
fn run_with_default_caches(dir: &Path, env: &[(&str, &Path)]) {
    let plugins = dir.join("plugins");
    fs::create_dir_all(&plugins).expect("plugin dir");
    let mut scan = Command::new(env!("CARGO_BIN_EXE_dawsome"));
    scan.arg("scan").arg("--plugin-path").arg(&plugins);
    let mut render = Command::new(env!("CARGO_BIN_EXE_dawsome"));
    render
        .current_dir(repo_root())
        .arg("render")
        .arg("tests/fixtures/p9_stretch.json")
        .arg(dir.join("p9_stretch.wav"))
        .args(["--song", "--bars", "1"]);
    for command in [&mut scan, &mut render] {
        command
            .env_remove("DAWSOME_CACHE_HOME")
            .env_remove("XDG_CACHE_HOME");
        for (name, value) in env {
            command.env(name, value);
        }
        let output = command.output().expect("run dawsome binary");
        assert!(
            output.status.success(),
            "{command:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// How many stretched files the cache folder `home` holds.
fn stretched_files(home: &Path) -> usize {
    fs::read_dir(home.join("dawsome").join("stretch"))
        .expect("the stretch folder exists")
        .count()
}

#[test]
fn dawsome_cache_home_holds_the_catalogue_and_stretched_clips() {
    // Absolute, as the variable must be: a test directory named from another platform's build
    // (a cross-built test run under a compatibility layer) is only rooted, not absolute.
    let dir = std::path::absolute(temp_dir("cache_home")).expect("an absolute path");
    let (ours, platform) = (dir.join("ours"), dir.join("platform"));
    run_with_default_caches(
        &dir,
        &[("DAWSOME_CACHE_HOME", &ours), ("XDG_CACHE_HOME", &platform)],
    );
    assert!(ours.join("dawsome").join("clap-catalogue.json").is_file());
    assert_eq!(stretched_files(&ours), 1);
    assert!(!platform.exists(), "the platform's folder is not used");
}

// Linux only: XDG_CACHE_HOME is the Linux platform's own variable.
#[cfg(target_os = "linux")]
#[test]
fn on_linux_xdg_cache_home_moves_the_cache_folder_too() {
    let dir = temp_dir("xdg_cache_home");
    let platform = dir.join("platform");
    run_with_default_caches(&dir, &[("XDG_CACHE_HOME", &platform)]);
    assert!(platform
        .join("dawsome")
        .join("clap-catalogue.json")
        .is_file());
    assert_eq!(stretched_files(&platform), 1);
}
