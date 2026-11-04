//! Distortion: the input driven into tanh, shaped with first-order antiderivative
//! anti-aliasing (ADAA: Parker, Zavalishin, Le Bivic, "Reducing the aliasing of nonlinear
//! waveshaping using continuous-time convolution", DAFx-16; Bilbao, Esqueda, Parker, Välimäki,
//! "Antiderivative antialiasing for memoryless nonlinearities", IEEE SPL 24(7), 2017; re-derived,
//! no code copied), then Auto gain, a one-pole low-pass (the tone), an output gain and the mix.
//!
//! Per side, with u = lin(drive)·x and u1 the frame before's: the shaper's output is the mean of
//! tanh between u1 and u, (F(u) − F(u1))/(u − u1) for F = ln cosh, written overflow-safe as
//! F(u) = |u| + ln(1 + e^(−2|u|)) − ln 2; where u and u1 are within 1e-5 of each other that
//! quotient loses its digits, and tanh((u + u1)/2) takes its place. Auto gain (on by default)
//! scales that by lin(−d/(1 + d/40)) for a drive of d dB (−9.2 dB at the default 12, −20 dB at
//! 40); off, by 1. Then lp += c·(y − lp) with c = 1 − e^(−2π·fc/rate), and the wet side is
//! lin(output)·lp, rounded to f32 once.
//!
//! Why Auto gain: driven into tanh, a sound gets louder by about
//! d/(1 + d/40) dB (measured on drums, bass, piano and pad: +10 dB at the default drive), so
//! every comparison with the dry sound was unfair and the master limiter took the rest. The fit
//! keeps the level within about 1.3 dB of the measured mean at every drive; the materials spread
//! ±4 dB about it at 40 dB, so it keeps the level roughly, not exactly.
//!
//! Why ADAA and not oversampling: no latency (Dawsome compensates none), one sample of state, and
//! about as little aliasing as 2× oversampling. Its cost: the shaper's output lags half a frame,
//! so at a partial mix the top of the band dips a little (−2.4 dB at 10 kHz, 44.1 kHz, mix 50%).
//! The tone at its top (20 kHz) is still a gentle one-pole, not a bypass. Silence stays exactly
//! 0: u = u1 = 0 gives tanh 0, and the tone's tail snaps to 0.
//!
//! Operation order (gen_p32_expected.py states it, and the Rust follows): lin(dB) =
//! exp2((dB/20)·log2 10) in f64; F(u) = (|u| + ln_1p(exp(−2|u|))) − ln 2; the makeup
//! lin(−(d/(1 + d/40))), y·makeup; tone fc = 1000·exp2(v·(1 + log2 10));
//! c = 1 − exp2(−(((2π·fc)/rate)·log2 e)).

use std::f64::consts::{LN_2, LOG2_10, LOG2_E, TAU};

use super::{blend, exp2, snap64};
use crate::processor::{ParamInfo, ProcessContext, Processor, ProcessorFactory, Taper};

pub const ID: &str = "dawsome.distortion";

const PARAMS: [ParamInfo; 5] = [
    ParamInfo {
        key: "drive_db",
        label: "Drive",
        min: 0.0,
        max: 40.0,
        default: 12.0,
        unit: "dB",
        whole: false,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "tone",
        label: "Tone",
        min: 0.0,
        max: 1.0,
        default: 1.0,
        unit: "Hz",
        whole: false,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "output_db",
        label: "Output",
        min: -24.0,
        max: 6.0,
        default: 0.0,
        unit: "dB",
        whole: false,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "auto_gain",
        label: "Auto gain",
        min: 0.0,
        max: 1.0,
        default: 1.0,
        unit: "",
        whole: true,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "mix",
        label: "Mix",
        min: 0.0,
        max: 1.0,
        default: 1.0,
        unit: "%",
        whole: false,
        taper: Taper::Linear,
    },
];

const DRIVE_DB: usize = 0;
const TONE: usize = 1;
const OUTPUT_DB: usize = 2;
const AUTO_GAIN: usize = 3;
const MIX: usize = 4;

/// The auto gain control's choices by value.
const AUTO_GAIN_NAMES: [&str; 2] = ["Off", "On"];

/// Below this difference between a frame's driven input and the one before, the shaper takes
/// tanh of their mean: the antiderivatives' quotient would be mostly rounding.
const SMALL_DIFFERENCE: f64 = 1e-5;

/// 10^(dB/20), by [`exp2`], kept in f64.
fn lin(db: f32) -> f64 {
    exp2(f64::from(db) / 20.0 * LOG2_10)
}

/// Auto gain's scale for a drive of `drive_db` when `on`: lin(−d/(1 + d/40)), the fit
/// of how much louder tanh makes a sound at that drive; 1 when off.
fn makeup(drive_db: f32, on: bool) -> f64 {
    if !on {
        return 1.0;
    }
    let drive = f64::from(drive_db);
    exp2(-(drive / (1.0 + drive / 40.0)) / 20.0 * LOG2_10)
}

/// The tone control's frequency: 1 kHz at 0, 20 kHz at 1, evenly in octaves:
/// 1000·2^(v·log2 20).
fn tone_hz(tone: f32) -> f64 {
    1000.0 * exp2(f64::from(tone) * (1.0 + LOG2_10))
}

/// The tone's one-pole coefficient at `tone` in a project at `sample_rate`: 1 − e^(−2π·fc/rate).
fn tone_coefficient(tone: f32, sample_rate: u32) -> f64 {
    let rate = f64::from(sample_rate.max(1));
    1.0 - exp2(-(TAU * tone_hz(tone) / rate * LOG2_E))
}

/// ln cosh u, the antiderivative of tanh, without overflow for any finite u.
fn ln_cosh(u: f64) -> f64 {
    let magnitude = u.abs();
    magnitude + (-2.0 * magnitude).exp().ln_1p() - LN_2
}

pub(super) struct Factory;

impl ProcessorFactory for Factory {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Distortion"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn shown(&self, index: usize, value: f32, _sample_rate: u32) -> f64 {
        match index {
            TONE => tone_hz(value),
            _ => f64::from(value),
        }
    }

    fn value_name(&self, index: usize, value: f32) -> Option<&'static str> {
        (index == AUTO_GAIN).then(|| AUTO_GAIN_NAMES[PARAMS[AUTO_GAIN].clamp(value) as usize])
    }

    fn create(&self, sample_rate: u32, values: &[f32]) -> Box<dyn Processor> {
        let mut distortion = Distortion {
            drive_db: PARAMS[DRIVE_DB].default,
            auto_gain: PARAMS[AUTO_GAIN].default == 1.0,
            makeup: 1.0,
            drive: lin(PARAMS[DRIVE_DB].default),
            tone: tone_coefficient(PARAMS[TONE].default, sample_rate),
            output: lin(PARAMS[OUTPUT_DB].default),
            mix: PARAMS[MIX].default,
            driven: [0.0; 2],
            low: [0.0; 2],
            sample_rate,
        };
        for (index, &value) in values.iter().enumerate() {
            distortion.set_param(index, value);
        }
        distortion.makeup = makeup(distortion.drive_db, distortion.auto_gain);
        Box::new(distortion)
    }
}

struct Distortion {
    /// The drive control, which the makeup follows.
    drive_db: f32,
    auto_gain: bool,
    /// Auto gain's scale after the shaper ([`makeup`]).
    makeup: f64,
    drive: f64,
    /// The tone's one-pole coefficient.
    tone: f64,
    output: f64,
    mix: f32,
    /// Per side, the frame before's driven input (u1).
    driven: [f64; 2],
    /// Per side, the tone's memory.
    low: [f64; 2],
    sample_rate: u32,
}

impl Distortion {
    /// Side `side`'s wet value for input `x`, advancing that side one frame.
    fn shape(&mut self, side: usize, x: f32) -> f32 {
        let u = self.drive * f64::from(x);
        let before = self.driven[side];
        let difference = u - before;
        let y = if difference.abs() < SMALL_DIFFERENCE {
            ((u + before) / 2.0).tanh()
        } else {
            (ln_cosh(u) - ln_cosh(before)) / difference
        };
        self.driven[side] = u;
        let y = y * self.makeup;
        let low = &mut self.low[side];
        *low = snap64(*low + self.tone * (y - *low));
        (self.output * *low) as f32
    }
}

impl Processor for Distortion {
    fn process(&mut self, _context: &ProcessContext, buffer: &mut [[f32; 2]]) {
        for frame in buffer {
            let wet = [self.shape(0, frame[0]), self.shape(1, frame[1])];
            *frame = blend(*frame, wet, self.mix);
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(info) = PARAMS.get(index) else {
            return;
        };
        let value = info.clamp(value);
        match index {
            DRIVE_DB => {
                self.drive_db = value;
                self.drive = lin(value);
            }
            TONE => self.tone = tone_coefficient(value, self.sample_rate),
            OUTPUT_DB => self.output = lin(value),
            AUTO_GAIN => self.auto_gain = value == 1.0,
            _ => self.mix = value,
        }
        self.makeup = makeup(self.drive_db, self.auto_gain);
    }

    fn reset(&mut self) {
        self.driven = [0.0; 2];
        self.low = [0.0; 2];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::tests::{bits, noise, whole_and_split, CONTEXT};

    fn distortion(values: [f32; 5]) -> Box<dyn Processor> {
        Factory.create(44_100, &values)
    }

    // ln cosh by hand: 0 at 0, ln cosh 1 = 0.433780830483027, and |u| − ln 2 far out (cosh u
    // ≈ e^|u|/2), where cosh itself would overflow f64 (u = 1000).
    #[test]
    fn ln_cosh_is_the_antiderivative_of_tanh_without_overflow() {
        assert!(ln_cosh(0.0).abs() < 1e-16);
        assert!((ln_cosh(1.0) - 0.433_780_830_483_027).abs() < 1e-15);
        assert!((ln_cosh(-1.0) - 0.433_780_830_483_027).abs() < 1e-15);
        assert_eq!(ln_cosh(1000.0), 1000.0 - LN_2);
    }

    // The tone's ends: 1 kHz and 20 kHz (2^(log2 20) = 20, by exp2, to an f64 step).
    #[test]
    fn the_tone_runs_from_1_khz_to_20_khz() {
        assert_eq!(tone_hz(0.0), 1000.0);
        assert!((tone_hz(1.0) - 20_000.0).abs() < 1e-9);
        assert!((tone_hz(0.5) - 1000.0 * 20f64.sqrt()).abs() < 1e-9);
    }

    // A constant input settles on tanh of its driven value: the shaper's mean of tanh over a
    // step of zero is tanh itself, and the one-pole passes DC. Drive 0 dB, output 0 dB, mix 1.
    #[test]
    fn a_constant_settles_on_its_tanh() {
        let mut frames = vec![[0.5, -0.25]; 2000];
        distortion([0.0, 1.0, 0.0, 1.0, 1.0]).process(&CONTEXT, &mut frames);
        let [l, r] = frames[1999].map(f64::from);
        assert!((l - 0.5f64.tanh()).abs() < 1e-7, "{l}");
        assert!((r - (-0.25f64).tanh()).abs() < 1e-7, "{r}");
    }

    // Driven hard, nothing leaves past 1 (tanh's ceiling, and the mean of tanh between two
    // points lies between their tanh); silence after it decays to exactly 0.
    #[test]
    fn hard_drive_stays_within_1_and_silence_returns_to_0() {
        let mut frames: Vec<[f32; 2]> = noise(4000).iter().map(|f| f.map(|v| v * 2.0)).collect();
        frames.extend(vec![[0.0; 2]; 200]);
        distortion([40.0, 1.0, 0.0, 1.0, 1.0]).process(&CONTEXT, &mut frames);
        assert!(frames.iter().flatten().all(|v| v.abs() <= 1.0));
        assert!(frames[4100..].iter().all(|f| *f == [0.0; 2]));
    }

    // The makeup by hand: 10^(−(d/(1 + d/40))/20): 1 at 0 dB (−0 dB is 1 exactly), 10^(−12/26)
    // at 12 dB (−9.23 dB), 10^(−1) at 40 dB (−20 dB); off is 1 at any drive.
    #[test]
    fn auto_gain_takes_the_drives_rise_back() {
        assert_eq!(makeup(0.0, true), 1.0);
        assert!((makeup(12.0, true) - 10f64.powf(-12.0 / 26.0)).abs() < 1e-15);
        assert!((makeup(40.0, true) - 0.1).abs() < 1e-16);
        assert_eq!(makeup(40.0, false), 1.0);
        assert_eq!(Factory.value_name(AUTO_GAIN, 0.0), Some("Off"));
        assert_eq!(Factory.value_name(AUTO_GAIN, 1.0), Some("On"));
        assert_eq!(Factory.value_name(DRIVE_DB, 1.0), None);
    }

    // Everything after the shaper is linear, so auto gain on is auto gain off times the makeup,
    // to an f32 rounding or two.
    #[test]
    fn auto_gain_scales_the_wet_sound() {
        let input: Vec<[f32; 2]> = noise(2000);
        let (mut on, mut off) = (input.clone(), input);
        distortion([18.0, 0.6, 0.0, 1.0, 1.0]).process(&CONTEXT, &mut on);
        distortion([18.0, 0.6, 0.0, 0.0, 1.0]).process(&CONTEXT, &mut off);
        let scale = makeup(18.0, true);
        for (a, b) in on.iter().flatten().zip(off.iter().flatten()) {
            assert!(
                (f64::from(*a) - scale * f64::from(*b)).abs() < 1e-7,
                "{a} {b}"
            );
        }
    }

    #[test]
    fn mix_0_passes_the_input_bit_for_bit() {
        let input = noise(1000);
        let mut out = input.clone();
        distortion([30.0, 0.3, 6.0, 1.0, 0.0]).process(&CONTEXT, &mut out);
        assert_eq!(bits(&out), bits(&input));
    }

    #[test]
    fn any_split_gives_the_same_bits() {
        let mut make = || distortion([24.0, 0.4, -3.0, 1.0, 0.7]);
        let (whole, split) = whole_and_split(&mut make, &noise(3000), &[1, 7, 512, 1, 999]);
        assert_eq!(bits(&whole), bits(&split));
    }

    #[test]
    fn reset_clears_the_shaper_and_the_tone() {
        let mut instance = distortion([12.0, 0.2, 0.0, 1.0, 1.0]);
        let mut loud = noise(100);
        instance.process(&CONTEXT, &mut loud);
        instance.reset();
        let mut silence = vec![[0.0; 2]; 4];
        instance.process(&CONTEXT, &mut silence);
        assert_eq!(silence, vec![[0.0; 2]; 4]);
    }
}
