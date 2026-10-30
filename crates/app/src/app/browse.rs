//! The Library panel's Samples tree and the sounds the playlist draws, in the window: folders
//! listed and the search index built in the background, previews, files dragged onto the rack
//! or a playlist lane, imported or dropped on the window, sound files given to sampler channels,
//! and what the playlist shows of audio and automation clips besides the document (each file's
//! waveform, each automation's name). The press held on any Library row, and the scroll of the
//! Library's list, live here too; the other sections' own gestures are in [`super::library`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use dawsome_engine::edit::{self, Edit};
use dawsome_engine::project::{
    AutomationId, ChannelId, ChannelKind, ClipId, ClipItem, Project, SoundEdits, SoundSource,
    StretchMode, TrackId,
};
use dawsome_engine::realtime::EngineEvent;
use dawsome_engine::source::ClipSource;
use dawsome_engine::time::Ticks;
use dawsome_host_clap::catalogue::Offered;
use dawsome_media::decode::probe;
use dawsome_media::{load_channel_sample, Peaks, SampleNotice};
use iced::widget::{canvas, column, pin};
use iced::{mouse, Element, Point, Task};

use super::files::{blocking, summarize_notices};
use super::{App, ChannelCommand, Message};
use crate::document::Host;
use crate::record;
use crate::sets::Found;
use crate::ui::browser::{self, BrowserMessage, Nav, Pick};
use crate::ui::browser_model::{self, Browser, Entry, Folders, RowKind};
use crate::ui::clip_gesture::{self, StretchRefused};
use crate::ui::clips::{self, FileLook, FileLooks};
use crate::ui::library::{Item, Library};
use crate::ui::panels::Panel;
use crate::ui::playlist::ClipProperties;

/// How far a press on a file moves before it drags the file.
const DRAG_DISTANCE: f32 = 4.0;

/// How often a preview's end and the files still loading are looked at.
pub(super) const MEDIA_PERIOD: Duration = Duration::from_millis(100);

/// The sampler menu's width: its row, "Change sample…", and a margin.
const SAMPLER_MENU_WIDTH: f32 = 160.0;

/// What background work, the desktop and the panels around the browser report.
#[derive(Debug, Clone)]
pub enum MediaMessage {
    /// The cursor came onto the rack, or left it.
    RackHover(bool),
    /// The rack header's "+ Sampler…": pick a sound file for a new sampler channel.
    AddSampler,
    /// The file "+ Sampler…" picked (none when the dialog was canceled).
    SamplerPicked(Option<PathBuf>),
    /// File → Import audio…
    ImportAudio,
    /// The files the import dialog picked (none when it was canceled).
    ImportPicked(Vec<PathBuf>),
    /// The folder "+ Add folder…" picked.
    FolderPicked(Option<PathBuf>),
    /// A file dropped on the window from the desktop that no part of it took
    /// ([`App::desktop_drop`]).
    FileDropped(PathBuf),
    /// A right press on a channel's name, at this point of the rack's canvas: its menu (first a
    /// sampler's, now every kind's).
    SamplerMenu { channel: ChannelId, at: Point },
    /// The sampler menu's "Change sample…", or a double press on the name of a sampler with no
    /// sound: pick a sound file for that channel.
    ChangeSample(ChannelId),
    /// The file picked for a sampler channel (none when the dialog was canceled).
    SampleChosen {
        channel: ChannelId,
        file: Option<PathBuf>,
    },
    /// The sampler row a file held from the browser is over, or none.
    OverSampler(Option<ChannelId>),
    /// The rack row a MIDI device held from the Library is over, or none.
    OverChannel(Option<ChannelId>),
    /// The channel whose header a sound, a set or a plugin held from the Library is over, or
    /// none.
    OverHeader(Option<ChannelId>),
    /// A file dropped from the desktop onto a sampler row.
    DroppedOnSampler { channel: ChannelId, file: PathBuf },
    /// A file dropped from the desktop over the playlist, the window having seen the cursor at
    /// this window point ([`App::desktop_drop`]).
    DroppedOnPlaylist { file: PathBuf, seen: Point },
    /// The instrument sets and shipped folders of sounds a look on disk found, and the shipped
    /// sets' kinds.
    SetsFound(Found, crate::ui::library::Kinds),
    /// A folder's listing, or why it cannot be read.
    Listed {
        dir: PathBuf,
        entries: Result<Vec<Entry>, String>,
    },
    /// The search index, and whether its limit cut it short.
    Indexed(Arc<(Vec<Entry>, bool)>),
    /// Time to look at the preview's end and the files loading.
    Tick,
}

/// What a press held on a Library row holds.
#[derive(Debug, Clone)]
pub(super) enum Held {
    /// A folder of the tree, which only takes the press.
    Folder,
    /// A sound file: the tree's, or the starter kit's.
    File(PathBuf),
    /// A plugin instrument.
    Plugin(Offered),
    /// An instrument set: its main .sfz file and its name.
    Instrument { sfz: PathBuf, name: String },
    /// A MIDI device, by its port's name.
    Device(String),
    /// "External input": a new input channel.
    Input,
}

/// A press held on a Library row.
#[derive(Debug, Clone)]
pub(super) struct Press {
    pub held: Held,
    /// What the drag's ghost says.
    pub name: String,
    from: Point,
    /// Where the cursor is, window coordinates.
    pub at: Point,
    pub dragging: bool,
}

impl Press {
    pub(super) fn new(held: Held, name: String, at: Point) -> Self {
        Self {
            held,
            name,
            from: at,
            at,
            dragging: false,
        }
    }
}

/// The menu open under a Library row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RowMenu {
    /// Remove, under the tree's row of a folder the user added.
    Remove(usize),
    /// Unlink, under a MIDI device linked to a channel.
    Unlink(String),
}

/// A preview's sound, made ready on a thread of its own.
pub(super) struct PreparedPreview {
    file: PathBuf,
    source: Box<dyn ClipSource>,
    notices: Vec<SampleNotice>,
}

/// The Library panel's state in the window.
pub(super) struct Browse {
    /// The Samples section's tree.
    pub model: Browser,
    /// Which sections are closed, and the list's scroll.
    pub library: Library,
    pub cache: canvas::Cache,
    pub press: Option<Press>,
    /// The menu open under a row.
    pub menu: Option<RowMenu>,
    pub rack_hover: bool,
    /// The sampler row a file held from the browser is over: let go there, the row takes it.
    pub drop_sampler: Option<ChannelId>,
    /// The rack row a MIDI device held from the Library is over: let go there, that channel
    /// plays the device.
    pub drop_channel: Option<ChannelId>,
    /// The channel whose header a sound, a set or a plugin held from the Library is over: let
    /// go there, the channel takes that sound.
    pub drop_header: Option<ChannelId>,
    /// The desktop drop on the playlist's tracks under way, for its next file.
    pub tracks_drop: Option<super::desktop_drop::TracksDrop>,
    /// The search index is being built.
    indexing: bool,
    /// When the preview playing ends by itself.
    preview_until: Option<Instant>,
    /// When the preview playing is cut short: a set's note is.
    pub preview_stop: Option<Instant>,
    /// The instrument row (a kit sound, a set or a plugin) last pressed: Enter adds it.
    pub chosen: Option<Pick>,
    /// The instrument row whose press is held, until it is let go of or dragged.
    pub pressing: Option<Pick>,
    /// The instrument row last clicked (pressed and let go of without a drag), and when: a press
    /// on it soon after adds it to the rack.
    pub clicked: Option<(Instant, Pick)>,
    /// Where the folders added are kept; `None` keeps them in memory.
    folders_file: Option<PathBuf>,
    /// Previews being made ready, and the ones made ready, taken on the media timer.
    converting: usize,
    previews: (
        mpsc::Sender<PreparedPreview>,
        mpsc::Receiver<PreparedPreview>,
    ),
}

impl std::fmt::Debug for Browse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Browse")
            .field("model", &self.model)
            .field("press", &self.press)
            .field("menu", &self.menu)
            .field("converting", &self.converting)
            .finish_non_exhaustive()
    }
}

/// The folders the user added to the Library that `host`'s window starts with: those its
/// folders file keeps, none when it lists one given folder alone (`--browser-root`).
pub fn added_folders(host: &Host) -> Vec<PathBuf> {
    if host.browser_root.is_some() {
        return Vec::new();
    }
    Folders::load(host.browser_folders.as_deref()).folders
}

impl Browse {
    /// The browser of `host`'s roots (or its one given root, listed at once so it shows from
    /// the first frame), with `project_dir` as the project folder.
    pub(super) fn new(host: &Host, project_dir: Option<PathBuf>) -> Self {
        let model = match &host.browser_root {
            Some(root) => {
                let mut model = Browser::given(root.clone());
                for dir in model.unlisted() {
                    let entries = browser_model::list(&dir).unwrap_or_default();
                    model.listed(dir, entries);
                }
                model
            }
            None => Browser::new(project_dir, host.music.clone(), &added_folders(host)),
        };
        Self {
            model,
            library: Library::default(),
            cache: canvas::Cache::new(),
            press: None,
            menu: None,
            rack_hover: false,
            drop_sampler: None,
            drop_channel: None,
            drop_header: None,
            tracks_drop: None,
            indexing: false,
            preview_until: None,
            preview_stop: None,
            chosen: None,
            pressing: None,
            clicked: None,
            folders_file: host.browser_folders.clone(),
            converting: 0,
            previews: mpsc::channel(),
        }
    }

    /// Whether a timer has something to look at.
    pub(super) fn waiting(&self) -> bool {
        self.preview_until.is_some() || self.preview_stop.is_some() || self.converting > 0
    }

    /// What is being dragged from the Library, if anything.
    fn dragged(&self) -> Option<&Held> {
        self.press
            .as_ref()
            .filter(|press| press.dragging)
            .map(|press| &press.held)
    }

    /// Whether a sound file is being dragged: a sampler row it is over takes it.
    pub(super) fn dragging_file(&self) -> bool {
        matches!(self.dragged(), Some(Held::File(_)))
    }

    /// Whether what is dragged lands on a playlist lane (and on a channel's header): a sound
    /// file, a plugin, a set or the external input.
    pub(super) fn dragging_to_tracks(&self) -> bool {
        matches!(
            self.dragged(),
            Some(Held::File(_) | Held::Plugin(_) | Held::Instrument { .. } | Held::Input)
        )
    }

    /// Whether a MIDI device is being dragged: a rack row it is over takes it.
    pub(super) fn dragging_device(&self) -> bool {
        matches!(self.dragged(), Some(Held::Device(_)))
    }
}

/// A sound loaded for its waveform: the path the project names its file by, the edits it
/// plays through, and its look.
type LoadedLook = (PathBuf, SoundEdits, FileLook);

/// What the playlist draws of files and automations besides the document.
#[derive(Debug)]
pub(super) struct Looks {
    /// Each audio channel's sound as loaded, by the path the project names its file by and
    /// the edits it plays through.
    loaded: FileLooks,
    /// As the playlist shows them: as loaded, or loading while playback still converts the
    /// file or makes its edited sound.
    pub files: FileLooks,
    /// What each automation sets, as its clips are titled.
    pub names: BTreeMap<AutomationId, String>,
    /// The audio clips whose stretched sound is still being made.
    pub stretching: BTreeSet<ClipId>,
    /// The files the loaders finished, taken on the media timer.
    done: (mpsc::Sender<LoadedLook>, mpsc::Receiver<LoadedLook>),
}

impl Default for Looks {
    fn default() -> Self {
        Self {
            loaded: FileLooks::new(),
            files: FileLooks::new(),
            names: BTreeMap::new(),
            stretching: BTreeSet::new(),
            done: mpsc::channel(),
        }
    }
}

impl Looks {
    /// Whether a file is still loading for its waveform or for playback.
    pub(super) fn waiting(&self) -> bool {
        self.files
            .values()
            .flat_map(BTreeMap::values)
            .any(|look| *look == FileLook::Loading)
    }
}

impl App {
    pub(super) fn browser_message(&mut self, message: BrowserMessage) -> Task<Message> {
        self.browse.cache.clear();
        match message {
            BrowserMessage::Pressed { row, button, at } => {
                // A press elsewhere in the Library than on an instrument ends its preview.
                let result = self.end_library_preview();
                self.report(result);
                return self.press_row(row, button, at);
            }
            BrowserMessage::Picked { pick, button, at } => return self.picked(pick, button, at),
            BrowserMessage::Section(section) => {
                let result = self.end_library_preview();
                self.report(result);
                self.focus = Panel::Browser;
                self.browse.menu = None;
                self.browse.library.toggle(section);
                self.scroll_library(0);
            }
            BrowserMessage::Moved(at) => {
                if let Some(press) = &mut self.browse.press {
                    press.at = at;
                    let draggable = !matches!(press.held, Held::Folder);
                    if draggable && !press.dragging && press.from.distance(at) >= DRAG_DISTANCE {
                        press.dragging = true;
                    }
                }
            }
            BrowserMessage::Released => {
                let pressing = self.browse.pressing.take();
                if let Some(press) = self.browse.press.take() {
                    if press.dragging {
                        self.drop_held(press.held);
                    } else {
                        self.browse.clicked = pressing.map(|pick| (Instant::now(), pick));
                    }
                }
                self.end_drag();
            }
            BrowserMessage::Scrolled(rows) => self.scroll_library(rows),
            BrowserMessage::Search(text) => {
                self.search_typing = true;
                let model = &mut self.browse.model;
                model.search = text;
                model.selected = None;
                self.browse.library.top = 0;
                return self.index_if_needed();
            }
            BrowserMessage::RemoveChosen => {
                let row = match self.browse.menu.take() {
                    Some(RowMenu::Remove(row)) => Some(row),
                    _ => None,
                };
                let path = row
                    .and_then(|row| self.browse.model.rows().into_iter().nth(row))
                    .map(|row| row.path);
                if let Some(path) = path {
                    self.browse.model.remove_folder(&path);
                    return self.folders_changed();
                }
            }
            BrowserMessage::UnlinkChosen => {
                if let Some(RowMenu::Unlink(port)) = self.browse.menu.take() {
                    self.unlink_device(&port);
                }
            }
            BrowserMessage::Key(nav) => return self.navigate(nav),
        }
        Task::none()
    }

    /// Scrolls the Library's list the least it can to show the tree's selected row. Why: the
    /// tree sits under the Instruments section, so walking it with the keys would otherwise
    /// leave the selection below the panel's edge.
    fn reveal_selection(&mut self) {
        let Some(selected) = self.browse.model.selected else {
            return;
        };
        let row = self
            .library_items()
            .iter()
            .position(|item| *item == Item::Tree(selected));
        if let Some(row) = row {
            let height = self.panels_height();
            self.browse
                .library
                .reveal(row, browser::visible_rows(height));
        }
    }

    /// Scrolls the Library's list by `rows` (up is negative) within the rows it has, in the
    /// panel's height under the toolbar.
    pub(super) fn scroll_library(&mut self, rows: i64) {
        let height = self.panels_height();
        let total = self.library_items().len();
        self.browse
            .library
            .scroll(rows, browser::visible_rows(height), total);
    }

    pub(super) fn media_message(&mut self, message: MediaMessage) -> Task<Message> {
        match message {
            MediaMessage::RackHover(over) => self.browse.rack_hover = over,
            MediaMessage::ImportAudio => {
                self.files.menu = false;
                if !self.host.background {
                    return Task::none();
                }
                let dir = self.document_dir();
                return Task::perform(pick_audio(dir), |files| {
                    Message::Media(MediaMessage::ImportPicked(files))
                });
            }
            MediaMessage::AddSampler => {
                self.focus = Panel::Rack;
                if !self.host.background {
                    return Task::none();
                }
                let dir = self.document_dir();
                return Task::perform(pick_sample(dir), |file| {
                    Message::Media(MediaMessage::SamplerPicked(file))
                });
            }
            MediaMessage::SamplerPicked(Some(file)) => {
                let edit = self.new_sampler_channel(&file);
                self.commit(edit);
            }
            MediaMessage::SamplerPicked(None) => {}
            MediaMessage::SamplerMenu { channel, at } => {
                self.focus = Panel::Rack;
                self.automate = None;
                self.sampler_menu = Some((channel, at));
            }
            MediaMessage::ChangeSample(channel) => {
                self.sampler_menu = None;
                self.focus = Panel::Rack;
                if !self.host.background {
                    return Task::none();
                }
                let dir = self.document_dir();
                return Task::perform(pick_sample(dir), move |file| {
                    Message::Media(MediaMessage::SampleChosen { channel, file })
                });
            }
            MediaMessage::SampleChosen {
                channel,
                file: Some(file),
            }
            | MediaMessage::DroppedOnSampler { channel, file } => {
                let edit = self.sampler_sample(channel, &file);
                self.commit(edit);
            }
            MediaMessage::SampleChosen { file: None, .. } => {}
            MediaMessage::OverSampler(over) => self.browse.drop_sampler = over,
            MediaMessage::OverChannel(over) => self.browse.drop_channel = over,
            MediaMessage::OverHeader(over) => self.browse.drop_header = over,
            MediaMessage::DroppedOnPlaylist { file, seen } => self.desktop_drop(&file, Some(seen)),
            MediaMessage::ImportPicked(files) => self.place_files(&files),
            MediaMessage::FileDropped(file) => self.desktop_drop(&file, None),
            MediaMessage::FolderPicked(Some(dir)) => {
                if self.browse.model.add_folder(dir) {
                    self.browse.cache.clear();
                    return self.folders_changed();
                }
            }
            MediaMessage::SetsFound(found, kinds) => {
                self.browse.library.sets = found.sets;
                self.browse.library.kinds = kinds;
                self.browse.model.set_shipped(&found.folders);
                self.browse.cache.clear();
                self.scroll_library(0);
                self.sets_found = true;
                return self.dump_adder();
            }
            MediaMessage::FolderPicked(None) => {}
            MediaMessage::Listed { dir, entries } => {
                let entries = entries.unwrap_or_else(|reason| {
                    self.notice = Some(format!("Cannot list {}: {reason}", dir.display()));
                    Vec::new()
                });
                self.browse.model.listed(dir, entries);
                self.browse.cache.clear();
            }
            MediaMessage::Indexed(index) => {
                self.browse.indexing = false;
                let (entries, cut) = Arc::unwrap_or_clone(index);
                if cut {
                    self.notice = Some(format!(
                        "The search covers the first {} files",
                        browser_model::SEARCH_LIMIT
                    ));
                }
                self.browse.model.index = Some((entries, cut));
                self.browse.cache.clear();
            }
            MediaMessage::Tick => {
                if self
                    .browse
                    .preview_stop
                    .is_some_and(|stop| Instant::now() >= stop)
                {
                    self.stop_preview();
                }
                if self
                    .browse
                    .preview_until
                    .is_some_and(|until| Instant::now() >= until)
                {
                    self.browse.preview_until = None;
                    self.browse.model.previewing = None;
                    self.browse.cache.clear();
                }
                let prepared: Vec<PreparedPreview> = self.browse.previews.1.try_iter().collect();
                for preview in prepared {
                    self.browse.converting = self.browse.converting.saturating_sub(1);
                    self.start_preview(preview);
                }
                let loaded: Vec<LoadedLook> = self.looks.done.1.try_iter().collect();
                for (path, edits, look) in loaded {
                    self.looks
                        .loaded
                        .entry(path)
                        .or_default()
                        .insert(edits, look);
                }
                self.forget_edited_looks();
                self.poll_sounds();
                self.show_files();
            }
        }
        Task::none()
    }

    /// A press on browser row `row`: a folder opens or closes, a file starts or stops its
    /// preview (and may be dragged), "+ Add folder…" asks for one, and the right button on a
    /// folder the user added offers Remove.
    fn press_row(&mut self, row: usize, button: mouse::Button, at: Point) -> Task<Message> {
        self.focus = Panel::Browser;
        self.browse.menu = None;
        self.browse.chosen = None;
        self.browse.clicked = None;
        let Some(shown) = self.browse.model.rows().into_iter().nth(row) else {
            return Task::none();
        };
        self.browse.model.selected = Some(row);
        if button == mouse::Button::Right {
            if let RowKind::Root { user: true, .. } = shown.kind {
                self.browse.menu = Some(RowMenu::Remove(row));
            }
            return Task::none();
        }
        match shown.kind {
            RowKind::Root { .. } | RowKind::Folder { .. } => {
                self.browse.press = Some(Press::new(Held::Folder, shown.name, at));
                self.toggle_folder(&shown.path)
            }
            RowKind::File => {
                self.toggle_preview(&shown.path);
                let held = Held::File(shown.path);
                self.browse.press = Some(Press::new(held, shown.name, at));
                Task::none()
            }
            RowKind::AddFolder => {
                if !self.host.background {
                    return Task::none();
                }
                Task::perform(pick_folder(), |dir| {
                    Message::Media(MediaMessage::FolderPicked(dir))
                })
            }
        }
    }

    /// Opens or closes a folder, listing it when it opens for the first time.
    fn toggle_folder(&mut self, dir: &Path) -> Task<Message> {
        match self.browse.model.toggle(dir) {
            Some(dir) => self.list(dir),
            None => Task::none(),
        }
    }

    /// Lists `dir` in the background (at once when nothing runs in the background).
    fn list(&mut self, dir: PathBuf) -> Task<Message> {
        let work = {
            let dir = dir.clone();
            move || browser_model::list(&dir).map_err(|error| error.to_string())
        };
        if !self.host.background {
            let entries = work();
            return self.media_message(MediaMessage::Listed { dir, entries });
        }
        Task::perform(blocking(work), move |entries| {
            let entries =
                entries.unwrap_or_else(|| Err("the folder could not be listed".to_owned()));
            Message::Media(MediaMessage::Listed { dir, entries })
        })
    }

    /// Builds the search index in the background once the search box holds text.
    fn index_if_needed(&mut self) -> Task<Message> {
        let browse = &mut self.browse;
        if browse.model.search.is_empty() || browse.model.index.is_some() || browse.indexing {
            return Task::none();
        }
        browse.indexing = true;
        let roots: Vec<PathBuf> = browse
            .model
            .roots
            .iter()
            .map(|root| root.path.clone())
            .collect();
        let work = move || Arc::new(browser_model::search_index(&roots));
        if !self.host.background {
            return self.media_message(MediaMessage::Indexed(work()));
        }
        Task::perform(blocking(work), |index| {
            Message::Media(MediaMessage::Indexed(
                index.unwrap_or_else(|| Arc::new((Vec::new(), false))),
            ))
        })
    }

    /// Up and Down move the selection, keeping it in view; Right opens the selected folder and
    /// Left closes it, Enter previews the selected file or opens or closes the selected folder.
    fn navigate(&mut self, nav: Nav) -> Task<Message> {
        // A group's label pressed last: Enter folds or opens it, Left folds, Right opens; Up and
        // Down walk the tree as ever.
        if let Some(Pick::Group(key)) = self.browse.chosen.clone() {
            let fold = match nav {
                Nav::Enter => Some(None),
                Nav::Left => Some(Some(true)),
                Nav::Right => Some(Some(false)),
                Nav::Up | Nav::Down => None,
            };
            if let Some(fold) = fold {
                self.fold_group(&key, fold);
                return Task::none();
            }
        }
        if nav == Nav::Enter && self.add_chosen() {
            return Task::none();
        }
        if nav != Nav::Enter {
            // The arrows walk the tree: Enter acts on it from then on.
            self.browse.chosen = None;
        }
        let model = &mut self.browse.model;
        let row = model
            .selected
            .and_then(|row| model.rows().into_iter().nth(row));
        match (nav, row) {
            (Nav::Up, _) => {
                model.move_selection(-1);
                self.reveal_selection();
            }
            (Nav::Down, _) => {
                model.move_selection(1);
                self.reveal_selection();
            }
            (Nav::Right, Some(row)) if is_closed_folder(row.kind) => {
                return self.toggle_folder(&row.path)
            }
            (Nav::Left, Some(row)) if !is_closed_folder(row.kind) && row.kind != RowKind::File => {
                return self.toggle_folder(&row.path)
            }
            (Nav::Enter, Some(row)) if row.kind == RowKind::File => self.toggle_preview(&row.path),
            (Nav::Enter, Some(row)) if row.kind != RowKind::AddFolder => {
                return self.toggle_folder(&row.path)
            }
            _ => {}
        }
        Task::none()
    }

    /// Previews `file`, or stops the preview when it is the one playing. Its sound is made
    /// ready (decoded, converted to the project's rate, or opened as a stream) on a thread of
    /// its own and starts when the media timer takes it; at once only when nothing runs in the
    /// background. Why every file, short ones too: decoding and converting a file takes long
    /// enough to stall the window (a 10-minute file at another rate took 0.6 s optimized).
    pub(super) fn toggle_preview(&mut self, file: &Path) {
        if self.browse.model.previewing.as_deref() == Some(file) {
            self.stop_preview();
            return;
        }
        self.browse.model.previewing = Some(file.to_path_buf());
        self.browse.preview_until = None;
        self.browse.preview_stop = None;
        let (rate, threshold) = (
            self.session.project().sample_rate,
            self.session.stream_threshold(),
        );
        if self.host.background {
            let done = self.browse.previews.0.clone();
            let file = file.to_path_buf();
            self.browse.converting += 1;
            let spawned = std::thread::Builder::new()
                .name("dawsome-preview".into())
                .spawn(move || {
                    let (source, notices) = dawsome_media::preview(&file, rate, threshold);
                    let _ = done.send(PreparedPreview {
                        file,
                        source,
                        notices,
                    });
                });
            if spawned.is_err() {
                self.browse.converting -= 1;
                self.browse.model.previewing = None;
            }
            return;
        }
        let (source, notices) = dawsome_media::preview(file, rate, threshold);
        self.start_preview(PreparedPreview {
            file: file.to_path_buf(),
            source,
            notices,
        });
    }

    /// Plays a preview made ready, unless another file was picked (or Stop pressed) meanwhile.
    fn start_preview(&mut self, prepared: PreparedPreview) {
        let PreparedPreview {
            file,
            source,
            notices,
        } = prepared;
        if self.browse.model.previewing.as_deref() != Some(file.as_path()) {
            return;
        }
        if let Some(notice) = notices.first() {
            self.notice = Some(notice.to_string());
        }
        self.session.play_preview(source);
        // The preview ends by itself when the file does; a file whose length its header does
        // not give stays lit until stopped.
        self.browse.preview_until = probe(&file).ok().and_then(|info| {
            let frames = info.frames?;
            let seconds = frames as f64 / f64::from(info.sample_rate.max(1));
            Some(Instant::now() + Duration::from_secs_f64(seconds))
        });
    }

    /// Stops the preview, if one plays. Returns whether one did.
    pub(super) fn stop_preview(&mut self) -> bool {
        self.browse.preview_stop = None;
        if self.browse.model.previewing.take().is_none() {
            return false;
        }
        self.browse.preview_until = None;
        self.browse.cache.clear();
        let result = self.session.stop_preview();
        self.report(result);
        true
    }

    /// Shows the starter kit's first sound held from the Library over `channel`'s header, as a
    /// drag there shows it (its outline and the name beside the cursor): `--drag-over`, for a
    /// screenshot of the drop target. Nothing is let go of, so nothing is written or dropped.
    pub fn drag_over_header(&mut self, channel: ChannelId) {
        let Some(row) = self.rack_rows().row_of(channel) else {
            return;
        };
        let header = crate::ui::layout::channel_header_rect(row);
        let left = if self.panels.browser {
            crate::ui::browser::WIDTH
        } else {
            0.0
        };
        // Over the knobs, the rack's rows unscrolled under the toolbar and the rack's header.
        let at = iced::Point::new(
            left + header.x + header.width - 20.0,
            crate::ui::layout::TOOLBAR_HEIGHT
                + crate::ui::layout::RACK_HEADER_HEIGHT
                + header.center_y(),
        );
        let name = crate::starter_kit::SOUNDS[0].name.to_owned();
        let mut press = Press::new(Held::File(PathBuf::from(format!("{name}.wav"))), name, at);
        press.dragging = true;
        self.browse.press = Some(press);
        self.browse.rack_hover = true;
        self.browse.drop_header = Some(channel);
    }

    /// Opens `channel`'s menu as a right press at the middle of its rack name does:
    /// `--channel-menu`, for a screenshot of the menu.
    pub fn open_channel_menu(&mut self, channel: ChannelId) {
        // The row the rack draws it on, the takes folded.
        if let Some(row) = self.rack_rows().row_of(channel) {
            let at = crate::ui::layout::name_button_rect(row).center();
            let _ = self.update(Message::Media(MediaMessage::SamplerMenu { channel, at }));
        }
    }

    /// Selects playlist clip `clip` alone, as a click on it does: `--select-clip`, for the Help
    /// guide's picture of the link control. Whether the playlist holds it.
    pub fn select_clip(&mut self, clip: dawsome_engine::project::ClipId) -> bool {
        let held = self.session.project().playlist.clips.contains_key(&clip);
        if held {
            self.arranger.selection = std::collections::BTreeSet::from([clip]);
        }
        held
    }

    /// Opens the playlist's curve menu on `automation`'s first tension handle in view, as a
    /// right press there does: `--curve-menu`, for a screenshot of the menu. Whether it opened.
    pub fn open_curve_menu(&mut self, automation: dawsome_engine::project::AutomationId) -> bool {
        let project = self.session.project();
        self.arranger.open_curve_menu(project, automation)
    }

    /// Esc while a file is dragged: the drag ends and nothing lands.
    pub(super) fn cancel_browser_drag(&mut self) -> bool {
        let dragging = self.browse.press.take().is_some_and(|press| press.dragging);
        self.end_drag();
        dragging
    }

    fn end_drag(&mut self) {
        self.browse.rack_hover = false;
        self.browse.drop_sampler = None;
        self.browse.drop_channel = None;
        self.browse.drop_header = None;
        self.arranger.hover = None;
        self.tracks_cache.clear();
    }

    /// What a drag from the Library let go of becomes.
    fn drop_held(&mut self, held: Held) {
        match held {
            Held::File(file) => self.drop_file(&file),
            Held::Plugin(plugin) => self.drop_plugin(&plugin),
            Held::Instrument { sfz, name } => self.drop_instrument(&sfz, &name),
            Held::Device(port) => self.drop_device(&port),
            Held::Input => self.drop_input(),
            Held::Folder => {}
        }
    }

    /// A file let go of: on a playlist lane it becomes a new audio channel and one clip of it
    /// at the floor-snapped tick there; on a channel's header, that channel's sound, played as
    /// a sampler; elsewhere on a sampler row, that sampler's sound; elsewhere on the rack, a new
    /// sampler channel; anywhere else, nothing.
    fn drop_file(&mut self, file: &Path) {
        let project = self.session.project();
        if let (true, Some(channel)) = (self.browse.rack_hover, self.browse.drop_header) {
            let source = SoundSource::sampler(self.sample_path(file));
            self.replace_sound(channel, source, &stem(file));
            return;
        }
        let edit = if let Some((track, tick)) = self.arranger.drop_target(project) {
            self.new_audio_clip(project, file, tick, Some(track))
        } else if self.browse.rack_hover {
            Ok(Some(match self.browse.drop_sampler {
                Some(channel) => self.sampler_sample(channel, file),
                None => self.new_sampler_channel(file),
            }))
        } else {
            Ok(None)
        };
        match edit {
            Ok(Some(edit)) => self.commit(edit),
            Ok(None) => {}
            Err(notice) => self.notice = Some(notice),
        }
    }

    /// The edit that adds `file` as a new sampler channel at the end of the rack, named after
    /// it: what a file dropped on the rack and "+ Sampler…" both do.
    fn new_sampler_channel(&self, file: &Path) -> Edit {
        Edit::AddSamplerChannel {
            id: self.session.project().next_channel_id,
            name: stem(file),
            sample: self.sample_path(file),
        }
    }

    /// The edit that gives sampler `channel` the sound in `file`, named as a new sampler would
    /// name it: what a drop on its row, its menu and a double press on its name all do.
    fn sampler_sample(&self, channel: ChannelId, file: &Path) -> Edit {
        Edit::SetChannelSample {
            id: channel,
            path: self.sample_path(file),
        }
    }

    /// The channel menu where the right press on a name was, on the window's top layer, when it
    /// is open: "Instrument window…", "Plugin window…" and "Parameters…" (plugin channels),
    /// "Mixer insert…", "Change sample…" (samplers), "Edit sample…" (samplers and audio channels with a file),
    /// "One note at a time" ticked while on (samplers and sets), Rename, Clone and Delete.
    pub(super) fn sampler_menu_view(&self) -> Option<Element<'_, Message>> {
        use crate::ui::hint::ChannelRow;
        let (channel, at) = self.sampler_menu?;
        let command = |command| Message::Channel(command);
        let mut rows = column![];
        for (row, ticked) in self.channel_menu_rows() {
            let label = |label, message| crate::ui::menu::row(label, Some(message));
            let button = match row {
                ChannelRow::InstrumentWindow => label(
                    "Instrument window\u{2026}",
                    Message::OpenInstrument(channel),
                ),
                // What a double press on the name does: the plugin's own window, or its
                // parameters when it has none.
                ChannelRow::PluginWindow => {
                    label("Plugin window\u{2026}", Message::ChannelOpened(channel))
                }
                ChannelRow::Parameters => {
                    label("Parameters\u{2026}", Message::ChannelParams(channel))
                }
                ChannelRow::MixerInsert => label(
                    "Mixer insert\u{2026}",
                    command(ChannelCommand::MixerInsert(channel)),
                ),
                ChannelRow::ChangeSample => label(
                    "Change sample\u{2026}",
                    Message::Media(MediaMessage::ChangeSample(channel)),
                ),
                ChannelRow::EditSample => {
                    label("Edit sample\u{2026}", Message::OpenSampleEditor(channel))
                }
                ChannelRow::OneNoteAtATime => crate::ui::menu::check_row(
                    "One note at a time",
                    ticked == Some(true),
                    command(ChannelCommand::OneNoteAtATime(channel)),
                ),
                ChannelRow::Rename => label("Rename", command(ChannelCommand::Rename(channel))),
                ChannelRow::Clone => label("Clone", command(ChannelCommand::Clone(channel))),
                ChannelRow::Delete => label("Delete", command(ChannelCommand::Delete(channel))),
            };
            rows = rows.push(crate::ui::hint::hover_area(
                button,
                crate::ui::hint::Hover::Menu(crate::ui::hint::MenuRow::Channel(row)),
            ));
        }
        let menu = crate::ui::menu::panel(rows, SAMPLER_MENU_WIDTH);
        let floating = crate::ui::menu::floating(menu, self.hint_bar_height());
        Some(pin(floating).x(at.x).y(at.y).into())
    }

    /// The rows the open channel menu shows, in order, for its channel's kind, each with whether
    /// it is ticked when it is a switch (One note at a time: the channel's `cut_itself`); none
    /// while no channel menu is open.
    pub(super) fn channel_menu_rows(&self) -> Vec<(crate::ui::hint::ChannelRow, Option<bool>)> {
        use crate::ui::hint::ChannelRow;
        let Some(shown) = self
            .sampler_menu
            .and_then(|(channel, _)| self.session.project().channels.get(&channel))
        else {
            return Vec::new();
        };
        // A sampler with no sound yet has none to edit.
        let edits = !shown.sample.as_os_str().is_empty();
        ChannelRow::ALL
            .into_iter()
            .filter(|row| row.shown_for(shown.kind))
            .filter(|&row| row != ChannelRow::EditSample || edits)
            .map(|row| {
                let ticked = (row == ChannelRow::OneNoteAtATime).then_some(shown.sound.cut_itself);
                (row, ticked)
            })
            .collect()
    }

    /// Imported or dropped files: each a new audio channel and one clip at the song marker, on
    /// successive tracks free over the clip (new ones added when none is), all one undo step.
    pub(super) fn place_files(&mut self, files: &[PathBuf]) {
        let mut project = self.session.project().clone();
        let mut edits = Vec::new();
        for file in files {
            let edit = match self.new_audio_clip(&project, file, self.song_marker, None) {
                Ok(Some(edit)) => edit,
                Ok(None) => continue,
                Err(notice) => {
                    self.notice = Some(notice);
                    continue;
                }
            };
            if edit::apply(&mut project, edit.clone()).is_err() {
                continue;
            }
            match edit {
                Edit::Transaction(inner) => edits.extend(inner),
                edit => edits.push(edit),
            }
        }
        match edits.len() {
            0 => {}
            1 => self.commit(edits.remove(0)),
            _ => self.commit(Edit::Transaction(edits)),
        }
    }

    /// The edit that adds `file` to `project` as a new audio channel and one clip of the whole
    /// file from `start`, on `track` (else the first track free over the clip, else a new one);
    /// what to tell the user when the file cannot be read.
    pub(super) fn new_audio_clip(
        &self,
        project: &Project,
        file: &Path,
        start: Ticks,
        track: Option<TrackId>,
    ) -> Result<Option<Edit>, String> {
        let frames = frames_at(file, project.sample_rate)
            .ok_or_else(|| format!("Cannot read {}", file.display()))?;
        Ok(record::new_audio_clip(
            project,
            self.sample_path(file),
            start,
            frames,
            track,
        ))
    }

    /// The path a new channel names `file` by: relative to the project's folder when the file
    /// lies under it, else absolute (an untitled document has no folder).
    pub(super) fn sample_path(&self, file: &Path) -> PathBuf {
        let base = match (&self.document.path, self.session.samples_dir()) {
            (None, dir) if dir.as_os_str().is_empty() => None,
            (Some(_), dir) if dir.as_os_str().is_empty() => Some(Path::new(".")),
            (_, dir) => Some(dir),
        };
        let stored = sample_path(file, base);
        // A file of the data folder's (the starter kit, a set) or of the installed sets is
        // named so that any user's data folder or install finds it; one under the project's
        // folder stays relative to it.
        let roots = dawsome_media::ShippedRoots {
            user: dawsome_media::shipped_root(),
            installed: self.host.installed_instruments.clone(),
        };
        let shipped = stored
            .is_absolute()
            .then(|| roots.reference(&stored))
            .flatten();
        shipped.unwrap_or(stored)
    }

    /// After a folder was added to the Library or removed: the folders saved, the sets inside
    /// them reading inside them ([`share_library`](Self::share_library)), and the sets looked
    /// for again.
    fn folders_changed(&mut self) -> Task<Message> {
        self.save_folders();
        self.share_library();
        self.scan_sets()
    }

    /// Hands the session the folders added to the Library: a set inside one reads anywhere
    /// inside it, so a library's `Instruments/x.sfz` plays its `../Samples/`.
    pub(super) fn share_library(&mut self) {
        let folders = self.browse.model.user_folders().folders;
        let shared = self.session.set_library(folders);
        self.report(shared);
    }

    /// Saves the folders the user added, saying so when that fails.
    fn save_folders(&mut self) {
        let Some(file) = self.browse.folders_file.clone() else {
            return;
        };
        if let Err(error) = self.browse.model.user_folders().save(&file) {
            self.notice = Some(format!("Browser folders not saved: {error}"));
        }
    }

    /// The project folder the browser shows, after the document moved or changed, the Library
    /// back at its top when that folder changed; the Library shows the new document's links too.
    pub(super) fn show_project_folder(&mut self) {
        if self.browse.model.editable {
            let dir = self.document_dir();
            let recordings = self.recordings_dir().filter(|dir| dir.is_dir());
            let project = self.browse.model.set_project(dir);
            if self.browse.model.set_recordings(recordings) || project {
                self.browse.library.top = 0;
            }
        }
        self.scroll_library(0);
        self.browse.cache.clear();
    }

    /// After a take was written to `file`: the Library's Recordings root (the folder may be new)
    /// and the listings that show its folder read again, so the take is there to drag.
    pub(super) fn show_recordings(&mut self, file: &Path) {
        self.show_project_folder();
        let Some(dir) = file.parent() else {
            return;
        };
        for listed in [Some(dir), dir.parent()].into_iter().flatten() {
            if self.browse.model.listings.contains_key(listed) {
                let entries = browser_model::list(listed).unwrap_or_default();
                self.browse.model.listed(listed.to_path_buf(), entries);
            }
        }
        self.browse.model.index = None;
        self.browse.cache.clear();
    }

    /// Forgets the old document's waveforms and the preview, for a new document.
    pub(super) fn reset_media(&mut self) {
        self.looks = Looks::default();
        self.browse.press = None;
        self.browse.model.previewing = None;
        self.browse.preview_until = None;
        self.refresh_looks();
    }

    /// Brings what the playlist draws in line with the document: each automation's name, and
    /// each audio channel's sound (its file through its edits), loaded in the background the
    /// first time it is named, at once when the sample editor shows it already (its peaks are
    /// the ones a load would make).
    pub(super) fn refresh_looks(&mut self) {
        let project = self.session.project();
        self.looks.names = project
            .automation
            .list
            .iter()
            .filter_map(|(&id, automation)| {
                let name =
                    clips::automation_name(project, self.session.registry(), &automation.target)?;
                Some((id, name))
            })
            .collect();
        let new: Vec<(PathBuf, SoundEdits)> = project
            .channels
            .values()
            .filter(|channel| channel.kind == ChannelKind::Audio)
            .filter(|channel| clips::look_of(&self.looks.loaded, channel).is_none())
            .map(|channel| (channel.sample.clone(), channel.edits.clone()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for (path, edits) in new {
            let shown = self.editor_look(&path, &edits);
            let look = shown.clone().unwrap_or(FileLook::Loading);
            let loaded = self.looks.loaded.entry(path.clone()).or_default();
            loaded.insert(edits.clone(), look);
            if shown.is_none() {
                self.load_look(path, edits);
            }
        }
        self.forget_edited_looks();
        self.show_files();
    }

    /// The look of `path` through `edits` from the sample editor's peaks, when it shows that
    /// sound now.
    fn editor_look(&self, path: &Path, edits: &SoundEdits) -> Option<FileLook> {
        let sound = self.sample_editor.as_ref()?.sound.as_ref()?;
        (sound.holds(path, edits) && sound.problem.is_none())
            .then(|| FileLook::Ready(sound.peaks.clone()))
    }

    /// Lets go of the looks of edited sounds no audio channel plays now (each edit of a long
    /// sound would keep its peaks otherwise: 1.8 MB for ten minutes of stereo); a file's own
    /// look stays, as it always has.
    fn forget_edited_looks(&mut self) {
        let project = self.session.project();
        let playing: BTreeSet<(&Path, &SoundEdits)> = project
            .channels
            .values()
            .filter(|channel| channel.kind == ChannelKind::Audio)
            .map(|channel| (channel.sample.as_path(), &channel.edits))
            .collect();
        for (path, looks) in &mut self.looks.loaded {
            looks.retain(|edits, _| edits.is_empty() || playing.contains(&(path.as_path(), edits)));
        }
    }

    /// Loads the file a project names by `path` for its waveform, played through `edits`, on a
    /// thread of its own (at once when nothing runs in the background); the media timer takes
    /// the result.
    fn load_look(&mut self, path: PathBuf, edits: SoundEdits) {
        let (dir, rate) = (
            self.session.samples_dir().to_path_buf(),
            self.session.project().sample_rate,
        );
        if !self.host.background {
            let look = look_of(&dir, &path, rate, &edits);
            self.looks
                .loaded
                .entry(path)
                .or_default()
                .insert(edits, look);
            return;
        }
        let done = self.looks.done.0.clone();
        let (name, list) = (path.clone(), edits.clone());
        let spawned = std::thread::Builder::new()
            .name("dawsome-waveform".into())
            .spawn(move || {
                let look = look_of(&dir, &name, rate, &list);
                let _ = done.send((name, list, look));
            });
        if spawned.is_err() {
            self.looks
                .loaded
                .entry(path)
                .or_default()
                .insert(edits, FileLook::Missing);
        }
    }

    /// The files as the playlist shows them: loading while playback converts them still. A
    /// file whose clip is being stretched shows as it is: the clip says it is stretching.
    fn show_files(&mut self) {
        let clips = self.stretching();
        let stretching: BTreeSet<PathBuf> = clips.values().cloned().collect();
        let clips: BTreeSet<ClipId> = clips.into_keys().collect();
        if clips != self.looks.stretching {
            self.looks.stretching = clips;
            self.tracks_cache.clear();
        }
        let files: FileLooks = self
            .looks
            .loaded
            .iter()
            .map(|(path, looks)| {
                let loading = self.session.is_loading(path) && !stretching.contains(path);
                let shown = looks
                    .iter()
                    .map(|(edits, look)| {
                        let look = if loading {
                            FileLook::Loading
                        } else {
                            look.clone()
                        };
                        (edits.clone(), look)
                    })
                    .collect();
                (path.clone(), shown)
            })
            .collect();
        if files != self.looks.files {
            self.looks.files = files;
            self.tracks_cache.clear();
        }
    }

    /// The audio clips whose stretched sound is still being made, with their files.
    fn stretching(&self) -> BTreeMap<ClipId, PathBuf> {
        let project = self.session.project();
        project
            .playlist
            .clips
            .iter()
            .filter_map(|(&id, clip)| {
                let ClipItem::Audio {
                    channel,
                    stretch: Some(stretch),
                    ..
                } = clip.item
                else {
                    return None;
                };
                let file = &project.channels.get(&channel)?.sample;
                self.session
                    .is_stretching(file, stretch)
                    .then(|| (id, file.clone()))
            })
            .collect()
    }

    /// The frames of the sound audio clip `id` plays (its channel's file through its edits) at
    /// the project's rate, and the fewest frames a time-stretch of it can make; `None`, with a
    /// notice, while it is not loaded.
    fn clip_file_frames(&mut self, id: ClipId) -> Option<(u64, u64)> {
        let project = self.session.project();
        let channel = project
            .playlist
            .clips
            .get(&id)
            .and_then(|clip| clip.audio_channel())
            .and_then(|channel| project.channels.get(&channel));
        let Some(FileLook::Ready(peaks)) =
            channel.and_then(|channel| clips::look_of(&self.looks.loaded, channel))
        else {
            self.notice = Some("The clip's file is not loaded yet: try again".to_owned());
            return None;
        };
        let channels = u8::try_from(peaks.channels.len()).unwrap_or(2);
        let shortest = dawsome_media::stretch::shortest_stretch(channels, project.sample_rate);
        Some((peaks.frames, shortest))
    }

    /// An Alt drag of audio clip `id`'s right edge ended at `end`: its file stretched by the
    /// clip's new length, or, when that cannot be made, a notice and nothing changed.
    pub(super) fn stretch_clip(&mut self, id: ClipId, end: Ticks) {
        let Some((frames, shortest)) = self.clip_file_frames(id) else {
            return;
        };
        let stretched = clip_gesture::stretched(self.session.project(), id, end, frames, shortest);
        self.commit_stretch(stretched);
    }

    /// The properties strip's Stretch choice for audio clip `id`.
    pub(super) fn set_clip_mode(&mut self, id: ClipId, mode: Option<StretchMode>) {
        let Some((frames, shortest)) = self.clip_file_frames(id) else {
            return;
        };
        let changed = clip_gesture::with_mode(self.session.project(), id, mode, frames, shortest);
        self.commit_stretch(changed);
    }

    /// Commits a stretch, or says why it cannot be made.
    fn commit_stretch(&mut self, stretch: Result<Option<Edit>, StretchRefused>) {
        match stretch {
            Ok(Some(edit)) => self.commit(edit),
            Ok(None) => {}
            Err(StretchRefused::Ratio) => {
                self.notice =
                    Some("A clip stretches from a quarter to four times its file".to_owned());
            }
            Err(StretchRefused::TooShort { shortest }) => {
                let rate = u64::from(self.session.project().sample_rate.max(1));
                let millis = shortest * 1000 / rate;
                self.notice = Some(format!("Too short to stretch: {millis} ms at least"));
            }
        }
    }

    /// What the properties strip shows of the clip it is open on: its Stretch choice, pitch
    /// and length over its file's, once the file is loaded.
    pub(super) fn clip_properties(&self) -> Option<ClipProperties> {
        let project = self.session.project();
        let clip = project.playlist.clips.get(&self.arranger.properties?)?;
        let ClipItem::Audio {
            channel, stretch, ..
        } = clip.item
        else {
            return None;
        };
        let channel = project.channels.get(&channel)?;
        let frames = match clips::look_of(&self.looks.files, channel) {
            Some(FileLook::Ready(peaks)) => Some(peaks.frames),
            _ => None,
        };
        Some(ClipProperties {
            mode: stretch.map(|stretch| stretch.mode),
            pitch: stretch.map_or(0, |stretch| stretch.pitch),
            percent: frames.map(|source| {
                clips::stretch_percent(stretch.map_or(source, |stretch| stretch.frames), source)
            }),
        })
    }

    /// Takes what the player posted: names the clips whose streams fell behind the disk or
    /// failed, and hands the live notes to a take of notes.
    pub(super) fn read_player_events(&mut self) {
        let events = self.session.take_events();
        // The playhead read with the events, after them, so it is never older than a note they
        // started (a note starts at its call's first sample, at or before the playhead the
        // player publishes after that call). Read only on the playhead's timer, a
        // note heard between two ticks (a port's arrival, a typed key) started past the roll's
        // playhead and drew as wrapped round the whole pattern for a frame.
        let position = self.session.position().ticks;
        self.playhead = self.playing.then_some(position);
        if let Some(notice) = stream_notice(self.session.project(), &events) {
            self.notice = Some(notice);
        }
        self.print_live_notes(&events);
        self.live_notes.hear(&events);
        // A note's start or end the player could not post (its ring was full) leaves the live
        // set unsure: it starts again empty, so no key stays lit with nothing held. A new
        // session's count starts again too.
        let lost = self.session.lost_live_events();
        if lost != self.live_lost {
            self.live_lost = lost;
            self.live_notes.clear();
        }
        self.hear_notes(&events, position);
        // Played while no take runs: what Capture lays out later.
        if !self.recording.recording() {
            self.capture.hear(&events, std::time::Instant::now());
        }
    }

    /// Compiles again once files have loaded in the background (or been converted for
    /// streaming), so their channels and clips play, and says what loading them found.
    fn poll_sounds(&mut self) {
        match self.session.poll_sounds() {
            Ok(true) => {
                if let Some(notice) = summarize_notices(&self.session.take_notices()) {
                    self.notice = Some(notice);
                }
                self.show_files();
            }
            Ok(false) => {}
            Err(error) => self.report(Err(error)),
        }
    }
}

/// What the toolbar says about `events`: the clips (by their channel's name) whose streams
/// failed, else those that fell behind; nothing when none did.
fn stream_notice(project: &Project, events: &[EngineEvent]) -> Option<String> {
    let name = |clip| {
        let channel = project.playlist.clips.get(&clip)?.audio_channel()?;
        Some(project.channels.get(&channel)?.name.clone())
    };
    let (mut failed, mut late) = (BTreeSet::new(), BTreeSet::new());
    for event in events {
        match *event {
            EngineEvent::StreamFailed { clip } => failed.extend(name(clip)),
            EngineEvent::StreamUnderrun { clip } => late.extend(name(clip)),
            // Live notes are not about the disk; a take of notes reads them.
            EngineEvent::LiveNoteStarted { .. } | EngineEvent::LiveNoteEnded { .. } => {}
        }
    }
    let names = |names: BTreeSet<String>| names.into_iter().collect::<Vec<_>>().join(", ");
    if !failed.is_empty() {
        return Some(format!(
            "Cannot read from disk: {} plays silence",
            names(failed)
        ));
    }
    (!late.is_empty()).then(|| format!("The disk fell behind: {} played silence", names(late)))
}

/// Whether a row is a folder or root that is closed.
fn is_closed_folder(kind: RowKind) -> bool {
    matches!(
        kind,
        RowKind::Root { open: false, .. } | RowKind::Folder { open: false }
    )
}

/// A file's name without its extension: a new channel's name.
fn stem(file: &Path) -> String {
    file.file_stem().map_or_else(
        || file.display().to_string(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

/// `file`'s length at `rate`: its header's length converted, as loading converts it (to
/// exactly ceil(n · rate / file rate) frames), else what decoding it gives; `None` when it
/// cannot be read.
fn frames_at(file: &Path, rate: u32) -> Option<u64> {
    let info = probe(file).ok()?;
    match info.frames {
        Some(frames) => {
            Some((frames * u64::from(rate)).div_ceil(u64::from(info.sample_rate.max(1))))
        }
        None => {
            let loaded = load_channel_sample(Path::new(""), file, rate);
            loaded.file_rate.map(|_| loaded.buffer.frames())
        }
    }
}

/// `file` as a project in `base` names it: relative when it lies under `base`, else absolute,
/// links resolved where the file exists; absolute when there is no `base`.
fn sample_path(file: &Path, base: Option<&Path>) -> PathBuf {
    let resolve = |path: &Path| {
        dunce::canonicalize(path)
            .unwrap_or_else(|_| std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()))
    };
    let file = resolve(file);
    let Some(base) = base else {
        return file;
    };
    match file.strip_prefix(resolve(base)) {
        Ok(inside) => inside.to_path_buf(),
        Err(_) => file,
    }
}

/// The waveform look of the file a project in `dir` names by `path`, played through `edits`
/// (the function playback applies them with, so the playlist draws what plays).
fn look_of(dir: &Path, path: &Path, rate: u32, edits: &SoundEdits) -> FileLook {
    let loaded = load_channel_sample(dir, path, rate);
    match loaded.file_rate {
        Some(_) if edits.is_empty() => FileLook::Ready(Peaks::of(&loaded.buffer)),
        Some(_) => FileLook::Ready(Peaks::of(&dawsome_engine::dsp::edits::apply(
            &loaded.buffer,
            edits,
        ))),
        None => FileLook::Missing,
    }
}

/// The desktop's folder picker, for "+ Add folder…".
async fn pick_folder() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("Add folder")
        .pick_folder()
        .await
        .map(|folder| folder.path().to_path_buf())
}

/// The desktop's Open dialog for sound files, titled `title`, opening in `dir`.
fn audio_dialog(title: &str, dir: Option<PathBuf>) -> rfd::AsyncFileDialog {
    let dialog = rfd::AsyncFileDialog::new()
        .set_title(title)
        .add_filter("Audio files", &browser_model::EXTENSIONS);
    match dir {
        Some(dir) => dialog.set_directory(dir),
        None => dialog,
    }
}

/// The one sound file "+ Sampler…" picks.
async fn pick_sample(dir: Option<PathBuf>) -> Option<PathBuf> {
    audio_dialog("New sampler channel", dir)
        .pick_file()
        .await
        .map(|file| file.path().to_path_buf())
}

/// The desktop's file picker for Import audio…, sound files only, several at once.
async fn pick_audio(dir: Option<PathBuf>) -> Vec<PathBuf> {
    audio_dialog("Import audio", dir)
        .pick_files()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|file| file.path().to_path_buf())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
    }

    // Loaded at the project's rate, p8_imp48k.wav's 12000 frames at 48 kHz are, at 44.1 kHz,
    // ceil(12000 · 44100 / 48000) = 11025; p8_saw.wav is at 44.1 kHz already.
    #[test]
    fn a_files_length_is_counted_at_the_projects_rate() {
        assert_eq!(
            frames_at(&fixtures().join("p8_imp48k.wav"), 44_100),
            Some(11_025)
        );
        assert_eq!(
            frames_at(&fixtures().join("p8_saw.wav"), 44_100),
            Some(30_000)
        );
        assert_eq!(frames_at(&fixtures().join("no-such.wav"), 44_100), None);
    }

    #[test]
    fn files_under_the_project_folder_are_named_relative_to_it() {
        let fixtures = fixtures();
        let file = fixtures.join("browser/tone.wav");
        assert_eq!(
            sample_path(&file, Some(&fixtures)),
            PathBuf::from("browser/tone.wav")
        );
        let elsewhere = sample_path(&file, Some(&fixtures.join("browser/drums")));
        assert!(elsewhere.is_absolute() && elsewhere.ends_with("browser/tone.wav"));
        assert!(
            sample_path(&file, None).is_absolute(),
            "an untitled document"
        );
    }
}
