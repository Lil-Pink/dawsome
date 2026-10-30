//! Dawsome's VST3 plugin host. The VST3 ABI (the `vst3` crate's bindings, generated from the
//! MIT VST 3 SDK's interfaces) lives only here: the engine sees plugins through its own
//! [`dawsome_engine::processor::ProcessorHost`] port, which [`Vst3Host`] implements.
//!
//! - [`scan`]: the scan child's half for `.vst3` modules ([`scan::describe_module`]); the parent
//!   half, the process tree and the one catalogue of every format are `dawsome_host_common`'s.
//! - [`Vst3Host`]: loads modules once (only as a scan child loaded them), creates component and
//!   controller on the main thread, carries values between the controller and the processor,
//!   plays a device's pitch bend and mod wheel as points of the parameters the plugin's MIDI
//!   mapping names (and leaves those parameters out of the panels), rounds a stepped
//!   parameter's value to a whole step, acts on the restarts a plugin asks for, serves the Linux
//!   run loop, opens a plugin's own editor in a window of ours (`view.rs`, with the frame it is
//!   given, `com/frame.rs`), keeps state as `.vstpreset` bytes, and gives the engine each
//!   instance's audio half as a `Processor`.
//!
//! The SDK's headers, its documentation and its hosting samples were the reference for the
//! protocol (read, not copied).
//!
//! # Unsafe code
//!
//! The VST3 ABI is C++ vtables, so every call into a plugin is unsafe: the boundary is each vtable
//! call (module factory, component, processor, controller, view), the raw pointers a plugin hands
//! our COM objects (`com/`), the module's entry points (`module.rs`) and `poll(2)`. Every block is
//! one operation under a `// SAFETY:` comment that names why its pointer is valid: a plugin object
//! we hold a reference to (the invariants on `Module`, `Parts` and `OpenView`), called on the main
//! thread unless it is the audio half's (`processor.rs`), or a plugin's argument that the interface
//! makes null or valid for the call, checked against null first. `vst3`'s safe `ComWrapper` and
//! `ComPtr` methods (`as_com_ref`, `to_com_ptr`, `cast`) stand in for raw pointers wherever they
//! can; the lints below enforce one operation per block, each with its comment. Four
//! crate-private functions trust a caller's raw pointer and are `unsafe fn` with a `# Safety`
//! section: `RunLoop::register_timer` and `register_fd` (`com/context.rs`) and
//! `AttributeList::set` and `get` (`com/message.rs`); each caller's block says why its pointer
//! is valid.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![deny(clippy::multiple_unsafe_ops_per_block)]

mod com;
mod host;
mod mapping;
mod module;
mod parts;
mod preset;
mod processor;
pub mod scan;
#[cfg(test)]
#[path = "../../../tests/support/test_plugins.rs"]
mod test_plugins;
mod uid;
mod view;

use std::collections::BTreeSet;

use dawsome_engine::project::Project;

pub use com::host_objects;
pub use host::{OpenEditor, Vst3Host, FORMAT};
pub use uid::{parse as parse_class_id, text as class_id_text};

/// The VST3 plugin ids (class id texts) `project` names, on plugin channels and in effect
/// slots.
pub fn plugin_ids(project: &Project) -> BTreeSet<String> {
    dawsome_host_common::plugin_ids(project, FORMAT)
}

/// The interfaces' enum constants are unsigned on Linux and signed on Windows, as the SDK's
/// enums are; named through these, one spelling serves both (so a cast that changes nothing on
/// one platform is the one the other needs).
#[allow(clippy::unnecessary_cast)]
mod constants {
    use vst3::Steinberg::Vst::{
        BusDirection, BusDirections_, Event_, MediaType, MediaTypes_, ParameterInfo_,
        ProcessContext_, ProcessModes_, RestartFlags_, SymbolicSampleSizes_,
    };
    use vst3::Steinberg::{PFactoryInfo_, TBool};

    pub(crate) const AUDIO: MediaType = MediaTypes_::kAudio as MediaType;
    pub(crate) const EVENT: MediaType = MediaTypes_::kEvent as MediaType;
    pub(crate) const INPUT: BusDirection = BusDirections_::kInput as BusDirection;
    pub(crate) const OUTPUT: BusDirection = BusDirections_::kOutput as BusDirection;
    pub(crate) const NOTE_ON: u16 = Event_::EventTypes_::kNoteOnEvent as u16;
    pub(crate) const NOTE_OFF: u16 = Event_::EventTypes_::kNoteOffEvent as u16;
    pub(crate) const LIVE: u16 = Event_::EventFlags_::kIsLive as u16;
    pub(crate) const SAMPLE_32: i32 = SymbolicSampleSizes_::kSample32 as i32;
    pub(crate) const REALTIME: i32 = ProcessModes_::kRealtime as i32;
    pub(crate) const UNICODE: i32 = PFactoryInfo_::FactoryFlags_::kUnicode as i32;
    pub(crate) const READ_ONLY: i32 = ParameterInfo_::ParameterFlags_::kIsReadOnly as i32;
    pub(crate) const HIDDEN: i32 = ParameterInfo_::ParameterFlags_::kIsHidden as i32;
    pub(crate) const RELOAD: i32 = RestartFlags_::kReloadComponent as i32;
    pub(crate) const IO_CHANGED: i32 = RestartFlags_::kIoChanged as i32;
    pub(crate) const VALUES_CHANGED: i32 = RestartFlags_::kParamValuesChanged as i32;
    pub(crate) const LATENCY_CHANGED: i32 = RestartFlags_::kLatencyChanged as i32;
    pub(crate) const TITLES_CHANGED: i32 = RestartFlags_::kParamTitlesChanged as i32;
    pub(crate) const MAPPING_CHANGED: i32 = RestartFlags_::kMidiCCAssignmentChanged as i32;
    pub(crate) const PLAYING: u32 = ProcessContext_::StatesAndFlags_::kPlaying as u32;
    pub(crate) const TEMPO_VALID: u32 = ProcessContext_::StatesAndFlags_::kTempoValid as u32;
    pub(crate) const MUSIC_VALID: u32 =
        ProcessContext_::StatesAndFlags_::kProjectTimeMusicValid as u32;
    pub(crate) const BAR_VALID: u32 = ProcessContext_::StatesAndFlags_::kBarPositionValid as u32;
    pub(crate) const SIGNATURE_VALID: u32 = ProcessContext_::StatesAndFlags_::kTimeSigValid as u32;
    pub(crate) const CONTINUOUS_VALID: u32 =
        ProcessContext_::StatesAndFlags_::kContTimeValid as u32;
    pub(crate) const ON: TBool = 1;
    pub(crate) const OFF: TBool = 0;
}
