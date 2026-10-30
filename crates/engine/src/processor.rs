//! The processor port: what every effect and generator implements, native or plugin, and the
//! registry the compiler creates them from.
//!
//! A processor has two halves. Its [`ProcessorFactory`] (or, for plugins, its
//! [`ProcessorHost`]) stays on the UI thread: it knows the parameters and creates instances
//! there, buffers and all. The instance, a [`Processor`], moves to the audio thread and only
//! processes audio, takes parameter values and resets; it never allocates, frees, locks or logs
//! there.

mod host;

use std::collections::BTreeMap;
use std::fmt;

pub use host::{Captured, Editor, HostError, HostNotice, NoticeKind, PluginParam, ProcessorHost};

use crate::time::{MilliBpm, Ticks};

/// What a processor may need to know about the song while it processes: a tempo-synced delay
/// times its echoes from it, a plugin reads the transport, and a generator plays `events`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcessContext<'a> {
    pub sample_rate: u32,
    pub tempo: MilliBpm,
    pub ppq: u16,
    pub transport: TransportInfo,
    /// The notes that start in this block, in time order; empty for effects.
    pub events: &'a [PortEvent],
}

/// Where playback is, as of the block's first frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransportInfo {
    pub playing: bool,
    /// The song tick the block's first frame is in.
    pub song_tick: Ticks,
    /// The output clock: frames played since the last Play, Stop or Seek. It never jumps back
    /// between those, so a plugin can time its voices by it.
    pub steady_time: u64,
}

/// Something a generator is told to play in a block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PortEvent {
    /// A note that starts on frame `frame` of the block and sounds for `length` frames (at least
    /// one), so the processor can end it itself however later blocks are cut. `velocity` is
    /// 0.0..=1.0.
    Note {
        frame: u32,
        key: u8,
        velocity: f32,
        length: u64,
    },
    /// A live note (played on a MIDI device or the typing keyboard) that starts on frame
    /// `frame` and sounds until a [`PortEvent::NoteOff`] of its key: its length is not known
    /// when it starts.
    NoteOn { frame: u32, key: u8, velocity: f32 },
    /// Ends the live note of `key` on frame `frame`.
    NoteOff { frame: u32, key: u8 },
    /// A device's pitch bend moved to `value` (−8192 all the way down to 8191 all the way up,
    /// 0 at rest) on frame `frame`: the plugin bends by its own range.
    Bend { frame: u32, value: i16 },
    /// A device's mod wheel (CC 1) moved to `value` (0 to 127) on frame `frame`: the plugin
    /// does with it what its sound says.
    Wheel { frame: u32, value: u8 },
}

/// The audio half of a processor.
///
/// The contract every implementation keeps: `process`, `set_param` and `reset` do not allocate,
/// free, lock or log (everything is sized in [`ProcessorFactory::create`]); with constant
/// parameters, processing frames `[a, b)` and then `[b, c)` gives the same bits as `[a, c)`;
/// finite input gives finite output. (Third-party plugins keep the first rule as far as their
/// host's side goes; the second is theirs to keep or not.)
pub trait Processor: Send {
    /// Processes `buffer`, stereo frames, in place. A generator gets it zeroed and writes its
    /// sound into it.
    fn process(&mut self, context: &ProcessContext<'_>, buffer: &mut [[f32; 2]]);

    /// Sets the parameter at `index` (its place in the factory's [`ProcessorFactory::params`])
    /// to `value`, clamped by [`ParamInfo::clamp`]. An index past the end is ignored.
    fn set_param(&mut self, index: usize, value: f32);

    /// Clears everything still ringing (delay lines, filter memory, sounding notes);
    /// parameters stay.
    fn reset(&mut self);

    /// Sets the plugin parameter with id `id` (the plugin's own number for it) to `value` from
    /// the start of the next block. Native processors have none.
    fn queue_param(&mut self, _id: u32, _value: f32) {}

    /// Called instead of `process` while playback is stopped, so parameter changes still reach
    /// a processor that wants them.
    fn idle(&mut self, _context: &ProcessContext<'_>) {}

    /// Called on the audio thread just before the processor leaves it, as its last call there.
    fn suspend(&mut self) {}
}

/// One parameter of a processor, as the mixer shows and saves it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamInfo {
    /// The key it is saved under in a [`crate::project::ProcessorRef`].
    pub key: &'static str,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// What [`ProcessorFactory::shown`] gives is in: "Hz", "dB", "ms", "steps", "%" (a 0..1
    /// value shown as a percentage) or "".
    pub unit: &'static str,
    /// Only whole numbers are values.
    pub whole: bool,
}

impl ParamInfo {
    /// `value` as this parameter takes it: inside `min..=max`, whole when it must be, and the
    /// default for NaN, which names no value.
    pub fn clamp(&self, value: f32) -> f32 {
        if value.is_nan() {
            return self.default;
        }
        let value = value.clamp(self.min, self.max);
        if self.whole {
            value.round()
        } else {
            value
        }
    }
}

/// One value per parameter in `params`, in order: the value saved under its key, else its
/// default, clamped. Saved keys that name no parameter are left out.
pub fn param_values(params: &[ParamInfo], saved: &BTreeMap<String, f32>) -> Vec<f32> {
    params
        .iter()
        .map(|info| info.clamp(saved.get(info.key).copied().unwrap_or(info.default)))
        .collect()
}

/// The UI-thread half of a processor: its identity, its parameters, and a way to make
/// instances.
pub trait ProcessorFactory: Send + Sync {
    /// The stable key a saved effect names it by, such as `"dawsome.delay"`.
    fn id(&self) -> &'static str;

    /// The name the mixer shows.
    fn name(&self) -> &'static str;

    fn params(&self) -> &'static [ParamInfo];

    /// The number parameter `index` at `value` stands for, in its [`ParamInfo::unit`], in a
    /// project at `sample_rate`: the value itself, unless the processor maps a control to what
    /// it sets (a 0..1 control that sets a gain is shown in decibels). Why the processor's own:
    /// the mapping is the one its sound is made with, so what is shown is what is heard.
    fn shown(&self, _index: usize, value: f32, _sample_rate: u32) -> f64 {
        f64::from(value)
    }

    /// A new instance for `sample_rate`, starting at `values` (one per parameter, in
    /// [`params`](Self::params) order, already clamped). Runs on the UI thread, so it may
    /// allocate everything the instance will ever need.
    fn create(&self, sample_rate: u32, values: &[f32]) -> Box<dyn Processor>;
}

/// The processors a build can create: native ones by id ([`Registry::native`] holds the native
/// effects), and plugins through the host of their format.
///
/// Why plugin hosts sit here and not as factories: a plugin id is not known when the build is
/// made (`"clap:org.example.synth"`), and a host keeps plugin instances that must stay on the
/// thread that made them, so it lives on the UI thread with the compiler that owns this.
#[derive(Default)]
pub struct Registry {
    factories: BTreeMap<&'static str, Box<dyn ProcessorFactory>>,
    hosts: Vec<Box<dyn ProcessorHost>>,
}

impl Registry {
    /// A registry with no processors.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Adds `factory`, replacing one with the same id.
    pub fn register(&mut self, factory: impl ProcessorFactory + 'static) {
        self.factories.insert(factory.id(), Box::new(factory));
    }

    /// Adds the host of a plugin format, replacing one for the same format.
    pub fn add_host(&mut self, host: Box<dyn ProcessorHost>) {
        self.hosts.retain(|known| known.format() != host.format());
        self.hosts.push(host);
    }

    pub fn get(&self, id: &str) -> Option<&dyn ProcessorFactory> {
        self.factories.get(id).map(Box::as_ref)
    }

    /// The host that creates processor `id`, a plugin id of the form `<format>:<plugin id>`.
    pub fn host(&self, id: &str) -> Option<&dyn ProcessorHost> {
        let (format, _) = id.split_once(':')?;
        self.hosts
            .iter()
            .find(|host| host.format() == format)
            .map(Box::as_ref)
    }

    /// Every plugin host, in the order added.
    pub fn hosts(&self) -> impl Iterator<Item = &dyn ProcessorHost> {
        self.hosts.iter().map(Box::as_ref)
    }

    /// Every processor, by id.
    pub fn factories(&self) -> impl Iterator<Item = &dyn ProcessorFactory> {
        self.factories.values().map(Box::as_ref)
    }
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hosts: Vec<&str> = self.hosts.iter().map(|host| host.format()).collect();
        f.debug_struct("Registry")
            .field("factories", &self.factories.keys().collect::<Vec<_>>())
            .field("hosts", &hosts)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEPS: ParamInfo = ParamInfo {
        key: "sync_steps",
        label: "Sync",
        min: 0.0,
        max: 32.0,
        default: 0.0,
        unit: "steps",
        whole: true,
    };
    const MIX: ParamInfo = ParamInfo {
        key: "mix",
        label: "Mix",
        min: 0.0,
        max: 1.0,
        default: 0.3,
        unit: "%",
        whole: false,
    };

    #[test]
    fn values_are_clamped_rounded_when_whole_and_nan_is_the_default() {
        assert_eq!(MIX.clamp(1.5), 1.0);
        assert_eq!(MIX.clamp(-0.5), 0.0);
        assert_eq!(MIX.clamp(0.25), 0.25);
        assert_eq!(MIX.clamp(f32::NAN), 0.3);
        assert_eq!(MIX.clamp(f32::INFINITY), 1.0);
        assert_eq!(STEPS.clamp(2.6), 3.0);
        assert_eq!(STEPS.clamp(40.0), 32.0);
    }

    #[test]
    fn saved_values_fill_in_defaults_and_unknown_keys_are_left_out() {
        let saved = BTreeMap::from([("mix".to_owned(), 0.9), ("color".to_owned(), 7.0)]);
        assert_eq!(param_values(&[STEPS, MIX], &saved), [0.0, 0.9]);
        let out_of_range = BTreeMap::from([("sync_steps".to_owned(), 99.0)]);
        assert_eq!(param_values(&[STEPS, MIX], &out_of_range), [32.0, 0.3]);
    }
}
