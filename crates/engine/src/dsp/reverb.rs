//! The reverb: a Schroeder–Moorer reverberator in the Freeverb arrangement (Jezar's public-domain
//! design). Per side, eight lowpass-feedback comb filters in parallel build the dense tail, and
//! four Schroeder allpasses in series smear it; the right side's delays are slightly longer, so
//! the two sides decorrelate into a stereo image.
//!
//! The settings, and why:
//! - Comb lengths at 44.1 kHz are eight primes from 25 to 37 ms (1117 to 1613 frames), so no two
//!   combs share a period and their echoes never stack into a pattern. Nothing comes out before
//!   the shortest comb's first pass.
//! - The right side adds 23 frames to every delay (the Freeverb stereo spread). That makes the
//!   right side's comb lengths even, so only the left set is mutually prime; the offset matters
//!   more for the image than primality does there.
//! - Allpasses: 556, 441, 341 and 225 frames (5 to 13 ms), gain 0.5, the Freeverb tuning.
//! - Input gain 0.015 on the mono sum and wet gain 3, the Freeverb levels.
//! - Comb feedback 0.70 + 0.28·room, damping 0.4·damping (the lowpass inside each comb).
//! - Every length scales with the sample rate, rounded to the nearest frame.

use super::{blend, snap};
use crate::processor::{ParamInfo, ProcessContext, Processor, ProcessorFactory};

pub const ID: &str = "dawsome.reverb";

const COMBS_44K: [usize; 8] = [1117, 1187, 1277, 1361, 1423, 1489, 1559, 1613];
const ALLPASSES_44K: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD_44K: usize = 23;
const REFERENCE_RATE: u64 = 44_100;

const INPUT_GAIN: f32 = 0.015;
const WET_GAIN: f32 = 3.0;
const ALLPASS_GAIN: f32 = 0.5;
const FEEDBACK_BASE: f32 = 0.7;
const FEEDBACK_PER_ROOM: f32 = 0.28;
const DAMPING_SCALE: f32 = 0.4;

const fn unit_param(key: &'static str, label: &'static str, default: f32) -> ParamInfo {
    ParamInfo {
        key,
        label,
        min: 0.0,
        max: 1.0,
        default,
        unit: "%",
        whole: false,
    }
}

const PARAMS: [ParamInfo; 4] = [
    unit_param("room", "Room", 0.5),
    unit_param("damping", "Damping", 0.5),
    unit_param("width", "Width", 1.0),
    unit_param("mix", "Mix", 0.25),
];

const ROOM: usize = 0;
const DAMPING: usize = 1;
const WIDTH: usize = 2;
const MIX: usize = 3;

pub(super) struct Factory;

impl ProcessorFactory for Factory {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Reverb"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn create(&self, sample_rate: u32, values: &[f32]) -> Box<dyn Processor> {
        let side = |spread: usize| Side {
            combs: COMBS_44K.map(|len| Comb::new(scaled(len + spread, sample_rate))),
            allpasses: ALLPASSES_44K.map(|len| Allpass::new(scaled(len + spread, sample_rate))),
        };
        let mut reverb = Reverb {
            sides: [side(0), side(STEREO_SPREAD_44K)],
            values: PARAMS.map(|info| info.default),
        };
        for (index, &value) in values.iter().enumerate() {
            reverb.set_param(index, value);
        }
        Box::new(reverb)
    }
}

/// `frames` at 44.1 kHz as frames at `sample_rate`, rounded to the nearest, at least 1.
fn scaled(frames: usize, sample_rate: u32) -> usize {
    let frames = frames as u64 * u64::from(sample_rate);
    let rounded = (frames + REFERENCE_RATE / 2) / REFERENCE_RATE;
    usize::try_from(rounded.max(1)).unwrap_or(usize::MAX)
}

/// A feedback comb with a one-pole lowpass in its loop.
struct Comb {
    line: Box<[f32]>,
    at: usize,
    /// The lowpass's memory.
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self {
            line: vec![0.0; len].into_boxed_slice(),
            at: 0,
            store: 0.0,
        }
    }

    fn tick(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let output = self.line[self.at];
        self.store = snap(output * (1.0 - damp) + self.store * damp);
        self.line[self.at] = snap(input + self.store * feedback);
        self.at = (self.at + 1) % self.line.len();
        output
    }

    fn reset(&mut self) {
        self.line.fill(0.0);
        self.at = 0;
        self.store = 0.0;
    }
}

/// A Schroeder allpass: flat in level, it spreads each echo out in time.
struct Allpass {
    line: Box<[f32]>,
    at: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self {
            line: vec![0.0; len].into_boxed_slice(),
            at: 0,
        }
    }

    fn tick(&mut self, input: f32) -> f32 {
        let delayed = self.line[self.at];
        self.line[self.at] = snap(input + delayed * ALLPASS_GAIN);
        self.at = (self.at + 1) % self.line.len();
        delayed - input
    }

    fn reset(&mut self) {
        self.line.fill(0.0);
        self.at = 0;
    }
}

struct Side {
    combs: [Comb; 8],
    allpasses: [Allpass; 4],
}

impl Side {
    fn tick(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let mut sum = 0.0;
        for comb in &mut self.combs {
            sum += comb.tick(input, feedback, damp);
        }
        for allpass in &mut self.allpasses {
            sum = allpass.tick(sum);
        }
        sum
    }
}

struct Reverb {
    sides: [Side; 2],
    values: [f32; 4],
}

impl Processor for Reverb {
    fn process(&mut self, _context: &ProcessContext, buffer: &mut [[f32; 2]]) {
        let feedback = FEEDBACK_BASE + FEEDBACK_PER_ROOM * self.values[ROOM];
        let damp = DAMPING_SCALE * self.values[DAMPING];
        let width = self.values[WIDTH];
        // Each side's own tail, and how much of the other side's is mixed in: none at full
        // width, half of each at width 0 (mono).
        let own = WET_GAIN * (width / 2.0 + 0.5);
        let other = WET_GAIN * ((1.0 - width) / 2.0);
        let mix = self.values[MIX];
        for frame in buffer {
            let input = (frame[0] + frame[1]) * INPUT_GAIN;
            let [left, right] = &mut self.sides;
            let tails = [
                left.tick(input, feedback, damp),
                right.tick(input, feedback, damp),
            ];
            let wet = [
                tails[0] * own + tails[1] * other,
                tails[1] * own + tails[0] * other,
            ];
            *frame = blend(*frame, wet, mix);
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(info) = PARAMS.get(index) {
            self.values[index] = info.clamp(value);
        }
    }

    fn reset(&mut self) {
        for side in &mut self.sides {
            side.combs.iter_mut().for_each(Comb::reset);
            side.allpasses.iter_mut().for_each(Allpass::reset);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::tests::{bits, impulse, noise, whole_and_split, CONTEXT};

    fn reverb(mix: f32) -> Box<dyn Processor> {
        Factory.create(44_100, &[0.5, 0.5, 1.0, mix])
    }

    #[test]
    fn comb_lengths_are_primes_from_25_to_37_ms() {
        let is_prime = |n: usize| {
            n > 1
                && (2..n)
                    .take_while(|d| d * d <= n)
                    .all(|d| !n.is_multiple_of(d))
        };
        for len in COMBS_44K {
            assert!(is_prime(len), "{len}");
            let ms = len as f64 / 44.1;
            assert!((25.0..=37.0).contains(&ms), "{len} frames is {ms} ms");
        }
        for len in ALLPASSES_44K {
            let ms = len as f64 / 44.1;
            assert!((5.0..=13.0).contains(&ms), "{len} frames is {ms} ms");
        }
        assert_eq!(scaled(1117, 44_100), 1117);
        // 1117 · 48000 / 44100 = 1215.78, rounded up.
        assert_eq!(scaled(1117, 48_000), 1216);
    }

    #[test]
    fn the_tail_starts_after_the_shortest_comb_and_the_sides_differ() {
        let mut buffer = impulse(44_100);
        reverb(1.0).process(&CONTEXT, &mut buffer);
        assert!(buffer[..1117].iter().all(|frame| *frame == [0.0; 2]));
        assert_ne!(buffer[1117], [0.0; 2]);
        assert!(buffer.iter().any(|frame| frame[0] != frame[1]));
        assert!(buffer[22_050..].iter().any(|frame| frame[0].abs() > 1e-4));
    }

    #[test]
    fn mix_0_passes_the_input_and_any_split_gives_the_same_bits() {
        let input = noise(5000);
        let mut output = input.clone();
        reverb(0.0).process(&CONTEXT, &mut output);
        assert_eq!(bits(&output), bits(&input));
        let mut make = || reverb(0.4);
        let (whole, split) = whole_and_split(&mut make, &input, &[1, 1117, 7, 512]);
        assert_eq!(bits(&whole), bits(&split));
    }

    #[test]
    fn reset_silences_the_tail() {
        let mut instance = reverb(1.0);
        let mut first = noise(3000);
        instance.process(&CONTEXT, &mut first);
        instance.reset();
        let mut after = vec![[0.0; 2]; 3000];
        instance.process(&CONTEXT, &mut after);
        assert!(after.iter().all(|frame| *frame == [0.0; 2]));
    }
}
