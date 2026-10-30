//! A long audio clip streamed from disk: a reader thread per stream decodes ahead into a ring
//! of blocks, and the audio thread reads from it without waiting. An offline render instead
//! waits for each block, so a streamed render has the bytes of one from memory.

use std::path::Path;
use std::thread;
use std::time::Duration;

use creek::read::ReadError;
use creek::{ReadDiskStream, ReadStreamOptions, SeekMode};
use dawsome_engine::source::{ClipSource, SourceRead};
use symphonia::core::errors::Error as SymphoniaError;
use thiserror::Error;

mod decoder;

use decoder::FrameDecoder;

/// Frames in one block the reader thread decodes. Why this size: a block of 8192 frames is
/// 0.19 s at 44.1 kHz, so the eight blocks read ahead hold 1.5 s against a slow disk, and each
/// stream's buffers stay under a megabyte.
const BLOCK_FRAMES: usize = 8192;

/// Blocks a stream reads ahead of its playhead.
const LOOK_AHEAD_BLOCKS: usize = 8;

/// Blocks each cache holds from where it starts: a seek that lands inside them plays at once.
const CACHE_BLOCKS: usize = 2;

/// The caches: one at the clip's first frame, one where the loop starts inside the clip.
const CACHES: usize = 2;

/// How long an offline render waits before it asks again when the channel to the reader thread
/// is full.
const FULL_CHANNEL_WAIT: Duration = Duration::from_millis(1);

/// Why a stream could not be opened.
#[derive(Debug, Error)]
pub enum StreamError {
    #[error("{0}")]
    Open(String),
}

/// Whether a stream may wait for its blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiting {
    /// Live playback: a block that is not there yet plays silence (an underrun).
    Never,
    /// Offline renders: every read waits until its frames are there.
    UntilReady,
}

/// One clip's file read from disk at the project's rate (the file is at that rate already: a
/// file at another rate is converted first).
pub struct StreamingSource {
    stream: ReadDiskStream<FrameDecoder>,
    frames: u64,
    /// The channels the file has; the first two are read.
    channels: usize,
    waiting: Waiting,
    /// The stream failed for good: it reads silence from then on. The error is kept rather
    /// than dropped, so nothing is freed on the audio thread.
    failed: Option<ReadError<SymphoniaError>>,
    /// The read that found the failure said so.
    failure_told: bool,
}

impl StreamingSource {
    /// Opens `path` (starting its reader thread) to read from `first_frame` on, with a second
    /// cache at `loop_frame` when the loop starts inside the clip. Runs on the UI thread.
    pub fn open(
        path: &Path,
        first_frame: u64,
        loop_frame: Option<u64>,
        waiting: Waiting,
    ) -> Result<Self, StreamError> {
        let options = ReadStreamOptions {
            num_cache_blocks: CACHE_BLOCKS,
            num_caches: CACHES,
            additional_opts: (),
            num_look_ahead_blocks: LOOK_AHEAD_BLOCKS,
            block_size: BLOCK_FRAMES,
            server_msg_channel_size: None,
        };
        let start = usize::try_from(first_frame).unwrap_or(usize::MAX);
        let mut stream = ReadDiskStream::<FrameDecoder>::new(path, start, options)
            .map_err(|error| StreamError::Open(error.to_string()))?;
        let info = stream.info();
        let (frames, channels) = (info.num_frames as u64, usize::from(info.num_channels));
        let cache = |stream: &mut ReadDiskStream<FrameDecoder>, index, frame: u64| {
            let frame = usize::try_from(frame).unwrap_or(usize::MAX);
            stream
                .cache(index, frame)
                .map(drop)
                .map_err(|error| StreamError::Open(error.to_string()))
        };
        cache(&mut stream, 0, first_frame)?;
        if let Some(frame) = loop_frame {
            cache(&mut stream, 1, frame)?;
        }
        stream
            .seek(start, SeekMode::TryOne(0))
            .map_err(|error| StreamError::Open(error.to_string()))?;
        Ok(Self {
            stream,
            frames,
            channels: channels.max(1),
            waiting,
            failed: None,
            failure_told: false,
        })
    }

    /// Puts the playhead on `frame` when it is not there.
    fn seek_to(&mut self, frame: u64) -> Result<(), ReadError<SymphoniaError>> {
        let frame = usize::try_from(frame).unwrap_or(usize::MAX);
        if self.stream.playhead() != frame {
            self.stream.seek(frame, SeekMode::Auto)?;
        }
        Ok(())
    }

    /// Reads `out.len()` frames from `frame` on into `out`; whether they were there in time.
    fn try_read(
        &mut self,
        frame: u64,
        out: &mut [[f32; 2]],
    ) -> Result<bool, ReadError<SymphoniaError>> {
        self.seek_to(frame)?;
        let ready = match self.waiting {
            Waiting::UntilReady => self.stream.block_until_ready().map(|()| true)?,
            Waiting::Never => self.stream.is_ready()?,
        };
        // Read even while it fills, so the playhead keeps up with the clip and the frames line
        // up by position once the blocks are in; what a missing block gives is silence.
        self.read_frames(out)?;
        Ok(ready)
    }

    /// Reads `out.len()` frames from the playhead on into `out`: a mono file on both sides,
    /// silence past the end of the file.
    fn read_frames(&mut self, out: &mut [[f32; 2]]) -> Result<(), ReadError<SymphoniaError>> {
        let mut done = 0;
        while done < out.len() {
            if self.stream.playhead() as u64 >= self.frames {
                out[done..].fill([0.0; 2]);
                return Ok(());
            }
            let data = self.stream.read(out.len() - done)?;
            let read = data.num_frames();
            let left = data.read_channel(0);
            let right = data.read_channel(if self.channels > 1 { 1 } else { 0 });
            for ((slot, &l), &r) in out[done..done + read].iter_mut().zip(left).zip(right) {
                *slot = [l, r];
            }
            done += read;
            if read == 0 {
                out[done..].fill([0.0; 2]);
                return Ok(());
            }
        }
        Ok(())
    }
}

impl ClipSource for StreamingSource {
    fn frames(&self) -> u64 {
        self.frames
    }

    fn channels(&self) -> u8 {
        if self.channels > 1 {
            2
        } else {
            1
        }
    }

    /// Only a fatal error stops the stream. Why every other one is a pause: creek calls them
    /// temporary (a full channel to the reader thread empties as it catches up) or the playhead
    /// did not move, so the next read asks again.
    fn read(&mut self, frame: u64, out: &mut [[f32; 2]]) -> SourceRead {
        loop {
            if self.failed.is_some() {
                out.fill([0.0; 2]);
                let told = std::mem::replace(&mut self.failure_told, true);
                return if told {
                    SourceRead::Ready
                } else {
                    SourceRead::Failed
                };
            }
            if frame >= self.frames {
                out.fill([0.0; 2]);
                return SourceRead::Ready;
            }
            match self.try_read(frame, out) {
                Ok(true) => return SourceRead::Ready,
                Ok(false) => {}
                Err(error @ ReadError::FatalError(_)) => {
                    self.failed = Some(error);
                    continue;
                }
                // An offline render waits for the disk, a full channel included.
                Err(ReadError::IOServerChannelFull) if self.waiting == Waiting::UntilReady => {
                    thread::sleep(FULL_CHANNEL_WAIT);
                    continue;
                }
                Err(_) => {}
            }
            out.fill([0.0; 2]);
            return SourceRead::NotReady;
        }
    }

    fn prepare(&mut self, frame: u64) {
        if self.failed.is_none() && frame < self.frames {
            if let Err(error @ ReadError::FatalError(_)) = self.seek_to(frame) {
                self.failed = Some(error);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    /// p8_saw.wav's frames as the decoder gives them: ((k mod 1000) + 1)/1024.
    fn saw(frame: u64) -> f32 {
        ((frame % 1000) + 1) as f32 / 1024.0
    }

    // Waiting for the disk, a stream reads the file frame for frame from any position, across
    // its blocks, back and forth, and silence past its end.
    #[test]
    fn a_waiting_stream_reads_the_file_by_position() {
        let path = fixture("p8_saw.wav");
        let mut source =
            StreamingSource::open(&path, 5000, None, Waiting::UntilReady).expect("opens");
        assert_eq!((source.frames(), source.channels()), (30_000, 1));
        let mut out = vec![[0.0; 2]; 9000];
        for start in [5000, 14_000, 100, 29_000] {
            assert_eq!(source.read(start, &mut out), SourceRead::Ready);
            for (k, frame) in out.iter().enumerate() {
                let at = start + k as u64;
                let want = if at < 30_000 { saw(at) } else { 0.0 };
                assert_eq!(*frame, [want; 2], "frame {at}");
            }
        }
    }

    // A stream lands on the frame it is asked for, also where frame/44100·44100 is not exactly
    // that frame in f64 (1434, 1483, 1565, ...): opened there, and seeked there from frame 0.
    #[test]
    fn a_stream_lands_on_every_frame_it_is_asked_for() {
        let path = fixture("p8_saw.wav");
        let mut seeking =
            StreamingSource::open(&path, 0, None, Waiting::UntilReady).expect("opens");
        let mut out = vec![[0.0; 2]; 64];
        for start in (1000..3000).step_by(7) {
            let mut opened =
                StreamingSource::open(&path, start, None, Waiting::UntilReady).expect("opens");
            for (name, source) in [("opened", &mut opened), ("seeked", &mut seeking)] {
                assert_eq!(source.read(start, &mut out), SourceRead::Ready);
                for (k, frame) in out.iter().enumerate() {
                    let at = start + k as u64;
                    assert_eq!(*frame, [saw(at); 2], "{name} at {start}: frame {at}");
                }
            }
        }
    }

    /// p8_saw.wav's saw, 10 s long at 44.1 kHz: each block cached after a seek takes the
    /// reader thread a while.
    fn long_saw() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dawsome-stream-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("saw10s.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).expect("creates");
        for frame in 0..441_000u64 {
            writer
                .write_sample(((frame % 1000) as i16 + 1) * 32)
                .expect("writes");
        }
        writer.finalize().expect("finalizes");
        path
    }

    // Live reads at uncached positions, faster than the reader thread takes its messages,
    // fill the channel to it (IOServerChannelFull, which creek calls temporary). Each read is
    // then silence counted as not ready, never "ready" with wrong frames; after a pause the
    // stream plays the file again.
    #[test]
    fn a_full_message_channel_is_a_pause_not_a_failure() {
        let path = long_saw();
        let mut source = StreamingSource::open(&path, 0, None, Waiting::Never).expect("opens");
        let mut out = vec![[0.0; 2]; 512];
        let mut not_ready = 0;
        for k in 0..400u64 {
            let start = 20_000 + (k * 9973) % 400_000;
            match source.read(start, &mut out) {
                SourceRead::NotReady => not_ready += 1,
                read => {
                    let want: Vec<[f32; 2]> = (start..start + 512).map(|at| [saw(at); 2]).collect();
                    assert_eq!((read, &out), (SourceRead::Ready, &want), "read {k}");
                }
            }
        }
        assert!(not_ready > 0, "the burst outran the reader thread");
        std::thread::sleep(std::time::Duration::from_millis(100));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while source.read(0, &mut out) != SourceRead::Ready {
            assert!(std::time::Instant::now() < deadline, "the stream recovers");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let want: Vec<[f32; 2]> = (0..512).map(|at| [saw(at); 2]).collect();
        assert_eq!(out, want, "after the pause the file plays");
    }

    // A stream that failed for good says so on one read and is silent from then on.
    #[test]
    fn a_failed_stream_says_so_once() {
        let path = fixture("p8_saw.wav");
        let mut source = StreamingSource::open(&path, 0, None, Waiting::Never).expect("opens");
        source.failed = Some(ReadError::FatalError(
            creek::read::FatalReadError::StreamClosed,
        ));
        let mut out = vec![[1.0; 2]; 64];
        assert_eq!(source.read(0, &mut out), SourceRead::Failed);
        assert_eq!(out, vec![[0.0; 2]; 64]);
        out.fill([1.0; 2]);
        assert_eq!(source.read(64, &mut out), SourceRead::Ready);
        assert_eq!(out, vec![[0.0; 2]; 64]);
    }

    #[test]
    fn a_missing_file_does_not_open() {
        assert!(
            StreamingSource::open(Path::new("/nonexistent.wav"), 0, None, Waiting::Never).is_err()
        );
    }
}
