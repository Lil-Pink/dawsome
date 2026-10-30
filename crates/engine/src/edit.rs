//! Edits: every change to a [`Project`] is an [`Edit`] value applied by [`apply`], which returns
//! the edit that undoes it and what it touched, so the compiler rebuilds only that part.
//! [`History`] keeps the undo and redo stacks.

mod automation;
mod channels;
pub mod copies;
mod links;
mod mixer;
mod patterns;
mod playlist;
pub mod sound_edits;
pub mod takes;

use std::collections::{BTreeSet, VecDeque};
use std::mem;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;

use crate::project::{
    Automation, AutomationId, AutomationPoint, AutomationTarget, Channel, ChannelId, ChannelKind,
    Clip, ClipId, ClipStretch, Controller, Curve, EffectId, InputSource, InsertId, LoopRange,
    MidiOut, Note, Pattern, PatternId, ProcessorOwner, ProcessorRef, Project, Sound, SoundEdits,
    SoundSource, Track, TrackId, ValidationError, DEFAULT_KEY, MAX_KEY, MAX_VOLUME, PAN_RANGE,
    TEMPO_RANGE,
};
use crate::time::{self, MilliBpm, Ticks};

/// Velocity of a step switched on in the step grid: 100 on the 0..128 velocity scale, the
/// same as a note drawn in the piano roll, and the default the project-file note record uses.
pub const STEP_VELOCITY: f32 = 100.0 / 128.0;

/// How many edits [`History`] can undo, and redo; the oldest are forgotten first.
pub const HISTORY_LIMIT: usize = 500;

/// One change to a project, as plain data. Applying an edit either changes exactly what it
/// names or fails and changes nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// Switches a step on (a zero-length note at `step * ppq/4` with [`STEP_VELOCITY`] and the
    /// default key) when the channel has no note starting on that tick, or off, removing every
    /// note the channel starts there, whatever its key or length (a step shows lit for any).
    ToggleStep {
        pattern: PatternId,
        channel: ChannelId,
        step: u32,
    },
    /// Removes each `remove` note, matched on every field, then adds each `add` note at its
    /// sorted place. Every note gesture is one of these (a move removes the originals and adds
    /// the moved copies), and its inverse swaps the two lists. Afterwards no two notes may
    /// share a channel, tick and key.
    ReplaceNotes {
        pattern: PatternId,
        remove: Vec<Note>,
        add: Vec<Note>,
    },
    SetChannelVolume {
        channel: ChannelId,
        value: f32,
    },
    SetChannelPan {
        channel: ChannelId,
        value: f32,
    },
    SetChannelMute {
        channel: ChannelId,
        value: bool,
    },
    /// Gives sampler channel `id` the sound file at `path` (relative to the project's folder,
    /// as a new sampler names it; empty for no sound). Its inverse is the same edit with the
    /// old path. Refused for a plugin channel, which plays its plugin, and for an audio
    /// channel, whose clips are measured in its file's frames. A sampler with edits given
    /// another file lets them go in the same step (they were the old file's), as a transaction
    /// whose inverse gives both back.
    SetChannelSample {
        id: ChannelId,
        path: PathBuf,
    },
    /// Gives `channel` the sound settings `sound`; its inverse is the same edit with the old
    /// ones. Refused for settings out of range or that the channel's kind cannot take
    /// ([`Sound::problem`]).
    SetChannelSound {
        channel: ChannelId,
        sound: Sound,
    },
    /// Gives a sampler or an audio channel the edit list `edits` (the whole list, so one op
    /// appended is one edit); its inverse is the same edit with the old list. Refused for a
    /// channel that plays no file of its own and for a list no sound can hold
    /// ([`SoundEdits::problem`]). [`sound_edits::append`] makes the step an op is, with the
    /// clips it moves.
    SetChannelEdits {
        channel: ChannelId,
        edits: SoundEdits,
    },
    SetTempo {
        mbpm: MilliBpm,
    },
    SetPatternBars {
        pattern: PatternId,
        bars: u32,
    },
    /// Adds an empty pattern of `bars` bars after the others, a copy of pattern `copied_from`
    /// when that is set (an ID below `id`; [`copies::unlink`] adds the notes after it). `id` must
    /// be [`Project::next_pattern_id`]: the caller reads it from there, so redoing an add gives
    /// the same ID.
    AddPattern {
        id: PatternId,
        name: String,
        bars: u32,
        copied_from: Option<PatternId>,
    },
    /// Takes the pattern added last out again and gives its ID back; the inverse of
    /// [`Edit::AddPattern`]. Accepted only for the last ID given out, while the pattern is as
    /// added (no notes, no clip plays it), so that its own inverse restores the project byte
    /// for byte.
    UnaddPattern {
        id: PatternId,
    },
    /// Deletes a pattern and its notes. Refused for the only pattern, and for one a clip still
    /// plays: deleting those clips first, in the same transaction, makes both one undo step.
    DeletePattern {
        id: PatternId,
    },
    /// Puts a deleted pattern back under its own ID; the inverse of [`Edit::DeletePattern`].
    RestorePattern {
        id: PatternId,
        pattern: Pattern,
    },
    RenamePattern {
        id: PatternId,
        name: String,
    },
    /// Adds a sampler channel at the end of the rack. `id` must be the project's
    /// `next_channel_id`: the caller reads it from there, so redoing an add gives the same ID.
    AddSamplerChannel {
        id: ChannelId,
        name: String,
        sample: PathBuf,
    },
    /// Adds a plugin channel playing `processor` at the end of the rack. `id` must be the
    /// project's `next_channel_id`, as for [`Edit::AddSamplerChannel`].
    AddPluginChannel {
        id: ChannelId,
        name: String,
        processor: ProcessorRef,
    },
    /// Adds an instrument channel playing the set whose .sfz file is `instrument` (a path like
    /// a sampler's) at the end of the rack. `id` must be the project's `next_channel_id`, as for
    /// [`Edit::AddSamplerChannel`].
    AddInstrumentChannel {
        id: ChannelId,
        name: String,
        instrument: PathBuf,
    },
    /// Adds an input channel playing the audio input `input` at the end of the rack. `id` must
    /// be the project's `next_channel_id`, as for [`Edit::AddSamplerChannel`]. Its input is
    /// changed later with [`Edit::ReplaceChannelSound`].
    AddInputChannel {
        id: ChannelId,
        name: String,
        input: InputSource,
    },
    /// Removes a channel; the inverse of [`Edit::AddSamplerChannel`],
    /// [`Edit::AddPluginChannel`], [`Edit::AddInstrumentChannel`] and
    /// [`Edit::AddInputChannel`]. Accepted only for the newest channel, unchanged since it was
    /// added and without notes, so that its own inverse restores the project byte for byte.
    RemoveChannel {
        id: ChannelId,
    },
    /// Stores a plugin's state (standard padded base64), or clears it (`None`, the plugin's
    /// defaults), for the plugin `owner` plays. A capture at the end of a gesture and its undo
    /// are both this edit; the live instance is brought in line by the compile that follows.
    SetProcessorState {
        owner: ProcessorOwner,
        state: Option<String>,
    },
    /// Places `clips` on the playlist with IDs `first`, `first + 1`, ... in order. `first` must
    /// be the playlist's `next_clip_id`: the caller reads it from there, so a redo gives the
    /// same IDs.
    PlaceClips {
        first: ClipId,
        clips: Vec<Clip>,
    },
    /// Removes the newest `count` clips, IDs `first..first + count`, and gives their IDs back;
    /// the inverse of [`Edit::PlaceClips`] and [`Edit::DuplicateClips`].
    UnplaceClips {
        first: ClipId,
        count: u32,
    },
    /// Copies the clips `ids` one selection length later (from the earliest start to the latest
    /// end), on the same tracks, with new IDs from `first` (the playlist's `next_clip_id`) in
    /// (start, id) order.
    DuplicateClips {
        ids: Vec<ClipId>,
        first: ClipId,
    },
    DeleteClips {
        ids: Vec<ClipId>,
    },
    /// Puts deleted clips back under their own IDs; the inverse of [`Edit::DeleteClips`].
    RestoreClips {
        clips: Vec<(ClipId, Clip)>,
    },
    /// Moves each clip to an absolute start tick and track. One invalid target refuses all.
    MoveClips {
        moves: Vec<(ClipId, Ticks, TrackId)>,
    },
    /// Sets a clip's length, which moves its right edge.
    ResizeClip {
        id: ClipId,
        length: Ticks,
    },
    /// Draws, moves, switches on or off, or clears the loop range.
    SetLoop {
        value: Option<LoopRange>,
    },
    /// Adds an audio channel playing `sample` (into the master) and one audio clip of it, from
    /// the file's first frame at unity gain: what dropping a sound file on the playlist does.
    /// `channel` and `clip` must be the project's next channel and clip IDs.
    AddAudioClip {
        channel: ChannelId,
        name: String,
        sample: PathBuf,
        clip: ClipId,
        track: TrackId,
        start: Ticks,
        length: Ticks,
    },
    /// Takes the newest audio channel and its one clip out again, as they were added, and gives
    /// their IDs back; the inverse of [`Edit::AddAudioClip`].
    UnaddAudioClip {
        channel: ChannelId,
        clip: ClipId,
    },
    /// Sets an audio clip's start, length and frame offset at once (absolute values): a left
    /// trim moves the start and the offset together.
    TrimAudioClip {
        clip: ClipId,
        start: Ticks,
        length: Ticks,
        frame_offset: u64,
    },
    /// Sets an audio clip's gain, in [`crate::project::CLIP_GAIN_RANGE`].
    SetClipGain {
        clip: ClipId,
        gain: f32,
    },
    /// Stretches an audio clip (or ends its stretch, `None`) and sets its frame offset and
    /// length with it, absolute values: both count the stretched file's frames, so a new
    /// stretch moves them together.
    SetClipStretch {
        clip: ClipId,
        stretch: Option<ClipStretch>,
        frame_offset: u64,
        length: Ticks,
    },
    /// Links a parameter to a MIDI controller, or unlinks it (`None`). A controller linked to
    /// another parameter moves to this one.
    SetMidiLink {
        target: AutomationTarget,
        link: Option<Controller>,
    },
    /// Makes `channel` play the notes of MIDI input `port` whatever channel is selected, or
    /// unlinks it (`None`). A port plays one channel: one linked to another channel moves to
    /// this one, and the inverse puts both back. Refused for an audio channel, which plays no
    /// notes.
    SetChannelMidiInput {
        channel: ChannelId,
        port: Option<String>,
    },
    /// Gives an input channel a MIDI out, changes it, or takes it away (`None`). Refused for any
    /// other kind and for a setting out of range. Taken away from a channel linked to a MIDI
    /// input, the link goes in the same step (the channel then takes no notes); the inverse
    /// gives both back.
    SetChannelMidiOut {
        channel: ChannelId,
        out: Option<MidiOut>,
    },
    /// Makes pattern clip `id` play pattern `pattern` instead of the one it plays; its inverse
    /// is the same edit with the old pattern. Its place, length and window stay. Refused for a
    /// clip of another kind, a pattern that does not exist and the pattern it already plays.
    SetClipPattern {
        id: ClipId,
        pattern: PatternId,
    },
    /// Mutes or unmutes clips of any kind.
    SetClipMute {
        ids: Vec<ClipId>,
        muted: bool,
    },
    /// Adds an automation of `target` with `points` and one clip of it. `automation` and
    /// `clip` must be the next automation and clip IDs; the target must exist and have no
    /// automation yet.
    AddAutomationClip {
        automation: AutomationId,
        clip: ClipId,
        target: AutomationTarget,
        track: TrackId,
        start: Ticks,
        length: Ticks,
        points: Vec<AutomationPoint>,
    },
    /// Takes the newest automation and its one clip out again and gives their IDs back; the
    /// inverse of [`Edit::AddAutomationClip`].
    UnaddAutomationClip {
        automation: AutomationId,
        clip: ClipId,
    },
    /// Removes an automation and every clip of it.
    DeleteAutomationClip {
        automation: AutomationId,
    },
    /// Puts a removed automation and its clips back under their own IDs; the inverse of
    /// [`Edit::DeleteAutomationClip`].
    RestoreAutomationClip {
        automation: AutomationId,
        data: Automation,
        clips: Vec<(ClipId, Clip)>,
    },
    /// Adds a point on a tick that has none.
    AddAutomationPoint {
        automation: AutomationId,
        point: AutomationPoint,
    },
    /// Moves the point on `from_tick` to `tick` with `value`, staying strictly between its
    /// neighbors.
    MoveAutomationPoint {
        automation: AutomationId,
        from_tick: Ticks,
        tick: Ticks,
        value: f32,
    },
    /// Sets the curve of the segment that starts at the point on `tick`.
    SetSegmentCurve {
        automation: AutomationId,
        tick: Ticks,
        curve: Curve,
    },
    /// Deletes the point on `tick`; an automation keeps at least one.
    DeleteAutomationPoint {
        automation: AutomationId,
        tick: Ticks,
    },
    /// Adds a track at the bottom. `id` must be the playlist's `next_track_id`.
    AddTrack {
        id: TrackId,
        name: String,
    },
    /// Renames a channel; its inverse gives the old name back. Nothing the plan holds changes.
    RenameChannel {
        id: ChannelId,
        name: String,
    },
    /// Gives `channel` another sound: its kind, file, plugin (with its state), set and input
    /// become `source`'s, its name `name` and its sound settings `sound`; it keeps its ID, notes,
    /// routing, mute, volume, pan and MIDI input. When its plugin changes (another plugin, or
    /// none), the automations of the old plugin's parameters (with their clips) and the
    /// controller links to them go in the same step, as a transaction: a plugin's parameter
    /// numbers are its own. The inverse is the same edit with what it had (and, after it, those
    /// automations and links restored), so undoing one gives the channel back as it was, a
    /// plugin's state too. Refused for an audio channel and an audio source (an audio channel's
    /// clips are measured in its file's frames), a source its kind does not hold, and settings
    /// the new kind cannot take ([`Sound::problem`]; [`Sound::kept_for`] gives those it can).
    ReplaceChannelSound {
        channel: ChannelId,
        name: String,
        source: Box<SoundSource>,
        sound: Sound,
    },
    /// Deletes a channel and its notes in every pattern; its inverse is
    /// [`Edit::RestoreChannel`]. Refused while an audio clip plays the channel (delete the
    /// clips first). The channel ID counter stays where it is, so IDs are never reused.
    DeleteChannel {
        id: ChannelId,
    },
    /// Puts a deleted channel back with its notes; its inverse is [`Edit::DeleteChannel`].
    RestoreChannel {
        id: ChannelId,
        channel: Box<Channel>,
        notes: Vec<(PatternId, Vec<Note>)>,
    },
    /// A new channel `id` (the next free ID) at the end of the rack, a copy of channel `from`
    /// with a numbered name and no MIDI input, and none of its notes; its inverse is
    /// [`Edit::Unclone`].
    CloneChannel {
        id: ChannelId,
        from: ChannelId,
    },
    /// Takes out the clone `id` of `from`, the newest channel, without notes; the ID counter goes
    /// back to it. The inverse of [`Edit::CloneChannel`].
    Unclone {
        id: ChannelId,
        from: ChannelId,
    },
    /// Deletes a track and every clip on it; its inverse is [`Edit::RestoreTrack`]. The track
    /// ID counter stays where it is.
    DeleteTrack {
        id: TrackId,
    },
    /// Puts a deleted track back with its clips; its inverse is [`Edit::DeleteTrack`].
    RestoreTrack {
        id: TrackId,
        track: Track,
        clips: Vec<(ClipId, Clip)>,
    },
    /// Removes a track; the inverse of [`Edit::AddTrack`]. Accepted only for the newest track,
    /// unmuted and without clips, so that its own inverse restores the project byte for byte.
    RemoveTrack {
        id: TrackId,
    },
    RenameTrack {
        id: TrackId,
        name: String,
    },
    SetTrackMute {
        id: TrackId,
        mute: bool,
    },
    /// Moves an insert's fader, a position in [`crate::project::FADER_RANGE`].
    SetInsertVolume {
        insert: InsertId,
        fader: f32,
    },
    SetInsertPan {
        insert: InsertId,
        pan: f32,
    },
    MuteInsert {
        insert: InsertId,
        mute: bool,
    },
    RenameInsert {
        insert: InsertId,
        name: String,
    },
    /// Replaces the whole solo set: an exclusive solo click and a click that adds or removes
    /// one insert are one edit each.
    SoloInsert {
        set: BTreeSet<InsertId>,
    },
    /// Plays a channel into another insert.
    RouteChannel {
        channel: ChannelId,
        insert: InsertId,
    },
    /// Sets the send from one insert to another to a linear amount, or removes it (`None`).
    /// Refused when `from` is the master, `to` is `from`, or `to`'s sound already reaches
    /// `from`, since the send would then close a loop.
    SetSend {
        from: InsertId,
        to: InsertId,
        amount: Option<f32>,
    },
    /// Puts a new effect in an empty slot. `id` must be the mixer's `next_effect_id`: the
    /// caller reads it from there, so a redo gives the same ID.
    AddEffect {
        insert: InsertId,
        slot: u8,
        id: EffectId,
        processor: ProcessorRef,
    },
    /// Takes the newest effect out and gives its ID back; the inverse of [`Edit::AddEffect`].
    UnaddEffect {
        id: EffectId,
    },
    RemoveEffect {
        id: EffectId,
    },
    /// Puts a removed effect back under its own ID; the inverse of [`Edit::RemoveEffect`].
    RestoreEffect {
        insert: InsertId,
        slot: u8,
        id: EffectId,
        processor: ProcessorRef,
    },
    /// Sets one parameter of an effect, or returns it to its default (`None` removes the key).
    SetEffectParam {
        effect: EffectId,
        key: String,
        value: Option<f32>,
    },
    /// Moves an effect to another slot of its insert; an effect already there takes the moved
    /// one's old slot. The effect keeps its ID, so playback keeps its running state.
    MoveEffect {
        effect: EffectId,
        slot: u8,
    },
    /// Applies the edits in order as one change, one undo step: all of them, or none when one
    /// is refused. Its inverse is the inverses of its edits, in reverse order.
    Transaction(Vec<Edit>),
}

/// What an edit changed, so the compiler can rebuild only those parts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Touched {
    pub patterns: BTreeSet<PatternId>,
    pub channels: BTreeSet<ChannelId>,
    pub tempo: bool,
    /// Tracks, clips or the loop range changed.
    pub playlist: bool,
    /// An insert, a send, the solo set or the order of an insert's effects changed.
    pub mixer: bool,
    /// Effects added, removed or with changed parameter values, which the compiler re-reads.
    pub effects: BTreeSet<EffectId>,
    /// Plugins whose stored state changed, which the compiler hands to the live instances.
    pub processors: BTreeSet<ProcessorOwner>,
    /// Automations whose target or points changed.
    pub automation: BTreeSet<AutomationId>,
}

impl Touched {
    /// Adds what `other` touched.
    pub fn merge(&mut self, other: Touched) {
        self.patterns.extend(other.patterns);
        self.channels.extend(other.channels);
        self.tempo |= other.tempo;
        self.playlist |= other.playlist;
        self.mixer |= other.mixer;
        self.effects.extend(other.effects);
        self.processors.extend(other.processors);
        self.automation.extend(other.automation);
    }

    /// Whether nothing was touched.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// A successful [`apply`]: the edit that undoes it, and what it touched.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    pub inverse: Edit,
    pub touched: Touched,
}

impl Applied {
    fn channel(inverse: Edit, id: ChannelId) -> Self {
        let touched = Touched {
            channels: BTreeSet::from([id]),
            ..Touched::default()
        };
        Self { inverse, touched }
    }

    fn pattern(inverse: Edit, id: PatternId) -> Self {
        let touched = Touched {
            patterns: BTreeSet::from([id]),
            ..Touched::default()
        };
        Self { inverse, touched }
    }

    fn playlist(inverse: Edit) -> Self {
        let touched = Touched {
            playlist: true,
            ..Touched::default()
        };
        Self { inverse, touched }
    }

    fn mixer(inverse: Edit) -> Self {
        let touched = Touched {
            mixer: true,
            ..Touched::default()
        };
        Self { inverse, touched }
    }

    /// An effect added or taken out: the mix changes, and the compiler creates or drops it.
    fn effect(inverse: Edit, id: EffectId) -> Self {
        let touched = Touched {
            mixer: true,
            effects: BTreeSet::from([id]),
            ..Touched::default()
        };
        Self { inverse, touched }
    }

    /// An effect's parameter values changed; the mix itself did not.
    fn params(inverse: Edit, id: EffectId) -> Self {
        let touched = Touched {
            effects: BTreeSet::from([id]),
            ..Touched::default()
        };
        Self { inverse, touched }
    }
}

/// Why an edit was refused. A refused edit has changed nothing.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum EditError {
    #[error("there is no channel {0}")]
    UnknownChannel(u32),
    #[error("there is no pattern {0}")]
    UnknownPattern(u32),
    #[error("step {step} is outside the pattern, which has {steps} steps")]
    StepOutsidePattern { step: u32, steps: u64 },
    #[error("a note needs a tick and a length of at least 0, a key from 0 to 127 and a velocity from 0 to 1")]
    InvalidNote,
    #[error("channel {channel} already has a note at tick {tick} on key {key}")]
    NoteExists { tick: i64, channel: u32, key: u8 },
    #[error("channel {channel} has no such note at tick {tick} on key {key}")]
    NoteMissing { tick: i64, channel: u32, key: u8 },
    #[error("the edit changes nothing")]
    NothingToChange,
    #[error("volume {0} must be a finite number from 0 to 4")]
    InvalidVolume(f32),
    #[error("pan {0} must be a number from -1 to 1")]
    InvalidPan(f32),
    #[error(
        "channel {channel}'s sound setting {setting} is out of range or not one its kind takes"
    )]
    InvalidSound { channel: u32, setting: &'static str },
    #[error("tempo {0} milli-BPM is outside 10000..=522000 (10 to 522 BPM)")]
    TempoOutOfRange(u32),
    #[error("a pattern needs at least 1 bar")]
    NoBars,
    #[error("a new pattern must take the next free id {expected}, not {got}")]
    NotNextPatternId { got: u32, expected: u32 },
    #[error("every pattern id has been used")]
    PatternIdsExhausted,
    #[error("the project already has the most patterns it can hold, 999")]
    TooManyPatterns,
    #[error(
        "pattern {0} cannot be taken out: only the pattern added last, without notes or clips, can be"
    )]
    PatternNotRemovable(u32),
    #[error("the only pattern cannot be deleted")]
    LastPattern,
    #[error("pattern {0} is still played by clips; delete them with it")]
    PatternInUse(u32),
    #[error("pattern {0} already exists")]
    PatternExists(u32),
    #[error("a new channel must take the next free id {expected}, not {got}")]
    NotNextChannelId { got: u32, expected: u32 },
    #[error("every channel id has been used")]
    ChannelIdsExhausted,
    #[error("channel {0} already exists, or its ID is not one given out")]
    ChannelExists(u32),
    #[error("channel {0} cannot be taken out: only the clone made last, without notes, can be")]
    ChannelNotRemovable(u32),
    #[error("channel {0} plays audio clips; delete them first")]
    ChannelPlaysClips(u32),
    #[error("track {0} already exists, or its ID is not one given out")]
    TrackExists(u32),
    #[error("channel {0} cannot be removed: only the newest channel, unchanged since it was added and without notes or clips, can be")]
    NotRemovable(u32),
    #[error("channel {0} plays no plugin")]
    NotAPluginChannel(u32),
    #[error("channel {0} is not a sampler")]
    NotASampler(u32),
    #[error("channel {0} plays audio clips: its sound cannot be another")]
    AudioSound(u32),
    #[error("channel {0} plays no sound file of its own to edit: only a sampler's or an audio channel's is")]
    NoSoundFile(u32),
    #[error("the edit would leave channel {0}'s sound without a frame")]
    NoFramesLeft(u32),
    #[error("channel {0} takes no notes of a MIDI device: an audio channel plays its clips, an input channel without a MIDI out its input")]
    PlaysNoNotes(u32),
    #[error(
        "channel {0} is not an input channel: only an input channel sends its notes to a MIDI out"
    )]
    NoMidiOut(u32),
    #[error("channel {channel}'s MIDI out {setting} is missing or out of range")]
    InvalidMidiOut { channel: u32, setting: &'static str },
    #[error("a MIDI input port needs a name")]
    EmptyPortName,
    #[error("there is no clip {0}")]
    UnknownClip(u32),
    #[error("clip {0} is not a pattern clip")]
    NotAPatternClip(u32),
    #[error("clip {0} plays no copy of a pattern that still exists")]
    NotACopy(u32),
    #[error("clip {0} is listed twice")]
    ClipListedTwice(u32),
    #[error("clip {0} already exists")]
    ClipExists(u32),
    #[error("new clips must take the next free ids from {expected}, not {got}")]
    NotNextClipId { got: u32, expected: u32 },
    #[error("every clip id has been used")]
    ClipIdsExhausted,
    #[error("clips {first}..{end} are not the newest clips")]
    NotNewestClips { first: u32, end: u64 },
    #[error("there is no track {0}")]
    UnknownTrack(u32),
    #[error("a new track must take the next free id {expected}, not {got}")]
    NotNextTrackId { got: u32, expected: u32 },
    #[error("every track id has been used")]
    TrackIdsExhausted,
    #[error("the playlist already has the most tracks it can hold, 500")]
    TooManyTracks,
    #[error("track {0} cannot be removed: only the newest track, unmuted and without clips, can be, and one track must remain")]
    TrackNotRemovable(u32),
    #[error("there is no insert {0}")]
    UnknownInsert(u16),
    #[error("fader {0} must be a number from 0 to 1")]
    InvalidFader(f32),
    #[error("send amount {0} must be a number from 0 to 1")]
    InvalidSendAmount(f32),
    #[error("the master sends nowhere")]
    SendFromMaster,
    #[error("insert {0} cannot send to itself")]
    SendToItself(u16),
    /// The send would close a loop: `to`'s sound already reaches `from`.
    #[error("routing loop: {to} already feeds {from}")]
    RoutingCycle { from: InsertId, to: InsertId },
    #[error("there is no effect {0}")]
    UnknownEffect(u32),
    #[error("effect slot {0} must be below 10")]
    InvalidEffectSlot(u8),
    #[error("slot {slot} of insert {insert} is taken")]
    SlotTaken { insert: u16, slot: u8 },
    #[error("a new effect must take the next free id {expected}, not {got}")]
    NotNextEffectId { got: u32, expected: u32 },
    #[error("every effect id has been used")]
    EffectIdsExhausted,
    #[error("effect {0} is not the newest effect")]
    NotNewestEffect(u32),
    #[error("effect {0} already exists")]
    EffectExists(u32),
    #[error("parameter value {0} is not a finite number")]
    InvalidParamValue(f32),
    #[error("clip {0} is not an audio clip")]
    NotAnAudioClip(u32),
    #[error("gain {0} must be a finite number from 0 to 4")]
    InvalidGain(f32),
    #[error("channel {channel} and clip {clip} are not the newest audio channel and its one clip, as added")]
    NotNewestAudioClip { channel: u32, clip: u32 },
    #[error("there is no automation {0}")]
    UnknownAutomation(u32),
    #[error("automation {0} already exists")]
    AutomationExists(u32),
    #[error("a new automation must take the next free id {expected}, not {got}")]
    NotNextAutomationId { got: u32, expected: u32 },
    #[error("every automation id has been used")]
    AutomationIdsExhausted,
    #[error("the project already has the most automations it can hold, 1024")]
    TooManyAutomations,
    #[error(
        "automation {automation} and clip {clip} are not the newest automation and its one clip"
    )]
    NotNewestAutomationClip { automation: u32, clip: u32 },
    #[error("the parameter to automate does not exist")]
    UnknownTarget,
    #[error("that parameter already has an automation clip")]
    AlreadyAutomated,
    #[error("an automation point needs a tick of at least 0, a value from 0 to 1 and a tension from -1 to 1")]
    InvalidPoint,
    #[error("there is already a point at tick {tick}")]
    PointExists { tick: i64 },
    #[error("there is no point at tick {tick}")]
    NoPointAt { tick: i64 },
    #[error("a point at tick {tick} would not stay between its neighbors")]
    PointOutOfPlace { tick: i64 },
    #[error("an automation keeps at least one point")]
    LastPoint,
    /// The edit would break a rule of the project model, such as a clip past the song's end.
    #[error(transparent)]
    Invalid(#[from] ValidationError),
}

/// Applies `edit` to `project`. On success returns the inverse edit (applying it restores the
/// project exactly) and what changed; on failure the project is untouched.
pub fn apply(project: &mut Project, edit: Edit) -> Result<Applied, EditError> {
    match edit {
        Edit::ToggleStep {
            pattern,
            channel,
            step,
        } => toggle_step(project, pattern, channel, step),
        Edit::ReplaceNotes {
            pattern,
            remove,
            add,
        } => replace_notes(project, pattern, remove, add),
        Edit::SetChannelVolume { channel, value } => {
            // `contains` is false for NaN and infinity.
            if !(0.0..=MAX_VOLUME).contains(&value) {
                return Err(EditError::InvalidVolume(value));
            }
            let old = mem::replace(&mut channel_mut(project, channel)?.volume, value);
            let inverse = Edit::SetChannelVolume {
                channel,
                value: old,
            };
            Ok(Applied::channel(inverse, channel))
        }
        Edit::SetChannelPan { channel, value } => {
            if !PAN_RANGE.contains(&value) {
                return Err(EditError::InvalidPan(value));
            }
            let old = mem::replace(&mut channel_mut(project, channel)?.pan, value);
            let inverse = Edit::SetChannelPan {
                channel,
                value: old,
            };
            Ok(Applied::channel(inverse, channel))
        }
        Edit::SetChannelMute { channel, value } => {
            let old = mem::replace(&mut channel_mut(project, channel)?.mute, value);
            let inverse = Edit::SetChannelMute {
                channel,
                value: old,
            };
            Ok(Applied::channel(inverse, channel))
        }
        Edit::SetChannelSample { id, path } => set_channel_sample(project, id, path),
        Edit::SetChannelSound { channel, sound } => {
            let shown = channel_mut(project, channel)?;
            if let Some(setting) = sound.problem(shown.kind) {
                return Err(EditError::InvalidSound {
                    channel: channel.0,
                    setting,
                });
            }
            let old = mem::replace(&mut shown.sound, sound);
            let inverse = Edit::SetChannelSound {
                channel,
                sound: old,
            };
            Ok(Applied::channel(inverse, channel))
        }
        Edit::SetChannelEdits { channel, edits } => sound_edits::set(project, channel, edits),
        Edit::SetTempo { mbpm } => {
            if !TEMPO_RANGE.contains(&mbpm.0) {
                return Err(EditError::TempoOutOfRange(mbpm.0));
            }
            let old = mem::replace(&mut project.tempo, mbpm);
            let touched = Touched {
                tempo: true,
                ..Touched::default()
            };
            let inverse = Edit::SetTempo { mbpm: old };
            Ok(Applied { inverse, touched })
        }
        Edit::SetPatternBars { pattern, bars } => {
            if bars == 0 {
                return Err(EditError::NoBars);
            }
            let old = mem::replace(&mut pattern_mut(project, pattern)?.length_bars, bars);
            let inverse = Edit::SetPatternBars { pattern, bars: old };
            Ok(Applied::pattern(inverse, pattern))
        }
        Edit::AddPattern {
            id,
            name,
            bars,
            copied_from,
        } => patterns::add(project, id, name, bars, copied_from),
        Edit::UnaddPattern { id } => patterns::unadd(project, id),
        Edit::DeletePattern { id } => patterns::delete(project, id),
        Edit::RestorePattern { id, pattern } => patterns::restore(project, id, pattern),
        Edit::RenamePattern { id, name } => patterns::rename(project, id, name),
        Edit::AddSamplerChannel { id, name, sample } => {
            add_channel(project, id, Channel::sampler(name, sample))
        }
        Edit::AddPluginChannel {
            id,
            name,
            processor,
        } => add_channel(project, id, Channel::plugin(name, processor)),
        Edit::AddInstrumentChannel {
            id,
            name,
            instrument,
        } => add_channel(project, id, Channel::instrument(name, instrument)),
        Edit::AddInputChannel { id, name, input } => {
            add_channel(project, id, Channel::input(name, input))
        }
        Edit::RemoveChannel { id } => remove_channel(project, id),
        Edit::SetProcessorState { owner, state } => set_processor_state(project, owner, state),
        Edit::Transaction(edits) => transaction(project, edits),
        Edit::PlaceClips { first, clips } => playlist::place(project, first, clips),
        Edit::UnplaceClips { first, count } => playlist::unplace(project, first, count),
        Edit::DuplicateClips { ids, first } => playlist::duplicate(project, &ids, first),
        Edit::DeleteClips { ids } => playlist::delete(project, &ids),
        Edit::RestoreClips { clips } => playlist::restore(project, clips),
        Edit::MoveClips { moves } => playlist::move_clips(project, &moves),
        Edit::ResizeClip { id, length } => playlist::resize(project, id, length),
        Edit::SetLoop { value } => playlist::set_loop(project, value),
        Edit::AddAudioClip {
            channel,
            name,
            sample,
            clip,
            track,
            start,
            length,
        } => playlist::add_audio_clip(project, channel, name, sample, clip, track, start, length),
        Edit::UnaddAudioClip { channel, clip } => {
            playlist::unadd_audio_clip(project, channel, clip)
        }
        Edit::TrimAudioClip {
            clip,
            start,
            length,
            frame_offset,
        } => playlist::trim_audio_clip(project, clip, start, length, frame_offset),
        Edit::SetClipGain { clip, gain } => playlist::set_clip_gain(project, clip, gain),
        Edit::SetClipStretch {
            clip,
            stretch,
            frame_offset,
            length,
        } => playlist::set_clip_stretch(project, clip, stretch, frame_offset, length),
        Edit::SetMidiLink { target, link } => links::set(project, target, link),
        Edit::SetChannelMidiInput { channel, port } => links::set_input(project, channel, port),
        Edit::SetChannelMidiOut { channel, out } => links::set_output(project, channel, out),
        Edit::SetClipPattern { id, pattern } => playlist::set_clip_pattern(project, id, pattern),
        Edit::SetClipMute { ids, muted } => playlist::set_clip_mute(project, &ids, muted),
        Edit::AddAutomationClip {
            automation,
            clip,
            target,
            track,
            start,
            length,
            points,
        } => automation::add_clip(
            project, automation, clip, target, track, start, length, points,
        ),
        Edit::UnaddAutomationClip { automation, clip } => {
            automation::unadd_clip(project, automation, clip)
        }
        Edit::DeleteAutomationClip { automation } => automation::delete(project, automation),
        Edit::RestoreAutomationClip {
            automation,
            data,
            clips,
        } => automation::restore(project, automation, data, clips),
        Edit::AddAutomationPoint { automation, point } => {
            automation::add_point(project, automation, point)
        }
        Edit::MoveAutomationPoint {
            automation,
            from_tick,
            tick,
            value,
        } => automation::move_point(project, automation, from_tick, tick, value),
        Edit::SetSegmentCurve {
            automation,
            tick,
            curve,
        } => automation::set_curve(project, automation, tick, curve),
        Edit::DeleteAutomationPoint { automation, tick } => {
            automation::delete_point(project, automation, tick)
        }
        Edit::AddTrack { id, name } => playlist::add_track(project, id, name),
        Edit::RemoveTrack { id } => playlist::remove_track(project, id),
        Edit::RenameChannel { id, name } => channels::rename(project, id, name),
        Edit::ReplaceChannelSound {
            channel,
            name,
            source,
            sound,
        } => channels::replace_sound(project, channel, name, *source, sound),
        Edit::DeleteChannel { id } => channels::delete(project, id),
        Edit::RestoreChannel { id, channel, notes } => {
            channels::restore(project, id, *channel, notes)
        }
        Edit::CloneChannel { id, from } => channels::clone(project, id, from),
        Edit::Unclone { id, from } => channels::unclone(project, id, from),
        Edit::DeleteTrack { id } => channels::delete_track(project, id),
        Edit::RestoreTrack { id, track, clips } => {
            channels::restore_track(project, id, track, clips)
        }
        Edit::RenameTrack { id, name } => playlist::rename_track(project, id, name),
        Edit::SetTrackMute { id, mute } => playlist::set_track_mute(project, id, mute),
        Edit::SetInsertVolume { insert, fader } => mixer::set_fader(project, insert, fader),
        Edit::SetInsertPan { insert, pan } => mixer::set_pan(project, insert, pan),
        Edit::MuteInsert { insert, mute } => mixer::set_mute(project, insert, mute),
        Edit::RenameInsert { insert, name } => mixer::rename(project, insert, name),
        Edit::SoloInsert { set } => mixer::solo(project, set),
        Edit::RouteChannel { channel, insert } => mixer::route_channel(project, channel, insert),
        Edit::SetSend { from, to, amount } => mixer::set_send(project, from, to, amount),
        Edit::AddEffect {
            insert,
            slot,
            id,
            processor,
        } => mixer::add_effect(project, insert, slot, id, processor),
        Edit::UnaddEffect { id } => mixer::unadd_effect(project, id),
        Edit::RemoveEffect { id } => mixer::remove_effect(project, id),
        Edit::RestoreEffect {
            insert,
            slot,
            id,
            processor,
        } => mixer::restore_effect(project, insert, slot, id, processor),
        Edit::SetEffectParam { effect, key, value } => {
            mixer::set_effect_param(project, effect, key, value)
        }
        Edit::MoveEffect { effect, slot } => mixer::move_effect(project, effect, slot),
    }
}

/// Whether applying `edit` left the project as it was, given the `inverse` that apply returned:
/// such an edit is its own inverse (a value set to the one it had, notes replaced by the same
/// notes, a transaction whose edits cancel).
///
/// The reverse holds for every edit but a step switched on, whose inverse is the same toggle
/// although the step changed; so an edit that toggles a step, alone or in a transaction, never
/// counts as unchanged.
pub fn changed_nothing(edit: &Edit, inverse: &Edit) -> bool {
    edit == inverse && !toggles_a_step(edit)
}

fn toggles_a_step(edit: &Edit) -> bool {
    match edit {
        Edit::ToggleStep { .. } => true,
        Edit::Transaction(edits) => edits.iter().any(toggles_a_step),
        _ => false,
    }
}

fn channel_mut(project: &mut Project, id: ChannelId) -> Result<&mut Channel, EditError> {
    project
        .channels
        .get_mut(&id)
        .ok_or(EditError::UnknownChannel(id.0))
}

fn pattern_mut(project: &mut Project, id: PatternId) -> Result<&mut Pattern, EditError> {
    project
        .patterns
        .get_mut(&id)
        .ok_or(EditError::UnknownPattern(id.0))
}

/// Fails unless `step` is one of the pattern's steps (16 per bar).
fn check_step(pattern: &Pattern, step: u32) -> Result<(), EditError> {
    let per_bar = u64::from(time::BEATS_PER_BAR * time::STEPS_PER_BEAT);
    let steps = u64::from(pattern.length_bars) * per_bar;
    if u64::from(step) < steps {
        Ok(())
    } else {
        Err(EditError::StepOutsidePattern { step, steps })
    }
}

fn toggle_step(
    project: &mut Project,
    pattern_id: PatternId,
    channel: ChannelId,
    step: u32,
) -> Result<Applied, EditError> {
    if !project.channels.contains_key(&channel) {
        return Err(EditError::UnknownChannel(channel.0));
    }
    let tick = time::step_to_ticks(step, project.ppq);
    let pattern = pattern_mut(project, pattern_id)?;
    check_step(pattern, step)?;
    // Notes are sorted by (tick, channel, key), so this channel's notes on this tick are one
    // run, and a note added at the run's start keeps the order: the JSON stays canonical.
    let on_step = |note: &Note| (note.start, note.channel).cmp(&(tick, channel));
    let first = pattern.notes.partition_point(|note| on_step(note).is_lt());
    let end = first + pattern.notes[first..].partition_point(|note| on_step(note).is_eq());
    let inverse = if first < end {
        let removed = pattern.notes.drain(first..end).collect();
        Edit::ReplaceNotes {
            pattern: pattern_id,
            remove: Vec::new(),
            add: removed,
        }
    } else {
        let note = Note {
            channel,
            start: tick,
            length: Ticks(0),
            velocity: STEP_VELOCITY,
            key: DEFAULT_KEY,
        };
        pattern.notes.insert(first, note);
        Edit::ToggleStep {
            pattern: pattern_id,
            channel,
            step,
        }
    };
    Ok(Applied::pattern(inverse, pattern_id))
}

fn replace_notes(
    project: &mut Project,
    pattern_id: PatternId,
    remove: Vec<Note>,
    add: Vec<Note>,
) -> Result<Applied, EditError> {
    if remove.is_empty() && add.is_empty() {
        return Err(EditError::NothingToChange);
    }
    for note in &add {
        if !project.channels.contains_key(&note.channel) {
            return Err(EditError::UnknownChannel(note.channel.0));
        }
        // `contains` is false for NaN and infinity, which are not velocities.
        if note.start < Ticks(0)
            || note.length < Ticks(0)
            || note.key > MAX_KEY
            || !(0.0..=1.0).contains(&note.velocity)
        {
            return Err(EditError::InvalidNote);
        }
    }
    let pattern = pattern_mut(project, pattern_id)?;
    // Every check runs before the pattern changes, so a refused edit leaves it as it was.
    let mut doomed = BTreeSet::new();
    for note in &remove {
        let found = pattern
            .notes
            .binary_search_by_key(&note.order_key(), Note::order_key)
            .ok()
            .filter(|&index| pattern.notes[index] == *note);
        // A note listed twice is found twice at the same index, but exists only once.
        match found {
            Some(index) if doomed.insert(index) => {}
            _ => {
                return Err(EditError::NoteMissing {
                    tick: note.start.0,
                    channel: note.channel.0,
                    key: note.key,
                })
            }
        }
    }
    let mut notes: Vec<Note> = pattern
        .notes
        .iter()
        .enumerate()
        .filter(|(index, _)| !doomed.contains(index))
        .map(|(_, note)| note.clone())
        .chain(add.iter().cloned())
        .collect();
    notes.sort_by_key(Note::order_key);
    if let Some(pair) = notes
        .windows(2)
        .find(|pair| pair[0].order_key() == pair[1].order_key())
    {
        return Err(EditError::NoteExists {
            tick: pair[1].start.0,
            channel: pair[1].channel.0,
            key: pair[1].key,
        });
    }
    pattern.notes = notes;
    let inverse = Edit::ReplaceNotes {
        pattern: pattern_id,
        remove: add,
        add: remove,
    };
    Ok(Applied::pattern(inverse, pattern_id))
}

fn transaction(project: &mut Project, edits: Vec<Edit>) -> Result<Applied, EditError> {
    if edits.is_empty() {
        return Err(EditError::NothingToChange);
    }
    // Why restore a copy instead of applying the inverses of the edits that went through: the
    // copy restores the document even if an inverse were refused (possible for a project that
    // never passed `validate`), so a refused transaction can never leave half its work behind.
    let before = project.clone();
    let mut inverses = Vec::with_capacity(edits.len());
    let mut touched = Touched::default();
    for edit in edits {
        match apply(project, edit) {
            Ok(applied) => {
                inverses.push(applied.inverse);
                touched.merge(applied.touched);
            }
            Err(error) => {
                *project = before;
                return Err(error);
            }
        }
    }
    inverses.reverse();
    Ok(Applied {
        inverse: Edit::Transaction(inverses),
        touched,
    })
}

fn add_channel(
    project: &mut Project,
    id: ChannelId,
    channel: Channel,
) -> Result<Applied, EditError> {
    if id != project.next_channel_id || project.channels.contains_key(&id) {
        return Err(EditError::NotNextChannelId {
            got: id.0,
            expected: project.next_channel_id.0,
        });
    }
    let next = id.0.checked_add(1).ok_or(EditError::ChannelIdsExhausted)?;
    project.channels.insert(id, channel.as_added());
    project.next_channel_id = ChannelId(next);
    Ok(Applied::channel(Edit::RemoveChannel { id }, id))
}

fn set_channel_sample(
    project: &mut Project,
    id: ChannelId,
    path: PathBuf,
) -> Result<Applied, EditError> {
    let channel = channel_mut(project, id)?;
    if channel.kind != ChannelKind::Sampler {
        return Err(EditError::NotASampler(id.0));
    }
    if !channel.edits.is_empty() && channel.sample != path {
        // The edits go first, so the transaction's inverse gives the old file back before them.
        let edits = Edit::SetChannelEdits {
            channel: id,
            edits: SoundEdits::default(),
        };
        return transaction(project, vec![edits, Edit::SetChannelSample { id, path }]);
    }
    let old = mem::replace(&mut channel.sample, path);
    Ok(Applied::channel(
        Edit::SetChannelSample { id, path: old },
        id,
    ))
}

fn remove_channel(project: &mut Project, id: ChannelId) -> Result<Applied, EditError> {
    let channel = project
        .channels
        .get(&id)
        .ok_or(EditError::UnknownChannel(id.0))?;
    let newest = id.0.checked_add(1) == Some(project.next_channel_id.0);
    let as_added = *channel
        == match (&channel.processor, channel.kind, &channel.input) {
            (Some(processor), ..) => Channel::plugin(channel.name.clone(), processor.clone()),
            (None, ChannelKind::Instrument, _) => {
                Channel::instrument(channel.name.clone(), channel.instrument.clone())
            }
            (None, ChannelKind::Input, Some(input)) => {
                Channel::input(channel.name.clone(), input.clone())
            }
            (None, ..) => Channel::sampler(channel.name.clone(), channel.sample.clone()),
        }
        .as_added();
    let has_notes = project
        .patterns
        .values()
        .any(|pattern| pattern.notes.iter().any(|note| note.channel == id));
    let has_clips = project
        .playlist
        .clips
        .values()
        .any(|clip| clip.audio_channel() == Some(id));
    if !newest || !as_added || has_notes || has_clips {
        return Err(EditError::NotRemovable(id.0));
    }
    let Some(channel) = project.channels.remove(&id) else {
        return Err(EditError::UnknownChannel(id.0));
    };
    // Giving the ID back is safe: it was the newest, and nothing refers to it any more.
    project.next_channel_id = id;
    let inverse = match (channel.processor, channel.kind, channel.input) {
        (Some(processor), ..) => Edit::AddPluginChannel {
            id,
            name: channel.name,
            processor,
        },
        (None, ChannelKind::Instrument, _) => Edit::AddInstrumentChannel {
            id,
            name: channel.name,
            instrument: channel.instrument,
        },
        (None, ChannelKind::Input, Some(input)) => Edit::AddInputChannel {
            id,
            name: channel.name,
            input,
        },
        (None, ..) => Edit::AddSamplerChannel {
            id,
            name: channel.name,
            sample: channel.sample,
        },
    };
    Ok(Applied::channel(inverse, id))
}

fn set_processor_state(
    project: &mut Project,
    owner: ProcessorOwner,
    state: Option<String>,
) -> Result<Applied, EditError> {
    let reference = match owner {
        ProcessorOwner::Channel(id) => {
            let channel = channel_mut(project, id)?;
            if channel.kind != ChannelKind::Plugin {
                return Err(EditError::NotAPluginChannel(id.0));
            }
            channel
                .processor
                .as_mut()
                .ok_or(EditError::NotAPluginChannel(id.0))?
        }
        ProcessorOwner::Effect(id) => project
            .mixer
            .inserts
            .iter_mut()
            .flat_map(|insert| &mut insert.effects)
            .find(|effect| effect.id == id)
            .map(|effect| &mut effect.processor)
            .ok_or(EditError::UnknownEffect(id.0))?,
    };
    let old = mem::replace(&mut reference.state, state);
    let touched = Touched {
        processors: BTreeSet::from([owner]),
        ..Touched::default()
    };
    let inverse = Edit::SetProcessorState { owner, state: old };
    Ok(Applied { inverse, touched })
}

/// Linear undo and redo, each remembering at most [`HISTORY_LIMIT`] edits, and which state of
/// the document was last saved.
///
/// Every recorded edit gets a new [`StateId`], the id of the document's state after it. The
/// state now is the id of the newest edit on the undo stack, so undo and redo move between
/// states that had ids before, and a new edit makes a state that never existed. The document is
/// dirty when that id is not the one marked saved.
#[derive(Debug)]
pub struct History {
    /// Edits that undo, newest last, each with the state it undoes.
    undo: VecDeque<Entry>,
    /// Edits that redo, newest last, each with the state it redoes to.
    redo: VecDeque<Entry>,
    /// The state when the undo stack is empty: the start, until the limit forgets an edit; then
    /// the state after the newest forgotten one, which is what undoing everything leaves.
    floor: StateId,
    /// This history's own number, part of every id it gives.
    history: u64,
    /// The state number the next recorded edit gets.
    next: u64,
    /// The state on file, or `None` when no state the history can reach is on file.
    saved: Option<StateId>,
}

/// One state of a document as [`History`] tells them apart. Two equal ids are the same state
/// of the same document; ids are never reused, not even by the history of another document.
/// Why the history's number is in it: a window replaces one document with the next while
/// work started for the old one (an autosave) still reports back by id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateId {
    history: u64,
    state: u64,
}

/// The number the next history made takes.
static NEXT_HISTORY: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct Entry {
    state: StateId,
    edit: Edit,
    /// What the step is called, as the caller named it when it was recorded; it stays with the
    /// step on both stacks.
    name: Option<&'static str>,
}

impl Default for History {
    /// An empty history whose document is clean: as it was opened.
    fn default() -> Self {
        let history = NEXT_HISTORY.fetch_add(1, Ordering::Relaxed);
        let start = StateId { history, state: 0 };
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            floor: start,
            history,
            next: 1,
            saved: Some(start),
        }
    }
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// The document's state now.
    pub fn state(&self) -> StateId {
        self.undo.back().map_or(self.floor, |entry| entry.state)
    }

    /// Whether the document differs from its state on file.
    pub fn is_dirty(&self) -> bool {
        self.saved != Some(self.state())
    }

    /// The document as it is now is on file.
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.state());
    }

    /// No state the history can reach is on file, as for a recovered document:
    /// it stays dirty until it is saved.
    pub fn mark_unsaved(&mut self) {
        self.saved = None;
    }

    /// Applies `edit` and remembers its inverse as one undo step, without a name. Clears redo.
    pub fn apply(&mut self, project: &mut Project, edit: Edit) -> Result<Touched, EditError> {
        let Applied { inverse, touched } = apply(project, edit)?;
        self.record(inverse);
        Ok(touched)
    }

    /// As [`record`](Self::record), with the name the step is shown by (an Edit menu's "Undo
    /// <name>"). The engine never reads it.
    pub fn record_as(&mut self, inverse: Edit, name: &'static str) {
        self.push(inverse, Some(name));
    }

    /// The name of the step [`undo`](Self::undo) would take back, when it was recorded with one.
    pub fn undo_name(&self) -> Option<&'static str> {
        self.undo.back().and_then(|entry| entry.name)
    }

    /// The name of the step [`redo`](Self::redo) would do again, when it was recorded with one.
    pub fn redo_name(&self) -> Option<&'static str> {
        self.redo.back().and_then(|entry| entry.name)
    }

    /// Remembers a change that is already applied as one undo step; `inverse` is the edit that
    /// undoes it. Clears redo. A knob drag uses this: while it moves, each value is applied
    /// with [`apply`]; on release the inverse returned at the press (holding the value from
    /// before the drag) is recorded, so the whole drag is one undo step.
    pub fn record(&mut self, inverse: Edit) {
        self.push(inverse, None);
    }

    /// Remembers a change that is already applied as part of the newest undo step, so one undo
    /// takes back both: `inverse` first, then what the step undid before. The step keeps its
    /// name and is a new state of the document, as a step of its own would be (one saved
    /// between the two is dirty again). Clears redo. With no step to fold into, `inverse` is
    /// recorded as one, unnamed. Several files dropped from the desktop at once use this: they
    /// arrive one by one, and the drop is one step.
    pub fn fold_into_newest(&mut self, inverse: Edit) {
        let Some(newest) = self.undo.pop_back() else {
            self.record(inverse);
            return;
        };
        self.push(Edit::Transaction(vec![inverse, newest.edit]), newest.name);
    }

    fn push(&mut self, inverse: Edit, name: Option<&'static str>) {
        let state = StateId {
            history: self.history,
            state: self.next,
        };
        self.next += 1;
        if let Some(forgotten) = push_limited(
            &mut self.undo,
            Entry {
                state,
                edit: inverse,
                name,
            },
        ) {
            self.floor = forgotten.state;
        }
        self.redo.clear();
    }

    /// Undoes the newest edit. `Ok(None)` when there is nothing to undo.
    pub fn undo(&mut self, project: &mut Project) -> Result<Option<Touched>, EditError> {
        step(&mut self.undo, &mut self.redo, project)
    }

    /// Redoes the newest undone edit. `Ok(None)` when there is nothing to redo.
    pub fn redo(&mut self, project: &mut Project) -> Result<Option<Touched>, EditError> {
        step(&mut self.redo, &mut self.undo, project)
    }

    /// How many edits can be undone.
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// How many edits can be redone.
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }
}

/// Applies the newest edit of `from` and pushes its inverse onto `to`, with the same state id:
/// the entry names the state on the undo side of it. A refused edit stays on `from`, and the
/// project is unchanged. No limit applies here: [`History::record`] clears redo, so the two
/// stacks together never hold more than [`HISTORY_LIMIT`] entries.
fn step(
    from: &mut VecDeque<Entry>,
    to: &mut VecDeque<Entry>,
    project: &mut Project,
) -> Result<Option<Touched>, EditError> {
    let Some(Entry { state, edit, name }) = from.pop_back() else {
        return Ok(None);
    };
    match apply(project, edit.clone()) {
        Ok(Applied { inverse, touched }) => {
            to.push_back(Entry {
                state,
                edit: inverse,
                name,
            });
            Ok(Some(touched))
        }
        Err(error) => {
            from.push_back(Entry { state, edit, name });
            Err(error)
        }
    }
}

/// Pushes `entry`, and returns the oldest entry when the stack is then over the limit.
fn push_limited(stack: &mut VecDeque<Entry>, entry: Entry) -> Option<Entry> {
    stack.push_back(entry);
    if stack.len() > HISTORY_LIMIT {
        return stack.pop_front();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const F1: &str = include_str!("../../../tests/fixtures/f1_four_on_floor.json");
    const F3: &str = include_str!("../../../tests/fixtures/f3_velocity_ramp.json");

    fn load(text: &str) -> Project {
        Project::load_json(text).expect("fixture loads")
    }

    fn toggle(channel: u32, step: u32) -> Edit {
        Edit::ToggleStep {
            pattern: PatternId(1),
            channel: ChannelId(channel),
            step,
        }
    }

    fn add_snare(project: &mut Project) -> Applied {
        let edit = Edit::AddSamplerChannel {
            id: ChannelId(2),
            name: "snare".into(),
            sample: "impulse.wav".into(),
        };
        apply(project, edit).expect("channel 2 is next")
    }

    // An edit is its own inverse when it changed nothing, but a step switched on is too: the
    // same toggle takes it off again. F1's channel 1 is at volume 1.0 with steps 3 and 5 empty.
    #[test]
    fn only_an_edit_that_changed_nothing_counts_as_unchanged() {
        let mut project = load(F1);
        let mut unchanged = |edit: Edit| {
            let applied = apply(&mut project, edit.clone()).expect("applies");
            changed_nothing(&edit, &applied.inverse)
        };
        let volume = |value| Edit::SetChannelVolume {
            channel: ChannelId(1),
            value,
        };
        assert!(unchanged(volume(1.0)), "already 1.0");
        assert!(!unchanged(volume(0.5)));
        let canceling = Edit::Transaction(vec![volume(0.25), volume(0.5)]);
        assert!(unchanged(canceling));
        assert!(!unchanged(toggle(1, 3)), "switched on");
        assert!(!unchanged(Edit::Transaction(vec![toggle(1, 5)])));
    }

    #[test]
    fn toggle_step_twice_is_identity_on_an_empty_step() {
        for step in [3, 15] {
            let mut project = load(F1);
            let before = project.clone();
            apply(&mut project, toggle(1, step)).expect("first toggle");
            assert_ne!(project, before, "step {step}");
            apply(&mut project, toggle(1, step)).expect("second toggle");
            assert_eq!(project, before, "step {step}");
        }
    }

    // F1's own hits have velocity 1.0. Switching one off and on again lights the step as a
    // fresh step, at STEP_VELOCITY; only the inverse of the first toggle restores the 1.0.
    #[test]
    fn an_occupied_step_toggled_twice_comes_back_as_a_fresh_step() {
        for step in [0, 4] {
            let mut project = load(F1);
            let before = project.clone();
            let off = apply(&mut project, toggle(1, step)).expect("first toggle");
            assert_ne!(project, before, "step {step}");
            apply(&mut project, toggle(1, step)).expect("second toggle");
            let tick = time::step_to_ticks(step, project.ppq);
            let notes = &project.patterns[&PatternId(1)].notes;
            let lit = notes.iter().find(|note| note.start == tick).expect("lit");
            assert_eq!(lit.velocity, STEP_VELOCITY, "step {step}");
            apply(&mut project, toggle(1, step)).expect("third toggle");
            apply(&mut project, off.inverse).expect("inverse of the first");
            assert_eq!(project, before, "step {step}");
        }
    }

    #[test]
    fn a_channel_never_has_two_notes_on_one_tick_and_key() {
        let mut project = load(F1);
        let before = project.clone();
        let note = project.patterns[&PatternId(1)].notes[1].clone(); // tick 96, step 4
        let refused = apply(
            &mut project,
            Edit::ReplaceNotes {
                pattern: PatternId(1),
                remove: Vec::new(),
                add: vec![note],
            },
        );
        assert_eq!(
            refused,
            Err(EditError::NoteExists {
                tick: 96,
                channel: 1,
                key: DEFAULT_KEY
            })
        );
        assert_eq!(project, before);
        // Toggling an occupied step removes its note rather than adding a second one.
        apply(&mut project, toggle(1, 4)).expect("toggle");
        assert_eq!(project.patterns[&PatternId(1)].notes.len(), 3);
    }

    // F3's one hit has velocity 0.5; a fresh step would have 100/128.
    #[test]
    fn switching_off_a_note_with_its_own_velocity_is_undone_exactly() {
        let mut project = load(F3);
        let before = project.to_json().expect("serializes");
        let applied = apply(&mut project, toggle(1, 0)).expect("toggle off");
        assert!(project.patterns[&PatternId(1)].notes.is_empty());
        apply(&mut project, applied.inverse).expect("undo");
        assert_eq!(project.to_json().expect("serializes"), before);
    }

    #[test]
    fn a_tempo_outside_10_to_522_bpm_is_rejected_and_changes_nothing() {
        for mbpm in [0, 9_999, 522_001] {
            let mut project = load(F1);
            let before = project.clone();
            let result = apply(
                &mut project,
                Edit::SetTempo {
                    mbpm: MilliBpm(mbpm),
                },
            );
            assert_eq!(result, Err(EditError::TempoOutOfRange(mbpm)));
            assert_eq!(project, before);
        }
    }

    #[test]
    fn undoing_an_add_restores_next_channel_id() {
        let mut project = load(F1);
        let before = project.clone();
        let mut history = History::new();
        let add = Edit::AddSamplerChannel {
            id: ChannelId(2),
            name: "snare".into(),
            sample: "impulse.wav".into(),
        };
        history.apply(&mut project, add).expect("add");
        assert_eq!(project.next_channel_id, ChannelId(3));
        history
            .undo(&mut project)
            .expect("undo")
            .expect("an edit to undo");
        assert_eq!(project.next_channel_id, ChannelId(2));
        assert_eq!(project, before);
        history
            .redo(&mut project)
            .expect("redo")
            .expect("an edit to redo");
        assert_eq!(project.next_channel_id, ChannelId(3));
        assert_eq!(project.channels[&ChannelId(2)].name, "snare");
    }

    // By hand: a channel that plays notes is added at 9/16 (20·log10(0.5625) = −4.998 dB),
    // an input channel at 1; either is still removable as added, and once its
    // volume moves it is not.
    #[test]
    fn a_channel_that_plays_notes_is_added_at_minus_5_db() {
        let mut project = load(F1);
        add_snare(&mut project);
        assert_eq!(project.channels[&ChannelId(2)].volume, 0.5625);
        assert_eq!(crate::project::NEW_CHANNEL_VOLUME, 0.5625);
        let input = Edit::AddInputChannel {
            id: ChannelId(3),
            name: "Line".into(),
            input: crate::project::InputSource {
                device: String::new(),
                channels: crate::project::InputChannels::Stereo(0, 1),
            },
        };
        apply(&mut project, input).expect("adds");
        assert_eq!(project.channels[&ChannelId(3)].volume, 1.0);
        let remove = |id| Edit::RemoveChannel { id: ChannelId(id) };
        let applied = apply(&mut project, remove(3)).expect("removable as added");
        apply(&mut project, applied.inverse).expect("added back");
        apply(&mut project, remove(3)).expect("removed");
        apply(
            &mut project,
            Edit::SetChannelVolume {
                channel: ChannelId(2),
                value: 1.0,
            },
        )
        .expect("volume");
        assert_eq!(
            apply(&mut project, remove(2)),
            Err(EditError::NotRemovable(2))
        );
    }

    #[test]
    fn only_the_newest_unchanged_unused_channel_can_be_removed() {
        let mut project = load(F1);
        let remove = |id| Edit::RemoveChannel { id: ChannelId(id) };
        add_snare(&mut project);
        assert_eq!(
            apply(&mut project, remove(1)),
            Err(EditError::NotRemovable(1))
        );
        apply(
            &mut project,
            Edit::SetChannelMute {
                channel: ChannelId(2),
                value: true,
            },
        )
        .expect("mute");
        assert_eq!(
            apply(&mut project, remove(2)),
            Err(EditError::NotRemovable(2))
        );
        apply(
            &mut project,
            Edit::SetChannelMute {
                channel: ChannelId(2),
                value: false,
            },
        )
        .expect("unmute");
        apply(&mut project, toggle(2, 3)).expect("note on channel 2");
        assert_eq!(
            apply(&mut project, remove(2)),
            Err(EditError::NotRemovable(2))
        );
        apply(&mut project, toggle(2, 3)).expect("note off again");
        let before_remove = project.clone();
        let applied = apply(&mut project, remove(2)).expect("removable now");
        apply(&mut project, applied.inverse).expect("add back");
        assert_eq!(project, before_remove);
    }

    #[test]
    fn a_refused_edit_changes_nothing() {
        let refused = [
            (toggle(9, 0), EditError::UnknownChannel(9)),
            (
                Edit::ToggleStep {
                    pattern: PatternId(9),
                    channel: ChannelId(1),
                    step: 0,
                },
                EditError::UnknownPattern(9),
            ),
            (
                toggle(1, 16),
                EditError::StepOutsidePattern {
                    step: 16,
                    steps: 16,
                },
            ),
            (
                Edit::SetChannelVolume {
                    channel: ChannelId(1),
                    value: 4.5,
                },
                EditError::InvalidVolume(4.5),
            ),
            (
                Edit::SetChannelPan {
                    channel: ChannelId(1),
                    value: -1.5,
                },
                EditError::InvalidPan(-1.5),
            ),
            (
                Edit::SetPatternBars {
                    pattern: PatternId(1),
                    bars: 0,
                },
                EditError::NoBars,
            ),
            (
                Edit::AddSamplerChannel {
                    id: ChannelId(5),
                    name: "x".into(),
                    sample: "x.wav".into(),
                },
                EditError::NotNextChannelId {
                    got: 5,
                    expected: 2,
                },
            ),
            (
                Edit::ReplaceNotes {
                    pattern: PatternId(1),
                    remove: vec![Note {
                        channel: ChannelId(1),
                        start: Ticks(30),
                        length: Ticks(0),
                        velocity: 1.0,
                        key: DEFAULT_KEY,
                    }],
                    add: Vec::new(),
                },
                EditError::NoteMissing {
                    tick: 30,
                    channel: 1,
                    key: DEFAULT_KEY,
                },
            ),
            (
                Edit::ReplaceNotes {
                    pattern: PatternId(1),
                    remove: Vec::new(),
                    add: vec![Note {
                        channel: ChannelId(1),
                        start: Ticks(30),
                        length: Ticks(0),
                        velocity: 1.0,
                        key: MAX_KEY + 1,
                    }],
                },
                EditError::InvalidNote,
            ),
            (Edit::Transaction(Vec::new()), EditError::NothingToChange),
        ];
        for (edit, error) in refused {
            let mut project = load(F1);
            let before = project.clone();
            assert_eq!(apply(&mut project, edit.clone()), Err(error), "{edit:?}");
            assert_eq!(project, before, "{edit:?}");
        }
    }

    #[test]
    fn touched_names_only_what_changed() {
        let mut project = load(F1);
        let touched = |project: &mut Project, edit| apply(project, edit).expect("applies").touched;
        let pattern_1 = Touched {
            patterns: BTreeSet::from([PatternId(1)]),
            ..Touched::default()
        };
        let channel = |id| Touched {
            channels: BTreeSet::from([ChannelId(id)]),
            ..Touched::default()
        };
        assert_eq!(touched(&mut project, toggle(1, 2)), pattern_1);
        let bars = Edit::SetPatternBars {
            pattern: PatternId(1),
            bars: 2,
        };
        assert_eq!(touched(&mut project, bars), pattern_1);
        let volume = Edit::SetChannelVolume {
            channel: ChannelId(1),
            value: 0.5,
        };
        assert_eq!(touched(&mut project, volume), channel(1));
        let tempo = Edit::SetTempo {
            mbpm: MilliBpm(140_000),
        };
        let tempo_only = Touched {
            tempo: true,
            ..Touched::default()
        };
        assert_eq!(touched(&mut project, tempo), tempo_only);
        assert_eq!(add_snare(&mut project).touched, channel(2));
    }
}
