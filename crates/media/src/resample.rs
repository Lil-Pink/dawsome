//! Converting a decoded file to another sample rate once, when it is loaded, so playback reads
//! it frame for frame at the project's rate.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use thiserror::Error;

/// Frames the resampler takes per step. Only the delay and the memory depend on it, and the
/// delay is trimmed.
const CHUNK_FRAMES: usize = 1024;

/// Frames of a loop played on past a file's last frame for the resampler to hear, at least.
/// Why this many: the converted frames near the end are made from the input up to a chunk and
/// more past them; four chunks gives the same bits as a far longer run.
const LOOP_FRAMES_AFTER: usize = 4 * CHUNK_FRAMES;

#[derive(Debug, Error)]
pub enum ResampleError {
    #[error("cannot resample {from} Hz to {to} Hz: {reason}")]
    Unsupported { from: u32, to: u32, reason: String },
}

/// `samples` (`channels` interleaved values per frame) at `from` Hz, resampled to `to` Hz by
/// an FFT resampler at the fixed ratio. The output has exactly ceil(frames · to / from) frames
/// and frame 0 stays at time 0: the resampler's delay is trimmed and its tail flushed.
///
/// `after_last` is the frame the sound goes on from after its last one, when it does: a loop's
/// start, where a voice reads next once the loop ends on the last frame. The resampler then
/// hears the loop played on past the end rather than silence, so the last frames come out as
/// the loop reads through its wrap (silence there bent them: a tick at every wrap). `None`:
/// silence follows the last frame.
pub fn resample(
    samples: &[f32],
    channels: usize,
    from: u32,
    to: u32,
    after_last: Option<usize>,
) -> Result<Vec<f32>, ResampleError> {
    let unsupported = |reason: String| ResampleError::Unsupported { from, to, reason };
    if from == to || samples.is_empty() {
        return Ok(samples.to_vec());
    }
    let channels = channels.max(1);
    let frames = samples.len() / channels;
    let looped;
    let (samples, input_frames) = match after_last.filter(|&start| start < frames) {
        Some(start) => {
            let held = &samples[..frames * channels];
            let lap = &held[start * channels..];
            let total = (frames + LOOP_FRAMES_AFTER) * channels;
            let mut on = Vec::with_capacity(total);
            on.extend_from_slice(held);
            while on.len() < total {
                let take = (total - on.len()).min(lap.len());
                on.extend_from_slice(&lap[..take]);
            }
            looped = on;
            (looped.as_slice(), looped.len() / channels)
        }
        None => (samples, frames),
    };
    let mut resampler = Fft::<f32>::new(
        from as usize,
        to as usize,
        CHUNK_FRAMES,
        channels,
        FixedSync::Input,
    )
    .map_err(|error| unsupported(error.to_string()))?;
    let input = InterleavedSlice::new(&samples[..input_frames * channels], channels, input_frames)
        .map_err(|error| unsupported(error.to_string()))?;
    let output = resampler
        .process_all(&input, input_frames, None)
        .map_err(|error| unsupported(error.to_string()))?;
    // Why not the resampler's own length: it is the ceiling of a float ratio, one frame long
    // when frames · to / from is a whole number the float lands just above (441 → 480 at
    // 44.1 to 48 kHz gives 481).
    let exact = (frames as u64 * u64::from(to)).div_ceil(u64::from(from)) as usize;
    let mut data = output.take_data();
    data.resize(exact * channels, 0.0);
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ceil(2205 · 48000 / 44100) = 2400 and ceil(1 · 48000 / 44100) = 2 (Python).
    #[test]
    fn the_output_has_the_whole_converted_length() {
        let click = [vec![1.0f32], vec![0.0; 2204]].concat();
        assert_eq!(
            resample(&click, 1, 44_100, 48_000, None)
                .expect("resamples")
                .len(),
            2400
        );
        assert_eq!(
            resample(&[1.0], 1, 44_100, 48_000, None)
                .expect("resamples")
                .len(),
            2
        );
        let stereo = vec![0.5f32; 2 * 441];
        assert_eq!(
            resample(&stereo, 2, 44_100, 48_000, None)
                .expect("resamples")
                .len(),
            2 * 480
        );
    }

    // A steady level stays that level away from the ends, and a click stays at the start:
    // its loudest frame is frame 0, so the delay was trimmed.
    #[test]
    fn levels_and_timing_survive_the_conversion() {
        let steady = vec![0.5f32; 44_100];
        let out = resample(&steady, 1, 44_100, 48_000, None).expect("resamples");
        let middle = &out[4_800..43_200];
        assert!(
            middle.iter().all(|&v| (v - 0.5).abs() < 1e-3),
            "steady level"
        );
        let click = [vec![1.0f32], vec![0.0; 4_409]].concat();
        let out = resample(&click, 1, 44_100, 48_000, None).expect("resamples");
        let loudest = out
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .map(|(at, _)| at);
        assert_eq!(loudest, Some(0));
    }

    // A 100 Hz sine of whole periods (480 frames at 48 kHz, 441 at 44.1 kHz) looping its last
    // ten periods to its last frame converts as the same file with ten more periods after it
    // (the 808s' way round it): the loop's last frames, as a voice reads them into its wrap,
    // are the sine's to 1e-6 of each other. With silence after, they were 1e-4 off. The output
    // keeps the file's own converted length, 8820 frames.
    #[test]
    fn a_loop_to_the_last_frame_converts_as_it_plays_on() {
        let period: Vec<f32> = (0..480)
            .map(|i| (0.5 * (2.0 * std::f64::consts::PI * f64::from(i) / 480.0).sin()) as f32)
            .collect();
        let file = period.repeat(20);
        let longer = period.repeat(30);
        let looped = resample(&file, 1, 48_000, 44_100, Some(4800)).expect("resamples");
        let silent = resample(&file, 1, 48_000, 44_100, None).expect("resamples");
        let reference = resample(&longer, 1, 48_000, 44_100, None).expect("resamples");
        assert_eq!((looped.len(), silent.len()), (8820, 8820));
        let off = |out: &[f32]| {
            out[4410..]
                .iter()
                .zip(&reference[4410..8820])
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max)
        };
        assert!(off(&looped) < 1e-6, "{}", off(&looped));
        assert!(off(&silent) > 1e-5, "{}", off(&silent));
        let far = resample(&file, 1, 48_000, 44_100, Some(0)).expect("resamples");
        assert_eq!(far.len(), 8820);
        assert_eq!(
            resample(&file, 1, 48_000, 44_100, Some(9600)).expect("past"),
            silent
        );
    }

    #[test]
    fn the_same_rate_or_no_audio_passes_through() {
        assert_eq!(
            resample(&[0.25, 0.5], 1, 48_000, 48_000, None).expect("same"),
            [0.25, 0.5]
        );
        assert!(resample(&[], 2, 44_100, 48_000, None)
            .expect("empty")
            .is_empty());
    }
}
