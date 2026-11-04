//! Chorus: three copies of the sound, each read from a delay line 15 ms back, the delay of each
//! swept up and down by one slow sine (the LFO) at its own phase, 0°, 120° and 240°, the right
//! side's sweep a further 90° on; the three are summed over √3, high-passed at 150 Hz and mixed
//! with the dry sound at equal power. No feedback.
//!
//! Per frame n and side s, voice v reads τ = (15 + 5·depth·sin(phase + v·2π/3 + s·π/2)) ms back
//! (10 to 20 ms at depth 100%), by the parts' [`DelayLine`] (a 4-point Hermite between frames,
//! read before the frame's input is written); the wet side is the three reads' sum over √3, then
//! the parts' [`Svf`] high-pass at 150 Hz (Q 1/√2), rounded to f32 once; the LFO then steps by
//! 2π·rate/sr; the mix is [`blend_power`]. Why three voices 120° apart and the right side 90° on:
//! the classic ensemble spread, and a mono source comes out wide.
//!
//! Why the sum over √3 and the equal-power mix: swept voices are
//! uncorrelated, so their powers add and √3 keeps the input's level; averaged (÷3) and mixed
//! linearly the chorus was 4–5 dB quieter than the sound it was put on. Why the high-pass: at
//! that level the chorused bass smeared the low end (+0.8 to +2.5 dB on a bass line), which is
//! why other DAWs' choruses filter their wet sound too. Why 0.5 Hz and 30% by default:
//! 1.5 ms of sweep wobbles the pitch about 8 cents, as the Roland Juno-60's chorus (6.5 and 11
//! cents) does; 0.8 Hz and 50% wobbled 22.
//!
//! Operation order (gen_p32_expected.py states it, and the Rust follows): τ in frames =
//! ((15 + (5·depth)·sin((phase + (v·2π)/3) + (s·2π)/4))·sr)/1000; wet =
//! f32(HP(((r0 + r1) + r2)/√3)), √3 the f64 nearest it.

use std::f64::consts::{FRAC_1_SQRT_2, TAU};

use super::blend_power;
use super::parts::{DelayLine, Lfo, Response, Svf};
use crate::processor::{ParamInfo, ProcessContext, Processor, ProcessorFactory, Taper};

pub const ID: &str = "dawsome.chorus";

const PARAMS: [ParamInfo; 3] = [
    ParamInfo {
        key: "rate",
        label: "Rate",
        min: 0.1,
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
        max: 1.0,
        default: 0.3,
        unit: "%",
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

const RATE: usize = 0;
const DEPTH: usize = 1;
const MIX: usize = 2;

/// Every voice's delay at the LFO's zero crossing, in milliseconds.
const BASE_MS: f64 = 15.0;
/// How far depth 100% sweeps a voice's delay either side of [`BASE_MS`].
const SWEEP_MS: f64 = 5.0;
/// The longest delay a voice reads, [`BASE_MS`] + [`SWEEP_MS`]: the line's length.
const LINE_MS: f64 = 20.0;

/// The voices' LFO phases, (v·2π)/3 for v = 0, 1, 2 (0°, 120°, 240°).
const VOICE_PHASES: [f64; 3] = [0.0 * TAU / 3.0, 1.0 * TAU / 3.0, 2.0 * TAU / 3.0];
/// Each side's further phase, (s·2π)/4: the right side 90° on.
const SIDE_PHASES: [f64; 2] = [0.0 * TAU / 4.0, 1.0 * TAU / 4.0];

/// √3, the f64 nearest it (std has no constant): the voices' sum over it keeps the level of
/// three uncorrelated voices at one's.
const SQRT_3: f64 = 1.732_050_807_568_877_2;
/// The wet sound's high-pass: below it the chorus leaves the sound dry.
const LOW_CUT_HZ: f64 = 150.0;

pub(super) struct Factory;

impl ProcessorFactory for Factory {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Chorus"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn create(&self, sample_rate: u32, values: &[f32]) -> Box<dyn Processor> {
        let mut chorus = Chorus {
            line: DelayLine::new(LINE_MS, sample_rate),
            low_cut: Svf::new(LOW_CUT_HZ, FRAC_1_SQRT_2, sample_rate),
            lfo: Lfo::new(f64::from(PARAMS[RATE].default), sample_rate),
            depth: f64::from(PARAMS[DEPTH].default),
            mix: PARAMS[MIX].default,
            sample_rate,
        };
        for (index, &value) in values.iter().enumerate() {
            chorus.set_param(index, value);
        }
        Box::new(chorus)
    }
}

struct Chorus {
    line: DelayLine,
    /// The wet sound's high-pass, both sides.
    low_cut: Svf,
    lfo: Lfo,
    depth: f64,
    mix: f32,
    sample_rate: u32,
}

impl Chorus {
    /// This frame's wet side `side`: the three voices' reads summed over √3, high-passed,
    /// advancing that side's high-pass one frame.
    fn wet(&mut self, side: usize) -> f32 {
        let phase = self.lfo.phase();
        let rate = f64::from(self.sample_rate);
        let read = |voice: usize| {
            let sweep = (phase + VOICE_PHASES[voice]) + SIDE_PHASES[side];
            let delay = ((BASE_MS + (SWEEP_MS * self.depth) * sweep.sin()) * rate) / 1000.0;
            f64::from(self.line.read(side, delay))
        };
        let sum = ((read(0) + read(1)) + read(2)) / SQRT_3;
        self.low_cut.process(side, sum, Response::HighPass) as f32
    }
}

impl Processor for Chorus {
    fn process(&mut self, _context: &ProcessContext, buffer: &mut [[f32; 2]]) {
        for frame in buffer {
            let wet = [self.wet(0), self.wet(1)];
            self.line.write(*frame);
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
            RATE => self.lfo.set_rate(f64::from(value), self.sample_rate),
            DEPTH => self.depth = f64::from(value),
            _ => self.mix = value,
        }
    }

    fn reset(&mut self) {
        self.line.reset();
        self.low_cut.reset();
        self.lfo.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::tests::{bits, impulse, noise, whole_and_split, CONTEXT};

    fn chorus(sample_rate: u32, values: [f32; 3]) -> Box<dyn Processor> {
        Factory.create(sample_rate, &values)
    }

    /// What the still chorus (depth 0, mix 100%) gives for `reads`, one voice's read per
    /// frame: the three voices' sum over √3 through a fresh 150 Hz high-pass, to f32.
    fn still_wet(reads: &[f32], sample_rate: u32) -> Vec<f32> {
        let mut low_cut = Svf::new(LOW_CUT_HZ, FRAC_1_SQRT_2, sample_rate);
        reads
            .iter()
            .map(|&read| {
                let read = f64::from(read);
                let sum = ((read + read) + read) / SQRT_3;
                low_cut.process(0, sum, Response::HighPass) as f32
            })
            .collect()
    }

    // At depth 0 every voice reads 15 ms back: 720 frames at 48 kHz, a whole delay, so each
    // voice reads the impulse on frame 720 alone, with its bits; the wet sound is three of it
    // over √3 through the high-pass, which rings on after it, and every frame before 720 is
    // exactly 0 (mix 100%).
    #[test]
    fn at_depth_0_an_impulse_comes_back_15_ms_later() {
        let mut frames = impulse(2000);
        chorus(48_000, [0.8, 0.0, 1.0]).process(&CONTEXT, &mut frames);
        let mut reads = vec![0.0; 2000];
        reads[720] = 1.0;
        let want = still_wet(&reads, 48_000);
        assert!(frames[..720].iter().all(|frame| *frame == [0.0; 2]));
        assert!(
            want[720] > 1.5,
            "√3 times the impulse, less a little: {}",
            want[720]
        );
        for (n, frame) in frames.iter().enumerate() {
            assert_eq!(frame[0].to_bits(), want[n].to_bits(), "left, frame {n}");
            assert_eq!(frame[1].to_bits(), want[n].to_bits(), "right, frame {n}");
        }
    }

    // At 44.1 kHz 15 ms is 661.5 frames: the Hermite halfway between two frames weighs the four
    // around it −1/16, 9/16, 9/16, −1/16, so each voice reads the impulse over frames 660 to
    // 663, and the wet sound is those reads through the sum and the high-pass.
    #[test]
    fn at_depth_0_a_half_frame_delay_reads_between_frames() {
        let mut frames = impulse(1000);
        chorus(44_100, [0.8, 0.0, 1.0]).process(&CONTEXT, &mut frames);
        let mut reads = vec![0.0; 1000];
        reads[660..664].copy_from_slice(&[-0.0625, 0.5625, 0.5625, -0.0625]);
        let want = still_wet(&reads, 44_100);
        let left: Vec<u32> = frames.iter().map(|frame| frame[0].to_bits()).collect();
        assert_eq!(left, want.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
        assert!(left[..660].iter().all(|&v| v == 0));
    }

    // Below the high-pass the wet sound fades: DC held long enough leaves the still chorus as
    // nothing (wet only), where the dry sound is all of it.
    #[test]
    fn the_wet_sound_leaves_the_lows_out() {
        let mut frames = vec![[0.5; 2]; 44_100];
        chorus(44_100, [0.5, 0.0, 1.0]).process(&CONTEXT, &mut frames);
        let tail = frames[44_000..]
            .iter()
            .flatten()
            .fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(tail < 1e-6, "{tail}");
    }

    // The right side's sweep runs 90° ahead of the left's, so a mono input comes out different
    // on each side once the voices sound.
    #[test]
    fn a_mono_input_comes_out_wide() {
        let mut frames: Vec<[f32; 2]> = noise(4000).iter().map(|f| [f[0], f[0]]).collect();
        chorus(44_100, [0.8, 1.0, 1.0]).process(&CONTEXT, &mut frames);
        let differ = frames[1000..].iter().filter(|[l, r]| l != r).count();
        assert!(differ > 2900, "{differ} of 3000 frames differ");
    }

    #[test]
    fn mix_0_passes_the_input_bit_for_bit() {
        let input = noise(1000);
        let mut out = input.clone();
        chorus(44_100, [3.0, 1.0, 0.0]).process(&CONTEXT, &mut out);
        assert_eq!(bits(&out), bits(&input));
    }

    #[test]
    fn any_split_gives_the_same_bits() {
        let mut make = || chorus(44_100, [4.5, 0.9, 0.7]);
        let (whole, split) = whole_and_split(&mut make, &noise(3000), &[1, 7, 512, 1, 999]);
        assert_eq!(bits(&whole), bits(&split));
    }

    // Reset empties the line and puts the LFO's phase back at 0: what follows is a fresh
    // instance's render, bit for bit.
    #[test]
    fn reset_gives_a_fresh_instances_render() {
        let input = noise(2000);
        let mut fresh = input.clone();
        chorus(44_100, [5.0, 1.0, 0.6]).process(&CONTEXT, &mut fresh);
        let mut instance = chorus(44_100, [5.0, 1.0, 0.6]);
        let mut first = noise(1234);
        instance.process(&CONTEXT, &mut first);
        instance.reset();
        let mut again = input.clone();
        instance.process(&CONTEXT, &mut again);
        assert_eq!(bits(&again), bits(&fresh));
    }
}
