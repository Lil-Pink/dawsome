//! Musical time: positions in integer ticks, tempo in integer milli-BPM, and the one function
//! that turns a tick into a sample. Every time conversion in the engine lives here.

use serde::{Deserialize, Serialize};

/// A position or length in musical time, counted in ticks (`ppq` ticks per beat). The default
/// is tick 0.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Ticks(pub i64);

/// A tempo in thousandths of a beat per minute: 120 BPM is `MilliBpm(120_000)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MilliBpm(pub u32);

impl MilliBpm {
    /// Converts a tempo in BPM (as written in a project file), rounding to the nearest 0.001 BPM.
    ///
    /// Returns `None` unless the result is a positive tempo that fits in a `u32`.
    pub fn from_bpm(bpm: f64) -> Option<Self> {
        let milli = (bpm * 1000.0).round();
        // `contains` is false for NaN, so a non-number is rejected along with out-of-range values.
        if (1.0..=f64::from(u32::MAX)).contains(&milli) {
            Some(Self(milli as u32))
        } else {
            None
        }
    }
}

/// Beats in one bar (every bar is 4/4).
pub const BEATS_PER_BAR: u16 = 4;

/// Steps in one beat: a step is a 1/16 note.
pub const STEPS_PER_BEAT: u16 = 4;

/// The tick where step `step` starts: `step * ppq / 4`.
pub fn step_to_ticks(step: u32, ppq: u16) -> Ticks {
    // Multiplying before dividing keeps this exact whenever `ppq` is a multiple of 4, which
    // `Project::validate` requires. A u32 times a u16 cannot overflow an i64.
    Ticks(i64::from(step) * i64::from(ppq) / i64::from(STEPS_PER_BEAT))
}

/// The length of `bars` bars, in ticks.
pub fn bars_to_ticks(bars: u32, ppq: u16) -> Ticks {
    Ticks(i64::from(bars) * i64::from(BEATS_PER_BAR) * i64::from(ppq))
}

/// The absolute sample where `tick` falls:
/// `floor(tick * 60 * sample_rate * 1000 / (milli_bpm * ppq))`.
///
/// Returns `None` when there is no such sample: a negative tick, a zero sample rate, tempo or
/// PPQ, or an answer beyond `u64::MAX`.
pub fn tick_to_sample(tick: Ticks, sample_rate: u32, tempo: MilliBpm, ppq: u16) -> Option<u64> {
    // Why exact integer math from the absolute tick, every time: the obvious shortcut, a cached
    // f64 "samples per tick" rate multiplied by the tick, lands one sample early exactly on
    // integer boundaries (174.5 BPM at 48 kHz, tick 48511: exact 8340000, cached rate 8339999).
    //
    // Why u128: tick < 2^63, 60 * 1000 < 2^16 and sample_rate < 2^32, so the numerator stays
    // below 2^111 and cannot overflow.
    //
    // Why a zero sample rate gives `None`: it would put every tick at sample 0, and a timeline
    // that never advances cannot be played.
    if sample_rate == 0 {
        return None;
    }
    let tick = u128::try_from(tick.0).ok()?;
    let numerator = tick * 60 * 1000 * u128::from(sample_rate);
    let denominator = u128::from(tempo.0) * u128::from(ppq);
    // Integer division of non-negative numbers rounds down, so this is the floor.
    u64::try_from(numerator.checked_div(denominator)?).ok()
}

/// The first tick that falls at or after absolute sample `sample`:
/// `ceil(sample * milli_bpm * ppq / (60 * sample_rate * 1000))`. It inverts [`tick_to_sample`]:
/// the returned tick lands at or after `sample`, and the tick before it lands before `sample`.
///
/// Returns `None` when there is no such tick: a zero sample rate, tempo or PPQ, or an answer
/// beyond `i64::MAX`.
pub fn first_tick_at_or_after(
    sample: u64,
    sample_rate: u32,
    tempo: MilliBpm,
    ppq: u16,
) -> Option<Ticks> {
    if tempo.0 == 0 || ppq == 0 {
        return None;
    }
    // Why u128: sample < 2^64, milli-BPM < 2^32 and ppq < 2^16, so the numerator stays below
    // 2^112. `tick_to_sample(t) >= sample` holds exactly when t >= sample * denominator /
    // numerator of that function, so the smallest such t is this quotient rounded up.
    let numerator = u128::from(sample) * u128::from(tempo.0) * u128::from(ppq);
    let denominator = 60 * 1000 * u128::from(sample_rate);
    if denominator == 0 {
        return None;
    }
    i64::try_from(numerator.div_ceil(denominator))
        .ok()
        .map(Ticks)
}

/// `value` ticks at `from` PPQ as ticks at `to` PPQ, rounded half up:
/// `floor(value · to / from + 1/2)`. Every file format read at another PPQ goes through this
/// one rule. `None` for a `from` of 0 or an answer past `u64::MAX`.
pub fn rescale_ticks(value: u64, from: u16, to: u16) -> Option<u64> {
    if from == 0 {
        return None;
    }
    // Why u128: value < 2^64 and `to` < 2^16, so twice the product stays below 2^81.
    let scaled = u128::from(value) * u128::from(to);
    let from = u128::from(from);
    u64::try_from((scaled * 2 + from) / (from * 2)).ok()
}

/// The length in ticks a new audio clip of `frames` frames gets when it starts on tick `start`:
/// the fewest ticks (at least 1) whose samples hold the whole file,
/// L = min{L ≥ 1 : s(start + L) − s(start) ≥ frames}.
///
/// Returns `None` when there is no such length (a tick or sample past what the time base holds).
pub fn ticks_holding(
    start: Ticks,
    frames: u64,
    sample_rate: u32,
    tempo: MilliBpm,
    ppq: u16,
) -> Option<Ticks> {
    // s(start + L) ≥ s(start) + frames exactly when start + L is at or after the first tick at
    // or after that sample.
    let from = tick_to_sample(start, sample_rate, tempo, ppq)?;
    let end = first_tick_at_or_after(from.checked_add(frames)?, sample_rate, tempo, ppq)?;
    Some(Ticks(end.0.checked_sub(start.0)?.max(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PPQ: u16 = 96;

    // From tests/fixtures/gen_p8_expected.py: a 1000-frame file dropped at tick 384 at 48 kHz and
    // 140 BPM needs 5 ticks; the 11025 frames of p8_resample at tick 0 at 44.1 kHz and 120 BPM 48,
    // and the 176400 of a two-bar recording 768. An empty file still gets a tick.
    #[test]
    fn a_new_audio_clip_is_long_enough_for_its_whole_file() {
        let length = |start, frames, rate, bpm| {
            ticks_holding(Ticks(start), frames, rate, MilliBpm(bpm), PPQ).expect("fits")
        };
        assert_eq!(length(384, 1000, 48_000, 140_000), Ticks(5));
        assert_eq!(length(0, 11_025, 44_100, 120_000), Ticks(48));
        assert_eq!(length(0, 176_400, 44_100, 120_000), Ticks(768));
        assert_eq!(length(0, 0, 44_100, 120_000), Ticks(1));
        assert_eq!(length(7, 1, 44_100, 120_000), Ticks(1));
    }

    fn bpm(whole: u32) -> MilliBpm {
        MilliBpm(whole * 1000)
    }

    // Expected values: a known-answer table worked out by hand and checked with Python's exact
    // integers: tick * 60 * sr * 1000 // (milli_bpm * 96).
    #[test]
    fn steps_and_bars_land_on_the_known_answer_table() {
        const STEPS: [u32; 8] = [0, 1, 2, 3, 4, 8, 12, 15];
        let table: [(u32, u32, [u64; 8], u64); 5] = [
            (
                44100,
                120,
                [0, 5512, 11025, 16537, 22050, 44100, 66150, 82687],
                88200,
            ),
            (
                48000,
                140,
                [0, 5142, 10285, 15428, 20571, 41142, 61714, 77142],
                82285,
            ),
            (
                96000,
                522,
                [0, 2758, 5517, 8275, 11034, 22068, 33103, 41379],
                44137,
            ),
            (
                44100,
                10,
                [0, 66150, 132300, 198450, 264600, 529200, 793800, 992250],
                1058400,
            ),
            (
                44100,
                999,
                [0, 662, 1324, 1986, 2648, 5297, 7945, 9932],
                10594,
            ),
        ];
        for (rate, tempo, step_samples, bar) in table {
            for (step, want) in STEPS.into_iter().zip(step_samples) {
                let got = tick_to_sample(step_to_ticks(step, PPQ), rate, bpm(tempo), PPQ);
                assert_eq!(got, Some(want), "{rate} Hz, {tempo} BPM, step {step}");
            }
            let got = tick_to_sample(bars_to_ticks(1, PPQ), rate, bpm(tempo), PPQ);
            assert_eq!(got, Some(bar), "{rate} Hz, {tempo} BPM, one bar");
        }
    }

    // Python: 48511 * 60 * 48000 * 1000 // (174500 * 96) == 8340000; a cached f64 rate gives 8339999.
    #[test]
    fn boundary_tick_that_a_cached_float_rate_gets_wrong() {
        let got = tick_to_sample(Ticks(48511), 48000, MilliBpm(174_500), PPQ);
        assert_eq!(got, Some(8_340_000));
    }

    #[test]
    fn tick_zero_is_sample_zero() {
        assert_eq!(tick_to_sample(Ticks(0), 44100, bpm(120), PPQ), Some(0));
    }

    #[test]
    fn samples_never_run_backwards() {
        for (rate, tempo) in [
            (44100, MilliBpm(174_500)),
            (48000, MilliBpm(93_217)),
            (96000, bpm(522)),
        ] {
            let mut previous = 0;
            for tick in 0..50_000 {
                let sample = tick_to_sample(Ticks(tick), rate, tempo, PPQ).expect("in range");
                assert!(
                    sample >= previous,
                    "{rate} Hz, {tempo:?}: tick {tick} went backwards"
                );
                previous = sample;
            }
        }
    }

    #[test]
    fn impossible_inputs_have_no_sample() {
        assert_eq!(tick_to_sample(Ticks(-1), 44100, bpm(120), PPQ), None);
        assert_eq!(tick_to_sample(Ticks(1), 0, bpm(120), PPQ), None);
        assert_eq!(tick_to_sample(Ticks(1), 44100, MilliBpm(0), PPQ), None);
        assert_eq!(tick_to_sample(Ticks(1), 44100, bpm(120), 0), None);
        // i64::MAX ticks at 0.001 BPM and PPQ 1 is far past u64::MAX samples.
        assert_eq!(
            tick_to_sample(Ticks(i64::MAX), u32::MAX, MilliBpm(1), 1),
            None
        );
    }

    // From Python (gen_p2_expected.py, case (e)): ceil(30000 * 120000 * 96 / (60 * 44100 * 1000)) =
    // ceil(130.61) = 131.
    #[test]
    fn first_tick_at_or_after_inverts_the_tick_rule() {
        assert_eq!(
            first_tick_at_or_after(30_000, 44100, bpm(120), PPQ),
            Some(Ticks(131))
        );
        assert_eq!(
            first_tick_at_or_after(0, 44100, bpm(120), PPQ),
            Some(Ticks(0))
        );
        // The defining property, swept where ticks are not a whole number of samples.
        for (rate, tempo) in [(48000, MilliBpm(174_500)), (44100, MilliBpm(93_217))] {
            for sample in (0..200_000).step_by(37) {
                let tick = first_tick_at_or_after(sample, rate, tempo, PPQ).expect("in range");
                let at = |t: i64| tick_to_sample(Ticks(t), rate, tempo, PPQ).expect("in range");
                assert!(
                    at(tick.0) >= sample,
                    "{rate} Hz, {tempo:?}: sample {sample}"
                );
                if tick.0 > 0 {
                    assert!(
                        at(tick.0 - 1) < sample,
                        "{rate} Hz, {tempo:?}: sample {sample}"
                    );
                }
            }
        }
        assert_eq!(first_tick_at_or_after(1, 0, bpm(120), PPQ), None);
        assert_eq!(first_tick_at_or_after(1, 44100, MilliBpm(0), PPQ), None);
    }

    // Half up, by hand: 77 ticks at PPQ 480 are 15.4 at 96 (15); 2.5 rounds up to 3; 1 tick at
    // 192 is 0.5 at 96 (1).
    #[test]
    fn rescaling_rounds_half_up() {
        assert_eq!(rescale_ticks(77, 480, 96), Some(15));
        assert_eq!(rescale_ticks(12, 480, 100), Some(3), "2.5");
        assert_eq!(rescale_ticks(1, 192, 96), Some(1), "0.5");
        assert_eq!(rescale_ticks(1300, 480, 96), Some(260));
        assert_eq!(rescale_ticks(5, 0, 96), None);
        assert_eq!(rescale_ticks(u64::MAX, 1, 2), None);
    }

    #[test]
    fn a_step_is_a_sixteenth_note() {
        assert_eq!(step_to_ticks(1, PPQ), Ticks(24));
        assert_eq!(step_to_ticks(4, PPQ), Ticks(96), "four steps make one beat");
        // A 2-bar pattern holds 32 steps: step 32 is exactly where it ends.
        assert_eq!(step_to_ticks(32, PPQ), bars_to_ticks(2, PPQ));
        assert_eq!(bars_to_ticks(2, PPQ), Ticks(768));
    }

    #[test]
    fn bpm_rounds_to_the_nearest_thousandth() {
        assert_eq!(MilliBpm::from_bpm(120.0), Some(MilliBpm(120_000)));
        assert_eq!(MilliBpm::from_bpm(174.5), Some(MilliBpm(174_500)));
        assert_eq!(MilliBpm::from_bpm(93.2174), Some(MilliBpm(93_217)));
        assert_eq!(MilliBpm::from_bpm(0.0006), Some(MilliBpm(1)));
        for bad in [0.0, 0.0004, -120.0, f64::NAN, f64::INFINITY, 5e6] {
            assert_eq!(MilliBpm::from_bpm(bad), None, "{bad}");
        }
    }
}
