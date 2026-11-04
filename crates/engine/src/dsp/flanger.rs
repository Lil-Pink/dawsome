//! Flanger: the sound mixed with a copy of itself a few milliseconds late, the delay swept by a
//! slow sine (the LFO), part of the delayed copy fed back into the line (J. O. Smith, "Flanging",
//! Physical Audio Signal Processing; the shape of Faust's `flanger_mono`; re-derived, no code
//! copied). The right side's sweep runs Spread (0° to 180°, 90° by default) ahead of the left's,
//! so the flange moves across the stereo field; at 0° both sides share one sweep (a mono flange).
//!
//! Per frame and side s: angle = phase on the left, phase + spread·π/180 on the right;
//! τ = max(2, delay·sr/1000 · (1 − depth·(1 − sin(angle))/2)) frames, so the sweep runs from the
//! set delay at the LFO's top down by depth's share of it (100% reaches 0, stopped at 2 frames,
//! the line's shortest read). d = the line's read τ back (a 4-point Hermite between frames, read
//! before the write); the line takes x + feedback·d, rounded to f32 once (tiny values to 0); the
//! wet side is d. The mix is [`blend_power`] (equal power). Mix 50% gives the deepest notches
//! (dry and delayed equal); |feedback| < 1 keeps it stable.
//!
//! Why Spread and equal power: the professional flangers it was
//! measured against all offset the sides' sweeps; a linear mix lost 1.6 to 4.6 dB at 50%.
//!
//! Operation order (gen_p32_expected.py states it, and the Rust follows): the right side's
//! offset (spread·π)/180; τ = max(2, ((delay·sr)/1000)·(1 − (depth·(1 − sin(angle)))/2)); the
//! write's sum x + feedback·d in f64, not f32 as the delay effect's.

use super::parts::{lfo_offset, DelayLine, Lfo};
use super::{blend_power, snap};
use crate::processor::{ParamInfo, ProcessContext, Processor, ProcessorFactory, Taper};

pub const ID: &str = "dawsome.flanger";

const PARAMS: [ParamInfo; 6] = [
    ParamInfo {
        key: "delay_ms",
        label: "Delay",
        min: 0.1,
        max: 10.0,
        default: 2.0,
        unit: "ms",
        whole: false,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "depth",
        label: "Depth",
        min: 0.0,
        max: 1.0,
        default: 0.5,
        unit: "%",
        whole: false,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "rate",
        label: "Rate",
        min: 0.05,
        max: 5.0,
        default: 0.3,
        unit: "Hz",
        whole: false,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "feedback",
        label: "Feedback",
        min: -0.95,
        max: 0.95,
        default: 0.5,
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

const DELAY_MS: usize = 0;
const DEPTH: usize = 1;
const RATE: usize = 2;
const FEEDBACK: usize = 3;
const SPREAD: usize = 4;
const MIX: usize = 5;

/// The shortest read of the line, in frames: the read takes the frame before the delayed
/// point too, which must already be written.
const SHORTEST: f64 = 2.0;

pub(super) struct Factory;

impl ProcessorFactory for Factory {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Flanger"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn create(&self, sample_rate: u32, values: &[f32]) -> Box<dyn Processor> {
        let mut flanger = Flanger {
            line: DelayLine::new(f64::from(PARAMS[DELAY_MS].max), sample_rate),
            lfo: Lfo::new(f64::from(PARAMS[RATE].default), sample_rate),
            delay: delay_frames(PARAMS[DELAY_MS].default, sample_rate),
            depth: f64::from(PARAMS[DEPTH].default),
            feedback: f64::from(PARAMS[FEEDBACK].default),
            offsets: [0.0, lfo_offset(PARAMS[SPREAD].default)],
            mix: PARAMS[MIX].default,
            sample_rate,
        };
        for (index, &value) in values.iter().enumerate() {
            flanger.set_param(index, value);
        }
        Box::new(flanger)
    }
}

/// The delay control's `delay_ms` in frames at `sample_rate`: (delay·sr)/1000.
fn delay_frames(delay_ms: f32, sample_rate: u32) -> f64 {
    (f64::from(delay_ms) * f64::from(sample_rate)) / 1000.0
}

struct Flanger {
    line: DelayLine,
    lfo: Lfo,
    /// The set delay, in frames: the sweep's top.
    delay: f64,
    depth: f64,
    feedback: f64,
    /// Each side's LFO offset: 0 on the left, the spread on the right.
    offsets: [f64; 2],
    mix: f32,
    sample_rate: u32,
}

impl Flanger {
    /// This frame's read of side `side`: the delay its angle of the sweep gives.
    fn read(&self, side: usize) -> f32 {
        let angle = self.lfo.phase() + self.offsets[side];
        let dip = (self.depth * (1.0 - angle.sin())) / 2.0;
        let delay = (self.delay * (1.0 - dip)).max(SHORTEST);
        self.line.read(side, delay)
    }
}

impl Processor for Flanger {
    fn process(&mut self, _context: &ProcessContext, buffer: &mut [[f32; 2]]) {
        for frame in buffer {
            let wet = [self.read(0), self.read(1)];
            let fed = |side: usize| {
                let sum = f64::from(frame[side]) + self.feedback * f64::from(wet[side]);
                snap(sum as f32)
            };
            self.line.write([fed(0), fed(1)]);
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
            DELAY_MS => self.delay = delay_frames(value, self.sample_rate),
            DEPTH => self.depth = f64::from(value),
            RATE => self.lfo.set_rate(f64::from(value), self.sample_rate),
            FEEDBACK => self.feedback = f64::from(value),
            SPREAD => self.offsets[1] = lfo_offset(value),
            _ => self.mix = value,
        }
    }

    fn reset(&mut self) {
        self.line.reset();
        self.lfo.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::tests::{bits, impulse, noise, whole_and_split, CONTEXT};

    fn flanger(values: [f32; 6]) -> Box<dyn Processor> {
        Factory.create(44_100, &values)
    }

    /// An impulse through a 10 ms comb (441 frames at 44.1 kHz, depth 0: no sweep) with
    /// `feedback`, wet alone.
    fn comb(feedback: f32) -> Vec<[f32; 2]> {
        let mut frames = impulse(2000);
        flanger([10.0, 0.0, 0.3, feedback, 90.0, 1.0]).process(&CONTEXT, &mut frames);
        frames
    }

    // Depth 0 is a fixed whole delay of 441 frames: the impulse comes back at 441, 882, 1323
    // and 1764, each echo the one before times the feedback (0.5: 1, 0.5, 0.25, 0.125, exact in
    // f32), and every other frame is exactly 0 (a whole delay reads its frame's bits).
    #[test]
    fn at_depth_0_an_impulse_echoes_down_by_the_feedback() {
        let frames = comb(0.5);
        for (n, frame) in frames.iter().enumerate() {
            let want = match n {
                441 => 1.0,
                882 => 0.5,
                1323 => 0.25,
                1764 => 0.125,
                _ => 0.0,
            };
            assert_eq!(
                frame.map(f32::to_bits),
                [want; 2].map(f32::to_bits),
                "frame {n}"
            );
        }
    }

    // Negative feedback flips each echo's sign: 1, −0.5, 0.25.
    #[test]
    fn negative_feedback_alternates_the_echoes() {
        let frames = comb(-0.5);
        assert_eq!(frames[441], [1.0; 2]);
        assert_eq!(frames[882], [-0.5; 2]);
        assert_eq!(frames[1323], [0.25; 2]);
    }

    // Spread moves the right side's sweep only: a mono input comes out the same on both sides
    // at 0°, different at 90°, and the left side is the same bits at any spread.
    #[test]
    fn spread_offsets_the_right_sides_sweep_only() {
        let mono: Vec<[f32; 2]> = noise(4000).iter().map(|f| [f[0], f[0]]).collect();
        let run = |spread: f32| {
            let mut frames = mono.clone();
            flanger([2.0, 0.8, 5.0, 0.5, spread, 0.5]).process(&CONTEXT, &mut frames);
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
        flanger([4.0, 1.0, 2.0, 0.9, 90.0, 0.0]).process(&CONTEXT, &mut out);
        assert_eq!(bits(&out), bits(&input));
    }

    #[test]
    fn any_split_gives_the_same_bits() {
        let mut make = || flanger([3.0, 0.8, 4.0, -0.7, 120.0, 0.6]);
        let (whole, split) = whole_and_split(&mut make, &noise(3000), &[1, 7, 512, 1, 999]);
        assert_eq!(bits(&whole), bits(&split));
    }

    // Reset empties the line and puts the LFO's phase back at 0: what follows is a fresh
    // instance's render, bit for bit.
    #[test]
    fn reset_gives_a_fresh_instances_render() {
        let input = noise(2000);
        let mut fresh = input.clone();
        flanger([7.0, 0.9, 5.0, 0.9, 45.0, 0.5]).process(&CONTEXT, &mut fresh);
        let mut instance = flanger([7.0, 0.9, 5.0, 0.9, 45.0, 0.5]);
        let mut first = noise(1234);
        instance.process(&CONTEXT, &mut first);
        instance.reset();
        let mut again = input.clone();
        instance.process(&CONTEXT, &mut again);
        assert_eq!(bits(&again), bits(&fresh));
    }
}
