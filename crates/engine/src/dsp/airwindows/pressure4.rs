//! The compressor: a port of Airwindows Pressure4 (github.com/airwindows/airwindows, MIT
//! License, Copyright (c) 2018 Chris Johnson; the full notice is in licenses/THIRD_PARTY.md), a vari-mu
//! compressor followed by a sine-shaped overdrive stage. Its four parameters are the original's
//! 0..1 values, so a null test maps one to one; the mix around it is Dawsome's own.

use super::{decibels, dither, guard, overall_scale, HALF_PI, SEEDS};
use crate::dsp::blend;
use crate::processor::{ParamInfo, ProcessContext, Processor, ProcessorFactory};

pub const ID: &str = "dawsome.pressure4";

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

/// The original's A to D, then Dawsome's mix: y = (1 − mix)·x + mix·wet. Shown (see
/// [`Factory::shown`]): pressure as the drive it raises the input by before the gain reduction,
/// in decibels (why not a ratio: it sets that drive and the level the reduction starts at, and
/// no fixed slope follows from them), speed as the time the gain takes to recover at rest,
/// mewiness as the original shows it (2C − 1), the output gain in decibels.
const PARAMS: [ParamInfo; 5] = [
    param("pressure", "Pressure", 0.0, "dB"),
    param("speed", "Speed", 0.2, "ms"),
    param("mewiness", "Mewiness", 1.0, ""),
    param("output", "Output", 1.0, "dB"),
    param("mix", "Mix", 1.0, "%"),
];

const PRESSURE: usize = 0;
const SPEED: usize = 1;
const MEWINESS: usize = 2;
const OUTPUT: usize = 3;

/// The level the gain reduction starts at, from `pressure`: full scale at 0, 5 % of it at 1.
/// The input is raised by its inverse first.
fn threshold(pressure: f32) -> f64 {
    1.0 - (f64::from(pressure) * 0.95)
}

/// The gain's recovery time constant at rest, in frames, from `speed`, at a rate whose
/// [`overall_scale`] is `scale`.
fn release(speed: f32, scale: f64) -> f64 {
    let mut release = (1.28 - f64::from(speed)).powf(5.0) * 32768.0;
    release /= scale;
    release
}

/// How far, and which way, the gain is shaped, from `mewiness`: −1 to 1.
fn shape_amount(mewiness: f32) -> f64 {
    (f64::from(mewiness) * 2.0) - 1.0
}

pub(crate) struct Factory;

impl ProcessorFactory for Factory {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Compressor (Pressure4)"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn shown(&self, index: usize, value: f32, sample_rate: u32) -> f64 {
        match index {
            PRESSURE => 20.0 * threshold(value).recip().log10(),
            SPEED => {
                let frames = release(value, overall_scale(sample_rate));
                frames / f64::from(sample_rate.max(1)) * 1000.0
            }
            MEWINESS => shape_amount(value),
            OUTPUT => decibels(value),
            _ => f64::from(value),
        }
    }

    fn create(&self, sample_rate: u32, values: &[f32]) -> Box<dyn Processor> {
        let mut pressure = Pressure4 {
            values: PARAMS.map(|info| info.default),
            overall_scale: overall_scale(sample_rate),
            state: State::new(),
        };
        for (index, &value) in values.iter().enumerate() {
            pressure.set_param(index, value);
        }
        Box::new(pressure)
    }
}

/// What carries from sample to sample: the gain-reduction coefficient and its speed, and the
/// two noise generators.
///
/// Why one coefficient and one speed where the original keeps two of each: it alternates
/// between its two sets on a flip flag that it never sets, so only the second set is ever used.
#[derive(Debug, Clone, Copy)]
struct State {
    speed: f64,
    coefficient: f64,
    noise: [u32; 2],
}

impl State {
    /// As the original constructs it.
    fn new() -> Self {
        Self {
            speed: 10000.0,
            coefficient: 1.0,
            noise: SEEDS,
        }
    }
}

struct Pressure4 {
    values: [f32; 5],
    overall_scale: f64,
    state: State,
}

impl Processor for Pressure4 {
    fn process(&mut self, _context: &ProcessContext, buffer: &mut [[f32; 2]]) {
        let [pressure, speed, mewiness, output, mix] = self.values;
        let threshold = threshold(pressure);
        let makeup = 1.0 / threshold;
        let release = release(speed, self.overall_scale);
        let fastest = release.sqrt();
        let mut mewiness = shape_amount(mewiness);
        let positive = mewiness >= 0.0;
        if !positive {
            mewiness = -mewiness;
        }
        let unmewiness = 1.0 - mewiness;
        let output_gain = f64::from(output);
        let state = &mut self.state;
        for frame in buffer.iter_mut() {
            let dry = *frame;
            let mut samples = [0.0; 2];
            for (side, sample) in samples.iter_mut().enumerate() {
                *sample = guard(f64::from(dry[side]), state.noise[side]) * makeup;
            }
            let [left, right] = samples;
            let mut sense = left.abs();
            if right.abs() > sense {
                sense = right.abs();
            }
            if sense > threshold {
                let vary = threshold / sense;
                let attack = state.speed.abs().sqrt();
                state.coefficient *= attack - 1.0;
                state.coefficient += if vary < threshold { threshold } else { vary };
                state.coefficient /= attack;
            } else {
                state.coefficient *= (state.speed * state.speed) - 1.0;
                state.coefficient += 1.0;
                state.coefficient /= state.speed * state.speed;
            }
            let new_speed = state.speed * (state.speed - 1.0) + (sense * release).abs() + fastest;
            state.speed = new_speed / state.speed;
            let shaped = if positive {
                state.coefficient * state.coefficient
            } else {
                state.coefficient.sqrt()
            };
            let coefficient = (shaped * mewiness) + (state.coefficient * unmewiness);
            let mut wet = [0.0f32; 2];
            for (side, sample) in samples.iter_mut().enumerate() {
                *sample *= coefficient;
                if output_gain != 1.0 {
                    *sample *= output_gain;
                }
                // The second overdrive stage: a sine up to π/2, full scale past it.
                let size = sample.abs();
                let shaped = if size > HALF_PI { 1.0 } else { size.sin() };
                *sample = if *sample > 0.0 { shaped } else { -shaped };
                wet[side] = dither(*sample, &mut state.noise[side]) as f32;
            }
            *frame = blend(dry, wet, mix);
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
        let values = [0.7, 0.3, 0.2, 0.8, 0.6];
        let mut make = || Factory.create(48_000, &values);
        let (whole, split) = whole_and_split(&mut make, &input, &[1, 7, 512, 1, 999]);
        assert_eq!(bits(&whole), bits(&split));
    }
}
