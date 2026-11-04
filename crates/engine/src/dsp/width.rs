//! Stereo width: each frame split into its middle M = (l + r)/2 and its sides S = (l − r)/2,
//! the sides scaled by the width w, and put back as (M + S, M − S). 0% is mono (both sides the
//! middle), 100% leaves the frame as it is, 200% doubles the sides.
//!
//! Bass mono (Off, or 50 to 500 Hz evenly in octaves, 50·2^(v·log2 10) for its control v > 0):
//! the sides go through a 12 dB/octave high-pass at that frequency (the parts' [`Svf`],
//! Butterworth's Q 1/√2) before the width scales them, so below it the sound is mono; the middle
//! is untouched. Why on the sides alone and not a crossover on each side: removing the sides'
//! lows is what "mono below" means, it needs one filter, and nothing is split and summed back
//! (Ableton's Utility does the same). Why: wide bass is lost on mono speakers and in clubs,
//! and cuts a vinyl groove badly.
//!
//! With bass mono off and the width exactly 100% the frames pass with no arithmetic, so the
//! effect at its defaults gives the input's bits (as the EQ skips a band at 0 dB). A mono input
//! has no sides, so it passes bit for bit at any width and bass mono.
//!
//! Operation order (gen_p32_expected.py states it): M = (l + r)/2; sides ((l − r)/2)·w, or
//! HP((l − r)/2)·w with bass mono; out = (f32(M + sides), f32(M − sides)).

use std::f64::consts::{FRAC_1_SQRT_2, LOG2_10};

use super::exp2;
use super::parts::{Response, Svf};
use crate::processor::{ParamInfo, ProcessContext, Processor, ProcessorFactory, Taper};

pub const ID: &str = "dawsome.width";

const PARAMS: [ParamInfo; 2] = [
    ParamInfo {
        key: "width",
        label: "Width",
        min: 0.0,
        max: 2.0,
        default: 1.0,
        unit: "%",
        whole: false,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "bass_mono",
        label: "Bass mono",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: "Hz",
        whole: false,
        taper: Taper::Linear,
    },
];

const WIDTH: usize = 0;
const BASS_MONO: usize = 1;

/// The bass mono control's frequency for `value` above 0: 50 Hz to 500 Hz, evenly in octaves,
/// 50·2^(v·log2 10). At 0 it is off.
fn bass_hz(value: f32) -> f64 {
    50.0 * exp2(f64::from(value) * LOG2_10)
}

pub(super) struct Factory;

impl ProcessorFactory for Factory {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Stereo width"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn shown(&self, index: usize, value: f32, _sample_rate: u32) -> f64 {
        match index {
            BASS_MONO => bass_hz(value),
            _ => f64::from(value),
        }
    }

    fn value_name(&self, index: usize, value: f32) -> Option<&'static str> {
        (index == BASS_MONO && PARAMS[BASS_MONO].clamp(value) == 0.0).then_some("Off")
    }

    fn create(&self, sample_rate: u32, values: &[f32]) -> Box<dyn Processor> {
        let mut width = Width {
            width: PARAMS[WIDTH].default,
            bass_mono: PARAMS[BASS_MONO].default,
            low_cut: Svf::new(
                bass_hz(PARAMS[BASS_MONO].default),
                FRAC_1_SQRT_2,
                sample_rate,
            ),
            sample_rate,
        };
        for (index, &value) in values.iter().enumerate() {
            width.set_param(index, value);
        }
        Box::new(width)
    }
}

struct Width {
    width: f32,
    /// The bass mono control; 0 is off.
    bass_mono: f32,
    /// The sides' high-pass with bass mono on (side 0's memory).
    low_cut: Svf,
    sample_rate: u32,
}

impl Processor for Width {
    fn process(&mut self, _context: &ProcessContext, buffer: &mut [[f32; 2]]) {
        let bass_mono = self.bass_mono != 0.0;
        if self.width == 1.0 && !bass_mono {
            return;
        }
        let width = f64::from(self.width);
        for frame in buffer {
            let [left, right] = frame.map(f64::from);
            let middle = (left + right) / 2.0;
            let sides = if bass_mono {
                let sides = self
                    .low_cut
                    .process(0, (left - right) / 2.0, Response::HighPass);
                sides * width
            } else {
                (left - right) / 2.0 * width
            };
            *frame = [(middle + sides) as f32, (middle - sides) as f32];
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(info) = PARAMS.get(index) else {
            return;
        };
        let value = info.clamp(value);
        match index {
            WIDTH => self.width = value,
            _ => {
                self.bass_mono = value;
                self.low_cut
                    .set(bass_hz(value), FRAC_1_SQRT_2, self.sample_rate);
            }
        }
    }

    fn reset(&mut self) {
        self.low_cut.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::tests::{bits, noise, whole_and_split, CONTEXT};

    fn width(value: f32, frames: &[[f32; 2]]) -> Vec<[f32; 2]> {
        let mut out = frames.to_vec();
        Factory
            .create(44_100, &[value, 0.0])
            .process(&CONTEXT, &mut out);
        out
    }

    // By hand: (0.5, 0.25) has M = 0.375 and S = 0.125, so 200% gives (0.375 ± 0.25) and 0%
    // gives M on both sides.
    #[test]
    fn the_sides_scale_by_the_width() {
        assert_eq!(width(2.0, &[[0.5, 0.25]]), [[0.625, 0.125]]);
        assert_eq!(width(0.0, &[[0.5, 0.25]]), [[0.375, 0.375]]);
    }

    #[test]
    fn at_100_percent_the_input_passes_bit_for_bit() {
        let mut input = noise(500);
        input.push([-0.0, f32::MIN_POSITIVE]);
        assert_eq!(bits(&width(1.0, &input)), bits(&input));
    }

    #[test]
    fn at_0_percent_both_sides_are_the_same_bits() {
        for [left, right] in width(0.0, &noise(2000)) {
            assert_eq!(left.to_bits(), right.to_bits());
        }
    }

    #[test]
    fn any_split_gives_the_same_bits() {
        for bass_mono in [0.0, 0.6] {
            let mut make = || Factory.create(44_100, &[1.7, bass_mono]);
            let (whole, split) = whole_and_split(&mut make, &noise(3000), &[1, 7, 512, 1, 999]);
            assert_eq!(bits(&whole), bits(&split), "bass mono {bass_mono}");
        }
    }

    // The bass mono control by hand: 50·10^v Hz, so 50 Hz just above 0, 158.1 Hz (50·√10) at
    // the middle and 500 Hz at the top; 0 is Off.
    #[test]
    fn bass_mono_runs_from_50_to_500_hz_and_0_is_off() {
        assert_eq!(bass_hz(0.0), 50.0);
        assert!((bass_hz(0.5) - 50.0 * 10f64.sqrt()).abs() < 1e-9);
        assert!((bass_hz(1.0) - 500.0).abs() < 1e-9);
        assert_eq!(Factory.value_name(BASS_MONO, 0.0), Some("Off"));
        assert_eq!(Factory.value_name(BASS_MONO, 0.5), None);
        assert_eq!(Factory.value_name(WIDTH, 0.0), None);
    }

    // A mono input has no sides: with bass mono on it passes bit for bit, at any width.
    #[test]
    fn bass_mono_passes_a_mono_input_bit_for_bit() {
        let input: Vec<[f32; 2]> = noise(3000).iter().map(|f| [f[0], f[0]]).collect();
        for value in [0.0, 1.0, 2.0] {
            let mut out = input.clone();
            Factory
                .create(44_100, &[value, 0.7])
                .process(&CONTEXT, &mut out);
            assert_eq!(bits(&out), bits(&input), "width {value}");
        }
    }

    // Below the bass mono frequency the sides go: a 20 Hz difference between the sides (far
    // under 500 Hz) comes out nearly mono once the high-pass settles, the middle kept whole.
    #[test]
    fn bass_mono_takes_the_sides_out_below_its_frequency() {
        let frames: Vec<[f32; 2]> = (0..44_100)
            .map(|n| {
                let x = 0.5 * (std::f64::consts::TAU * 20.0 * n as f64 / 44_100.0).sin();
                [(0.25 + x) as f32, (0.25 - x) as f32]
            })
            .collect();
        let mut out = frames.clone();
        Factory
            .create(44_100, &[1.0, 1.0])
            .process(&CONTEXT, &mut out);
        for [l, r] in &out[22_050..] {
            assert!((l - r).abs() < 0.002, "{l} {r}");
            assert!(((l + r) / 2.0 - 0.25).abs() < 1e-6, "the middle");
        }
    }
}
