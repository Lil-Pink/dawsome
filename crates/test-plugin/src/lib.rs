//! Dawsome's own CLAP test plugins, the host's oracles: their output follows exact formulas, so
//! a known-answer test needs no third-party binary.
//!
//! - `org.dawsome.test.sine` (instrument): `y[S0+n] = f32(gain·velocity·cos(τ·f·n/sr))` from the
//!   note-on frame `S0` up to the frame before the note-off, `f = 440·2^((key−69)/12)`, `n`
//!   counted from the note-on, 0.0 elsewhere. Monophonic: a new note restarts it. Its note port
//!   takes CLAP notes and MIDI messages; of the MIDI it hears pitch bend alone: `b` (En ll mm,
//!   ll + 128·mm − 8192) at frame `n = k` of a note plays it on from where its phase is at
//!   `f·2^(b/49152)` (±2 semitones less a step at the ends):
//!   `y = f32(gain·velocity·cos(τ·(P + f·r·(n − k))/sr))` with `P` the phase sum `f·k` (or the
//!   last bend's) and `r` the ratio; a bend with no note sounding bends the next note from its
//!   start.
//! - `org.dawsome.test.gain` (audio effect): `y = f32(f64(x)·gain)`.
//!
//! Both have one parameter (id 0, "Gain", 0..1, default 1), which a parameter event changes at
//! its own frame, and the state `b"OTP1"` + gain as f64 little-endian (12 bytes). Loading any
//! other bytes fails. Neither allocates while it processes.
//!
//! A copy of this library named `dawsome_test_variants.clap` offers variants of the sine
//! instead, for what the host must survive or serve:
//! - `org.dawsome.test.big`: its state is the sine's 12 bytes and then zeros, 5 MiB in all,
//!   past what the app captures with every change.
//! - `org.dawsome.test.fail`: plays [`FAIL_AFTER_BLOCKS`] blocks, then fails every `process`.
//! - `org.dawsome.test.timer`: registers a [`TIMER_PERIOD_MS`] main-thread timer with its host
//!   as it is created, work that comes due by the clock without the plugin asking.
//!
//! A copy named `dawsome_test_gui.clap` offers `org.dawsome.test.gui` alone: the sine with an
//! interface embedded in the host's X11 window (Linux; `gui.rs`), which logs each call its host
//! makes into it, for the host's plugin windows (known answers `gen_p29_expected.py`, its "clap"
//! key). Elsewhere it is the sine without one.
//!
//! # Unsafe code
//!
//! `clack-plugin` makes the CLAP ABI safe; the two blocks left are in `gui.rs` (the host handle
//! kept past its borrow, and the log copied into a caller's buffer), each under a `// SAFETY:`
//! comment, which the lints below enforce.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![deny(clippy::multiple_unsafe_ops_per_block)]

use std::f64::consts::TAU;
use std::ffi::CStr;
use std::fmt::Write as _;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use clack_extensions::audio_ports::{
    AudioPortFlags, AudioPortInfo, AudioPortInfoWriter, AudioPortType, PluginAudioPorts,
    PluginAudioPortsImpl,
};
use clack_extensions::note_ports::{
    NoteDialect, NoteDialects, NotePortInfo, NotePortInfoWriter, PluginNotePorts,
    PluginNotePortsImpl,
};
use clack_extensions::params::{
    HostParams, ParamDisplayWriter, ParamInfo, ParamInfoFlags, ParamInfoWriter, ParamRescanFlags,
    PluginAudioProcessorParams, PluginMainThreadParams, PluginParams,
};
use clack_extensions::state::{PluginState, PluginStateImpl};
use clack_extensions::timer::{HostTimer, PluginTimer, PluginTimerImpl, TimerId};
use clack_plugin::entry::prelude::*;
use clack_plugin::events::spaces::CoreEventSpace;
use clack_plugin::prelude::*;
use clack_plugin::stream::{InputStream, OutputStream};
use clack_plugin::utils::Cookie;

#[cfg(target_os = "linux")]
mod gui;

pub const SINE_ID: &str = "org.dawsome.test.sine";
pub const GAIN_ID: &str = "org.dawsome.test.gain";
pub const BIG_ID: &str = "org.dawsome.test.big";
pub const FAIL_ID: &str = "org.dawsome.test.fail";
pub const TIMER_ID: &str = "org.dawsome.test.timer";
pub const GUI_ID: &str = "org.dawsome.test.gui";

/// The file name that makes a copy of this library offer the variants.
const VARIANTS_FILE: &str = "dawsome_test_variants.clap";
/// The file name that makes a copy offer the sine with an interface.
const GUI_FILE: &str = "dawsome_test_gui.clap";

/// The state's first four bytes.
const STATE_MAGIC: &[u8; 4] = b"OTP1";
const STATE_LEN: usize = 12;
/// The big variant's state length.
const BIG_STATE_LEN: usize = 5 << 20;
/// Blocks the fail variant plays before every later `process` fails.
pub const FAIL_AFTER_BLOCKS: u64 = 4;
/// The period of the timer the timer variant registers.
pub const TIMER_PERIOD_MS: u32 = 100;
const GAIN_PARAM: u32 = 0;

/// The parameter value both plugins share between their threads, and how many times a state
/// load has replaced it.
///
/// Why a load counter: the audio processor keeps its own copy of the gain, so that a parameter
/// event changes it at its exact frame; a load made on the main thread reaches it at the start
/// of its next block, when it sees the counter has moved.
pub struct Shared {
    gain_bits: AtomicU64,
    loads: AtomicU64,
}

impl Shared {
    fn new() -> Self {
        Self {
            gain_bits: AtomicU64::new(1.0f64.to_bits()),
            loads: AtomicU64::new(0),
        }
    }

    fn gain(&self) -> f64 {
        f64::from_bits(self.gain_bits.load(Ordering::Acquire))
    }

    fn set_gain(&self, gain: f64) {
        self.gain_bits.store(gain.to_bits(), Ordering::Release);
    }
}

impl PluginShared<'_> for Shared {}

/// Which plugin an instance is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Sine,
    Gain,
    Big,
    Fail,
    Timer,
    /// The sine with an interface.
    Gui,
}

impl Kind {
    /// The sine and its variants play notes; the gain processes audio.
    fn plays_notes(self) -> bool {
        self != Kind::Gain
    }

    fn state_len(self) -> usize {
        if self == Kind::Big {
            BIG_STATE_LEN
        } else {
            STATE_LEN
        }
    }
}

pub struct MainThread<'a> {
    shared: &'a Shared,
    kind: Kind,
    host: HostMainThreadHandle<'a>,
    /// The host's parameter callbacks: a state load tells it the values changed.
    host_params: Option<HostParams>,
    /// The interface, the GUI file's sine's alone.
    #[cfg(target_os = "linux")]
    gui: Option<gui::Gui>,
}

impl<'a> MainThread<'a> {
    fn new(host: HostMainThreadHandle<'a>, shared: &'a Shared, kind: Kind) -> Self {
        let host_params = host.get_extension();
        if kind == Kind::Timer {
            if let Some(timer) = host.get_extension::<HostTimer>() {
                // A host that refuses is one the variant has nothing to show.
                let _ = timer.register_timer(&host, TIMER_PERIOD_MS);
            }
        }
        Self {
            shared,
            kind,
            #[cfg(target_os = "linux")]
            gui: (kind == Kind::Gui).then(|| gui::Gui::new(host.shared())),
            host,
            host_params,
        }
    }
}

impl<'a> PluginMainThread<'a, Shared> for MainThread<'a> {}

// The timer variant's timer has no work to do: what a test looks at is the host serving it.
impl PluginTimerImpl for MainThread<'_> {
    fn on_timer(&self, _timer_id: TimerId) {}
}

/// A gain value as the parameter takes it: inside 0..1, and the default for NaN.
fn clamp_gain(value: f64) -> f64 {
    if value.is_nan() {
        1.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// The gain a parameter event sets, if `event` is one for the gain parameter.
fn gain_event(event: &UnknownEvent) -> Option<f64> {
    match event.as_core_event()? {
        CoreEventSpace::ParamValue(value)
            if value.param_id().map(|id| id.get()) == Some(GAIN_PARAM) =>
        {
            Some(clamp_gain(value.value()))
        }
        _ => None,
    }
}

impl PluginMainThreadParams for MainThread<'_> {
    fn count(&self) -> u32 {
        1
    }

    fn get_info(&self, param_index: u32, info: &mut ParamInfoWriter) {
        if param_index != 0 {
            return;
        }
        info.set(&ParamInfo {
            id: ClapId::new(GAIN_PARAM),
            flags: ParamInfoFlags::IS_AUTOMATABLE,
            cookie: Cookie::default(),
            name: b"Gain",
            module: b"",
            min_value: 0.0,
            max_value: 1.0,
            default_value: 1.0,
        });
    }

    fn get_value(&self, param_id: ClapId) -> Option<f64> {
        (param_id.get() == GAIN_PARAM).then(|| self.shared.gain())
    }

    fn value_to_text(
        &self,
        param_id: ClapId,
        value: f64,
        writer: &mut ParamDisplayWriter,
    ) -> std::fmt::Result {
        if param_id.get() != GAIN_PARAM {
            return Err(std::fmt::Error);
        }
        write!(writer, "{value:.2}")
    }

    fn text_to_value(&self, param_id: ClapId, text: &CStr) -> Option<f64> {
        if param_id.get() != GAIN_PARAM {
            return None;
        }
        text.to_str().ok()?.trim().parse().ok().map(clamp_gain)
    }

    fn flush(&self, input: &InputEvents, _output: &mut OutputEvents) {
        for gain in input.iter().filter_map(gain_event) {
            self.shared.set_gain(gain);
            self.shared.loads.fetch_add(1, Ordering::AcqRel);
        }
    }
}

impl PluginStateImpl for MainThread<'_> {
    fn save(&self, output: &mut OutputStream) -> Result<(), PluginError> {
        output.write_all(STATE_MAGIC)?;
        output.write_all(&self.shared.gain().to_le_bytes())?;
        let padding = (self.kind.state_len() - STATE_LEN) as u64;
        io::copy(&mut io::repeat(0).take(padding), output)?;
        Ok(())
    }

    fn load(&self, input: &mut InputStream) -> Result<(), PluginError> {
        let mut bytes = Vec::with_capacity(self.kind.state_len());
        input.read_to_end(&mut bytes)?;
        let gain = match bytes.get(STATE_MAGIC.len()..STATE_LEN) {
            Some(value)
                if bytes.starts_with(STATE_MAGIC) && bytes.len() == self.kind.state_len() =>
            {
                let mut gain = [0; 8];
                gain.copy_from_slice(value);
                f64::from_le_bytes(gain)
            }
            _ => return Err(PluginError::Message("not a Dawsome test plugin state")),
        };
        self.shared.set_gain(clamp_gain(gain));
        self.shared.loads.fetch_add(1, Ordering::AcqRel);
        // The values the host reads changed under it: CLAP asks for a rescan of them.
        if let Some(params) = &self.host_params {
            params.rescan(&self.host, ParamRescanFlags::VALUES);
        }
        Ok(())
    }
}

impl PluginAudioPortsImpl for MainThread<'_> {
    fn count(&self, is_input: bool) -> u32 {
        u32::from(!(is_input && self.kind.plays_notes()))
    }

    fn get(&self, index: u32, is_input: bool, writer: &mut AudioPortInfoWriter) {
        if index != 0 || (self.kind.plays_notes() && is_input) {
            return;
        }
        writer.set(&AudioPortInfo {
            id: ClapId::new(0),
            name: if is_input { b"Input" } else { b"Output" },
            channel_count: 2,
            flags: AudioPortFlags::IS_MAIN,
            port_type: Some(AudioPortType::STEREO),
            in_place_pair: None,
        });
    }
}

impl PluginNotePortsImpl for MainThread<'_> {
    fn count(&self, is_input: bool) -> u32 {
        u32::from(self.kind.plays_notes() && is_input)
    }

    fn get(&self, index: u32, is_input: bool, writer: &mut NotePortInfoWriter) {
        if index != 0 || !is_input || !self.kind.plays_notes() {
            return;
        }
        writer.set(&NotePortInfo {
            id: ClapId::new(0),
            name: b"Notes",
            supported_dialects: NoteDialects::CLAP | NoteDialects::MIDI,
            preferred_dialect: Some(NoteDialect::Clap),
        });
    }
}

/// The gain a processor plays with, its own copy: parameter events change it at their frame,
/// state loads at the start of the next block.
struct Gain<'a> {
    shared: &'a Shared,
    value: f64,
    loads_seen: u64,
}

impl<'a> Gain<'a> {
    fn new(shared: &'a Shared) -> Self {
        Self {
            shared,
            value: shared.gain(),
            loads_seen: shared.loads.load(Ordering::Acquire),
        }
    }

    /// Takes a state load the main thread made since the last block.
    fn take_load(&mut self) {
        let loads = self.shared.loads.load(Ordering::Acquire);
        if loads != self.loads_seen {
            self.loads_seen = loads;
            self.value = self.shared.gain();
        }
    }

    fn set(&mut self, value: f64) {
        self.value = value;
        self.shared.set_gain(value);
    }
}

pub struct SineProcessor<'a> {
    gain: Gain<'a>,
    sample_rate: f64,
    note: Option<SineNote>,
    /// The pitch bend's ratio, 1 at rest: what the next note starts at.
    bend: f64,
    /// The fail variant's blocks before it fails.
    fail_after: Option<u64>,
    blocks: u64,
}

/// The sounding note: its key, velocity, frequency, frames played since its note-on, and where a
/// pitch bend moved its phase: from frame `from` it runs at `frequency·ratio` from the phase sum
/// `base` (`None` while no bend moved it: the unbent formula, to its last bit).
#[derive(Clone, Copy)]
struct SineNote {
    key: i16,
    velocity: f64,
    frequency: f64,
    elapsed: u64,
    bent: Option<Bent>,
}

#[derive(Clone, Copy)]
struct Bent {
    ratio: f64,
    base: f64,
    from: u64,
}

impl SineNote {
    /// The phase sum (cycles times the rate) at frame `n` of the note.
    fn sum(&self, n: u64) -> f64 {
        match self.bent {
            None => self.frequency * n as f64,
            Some(bent) => bent.base + self.frequency * bent.ratio * (n - bent.from) as f64,
        }
    }
}

/// The ratio pitch bend `bytes` (En ll mm) plays at: 2^(b/49152), b = ll + 128·mm − 8192.
fn bend_ratio(bytes: &[u8]) -> Option<f64> {
    match *bytes {
        [status, low, high, ..] if status & 0xF0 == 0xE0 => {
            let b = i32::from(low) + 128 * i32::from(high) - 8192;
            Some(2f64.powf(f64::from(b) / 49152.0))
        }
        _ => None,
    }
}

impl SineProcessor<'_> {
    fn apply(&mut self, event: &UnknownEvent) {
        if let Some(gain) = gain_event(event) {
            self.gain.set(gain);
            return;
        }
        match event.as_core_event() {
            Some(CoreEventSpace::NoteOn(on)) => {
                let key = on.pckn().raw_key();
                // Left to right in f64, as the formula is written.
                let frequency = 440.0 * 2f64.powf(f64::from(key - 69) / 12.0);
                let bent = (self.bend != 1.0).then_some(Bent {
                    ratio: self.bend,
                    base: 0.0,
                    from: 0,
                });
                self.note = Some(SineNote {
                    key,
                    velocity: on.velocity(),
                    frequency,
                    elapsed: 0,
                    bent,
                });
            }
            Some(CoreEventSpace::Midi(midi)) => {
                let Some(ratio) = bend_ratio(&midi.data()) else {
                    return;
                };
                self.bend = ratio;
                if let Some(note) = &mut self.note {
                    let n = note.elapsed;
                    note.bent = Some(Bent {
                        ratio,
                        base: note.sum(n),
                        from: n,
                    });
                }
            }
            Some(CoreEventSpace::NoteOff(off)) => {
                if self
                    .note
                    .is_some_and(|note| note.key == off.pckn().raw_key())
                {
                    self.note = None;
                }
            }
            _ => {}
        }
    }

    /// The next output frame, advancing the note.
    fn next_value(&mut self) -> f32 {
        let Some(note) = &mut self.note else {
            return 0.0;
        };
        let phase = match note.bent {
            None => TAU * note.frequency * note.elapsed as f64 / self.sample_rate,
            Some(_) => TAU * note.sum(note.elapsed) / self.sample_rate,
        };
        note.elapsed += 1;
        (self.gain.value * note.velocity * phase.cos()) as f32
    }
}

impl<'a> PluginAudioProcessor<'a, Shared, MainThread<'a>> for SineProcessor<'a> {
    fn activate(
        _host: HostAudioProcessorHandle<'a>,
        main_thread: &MainThread<'a>,
        shared: &'a Shared,
        audio_config: PluginAudioConfiguration,
    ) -> Result<Self, PluginError> {
        Ok(Self {
            gain: Gain::new(shared),
            sample_rate: audio_config.sample_rate,
            note: None,
            bend: 1.0,
            fail_after: (main_thread.kind == Kind::Fail).then_some(FAIL_AFTER_BLOCKS),
            blocks: 0,
        })
    }

    fn process(
        &mut self,
        _process: Process,
        mut audio: Audio,
        events: Events,
    ) -> Result<ProcessStatus, PluginError> {
        if self.fail_after.is_some_and(|after| self.blocks >= after) {
            return Err(PluginError::Message("the fail variant fails"));
        }
        self.blocks = self.blocks.saturating_add(1);
        self.gain.take_load();
        let frames = audio.frames_count();
        let mut port = audio
            .output_port(0)
            .ok_or(PluginError::Message("no output port"))?;
        let mut channels = port
            .channels()?
            .into_f32()
            .ok_or(PluginError::Message("no 32-bit output"))?;
        let mut sides = channels.iter_mut();
        let (Some(left), Some(right)) = (sides.next(), sides.next()) else {
            return Err(PluginError::Message("the output is not stereo"));
        };
        let mut pending = events.input.iter().peekable();
        for frame in 0..frames {
            while let Some(event) = pending.next_if(|event| event.header().time() <= frame) {
                self.apply(event);
            }
            let value = self.next_value();
            let index = frame as usize;
            left[index] = value;
            right[index] = value;
        }
        // Events stamped past the block (a host error) still take effect.
        for event in pending {
            self.apply(event);
        }
        Ok(ProcessStatus::Continue)
    }

    fn reset(&mut self) {
        self.note = None;
    }
}

impl PluginAudioProcessorParams for SineProcessor<'_> {
    fn flush(&mut self, input: &InputEvents, _output: &mut OutputEvents) {
        for gain in input.iter().filter_map(gain_event) {
            self.gain.set(gain);
        }
    }
}

pub struct GainProcessor<'a> {
    gain: Gain<'a>,
}

impl GainProcessor<'_> {
    /// Frames `from..to` of every channel pair, scaled by the gain in force.
    fn scale(&self, audio: &mut Audio, from: usize, to: usize) -> Result<(), PluginError> {
        if from >= to {
            return Ok(());
        }
        let gain = self.gain.value;
        for mut pair in audio.port_pairs() {
            let Some(channels) = pair.channels()?.into_f32() else {
                continue;
            };
            for channel in channels {
                match channel {
                    ChannelPair::InputOutput(input, output) => {
                        for (out, sample) in output[from..to].iter_mut().zip(&input[from..to]) {
                            *out = (f64::from(*sample) * gain) as f32;
                        }
                    }
                    ChannelPair::InPlace(buffer) => {
                        for sample in &mut buffer[from..to] {
                            *sample = (f64::from(*sample) * gain) as f32;
                        }
                    }
                    ChannelPair::OutputOnly(output) => output[from..to].fill(0.0),
                    ChannelPair::InputOnly(_) => {}
                }
            }
        }
        Ok(())
    }
}

impl<'a> PluginAudioProcessor<'a, Shared, MainThread<'a>> for GainProcessor<'a> {
    fn activate(
        _host: HostAudioProcessorHandle<'a>,
        _main_thread: &MainThread<'a>,
        shared: &'a Shared,
        _audio_config: PluginAudioConfiguration,
    ) -> Result<Self, PluginError> {
        Ok(Self {
            gain: Gain::new(shared),
        })
    }

    fn process(
        &mut self,
        _process: Process,
        mut audio: Audio,
        events: Events,
    ) -> Result<ProcessStatus, PluginError> {
        self.gain.take_load();
        let frames = audio.frames_count() as usize;
        let mut from = 0;
        for event in events.input {
            let Some(gain) = gain_event(event) else {
                continue;
            };
            let at = (event.header().time() as usize).min(frames);
            self.scale(&mut audio, from, at)?;
            from = from.max(at);
            self.gain.set(gain);
        }
        self.scale(&mut audio, from, frames)?;
        Ok(ProcessStatus::Continue)
    }
}

impl PluginAudioProcessorParams for GainProcessor<'_> {
    fn flush(&mut self, input: &InputEvents, _output: &mut OutputEvents) {
        for gain in input.iter().filter_map(gain_event) {
            self.gain.set(gain);
        }
    }
}

pub struct SinePlugin;

impl Plugin for SinePlugin {
    type AudioProcessor<'a> = SineProcessor<'a>;
    type Shared<'a> = Shared;
    type MainThread<'a> = MainThread<'a>;

    fn declare_extensions(builder: &mut PluginExtensions<Self>, _shared: Option<&Shared>) {
        builder
            .register::<PluginAudioPorts>()
            .register::<PluginNotePorts>()
            .register::<PluginParams>()
            .register::<PluginState>()
            .register::<PluginTimer>();
    }
}

/// The sine with an interface: its extensions and, on Linux, `gui`.
pub struct GuiSinePlugin;

impl Plugin for GuiSinePlugin {
    type AudioProcessor<'a> = SineProcessor<'a>;
    type Shared<'a> = Shared;
    type MainThread<'a> = MainThread<'a>;

    fn declare_extensions(builder: &mut PluginExtensions<Self>, _shared: Option<&Shared>) {
        builder
            .register::<PluginAudioPorts>()
            .register::<PluginNotePorts>()
            .register::<PluginParams>()
            .register::<PluginState>()
            .register::<PluginTimer>();
        #[cfg(target_os = "linux")]
        builder.register::<clack_extensions::gui::PluginGui>();
    }
}

pub struct GainPlugin;

impl Plugin for GainPlugin {
    type AudioProcessor<'a> = GainProcessor<'a>;
    type Shared<'a> = Shared;
    type MainThread<'a> = MainThread<'a>;

    fn declare_extensions(builder: &mut PluginExtensions<Self>, _shared: Option<&Shared>) {
        builder
            .register::<PluginAudioPorts>()
            .register::<PluginParams>()
            .register::<PluginState>();
    }
}

/// Which plugins a copy of the library offers, by its file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum File {
    /// The sine and the gain.
    Main,
    /// The big-state, failing and timer sines.
    Variants,
    /// The sine with an interface.
    Gui,
}

/// The factory listing the file's plugins: the sine and the gain, the variants, or the sine with
/// an interface.
pub struct TestFactory {
    plugins: Vec<(PluginDescriptor, Kind)>,
}

impl TestFactory {
    fn new(file: File) -> Self {
        use clack_plugin::plugin::features::{AUDIO_EFFECT, INSTRUMENT, STEREO, SYNTHESIZER};
        let describe = |id, name| {
            PluginDescriptor::new(id, name)
                .with_vendor("Dawsome")
                .with_version("1.0.0")
        };
        let instrument =
            |id, name| describe(id, name).with_features([INSTRUMENT, SYNTHESIZER, STEREO]);
        let plugins = match file {
            File::Variants => vec![
                (instrument(BIG_ID, "Dawsome Test Big State"), Kind::Big),
                (instrument(FAIL_ID, "Dawsome Test Fail"), Kind::Fail),
                (instrument(TIMER_ID, "Dawsome Test Timer"), Kind::Timer),
            ],
            File::Gui => vec![(instrument(GUI_ID, "Dawsome Test GUI"), Kind::Gui)],
            File::Main => vec![
                (instrument(SINE_ID, "Dawsome Test Sine"), Kind::Sine),
                (
                    describe(GAIN_ID, "Dawsome Test Gain").with_features([AUDIO_EFFECT, STEREO]),
                    Kind::Gain,
                ),
            ],
        };
        Self { plugins }
    }
}

impl PluginFactoryImpl for TestFactory {
    fn plugin_count(&self) -> u32 {
        u32::try_from(self.plugins.len()).unwrap_or(u32::MAX)
    }

    fn plugin_descriptor(&self, index: u32) -> Option<&PluginDescriptor> {
        let (descriptor, _) = self.plugins.get(usize::try_from(index).ok()?)?;
        Some(descriptor)
    }

    fn create_plugin<'a>(
        &'a self,
        host_info: HostInfo<'a>,
        plugin_id: &CStr,
    ) -> Option<PluginInstance<'a>> {
        let shared = |_host: HostSharedHandle<'a>| Ok(Shared::new());
        let (descriptor, kind) = self
            .plugins
            .iter()
            .find(|(descriptor, _)| descriptor.id() == Some(plugin_id))?;
        let kind = *kind;
        let main_thread = move |host, shared| Ok(MainThread::new(host, shared, kind));
        Some(match kind {
            Kind::Gain => {
                PluginInstance::new::<GainPlugin>(host_info, descriptor, shared, main_thread)
            }
            Kind::Sine | Kind::Big | Kind::Fail | Kind::Timer => {
                PluginInstance::new::<SinePlugin>(host_info, descriptor, shared, main_thread)
            }
            Kind::Gui => {
                PluginInstance::new::<GuiSinePlugin>(host_info, descriptor, shared, main_thread)
            }
        })
    }
}

pub struct TestEntry {
    factory: PluginFactoryWrapper<TestFactory>,
}

impl Entry for TestEntry {
    fn new(bundle_path: Option<&CStr>) -> Result<Self, EntryLoadError> {
        // Why the file's name decides: one build makes every file, and a file of their own
        // keeps the variants and the interface out of every scan of the main one. The path is
        // UTF-8 by the CLAP standard; a byte that is not leaves the file name, which is ASCII,
        // as it is.
        let name = bundle_path.and_then(|path| {
            let path = String::from_utf8_lossy(path.to_bytes());
            Path::new(path.as_ref())
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        });
        let file = match name.as_deref() {
            Some(VARIANTS_FILE) => File::Variants,
            Some(GUI_FILE) => File::Gui,
            _ => File::Main,
        };
        Ok(Self {
            factory: PluginFactoryWrapper::new(TestFactory::new(file)),
        })
    }

    fn declare_factories<'a>(&'a self, builder: &mut EntryFactories<'a>) {
        builder.register_factory(&self.factory);
    }
}

clack_export_entry!(TestEntry);
