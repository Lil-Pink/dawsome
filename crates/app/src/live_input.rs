//! The input channels' devices in the window: one input stream per device the document's input
//! channels name, opened while the document has an input channel. Its callback fills, for each
//! input channel on that device, a monitor ring (the player's feed: the channel's sound) and a
//! record ring (Record → Input while the channel is selected), each with the channel's picked
//! sides; nothing in the callback allocates, locks or waits.
//!
//! A device opens at the project's rate when it offers it. Otherwise it opens at a rate of its
//! own (its default, else the offered rate nearest it) and its frames reach the monitors through
//! an [`InputBridge`], the output's resampler run the other way; the record ring keeps the
//! device's frames as they come, and the take's writer converts them on the way to the file, as
//! it does for the default input.
//!
//! Only a window run nobody scripts lists or opens a device ([`InputDevices::System`]). A
//! scripted run and the tests name their devices ([`InputDevices::Given`]): a channel on one of
//! those gets its rings and no stream, and whoever holds the writers (a test) feeds them.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleFormat, StreamConfig, SupportedBufferSize};
use dawsome_engine::input::InputFeeds;
use dawsome_engine::project::{ChannelId, ChannelKind, InputChannels, InputSource, Project};
use dawsome_engine::realtime::RecordClock;
use dawsome_media::record::{
    monitor_ring, picked_ring, Feed, InputWriter, MonitorFeed, MonitorStats, Pacing,
};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Resampler, ResamplerConstructionError};

use crate::audio::{self, stream_nanos, PlaybackClock, RatePlan};
use crate::record::first_kept;

/// Seconds of input each ring holds before frames are dropped (and counted).
const RING_SECONDS: usize = 4;

/// Frames per input buffer asked of a device, when it takes a fixed size: the output's.
const INPUT_BUFFER_FRAMES: u32 = 512;

/// How long opening an input device may take before it counts as failed.
const OPEN_TIMEOUT: Duration = Duration::from_secs(2);

/// Where the input devices come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputDevices {
    /// The system's, listed and opened through the audio host: a window run nobody scripts.
    System,
    /// These devices only, each with its channel count, opening no stream: a scripted run
    /// names none, so its screenshots never depend on the machine, and a test names its own.
    Given(Vec<DeviceInfo>),
}

/// An input device as the window lists it: its name and how many channels it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Empty for the system's default input.
    pub name: String,
    pub channels: u16,
    /// The rates a given device offers, its default first; none: any rate, so it opens at the
    /// project's. The system's listing leaves them out (the picker shows names), and opening a
    /// system device asks the device itself.
    pub rates: Vec<RangeInclusive<u32>>,
}

/// The name an input source's `device` shows as: "System default input" for none.
pub fn device_label(device: &str) -> &str {
    if device.is_empty() {
        "System default input"
    } else {
        device
    }
}

/// Names the audio host lists among the inputs that stand for the system's default input or for
/// no device at all (as for the outputs): the default itself is offered as "System default
/// input".
const STAND_INS: [&str; 9] = [
    "default_source",
    "default_input",
    "default_sink",
    "default_output",
    "default",
    "sysdefault",
    "pulse",
    "pipewire",
    "unknown",
];

/// Whether `name`, as the audio host lists it among the inputs, is an input device the window
/// offers.
pub fn is_input_device(name: &str) -> bool {
    !STAND_INS.contains(&name)
}

/// The system's input devices: the default first (its name empty), then each device the audio
/// host lists, once, with its most channels. Listing opens no stream. No test runs it.
pub fn system_devices() -> Result<Vec<DeviceInfo>, String> {
    let host = cpal::default_host();
    let most_channels = |device: &cpal::Device| {
        device
            .supported_input_configs()
            .map(|configs| configs.map(|config| config.channels()).max().unwrap_or(0))
            .unwrap_or(0)
    };
    let mut listed = vec![DeviceInfo {
        name: String::new(),
        channels: host
            .default_input_device()
            .map_or(2, |device| most_channels(&device)),
        rates: Vec::new(),
    }];
    let devices = host.input_devices().map_err(|error| error.to_string())?;
    for device in devices {
        let name = device.to_string();
        if is_input_device(&name) && !listed.iter().any(|known| known.name == name) {
            listed.push(DeviceInfo {
                name,
                channels: most_channels(&device),
                rates: Vec::new(),
            });
        }
    }
    Ok(listed)
}

/// What became of an input channel's device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputState {
    /// Open: how many channels the device has, the frames per buffer it was asked for (none
    /// when it chose its own), and how it is resampled when it runs at another rate than the
    /// project's.
    Open {
        device_channels: u16,
        block: Option<u32>,
        resampled: Option<Resampled>,
    },
    /// Not open, and why: shown in the channel's window and the toolbar.
    Refused(String),
}

/// An input device running at another rate than the project's: its rate, and the frames (at
/// the project's rate) its resampler holds a sound back by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resampled {
    pub device_rate: u32,
    pub delay: u64,
}

/// One input channel's live input.
pub struct ChannelInput {
    pub source: InputSource,
    pub state: InputState,
    /// Its meter's peak cell (f32 bits, taken with `swap(0)`).
    peak: Arc<AtomicU32>,
    /// Its level as the meter shows it, falling after a peak.
    level: f32,
    /// Its record ring's feed, while no take holds it.
    record: Option<Box<dyn Feed>>,
    /// What its monitor feed says of itself, while open.
    stats: Option<Arc<MonitorStats>>,
    /// A given device's ring writers, for whoever feeds them: what a system device's callback
    /// pushes into, so a test's frames take the stream's path.
    given: Option<DeviceWriters>,
}

impl ChannelInput {
    fn refused(source: InputSource, why: String) -> Self {
        Self {
            source,
            state: InputState::Refused(why),
            peak: Arc::new(AtomicU32::new(0)),
            level: 0.0,
            record: None,
            stats: None,
            given: None,
        }
    }

    /// The level its meter shows, 0 to 1 and over.
    pub fn level(&self) -> f32 {
        self.level
    }

    /// The frames its monitor keeps waiting (its cushion), once it has read.
    pub fn cushion(&self) -> Option<u64> {
        let cushion = self.stats.as_ref()?.cushion.load(Ordering::Relaxed);
        (cushion > 0).then_some(cushion)
    }

    /// How often its monitor ran dry.
    pub fn underruns(&self) -> u64 {
        self.stats
            .as_ref()
            .map_or(0, |stats| stats.underruns.load(Ordering::Relaxed))
    }
}

/// An open device stream; dropping it closes the device.
struct DeviceStream {
    _stream: cpal::Stream,
    name: String,
    /// The device reported an error.
    failed: Arc<AtomicBool>,
}

/// The input channels' live inputs: what is open, by channel, and the device streams.
#[derive(Default)]
pub struct LiveInputs {
    channels: BTreeMap<ChannelId, ChannelInput>,
    streams: Vec<DeviceStream>,
    /// A set of feeds the player had no room for yet; the next sync takes it
    /// ([`take_pending`](Self::take_pending)) and hands it over.
    pending: Option<InputFeeds>,
    /// The output clock the streams were opened with, which lines a recording's frames up.
    playback: Option<Arc<PlaybackClock>>,
}

/// The input channels of `project`, each with its source.
pub fn wanted(project: &Project) -> BTreeMap<ChannelId, InputSource> {
    project
        .channels
        .iter()
        .filter(|(_, channel)| channel.kind == ChannelKind::Input)
        .filter_map(|(&id, channel)| Some((id, channel.input.clone()?)))
        .collect()
}

/// What an input stream is opened for: the rate the player renders at (the project's), the
/// output's block, and the clocks a recording's frames are lined up by.
pub struct Opening {
    pub rate: u32,
    pub output_block: u64,
    pub clock: RecordClock,
    pub playback: Option<Arc<PlaybackClock>>,
}

impl LiveInputs {
    /// Opens the devices `wanted` names, one stream per device: each channel's rings and, for
    /// the player, a set of its monitor feeds. A device that is not there, or offers no rate the
    /// resampler can bring to the player's, refuses its channels, which play silence and say
    /// why.
    pub fn open(
        devices: &InputDevices,
        wanted: &BTreeMap<ChannelId, InputSource>,
        opening: &Opening,
    ) -> (Self, InputFeeds) {
        let mut inputs = Self {
            playback: opening.playback.clone(),
            ..Self::default()
        };
        let mut feeds = InputFeeds::new();
        let mut by_device: BTreeMap<&str, Vec<(ChannelId, &InputSource)>> = BTreeMap::new();
        for (&id, source) in wanted {
            by_device
                .entry(source.device.as_str())
                .or_default()
                .push((id, source));
        }
        for (device, channels) in by_device {
            let opened = match devices {
                InputDevices::System => open_system(device, &channels, opening),
                InputDevices::Given(listed) => open_given(listed, device, &channels, opening),
            };
            match opened {
                Ok((stream, opened)) => {
                    inputs.streams.extend(stream);
                    for (id, input, feed) in opened {
                        feeds.insert(id, Box::new(feed));
                        inputs.channels.insert(id, input);
                    }
                }
                Err(why) => {
                    for (id, source) in channels {
                        let input = ChannelInput::refused(source.clone(), why.clone());
                        inputs.channels.insert(id, input);
                    }
                }
            }
        }
        (inputs, feeds)
    }

    /// Whether what is open is what `wanted` names.
    pub fn matches(&self, wanted: &BTreeMap<ChannelId, InputSource>) -> bool {
        self.channels.len() == wanted.len()
            && wanted.iter().all(|(id, source)| {
                self.channels
                    .get(id)
                    .is_some_and(|input| &input.source == source)
            })
    }

    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// Whether what is open was opened for the output whose clock is `playback` (none: no
    /// output); with nothing open, any output will do. An output opened again (another device
    /// chosen, a stream retried) has a clock of its own.
    pub fn opened_for(&self, playback: Option<&Arc<PlaybackClock>>) -> bool {
        self.channels.is_empty()
            || match (&self.playback, playback) {
                (Some(opened), Some(now)) => Arc::ptr_eq(opened, now),
                (None, None) => true,
                _ => false,
            }
    }

    /// Input channel `id`'s live input, when the document has that input channel.
    pub fn channel(&self, id: ChannelId) -> Option<&ChannelInput> {
        self.channels.get(&id)
    }

    /// The first refusal, with its channel: what the toolbar says.
    pub fn refusal(&self) -> Option<(ChannelId, &str)> {
        self.channels
            .iter()
            .find_map(|(&id, input)| match &input.state {
                InputState::Refused(why) => Some((id, why.as_str())),
                InputState::Open { .. } => None,
            })
    }

    /// A device whose stream has reported an error, by name.
    pub fn failed_device(&self) -> Option<&str> {
        self.streams
            .iter()
            .find(|stream| stream.failed.load(Ordering::Relaxed))
            .map(|stream| device_label(&stream.name))
    }

    /// Input channel `id`'s record feed, for a take; `None` when it is not open or a take holds
    /// it already.
    pub fn take_record_feed(&mut self, id: ChannelId) -> Option<Box<dyn Feed>> {
        self.channels.get_mut(&id)?.record.take()
    }

    /// Gives a take's record feed back to input channel `id`; dropped when the channel is gone
    /// (its device closed meanwhile).
    pub fn give_back(&mut self, id: ChannelId, feed: Box<dyn Feed>) {
        if let Some(input) = self.channels.get_mut(&id) {
            input.record.get_or_insert(feed);
        }
    }

    /// Whether a take holds a channel's record feed: the devices then stay as they are.
    pub fn recording(&self) -> bool {
        self.channels
            .values()
            .any(|input| matches!(input.state, InputState::Open { .. }) && input.record.is_none())
    }

    /// Reads each channel's peak since the last read into its level: the peak when louder, else
    /// the level falling by half every 100 ms since `last`.
    pub fn read_meters(&mut self, since: Duration) {
        let fall = 0.5f32.powf(since.as_secs_f32() / 0.1);
        for input in self.channels.values_mut() {
            let peak = f32::from_bits(input.peak.swap(0, Ordering::Relaxed));
            input.level = peak.max(input.level * fall);
        }
    }

    /// Keeps `feeds` to hand the player later: it had no room for them.
    pub fn keep_pending(&mut self, feeds: InputFeeds) {
        self.pending = Some(feeds);
    }

    /// The set of feeds the player had no room for, to try again.
    pub fn take_pending(&mut self) -> Option<InputFeeds> {
        self.pending.take()
    }
}

#[cfg(test)]
impl ChannelInput {
    /// What its monitor feed says of itself: a handle that outlives neither the feed nor this.
    pub fn monitor_stats(&self) -> Option<Arc<MonitorStats>> {
        self.stats.clone()
    }
}

#[cfg(test)]
impl LiveInputs {
    /// Feeds input channel `id` of a given device as its callback would: `samples` of
    /// `device_channels` interleaved channels into its monitor ring, and into its record ring
    /// from frame `keep_from` on.
    pub fn feed_given(
        &mut self,
        id: ChannelId,
        samples: &[f32],
        device_channels: usize,
        keep_from: Option<usize>,
    ) {
        if let Some(writers) = self
            .channels
            .get_mut(&id)
            .and_then(|input| input.given.as_mut())
        {
            writers.push(samples, device_channels, keep_from);
        }
    }
}

/// A device's input frames resampled to the project's rate, for the monitors: device buffers of
/// any size gather into the resampler's fixed input block ([`audio::bridge_resampler`], the
/// output's, run from the device's rate to the project's), and each block's output goes on
/// whole, so what the monitors get does not depend on how the device sizes its buffers.
///
/// Every buffer is sized when it is built, on the UI thread; [`push`](Self::push) never
/// allocates, frees, locks or logs. The cost is latency: the resampler's output delay
/// ([`delay`](Self::delay)) and, when the device's buffers are not the block's size, part of a
/// block waiting to fill.
pub struct InputBridge {
    resampler: Box<dyn Resampler<f32>>,
    channels: usize,
    /// One input block, `filled` frames of it gathered so far.
    block: Box<[f32]>,
    filled: usize,
    /// One block's output.
    out: Box<[f32]>,
    /// The device's rate and the project's.
    from: u32,
    to: u32,
}

impl InputBridge {
    /// A bridge from `device_rate` to `project_rate` for `channels` interleaved channels.
    pub fn new(
        device_rate: u32,
        project_rate: u32,
        channels: usize,
    ) -> Result<Self, ResamplerConstructionError> {
        let channels = channels.max(1);
        let resampler = audio::bridge_resampler(device_rate, project_rate, channels)?;
        let block = vec![0.0; resampler.input_frames_next() * channels].into_boxed_slice();
        let out = vec![0.0; resampler.output_frames_max() * channels].into_boxed_slice();
        Ok(Self {
            resampler,
            channels,
            block,
            filled: 0,
            out,
            from: device_rate,
            to: project_rate,
        })
    }

    /// Frames, at the project's rate, from a sound entering the resampler to it leaving.
    pub fn delay(&self) -> u64 {
        self.resampler.output_delay() as u64
    }

    /// A device buffer of `frames` frames as frames at the project's rate, rounded up: the
    /// input's block as the monitor counts it. Why not the frames this buffer made: the
    /// resampler gives them in sub-blocks (294 frames at 48 to 44.1 kHz, one or two a 512-frame
    /// block), so one buffer of 1024 frames makes 882 or 1176 as the sub-blocks fall, and a
    /// block taken from them is as uneven; and why not the most a block can give (588): the
    /// monitor drains a block from what waits when its cushion shrinks, and a block said too
    /// long drained it dry (measured in the burst tests below).
    pub fn project_frames(&self, frames: usize) -> u64 {
        (frames as u64 * u64::from(self.to)).div_ceil(u64::from(self.from.max(1)))
    }

    /// The input's block a device buffer of `frames` frames gives the monitors: its span at
    /// the project's rate ([`project_frames`](Self::project_frames)), and no less than the most
    /// one resampler step gives (588 frames at 48 to 44.1 kHz), since a buffer shorter than that
    /// still sees its frames come a step at a time.
    pub fn block_of(&self, frames: usize) -> u64 {
        let step = (self.out.len() / self.channels) as u64;
        self.project_frames(frames).max(step)
    }

    /// Takes `samples` (whole frames of the bridge's channels; a part frame at the end is left
    /// out) and hands each block's resampled frames to `emit` as they are made.
    pub fn push(&mut self, samples: &[f32], mut emit: impl FnMut(&[f32])) {
        let channels = self.channels;
        let block_frames = self.block.len() / channels;
        let mut rest = &samples[..samples.len() / channels * channels];
        while !rest.is_empty() {
            let take = (block_frames - self.filled).min(rest.len() / channels);
            let at = self.filled * channels;
            self.block[at..at + take * channels].copy_from_slice(&rest[..take * channels]);
            self.filled += take;
            rest = &rest[take * channels..];
            if self.filled == block_frames {
                self.filled = 0;
                if let Some(written) = self.step() {
                    emit(&self.out[..written * channels]);
                }
            }
        }
    }

    /// Resamples the block gathered; the frames written, none when the resampler refused the
    /// buffers. Why that cannot happen and is still handled: every size comes from the
    /// resampler itself at construction; if it ever did, the block is left out rather than
    /// stale frames played.
    fn step(&mut self) -> Option<usize> {
        let channels = self.channels;
        let capacity = self.out.len() / channels;
        let input = InterleavedSlice::new(&self.block[..], channels, self.block.len() / channels);
        let output = InterleavedSlice::new_mut(&mut self.out[..], channels, capacity);
        match (input, output) {
            (Ok(input), Ok(mut output)) => self
                .resampler
                .process_into_buffer(&input, &mut output, None)
                .ok()
                .map(|(_, written)| written),
            _ => None,
        }
    }
}

/// What a device's frames go into: each channel's monitor and record writers, and the bridge
/// to the project's rate when the device runs at another. The input callback's one job, also
/// what a given device's frames take in a test.
pub struct DeviceWriters {
    bridge: Option<InputBridge>,
    /// Monitor then record, per channel.
    writers: Vec<(InputWriter, InputWriter)>,
}

impl DeviceWriters {
    /// Pushes a device buffer of `device_channels` interleaved channels: whole into each record
    /// ring from frame `keep_from` on (at the device's rate: the take's writer converts it),
    /// and into each monitor, through the bridge when there is one, the frames it made from this
    /// buffer, its length at the project's rate ([`InputBridge::block_of`]) then noted as the
    /// input's block: the monitor's cushion is sized by the burst one buffer brings, not by
    /// one resampler block of it. Nothing here allocates, locks or waits.
    pub fn push(&mut self, samples: &[f32], device_channels: usize, keep_from: Option<usize>) {
        let writers = &mut self.writers;
        for (_, record) in writers.iter_mut() {
            record.push(samples, device_channels, keep_from);
        }
        match &mut self.bridge {
            None => {
                for (monitor, _) in writers.iter_mut() {
                    monitor.push(samples, device_channels, Some(0));
                }
            }
            Some(bridge) => {
                bridge.push(samples, |frames| {
                    for (monitor, _) in writers.iter_mut() {
                        monitor.push(frames, device_channels, Some(0));
                    }
                });
                let block = bridge.block_of(samples.len() / device_channels.max(1));
                for (monitor, _) in writers.iter() {
                    monitor.note_buffer(block);
                }
            }
        }
    }
}

/// One channel's rings: its live input (holding the record feed) and its monitor feed for the
/// player, with the writers the device's callback fills. The monitor holds frames at the
/// player's rate; the record ring at the device's (`device_rate`), which the take's writer
/// converts.
fn rings(
    source: &InputSource,
    opening: &Opening,
    state: InputState,
    device_rate: u32,
) -> (ChannelInput, MonitorFeed, (InputWriter, InputWriter)) {
    let picked = source.channels;
    let frames = RING_SECONDS * usize::try_from(opening.rate).unwrap_or(48_000);
    let pacing = Pacing {
        output_block: opening.output_block,
        rate: opening.rate,
    };
    let (monitor, feed, peak) = monitor_ring(picked, frames, Some(pacing));
    let (record, record_feed, _) = picked_ring(picked, device_rate, RING_SECONDS);
    let input = ChannelInput {
        source: source.clone(),
        state,
        peak,
        level: 0.0,
        record: Some(Box::new(record_feed)),
        stats: Some(feed.stats()),
        given: None,
    };
    (input, feed, (monitor, record))
}

/// What opening one device gives: its stream (none for a given device), and each channel's live
/// input with its monitor feed.
type Opened = (
    Option<DeviceStream>,
    Vec<(ChannelId, ChannelInput, MonitorFeed)>,
);

/// A given device: rings with no stream, their writers kept for whoever feeds them, through a
/// bridge of each channel's own when the device does not offer the project's rate; refused when
/// the device is not listed.
fn open_given(
    listed: &[DeviceInfo],
    device: &str,
    channels: &[(ChannelId, &InputSource)],
    opening: &Opening,
) -> Result<Opened, String> {
    let info = listed
        .iter()
        .find(|info| info.name == device)
        .ok_or_else(|| format!("{} is not there", device_label(device)))?;
    let any = [1..=u32::MAX];
    let offered = if info.rates.is_empty() {
        &any[..]
    } else {
        &info.rates[..]
    };
    let default = offered.first().map_or(opening.rate, |range| *range.start());
    let rate = input_rate(device, opening.rate, offered, default)?;
    let device_channels = usize::from(info.channels).max(1);
    let bridge = || bridge_for(device, rate, opening.rate, device_channels);
    let resampled = bridge()?.map(|bridge| Resampled {
        device_rate: rate,
        delay: bridge.delay(),
    });
    let state = InputState::Open {
        device_channels: info.channels,
        block: Some(INPUT_BUFFER_FRAMES),
        resampled,
    };
    let mut opened = Vec::new();
    for &(id, source) in channels {
        let (mut input, feed, pair) = rings(source, opening, state.clone(), rate);
        input.given = Some(DeviceWriters {
            bridge: bridge()?,
            writers: vec![pair],
        });
        opened.push((id, input, feed));
    }
    Ok((None, opened))
}

/// The rate an input device opens at for a project at `project_rate`: the project's when one
/// of the `offered` 32-bit float ranges holds it (nothing resampled), else the device's
/// `default` rate, or the offered rate nearest it, resampled to the project's: the output
/// stream's rule ([`audio::choose_rate`]). Why not when it offers no float input at all.
pub fn input_rate(
    device: &str,
    project_rate: u32,
    offered: &[RangeInclusive<u32>],
    default: u32,
) -> Result<u32, String> {
    match audio::choose_rate(project_rate, offered, default, None) {
        Ok(RatePlan::Direct) => Ok(project_rate),
        Ok(RatePlan::Resample { device_rate }) => Ok(device_rate),
        Err(_) => Err(format!(
            "{} offers no 32-bit float input",
            device_label(device)
        )),
    }
}

/// The bridge a device at `device_rate` needs to reach the project's rate, none at that rate;
/// why not when the resampler cannot be built for the two rates.
fn bridge_for(
    device: &str,
    device_rate: u32,
    project_rate: u32,
    device_channels: usize,
) -> Result<Option<InputBridge>, String> {
    if device_rate == project_rate {
        return Ok(None);
    }
    InputBridge::new(device_rate, project_rate, device_channels)
        .map(Some)
        .map_err(|_| {
            format!(
                "{}: {device_rate} Hz cannot be resampled to {project_rate} Hz",
                device_label(device)
            )
        })
}

/// A system device: opened as 32-bit float at the player's rate, else at its own resampled to
/// the player's ([`input_rate`]), its callback filling every channel's rings. No test runs it.
fn open_system(
    device: &str,
    channels: &[(ChannelId, &InputSource)],
    opening: &Opening,
) -> Result<Opened, String> {
    let host = cpal::default_host();
    let found = if device.is_empty() {
        host.default_input_device()
    } else {
        host.input_devices()
            .map_err(|error| error.to_string())?
            .find(|candidate| candidate.to_string() == device)
    };
    let opened = found.ok_or_else(|| format!("{} is not there", device_label(device)))?;
    let float: Vec<_> = opened
        .supported_input_configs()
        .map_err(|error| error.to_string())?
        .filter(|range| range.sample_format() == SampleFormat::F32)
        .collect();
    let offered: Vec<_> = float
        .iter()
        .map(|range| range.min_sample_rate()..=range.max_sample_rate())
        .collect();
    let default = opened
        .default_input_config()
        .map_or(opening.rate, |config| config.sample_rate());
    let rate = input_rate(device, opening.rate, &offered, default)?;
    let supported = float
        .iter()
        .filter_map(|range| range.try_with_sample_rate(rate))
        .max_by_key(cpal::SupportedStreamConfig::channels)
        .ok_or_else(|| format!("{} cannot be opened", device_label(device)))?;
    let buffer_size = match supported.buffer_size() {
        SupportedBufferSize::Range { min, max } if (*min..=*max).contains(&INPUT_BUFFER_FRAMES) => {
            BufferSize::Fixed(INPUT_BUFFER_FRAMES)
        }
        _ => BufferSize::Default,
    };
    let block = match buffer_size {
        BufferSize::Fixed(frames) => Some(frames),
        BufferSize::Default => None,
    };
    let device_channels = supported.channels();
    let bridge = bridge_for(
        device,
        rate,
        opening.rate,
        usize::from(device_channels).max(1),
    )?;
    let state = InputState::Open {
        device_channels,
        block,
        resampled: bridge.as_ref().map(|bridge| Resampled {
            device_rate: rate,
            delay: bridge.delay(),
        }),
    };
    let mut opened_channels = Vec::new();
    let mut writers = Vec::new();
    for &(id, source) in channels {
        let (input, feed, pair) = rings(source, opening, state.clone(), rate);
        opened_channels.push((id, input, feed));
        writers.push(pair);
    }
    let config = StreamConfig {
        buffer_size,
        ..supported.config()
    };
    let stream = build_stream(
        &opened,
        config,
        usize::from(device_channels).max(1),
        DeviceWriters { bridge, writers },
        opening,
    )?;
    Ok((
        Some(DeviceStream {
            _stream: stream.0,
            name: device.to_owned(),
            failed: stream.1,
        }),
        opened_channels,
    ))
}

/// The input stream: every buffer into each channel's monitor ring whole (through the bridge
/// when the device runs at another rate), and into its record ring from the frame captured as
/// the song's first played while recording (none otherwise), timed by the output's clock as the
/// default input's recording is.
fn build_stream(
    device: &cpal::Device,
    config: StreamConfig,
    device_channels: usize,
    mut writers: DeviceWriters,
    opening: &Opening,
) -> Result<(cpal::Stream, Arc<AtomicBool>), String> {
    let failed = Arc::new(AtomicBool::new(false));
    let reporter = Arc::clone(&failed);
    let offset = Arc::new(AtomicI64::new(0));
    let stream_offset = Arc::clone(&offset);
    let song = opening.playback.clone();
    let clock = opening.clock.clone();
    let rate = config.sample_rate;
    let stream = device
        .build_input_stream(
            config,
            move |data: &[f32], info: &cpal::InputCallbackInfo| {
                let keep_from = clock.recording_from().map(|from| {
                    let start = song.as_ref().and_then(|song| song.time_of(from));
                    let capture = stream_nanos(info.timestamp().capture)
                        + i128::from(stream_offset.load(Ordering::Relaxed));
                    first_kept(start, capture, rate, data.len() / device_channels)
                });
                writers.push(data, device_channels, keep_from);
            },
            move |_: cpal::Error| reporter.store(true, Ordering::Relaxed),
            Some(OPEN_TIMEOUT),
        )
        .map_err(|error| format!("cannot open the input: {error}"))?;
    if let Some(playback) = &opening.playback {
        // Before `play`: no callback runs until then.
        offset.store(playback.offset_of(stream.now()), Ordering::Relaxed);
    }
    stream
        .play()
        .map_err(|error| format!("cannot start the input: {error}"))?;
    Ok((stream, failed))
}

/// "live, about N ms behind": what the input channel's window says of its delay, N the
/// monitor's cushion, the output's block and the resampler's delay (0 without one) at `rate`,
/// rounded.
pub fn behind_words(cushion: u64, output_block: u64, resampler_delay: u64, rate: u32) -> String {
    let frames = cushion + output_block + resampler_delay;
    let ms = (frames * 1000 + u64::from(rate) / 2) / u64::from(rate.max(1));
    format!("live, about {ms} ms behind")
}

/// `device_frames` at `device_rate` as frames at `project_rate`, rounded up: a device block as
/// the resampled block that reaches the monitor.
pub fn project_frames(device_frames: u32, device_rate: u32, project_rate: u32) -> u32 {
    let frames = (u64::from(device_frames) * u64::from(project_rate))
        .div_ceil(u64::from(device_rate.max(1)));
    u32::try_from(frames).unwrap_or(u32::MAX)
}

/// The cushion a monitor starts with for an input block of `input_block` frames (the output's
/// when the device chose its own) and an output block of `output_block`: one of each.
pub fn first_cushion(input_block: Option<u32>, output_block: u64) -> u64 {
    input_block.map_or(output_block, u64::from) + output_block
}

/// The channels a picker offers on a device of `count` channels: each mono channel, then each
/// adjacent pair as stereo, the first pair first.
pub fn channel_choices(count: u16) -> Vec<InputChannels> {
    let count = count.max(1);
    let mut choices: Vec<InputChannels> = (0..count.saturating_sub(1))
        .step_by(2)
        .map(|left| InputChannels::Stereo(left, left + 1))
        .collect();
    choices.extend((0..count).map(InputChannels::Mono));
    choices
}

/// How the window names `channels`: "1+2 (stereo)", "3 (mono)", counted from 1.
pub fn channels_words(channels: InputChannels) -> String {
    match channels {
        InputChannels::Stereo(left, right) => {
            format!("{}+{} (stereo)", u32::from(left) + 1, u32::from(right) + 1)
        }
        InputChannels::Mono(channel) => format!("{} (mono)", u32::from(channel) + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A device that offers the project's rate opens at it; one that does not
    // opens at its own default (or the offered rate nearest it) and is resampled, as the output
    // is; only a device offering no float input at all is refused, in one short line.
    #[test]
    fn an_input_opens_at_the_project_rate_else_at_its_own() {
        assert_eq!(
            input_rate("USB", 44_100, &[44_100..=96_000], 48_000),
            Ok(44_100)
        );
        assert_eq!(
            input_rate("USB", 44_100, &[48_000..=48_000], 48_000),
            Ok(48_000)
        );
        assert_eq!(
            input_rate("USB", 44_100, &[96_000..=96_000], 48_000),
            Ok(96_000)
        );
        assert_eq!(
            input_rate("", 44_100, &[], 48_000),
            Err("System default input offers no 32-bit float input".to_owned())
        );
    }

    /// Everything `bridge` gives for `samples` pushed in pieces of `split` frames.
    fn through(
        bridge: &mut InputBridge,
        samples: &[f32],
        channels: usize,
        split: usize,
    ) -> Vec<f32> {
        let mut out = Vec::new();
        for piece in samples.chunks(split * channels) {
            bridge.push(piece, |frames| out.extend_from_slice(frames));
        }
        out
    }

    // The delay the bridge reports is where a sound comes out: a click at the device's frame 0
    // is loudest within a frame of it at the project's rate (the resampler's filter is
    // centered there; its tails are never exactly zero, so "the first non-zero frame" would be
    // frame 0). 48000 → 44100 and 44100 → 48000 in stereo, and an interface's four channels
    // whose last two are silent and stay so.
    #[test]
    fn the_input_bridge_puts_a_click_at_the_delay_it_reports() {
        for (from, to, channels) in [
            (48_000, 44_100, 2),
            (44_100, 48_000, 2),
            (48_000, 44_100, 4),
        ] {
            let mut bridge = InputBridge::new(from, to, channels).expect("a bridge");
            let delay = bridge.delay();
            let mut input = vec![0.0f32; channels * 8192];
            input[0] = 1.0;
            input[1] = 1.0;
            let out = through(&mut bridge, &input, channels, 8192);
            let left: Vec<f32> = out.iter().step_by(channels).copied().collect();
            let (loudest, peak) = left
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                .map(|(at, value)| (at as u64, *value))
                .expect("frames came out");
            println!("{from} -> {to}: delay {delay}, loudest at {loudest} ({peak})");
            assert!(
                loudest.abs_diff(delay) <= 1,
                "{from} -> {to}: loudest at {loudest}, delay {delay}"
            );
            assert!(peak > 0.5, "the click came through: {peak}");
            if channels == 4 {
                assert!(
                    out.chunks(4)
                        .all(|frame| frame[2] == 0.0 && frame[3] == 0.0),
                    "silent channels stay silent"
                );
            }
        }
    }

    // Device buffers of any size give the same resampled frames, bit for bit: the bridge only
    // ever resamples whole blocks (1, 7, 64 and 4096 frames against one buffer of all).
    #[test]
    fn the_input_bridge_gives_the_same_bits_for_any_buffer_size() {
        let tone: Vec<f32> = (0..20_000)
            .flat_map(|n| {
                let phase = 2.0 * std::f64::consts::PI * 440.0 * f64::from(n) / 48_000.0;
                let value = (0.5 * phase.sin()) as f32;
                [value, -value]
            })
            .collect();
        let bits = |split: usize| {
            let mut bridge = InputBridge::new(48_000, 44_100, 2).expect("a bridge");
            through(&mut bridge, &tone, 2, split)
                .into_iter()
                .map(f32::to_bits)
                .collect::<Vec<_>>()
        };
        let whole = bits(20_000);
        assert!(whole.len() > 2 * 18_000, "{} values", whole.len());
        for split in [1, 7, 64, 4096] {
            assert!(bits(split) == whole, "buffers of {split} frames differ");
        }
    }

    // A two-channel device offers its pair, then each channel; four channels two pairs.
    #[test]
    fn the_channel_picker_offers_pairs_then_single_channels() {
        assert_eq!(
            channel_choices(2),
            [
                InputChannels::Stereo(0, 1),
                InputChannels::Mono(0),
                InputChannels::Mono(1)
            ]
        );
        assert_eq!(channel_choices(4).len(), 2 + 4);
        assert_eq!(channel_choices(1), [InputChannels::Mono(0)]);
        assert_eq!(channels_words(InputChannels::Stereo(2, 3)), "3+4 (stereo)");
        assert_eq!(channels_words(InputChannels::Mono(0)), "1 (mono)");
    }

    /// A late device callback: one in `every`, delivered `frames` output frames' time late
    /// (a seeded pick in that range).
    #[derive(Clone, Copy)]
    struct Late {
        every: u64,
        frames: (u64, u64),
    }

    /// A burst simulation of late device buffers, made deterministic and run at a
    /// tenth of the rates: a mono device at 4800 Hz delivering seeded callbacks of 1024, 1536
    /// or 2048 frames (PipeWire's quanta) to one input channel's writers, the player reading
    /// 512-frame blocks at 4410 Hz, both on one simulated clock, `seconds` long. `bridged`: the
    /// frames reach the monitor through the resampler, as a 48 kHz device's do on a 44.1 kHz
    /// project; else each callback is one push of the same span at the project's rate (its
    /// frames counted exactly: floor(frames so far · 4410 / 4800) less the frames before), a
    /// device at the project's rate bringing the same bursts at the same times. The device's
    /// clock runs `slow_ppm` slow; `late` callbacks come late, and the ones due meanwhile come
    /// at once after them. The monitor's dry spells and the frames it dropped.
    fn burst_run(
        bridged: bool,
        seconds: f64,
        slow_ppm: f64,
        late: Option<Late>,
        seed: u64,
    ) -> (u64, u64) {
        use dawsome_engine::input::InputFeed;
        use dawsome_media::record::{monitor_ring, picked_ring, Pacing};
        const PROJECT: u32 = 4410;
        const DEVICE: u32 = 4800;
        let pacing = Pacing {
            output_block: 512,
            rate: PROJECT,
        };
        let picked = InputChannels::Mono(0);
        let (monitor, mut feed, _) = monitor_ring(picked, 4 * PROJECT as usize, Some(pacing));
        let (record, _record_feed, _) = picked_ring(picked, DEVICE, 1);
        let mut writers = DeviceWriters {
            bridge: bridged.then(|| InputBridge::new(DEVICE, PROJECT, 1).expect("a bridge")),
            writers: vec![(monitor, record)],
        };
        let stats = feed.stats();
        let mut seed = seed;
        let mut next = move |below: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % below
        };
        let buffer = vec![0.25f32; 2048];
        let mut out = [[0.0f32; 2]; 512];
        let device_rate = f64::from(DEVICE) * (1.0 - slow_ppm * 1e-6);
        // When a callback of `size` frames after `captured` comes, `late` output frames late,
        // never before the one `delivered` last.
        let due = |captured: u64, size: u64, late: u64, delivered: f64| {
            let on_time = (captured + size) as f64 / device_rate;
            (on_time + late as f64 / f64::from(PROJECT)).max(delivered)
        };
        let late_by = |next: &mut dyn FnMut(u64) -> u64| {
            late.filter(|late| next(late.every) == 0).map_or(0, |late| {
                late.frames.0 + next(late.frames.1 - late.frames.0 + 1)
            })
        };
        let at_project = |frames: u64| frames * u64::from(PROJECT) / u64::from(DEVICE);
        let (mut captured, mut delivered) = (0u64, 0.0f64);
        let mut size = 1024 * (2 + next(3)) / 2;
        let mut callback = due(captured, size, late_by(&mut next), delivered);
        let mut reads = 0u64;
        let end = seconds * f64::from(PROJECT) / 512.0;
        while (reads as f64) < end {
            let read = reads as f64 * 512.0 / f64::from(PROJECT);
            if callback < read {
                let frames = if bridged {
                    size
                } else {
                    at_project(captured + size) - at_project(captured)
                };
                writers.push(&buffer[..frames as usize], 1, None);
                captured += size;
                delivered = callback;
                size = 1024 * (2 + next(3)) / 2;
                let late = late_by(&mut next);
                callback = due(captured, size, late, delivered);
            } else {
                feed.read(&mut out);
                reads += 1;
            }
        }
        let count = |count: &std::sync::atomic::AtomicU64| count.load(Ordering::Relaxed);
        (count(&stats.underruns), count(&stats.skipped))
    }

    /// A burst run's dry spells and frames dropped, direct then bridged.
    type Runs = Vec<((u64, u64), (u64, u64))>;

    /// [`burst_run`] over `seeds`, bridged and direct: each run's dry spells and frames
    /// dropped, (direct, bridged).
    fn burst_runs(seconds: f64, slow_ppm: f64, late: Option<Late>, seeds: &[u64]) -> Runs {
        seeds
            .iter()
            .map(|&seed| {
                let direct = burst_run(false, seconds, slow_ppm, late, seed);
                let bridged = burst_run(true, seconds, slow_ppm, late, seed);
                println!("seed {seed}: direct {direct:?}, bridged {bridged:?} (dry, dropped)");
                (direct, bridged)
            })
            .collect()
    }

    /// The runs' dry spells and dropped frames summed: direct, then bridged.
    fn summed(runs: &Runs) -> [u64; 4] {
        runs.iter().fold([0; 4], |sum, (direct, bridged)| {
            [
                sum[0] + direct.0,
                sum[1] + direct.1,
                sum[2] + bridged.0,
                sum[3] + bridged.1,
            ]
        })
    }

    const SEEDS: [u64; 4] = [0x5eed, 1, 2, 3];

    // Through the resampler a device callback of 2048 frames reached the
    // monitor as pushes of one resampler block each (294 or 588 frames), so the monitor took
    // one block for the input's, kept too short a cushion and too low a ceiling for a
    // callback's burst, dropped the excess and then ran dry, with matched clocks too. Now the
    // callback's span at the project's rate is the block. With matched clocks the resampled input does no worse, seed by seed, than a
    // device at the project's rate bringing the same bursts (before the fix, over six seeds,
    // 34 dry spells and 54215 frames dropped against 5 and 15501; the direct device's are the
    // monitor's own start: a dry spell in its first second as it grows its cushion).
    #[test]
    fn a_resampled_burst_does_no_worse_than_a_direct_one_with_matched_clocks() {
        let runs = burst_runs(300.0, 0.0, None, &SEEDS);
        for ((direct, bridged), seed) in runs.iter().zip(SEEDS) {
            assert!(
                bridged.0 <= direct.0 && bridged.1 <= direct.1,
                "seed {seed}: {bridged:?} against {direct:?}"
            );
        }
    }

    // With one callback in 100 late by 600 to 1500 frames: the resampled input within
    // the seeds' spread of the direct one, summed over four seeds (at most 1.5 times its dry
    // spells and dropped frames; before the fix, over six seeds, 88 dry spells and 166218
    // frames against 18 and 42424). Why one in 100 and not one in 500: at this
    // length that is about four late callbacks a run, too few to compare (six seeds: 14 dry
    // spells against 10; at three times the length 12 seeds give 44 against 41), and the
    // resampler's sub-blocks (above) add up to 235 frames of their own unevenness.
    #[test]
    fn a_resampled_burst_does_no_worse_than_a_direct_one_with_late_callbacks() {
        let late = Late {
            every: 100,
            frames: (600, 1500),
        };
        let [dry, dropped, bridged_dry, bridged_dropped] =
            summed(&burst_runs(600.0, 0.0, Some(late), &SEEDS));
        println!("summed: direct {dry} / {dropped}, bridged {bridged_dry} / {bridged_dropped}");
        assert!(
            2 * bridged_dry <= 3 * dry,
            "{bridged_dry} dry spells against {dry}"
        );
        assert!(
            2 * bridged_dropped <= 3 * dropped,
            "{bridged_dropped} dropped against {dropped}"
        );
    }

    // With the device's clock slow (30 ppm for 2 hours at 44.1 kHz, scaled:
    // at a tenth of the rate and a tenth of the time, 3000 ppm drifts as many blocks): the
    // resampled input within the seeds' spread of the direct one, summed over four seeds
    // (before the fix, over six seeds at 1500 ppm for 24 min, 64 dry spells and 109821 frames
    // against 20 and 50347).
    #[test]
    fn a_resampled_burst_does_no_worse_than_a_direct_one_with_a_slow_clock() {
        let [dry, dropped, bridged_dry, bridged_dropped] =
            summed(&burst_runs(720.0, 3000.0, None, &SEEDS));
        println!("summed: direct {dry} / {dropped}, bridged {bridged_dry} / {bridged_dropped}");
        assert!(
            2 * bridged_dry <= 3 * dry,
            "{bridged_dry} dry spells against {dry}"
        );
        assert!(
            2 * bridged_dropped <= 3 * dropped,
            "{bridged_dropped} dropped against {dropped}"
        );
    }

    // The input's block as the monitor counts it: a device buffer's span at the project's
    // rate, rounded up (ceil(2048 · 44100 / 48000) = 1882, ceil(1024 · …) = 941), and for a
    // buffer shorter than a resampler step (ceil(512 · …) = 471, ceil(64 · …) = 59) the most a
    // step gives, 588: its frames still come a step at a time.
    #[test]
    fn a_device_buffer_counts_as_its_span_at_the_project_rate() {
        let bridge = InputBridge::new(48_000, 44_100, 2).expect("a bridge");
        assert_eq!(
            [2048, 1024, 512, 64].map(|frames| bridge.project_frames(frames)),
            [1882, 941, 471, 59]
        );
        assert_eq!(
            [2048, 1024, 512, 64].map(|frames| bridge.block_of(frames)),
            [1882, 941, 588, 588]
        );
    }

    // 512-frame blocks at 48 kHz: a cushion of 1024 frames and a 512-frame output block are
    // 1536 frames, 32 ms. Resampled to 44.1 kHz: a 512-frame device block is 471 project frames
    // (ceil(512 · 44100 / 48000)), a cushion of 983, and with the output block and a 147-frame
    // resampler delay 1642 frames, 37.2 ms, said 37.
    #[test]
    fn the_delay_words_count_the_cushion_the_output_block_and_the_resampler() {
        assert_eq!(first_cushion(Some(512), 512), 1024);
        assert_eq!(first_cushion(None, 256), 512);
        assert_eq!(
            behind_words(1024, 512, 0, 48_000),
            "live, about 32 ms behind"
        );
        assert_eq!(project_frames(512, 48_000, 44_100), 471);
        assert_eq!(first_cushion(Some(471), 512), 983);
        assert_eq!(
            behind_words(983, 512, 147, 44_100),
            "live, about 37 ms behind"
        );
    }

    // The stand-ins for the default (and programs' streams) are not offered as devices.
    #[test]
    fn stand_ins_are_not_input_devices() {
        assert!(!is_input_device("default_source"));
        assert!(!is_input_device("pipewire"));
        assert!(is_input_device("USB Audio Line Input"));
    }
}
