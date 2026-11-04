//! The filter: low-pass, high-pass, band-pass or notch, 12 or 24 dB per octave, on the parts'
//! state-variable filter ([`Svf`], Simper's trapezoidal SVF). No mix: a filter's dry signal is
//! what its type leaves out.
//!
//! The cutoff is a 0..1 control swept evenly in octaves, 20 Hz to 20 kHz:
//! fc = 20·2^(v·log2 1000) (632 Hz at the middle); the resonance likewise, Q = 0.5·2^(r·log2 40)
//! from 0.5 to 20 (0.723 at the default 10%, near Butterworth's 0.707). Why exponential Q: it is
//! heard by ratio, and a linear 0.5–20 would put every useful setting in its first quarter.
//!
//! The 12 dB slope is one SVF at Q; the 24 dB slope two in series, the same cutoff and the same Q
//! on both, √Q each, so the pair's gain at the cutoff is Q, as one stage's is: the two slopes
//! meet there, and one level rule serves both. (At Q on each the pair would ring Q² = 400 times
//! the input at 100%, +52 dB.)
//!
//! Resonance is level-compensated above the default (clean, no saturation): the low- and
//! high-pass, which ring Q times the input at their cutoff, are scaled by min(1, √(Q0/Q)), Q0
//! the default resonance's Q (0.723): exactly 1 at the default and below it, so the default
//! filter's passband stays at 0 dB, and √(Q0/20) = −14.4 dB at 100%, where a sine at the cutoff
//! then comes out √(Q0·20) = +11.6 dB. Why: uncompensated, a
//! −6 dBFS saw chord swept at full resonance peaked 13 dB over its input at 12 dB and 16 dB at
//! 24 dB, so the master limiter did the work. Why √ and not 1/Q: 1/Q keeps the peak at the
//! input's level but drops everything else by 26 dB. The band-pass is unity at its cutoff
//! (resonance narrows it without raising it) and the notch never rises, so they are not
//! scaled.
//!
//! The coefficients are set when a parameter changes, not per frame, from the cutoff clamped to
//! 20 Hz .. 0.45·rate. Operation order (gen_p32_expected.py states it): the constant log2 1000 =
//! 3·log2 10 is formed first, then fc = 20·exp2(v·log2 1000); Q = 0.5·exp2(r·(2 + log2 10)); a
//! 24 dB stage's Q √Q; Q0 = 0.5·exp2(0.1·(2 + log2 10)), the default (an f32) through the same
//! formula; the scale min(1, √(Q0/Q)); out = f32(y·scale).

use std::f64::consts::LOG2_10;

use super::exp2;
use super::parts::{Response, Svf};
use crate::processor::{ParamInfo, ProcessContext, Processor, ProcessorFactory, Taper};

pub const ID: &str = "dawsome.filter";

const PARAMS: [ParamInfo; 4] = [
    ParamInfo {
        key: "type",
        label: "Type",
        min: 0.0,
        max: 3.0,
        default: 0.0,
        unit: "",
        whole: true,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "slope",
        label: "Slope",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: "",
        whole: true,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "cutoff",
        label: "Cutoff",
        min: 0.0,
        max: 1.0,
        default: 0.5,
        unit: "Hz",
        whole: false,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "resonance",
        label: "Resonance",
        min: 0.0,
        max: 1.0,
        default: 0.1,
        unit: "%",
        whole: false,
        taper: Taper::Linear,
    },
];

const TYPE: usize = 0;
const SLOPE: usize = 1;
const CUTOFF: usize = 2;
const RESONANCE: usize = 3;

/// The slope control's choices by value: one stage, or two in series.
const SLOPES: [&str; 2] = ["12 dB", "24 dB"];

/// The types, in the order of the type control's values.
const TYPES: [(&str, Response); 4] = [
    ("Low-pass", Response::LowPass),
    ("High-pass", Response::HighPass),
    ("Band-pass", Response::BandPass),
    ("Notch", Response::Notch),
];

/// log2 1000, from log2 10 by a product alone, so every platform and the generator get its bits.
const LOG2_1000: f64 = 3.0 * LOG2_10;

/// The cutoff control's frequency: 20 Hz at 0, 20 kHz at 1, evenly in octaves.
fn cutoff_hz(cutoff: f32) -> f64 {
    20.0 * exp2(f64::from(cutoff) * LOG2_1000)
}

/// The resonance control's Q: 0.5 at 0, 20 at 1 (0.5·2^(r·log2 40)), evenly in octaves.
fn q(resonance: f32) -> f64 {
    0.5 * exp2(f64::from(resonance) * (2.0 + LOG2_10))
}

/// The type control's response; any value is clamped and rounded to one of the four first.
fn response(kind: f32) -> Response {
    TYPES[PARAMS[TYPE].clamp(kind) as usize].1
}

/// How many SVF stages the slope control's `value` runs: 1 (12 dB) or 2 (24 dB).
fn stages(value: f32) -> usize {
    1 + PARAMS[SLOPE].clamp(value) as usize
}

/// The level compensation of `response` at the control's `q`: for the low- and high-pass, whose
/// resonance rises above the input, min(1, √(Q0/Q)), Q0 the default resonance's Q, so exactly 1
/// at the default and below; 1 for the band-pass and the notch.
fn compensation(response: Response, q: f64) -> f64 {
    match response {
        Response::LowPass | Response::HighPass => {
            let q0 = self::q(PARAMS[RESONANCE].default);
            (q0 / q).sqrt().min(1.0)
        }
        Response::BandPass | Response::Notch => 1.0,
    }
}

pub(super) struct Factory;

impl ProcessorFactory for Factory {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Filter"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn shown(&self, index: usize, value: f32, _sample_rate: u32) -> f64 {
        match index {
            CUTOFF => cutoff_hz(value),
            _ => f64::from(value),
        }
    }

    fn value_name(&self, index: usize, value: f32) -> Option<&'static str> {
        match index {
            TYPE => Some(TYPES[PARAMS[TYPE].clamp(value) as usize].0),
            SLOPE => Some(SLOPES[stages(value) - 1]),
            _ => None,
        }
    }

    fn create(&self, sample_rate: u32, values: &[f32]) -> Box<dyn Processor> {
        let svf = || Svf::new(1000.0, 1.0, sample_rate);
        let mut filter = Filter {
            svfs: [svf(), svf()],
            stages: stages(PARAMS[SLOPE].default),
            response: response(PARAMS[TYPE].default),
            scale: 1.0,
            cutoff: PARAMS[CUTOFF].default,
            resonance: PARAMS[RESONANCE].default,
            sample_rate,
        };
        for (index, info) in PARAMS.iter().enumerate() {
            filter.set_param(index, values.get(index).copied().unwrap_or(info.default));
        }
        Box::new(filter)
    }
}

struct Filter {
    /// The stages in series; the first `stages` of them run.
    svfs: [Svf; 2],
    stages: usize,
    response: Response,
    /// The level compensation the output is scaled by ([`compensation`]).
    scale: f64,
    cutoff: f32,
    resonance: f32,
    sample_rate: u32,
}

impl Filter {
    /// The stages' cutoff and Q, and the scale, from the controls.
    fn tune(&mut self) {
        let q = q(self.resonance);
        let stage_q = if self.stages == 1 { q } else { q.sqrt() };
        for svf in &mut self.svfs {
            svf.set(cutoff_hz(self.cutoff), stage_q, self.sample_rate);
        }
        self.scale = compensation(self.response, q);
    }
}

impl Processor for Filter {
    fn process(&mut self, _context: &ProcessContext, buffer: &mut [[f32; 2]]) {
        let (response, scale) = (self.response, self.scale);
        let stages = &mut self.svfs[..self.stages];
        for frame in buffer {
            for (side, value) in frame.iter_mut().enumerate() {
                let mut y = f64::from(*value);
                for svf in stages.iter_mut() {
                    y = svf.process(side, y, response);
                }
                *value = (y * scale) as f32;
            }
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(info) = PARAMS.get(index) else {
            return;
        };
        let value = info.clamp(value);
        match index {
            TYPE => self.response = response(value),
            SLOPE => {
                let before = self.stages;
                self.stages = stages(value);
                // A second stage switched in starts clear, not with what it held when last on.
                if self.stages > before {
                    self.svfs[1].reset();
                }
            }
            CUTOFF => self.cutoff = value,
            _ => self.resonance = value,
        }
        self.tune();
    }

    fn reset(&mut self) {
        for svf in &mut self.svfs {
            svf.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::tests::{bits, noise, whole_and_split, CONTEXT};

    // The controls' ends and middles by hand: 20·1000^v Hz (632.46 Hz at 0.5) and 0.5·40^r
    // (0.5·√40 = 3.162 at 0.5, 0.5·40^0.1 = 0.7231 at the default).
    #[test]
    fn the_controls_sweep_in_octaves() {
        assert_eq!(cutoff_hz(0.0), 20.0);
        assert!((cutoff_hz(0.5) - 20.0 * 1000f64.sqrt()).abs() < 1e-9);
        assert!((cutoff_hz(1.0) - 20_000.0).abs() < 1e-9);
        assert_eq!(q(0.0), 0.5);
        assert!((q(0.5) - 0.5 * 40f64.sqrt()).abs() < 1e-12);
        assert!((q(1.0) - 20.0).abs() < 1e-12);
        // The default as the file holds it: 0.1 as an f32.
        assert!((q(0.1) - 0.5 * 40f64.powf(f64::from(0.1f32))).abs() < 1e-12);
    }

    #[test]
    fn the_type_is_named_and_picks_the_response() {
        let names: Vec<_> = (0..4).map(|v| Factory.value_name(TYPE, v as f32)).collect();
        assert_eq!(
            names,
            [
                Some("Low-pass"),
                Some("High-pass"),
                Some("Band-pass"),
                Some("Notch")
            ]
        );
        assert_eq!(Factory.value_name(TYPE, 2.4), Some("Band-pass"));
        assert_eq!(Factory.value_name(TYPE, 9.0), Some("Notch"));
        assert_eq!(Factory.value_name(CUTOFF, 0.5), None);
        assert_eq!(response(3.0), Response::Notch);
        assert_eq!(Factory.value_name(SLOPE, 0.0), Some("12 dB"));
        assert_eq!(Factory.value_name(SLOPE, 1.0), Some("24 dB"));
        assert_eq!([0.0, 1.0, 7.0].map(stages), [1, 2, 2]);
    }

    /// The default resonance's Q by hand: 0.5·40^r for r the f32 nearest 0.1 (0.7231).
    fn default_q() -> f64 {
        0.5 * 40f64.powf(f64::from(0.1f32))
    }

    // The compensation by hand: min(1, √(Q0/Q)) for the low- and high-pass, Q0 the default's Q:
    // exactly 1 at the default and at resonance 0 (Q 0.5), √(Q0/20) at 100% (Q 20, −14.4 dB);
    // 1 for the band-pass and the notch at any Q.
    #[test]
    fn resonance_is_compensated_above_the_default_on_the_low_and_high_pass_only() {
        assert_eq!(compensation(Response::LowPass, q(0.1)), 1.0);
        assert_eq!(compensation(Response::LowPass, q(0.0)), 1.0);
        let top = compensation(Response::HighPass, q(1.0));
        assert!((top - (default_q() / 20.0).sqrt()).abs() < 1e-12, "{top}");
        assert_eq!(compensation(Response::BandPass, 20.0), 1.0);
        assert_eq!(compensation(Response::Notch, 20.0), 1.0);
    }

    /// The amplitude of a 441 Hz sine (100 frames a period at 44.1 kHz) out of `filter`, after
    /// a second to settle, by projection over 10 whole periods.
    fn amplitude_441(filter: &mut dyn Processor) -> f64 {
        let sine = |n: usize| (std::f64::consts::TAU * (n % 100) as f64 / 100.0).sin() as f32;
        let mut frames: Vec<[f32; 2]> = (0..45_100).map(|n| [sine(n); 2]).collect();
        filter.process(&CONTEXT, &mut frames);
        let (mut on_sine, mut on_cosine) = (0.0, 0.0);
        for (n, frame) in frames.iter().enumerate().skip(44_100) {
            let angle = std::f64::consts::TAU * (n % 100) as f64 / 100.0;
            on_sine += f64::from(frame[0]) * angle.sin();
            on_cosine += f64::from(frame[0]) * angle.cos();
        }
        (on_sine * 2.0 / 1000.0).hypot(on_cosine * 2.0 / 1000.0)
    }

    // At its cutoff a low-pass rings Q times the input at 12 dB, and √Q·√Q = Q times at 24 dB:
    // the slopes meet there. Compensated: Q·√(Q0/Q) = √(Q0·Q), √(20·Q0) = 3.80 at resonance 100%.
    // The cutoff control for 441 Hz is log(441/20)/log(1000); a sine 1 ulp off it is close
    // enough to 1e-4.
    #[test]
    fn at_the_cutoff_both_slopes_ring_root_q0_q() {
        let cutoff = ((441.0f64 / 20.0).ln() / 1000f64.ln()) as f32;
        let want = (20.0 * default_q()).sqrt();
        for slope in [0.0, 1.0] {
            let got = amplitude_441(&mut *Factory.create(44_100, &[0.0, slope, cutoff, 1.0]));
            assert!((got - want).abs() < 1e-4, "slope {slope}: {got}");
        }
    }

    // Two octaves above the cutoff the 24 dB slope is about 24 dB further down than the 12 dB
    // one: 1/W² per stage against 1/W⁴ for the pair, W ≈ 4 here (prewarped).
    #[test]
    fn the_24_db_slope_falls_twice_as_fast() {
        let cutoff = ((110.25f64 / 20.0).ln() / 1000f64.ln()) as f32;
        let level =
            |slope: f32| amplitude_441(&mut *Factory.create(44_100, &[0.0, slope, cutoff, 0.0]));
        let fall = 20.0 * (level(1.0) / level(0.0)).log10();
        assert!((-25.0..-23.0).contains(&fall), "{fall} dB");
    }

    // DC through each type at the default cutoff and resonance (no compensation there): the
    // low-pass passes it whole, the high-pass and the band-pass stop it, the notch (far from its
    // cutoff) passes it whole; either slope.
    #[test]
    fn dc_passes_the_low_pass_and_the_notch_only() {
        for (kind, want) in [(0.0, 0.5), (1.0, 0.0), (2.0, 0.0), (3.0, 0.5)] {
            for slope in [0.0, 1.0] {
                let mut frames = vec![[0.5, 0.5]; 44_100];
                Factory
                    .create(44_100, &[kind, slope, 0.5, 0.1])
                    .process(&CONTEXT, &mut frames);
                let [l, r] = frames[44_099];
                assert!(
                    (f64::from(l) - want).abs() < 1e-6,
                    "type {kind}, slope {slope}: {l}"
                );
                assert_eq!(l.to_bits(), r.to_bits(), "type {kind}");
            }
        }
    }

    #[test]
    fn any_split_gives_the_same_bits() {
        for kind in 0..4 {
            let mut make = || Factory.create(44_100, &[kind as f32, (kind % 2) as f32, 0.3, 0.8]);
            let (whole, split) = whole_and_split(&mut make, &noise(3000), &[1, 7, 512, 1, 999]);
            assert_eq!(bits(&whole), bits(&split), "type {kind}");
        }
    }

    #[test]
    fn reset_clears_the_filters_memory() {
        let mut instance = Factory.create(44_100, &[0.0, 1.0, 0.2, 1.0]);
        let mut loud = noise(100);
        instance.process(&CONTEXT, &mut loud);
        instance.reset();
        let mut silence = vec![[0.0; 2]; 4];
        instance.process(&CONTEXT, &mut silence);
        assert_eq!(silence, vec![[0.0; 2]; 4]);
    }
}
