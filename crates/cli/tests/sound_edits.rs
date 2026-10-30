//! A sound's edits through the command line (the sample editor, P1a): each render case of
//! tests/fixtures/gen_p30_expected.py renders the frames it states (p30_expected.json) through
//! `dawsome render`, the same bytes at any block size; a whole-sound reverse renders the bytes
//! of the sound setting's reverse; a clip moved by a crop or a delete renders what it rendered
//! before; a ten-minute edited sound is made once into the edited cache, streamed from there and
//! found there again; the DAWproject export says the edits stay behind; and no sound file is
//! ever written. Every run gets per-user folders of its own under the test's temp dir.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime};

use serde_json::Value;
use sha2::{Digest, Sha256};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> PathBuf {
    repo_root().join("tests/fixtures")
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p30_expected.json"))
        .expect("p30_expected.json (run gen_p30_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("parses")
}

/// Fresh empty directory under cargo's per-target test directory.
fn temp_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("sound_edits")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Runs `dawsome <args>` with every per-user folder under `home`, and requires success.
fn dawsome(args: &[&std::ffi::OsStr], home: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .env("DAWSOME_DATA_HOME", home.join("data"))
        .env("DAWSOME_CACHE_HOME", home.join("cache"))
        .env("DAWSOME_CONFIG_HOME", home.join("config"))
        .env("DAWSOME_STATE_HOME", home.join("state"))
        .args(args)
        .output()
        .expect("run dawsome");
    assert!(
        output.status.success(),
        "dawsome {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// `dawsome render <project> <out> [--song] <extra>`, stretched clips kept under `home`.
fn render(project: &Path, out: &Path, song: bool, extra: &[&str], home: &Path) {
    let cache = home.join("stretch");
    let mut args: Vec<&std::ffi::OsStr> = vec![
        "render".as_ref(),
        project.as_os_str(),
        out.as_os_str(),
        "--cache-dir".as_ref(),
        cache.as_os_str(),
    ];
    if song {
        args.push("--song".as_ref());
    }
    args.extend(extra.iter().map(std::ffi::OsStr::new));
    dawsome(&args, home);
}

/// A render's values, left and right interleaved.
fn samples(path: &Path) -> Vec<f32> {
    let mut reader = hound::WavReader::open(path).expect("open rendered wav");
    assert_eq!(reader.spec().channels, 2);
    reader
        .samples::<f32>()
        .map(|v| v.expect("decodes"))
        .collect()
}

fn sha_of(values: &[f32]) -> String {
    let mut hash = Sha256::new();
    for value in values {
        hash.update(value.to_le_bytes());
    }
    hex(&hash.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A file's SHA-256 and time: what "never written" is checked by.
fn fingerprint(path: &Path) -> (String, SystemTime) {
    let bytes = fs::read(path).expect("a sound");
    let time = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .expect("a time");
    (hex(&Sha256::digest(&bytes)), time)
}

/// Every p30 sound, fingerprinted.
fn sounds() -> Vec<(PathBuf, (String, SystemTime))> {
    let mut sounds: Vec<PathBuf> = fs::read_dir(fixtures().join("p30_sound"))
        .expect("the sounds")
        .map(|entry| entry.expect("an entry").path())
        .collect();
    sounds.sort();
    sounds
        .into_iter()
        .map(|path| {
            let print = fingerprint(&path);
            (path, print)
        })
        .collect()
}

/// Asserts the render in `values` is the case's: its frames, their bits by the generator's
/// SHA-256, and (to say where when they differ) every sounding frame or its probes.
fn assert_render(name: &str, values: &[f32], want: &Value) {
    assert_eq!(
        values.len() as u64,
        2 * want["frames"].as_u64().expect("frames"),
        "{name}"
    );
    let entries = want
        .get("nonzero")
        .or_else(|| want.get("probes"))
        .and_then(Value::as_array)
        .expect("frames to compare");
    for entry in entries {
        let at = entry[0].as_u64().expect("a frame") as usize;
        let side = |index: usize| entry[index].as_f64().expect("a value") as f32;
        assert_eq!(
            [values[2 * at].to_bits(), values[2 * at + 1].to_bits()],
            [side(1).to_bits(), side(2).to_bits()],
            "{name}, frame {at}"
        );
    }
    let sounding = values
        .chunks_exact(2)
        .filter(|frame| frame[0] != 0.0 || frame[1] != 0.0)
        .count();
    assert_eq!(
        sounding as u64,
        want["sounding"].as_u64().expect("count"),
        "{name}"
    );
    assert_eq!(
        sha_of(values),
        want["sha256"].as_str().expect("sha256"),
        "{name}"
    );
}

// (1)-(6), the stretch-of-edited path: every render case through the real entry point, a
// sampler of an edited sound at keys 60 and 67, an audio clip of one, a resampled clip of one;
// a whole-sound reverse to the bytes of the sound setting's reverse at both keys; no sound file
// written by any of it.
#[test]
fn p30_edited_sounds_render_their_known_answers() {
    let before = sounds();
    let want = expected();
    let dir = temp_dir("known");
    let mut bytes = std::collections::BTreeMap::new();
    for (name, case) in want["renders"].as_object().expect("renders") {
        let project = fixtures().join(case["fixture"].as_str().expect("fixture"));
        let out = dir.join(format!("{name}.wav"));
        render(&project, &out, case["song"] == true, &[], &dir);
        assert_render(name, &samples(&out), case);
        bytes.insert(name.clone(), fs::read(&out).expect("the render"));
    }
    for key in ["60", "67"] {
        assert!(
            bytes[&format!("reverse{key}")] == bytes[&format!("reverse{key}_setting")],
            "key {key}: the edited reverse renders other bytes than Sound.reverse"
        );
    }
    assert_eq!(sounds(), before, "a sound file was written");
}

// (12) An edited clip and an edited sampler render the same bytes at --block 1, 7, 64 and 4096
// as at the default 512.
#[test]
fn p30_edited_sounds_render_the_same_bytes_at_any_block() {
    let dir = temp_dir("blocks");
    for (fixture, song) in [
        ("p30_clip.json", true),
        ("p30_stretch.json", true),
        ("p30_crop.json", false),
    ] {
        let project = fixtures().join(fixture);
        let whole = dir.join(format!("{fixture}.wav"));
        render(&project, &whole, song, &[], &dir);
        let whole = fs::read(&whole).expect("the render");
        for block in ["1", "7", "64", "4096"] {
            let split = dir.join(format!("{fixture}-{block}.wav"));
            render(&project, &split, song, &["--block", block], &dir);
            assert!(
                fs::read(&split).expect("the render") == whole,
                "{fixture}: --block {block} renders other bytes"
            );
        }
    }
}

// (7) A clip moved by a crop or a delete in front of it (and a stretched clip, its ratio kept)
// renders the bytes it rendered before: the same audio from the edited sound.
#[test]
fn p30_a_moved_clip_renders_what_it_rendered_before() {
    let dir = temp_dir("remap");
    let sound = fixtures().join("p30_sound/ramp72000.wav");
    for (name, case) in expected()["remap"].as_object().expect("remap") {
        let fixture = fixtures().join(case["fixture"].as_str().expect("fixture"));
        let before = dir.join(format!("{name}-before.wav"));
        render(&fixture, &before, true, &[], &dir);
        // The document after, its sound named by its whole path from the temp folder.
        let after_text = case["after"].as_str().expect("after").replace(
            "\"p30_sound/ramp72000.wav\"",
            &serde_json::to_string(&sound.to_string_lossy()).expect("a path"),
        );
        let after_project = dir.join(format!("{name}.json"));
        fs::write(&after_project, after_text).expect("writes");
        let after = dir.join(format!("{name}-after.wav"));
        render(&after_project, &after, true, &[], &dir);
        let (before, after) = (samples(&before), samples(&after));
        assert!(before.iter().any(|value| *value != 0.0), "{name}: sounds");
        assert!(before == after, "{name}: the moved clip plays other audio");
    }
}

/// The ten-minute sound's frame `k` (p30_expected.json's "long"): a sawtooth of `period` frames,
/// the right side its half negated.
fn long_frame(k: u64, period: u64) -> [f32; 2] {
    let left = ((k % period) as f64 - (period / 2) as f64) / period as f64;
    [left as f32, (-left / 2.0) as f32]
}

// The 10x case: ten minutes of stereo at 48 kHz with a fade and a gain over nine and a half of
// them. The render makes the edited sound once into dawsome/edited of the user's cache folder (a
// 32-bit float WAV of every frame, 230 MB), streams the clip from there, and renders the frames
// stated; a second render finds the file, writes nothing and renders the same bytes. The
// sound's own file is never written. Its bound in time is the spec's "seconds", with room for
// a debug build.
#[test]
fn p30_a_ten_minute_edited_sound_goes_through_the_edited_cache() {
    let want = expected();
    let long = &want["long"];
    let dir = temp_dir("long");
    let frames = long["frames"].as_u64().expect("frames");
    let period = long["sawtooth"].as_u64().expect("period");
    let rate = long["rate"].as_u64().expect("rate") as u32;
    let sound = dir.join(long["sound"].as_str().expect("sound"));
    {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(&sound, spec).expect("creates");
        for k in 0..frames {
            for value in long_frame(k, period) {
                writer.write_sample(value).expect("writes");
            }
        }
        writer.finalize().expect("finishes");
    }
    let print = fingerprint(&sound);
    let project = dir.join("p30_long.json");
    fs::copy(
        fixtures().join(long["fixture"].as_str().expect("fixture")),
        &project,
    )
    .expect("copies");
    let out = dir.join("long.wav");
    let started = Instant::now();
    render(&project, &out, true, &[], &dir);
    let took = started.elapsed();
    let bound = long["seconds"].as_u64().expect("seconds");
    assert!(took.as_secs() < bound, "the render took {took:?}");
    eprintln!("ten minutes edited and rendered in {took:?}");
    let render_want = &long["render"];
    let values = samples(&out);
    assert_eq!(
        values.len() as u64,
        2 * render_want["frames"].as_u64().expect("frames")
    );
    for probe in render_want["probes"].as_array().expect("probes") {
        let at = probe[0].as_u64().expect("a frame") as usize;
        let side = |index: usize| probe[index].as_f64().expect("a value") as f32;
        assert_eq!(
            [values[2 * at], values[2 * at + 1]],
            [side(1), side(2)],
            "frame {at}"
        );
    }
    assert_eq!(
        sha_of(&values),
        render_want["sha256"].as_str().expect("sha256")
    );
    // The edited cache holds the whole edited sound, once.
    let cache = dir.join("cache/dawsome/edited");
    let files: Vec<PathBuf> = fs::read_dir(&cache)
        .expect("the edited cache")
        .map(|entry| entry.expect("an entry").path())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    let reader = hound::WavReader::open(&files[0]).expect("a WAV");
    let spec = reader.spec();
    assert_eq!(
        (spec.channels, spec.sample_rate, spec.bits_per_sample),
        (2, rate, 32)
    );
    assert_eq!(spec.sample_format, hound::SampleFormat::Float);
    assert_eq!(u64::from(reader.duration()), frames);
    let bytes = fs::metadata(&files[0]).expect("a size").len();
    let data = long["cache_bytes"].as_u64().expect("bytes");
    assert!(bytes >= data && bytes < data + 1024, "{bytes} bytes");
    drop(reader);
    let made = fingerprint(&files[0]);
    // Found again: nothing written, the same bytes.
    let again = dir.join("again.wav");
    render(&project, &again, true, &[], &dir);
    assert!(fs::read(&again).expect("a render") == fs::read(&out).expect("a render"));
    assert_eq!(
        fingerprint(&files[0]),
        made,
        "the cached file was made again"
    );
    assert_eq!(
        fingerprint(&sound),
        print,
        "the sound's own file was written"
    );
}

// The DAWproject export carries the file as it is on disk and says the edits stayed behind
// (spec question 11's default: a line in P1, the edited sound rendered into the package later).
#[test]
fn p30_the_dawproject_export_names_the_edits_it_leaves_out() {
    let dir = temp_dir("dawproject");
    let out = dir.join("crop.dawproject");
    let project = fixtures().join("p30_crop.json");
    dawsome(
        &[
            "export-dawproject".as_ref(),
            project.as_os_str(),
            out.as_os_str(),
        ],
        &dir,
    );
    let line = "Sound's sound edits (its file is exported as on disk)";
    let report = fs::read_to_string(dir.join("crop.dawproject.report.txt")).expect("a report");
    assert!(report.contains(&format!("  {line}\n")), "{report}");
}

/// Negates every value of the 32-bit float WAV at `path` in place (each value's sign bit
/// flipped), its size kept, and sets its time back to what it was: a sound replaced as
/// `cp -p`, `rsync -t` or `touch -r` leave one.
fn negate_keeping_size_and_time(path: &Path) {
    let time = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .expect("a time");
    let mut bytes = fs::read(path).expect("a sound");
    let size = bytes.len();
    let word = |bytes: &[u8], at: usize| {
        u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes")) as usize
    };
    let mut at = 12;
    let mut negated = 0;
    while at + 8 <= bytes.len() {
        let (id, length) = (&bytes[at..at + 4], word(&bytes, at + 4));
        if id == b"fmt " {
            let tag = u16::from_le_bytes([bytes[at + 8], bytes[at + 9]]);
            let bits = u16::from_le_bytes([bytes[at + 22], bytes[at + 23]]);
            assert_eq!((tag, bits), (3, 32), "a 32-bit float WAV");
        }
        if id == b"data" {
            for value in bytes[at + 8..at + 8 + length].chunks_exact_mut(4) {
                value[3] ^= 0x80;
                negated += 1;
            }
        }
        at += 8 + length + length % 2;
    }
    assert!(negated > 0, "no data chunk in {path:?}");
    fs::write(path, &bytes).expect("written");
    fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(time))
        .expect("dated");
    let metadata = fs::metadata(path).expect("a sound");
    assert_eq!(metadata.len() as usize, size);
    assert_eq!(metadata.modified().expect("a time"), time);
}

// A sound replaced in place by one of the same size and time plays the new sound, not the old
// edited one from the edited cache. p30_undo with its audio channel faded out over the whole
// sound, eight bars of its song (both clips, one stretched) streamed from the caches
// (--stream-threshold 100); then ramp72000.wav's values negated, its size and time kept:
// rendered again with the caches as they are, the song is the first render negated, value for
// value (the fade, the stretch's read and the mix are linear, so the sign goes through them
// exactly), and the edited cache holds a second sound.
#[test]
fn p30_a_sound_replaced_at_the_same_size_and_time_renders_the_new_sound() {
    let dir = temp_dir("replaced");
    fs::create_dir_all(dir.join("p30_sound")).expect("a folder");
    for sound in ["ramp48000.wav", "ramp72000.wav"] {
        fs::copy(
            fixtures().join("p30_sound").join(sound),
            dir.join("p30_sound").join(sound),
        )
        .expect("copies");
    }
    let text = fs::read_to_string(fixtures().join("p30_undo.json")).expect("p30_undo");
    let mut project: Value = serde_json::from_str(&text).expect("parses");
    project["format_version"] = 8.into();
    let channel = project["channels"]
        .as_array_mut()
        .and_then(|channels| channels.iter_mut().find(|channel| channel["id"] == 2))
        .expect("the audio channel");
    channel["edits"] = serde_json::json!({"ops": [{"fade_out": {"from": 0, "to": 72000}}]});
    let path = dir.join("p30_faded.json");
    fs::write(&path, project.to_string()).expect("written");
    let args = ["--bars", "8", "--stream-threshold", "100"];
    let first = dir.join("first.wav");
    render(&path, &first, true, &args, &dir);
    let before = samples(&first);
    assert!(before.iter().any(|value| *value != 0.0), "the song sounds");

    negate_keeping_size_and_time(&dir.join("p30_sound/ramp72000.wav"));
    let second = dir.join("second.wav");
    render(&path, &second, true, &args, &dir);
    let after = samples(&second);
    assert_eq!(after.len(), before.len());
    let stale = after
        .iter()
        .zip(&before)
        .position(|(after, before)| *after != -*before);
    assert_eq!(stale, None, "a value of the old sound in the second render");
    let cached = fs::read_dir(dir.join("cache/dawsome/edited"))
        .expect("the edited cache")
        .count();
    assert_eq!(cached, 2, "the replaced sound edited and cached on its own");
}
