//! The open project and its path to the audio thread: the document, its undo history, the
//! compiler, and the UI half of the realtime link.
//!
//! Every change goes the same way: apply the edit to the document, recompile what it touched
//! and send the new plan, or send the new tempo over the control ring. A control being dragged
//! (a knob, a fader, a send, an effect parameter) is the exception while it moves: each value
//! goes to the player over the control ring at once, and the plan follows once, on release. The
//! window and the `--play-to` render both drive the engine through this one type.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dawsome_engine::compile::{CompileError, Compiler, MixerError};
use dawsome_engine::edit::{self, Applied, Edit, EditError, History, StateId, Touched};
use dawsome_engine::input::InputFeeds;
use dawsome_engine::midi_out::MidiOutReader;
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::processor::Registry;
use dawsome_engine::project::{
    AutomationId, Channel, ChannelId, Clip, ClipId, ClipStretch, InsertId, PatternId,
    ProcessorOwner, Project, TrackId,
};
use dawsome_engine::realtime::{
    self, ControlRingFull, EngineEvent, MidiInbox, PlanQueueFull, Player, Position, RealtimeLink,
    RecordClock, RecordTap, TimeBaseMismatch,
};
use dawsome_engine::source::ClipSource;
use dawsome_engine::time::MilliBpm;
use dawsome_engine::transport::{Control, ParamTarget};
use dawsome_media::{InstrumentReport, MediaSounds, Playback, SampleNotice};
use thiserror::Error;

use crate::audio::Parked;
use crate::edit_names;

/// Why a change could not reach the engine. A refused edit has changed nothing. (A sound file
/// that cannot be read is not one: its channel plays silence and a notice says so.)
#[derive(Debug, Error)]
pub enum SessionError {
    #[error("edit refused: {0}")]
    Edit(#[from] EditError),
    #[error(transparent)]
    TimeBase(#[from] TimeBaseMismatch),
    #[error("audio is not responding; Play and Stop were not sent")]
    ControlRingFull(#[from] ControlRingFull),
    #[error(transparent)]
    PlanQueueFull(#[from] PlanQueueFull),
    #[error(transparent)]
    Mixer(#[from] MixerError),
    /// A change a song take's undo step could not take back past, while the take runs.
    #[error("Stop the recording first")]
    Recording,
}

/// What a running song take keeps from other edits: its pattern and its clip, and the track it
/// added when no track had room (its undo takes that track out, which it refuses for a muted
/// track or one holding clips).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TakeParts {
    pub pattern: PatternId,
    pub clip: ClipId,
    pub track: Option<TrackId>,
}

/// The id the Library's preview channel plays under in the plans: one no document gives out,
/// since a project's channel ids stay below its `next_channel_id`, which is `u32::MAX` at most.
pub const PREVIEW_CHANNEL: ChannelId = ChannelId(u32::MAX);

/// `History::undo` or `History::redo`.
type HistoryStep = fn(&mut History, &mut Project) -> Result<Option<Touched>, EditError>;

/// The project document and everything that keeps the player in step with it. Lives on the
/// UI thread; the matching [`Player`] goes to the audio callback or the offline render.
pub struct Session {
    project: Project,
    history: History,
    compiler: Compiler,
    link: RealtimeLink,
    /// What the plans play.
    mode: PlayMode,
    /// An input latency measurement runs: the plans play nothing of the document
    /// ([`PlayMode::Nothing`]) whatever `mode` says, so its clicks are what the output plays.
    measuring: bool,
    /// The channel live notes play: the one selected in the rack.
    live_channel: Option<ChannelId>,
    /// The directory the project's sample paths are relative to.
    samples_dir: PathBuf,
    /// Where the compiler's sounds come from: the files, decoded, streamed or converted.
    sounds: MediaSounds,
    /// A tempo the control ring had no room for; sent by the next [`flush`](Self::flush).
    unsent_tempo: Option<MilliBpm>,
    /// The newest live parameter value the control ring had no room for, sent like the tempo.
    unsent_param: Option<(ParamTarget, f32)>,
    /// What live parameter changes touched since the last plan; the next plan carries it.
    pending: Touched,
    /// What loading the channels' sounds had to say, since this was last taken.
    notices: Vec<SampleNotice>,
    /// What creating plugins had to say ("Missing plugin: ..."), since this was last taken.
    plugin_notices: Vec<String>,
    /// The player, when no stream plays it.
    parked: Option<Parked>,
    /// The Library's preview: a channel the plans play under [`PREVIEW_CHANNEL`] that the
    /// document, its saves, its exports and its history never hold.
    preview: Option<Channel>,
    /// The parts of the song take running, which no other edit may change, or change the IDs
    /// around, until it stops ([`refused_under_take`], [`Session::apply_live`]).
    take: Option<TakeParts>,
    /// Plans sent to the player, so a test can see a drag compile once.
    #[cfg(test)]
    plans_sent: usize,
    /// Controls sent to the player, in order, so a test can see what a gesture asked for.
    #[cfg(test)]
    controls_sent: Vec<Control>,
}

impl Session {
    /// Compiles `project` to play its playlist when `song`, else to loop its first pattern
    /// (silence without patterns), and makes the player for it, stopped at tick 0. Native
    /// effects only: the app opens its sessions with its plugin host (see
    /// [`open_playing`](Self::open_playing)).
    #[cfg(test)]
    pub fn open(
        project: Project,
        samples_dir: PathBuf,
        song: bool,
    ) -> Result<(Self, Player), SessionError> {
        let mode = Self::mode_for(&project, song);
        Self::open_in(project, samples_dir, mode)
    }

    /// What a session of `project` plays at first: the playlist when `song`, else its first
    /// pattern (silence without patterns).
    pub fn mode_for(project: &Project, song: bool) -> PlayMode {
        if song {
            PlayMode::Song
        } else {
            first_pattern_mode(project)
        }
    }

    /// As [`open`](Self::open), playing `mode`.
    #[cfg(test)]
    pub fn open_in(
        project: Project,
        samples_dir: PathBuf,
        mode: PlayMode,
    ) -> Result<(Self, Player), SessionError> {
        Self::open_with(project, samples_dir, mode, Registry::native())
    }

    /// As [`open_in`](Self::open_in), creating processors from `registry` (the native effects
    /// and any plugin hosts), for an offline render: streamed clips wait for the disk.
    #[cfg(test)]
    pub fn open_with(
        project: Project,
        samples_dir: PathBuf,
        mode: PlayMode,
        registry: Registry,
    ) -> Result<(Self, Player), SessionError> {
        Self::open_playing(
            project,
            samples_dir,
            mode,
            registry,
            Playback::offline(None),
        )
    }

    /// Compiles `project` to play `mode`, creating processors from `registry` (the native
    /// effects and any plugin hosts) and playing its files as `playback` says: live playback
    /// never waits for the disk, an offline render does. Live notes play its first channel,
    /// the one the window selects first. No folder is added to the Library.
    #[cfg(test)]
    pub fn open_playing(
        project: Project,
        samples_dir: PathBuf,
        mode: PlayMode,
        registry: Registry,
        playback: Playback,
    ) -> Result<(Self, Player), SessionError> {
        Self::open_with_library(project, samples_dir, mode, registry, playback, Vec::new())
    }

    /// Compiles `project` to play `mode`, creating processors from `registry` (the native
    /// effects and any plugin hosts) and playing its files as `playback` says: live playback
    /// never waits for the disk, an offline render does. Live notes play its first channel,
    /// the one the window selects first. `library` is the folders added to the Library from the
    /// first compile on: a set inside one reads inside it, within the bounds
    /// [`MediaSounds::set_library`] gives.
    pub fn open_with_library(
        project: Project,
        samples_dir: PathBuf,
        mode: PlayMode,
        registry: Registry,
        playback: Playback,
        library: Vec<PathBuf>,
    ) -> Result<(Self, Player), SessionError> {
        let mut compiler = Compiler::with_registry(registry);
        let live_channel = project.channels.keys().next().copied();
        compiler.set_live_channel(live_channel);
        let mut sounds = MediaSounds::new(&samples_dir, project.sample_rate, playback);
        // Nothing is read yet, so nothing is let go.
        sounds.set_library(library);
        let plan = compile(
            &mut compiler,
            &project,
            &Touched::default(),
            mode,
            &mut sounds,
        )?;
        let notices = sounds.take_notices();
        let (link, player) = realtime::link(plan, project.tempo);
        let plugin_notices = compiler.take_notices();
        let session = Self {
            project,
            history: History::new(),
            compiler,
            link,
            mode,
            measuring: false,
            live_channel,
            samples_dir,
            sounds,
            unsent_tempo: None,
            unsent_param: None,
            pending: Touched::default(),
            notices,
            plugin_notices,
            parked: None,
            preview: None,
            take: None,
            #[cfg(test)]
            plans_sent: 0,
            #[cfg(test)]
            controls_sent: Vec::new(),
        };
        Ok((session, player))
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Plays `channel` as the Library's preview, under [`PREVIEW_CHANNEL`], from the next plan
    /// on, or none: a channel of the plans alone, which live notes sent to that id play. Its
    /// plugin, when it is one, is created as a rack plugin's is, and goes when it is replaced
    /// or ended (the compile that drops one creates the next). On a failure the plans keep the
    /// preview before.
    pub fn set_preview(&mut self, channel: Option<Channel>) -> Result<(), SessionError> {
        if self.preview == channel {
            return Ok(());
        }
        let before = std::mem::replace(&mut self.preview, channel);
        let mut touched = Touched::default();
        touched.channels.insert(PREVIEW_CHANNEL);
        let result = self.publish(&touched);
        if result.is_err() {
            self.preview = before;
        }
        result
    }

    /// The Library's preview channel, while there is one.
    pub fn preview(&self) -> Option<&Channel> {
        self.preview.as_ref()
    }

    /// The directory the project's relative sample paths start at.
    pub fn samples_dir(&self) -> &Path {
        &self.samples_dir
    }

    /// What loading the channels' sounds had to say since this was last called.
    pub fn take_notices(&mut self) -> Vec<SampleNotice> {
        std::mem::take(&mut self.notices)
    }

    /// What creating plugins had to say since this was last called.
    pub fn take_plugin_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.plugin_notices)
    }

    /// Keeps this session's `player`, which no stream plays (there is no device, or its stream
    /// failed), running without one, so the processors in it stay alive.
    pub fn park(&mut self, player: Player) {
        self.parked = Some(Parked::new(player, self.project.sample_rate));
    }

    /// Whether this session's player is parked, so a stream could take it.
    pub fn is_parked(&self) -> bool {
        self.parked.is_some()
    }

    /// Takes the parked player back, as it is, for a stream to play.
    pub fn unpark(&mut self) -> Option<Player> {
        self.parked.take()?.into_player()
    }

    /// Where this session's processors come from: the native effects' parameters, and the
    /// plugin hosts the app asks about plugins.
    pub fn registry(&self) -> &Registry {
        self.compiler.registry()
    }

    /// Creates `owner`'s processor again from the state the document holds, and drops the old
    /// one: what a plugin asking to be restarted gets.
    pub fn recreate(&mut self, owner: ProcessorOwner) -> Result<(), SessionError> {
        self.compiler.recreate(owner);
        self.recompile(&Touched::default())
    }

    /// Creates again every plugin that could not be created (missing, or it failed), from the
    /// state the document holds: what a plugin scan that may have found them leads to. Returns
    /// whether there was one; those that fail again say so in the plugin notices.
    pub fn retry_plugins(&mut self) -> Result<bool, SessionError> {
        if !self.compiler.recreate_stand_ins() {
            return Ok(false);
        }
        self.recompile(&Touched::default())?;
        Ok(true)
    }

    /// Whether the document differs from its state on file.
    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    /// The document's state as the history tells states apart.
    pub fn state(&self) -> StateId {
        self.history.state()
    }

    /// The document as it is now is on file.
    pub fn mark_saved(&mut self) {
        self.history.mark_saved();
    }

    /// No state of the document is on file (recovered).
    pub fn mark_unsaved(&mut self) {
        self.history.mark_unsaved();
    }

    /// Applies `edit` as one undo step, named after it ([`edit_names::name`]), and sends what
    /// it changed to the player. An edit that changed nothing records no step: one would empty
    /// the redo stack and make a saved document dirty.
    pub fn apply(&mut self, edit: Edit) -> Result<(), SessionError> {
        let inverse = self.apply_live(edit.clone())?;
        if !edit::changed_nothing(&edit, &inverse) {
            self.history.record_as(inverse, edit_names::name(&edit));
        }
        Ok(())
    }

    /// Applies `edit` as part of the newest undo step, so one undo takes back both, and sends
    /// what it changed to the player: the files after the first of a desktop drop, which the
    /// window hands over one by one ([`History::fold_into_newest`]).
    pub fn apply_into_newest(&mut self, edit: Edit) -> Result<(), SessionError> {
        let inverse = self.apply_live(edit)?;
        self.history.fold_into_newest(inverse);
        Ok(())
    }

    /// Applies `edit` and sends it to the player without an undo step, returning the edit that
    /// undoes it. A drag calls this for every move and hands the inverse of its first move to
    /// [`record`](Self::record) on release, so the whole drag is one undo step.
    pub fn apply_live(&mut self, edit: Edit) -> Result<Edit, SessionError> {
        let Some(parts) = self.take else {
            return self.apply_take(edit);
        };
        if refused_under_take(&edit, parts) {
            return Err(SessionError::Recording);
        }
        // Whatever its kind, an edit that changes the take's own
        // pattern or clip (a rename, a move, a mute) is taken back and refused, so no step the
        // take's undo could not get past lands under it. So is one that
        // changes the track the take added (its mute, name or clips), which its undo removes.
        let take_parts = |project: &Project| {
            let track = parts.track.map(|track| {
                let clips: Vec<(ClipId, Clip)> = project
                    .playlist
                    .clips
                    .iter()
                    .filter(|(_, clip)| clip.track == track)
                    .map(|(&id, clip)| (id, clip.clone()))
                    .collect();
                (project.playlist.tracks.get(&track).cloned(), clips)
            });
            (
                project.patterns.get(&parts.pattern).cloned(),
                project.playlist.clips.get(&parts.clip).cloned(),
                track,
            )
        };
        let before = take_parts(&self.project);
        let Applied { inverse, touched } = edit::apply(&mut self.project, edit)?;
        if take_parts(&self.project) != before {
            edit::apply(&mut self.project, inverse)?;
            return Err(SessionError::Recording);
        }
        if let Err(error) = self.publish(&touched) {
            edit::apply(&mut self.project, inverse)?;
            return Err(error);
        }
        Ok(inverse)
    }

    /// As [`apply_live`](Self::apply_live), for a take's own edits, which the song take's
    /// guard lets through.
    pub fn apply_take(&mut self, edit: Edit) -> Result<Edit, SessionError> {
        let Applied { inverse, touched } = edit::apply(&mut self.project, edit)?;
        if let Err(error) = self.publish(&touched) {
            // Keep the document and the plan the player has in step: take the edit back.
            edit::apply(&mut self.project, inverse)?;
            return Err(error);
        }
        Ok(inverse)
    }

    /// While a song take into `parts` runs, refuses the edits its undo step could not take
    /// back past; `None` when it stops.
    pub fn guard_take(&mut self, parts: Option<TakeParts>) {
        self.take = parts;
    }

    /// Remembers an applied change as one undo step; see [`apply_live`](Self::apply_live). The
    /// step is named after `inverse`, which names a drag rightly: every drag's edit is of the
    /// kind its own inverse is. A change whose inverse is of another kind names itself with
    /// [`record_as`](Self::record_as).
    pub fn record(&mut self, inverse: Edit) {
        let name = edit_names::name(&inverse);
        self.history.record_as(inverse, name);
    }

    /// As [`record`](Self::record), named `name`.
    pub fn record_as(&mut self, inverse: Edit, name: &'static str) {
        self.history.record_as(inverse, name);
    }

    /// Applies `edit` to the document and sends `value` for `target` to the player over the
    /// control ring, with no new plan and no undo step, and returns the edit that undoes it. A
    /// drag calls this for every move, then [`commit_params`](Self::commit_params) and
    /// [`record`](Self::record) on release: one plan and one undo step for the whole drag.
    ///
    /// Why not a plan per move: a plan is a compile and a swap, and a drag sends dozens of
    /// values a second; the player needs only the number.
    pub fn apply_param(
        &mut self,
        edit: Edit,
        target: ParamTarget,
        value: f32,
    ) -> Result<Edit, SessionError> {
        let Applied { inverse, touched } = edit::apply(&mut self.project, edit)?;
        self.pending.merge(touched);
        self.unsent_param = Some((target, value));
        self.flush();
        Ok(inverse)
    }

    /// Sends the plan that carries what live parameter changes set since the last plan.
    pub fn commit_params(&mut self) -> Result<(), SessionError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        self.publish(&Touched::default())
    }

    /// Plugin-channel notes the player had no room for in a block, so far.
    pub fn dropped_notes(&self) -> u64 {
        self.link.dropped_notes()
    }

    /// The loudest sample `insert` put out on each side since this was last asked.
    pub fn take_peak(&self, insert: InsertId) -> [f32; 2] {
        self.link.take_peak(insert)
    }

    /// Undoes the newest edit. Returns whether there was one.
    pub fn undo(&mut self) -> Result<bool, SessionError> {
        self.walk_history(History::undo, History::redo)
    }

    /// Redoes the newest undone edit. Returns whether there was one.
    pub fn redo(&mut self) -> Result<bool, SessionError> {
        self.walk_history(History::redo, History::undo)
    }

    /// Live notes play `channel` from the next plan on (a note held already stays on its own).
    /// The same channel again compiles nothing.
    pub fn set_live_channel(&mut self, channel: Option<ChannelId>) -> Result<(), SessionError> {
        if channel == self.live_channel {
            return Ok(());
        }
        self.compiler.set_live_channel(channel);
        let previous = std::mem::replace(&mut self.live_channel, channel);
        let result = self.recompile(&Touched::default());
        if result.is_err() {
            self.compiler.set_live_channel(previous);
            self.live_channel = previous;
        }
        result
    }

    /// The inbox MIDI inputs hand this session's player their notes through.
    pub fn midi_inbox(&self) -> MidiInbox {
        self.link.midi_inbox()
    }

    /// The player's MIDI-out ring, the first time it is asked for: what the MIDI-out thread (or
    /// a render's log) reads.
    pub fn take_midi_out(&mut self) -> Option<MidiOutReader> {
        self.link.take_midi_out()
    }

    /// The channel live notes play: the selected one.
    pub fn live_channel(&self) -> Option<ChannelId> {
        self.live_channel
    }

    /// Live-note events the player could not post, so far: notes a recording lost.
    pub fn lost_live_events(&self) -> u64 {
        self.link.lost_live_events()
    }

    /// Plays `mode` from the next plan on.
    pub fn set_mode(&mut self, mode: PlayMode) -> Result<(), SessionError> {
        let previous = std::mem::replace(&mut self.mode, mode);
        let result = self.recompile(&Touched::default());
        if result.is_err() {
            self.mode = previous;
        }
        result
    }

    /// Plays nothing of the document while `on` (an input latency measurement), then what the
    /// mode says again: one plan each way. A mode set meanwhile waits for the measurement's end.
    pub fn set_measuring(&mut self, on: bool) -> Result<(), SessionError> {
        if self.measuring == on {
            return Ok(());
        }
        self.measuring = on;
        let result = self.recompile(&Touched::default());
        if result.is_err() {
            self.measuring = !on;
        }
        result
    }

    /// Suspends every processor the player runs and waits up to `wait` for the player to have
    /// done it; returns whether it did. Sent before the stream stops, so each plugin's last
    /// audio-thread call comes from the audio thread.
    pub fn suspend_processors(&mut self, wait: Duration) -> bool {
        let before = self.link.suspends();
        if self.send(Control::Suspend).is_err() {
            return false;
        }
        let deadline = Instant::now() + wait;
        while Instant::now() < deadline {
            if self.link.suspends() != before {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    }

    /// Sends a transport command, after anything still waiting for room.
    pub fn send(&mut self, control: Control) -> Result<(), SessionError> {
        self.flush();
        self.link.send(control)?;
        #[cfg(test)]
        self.controls_sent.push(control);
        Ok(())
    }

    /// Plays `source` from its start into the master, in place of any preview. The document
    /// does not change.
    pub fn play_preview(&mut self, source: Box<dyn ClipSource>) {
        // A full preview ring means four previews wait already: this one is not needed.
        let _ = self.link.preview(source);
    }

    /// Files of more frames than this stream (at the project's rate).
    pub fn stream_threshold(&self) -> u64 {
        self.sounds.stream_threshold()
    }

    /// Stops the preview.
    pub fn stop_preview(&mut self) -> Result<(), SessionError> {
        self.send(Control::StopPreview)
    }

    /// The value automation `id` set in the player's last buffer, while it governs.
    pub fn automation_value(&self, id: AutomationId) -> Option<f32> {
        let index = self
            .project
            .automation
            .list
            .keys()
            .position(|&key| key == id)?;
        let (governing, value) = self.link.automation_value(index)?;
        (governing == id).then_some(value)
    }

    /// Audio clip blocks that played silence because their stream was not ready, so far.
    pub fn underruns(&self) -> u64 {
        self.link.underruns()
    }

    /// What the player posted since this was last called (streams too late or failed), oldest
    /// first. Why take them regularly: the ring holds 256, and later ones are only counted.
    pub fn take_events(&mut self) -> Vec<EngineEvent> {
        self.link.take_events()
    }

    /// The record tap, once (give it back with [`return_record_tap`](Self::return_record_tap)).
    pub fn take_record_tap(&mut self) -> Option<RecordTap> {
        self.link.take_record_tap()
    }

    pub fn return_record_tap(&mut self, tap: RecordTap) {
        self.link.return_record_tap(tap);
    }

    /// Hands the player `feeds` in place of the input channels' feeds it reads; handed back when
    /// the player has not taken the sets waiting already.
    pub fn set_input_feeds(&mut self, feeds: InputFeeds) -> Result<(), InputFeeds> {
        self.link.set_input_feeds(feeds)
    }

    /// What an input device's callback reads the recording's start through.
    pub fn record_clock(&self) -> RecordClock {
        self.link.record_clock()
    }

    /// Whether a count-in was playing at the end of the player's last buffer.
    pub fn counting_in(&self) -> bool {
        self.link.counting_in()
    }

    /// Compiles again when files finished loading or converting in the background, so their
    /// channels and clips get their sounds. Returns whether any did.
    pub fn poll_sounds(&mut self) -> Result<bool, SessionError> {
        if self.sounds.take_loaded().is_empty() {
            return Ok(false);
        }
        self.recompile(&Touched::default())?;
        Ok(true)
    }

    /// The folders added to the Library are `folders` from now on
    /// ([`MediaSounds::set_library`]): a set already read that now reads inside another folder
    /// is read again and compiled in, its channels silent until it is.
    pub fn set_library(&mut self, folders: Vec<PathBuf>) -> Result<(), SessionError> {
        let changed = self.sounds.set_library(folders);
        if changed.is_empty() {
            return Ok(());
        }
        for path in &changed {
            self.compiler.forget_instrument(path);
        }
        self.recompile(&Touched::default())
    }

    /// Whether the file at `path` is still loading (or being converted for streaming).
    pub fn is_loading(&self, path: &Path) -> bool {
        self.sounds.is_loading(path)
    }

    /// Whether any file is still loading (or being converted for streaming).
    pub fn loading(&self) -> bool {
        self.sounds.any_loading()
    }

    /// While instrument sets load: how many of their sounds are in, of how many.
    pub fn instrument_progress(&self) -> Option<(usize, usize)> {
        self.sounds.instrument_progress()
    }

    /// What loading the instrument set the project names `path` left out, once it is loaded.
    pub fn instrument_report(&self, path: &Path) -> Option<&InstrumentReport> {
        self.sounds.report(path)
    }

    /// Whether the clip sound of `path` stretched as `stretch` is still being made.
    pub fn is_stretching(&self, path: &Path, stretch: ClipStretch) -> bool {
        self.sounds.is_stretching(path, stretch)
    }

    /// Drops the plan updates the player has finished with (their processor slots go back to
    /// the compiler) and sends what waited for room: the updates in order, then the newest
    /// tempo. Call it regularly while playing.
    pub fn flush(&mut self) {
        self.link.drain_retired();
        self.release_freed();
        if let Some(tempo) = self.unsent_tempo {
            if self.link.send(Control::SetTempo(tempo)).is_ok() {
                self.unsent_tempo = None;
            }
        }
        if let Some((target, value)) = self.unsent_param {
            if self.link.send(Control::Param(target, value)).is_ok() {
                self.unsent_param = None;
            }
        }
    }

    /// Whether the player was playing at the end of its last buffer.
    pub fn playing(&self) -> bool {
        self.link.playing()
    }

    /// The playhead at the end of the player's last buffer.
    pub fn position(&self) -> Position {
        self.link.position()
    }

    /// The undo history: what Undo and Redo would do, as the Edit menu names them.
    pub fn history(&self) -> &History {
        &self.history
    }

    /// What the player plays: a pattern, or the song.
    #[cfg(test)]
    pub fn mode(&self) -> PlayMode {
        self.mode
    }

    #[cfg(test)]
    pub fn plans_sent(&self) -> usize {
        self.plans_sent
    }

    #[cfg(test)]
    pub fn controls_sent(&self) -> &[Control] {
        &self.controls_sent
    }

    /// Whether the plans play nothing of the document (a latency measurement runs).
    #[cfg(test)]
    pub fn measuring(&self) -> bool {
        self.measuring
    }

    fn walk_history(&mut self, step: HistoryStep, back: HistoryStep) -> Result<bool, SessionError> {
        let Some(touched) = step(&mut self.history, &mut self.project)? else {
            return Ok(false);
        };
        if let Err(error) = self.publish(&touched) {
            // As in `apply_live`: the step is taken back so document and plan agree.
            back(&mut self.history, &mut self.project)?;
            return Err(error);
        }
        Ok(true)
    }

    /// Sends the player what `touched` changed: a new plan for patterns, channels, the
    /// playlist, the mixer and effects, the new tempo over the control ring (a plan carries no
    /// tempo).
    ///
    /// Any plan sent also carries what live parameter changes touched since the last one.
    fn publish(&mut self, touched: &Touched) -> Result<(), SessionError> {
        let mut touched = touched.clone();
        touched.merge(self.pending.clone());
        let plan_changed = !touched.patterns.is_empty()
            || !touched.channels.is_empty()
            || touched.playlist
            || touched.mixer
            || !touched.effects.is_empty()
            || !touched.processors.is_empty()
            || !touched.automation.is_empty();
        if plan_changed {
            self.recompile(&touched)?;
            self.pending = Touched::default();
        }
        if touched.tempo {
            // Only the newest tempo matters, so a tempo that finds the ring full waits instead
            // of failing the edit, as a live parameter value does.
            self.unsent_tempo = Some(self.project.tempo);
            self.flush();
        }
        Ok(())
    }

    fn recompile(&mut self, touched: &Touched) -> Result<(), SessionError> {
        // Before the compile, which hands out slots a refused update would never give back.
        self.link.check_room()?;
        // Slots the player has let go of are free for the effects and clips this compile adds.
        self.release_freed();
        let played = played(&self.project, self.preview.as_ref());
        let plan = compile(
            &mut self.compiler,
            &played,
            touched,
            if self.measuring {
                PlayMode::Nothing
            } else {
                self.mode
            },
            &mut self.sounds,
        )?;
        self.notices.extend(self.sounds.take_notices());
        self.plugin_notices.extend(self.compiler.take_notices());
        self.link.push_plan(plan)?;
        if let Some(parked) = &self.parked {
            parked.catch_up();
        }
        #[cfg(test)]
        {
            self.plans_sent += 1;
        }
        Ok(())
    }
}

impl Session {
    /// Hands the compiler back the processor and source slots the player has let go of.
    fn release_freed(&mut self) {
        self.compiler.release(self.link.take_freed_slots());
        self.compiler
            .release_sources(self.link.take_freed_sources());
    }
}

/// The document as the plans play it: with the Library's preview channel under
/// [`PREVIEW_CHANNEL`] while there is one (a copy, compiled and dropped), else itself.
fn played<'a>(project: &'a Project, preview: Option<&Channel>) -> Cow<'a, Project> {
    match preview {
        None => Cow::Borrowed(project),
        Some(channel) => {
            let mut played = project.clone();
            played.channels.insert(PREVIEW_CHANNEL, channel.clone());
            Cow::Owned(played)
        }
    }
}

/// Loops the lowest pattern ID; silence when there is none.
fn first_pattern_mode(project: &Project) -> PlayMode {
    project
        .patterns
        .keys()
        .next()
        .map_or(PlayMode::Song, |&id| PlayMode::Pattern(id))
}

fn compile(
    compiler: &mut Compiler,
    project: &Project,
    touched: &Touched,
    mode: PlayMode,
    sounds: &mut MediaSounds,
) -> Result<PlanUpdate, SessionError> {
    compiler
        .compile_with(project, touched, mode, sounds)
        .map_err(|error| match error {
            CompileError::Load(never) => match never {},
            CompileError::Mixer(error) => SessionError::Mixer(error),
        })
}

/// Whether `edit` would leave a song take into `pattern` and `clip` unable to be taken back in
/// one undo step: its step takes out the take's notes one by one, then its clip and pattern by
/// giving their IDs back (`UnplaceClips`, `UnaddPattern`), and a track it added the same way, so
/// while it runs no other pattern, clip or track may be added or taken out and no channel
/// deleted (its notes go from every pattern). The take's own pattern and clip are kept from any
/// change by comparing them around the edit ([`Session::apply_live`]); these kinds are refused
/// before they apply. Everything else (sounds, mixer, notes of other patterns, other clips
/// moved) goes through.
fn refused_under_take(edit: &Edit, parts: TakeParts) -> bool {
    let TakeParts { pattern, clip, .. } = parts;
    match edit {
        Edit::Transaction(edits) => edits.iter().any(|edit| refused_under_take(edit, parts)),
        Edit::AddPattern { .. }
        | Edit::UnaddPattern { .. }
        | Edit::DeletePattern { .. }
        | Edit::RestorePattern { .. }
        | Edit::PlaceClips { .. }
        | Edit::UnplaceClips { .. }
        | Edit::DuplicateClips { .. }
        | Edit::RestoreClips { .. }
        | Edit::AddAudioClip { .. }
        | Edit::UnaddAudioClip { .. }
        | Edit::AddAutomationClip { .. }
        | Edit::UnaddAutomationClip { .. }
        | Edit::RestoreAutomationClip { .. }
        | Edit::AddTrack { .. }
        | Edit::RemoveTrack { .. }
        | Edit::DeleteTrack { .. }
        | Edit::RestoreTrack { .. }
        | Edit::DeleteChannel { .. }
        | Edit::RestoreChannel { .. } => true,
        Edit::ReplaceNotes { pattern: id, .. }
        | Edit::ToggleStep { pattern: id, .. }
        | Edit::SetPatternBars { pattern: id, .. } => *id == pattern,
        Edit::DeleteClips { ids } => ids.contains(&clip),
        Edit::ResizeClip { id, .. } | Edit::SetClipPattern { id, .. } => *id == clip,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use dawsome_engine::project::{ChannelId, Clip, ClipId, Note, PatternId, TrackId, DEFAULT_KEY};
    use dawsome_engine::time::Ticks;

    use super::*;
    use crate::project_file;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    fn f1() -> (Session, Player) {
        open_f1(false)
    }

    fn open_f1(song: bool) -> (Session, Player) {
        let dir = PathBuf::from(FIXTURES);
        let opened = project_file::open(&dir.join("f1_four_on_floor.json")).expect("F1 opens");
        Session::open(opened.project, dir, song).expect("F1 compiles")
    }

    fn toggle(step: u32) -> Edit {
        Edit::ToggleStep {
            pattern: PatternId(1),
            channel: ChannelId(1),
            step,
        }
    }

    /// Plays `frames` frames from a stopped player and returns the frames that are not silent.
    fn impulses(session: &mut Session, player: &mut Player, frames: usize) -> Vec<usize> {
        session.send(Control::Play).expect("ring has room");
        let mut out = vec![[0.0f32; 2]; frames];
        player.process(&mut out);
        session.send(Control::Stop).expect("ring has room");
        player.process(&mut []);
        out.iter()
            .enumerate()
            .filter(|(_, frame)| frame[0] != 0.0)
            .map(|(index, _)| index)
            .collect()
    }

    /// `project` (its files in the fixtures) opened for live playback: files load in the
    /// background and streams never wait.
    fn open_live(project: Project, mode: PlayMode) -> (Session, Player) {
        let playback = Playback {
            stream_threshold: None,
            waiting: dawsome_media::Waiting::Never,
            convert_dir: None,
            stretch_dir: None,
            edited_dir: None,
        };
        Session::open_playing(
            project,
            PathBuf::from(FIXTURES),
            mode,
            Registry::native(),
            playback,
        )
        .expect("compiles")
    }

    /// Polls `session`'s sounds, as the media timer does, until no file loads any more.
    fn wait_for_sounds(session: &mut Session) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while session.loading() {
            assert!(Instant::now() < deadline, "the files load");
            std::thread::sleep(Duration::from_millis(2));
            session.poll_sounds().expect("compiles again");
        }
    }

    /// The channels live notes started on since the events were last taken, after the player
    /// took every plan waiting.
    fn started(session: &mut Session, player: &mut Player) -> Vec<ChannelId> {
        player.process(&mut [[0.0; 2]; 64]);
        session
            .take_events()
            .into_iter()
            .filter_map(|event| match event {
                dawsome_engine::realtime::EngineEvent::LiveNoteStarted { channel, .. } => {
                    Some(channel)
                }
                _ => None,
            })
            .collect()
    }

    // v2 addition: the Library's preview is a channel of the plans alone, under
    // PREVIEW_CHANNEL. A live note on it plays it (the plan has it) while the document, its
    // JSON and its history stay as they were; ended, the plans have it no more and a
    // note sent to it plays the selected channel, as for any channel a plan lacks. A preview
    // the same as the one playing compiles nothing.
    #[test]
    fn the_preview_plays_in_the_plans_and_never_in_the_document() {
        use dawsome_engine::project::Channel;
        let (mut session, mut player) = f1();
        let saved = |session: &Session| serde_json::to_string(session.project()).expect("JSON");
        let before = saved(&session);
        let plans = session.plans_sent();
        let preview = Channel::sampler("Snare", "impulse.wav");
        session
            .set_preview(Some(preview.clone()))
            .expect("compiles");
        assert_eq!(session.plans_sent(), plans + 1);
        session
            .set_preview(Some(preview.clone()))
            .expect("compiles");
        assert_eq!(session.plans_sent(), plans + 1, "nothing changed");
        assert_eq!(session.preview(), Some(&preview));
        assert_eq!(saved(&session), before);
        assert!(!session.project().channels.contains_key(&PREVIEW_CHANNEL));
        assert_eq!(session.history().undo_len(), 0);
        assert!(!session.is_dirty());
        let _ = started(&mut session, &mut player);
        let note = Control::ChannelNoteOn {
            channel: PREVIEW_CHANNEL,
            key: 60,
            velocity: 1.0,
        };
        session.send(note).expect("room");
        assert_eq!(started(&mut session, &mut player), [PREVIEW_CHANNEL]);
        session
            .send(Control::LiveNoteOff { key: 60 })
            .expect("room");
        session.set_preview(None).expect("compiles");
        for _ in 0..16 {
            player.process(&mut [[0.0; 2]; 64]);
        }
        let _ = session.take_events();
        session.send(note).expect("room");
        assert_eq!(
            started(&mut session, &mut player),
            [ChannelId(1)],
            "gone from the plan"
        );
        assert_eq!(saved(&session), before);
        assert_eq!(PREVIEW_CHANNEL, ChannelId(u32::MAX));
    }

    // A project naming a library's set, lib/Instruments/a.sfz playing ../Samples/a.wav, opened
    // with no folder added: its one region is left out and its note at tick 0 is silent. Once
    // lib/ is added the set is read again and compiled in, and the note plays; removed, silent
    // again.
    #[test]
    fn a_set_reads_again_when_the_folders_added_change() {
        let root = std::env::temp_dir().join(format!("dawsome-session-{}", std::process::id()));
        let lib = root.join("library");
        let _ = std::fs::remove_dir_all(&lib);
        std::fs::create_dir_all(lib.join("Instruments")).expect("dir");
        std::fs::create_dir_all(lib.join("Samples")).expect("dir");
        std::fs::copy(
            Path::new(FIXTURES).join("impulse.wav"),
            lib.join("Samples/a.wav"),
        )
        .expect("copies");
        let main = lib.join("Instruments/a.sfz");
        std::fs::write(&main, "<region> sample=../Samples/a.wav\n").expect("written");
        let text =
            std::fs::read_to_string(Path::new(FIXTURES).join("p11_pad.json")).expect("reads");
        let path = serde_json::to_string(&main).expect("a path");
        let text = text.replace("\"instruments/pad/pad.sfz\"", &path);
        let project = Project::load_json(&text).expect("loads");
        let (mut session, mut player) = Session::open_in(
            project,
            PathBuf::from(FIXTURES),
            PlayMode::Pattern(PatternId(1)),
        )
        .expect("compiles");
        let zones = |session: &Session| session.instrument_report(&main).map(|r| r.zones);
        assert_eq!(zones(&session), Some(0));
        assert!(impulses(&mut session, &mut player, 64).is_empty());
        session.set_library(vec![lib.clone()]).expect("compiles");
        assert_eq!(zones(&session), Some(1));
        player.process(&mut []);
        assert!(!impulses(&mut session, &mut player, 64).is_empty());
        session.set_library(Vec::new()).expect("compiles");
        player.process(&mut []);
        assert_eq!(zones(&session), Some(0));
        assert!(impulses(&mut session, &mut player, 64).is_empty());
        let _ = std::fs::remove_dir_all(&lib);
    }

    // Notes on an audio channel play its file. Live, that file loads in the background, never in
    // the compile. p8_trim's audio channel "saw" gets a note at tick 0 of pattern 1; the session
    // opens at once with the file loading and the note silent, and once it is in, the note plays
    // the file: p8_saw.wav's frame 0 is 1/1024.
    #[test]
    fn live_an_audio_channels_file_loads_in_the_background() {
        let text =
            std::fs::read_to_string(Path::new(FIXTURES).join("p8_trim.json")).expect("reads");
        let mut project = Project::load_json(&text).expect("loads");
        let pattern = project.patterns.get_mut(&PatternId(1)).expect("pattern 1");
        pattern.notes.push(Note {
            channel: ChannelId(1),
            start: Ticks(0),
            length: Ticks(0),
            velocity: 1.0,
            key: DEFAULT_KEY,
        });
        let (mut session, mut player) = open_live(project, PlayMode::Pattern(PatternId(1)));
        assert!(
            session.is_loading(Path::new("p8_saw.wav")),
            "loading, not decoded"
        );
        let silent = impulses(&mut session, &mut player, 512);
        assert_eq!(silent, Vec::<usize>::new(), "silent while it loads");
        wait_for_sounds(&mut session);
        session.send(Control::Play).expect("room");
        let mut out = [[0.0f32; 2]; 4];
        player.process(&mut out);
        assert_eq!(out[0], [1.0 / 1024.0; 2]);
    }

    // A sampler channel added live (a file dropped on the rack) loads its file in the background
    // too: the edit compiles at once with the file loading, and the next poll once it is in
    // plays it.
    #[test]
    fn live_a_new_sampler_channel_loads_in_the_background() {
        let dir = PathBuf::from(FIXTURES);
        let opened = project_file::open(&dir.join("f1_four_on_floor.json")).expect("F1 opens");
        let (mut session, mut player) = open_live(opened.project, PlayMode::Pattern(PatternId(1)));
        wait_for_sounds(&mut session);
        assert_eq!(
            impulses(&mut session, &mut player, 88_200),
            [0, 22_050, 44_100, 66_150]
        );
        let add = Edit::AddSamplerChannel {
            id: ChannelId(2),
            name: "imp48".into(),
            sample: "p8_imp48k.wav".into(),
        };
        session.apply(add).expect("adds");
        assert!(
            session.is_loading(Path::new("p8_imp48k.wav")),
            "loading, not decoded"
        );
        wait_for_sounds(&mut session);
        assert!(!session.is_loading(Path::new("p8_imp48k.wav")));
    }

    // F1: 44.1 kHz, 120 BPM, steps 0, 4, 8 and 12. A beat (4 steps, 96 ticks) is
    // 96·60·44100·1000 / (120000·96) = 22050 samples; step 2 (tick 48) falls at 11025.
    #[test]
    fn an_edit_reaches_the_player_and_undo_takes_it_back() {
        let (mut session, mut player) = f1();
        let original = session.project().to_json().expect("serializes");
        assert_eq!(
            impulses(&mut session, &mut player, 88_200),
            [0, 22050, 44100, 66150]
        );

        session.apply(toggle(2)).expect("step 2 toggles");
        assert_eq!(
            impulses(&mut session, &mut player, 88_200),
            [0, 11025, 22050, 44100, 66150]
        );

        assert!(session.undo().expect("undo applies"));
        assert_eq!(session.project().to_json().expect("serializes"), original);
        assert_eq!(
            impulses(&mut session, &mut player, 88_200),
            [0, 22050, 44100, 66150]
        );
        assert!(session.redo().expect("redo applies"));
        assert_eq!(
            impulses(&mut session, &mut player, 88_200),
            [0, 11025, 22050, 44100, 66150]
        );
    }

    // At 140 BPM a beat is 96·60·44100·1000 / (140000·96) = 18900 samples.
    #[test]
    fn a_tempo_edit_travels_over_the_control_ring() {
        let (mut session, mut player) = f1();
        session
            .apply(Edit::SetTempo {
                mbpm: MilliBpm(140_000),
            })
            .expect("140 BPM is in range");
        assert_eq!(
            impulses(&mut session, &mut player, 75_600),
            [0, 18900, 37800, 56700]
        );
    }

    // A control dragged live sends each value over the control ring, heard at once with no plan,
    // and one plan follows on release. F1's kick plays its click on frame 0.
    #[test]
    fn a_live_parameter_is_heard_at_once_and_compiled_once() {
        let (mut session, mut player) = f1();
        let set = |value| Edit::SetChannelVolume {
            channel: ChannelId(1),
            value,
        };
        let target = ParamTarget::ChannelVolume(ChannelId(1));
        let first = session
            .apply_param(set(2.0), target, 2.0)
            .expect("in range");
        session
            .apply_param(set(3.0), target, 3.0)
            .expect("in range");
        assert_eq!(session.plans_sent(), 0, "no plan while the control moves");
        session.send(Control::Play).expect("room");
        let mut out = vec![[0.0f32; 2]; 512];
        player.process(&mut out);
        assert_eq!(out[0], [3.0; 2], "the newest value is heard without a plan");
        session.commit_params().expect("compiles");
        session.record(first);
        assert_eq!(session.plans_sent(), 1, "one plan on release");
        session.commit_params().expect("nothing pending");
        assert_eq!(session.plans_sent(), 1, "a second commit sends nothing");
        assert!(session.undo().expect("undo applies"));
        assert_eq!(session.project().channels[&ChannelId(1)].volume, 1.0);
    }

    #[test]
    fn a_live_drag_is_one_undo_step() {
        let (mut session, _player) = f1();
        let volume = |session: &Session| session.project().channels[&ChannelId(1)].volume;
        let set = |value| Edit::SetChannelVolume {
            channel: ChannelId(1),
            value,
        };
        let first = session.apply_live(set(0.5)).expect("in range");
        session.apply_live(set(0.25)).expect("in range");
        session.record(first);
        assert_eq!(volume(&session), 0.25);
        assert!(session.undo().expect("undo applies"));
        assert_eq!(volume(&session), 1.0);
        assert!(!session.undo().expect("nothing left"));
    }

    #[test]
    fn a_refused_edit_changes_nothing() {
        let (mut session, _player) = f1();
        let before = session.project().clone();
        let result = session.apply(toggle(16));
        assert!(matches!(result, Err(SessionError::Edit(_))), "{result:?}");
        assert_eq!(session.project(), &before);
        assert!(!session.undo().expect("nothing to undo"));
    }

    // F1 has no clips, so its song is silent. Pattern 1 placed at bar 2 (ticks 384..768) plays its
    // beats from sample 88200 on, 22050 apart; undo takes it out again.
    #[test]
    fn a_placed_clip_plays_in_song_mode_and_undo_takes_it_out() {
        let (mut session, mut player) = open_f1(true);
        assert_eq!(
            impulses(&mut session, &mut player, 176_400),
            Vec::<usize>::new()
        );
        let clip = Clip::of_pattern(PatternId(1), TrackId(1), Ticks(384), Ticks(384));
        session
            .apply(Edit::PlaceClips {
                first: ClipId(1),
                clips: vec![clip],
            })
            .expect("clip places");
        assert_eq!(
            impulses(&mut session, &mut player, 176_400),
            [88_200, 110_250, 132_300, 154_350]
        );
        assert!(session.undo().expect("undo applies"));
        assert_eq!(
            impulses(&mut session, &mut player, 176_400),
            Vec::<usize>::new()
        );
    }

    // A channel whose sound is missing plays silence, the session opens, and the notice names the
    // file.
    #[test]
    fn a_missing_sample_opens_silent_with_a_notice() {
        let project = project_file::open(&PathBuf::from(FIXTURES).join("f1_four_on_floor.json"))
            .expect("F1 opens")
            .project;
        let (mut session, mut player) =
            Session::open(project, PathBuf::from("/nonexistent"), false).expect("opens");
        let notices = session.take_notices();
        assert!(
            matches!(notices.as_slice(), [SampleNotice::Unreadable { path, .. }]
                if path.ends_with("impulse.wav")),
            "{notices:?}"
        );
        assert!(session.take_notices().is_empty(), "taken once");
        assert_eq!(
            impulses(&mut session, &mut player, 88_200),
            Vec::<usize>::new()
        );
    }

    // A step switched on is its own inverse (the same toggle switches it off again), yet it is
    // a change: one undo step that empties the redo stack, as any change is.
    #[test]
    fn a_step_switched_on_is_one_undo_step_though_its_inverse_is_itself() {
        let (mut session, _player) = f1();
        session.apply(toggle(2)).expect("toggles on");
        assert!(session.undo().expect("undo applies"));
        session.apply(toggle(3)).expect("toggles on");
        let history = session.history();
        assert_eq!((history.undo_len(), history.redo_len()), (1, 0));
        assert!(session.is_dirty());
    }

    // A player that is alive but never runs (a stalled stream) takes no plans: 8 updates fill
    // the plan ring and 64 more may wait, then an edit that needs a plan is refused, says why,
    // and leaves the document and the history as they were.
    #[test]
    fn edits_are_refused_while_the_player_takes_no_plans() {
        let (mut session, _player) = f1();
        for _ in 0..72 {
            session.apply(toggle(2)).expect("queued");
        }
        let before = session.project().clone();
        let result = session.apply(toggle(2));
        assert_eq!(
            result.map_err(|error| error.to_string()),
            Err("the audio thread is not taking new plans".to_owned())
        );
        assert_eq!(session.project(), &before);
        assert_eq!(session.history().undo_len(), 72);
    }

    #[test]
    fn the_session_reports_the_history_save_marker() {
        let (mut session, _player) = f1();
        assert!(!session.is_dirty());
        session.apply(toggle(2)).expect("toggles");
        assert!(session.is_dirty());
        session.mark_saved();
        assert!(!session.is_dirty());
        session.mark_unsaved();
        assert!(session.is_dirty());
    }

    #[test]
    fn song_mode_is_silent() {
        let (mut session, mut player) = f1();
        session.set_mode(PlayMode::Song).expect("compiles");
        assert_eq!(
            impulses(&mut session, &mut player, 22_051),
            Vec::<usize>::new()
        );
        session
            .set_mode(PlayMode::Pattern(PatternId(1)))
            .expect("compiles");
        assert_eq!(impulses(&mut session, &mut player, 22_051), [0, 22050]);
    }
}
