//! The decoder a stream's reader thread runs: symphonia, positioned by exact frame. Why our own
//! and not creek's: creek's seeks by a time in seconds, which lands one frame early wherever
//! frame / rate · rate does not come back to the frame in f64 (1434 of 44100, among others).
//! Every format decoded here times its packets in frames, so a seek by timestamp is exact.

use std::io;
use std::path::PathBuf;

use creek::{DataBlock, Decoder, FileInfo};
use symphonia::core::codecs::{Decoder as CodecDecoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatReader, SeekMode, SeekTo};
use symphonia::core::units::TimeBase;

use crate::decode::{audio_track, each_value, format_reader, DecodeError};

pub struct FrameDecoder {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn CodecDecoder>,
    track: u32,
    frames: u64,
    channels: usize,
    block_size: usize,
    /// The next frame to hand out.
    playhead: u64,
    /// The last packet decoded, as f32 per channel; its first frame is file frame
    /// `pending_start`.
    pending: Vec<Vec<f32>>,
    pending_start: u64,
}

impl FrameDecoder {
    fn pending_end(&self) -> u64 {
        self.pending_start + self.pending.first().map_or(0, Vec::len) as u64
    }

    /// Decodes the next packet of the track into `pending`. Returns false at the end of the
    /// file. A packet the decoder reports as corrupt is skipped, as [`crate::decode`] skips it.
    fn next_packet(&mut self) -> Result<bool, SymphoniaError> {
        loop {
            let packet = match self.reader.next_packet() {
                Ok(packet) => packet,
                // How every format reader reports the end of the file.
                Err(SymphoniaError::IoError(error))
                    if error.kind() == io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(false)
                }
                Err(error) => return Err(error),
            };
            if packet.track_id() != self.track {
                continue;
            }
            let buffer = match self.decoder.decode(&packet) {
                Ok(buffer) => buffer,
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(error) => return Err(error),
            };
            if buffer.spec().channels.count() != self.channels {
                return Err(SymphoniaError::DecodeError(
                    "the channel count changes part way",
                ));
            }
            for plane in &mut self.pending {
                plane.clear();
            }
            let pending = &mut self.pending;
            each_value(&buffer, 0..buffer.frames(), |channel, value| {
                pending[channel].push(value);
            });
            self.pending_start = packet.ts();
            return Ok(true);
        }
    }
}

impl Decoder for FrameDecoder {
    type T = f32;
    type AdditionalOpts = ();
    type FileParams = ();
    type OpenError = DecodeError;
    type FatalError = SymphoniaError;

    const DEFAULT_BLOCK_SIZE: usize = 8192;
    const DEFAULT_NUM_CACHE_BLOCKS: usize = 2;
    const DEFAULT_NUM_LOOK_AHEAD_BLOCKS: usize = 8;

    fn new(
        file: PathBuf,
        start_frame: usize,
        block_size: usize,
        _: (),
    ) -> Result<(Self, FileInfo<()>), DecodeError> {
        let reader = format_reader(&file)?;
        let track = audio_track(&*reader)?;
        let params = &track.codec_params;
        let rate = params.sample_rate.ok_or(DecodeError::NoSampleRate)?;
        // creek plans its blocks from the length, so a header that does not give it, or times
        // packets other than in frames, cannot stream (the clip loads into memory instead).
        let frames = params.n_frames.ok_or(DecodeError::NoLength)?;
        if params
            .time_base
            .is_some_and(|base| base != TimeBase::new(1, rate))
        {
            return Err(DecodeError::NotInFrames);
        }
        let channels = params.channels.map_or(1, |layout| layout.count()).max(1);
        let decoder = symphonia::default::get_codecs().make(params, &DecoderOptions::default())?;
        let info = FileInfo {
            params: (),
            num_frames: usize::try_from(frames).unwrap_or(usize::MAX),
            num_channels: u16::try_from(channels).unwrap_or(u16::MAX),
            sample_rate: Some(rate),
        };
        let mut decoder = Self {
            track: track.id,
            reader,
            decoder,
            frames,
            channels,
            block_size,
            playhead: 0,
            pending: vec![Vec::new(); channels],
            pending_start: 0,
        };
        if start_frame != 0 {
            decoder.seek(start_frame)?;
        }
        Ok((decoder, info))
    }

    fn seek(&mut self, frame: usize) -> Result<(), SymphoniaError> {
        let frame = (frame as u64).min(self.frames);
        self.playhead = frame;
        if (self.pending_start..self.pending_end()).contains(&frame) {
            return Ok(());
        }
        for plane in &mut self.pending {
            plane.clear();
        }
        self.pending_start = frame;
        if frame == self.frames {
            return Ok(());
        }
        // An accurate seek lands on the packet that holds the frame or one before it; the
        // frames before it are decoded and dropped by `decode`.
        self.reader.seek(
            SeekMode::Accurate,
            SeekTo::TimeStamp {
                ts: frame,
                track_id: self.track,
            },
        )?;
        self.decoder.reset();
        Ok(())
    }

    /// Fills `block` with the next `block_size` frames, fewer only at the end of the file. Why
    /// whole blocks even where the file falls short of its header: creek places each block
    /// `block_size` frames after the one before, so a short one would shift every later frame.
    fn decode(&mut self, block: &mut DataBlock<f32>) -> Result<(), SymphoniaError> {
        let mut filled = 0;
        while filled < self.block_size && self.playhead < self.frames {
            let room = self.block_size - filled;
            let to_end = usize::try_from(self.frames - self.playhead).unwrap_or(usize::MAX);
            if self.playhead >= self.pending_end() {
                if self.next_packet()? {
                    continue;
                }
                // The file holds fewer frames than its header says: silence up to its length.
                silence(block, room.min(to_end));
                self.playhead += room.min(to_end) as u64;
                break;
            }
            if self.playhead < self.pending_start {
                // The reader skipped past the playhead: silence keeps later frames in place.
                let gap = usize::try_from(self.pending_start - self.playhead)
                    .map_or(room, |gap| gap.min(room));
                silence(block, gap);
                filled += gap;
                self.playhead += gap as u64;
                continue;
            }
            let from = (self.playhead - self.pending_start) as usize;
            let count = room.min(to_end).min(self.pending[0].len() - from);
            for (plane, pending) in block.block.iter_mut().zip(&self.pending) {
                plane.extend_from_slice(&pending[from..from + count]);
            }
            filled += count;
            self.playhead += count as u64;
        }
        Ok(())
    }

    fn current_frame(&self) -> usize {
        usize::try_from(self.playhead).unwrap_or(usize::MAX)
    }
}

/// Appends `frames` silent frames to every channel of `block`.
fn silence(block: &mut DataBlock<f32>, frames: usize) {
    for plane in &mut block.block {
        plane.resize(plane.len() + frames, 0.0);
    }
}
