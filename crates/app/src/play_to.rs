//! Offline renders through the app's own engine path (session, compiler, rings, player) into a
//! file: `--play-to`, with no window, and the window's export. The live stream is never involved.
//! `--play-to`'s output must equal `dawsome render` byte for byte, but for one case: a clip
//! time-stretched at a ratio of 2 or more, stretched into two separate cache folders, may differ
//! (inaudibly), because each new stretch there comes out a little different. Both keep stretches in
//! the same folder unless told otherwise, so within one cache folder they are equal.
//!
//! A `--play-to` may also record the master through the record tap, as the window's Record
//! does, and save the project with the recording's clip; or play MIDI messages timed by call
//! through the same path a device's take, recording the notes into the pattern and moving
//! linked controls; and write what the input channels' MIDI outs send (`--midi-out-log`), the
//! notes those messages play going out or not as the window's echo rules say.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use dawsome_engine::plan::PlayMode;
use dawsome_engine::processor::{ProcessorHost, Registry};
use dawsome_engine::project::{Channel, ChannelId, PatternId, Project};
use dawsome_engine::time::{self, Ticks};
use dawsome_engine::transport::Control;
use dawsome_host_clap::{ClapHost, PluginSetup, RenderPump};
use dawsome_host_common::catalogue::PluginFormat;
use dawsome_host_vst3::Vst3Host;
use dawsome_media::record::{self as recording, Feed, RecordingFile, TapFeed};
use dawsome_media::{BitDepth, Dither, Encoding, Playback, Sink, WriteError};
use serde::Deserialize;
use thiserror::Error;

use crate::links::LinkDriver;
use crate::midi::{self, ControllerQueue, Router};
use crate::notes::NoteTake;
use crate::project_file::{self, OpenError, SaveError};
use crate::record;
use crate::session::{Session, SessionError};

mod main_thread;

pub use main_thread::{MainThread, RenderHosts};

/// Frames per `Player::process` call, as the command line renders by default.
const BLOCK_FRAMES: usize = 512;

/// What `--play-to` renders: `bars` played bars of the first pattern of the project, or of its
/// playlist when `song`, into `out`, as WAV at `bits` (32-bit float when absent) or MP3 for an
/// `.mp3` path; recording the master when `record` says so, or the notes `midi_events` plays
/// when `record_notes` (quantized to the step when `quantize`), after a bar of count-in when
/// `count_in`; then writing the project to `save_to`. Stretched clips are kept in `cache_dir`,
/// without one in the user's cache folder, where `dawsome render` keeps them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayTo {
    pub out: PathBuf,
    /// At least 1.
    pub bars: u32,
    pub song: bool,
    pub bits: Option<BitDepth>,
    pub record: Option<RecordSource>,
    pub record_notes: bool,
    pub quantize: bool,
    pub count_in: bool,
    pub save_to: Option<PathBuf>,
    pub stream_threshold: Option<u64>,
    pub midi_events: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    /// Print the pattern's notes once this many calls have played (`--dump-notes-at`): what a
    /// take of notes has written by then. Read-only.
    pub dump_notes_at: Option<u64>,
    /// Where the MIDI outs' messages are written (`--midi-out-log`).
    pub midi_out_log: Option<PathBuf>,
}

/// What a recording takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordSource {
    /// What the master plays ("what you hear").
    Master,
    /// The system's default input device.
    Input,
}

/// A recording made while an offline render plays: of the master, into a new file in `dir`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineRecording {
    pub dir: PathBuf,
    pub count_in: bool,
}

/// A take of the notes an offline render's MIDI messages play, into the pattern it plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OfflineNotes {
    /// Quantize the starts to a step, the piano roll's snap as it opens.
    pub quantize: bool,
    pub count_in: bool,
}

/// MIDI messages an offline render plays, each just before the call of [`BLOCK_FRAMES`] frames
/// it is timed for, as a device's would arrive:
/// `{"buffer":512,"events":[{"call":k,"bytes":[..],"port":"<name>"}]}`, the port the message
/// comes from being optional (a port the document links to a channel plays that channel).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MidiEvents {
    buffer: usize,
    events: Vec<TimedMessage>,
}

/// One MIDI message, the call it comes before, and the port it comes from.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct TimedMessage {
    call: u64,
    bytes: Vec<u8>,
    #[serde(default)]
    port: Option<String>,
}

impl MidiEvents {
    /// The messages, each with the call it comes before, in call order (a rehearsed take hands
    /// them over by the wall clock, `--rehearse`).
    pub fn messages(&self) -> Vec<(u64, Vec<u8>)> {
        self.events
            .iter()
            .map(|event| (event.call, event.bytes.clone()))
            .collect()
    }

    /// The messages in the file at `path`, in call order (messages for one call in file order).
    pub fn read(path: &Path) -> Result<Self, PlayToError> {
        let failed = |reason: String| PlayToError::MidiEvents {
            path: path.to_path_buf(),
            reason,
        };
        let text = fs::read_to_string(path).map_err(|error| failed(error.to_string()))?;
        let mut events: Self =
            serde_json::from_str(&text).map_err(|error| failed(error.to_string()))?;
        if events.buffer != BLOCK_FRAMES {
            return Err(failed(format!(
                "its calls are {} frames long; renders call the player with {BLOCK_FRAMES}",
                events.buffer
            )));
        }
        events.events.sort_by_key(|event| event.call);
        Ok(events)
    }
}

/// One offline render of a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Render {
    pub out: PathBuf,
    /// At least 1: played bars, so loop and song-end wraps count.
    pub bars: u32,
    pub mode: PlayMode,
    pub encoding: Encoding,
}

/// Why a render failed.
#[derive(Debug, Error)]
pub enum PlayToError {
    #[error(transparent)]
    Open(#[from] OpenError),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error("refusing to write {}: it is also an input file of this project", path.display())]
    OutputIsInput { path: PathBuf },
    #[error("{bars} bars are too long for one WAV file (4 GiB at most)")]
    TooLong { bars: u32 },
    #[error("cannot write {}: {source}", path.display())]
    Write { path: PathBuf, source: WriteError },
    #[error("cannot start the render thread: {0}")]
    Thread(std::io::Error),
    #[error("cannot write the recording {}: {source}", path.display())]
    Record { path: PathBuf, source: WriteError },
    #[error("the player's recording feed is in use")]
    NoRecordTap,
    #[error(transparent)]
    Save(#[from] SaveError),
    #[error("cannot read the MIDI events {}: {reason}", path.display())]
    MidiEvents { path: PathBuf, reason: String },
    #[error(
        "MIDI events, note recording and the MIDI-out log play projects without plugins only: a \
         plugin project renders on a thread of its own, which the messages could not be timed \
         against"
    )]
    MidiWithPlugins,
    #[error("note recording plays a pattern, and this project has none")]
    NotesWithoutPattern,
}

/// [`run_with`] with no instrument window open and nothing previewed.
#[cfg(test)]
pub fn run(
    project: &Path,
    request: &PlayTo,
    plugins: Option<&PluginSetup>,
) -> Result<(), PlayToError> {
    run_with(project, request, plugins, Live::default())
}

/// What the MIDI notes no link routes play in a `--play-to`, as in the window: the instrument
/// window's channel (`--open-instrument`), or the Library's preview of a set
/// (`--library-preview`), which wins as the window's does.
#[derive(Debug, Clone, Copy, Default)]
pub struct Live<'a> {
    pub instrument_window: Option<ChannelId>,
    /// The main file of the instrument set previewed.
    pub library_preview: Option<&'a Path>,
}

/// Opens `project`, plays it from tick 0 and writes what the player renders to `request.out`,
/// finding its plugins as `plugins` says; while it plays, the instrument window is open on
/// channel `live.instrument_window` when the project has it (`--open-instrument`), so the MIDI
/// notes no link routes play that channel, as in the window; or the Library previews the set
/// `live.library_preview` names, which those notes then play.
pub fn run_with(
    project: &Path,
    request: &PlayTo,
    plugins: Option<&PluginSetup>,
    live: Live<'_>,
) -> Result<(), PlayToError> {
    let opened = project_file::open(project)?;
    let instrument_window = live
        .instrument_window
        .filter(|channel| opened.project.channels.contains_key(channel));
    // At the sound settings the set is added with (the defaults; an 808 bass cutting itself) into
    // the master, as a click on the set's Library row previews it; its path as given (read inside
    // the set's own folder).
    let library_preview = live.library_preview.map(|sfz| {
        let name = sfz
            .file_stem()
            .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
        let file = std::path::absolute(sfz).unwrap_or_else(|_| sfz.to_path_buf());
        let mut channel = Channel::instrument(name, file).as_added();
        channel.sound = crate::sets::added_sound(sfz);
        channel
    });
    let midi = match &request.midi_events {
        Some(path) => Some(MidiEvents::read(path)?),
        None => None,
    };
    // A recording of the master plays the song, as Record in pattern mode switches to it; notes
    // record into the pattern as it loops.
    let song = request.song || request.record.is_some();
    let mode = match opened.project.patterns.keys().next() {
        Some(&id) if !song => PlayMode::Pattern(id),
        _ => PlayMode::Song,
    };
    let render = Render {
        out: request.out.clone(),
        bars: request.bars,
        mode,
        // Dithered at 16 bits, as `dawsome render` writes a 16-bit file without --no-dither.
        encoding: Encoding::for_path(
            &request.out,
            request.bits.unwrap_or(BitDepth::Float32),
            Dither::Tpdf,
        ),
    };
    let recording = request.record.map(|_| OfflineRecording {
        dir: opened.samples_dir.join("samples"),
        count_in: request.count_in,
    });
    let notes = request.record_notes.then_some(OfflineNotes {
        quantize: request.quantize,
        count_in: request.count_in,
    });
    let samples_dir = opened.samples_dir.clone();
    let options = RenderOptions {
        document: Some(project),
        plugins,
        recording: recording.as_ref(),
        stream_threshold: request.stream_threshold,
        midi: midi.as_ref(),
        notes,
        cache_dir: request
            .cache_dir
            .clone()
            .or_else(dawsome_media::stretch_dir),
        library: Vec::new(),
        instrument_window,
        library_preview,
        dump_notes_at: request.dump_notes_at,
        midi_out_log: request.midi_out_log.as_deref(),
        main_thread: None,
    };
    let rendered = render_session(
        opened.project,
        opened.samples_dir,
        &render,
        &mut |_, _| true,
        &options,
    )?;
    if let Some(target) = &request.save_to {
        let json = project_file::json_for(&rendered.project, &samples_dir, target)?;
        project_file::write_atomically(target, json.as_bytes())?;
    }
    Ok(())
}

/// What a render did, beyond writing its file.
pub struct RenderOutcome {
    /// `false` when `progress` asked to stop.
    pub finished: bool,
    /// The document as the render leaves it (with the recording's clip, when it recorded).
    pub project: Project,
    /// The plugin failures it reported (on stderr too), each once.
    pub failures: Vec<String>,
}

/// What a render may do beyond playing the document into a file.
#[derive(Default)]
pub struct RenderOptions<'a> {
    /// The file the document was opened from, which the render may not overwrite.
    pub document: Option<&'a Path>,
    /// Where its plugins are found.
    pub plugins: Option<&'a PluginSetup>,
    /// Record the master while it plays.
    pub recording: Option<&'a OfflineRecording>,
    pub stream_threshold: Option<u64>,
    /// MIDI messages to play as it renders.
    pub midi: Option<&'a MidiEvents>,
    /// Record the notes they play.
    pub notes: Option<OfflineNotes>,
    /// Where stretched clips are kept; in memory only when `None`.
    pub cache_dir: Option<PathBuf>,
    /// The folders added to the Library, which the sets inside them read inside, as the window
    /// plays them (none for a scripted run, which has none).
    pub library: Vec<PathBuf>,
    /// The channel whose instrument window is open while it renders: the MIDI notes no link
    /// routes play it, as in the window.
    pub instrument_window: Option<ChannelId>,
    /// The Library's preview while it renders: a channel the plans play beside the document's,
    /// which the MIDI notes no link routes play before the instrument window's channel, as in
    /// the window. The document and the render's own notes never hold it.
    pub library_preview: Option<Channel>,
    /// Print the pattern's notes to stdout once this many calls have played: the take's
    /// pattern, else the one played (`--dump-notes-at`).
    pub dump_notes_at: Option<u64>,
    /// Write the MIDI outs' messages there, as the command line's `--midi-out-log` does: read
    /// from the player's MIDI-out ring after every call, the render ending with Stop.
    pub midi_out_log: Option<&'a Path>,
    /// The window's thread, where the plugins are made and served (the window's export on its
    /// worker, [`RenderHosts`]): the render then plays them on this thread, block by block, and
    /// `plugins` is not read. Without it, the plugins' hosts are made on this thread, which is
    /// then their main thread, and the render takes a thread of its own.
    pub main_thread: Option<&'a MainThread>,
}

/// The plugin hosts a render of a project needs: CLAP's when it names CLAP plugins, VST3's when
/// it names VST3 ones, over one catalogue (the command line's `plugin_hosts`).
#[derive(Default)]
pub struct PluginHosts {
    clap: Option<ClapHost>,
    vst3: Option<Vst3Host>,
}

impl PluginHosts {
    fn is_empty(&self) -> bool {
        self.clap.is_none() && self.vst3.is_none()
    }

    /// Each host, for the registry or the pump.
    fn boxed(&self) -> Vec<Box<dyn ProcessorHost>> {
        let mut hosts: Vec<Box<dyn ProcessorHost>> = Vec::new();
        if let Some(clap) = &self.clap {
            hosts.push(Box::new(clap.clone()));
        }
        if let Some(vst3) = &self.vst3 {
            hosts.push(Box::new(vst3.clone()));
        }
        hosts
    }
}

/// Whether `project` names a plugin of any format.
pub fn names_plugins(project: &Project) -> bool {
    !dawsome_host_clap::plugin_ids(project).is_empty()
        || !dawsome_host_vst3::plugin_ids(project).is_empty()
}

/// The hosts for `project`'s plugins, when `plugins` says where they are: their catalogue,
/// after a scan when it lacks one of them. Made on the calling thread (an export's worker
/// makes its own; a VST3 module the window's host holds is entered again, as the format's
/// entry counts).
pub fn plugin_hosts(project: &Project, plugins: Option<&PluginSetup>) -> PluginHosts {
    let Some(setup) = plugins else {
        return PluginHosts::default();
    };
    let clap = dawsome_host_clap::plugin_ids(project);
    let vst3 = dawsome_host_vst3::plugin_ids(project);
    if clap.is_empty() && vst3.is_empty() {
        return PluginHosts::default();
    }
    // A VST3 id as the catalogue writes it (upper-case hex), whatever case the project wrote.
    let canonical = |id: &String| {
        dawsome_host_vst3::parse_class_id(id)
            .map_or_else(|| id.clone(), |cid| dawsome_host_vst3::class_id_text(&cid))
    };
    let wanted = clap
        .iter()
        .map(|id| (PluginFormat::Clap, id.clone()))
        .chain(vst3.iter().map(|id| (PluginFormat::Vst3, canonical(id))))
        .collect();
    let (catalogue, scanned) = setup.catalogue_for_plugins(&wanted);
    if let Some(unsaved) = scanned.and_then(|scanned| scanned.unsaved) {
        eprintln!("dawsome-app: warning: {unsaved}");
    }
    PluginHosts {
        clap: (!clap.is_empty()).then(|| ClapHost::new(catalogue.clone())),
        vst3: (!vst3.is_empty()).then(|| Vst3Host::new(catalogue)),
    }
}

/// Renders `project` (sample paths relative to `samples_dir`; opened from `document` when it
/// came from a file) as `render` asks, calling `progress(done, total)` in frames as it goes,
/// with its plugins found as `plugins` says and its stretched clips kept in `cache_dir`, where
/// live playback finds them too: [`render_session`] with those options alone, as the export
/// renders a document with no folder added to the Library.
/// Its outcome is not finished when `progress` asked to stop; the partial file is then removed.
#[cfg(test)]
pub fn render_document(
    project: Project,
    samples_dir: PathBuf,
    document: Option<&Path>,
    render: &Render,
    progress: &mut dyn FnMut(u64, u64) -> bool,
    plugins: Option<&PluginSetup>,
    cache_dir: Option<PathBuf>,
) -> Result<RenderOutcome, PlayToError> {
    let options = RenderOptions {
        document,
        plugins,
        cache_dir,
        ..RenderOptions::default()
    };
    render_session(project, samples_dir, render, progress, &options)
}

/// Renders `project` (sample paths relative to `samples_dir`) as `render` asks, calling
/// `progress(done, total)` in frames as it goes, as `options` say: what the export (on its
/// worker) and `--play-to` run. Its outcome is not finished when `progress` asked to stop; the
/// partial file is then removed.
pub fn render_session(
    project: Project,
    samples_dir: PathBuf,
    render: &Render,
    progress: &mut dyn FnMut(u64, u64) -> bool,
    options: &RenderOptions,
) -> Result<RenderOutcome, PlayToError> {
    let (document, plugins) = (options.document, options.plugins);
    let samples = dawsome_media::input_files(&project, &samples_dir);
    let inputs: Vec<PathBuf> = document
        .map(Path::to_path_buf)
        .into_iter()
        .chain(samples)
        .collect();
    for out in std::iter::once(render.out.as_path()).chain(options.midi_out_log) {
        if let Some(path) = dawsome_media::overwritten_input(out, inputs.iter().cloned()) {
            return Err(PlayToError::OutputIsInput { path });
        }
    }
    let (sample_rate, ppq) = (project.sample_rate, project.ppq);
    let hosts = match options.main_thread {
        // Made and pumped on the window's thread.
        Some(_) => PluginHosts::default(),
        None => plugin_hosts(&project, plugins),
    };
    let plays_midi =
        options.midi.is_some() || options.notes.is_some() || options.midi_out_log.is_some();
    if plays_midi && names_plugins(&project) {
        return Err(PlayToError::MidiWithPlugins);
    }
    let mut pump = (!hosts.is_empty()).then(|| RenderPump::of(hosts.boxed(), &project));
    let mut failures = Vec::new();
    let mut report = |pump: &mut RenderPump| {
        for failure in pump.pump() {
            // One write, so a plugin printing from its own threads cannot split the line.
            let line = format!("dawsome-app: warning: {failure}\n");
            let _ = io::stderr().write_all(line.as_bytes());
            failures.push(failure);
        }
    };
    let mut registry = Registry::native();
    for host in hosts.boxed() {
        registry.add_host(host);
    }
    if let Some(main) = options.main_thread {
        for host in main.hosts() {
            registry.add_host(host);
        }
    }
    let playback =
        Playback::offline(options.stream_threshold).stretching_into(options.cache_dir.clone());
    let (mut session, mut player) = Session::open_with_library(
        project,
        samples_dir,
        render.mode,
        registry,
        playback,
        options.library.clone(),
    )?;
    let take = match (options.notes, render.mode) {
        (Some(notes), PlayMode::Pattern(pattern)) => {
            let step = time::step_to_ticks(1, ppq);
            Some(NoteTake::new(pattern, notes.quantize.then_some(step)))
        }
        (Some(_), PlayMode::Song | PlayMode::Nothing) => {
            return Err(PlayToError::NotesWithoutPattern)
        }
        (None, _) => None,
    };
    if let Some(channel) = &options.library_preview {
        session.set_preview(Some(channel.clone()))?;
        // The player takes the plan with the preview before the first call, which hears the
        // first notes, as the window's has long before a key is pressed: a call of no frames
        // swaps the plan and renders nothing.
        player.process(&mut []);
    }
    let routed = match options.library_preview {
        Some(_) => Some(crate::session::PREVIEW_CHANNEL),
        None => options.instrument_window,
    };
    let dumped = match (&take, render.mode) {
        (Some(take), _) => Some(take.pattern()),
        (None, PlayMode::Pattern(pattern)) => Some(pattern),
        (None, PlayMode::Song | PlayMode::Nothing) => None,
    };
    let dump = options.dump_notes_at.zip(dumped);
    // The log's messages, read from the MIDI-out ring after every call.
    let mut midi_out = options.midi_out_log.and_then(|_| session.take_midi_out());
    let mut sent = Vec::new();
    let mut door = plays_midi.then(|| {
        MidiDoor::new(options.midi, take, &session, sample_rate)
            .with_window(routed)
            .with_dump(dump)
    });
    for notice in session.take_notices() {
        eprintln!("dawsome-app: warning: {notice}");
    }
    for notice in session.take_plugin_notices() {
        eprintln!("dawsome-app: warning: {notice}");
    }
    let too_long = PlayToError::TooLong { bars: render.bars };
    let song_frames = dawsome_media::frames_for_bars(&player, render.bars, ppq, render.encoding)
        .ok_or(too_long)?;
    // The recording, when there is one: its file and the tap it drains, and the bar of
    // count-in the render plays first (s(384) at the tempo).
    let mut taping = match options.recording {
        Some(recording) => Some(Taping::start(&mut session, recording, sample_rate)?),
        None => None,
    };
    let counts_in = options
        .recording
        .is_some_and(|recording| recording.count_in)
        || options.notes.is_some_and(|notes| notes.count_in);
    let count_in = match counts_in {
        true => dawsome_media::frames_for_bars(&player, 1, ppq, render.encoding)
            .ok_or(PlayToError::TooLong { bars: 1 })?,
        false => 0,
    };
    match (options.recording, options.notes) {
        (Some(recording), _) => session.send(Control::Record {
            count_in: recording.count_in,
            tap_master: true,
        })?,
        (None, Some(notes)) => session.send(Control::Record {
            count_in: notes.count_in,
            tap_master: false,
        })?,
        (None, None) => session.send(Control::Play)?,
    }
    let frames = song_frames
        .checked_add(count_in)
        .filter(|&frames| frames <= render.encoding.max_frames())
        .ok_or(PlayToError::TooLong { bars: render.bars })?;
    let write_error = |source| PlayToError::Write {
        path: render.out.clone(),
        source,
    };
    let mut sink = Sink::create(&render.out, sample_rate, render.encoding).map_err(write_error)?;
    // Why the tap is drained after every block, on the thread that renders: an offline render
    // outruns any thread draining it on a timer, and the tap must never drop a frame of a
    // recording made this way.
    let mut drain = || {
        if let Some(taping) = &mut taping {
            taping.drain();
        }
    };
    if let Some(door) = &mut door {
        door.before_call(&mut session, 0)?;
    }
    // The first failure of the MIDI path, which stops the render.
    let mut door_failed = None;
    // Why two loops: plugins whose hosts were made here need their render on a thread of its
    // own while this one serves their main-thread work; without them (no plugins, or plugins
    // the window's thread serves) the render stays here, block by block, so an export's progress
    // and Cancel act on exact blocks.
    let (finished, sink) = match &mut pump {
        Some(pump) => {
            let rendered = dawsome_media::render_on_thread(
                player,
                frames,
                BLOCK_FRAMES,
                sink,
                &mut drain,
                &mut || report(pump),
                &mut |done| progress(done, frames),
            )
            .map_err(PlayToError::Thread)?;
            // The processors drop here, on the plugins' main thread; their instances go with the
            // host.
            drop(rendered.player);
            report(pump);
            (rendered.result, rendered.sink)
        }
        None => {
            let finished =
                dawsome_media::render(&mut player, frames, BLOCK_FRAMES, &mut sink, &mut |done| {
                    drain();
                    if let Some(reader) = &mut midi_out {
                        reader.read_into(&mut sent);
                    }
                    if let Some(door) = &mut door {
                        // `BLOCK_FRAMES` fits a u64, and every call but the last is that long.
                        let call = done / BLOCK_FRAMES as u64;
                        let stepped = door.after_call(&mut session, done).and_then(|()| {
                            door.dump(&session, call);
                            door.before_call(&mut session, call)
                        });
                        if let Err(error) = stepped {
                            door_failed = Some(error);
                            return false;
                        }
                    }
                    progress(done, frames)
                });
            if midi_out.is_some() {
                // A logged render ends as playback does: Stop, taken by one more call, ends the
                // notes still sounding on their MIDI outs, on the render's last frame.
                session.send(Control::Stop)?;
                player.process(&mut []);
            }
            if options.main_thread.is_some() {
                // The plugins' audio halves stop on the thread that played them; their instances
                // go on the window's thread once the player is dropped.
                player.suspend_processors();
            }
            (finished, sink)
        }
    };
    if let Some(error) = door_failed {
        drop(sink);
        let _ = fs::remove_file(&render.out);
        return Err(error.into());
    }
    let recorded = taping.map(|mut taping| {
        taping.drain();
        taping.finish()
    });
    let outcome = match finished {
        Ok(true) => {
            sink.finish().map_err(write_error)?;
            true
        }
        Ok(false) => {
            drop(sink);
            let _ = fs::remove_file(&render.out);
            false
        }
        Err(error) => {
            drop(sink);
            let _ = fs::remove_file(&render.out);
            return Err(write_error(error));
        }
    };
    if let Some(door) = door {
        door.finish(&mut session)?;
    }
    if let (Some(path), Some(reader)) = (options.midi_out_log, midi_out.as_mut()) {
        reader.read_into(&mut sent);
        project_file::write_atomically(path, reader.log(&sent).as_bytes())?;
    }
    if let Some(recorded) = recorded {
        let (sample, frames) = recorded?;
        // Play started on tick 0, so the clip does; the recording plays from where it began.
        let relative = sample
            .strip_prefix(session.samples_dir())
            .map_or_else(|_| sample.clone(), Path::to_path_buf);
        if let Some(edit) =
            record::new_audio_clip(session.project(), relative, Ticks(0), frames, None)
        {
            session.apply(edit)?;
        }
    }
    let project = session.project().clone();
    drop(session);
    Ok(RenderOutcome {
        finished: outcome,
        project,
        failures,
    })
}

/// A recording being written as an offline render plays: the record tap drained into its file
/// after every block.
struct Taping {
    feed: TapFeed,
    file: RecordingFile,
    path: PathBuf,
    buffer: Vec<f32>,
    padded: u64,
    /// The first write that failed; the rest of the recording is skipped.
    error: Option<WriteError>,
}

impl Taping {
    fn start(
        session: &mut Session,
        recording: &OfflineRecording,
        rate: u32,
    ) -> Result<Self, PlayToError> {
        let tap = session.take_record_tap().ok_or(PlayToError::NoRecordTap)?;
        let path = record::next_recording_path(&recording.dir);
        let file =
            RecordingFile::create(&path, 2, rate, rate).map_err(|source| PlayToError::Record {
                path: path.clone(),
                source,
            })?;
        let feed = TapFeed { tap, rate };
        let padded = feed.dropped();
        Ok(Self {
            feed,
            file,
            path,
            buffer: Vec::new(),
            padded,
            error: None,
        })
    }

    fn drain(&mut self) {
        if self.error.is_none() {
            let drained = recording::drain(
                &mut self.feed,
                &mut self.file,
                &mut self.buffer,
                &mut self.padded,
            );
            self.error = drained.err();
        }
    }

    /// Completes the file: its path and its length in frames.
    fn finish(self) -> Result<(PathBuf, u64), PlayToError> {
        let failed = |source| PlayToError::Record {
            path: self.path.clone(),
            source,
        };
        if let Some(error) = self.error {
            return Err(failed(error));
        }
        let frames = self.file.finish().map_err(failed)?;
        Ok((self.path, frames))
    }
}

/// The MIDI path of an offline render: before each call, the messages timed for it go through
/// the parse a device's messages go through, notes to the player's inbox and controller moves
/// to the queue the window drains; the moves then set their linked parameters. After each
/// call, what the player reported goes to the take of notes, and bursts of moves whose
/// controllers have been still for 300 ms of rendered audio become undo steps.
struct MidiDoor<'a> {
    events: &'a [TimedMessage],
    /// The first message not yet played.
    next: usize,
    router: Router,
    /// Each port a message names, with its own router, as each connected port has one: its
    /// note-offs end only its own notes.
    ports: BTreeMap<String, Router>,
    controllers: ControllerQueue,
    links: LinkDriver,
    take: Option<NoteTake>,
    rate: u32,
    /// The channel whose instrument window is open, which the notes no link routes play.
    window: Option<ChannelId>,
    /// The call after which a pattern's notes are printed, and the pattern.
    dump: Option<(u64, PatternId)>,
}

impl<'a> MidiDoor<'a> {
    fn new(
        events: Option<&'a MidiEvents>,
        take: Option<NoteTake>,
        session: &Session,
        rate: u32,
    ) -> Self {
        let controllers = ControllerQueue::default();
        Self {
            events: events.map_or(&[], |events| &events.events),
            next: 0,
            router: Router::new(session.midi_inbox(), controllers.clone()),
            ports: BTreeMap::new(),
            controllers,
            links: LinkDriver::default(),
            take,
            rate,
            window: None,
            dump: None,
        }
    }

    /// The door printing `dump`'s pattern's notes once its call has played.
    fn with_dump(self, dump: Option<(u64, PatternId)>) -> Self {
        Self { dump, ..self }
    }

    /// Once `call` calls have played and what they reported was heard: the dumped pattern's
    /// notes on stdout as one line, `{"call":N,"notes":[...]}` (notes as the project file
    /// writes them).
    fn dump(&self, session: &Session, call: u64) {
        let Some((at, pattern)) = self.dump.filter(|(at, _)| *at == call) else {
            return;
        };
        let notes = session
            .project()
            .patterns
            .get(&pattern)
            .map(|held| held.notes.as_slice())
            .unwrap_or_default();
        println!("{}", serde_json::json!({ "call": at, "notes": notes }));
    }

    /// The door with the instrument window open on `window`: a message naming no port comes
    /// from no linked port, so it plays the window's channel too.
    fn with_window(self, window: Option<ChannelId>) -> Self {
        self.router.route(window);
        Self { window, ..self }
    }

    /// Plays the messages timed for call `call` (and any still waiting from before it), then
    /// moves the parameters their controllers are linked to. The first failure is reported
    /// once every move was tried.
    fn before_call(&mut self, session: &mut Session, call: u64) -> Result<(), SessionError> {
        while let Some(message) = self.events.get(self.next).filter(|m| m.call <= call) {
            match &message.port {
                // Routed as the window routes a connected port: by the document's links, else
                // to the instrument window's channel.
                Some(port) => {
                    let router = self
                        .ports
                        .entry(port.clone())
                        .or_insert_with(|| self.router.for_port(false));
                    let project = session.project();
                    let channel = midi::port_channel(project, port, self.window);
                    let plays = channel.or(session.live_channel());
                    router.route(channel);
                    router.echo(midi::port_echo(project, port, plays));
                    router.guard(midi::has_midi_out(project, plays));
                    router.links(midi::linked_controls(project));
                    router.deliver(&message.bytes);
                }
                None => {
                    let project = session.project();
                    let plays = self.window.or(session.live_channel());
                    self.router.guard(midi::has_midi_out(project, plays));
                    self.router.links(midi::linked_controls(project));
                    self.router.deliver(&message.bytes);
                }
            }
            self.next += 1;
        }
        let now = call.saturating_mul(BLOCK_FRAMES as u64);
        let mut result = Ok(());
        while let Some(moved) = self.controllers.pop() {
            let applied = self.links.moved(session, moved, now);
            if result.is_ok() {
                result = applied;
            }
        }
        result
    }

    /// After the call that brought the render to `done` frames: the notes the player reported
    /// go into the take, and the bursts that are still become undo steps.
    fn after_call(&mut self, session: &mut Session, done: u64) -> Result<(), SessionError> {
        for event in session.take_events() {
            if let Some(take) = &mut self.take {
                take.hear(session, &event)?;
            }
        }
        // No plugin plays here (they are refused), so no state waits to be captured.
        self.links.settle(session, done, self.rate).map(drop)
    }

    /// The render is over: the take stops where the playhead is, and every burst ends.
    fn finish(mut self, session: &mut Session) -> Result<(), SessionError> {
        if let Some(take) = self.take.take() {
            take.finish(session, session.position().ticks)?;
        }
        self.links.settle_all(session).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    /// Plays `bars` bars of `fixture` in pattern mode, call by call as a render does, with the
    /// messages of `events` through the MIDI door and, when `take` is given, their notes
    /// recorded; returns the session it leaves.
    fn through_the_door(
        fixture: &str,
        events: &str,
        bars: u32,
        take: Option<NoteTake>,
    ) -> (Session, Project) {
        let dir = Path::new(FIXTURES);
        let opened = project_file::open(&dir.join(fixture)).expect("opens");
        let before = opened.project.clone();
        let events = MidiEvents::read(&dir.join(events)).expect("events");
        let mode = PlayMode::Pattern(dawsome_engine::project::PatternId(1));
        let (mut session, mut player) =
            Session::open_in(opened.project, opened.samples_dir, mode).expect("compiles");
        let mut door = MidiDoor::new(Some(&events), take, &session, 44_100);
        let control = if door.take.is_some() {
            Control::Record {
                count_in: false,
                tap_master: false,
            }
        } else {
            Control::Play
        };
        session.send(control).expect("room");
        // A bar at 120 BPM and 44.1 kHz is 88200 frames.
        let frames = u64::from(bars) * 88_200;
        let mut out = vec![[0.0f32; 2]; BLOCK_FRAMES];
        door.before_call(&mut session, 0).expect("plays");
        let mut done = 0;
        while done < frames {
            let len = (frames - done).min(BLOCK_FRAMES as u64) as usize;
            player.process(&mut out[..len]);
            done += len as u64;
            door.after_call(&mut session, done).expect("hears");
            door.before_call(&mut session, done / BLOCK_FRAMES as u64)
                .expect("plays");
        }
        door.finish(&mut session).expect("finishes");
        (session, before)
    }

    // Known answer (4) from p9_expected.json: one undo step per burst, two in all (the fader's
    // move at call 10, the pan's at calls 14 and 16; the move at call 12 is on another MIDI
    // channel than the pan's link), and undoing both gives the document back.
    #[test]
    fn p9_cc_through_the_door_is_two_undo_steps() {
        let (mut session, before) = through_the_door("p9_cc.json", "p9_cc_events.json", 1, None);
        let project = session.project();
        let fader = project
            .mixer
            .get(dawsome_engine::project::InsertId(1))
            .expect("1");
        assert_eq!(fader.fader, 0.787_401_56);
        let pan = project.channels[&dawsome_engine::project::ChannelId(1)].pan;
        assert_eq!(pan, 0.007_874_016);
        assert_eq!(session.history().undo_len(), 2);
        assert!(session.undo().expect("undoes") && session.undo().expect("undoes"));
        assert_eq!(session.project(), &before);
    }

    // Known answer (3): the take of p9_record's events is one undo step, which takes all three
    // notes back out.
    #[test]
    fn p9_record_through_the_door_is_one_undo_step() {
        let take = NoteTake::new(dawsome_engine::project::PatternId(1), Some(Ticks(24)));
        let (mut session, before) =
            through_the_door("p9_record.json", "p9_record_events.json", 2, Some(take));
        let pattern = &session.project().patterns[&dawsome_engine::project::PatternId(1)];
        assert_eq!(pattern.notes.len(), 3);
        assert_eq!(session.history().undo_len(), 1);
        assert!(session.undo().expect("undoes"));
        assert_eq!(session.project(), &before);
    }

    #[test]
    fn midi_events_timed_for_another_buffer_are_refused() {
        let dir = std::env::temp_dir().join(format!("dawsome-midi-events-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("events.json");
        fs::write(&path, r#"{"buffer":256,"events":[]}"#).expect("writes");
        assert!(matches!(
            MidiEvents::read(&path),
            Err(PlayToError::MidiEvents { .. })
        ));
        fs::write(
            &path,
            r#"{"buffer":512,"events":[{"call":1,"bytes":[300]}]}"#,
        )
        .expect("writes");
        assert!(matches!(
            MidiEvents::read(&path),
            Err(PlayToError::MidiEvents { .. })
        ));
        fs::remove_dir_all(&dir).expect("removed");
    }

    // An export's Cancel: the render stops at the next block and its partial file goes.
    #[test]
    fn a_stopped_render_removes_its_partial_file() {
        let dir = std::env::temp_dir().join(format!("dawsome-play-to-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let opened =
            project_file::open(&Path::new(FIXTURES).join("f1_four_on_floor.json")).expect("F1");
        let render = Render {
            out: dir.join("stopped.wav"),
            bars: 4,
            mode: PlayMode::Song,
            encoding: Encoding::Wav(BitDepth::Int24, Dither::Tpdf),
        };
        let mut blocks = 0;
        let outcome = render_document(
            opened.project,
            opened.samples_dir,
            None,
            &render,
            &mut |done, total| {
                blocks += 1;
                assert!(done <= total);
                blocks < 3
            },
            None,
            None,
        )
        .expect("renders");
        assert!(!outcome.finished);
        assert_eq!(blocks, 3);
        assert!(!render.out.exists());
    }
}
