//! Native sound generators and effects, and the gain laws the mixer applies. Each one renders by
//! absolute sample position or by state advanced one frame at a time, so rendering frames
//! `[a, b)` and then `[b, c)` gives exactly the output of rendering `[a, c)` in one go.

pub mod airwindows;
pub mod chorus;
pub mod delay;
pub mod distortion;
pub mod edits;
pub mod eq;
pub mod filter;
pub mod flanger;
pub mod gate;
pub mod instrument;
pub mod laws;
pub mod limiter;
pub mod parts;
pub mod phaser;
pub(crate) mod pitch;
pub mod reverb;
pub mod sampler;
pub(crate) mod shape;
pub mod synth;
pub mod width;

use crate::processor::Registry;

impl Registry {
    /// The effects this build has: the three-band EQ, the delay, the reverb, the limiter, the
    /// compressor and saturator ported from Airwindows, the filter, the distortion, the chorus,
    /// the flanger, the phaser, the gate and the stereo width; and its one instrument, Dawsome
    /// Synth, which a synth channel plays (its factory says so: `is_instrument`).
    pub fn native() -> Self {
        let mut registry = Self::empty();
        registry.register(eq::Factory);
        registry.register(delay::Factory);
        registry.register(reverb::Factory);
        registry.register(limiter::Factory);
        registry.register(airwindows::pressure4::Factory);
        registry.register(airwindows::density::Factory);
        registry.register(filter::Factory);
        registry.register(distortion::Factory);
        registry.register(chorus::Factory);
        registry.register(flanger::Factory);
        registry.register(phaser::Factory);
        registry.register(gate::Factory);
        registry.register(width::Factory);
        registry.register(synth::Factory);
        registry
    }
}

/// Recursive state this close to zero is set to zero.
///
/// Why: a decaying tail would otherwise sink into subnormal numbers, which many CPUs process
/// far more slowly; and a fixed threshold, unlike a CPU's flush-to-zero mode, gives the same
/// bits on every machine.
const TINY: f64 = 1e-30;

/// 2^x from + − × ÷ alone: 2^⌊x⌋ (exact) times 2^f = e^(f·ln 2) for the fraction f by 17 terms
/// of its Taylor series, which reach the last bit of an f64 there.
///
/// Why not `f64::exp2` or `powf`: those come from the platform's maths library, whose last bit
/// differs between platforms, and an instrument's gains, tuning and release curves must render
/// the same bits everywhere (and in the Python generators that state the known answers).
pub(crate) fn exp2(x: f64) -> f64 {
    let whole = x.floor();
    let fraction = x - whole;
    let y = fraction * std::f64::consts::LN_2;
    let (mut term, mut total) = (1.0, 1.0);
    for k in 1..=17 {
        term = term * y / f64::from(k);
        total += term;
    }
    // A power of two is exact, so this only moves the exponent.
    total * 2f64.powi(whole as i32)
}

/// 10^(dB/20), by [`exp2`]: (dB / 20)·log2 10, kept in f64.
pub(crate) fn lin(db: f64) -> f64 {
    exp2(db / 20.0 * std::f64::consts::LOG2_10)
}

fn snap(value: f32) -> f32 {
    if f64::from(value).abs() < TINY {
        0.0
    } else {
        value
    }
}

fn snap64(value: f64) -> f64 {
    if value.abs() < TINY {
        0.0
    } else {
        value
    }
}

/// `(1 − mix)·dry + mix·wet` on each side. At mix 0 it returns `dry` and at mix 1 `wet`, bit
/// for bit, so an effect at either end passes exactly the dry or the wet sound.
fn blend(dry: [f32; 2], wet: [f32; 2], mix: f32) -> [f32; 2] {
    if mix == 0.0 {
        return dry;
    }
    if mix == 1.0 {
        return wet;
    }
    let keep = 1.0 - mix;
    [keep * dry[0] + mix * wet[0], keep * dry[1] + mix * wet[1]]
}

/// √(1 − mix)·dry + √mix·wet on each side, in f64, rounded to f32 once: an equal-power mix (the
/// gains' squares sum to 1), so a wet sound unlike the dry one keeps the level as the mix
/// moves, where [`blend`] dips 3 dB at 50%. At mix 0 it returns `dry` and at mix 1 `wet`, bit
/// for bit. Why √ and not cos/sin of mix·π/2 (the same at 0, 50% and 100%): `sqrt` is correctly
/// rounded on every platform, `cos` and `sin` are not. Used by the chorus, the flanger and the
/// phaser; the effects that shipped in 0.1.0 keep [`blend`], so saved projects keep their sound.
fn blend_power(dry: [f32; 2], wet: [f32; 2], mix: f32) -> [f32; 2] {
    if mix == 0.0 {
        return dry;
    }
    if mix == 1.0 {
        return wet;
    }
    let mix = f64::from(mix);
    let (keep, add) = ((1.0 - mix).sqrt(), mix.sqrt());
    [0, 1].map(|side| (keep * f64::from(dry[side]) + add * f64::from(wet[side])) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processor::{ProcessContext, Processor, TransportInfo};
    use crate::time::{MilliBpm, Ticks};

    pub(crate) const CONTEXT: ProcessContext<'static> = ProcessContext {
        sample_rate: 44_100,
        tempo: MilliBpm(120_000),
        ppq: 96,
        bar: crate::time::BarPos {
            bar: 0,
            start: crate::time::Ticks(0),
            signature: crate::time::TimeSignature::COMMON,
        },
        transport: TransportInfo {
            playing: true,
            song_tick: Ticks(0),
            steady_time: 0,
        },
        events: &[],
    };

    /// A unit impulse on both sides, then silence, `frames` long.
    pub(crate) fn impulse(frames: usize) -> Vec<[f32; 2]> {
        let mut buffer = vec![[0.0; 2]; frames];
        buffer[0] = [1.0; 2];
        buffer
    }

    /// Seeded noise, different on each side, so a test sees both sides and every frame.
    pub(crate) fn noise(frames: usize) -> Vec<[f32; 2]> {
        let mut state = 0x9e37_79b9_u32;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            f64::from(state) / f64::from(u32::MAX) - 0.5
        };
        (0..frames)
            .map(|_| [next() as f32, next() as f32])
            .collect()
    }

    /// `input` processed in one call, and in calls of the sizes `cuts` gives (then the rest).
    pub(crate) fn whole_and_split(
        processor: &mut dyn FnMut() -> Box<dyn Processor>,
        input: &[[f32; 2]],
        cuts: &[usize],
    ) -> (Vec<[f32; 2]>, Vec<[f32; 2]>) {
        let mut whole = input.to_vec();
        processor().process(&CONTEXT, &mut whole);
        let mut split = input.to_vec();
        let mut instance = processor();
        let mut rest: &mut [[f32; 2]] = &mut split;
        for &cut in cuts {
            let (piece, later) = rest.split_at_mut(cut.min(rest.len()));
            instance.process(&CONTEXT, piece);
            rest = later;
        }
        instance.process(&CONTEXT, rest);
        (whole, split)
    }

    pub(crate) fn bits(frames: &[[f32; 2]]) -> Vec<[u32; 2]> {
        frames.iter().map(|frame| frame.map(f32::to_bits)).collect()
    }

    #[test]
    fn blend_is_exact_at_both_ends() {
        let (dry, wet) = ([-0.0, f32::MIN_POSITIVE], [0.3, -0.7]);
        assert_eq!(bits(&[blend(dry, wet, 0.0)]), bits(&[dry]));
        assert_eq!(bits(&[blend(dry, wet, 1.0)]), bits(&[wet]));
        assert_eq!(blend([1.0, 0.0], [0.0, 1.0], 0.25), [0.75, 0.25]);
    }

    // Equal power: at 50% both gains are √0.5 (correctly rounded), so 0.5 dry and 0.5 wet give
    // f32(2·√0.5·0.5) = f32(√0.5); a 1 on one side only comes out √0.5 or √(1 − 0.36) = 0.8.
    #[test]
    fn blend_power_is_exact_at_both_ends_and_equal_power_between() {
        let (dry, wet) = ([-0.0, f32::MIN_POSITIVE], [0.3, -0.7]);
        assert_eq!(bits(&[blend_power(dry, wet, 0.0)]), bits(&[dry]));
        assert_eq!(bits(&[blend_power(dry, wet, 1.0)]), bits(&[wet]));
        let half = 0.5f64.sqrt();
        assert_eq!(
            blend_power([0.5, 1.0], [0.5, 0.0], 0.5),
            [(2.0 * half * 0.5) as f32, half as f32]
        );
        assert_eq!(blend_power([1.0, 0.0], [0.0, 1.0], 0.36), [0.8, 0.6]);
    }

    // Exact where the answer is a power of two; within an f64 step of the true value
    // elsewhere (2^0.5 = √2, 2^(−1/12), 10^−1 as 2^(−log2 10)).
    #[test]
    fn exp2_is_exact_on_powers_of_two_and_close_elsewhere() {
        assert_eq!(exp2(0.0), 1.0);
        assert_eq!(exp2(3.0), 8.0);
        assert_eq!(exp2(-15.0), 1.0 / 32768.0);
        let close = |got: f64, want: f64| (got - want).abs() <= want * 4.0 * f64::EPSILON;
        assert!(close(exp2(0.5), std::f64::consts::SQRT_2));
        assert!(close(exp2(-1.0 / 12.0), 0.943_874_312_681_693_5));
        assert!(close(exp2(-std::f64::consts::LOG2_10), 0.1));
    }

    #[test]
    fn tiny_values_snap_to_zero() {
        assert_eq!(snap(1e-31), 0.0);
        assert_eq!(snap(-1e-31), 0.0);
        assert_eq!(snap(2e-30), 2e-30);
        assert_eq!(snap64(-5e-31), 0.0);
        assert_eq!(snap64(1e-20), 1e-20);
    }

    #[test]
    fn the_native_registry_holds_every_native_effect() {
        let registry = Registry::native();
        let ids: Vec<&str> = registry.factories().map(|f| f.id()).collect();
        assert_eq!(
            ids,
            [
                "dawsome.chorus",
                "dawsome.delay",
                "dawsome.density",
                "dawsome.distortion",
                "dawsome.eq3",
                "dawsome.filter",
                "dawsome.flanger",
                "dawsome.gate",
                "dawsome.limiter",
                "dawsome.phaser",
                "dawsome.pressure4",
                "dawsome.reverb",
                "dawsome.synth",
                "dawsome.width"
            ]
        );
        assert_eq!(registry.get("dawsome.eq3").map(|f| f.name()), Some("EQ"));
        assert!(registry.get("dawsome.missing").is_none());
    }

    // Hand-written: the sliders laid out in octaves are
    // the EQ's three frequencies and three Qs (decades in hertz and Q), the delay's time, the
    // limiter's release and the gate's attack, hold and release (milliseconds over two to three
    // decades). Every other parameter is linear: a 0..1 control already mapped in octaves (the
    // filter's cutoff, the phaser's center), a decibel value, a share, a choice. The compressor
    // has none: its Speed is the original's 0..1 value, shown in milliseconds.
    #[test]
    fn the_log_tapered_parameters_are_the_wide_ranges() {
        use crate::processor::Taper;
        let registry = Registry::native();
        let mut log = Vec::new();
        for factory in registry.factories() {
            for info in factory.params() {
                if info.taper == Taper::Log {
                    log.push((factory.id(), info.key));
                }
            }
        }
        assert_eq!(
            log,
            [
                ("dawsome.delay", "time_ms"),
                ("dawsome.eq3", "low_freq"),
                ("dawsome.eq3", "low_q"),
                ("dawsome.eq3", "mid_freq"),
                ("dawsome.eq3", "mid_q"),
                ("dawsome.eq3", "high_freq"),
                ("dawsome.eq3", "high_q"),
                ("dawsome.gate", "attack_ms"),
                ("dawsome.gate", "hold_ms"),
                ("dawsome.gate", "release_ms"),
                ("dawsome.limiter", "release_ms"),
            ]
        );
    }

    // A project saved under the former prefix finds its effects only if every native id has
    // the prefix the loader moves them to.
    #[test]
    fn every_native_id_starts_with_the_native_prefix() {
        let registry = Registry::native();
        for factory in registry.factories() {
            assert!(
                factory.id().starts_with(crate::project::NATIVE_ID_PREFIX),
                "{}",
                factory.id()
            );
        }
    }
}
