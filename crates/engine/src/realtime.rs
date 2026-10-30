//! The boundary between the UI thread and the audio thread.
//!
//! [`link`] makes its two halves:
//! - [`RealtimeLink`], the UI half: sends plan updates and [`Control`]s, reads the playhead and
//!   the meters, and drops what the audio thread has finished with.
//! - [`Player`], the audio half: [`Player::process`] renders audio. Live playback calls it from
//!   the audio callback; an offline render calls it in a loop. It never allocates, frees, locks
//!   or waits.
//!
//! Wait-free single-producer, single-consumer rings connect them: plan updates (UI to audio),
//! retired updates (audio to UI), controls (UI to audio), previews and their retirement, the
//! input channels' feeds and their retirement, one-off events (audio to UI), the record tap
//! (the master's frames while recording) and the MIDI-out messages ([`crate::midi_out`]). The
//! playhead, the peak meters and the automated values come back through atomics.

use std::collections::VecDeque;
use std::mem;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use crossbeam_queue::ArrayQueue;
use rtrb::{Consumer, Producer, PushError, RingBuffer};
use thiserror::Error;

use crate::dsp::laws::{channel_pan_gains, fader_gain};
use crate::input::InputFeeds;
use crate::midi_out::{self, Course, MidiOutEvent, MidiOutReader, MidiOutState, PEDAL_DOWN};
use crate::plan::{PlanUpdate, PlayPlan, Playing, ProcessorSlot, CONTROL_GRID};
use crate::processor::{ProcessContext, TransportInfo};
use crate::project::{
    AutomationId, ChannelId, ClipId, InsertId, FADER_RANGE, MAX_AUTOMATIONS, MAX_KEY, MAX_VOLUME,
    PAN_RANGE, SEND_RANGE,
};
use crate::render::{self, Controls, Preview, Runtime, BUSES, CHUNK};
use crate::source::{ClipSource, SourceSlot};
use crate::time::{self, MilliBpm, Ticks};
use crate::transport::{Control, ParamTarget, Transport, Wrap};

/// The most frames one block covers: every processor, plugins included, is called for at most
/// this many frames at a time, however long the buffer `Player::process` is given.
pub const BLOCK_FRAMES: usize = CHUNK;

/// Plan updates that can wait in the plan ring; the retire ring holds as many.
const PLAN_RING_CAPACITY: usize = 8;

/// Controls that can wait in the control ring.
const CONTROL_RING_CAPACITY: usize = 256;

/// Plan updates that can wait behind a full plan ring. Only a running player takes them, and
/// each holds a whole plan and its new processors, so without a bound a player that is alive but
/// not running (a stalled stream) would make every edit cost memory until it ran again.
const WAITING_LIMIT: usize = 64;

/// Previews that can wait for the player; it plays the newest.
const PREVIEW_RING_CAPACITY: usize = 4;

/// Previews the player has finished with that can wait to be dropped on the UI thread.
const PREVIEW_RETIRE_CAPACITY: usize = 8;

/// Sets of input feeds that can wait for the player; it takes the newest.
const INPUT_RING_CAPACITY: usize = 4;

/// Sets of input feeds the player has replaced that can wait to be dropped on the UI thread.
const INPUT_RETIRE_CAPACITY: usize = 8;

/// Events that can wait for the UI; more are dropped and counted.
const EVENT_RING_CAPACITY: usize = 256;

/// Seconds of master output the record tap holds before frames are dropped (and counted).
const TAP_SECONDS: usize = 4;

/// Song-mode frames are cut where the count-in clicks of a beat start; each lasts this many
/// frames, as the metronome's do.
const CLICK_FRAMES: u64 = 64;

/// Frame `into` of a click, a 64-frame burst that falls from its amplitude to silence, its
/// sign flipping every 8 frames: 0.25 (−12 dBFS) when `accented` (a bar's first beat), else
/// 0.125 (−18 dBFS); `None` past its end. Every value is a multiple of 1/512, exact in f32.
/// The count-in's and the metronome's clicks. Why 0.25 and not 10^(−12/20): a power of two
/// keeps every value exact (−12.04 dBFS), so its known answers hold to the bit.
fn click(into: u64, accented: bool) -> Option<f32> {
    if into >= CLICK_FRAMES {
        return None;
    }
    let amplitude = if accented { 0.25 } else { 0.125 };
    let sign = if (into / 8).is_multiple_of(2) {
        1.0
    } else {
        -1.0
    };
    Some(amplitude * (1.0 - into as f32 / CLICK_FRAMES as f32) * sign)
}

/// Adds a click's `value` to both sides of `frame` without taking a side past full scale
/// because of it: the sum is held to ±1, or to the side's own level where that is past ±1
/// already, so a click never clips the device output and never cuts the master's own sound.
/// Each frame on its own, so a render gives the same bits however it is split.
fn add_click(frame: &mut [f32; 2], value: f32) {
    *frame = frame.map(|side| {
        let limit = side.abs().max(1.0);
        (side + value).clamp(-limit, limit)
    });
}

/// Live notes that can wait in the inbox for the next call; more are dropped and counted.
const INBOX_CAPACITY: usize = 512;

/// Something that happened on the audio thread that the UI should hear about once.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EngineEvent {
    /// An audio clip's stream was not ready in time: that block of it played silence.
    StreamUnderrun { clip: ClipId },
    /// An audio clip's stream failed for good: the clip plays silence until it is loaded again.
    StreamFailed { clip: ClipId },
    /// A live note started on `channel`. `tick` is the playhead tick of the first sample of the
    /// call it sounded from while the song timeline runs, `None` while stopped or counting in:
    /// a note is recorded where it was heard. `frame` is the output clock's frame at that
    /// sample, which never jumps and runs while stopped too: what Capture times notes by.
    LiveNoteStarted {
        channel: ChannelId,
        key: u8,
        velocity: f32,
        tick: Option<Ticks>,
        frame: u64,
    },
    /// The live note of `key` ended on `channel`, the channel it started on; `tick` and `frame`
    /// as for its start.
    LiveNoteEnded {
        channel: ChannelId,
        key: u8,
        tick: Option<Ticks>,
        frame: u64,
    },
}

impl EngineEvent {
    fn is_live_note(&self) -> bool {
        matches!(
            self,
            EngineEvent::LiveNoteStarted { .. } | EngineEvent::LiveNoteEnded { .. }
        )
    }
}

/// A note from a MIDI device: a key pressed at a velocity from 0 to 1, or released.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LiveNote {
    On { key: u8, velocity: f32 },
    Off { key: u8 },
}

/// A performance control a device plays beside its notes, as the player takes it: from the
/// first frame of the call that takes it, until the next. Each input's controls are its own on
/// each channel it plays (one device's pedal never holds another's notes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveControl {
    /// The sustain pedal (CC 64), 0 to 127: from 64 it is down, and a key the input lets go of
    /// on the channel sounds on until the pedal lifts. Let go of, it lifts on every channel the
    /// input's pedal holds notes on, ending them in the order they were let go of. A value past
    /// 127 is ignored.
    Sustain(u8),
    /// The pitch bend wheel, −8192 (all the way down) to 8191 (all the way up), 0 at rest: the
    /// input's live notes on the channel read on from where they are at their speed bent by the
    /// channel's bend range ([`crate::project::Sound::bend_range`]), and the notes it starts
    /// there begin bent. A plugin is sent the bend (it bends by its own range). A value outside
    /// that range is ignored. A bend moved on one channel rests on every other the input bent:
    /// one channel holds a device's bend at a time, the one its notes play.
    Bend(i16),
    /// The mod wheel (CC 1), 0 to 127: on a channel whose low-pass filter is on it opens the
    /// filter of the input's live notes there by up to 4 octaves; on any other they swing a
    /// vibrato of up to 50 cents at 5 Hz. A plugin is sent it. A value past 127 is ignored. As
    /// the bend, moved on one channel it rests on every other.
    Wheel(u8),
}

/// Which control [`Player::rest_elsewhere`] rests.
#[derive(Debug, Clone, Copy)]
enum Moved {
    Bend,
    Wheel,
}

/// How many (input, channel) pairs can hold a control off its rest (the pedal not up, the bend
/// not centered, the wheel not at 0) at once; a control that would need one more is dropped and counted with the
/// dropped notes.
const EXPRESSIONS: usize = 64;

/// Which input a live note comes from: the control ring (the typing keyboard), or a handle on
/// the MIDI inbox ([`MidiInbox::for_port`] gives each port one of its own). The player holds
/// keys by input, so one device's note-off never ends another device's note of the same key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Input(u64);

impl Input {
    /// The control ring's notes: the typing keyboard's.
    const CONTROLS: Input = Input(0);
    /// The inbox's own handle, the one [`RealtimeLink::midi_inbox`] gives.
    const INBOX: Input = Input(1);
}

/// What an input hands the player through the inbox.
#[derive(Debug, Clone, Copy)]
enum Delivery {
    /// A note, with the channel it plays (`None`: the selected one), and whether a note on goes
    /// to the channel's MIDI out too.
    Note {
        channel: Option<ChannelId>,
        note: LiveNote,
        out: bool,
    },
    /// A performance control, with the channel it plays as a note's (`None`: the selected one),
    /// and whether it goes on to the channel's MIDI out too.
    Control {
        channel: Option<ChannelId>,
        control: LiveControl,
        out: bool,
    },
    /// The input is gone: its pedal lifts and every key it holds lets go.
    Release,
}

/// Where MIDI input threads put the notes they receive for the player, which takes them at
/// the start of its next call, after the controls. Any number of threads may push at once
/// (each device's connection has a handle); pushing never blocks, waits or allocates, and a
/// full inbox drops the note and counts it. A clone is the same input; [`for_port`] makes
/// another one.
///
/// [`for_port`]: MidiInbox::for_port
#[derive(Debug, Clone)]
pub struct MidiInbox {
    /// Each delivery with the input it came from.
    notes: Arc<ArrayQueue<(Input, Delivery)>>,
    dropped: Arc<AtomicU64>,
    /// The input this handle's notes come from.
    input: Input,
    /// The next input [`for_port`](Self::for_port) gives out.
    inputs: Arc<AtomicU64>,
}

impl MidiInbox {
    fn new() -> Self {
        Self {
            notes: Arc::new(ArrayQueue::new(INBOX_CAPACITY)),
            dropped: Arc::new(AtomicU64::new(0)),
            input: Input::INBOX,
            inputs: Arc::new(AtomicU64::new(Input::INBOX.0 + 1)),
        }
    }

    /// A handle into the same inbox for an input of its own, one MIDI port's: the keys it
    /// holds are its own, so its note-offs end only its notes, and
    /// [`release`](Self::release) lets go of them all.
    pub fn for_port(&self) -> Self {
        Self {
            input: Input(self.inputs.fetch_add(1, Ordering::Relaxed)),
            ..self.clone()
        }
    }

    /// Hands `note` to the player to play on the selected channel; `false` when the inbox was
    /// full and it was dropped.
    pub fn push(&self, note: LiveNote) -> bool {
        self.queue(Delivery::Note {
            channel: None,
            note,
            out: true,
        })
    }

    /// Hands `note` to the player to play on `channel` whatever is selected (the selected
    /// channel after all when the plan has no `channel`); `false` when the inbox was full and
    /// it was dropped. A note-off ends this input's note of its key on the channel it started
    /// on, as any note-off does.
    pub fn push_to(&self, channel: ChannelId, note: LiveNote) -> bool {
        self.queue(Delivery::Note {
            channel: Some(channel),
            note,
            out: true,
        })
    }

    /// As [`push_to`](Self::push_to) (`Some`) or [`push`](Self::push) (`None`), but the note
    /// never goes to the channel's MIDI out: it plays and is recorded as any note does. What a
    /// port whose instrument already hears its own keys hands in (echo off). A note-off ends the
    /// note as any does, and a note sent out before ends there too.
    pub fn push_quiet(&self, channel: Option<ChannelId>, note: LiveNote) -> bool {
        self.queue(Delivery::Note {
            channel,
            note,
            out: false,
        })
    }

    /// Hands `control` to the player for `channel` (the selected channel when `None`, or when
    /// the plan lacks it), as [`push_to`](Self::push_to) a note: it goes on to the channel's MIDI
    /// out too when `out`, else it acts quietly, as [`push_quiet`](Self::push_quiet)'s notes do.
    /// `false` when the inbox was full and it was dropped.
    pub fn push_control(
        &self,
        channel: Option<ChannelId>,
        control: LiveControl,
        out: bool,
    ) -> bool {
        self.queue(Delivery::Control {
            channel,
            control,
            out,
        })
    }

    /// Lets go of every key this input holds and lifts its pedal, after the notes it handed in
    /// before: what a port whose connection closed needs, as its note-offs can no longer come.
    /// `false` when the inbox was full and it was dropped; [`Control::LiveNotesOff`] then ends
    /// every live note.
    pub fn release(&self) -> bool {
        self.queue(Delivery::Release)
    }

    fn queue(&self, delivery: Delivery) -> bool {
        let taken = self.notes.push((self.input, delivery)).is_ok();
        if !taken {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        taken
    }

    /// Notes dropped so far because the inbox was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// Makes the two halves for playing `update`'s plan at `tempo`, stopped at tick 0. Its new
/// processors are installed here, on the caller's thread. Later tempo changes go through
/// [`Control::SetTempo`]; a plan carries no tempo.
pub fn link(update: PlanUpdate, tempo: MilliBpm) -> (RealtimeLink, Player) {
    let PlanUpdate { plan, mut delta } = update;
    let mut runtime = Box::new(Runtime::new());
    runtime.apply(&mut delta);
    let (plans_in, plans_out) = RingBuffer::new(PLAN_RING_CAPACITY);
    let (retired_in, retired_out) = RingBuffer::new(PLAN_RING_CAPACITY);
    let (controls_in, controls_out) = RingBuffer::new(CONTROL_RING_CAPACITY);
    let (previews_in, previews_out) = RingBuffer::new(PREVIEW_RING_CAPACITY);
    let (preview_retire_in, preview_retire_out) = RingBuffer::new(PREVIEW_RETIRE_CAPACITY);
    let (inputs_in, inputs_out) = RingBuffer::new(INPUT_RING_CAPACITY);
    let (input_retire_in, input_retire_out) = RingBuffer::new(INPUT_RETIRE_CAPACITY);
    let (events_in, events_out) = RingBuffer::new(EVENT_RING_CAPACITY);
    let tap_capacity = TAP_SECONDS * 2 * usize::try_from(plan.sample_rate).unwrap_or(48_000);
    let (tap_in, tap_out) = RingBuffer::new(tap_capacity);
    let shared = Arc::new(Shared::new());
    let inbox = MidiInbox::new();
    let (midi, midi_reader) = midi_out::ring();
    let link = RealtimeLink {
        plans: plans_in,
        retired: retired_out,
        controls: controls_in,
        previews: previews_in,
        retired_previews: preview_retire_out,
        inputs: inputs_in,
        retired_inputs: input_retire_out,
        events: events_out,
        tap: Some(RecordTap {
            frames: tap_out,
            shared: Arc::clone(&shared),
        }),
        shared: Arc::clone(&shared),
        waiting: VecDeque::new(),
        freed: Vec::new(),
        freed_sources: Vec::new(),
        sample_rate: plan.sample_rate,
        ppq: plan.ppq,
        inbox: inbox.clone(),
        midi_out: Some(midi_reader),
    };
    let player = Player {
        transport: Transport::new(tempo, plan.sample_rate, plan.ppq),
        plan,
        plans: plans_out,
        retire: retired_in,
        controls: controls_out,
        previews: previews_out,
        retire_previews: preview_retire_in,
        inputs: inputs_out,
        retire_inputs: input_retire_in,
        events: events_in,
        tap: tap_in,
        shared,
        runtime,
        inbox,
        held: HeldKeys::default(),
        expressions: Expressions::default(),
        let_go: 0,
        count_in: None,
        metronome: false,
        last_click: None,
        recording: None,
        control_due: false,
        output_frames: 0,
        midi,
    };
    (link, player)
}

/// What the audio half publishes after every `process` call.
#[derive(Debug)]
struct Shared {
    playing: AtomicBool,
    position_samples: AtomicU64,
    position_ticks: AtomicU64,
    /// Each insert's peak since the UI last took it, left then right, as f32 bits.
    meters: Box<[AtomicU32]>,
    /// Notes dropped so far because a block (plugin channels) or the held keys had no room for
    /// them.
    dropped_notes: AtomicU64,
    /// How many `Control::Suspend`s the player has carried out.
    suspends: AtomicU64,
    /// Each automation's value after the last call, by the plan's order: its ID in the high
    /// 32 bits, the value's f32 bits in the low 32 (NaN while it does not govern).
    automation_values: Box<[AtomicU64]>,
    /// Audio clip blocks that played silence because their stream was not ready.
    underruns: AtomicU64,
    /// Events dropped because the event ring was full.
    events_dropped: AtomicU64,
    /// The output frame (counted since the link was made) where song play began while
    /// recording; [`NOT_RECORDING`] otherwise.
    recording_from: AtomicU64,
    /// Whether a count-in is playing.
    counting_in: AtomicBool,
    /// Master frames the record tap had no room for.
    tap_dropped: AtomicU64,
    /// Live-note events dropped because the event ring was full: notes a recording lost.
    live_events_dropped: AtomicU64,
}

/// `recording_from` while not recording.
const NOT_RECORDING: u64 = u64::MAX;

impl Shared {
    fn new() -> Self {
        Self {
            playing: AtomicBool::new(false),
            position_samples: AtomicU64::new(0),
            position_ticks: AtomicU64::new(0),
            meters: (0..2 * BUSES).map(|_| AtomicU32::new(0)).collect(),
            dropped_notes: AtomicU64::new(0),
            suspends: AtomicU64::new(0),
            automation_values: (0..MAX_AUTOMATIONS)
                .map(|_| AtomicU64::new(u64::from(f32::NAN.to_bits())))
                .collect(),
            underruns: AtomicU64::new(0),
            events_dropped: AtomicU64::new(0),
            recording_from: AtomicU64::new(NOT_RECORDING),
            counting_in: AtomicBool::new(false),
            tap_dropped: AtomicU64::new(0),
            live_events_dropped: AtomicU64::new(0),
        }
    }
}

/// Where the player says a recording's song play began, for any thread to read without
/// waiting.
#[derive(Clone)]
pub struct RecordClock(Arc<Shared>);

impl RecordClock {
    /// The output frame where song play began while recording, or `None` when not recording.
    pub fn recording_from(&self) -> Option<u64> {
        let from = self.0.recording_from.load(Ordering::Acquire);
        (from != NOT_RECORDING).then_some(from)
    }
}

/// The UI side of the record tap: the master's frames, interleaved left and right, from where
/// song play began while recording with the tap on. Take it from the link once
/// ([`RealtimeLink::take_record_tap`]) and give it to the thread that writes the recording.
pub struct RecordTap {
    frames: Consumer<f32>,
    shared: Arc<Shared>,
}

impl RecordTap {
    /// Moves every value waiting into `out` (interleaved stereo), and returns how many.
    /// Whole stereo frames only: the audio thread pushes a frame value by value, so a read
    /// between the two values would hand a writer half a frame.
    pub fn read_into(&mut self, out: &mut Vec<f32>) -> usize {
        let values = self.frames.slots() / 2 * 2;
        out.reserve(values);
        let mut moved = 0;
        while moved < values {
            let Ok(value) = self.frames.pop() else { break };
            out.push(value);
            moved += 1;
        }
        moved
    }

    /// Frames the tap had no room for, so far. A writer pads as many silent frames, so a
    /// recording keeps its length.
    pub fn dropped(&self) -> u64 {
        self.shared.tap_dropped.load(Ordering::Acquire)
    }
}

/// Where the playhead is: the absolute sample of the next frame to play, and the tick it is
/// in. Both count from tick 0, sample 0 of the current play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub samples: u64,
    pub ticks: Ticks,
}

/// A plan for a different sample rate or PPQ than the player was made for. Changing either
/// needs a new link.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("plan is for {got_rate} Hz at ppq {got_ppq}, but the player runs {rate} Hz at ppq {ppq}")]
pub struct TimeBaseMismatch {
    pub rate: u32,
    pub ppq: u16,
    pub got_rate: u32,
    pub got_ppq: u16,
}

/// The control ring was full, so the control was not sent; it is handed back.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
#[error("the control ring is full")]
pub struct ControlRingFull(pub Control);

/// The player has not taken the plan updates already waiting, so no more are queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("the audio thread is not taking new plans")]
pub struct PlanQueueFull;

/// The UI half of the link. Lives on the UI thread, next to the project and the compiler.
pub struct RealtimeLink {
    plans: Producer<Box<PlanUpdate>>,
    retired: Consumer<Box<PlanUpdate>>,
    controls: Producer<Control>,
    previews: Producer<Box<dyn ClipSource>>,
    retired_previews: Consumer<Box<dyn ClipSource>>,
    inputs: Producer<Box<InputFeeds>>,
    retired_inputs: Consumer<Box<InputFeeds>>,
    events: Consumer<EngineEvent>,
    /// Until taken.
    tap: Option<RecordTap>,
    shared: Arc<Shared>,
    /// Updates that did not fit in the plan ring yet, oldest first.
    waiting: VecDeque<Box<PlanUpdate>>,
    /// Slots whose processors came back from the player, for the compiler to reuse.
    freed: Vec<ProcessorSlot>,
    /// Slots whose clip sources came back from the player.
    freed_sources: Vec<SourceSlot>,
    sample_rate: u32,
    ppq: u16,
    inbox: MidiInbox,
    /// Until taken.
    midi_out: Option<MidiOutReader>,
}

impl RealtimeLink {
    /// The inbox MIDI input threads hand the player their notes through; give each connection
    /// a clone.
    pub fn midi_inbox(&self) -> MidiInbox {
        self.inbox.clone()
    }

    /// Live-note events the player could not post because the event ring was full, so far:
    /// notes a recording lost.
    pub fn lost_live_events(&self) -> u64 {
        self.shared.live_events_dropped.load(Ordering::Relaxed)
    }

    /// Whether another plan update may be queued: `Err` while [`WAITING_LIMIT`] updates wait
    /// for a player that is alive but not taking them. It first hands the player what it has
    /// room for, and drops everything when the player is gone.
    ///
    /// Ask before compiling the update, not after: compiling gives new effects their slots, so
    /// an update refused once built would leave the compiler naming processors the player never
    /// gets.
    pub fn check_room(&mut self) -> Result<(), PlanQueueFull> {
        self.drain_retired();
        if self.waiting.len() < WAITING_LIMIT {
            Ok(())
        } else {
            Err(PlanQueueFull)
        }
    }

    /// Queues `update` for the audio half, which applies it at the start of a later `process`
    /// call. When the plan ring is full the update waits here and is sent, in order, by a later
    /// call of this or [`drain_retired`](Self::drain_retired). Call
    /// [`check_room`](Self::check_room) before compiling `update`.
    ///
    /// Why every update is sent, none merged into a newer one: each carries processor changes
    /// (new processors, removals) that a newer plan does not repeat, so dropping one would
    /// leave processors running that the document no longer has.
    pub fn push_plan(&mut self, update: PlanUpdate) -> Result<(), TimeBaseMismatch> {
        debug_assert!(
            self.waiting.len() < WAITING_LIMIT,
            "check_room comes before compiling a plan update"
        );
        let plan = &update.plan;
        if (plan.sample_rate, plan.ppq) != (self.sample_rate, self.ppq) {
            return Err(TimeBaseMismatch {
                rate: self.sample_rate,
                ppq: self.ppq,
                got_rate: plan.sample_rate,
                got_ppq: plan.ppq,
            });
        }
        self.waiting.push_back(Box::new(update));
        self.drain_retired();
        Ok(())
    }

    /// Sends a control; it takes effect at the start of the next `process` call.
    pub fn send(&mut self, control: Control) -> Result<(), ControlRingFull> {
        self.controls
            .push(control)
            .map_err(|PushError::Full(control)| ControlRingFull(control))
    }

    /// Plays `source` from its first frame into the master's input at unity, in place of any
    /// file being previewed, whether or not the transport plays. When the preview ring is full
    /// the source is handed back.
    pub fn preview(&mut self, source: Box<dyn ClipSource>) -> Result<(), Box<dyn ClipSource>> {
        self.drop_retired_previews();
        self.previews
            .push(source)
            .map_err(|PushError::Full(source)| source)
    }

    /// Hands the player `feeds` in place of the input feeds it reads, from the start of its next
    /// call: from then on output frame k plays frame k of each feed. An empty set takes them all
    /// away. When the ring is full the set is handed back.
    pub fn set_input_feeds(&mut self, feeds: InputFeeds) -> Result<(), InputFeeds> {
        self.drop_retired_inputs();
        self.inputs
            .push(Box::new(feeds))
            .map_err(|PushError::Full(feeds)| *feeds)
    }

    /// Drops the sets of input feeds the player has replaced, here on the UI thread.
    fn drop_retired_inputs(&mut self) {
        while let Ok(feeds) = self.retired_inputs.pop() {
            drop(feeds);
        }
    }

    /// The events the player has posted since the last call, oldest first.
    pub fn take_events(&mut self) -> Vec<EngineEvent> {
        let mut events = Vec::with_capacity(self.events.slots());
        while let Ok(event) = self.events.pop() {
            events.push(event);
        }
        events
    }

    /// Audio clip blocks that played silence because their stream was not ready, so far.
    pub fn underruns(&self) -> u64 {
        self.shared.underruns.load(Ordering::Relaxed)
    }

    /// The value the `index`th automation of the plan (by ID) set in the last `process` call,
    /// with that automation's ID; `None` while it does not govern.
    pub fn automation_value(&self, index: usize) -> Option<(AutomationId, f32)> {
        let packed = self
            .shared
            .automation_values
            .get(index)?
            .load(Ordering::Relaxed);
        let value = f32::from_bits(packed as u32);
        let id = AutomationId((packed >> 32) as u32);
        (!value.is_nan()).then_some((id, value))
    }

    /// A handle any thread can read the recording's start through: an input device's callback
    /// keeps its frames from the first callback after song play began.
    pub fn record_clock(&self) -> RecordClock {
        RecordClock(Arc::clone(&self.shared))
    }

    /// The record tap, once: the master's frames while recording with the tap on.
    pub fn take_record_tap(&mut self) -> Option<RecordTap> {
        self.tap.take()
    }

    /// The MIDI-out ring's reader, once: the note on and note off messages the player sends for
    /// input channels with a MIDI out, each with the output frame it is due on. Give it to the
    /// thread that sends them to the ports.
    pub fn take_midi_out(&mut self) -> Option<MidiOutReader> {
        self.midi_out.take()
    }

    /// Gives the record tap back, for the next recording.
    pub fn return_record_tap(&mut self, tap: RecordTap) {
        self.tap = Some(tap);
    }

    /// Whether a count-in was playing at the end of the last `process` call.
    pub fn counting_in(&self) -> bool {
        self.shared.counting_in.load(Ordering::Relaxed)
    }

    /// Drops the previews the player has finished with, here on the UI thread.
    fn drop_retired_previews(&mut self) {
        while let Ok(source) = self.retired_previews.pop() {
            drop(source);
        }
    }

    /// The playhead as of the end of the last `process` call.
    pub fn position(&self) -> Position {
        let ticks = self.shared.position_ticks.load(Ordering::Relaxed);
        Position {
            samples: self.shared.position_samples.load(Ordering::Relaxed),
            ticks: Ticks(i64::try_from(ticks).unwrap_or(i64::MAX)),
        }
    }

    /// Whether the player was playing at the end of the last `process` call.
    pub fn playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }

    /// How many [`Control::Suspend`]s the player has carried out: when it moves past the count
    /// read before sending one, every processor has been suspended on the audio thread.
    pub fn suspends(&self) -> u64 {
        self.shared.suspends.load(Ordering::Acquire)
    }

    /// Notes the player has dropped so far: plugin-channel notes because one block started
    /// more than it has room for, and live notes struck with every hold taken.
    pub fn dropped_notes(&self) -> u64 {
        self.shared.dropped_notes.load(Ordering::Relaxed)
    }

    /// The loudest sample insert `insert` put out on each side since this was last called,
    /// after its fader, pan, mute and solo; 0 for an insert the mixer does not have.
    pub fn take_peak(&self, insert: InsertId) -> [f32; 2] {
        let take = |side: usize| {
            self.shared
                .meters
                .get(2 * usize::from(insert.0) + side)
                .map_or(0.0, |meter| {
                    f32::from_bits(meter.swap(0, Ordering::Relaxed))
                })
        };
        [take(0), take(1)]
    }

    /// Drops the updates the audio half has finished with, here on the UI thread (the old plans
    /// and the processors it took out), keeps their freed slots for
    /// [`take_freed_slots`](Self::take_freed_slots), then sends updates that were waiting for
    /// room. Returns how many updates came back.
    ///
    /// Call it regularly (the playhead timer is a good place): the player swaps in a new plan
    /// only while the retire ring has room for the old one.
    pub fn drain_retired(&mut self) -> usize {
        let mut dropped = 0;
        self.drop_retired_previews();
        self.drop_retired_inputs();
        // Each popped update is dropped at the end of its loop iteration.
        while let Ok(update) = self.retired.pop() {
            self.freed.extend_from_slice(&update.delta.remove);
            self.freed_sources
                .extend_from_slice(&update.delta.remove_sources);
            dropped += 1;
        }
        if self.plans.is_abandoned() {
            // The player is gone (no audio device, say): nothing will ever apply these updates,
            // so they are dropped here, processors and all, and every slot they name is free
            // again. Without this the queue would grow with every edit.
            for update in self.waiting.drain(..) {
                self.freed.extend_from_slice(&update.delta.remove);
                self.freed
                    .extend(update.delta.install.iter().map(|&(slot, _)| slot));
                self.freed_sources
                    .extend_from_slice(&update.delta.remove_sources);
                self.freed_sources
                    .extend(update.delta.install_sources.iter().map(|&(slot, _)| slot));
            }
            return dropped;
        }
        while let Some(update) = self.waiting.pop_front() {
            if let Err(PushError::Full(update)) = self.plans.push(update) {
                self.waiting.push_front(update);
                break;
            }
        }
        dropped
    }

    /// The processor slots the player has let go of since the last call; hand them to
    /// [`crate::compile::Compiler::release`].
    pub fn take_freed_slots(&mut self) -> Vec<ProcessorSlot> {
        mem::take(&mut self.freed)
    }

    /// The clip source slots the player has let go of since the last call; hand them to
    /// [`crate::compile::Compiler::release_sources`].
    pub fn take_freed_sources(&mut self) -> Vec<SourceSlot> {
        mem::take(&mut self.freed_sources)
    }
}

/// The audio half of the link: renders the current plan. It owns its plan, the voices, the
/// buses, the processors, the clip sources, the preview and the transport; nothing else touches
/// them.
pub struct Player {
    plan: Box<PlayPlan>,
    plans: Consumer<Box<PlanUpdate>>,
    retire: Producer<Box<PlanUpdate>>,
    controls: Consumer<Control>,
    previews: Consumer<Box<dyn ClipSource>>,
    retire_previews: Producer<Box<dyn ClipSource>>,
    inputs: Consumer<Box<InputFeeds>>,
    retire_inputs: Producer<Box<InputFeeds>>,
    events: Producer<EngineEvent>,
    tap: Producer<f32>,
    shared: Arc<Shared>,
    /// Boxed: its voice pools and tables are tens of kilobytes, and a player is moved by value
    /// (out of `link`, into the thread that runs it), each move a copy on the stack.
    runtime: Box<Runtime>,
    transport: Transport,
    inbox: MidiInbox,
    /// The live keys held down: which input holds each, and the channel it plays.
    held: HeldKeys,
    /// What each input's performance controls hold on each channel: the pedal.
    expressions: Expressions,
    /// Keys let go of under a pedal so far: the order they end in when it lifts.
    let_go: u64,
    /// The count-in playing before a recording starts.
    count_in: Option<CountIn>,
    /// Whether the metronome clicks while the transport plays, and the click it played last:
    /// the output frame it began on and whether it is accented.
    metronome: bool,
    last_click: Option<(u64, bool)>,
    /// While recording: whether the master goes into the record tap.
    recording: Option<Recording>,
    /// The next block starts on a control point although it may be off the grid: after Play,
    /// a Seek, a wrap or a new plan.
    control_due: bool,
    /// Frames put out since the link was made.
    output_frames: u64,
    /// The input channels' MIDI out: the notes sounding there and the ring they go out on.
    midi: MidiOutState,
}

/// The live keys held down, by input: the channel each plays. An input holds a key once and a
/// channel sounds a key once, so the runtime holds the same notes, by channel, in a table as
/// long as this one.
#[derive(Debug)]
struct HeldKeys([Option<HeldKey>; render::HELD_NOTES]);

#[derive(Debug, Clone, Copy)]
struct HeldKey {
    input: Input,
    key: u8,
    channel: ChannelId,
    /// The MIDI channel (0 to 15) its note on went out on, when it went to a MIDI out.
    out: Option<u8>,
    /// When its key was let go of while the input's pedal held it, how many keys had been let
    /// go of under a pedal by then: it sounds on until the pedal lifts, and the lift ends such
    /// keys in that order. `None` while the key is down.
    let_go: Option<u64>,
}

impl Default for HeldKeys {
    fn default() -> Self {
        Self([None; render::HELD_NOTES])
    }
}

impl HeldKeys {
    /// Holds `held`; `false` when every hold is taken.
    fn hold(&mut self, held: HeldKey) -> bool {
        let Some(free) = self.0.iter_mut().find(|slot| slot.is_none()) else {
            return false;
        };
        *free = Some(held);
        true
    }

    /// Lets go of `input`'s `key`, returning how it was held.
    fn take(&mut self, input: Input, key: u8) -> Option<HeldKey> {
        let held = self
            .0
            .iter_mut()
            .find(|held| held.is_some_and(|held| held.input == input && held.key == key))?;
        held.take()
    }

    /// `input`'s `key` while it is down (not let go of under a pedal).
    fn down(&self, input: Input, key: u8) -> Option<HeldKey> {
        self.find(|held| held.input == input && held.key == key && held.let_go.is_none())
    }

    /// Marks `input`'s `key` let go of under its pedal, the `order`th so far.
    fn let_go(&mut self, input: Input, key: u8, order: u64) {
        let found = self
            .0
            .iter_mut()
            .flatten()
            .find(|held| held.input == input && held.key == key);
        if let Some(held) = found {
            held.let_go = Some(order);
        }
    }

    /// The key `input` let go of under its pedal first, of those still sounding.
    fn first_let_go(&self, input: Input) -> Option<u8> {
        self.0
            .iter()
            .flatten()
            .filter(|held| held.input == input)
            .filter_map(|held| held.let_go.map(|order| (order, held.key)))
            .min()
            .map(|(_, key)| key)
    }

    /// The input holding `key` on `channel`.
    fn holder(&self, channel: ChannelId, key: u8) -> Option<Input> {
        self.find(|held| held.channel == channel && held.key == key)
            .map(|held| held.input)
    }

    /// A key `input` holds.
    fn key_of(&self, input: Input) -> Option<u8> {
        self.find(|held| held.input == input).map(|held| held.key)
    }

    /// An input holding `key`, on any channel.
    fn input_holding(&self, key: u8) -> Option<Input> {
        self.find(|held| held.key == key).map(|held| held.input)
    }

    fn find(&self, matches: impl Fn(&HeldKey) -> bool) -> Option<HeldKey> {
        self.0.iter().flatten().copied().find(|held| matches(held))
    }
}

/// What one input's performance controls hold on one channel it plays: the pedal's value, the
/// bend's, the wheel's, and the MIDI channel they last went out on when the channel's MIDI out
/// sent them.
#[derive(Debug, Clone, Copy)]
struct Expression {
    input: Input,
    channel: ChannelId,
    sustain: u8,
    bend: i16,
    wheel: u8,
    out: Option<u8>,
}

impl Expression {
    /// Whether its pedal is down, holding the notes the input lets go of.
    fn holds(&self) -> bool {
        self.sustain >= PEDAL_DOWN
    }

    /// Whether every control is at its rest: then nothing needs keeping.
    fn at_rest(&self) -> bool {
        self.sustain == 0 && self.bend == 0 && self.wheel == 0
    }

    /// What a live note it plays begins with.
    fn controls(&self) -> Controls {
        Controls {
            bend: self.bend,
            wheel: self.wheel,
        }
    }
}

/// The inputs' controls off their rest, by (input, channel), [`EXPRESSIONS`] at most. A fixed
/// table, so the audio thread never allocates for it.
#[derive(Debug)]
struct Expressions([Option<Expression>; EXPRESSIONS]);

impl Default for Expressions {
    fn default() -> Self {
        Self([None; EXPRESSIONS])
    }
}

impl Expressions {
    /// What `input`'s controls hold on `channel`, when off their rest.
    fn get(&self, input: Input, channel: ChannelId) -> Option<Expression> {
        self.0
            .iter()
            .flatten()
            .copied()
            .find(|held| held.input == input && held.channel == channel)
    }

    /// The entry of `input` on `channel`, made at rest when there is none; `None` when the
    /// table is full.
    fn entry(&mut self, input: Input, channel: ChannelId) -> Option<&mut Expression> {
        let index = self
            .0
            .iter()
            .position(|held| {
                held.is_some_and(|held| held.input == input && held.channel == channel)
            })
            .or_else(|| self.0.iter().position(Option::is_none))?;
        let slot = &mut self.0[index];
        Some(slot.get_or_insert(Expression {
            input,
            channel,
            sustain: 0,
            bend: 0,
            wheel: 0,
            out: None,
        }))
    }

    /// Whether `input` has an entry on `channel` or the table room for one.
    fn has_room_for(&self, input: Input, channel: ChannelId) -> bool {
        self.get(input, channel).is_some() || self.0.iter().any(Option::is_none)
    }

    /// Lets go of the entries back at rest.
    fn tidy(&mut self) {
        for slot in &mut self.0 {
            if slot.is_some_and(|held| held.at_rest()) {
                *slot = None;
            }
        }
    }
}

/// A count-in: one bar of clicks, `done` of its `frames` frames played.
#[derive(Debug, Clone, Copy)]
struct CountIn {
    frames: u64,
    done: u64,
    /// Where each beat's click starts, counted from the count-in's start.
    beats: [u64; 4],
}

#[derive(Debug, Clone, Copy)]
struct Recording {
    tap_master: bool,
}

/// Puts `event` on the event ring, dropping and counting it when the ring is full.
fn post(events: &mut Producer<EngineEvent>, shared: &Shared, event: EngineEvent) {
    if events.push(event).is_err() {
        shared.events_dropped.fetch_add(1, Ordering::Relaxed);
        if event.is_live_note() {
            shared.live_events_dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl CountIn {
    /// The click at frame `frame` of the count-in: [`click`] at each beat, accented on beat 1.
    fn click(&self, frame: u64) -> f32 {
        self.beats
            .iter()
            .enumerate()
            .find_map(|(beat, &at)| click(frame.checked_sub(at)?, beat == 0))
            .unwrap_or(0.0)
    }
}

impl Player {
    /// Renders the next `out.len()` frames (interleaved stereo) into `out`, overwriting it:
    /// silence while stopped (but for a previewed file, live notes and the input channels). In
    /// order: apply every waiting control, swap in at most one new plan, take the newest preview
    /// and the newest set of input feeds, render, publish the playhead. Controls and swaps happen
    /// only here, at the start of a call, never part way through one. Every input feed is read
    /// once for each frame put out.
    ///
    /// In song mode the buffer is cut where the playhead reaches the plan's wrap point (the
    /// wrap is taken at the start of the next piece, or of the next call when a buffer ends
    /// exactly there, so the frame that would play the wrap tick plays the tick it wraps to),
    /// and where automation edges and control points fall.
    ///
    /// Why it never allocates or frees: live playback runs this on the audio thread, where
    /// waiting on the allocator can miss the device deadline and cause an audible glitch.
    pub fn process(&mut self, out: &mut [[f32; 2]]) {
        self.drain_controls();
        self.drain_inbox();
        self.swap_plan();
        self.take_preview();
        self.take_inputs();
        let mut rest = out;
        while !rest.is_empty() {
            let len = if let Some(count_in) = self.count_in {
                self.count_in_piece(rest, count_in)
            } else if self.transport.playing() {
                self.playing_piece(rest)
            } else {
                self.stopped(rest);
                rest.len()
            };
            let (_, later) = rest.split_at_mut(len);
            self.output_frames = self.output_frames.saturating_add(len as u64);
            rest = later;
        }
        self.retire_finished_preview();
        self.publish();
    }

    /// Renders all of `out` while stopped: the previewed file, the live notes and the input
    /// channels heard through the mixer, else silence (the input feeds read on all the same);
    /// every processor gets its idle call.
    fn stopped(&mut self, out: &mut [[f32; 2]]) {
        let context = ProcessContext {
            sample_rate: self.plan.sample_rate,
            tempo: self.transport.timeline().tempo(),
            ppq: self.plan.ppq,
            transport: TransportInfo {
                playing: false,
                song_tick: self.transport.playhead_tick(),
                steady_time: self.transport.clock(),
            },
            events: &[],
        };
        if self.runtime.preview.is_some()
            || self.runtime.live_sounding(self.output_frames)
            || self.runtime.inputs_heard(&self.plan)
        {
            let mut at = self.output_frames;
            for piece in out.chunks_mut(CHUNK) {
                render::stopped_block(
                    &mut self.plan,
                    &mut self.runtime,
                    &context,
                    piece,
                    &self.shared.meters,
                    at,
                );
                at = at.saturating_add(piece.len() as u64);
            }
        } else {
            out.fill([0.0; 2]);
            self.runtime.skip_inputs(out.len());
        }
        self.runtime.idle(&context);
    }

    /// Renders up to the count-in's end into `out` (stopped, the clicks added after the master)
    /// and, at its end, starts the song playing; returns the frames rendered.
    fn count_in_piece(&mut self, out: &mut [[f32; 2]], count_in: CountIn) -> usize {
        let left = count_in.frames.saturating_sub(count_in.done);
        let len = usize::try_from(left).map_or(out.len(), |left| left.min(out.len()));
        let piece = &mut out[..len];
        self.stopped(piece);
        for (frame, value) in (count_in.done..).zip(piece.iter_mut()) {
            add_click(value, count_in.click(frame));
        }
        let done = count_in.done + len as u64;
        if done >= count_in.frames {
            self.count_in = None;
            self.start_song(self.output_frames.saturating_add(len as u64));
        } else {
            self.count_in = Some(CountIn { done, ..count_in });
        }
        len
    }

    /// Starts playing from the play-from tick, as Play does; while recording, notes the output
    /// frame where the song begins.
    fn start_song(&mut self, at_output_frame: u64) {
        if self.transport.apply(Control::Play) {
            self.jumped(at_output_frame);
        }
        if self.recording.is_some() {
            self.shared
                .recording_from
                .store(at_output_frame, Ordering::Release);
        }
    }

    /// Renders the next piece of a playing transport into the front of `out`, up to a wrap, an
    /// automation edge, a control point or [`CHUNK`] frames; returns its length.
    fn playing_piece(&mut self, out: &mut [[f32; 2]]) -> usize {
        let wrap = self.wrap();
        if self.transport.wrap_if_due(wrap) {
            self.control_due = true;
        }
        let position = self.transport.position();
        let song = self.plan.playing == Playing::Song;
        // `frames_to_wrap` is never 0, so every piece renders at least one frame.
        let mut len = self
            .transport
            .frames_to_wrap(wrap)
            .and_then(|frames| usize::try_from(frames).ok())
            .map_or(out.len(), |frames| frames.min(out.len()))
            .min(CHUNK);
        if song {
            if let Some(frames) = self.frames_to_automation_edge(position) {
                len = len.min(frames);
            }
            if self.plan.control_rate {
                let to_grid = CONTROL_GRID - position % CONTROL_GRID;
                len = len.min(usize::try_from(to_grid).unwrap_or(len));
            }
        }
        let control_point = self.control_due
            || (song
                && self.plan.control_rate
                && (position.is_multiple_of(CONTROL_GRID) || self.on_automation_edge(position)));
        let piece = &mut out[..len];
        render::block(
            &mut self.plan,
            &mut self.runtime,
            self.transport.timeline(),
            position,
            self.transport.clock(),
            piece,
            &self.shared.meters,
            control_point,
            self.output_frames,
        );
        self.control_due = false;
        self.midi.piece(
            &mut self.plan,
            self.transport.timeline(),
            position,
            wrap,
            self.output_frames,
            len,
        );
        self.post_stream_events();
        if self.recording.is_some_and(|recording| recording.tap_master) {
            self.tap_frames(piece);
        }
        // After the tap: the metronome is heard, never recorded.
        if self.metronome {
            self.add_clicks(piece, position);
        }
        self.transport.advance(len);
        len
    }

    /// Where song playback wraps: the plan's loop or song end. Recording plays straight
    /// through: no loop, no song-end wrap.
    fn wrap(&self) -> Option<Wrap> {
        match self.recording {
            Some(_) => None,
            None => self.plan.wrap().map(|(at, to)| Wrap { at, to }),
        }
    }

    /// How the song plays on from the next frame, while it plays: what MIDI out works out where
    /// notes land by.
    fn course(&self) -> Option<Course> {
        self.transport.playing().then(|| Course {
            timeline: *self.transport.timeline(),
            position: self.transport.position(),
            wrap: self.wrap(),
            at: self.output_frames,
        })
    }

    /// Adds the metronome's clicks into `piece`, which plays from song sample `position`: one
    /// from the first sample of every beat (a tick that is a multiple of the ppq) the piece
    /// plays, accented on a bar's first, timed by the output frames, so a click that began in
    /// an earlier piece (before a wrap too) plays on; each held to full scale ([`add_click`]).
    fn add_clicks(&mut self, piece: &mut [[f32; 2]], position: u64) {
        let timeline = self.transport.timeline();
        let beat = i64::from(self.plan.ppq);
        let bar = beat * i64::from(time::BEATS_PER_BAR);
        // From the beat the piece's first sample is in, the first that starts in the piece.
        let mut tick = timeline.tick_at(position).0.div_euclid(beat) * beat;
        let mut next = timeline.tick_to_sample(Ticks(tick));
        while next.is_some_and(|sample| sample < position) {
            tick += beat;
            next = timeline.tick_to_sample(Ticks(tick));
        }
        for (offset, frame) in (0u64..).zip(piece.iter_mut()) {
            let at = self.output_frames.saturating_add(offset);
            if next == Some(position.saturating_add(offset)) {
                self.last_click = Some((at, tick.rem_euclid(bar) == 0));
                tick += beat;
                next = timeline.tick_to_sample(Ticks(tick));
            }
            let value = self
                .last_click
                .and_then(|(start, accented)| click(at.checked_sub(start)?, accented));
            if let Some(value) = value {
                add_click(frame, value);
            }
        }
    }

    /// Frames from song sample `position` to the next automation edge after it.
    fn frames_to_automation_edge(&self, position: u64) -> Option<usize> {
        let timeline = self.transport.timeline();
        let tick = timeline.tick_at(position);
        let edges = &self.plan.automation_edges;
        let next = edges.get(edges.partition_point(|&edge| edge <= tick))?;
        let at = timeline.tick_to_sample(*next)?;
        usize::try_from(at.checked_sub(position)?).ok()
    }

    /// Whether an automation edge falls on song sample `position`.
    fn on_automation_edge(&self, position: u64) -> bool {
        let timeline = self.transport.timeline();
        let tick = timeline.tick_at(position);
        timeline.tick_to_sample(tick) == Some(position)
            && self.plan.automation_edges.binary_search(&tick).is_ok()
    }

    /// Puts the clips whose streams were not ready, or failed, on the event ring, dropping (and
    /// counting) what does not fit.
    fn post_stream_events(&mut self) {
        let Self {
            runtime,
            events,
            shared,
            ..
        } = self;
        shared
            .underruns
            .store(runtime.underrun_count, Ordering::Relaxed);
        let underruns = runtime.underruns.iter();
        let failures = runtime.failures.iter();
        let stream_events = underruns
            .map(|&clip| EngineEvent::StreamUnderrun { clip })
            .chain(failures.map(|&clip| EngineEvent::StreamFailed { clip }));
        for event in stream_events {
            post(events, shared, event);
        }
    }

    /// Copies `master`, the master's output of the piece just rendered, into the record tap;
    /// frames it has no room for are counted.
    fn tap_frames(&mut self, master: &[[f32; 2]]) {
        let room = self.tap.slots() / 2;
        let kept = master.len().min(room);
        for frame in &master[..kept] {
            // Room for both values was checked above.
            let _ = self.tap.push(frame[0]);
            let _ = self.tap.push(frame[1]);
        }
        let dropped = (master.len() - kept) as u64;
        if dropped > 0 {
            self.shared
                .tap_dropped
                .fetch_add(dropped, Ordering::Release);
        }
    }

    /// Applies every waiting control, in the order sent. [`process`](Self::process) calls
    /// this first; call it directly only to act on controls without rendering.
    pub fn drain_controls(&mut self) {
        while let Ok(control) = self.controls.pop() {
            match control {
                Control::Param(target, value) => self.set_param(target, value),
                Control::Suspend => {
                    self.end_recording();
                    self.transport.apply(control);
                    self.runtime.silence();
                    self.plan.mixer.settle();
                    self.runtime.release_automation(&self.plan);
                    self.runtime.suspend_all();
                    self.midi.jump(&mut self.plan, self.output_frames);
                    self.shared.suspends.fetch_add(1, Ordering::AcqRel);
                }
                Control::StopPreview => self.stop_preview(),
                Control::Metronome(on) => {
                    self.metronome = on;
                    if !on {
                        self.last_click = None;
                    }
                }
                Control::LiveNoteOn { key, velocity } => {
                    self.live_note_on(Input::CONTROLS, None, key, velocity, true);
                }
                Control::ChannelNoteOn {
                    channel,
                    key,
                    velocity,
                } => {
                    self.live_note_on(Input::CONTROLS, Some(channel), key, velocity, true);
                }
                Control::LiveNoteOff { key } => self.live_note_off(Input::CONTROLS, key),
                Control::LiveControl { channel, control } => {
                    self.live_control(Input::CONTROLS, channel, control, true);
                }
                Control::LiveNotesOff => self.end_every_live_note(),
                Control::Record {
                    count_in,
                    tap_master,
                } => self.record(count_in, tap_master),
                Control::EndRecording => self.end_recording(),
                // No jump, but the MIDI-out notes sent ahead were placed by the old tempo.
                Control::SetTempo(_) => {
                    let before = self.course();
                    self.transport.apply(control);
                    if let (Some(before), Some(after)) = (before, self.course()) {
                        if before.timeline.tempo() != after.timeline.tempo() {
                            midi_out::retimed(&mut self.plan, &before, &after);
                        }
                    }
                }
                // A jump (Play, Stop, Seek) leaves nothing ringing from the old position: no
                // voice, no effect tail, no ramp half way, no automated parameter held. Stop ends
                // the live notes too, so the jump's reset strikes no held key again: a key whose
                // release never arrived (2026-10-28: a plugin note that came back on every Play)
                // stops for good at Stop, as every note does.
                other => {
                    if matches!(other, Control::Stop) {
                        self.end_recording();
                        self.end_every_live_note();
                    }
                    if self.transport.apply(other) {
                        self.jumped(self.output_frames);
                    }
                }
            }
        }
    }

    /// Ends every live note, as all notes off asks: every pedal lifts first, so the keys it
    /// held end now; then every input's keys, in key order; then the live voices are silenced.
    fn end_every_live_note(&mut self) {
        self.lift_every_pedal();
        for key in 0..=MAX_KEY {
            while let Some(input) = self.held.input_holding(key) {
                self.end_note(input, key);
            }
        }
        self.runtime.live_silence();
    }

    /// Takes every note waiting in the MIDI inbox, in the order received.
    fn drain_inbox(&mut self) {
        while let Some((input, delivery)) = self.inbox.notes.pop() {
            match delivery {
                Delivery::Note {
                    channel,
                    note: LiveNote::On { key, velocity },
                    out,
                } => self.live_note_on(input, channel, key, velocity, out),
                Delivery::Note {
                    note: LiveNote::Off { key },
                    ..
                } => self.live_note_off(input, key),
                Delivery::Control {
                    channel,
                    control,
                    out,
                } => self.live_control(input, channel, control, out),
                Delivery::Release => {
                    self.lift(input, 0);
                    while let Some(key) = self.held.key_of(input) {
                        self.end_note(input, key);
                    }
                    self.rest_controls(input);
                }
            }
        }
    }

    /// The tick a live note is heard at: the playhead tick of the call's first sample while the
    /// song timeline runs; none while stopped or counting in.
    fn live_tick(&self) -> Option<Ticks> {
        (self.transport.playing() && self.count_in.is_none())
            .then(|| self.transport.playhead_tick())
    }

    /// Starts `input`'s live note from the call's first frame on `channel`, or on the plan's
    /// live channel when that is `None` or a channel the plan does not have. A key the input
    /// holds already is ended first, held down or by its pedal (struck again, it starts anew);
    /// so is another input's note of the key on that channel, since a channel sounds a key once
    /// (that input's note-off then ends nothing). Another input's note of the key on another
    /// channel plays on. A key past 127 or a velocity outside 0 to 1 is ignored. When `out`, a
    /// channel with a MIDI out that is not muted sends its note on at the call's first frame
    /// (its note-off goes the same way).
    fn live_note_on(
        &mut self,
        input: Input,
        channel: Option<ChannelId>,
        key: u8,
        velocity: f32,
        out: bool,
    ) {
        if key > MAX_KEY || !(0.0..=1.0).contains(&velocity) {
            return;
        }
        self.end_note(input, key);
        let channel = channel.filter(|&id| self.plan.channel(id).is_some());
        let Some(channel) = channel.or(self.plan.live_channel) else {
            return;
        };
        if let Some(other) = self.held.holder(channel, key) {
            self.end_note(other, key);
        }
        let controls = self
            .expressions
            .get(input, channel)
            .map_or(Controls::default(), |expression| expression.controls());
        // The sequence's voices are timed by the transport's clock while the song plays; a
        // note on a channel that cuts itself cuts them there too.
        let sequence = self.transport.playing().then(|| self.transport.clock());
        if !self.runtime.live_on(
            &self.plan,
            input.0,
            channel,
            key,
            velocity,
            self.output_frames,
            sequence,
            controls,
        ) {
            // No hold free: dropped and counted.
            return;
        }
        let sent = self
            .plan
            .channel(channel)
            .filter(|target| out && !target.muted)
            .and_then(|target| target.midi_out)
            .map(|midi_out| midi_out.status_channel)
            .filter(|&status| {
                let on = MidiOutEvent::note_on(self.output_frames, channel, status, key, velocity);
                self.midi.live(on)
            });
        let held = self.held.hold(HeldKey {
            input,
            key,
            channel,
            out: sent,
            let_go: None,
        });
        debug_assert!(
            held,
            "the runtime holds the same keys, so this table had room"
        );
        let tick = self.live_tick();
        self.post(EngineEvent::LiveNoteStarted {
            channel,
            key,
            velocity,
            tick,
            frame: self.output_frames,
        });
    }

    /// `input` let go of `key`: its live note ends on the channel it started on, or, while the
    /// input's pedal is down there, sounds on until the pedal lifts. Nothing when the input
    /// holds no such key down.
    fn live_note_off(&mut self, input: Input, key: u8) {
        let Some(held) = self.held.down(input, key) else {
            return;
        };
        if self
            .expressions
            .get(input, held.channel)
            .is_some_and(|expression| expression.holds())
        {
            self.let_go = self.let_go.wrapping_add(1);
            self.held.let_go(input, key, self.let_go);
            return;
        }
        self.end_note(input, key);
    }

    /// Ends `input`'s live note of `key` on the channel it started on, if it holds one, down or
    /// by its pedal.
    fn end_note(&mut self, input: Input, key: u8) {
        let Some(HeldKey { channel, out, .. }) = self.held.take(input, key) else {
            return;
        };
        if let Some(status) = out {
            let off = MidiOutEvent::note_off(self.output_frames, channel, status, key);
            self.midi.live(off);
        }
        self.runtime
            .live_off(&self.plan, channel, key, self.output_frames);
        let tick = self.live_tick();
        self.post(EngineEvent::LiveNoteEnded {
            channel,
            key,
            tick,
            frame: self.output_frames,
        });
    }

    fn post(&mut self, event: EngineEvent) {
        post(&mut self.events, &self.shared, event);
    }

    /// Takes `input`'s performance control for `channel`, or for the plan's live channel when
    /// that is `None` or a channel the plan does not have, from the call's first frame. When
    /// `out`, a channel with a MIDI out that is not muted sends it on as the same message, but
    /// only when it changes what the input holds there: an instrument sending back what it was
    /// sent stops there.
    fn live_control(
        &mut self,
        input: Input,
        channel: Option<ChannelId>,
        control: LiveControl,
        out: bool,
    ) {
        let channel = channel.filter(|&id| self.plan.channel(id).is_some());
        let Some(channel) = channel.or(self.plan.live_channel) else {
            return;
        };
        match control {
            LiveControl::Sustain(value) => self.sustain(input, channel, value, out),
            LiveControl::Bend(value) => self.bend(input, channel, value, out),
            LiveControl::Wheel(value) => self.wheel(input, channel, value, out),
        }
    }

    /// `input`'s mod wheel at `value` on `channel`, from the call's first frame: its live voices
    /// there open their filter or swing their vibrato, and the channel's MIDI out or plugin hears
    /// it, when it changed. First the wheel rests on every other channel the input holds it off
    /// rest ([`rest_elsewhere`](Self::rest_elsewhere)).
    fn wheel(&mut self, input: Input, channel: ChannelId, value: u8, out: bool) {
        if value > 127 {
            return;
        }
        self.rest_elsewhere(input, channel, Moved::Wheel);
        let before = self
            .expressions
            .get(input, channel)
            .map_or(0, |expression| expression.wheel);
        if value == before {
            return;
        }
        if !self.expressions.has_room_for(input, channel) {
            self.runtime.count_dropped();
            return;
        }
        let at = self.output_frames;
        let sent = self.forward(channel, out, |status| {
            MidiOutEvent::mod_wheel(at, channel, status, value)
        });
        if let Some(expression) = self.expressions.entry(input, channel) {
            expression.wheel = value;
            if sent.is_some() {
                expression.out = sent;
            }
        }
        self.runtime
            .live_wheel(&self.plan, input.0, channel, value, at);
        self.expressions.tidy();
    }

    /// `input`'s pitch bend at `value` on `channel`, from the call's first frame: its live
    /// voices there read on bent from where they are, and the channel's MIDI out or plugin hears
    /// it, when it changed. First the bend rests on every other channel the input holds it off
    /// rest ([`rest_elsewhere`](Self::rest_elsewhere)).
    fn bend(&mut self, input: Input, channel: ChannelId, value: i16, out: bool) {
        if !(-8192..=8191).contains(&value) {
            return;
        }
        self.rest_elsewhere(input, channel, Moved::Bend);
        let before = self
            .expressions
            .get(input, channel)
            .map_or(0, |expression| expression.bend);
        if value == before {
            return;
        }
        if !self.expressions.has_room_for(input, channel) {
            self.runtime.count_dropped();
            return;
        }
        let at = self.output_frames;
        let sent = self.forward(channel, out, |status| {
            MidiOutEvent::pitch_bend(at, channel, status, value)
        });
        if let Some(expression) = self.expressions.entry(input, channel) {
            expression.bend = value;
            if sent.is_some() {
                expression.out = sent;
            }
        }
        self.runtime
            .live_bend(&self.plan, input.0, channel, value, at);
        self.expressions.tidy();
    }

    /// `input`'s bend or wheel (`moved`) moved on `channel`: it rests (centered, at 0) on every
    /// other channel the input holds it off rest, from this frame: the live voices there read on
    /// unbent or unswung, the plugin and the MIDI out it went to are sent the rest.
    ///
    /// Why: a device's controls go to the channel its notes play now, and that moves with the
    /// selection. A wheel left up on the channel left behind would swing every later note there
    /// while the device's wheel is at rest, with nothing left to move it back; the pedal's lift
    /// lets go on every channel for the same reason. One channel holds a device's bend and wheel
    /// at a time: the one it moved them on last.
    fn rest_elsewhere(&mut self, input: Input, channel: ChannelId, moved: Moved) {
        let at = self.output_frames;
        for slot in &mut self.expressions.0 {
            let Some(expression) = slot.as_mut() else {
                continue;
            };
            if expression.input != input || expression.channel == channel {
                continue;
            }
            let other = expression.channel;
            match moved {
                Moved::Bend if expression.bend != 0 => {
                    expression.bend = 0;
                    if let Some(status) = expression.out {
                        self.midi
                            .live(MidiOutEvent::pitch_bend(at, other, status, 0));
                    }
                    self.runtime.live_bend(&self.plan, input.0, other, 0, at);
                }
                Moved::Wheel if expression.wheel != 0 => {
                    expression.wheel = 0;
                    if let Some(status) = expression.out {
                        self.midi
                            .live(MidiOutEvent::mod_wheel(at, other, status, 0));
                    }
                    self.runtime.live_wheel(&self.plan, input.0, other, 0, at);
                }
                Moved::Bend | Moved::Wheel => {}
            }
        }
        self.expressions.tidy();
    }

    /// `input` is gone: its bend and its wheel go back to rest wherever they are off it (sent on
    /// to the MIDI outs they went to, and to the plugins), and its entries go.
    fn rest_controls(&mut self, input: Input) {
        let at = self.output_frames;
        for slot in &mut self.expressions.0 {
            let Some(expression) = slot.filter(|expression| expression.input == input) else {
                continue;
            };
            *slot = None;
            let channel = expression.channel;
            if expression.bend != 0 {
                if let Some(status) = expression.out {
                    self.midi
                        .live(MidiOutEvent::pitch_bend(at, channel, status, 0));
                }
                self.runtime.live_bend(&self.plan, input.0, channel, 0, at);
            }
            if expression.wheel != 0 {
                if let Some(status) = expression.out {
                    self.midi
                        .live(MidiOutEvent::mod_wheel(at, channel, status, 0));
                }
                self.runtime.live_wheel(&self.plan, input.0, channel, 0, at);
            }
        }
    }

    /// `input`'s pedal at `value` on `channel`. Let go of (below 64), it lifts on every channel
    /// the input's pedal is down on, not only this one: the selection may have moved since it
    /// went down, and a pedal that never lifted would hold its notes for good.
    fn sustain(&mut self, input: Input, channel: ChannelId, value: u8, out: bool) {
        if value > 127 {
            return;
        }
        if value < PEDAL_DOWN {
            self.lift(input, value);
        }
        let before = self
            .expressions
            .get(input, channel)
            .map_or(0, |expression| expression.sustain);
        if value == before {
            return;
        }
        if !self.expressions.has_room_for(input, channel) {
            self.runtime.count_dropped();
            return;
        }
        let at = self.output_frames;
        let sent = self.forward(channel, out, |status| {
            MidiOutEvent::sustain(at, channel, status, value)
        });
        if let Some(expression) = self.expressions.entry(input, channel) {
            expression.sustain = value;
            if sent.is_some() {
                expression.out = sent;
            }
        }
        self.expressions.tidy();
    }

    /// Lifts `input`'s pedal to `value` (below 64) on every channel it is down on, each sending
    /// the lift to the MIDI out its pedal went to, then ends the keys it held, in the order they
    /// were let go of.
    fn lift(&mut self, input: Input, value: u8) {
        let at = self.output_frames;
        for slot in &mut self.expressions.0 {
            let Some(expression) = slot.as_mut() else {
                continue;
            };
            if expression.input != input || !expression.holds() {
                continue;
            }
            if let Some(status) = expression.out {
                self.midi
                    .live(MidiOutEvent::sustain(at, expression.channel, status, value));
            }
            expression.sustain = value;
        }
        self.expressions.tidy();
        while let Some(key) = self.held.first_let_go(input) {
            self.end_note(input, key);
        }
    }

    /// Every input's controls go back to rest (sent where they went), as all notes off asks:
    /// every pedal lifts, every bend centers and every wheel goes to 0; the keys the pedals held are ended by the
    /// caller, and the live voices are silenced after.
    fn lift_every_pedal(&mut self) {
        let at = self.output_frames;
        for slot in &mut self.expressions.0 {
            let Some(expression) = slot.take() else {
                continue;
            };
            // A plugin keeps the bend and the wheel it was sent until it is sent others.
            let input = expression.input.0;
            if expression.bend != 0 {
                self.runtime
                    .live_bend(&self.plan, input, expression.channel, 0, at);
            }
            if expression.wheel != 0 {
                self.runtime
                    .live_wheel(&self.plan, input, expression.channel, 0, at);
            }
            let Some(status) = expression.out else {
                continue;
            };
            if expression.sustain != 0 {
                self.midi
                    .live(MidiOutEvent::sustain(at, expression.channel, status, 0));
            }
            if expression.bend != 0 {
                self.midi
                    .live(MidiOutEvent::pitch_bend(at, expression.channel, status, 0));
            }
            if expression.wheel != 0 {
                self.midi
                    .live(MidiOutEvent::mod_wheel(at, expression.channel, status, 0));
            }
        }
    }

    /// Sends the message `make` makes for its MIDI channel (0 to 15) on `channel`'s MIDI out
    /// now, when `out` and the channel has one and is not muted; the MIDI channel it went on.
    fn forward(
        &mut self,
        channel: ChannelId,
        out: bool,
        make: impl FnOnce(u8) -> MidiOutEvent,
    ) -> Option<u8> {
        let status = self
            .plan
            .channel(channel)
            .filter(|target| out && !target.muted)
            .and_then(|target| target.midi_out)?
            .status_channel;
        self.midi.live(make(status));
        Some(status)
    }

    /// After the playhead jumped on output frame `at`: silence what rings (but for the keys
    /// still held), end the notes sent to MIDI outs there, settle ramps, start automation
    /// afresh, and while stopped get every streamed clip ready at the play-from position.
    fn jumped(&mut self, at: u64) {
        self.runtime.silence();
        self.midi.jump(&mut self.plan, at);
        self.plan.reset_round_robins();
        self.runtime.strike_held(&self.plan);
        self.plan.mixer.settle();
        self.runtime.release_automation(&self.plan);
        self.control_due = true;
        if !self.transport.playing() {
            self.runtime.prepare_sources(
                &self.plan,
                self.transport.timeline(),
                self.transport.position(),
            );
        }
    }

    /// Starts recording: after a bar of count-in when `count_in`, else at once.
    fn record(&mut self, count_in: bool, tap_master: bool) {
        if self.transport.playing() || self.count_in.is_some() {
            return;
        }
        self.recording = Some(Recording { tap_master });
        self.shared.tap_dropped.store(0, Ordering::Release);
        if !count_in {
            self.start_song(self.output_frames);
            return;
        }
        let timeline = self.transport.timeline();
        let tempo = timeline.tempo();
        let (rate, ppq) = (self.plan.sample_rate, self.plan.ppq);
        let at = |tick: Ticks| time::tick_to_sample(tick, rate, tempo, ppq).unwrap_or(0);
        let beat = time::step_to_ticks(time::STEPS_PER_BEAT.into(), ppq);
        let mut beats = [0; 4];
        for (index, start) in (0..).zip(beats.iter_mut()) {
            *start = at(Ticks(beat.0 * index));
        }
        self.count_in = Some(CountIn {
            frames: at(time::bars_to_ticks(1, ppq)),
            done: 0,
            beats,
        });
    }

    /// Stops a recording (and its count-in).
    fn end_recording(&mut self) {
        self.count_in = None;
        self.recording = None;
        self.shared
            .recording_from
            .store(NOT_RECORDING, Ordering::Release);
    }

    /// Takes the newest waiting preview in place of the one playing, which goes back to the UI
    /// thread; only while the preview retire ring has room for it.
    fn take_preview(&mut self) {
        while !self.retire_previews.is_full() {
            let Ok(source) = self.previews.pop() else {
                return;
            };
            let new = Preview { source, frame: 0 };
            if let Some(old) = self.runtime.preview.replace(new) {
                if let Err(PushError::Full(old)) = self.retire_previews.push(old.source) {
                    // Unreachable: the ring had room above. Leaking beats freeing here.
                    mem::forget(old);
                }
            }
        }
    }

    /// Takes the newest waiting set of input feeds in place of the one it reads, which goes back
    /// to the UI thread; only while the retire ring has room for it.
    fn take_inputs(&mut self) {
        while !self.retire_inputs.is_full() {
            let Ok(feeds) = self.inputs.pop() else {
                return;
            };
            if let Some(old) = self.runtime.inputs.replace(feeds) {
                if let Err(PushError::Full(old)) = self.retire_inputs.push(old) {
                    // Unreachable: the ring had room above. Leaking beats freeing here.
                    mem::forget(old);
                }
            }
        }
    }

    /// Sends the preview back to the UI thread, if the retire ring has room.
    fn stop_preview(&mut self) {
        if self.retire_previews.is_full() {
            return;
        }
        if let Some(preview) = self.runtime.preview.take() {
            if let Err(PushError::Full(source)) = self.retire_previews.push(preview.source) {
                mem::forget(source);
            }
        }
    }

    fn retire_finished_preview(&mut self) {
        if self.runtime.preview.as_ref().is_some_and(Preview::finished) {
            self.stop_preview();
        }
    }

    /// Suspends every processor the player runs. An offline render calls this on its render
    /// thread when it is done, before the player goes back to be dropped where it was made, so
    /// every plugin's last audio-thread call comes from the thread that processed it.
    pub fn suspend_processors(&mut self) {
        self.runtime.suspend_all();
    }

    /// Frames put out since the link was made: the count [`RecordClock::recording_from`] is in.
    /// An output stream reads it as it starts, to time the frames it plays.
    pub fn output_frames(&self) -> u64 {
        self.output_frames
    }

    /// The absolute sample where `tick` falls at the tempo in force, or `None` for a tick
    /// before the latest tempo change. An offline render uses it to stop exactly on a bar line.
    pub fn tick_to_sample(&self, tick: Ticks) -> Option<u64> {
        self.transport.timeline().tick_to_sample(tick)
    }

    /// Whether an automation governs `target` in the block just rendered: then a live value
    /// for it is ignored.
    fn automated(&self, target: ParamTarget) -> bool {
        self.plan
            .automation
            .iter()
            .enumerate()
            .take(MAX_AUTOMATIONS)
            .any(|(index, automation)| {
                automation.target == Some(target) && self.runtime.governs(index)
            })
    }

    /// Sets a live value in the current plan or a running processor. While stopped, nothing is
    /// heard moving, so faders, pans and sends jump to it instead of ramping.
    fn set_param(&mut self, target: ParamTarget, value: f32) {
        if !value.is_finite() || self.automated(target) {
            return;
        }
        let at_once = !self.transport.playing();
        let clamp =
            |range: std::ops::RangeInclusive<f32>| value.clamp(*range.start(), *range.end());
        let mixer = &mut self.plan.mixer;
        match target {
            ParamTarget::ChannelVolume(id) => {
                if let Some(channel) = self.plan.channel_mut(id) {
                    channel.volume = value.clamp(0.0, MAX_VOLUME);
                    channel.gain = if channel.muted { 0.0 } else { channel.volume };
                }
            }
            ParamTarget::ChannelPan(id) => {
                if let Some(channel) = self.plan.channel_mut(id) {
                    channel.pan = clamp(PAN_RANGE);
                    channel.pan_gains = channel_pan_gains(channel.pan);
                }
            }
            ParamTarget::InsertFader(id) => {
                if let Some(insert) = mixer.insert_mut(id) {
                    insert
                        .fader
                        .retarget(fader_gain(clamp(FADER_RANGE)), at_once);
                }
            }
            ParamTarget::InsertPan(id) => {
                if let Some(insert) = mixer.insert_mut(id) {
                    insert.pan.retarget(clamp(PAN_RANGE), at_once);
                }
            }
            ParamTarget::Send { from, to } => {
                if let Some(send) = mixer
                    .insert_mut(from)
                    .and_then(|insert| insert.send_mut(to))
                {
                    send.amount.retarget(clamp(SEND_RANGE), at_once);
                }
            }
            ParamTarget::Effect { .. } | ParamTarget::Plugin { .. } => {
                self.runtime.set_processor_param(&self.plan, target, value);
            }
        }
    }

    /// Applies the next waiting update: its processor and source changes, then its plan, whose
    /// faders, pans and sends carry on from the old plan's (ramping to changed values while
    /// playing). The old plan and what was taken out go back on the retire ring.
    fn swap_plan(&mut self) {
        // Why only when the retire ring has room: the old plan must leave through it, because
        // dropping it here would free memory on the audio thread.
        if self.retire.is_full() {
            return;
        }
        let Ok(mut update) = self.plans.pop() else {
            return;
        };
        self.runtime.apply(&mut update.delta);
        if self.transport.playing() {
            update.plan.mixer.carry_ramps(&self.plan.mixer);
        }
        update.plan.carry_round_robins(&self.plan);
        let was = self.course();
        mem::swap(&mut self.plan, &mut update.plan);
        let courses = was.zip(self.course());
        self.midi
            .swapped(&update.plan, &mut self.plan, self.output_frames, courses);
        // The new plan's processors may hold other values than the automation last sent.
        self.runtime.resend_automation();
        self.control_due = true;
        if let Err(PushError::Full(update)) = self.retire.push(update) {
            // Unreachable: this thread is the ring's only producer and saw a free slot above,
            // and the consumer can only add room. Leaking is still better than freeing here.
            mem::forget(update);
        }
    }

    fn publish(&self) {
        let ticks = u64::try_from(self.transport.playhead_tick().0).unwrap_or(0);
        self.shared
            .playing
            .store(self.transport.playing(), Ordering::Release);
        self.shared
            .position_samples
            .store(self.transport.position(), Ordering::Release);
        self.shared.position_ticks.store(ticks, Ordering::Release);
        self.shared
            .dropped_notes
            .store(self.runtime.dropped_notes(), Ordering::Relaxed);
        self.shared
            .counting_in
            .store(self.count_in.is_some(), Ordering::Relaxed);
        let cells = self.shared.automation_values.iter();
        for (index, (automation, cell)) in self.plan.automation.iter().zip(cells).enumerate() {
            let value = self.runtime.automation_value(index).to_bits();
            cell.store(
                u64::from(automation.id.0) << 32 | u64::from(value),
                Ordering::Relaxed,
            );
        }
    }
}
