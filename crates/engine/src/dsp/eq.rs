//! The three-band equalizer: a low shelf, a peaking band and a high shelf in series. Each band is
//! a biquad with the coefficients of Robert Bristow-Johnson's Audio EQ Cookbook, run per side
//! in f64 as Direct Form I. A band at exactly 0 dB is skipped, so an untouched EQ passes its
//! input bit for bit.

use std::f32::consts::FRAC_1_SQRT_2;
use std::f64::consts::PI;

use super::{blend, snap64};
use crate::processor::{ParamInfo, ProcessContext, Processor, ProcessorFactory};

pub const ID: &str = "dawsome.eq3";

/// Band frequencies stay below this fraction of the sample rate, where the cookbook's filters
/// are still well behaved.
const MAX_FREQ_FRACTION: f64 = 0.45;

const fn param(
    key: &'static str,
    label: &'static str,
    (min, max, default): (f32, f32, f32),
    unit: &'static str,
) -> ParamInfo {
    ParamInfo {
        key,
        label,
        min,
        max,
        default,
        unit,
        whole: false,
    }
}

/// The shelves' default Q, 1/√2: the flattest shelf, with no bump at the corner.
const BUTTERWORTH_Q: f32 = FRAC_1_SQRT_2;

const FREQ: (f32, f32) = (20.0, 20_000.0);
const GAIN: (f32, f32) = (-18.0, 18.0);
const Q: (f32, f32) = (0.1, 10.0);

/// Three values per band (frequency, gain, Q) in band order, then the mix.
const PARAMS: [ParamInfo; 10] = [
    param("low_freq", "Low freq", (FREQ.0, FREQ.1, 100.0), "Hz"),
    param("low_gain_db", "Low gain", (GAIN.0, GAIN.1, 0.0), "dB"),
    param("low_q", "Low Q", (Q.0, Q.1, BUTTERWORTH_Q), ""),
    param("mid_freq", "Mid freq", (FREQ.0, FREQ.1, 1000.0), "Hz"),
    param("mid_gain_db", "Mid gain", (GAIN.0, GAIN.1, 0.0), "dB"),
    param("mid_q", "Mid Q", (Q.0, Q.1, 1.0), ""),
    param("high_freq", "High freq", (FREQ.0, FREQ.1, 8000.0), "Hz"),
    param("high_gain_db", "High gain", (GAIN.0, GAIN.1, 0.0), "dB"),
    param("high_q", "High Q", (Q.0, Q.1, BUTTERWORTH_Q), ""),
    param("mix", "Mix", (0.0, 1.0, 1.0), "%"),
];

const MIX: usize = 9;

pub(super) struct Factory;

impl ProcessorFactory for Factory {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "EQ"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn create(&self, sample_rate: u32, values: &[f32]) -> Box<dyn Processor> {
        let mut eq = Eq {
            bands: [Shape::LowShelf, Shape::Peaking, Shape::HighShelf].map(Band::new),
            mix: PARAMS[MIX].default,
            sample_rate,
        };
        for (index, &value) in values.iter().enumerate() {
            eq.set_param(index, value);
        }
        Box::new(eq)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    LowShelf,
    Peaking,
    HighShelf,
}

/// One band: its settings, its normalized coefficients `[b0, b1, b2, a1, a2]` and, per side,
/// the last two inputs and outputs.
struct Band {
    shape: Shape,
    freq: f32,
    gain_db: f32,
    q: f32,
    coefficients: [f64; 5],
    /// `[x1, x2, y1, y2]` per side.
    memory: [[f64; 4]; 2],
}

impl Band {
    fn new(shape: Shape) -> Self {
        Self {
            shape,
            freq: 1000.0,
            gain_db: 0.0,
            q: 1.0,
            coefficients: [1.0, 0.0, 0.0, 0.0, 0.0],
            memory: [[0.0; 4]; 2],
        }
    }

    fn active(&self) -> bool {
        self.gain_db != 0.0
    }

    fn update(&mut self, sample_rate: u32) {
        let fs = f64::from(sample_rate);
        let freq = f64::from(self.freq).min(MAX_FREQ_FRACTION * fs);
        self.coefficients = coefficients(
            self.shape,
            freq,
            f64::from(self.gain_db),
            f64::from(self.q),
            fs,
        );
    }

    /// One sample of side `side` through the band.
    fn tick(&mut self, side: usize, x: f64) -> f64 {
        let [b0, b1, b2, a1, a2] = self.coefficients;
        let [x1, x2, y1, y2] = self.memory[side];
        let y = snap64(b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2);
        self.memory[side] = [x, x1, y, y1];
        y
    }
}

/// The cookbook coefficients of a band at `freq` Hz, `gain_db` and `q`, for sample rate `fs`,
/// divided by a0: `[b0, b1, b2, a1, a2]`.
fn coefficients(shape: Shape, freq: f64, gain_db: f64, q: f64, fs: f64) -> [f64; 5] {
    let a = 10f64.powf(gain_db / 40.0);
    let w0 = 2.0 * PI * freq / fs;
    let cos = w0.cos();
    let alpha = w0.sin() / (2.0 * q);
    let shelf = 2.0 * a.sqrt() * alpha;
    let [b0, b1, b2, a0, a1, a2] = match shape {
        Shape::Peaking => [
            1.0 + alpha * a,
            -2.0 * cos,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos,
            1.0 - alpha / a,
        ],
        Shape::LowShelf => [
            a * ((a + 1.0) - (a - 1.0) * cos + shelf),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
            a * ((a + 1.0) - (a - 1.0) * cos - shelf),
            (a + 1.0) + (a - 1.0) * cos + shelf,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos),
            (a + 1.0) + (a - 1.0) * cos - shelf,
        ],
        Shape::HighShelf => [
            a * ((a + 1.0) + (a - 1.0) * cos + shelf),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
            a * ((a + 1.0) + (a - 1.0) * cos - shelf),
            (a + 1.0) - (a - 1.0) * cos + shelf,
            2.0 * ((a - 1.0) - (a + 1.0) * cos),
            (a + 1.0) - (a - 1.0) * cos - shelf,
        ],
    };
    [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0]
}

struct Eq {
    bands: [Band; 3],
    mix: f32,
    sample_rate: u32,
}

impl Processor for Eq {
    fn process(&mut self, _context: &ProcessContext, buffer: &mut [[f32; 2]]) {
        if !self.bands.iter().any(Band::active) {
            return;
        }
        for frame in buffer {
            let mut wet = *frame;
            for (side, sample) in wet.iter_mut().enumerate() {
                let mut value = f64::from(*sample);
                for band in self.bands.iter_mut().filter(|band| band.active()) {
                    value = band.tick(side, value);
                }
                *sample = value as f32;
            }
            *frame = blend(*frame, wet, self.mix);
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(info) = PARAMS.get(index) else {
            return;
        };
        let value = info.clamp(value);
        if index == MIX {
            self.mix = value;
            return;
        }
        let band = &mut self.bands[index / 3];
        let was_active = band.active();
        match index % 3 {
            0 => band.freq = value,
            1 => band.gain_db = value,
            _ => band.q = value,
        }
        // Why clear a band coming back from 0 dB: its memory stopped when it was skipped, and
        // old values from then would sound as a click.
        if band.active() && !was_active {
            band.memory = [[0.0; 4]; 2];
        }
        band.update(self.sample_rate);
    }

    fn reset(&mut self) {
        for band in &mut self.bands {
            band.memory = [[0.0; 4]; 2];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::tests::{bits, noise, whole_and_split};

    /// H(z) of `[b0, b1, b2, a1, a2]` at z = 1 (DC) and z = −1 (half the sample rate).
    fn dc_and_nyquist([b0, b1, b2, a1, a2]: [f64; 5]) -> (f64, f64) {
        (
            (b0 + b1 + b2) / (1.0 + a1 + a2),
            (b0 - b1 + b2) / (1.0 - a1 + a2),
        )
    }

    fn eq(values: &[(usize, f32)]) -> Box<dyn Processor> {
        let mut defaults: Vec<f32> = PARAMS.iter().map(|info| info.default).collect();
        for &(index, value) in values {
            defaults[index] = value;
        }
        Factory.create(44_100, &defaults)
    }

    // The coefficients tests/fixtures/gen_p5_expected.py prints for 1 kHz, +6 dB, Q 1 at
    // 48 kHz: b = [1.0439530870, -1.8953207239, 0.8677222848], a = [1, -1.8953207239,
    // 0.9116753718].
    #[test]
    fn the_peaking_band_has_the_cookbook_coefficients() {
        let got = coefficients(Shape::Peaking, 1000.0, 6.0, 1.0, 48_000.0);
        let want = [
            1.043_953_087_0,
            -1.895_320_723_9,
            0.867_722_284_8,
            -1.895_320_723_9,
            0.911_675_371_8,
        ];
        for (got, want) in got.iter().zip(want) {
            assert!((got - want).abs() < 1e-9, "{got} != {want}");
        }
    }

    // A shelf's gain far from its corner is the shelf gain, 10^(dB/20) whatever the Q: 3.98107
    // for +12 dB at DC on the low shelf, 0.251189 for −12 dB at half the sample rate on the
    // high shelf, and unity on the other end of each (Python, from the cookbook formulas).
    #[test]
    fn shelves_reach_their_gain_beyond_the_corner() {
        let (dc, nyquist) = dc_and_nyquist(coefficients(
            Shape::LowShelf,
            100.0,
            12.0,
            std::f64::consts::FRAC_1_SQRT_2,
            48_000.0,
        ));
        assert!((dc - 3.981_071_705_534_972).abs() < 1e-9, "{dc}");
        assert!((nyquist - 1.0).abs() < 1e-9, "{nyquist}");
        let (dc, nyquist) = dc_and_nyquist(coefficients(
            Shape::HighShelf,
            8000.0,
            -12.0,
            std::f64::consts::FRAC_1_SQRT_2,
            48_000.0,
        ));
        assert!((nyquist - 0.251_188_643_150_958).abs() < 1e-9, "{nyquist}");
        assert!((dc - 1.0).abs() < 1e-9, "{dc}");
    }

    #[test]
    fn flat_bands_and_mix_0_pass_the_input_bit_for_bit() {
        let input = noise(2000);
        for mut instance in [eq(&[]), eq(&[(1, 6.0), (4, -3.0), (MIX, 0.0)])] {
            let mut output = input.clone();
            instance.process(&crate::dsp::tests::CONTEXT, &mut output);
            assert_eq!(bits(&output), bits(&input));
        }
    }

    #[test]
    fn any_split_gives_the_same_bits() {
        let mut make = || eq(&[(1, 9.0), (4, -6.0), (7, 4.0), (MIX, 0.7)]);
        let (whole, split) = whole_and_split(&mut make, &noise(3000), &[1, 7, 512, 100]);
        assert_eq!(bits(&whole), bits(&split));
    }

    #[test]
    fn reset_forgets_the_signal() {
        let mut instance = eq(&[(4, 12.0)]);
        let mut loud = noise(500);
        instance.process(&crate::dsp::tests::CONTEXT, &mut loud);
        instance.reset();
        let mut quiet = vec![[0.0; 2]; 64];
        instance.process(&crate::dsp::tests::CONTEXT, &mut quiet);
        assert!(quiet.iter().all(|frame| *frame == [0.0; 2]));
    }
}
