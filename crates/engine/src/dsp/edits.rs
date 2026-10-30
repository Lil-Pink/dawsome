//! A sound's edits ([`SoundEdits`]) applied to its frames ([`apply`]), before anything plays
//! them, and where an edge snaps to the nearest zero crossing ([`zero_crossing`]). Pure: the
//! media layer runs them on a decoded file (on a worker, or before an offline render), and the
//! audio thread only ever plays what they gave.
//!
//! The rules (tests/fixtures/gen_p30_expected.py restates them):
//! - range: an op's `[from, to)` in frames of the sound as the ops before it leave it, cut to
//!   its end (`lo = min(from, n)`, `hi = min(to, n)` for a sound of n frames); an op whose cut
//!   range is empty does nothing; N = hi − lo, k = 0..N−1 the frame within it.
//! - crop keeps [lo, hi); delete removes it, the rest closing up; silence sets it to 0.0.
//! - fade in scales frame k by k/N, fade out by (N − 1 − k)/N.
//! - gain c scales by exp2(c/100 · log2 10/20), normalize t by exp2(t/100 · log2 10/20)/P, P the
//!   largest |x| in the range on either side (nothing when P = 0); [`exp2`] is + − × ÷ only.
//! - reverse: frame lo + k takes the old frame hi − 1 − k.
//! - every product formed in f64 and rounded to f32 once, so Linux, Windows and the Python
//!   generator give the same bits.

use std::f64::consts::LOG2_10;

use super::exp2;
use super::sampler::SampleBuffer;
use crate::project::{SoundEdits, SoundOp};

/// `sound` played through `edits`: the frames the list gives, in order. An empty list gives the
/// sound itself, its frames shared.
pub fn apply(sound: &SampleBuffer, edits: &SoundEdits) -> SampleBuffer {
    apply_ops(sound, &edits.ops)
}

/// `sound` through `ops`, in order: [`apply`] for a part of a list, so a sound some first ops
/// already made takes the rest alone and gives the bits the whole list gives.
pub fn apply_ops(sound: &SampleBuffer, ops: &[SoundOp]) -> SampleBuffer {
    if ops.is_empty() {
        return sound.clone();
    }
    let sides = usize::from(sound.channels());
    let mut samples = sound.samples().to_vec();
    for op in ops {
        apply_in_place(&mut samples, sides, op);
    }
    if sides == 1 {
        SampleBuffer::mono(samples)
    } else {
        SampleBuffer::stereo(samples)
    }
}

/// The gain of `centi_db` hundredths of a dB: exp2(c/100 · log2 10/20).
fn db_gain(centi_db: i32) -> f64 {
    exp2(f64::from(centi_db) / 100.0 * LOG2_10 / 20.0)
}

/// Applies `op` to the interleaved `samples` of `sides` sides (1 or 2) where they are: the
/// frames of its range alone are touched, but for a crop or a delete, which move the rest. Why
/// apart from [`apply_ops`]: a sound held for editing takes one op more in O(range) this way,
/// where a new [`SampleBuffer`] copies every frame twice (230 MB each for ten minutes of
/// stereo).
pub fn apply_in_place(samples: &mut Vec<f32>, sides: usize, op: &SoundOp) {
    let frames = (samples.len() / sides) as u64;
    let (lo, hi) = op.cut(frames);
    if lo == hi {
        return;
    }
    // Inside the samples, so they fit a usize.
    let (lo, hi) = (lo as usize, hi as usize);
    let n = hi - lo;
    let range = lo * sides..hi * sides;
    // Frame k of the range (its values side by side) scaled by `factor(k)`.
    let scale = |samples: &mut [f32], factor: &dyn Fn(usize) -> f64| {
        for (k, frame) in samples.chunks_exact_mut(sides).enumerate() {
            let factor = factor(k);
            for value in frame {
                *value = (f64::from(*value) * factor) as f32;
            }
        }
    };
    match *op {
        SoundOp::Crop { .. } => {
            samples.truncate(range.end);
            samples.drain(..range.start);
        }
        SoundOp::Delete { .. } => {
            samples.drain(range);
        }
        SoundOp::Silence { .. } => samples[range].fill(0.0),
        SoundOp::FadeIn { .. } => scale(&mut samples[range], &|k| k as f64 / n as f64),
        SoundOp::FadeOut { .. } => {
            scale(&mut samples[range], &|k| (n - 1 - k) as f64 / n as f64);
        }
        SoundOp::Gain { centi_db, .. } => {
            let gain = db_gain(centi_db);
            scale(&mut samples[range], &|_| gain);
        }
        SoundOp::Normalize { centi_db, .. } => {
            let peak = samples[range.clone()]
                .iter()
                .map(|value| f64::from(value.abs()))
                .fold(0.0, f64::max);
            if peak == 0.0 {
                return;
            }
            let gain = db_gain(centi_db) / peak;
            scale(&mut samples[range], &|_| gain);
        }
        SoundOp::Reverse { .. } => {
            let part = &mut samples[range];
            for k in 0..n / 2 {
                let (front, back) = (k * sides, (n - 1 - k) * sides);
                for side in 0..sides {
                    part.swap(front + side, back + side);
                }
            }
        }
    }
}

/// The frame an edge at `frame` of `sound` snaps to: the nearest boundary within `reach` frames
/// where the sound (its sides summed) crosses zero, the earlier of two as near; `frame` itself
/// when none is in reach. A frame whose sum is exactly 0 is a boundary at it; a sign change
/// between frames i − 1 and i is a boundary at i. Why the sum: an edge cuts both sides at once,
/// and where their sum crosses is where the cut is quietest.
pub fn zero_crossing(sound: &SampleBuffer, frame: u64, reach: u64) -> u64 {
    zero_crossing_in(sound.samples(), usize::from(sound.channels()), frame, reach)
}

/// As [`zero_crossing`], in the interleaved `samples` of `sides` sides (1 or 2): a sound held
/// as frames to edit (the sample editor's).
pub fn zero_crossing_in(samples: &[f32], sides: usize, frame: u64, reach: u64) -> u64 {
    let sides = sides.max(1);
    let frames = (samples.len() / sides) as u64;
    let sum = |index: u64| {
        usize::try_from(index)
            .ok()
            .and_then(|index| samples.get(index * sides..(index + 1) * sides))
            .map_or(0.0, |values| {
                values.iter().map(|&value| f64::from(value)).sum()
            })
    };
    let boundary = |index: u64| {
        if index >= frames {
            return false;
        }
        let here = sum(index);
        let crossed = index > 0 && {
            let before = sum(index - 1);
            (before < 0.0 && here > 0.0) || (before > 0.0 && here < 0.0)
        };
        here == 0.0 || crossed
    };
    for distance in 0..=reach {
        if let Some(before) = frame.checked_sub(distance) {
            if boundary(before) {
                return before;
            }
        }
        if let Some(after) = frame.checked_add(distance) {
            if distance > 0 && boundary(after) {
                return after;
            }
        }
        // Past both ends of the sound nothing more is found.
        if distance > frame && frame.saturating_add(distance) >= frames {
            break;
        }
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    // Applying a list op by op from a sound a first part made gives the bits of the whole list
    // (the incremental path media takes rests on it); an empty list shares the frames.
    #[test]
    fn a_list_applied_in_two_parts_gives_the_bits_of_the_whole() {
        // 64 frames of stereo.
        let sound = SampleBuffer::stereo((0..128).map(|i| (i as f32 - 20.0) / 37.0).collect());
        let ops = [
            SoundOp::FadeIn { from: 0, to: 10 },
            SoundOp::Crop { from: 3, to: 50 },
            SoundOp::Gain {
                from: 5,
                to: 30,
                centi_db: -250,
            },
            SoundOp::Reverse { from: 0, to: 47 },
            SoundOp::Normalize {
                from: 0,
                to: 47,
                centi_db: -30,
            },
        ];
        let whole = apply_ops(&sound, &ops);
        let first = apply_ops(&sound, &ops[..2]);
        let rest = apply_ops(&first, &ops[2..]);
        assert_eq!(whole.samples(), rest.samples());
        assert_eq!(whole.frames(), 47);
        let same = apply_ops(&sound, &[]);
        assert!(std::ptr::eq(same.samples(), sound.samples()));
    }

    // A frame reached from both sides at once: the earlier boundary wins; nothing within reach
    // leaves the edge; an edge at the sound's end looks back into it.
    #[test]
    fn an_edge_snaps_to_the_nearest_crossing_in_reach() {
        let sound = SampleBuffer::mono(vec![1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0]);
        // Crossings at 2 (1 to -1) and 4 (-1 to 1).
        assert_eq!(zero_crossing(&sound, 3, 5), 2);
        assert_eq!(zero_crossing(&sound, 4, 0), 4);
        assert_eq!(zero_crossing(&sound, 6, 1), 6);
        assert_eq!(zero_crossing(&sound, 6, 2), 4);
        assert_eq!(zero_crossing(&sound, 7, 3), 4);
        assert_eq!(zero_crossing(&sound, 0, 100), 2);
    }
}
