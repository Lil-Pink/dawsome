//! The document's life on disk: New, Open, Save, Save As, the unsaved-changes question before anything that would lose changes, autosave and
//! recovering one, the recent files, and closing the window.

use std::borrow::Cow;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use dawsome_engine::edit::StateId;
use dawsome_engine::project::{ChannelId, ChannelKind, LoadNote, Project};
use dawsome_media::export::Report;
use dawsome_media::InstrumentReport;
use iced::Task;

use super::browse::MediaMessage;
use super::export::{ExportKind, ExportMessage, ExportState};
use super::{App, Message, PlayMode};
use crate::audio::Output;
use crate::demo;
use crate::document::{AudioChoice, Document};
use crate::project_file::{self, Opened};
use crate::session::Session;
use crate::starter_kit;
use crate::ui::arranger::Arranger;
use crate::ui::mixer_state::MixerState;
use crate::ui::roll::Roll;

/// How often unsaved changes are written to the autosave.
pub const AUTOSAVE_PERIOD: Duration = Duration::from_secs(120);

/// Everything the File menu, the file dialogs and the file questions report.
#[derive(Debug, Clone)]
pub enum FileMessage {
    /// The toolbar's File button.
    ToggleMenu,
    CloseMenu,
    New,
    /// File → New empty project.
    NewEmpty,
    Open,
    /// File → Open demo.
    OpenDemo,
    OpenRecent(PathBuf),
    Save,
    SaveAs,
    Export(ExportKind),
    /// The Open dialog's answer; `None` when it was canceled.
    OpenPicked(Option<PathBuf>),
    /// The Save As dialog's answer.
    SavePicked(Option<PathBuf>),
    /// The answer to "Save changes?".
    Unsaved(Choice),
    /// Recover the autosave (`true`), or open what is on file.
    Recover(bool),
    CloseReport,
    /// The window's close button.
    CloseRequested,
    /// The main window is gone without asking (the system closed it): the app closes now, the
    /// changes kept by the autosave's recovery, since no dialog can be shown.
    MainWindowClosed,
    AutosaveTick,
    /// A background autosave of `state` finished.
    Autosaved {
        state: StateId,
        result: Result<(), String>,
    },
    ExportForm(ExportMessage),
    /// File → Import MIDI…
    ImportMidi,
    /// The MIDI file the import picker picked; `None` when it was canceled.
    MidiPicked(Option<PathBuf>),
    /// File → Export pattern as MIDI…
    ExportMidi,
    /// Where the MIDI export's Save dialog says to write; `None` when it was canceled.
    MidiExportPicked(Option<PathBuf>),
    /// File → Export song as MIDI…
    ExportSongMidi,
    /// Where the song's MIDI file goes; `None` when the dialog was canceled.
    SongMidiPicked(Option<PathBuf>),
    /// File → Export DAWproject…
    ExportDawproject,
    /// Where the DAWproject file goes; `None` when the dialog was canceled.
    DawprojectPicked(Option<PathBuf>),
    /// A DAWproject export written in the background: its report, or why it failed.
    DawprojectWritten(Result<Report, String>),
}

impl FileMessage {
    /// Whether this is something the user did, which closes the File menu (a timer or a
    /// finished background write does not).
    fn closes_menu(&self) -> bool {
        !matches!(
            self,
            FileMessage::ToggleMenu
                | FileMessage::AutosaveTick
                | FileMessage::Autosaved { .. }
                | FileMessage::ExportForm(ExportMessage::Poll)
                | FileMessage::DawprojectWritten(_)
        )
    }

    /// Whether this answers the dialog on screen (or finishes work it started), rather than
    /// starting something new under it.
    fn answers_dialog(&self) -> bool {
        matches!(
            self,
            FileMessage::OpenPicked(_)
                | FileMessage::SavePicked(_)
                | FileMessage::MidiPicked(_)
                | FileMessage::MidiExportPicked(_)
                | FileMessage::SongMidiPicked(_)
                | FileMessage::DawprojectPicked(_)
                | FileMessage::DawprojectWritten(_)
                | FileMessage::Unsaved(_)
                | FileMessage::Recover(_)
                | FileMessage::CloseReport
                | FileMessage::CloseRequested
                | FileMessage::MainWindowClosed
                | FileMessage::AutosaveTick
                | FileMessage::Autosaved { .. }
                | FileMessage::ExportForm(_)
        )
    }
}

/// The three answers to "Save changes to <name>?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Save,
    DontSave,
    Cancel,
}

/// What waits for the document to be saved or let go of.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    New,
    /// File → New empty project: one silent sampler, as File → New made before the starter
    /// document.
    NewEmpty,
    Open,
    OpenPath(PathBuf),
    Close,
}

/// The dialog over the window.
pub(super) enum Modal {
    Unsaved,
    Recover {
        /// The file the autosave belongs to; `None` for a document that had none.
        file: Option<PathBuf>,
        autosave: PathBuf,
        age: String,
    },
    Report {
        summary: String,
        groups: Vec<(&'static str, Vec<String>)>,
    },
    Export(ExportState),
}

/// The window's file state.
#[derive(Default)]
pub(super) struct Files {
    pub(super) menu: bool,
    pub(super) modal: Option<Modal>,
    pending: Option<Pending>,
    /// The document state the last autosave holds.
    autosaved: Option<StateId>,
    /// How many times the autosave was taken away; held while an autosave writes and while one
    /// is removed. Why: a background write landing after Save removed the autosave would leave
    /// a stale one behind, offered as a recovery the next time the file opens. A write started
    /// before the count changed stands down instead.
    autosave_removals: Arc<Mutex<u64>>,
    /// Set once the window is to close.
    pub(super) exiting: bool,
}

impl Files {
    /// Whether `message` may reach the app while a dialog is shown.
    pub(super) fn lets_through(&self, message: &Message) -> bool {
        match message {
            _ if self.modal.is_none() => true,
            Message::File(file) => file.answers_dialog(),
            Message::Tick
            | Message::AudioCheck
            | Message::RetryAudio
            | Message::Headless(_)
            | Message::ScaleFactor(_)
            | Message::MonitorSized(_)
            | Message::OutputsListed(_)
            | Message::InputsListed(_)
            | Message::WindowResized(_)
            | Message::WindowMaximized(_)
            | Message::ModifiersChanged(_)
            | Message::ClearSelection
            | Message::WindowUnfocused
            // A key let go of under a dialog still ends its note.
            | Message::TypingKey { pressed: false, .. }
            // The Help window's pages, which change nothing of the document, read on under a
            // dialog of the main window.
            | Message::Help(_)
            | Message::Media(
                MediaMessage::Listed { .. } | MediaMessage::Indexed(_) | MediaMessage::Tick,
            ) => true,
            _ => false,
        }
    }
}

/// What a document is opened as.
pub struct Start {
    pub document: Document,
}

/// The autosave removal count. A writer that panicked holding it left the count as it was.
fn lock_removals(removals: &Mutex<u64>) -> MutexGuard<'_, u64> {
    removals.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs `work` on a thread of its own and resolves to its result; `None` if the thread died.
/// Why a thread: a file write or a dialog waits on the disk or on the user, and the UI's
/// executor must not.
pub(super) fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> impl Future<Output = Option<T>> {
    joinable(work).1
}

/// As [`blocking`], with the thread's handle, for work the app's quit waits for: a write the
/// process ending under it would leave half done.
pub(super) fn joinable<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> (std::thread::JoinHandle<()>, impl Future<Output = Option<T>>) {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let thread = std::thread::spawn(move || {
        let _ = sender.send(work());
    });
    (thread, async move { receiver.await.ok() })
}

/// The Open dialog's filter: the Dawsome projects under either extension (`.dawsome`, and
/// `.json` as projects were saved before it).
pub(super) const OPEN_FILTERS: [(&str, &[&str]); 1] = [(
    "Dawsome projects (*.dawsome, *.json)",
    &project_file::PROJECT_EXTENSIONS,
)];

/// The Save dialog's filter for a project.
pub(super) const SAVE_FILTER: (&str, &str) =
    ("Dawsome project (*.dawsome)", project_file::EXTENSION);

/// The desktop's Open dialog, offering Dawsome projects.
async fn pick_open(dir: Option<PathBuf>) -> Option<PathBuf> {
    let mut dialog = rfd::AsyncFileDialog::new().set_title("Open");
    for (name, extensions) in OPEN_FILTERS {
        dialog = dialog.add_filter(name, extensions);
    }
    if let Some(dir) = dir {
        dialog = dialog.set_directory(dir);
    }
    dialog
        .pick_file()
        .await
        .map(|file| file.path().to_path_buf())
}

/// The desktop's Save dialog for one kind of file (`filter`: its name and extension),
/// suggesting `name` in `dir`.
pub(super) async fn pick_save(
    title: &'static str,
    dir: Option<PathBuf>,
    name: String,
    filter: (&'static str, &'static str),
) -> Option<PathBuf> {
    let mut dialog = rfd::AsyncFileDialog::new()
        .set_title(title)
        .set_file_name(name)
        .add_filter(filter.0, &[filter.1]);
    if let Some(dir) = dir {
        dialog = dialog.set_directory(dir);
    }
    dialog
        .save_file()
        .await
        .map(|file| file.path().to_path_buf())
}

/// The desktop's folder dialog, opening in `dir`.
pub(super) async fn pick_folder(title: &'static str, dir: Option<PathBuf>) -> Option<PathBuf> {
    let mut dialog = rfd::AsyncFileDialog::new().set_title(title);
    if let Some(dir) = dir {
        dialog = dialog.set_directory(dir);
    }
    dialog
        .pick_folder()
        .await
        .map(|folder| folder.path().to_path_buf())
}

/// `path` with `.extension` added when it has no extension.
pub(super) fn with_extension(path: &Path, extension: &str) -> PathBuf {
    if path.extension().is_some() {
        path.to_path_buf()
    } else {
        path.with_extension(extension)
    }
}

/// When the autosave beside the Dawsome document at `path` is newer than it: how long ago it
/// was written.
fn newer_autosave(path: &Path) -> Option<String> {
    let modified = |path: &Path| fs::metadata(path).and_then(|meta| meta.modified()).ok();
    let autosave = modified(&project_file::autosave_beside(path))?;
    match modified(path) {
        Some(saved) if saved >= autosave => None,
        _ => Some(age(autosave)),
    }
}

/// "3 minutes ago" for a file written at `time`.
fn age(time: SystemTime) -> String {
    let seconds = SystemTime::now()
        .duration_since(time)
        .map_or(0, |elapsed| elapsed.as_secs());
    let (count, unit) = match seconds {
        0..60 => return "less than a minute ago".to_owned(),
        60..3600 => (seconds / 60, "minute"),
        3600..86_400 => (seconds / 3600, "hour"),
        _ => (seconds / 86_400, "day"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

/// One line for the toolbar about the channels' sounds (or what opening left out of the
/// document), naming the first.
pub(super) fn summarize_notices(notices: &[impl std::fmt::Display]) -> Option<String> {
    let first = notices.first()?;
    Some(match notices.len() {
        1 => first.to_string(),
        count => format!("{first} (and {} more)", count - 1),
    })
}

/// An instrument set's load report as the dialog shows it: how many zones play, then what was
/// left out (each opcode or header once, how often, and where first) and what could not be
/// read.
fn instrument_modal(name: &str, report: &InstrumentReport) -> Modal {
    let zones = match report.zones {
        1 => "1 sound plays".to_owned(),
        count => format!("{count} sounds play"),
    };
    let summary = format!("{name}: {zones}; some of the set is left out");
    let mut groups = Vec::new();
    if !report.ignored.is_empty() {
        let lines = report
            .ignored
            .iter()
            .map(|ignored| {
                let times = if ignored.count == 1 { "once" } else { "times" };
                let count = if ignored.count == 1 {
                    String::new()
                } else {
                    format!("{} ", ignored.count)
                };
                format!(
                    "{}: {count}{times}, first at {}",
                    ignored.name, ignored.first
                )
            })
            .collect();
        groups.push(("Not played by this reader", lines));
    }
    if !report.problems.is_empty() {
        groups.push(("Could not be read", report.problems.clone()));
    }
    Modal::Report { summary, groups }
}

impl App {
    /// A double press on an instrument channel's name: its set's load report, when the set
    /// left something out. Returns whether a report was shown (then nothing else opens).
    pub(super) fn show_instrument_report(&mut self, channel: ChannelId) -> bool {
        let project = self.session.project();
        let Some(channel) = project
            .channels
            .get(&channel)
            .filter(|channel| channel.kind == ChannelKind::Instrument)
        else {
            return false;
        };
        let report = self
            .session
            .instrument_report(&channel.instrument)
            .filter(|report| !report.is_complete());
        let Some(report) = report else {
            return false;
        };
        self.files.modal = Some(instrument_modal(&channel.name, report));
        true
    }

    pub(super) fn file_message(&mut self, message: FileMessage) -> Task<Message> {
        if message.closes_menu() {
            self.files.menu = false;
        }
        match message {
            FileMessage::ToggleMenu => {
                self.files.menu = !self.files.menu;
                Task::none()
            }
            FileMessage::CloseMenu => Task::none(),
            FileMessage::New => self.request(Pending::New),
            FileMessage::NewEmpty => self.request(Pending::NewEmpty),
            FileMessage::Open => self.request(Pending::Open),
            FileMessage::OpenDemo => match self.install_demo() {
                Some(demo) => self.request(Pending::OpenPath(demo)),
                None => Task::none(),
            },
            FileMessage::OpenRecent(path) => {
                if path.exists() {
                    return self.request(Pending::OpenPath(path));
                }
                let result = self.recent.remove(&path);
                self.notice = Some(format!(
                    "{} is gone; it was taken off the recent files",
                    path.display()
                ));
                self.report_save(result);
                Task::none()
            }
            FileMessage::Save => self.save(),
            FileMessage::SaveAs => self.save_as(),
            FileMessage::Export(kind) => {
                self.open_export(kind);
                Task::none()
            }
            FileMessage::OpenPicked(Some(path)) => self.open_path(&path),
            FileMessage::OpenPicked(None) => Task::none(),
            FileMessage::SavePicked(Some(path)) => self.save_to(&project_file::project_path(&path)),
            FileMessage::SavePicked(None) => {
                // Canceling the name cancels what was waiting for the save.
                self.files.pending = None;
                Task::none()
            }
            FileMessage::Unsaved(choice) => self.answer_unsaved(choice),
            FileMessage::Recover(recover) => self.answer_recover(recover),
            FileMessage::CloseReport => {
                self.files.modal = None;
                Task::none()
            }
            FileMessage::CloseRequested => {
                // Closing stops a running export, whatever the answer about unsaved changes.
                self.abandon_export();
                self.request(Pending::Close)
            }
            FileMessage::MainWindowClosed => {
                self.abandon_export();
                self.files.modal = None;
                // Unsaved changes are written to the autosave now, for the next start to offer.
                if let Some((state, write)) = self.autosave_job() {
                    let written = write();
                    self.autosaved(state, written);
                }
                self.perform(Pending::Close)
            }
            FileMessage::AutosaveTick => self.autosave(),
            FileMessage::Autosaved { state, result } => {
                self.autosaved(state, result);
                Task::none()
            }
            FileMessage::ExportForm(message) => self.export_message(message),
            FileMessage::ImportMidi => self.import_midi(),
            FileMessage::MidiPicked(Some(path)) => {
                self.midi_picked(&path);
                Task::none()
            }
            FileMessage::ExportMidi => self.export_midi(),
            FileMessage::MidiExportPicked(Some(path)) => {
                self.midi_export_picked(&path);
                Task::none()
            }
            FileMessage::ExportSongMidi => self.export_song_midi(),
            FileMessage::SongMidiPicked(Some(path)) => {
                self.song_midi_picked(&path);
                Task::none()
            }
            FileMessage::ExportDawproject => self.export_dawproject(),
            FileMessage::DawprojectPicked(Some(path)) => self.dawproject_picked(&path),
            FileMessage::DawprojectWritten(result) => {
                self.dawproject_written(result);
                Task::none()
            }
            FileMessage::MidiPicked(None)
            | FileMessage::MidiExportPicked(None)
            | FileMessage::SongMidiPicked(None)
            | FileMessage::DawprojectPicked(None) => Task::none(),
        }
    }

    /// The window's title: the document's name, starred while it has unsaved changes, and how
    /// many of their sounds are in while instrument sets load.
    pub fn title(&self) -> String {
        let title = self.document.title(self.session.is_dirty());
        match self.session.instrument_progress() {
            Some((done, all)) => format!("{title} (loading instruments: {done} of {all} sounds)"),
            None => title,
        }
    }

    /// Esc on a dialog: Cancel where it has one; the report closes.
    pub(super) fn escape_dialog(&mut self) -> Task<Message> {
        match &self.files.modal {
            Some(Modal::Unsaved) => self.answer_unsaved(Choice::Cancel),
            Some(Modal::Report { .. }) => {
                self.files.modal = None;
                Task::none()
            }
            Some(Modal::Export(_)) => self.export_message(ExportMessage::Cancel),
            Some(Modal::Recover { .. }) | None => Task::none(),
        }
    }

    /// Runs `pending` now, or first asks to save when there are unsaved changes.
    fn request(&mut self, pending: Pending) -> Task<Message> {
        if self.session.is_dirty() {
            self.files.pending = Some(pending);
            self.files.modal = Some(Modal::Unsaved);
            return Task::none();
        }
        self.perform(pending)
    }

    fn perform(&mut self, pending: Pending) -> Task<Message> {
        match pending {
            Pending::NewEmpty => {
                let opened = Opened {
                    project: self.at_new_rate(project_file::new_document()),
                    samples_dir: PathBuf::new(),
                };
                self.install(opened, Document::untitled());
                Task::none()
            }
            Pending::New => {
                let first = project_file::first_document(self.host.starter_kit.as_deref());
                let opened = Opened {
                    project: self.at_new_rate(first),
                    samples_dir: PathBuf::new(),
                };
                self.install(opened, Document::untitled());
                Task::none()
            }
            Pending::Open => {
                if !self.host.background {
                    return Task::none();
                }
                let dir = self.document_dir();
                Task::perform(pick_open(dir), |path| {
                    Message::File(FileMessage::OpenPicked(path))
                })
            }
            Pending::OpenPath(path) => self.open_path(&path),
            Pending::Close => {
                // A sound the sample editor saves lands whole first, its button saying
                // "Saving…" meanwhile, so no temporary file of it stays beside the project.
                self.finish_sound_saves();
                // A measurement running leaves no files in the temporary folder.
                self.cancel_latency();
                self.files.exiting = true;
                self.save_window_layout();
                self.settle_controllers();
                self.release_plugins();
                self.quit_midi_out();
                iced::exit()
            }
        }
    }

    fn answer_unsaved(&mut self, choice: Choice) -> Task<Message> {
        self.files.modal = None;
        match choice {
            Choice::Save => self.save(),
            Choice::DontSave => {
                let Some(pending) = self.files.pending.take() else {
                    return Task::none();
                };
                if pending == Pending::Close {
                    // Closing without saving lets the changes go, autosave included.
                    self.remove_autosaves(self.autosave_target());
                }
                self.perform(pending)
            }
            Choice::Cancel => {
                self.files.pending = None;
                Task::none()
            }
        }
    }

    /// Save: to the document's file, or by Save As when it has none (a new one).
    fn save(&mut self) -> Task<Message> {
        match self.document.path.clone() {
            Some(path) => self.save_to(&path),
            None => self.save_as(),
        }
    }

    fn save_as(&mut self) -> Task<Message> {
        if !self.host.background {
            return Task::none();
        }
        let dialog = pick_save(
            "Save As",
            self.document_dir(),
            self.document.save_as_name(),
            SAVE_FILTER,
        );
        Task::perform(dialog, |path| Message::File(FileMessage::SavePicked(path)))
    }

    /// Writes the document to `path` atomically; then it is clean, named after `path`, on the
    /// recent list, and its autosaves are gone. A failure is shown and keeps the old file.
    fn save_to(&mut self, path: &Path) -> Task<Message> {
        // A take running is kept first, so the file holds it and its undo step is behind the
        // saved state.
        self.keep_take("Take kept before saving");
        // A burst of controller moves ends before the document is marked saved, so its undo
        // step cannot make the saved document look changed.
        self.settle_controllers();
        self.capture_big_states();
        let session = &self.session;
        let written = project_file::json_for(session.project(), session.samples_dir(), path)
            .and_then(|json| project_file::write_atomically(path, json.as_bytes()));
        if let Err(error) = written {
            self.error = Some(error.to_string());
            self.files.pending = None;
            return Task::none();
        }
        let old_autosave = self.autosave_target();
        self.session.mark_saved();
        self.document = Document::saved_at(path);
        self.show_project_folder();
        self.remove_autosaves([old_autosave, self.autosave_target()].into_iter().flatten());
        self.files.autosaved = None;
        self.error = None;
        self.remember_opened(path);
        match self.files.pending.take() {
            Some(pending) => self.perform(pending),
            None => Task::none(),
        }
    }

    /// Opens `path`, unless a newer autosave of it is waiting: then asks which to open.
    fn open_path(&mut self, path: &Path) -> Task<Message> {
        if self.host.autosaves() {
            if let Some(age) = newer_autosave(path) {
                self.files.modal = Some(Modal::Recover {
                    file: Some(path.to_path_buf()),
                    autosave: project_file::autosave_beside(path),
                    age,
                });
                return Task::none();
            }
        }
        self.load(path);
        Task::none()
    }

    /// Opens `path` as the document; a failure is shown and the current document stays.
    fn load(&mut self, path: &Path) {
        match project_file::open_noting(path) {
            Ok((opened, notes)) => {
                if self.install(opened, Document::saved_at(path)) {
                    self.show_load_notes(&notes);
                    self.remember_opened(path);
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn answer_recover(&mut self, recover: bool) -> Task<Message> {
        let Some(Modal::Recover { file, autosave, .. }) = self.files.modal.take() else {
            return Task::none();
        };
        if !recover {
            match file {
                Some(file) => self.load(&file),
                // A leftover with no file of its own: discarding it means it is never asked
                // about again.
                None => {
                    let _ = fs::remove_file(&autosave);
                }
            }
            return Task::none();
        }
        match project_file::open_noting(&autosave) {
            Ok((opened, notes)) => {
                let document = file
                    .as_deref()
                    .map_or_else(Document::untitled, Document::saved_at);
                if self.install(opened, document) {
                    self.show_load_notes(&notes);
                    self.session.mark_unsaved();
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        Task::none()
    }

    /// Where the document autosaves: beside its file, or in the per-user autosave for one with
    /// none. `None` when autosave is off.
    fn autosave_target(&self) -> Option<PathBuf> {
        self.host.untitled_autosave.as_ref()?;
        match &self.document.path {
            Some(path) => Some(project_file::autosave_beside(path)),
            None => self.host.untitled_autosave.clone(),
        }
    }

    /// Removes the autosaves at `targets` once an autosave already writing has landed, so that
    /// one started before cannot land after them.
    fn remove_autosaves(&self, targets: impl IntoIterator<Item = PathBuf>) {
        let mut removals = lock_removals(&self.files.autosave_removals);
        *removals = removals.wrapping_add(1);
        for target in targets {
            let _ = fs::remove_file(target);
        }
    }

    /// Writes the document to its autosave when it has changes the autosave does not hold yet.
    /// Serialized here, written by background work.
    fn autosave(&mut self) -> Task<Message> {
        let Some((state, write)) = self.autosave_job() else {
            return Task::none();
        };
        if !self.host.background {
            self.autosaved(state, write());
            return Task::none();
        }
        Task::perform(blocking(write), move |result| {
            let result =
                result.unwrap_or_else(|| Err("the autosave stopped before it finished".to_owned()));
            Message::File(FileMessage::Autosaved { state, result })
        })
    }

    /// The state an autosave would hold and the work that writes it, when the document has
    /// changes the autosave does not hold yet.
    fn autosave_job(
        &mut self,
    ) -> Option<(
        StateId,
        impl FnOnce() -> Result<(), String> + Send + 'static,
    )> {
        // A big state captured now changes the document under the same state id (it takes no
        // undo step), so it is written though that state was autosaved before.
        let captured = self.capture_big_states();
        let state = self.session.state();
        if !self.session.is_dirty() || (self.files.autosaved == Some(state) && !captured) {
            return None;
        }
        let target = self.autosave_target()?;
        let session = &self.session;
        let json = match project_file::json_for(session.project(), session.samples_dir(), &target) {
            Ok(json) => json,
            Err(error) => {
                self.notice = Some(format!("Autosave failed: {error}"));
                return None;
            }
        };
        let removals = Arc::clone(&self.files.autosave_removals);
        let started = *lock_removals(&removals);
        let write = move || {
            let removed = lock_removals(&removals);
            if *removed != started {
                // Taken away since: those changes were saved or let go.
                return Ok(());
            }
            if let Some(dir) = target.parent() {
                fs::create_dir_all(dir).map_err(|error| error.to_string())?;
            }
            project_file::write_atomically(&target, json.as_bytes()).map_err(|e| e.to_string())
        };
        Some((state, write))
    }

    fn autosaved(&mut self, state: StateId, result: Result<(), String>) {
        match result {
            Ok(()) => self.files.autosaved = Some(state),
            Err(error) => self.notice = Some(format!("Autosave failed: {error}")),
        }
    }

    /// The demo's project file, put in the demo folder when it is not there yet; `None`, with
    /// the reason shown, when it cannot be.
    fn install_demo(&mut self) -> Option<PathBuf> {
        let Some(dir) = self.host.demo_dir.clone() else {
            self.error = Some("There is no folder to put the demo in".to_owned());
            return None;
        };
        match demo::install(&dir) {
            Ok(demo) => Some(demo),
            Err(error) => {
                self.error = Some(error.to_string());
                None
            }
        }
    }

    /// On a first launch, with no recent files yet, lists the demo in them, so there is a song
    /// to open from the start.
    pub(super) fn list_demo_on_first_launch(&mut self) {
        if !self.recent.entries().is_empty() || self.host.demo_dir.is_none() {
            return;
        }
        if let Some(demo) = self.install_demo() {
            self.remember_opened(&demo);
        }
    }

    /// Puts a document's file on the recent list.
    pub(super) fn remember_opened(&mut self, path: &Path) {
        let result = self.recent.add(path);
        self.report_save(result);
    }

    fn report_save(&mut self, result: Result<(), project_file::SaveError>) {
        if let Err(error) = result {
            self.notice = Some(format!("Recent files not saved: {error}"));
        }
    }

    /// The directory dialogs open in: the document's file's, else its samples'.
    pub(super) fn document_dir(&self) -> Option<PathBuf> {
        let dir = match &self.document.path {
            Some(path) => path.parent().map(Path::to_path_buf),
            None => Some(self.session.samples_dir().to_path_buf()),
        };
        dir.filter(|dir| !dir.as_os_str().is_empty())
    }

    /// Makes `opened` the window's document: a new session and player (the old stream stops
    /// before the new one opens the device), the views reset to it and its sound notices shown.
    /// A document whose playlist holds music lands on it
    /// in song mode ([`super::Landing`]). Returns false, with the reason shown and the old
    /// document kept, when the document cannot be played.
    pub(super) fn install(&mut self, opened: Opened, document: Document) -> bool {
        let Opened {
            project,
            samples_dir,
        } = opened;
        let rate = project.sample_rate;
        if let Err(error) = starter_kit::install_for(&project, self.host.starter_kit.as_deref()) {
            self.notice = Some(format!("The starter kit was not written: {error}"));
        }
        // A document whose playlist holds music opens on it, playing the song; one without
        // keeps the mode the window is in.
        let landing = super::Landing::of(&project);
        let song = landing.is_some() || self.mode == PlayMode::Song;
        let mode = Session::mode_for(&project, song);
        let playback = self.host.playback.clone();
        let library = self.browse.model.user_folders().folders;
        let opened = Session::open_with_library(
            project,
            samples_dir,
            mode,
            self.host.registry(),
            playback,
            library,
        );
        let (mut session, player) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                self.error = Some(error.to_string());
                return false;
            }
        };
        let notices = session.take_notices();
        // The window shows a channel of the document going: its held key ends with it.
        self.close_instrument();
        // The parameter window shows a plugin of the session going (another document's
        // channel 1 is another plugin, if any).
        self.close_params();
        // The sample editor shows a sound of the document going.
        self.close_sample_editor();
        // The next session has no preview.
        self.forget_library_preview();
        self.settle_controllers();
        self.controllers.learning = None;
        self.release_plugins();
        self.reset_recording();
        self.audio = None;
        self.session = session;
        // The new player holds no note.
        self.live_notes.clear();
        self.reconnect_ports();
        // The last document's MIDI outs end their notes; this one's set their programs.
        self.open_midi_out();
        self.start_audio(player, rate);
        if self.metronome {
            let result = self
                .session
                .send(dawsome_engine::transport::Control::Metronome(true));
            self.report(result);
        }
        self.document = document;
        self.sweep_partial_saves();
        self.files.autosaved = None;
        self.dropped_notes = 0;
        if landing.is_some() {
            self.mode = PlayMode::Song;
        }
        self.reset_views();
        if let Some(landing) = landing {
            self.land(landing);
        }
        self.notice = summarize_notices(&notices);
        self.error = None;
        true
    }

    /// Tells in the toolbar what reading the document left out of it, after what its sounds
    /// had to say.
    pub(crate) fn show_load_notes(&mut self, notes: &[LoadNote]) {
        let Some(said) = summarize_notices(notes) else {
            return;
        };
        self.notice = Some(match self.notice.take() {
            Some(sounds) => format!("{sounds}; {said}"),
            None => said,
        });
    }

    /// The questions a start can raise: a newer autosave of the opened file, or one left by a
    /// document that had no file.
    pub(super) fn startup_recovery(&mut self, start: &Start) {
        if !self.host.autosaves() {
            return;
        }
        let modal = match (&start.document.path, &self.host.untitled_autosave) {
            (Some(path), _) => newer_autosave(path).map(|age| Modal::Recover {
                file: Some(path.clone()),
                autosave: project_file::autosave_beside(path),
                age,
            }),
            (None, Some(untitled)) => fs::metadata(untitled)
                .and_then(|meta| meta.modified())
                .ok()
                .map(|time| Modal::Recover {
                    file: None,
                    autosave: untitled.clone(),
                    age: age(time),
                }),
            (None, None) => None,
        };
        if modal.is_some() {
            self.files.modal = modal;
        }
    }

    /// `project`, a new document, at the rate a new document takes now
    /// ([`project_file::new_document_rate`]): the own rate of the device it will play on, the
    /// one Settings chose: the open output's when that is the device, else the chosen device's
    /// asked without opening it, as the first window asks. Why not the open output's always:
    /// installing the document opens the chosen device, and a document at the old device's
    /// rate played resampled on the new one.
    fn at_new_rate(&self, mut project: Project) -> Project {
        let chosen = self.settings.audio_output();
        let own = match &self.audio {
            Some(output) if output.asked() == chosen => output.own_rate(),
            _ => self.device_rate(chosen),
        };
        project.sample_rate = project_file::new_document_rate(own);
        project
    }

    /// The rate the output device the settings keep as `chosen` (none: the system's default)
    /// plays at on its own, asked without opening a stream; `None` when it cannot say.
    fn device_rate(&self, chosen: Option<&str>) -> Option<u32> {
        match self.host.audio {
            AudioChoice::Device { rate: forced } => crate::audio::device_own_rate(forced, chosen),
            #[cfg(test)]
            AudioChoice::FakeRated(_, rate) => rate(chosen),
            #[cfg(test)]
            _ => None,
        }
    }

    /// Plays `player`, at `rate`, through the output device the settings chose; when it cannot
    /// open, the toolbar says why and the player is parked.
    ///
    /// The input channels' devices open again for it: the stream's clock and block are new.
    pub(super) fn start_audio(&mut self, player: dawsome_engine::realtime::Player, rate: u32) {
        self.open_output(player, rate);
        self.time_midi_out();
        self.sync_live_inputs(true);
    }

    fn open_output(&mut self, player: dawsome_engine::realtime::Player, rate: u32) {
        #[cfg(test)]
        if matches!(self.host.audio, AudioChoice::Off) {
            self.parked_player = Some(player);
            return;
        }
        match self.open_chosen(player, rate) {
            Ok(output) => self.install_output(output),
            Err(failure) => {
                self.audio_note = None;
                let problem = format!("Audio off: {}", failure.error);
                self.audio_problem = Some(Cow::Owned(problem));
                if let Some(player) = failure.player {
                    self.session.park(*player);
                }
            }
        }
    }

    /// Opens the output device the settings chose for `player` at `rate`: the system's (or a
    /// test's stand-ins), a device that is not there named as Settings showed it. Nothing else
    /// changes: the caller installs the stream, or parks the player handed back.
    pub(super) fn open_chosen(
        &self,
        player: dawsome_engine::realtime::Player,
        rate: u32,
    ) -> Result<Output, crate::audio::StartFailure> {
        let chosen = self.chosen_output();
        match self.host.audio {
            AudioChoice::Device { rate: forced } => Output::start(player, rate, forced, chosen),
            #[cfg(test)]
            AudioChoice::Fake(open) => open(player),
            #[cfg(test)]
            AudioChoice::FakeNamed(open) | AudioChoice::FakeRated(open, _) => {
                open(player, chosen.map(|chosen| chosen.id))
            }
            #[cfg(test)]
            AudioChoice::Off => Err(crate::audio::StartFailure {
                error: crate::audio::StartError::NoDevice,
                player: Some(Box::new(player)),
            }),
        }
    }

    /// Plays through `output` from now: its resampling note shown, the audio problem gone.
    pub(super) fn install_output(&mut self, output: Output) {
        self.audio_note = output.notice();
        self.audio_problem = None;
        self.audio = Some(output);
    }

    /// Every view back to the start for a new document, at the sizes the panels have now.
    fn reset_views(&mut self) {
        let project = self.session.project();
        let step = dawsome_engine::time::step_to_ticks(1, project.ppq);
        self.pattern = project.patterns.keys().next().copied();
        self.selected_channel = project.channels.keys().next().copied();
        // The lane's height is the user's, not the document's: it stays.
        self.roll =
            Roll::new(step, self.roll.grid_size()).with_lane_height(self.roll.lane_height());
        // The playlist's lanes and the mixer's height are the user's, not the document's: they
        // stay.
        let lanes = self.arranger.lanes();
        self.arranger = Arranger::new(project.ppq, self.arranger.tracks_size(), lanes);
        self.arranger.refresh(project);
        let mixer_height = self.settings.kept_heights().mixer_height();
        self.mixer = MixerState::new(self.mixer.size().width, mixer_height);
        self.transport_on = false;
        self.playing = false;
        self.playhead = None;
        self.drag = None;
        self.song_marker = dawsome_engine::time::Ticks(0);
        self.rack_cache.clear();
        if self.panels.piano_roll {
            self.show_roll();
        }
        self.select_channel_insert();
        self.show_project_folder();
        self.reset_media();
        self.read_automation();
        self.update_input();
    }
}

#[cfg(test)]
pub(super) mod testing {
    //! What the app's tests reach in the file state.
    use super::*;

    impl App {
        pub(crate) fn exiting(&self) -> bool {
            self.files.exiting
        }

        pub(crate) fn shows_unsaved_prompt(&self) -> bool {
            matches!(self.files.modal, Some(Modal::Unsaved))
        }

        pub(crate) fn shows_recovery(&self) -> bool {
            matches!(self.files.modal, Some(Modal::Recover { .. }))
        }

        pub(crate) fn take_parked_player(&mut self) -> Option<dawsome_engine::realtime::Player> {
            self.parked_player.take()
        }
    }
}

#[cfg(test)]
mod tests {
    //! Titles, the unsaved-changes question, save and autosave bytes, recovery, failed opens and
    //! saves, recent files, and export. Every file a test writes goes into a temporary directory;
    //! fixtures are only read.

    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use dawsome_engine::project::{ChannelId, ChannelKind, PatternId};
    use iced::keyboard::{Key, Modifiers};

    use super::*;
    use crate::app::{shortcut, MediaMessage, Message};
    use crate::audio;
    use crate::document::Host;
    use crate::headless;
    use crate::play_to;
    use crate::ui::layout;
    use crate::ui::panels::Panels;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    fn fixture(name: &str) -> PathBuf {
        Path::new(FIXTURES).join(name)
    }

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("dawsome-app-documents-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// Copies fixture files into `dir`; returns the first one's new path.
    fn copy_in(dir: &Path, names: &[&str]) -> PathBuf {
        for name in names {
            fs::copy(fixture(name), dir.join(name)).expect("copy fixture");
        }
        dir.join(names[0])
    }

    /// A host with no device and no background work, keeping recent files and autosaves in
    /// `dir`.
    fn host_in(dir: &Path) -> Host {
        Host {
            recent_file: Some(dir.join("config/dawsome/recent.json")),
            untitled_autosave: Some(dir.join("state/dawsome/autosave/untitled.autosave.json")),
            ..Host::offline()
        }
    }

    /// The app on a new document, as `dawsome-app` with no `--load` starts.
    fn untitled_app(host: Host) -> App {
        let empty = Opened {
            project: project_file::new_document(),
            samples_dir: PathBuf::new(),
        };
        boot(host, empty, Document::untitled())
    }

    fn boot(host: Host, opened: Opened, document: Document) -> App {
        let (session, _player) =
            Session::open(opened.project, opened.samples_dir, false).expect("compiles");
        let start = Start { document };
        let (app, _) = App::boot(
            session,
            Err(audio::StartError::NoDevice),
            Panels::default(),
            layout::DEFAULT_WINDOW_SIZE,
            false,
            headless::Script::default(),
            Arc::new(AtomicBool::new(false)),
            host,
            start,
        );
        app
    }

    fn send(app: &mut App, message: FileMessage) {
        let _ = app.update(Message::File(message));
    }

    fn open(app: &mut App, path: &Path) {
        send(app, FileMessage::OpenPicked(Some(path.to_path_buf())));
    }

    fn step(app: &mut App, step: u32) {
        let channel = app.selected_channel.expect("a channel");
        let _ = app.update(Message::StepClicked { channel, step });
    }

    fn json(app: &App) -> String {
        app.session.project().to_json().expect("serializes")
    }

    /// `--play-to <out>` of the first pattern, one bar, 32-bit float WAV.
    fn one_bar_to(out: PathBuf) -> play_to::PlayTo {
        play_to::PlayTo {
            out,
            bars: 1,
            song: false,
            bits: None,
            record: None,
            record_notes: false,
            quantize: true,
            count_in: true,
            save_to: None,
            stream_threshold: None,
            midi_events: None,
            cache_dir: None,
            dump_notes_at: None,
            midi_out_log: None,
        }
    }

    /// Both sides of every frame of the 32-bit float WAV at `path`.
    fn wav_frames(path: &Path) -> Vec<[f32; 2]> {
        let mut reader = hound::WavReader::open(path).expect("a WAV");
        assert_eq!(reader.spec().channels, 2);
        let samples: Vec<f32> = reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .expect("decodes");
        samples
            .chunks_exact(2)
            .map(|pair| [pair[0], pair[1]])
            .collect()
    }

    // A window opened with no file can make music at once: Pattern 1 of one bar, the default 8
    // tracks and 16 inserts, and one sampler channel with no sound yet (silent, and nothing to
    // warn about) to light steps on, each step one undo step. Played, one bar of it is 2 s of
    // silence: 4 beats at 120 BPM, 88,200 frames at 44.1 kHz.
    #[test]
    fn a_new_document_has_pattern_1_and_a_silent_sampler_to_light_steps_on() {
        let dir = temp_dir("new_document");
        let mut app = untitled_app(Host::offline());
        let project = app.session.project();
        assert_eq!(project.validate(), Ok(()));
        let patterns: Vec<(&str, u32)> = project
            .patterns
            .values()
            .map(|pattern| (pattern.name.as_str(), pattern.length_bars))
            .collect();
        assert_eq!(patterns, [("Pattern 1", 1)]);
        let channels: Vec<(&str, ChannelKind, &Path)> = project
            .channels
            .values()
            .map(|channel| {
                (
                    channel.name.as_str(),
                    channel.kind,
                    channel.sample.as_path(),
                )
            })
            .collect();
        assert_eq!(channels, [("Sampler", ChannelKind::Sampler, Path::new(""))]);
        assert_eq!(project.playlist.tracks.len(), 8);
        assert_eq!(project.mixer.ids().count(), 17, "the master and 16 inserts");
        assert_eq!(
            (app.pattern, app.selected_channel),
            (Some(PatternId(1)), Some(ChannelId(1)))
        );
        assert_eq!(app.notice, None);

        let before = json(&app);
        step(&mut app, 0);
        assert_eq!(app.title(), "Untitled* - Dawsome");
        let _ = app.update(Message::Undo);
        assert_eq!(json(&app), before, "one undo takes the step back");
        assert_eq!(app.title(), "Untitled - Dawsome");
        let _ = app.update(Message::Redo);

        let saved = dir.join("new.json");
        send(&mut app, FileMessage::SavePicked(Some(saved.clone())));
        let reopened =
            Project::load_json(&fs::read_to_string(&saved).expect("saved")).expect("loads");
        assert_eq!(
            reopened.channels[&ChannelId(1)].sample,
            PathBuf::new(),
            "still no sound, not the folder it was saved in"
        );
        play_to::run(&saved, &one_bar_to(dir.join("new.wav")), None).expect("renders");
        let frames = wav_frames(&dir.join("new.wav"));
        assert_eq!(frames.len(), 88_200);
        assert!(frames.iter().all(|&frame| frame == [0.0; 2]), "silent");
    }

    // The new document's "Sampler" plays the file it is given: impulse.wav picked for it, steps 1,
    // 5, 9 and 13 lit and the document saved, `--play-to` renders F1's kick onsets, frames 0,
    // 22050, 44100 and 66150 of 88200 (t2s in gen_p2_expected.py, 44.1 kHz, 120 BPM), each at a lit
    // step's velocity, STEP_VELOCITY = 100/128 = 0.78125 there, times the volume a channel is
    // added at, 9/16: 0.439453125, exact; and silence elsewhere.
    #[test]
    fn the_new_documents_sampler_plays_the_file_it_is_given() {
        let dir = temp_dir("new_sampler_file");
        let mut app = untitled_app(Host::offline());
        let _ = app.update(Message::Media(MediaMessage::SampleChosen {
            channel: ChannelId(1),
            file: Some(fixture("impulse.wav")),
        }));
        for onset in [0, 4, 8, 12] {
            step(&mut app, onset);
        }
        let saved = dir.join("new.json");
        send(&mut app, FileMessage::SavePicked(Some(saved.clone())));
        play_to::run(&saved, &one_bar_to(dir.join("new.wav")), None).expect("renders");
        let frames = wav_frames(&dir.join("new.wav"));
        assert_eq!(frames.len(), 88_200);
        let sounding: Vec<(usize, [f32; 2])> = frames
            .iter()
            .enumerate()
            .filter(|(_, &frame)| frame != [0.0; 2])
            .map(|(at, &frame)| (at, frame))
            .collect();
        let kick = [0.781_25 * 0.5625; 2];
        assert_eq!(
            sounding,
            [(0, kick), (22_050, kick), (44_100, kick), (66_150, kick)]
        );
    }

    fn example(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name)
    }

    /// A host keeping recent files and autosaves in `dir`, and the demo in `dir/demo`.
    fn demo_host(dir: &Path) -> Host {
        Host {
            demo_dir: Some(dir.join("demo")),
            ..host_in(dir)
        }
    }

    const DEMO_FILES: [&str; 5] = [
        "demo.json",
        "sounds/kick.wav",
        "sounds/snare.wav",
        "sounds/hat.wav",
        "sounds/click.wav",
    ];

    // File → Open demo puts the demo (its project and its own sounds) in the demo folder and
    // opens it from there with every sound found; saved, it keeps the bytes it was made with.
    #[test]
    fn the_demo_opens_from_the_file_menu_and_saves_back_to_its_own_bytes() {
        let dir = temp_dir("demo");
        let mut app = untitled_app(demo_host(&dir));
        send(&mut app, FileMessage::OpenDemo);
        let demo = dir.join("demo/demo.json");
        assert_eq!(app.document.path.as_deref(), Some(demo.as_path()));
        assert_eq!(app.title(), "demo - Dawsome");
        let project = app.session.project();
        let names: Vec<&str> = project
            .channels
            .values()
            .map(|channel| channel.name.as_str())
            .collect();
        assert_eq!(names, ["Kick", "Snare", "Hat", "Click"]);
        assert_eq!(project.patterns.len(), 2);
        assert_eq!(app.notice, None, "every sound is there");
        send(&mut app, FileMessage::Save);
        for name in DEMO_FILES {
            assert_eq!(
                fs::read(dir.join("demo").join(name)).expect("in the demo folder"),
                fs::read(example(name)).expect("in examples"),
                "{name}"
            );
        }
    }

    // A first launch (no recent files yet) lists the demo in the recent files, put in its
    // folder; a later launch lists nothing new, even when its list no longer holds the demo,
    // and a copy the user changed is never written over, not even by Open demo.
    #[test]
    fn a_first_launch_lists_the_demo_in_the_recent_files() {
        let dir = temp_dir("demo_first_launch");
        let app = untitled_app(demo_host(&dir));
        let demo = dir.join("demo/demo.json");
        assert_eq!(app.recent.entries(), std::slice::from_ref(&demo));
        assert!(dir.join("demo/sounds/kick.wav").is_file());
        fs::write(&demo, b"changed").expect("writes");
        let song = dir.join("song.json");
        let listed = serde_json::to_string(&[&song]).expect("a list of paths");
        fs::write(dir.join("config/dawsome/recent.json"), listed).expect("writes");
        let mut again = untitled_app(demo_host(&dir));
        assert_eq!(again.recent.entries(), std::slice::from_ref(&song));
        send(&mut again, FileMessage::OpenDemo);
        assert_eq!(fs::read(&demo).expect("there"), b"changed");
        assert!(again.error.is_some(), "the changed copy does not open");
        assert_eq!(again.title(), "Untitled - Dawsome");
        let offline = untitled_app(host_in(&temp_dir("demo_offline")));
        assert!(
            offline.recent.entries().is_empty(),
            "no demo folder, no demo"
        );
    }

    #[test]
    fn the_title_follows_the_document() {
        let dir = temp_dir("titles");
        let mut app = untitled_app(host_in(&dir));
        assert_eq!(app.title(), "Untitled - Dawsome");
        open(&mut app, &fixture("f1_four_on_floor.json"));
        assert_eq!(app.title(), "f1_four_on_floor - Dawsome");
        step(&mut app, 2);
        assert_eq!(app.title(), "f1_four_on_floor* - Dawsome");
        let _ = app.update(Message::Undo);
        assert_eq!(app.title(), "f1_four_on_floor - Dawsome");
        send(&mut app, FileMessage::New);
        assert_eq!(app.title(), "Untitled - Dawsome");
        assert_eq!(
            app.session.project(),
            &project_file::new_document(),
            "New starts the new document"
        );

        // Save on a new document asks for a name; the test gives it.
        send(&mut app, FileMessage::Save);
        send(&mut app, FileMessage::SavePicked(Some(dir.join("song"))));
        assert_eq!(app.title(), "song - Dawsome");
        assert!(
            dir.join("song.dawsome").is_file(),
            "the .dawsome extension is added"
        );
    }

    #[test]
    fn a_close_with_unsaved_changes_asks_and_dont_save_leaves_no_autosave() {
        let dir = temp_dir("close");
        let path = copy_in(&dir, &["f1_four_on_floor.json", "impulse.wav"]);
        let mut app = untitled_app(host_in(&dir));
        open(&mut app, &path);
        step(&mut app, 2);
        send(&mut app, FileMessage::AutosaveTick);
        let autosave = dir.join("f1_four_on_floor.autosave.json");
        assert!(autosave.is_file(), "the tick wrote the autosave");

        send(&mut app, FileMessage::CloseRequested);
        assert!(app.shows_unsaved_prompt());
        assert!(!app.exiting());
        send(&mut app, FileMessage::Unsaved(Choice::Cancel));
        assert!(!app.shows_unsaved_prompt());
        assert!(!app.exiting(), "Cancel keeps the window");

        send(&mut app, FileMessage::CloseRequested);
        send(&mut app, FileMessage::Unsaved(Choice::DontSave));
        assert!(app.exiting());
        assert!(
            !autosave.exists(),
            "Don't Save at close removes the autosave"
        );
    }

    #[test]
    fn the_file_menu_stays_open_through_timers_and_closes_on_an_action() {
        let dir = temp_dir("menu");
        let mut app = untitled_app(host_in(&dir));
        send(&mut app, FileMessage::ToggleMenu);
        send(&mut app, FileMessage::AutosaveTick);
        assert!(app.files.menu, "a timer does not close it");
        send(&mut app, FileMessage::New);
        assert!(!app.files.menu);
        send(&mut app, FileMessage::ToggleMenu);
        let _ = app.update(Message::ClearSelection);
        assert!(!app.files.menu, "Esc closes it");
    }

    #[test]
    fn a_clean_document_closes_without_asking() {
        let mut app = untitled_app(Host::offline());
        send(&mut app, FileMessage::CloseRequested);
        assert!(!app.shows_unsaved_prompt());
        assert!(app.exiting());
    }

    // While the question is up, editing does not reach the document.
    #[test]
    fn the_question_blocks_edits_until_answered() {
        let dir = temp_dir("blocks");
        let path = copy_in(&dir, &["f1_four_on_floor.json", "impulse.wav"]);
        let mut app = untitled_app(Host::offline());
        open(&mut app, &path);
        step(&mut app, 2);
        send(&mut app, FileMessage::New);
        assert!(app.shows_unsaved_prompt());
        let before = json(&app);
        step(&mut app, 3);
        let _ = app.update(Message::Undo);
        assert_eq!(json(&app), before);
        let _ = app.update(Message::ClearSelection);
        assert!(!app.shows_unsaved_prompt(), "Esc cancels");
        assert_eq!(app.title(), "f1_four_on_floor* - Dawsome");
    }

    // Save on an untitled document asks for a name first; canceling that cancels the New.
    #[test]
    fn canceling_the_name_cancels_the_whole_action() {
        let mut app = untitled_app(Host::offline());
        step(&mut app, 2);
        send(&mut app, FileMessage::New);
        send(&mut app, FileMessage::Unsaved(Choice::Save));
        send(&mut app, FileMessage::SavePicked(None));
        assert_eq!(app.title(), "Untitled* - Dawsome", "nothing replaced it");
        assert!(!app.shows_unsaved_prompt());
    }

    // (a) and (b): saved in place, every fixture keeps its bytes; Save As into another
    // directory stores the sample absolute, and the copy renders the same bytes.
    #[test]
    fn saving_keeps_the_bytes_and_save_as_keeps_the_sound() {
        let dir = temp_dir("round_trip");
        for name in [
            "p2_three_patterns.json",
            "p3_roll.json",
            "p4_loop.json",
            "p5_gain.json",
        ] {
            let path = copy_in(&dir, &[name, "impulse.wav"]);
            let mut app = untitled_app(Host::offline());
            open(&mut app, &path);
            step(&mut app, 3);
            let _ = app.update(Message::Undo);
            send(&mut app, FileMessage::Save);
            assert_eq!(
                fs::read(&path).expect("saved"),
                fs::read(fixture(name)).expect("fixture"),
                "{name}"
            );
        }
        let elsewhere = dir.join("elsewhere");
        fs::create_dir_all(&elsewhere).expect("dir");
        let mut app = untitled_app(Host::offline());
        open(&mut app, &dir.join("p4_loop.json"));
        send(
            &mut app,
            FileMessage::SavePicked(Some(elsewhere.join("p4_loop.json"))),
        );
        let moved =
            Project::load_json(&fs::read_to_string(elsewhere.join("p4_loop.json")).expect("saved"))
                .expect("loads");
        let sample = &moved.channels.values().next().expect("a channel").sample;
        assert_eq!(
            *sample,
            dunce::canonicalize(dir.join("impulse.wav")).expect("impulse")
        );
        let render = |path: &Path, out: &str| {
            let request = play_to::PlayTo {
                out: dir.join(out),
                bars: 3,
                song: true,
                bits: None,
                record: None,
                record_notes: false,
                quantize: true,
                count_in: true,
                save_to: None,
                stream_threshold: None,
                midi_events: None,
                cache_dir: None,
                dump_notes_at: None,
                midi_out_log: None,
            };
            play_to::run(path, &request, None).expect("renders");
            fs::read(dir.join(out)).expect("rendered")
        };
        assert!(
            render(&dir.join("p4_loop.json"), "before.wav")
                == render(&elsewhere.join("p4_loop.json"), "after.wav")
        );
    }

    // (c) The autosave holds the document's bytes; Save writes the same bytes and takes the
    // autosave away; a tick with nothing new writes nothing.
    #[test]
    fn autosave_holds_the_document_until_it_is_saved() {
        let dir = temp_dir("autosave");
        let path = copy_in(&dir, &["p4_loop.json", "impulse.wav"]);
        let mut app = untitled_app(host_in(&dir));
        open(&mut app, &path);
        send(&mut app, FileMessage::AutosaveTick);
        let autosave = dir.join("p4_loop.autosave.json");
        assert!(!autosave.exists(), "a clean document is not autosaved");
        step(&mut app, 1);
        send(&mut app, FileMessage::AutosaveTick);
        // A saved file is the canonical JSON and a final newline, as every fixture is.
        let autosaved = fs::read_to_string(&autosave).expect("autosaved");
        assert_eq!(autosaved, json(&app) + "\n");
        fs::remove_file(&autosave).expect("removed");
        send(&mut app, FileMessage::AutosaveTick);
        assert!(
            !autosave.exists(),
            "nothing changed since the last autosave"
        );
        send(&mut app, FileMessage::Save);
        assert_eq!(fs::read_to_string(&path).expect("saved"), autosaved);
        assert!(!autosave.exists());
        assert_eq!(app.title(), "p4_loop - Dawsome");
    }

    // A background autosave still writing when Save takes the autosave away must not land
    // after it: the stale file would be offered as a recovery of changes that are saved.
    #[test]
    fn a_late_background_autosave_leaves_nothing_after_save() {
        let dir = temp_dir("late_autosave");
        let path = copy_in(&dir, &["f1_four_on_floor.json", "impulse.wav"]);
        let host = Host {
            background: true,
            ..host_in(&dir)
        };
        let mut app = untitled_app(host.clone());
        open(&mut app, &path);
        step(&mut app, 2);
        // The write a tick hands to background work, held back until the Save is done.
        let (_, write) = app.autosave_job().expect("changes to autosave");
        send(&mut app, FileMessage::Save);
        write().expect("the late write");
        let mut reopened = untitled_app(host);
        open(&mut reopened, &path);
        assert!(!reopened.shows_recovery(), "a recovery of saved changes");
        assert!(!dir.join("f1_four_on_floor.autosave.json").exists());
    }

    // A background autosave of the last document that lands after the next one opened names a
    // state of the last document's history, not of the new one's: the new document's first
    // change is still autosaved at the next tick.
    #[test]
    fn a_late_autosave_of_the_last_document_does_not_skip_the_next_ones() {
        let dir = temp_dir("late_autosave_other_document");
        let first = copy_in(&dir, &["f1_four_on_floor.json", "impulse.wav"]);
        let second = copy_in(&dir, &["p4_loop.json"]);
        let mut app = untitled_app(host_in(&dir));
        open(&mut app, &first);
        step(&mut app, 2);
        let (state, write) = app.autosave_job().expect("changes to autosave");
        open(&mut app, &second);
        send(&mut app, FileMessage::Unsaved(Choice::DontSave));
        assert_eq!(app.document.path.as_deref(), Some(second.as_path()));
        step(&mut app, 2);
        let result = write();
        send(&mut app, FileMessage::Autosaved { state, result });
        send(&mut app, FileMessage::AutosaveTick);
        let autosave = dir.join("p4_loop.autosave.json");
        let autosaved = fs::read_to_string(&autosave).expect("the new document is autosaved");
        assert_eq!(autosaved, json(&app) + "\n");
    }

    #[test]
    fn a_newer_autosave_is_offered_when_its_file_opens() {
        let dir = temp_dir("recover");
        let path = copy_in(&dir, &["f1_four_on_floor.json", "impulse.wav"]);
        let mut app = untitled_app(host_in(&dir));
        open(&mut app, &path);
        step(&mut app, 2);
        send(&mut app, FileMessage::AutosaveTick);
        let changed = json(&app);
        // Open the file again elsewhere: the autosave is newer than it.
        let mut second = untitled_app(host_in(&dir));
        open(&mut second, &path);
        assert!(second.shows_recovery());
        send(&mut second, FileMessage::Recover(true));
        assert_eq!(json(&second), changed);
        assert_eq!(second.title(), "f1_four_on_floor* - Dawsome");
        assert_eq!(second.document.path.as_deref(), Some(path.as_path()));

        let mut third = untitled_app(host_in(&dir));
        open(&mut third, &path);
        send(&mut third, FileMessage::Recover(false));
        assert_eq!(third.title(), "f1_four_on_floor - Dawsome");
        assert_ne!(json(&third), changed);
    }

    #[test]
    fn a_leftover_untitled_autosave_is_offered_at_start() {
        let dir = temp_dir("leftover");
        let host = host_in(&dir);
        let leftover = host.untitled_autosave.clone().expect("autosave on");
        fs::create_dir_all(leftover.parent().expect("dir")).expect("dir");
        fs::copy(fixture("f1_four_on_floor.json"), &leftover).expect("copy");
        let mut app = untitled_app(host.clone());
        assert!(app.shows_recovery());
        send(&mut app, FileMessage::Recover(false));
        assert!(!leftover.exists(), "Discard removes it");
        assert!(!untitled_app(host).shows_recovery());
    }

    // A failed open names the file and why; the document stays.
    #[test]
    fn a_failed_open_keeps_the_document() {
        let dir = temp_dir("failed_open");
        let mut app = untitled_app(Host::offline());
        open(&mut app, &fixture("f1_four_on_floor.json"));
        let before = json(&app);
        open(&mut app, &dir.join("gone.json"));
        let error = app.error.clone().expect("an error");
        assert!(error.contains("gone.json"), "{error}");
        fs::write(dir.join("cut.json"), b"{").expect("write");
        open(&mut app, &dir.join("cut.json"));
        let error = app.error.clone().expect("an error");
        assert!(error.contains("cut.json"), "{error}");
        assert_eq!(json(&app), before);
        assert_eq!(app.title(), "f1_four_on_floor - Dawsome");
    }

    // A save into a directory that takes no files shows why and leaves the old file. Linux
    // only: a Windows folder has no read-only mode bits to refuse new files with.
    #[cfg(unix)]
    #[test]
    fn a_failed_save_keeps_the_old_file_and_stays_dirty() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("failed_save");
        let path = copy_in(&dir, &["f1_four_on_floor.json", "impulse.wav"]);
        let before = fs::read(&path).expect("read");
        let mut app = untitled_app(Host::offline());
        open(&mut app, &path);
        step(&mut app, 2);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).expect("chmod");
        send(&mut app, FileMessage::Save);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("chmod back");
        assert!(app
            .error
            .as_deref()
            .is_some_and(|e| e.contains("cannot save")));
        assert_eq!(fs::read(&path).expect("read"), before);
        assert_eq!(app.title(), "f1_four_on_floor* - Dawsome");
    }

    // A project whose sample is gone opens silent with a notice.
    #[test]
    fn a_missing_sample_opens_with_a_notice() {
        let dir = temp_dir("missing_sample");
        let path = copy_in(&dir, &["f1_four_on_floor.json"]);
        let mut app = untitled_app(Host::offline());
        open(&mut app, &path);
        let notice = app.notice.clone().expect("a notice");
        assert!(
            notice.contains("impulse.wav") && notice.contains("plays silence"),
            "{notice}"
        );
        assert_eq!(app.title(), "f1_four_on_floor - Dawsome");
    }

    // A link no edit could put back (here on an audio channel, which plays no device's notes)
    // is left out when the file opens, and the toolbar says so, naming the channel and the port,
    // after the notice of the missing sound (kick's impulse.wav is not copied).
    #[test]
    fn a_link_left_out_on_opening_is_named_in_the_toolbar() {
        let dir = temp_dir("link_left_out");
        let text = fs::read_to_string(fixture("p10_link.json")).expect("reads");
        let audio = text.replace(
            r#""kind":"sampler","sample":"ramp4.wav""#,
            r#""kind":"audio","sample":"ramp4.wav""#,
        );
        assert_ne!(audio, text);
        let path = dir.join("linked.json");
        fs::write(&path, audio).expect("writes");
        let mut app = untitled_app(Host::offline());
        open(&mut app, &path);
        assert_eq!(app.title(), "linked - Dawsome");
        let notice = app.notice.clone().expect("a notice");
        assert!(
            notice.contains("impulse.wav")
                && notice.contains("'keys'")
                && notice.contains("'USB Keyboard MIDI 1'")
                && notice.contains("left out"),
            "{notice}"
        );
        assert!(
            notice.find("impulse.wav") < notice.find("'keys'"),
            "the sound first: {notice}"
        );
        assert_eq!(
            app.session
                .project()
                .channel_for_port("USB Keyboard MIDI 1"),
            None
        );
        assert!(!json(&app).contains("midi_input"), "left out");
    }

    #[test]
    fn opened_and_saved_documents_go_on_the_recent_list() {
        let dir = temp_dir("recent");
        let path = copy_in(&dir, &["f1_four_on_floor.json", "impulse.wav"]);
        let host = host_in(&dir);
        let mut app = untitled_app(host.clone());
        open(&mut app, &path);
        open(&mut app, &fixture("p4_loop.json"));
        let reloaded = crate::recent::Recent::load(host.recent_file.clone());
        assert_eq!(
            reloaded.entries(),
            [fixture("p4_loop.json"), path.clone()].map(|p| std::path::absolute(p).expect("abs"))
        );
        // A gone entry comes off the list when clicked.
        let gone = dir.join("gone.json");
        app.recent.add(&gone).expect("adds");
        send(&mut app, FileMessage::OpenRecent(gone.clone()));
        assert!(!app.recent.entries().contains(&gone));
        assert!(app
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("gone.json")));
    }

    // A save used to give "Untitled.json": a `.json` project given a bare name by Save As
    // is written as `<name>.dawsome`, byte for byte what its `.json` save writes (the format
    // does not change), titled without the extension, and both files are on the recent list,
    // newest first. The `.dawsome` file opens again from that list, Save writes back to it, and
    // its autosave is `song.autosave.dawsome`, apart from the `.json` file's.
    #[test]
    fn save_as_writes_a_dawsome_file_and_the_recent_list_holds_both() {
        let dir = temp_dir("dawsome_extension");
        let path = copy_in(&dir, &["p4_loop.json", "impulse.wav"]);
        let host = host_in(&dir);
        let mut app = untitled_app(host.clone());
        open(&mut app, &path);
        send(&mut app, FileMessage::SavePicked(Some(dir.join("song"))));
        let saved = dir.join("song.dawsome");
        assert_eq!(
            fs::read(&saved).expect("saved"),
            fs::read(fixture("p4_loop.json")).expect("fixture")
        );
        assert_eq!(app.title(), "song - Dawsome");
        let absolute = |path: &Path| std::path::absolute(path).expect("abs");
        let reloaded = crate::recent::Recent::load(host.recent_file.clone());
        assert_eq!(reloaded.entries(), [absolute(&saved), absolute(&path)]);

        open(&mut app, &path);
        assert_eq!(app.title(), "p4_loop - Dawsome");
        send(&mut app, FileMessage::OpenRecent(absolute(&saved)));
        assert_eq!(app.title(), "song - Dawsome");
        assert_eq!(
            app.document.path.as_deref(),
            Some(absolute(&saved).as_path())
        );
        step(&mut app, 3);
        send(&mut app, FileMessage::AutosaveTick);
        assert!(dir.join("song.autosave.dawsome").exists());
        assert!(!dir.join("song.autosave.json").exists());
        send(&mut app, FileMessage::Save);
        assert_eq!(app.title(), "song - Dawsome");
        assert!(
            !dir.join("song.autosave.dawsome").exists(),
            "saved: autosave gone"
        );
        let written = Project::load_json(&fs::read_to_string(&saved).expect("saved"));
        assert_eq!(written.expect("loads"), *app.session.project());
    }

    // The Open dialog shows the Dawsome projects under either extension; Save As offers
    // `.dawsome` alone.
    #[test]
    fn the_open_dialog_offers_dawsome_projects_under_either_extension() {
        let filters: Vec<(&str, Vec<&str>)> = OPEN_FILTERS
            .iter()
            .map(|(name, extensions)| (*name, extensions.to_vec()))
            .collect();
        assert_eq!(
            filters,
            [(
                "Dawsome projects (*.dawsome, *.json)",
                vec!["dawsome", "json"]
            )]
        );
        assert_eq!(SAVE_FILTER, ("Dawsome project (*.dawsome)", "dawsome"));
    }

    // (g) through the dialog: F1, 1 bar, 16-bit with Dither set Off: 32767 at 0, 22050, 44100 and
    // 66150 (from the Python rule round(x·2^15) clamped), 88200 frames. The dialog opens with
    // Dither on (TPDF).
    #[test]
    fn the_export_dialog_writes_the_chosen_depth() {
        let dir = temp_dir("export");
        let path = copy_in(&dir, &["f1_four_on_floor.json", "impulse.wav"]);
        let mut app = untitled_app(Host::offline());
        open(&mut app, &path);
        send(&mut app, FileMessage::Export(ExportKind::Wav));
        let form = app.export_form().expect("the dialog is up");
        assert_eq!(
            (form.bars, form.depth, form.dither, form.rate),
            (
                "1",
                dawsome_media::BitDepth::Int24,
                dawsome_media::Dither::Tpdf,
                44_100
            )
        );
        let export = |message| FileMessage::ExportForm(message);
        send(
            &mut app,
            export(ExportMessage::Depth(dawsome_media::BitDepth::Int16)),
        );
        send(
            &mut app,
            export(ExportMessage::Dither(dawsome_media::Dither::Off)),
        );
        send(&mut app, export(ExportMessage::Bars("zero".into())));
        send(&mut app, export(ExportMessage::Start));
        assert!(app.export_form().and_then(|form| form.error).is_some());
        send(&mut app, export(ExportMessage::Bars("1".into())));
        send(&mut app, export(ExportMessage::Start));
        send(
            &mut app,
            export(ExportMessage::Picked(Some(dir.join("f1")))),
        );
        assert!(app.export_form().is_none(), "done closes the dialog");
        let out = dir.join("f1.wav");
        let mut reader = hound::WavReader::open(&out).expect("exported");
        assert_eq!(reader.spec().bits_per_sample, 16);
        let left: Vec<i32> = reader
            .samples::<i32>()
            .step_by(2)
            .collect::<Result<_, _>>()
            .expect("samples");
        assert_eq!(left.len(), 88_200);
        let lit: Vec<(usize, i32)> = left
            .iter()
            .enumerate()
            .filter(|(_, &v)| v != 0)
            .map(|(at, &v)| (at, v))
            .collect();
        assert_eq!(
            lit,
            [
                (0, 32_767),
                (22_050, 32_767),
                (44_100, 32_767),
                (66_150, 32_767)
            ]
        );
        assert!(app.notice.as_deref().is_some_and(|n| n.contains("f1.wav")));
    }

    // Known answer (gen_p25_expected.py's f1_16) through the dialog: F1 at 16 bits with Dither as
    // it opens (TPDF) is the beats at 32767 and the dither's noise everywhere else, value for
    // value the command line's `render --bits 16` (one LCG run from the same seed per file).
    #[test]
    fn the_export_dialog_dithers_sixteen_bits_as_it_opens() {
        use sha2::Digest;
        let dir = temp_dir("export_dither");
        let path = copy_in(&dir, &["f1_four_on_floor.json", "impulse.wav"]);
        let mut app = untitled_app(Host::offline());
        open(&mut app, &path);
        send(&mut app, FileMessage::Export(ExportKind::Wav));
        let export = |message| FileMessage::ExportForm(message);
        send(
            &mut app,
            export(ExportMessage::Depth(dawsome_media::BitDepth::Int16)),
        );
        send(&mut app, export(ExportMessage::Start));
        send(
            &mut app,
            export(ExportMessage::Picked(Some(dir.join("f1")))),
        );
        assert!(app.export_form().is_none(), "done closes the dialog");
        let text = fs::read_to_string(fixture("p25_expected.json")).expect("p25_expected.json");
        let expected: serde_json::Value = serde_json::from_str(&text).expect("parses");
        let want = &expected["dither"]["f1_16"];
        let mut reader = hound::WavReader::open(dir.join("f1.wav")).expect("exported");
        assert_eq!(reader.spec().bits_per_sample, 16);
        let values: Vec<i32> = reader
            .samples::<i32>()
            .collect::<Result<_, _>>()
            .expect("samples");
        assert_eq!(values.len(), 2 * 88_200);
        for beat in want["beats"].as_array().expect("beats") {
            let at = 2 * beat.as_u64().expect("a frame") as usize;
            assert_eq!(values[at..at + 2], [32_767, 32_767], "the beat at {beat}");
        }
        let nonzero = values.iter().filter(|&&v| v != 0).count() as u64;
        assert_eq!(nonzero, want["nonzero"].as_u64().expect("nonzero"));
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let hash: String = sha2::Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(hash, want["sha256"].as_str().expect("sha256"));
    }

    // The song's bars by default in Song mode: p4_loop's clips end at tick 1152, 3 bars. It
    // opens in song mode, on the music (app/landing.rs); pattern mode is chosen first.
    #[test]
    fn the_export_bars_default_to_the_song_or_the_pattern() {
        let mut app = untitled_app(Host::offline());
        open(&mut app, &fixture("p4_loop.json"));
        assert_eq!(app.mode, crate::app::PlayMode::Song, "opened on the song");
        let _ = app.update(Message::ModeSelected(crate::app::PlayMode::Pattern));
        send(&mut app, FileMessage::Export(ExportKind::Wav));
        assert_eq!(
            app.export_form().map(|form| form.bars.to_owned()),
            Some("1".into())
        );
        send(&mut app, FileMessage::ExportForm(ExportMessage::Cancel));
        let _ = app.update(Message::ModeSelected(crate::app::PlayMode::Song));
        send(&mut app, FileMessage::Export(ExportKind::Wav));
        assert_eq!(
            app.export_form().map(|form| form.bars.to_owned()),
            Some("3".into())
        );
        assert_eq!(app.pattern, Some(PatternId(1)));
    }

    /// The left channel's frame count and its frames that are not silent, of a 32-bit float
    /// WAV file.
    fn float_onsets(path: &Path) -> (usize, Vec<usize>) {
        let mut reader = hound::WavReader::open(path).expect("exported");
        let left: Vec<f32> = reader
            .samples::<f32>()
            .step_by(2)
            .collect::<Result<_, _>>()
            .expect("samples");
        let onsets = (0..left.len()).filter(|&at| left[at] != 0.0).collect();
        (left.len(), onsets)
    }

    // B3: the dialog chooses what plays, the toolbar's mode by default. p4_two_clips in pattern
    // mode (chosen after it opens): Pattern 1, 1 bar; Song chosen, the bars untouched follow it to the song's 3 bars,
    // and the file is the song's, as tests/fixtures/gen_p4_expected.py prints it for --song
    // --bars 3: 264600 frames, onsets 0, 22050, 44100, 66150, 176400, 198450, 220500, 242550;
    // and the same bytes `dawsome render --song --bars 3` writes. Bars typed stay when the
    // choice changes; Pattern chosen again plays the pattern, looping: 2 bars of the 1-bar
    // pattern (88200 frames, onsets 0, 22050, 44100, 66150) are 176400 frames, the onsets again
    // 88200 later.
    #[test]
    fn the_export_dialog_plays_the_pattern_or_the_song_as_chosen() {
        use crate::app::PlayMode;
        let dir = temp_dir("export_plays");
        let path = copy_in(&dir, &["p4_two_clips.json", "impulse.wav"]);
        let mut app = untitled_app(Host::offline());
        open(&mut app, &path);
        // It opens on its song (app/landing.rs); the toolbar is put in pattern mode.
        assert_eq!(app.mode, PlayMode::Song, "opened on the song");
        let _ = app.update(Message::ModeSelected(PlayMode::Pattern));
        let export = |message| FileMessage::ExportForm(message);
        send(&mut app, FileMessage::Export(ExportKind::Wav));
        let form = app.export_form().expect("the dialog is up");
        assert_eq!(
            (form.plays, form.pattern, form.bars),
            (PlayMode::Pattern, Some("Pattern 1"), "1")
        );
        send(&mut app, export(ExportMessage::Plays(PlayMode::Song)));
        let form = app.export_form().expect("the dialog is up");
        assert_eq!((form.plays, form.bars), (PlayMode::Song, "3"));
        send(
            &mut app,
            export(ExportMessage::Depth(dawsome_media::BitDepth::Float32)),
        );
        send(&mut app, export(ExportMessage::Start));
        send(
            &mut app,
            export(ExportMessage::Picked(Some(dir.join("song")))),
        );
        assert!(app.export_form().is_none(), "done closes the dialog");
        assert_eq!(app.mode, PlayMode::Pattern, "the toolbar's mode stays");
        let song = dir.join("song.wav");
        assert_eq!(
            float_onsets(&song),
            (
                264_600,
                vec![0, 22_050, 44_100, 66_150, 176_400, 198_450, 220_500, 242_550]
            )
        );
        let cli = std::env::current_exe()
            .ok()
            .and_then(|test| Some(test.parent()?.parent()?.join("dawsome")))
            .filter(|cli| cli.is_file())
            .expect("the command line is built beside the tests: run scripts/check.sh");
        let rendered = dir.join("cli.wav");
        let status = std::process::Command::new(cli)
            .arg("render")
            .arg(&path)
            .arg(&rendered)
            .args(["--song", "--bars", "3"])
            .status()
            .expect("the command line starts");
        assert!(status.success());
        assert!(
            fs::read(&song).expect("read") == fs::read(&rendered).expect("read"),
            "the export is the command line's song render"
        );
        // Typed bars stay; Pattern chosen again plays the pattern.
        send(&mut app, FileMessage::Export(ExportKind::Wav));
        assert_eq!(
            app.export_form().map(|form| form.plays),
            Some(PlayMode::Pattern)
        );
        send(&mut app, export(ExportMessage::Plays(PlayMode::Song)));
        send(&mut app, export(ExportMessage::Bars("2".into())));
        send(&mut app, export(ExportMessage::Plays(PlayMode::Pattern)));
        assert_eq!(app.export_form().map(|form| form.bars), Some("2"));
        send(
            &mut app,
            export(ExportMessage::Depth(dawsome_media::BitDepth::Float32)),
        );
        send(&mut app, export(ExportMessage::Start));
        send(
            &mut app,
            export(ExportMessage::Picked(Some(dir.join("pattern")))),
        );
        assert_eq!(
            float_onsets(&dir.join("pattern.wav")),
            (
                176_400,
                vec![0, 22_050, 44_100, 66_150, 88_200, 110_250, 132_300, 154_350]
            )
        );
        // In song mode the dialog chooses the song first.
        let _ = app.update(Message::ModeSelected(PlayMode::Song));
        send(&mut app, FileMessage::Export(ExportKind::Wav));
        assert_eq!(
            app.export_form().map(|form| form.plays),
            Some(PlayMode::Song)
        );
    }

    // The opened document plays through its own new player: F1, one bar of four kicks at
    // 120 BPM and 44.1 kHz, a beat every 22050 frames (0.5 s), as the command line renders it.
    #[test]
    fn an_opened_document_plays_through_its_new_player() {
        let mut app = untitled_app(Host::offline());
        open(&mut app, &fixture("f1_four_on_floor.json"));
        let mut player = app.take_parked_player().expect("the new document's player");
        let _ = app.update(Message::Play);
        let mut out = vec![[0.0f32; 2]; 88_200];
        player.process(&mut out);
        let sounding: Vec<usize> = (0..out.len()).filter(|&i| out[i][0] != 0.0).collect();
        assert_eq!(sounding, [0, 22_050, 44_100, 66_150]);
    }

    #[test]
    fn mp3_is_offered_at_the_rates_it_writes() {
        assert_eq!(super::super::export::mp3_unavailable(44_100), None);
        assert!(super::super::export::mp3_unavailable(22_050).is_some());
    }

    #[test]
    fn control_keys_are_new_open_save_and_save_as() {
        let ctrl = Modifiers::CTRL;
        let file = |key: &str, modifiers| match shortcut(Key::Character(key), modifiers, false) {
            Some(Message::File(message)) => Some(format!("{message:?}")),
            _ => None,
        };
        assert_eq!(file("s", ctrl).as_deref(), Some("Save"));
        assert_eq!(
            file("S", ctrl | Modifiers::SHIFT).as_deref(),
            Some("SaveAs")
        );
        assert_eq!(file("o", ctrl).as_deref(), Some("Open"));
        assert_eq!(file("n", ctrl).as_deref(), Some("New"));
        assert_eq!(file("s", Modifiers::empty()), None);
    }

    #[test]
    fn ages_read_in_whole_units() {
        let ago = |seconds| age(SystemTime::now() - Duration::from_secs(seconds));
        assert_eq!(ago(5), "less than a minute ago");
        assert_eq!(ago(61), "1 minute ago");
        assert_eq!(ago(7300), "2 hours ago");
    }

    /// An output standing in for a 48 kHz-only one: its device plays 48000 Hz on its own, and the
    /// stream resamples the 44.1 kHz document to it.
    fn owners_output(
        player: dawsome_engine::realtime::Player,
    ) -> Result<Output, crate::audio::StartFailure> {
        Ok(Output::holding_at(player, 48_000, Some((44_100, 48_000))))
    }

    // While an output is open, File → New and New empty project take
    // the rate its device plays at on its own (48000 here, where the stream resampled the
    // 44.1 kHz document), so nothing is resampled; the rest is the document they made before.
    // A file opened keeps its own rate; with no output open, a new document is 44.1 kHz.
    #[test]
    fn a_new_document_takes_the_output_devices_rate() {
        let dir = temp_dir("new_rate");
        let host = Host {
            audio: AudioChoice::Fake(owners_output),
            starter_kit: Some(dir.join("starter-kit")),
            ..host_in(&dir)
        };
        let mut app = untitled_app(host);
        assert!(app.audio.is_none(), "booted without an output");
        send(&mut app, FileMessage::NewEmpty);
        assert_eq!(
            app.session.project(),
            &project_file::new_document(),
            "no output open yet: 44.1 kHz"
        );
        assert!(app.audio.is_some(), "the stand-in opened");
        let at_48k = |mut project: Project| {
            project.sample_rate = 48_000;
            project
        };
        send(&mut app, FileMessage::New);
        assert_eq!(
            app.session.project(),
            &at_48k(project_file::starter_document())
        );
        send(&mut app, FileMessage::NewEmpty);
        assert_eq!(app.session.project(), &at_48k(project_file::new_document()));
        open(&mut app, &fixture("f1_four_on_floor.json"));
        assert_eq!(
            app.session.project().sample_rate,
            44_100,
            "F1 keeps its rate"
        );
        assert_eq!(project_file::new_document_rate(None), 44_100);
        assert_eq!(project_file::new_document_rate(Some(96_000)), 96_000);
    }

    /// The rate the stand-in devices play at on their own: "B 44k" 44100 Hz, the system's
    /// default (and any other) 48000.
    fn rate_by_name(device: Option<&str>) -> Option<u32> {
        Some(if device == Some("B 44k") {
            44_100
        } else {
            48_000
        })
    }

    /// An output on the stand-in device named `device`, at its own rate.
    fn open_by_name(
        player: dawsome_engine::realtime::Player,
        device: Option<&str>,
    ) -> Result<Output, crate::audio::StartFailure> {
        let rate = rate_by_name(device).expect("a rate");
        Ok(Output::holding_named_at(player, device, rate))
    }

    // File → New takes the rate of the output it will play on: with A
    // (48 kHz) open and B (44.1 kHz) chosen in Settings since, the new document is 44.1 kHz,
    // as B is, not A's 48 kHz played resampled on B; the next File → New, on B, stays so. The
    // sequence before the fix gave 48000 Hz on B, then 44100.
    #[test]
    fn a_new_document_takes_the_rate_of_the_output_chosen_in_settings() {
        let dir = temp_dir("new_rate_chosen");
        let host = Host {
            audio: AudioChoice::FakeRated(open_by_name, rate_by_name),
            ..host_in(&dir)
        };
        let mut app = untitled_app(host);
        let on = |app: &App| {
            let output = app.audio.as_ref().expect("an output");
            (
                app.session.project().sample_rate,
                output.asked().map(str::to_owned),
                output.own_rate(),
            )
        };
        send(&mut app, FileMessage::NewEmpty);
        assert_eq!(
            on(&app),
            (48_000, None, Some(48_000)),
            "on the default device"
        );
        app.settings.set_audio_output(Some("B 44k".to_owned()));
        send(&mut app, FileMessage::NewEmpty);
        assert_eq!(on(&app), (44_100, Some("B 44k".to_owned()), Some(44_100)));
        send(&mut app, FileMessage::New);
        assert_eq!(on(&app), (44_100, Some("B 44k".to_owned()), Some(44_100)));
    }

    // "Resampling 44100 → 48000 Hz" left the toolbar: it is a line of
    // Settings → Audio (and the hint over the position); the toolbar's notices are the
    // warnings alone, one line a dot apart, and a press on them opens Settings on its General
    // page, where the audio output and input are.
    #[test]
    fn the_resampling_line_is_in_settings_and_a_notice_press_opens_them() {
        use crate::ui::panels::Panel;
        use crate::ui::settings_layout::SettingsPage;
        let dir = temp_dir("resampling_line");
        let host = Host {
            audio: AudioChoice::Fake(owners_output),
            ..host_in(&dir)
        };
        let mut app = untitled_app(host);
        send(&mut app, FileMessage::NewEmpty);
        assert_eq!(
            app.toolbar().notice_text(),
            None,
            "the resampling is no notice"
        );
        assert_eq!(
            app.settings_panel().resampling.as_deref(),
            Some("Resampling 44100 \u{2192} 48000 Hz")
        );
        assert_eq!(
            app.hint_context().resampling,
            Some("Resampling 44100 \u{2192} 48000 Hz")
        );
        app.notice = Some("Kick: sample missing".to_owned());
        app.error = Some("Nothing to undo".to_owned());
        assert_eq!(
            app.toolbar().notice_text().as_deref(),
            Some("Kick: sample missing \u{b7} Nothing to undo")
        );
        app.settings_page = SettingsPage::Keys;
        let _ = app.update(Message::NoticePressed);
        assert!(app.panels.is_open(Panel::Settings));
        assert_eq!(app.settings_page, SettingsPage::General);
        let _ = app.update(Message::NoticePressed);
        assert!(
            app.panels.is_open(Panel::Settings),
            "a second press keeps them open"
        );
    }

    // Where the starter kit can be written, New starts the starter document and writes the
    // kit its four rows play; New empty project starts the one-sampler document New made
    // before. Without a folder for the kit (a scripted run) New is the empty one.
    #[test]
    fn new_starts_the_starter_document_where_the_kit_can_be_written() {
        let dir = temp_dir("starter_new");
        let host = Host {
            starter_kit: Some(dir.join("starter-kit")),
            ..host_in(&dir)
        };
        let mut app = untitled_app(host);
        send(&mut app, FileMessage::New);
        assert_eq!(app.session.project(), &project_file::starter_document());
        assert_eq!(app.title(), "Untitled - Dawsome");
        for name in project_file::STARTER_ROWS {
            assert!(
                dir.join(format!("starter-kit/{name}.wav")).is_file(),
                "{name} is written"
            );
        }
        send(&mut app, FileMessage::NewEmpty);
        assert_eq!(app.session.project(), &project_file::new_document());
        let mut offline = untitled_app(Host::offline());
        send(&mut offline, FileMessage::New);
        assert_eq!(offline.session.project(), &project_file::new_document());
    }
}
