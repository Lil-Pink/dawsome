//! Input latency measurement: the click burst the output plays while a take of the input runs,
//! and where that burst is found in the input's recording, by cross-correlation against the
//! master's recording of the same take. Known answers: `tests/fixtures/gen_p23_expected.py`.
//!
//! The burst is three clicks of white noise from one LCG run, so no two stretches of it look
//! alike and the correlation has one sharp peak (a periodic click, the metronome's, matches
//! itself half a period away, inverted). Why the master's recording as well as the burst: the
//! burst plays as a preview, which the player takes at the start of a buffer, the buffer the
//! take begins in or the next; the master's recording says which, to the frame, so the latency
//! counts from where the burst really played. Both recordings start at the take's first frame
//! (the master's from the record tap, the input's lined up by the output's clock), so frame j of
//! the input holds what the master played at frame j - L.

use std::ops::RangeInclusive;
use std::path::Path;

use thiserror::Error;

/// Clicks in the burst, and the frames of each.
pub const CLICKS: usize = 3;
pub const CLICK_FRAMES: usize = 256;

/// Each click's level: −12 dBFS of white noise, loud enough down a cable and from a speaker to
/// a microphone, gentle on ears and tweeters.
const AMPLITUDE: f32 = 0.25;

/// The LCG run the clicks take their values from.
const SEED: u32 = 0x5EED_1234;

/// The least |normalized correlation| at which the burst counts as heard. Why 0.3: hum or noise
/// alone peaks near 0.16 over the lags searched (4.5 standard deviations of 1/√768, the clicks'
/// frames), a cable loop near 0.9, a microphone in a room 0.4 and up.
const LEAST_HEARD: f64 = 0.3;

/// The least match at which the master holds the burst: the player plays it as made, so only a
/// master effect or a muted master brings it under.
const LEAST_PLAYED: f64 = 0.5;

/// Frames of silence before the first click: the preview may start a buffer after the take, and
/// an early latency (an input timed ahead) needs room before the clicks.
pub fn lead(rate: u32) -> usize {
    rate as usize / 10
}

/// Frames from one click's start to the next's.
fn spacing(rate: u32) -> usize {
    rate as usize / 5
}

/// The burst the measurement plays, mono at `rate`: [`lead`] frames of silence, then the
/// [`CLICKS`] clicks of [`CLICK_FRAMES`] frames, [`spacing`] apart, each frame the next value of
/// one LCG run as `AMPLITUDE·((x >> 8)/2²³ − 1)`, every value exact in f32.
pub fn burst(rate: u32) -> Vec<f32> {
    let (lead, spacing) = (lead(rate), spacing(rate));
    let mut out = vec![0.0; lead + (CLICKS - 1) * spacing + CLICK_FRAMES];
    let mut x = SEED;
    for click in 0..CLICKS {
        let from = lead + click * spacing;
        for value in &mut out[from..from + CLICK_FRAMES] {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *value = AMPLITUDE * ((x >> 8) as f32 / 8_388_608.0 - 1.0);
        }
    }
    out
}

/// Frames the measurement records of the master and of the input, at `rate`: the burst, the
/// latest latency searched, and a quarter second for the buffer the preview may wait.
pub fn recording_frames(rate: u32) -> u64 {
    let burst = lead(rate) + (CLICKS - 1) * spacing(rate) + CLICK_FRAMES;
    (burst + rate as usize / 2 + rate as usize / 4) as u64
}

/// The latencies searched, in frames at `rate`: from 50 ms early to 500 ms late.
pub fn lags(rate: u32) -> RangeInclusive<i64> {
    -(i64::from(rate) / 20)..=i64::from(rate) / 2
}

/// Why no latency came of a measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum NotFound {
    /// The master's recording holds no burst: nothing was played.
    #[error("no click played")]
    NotPlayed,
    /// The input's recording holds no burst at any lag searched.
    #[error("no click heard")]
    NotHeard,
}

/// The burst's clicks: each one's frames and where it starts, counted from the first's start.
fn clicks(rate: u32) -> Vec<(usize, Vec<f32>)> {
    let (lead, spacing) = (lead(rate), spacing(rate));
    let burst = burst(rate);
    (0..CLICKS)
        .map(|click| {
            let from = lead + click * spacing;
            (click * spacing, burst[from..from + CLICK_FRAMES].to_vec())
        })
        .collect()
}

/// How well the clicks match `signal` with the first click at frame `at`: the normalized
/// correlation |Σ c·x| / √(Σc² · Σx²) over the clicks' frames (a frame outside `signal` is
/// silence), 0 where `signal` is silent there. Only the clicks' frames count: the burst is
/// silent elsewhere, so this is the whole cross-correlation at that lag.
fn matched(clicks: &[(usize, Vec<f32>)], signal: &[f32], at: i64) -> f64 {
    let (mut dot, mut heard, mut made) = (0.0f64, 0.0f64, 0.0f64);
    for (offset, values) in clicks {
        for (frame, &value) in values.iter().enumerate() {
            let index = at + (offset + frame) as i64;
            let x = usize::try_from(index)
                .ok()
                .and_then(|index| signal.get(index))
                .map_or(0.0, |&x| f64::from(x));
            let c = f64::from(value);
            dot += c * x;
            heard += x * x;
            made += c * c;
        }
    }
    if heard == 0.0 || made == 0.0 {
        return 0.0;
    }
    (dot / (made * heard).sqrt()).abs()
}

/// The frame in `range` where the clicks match `signal` best, and how well; the earliest of
/// equal matches.
fn best(clicks: &[(usize, Vec<f32>)], signal: &[f32], range: RangeInclusive<i64>) -> (i64, f64) {
    let mut found = (*range.start(), 0.0);
    for at in range {
        let score = matched(clicks, signal, at);
        if score > found.1 {
            found = (at, score);
        }
    }
    found
}

/// How many frames later than `master` the burst sits in `input`, the master mono and the input
/// as its channels, all at `rate` and from the take's first frame: where the master holds the
/// burst (from its first frame to a quarter second past the lead: the preview's buffer), then
/// the lag in [`lags`] at which the input matches it best, polarity either way (a cable or an
/// interface may invert it). Each channel is searched, and with two or more their sum too, and
/// the best match is kept: a pair wired in opposite polarity sums to silence (found
/// 2026-10-26), a click on one side only is drowned by the other's hum, and the sum of two
/// in phase hears it through noise either alone would not.
pub fn find(master: &[f32], input: &[Vec<f32>], rate: u32) -> Result<i64, NotFound> {
    let clicks = clicks(rate);
    let latest_start = (lead(rate) + rate as usize / 4) as i64;
    let (played, score) = best(&clicks, master, 0..=latest_start);
    if score < LEAST_PLAYED {
        return Err(NotFound::NotPlayed);
    }
    let lags = lags(rate);
    let summed = (input.len() > 1).then(|| {
        let len = input.iter().map(Vec::len).max().unwrap_or(0);
        (0..len)
            .map(|frame| input.iter().filter_map(|side| side.get(frame)).sum())
            .collect::<Vec<f32>>()
    });
    let mut found = (0, 0.0);
    for signal in input.iter().chain(&summed) {
        let (heard, score) = best(&clicks, signal, played + lags.start()..=played + lags.end());
        if score > found.1 {
            found = (heard, score);
        }
    }
    if found.1 < LEAST_HEARD {
        return Err(NotFound::NotHeard);
    }
    Ok(found.0 - played)
}

/// Why a measurement's files gave no latency.
#[derive(Debug, Error)]
pub enum MeasureError {
    #[error("{0}")]
    NotFound(#[from] NotFound),
    #[error("cannot read the recording: {0}")]
    Read(#[from] hound::Error),
}

/// A recording's channels apart, and its rate.
fn channels(path: &Path) -> Result<(Vec<Vec<f32>>, u32), hound::Error> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let count = usize::from(spec.channels.max(1));
    let values = reader.samples::<f32>().collect::<Result<Vec<f32>, _>>()?;
    let apart = (0..count)
        .map(|channel| {
            values
                .iter()
                .skip(channel)
                .step_by(count)
                .copied()
                .collect()
        })
        .collect();
    Ok((apart, spec.sample_rate))
}

/// [`find`] on the two recordings a measurement wrote (32-bit float, any channels): the master's
/// channels summed (the player plays the burst alike on both), the input's apart; the latency in
/// frames, and the rate it counts (the master's).
pub fn measure_files(master: &Path, input: &Path) -> Result<(i64, u32), MeasureError> {
    let (master, rate) = channels(master)?;
    let len = master.iter().map(Vec::len).max().unwrap_or(0);
    let master: Vec<f32> = (0..len)
        .map(|frame| master.iter().filter_map(|side| side.get(frame)).sum())
        .collect();
    let (input, _) = channels(input)?;
    Ok((find(&master, &input, rate)?, rate))
}
