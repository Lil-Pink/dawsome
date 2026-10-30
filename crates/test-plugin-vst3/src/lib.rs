//! Dawsome's own VST3 test plugins, the VST3 host's oracles: their output follows exact formulas,
//! the CLAP test plugins' (crates/test-plugin), so a known-answer test needs no third-party
//! binary, and the same frames come out of either format. One module, `Dawsome Test VST3.vst3`,
//! with these classes (each id is the ASCII of a 16-byte name, so its text is that ASCII in hex
//! on every platform):
//!
//! - `DawsomeTestSine1`, "Dawsome Test Sine" (instrument): `y[S0+n] = f32(gain·velocity·cos(τ·f·n/sr))`
//!   from the note-on frame `S0` up to the frame before the note-off, `f = 440·2^((key−69)/12)`,
//!   `n` counted from the note-on, 0.0 elsewhere. Monophonic: a note-on replaces the note
//!   sounding; a note-off ends the note whose id it names (by key when it names none).
//!   `setProcessing(false)` and `setActive(false)` silence it. Its controller is a class of its
//!   own, `DawsomeTestCtrl1`, joined to it through the host's connection: at `connect` the
//!   controller sends the component a message "hello" (integer attribute "n" = 7), the component
//!   answers "ack" with n + 1, and on an "ack" of 8 the controller sets its read-only parameter 9
//!   "Linked" to 1.0, which proves both directions of the connection and the host's messages and
//!   attribute lists. Its controller also asks the host for `restartComponent(kParamValuesChanged)`
//!   inside `setComponentState`, as real plugins do, so the host is called back in the middle of
//!   its own call. Its controller answers `IMidiMapping` as JUCE's do: on MIDI bus 0, channel 0,
//!   pitch bend (controller 129) moves parameter 1 "Bend" (default 8192/16383) and the mod wheel
//!   (controller 1) parameter 2 "Wheel" (default 0), both with no flags; nothing else is mapped.
//!   Its processor bends as the CLAP test sine does: a point `v` of "Bend" on frame `k` is
//!   `b = round(v·16383) − 8192`, and from frame `k` the note reads on from where its phase is at
//!   `f·2^(b/49152)`: `y = f32(gain·velocity·cos(τ·(P + f·r·(n − k))/sr))`, `P` the phase sum at
//!   `k` (`f·k` while unbent, then the last bend's), `r` the ratio; a bend with no note sounding
//!   bends the next note from its start. A point of "Wheel" it reports back as an output change
//!   of read-only parameter 3 "Wheel heard".
//! - `DawsomeTestGain1`, "Dawsome Test Gain" (effect): `y = f32(f64(x)·gain)`, its own controller.
//!   Parameter 1 "Steps" has [`STEPS`] steps (default 1.0, the last; text "k/4", `k =
//!   min(4, floor(v·5))`, the format's decoding), and what its processor is given of it (a flush
//!   of no frames included) it reports back as read-only parameter 2 "Steps heard". Steps is not
//!   saved: the state is the 12 bytes every class has.
//! - `DawsomeTestFail1`, "Dawsome Test Fail": the sine as its own controller, which returns
//!   `kResultFalse` from every `process` after [`FAIL_AFTER`] calls.
//! - `DawsomeTestTick1`, "Dawsome Test Tick": the sine as its own controller, which on Linux
//!   registers a [`TIMER_MS`] timer with the run loop its `initialize` context offers; read-only
//!   parameter 8 "Ticks" = timer calls / 1000. On its first timer call the controller edits its
//!   own gain to [`TICK_EDIT`] (`beginEdit`, `performEdit`, `endEdit`), as an editor would, so
//!   the host's carrying of a controller's edit to the processor is heard.
//! - `DawsomeTestLate1`, "Dawsome Test Latency" (effect): the gain on its input [`LATENCY`] frames
//!   late, `y[n] = f32(f64(x[n − 64])·gain)`, reporting a latency of 64 frames; asked to restart
//!   with `kLatencyChanged` ([`dawsome_test_vst3_restart`]) it reports and plays
//!   [`LATENCY_AFTER_RESTART`] from its next `process`. It also reports
//!   the transport each block was given, as output changes of its read-only parameters 10
//!   "Tempo" (BPM), 11 "Beats" (`projectTimeMusic`), 12 "Bar" (`barPositionMusic`), 13
//!   "Playing" (1 or 0), 14 "Samples" (`projectTimeSamples`) and 15 "Clock"
//!   (`continousTimeSamples`), each divided by [`READOUT_DIVISOR`] (2^24, exact) so that it is a
//!   normalized value, which its controller takes back from the host (`setParamNormalized`) and
//!   gives out: the host's carrying of a processor's own changes to the controller is then
//!   seen. Its read-only parameter 16 "Wild" reports, on its audio calls 0, 1, 2, 3, ..., the
//!   values [`WILD`] gives in turn, out of a normalized value's range on purpose (below 0,
//!   above 1, NaN), and its controller keeps what the host gives it as it is: what a host
//!   passes on of a plugin's bad output values is then seen.
//! - `DawsomeTestView1`, "Dawsome Test View" (instrument): the sine (its sound, parameters, link,
//!   MIDI mapping and state) with a controller of its own, `DawsomeTestCtrl2`, whose
//!   `createView("editor")` makes an editor on Linux (`view.rs`; any other name, or another
//!   platform: none). The editor logs each call its host makes into it, and the View's parts log
//!   their `terminate` ("controller terminate", "component terminate"), into the log the
//!   `dawsome_test_vst3_view_*` exports read; a class of its own, so no other test's plugin ever
//!   opens a window.
//!
//! Every class has parameter 0 "Gain" (normalized value = gain, 0..1, default 1), which a point
//! changes at its own sample offset. The component's state is `b"OTP1"` + gain as f64
//! little-endian (12 bytes), anything else refused; the sine's controller's state is `b"OTC1"` +
//! u32 0 (8 bytes); a class that is its own controller writes no controller state. Nothing
//! allocates while it processes.
//!
//! For the host's reference-count test the module exports `dawsome_test_vst3_live_objects()`,
//! how many of its COM objects are alive, and `dawsome_test_vst3_module_entered()`, how many times
//! `ModuleEntry` was called less how many times `ModuleExit` was. For the host's restart tests it
//! exports [`dawsome_test_vst3_restart`]: every controller made on the caller's thread asks its
//! host to restart with the flags given, as a plugin does on its main thread. For the window
//! tests: the View's log (`dawsome_test_vst3_view_log`, `..._view_log_clear`) and its editor's
//! (`view.rs`).
//!
//! # Unsafe code
//!
//! A VST3 plugin is called through C++ vtables with raw pointers, so each interface method is an
//! `unsafe fn` (the `vst3` traits declare them so) and each pointer use inside is a block of its
//! own. The contract every `// SAFETY:` comment here leans on: the host keeps the interface's
//! rules, so a pointer it passes is null or valid for the call (null is checked first), a buffer
//! has the size it states, and `process`'s output buffers are that call's alone (the Dawsome host
//! gives inputs and outputs apart, never in place). The lints below enforce one operation per
//! block, each with its comment.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![deny(clippy::multiple_unsafe_ops_per_block)]

use std::cell::RefCell;
use std::f64::consts::TAU;
use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

#[cfg(target_os = "linux")]
use vst3::Steinberg::Linux::{IRunLoop, IRunLoopTrait, ITimerHandler, ITimerHandlerTrait};
use vst3::Steinberg::Vst::{
    BusDirection, BusDirections_, BusInfo, BusInfo_, BusTypes_, Event, Event_, IAttributeListTrait,
    IAudioProcessor, IAudioProcessorTrait, IComponent, IComponentHandler, IComponentHandlerTrait,
    IComponentTrait, IConnectionPoint, IConnectionPointTrait, IEditController,
    IEditControllerTrait, IEventListTrait, IHostApplication, IHostApplicationTrait, IMessage,
    IMessageTrait, IMessage_iid, IMidiMapping, IMidiMappingTrait, IParamValueQueue,
    IParamValueQueueTrait, IParameterChanges, IParameterChangesTrait, IoMode, MediaType,
    MediaTypes_, ParamID, ParamValue, ParameterInfo, ParameterInfo_, ProcessContext_, ProcessData,
    ProcessSetup, RestartFlags_, RoutingInfo, SpeakerArr, SpeakerArrangement, String128,
    SymbolicSampleSizes_, TChar,
};
use vst3::Steinberg::{int16, int32};
use vst3::Steinberg::{
    kInvalidArgument, kNotImplemented, kResultFalse, kResultOk, kResultTrue, FIDString, FUnknown,
    IBStream, IBStreamTrait, IPlugView, IPluginBaseTrait, IPluginFactory, IPluginFactory2Trait,
    IPluginFactory3, IPluginFactory3Trait, IPluginFactoryTrait, PClassInfo, PClassInfo2,
    PClassInfoW, PClassInfo_, PFactoryInfo, PFactoryInfo_, TBool, TUID,
};
use vst3::{uid, Class, ComPtr, ComRef, ComWrapper};

#[cfg(target_os = "linux")]
mod view;

/// The class id of the 16 ASCII characters of `name`, made as the SDK's `INLINE_UID` makes one
/// from four 32-bit words, so `FUID::toString` prints the same 32 hex digits on every platform
/// (Windows keeps the first words in another byte order and prints them as integers).
const fn ascii_uid(name: &[u8; 16]) -> TUID {
    const fn word(name: &[u8; 16], at: usize) -> u32 {
        (name[at] as u32) << 24
            | (name[at + 1] as u32) << 16
            | (name[at + 2] as u32) << 8
            | name[at + 3] as u32
    }
    uid(word(name, 0), word(name, 4), word(name, 8), word(name, 12))
}

pub const SINE_CID: TUID = ascii_uid(b"DawsomeTestSine1");
pub const CONTROLLER_CID: TUID = ascii_uid(b"DawsomeTestCtrl1");
pub const GAIN_CID: TUID = ascii_uid(b"DawsomeTestGain1");
pub const FAIL_CID: TUID = ascii_uid(b"DawsomeTestFail1");
pub const TICK_CID: TUID = ascii_uid(b"DawsomeTestTick1");
pub const LATENCY_CID: TUID = ascii_uid(b"DawsomeTestLate1");
pub const VIEW_CID: TUID = ascii_uid(b"DawsomeTestView1");
pub const VIEW_CONTROLLER_CID: TUID = ascii_uid(b"DawsomeTestCtrl2");

/// Calls of `process` the Fail class plays before every later one fails.
pub const FAIL_AFTER: u64 = 4;
/// The period of the timer the Tick class registers.
pub const TIMER_MS: u64 = 100;
/// How late the Latency class plays its input, in frames, as it reports.
pub const LATENCY: usize = 64;
/// How late it plays and reports once asked to restart with `kLatencyChanged`: its delay line's
/// whole length.
pub const LATENCY_AFTER_RESTART: usize = 128;
/// The steps of the gain's parameter 1 "Steps".
pub const STEPS: i32 = 4;
/// The title parameter 0 takes once its controller is asked to restart with
/// `kParamTitlesChanged`.
pub const RENAMED: &str = "Level";
/// The gain the Tick class's controller sets on its first timer call.
pub const TICK_EDIT: f64 = 0.125;
/// What the Latency class reports as parameter 16 "Wild" on its audio calls, in turn.
pub const WILD: [f64; 3] = [-0.5, 1.5, f64::NAN];
/// The Latency class reports each transport value divided by this, 2^24, so that it is a
/// normalized value, as a host passes on only those (inside 0..1 up to 16,777,216: six minutes
/// of samples at 44.1 kHz); a division by a power of two is exact, so the readout times 2^24 is
/// the transport value to the bit.
pub const READOUT_DIVISOR: f64 = 16_777_216.0;
/// The Latency class's readouts: the transport (Tempo, Beats, Bar, Playing, Samples, Clock),
/// then Wild.
const READOUTS: [ParamID; 7] = [10, 11, 12, 13, 14, 15, 16];

const GAIN_PARAM: ParamID = 0;
/// The sine's: the bend and the wheel its MIDI mapping names, and what it heard of the wheel.
const BEND_PARAM: ParamID = 1;
const WHEEL_PARAM: ParamID = 2;
const WHEEL_HEARD_PARAM: ParamID = 3;
/// The gain's stepped parameter and what it heard of it.
const STEPS_PARAM: ParamID = 1;
const STEPS_HEARD_PARAM: ParamID = 2;
const TICKS_PARAM: ParamID = 8;
const LINKED_PARAM: ParamID = 9;
/// The parameters a controller keeps as the host sets them (ids 1 to 3).
const KEPT: usize = 3;
/// The most parameter queues a block's points are read from.
const QUEUES_READ: usize = 8;
const COMPONENT_MAGIC: &[u8; 4] = b"OTP1";
const COMPONENT_STATE_LEN: usize = 12;
const CONTROLLER_STATE: [u8; 8] = *b"OTC1\0\0\0\0";

// The enum constants of the interfaces are unsigned on Linux and signed on Windows, as the SDK's
// enums are; compared through these, one spelling serves both (so a cast that changes nothing on
// one platform is the one the other needs).
#[allow(clippy::unnecessary_cast)]
mod constants {
    use super::*;

    pub const AUDIO: MediaType = MediaTypes_::kAudio as MediaType;
    pub const EVENT: MediaType = MediaTypes_::kEvent as MediaType;
    pub const INPUT: BusDirection = BusDirections_::kInput as BusDirection;
    pub const OUTPUT: BusDirection = BusDirections_::kOutput as BusDirection;
    pub const NOTE_ON: u16 = Event_::EventTypes_::kNoteOnEvent as u16;
    pub const NOTE_OFF: u16 = Event_::EventTypes_::kNoteOffEvent as u16;
    pub const SAMPLE_32: i32 = SymbolicSampleSizes_::kSample32 as i32;
    pub const MAIN_BUS: i32 = BusTypes_::kMain as i32;
    pub const DEFAULT_ACTIVE: u32 = BusInfo_::BusFlags_::kDefaultActive as u32;
    pub const UNICODE: i32 = PFactoryInfo_::FactoryFlags_::kUnicode as i32;
    pub const MANY_INSTANCES: i32 = PClassInfo_::ClassCardinality_::kManyInstances as i32;
    pub const READ_ONLY: i32 = ParameterInfo_::ParameterFlags_::kIsReadOnly as i32;
    pub const CAN_AUTOMATE: i32 = ParameterInfo_::ParameterFlags_::kCanAutomate as i32;
    pub const PLAYING: u32 = ProcessContext_::StatesAndFlags_::kPlaying as u32;
    pub const PITCH_BEND: int16 = vst3::Steinberg::Vst::ControllerNumbers_::kPitchBend as int16;
    pub const MOD_WHEEL: int16 = vst3::Steinberg::Vst::ControllerNumbers_::kCtrlModWheel as int16;
    pub const TITLES_CHANGED: int32 = RestartFlags_::kParamTitlesChanged as int32;
    pub const MAPPING_CHANGED: int32 = RestartFlags_::kMidiCCAssignmentChanged as int32;
    pub const LATENCY_CHANGED: int32 = RestartFlags_::kLatencyChanged as int32;
}

use constants::*;

static LIVE: AtomicIsize = AtomicIsize::new(0);
static ENTERED: AtomicI32 = AtomicI32::new(0);
/// The View class's log: each call its editor gets, and its parts' terminate, in turn.
static VIEW_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn log(line: impl Into<String>) {
    if let Ok(mut log) = VIEW_LOG.lock() {
        log.push(line.into());
    }
}

/// `text` into `buffer` of `room` bytes, as much as fits; `text`'s whole length.
fn copy_out(text: &str, buffer: *mut u8, room: usize) -> usize {
    let bytes = text.as_bytes();
    if !buffer.is_null() {
        // SAFETY: the caller's buffer of `room` bytes; at most that many are written.
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, bytes.len().min(room)) };
    }
    bytes.len()
}

/// The View's log, its lines joined by newlines, into `buffer` of `room` bytes; its whole
/// length (call again with room for it when that is more than `room`).
#[no_mangle]
pub extern "C" fn dawsome_test_vst3_view_log(buffer: *mut u8, room: usize) -> usize {
    let text = VIEW_LOG
        .lock()
        .map(|log| log.join("\n"))
        .unwrap_or_default();
    copy_out(&text, buffer, room)
}

/// Empties the View's log and what its editor heard.
#[no_mangle]
pub extern "C" fn dawsome_test_vst3_view_log_clear() {
    if let Ok(mut log) = VIEW_LOG.lock() {
        log.clear();
    }
    #[cfg(target_os = "linux")]
    view::clear_heard();
}

/// How many of this module's COM objects are alive (factories, components, controllers,
/// timer handlers).
#[no_mangle]
pub extern "C" fn dawsome_test_vst3_live_objects() -> isize {
    LIVE.load(Ordering::Acquire)
}

/// `ModuleEntry` calls less `ModuleExit` calls: 1 while a host holds the module entered.
#[no_mangle]
pub extern "C" fn dawsome_test_vst3_module_entered() -> i32 {
    ENTERED.load(Ordering::Acquire)
}

/// One live COM object, counted from its making to its drop.
struct Live;

impl Live {
    fn new() -> Self {
        LIVE.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Which class an object plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Sine,
    Gain,
    Fail,
    Tick,
    Latency,
    View,
}

impl Kind {
    fn is_effect(self) -> bool {
        matches!(self, Kind::Gain | Kind::Latency)
    }

    /// The sine, or the View, which is the sine with an editor: their sound, parameters, link,
    /// mapping and state are one.
    fn is_sine(self) -> bool {
        matches!(self, Kind::Sine | Kind::View)
    }

    /// The class of its controller, when that is an object of its own.
    fn controller_cid(self) -> Option<TUID> {
        match self {
            Kind::Sine => Some(CONTROLLER_CID),
            Kind::View => Some(VIEW_CONTROLLER_CID),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Sine => "Dawsome Test Sine",
            Kind::Gain => "Dawsome Test Gain",
            Kind::Fail => "Dawsome Test Fail",
            Kind::Tick => "Dawsome Test Tick",
            Kind::Latency => "Dawsome Test Latency",
            Kind::View => "Dawsome Test View",
        }
    }

    fn cid(self) -> TUID {
        match self {
            Kind::Sine => SINE_CID,
            Kind::Gain => GAIN_CID,
            Kind::Fail => FAIL_CID,
            Kind::Tick => TICK_CID,
            Kind::Latency => LATENCY_CID,
            Kind::View => VIEW_CID,
        }
    }

    fn sub_categories(self) -> &'static str {
        match self {
            Kind::Gain => "Fx",
            Kind::Latency => "Fx|Delay",
            Kind::Sine | Kind::Fail | Kind::Tick | Kind::View => "Instrument|Synth",
        }
    }

    /// Its parameters, in the order its controller lists them.
    fn params(self) -> &'static [Param] {
        match self {
            Kind::Sine | Kind::View => &SINE_PARAMS,
            Kind::Gain => &GAIN_PARAMS,
            Kind::Tick => &TICK_PARAMS,
            Kind::Latency => &LATENCY_PARAMS,
            Kind::Fail => &[GAIN],
        }
    }
}

const GAIN: Param = Param::automated(GAIN_PARAM, "Gain", 0, 1.0);
const SINE_PARAMS: [Param; 5] = [
    GAIN,
    Param::plain(BEND_PARAM, "Bend", 8192.0 / 16383.0),
    Param::plain(WHEEL_PARAM, "Wheel", 0.0),
    Param::read_only(WHEEL_HEARD_PARAM, "Wheel heard"),
    Param::read_only(LINKED_PARAM, "Linked"),
];
const GAIN_PARAMS: [Param; 3] = [
    GAIN,
    Param::automated(STEPS_PARAM, "Steps", STEPS, 1.0),
    Param::read_only(STEPS_HEARD_PARAM, "Steps heard"),
];
const TICK_PARAMS: [Param; 2] = [GAIN, Param::read_only(TICKS_PARAM, "Ticks")];
const LATENCY_PARAMS: [Param; 8] = [
    GAIN,
    Param::read_only(10, "Tempo"),
    Param::read_only(11, "Beats"),
    Param::read_only(12, "Bar"),
    Param::read_only(13, "Playing"),
    Param::read_only(14, "Samples"),
    Param::read_only(15, "Clock"),
    Param::read_only(16, "Wild"),
];

/// One parameter as a controller describes it.
#[derive(Debug, Clone, Copy)]
struct Param {
    id: ParamID,
    title: &'static str,
    flags: i32,
    steps: i32,
    default: f64,
}

impl Param {
    /// One the host may automate.
    const fn automated(id: ParamID, title: &'static str, steps: i32, default: f64) -> Self {
        Self {
            id,
            title,
            flags: CAN_AUTOMATE,
            steps,
            default,
        }
    }

    /// One of no flags, as JUCE gives the parameters its MIDI mapping names.
    const fn plain(id: ParamID, title: &'static str, default: f64) -> Self {
        Self {
            id,
            title,
            flags: 0,
            steps: 0,
            default,
        }
    }

    /// One the plugin alone sets, 0 to start.
    const fn read_only(id: ParamID, title: &'static str) -> Self {
        Self {
            id,
            title,
            flags: READ_ONLY,
            steps: 0,
            default: 0.0,
        }
    }
}

/// A gain value as the parameter takes it: inside 0..1, and the default for NaN.
fn clamp_gain(value: f64) -> f64 {
    if value.is_nan() {
        1.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

fn load_f64(value: &AtomicU64) -> f64 {
    f64::from_bits(value.load(Ordering::Acquire))
}

fn store_f64(value: &AtomicU64, to: f64) {
    value.store(to.to_bits(), Ordering::Release);
}

fn copy_cstring(src: &str, dst: &mut [c_char]) {
    let len = src.len().min(dst.len().saturating_sub(1));
    for (to, from) in dst.iter_mut().zip(&src.as_bytes()[..len]) {
        *to = *from as c_char;
    }
    if let Some(end) = dst.get_mut(len) {
        *end = 0;
    }
}

fn copy_wstring(src: &str, dst: &mut [TChar]) {
    let room = dst.len().saturating_sub(1);
    let mut len = 0;
    for (to, from) in dst.iter_mut().take(room).zip(src.encode_utf16()) {
        *to = from;
        len += 1;
    }
    if let Some(end) = dst.get_mut(len) {
        *end = 0;
    }
}

/// Everything in `stream`, at most `most` bytes; `None` past that or on a failed read.
///
/// # Safety
/// `stream` is null or a live `IBStream`, as a host hands one to `setState` for the call.
unsafe fn read_all(stream: *mut IBStream, most: usize) -> Option<Vec<u8>> {
    // SAFETY: the caller's: null or live for the call.
    let stream = unsafe { ComRef::from_raw(stream) }?;
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0u8; 64];
        let mut read = 0;
        // SAFETY: the live stream; `chunk` has room for the 64 bytes asked for, `read` is ours.
        let result =
            unsafe { stream.read(chunk.as_mut_ptr().cast(), chunk.len() as i32, &mut read) };
        if result != kResultOk || read <= 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read as usize]);
        if bytes.len() > most {
            return None;
        }
    }
    Some(bytes)
}

/// Writes `bytes` whole to `stream`.
///
/// # Safety
/// As [`read_all`], for `getState`.
unsafe fn write_all(stream: *mut IBStream, bytes: &[u8]) -> bool {
    // SAFETY: the caller's: null or live for the call.
    let Some(stream) = (unsafe { ComRef::from_raw(stream) }) else {
        return false;
    };
    let mut written = 0;
    // SAFETY: the live stream; it reads the `bytes.len()` bytes of `bytes` and never writes
    // them; `written` is ours.
    let result = unsafe {
        stream.write(
            bytes.as_ptr().cast_mut().cast(),
            bytes.len() as i32,
            &mut written,
        )
    };
    result == kResultOk && written == bytes.len() as i32
}

/// The gain a component state holds.
fn parse_component_state(bytes: &[u8]) -> Option<f64> {
    if bytes.len() != COMPONENT_STATE_LEN || !bytes.starts_with(COMPONENT_MAGIC) {
        return None;
    }
    let mut gain = [0; 8];
    gain.copy_from_slice(&bytes[4..]);
    Some(f64::from_le_bytes(gain))
}

/// Sends `peer` a message `id` with the integer attribute "n" = `n`, made by `host`.
///
/// # Safety
/// `host` and `peer` are the host's objects (held, so live), called on the main thread.
unsafe fn send(
    host: &ComPtr<IHostApplication>,
    peer: &ComPtr<IConnectionPoint>,
    id: &CStr,
    n: i64,
) {
    let mut cid = IMessage_iid;
    let mut iid = IMessage_iid;
    let mut made = ptr::null_mut();
    // SAFETY: the caller's live host; the ids and `made` are ours.
    if unsafe { host.createInstance(&mut cid, &mut iid, &mut made) } != kResultOk {
        return;
    }
    // SAFETY: `createInstance` gave an `IMessage` with one reference for us, or null.
    let Some(message) = (unsafe { ComPtr::<IMessage>::from_raw(made.cast()) }) else {
        return;
    };
    // SAFETY: the live message; `id` is NUL-terminated (a `CStr`).
    unsafe { message.setMessageID(id.as_ptr()) };
    // SAFETY: the live message.
    let attributes = unsafe { message.getAttributes() };
    // SAFETY: the message keeps its list while `message` lives.
    if let Some(attributes) = unsafe { ComRef::from_raw(attributes) } {
        // SAFETY: the live list; "n" is a NUL-terminated constant.
        unsafe { attributes.setInt(c"n".as_ptr(), n) };
    }
    // SAFETY: the caller's live peer; the message lives through the call.
    unsafe { peer.notify(message.as_ptr()) };
}

/// A message's id and its "n" attribute.
///
/// # Safety
/// `message` is null or a live `IMessage`, as `IConnectionPoint::notify` hands one for the call.
unsafe fn read_message(message: *mut IMessage) -> Option<(Vec<u8>, i64)> {
    // SAFETY: the caller's: null or live for the call.
    let message = unsafe { ComRef::from_raw(message) }?;
    // SAFETY: the live message.
    let id = unsafe { message.getMessageID() };
    if id.is_null() {
        return None;
    }
    // SAFETY: a non-null message id is a NUL-terminated string the message keeps (`FIDString`).
    let id = unsafe { CStr::from_ptr(id) }.to_bytes().to_vec();
    // SAFETY: the live message.
    let attributes = unsafe { message.getAttributes() };
    // SAFETY: the message keeps its list while it lives.
    let attributes = unsafe { ComRef::from_raw(attributes) }?;
    let mut n = 0;
    // SAFETY: the live list; "n" is a NUL-terminated constant, `n` is ours.
    (unsafe { attributes.getInt(c"n".as_ptr(), &mut n) } == kResultOk).then_some((id, n))
}

/// The sounding note: its key, id, velocity, frequency, frames played since its note-on, and
/// where a pitch bend moved its phase (`None` while none did: the unbent formula, to its last
/// bit), as the CLAP test sine keeps them.
#[derive(Clone, Copy)]
struct Note {
    key: i16,
    id: i32,
    velocity: f64,
    frequency: f64,
    elapsed: u64,
    bent: Option<Bent>,
}

/// From frame `from` of its note the sine runs at `frequency·ratio` from the phase sum `base`.
#[derive(Clone, Copy)]
struct Bent {
    ratio: f64,
    base: f64,
    from: u64,
}

impl Note {
    /// The phase sum (cycles times the rate) at frame `n` of the note.
    fn sum(&self, n: u64) -> f64 {
        match self.bent {
            None => self.frequency * n as f64,
            Some(bent) => bent.base + self.frequency * bent.ratio * (n - bent.from) as f64,
        }
    }
}

/// What a processor keeps between its calls on the audio thread.
struct Engine {
    sample_rate: f64,
    /// The gain in force: parameter points change it at their frame, state loads at the start
    /// of the next call.
    value: f64,
    loads_seen: u64,
    note: Option<Note>,
    /// The pitch bend's ratio, 1 at rest: what the next note starts at.
    bend: f64,
    /// Audio calls so far, which the Fail and Latency classes count.
    calls: u64,
    delay: [[f32; LATENCY_AFTER_RESTART]; 2],
    at: usize,
}

impl Engine {
    /// The next sine frame, advancing the note.
    fn next_value(&mut self) -> f32 {
        let Some(note) = &mut self.note else {
            return 0.0;
        };
        let phase = match note.bent {
            None => TAU * note.frequency * note.elapsed as f64 / self.sample_rate,
            Some(_) => TAU * note.sum(note.elapsed) / self.sample_rate,
        };
        note.elapsed += 1;
        (self.value * note.velocity * phase.cos()) as f32
    }

    /// A point `v` of the sine's "Bend": `b = round(v·16383) − 8192`, the CLAP test sine's
    /// ratio 2^(b/49152), the sounding note reading on from where its phase is.
    fn bend_to(&mut self, v: f64) {
        let b = (v * 16383.0).round() as i32 - 8192;
        let ratio = 2f64.powf(f64::from(b) / 49152.0);
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

    fn apply(&mut self, event: &Event) {
        match event.r#type {
            NOTE_ON => {
                // SAFETY: the event says it is a note-on, so the union holds a `NoteOnEvent`
                // (plain numbers, for which any bytes are a value).
                let on = unsafe { event.__field0.noteOn };
                // Left to right in f64, as the formula is written.
                let frequency = 440.0 * 2f64.powf(f64::from(on.pitch - 69) / 12.0);
                let bent = (self.bend != 1.0).then_some(Bent {
                    ratio: self.bend,
                    base: 0.0,
                    from: 0,
                });
                self.note = Some(Note {
                    key: on.pitch,
                    id: on.noteId,
                    velocity: f64::from(on.velocity),
                    frequency,
                    elapsed: 0,
                    bent,
                });
            }
            NOTE_OFF => {
                // SAFETY: as for the note-on: a `NoteOffEvent`, plain numbers.
                let off = unsafe { event.__field0.noteOff };
                let ends = self.note.is_some_and(|note| {
                    if off.noteId == -1 {
                        note.key == off.pitch
                    } else {
                        note.id == off.noteId
                    }
                });
                if ends {
                    self.note = None;
                }
            }
            _ => {}
        }
    }
}

/// A block's parameter points, every parameter's (the first [`QUEUES_READ`]), read in frame
/// order.
struct Points<'a> {
    queues: [Option<(ParamID, ComRef<'a, IParamValueQueue>, i32)>; QUEUES_READ],
    next: [i32; QUEUES_READ],
}

impl Points<'_> {
    /// # Safety
    /// `changes` is null or the host's live input changes of a `process` call, and the
    /// `Points` made from them is dropped before that call returns.
    unsafe fn of(changes: *mut IParameterChanges) -> Self {
        let mut points = Self {
            queues: [None; QUEUES_READ],
            next: [0; QUEUES_READ],
        };
        // SAFETY: the caller's: null or live for the call.
        if let Some(changes) = unsafe { ComRef::from_raw(changes) } {
            // SAFETY: the live changes.
            let count = unsafe { changes.getParameterCount() }.clamp(0, QUEUES_READ as i32);
            for index in 0..count {
                // SAFETY: the live changes; `index` is below their count.
                let queue = unsafe { changes.getParameterData(index) };
                // SAFETY: the changes keep their queues for the call, as long as `points` lives.
                if let Some(queue) = unsafe { ComRef::from_raw(queue) } {
                    // SAFETY: the live queue.
                    let id = unsafe { queue.getParameterId() };
                    // SAFETY: the live queue.
                    let points_in = unsafe { queue.getPointCount() };
                    points.queues[index as usize] = Some((id, queue, points_in));
                }
            }
        }
        points
    }

    /// The next point not taken yet, of any parameter, on frame `frame` or before (`None`: any
    /// frame, for the points stamped past the block): its parameter and value.
    ///
    /// # Safety
    /// Called inside the `process` call whose changes made `self` ([`Points::of`]).
    unsafe fn next_until(&mut self, frame: Option<usize>) -> Option<(ParamID, f64)> {
        for (queue, next) in self.queues.iter().zip(self.next.iter_mut()) {
            let Some((id, queue, count)) = *queue else {
                continue;
            };
            while *next < count {
                let (mut offset, mut value) = (0, 0.0);
                // SAFETY: the caller's: the queue is live for the call; out parameters ours.
                if unsafe { queue.getPoint(*next, &mut offset, &mut value) } != kResultOk {
                    *next += 1;
                    continue;
                }
                if frame.is_some_and(|frame| offset as usize > frame) {
                    break;
                }
                *next += 1;
                return Some((id, value));
            }
        }
        None
    }
}

/// A point of parameter `id` taken by a `kind` processor: the gain changes, the sine's bend
/// moves its pitch, and what the sine's wheel and the gain's steps were given goes back as an
/// output change of their "heard" parameters, at offset 0.
///
/// # Safety
/// `out` is the host's output changes of the `process` call this is made inside.
unsafe fn take_point(
    kind: Kind,
    engine: &mut Engine,
    id: ParamID,
    value: f64,
    out: Option<ComRef<'_, IParameterChanges>>,
) {
    let heard = match (kind, id) {
        (_, GAIN_PARAM) => {
            engine.value = clamp_gain(value);
            None
        }
        (kind, BEND_PARAM) if kind.is_sine() => {
            engine.bend_to(value);
            None
        }
        (kind, WHEEL_PARAM) if kind.is_sine() => Some(WHEEL_HEARD_PARAM),
        (Kind::Gain, STEPS_PARAM) => Some(STEPS_HEARD_PARAM),
        _ => None,
    };
    if let (Some(heard), Some(out)) = (heard, out) {
        let mut index = 0;
        // SAFETY: the caller's: `out` is live for the call; `heard` and `index` are ours.
        let queue = unsafe { out.addParameterData(&heard, &mut index) };
        // SAFETY: the changes keep their queue for the call.
        if let Some(queue) = unsafe { ComRef::from_raw(queue) } {
            // SAFETY: the live queue; `index` is ours.
            unsafe { queue.addPoint(0, value, &mut index) };
        }
    }
}

/// The audio processor half every class has: buses, state and `process`.
struct Processor {
    kind: Kind,
    /// The gain the state holds (the processor's, as `process` left it or a state load set it).
    gain: AtomicU64,
    /// State loads so far, so `process` takes a load at the start of its next call.
    loads: AtomicU64,
    /// How late the Latency class plays and says it plays, in frames: shared with its
    /// controller, which a restart with `kLatencyChanged` makes set it.
    delay: Arc<AtomicUsize>,
    engine: RefCell<Engine>,
}

/// Each `unsafe fn` here is the body of the interface method of the same name (the
/// `processor_methods!` macro), and its safety contract is that method's: the pointers are the
/// host's, null or valid for the call, of the sizes the interface states.
impl Processor {
    fn new(kind: Kind, delay: Arc<AtomicUsize>) -> Self {
        Self {
            kind,
            gain: AtomicU64::new(1.0f64.to_bits()),
            loads: AtomicU64::new(0),
            delay,
            engine: RefCell::new(Engine {
                sample_rate: 48_000.0,
                value: 1.0,
                loads_seen: 0,
                note: None,
                bend: 1.0,
                calls: 0,
                delay: [[0.0; LATENCY_AFTER_RESTART]; 2],
                at: 0,
            }),
        }
    }

    fn bus_count(&self, media: MediaType, dir: BusDirection) -> i32 {
        let effect = self.kind.is_effect();
        match (media, dir) {
            (AUDIO, OUTPUT) => 1,
            (AUDIO, INPUT) => i32::from(effect),
            (EVENT, INPUT) => i32::from(!effect),
            _ => 0,
        }
    }

    unsafe fn bus_info(
        &self,
        media: MediaType,
        dir: BusDirection,
        index: i32,
        bus: *mut BusInfo,
    ) -> i32 {
        if index != 0 || index >= self.bus_count(media, dir) || bus.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the host's out parameter (`getBusInfo`), non-null (checked).
        let bus = unsafe { &mut *bus };
        bus.mediaType = media;
        bus.direction = dir;
        let (channels, name) = match (media, dir) {
            (EVENT, _) => (16, "Notes"),
            (_, INPUT) => (2, "Input"),
            _ => (2, "Output"),
        };
        bus.channelCount = channels;
        copy_wstring(name, &mut bus.name);
        bus.busType = MAIN_BUS;
        bus.flags = DEFAULT_ACTIVE;
        kResultOk
    }

    unsafe fn set_arrangements(
        &self,
        inputs: *mut SpeakerArrangement,
        ins: i32,
        outputs: *mut SpeakerArrangement,
        outs: i32,
    ) -> i32 {
        let want_ins = i32::from(self.kind.is_effect());
        if ins != want_ins || outs != 1 {
            return kResultFalse;
        }
        let stereo = |list: *mut SpeakerArrangement, count: i32| {
            // SAFETY: the host's array of `count` arrangements (`setBusArrangements`): when
            // `count` is not 0 its first entry is read, behind the null check.
            count == 0 || (!list.is_null() && unsafe { *list } == SpeakerArr::kStereo)
        };
        if stereo(inputs, ins) && stereo(outputs, outs) {
            kResultTrue
        } else {
            kResultFalse
        }
    }

    unsafe fn arrangement(
        &self,
        dir: BusDirection,
        index: i32,
        arrangement: *mut SpeakerArrangement,
    ) -> i32 {
        if index != 0 || arrangement.is_null() || self.bus_count(AUDIO, dir) == 0 {
            return kInvalidArgument;
        }
        // SAFETY: the host's out parameter (`getBusArrangement`), non-null (checked).
        unsafe { *arrangement = SpeakerArr::kStereo };
        kResultOk
    }

    /// Its delay, held to the delay line's length.
    fn delay(&self) -> usize {
        self.delay
            .load(Ordering::Acquire)
            .clamp(1, LATENCY_AFTER_RESTART)
    }

    fn latency(&self) -> u32 {
        if self.kind == Kind::Latency {
            self.delay() as u32
        } else {
            0
        }
    }

    unsafe fn setup(&self, setup: *mut ProcessSetup) -> i32 {
        // SAFETY: the host's setup (`setupProcessing`), null or live for the call.
        let Some(setup) = (unsafe { setup.as_ref() }) else {
            return kInvalidArgument;
        };
        self.engine.borrow_mut().sample_rate = setup.sampleRate;
        kResultOk
    }

    /// Off (processing stopped, or the processor deactivated): the note stops, the delay line
    /// empties.
    fn stop(&self) {
        let mut engine = self.engine.borrow_mut();
        engine.note = None;
        engine.delay = [[0.0; LATENCY_AFTER_RESTART]; 2];
        engine.at = 0;
    }

    unsafe fn process(&self, data: *mut ProcessData) -> i32 {
        // SAFETY: the host's process data (`process`), null or live for the call; every
        // pointer in it below is the host's for this call alone, sized as it says.
        let Some(data) = (unsafe { data.as_ref() }) else {
            return kInvalidArgument;
        };
        let mut engine = self.engine.borrow_mut();
        let loads = self.loads.load(Ordering::Acquire);
        if loads != engine.loads_seen {
            engine.loads_seen = loads;
            engine.value = load_f64(&self.gain);
        }
        // SAFETY: the data's input changes (null or live), and `points` is dropped as this
        // call returns.
        let mut points = unsafe { Points::of(data.inputParameterChanges) };
        // SAFETY: the data's output changes, null or live for the call.
        let reported = unsafe { ComRef::from_raw(data.outputParameterChanges) };
        if data.numSamples <= 0 || data.numOutputs <= 0 || data.outputs.is_null() {
            // A flush: the values alone, no audio.
            // SAFETY: inside the call that made `points`.
            while let Some((id, value)) = unsafe { points.next_until(None) } {
                // SAFETY: `reported` is this call's output changes.
                unsafe { take_point(self.kind, &mut engine, id, value, reported) };
            }
            store_f64(&self.gain, engine.value);
            return kResultOk;
        }
        let delay = self.delay();
        if self.kind == Kind::Fail {
            if engine.calls >= FAIL_AFTER {
                return kResultFalse;
            }
            engine.calls += 1;
        }
        let frames = data.numSamples as usize;
        // SAFETY: `outputs` is non-null (checked) and holds `numOutputs` (> 0) buses.
        let out = unsafe { &*data.outputs };
        // SAFETY: the union's 32-bit member, the sample size set up (`SAMPLE_32`); a pointer.
        let buffers = unsafe { out.__field0.channelBuffers32 };
        if out.numChannels < 2 || buffers.is_null() {
            return kResultFalse;
        }
        // SAFETY: non-null (checked), `numChannels` (at least 2) channel pointers.
        let out = unsafe { slice::from_raw_parts(buffers, 2) };
        // SAFETY: each output channel holds `numSamples` frames, a buffer of its own that this
        // call alone writes (our host gives inputs and outputs apart, never in place).
        let left = unsafe { slice::from_raw_parts_mut(out[0], frames) };
        // SAFETY: as for `left`.
        let right = unsafe { slice::from_raw_parts_mut(out[1], frames) };
        let input = if self.kind.is_effect() && data.numInputs > 0 && !data.inputs.is_null() {
            // SAFETY: `inputs` is non-null (checked) and holds `numInputs` (> 0) buses.
            let bus = unsafe { &*data.inputs };
            // SAFETY: as for the output's buffers.
            let buffers = unsafe { bus.__field0.channelBuffers32 };
            (bus.numChannels >= 2 && !buffers.is_null()).then(|| {
                // SAFETY: as for the output's channel pointers.
                let channels = unsafe { slice::from_raw_parts(buffers, 2) };
                [
                    // SAFETY: each input channel holds `numSamples` frames, only read.
                    unsafe { slice::from_raw_parts(channels[0], frames) },
                    // SAFETY: as for the first.
                    unsafe { slice::from_raw_parts(channels[1], frames) },
                ]
            })
        } else {
            None
        };
        // SAFETY: the data's input events, null or live for the call.
        let events = unsafe { ComRef::from_raw(data.inputEvents) };
        // SAFETY: the live event list.
        let event_count = events.map_or(0, |events| unsafe { events.getEventCount() });
        let mut next_event = 0;
        for frame in 0..frames {
            // SAFETY: inside the call that made `points`.
            while let Some((id, value)) = unsafe { points.next_until(Some(frame)) } {
                // SAFETY: `reported` is this call's output changes.
                unsafe { take_point(self.kind, &mut engine, id, value, reported) };
            }
            if let Some(events) = events {
                while next_event < event_count {
                    // SAFETY: `Event` is plain C data (numbers and a union of plain structs),
                    // for which all zero bytes are a value.
                    let mut event: Event = unsafe { std::mem::zeroed() };
                    // SAFETY: the live event list; `event` is ours to fill.
                    if unsafe { events.getEvent(next_event, &mut event) } != kResultOk {
                        next_event += 1;
                        continue;
                    }
                    if event.sampleOffset as usize > frame {
                        break;
                    }
                    engine.apply(&event);
                    next_event += 1;
                }
            }
            let (l, r) = match (self.kind, input) {
                (Kind::Gain, Some([l, r])) => (
                    (f64::from(l[frame]) * engine.value) as f32,
                    (f64::from(r[frame]) * engine.value) as f32,
                ),
                (Kind::Latency, Some([l, r])) => {
                    // Read before the write: at a delay of the whole line the slot read is
                    // the one about to be written.
                    let at = engine.at;
                    let read = (at + LATENCY_AFTER_RESTART - delay) % LATENCY_AFTER_RESTART;
                    let late = [engine.delay[0][read], engine.delay[1][read]];
                    engine.delay[0][at] = l[frame];
                    engine.delay[1][at] = r[frame];
                    engine.at = (at + 1) % LATENCY_AFTER_RESTART;
                    (
                        (f64::from(late[0]) * engine.value) as f32,
                        (f64::from(late[1]) * engine.value) as f32,
                    )
                }
                (Kind::Gain | Kind::Latency, None) => (0.0, 0.0),
                (Kind::Sine | Kind::Fail | Kind::Tick | Kind::View, _) => {
                    let value = engine.next_value();
                    (value, value)
                }
            };
            left[frame] = l;
            right[frame] = r;
        }
        // Points and events stamped past the block (a host error) still take effect.
        // SAFETY: inside the call that made `points`.
        while let Some((id, value)) = unsafe { points.next_until(None) } {
            // SAFETY: `reported` is this call's output changes.
            unsafe { take_point(self.kind, &mut engine, id, value, reported) };
        }
        if let Some(events) = events {
            while next_event < event_count {
                // SAFETY: as above: plain C data.
                let mut event: Event = unsafe { std::mem::zeroed() };
                // SAFETY: as above: the live event list.
                if unsafe { events.getEvent(next_event, &mut event) } == kResultOk {
                    engine.apply(&event);
                }
                next_event += 1;
            }
        }
        store_f64(&self.gain, engine.value);
        if self.kind == Kind::Latency {
            // SAFETY: `data` is this call's process data.
            unsafe { report_readouts(data, engine.calls) };
            engine.calls += 1;
        }
        kResultOk
    }

    unsafe fn get_state(&self, stream: *mut IBStream) -> i32 {
        let mut bytes = COMPONENT_MAGIC.to_vec();
        bytes.extend_from_slice(&load_f64(&self.gain).to_le_bytes());
        // SAFETY: the host's stream (`getState`), null or live for the call.
        if unsafe { write_all(stream, &bytes) } {
            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn set_state(&self, stream: *mut IBStream) -> i32 {
        // SAFETY: the host's stream (`setState`), null or live for the call.
        match unsafe { read_all(stream, 64) }
            .as_deref()
            .and_then(parse_component_state)
        {
            Some(gain) => {
                store_f64(&self.gain, clamp_gain(gain));
                self.loads.fetch_add(1, Ordering::AcqRel);
                kResultOk
            }
            None => kResultFalse,
        }
    }
}

/// The Latency class's report of the transport `data` carries, and of its Wild value on audio
/// call `call`, as output changes of its readouts at offset 0.
///
/// # Safety
/// `data` is the host's process data of the `process` call this is made inside.
unsafe fn report_readouts(data: &ProcessData, call: u64) {
    // SAFETY: the caller's: the context is null or live for the call.
    let context = unsafe { data.processContext.as_ref() };
    // SAFETY: the caller's: the output changes are null or live for the call.
    let changes = unsafe { ComRef::from_raw(data.outputParameterChanges) };
    let (Some(context), Some(changes)) = (context, changes) else {
        return;
    };
    let playing = context.state & PLAYING != 0;
    let transport = [
        context.tempo,
        context.projectTimeMusic,
        context.barPositionMusic,
        if playing { 1.0 } else { 0.0 },
        context.projectTimeSamples as f64,
        context.continousTimeSamples as f64,
    ]
    .map(|value| value / READOUT_DIVISOR);
    let wild = WILD[(call % WILD.len() as u64) as usize];
    let values = transport.into_iter().chain([wild]);
    for (id, value) in READOUTS.iter().zip(values) {
        let mut index = 0;
        // SAFETY: the live output changes; `id` and `index` are ours.
        let queue = unsafe { changes.addParameterData(id, &mut index) };
        // SAFETY: the changes keep their queue for the call.
        if let Some(queue) = unsafe { ComRef::from_raw(queue) } {
            // SAFETY: the live queue; `index` is ours.
            unsafe { queue.addPoint(0, value, &mut index) };
        }
    }
}

/// The Tick class's timer: counts its calls, and on the first edits the gain.
#[cfg(target_os = "linux")]
struct TickTimer {
    ticks: Arc<AtomicU64>,
    handler: Arc<std::sync::Mutex<Option<ComPtr<IComponentHandler>>>>,
    _live: Live,
}

#[cfg(target_os = "linux")]
impl Class for TickTimer {
    type Interfaces = (ITimerHandler,);
}

#[cfg(target_os = "linux")]
impl ITimerHandlerTrait for TickTimer {
    unsafe fn onTimer(&self) {
        if self.ticks.fetch_add(1, Ordering::AcqRel) == 0 {
            let handler = self.handler.lock().ok().and_then(|handler| handler.clone());
            if let Some(handler) = handler {
                // SAFETY: the host's handler, held (a `ComPtr`, given back at `terminate`);
                // a timer is called on the main thread, where edits are made.
                unsafe { handler.beginEdit(GAIN_PARAM) };
                // SAFETY: as for `beginEdit`.
                unsafe { handler.performEdit(GAIN_PARAM, TICK_EDIT) };
                // SAFETY: as for `beginEdit`.
                unsafe { handler.endEdit(GAIN_PARAM) };
            }
        }
    }
}

/// What [`dawsome_test_vst3_restart`] reaches of a live controller: the thread it was made on
/// (the plugins' main thread), its handler, and what a restart changes first.
struct Restartable {
    kind: Kind,
    /// [`this_thread`] of the thread that initialized it.
    thread: std::sync::OnceLock<usize>,
    /// Shared with the controller (and the Tick class's timer).
    handler: Arc<Mutex<Option<ComPtr<IComponentHandler>>>>,
    /// Parameter 0 is called [`RENAMED`] (`kParamTitlesChanged`).
    renamed: AtomicBool,
    /// The sine's MIDI mapping maps nothing (`kMidiCCAssignmentChanged`).
    unmapped: AtomicBool,
    /// The Latency class's delay (`kLatencyChanged`), shared with its processor.
    delay: Arc<AtomicUsize>,
}

/// Every live controller, made by any host in this process.
static CONTROLLERS: Mutex<Vec<Weak<Restartable>>> = Mutex::new(Vec::new());

thread_local! {
    /// A byte per thread, whose address tells the threads apart. Why not
    /// `std::thread::current()`: that registers a thread-local destructor in this module, which
    /// runs when the thread ends, after the host has unloaded the module (measured: the
    /// reference test's thread crashed in unmapped code); a byte has no destructor.
    static THREAD_MARK: u8 = const { 0 };
}

/// What tells this thread apart from the others alive now.
fn this_thread() -> usize {
    THREAD_MARK.with(|mark| std::ptr::from_ref(mark) as usize)
}

/// Every controller made on the caller's thread asks its host for `restartComponent(flags)`,
/// now, on this thread, having first made the change the flags announce: with
/// `kParamTitlesChanged` parameter 0 is called [`RENAMED`], with `kMidiCCAssignmentChanged` the
/// sine maps nothing, with `kLatencyChanged` the Latency class is [`LATENCY_AFTER_RESTART`] frames
/// late from its next `process`. Returns how many asked. Why the caller's thread only: a plugin
/// asks on its main thread, the one that made it, and the tests that run at the same time in
/// one process each make their own on threads of their own.
#[no_mangle]
pub extern "C" fn dawsome_test_vst3_restart(flags: i32) -> i32 {
    let here = this_thread();
    let mine: Vec<Arc<Restartable>> = CONTROLLERS
        .lock()
        .map(|list| {
            list.iter()
                .filter_map(Weak::upgrade)
                .filter(|controller| controller.thread.get() == Some(&here))
                .collect()
        })
        .unwrap_or_default();
    let mut asked = 0;
    for controller in mine {
        if flags & TITLES_CHANGED != 0 {
            controller.renamed.store(true, Ordering::Release);
        }
        if flags & MAPPING_CHANGED != 0 {
            controller.unmapped.store(true, Ordering::Release);
        }
        if flags & LATENCY_CHANGED != 0 && controller.kind == Kind::Latency {
            controller
                .delay
                .store(LATENCY_AFTER_RESTART, Ordering::Release);
        }
        let handler = controller
            .handler
            .lock()
            .ok()
            .and_then(|handler| handler.clone());
        if let Some(handler) = handler {
            // SAFETY: the host's handler, live while its controller holds it (given back at
            // `terminate`); `restartComponent` is a main-thread call, made here on the thread
            // that made the controller, with no lock of ours held.
            unsafe { handler.restartComponent(flags) };
            asked += 1;
        }
    }
    asked
}

/// The edit-controller half every class has: parameters, the controller's state, the host.
struct Controller {
    kind: Kind,
    /// The gain as the controller has it, normalized.
    gain: AtomicU64,
    /// The sine's "Linked", 1.0 once the link answered.
    linked: AtomicU64,
    /// The Tick class's timer calls.
    ticks: Arc<AtomicU64>,
    /// The Latency class's readouts, as the host gave them back.
    readouts: [AtomicU64; 7],
    /// Parameters 1 to 3 as the host set them last (the sine's Bend, Wheel and Wheel heard, the
    /// gain's Steps and Steps heard), else their defaults.
    kept: [AtomicU64; KEPT],
    restartable: Arc<Restartable>,
    /// Shared with the Tick class's timer, which edits through it.
    handler: Arc<std::sync::Mutex<Option<ComPtr<IComponentHandler>>>>,
    host: RefCell<Option<ComPtr<IHostApplication>>>,
    peer: RefCell<Option<ComPtr<IConnectionPoint>>>,
    #[cfg(target_os = "linux")]
    timer: RefCell<Option<(ComPtr<IRunLoop>, ComWrapper<TickTimer>)>>,
}

/// As `Processor`'s: each `unsafe fn` is the body of the interface method of the same name
/// (`controller_methods!` and the classes' own impls), with that method's contract.
impl Controller {
    fn new(kind: Kind, delay: Arc<AtomicUsize>) -> Self {
        let handler: Arc<Mutex<Option<ComPtr<IComponentHandler>>>> = Arc::default();
        let default = |id: ParamID| {
            let param = kind.params().iter().find(|param| param.id == id);
            AtomicU64::new(param.map_or(0.0, |param| param.default).to_bits())
        };
        Self {
            kind,
            gain: AtomicU64::new(1.0f64.to_bits()),
            linked: AtomicU64::new(0.0f64.to_bits()),
            ticks: Arc::default(),
            readouts: Default::default(),
            kept: [default(1), default(2), default(3)],
            restartable: Arc::new(Restartable {
                kind,
                thread: std::sync::OnceLock::new(),
                handler: Arc::clone(&handler),
                renamed: AtomicBool::new(false),
                unmapped: AtomicBool::new(false),
                delay,
            }),
            handler,
            host: RefCell::new(None),
            peer: RefCell::new(None),
            #[cfg(target_os = "linux")]
            timer: RefCell::new(None),
        }
    }

    unsafe fn initialize(&self, context: *mut FUnknown) -> i32 {
        // SAFETY: the host's context (`initialize`), null or live for the call; `cast` keeps
        // references of our own.
        let Some(context) = (unsafe { ComRef::from_raw(context) }) else {
            return kInvalidArgument;
        };
        *self.host.borrow_mut() = context.cast::<IHostApplication>();
        let _ = self.restartable.thread.set(this_thread());
        if let Ok(mut list) = CONTROLLERS.lock() {
            list.retain(|controller| controller.strong_count() > 0);
            list.push(Arc::downgrade(&self.restartable));
        }
        #[cfg(target_os = "linux")]
        if self.kind == Kind::Tick {
            if let Some(run_loop) = context.cast::<IRunLoop>() {
                let timer = ComWrapper::new(TickTimer {
                    ticks: Arc::clone(&self.ticks),
                    handler: Arc::clone(&self.handler),
                    _live: Live::new(),
                });
                if let Some(handler) = timer.as_com_ref::<ITimerHandler>() {
                    // SAFETY: the host's run loop, held; the handler is our object, which the
                    // run loop keeps until `terminate` unregisters it.
                    let registered = unsafe { run_loop.registerTimer(handler.as_ptr(), TIMER_MS) };
                    if registered == kResultOk {
                        *self.timer.borrow_mut() = Some((run_loop, timer));
                    }
                }
            }
        }
        kResultOk
    }

    unsafe fn terminate(&self) -> i32 {
        if self.kind == Kind::View {
            log("controller terminate");
        }
        if let Ok(mut list) = CONTROLLERS.lock() {
            let mine = Arc::as_ptr(&self.restartable);
            list.retain(|controller| controller.strong_count() > 0 && controller.as_ptr() != mine);
        }
        #[cfg(target_os = "linux")]
        if let Some((run_loop, timer)) = self.timer.borrow_mut().take() {
            if let Some(handler) = timer.as_com_ref::<ITimerHandler>() {
                // SAFETY: the run loop is held; the handler is the one registered.
                unsafe { run_loop.unregisterTimer(handler.as_ptr()) };
            }
        }
        let handler = self
            .handler
            .lock()
            .ok()
            .and_then(|mut handler| handler.take());
        drop(handler);
        self.host.borrow_mut().take();
        self.peer.borrow_mut().take();
        kResultOk
    }

    unsafe fn set_component_state(&self, stream: *mut IBStream) -> i32 {
        // SAFETY: the host's stream (`setComponentState`), null or live for the call.
        let Some(gain) = unsafe { read_all(stream, 64) }
            .as_deref()
            .and_then(parse_component_state)
        else {
            return kResultFalse;
        };
        store_f64(&self.gain, clamp_gain(gain));
        if self.kind.is_sine() {
            // Called back in the middle of the host's own call, as real plugins do.
            let handler = self.handler.lock().ok().and_then(|handler| handler.clone());
            if let Some(handler) = handler {
                // SAFETY: the host's handler, held; on the main thread, inside the host's call.
                unsafe { handler.restartComponent(RestartFlags_::kParamValuesChanged) };
            }
        }
        kResultOk
    }

    unsafe fn set_state(&self, stream: *mut IBStream) -> i32 {
        // SAFETY: the host's stream (`setState`), null or live for the call.
        let bytes = unsafe { read_all(stream, 64) };
        let want: &[u8] = if self.kind.is_sine() {
            &CONTROLLER_STATE
        } else {
            &[]
        };
        if bytes.as_deref() == Some(want) {
            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn get_state(&self, stream: *mut IBStream) -> i32 {
        // SAFETY: the host's stream (`getState`), null or live for the call.
        if !self.kind.is_sine() || unsafe { write_all(stream, &CONTROLLER_STATE) } {
            kResultOk
        } else {
            kResultFalse
        }
    }

    fn param_count(&self) -> i32 {
        self.kind.params().len() as i32
    }

    unsafe fn param_info(&self, index: i32, info: *mut ParameterInfo) -> i32 {
        // SAFETY: the host's out parameter (`getParameterInfo`), null or live for the call.
        let info = unsafe { info.as_mut() };
        let (Some(&param), Some(info)) = (
            usize::try_from(index)
                .ok()
                .and_then(|index| self.kind.params().get(index)),
            info,
        ) else {
            return kInvalidArgument;
        };
        let renamed = self.restartable.renamed.load(Ordering::Acquire);
        let title = if param.id == GAIN_PARAM && renamed {
            RENAMED
        } else {
            param.title
        };
        info.id = param.id;
        copy_wstring(title, &mut info.title);
        copy_wstring(title, &mut info.shortTitle);
        copy_wstring("", &mut info.units);
        info.stepCount = param.steps;
        info.defaultNormalizedValue = param.default;
        info.unitId = 0;
        info.flags = param.flags;
        kResultOk
    }

    fn knows(&self, id: ParamID) -> bool {
        self.kind.params().iter().any(|param| param.id == id)
    }

    /// Where parameter `id` is kept, for one of ids 1 to 3 this class has.
    fn kept(&self, id: ParamID) -> Option<&AtomicU64> {
        let at = usize::try_from(id).ok()?.checked_sub(1)?;
        self.kept.get(at).filter(|_| self.knows(id))
    }

    /// The sine's MIDI mapping: on bus 0, channel 0, pitch bend moves "Bend" and the mod wheel
    /// "Wheel"; nothing once a restart with `kMidiCCAssignmentChanged` took it.
    unsafe fn assignment(
        &self,
        bus: int32,
        channel: int16,
        number: int16,
        id: *mut ParamID,
    ) -> i32 {
        // SAFETY: the host's out parameter (`getMidiControllerAssignment`), null or live.
        let Some(id) = (unsafe { id.as_mut() }) else {
            return kInvalidArgument;
        };
        if bus != 0 || channel != 0 || self.restartable.unmapped.load(Ordering::Acquire) {
            return kResultFalse;
        }
        *id = match number {
            PITCH_BEND => BEND_PARAM,
            MOD_WHEEL => WHEEL_PARAM,
            _ => return kResultFalse,
        };
        kResultTrue
    }

    fn normalized(&self, id: ParamID) -> f64 {
        if let Some(at) = self.readout(id) {
            return load_f64(&self.readouts[at]);
        }
        match id {
            GAIN_PARAM => load_f64(&self.gain),
            LINKED_PARAM if self.kind.is_sine() => load_f64(&self.linked),
            TICKS_PARAM if self.kind == Kind::Tick => {
                self.ticks.load(Ordering::Acquire) as f64 / 1000.0
            }
            _ => self.kept(id).map_or(0.0, load_f64),
        }
    }

    /// Where readout `id` is kept, for the Latency class.
    fn readout(&self, id: ParamID) -> Option<usize> {
        (self.kind == Kind::Latency)
            .then(|| READOUTS.iter().position(|&known| known == id))
            .flatten()
    }

    fn set_normalized(&self, id: ParamID, value: f64) -> i32 {
        if let Some(at) = self.readout(id) {
            store_f64(&self.readouts[at], value);
            kResultOk
        } else if id == GAIN_PARAM {
            store_f64(&self.gain, clamp_gain(value));
            kResultOk
        } else if let Some(kept) = self.kept(id) {
            store_f64(kept, value);
            kResultOk
        } else {
            kInvalidArgument
        }
    }

    unsafe fn string_of(&self, id: ParamID, value: ParamValue, string: *mut String128) -> i32 {
        // SAFETY: the host's out parameter (`getParamStringByValue`), null or 128 units.
        let Some(string) = (unsafe { string.as_mut() }).filter(|_| self.knows(id)) else {
            return kInvalidArgument;
        };
        let text = if self.kind == Kind::Gain && id == STEPS_PARAM {
            // The step the format's decoding gives: min(n, floor(v·(n + 1))).
            let step = ((value * f64::from(STEPS + 1)).floor() as i32).min(STEPS);
            format!("{step}/{STEPS}")
        } else {
            format!("{value:.2}")
        };
        copy_wstring(&text, string);
        kResultOk
    }

    unsafe fn value_of(&self, id: ParamID, string: *mut TChar, value: *mut ParamValue) -> i32 {
        if string.is_null() || value.is_null() || !self.knows(id) {
            return kInvalidArgument;
        }
        // SAFETY: the host's string (`getParamValueByString`), non-null (checked) and
        // NUL-terminated, so every unit up to its 0 may be read.
        let len = (0..)
            .take_while(|&at| unsafe { *string.wrapping_add(at) } != 0)
            .count();
        // SAFETY: the `len` units before the 0, just counted.
        let text = String::from_utf16_lossy(unsafe { slice::from_raw_parts(string, len) });
        match text.trim().parse::<f64>() {
            Ok(parsed) => {
                // SAFETY: the host's out parameter, non-null (checked).
                unsafe { *value = clamp_gain(parsed) };
                kResultOk
            }
            Err(_) => kInvalidArgument,
        }
    }

    unsafe fn set_handler(&self, handler: *mut IComponentHandler) -> i32 {
        // SAFETY: the host's handler (`setComponentHandler`), null or live for the call; the
        // `ComPtr` keeps a reference of our own.
        let handler = unsafe { ComRef::from_raw(handler) }.map(|handler| handler.to_com_ptr());
        if let Ok(mut held) = self.handler.lock() {
            *held = handler;
        }
        kResultOk
    }

    /// The View's editor, for `name` "editor" (on Linux); none for another name, class or
    /// platform. The editor's one reference is the caller's.
    unsafe fn create_view(&self, name: FIDString) -> *mut IPlugView {
        if self.kind != Kind::View || name.is_null() {
            return ptr::null_mut();
        }
        // SAFETY: a non-null view name is NUL-terminated (`FIDString`), live for the call.
        let name = unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned();
        log(format!("createView {name}"));
        if name != "editor" {
            return ptr::null_mut();
        }
        #[cfg(target_os = "linux")]
        return view::make();
        #[cfg(not(target_os = "linux"))]
        ptr::null_mut()
    }

    /// The sine controller's side of the link: on connecting, "hello" with 7.
    unsafe fn connect(&self, other: *mut IConnectionPoint) -> i32 {
        // SAFETY: the host's point (`connect`), null or live for the call; `to_com_ptr` keeps a
        // reference of our own.
        let Some(other) = (unsafe { ComRef::from_raw(other) }) else {
            return kInvalidArgument;
        };
        let peer = other.to_com_ptr();
        *self.peer.borrow_mut() = Some(peer.clone());
        let host = self.host.borrow().clone();
        if let Some(host) = host {
            // SAFETY: both held, so live; `connect` is a main-thread call.
            unsafe { send(&host, &peer, c"hello", 7) };
        }
        kResultOk
    }

    /// An "ack" of 8 completes the link.
    unsafe fn notify(&self, message: *mut IMessage) -> i32 {
        // SAFETY: the host's message (`notify`), null or live for the call.
        match unsafe { read_message(message) } {
            Some((id, 8)) if id == b"ack" => {
                store_f64(&self.linked, 1.0);
                kResultOk
            }
            _ => kResultFalse,
        }
    }
}

/// The IComponent and IAudioProcessor methods of a class whose `core` field is its
/// [`Processor`].
macro_rules! processor_methods {
    ($class:ty) => {
        impl IComponentTrait for $class {
            unsafe fn getControllerClassId(&self, class_id: *mut TUID) -> i32 {
                match (self.core.kind.controller_cid(), class_id.is_null()) {
                    (Some(cid), false) => {
                        // SAFETY: the host's out parameter, non-null (checked).
                        unsafe { *class_id = cid };
                        kResultOk
                    }
                    _ => kResultFalse,
                }
            }

            unsafe fn setIoMode(&self, _mode: IoMode) -> i32 {
                kResultOk
            }

            unsafe fn getBusCount(&self, media: MediaType, dir: BusDirection) -> i32 {
                self.core.bus_count(media, dir)
            }

            unsafe fn getBusInfo(
                &self,
                media: MediaType,
                dir: BusDirection,
                index: i32,
                bus: *mut BusInfo,
            ) -> i32 {
                // SAFETY: this method's contract, which `bus_info` shares.
                unsafe { self.core.bus_info(media, dir, index, bus) }
            }

            unsafe fn getRoutingInfo(
                &self,
                _in_info: *mut RoutingInfo,
                _out_info: *mut RoutingInfo,
            ) -> i32 {
                kNotImplemented
            }

            unsafe fn activateBus(
                &self,
                _media: MediaType,
                _dir: BusDirection,
                _index: i32,
                _state: TBool,
            ) -> i32 {
                kResultOk
            }

            unsafe fn setActive(&self, state: TBool) -> i32 {
                if state == 0 {
                    self.core.stop();
                }
                kResultOk
            }

            unsafe fn setState(&self, stream: *mut IBStream) -> i32 {
                // SAFETY: as for `getBusInfo`.
                unsafe { self.core.set_state(stream) }
            }

            unsafe fn getState(&self, stream: *mut IBStream) -> i32 {
                // SAFETY: as for `getBusInfo`.
                unsafe { self.core.get_state(stream) }
            }
        }

        impl IAudioProcessorTrait for $class {
            unsafe fn setBusArrangements(
                &self,
                inputs: *mut SpeakerArrangement,
                ins: i32,
                outputs: *mut SpeakerArrangement,
                outs: i32,
            ) -> i32 {
                // SAFETY: as for `getBusInfo`.
                unsafe { self.core.set_arrangements(inputs, ins, outputs, outs) }
            }

            unsafe fn getBusArrangement(
                &self,
                dir: BusDirection,
                index: i32,
                arrangement: *mut SpeakerArrangement,
            ) -> i32 {
                // SAFETY: as for `getBusInfo`.
                unsafe { self.core.arrangement(dir, index, arrangement) }
            }

            unsafe fn canProcessSampleSize(&self, size: i32) -> i32 {
                if size == SAMPLE_32 {
                    kResultTrue
                } else {
                    kResultFalse
                }
            }

            unsafe fn getLatencySamples(&self) -> u32 {
                self.core.latency()
            }

            unsafe fn setupProcessing(&self, setup: *mut ProcessSetup) -> i32 {
                // SAFETY: as for `getBusInfo`.
                unsafe { self.core.setup(setup) }
            }

            unsafe fn setProcessing(&self, state: TBool) -> i32 {
                if state == 0 {
                    self.core.stop();
                }
                kResultOk
            }

            unsafe fn process(&self, data: *mut ProcessData) -> i32 {
                // SAFETY: as for `getBusInfo`.
                unsafe { self.core.process(data) }
            }

            unsafe fn getTailSamples(&self) -> u32 {
                0
            }
        }
    };
}

/// The IEditController methods of a class whose `controller` field is its [`Controller`].
macro_rules! controller_methods {
    ($class:ty) => {
        impl IEditControllerTrait for $class {
            unsafe fn setComponentState(&self, stream: *mut IBStream) -> i32 {
                // SAFETY: this method's contract, which the `Controller` method shares.
                unsafe { self.controller.set_component_state(stream) }
            }

            unsafe fn setState(&self, stream: *mut IBStream) -> i32 {
                // SAFETY: as for `setComponentState`.
                unsafe { self.controller.set_state(stream) }
            }

            unsafe fn getState(&self, stream: *mut IBStream) -> i32 {
                // SAFETY: as for `setComponentState`.
                unsafe { self.controller.get_state(stream) }
            }

            unsafe fn getParameterCount(&self) -> i32 {
                self.controller.param_count()
            }

            unsafe fn getParameterInfo(&self, index: i32, info: *mut ParameterInfo) -> i32 {
                // SAFETY: as for `setComponentState`.
                unsafe { self.controller.param_info(index, info) }
            }

            unsafe fn getParamStringByValue(
                &self,
                id: ParamID,
                value: ParamValue,
                string: *mut String128,
            ) -> i32 {
                // SAFETY: as for `setComponentState`.
                unsafe { self.controller.string_of(id, value, string) }
            }

            unsafe fn getParamValueByString(
                &self,
                id: ParamID,
                string: *mut TChar,
                value: *mut ParamValue,
            ) -> i32 {
                // SAFETY: as for `setComponentState`.
                unsafe { self.controller.value_of(id, string, value) }
            }

            unsafe fn normalizedParamToPlain(&self, _id: ParamID, value: ParamValue) -> f64 {
                value
            }

            unsafe fn plainParamToNormalized(&self, _id: ParamID, value: ParamValue) -> f64 {
                value
            }

            unsafe fn getParamNormalized(&self, id: ParamID) -> f64 {
                self.controller.normalized(id)
            }

            unsafe fn setParamNormalized(&self, id: ParamID, value: ParamValue) -> i32 {
                self.controller.set_normalized(id, value)
            }

            unsafe fn setComponentHandler(&self, handler: *mut IComponentHandler) -> i32 {
                // SAFETY: as for `setComponentState`.
                unsafe { self.controller.set_handler(handler) }
            }

            unsafe fn createView(&self, name: FIDString) -> *mut IPlugView {
                // SAFETY: as for `setComponentState`.
                unsafe { self.controller.create_view(name) }
            }
        }
    };
}

/// The sine's component (and the View's), whose controller is a class of its own.
struct SineComponent {
    core: Processor,
    host: RefCell<Option<ComPtr<IHostApplication>>>,
    peer: RefCell<Option<ComPtr<IConnectionPoint>>>,
    _live: Live,
}

impl Class for SineComponent {
    type Interfaces = (IComponent, IAudioProcessor, IConnectionPoint);
}

impl IPluginBaseTrait for SineComponent {
    unsafe fn initialize(&self, context: *mut FUnknown) -> i32 {
        // SAFETY: the host's context, null or live for the call; `cast` keeps our own reference.
        let Some(context) = (unsafe { ComRef::from_raw(context) }) else {
            return kInvalidArgument;
        };
        *self.host.borrow_mut() = context.cast::<IHostApplication>();
        kResultOk
    }

    unsafe fn terminate(&self) -> i32 {
        if self.core.kind == Kind::View {
            log("component terminate");
        }
        self.host.borrow_mut().take();
        self.peer.borrow_mut().take();
        kResultOk
    }
}

processor_methods!(SineComponent);

impl IConnectionPointTrait for SineComponent {
    unsafe fn connect(&self, other: *mut IConnectionPoint) -> i32 {
        // SAFETY: the host's point, null or live for the call; `to_com_ptr` keeps our own
        // reference.
        let Some(other) = (unsafe { ComRef::from_raw(other) }) else {
            return kInvalidArgument;
        };
        *self.peer.borrow_mut() = Some(other.to_com_ptr());
        kResultOk
    }

    unsafe fn disconnect(&self, _other: *mut IConnectionPoint) -> i32 {
        self.peer.borrow_mut().take();
        kResultOk
    }

    /// "hello" with n: answered "ack" with n + 1.
    unsafe fn notify(&self, message: *mut IMessage) -> i32 {
        // SAFETY: the host's message (`notify`), null or live for the call.
        let Some((id, n)) = (unsafe { read_message(message) }) else {
            return kResultFalse;
        };
        if id != b"hello" {
            return kResultFalse;
        }
        let (host, peer) = (self.host.borrow().clone(), self.peer.borrow().clone());
        if let (Some(host), Some(peer)) = (host, peer) {
            // SAFETY: both held, so live; `notify` comes on the main thread.
            unsafe { send(&host, &peer, c"ack", n + 1) };
        }
        kResultOk
    }
}

/// The sine's controller (and the View's).
struct SineController {
    controller: Controller,
    _live: Live,
}

impl Class for SineController {
    type Interfaces = (IEditController, IConnectionPoint, IMidiMapping);
}

impl IMidiMappingTrait for SineController {
    unsafe fn getMidiControllerAssignment(
        &self,
        bus: int32,
        channel: int16,
        number: int16,
        id: *mut ParamID,
    ) -> i32 {
        // SAFETY: this method's contract, which `assignment` shares.
        unsafe { self.controller.assignment(bus, channel, number, id) }
    }
}

impl IPluginBaseTrait for SineController {
    unsafe fn initialize(&self, context: *mut FUnknown) -> i32 {
        // SAFETY: this method's contract, which `Controller::initialize` shares.
        unsafe { self.controller.initialize(context) }
    }

    unsafe fn terminate(&self) -> i32 {
        // SAFETY: as for `initialize`.
        unsafe { self.controller.terminate() }
    }
}

controller_methods!(SineController);

impl IConnectionPointTrait for SineController {
    unsafe fn connect(&self, other: *mut IConnectionPoint) -> i32 {
        // SAFETY: this method's contract, which `Controller::connect` shares.
        unsafe { self.controller.connect(other) }
    }

    unsafe fn disconnect(&self, _other: *mut IConnectionPoint) -> i32 {
        self.controller.peer.borrow_mut().take();
        kResultOk
    }

    unsafe fn notify(&self, message: *mut IMessage) -> i32 {
        // SAFETY: this method's contract, which `Controller::notify` shares.
        unsafe { self.controller.notify(message) }
    }
}

/// A class that is its own controller: the gain, the fail, the tick and the latency classes.
struct SingleComponent {
    core: Processor,
    controller: Controller,
    _live: Live,
}

impl Class for SingleComponent {
    type Interfaces = (IComponent, IAudioProcessor, IEditController);
}

impl IPluginBaseTrait for SingleComponent {
    unsafe fn initialize(&self, context: *mut FUnknown) -> i32 {
        // SAFETY: this method's contract, which `Controller::initialize` shares.
        unsafe { self.controller.initialize(context) }
    }

    unsafe fn terminate(&self) -> i32 {
        // SAFETY: as for `initialize`.
        unsafe { self.controller.terminate() }
    }
}

processor_methods!(SingleComponent);
controller_methods!(SingleComponent);

/// A class the factory lists: a kind's audio class, or the controller class of its own a kind
/// has.
#[derive(Debug, Clone, Copy)]
enum Listed {
    Audio(Kind),
    Controller(Kind),
}

/// What the factory lists, in its order.
const CLASSES: [Listed; 8] = [
    Listed::Audio(Kind::Sine),
    Listed::Controller(Kind::Sine),
    Listed::Audio(Kind::Gain),
    Listed::Audio(Kind::Fail),
    Listed::Audio(Kind::Tick),
    Listed::Audio(Kind::Latency),
    Listed::Audio(Kind::View),
    Listed::Controller(Kind::View),
];

/// Class `index`'s id, category, name and sub-categories.
fn class_entry(index: i32) -> Option<(TUID, &'static str, &'static str, &'static str)> {
    Some(match *CLASSES.get(usize::try_from(index).ok()?)? {
        Listed::Audio(kind) => (
            kind.cid(),
            "Audio Module Class",
            kind.name(),
            kind.sub_categories(),
        ),
        Listed::Controller(kind) => {
            let name = if kind == Kind::View {
                "Dawsome Test View Controller"
            } else {
                "Dawsome Test Sine Controller"
            };
            (
                kind.controller_cid()?,
                "Component Controller Class",
                name,
                "",
            )
        }
    })
}

struct Factory {
    /// What the host gave `setHostContext`, kept as the SDK's factories keep it.
    context: RefCell<Option<ComPtr<FUnknown>>>,
    _live: Live,
}

impl Class for Factory {
    type Interfaces = (IPluginFactory3,);
}

impl IPluginFactoryTrait for Factory {
    unsafe fn getFactoryInfo(&self, info: *mut PFactoryInfo) -> i32 {
        // SAFETY: the host's out parameter, null or live for the call.
        let Some(info) = (unsafe { info.as_mut() }) else {
            return kInvalidArgument;
        };
        copy_cstring("Dawsome", &mut info.vendor);
        copy_cstring("", &mut info.url);
        copy_cstring("", &mut info.email);
        info.flags = UNICODE;
        kResultOk
    }

    unsafe fn countClasses(&self) -> i32 {
        CLASSES.len() as i32
    }

    unsafe fn getClassInfo(&self, index: i32, info: *mut PClassInfo) -> i32 {
        // SAFETY: the host's out parameter, null or live for the call.
        let info = unsafe { info.as_mut() };
        let (Some((cid, category, name, _)), Some(info)) = (class_entry(index), info) else {
            return kInvalidArgument;
        };
        info.cid = cid;
        info.cardinality = MANY_INSTANCES;
        copy_cstring(category, &mut info.category);
        copy_cstring(name, &mut info.name);
        kResultOk
    }

    unsafe fn createInstance(&self, cid: FIDString, iid: FIDString, obj: *mut *mut c_void) -> i32 {
        if cid.is_null() || iid.is_null() || obj.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the host's out parameter, non-null (checked).
        unsafe { *obj = ptr::null_mut() };
        // SAFETY: a class id is 16 bytes (`createInstance`'s `cid`), non-null (checked); `TUID`
        // is bytes, so any address is aligned for it.
        let cid = unsafe { *(cid as *const TUID) };
        let separate = [Kind::Sine, Kind::View];
        let made: Option<ComPtr<FUnknown>> =
            if let Some(kind) = separate.into_iter().find(|kind| kind.cid() == cid) {
                ComWrapper::new(SineComponent {
                    core: Processor::new(kind, Arc::new(AtomicUsize::new(LATENCY))),
                    host: RefCell::new(None),
                    peer: RefCell::new(None),
                    _live: Live::new(),
                })
                .to_com_ptr()
            } else if let Some(kind) = separate
                .into_iter()
                .find(|kind| kind.controller_cid() == Some(cid))
            {
                ComWrapper::new(SineController {
                    controller: Controller::new(kind, Arc::new(AtomicUsize::new(LATENCY))),
                    _live: Live::new(),
                })
                .to_com_ptr()
            } else {
                [Kind::Gain, Kind::Fail, Kind::Tick, Kind::Latency]
                    .into_iter()
                    .find(|kind| kind.cid() == cid)
                    .and_then(|kind| {
                        let delay = Arc::new(AtomicUsize::new(LATENCY));
                        ComWrapper::new(SingleComponent {
                            core: Processor::new(kind, Arc::clone(&delay)),
                            controller: Controller::new(kind, delay),
                            _live: Live::new(),
                        })
                        .to_com_ptr()
                    })
            };
        let Some(made) = made else {
            return kInvalidArgument;
        };
        let unknown = made.as_ptr();
        // SAFETY: `unknown` is our object, live while `made` holds it; a COM object's first
        // field is its vtable pointer.
        let vtbl = unsafe { (*unknown).vtbl };
        // SAFETY: the vtable of our own class (`vst3::Class`), static and never null.
        let query = unsafe { (*vtbl).queryInterface };
        // SAFETY: our queryInterface on our live object, with the host's 16-byte id (non-null,
        // checked) and out parameter; it adds the reference the host gets.
        unsafe { query(unknown, iid as *const TUID, obj) }
    }
}

impl IPluginFactory2Trait for Factory {
    unsafe fn getClassInfo2(&self, index: i32, info: *mut PClassInfo2) -> i32 {
        // SAFETY: the host's out parameter, null or live for the call.
        let info = unsafe { info.as_mut() };
        let (Some((cid, category, name, sub_categories)), Some(info)) = (class_entry(index), info)
        else {
            return kInvalidArgument;
        };
        info.cid = cid;
        info.cardinality = MANY_INSTANCES;
        copy_cstring(category, &mut info.category);
        copy_cstring(name, &mut info.name);
        info.classFlags = 0;
        copy_cstring(sub_categories, &mut info.subCategories);
        copy_cstring("Dawsome", &mut info.vendor);
        copy_cstring("1.0.0", &mut info.version);
        copy_cstring("VST 3.8.0", &mut info.sdkVersion);
        kResultOk
    }
}

impl IPluginFactory3Trait for Factory {
    unsafe fn getClassInfoUnicode(&self, index: i32, info: *mut PClassInfoW) -> i32 {
        // SAFETY: the host's out parameter, null or live for the call.
        let info = unsafe { info.as_mut() };
        let (Some((cid, category, name, sub_categories)), Some(info)) = (class_entry(index), info)
        else {
            return kInvalidArgument;
        };
        info.cid = cid;
        info.cardinality = MANY_INSTANCES;
        copy_cstring(category, &mut info.category);
        copy_wstring(name, &mut info.name);
        info.classFlags = 0;
        copy_cstring(sub_categories, &mut info.subCategories);
        copy_wstring("Dawsome", &mut info.vendor);
        copy_wstring("1.0.0", &mut info.version);
        copy_wstring("VST 3.8.0", &mut info.sdkVersion);
        kResultOk
    }

    unsafe fn setHostContext(&self, context: *mut FUnknown) -> i32 {
        // SAFETY: the host's context, null or live for the call; the `ComPtr` keeps our own
        // reference.
        let context = unsafe { ComRef::from_raw(context) };
        *self.context.borrow_mut() = context.map(|context| context.to_com_ptr());
        kResultOk
    }
}

#[cfg(target_os = "linux")]
#[no_mangle]
extern "system" fn ModuleEntry(_library_handle: *mut c_void) -> bool {
    ENTERED.fetch_add(1, Ordering::AcqRel);
    true
}

#[cfg(target_os = "linux")]
#[no_mangle]
extern "system" fn ModuleExit() -> bool {
    ENTERED.fetch_sub(1, Ordering::AcqRel);
    true
}

#[cfg(windows)]
#[no_mangle]
extern "system" fn InitDll() -> bool {
    ENTERED.fetch_add(1, Ordering::AcqRel);
    true
}

#[cfg(windows)]
#[no_mangle]
extern "system" fn ExitDll() -> bool {
    ENTERED.fetch_sub(1, Ordering::AcqRel);
    true
}

/// A new factory, its one reference the caller's.
#[no_mangle]
extern "system" fn GetPluginFactory() -> *mut IPluginFactory {
    ComWrapper::new(Factory {
        context: RefCell::new(None),
        _live: Live::new(),
    })
    .to_com_ptr::<IPluginFactory>()
    .map_or(ptr::null_mut(), ComPtr::into_raw)
}
