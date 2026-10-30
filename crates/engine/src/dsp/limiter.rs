//! The limiter: a zero-latency peak limiter with one gain for both sides. Per frame the gain is
//! pulled down at once, so no frame leaves above the ceiling, then recovers towards 1 with an
//! exponential release. It looks at nothing ahead, so it delays nothing and every frame stays
//! where it was; a frame at the ceiling or under passes as it is while the gain is at rest.
//!
//! Why no lookahead: a lookahead limiter delays the whole mix by its window, which would move
//! every sample of every render and of the device's output; a peak caught on its own frame is
//! enough to keep a chord from clipping.

use super::{exp2, snap64};
use crate::processor::{ParamInfo, ProcessContext, Processor, ProcessorFactory};

pub const ID: &str = "dawsome.limiter";

const PARAMS: [ParamInfo; 2] = [
    ParamInfo {
        key: "ceiling_db",
        label: "Ceiling",
        min: -12.0,
        max: 0.0,
        default: -0.3,
        unit: "dB",
        whole: false,
    },
    ParamInfo {
        key: "release_ms",
        label: "Release",
        min: 1.0,
        max: 1000.0,
        default: 100.0,
        unit: "ms",
        whole: false,
    },
];

const CEILING_DB: usize = 0;
const RELEASE_MS: usize = 1;

/// The level no frame leaves above, from the ceiling in decibels: 10^(dB/20) by [`exp2`], as an
/// f32 (held in an f64), so a frame pulled down to it lands on it exactly and 0 dB is exactly 1.
fn ceiling(db: f32) -> f64 {
    f64::from(exp2(f64::from(db) / 20.0 * std::f64::consts::LOG2_10) as f32)
}

/// What the gain's reduction is multiplied by each frame: e^(−1/(T·rate)) by [`exp2`], so the
/// reduction falls to 1/e of itself in the release time T.
fn release(ms: f32, sample_rate: u32) -> f64 {
    let frames = f64::from(ms) / 1000.0 * f64::from(sample_rate.max(1));
    exp2(-std::f64::consts::LOG2_E / frames)
}

pub(super) struct Factory;

impl ProcessorFactory for Factory {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Limiter"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn create(&self, sample_rate: u32, values: &[f32]) -> Box<dyn Processor> {
        let mut limiter = Limiter {
            ceiling: ceiling(PARAMS[CEILING_DB].default),
            release: release(PARAMS[RELEASE_MS].default, sample_rate),
            reduction: 0.0,
            sample_rate,
        };
        for (index, &value) in values.iter().enumerate() {
            limiter.set_param(index, value);
        }
        Box::new(limiter)
    }
}

struct Limiter {
    ceiling: f64,
    /// The release's factor per frame.
    release: f64,
    /// How far the gain is below 1: the gain is 1 − reduction. Why the reduction is the state
    /// and not the gain: it is what decays, so the engine's snap of tiny state to zero applies
    /// to it and it never sinks into subnormal numbers.
    reduction: f64,
    sample_rate: u32,
}

impl Processor for Limiter {
    fn process(&mut self, _context: &ProcessContext, buffer: &mut [[f32; 2]]) {
        for frame in buffer {
            self.reduction = snap64(self.reduction * self.release);
            let mut gain = 1.0 - self.reduction;
            // An infinite side (a plugin gone wrong) is held to f32::MAX first: infinity times
            // the gain it earns (ceiling / infinity = 0) would be NaN, and the frame would go
            // out as NaN then near-silence for a release. Held, it leaves at the ceiling.
            let held = frame.map(|side| f64::from(side.clamp(-f32::MAX, f32::MAX)));
            let peak = held[0].abs().max(held[1].abs());
            if peak * gain > self.ceiling {
                gain = self.ceiling / peak;
                self.reduction = 1.0 - gain;
            }
            *frame = held.map(|side| (side * gain) as f32);
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(info) = PARAMS.get(index) else {
            return;
        };
        let value = info.clamp(value);
        match index {
            CEILING_DB => self.ceiling = ceiling(value),
            _ => self.release = release(value, self.sample_rate),
        }
    }

    fn reset(&mut self) {
        self.reduction = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::tests::{bits, noise, whole_and_split, CONTEXT};

    fn limiter(ceiling_db: f32, release_ms: f32) -> Box<dyn Processor> {
        Factory.create(44_100, &[ceiling_db, release_ms])
    }

    // By hand: 10^(−0.3/20) = 0.96605087 and 10^(−12/20) = 0.25118864 as f32; 0 dB is exactly 1.
    // e^(−1/4410) = 0.99977327 per frame for 100 ms at 44.1 kHz, e^(−1/44.1) for 1 ms.
    #[test]
    fn the_ceiling_and_the_release_are_the_rules() {
        assert_eq!(ceiling(0.0), 1.0);
        assert_eq!(ceiling(-0.3) as f32, 0.966_050_86);
        assert_eq!(ceiling(-12.0) as f32, 0.251_188_64);
        assert!((release(100.0, 44_100) - (-1.0f64 / 4410.0).exp()).abs() < 1e-15);
        assert!((release(1.0, 44_100) - (-1.0f64 / 44.1).exp()).abs() < 1e-15);
    }

    // A loud frame comes out on the ceiling, both sides scaled alike (the image holds); the gain
    // then recovers by the release: half the reduction is left after ln 2 time constants.
    #[test]
    fn a_peak_is_caught_on_its_frame_and_the_gain_recovers() {
        let mut instance = limiter(0.0, 100.0);
        let mut frames = vec![[2.0, -1.0]];
        frames.extend(std::iter::repeat_n([0.5, 0.25], 3057));
        instance.process(&CONTEXT, &mut frames);
        assert_eq!(frames[0], [1.0, -0.5]);
        // After n frames the gain is 1 − 0.5·r^n; r^3057 ≈ 1/2 (3057 ≈ 4410·ln 2).
        let gain = frames[3057][0] / 0.5;
        assert!((gain - 0.75).abs() < 1e-4, "{gain}");
        assert_eq!(frames[3057][1], frames[3057][0] / 2.0);
    }

    #[test]
    fn a_mix_under_the_ceiling_passes_bit_for_bit() {
        let input: Vec<[f32; 2]> = noise(2000)
            .into_iter()
            .map(|[l, r]| [l, -r])
            .chain([[0.966_050_86, -0.966_050_86], [-0.0, 0.0]])
            .collect();
        let mut output = input.clone();
        limiter(-0.3, 100.0).process(&CONTEXT, &mut output);
        assert_eq!(bits(&output), bits(&input));
    }

    // An infinite side (a plugin gone wrong) leaves at the ceiling, finite, and the gain
    // recovers as after any peak: infinity times its gain of zero would be NaN. f32::MAX is
    // held the same way.
    #[test]
    fn an_infinite_sample_leaves_at_the_ceiling() {
        let mut instance = limiter(0.0, 100.0);
        let mut frames = vec![
            [f32::INFINITY, 0.5],
            [f32::NEG_INFINITY, f32::MAX],
            [0.5, 0.5],
        ];
        instance.process(&CONTEXT, &mut frames);
        // The held side earns a gain of 1 / f32::MAX; the other side takes the same gain (the
        // image holds), so 0.5 leaves as a subnormal, not as 0.
        let tiny = (0.5 * (1.0 / f64::from(f32::MAX))) as f32;
        assert_eq!(frames[0], [1.0, tiny]);
        assert_eq!(frames[1], [-1.0, 1.0]);
        assert!(
            frames[2]
                .iter()
                .all(|v| v.is_finite() && *v >= 0.0 && *v <= 0.5),
            "{:?}",
            frames[2]
        );
    }

    #[test]
    fn any_split_gives_the_same_bits() {
        let loud: Vec<[f32; 2]> = noise(3000).iter().map(|f| f.map(|v| v * 4.0)).collect();
        let mut make = || limiter(-6.0, 5.0);
        let (whole, split) = whole_and_split(&mut make, &loud, &[1, 7, 512, 1, 999]);
        assert_eq!(bits(&whole), bits(&split));
    }

    #[test]
    fn reset_lets_the_gain_go() {
        let mut instance = limiter(-0.3, 1000.0);
        let mut loud = vec![[4.0, 4.0]; 10];
        instance.process(&CONTEXT, &mut loud);
        instance.reset();
        let mut quiet = vec![[0.5, 0.5]; 4];
        instance.process(&CONTEXT, &mut quiet);
        assert_eq!(quiet, vec![[0.5, 0.5]; 4]);
    }

    // The reduction decays to exactly zero (the snap), so a long rest is the identity again.
    #[test]
    fn a_long_rest_brings_the_gain_back_to_exactly_one() {
        let mut instance = limiter(-0.3, 1.0);
        let mut frames = vec![[8.0, 8.0]];
        frames.extend(std::iter::repeat_n([0.9, 0.9], 44_100));
        instance.process(&CONTEXT, &mut frames);
        assert_eq!(frames.last(), Some(&[0.9, 0.9]));
    }
}
