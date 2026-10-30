//! The gain laws: how a fader position, a channel pan and an insert pan become gains. Each one
//! is computed in f64 and rounded to f32 once, so the renderer, a live parameter change and the
//! mixer's readouts all get the same number.

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, SQRT_2};

/// The fader position of unity gain, as the f64 the law divides by.
///
/// Why not the f32 position widened: f64::from(0.8_f32) is 0.800000011920929, and dividing by
/// that moves some gains by one f32 step. The law is defined on the decimal 0.8.
const UNITY_POSITION: f64 = 0.8;

/// A fader position's linear gain, by the cube law (position / 0.8)³: unity at 0.8, 1.953125
/// (+5.81 dB) at the top, silence at 0.
///
/// Why a cube: it gives a usable taper with no table, and its ends land exactly (unity rounds to
/// exactly 1.0, the top is a short binary fraction), so an untouched fader changes no sample.
pub fn fader_gain(position: f32) -> f32 {
    let ratio = f64::from(position) / UNITY_POSITION;
    (ratio * ratio * ratio) as f32
}

/// A channel pan's gains for the left and right side of its mono sound: the circular
/// (constant-power) law scaled by √2, √2·sin((1 ∓ pan)·π/4).
///
/// Why scaled: centered gives exactly (1, 1), so a centered channel sounds as it did before pan
/// existed, and a hard pan is +3 dB on one side and silence on the other. Sines only, so the
/// ends are exactly 0 and √2.
pub fn channel_pan_gains(pan: f32) -> [f32; 2] {
    let pan = f64::from(pan);
    let side = |towards: f64| (SQRT_2 * ((1.0 + towards) * FRAC_PI_4).sin()) as f32;
    [side(-pan), side(pan)]
}

/// An insert pan's crossfeed gains `(fold, keep)` for a pan other than 0: the side panned away
/// from is scaled by `keep`, and `fold` of it is added to the side panned towards. At ±1 the
/// near side carries the mono sum and the far side is silent.
///
/// Why crossfeed rather than turning one side down: a stereo bus keeps everything on it, and
/// the moved part keeps its power (fold² + keep² = 1).
pub fn insert_pan_gains(pan: f32) -> (f32, f32) {
    let away = f64::from(pan).abs();
    let fold = (away * FRAC_PI_2).sin() as f32;
    let keep = ((1.0 - away) * FRAC_PI_2).sin() as f32;
    (fold, keep)
}

/// One stereo frame through an insert pan of `pan`, whose [`insert_pan_gains`] are `gains`.
pub fn insert_pan(pan: f32, (fold, keep): (f32, f32), [left, right]: [f32; 2]) -> [f32; 2] {
    if pan < 0.0 {
        [left + right * fold, right * keep]
    } else {
        [left * keep, right + left * fold]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::UNITY_FADER;

    // Expected values from tests/fixtures/gen_p5_expected.py: unity exactly 1.0, the top
    // 1.953125, position 0.63496 gives 0.4999989867210388; 0.4 gives 0.125.
    #[test]
    fn the_fader_is_a_cube_law_with_unity_at_0_8() {
        assert_eq!(fader_gain(UNITY_FADER), 1.0);
        assert_eq!(fader_gain(1.0), 1.953_125);
        assert_eq!(fader_gain(0.0), 0.0);
        assert_eq!(fader_gain(0.4), 0.125);
        assert_eq!(f64::from(fader_gain(0.634_96)), 0.499_998_986_721_038_8);
    }

    // Python: f32(sqrt(2)·sin(π/2)) = 1.4142135381698608, sin(0) = 0, center exactly (1, 1).
    #[test]
    fn channel_pan_is_unity_at_the_center_and_plus_3_db_at_the_sides() {
        assert_eq!(channel_pan_gains(0.0), [1.0, 1.0]);
        assert_eq!(channel_pan_gains(-1.0), [std::f32::consts::SQRT_2, 0.0]);
        assert_eq!(channel_pan_gains(1.0), [0.0, std::f32::consts::SQRT_2]);
        let [left, right] = channel_pan_gains(0.5);
        let power = f64::from(left).powi(2) + f64::from(right).powi(2);
        assert!((power - 2.0).abs() < 1e-6, "constant power, {power}");
    }

    #[test]
    fn insert_pan_folds_the_far_side_into_the_near_one() {
        assert_eq!(insert_pan_gains(-1.0), (1.0, 0.0));
        assert_eq!(
            insert_pan(-1.0, insert_pan_gains(-1.0), [0.25, 0.5]),
            [0.75, 0.0]
        );
        assert_eq!(
            insert_pan(1.0, insert_pan_gains(1.0), [0.25, 0.5]),
            [0.0, 0.75]
        );
        let (fold, keep) = insert_pan_gains(0.3);
        let power = f64::from(fold).powi(2) + f64::from(keep).powi(2);
        assert!(
            (power - 1.0).abs() < 1e-6,
            "the moved side keeps its power, {power}"
        );
    }
}
