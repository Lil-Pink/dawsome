//! Input latency's known answers (tests/fixtures/gen_p23_expected.py): the burst the measurement
//! plays, and the latency found in each input feed the generator wrote, against the master's
//! recording of the burst played from the take's first frame.

use std::path::{Path, PathBuf};

use dawsome_media::latency::{self, MeasureError, NotFound};
use sha2::{Digest, Sha256};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

fn expected() -> serde_json::Value {
    let text = std::fs::read_to_string(Path::new(FIXTURES).join("p23_expected.json"))
        .expect("the known answers");
    serde_json::from_str(&text).expect("parses")
}

fn number(value: &serde_json::Value) -> i64 {
    value.as_i64().expect("a number")
}

/// A feed file's two sides apart, each then silent to the recording's length (after the file's
/// last frame every frame is silence).
fn feed(name: &str, frames: usize) -> Vec<Vec<f32>> {
    let mut reader = hound::WavReader::open(Path::new(FIXTURES).join(name)).expect("the feed");
    let values: Vec<f32> = reader
        .samples::<i16>()
        .map(|value| f32::from(value.expect("reads")) / 32768.0)
        .collect();
    (0..2)
        .map(|side| {
            let mut channel: Vec<f32> = values.iter().skip(side).step_by(2).copied().collect();
            channel.resize(frames.max(channel.len()), 0.0);
            channel
        })
        .collect()
}

/// The master's recording as the player makes it: the burst from frame `at` (the preview's
/// first frame: 0 when it starts with the take), on both sides (summed), then silence.
fn master(at: usize, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0; frames];
    for (n, value) in latency::burst(48_000).into_iter().enumerate() {
        out[at + n] = 2.0 * value;
    }
    out
}

// The burst is the generator's to the bit: its length, its SHA-256 (f32 little-endian) and a
// few of its frames; the recording's length and the lags searched are the generator's too.
#[test]
fn the_burst_is_the_generators() {
    let want = expected();
    let burst = latency::burst(48_000);
    assert_eq!(burst.len() as i64, number(&want["burst_frames"]));
    let bytes: Vec<u8> = burst.iter().flat_map(|value| value.to_le_bytes()).collect();
    let hash: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(hash, want["burst_sha256"].as_str().expect("a hash"));
    for probe in want["burst_probes"].as_array().expect("probes") {
        let at = usize::try_from(number(&probe[0])).expect("an index");
        // As f32: serde_json's default float reading can be a unit in the last place off.
        assert_eq!(burst[at], probe[1].as_f64().expect("a value") as f32);
    }
    assert_eq!(
        latency::recording_frames(48_000) as i64,
        number(&want["recording_frames"])
    );
    assert_eq!(
        latency::lags(48_000),
        number(&want["earliest"])..=number(&want["latest"])
    );
}

// Known answers 1 and 4: the latency each feed holds is found (clean 1234, messy 2205: inverted,
// on one side, an echo, hum and a DC offset beside it, early -960, and 1234 on two
// sides in opposite polarity, whose sum is silence); a burst past the lags searched, hum alone
// and silence are "no click heard".
#[test]
fn each_feed_gives_the_latency_it_was_made_with() {
    let want = expected();
    let frames = usize::try_from(number(&want["recording_frames"])).expect("fits");
    let master = master(0, frames);
    for (name, case) in want["measure"].as_object().expect("cases") {
        let input = feed(case["file"].as_str().expect("a file"), frames);
        let found = latency::find(&master, &input, 48_000);
        match case["latency"].as_i64() {
            Some(late) => assert_eq!(found, Ok(late), "{name}"),
            None => assert_eq!(found, Err(NotFound::NotHeard), "{name}"),
        }
    }
    let silent = [vec![0.0; frames]];
    assert_eq!(
        latency::find(&master, &silent, 48_000),
        Err(NotFound::NotHeard)
    );
}

// The burst is found where the master's recording holds it: one played a call (512 frames)
// after the take began is found there, and the latency counts from it; a master holding no
// burst is "no click played".
#[test]
fn the_latency_counts_from_where_the_master_holds_the_burst() {
    let want = expected();
    let frames = usize::try_from(number(&want["recording_frames"])).expect("fits");
    let input: Vec<Vec<f32>> = feed("p23_feed_clean.wav", frames - 512)
        .into_iter()
        .map(|side| {
            let mut later = vec![0.0; 512];
            later.extend(side);
            later.truncate(frames);
            later
        })
        .collect();
    assert_eq!(
        latency::find(&master(512, frames), &input, 48_000),
        Ok(1234)
    );
    assert_eq!(
        latency::find(&vec![0.0; frames], &input, 48_000),
        Err(NotFound::NotPlayed)
    );
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dawsome-latency-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

/// `sides` written as a take's file is: 32-bit float at 48 kHz, a channel each.
fn write(path: &Path, sides: &[&[f32]]) {
    let spec = hound::WavSpec {
        channels: sides.len() as u16,
        sample_rate: 48_000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec).expect("creates");
    for frame in 0..sides[0].len() {
        for side in sides {
            writer.write_sample(side[frame]).expect("writes");
        }
    }
    writer.finalize().expect("finalizes");
}

// The files a measurement writes, read back: the master stereo, the input mono, the latency and
// the rate it counts; a stereo input wired in opposite polarity, whose sides sum
// to silence, still measures; an input holding no burst is refused.
#[test]
fn a_measurements_files_give_the_latency_and_its_rate() {
    let want = expected();
    let frames = usize::try_from(number(&want["recording_frames"])).expect("fits");
    let (master_file, input_file) = (temp("master.wav"), temp("input.wav"));
    let halved: Vec<f32> = master(0, frames).iter().map(|v| v / 2.0).collect();
    write(&master_file, &[&halved, &halved]);
    let messy = feed("p23_feed_messy.wav", frames);
    let summed: Vec<f32> = messy[0].iter().zip(&messy[1]).map(|(l, r)| l + r).collect();
    write(&input_file, &[&summed]);
    assert_eq!(
        latency::measure_files(&master_file, &input_file).expect("measures"),
        (2205, 48_000)
    );
    let opposite = feed("p23_feed_opposite.wav", frames);
    write(&input_file, &[&opposite[0], &opposite[1]]);
    assert_eq!(
        latency::measure_files(&master_file, &input_file).expect("measures"),
        (1234, 48_000)
    );
    write(&input_file, &[&vec![0.0; frames]]);
    assert!(matches!(
        latency::measure_files(&master_file, &input_file),
        Err(MeasureError::NotFound(NotFound::NotHeard))
    ));
}
