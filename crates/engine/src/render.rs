//! Renders one block of a play plan: starts each note at its exact sample, mixes the sounding
//! voices, the plugin instruments and the audio clips into their channels' inserts, and runs the
//! mixer. Only [`crate::realtime::Player::process`] calls into this module.
//!
//! Two clocks meet here. Notes and audio clips are found by song sample, on the transport's
//! timeline; voices are timed by the output clock, which counts frames since the last Play,
//! Seek or Stop and never jumps, so a wrap back to the loop start leaves sounding voices
//! ringing. Audio clips read their file by song position, so a wrap or a seek re-reads by
//! position.
//!
//! Plugin instruments are not cut at note starts: the notes a block starts on a plugin channel
//! are handed to its processor with their frame and length, and the processor renders the whole
//! block. Its output is added into the channel's insert after the sampler voices, in channel
//! order, and the audio clips after that in timeline order, so every frame sums in the same
//! order however the render is split.
//!
//! Automation is worked out per block in song mode: the player cuts blocks wherever a window
//! starts or ends or a point falls, so in one block each automation either follows one window,
//! holds the end value of the window that ended last, or does not govern. Volumes, pans, faders
//! and sends take their automated value frame by frame in the mixing loops; processor
//! parameters take theirs at the start of a block that begins on a control point.
//!
//! The mixer runs once per block, insert by insert in the plan's order, so an insert's bus has
//! everything sent to it before it runs: effects in slot order, fader, pan, mute and solo, the
//! meter, then each send. The master runs last and is the output.
//!
//! (`usize` to `u64` casts here never truncate: `usize` is at most 64 bits on every target.)

use std::cmp::Ordering;
use std::ops::Range;
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

use crate::dsp::instrument::Zone;
use crate::dsp::laws::{channel_pan_gains, fader_gain, insert_pan, insert_pan_gains};
use crate::dsp::pitch::{bend_ratio, Pitch, Vibrato};
use crate::dsp::sampler::{self, playback_speed, SampleBuffer};
use crate::dsp::shape::{ended, wheel_tone, Cut, FilterMemory, Shape};
use crate::input::InputFeeds;
use crate::plan::{
    AudioClipPlan, AutomationPlan, AutomationWindow, ChannelPlan, ChannelSound, ClipPlan,
    InsertPlan, InstrumentSound, NoteEvent, PatternPlan, PlayPlan, Playing, ProcessorSlot, Ramp,
    Rate, RuntimeDelta, PLUGIN_CHANNELS, PROCESSOR_SLOTS, TABLE_SLOTS,
};
use crate::processor::{PortEvent, ProcessContext, Processor, TransportInfo};
use crate::project::{ChannelId, ClipId, Curve, MAX_AUDIO_CLIPS, MAX_AUTOMATIONS, MAX_INSERTS};
use crate::source::{ClipSource, SourceRead};
use crate::time::{self, Ticks};
use crate::transport::{ParamTarget, TempoSegment};

/// How many notes can sound at once. When every voice is busy, a new note takes over the one
/// that started first. The pool is a fixed array, so starting a note never allocates.
pub(crate) const VOICE_COUNT: usize = 64;

/// The most frames one block renders: every bus holds this many. The player cuts longer
/// buffers into blocks; nothing depends on where they are cut.
pub(crate) const CHUNK: usize = 512;

/// One bus per insert a project can have, the master included.
pub(crate) const BUSES: usize = MAX_INSERTS as usize + 1;

/// The most plugin-channel notes one block can start, over all plugin channels; more are
/// dropped and counted.
pub(crate) const PLUGIN_NOTES_PER_BLOCK: usize = 4096;

/// The most notes one plugin channel is sent in a block; more are dropped and counted.
pub(crate) const NOTES_PER_PLUGIN: usize = 1024;

/// The most audio clips whose underruns one block names; more are only counted.
pub(crate) const UNDERRUNS_PER_BLOCK: usize = 64;

/// How many live notes can sound at once on samplers, apart from the sequence's voices. When
/// every one is busy, a new note takes over the one that started first.
pub(crate) const LIVE_VOICES: usize = 16;

/// How many zones of the sequence's instrument notes can sound at once (a note sounds one zone
/// per layer). When every one is busy, a new zone takes over the one that started first. Why a
/// pool of their own: held notes and release tails keep zones sounding far longer than a
/// one-shot sample, and the samplers' pool and its stealing stay as they were.
pub(crate) const ZONE_VOICES: usize = 128;

/// How many zones of live notes on instrument channels can sound at once.
pub(crate) const LIVE_ZONE_VOICES: usize = 32;

/// How many live keys can be held down at once, over every channel: two whole keyboards. A key
/// struck with every hold taken is dropped and counted, since a note its note-off could not
/// find would sound on.
pub(crate) const HELD_NOTES: usize = 256;

/// The most live note-ons and note-offs for plugin channels one call can carry; more are
/// dropped and counted with the plugin notes.
const LIVE_PLUGIN_NOTES: usize = 256;

/// Seconds a plugin channel keeps being rendered while stopped after its last live note ends,
/// so its release rings out.
const LIVE_TAIL_SECONDS: u64 = 4;

/// Stereo frames, left then right.
type Frames = [[f32; 2]];

/// The mutable side of playback: the voice pool, a bus per insert, the processors, the audio
/// clips' sources, the previewed file and the automation's working values. The sounds the
/// voices read live in the plan, so swapping in a new plan needs no change here.
///
/// Why every bus is allocated up front (501 × 512 stereo frames, about 2 MB): the audio thread
/// can then play any project without allocating, and the memory costs less than carrying new
/// buses over with each plan. The automation values (1024 × 512 frames, 2 MB) are sized the
/// same way.
pub(crate) struct Runtime {
    voices: [Option<Voice>; VOICE_COUNT],
    /// Bus k is `buses[k * CHUNK..][..CHUNK]`. Only a bus that is fed holds this block's
    /// sound; the others hold whatever an earlier block left.
    buses: Box<[[f32; 2]]>,
    /// Whether anything went into each bus during the block being rendered. A bus is zeroed
    /// when it is first fed, so a block costs nothing for the inserts it does not use.
    fed: Box<[bool]>,
    processors: Box<[Option<Box<dyn Processor>>]>,
    /// The notes the block being rendered starts on plugin channels, in the order found.
    plugin_notes: Vec<(ChannelId, PortEvent)>,
    /// One plugin channel's share of `plugin_notes`, as its processor is sent them.
    port_events: Vec<PortEvent>,
    /// Where a plugin instrument renders a block before it is mixed in.
    scratch: Box<[[f32; 2]]>,
    /// Plugin-channel notes dropped because a block had no room left for them.
    dropped_notes: u64,
    /// Audio clip sources, by slot.
    sources: Box<[Option<Box<dyn ClipSource>>]>,
    /// Where an audio clip's (or the preview's) frames are read before they are mixed.
    clip_scratch: Box<[[f32; 2]]>,
    automation: AutomationState,
    /// The clips whose sources were not ready in the block just rendered, for the player to
    /// report; at most [`UNDERRUNS_PER_BLOCK`].
    pub(crate) underruns: Vec<ClipId>,
    /// Every underrun so far.
    pub(crate) underrun_count: u64,
    /// The audio clips whose source failed for good in the block just rendered; at most
    /// [`UNDERRUNS_PER_BLOCK`].
    pub(crate) failures: Vec<ClipId>,
    /// The file being previewed into the master, while there is one.
    pub(crate) preview: Option<Preview>,
    /// Live notes on samplers, timed by the output frame, which no Play, Stop or Seek resets:
    /// a note played while the transport moves rings on.
    live: [Option<LiveVoice>; LIVE_VOICES],
    /// The live keys held down, each on its channel at most once, with its velocity, so a
    /// note-off ends that note and a reset can strike it again. Which input holds each is the
    /// player's to know.
    held: [Option<HeldNote>; HELD_NOTES],
    /// Live note-ons and note-offs for plugin channels, for the next block rendered.
    live_plugin_notes: Vec<(ChannelId, PortEvent)>,
    /// Per plugin-channel slot, the output frame until which it is rendered while stopped:
    /// `u64::MAX` while a live note is held on it, then a few seconds more for its release.
    live_until: Box<[u64]>,
    /// The sequence's instrument zones sounding, timed by the output clock as `voices` are.
    zones: [Option<ZoneVoice>; ZONE_VOICES],
    /// Live notes' instrument zones, timed by the output frame as `live` is.
    live_zones: [Option<ZoneVoice>; LIVE_ZONE_VOICES],
    /// The input channels' feeds, while the player has been handed a set.
    pub(crate) inputs: Option<Box<InputFeeds>>,
    /// Where a feed's frames are read before they are mixed.
    input_scratch: Box<[[f32; 2]]>,
}

/// A live key held down: the channel it plays, its key and the velocity it was struck at.
#[derive(Debug, Clone, Copy)]
struct HeldNote {
    channel: ChannelId,
    key: u8,
    velocity: f32,
}

/// One zone of an instrument note sounding: its channel, the set and zone it plays, its key,
/// the frame it started on (of the output clock in the sequence, of the output frame count
/// live), when its note ended, where it reads (its speed, bent live) and how loud it plays. The
/// channel's gain is applied while mixing, as for a sampler voice.
#[derive(Debug, Clone, Copy)]
struct ZoneVoice {
    channel: ChannelId,
    /// The input a live note came from (the player's number for it), whose bend moves it; `None`
    /// in the sequence.
    input: Option<u64>,
    /// The set, by identity: the voice ends once its channel plays another.
    instrument: usize,
    zone: u32,
    key: u8,
    start: u64,
    /// Frames after `start` its note ended; `None` while a live key is held.
    off: Option<u64>,
    pitch: Pitch,
    gain: f32,
    /// Its filter's memory, while its channel's sound settings filter it.
    filter: FilterMemory,
    /// The filter's coefficients while a device's mod wheel opens it (live), in place of the
    /// channel's own.
    tone: Option<[f64; 5]>,
    /// Where a later note on its channel, which cuts itself, cut it.
    cut: Option<Cut>,
}

impl ZoneVoice {
    /// Cuts it at frame `at` of its pool's clock ([`Runtime::cut`]).
    fn cut_at(&mut self, at: u64, rate: u32) {
        let Some(cut) = cut_from(self.start, self.cut, at, rate) else {
            return;
        };
        // A sustain loop reads on from where the cut ends its note, also once bent after.
        if self.cut.is_none() && Cut::ends(Some(cut), self.off).is_some() {
            self.pitch.release(cut.at);
        }
        self.cut = Some(cut);
    }

    /// Its channel and zone and the channel's sound settings, while the channel in `plan` still
    /// plays its set.
    fn zone<'a>(
        &self,
        plan: &'a PlayPlan,
    ) -> Option<(&'a ChannelPlan, &'a Zone, Option<&'a Shape>)> {
        let channel = plan.channel(self.channel)?;
        let sound = instrument_sound(channel)?;
        if sound.instrument.identity() != self.instrument {
            return None;
        }
        Some((
            channel,
            sound.instrument.zone(self.zone)?,
            sound.shape.as_ref(),
        ))
    }

    /// Its channel, zone and sound settings, and the frames it has played by frame `at`, while
    /// it still sounds there.
    fn sounding<'a>(&self, plan: &'a PlayPlan, at: u64) -> Option<Sounding<'a>> {
        let (channel, zone, shape) = self.zone(plan)?;
        let elapsed = at.saturating_sub(self.start);
        zone.frame_shaped(elapsed, self.off, self.cut, &self.pitch, shape)?;
        Some((channel, zone, shape, elapsed))
    }
}

/// A zone voice that sounds: its channel, zone and the channel's sound settings, and the frames
/// it has played.
type Sounding<'a> = (&'a ChannelPlan, &'a Zone, Option<&'a Shape>, u64);

/// A live note on a sampler: its channel and key, the input it came from (whose bend moves it),
/// the output frame it started on, how hard it plays and where it reads its sound.
#[derive(Debug, Clone, Copy)]
struct LiveVoice {
    channel: ChannelId,
    key: u8,
    input: u64,
    start: u64,
    velocity: f32,
    pitch: Pitch,
    /// Frames after `start` its key was let go; heard only under sound settings that hear it.
    off: Option<u64>,
    filter: FilterMemory,
    /// The filter's coefficients while the input's mod wheel opens it.
    tone: Option<[f64; 5]>,
    /// Where a later note on its channel, which cuts itself, cut it.
    cut: Option<Cut>,
}

/// A previewed file: its source and the frame it plays next.
pub(crate) struct Preview {
    pub(crate) source: Box<dyn ClipSource>,
    pub(crate) frame: u64,
}

impl Preview {
    /// Whether it has played its last frame.
    pub(crate) fn finished(&self) -> bool {
        self.frame >= self.source.frames()
    }
}

/// The automation's working state, by index into the plan's automation list.
struct AutomationState {
    /// Each automation's value for every frame of the block, [`CHUNK`] per automation: filled
    /// for audio-rate automations that govern the block.
    values: Box<[f32]>,
    /// Whether each governs the block.
    governs: Box<[bool]>,
    /// The value last given a processor parameter, NaN for none since it last stopped
    /// governing.
    sent: Box<[f32]>,
    /// The value each set last, NaN while it does not govern: what a knob shows.
    last: Box<[f32]>,
}

impl AutomationState {
    fn new() -> Self {
        Self {
            values: vec![0.0; MAX_AUTOMATIONS * CHUNK].into_boxed_slice(),
            governs: vec![false; MAX_AUTOMATIONS].into_boxed_slice(),
            sent: vec![f32::NAN; MAX_AUTOMATIONS].into_boxed_slice(),
            last: vec![f32::NAN; MAX_AUTOMATIONS].into_boxed_slice(),
        }
    }

    /// The values of automation `index` over the block, when it governs it.
    fn values(&self, index: Option<u16>) -> Option<&[f32]> {
        let index = usize::from(index?);
        if !*self.governs.get(index)? {
            return None;
        }
        self.values.get(index * CHUNK..(index + 1) * CHUNK)
    }
}

/// A sounding note: the channel whose sound it plays, the output-clock frame it started at, its
/// velocity and where it reads its sound (at its playback speed). The channel's gain is applied
/// while mixing, so volume and mute changes reach notes that are already sounding.
#[derive(Debug, Clone, Copy)]
struct Voice {
    channel: ChannelId,
    start: u64,
    velocity: f32,
    pitch: Pitch,
    /// Frames after `start` its note ends; heard only under sound settings that hear it.
    off: Option<u64>,
    filter: FilterMemory,
    /// The filter's coefficients while a device's mod wheel opens it (a live note's).
    tone: Option<[f64; 5]>,
    /// Where a later note on its channel, which cuts itself, cut it.
    cut: Option<Cut>,
}

/// A channel's volume and pan over one block: the plan's values, or per frame where an
/// automation governs one of them.
#[derive(Clone, Copy)]
enum Levels<'a> {
    Fixed {
        gain: f32,
        pan: f32,
        pan_gains: [f32; 2],
    },
    Varying {
        muted: bool,
        volume: Level<'a>,
        pan: Level<'a>,
        pan_gains: [f32; 2],
    },
}

/// One value over a block: fixed, or one per frame.
#[derive(Clone, Copy)]
enum Level<'a> {
    Fixed(f32),
    PerFrame(&'a [f32]),
}

impl Level<'_> {
    fn at(self, frame: usize) -> f32 {
        match self {
            Level::Fixed(value) => value,
            Level::PerFrame(values) => values.get(frame).copied().unwrap_or(0.0),
        }
    }
}

impl<'a> Levels<'a> {
    fn of(channel: &ChannelPlan, automation: &'a AutomationState) -> Self {
        let volume = automation.values(channel.volume_automation);
        let pan = automation.values(channel.pan_automation);
        if volume.is_none() && pan.is_none() {
            return Levels::Fixed {
                gain: channel.gain,
                pan: channel.pan,
                pan_gains: channel.pan_gains,
            };
        }
        Levels::Varying {
            muted: channel.muted,
            volume: volume.map_or(Level::Fixed(channel.gain), Level::PerFrame),
            pan: pan.map_or(Level::Fixed(channel.pan), Level::PerFrame),
            pan_gains: channel.pan_gains,
        }
    }

    /// Whether an automation governs the volume or the pan.
    fn varies(&self) -> bool {
        matches!(self, Levels::Varying { .. })
    }

    /// The volume on block frame `frame`: 0 while muted.
    fn volume(&self, frame: usize) -> f32 {
        match *self {
            Levels::Fixed { gain, .. } => gain,
            Levels::Varying { muted: true, .. } => 0.0,
            Levels::Varying { volume, .. } => volume.at(frame),
        }
    }

    /// The pan position on block frame `frame`.
    fn pan(&self, frame: usize) -> f32 {
        match *self {
            Levels::Fixed { pan, .. } => pan,
            Levels::Varying { pan, .. } => pan.at(frame),
        }
    }

    /// The mono pan law's gains on block frame `frame`.
    fn pan_gains(&self, frame: usize) -> [f32; 2] {
        match *self {
            Levels::Fixed { pan_gains, .. }
            | Levels::Varying {
                pan: Level::Fixed(_),
                pan_gains,
                ..
            } => pan_gains,
            Levels::Varying {
                pan: Level::PerFrame(values),
                ..
            } => channel_pan_gains(values.get(frame).copied().unwrap_or(0.0)),
        }
    }
}

/// Adds `value`, a frame at `gain`, into `out`: a mono frame through the channel pan law, a
/// stereo frame through the balance law (the insert pan's crossfeed).
fn add_panned(
    out: &mut [f32; 2],
    value: [f32; 2],
    gain: f32,
    stereo: bool,
    levels: &Levels,
    at: usize,
) {
    if stereo {
        let scaled = [value[0] * gain, value[1] * gain];
        let pan = levels.pan(at);
        let panned = if pan == 0.0 {
            scaled
        } else {
            insert_pan(pan, insert_pan_gains(pan), scaled)
        };
        out[0] += panned[0];
        out[1] += panned[1];
    } else {
        let [left, right] = levels.pan_gains(at);
        out[0] += value[0] * (gain * left);
        out[1] += value[1] * (gain * right);
    }
}

impl Runtime {
    /// No voices sounding, silent buses, no processors and no sources. Allocates everything
    /// playback needs.
    pub(crate) fn new() -> Self {
        Self {
            voices: [None; VOICE_COUNT],
            buses: vec![[0.0; 2]; BUSES * CHUNK].into_boxed_slice(),
            fed: vec![false; BUSES].into_boxed_slice(),
            processors: std::iter::repeat_with(|| None).take(TABLE_SLOTS).collect(),
            plugin_notes: Vec::with_capacity(PLUGIN_NOTES_PER_BLOCK),
            port_events: Vec::with_capacity(NOTES_PER_PLUGIN),
            scratch: vec![[0.0; 2]; CHUNK].into_boxed_slice(),
            dropped_notes: 0,
            sources: std::iter::repeat_with(|| None)
                .take(MAX_AUDIO_CLIPS)
                .collect(),
            clip_scratch: vec![[0.0; 2]; CHUNK].into_boxed_slice(),
            automation: AutomationState::new(),
            underruns: Vec::with_capacity(UNDERRUNS_PER_BLOCK),
            underrun_count: 0,
            failures: Vec::with_capacity(UNDERRUNS_PER_BLOCK),
            preview: None,
            live: [None; LIVE_VOICES],
            held: [None; HELD_NOTES],
            live_plugin_notes: Vec::with_capacity(LIVE_PLUGIN_NOTES),
            live_until: vec![0; PLUGIN_CHANNELS].into_boxed_slice(),
            zones: [None; ZONE_VOICES],
            live_zones: [None; LIVE_ZONE_VOICES],
            inputs: None,
            input_scratch: vec![[0.0; 2]; CHUNK].into_boxed_slice(),
        }
    }

    /// Whether an input channel of `plan` is heard: it has a feed and is not muted. While
    /// stopped the player then runs the mixer, so the input is heard (monitored) through its
    /// channel.
    pub(crate) fn inputs_heard(&self, plan: &PlayPlan) -> bool {
        self.inputs.as_ref().is_some_and(|inputs| {
            inputs.channels().any(|id| {
                plan.channel(id).is_some_and(|channel| {
                    matches!(channel.sound, ChannelSound::Input) && !channel.muted
                })
            })
        })
    }

    /// Reads and drops the next `len` frames of every feed: frames played while nothing is
    /// heard, so each feed stays at the frame being played.
    pub(crate) fn skip_inputs(&mut self, len: usize) {
        let Some(inputs) = self.inputs.as_mut() else {
            return;
        };
        let mut left = len;
        while left > 0 {
            let now = left.min(CHUNK);
            for (_, feed) in inputs.iter_mut() {
                feed.read(&mut self.input_scratch[..now]);
            }
            left -= now;
        }
    }

    /// Starts a live note of `key` from `input` (the player's number for it) on `channel` at
    /// output frame `at`, as the input's controls there play it (its pitch bend and mod wheel): a
    /// sampler voice (one already sounding for this channel and key restarts; a full pool takes
    /// over the oldest), or a note-on for a plugin channel that is not muted. The key is held on
    /// `channel` either way (struck again, at its new velocity), so its note-off finds it. A mute
    /// does not stop a sampler's live notes: mute silences the sequence, not what is played. On a
    /// channel that cuts itself the note first cuts the channel's voices that began before it
    /// ([`cut`](Self::cut)), the sequence's at its output-clock frame `sequence` (while the song
    /// plays) too.
    /// Returns `false`, starting nothing and counting the note dropped, when every hold is taken.
    #[expect(
        clippy::too_many_arguments,
        reason = "a live note: its input, channel, key, velocity, frame and the controls it begins with"
    )]
    pub(crate) fn live_on(
        &mut self,
        plan: &PlayPlan,
        input: u64,
        channel: ChannelId,
        key: u8,
        velocity: f32,
        at: u64,
        sequence: Option<u64>,
        controls: Controls,
    ) -> bool {
        let slot = self
            .held
            .iter()
            .position(|held| held.is_some_and(|held| held.channel == channel && held.key == key))
            .or_else(|| self.held.iter().position(Option::is_none));
        let Some(slot) = slot else {
            self.dropped_notes += 1;
            return false;
        };
        self.held[slot] = Some(HeldNote {
            channel,
            key,
            velocity,
        });
        let Some(target) = plan.channel(channel).filter(|target| target.plays_live) else {
            return true;
        };
        if target.cut_itself && velocity > 0.0 {
            self.cut(plan, channel, sequence, at);
        }
        let ratio = bend_ratio(controls.bend, target.bend_range);
        let (vibrato, tone) = wheel_route(plan, target, controls.wheel);
        match target.sound {
            ChannelSound::Sampler(_) | ChannelSound::Shaped(_) => {
                // On a channel that cuts itself the key's voice was cut just now and falls from
                // here: taking its slot over would end it on one frame, the click the cut's fall
                // is there to avoid, so the note takes a free slot instead.
                let index = self
                    .live
                    .iter()
                    .position(|slot| {
                        !target.cut_itself
                            && slot
                                .is_some_and(|voice| voice.channel == channel && voice.key == key)
                    })
                    .or_else(|| {
                        self.live
                            .iter()
                            .position(|slot| live_free_at(plan, *slot, at))
                    })
                    .unwrap_or_else(|| {
                        // `min_by_key` returns the first of equal minimums: the lowest slot.
                        self.live
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, slot)| slot.map_or(0, |voice| voice.start))
                            .map_or(0, |(index, _)| index)
                    });
                let speed = tuned(
                    playback_speed(key),
                    voiced(target).and_then(|(_, shape)| shape),
                );
                self.live[index] = Some(LiveVoice {
                    channel,
                    key,
                    input,
                    start: at,
                    velocity,
                    pitch: Pitch::new(speed, ratio, vibrato),
                    off: None,
                    filter: FilterMemory::default(),
                    tone,
                    cut: None,
                });
            }
            ChannelSound::Plugin(slot) if !target.muted => {
                self.live_plugin_event(
                    channel,
                    PortEvent::NoteOn {
                        frame: 0,
                        key,
                        velocity,
                    },
                );
                if let Some(until) = self.live_until_mut(slot) {
                    *until = u64::MAX;
                }
            }
            ChannelSound::Instrument(ref sound) => {
                let note = ZoneNote {
                    key,
                    velocity,
                    at,
                    off: None,
                    input: Some(input),
                    ratio,
                    vibrato,
                    tone,
                };
                start_zones(&mut self.live_zones, plan, target, sound, note);
            }
            ChannelSound::Plugin(_) | ChannelSound::Clips | ChannelSound::Input => {}
        }
        true
    }

    /// `input`'s pitch bend on `channel` moved to `bend` (−8192 to 8191) at output frame `at`:
    /// every live voice of that input on that channel reads on from where it was at its speed
    /// bent by the channel's range; a plugin channel, muted or not, is sent the bend, at frame 0
    /// of the next block.
    ///
    /// Why muted too: the plugin keeps the bend for its next notes, and a muted plugin that
    /// missed the bend's return to rest played bent once unmuted, with the device at rest (found
    /// 2026-10-26). A muted channel is sent no new notes,
    /// so the bend alone sounds nothing.
    pub(crate) fn live_bend(
        &mut self,
        plan: &PlayPlan,
        input: u64,
        channel: ChannelId,
        bend: i16,
        at: u64,
    ) {
        let Some(target) = plan.channel(channel) else {
            return;
        };
        if let ChannelSound::Plugin(_) = target.sound {
            let event = PortEvent::Bend {
                frame: 0,
                value: bend,
            };
            self.live_plugin_event(channel, event);
            return;
        }
        let ratio = bend_ratio(bend, target.bend_range);
        for voice in self.live.iter_mut().flatten() {
            if voice.channel == channel && voice.input == input {
                voice.pitch.bend(at.saturating_sub(voice.start), ratio);
            }
        }
        for voice in self.live_zones.iter_mut().flatten() {
            if voice.channel == channel && voice.input == Some(input) {
                voice.pitch.bend(at.saturating_sub(voice.start), ratio);
            }
        }
    }

    /// `input`'s mod wheel on `channel` moved to `wheel` (0 to 127) at output frame `at`: on a
    /// channel whose filter is on, every live voice of that input there plays the filter it
    /// opens from that frame; on any other they swing its vibrato from where they are; a plugin
    /// channel, muted or not, is sent it, at frame 0 of the next block (muted too for the bend's
    /// reason, [`live_bend`](Self::live_bend)).
    pub(crate) fn live_wheel(
        &mut self,
        plan: &PlayPlan,
        input: u64,
        channel: ChannelId,
        wheel: u8,
        at: u64,
    ) {
        let Some(target) = plan.channel(channel) else {
            return;
        };
        if let ChannelSound::Plugin(_) = target.sound {
            let event = PortEvent::Wheel {
                frame: 0,
                value: wheel,
            };
            self.live_plugin_event(channel, event);
            return;
        }
        let (vibrato, tone) = wheel_route(plan, target, wheel);
        let filters = target.wheel_filter.is_some();
        for voice in self.live.iter_mut().flatten() {
            if voice.channel == channel && voice.input == input {
                if filters {
                    voice.tone = tone;
                } else {
                    voice.pitch.vibrate(at.saturating_sub(voice.start), vibrato);
                }
            }
        }
        for voice in self.live_zones.iter_mut().flatten() {
            if voice.channel == channel && voice.input == Some(input) {
                if filters {
                    voice.tone = tone;
                } else {
                    voice.pitch.vibrate(at.saturating_sub(voice.start), vibrato);
                }
            }
        }
    }

    /// Ends the live note of `key` held on `channel` since its note-on, at output frame `at`;
    /// nothing when none is held there. A sampler's note rings out (its sound is one-shot); an
    /// instrument's zones go into their release; a plugin channel gets its note-off, and once
    /// no key of its is held it renders on while stopped for a few seconds of release.
    pub(crate) fn live_off(&mut self, plan: &PlayPlan, channel: ChannelId, key: u8, at: u64) {
        let Some(held) = self
            .held
            .iter_mut()
            .find(|held| held.is_some_and(|held| held.channel == channel && held.key == key))
        else {
            return;
        };
        *held = None;
        for voice in self.live_zones.iter_mut().flatten() {
            if voice.channel == channel && voice.key == key && voice.off.is_none() {
                let off = at.saturating_sub(voice.start);
                voice.off = Some(off);
                // A sustain loop reads on from where it was here, also once bent after; a cut
                // before this ended its note there already, and that frame is kept.
                if voice.cut.is_none() {
                    voice.pitch.release(off);
                }
            }
        }
        for voice in self.live.iter_mut().flatten() {
            if voice.channel == channel && voice.key == key && voice.off.is_none() {
                voice.off = Some(at.saturating_sub(voice.start));
            }
        }
        if let Some(ChannelSound::Plugin(slot)) = plan.channel(channel).map(|target| &target.sound)
        {
            self.live_plugin_event(channel, PortEvent::NoteOff { frame: 0, key });
            let still_held = self
                .held
                .iter()
                .flatten()
                .any(|held| held.channel == channel);
            let tail = LIVE_TAIL_SECONDS.saturating_mul(u64::from(plan.sample_rate));
            if let (false, Some(until)) = (still_held, self.live_until_mut(*slot)) {
                *until = at.saturating_add(tail);
            }
        }
    }

    /// Strikes every key held on a plugin channel again, as [`live_on`](Self::live_on) struck
    /// it, after [`silence`](Self::silence) reset the channel's processor (Play and Seek; Stop
    /// ends every held key first, so it strikes none): the key is still down, so its note
    /// sounds on until its note-off. A key whose note-on is still waiting is struck once, not
    /// twice: a second note-on would leave a note its one note-off cannot end.
    pub(crate) fn strike_held(&mut self, plan: &PlayPlan) {
        let held = self.held;
        for HeldNote {
            channel,
            key,
            velocity,
        } in held.into_iter().flatten()
        {
            let plays = plan.channel(channel).is_some_and(|target| {
                target.plays_live
                    && !target.muted
                    && matches!(target.sound, ChannelSound::Plugin(_))
            });
            let waiting = self.live_plugin_notes.iter().any(|&(id, event)| {
                id == channel && matches!(event, PortEvent::NoteOn { key: on, .. } if on == key)
            });
            if plays && !waiting {
                self.live_plugin_event(
                    channel,
                    PortEvent::NoteOn {
                        frame: 0,
                        key,
                        velocity,
                    },
                );
            }
        }
    }

    /// Silences every live voice on a sampler or an instrument channel.
    pub(crate) fn live_silence(&mut self) {
        self.live = [None; LIVE_VOICES];
        self.live_zones = [None; LIVE_ZONE_VOICES];
    }

    /// Whether anything live may sound from output frame `at`: a sampler voice, a plugin event
    /// waiting, or a plugin channel still rendering its live notes.
    pub(crate) fn live_sounding(&self, at: u64) -> bool {
        self.live.iter().any(Option::is_some)
            || self.live_zones.iter().any(Option::is_some)
            || !self.live_plugin_notes.is_empty()
            || self.live_until.iter().any(|&until| until > at)
    }

    fn live_plugin_event(&mut self, channel: ChannelId, event: PortEvent) {
        if self.live_plugin_notes.len() < LIVE_PLUGIN_NOTES {
            self.live_plugin_notes.push((channel, event));
        } else {
            self.dropped_notes += 1;
        }
    }

    fn live_until_mut(&mut self, slot: ProcessorSlot) -> Option<&mut u64> {
        let index = slot.index().checked_sub(PROCESSOR_SLOTS)?;
        self.live_until.get_mut(index)
    }

    /// Adds every live sampler voice, then every live instrument zone, into its channel's bus
    /// over frames `0..len` of a block whose first frame is output frame `at`, at velocity (a
    /// zone's gain and envelope) × the channel's volume (mute or not) × its pan, in slot order.
    fn mix_live(&mut self, plan: &PlayPlan, at: u64, len: usize) {
        let Self {
            live,
            live_zones,
            buses,
            fed,
            ..
        } = self;
        for slot in live.iter_mut() {
            let Some(live_voice) = slot.as_mut() else {
                continue;
            };
            let mut voice = Voice {
                channel: live_voice.channel,
                start: live_voice.start,
                velocity: live_voice.velocity,
                pitch: live_voice.pitch,
                off: live_voice.off,
                filter: live_voice.filter,
                tone: live_voice.tone,
                cut: live_voice.cut,
            };
            let sounding = plan.channel(voice.channel).and_then(|channel| {
                let (sample, shape) = voiced(channel)?;
                let elapsed = voice_elapsed(&voice, at, sample, shape)?;
                Some((channel, sample, shape, elapsed))
            });
            let Some((channel, sample, shape, elapsed)) = sounding else {
                // Played out, or its channel is gone from the plan.
                *slot = None;
                continue;
            };
            let Some(out) = feed(buses, fed, usize::from(channel.bus.0), len) else {
                continue;
            };
            let levels = Levels::Fixed {
                gain: channel.volume,
                pan: channel.pan,
                pan_gains: channel.pan_gains,
            };
            match shape {
                None => add_voice(sample, elapsed, voice, &levels, 0, out),
                Some(shape) => {
                    add_shaped(sample, shape, elapsed, &mut voice, &levels, 0, out);
                    live_voice.filter = voice.filter;
                }
            }
        }
        for slot in live_zones.iter_mut() {
            let Some(voice) = slot.as_mut() else { continue };
            let Some((channel, zone, shape, elapsed)) = voice.sounding(plan, at) else {
                // Released or played out, or its channel plays another set now.
                *slot = None;
                continue;
            };
            let Some(out) = feed(buses, fed, usize::from(channel.bus.0), len) else {
                continue;
            };
            let levels = Levels::Fixed {
                gain: channel.volume,
                pan: channel.pan,
                pan_gains: channel.pan_gains,
            };
            add_zone(zone, shape, voice, elapsed, &levels, 0, out);
        }
    }

    /// Plugin-channel notes dropped so far because a block had no room for them.
    pub(crate) fn dropped_notes(&self) -> u64 {
        self.dropped_notes
    }

    /// Counts one more note (or a performance control) the player had no room for.
    pub(crate) fn count_dropped(&mut self) {
        self.dropped_notes += 1;
    }

    /// Silences every voice and clears every processor's tail.
    pub(crate) fn silence(&mut self) {
        self.voices = [None; VOICE_COUNT];
        self.zones = [None; ZONE_VOICES];
        for processor in self.processors.iter_mut().flatten() {
            processor.reset();
        }
    }

    /// Makes the processor and source changes `delta` asks for: processors and sources leave
    /// first (processors suspended here), then into `delta.removed` and
    /// `delta.removed_sources`, to be dropped on the UI thread; new ones go into their slots,
    /// then running processors take their new values. Never allocates: the removal lists have
    /// room for every one.
    pub(crate) fn apply(&mut self, delta: &mut RuntimeDelta) {
        for slot in &delta.remove {
            if let Some(mut processor) = self.slot_mut(*slot).and_then(Option::take) {
                processor.suspend();
                delta.removed.push(processor);
            }
        }
        for (slot, processor) in delta.install.drain(..) {
            let Some(entry) = self.processors.get_mut(slot.index()) else {
                delta.removed.push(processor);
                continue;
            };
            // The compiler never hands out a held slot; should one be held, its processor goes
            // back to the UI thread rather than being dropped here.
            if let Some(held) = entry.replace(processor) {
                delta.removed.push(held);
            }
        }
        for &(slot, index, value) in &delta.params {
            self.set_param(slot, index, value);
        }
        for slot in &delta.remove_sources {
            if let Some(source) = self.sources.get_mut(slot.index()).and_then(Option::take) {
                delta.removed_sources.push(source);
            }
        }
        for (slot, source) in delta.install_sources.drain(..) {
            let Some(entry) = self.sources.get_mut(slot.index()) else {
                delta.removed_sources.push(source);
                continue;
            };
            if let Some(held) = entry.replace(source) {
                delta.removed_sources.push(held);
            }
        }
    }

    pub(crate) fn set_param(&mut self, slot: ProcessorSlot, index: usize, value: f32) {
        if let Some(processor) = self.slot_mut(slot).and_then(Option::as_mut) {
            processor.set_param(index, value);
        }
    }

    pub(crate) fn queue_param(&mut self, slot: ProcessorSlot, id: u32, value: f32) {
        if let Some(processor) = self.slot_mut(slot).and_then(Option::as_mut) {
            processor.queue_param(id, value);
        }
    }

    /// Gives a processor parameter of `plan` a value: an effect's by its index, a plugin's by
    /// its own number. Other targets are not processors and are ignored.
    pub(crate) fn set_processor_param(&mut self, plan: &PlayPlan, target: ParamTarget, value: f32) {
        match target {
            ParamTarget::Effect { effect, param } => {
                if let Some(slot) = plan.mixer.effect_slot(effect) {
                    self.set_param(slot, usize::from(param), value);
                }
            }
            ParamTarget::Plugin { owner, id } => {
                if let Some(slot) = plan.processor_slot(owner) {
                    self.queue_param(slot, id, value);
                }
            }
            ParamTarget::ChannelVolume(_)
            | ParamTarget::ChannelPan(_)
            | ParamTarget::InsertFader(_)
            | ParamTarget::InsertPan(_)
            | ParamTarget::Send { .. } => {}
        }
    }

    /// While stopped: lets every processor apply what waits for it.
    pub(crate) fn idle(&mut self, context: &ProcessContext<'_>) {
        for processor in self.processors.iter_mut().flatten() {
            processor.idle(context);
        }
    }

    /// Suspends every processor: their last call on this thread before they leave with it.
    pub(crate) fn suspend_all(&mut self) {
        for processor in self.processors.iter_mut().flatten() {
            processor.suspend();
        }
    }

    fn slot_mut(&mut self, slot: ProcessorSlot) -> Option<&mut Option<Box<dyn Processor>>> {
        self.processors.get_mut(slot.index())
    }

    /// Whether automation `index` governs the block being rendered.
    pub(crate) fn governs(&self, index: usize) -> bool {
        self.automation.governs.get(index).copied().unwrap_or(false)
    }

    /// The value automation `index` set last, NaN while it does not govern.
    pub(crate) fn automation_value(&self, index: usize) -> f32 {
        self.automation.last.get(index).copied().unwrap_or(f32::NAN)
    }

    /// Ends every automation's hold on its processor parameter: each one that set a value goes
    /// back to the value the project holds. Play, Stop and Seek start the automation afresh.
    pub(crate) fn release_automation(&mut self, plan: &PlayPlan) {
        for (index, automation) in plan.automation.iter().enumerate().take(MAX_AUTOMATIONS) {
            self.automation.governs[index] = false;
            self.automation.last[index] = f32::NAN;
            if !self.automation.sent[index].is_nan() {
                self.automation.sent[index] = f32::NAN;
                if let (Some(target), Rate::Control) = (automation.target, automation.rate) {
                    self.set_processor_param(plan, target, automation.stored);
                }
            }
        }
    }

    /// Forgets which processor values the automation last sent, so the next control point sends
    /// them all again: a new plan's processors may hold other values.
    pub(crate) fn resend_automation(&mut self) {
        self.automation.sent.fill(f32::NAN);
    }

    /// While stopped: every streamed clip gets ready to read the frame it will play first from
    /// song sample `position` on, so Play starts warm.
    pub(crate) fn prepare_sources(
        &mut self,
        plan: &PlayPlan,
        timeline: &TempoSegment,
        position: u64,
    ) {
        let position = i128::from(position);
        for clip in &plan.song.audio {
            let Some(source) = clip
                .source
                .and_then(|slot| self.sources.get_mut(slot.index()))
                .and_then(Option::as_mut)
            else {
                continue;
            };
            let (start, end) = (timeline.sample_at(clip.start), timeline.sample_at(clip.end));
            if end <= position {
                continue;
            }
            let into = u64::try_from((position - start).max(0)).unwrap_or(0);
            source.prepare(clip.frame_offset.saturating_add(into));
        }
    }

    /// Adds every sounding voice, then every sounding instrument zone, into its channel's bus,
    /// frames `range` of a block of `len` frames, whose first frame is output-clock frame `at`.
    /// Voices are added in slot order, so each frame sums its voices in the same order however
    /// the render is split into blocks.
    fn mix(&mut self, plan: &PlayPlan, at: u64, range: Range<usize>, len: usize) {
        if range.is_empty() {
            return;
        }
        let Self {
            voices,
            zones,
            buses,
            fed,
            automation,
            ..
        } = self;
        for slot in voices.iter_mut() {
            let Some(voice) = slot.as_mut() else { continue };
            let sounding = plan.channel(voice.channel).and_then(|channel| {
                let (sample, shape) = voiced(channel)?;
                let elapsed = voice_elapsed(voice, at, sample, shape)?;
                Some((channel, sample, shape, elapsed))
            });
            match sounding {
                Some((channel, sample, shape, elapsed)) => {
                    let Some(out) = feed(buses, fed, usize::from(channel.bus.0), len)
                        .and_then(|bus| bus.get_mut(range.clone()))
                    else {
                        continue;
                    };
                    let levels = Levels::of(channel, automation);
                    match shape {
                        None => add_voice(sample, elapsed, *voice, &levels, range.start, out),
                        Some(shape) => {
                            add_shaped(sample, shape, elapsed, voice, &levels, range.start, out)
                        }
                    }
                }
                // Played out, or its channel is gone from the plan (or plays a plugin now).
                None => *slot = None,
            }
        }
        for slot in zones.iter_mut() {
            let Some(voice) = slot.as_mut() else { continue };
            let Some((channel, zone, shape, elapsed)) = voice.sounding(plan, at) else {
                // Released or played out, or its channel plays another set now.
                *slot = None;
                continue;
            };
            let Some(out) = feed(buses, fed, usize::from(channel.bus.0), len)
                .and_then(|bus| bus.get_mut(range.clone()))
            else {
                continue;
            };
            let levels = Levels::of(channel, automation);
            add_zone(zone, shape, voice, elapsed, &levels, range.start, out);
        }
    }

    /// Starts the zones `event` sounds on its instrument channel at output-clock frame `at`
    /// (output frame `live`), its note ending `off` frames later. A channel that is missing,
    /// silent (as for [`start_voice`](Self::start_voice)) or not an instrument starts none; on
    /// one that cuts itself, a note at a velocity above 0 first cuts its earlier voices
    /// ([`cut`](Self::cut)), whether or not a zone answers its key.
    fn start_zones(&mut self, plan: &PlayPlan, event: NoteEvent, at: u64, off: u64, live: u64) {
        let Some(channel) = plan.channel(event.channel) else {
            return;
        };
        if channel.gain == 0.0 && self.automation.values(channel.volume_automation).is_none() {
            return;
        }
        let Some(sound) = instrument_sound(channel) else {
            return;
        };
        if channel.cut_itself && event.velocity > 0.0 {
            self.cut(plan, event.channel, Some(at), live);
        }
        let note = ZoneNote {
            key: event.key,
            velocity: event.velocity,
            at,
            off: Some(off),
            input: None,
            ratio: 1.0,
            vibrato: Vibrato::NONE,
            tone: None,
        };
        start_zones(&mut self.zones, plan, channel, sound, note);
    }

    /// Starts a note at output-clock frame `at` (output frame `live`), ending `off` frames after
    /// it (heard only under sound settings that hear it): in the first free voice, or, when all
    /// are busy, in the voice that started first. "Free" is judged at `at` itself, never at a
    /// block edge, so which voice a note gets does not depend on how the render is split. A note
    /// at velocity 0 (as on an instrument set, whatever the channel's velocity weight), or on a
    /// channel that is missing, silent (muted, or at volume 0, unless an automation governs the
    /// volume, which a fade-in from 0 needs) or not playing a sample takes no voice. On a channel
    /// that cuts itself a note that takes one first cuts the channel's earlier voices
    /// ([`cut`](Self::cut)), before its own is chosen: a voice falling from the cut is not free.
    fn start_voice_until(
        &mut self,
        plan: &PlayPlan,
        event: NoteEvent,
        at: u64,
        off: u64,
        live: u64,
    ) {
        let silent = |channel: &ChannelPlan| {
            channel.gain == 0.0 && self.automation.values(channel.volume_automation).is_none()
        };
        if event.velocity.is_nan() || event.velocity <= 0.0 {
            return;
        }
        let Some(channel) = plan.channel(event.channel) else {
            return;
        };
        let Some((_, shape)) = voiced(channel).filter(|_| !silent(channel)) else {
            return;
        };
        if channel.cut_itself {
            self.cut(plan, event.channel, Some(at), live);
        }
        let free = self
            .voices
            .iter()
            .position(|slot| is_free_at(plan, *slot, at));
        // `min_by_key` returns the first of equal minimums, so a tie goes to the lowest slot.
        let oldest = || {
            self.voices
                .iter()
                .enumerate()
                .min_by_key(|(_, slot)| slot.map_or(0, |voice| voice.start))
                .map_or(0, |(index, _)| index)
        };
        let index = free.unwrap_or_else(oldest);
        self.voices[index] = Some(Voice {
            channel: event.channel,
            start: at,
            velocity: event.velocity,
            pitch: Pitch::steady(tuned(event.speed, shape)),
            off: Some(off),
            filter: FilterMemory::default(),
            tone: None,
            cut: None,
        });
    }

    /// A note starts on `channel`, which cuts itself, at frame `sequence` of the output clock
    /// (while the song plays) and frame `live` of the output frame count: every voice of the
    /// channel that began before it is cut there, the sequence's voices and zones and the live
    /// ones alike, since the channel sounds as one (a live note cuts a pattern note and a pattern
    /// note a live one). Voices that begin on the note's own frame (a chord on one tick) are
    /// kept, and a voice keeps the first cut it is given. A cut voice falls from the cut over the
    /// release its note's end would take, or over 2 ms with none ([`ended`]); one the sustain
    /// pedal holds is cut as any other.
    fn cut(&mut self, plan: &PlayPlan, channel: ChannelId, sequence: Option<u64>, live: u64) {
        let rate = plan.sample_rate;
        if let Some(at) = sequence {
            for voice in self.voices.iter_mut().flatten() {
                if voice.channel == channel {
                    voice.cut = cut_from(voice.start, voice.cut, at, rate);
                }
            }
            for voice in self.zones.iter_mut().flatten() {
                if voice.channel == channel {
                    voice.cut_at(at, rate);
                }
            }
        }
        for voice in self.live.iter_mut().flatten() {
            if voice.channel == channel {
                voice.cut = cut_from(voice.start, voice.cut, live, rate);
            }
        }
        for voice in self.live_zones.iter_mut().flatten() {
            if voice.channel == channel {
                voice.cut_at(live, rate);
            }
        }
    }
}

/// The cut of a voice that began at frame `start` and was cut at `cut` (none: not yet) once a
/// later note starts at frame `at` of the same clock, at `rate`: the first cut it was given, else
/// one there when it began before `at`.
fn cut_from(start: u64, cut: Option<Cut>, at: u64, rate: u32) -> Option<Cut> {
    cut.or_else(|| (start < at).then(|| Cut::new(at - start, rate)))
}

/// Adds `voice`, `elapsed` frames into `sample`, into `out`, whose first frame is block frame
/// `first`, at its velocity and the channel's `levels`; once cut, at the cut's fall too.
///
/// Why the fixed mono case keeps its own path: it is the one every project before automation
/// and stereo samples played, and computing its two gains once keeps those renders bit for bit.
fn add_voice(
    sample: &SampleBuffer,
    elapsed: u64,
    voice: Voice,
    levels: &Levels,
    first: usize,
    out: &mut Frames,
) {
    let stereo = sample.channels() == 2;
    if let Some(cut) = voice.cut {
        // The sampler's own sound has no envelope: the cut's fall (no release here) alone.
        sampler::read(sample, elapsed, &voice.pitch, out.len(), |index, value| {
            let Some(level) = ended(0, 0, elapsed + index as u64, None, Some(cut)) else {
                return;
            };
            let at = first + index;
            let gain = voice.velocity * level as f32 * levels.volume(at);
            add_panned(&mut out[index], value, gain, stereo, levels, at);
        });
        return;
    }
    match *levels {
        Levels::Fixed {
            gain, pan_gains, ..
        } if !stereo => {
            let gain = voice.velocity * gain;
            let [left, right] = pan_gains;
            sampler::play(
                sample,
                elapsed,
                &voice.pitch,
                [gain * left, gain * right],
                out,
            );
        }
        _ => sampler::read(sample, elapsed, &voice.pitch, out.len(), |index, value| {
            let at = first + index;
            let gain = voice.velocity * levels.volume(at);
            add_panned(&mut out[index], value, gain, stereo, levels, at);
        }),
    }
}

/// Adds instrument zone `voice`, `elapsed` frames into its note, into `out`, whose first frame
/// is block frame `first`, at its gain and envelope and the channel's `levels`, until it ends;
/// played with the channel's sound settings `shape` when it has them.
fn add_zone(
    zone: &Zone,
    shape: Option<&Shape>,
    voice: &mut ZoneVoice,
    elapsed: u64,
    levels: &Levels,
    first: usize,
    out: &mut Frames,
) {
    let stereo = zone.stereo();
    for (index, frame) in out.iter_mut().enumerate() {
        // Once ended a zone never sounds again: its release and its read position only move on.
        let Some((value, level)) = zone.frame_shaped(
            elapsed + index as u64,
            voice.off,
            voice.cut,
            &voice.pitch,
            shape,
        ) else {
            break;
        };
        let value = match shape {
            Some(shape) => shape.filtered(&mut voice.filter, value, voice.tone),
            None => value,
        };
        let at = first + index;
        let gain = voice.gain * level * levels.volume(at);
        add_panned(frame, value, gain, stereo, levels, at);
    }
}

/// Adds sampler `voice`, `elapsed` frames into `sample`, played with the channel's sound
/// settings `shape`, into `out`, whose first frame is block frame `first`, at its velocity's
/// level, its envelope and the channel's `levels`, until it ends.
fn add_shaped(
    sample: &SampleBuffer,
    shape: &Shape,
    elapsed: u64,
    voice: &mut Voice,
    levels: &Levels,
    first: usize,
    out: &mut Frames,
) {
    let stereo = sample.channels() == 2;
    let velocity = shape.velocity_level(voice.velocity);
    for (index, frame) in out.iter_mut().enumerate() {
        let Some((value, level)) = shape.sampler_frame(
            sample,
            elapsed + index as u64,
            voice.off,
            voice.cut,
            &voice.pitch,
        ) else {
            break;
        };
        let value = shape.filtered(&mut voice.filter, value, voice.tone);
        let at = first + index;
        let gain = velocity * level * levels.volume(at);
        add_panned(frame, value, gain, stereo, levels, at);
    }
}

/// A note an instrument channel's zones start for: its key and velocity, the frame of the pool's
/// clock it starts at, when it ends (`off` frames later; `None`: a key held), the input a live
/// note came from, and the bend's ratio, the vibrato and the filter it begins with (none of them
/// in the sequence).
#[derive(Debug, Clone, Copy)]
struct ZoneNote {
    key: u8,
    velocity: f32,
    at: u64,
    off: Option<u64>,
    input: Option<u64>,
    ratio: f64,
    vibrato: Vibrato,
    tone: Option<[f64; 5]>,
}

/// What a device's controls hold where a live note starts: its pitch bend (−8192 to 8191) and
/// its mod wheel (0 to 127).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Controls {
    pub(crate) bend: i16,
    pub(crate) wheel: u8,
}

/// What the mod wheel at `wheel` does to `channel`'s live voices: opens its filter while the
/// filter is on (the coefficients), else swings a vibrato.
fn wheel_route(plan: &PlayPlan, channel: &ChannelPlan, wheel: u8) -> (Vibrato, Option<[f64; 5]>) {
    match channel.wheel_filter {
        Some([cutoff, resonance]) => (
            Vibrato::NONE,
            wheel_tone(cutoff, resonance, wheel, plan.sample_rate),
        ),
        None => (Vibrato::of_wheel(wheel, plan.sample_rate), None),
    }
}

/// Starts in `pool` the zones `note` sounds on instrument `channel` (whose set is `sound`). Each
/// takes the first slot free at the note's frame (empty, or its zone ended), or, with none free,
/// the one that started first.
fn start_zones(
    pool: &mut [Option<ZoneVoice>],
    plan: &PlayPlan,
    channel: &ChannelPlan,
    sound: &InstrumentSound,
    note: ZoneNote,
) {
    let ZoneNote {
        key,
        velocity,
        at,
        off,
        input,
        ratio,
        vibrato,
        tone,
    } = note;
    let instrument = &sound.instrument;
    let shape = sound.shape.as_ref();
    instrument.start_shaped(
        key,
        velocity,
        &sound.counters,
        shape,
        |zone, speed, gain| {
            let free = pool
                .iter()
                .position(|slot| slot.is_none_or(|voice| voice.sounding(plan, at).is_none()));
            // `min_by_key` returns the first of equal minimums, so a tie goes to the lowest slot.
            let index = free.unwrap_or_else(|| {
                pool.iter()
                    .enumerate()
                    .min_by_key(|(_, slot)| slot.map_or(0, |voice| voice.start))
                    .map_or(0, |(index, _)| index)
            });
            if let Some(slot) = pool.get_mut(index) {
                *slot = Some(ZoneVoice {
                    channel: channel.id,
                    input,
                    instrument: instrument.identity(),
                    zone,
                    key,
                    start: at,
                    off,
                    pitch: Pitch::new(speed, ratio, vibrato),
                    gain,
                    filter: FilterMemory::default(),
                    tone,
                    cut: None,
                });
            }
        },
    );
}

/// Whether a live voice slot is free at output frame `at`: empty, or its note played out.
fn live_free_at(plan: &PlayPlan, slot: Option<LiveVoice>, at: u64) -> bool {
    slot.is_none_or(|voice| {
        let voice = Voice {
            channel: voice.channel,
            start: voice.start,
            velocity: voice.velocity,
            pitch: voice.pitch,
            off: voice.off,
            filter: voice.filter,
            tone: voice.tone,
            cut: voice.cut,
        };
        is_free_at(plan, Some(voice), at)
    })
}

fn is_free_at(plan: &PlayPlan, slot: Option<Voice>, at: u64) -> bool {
    slot.is_none_or(|voice| {
        plan.channel(voice.channel)
            .and_then(voiced)
            .is_none_or(|(sample, shape)| voice_elapsed(&voice, at, sample, shape).is_none())
    })
}

/// The sound a channel's notes play and its sound settings when they are not the defaults;
/// `None` for a plugin channel, an instrument channel, an input channel and an audio channel
/// without notes.
fn voiced(channel: &ChannelPlan) -> Option<(&SampleBuffer, Option<&Shape>)> {
    match &channel.sound {
        ChannelSound::Sampler(sample) => Some((sample, None)),
        ChannelSound::Shaped(shaped) => Some((&shaped.sample, Some(&shaped.shape))),
        ChannelSound::Plugin(_)
        | ChannelSound::Clips
        | ChannelSound::Instrument(_)
        | ChannelSound::Input => None,
    }
}

/// `speed` tuned by the sound settings `shape`: unchanged without them or untuned, so a
/// default channel's speeds keep their bits.
fn tuned(speed: f64, shape: Option<&Shape>) -> f64 {
    match shape {
        Some(shape) if shape.tune != 1.0 => speed * shape.tune,
        _ => speed,
    }
}

/// An instrument channel's set; `None` for every other channel.
fn instrument_sound(channel: &ChannelPlan) -> Option<&InstrumentSound> {
    match &channel.sound {
        ChannelSound::Instrument(sound) => Some(sound),
        ChannelSound::Sampler(_)
        | ChannelSound::Shaped(_)
        | ChannelSound::Plugin(_)
        | ChannelSound::Clips
        | ChannelSound::Input => None,
    }
}

/// A looping pattern's events in time order, starting from any tick.
///
/// Why a walk built fresh for every block instead of a cursor kept between blocks: the events
/// in a block are then found by position alone, so a plan swapped in between two blocks needs
/// no cursor fix-up, and a note added behind the playhead waits for the next loop instead of
/// firing late.
pub(crate) struct EventWalk<'a> {
    events: &'a [NoteEvent],
    /// Pattern length in ticks, at least 1.
    length: i64,
    /// Loop pass of `index`: pass N starts at tick N * length.
    pass: i64,
    index: usize,
}

impl<'a> EventWalk<'a> {
    /// Starts at the first event at or after absolute tick `from` (tick 0 starts pass 0).
    pub(crate) fn from_tick(pattern: &'a PatternPlan, from: Ticks) -> Self {
        let length = pattern.length.0.max(1);
        let offset = from.0.rem_euclid(length);
        Self {
            events: &pattern.events,
            length,
            pass: from.0.div_euclid(length),
            index: pattern
                .events
                .partition_point(|event| event.tick.0 < offset),
        }
    }

    /// The next event, its absolute sample and its absolute tick, if it starts before sample
    /// `end`; then moves past it. Each sample comes straight from the event's absolute tick, so
    /// loop passes never accumulate rounding.
    pub(crate) fn next_before(
        &mut self,
        timeline: &TempoSegment,
        end: u64,
    ) -> Option<(u64, Ticks, NoteEvent)> {
        if self.index == self.events.len() {
            self.index = 0;
            self.pass = self.pass.checked_add(1)?;
        }
        // `None` for an empty pattern.
        let event = *self.events.get(self.index)?;
        let tick = self
            .pass
            .checked_mul(self.length)?
            .checked_add(event.tick.0)?;
        let sample = timeline
            .tick_to_sample(Ticks(tick))
            .filter(|&sample| sample < end)?;
        self.index += 1;
        Some((sample, Ticks(tick), event))
    }
}

/// Renders `out`, at most [`CHUNK`] frames, as the frames that start at song sample `start`
/// on `timeline` and at frame `clock` of the output clock, overwriting it, and raises each
/// insert's peak in `meters` (two per insert, left then right, as f32 bits). The block must not
/// cross a wrap, nor (in song mode) an automation edge: the player cuts it there. When
/// `control_point`, the block starts where automated processor parameters take their values.
/// Live notes sound in it too, `live_at` being the block's first output frame.
///
/// Why any split gives bit-identical output: notes start at samples computed from absolute
/// ticks, which notes fall in a block is decided by position alone, the block is cut at every
/// note start, voices read their sound by the clock frame, audio clips read their file by song
/// sample, automated values are worked out from the song sample of each frame, and the mixer's
/// effects and ramps run frame by frame. Nothing depends on where a block begins or ends.
#[expect(
    clippy::too_many_arguments,
    reason = "separate borrows of the player's fields, the block's place on three clocks (song \
              sample, output frame, live-note frame) and its control-point flag; the player's one \
              call per block would build a struct for them only to take it apart here"
)]
pub(crate) fn block(
    plan: &mut PlayPlan,
    runtime: &mut Runtime,
    timeline: &TempoSegment,
    start: u64,
    clock: u64,
    out: &mut [[f32; 2]],
    meters: &[AtomicU32],
    control_point: bool,
    live_at: u64,
) {
    let len = out.len().min(CHUNK);
    let inserts = plan.mixer.place.len().min(BUSES);
    runtime.fed[..inserts].fill(false);
    runtime.plugin_notes.clear();
    runtime.underruns.clear();
    runtime.failures.clear();
    let song = plan.playing == Playing::Song;
    if song {
        automate(plan, runtime, timeline, start, len, control_point);
    } else {
        runtime.automation.governs.fill(false);
        runtime.automation.last.fill(f32::NAN);
    }
    // The live events go first: they are all on frame 0, and a processor is sent its events in
    // time order.
    runtime.take_live_plugin_notes();
    voices(plan, runtime, timeline, start, clock, live_at, len);
    runtime.mix_live(plan, live_at, len);
    let context = ProcessContext {
        sample_rate: plan.sample_rate,
        tempo: timeline.tempo(),
        ppq: plan.ppq,
        transport: TransportInfo {
            playing: true,
            song_tick: timeline.tick_at(start),
            steady_time: clock,
        },
        events: &[],
    };
    generators(plan, runtime, &context, len, None);
    inputs(plan, runtime, len);
    if song {
        audio_clips(plan, runtime, timeline, start, len);
    }
    preview(runtime, len);
    mix_down(plan, runtime, &context, &mut out[..len], meters);
}

/// Renders `out` while playback is stopped: the live notes on their channels' inserts (sampler
/// voices, and the plugin channels that play live notes), the input channels' feeds, and the
/// previewed file into the master's input, through the mixer (whose other inserts' effect tails
/// run on). `live_at` is the block's first output frame.
pub(crate) fn stopped_block(
    plan: &mut PlayPlan,
    runtime: &mut Runtime,
    context: &ProcessContext<'_>,
    out: &mut [[f32; 2]],
    meters: &[AtomicU32],
    live_at: u64,
) {
    let len = out.len().min(CHUNK);
    let inserts = plan.mixer.place.len().min(BUSES);
    runtime.fed[..inserts].fill(false);
    runtime.plugin_notes.clear();
    runtime.mix_live(plan, live_at, len);
    runtime.take_live_plugin_notes();
    generators(plan, runtime, context, len, Some(live_at));
    inputs(plan, runtime, len);
    preview(runtime, len);
    mix_down(plan, runtime, context, &mut out[..len], meters);
}

impl Runtime {
    /// Moves the live plugin events waiting into this block's plugin notes, at frame 0.
    fn take_live_plugin_notes(&mut self) {
        let Self {
            plugin_notes,
            live_plugin_notes,
            dropped_notes,
            ..
        } = self;
        for event in live_plugin_notes.drain(..) {
            if plugin_notes.len() < PLUGIN_NOTES_PER_BLOCK {
                plugin_notes.push(event);
            } else {
                *dropped_notes += 1;
            }
        }
    }
}

/// Adds the previewed file's next `len` frames into the master's bus at unity.
fn preview(runtime: &mut Runtime, len: usize) {
    let Runtime {
        buses,
        fed,
        clip_scratch,
        preview,
        ..
    } = runtime;
    let Some(preview) = preview.as_mut().filter(|preview| !preview.finished()) else {
        return;
    };
    let scratch = &mut clip_scratch[..len];
    let ready = preview.source.read(preview.frame, scratch) == SourceRead::Ready;
    preview.frame = preview.frame.saturating_add(len as u64);
    if !ready {
        return;
    }
    if let Some(bus) = feed(buses, fed, 0, len) {
        for (frame, value) in bus.iter_mut().zip(scratch.iter()) {
            frame[0] += value[0];
            frame[1] += value[1];
        }
    }
}

/// What an automation does over one block.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Governs {
    /// Before every window: the project's value rules.
    Not,
    /// Window `index` (into `windows`) shows its points.
    Window(usize),
    /// The window that ended last holds the value its end sample had.
    Hold(usize),
}

/// How `automation` governs song sample `sample`: inside a window its curve (the window that
/// starts later wins, then the higher clip ID), after every window that contains it the value
/// the window that ended last ended on, before every window not at all.
fn governs_at(automation: &AutomationPlan, timeline: &TempoSegment, sample: u64) -> Governs {
    // The last tick that starts at or before the sample: a window holds the sample when it
    // starts at or before that tick and ends after it.
    let tick = timeline.tick_at(sample);
    let windows = &automation.windows;
    let started = windows.partition_point(|window| window.start <= tick);
    if let Some(index) = (0..started).rev().find(|&index| windows[index].end > tick) {
        return Governs::Window(index);
    }
    let by_end = &automation.by_end;
    let ended = by_end.partition_point(|&index| windows[index as usize].end <= tick);
    match ended.checked_sub(1) {
        Some(last) => Governs::Hold(by_end[last] as usize),
        None => Governs::Not,
    }
}

/// The song sample a point at `tick` of `window`'s automation falls on.
fn point_sample(window: &AutomationWindow, tick: Ticks, timeline: &TempoSegment) -> i128 {
    timeline.sample_at(Ticks(window.start.0 - window.offset.0 + tick.0))
}

/// What a window's points give over the song samples `from..to`: one value, or one segment's
/// curve. Why it is kept between frames: finding it takes tick → sample divisions, and an
/// automation evaluated every frame would otherwise do them every frame.
#[derive(Debug, Clone, Copy)]
struct Segment {
    from: i128,
    to: i128,
    shape: SegmentShape,
}

#[derive(Debug, Clone, Copy)]
enum SegmentShape {
    /// Before the first point v_0, from the last on v_last.
    Flat(f64),
    /// v_i to v_{i+1} by `curve` over the `span` samples from `start` (`span` > 0).
    Curve {
        start: i128,
        span: f64,
        v0: f64,
        v1: f64,
        curve: Curve,
    },
}

impl Segment {
    /// The segment of `window`'s points that holds song sample `sample`.
    fn at(
        automation: &AutomationPlan,
        window: &AutomationWindow,
        timeline: &TempoSegment,
        sample: i128,
    ) -> Self {
        let points = &automation.points;
        let at = |index: usize| point_sample(window, points[index].tick, timeline);
        let flat = |from, to, value: f32| Segment {
            from,
            to,
            shape: SegmentShape::Flat(f64::from(value)),
        };
        let Some(last) = points.len().checked_sub(1) else {
            return flat(i128::MIN, i128::MAX, 0.0);
        };
        let first = at(0);
        if sample < first {
            return flat(i128::MIN, first, points[0].value);
        }
        let end = at(last);
        if sample >= end {
            return flat(end, i128::MAX, points[last].value);
        }
        // The last point at or before the sample, short of the last point; points are in order.
        let index = points[..last]
            .partition_point(|point| point_sample(window, point.tick, timeline) <= sample)
            - 1;
        let (start, end) = (at(index), at(index + 1));
        let (v0, v1) = (
            f64::from(points[index].value),
            f64::from(points[index + 1].value),
        );
        let shape = if end == start {
            SegmentShape::Flat(v1)
        } else {
            SegmentShape::Curve {
                start,
                span: (end - start) as f64,
                v0,
                v1,
                curve: points[index].curve,
            }
        };
        Segment {
            from: start,
            to: end,
            shape,
        }
    }

    fn holds(&self, sample: i128) -> bool {
        (self.from..self.to).contains(&sample)
    }

    /// The normalized value at song sample `sample`, inside the segment: v = v_i + (v_{i+1} −
    /// v_i)·f(x) with x = (n − S_i)/(S_{i+1} − S_i) in f64.
    fn value(&self, sample: i128) -> f64 {
        match self.shape {
            SegmentShape::Flat(value) => value,
            SegmentShape::Curve {
                start,
                span,
                v0,
                v1,
                curve,
            } => {
                // Inside the segment the offset is below the span, far below 2^63 for any valid
                // project, so it fits an i64, whose conversion gives the same f64 as the i128's
                // (both round the exact integer) without the i128 conversion's library call,
                // which costs even unused: with it as a fallback, 64 automations took 1.8 times
                // as long.
                let offset = i64::try_from(sample - start).unwrap_or(i64::MAX) as f64;
                let x = offset / span;
                v0 + (v1 - v0) * curve.shape(x)
            }
        }
    }
}

/// The parameter value of normalized `value`: lo + v·(hi − lo) in f64, rounded to f32 once.
fn parameter(automation: &AutomationPlan, value: f64) -> f32 {
    (automation.lo + value * (automation.hi - automation.lo)) as f32
}

/// Works out every automation over a song-mode block of `len` frames from song sample `start`:
/// which govern it, the per-frame values of the audio-rate ones, and, when the block starts on
/// a control point, the processor parameters whose value changed. An automation that stops
/// governing gives its processor parameter back the project's value.
fn automate(
    plan: &PlayPlan,
    runtime: &mut Runtime,
    timeline: &TempoSegment,
    start: u64,
    len: usize,
    control_point: bool,
) {
    for (index, automation) in plan.automation.iter().enumerate().take(MAX_AUTOMATIONS) {
        let governs = match automation.target {
            Some(_) => governs_at(automation, timeline, start),
            None => Governs::Not,
        };
        let state = &mut runtime.automation;
        let (window, hold) = match governs {
            Governs::Not => {
                state.governs[index] = false;
                state.last[index] = f32::NAN;
                if !state.sent[index].is_nan() {
                    state.sent[index] = f32::NAN;
                    if let Some(target) = automation.target {
                        runtime.set_processor_param(plan, target, automation.stored);
                    }
                }
                continue;
            }
            Governs::Window(window) => (&automation.windows[window], None),
            Governs::Hold(window) => {
                let window = &automation.windows[window];
                let end = timeline.sample_at(window.end);
                let value = Segment::at(automation, window, timeline, end).value(end);
                (window, Some(parameter(automation, value)))
            }
        };
        state.governs[index] = true;
        let mut segment: Option<Segment> = None;
        let mut value_at = |sample: u64| {
            hold.unwrap_or_else(|| {
                let sample = i128::from(sample);
                let current = match segment {
                    Some(current) if current.holds(sample) => current,
                    _ => *segment.insert(Segment::at(automation, window, timeline, sample)),
                };
                parameter(automation, current.value(sample))
            })
        };
        match automation.rate {
            Rate::Audio => {
                let values = &mut state.values[index * CHUNK..][..len];
                for (frame, value) in (start..).zip(values.iter_mut()) {
                    *value = value_at(frame);
                }
                state.last[index] = values.last().copied().unwrap_or(f32::NAN);
            }
            Rate::Control => {
                if control_point || state.sent[index].is_nan() {
                    let value = value_at(start);
                    if value.to_bits() != state.sent[index].to_bits() {
                        state.sent[index] = value;
                        if let Some(target) = automation.target {
                            runtime.set_processor_param(plan, target, value);
                        }
                    }
                }
                let state = &mut runtime.automation;
                state.last[index] = state.sent[index];
            }
        }
    }
}

/// Starts the block's notes and mixes the voices into the buses, frames `0..len`; `live` is
/// the block's first frame of the output frame count, where a cut finds the live voices.
fn voices(
    plan: &PlayPlan,
    runtime: &mut Runtime,
    timeline: &TempoSegment,
    start: u64,
    clock: u64,
    live: u64,
    len: usize,
) {
    let end = start.saturating_add(len as u64);
    let mut block = Block {
        plan,
        runtime,
        timeline,
        len,
        start,
        clock,
        live,
        mixed: 0,
    };
    // From the tick a new tempo starts on, for a block that starts before it: a live tempo
    // change begins on the first tick at or after the playhead ([`TempoSegment::first_tick_from`]),
    // and the note on that tick is the first one still to play.
    let Some(from) = timeline.first_tick_from(start) else {
        block.finish();
        return;
    };
    match plan.playing {
        Playing::Pattern(index) => {
            if let Some(pattern) = plan.patterns.get(index) {
                let mut walk = EventWalk::from_tick(pattern, from);
                while let Some((sample, tick, event)) = walk.next_before(timeline, end) {
                    block.start_note(sample, tick, event, None);
                }
            }
        }
        Playing::Song => {
            // Notes before the first tick at or after `end` start before `end`.
            let until = timeline.first_tick_from(end).unwrap_or(Ticks(i64::MAX));
            let mut walk = SongWalk::new(plan, from, until);
            while let Some((sample, tick, event, clip_end)) = walk.next_before(timeline, end) {
                block.start_note(sample, tick, event, Some(clip_end));
            }
        }
        // A pattern the project no longer has starts no notes.
        Playing::Nothing => {}
    }
    block.finish();
}

/// A block's voices being rendered: mixed up to `mixed`, a note start at a time.
struct Block<'a, 'b> {
    plan: &'a PlayPlan,
    runtime: &'b mut Runtime,
    timeline: &'a TempoSegment,
    /// Frames in the block.
    len: usize,
    /// The song sample of the block's first frame.
    start: u64,
    /// The output-clock frame of the block's first frame.
    clock: u64,
    /// The block's first frame of the output frame count, which times the live voices.
    live: u64,
    /// Frames rendered so far.
    mixed: usize,
}

impl Block<'_, '_> {
    /// Starts `event`, found at song sample `sample` on absolute tick `tick` in a clip that
    /// ends at `clip_end` (none in pattern mode): a sampler voice, or a note for a plugin
    /// channel.
    fn start_note(&mut self, sample: u64, tick: Ticks, event: NoteEvent, clip_end: Option<Ticks>) {
        match self.plan.channel(event.channel) {
            Some(channel) if matches!(channel.sound, ChannelSound::Plugin(_)) => {
                // A muted plugin channel is sent no new notes; the ones sounding end as planned.
                if !channel.muted {
                    self.plugin_note(sample, tick, event, clip_end);
                }
            }
            Some(channel) if matches!(channel.sound, ChannelSound::Instrument(_)) => {
                self.start_zones(sample, tick, event, clip_end);
            }
            _ => self.start_voice(sample, tick, event, clip_end),
        }
    }

    /// How many frames `event`, found at song sample `sample` on absolute tick `tick` in a clip
    /// that ends at `clip_end`, sounds ([`gate_frames`]).
    fn gate(&self, sample: u64, tick: Ticks, event: NoteEvent, clip_end: Option<Ticks>) -> u64 {
        gate_frames(self.timeline, self.plan.ppq, sample, tick, event, clip_end)
    }

    /// Queues `event` for its plugin channel: its frame in the block and how long it sounds
    /// (its [`gate`](Self::gate)).
    fn plugin_note(&mut self, sample: u64, tick: Ticks, event: NoteEvent, clip_end: Option<Ticks>) {
        let length = self.gate(sample, tick, event, clip_end);
        let frame = offset_in_block(sample, self.start, self.len);
        let note = PortEvent::Note {
            frame: u32::try_from(frame).unwrap_or(u32::MAX),
            key: event.key,
            velocity: event.velocity,
            length,
        };
        let notes = &mut self.runtime.plugin_notes;
        if notes.len() < PLUGIN_NOTES_PER_BLOCK {
            notes.push((event.channel, note));
        } else {
            self.runtime.dropped_notes += 1;
        }
    }

    /// Mixes up to song sample `sample`, then starts a sampler voice for `event` there, its
    /// note ending after its [`gate`](Self::gate) (heard only under sound settings that hear
    /// it).
    fn start_voice(&mut self, sample: u64, tick: Ticks, event: NoteEvent, clip_end: Option<Ticks>) {
        let off = self.gate(sample, tick, event, clip_end);
        let at = self.mix_to(sample);
        let live = self.live_frame(sample);
        self.runtime
            .start_voice_until(self.plan, event, at, off, live);
    }

    /// Mixes up to song sample `sample`, then starts there the instrument zones `event` sounds
    /// for its [`gate`](Self::gate).
    fn start_zones(&mut self, sample: u64, tick: Ticks, event: NoteEvent, clip_end: Option<Ticks>) {
        let off = self.gate(sample, tick, event, clip_end);
        let at = self.mix_to(sample);
        let live = self.live_frame(sample);
        self.runtime.start_zones(self.plan, event, at, off, live);
    }

    /// Song sample `sample` of the block as a frame of the output frame count.
    fn live_frame(&self, sample: u64) -> u64 {
        self.live.saturating_add(sample.saturating_sub(self.start))
    }

    /// Mixes the voices up to song sample `sample` and returns its output-clock frame.
    fn mix_to(&mut self, sample: u64) -> u64 {
        // Events never run backwards, so `max` only guards the slice bounds.
        let split = offset_in_block(sample, self.start, self.len).max(self.mixed);
        let at = self.clock.saturating_add(self.mixed as u64);
        self.runtime.mix(self.plan, at, self.mixed..split, self.len);
        self.mixed = split;
        let offset = sample.saturating_sub(self.start);
        self.clock.saturating_add(offset)
    }

    /// Mixes the rest of the block.
    fn finish(self) {
        let at = self.clock.saturating_add(self.mixed as u64);
        self.runtime
            .mix(self.plan, at, self.mixed..self.len, self.len);
    }
}

/// How many frames `event`, found at song sample `sample` on absolute tick `tick` of `timeline`
/// in a clip that ends at `clip_end` (none in pattern mode), sounds: from its start to its end
/// (a step hit lasts one step of `ppq / 4` ticks) or the clip's end, whichever comes first, at
/// the tempo in force now; at least a frame, so its end never comes before its start. The one
/// rule voices, plugin notes and MIDI out end notes by.
pub(crate) fn gate_frames(
    timeline: &TempoSegment,
    ppq: u16,
    sample: u64,
    tick: Ticks,
    event: NoteEvent,
    clip_end: Option<Ticks>,
) -> u64 {
    let gate = if event.length.0 > 0 {
        event.length
    } else {
        time::step_to_ticks(1, ppq)
    };
    let mut end = Ticks(tick.0.saturating_add(gate.0));
    if let Some(clip_end) = clip_end {
        end = end.min(clip_end);
    }
    timeline
        .tick_to_sample(end)
        .map_or(u64::MAX, |off| off.saturating_sub(sample))
        .max(1)
}

/// Renders every plugin channel's instrument over the block's first `len` frames with the notes
/// the block starts on it, and adds its sound into the channel's insert, scaled by the channel's
/// volume and pan. A muted channel's instrument still renders (its notes end on time), but its
/// sound is not added. While stopped (`stopped_at`, the block's first output frame), only the
/// channels with live notes render; the others idle.
fn generators(
    plan: &PlayPlan,
    runtime: &mut Runtime,
    context: &ProcessContext<'_>,
    len: usize,
    stopped_at: Option<u64>,
) {
    let Runtime {
        buses,
        fed,
        processors,
        plugin_notes,
        port_events,
        scratch,
        dropped_notes,
        automation,
        live_until,
        ..
    } = runtime;
    for channel in &plan.channels {
        let ChannelSound::Plugin(slot) = channel.sound else {
            continue;
        };
        if let Some(at) = stopped_at {
            let ringing = slot
                .index()
                .checked_sub(PROCESSOR_SLOTS)
                .and_then(|index| live_until.get(index))
                .is_some_and(|&until| until > at);
            let told = plugin_notes.iter().any(|(id, _)| *id == channel.id);
            if !ringing && !told {
                continue;
            }
        }
        let Some(processor) = processors.get_mut(slot.index()).and_then(Option::as_mut) else {
            continue;
        };
        port_events.clear();
        for &(_, note) in plugin_notes.iter().filter(|(id, _)| *id == channel.id) {
            if port_events.len() < NOTES_PER_PLUGIN {
                port_events.push(note);
            } else {
                *dropped_notes += 1;
            }
        }
        let out = &mut scratch[..len];
        out.fill([0.0; 2]);
        let events = ProcessContext {
            events: port_events,
            ..*context
        };
        processor.process(&events, out);
        let levels = Levels::of(channel, automation);
        if levels.varies() {
            if channel.muted {
                continue;
            }
            let Some(bus) = feed(buses, fed, usize::from(channel.bus.0), len) else {
                continue;
            };
            for (at, (frame, sound)) in bus.iter_mut().zip(out.iter()).enumerate() {
                let volume = levels.volume(at);
                let [left, right] = levels.pan_gains(at);
                *frame = [
                    frame[0] + sound[0] * (volume * left),
                    frame[1] + sound[1] * (volume * right),
                ];
            }
            continue;
        }
        let [left, right] = channel.pan_gains;
        let gains = [channel.gain * left, channel.gain * right];
        if gains == [0.0; 2] {
            continue;
        }
        let Some(bus) = feed(buses, fed, usize::from(channel.bus.0), len) else {
            continue;
        };
        for (frame, sound) in bus.iter_mut().zip(out.iter()) {
            *frame = [
                frame[0] + sound[0] * gains[0],
                frame[1] + sound[1] * gains[1],
            ];
        }
    }
}

/// Adds every audio clip that sounds in the song-mode block of `len` frames from song sample
/// `start` into its channel's insert: song sample n of a clip plays frame
/// `frame_offset + n − s(start)` of its source, at (clip gain × channel volume) × pan, a mono
/// file through the channel pan law and a stereo one through the balance law. A source that is
/// not ready silences its clip for the block and is counted; one that failed is named.
fn audio_clips(
    plan: &PlayPlan,
    runtime: &mut Runtime,
    timeline: &TempoSegment,
    start: u64,
    len: usize,
) {
    let song = &plan.song;
    let end = start.saturating_add(len as u64);
    // A clip that starts at or before `from - longest` has ended by the block; one that starts
    // at or after `until` starts after it.
    let from = timeline.tick_at(start);
    let until = timeline
        .first_tick_at_or_after(end)
        .unwrap_or(Ticks(i64::MAX));
    let reach = from.0.saturating_sub(song.audio_longest.0);
    let first = song.audio.partition_point(|clip| clip.start.0 < reach);
    let last = song.audio.partition_point(|clip| clip.start < until);
    let Runtime {
        buses,
        fed,
        sources,
        clip_scratch,
        automation,
        underruns,
        underrun_count,
        failures,
        ..
    } = runtime;
    for clip in song.audio.get(first..last.max(first)).unwrap_or_default() {
        let (clip_start, clip_end) = (timeline.sample_at(clip.start), timeline.sample_at(clip.end));
        let lo = clip_start.max(i128::from(start));
        let hi = clip_end.min(i128::from(end));
        if lo >= hi {
            continue;
        }
        let Some(channel) = plan.channel(clip.channel) else {
            continue;
        };
        if channel.muted {
            continue;
        }
        let Some(source) = clip
            .source
            .and_then(|slot| sources.get_mut(slot.index()))
            .and_then(Option::as_mut)
        else {
            continue;
        };
        // In the block, so both fit a usize; the file frame fits a u64 as the clip's start does.
        let (from_frame, to_frame) = (
            (lo - i128::from(start)) as usize,
            (hi - i128::from(start)) as usize,
        );
        let file_frame = clip
            .frame_offset
            .saturating_add(u64::try_from(lo - clip_start).unwrap_or(0));
        let scratch = &mut clip_scratch[..to_frame - from_frame];
        match source.read(file_frame, scratch) {
            SourceRead::Ready => {}
            SourceRead::NotReady => {
                note_underrun(underruns, underrun_count, clip);
                continue;
            }
            SourceRead::Failed => {
                if failures.len() < UNDERRUNS_PER_BLOCK {
                    failures.push(clip.clip);
                }
                continue;
            }
        }
        let Some(bus) = feed(buses, fed, usize::from(channel.bus.0), len) else {
            continue;
        };
        let stereo = source.channels() == 2;
        let levels = Levels::of(channel, automation);
        for (offset, (frame, value)) in bus[from_frame..to_frame]
            .iter_mut()
            .zip(scratch.iter())
            .enumerate()
        {
            let at = from_frame + offset;
            add_panned(
                frame,
                *value,
                clip.gain * levels.volume(at),
                stereo,
                &levels,
                at,
            );
        }
    }
}

/// Reads the next `len` frames of every input feed, in channel order, and adds each into its
/// input channel's insert at the channel's volume and pan, as an audio clip's frames are: a
/// mono input through the channel pan law, a stereo one through the balance law. A feed whose
/// channel the plan lacks, is not an input channel or is muted is read all the same, so it stays
/// at the frame being played; its frames go nowhere.
fn inputs(plan: &PlayPlan, runtime: &mut Runtime, len: usize) {
    let Runtime {
        buses,
        fed,
        automation,
        inputs,
        input_scratch,
        ..
    } = runtime;
    let Some(inputs) = inputs.as_mut() else {
        return;
    };
    let scratch = &mut input_scratch[..len];
    for (id, input) in inputs.iter_mut() {
        input.read(scratch);
        let Some(channel) = plan
            .channel(id)
            .filter(|channel| matches!(channel.sound, ChannelSound::Input) && !channel.muted)
        else {
            continue;
        };
        let Some(bus) = feed(buses, fed, usize::from(channel.bus.0), len) else {
            continue;
        };
        let stereo = input.channels() == 2;
        let levels = Levels::of(channel, automation);
        for (at, (frame, value)) in bus.iter_mut().zip(scratch.iter()).enumerate() {
            add_panned(frame, *value, levels.volume(at), stereo, &levels, at);
        }
    }
}

/// Counts an audio clip whose source was not ready, and names it when the block has room.
fn note_underrun(underruns: &mut Vec<ClipId>, count: &mut u64, clip: &AudioClipPlan) {
    *count += 1;
    if underruns.len() < UNDERRUNS_PER_BLOCK {
        underruns.push(clip.clip);
    }
}

/// Runs every insert of the plan in its order over the block's `out.len()` frames, and writes
/// the master into `out`.
///
/// An insert with nothing in its bus and no effects is skipped: its output would be silence.
/// Why skip rather than run it: adding a silent bus into another is not quite a no-op (0.0 +
/// −0.0 is 0.0, so a negative zero would change sign), and skipping keeps a project whose
/// channels all play into the master exactly as it sounded before the mixer existed.
fn mix_down(
    plan: &mut PlayPlan,
    runtime: &mut Runtime,
    context: &ProcessContext,
    out: &mut [[f32; 2]],
    meters: &[AtomicU32],
) {
    let len = out.len();
    out.fill([0.0; 2]);
    let Runtime {
        buses,
        fed,
        processors,
        automation,
        ..
    } = runtime;
    for insert in &mut plan.mixer.order {
        let id = usize::from(insert.id.0);
        let quiet = !fed.get(id).copied().unwrap_or(false) && insert.effects.is_empty();
        // Effects run on a silent bus too: their tails go on without input.
        let Some(bus) = (!quiet).then(|| feed(buses, fed, id, len)).flatten() else {
            advance_ramps(insert, len);
            continue;
        };
        for slot in &insert.effects {
            if let Some(processor) = processors.get_mut(slot.index()).and_then(Option::as_mut) {
                processor.process(context, bus);
            }
        }
        apply_fader(
            &mut insert.fader,
            bus,
            automation.values(insert.fader_automation),
        );
        apply_pan(
            &mut insert.pan,
            bus,
            automation.values(insert.pan_automation),
        );
        if !insert.audible {
            // Muted or silenced by solo: nothing comes out, sends included, but the effects
            // above have run, so their tails go on.
            bus.fill([0.0; 2]);
            for send in &mut insert.sends {
                send.amount.advance(len);
            }
            continue;
        }
        raise_meter(meters, id, bus);
        if insert.id.is_master() {
            out.copy_from_slice(bus);
        }
        for send in &mut insert.sends {
            let to = usize::from(send.to.0);
            if feed(buses, fed, to, len).is_some() {
                if let Some((from, into)) = bus_pair(buses, id, to, len) {
                    add_scaled(from, into, &send.amount, automation.values(send.automation));
                }
            }
            send.amount.advance(len);
        }
    }
}

/// The first `len` frames of bus `id`, zeroed if this is the first time the block feeds it;
/// `None` for a bus the runtime does not have.
fn feed<'a>(
    buses: &'a mut [[f32; 2]],
    fed: &mut [bool],
    id: usize,
    len: usize,
) -> Option<&'a mut [[f32; 2]]> {
    let first = !std::mem::replace(fed.get_mut(id)?, true);
    let bus = buses.get_mut(id * CHUNK..)?.get_mut(..len)?;
    if first {
        bus.fill([0.0; 2]);
    }
    Some(bus)
}

/// Moves an insert's ramps past `len` frames without playing them.
fn advance_ramps(insert: &mut InsertPlan, len: usize) {
    insert.fader.advance(len);
    insert.pan.advance(len);
    for send in &mut insert.sends {
        send.amount.advance(len);
    }
}

/// Multiplies `bus` by the fader's gain: the automated position's, by the fader law, frame by
/// frame while an automation governs, else the ramp's, frame by frame while it ramps.
fn apply_fader(fader: &mut Ramp, bus: &mut [[f32; 2]], automated: Option<&[f32]>) {
    if let Some(positions) = automated {
        for (frame, &position) in bus.iter_mut().zip(positions) {
            let gain = fader_gain(position);
            *frame = [frame[0] * gain, frame[1] * gain];
        }
    } else if fader.is_settled() {
        let gain = fader.target();
        // Unity changes no bit, so it is not worth the pass.
        if gain != 1.0 {
            for frame in bus.iter_mut() {
                *frame = [frame[0] * gain, frame[1] * gain];
            }
        }
    } else {
        for (ahead, frame) in bus.iter_mut().enumerate() {
            let gain = fader.at(ahead);
            *frame = [frame[0] * gain, frame[1] * gain];
        }
    }
    fader.advance(bus.len());
}

/// Pans `bus` by the insert pan law, at the automated position frame by frame while an
/// automation governs; a centered pan leaves it untouched.
fn apply_pan(pan: &mut Ramp, bus: &mut [[f32; 2]], automated: Option<&[f32]>) {
    if let Some(positions) = automated {
        for (frame, &position) in bus.iter_mut().zip(positions) {
            if position != 0.0 {
                *frame = insert_pan(position, insert_pan_gains(position), *frame);
            }
        }
    } else if pan.is_settled() {
        let position = pan.target();
        if position != 0.0 {
            let gains = insert_pan_gains(position);
            for frame in bus.iter_mut() {
                *frame = insert_pan(position, gains, *frame);
            }
        }
    } else {
        for (ahead, frame) in bus.iter_mut().enumerate() {
            let position = pan.at(ahead);
            if position != 0.0 {
                *frame = insert_pan(position, insert_pan_gains(position), *frame);
            }
        }
    }
    pan.advance(bus.len());
}

/// Adds `from` × the send amount into `into`: the automated amount frame by frame while an
/// automation governs, else the ramp's, frame by frame while it ramps.
fn add_scaled(from: &[[f32; 2]], into: &mut [[f32; 2]], amount: &Ramp, automated: Option<&[f32]>) {
    if let Some(amounts) = automated {
        for ((target, source), &gain) in into.iter_mut().zip(from).zip(amounts) {
            *target = [target[0] + source[0] * gain, target[1] + source[1] * gain];
        }
    } else if amount.is_settled() {
        let gain = amount.target();
        for (target, source) in into.iter_mut().zip(from) {
            *target = [target[0] + source[0] * gain, target[1] + source[1] * gain];
        }
    } else {
        for (ahead, (target, source)) in into.iter_mut().zip(from).enumerate() {
            let gain = amount.at(ahead);
            *target = [target[0] + source[0] * gain, target[1] + source[1] * gain];
        }
    }
}

/// Bus `from` to read and bus `to` to add into, the first `len` frames of each; `None` when
/// they are the same bus (validation refuses a send to itself) or out of range.
fn bus_pair(
    buses: &mut Frames,
    from: usize,
    to: usize,
    len: usize,
) -> Option<(&Frames, &mut Frames)> {
    if from == to || from.max(to) >= BUSES {
        return None;
    }
    if from < to {
        let (low, high) = buses.split_at_mut(to * CHUNK);
        Some((&low[from * CHUNK..][..len], &mut high[..len]))
    } else {
        let (low, high) = buses.split_at_mut(from * CHUNK);
        Some((&high[..len], &mut low[to * CHUNK..][..len]))
    }
}

/// Raises insert `id`'s two meters to the block's peaks, if higher.
///
/// Why `fetch_max` on the bits: a non-negative f32's bits order like its value, so the atomic
/// keeps the loudest peak until the UI takes it, with no lock and no lost update.
fn raise_meter(meters: &[AtomicU32], id: usize, bus: &[[f32; 2]]) {
    let mut peaks = [0.0_f32; 2];
    for frame in bus {
        peaks = [peaks[0].max(frame[0].abs()), peaks[1].max(frame[1].abs())];
    }
    for (side, peak) in peaks.into_iter().enumerate() {
        if peak > 0.0 {
            if let Some(meter) = meters.get(2 * id + side) {
                meter.fetch_max(peak.to_bits(), AtomicOrdering::Relaxed);
            }
        }
    }
}

/// The song's note starts in time order, from tick `from` up to (not including) `until`.
///
/// Why no cursor per clip: the next note is the earliest, over the clips that overlap the
/// span, of each clip's first note after the last one started (one binary search in the shared
/// event list). That needs no storage, so any number of clips may overlap, and like
/// [`EventWalk`] it starts from position alone, so a plan swapped in between blocks needs no
/// fix-up. Notes on one tick start in (clip timeline index, event index) order, so voices are
/// allocated and summed the same way however a render is split.
pub(crate) struct SongWalk<'a> {
    plan: &'a PlayPlan,
    /// Timeline index of the first clip that can sound in the span.
    first: usize,
    /// The clips that can sound in the span: every clip that starts after `from - longest`
    /// and before `until`.
    clips: &'a [ClipPlan],
    from: Ticks,
    /// The last note started: its tick, clip timeline index and event index.
    last: Option<(Ticks, usize, usize)>,
}

impl<'a> SongWalk<'a> {
    pub(crate) fn new(plan: &'a PlayPlan, from: Ticks, until: Ticks) -> Self {
        let song = &plan.song;
        // A clip that starts at or before `from - longest` has ended by `from`.
        let reach = from.0.saturating_sub(song.longest.0);
        let first = song.clips.partition_point(|clip| clip.start.0 <= reach);
        let end = song.clips.partition_point(|clip| clip.start < until);
        Self {
            plan,
            first,
            clips: song.clips.get(first..end.max(first)).unwrap_or_default(),
            from,
            last: None,
        }
    }

    /// The next note, its song sample, its song tick and the end of its clip, if it starts
    /// before sample `end`; then moves past it.
    pub(crate) fn next_before(
        &mut self,
        timeline: &TempoSegment,
        end: u64,
    ) -> Option<(u64, Ticks, NoteEvent, Ticks)> {
        let mut next: Option<(Ticks, usize, usize)> = None;
        for (offset, clip) in self.clips.iter().enumerate() {
            let index = self.first + offset;
            // The first note this clip may start: after the last note started, in (tick, clip
            // index, event index) order.
            let (tick, min_event) = match self.last {
                None => (self.from, 0),
                Some((tick, last, event)) => match index.cmp(&last) {
                    Ordering::Less => (Ticks(tick.0.saturating_add(1)), 0),
                    Ordering::Equal => (tick, event + 1),
                    Ordering::Greater => (tick, 0),
                },
            };
            let Some(pattern) = self.plan.patterns.get(clip.pattern) else {
                continue;
            };
            if let Some((at, event)) = first_note(pattern, clip, tick, min_event) {
                // Strictly earlier only: on a tie the lower timeline index, met first, wins.
                if next.is_none_or(|(best, _, _)| at < best) {
                    next = Some((at, index, event));
                }
            }
        }
        let (tick, index, event) = next?;
        let sample = timeline.tick_to_sample(tick).filter(|&s| s < end)?;
        let clip = self.clips.get(index - self.first)?;
        let pattern = self.plan.patterns.get(clip.pattern)?;
        let note = *pattern.events.get(event)?;
        self.last = Some((tick, index, event));
        Some((sample, tick, note, clip.end))
    }
}

/// The first note `clip` plays at song tick `from` or later, as (song tick, event index): at
/// `from` itself only events from `min_event` on. The window rule: song tick T plays the event
/// on data tick (T - start + offset) mod L, for T in `start..end`.
fn first_note(
    pattern: &PatternPlan,
    clip: &ClipPlan,
    from: Ticks,
    min_event: usize,
) -> Option<(Ticks, usize)> {
    let events = &pattern.events;
    let (from, min_event) = if from < clip.start {
        (clip.start, 0)
    } else {
        (from, min_event)
    };
    if from >= clip.end || events.is_empty() {
        return None;
    }
    let length = pattern.length.0.max(1);
    let position = (from.0 - clip.start.0).checked_add(clip.offset.0)?;
    let (pass, data) = (position.div_euclid(length), position.rem_euclid(length));
    let at = events.partition_point(|event| event.tick.0 < data);
    let after = events.partition_point(|event| event.tick.0 <= data);
    // Events from `min_event` on, among those on this very data tick; then the later ones.
    let index = at.max(min_event.min(after));
    let (pass, index) = if index == events.len() {
        (pass.checked_add(1)?, 0)
    } else {
        (pass, index)
    };
    let data_start = pass
        .checked_mul(length)?
        .checked_add(events[index].tick.0)?;
    let tick = clip.start.0.checked_add(data_start - clip.offset.0)?;
    (tick < clip.end.0).then_some((Ticks(tick), index))
}

/// How many frames `voice` has played by output-clock frame `at`, or `None` once it has read
/// past the end of `sample`.
/// The frames `voice` has played of `sample` by frame `at`, while it still sounds there: read
/// as the sampler reads, or with the sound settings `shape` when the channel has them.
fn voice_elapsed(
    voice: &Voice,
    at: u64,
    sample: &SampleBuffer,
    shape: Option<&Shape>,
) -> Option<u64> {
    let elapsed = at.saturating_sub(voice.start);
    let sounding = match shape {
        None => {
            sampler::is_sounding(sample, elapsed, &voice.pitch)
                && ended(0, 0, elapsed, None, voice.cut).is_some()
        }
        Some(shape) => shape
            .sampler_frame(sample, elapsed, voice.off, voice.cut, &voice.pitch)
            .is_some(),
    };
    sounding.then_some(elapsed)
}

/// Where absolute sample `at` falls in a block of `len` frames that starts at `start`, clamped
/// to `0..=len`.
fn offset_in_block(at: u64, start: u64, len: usize) -> usize {
    usize::try_from(at.saturating_sub(start)).map_or(len, |offset| offset.min(len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{MixerPlan, SongPlan};
    use crate::project::InsertId;

    /// A plan with channel 1 playing `len` frames of 1.0 at `gain` into the master, and no
    /// patterns.
    fn plan_with_sample(len: usize, gain: f32) -> PlayPlan {
        PlayPlan {
            sample_rate: 44100,
            ppq: 96,
            channels: vec![ChannelPlan {
                id: ChannelId(1),
                sound: ChannelSound::Sampler(SampleBuffer::from(vec![1.0; len])),
                gain,
                volume: gain,
                muted: gain == 0.0,
                plays_live: true,
                bus: InsertId::MASTER,
                pan_gains: [1.0; 2],
                pan: 0.0,
                volume_automation: None,
                pan_automation: None,
                midi_out: None,
                bend_range: crate::project::DEFAULT_BEND_RANGE,
                wheel_filter: None,
                cut_itself: false,
            }],
            patterns: Vec::new(),
            playing: Playing::Nothing,
            song: SongPlan::default(),
            mixer: MixerPlan::default(),
            automation: Vec::new(),
            automation_edges: Vec::new(),
            control_rate: false,
            live_channel: None,
        }
    }

    /// The voices of `runtime` mixed into frames `0..frames` of the master's bus, from
    /// output-clock frame `at`.
    fn mix_master(runtime: &mut Runtime, plan: &PlayPlan, at: u64, frames: usize) -> Vec<[f32; 2]> {
        runtime.buses[..CHUNK].fill([0.0; 2]);
        runtime.mix(plan, at, 0..frames, frames);
        runtime.buses[..frames].to_vec()
    }

    fn note_on(channel: u32) -> NoteEvent {
        NoteEvent {
            tick: Ticks(0),
            channel: ChannelId(channel),
            velocity: 1.0,
            speed: 1.0,
            key: 60,
            length: Ticks(0),
        }
    }

    fn start_note(runtime: &mut Runtime, plan: &PlayPlan, at: u64) {
        runtime.start_voice_until(plan, note_on(1), at, u64::MAX, at);
    }

    fn starts(runtime: &Runtime) -> Vec<Option<u64>> {
        runtime
            .voices
            .iter()
            .map(|slot| slot.map(|voice| voice.start))
            .collect()
    }

    #[test]
    fn a_voice_is_free_from_the_sample_its_note_ends() {
        let plan = plan_with_sample(10, 1.0);
        let mut runtime = Runtime::new();
        start_note(&mut runtime, &plan, 0); // slot 0 sounds during samples 0..10
        start_note(&mut runtime, &plan, 9); // slot 0 is still sounding, so slot 1
        start_note(&mut runtime, &plan, 10); // slot 0 has just ended, so it is reused
        assert_eq!(starts(&runtime)[..3], [Some(10), Some(9), None]);
    }

    #[test]
    fn a_full_pool_takes_over_the_voice_that_started_first() {
        let plan = plan_with_sample(1000, 1.0);
        let mut runtime = Runtime::new();
        start_note(&mut runtime, &plan, 0); // slot 0 sounds during samples 0..1000
        for at in 501..500 + VOICE_COUNT as u64 {
            start_note(&mut runtime, &plan, at); // slots 1.. start at 501, 502, ...
        }
        start_note(&mut runtime, &plan, 1000); // slot 0 has ended: reused
        start_note(&mut runtime, &plan, 1001); // pool full: slot 1 (started at 501) is the oldest
        let starts = starts(&runtime);
        assert_eq!(starts[..3], [Some(1000), Some(1001), Some(502)]);
        assert!(starts.iter().all(Option::is_some));
    }

    #[test]
    fn a_note_on_a_missing_or_muted_channel_takes_no_voice() {
        let mut runtime = Runtime::new();
        runtime.start_voice_until(&plan_with_sample(10, 1.0), note_on(9), 0, u64::MAX, 0);
        runtime.start_voice_until(&plan_with_sample(10, 0.0), note_on(1), 0, u64::MAX, 0);
        assert!(starts(&runtime).iter().all(Option::is_none));
    }

    /// Plugin channel 2, in the first live plugin slot, playing live notes.
    fn plugin_channel() -> ChannelPlan {
        ChannelPlan {
            id: ChannelId(2),
            sound: ChannelSound::Plugin(ProcessorSlot(PROCESSOR_SLOTS as u16)),
            gain: 1.0,
            volume: 1.0,
            muted: false,
            plays_live: true,
            bus: InsertId::MASTER,
            pan_gains: [1.0; 2],
            pan: 0.0,
            volume_automation: None,
            pan_automation: None,
            midi_out: None,
            bend_range: crate::project::DEFAULT_BEND_RANGE,
            wheel_filter: None,
            cut_itself: false,
        }
    }

    // After a reset, a key held on a plugin channel is struck again, once: not while its
    // note-on still waits, not on a muted plugin channel, and never for a sampler's key.
    #[test]
    fn a_reset_strikes_each_held_plugin_key_once() {
        let mut plan = plan_with_sample(10, 1.0);
        plan.channels.push(plugin_channel());
        let mut runtime = Runtime::new();
        let rest = Controls::default();
        runtime.live_on(&plan, 0, ChannelId(2), 60, 0.5, 0, None, rest);
        runtime.live_on(&plan, 0, ChannelId(1), 62, 1.0, 0, None, rest);
        let struck = PortEvent::NoteOn {
            frame: 0,
            key: 60,
            velocity: 0.5,
        };
        runtime.strike_held(&plan);
        assert_eq!(runtime.live_plugin_notes, [(ChannelId(2), struck)]);
        runtime.live_plugin_notes.clear();
        runtime.strike_held(&plan);
        assert_eq!(runtime.live_plugin_notes, [(ChannelId(2), struck)]);
        runtime.live_plugin_notes.clear();
        plan.channels[1].muted = true;
        runtime.strike_held(&plan);
        assert!(runtime.live_plugin_notes.is_empty());
    }

    // A muted plugin channel is sent the bend and the wheel, off rest and back, as an unmuted one
    // is (found 2026-10-26): its plugin holds what the device
    // does when it is unmuted. It is sent no note.
    #[test]
    fn a_muted_plugin_channel_is_sent_the_bend_and_the_wheel() {
        let mut plan = plan_with_sample(10, 1.0);
        plan.channels.push(ChannelPlan {
            muted: true,
            ..plugin_channel()
        });
        let mut runtime = Runtime::new();
        let channel = ChannelId(2);
        runtime.live_on(&plan, 0, channel, 60, 0.5, 0, None, Controls::default());
        runtime.live_bend(&plan, 0, channel, 8191, 0);
        runtime.live_wheel(&plan, 0, channel, 100, 0);
        runtime.live_bend(&plan, 0, channel, 0, 512);
        runtime.live_wheel(&plan, 0, channel, 0, 512);
        let bend = |value| PortEvent::Bend { frame: 0, value };
        let wheel = |value| PortEvent::Wheel { frame: 0, value };
        assert_eq!(
            runtime.live_plugin_notes,
            [
                (channel, bend(8191)),
                (channel, wheel(100)),
                (channel, bend(0)),
                (channel, wheel(0)),
            ]
        );
    }

    #[test]
    fn muting_silences_a_note_that_is_already_sounding() {
        let mut runtime = Runtime::new();
        start_note(&mut runtime, &plan_with_sample(10, 1.0), 0);
        let out = mix_master(&mut runtime, &plan_with_sample(10, 0.0), 2, 4);
        assert_eq!(out, [[0.0; 2]; 4]);
    }
}
