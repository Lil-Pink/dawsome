//! Effects ported from Airwindows (github.com/airwindows/airwindows, MIT License,
//! Copyright (c) 2018 Chris Johnson; the full notice is in licenses/THIRD_PARTY.md), and what they share:
//! the per-instance noise generator, the denormal guard and the 32-bit output dither.
//!
//! The ports compute in f64 in the originals' order of operations and convert at the edges (f32 in,
//! f64 inside, f32 out rounded to nearest), so they null against the originals' own renders bit for
//! bit; tests/fixtures/ref holds those renders and how they were made. The other native effects'
//! snap of tiny state to zero is not used here: it would break the null.

pub mod density;
pub mod pressure4;

/// The left and right generators' seeds: what the originals' constructor makes from the first
/// two `rand()` values after `srand(1)` (glibc), which is how the reference renders were made.
/// Why fixed: the dither is part of each output sample, so a null needs the same sequence.
pub(crate) const SEEDS: [u32; 2] = [2_490_677_913, 3_448_036_410];

/// How the originals scale their timing to the rate: 1 at 44.1 kHz, computed as they compute
/// it.
pub(crate) fn overall_scale(sample_rate: u32) -> f64 {
    let mut scale = 1.0;
    scale /= 44100.0;
    scale *= f64::from(sample_rate);
    scale
}

/// A linear gain in decibels: −∞ at 0.
pub(crate) fn decibels(gain: f32) -> f64 {
    20.0 * f64::from(gain).log10()
}

/// The originals' sine-stage limit: their eight-digit π/2. Why not `FRAC_PI_2`: the shaping
/// stages compare and multiply with this exact value, and a null needs the same bits.
#[allow(clippy::approx_constant)]
pub(crate) const HALF_PI: f64 = 1.57079633;

/// Input this close to zero is replaced by the generator's state times 1.18e-17 before it is
/// processed, so the effects' feedback never sinks into subnormal numbers. A silent input thus
/// comes out as noise of about 1e-6 at most, which the originals do by design.
const DENORMAL: f64 = 1.18e-23;
const DENORMAL_NOISE: f64 = 1.18e-17;

/// The generator: xorshift with shifts 13, 17 and 5.
pub(crate) fn next(state: u32) -> u32 {
    let mut state = state;
    state ^= state << 13;
    state ^= state >> 17;
    state ^= state << 5;
    state
}

/// `sample`, or the generator's noise in its place when it is too close to zero.
pub(crate) fn guard(sample: f64, state: u32) -> f64 {
    if sample.abs() < DENORMAL {
        f64::from(state) * DENORMAL_NOISE
    } else {
        sample
    }
}

/// The exponent `e` with `value = m · 2^e`, `m` in [0.5, 1): C's `frexpf`, which Rust's std
/// lacks. 0, infinities and NaN give 0, as glibc's does.
fn frexp_exponent(value: f32) -> i32 {
    let bits = value.to_bits() & 0x7fff_ffff;
    let field = (bits >> 23) as i32;
    match (bits, field) {
        (0, _) | (_, 0xff) => 0,
        // A subnormal: its exponent is that of its highest set bit.
        (_, 0) => -126 - (bits.leading_zeros() as i32 - 9),
        _ => field - 126,
    }
}

/// `2^power` exactly, for a power a normal f64 holds.
fn power_of_two(power: i32) -> f64 {
    f64::from_bits(((i64::from(power) + 1023) as u64) << 52)
}

/// The originals' 32-bit output dither: advances the generator, then adds noise scaled to the
/// size of `sample` as an f32 will hold it.
pub(crate) fn dither(sample: f64, state: &mut u32) -> f64 {
    let exponent = frexp_exponent(sample as f32);
    *state = next(*state);
    // e ranges from −148 to 128, so 2^(e + 62) is a normal f64.
    sample + (f64::from(*state) - 2_147_483_647.0) * 5.5e-36 * power_of_two(exponent + 62)
}

#[cfg(test)]
mod tests {
    use super::*;

    // frexpf by hand: 1.0 = 0.5 · 2^1, 0.75 = 0.75 · 2^0, 3.0 = 0.75 · 2^2, the smallest
    // normal 2^−126 = 0.5 · 2^−125, the smallest subnormal 2^−149 = 0.5 · 2^−148.
    #[test]
    fn the_exponent_is_frexps() {
        assert_eq!(frexp_exponent(1.0), 1);
        assert_eq!(frexp_exponent(0.75), 0);
        assert_eq!(frexp_exponent(-3.0), 2);
        assert_eq!(frexp_exponent(f32::MIN_POSITIVE), -125);
        assert_eq!(frexp_exponent(f32::from_bits(1)), -148);
        assert_eq!(frexp_exponent(f32::from_bits(0x0040_0000)), -126);
        assert_eq!(frexp_exponent(0.0), 0);
        assert_eq!(frexp_exponent(f32::INFINITY), 0);
        assert_eq!(power_of_two(-86), 2f64.powi(-86));
        assert_eq!(power_of_two(191), 2f64.powi(191));
    }

    // The generator's first step from the left seed, by hand in 32 bits.
    #[test]
    fn the_generator_is_xorshift_13_17_5() {
        let mut x: u64 = 2_490_677_913;
        x ^= (x << 13) & 0xffff_ffff;
        x ^= x >> 17;
        x ^= (x << 5) & 0xffff_ffff;
        assert_eq!(u64::from(next(SEEDS[0])), x);
    }
}
