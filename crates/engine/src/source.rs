//! Audio clip sources: what the player reads an audio clip's frames from. A source is created on
//! the UI thread (by the media layer, or from a decoded buffer), moves to the audio thread with a
//! plan update, and comes back on the retire ring to be dropped where it was made. A source in
//! memory is always ready; one that streams from disk may not be, and then its clip is silent
//! for that block, which the player counts.

use crate::dsp::sampler::SampleBuffer;

/// Index of an audio clip's source in the player's source table. The compiler hands slots out
/// and takes one back only once the update that removed its source has returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceSlot(pub(crate) u16);

impl SourceSlot {
    pub(crate) fn index(self) -> usize {
        usize::from(self.0)
    }
}

/// Whether a read delivered the frames it was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRead {
    Ready,
    /// The frames were not there in time (a stream still filling): the output is silence.
    NotReady,
    /// The source stopped for good on this read (a stream whose reader failed): the output is
    /// silence, and every later read gives silence as `Ready`, so the failure is told once.
    Failed,
}

/// The frames of one audio clip's file, at the project's rate. Addressed by frame, so a wrap or
/// a seek reads by position.
///
/// The contract: `read` and `prepare` do not allocate, free, lock or log; they run on the audio
/// thread. Everything they need is made when the source is.
pub trait ClipSource: Send {
    /// How many frames the file holds.
    fn frames(&self) -> u64;

    /// 1 for mono, 2 for stereo.
    fn channels(&self) -> u8;

    /// Writes frames `frame..frame + out.len()` of the file into `out`, (left, right) each,
    /// a mono frame on both sides, and 0.0 past the file's end. On
    /// [`NotReady`](SourceRead::NotReady) and [`Failed`](SourceRead::Failed), `out` is all zeros.
    fn read(&mut self, frame: u64, out: &mut [[f32; 2]]) -> SourceRead;

    /// Gets ready to read from `frame` next, while playback is stopped: a stream seeks there so
    /// the next Play starts warm. A source in memory has nothing to do.
    fn prepare(&mut self, _frame: u64) {}
}

/// A clip's file decoded into memory, shared with every other clip of the file. Always ready.
#[derive(Debug, Clone)]
pub struct MemorySource {
    buffer: SampleBuffer,
}

impl MemorySource {
    pub fn new(buffer: SampleBuffer) -> Self {
        Self { buffer }
    }
}

impl ClipSource for MemorySource {
    fn frames(&self) -> u64 {
        self.buffer.frames()
    }

    fn channels(&self) -> u8 {
        self.buffer.channels()
    }

    fn read(&mut self, frame: u64, out: &mut [[f32; 2]]) -> SourceRead {
        for (at, slot) in (frame..).zip(out.iter_mut()) {
            *slot = self.buffer.frame(at).unwrap_or([0.0; 2]);
        }
        SourceRead::Ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_memory_source_reads_by_frame_and_is_silent_past_the_end() {
        let mut mono = MemorySource::new(SampleBuffer::mono(vec![1.0, 0.5, 0.25]));
        let mut out = [[9.0; 2]; 3];
        assert_eq!(mono.read(1, &mut out), SourceRead::Ready);
        assert_eq!(out, [[0.5; 2], [0.25; 2], [0.0; 2]]);
        let mut stereo = MemorySource::new(SampleBuffer::stereo(vec![1.0, -1.0, 0.5, -0.5]));
        assert_eq!((stereo.frames(), stereo.channels()), (2, 2));
        stereo.read(0, &mut out);
        assert_eq!(out, [[1.0, -1.0], [0.5, -0.5], [0.0; 2]]);
    }
}
