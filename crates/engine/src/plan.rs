//! The compiled form of a project: what the player reads while it renders, and the update that
//! carries a new plan and the processor changes that go with it to the audio thread.

use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::dsp::instrument::Instrument;
use crate::dsp::sampler::SampleBuffer;
use crate::dsp::shape::Shape;
use crate::processor::Processor;
use crate::project::{
    AutomationId, AutomationPoint, ChannelId, ClipId, EffectId, InsertId, MidiOut, PatternId,
    ProcessorOwner, EFFECT_SLOTS, MAX_INSERTS,
};
use crate::source::{ClipSource, SourceSlot};
use crate::time::Ticks;
use crate::transport::ParamTarget;

/// What a plan plays. Playback state, like Play and Stop: it is never saved or undone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayMode {
    /// Loop one pattern.
    Pattern(PatternId),
    /// Play the arrangement: the playlist's clips, looping over the loop range while it is on,
    /// else wrapping from the song's end to its start.
    Song,
    /// Play nothing of the document: no pattern, no clip, no MIDI out's notes and no input
    /// channel, while the transport runs on (a recording's clock, previews and live notes as
    /// ever). What an input latency measurement plays under its clicks.
    Nothing,
}

/// A compiled song: each channel's sound and gain, every pattern as a sorted timeline of note
/// events, the arrangement, and the mixer in the order it is mixed. Only
/// [`crate::compile::Compiler`] builds one, so the player can trust its order and ranges.
/// Afterwards only the player changes it, and only its live values: the gains a parameter
/// message sets until the next plan, and the ramps that move them. Tempo is not part of a plan:
/// the player's transport owns it, so a tempo change needs no new plan.
#[derive(Debug)]
pub struct PlayPlan {
    pub(crate) sample_rate: u32,
    pub(crate) ppq: u16,
    /// Channels in ID order (rack order).
    pub(crate) channels: Vec<ChannelPlan>,
    /// Patterns in ID order.
    pub(crate) patterns: Vec<PatternPlan>,
    pub(crate) playing: Playing,
    pub(crate) song: SongPlan,
    pub(crate) mixer: MixerPlan,
    /// Every automation, by ID (the order the player's readback cells follow).
    pub(crate) automation: Vec<AutomationPlan>,
    /// Every tick where an automation window starts or ends, or a point falls inside one,
    /// sorted: song playback is cut there, so a value that changes on one is heard from its
    /// exact sample however a render is split.
    pub(crate) automation_edges: Vec<Ticks>,
    /// Whether an effect or plugin parameter is automated: then song playback is also cut on
    /// every [`CONTROL_GRID`]th sample, where those values are set.
    pub(crate) control_rate: bool,
    /// The channel live notes play: the one selected in the rack. A new plan moves it; a held
    /// note stays on the channel it started on.
    pub(crate) live_channel: Option<ChannelId>,
}

/// Effect and plugin parameters under automation are set on every song sample divisible by
/// this, and where segments and windows start; between those points they hold.
///
/// Why not every sample: a processor takes a parameter value per block, and 32 samples (0.7 ms
/// at 44.1 kHz) is below what a parameter sweep can be heard to step at.
pub(crate) const CONTROL_GRID: u64 = 32;

/// One automation as playback needs it: what it sets, the range its normalized values span,
/// the windows its clips show, and its points.
#[derive(Debug)]
pub(crate) struct AutomationPlan {
    pub(crate) id: AutomationId,
    /// What it sets, resolved as a control message names it; `None` when the target is gone
    /// (an effect removed): then it is inert.
    pub(crate) target: Option<ParamTarget>,
    pub(crate) rate: Rate,
    /// The parameter's value at normalized 0 and at 1.
    pub(crate) lo: f64,
    pub(crate) hi: f64,
    /// The value the project holds, which rules before every window and while not playing a
    /// song. For a processor parameter, what it goes back to.
    pub(crate) stored: f32,
    /// The clips' windows, sorted by (start, clip ID); muted clips and clips on muted tracks
    /// are left out.
    pub(crate) windows: Vec<AutomationWindow>,
    /// `windows` indices sorted by (end, start, clip ID), for the window that ended last.
    pub(crate) by_end: Vec<u32>,
    /// Shared with every later plan until an edit touches this automation.
    pub(crate) points: Arc<[AutomationPoint]>,
}

/// How often an automated value is worked out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rate {
    /// Every sample, in the mixing loops: channel volume and pan, insert fader and pan, sends.
    Audio,
    /// On the [`CONTROL_GRID`] and on segment and window edges: processor parameters.
    Control,
}

/// One automation clip: song ticks `start..end` show the points from tick `offset` on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AutomationWindow {
    pub(crate) start: Ticks,
    pub(crate) end: Ticks,
    pub(crate) offset: Ticks,
    pub(crate) clip: ClipId,
}

/// What the player renders from a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Playing {
    /// Loop the pattern at this index into `PlayPlan::patterns`.
    Pattern(usize),
    /// Play `PlayPlan::song`.
    Song,
    /// A pattern the project no longer has: silence.
    Nothing,
}

/// The arrangement as playback needs it.
#[derive(Debug, Default)]
pub(crate) struct SongPlan {
    /// The clips that play, sorted by (start, clip ID). Muted clips and clips on muted tracks
    /// are left out.
    pub(crate) clips: Vec<ClipPlan>,
    /// The length of the longest clip in `clips`: every clip that sounds at tick t starts after
    /// t - longest, so two binary searches find them.
    pub(crate) longest: Ticks,
    /// Where the song ends: the latest end of any clip, muted or not. 0 without clips.
    pub(crate) end: Ticks,
    /// The loop range (start, end), only while the loop is on.
    pub(crate) loop_range: Option<(Ticks, Ticks)>,
    /// The audio clips that play, sorted by (start, clip ID), muted ones left out.
    pub(crate) audio: Vec<AudioClipPlan>,
    /// The length of the longest clip in `audio`.
    pub(crate) audio_longest: Ticks,
}

/// One audio clip: song sample n in `s(start)..s(end)` plays file frame
/// `frame_offset + n − s(start)` of its source, at `gain`, through its channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AudioClipPlan {
    pub(crate) clip: ClipId,
    pub(crate) start: Ticks,
    pub(crate) end: Ticks,
    pub(crate) channel: ChannelId,
    /// `None` while the file loads, or when it cannot be read: the clip is silent.
    pub(crate) source: Option<SourceSlot>,
    pub(crate) frame_offset: u64,
    pub(crate) gain: f32,
}

/// One clip: song ticks `start..end` play its pattern from data tick `offset` on, repeating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClipPlan {
    pub(crate) start: Ticks,
    pub(crate) end: Ticks,
    /// Below the pattern's length (the window repeats every pattern length anyway), so the
    /// arithmetic on it stays small.
    pub(crate) offset: Ticks,
    /// Index into `PlayPlan::patterns`. Clips share the pattern's event list.
    pub(crate) pattern: usize,
}

impl SongPlan {
    /// Where song playback jumps, as (from tick, to tick): from the loop end to the loop start
    /// while the loop is on, else from the song end to tick 0. An empty song never wraps.
    pub(crate) fn wrap(&self) -> Option<(Ticks, Ticks)> {
        match self.loop_range {
            Some((start, end)) => Some((end, start)),
            None => (self.end > Ticks(0)).then_some((self.end, Ticks(0))),
        }
    }
}

/// A channel as playback needs it.
#[derive(Debug)]
pub(crate) struct ChannelPlan {
    pub(crate) id: ChannelId,
    pub(crate) sound: ChannelSound,
    /// The channel volume, or 0 when muted. A live volume change sets it until the next plan.
    pub(crate) gain: f32,
    /// The channel volume whether muted or not, which live notes on a sampler play at (mute
    /// silences the sequence, not what is played live). Set with `gain`.
    pub(crate) volume: f32,
    pub(crate) muted: bool,
    /// Whether live notes play it: a sampler, a plugin or an instrument channel. An audio
    /// channel's sound is its clips, an input channel's its input.
    pub(crate) plays_live: bool,
    /// The insert the channel plays into.
    pub(crate) bus: InsertId,
    /// Left and right gains of its pan. A live pan change sets them until the next plan.
    pub(crate) pan_gains: [f32; 2],
    /// The pan position, for stereo sounds (the balance law). Set with `pan_gains`.
    pub(crate) pan: f32,
    /// The automations of its volume and pan, as indices into `PlayPlan::automation`.
    pub(crate) volume_automation: Option<u16>,
    pub(crate) pan_automation: Option<u16>,
    /// An input channel's MIDI out, as the player sends its notes; `None` when it has none.
    pub(crate) midi_out: Option<MidiOutPlan>,
    /// Semitones a device's pitch bend reaches on its live notes, all the way up or down.
    pub(crate) bend_range: u8,
    /// The low-pass filter's cutoff and resonance while its sound settings have it on: then a
    /// device's mod wheel opens it on the live notes; without, the wheel swings a vibrato.
    pub(crate) wheel_filter: Option<[f32; 2]>,
    /// Whether a note's start cuts the channel's notes still sounding (its sound settings'
    /// `cut_itself`): its voices of the sequence and the live ones alike.
    pub(crate) cut_itself: bool,
}

/// An input channel's MIDI out as the player sends it. The port is not here: the thread that
/// sends the messages finds it by the channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MidiOutPlan {
    /// The MIDI channel as a status byte holds it, 0 to 15.
    pub(crate) status_channel: u8,
    /// How many frames ahead of their natural frame its notes are sent
    /// ([`crate::project::MidiOut::latency_frames`]).
    pub(crate) latency_frames: u64,
    /// The player's own: every note of the channel whose natural frame (the output frame its
    /// tick plays on) is before this has been sent. Carried to the next plan while the MIDI
    /// channel and latency stay; 0 in a new plan.
    pub(crate) sent_until: u64,
}

impl MidiOutPlan {
    pub(crate) fn new(midi_out: &MidiOut, sample_rate: u32) -> Self {
        Self {
            status_channel: midi_out.channel.saturating_sub(1).min(15),
            latency_frames: midi_out.latency_frames(sample_rate),
            sent_until: 0,
        }
    }

    /// Whether `other` sends on the same MIDI channel with the same latency.
    pub(crate) fn sends_as(&self, other: &MidiOutPlan) -> bool {
        (self.status_channel, self.latency_frames) == (other.status_channel, other.latency_frames)
    }
}

/// What makes a channel's sound in a plan.
#[derive(Debug)]
pub(crate) enum ChannelSound {
    /// The decoded sound, shared with the compiler's cache and with other plans.
    Sampler(SampleBuffer),
    /// The plugin instrument in this slot of the player's processor table, sent the channel's
    /// notes.
    Plugin(ProcessorSlot),
    /// An audio channel with no notes: only its audio clips sound, so its file is not decoded
    /// for voices (a long one streams).
    Clips,
    /// An instrument set, its zones shared with the compiler's cache and with other plans.
    Instrument(InstrumentSound),
    /// A sampler whose sound settings are not the defaults: its sound and how its voices play
    /// it. A sampler at the defaults is [`ChannelSound::Sampler`], played as before the
    /// settings existed.
    Shaped(ShapedSample),
    /// An input channel: the frames of the player's input feed for it
    /// ([`crate::input::InputFeeds`]), silence while it has none. Notes start nothing on it.
    Input,
}

/// A sampler's sound and its sound settings as its voices play them.
#[derive(Debug)]
pub(crate) struct ShapedSample {
    pub(crate) sample: SampleBuffer,
    pub(crate) shape: Shape,
}

/// An instrument channel's set and its round-robin counters: how many notes each round-robin
/// zone has heard since Play.
///
/// Why atomics in an immutable plan: the player counts notes through a shared reference (its
/// live notes start from one), and a relaxed atomic is a plain load and store, with no lock
/// and no allocation. Each plan gets its own counters, zeroed; the player carries them over
/// from the plan it replaces while the set is the same, and zeroes them on Play, Stop and Seek.
#[derive(Debug)]
pub(crate) struct InstrumentSound {
    pub(crate) instrument: Instrument,
    pub(crate) counters: Box<[AtomicU32]>,
    /// The channel's sound settings as its zones play them; `None` at the defaults.
    pub(crate) shape: Option<Shape>,
}

impl InstrumentSound {
    pub(crate) fn new(instrument: Instrument) -> Self {
        let counters = (0..instrument.counters())
            .map(|_| AtomicU32::new(0))
            .collect();
        Self {
            instrument,
            counters,
            shape: None,
        }
    }

    /// The set played with the sound settings `shape`.
    pub(crate) fn with_shape(self, shape: Option<Shape>) -> Self {
        Self { shape, ..self }
    }

    /// Starts every count again from zero.
    pub(crate) fn reset(&self) {
        for counter in self.counters.iter() {
            counter.store(0, Ordering::Relaxed);
        }
    }

    /// Takes the counts of `old` when it plays the same set.
    pub(crate) fn carry(&self, old: &InstrumentSound) {
        if self.instrument.same(&old.instrument) {
            for (counter, before) in self.counters.iter().zip(old.counters.iter()) {
                counter.store(before.load(Ordering::Relaxed), Ordering::Relaxed);
            }
        }
    }
}

/// Index of a running processor in the player's processor table. The compiler hands slots out,
/// and takes one back only once the update that removed its processor has returned from the
/// audio thread, so a slot never holds two processors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcessorSlot(pub(crate) u16);

impl ProcessorSlot {
    pub(crate) fn index(self) -> usize {
        usize::from(self.0)
    }
}

/// How many effects can run at once: every slot of the master and of the most inserts a project
/// can have. They take the table's first slots.
pub(crate) const PROCESSOR_SLOTS: usize = (MAX_INSERTS as usize + 1) * EFFECT_SLOTS as usize;

/// How many plugin channels can play at once. Their instruments take the slots after the
/// effects', so a mixer full of effects never leaves an instrument without one.
pub(crate) const PLUGIN_CHANNELS: usize = 1000;

/// The processor table's size: the effects' slots, then the plugin channels'.
pub(crate) const TABLE_SLOTS: usize = PROCESSOR_SLOTS + PLUGIN_CHANNELS;

/// Frames a fader, pan or send takes to reach a new value.
pub(crate) const RAMP_FRAMES: u16 = 256;

/// A gain that moves to a new value in a straight line over [`RAMP_FRAMES`] frames, counted
/// frame by frame from where the change was heard, so a jump never clicks. Once there it is
/// exactly the new value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Ramp {
    from: f32,
    to: f32,
    /// Frames of the ramp already played; [`RAMP_FRAMES`] once settled.
    done: u16,
}

impl Ramp {
    pub(crate) fn settled(value: f32) -> Self {
        Self {
            from: value,
            to: value,
            done: RAMP_FRAMES,
        }
    }

    pub(crate) fn is_settled(&self) -> bool {
        self.done >= RAMP_FRAMES
    }

    /// The value it is heading for (the value itself, once settled).
    pub(crate) fn target(&self) -> f32 {
        self.to
    }

    /// The value `ahead` frames from now: the old value on the first frame of a ramp, the new
    /// one exactly from frame [`RAMP_FRAMES`] on.
    pub(crate) fn at(&self, ahead: usize) -> f32 {
        let step = usize::from(self.done) + ahead;
        if step >= usize::from(RAMP_FRAMES) {
            self.to
        } else {
            self.from + (self.to - self.from) * (step as f32 / f32::from(RAMP_FRAMES))
        }
    }

    pub(crate) fn advance(&mut self, frames: usize) {
        let done = usize::from(self.done).saturating_add(frames);
        self.done = u16::try_from(done.min(usize::from(RAMP_FRAMES))).unwrap_or(RAMP_FRAMES);
    }

    /// This ramp heading for `to` from where it is now; a ramp already heading there goes on.
    pub(crate) fn toward(self, to: f32) -> Self {
        if to == self.to {
            return self;
        }
        Self {
            from: self.at(0),
            to,
            done: 0,
        }
    }

    /// Heads for `to`, or jumps there when `at_once` (while stopped nothing is heard moving).
    pub(crate) fn retarget(&mut self, to: f32, at_once: bool) {
        *self = if at_once {
            Self::settled(to)
        } else {
            self.toward(to)
        };
    }
}

/// The mixer as playback needs it.
#[derive(Debug, Default)]
pub(crate) struct MixerPlan {
    /// Every insert, each after every insert that sends to it, the master last.
    pub(crate) order: Vec<InsertPlan>,
    /// Each insert's index in `order`, by insert ID.
    pub(crate) place: Vec<usize>,
    /// Every running effect's slot, sorted by effect ID, for live parameter changes.
    pub(crate) effects: Vec<(EffectId, ProcessorSlot)>,
}

/// One insert as playback needs it.
#[derive(Debug)]
pub(crate) struct InsertPlan {
    pub(crate) id: InsertId,
    /// The fader's gain, after the fader law.
    pub(crate) fader: Ramp,
    /// The pan position.
    pub(crate) pan: Ramp,
    /// False when muted or silenced by solo. The effects still run either way, so their tails
    /// stay continuous.
    pub(crate) audible: bool,
    /// Its processors, in slot order.
    pub(crate) effects: Vec<ProcessorSlot>,
    /// Its sends, sorted by target.
    pub(crate) sends: Vec<SendPlan>,
    /// The automations of its fader and pan, as indices into `PlayPlan::automation`.
    pub(crate) fader_automation: Option<u16>,
    pub(crate) pan_automation: Option<u16>,
}

#[derive(Debug)]
pub(crate) struct SendPlan {
    pub(crate) to: InsertId,
    pub(crate) amount: Ramp,
    /// The automation of its amount, as an index into `PlayPlan::automation`.
    pub(crate) automation: Option<u16>,
}

impl MixerPlan {
    pub(crate) fn insert_mut(&mut self, id: InsertId) -> Option<&mut InsertPlan> {
        let index = *self.place.get(usize::from(id.0))?;
        self.order.get_mut(index).filter(|insert| insert.id == id)
    }

    fn insert(&self, id: InsertId) -> Option<&InsertPlan> {
        let index = *self.place.get(usize::from(id.0))?;
        self.order.get(index).filter(|insert| insert.id == id)
    }

    pub(crate) fn effect_slot(&self, id: EffectId) -> Option<ProcessorSlot> {
        let index = self.effects.binary_search_by_key(&id, |&(id, _)| id).ok()?;
        self.effects.get(index).map(|&(_, slot)| slot)
    }

    /// Starts this plan's gains from where `old`'s are now, so a fader, pan or send that changed
    /// ramps to its new value instead of jumping, and one still ramping goes on. Inserts and
    /// sends that `old` did not have start at their value.
    pub(crate) fn carry_ramps(&mut self, old: &MixerPlan) {
        for insert in &mut self.order {
            let Some(before) = old.insert(insert.id) else {
                continue;
            };
            insert.fader = before.fader.toward(insert.fader.target());
            insert.pan = before.pan.toward(insert.pan.target());
            for send in &mut insert.sends {
                if let Some(prior) = before.send(send.to) {
                    send.amount = prior.amount.toward(send.amount.target());
                }
            }
        }
    }

    /// Ends every ramp at its value.
    pub(crate) fn settle(&mut self) {
        for insert in &mut self.order {
            insert.fader = Ramp::settled(insert.fader.target());
            insert.pan = Ramp::settled(insert.pan.target());
            for send in &mut insert.sends {
                send.amount = Ramp::settled(send.amount.target());
            }
        }
    }
}

impl InsertPlan {
    fn send(&self, to: InsertId) -> Option<&SendPlan> {
        let index = self.sends.binary_search_by_key(&to, |send| send.to).ok()?;
        self.sends.get(index)
    }

    pub(crate) fn send_mut(&mut self, to: InsertId) -> Option<&mut SendPlan> {
        let index = self.sends.binary_search_by_key(&to, |send| send.to).ok()?;
        self.sends.get_mut(index)
    }
}

/// A new plan, and what the player's processor table must change before it plays it. The
/// player applies both at the start of a call and sends the update back, holding the old plan
/// and the processors it took out, so everything is dropped on the UI thread.
#[derive(Debug)]
pub struct PlanUpdate {
    pub(crate) plan: Box<PlayPlan>,
    pub(crate) delta: RuntimeDelta,
}

impl PlanUpdate {
    pub fn plan(&self) -> &PlayPlan {
        &self.plan
    }
}

/// Changes to the processor and source tables that come with a plan.
#[derive(Default)]
pub(crate) struct RuntimeDelta {
    /// New processors, created on the UI thread, with the free slots they go in.
    pub(crate) install: Vec<(ProcessorSlot, Box<dyn Processor>)>,
    /// Slots whose processors leave.
    pub(crate) remove: Vec<ProcessorSlot>,
    /// New values for running processors: (slot, parameter index, value).
    pub(crate) params: Vec<(ProcessorSlot, usize, f32)>,
    /// Where the player puts the processors it takes out, so they go back to be dropped on the
    /// UI thread. Its capacity covers every removal and every install, so the player's pushes
    /// never allocate.
    pub(crate) removed: Vec<Box<dyn Processor>>,
    /// New audio clip sources, made on the UI thread, with the free slots they go in.
    pub(crate) install_sources: Vec<(SourceSlot, Box<dyn ClipSource>)>,
    /// Slots whose sources leave.
    pub(crate) remove_sources: Vec<SourceSlot>,
    /// Where the player puts the sources it takes out, as `removed` for processors.
    pub(crate) removed_sources: Vec<Box<dyn ClipSource>>,
}

impl fmt::Debug for RuntimeDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let installed: Vec<ProcessorSlot> = self.install.iter().map(|(slot, _)| *slot).collect();
        let sources: Vec<SourceSlot> = self.install_sources.iter().map(|(slot, _)| *slot).collect();
        f.debug_struct("RuntimeDelta")
            .field("install", &installed)
            .field("remove", &self.remove)
            .field("params", &self.params)
            .field("removed", &self.removed.len())
            .field("install_sources", &sources)
            .field("remove_sources", &self.remove_sources)
            .field("removed_sources", &self.removed_sources.len())
            .finish()
    }
}

/// One pattern as a timeline: events sorted by tick, each inside `0..length`.
#[derive(Debug)]
pub(crate) struct PatternPlan {
    pub(crate) id: PatternId,
    pub(crate) length: Ticks,
    /// Shared with every later plan until an edit touches this pattern.
    pub(crate) events: Arc<[NoteEvent]>,
}

/// One note to start: when (ticks from the pattern start), on which channel, how hard, and how
/// fast its sound plays (its key, compiled to a speed). A plugin instrument is sent the key and
/// the length instead.
///
/// Why the channel is named by ID rather than by a dense slot: a pattern's event list outlives
/// the plan it was compiled for, and a later plan's channel table may differ.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteEvent {
    pub(crate) tick: Ticks,
    pub(crate) channel: ChannelId,
    pub(crate) velocity: f32,
    pub(crate) speed: f64,
    pub(crate) key: u8,
    /// 0 for a step hit.
    pub(crate) length: Ticks,
}

impl NoteEvent {
    /// Ticks from the start of the pattern.
    pub fn tick(&self) -> Ticks {
        self.tick
    }

    pub fn channel(&self) -> ChannelId {
        self.channel
    }

    /// 0.0..=1.0, a linear gain.
    pub fn velocity(&self) -> f32 {
        self.velocity
    }

    /// Playback speed of the note's sound: 1.0 at key 60, doubling every 12 keys up.
    pub fn speed(&self) -> f64 {
        self.speed
    }

    pub fn key(&self) -> u8 {
        self.key
    }

    /// The note's length in ticks; 0 for a step hit.
    pub fn length(&self) -> Ticks {
        self.length
    }
}

impl PlayPlan {
    /// Output sample rate in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Ticks per beat.
    pub fn ppq(&self) -> u16 {
        self.ppq
    }

    /// The compiled events of pattern `id`, sorted by tick. Plans compiled by one
    /// [`crate::compile::Compiler`] share this list until an edit touches the pattern.
    pub fn pattern_events(&self, id: PatternId) -> Option<&Arc<[NoteEvent]>> {
        let index = self.patterns.binary_search_by_key(&id, |p| p.id).ok()?;
        self.patterns.get(index).map(|pattern| &pattern.events)
    }

    pub(crate) fn channel(&self, id: ChannelId) -> Option<&ChannelPlan> {
        let index = self.channels.binary_search_by_key(&id, |c| c.id).ok()?;
        self.channels.get(index)
    }

    pub(crate) fn channel_mut(&mut self, id: ChannelId) -> Option<&mut ChannelPlan> {
        let index = self.channels.binary_search_by_key(&id, |c| c.id).ok()?;
        self.channels.get_mut(index)
    }

    /// The table slot of the processor `owner` plays: a plugin channel's instrument or a slot's
    /// effect.
    pub(crate) fn processor_slot(&self, owner: ProcessorOwner) -> Option<ProcessorSlot> {
        match owner {
            ProcessorOwner::Channel(id) => match self.channel(id)?.sound {
                ChannelSound::Plugin(slot) => Some(slot),
                ChannelSound::Sampler(_)
                | ChannelSound::Shaped(_)
                | ChannelSound::Clips
                | ChannelSound::Instrument(_)
                | ChannelSound::Input => None,
            },
            ProcessorOwner::Effect(id) => self.mixer.effect_slot(id),
        }
    }

    /// Zeroes every instrument channel's round-robin counts.
    pub(crate) fn reset_round_robins(&self) {
        for channel in &self.channels {
            if let ChannelSound::Instrument(sound) = &channel.sound {
                sound.reset();
            }
        }
    }

    /// Carries the round-robin counts of `old`'s instrument channels over to this plan's,
    /// channel by channel, where the set is the same.
    pub(crate) fn carry_round_robins(&self, old: &PlayPlan) {
        for channel in &self.channels {
            let (ChannelSound::Instrument(sound), Some(before)) =
                (&channel.sound, old.channel(channel.id))
            else {
                continue;
            };
            if let ChannelSound::Instrument(before) = &before.sound {
                sound.carry(before);
            }
        }
    }

    /// Where playback wraps: in song mode, see [`SongPlan::wrap`]; a pattern never wraps (it
    /// loops by itself).
    pub(crate) fn wrap(&self) -> Option<(Ticks, Ticks)> {
        match self.playing {
            Playing::Song => self.song.wrap(),
            Playing::Pattern(_) | Playing::Nothing => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // By hand: from 1.0 to 0.125 the ramp is 1.0 on its first frame, 1 − 0.875·k/256 on frame k,
    // and exactly 0.125 from frame 256 on.
    #[test]
    fn a_ramp_runs_256_frames_then_holds_its_value_exactly() {
        let mut ramp = Ramp::settled(1.0).toward(0.125);
        assert!(!ramp.is_settled());
        assert_eq!(ramp.at(0), 1.0);
        assert_eq!(ramp.at(128), 0.5625);
        assert_eq!(ramp.at(256), 0.125);
        ramp.advance(100);
        assert_eq!(ramp.at(28), 0.5625, "counted from where the ramp is");
        ramp.advance(156);
        assert!(ramp.is_settled());
        assert_eq!(ramp.at(0), 0.125);
    }

    #[test]
    fn a_ramp_heading_for_its_target_goes_on_and_a_new_one_starts_from_here() {
        let mut ramp = Ramp::settled(0.0).toward(1.0);
        ramp.advance(64);
        assert_eq!(ramp.toward(1.0), ramp, "the same target keeps the ramp");
        let turned = ramp.toward(0.0);
        assert_eq!(turned.at(0), 0.25, "from the value it had reached");
        assert_eq!(turned.at(256), 0.0);
        let mut jumped = ramp;
        jumped.retarget(0.5, true);
        assert_eq!(jumped, Ramp::settled(0.5));
    }
}
