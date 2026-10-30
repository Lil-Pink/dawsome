//! The master limiter through the command line: projects with the Limiter in the master's
//! first slot render the frames tests/fixtures/gen_p24_expected.py computes from the rule it
//! states (p24_expected.json), to the bit and whatever the block size; and an instrument zone's
//! loop at another rate than its file's wraps where the file's loop does. Every run gets
//! per-user folders of its own under the test's temp dir.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha256};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> PathBuf {
    repo_root().join("tests/fixtures")
}

/// Fresh empty directory under cargo's per-target test directory.
fn temp_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("master_limiter")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p24_expected.json"))
        .expect("p24_expected.json (run gen_p24_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("parses")
}

/// `dawsome render <fixture> <dir>/<name>.wav --bits 32f <extra>`, every per-user folder under
/// `dir`; the frames it wrote, left and right.
fn render(fixture: &str, dir: &Path, name: &str, extra: &[&str]) -> Vec<[f32; 2]> {
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
        .arg(fixtures().join(fixture))
        .arg(&out)
        .args(["--bits", "32f"])
        .args(extra)
        .output()
        .expect("run dawsome");
    assert!(
        output.status.success(),
        "dawsome render {fixture} failed: {}",
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

/// Asserts `frames` is render `name` of the generator: length, probes and sounding frames to the
/// bit, the hash of every frame, and the spec's own words: no frame past the ceiling, the first
/// frame the mix took past it exactly on it.
fn assert_render(name: &str, frames: &[[f32; 2]], want: &Value) {
    assert_eq!(
        frames.len() as u64,
        want["frames"].as_u64().expect("frames"),
        "{name}: frames"
    );
    for probe in want["probes"].as_array().expect("probes") {
        let at = probe[0].as_u64().expect("a frame") as usize;
        let side = |index: usize| number(&probe[index]) as f32;
        assert_eq!(
            frames[at].map(f32::to_bits),
            [side(1), side(2)].map(f32::to_bits),
            "{name}, frame {at}: {:?} != [{}, {}]",
            frames[at],
            side(1),
            side(2)
        );
    }
    let sounding = frames.iter().filter(|f| **f != [0.0; 2]).count() as u64;
    assert_eq!(
        sounding,
        want["sounding"].as_u64().expect("sounding"),
        "{name}"
    );
    assert_eq!(
        frames_hash(frames),
        want["sha256"].as_str().expect("sha256"),
        "{name}: the hash of every frame"
    );
    let ceiling = number(&want["ceiling"]) as f32;
    let peak = frames
        .iter()
        .flat_map(|frame| frame.map(f32::abs))
        .fold(0.0f32, f32::max);
    assert!(peak <= ceiling, "{name}: {peak} past the ceiling {ceiling}");
    if let Some([at, left, right]) = want["first_over"].as_array().map(Vec::as_slice) {
        let at = at.as_u64().expect("a frame") as usize;
        let on = frames[at].map(f32::abs).into_iter().fold(0.0f32, f32::max);
        assert_eq!(
            on, ceiling,
            "{name}: the first frame over lands on the ceiling"
        );
        assert_eq!(frames[at], [number(left) as f32, number(right) as f32]);
    }
}

/// Known answers (gen_p24_expected.py): a +6 dBFS burst, a chord of two full-scale notes, a
/// +20 dBFS spike, a DC step to the ceiling, the ceiling at 0 dB with a 1 ms release, and
/// silence, each limited frame by frame as the rule says.
#[test]
fn p24_the_limiter_holds_every_mix_to_its_ceiling() {
    let want = expected();
    let dir = temp_dir("renders");
    for (name, answer) in want["renders"].as_object().expect("renders") {
        let fixture = answer["fixture"].as_str().expect("fixture");
        let frames = render(fixture, &dir, name, &[]);
        assert_render(name, &frames, answer);
    }
}

/// Known answers: a mix under the ceiling throughout, and the DC step to the ceiling exactly,
/// pass bit for bit; the under-ceiling mix is the render of the same project with no limiter.
#[test]
fn p24_a_mix_under_the_ceiling_passes_bit_for_bit() {
    let want = expected();
    let dir = temp_dir("under");
    for name in ["under", "dc", "silence"] {
        assert_eq!(want["renders"][name]["unchanged"], true, "{name}");
    }
    let limited = render("p24_under.json", &dir, "limited", &[]);
    let plain_fixture = want["same_as"]["under"].as_str().expect("plain");
    let plain = render(plain_fixture, &dir, "plain", &[]);
    assert_eq!(frames_hash(&limited), frames_hash(&plain));
    let silent = render("p24_silence.json", &dir, "silence", &[]);
    assert!(silent.iter().all(|f| f.map(f32::to_bits) == [0, 0]));
}

/// The limiter's state runs frame by frame, so block sizes 1, 7, 64 and 4096 give the 512
/// render's bytes.
#[test]
fn p24_the_limiter_renders_the_same_bytes_at_any_block_size() {
    let want = expected();
    let dir = temp_dir("blocks");
    for name in want["blocks"]["cases"].as_array().expect("cases") {
        let name = name.as_str().expect("a case");
        let fixture = want["renders"][name]["fixture"].as_str().expect("fixture");
        let reference = frames_hash(&render(fixture, &dir, name, &[]));
        for size in want["blocks"]["sizes"].as_array().expect("sizes") {
            let size = size.as_u64().expect("a size").to_string();
            let frames = render(
                fixture,
                &dir,
                &format!("{name}-{size}"),
                &["--block", &size],
            );
            assert_eq!(frames_hash(&frames), reference, "{name} at --block {size}");
        }
    }
}

/// Known answer (gen_p24_expected.py, found while the 808 sets were made): a 44.1 kHz file's loop
/// of 2205 frames at 96 kHz is 4800 frames long, so the render repeats every 4800 frames from the
/// loop's start on, to the bit (a loop one frame short repeats every 4799).
#[test]
fn p24_a_loop_at_96k_wraps_every_whole_converted_length() {
    let want = &expected()["loop96"];
    let dir = temp_dir("loop96");
    let frames = render(want["fixture"].as_str().expect("fixture"), &dir, "up", &[]);
    assert_eq!(
        frames.len() as u64,
        want["frames"].as_u64().expect("frames")
    );
    let period = want["period"].as_u64().expect("period") as usize;
    assert_eq!(Some(period as u64), want["period_exact"].as_u64());
    let from = want["loop"][0].as_u64().expect("loop start") as usize;
    assert!(frames[from..from + period].iter().any(|f| *f != [0.0; 2]));
    for at in from..frames.len() - period {
        assert_eq!(
            frames[at].map(f32::to_bits),
            frames[at + period].map(f32::to_bits),
            "frame {at} and one loop later"
        );
    }
}

/// Known answer (gen_p24_expected.py, found while the 808 sets were made): a 48 kHz sine loop of
/// whole periods that ends on its file's last frame plays at 44.1 kHz as the sine, to 1e-6 at every
/// frame around each wrap and every 50th between (the conversion once bent its last frames: a -80
/// dB step at each wrap of this sine, -50 dB on the 808s).
#[test]
fn p24_a_loop_to_the_files_last_frame_wraps_without_a_step() {
    let want = &expected()["looplast"];
    let dir = temp_dir("looplast");
    let frames = render(
        want["fixture"].as_str().expect("fixture"),
        &dir,
        "last",
        &[],
    );
    assert_eq!(
        frames.len() as u64,
        want["frames"].as_u64().expect("frames")
    );
    let tolerance = number(&want["tolerance"]);
    for probe in want["probes"].as_array().expect("probes") {
        let at = probe[0].as_u64().expect("a frame") as usize;
        let ideal = number(&probe[1]);
        for side in frames[at] {
            assert!(
                (f64::from(side) - ideal).abs() <= tolerance,
                "frame {at}: {side} is not the sine's {ideal}"
            );
        }
    }
}

/// |y[k + 1] − 2 y[k] + y[k − 1]| for each frame k, 0 at the first and the last.
fn second_differences(y: &[f64]) -> Vec<f64> {
    let mut d = vec![0.0; y.len()];
    for k in 1..y.len().saturating_sub(1) {
        d[k] = (y[k + 1] - 2.0 * y[k] + y[k - 1]).abs();
    }
    d
}

/// The generator's seam measure: the largest, over every wrap but the last, of its |second
/// difference| (the largest of the 4 frames from 2 before it) over the largest in the middle
/// half of the lap after it, up to the next wrap.
fn seam_ratio(y: &[f64], wraps: &[usize]) -> f64 {
    let d = second_differences(y);
    let largest = |frames: std::ops::Range<usize>| frames.map(|k| d[k]).fold(0.0, f64::max);
    wraps
        .windows(2)
        .map(|pair| {
            let (wrap, next) = (pair[0], pair[1]);
            let quarter = (next - wrap) / 4;
            largest(wrap - 2..wrap + 2) / largest(wrap + quarter..next - quarter)
        })
        .fold(0.0, f64::max)
}

/// The frequency of `y`'s rising zero crossings from frame `start` on, each placed between its
/// two frames on the straight line through them (the generator's).
fn crossing_hz(y: &[f64], rate: f64, start: usize) -> f64 {
    let at: Vec<f64> = (start..y.len() - 1)
        .filter(|&k| y[k] < 0.0 && 0.0 <= y[k + 1])
        .map(|k| k as f64 + y[k] / (y[k] - y[k + 1]))
        .collect();
    (at.len() - 1) as f64 * rate / (at[at.len() - 1] - at[0])
}

/// Known answer (gen_p24_expected.py, found rendering FreePats' 44.1 kHz loops at 48 kHz): a
/// loop that is no whole number of frames at the project's rate (300 file frames are 326.53 at
/// 48 kHz from 44.1 kHz and 275.625 at 44.1 kHz from 48 kHz) wraps every 300 of its file's
/// frames, as at the file's own rate: the render is the generator's to the bit at any block size,
/// no wrap's second difference passes 1.5 times the loop's own (a loop rounded to whole frames
/// slipped about half a frame a wrap: 9 times), and the sine keeps its pitch within 1 cent
/// (rounded: 2.8 and 3.9 cents sharp). The same of a file above the project's rate, band
/// limited on load (loop441, alias96, seam96; 0.5 cent for the last two): seam96's file goes on
/// with another sound past the loop's end, which filtered over the file's own frames rang into
/// every wrap (40 times the loop's own); its loop is filtered as the voice plays it, on and on.
#[test]
fn p24_a_loop_no_whole_frames_long_at_the_project_rate_wraps_where_its_file_does() {
    for case in ["loop48", "loop441", "alias96", "seam96"] {
        let want = &expected()[case];
        let fixture = want["fixture"].as_str().expect("fixture");
        let dir = temp_dir(case);
        let frames = render(fixture, &dir, case, &[]);
        assert_eq!(
            frames.len() as u64,
            want["frames"].as_u64().expect("frames"),
            "{case}"
        );
        let wraps: Vec<usize> = want["wraps"]
            .as_array()
            .expect("wraps")
            .iter()
            .map(|w| w.as_u64().expect("a frame") as usize)
            .collect();
        let left: Vec<f64> = frames.iter().map(|f| f64::from(f[0])).collect();
        let seam = seam_ratio(&left, &wraps);
        assert!(
            seam <= number(&want["seam_bound"]),
            "{case}: a wrap's second difference is {seam} times the loop's own"
        );
        let hz = crossing_hz(&left, number(&want["rate"]), wraps[0]);
        let cents = 1200.0 * (hz / number(&want["hz"])).log2();
        assert!(
            cents.abs() <= number(&want["cents_tolerance"]),
            "{case}: the loop plays {hz} Hz, {cents} cents off {}",
            want["hz"]
        );
        let hash = frames_hash(&frames);
        assert_eq!(hash, want["sha256"].as_str().expect("sha256"), "{case}");
        for size in want["blocks"].as_array().expect("blocks") {
            let size = size.as_u64().expect("a size").to_string();
            let split = render(
                fixture,
                &dir,
                &format!("{case}-{size}"),
                &["--block", &size],
            );
            assert_eq!(frames_hash(&split), hash, "{case} at --block {size}");
        }
    }
}

/// The level of `y`'s component at `hz` over `frames` (whole periods of it), dBFS of its
/// amplitude: 2/n |Σ y[t] e^(−i 2π hz t / rate)| (the generator's).
fn bin_db(y: &[f64], hz: f64, rate: f64, frames: std::ops::Range<usize>) -> f64 {
    let n = frames.len() as f64;
    let (re, im) = frames.fold((0.0, 0.0), |(re, im), t| {
        let angle = 2.0 * std::f64::consts::PI * hz * t as f64 / rate;
        (re + y[t] * angle.cos(), im - y[t] * angle.sin())
    });
    20.0 * (2.0 * re.hypot(im) / n).log10()
}

/// Known answer (gen_p24_expected.py's alias96, the review's probe): a 96 kHz file of a 19.2 kHz
/// and a 32 kHz tone at 0.25 each, its loop 1005 frames (502.5 at 48 kHz, so its zone reads the
/// file's own frames, at speed 2), played at 48 kHz. Read unfiltered, a frame in two kept, the
/// 32 kHz tone folded to 16 kHz at its full level (-12.0 dBFS); band limited on load it is gone
/// (below -90 dBFS) and the 19.2 kHz tone keeps its level (-12.04 dBFS, within 0.5 dB).
#[test]
fn p24_a_file_above_the_project_rate_folds_nothing_down() {
    let want = &expected()["alias96"];
    let levels = &want["levels"];
    let dir = temp_dir("alias96-levels");
    let frames = render(
        want["fixture"].as_str().expect("fixture"),
        &dir,
        "alias96",
        &[],
    );
    let left: Vec<f64> = frames.iter().map(|f| f64::from(f[0])).collect();
    let window = levels["window"].as_array().expect("window");
    let start = window[0].as_u64().expect("a frame") as usize;
    let window = start..start + window[1].as_u64().expect("frames") as usize;
    let rate = number(&want["rate"]);
    let (tone, fold) = (&levels["tone"], &levels["fold"]);
    let tone_db = bin_db(&left, number(&tone["hz"]), rate, window.clone());
    assert!(
        (tone_db - number(&tone["db"])).abs() <= number(&tone["tolerance_db"]),
        "the {} Hz tone is at {tone_db} dBFS, not {}",
        tone["hz"],
        tone["db"]
    );
    let fold_db = bin_db(&left, number(&fold["hz"]), rate, window);
    assert!(
        fold_db < number(&fold["below_db"]),
        "the 32 kHz tone folds to {} Hz at {fold_db} dBFS",
        fold["hz"]
    );
}
