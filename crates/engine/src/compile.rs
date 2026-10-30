//! Turns the project document into the [`PlanUpdate`] the player reads: a new plan, rebuilding
//! only the patterns an edit touched, the processors to create, drop or retune (the effects in
//! mixer slots and the instruments of plugin channels), and the sources of the audio clips.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

use crate::dsp::edits;
use crate::dsp::instrument::Instrument;
use crate::dsp::laws::{channel_pan_gains, fader_gain};
use crate::dsp::sampler::{playback_speed, SampleBuffer};
use crate::dsp::shape::Shape;
use crate::edit::Touched;
use crate::plan::{
    AudioClipPlan, AutomationPlan, AutomationWindow, ChannelPlan, ChannelSound, ClipPlan,
    InsertPlan, InstrumentSound, MidiOutPlan, MixerPlan, NoteEvent, PatternPlan, PlanUpdate,
    PlayMode, PlayPlan, Playing, ProcessorSlot, Ramp, Rate, RuntimeDelta, SendPlan, ShapedSample,
    SongPlan, PLUGIN_CHANNELS, PROCESSOR_SLOTS, TABLE_SLOTS,
};
use crate::processor::{param_values, HostError, ProcessContext, Processor, Registry};
use crate::project::{
    loop_text, AutomationId, AutomationPoint, AutomationTarget, ChannelId, ChannelKind, Clip,
    ClipId, ClipItem, ClipStretch, InsertId, Pattern, PatternId, ProcessorOwner, ProcessorRef,
    Project, SoundEdits, MAX_AUDIO_CLIPS, MAX_VOLUME,
};
use crate::source::{ClipSource, MemorySource, SourceSlot};
use crate::time::{self, Ticks};
use crate::transport::ParamTarget;

/// Why a compile failed. A failed compile changes nothing the next compile depends on.
#[derive(Debug, Error)]
pub enum CompileError<E> {
    /// A sample file could not be loaded; the loader's own error.
    #[error(transparent)]
    Load(E),
    #[error(transparent)]
    Mixer(#[from] MixerError),
}

/// A mixer the player cannot run.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MixerError {
    /// The sends form a loop: `validate` and the edits refuse one, so only a project that never
    /// passed them can get here.
    #[error("routing cycle through inserts {}", loop_text(&.0.iter().map(|id| id.0).collect::<Vec<_>>()))]
    RoutingCycle(Vec<InsertId>),
    #[error("every one of the {PROCESSOR_SLOTS} effect slots the player has is in use")]
    SlotsFull,
    #[error("a project plays at most {PLUGIN_CHANNELS} plugin channels")]
    PluginChannelsFull,
}

/// Where a compile gets the sounds a project plays: the decoded files of sampler channels, and
/// a source for each audio clip. The media layer implements it with real files; tests with
/// buffers of their own.
pub trait Sounds {
    type Error;

    /// The decoded sound of a channel's file, given its path as the project writes it; `None`
    /// while it loads in the background (the channel plays silence until a later compile
    /// gets it).
    fn sample(&mut self, path: &Path) -> Result<Option<SampleBuffer>, Self::Error>;

    /// The sound of a new audio clip of the file at `path`: decoded into memory (shared by
    /// every clip of the file), a source of its own (a stream from disk), or nothing yet while
    /// the file loads. A file that cannot be read is a silent memory sound, as for samples. A
    /// stretched clip (`hint.stretch`) is given the stretched file, never the file itself.
    fn clip(&mut self, path: &Path, hint: ClipHint) -> Result<ClipSound, Self::Error>;

    /// The instrument set an instrument channel plays, given its .sfz file's path as the
    /// project writes it, every zone's sound decoded; `None` while they load in the background
    /// (the channel plays silence until a later compile gets it). By default an empty set: a
    /// source of sound files alone has no sets, and its instrument channels stay silent.
    fn instrument(&mut self, path: &Path) -> Result<Option<Instrument>, Self::Error> {
        let _ = path;
        Ok(Some(Instrument::default()))
    }

    /// The sound of a channel's file played through `edits` (never empty), given the file's
    /// path as the project writes it; `None` while it loads. A sound given
    /// [`for_now`](EditedSound::for_now) is not the edited sound yet but one to play while it
    /// is made (the sound the channel had): the compiler plays it in this plan alone and asks
    /// again at the next compile. By default the file as [`sample`](Self::sample) gives it,
    /// edited here ([`edits::apply`]).
    fn edited_sample(
        &mut self,
        path: &Path,
        edits: &SoundEdits,
    ) -> Result<Option<EditedSound<SampleBuffer>>, Self::Error> {
        Ok(self
            .sample(path)?
            .map(|sample| EditedSound::ready(edits::apply(&sample, edits))))
    }

    /// The sound of a new audio clip of the file at `path` played through `edits` (never
    /// empty), as [`clip`](Self::clip) gives one of the file itself; a stretched clip stretches
    /// the edited sound. A sound for now as for [`edited_sample`](Self::edited_sample). By
    /// default the file as [`sample`](Self::sample) gives it, edited here; a stretched clip
    /// plays silence, as with [`Compiler::compile`] (stretching is the media layer's).
    fn edited_clip(
        &mut self,
        path: &Path,
        edits: &SoundEdits,
        hint: ClipHint,
    ) -> Result<EditedSound<ClipSound>, Self::Error> {
        if hint.stretch.is_some() {
            return Ok(EditedSound::ready(ClipSound::Memory(SampleBuffer::from(
                Vec::new(),
            ))));
        }
        let sound = match self.sample(path)? {
            Some(sample) => ClipSound::Memory(edits::apply(&sample, edits)),
            None => ClipSound::Loading,
        };
        Ok(EditedSound::ready(sound))
    }

    /// Told, once a compile has asked for every sound it needs, the project it compiles: what
    /// no channel or clip of `project` plays any more may be let go. Nothing by default.
    fn forget_unused(&mut self, _project: &Project) {}
}

/// Where a new audio clip will first read its file, so a stream can fill from there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipHint {
    /// The clip's `frame_offset`.
    pub first_frame: u64,
    /// The frame the loop start plays, when the loop starts inside the clip.
    pub loop_frame: Option<u64>,
    /// How the clip stretches its file; both frames above count the stretched file's.
    pub stretch: Option<ClipStretch>,
}

/// What [`Sounds::edited_sample`] and [`Sounds::edited_clip`] give: the edited sound, or one to
/// play for now while it is made.
pub struct EditedSound<T> {
    pub sound: T,
    /// Not the edited sound yet: played in one plan, never kept, asked for again.
    pub for_now: bool,
}

impl<T> EditedSound<T> {
    /// The edited sound itself.
    pub fn ready(sound: T) -> Self {
        Self {
            sound,
            for_now: false,
        }
    }

    /// A sound to play while the edited one is made.
    pub fn for_now(sound: T) -> Self {
        Self {
            sound,
            for_now: true,
        }
    }
}

/// What [`Sounds::clip`] gives.
pub enum ClipSound {
    Memory(SampleBuffer),
    Stream(Box<dyn ClipSource>),
    /// Still loading: the clip plays silence until a later compile gets its sound.
    Loading,
}

/// [`Sounds`] from one function that decodes a file into memory, for every sampler and clip.
struct DecodeFn<F>(F);

impl<F, E> Sounds for DecodeFn<F>
where
    F: FnMut(&Path) -> Result<SampleBuffer, E>,
{
    type Error = E;

    fn sample(&mut self, path: &Path) -> Result<Option<SampleBuffer>, E> {
        (self.0)(path).map(Some)
    }

    /// Why a stretched clip plays silence here: stretching is the media layer's, and a
    /// function that decodes files knows nothing of it.
    fn clip(&mut self, path: &Path, hint: ClipHint) -> Result<ClipSound, E> {
        if hint.stretch.is_some() {
            return Ok(ClipSound::Memory(SampleBuffer::from(Vec::new())));
        }
        (self.0)(path).map(ClipSound::Memory)
    }
}

/// Builds play plans, remembering what it built so the next compile can reuse it: each
/// pattern's event list, each decoded sample file, every processor the player runs (the
/// effects' and the plugin channels') and every audio clip's source. Lives on the UI side, next
/// to the project.
pub struct Compiler {
    /// Compiled events of each pattern, as of the last time an edit touched it.
    patterns: BTreeMap<PatternId, Arc<[NoteEvent]>>,
    /// Decoded sample files by path and the edits they play through (none for the file as it
    /// is), so each file is decoded (and each list applied) once however many channels and
    /// clips play it.
    samples: BTreeMap<SoundKey, SampleBuffer>,
    /// Instrument sets by their .sfz file's path, so each set loads once however many channels
    /// play it, and every plan shares its zones (a sounding voice goes on across plans).
    instruments: BTreeMap<PathBuf, Instrument>,
    /// Where new processors come from: native factories and plugin hosts.
    registry: Registry,
    /// Every processor the player runs or is about to, by who it plays for.
    running: BTreeMap<ProcessorOwner, Running>,
    /// Effect slots no processor holds, lowest first.
    free: BTreeSet<ProcessorSlot>,
    /// Plugin-channel slots no processor holds, lowest first.
    free_channels: BTreeSet<ProcessorSlot>,
    /// What creating plugins had to say since this was last taken.
    notices: Vec<String>,
    /// The `ProcessorOwner`s whose processors the next compile creates again.
    restart: BTreeSet<ProcessorOwner>,
    /// The source each audio clip reads from, with the file it is of.
    sources: BTreeMap<ClipId, InstalledSource>,
    /// Source slots no source holds, lowest first.
    free_sources: BTreeSet<SourceSlot>,
    /// Each automation's points, as of the last time an edit touched it.
    automation_points: BTreeMap<AutomationId, Arc<[AutomationPoint]>>,
    /// The channel live notes play, the next plan on.
    live_channel: Option<ChannelId>,
}

/// A processor as the player has it: its slot, the processor it is, and the values it was last
/// given (a native effect's; plugins hold their own).
#[derive(Debug)]
struct Running {
    slot: ProcessorSlot,
    processor: String,
    values: Vec<f32>,
    /// It stands in for a plugin that could not be created (missing, or it failed).
    stand_in: bool,
}

/// A sound file as a channel plays it: its path as the project writes it, and its edits.
type SoundKey = (PathBuf, SoundEdits);

/// An audio clip's source as the player has it: of which file, through which edits, stretched
/// how, and whether it is only for now (the edited sound still being made). A clip whose file,
/// edits or stretch changes gets a new source, and one for now is asked for again.
#[derive(Debug)]
struct InstalledSource {
    slot: SourceSlot,
    path: PathBuf,
    edits: SoundEdits,
    stretch: Option<ClipStretch>,
    for_now: bool,
}

/// A new audio clip source waiting for its slot: made before anything else changes, so a
/// refused compile has changed nothing.
struct NewSource {
    clip: ClipId,
    path: PathBuf,
    edits: SoundEdits,
    stretch: Option<ClipStretch>,
    for_now: bool,
    source: Option<Box<dyn ClipSource>>,
}

/// What an automation sets, resolved: the control target, how often it is worked out, the
/// range its normalized values span and the value the project holds.
struct Resolved {
    target: ParamTarget,
    rate: Rate,
    lo: f64,
    hi: f64,
    stored: f32,
}

impl std::fmt::Debug for Compiler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compiler")
            .field("patterns", &self.patterns.len())
            .field("samples", &self.samples.len())
            .field("instruments", &self.instruments.len())
            .field("registry", &self.registry)
            .field("running", &self.running)
            .field("sources", &self.sources)
            .finish_non_exhaustive()
    }
}

impl Default for Compiler {
    fn default() -> Self {
        Self::with_registry(Registry::native())
    }
}

impl Compiler {
    /// A compiler that creates effects from the native registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// A compiler that creates processors from `registry`.
    pub fn with_registry(registry: Registry) -> Self {
        // TABLE_SLOTS is 6010, so every slot index fits in a u16.
        let slots = |range: std::ops::Range<usize>| {
            range
                .filter_map(|slot| u16::try_from(slot).ok())
                .map(ProcessorSlot)
                .collect()
        };
        Self {
            patterns: BTreeMap::new(),
            samples: BTreeMap::new(),
            instruments: BTreeMap::new(),
            registry,
            running: BTreeMap::new(),
            free: slots(0..PROCESSOR_SLOTS),
            free_channels: slots(PROCESSOR_SLOTS..TABLE_SLOTS),
            notices: Vec::new(),
            restart: BTreeSet::new(),
            sources: BTreeMap::new(),
            // MAX_AUDIO_CLIPS is 4096, so every source slot fits in a u16.
            free_sources: (0..MAX_AUDIO_CLIPS)
                .filter_map(|slot| u16::try_from(slot).ok())
                .map(SourceSlot)
                .collect(),
            automation_points: BTreeMap::new(),
            live_channel: None,
        }
    }

    /// Live notes play `channel` (the one selected in the rack) from the next compile on; a
    /// channel the project does not have plays none.
    pub fn set_live_channel(&mut self, channel: Option<ChannelId>) {
        self.live_channel = channel;
    }

    /// Where this compiler's processors come from, for reading their parameters and reaching
    /// their plugin hosts.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// What creating plugins had to say since this was last called: one line per plugin that
    /// is missing ("Missing plugin: <name> (<id>)") or failed ("Plugin failed: <name>:
    /// <reason>").
    pub fn take_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notices)
    }

    /// Gives back slots whose processors the player has let go of: the `remove` list of every
    /// update that has come back from it (see [`crate::realtime::RealtimeLink::take_freed_slots`]).
    pub fn release(&mut self, slots: impl IntoIterator<Item = ProcessorSlot>) {
        for slot in slots {
            if slot.index() < PROCESSOR_SLOTS {
                self.free.insert(slot);
            } else {
                self.free_channels.insert(slot);
            }
        }
    }

    /// Gives back source slots the player has let go of (see
    /// [`crate::realtime::RealtimeLink::take_freed_sources`]).
    pub fn release_sources(&mut self, slots: impl IntoIterator<Item = SourceSlot>) {
        self.free_sources.extend(slots);
    }

    /// Forgets `owner`'s running processor, so the next compile creates it again (from the
    /// state the document holds then) and drops the old one. A plugin that asks to be restarted
    /// goes through here.
    pub fn recreate(&mut self, owner: ProcessorOwner) {
        self.restart.insert(owner);
    }

    /// Forgets the instrument set the project names `path`, so the next compile asks its sounds
    /// for it again: for a set to be read anew (the folders it may read from changed). Until
    /// the new one is in, its channels play silence, as a set still loading does.
    pub fn forget_instrument(&mut self, path: &Path) {
        self.instruments.remove(path);
    }

    /// Has the next compile try again every plugin that could not be created (missing, or it
    /// failed), as after a plugin scan that may have found it. Returns whether there was one.
    pub fn recreate_stand_ins(&mut self) -> bool {
        let stand_ins: Vec<ProcessorOwner> = self
            .running
            .iter()
            .filter(|(_, running)| running.stand_in)
            .map(|(&owner, _)| owner)
            .collect();
        self.restart.extend(stand_ins.iter().copied());
        !stand_ins.is_empty()
    }

    /// Compiles `project` into an update that plays `mode`, decoding every sound file into
    /// memory with `load` (given its path as written in the project). See
    /// [`compile_with`](Self::compile_with).
    pub fn compile<E>(
        &mut self,
        project: &Project,
        touched: &Touched,
        mode: PlayMode,
        load: impl FnMut(&Path) -> Result<SampleBuffer, E>,
    ) -> Result<PlanUpdate, CompileError<E>> {
        self.compile_with(project, touched, mode, &mut DecodeFn(load))
    }

    /// Compiles `project` into an update that plays `mode`.
    ///
    /// Patterns in `touched.patterns`, and patterns this compiler has not compiled before, get
    /// a new event list; every other pattern shares the previous plan's list (the same `Arc`).
    /// Channel, mixer and tempo edits never recompile a pattern: events are in ticks, they name
    /// their channel by ID, and gains and routing live in the plan's small tables. Automations
    /// share their point lists the same way, by `touched.automation`.
    ///
    /// Effects and plugin channels new to this compiler get a processor, created here on the
    /// caller's thread; those gone from the project lose theirs; effects in `touched.effects`
    /// send the values that changed, and plugins in `touched.processors` take the state the
    /// document now holds. Audio clips new to this compiler, or whose file changed, get a
    /// source from `sounds`; those gone lose theirs.
    ///
    /// `sounds` runs here, on the caller's thread, only for files and clips this compiler has
    /// no sound for yet; then it is told the project, to let go of what nothing plays any more
    /// ([`Sounds::forget_unused`]). If it fails, its error is returned and nothing the next compile
    /// depends on has changed; a mixer error leaves the processors and sources as they were too.
    ///
    /// Expects a project that passed [`Project::validate`].
    pub fn compile_with<S: Sounds>(
        &mut self,
        project: &Project,
        touched: &Touched,
        mode: PlayMode,
        sounds: &mut S,
    ) -> Result<PlanUpdate, CompileError<S::Error>> {
        // Sounds first, so a failed load returns before anything else changes.
        let voiced = voiced_files(project);
        // Edited sounds still being made: what their channels play in this plan alone.
        let mut for_now: BTreeMap<SoundKey, SampleBuffer> = BTreeMap::new();
        for &(path, edits) in &voiced {
            let key = (path.to_path_buf(), edits.clone());
            if self.samples.contains_key(&key) {
                continue;
            }
            if edits.is_empty() {
                if let Some(sample) = sounds.sample(path).map_err(CompileError::Load)? {
                    self.samples.insert(key, sample);
                }
            } else if let Some(edited) = sounds
                .edited_sample(path, edits)
                .map_err(CompileError::Load)?
            {
                if edited.for_now {
                    for_now.insert(key, edited.sound);
                } else {
                    self.samples.insert(key, edited.sound);
                }
            }
        }
        let new_sources = self.new_sources(project, sounds)?;
        let sets: BTreeSet<&Path> = project
            .channels
            .values()
            .filter(|channel| channel.kind == ChannelKind::Instrument)
            .map(|channel| channel.instrument.as_path())
            .collect();
        for path in &sets {
            if !self.instruments.contains_key(*path) {
                if let Some(instrument) = sounds.instrument(path).map_err(CompileError::Load)? {
                    self.instruments.insert(path.to_path_buf(), instrument);
                }
            }
        }
        self.instruments
            .retain(|path, _| sets.contains(&path.as_path()));
        sounds.forget_unused(project);
        let in_use: BTreeSet<(&Path, &SoundEdits)> = project
            .channels
            .values()
            .filter(|channel| channel.plays_a_file())
            .map(|channel| (channel.sample.as_path(), &channel.edits))
            .collect();
        self.samples
            .retain(|(path, edits), _| in_use.contains(&(path.as_path(), edits)));
        let order = project.mixer.schedule().map_err(MixerError::RoutingCycle)?;
        let mut delta = self.update_processors(project, touched)?;
        self.update_sources(project, new_sources, &mut delta);
        let mut channels: Vec<ChannelPlan> = project
            .channels
            .iter()
            .map(|(&id, channel)| {
                let shape = Shape::compile(&channel.sound, channel.kind, project.sample_rate);
                let sound = match channel.kind {
                    // A set still loading plays silence until the compile after it is in.
                    ChannelKind::Instrument => ChannelSound::Instrument(
                        InstrumentSound::new(
                            self.instruments
                                .get(&channel.instrument)
                                .cloned()
                                .unwrap_or_default(),
                        )
                        .with_shape(shape),
                    ),
                    // Its frames come from the player's input feed for the channel.
                    ChannelKind::Input => ChannelSound::Input,
                    ChannelKind::Sampler | ChannelKind::Audio | ChannelKind::Plugin => {
                        let sound = self.file_or_plugin_sound(id, channel, &voiced, &for_now);
                        match (sound, shape) {
                            (ChannelSound::Sampler(sample), Some(shape)) => {
                                ChannelSound::Shaped(ShapedSample { sample, shape })
                            }
                            (sound, _) => sound,
                        }
                    }
                };
                // Muted is gain 0, so muting needs no pattern recompile either. Playing nothing
                // of the document (a latency measurement) plays no input channel either: down a
                // cable looped from the output into the input, one would feed itself back
                // (found 2026-10-26). Its device's recording is filled by the device
                // callback, not by the plan.
                let muted = channel.mute
                    || (mode == PlayMode::Nothing && channel.kind == ChannelKind::Input);
                let gain = if muted { 0.0 } else { channel.volume };
                ChannelPlan {
                    id,
                    sound,
                    gain,
                    volume: channel.volume,
                    muted,
                    plays_live: channel.plays_notes(),
                    bus: channel.insert,
                    pan_gains: channel_pan_gains(channel.pan),
                    pan: channel.pan,
                    volume_automation: None,
                    pan_automation: None,
                    midi_out: channel
                        .midi_out
                        .as_ref()
                        .map(|midi_out| MidiOutPlan::new(midi_out, project.sample_rate)),
                    bend_range: channel.sound.bend_range,
                    wheel_filter: channel
                        .sound
                        .filter
                        .then_some([channel.sound.cutoff, channel.sound.resonance]),
                    cut_itself: channel.sound.cut_itself,
                }
            })
            .collect();

        self.patterns
            .retain(|id, _| project.patterns.contains_key(id) && !touched.patterns.contains(id));
        let patterns: Vec<PatternPlan> = project
            .patterns
            .iter()
            .map(|(&id, pattern)| {
                let events = self
                    .patterns
                    .entry(id)
                    .or_insert_with(|| compile_pattern(pattern, project.ppq));
                PatternPlan {
                    id,
                    length: time::bars_to_ticks(pattern.length_bars, project.ppq),
                    events: Arc::clone(events),
                }
            })
            .collect();
        let playing = match mode {
            PlayMode::Pattern(id) => patterns
                .iter()
                .position(|pattern| pattern.id == id)
                .map_or(Playing::Nothing, Playing::Pattern),
            PlayMode::Song => Playing::Song,
            PlayMode::Nothing => Playing::Nothing,
        };
        let song = compile_song(project, &patterns, &self.sources);
        let mut mixer = self.compile_mixer(project, &order);
        let automation = self.compile_automation(project, touched);
        attach_automation(&automation, &mut channels, &mut mixer);
        let automation_edges = automation_edges(&automation);
        let control_rate = automation.iter().any(|automation| {
            automation.rate == Rate::Control
                && automation.target.is_some()
                && !automation.windows.is_empty()
        });
        let plan = PlayPlan {
            sample_rate: project.sample_rate,
            ppq: project.ppq,
            channels,
            patterns,
            playing,
            song,
            mixer,
            automation,
            automation_edges,
            control_rate,
            live_channel: self
                .live_channel
                .filter(|id| project.channels.contains_key(id)),
        };
        Ok(PlanUpdate {
            plan: Box::new(plan),
            delta,
        })
    }

    /// The sound of `channel` (`id`), a sampler, an audio or a plugin channel: its decoded file
    /// through its edits (the one for now while the edited sound is made), its plugin's slot, or
    /// (for an audio channel without notes) its clips alone.
    fn file_or_plugin_sound(
        &self,
        id: ChannelId,
        channel: &crate::project::Channel,
        voiced: &BTreeSet<(&Path, &SoundEdits)>,
        for_now: &BTreeMap<SoundKey, SampleBuffer>,
    ) -> ChannelSound {
        let kind = channel.kind;
        let decoded = || {
            let key = (channel.sample.clone(), channel.edits.clone());
            self.samples
                .get(&key)
                .or_else(|| for_now.get(&key))
                .cloned()
        };
        let voiced = voiced.contains(&(channel.sample.as_path(), &channel.edits));
        match kind {
            ChannelKind::Sampler => decoded(),
            ChannelKind::Audio if voiced => decoded(),
            ChannelKind::Audio
            | ChannelKind::Plugin
            | ChannelKind::Instrument
            | ChannelKind::Input => None,
        }
        .map(ChannelSound::Sampler)
        .or_else(|| {
            let running = self.running.get(&ProcessorOwner::Channel(id))?;
            Some(ChannelSound::Plugin(running.slot))
        })
        .unwrap_or(match kind {
            ChannelKind::Audio => ChannelSound::Clips,
            // A sample still loading plays silence until the compile after it is in. (A plugin
            // channel without a slot cannot get here: the compile was refused.)
            ChannelKind::Sampler
            | ChannelKind::Plugin
            | ChannelKind::Instrument
            | ChannelKind::Input => ChannelSound::Sampler(SampleBuffer::from(Vec::new())),
        })
    }

    /// A sound for every audio clip that has none yet, or whose channel's file or edits changed,
    /// or that plays one for now: the decoded (edited) file when this compiler holds it, else
    /// what `sounds` gives. A clip playing one for now that is given one for now again keeps the
    /// one it has. Nothing is installed here, so a compile refused later has changed nothing.
    fn new_sources<S: Sounds>(
        &mut self,
        project: &Project,
        sounds: &mut S,
    ) -> Result<Vec<NewSource>, CompileError<S::Error>> {
        let mut new = Vec::new();
        for (&id, clip) in &project.playlist.clips {
            let ClipItem::Audio {
                channel, stretch, ..
            } = clip.item
            else {
                continue;
            };
            let Some(channel) = project.channels.get(&channel) else {
                continue;
            };
            let (path, edits) = (&channel.sample, &channel.edits);
            let installed = self.sources.get(&id).filter(|installed| {
                installed.path == *path && installed.edits == *edits && installed.stretch == stretch
            });
            if installed.is_some_and(|installed| !installed.for_now) {
                continue;
            }
            let key = (path.clone(), edits.clone());
            // A stretched clip's sound is its own: the decoded file is not what it plays, and
            // what it plays is not the file the samples cache keeps.
            let shared = self.samples.get(&key).filter(|_| stretch.is_none());
            let (sound, for_now) = match shared {
                Some(buffer) => (ClipSound::Memory(buffer.clone()), false),
                None if edits.is_empty() => (
                    sounds
                        .clip(path, clip_hint(project, clip))
                        .map_err(CompileError::Load)?,
                    false,
                ),
                None => {
                    let edited = sounds
                        .edited_clip(path, edits, clip_hint(project, clip))
                        .map_err(CompileError::Load)?;
                    (edited.sound, edited.for_now)
                }
            };
            // Still for now: the one the clip plays stays until the edited sound is in.
            if for_now && installed.is_some() {
                continue;
            }
            let source: Option<Box<dyn ClipSource>> = match sound {
                ClipSound::Memory(buffer) => {
                    if stretch.is_none() && !for_now {
                        self.samples.insert(key, buffer.clone());
                    }
                    Some(Box::new(MemorySource::new(buffer)))
                }
                ClipSound::Stream(source) => Some(source),
                ClipSound::Loading => None,
            };
            new.push(NewSource {
                clip: id,
                path: path.clone(),
                edits: edits.clone(),
                stretch,
                for_now,
                source,
            });
        }
        Ok(new)
    }

    /// Brings the clip sources in line with the project: sources of clips that are gone, or
    /// being replaced, leave; `new` ones take free slots. A clip with no sound yet (loading),
    /// or none left over when every slot waits to come back, plays silence until a later
    /// compile gives it one.
    fn update_sources(&mut self, project: &Project, new: Vec<NewSource>, delta: &mut RuntimeDelta) {
        let replaced: BTreeSet<ClipId> = new.iter().map(|source| source.clip).collect();
        let gone: Vec<ClipId> = self
            .sources
            .keys()
            .copied()
            .filter(|id| {
                replaced.contains(id)
                    || project
                        .playlist
                        .clips
                        .get(id)
                        .is_none_or(|clip| clip.audio_channel().is_none())
            })
            .collect();
        for id in gone {
            if let Some(installed) = self.sources.remove(&id) {
                delta.remove_sources.push(installed.slot);
            }
        }
        for NewSource {
            clip,
            path,
            edits,
            stretch,
            for_now,
            source,
        } in new
        {
            let Some(source) = source else {
                continue;
            };
            // Slots of sources leaving now are not free until the player has let them go.
            let Some(slot) = self.free_sources.pop_first() else {
                continue;
            };
            delta.install_sources.push((slot, source));
            let installed = InstalledSource {
                slot,
                path,
                edits,
                stretch,
                for_now,
            };
            self.sources.insert(clip, installed);
        }
        delta.removed_sources =
            Vec::with_capacity(delta.remove_sources.len() + delta.install_sources.len());
    }
    /// Brings `running` in line with the project's effects and plugin channels and says what
    /// the player must change. The slots are checked before anything changes, so a refusal
    /// leaves everything as it was.
    fn update_processors(
        &mut self,
        project: &Project,
        touched: &Touched,
    ) -> Result<RuntimeDelta, MixerError> {
        let effects = project
            .mixer
            .inserts
            .iter()
            .flat_map(|insert| &insert.effects)
            .map(|effect| (ProcessorOwner::Effect(effect.id), &effect.processor));
        let instruments = project.channels.iter().filter_map(|(&id, channel)| {
            let processor = channel.processor.as_ref()?;
            Some((ProcessorOwner::Channel(id), processor))
        });
        let wanted: BTreeMap<ProcessorOwner, &ProcessorRef> = effects.chain(instruments).collect();
        // A processor that is now another one (only a reloaded document can do that) gets a new
        // one, like one that is gone and one that is new; so does one asked to restart.
        let gone: Vec<ProcessorOwner> = self
            .running
            .iter()
            .filter(|(owner, running)| {
                self.restart.contains(owner)
                    || wanted
                        .get(owner)
                        .is_none_or(|reference| reference.id != running.processor)
            })
            .map(|(&owner, _)| owner)
            .collect();
        let new: Vec<(ProcessorOwner, &ProcessorRef)> = wanted
            .iter()
            .filter(|(owner, _)| !self.running.contains_key(owner) || gone.contains(owner))
            .map(|(&owner, &reference)| (owner, reference))
            .collect();
        // Slots of processors leaving now are not free until the player has let them go.
        let new_channels = new
            .iter()
            .filter(|(owner, _)| matches!(owner, ProcessorOwner::Channel(_)))
            .count();
        if new.len() - new_channels > self.free.len() {
            return Err(MixerError::SlotsFull);
        }
        if new_channels > self.free_channels.len() {
            return Err(MixerError::PluginChannelsFull);
        }
        self.restart.clear();
        let mut delta = RuntimeDelta::default();
        for owner in gone {
            if let Some(running) = self.running.remove(&owner) {
                if let Some(host) = self.registry.host(&running.processor) {
                    host.release(owner);
                }
                delta.remove.push(running.slot);
            }
        }
        let sample_rate = project.sample_rate;
        for (owner, reference) in new {
            let free = match owner {
                ProcessorOwner::Effect(_) => &mut self.free,
                ProcessorOwner::Channel(_) => &mut self.free_channels,
            };
            let Some(slot) = free.pop_first() else {
                return Err(MixerError::SlotsFull);
            };
            let (processor, values, stand_in) = self.create(owner, reference, sample_rate);
            delta.install.push((slot, processor));
            let processor = reference.id.clone();
            self.running.insert(
                owner,
                Running {
                    slot,
                    processor,
                    values,
                    stand_in,
                },
            );
        }
        let installed: BTreeSet<ProcessorSlot> =
            delta.install.iter().map(|&(slot, _)| slot).collect();
        for id in &touched.effects {
            let owner = ProcessorOwner::Effect(*id);
            let (Some(running), Some(reference)) =
                (self.running.get_mut(&owner), wanted.get(&owner))
            else {
                continue;
            };
            if installed.contains(&running.slot) {
                continue;
            }
            let Some(factory) = self.registry.get(&reference.id) else {
                continue;
            };
            let values = param_values(factory.params(), &reference.params);
            for (index, (&now, &before)) in values.iter().zip(&running.values).enumerate() {
                if now != before {
                    delta.params.push((running.slot, index, now));
                }
            }
            running.values = values;
        }
        for owner in &touched.processors {
            let (Some(running), Some(reference)) = (self.running.get(owner), wanted.get(owner))
            else {
                continue;
            };
            // A processor created just now already started from the document's state.
            if installed.contains(&running.slot) {
                continue;
            }
            if let Some(host) = self.registry.host(&reference.id) {
                host.sync_state(*owner, reference);
            }
        }
        delta.removed = Vec::with_capacity(delta.remove.len() + delta.install.len());
        Ok(delta)
    }

    /// A processor for `reference`, playing for `owner`, at `sample_rate`, the values it
    /// starts with, and whether it stands in for a plugin: a native effect from the registry,
    /// a plugin from its format's host, or one that changes nothing in place of a processor
    /// this build cannot give (an effect passes its insert through, an instrument stays
    /// silent). A plugin that is missing or fails is noted for
    /// [`take_notices`](Self::take_notices); its reference is kept as it is.
    fn create(
        &mut self,
        owner: ProcessorOwner,
        reference: &ProcessorRef,
        sample_rate: u32,
    ) -> (Box<dyn Processor>, Vec<f32>, bool) {
        if let (ProcessorOwner::Effect(_), Some(factory)) =
            (owner, self.registry.get(&reference.id))
        {
            let values = param_values(factory.params(), &reference.params);
            return (factory.create(sample_rate, &values), values, false);
        }
        let failure = match self.registry.host(&reference.id) {
            Some(host) => match host.create(owner, reference, sample_rate) {
                Ok(processor) => return (processor, Vec::new(), false),
                Err(error) => Some(error),
            },
            // A plugin id whose format this build does not host, or an instrument that names a
            // native effect, is missing; an unknown native effect passes its audio through
            // quietly, as it always has.
            None if reference.format().is_some() || matches!(owner, ProcessorOwner::Channel(_)) => {
                Some(HostError::Missing)
            }
            None => None,
        };
        let stand_in = failure.is_some();
        if let Some(error) = failure {
            self.notices.push(plugin_notice(reference, &error));
        }
        (Box::new(PassThrough), Vec::new(), stand_in)
    }

    /// The mixer in `order`, each insert's gains settled at their values (the player ramps from
    /// the previous plan's), solo and mute folded into one audible flag.
    fn compile_mixer(&self, project: &Project, order: &[InsertId]) -> MixerPlan {
        let mixer = &project.mixer;
        let solo_audible = mixer.solo_audible();
        let order: Vec<InsertPlan> = order
            .iter()
            .filter_map(|&id| {
                let insert = mixer.get(id)?;
                let audible =
                    !insert.mute && solo_audible.get(usize::from(id.0)).copied().unwrap_or(true);
                Some(InsertPlan {
                    id,
                    fader: Ramp::settled(fader_gain(insert.fader)),
                    pan: Ramp::settled(insert.pan),
                    audible,
                    effects: insert
                        .effects
                        .iter()
                        .filter_map(|effect| self.running.get(&ProcessorOwner::Effect(effect.id)))
                        .map(|running| running.slot)
                        .collect(),
                    sends: insert
                        .routes
                        .iter()
                        .map(|(&to, &amount)| SendPlan {
                            to,
                            amount: Ramp::settled(amount),
                            automation: None,
                        })
                        .collect(),
                    fader_automation: None,
                    pan_automation: None,
                })
            })
            .collect();
        let mut place = vec![0; mixer.inserts.len()];
        for (index, insert) in order.iter().enumerate() {
            if let Some(slot) = place.get_mut(usize::from(insert.id.0)) {
                *slot = index;
            }
        }
        MixerPlan {
            order,
            place,
            // `ProcessorOwner`s sort channels first, then effects by ID, so this list stays sorted
            // by ID.
            effects: self
                .running
                .iter()
                .filter_map(|(&owner, running)| match owner {
                    ProcessorOwner::Effect(id) => Some((id, running.slot)),
                    ProcessorOwner::Channel(_) => None,
                })
                .collect(),
        }
    }

    /// Every automation as playback needs it, by ID: its target resolved (inert when it is
    /// gone), its range and stored value, its clips' windows and its points (shared with the
    /// previous plan unless an edit touched them).
    fn compile_automation(&mut self, project: &Project, touched: &Touched) -> Vec<AutomationPlan> {
        let list = &project.automation.list;
        self.automation_points
            .retain(|id, _| list.contains_key(id) && !touched.automation.contains(id));
        let playlist = &project.playlist;
        list.iter()
            .map(|(&id, automation)| {
                let points = Arc::clone(
                    self.automation_points
                        .entry(id)
                        .or_insert_with(|| automation.points.as_slice().into()),
                );
                let mut windows: Vec<AutomationWindow> = playlist
                    .clips
                    .iter()
                    .filter(|(_, clip)| clip.automation() == Some(id) && project.clip_plays(clip))
                    .map(|(&clip_id, clip)| AutomationWindow {
                        start: clip.start,
                        end: clip.end(),
                        offset: clip.offset,
                        clip: clip_id,
                    })
                    .collect();
                windows.sort_by_key(|window| (window.start, window.clip));
                let mut by_end: Vec<u32> = (0..windows.len())
                    .filter_map(|index| u32::try_from(index).ok())
                    .collect();
                by_end.sort_by_key(|&index| {
                    let window = &windows[index as usize];
                    (window.end, window.start, window.clip)
                });
                let resolved = self.resolve(project, &automation.target);
                let (target, rate, lo, hi, stored) = match resolved {
                    Some(resolved) => (
                        Some(resolved.target),
                        resolved.rate,
                        resolved.lo,
                        resolved.hi,
                        resolved.stored,
                    ),
                    None => (None, Rate::Control, 0.0, 1.0, 0.0),
                };
                AutomationPlan {
                    id,
                    target,
                    rate,
                    lo,
                    hi,
                    stored,
                    windows,
                    by_end,
                    points,
                }
            })
            .collect()
    }

    /// What `target` sets now, or `None` when it is gone (its channel, insert, send or effect
    /// removed, or a parameter its processor does not have).
    fn resolve(&self, project: &Project, target: &AutomationTarget) -> Option<Resolved> {
        let audio = |target, lo, hi, stored| Resolved {
            target,
            rate: Rate::Audio,
            lo,
            hi,
            stored,
        };
        let mixer = &project.mixer;
        Some(match target {
            AutomationTarget::ChannelVolume(id) => audio(
                ParamTarget::ChannelVolume(*id),
                0.0,
                f64::from(MAX_VOLUME),
                project.channels.get(id)?.volume,
            ),
            AutomationTarget::ChannelPan(id) => audio(
                ParamTarget::ChannelPan(*id),
                -1.0,
                1.0,
                project.channels.get(id)?.pan,
            ),
            AutomationTarget::InsertFader(id) => audio(
                ParamTarget::InsertFader(*id),
                0.0,
                1.0,
                mixer.get(*id)?.fader,
            ),
            AutomationTarget::InsertPan(id) => {
                audio(ParamTarget::InsertPan(*id), -1.0, 1.0, mixer.get(*id)?.pan)
            }
            AutomationTarget::Send { from, to } => audio(
                ParamTarget::Send {
                    from: *from,
                    to: *to,
                },
                0.0,
                1.0,
                *mixer.get(*from)?.routes.get(to)?,
            ),
            AutomationTarget::EffectParam { effect, key } => {
                let (_, placed) = mixer.effect(*effect)?;
                let factory = self.registry.get(&placed.processor.id)?;
                let params = factory.params();
                let index = params.iter().position(|info| info.key == key)?;
                let info = params[index];
                let stored = param_values(params, &placed.processor.params)[index];
                Resolved {
                    target: ParamTarget::Effect {
                        effect: *effect,
                        param: u16::try_from(index).ok()?,
                    },
                    rate: Rate::Control,
                    lo: f64::from(info.min),
                    hi: f64::from(info.max),
                    stored,
                }
            }
            AutomationTarget::EffectPlugin { effect, param_id } => {
                let (_, placed) = mixer.effect(*effect)?;
                self.resolve_plugin(
                    ProcessorOwner::Effect(*effect),
                    &placed.processor,
                    *param_id,
                )?
            }
            AutomationTarget::ChannelParam { channel, param_id } => {
                let processor = project.channels.get(channel)?.processor.as_ref()?;
                self.resolve_plugin(ProcessorOwner::Channel(*channel), processor, *param_id)?
            }
        })
    }

    /// Parameter `id` of the plugin `owner` plays, by its host: its range and its value now.
    fn resolve_plugin(
        &self,
        owner: ProcessorOwner,
        reference: &ProcessorRef,
        id: u32,
    ) -> Option<Resolved> {
        let host = self.registry.host(&reference.id)?;
        let param = host
            .params(owner)
            .into_iter()
            .find(|param| param.id == id)?;
        let stored = host.value(owner, id).unwrap_or(param.default) as f32;
        Some(Resolved {
            target: ParamTarget::Plugin { owner, id },
            rate: Rate::Control,
            lo: param.min,
            hi: param.max,
            stored,
        })
    }
}

/// What the user is told about a plugin that could not be created, or that failed.
pub fn plugin_notice(reference: &ProcessorRef, error: &HostError) -> String {
    let name = reference.display_name();
    match error {
        HostError::Missing => format!("Missing plugin: {name} ({})", reference.id),
        HostError::Failed(reason) => format!("Plugin failed: {name}: {reason}"),
    }
}

/// Runs in place of a processor this build cannot give: an effect's audio passes unchanged, and
/// an instrument's buffer stays the silence it is given, its notes swallowed. The saved
/// reference is kept as it is, so the project still round-trips.
struct PassThrough;

impl Processor for PassThrough {
    fn process(&mut self, _context: &ProcessContext<'_>, _buffer: &mut [[f32; 2]]) {}

    fn set_param(&mut self, _index: usize, _value: f32) {}

    fn reset(&mut self) {}
}

/// The playlist as a timeline of the clips that play, sorted by (start, clip ID): the pattern
/// clips, each naming its pattern's compiled events by index, so no clip copies an event list,
/// and the audio clips, each with the slot of its source.
///
/// Why it is rebuilt on every compile rather than cached like the pattern lists: it is a sort
/// of the clips (n log n), small beside decoding or compiling a pattern, and a cache would need
/// its own invalidation rules for clip, track and pattern-length changes.
fn compile_song(
    project: &Project,
    patterns: &[PatternPlan],
    sources: &BTreeMap<ClipId, InstalledSource>,
) -> SongPlan {
    let playlist = &project.playlist;
    let audible: Vec<(&ClipId, &Clip)> = playlist
        .clips
        .iter()
        .filter(|(_, clip)| project.clip_plays(clip))
        .collect();
    let mut clips: Vec<ClipPlan> = audible
        .iter()
        .filter_map(|(_, clip)| {
            let index = patterns
                .binary_search_by_key(&clip.pattern()?, |pattern| pattern.id)
                .ok()?;
            let length = patterns[index].length.0.max(1);
            Some(ClipPlan {
                start: clip.start,
                end: clip.end(),
                offset: Ticks(clip.offset.0.rem_euclid(length)),
                pattern: index,
            })
        })
        .collect();
    // The clips come in ID order and the sort is stable, so equal starts stay in ID order.
    clips.sort_by_key(|clip| clip.start);
    let longest = clips
        .iter()
        .map(|clip| clip.end.0 - clip.start.0)
        .max()
        .unwrap_or(0);
    let mut audio: Vec<AudioClipPlan> = audible
        .iter()
        .filter_map(|&(&id, clip)| {
            let ClipItem::Audio {
                channel,
                frame_offset,
                gain,
                ..
            } = clip.item
            else {
                return None;
            };
            Some(AudioClipPlan {
                clip: id,
                start: clip.start,
                end: clip.end(),
                channel,
                source: sources.get(&id).map(|installed| installed.slot),
                frame_offset,
                gain,
            })
        })
        .collect();
    audio.sort_by_key(|clip| clip.start);
    let audio_longest = audio
        .iter()
        .map(|clip| clip.end.0 - clip.start.0)
        .max()
        .unwrap_or(0);
    SongPlan {
        clips,
        longest: Ticks(longest),
        end: playlist.end(),
        loop_range: playlist
            .loop_range
            .filter(|range| range.on)
            .map(|range| (range.start, range.end)),
        audio,
        audio_longest: Ticks(audio_longest),
    }
}

/// Where a new clip will first read its file: its frame offset, and the frame the loop start
/// plays when the loop starts inside the clip (at the project's tempo).
fn clip_hint(project: &Project, clip: &Clip) -> ClipHint {
    let (first_frame, stretch) = match clip.item {
        ClipItem::Audio {
            frame_offset,
            stretch,
            ..
        } => (frame_offset, stretch),
        ClipItem::Pattern { .. } | ClipItem::Automation { .. } => (0, None),
    };
    let sample = |tick| time::tick_to_sample(tick, project.sample_rate, project.tempo, project.ppq);
    let loop_frame = project
        .playlist
        .loop_range
        .filter(|range| range.on && (clip.start..clip.end()).contains(&range.start))
        .and_then(|range| Some(first_frame + sample(range.start)? - sample(clip.start)?));
    ClipHint {
        first_frame,
        loop_frame,
        stretch,
    }
}

/// The files the channels' notes play, decoded whole, each with the edits it plays through:
/// every sampler's, and an audio channel's when a pattern has notes for it (a project file
/// can hold them). An audio channel without notes is left out, so a long file of one only
/// ever streams.
fn voiced_files(project: &Project) -> BTreeSet<(&Path, &SoundEdits)> {
    let noted: BTreeSet<_> = project
        .patterns
        .values()
        .flat_map(|pattern| pattern.notes.iter().map(|note| note.channel))
        .collect();
    project
        .channels
        .iter()
        .filter(|(id, channel)| match channel.kind {
            ChannelKind::Sampler => true,
            ChannelKind::Audio => noted.contains(id),
            ChannelKind::Plugin | ChannelKind::Instrument | ChannelKind::Input => false,
        })
        .map(|(_, channel)| (channel.sample.as_path(), &channel.edits))
        .collect()
}

/// Gives each channel, insert and send the index of the automation that sets its volume, pan,
/// fader or amount, so the mixing loops read the automated values instead of the static ones.
fn attach_automation(
    automation: &[AutomationPlan],
    channels: &mut [ChannelPlan],
    mixer: &mut MixerPlan,
) {
    for (index, plan) in automation.iter().enumerate() {
        let (Some(target), Ok(index)) = (plan.target, u16::try_from(index)) else {
            continue;
        };
        match target {
            ParamTarget::ChannelVolume(id) => {
                if let Some(channel) = channel_mut(channels, id) {
                    channel.volume_automation = Some(index);
                }
            }
            ParamTarget::ChannelPan(id) => {
                if let Some(channel) = channel_mut(channels, id) {
                    channel.pan_automation = Some(index);
                }
            }
            ParamTarget::InsertFader(id) => {
                if let Some(insert) = mixer.insert_mut(id) {
                    insert.fader_automation = Some(index);
                }
            }
            ParamTarget::InsertPan(id) => {
                if let Some(insert) = mixer.insert_mut(id) {
                    insert.pan_automation = Some(index);
                }
            }
            ParamTarget::Send { from, to } => {
                if let Some(send) = mixer
                    .insert_mut(from)
                    .and_then(|insert| insert.send_mut(to))
                {
                    send.automation = Some(index);
                }
            }
            ParamTarget::Effect { .. } | ParamTarget::Plugin { .. } => {}
        }
    }
}

fn channel_mut(
    channels: &mut [ChannelPlan],
    id: crate::project::ChannelId,
) -> Option<&mut ChannelPlan> {
    let at = channels
        .binary_search_by_key(&id, |channel| channel.id)
        .ok()?;
    channels.get_mut(at)
}

/// Every tick where a live automation's window starts or ends or one of its points falls inside
/// a window, sorted, each once.
fn automation_edges(automation: &[AutomationPlan]) -> Vec<Ticks> {
    let mut edges: Vec<Ticks> = automation
        .iter()
        .filter(|automation| automation.target.is_some())
        .flat_map(|automation| {
            automation.windows.iter().flat_map(|window| {
                let inside = automation.points.iter().filter_map(move |point| {
                    let tick = window.start.0 - window.offset.0 + point.tick.0;
                    (window.start.0 < tick && tick < window.end.0).then_some(Ticks(tick))
                });
                [window.start, window.end].into_iter().chain(inside)
            })
        })
        .collect();
    edges.sort_unstable();
    edges.dedup();
    edges
}

/// A pattern's notes as events sorted by tick. Notes at or past the pattern's end are kept in
/// the document but silent, so they are left out here; so are negative ticks, which a valid
/// project never has.
fn compile_pattern(pattern: &Pattern, ppq: u16) -> Arc<[NoteEvent]> {
    let length = time::bars_to_ticks(pattern.length_bars, ppq);
    let mut events: Vec<NoteEvent> = pattern
        .notes
        .iter()
        .filter(|note| (Ticks(0)..length).contains(&note.start))
        .map(|note| NoteEvent {
            tick: note.start,
            channel: note.channel,
            velocity: note.velocity,
            speed: playback_speed(note.key),
            key: note.key,
            length: note.length,
        })
        .collect();
    // Valid notes are already sorted, but the player's binary search depends on the order, so
    // it is enforced here. The sort is stable: notes that start together keep their order,
    // which fixes the order their voices are mixed in.
    events.sort_by_key(|event| event.tick);
    events.into()
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;
    use crate::plan::ProcessorSlot;
    use crate::project::{
        Automations, Channel, ChannelId, ClipId, EffectId, MidiLinks, Mixer, Note, Playlist,
        TrackId, DEFAULT_KEY, MAX_KEY,
    };
    use crate::time::MilliBpm;

    const F1: &str = include_str!("../../../tests/fixtures/f1_four_on_floor.json");

    fn note(channel: u32, start: i64, velocity: f32) -> Note {
        Note {
            channel: ChannelId(channel),
            start: Ticks(start),
            length: Ticks(0),
            velocity,
            key: DEFAULT_KEY,
        }
    }

    /// Channels with IDs 3 (kick, volume 1.0) and 7 (snare, volume 0.5), one 2-bar pattern.
    fn project(notes: Vec<Note>) -> Project {
        let channel = |name: &str, volume| Channel {
            volume,
            ..Channel::sampler(name, format!("{name}.wav"))
        };
        Project {
            sample_rate: 44100,
            tempo: MilliBpm(120_000),
            ppq: 96,
            next_channel_id: ChannelId(8),
            channels: BTreeMap::from([
                (ChannelId(7), channel("snare", 0.5)),
                (ChannelId(3), channel("kick", 1.0)),
            ]),
            next_pattern_id: None,
            patterns: BTreeMap::from([(
                PatternId(1),
                Pattern {
                    name: "Pattern 1".into(),
                    length_bars: 2,
                    copied_from: None,
                    notes,
                },
            )]),
            playlist: Playlist::default(),
            mixer: Mixer::default(),
            automation: Automations::default(),
            midi_links: MidiLinks::default(),
        }
    }

    fn compile_all(project: &Project) -> PlayPlan {
        let silent = |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(vec![0.0]));
        *Compiler::new()
            .compile(project, &Touched::default(), PlayMode::Song, silent)
            .expect("in-memory samples load")
            .plan
    }

    #[test]
    fn patterns_become_sorted_timelines_of_channels_and_velocities() {
        let plan = compile_all(&project(vec![
            note(7, 96, 0.5),
            note(3, 0, 0.25),
            note(7, 24, 1.0),
        ]));
        // The channel table follows ID order and carries each channel's volume as its gain.
        let channels: Vec<(u32, f32)> = plan.channels.iter().map(|c| (c.id.0, c.gain)).collect();
        assert_eq!(channels, [(3, 1.0), (7, 0.5)]);
        let pattern = &plan.patterns[0];
        assert_eq!(pattern.length, Ticks(768), "2 bars of 4 beats of 96 ticks");
        let events: Vec<(i64, u32, f32)> = pattern
            .events
            .iter()
            .map(|event| (event.tick.0, event.channel.0, event.velocity))
            .collect();
        assert_eq!(events, [(0, 3, 0.25), (24, 7, 1.0), (96, 7, 0.5)]);
    }

    // 2^((key − 60)/12): key 60 is the sample's own speed, octaves are exact powers of two, and
    // a semitone is the twelfth root of two (1.0594630943592953 correctly rounded).
    #[test]
    fn a_key_sets_the_playback_speed() {
        assert_eq!(playback_speed(60), 1.0);
        assert_eq!(playback_speed(72), 2.0);
        assert_eq!(playback_speed(48), 0.5);
        assert_eq!(playback_speed(0), 1.0 / 32.0);
        assert_eq!(playback_speed(61), 1.059_463_094_359_295_3);
        assert_eq!(playback_speed(49), 1.059_463_094_359_295_3 / 2.0);
        for key in 0..MAX_KEY {
            let step = playback_speed(key + 1) / playback_speed(key);
            assert!((step - 2f64.powf(1.0 / 12.0)).abs() < 1e-12, "key {key}");
        }
        let plan = compile_all(&project(vec![
            note(3, 0, 1.0),
            Note {
                key: 72,
                ..note(3, 24, 1.0)
            },
        ]));
        let speeds: Vec<f64> = plan.patterns[0].events.iter().map(|e| e.speed()).collect();
        assert_eq!(speeds, [1.0, 2.0]);
    }

    #[test]
    fn a_muted_channel_plays_at_gain_zero() {
        let mut project = project(Vec::new());
        project.channels.get_mut(&ChannelId(3)).expect("kick").mute = true;
        let plan = compile_all(&project);
        assert_eq!(plan.channel(ChannelId(3)).map(|c| c.gain), Some(0.0));
        assert_eq!(plan.channel(ChannelId(7)).map(|c| c.gain), Some(0.5));
    }

    #[test]
    fn notes_outside_the_pattern_are_left_out() {
        // The pattern is 768 ticks long, so 768 and -1 are outside it.
        let notes = vec![note(3, -1, 1.0), note(3, 767, 1.0), note(3, 768, 1.0)];
        let plan = compile_all(&project(notes));
        let starts: Vec<i64> = plan.patterns[0].events.iter().map(|e| e.tick.0).collect();
        assert_eq!(starts, [767]);
    }

    /// F1 with `count` channels: all on F1's one file, or each on a file of its own.
    fn f1_with_channels(count: u32, distinct: bool) -> Project {
        let mut project = Project::load_json(F1).expect("F1 loads");
        let kick = project.channels[&ChannelId(1)].clone();
        for id in 2..=count {
            let mut channel = kick.clone();
            if distinct {
                channel.sample = format!("kick{id}.wav").into();
            }
            project.channels.insert(ChannelId(id), channel);
        }
        project
    }

    /// Compiles `project` with a counting loader and returns how many files it decoded.
    fn decode_calls(project: &Project) -> usize {
        let mut calls = 0;
        let counting = |_: &Path| {
            calls += 1;
            Ok::<_, Infallible>(SampleBuffer::from(vec![1.0]))
        };
        Compiler::new()
            .compile(project, &Touched::default(), PlayMode::Song, counting)
            .expect("loads");
        calls
    }

    #[test]
    fn twenty_channels_on_one_file_decode_it_once() {
        assert_eq!(decode_calls(&f1_with_channels(20, false)), 1);
    }

    #[test]
    fn each_distinct_file_is_decoded_once() {
        assert_eq!(decode_calls(&f1_with_channels(20, true)), 20);
    }

    const P4_LOOP: &str = include_str!("../../../tests/fixtures/p4_loop.json");

    /// The timeline as (start, end, offset, pattern index) rows.
    fn timeline(plan: &PlayPlan) -> Vec<(i64, i64, i64, usize)> {
        plan.song
            .clips
            .iter()
            .map(|clip| (clip.start.0, clip.end.0, clip.offset.0, clip.pattern))
            .collect()
    }

    // p4_loop: clip 1 = pattern 1 at 0, clip 2 = pattern 2 at 384, clip 3 = pattern 1 at 768,
    // 384 ticks each, loop 384..768 on. Pattern indices: pattern 1 is 0, pattern 2 is 1.
    #[test]
    fn the_song_timeline_is_sorted_by_start_then_clip_id() {
        let mut project = Project::load_json(P4_LOOP).expect("fixture loads");
        let plan = compile_all(&project);
        assert_eq!(plan.playing, Playing::Song);
        assert_eq!(
            timeline(&plan),
            [(0, 384, 0, 0), (384, 768, 0, 1), (768, 1152, 0, 0)]
        );
        assert_eq!(plan.song.end, Ticks(1152));
        assert_eq!(plan.song.longest, Ticks(384));
        assert_eq!(plan.song.loop_range, Some((Ticks(384), Ticks(768))));
        assert_eq!(plan.wrap(), Some((Ticks(768), Ticks(384))));
        // Clip 1 moved after the others sorts last; clip 4 on clip 2's tick sorts after it.
        let clips = &mut project.playlist.clips;
        clips.get_mut(&ClipId(1)).expect("clip 1").start = Ticks(1152);
        let copy = Clip {
            length: Ticks(1000),
            offset: Ticks(400),
            ..clips[&ClipId(3)].clone()
        };
        clips.insert(
            ClipId(4),
            Clip {
                start: Ticks(384),
                ..copy
            },
        );
        project.playlist.next_clip_id = ClipId(5);
        project.validate().expect("valid");
        let plan = compile_all(&project);
        // Clip 4's offset 400 is reduced below pattern 1's length (384) to 16.
        assert_eq!(
            timeline(&plan),
            [
                (384, 768, 0, 1),
                (384, 1384, 16, 0),
                (768, 1152, 0, 0),
                (1152, 1536, 0, 0)
            ]
        );
        assert_eq!(
            (plan.song.end, plan.song.longest),
            (Ticks(1536), Ticks(1000))
        );
    }

    // The song ends at the last clip end, muted or not; muted clips and clips on muted tracks are
    // not on the timeline. A loop that is off is not in the plan: the song end wraps.
    #[test]
    fn muted_clips_leave_the_timeline_but_not_the_song_length() {
        let mut project = Project::load_json(P4_LOOP).expect("fixture loads");
        let clips = &mut project.playlist.clips;
        clips.get_mut(&ClipId(3)).expect("clip 3").muted = true;
        clips.get_mut(&ClipId(2)).expect("clip 2").track = TrackId(2);
        project
            .playlist
            .tracks
            .get_mut(&TrackId(2))
            .expect("track 2")
            .mute = true;
        if let Some(range) = &mut project.playlist.loop_range {
            range.on = false;
        }
        let plan = compile_all(&project);
        assert_eq!(timeline(&plan), [(0, 384, 0, 0)]);
        assert_eq!(plan.song.end, Ticks(1152));
        assert_eq!(plan.song.loop_range, None);
        assert_eq!(plan.wrap(), Some((Ticks(1152), Ticks(0))));
        // No clips: an empty song never wraps, and pattern mode never does.
        project.playlist.clips.clear();
        assert_eq!(compile_all(&project).wrap(), None);
        let pattern = Compiler::new()
            .compile(
                &Project::load_json(P4_LOOP).expect("fixture loads"),
                &Touched::default(),
                PlayMode::Pattern(PatternId(1)),
                |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(vec![0.0])),
            )
            .expect("loads");
        assert_eq!(pattern.plan.wrap(), None);
    }

    const P5_CHAIN: &str = include_str!("../../../tests/fixtures/p5_chain.json");
    const P5_SOLO: &str = include_str!("../../../tests/fixtures/p5_solo.json");
    const P5_PAN: &str = include_str!("../../../tests/fixtures/p5_pan.json");
    const P5_DELAY_SYNC: &str = include_str!("../../../tests/fixtures/p5_delay_sync.json");

    fn load(text: &str) -> Project {
        Project::load_json(text).expect("fixture loads")
    }

    fn compile_update(compiler: &mut Compiler, project: &Project, touched: &Touched) -> PlanUpdate {
        let silent = |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(vec![0.0]));
        compiler
            .compile(project, touched, PlayMode::Song, silent)
            .expect("compiles")
    }

    // p5_chain: 3 → 2 → 1 → master, so 3, 2, 1 come first, then 4..16, the master last.
    #[test]
    fn the_mixer_plan_follows_the_routing_order() {
        let plan = compile_all(&load(P5_CHAIN));
        let order: Vec<u16> = plan.mixer.order.iter().map(|insert| insert.id.0).collect();
        let mut expected = vec![3, 2, 1];
        expected.extend(4..=16);
        expected.push(0);
        assert_eq!(order, expected);
        for (index, insert) in plan.mixer.order.iter().enumerate() {
            assert_eq!(plan.mixer.place[usize::from(insert.id.0)], index);
        }
        let three = &plan.mixer.order[0];
        let sends: Vec<(u16, f32)> = three
            .sends
            .iter()
            .map(|send| (send.to.0, send.amount.target()))
            .collect();
        assert_eq!(sends, [(2, 1.0)]);
        assert!(
            plan.mixer.order[16].sends.is_empty(),
            "the master sends nowhere"
        );
    }

    // p5_solo: solo {3}, insert 2 muted: 3, 4 (fed by 3), 5 (feeds 3) and the master are
    // heard; 1 and 2 are not (2 is muted as well).
    #[test]
    fn mute_and_solo_fold_into_one_audible_flag() {
        let plan = compile_all(&load(P5_SOLO));
        let mut heard: Vec<u16> = plan
            .mixer
            .order
            .iter()
            .filter(|insert| insert.audible)
            .map(|insert| insert.id.0)
            .collect();
        heard.sort_unstable();
        assert_eq!(heard, [0, 3, 4, 5]);
    }

    // p5_pan: channels 1 and 3 hard left and right, channel 4 into insert 1, whose pan is −1.
    #[test]
    fn channels_carry_their_insert_and_pan_gains() {
        let plan = compile_all(&load(P5_PAN));
        let root2 = std::f32::consts::SQRT_2;
        let channels: Vec<(u32, u16, [f32; 2])> = plan
            .channels
            .iter()
            .map(|c| (c.id.0, c.bus.0, c.pan_gains))
            .collect();
        assert_eq!(
            channels,
            [
                (1, 0, [root2, 0.0]),
                (2, 0, [1.0, 1.0]),
                (3, 0, [0.0, root2]),
                (4, 1, [1.0, 1.0])
            ]
        );
        let insert_1 = &plan.mixer.order[0];
        assert_eq!((insert_1.id.0, insert_1.pan.target()), (1, -1.0));
        assert_eq!(insert_1.fader.target(), 1.0, "unity at 0.8");
    }

    // An effect gets a processor once; a parameter edit sends only the value that changed; a
    // removal frees the slot only once the player has let it go.
    #[test]
    fn processors_are_created_once_retuned_and_removed_through_the_delta() {
        let mut project = load(P5_DELAY_SYNC);
        let mut compiler = Compiler::new();
        let first = compile_update(&mut compiler, &project, &Touched::default());
        let installed: Vec<ProcessorSlot> = first.delta.install.iter().map(|&(s, _)| s).collect();
        assert_eq!(installed, [ProcessorSlot(0)]);
        assert_eq!(first.plan.mixer.effects, [(EffectId(1), ProcessorSlot(0))]);
        assert_eq!(first.plan.mixer.order[0].effects, [ProcessorSlot(0)]);

        let mut edit = |project: &mut Project, edit| {
            let applied = crate::edit::apply(project, edit).expect("applies");
            compile_update(&mut compiler, project, &applied.touched)
        };
        let feedback = crate::edit::Edit::SetEffectParam {
            effect: EffectId(1),
            key: "feedback".into(),
            value: Some(0.75),
        };
        let retuned = edit(&mut project, feedback);
        assert!(retuned.delta.install.is_empty() && retuned.delta.remove.is_empty());
        // The delay's parameters: time, sync, feedback (index 2), mix.
        assert_eq!(retuned.delta.params, [(ProcessorSlot(0), 2, 0.75)]);

        let removed = edit(
            &mut project,
            crate::edit::Edit::RemoveEffect { id: EffectId(1) },
        );
        assert_eq!(removed.delta.remove, [ProcessorSlot(0)]);
        assert!(removed.delta.removed.capacity() >= 1);
        let again = crate::edit::Edit::AddEffect {
            insert: InsertId(1),
            slot: 0,
            id: EffectId(2),
            processor: ProcessorRef::new("dawsome.reverb"),
        };
        let added = edit(&mut project, again);
        let installed: Vec<ProcessorSlot> = added.delta.install.iter().map(|&(s, _)| s).collect();
        assert_eq!(installed, [ProcessorSlot(1)], "slot 0 is not back yet");
        compiler.release([ProcessorSlot(0)]);
        assert_eq!(compiler.free.first(), Some(&ProcessorSlot(0)));
    }

    // A plugin this build could not create stands in silently, and asking for the stand-ins
    // again (after a plugin scan) creates it anew in the next compile; an unknown native
    // effect passes through quietly and is no plugin to look for.
    #[test]
    fn stand_ins_for_plugins_are_created_again_when_asked() {
        let mut project = load(P5_DELAY_SYNC);
        project.mixer.inserts[1].effects[0].processor = ProcessorRef::new("vendor.missing");
        let mut compiler = Compiler::new();
        compile_update(&mut compiler, &project, &Touched::default());
        assert!(!compiler.recreate_stand_ins(), "an unknown native effect");
        project.mixer.inserts[1].effects[0].processor = ProcessorRef::new("clap:org.example.gone");
        let first = compile_update(&mut compiler, &project, &Touched::default());
        assert_eq!(first.delta.install.len(), 1);
        assert_eq!(compiler.take_notices().len(), 1, "named as missing");
        let again = compile_update(&mut compiler, &project, &Touched::default());
        assert!(again.delta.install.is_empty(), "left alone until asked");
        assert!(compiler.recreate_stand_ins());
        let retried = compile_update(&mut compiler, &project, &Touched::default());
        assert_eq!(retried.delta.install.len(), 1);
        assert_eq!(retried.delta.remove.len(), 1, "the old stand-in goes");
    }

    // Every slot in use: 500 inserts and the master with ten effects each. Swapping one effect
    // for another needs a free slot, which the removed one becomes only once released.
    #[test]
    fn a_full_slot_table_refuses_new_effects_until_a_slot_comes_back() {
        let mut project = load(P5_DELAY_SYNC);
        project.mixer.inserts = (0..=500)
            .map(|id| crate::project::Insert::numbered(InsertId(id)))
            .collect();
        let mut next = 1;
        for insert in &mut project.mixer.inserts {
            for slot in 0..10 {
                insert.effects.push(crate::project::Effect {
                    id: EffectId(next),
                    slot,
                    processor: ProcessorRef::new("vendor.missing"),
                });
                next += 1;
            }
        }
        project.mixer.next_effect_id = EffectId(next);
        project.validate().expect("valid");
        let mut compiler = Compiler::new();
        let full = compile_update(&mut compiler, &project, &Touched::default());
        assert_eq!(full.delta.install.len(), PROCESSOR_SLOTS);
        assert!(compiler.free.is_empty());
        // Effect 31 (insert 3, slot 0) replaced by a new one, `next`.
        project.mixer.inserts[3].effects[0].id = EffectId(next);
        project.mixer.next_effect_id = EffectId(next + 1);
        let silent = |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(vec![0.0]));
        let refused = compiler.compile(&project, &Touched::default(), PlayMode::Song, silent);
        assert!(matches!(
            refused,
            Err(CompileError::Mixer(MixerError::SlotsFull))
        ));
        assert_eq!(
            compiler.running.len(),
            PROCESSOR_SLOTS,
            "a refused compile changes nothing"
        );
        // Taking the old effect out frees nothing yet: its slot is the player's until the update
        // that removes it comes back.
        let slot = compiler.running[&ProcessorOwner::Effect(EffectId(31))].slot;
        let replacement = project.mixer.inserts[3].effects.remove(0);
        let removal = compile_update(&mut compiler, &project, &Touched::default());
        assert_eq!(removal.delta.remove, [slot]);
        project.mixer.inserts[3].effects.insert(0, replacement);
        let early = compiler.compile(&project, &Touched::default(), PlayMode::Song, silent);
        assert!(matches!(
            early,
            Err(CompileError::Mixer(MixerError::SlotsFull))
        ));
        compiler.release([slot]);
        let added = compile_update(&mut compiler, &project, &Touched::default());
        let installed: Vec<ProcessorSlot> = added.delta.install.iter().map(|&(s, _)| s).collect();
        assert_eq!(installed, [slot]);
    }
}
