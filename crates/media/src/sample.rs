//! The sound a channel plays, decoded into memory at the project's rate. Loading never refuses
//! a project: a channel whose file is missing or unreadable plays silence, and the caller shows
//! a notice.

use std::path::{Path, PathBuf};

use dawsome_engine::dsp::instrument::FileLoop;
use dawsome_engine::dsp::sampler::SampleBuffer;
use thiserror::Error;

use crate::band_limit::band_limit;
use crate::decode::{decode, Decoded};
use crate::resample::resample;

/// Something worth telling the user about a channel's sound.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SampleNotice {
    #[error("sample {} cannot be read ({reason}); its channel plays silence", path.display())]
    Unreadable { path: PathBuf, reason: String },
    /// The file could not be converted to the project's rate, so it plays frame for frame:
    /// faster or slower than recorded.
    #[error("sample {} is {file_rate} Hz but the project is {project_rate} Hz, and it cannot be converted ({reason}); it plays at the wrong speed", path.display())]
    WrongRate {
        path: PathBuf,
        file_rate: u32,
        project_rate: u32,
        reason: String,
    },
    /// Sounds play in mono or stereo; a file with more channels plays its first two.
    #[error("sample {} has {channels} channels; it plays the first two", path.display())]
    ExtraChannels { path: PathBuf, channels: usize },
    /// A stretched clip's file could not be stretched: the clip plays silence.
    #[error("{} cannot be stretched ({reason}); its clip plays silence", path.display())]
    Unstretchable { path: PathBuf, reason: String },
    /// Every stream is in use (64): this long file loads into memory instead.
    #[error("{} loads into memory: 64 files stream already", path.display())]
    TooManyStreams { path: PathBuf },
    /// An instrument set left something out: opcodes or headers its reader does not play, or
    /// parts it could not read (see its [`crate::sounds::InstrumentReport`]).
    #[error("instrument {}: {summary}", path.display())]
    Instrument { path: PathBuf, summary: String },
}

/// A channel's sound and what to tell the user about it.
#[derive(Debug, Clone)]
pub struct ChannelSample {
    pub buffer: SampleBuffer,
    /// The file's own rate, before conversion; `None` for silence.
    pub file_rate: Option<u32>,
    /// `buffer` holds the file's own frames at `file_rate`, not converted: a zone whose loop is
    /// no whole number of frames at the project's rate ([`FileLoop::reads_file_rate`]); low-passed
    /// below half the project's rate, frame for frame, when the file's is above it.
    pub at_file_rate: bool,
    pub notices: Vec<SampleNotice>,
}

/// Loads `sample`, relative to `dir` unless absolute, for a project at `project_rate`: mono or
/// stereo (the first two channels of a file with more), converted to the project's rate when
/// it has another, so playback reads it frame for frame. An empty path is
/// a channel without a sound: silence, and nothing to tell.
pub fn load_channel_sample(dir: &Path, sample: &Path, project_rate: u32) -> ChannelSample {
    load_zone_sample(dir, sample, project_rate, None)
}

/// As [`load_channel_sample`], for an instrument zone that plays `file_loop`: a file converted
/// to the project's rate whose loop ends on its last frame is converted as the loop plays on
/// past it ([`FileLoop::after_last`]); a file whose loop is no whole number of frames at the
/// project's rate is not converted at all ([`FileLoop::reads_file_rate`]), only low-passed when
/// its rate is above the project's (`band_limit`), its loop filtered as it plays on and on.
pub fn load_zone_sample(
    dir: &Path,
    sample: &Path,
    project_rate: u32,
    file_loop: Option<FileLoop>,
) -> ChannelSample {
    if sample.as_os_str().is_empty() {
        return silence(Vec::new());
    }
    // An absolute path is kept as it is; a shipped one is read from the data folder.
    let path = crate::paths::sound_file(dir, sample);
    match decode(&path) {
        Ok(decoded) => at_project_rate(&path, decoded, project_rate, file_loop),
        Err(error) => silence(vec![SampleNotice::Unreadable {
            path,
            reason: error.to_string(),
        }]),
    }
}

/// `decoded`, from the file at `path`, as a project at `project_rate` plays it, for a zone that
/// plays `file_loop` (or none).
pub(crate) fn at_project_rate(
    path: &Path,
    decoded: Decoded,
    project_rate: u32,
    file_loop: Option<FileLoop>,
) -> ChannelSample {
    let mut notices = Vec::new();
    let frames = decoded.frames() as u64;
    let after_last = file_loop
        .and_then(|file_loop| file_loop.after_last(frames))
        .and_then(|start| usize::try_from(start).ok());
    let file_rate = decoded.sample_rate;
    let at_file_rate = file_loop
        .is_some_and(|file_loop| file_loop.reads_file_rate(frames, file_rate, project_rate));
    let channels = decoded.channels.max(1);
    let kept = channels.min(2);
    let mut samples = if channels <= 2 {
        decoded.samples
    } else {
        notices.push(SampleNotice::ExtraChannels {
            path: path.to_path_buf(),
            channels,
        });
        decoded
            .samples
            .chunks_exact(channels)
            .flat_map(|frame| frame[..kept].iter().copied())
            .collect()
    };
    if at_file_rate {
        // Read at file_rate / project_rate times its speed, what the file holds above half the
        // project's rate would fold down; the loop's frames, as its zone plays them.
        let looped = file_loop
            .and_then(|file_loop| file_loop.kept(frames))
            .and_then(|(start, end)| {
                Some((usize::try_from(start).ok()?, usize::try_from(end).ok()?))
            });
        band_limit(&mut samples, kept, file_rate, project_rate, looped);
    } else if file_rate != project_rate {
        match resample(&samples, kept, file_rate, project_rate, after_last) {
            Ok(resampled) => samples = resampled,
            Err(error) => notices.push(SampleNotice::WrongRate {
                path: path.to_path_buf(),
                file_rate,
                project_rate,
                reason: error.to_string(),
            }),
        }
    }
    let buffer = if kept == 1 {
        SampleBuffer::mono(samples)
    } else {
        SampleBuffer::stereo(samples)
    };
    ChannelSample {
        buffer,
        file_rate: Some(file_rate),
        at_file_rate,
        notices,
    }
}

fn silence(notices: Vec<SampleNotice>) -> ChannelSample {
    ChannelSample {
        buffer: SampleBuffer::from(Vec::new()),
        file_rate: None,
        at_file_rate: false,
        notices,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dawsome-sample-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// A 32-bit float WAV of `channels` interleaved `values` at 44.1 kHz in `dir`.
    fn wav(dir: &Path, name: &str, channels: u16, values: &[f32]) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: 44_100,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(dir.join(name), spec).expect("creates");
        for &value in values {
            writer.write_sample(value).expect("writes");
        }
        writer.finalize().expect("finalizes");
    }

    #[test]
    fn a_matching_mono_file_loads_without_a_notice() {
        let loaded = load_channel_sample(&fixtures(), Path::new("impulse.wav"), 44_100);
        assert!(loaded.notices.is_empty(), "{:?}", loaded.notices);
        assert_eq!((loaded.buffer.channels(), loaded.buffer.frames()), (1, 1));
        assert_eq!(loaded.file_rate, Some(44_100));
    }

    // Missing or unreadable means silent plus a notice, never a refused project.
    #[test]
    fn a_missing_file_is_silence_with_a_notice_and_an_empty_path_is_silence() {
        let loaded = load_channel_sample(&fixtures(), Path::new("gone.wav"), 44_100);
        assert!(matches!(
            loaded.notices.as_slice(),
            [SampleNotice::Unreadable { path, .. }] if path.ends_with("gone.wav")
        ));
        let empty = load_channel_sample(&fixtures(), Path::new(""), 44_100);
        assert!(empty.notices.is_empty());
    }

    // A stereo file plays stereo, and a file with three channels keeps the first two, with a
    // notice.
    #[test]
    fn a_stereo_file_plays_stereo_and_a_third_channel_is_dropped() {
        let dir = temp_dir();
        wav(&dir, "pair.wav", 2, &[1.0, 0.0, 0.25, -0.5]);
        let loaded = load_channel_sample(&dir, Path::new("pair.wav"), 44_100);
        assert!(loaded.notices.is_empty(), "{:?}", loaded.notices);
        assert_eq!(loaded.buffer.channels(), 2);
        assert_eq!(loaded.buffer.frame(1), Some([0.25, -0.5]));
        wav(&dir, "three.wav", 3, &[1.0, 0.5, 9.0, -1.0, -0.5, 9.0]);
        let loaded = load_channel_sample(&dir, Path::new("three.wav"), 44_100);
        assert!(matches!(
            loaded.notices.as_slice(),
            [SampleNotice::ExtraChannels { channels: 3, .. }]
        ));
        assert_eq!(loaded.buffer.samples(), [1.0, 0.5, -1.0, -0.5]);
    }

    // A 44.1 kHz zone looping 300 frames (326.53 at 48 kHz, no whole number) keeps its file's
    // own 4000 frames, bit for bit, at 48 kHz; looping 147 (160 at 48 kHz) or not at all, or at
    // 44.1 kHz, it is converted as any sound: ceil(4000 · 48000 / 44100) = 4354 frames.
    #[test]
    fn a_zone_whose_loop_is_not_whole_at_the_project_rate_keeps_its_files_frames() {
        let dir = temp_dir();
        let values: Vec<f32> = (0..4000).map(|i| (i as f32 * 0.001).sin()).collect();
        wav(&dir, "loop.wav", 1, &values);
        let file_loop = |sfz: &str| {
            let read = dawsome_engine::sfz::parse("t.sfz", sfz, &mut |_| Err(String::new()));
            FileLoop::of(&read.regions[0])
        };
        let slip = file_loop("<region> sample=loop.wav loop_start=2000 loop_end=2299");
        let loaded = load_zone_sample(&dir, Path::new("loop.wav"), 48_000, slip);
        assert!(loaded.notices.is_empty(), "{:?}", loaded.notices);
        assert!(loaded.at_file_rate);
        assert_eq!(loaded.file_rate, Some(44_100));
        assert_eq!(loaded.buffer.samples(), values.as_slice());
        let whole = file_loop("<region> sample=loop.wav loop_start=2000 loop_end=2146");
        for (rate, file_loop, frames) in [
            (48_000, whole, 4354),
            (48_000, None, 4354),
            (44_100, slip, 4000),
        ] {
            let loaded = load_zone_sample(&dir, Path::new("loop.wav"), rate, file_loop);
            assert!(!loaded.at_file_rate, "{rate} {file_loop:?}");
            assert_eq!(loaded.buffer.frames(), frames, "{rate} {file_loop:?}");
        }
    }

    // A file at another rate is converted on load, to exactly
    // ceil(24000 · 44100 / 48000) = 22050 frames, with nothing to tell.
    #[test]
    fn a_file_at_another_rate_loads_at_the_project_rate() {
        let loaded = load_channel_sample(&fixtures(), Path::new("sine1k_48k.wav"), 44_100);
        assert!(loaded.notices.is_empty(), "{:?}", loaded.notices);
        assert_eq!(loaded.buffer.frames(), 22_050);
        assert_eq!(loaded.file_rate, Some(48_000));
    }
}
