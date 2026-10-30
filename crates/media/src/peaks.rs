//! Waveform peaks: the smallest and largest value of every 256 frames of a sound, per channel,
//! which is all a clip's waveform needs to be drawn at any zoom.

use dawsome_engine::dsp::sampler::SampleBuffer;

/// Frames each peak covers.
pub const PEAK_FRAMES: usize = 256;

/// A sound's peaks: for each channel, a (min, max) pair per [`PEAK_FRAMES`] frames, the last
/// pair covering what is left.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Peaks {
    /// The sound's length in frames.
    pub frames: u64,
    /// One list per channel (1 or 2).
    pub channels: Vec<Vec<(f32, f32)>>,
}

impl Peaks {
    /// The peaks of `buffer`.
    pub fn of(buffer: &SampleBuffer) -> Self {
        Self::of_samples(buffer.samples(), buffer.channels())
    }

    /// The peaks of the interleaved `samples` of `channels` sides (1 or 2): a sound held as
    /// frames to edit rather than as a [`SampleBuffer`] (the sample editor's).
    pub fn of_samples(samples: &[f32], channels: u8) -> Self {
        let width = usize::from(channels.max(1));
        let pairs = samples.len().div_ceil(PEAK_FRAMES * width);
        let mut peaks = Self {
            frames: (samples.len() / width) as u64,
            channels: vec![Vec::with_capacity(pairs); width],
        };
        peaks.refresh(samples, channels, 0, None);
        peaks
    }

    /// Makes the pairs again over the frames from `from` to `to` of the interleaved `samples`
    /// (of `channels` sides), which changed there, or from `from` to their end when `to` is
    /// none (their length changed), the lists then cut to the sound's length. Why in parts: an
    /// op on a part of ten minutes changes that part alone, and every pair is 256 frames.
    pub fn refresh(&mut self, samples: &[f32], channels: u8, from: u64, to: Option<u64>) {
        let width = usize::from(channels.max(1));
        let frames = samples.len() / width;
        self.frames = frames as u64;
        let pairs = frames.div_ceil(PEAK_FRAMES);
        let first = usize::try_from(from / PEAK_FRAMES as u64).unwrap_or(usize::MAX);
        let end = match to {
            Some(to) => usize::try_from(to.div_ceil(PEAK_FRAMES as u64)).unwrap_or(usize::MAX),
            None => pairs,
        }
        .min(pairs);
        self.channels.resize(width, Vec::new());
        for (channel, list) in self.channels.iter_mut().enumerate() {
            list.resize(pairs, (0.0, 0.0));
            for (pair, slot) in list.iter_mut().enumerate().take(end).skip(first) {
                let start = pair * PEAK_FRAMES * width;
                let stop = (start + PEAK_FRAMES * width).min(samples.len());
                *slot = samples[start..stop]
                    .iter()
                    .skip(channel)
                    .step_by(width)
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &v| {
                        (lo.min(v), hi.max(v))
                    });
            }
        }
    }

    /// The smallest and largest value of `channel` over frames `from..to` (0 past the end), or
    /// `None` for an empty span or a channel the sound does not have.
    pub fn span(&self, channel: usize, from: u64, to: u64) -> Option<(f32, f32)> {
        let pairs = self.channels.get(channel)?;
        let to = to.min(self.frames);
        if from >= to {
            return None;
        }
        let first = usize::try_from(from / PEAK_FRAMES as u64).ok()?;
        let last = usize::try_from((to - 1) / PEAK_FRAMES as u64).ok()?;
        pairs
            .get(first..=last.min(pairs.len().saturating_sub(1)))?
            .iter()
            .copied()
            .reduce(|(lo, hi), (a, b)| (lo.min(a), hi.max(b)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 600 mono frames k/600: peaks over 0..256, 256..512 and the 88 left.
    #[test]
    fn peaks_cover_every_256_frames_and_spans_combine_them() {
        let buffer = SampleBuffer::mono((0..600).map(|k| k as f32 / 600.0).collect());
        let peaks = Peaks::of(&buffer);
        assert_eq!(peaks.frames, 600);
        assert_eq!(
            peaks.channels,
            [vec![
                (0.0, 255.0 / 600.0),
                (256.0 / 600.0, 511.0 / 600.0),
                (512.0 / 600.0, 599.0 / 600.0)
            ]]
        );
        assert_eq!(
            peaks.span(0, 300, 520),
            Some((256.0 / 600.0, 599.0 / 600.0))
        );
        assert_eq!(peaks.span(0, 600, 700), None, "past the end");
        assert_eq!(peaks.span(1, 0, 10), None, "no second channel");
        let stereo = Peaks::of(&SampleBuffer::stereo(vec![1.0, -1.0, 0.5, 0.25]));
        assert_eq!(stereo.channels, [vec![(0.5, 1.0)], vec![(-1.0, 0.25)]]);
    }

    // A part made again is what the whole made anew gives: 600 frames k/600 with frames 300 to
    // 309 set to 2.0 (in the second pair), then with 400 frames taken off the front (a crop's
    // length changing: refreshed to the end, the lists cut to 200 frames, one pair).
    #[test]
    fn a_part_made_again_gives_the_peaks_of_the_whole() {
        let mut samples: Vec<f32> = (0..600).map(|k| k as f32 / 600.0).collect();
        let mut peaks = Peaks::of_samples(&samples, 1);
        samples[300..310].fill(2.0);
        peaks.refresh(&samples, 1, 300, Some(310));
        assert_eq!(peaks, Peaks::of_samples(&samples, 1));
        assert_eq!(peaks.channels[0][1], (256.0 / 600.0, 2.0));
        samples.drain(..400);
        peaks.refresh(&samples, 1, 0, None);
        assert_eq!(peaks, Peaks::of(&SampleBuffer::mono(samples.clone())));
        assert_eq!(peaks.frames, 200);
        assert_eq!(peaks.channels[0], [(400.0 / 600.0, 599.0 / 600.0)]);
    }
}
