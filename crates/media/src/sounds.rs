//! The sounds a compile asks for, from files: sampler samples and short audio clips decoded
//! into memory at the project's rate, long audio clips streamed from disk, a long file at
//! another rate converted once into the cache (live playback) or into memory (offline), a
//! sound's edits applied (the edited sound in memory, or a long one in the edited cache, made
//! on a worker live: [`crate::edited`]), and instrument sets: their SFZ files read and every
//! zone's sound decoded into memory, with a report of what they leave out.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, SystemTime};

use dawsome_engine::compile::{ClipHint, ClipSound, EditedSound, Sounds};
use dawsome_engine::dsp::instrument::{FileLoop, Instrument, ZoneSample};
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::project::{ChannelKind, ClipItem, ClipStretch, Project, SoundEdits};
use dawsome_engine::sfz::{self, Ignored};
use dawsome_engine::source::{ClipSource, MemorySource, SourceRead};
use sha2::{Digest, Sha256};

use crate::decode::probe;
use crate::edited::{self, EditJob, EditOutcome, Edited};
use crate::paths::{sound_file_from, ShippedRoots};
use crate::sample::{load_channel_sample, load_zone_sample, ChannelSample, SampleNotice};
use crate::set_files::{self, SetFolder};
use crate::stream::{StreamingSource, Waiting};
use crate::stretch::{EditedSource, StretchJob, StretchOutcome, Stretched};

/// Seconds of a file, at the project's rate, that still load into memory.
pub const MEMORY_SECONDS: u64 = 30;

/// The most clips that stream at once; past that they load into memory.
pub const MAX_STREAMS: usize = 64;

/// How a session plays its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playback {
    /// Files of more frames than this (at the project's rate) stream; `None` is
    /// [`MEMORY_SECONDS`] of frames.
    pub stream_threshold: Option<u64>,
    /// Live playback never waits for the disk; offline renders do.
    pub waiting: Waiting,
    /// Where a long file at another rate is converted to live playback's rate
    /// ([`convert_dir`]); `None` loads such files into memory.
    pub convert_dir: Option<PathBuf>,
    /// Where stretched clips are kept ([`stretch_dir`]); `None` stretches into memory each
    /// time.
    pub stretch_dir: Option<PathBuf>,
    /// Where long edited sounds are kept ([`edited_dir`]); `None` keeps them in memory.
    pub edited_dir: Option<PathBuf>,
}

impl Playback {
    /// An offline render: streams wait for the disk, nothing is written to the cache.
    pub fn offline(stream_threshold: Option<u64>) -> Self {
        Self {
            stream_threshold,
            waiting: Waiting::UntilReady,
            convert_dir: None,
            stretch_dir: None,
            edited_dir: None,
        }
    }

    /// This, keeping stretched clips in `dir`.
    pub fn stretching_into(self, dir: Option<PathBuf>) -> Self {
        Self {
            stretch_dir: dir,
            ..self
        }
    }

    /// This, keeping long edited sounds in `dir`.
    pub fn editing_into(self, dir: Option<PathBuf>) -> Self {
        Self {
            edited_dir: dir,
            ..self
        }
    }
}

/// A file as the memory cache knows it: where it really is, its size and time, the rate it
/// was loaded at, and the loop of the zone it was loaded for (a file converted to the project's
/// rate is converted as that loop plays on past its end: [`FileLoop`]). A file that changes on
/// disk is a new one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FileKey {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
    rate: u32,
    file_loop: Option<FileLoop>,
}

impl FileKey {
    fn of(path: &Path, rate: u32) -> Self {
        Self::of_zone(path, rate, None)
    }

    /// The key of `path` loaded for a zone that plays `file_loop`.
    fn of_zone(path: &Path, rate: u32, file_loop: Option<FileLoop>) -> Self {
        let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let metadata = fs::metadata(&path).ok();
        Self {
            size: metadata.as_ref().map_or(0, fs::Metadata::len),
            modified: metadata.and_then(|metadata| metadata.modified().ok()),
            path,
            rate,
            file_loop,
        }
    }

    /// The converted file's name: the first 16 hex digits of the SHA-256 of what makes the
    /// file this file at this rate.
    fn converted_name(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(self.path.as_os_str().as_encoded_bytes());
        hash.update(self.size.to_le_bytes());
        let since = self
            .modified
            .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_nanos());
        hash.update(since.to_le_bytes());
        hash.update(self.rate.to_le_bytes());
        let digest = hash.finalize();
        let hex: String = digest
            .iter()
            .take(8)
            .map(|byte| format!("{byte:02x}"))
            .collect();
        format!("{hex}.wav")
    }
}

/// A file the loader thread decoded: the path the project names it by, what makes it that
/// file, and what loading it gave.
type LoadedFile = (PathBuf, FileKey, ChannelSample);

/// What loading an instrument set left out, for the user to read: the opcodes and headers its
/// reader does not play, and what could not be read or played as written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstrumentReport {
    /// The set's main file, as it is on disk.
    pub file: PathBuf,
    /// How many zones play.
    pub zones: usize,
    pub ignored: Vec<Ignored>,
    /// Each with where in the set it is.
    pub problems: Vec<String>,
}

impl InstrumentReport {
    /// Whether the set played all of itself.
    pub fn is_complete(&self) -> bool {
        self.ignored.is_empty() && self.problems.is_empty()
    }

    /// One line: what was left out, each name once with how often.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.ignored.is_empty() {
            let names: Vec<String> = self
                .ignored
                .iter()
                .map(|ignored| format!("{} (\u{d7}{})", ignored.name, ignored.count))
                .collect();
            parts.push(format!("leaves out {}", names.join(", ")));
        }
        match self.problems.len() {
            0 => {}
            1 => parts.push(format!("1 problem: {}", self.problems[0])),
            count => parts.push(format!("{count} problems, the first: {}", self.problems[0])),
        }
        parts.join("; ")
    }
}

/// An instrument set as loaded: the main file as it was read, what reading it gave (or why it
/// could not be read), and the set once every zone's sound is in.
struct LoadedSet {
    key: FileKey,
    /// The folder added to the Library it was read inside, or `None` for its own folder.
    root: Option<PathBuf>,
    read: sfz::Sfz,
    unreadable: Option<String>,
    /// Its zones' files as the memory cache knows them, once built.
    files: Vec<FileKey>,
    built: Option<Instrument>,
}

/// A clip's file as the project names it, the edits it plays through, and how it is stretched.
type StretchKey = (PathBuf, SoundEdits, ClipStretch);

/// A stretch as it is kept once made: the file as it is on disk, so a rewritten file is
/// stretched anew, its edits (none: the file itself) and how it is stretched.
type MadeKey = (FileKey, SoundEdits, ClipStretch);

/// A stretch the worker finished: as asked for, as kept, and what it gave.
type MadeStretch = (StretchKey, MadeKey, StretchOutcome);

/// An edited sound as asked for: the file as the project names it, and its edits.
type EditAsk = (PathBuf, SoundEdits);

/// An edited sound as it is kept once made: the file as it is on disk, so a rewritten file is
/// edited anew, and its edits.
type EditKey = (FileKey, SoundEdits);

/// An edited sound the worker finished: as asked for, as kept, and what it gave.
type MadeEdit = (EditAsk, EditKey, EditOutcome);

/// [`Sounds`] from the files of a project whose sample paths are relative to `dir`.
pub struct MediaSounds {
    dir: PathBuf,
    /// Where shipped references are read from: the user's folder, then the installed sets.
    roots: ShippedRoots,
    /// The folders added to the Library: a set inside one reads inside it
    /// ([`set_files::library_root`]).
    library: Vec<PathBuf>,
    rate: u32,
    playback: Playback,
    memory: HashMap<FileKey, SampleBuffer>,
    /// Streams open now, counted by the streams themselves as they open and drop.
    streams: Arc<AtomicUsize>,
    notices: Vec<SampleNotice>,
    /// Files being converted on a worker, by their path as the project writes it.
    converting: BTreeSet<PathBuf>,
    converted: (mpsc::Sender<PathBuf>, mpsc::Receiver<PathBuf>),
    /// Files the loader thread is decoding into memory (live playback), by their path as the
    /// project writes it and the loop of the zone they are for.
    loading: BTreeSet<(PathBuf, Option<FileLoop>)>,
    /// The loader thread's queue, once it runs, and what it hands back.
    loader: Option<mpsc::Sender<(PathBuf, Option<FileLoop>)>>,
    loaded: (mpsc::Sender<LoadedFile>, mpsc::Receiver<LoadedFile>),
    /// Stretched files made (or found in the cache), and silence for those that cannot be,
    /// for the clips the last compile plays.
    stretched: HashMap<MadeKey, Stretched>,
    /// Stretches the worker is making.
    stretching: HashSet<StretchKey>,
    /// The stretch worker's queue, once it runs, and what it hands back.
    stretcher: Option<mpsc::Sender<(StretchKey, MadeKey, StretchJob)>>,
    made: (mpsc::Sender<MadeStretch>, mpsc::Receiver<MadeStretch>),
    /// Edited sounds made (or found in the cache) for the channels the last compile plays, and
    /// the last one made of each file.
    edited: HashMap<EditKey, Edited>,
    /// Edited sounds the worker is making.
    editing: HashSet<EditAsk>,
    /// The edit worker's queue, once it runs, and what it hands back.
    editor: Option<mpsc::Sender<(EditAsk, EditKey, EditJob)>>,
    edits_made: (mpsc::Sender<MadeEdit>, mpsc::Receiver<MadeEdit>),
    /// For each file (as the project names it), the edited sound made last: what its channel
    /// plays for now while the sound of a newer list is made.
    last_edited: HashMap<PathBuf, EditKey>,
    /// Each file's own rate, for the files in `memory` (`None`: it could not be read), and
    /// whether its frames there are still at that rate (a zone's, [`ZoneSample::at_file_rate`]).
    rates: HashMap<FileKey, (Option<u32>, bool)>,
    /// Instrument sets by their main file's path as the project writes it.
    sets: HashMap<PathBuf, LoadedSet>,
    /// What loading each set left out, by the same path.
    reports: HashMap<PathBuf, InstrumentReport>,
    /// The sets whose zones are still loading: (sounds in, sounds in all).
    progress: BTreeMap<PathBuf, (usize, usize)>,
}

impl MediaSounds {
    pub fn new(dir: &Path, project_rate: u32, playback: Playback) -> Self {
        Self {
            dir: dir.to_path_buf(),
            roots: ShippedRoots::find(),
            library: Vec::new(),
            rate: project_rate,
            playback,
            memory: HashMap::new(),
            streams: Arc::new(AtomicUsize::new(0)),
            notices: Vec::new(),
            converting: BTreeSet::new(),
            converted: mpsc::channel(),
            loading: BTreeSet::new(),
            loader: None,
            loaded: mpsc::channel(),
            stretched: HashMap::new(),
            stretching: HashSet::new(),
            stretcher: None,
            made: mpsc::channel(),
            edited: HashMap::new(),
            editing: HashSet::new(),
            editor: None,
            edits_made: mpsc::channel(),
            last_edited: HashMap::new(),
            rates: HashMap::new(),
            sets: HashMap::new(),
            reports: HashMap::new(),
            progress: BTreeMap::new(),
        }
    }

    /// The file `path`, as the project writes it, names.
    fn file(&self, path: &Path) -> PathBuf {
        sound_file_from(&self.dir, path, &self.roots)
    }

    /// The folders added to the Library are `folders` from now on: a set whose main file lies
    /// inside one reads inside the outermost holding it, at most two folders above its own; the
    /// home folder, a folder holding it or a filesystem's root added widens no set
    /// ([`set_files::library_root`]);
    /// a shipped set reads inside its own folder whatever is added. The sets read before that
    /// would now read inside another folder are let go, to be read again when next asked for;
    /// returns them (as the project names them), for the caller to ask for them again.
    pub fn set_library(&mut self, folders: Vec<PathBuf>) -> Vec<PathBuf> {
        self.library = folders;
        let changed: Vec<PathBuf> = self
            .sets
            .iter()
            .filter(|(path, set)| self.set_root(path) != set.root)
            .map(|(path, _)| path.clone())
            .collect();
        for path in &changed {
            self.sets.remove(path);
            self.reports.remove(path);
            self.progress.remove(path);
        }
        changed
    }

    /// The folder added to the Library the set the project names `path` reads inside, or
    /// `None` for its own folder: always for a shipped set, whose files are the install's.
    fn set_root(&self, path: &Path) -> Option<PathBuf> {
        if crate::paths::shipped_part(path).is_some() {
            return None;
        }
        set_files::library_root(&self.file(path), &self.library)
    }

    /// What loading the instrument set at `path` (as the project writes it) left out, once it
    /// is loaded.
    pub fn report(&self, path: &Path) -> Option<&InstrumentReport> {
        self.reports.get(path)
    }

    /// While instrument sets load: how many of their sounds are in, of how many.
    pub fn instrument_progress(&self) -> Option<(usize, usize)> {
        (!self.progress.is_empty()).then(|| {
            self.progress
                .values()
                .fold((0, 0), |(done, all), &(set_done, set_all)| {
                    (done + set_done, all + set_all)
                })
        })
    }

    /// The directory sample paths are relative to.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// What loading had to say since this was last called.
    pub fn take_notices(&mut self) -> Vec<SampleNotice> {
        std::mem::take(&mut self.notices)
    }

    /// The files whose loading, conversion, stretch or edit finished since this was last called
    /// (paths as the project writes them): compile again, and their channels and clips get their
    /// sounds.
    pub fn take_loaded(&mut self) -> Vec<PathBuf> {
        let mut done: Vec<PathBuf> = self.converted.1.try_iter().collect();
        for path in &done {
            self.converting.remove(path);
        }
        let loaded: Vec<LoadedFile> = self.loaded.1.try_iter().collect();
        for (path, key, sample) in loaded {
            self.notices.extend(sample.notices);
            self.loading.remove(&(path.clone(), key.file_loop));
            self.rates
                .insert(key.clone(), (sample.file_rate, sample.at_file_rate));
            self.memory.insert(key, sample.buffer);
            done.push(path);
        }
        let finished: Vec<MadeStretch> = self.made.1.try_iter().collect();
        for (key, made, outcome) in finished {
            self.stretching.remove(&key);
            self.keep_stretched(&key.0, made, outcome);
            done.push(key.0);
        }
        let edited: Vec<MadeEdit> = self.edits_made.1.try_iter().collect();
        for (ask, key, outcome) in edited {
            self.editing.remove(&ask);
            self.keep_edited(&ask.0, key, outcome);
            done.push(ask.0);
        }
        done
    }

    /// Whether a file is still being loaded, converted, stretched or edited.
    pub fn is_loading(&self, path: &Path) -> bool {
        self.loading.iter().any(|(file, _)| file == path)
            || self.converting.contains(path)
            || self.stretching.iter().any(|(file, ..)| file == path)
            || self.editing.iter().any(|(file, _)| file == path)
    }

    /// Whether any file is being loaded, converted, stretched or edited.
    pub fn any_loading(&self) -> bool {
        !self.loading.is_empty()
            || !self.converting.is_empty()
            || !self.stretching.is_empty()
            || !self.editing.is_empty()
    }

    /// Whether the clip of `path` stretched as `stretch` is still being made (of the file, or
    /// of its edited sound).
    pub fn is_stretching(&self, path: &Path, stretch: ClipStretch) -> bool {
        self.stretching
            .iter()
            .any(|(file, _, made)| file == path && *made == stretch)
    }

    /// The sound of a clip of `path` played through `edits` (none: the file itself) and
    /// stretched as `stretch`: made already, found in the cache or made now (offline, or with
    /// no worker), or, live, made on the worker while the clip plays silence. An edited clip
    /// stretches the edited sound, once it is made. A file that cannot be stretched plays
    /// silence, with a notice.
    fn stretched_clip(
        &mut self,
        path: &Path,
        edits: &SoundEdits,
        stretch: ClipStretch,
        hint: ClipHint,
    ) -> ClipSound {
        let key = (path.to_path_buf(), edits.clone(), stretch);
        let made = (
            FileKey::of(&self.file(path), self.rate),
            edits.clone(),
            stretch,
        );
        if !self.stretched.contains_key(&made) {
            if self.stretching.contains(&key) {
                return ClipSound::Loading;
            }
            let edited = if edits.is_empty() {
                None
            } else {
                let Some(sound) = self.edited(path, edits, true) else {
                    return ClipSound::Loading;
                };
                Some(EditedSource {
                    digest: edited::digest(&self.file(path), edits),
                    sound,
                })
            };
            let job = StretchJob {
                file: self.file(path),
                rate: self.rate,
                stretch,
                cache: self.playback.stretch_dir.clone(),
                stream_threshold: self.stream_threshold(),
                edited,
            };
            if self.playback.waiting == Waiting::Never
                && self.queue_stretch(key, made.clone(), &job)
            {
                return ClipSound::Loading;
            }
            let outcome = job.run();
            self.keep_stretched(path, made.clone(), outcome);
        }
        match self.stretched.get(&made).cloned() {
            Some(Stretched::Memory(buffer)) => ClipSound::Memory(buffer),
            Some(Stretched::Cached(file)) => self
                .stream(path, &file, hint)
                .unwrap_or_else(|| self.memory_clip(&file)),
            None => ClipSound::Memory(SampleBuffer::from(Vec::new())),
        }
    }

    /// Keeps what a stretch of the clip file `path` gave, and says why when it failed: that
    /// clip plays silence.
    fn keep_stretched(&mut self, path: &Path, made: MadeKey, outcome: StretchOutcome) {
        self.notices.extend(outcome.notices);
        let stretched = outcome.result.unwrap_or_else(|error| {
            self.notices.push(SampleNotice::Unstretchable {
                path: self.file(path),
                reason: error.to_string(),
            });
            Stretched::Memory(SampleBuffer::from(Vec::new()))
        });
        self.stretched.insert(made, stretched);
    }

    /// Puts a stretch on the worker's queue, once, starting the worker the first time. Returns
    /// false when there is no worker to make it, so the caller makes it itself.
    fn queue_stretch(&mut self, key: StretchKey, made: MadeKey, job: &StretchJob) -> bool {
        if self.stretcher.is_none() {
            let (jobs, queue) = mpsc::channel::<(StretchKey, MadeKey, StretchJob)>();
            let done = self.made.0.clone();
            let spawned = thread::Builder::new()
                .name("dawsome-stretch".into())
                .spawn(move || {
                    for (key, made, job) in queue {
                        if done.send((key, made, job.run())).is_err() {
                            break;
                        }
                    }
                });
            if spawned.is_err() {
                return false;
            }
            self.stretcher = Some(jobs);
        }
        let queued = self
            .stretcher
            .as_ref()
            .is_some_and(|stretcher| stretcher.send((key.clone(), made, job.clone())).is_ok());
        if queued {
            self.stretching.insert(key);
        }
        queued
    }

    /// The edited sound of `path` (as the project names it) through `edits` (never empty), as
    /// one that `streams` (a clip) takes it: made already, found in the cache, or made now: on
    /// this thread when it is one op past a short sound held in memory (the list before it, or
    /// the file itself for a first op), else on the worker live (`None` until it lands) and here
    /// offline.
    fn edited(&mut self, path: &Path, edits: &SoundEdits, streams: bool) -> Option<Edited> {
        let file = self.file(path);
        let key = (FileKey::of(&file, self.rate), edits.clone());
        if let Some(made) = self.edited.get(&key) {
            return Some(made.clone());
        }
        let ask = (path.to_path_buf(), edits.clone());
        if self.editing.contains(&ask) {
            return None;
        }
        let ops = edits.ops.len();
        let before = SoundEdits {
            ops: edits.ops[..ops.saturating_sub(1)].to_vec(),
            source: edits.source,
        };
        let base = if before.is_empty() {
            self.memory.get(&key.0).cloned().map(Edited::Memory)
        } else {
            self.edited.get(&(key.0.clone(), before)).cloned()
        };
        let threshold = self.stream_threshold();
        let short =
            |base: &Edited| matches!(base, Edited::Memory(buffer) if buffer.frames() <= threshold);
        let inline = base.as_ref().is_some_and(short);
        let job = EditJob {
            file,
            rate: self.rate,
            edits: edits.clone(),
            cache: self.playback.edited_dir.clone(),
            stream_threshold: threshold,
            streams,
            base: base.map(|base| (base, ops.saturating_sub(1))),
        };
        if !inline
            && self.playback.waiting == Waiting::Never
            && self.queue_edit(ask, key.clone(), &job)
        {
            return None;
        }
        let outcome = job.run();
        self.keep_edited(path, key.clone(), outcome);
        self.edited.get(&key).cloned()
    }

    /// Keeps what editing the file `path` gave, as the last made of it, and keeps the edited
    /// cache under its cap when it wrote a file there.
    fn keep_edited(&mut self, path: &Path, key: EditKey, outcome: EditOutcome) {
        self.notices.extend(outcome.notices);
        self.edited.insert(key.clone(), outcome.result);
        self.last_edited.insert(path.to_path_buf(), key);
        if let (true, Some(dir)) = (outcome.wrote, self.playback.edited_dir.clone()) {
            let keep: HashSet<PathBuf> = self
                .edited
                .values()
                .filter_map(|made| match made {
                    Edited::Cached(file) => Some(file.clone()),
                    Edited::Memory(_) => None,
                })
                .collect();
            edited::trim(&dir, edited::CACHE_CAP, &keep);
        }
    }

    /// Puts an edit on the worker's queue, once, starting the worker the first time. Returns
    /// false when there is no worker to make it, so the caller makes it itself.
    fn queue_edit(&mut self, ask: EditAsk, key: EditKey, job: &EditJob) -> bool {
        if self.editor.is_none() {
            let (jobs, queue) = mpsc::channel::<(EditAsk, EditKey, EditJob)>();
            let done = self.edits_made.0.clone();
            let spawned = thread::Builder::new()
                .name("dawsome-edit".into())
                .spawn(move || {
                    for (ask, key, job) in queue {
                        if done.send((ask, key, job.run())).is_err() {
                            break;
                        }
                    }
                });
            if spawned.is_err() {
                return false;
            }
            self.editor = Some(jobs);
        }
        let queued = self
            .editor
            .as_ref()
            .is_some_and(|editor| editor.send((ask.clone(), key, job.clone())).is_ok());
        if queued {
            self.editing.insert(ask);
        }
        queued
    }

    /// The edited sound of `path` made last, when it is not the one of `edits`: what the
    /// channel plays for now while that one is made.
    fn last_made(&self, path: &Path, edits: &SoundEdits) -> Option<&Edited> {
        let key = self.last_edited.get(path).filter(|key| key.1 != *edits)?;
        self.edited.get(key)
    }

    /// What a sampler of `path` plays while its sound through `edits` is made: the edited sound
    /// made last, else the file as it is, when either is in memory; else nothing (silence).
    fn for_now_sample(&self, path: &Path, edits: &SoundEdits) -> Option<SampleBuffer> {
        let at_rate = |file: &Path| FileKey::of(file, self.rate);
        match self.last_made(path, edits) {
            Some(Edited::Memory(buffer)) => Some(buffer.clone()),
            Some(Edited::Cached(file)) => self.memory.get(&at_rate(file)).cloned(),
            None => self.memory.get(&at_rate(&self.file(path))).cloned(),
        }
    }

    /// Files of more frames than this (at the project's rate) stream.
    pub fn stream_threshold(&self) -> u64 {
        self.playback
            .stream_threshold
            .unwrap_or(MEMORY_SECONDS * u64::from(self.rate))
    }

    /// The file `path` names, decoded at the project's rate, from the cache when it holds it.
    /// Live playback decodes it on the loader thread instead and gives `None` until it is in.
    /// Why: a long file takes seconds to decode and convert, and the thread asking is the UI
    /// thread, compiling.
    pub fn memory(&mut self, path: &Path) -> Option<SampleBuffer> {
        self.memory_for(path, None)
    }

    /// As [`memory`](Self::memory), loaded for a zone that plays `file_loop`.
    fn memory_for(&mut self, path: &Path, file_loop: Option<FileLoop>) -> Option<SampleBuffer> {
        let key = FileKey::of_zone(&self.file(path), self.rate, file_loop);
        if let Some(buffer) = self.memory.get(&key) {
            return Some(buffer.clone());
        }
        if self.playback.waiting == Waiting::Never && self.load(path, file_loop) {
            return None;
        }
        let loaded = load_zone_sample(&self.dir, path, self.rate, file_loop);
        self.notices.extend(loaded.notices);
        self.rates
            .insert(key.clone(), (loaded.file_rate, loaded.at_file_rate));
        self.memory.insert(key, loaded.buffer.clone());
        Some(loaded.buffer)
    }

    /// The zone sound the project would name `path`, for a zone that plays `file_loop`, from
    /// memory as [`memory`](Self::memory) loads it, with its file's own rate.
    fn zone_sample(&mut self, path: &Path, file_loop: Option<FileLoop>) -> Option<ZoneSample> {
        let buffer = self.memory_for(path, file_loop)?;
        let key = FileKey::of_zone(&self.file(path), self.rate, file_loop);
        let (file_rate, at_file_rate) = self.rates.get(&key).copied().unwrap_or((None, false));
        Some(ZoneSample {
            buffer,
            file_rate,
            at_file_rate,
        })
    }

    /// The instrument set whose main file the project names `path`: read (its includes from
    /// beside it), then, once every zone's sound is in memory, built and kept. Live playback
    /// loads the sounds on the loader thread and gives `None` until the last is in, counting
    /// them for [`instrument_progress`](Self::instrument_progress). A main file that cannot be
    /// read is an empty set with a report saying so.
    fn instrument_set(&mut self, path: &Path) -> Option<Instrument> {
        let main = self.file(path);
        let key = FileKey::of(&main, self.rate);
        if self.sets.get(path).is_none_or(|set| set.key != key) {
            let set = read_set(&main, key, self.set_root(path));
            self.sets.insert(path.to_path_buf(), set);
            self.reports.remove(path);
        }
        let set = self.sets.get(path)?;
        if let Some(built) = &set.built {
            return Some(built.clone());
        }
        // Each sound once per loop its zones play: a file a loop ends on is converted for it.
        let mut wanted: Vec<(String, Option<FileLoop>)> = set
            .read
            .regions
            .iter()
            .map(|region| (region.sample.clone(), FileLoop::of(region)))
            .collect();
        wanted.sort();
        wanted.dedup();
        let mut sounds = HashMap::new();
        for (name, file_loop) in &wanted {
            if let Some(sample) = self.zone_sample(&beside(path, name), *file_loop) {
                sounds.insert((name.clone(), *file_loop), sample);
            }
        }
        if sounds.len() < wanted.len() {
            self.progress
                .insert(path.to_path_buf(), (sounds.len(), wanted.len()));
            return None;
        }
        self.progress.remove(path);
        let files = wanted
            .iter()
            .map(|(name, file_loop)| {
                FileKey::of_zone(&self.file(&beside(path, name)), self.rate, *file_loop)
            })
            .collect();
        let rate = self.rate;
        let set = self.sets.get_mut(path)?;
        let silent = || ZoneSample {
            buffer: SampleBuffer::from(Vec::new()),
            file_rate: None,
            at_file_rate: false,
        };
        let (built, problems) = Instrument::build_with(
            &set.read.regions,
            |region| {
                let key = (region.sample.clone(), FileLoop::of(region));
                sounds.get(&key).cloned().unwrap_or_else(silent)
            },
            rate,
        );
        set.built = Some(built.clone());
        set.files = files;
        let mut all: Vec<String> = set.unreadable.iter().cloned().collect();
        all.extend(set.read.problems.iter().map(ToString::to_string));
        all.extend(problems);
        let report = InstrumentReport {
            file: main.clone(),
            zones: built.zones(),
            ignored: set.read.ignored.clone(),
            problems: all,
        };
        if !report.is_complete() {
            self.notices.push(SampleNotice::Instrument {
                path: main,
                summary: report.summary(),
            });
        }
        self.reports.insert(path.to_path_buf(), report);
        Some(built)
    }

    /// An edited sound of `path` as a clip plays it: from memory, or a stream of its cached file
    /// (a sound in memory when no stream can be had).
    fn edited_sound(&mut self, path: &Path, made: Edited, hint: ClipHint) -> ClipSound {
        match made {
            Edited::Memory(buffer) => ClipSound::Memory(buffer),
            Edited::Cached(file) => self
                .stream(path, &file, hint)
                .unwrap_or_else(|| self.memory_clip(&file)),
        }
    }

    /// [`memory`](Self::memory) as a clip's sound.
    fn memory_clip(&mut self, path: &Path) -> ClipSound {
        self.memory(path)
            .map_or(ClipSound::Loading, ClipSound::Memory)
    }

    /// Puts `path` on the loader thread's queue, once, starting the thread the first time.
    /// Returns false when there is no thread to load it, so the caller loads it itself.
    fn load(&mut self, path: &Path, file_loop: Option<FileLoop>) -> bool {
        let job = (path.to_path_buf(), file_loop);
        if self.loading.contains(&job) {
            return true;
        }
        let queued = self
            .loader()
            .is_some_and(|loader| loader.send(job.clone()).is_ok());
        if queued {
            self.loading.insert(job);
        }
        queued
    }

    /// The loader thread's queue: one thread decodes the files in the order asked, so opening a
    /// project of many samples starts one thread, not one per file.
    fn loader(&mut self) -> Option<&mpsc::Sender<(PathBuf, Option<FileLoop>)>> {
        if self.loader.is_none() {
            let (jobs, queue) = mpsc::channel::<(PathBuf, Option<FileLoop>)>();
            let (dir, rate, done) = (self.dir.clone(), self.rate, self.loaded.0.clone());
            let roots = self.roots.clone();
            thread::Builder::new()
                .name("dawsome-load".into())
                .spawn(move || {
                    for (path, file_loop) in queue {
                        let file = sound_file_from(&dir, &path, &roots);
                        let key = FileKey::of_zone(&file, rate, file_loop);
                        let sample = load_zone_sample(Path::new(""), &file, rate, file_loop);
                        if done.send((path, key, sample)).is_err() {
                            break;
                        }
                    }
                })
                .ok()?;
            self.loader = Some(jobs);
        }
        self.loader.as_ref()
    }

    /// A stream of the file at `file` (at the project's rate already), counted while open.
    fn stream(&mut self, path: &Path, file: &Path, hint: ClipHint) -> Option<ClipSound> {
        if self.streams.load(Ordering::Relaxed) >= MAX_STREAMS {
            self.notices.push(SampleNotice::TooManyStreams {
                path: self.file(path),
            });
            return None;
        }
        match StreamingSource::open(
            file,
            hint.first_frame,
            hint.loop_frame,
            self.playback.waiting,
        ) {
            Ok(stream) => {
                self.streams.fetch_add(1, Ordering::Relaxed);
                Some(ClipSound::Stream(Box::new(Counted {
                    stream,
                    open: Arc::clone(&self.streams),
                })))
            }
            Err(error) => {
                self.notices.push(SampleNotice::Unreadable {
                    path: file.to_path_buf(),
                    reason: error.to_string(),
                });
                None
            }
        }
    }

    /// Starts converting `file` to the project's rate into `target` on a worker, once.
    fn convert(&mut self, path: &Path, file: PathBuf, target: PathBuf) {
        if !self.converting.insert(path.to_path_buf()) {
            return;
        }
        let (done, name, rate) = (self.converted.0.clone(), path.to_path_buf(), self.rate);
        let spawned = thread::Builder::new()
            .name("dawsome-convert".into())
            .spawn(move || {
                let loaded = load_channel_sample(Path::new(""), &file, rate);
                // A failed write leaves no file, so the next compile tries again.
                let _ = write_float_wav(&loaded.buffer, rate, &target);
                let _ = done.send(name);
            });
        if spawned.is_err() {
            self.converting.remove(path);
        }
    }
}

/// Writes `buffer` as a 32-bit float WAV at `rate` to `target`, by way of a temporary file of
/// this write's own so a half-written one is never read; a failed write leaves no temporary
/// file and `target` as it was. Why its own: two renders may make the same cached file at
/// once, and each must land whole without failing or removing what the other wrote.
pub(crate) fn write_float_wav(
    buffer: &SampleBuffer,
    rate: u32,
    target: &Path,
) -> Result<(), crate::WriteError> {
    write_float_frames(buffer.samples(), buffer.channels(), rate, target)
}

/// As [`write_float_wav`], for the interleaved `samples` of `channels` sides: a sound held as
/// frames to edit (the sample editor's Save as new file).
pub fn write_float_frames(
    samples: &[f32],
    channels: u8,
    rate: u32,
    target: &Path,
) -> Result<(), crate::WriteError> {
    let partial = partial_path(target);
    let written = write_partial(samples, channels, rate, target, &partial);
    if written.is_err() {
        let _ = fs::remove_file(&partial);
    }
    written
}

/// `target` with `.<process id>-<write>.partial` added: a name no other write uses meanwhile.
fn partial_path(target: &Path) -> PathBuf {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let write = WRITES.fetch_add(1, Ordering::Relaxed);
    let mut name = target.as_os_str().to_owned();
    name.push(format!(".{}-{write}.partial", std::process::id()));
    PathBuf::from(name)
}

/// Whether `name` is one [`partial_path`] gives a WAV's write: `<stem>.wav.<process id>-<write>.partial`.
fn is_partial_wav(name: &str) -> bool {
    let number = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    let Some((target, write)) = name
        .strip_suffix(".partial")
        .and_then(|rest| rest.rsplit_once('.'))
    else {
        return false;
    };
    let Some((process, count)) = write.split_once('-') else {
        return false;
    };
    let stem = target.strip_suffix(".wav").unwrap_or_default();
    number(process) && number(count) && !stem.is_empty()
}

/// Removes from `dir` what writes of [`write_float_frames`] cut short left there (the process
/// ended under them): regular files named as it names a WAV's temporary file and last written
/// longer than `idle` ago, so a write under way (this process's or another's) is never touched;
/// nothing else, and nothing in folders below. Returns the files removed.
pub fn remove_stale_partials(dir: &Path, idle: Duration) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let now = SystemTime::now();
    let mut removed = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        if !entry.file_name().to_str().is_some_and(is_partial_wav) {
            continue;
        }
        // The entry itself: a link is not followed.
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let unwritten = metadata
            .modified()
            .ok()
            .and_then(|written| now.duration_since(written).ok());
        let path = entry.path();
        if metadata.is_file()
            && unwritten.is_some_and(|unwritten| unwritten > idle)
            && fs::remove_file(&path).is_ok()
        {
            removed.push(path);
        }
    }
    removed.sort();
    removed
}

fn write_partial(
    samples: &[f32],
    channels: u8,
    rate: u32,
    target: &Path,
    partial: &Path,
) -> Result<(), crate::WriteError> {
    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir)?;
    }
    let spec = hound::WavSpec {
        channels: u16::from(channels),
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(partial, spec)?;
    for &value in samples {
        writer.write_sample(value)?;
    }
    writer.finalize()?;
    fs::rename(partial, target)?;
    Ok(())
}

impl Sounds for MediaSounds {
    type Error = Infallible;

    fn sample(&mut self, path: &Path) -> Result<Option<SampleBuffer>, Infallible> {
        Ok(self.memory(path))
    }

    fn instrument(&mut self, path: &Path) -> Result<Option<Instrument>, Infallible> {
        Ok(self.instrument_set(path))
    }

    /// Lets go of the stretches no clip of `project` plays, and of the files in memory that no
    /// channel of it plays and no stretch kept is read from, as its files are on disk now. Why:
    /// every drag of a clip's edge makes a new stretch, every sound tried on a channel loads
    /// its file, and each one kept is a whole file in memory.
    fn forget_unused(&mut self, project: &Project) {
        if !self.stretched.is_empty() {
            let playing: HashSet<MadeKey> = project
                .playlist
                .clips
                .values()
                .filter_map(|clip| {
                    let ClipItem::Audio {
                        channel,
                        stretch: Some(stretch),
                        ..
                    } = clip.item
                    else {
                        return None;
                    };
                    let channel = project.channels.get(&channel)?;
                    let file = FileKey::of(&self.file(&channel.sample), self.rate);
                    Some((file, channel.edits.clone(), stretch))
                })
                .collect();
            self.stretched.retain(|made, _| playing.contains(made));
        }
        // The edited sounds the channels play, and the last made of each file they play.
        let played: HashSet<&Path> = project
            .channels
            .values()
            .filter(|channel| channel.plays_a_file())
            .map(|channel| channel.sample.as_path())
            .collect();
        self.last_edited
            .retain(|path, _| played.contains(path.as_path()));
        if !self.edited.is_empty() {
            let mut keep: HashSet<EditKey> = project
                .channels
                .values()
                .filter(|channel| channel.plays_a_file() && !channel.edits.is_empty())
                .map(|channel| {
                    let file = FileKey::of(&self.file(&channel.sample), self.rate);
                    (file, channel.edits.clone())
                })
                .collect();
            keep.extend(self.last_edited.values().cloned());
            self.edited.retain(|key, _| keep.contains(key));
        }
        let sets: HashSet<&Path> = project
            .channels
            .values()
            .filter(|channel| channel.kind == ChannelKind::Instrument)
            .map(|channel| channel.instrument.as_path())
            .collect();
        self.sets.retain(|path, _| sets.contains(path.as_path()));
        self.reports.retain(|path, _| sets.contains(path.as_path()));
        self.progress
            .retain(|path, _| sets.contains(path.as_path()));
        if !self.memory.is_empty() {
            // A set still loading keeps the sounds already in too: let go of, they would be
            // asked for again at the next compile, and a set whose files do not all land
            // between two compiles would never be built.
            let zones: Vec<FileKey> = self
                .sets
                .iter()
                .flat_map(|(path, set)| match &set.built {
                    Some(_) => set.files.clone(),
                    None => set
                        .read
                        .regions
                        .iter()
                        .map(|region| {
                            let file = self.file(&beside(path, &region.sample));
                            FileKey::of_zone(&file, self.rate, FileLoop::of(region))
                        })
                        .collect(),
                })
                .collect();
            let samples = project
                .channels
                .values()
                .filter(|channel| channel.plays_a_file())
                .map(|channel| channel.sample.as_path());
            let cached = self
                .stretched
                .values()
                .filter_map(|stretched| match stretched {
                    Stretched::Cached(file) => Some(file.as_path()),
                    Stretched::Memory(_) => None,
                });
            // A sampler of a long edited sound plays its cached file from memory.
            let edited = self.edited.values().filter_map(|made| match made {
                Edited::Cached(file) => Some(file.as_path()),
                Edited::Memory(_) => None,
            });
            let mut playing: HashSet<FileKey> = samples
                .chain(cached)
                .chain(edited)
                .map(|path| FileKey::of(&self.file(path), self.rate))
                .collect();
            playing.extend(zones);
            self.memory.retain(|file, _| playing.contains(file));
            self.rates.retain(|file, _| playing.contains(file));
        }
    }

    fn clip(&mut self, path: &Path, hint: ClipHint) -> Result<ClipSound, Infallible> {
        if let Some(stretch) = hint.stretch {
            return Ok(self.stretched_clip(path, &SoundEdits::default(), stretch, hint));
        }
        let file = self.file(path);
        let Ok(info) = probe(&file) else {
            // Unreadable: the memory load says why, once, and plays silence.
            return Ok(self.memory_clip(path));
        };
        let at_rate = info.frames.map(|frames| {
            (frames * u64::from(self.rate)).div_ceil(u64::from(info.sample_rate.max(1)))
        });
        // A file whose length its header does not give is decoded to find out.
        let short = at_rate.is_none_or(|frames| frames <= self.stream_threshold());
        if short {
            return Ok(self.memory_clip(path));
        }
        let streamed = if info.sample_rate == self.rate {
            self.stream(path, &file, hint)
        } else {
            match (self.playback.waiting, self.playback.convert_dir.clone()) {
                // Offline the converted frames would be the same as these: load them.
                (Waiting::UntilReady, _) | (_, None) => None,
                (Waiting::Never, Some(dir)) => {
                    let target = dir.join(FileKey::of(&file, self.rate).converted_name());
                    if target.is_file() {
                        self.stream(path, &target, hint)
                    } else {
                        self.convert(path, file, target);
                        return Ok(ClipSound::Loading);
                    }
                }
            }
        };
        Ok(streamed.unwrap_or_else(|| self.memory_clip(path)))
    }

    /// The edited sound, once made; while it is made (live), the edited sound made last of the
    /// file, else the file as it is, for now.
    fn edited_sample(
        &mut self,
        path: &Path,
        edits: &SoundEdits,
    ) -> Result<Option<EditedSound<SampleBuffer>>, Infallible> {
        let ready = match self.edited(path, edits, false) {
            Some(Edited::Memory(buffer)) => Some(buffer),
            // A long result another clip streams: its file, loaded as any sound is.
            Some(Edited::Cached(file)) => self.memory(&file),
            None => None,
        };
        Ok(match ready {
            Some(buffer) => Some(EditedSound::ready(buffer)),
            None => self.for_now_sample(path, edits).map(EditedSound::for_now),
        })
    }

    /// The edited clip, once made (a long one streamed from the edited cache); while it is made
    /// (live), the edited sound made last of the file, else the clip of the file as it is, for
    /// now. A stretched clip stretches the edited sound once it is made, silent meanwhile, as
    /// every new stretch is.
    fn edited_clip(
        &mut self,
        path: &Path,
        edits: &SoundEdits,
        hint: ClipHint,
    ) -> Result<EditedSound<ClipSound>, Infallible> {
        if let Some(stretch) = hint.stretch {
            return Ok(EditedSound::ready(
                self.stretched_clip(path, edits, stretch, hint),
            ));
        }
        if let Some(made) = self.edited(path, edits, true) {
            return Ok(EditedSound::ready(self.edited_sound(path, made, hint)));
        }
        let sound = match self.last_made(path, edits).cloned() {
            Some(made) => self.edited_sound(path, made, hint),
            None => self.clip(path, hint)?,
        };
        Ok(EditedSound::for_now(sound))
    }
}

/// The set whose main file is `main` (`key`), read with its includes from its folder, or from
/// `root` (a folder added to the Library holding it) when given, and nowhere else
/// ([`crate::set_files`]); a main file that cannot be read is a set of no regions, saying why.
fn read_set(main: &Path, key: FileKey, root: Option<PathBuf>) -> LoadedSet {
    let folder = match &root {
        Some(root) => SetFolder::inside(main, root),
        None => SetFolder::of(main),
    };
    let (read, unreadable) = match set_files::read_set_in(main, &folder) {
        Ok(read) => (read, None),
        Err(error) => (
            sfz::Sfz::default(),
            Some(format!("it cannot be read ({error})")),
        ),
    };
    LoadedSet {
        key,
        root,
        read,
        unreadable,
        files: Vec::new(),
        built: None,
    }
}

/// The path a project would name `name` by, a file of the set whose main file it names `set`:
/// beside it, a shipped reference staying one.
fn beside(set: &Path, name: &str) -> PathBuf {
    match crate::paths::shipped_part(set) {
        Some(part) => {
            let folder = part.rsplit_once('/').map_or("", |(folder, _)| folder);
            let joined = if folder.is_empty() {
                name.to_owned()
            } else {
                format!("{folder}/{name}")
            };
            PathBuf::from(format!("{}{joined}", crate::paths::SHIPPED_PREFIX))
        }
        None => set.parent().unwrap_or(Path::new("")).join(name),
    }
}

/// A stream that counts itself among the open ones until it is dropped (on the UI thread, when
/// the player hands it back).
struct Counted {
    stream: StreamingSource,
    open: Arc<AtomicUsize>,
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.open.fetch_sub(1, Ordering::Relaxed);
    }
}

impl ClipSource for Counted {
    fn frames(&self) -> u64 {
        self.stream.frames()
    }

    fn channels(&self) -> u8 {
        self.stream.channels()
    }

    fn read(&mut self, frame: u64, out: &mut [[f32; 2]]) -> SourceRead {
        self.stream.read(frame, out)
    }

    fn prepare(&mut self, frame: u64) {
        self.stream.prepare(frame);
    }
}

/// How a preview of the file at `path` (absolute, or relative to the working directory) gets
/// its sound for a project at `rate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewKind {
    /// Up to `threshold` frames at the project's rate (or a length the header does not give):
    /// decoded into memory, quickly.
    Short,
    /// Longer, at the project's rate: streamed from disk.
    Stream,
    /// Longer, at another rate: decoded and converted whole, which takes a while.
    Convert,
}

fn preview_kind(path: &Path, rate: u32, threshold: u64) -> PreviewKind {
    let Ok(info) = probe(path) else {
        return PreviewKind::Short;
    };
    let at_rate = info
        .frames
        .map(|frames| (frames * u64::from(rate)).div_ceil(u64::from(info.sample_rate.max(1))));
    match at_rate {
        Some(frames) if frames > threshold && info.sample_rate == rate => PreviewKind::Stream,
        Some(frames) if frames > threshold => PreviewKind::Convert,
        _ => PreviewKind::Short,
    }
}

/// The sound a preview of the file at `path` plays at the project's `rate`: a short file decoded
/// into memory, a long one at the project's rate streamed from disk (never waiting), a long one
/// at another rate decoded and converted. Why not through [`MediaSounds`]' cache: a browser
/// previews many files once each, and keeping each would grow without end.
pub fn preview(path: &Path, rate: u32, threshold: u64) -> (Box<dyn ClipSource>, Vec<SampleNotice>) {
    if preview_kind(path, rate, threshold) == PreviewKind::Stream {
        match StreamingSource::open(path, 0, None, Waiting::Never) {
            Ok(stream) => return (Box::new(stream), Vec::new()),
            Err(error) => {
                let notice = SampleNotice::Unreadable {
                    path: path.to_path_buf(),
                    reason: error.to_string(),
                };
                return (
                    Box::new(MemorySource::new(SampleBuffer::from(Vec::new()))),
                    vec![notice],
                );
            }
        }
    }
    let loaded = load_channel_sample(Path::new(""), path, rate);
    (Box::new(MemorySource::new(loaded.buffer)), loaded.notices)
}

/// `dawsome/converted` in the per-user cache folder (see [`cache_dir`]): where live playback
/// keeps long files converted to another rate.
pub fn convert_dir() -> Option<PathBuf> {
    cache_dir("converted")
}

/// `dawsome/stretch` in the per-user cache folder (see [`cache_dir`]): where stretched clips
/// are kept, for live playback and renders alike.
pub fn stretch_dir() -> Option<PathBuf> {
    cache_dir("stretch")
}

/// `dawsome/edited` in the per-user cache folder (see [`cache_dir`]): where long edited sounds
/// are kept, for live playback and renders alike ([`crate::edited`]).
pub fn edited_dir() -> Option<PathBuf> {
    cache_dir("edited")
}

/// `dawsome/<name>` in the per-user cache folder: `$DAWSOME_CACHE_HOME` when that is an absolute
/// path, else the platform's (`$XDG_CACHE_HOME` or `~/.cache` on Linux, `%LOCALAPPDATA%` on
/// Windows). Why a variable of our own: tests and scripted runs move the folder the same way on
/// every platform. The plugin host's catalogue goes by the same rule; neither crate depends on
/// the other.
fn cache_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os("DAWSOME_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(dirs::cache_dir)
        .map(|dir| dir.join("dawsome").join(name))
}

#[cfg(test)]
mod tests {
    use dawsome_engine::compile::Compiler;
    use dawsome_engine::edit::{self, Edit, Touched};
    use dawsome_engine::plan::{PlanUpdate, PlayMode};
    use dawsome_engine::project::{ClipId, Project, StretchMode};
    use dawsome_engine::realtime;
    use dawsome_engine::time::Ticks;
    use dawsome_engine::transport::Control;

    use super::*;

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
    }

    /// A temp folder of this test's own holding `tone.wav`, `frames` mono frames of `value` at
    /// 44.1 kHz, and p9_resample.json's project playing it: clip 1 resampled to 2000 frames.
    fn stretched_tone(test: &str, value: f32, frames: usize) -> (PathBuf, Project) {
        let dir = std::env::temp_dir().join(format!("dawsome-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        write_tone(&dir, value, frames);
        let text = fs::read_to_string(fixtures().join("p9_resample.json"))
            .expect("fixture")
            .replace("p9_clicks.wav", "tone.wav")
            .replace(r#""frames":44100"#, r#""frames":2000"#);
        (dir, Project::load_json(&text).expect("loads"))
    }

    fn write_tone(dir: &Path, value: f32, frames: usize) {
        let tone = SampleBuffer::mono(vec![value; frames]);
        write_float_wav(&tone, 44_100, &dir.join("tone.wav")).expect("written");
    }

    /// Clip 1 resampled to `frames` frames, kept one bar long.
    fn resample_to(frames: u64) -> Edit {
        Edit::SetClipStretch {
            clip: ClipId(1),
            stretch: Some(ClipStretch {
                mode: StretchMode::Resample,
                frames,
                pitch: 0,
            }),
            frame_offset: 0,
            length: Ticks(384),
        }
    }

    fn compile(
        compiler: &mut Compiler,
        project: &Project,
        touched: &Touched,
        sounds: &mut MediaSounds,
    ) -> PlanUpdate {
        compiler
            .compile_with(project, touched, PlayMode::Song, sounds)
            .expect("compiles")
    }

    // The clip's file is rewritten on disk (0.25 for 1000 frames, then 0.5 for 1200) while the
    // clip still plays the old one, and the clip is duplicated into bar 2: the copy, stretched
    // as the clip is, plays the new file, twice as loud at its frame 0 (resampling a constant
    // keeps it), not the stretch made of the old file.
    #[test]
    fn a_rewritten_file_plays_its_new_content_through_a_stretched_clip() {
        let (dir, mut project) = stretched_tone("rewritten", 0.25, 1000);
        let mut sounds = MediaSounds::new(&dir, 44_100, Playback::offline(None));
        let mut compiler = Compiler::new();
        let update = compile(&mut compiler, &project, &Touched::default(), &mut sounds);
        let (mut link, mut player) = realtime::link(update, project.tempo);
        let mut out = [[0.0f32; 2]; 64];
        link.send(Control::Play).expect("room");
        player.process(&mut out);
        let old = out[0][0];
        assert!(old > 0.0, "the old file plays: {old}");
        write_tone(&dir, 0.5, 1200);
        let copy = Edit::DuplicateClips {
            ids: vec![ClipId(1)],
            first: ClipId(2),
        };
        let applied = edit::apply(&mut project, copy).expect("duplicates");
        let update = compile(&mut compiler, &project, &applied.touched, &mut sounds);
        link.push_plan(update).expect("same time base");
        link.send(Control::Seek(Ticks(384))).expect("room");
        player.process(&mut out);
        assert_eq!(out[0][0], 2.0 * old, "the copy plays the new file");
        fs::remove_dir_all(&dir).expect("removed");
    }

    // Two renders stretching one clip into one cache folder write the same file at the same
    // time: each write lands whole (the file then holds one of the two, all of it), and
    // neither fails nor removes what the other wrote.
    #[test]
    fn two_writers_of_one_cached_file_each_land_whole() {
        let dir = std::env::temp_dir().join(format!("dawsome-two-writers-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let target = dir.join("stretched.wav");
        let buffers = [
            SampleBuffer::mono(vec![0.25; 200_000]),
            SampleBuffer::mono(vec![0.5; 200_000]),
        ];
        for round in 0..20 {
            let barrier = std::sync::Barrier::new(buffers.len());
            let (barrier, target) = (&barrier, &target);
            let results: Vec<Result<(), crate::WriteError>> = thread::scope(|scope| {
                let writers: Vec<_> = buffers
                    .iter()
                    .map(|buffer| {
                        scope.spawn(move || {
                            barrier.wait();
                            write_float_wav(buffer, 44_100, target)
                        })
                    })
                    .collect();
                writers
                    .into_iter()
                    .map(|writer| writer.join().expect("the writer returns"))
                    .collect()
            });
            assert!(
                target.is_file(),
                "round {round}: the file is gone: {results:?}"
            );
            let written = load_channel_sample(Path::new(""), target, 44_100).buffer;
            assert!(
                buffers
                    .iter()
                    .any(|buffer| buffer.samples() == written.samples()),
                "round {round}: the file is neither write whole"
            );
            assert!(
                results.iter().all(Result::is_ok),
                "round {round}: {results:?}"
            );
        }
        let files = fs::read_dir(&dir).expect("the folder").count();
        assert_eq!(files, 1, "no temporary file left");
        fs::remove_dir_all(&dir).expect("removed");
    }

    // Twenty stretches of one file, one after the other, as dragging a clip's edge makes them:
    // only the one the project still plays is kept.
    #[test]
    fn stretches_no_clip_plays_are_let_go() {
        let (dir, mut project) = stretched_tone("restretched", 0.25, 1000);
        let mut sounds = MediaSounds::new(&dir, 44_100, Playback::offline(None));
        let mut compiler = Compiler::new();
        compile(&mut compiler, &project, &Touched::default(), &mut sounds);
        for step in 1..=20 {
            let applied = edit::apply(&mut project, resample_to(2000 + 50 * step)).expect("ok");
            compile(&mut compiler, &project, &applied.touched, &mut sounds);
        }
        assert_eq!(sounds.stretched.len(), 1, "stretches kept");
        fs::remove_dir_all(&dir).expect("removed");
    }

    // Twenty files loaded one after the other, as trying sounds on a channel loads them, then
    // a project whose one channel plays the eighth: only that file stays in memory.
    #[test]
    fn files_no_channel_plays_are_let_go() {
        let dir = std::env::temp_dir().join(format!("dawsome-tried-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let name = |file: usize| PathBuf::from(format!("tone{file}.wav"));
        for file in 0..20 {
            let tone = SampleBuffer::mono(vec![0.25; 100 + file]);
            write_float_wav(&tone, 44_100, &dir.join(name(file))).expect("written");
        }
        let mut sounds = MediaSounds::new(&dir, 44_100, Playback::offline(None));
        for file in 0..20 {
            assert!(matches!(sounds.sample(&name(file)), Ok(Some(_))));
        }
        assert_eq!(sounds.memory.len(), 20);
        let text = fs::read_to_string(fixtures().join("p9_resample.json"))
            .expect("fixture")
            .replace("p9_clicks.wav", "tone7.wav");
        let project = Project::load_json(&text).expect("loads");
        sounds.forget_unused(&project);
        let kept: Vec<&Path> = sounds.memory.keys().map(|key| key.path.as_path()).collect();
        assert_eq!(kept.len(), 1, "files kept: {kept:?}");
        assert!(kept[0].ends_with("tone7.wav"), "{kept:?}");
        fs::remove_dir_all(&dir).expect("removed");
    }

    fn hint() -> ClipHint {
        ClipHint {
            first_frame: 0,
            loop_frame: None,
            stretch: None,
        }
    }

    // Up to the threshold a clip's file loads into memory, past it it streams; a file at another
    // rate loads into memory offline, and live without a cache folder.
    #[test]
    fn short_files_load_and_long_ones_stream() {
        let mut sounds = MediaSounds::new(&fixtures(), 44_100, Playback::offline(None));
        let saw = Path::new("p8_saw.wav");
        assert!(matches!(sounds.clip(saw, hint()), Ok(ClipSound::Memory(_))));
        let mut streaming = MediaSounds::new(&fixtures(), 44_100, Playback::offline(Some(0)));
        let Ok(ClipSound::Stream(source)) = streaming.clip(saw, hint()) else {
            panic!("p8_saw.wav streams past a threshold of 0");
        };
        assert_eq!(source.frames(), 30_000);
        assert_eq!(streaming.streams.load(Ordering::Relaxed), 1);
        drop(source);
        assert_eq!(
            streaming.streams.load(Ordering::Relaxed),
            0,
            "a dropped stream closes"
        );
        let other_rate = Path::new("p8_imp48k.wav");
        let Ok(ClipSound::Memory(buffer)) = streaming.clip(other_rate, hint()) else {
            panic!("offline a 48 kHz file loads converted into memory");
        };
        assert_eq!(buffer.frames(), 11_025);
        assert!(streaming.take_notices().is_empty());
    }

    // Live, a channel's file is decoded on the loader thread, never by the compile that asks
    // for it: the first ask gives nothing yet and the file shows as loading until it is in,
    // converted to the project's rate (12000 frames at 48 kHz are 11025 at 44.1 kHz). A short
    // clip of the same file then shares that buffer at once.
    #[test]
    fn live_a_channels_file_loads_in_the_background() {
        let playback = Playback {
            stream_threshold: None,
            waiting: Waiting::Never,
            convert_dir: None,
            stretch_dir: None,
            edited_dir: None,
        };
        let mut sounds = MediaSounds::new(&fixtures(), 44_100, playback);
        let file = Path::new("p8_imp48k.wav");
        assert!(matches!(sounds.sample(file), Ok(None)), "not decoded here");
        assert!(sounds.is_loading(file) && sounds.any_loading());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while sounds.take_loaded().is_empty() {
            assert!(std::time::Instant::now() < deadline, "the file loads");
            thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(!sounds.is_loading(file));
        let Ok(Some(buffer)) = sounds.sample(file) else {
            panic!("the loaded file is there");
        };
        assert_eq!(buffer.frames(), 11_025);
        assert!(matches!(
            sounds.clip(file, hint()),
            Ok(ClipSound::Memory(_))
        ));
        assert!(sounds.take_notices().is_empty());
    }

    // A preview of a short file is decoded, of a long file at the project's rate streamed, and
    // of a long file at another rate converted, which is the slow one.
    #[test]
    fn previews_decode_short_files_and_stream_long_ones() {
        let saw = fixtures().join("p8_saw.wav");
        let (short, notices) = preview(&saw, 44_100, 44_100);
        assert_eq!((short.frames(), notices.len()), (30_000, 0));
        assert!(
            preview_kind(&saw, 44_100, 0) != PreviewKind::Convert,
            "at the project's rate it streams"
        );
        let (streamed, _) = preview(&saw, 44_100, 0);
        assert_eq!(streamed.frames(), 30_000);
        let other = fixtures().join("p8_imp48k.wav");
        assert!(preview_kind(&other, 44_100, 0) == PreviewKind::Convert);
        assert_eq!(preview(&other, 44_100, 0).0.frames(), 11_025);
        let (missing, notices) = preview(&fixtures().join("no-such.wav"), 44_100, 0);
        assert_eq!((missing.frames(), notices.len()), (0, 1));
    }

    // Live, a long file at another rate is converted once on a worker into the
    // cache folder, and the next ask streams the converted file.
    #[test]
    fn live_a_long_file_at_another_rate_is_converted_then_streamed() {
        let cache = std::env::temp_dir().join(format!("dawsome-convert-{}", std::process::id()));
        let _ = fs::remove_dir_all(&cache);
        let playback = Playback {
            stream_threshold: Some(0),
            waiting: Waiting::Never,
            convert_dir: Some(cache.clone()),
            stretch_dir: None,
            edited_dir: None,
        };
        let mut sounds = MediaSounds::new(&fixtures(), 44_100, playback);
        let file = Path::new("p8_imp48k.wav");
        assert!(matches!(sounds.clip(file, hint()), Ok(ClipSound::Loading)));
        assert!(sounds.is_loading(file));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while sounds.take_loaded().is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "the conversion finishes"
            );
            thread::sleep(std::time::Duration::from_millis(5));
        }
        let Ok(ClipSound::Stream(source)) = sounds.clip(file, hint()) else {
            panic!("the converted file streams");
        };
        assert_eq!(source.frames(), 11_025);
        let written: Vec<_> = fs::read_dir(&cache).expect("cache").collect();
        assert_eq!(written.len(), 1, "one converted file, no partial left");
        drop(source);
        let _ = fs::remove_dir_all(&cache);
    }

    // Live playback loads a set's sounds on the loader thread: until the last of layers' seven
    // is in, the set is not given and its progress counts them; then it is built, complete,
    // and a project that plays it keeps its sounds in memory across compiles.
    #[test]
    fn a_set_loads_live_in_the_background_and_counts_its_sounds() {
        let playback = Playback {
            stream_threshold: None,
            waiting: Waiting::Never,
            convert_dir: None,
            stretch_dir: None,
            edited_dir: None,
        };
        let mut sounds = MediaSounds::new(&fixtures(), 44_100, playback);
        let set = Path::new("instruments/layers/layers.sfz");
        assert!(matches!(sounds.instrument(set), Ok(None)), "loading");
        assert_eq!(sounds.instrument_progress(), Some((0, 7)));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let built = loop {
            sounds.take_loaded();
            if let Ok(Some(built)) = sounds.instrument(set) {
                break built;
            }
            assert!(std::time::Instant::now() < deadline, "the set loads");
            thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(built.zones(), 7);
        assert_eq!(sounds.instrument_progress(), None);
        let report = sounds.report(set).expect("reported");
        assert!(report.is_complete(), "{report:?}");
        assert_eq!(report.zones, 7);
        let project = Project::load_json(
            &fs::read_to_string(fixtures().join("p11_layers.json")).expect("fixture"),
        )
        .expect("loads");
        sounds.forget_unused(&project);
        assert!(
            sounds
                .memory(Path::new("instruments/layers/a_soft.wav"))
                .is_some(),
            "kept"
        );
        sounds.forget_unused(&Project::new(
            44_100,
            dawsome_engine::time::MilliBpm(120_000),
            96,
        ));
        assert!(sounds.report(set).is_none(), "let go once nothing plays it");
    }

    // A library laid out lib/Instruments/a.sfz + lib/Samples/: read as its own folder's set,
    // both regions (../Samples/) are left out and the set has no zones; once lib/ is a folder
    // added to the Library the set read before is let go and read again, and both play. Folders
    // that leave its limit as it was read let nothing go.
    #[test]
    fn a_set_in_a_folder_added_plays_its_sibling_samples() {
        let root = std::env::temp_dir().join(format!("dawsome-library-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let lib = root.join("lib");
        fs::create_dir_all(lib.join("Instruments")).expect("dir");
        fs::create_dir_all(lib.join("Samples")).expect("dir");
        fs::copy(fixtures().join("impulse.wav"), lib.join("Samples/a.wav")).expect("copies");
        fs::write(
            lib.join("Instruments/a.sfz"),
            "<region> sample=../Samples/a.wav lokey=0 hikey=59\n\
             <region> sample=../Samples/a.wav lokey=60 hikey=127\n",
        )
        .expect("written");
        let set = lib.join("Instruments/a.sfz");
        let mut sounds = MediaSounds::new(&root, 44_100, Playback::offline(None));
        let own = sounds.instrument(&set).expect("infallible").expect("built");
        assert_eq!(own.zones(), 0);
        assert_eq!(sounds.report(&set).expect("reported").problems.len(), 2);
        assert!(sounds.set_library(Vec::new()).is_empty(), "nothing changes");
        assert_eq!(
            sounds.set_library(vec![lib.clone()]),
            std::slice::from_ref(&set),
            "read again"
        );
        assert!(sounds.report(&set).is_none(), "let go");
        let added = sounds.instrument(&set).expect("infallible").expect("built");
        assert_eq!(added.zones(), 2);
        assert!(sounds.report(&set).expect("reported").is_complete());
        assert!(
            sounds.set_library(vec![lib.clone()]).is_empty(),
            "the same folders"
        );
        assert!(
            sounds
                .set_library(vec![lib.join("Instruments"), lib.clone()])
                .is_empty(),
            "a folder added inside lib/ changes nothing"
        );
        assert_eq!(
            sounds.set_library(Vec::new()),
            [set],
            "lib/ removed: read again"
        );
        let _ = fs::remove_dir_all(&root);
    }

    // The filesystem's root added to the Library widens no set: a set anywhere still reads
    // inside its own folder, so the file its include names one folder up stays out of its
    // report, and its own sound plays.
    #[test]
    fn the_root_added_to_the_library_widens_no_set() {
        let root = std::env::temp_dir().join(format!("dawsome-root-added-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let pack = root.join("pack");
        fs::create_dir_all(&pack).expect("dir");
        fs::copy(fixtures().join("impulse.wav"), pack.join("a.wav")).expect("copies");
        fs::write(root.join("token.txt"), "API_TOKEN=hunter2\n").expect("written");
        let set = pack.join("x.sfz");
        fs::write(&set, "#include \"../token.txt\"\n<region> sample=a.wav\n").expect("written");
        let mut sounds = MediaSounds::new(&root, 44_100, Playback::offline(None));
        sounds.set_library(vec![PathBuf::from("/")]);
        let built = sounds.instrument(&set).expect("infallible").expect("built");
        assert_eq!(built.zones(), 1);
        assert_eq!(
            sounds.report(&set).expect("reported").problems,
            ["x.sfz:1: #include \"../token.txt\": outside the set's folder"]
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// Polls `sounds` until `path` has landed (loaded, edited), or fails after 20 s.
    fn wait_for(sounds: &mut MediaSounds, path: &Path) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !sounds.take_loaded().iter().any(|done| done == path) {
            assert!(
                std::time::Instant::now() < deadline,
                "{} lands",
                path.display()
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn live(stream_threshold: Option<u64>, edited_dir: Option<PathBuf>) -> Playback {
        Playback {
            stream_threshold,
            waiting: Waiting::Never,
            convert_dir: None,
            stretch_dir: None,
            edited_dir,
        }
    }

    // Live, one op past a short sound held in memory (the file, for a first op) is applied on
    // this thread at once; a list that is not is made on the worker while the channel plays, for
    // now, the edited sound made last, and once in it is kept.
    #[test]
    fn live_an_edited_sound_is_made_on_the_worker_the_last_one_playing_meanwhile() {
        use dawsome_engine::dsp::edits::apply;
        use dawsome_engine::project::SoundOp;
        let mut sounds = MediaSounds::new(&fixtures(), 44_100, live(None, None));
        let file = Path::new("p30_sound/ramp64.wav");
        assert!(
            matches!(sounds.sample(file), Ok(None)),
            "loads in the background"
        );
        wait_for(&mut sounds, file);
        let raw = sounds.sample(file).expect("infallible").expect("in memory");
        let reverse = SoundEdits {
            ops: vec![SoundOp::Reverse { from: 0, to: 64 }],
            source: None,
        };
        let got = sounds
            .edited_sample(file, &reverse)
            .expect("infallible")
            .expect("made at once");
        assert!(!got.for_now && !sounds.is_loading(file));
        let reversed = apply(&raw, &reverse);
        assert_eq!(got.sound.samples(), reversed.samples());
        let other = SoundEdits {
            ops: vec![
                SoundOp::Gain {
                    from: 0,
                    to: 64,
                    centi_db: -600,
                },
                SoundOp::FadeIn { from: 0, to: 32 },
            ],
            source: None,
        };
        let meanwhile = sounds
            .edited_sample(file, &other)
            .expect("infallible")
            .expect("a sound for now");
        assert!(meanwhile.for_now && sounds.is_loading(file) && sounds.any_loading());
        assert_eq!(
            meanwhile.sound.samples(),
            reversed.samples(),
            "the last made"
        );
        wait_for(&mut sounds, file);
        let got = sounds
            .edited_sample(file, &other)
            .expect("infallible")
            .expect("made");
        assert!(!got.for_now);
        assert_eq!(got.sound.samples(), apply(&raw, &other).samples());
        assert!(!sounds.is_loading(file));
    }

    // Live, a long edited clip (past the stream threshold) is made on the worker into the edited
    // cache while the clip streams the file as it is, then streams the cached edited sound; the
    // project dropping the channel lets both go.
    #[test]
    fn live_a_long_edited_clip_streams_from_the_edited_cache() {
        use dawsome_engine::project::SoundOp;
        let dir = std::env::temp_dir().join(format!("dawsome-edited-live-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let cache = dir.join("edited");
        let mut sounds = MediaSounds::new(&fixtures(), 44_100, live(Some(16), Some(cache.clone())));
        let file = Path::new("p30_sound/ramp64.wav");
        let edits = SoundEdits {
            ops: vec![SoundOp::FadeOut { from: 0, to: 64 }],
            source: None,
        };
        let hint = ClipHint {
            first_frame: 0,
            loop_frame: None,
            stretch: None,
        };
        let meanwhile = sounds.edited_clip(file, &edits, hint).expect("infallible");
        assert!(meanwhile.for_now);
        assert!(
            matches!(meanwhile.sound, ClipSound::Stream(_)),
            "the file streams"
        );
        wait_for(&mut sounds, file);
        let got = sounds.edited_clip(file, &edits, hint).expect("infallible");
        assert!(!got.for_now);
        let ClipSound::Stream(stream) = got.sound else {
            panic!("the edited sound streams");
        };
        assert_eq!(stream.frames(), 64);
        let written: Vec<PathBuf> = fs::read_dir(&cache)
            .expect("the edited cache")
            .map(|entry| entry.expect("an entry").path())
            .collect();
        assert_eq!(
            written,
            [cache.join(edited::cache_name(&fixtures().join(file), 44_100, &edits))]
        );
        sounds.forget_unused(&Project::new(
            44_100,
            dawsome_engine::time::MilliBpm(120_000),
            96,
        ));
        assert!(sounds.edited.is_empty() && sounds.last_edited.is_empty());
        drop(stream);
        let _ = fs::remove_dir_all(&dir);
    }

    // The sweep's rule follows the writer's names: every temporary file `write_float_frames`
    // names for a WAV is one `remove_stale_partials` takes once it is idle; the WAV it becomes,
    // and the names around it, are not.
    #[test]
    fn the_sweep_knows_the_writers_temporary_names() {
        for target in ["/x/ramp (edited).wav", "/x/ramp (edited 2).wav", "a.wav"] {
            let partial = partial_path(Path::new(target));
            let name = partial.file_name().and_then(|name| name.to_str());
            assert!(name.is_some_and(is_partial_wav), "{partial:?}");
            let wav = Path::new(target).file_name().and_then(|name| name.to_str());
            assert!(!wav.is_some_and(is_partial_wav), "{target}");
        }
        for other in [
            "a.partial",
            "a.wav.1-2",
            "a.wav.x-2.partial",
            "a.mp3.1-2.partial",
            ".wav.1-2.partial",
        ] {
            assert!(!is_partial_wav(other), "{other}");
        }
    }
}
