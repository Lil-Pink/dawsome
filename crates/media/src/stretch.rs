//! Stretched audio clips. A clip's whole file is made exactly `frames` frames long, by reading
//! it faster or slower (resampling: longer is lower) or by time-stretching with Signalsmith
//! Stretch (the pitch kept, or shifted by whole semitones). It is rendered ahead of playback,
//! on a worker or before an offline render starts, into the cache, and played from there as an
//! ordinary file: playback never stretches, so renders stay exact and block-invariant.
//!
//! A clip with warp markers ([`dawsome_engine::warp`]) stretches each part between two markers
//! by its own ratio, still to exactly `frames` frames: resampling reads each part at its own
//! speed, and time-stretching feeds one library instance the parts in turn.
//!
//! The cache file is named by everything that makes it what it is (the file, its size and time,
//! the rate, the algorithm, the stretch and its markers), so deleting the cache is always safe. Why renders
//! read the cached file rather than stretching again: above a ratio of 2 the library chooses
//! random phases from a seed it does not let us set, so two runs differ (inaudibly); once
//! cached, a project's renders are stable until the clip is stretched anew.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use dawsome_engine::dsp::sampler::{interpolate, SampleBuffer};
use dawsome_engine::project::{ClipStretch, StretchMode};
use dawsome_engine::warp::{self, WarpMarker};
use sha2::{Digest, Sha256};
use signalsmith_stretch::Stretch;
use thiserror::Error;

use crate::edited::Edited;
use crate::sample::{load_channel_sample, SampleNotice};
use crate::sounds::write_float_wav;

/// What the cache name starts with: a change to how files are stretched bumps it.
const CACHE_VERSION: &str = "stretch-v1";

/// The algorithms, as cache names know them.
pub const RESAMPLE_ID: &str = "resample-hermite-1";
pub const STRETCH_ID: &str = "signalsmith-stretch 0.1.3 (1.3.1) preset_default";

/// Why a clip's file could not be stretched.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StretchError {
    /// The library needs at least twice its output latency of frames to stretch into.
    #[error("too short to stretch: {frames} frames, at least {min} needed")]
    TooShort { frames: u64, min: u64 },
    #[error(
        "cannot stretch {source_frames} frames to {frames}: a clip stretches to between a quarter \
         and four times its file's length"
    )]
    Ratio { source_frames: u64, frames: u64 },
    /// The markers do not fit the sound as it is (its length is known only here and to the
    /// gesture): the last part's spacing or ratio.
    #[error("the warp markers do not fit the sound's {source_frames} frames: {reason}")]
    Warp {
        source_frames: u64,
        reason: &'static str,
    },
}

/// `source` stretched as `stretch` says through `markers` (none: one ratio), at `rate`: exactly
/// `stretch.frames` frames.
pub fn render(
    source: &SampleBuffer,
    rate: u32,
    stretch: ClipStretch,
    markers: &[WarpMarker],
) -> Result<SampleBuffer, StretchError> {
    let source_frames = source.frames();
    if !stretch.fits(source_frames) {
        return Err(StretchError::Ratio {
            source_frames,
            frames: stretch.frames,
        });
    }
    if !markers.is_empty() {
        if let Some(reason) = warp::problem(markers, source_frames, stretch.frames) {
            return Err(StretchError::Warp {
                source_frames,
                reason,
            });
        }
    }
    match stretch.mode {
        StretchMode::Resample => Ok(resample(source, stretch.frames, markers)),
        StretchMode::Stretch => time_stretch(source, rate, stretch.frames, stretch.pitch, markers),
    }
}

/// The fewest frames a time-stretch of a `channels`-channel file at `rate` can produce: twice
/// the library's output latency. A gesture asking for fewer is refused.
pub fn shortest_stretch(channels: u8, rate: u32) -> u64 {
    let latency = Stretch::preset_default(u32::from(channels.max(1)), rate).output_latency();
    2 * latency as u64
}

/// Output frame j of the part from anchor (s0, o0) to (s1, o1) reads the source at exactly
/// s0 + (j − o0)·(s1 − s0)/(o1 − o0): with no markers the one part (0, 0) to (S, T), so j·S/T.
/// The frame is the whole part and the fraction the rest over (o1 − o0) in f64, between frames
/// by the sampler's 4-point [`interpolate`] (the frames before the first and past the end
/// counting 0, read across the parts' edges from the whole sound), on a frame that frame: the
/// read every pitched voice makes, so a resampled clip is read as a note played off its root is.
fn resample(source: &SampleBuffer, frames: u64, markers: &[WarpMarker]) -> SampleBuffer {
    let (total, channels) = (source.frames(), source.channels());
    let at = |frame: Option<u64>| {
        frame
            .and_then(|frame| source.frame(frame))
            .unwrap_or([0.0; 2])
    };
    let mut out = Vec::with_capacity(usize::try_from(frames).unwrap_or(0) * usize::from(channels));
    for (s0, o0, s1, o1) in warp::segments(markers, total, frames) {
        // `render` checked the parts: each rises in both columns.
        let (span, width) = (u128::from(s1 - s0), u128::from(o1 - o0));
        for j in o0..o1 {
            // j − o0 < T and S fit u64, so the product fits u128.
            let position = u128::from(j - o0) * span;
            let (whole, rest) = (position / width, position % width);
            let frame = u64::try_from(whole)
                .ok()
                .and_then(|whole| s0.checked_add(whole))
                .unwrap_or(u64::MAX);
            let current = at(Some(frame));
            let value = if rest == 0 {
                current
            } else {
                let points = [
                    at(frame.checked_sub(1)),
                    current,
                    at(frame.checked_add(1)),
                    at(frame.checked_add(2)),
                ];
                interpolate(points, rest as f64 / width as f64)
            };
            out.extend_from_slice(&value[..usize::from(channels)]);
        }
    }
    if channels == 1 {
        SampleBuffer::mono(out)
    } else {
        SampleBuffer::stereo(out)
    }
}

/// `source` time-stretched to `frames` frames and shifted `pitch` semitones: the library's
/// default preset for the rate, given the whole file at once, or the parts between `markers` in
/// turn ([`through_markers`]).
fn time_stretch(
    source: &SampleBuffer,
    rate: u32,
    frames: u64,
    pitch: i8,
    markers: &[WarpMarker],
) -> Result<SampleBuffer, StretchError> {
    let channels = source.channels();
    let mut stretch = Stretch::preset_default(u32::from(channels), rate);
    let min = 2 * stretch.output_latency() as u64;
    let too_short = StretchError::TooShort { frames, min };
    if frames < min {
        return Err(too_short);
    }
    stretch.set_transpose_factor_semitones(f32::from(pitch), None);
    let length = usize::try_from(frames)
        .ok()
        .and_then(|frames| frames.checked_mul(usize::from(channels)))
        .ok_or(too_short.clone())?;
    let mut out = vec![0.0; length];
    // Why no markers keeps the library's own call: a clip stretched before markers existed
    // keeps its bits (and its cached file stays the one it plays).
    if markers.is_empty() {
        if !stretch.exact(source.samples(), &mut out) {
            return Err(too_short);
        }
    } else {
        through_markers(&mut stretch, source, frames, markers, &mut out);
    }
    Ok(if channels == 1 {
        SampleBuffer::mono(out)
    } else {
        SampleBuffer::stereo(out)
    })
}

/// The library's whole-file call taken apart so each part between markers goes in at its own
/// ratio: pre-roll of the sound's first input-latency frames, then the sound read that far
/// ahead (silence past its end), each part's frames in and its stretched frames out as one
/// call of the running stream; then, as the whole-file call does, the first output-latency
/// frames folded back onto the next, everything moved that far earlier, and the last ones
/// flushed out. `out` (`frames` frames, at least twice the output latency) ends exactly full.
fn through_markers(
    stretch: &mut Stretch,
    source: &SampleBuffer,
    frames: u64,
    markers: &[WarpMarker],
    out: &mut [f32],
) {
    let width = usize::from(source.channels());
    let samples = source.samples();
    let index = |frame: u64| {
        usize::try_from(frame)
            .unwrap_or(usize::MAX)
            .saturating_mul(width)
    };
    let ahead_by = index(stretch.input_latency() as u64).min(samples.len());
    let mut ahead = vec![0.0; samples.len()];
    ahead[..samples.len() - ahead_by].copy_from_slice(&samples[ahead_by..]);
    let parts = || warp::segments(markers, source.frames(), frames);
    // The pre-roll's speed, as the whole-file call gives its one (in f32, as it computes it).
    let first = parts().next().map_or(1.0, |(s0, o0, s1, o1)| {
        f64::from((s1 - s0) as f32 / (o1 - o0) as f32)
    });
    stretch.seek(&samples[..ahead_by], first);
    for (s0, o0, s1, o1) in parts() {
        stretch.process(&ahead[index(s0)..index(s1)], &mut out[index(o0)..index(o1)]);
    }
    let latency = stretch.output_latency();
    let total = index(frames) / width.max(1);
    for side in 0..width {
        for i in 0..(total - latency).min(latency) {
            out[(i + latency) * width + side] -= out[(latency - 1 - i) * width + side];
        }
    }
    out.copy_within(latency * width.., 0);
    stretch.flush(&mut out[(total - latency) * width..]);
}

/// The cache file's name for `stretch` of the file at `file`, `channels` and `source_frames`
/// frames once loaded at `rate` (and edited, when `edits` gives the list's digest), through
/// `markers`: the first 32 hex digits of the SHA-256 of
/// `stretch-v1|<algorithm>|<canonical path>|<size>|<mtime ns>|<rate>|<channels>|<S>|<mode>|<T>|<pitch>`,
/// then `|edits:<digest>` only for an edited sound and `|warp:<digest>` only for a clip with
/// markers ([`warp_digest`]), so a clip without either keeps its name (and its cached stretch,
/// whose random phases above a ratio of 2 would not repeat), and `CACHE_VERSION` stays.
pub fn cache_name(
    file: &Path,
    rate: u32,
    channels: u8,
    source_frames: u64,
    stretch: ClipStretch,
    edits: Option<&str>,
    markers: &[WarpMarker],
) -> String {
    let path = fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let metadata = fs::metadata(&path).ok();
    let size = metadata.as_ref().map_or(0, fs::Metadata::len);
    let modified = metadata
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos());
    let (algorithm, mode) = match stretch.mode {
        StretchMode::Resample => (RESAMPLE_ID, "resample"),
        StretchMode::Stretch => (STRETCH_ID, "stretch"),
    };
    let mut key = format!(
        "{CACHE_VERSION}|{algorithm}|{}|{size}|{modified}|{rate}|{channels}|{source_frames}|{mode}|{}|{}",
        path.display(),
        stretch.frames,
        stretch.pitch
    );
    if let Some(digest) = edits {
        key.push_str("|edits:");
        key.push_str(digest);
    }
    if !markers.is_empty() {
        key.push_str("|warp:");
        key.push_str(&warp_digest(markers));
    }
    let digest = Sha256::digest(key.as_bytes());
    let hex: String = digest
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{hex}.wav")
}

/// The hex SHA-256 of a marker list written `<source>:<out>` per marker, joined by commas.
pub fn warp_digest(markers: &[WarpMarker]) -> String {
    let text = markers
        .iter()
        .map(|marker| format!("{}:{}", marker.source, marker.out))
        .collect::<Vec<_>>()
        .join(",");
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A stretched file ready to play: in memory, or (past the streaming threshold) in the cache,
/// to stream.
#[derive(Debug, Clone)]
pub enum Stretched {
    Memory(SampleBuffer),
    Cached(PathBuf),
}

/// What making a stretched file gave: it, or why not, and what loading the file had to say.
#[derive(Debug, Clone)]
pub struct StretchOutcome {
    pub result: Result<Stretched, StretchError>,
    pub notices: Vec<SampleNotice>,
}

/// The edited sound a stretch is made from in place of its file, with its list's digest
/// ([`crate::edited::digest`]).
#[derive(Debug, Clone)]
pub struct EditedSource {
    pub digest: String,
    pub sound: Edited,
}

/// One stretch to make: the file (absolute, or relative to the working directory), at the
/// project's `rate`, through the clip's warp markers, the cache folder (none: memory only), the
/// frames above which the result streams from the cache rather than loading, and the edited
/// sound it stretches (none: the file itself).
#[derive(Debug, Clone)]
pub struct StretchJob {
    pub file: PathBuf,
    pub rate: u32,
    pub stretch: ClipStretch,
    pub warp: Vec<WarpMarker>,
    pub cache: Option<PathBuf>,
    pub stream_threshold: u64,
    pub edited: Option<EditedSource>,
}

impl StretchJob {
    /// Finds the stretched file in the cache, or makes it (writing it there by way of a
    /// temporary file, so a half-written one is never read). A long result is handed back by
    /// its cached path to stream; a short one, or one the cache could not take, in memory.
    pub fn run(&self) -> StretchOutcome {
        let (source, notices) = match &self.edited {
            Some(edited) => edited.sound.buffer(self.rate),
            None => {
                let loaded = load_channel_sample(Path::new(""), &self.file, self.rate);
                (loaded.buffer, loaded.notices)
            }
        };
        let name = cache_name(
            &self.file,
            self.rate,
            source.channels(),
            source.frames(),
            self.stretch,
            self.edited.as_ref().map(|edited| edited.digest.as_str()),
            &self.warp,
        );
        let cached = self.cache.as_ref().map(|dir| dir.join(name));
        let streams = self.stretch.frames > self.stream_threshold;
        if let Some(path) = cached.as_ref().filter(|path| path.is_file()) {
            let result = if streams {
                Ok(Stretched::Cached(path.clone()))
            } else {
                Ok(Stretched::Memory(
                    load_channel_sample(Path::new(""), path, self.rate).buffer,
                ))
            };
            return StretchOutcome { result, notices };
        }
        let result = render(&source, self.rate, self.stretch, &self.warp).map(|buffer| {
            // A failed write leaves no file, so the next ask stretches again.
            let written = cached
                .as_ref()
                .filter(|path| write_float_wav(&buffer, self.rate, path).is_ok());
            match written {
                Some(path) if streams => Stretched::Cached(path.clone()),
                _ => Stretched::Memory(buffer),
            }
        });
        StretchOutcome { result, notices }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stretch(mode: StretchMode, frames: u64, pitch: i8) -> ClipStretch {
        ClipStretch {
            mode,
            frames,
            pitch,
        }
    }

    // The resample mode by hand on four frames 1, 0, 0, 0 made six: j·4/6 reads 0, 2/3, 4/3, 2,
    // 8/3, 10/3. The 4-point Hermite of the points (0, 1, 0, 0) at f is 1 − 2.5f² + 1.5f³ (1/3 at
    // f = 2/3), of (1, 0, 0, 0) f(−0.5 + f − 0.5f²) (−2/27 at f = 1/3: the impulse rings once
    // below 0, as a pitched voice's does), then 0 from frame 2 on.
    #[test]
    fn resampling_reads_the_exact_position() {
        let source = SampleBuffer::mono(vec![1.0, 0.0, 0.0, 0.0]);
        let out = render(&source, 44_100, stretch(StretchMode::Resample, 6, 0), &[]).expect("fits");
        // The fractions the mode takes: (4 mod 6)/6 and (8 mod 6)/6.
        let (two, one) = (4.0f64 / 6.0, 2.0f64 / 6.0);
        let near = ((1.5 * two - 2.5) * two * two + 1.0) as f32;
        let ring = (((-0.5 * one + 1.0) * one - 0.5) * one) as f32;
        assert_eq!(out.samples(), [1.0, near, ring, 0.0, 0.0, 0.0]);
        assert!((near - 1.0 / 3.0).abs() < 1e-7 && (ring + 2.0 / 27.0).abs() < 1e-7);
        // Stereo reads each side; half way it is (−y0 + 9y1 + 9y2 − y3)/16, a point before the
        // first frame or past the last counting 0: 13.5/16, −4.5/16, then 3.5/16, 5.5/16.
        let pair = SampleBuffer::stereo(vec![1.0, -1.0, 0.5, 0.5]);
        let out = render(&pair, 44_100, stretch(StretchMode::Resample, 4, 0), &[]).expect("fits");
        assert_eq!(
            out.samples(),
            [1.0, -1.0, 0.843_75, -0.281_25, 0.5, 0.5, 0.218_75, 0.343_75]
        );
    }

    // The whole-file call taken apart, given the one part (0, 0) to (S, T), is the whole-file
    // call: what shows the parts are fed as it feeds them.
    #[test]
    fn the_parts_fed_in_turn_with_one_part_are_the_whole_file_call() {
        let source = SampleBuffer::stereo(
            (0..40_000)
                .flat_map(|k| {
                    [
                        ((k * 7919) % 4096) as f32 / 8192.0,
                        (k % 300) as f32 / 600.0,
                    ]
                })
                .collect(),
        );
        for frames in [30_000_u64, 52_000] {
            let mut whole = Stretch::preset_default(2, 48_000);
            let mut expected = vec![0.0; frames as usize * 2];
            assert!(whole.exact(source.samples(), &mut expected));
            let mut parts = Stretch::preset_default(2, 48_000);
            let mut out = vec![0.0; frames as usize * 2];
            through_markers(&mut parts, &source, frames, &[], &mut out);
            assert_eq!(out, expected, "{frames}");
        }
    }

    // Without markers the name is the hash of the key the documentation gives, as before markers
    // existed; with them, another name.
    #[test]
    fn only_a_clip_with_markers_gets_another_cache_name() {
        let file = Path::new("no/such/file.wav");
        let plain = stretch(StretchMode::Resample, 36_000, 0);
        let key =
            format!("stretch-v1|{RESAMPLE_ID}|no/such/file.wav|0|0|48000|1|24000|resample|36000|0");
        let hex: String = Sha256::digest(key.as_bytes())
            .iter()
            .take(16)
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(
            cache_name(file, 48_000, 1, 24_000, plain, None, &[]),
            hex + ".wav"
        );
        let marked = [WarpMarker {
            source: 12_000,
            out: 24_000,
        }];
        let warped = cache_name(file, 48_000, 1, 24_000, plain, None, &marked);
        assert_ne!(
            warped,
            cache_name(file, 48_000, 1, 24_000, plain, None, &[])
        );
        let moved = [WarpMarker {
            source: 12_000,
            out: 24_001,
        }];
        assert_ne!(
            warped,
            cache_name(file, 48_000, 1, 24_000, plain, None, &moved)
        );
    }

    #[test]
    fn a_ratio_past_four_or_a_stretch_too_short_is_refused() {
        let source = SampleBuffer::mono(vec![0.5; 4410]);
        assert!(matches!(
            render(
                &source,
                44_100,
                stretch(StretchMode::Resample, 5 * 4410, 0),
                &[]
            ),
            Err(StretchError::Ratio { .. })
        ));
        let min = shortest_stretch(1, 44_100);
        assert!(min > 1000, "{min}");
        assert!(matches!(
            render(
                &source,
                44_100,
                stretch(StretchMode::Stretch, min - 1, 0),
                &[]
            ),
            Err(StretchError::TooShort { .. })
        ));
        let long = SampleBuffer::mono(vec![0.25; 44_100]);
        let out =
            render(&long, 44_100, stretch(StretchMode::Stretch, 66_150, 0), &[]).expect("fits");
        assert_eq!(out.frames(), 66_150);
    }
}
