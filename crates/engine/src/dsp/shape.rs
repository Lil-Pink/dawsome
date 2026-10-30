//! A channel's sound settings ([`Sound`]) as a voice plays them ([`Shape`]): the tuning as a
//! speed ratio, the velocity's weight, the envelope in frames, the gate, a sampler's direction
//! and start, and the low-pass filter's coefficients. Every value a voice gives is a function of
//! the frames elapsed since its note began and the frame its note ended, except the filter,
//! whose memory each voice carries and advances one frame at a time; so any split of a render
//! into blocks gives the same bits.
//!
//! The rules, for the known answers (tests/fixtures/gen_p13_expected.py restates them):
//! - tune: speed × 2^(semitones/12) from the sampler's semitone table, times exp2(cents/1200)
//!   when cents ≠ 0 ([`exp2`]: + − × ÷ only, the same bits everywhere).
//! - velocity: the level 1 − w(1 − v) in f32, for weight w ≠ 1 (w = 1: v itself); a note at
//!   velocity 0 plays nothing, on a sampler as on a set, whatever the weight.
//! - envelope: attack A and release R frames (round half away from zero of seconds × rate; 0
//!   keeps the sound's own: none for a sampler); level t/A for t < A, else 1; after the note's
//!   end at frame `off` (heard when the gate is Hold, or a one-shot with R > 0): level(off) ×
//!   exp2(−15 r/R) for r = t − off < R, then the voice ends (R = 0: at `off`).
//! - a set's zones: one that loops always hears its note's end; any other hears it when the
//!   channel holds or sets a release, not when the channel is one-shot without a release; with
//!   no gate set, the zone's own loop mode decides (a one-shot zone plays on, any other is
//!   held). The channel's gate wins over a zone's `loop_mode=one_shot`.
//! - sampler reading: start frame S = floor(start × frames); reversed, frame i reads the
//!   sound's frame (frames − 1 − i); position p = S + P(t), as the sampler reads (P(t) = t ×
//!   speed for a voice no bend moved, [`Pitch`]), between frames by the sampler's 4-point
//!   [`interpolate`], the points before S and past the last frame counting 0 (the frames the
//!   voice plays are S to the last, in the order it reads them).
//! - a cut (a later note on a channel that cuts itself, at frame c of the voice): when the
//!   note's end it hears comes at c or later (or never), c is its end: level(c) × exp2(−15 r/R)
//!   as above, or with R = 0 a straight fall level(c) × (D − r)/D over D frames
//!   ([`CUT_FADE_SECONDS`]), then the voice ends; an end it heard before c stands ([`ended`]).
//! - filter: the cookbook's low-pass biquad (cutoff kept below 0.45 × rate, Q = 1/√2 +
//!   resonance × (10 − 1/√2)), Direct Form I in f64 per side on the voice's value before its
//!   gain; a voice ends with its sound, its filter's tail with it. A device's mod wheel at w opens
//!   a live voice's filter to min(cutoff × exp2(4w/127), 20000) Hz from the frame it moves on
//!   ([`wheel_tone`]), the filter's memory going on.

use std::f64::consts::{FRAC_1_SQRT_2, PI};

use super::exp2;
use super::pitch::Pitch;
use super::sampler::{interpolate, semitone_speed, SampleBuffer};
use crate::project::{ChannelKind, Gate, Sound, CUTOFF_RANGE};

/// The halvings a release falls through over its length: 15, about 90 dB, after which the
/// voice ends. Why linear in decibels: it is how sounds die away; and why a floor: a level
/// falling in decibels never reaches silence. An instrument zone's release and a channel's
/// both fall so ([`envelope`]).
const RELEASE_HALVINGS: u64 = 15;

/// How long a cut voice whose release is 0 falls for: a straight fall from where it is to
/// silence. Why not end it on the cut's frame, as a note's end with no release does: that is a
/// step of up to the wave's height, which clicks (reproduced on 800af0b: a held sine ended from
/// 0.295 to 0 in one frame, twelve times the wave's own largest step); 2 ms is short enough to
/// read as the next note's attack.
pub(crate) const CUT_FADE_SECONDS: f64 = 0.002;

/// The highest cutoff the filter takes, as a fraction of the sample rate.
const MAX_CUTOFF_FRACTION: f64 = 0.45;

/// How many octaves the mod wheel all the way up opens the filter by.
const WHEEL_OCTAVES: f64 = 4.0;

/// The filter's Q at full resonance.
const MAX_Q: f64 = 10.0;

/// Recursive filter state this close to zero is set to zero (see the effects' own rule).
const TINY: f64 = 1e-30;

/// A channel's sound settings as its voices play them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Shape {
    /// The speed ratio the tuning gives, exactly 1 untuned.
    pub(crate) tune: f64,
    /// The velocity's weight, 1 by default.
    pub(crate) velocity: f32,
    /// In frames; 0 keeps the sound's own.
    pub(crate) attack: u64,
    pub(crate) release: u64,
    pub(crate) gate: Gate,
    /// The gate the channel sets itself, none when it leaves it to the sound: a set's zones
    /// then go by their own loop modes.
    pub(crate) chosen_gate: Option<Gate>,
    pub(crate) reverse: bool,
    pub(crate) start: f32,
    /// The low-pass filter's normalized coefficients `[b0, b1, b2, a1, a2]`, while it is on.
    pub(crate) filter: Option<[f64; 5]>,
}

/// A voice's filter memory: `[x1, x2, y1, y2]` per side.
pub(crate) type FilterMemory = [[f64; 4]; 2];

impl Shape {
    /// How a channel of `kind` with `sound` plays at `rate`; `None` at the defaults (the bend
    /// range aside: only a live note's bend reads it), which play as before sound settings by
    /// the voices' own paths.
    pub(crate) fn compile(sound: &Sound, kind: ChannelKind, rate: u32) -> Option<Self> {
        if sound.shapes_nothing() {
            return None;
        }
        let tune = semitone_speed(i32::from(sound.semitones))
            * if sound.cents == 0 {
                1.0
            } else {
                exp2(f64::from(sound.cents) / 1200.0)
            };
        Some(Self {
            tune,
            velocity: sound.velocity,
            attack: frames_of(sound.attack, rate),
            release: frames_of(sound.release, rate),
            gate: sound.gate_for(kind),
            chosen_gate: sound.gate,
            reverse: sound.reverse,
            start: sound.start,
            filter: sound
                .filter
                .then(|| low_pass(f64::from(sound.cutoff), f64::from(sound.resonance), rate)),
        })
    }

    /// The level a note at `velocity` plays at.
    pub(crate) fn velocity_level(&self, velocity: f32) -> f32 {
        if self.velocity == 1.0 {
            velocity
        } else {
            1.0 - self.velocity * (1.0 - velocity)
        }
    }

    /// An instrument zone's velocity gain `gain` (its own curve's) weighted as
    /// [`velocity_level`](Self::velocity_level) weights a velocity.
    pub(crate) fn weighted(&self, gain: f64) -> f64 {
        if self.velocity == 1.0 {
            gain
        } else {
            1.0 - f64::from(self.velocity) * (1.0 - gain)
        }
    }

    /// Whether a note's end is heard by a sound that would otherwise play on to its end.
    pub(crate) fn hears_off(&self) -> bool {
        self.gate == Gate::Hold || self.release > 0
    }

    /// Whether a set's zone that does not loop hears its note's end, the zone a one-shot
    /// (`one_shot`) or not: when the channel holds or sets a release; not when it is one-shot
    /// without one; with no gate set, unless the zone is a one-shot.
    pub(crate) fn zone_hears_off(&self, one_shot: bool) -> bool {
        match self.chosen_gate {
            _ if self.release > 0 => true,
            Some(gate) => gate == Gate::Hold,
            None => !one_shot,
        }
    }

    /// A sampler voice's envelope at frame `t`, its note ended at `off`, cut at `cut`: `None`
    /// once released.
    pub(crate) fn level(&self, t: u64, off: Option<u64>, cut: Option<Cut>) -> Option<f64> {
        ended(
            self.attack,
            self.release,
            t,
            off.filter(|_| self.hears_off()),
            cut,
        )
    }

    /// A sampler voice's value and envelope `t` frames after its note began, reading `sample` as
    /// `pitch` says, its note ended at `off`, cut at `cut`; `None` once it has ended (read past
    /// its sound, or released).
    pub(crate) fn sampler_frame(
        &self,
        sample: &SampleBuffer,
        t: u64,
        off: Option<u64>,
        cut: Option<Cut>,
        pitch: &Pitch,
    ) -> Option<([f32; 2], f32)> {
        let level = self.level(t, off, cut)?;
        let frames = sample.frames();
        let start = (f64::from(self.start) * frames as f64).floor() as u64;
        let position = start as f64 + pitch.at(t);
        // Positions are never negative, so the conversion to an integer is the floor.
        let frame = position as u64;
        if frame >= frames {
            return None;
        }
        let read = |index: u64| {
            let index = if self.reverse {
                frames - 1 - index
            } else {
                index
            };
            sample.frame(index)
        };
        let current = read(frame)?;
        let fraction = position - frame as f64;
        let value = if fraction == 0.0 {
            current
        } else {
            // The voice plays frames `start` to the last; a point outside them counts 0.
            let point = |index: Option<u64>| {
                index
                    .filter(|index| (start..frames).contains(index))
                    .and_then(read)
                    .unwrap_or([0.0; 2])
            };
            let points = [
                point(frame.checked_sub(1)),
                current,
                point(Some(frame + 1)),
                point(Some(frame + 2)),
            ];
            interpolate(points, fraction)
        };
        Some((value, level as f32))
    }

    /// `value` through the filter, advancing `memory`, when it is on (with `tone`'s
    /// coefficients in place of its own while the mod wheel opens it); else `value`.
    pub(crate) fn filtered(
        &self,
        memory: &mut FilterMemory,
        value: [f32; 2],
        tone: Option<[f64; 5]>,
    ) -> [f32; 2] {
        let Some(own) = self.filter else {
            return value;
        };
        let [b0, b1, b2, a1, a2] = tone.unwrap_or(own);
        let mut out = [0.0; 2];
        for (side, (state, x)) in memory.iter_mut().zip(value).enumerate() {
            let [x1, x2, y1, y2] = *state;
            let x = f64::from(x);
            let y = b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
            let y = if y.abs() < TINY { 0.0 } else { y };
            *state = [x, x1, y, y1];
            out[side] = y as f32;
        }
        out
    }
}

/// The envelope of `attack` and `release` frames at frame `t`, the note's end heard at `off`:
/// the attack's straight rise, then from `off` the release falling linearly in decibels from
/// where the note left it; `None` once released.
pub(crate) fn envelope(attack: u64, release: u64, t: u64, off: Option<u64>) -> Option<f64> {
    let Some(off) = off.filter(|&off| t >= off) else {
        return Some(rise(attack, t));
    };
    let released = t - off;
    if released >= release {
        return None;
    }
    let halvings = (RELEASE_HALVINGS * released) as f64 / release as f64;
    Some(rise(attack, off) * exp2(-halvings))
}

/// The attack's straight rise over `attack` frames, at frame `t`.
fn rise(attack: u64, t: u64) -> f64 {
    if attack == 0 || t >= attack {
        1.0
    } else {
        t as f64 / attack as f64
    }
}

/// Where a later note on a channel that cuts itself cut a voice: `at` frames after the voice
/// began, and the frames it falls over when its release is 0 ([`CUT_FADE_SECONDS`] at the
/// project's rate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cut {
    pub(crate) at: u64,
    pub(crate) fade: u64,
}

impl Cut {
    /// The cut `at` frames into a voice, at `rate`.
    pub(crate) fn new(at: u64, rate: u32) -> Self {
        let fade = (CUT_FADE_SECONDS * f64::from(rate)).round() as u64;
        Self {
            at,
            fade: fade.max(1),
        }
    }

    /// Whether it ends a voice whose heard note's end is `off`: when that end comes on the cut's
    /// frame or after it, or never. An end before the cut stands, and the cut changes nothing.
    pub(crate) fn ends(cut: Option<Cut>, off: Option<u64>) -> Option<Cut> {
        cut.filter(|cut| off.is_none_or(|off| cut.at <= off))
    }

    /// Where a voice whose heard note's end is `off`, cut at `cut`, ends its note: the cut's frame
    /// when the cut ends it, else `off`. A sustain loop reads on from there.
    pub(crate) fn end(cut: Option<Cut>, off: Option<u64>) -> Option<u64> {
        Cut::ends(cut, off).map_or(off, |cut| Some(cut.at))
    }
}

/// [`envelope`] for a voice cut at `cut` (none: never): from a cut that ends it ([`Cut::ends`])
/// the release falls from the cut's frame, or with no release a straight fall over the cut's
/// fade; `None` once it has ended. Every value a function of `t`, the note's end and the cut's
/// frame, so a render split anywhere gives the same bits.
pub(crate) fn ended(
    attack: u64,
    release: u64,
    t: u64,
    off: Option<u64>,
    cut: Option<Cut>,
) -> Option<f64> {
    let Some(cut) = Cut::ends(cut, off) else {
        return envelope(attack, release, t, off);
    };
    if t < cut.at || release > 0 {
        return envelope(attack, release, t, Some(cut.at));
    }
    let fallen = t - cut.at;
    if fallen >= cut.fade {
        return None;
    }
    Some(rise(attack, cut.at) * (cut.fade - fallen) as f64 / cut.fade as f64)
}

/// The filter coefficients a live voice plays while a device's mod wheel at `wheel` opens a
/// filter of `cutoff` Hz and `resonance`, at `rate`: the cutoff raised by up to
/// [`WHEEL_OCTAVES`] octaves, kept to the slider's top; `None` at rest (the channel's own).
pub(crate) fn wheel_tone(cutoff: f32, resonance: f32, wheel: u8, rate: u32) -> Option<[f64; 5]> {
    if wheel == 0 {
        return None;
    }
    let opened = f64::from(cutoff) * exp2(WHEEL_OCTAVES * f64::from(wheel) / 127.0);
    let cutoff = opened.min(f64::from(*CUTOFF_RANGE.end()));
    Some(low_pass(cutoff, f64::from(resonance), rate))
}

/// `seconds` as whole frames at `rate`, rounded half away from zero.
fn frames_of(seconds: f32, rate: u32) -> u64 {
    (f64::from(seconds) * f64::from(rate)).round() as u64
}

/// The cookbook's low-pass biquad at `cutoff` Hz with `resonance` (0..1) at `rate`, normalized:
/// `[b0, b1, b2, a1, a2]`.
fn low_pass(cutoff: f64, resonance: f64, rate: u32) -> [f64; 5] {
    let fs = f64::from(rate);
    let cutoff = cutoff.min(MAX_CUTOFF_FRACTION * fs);
    let q = FRAC_1_SQRT_2 + resonance * (MAX_Q - FRAC_1_SQRT_2);
    let w0 = 2.0 * PI * cutoff / fs;
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / (2.0 * q);
    let a0 = 1.0 + alpha;
    let b1 = 1.0 - cos;
    let b0 = b1 / 2.0;
    [
        b0 / a0,
        b1 / a0,
        b0 / a0,
        -2.0 * cos / a0,
        (1.0 - alpha) / a0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(change: impl FnOnce(&mut Sound)) -> Shape {
        let mut sound = Sound::default();
        change(&mut sound);
        Shape::compile(&sound, ChannelKind::Sampler, 44_100).expect("not the defaults")
    }

    // The defaults compile to nothing; a tune of 12 semitones is exactly twice the speed and
    // one of 100 cents the semitone's ratio as exp2 gives it.
    #[test]
    fn the_defaults_are_none_and_tunes_are_ratios() {
        assert_eq!(
            Shape::compile(&Sound::default(), ChannelKind::Sampler, 44_100),
            None
        );
        assert_eq!(shape(|sound| sound.semitones = 12).tune, 2.0);
        assert_eq!(shape(|sound| sound.semitones = -12).tune, 0.5);
        assert_eq!(shape(|sound| sound.cents = 100).tune, exp2(100.0 / 1200.0));
    }

    // Hand-computed: a weight of 0.5 plays velocity 0.5 at 0.75, velocity 1 at 1; weight 0 plays
    // every velocity at 1.
    #[test]
    fn the_velocity_weight_lifts_soft_notes() {
        let half = shape(|sound| sound.velocity = 0.5);
        assert_eq!(half.velocity_level(0.5), 0.75);
        assert_eq!(half.velocity_level(1.0), 1.0);
        assert_eq!(shape(|sound| sound.velocity = 0.0).velocity_level(0.2), 1.0);
    }

    // Attack 4 frames: 0, 1/4, 1/2, 3/4, 1. A one-shot without release plays on through its
    // note's end; held, it ends there; a release of 8 frames falls 15 halvings from the level
    // the note left.
    #[test]
    fn the_envelope_rises_and_the_gate_decides_the_end() {
        let attack = shape(|sound| sound.attack = 4.0 / 44_100.0);
        assert_eq!(attack.attack, 4);
        let levels: Vec<f64> = (0..5)
            .map(|t| attack.level(t, None, None).expect("on"))
            .collect();
        assert_eq!(levels, [0.0, 0.25, 0.5, 0.75, 1.0]);
        assert_eq!(
            attack.level(10, Some(3), None),
            Some(1.0),
            "a one-shot hears no end"
        );
        let held = shape(|sound| sound.gate = Some(Gate::Hold));
        assert_eq!(held.level(2, Some(3), None), Some(1.0));
        assert_eq!(
            held.level(3, Some(3), None),
            None,
            "held, it ends at its end"
        );
        let release = shape(|sound| sound.release = 8.0 / 44_100.0);
        assert_eq!(
            release.level(5, Some(3), None),
            Some(exp2(-(15.0 * 2.0) / 8.0))
        );
        assert_eq!(release.level(11, Some(3), None), None);
    }

    // By hand: 2 ms is 88.2 frames at 44.1 kHz (88), 96 at 48 kHz. Cut at frame 4 with a fade of
    // 4 frames and no release: 1 up to the cut and on it, then 3/4, 2/4, 1/4, then ended; with a
    // release of 8 frames, 15 halvings over 8 from the cut; cut inside a 8-frame attack (1/2 at
    // frame 4), the fall starts from 1/2. A note's end heard before the cut stands (held with no
    // release, it ended at frame 3); one on the cut's frame is the cut's.
    #[test]
    fn a_cut_falls_from_its_frame_over_the_release_or_the_fade() {
        assert_eq!(Cut::new(0, 44_100).fade, 88);
        assert_eq!(Cut::new(0, 48_000).fade, 96);
        let cut = Some(Cut { at: 4, fade: 4 });
        let fall: Vec<Option<f64>> = (3..9).map(|t| ended(0, 0, t, None, cut)).collect();
        assert_eq!(
            fall,
            [
                Some(1.0),
                Some(1.0),
                Some(0.75),
                Some(0.5),
                Some(0.25),
                None
            ]
        );
        assert_eq!(ended(0, 8, 6, None, cut), Some(exp2(-(15.0 * 2.0) / 8.0)));
        assert_eq!(ended(0, 8, 12, None, cut), None);
        assert_eq!(ended(8, 0, 6, None, cut), Some(0.25));
        assert_eq!(ended(0, 0, 5, Some(3), cut), None, "its own end came first");
        assert_eq!(ended(0, 0, 3, Some(3), cut), None);
        assert_eq!(
            ended(0, 0, 5, Some(4), cut),
            Some(0.75),
            "an end on the cut's frame"
        );
        assert_eq!(ended(0, 0, 5, Some(9), cut), Some(0.75));
        assert_eq!(Cut::end(cut, Some(9)), Some(4));
        assert_eq!(Cut::end(cut, Some(3)), Some(3));
        assert_eq!(Cut::end(None, Some(3)), Some(3));
    }

    // A ramp 0, 1, 2, 3 read backwards from its start is 3, 2, 1, 0; started half way, 1, 0;
    // forwards half way, 2, 3; at half speed backwards the half frames are the 4-point Hermite
    // half way, (-y0 + 9 y1 + 9 y2 - y3)/16 of the reversed frames, a point before the first or
    // past the last 0: (27 + 18 - 1)/16 = 2.75, (-3 + 18 + 9)/16 = 1.5, (-2 + 9)/16 = 0.4375,
    // -1/16.
    #[test]
    fn a_sampler_reads_backwards_and_from_its_start() {
        let ramp = SampleBuffer::from(vec![0.0, 1.0, 2.0, 3.0]);
        let read = |shape: Shape, speed: f64| -> Vec<f32> {
            (0..10)
                .map_while(|t| shape.sampler_frame(&ramp, t, None, None, &Pitch::steady(speed)))
                .map(|(value, _)| value[0])
                .collect()
        };
        let reverse = shape(|sound| sound.reverse = true);
        assert_eq!(read(reverse, 1.0), [3.0, 2.0, 1.0, 0.0]);
        assert_eq!(
            read(reverse, 0.5),
            [3.0, 2.75, 2.0, 1.5, 1.0, 0.4375, 0.0, -0.0625]
        );
        let half = shape(|sound| sound.start = 0.5);
        assert_eq!(read(half, 1.0), [2.0, 3.0]);
        let both = shape(|sound| {
            sound.start = 0.5;
            sound.reverse = true;
        });
        assert_eq!(read(both, 1.0), [1.0, 0.0]);
    }

    // The filter at its defaults is unity at DC: a step settles to 1, and a filter split across
    // two runs of frames gives the same values as one run.
    #[test]
    fn the_low_pass_passes_dc_and_carries_its_memory() {
        let filter = shape(|sound| {
            sound.filter = true;
            sound.cutoff = 1000.0;
        });
        let mut memory = FilterMemory::default();
        let run: Vec<f32> = (0..4000)
            .map(|_| filter.filtered(&mut memory, [1.0, 1.0], None)[0])
            .collect();
        assert!(
            (run[3999] - 1.0).abs() < 1e-6,
            "settles to 1: {}",
            run[3999]
        );
        assert!(run[0] > 0.0 && run[0] < 0.01, "starts low: {}", run[0]);
        let mut first = FilterMemory::default();
        let mut split: Vec<f32> = (0..1000)
            .map(|_| filter.filtered(&mut first, [1.0, 1.0], None)[0])
            .collect();
        split.extend((1000..4000).map(|_| filter.filtered(&mut first, [1.0, 1.0], None)[0]));
        assert_eq!(split, run);
    }

    // By hand: the wheel at rest leaves the channel's filter; all the way up it opens 1 kHz four
    // octaves, to the coefficients of 16 kHz set by hand (exp2(4) is exactly 16); a wide-open
    // cutoff stops at the slider's 20 kHz.
    #[test]
    fn the_wheel_opens_the_filter_four_octaves() {
        assert_eq!(wheel_tone(1000.0, 0.0, 0, 44_100), None);
        let by_hand = shape(|sound| {
            sound.filter = true;
            sound.cutoff = 16_000.0;
        });
        assert_eq!(wheel_tone(1000.0, 0.0, 127, 44_100), by_hand.filter);
        let top = shape(|sound| {
            sound.filter = true;
            sound.cutoff = 20_000.0;
            sound.resonance = 0.5;
        });
        assert_eq!(wheel_tone(5000.0, 0.5, 127, 44_100), top.filter);
    }
}
