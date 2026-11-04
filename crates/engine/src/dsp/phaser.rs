//! Phaser: the sound through a chain of 4, 6 or 8 first-order allpass stages whose break
//! frequency a slow sine (the LFO) sweeps, mixed with the dry sound, so the notches where the
//! chain's phase turns the wet sound against the dry one move up and down (J. O. Smith, "An
//! Allpass Approach to Digital Phasing and Flanging", 1984; re-derived, no code copied). One
//! break frequency for every stage of a side; the right side's sweep runs Spread (0° to 180°,
//! 90° by default) ahead of the left's, so the notches move across the stereo field. At 0° both
//! sides share one sweep and the phaser is mono-compatible.
//!
//! Per frame and side: angle = phase on the left, phase + spread·π/180 on the right;
//! f = center·2^(depth/2 · sin(angle)), held to 20 Hz .. 0.45·rate; t = tan(π·f/sr);
//! a = (t − 1)/(t + 1), so each stage H(z) = (a + z⁻¹)/(1 + a·z⁻¹) lags exactly 90° at f. Per
//! side, u = x + feedback·(the chain's last output); then per stage y = a·u + x1 − a·y1, x1 = u,
//! y1 = y, u = y (tiny state to 0); the wet side is u, rounded to f32 once; the mix is
//! [`blend_power`] (equal power). The center is a 0..1 control, 100·2^(v·log2 40) Hz (100 Hz to
//! 4 kHz, 632 Hz at the middle); depth is the sweep's span in octaves, half of it either side of
//! the center.
//!
//! Why Spread at 90° and equal power: the professional phasers it
//! was measured against all sweep the sides apart by default, and the stereo swirl is what
//! people know a phaser by; a linear mix lost 3.8 dB at 50%.
//!
//! Operation order (gen_p32_expected.py states it, and the Rust follows): the right side's
//! offset (spread·π)/180; center = 100·exp2(v·(2 + log2 10)); per side
//! f = center·exp2((depth/2)·sin(angle)); t = tan((π·f)/sr); both sides' a first, then the
//! chains, left then right; a stage y = ((a·u) + x1) − (a·y1).

use std::f64::consts::{LOG2_10, PI};

use super::eq::MAX_FREQ_FRACTION;
use super::parts::{lfo_offset, Lfo};
use super::{blend_power, exp2, snap64};
use crate::processor::{ParamInfo, ProcessContext, Processor, ProcessorFactory, Taper};

pub const ID: &str = "dawsome.phaser";

const PARAMS: [ParamInfo; 7] = [
    ParamInfo {
        key: "stages",
        label: "Stages",
        min: 0.0,
        max: 2.0,
        default: 1.0,
        unit: "",
        whole: true,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "rate",
        label: "Rate",
        min: 0.05,
        max: 5.0,
        default: 0.5,
        unit: "Hz",
        whole: false,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "depth",
        label: "Depth",
        min: 0.0,
        max: 4.0,
        default: 2.0,
        unit: "oct",
        whole: false,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "center",
        label: "Center",
        min: 0.0,
        max: 1.0,
        default: 0.5,
        unit: "Hz",
        whole: false,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "feedback",
        label: "Feedback",
        min: -0.95,
        max: 0.95,
        default: 0.0,
        unit: "%",
        whole: false,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "spread",
        label: "Spread",
        min: 0.0,
        max: 180.0,
        default: 90.0,
        unit: "°",
        whole: false,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "mix",
        label: "Mix",
        min: 0.0,
        max: 1.0,
        default: 0.5,
        unit: "%",
        whole: false,
        taper: Taper::Linear,
    },
];

const STAGES: usize = 0;
const RATE: usize = 1;
const DEPTH: usize = 2;
const CENTER: usize = 3;
const FEEDBACK: usize = 4;
const SPREAD: usize = 5;
const MIX: usize = 6;

/// The stages control's choices by value: N = 4 + 2·value stages.
const STAGE_NAMES: [&str; 3] = ["4 stages", "6 stages", "8 stages"];
/// The most stages the chain runs: the memory every instance holds.
const MOST_STAGES: usize = 8;

/// The break frequency goes no lower than this.
const LOWEST_HZ: f64 = 20.0;

/// The center control's frequency: 100 Hz at 0, 4 kHz at 1, evenly in octaves:
/// 100·2^(v·log2 40), log2 40 = 2 + log2 10.
fn center_hz(center: f32) -> f64 {
    100.0 * exp2(f64::from(center) * (2.0 + LOG2_10))
}

/// How many stages the stages control's `value` runs, clamped and rounded first.
fn stage_count(value: f32) -> usize {
    4 + 2 * PARAMS[STAGES].clamp(value) as usize
}

pub(super) struct Factory;

impl ProcessorFactory for Factory {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Phaser"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn shown(&self, index: usize, value: f32, _sample_rate: u32) -> f64 {
        match index {
            CENTER => center_hz(value),
            _ => f64::from(value),
        }
    }

    fn value_name(&self, index: usize, value: f32) -> Option<&'static str> {
        (index == STAGES).then(|| STAGE_NAMES[PARAMS[STAGES].clamp(value) as usize])
    }

    fn create(&self, sample_rate: u32, values: &[f32]) -> Box<dyn Processor> {
        let mut phaser = Phaser {
            lfo: Lfo::new(f64::from(PARAMS[RATE].default), sample_rate),
            stages: stage_count(PARAMS[STAGES].default),
            depth: f64::from(PARAMS[DEPTH].default),
            center: center_hz(PARAMS[CENTER].default),
            feedback: f64::from(PARAMS[FEEDBACK].default),
            offsets: [0.0, lfo_offset(PARAMS[SPREAD].default)],
            mix: PARAMS[MIX].default,
            inputs: [[0.0; MOST_STAGES]; 2],
            outputs: [[0.0; MOST_STAGES]; 2],
            last: [0.0; 2],
            sample_rate,
        };
        for (index, &value) in values.iter().enumerate() {
            phaser.set_param(index, value);
        }
        Box::new(phaser)
    }
}

struct Phaser {
    lfo: Lfo,
    stages: usize,
    depth: f64,
    /// The center control's frequency, in Hz.
    center: f64,
    feedback: f64,
    /// Each side's LFO offset: 0 on the left, the spread on the right.
    offsets: [f64; 2],
    mix: f32,
    /// Per side and stage, the stage's input a frame ago (x1).
    inputs: [[f64; MOST_STAGES]; 2],
    /// Per side and stage, the stage's output a frame ago (y1).
    outputs: [[f64; MOST_STAGES]; 2],
    /// Per side, the chain's output a frame ago, which the feedback adds to the input.
    last: [f64; 2],
    sample_rate: u32,
}

impl Phaser {
    /// This frame's allpass coefficient a on side `side`, from its angle of the sweep.
    fn coefficient(&self, side: usize) -> f64 {
        let rate = f64::from(self.sample_rate);
        let angle = self.lfo.phase() + self.offsets[side];
        let swept = self.center * exp2((self.depth / 2.0) * angle.sin());
        let f = swept.max(LOWEST_HZ).min(MAX_FREQ_FRACTION * rate);
        let t = ((PI * f) / rate).tan();
        (t - 1.0) / (t + 1.0)
    }

    /// Side `side`'s wet value for input `x` through the chain at coefficient `a`, advancing
    /// that side one frame.
    fn chain(&mut self, side: usize, x: f32, a: f64) -> f32 {
        let mut u = f64::from(x) + self.feedback * self.last[side];
        let inputs = &mut self.inputs[side][..self.stages];
        let outputs = &mut self.outputs[side][..self.stages];
        for (x1, y1) in inputs.iter_mut().zip(outputs.iter_mut()) {
            let y = ((a * u) + *x1) - (a * *y1);
            *x1 = snap64(u);
            *y1 = snap64(y);
            u = y;
        }
        self.last[side] = snap64(u);
        u as f32
    }
}

impl Processor for Phaser {
    fn process(&mut self, _context: &ProcessContext, buffer: &mut [[f32; 2]]) {
        for frame in buffer {
            let a = [self.coefficient(0), self.coefficient(1)];
            let wet = [self.chain(0, frame[0], a[0]), self.chain(1, frame[1], a[1])];
            self.lfo.advance();
            *frame = blend_power(*frame, wet, self.mix);
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(info) = PARAMS.get(index) else {
            return;
        };
        let value = info.clamp(value);
        match index {
            STAGES => self.stages = stage_count(value),
            RATE => self.lfo.set_rate(f64::from(value), self.sample_rate),
            DEPTH => self.depth = f64::from(value),
            CENTER => self.center = center_hz(value),
            FEEDBACK => self.feedback = f64::from(value),
            SPREAD => self.offsets[1] = lfo_offset(value),
            _ => self.mix = value,
        }
    }

    fn reset(&mut self) {
        self.lfo.reset();
        self.inputs = [[0.0; MOST_STAGES]; 2];
        self.outputs = [[0.0; MOST_STAGES]; 2];
        self.last = [0.0; 2];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::tests::{bits, noise, whole_and_split, CONTEXT};

    fn phaser(values: [f32; 7]) -> Box<dyn Processor> {
        Factory.create(44_100, &values)
    }

    // The center control by hand: 100·40^v Hz, so 100 Hz at 0, 100·√40 = 632.46 Hz at 0.5 and
    // 4 kHz at 1.
    #[test]
    fn the_center_sweeps_100_hz_to_4_khz_in_octaves() {
        assert_eq!(center_hz(0.0), 100.0);
        assert!((center_hz(0.5) - 100.0 * 40f64.sqrt()).abs() < 1e-9);
        assert!((center_hz(1.0) - 4000.0).abs() < 1e-9);
    }

    #[test]
    fn the_stages_are_named_and_counted() {
        let names: Vec<_> = (0..3)
            .map(|v| Factory.value_name(STAGES, v as f32))
            .collect();
        assert_eq!(
            names,
            [Some("4 stages"), Some("6 stages"), Some("8 stages")]
        );
        assert_eq!(Factory.value_name(STAGES, 9.0), Some("8 stages"));
        assert_eq!(Factory.value_name(CENTER, 0.5), None);
        assert_eq!([0.0, 1.0, 2.0, 1.4].map(stage_count), [4, 6, 8, 6]);
    }

    /// The amplitude of a 100 Hz sine (441 frames a period at 44.1 kHz) in `side` over the
    /// whole periods of [lo, hi): the projection onto sine and cosine.
    fn amplitude_100(side: &[f32], lo: usize, hi: usize) -> f64 {
        let (mut s, mut c) = (0.0, 0.0);
        for (n, &y) in side.iter().enumerate().take(hi).skip(lo) {
            let phase = std::f64::consts::TAU * 100.0 * n as f64 / 44_100.0;
            s += f64::from(y) * phase.sin();
            c += f64::from(y) * phase.cos();
        }
        2.0 * s.hypot(c) / (hi - lo) as f64
    }

    // With no sweep (depth 0) and the center at 0 (100 Hz), each stage lags a 100 Hz sine by
    // exactly 90°: 4 stages turn it a whole 360°, so at mix 50% dry and wet add, each at √0.5
    // (equal power: amplitude 0.5 in, 0.5·2·√0.5 = √0.5 out); 6 stages turn it 540°, against the
    // dry sound: the notch, nothing out; 8 stages 720°, whole again.
    #[test]
    fn a_still_chain_notches_its_center_at_6_stages() {
        let whole = 0.5f64.sqrt();
        for (stages, want) in [(0.0, whole), (1.0, 0.0), (2.0, whole)] {
            let mut frames: Vec<[f32; 2]> = (0..44_100)
                .map(|n| {
                    let x = 0.5 * (std::f64::consts::TAU * 100.0 * n as f64 / 44_100.0).sin();
                    [x as f32; 2]
                })
                .collect();
            phaser([stages, 0.5, 0.0, 0.0, 0.0, 90.0, 0.5]).process(&CONTEXT, &mut frames);
            let left: Vec<f32> = frames.iter().map(|frame| frame[0]).collect();
            let got = amplitude_100(&left, 22_050, 44_100);
            assert!((got - want).abs() < 1e-5, "stages {stages}: {got}");
        }
    }

    // Spread moves the right side's sweep only: a mono input comes out the same on both sides
    // at 0°, different at 90°, and the left side is the same bits at any spread.
    #[test]
    fn spread_offsets_the_right_sides_sweep_only() {
        let mono: Vec<[f32; 2]> = noise(4000).iter().map(|f| [f[0], f[0]]).collect();
        let run = |spread: f32| {
            let mut frames = mono.clone();
            phaser([1.0, 5.0, 3.0, 0.5, 0.5, spread, 0.5]).process(&CONTEXT, &mut frames);
            frames
        };
        let (still, wide) = (run(0.0), run(90.0));
        assert!(still.iter().all(|[l, r]| l.to_bits() == r.to_bits()));
        let differ = wide[1000..].iter().filter(|[l, r]| l != r).count();
        assert!(differ > 2900, "{differ} of 3000 frames differ");
        let left = |frames: &[[f32; 2]]| frames.iter().map(|f| f[0].to_bits()).collect::<Vec<_>>();
        assert_eq!(left(&still), left(&wide));
    }

    #[test]
    fn mix_0_passes_the_input_bit_for_bit() {
        let input = noise(1000);
        let mut out = input.clone();
        phaser([2.0, 3.0, 4.0, 0.7, -0.9, 90.0, 0.0]).process(&CONTEXT, &mut out);
        assert_eq!(bits(&out), bits(&input));
    }

    #[test]
    fn any_split_gives_the_same_bits() {
        for stages in 0..3 {
            let mut make = || phaser([stages as f32, 4.0, 3.0, 0.4, 0.8, 150.0, 0.6]);
            let (whole, split) = whole_and_split(&mut make, &noise(3000), &[1, 7, 512, 1, 999]);
            assert_eq!(bits(&whole), bits(&split), "stages {stages}");
        }
    }

    // Reset empties every stage and the feedback and puts the LFO's phase back at 0: what
    // follows is a fresh instance's render, bit for bit.
    #[test]
    fn reset_gives_a_fresh_instances_render() {
        let input = noise(2000);
        let mut fresh = input.clone();
        phaser([2.0, 5.0, 4.0, 0.3, -0.95, 30.0, 0.5]).process(&CONTEXT, &mut fresh);
        let mut instance = phaser([2.0, 5.0, 4.0, 0.3, -0.95, 30.0, 0.5]);
        let mut first = noise(1234);
        instance.process(&CONTEXT, &mut first);
        instance.reset();
        let mut again = input.clone();
        instance.process(&CONTEXT, &mut again);
        assert_eq!(bits(&again), bits(&fresh));
    }
}
