//! Transients: the frames where a sound's clear hits start (drums, plucks), found offline in the
//! sound a channel plays (its file through its edits), for the marks drawn on a selected audio
//! clip and the markers a quantize places. Soft or legato material gives few or wrong marks;
//! markers can always be placed by hand.
//!
//! The rule, block by block (gen_p39_expected.py restates it): the energy of each [`BLOCK`]
//! frames, every side's squares summed (why not a mix to one side: a click whose sides are in
//! opposite phase would sum to nothing); a block whose energy is not finite (a NaN or an infinity
//! in it) is no onset and counts 0 in every mean; a block is an onset when its energy is above
//! [`RISE`] times the mean of the [`HISTORY`] blocks before it plus [`FLOOR`], and at least
//! [`GAP`] blocks after the onset before it; its frame is the first of that block and the next
//! whose magnitude (the largest of its sides', a value not finite counting 0) reaches a quarter
//! of their peak. Every sum is taken in frame order, side by side, so the generator gives the
//! same frames bit for bit.

use dawsome_engine::dsp::sampler::SampleBuffer;

/// Frames each energy is taken over.
pub const BLOCK: usize = 128;

/// The blocks before a block whose mean energy it is held to (those before the sound count as
/// silence).
pub const HISTORY: usize = 16;

/// How many times the mean before it a block's energy must be. Why 2.5: a step up of 6 dB (4
/// times the energy) is a hit, one of 2 dB (1.6 times) is not; 2.5 lies between them by the
/// same factor either way.
pub const RISE: f64 = 2.5;

/// The energy a block must be above besides: a block of 128 frames at -60 dBFS RMS, so noise in
/// a quiet passage is no hit.
pub const FLOOR: f64 = 1.28e-4;

/// The fewest blocks from one onset to the next (64 ms at 48 kHz): a hit's own ringing is not
/// another.
pub const GAP: usize = 24;

/// The onsets of `sound`, rising, in its frames.
pub fn of(sound: &SampleBuffer) -> Vec<u64> {
    of_samples(sound.samples(), sound.channels())
}

/// The onsets of the interleaved `samples` of `channels` sides: a sound held as frames to edit
/// rather than as a [`SampleBuffer`] (the sample editor's). Read in place, block by block: the
/// only copy is one energy per block.
pub fn of_samples(samples: &[f32], channels: u8) -> Vec<u64> {
    let sides = usize::from(channels.max(1));
    let span = BLOCK * sides;
    let energy: Vec<f64> = samples
        .chunks(span)
        .map(|block| {
            block.iter().fold(0.0, |sum, &value| {
                let value = f64::from(value);
                sum + value * value
            })
        })
        .collect();
    let mut found = Vec::new();
    let mut last: Option<usize> = None;
    for (index, &level) in energy.iter().enumerate() {
        if last.is_some_and(|last| index - last < GAP) || !level.is_finite() {
            continue;
        }
        let before = energy[index.saturating_sub(HISTORY)..index]
            .iter()
            .filter(|level| level.is_finite())
            .fold(0.0, |sum, level| sum + level);
        if level <= RISE * (before / HISTORY as f64) + FLOOR {
            continue;
        }
        let start = index * span;
        let window = &samples[start..(start + 2 * span).min(samples.len())];
        let magnitudes = || window.chunks(sides).map(magnitude);
        let quarter = magnitudes().fold(0.0_f64, f64::max) * 0.25;
        if let Some(offset) = magnitudes().position(|level| level >= quarter) {
            found.push((index * BLOCK + offset) as u64);
            last = Some(index);
        }
    }
    found
}

/// The largest magnitude of one frame's sides, a value not finite counting 0.
fn magnitude(frame: &[f32]) -> f64 {
    frame
        .iter()
        .map(|&value| f64::from(value))
        .filter(|value| value.is_finite())
        .fold(0.0, |peak, value| peak.max(value.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_and_an_empty_sound_hold_no_onset() {
        assert!(of(&SampleBuffer::mono(vec![0.0; 10_000])).is_empty());
        assert!(of(&SampleBuffer::mono(Vec::new())).is_empty());
        // Below the floor: a block at about -66 dBFS.
        assert!(of(&SampleBuffer::mono(vec![0.0005; 10_000])).is_empty());
    }

    // A click in the sound's last, short block is found there.
    #[test]
    fn a_click_in_the_last_short_block_is_found() {
        let mut values = vec![0.0; 1000];
        values[990] = 0.5;
        assert_eq!(of(&SampleBuffer::mono(values)), [990]);
    }

    // A click on the right side alone of a stereo sound is found, at its frame.
    #[test]
    fn a_click_on_one_side_is_found() {
        let mut values = vec![0.0; 2 * 4000];
        values[2 * 3000 + 1] = -0.5;
        assert_eq!(of(&SampleBuffer::stereo(values)), [3000]);
    }
}
