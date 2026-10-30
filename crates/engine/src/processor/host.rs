//! The plugin-host port: what the engine and the app ask of a plugin format's host (CLAP, VST3),
//! in the engine's own words, so neither ever sees a plugin ABI type.
//!
//! A host lives on the UI thread, which is the plugins' main thread: every method here runs
//! there. The processors it creates move to the audio thread like any other.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use thiserror::Error;

use super::Processor;
use crate::project::{ProcessorOwner, ProcessorRef};

/// Why a host could not give a plugin's processor, or do what was asked of an instance.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HostError {
    /// The plugin is not installed, or no instance plays for that `owner`.
    #[error("not found")]
    Missing,
    /// It is there but failed: it would not load, refused its state, or crashed in a scan.
    #[error("{0}")]
    Failed(String),
}

/// One parameter of a plugin instance, as the app's parameter window lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginParam {
    /// The plugin's own id for it, what [`crate::transport::ParamTarget::Plugin`] names.
    pub id: u32,
    pub name: String,
    /// The plugin's grouping of it, such as "Filter/Envelope"; empty for none.
    pub group: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    /// Only whole steps between `min` and `max` are values.
    pub stepped: bool,
    pub hidden: bool,
    pub read_only: bool,
}

/// A plugin instance's state as the document stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    /// The plugin's own bytes as standard padded base64, the text a
    /// [`ProcessorRef::state`] holds.
    pub state: String,
    /// How many bytes the plugin wrote, before encoding.
    pub bytes: usize,
}

/// What opening a plugin's window opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Editor {
    /// The plugin's own interface, inside a window of ours.
    Embedded,
    /// The plugin's own interface, in a window the plugin made.
    Floating,
    /// Nothing: the plugin has no interface it can show here, so the app shows its own panel
    /// of the plugin's parameters.
    Generic,
}

/// Something that happened to a plugin instance since the last [`ProcessorHost::tick`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostNotice {
    pub owner: ProcessorOwner,
    pub kind: NoticeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeKind {
    /// Its values changed in its own window, or it says its state changed: the document should
    /// capture the state.
    Dirty,
    /// Its window closed.
    Closed,
    /// It asks to be created again (its ports or parameter list changed).
    Restart,
    /// Its parameters' names, flags or MIDI mapping changed: a panel showing them should be built
    /// again. Why not [`Restart`](Self::Restart): a plugin renames its parameters on a program
    /// change, and creating it again would reload all of it; why not [`Dirty`](Self::Dirty): a
    /// capture rebuilds no panel, whose names would go stale. CLAP's host never sends it.
    Params,
    /// It failed in a way the user should hear about.
    Failed(String),
}

/// The host of one plugin format. Registered in a [`super::Registry`] under its format name;
/// processor ids `"<format>:<plugin id>"` resolve to it.
pub trait ProcessorHost {
    /// The format name its ids start with, such as `"clap"`.
    fn format(&self) -> &'static str;

    /// A new instance of `reference`'s plugin playing for `owner` (a channel's instrument or a
    /// slot's effect), at `sample_rate`, with the state the reference holds. The processor it
    /// returns is ready to move to the audio thread.
    fn create(
        &self,
        owner: ProcessorOwner,
        reference: &ProcessorRef,
        sample_rate: u32,
    ) -> Result<Box<dyn Processor>, HostError>;

    /// `owner`'s processor has left the plan (its channel or slot is gone, or it is created
    /// again): its instance may go once the audio half has come back. Only this lets an
    /// instance go while its session lasts; a player dropped with the processor still in the
    /// plan takes no instance with it.
    fn release(&self, owner: ProcessorOwner);

    /// The document's state for `owner` is now `reference.state` (an undo, a redo): bring the
    /// live instance in line. A state the instance already has (one just captured from it)
    /// changes nothing.
    fn sync_state(&self, owner: ProcessorOwner, reference: &ProcessorRef);

    /// The parameters of `owner`'s instance, in the plugin's order; empty when there is none.
    fn params(&self, owner: ProcessorOwner) -> Vec<PluginParam>;

    /// The value of parameter `id` of `owner`'s instance now.
    fn value(&self, owner: ProcessorOwner, id: u32) -> Option<f64>;

    /// `value` as the plugin writes it for parameter `id`, such as "-6.0 dB".
    fn value_text(&self, owner: ProcessorOwner, id: u32, value: f64) -> Option<String>;

    /// The state of `owner`'s instance now, which the host also remembers as what the document
    /// holds, so the [`sync_state`](Self::sync_state) that follows the edit changes nothing.
    fn capture_state(&self, owner: ProcessorOwner) -> Result<Captured, HostError>;

    /// How many frames late `owner`'s plugin says its sound comes out (its latency), when it
    /// says. Nothing compensates it: the command line names it, so a render's delay has a
    /// reason. By default none.
    fn latency(&self, _owner: ProcessorOwner) -> Option<u32> {
        None
    }

    /// Shows `owner`'s plugin window, titled `title`.
    fn open_editor(&self, owner: ProcessorOwner, title: &str) -> Result<Editor, HostError>;

    /// Keeps the plugin windows of the host's own (those a plugin's interface is embedded in)
    /// above every other window (`true`), or stacked as any window, now and as they open. A
    /// plugin's own floating window is the plugin's and keeps its stacking. Why: the app keeps
    /// its pop-out windows above its main window while it has the keyboard. By default nothing.
    fn keep_windows_above(&self, _above: bool) {}

    /// Runs the plugins' main-thread work that is due (their timers, file descriptors,
    /// callbacks, window events) and lets instances go that were released, or whose session
    /// ended, once their processors have come back. Call it about every 16 ms while
    /// [`needs_ticks`](Self::needs_ticks), and within about 16 ms of the
    /// [`doorbell`](Self::doorbell) ringing.
    fn tick(&self) -> Vec<HostNotice>;

    /// Whether any instance is alive.
    fn has_instances(&self) -> bool;

    /// Whether `tick` has work a new request would not announce: a plugin's timer or file
    /// descriptor, an open plugin window, a notice waiting, an instance on its way out, or a
    /// request already rung but not yet served. While it is false, `tick` has work only once
    /// the doorbell rings. By default, whenever an instance is alive.
    fn needs_ticks(&self) -> bool {
        self.has_instances()
    }

    /// Set from any thread, the audio thread included, whenever a plugin asks for main-thread
    /// work (a callback, a restart, a changed state, a failure); the caller clears it as it
    /// looks. Why: a pump that ran every 16 ms whether or not anything was due woke the window
    /// 60 times a second for as long as a plugin was loaded. `None`: the host has none, and is
    /// ticked while [`needs_ticks`](Self::needs_ticks).
    fn doorbell(&self) -> Option<Arc<AtomicBool>> {
        None
    }
}
