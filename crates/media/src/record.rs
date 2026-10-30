//! Recording into a sound file: frames from a feed (the master's record tap, or the input
//! device's ring) are converted to the project's rate when they come at another and written as
//! a 32-bit float WAV of the feed's channel count, by a writer thread that drains the feed every
//! 20 ms, or by the caller, block by block, in an offline render.
//!
//! Also the input channel's monitor ring: the input callback's frames for the player, which
//! reads them as the channel's sound ([`MonitorFeed`], an engine
//! [`InputFeed`](dawsome_engine::input::InputFeed)).

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

// The engine's name for what the player reads; `InputFeed` here is the recording's ring half.
use dawsome_engine::input::InputFeed as PlayerFeed;
use dawsome_engine::project::InputChannels;
use dawsome_engine::realtime::RecordTap;
use hound::{SampleFormat, WavSpec, WavWriter};
use rtrb::{Consumer, Producer, RingBuffer};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

use crate::write::WriteError;

/// How often the writer thread drains its feed.
const DRAIN_PERIOD: Duration = Duration::from_millis(20);

/// Frames the streaming resampler takes per step.
const RESAMPLE_CHUNK: usize = 1024;

/// Where a recording's frames come from, interleaved.
pub trait Feed: Send {
    fn channels(&self) -> usize;
    /// The rate the frames come at.
    fn rate(&self) -> u32;
    /// Moves every value waiting into `out`; returns how many.
    fn read_into(&mut self, out: &mut Vec<f32>) -> usize;
    /// Frames the feed had to drop so far (it was full); the writer pads as many silent ones.
    fn dropped(&self) -> u64;
    /// The record tap this feed reads, given back to the session once the recording is done.
    fn into_tap(self: Box<Self>) -> Option<RecordTap> {
        None
    }
}

/// The master's record tap as a feed: stereo, at the project's rate.
pub struct TapFeed {
    pub tap: RecordTap,
    pub rate: u32,
}

impl Feed for TapFeed {
    fn channels(&self) -> usize {
        2
    }

    fn rate(&self) -> u32 {
        self.rate
    }

    fn read_into(&mut self, out: &mut Vec<f32>) -> usize {
        self.tap.read_into(out)
    }

    fn dropped(&self) -> u64 {
        self.tap.dropped()
    }

    fn into_tap(self: Box<Self>) -> Option<RecordTap> {
        Some(self.tap)
    }
}

/// The input callback's half of an input ring: copies a device buffer in without allocating,
/// locking or logging, keeps its peak for the meter, and counts what it had no room for.
pub struct InputWriter {
    ring: Producer<f32>,
    channels: usize,
    /// The device channel each of the ring's channels takes (the first `channels` used).
    sides: [usize; 2],
    peak: Arc<AtomicU32>,
    dropped: Arc<AtomicU64>,
    /// Frames in the last device buffer pushed: the input's block, for a monitor's cushion.
    last_buffer: Arc<AtomicU64>,
}

impl InputWriter {
    fn new(ring: Producer<f32>, channels: usize, sides: [usize; 2]) -> Self {
        Self {
            ring,
            channels,
            sides,
            peak: Arc::new(AtomicU32::new(0)),
            dropped: Arc::new(AtomicU64::new(0)),
            last_buffer: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Takes the ring's channels from each frame of `samples`, a device buffer of
    /// `device_channels` interleaved channels, from frame `keep_from` of the buffer on (none
    /// when `None`), and raises the meter's peak over the channels taken in all of it. A side
    /// naming a channel the device does not have takes its first: a mono device gives its one
    /// channel to both sides of a stereo ring.
    pub fn push(&mut self, samples: &[f32], device_channels: usize, keep_from: Option<usize>) {
        let device_channels = device_channels.max(1);
        let sides = &self.sides[..self.channels];
        let mut peak = 0.0f32;
        let mut frames = 0u64;
        for (index, frame) in samples.chunks(device_channels).enumerate() {
            frames += 1;
            let value = |side: usize| frame.get(side).or_else(|| frame.first()).copied();
            for &side in sides {
                peak = peak.max(value(side).unwrap_or(0.0).abs());
            }
            if keep_from.is_none_or(|first| index < first) {
                continue;
            }
            if self.ring.slots() < self.channels {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            for &side in sides {
                let _ = self.ring.push(value(side).unwrap_or(0.0));
            }
        }
        self.peak.fetch_max(peak.to_bits(), Ordering::Relaxed);
        self.last_buffer.store(frames, Ordering::Relaxed);
    }

    /// Notes that the device's last buffer reached this ring as `frames` frames in all, pushed
    /// in parts (a resampler's blocks): the input's block, for a monitor's cushion, in place of
    /// the last part's. One atomic store. Why: a monitor that took one part for the block kept
    /// too short a cushion for the buffer's whole burst, dropped the excess and then ran dry
    /// (found in a burst simulation).
    pub fn note_buffer(&self, frames: u64) {
        self.last_buffer.store(frames, Ordering::Relaxed);
    }
}

/// The writer's half of an input ring.
pub struct InputFeed {
    ring: Consumer<f32>,
    channels: usize,
    rate: u32,
    dropped: Arc<AtomicU64>,
}

impl Feed for InputFeed {
    fn channels(&self) -> usize {
        self.channels
    }

    fn rate(&self) -> u32 {
        self.rate
    }

    /// Whole frames only: the callback pushes a frame one value at a time, so a read that took
    /// everything could end on half a frame and the writer would count one frame short (a
    /// resampled take then lost its last frame, gate 2026-10-24). The odd value waits for the
    /// next read.
    fn read_into(&mut self, out: &mut Vec<f32>) -> usize {
        let values = self.ring.slots() / self.channels * self.channels;
        out.reserve(values);
        let mut moved = 0;
        while moved < values {
            let Ok(value) = self.ring.pop() else { break };
            out.push(value);
            moved += 1;
        }
        moved
    }

    fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// A ring for the first `channels` (1 or 2) of a device's input at `rate`, holding `seconds` of
/// it, built before the input stream opens; and the peak cell its meter reads (f32 bits, taken
/// with `swap(0)`).
pub fn input_ring(
    channels: usize,
    rate: u32,
    seconds: usize,
) -> (InputWriter, InputFeed, Arc<AtomicU32>) {
    let channels = channels.clamp(1, 2);
    record_ring(channels, [0, 1], rate, seconds)
}

/// As [`input_ring`], for the device channels an input channel takes: one for a mono input, two
/// for a stereo one.
pub fn picked_ring(
    picked: InputChannels,
    rate: u32,
    seconds: usize,
) -> (InputWriter, InputFeed, Arc<AtomicU32>) {
    record_ring(picked.count(), sides(picked), rate, seconds)
}

fn sides(picked: InputChannels) -> [usize; 2] {
    picked.sides().map(usize::from)
}

fn record_ring(
    channels: usize,
    sides: [usize; 2],
    rate: u32,
    seconds: usize,
) -> (InputWriter, InputFeed, Arc<AtomicU32>) {
    let capacity = seconds.max(1) * channels * usize::try_from(rate).unwrap_or(48_000);
    let (producer, consumer) = RingBuffer::new(capacity);
    let writer = InputWriter::new(producer, channels, sides);
    let peak = Arc::clone(&writer.peak);
    let feed = InputFeed {
        ring: consumer,
        channels,
        rate,
        dropped: Arc::clone(&writer.dropped),
    };
    (writer, feed, peak)
}

/// Most input blocks a live monitor's cushion grows to after underruns.
const MOST_CUSHION_BLOCKS: u64 = 8;

/// The shortest stretch, in seconds, a live monitor watches what waits in it before its cushion
/// may shrink by an input block. Why it shrinks at all: the two clocks always drift a little,
/// so over a long session dry spells come one after another, and a cushion that only grew would
/// stay many blocks behind once the clocks matched again (found 2026-10-23).
const SHRINK_AFTER_SECONDS: u64 = 10;

/// How a monitor feed paces a live input, whose frames arrive in device buffers while the player
/// takes them in its own: `None` offline, where every frame is there before the render starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pacing {
    /// Frames in one buffer of the output stream (its block).
    pub output_block: u64,
    /// The rate the player reads at, frames a second: what a clean stretch is timed by.
    pub rate: u32,
}

/// What a live monitor feed tells the window: the frames it keeps waiting before it plays (its
/// cushion), how often it ran dry, and how many frames it dropped because its input ran ahead.
#[derive(Debug, Default)]
pub struct MonitorStats {
    pub cushion: AtomicU64,
    pub underruns: AtomicU64,
    pub skipped: AtomicU64,
}

/// The player's half of an input channel's monitor ring: the channel's sound, read block by block
/// on the audio thread (an engine [`InputFeed`](dawsome_engine::input::InputFeed)).
///
/// Live ([`Pacing`]), it plays once its ring holds a cushion of one input block and one output
/// block (the callbacks of two streams come in no fixed order, so one of each can come before
/// the other), plays silence and waits for the cushion again when it runs dry, the cushion then
/// one input block longer (up to [`MOST_CUSHION_BLOCKS`]), and drops what waits past the cushion
/// once more than two blocks over it wait (the input's clock running ahead of the output's).
/// A grown cushion is an input block shorter again, never under its base, only once what waits
/// has stayed level over a watched stretch (at least [`SHRINK_AFTER_SECONDS`], and two drift
/// steps long after a dry spell drift caused) and the block went unused since the last change
/// (or what waits sits a block under the cushion); what waits past the shorter cushion then
/// drains a frame a read. So late buffers and a drift still going keep the cushion they need,
/// and matched clocks bring it back (its `watched` says how). The input is heard the
/// cushion plus the output buffer behind, about. Offline it plays every frame as it comes and
/// silence past the last.
pub struct MonitorFeed {
    ring: Consumer<f32>,
    channels: usize,
    pacing: Option<Pacing>,
    /// The input's block, as its writer last saw it.
    input_block: Arc<AtomicU64>,
    /// Input blocks the cushion has grown by since the feed began.
    grown: u64,
    /// Frames played since the feed last primed: how long its last dry spell took to come.
    since_primed: u64,
    /// Frames each watched stretch lasts, and frames played in the current one.
    stretch: u64,
    played: u64,
    /// The fewest frames waiting before a read since the feed last ran dry or its cushion last
    /// shrank: how much of the cushion was used.
    low: usize,
    /// The fewest and the most frames waiting before a read in the current stretch, and the
    /// fewest in the one before it (none since the last change).
    stretch_low: usize,
    stretch_high: usize,
    last_low: Option<usize>,
    /// Frames still to drain, one a read, since the cushion last shrank.
    drain: u64,
    /// Waiting for the cushion to fill.
    priming: bool,
    stats: Arc<MonitorStats>,
}

impl MonitorFeed {
    /// What the window reads of it.
    pub fn stats(&self) -> Arc<MonitorStats> {
        Arc::clone(&self.stats)
    }

    /// Forgets what the stretches watched saw: the cushion changed, so what waits will too.
    fn changed(&mut self) {
        self.played = 0;
        self.low = usize::MAX;
        self.stretch_low = usize::MAX;
        self.stretch_high = 0;
        self.last_low = None;
    }

    /// A stretch watched to its end: the cushion shrinks by an input block when what waits
    /// stayed level (no lower than in the stretch before, so no drift is under way) and the
    /// block went unused (one block fewer would still have left every read whole since the
    /// last change: a late buffer dips it, so input that comes late now and then keeps its
    /// cushion), what waits above the shorter cushion then draining a frame a read; or when
    /// what waits stayed level a block or more under the cushion, whose top block holds nothing
    /// then (nothing to drain). Either way the next stretch is the shortest, the clocks having
    /// shown they match.
    fn watched(&mut self, pacing: Pacing, shortest: u64) {
        let input = self.input_block(pacing);
        let frames = |count: usize| u64::try_from(count).unwrap_or(u64::MAX);
        let level = self.last_low.is_some_and(|last| self.stretch_low >= last);
        let (cushion, _) = self.cushion(pacing);
        let unused = frames(self.low) >= pacing.output_block + input;
        let empty_top = frames(self.stretch_high) + input <= frames(cushion);
        if self.grown > 0 && level && (unused || empty_top) {
            self.grown -= 1;
            self.drain = if unused { input } else { 0 };
            self.stretch = shortest;
            self.changed();
            return;
        }
        self.last_low = Some(self.stretch_low);
        self.played = 0;
        self.stretch_low = usize::MAX;
        self.stretch_high = 0;
    }

    /// The input's block, as its writer last saw it (the output's before the first).
    fn input_block(&self, pacing: Pacing) -> u64 {
        let seen = self.input_block.load(Ordering::Relaxed);
        if seen == 0 { pacing.output_block } else { seen }.max(1)
    }

    /// Frames to wait for before playing, and frames past which waiting ones are dropped.
    fn cushion(&self, pacing: Pacing) -> (usize, usize) {
        let input = self.input_block(pacing);
        let cushion = input * (1 + self.grown) + pacing.output_block;
        let most = cushion + 2 * input.max(pacing.output_block);
        let frames = |count: u64| usize::try_from(count).unwrap_or(usize::MAX);
        (frames(cushion), frames(most))
    }
}

impl PlayerFeed for MonitorFeed {
    fn channels(&self) -> usize {
        self.channels
    }

    fn read(&mut self, out: &mut [[f32; 2]]) {
        let channels = self.channels;
        let mut waiting = self.ring.slots() / channels;
        if let Some(pacing) = self.pacing {
            let (cushion, most) = self.cushion(pacing);
            self.stats.cushion.store(cushion as u64, Ordering::Relaxed);
            if self.priming {
                if waiting < cushion {
                    out.fill([0.0; 2]);
                    return;
                }
                self.priming = false;
            }
            self.low = self.low.min(waiting);
            self.stretch_low = self.stretch_low.min(waiting);
            self.stretch_high = self.stretch_high.max(waiting);
            if waiting > most {
                let extra = waiting - cushion;
                for _ in 0..extra * channels {
                    let _ = self.ring.pop();
                }
                waiting = cushion;
                self.stats
                    .skipped
                    .fetch_add(extra as u64, Ordering::Relaxed);
            } else if self.drain > 0 && waiting > cushion {
                // Since the cushion shrank, one frame a read goes until what waits is down to
                // it: a frame left out here and there, not a jump of a block at once.
                for _ in 0..channels {
                    let _ = self.ring.pop();
                }
                waiting -= 1;
                self.drain -= 1;
                self.stats.skipped.fetch_add(1, Ordering::Relaxed);
            }
        }
        let taken = waiting.min(out.len());
        for frame in &mut out[..taken] {
            let left = self.ring.pop().unwrap_or(0.0);
            let right = if channels == 2 {
                self.ring.pop().unwrap_or(0.0)
            } else {
                left
            };
            *frame = [left, right];
        }
        out[taken..].fill([0.0; 2]);
        let Some(pacing) = self.pacing else {
            return;
        };
        let shortest = SHRINK_AFTER_SECONDS * u64::from(pacing.rate);
        if taken < out.len() {
            // What waits falls an input block at a time as a slow input clock drifts, so the
            // dry spell this one ended took about (blocks of cushion + 1) such steps: every
            // stretch watched from now on lasts at least two of them, so a drift still going
            // shows as a step down within each.
            let blocks = 1 + self.grown;
            self.stretch = shortest.max(2 * self.since_primed / blocks);
            self.priming = true;
            self.grown = (self.grown + 1).min(MOST_CUSHION_BLOCKS - 1);
            self.since_primed = 0;
            self.drain = 0;
            self.stats.underruns.fetch_add(1, Ordering::Relaxed);
            self.changed();
            return;
        }
        self.since_primed = self.since_primed.saturating_add(taken as u64);
        self.played = self.played.saturating_add(taken as u64);
        if self.played >= self.stretch.max(shortest) {
            self.watched(pacing, shortest);
        }
    }
}

/// A monitor ring for the device channels an input channel takes, holding `frames` frames,
/// paced by `pacing` (live) or not (offline): its writer for the input callback, its feed for
/// the player, and the peak cell its meter reads (f32 bits, taken with `swap(0)`).
pub fn monitor_ring(
    picked: InputChannels,
    frames: usize,
    pacing: Option<Pacing>,
) -> (InputWriter, MonitorFeed, Arc<AtomicU32>) {
    let channels = picked.count();
    let (producer, consumer) = RingBuffer::new(frames.max(1) * channels);
    let writer = InputWriter::new(producer, channels, sides(picked));
    let peak = Arc::clone(&writer.peak);
    let feed = MonitorFeed {
        ring: consumer,
        channels,
        pacing,
        input_block: Arc::clone(&writer.last_buffer),
        grown: 0,
        since_primed: 0,
        stretch: 0,
        played: 0,
        low: usize::MAX,
        stretch_low: usize::MAX,
        stretch_high: 0,
        last_low: None,
        drain: 0,
        priming: pacing.is_some(),
        stats: Arc::new(MonitorStats::default()),
    };
    (writer, feed, peak)
}

/// A feed's frames converted from its rate to the project's at a fixed ratio, as they come:
/// the resampler's delay is dropped from the start and its tail flushed at the end, so the
/// output has exactly ceil(n · out / in) frames for n frames in, frame 0 at time 0.
struct StreamResampler {
    resampler: Fft<f32>,
    channels: usize,
    /// Input not yet a whole chunk.
    pending: Vec<f32>,
    /// One step's output.
    output: Vec<f32>,
    /// Output frames still to drop: the delay.
    to_trim: usize,
    /// Input values taken so far: counted as values, since a read may end on half a frame.
    input_values: u64,
    written: u64,
    from: u32,
    to: u32,
}

impl StreamResampler {
    fn new(channels: usize, from: u32, to: u32) -> Result<Self, WriteError> {
        let resampler = Fft::<f32>::new(
            from as usize,
            to as usize,
            RESAMPLE_CHUNK,
            channels,
            FixedSync::Input,
        )
        .map_err(|error| WriteError::Resample(error.to_string()))?;
        let to_trim = resampler.output_delay();
        let output = vec![0.0; resampler.output_frames_max() * channels];
        Ok(Self {
            resampler,
            channels,
            pending: Vec::new(),
            output,
            to_trim,
            input_values: 0,
            written: 0,
            from,
            to,
        })
    }

    /// Takes `input` and hands every whole step's output to `write`.
    fn push(
        &mut self,
        input: &[f32],
        write: &mut dyn FnMut(&[f32]) -> Result<(), WriteError>,
    ) -> Result<(), WriteError> {
        self.input_values += input.len() as u64;
        self.pending.extend_from_slice(input);
        while self.pending.len() >= RESAMPLE_CHUNK * self.channels {
            self.step(write, None)?;
        }
        Ok(())
    }

    /// Flushes the tail: zeros through the resampler until the whole length is out.
    fn finish(
        &mut self,
        write: &mut dyn FnMut(&[f32]) -> Result<(), WriteError>,
    ) -> Result<(), WriteError> {
        let frames = self.input_values / self.channels as u64;
        let want = (frames * u64::from(self.to)).div_ceil(u64::from(self.from.max(1)));
        while self.written < want {
            self.pending.resize(RESAMPLE_CHUNK * self.channels, 0.0);
            self.step(write, Some(want))?;
        }
        Ok(())
    }

    /// Resamples one chunk off the front of `pending`; writes what is past the delay, no more
    /// than `limit` frames in all.
    fn step(
        &mut self,
        write: &mut dyn FnMut(&[f32]) -> Result<(), WriteError>,
        limit: Option<u64>,
    ) -> Result<(), WriteError> {
        let channels = self.channels;
        let capacity = self.output.len() / channels;
        let input = InterleavedSlice::new(
            &self.pending[..RESAMPLE_CHUNK * channels],
            channels,
            RESAMPLE_CHUNK,
        )
        .map_err(|error| WriteError::Resample(error.to_string()))?;
        let mut output = InterleavedSlice::new_mut(&mut self.output, channels, capacity)
            .map_err(|error| WriteError::Resample(error.to_string()))?;
        let (_, produced) = self
            .resampler
            .process_into_buffer(&input, &mut output, None)
            .map_err(|error| WriteError::Resample(error.to_string()))?;
        self.pending.drain(..RESAMPLE_CHUNK * channels);
        let trimmed = produced.min(self.to_trim);
        self.to_trim -= trimmed;
        let mut frames = (produced - trimmed) as u64;
        if let Some(limit) = limit {
            frames = frames.min(limit.saturating_sub(self.written));
        }
        let from = trimmed * channels;
        write(&self.output[from..from + frames as usize * channels])?;
        self.written += frames;
        Ok(())
    }
}

/// A recording file being written.
pub struct RecordingFile {
    writer: WavWriter<BufWriter<File>>,
    channels: usize,
    resampler: Option<StreamResampler>,
    /// Frames written, at the project's rate.
    frames: u64,
    /// The most frames the file takes, at the project's rate; what comes after is dropped.
    cap: Option<u64>,
}

impl RecordingFile {
    /// Creates (or truncates) `path` for `channels` channels (1 or 2) of frames at
    /// `source_rate`, written at `project_rate`.
    pub fn create(
        path: &Path,
        channels: usize,
        source_rate: u32,
        project_rate: u32,
    ) -> Result<Self, WriteError> {
        let channels = channels.clamp(1, 2);
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let spec = WavSpec {
            channels: channels as u16,
            sample_rate: project_rate,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        };
        let resampler = (source_rate != project_rate)
            .then(|| StreamResampler::new(channels, source_rate, project_rate))
            .transpose()?;
        Ok(Self {
            writer: WavWriter::create(path, spec)?,
            channels,
            resampler,
            frames: 0,
            cap: None,
        })
    }

    /// The file taking no more than `cap` frames at the project's rate (none: no end): a take
    /// of one pass of a pattern. What comes after is read and dropped.
    pub fn capped(self, cap: Option<u64>) -> Self {
        Self { cap, ..self }
    }

    /// Frames written so far, at the project's rate.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Whether the file holds the frames its cap allows.
    pub fn is_full(&self) -> bool {
        self.cap.is_some_and(|cap| self.frames >= cap)
    }

    /// Appends interleaved `samples` at the source rate.
    pub fn write(&mut self, samples: &[f32]) -> Result<(), WriteError> {
        let (writer, frames, channels) = (&mut self.writer, &mut self.frames, self.channels);
        let cap = self.cap;
        let mut put = |values: &[f32]| -> Result<(), WriteError> {
            let values = within_cap(values, *frames, cap, channels);
            for &value in values {
                writer.write_sample(value)?;
            }
            *frames += (values.len() / channels) as u64;
            Ok(())
        };
        match &mut self.resampler {
            Some(resampler) => resampler.push(samples, &mut put),
            None => put(samples),
        }
    }

    /// Appends `frames` silent frames at the source rate: frames the feed dropped, so the
    /// recording keeps its length.
    pub fn pad(&mut self, frames: u64) -> Result<(), WriteError> {
        let silence = vec![0.0; self.channels * 1024];
        let mut left = frames;
        while left > 0 {
            let now = left.min(1024);
            self.write(&silence[..now as usize * self.channels])?;
            left -= now;
        }
        Ok(())
    }

    /// Completes the file and returns its length in frames at the project's rate.
    pub fn finish(mut self) -> Result<u64, WriteError> {
        let (writer, frames, channels) = (&mut self.writer, &mut self.frames, self.channels);
        let cap = self.cap;
        if let Some(resampler) = &mut self.resampler {
            resampler.finish(&mut |values: &[f32]| -> Result<(), WriteError> {
                let values = within_cap(values, *frames, cap, channels);
                for &value in values {
                    writer.write_sample(value)?;
                }
                *frames += (values.len() / channels) as u64;
                Ok(())
            })?;
        }
        let frames = self.frames;
        self.writer.finalize()?;
        Ok(frames)
    }
}

/// The front of `values` (interleaved, `channels` a frame) that fits a file holding `frames`
/// frames under `cap`: whole frames, none past the cap.
fn within_cap(values: &[f32], frames: u64, cap: Option<u64>, channels: usize) -> &[f32] {
    let Some(cap) = cap else {
        return values;
    };
    let room = usize::try_from(cap.saturating_sub(frames))
        .unwrap_or(usize::MAX)
        .saturating_mul(channels);
    &values[..values.len().min(room)]
}

/// Why a recording failed, and its feed, given back for the next one.
pub type RecordFailure = (WriteError, Box<dyn Feed>);

/// A recording made on a writer thread.
pub struct Recorder {
    stop: Arc<AtomicBool>,
    /// The file holds the frames its cap allows.
    full: Arc<AtomicBool>,
    handle: JoinHandle<Result<Recorded, RecordFailure>>,
    path: PathBuf,
}

/// A finished recording: its file, its length at the project's rate, and the feed, given back
/// for the next recording.
pub struct Recorded {
    pub path: PathBuf,
    pub frames: u64,
    pub feed: Box<dyn Feed>,
}

impl Recorder {
    /// Starts writing `feed` into `path` at `project_rate` on a thread of its own. On failure
    /// the feed comes back, unless the thread could not start, which takes it along.
    pub fn start(
        path: PathBuf,
        feed: Box<dyn Feed>,
        project_rate: u32,
    ) -> Result<Self, (WriteError, Option<Box<dyn Feed>>)> {
        Self::start_capped(path, feed, project_rate, None)
    }

    /// As [`start`](Self::start), the file taking no more than `cap` frames at the project's
    /// rate: once it holds them [`full`](Self::full) says so, and the feed is read on and
    /// dropped until [`finish`](Self::finish), so it never fills.
    pub fn start_capped(
        path: PathBuf,
        mut feed: Box<dyn Feed>,
        project_rate: u32,
        cap: Option<u64>,
    ) -> Result<Self, (WriteError, Option<Box<dyn Feed>>)> {
        // Whatever waits in the feed was played before this take began: the end of the last
        // take, which can reach the feed after that take's last drain (Stop takes effect at the
        // player's next buffer, not at once).
        feed.read_into(&mut Vec::new());
        let mut file =
            match RecordingFile::create(&path, feed.channels(), feed.rate(), project_rate) {
                Ok(file) => file.capped(cap),
                Err(error) => return Err((error, Some(feed))),
            };
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let full = Arc::new(AtomicBool::new(false));
        let filled = Arc::clone(&full);
        let target = path.clone();
        let handle = thread::Builder::new()
            .name("dawsome-record".into())
            .spawn(move || {
                let mut buffer = Vec::new();
                let mut padded = feed.dropped();
                loop {
                    let last = stopping.load(Ordering::Acquire);
                    let written = drain(&mut *feed, &mut file, &mut buffer, &mut padded);
                    if let Err(error) = written {
                        return Err((error, feed));
                    }
                    if file.is_full() {
                        filled.store(true, Ordering::Release);
                    }
                    if last {
                        break;
                    }
                    thread::sleep(DRAIN_PERIOD);
                }
                match file.finish() {
                    Ok(frames) => Ok(Recorded {
                        path: target,
                        frames,
                        feed,
                    }),
                    Err(error) => Err((error, feed)),
                }
            });
        match handle {
            Ok(handle) => Ok(Self {
                stop,
                full,
                handle,
                path,
            }),
            // The thread never started, and the feed went with its closure.
            Err(error) => Err((WriteError::Io(error), None)),
        }
    }

    /// The file being written.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the writer thread has stopped by itself, which it only does when a write failed
    /// (a full disk, say): [`finish`](Self::finish) then gives the error.
    pub fn failed(&self) -> bool {
        self.handle.is_finished()
    }

    /// Whether the file holds the frames its cap allows: the take is done.
    pub fn full(&self) -> bool {
        self.full.load(Ordering::Acquire)
    }

    /// Writes what is left and completes the file.
    pub fn finish(self) -> Result<Recorded, RecordFailure> {
        self.stop.store(true, Ordering::Release);
        self.handle
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    }
}

/// Moves what `feed` holds into `file`, padding the frames it dropped since `padded`.
pub fn drain(
    feed: &mut dyn Feed,
    file: &mut RecordingFile,
    buffer: &mut Vec<f32>,
    padded: &mut u64,
) -> Result<(), WriteError> {
    buffer.clear();
    feed.read_into(buffer);
    file.write(buffer)?;
    let dropped = feed.dropped();
    if dropped > *padded {
        file.pad(dropped - *padded)?;
        *padded = dropped;
    }
    Ok(())
}

#[cfg(test)]
mod tests {

    // Gate 2026-10-24: the callback pushes a frame value by value, so a drain under load saw
    // half a stereo frame and a 48 → 44.1 kHz take came out 1411 frames instead of 1412.
    #[test]
    fn an_input_feed_reads_whole_frames_only() {
        let (mut writer, mut feed, _peak) = input_ring(2, 48_000, 1);
        let _ = writer.ring.push(0.25);
        let mut out = Vec::new();
        assert_eq!(feed.read_into(&mut out), 0, "half a frame waits");
        let _ = writer.ring.push(0.5);
        assert_eq!(feed.read_into(&mut out), 2);
        assert_eq!(out, [0.25, 0.5]);
    }

    #[test]
    fn a_resampled_take_counts_a_frame_split_across_reads() {
        let mut resampler = StreamResampler::new(2, 48_000, 44_100).expect("a resampler");
        let mut written = 0u64;
        let mut write = |frames: &[f32]| {
            written += (frames.len() / 2) as u64;
            Ok(())
        };
        resampler
            .push(&vec![0.0; 1535 * 2 + 1], &mut write)
            .expect("pushes");
        resampler
            .push(&[0.0], &mut write)
            .expect("pushes the other half");
        resampler.finish(&mut write).expect("finishes");
        assert_eq!(written, 1412, "ceil(1536 · 44100 / 48000)");
    }

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dawsome-record-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir.join(name)
    }

    fn read(path: &Path) -> (hound::WavSpec, Vec<f32>) {
        let mut reader = hound::WavReader::open(path).expect("opens");
        let spec = reader.spec();
        let samples = reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .expect("reads");
        (spec, samples)
    }

    // A capped file (a pattern take of one pass): of 7 stereo frames written in pieces it keeps
    // the first 5 and says it is full; padding past the cap adds nothing. At another rate the
    // cap counts the project's frames: 1536 frames at 48 kHz are 1412 at 44.1 kHz, capped to
    // 1000, the tail flushed at the end included.
    #[test]
    fn a_capped_recording_keeps_its_first_frames() {
        let path = temp("capped.wav");
        let mut file = RecordingFile::create(&path, 2, 44_100, 44_100)
            .expect("creates")
            .capped(Some(5));
        let frames: Vec<f32> = (0..14).map(|value| value as f32).collect();
        file.write(&frames[..6]).expect("writes");
        assert!(!file.is_full());
        file.write(&frames[6..]).expect("writes");
        assert!(file.is_full());
        file.pad(3).expect("pads");
        assert_eq!(file.finish().expect("finishes"), 5);
        let (_, samples) = read(&path);
        assert_eq!(samples, frames[..10]);
        let path = temp("capped_resampled.wav");
        let mut file = RecordingFile::create(&path, 2, 48_000, 44_100)
            .expect("creates")
            .capped(Some(1000));
        file.write(&vec![0.25; 1536 * 2]).expect("writes");
        assert_eq!(file.finish().expect("finishes"), 1000);
        assert_eq!(read(&path).1.len(), 2000);
    }

    // At the project's rate the file holds the frames as they came, at the feed's channel
    // count; dropped frames are padded with silence.
    #[test]
    fn a_recording_at_the_project_rate_keeps_its_frames() {
        let path = temp("mono.wav");
        let mut file = RecordingFile::create(&path, 1, 44_100, 44_100).expect("creates");
        file.write(&[0.5, -0.25]).expect("writes");
        file.pad(2).expect("pads");
        file.write(&[1.0]).expect("writes");
        assert_eq!(file.finish().expect("finishes"), 5);
        let (spec, samples) = read(&path);
        assert_eq!((spec.channels, spec.sample_rate), (1, 44_100));
        assert_eq!(samples, [0.5, -0.25, 0.0, 0.0, 1.0]);
    }

    // At another rate the file is converted as it is written: 48000 frames of a steady level at
    // 48 kHz become exactly ceil(48000 · 44100 / 48000) = 44100 frames at 44.1 kHz, the level
    // kept away from the ends, however the frames arrive.
    #[test]
    fn a_recording_at_another_rate_is_converted_to_the_whole_length() {
        let path = temp("converted.wav");
        let mut file = RecordingFile::create(&path, 2, 48_000, 44_100).expect("creates");
        let steady = vec![0.5f32; 2 * 48_000];
        for piece in steady.chunks(2 * 777) {
            file.write(piece).expect("writes");
        }
        assert_eq!(file.finish().expect("finishes"), 44_100);
        let (spec, samples) = read(&path);
        assert_eq!((spec.channels, spec.sample_rate), (2, 44_100));
        assert_eq!(samples.len(), 2 * 44_100);
        assert!(samples[2 * 4410..2 * 39_690]
            .iter()
            .all(|v| (v - 0.5).abs() < 1e-3));
    }

    // An input channel's monitor ring takes the sides it picks from each device frame (here
    // swapped, and a mono pick played on both sides) and, offline, plays every frame as it
    // came, then silence; a side the device lacks takes its first channel.
    #[test]
    fn an_offline_monitor_plays_its_picked_sides_then_silence() {
        let device = [0.25, -0.5, 9.0, 0.125, 1.0, 9.0];
        let (mut writer, mut feed, peak) = monitor_ring(InputChannels::Stereo(1, 0), 4, None);
        writer.push(&device, 3, Some(0));
        assert_eq!(f32::from_bits(peak.swap(0, Ordering::Relaxed)), 1.0);
        assert_eq!(feed.channels(), 2);
        let mut out = [[7.0; 2]; 3];
        feed.read(&mut out);
        assert_eq!(out, [[-0.5, 0.25], [1.0, 0.125], [0.0, 0.0]]);
        let (mut writer, mut feed, _) = monitor_ring(InputChannels::Mono(1), 4, None);
        writer.push(&device, 3, Some(0));
        let mut out = [[7.0; 2]; 2];
        feed.read(&mut out);
        assert_eq!((feed.channels(), out), (1, [[-0.5, -0.5], [1.0, 1.0]]));
        let (mut writer, mut feed, _) = monitor_ring(InputChannels::Stereo(0, 5), 4, None);
        writer.push(&[0.5], 1, Some(0));
        feed.read(&mut out[..1]);
        assert_eq!(out[0], [0.5, 0.5]);
    }

    // Live, with 4-frame input buffers and 4-frame output blocks: silence until a cushion of 8
    // frames waits, then the frames in order; dry, it plays silence and waits for a cushion one
    // input block longer (12); more than two blocks past the cushion, what waits past it is
    // dropped.
    #[test]
    fn a_live_monitor_waits_for_its_cushion_grows_it_and_drops_what_runs_ahead() {
        let (mut writer, mut feed, _) = monitor_ring(
            InputChannels::Mono(0),
            64,
            Some(Pacing {
                output_block: 4,
                rate: 48_000,
            }),
        );
        let stats = feed.stats();
        let buffer = |from: usize| -> Vec<f32> { (from..from + 4).map(|v| v as f32).collect() };
        let mut out = [[9.0; 2]; 4];
        writer.push(&buffer(0), 1, Some(0));
        feed.read(&mut out);
        assert_eq!(out, [[0.0; 2]; 4], "4 frames wait: below the cushion");
        assert_eq!(stats.cushion.load(Ordering::Relaxed), 8);
        writer.push(&buffer(4), 1, Some(0));
        feed.read(&mut out);
        assert_eq!(out.map(|f| f[0]), [0.0, 1.0, 2.0, 3.0]);
        feed.read(&mut out);
        assert_eq!(out.map(|f| f[0]), [4.0, 5.0, 6.0, 7.0]);
        feed.read(&mut out);
        assert_eq!(out, [[0.0; 2]; 4], "dry");
        assert_eq!(stats.underruns.load(Ordering::Relaxed), 1);
        for from in [8, 12] {
            writer.push(&buffer(from), 1, Some(0));
        }
        feed.read(&mut out);
        assert_eq!(out, [[0.0; 2]; 4], "8 wait: below the grown cushion of 12");
        writer.push(&buffer(16), 1, Some(0));
        feed.read(&mut out);
        assert_eq!(out.map(|f| f[0]), [8.0, 9.0, 10.0, 11.0]);
        for from in [20, 24, 28, 32, 36] {
            writer.push(&buffer(from), 1, Some(0));
        }
        // 28 wait (12..40), past 12 + 8: the oldest 16 go, the newest 12 stay.
        feed.read(&mut out);
        assert_eq!(out.map(|f| f[0]), [28.0, 29.0, 30.0, 31.0]);
        assert_eq!(stats.skipped.load(Ordering::Relaxed), 16);
    }

    /// A live monitor run against an input whose buffers come on a clock of their own: the
    /// player reads `block` frames every `block / rate` seconds; the input completes a buffer of
    /// `block` frames every `block / (rate · ratio)` seconds (below 1 its clock runs slow), and
    /// some buffers come late, in order (a buffer never comes before the one ahead of it). Every
    /// buffer due is pushed before each read. Each input frame holds its number (from 1, modulo
    /// 2^23), so a frame played twice or out of order is seen. Seeded, so every run is the same.
    struct Pacer {
        writer: InputWriter,
        feed: MonitorFeed,
        stats: Arc<MonitorStats>,
        rate: f64,
        block: usize,
        /// When the next read happens, and when the next input buffer completes and comes.
        now: f64,
        input_clock: f64,
        next_delivery: f64,
        frames_in: u64,
        last_played: f32,
        backwards: u64,
        most_cushion: u64,
        rng: u64,
    }

    /// Which input buffers come late: `per_mille` of them, by `lo..hi` frames.
    #[derive(Clone, Copy)]
    struct Late {
        per_mille: u64,
        lo: u64,
        hi: u64,
    }

    impl Pacer {
        fn new(rate: u64, block: usize) -> Self {
            let pacing = Pacing {
                output_block: block as u64,
                rate: rate as u32,
            };
            let (writer, feed, _) =
                monitor_ring(InputChannels::Mono(0), 4 * rate as usize, Some(pacing));
            let stats = feed.stats();
            let first = block as f64 / rate as f64;
            Self {
                writer,
                feed,
                stats,
                rate: rate as f64,
                block,
                now: first,
                input_clock: first,
                next_delivery: first,
                frames_in: 0,
                last_played: 0.0,
                backwards: 0,
                most_cushion: 0,
                rng: 0x5eed,
            }
        }

        fn random(&mut self) -> u64 {
            // xorshift64, so a run reproduces exactly.
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 7;
            self.rng ^= self.rng << 17;
            self.rng
        }

        /// `seconds` of reads, the input at `ratio` with `late` buffers.
        fn run(&mut self, seconds: f64, ratio: f64, late: Option<Late>) {
            let period = self.block as f64 / self.rate;
            let mut buffer = vec![0.0f32; self.block];
            let mut out = vec![[0.0f32; 2]; self.block];
            let end = self.now + seconds;
            while self.now < end {
                while self.next_delivery <= self.now {
                    for value in &mut buffer {
                        self.frames_in += 1;
                        *value = (self.frames_in % (1 << 23)) as f32;
                    }
                    self.writer.push(&buffer, 1, Some(0));
                    self.input_clock += period / ratio;
                    let behind = match late {
                        Some(late) if self.random() % 1000 < late.per_mille => {
                            let frames = late.lo + self.random() % (late.hi - late.lo);
                            frames as f64 / self.rate
                        }
                        _ => 0.0,
                    };
                    self.next_delivery = (self.input_clock + behind).max(self.next_delivery);
                }
                self.feed.read(&mut out);
                for frame in &out {
                    let played = frame[0];
                    let wrapped = self.last_played > 8_000_000.0 && played < 1000.0;
                    if played != 0.0 && played <= self.last_played && !wrapped {
                        self.backwards += 1;
                    }
                    if played != 0.0 {
                        self.last_played = played;
                    }
                }
                let cushion = self.stats.cushion.load(Ordering::Relaxed);
                self.most_cushion = self.most_cushion.max(cushion);
                self.now += period;
            }
        }

        fn underruns(&self) -> u64 {
            self.stats.underruns.load(Ordering::Relaxed)
        }

        fn skipped(&self) -> u64 {
            self.stats.skipped.load(Ordering::Relaxed)
        }

        fn cushion(&self) -> u64 {
            self.stats.cushion.load(Ordering::Relaxed)
        }

        fn waiting(&self) -> u64 {
            (self.feed.ring.slots() / self.feed.channels) as u64
        }
    }

    /// The pacing scenarios run at 4000 Hz with 64-frame blocks (16 ms each; 512-frame blocks at
    /// 48 kHz are 10.7 ms): drift and lateness are relative (ppm, and late by blocks), so their
    /// shape is the one at 48 kHz, and two hours run in a debug test. The late buffers,
    /// 600..1500 and 400..1100 frames behind at 512-frame blocks, are 75..187 and 50..137 here.
    const PACED: (u64, usize) = (4000, 64);

    /// The base cushion at [`PACED`]: one input block and one output block.
    const PACED_BASE: u64 = 2 * PACED.1 as u64;

    // Found 2026-10-23: input buffers that come late now and then, in order (a
    // simulation: 0.2 % of them 600..1500 frames late for 30 min, 1 % of them 400..1100
    // late for 10 min, at 48 kHz). The monitor runs dry no more often than before the cushion
    // could shrink (c65b6c8's read, measured here with the shrink switched off: 1 and 2 dry
    // spells; f9f236c's shrink after every 10 s gave 112 and 48), drops nothing and plays every
    // frame once, in order: a late buffer dips what waits, so the block it used is not taken
    // away again.
    #[test]
    fn late_input_buffers_keep_the_cushion_they_need() {
        let (rate, block) = PACED;
        for (seconds, late, before) in [
            (
                1800.0,
                Late {
                    per_mille: 2,
                    lo: 75,
                    hi: 187,
                },
                1,
            ),
            (
                600.0,
                Late {
                    per_mille: 10,
                    lo: 50,
                    hi: 137,
                },
                2,
            ),
        ] {
            let mut pacer = Pacer::new(rate, block);
            pacer.run(seconds, 1.0, Some(late));
            assert!(pacer.underruns() >= 1, "the first late buffer runs it dry");
            assert!(
                pacer.underruns() <= before,
                "{} per mille late: {} dry spells, {before} before",
                late.per_mille,
                pacer.underruns()
            );
            assert_eq!((pacer.skipped(), pacer.backwards), (0, 0));
            assert!(pacer.cushion() > PACED_BASE, "it keeps the grown cushion");
        }
    }

    // Two hours of an input clock 30 or 100 ppm slow (the
    // same simulation). What waits falls a block at a time, so the monitor runs dry now and
    // then and its cushion grows; it never shrinks while the drift goes on (each stretch it
    // watches lasts two such steps, and a step down shows in it), so it runs dry no more often
    // than c65b6c8's read did (3 and 8 times here; f9f236c gave 6 and 22) and drops nothing.
    // Once the clocks match, what waits stays level, and the cushion comes back to its base
    // within 45 minutes, and what waits with it.
    #[test]
    fn drift_runs_dry_no_more_than_before_and_the_cushion_comes_back() {
        let (rate, block) = PACED;
        for (ppm, before) in [(30e-6, 3), (100e-6, 8)] {
            let mut pacer = Pacer::new(rate, block);
            pacer.run(7200.0, 1.0 - ppm, None);
            assert!(
                pacer.underruns() <= before,
                "{ppm}: {} dry spells, {before} before",
                pacer.underruns()
            );
            assert_eq!(pacer.skipped(), 0, "{ppm}: nothing dropped while drifting");
            assert!(pacer.most_cushion > PACED_BASE, "{ppm}: grown");
            let dry = pacer.underruns();
            pacer.run(2700.0, 1.0, None);
            assert_eq!(pacer.underruns(), dry, "{ppm}: matched, it never runs dry");
            assert_eq!(pacer.cushion(), PACED_BASE, "{ppm}: back at its base");
            assert!(
                pacer.waiting() + block as u64 <= PACED_BASE,
                "{ppm}: what waits is back too"
            );
            assert_eq!(pacer.backwards, 0);
        }
    }

    // The input ring keeps the first channels of each device frame, meters every frame, and
    // counts the frames it has no room for.
    #[test]
    fn the_input_ring_keeps_meters_and_counts_what_it_drops() {
        let (mut writer, mut feed, peak) = input_ring(1, 4, 1);
        writer.push(&[0.25, 9.0, -0.5, 9.0, 0.125, 9.0], 2, None);
        assert_eq!(
            f32::from_bits(peak.swap(0, Ordering::Relaxed)),
            0.5,
            "metered, not kept"
        );
        writer.push(
            &[0.25, 9.0, -0.5, 9.0, 0.125, 9.0, 1.0, 9.0, 2.0, 9.0],
            2,
            Some(0),
        );
        let mut out = Vec::new();
        assert_eq!(feed.read_into(&mut out), 4);
        assert_eq!(out, [0.25, -0.5, 0.125, 1.0]);
        assert_eq!(feed.dropped(), 1);
    }
}
