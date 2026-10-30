//! The project document: plain data keyed by stable IDs. It is read from JSON of any supported
//! `format_version` (older versions are migrated on load), checked by [`Project::validate`], and
//! written back as canonical JSON by [`Project::to_json`].

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::time::{self, MilliBpm, Ticks};

pub(crate) mod automation;
mod edits;
mod input;
mod links;
mod midi_out;
mod mixer;
mod sound;

pub use automation::{
    Automation, AutomationId, AutomationPoint, AutomationTarget, Automations, Curve, EXP_STEEPNESS,
    MAX_AUTOMATIONS, TENSION_RANGE, VALUE_RANGE,
};
pub use edits::{
    EditsProblem, EditsSource, SoundEdits, SoundOp, EDIT_GAIN_RANGE, MAX_SOUND_EDITS,
    NORMALIZE_RANGE,
};
pub use input::{InputChannels, InputSource};
pub use links::{Controller, MidiLinks, MAX_CONTROLLER, MIDI_CHANNELS};
pub use midi_out::{MidiOut, LATENCY_RANGE, MIDI_OUT_CHANNELS, PROGRAM_RANGE};
pub use mixer::{
    loop_text, Effect, EffectId, Insert, InsertId, Mixer, ProcessorRef, DEFAULT_INSERTS,
    EFFECT_SLOTS, FADER_RANGE, FORMER_NATIVE_ID_PREFIX, MAX_INSERTS, NATIVE_ID_PREFIX, SEND_RANGE,
    UNITY_FADER,
};
pub use sound::{
    Gate, Sound, ATTACK_RANGE, BEND_RANGE, CENT_RANGE, CUTOFF_RANGE, DEFAULT_BEND_RANGE,
    RELEASE_RANGE, RESONANCE_RANGE, SEMITONE_RANGE, START_RANGE, VELOCITY_RANGE,
};

/// Stable ID of a channel. New IDs come from [`Project::next_channel_id`] and are never reused,
/// so references to a channel survive edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChannelId(pub u32);

/// Stable ID of a pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PatternId(pub u32);

/// Stable ID of a playlist track. ID order is track order, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TrackId(pub u32);

/// Stable ID of a playlist clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClipId(pub u32);

/// The `format_version` this build writes for a project without instrument channels.
pub const FORMAT_VERSION: u32 = 2;

/// The `format_version` this build writes for a project with an instrument channel, which a
/// build that knows only [`FORMAT_VERSION`] then refuses by its version rather than by a field
/// it cannot read. A project without one keeps writing version 2, so its bytes do not change.
///
/// Why 4 and not 3: no file was ever written as version 3, and the loader's tests already use 3
/// as the example of a version from a newer build, which this one must refuse.
pub const INSTRUMENT_FORMAT_VERSION: u32 = 4;

/// The `format_version` this build writes for a project with a channel whose sound settings
/// are not the defaults ([`Sound`]); it reads as version 4 does, with the settings. A project
/// without such a channel keeps writing 2 or 4, so its bytes do not change.
pub const SOUND_FORMAT_VERSION: u32 = 5;

/// The `format_version` this build writes for a project with an input channel; it reads as
/// version 5 does, with the channel kind `input`. A project without one keeps writing 2, 4 or 5,
/// so its bytes do not change, and a build that knows only those refuses it by its version.
pub const INPUT_FORMAT_VERSION: u32 = 6;

/// The `format_version` this build writes for a project with an input channel that has a MIDI
/// out ([`MidiOut`]); it reads as version 6 does, with the channel's `midi_out`. A project
/// without one keeps writing 2, 4, 5 or 6, so its bytes do not change, and a build that knows
/// only those refuses it by its version rather than dropping the MIDI out on save.
pub const MIDI_OUT_FORMAT_VERSION: u32 = 7;

/// The `format_version` this build writes for a project with a sound that has edits
/// ([`SoundEdits`]); it reads as version 7 does, with the channels' `edits`. A project without
/// any keeps writing 2, 4, 5, 6 or 7, so its bytes do not change, and a build that knows only
/// those refuses it by its version. Why not read past, as `copied_from` is: an older build would
/// play the file unedited (a crop undone, a fade gone) and drop the edits when it saved. The
/// list's `source` (what its first op found of the sound) came within version 8: a build before
/// it reads past it, as it reads past every field beside the ops, since it plays nothing.
pub const EDITS_FORMAT_VERSION: u32 = 8;

/// The most tracks a playlist holds: the range of the project-file playlist item's track index.
pub const MAX_TRACKS: usize = 500;

/// Tracks the playlist of a new project has.
pub const DEFAULT_TRACKS: u32 = 8;

/// The most patterns a project gets by adding them. It counts patterns, not IDs: a deleted
/// pattern's ID is never given out again, so a new pattern's ID can be past 999.
pub const MAX_PATTERNS: usize = 999;

/// The longest song, in bars: no clip and no loop point reaches past its end.
pub const MAX_SONG_BARS: u32 = 100_000;

/// The tick where the longest song ends, [`MAX_SONG_BARS`] bars.
pub fn max_song_tick(ppq: u16) -> Ticks {
    time::bars_to_ticks(MAX_SONG_BARS, ppq)
}

/// The key a note plays when the file does not give one: middle C.
pub const DEFAULT_KEY: u8 = 60;

/// The highest key a note can play (keys follow the MIDI range 0..=127).
pub const MAX_KEY: u8 = 127;

/// Sample rates the engine accepts, in Hz.
pub const SAMPLE_RATE_RANGE: RangeInclusive<u32> = 8_000..=384_000;

/// Tempos the engine accepts, in milli-BPM (10 to 522 BPM).
pub const TEMPO_RANGE: RangeInclusive<u32> = 10_000..=522_000;

/// The loudest channel volume (linear gain 4.0, about +12 dB).
pub const MAX_VOLUME: f32 = 4.0;

/// The volume a channel that plays notes (a sampler, a plugin, an instrument set) is added at:
/// 9/16, or −5.0 dB. Why not full volume: every
/// shipped set is made so one full note peaks at −1 dBFS, so a channel at unity passed full scale
/// with two notes together, and the master's limiter then pulled every chord down. Audio and input
/// channels are added at 1: their files and inputs play at the level they were recorded at.
pub const NEW_CHANNEL_VOLUME: f32 = 0.5625;

/// Pan positions: -1.0 is hard left, 0.0 center, 1.0 hard right.
pub const PAN_RANGE: RangeInclusive<f32> = -1.0..=1.0;

/// An audio clip's gain, linear: 1 plays the file as it is, 4 is about +12 dB.
pub const CLIP_GAIN_RANGE: RangeInclusive<f32> = 0.0..=4.0;

/// The most audio clips a playlist holds: the player keeps a source slot for each.
pub const MAX_AUDIO_CLIPS: usize = 4096;

/// How far a stretch may lengthen or shorten its file: to at most 4 times its frames, and to at
/// least a quarter of them.
pub const MAX_STRETCH_RATIO: u64 = 4;

/// A stretch's pitch shift, in whole semitones.
pub const STRETCH_PITCH_RANGE: RangeInclusive<i8> = -24..=24;

/// The whole song document. The UI side owns it and changes it through
/// [`crate::edit::apply`]; playback reads a compiled `PlayPlan` instead.
///
/// Field order is JSON field order, and maps and notes are kept sorted, so equal projects
/// always serialize to the same bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// Output sample rate in Hz.
    pub sample_rate: u32,
    #[serde(rename = "tempo_mbpm")]
    pub tempo: MilliBpm,
    /// Ticks per beat.
    pub ppq: u16,
    /// The ID the next new channel gets: above every ID ever given out, so none is reused.
    pub next_channel_id: ChannelId,
    /// Channels by ID. ID order is rack order.
    #[serde(with = "id_list")]
    pub channels: BTreeMap<ChannelId, Channel>,
    /// The ID the next new pattern gets, written only when the patterns do not say it (one
    /// above the highest, or 1): after the newest pattern was deleted. Read it through
    /// [`Project::next_pattern_id`]; the pattern edits keep it. Why kept at all: anything that
    /// holds a pattern by its ID outside the document (the window's selection, a rename under
    /// way) must never come to name a different pattern than the one it was taken from. Why
    /// only then: every project whose newest pattern was never deleted keeps its bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) next_pattern_id: Option<PatternId>,
    /// Patterns by ID. The lowest ID is pattern 1.
    #[serde(with = "id_list")]
    pub patterns: BTreeMap<PatternId, Pattern>,
    /// The arrangement. A file without one loads [`Playlist::default`], and a playlist equal
    /// to it is not written, so projects made before playlists existed keep their bytes.
    #[serde(default, skip_serializing_if = "Playlist::is_default")]
    pub playlist: Playlist,
    /// The inserts, their sends and effects. Like the playlist: a file without one loads
    /// [`Mixer::default`], and a default mixer is not written.
    #[serde(default, skip_serializing_if = "Mixer::is_default")]
    pub mixer: Mixer,
    /// The automated parameters. Like the mixer: absent is the default (none), which is not
    /// written.
    #[serde(default, skip_serializing_if = "Automations::is_default")]
    pub automation: Automations,
    /// The controller links. Absent is none, which is not written.
    #[serde(default, skip_serializing_if = "MidiLinks::is_empty")]
    pub midi_links: MidiLinks,
}

/// A rack channel: a sampler, which plays one sound file from its start on every note at the
/// speed the note's key sets; an audio channel, whose file the playlist's audio clips play; a
/// plugin instrument, which is sent the notes; or an instrument set, whose zones the notes play.
///
/// The file names a sampler's or an audio channel's `sample`, a plugin channel's `processor`,
/// an instrument channel's `instrument` and an input channel's `input`, one of them: a sampler
/// is written `"kind":"sampler","sample":...` exactly as before other kinds existed, an audio
/// channel `"kind":"audio","sample":...`, a plugin channel `"kind":"plugin","processor":{...}`,
/// an instrument channel `"kind":"instrument","instrument":...` and an input channel
/// `"kind":"input","input":{...}`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(try_from = "ChannelFile<PathBuf, ProcessorRef>")]
pub struct Channel {
    pub name: String,
    pub kind: ChannelKind,
    /// A sampler's or an audio channel's sound file, relative to the directory of the project
    /// file (absolute paths are allowed); empty for a plugin channel.
    pub sample: PathBuf,
    /// A plugin channel's instrument; `None` for a sampler.
    pub processor: Option<ProcessorRef>,
    /// An instrument channel's set: its .sfz file, a path like `sample`'s; empty for every other
    /// kind.
    pub instrument: PathBuf,
    /// An input channel's audio input; `None` for every other kind.
    pub input: Option<InputSource>,
    /// Linear gain applied to every note on this channel, 0 to [`MAX_VOLUME`].
    pub volume: f32,
    /// Stereo position in [`PAN_RANGE`], by the law in [`crate::dsp::laws::channel_pan_gains`].
    pub pan: f32,
    /// A muted channel starts no notes and silences the ones already sounding.
    pub mute: bool,
    /// The mixer insert the channel plays into. Written only when it is not the master.
    pub insert: InsertId,
    /// The MIDI input port whose notes this channel plays whatever channel is selected; with
    /// none, the port's notes play the selected channel. At most one channel names a port, the
    /// name is never empty, and an audio channel has none (its clips play its file, not notes):
    /// reading a file leaves such links out ([`LoadNote`]). Written only when there is one.
    pub midi_input: Option<String>,
    /// An input channel's MIDI out: the port its notes are sent to; `None` for every other kind
    /// and by default. An input channel with one takes a MIDI input link too. Written only when
    /// there is one.
    pub midi_out: Option<MidiOut>,
    /// How its notes play its sound: tuning, envelope, velocity, filter, a sampler's direction
    /// and start. Written only when not the defaults.
    pub sound: Sound,
    /// A sampler's or an audio channel's sound file as it plays: the file through these edits,
    /// in order; every clip of the channel and every note on it hears the edited sound, and the
    /// file stays as it is. Written after the sound settings, only when it holds an op; any
    /// other kind holds none.
    pub edits: SoundEdits,
}

/// What makes a channel's sound, apart from its settings: the fields of [`Channel`] its kind
/// reads, as they are, so a channel given another source and then its own back is the channel
/// it was.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundSource {
    pub kind: ChannelKind,
    pub sample: PathBuf,
    pub processor: Option<ProcessorRef>,
    pub instrument: PathBuf,
    pub input: Option<InputSource>,
    /// The file's edits, which go with the sound it is: another file comes with none, and a
    /// channel given its own sound back gets its edits back.
    pub edits: SoundEdits,
}

impl SoundSource {
    /// A sampler playing `sample`, as [`Channel::sampler`] has it.
    pub fn sampler(sample: impl Into<PathBuf>) -> Self {
        Channel::sampler("", sample).source()
    }

    /// A plugin instrument, as [`Channel::plugin`] has it.
    pub fn plugin(processor: ProcessorRef) -> Self {
        Channel::plugin("", processor).source()
    }

    /// The instrument set whose .sfz file is `instrument`, as [`Channel::instrument`] has it.
    pub fn instrument(instrument: impl Into<PathBuf>) -> Self {
        Channel::instrument("", instrument).source()
    }

    /// The audio input `input`, as [`Channel::input`] has it.
    pub fn input(input: InputSource) -> Self {
        Channel::input("", input).source()
    }

    /// The plugin that makes this sound, by its registry key; none for a sampler or a set.
    pub fn plugin_id(&self) -> Option<&str> {
        self.processor
            .as_ref()
            .map(|processor| processor.id.as_str())
    }

    /// Whether `other` is this sound: the same kind, file, set, plugin and input. A plugin is
    /// compared by its key alone, since its state and parameter values are the instance's
    /// settings, not which sound it is; a file's edits are how it plays, not which file it is.
    pub fn same_sound(&self, other: &SoundSource) -> bool {
        self.kind == other.kind
            && self.sample == other.sample
            && self.instrument == other.instrument
            && self.input == other.input
            && self.plugin_id() == other.plugin_id()
    }
}

/// What makes a channel's sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelKind {
    /// Plays a sound file on every note.
    Sampler,
    /// Plays a plugin instrument.
    Plugin,
    /// The channel of audio clips: they play its sound file on the playlist. Notes on it play
    /// the file as a sampler's would.
    Audio,
    /// Plays an instrument set: each note sounds the zones that hold its key and velocity.
    Instrument,
    /// Plays an audio input (a device's channels) as its sound; notes on it play nothing.
    Input,
}

impl ChannelKind {
    /// Whether a channel of this kind plays notes (from a pattern or a MIDI device): not an
    /// audio channel, whose sound is its clips, nor an input channel, whose sound is its input.
    pub fn plays_notes(self) -> bool {
        !matches!(self, ChannelKind::Audio | ChannelKind::Input)
    }
}

impl Channel {
    /// A sampler channel at full volume, centered, not muted, playing into the master; added to
    /// a project, it is [`as_added`](Self::as_added).
    pub fn sampler(name: impl Into<String>, sample: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            kind: ChannelKind::Sampler,
            sample: sample.into(),
            processor: None,
            instrument: PathBuf::new(),
            input: None,
            volume: 1.0,
            pan: 0.0,
            mute: false,
            insert: InsertId::MASTER,
            midi_input: None,
            midi_out: None,
            sound: Sound::default(),
            edits: SoundEdits::default(),
        }
    }

    /// An audio channel playing `sample`, as an audio clip drop adds it.
    pub fn audio(name: impl Into<String>, sample: impl Into<PathBuf>) -> Self {
        Self {
            kind: ChannelKind::Audio,
            ..Self::sampler(name, sample)
        }
    }

    /// A plugin channel playing `processor`, as [`sampler`](Self::sampler) makes one.
    pub fn plugin(name: impl Into<String>, processor: ProcessorRef) -> Self {
        Self {
            kind: ChannelKind::Plugin,
            sample: PathBuf::new(),
            processor: Some(processor),
            ..Self::sampler(name, "")
        }
    }

    /// An instrument channel playing the set whose .sfz file is `instrument`, as
    /// [`sampler`](Self::sampler) makes one.
    pub fn instrument(name: impl Into<String>, instrument: impl Into<PathBuf>) -> Self {
        Self {
            kind: ChannelKind::Instrument,
            instrument: instrument.into(),
            ..Self::sampler(name, "")
        }
    }

    /// An input channel playing the audio input `input`, as [`sampler`](Self::sampler) makes
    /// one.
    pub fn input(name: impl Into<String>, input: InputSource) -> Self {
        Self {
            kind: ChannelKind::Input,
            input: Some(input),
            ..Self::sampler(name, "")
        }
    }

    /// The channel as an add puts it in the rack: one that plays notes at
    /// [`NEW_CHANNEL_VOLUME`], an audio or an input channel as it is.
    pub fn as_added(self) -> Self {
        if self.kind.plays_notes() {
            Self {
                volume: NEW_CHANNEL_VOLUME,
                ..self
            }
        } else {
            self
        }
    }

    /// What makes the channel's sound, apart from its settings: its kind, file, plugin, set and
    /// input.
    pub fn source(&self) -> SoundSource {
        SoundSource {
            kind: self.kind,
            sample: self.sample.clone(),
            processor: self.processor.clone(),
            instrument: self.instrument.clone(),
            input: self.input.clone(),
            edits: self.edits.clone(),
        }
    }

    /// Whether the channel names what its kind needs and nothing else: a sampler or an audio
    /// channel no plugin, a plugin channel a plugin and no sample file, an instrument channel
    /// neither, an input channel its input and none of them; only an instrument channel names a
    /// set and only an input channel an input.
    pub(crate) fn sound_is_consistent(&self) -> bool {
        let no_set = self.instrument.as_os_str().is_empty();
        let no_sample = self.sample.as_os_str().is_empty();
        let input = self.input.is_some();
        match self.kind {
            ChannelKind::Sampler | ChannelKind::Audio => {
                self.processor.is_none() && no_set && !input
            }
            ChannelKind::Plugin => self.processor.is_some() && no_sample && no_set && !input,
            ChannelKind::Instrument => self.processor.is_none() && no_sample && !input,
            ChannelKind::Input => self.processor.is_none() && no_sample && no_set && input,
        }
    }

    /// What makes the channel's edits ones it cannot hold: any on a channel that plays no file of
    /// its own, too many, an op over no frames or a gain out of its range.
    pub(crate) fn edits_problem(&self) -> Option<ValidationError> {
        let channel = || self.name.clone();
        if !self.edits.is_empty() && !self.plays_a_file() {
            return Some(ValidationError::EditsOnOtherKind { channel: channel() });
        }
        Some(match self.edits.problem()? {
            EditsProblem::TooMany(count) => ValidationError::TooManySoundEdits {
                channel: channel(),
                count,
            },
            EditsProblem::EmptyRange { index, from, to } => ValidationError::EmptyEditRange {
                channel: channel(),
                index,
                from,
                to,
            },
            EditsProblem::Gain { index, centi_db } => ValidationError::EditGainOutOfRange {
                channel: channel(),
                index,
                centi_db,
            },
        })
    }

    /// Whether the channel plays a sound file (a sampler or an audio channel).
    pub fn plays_a_file(&self) -> bool {
        matches!(self.kind, ChannelKind::Sampler | ChannelKind::Audio)
    }

    /// Whether the channel plays notes from a MIDI device: a sampler, a plugin or an instrument
    /// channel. An audio channel's sound is its clips and an input channel's its input.
    pub fn plays_notes(&self) -> bool {
        self.kind.plays_notes()
    }

    /// Whether the channel takes notes, from a MIDI input link, a take or the roll: one that
    /// plays them, or an input channel that sends them to its MIDI out. The one rule every link
    /// and capture asks.
    pub fn takes_midi_input(&self) -> bool {
        self.plays_notes() || self.midi_out.is_some()
    }
}

/// A channel as the file writes it: `sample` for a sampler or an audio channel, `processor` for a
/// plugin channel, `instrument` for an instrument channel.
#[derive(Serialize, Deserialize)]
struct ChannelFile<Sample, Processor> {
    name: String,
    kind: ChannelKind,
    // A missing optional field reads as `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    sample: Option<Sample>,
    #[serde(skip_serializing_if = "Option::is_none")]
    processor: Option<Processor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    instrument: Option<Sample>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input: Option<InputSource>,
    volume: f32,
    pan: f32,
    mute: bool,
    #[serde(default, skip_serializing_if = "InsertId::is_master")]
    insert: InsertId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    midi_input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    midi_out: Option<MidiOut>,
    #[serde(default, skip_serializing_if = "Sound::is_default")]
    sound: Sound,
    #[serde(default, skip_serializing_if = "SoundEdits::is_empty")]
    edits: SoundEdits,
}

impl Serialize for Channel {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ChannelFile {
            name: self.name.clone(),
            kind: self.kind,
            sample: self.plays_a_file().then(|| written_sample(&self.sample)),
            processor: self.processor.as_ref(),
            instrument: (self.kind == ChannelKind::Instrument)
                .then(|| written_sample(&self.instrument)),
            input: self.input.clone(),
            volume: self.volume,
            pan: self.pan,
            mute: self.mute,
            insert: self.insert,
            midi_input: self.midi_input.clone(),
            midi_out: self.midi_out.clone(),
            sound: self.sound,
            edits: self.edits.clone(),
        }
        .serialize(serializer)
    }
}

/// A sound file's path as the project file writes it. On Windows a relative path's separators
/// are written `/`, so a project saved there opens on Linux, where `\` is part of a file name
/// and `/` is the only separator; Windows reads either. Absolute paths name one machine's files
/// and are written as they are.
#[cfg(windows)]
fn written_sample(sample: &Path) -> Cow<'_, Path> {
    use std::path::Component;
    let parts: Option<Vec<&str>> = sample
        .components()
        .map(|part| match part {
            Component::Normal(_) | Component::CurDir | Component::ParentDir => {
                part.as_os_str().to_str()
            }
            Component::Prefix(_) | Component::RootDir => None,
        })
        .collect();
    match parts {
        Some(parts) => Cow::Owned(PathBuf::from(parts.join("/"))),
        None => Cow::Borrowed(sample),
    }
}

/// A sound file's path as the project file writes it: as it is, `/` being the separator here.
#[cfg(not(windows))]
fn written_sample(sample: &Path) -> Cow<'_, Path> {
    Cow::Borrowed(sample)
}

impl TryFrom<ChannelFile<PathBuf, ProcessorRef>> for Channel {
    type Error = String;

    fn try_from(file: ChannelFile<PathBuf, ProcessorRef>) -> Result<Self, String> {
        let sound = (
            file.kind,
            file.sample,
            file.processor,
            file.instrument,
            file.input,
        );
        let (sample, processor, instrument, input) = match sound {
            (ChannelKind::Sampler | ChannelKind::Audio, Some(sample), None, None, None) => {
                (sample, None, PathBuf::new(), None)
            }
            (ChannelKind::Plugin, None, Some(processor), None, None) => {
                (PathBuf::new(), Some(processor), PathBuf::new(), None)
            }
            (ChannelKind::Instrument, None, None, Some(instrument), None) => {
                (PathBuf::new(), None, instrument, None)
            }
            (ChannelKind::Input, None, None, None, Some(input)) => {
                (PathBuf::new(), None, PathBuf::new(), Some(input))
            }
            (ChannelKind::Sampler | ChannelKind::Audio, ..) => {
                return Err(format!(
                    "channel '{}': a sampler or an audio channel names a sample and no processor",
                    file.name
                ))
            }
            (ChannelKind::Plugin, ..) => {
                return Err(format!(
                    "channel '{}': a plugin channel names a processor and no sample",
                    file.name
                ))
            }
            (ChannelKind::Instrument, ..) => {
                return Err(format!(
                    "channel '{}': an instrument channel names an instrument and no sample or processor",
                    file.name
                ))
            }
            (ChannelKind::Input, ..) => {
                return Err(format!(
                    "channel '{}': an input channel names an input and no sample, processor or instrument",
                    file.name
                ))
            }
        };
        Ok(Self {
            name: file.name,
            kind: file.kind,
            sample,
            processor,
            instrument,
            input,
            volume: file.volume,
            pan: file.pan,
            mute: file.mute,
            insert: file.insert,
            midi_input: file.midi_input,
            midi_out: file.midi_out,
            sound: file.sound,
            edits: file.edits,
        })
    }
}

/// Who a running processor plays for: a plugin channel's instrument, or the effect in a mixer
/// slot. Both IDs are stable, so a `ProcessorOwner` names the same processor across edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProcessorOwner {
    Channel(ChannelId),
    Effect(EffectId),
}

/// A pattern: notes for any number of channels, looping every `length_bars` bars.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pattern {
    pub name: String,
    pub length_bars: u32,
    /// The pattern this one was copied from when a clip was given its own copy (unlinked), so
    /// the clip can be linked back to it; it may since have been deleted. Below the pattern's
    /// own ID, since a copy is made after its original, and pattern IDs are never given out
    /// twice, so it never comes to name another pattern. Written only when set: a build that
    /// knows nothing of copies reads past it (and drops it when it saves), so the format
    /// version stays.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copied_from: Option<PatternId>,
    /// Sorted by (tick, channel, key), at most one note per channel, tick and key, so a channel
    /// can play a chord. Notes at or past the pattern's end are kept, so shortening a pattern
    /// and lengthening it again loses nothing, but they are silent.
    pub notes: Vec<Note>,
}

/// One note in a pattern. A step hit is a note with `length` 0: steps and notes are one
/// representation, and a step grid is only a way of looking at notes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub channel: ChannelId,
    /// Ticks from the start of the pattern.
    #[serde(rename = "tick")]
    pub start: Ticks,
    /// 0 for a step hit.
    pub length: Ticks,
    /// 0.0..=1.0, used as a linear gain.
    pub velocity: f32,
    /// Sets the playback speed: key 60 plays the sample as recorded, each key above or below
    /// one semitone higher or lower. Written only when it is not [`DEFAULT_KEY`], so step-only
    /// patterns stay free of it.
    #[serde(default = "default_key", skip_serializing_if = "is_default_key")]
    pub key: u8,
}

impl Note {
    /// The order notes are kept in: by tick, then channel, then key.
    pub(crate) fn order_key(&self) -> (Ticks, ChannelId, u8) {
        (self.start, self.channel, self.key)
    }
}

fn default_key() -> u8 {
    DEFAULT_KEY
}

fn is_default_key(key: &u8) -> bool {
    *key == DEFAULT_KEY
}

/// The song: tracks holding clips (each a window onto a pattern, an audio file or an
/// automation), and the loop range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Playlist {
    /// The ID the next new track gets: above every ID ever given out, so none is reused.
    pub next_track_id: TrackId,
    /// The ID the next new clip gets.
    pub next_clip_id: ClipId,
    /// Tracks by ID; ID order is track order, top to bottom.
    #[serde(with = "id_list")]
    pub tracks: BTreeMap<TrackId, Track>,
    #[serde(with = "id_list")]
    pub clips: BTreeMap<ClipId, Clip>,
    /// Written only when there is one.
    #[serde(rename = "loop", default, skip_serializing_if = "Option::is_none")]
    pub loop_range: Option<LoopRange>,
}

impl Default for Playlist {
    /// [`DEFAULT_TRACKS`] unmuted tracks named "Track 1" onwards, no clips and no loop.
    fn default() -> Self {
        let tracks = (1..=DEFAULT_TRACKS)
            .map(|id| (TrackId(id), Track::named(format!("Track {id}"))))
            .collect();
        Self {
            next_track_id: TrackId(DEFAULT_TRACKS + 1),
            next_clip_id: ClipId(1),
            tracks,
            clips: BTreeMap::new(),
            loop_range: None,
        }
    }
}

impl Playlist {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Where the song ends: the latest clip end, muted clips included; tick 0 without clips.
    pub fn end(&self) -> Ticks {
        self.clips.values().map(Clip::end).max().unwrap_or(Ticks(0))
    }
}

/// A playlist row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub name: String,
    /// A muted track plays none of its clips.
    pub mute: bool,
}

impl Track {
    /// A track as it is when first added: unmuted.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            mute: false,
        }
    }
}

/// A clip: the part of its item that plays from song tick `start` for `length` ticks. A clip
/// longer than its pattern repeats the pattern; a shorter one plays the pattern's first part.
/// An audio clip plays its channel's file from its `frame_offset`; an automation clip shows its
/// automation's points from tick `offset`, holding the last value past them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub track: TrackId,
    pub start: Ticks,
    /// At least 1.
    pub length: Ticks,
    #[serde(flatten)]
    pub item: ClipItem,
    /// Where in the item's data the clip's window starts: song tick `start` plays data tick
    /// `offset`. Written only when it is not 0. Always 0 on an audio clip, whose window starts
    /// at its `frame_offset` instead.
    #[serde(default, skip_serializing_if = "is_zero_ticks")]
    pub offset: Ticks,
    /// A muted clip plays nothing. Written only when set.
    #[serde(default, skip_serializing_if = "is_false")]
    pub muted: bool,
}

impl Clip {
    /// A pattern clip as placed with the mouse: no offset, not muted.
    pub fn of_pattern(pattern: PatternId, track: TrackId, start: Ticks, length: Ticks) -> Self {
        Self {
            track,
            start,
            length,
            item: ClipItem::Pattern { pattern },
            offset: Ticks(0),
            muted: false,
        }
    }

    /// The song tick just after the clip.
    pub fn end(&self) -> Ticks {
        Ticks(self.start.0.saturating_add(self.length.0))
    }

    /// An audio clip of `channel`'s file, from its first frame at unity gain, as a drop places
    /// it.
    pub fn of_audio(channel: ChannelId, track: TrackId, start: Ticks, length: Ticks) -> Self {
        Self {
            track,
            start,
            length,
            item: ClipItem::Audio {
                channel,
                frame_offset: 0,
                gain: 1.0,
                stretch: None,
            },
            offset: Ticks(0),
            muted: false,
        }
    }

    /// An automation clip of `automation`, from its first tick.
    pub fn of_automation(
        automation: AutomationId,
        track: TrackId,
        start: Ticks,
        length: Ticks,
    ) -> Self {
        Self {
            track,
            start,
            length,
            item: ClipItem::Automation { automation },
            offset: Ticks(0),
            muted: false,
        }
    }

    /// The pattern a pattern clip plays.
    pub fn pattern(&self) -> Option<PatternId> {
        match self.item {
            ClipItem::Pattern { pattern } => Some(pattern),
            ClipItem::Audio { .. } | ClipItem::Automation { .. } => None,
        }
    }

    /// The channel an audio clip plays.
    pub fn audio_channel(&self) -> Option<ChannelId> {
        match self.item {
            ClipItem::Audio { channel, .. } => Some(channel),
            ClipItem::Pattern { .. } | ClipItem::Automation { .. } => None,
        }
    }

    /// The automation an automation clip shows.
    pub fn automation(&self) -> Option<AutomationId> {
        match self.item {
            ClipItem::Automation { automation } => Some(automation),
            ClipItem::Pattern { .. } | ClipItem::Audio { .. } => None,
        }
    }
}

/// What a clip plays, tagged by `kind` in the file.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ClipItem {
    Pattern {
        pattern: PatternId,
    },
    /// An audio channel's file: song sample n of the clip plays file frame
    /// `frame_offset + n − s(start)`, scaled by `gain`. A stretched clip plays the stretched
    /// file instead, and its frames count the stretched file's.
    Audio {
        channel: ChannelId,
        /// Where the clip's window starts in the file, in frames at the project's rate.
        ///
        /// Why frames and not ticks: audio lives in seconds, so a tempo change must not change
        /// which part of the file plays. Written only when it is not 0.
        #[serde(default, skip_serializing_if = "is_zero_frames")]
        frame_offset: u64,
        /// Linear, in [`CLIP_GAIN_RANGE`]. Written only when it is not 1.
        #[serde(default = "unity", skip_serializing_if = "is_unity")]
        gain: f32,
        /// How the file is made longer or shorter before it plays. Written only when set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stretch: Option<ClipStretch>,
    },
    /// An automation's points, from tick `offset` of the clip.
    Automation {
        automation: AutomationId,
    },
}

/// How an audio clip's file is fitted to a new length before it plays: the whole file becomes
/// exactly `frames` frames at the project's rate, so the ratio is `frames` over the file's own
/// frames. The stretched file is rendered ahead of playback, never while it plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClipStretch {
    pub mode: StretchMode,
    /// The whole stretched file's length, at least 1.
    pub frames: u64,
    /// Semitones in [`STRETCH_PITCH_RANGE`], for [`StretchMode::Stretch`] only (resampling
    /// shifts the pitch by the ratio itself). Written only when it is not 0.
    #[serde(default, skip_serializing_if = "is_zero_pitch")]
    pub pitch: i8,
}

/// How a stretch changes the length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StretchMode {
    /// Read faster or slower, like a tape: longer is lower.
    Resample,
    /// Change the length and keep the pitch (or shift it by `pitch` alone).
    Stretch,
}

impl ClipStretch {
    /// Whether stretching a file of `source_frames` frames to `frames` stays within
    /// [`MAX_STRETCH_RATIO`] either way. Why not a rule of `validate`: the file's own length is
    /// read from the file, which the document does not hold.
    pub fn fits(&self, source_frames: u64) -> bool {
        let (target, source) = (u128::from(self.frames), u128::from(source_frames));
        let ratio = u128::from(MAX_STRETCH_RATIO);
        source > 0 && target * ratio >= source && target <= source * ratio
    }

    /// Whether the stretch follows the rules a document can be checked for alone: a length of
    /// at least a frame, a pitch in range, and no pitch on a resample.
    fn is_valid(&self) -> bool {
        self.frames >= 1
            && STRETCH_PITCH_RANGE.contains(&self.pitch)
            && (self.mode == StretchMode::Stretch || self.pitch == 0)
    }
}

fn is_zero_pitch(pitch: &i8) -> bool {
    *pitch == 0
}

/// The range song playback loops over: from `start` up to, not including, `end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopRange {
    pub start: Ticks,
    pub end: Ticks,
    /// Off keeps the range but plays through it.
    pub on: bool,
}

fn is_zero_ticks(ticks: &Ticks) -> bool {
    ticks.0 == 0
}

fn is_zero_frames(frames: &u64) -> bool {
    *frames == 0
}

fn unity() -> f32 {
    1.0
}

fn is_unity(gain: &f32) -> bool {
    *gain == 1.0
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Why a project file could not be loaded.
#[derive(Debug, Error)]
pub enum LoadError {
    #[error("not a valid project file: {0}")]
    Json(#[from] serde_json::Error),
    #[error(
        "format_version {0} is not supported (this build reads versions 1, 2, 4, 5, 6, 7 and 8)"
    )]
    UnsupportedVersion(u32),
    #[error("tempo_bpm {0} is not a positive tempo")]
    InvalidTempo(f64),
    #[error(transparent)]
    Invalid(#[from] ValidationError),
}

/// The part of MIDI input port name `name` that names its device from one session to the next:
/// all of it but a trailing ` <client>:<port>` pair of numbers. On Linux the system names a port
/// `<client name>:<port name> <client number>:<port number>` (the ALSA sequencer's numbers, as
/// midir writes them), and hands client numbers out in the order it finds devices, so they
/// change when another device comes first. Windows names (a device's product name) end in no
/// such pair and are their own. Links and the app's choice of ports store this part and compare
/// by it, so a name stored whole before still matches its device. Two devices whose names
/// differ only in those numbers are one port to links and settings.
pub fn stable_port_name(name: &str) -> &str {
    let numbers = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    match name.rsplit_once(' ') {
        Some((device, address))
            if !device.is_empty()
                && address
                    .split_once(':')
                    .is_some_and(|(client, port)| numbers(client) && numbers(port)) =>
        {
            device
        }
        _ => name,
    }
}

/// Whether MIDI input port names `a` and `b` name one port: their [`stable_port_name`]s match.
pub fn same_port(a: &str, b: &str) -> bool {
    stable_port_name(a) == stable_port_name(b)
}

/// What reading a project file left out of the document: a link this build never writes and no
/// edit could put back, so that every edit's inverse applies to every document read. Each is a
/// line for the user.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LoadNote {
    /// An audio channel plays its clips' file and an input channel its input, never a device's
    /// notes, unless the input channel sends them to its MIDI out.
    #[error("the link of channel '{channel}' to MIDI input '{port}' was left out: an audio channel, or an input channel without a MIDI out, takes no device's notes")]
    PortOnAudioChannel { channel: String, port: String },
    #[error("the link of channel '{channel}' to a MIDI input with no name was left out")]
    NamelessPort { channel: String },
    /// Two channels linked to one device under names that differ only in the system's numbers
    /// (an earlier build compared names whole); the first channel keeps it.
    #[error("the link of channel '{channel}' to MIDI input '{port}' was left out: channel '{kept}' plays that device, whose name differs only in the system's numbers")]
    SamePortTwice {
        channel: String,
        port: String,
        kept: String,
    },
}

/// A rule of the project model that a project breaks.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ValidationError {
    #[error("sample_rate {0} Hz is outside 8000..=384000")]
    SampleRateOutOfRange(u32),
    #[error("tempo {0} milli-BPM is outside 10000..=522000 (10 to 522 BPM)")]
    TempoOutOfRange(u32),
    #[error("ppq {0} must be a positive multiple of 4, so that a step (ppq/4 ticks) is a whole number of ticks")]
    InvalidPpq(u16),
    #[error(
        "channel id {id} is not below next_channel_id {next}, so a new channel could reuse it"
    )]
    UnallocatedChannelId { id: u32, next: u32 },
    #[error(
        "pattern id {id} is not below next_pattern_id {next}, so a new pattern could reuse it"
    )]
    UnallocatedPatternId { id: u32, next: u32 },
    #[error("channel '{channel}': volume {volume} must be a finite number from 0 to 4")]
    InvalidVolume { channel: String, volume: f32 },
    #[error("channel '{channel}': pan {pan} must be a number from -1 to 1")]
    InvalidPan { channel: String, pan: f32 },
    #[error("channel '{channel}': a sampler names a sample and no processor, a plugin channel a processor and no sample, an instrument channel an instrument alone")]
    InconsistentChannel { channel: String },
    #[error("channel '{channel}': its sound setting {setting} is out of range or not one its kind takes")]
    SoundSetting {
        channel: String,
        setting: &'static str,
    },
    #[error("channel '{channel}' has sound edits; only a sampler's or an audio channel's sound file is edited")]
    EditsOnOtherKind { channel: String },
    #[error("channel '{channel}' has {count} sound edits; a sound holds at most 1000")]
    TooManySoundEdits { channel: String, count: usize },
    #[error(
        "channel '{channel}': sound edit {index} is over frames {from}..{to}, which hold none"
    )]
    EmptyEditRange {
        channel: String,
        index: usize,
        from: u64,
        to: u64,
    },
    #[error("channel '{channel}': sound edit {index} sets {centi_db} hundredths of a dB; a gain takes -4800 to 2400, a normalize -4800 to 0")]
    EditGainOutOfRange {
        channel: String,
        index: usize,
        centi_db: i32,
    },
    #[error("pattern '{pattern}': length_bars must be at least 1")]
    EmptyPattern { pattern: String },
    #[error("pattern '{pattern}' is a copy of pattern {from}, which is not one made before it")]
    CopyOfLaterPattern { pattern: String, from: u32 },
    #[error("pattern '{pattern}': a note uses channel {channel}, which does not exist")]
    UnknownChannel { pattern: String, channel: u32 },
    #[error("pattern '{pattern}': a note starts at tick {start}, outside the pattern (ticks 0..{length})")]
    NoteOutsidePattern {
        pattern: String,
        start: i64,
        length: i64,
    },
    #[error("pattern '{pattern}': a note has negative length {length}")]
    NegativeLength { pattern: String, length: i64 },
    #[error("pattern '{pattern}': key {key} is outside 0..=127")]
    InvalidKey { pattern: String, key: u8 },
    #[error("pattern '{pattern}': velocity {velocity} must be between 0 and 1")]
    InvalidVelocity { pattern: String, velocity: f32 },
    #[error("pattern '{pattern}': notes must be sorted by tick, then channel, then key, with one note per channel, tick and key; the note at tick {tick} on channel {channel}, key {key} is out of place")]
    NotesOutOfOrder {
        pattern: String,
        tick: i64,
        channel: u32,
        key: u8,
    },
    #[error("the playlist has {0} tracks; it needs 1 to 500")]
    TrackCount(usize),
    #[error("track id {id} is not below next_track_id {next}, so a new track could reuse it")]
    UnallocatedTrackId { id: u32, next: u32 },
    #[error("clip id {id} is not below next_clip_id {next}, so a new clip could reuse it")]
    UnallocatedClipId { id: u32, next: u32 },
    #[error("clip {clip} is on track {track}, which does not exist")]
    UnknownTrack { clip: u32, track: u32 },
    #[error("clip {clip} plays pattern {pattern}, which does not exist")]
    UnknownClipPattern { clip: u32, pattern: u32 },
    #[error("clip {clip} plays channel {channel}, which does not exist")]
    UnknownClipChannel { clip: u32, channel: u32 },
    #[error("clip {clip} plays channel {channel}, which is not an audio channel")]
    NotAnAudioChannel { clip: u32, channel: u32 },
    #[error("clip {clip}: an audio clip starts its window by frame_offset, so its tick offset must be 0, not {offset}")]
    AudioClipTickOffset { clip: u32, offset: i64 },
    #[error("clip {clip}: gain {gain} must be a finite number from 0 to 4")]
    InvalidClipGain { clip: u32, gain: f32 },
    #[error("clip {clip} shows automation {automation}, which does not exist")]
    UnknownClipAutomation { clip: u32, automation: u32 },
    #[error("the playlist holds {0} audio clips; it holds at most 4096")]
    TooManyAudioClips(usize),
    #[error("the project has {0} automations; it holds at most 1024")]
    TooManyAutomations(usize),
    #[error("automation id {id} is not below next_id {next}, so a new automation could reuse it")]
    UnallocatedAutomationId { id: u32, next: u32 },
    #[error("automation {automation} has no points; it needs at least one")]
    NoAutomationPoints { automation: u32 },
    #[error("automation {automation}: the point at tick {tick} is out of place; ticks start at 0 or later and rise from point to point")]
    AutomationPointOutOfOrder { automation: u32, tick: i64 },
    #[error("automation {automation}: the point at tick {tick} has value {value}; it must be a number from 0 to 1")]
    InvalidAutomationValue {
        automation: u32,
        tick: i64,
        value: f32,
    },
    #[error("automation {automation}: the point at tick {tick} has tension {tension}; it must be a number from -1 to 1")]
    InvalidTension {
        automation: u32,
        tick: i64,
        tension: f32,
    },
    #[error("automation {automation} sets a parameter another automation already sets")]
    TargetAutomatedTwice { automation: u32 },
    #[error("clip {clip}: start {start}, length {length} and offset {offset} must be at least 0, 1 and 0, and the clip must end by tick {max}")]
    InvalidClipSpan {
        clip: u32,
        start: i64,
        length: i64,
        offset: i64,
        max: i64,
    },
    #[error("loop {start}..{end} must start at tick 0 or later, span at least one step ({step} ticks) and end by tick {max}")]
    InvalidLoop {
        start: i64,
        end: i64,
        step: i64,
        max: i64,
    },
    #[error("the mixer has {0} inserts besides the master; it needs 1 to 500")]
    InsertCount(usize),
    #[error("channel '{channel}' plays into insert {insert}, which does not exist")]
    UnknownChannelInsert { channel: String, insert: u16 },
    #[error("insert {insert}: fader {fader} must be a number from 0 to 1")]
    InvalidFader { insert: u16, fader: f32 },
    #[error("insert {insert}: pan {pan} must be a number from -1 to 1")]
    InvalidInsertPan { insert: u16, pan: f32 },
    #[error("the master sends nowhere: its routes must be empty")]
    MasterSends,
    #[error("insert {from} sends to insert {to}, which does not exist")]
    UnknownSendTarget { from: u16, to: u16 },
    #[error("insert {0} sends to itself")]
    SendToItself(u16),
    #[error("insert {from}: the send to insert {to} has amount {amount}; it must be a number from 0 to 1")]
    InvalidSendAmount { from: u16, to: u16, amount: f32 },
    #[error("routing cycle through inserts {}", loop_text(.path))]
    RoutingCycle { path: Vec<u16> },
    #[error("insert {0} is soloed, but does not exist")]
    UnknownSolo(u16),
    #[error("clip {clip}: a stretch needs at least one frame and a pitch from -24 to 24 semitones, and a resample no pitch")]
    InvalidStretch { clip: u32 },
    #[error("a controller link listens to cc {cc} on MIDI channel {}; links take cc 0 to 119 on channels 1 to 16", channel_text(.channel))]
    InvalidMidiLink { cc: u8, channel: Option<u8> },
    #[error("cc {cc} on MIDI channel {} is linked to two parameters", channel_text(.channel))]
    ControllerLinkedTwice { cc: u8, channel: Option<u8> },
    #[error("MIDI input '{port}' is linked to two channels; a port plays one")]
    PortLinkedTwice { port: String },
    #[error("channel '{channel}' is linked to MIDI input '{port}'; an audio channel, or an input channel without a MIDI out, takes no device's notes")]
    PortOnAudioChannel { channel: String, port: String },
    #[error("channel '{channel}' is linked to a MIDI input with no name")]
    NamelessPort { channel: String },
    #[error(
        "channel '{channel}' has a MIDI out; only an input channel sends its notes to a MIDI port"
    )]
    MidiOutOnOtherKind { channel: String },
    #[error("channel '{channel}': its MIDI out's {setting} is missing or out of range")]
    InvalidMidiOut {
        channel: String,
        setting: &'static str,
    },
    #[error("insert {insert} holds {count} effects; it has 10 slots")]
    TooManyEffects { insert: u16, count: usize },
    #[error("insert {insert}: effect slot {slot} must be below 10, and slots must be listed in order, each once")]
    InvalidEffectSlot { insert: u16, slot: u8 },
    #[error("effect id {id} is not below next_effect_id {next}, so a new effect could reuse it")]
    UnallocatedEffectId { id: u32, next: u32 },
    #[error("effect id {0} appears twice")]
    EffectIdTwice(u32),
    #[error("effect {effect}: parameter '{key}' is {value}, not a finite number")]
    InvalidEffectParam {
        effect: u32,
        key: String,
        value: f32,
    },
}

/// A link's MIDI channel as people count it, 1 to 16, or "any".
fn channel_text(channel: &Option<u8>) -> String {
    channel.map_or_else(
        || "any".to_owned(),
        |channel| (u16::from(channel) + 1).to_string(),
    )
}

impl Project {
    /// An empty project: no channels or patterns, channel IDs starting at 1, and the default
    /// playlist.
    pub fn new(sample_rate: u32, tempo: MilliBpm, ppq: u16) -> Self {
        Self {
            sample_rate,
            tempo,
            ppq,
            next_channel_id: ChannelId(1),
            channels: BTreeMap::new(),
            next_pattern_id: None,
            patterns: BTreeMap::new(),
            playlist: Playlist::default(),
            mixer: Mixer::default(),
            automation: Automations::default(),
            midi_links: MidiLinks::default(),
        }
    }

    /// The ID a new pattern takes: above every ID given out so far, so the list keeps the
    /// order patterns were made in and no ID is used twice; 1 for the first; `None` once IDs
    /// are used up. `u32::MAX` itself is never given out, as with channels, so the next ID
    /// can always be written down.
    pub fn next_pattern_id(&self) -> Option<PatternId> {
        let next = self.pattern_counter();
        (next.0 != u32::MAX).then_some(next)
    }

    /// The pattern ID counter: the one written down, else the one the patterns imply.
    pub(crate) fn pattern_counter(&self) -> PatternId {
        self.next_pattern_id
            .unwrap_or_else(|| self.implied_pattern_counter())
    }

    /// One above the highest pattern ID (`u32::MAX` at most), or 1 with no pattern.
    fn implied_pattern_counter(&self) -> PatternId {
        self.patterns
            .keys()
            .next_back()
            .map_or(PatternId(1), |highest| {
                PatternId(highest.0.saturating_add(1))
            })
    }

    /// Sets the pattern ID counter, writing it down only when the patterns do not imply it.
    pub(crate) fn set_pattern_counter(&mut self, next: PatternId) {
        self.next_pattern_id = (next != self.implied_pattern_counter()).then_some(next);
    }

    /// Reads a project from JSON text: parse, convert from the file's `format_version`, move
    /// native effect ids saved under [`FORMER_NATIVE_ID_PREFIX`] to the current prefix, leave
    /// out the links no edit could put back (see [`LoadNote`]), then
    /// [`validate`](Self::validate). Every project read from a file comes through here.
    pub fn load_json(text: &str) -> Result<Self, LoadError> {
        Self::load_json_noting(text).map(|(project, _)| project)
    }

    /// As [`load_json`](Self::load_json), with what reading the file left out of the document.
    pub fn load_json_noting(text: &str) -> Result<(Self, Vec<LoadNote>), LoadError> {
        let VersionProbe { format_version } = serde_json::from_str(text)?;
        let mut project = match format_version {
            1 => serde_json::from_str::<v1::ProjectFile>(text)?.into_project()?,
            // Version 4 added the instrument channel, 5 the sound settings, 6 the input channel,
            // 7 its MIDI out and 8 a sound's edits, and nothing else, so they read with the same
            // shape; a version 2 file is a version 4 file without an instrument channel, a
            // version 4 file a version 5 file without sound settings, a version 5 file a version
            // 6 file without an input channel, a version 6 file a version 7 file without a MIDI
            // out, and a version 7 file a version 8 file without edits.
            FORMAT_VERSION
            | INSTRUMENT_FORMAT_VERSION
            | SOUND_FORMAT_VERSION
            | INPUT_FORMAT_VERSION
            | MIDI_OUT_FORMAT_VERSION
            | EDITS_FORMAT_VERSION => serde_json::from_str::<Self>(text)?,
            other => return Err(LoadError::UnsupportedVersion(other)),
        };
        project.mixer.adopt_former_native_ids();
        let notes = project.leave_out_unrestorable_links();
        project.validate()?;
        Ok((project, notes))
    }

    /// Leaves out every MIDI input link that no edit could put back, once undone: one on a
    /// channel that plays no notes, one naming no port, and a second link to one device under a
    /// name that differs only in the system's numbers (the channel first in the rack keeps
    /// it). Why on reading, not by refusing the file: such a link plays nothing a valid one
    /// would not, and a whole project is too much to lose over it. One name linked twice is
    /// left for `validate` to refuse: no build writes that.
    fn leave_out_unrestorable_links(&mut self) -> Vec<LoadNote> {
        let mut notes = Vec::new();
        // Each device linked so far, by its stable name: its whole name and its channel's.
        let mut linked: BTreeMap<String, (String, String)> = BTreeMap::new();
        for channel in self.channels.values_mut() {
            let note = match channel.midi_input.as_deref() {
                Some("") => LoadNote::NamelessPort {
                    channel: channel.name.clone(),
                },
                Some(port) if !channel.takes_midi_input() => LoadNote::PortOnAudioChannel {
                    channel: channel.name.clone(),
                    port: port.to_owned(),
                },
                Some(port) => match linked.get(stable_port_name(port)) {
                    Some((whole, kept)) if whole != port => LoadNote::SamePortTwice {
                        channel: channel.name.clone(),
                        port: port.to_owned(),
                        kept: kept.clone(),
                    },
                    Some(_) => continue,
                    None => {
                        let entry = (port.to_owned(), channel.name.clone());
                        linked.insert(stable_port_name(port).to_owned(), entry);
                        continue;
                    }
                },
                None => continue,
            };
            channel.midi_input = None;
            notes.push(note);
        }
        notes
    }

    /// The canonical JSON text of this project, in the current format (version 2; 4 when it has
    /// an instrument channel, 5 when a channel holds sound settings, 6 when it has an input
    /// channel, 7 when one of those has a MIDI out, 8 when a sound has edits): compact, fields in a fixed order, channels and patterns by
    /// ID, notes by (tick, channel, key). Equal projects give identical bytes, so an undo can be
    /// checked byte for byte.
    ///
    /// Fails only for a sample path that is not valid UTF-8.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        /// The file shape: the version first, then the document's own fields.
        #[derive(Serialize)]
        struct File<'a> {
            format_version: u32,
            #[serde(flatten)]
            project: &'a Project,
        }
        let instruments = self
            .channels
            .values()
            .any(|channel| channel.kind == ChannelKind::Instrument);
        let sounds = self
            .channels
            .values()
            .any(|channel| !channel.sound.is_default());
        let inputs = self
            .channels
            .values()
            .any(|channel| channel.kind == ChannelKind::Input);
        let midi_outs = self
            .channels
            .values()
            .any(|channel| channel.midi_out.is_some());
        let edits = self
            .channels
            .values()
            .any(|channel| !channel.edits.is_empty());
        let format_version = if edits {
            EDITS_FORMAT_VERSION
        } else if midi_outs {
            MIDI_OUT_FORMAT_VERSION
        } else if inputs {
            INPUT_FORMAT_VERSION
        } else if sounds {
            SOUND_FORMAT_VERSION
        } else if instruments {
            INSTRUMENT_FORMAT_VERSION
        } else {
            FORMAT_VERSION
        };
        serde_json::to_string(&File {
            format_version,
            project: self,
        })
    }

    /// Checks every rule the edits, the compiler and the player rely on.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if !SAMPLE_RATE_RANGE.contains(&self.sample_rate) {
            return Err(ValidationError::SampleRateOutOfRange(self.sample_rate));
        }
        if !TEMPO_RANGE.contains(&self.tempo.0) {
            return Err(ValidationError::TempoOutOfRange(self.tempo.0));
        }
        if self.ppq == 0 || !self.ppq.is_multiple_of(time::STEPS_PER_BEAT) {
            return Err(ValidationError::InvalidPpq(self.ppq));
        }
        if let (Some(next), Some(&highest)) =
            (self.next_pattern_id, self.patterns.keys().next_back())
        {
            if highest >= next {
                return Err(ValidationError::UnallocatedPatternId {
                    id: highest.0,
                    next: next.0,
                });
            }
        }
        let mut ports = BTreeSet::new();
        for (&id, channel) in &self.channels {
            if let Some(port) = &channel.midi_input {
                // The link edit refuses both, so no inverse could put either back.
                if port.is_empty() {
                    return Err(ValidationError::NamelessPort {
                        channel: channel.name.clone(),
                    });
                }
                if !channel.takes_midi_input() {
                    return Err(ValidationError::PortOnAudioChannel {
                        channel: channel.name.clone(),
                        port: port.clone(),
                    });
                }
                // One port, one channel, however the system numbered it when it was linked.
                if !ports.insert(stable_port_name(port)) {
                    return Err(ValidationError::PortLinkedTwice { port: port.clone() });
                }
            }
            if id >= self.next_channel_id {
                return Err(ValidationError::UnallocatedChannelId {
                    id: id.0,
                    next: self.next_channel_id.0,
                });
            }
            // `contains` is false for NaN and infinity, which are not volumes or pans.
            if !(0.0..=MAX_VOLUME).contains(&channel.volume) {
                return Err(ValidationError::InvalidVolume {
                    channel: channel.name.clone(),
                    volume: channel.volume,
                });
            }
            if !PAN_RANGE.contains(&channel.pan) {
                return Err(ValidationError::InvalidPan {
                    channel: channel.name.clone(),
                    pan: channel.pan,
                });
            }
            if !channel.sound_is_consistent() {
                return Err(ValidationError::InconsistentChannel {
                    channel: channel.name.clone(),
                });
            }
            if let Some(setting) = channel.sound.problem(channel.kind) {
                return Err(ValidationError::SoundSetting {
                    channel: channel.name.clone(),
                    setting,
                });
            }
            if let Some(problem) = channel.edits_problem() {
                return Err(problem);
            }
            if let Some(midi_out) = &channel.midi_out {
                // The edit refuses both, so no inverse could put either back.
                if channel.kind != ChannelKind::Input {
                    return Err(ValidationError::MidiOutOnOtherKind {
                        channel: channel.name.clone(),
                    });
                }
                if let Some(setting) = midi_out.problem() {
                    return Err(ValidationError::InvalidMidiOut {
                        channel: channel.name.clone(),
                        setting,
                    });
                }
            }
        }
        self.patterns
            .iter()
            .try_for_each(|(&id, pattern)| self.validate_pattern(id, pattern))?;
        self.validate_playlist()?;
        self.mixer.validate(&self.channels)?;
        self.automation.validate()?;
        self.midi_links.validate()
    }

    /// Whether `clip` plays: it is not muted and its track is not.
    pub fn clip_plays(&self, clip: &Clip) -> bool {
        !clip.muted
            && self
                .playlist
                .tracks
                .get(&clip.track)
                .is_some_and(|track| !track.mute)
    }

    /// The channel that plays MIDI input `port`'s notes whatever is selected: the one linked to
    /// it (compared by [`stable_port_name`]), when that takes notes.
    pub fn channel_for_port(&self, port: &str) -> Option<ChannelId> {
        self.channels
            .iter()
            .find(|(_, channel)| {
                channel.takes_midi_input()
                    && channel
                        .midi_input
                        .as_deref()
                        .is_some_and(|linked| same_port(linked, port))
            })
            .map(|(&id, _)| id)
    }

    /// Whether what `target` names is in the project: its channel (a plugin channel, for an
    /// instrument parameter), insert, send or effect. A parameter key or number is not checked
    /// here (the processor's own list is), so a target whose parameter the processor lacks
    /// does nothing, like one whose effect is gone.
    pub fn has_target(&self, target: &AutomationTarget) -> bool {
        let mixer = &self.mixer;
        match target {
            AutomationTarget::ChannelVolume(id) | AutomationTarget::ChannelPan(id) => {
                self.channels.contains_key(id)
            }
            AutomationTarget::ChannelParam { channel, .. } => self
                .channels
                .get(channel)
                .is_some_and(|channel| channel.kind == ChannelKind::Plugin),
            AutomationTarget::InsertFader(id) | AutomationTarget::InsertPan(id) => {
                mixer.get(*id).is_some()
            }
            AutomationTarget::Send { from, to } => mixer
                .get(*from)
                .is_some_and(|insert| insert.routes.contains_key(to)),
            AutomationTarget::EffectParam { effect, .. }
            | AutomationTarget::EffectPlugin { effect, .. } => mixer.effect(*effect).is_some(),
        }
    }

    fn validate_playlist(&self) -> Result<(), ValidationError> {
        let playlist = &self.playlist;
        if !(1..=MAX_TRACKS).contains(&playlist.tracks.len()) {
            return Err(ValidationError::TrackCount(playlist.tracks.len()));
        }
        if let Some(&id) = playlist.tracks.keys().next_back() {
            if id >= playlist.next_track_id {
                return Err(ValidationError::UnallocatedTrackId {
                    id: id.0,
                    next: playlist.next_track_id.0,
                });
            }
        }
        let audio_clips = playlist
            .clips
            .values()
            .filter(|clip| clip.audio_channel().is_some())
            .count();
        if audio_clips > MAX_AUDIO_CLIPS {
            return Err(ValidationError::TooManyAudioClips(audio_clips));
        }
        for (&id, clip) in &playlist.clips {
            if id >= playlist.next_clip_id {
                return Err(ValidationError::UnallocatedClipId {
                    id: id.0,
                    next: playlist.next_clip_id.0,
                });
            }
            self.check_clip(id, clip)?;
        }
        playlist
            .loop_range
            .as_ref()
            .map_or(Ok(()), |range| self.check_loop(range))
    }

    /// The rules one clip must follow: it sits on an existing track and plays what exists (a
    /// pattern; an audio channel's file, with no tick offset and a gain in
    /// [`CLIP_GAIN_RANGE`]; an automation), starts at tick 0 or later, is at least a tick long,
    /// and ends by [`max_song_tick`]. Edits check the clips they make with it too.
    pub(crate) fn check_clip(&self, id: ClipId, clip: &Clip) -> Result<(), ValidationError> {
        if !self.playlist.tracks.contains_key(&clip.track) {
            return Err(ValidationError::UnknownTrack {
                clip: id.0,
                track: clip.track.0,
            });
        }
        match clip.item {
            ClipItem::Pattern { pattern } if !self.patterns.contains_key(&pattern) => {
                return Err(ValidationError::UnknownClipPattern {
                    clip: id.0,
                    pattern: pattern.0,
                });
            }
            ClipItem::Pattern { .. } => {}
            ClipItem::Audio {
                channel,
                gain,
                stretch,
                ..
            } => {
                let Some(played) = self.channels.get(&channel) else {
                    return Err(ValidationError::UnknownClipChannel {
                        clip: id.0,
                        channel: channel.0,
                    });
                };
                if played.kind != ChannelKind::Audio {
                    return Err(ValidationError::NotAnAudioChannel {
                        clip: id.0,
                        channel: channel.0,
                    });
                }
                if clip.offset.0 != 0 {
                    return Err(ValidationError::AudioClipTickOffset {
                        clip: id.0,
                        offset: clip.offset.0,
                    });
                }
                // `contains` is false for NaN and infinity.
                if !CLIP_GAIN_RANGE.contains(&gain) {
                    return Err(ValidationError::InvalidClipGain { clip: id.0, gain });
                }
                if stretch.is_some_and(|stretch| !stretch.is_valid()) {
                    return Err(ValidationError::InvalidStretch { clip: id.0 });
                }
            }
            ClipItem::Automation { automation } => {
                if !self.automation.list.contains_key(&automation) {
                    return Err(ValidationError::UnknownClipAutomation {
                        clip: id.0,
                        automation: automation.0,
                    });
                }
            }
        }
        let max = max_song_tick(self.ppq);
        let fits = clip.start.0 >= 0
            && clip.length.0 >= 1
            && clip.offset.0 >= 0
            && clip
                .start
                .0
                .checked_add(clip.length.0)
                .is_some_and(|end| end <= max.0);
        if !fits {
            return Err(ValidationError::InvalidClipSpan {
                clip: id.0,
                start: clip.start.0,
                length: clip.length.0,
                offset: clip.offset.0,
                max: max.0,
            });
        }
        Ok(())
    }

    /// The rules a loop range must follow: it starts at tick 0 or later, ends by
    /// [`max_song_tick`], and spans at least one step, so a loop pass is never empty.
    pub(crate) fn check_loop(&self, range: &LoopRange) -> Result<(), ValidationError> {
        let step = time::step_to_ticks(1, self.ppq);
        let max = max_song_tick(self.ppq);
        let span = range.end.0.checked_sub(range.start.0);
        if range.start.0 < 0 || range.end > max || span.is_none_or(|span| span < step.0) {
            return Err(ValidationError::InvalidLoop {
                start: range.start.0,
                end: range.end.0,
                step: step.0,
                max: max.0,
            });
        }
        Ok(())
    }

    pub(crate) fn validate_pattern(
        &self,
        id: PatternId,
        pattern: &Pattern,
    ) -> Result<(), ValidationError> {
        let name = || pattern.name.clone();
        if pattern.length_bars == 0 {
            return Err(ValidationError::EmptyPattern { pattern: name() });
        }
        if let Some(from) = pattern.copied_from.filter(|&from| from >= id) {
            return Err(ValidationError::CopyOfLaterPattern {
                pattern: name(),
                from: from.0,
            });
        }
        for note in &pattern.notes {
            if !self.channels.contains_key(&note.channel) {
                return Err(ValidationError::UnknownChannel {
                    pattern: name(),
                    channel: note.channel.0,
                });
            }
            if note.start < Ticks(0) {
                return Err(ValidationError::NoteOutsidePattern {
                    pattern: name(),
                    start: note.start.0,
                    length: time::bars_to_ticks(pattern.length_bars, self.ppq).0,
                });
            }
            if note.length < Ticks(0) {
                return Err(ValidationError::NegativeLength {
                    pattern: name(),
                    length: note.length.0,
                });
            }
            if note.key > MAX_KEY {
                return Err(ValidationError::InvalidKey {
                    pattern: name(),
                    key: note.key,
                });
            }
            if !(0.0..=1.0).contains(&note.velocity) {
                return Err(ValidationError::InvalidVelocity {
                    pattern: name(),
                    velocity: note.velocity,
                });
            }
        }
        // Strictly increasing (tick, channel, key) means sorted and one note per channel, tick
        // and key.
        let misplaced = pattern
            .notes
            .windows(2)
            .find(|pair| pair[0].order_key() >= pair[1].order_key());
        match misplaced {
            Some(pair) => Err(ValidationError::NotesOutOfOrder {
                pattern: name(),
                tick: pair[1].start.0,
                channel: pair[1].channel.0,
                key: pair[1].key,
            }),
            None => Ok(()),
        }
    }
}

/// The version field alone. It is read first so that each version is parsed with its own shape.
#[derive(Deserialize)]
struct VersionProbe {
    format_version: u32,
}

/// Serde for an ID-keyed map stored as a JSON array of objects whose first field is `"id"`,
/// such as `[{"id":1,"name":"kick",...}]`. Loading rejects an ID that appears twice.
mod id_list {
    use std::collections::BTreeMap;
    use std::fmt::Debug;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize)]
    struct EntryRef<'a, K, V> {
        id: &'a K,
        #[serde(flatten)]
        value: &'a V,
    }

    #[derive(Deserialize)]
    struct Entry<K, V> {
        id: K,
        #[serde(flatten)]
        value: V,
    }

    pub(super) fn serialize<S, K, V>(map: &BTreeMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        K: Serialize,
        V: Serialize,
    {
        serializer.collect_seq(map.iter().map(|(id, value)| EntryRef { id, value }))
    }

    pub(super) fn deserialize<'de, D, K, V>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        D: Deserializer<'de>,
        K: Deserialize<'de> + Ord + Debug,
        V: Deserialize<'de>,
    {
        let mut map = BTreeMap::new();
        for Entry { id, value } in Vec::<Entry<K, V>>::deserialize(deserializer)? {
            if map.contains_key(&id) {
                let message = format_args!("{id:?} appears twice");
                return Err(<D::Error as serde::de::Error>::custom(message));
            }
            map.insert(id, value);
        }
        Ok(map)
    }
}

/// Project JSON, format version 1: tempo as decimal BPM, notes as lit steps, and channels and
/// patterns identified by their position in the file.
mod v1 {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use serde::Deserialize;

    use super::{
        Automations, Channel, ChannelId, LoadError, MidiLinks, Mixer, Note, Pattern, PatternId,
        Playlist, Project, ValidationError, DEFAULT_KEY,
    };
    use crate::time::{self, MilliBpm, Ticks};

    #[derive(Deserialize)]
    pub(super) struct ProjectFile {
        sample_rate: u32,
        tempo_bpm: f64,
        ppq: u16,
        channels: Vec<ChannelFile>,
        patterns: Vec<PatternFile>,
    }

    #[derive(Deserialize)]
    struct ChannelFile {
        name: String,
        kind: ChannelKind,
        sample: PathBuf,
        volume: f32,
    }

    /// The channel kinds version 1 knows.
    #[derive(Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum ChannelKind {
        Sampler,
    }

    #[derive(Deserialize)]
    struct PatternFile {
        name: String,
        length_bars: u32,
        steps: Vec<StepRow>,
    }

    /// One channel's lit steps in a pattern.
    #[derive(Deserialize)]
    struct StepRow {
        channel: u32,
        hits: Vec<StepHit>,
    }

    #[derive(Deserialize)]
    struct StepHit {
        step: u32,
        velocity: f32,
    }

    impl ProjectFile {
        /// Migrates to the current model. Version 1 names channels and patterns by position;
        /// the current format gives each a stable ID, position + 1.
        pub(super) fn into_project(self) -> Result<Project, LoadError> {
            let tempo = MilliBpm::from_bpm(self.tempo_bpm)
                .ok_or(LoadError::InvalidTempo(self.tempo_bpm))?;
            let ppq = self.ppq;
            let channel_count = self.channels.len();
            // `1..=u32::MAX` ends instead of overflowing, and no file that fits in memory has
            // more entries than that.
            let channels: BTreeMap<_, _> = (1..=u32::MAX)
                .zip(self.channels)
                .map(|(id, channel)| (ChannelId(id), channel.into_channel()))
                .collect();
            let patterns = (1..=u32::MAX)
                .zip(self.patterns)
                .map(|(id, pattern)| Ok((PatternId(id), pattern.into_pattern(ppq, channel_count)?)))
                .collect::<Result<_, ValidationError>>()?;
            let next_channel_id = channels
                .keys()
                .next_back()
                .map_or(1, |last: &ChannelId| last.0.saturating_add(1));
            Ok(Project {
                sample_rate: self.sample_rate,
                tempo,
                ppq,
                next_channel_id: ChannelId(next_channel_id),
                channels,
                next_pattern_id: None,
                patterns,
                playlist: Playlist::default(),
                mixer: Mixer::default(),
                automation: Automations::default(),
                midi_links: MidiLinks::default(),
            })
        }
    }

    impl ChannelFile {
        fn into_channel(self) -> Channel {
            match self.kind {
                ChannelKind::Sampler => Channel {
                    volume: self.volume,
                    ..Channel::sampler(self.name, self.sample)
                },
            }
        }
    }

    impl PatternFile {
        /// Each lit step becomes a zero-length note at `step * ppq/4`. The rules version 1 had
        /// (rows name an existing channel, steps lie inside a non-empty pattern) are checked here,
        /// in version 1's own numbering; version 1 had no rule for note order, so notes are sorted.
        fn into_pattern(self, ppq: u16, channel_count: usize) -> Result<Pattern, ValidationError> {
            if self.length_bars == 0 {
                return Err(ValidationError::EmptyPattern { pattern: self.name });
            }
            let length = time::bars_to_ticks(self.length_bars, ppq);
            let mut notes = Vec::new();
            for row in &self.steps {
                if !usize::try_from(row.channel).is_ok_and(|index| index < channel_count) {
                    return Err(ValidationError::UnknownChannel {
                        pattern: self.name,
                        channel: row.channel,
                    });
                }
                // Below `channel_count`, which is at most u32::MAX, so adding 1 cannot overflow.
                let channel = ChannelId(row.channel + 1);
                for hit in &row.hits {
                    let start = time::step_to_ticks(hit.step, ppq);
                    if start >= length {
                        return Err(ValidationError::NoteOutsidePattern {
                            pattern: self.name,
                            start: start.0,
                            length: length.0,
                        });
                    }
                    notes.push(Note {
                        channel,
                        start,
                        length: Ticks(0),
                        velocity: hit.velocity,
                        key: DEFAULT_KEY,
                    });
                }
            }
            notes.sort_by_key(Note::order_key);
            Ok(Pattern {
                name: self.name,
                length_bars: self.length_bars,
                copied_from: None,
                notes,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    const F1: &str = include_str!("../../../tests/fixtures/f1_four_on_floor.json");

    /// F1 with one piece of text replaced. Fails if the text is not there, so a typo here
    /// cannot quietly turn a test into a check of the unchanged fixture.
    fn f1_with(from: &str, to: &str) -> String {
        assert!(F1.contains(from), "fixture has no {from:?}");
        F1.replace(from, to)
    }

    /// F1 with its first note changed by `change`.
    fn f1_first_note(change: impl FnOnce(&mut Note)) -> Project {
        let mut project = Project::load_json(F1).expect("F1 loads");
        let pattern = project.patterns.get_mut(&PatternId(1)).expect("pattern 1");
        change(&mut pattern.notes[0]);
        project
    }

    // Expected: steps 0, 4, 8, 12 at ppq/4 = 24 ticks per step are ticks 0, 96, 192, 288.
    #[test]
    fn v1_steps_load_as_zero_length_notes() {
        let project = Project::load_json(F1).expect("F1 loads");
        assert_eq!(project.sample_rate, 44100);
        assert_eq!(project.tempo, MilliBpm(120_000));
        assert_eq!(project.ppq, 96);
        let kick = &project.channels[&ChannelId(1)];
        assert_eq!(kick.name, "kick");
        assert_eq!(kick.sample, Path::new("impulse.wav"));
        assert_eq!(kick.volume, 1.0);
        assert_eq!((kick.pan, kick.mute), (0.0, false));
        assert_eq!(project.next_channel_id, ChannelId(2));
        let pattern = &project.patterns[&PatternId(1)];
        assert_eq!(pattern.length_bars, 1);
        let expected: Vec<Note> = [0, 96, 192, 288]
            .into_iter()
            .map(|tick| Note {
                channel: ChannelId(1),
                start: Ticks(tick),
                length: Ticks(0),
                key: DEFAULT_KEY,
                velocity: 1.0,
            })
            .collect();
        assert_eq!(pattern.notes, expected);
    }

    #[test]
    fn an_unsupported_version_is_named() {
        let result = Project::load_json(&f1_with(r#""format_version":1"#, r#""format_version":3"#));
        assert!(
            matches!(result, Err(LoadError::UnsupportedVersion(3))),
            "{result:?}"
        );
    }

    /// A broken-file case: text to replace in F1, its replacement, and the error expected.
    type BrokenFile = (&'static str, &'static str, fn(&LoadError) -> bool);

    #[test]
    fn broken_files_are_rejected_with_the_rule_they_break() {
        use LoadError::Invalid;
        use ValidationError as V;
        let cases: [BrokenFile; 10] = [
            (r#""sample_rate":44100"#, r#""sample_rate":0"#, |e| {
                matches!(e, Invalid(V::SampleRateOutOfRange(0)))
            }),
            (r#""tempo_bpm":120.0"#, r#""tempo_bpm":0.0"#, |e| {
                matches!(e, LoadError::InvalidTempo(_))
            }),
            (r#""ppq":96"#, r#""ppq":90"#, |e| {
                matches!(e, Invalid(V::InvalidPpq(90)))
            }),
            (r#""ppq":96"#, r#""ppq":70000"#, |e| {
                matches!(e, LoadError::Json(_))
            }),
            (r#""kind":"sampler""#, r#""kind":"synth""#, |e| {
                matches!(e, LoadError::Json(_))
            }),
            (r#""volume":1.0"#, r#""volume":-1.0"#, |e| {
                matches!(e, Invalid(V::InvalidVolume { .. }))
            }),
            (r#""length_bars":1"#, r#""length_bars":0"#, |e| {
                matches!(e, Invalid(V::EmptyPattern { .. }))
            }),
            (r#""channel":0"#, r#""channel":3"#, |e| {
                matches!(e, Invalid(V::UnknownChannel { channel: 3, .. }))
            }),
            (r#"{"step":12,"#, r#"{"step":16,"#, |e| {
                matches!(
                    e,
                    Invalid(V::NoteOutsidePattern {
                        start: 384,
                        length: 384,
                        ..
                    })
                )
            }),
            (
                r#"{"step":0,"velocity":1.0}"#,
                r#"{"step":0,"velocity":1.5}"#,
                |e| matches!(e, Invalid(V::InvalidVelocity { .. })),
            ),
        ];
        for (from, to, is_expected) in cases {
            match Project::load_json(&f1_with(from, to)) {
                Err(error) => assert!(is_expected(&error), "{to}: wrong error {error:?}"),
                Ok(_) => panic!("{to}: loaded, but should have been rejected"),
            }
        }
        assert!(matches!(
            Project::load_json("not json"),
            Err(LoadError::Json(_))
        ));
    }

    #[test]
    fn rules_the_file_format_cannot_break_are_checked_too() {
        let project = f1_first_note(|note| note.key = 128);
        assert!(matches!(
            project.validate(),
            Err(ValidationError::InvalidKey { key: 128, .. })
        ));
        let project = f1_first_note(|note| note.length = Ticks(-1));
        assert!(matches!(
            project.validate(),
            Err(ValidationError::NegativeLength { length: -1, .. })
        ));
        let mut project = Project::load_json(F1).expect("F1 loads");
        project.tempo = MilliBpm(0);
        assert_eq!(project.validate(), Err(ValidationError::TempoOutOfRange(0)));
    }

    /// F1 with one field changed, validated.
    fn validated(change: impl FnOnce(&mut Project)) -> Result<(), ValidationError> {
        let mut project = Project::load_json(F1).expect("F1 loads");
        change(&mut project);
        project.validate()
    }

    #[test]
    fn tempo_must_be_within_10_to_522_bpm() {
        for inside in [10_000, 120_000, 522_000] {
            assert_eq!(
                validated(|p| p.tempo = MilliBpm(inside)),
                Ok(()),
                "{inside}"
            );
        }
        for outside in [9_999, 522_001] {
            assert_eq!(
                validated(|p| p.tempo = MilliBpm(outside)),
                Err(ValidationError::TempoOutOfRange(outside))
            );
        }
    }

    #[test]
    fn sample_rate_must_be_within_8k_to_384k() {
        for inside in [8_000, 44_100, 384_000] {
            assert_eq!(validated(|p| p.sample_rate = inside), Ok(()), "{inside}");
        }
        for outside in [7_999, 384_001] {
            assert_eq!(
                validated(|p| p.sample_rate = outside),
                Err(ValidationError::SampleRateOutOfRange(outside))
            );
        }
    }

    #[test]
    fn channel_volume_must_be_finite_and_within_0_to_4() {
        let set = |volume: f32| {
            validated(|p| p.channels.get_mut(&ChannelId(1)).expect("kick").volume = volume)
        };
        for inside in [0.0, 1.0, 4.0] {
            assert_eq!(set(inside), Ok(()), "{inside}");
        }
        for outside in [-0.1, 4.1, f32::INFINITY, f32::NAN] {
            assert!(
                matches!(set(outside), Err(ValidationError::InvalidVolume { .. })),
                "{outside}"
            );
        }
    }

    const P2_THREE: &str = include_str!("../../../tests/fixtures/p2_three_patterns.json");

    /// The three-pattern fixture with one change, validated.
    fn v2_validated(change: impl FnOnce(&mut Project)) -> Result<(), ValidationError> {
        let mut project = Project::load_json(P2_THREE).expect("fixture loads");
        change(&mut project);
        project.validate()
    }

    #[test]
    fn current_format_rules_are_checked() {
        fn notes(project: &mut Project) -> &mut Vec<Note> {
            &mut project.patterns.get_mut(&PatternId(1)).expect("p1").notes
        }
        assert!(matches!(
            v2_validated(|p| notes(p).swap(0, 1)),
            Err(ValidationError::NotesOutOfOrder { tick: 0, .. })
        ));
        assert!(matches!(
            v2_validated(|p| {
                let first = notes(p)[0].clone();
                notes(p).insert(0, first);
            }),
            Err(ValidationError::NotesOutOfOrder {
                tick: 0,
                channel: 1,
                ..
            })
        ));
        assert!(matches!(
            v2_validated(|p| notes(p)[0].start = Ticks(-1)),
            Err(ValidationError::NoteOutsidePattern { start: -1, .. })
        ));
        assert_eq!(
            v2_validated(|p| p.next_channel_id = ChannelId(1)),
            Err(ValidationError::UnallocatedChannelId { id: 1, next: 1 })
        );
        for pan in [1.5, f32::NAN] {
            assert!(matches!(
                v2_validated(|p| p.channels.get_mut(&ChannelId(1)).expect("kick").pan = pan),
                Err(ValidationError::InvalidPan { .. })
            ));
        }
        // A note past the pattern's end is kept (it is silent), so it is valid.
        assert_eq!(v2_validated(|p| notes(p)[3].start = Ticks(500)), Ok(()));
    }

    // A channel plays a chord as several notes on one tick, one per key, sorted by key.
    #[test]
    fn one_channel_may_hold_one_note_per_key_on_a_tick() {
        fn notes(project: &mut Project) -> &mut Vec<Note> {
            &mut project.patterns.get_mut(&PatternId(1)).expect("p1").notes
        }
        let with_key = |key| {
            move |p: &mut Project| {
                let second = Note {
                    key,
                    ..notes(p)[0].clone()
                };
                notes(p).insert(1, second);
            }
        };
        assert_eq!(v2_validated(with_key(64)), Ok(()));
        assert!(matches!(
            v2_validated(with_key(DEFAULT_KEY)),
            Err(ValidationError::NotesOutOfOrder {
                tick: 0,
                channel: 1,
                key: DEFAULT_KEY,
                ..
            })
        ));
        // Key 59 sorts before the first note's key 60, so it is out of place after it.
        assert!(matches!(
            v2_validated(with_key(59)),
            Err(ValidationError::NotesOutOfOrder {
                tick: 0,
                channel: 1,
                key: 59,
                ..
            })
        ));
    }

    #[test]
    fn a_new_project_is_valid_and_round_trips() {
        let project = Project::new(44_100, MilliBpm(120_000), 96);
        assert_eq!(project.validate(), Ok(()));
        let saved = project.to_json().expect("serializes");
        assert_eq!(Project::load_json(&saved).expect("loads"), project);
    }

    const P4_LOOP: &str = include_str!("../../../tests/fixtures/p4_loop.json");

    /// p4_loop (clips 1 to 3 on track 1, loop 384..768) with one change, validated.
    fn p4_validated(change: impl FnOnce(&mut Project)) -> Result<(), ValidationError> {
        let mut project = Project::load_json(P4_LOOP).expect("fixture loads");
        change(&mut project);
        project.validate()
    }

    fn clip_1(project: &mut Project) -> &mut Clip {
        project.playlist.clips.get_mut(&ClipId(1)).expect("clip 1")
    }

    // A new playlist: 8 tracks "Track 1".."Track 8" (ids 1-8, unmuted), no clips, no loop, next ids
    // 9 and 1; a file without a playlist loads it, and it is not written back.
    #[test]
    fn a_project_without_a_playlist_has_the_default_one_and_writes_none() {
        let project = Project::load_json(P2_THREE).expect("fixture loads");
        let playlist = &project.playlist;
        assert_eq!(
            (playlist.next_track_id, playlist.next_clip_id),
            (TrackId(9), ClipId(1))
        );
        let tracks: Vec<(u32, &str, bool)> = playlist
            .tracks
            .iter()
            .map(|(id, track)| (id.0, track.name.as_str(), track.mute))
            .collect();
        let expected: Vec<(u32, String, bool)> = (1..=8)
            .map(|id| (id, format!("Track {id}"), false))
            .collect();
        let expected: Vec<(u32, &str, bool)> = expected
            .iter()
            .map(|(id, name, mute)| (*id, name.as_str(), *mute))
            .collect();
        assert_eq!(tracks, expected);
        assert!(playlist.clips.is_empty() && playlist.loop_range.is_none());
        assert_eq!(project.playlist, Playlist::default());
        let saved = project.to_json().expect("serializes");
        assert!(!saved.contains("playlist"), "{saved}");
        // The default playlist written out in full still loads as the default.
        let explicit = saved.trim_end_matches('}').to_owned()
            + r#","playlist":{"next_track_id":9,"next_clip_id":1,"tracks":["#
            + &(1..=8)
                .map(|id| format!(r#"{{"id":{id},"name":"Track {id}","mute":false}}"#))
                .collect::<Vec<_>>()
                .join(",")
            + r#"],"clips":[]}}"#;
        let again = Project::load_json(&explicit).expect("explicit default loads");
        assert_eq!(again, project);
        assert_eq!(again.to_json().expect("serializes"), saved);
    }

    // Playlist validation: an audio clip, an unknown pattern or track, a 23-tick loop, and start +
    // length past MAX_SONG_TICK (384 × 100000 = 38400000 at ppq 96) are each refused.
    #[test]
    fn playlist_rules_are_checked() {
        use ValidationError as V;
        assert_eq!(p4_validated(|_| {}), Ok(()));
        // Channel 1 is a sampler: an audio clip plays an audio channel's file.
        let audio = ClipItem::Audio {
            channel: ChannelId(1),
            frame_offset: 0,
            gain: 1.0,
            stretch: None,
        };
        assert_eq!(
            p4_validated(|p| clip_1(p).item = audio),
            Err(V::NotAnAudioChannel {
                clip: 1,
                channel: 1
            })
        );
        let pattern_9 = ClipItem::Pattern {
            pattern: PatternId(9),
        };
        assert_eq!(
            p4_validated(|p| clip_1(p).item = pattern_9),
            Err(V::UnknownClipPattern {
                clip: 1,
                pattern: 9
            })
        );
        assert_eq!(
            p4_validated(|p| clip_1(p).track = TrackId(9)),
            Err(V::UnknownTrack { clip: 1, track: 9 })
        );
        let range = |start, end| {
            Some(LoopRange {
                start: Ticks(start),
                end: Ticks(end),
                on: true,
            })
        };
        let max = 38_400_000;
        assert_eq!(max_song_tick(96), Ticks(max));
        assert_eq!(
            p4_validated(|p| p.playlist.loop_range = range(384, 407)),
            Err(V::InvalidLoop {
                start: 384,
                end: 407,
                step: 24,
                max
            })
        );
        assert_eq!(
            p4_validated(|p| p.playlist.loop_range = range(384, 408)),
            Ok(())
        );
        assert!(matches!(
            p4_validated(|p| p.playlist.loop_range = range(-1, 384)),
            Err(V::InvalidLoop { start: -1, .. })
        ));
        assert!(matches!(
            p4_validated(|p| p.playlist.loop_range = range(0, max + 1)),
            Err(V::InvalidLoop { .. })
        ));
        // A clip may end exactly on MAX_SONG_TICK, not one tick later.
        let at = |start, length| {
            move |p: &mut Project| {
                clip_1(p).start = Ticks(start);
                clip_1(p).length = Ticks(length);
            }
        };
        assert_eq!(p4_validated(at(max - 384, 384)), Ok(()));
        let past_the_end = V::InvalidClipSpan {
            clip: 1,
            start: max - 383,
            length: 384,
            offset: 0,
            max,
        };
        assert_eq!(p4_validated(at(max - 383, 384)), Err(past_the_end));
        assert!(matches!(
            p4_validated(at(0, 0)),
            Err(V::InvalidClipSpan { length: 0, .. })
        ));
        assert!(matches!(
            p4_validated(at(-1, 384)),
            Err(V::InvalidClipSpan { start: -1, .. })
        ));
        assert!(matches!(
            p4_validated(|p| clip_1(p).offset = Ticks(-1)),
            Err(V::InvalidClipSpan { offset: -1, .. })
        ));
    }

    #[test]
    fn playlist_ids_and_track_counts_are_checked() {
        use ValidationError as V;
        assert_eq!(
            p4_validated(|p| p.playlist.next_clip_id = ClipId(3)),
            Err(V::UnallocatedClipId { id: 3, next: 3 })
        );
        assert_eq!(
            p4_validated(|p| p.playlist.next_track_id = TrackId(8)),
            Err(V::UnallocatedTrackId { id: 8, next: 8 })
        );
        assert_eq!(
            p4_validated(|p| {
                p.playlist.clips.clear();
                p.playlist.tracks.clear();
            }),
            Err(V::TrackCount(0))
        );
        let tracks = |count: u32| {
            move |p: &mut Project| {
                p.playlist.tracks = (1..=count)
                    .map(|id| (TrackId(id), Track::named("t")))
                    .collect();
                p.playlist.next_track_id = TrackId(count + 1);
            }
        };
        assert_eq!(p4_validated(tracks(500)), Ok(()));
        assert_eq!(p4_validated(tracks(501)), Err(V::TrackCount(501)));
    }

    // An audio clip parses, and validation refuses one that names a sampler channel.
    #[test]
    fn an_audio_clip_of_a_sampler_channel_is_refused() {
        let pattern_clip =
            r#""id":1,"track":1,"start":0,"length":384,"kind":"pattern","pattern":1"#;
        assert!(P4_LOOP.contains(pattern_clip));
        let audio_clip = P4_LOOP.replace(
            pattern_clip,
            r#""id":1,"track":1,"start":0,"length":384,"kind":"audio","channel":1"#,
        );
        assert!(matches!(
            Project::load_json(&audio_clip),
            Err(LoadError::Invalid(ValidationError::NotAnAudioChannel {
                clip: 1,
                channel: 1
            }))
        ));
        let unknown_kind = P4_LOOP.replace(r#""kind":"pattern""#, r#""kind":"video""#);
        assert!(matches!(
            Project::load_json(&unknown_kind),
            Err(LoadError::Json(_))
        ));
    }

    // Offset and muted are written only when set, after the item's fields.
    #[test]
    fn a_clip_writes_offset_and_muted_only_when_set() {
        let mut project = Project::load_json(P4_LOOP).expect("fixture loads");
        let clip = clip_1(&mut project);
        clip.offset = Ticks(96);
        clip.muted = true;
        let saved = project.to_json().expect("serializes");
        let written = r#"{"id":1,"track":1,"start":0,"length":384,"kind":"pattern","pattern":1,"offset":96,"muted":true}"#;
        assert!(saved.contains(written), "{saved}");
        assert_eq!(Project::load_json(&saved).expect("loads"), project);
    }

    #[test]
    fn an_id_used_twice_is_rejected() {
        let channel = r#"{"id":1,"name":"kick","kind":"sampler","sample":"impulse.wav","volume":1.0,"pan":0.0,"mute":false}"#;
        let twice = P2_THREE.replacen(channel, &format!("{channel},{channel}"), 1);
        assert_ne!(twice, P2_THREE, "fixture has no {channel}");
        assert!(matches!(
            Project::load_json(&twice),
            Err(LoadError::Json(_))
        ));
    }
}
