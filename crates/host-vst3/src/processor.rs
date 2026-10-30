//! The audio half of a VST3 plugin: what the player calls on the audio thread. It holds the
//! plugin's audio processor alone, never its controller (every controller method belongs to the
//! main thread). Everything it touches there (bus buffers and their pointer arrays, the event
//! list, the parameter changes, the process context, the note schedule, the tables of waiting
//! values and stepped parameters, the rings to and from the main thread) is sized when the
//! plugin is created on the main thread, so a block allocates nothing.
//!
//! Per block it sends the plugin: the values that waited (from the engine, as points at frame
//! 0; a stepped parameter's rounded to a whole step) and the edits its controller made, a
//! device's pitch bend and mod wheel as points of the parameters the plugin's MIDI mapping names
//! for them ([`crate::mapping`]), the note-offs and note-ons of the block (the [`NoteSchedule`]
//! every format's host shares: offs before ons on a shared frame, ids kept), and the transport.
//! After it, every value it sent and every value the plugin changed itself go back to the main
//! thread, so the controller and the host's `value` follow automation and the device.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use dawsome_engine::processor::{PortEvent, ProcessContext, Processor};
use dawsome_engine::time::tick_to_sample;
use dawsome_host_common::notes::{NoteEvent, NoteSchedule, EVENTS_PER_BLOCK};
use vst3::Steinberg::kResultOk;
use vst3::Steinberg::Vst::{
    AudioBusBuffers, AudioBusBuffers__type0, Event, Event__type0, IAudioProcessor,
    IAudioProcessorTrait, IEventList, IParameterChanges, NoteOffEvent, NoteOnEvent,
    ProcessContext as Vst3Context, ProcessData,
};
use vst3::{ComPtr, ComWrapper};

use crate::com::events::EventList;
use crate::com::handler::{Edit, Signals};
use crate::com::params::ParameterChanges;
use crate::constants::{
    BAR_VALID, CONTINUOUS_VALID, LIVE, MUSIC_VALID, NOTE_OFF, NOTE_ON, OFF, ON, PLAYING, REALTIME,
    SAMPLE_32, SIGNATURE_VALID, TEMPO_VALID,
};
use crate::mapping::{bend_point, wheel_point, Controls, NO_PARAM};
use crate::module::Module;
use crate::parts::{Layout, Role};

/// Values that can wait for one block; more are dropped (a newer value for a waiting parameter
/// replaces it, so this is room for 64 different parameters).
pub(crate) const WAITING_PARAMS: usize = 64;

/// Room on the ring of values back to the main thread.
pub(crate) const VALUES: usize = 1024;

/// Events one block can send: every note-off and every note-on (`EVENTS_PER_BLOCK`).
const EVENTS: usize = EVENTS_PER_BLOCK;

/// A parameter value on its way to the main thread.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Value {
    pub(crate) id: u32,
    /// Normalized, 0..1.
    pub(crate) value: f64,
    /// The plugin changed it itself (an output change), not the host.
    pub(crate) from_plugin: bool,
}

/// What the audio half is made of, gathered on the main thread.
pub(crate) struct Making {
    pub(crate) audio: ComPtr<IAudioProcessor>,
    pub(crate) module: Arc<Module>,
    pub(crate) layout: Layout,
    pub(crate) signals: Arc<Signals>,
    pub(crate) processing: Arc<AtomicBool>,
    /// Each stepped parameter's id and step count, sorted by id.
    pub(crate) steps: Vec<(u32, i32)>,
    pub(crate) edits: rtrb::Consumer<Edit>,
    pub(crate) values: rtrb::Producer<Value>,
    pub(crate) alive: Arc<()>,
    /// The parameters the plugin's MIDI mapping names for the bend and the wheel.
    pub(crate) controls: Arc<Controls>,
}

/// One direction's audio buses: per bus its channels' frames, the channels' pointers the
/// plugin reads or writes, and the bus records pointing at them.
struct Buses {
    channels: Vec<Vec<Vec<f32>>>,
    /// What the records point at: kept, never read here.
    _pointers: Vec<Vec<*mut f32>>,
    records: Vec<AudioBusBuffers>,
}

impl Buses {
    fn new(counts: &[u32], frames: usize) -> Self {
        let mut channels: Vec<Vec<Vec<f32>>> = counts
            .iter()
            .map(|&count| (0..count).map(|_| vec![0.0; frames]).collect())
            .collect();
        let mut pointers: Vec<Vec<*mut f32>> = channels
            .iter_mut()
            .map(|bus| bus.iter_mut().map(|channel| channel.as_mut_ptr()).collect())
            .collect();
        let records = pointers
            .iter_mut()
            .map(|bus| AudioBusBuffers {
                numChannels: bus.len() as i32,
                silenceFlags: 0,
                __field0: AudioBusBuffers__type0 {
                    channelBuffers32: bus.as_mut_ptr(),
                },
            })
            .collect();
        Self {
            channels,
            _pointers: pointers,
            records,
        }
    }

    fn silence(&mut self, len: usize) {
        for channel in self.channels.iter_mut().flatten() {
            channel[..len].fill(0.0);
        }
    }

    /// The records for a `ProcessData`: none (null) when there are no buses.
    fn as_data(&mut self) -> (i32, *mut AudioBusBuffers) {
        if self.records.is_empty() {
            (0, std::ptr::null_mut())
        } else {
            (self.records.len() as i32, self.records.as_mut_ptr())
        }
    }
}

pub(crate) struct Vst3Processor {
    audio: ComPtr<IAudioProcessor>,
    role: Role,
    signals: Arc<Signals>,
    processing: Arc<AtomicBool>,
    inputs: Buses,
    outputs: Buses,
    max_frames: usize,
    events: ComWrapper<EventList>,
    sent_back: ComWrapper<EventList>,
    changes: ComWrapper<ParameterChanges>,
    changed: ComWrapper<ParameterChanges>,
    context: Vst3Context,
    notes: NoteSchedule,
    waiting: Vec<(u32, f64)>,
    steps: Vec<(u32, i32)>,
    edits: rtrb::Consumer<Edit>,
    values: rtrb::Producer<Value>,
    controls: Arc<Controls>,
    /// Holds the module loaded while the processor lives, wherever it goes.
    _module: Arc<Module>,
    /// Shared with the host, which lets the instance go once this is the last holder: declared
    /// last, so it drops after the processor above.
    _alive: Arc<()>,
}

// SAFETY: the raw pointers are into the buffers this struct owns (heap buffers that never move
// or grow after `new`), used only by the thread that holds the struct; the COM objects it holds
// are its own (the event list and parameter changes) or the plugin's audio processor, which the
// format lets the audio thread call.
unsafe impl Send for Vst3Processor {}

impl Vst3Processor {
    /// The audio half of an activated plugin, its buffers sized for `making.layout`. Runs on the
    /// main thread.
    pub(crate) fn new(making: Making) -> Self {
        let Making {
            audio,
            module,
            layout,
            signals,
            processing,
            steps,
            edits,
            values,
            alive,
            controls,
        } = making;
        let max_frames = dawsome_engine::realtime::BLOCK_FRAMES;
        Self {
            audio,
            role: layout.role,
            signals,
            processing,
            inputs: Buses::new(&layout.inputs, max_frames),
            outputs: Buses::new(&layout.outputs, max_frames),
            max_frames,
            events: EventList::new(EVENTS),
            sent_back: EventList::new(0),
            changes: ParameterChanges::new(),
            changed: ParameterChanges::new(),
            // SAFETY: the context is plain data (numbers and two plain structs), for which all
            // zero bytes are a value; every block fills it before `process`.
            context: unsafe { std::mem::zeroed() },
            notes: NoteSchedule::new(layout.takes_notes),
            waiting: Vec::with_capacity(WAITING_PARAMS),
            steps,
            edits,
            values,
            controls,
            _module: module,
            _alive: alive,
        }
    }

    /// `setProcessing(true)`, on the first call that needs it.
    fn start(&mut self) {
        if !self.processing.swap(true, Ordering::AcqRel) {
            // SAFETY: the processor is active (set up and activated on the main thread);
            // setProcessing may be called from the audio thread.
            unsafe { self.audio.setProcessing(ON) };
        }
    }

    /// The main input bus's channels from `frames`: both sides, or their average for a mono bus
    /// (so a centered sound keeps its level). Every other input is silence.
    fn fill_inputs(&mut self, frames: &[[f32; 2]]) {
        self.inputs.silence(frames.len());
        if self.role != Role::Effect {
            return;
        }
        match self.inputs.channels.first_mut().map(Vec::as_mut_slice) {
            Some([mono]) => {
                for (sample, frame) in mono.iter_mut().zip(frames) {
                    *sample = 0.5 * (frame[0] + frame[1]);
                }
            }
            Some([left, right, ..]) => {
                for ((l, r), frame) in left.iter_mut().zip(right.iter_mut()).zip(frames) {
                    (*l, *r) = (frame[0], frame[1]);
                }
            }
            _ => {}
        }
    }

    /// The main output bus's channels into `frames`: both sides, or the one channel twice.
    fn write_outputs(&self, frames: &mut [[f32; 2]]) {
        match self.outputs.channels.first().map(Vec::as_slice) {
            Some([mono]) => {
                for (frame, &sample) in frames.iter_mut().zip(mono) {
                    *frame = [sample, sample];
                }
            }
            Some([left, right, ..]) => {
                for ((frame, &l), &r) in frames.iter_mut().zip(left).zip(right) {
                    *frame = [l, r];
                }
            }
            _ => {}
        }
    }

    /// The block's note events, in the schedule's order.
    fn queue_notes(&mut self, events: &[PortEvent], clock: u64, len: usize) {
        self.events.clear();
        let list = &self.events;
        self.notes
            .block(events, clock, len, &self.signals.dropped_notes, |event| {
                // The room covers every event a block can make; this only guards against a bug.
                let _ = list.push(note_event(event));
            });
    }

    /// The values to send at the block's start: the waiting ones (each also on its way back to
    /// the main thread, for the controller) and the controller's own edits.
    fn queue_values(&mut self) {
        self.changes.clear();
        for index in 0..self.waiting.len() {
            let (id, value) = self.waiting[index];
            let value = self.normalized(id, value);
            if self.changes.set_at_start(id, value) {
                self.send_back(Value {
                    id,
                    value,
                    from_plugin: false,
                });
            }
        }
        self.waiting.clear();
        while let Ok((id, value)) = self.edits.pop() {
            self.changes.set_at_start(id, value);
        }
    }

    /// A value as the engine gives it (0..1, or 0..n for a parameter of n steps) normalized.
    fn normalized(&self, id: u32, value: f64) -> f64 {
        let steps = self
            .steps
            .binary_search_by_key(&id, |&(known, _)| known)
            .map_or(0, |at| self.steps[at].1);
        normalized(value, steps)
    }

    /// A device's pitch bend and mod wheel in `events`, as points of the parameters the plugin's
    /// MIDI mapping names for them, each at its control's frame (a later one on the same frame
    /// replacing the earlier, as the last MIDI message wins in CLAP), each also on its way back to
    /// the main thread, so the controller follows (a value the host sent: no changed state). A
    /// plugin whose mapping names none is sent neither, as a CLAP plugin whose note port takes
    /// no MIDI is not.
    fn queue_controls(&mut self, events: &[PortEvent]) {
        let bend = self.controls.bend.load(Ordering::Acquire);
        let wheel = self.controls.wheel.load(Ordering::Acquire);
        for &event in events {
            let (id, frame, value) = match event {
                PortEvent::Bend { frame, value } => (bend, frame, bend_point(value)),
                PortEvent::Wheel { frame, value } => (wheel, frame, wheel_point(value)),
                PortEvent::Note { .. } | PortEvent::NoteOn { .. } | PortEvent::NoteOff { .. } => {
                    continue
                }
            };
            if id == NO_PARAM {
                continue;
            }
            if self.changes.set_at(id, frame, value) {
                self.send_back(Value {
                    id,
                    value,
                    from_plugin: false,
                });
            }
        }
    }

    fn send_back(&mut self, value: Value) {
        if self.values.push(value).is_err() {
            self.signals.dropped_values.fetch_add(1, Ordering::Relaxed);
        }
        self.signals.ring();
    }

    /// The values the plugin changed in the block, on their way to the main thread, each held
    /// inside 0..1; a NaN is not sent, so the controller keeps the value it had. Why: the
    /// format's values are normalized, but a plugin's output change is whatever it wrote, and
    /// its controller and the engine's `value` took it as it was (review 2026-10-26: -0.5, 1.5
    /// and NaN all came back from `value`).
    fn take_changes(&mut self) {
        for index in 0.. {
            let Some((id, value)) = self.changed.last_values().nth(index) else {
                break;
            };
            if value.is_nan() {
                continue;
            }
            self.send_back(Value {
                id,
                value: value.clamp(0.0, 1.0),
                from_plugin: true,
            });
        }
    }

    /// The transport for a block: tempo, playing, the song position in quarter notes and in
    /// samples (at the tempo in force), the bar it is in, 4/4, and the output clock.
    fn fill_context(&mut self, context: &ProcessContext<'_>) {
        let transport = context.transport;
        let beats = transport.song_tick.0 as f64 / f64::from(context.ppq);
        let mut state = TEMPO_VALID | MUSIC_VALID | BAR_VALID | SIGNATURE_VALID | CONTINUOUS_VALID;
        if transport.playing {
            state |= PLAYING;
        }
        let at = tick_to_sample(
            transport.song_tick,
            context.sample_rate,
            context.tempo,
            context.ppq,
        )
        .unwrap_or(0);
        let ctx = &mut self.context;
        ctx.state = state;
        ctx.sampleRate = f64::from(context.sample_rate);
        ctx.projectTimeSamples = i64::try_from(at).unwrap_or(i64::MAX);
        ctx.continousTimeSamples = i64::try_from(transport.steady_time).unwrap_or(i64::MAX);
        ctx.projectTimeMusic = beats;
        ctx.barPositionMusic = (beats / 4.0).floor() * 4.0;
        ctx.tempo = f64::from(context.tempo.0) / 1000.0;
        ctx.timeSigNumerator = 4;
        ctx.timeSigDenominator = 4;
    }

    /// One `process` call over `len` frames (0: a flush of values alone, with no buses); false
    /// when the plugin failed it.
    fn run(&mut self, len: usize, with_audio: bool) -> bool {
        self.changed.clear();
        self.sent_back.clear();
        let ((inputs, input_buses), (outputs, output_buses)) = if with_audio {
            (self.inputs.as_data(), self.outputs.as_data())
        } else {
            ((0, std::ptr::null_mut()), (0, std::ptr::null_mut()))
        };
        let as_list = |list: &ComWrapper<EventList>| {
            list.as_com_ref::<IEventList>()
                .map_or(std::ptr::null_mut(), |list| list.as_ptr())
        };
        let as_changes = |changes: &ComWrapper<ParameterChanges>| {
            changes
                .as_com_ref::<IParameterChanges>()
                .map_or(std::ptr::null_mut(), |changes| changes.as_ptr())
        };
        let mut data = ProcessData {
            processMode: REALTIME,
            symbolicSampleSize: SAMPLE_32,
            numSamples: len as i32,
            numInputs: inputs,
            numOutputs: outputs,
            inputs: input_buses,
            outputs: output_buses,
            inputParameterChanges: as_changes(&self.changes),
            outputParameterChanges: as_changes(&self.changed),
            inputEvents: as_list(&self.events),
            outputEvents: as_list(&self.sent_back),
            processContext: &mut self.context,
        };
        // SAFETY: the processor is active and processing; every pointer in `data` is into this
        // struct's own buffers and objects, sized for `max_frames` (≥ `len`) and live for the
        // call (the format's `ProcessData` contract).
        unsafe { self.audio.process(&mut data) == kResultOk }
    }
}

/// A value as the engine gives it, normalized for the format: a parameter of `steps` steps
/// (0..n in the engine) the nearest whole step, half away from zero, held to 0..n, over n; any
/// other held inside 0..1; NaN as 0. Why the whole step: the format decodes `min(n, floor(v·(n +
/// 1)))`, so 2.4 of 4 sent as 0.6 is heard as step 3 where the panel, a held point or a link
/// meant 2.
pub(crate) fn normalized(value: f64, steps: i32) -> f64 {
    if value.is_nan() {
        return 0.0;
    }
    if steps > 0 {
        let steps = f64::from(steps);
        value.round().clamp(0.0, steps) / steps
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// A note event of the schedule as a VST3 event: on bus 0, MIDI channel 0, no tuning, the
/// note's id, live notes flagged.
fn note_event(event: NoteEvent) -> Event {
    let (frame, live, kind, body) = match event {
        NoteEvent::On {
            frame,
            key,
            velocity,
            id,
            live,
        } => (
            frame,
            live,
            NOTE_ON,
            Event__type0 {
                noteOn: NoteOnEvent {
                    channel: 0,
                    pitch: i16::from(key),
                    tuning: 0.0,
                    velocity,
                    length: 0,
                    noteId: id,
                },
            },
        ),
        NoteEvent::Off {
            frame,
            key,
            id,
            live,
        } => (
            frame,
            live,
            NOTE_OFF,
            Event__type0 {
                noteOff: NoteOffEvent {
                    channel: 0,
                    pitch: i16::from(key),
                    velocity: 0.0,
                    noteId: id,
                    tuning: 0.0,
                },
            },
        ),
    };
    Event {
        busIndex: 0,
        sampleOffset: i32::try_from(frame).unwrap_or(i32::MAX),
        ppqPosition: 0.0,
        flags: if live { LIVE } else { 0 },
        r#type: kind,
        __field0: body,
    }
}

impl Processor for Vst3Processor {
    fn process(&mut self, context: &ProcessContext<'_>, buffer: &mut [[f32; 2]]) {
        let len = buffer.len().min(self.max_frames);
        let frames = &mut buffer[..len];
        self.start();
        self.fill_inputs(frames);
        self.queue_notes(context.events, context.transport.steady_time, len);
        self.queue_values();
        self.queue_controls(context.events);
        self.fill_context(context);
        self.outputs.silence(len);
        if self.run(len, true) {
            self.write_outputs(frames);
            self.take_changes();
        } else {
            self.signals.raise(&self.signals.failed);
            // An instrument that failed is silent; an effect that failed passes its input on.
            if self.role == Role::Instrument {
                frames.fill([0.0; 2]);
            }
        }
    }

    fn set_param(&mut self, _index: usize, _value: f32) {}

    // Play, Stop and Seek: every sounding note ends on frame 0 of the next call (the output
    // clock starts again, so their own ends are of the old one), and processing stops and
    // starts again, which is where a VST3 plugin resets its processing state. Why both: the
    // format has no reset call, and not every plugin clears its voices on the toggle (plan
    // risk 18); the note-offs end them in one that does not.
    fn reset(&mut self) {
        self.notes.end_all();
        if self.processing.load(Ordering::Acquire) {
            // SAFETY: as in `start`.
            unsafe { self.audio.setProcessing(OFF) };
            // SAFETY: as in `start`.
            unsafe { self.audio.setProcessing(ON) };
        }
    }

    fn queue_param(&mut self, id: u32, value: f32) {
        let value = f64::from(value);
        if let Some(waiting) = self.waiting.iter_mut().find(|(known, _)| *known == id) {
            waiting.1 = value;
        } else if self.waiting.len() < WAITING_PARAMS {
            self.waiting.push((id, value));
        }
    }

    // While stopped: values that wait (the engine's or the controller's) reach the plugin
    // through a `process` call with no buses and no frames, the format's flush (portal FAQ).
    fn idle(&mut self, context: &ProcessContext<'_>) {
        if self.waiting.is_empty() && self.edits.is_empty() {
            return;
        }
        self.start();
        self.queue_values();
        self.events.clear();
        self.fill_context(context);
        if self.run(0, false) {
            self.take_changes();
        } else {
            self.signals.raise(&self.signals.failed);
        }
    }

    fn suspend(&mut self) {
        if self.processing.swap(false, Ordering::AcqRel) {
            // SAFETY: as in `start`; the last call on this thread.
            unsafe { self.audio.setProcessing(OFF) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // By hand: a parameter of 4 steps hears the nearest whole step over 4, half away from zero,
    // held to 0..4 (2.4 → 2/4, 2.5 → 3/4, 7 → 4/4, -1 → 0); NaN is 0 for either kind; a
    // continuous one is held inside 0..1 and otherwise passed as it is.
    #[test]
    fn a_stepped_value_is_sent_as_a_whole_step() {
        for (value, want) in [
            (2.4, 0.5),
            (2.5, 0.75),
            (2.6, 0.75),
            (7.0, 1.0),
            (-1.0, 0.0),
            (-0.5, 0.0),
            (f64::NAN, 0.0),
        ] {
            assert_eq!(normalized(value, 4), want, "{value}");
        }
        assert_eq!(normalized(0.3, 0), 0.3);
        assert_eq!(normalized(1.5, 0), 1.0);
        assert_eq!(normalized(f64::NAN, 0), 0.0);
    }
}
