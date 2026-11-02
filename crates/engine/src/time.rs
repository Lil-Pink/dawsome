//! Musical time: positions in integer ticks, tempo in integer milli-BPM, and the one function
//! that turns a tick into a sample. Every time conversion in the engine lives here.

use std::ops::RangeInclusive;

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

/// A pattern's swing when it has none: each 16th takes half of its 8th-note pair.
pub const STRAIGHT_SWING: u16 = 500;

/// The swing of a triplet feel, 2:1: the first 16th of a pair takes two thirds of it. Why 667
/// and not 666 or 670: it alone lands the second 16th on the exact triplet at PPQ 96 (tick 32)
/// and at PPQ 960 (tick 320).
pub const TRIPLET_SWING: u16 = 667;

/// The swings a pattern takes, in tenths of a per cent of an 8th-note pair the first 16th of it
/// takes: 50 % (straight) to 75 % (dotted, 3:1).
pub const SWING_RANGE: RangeInclusive<u16> = STRAIGHT_SWING..=750;

/// Where the second of a pair of notes `unit` ticks apart plays, from the pair's start, when the
/// first takes `swing_tenths` tenths of a per cent of the pair: `floor(2 · unit · swing / 1000)`.
/// `unit` itself at [`STRAIGHT_SWING`].
pub fn swing_offset(unit: Ticks, swing_tenths: u16) -> Ticks {
    // A unit is ppq/4 < 2^14 ticks, so the product stays far inside an i64.
    Ticks((2 * unit.0 * i64::from(swing_tenths)).div_euclid(1000))
}

/// The tick a note stored at `start` plays on in a pattern swung by `swing_tenths`: a note on
/// the second 16th of an 8th-note pair (`start mod ppq/2 == ppq/4`) moves to
/// `start − ppq/4 + swing_offset(ppq/4, swing)`; every other note (on-beats, notes off the step
/// grid) stays. Why only those: a step sequencer's swing, by long convention, moves the steps,
/// and no rule says where a hand-placed note between steps would go.
pub fn swung(start: Ticks, swing_tenths: u16, ppq: u16) -> Ticks {
    let unit = step_to_ticks(1, ppq);
    // `checked_rem_euclid` is `None` for a pair of 0 ticks (a PPQ below 4): nothing swings.
    if start.0.checked_rem_euclid(2 * unit.0) != Some(unit.0) {
        return start;
    }
    let offset = swing_offset(unit, swing_tenths);
    Ticks(start.0.saturating_sub(unit.0).saturating_add(offset.0))
}

/// How long a step hit that swing moved late lasts: `2 · unit − swing_offset(unit, swing)`, so
/// it still ends where the next step starts, as an unmoved step hit (one `unit`) does. Why not
/// one step: its note-off would then fall after the next step's note-on, and on a plugin or a
/// MIDI out that note-off of the same key would silence the next hit.
pub fn swung_step_gate(unit: Ticks, swing_tenths: u16) -> Ticks {
    Ticks(2 * unit.0 - swing_offset(unit, swing_tenths).0)
}

/// A pattern note stored at `start`, `length` ticks long (0 for a step hit, which lasts one
/// step), as it plays in a pattern swung by `swing_tenths`: its tick ([`swung`]) and its length,
/// its own unless it is a step hit swing moved, which lasts [`swung_step_gate`]. The one rule
/// playback, the song's notes and the MIDI exports swing a note by.
pub fn swung_note(start: Ticks, length: Ticks, swing_tenths: u16, ppq: u16) -> (Ticks, Ticks) {
    let tick = swung(start, swing_tenths, ppq);
    if length.0 > 0 || tick == start {
        return (tick, length);
    }
    (tick, swung_step_gate(step_to_ticks(1, ppq), swing_tenths))
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

    // Known answers worked by hand (the swing design's table, checked with Python's integers):
    // a note at t with t mod 2U = U plays at t − U + floor(2U·S/1000), U = ppq/4; any other
    // note stays.
    #[test]
    fn the_second_sixteenth_of_each_pair_plays_late() {
        let at = |start, swing, ppq| swung(Ticks(start), swing, ppq).0;
        let ppq_96 = [
            (500, 24, 24),
            (521, 24, 25),
            (520, 24, 24),
            (666, 24, 31),
            (667, 24, 32),
            (750, 24, 36),
            (750, 72, 84),
            (667, 0, 0),
            (667, 30, 30),
            (667, 48, 48),
        ];
        for (swing, start, want) in ppq_96 {
            assert_eq!(
                at(start, swing, PPQ),
                want,
                "{swing} at tick {start}, PPQ 96"
            );
        }
        for (swing, start, want) in [
            (667, 240, 320),
            (750, 240, 360),
            (505, 240, 242),
            (667, 250, 250),
        ] {
            assert_eq!(
                at(start, swing, 960),
                want,
                "{swing} at tick {start}, PPQ 960"
            );
        }
        // A PPQ too small for a step (never a valid project's) swings nothing.
        assert_eq!(at(1, 750, 2), 1);
        assert_eq!(at(0, 750, 0), 0);
    }

    // 2:1 exactly at PPQ 96 and 960: the triplet's second 16th on 32 of 48 and 320 of 480 ticks.
    // Straight, the offset is the unit itself. At PPQ 96 the ear hears 13 positions over the
    // range: delay d first sounds at S = ceil(1000·(24 + d)/48).
    #[test]
    fn the_swing_offset_has_thirteen_steps_at_ppq_96() {
        assert_eq!(swing_offset(Ticks(24), TRIPLET_SWING), Ticks(32));
        assert_eq!(swing_offset(Ticks(240), TRIPLET_SWING), Ticks(320));
        assert_eq!(swing_offset(Ticks(24), STRAIGHT_SWING), Ticks(24));
        let firsts = [
            500, 521, 542, 563, 584, 605, 625, 646, 667, 688, 709, 730, 750,
        ];
        for (delay, first) in (0..).zip(firsts) {
            assert_eq!(swing_offset(Ticks(24), first), Ticks(24 + delay), "{first}");
            if first > *SWING_RANGE.start() {
                assert_eq!(
                    swing_offset(Ticks(24), first - 1),
                    Ticks(23 + delay),
                    "{first}"
                );
            }
        }
    }

    // A swung step hit lasts U − d: 16 at 66.7 % and 12 at 75 % (PPQ 96), one step straight; so
    // it ends where the next pair starts, for every swing and PPQ.
    #[test]
    fn a_swung_step_hit_ends_on_the_next_step() {
        let gate = |unit, swing| swung_step_gate(Ticks(unit), swing).0;
        assert_eq!([gate(24, 500), gate(24, 667), gate(24, 750)], [24, 16, 12]);
        assert_eq!(
            [gate(240, 500), gate(240, 667), gate(240, 750)],
            [240, 160, 120]
        );
        for ppq in [4, 8, 96, 960] {
            let unit = step_to_ticks(1, ppq);
            let start = Ticks(3 * 2 * unit.0 + unit.0);
            for swing in SWING_RANGE {
                let (tick, length) = swung_note(start, Ticks(0), swing, ppq);
                if tick == start {
                    assert_eq!(length, Ticks(0), "unmoved: PPQ {ppq}, {swing}");
                } else {
                    assert_eq!(tick.0 + length.0, 8 * unit.0, "PPQ {ppq}, {swing}");
                }
            }
        }
    }

    // A note keeps its own length (its end moves with its start); only a step hit swing moved is
    // given one, and an unmoved one stays a step hit.
    #[test]
    fn a_swung_note_keeps_its_length() {
        let note = |start, length, swing| {
            let (tick, length) = swung_note(Ticks(start), Ticks(length), swing, PPQ);
            (tick.0, length.0)
        };
        assert_eq!(note(24, 0, 667), (32, 16));
        assert_eq!(note(24, 12, 667), (32, 12));
        assert_eq!(note(120, 48, 750), (132, 48));
        assert_eq!(note(0, 0, 667), (0, 0));
        assert_eq!(note(30, 0, 750), (30, 0));
        assert_eq!(note(24, 0, 520), (24, 0), "a swing too small to move it");
        assert_eq!(note(24, 0, STRAIGHT_SWING), (24, 0));
    }

    // Why a swung note never leaves its pattern: a moved start pP + U is below the length (a
    // whole number of bars, so of pairs), and the offset is below a pair, so pP + offset is too.
    #[test]
    fn every_swung_note_stays_inside_its_pattern() {
        for ppq in [4, 8, 96, 960] {
            let length = bars_to_ticks(1, ppq);
            for swing in SWING_RANGE {
                for start in 0..length.0 {
                    let tick = swung(Ticks(start), swing, ppq);
                    assert!(
                        tick.0 >= start && tick < length,
                        "PPQ {ppq}, {swing}: {start} -> {}",
                        tick.0
                    );
                }
            }
        }
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
