//! The saturator: a port of Airwindows Density (github.com/airwindows/airwindows, MIT License,
//! Copyright (c) 2018 Chris Johnson; the full notice is in licenses/THIRD_PARTY.md), a sine-shaped
//! saturation blended by its density, after a one-pole highpass, with its own output level and
//! dry/wet. Its four parameters are the original's 0..1 values, so a null test maps one to one.

use super::{decibels, dither, guard, overall_scale, HALF_PI, SEEDS};
use crate::processor::{ParamInfo, ProcessContext, Processor, ProcessorFactory};

pub const ID: &str = "dawsome.density";

const fn param(
    key: &'static str,
    label: &'static str,
    default: f32,
    unit: &'static str,
) -> ParamInfo {
    ParamInfo {
        key,
        label,
        min: 0.0,
        max: 1.0,
        default,
        unit,
        whole: false,
    }
}

/// The original's A to D. Density 0.2 is no change (its display, 5A − 1, reads 0 there).
/// Shown (see [`Factory::shown`]): density as the original shows it, the highpass's −3 dB
/// point in hertz, the output gain in decibels.
const PARAMS: [ParamInfo; 4] = [
    param("density", "Density", 0.2, ""),
    param("highpass", "Highpass", 0.0, "Hz"),
    param("output", "Output", 1.0, "dB"),
    param("mix", "Mix", 1.0, "%"),
];

const DENSITY: usize = 0;
const HIGHPASS: usize = 1;
const OUTPUT: usize = 2;
const MIX: usize = 3;

/// How many sine stages the saturation runs, from `density`: −1 to 4, 0 at 0.2.
fn drive(density: f32) -> f64 {
    (f64::from(density) * 5.0) - 1.0
}

/// The one-pole lowpass coefficient the highpass subtracts, from `highpass`, at a rate whose
/// [`overall_scale`] is `scale`.
fn iir_amount(highpass: f32, scale: f64) -> f64 {
    f64::from(highpass).powf(3.0) / scale
}

/// The −3 dB point, in hertz, of the highpass with lowpass coefficient `amount` at
/// `sample_rate`. Why at half the rate: its two memories take turns, each seeing every other
/// sample, so each is the one-pole highpass b(1 − z⁻¹)/(1 − bz⁻¹), b = 1 − amount, at rate/2,
/// whose gain is 1/√2 where cos ω = (1 − 3b²)/(2b − 4b²). Infinite when there is no such
/// point: a coefficient of 1 or more leaves no lowpass memory, so nothing passes, and below
/// b = 1/(2√2 − 1) the gain stays under 1/√2 at every frequency.
fn cutoff(amount: f64, sample_rate: u32) -> f64 {
    if amount >= 1.0 {
        return f64::INFINITY;
    }
    let b = 1.0 - amount;
    let cosine = (1.0 - 3.0 * b * b) / (2.0 * b - 4.0 * b * b);
    if !(-1.0..=1.0).contains(&cosine) {
        return f64::INFINITY;
    }
    cosine.acos() * (f64::from(sample_rate) / 2.0) / std::f64::consts::TAU
}

pub(crate) struct Factory;

impl ProcessorFactory for Factory {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Saturator (Density)"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn shown(&self, index: usize, value: f32, sample_rate: u32) -> f64 {
        match index {
            DENSITY => drive(value),
            HIGHPASS => cutoff(iir_amount(value, overall_scale(sample_rate)), sample_rate),
            OUTPUT => decibels(value),
            _ => f64::from(value),
        }
    }

    fn create(&self, sample_rate: u32, values: &[f32]) -> Box<dyn Processor> {
        let mut density = Density {
            values: PARAMS.map(|info| info.default),
            overall_scale: overall_scale(sample_rate),
            state: State::new(),
        };
        for (index, &value) in values.iter().enumerate() {
            density.set_param(index, value);
        }
        Box::new(density)
    }
}

/// What carries from sample to sample: two highpass memories per side, used on alternate
/// samples, which one is next, and the two noise generators.
#[derive(Debug, Clone, Copy)]
struct State {
    /// `[left, right]` for each of the two alternating filters.
    highpass: [[f64; 2]; 2],
    /// Whether the first filter runs on the next sample.
    first: bool,
    noise: [u32; 2],
}

impl State {
    /// As the original constructs it.
    fn new() -> Self {
        Self {
            highpass: [[0.0; 2]; 2],
            first: true,
            noise: SEEDS,
        }
    }
}

struct Density {
    values: [f32; 4],
    overall_scale: f64,
    state: State,
}

/// `sample` pushed toward full scale by a sine up to π/2, keeping its sign.
fn saturate(sample: f64) -> f64 {
    let shaped = (sample.abs() * HALF_PI).min(HALF_PI).sin();
    if sample > 0.0 {
        shaped
    } else {
        -shaped
    }
}

impl Processor for Density {
    fn process(&mut self, _context: &ProcessContext, buffer: &mut [[f32; 2]]) {
        let [density, highpass, output, wet] = self.values;
        let mut density = drive(density);
        let iir_amount = iir_amount(highpass, self.overall_scale);
        let output = f64::from(output);
        let wet = f64::from(wet);
        let dry = 1.0 - wet;
        // The original takes whole units off `out` inside its sample loop, which changes it on
        // the first sample only: the same as once here.
        let mut out = density.abs();
        density *= density.abs();
        while out > 1.0 {
            out -= 1.0;
        }
        let keep = 1.0 - out;
        let state = &mut self.state;
        for frame in buffer.iter_mut() {
            let input = *frame;
            let filter = usize::from(!state.first);
            let mut wet_frame = [0.0f32; 2];
            for side in 0..2 {
                let dry_sample = guard(f64::from(input[side]), state.noise[side]);
                let memory = &mut state.highpass[filter][side];
                *memory = (*memory * (1.0 - iir_amount)) + (dry_sample * iir_amount);
                let mut sample = dry_sample - *memory;
                let mut count = density;
                while count > 1.0 {
                    sample = saturate(sample);
                    count -= 1.0;
                }
                let size = (sample.abs() * HALF_PI).min(HALF_PI);
                let shaped = if density > 0.0 {
                    size.sin()
                } else {
                    1.0 - size.cos()
                };
                sample = if sample > 0.0 {
                    (sample * keep) + (shaped * out)
                } else {
                    (sample * keep) - (shaped * out)
                };
                if output < 1.0 {
                    sample *= output;
                }
                if wet < 1.0 {
                    sample = (dry_sample * dry) + (sample * wet);
                }
                wet_frame[side] = dither(sample, &mut state.noise[side]) as f32;
            }
            state.first = !state.first;
            // Why mix 0 returns the input itself: the original would still add its dither, and
            // a dry setting must pass the sound bit for bit. The state above has advanced.
            if self.values[MIX] != 0.0 {
                *frame = wet_frame;
            }
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let (Some(slot), Some(info)) = (self.values.get_mut(index), PARAMS.get(index)) {
            *slot = info.clamp(value);
        }
    }

    /// Back to the constructed state, the noise generators' seeds included.
    fn reset(&mut self) {
        self.state = State::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::tests::{bits, noise, whole_and_split};

    #[test]
    fn any_split_gives_the_same_bits() {
        let input = noise(3000);
        let values = [0.9, 0.4, 0.7, 0.5];
        let mut make = || Factory.create(44_100, &values);
        let (whole, split) = whole_and_split(&mut make, &input, &[1, 7, 512, 1, 999]);
        assert_eq!(bits(&whole), bits(&split));
    }

    // The highpass's −3 dB point as a sine sweep finds it, at 0.5 and 44.1 kHz with the
    // saturation at no change: the gain of a quarter-scale sine, measured past the filter's
    // settling, first reaches 1/√2 between two of the sweep's 1 % steps; interpolated there in
    // decibels over log frequency, it lies within 2 % of the cutoff shown.
    #[test]
    fn a_sine_sweep_finds_the_highpass_cutoff_shown() {
        const RATE: u32 = 44_100;
        const SETTLED: usize = 2048;
        let decibels_at = |frequency: f64| {
            let input: Vec<[f32; 2]> = (0..8192)
                .map(|frame| {
                    let phase = std::f64::consts::TAU * frequency * frame as f64;
                    let sample = (0.25 * (phase / f64::from(RATE)).sin()) as f32;
                    [sample; 2]
                })
                .collect();
            let mut output = input.clone();
            Factory
                .create(RATE, &[0.2, 0.5, 1.0, 1.0])
                .process(&crate::dsp::tests::CONTEXT, &mut output);
            let power = |frames: &[[f32; 2]]| -> f64 {
                frames.iter().map(|frame| f64::from(frame[0]).powi(2)).sum()
            };
            10.0 * (power(&output[SETTLED..]) / power(&input[SETTLED..])).log10()
        };
        let half_power = -10.0 * 2f64.log10();
        let mut below = (100.0f64, decibels_at(100.0));
        let measured = loop {
            let frequency = below.0 * 1.01;
            assert!(
                frequency < f64::from(RATE) / 4.0,
                "the gain never reached −3 dB"
            );
            let gain = decibels_at(frequency);
            if gain >= half_power {
                let along = (half_power - below.1) / (gain - below.1);
                break (below.0.ln() + along * (frequency.ln() - below.0.ln())).exp();
            }
            below = (frequency, gain);
        };
        let shown = Factory.shown(HIGHPASS, 0.5, RATE);
        assert!(
            (measured / shown - 1.0).abs() < 0.02,
            "measured {measured:.1} Hz, shown {shown:.1} Hz"
        );
    }

    // Density 1 by hand: 5·1 − 1 = 4, so out = 4 − 3 = 1 and the count is 4·4 = 16; the loop
    // saturates 15 times (16 down to 2), and the blend at out = 1 is one more sine stage: 16 in
    // all, applied to a quarter-scale sample in f64.
    #[test]
    fn full_density_is_sixteen_sine_stages() {
        let mut expected = 0.25f64;
        for _ in 0..16 {
            expected = (expected.abs() * HALF_PI).min(HALF_PI).sin();
        }
        let mut effect = Factory.create(44_100, &[1.0, 0.0, 1.0, 1.0]);
        let mut buffer = [[0.25, -0.25]];
        effect.process(&crate::dsp::tests::CONTEXT, &mut buffer);
        // The dither moves the result by less than its f32 rounding step.
        assert!(
            (f64::from(buffer[0][0]) - expected).abs() < 1e-6,
            "{buffer:?}"
        );
        assert!(
            (f64::from(buffer[0][1]) + expected).abs() < 1e-6,
            "{buffer:?}"
        );
    }
}
