//! Live output: one cpal stream on the output device the settings choose (the system's default
//! unless one is chosen). Its callback fills each buffer from the engine's player and does
//! nothing else, so it never allocates, locks or logs.
//!
//! The stream runs at the project's sample rate when the device offers it, and the callback
//! hands its buffer straight to [`Player::process`]. Otherwise the stream runs at the device's
//! own rate and a [`RateBridge`] sits between: the engine still renders at the project's rate,
//! and its output is resampled to the device's.
//!
//! A stream that cannot open, or that fails, hands its player back; a [`Parked`] player then
//! runs without a device, so the plugins in it stay alive.

use std::any::Any;
use std::fmt;
use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicU8, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, ErrorKind, SampleFormat, StreamConfig, SupportedBufferSize, SupportedStreamConfig,
    SupportedStreamConfigRange,
};
use dawsome_engine::realtime::Player;
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, Fft, FixedAsync, FixedSync, Resampler, ResamplerConstructionError,
    SincInterpolationParameters,
};
use thiserror::Error;

/// Frames per device buffer asked for, and frames per engine block behind a resampler: the
/// block size of an offline render, so live playback splits the timeline at the same places
/// (controls and plan swaps land on block starts).
pub(crate) const BUFFER_FRAMES: u32 = 512;

/// [`BUFFER_FRAMES`] as a slice length.
const BLOCK_FRAMES: usize = BUFFER_FRAMES as usize;

/// The longest resampler delay a bridge accepts, in output frames (43 ms at 48 kHz). Why a
/// bound: the FFT resampler's block is the whole reduced rate ratio, so a project rate sharing
/// no factor with the device's (44101 or 47999 Hz on a 48 kHz device) would delay the sound by
/// 24000 frames, half a second after Play.
const MAX_DELAY_FRAMES: usize = 2048;

/// How long opening the stream may take before it counts as failed.
const OPEN_TIMEOUT: Duration = Duration::from_secs(2);

/// How long a stream that goes may take to hand its player back.
const RETURN_TIMEOUT: Duration = Duration::from_millis(500);

/// How long the UI waits for a parked player to take a plan update.
const CATCH_UP_TIMEOUT: Duration = Duration::from_millis(200);

/// Why a stream did not start, and the player it was to play, handed back (`None` only if the
/// audio backend kept the callback past the failure).
pub struct StartFailure {
    pub error: StartError,
    pub player: Option<Box<Player>>,
}

/// Why there is no live output.
#[derive(Debug, Error)]
pub enum StartError {
    #[error("no audio output device")]
    NoDevice,
    #[error("the output device offers no 32-bit float stereo stream")]
    NoFloatStereo,
    #[error("the output device cannot play {rate} Hz, the rate --device-rate asks for")]
    RateNotOffered { rate: u32 },
    #[error("cannot resample to the device's rate: {0}")]
    Resampler(#[from] ResamplerConstructionError),
    #[error("cannot open the output device: {0}")]
    Device(#[from] cpal::Error),
    #[error("the output device {name} is not there; choose another in Settings")]
    DeviceGone { name: String },
}

/// What the settings keep for a sound server's sink ahead of its node name: `sink:` and the
/// node's name (`sink:alsa_output.usb-SteelSeries_Arctis_Nova_7-00.analog-stereo`), where every
/// other device is kept by the name it is listed by. Why the node: its description is what
/// Settings shows, and it changes with the desktop's language, a rename or a second device of the
/// same kind, where the node's name stays; why a prefix: a choice saved by name before still
/// means what it meant.
pub const SINK: &str = "sink:";

/// An output device as Settings lists it: what the settings keep for it ([`SINK`] and a sink's
/// node name, else its name) and the name Settings shows (a sink's description, a device's own
/// name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedOutput {
    pub id: String,
    pub name: String,
}

impl ListedOutput {
    /// A device kept by the name it is listed by (Windows' devices, ALSA's, a test's).
    pub fn named(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            id: name.clone(),
            name,
        }
    }

    /// Whether the settings' `kept` value names this device: by what they keep for it now, or by
    /// the name it is listed by, as a choice saved before sinks were kept by node does.
    pub fn is(&self, kept: &str) -> bool {
        self.id == kept || self.name == kept
    }
}

/// What the output device list is read from: the output devices there are. Only output devices
/// are asked for, so listing them opens no input.
pub trait OutputLister {
    fn outputs(&self) -> Result<Vec<ListedOutput>, String>;
}

/// Names already listed, each kept by its name.
impl OutputLister for [String] {
    fn outputs(&self) -> Result<Vec<ListedOutput>, String> {
        Ok(self.iter().map(ListedOutput::named).collect())
    }
}

/// Devices already listed.
impl OutputLister for [ListedOutput] {
    fn outputs(&self) -> Result<Vec<ListedOutput>, String> {
        Ok(self.to_vec())
    }
}

/// The system's output devices, as cpal lists them, without the entries that are no device
/// ([`listed_output`]). No test runs it: a headless run lists none. `dawsome-app
/// --list-outputs` prints what it finds.
pub struct SystemOutputs;

impl OutputLister for SystemOutputs {
    fn outputs(&self) -> Result<Vec<ListedOutput>, String> {
        let devices = cpal::default_host()
            .output_devices()
            .map_err(|error| error.to_string())?;
        Ok(devices
            .filter_map(|device| listed_output(device.to_string(), device.id().ok().as_ref()))
            .collect())
    }
}

/// Names cpal lists among the outputs that stand for the system's default output, which
/// "System default" already offers: the PipeWire host's two ("default_sink", "default_output")
/// and ALSA's PCM ids ("default", "sysdefault", "pulse", "pipewire"), which cpal 0.17 listed
/// as their names (0.18 lists ALSA's PCMs by description and [`ALSA_PLUGINS`] leaves them out
/// by id).
const DEFAULT_STAND_INS: [&str; 6] = [
    "default_sink",
    "default_output",
    "default",
    "sysdefault",
    "pulse",
    "pipewire",
];

/// The name the PipeWire host gives a node without a description: another program's playback
/// stream (and this one's while it plays), which cpal lists as an output.
const STREAM_NAME: &str = "unknown";

/// Whether `name`, as cpal lists it among the outputs, is an output device Settings offers:
/// neither a stand-in for the system default nor a program's stream. Why by name: the names
/// are what cpal gives on every host, and these are the ones it gives for no device.
pub fn is_output_device(name: &str) -> bool {
    name != STREAM_NAME && !DEFAULT_STAND_INS.contains(&name)
}

/// ALSA's PCMs that are no output of their own, by id, as cpal's ALSA host lists them when no
/// sound server runs: the default and the sound servers' PCMs (which "System default" stands
/// for), the null sink, rate converters, the Speex and channel-mixing plugins, OSS emulation.
/// Each card's own PCMs stay ("sysdefault:CARD=…", "front:…", "hdmi:…", "hw:…"). "jack" stays
/// too: with JACK running and no PipeWire it is the way into JACK, and when JACK is not running
/// it says so as it fails to open.
#[cfg(target_os = "linux")]
const ALSA_PLUGINS: [&str; 12] = [
    "default",
    "sysdefault",
    "pulse",
    "pipewire",
    "null",
    "lavrate",
    "samplerate",
    "speexrate",
    "speex",
    "upmix",
    "vdownmix",
    "oss",
];

/// The device cpal lists among the outputs as `name`, with the id it gives it, as Settings lists
/// it; none when it is no output Settings offers ([`is_output_device`], [`ALSA_PLUGINS`], ALSA's
/// "usbstream:" PCMs, which play no audio). A PipeWire node is kept by its node name
/// ([`SINK`]), every other device by its name.
pub fn listed_output(name: String, id: Option<&cpal::DeviceId>) -> Option<ListedOutput> {
    if !is_output_device(&name) || id.is_some_and(is_alsa_plugin) {
        return None;
    }
    if let Some(node) = id.and_then(pipewire_node) {
        return Some(ListedOutput {
            id: format!("{SINK}{node}"),
            name,
        });
    }
    let card = id
        .and_then(alsa_default_card)
        .filter(|_| name == ALSA_DEFAULT_NAME);
    Some(ListedOutput::named(card.map_or(name, str::to_owned)))
}

/// The name cpal's ALSA host gives a card's default PCM ("sysdefault:CARD=…") whose card has no
/// description of its own, as the bare "sysdefault" is named too: such a PCM is listed by its
/// card ("Oriole"), so no row but the first reads like the system's default.
const ALSA_DEFAULT_NAME: &str = "Default Audio Device";

/// The card of ALSA's `sysdefault:CARD=<card>` PCM, when `id` is that.
#[cfg(target_os = "linux")]
fn alsa_default_card(id: &cpal::DeviceId) -> Option<&str> {
    if id.host() != cpal::HostId::Alsa {
        return None;
    }
    let card = id.id().strip_prefix("sysdefault:CARD=")?;
    Some(card.split(',').next().unwrap_or(card))
}

/// No ALSA outside Linux.
#[cfg(not(target_os = "linux"))]
fn alsa_default_card(_: &cpal::DeviceId) -> Option<&str> {
    None
}

/// The PipeWire node `id` names, when it is one of the PipeWire host's.
#[cfg(target_os = "linux")]
fn pipewire_node(id: &cpal::DeviceId) -> Option<&str> {
    (id.host() == cpal::HostId::PipeWire).then(|| id.id())
}

/// No PipeWire host outside Linux.
#[cfg(not(target_os = "linux"))]
fn pipewire_node(_: &cpal::DeviceId) -> Option<&str> {
    None
}

/// Whether `id` is one of ALSA's PCMs that is no output of its own ([`ALSA_PLUGINS`]).
#[cfg(target_os = "linux")]
fn is_alsa_plugin(id: &cpal::DeviceId) -> bool {
    id.host() == cpal::HostId::Alsa
        && (ALSA_PLUGINS.contains(&id.id()) || id.id().starts_with("usbstream:"))
}

/// No ALSA outside Linux.
#[cfg(not(target_os = "linux"))]
fn is_alsa_plugin(_: &cpal::DeviceId) -> bool {
    false
}

/// An output device the settings chose: what they keep for it (its listed id) and the name to
/// call it by while it is not there (the name it was listed by when chosen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chosen<'a> {
    pub id: &'a str,
    pub name: &'a str,
}

impl<'a> Chosen<'a> {
    /// The choice kept as `id`, listed as `name` when it was chosen; without a name (a choice
    /// kept by its name, or saved before names were kept) the id, a sink's without [`SINK`].
    pub fn new(id: &'a str, name: Option<&'a str>) -> Self {
        let name = name.unwrap_or_else(|| id.strip_prefix(SINK).unwrap_or(id));
        Self { id, name }
    }
}

/// An output device Settings offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputChoice {
    /// What the settings keep when it is chosen; none for the system's default.
    pub id: Option<String>,
    /// The name it shows; none for the system's default.
    pub name: Option<String>,
    /// The chosen device, listed though it is not there now.
    pub missing: bool,
    /// Whether it is the one chosen.
    pub chosen: bool,
}

/// What Settings offers to play through: the system's default first, then each device `lister`
/// lists, once by name, in its order; a `chosen` device it does not list last, marked missing,
/// so the choice still shows. The chosen one is marked: the first whose id the settings keep,
/// else the first listed by that name (a choice saved by name). A list that fails offers the
/// default (and the chosen one); the error comes back beside it.
pub fn output_choices(
    lister: &(impl OutputLister + ?Sized),
    chosen: Option<Chosen<'_>>,
) -> (Vec<OutputChoice>, Option<String>) {
    let (listed, problem) = match lister.outputs() {
        Ok(listed) => (listed, None),
        Err(problem) => (Vec::new(), Some(problem)),
    };
    let mut devices: Vec<ListedOutput> = Vec::new();
    for device in listed {
        if !devices.iter().any(|kept| kept.name == device.name) {
            devices.push(device);
        }
    }
    let marked = chosen.and_then(|chosen| {
        devices
            .iter()
            .position(|device| device.id == chosen.id)
            .or_else(|| devices.iter().position(|device| device.name == chosen.id))
    });
    let mut choices = vec![OutputChoice {
        id: None,
        name: None,
        missing: false,
        chosen: chosen.is_none(),
    }];
    choices.extend(
        devices
            .into_iter()
            .enumerate()
            .map(|(index, device)| OutputChoice {
                id: Some(device.id),
                name: Some(device.name),
                missing: false,
                chosen: marked == Some(index),
            }),
    );
    if let (Some(chosen), None) = (chosen, marked) {
        choices.push(OutputChoice {
            id: Some(chosen.id.to_owned()),
            name: Some(chosen.name.to_owned()),
            missing: true,
            chosen: true,
        });
    }
    (choices, problem)
}

/// What `dawsome-app --list-outputs` prints: each row Settings would show, a line each, the
/// chosen one marked "*", then a tab and what the settings keep when it is chosen (nothing for
/// the system's default); and why the list is short, when it is.
pub fn listing(choices: &[OutputChoice], problem: Option<&str>) -> String {
    let mut text = String::new();
    for choice in choices {
        let mark = if choice.chosen { '*' } else { ' ' };
        let name = choice.name.as_deref().unwrap_or("System default");
        let missing = if choice.missing {
            " (not plugged in)"
        } else {
            ""
        };
        let id = choice.id.as_deref().unwrap_or_default();
        text.push_str(&format!("{mark} {name}{missing}\t{id}\n"));
    }
    if let Some(problem) = problem {
        text.push_str(&format!("could not list the devices: {problem}\n"));
    }
    text
}

/// The name a stream's device is shown by: the device's own (a sink's description), or "System
/// default" for a stand-in (the PipeWire host's default device is "default_output", which
/// follows the sound server's default sink).
pub fn shown_device(name: &str) -> &str {
    if is_output_device(name) {
        name
    } else {
        "System default"
    }
}

/// The line Settings shows while a stream plays: "Playing through: <device>, 48000 Hz, 512
/// frames"; the frames are the buffer size asked for, else the largest the device has given,
/// else unsaid until it gives one.
pub fn playing_through(device: &str, rate: u32, frames: Option<u64>) -> String {
    match frames {
        Some(frames) => format!("Playing through: {device}, {rate} Hz, {frames} frames"),
        None => format!("Playing through: {device}, {rate} Hz"),
    }
}

/// The rate a stream runs at, measured against the project's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RatePlan {
    /// The device plays the project's rate; nothing is resampled.
    Direct,
    /// The device plays `device_rate`, and the engine's output is resampled to it.
    Resample { device_rate: u32 },
}

/// Picks the stream's rate from the device's 32-bit float stereo rate ranges: the project's own
/// rate when a range holds it, else the device's default rate (or the offered rate nearest to
/// it). `forced` (from `--device-rate`) overrides both and must be offered.
pub fn choose_rate(
    project_rate: u32,
    offered: &[RangeInclusive<u32>],
    device_default: u32,
    forced: Option<u32>,
) -> Result<RatePlan, StartError> {
    let plan = |rate| {
        if rate == project_rate {
            RatePlan::Direct
        } else {
            RatePlan::Resample { device_rate: rate }
        }
    };
    if offered.is_empty() {
        return Err(StartError::NoFloatStereo);
    }
    if let Some(rate) = forced {
        return if offered.iter().any(|range| range.contains(&rate)) {
            Ok(plan(rate))
        } else {
            Err(StartError::RateNotOffered { rate })
        };
    }
    if offered.iter().any(|range| range.contains(&project_rate)) {
        return Ok(RatePlan::Direct);
    }
    let nearest = offered
        .iter()
        .map(|range| device_default.clamp(*range.start(), *range.end()))
        .min_by_key(|rate| rate.abs_diff(device_default))
        .unwrap_or(device_default);
    Ok(plan(nearest))
}

/// The rate a stream on a device plays at when the project can take any rate: `forced` (from
/// `--device-rate`) when given, else the device's default rate when its 32-bit float stereo
/// ranges hold it, else the offered rate nearest to it ([`choose_rate`]'s rule for a project at
/// the device's default rate). What a new document takes, so nothing is resampled on the way
/// out and an input device on the same audio server opens as it is.
pub fn own_rate(
    offered: &[RangeInclusive<u32>],
    device_default: u32,
    forced: Option<u32>,
) -> Result<u32, StartError> {
    Ok(
        match choose_rate(device_default, offered, device_default, forced)? {
            RatePlan::Direct => device_default,
            RatePlan::Resample { device_rate } => device_rate,
        },
    )
}

/// The rate the output device the settings keep as `id` (the system's default when none) plays
/// at on its own ([`own_rate`]), read from its configurations without opening a stream: what a
/// new window's first document takes before its stream opens. `None` when there is no such
/// device or it offers no 32-bit float stereo. No test runs it.
pub fn device_own_rate(forced: Option<u32>, id: Option<&str>) -> Option<u32> {
    let device = output_device(id.map(|id| Chosen::new(id, None))).ok()?;
    let (default, float_stereo) = float_stereo(&device).ok()?;
    own_rate(&offered_rates(&float_stereo), default.sample_rate(), forced).ok()
}

/// The engine's output resampled to the device's rate. The player renders [`BLOCK_FRAMES`]
/// project-rate frames at a time, the same blocks `--play-to` renders; each block is resampled
/// (FFT resampler, or a sinc one where the FFT's delay would pass [`MAX_DELAY_FRAMES`]; fixed
/// input size) into a queue that device buffers of any size are filled from, so the output
/// does not depend on how the device sizes its buffers.
///
/// Every buffer is sized when it is built, on the UI thread; [`fill`](Self::fill) never
/// allocates, frees, locks or logs. The cost is latency: the resampler's output delay plus up
/// to one block. The playhead the player publishes counts project samples, so it runs that far
/// ahead of the sound.
pub struct RateBridge {
    player: Player,
    resampler: Box<dyn Resampler<f32>>,
    /// One engine block.
    block: Box<[[f32; 2]]>,
    /// Resampled frames; `queue[read..read + held]` are still to go to the device.
    queue: Box<[[f32; 2]]>,
    read: usize,
    held: usize,
}

/// The resampler a bridge from `from` Hz to `to` Hz runs over `channels` interleaved channels,
/// [`BLOCK_FRAMES`] input frames a step: the output's (project to device, stereo) and an input
/// device's (device to project, the device's channels, [`crate::live_input::InputBridge`]).
/// One function, so the two directions resample alike.
pub(crate) fn bridge_resampler(
    from: u32,
    to: u32,
    channels: usize,
) -> Result<Box<dyn Resampler<f32>>, ResamplerConstructionError> {
    let fft = Fft::new(
        from as usize,
        to as usize,
        BLOCK_FRAMES,
        channels,
        FixedSync::Input,
    )?;
    if fft.output_delay() <= MAX_DELAY_FRAMES {
        return Ok(Box::new(fft));
    }
    // Why sinc, not an FFT on a nearby ratio with a shorter block: any ratio but the device's
    // own drifts away from its clock without end. The sinc resampler runs that ratio, delayed
    // by half its filter (128 input frames).
    Ok(Box::new(Async::new_sinc(
        f64::from(to) / f64::from(from),
        1.0,
        &SincInterpolationParameters::default(),
        BLOCK_FRAMES,
        channels,
        FixedAsync::Input,
    )?))
}

impl RateBridge {
    /// A bridge from `player` at `project_rate` to a device at `device_rate`.
    #[cfg(test)]
    pub fn new(
        player: Player,
        project_rate: u32,
        device_rate: u32,
    ) -> Result<Self, ResamplerConstructionError> {
        Ok(Self::with_resampler(
            player,
            bridge_resampler(project_rate, device_rate, 2)?,
        ))
    }

    /// A bridge from `player` through `resampler` (see [`bridge_resampler`]).
    fn with_resampler(player: Player, resampler: Box<dyn Resampler<f32>>) -> Self {
        let capacity = resampler.output_frames_max();
        Self {
            player,
            resampler,
            block: vec![[0.0; 2]; BLOCK_FRAMES].into_boxed_slice(),
            queue: vec![[0.0; 2]; capacity].into_boxed_slice(),
            read: 0,
            held: 0,
        }
    }

    /// Frames from a sound entering the resampler to it leaving, at the device's rate.
    #[cfg(test)]
    pub fn output_delay(&self) -> usize {
        self.resampler.output_delay()
    }

    /// Fills `out` with the next resampled frames, rendering and resampling engine blocks as
    /// the queue runs dry.
    pub fn fill(&mut self, out: &mut [[f32; 2]]) {
        let mut done = 0;
        while done < out.len() {
            if self.held == 0 && !self.refill() {
                out[done..].fill([0.0; 2]);
                return;
            }
            let count = self.held.min(out.len() - done);
            out[done..done + count].copy_from_slice(&self.queue[self.read..self.read + count]);
            done += count;
            self.read += count;
            self.held -= count;
        }
    }

    /// Renders one engine block and resamples it into the queue. Returns false when the
    /// resampler refused the buffers. Why that cannot happen and is still handled: every size
    /// comes from the resampler itself at construction; if it ever did, the device plays
    /// silence rather than stale frames.
    fn refill(&mut self) -> bool {
        self.player.process(&mut self.block);
        let capacity = self.queue.len();
        let input = InterleavedSlice::new(self.block.as_flattened(), 2, BLOCK_FRAMES);
        let output = InterleavedSlice::new_mut(self.queue.as_flattened_mut(), 2, capacity);
        let written = match (input, output) {
            (Ok(input), Ok(mut output)) => self
                .resampler
                .process_into_buffer(&input, &mut output, None)
                .ok()
                .map(|(_, written)| written),
            _ => None,
        };
        let Some(written) = written else {
            return false;
        };
        (self.read, self.held) = (0, written);
        true
    }
}

/// When the player's output frames play, on one clock every device stream can be read against:
/// nanoseconds since the clock was made. An output stream publishes it as it plays, and an
/// input stream reads it to line a recording up with what was heard. Why a clock of its own:
/// cpal times each stream on its own clock (on some hosts each starts at zero), so each stream's
/// instants are moved onto this one by an offset measured as the stream opens.
pub struct PlaybackClock {
    epoch: Instant,
    project_rate: u32,
    /// When the player's output frame 0 plays, or [`UNKNOWN`] while no stream plays it.
    origin: AtomicI64,
}

/// [`PlaybackClock::origin`] while no stream plays the player.
const UNKNOWN: i64 = i64::MIN;

const NANOS: i128 = 1_000_000_000;

impl PlaybackClock {
    pub fn new(project_rate: u32) -> Self {
        Self {
            epoch: Instant::now(),
            project_rate: project_rate.max(1),
            origin: AtomicI64::new(UNKNOWN),
        }
    }

    /// What to add to the nanoseconds of a stream's instants to read them on this clock,
    /// measured from `stream_now`, the stream's own reading of now.
    pub fn offset_of(&self, stream_now: cpal::StreamInstant) -> i64 {
        i64::try_from(self.now() - stream_nanos(stream_now)).unwrap_or(0)
    }

    /// Now, on this clock.
    pub fn now(&self) -> i128 {
        i128::try_from(self.epoch.elapsed().as_nanos()).unwrap_or(i128::MAX)
    }

    /// Notes that the player's output frame 0 plays at `origin` on this clock.
    pub fn publish(&self, origin: i128) {
        let origin = i64::try_from(origin).unwrap_or(UNKNOWN);
        self.origin.store(origin, Ordering::Relaxed);
    }

    /// No stream plays the player any more.
    pub fn forget(&self) {
        self.origin.store(UNKNOWN, Ordering::Relaxed);
    }

    /// When the player's output frame `frame` plays, on this clock, while a stream plays it.
    pub fn time_of(&self, frame: u64) -> Option<i128> {
        let origin = self.origin.load(Ordering::Relaxed);
        (origin != UNKNOWN)
            .then(|| i128::from(origin) + i128::from(frame) * NANOS / i128::from(self.project_rate))
    }
}

/// A stream instant in nanoseconds of its stream's clock.
pub fn stream_nanos(instant: cpal::StreamInstant) -> i128 {
    i128::try_from(instant.as_nanos()).unwrap_or(i128::MAX)
}

/// Which player frame a stream's device frames play: device frame `delay` (counted from the
/// stream's start) plays player frame `first`, and both run on at their rates.
#[derive(Debug, Clone, Copy)]
pub struct FrameTiming {
    /// The player's output frames when the stream started.
    pub first: u64,
    /// Device frames from a frame entering the resampler to it leaving (0 without one).
    pub delay: u64,
    pub device_rate: u32,
    pub project_rate: u32,
}

impl FrameTiming {
    /// When the player's output frame 0 plays, given that device frame `device_frame` plays at
    /// `playback` (nanoseconds on the playback clock).
    pub fn origin(&self, playback: i128, device_frame: u64) -> i128 {
        let (device, project) = (
            i128::from(self.device_rate.max(1)),
            i128::from(self.project_rate.max(1)),
        );
        let to_delay = i128::from(self.delay) - i128::from(device_frame);
        playback + to_delay * NANOS / device - i128::from(self.first) * NANOS / project
    }
}

/// Problems the stream reports after it started, as one atomic code the error callback can
/// raise without allocating. Higher codes win.
#[derive(Debug, Default)]
struct Health(AtomicU8);

const HEALTHY: u8 = 0;
const DROPOUT: u8 = 1;
const NORMAL_PRIORITY: u8 = 2;
const FAILED: u8 = 3;

impl Health {
    fn report(&self, kind: ErrorKind) {
        let code = match kind {
            // The backend recovers from an under- or overrun by itself and plays on.
            ErrorKind::Xrun => DROPOUT,
            ErrorKind::RealtimeDenied => NORMAL_PRIORITY,
            // The stream followed the default device to a new route and keeps playing.
            ErrorKind::DeviceChanged => HEALTHY,
            // Why every other kind counts as failed, not only a lost device, stream or host:
            // some backends stop the stream on others too (PipeWire deactivates it on a format
            // mismatch, reported as `UnsupportedConfig`), and calling that a glitch would hide
            // it.
            _ => FAILED,
        };
        self.0.fetch_max(code, Ordering::Relaxed);
    }
}

/// How long a stream's callbacks take to render, gathered as it plays: counters the callback
/// raises without allocating or locking (two clock reads per callback).
#[derive(Debug)]
pub struct CallbackStats {
    /// The stream's rate.
    rate: u32,
    callbacks: AtomicU64,
    busy_nanos: AtomicU64,
    worst_nanos: AtomicU64,
    /// The heaviest callback's time as a share of the time its buffer plays, in millionths.
    worst_load_ppm: AtomicU64,
    most_frames: AtomicU64,
}

impl CallbackStats {
    /// Nothing counted yet, for a stream at `rate` Hz.
    fn new(rate: u32) -> Self {
        Self {
            rate,
            callbacks: AtomicU64::new(0),
            busy_nanos: AtomicU64::new(0),
            worst_nanos: AtomicU64::new(0),
            worst_load_ppm: AtomicU64::new(0),
            most_frames: AtomicU64::new(0),
        }
    }

    /// Notes one callback that rendered `frames` frames in `took`.
    fn record(&self, took: Duration, frames: usize) {
        let nanos = u64::try_from(took.as_nanos()).unwrap_or(u64::MAX);
        self.callbacks.fetch_add(1, Ordering::Relaxed);
        self.busy_nanos.fetch_add(nanos, Ordering::Relaxed);
        self.worst_nanos.fetch_max(nanos, Ordering::Relaxed);
        let frames = frames as u64;
        self.most_frames.fetch_max(frames, Ordering::Relaxed);
        if frames > 0 {
            // nanos / (frames / rate · 10⁹) in millionths.
            let ppm = u128::from(nanos) * u128::from(self.rate) / (u128::from(frames) * 1000);
            let ppm = u64::try_from(ppm).unwrap_or(u64::MAX);
            self.worst_load_ppm.fetch_max(ppm, Ordering::Relaxed);
        }
    }
}

impl fmt::Display for CallbackStats {
    /// "audio callbacks: 1875, mean 41.0 µs, worst 230.0 µs (2.16 % of its buffer's time),
    /// buffers up to 512 frames at 48000 Hz".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let callbacks = self.callbacks.load(Ordering::Relaxed);
        if callbacks == 0 {
            return write!(f, "audio callbacks: none");
        }
        let micros = |nanos: u64| nanos as f64 / 1000.0;
        write!(
            f,
            "audio callbacks: {callbacks}, mean {:.1} µs, worst {:.1} µs ({:.2} % of its \
             buffer's time), buffers up to {} frames at {} Hz",
            micros(self.busy_nanos.load(Ordering::Relaxed) / callbacks),
            micros(self.worst_nanos.load(Ordering::Relaxed)),
            self.worst_load_ppm.load(Ordering::Relaxed) as f64 / 10_000.0,
            self.most_frames.load(Ordering::Relaxed),
            self.rate,
        )
    }
}

/// What a stream's callback renders with.
enum Render {
    Direct(Player),
    Bridge(RateBridge),
}

impl Render {
    fn fill(&mut self, frames: &mut [[f32; 2]]) {
        match self {
            Self::Direct(player) => player.process(frames),
            Self::Bridge(bridge) => bridge.fill(frames),
        }
    }

    fn into_player(self) -> Player {
        match self {
            Self::Direct(player) => player,
            Self::Bridge(bridge) => bridge.player,
        }
    }
}

/// A stream callback's hold on the player: when the callback goes (the stream is dropped or
/// never opened), the player is handed back on `back` instead of going with it.
struct Lease {
    render: Option<Render>,
    back: mpsc::SyncSender<Player>,
}

impl Lease {
    /// A lease on `render`, and where its player comes back. The channel holds the one player
    /// from the start, so handing it back allocates nothing on the audio thread.
    fn new(render: Render) -> (Self, mpsc::Receiver<Player>) {
        let (back, returned) = mpsc::sync_channel(1);
        let lease = Self {
            render: Some(render),
            back,
        };
        (lease, returned)
    }

    fn fill(&mut self, frames: &mut [[f32; 2]]) {
        if let Some(render) = &mut self.render {
            render.fill(frames);
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(render) = self.render.take() {
            let _ = self.back.try_send(render.into_player());
        }
    }
}

/// A running output stream. Dropping it stops playback.
pub struct Output {
    /// The stream; dropping it stops it, and its callback's [`Lease`] hands the player back to
    /// `returned`. Why `Any`: whatever holds a lease can then stand in for it, as in a test.
    stream: Box<dyn Any>,
    returned: mpsc::Receiver<Player>,
    health: Arc<Health>,
    /// Project and device rate, when the stream resamples.
    resampling: Option<(u32, u32)>,
    /// The rate the device plays at on its own ([`own_rate`]), when its configurations said.
    own_rate: Option<u32>,
    /// When the player's frames play, as the stream's callback publishes it.
    clock: Arc<PlaybackClock>,
    /// How long its callbacks have taken.
    stats: Arc<CallbackStats>,
    /// The device's name as Settings shows it ([`shown_device`]), what the settings keep for
    /// the device asked for (none: the system's default), and the buffer size asked of it, when
    /// it took one.
    device: String,
    asked: Option<String>,
    buffer: Option<u32>,
}

impl Output {
    /// Opens the output device the settings chose (`device`; the system's default when none) as
    /// 32-bit float stereo and starts rendering through `player`: at `sample_rate` when the
    /// device offers it, else at the device's rate through a resampler. `device_rate` forces
    /// the device's rate. On failure nothing plays and the player is handed back.
    pub fn start(
        player: Player,
        sample_rate: u32,
        device_rate: Option<u32>,
        device: Option<Chosen<'_>>,
    ) -> Result<Self, StartFailure> {
        let Prepared {
            device: opened,
            config,
            resampler,
            own_rate,
        } = match prepare(sample_rate, device_rate, device) {
            Ok(prepared) => prepared,
            Err(error) => {
                return Err(StartFailure {
                    error,
                    player: Some(Box::new(player)),
                })
            }
        };
        let mut timing = FrameTiming {
            first: player.output_frames(),
            delay: 0,
            device_rate: sample_rate,
            project_rate: sample_rate,
        };
        let (render, resampling) = match resampler {
            None => (Render::Direct(player), None),
            Some((resampler, device_rate)) => {
                timing.delay = resampler.output_delay() as u64;
                timing.device_rate = device_rate;
                (
                    Render::Bridge(RateBridge::with_resampler(player, resampler)),
                    Some((sample_rate, device_rate)),
                )
            }
        };
        let health = Arc::new(Health::default());
        let clock = Arc::new(PlaybackClock::new(sample_rate));
        let stats = Arc::new(CallbackStats::new(config.sample_rate));
        let buffer = match config.buffer_size {
            BufferSize::Fixed(frames) => Some(frames),
            BufferSize::Default => None,
        };
        let (mut lease, returned) = Lease::new(render);
        let render = move |frames: &mut [[f32; 2]]| lease.fill(frames);
        match open_stream(&opened, config, &health, (&clock, timing), &stats, render) {
            Ok(stream) => Ok(Self {
                stream: Box::new(stream),
                returned,
                health,
                resampling,
                own_rate,
                clock,
                stats,
                device: shown_device(&opened.to_string()).to_owned(),
                asked: device.map(|chosen| chosen.id.to_owned()),
                buffer,
            }),
            Err(error) => Err(StartFailure {
                error,
                player: returned.recv_timeout(RETURN_TIMEOUT).ok().map(Box::new),
            }),
        }
    }

    /// Stops the stream and gives back its player; `None` only if the audio backend kept the
    /// callback past [`RETURN_TIMEOUT`].
    pub fn stop(self) -> Option<Player> {
        drop(self.stream);
        self.clock.forget();
        self.returned.recv_timeout(RETURN_TIMEOUT).ok()
    }

    /// When the player's frames play, for an input stream to line a recording up by.
    pub fn playback_clock(&self) -> Arc<PlaybackClock> {
        Arc::clone(&self.clock)
    }

    /// How long this stream's callbacks have taken, kept after the stream goes.
    pub fn stats(&self) -> Arc<CallbackStats> {
        Arc::clone(&self.stats)
    }

    /// What went wrong since the stream started, if anything.
    pub fn problem(&self) -> Option<&'static str> {
        match self.health.0.load(Ordering::Relaxed) {
            HEALTHY => None,
            DROPOUT => Some("Audio dropped out briefly and played on"),
            NORMAL_PRIORITY => Some("Audio runs at normal priority and may crackle under load"),
            _ => Some("Audio stopped: the output device failed"),
        }
    }

    /// Whether the stream has reported a failure: it plays no more and never calls its player
    /// again.
    pub fn failed(&self) -> bool {
        self.health.0.load(Ordering::Relaxed) == FAILED
    }

    /// What the settings keep for the device asked for when the stream opened: none for the
    /// system's default.
    pub fn asked(&self) -> Option<&str> {
        self.asked.as_deref()
    }

    /// Frames per buffer of the stream: the size asked for, else the largest the device has
    /// given, else the size asked for when it takes one. An input channel's monitor keeps one
    /// such block waiting beside its input's own.
    pub fn block_frames(&self) -> u64 {
        let seen = self.stats.most_frames.load(Ordering::Relaxed);
        self.buffer
            .map(u64::from)
            .or((seen > 0).then_some(seen))
            .unwrap_or(u64::from(BUFFER_FRAMES))
    }

    /// What Settings says while the stream plays: its device, rate and buffer size.
    pub fn playing_through(&self) -> String {
        let seen = self.stats.most_frames.load(Ordering::Relaxed);
        let frames = self.buffer.map(u64::from).or((seen > 0).then_some(seen));
        playing_through(&self.device, self.stats.rate, frames)
    }

    /// The rate its device plays at on its own ([`own_rate`]): the rate a new document takes
    /// while this stream is open. `None` when the device's configurations did not say.
    pub fn own_rate(&self) -> Option<u32> {
        self.own_rate
    }

    /// A line for Settings → Audio when the stream resamples: "Resampling 44100 → 48000 Hz".
    pub fn notice(&self) -> Option<String> {
        self.resampling
            .map(|(project, device)| format!("Resampling {project} → {device} Hz"))
    }
}

impl Output {
    /// An output that plays to no device (`--null-output`, a scripted run that must open none):
    /// a thread of its own calls the player in real time, [`BUFFER_FRAMES`] at a time, and drops
    /// what it plays; the player comes back when the output goes.
    pub fn null(player: Player, sample_rate: u32) -> Self {
        let clock = Arc::new(PlaybackClock::new(sample_rate));
        let (lease, returned) = Lease::new(Render::Direct(player));
        Self {
            stream: Box::new(NullStream::start(lease, sample_rate)),
            returned,
            health: Arc::default(),
            resampling: None,
            own_rate: Some(sample_rate),
            clock,
            stats: Arc::new(CallbackStats::new(sample_rate)),
            device: "No device (--null-output)".to_owned(),
            asked: None,
            buffer: Some(BUFFER_FRAMES),
        }
    }
}

/// The thread a null output's player runs on; dropping it stops the thread, which hands the
/// player back as it lets go of the lease.
struct NullStream {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl NullStream {
    fn start(mut lease: Lease, rate: u32) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("dawsome-null-output".into())
            .spawn(move || {
                let mut block = vec![[0.0f32; 2]; BLOCK_FRAMES];
                let started = Instant::now();
                let mut played = 0u64;
                while !stopping.load(Ordering::Acquire) {
                    lease.fill(&mut block);
                    played += BLOCK_FRAMES as u64;
                    let due = started + Duration::from_secs_f64(played as f64 / f64::from(rate));
                    if let Some(wait) = due.checked_duration_since(Instant::now()) {
                        thread::sleep(wait);
                    }
                }
            })
            .ok();
        Self { stop, thread }
    }
}

impl Drop for NullStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
impl Output {
    /// An output without a device, holding `player` where a stream's callback would, so the
    /// player comes back when the output goes.
    pub(crate) fn holding(player: Player) -> Self {
        let clock = Arc::new(PlaybackClock::new(48_000));
        let (lease, returned) = Lease::new(Render::Direct(player));
        Self {
            stream: Box::new(lease),
            returned,
            health: Arc::default(),
            resampling: None,
            own_rate: None,
            clock,
            stats: Arc::new(CallbackStats::new(48_000)),
            device: "Test output".to_owned(),
            asked: None,
            buffer: Some(BUFFER_FRAMES),
        }
    }

    /// As [`holding`](Self::holding), as if its device played at `own_rate` on its own, and
    /// the stream resampled `resampling` (project, device rate) when given.
    pub(crate) fn holding_at(
        player: Player,
        own_rate: u32,
        resampling: Option<(u32, u32)>,
    ) -> Self {
        Self {
            own_rate: Some(own_rate),
            resampling,
            ..Self::holding(player)
        }
    }

    /// As [`holding`](Self::holding), as if opened on the device `asked` for (none: the
    /// system's default), which plays at `own_rate` on its own.
    pub(crate) fn holding_named_at(player: Player, asked: Option<&str>, own_rate: u32) -> Self {
        Self {
            device: asked.unwrap_or("Test output").to_owned(),
            asked: asked.map(str::to_owned),
            own_rate: Some(own_rate),
            ..Self::holding(player)
        }
    }

    /// As [`holding`](Self::holding), as if opened on the device `asked` for.
    pub(crate) fn holding_on(player: Player, asked: &str) -> Self {
        Self::holding_as(player, asked, asked)
    }

    /// As [`holding`](Self::holding), as if opened on the device the settings keep as `asked`,
    /// which Settings shows as `device`.
    pub(crate) fn holding_as(player: Player, asked: &str, device: &str) -> Self {
        Self {
            device: device.to_owned(),
            asked: Some(asked.to_owned()),
            ..Self::holding(player)
        }
    }

    /// Reports `kind` as the stream's error callback does.
    pub(crate) fn report(&self, kind: ErrorKind) {
        self.health.report(kind);
    }
}

/// Opens and starts a stream on `device` whose callback is `render` and, before it, the
/// publishing of when the player's frames play on `clock` (from `timing`); its errors are
/// reported to `health` and its render times to `stats`.
fn open_stream(
    device: &cpal::Device,
    config: StreamConfig,
    health: &Arc<Health>,
    (clock, timing): (&Arc<PlaybackClock>, FrameTiming),
    stats: &Arc<CallbackStats>,
    mut render: impl FnMut(&mut [[f32; 2]]) + Send + 'static,
) -> Result<cpal::Stream, StartError> {
    let reporter = Arc::clone(health);
    let offset = Arc::new(AtomicI64::new(0));
    let (playback_clock, stream_offset) = (Arc::clone(clock), Arc::clone(&offset));
    let stats = Arc::clone(stats);
    let mut played = 0u64;
    let stream = device.build_output_stream(
        config,
        move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
            let playback = stream_nanos(info.timestamp().playback)
                + i128::from(stream_offset.load(Ordering::Relaxed));
            playback_clock.publish(timing.origin(playback, played));
            // Interleaved stereo is a run of [left, right] frames; a two-channel buffer has no
            // remainder.
            let frames = data.as_chunks_mut::<2>().0;
            let started = Instant::now();
            render(frames);
            stats.record(started.elapsed(), frames.len());
            played += frames.len() as u64;
        },
        move |error: cpal::Error| reporter.report(error.kind()),
        Some(OPEN_TIMEOUT),
    )?;
    // Before `play`: no callback runs until then.
    offset.store(clock.offset_of(stream.now()), Ordering::Relaxed);
    stream.play()?;
    Ok(stream)
}

/// A stream's output side before it starts, as [`prepare`] opens it.
struct Prepared {
    device: cpal::Device,
    config: StreamConfig,
    /// When the stream resamples: the resampler and the device's rate.
    resampler: Option<(Box<dyn Resampler<f32>>, u32)>,
    /// The rate the device plays at on its own ([`own_rate`]).
    own_rate: Option<u32>,
}

/// The output device the settings chose, the system's default when none: the first output
/// device Settings lists ([`listed_output`]) that the kept value names ([`ListedOutput::is`]:
/// a sink by its node, any device by its name). A value Settings never offers
/// ([`is_output_device`]) is not there; a device that is not there is named as it was chosen.
fn output_device(chosen: Option<Chosen<'_>>) -> Result<cpal::Device, StartError> {
    let gone = |chosen: Chosen<'_>| StartError::DeviceGone {
        name: chosen.name.to_owned(),
    };
    if let Some(chosen) = chosen.filter(|chosen| !is_output_device(chosen.id)) {
        return Err(gone(chosen));
    }
    let host = cpal::default_host();
    match chosen {
        None => host.default_output_device().ok_or(StartError::NoDevice),
        Some(chosen) => host
            .output_devices()?
            .find(|device| {
                listed_output(device.to_string(), device.id().ok().as_ref())
                    .is_some_and(|listed| listed.is(chosen.id))
            })
            .ok_or_else(|| gone(chosen)),
    }
}

/// `device`'s default output configuration and its 32-bit float stereo ranges.
fn float_stereo(
    device: &cpal::Device,
) -> Result<(SupportedStreamConfig, Vec<SupportedStreamConfigRange>), StartError> {
    let default = device.default_output_config()?;
    let float_stereo = device
        .supported_output_configs()?
        .filter(|range| range.channels() == 2 && range.sample_format() == SampleFormat::F32)
        .collect();
    Ok((default, float_stereo))
}

/// The rates `ranges` offer.
fn offered_rates(ranges: &[SupportedStreamConfigRange]) -> Vec<RangeInclusive<u32>> {
    ranges
        .iter()
        .map(|range| range.min_sample_rate()..=range.max_sample_rate())
        .collect()
}

/// The output device the settings chose (the system's default when none, [`output_device`]),
/// its stream configuration and, when the stream resamples, the resampler and the device's
/// rate: everything that can fail before the player is handed over.
fn prepare(
    sample_rate: u32,
    device_rate: Option<u32>,
    chosen: Option<Chosen<'_>>,
) -> Result<Prepared, StartError> {
    let device = output_device(chosen)?;
    let (config, plan, own_rate) = stream_config(&device, sample_rate, device_rate)?;
    let resampler = match plan {
        RatePlan::Direct => None,
        RatePlan::Resample { device_rate } => {
            Some((bridge_resampler(sample_rate, device_rate, 2)?, device_rate))
        }
    };
    Ok(Prepared {
        device,
        config,
        resampler,
        own_rate,
    })
}

/// A player no stream plays, run on a thread of its own as a silent device would: a block at a
/// time at the project's rate. Plans, controls and plugin values reach it as they reach a live
/// one. Its thread is its plugins' audio thread until a stream takes the player back
/// ([`into_player`](Self::into_player)); a player left parked has its processors suspended
/// there before it is dropped there.
///
/// Why park rather than drop: dropping the player drops every processor in it, plugins
/// included, while the document still has them.
pub struct Parked {
    /// What the thread is asked to do; dropping it ends the thread.
    asks: Option<mpsc::Sender<Ask>>,
    answers: mpsc::Receiver<()>,
    /// Where the thread hands the player back when asked to.
    returned: mpsc::Receiver<Player>,
    thread: Option<JoinHandle<()>>,
}

/// What a parked player's thread is asked to do.
enum Ask {
    /// Take what waits for the player now.
    CatchUp,
    /// Hand the player back, as it is, and end.
    GiveBack,
}

impl Parked {
    /// Parks `player`, which plays at `sample_rate`. When no thread can be started the player
    /// is dropped, and its processors with it.
    pub fn new(player: Player, sample_rate: u32) -> Self {
        let period = Duration::from_secs_f64(BLOCK_FRAMES as f64 / f64::from(sample_rate.max(1)));
        let (asks, asked) = mpsc::channel();
        let (answer, answers) = mpsc::channel();
        let (give_back, returned) = mpsc::channel();
        let run = move || {
            let mut player = player;
            let mut block = vec![[0.0f32; 2]; BLOCK_FRAMES];
            loop {
                match asked.recv_timeout(period) {
                    // Takes the plan update just queued without playing a block.
                    Ok(Ask::CatchUp) => {
                        player.process(&mut []);
                        let _ = answer.send(());
                    }
                    // Not suspended: the stream it goes to plays its processors on, as a
                    // stream that fails hands them to this thread without a suspend.
                    Ok(Ask::GiveBack) => {
                        let _ = give_back.send(player);
                        return;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => player.process(&mut block),
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            player.suspend_processors();
        };
        let thread = thread::Builder::new()
            .name("dawsome-parked".into())
            .spawn(run)
            .ok();
        Self {
            asks: Some(asks),
            answers,
            returned,
            thread,
        }
    }

    /// Stops the thread and hands back its player, for a stream to play; `None` when no thread
    /// could be started for it (it was dropped then).
    pub fn into_player(mut self) -> Option<Player> {
        let asks = self.asks.take()?;
        let _ = asks.send(Ask::GiveBack);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.returned.try_recv().ok()
    }

    /// Has the player take the plan update just queued, waiting up to [`CATCH_UP_TIMEOUT`].
    /// Why wait: the plan ring holds only a few updates, and edits faster than the parked
    /// player's blocks would otherwise fill it and be refused.
    pub fn catch_up(&self) {
        let Some(asks) = &self.asks else {
            return;
        };
        // An answer to an earlier ask that timed out is not this one's.
        while self.answers.try_recv().is_ok() {}
        if asks.send(Ask::CatchUp).is_ok() {
            let _ = self.answers.recv_timeout(CATCH_UP_TIMEOUT);
        }
    }
}

impl Drop for Parked {
    fn drop(&mut self) {
        drop(self.asks.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A stereo 32-bit float configuration at the rate [`choose_rate`] picks, with
/// [`BUFFER_FRAMES`] per buffer when the device accepts that size; and the rate the device
/// plays at on its own ([`own_rate`]).
fn stream_config(
    device: &cpal::Device,
    sample_rate: u32,
    forced: Option<u32>,
) -> Result<(StreamConfig, RatePlan, Option<u32>), StartError> {
    let (default, float_stereo) = self::float_stereo(device)?;
    let offered = offered_rates(&float_stereo);
    let plan = choose_rate(sample_rate, &offered, default.sample_rate(), forced)?;
    let own = own_rate(&offered, default.sample_rate(), forced).ok();
    let rate = match plan {
        RatePlan::Direct => sample_rate,
        RatePlan::Resample { device_rate } => device_rate,
    };
    let supported = if is_float_stereo(&default) && default.sample_rate() == rate {
        default
    } else {
        // `choose_rate` only picks an offered rate, so a range holds it.
        float_stereo
            .into_iter()
            .find_map(|range| range.try_with_sample_rate(rate))
            .ok_or(StartError::RateNotOffered { rate })?
    };
    let buffer_size = match supported.buffer_size() {
        SupportedBufferSize::Range { min, max } if (*min..=*max).contains(&BUFFER_FRAMES) => {
            BufferSize::Fixed(BUFFER_FRAMES)
        }
        _ => BufferSize::Default,
    };
    let config = StreamConfig {
        buffer_size,
        ..supported.config()
    };
    Ok((config, plan, own))
}

fn is_float_stereo(config: &SupportedStreamConfig) -> bool {
    config.channels() == 2 && config.sample_format() == SampleFormat::F32
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use dawsome_engine::transport::Control;

    use super::*;
    use crate::counting_allocator::heap_calls_during;
    use crate::project_file;
    use crate::session::Session;

    // The song-take film: `--null-output` calls the player in real time with no device, about
    // 512 frames per 512 / 44100 s, and hands it back when it stops.
    #[test]
    fn a_null_output_plays_in_real_time_and_gives_the_player_back() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/f1_four_on_floor.json");
        let opened = project_file::open(&path).expect("F1");
        let (_session, player) =
            Session::open(opened.project, opened.samples_dir, false).expect("opens");
        let output = Output::null(player, 44_100);
        std::thread::sleep(Duration::from_millis(200));
        let player = output.stop().expect("the player comes back");
        // 200 ms is 8820 frames; the thread wakes late at times, never early.
        let frames = player.output_frames();
        assert!((4_096..=16_384).contains(&frames), "{frames} frames played");
    }

    /// An output list a test writes, counting how often it is read.
    struct Listed {
        names: Result<Vec<String>, String>,
        reads: std::cell::Cell<u32>,
    }

    impl OutputLister for Listed {
        fn outputs(&self) -> Result<Vec<ListedOutput>, String> {
            self.reads.set(self.reads.get() + 1);
            self.names
                .clone()
                .map(|names| names.into_iter().map(ListedOutput::named).collect())
        }
    }

    fn listed(names: Result<&[&str], &str>) -> Listed {
        Listed {
            names: names
                .map(|names| names.iter().map(|name| (*name).to_owned()).collect())
                .map_err(str::to_owned),
            reads: std::cell::Cell::new(0),
        }
    }

    /// A row for the device kept and shown as `name`.
    fn named(name: &str, missing: bool) -> OutputChoice {
        OutputChoice {
            id: Some(name.to_owned()),
            name: Some(name.to_owned()),
            missing,
            chosen: missing,
        }
    }

    /// A row for the device kept as `id` and shown as `name`, `chosen` or not.
    fn row(id: &str, name: &str, chosen: bool) -> OutputChoice {
        OutputChoice {
            id: Some(id.to_owned()),
            name: Some(name.to_owned()),
            missing: false,
            chosen,
        }
    }

    const DEFAULT: OutputChoice = OutputChoice {
        id: None,
        name: None,
        missing: false,
        chosen: false,
    };

    const DEFAULT_CHOSEN: OutputChoice = OutputChoice {
        chosen: true,
        ..DEFAULT
    };

    // B8: Settings offers the system's default first, then each output device once in the
    // order listed (the list is read once, and only for outputs); a chosen device not listed
    // stays last, marked missing; a list that fails offers the default and says why.
    #[test]
    fn the_output_choices_start_with_the_default_and_keep_the_chosen_device() {
        let two = listed(Ok(&["Speakers", "USB Audio", "Speakers"]));
        let (choices, problem) = output_choices(&two, Some(Chosen::new("USB Audio", None)));
        assert_eq!(
            choices,
            [
                DEFAULT,
                named("Speakers", false),
                row("USB Audio", "USB Audio", true)
            ]
        );
        assert_eq!((problem, two.reads.get()), (None, 1));
        let (choices, _) = output_choices(&two, Some(Chosen::new("Headphones", None)));
        assert_eq!(choices[3], named("Headphones", true));
        let failing = listed(Err("the audio server is not running"));
        let (choices, problem) = output_choices(&failing, None);
        assert_eq!(choices, [DEFAULT_CHOSEN]);
        assert_eq!(problem.as_deref(), Some("the audio server is not running"));
    }

    /// What cpal 0.18.2's PipeWire host (the app's host while PipeWire runs: its `pipewire`
    /// feature) listed among the outputs on a desktop machine on 2026-10-27, in its order: the
    /// name it gives each (a node's description) and its id (the node's name). Captured with a
    /// probe printing `device` and `device.id()`; `pactl list short sinks` named the same seven
    /// sinks. The playback streams of two browsers are nodes without a description.
    #[cfg(target_os = "linux")]
    const PIPEWIRE_LISTED: [(&str, &str); 17] = [
        ("default_sink", "sink_default"),
        ("default_output", "output_default"),
        (
            "Radeon High Definition Audio Controller Pro",
            "alsa_output.pci-0000_0d_00.1.pro-output-3",
        ),
        (
            "Radeon High Definition Audio Controller Pro 7",
            "alsa_output.pci-0000_0d_00.1.pro-output-7",
        ),
        (
            "AD103 High Definition Audio Controller Digital Stereo (HDMI) [27M2V]",
            "alsa_output.pci-0000_01_00.1.hdmi-stereo",
        ),
        (
            "USB Audio S/PDIF Output",
            "alsa_output.usb-Generic_USB_Audio-00.HiFi__SPDIF__sink",
        ),
        (
            "USB Audio Speakers",
            "alsa_output.usb-Generic_USB_Audio-00.HiFi__Speaker__sink",
        ),
        (
            "USB Audio Front Headphones",
            "alsa_output.usb-Generic_USB_Audio-00.HiFi__Headphones__sink",
        ),
        ("unknown", "Zen"),
        ("unknown", "Zen"),
        ("unknown", "Zen"),
        ("unknown", "Firefox"),
        ("unknown", "Firefox"),
        ("unknown", "Zen"),
        ("unknown", "Firefox"),
        (
            "Arctis Nova 7 Analog Stereo",
            "alsa_output.usb-SteelSeries_Arctis_Nova_7-00.analog-stereo",
        ),
        ("unknown", "Firefox"),
    ];

    /// [`PIPEWIRE_LISTED`] as Settings lists it.
    #[cfg(target_os = "linux")]
    fn pipewire_listed() -> Vec<ListedOutput> {
        PIPEWIRE_LISTED
            .iter()
            .filter_map(|(name, node)| {
                let id = cpal::DeviceId::new(cpal::HostId::PipeWire, node);
                listed_output((*name).to_owned(), Some(&id))
            })
            .collect()
    }

    // The PipeWire host's outputs on that machine: the seven sinks, each shown by its
    // description and kept by "sink:" and its node name (the node names `pactl list short
    // sinks` gives); the two stand-ins for the default and the browsers' streams are no row.
    // The HDMI sink's description took its monitor's name ("[27M2V]") between two captures of
    // this list, where its node name stayed: why the node is what the settings keep.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_sound_servers_sinks_show_their_descriptions_and_are_kept_by_node() {
        let sink = |name: &str, node: &str| ListedOutput {
            id: format!("sink:{node}"),
            name: name.to_owned(),
        };
        assert_eq!(
            pipewire_listed(),
            [
                sink(
                    "Radeon High Definition Audio Controller Pro",
                    "alsa_output.pci-0000_0d_00.1.pro-output-3"
                ),
                sink(
                    "Radeon High Definition Audio Controller Pro 7",
                    "alsa_output.pci-0000_0d_00.1.pro-output-7"
                ),
                sink(
                    "AD103 High Definition Audio Controller Digital Stereo (HDMI) [27M2V]",
                    "alsa_output.pci-0000_01_00.1.hdmi-stereo"
                ),
                sink(
                    "USB Audio S/PDIF Output",
                    "alsa_output.usb-Generic_USB_Audio-00.HiFi__SPDIF__sink"
                ),
                sink(
                    "USB Audio Speakers",
                    "alsa_output.usb-Generic_USB_Audio-00.HiFi__Speaker__sink"
                ),
                sink(
                    "USB Audio Front Headphones",
                    "alsa_output.usb-Generic_USB_Audio-00.HiFi__Headphones__sink"
                ),
                sink(
                    "Arctis Nova 7 Analog Stereo",
                    "alsa_output.usb-SteelSeries_Arctis_Nova_7-00.analog-stereo"
                ),
            ]
        );
    }

    // The choice is marked by what the settings keep: a sink by its node; a choice saved by
    // name before (that machine's settings.json held "Arctis Nova 7 Analog Stereo") marks the sink
    // of that name, with no second row; a chosen sink that is not there is the last row, named
    // as it was when chosen, or by its node when no name was kept; none marks the default.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_chosen_sink_is_marked_by_its_node_or_by_the_name_it_was_saved_by() {
        let headset = "sink:alsa_output.usb-SteelSeries_Arctis_Nova_7-00.analog-stereo";
        let headset_name = "Arctis Nova 7 Analog Stereo";
        let speakers = "sink:alsa_output.usb-Generic_USB_Audio-00.HiFi__Speaker__sink";
        let listed = pipewire_listed();
        let marked = |chosen: Option<Chosen<'_>>| -> Vec<(Option<String>, bool)> {
            let (choices, problem) = output_choices(listed.as_slice(), chosen);
            assert_eq!(problem, None);
            choices
                .into_iter()
                .filter(|choice| choice.chosen || choice.missing)
                .map(|choice| (choice.name, choice.missing))
                .collect()
        };
        assert_eq!(marked(None), [(None, false)]);
        for kept in [headset, headset_name] {
            assert_eq!(
                marked(Some(Chosen::new(kept, Some(headset_name)))),
                [(Some(headset_name.to_owned()), false)],
                "{kept}"
            );
        }
        assert_eq!(
            marked(Some(Chosen::new(speakers, Some("USB Audio Speakers")))),
            [(Some("USB Audio Speakers".to_owned()), false)]
        );
        let (choices, _) = output_choices(listed.as_slice(), Some(Chosen::new(headset, None)));
        assert_eq!(choices.len(), 8, "the default and the seven sinks");
        let unplugged = &listed[..6];
        let (choices, _) =
            output_choices(unplugged, Some(Chosen::new(headset, Some(headset_name))));
        assert_eq!(
            choices.last(),
            Some(&OutputChoice {
                id: Some(headset.to_owned()),
                name: Some(headset_name.to_owned()),
                missing: true,
                chosen: true,
            })
        );
        let (choices, _) = output_choices(unplugged, Some(Chosen::new(headset, None)));
        assert_eq!(
            choices.last().and_then(|choice| choice.name.as_deref()),
            Some("alsa_output.usb-SteelSeries_Arctis_Nova_7-00.analog-stereo")
        );
    }

    // What cpal 0.18.2's ALSA host (the app's host when no PipeWire runs) listed among the
    // outputs on the same machine on 2026-10-27, by name and PCM id: its first 54 rows (the
    // capture was cut there).
    // Settings offers the cards: the default and the sound servers' PCMs (which "System
    // default" stands for), the null sink, the rate converters, the Speex and channel mixing
    // plugins, OSS and the USB stream PCMs are no row; each card's PCMs share its name and show
    // once (the first, its "sysdefault:CARD=…"); a card whose default PCM is named as the bare
    // default is ("Oriole"). JACK's PCM stays.
    #[cfg(target_os = "linux")]
    #[test]
    fn alsas_plugins_are_no_row_and_each_card_shows_once() {
        const ALSA_LISTED: [(&str, &str); 54] = [
            (
                "Discard all samples (playback) or generate zero samples (capture)",
                "null",
            ),
            ("Default Audio Device", "sysdefault"),
            (
                "Rate Converter Plugin Using Libav/FFmpeg Library",
                "lavrate",
            ),
            (
                "Rate Converter Plugin Using Samplerate Library",
                "samplerate",
            ),
            ("Rate Converter Plugin Using Speex Resampler", "speexrate"),
            ("JACK Audio Connection Kit", "jack"),
            ("Open Sound System", "oss"),
            ("PipeWire Sound Server", "pipewire"),
            ("PulseAudio Sound Server", "pulse"),
            (
                "Plugin using Speex DSP (resample, agc, denoise, echo, dereverb)",
                "speex",
            ),
            ("Plugin for channel upmix (4,6,8)", "upmix"),
            (
                "Plugin for channel downmix (stereo) with a simple spacialization",
                "vdownmix",
            ),
            (
                "Default ALSA Output (currently PipeWire Media Server)",
                "default",
            ),
            ("Arctis Nova 7, USB Audio", "sysdefault:CARD=A7"),
            ("Arctis Nova 7, USB Audio", "front:CARD=A7,DEV=0"),
            ("Arctis Nova 7, USB Audio", "surround21:CARD=A7,DEV=0"),
            ("Arctis Nova 7, USB Audio", "surround40:CARD=A7,DEV=0"),
            ("Arctis Nova 7, USB Audio", "surround41:CARD=A7,DEV=0"),
            ("Arctis Nova 7, USB Audio", "surround50:CARD=A7,DEV=0"),
            ("Arctis Nova 7, USB Audio", "surround51:CARD=A7,DEV=0"),
            ("Arctis Nova 7, USB Audio", "surround71:CARD=A7,DEV=0"),
            ("Arctis Nova 7, USB Audio", "iec958:CARD=A7,DEV=0"),
            ("Arctis Nova 7", "usbstream:CARD=A7"),
            ("HDA NVidia, 27M2V", "hdmi:CARD=NVidia,DEV=0"),
            ("HDA NVidia, Optix MAG27C", "hdmi:CARD=NVidia,DEV=1"),
            ("HDA NVidia, HDMI 2", "hdmi:CARD=NVidia,DEV=2"),
            ("HDA NVidia, HDMI 3", "hdmi:CARD=NVidia,DEV=3"),
            ("HDA NVidia", "usbstream:CARD=NVidia"),
            ("HD-Audio Generic, ASUS VP228", "hdmi:CARD=Generic,DEV=0"),
            ("HD-Audio Generic, HDMI 1", "hdmi:CARD=Generic,DEV=1"),
            ("HD-Audio Generic", "usbstream:CARD=Generic"),
            ("Default Audio Device", "sysdefault:CARD=Oriole"),
            ("Oriole", "usbstream:CARD=Oriole"),
            ("USB Audio, USB Audio", "sysdefault:CARD=Audio"),
            ("USB Audio, USB Audio", "front:CARD=Audio,DEV=0"),
            ("USB Audio, USB Audio", "surround21:CARD=Audio,DEV=0"),
            ("USB Audio, USB Audio", "surround40:CARD=Audio,DEV=0"),
            ("USB Audio, USB Audio", "surround41:CARD=Audio,DEV=0"),
            ("USB Audio, USB Audio", "surround50:CARD=Audio,DEV=0"),
            ("USB Audio, USB Audio", "surround51:CARD=Audio,DEV=0"),
            ("USB Audio, USB Audio", "surround71:CARD=Audio,DEV=0"),
            ("USB Audio, USB Audio", "iec958:CARD=Audio,DEV=0"),
            ("USB Audio, USB Audio #1", "iec958:CARD=Audio,DEV=1"),
            ("USB Audio", "usbstream:CARD=Audio"),
            ("Arctis Nova 7, USB Audio", "hw:CARD=0,DEV=0"),
            ("Arctis Nova 7, USB Audio", "plughw:CARD=0,DEV=0"),
            ("HDA NVidia, 27M2V", "hw:CARD=1,DEV=3"),
            ("HDA NVidia, 27M2V", "plughw:CARD=1,DEV=3"),
            ("HDA NVidia, Optix MAG27C", "hw:CARD=1,DEV=7"),
            ("HDA NVidia, Optix MAG27C", "plughw:CARD=1,DEV=7"),
            ("HDA NVidia, HDMI 2", "hw:CARD=1,DEV=8"),
            ("HDA NVidia, HDMI 2", "plughw:CARD=1,DEV=8"),
            ("HDA NVidia, HDMI 3", "hw:CARD=1,DEV=9"),
            ("HDA NVidia, HDMI 3", "plughw:CARD=1,DEV=9"),
        ];
        let listed: Vec<ListedOutput> = ALSA_LISTED
            .iter()
            .filter_map(|(name, pcm)| {
                let id = cpal::DeviceId::new(cpal::HostId::Alsa, pcm);
                listed_output((*name).to_owned(), Some(&id))
            })
            .collect();
        let (choices, _) = output_choices(listed.as_slice(), None);
        let rows: Vec<&str> = choices
            .iter()
            .map(|choice| choice.name.as_deref().unwrap_or("System default"))
            .collect();
        assert_eq!(
            rows,
            [
                "System default",
                "JACK Audio Connection Kit",
                "Arctis Nova 7, USB Audio",
                "HDA NVidia, 27M2V",
                "HDA NVidia, Optix MAG27C",
                "HDA NVidia, HDMI 2",
                "HDA NVidia, HDMI 3",
                "HD-Audio Generic, ASUS VP228",
                "HD-Audio Generic, HDMI 1",
                "Oriole",
                "USB Audio, USB Audio",
                "USB Audio, USB Audio #1",
            ]
        );
        assert!(
            choices[1..].iter().all(|choice| choice.id == choice.name),
            "ALSA's devices are kept by name, as before"
        );
    }

    // What cpal lists among the outputs on every host: a stand-in for the system default and a
    // program's playback stream are no device; a name Settings never offers is not opened: it
    // is "not there", named as it was chosen, before any audio host is asked.
    #[test]
    fn settings_offers_only_real_output_devices() {
        for name in [
            "default_sink",
            "default_output",
            "unknown",
            "default",
            "sysdefault",
            "pulse",
            "pipewire",
        ] {
            assert!(!is_output_device(name), "{name}");
            assert_eq!(listed_output(name.to_owned(), None), None, "{name}");
        }
        assert_eq!(
            listed_output("Speakers (Realtek(R) Audio)".to_owned(), None),
            Some(ListedOutput::named("Speakers (Realtek(R) Audio)")),
            "a device without a PipeWire or ALSA id is kept by its name, as on Windows"
        );
        for (id, name) in [("unknown", None), ("default_sink", Some("Old default"))] {
            let chosen = Chosen::new(id, name);
            assert!(matches!(
                prepare(48_000, None, Some(chosen)),
                Err(StartError::DeviceGone { name: gone }) if gone == chosen.name
            ));
        }
    }

    // Settings shows the stream of a stand-in for the system default (the PipeWire host's
    // "default_output") as "System default", and any device by its own name.
    #[test]
    fn a_stream_on_the_default_stand_in_is_shown_as_the_system_default() {
        assert_eq!(shown_device("default_output"), "System default");
        assert_eq!(shown_device("default_sink"), "System default");
        assert_eq!(
            shown_device("Arctis Nova 7 Analog Stereo"),
            "Arctis Nova 7 Analog Stereo"
        );
        assert_eq!(
            shown_device("Speakers (Realtek(R) Audio)"),
            "Speakers (Realtek(R) Audio)"
        );
    }

    // `--list-outputs`: a line a row, the chosen one marked, then a tab and what the settings
    // keep for it; why the list is short last.
    #[test]
    fn the_listing_marks_the_chosen_row_and_names_what_is_kept() {
        let choices = [
            DEFAULT,
            row("sink:a.node", "Headset", true),
            row("Speakers", "Speakers", false),
            OutputChoice {
                missing: true,
                ..row("sink:b.node", "Old", false)
            },
        ];
        assert_eq!(
            listing(&choices, Some("no server")),
            "  System default\t\n* Headset\tsink:a.node\n  Speakers\tSpeakers\n  Old (not plugged \
             in)\tsink:b.node\ncould not list the devices: no server\n"
        );
    }

    // The Settings line names the device, its rate and its buffer, or leaves the buffer out
    // until the device has given one.
    #[test]
    fn the_playing_through_line_names_the_device_rate_and_buffer() {
        assert_eq!(
            playing_through("USB Audio", 48_000, Some(512)),
            "Playing through: USB Audio, 48000 Hz, 512 frames"
        );
        assert_eq!(
            playing_through("USB Audio", 44_100, None),
            "Playing through: USB Audio, 44100 Hz"
        );
    }

    // A stream at the project's rate plays the player's frames one for one: if device frame
    // 4800 (the 4800th since the stream began, the player 1000 frames in when it did) plays at
    // 2 s, player frame 5800 plays then and frame 0 at 2 s − 5800/48000 s.
    #[test]
    fn the_playback_clock_times_the_players_frames() {
        let clock = PlaybackClock::new(48_000);
        assert_eq!(clock.time_of(0), None, "no stream plays yet");
        let timing = FrameTiming {
            first: 1000,
            delay: 0,
            device_rate: 48_000,
            project_rate: 48_000,
        };
        clock.publish(timing.origin(2_000_000_000, 4800));
        assert_eq!(clock.time_of(5800), Some(2_000_000_000));
        assert_eq!(clock.time_of(5800 + 48_000), Some(3_000_000_000));
        clock.forget();
        assert_eq!(clock.time_of(5800), None, "the stream is gone");
    }

    // Behind a resampler a 44.1 kHz player on a 48 kHz device reaches the device 100 device
    // frames late (the resampler's delay): device frame 4900 plays player frame
    // 1000 + (4900 − 100) · 44100/48000 = 5410.
    #[test]
    fn the_playback_clock_counts_the_resamplers_delay() {
        let clock = PlaybackClock::new(44_100);
        let timing = FrameTiming {
            first: 1000,
            delay: 100,
            device_rate: 48_000,
            project_rate: 44_100,
        };
        clock.publish(timing.origin(2_000_000_000, 4900));
        let at = clock.time_of(5410).expect("timed");
        assert!((at - 2_000_000_000).abs() <= 1, "{at}");
    }

    // Hand-computed: 100 µs over 480 frames at 48 kHz is 1 % of the 10 ms they play; 300 µs
    // over 512 frames (10.667 ms) is 2.8125 %, the worst; an empty buffer counts but has no
    // share. Mean (100 + 300 + 40) / 3 = 146.67 µs.
    #[test]
    fn callback_stats_keep_the_worst_share_of_a_buffers_time() {
        let stats = CallbackStats::new(48_000);
        assert_eq!(stats.to_string(), "audio callbacks: none");
        let calls = heap_calls_during(|| {
            stats.record(Duration::from_micros(100), 480);
            stats.record(Duration::from_micros(300), 512);
            stats.record(Duration::from_micros(40), 0);
        });
        assert_eq!(calls, 0, "counting a callback touched the heap");
        assert_eq!(stats.worst_load_ppm.load(Ordering::Relaxed), 28_125);
        assert_eq!(
            stats.to_string(),
            "audio callbacks: 3, mean 146.7 µs, worst 300.0 µs (2.81 % of its buffer's time), \
             buffers up to 512 frames at 48000 Hz"
        );
    }

    #[test]
    fn the_worst_reported_problem_wins() {
        let health = Health::default();
        health.report(ErrorKind::DeviceChanged);
        assert_eq!(health.0.load(Ordering::Relaxed), HEALTHY);
        health.report(ErrorKind::Xrun);
        assert_eq!(health.0.load(Ordering::Relaxed), DROPOUT);
        health.report(ErrorKind::RealtimeDenied);
        assert_eq!(health.0.load(Ordering::Relaxed), NORMAL_PRIORITY);
        health.report(ErrorKind::Xrun);
        assert_eq!(health.0.load(Ordering::Relaxed), NORMAL_PRIORITY);
        health.report(ErrorKind::DeviceNotAvailable);
        health.report(ErrorKind::RealtimeDenied);
        health.report(ErrorKind::Xrun);
        assert_eq!(health.0.load(Ordering::Relaxed), FAILED);
    }

    #[test]
    fn an_xrun_is_a_dropout_and_a_lost_device_stream_or_host_is_fatal() {
        let after = |kind| {
            let health = Health::default();
            health.report(kind);
            health.0.load(Ordering::Relaxed)
        };
        assert_eq!(
            after(ErrorKind::Xrun),
            DROPOUT,
            "the stream recovers and plays on"
        );
        for kind in [
            ErrorKind::DeviceNotAvailable,
            ErrorKind::StreamInvalidated,
            ErrorKind::HostUnavailable,
        ] {
            assert_eq!(after(kind), FAILED, "{kind:?}");
        }
    }

    #[test]
    fn the_project_rate_plays_direct_when_offered_and_is_resampled_otherwise() {
        let choose =
            |offered: &[RangeInclusive<u32>], forced| choose_rate(44_100, offered, 48_000, forced);
        assert_eq!(
            choose(&[44_100..=44_100], None).ok(),
            Some(RatePlan::Direct)
        );
        assert_eq!(
            choose(&[8_000..=192_000], None).ok(),
            Some(RatePlan::Direct)
        );
        let resample = |rate| Some(RatePlan::Resample { device_rate: rate });
        assert_eq!(choose(&[48_000..=48_000], None).ok(), resample(48_000));
        // A default the float stereo ranges do not hold: the nearest rate they offer.
        assert_eq!(choose(&[96_000..=96_000], None).ok(), resample(96_000));
        assert!(matches!(choose(&[], None), Err(StartError::NoFloatStereo)));
        // --device-rate forces a rate, which must be offered.
        assert_eq!(
            choose(&[8_000..=192_000], Some(48_000)).ok(),
            resample(48_000)
        );
        assert_eq!(
            choose(&[8_000..=192_000], Some(44_100)).ok(),
            Some(RatePlan::Direct)
        );
        assert!(matches!(
            choose(&[44_100..=48_000], Some(96_000)),
            Err(StartError::RateNotOffered { rate: 96_000 })
        ));
    }

    // A new document takes the rate the device plays at on its own: its
    // default when its float stereo ranges hold it (48000 on a 48 kHz-only device),
    // else the offered rate nearest the default, else what --device-rate forces.
    #[test]
    fn a_device_plays_its_default_rate_on_its_own() {
        assert_eq!(
            own_rate(&[48_000..=48_000], 48_000, None).ok(),
            Some(48_000)
        );
        assert_eq!(
            own_rate(&[8_000..=192_000], 44_100, None).ok(),
            Some(44_100)
        );
        assert_eq!(
            own_rate(&[96_000..=96_000], 48_000, None).ok(),
            Some(96_000)
        );
        assert_eq!(
            own_rate(&[8_000..=192_000], 48_000, Some(88_200)).ok(),
            Some(88_200)
        );
        assert!(matches!(
            own_rate(&[], 48_000, None),
            Err(StartError::NoFloatStereo)
        ));
    }

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    /// A bridge from F1, playing from tick 0, to a 48 kHz device; and the session that is the
    /// other half of its player's link. The bridge sees only frames, so F1's 44.1 kHz frames
    /// stand in for a project at any `project_rate`: only the rate ratio changes.
    fn f1_bridge(project_rate: u32) -> (Session, RateBridge) {
        let dir = PathBuf::from(FIXTURES);
        let project = project_file::open(&dir.join("f1_four_on_floor.json"))
            .expect("F1 opens")
            .project;
        let (mut session, player) = Session::open(project, dir, false).expect("F1 compiles");
        session
            .send(Control::Play)
            .expect("the control ring has room");
        let bridge = RateBridge::new(player, project_rate, 48_000).expect("a bridge to 48 kHz");
        (session, bridge)
    }

    /// Pulls `frames` device frames in buffers of `sizes` (repeating the last), counting the
    /// heap calls the pulls make.
    fn pull(bridge: &mut RateBridge, frames: usize, sizes: &[usize]) -> (Vec<[f32; 2]>, usize) {
        let mut out = vec![[f32::NAN; 2]; frames];
        let mut heap_calls = 0;
        let mut done = 0;
        for &size in sizes
            .iter()
            .chain(std::iter::repeat(sizes.last().expect("a size")))
        {
            if done == frames {
                break;
            }
            let end = (done + size).min(frames);
            let buffer = &mut out[done..end];
            heap_calls += heap_calls_during(|| bridge.fill(buffer));
            done = end;
        }
        (out, heap_calls)
    }

    /// Seeded device buffer sizes from 1 to 2048 (xorshift64, so a failure reproduces).
    fn random_sizes(seed: u64, count: usize) -> Vec<usize> {
        let mut state = seed;
        (0..count)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                usize::try_from(state % 2048).expect("below 2048") + 1
            })
            .collect()
    }

    // From gen_p3_expected.py, case (h): 44100 → 48000 is 160/147, and F1's impulses are 22050
    // project frames apart, a multiple of 147, so every one resamples at the same phase: peaks
    // exactly 24000 device frames apart, the first at the resampler's own delay, all of one height.
    // Two project bars are 192000 device frames.
    #[test]
    fn h_the_bridge_keeps_f1s_beat_at_48k() {
        const FRAMES: usize = 192_000;
        // Control: the counter must see an allocation, or the zeros below would prove nothing.
        let control = heap_calls_during(|| drop(std::hint::black_box(vec![0u8; 64])));
        assert!(control > 0, "the counting allocator is not counting");
        let (_session, mut bridge) = f1_bridge(44_100);
        let delay = bridge.output_delay();
        let (out, heap_calls) = pull(&mut bridge, FRAMES, &random_sizes(0x5eed, 4000));
        assert_eq!(heap_calls, 0, "filling device buffers touched the heap");
        let peaks: Vec<(usize, f32)> = (0..8)
            .map(|i| {
                let around = delay + 24_000 * i;
                let window = around.saturating_sub(2000)..around + 2000;
                let (offset, frame) = out[window.clone()]
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1[0].abs().total_cmp(&b.1[0].abs()))
                    .expect("a non-empty window");
                (window.start + offset, frame[0])
            })
            .collect();
        println!("delay {delay}, peaks {peaks:?}");
        assert!(
            peaks[0].0.abs_diff(delay) <= 1,
            "first peak at {} for delay {delay}",
            peaks[0].0
        );
        for pair in peaks.windows(2) {
            assert_eq!(pair[1].0 - pair[0].0, 24_000, "peak spacing");
            assert!(
                (pair[1].1 - pair[0].1).abs() < 1e-6,
                "peak heights {pair:?}"
            );
        }
        assert!(peaks[0].1 > 0.5, "an impulse came through: {}", peaks[0].1);
        assert!(
            out.iter().all(|frame| frame[0] == frame[1]),
            "both channels alike"
        );
    }

    /// Checks that F1 through a bridge at `project_rate` gives the same bits for device buffers
    /// of 1, 480, 512, 2048 and seeded random frames as for one buffer, without touching the
    /// heap.
    fn same_bits_for_any_buffer_size(project_rate: u32) {
        const FRAMES: usize = 60_000;
        let bits = |sizes: &[usize]| {
            let (_session, mut bridge) = f1_bridge(project_rate);
            let (out, heap_calls) = pull(&mut bridge, FRAMES, sizes);
            assert_eq!(
                heap_calls, 0,
                "{project_rate} Hz, {sizes:?}: touched the heap"
            );
            out.iter()
                .map(|frame| frame.map(f32::to_bits))
                .collect::<Vec<_>>()
        };
        let whole = bits(&[FRAMES]);
        assert!(
            whole.iter().any(|frame| *frame != [0, 0]),
            "{project_rate} Hz: rendered silence"
        );
        for sizes in [
            vec![1],
            vec![480],
            vec![512],
            vec![2048],
            random_sizes(7, 2000),
        ] {
            assert!(
                bits(&sizes) == whole,
                "{project_rate} Hz: buffers of {:?}... differ",
                &sizes[..1]
            );
        }
    }

    // The bridge always renders and resamples whole 512-frame blocks, so the device's buffer
    // size cannot change a single bit of the output.
    #[test]
    fn h_any_device_buffer_size_gives_the_same_bits() {
        same_bits_for_any_buffer_size(44_100);
    }

    #[test]
    fn h_at_44101_hz_any_device_buffer_size_gives_the_same_bits() {
        same_bits_for_any_buffer_size(44_101);
    }

    // The FFT resampler's block is the reduced rate ratio: 147 to 160 frames for 44100 Hz
    // (delay 160 device frames), but the whole 44101 to 48000 or 47999 to 48000 for rates
    // sharing no factor with the device's (delay 24000 frames, half a second after Play).
    #[test]
    fn coprime_project_rates_start_within_2048_device_frames() {
        for project_rate in [44_101, 47_999] {
            let (_session, bridge) = f1_bridge(project_rate);
            let delay = bridge.output_delay();
            assert!(delay <= 2048, "{project_rate} Hz: delay {delay} frames");
        }
        let (_session, bridge) = f1_bridge(44_100);
        assert_eq!(
            bridge.output_delay(),
            160,
            "44100 Hz keeps the FFT resampler"
        );
    }

    // A 256-tap sinc filter delays by half its length, 128 project frames: at 44101 Hz that is
    // 128 * 48000 / 44101 = 139.3 device frames (computed in Python), so F1's first impulse
    // peaks within a frame of 139. The next is 23999.5 frames later, past the 4096 pulled.
    #[test]
    fn h_at_44101_hz_the_first_impulse_peaks_within_the_bound() {
        let (_session, mut bridge) = f1_bridge(44_101);
        let (out, heap_calls) = pull(&mut bridge, 4096, &random_sizes(0x5eed, 100));
        assert_eq!(heap_calls, 0, "filling device buffers touched the heap");
        let (peak, frame) = out
            .iter()
            .enumerate()
            .max_by(|a, b| a.1[0].abs().total_cmp(&b.1[0].abs()))
            .expect("frames were pulled");
        assert!(peak.abs_diff(139) <= 1, "first peak at frame {peak}");
        assert!(frame[0] > 0.5, "an impulse came through: {}", frame[0]);
    }
}
