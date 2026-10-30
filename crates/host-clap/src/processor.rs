//! The audio half of a CLAP plugin: what the player calls on the audio thread. Everything it
//! touches there (port buffers, the event queue, the tables of waiting parameters and sounding
//! notes) is sized when the plugin is created on the main thread, so a block allocates nothing.
//!
//! Per block it sends the plugin, in time order: the parameter values that waited (at frame 0),
//! the performance controls a device played (pitch bend, the mod wheel, at frame 0, as MIDI
//! messages to a
//! plugin whose note port takes MIDI; none to one that takes CLAP notes only), the note-offs of
//! notes whose length runs out in the block, and the note-ons the engine sent. Within one frame
//! that is the order too: parameters, controls, note-offs, note-ons, so a note that ends where
//! another starts on the same key does not cut the new one, and a note struck with a bend starts
//! bent. Which notes sound and when each ends is the [`NoteSchedule`] every format's host
//! shares; this file turns its events into CLAP's.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use clack_extensions::params::PluginParams;
use clack_host::events::event_types::{
    MidiEvent, NoteOffEvent, NoteOnEvent, ParamValueEvent, TransportEvent, TransportFlags,
};
use clack_host::events::io::{InputEventBuffer, OutputEventBuffer, TryPushError};
use clack_host::events::spaces::CoreEventSpace;
use clack_host::events::{EventFlags, EventHeader, Pckn, UnknownEvent};
use clack_host::prelude::{
    AudioPortBuffer, AudioPortBufferType, AudioPorts, InputChannel, InputEvents, OutputEvents,
    PluginAudioProcessor,
};
use clack_host::utils::{BeatTime, ClapId, SecondsTime};
use dawsome_engine::processor::{PortEvent, ProcessContext, Processor};
use dawsome_host_common::notes::{NoteEvent, NoteSchedule, EVENTS_PER_BLOCK};

use crate::handlers::{DawsomeHost, Signals, IN_AUDIO};

/// Parameter values that can wait for one block; more are dropped (a newer value for a waiting
/// parameter replaces it, so this is room for 64 different parameters).
pub(crate) const WAITING_PARAMS: usize = 64;

/// Performance controls one block can send; more are dropped.
pub(crate) const CONTROLS_PER_BLOCK: usize = 256;

/// Events one block can send: every waiting parameter, the controls, and the note events
/// (`EVENTS_PER_BLOCK`: every note-off and every note-on).
const QUEUE_CAPACITY: usize = WAITING_PARAMS + CONTROLS_PER_BLOCK + EVENTS_PER_BLOCK;

/// What the plugin plays for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// A plugin channel: no audio in, notes in, audio out.
    Instrument,
    /// A mixer slot: the insert's audio in and out, no notes.
    Effect,
}

/// How the plugin takes notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dialect {
    Clap,
    /// MIDI note-on and note-off messages.
    Midi,
    /// It has no note input.
    None,
}

/// One event the block sends the plugin.
#[derive(Debug, Clone, Copy)]
enum Queued {
    Param(ParamValueEvent),
    NoteOn(NoteOnEvent),
    NoteOff(NoteOffEvent),
    Midi(MidiEvent),
}

/// The events a block sends, in time order, kept in place between blocks.
struct Queue {
    events: Vec<Queued>,
}

impl Queue {
    fn push(&mut self, event: Queued) {
        // The capacity covers every event a block can make; this only guards against a bug.
        if self.events.len() < QUEUE_CAPACITY {
            self.events.push(event);
        }
    }
}

impl InputEventBuffer for Queue {
    fn len(&self) -> u32 {
        // At most QUEUE_CAPACITY, far below u32::MAX.
        u32::try_from(self.events.len()).unwrap_or(u32::MAX)
    }

    fn get(&self, index: u32) -> Option<&UnknownEvent> {
        Some(match self.events.get(usize::try_from(index).ok()?)? {
            Queued::Param(event) => event.as_ref(),
            Queued::NoteOn(event) => event.as_ref(),
            Queued::NoteOff(event) => event.as_ref(),
            Queued::Midi(event) => event.as_ref(),
        })
    }
}

/// What the plugin sends back, reduced to flags as it arrives: nothing is stored, so nothing
/// can overflow or allocate. A parameter it changes itself, outside a gesture, or the end of a
/// gesture, marks its state dirty for the main thread to capture.
struct Sink {
    signals: Arc<Signals>,
}

impl OutputEventBuffer for Sink {
    fn try_push(&mut self, event: &UnknownEvent) -> Result<(), TryPushError> {
        let signals = &self.signals;
        match event.as_core_event() {
            Some(CoreEventSpace::ParamGestureBegin(_)) => {
                signals.gestures.fetch_add(1, Ordering::AcqRel);
            }
            Some(CoreEventSpace::ParamGestureEnd(_)) => {
                signals.gestures.fetch_sub(1, Ordering::AcqRel);
                signals.raise(&signals.dirty);
            }
            Some(CoreEventSpace::ParamValue(_))
                if signals.gestures.load(Ordering::Acquire) <= 0 =>
            {
                signals.raise(&signals.dirty);
            }
            _ => {}
        }
        Ok(())
    }
}

/// Marks the thread as the audio thread while a plugin is being called there, for the
/// thread-check extension.
struct AudioThread;

impl AudioThread {
    fn enter() -> Self {
        IN_AUDIO.with(|flag| flag.set(true));
        Self
    }
}

impl Drop for AudioThread {
    fn drop(&mut self) {
        IN_AUDIO.with(|flag| flag.set(false));
    }
}

/// The port layout and the extensions the audio half uses, read on the main thread.
pub(crate) struct Layout {
    pub(crate) role: Role,
    pub(crate) dialect: Dialect,
    /// Whether its note port takes MIDI messages (whatever dialect its notes go in): the
    /// performance controls go to it as MIDI.
    pub(crate) takes_midi: bool,
    /// Channels of each input port, then of each output port; port 0 is the main one.
    pub(crate) inputs: Vec<u32>,
    pub(crate) outputs: Vec<u32>,
    pub(crate) max_frames: usize,
    pub(crate) params: Option<PluginParams>,
}

/// A CLAP plugin as the player runs it.
pub(crate) struct ClapProcessor {
    audio: PluginAudioProcessor<DawsomeHost>,
    role: Role,
    dialect: Dialect,
    takes_midi: bool,
    params: Option<PluginParams>,
    signals: Arc<Signals>,
    input_ports: AudioPorts,
    output_ports: AudioPorts,
    /// Port, channel, frame.
    inputs: Vec<Vec<Vec<f32>>>,
    outputs: Vec<Vec<Vec<f32>>>,
    max_frames: usize,
    queue: Queue,
    sink: Sink,
    waiting: Vec<(ClapId, f64)>,
    notes: NoteSchedule,
    /// Shared with the host, which lets the plugin instance go once this is the last holder:
    /// declared last, so it drops after the audio processor above.
    _alive: Arc<()>,
}

impl ClapProcessor {
    /// The audio half of an activated plugin, its buffers sized for `layout`. Runs on the main
    /// thread.
    pub(crate) fn new(
        audio: PluginAudioProcessor<DawsomeHost>,
        layout: Layout,
        signals: Arc<Signals>,
        alive: Arc<()>,
    ) -> Self {
        let buffers = |ports: &[u32]| -> Vec<Vec<Vec<f32>>> {
            ports
                .iter()
                .map(|&channels| {
                    (0..channels)
                        .map(|_| vec![0.0; layout.max_frames])
                        .collect()
                })
                .collect()
        };
        let channels = |ports: &[u32]| ports.iter().map(|&count| count as usize).sum();
        Self {
            audio,
            role: layout.role,
            dialect: layout.dialect,
            takes_midi: layout.takes_midi,
            params: layout.params,
            input_ports: AudioPorts::with_capacity(channels(&layout.inputs), layout.inputs.len()),
            output_ports: AudioPorts::with_capacity(
                channels(&layout.outputs),
                layout.outputs.len(),
            ),
            inputs: buffers(&layout.inputs),
            outputs: buffers(&layout.outputs),
            max_frames: layout.max_frames,
            queue: Queue {
                events: Vec::with_capacity(QUEUE_CAPACITY),
            },
            sink: Sink {
                signals: Arc::clone(&signals),
            },
            signals,
            waiting: Vec::with_capacity(WAITING_PARAMS),
            notes: NoteSchedule::new(layout.dialect != Dialect::None),
            _alive: alive,
        }
    }

    /// The main input port's channels from `frames`: both sides, or their average for a mono
    /// port (so a centered sound keeps its level). Every other input is silence.
    fn fill_inputs(&mut self, frames: &[[f32; 2]]) {
        let len = frames.len();
        for channel in self.inputs.iter_mut().flatten() {
            channel[..len].fill(0.0);
        }
        if self.role != Role::Effect {
            return;
        }
        match self.inputs.first_mut().map(Vec::as_mut_slice) {
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

    /// Fills the queue for a block of `len` frames starting at output-clock frame `clock`.
    fn queue_events(&mut self, events: &[PortEvent], clock: u64, len: usize) {
        self.queue.events.clear();
        for &(id, value) in &self.waiting {
            let event = ParamValueEvent::new(0, id, Pckn::match_all(), value);
            self.queue.push(Queued::Param(event));
        }
        self.waiting.clear();
        // The controls first: the engine sends them at frame 0, before any note of the block.
        let mut controls = 0;
        for &event in events {
            if let Some(bytes) = control_message(event) {
                if self.takes_midi && controls < CONTROLS_PER_BLOCK {
                    controls += 1;
                    self.queue
                        .push(Queued::Midi(MidiEvent::new(event_frame(event), 0, bytes)));
                }
            }
        }
        let (queue, dialect) = (&mut self.queue, self.dialect);
        self.notes
            .block(events, clock, len, &self.signals.dropped_notes, |event| {
                queue.push(queued(event, dialect))
            });
    }

    /// The main output port's channels into `frames`: both sides, or the one channel twice.
    fn write_outputs(&self, frames: &mut [[f32; 2]]) {
        match self.outputs.first().map(Vec::as_slice) {
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

    /// One `process` call over `len` frames; `false` when the plugin failed it.
    fn run(&mut self, len: usize, clock: u64, transport: &TransportEvent) -> bool {
        let Ok(started) = self.audio.ensure_processing_started() else {
            return false;
        };
        let inputs = self
            .input_ports
            .with_input_buffers(self.inputs.iter_mut().map(|port| {
                AudioPortBuffer {
                    latency: 0,
                    channels: AudioPortBufferType::f32_input_only(
                        port.iter_mut()
                            .map(|channel| InputChannel::variable(&mut channel[..len])),
                    ),
                }
            }));
        let mut outputs = self
            .output_ports
            .with_output_buffers(self.outputs.iter_mut().map(|port| AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_output_only(
                    port.iter_mut().map(|channel| &mut channel[..len]),
                ),
            }));
        let events = InputEvents::from_buffer(&self.queue);
        let mut sent = OutputEvents::from_buffer(&mut self.sink);
        started
            .process(
                &inputs,
                &mut outputs,
                &events,
                &mut sent,
                Some(clock),
                Some(transport),
            )
            .is_ok()
    }
}

/// The MIDI message a performance control is sent to a plugin as, on MIDI channel 1: pitch bend
/// En ll mm (the 14-bit value + 8192, its low 7 bits first), the mod wheel Bn 01 vv; `None` for a
/// note.
fn control_message(event: PortEvent) -> Option<[u8; 3]> {
    match event {
        PortEvent::Bend { value, .. } => {
            let raw = (i32::from(value) + 8192).clamp(0, 0x3FFF) as u16;
            Some([0xE0, (raw & 0x7F) as u8, (raw >> 7) as u8])
        }
        PortEvent::Wheel { value, .. } => Some([0xB0, 1, value & 0x7F]),
        PortEvent::Note { .. } | PortEvent::NoteOn { .. } | PortEvent::NoteOff { .. } => None,
    }
}

/// The frame of the block an event falls on.
fn event_frame(event: PortEvent) -> u32 {
    match event {
        PortEvent::Note { frame, .. }
        | PortEvent::NoteOn { frame, .. }
        | PortEvent::NoteOff { frame, .. }
        | PortEvent::Bend { frame, .. }
        | PortEvent::Wheel { frame, .. } => frame,
    }
}

fn pckn(key: u8, id: i32) -> Pckn {
    Pckn::new(0u16, 0u16, u16::from(key), u32::try_from(id).unwrap_or(0))
}

/// A note event of the schedule as the plugin takes it: a CLAP note event, or a MIDI message.
fn queued(event: NoteEvent, dialect: Dialect) -> Queued {
    match (event, dialect) {
        (
            NoteEvent::On {
                frame,
                key,
                velocity,
                ..
            },
            Dialect::Midi,
        ) => {
            // MIDI velocities are 1..=127; 0 would be read as a note-off.
            let velocity = (velocity * 127.0).round().clamp(1.0, 127.0) as u8;
            Queued::Midi(MidiEvent::new(frame, 0, [0x90, key, velocity]))
        }
        (NoteEvent::Off { frame, key, .. }, Dialect::Midi) => {
            Queued::Midi(MidiEvent::new(frame, 0, [0x80, key, 0]))
        }
        (
            NoteEvent::On {
                frame,
                key,
                velocity,
                id,
                ..
            },
            Dialect::Clap | Dialect::None,
        ) => Queued::NoteOn(NoteOnEvent::new(frame, pckn(key, id), f64::from(velocity))),
        (NoteEvent::Off { frame, key, id, .. }, Dialect::Clap | Dialect::None) => {
            Queued::NoteOff(NoteOffEvent::new(frame, pckn(key, id), 0.0))
        }
    }
}

/// The CLAP transport for a block: tempo, playing, the song position in beats and seconds (at
/// the tempo in force), 4/4.
fn transport_event(context: &ProcessContext<'_>) -> TransportEvent {
    let bpm = f64::from(context.tempo.0) / 1000.0;
    let beats = context.transport.song_tick.0 as f64 / f64::from(context.ppq);
    let bar = (beats / 4.0).floor();
    let mut flags = TransportFlags::HAS_TEMPO
        | TransportFlags::HAS_BEATS_TIMELINE
        | TransportFlags::HAS_SECONDS_TIMELINE
        | TransportFlags::HAS_TIME_SIGNATURE;
    if context.transport.playing {
        flags |= TransportFlags::IS_PLAYING;
    }
    TransportEvent {
        header: EventHeader::new_core(0, EventFlags::empty()),
        flags,
        song_pos_beats: BeatTime::from_float(beats),
        song_pos_seconds: SecondsTime::from_float(beats * 60.0 / bpm),
        tempo: bpm,
        tempo_inc: 0.0,
        loop_start_beats: BeatTime::from_int(0),
        loop_end_beats: BeatTime::from_int(0),
        loop_start_seconds: SecondsTime::from_int(0),
        loop_end_seconds: SecondsTime::from_int(0),
        bar_start: BeatTime::from_float(bar * 4.0),
        bar_number: bar as i32,
        time_signature_numerator: 4,
        time_signature_denominator: 4,
    }
}

impl Processor for ClapProcessor {
    fn process(&mut self, context: &ProcessContext<'_>, buffer: &mut [[f32; 2]]) {
        let _audio = AudioThread::enter();
        let len = buffer.len().min(self.max_frames);
        let frames = &mut buffer[..len];
        let clock = context.transport.steady_time;
        self.fill_inputs(frames);
        self.queue_events(context.events, clock, len);
        for channel in self.outputs.iter_mut().flatten() {
            channel[..len].fill(0.0);
        }
        let transport = transport_event(context);
        if self.run(len, clock, &transport) {
            self.write_outputs(frames);
        } else {
            self.signals.raise(&self.signals.failed);
            // An instrument that failed is silent; an effect that failed passes its input on.
            if self.role == Role::Instrument {
                frames.fill([0.0; 2]);
            }
        }
    }

    fn set_param(&mut self, _index: usize, _value: f32) {}

    fn reset(&mut self) {
        let _audio = AudioThread::enter();
        self.notes.clear();
        self.audio.reset();
    }

    fn queue_param(&mut self, id: u32, value: f32) {
        let Some(id) = ClapId::from_raw(id) else {
            return;
        };
        let value = f64::from(value);
        if let Some(waiting) = self.waiting.iter_mut().find(|(known, _)| *known == id) {
            waiting.1 = value;
        } else if self.waiting.len() < WAITING_PARAMS {
            self.waiting.push((id, value));
        }
    }

    fn idle(&mut self, _context: &ProcessContext<'_>) {
        let flush = Signals::take(&self.signals.flush);
        if self.waiting.is_empty() && !flush {
            return;
        }
        let _audio = AudioThread::enter();
        self.queue.events.clear();
        for &(id, value) in &self.waiting {
            let event = ParamValueEvent::new(0, id, Pckn::match_all(), value);
            self.queue.push(Queued::Param(event));
        }
        self.waiting.clear();
        if let Some(params) = &self.params {
            let mut handle = self.audio.plugin_handle();
            params.flush_active(
                &mut handle,
                &InputEvents::from_buffer(&self.queue),
                &mut OutputEvents::from_buffer(&mut self.sink),
            );
        }
    }

    fn suspend(&mut self) {
        let _audio = AudioThread::enter();
        self.audio.ensure_processing_stopped();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue() -> Queue {
        Queue {
            events: Vec::with_capacity(QUEUE_CAPACITY),
        }
    }

    /// The queue as (frame, kind, key) rows.
    fn rows(queue: &Queue) -> Vec<(u32, &'static str, i16)> {
        (0..queue.len())
            .filter_map(|index| queue.get(index))
            .map(|event| {
                let time = event.header().time();
                match event.as_core_event() {
                    Some(CoreEventSpace::NoteOn(on)) => (time, "on", on.pckn().raw_key()),
                    Some(CoreEventSpace::NoteOff(off)) => (time, "off", off.pckn().raw_key()),
                    Some(CoreEventSpace::ParamValue(_)) => (time, "param", -1),
                    Some(CoreEventSpace::Midi(midi)) => {
                        let data = midi.data();
                        let kind = if data[0] == 0x90 {
                            "midi on"
                        } else {
                            "midi off"
                        };
                        (time, kind, i16::from(data[1]))
                    }
                    _ => (time, "other", -1),
                }
            })
            .collect()
    }

    // By hand from MIDI 1.0: a bend goes to a plugin as En ll mm on MIDI channel 1, the 14-bit
    // value + 8192 with its low 7 bits first; a note is no control.
    #[test]
    fn a_bend_is_sent_as_its_midi_bytes() {
        let bend = |value| control_message(PortEvent::Bend { frame: 3, value });
        assert_eq!(bend(8191), Some([0xE0, 0x7F, 0x7F]));
        assert_eq!(bend(-8192), Some([0xE0, 0x00, 0x00]));
        assert_eq!(bend(0), Some([0xE0, 0x00, 0x40]));
        let wheel = PortEvent::Wheel {
            frame: 0,
            value: 100,
        };
        assert_eq!(control_message(wheel), Some([0xB0, 0x01, 100]));
        let note = PortEvent::NoteOff { frame: 3, key: 60 };
        assert_eq!(control_message(note), None);
        assert_eq!(event_frame(PortEvent::Bend { frame: 3, value: 0 }), 3);
    }

    // The schedule's events in the plugin's dialect: CLAP note events with their keys, or MIDI
    // note on and off messages (its frame order is the schedule's own, `dawsome_host_common`'s
    // test of it).
    #[test]
    fn schedule_events_become_clap_notes_or_midi_messages() {
        let on = |frame, key, id| NoteEvent::On {
            frame,
            key,
            velocity: 1.0,
            id,
            live: false,
        };
        let off = |frame, key, id| NoteEvent::Off {
            frame,
            key,
            id,
            live: false,
        };
        let events = [
            on(0, 64, 2),
            off(10, 60, 0),
            on(10, 60, 3),
            off(100, 62, 1),
            on(200, 65, 4),
        ];
        let mut clap = queue();
        for event in events {
            clap.push(queued(event, Dialect::Clap));
        }
        assert_eq!(
            rows(&clap),
            [
                (0, "on", 64),
                (10, "off", 60),
                (10, "on", 60),
                (100, "off", 62),
                (200, "on", 65)
            ]
        );
        let mut midi = queue();
        for event in &events[1..3] {
            midi.push(queued(*event, Dialect::Midi));
        }
        assert_eq!(rows(&midi), [(10, "midi off", 60), (10, "midi on", 60)]);
    }
}
