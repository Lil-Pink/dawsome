//! A plugin's parts and their lifecycle, as the SDK's call-sequence diagrams and
//! `plugprovider.cpp` order them (plan 1.4d), shared by the scan child (which describes a class)
//! and the host (which plays one):
//!
//! 1. the component made and initialized with a host context of its own;
//! 2. the controller: the component itself when it answers `IEditController` (a single
//!    component), else the class `getControllerClassId` names, made and initialized;
//! 3. two separate parts connected both ways, each to a proxy that passes messages on to the
//!    other on the main thread ([`ConnectionProxy`]);
//! 4. the controller given the instance's component handler;
//! 5. (the host) the state, then the buses, sample size, processing setup and activation.
//!
//! Closing undoes it in reverse: processing off, deactivated, disconnected, the controller
//! terminated (when it is a part of its own) then the component, the context's registrations
//! released. The parts close once; a single component is terminated once.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use dawsome_engine::realtime::BLOCK_FRAMES;
use vst3::Steinberg::Vst::{
    BusInfo, IAudioProcessor, IAudioProcessorTrait, IComponent, IComponentHandler, IComponentTrait,
    IConnectionPoint, IConnectionPointTrait, IEditController, IEditControllerTrait, ProcessSetup,
    SpeakerArr, SpeakerArrangement,
};
use vst3::Steinberg::{kResultOk, IBStream, IPluginBaseTrait, TUID};
use vst3::{ComPtr, ComWrapper};

use crate::com::connection::ConnectionProxy;
use crate::com::context::{self, HostContext};
use crate::com::handler::{ComponentHandler, Edit, Signals};
use crate::com::stream::MemoryStream;
use crate::constants::{AUDIO, EVENT, INPUT, OFF, ON, OUTPUT, REALTIME, SAMPLE_32};
use crate::module::Module;
use crate::preset::{self, Preset};
use crate::uid;

/// The most channels one audio bus of a hosted plugin may have (the CLAP host's limit).
pub(crate) const MAX_BUS_CHANNELS: u32 = 32;

/// What a plugin plays for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// A plugin channel: no audio in, notes in, audio out.
    Instrument,
    /// A mixer slot: the insert's audio in and out, no notes.
    Effect,
}

/// The buses the audio half sizes its buffers for, as the plugin settled them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layout {
    pub(crate) role: Role,
    /// Channels of each audio input bus, then of each output bus; bus 0 is the main one.
    pub(crate) inputs: Vec<u32>,
    pub(crate) outputs: Vec<u32>,
    /// It has an event input and plays as an instrument: it is sent notes.
    pub(crate) takes_notes: bool,
    pub(crate) latency: u32,
}

/// The two ends of a connection and the proxies between them.
struct Connection {
    component: ComPtr<IConnectionPoint>,
    to_controller: ComWrapper<ConnectionProxy>,
    controller: ComPtr<IConnectionPoint>,
    to_component: ComWrapper<ConnectionProxy>,
}

/// A connection proxy as the connection point a part is connected to.
fn as_point(proxy: &ComWrapper<ConnectionProxy>) -> *mut IConnectionPoint {
    proxy
        .as_com_ref::<IConnectionPoint>()
        .map_or(std::ptr::null_mut(), |point| point.as_ptr())
}

/// A stream as the `IBStream` a part reads or writes.
fn as_stream(stream: &ComWrapper<MemoryStream>) -> *mut IBStream {
    stream
        .as_com_ref::<IBStream>()
        .map_or(std::ptr::null_mut(), |stream| stream.as_ptr())
}

/// One plugin instance's parts. Invariant every `unsafe` call on them relies on: `component`,
/// `audio` and `controller` are live (this holds a reference to each) and initialized from
/// `open` until `close`, and every call into them is made on the main thread, except the audio
/// half's, which holds its own reference to the processor.
pub(crate) struct Parts {
    pub(crate) cid: TUID,
    pub(crate) component: ComPtr<IComponent>,
    pub(crate) audio: Option<ComPtr<IAudioProcessor>>,
    pub(crate) controller: Option<ComPtr<IEditController>>,
    /// The controller is an object of its own, initialized and terminated apart.
    separate: bool,
    connection: Option<Connection>,
    pub(crate) context: ComWrapper<HostContext>,
    pub(crate) handler: ComWrapper<ComponentHandler>,
    pub(crate) signals: Arc<Signals>,
    /// `setProcessing(true)` was called and not undone; shared with the audio half.
    pub(crate) processing: Arc<AtomicBool>,
    active: Cell<bool>,
    closed: Cell<bool>,
    /// Held so the module outlives its objects: declared last, so it drops after every
    /// reference to them above.
    _module: Arc<Module>,
}

impl Parts {
    /// Makes class `cid` of `module` up to its handler (steps 1 to 4); the reason when it will
    /// not start. What was made is closed again on the way out.
    pub(crate) fn open(
        module: &Arc<Module>,
        cid: &TUID,
        signals: Arc<Signals>,
        edits: rtrb::Producer<Edit>,
    ) -> Result<Self, String> {
        let component = module
            .component(cid)
            .ok_or_else(|| "would not start: its factory made no component".to_owned())?;
        let context = HostContext::new();
        // SAFETY: the component is live; the context is our object, which it may keep.
        if unsafe { component.initialize(context::as_unknown(&context)) } != kResultOk {
            return Err("would not start: its component would not initialize".into());
        }
        let mut parts = Self {
            _module: Arc::clone(module),
            cid: *cid,
            audio: component.cast::<IAudioProcessor>(),
            controller: None,
            separate: false,
            connection: None,
            handler: ComponentHandler::new(Arc::clone(&signals), edits),
            component,
            context,
            signals,
            processing: Arc::new(AtomicBool::new(false)),
            active: Cell::new(false),
            closed: Cell::new(false),
        };
        if let Some(single) = parts.component.cast::<IEditController>() {
            parts.controller = Some(single);
        } else {
            let mut class: TUID = [0; 16];
            // SAFETY: the component is live; `class` is our out parameter.
            if unsafe { parts.component.getControllerClassId(&mut class) } == kResultOk {
                if let Some(controller) = module.create::<IEditController>(&class) {
                    // SAFETY: as for the component.
                    let started =
                        unsafe { controller.initialize(context::as_unknown(&parts.context)) }
                            == kResultOk;
                    if started {
                        parts.controller = Some(controller);
                        parts.separate = true;
                    }
                }
            }
        }
        parts.connect();
        if let Some(controller) = &parts.controller {
            let handler = parts
                .handler
                .as_com_ref::<IComponentHandler>()
                .map_or(std::ptr::null_mut(), |handler| handler.as_ptr());
            // SAFETY: the controller is live; the handler is our object, which it keeps.
            unsafe { controller.setComponentHandler(handler) };
        }
        Ok(parts)
    }

    /// Connects two separate parts both ways through proxies (step 3).
    fn connect(&mut self) {
        if !self.separate {
            return;
        }
        let (Some(component), Some(controller)) = (
            self.component.cast::<IConnectionPoint>(),
            self.controller
                .as_ref()
                .and_then(|controller| controller.cast::<IConnectionPoint>()),
        ) else {
            return;
        };
        let to_controller = ConnectionProxy::new(controller.clone());
        let to_component = ConnectionProxy::new(component.clone());
        // SAFETY: both points are live parts (`self` holds them until `close`); the proxies are
        // our objects, which they keep. The component first, so the controller's first message
        // (sent from its `connect`) finds the component connected.
        unsafe { component.connect(as_point(&to_controller)) };
        // SAFETY: as for the component.
        unsafe { controller.connect(as_point(&to_component)) };
        self.connection = Some(Connection {
            component,
            to_controller,
            controller,
            to_component,
        });
    }

    /// The class id's text.
    pub(crate) fn class(&self) -> String {
        uid::text(&self.cid)
    }

    /// With no saved state: the component's state as it starts, handed to the controller, as a
    /// host does after setting up a new instance.
    pub(crate) fn share_component_state(&self) {
        let stream = MemoryStream::new(Vec::new());
        // SAFETY: the component is live; the stream is ours.
        if unsafe { self.component.getState(as_stream(&stream)) } != kResultOk {
            return;
        }
        if let Some(controller) = &self.controller {
            stream.rewind();
            // SAFETY: as above.
            unsafe { controller.setComponentState(as_stream(&stream)) };
        }
    }

    /// The component's and the controller's states now, as a preset file; the reason when the
    /// component will not save its state.
    pub(crate) fn capture(&self) -> Result<Vec<u8>, String> {
        let component = MemoryStream::new(Vec::new());
        // SAFETY: the component is live; the stream is ours.
        if unsafe { self.component.getState(as_stream(&component)) } != kResultOk {
            return Err("it would not save its state".into());
        }
        let controller = self.controller.as_ref().and_then(|controller| {
            let stream = MemoryStream::new(Vec::new());
            // SAFETY: as above.
            (unsafe { controller.getState(as_stream(&stream)) } == kResultOk).then(|| stream.data())
        });
        Ok(preset::write(
            &self.class(),
            &component.data(),
            controller.as_deref(),
        ))
    }

    /// Loads `preset` into the parts: the component's state, then the same bytes into the
    /// controller (`setComponentState`), then the controller's own when the preset holds it.
    /// The reason when the component refuses its state.
    pub(crate) fn load(&self, preset: &Preset<'_>) -> Result<(), String> {
        let component = MemoryStream::new(preset.component.to_vec());
        // SAFETY: the component is live; the stream is ours.
        if unsafe { self.component.setState(as_stream(&component)) } != kResultOk {
            return Err("it refused its saved state".into());
        }
        if let Some(controller) = &self.controller {
            component.rewind();
            // SAFETY: as above. A controller that takes no component state (some answer
            // kNotImplemented) keeps its own.
            unsafe { controller.setComponentState(as_stream(&component)) };
            if let Some(state) = preset.controller {
                let stream = MemoryStream::new(state.to_vec());
                // SAFETY: as above.
                unsafe { controller.setState(as_stream(&stream)) };
            }
        }
        Ok(())
    }

    /// The channel count of each audio bus of `direction` as the plugin describes it now.
    fn bus_channels(&self, direction: i32) -> Vec<u32> {
        // SAFETY: the component is live until `close` (the struct's invariant).
        let count = unsafe { self.component.getBusCount(AUDIO, direction) };
        (0..count)
            .map(|index| {
                // SAFETY: `BusInfo` is plain C data (numbers and a UTF-16 array), for which all
                // zero bytes are a value.
                let mut info: BusInfo = unsafe { std::mem::zeroed() };
                // SAFETY: as for the count; `info` is ours to fill.
                let found = unsafe {
                    self.component
                        .getBusInfo(AUDIO, direction, index, &mut info)
                };
                if found == kResultOk {
                    u32::try_from(info.channelCount).unwrap_or(0)
                } else {
                    0
                }
            })
            .collect()
    }

    /// The buses as a scan describes them: each audio input's and output's channels, as the
    /// plugin offers them before any arrangement is asked of it, and whether it has an event
    /// input.
    pub(crate) fn shape(&self) -> (Vec<u32>, Vec<u32>, bool) {
        // SAFETY: the component is live.
        let events = unsafe { self.component.getBusCount(EVENT, INPUT) } > 0;
        (self.bus_channels(INPUT), self.bus_channels(OUTPUT), events)
    }

    /// How many parameters its controller has (0 with none).
    pub(crate) fn param_count(&self) -> u32 {
        self.controller.as_ref().map_or(0, |controller| {
            // SAFETY: the controller is live.
            u32::try_from(unsafe { controller.getParameterCount() }).unwrap_or(0)
        })
    }

    /// Step 5 after the state: buses settled (the main ones stereo, every other kept as the
    /// plugin has it), the buses used activated, 32-bit samples, the processing setup for
    /// `sample_rate`, and activation. The CLAP host's texts for what cannot be hosted.
    pub(crate) fn activate(&self, role: Role, sample_rate: u32) -> Result<Layout, String> {
        let audio = self
            .audio
            .as_ref()
            .ok_or_else(|| "its main output is not mono or stereo".to_owned())?;
        let arrangements = |direction: i32| -> Vec<SpeakerArrangement> {
            // SAFETY: the component is live until `close` (the struct's invariant).
            let count = unsafe { self.component.getBusCount(AUDIO, direction) };
            (0..count)
                .map(|index| {
                    let mut arrangement: SpeakerArrangement = 0;
                    // SAFETY: the processor is the component's own interface, live as it is;
                    // `arrangement` is ours to fill.
                    let found =
                        unsafe { audio.getBusArrangement(direction, index, &mut arrangement) };
                    if found == kResultOk {
                        arrangement
                    } else {
                        SpeakerArr::kEmpty
                    }
                })
                .collect()
        };
        let mut inputs = arrangements(INPUT);
        let mut outputs = arrangements(OUTPUT);
        if let Some(main) = outputs.first_mut() {
            *main = SpeakerArr::kStereo;
        }
        if role == Role::Effect {
            if let Some(main) = inputs.first_mut() {
                *main = SpeakerArr::kStereo;
            }
        }
        // SAFETY: the processor is inactive (arrangements change only then); the arrays are
        // ours, of the counts given. A plugin that will not take them keeps its own, read back
        // below.
        unsafe {
            audio.setBusArrangements(
                inputs.as_mut_ptr(),
                inputs.len() as i32,
                outputs.as_mut_ptr(),
                outputs.len() as i32,
            );
        }
        let settled = |direction: i32| -> Result<Vec<u32>, String> {
            arrangements(direction)
                .into_iter()
                .map(|arrangement| {
                    let channels = arrangement.count_ones();
                    if channels > MAX_BUS_CHANNELS {
                        Err("it has too many audio channels".to_owned())
                    } else {
                        Ok(channels)
                    }
                })
                .collect()
        };
        let (inputs, outputs) = (settled(INPUT)?, settled(OUTPUT)?);
        let mono_or_stereo = |buses: &[u32]| matches!(buses.first(), Some(1 | 2));
        if !mono_or_stereo(&outputs) {
            return Err("its main output is not mono or stereo".into());
        }
        if role == Role::Effect && !mono_or_stereo(&inputs) {
            return Err("it has no mono or stereo input".into());
        }
        // SAFETY: the component is live and inactive.
        let events = unsafe { self.component.getBusCount(EVENT, INPUT) } > 0;
        let takes_notes = role == Role::Instrument && events;
        // The calls below are on the live, still inactive component and its processor, on the
        // main thread, in the order the format's setup takes (buses, sample size, processing
        // setup, then activation).
        if role == Role::Effect {
            // SAFETY: as just said; buses are activated while inactive.
            unsafe { self.component.activateBus(AUDIO, INPUT, 0, ON) };
        }
        // SAFETY: as said above `activateBus`.
        unsafe { self.component.activateBus(AUDIO, OUTPUT, 0, ON) };
        if takes_notes {
            // SAFETY: as said above `activateBus`.
            unsafe { self.component.activateBus(EVENT, INPUT, 0, ON) };
        }
        // SAFETY: as said above `activateBus`.
        if unsafe { audio.canProcessSampleSize(SAMPLE_32) } != kResultOk {
            return Err("it does not process 32-bit samples".into());
        }
        let mut setup = ProcessSetup {
            processMode: REALTIME,
            symbolicSampleSize: SAMPLE_32,
            maxSamplesPerBlock: BLOCK_FRAMES as i32,
            sampleRate: f64::from(sample_rate),
        };
        // SAFETY: as said above `activateBus`; `setup` is ours, read for the call.
        if unsafe { audio.setupProcessing(&mut setup) } != kResultOk {
            return Err("would not activate: it refused its processing setup".into());
        }
        // SAFETY: as said above `activateBus`; set up just now.
        if unsafe { self.component.setActive(ON) } != kResultOk {
            return Err("would not activate".into());
        }
        self.active.set(true);
        Ok(Layout {
            role,
            inputs,
            outputs,
            takes_notes,
            latency: self.latency(),
        })
    }

    /// The latency the processor reports, in frames.
    pub(crate) fn latency(&self) -> u32 {
        self.audio.as_ref().map_or(0, |audio| {
            // SAFETY: the processor is live and set up (the format's "Setup Done or later").
            unsafe { audio.getLatencySamples() }
        })
    }

    /// Undoes [`open`](Self::open) and [`activate`](Self::activate), once, in reverse. Runs
    /// on the main thread with the audio half gone (or never made).
    pub(crate) fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        // Every part is live until this struct drops; the calls below are the teardown the
        // format orders, made once (`closed`), on the main thread with the audio half gone.
        if let Some(audio) = &self.audio {
            if self.processing.swap(false, Ordering::AcqRel) {
                // SAFETY: as just said.
                unsafe { audio.setProcessing(OFF) };
            }
        }
        if self.active.replace(false) {
            // SAFETY: as said at the top of the teardown.
            unsafe { self.component.setActive(OFF) };
        }
        if let Some(connection) = &self.connection {
            // SAFETY: as said at the top of the teardown; the proxies are ours, held by
            // `connection`.
            unsafe {
                connection
                    .component
                    .disconnect(as_point(&connection.to_controller))
            };
            // SAFETY: as for the component.
            unsafe {
                connection
                    .controller
                    .disconnect(as_point(&connection.to_component))
            };
            connection.to_controller.cut();
            connection.to_component.cut();
        }
        if let (true, Some(controller)) = (self.separate, &self.controller) {
            // SAFETY: as said at the top of the teardown; a separate controller is terminated
            // on its own, a single one with the component.
            unsafe { controller.terminate() };
        }
        // SAFETY: as said at the top of the teardown; the last call into it.
        unsafe { self.component.terminate() };
        self.context.run_loop().close();
    }
}

impl Drop for Parts {
    fn drop(&mut self) {
        self.close();
    }
}
