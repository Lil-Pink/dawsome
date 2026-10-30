//! Where a live voice reads its sound, frame by frame, as a device's pitch bend and mod wheel
//! move it.
//!
//! A voice reads at P(t) = base + (t − from)·rate + depth·sin(ω·t), t its frames since it began:
//! `rate` is its own speed times the bend's ratio; `depth` is the mod wheel's vibrato, its swing
//! in frames of the sound (0 without, and the sine is then left out), ω its 5 Hz in radians a
//! frame. It begins at base 0, from 0, so a voice the bend and the wheel never moved reads at
//! P(t) = t·speed, the bits it always read. When either moves at frame T, the voice re-bases
//! there (from = T, base = P(T) as it was less the new vibrato's sine at T), so it reads on from
//! where it was, at the new rate and swing, with no jump. The vibrato's phase runs from the
//! voice's start.
//!
//! Why a closed form rather than a position stepped frame by frame: the read point then depends
//! only on the frame and on the frames where the pitch moved, which the player takes at the first
//! frame of the call that brings them, never on where a render is cut into blocks; and no rounding
//! builds up over a long note. Only the latest segment is kept, so P is asked of frames from the
//! last move on; the one earlier frame a voice needs, the frame its note ended (a sustain loop
//! reads on from there), is kept when the note ends ([`Pitch::release`]).
//!
//! The bend's ratio: b from −8192 to 8191 (the wheel all the way down to all the way up) reaches
//! R semitones (the channel's bend range), st = R·b/8191 above the center and R·b/8192 below, so
//! both ends reach exactly ±R; ratio = exp2(st/12) by [`exp2`] (+ − × ÷ only, the bits a Python
//! generator states), exactly 1 at the center. The vibrato: the mod wheel at w (0 to 127) swings
//! c = 50·w/127 cents at 5 Hz: depth = rate·(exp2(c/1200) − 1)/ω, so the rate peaks c cents up
//! (and falls as far below in rate, a little over c cents).

use std::f64::consts::TAU;

use super::exp2;

/// The bend's value all the way up, and the depth of its scale below the center.
const BEND_UP: f64 = 8191.0;
const BEND_DOWN: f64 = 8192.0;

/// The vibrato's rate, and its swing with the mod wheel all the way up.
const VIBRATO_HZ: f64 = 5.0;
const VIBRATO_CENTS: f64 = 50.0;

/// The speed ratio pitch bend `bend` (−8192 to 8191) gives at a range of `range` semitones:
/// exactly 1 at the center.
pub(crate) fn bend_ratio(bend: i16, range: u8) -> f64 {
    if bend == 0 {
        return 1.0;
    }
    let scale = if bend > 0 { BEND_UP } else { BEND_DOWN };
    let semitones = f64::from(range) * f64::from(bend) / scale;
    exp2(semitones / 12.0)
}

/// The mod wheel's vibrato as voices play it: how far above its own rate a voice's rate peaks
/// (exp2(c/1200) − 1 for c cents), and its 5 Hz in radians a frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Vibrato {
    swing: f64,
    omega: f64,
}

impl Vibrato {
    /// No vibrato: the wheel at rest.
    pub(crate) const NONE: Self = Self {
        swing: 0.0,
        omega: 0.0,
    };

    /// The vibrato of the mod wheel at `wheel` (0 to 127) at `rate` frames a second.
    pub(crate) fn of_wheel(wheel: u8, rate: u32) -> Self {
        if wheel == 0 {
            return Self::NONE;
        }
        let cents = VIBRATO_CENTS * f64::from(wheel) / 127.0;
        Self {
            swing: exp2(cents / 1200.0) - 1.0,
            omega: TAU * VIBRATO_HZ / f64::from(rate),
        }
    }

    /// Its swing in frames of the sound for a voice reading at `rate`.
    fn depth(self, rate: f64) -> f64 {
        if self.swing == 0.0 {
            0.0
        } else {
            rate * self.swing / self.omega
        }
    }
}

/// Where a voice reads its sound: see the module's documentation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Pitch {
    /// The voice's own speed, unbent.
    speed: f64,
    /// Its speed now: `speed` times the bend's ratio.
    rate: f64,
    /// Where it read at frame `from`, less the vibrato's sine there.
    base: f64,
    /// The frame (since it began) its pitch last moved on.
    from: u64,
    vibrato: Vibrato,
    /// The vibrato's swing in frames of the sound, at `rate`; 0 without.
    depth: f64,
    /// The frame its note ended on and where it read there, once it has ended.
    released: Option<(u64, f64)>,
}

impl Pitch {
    /// A voice that reads at `speed` from its first frame: P(t) = t·speed.
    pub(crate) fn steady(speed: f64) -> Self {
        Self::new(speed, 1.0, Vibrato::NONE)
    }

    /// A voice of `speed` that begins bent by `ratio`, with `vibrato`.
    pub(crate) fn new(speed: f64, ratio: f64, vibrato: Vibrato) -> Self {
        let rate = bent(speed, ratio);
        Self {
            speed,
            rate,
            base: 0.0,
            from: 0,
            vibrato,
            depth: vibrato.depth(rate),
            released: None,
        }
    }

    /// Where it reads `t` frames after it began: from frame 0 on, only ever forwards. Why the
    /// vibrato never reads backwards: its swing moves the read point at most `swing·rate` a frame
    /// (depth·ω), under 3 % of the rate at the wheel's top, so P only grows.
    pub(crate) fn at(&self, t: u64) -> f64 {
        let linear = self.base + t.saturating_sub(self.from) as f64 * self.rate;
        if self.depth == 0.0 {
            linear
        } else {
            linear + self.depth * (self.vibrato.omega * t as f64).sin()
        }
    }

    /// Where it read on frame `t`, the frame its note ended on (kept by
    /// [`release`](Self::release)) or a frame from its last move on.
    pub(crate) fn at_end(&self, t: u64) -> f64 {
        match self.released {
            Some((at, read)) if at == t => read,
            _ => self.at(t),
        }
    }

    /// How far it reads from frame `from` to frame `to`: `from` being the frame its note ended
    /// on (kept by [`release`](Self::release)) or a frame from its last move on. For a voice whose
    /// pitch never moved, (to − from)·speed: the bits a loop's release always read on by.
    pub(crate) fn between(&self, from: u64, to: u64) -> f64 {
        if self.never_moved() {
            return to.saturating_sub(from) as f64 * self.rate;
        }
        self.at(to) - self.at_end(from)
    }

    /// Its note ended on frame `t`: where it read there is kept, for [`between`](Self::between)
    /// to measure from after later moves.
    pub(crate) fn release(&mut self, t: u64) {
        self.released = Some((t, self.at(t)));
    }

    /// Whether it reads its sound's frames one by one, as recorded: unmoved, at speed 1.
    pub(crate) fn whole_frames(&self) -> bool {
        self.never_moved() && self.rate == 1.0
    }

    fn never_moved(&self) -> bool {
        self.from == 0 && self.base == 0.0 && self.depth == 0.0
    }

    /// The bend moved to `ratio` at frame `t`: from there it reads on from where it was at its
    /// speed times `ratio`, its vibrato swinging as many cents.
    pub(crate) fn bend(&mut self, t: u64, ratio: f64) {
        self.moved(t, bent(self.speed, ratio), self.vibrato);
    }

    /// The mod wheel moved the vibrato to `vibrato` at frame `t`: from there it reads on from
    /// where it was at the rate it has, swinging so.
    pub(crate) fn vibrate(&mut self, t: u64, vibrato: Vibrato) {
        self.moved(t, self.rate, vibrato);
    }

    fn moved(&mut self, t: u64, rate: f64, vibrato: Vibrato) {
        let here = self.at(t);
        let depth = vibrato.depth(rate);
        let base = if depth == 0.0 {
            here
        } else {
            here - depth * (vibrato.omega * t as f64).sin()
        };
        *self = Self {
            rate,
            base,
            from: t,
            vibrato,
            depth,
            ..*self
        };
    }
}

/// `speed` bent by `ratio`: `speed` itself, to the bit, at the center.
fn bent(speed: f64, ratio: f64) -> f64 {
    if ratio == 1.0 {
        speed
    } else {
        speed * ratio
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // By hand: the ends of the wheel reach the range exactly (2 semitones: the sampler's table's
    // 2^(2/12) and its inverse, to the last bit as exp2 gives them); the center is exactly 1; a
    // range of 12 doubles the speed.
    #[test]
    fn the_wheels_ends_reach_the_range() {
        assert_eq!(bend_ratio(0, 2), 1.0);
        assert_eq!(bend_ratio(8191, 2), exp2(2.0 / 12.0));
        assert_eq!(bend_ratio(-8192, 2), exp2(-2.0 / 12.0));
        assert_eq!(bend_ratio(8191, 12), 2.0);
        assert_eq!(bend_ratio(-8192, 12), 0.5);
        assert_eq!(bend_ratio(4096, 2), exp2(2.0 * 4096.0 / 8191.0 / 12.0));
    }

    // An unmoved voice reads t·speed bit for bit; bent at frame 10 it reads on from position 10
    // at the new rate, and back at the center at its own speed again from where it got to.
    #[test]
    fn a_bend_reads_on_from_where_the_voice_was() {
        let speed = 1.059_463_094_359_295_3;
        let mut pitch = Pitch::steady(speed);
        for t in [0, 1, 17, 44_100] {
            assert_eq!(pitch.at(t).to_bits(), (t as f64 * speed).to_bits());
        }
        assert_eq!(
            pitch.between(3, 9).to_bits(),
            (6.0 * speed).to_bits(),
            "the loop's release rule"
        );
        let mut plain = Pitch::steady(1.0);
        assert!(plain.whole_frames());
        plain.bend(10, 2.0);
        assert!(!plain.whole_frames());
        assert_eq!(plain.at(10), 10.0);
        assert_eq!(plain.at(13), 16.0);
        plain.release(15);
        plain.bend(20, 1.0);
        assert_eq!(plain.at(20), 30.0);
        assert_eq!(plain.at(25), 35.0);
        assert_eq!(
            plain.between(15, 25),
            15.0,
            "from its note's end (read 20), past the move"
        );
        assert_eq!(plain.between(20, 25), 5.0);
        assert_eq!(plain.at_end(15), 20.0, "where its note ended");
        assert_eq!(plain.at_end(22), 32.0);
        pitch.bend(5, 2.0);
        assert_eq!(pitch.at(5).to_bits(), (5.0 * speed).to_bits());
    }

    // By hand: the wheel at rest is no vibrato; all the way up it swings 50 cents at 5 Hz, its
    // depth rate·(2^(50/1200) − 1)/(2π·5/44100) frames. Moved on at frame 100 it reads the same
    // point there (no jump), then swings from it; a bend after keeps the swing in cents (its
    // depth follows the rate).
    #[test]
    fn the_wheels_vibrato_swings_from_where_the_voice_is() {
        assert_eq!(Vibrato::of_wheel(0, 44_100), Vibrato::NONE);
        let full = Vibrato::of_wheel(127, 44_100);
        let omega = TAU * 5.0 / 44_100.0;
        assert_eq!(full.omega, omega);
        assert_eq!(full.swing, exp2(50.0 / 1200.0) - 1.0);
        let mut pitch = Pitch::steady(1.0);
        pitch.vibrate(100, full);
        assert_eq!(pitch.at(100), 100.0);
        assert!(!pitch.whole_frames());
        let depth = full.swing / omega;
        let want = 100.0 - depth * (omega * 100.0).sin() + 10.0 + depth * (omega * 110.0).sin();
        assert_eq!(pitch.at(110), want);
        let before = pitch.at(200);
        pitch.bend(200, 2.0);
        assert_eq!(pitch.at(200), before, "no jump at the bend");
        assert_eq!(pitch.depth, 2.0 * full.swing / omega);
        let mut rest = pitch;
        rest.vibrate(300, Vibrato::NONE);
        assert_eq!(rest.at(310) - rest.at(300), 20.0, "back to a straight read");
    }

    // The wheel moved on a note's third frame swings about 40 frames each way (50 cents at 5 Hz
    // and 44.1 kHz: (2^(50/1200) − 1)/(2π·5/44100) frames), yet the read point never goes back:
    // a swing moves it under 3 % of a frame a frame, the rate a whole frame.
    #[test]
    fn a_swing_only_ever_reads_forwards() {
        let mut pitch = Pitch::steady(1.0);
        pitch.vibrate(2, Vibrato::of_wheel(127, 44_100));
        assert_eq!(pitch.at(2), 2.0);
        for t in 2..20_000 {
            assert!(pitch.at(t + 1) > pitch.at(t), "frame {t}");
        }
    }
}
