//! Application state, messages, and the update and view functions.
//!
//! The app holds a [`Session`] (the document and its link to the player) and the state of the
//! window around it: which panels are open (the rack, the playlist, the piano roll, the mixer)
//! and which one has the keys, the panels' views and selections, and the song position marker.
//! Gestures become edits on the session, each one undo step; a dragged control sends its value
//! live and commits once on release. The playhead and the mixer's meters come back from the
//! audio thread through a 30 Hz timer that runs only while something moves, and the output
//! stream's health through a slower one that always runs. The document's file life (open,
//! save, autosave, the questions before losing changes) is in [`files`], export in [`export`].

mod automate;
mod browse;
mod clip_link;
mod controllers;
mod desktop_drop;
mod dividers;
mod editor_sound;
mod export;
mod files;
mod help_window;
mod input_channels;
mod inputs;
mod instrument;
mod interchange;
mod key_input;
mod landing;
mod latency;
mod library;
mod library_preview;
mod midi_files;
mod midi_outs;
mod note_preview;
mod param_window;
mod patterns;
mod plugins;
mod recording;
mod rehearse;
mod roll_edits;
mod sample_editor;
mod settings_window;
mod typing;
#[cfg(test)]
mod vst3_tests;
#[cfg(test)]
mod widget_events;

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ops::Bound;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use dawsome_engine::edit::{Edit, EditError};
use dawsome_engine::project::{
    ChannelId, ChannelKind, LoopRange, PatternId, ProcessorOwner, TEMPO_RANGE,
};
#[cfg(test)]
use dawsome_engine::realtime::Player;
use dawsome_engine::time::{MilliBpm, Ticks};
use dawsome_engine::transport::Control;
use dawsome_host_clap::scan::Scanned;
#[cfg(test)]
use iced::keyboard::Key;
use iced::keyboard::{key, Modifiers};
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{
    canvas, column, container, mouse_area, operation, pin, responsive, row, scrollable, space,
    stack, text, Id,
};
use iced::{event, mouse, time, window, Element, Event, Fill, Point, Size, Subscription, Task};

use crate::audio::{self, Output};
use crate::document::{Document, Host};
use crate::headless;
use crate::links::LinkDriver;
use crate::midi::{ControllerQueue, MidiPorts};
use crate::recent::Recent;
use crate::session::{Session, SessionError};
use crate::settings::Settings;
use crate::ui::arranger::{self, Arranger, PlaylistMessage};
use crate::ui::browser::{self, BrowserMessage, Nav};
use crate::ui::callout::CalloutMessage;
use crate::ui::dialog::{self, FileMenu};
use crate::ui::layout;
use crate::ui::mixer::{MixerPanel, RENAME_INPUT as MIXER_RENAME_INPUT};
use crate::ui::mixer_layout;
use crate::ui::mixer_state::{Context, MixerCommand, MixerDrag, MixerMessage, MixerState};
use crate::ui::note_gesture::Target;
use crate::ui::panels::{self, Panel, PanelUse, Panels};
use crate::ui::param_window::{ParamMessage, ParamPanel};
use crate::ui::piano_layout;
use crate::ui::piano_roll::PianoRoll;
use crate::ui::picker::{Picker, PickerMessage, PickerTarget};
use crate::ui::playlist::{Playlist, RENAME_INPUT};
use crate::ui::playlist_layout;
use crate::ui::rack::{ChannelRack, DeviceDrop, Knob, SoundDrop};
use crate::ui::roll::{Commit, Roll, RollMessage};
use crate::ui::sample_editor::SampleEditorMessage;
use crate::ui::settings_panel::SettingsMessage;
use crate::ui::step_grid;
use crate::ui::style;
use crate::ui::toolbar::{self, RecordLook, Toolbar, TypingLook};

pub use browse::{added_folders, MediaMessage};
pub use clip_link::{ClipLink, SelectionLink};
pub use dividers::DividerMessage;
pub use export::{ExportKind, ExportMessage};
pub use files::{Choice, FileMessage, Start};
pub use key_input::KeyInput;
pub use landing::{at_start as landing_at_start, Landing};
pub use patterns::{PatternMessage, RENAME_INPUT as PATTERN_RENAME_INPUT};
use plugins::PluginDrag;
pub use recording::{RecordMessage, TakeKind};

/// How often the playhead is read from the audio thread while the transport runs.
const PLAYHEAD_PERIOD: Duration = Duration::from_millis(33);
/// How often a rehearsed take (`--rehearse`) hands the player its messages: under a call's
/// length (11.6 ms at 44.1 kHz), so each lands on its call or the next.
const REHEARSAL_MS: u64 = 10;

/// How long the playhead timer runs past a live note's key or port message while the piano
/// roll is open, so the player's report of the note is read while the song is stopped.
const LIVE_WAKE: Duration = Duration::from_millis(500);

/// How often the MIDI ports' lights are looked at while one is lit: a third of the time a light
/// stays lit, so a light goes out within 50 ms of its time.
const MIDI_LIGHT_PERIOD: Duration = Duration::from_millis(50);

/// How often the output stream's health, and the notes plugins had no room for, are read. It
/// runs while stopped too, so a device that fails between plays shows at once rather than at the
/// next Play.
const AUDIO_CHECK_PERIOD: Duration = Duration::from_millis(500);

/// How often the output device is tried again while the player is parked without one.
const AUDIO_RETRY_PERIOD: Duration = Duration::from_secs(5);

/// How often the plugins' main-thread work runs while some of it comes due by the clock (CLAP
/// timers are served at this granularity), and how often their doorbells are looked at
/// otherwise.
const PLUGIN_PERIOD: Duration = Duration::from_millis(16);

/// How often the parameter window reads its values again while it is open.
const PANEL_PERIOD: Duration = Duration::from_millis(33);

/// Pixels of vertical drag that sweep a knob from one stop to the other.
const KNOB_DRAG_PIXELS: f32 = 200.0;

/// Pixels of vertical drag per tempo step.
const TEMPO_DRAG_PIXELS: f32 = 2.0;

/// How much slower a mixer control moves with Shift held.
const FINE_DRAG: f32 = 0.1;

/// The hint bar's text size.
const HINT_TEXT_SIZE: f32 = 12.0;

/// Tempo steps in milli-BPM: whole BPM, or 0.001 BPM with Shift held.
const TEMPO_STEP: u32 = 1000;

/// What a press on a chord without a note at the row's default key says.
const MELODY_STEP: &str = "A chord starts on that step: change it in the piano roll";

/// The id the tempo's box is focused by.
pub const TEMPO_INPUT: &str = "tempo";

/// Pixels a press on the tempo may move and still be a click, which types the tempo.
const CLICK_TRAVEL: f32 = 3.0;
const TEMPO_FINE_STEP: u32 = 1;

/// What the transport plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayMode {
    /// The selected pattern, looping.
    Pattern,
    /// The arrangement.
    Song,
}

/// Everything that can happen in the window.
#[derive(Debug, Clone)]
pub enum Message {
    Play,
    Stop,
    /// Space: play when stopped, stop when playing.
    TogglePlay,
    Undo,
    Redo,
    StepClicked {
        channel: ChannelId,
        step: u32,
    },
    MuteToggled(ChannelId),
    ChannelSelected(ChannelId),
    KnobPressed {
        channel: ChannelId,
        knob: Knob,
    },
    TempoPressed,
    TempoScrolled(mouse::ScrollDelta),
    /// The tempo's box, opened by a click on the tempo: its text, and Enter.
    TempoEdited(String),
    TempoSubmitted,
    /// The cursor moved to this window y (or x, on a slider) while a drag is held.
    DragMoved(f32),
    DragEnded,
    ModifiersChanged(Modifiers),
    PatternSelected(PatternId),
    /// The pattern controls: the toolbar's next arrow and the name's menu, and the rack
    /// header's bar count.
    Pattern(PatternMessage),
    ModeSelected(PlayMode),
    /// L: switch between pattern and song mode.
    ToggleMode,
    /// The toolbar's Loop button: switch the loop range on or off.
    ToggleLoop,
    /// F5, F6, F7: show or hide a panel.
    TogglePanel(Panel),
    /// Something the piano roll's canvases reported.
    Roll(RollMessage),
    /// Something the playlist's canvases or header reported.
    Playlist(PlaylistMessage),
    /// The playlist header's link control, or a row of a clip's menu: a clip, or the selected
    /// clips, unlinked (given their own copies of their patterns) or linked back.
    ClipLink(ClipLink),
    /// Something the mixer's canvas or rename box reported.
    Mixer(MixerMessage),
    /// Something a divider between two stacked panels reported.
    Divider(DividerMessage),
    /// Ctrl+L: play the selected channel into the selected insert.
    RouteToSelectedInsert,
    /// Delete: remove the selected clips while the playlist has the keys, else the selected
    /// notes while the piano roll is open.
    Delete,
    /// Ctrl+B: copy the selection one extent later, in the playlist or the piano roll as for
    /// [`Message::Delete`].
    Duplicate,
    /// Ctrl+A, while the piano roll has the keys: select every note of the channel.
    SelectAllNotes,
    /// A piano roll key (the arrows, Ctrl+C/X/V): acts while the roll has the keys.
    RollKey(crate::ui::roll::NoteEdit),
    /// A row of the piano roll's notes menu.
    NoteEdit(crate::ui::roll::NoteEdit),
    /// The Quantize dialog.
    Quantize(crate::ui::roll::QuantizeMessage),
    /// The output devices a look in the background found, or why it could not look.
    OutputsListed(Result<Vec<crate::audio::ListedOutput>, String>),
    /// The input devices a look in the background found, or why it could not look.
    InputsListed(Result<Vec<crate::live_input::DeviceInfo>, String>),
    /// The toolbar's metronome: switch it on or off.
    ToggleMetronome,
    /// The toolbar's Edit button: open or close the Edit menu.
    ToggleEditMenu,
    /// A press beside the open Edit menu.
    CloseEditMenu,
    /// Esc: stop renaming a track, or else select nothing in the panel with the keys.
    ClearSelection,
    /// Time to read the playhead and the meters.
    Tick,
    /// Time to read what the output stream has reported.
    AudioCheck,
    /// Try the output device again: the timer while the player is parked without one, or the
    /// toolbar's Retry audio.
    RetryAudio,
    /// A press on the toolbar's notices: Settings opens on its General page, where the audio
    /// output and input are.
    NoticePressed,
    /// F1, or the toolbar's Help button: the Help window opened, or closed when open.
    ToggleHelp,
    /// Something the Help window reported.
    Help(crate::ui::help_window::HelpMessage),
    /// The window's device pixels per logical pixel.
    ScaleFactor(f32),
    /// The main window as the window system names it, from the toolkit once it is open: what
    /// the pointer is asked about when a file is dropped.
    MainNative(Option<dawsome_host_common::window::WindowId>),
    Headless(headless::Message),
    /// The File menu, file dialogs and file questions.
    File(FileMessage),
    /// Open the plugin picker for a new plugin channel or an empty mixer slot.
    OpenPicker(PickerTarget),
    /// Something the plugin picker reported.
    Picker(PickerMessage),
    /// Something the parameter window reported.
    Params(ParamMessage),
    /// A second press on a channel's name: open its plugin's window.
    ChannelOpened(ChannelId),
    /// A plugin channel's menu's "Parameters…": its plugin's parameter window.
    ChannelParams(ChannelId),
    /// Enter: open the selected channel's plugin.
    OpenSelectedChannel,
    /// Time for the plugins' main-thread work and the captures that wait for it.
    PluginTick,
    /// Time to read the parameter window's values again.
    PanelTick,
    /// The window has opened, so the plugin window a scripted run asked for can open.
    ScriptedPluginDue,
    /// The window was resized to this size.
    WindowResized(Size),
    /// The size of the monitor the window opened on, when the system says.
    MonitorSized(Option<Size>),
    /// Whether the window is maximized, as asked after a resize.
    WindowMaximized(bool),
    /// Something the sample browser reported.
    Browser(BrowserMessage),
    /// Background loading, imports and drops of sound files.
    Media(MediaMessage),
    /// The Record button, its menu and keys.
    Record(RecordMessage),
    /// A right press on a rack knob, at this point of the rack's canvas.
    KnobMenu {
        channel: ChannelId,
        knob: Knob,
        at: iced::Point,
    },
    /// The automation menu's row.
    AutomateChosen,
    /// A press on the rack's Recordings header: the audio takes shown under it, or folded.
    ToggleTakes,
    /// A rehearsed take's timer (`--rehearse`).
    RehearsalTick,
    /// Ctrl+T, or the toolbar's KEYS button: the typing keyboard on or off.
    ToggleTyping,
    /// The typing keyboard's octave buttons: down (−1) or up (1) an octave.
    TypingOctave(i8),
    /// A key went down or up while the typing keyboard is on, by its place on the keyboard.
    TypingKey {
        code: key::Code,
        pressed: bool,
        repeat: bool,
    },
    /// The window lost the keyboard.
    WindowUnfocused,
    /// The automation menu's link row: link the control to the next controller moved, or
    /// unlink it.
    LinkChosen,
    /// Time to take the controller moves waiting.
    ControllerTick,
    /// Something the settings panel reported.
    Settings(SettingsMessage),
    /// Time to list the MIDI input ports again.
    PortsTick,
    /// A MIDI port sent something after a quiet spell, or the lights' timer: the Library's
    /// lights against the clock (and a `--midi-monitor` run's printout).
    MidiActivity,
    /// Time to look whether the piano roll's previewed note has ended.
    NotePreviewTick,
    /// The piano roll strip's Preview switch.
    ToggleNotePreview,
    /// A view's Follow switch: the piano roll's strip's, the playlist header's.
    ToggleFollow(Panel),
    /// The rack rows' canvas was laid out this wide.
    RackResized(f32),
    /// The rack steps' scroll bar (or the wheel over the steps) asks for another view.
    RackBar(crate::ui::scroll::BarMessage),
    /// The cursor came onto a control, or left one: what the hint bar names.
    Hover(crate::ui::hint::HoverMessage),
    /// Open the instrument window on this channel.
    OpenInstrument(ChannelId),
    /// Open the sample editor on this channel's sound.
    OpenSampleEditor(ChannelId),
    /// Something the sample editor reported.
    SampleEditor(SampleEditorMessage),
    /// A channel's menu and its rename.
    Channel(ChannelCommand),
    /// Something the instrument window reported.
    Instrument(crate::ui::instrument_window::InstrumentMessage),
    /// The first-run note's frame came, or one of its buttons was pressed.
    Callout(CalloutMessage),
    /// A window reported its new scale factor, size or place: the main window's become
    /// [`Message::ScaleFactor`] and [`Message::WindowResized`]; the pop-out windows' (the
    /// instrument window's, the Settings window's) are kept for their next opening.
    Windowed(window::Id, WindowChange),
    /// A window's close button: the main window's asks about unsaved changes, then closes the
    /// app; a pop-out window's closes it.
    WindowCloseRequested(window::Id),
    /// A window has closed.
    WindowClosed(window::Id),
    /// Esc, in this window: a pop-out window's closes it; the main window's is
    /// [`Message::ClearSelection`].
    Escape(window::Id),
    /// A key pressed or let go of in either window: `App::key_message` makes it a shortcut, a
    /// note of the computer keyboard, Esc or nothing before anything else.
    Key(KeyInput),
    /// Files dropped from the desktop on a window (no sampler row having taken them): the
    /// main window's become [`MediaMessage::FileDropped`]; the pop-out windows take none.
    FileDroppedOn(window::Id, std::path::PathBuf),
}

/// Which first-run note shows (or waits): the keyboard's first, then Record's, each once per
/// install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CalloutNote {
    /// Under the keyboard switch: the computer keyboard plays notes.
    Keyboard,
    /// Under Record: arm, press Space, play.
    Record,
}

/// The first-run note `settings` have not put away yet, the keyboard's before Record's.
fn waiting_note(settings: &crate::settings::Settings) -> Option<CalloutNote> {
    if !settings.keyboard_callout_shown() {
        Some(CalloutNote::Keyboard)
    } else if !settings.record_callout_shown() {
        Some(CalloutNote::Record)
    } else {
        None
    }
}

/// Where the first-run note is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Callout {
    /// Waiting for the window's first frame.
    Due,
    Shown,
}

/// What a window reported about itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowChange {
    /// It opened at this size, and at this position where the system says.
    Opened {
        size: Size,
        position: Option<Point>,
    },
    Resized(Size),
    Moved(Point),
    Rescaled(f32),
    /// It got the keyboard (`true`) or lost it.
    Focus(bool),
}

impl From<crate::ui::hint::HoverMessage> for Message {
    fn from(message: crate::ui::hint::HoverMessage) -> Self {
        Message::Hover(message)
    }
}

/// A channel's menu rows (rename, clone, delete, one note at a time) and its rename box.
#[derive(Debug, Clone, PartialEq)]
pub enum ChannelCommand {
    Rename(ChannelId),
    RenameEdited(String),
    RenameSubmitted,
    Clone(ChannelId),
    Delete(ChannelId),
    /// The menu's "One note at a time": the channel's `cut_itself` switched.
    OneNoteAtATime(ChannelId),
    /// The menu's "Mixer insert…": an insert of the channel's own, and the mixer there.
    MixerInsert(ChannelId),
}

/// The id the channel rename box is focused by.
pub const CHANNEL_RENAME_INPUT: &str = "channel-rename";

/// What a drag changes.
#[derive(Debug, Clone, Copy, PartialEq)]
enum DragTarget {
    Knob {
        channel: ChannelId,
        knob: Knob,
    },
    Tempo,
    Mixer(MixerDrag),
    /// A plugin parameter's slider, in the parameter window or the mixer's slot panel.
    Plugin(PluginDrag),
}

impl DragTarget {
    /// Whether the drag moves left to right (a slider) rather than up and down.
    fn horizontal(self) -> bool {
        match self {
            DragTarget::Mixer(drag) => drag.horizontal(),
            DragTarget::Plugin(_) => true,
            DragTarget::Knob { .. } | DragTarget::Tempo => false,
        }
    }
}

/// A drag in progress: up (or right, on a slider) raises the value.
#[derive(Debug, Clone, PartialEq)]
struct Drag {
    target: DragTarget,
    /// The value when the drag started, as a knob value or milli-BPM.
    start: f32,
    /// The cursor coordinate along the drag of the first move. Moves are measured from it
    /// rather than from the press, so no panel's scroll offset enters the arithmetic.
    anchor_y: Option<f32>,
    /// How far the cursor has gone from the first move, at most: a press let go of with the
    /// value unchanged and less than [`CLICK_TRAVEL`] gone is a click.
    travel: f32,
    /// Undoes the whole drag: the inverse of its first change. Recorded on release, if the value
    /// then differs from `start`.
    undo: Option<Edit>,
}

/// The application: the open session and the state of the window around it.
pub struct App {
    session: Session,
    /// The live output stream; `None` when the device could not be opened.
    audio: Option<Output>,
    /// The pattern the rack shows and pattern mode plays; `None` when the project has none.
    pattern: Option<PatternId>,
    /// The pattern name's menu and rename.
    patterns: patterns::PatternUi,
    selected_channel: Option<ChannelId>,
    mode: PlayMode,
    /// Play was asked for and Stop was not; the playhead timer runs while this or `playing`.
    transport_on: bool,
    /// Whether the player was playing, as last read.
    playing: bool,
    /// The play position as last read, `None` while stopped.
    playhead: Option<Ticks>,
    drag: Option<Drag>,
    /// The divider between two stacked panels held, if one is.
    divider: Option<dividers::DividerDrag>,
    modifiers: Modifiers,
    /// What is wrong with live audio: why there is no output, or what the stream reported at
    /// the last check.
    audio_problem: Option<Cow<'static, str>>,
    /// How live audio runs, when it resamples to the device's rate.
    audio_note: Option<String>,
    /// Why the output device chosen last in Settings did not open (the stream stayed on the one
    /// before); cleared by the next choice.
    output_refused: Option<String>,
    /// Why the last action failed; cleared by the next one that succeeds.
    error: Option<String>,
    /// The rack's drawn rows; cleared whenever what they show changes.
    rack_cache: canvas::Cache,
    /// The rack's Recordings header is open: the audio takes show under it.
    takes_open: bool,
    panels: Panels,
    /// The piano roll's view, snap, selection and gesture in progress.
    roll: Roll,
    /// The notes last copied or cut in the piano roll, from any pattern and channel.
    clipboard: Option<crate::ui::note_ops::Clip>,
    /// The Quantize dialog, while it is open.
    quantize_form: Option<crate::ui::roll::QuantizeForm>,
    /// The notes played live now, as the player reported them: the piano roll shows its
    /// channel's.
    live_notes: crate::ui::live_notes::LiveNotes,
    /// The notes played lately while no take ran, for Capture.
    capture: crate::capture::Capture,
    /// A take rehearsed by a scripted run (`--rehearse`).
    rehearsal: Option<rehearse::Rehearsal>,
    /// When a key or a port last played or let go of a note: the playhead timer runs a little
    /// past it while the roll is open, so the player's report of it is read.
    live_wake: Option<Instant>,
    /// The player's count of live-note events it could not post, as last read.
    live_lost: u64,
    /// Letters were typed into the Library's search box and nothing else done in the window
    /// since (timers, hovers and key releases aside): the box may hold the keys, so the piano
    /// roll's keys edit nothing.
    search_typing: bool,
    /// The piano roll's drawn grid and lane; cleared on every message that could change them.
    roll_grid_cache: canvas::Cache,
    roll_lane_cache: canvas::Cache,
    /// The playlist's view, snap, selection, gesture in progress and rename.
    arranger: Arranger,
    /// The playlist's drawn ruler and tracks; cleared like the piano roll's.
    ruler_cache: canvas::Cache,
    tracks_cache: canvas::Cache,
    /// The mixer's selection, scroll, menus, rename and meters.
    mixer: MixerState,
    /// The mixer's drawn strips and slots; cleared like the piano roll's.
    mixer_cache: canvas::Cache,
    /// The plugin picker, while it is open.
    picker: Option<Picker>,
    /// The parameter window's list, while it is open: [`App::sync_param_window`] opens and
    /// closes its window to match.
    panel: Option<ParamPanel>,
    /// The plugin (its registry key) the list was built for, so the list is built again when
    /// another plugin takes its place; read only while the list is open.
    panel_plugin: Option<String>,
    /// A plugin scan running in the background, until its result is taken.
    scan: Option<mpsc::Receiver<Scanned>>,
    /// Plugins whose state is captured into the document once the time given comes.
    captures: BTreeMap<ProcessorOwner, Instant>,
    /// The plugin hosts of exports rendering on their workers, made and served here, each kept
    /// until its worker and its instances are done ([`crate::play_to::RenderHosts`]).
    render_hosts: Vec<crate::play_to::RenderHosts>,
    /// Plugins whose state is too big to capture with each change: captured when saving only.
    big_states: std::collections::BTreeSet<ProcessorOwner>,
    /// The window's size, as last reported.
    window_size: Size,
    /// What the cursor is over, as the hint bar names it.
    hover: Option<crate::ui::hint::Hover>,
    /// The instrument window, while it is open.
    instrument: Option<instrument::Instrument>,
    /// The main window, as main opened it; none in a test's app, which opens no window.
    main_window: Option<window::Id>,
    /// The main window as the window system names it (its X11 id, its Win32 handle), once the
    /// toolkit has said; none in a test's app and for a Wayland window.
    main_native: Option<dawsome_host_common::window::WindowId>,
    /// The instrument window's own window while it is open or opening: [`App::update`] opens
    /// and closes it to match `instrument`.
    instrument_window: Option<window::Id>,
    /// The app's window that has the keyboard, the main one or a pop-out window; none while
    /// another program has it.
    focused_window: Option<window::Id>,
    /// Whether the app's pop-out windows (the instrument window, the Settings window, our
    /// plugin windows) were last asked to stay above every window, as they are while one of the
    /// app's windows has the keyboard ([`App::sync_windows`]).
    windows_above: bool,
    /// Where the instrument window opens: its size and position as it last was.
    instrument_place: crate::settings::WindowPlace,
    /// The instrument window's width as it last reported it, which its content is laid out
    /// for; none before it reports one.
    instrument_width: Option<f32>,
    /// The instrument window's drawings, cleared as the panels' are.
    instrument_caches: crate::ui::instrument_window::Caches,
    /// The Settings window's own window while it is open or opening: [`App::update`] opens and
    /// closes it to match `panels.settings`.
    settings_window: Option<window::Id>,
    /// Where the Settings window opens: its size and position as it last was.
    settings_place: crate::settings::WindowPlace,
    /// The Help window: whether it is open, its section, its own window and its pictures.
    help: help_window::Help,
    /// The parameter window's own window, its place and size.
    params: param_window::ParamWindow,
    /// The sample editor, while it is open: [`App::sync_sample_editor_window`] opens and closes
    /// its window to match.
    sample_editor: Option<sample_editor::SampleEditor>,
    /// The sample editor's own window, its place and size, and the sounds it is made.
    editor: sample_editor::EditorWindow,
    /// The channel being renamed and the name typed so far.
    channel_rename: Option<(ChannelId, String)>,
    /// The note the piano roll previews, while it sounds.
    note_preview: Option<note_preview::Sounding>,
    /// Where a scripted run writes the "+ Instrument…" picker's rows once the sets are found.
    adder_dump: Option<std::path::PathBuf>,
    /// Whether a look on disk for the instrument sets has landed.
    sets_found: bool,
    /// The hint bar as `--hints` says: shown, hidden, or (none) as the settings say.
    hints_flag: Option<bool>,
    /// Whether the window is maximized, as last reported, and its size when it last was not:
    /// what the layout saved on closing holds.
    maximized: bool,
    restored_size: Size,
    /// The last value sent for the plugin parameter being dragged: (`owner`, `id`, `value`).
    plugin_sent: Option<(ProcessorOwner, u32, f32)>,
    /// Where song mode plays from and Stop returns to: the last ruler click, tick 0 until one.
    song_marker: Ticks,
    /// The panel the editing keys (Delete, Ctrl+B, Esc) act on: the last one opened or pressed.
    focus: Panel,
    /// Whether the Edit menu is open.
    edit_menu: bool,
    /// The tempo's box while it is open: the text typed so far.
    tempo_edit: Option<String>,
    /// Whether the metronome clicks while the transport plays (the player's own switch,
    /// sent again to each new document's player).
    metronome: bool,
    /// An input latency measurement in progress (Settings → Input latency).
    latency: Option<latency::Measuring>,
    /// The output devices Settings last listed, or why it could not list them.
    listed_outputs: Result<Vec<crate::audio::ListedOutput>, String>,
    /// The stacked panels by when they last had the focus: the least recently used close first
    /// when a panel opened leaves no room.
    panel_use: PanelUse,
    /// The panels the make-room rule closed, by the panel whose opening closed them: closing
    /// that one by hand brings them back.
    displaced: panels::Displaced,
    /// The window's device pixels per logical pixel, 1 until the window reports it.
    scale_factor: f32,
    headless: headless::Runner,
    /// The plugin whose window a scripted run opens once the app's window is open.
    scripted_plugin: Option<ProcessorOwner>,
    /// Where a scripted run writes the parameter window's list once `--open-plugin` or
    /// `--open-params` opens it (`--dump-params`).
    params_dump: Option<std::path::PathBuf>,
    /// What the app may touch outside its window.
    host: Host,
    /// The open document's name and file.
    document: Document,
    /// The File menu, the dialog on screen, what waits for a save, the autosave's state.
    files: files::Files,
    recent: Recent,
    /// What the last file or sound action has to tell, such as a sample that is missing.
    notice: Option<String>,
    /// The last notice about plugins that could not be created (what creating them said, or
    /// that one asked to open is missing): the one a plugin scan that creates them replaces.
    plugin_notice: Option<String>,
    /// The sample browser: its folders, search, preview and the file being dragged.
    browse: browse::Browse,
    /// What the playlist draws of audio and automation clips besides the document.
    looks: browse::Looks,
    /// The Record button's state and the take in progress.
    recording: recording::Recording,
    /// The automation menu a right press on a control opened.
    automate: Option<automate::AutomateMenu>,
    /// The sampler menu a right press on a sampler's name opened: its channel, and where the
    /// press was on the rack's canvas.
    sampler_menu: Option<(ChannelId, iced::Point)>,
    /// How far the rack's steps are scrolled left, in pixels (kept inside their reach once the
    /// rows' width is known, [`App::keep_rack_steps`]).
    rack_steps: f32,
    /// The rack rows' canvas width, as it last said it; 0 until it has.
    rack_width: f32,
    /// The automated controls and the values their automations set last.
    automated: automate::Automated,
    /// The typing keyboard.
    typing: typing::Typing,
    /// The shortcuts: which chord runs which action, as the settings and the Keys page say.
    keys: crate::keys::Keys,
    /// The Settings window's page.
    settings_page: crate::ui::settings_layout::SettingsPage,
    /// The action the Keys page waits for a new chord for, after a click on its row.
    rebinding: Option<crate::keys::Action>,
    /// The first-run note saying the computer keyboard plays notes, until it is put away:
    /// waiting for the window's first frame, or shown.
    callout: Option<Callout>,
    /// Which first-run note `callout` is.
    callout_note: CalloutNote,
    /// The controller moves the MIDI inputs deliver, waiting for the controller timer.
    controller_queue: ControllerQueue,
    /// The MIDI ports' clock and wake for the Library's lights, which every port's router
    /// stamps as it delivers.
    midi_activity: crate::midi::Activity,
    /// The ports whose lights are lit, as the Library last drew them.
    lit_ports: std::collections::BTreeSet<String>,
    /// A `--midi-monitor` run: what the ports deliver, and what the player plays of it, is
    /// printed.
    midi_monitor: bool,
    /// Bursts of controller moves under way.
    links: LinkDriver,
    /// The control waiting to be linked, and the clock bursts are timed by.
    controllers: controllers::Controllers,
    /// The MIDI input ports and the connections open to them.
    ports: MidiPorts,
    /// The thread that sends what the input channels' MIDI outs send; `None` when it could not
    /// start (the toolbar said why).
    midi_out: Option<crate::midi_out::MidiOutThread>,
    /// The channels' routes as the MIDI-out thread was last told them.
    midi_routes: crate::midi_out::Routes,
    /// The MIDI output ports as last listed, for the MIDI OUT group's Port picker, or why they
    /// could not be.
    midi_output_names: Result<Vec<String>, String>,
    /// The app's own settings: which MIDI inputs play.
    settings: Settings,
    /// Audio clip blocks that played silence, as last read.
    underruns: u64,
    /// Notes plugin channels had no room for, as the toolbar last said.
    dropped_notes: u64,
    /// The input channels' devices, open while the document names them.
    live_inputs: crate::live_input::LiveInputs,
    /// When the input channels' meters were last read, for their fall.
    live_meters_at: Instant,
    /// The input devices as last listed, for the input channel's window and Settings.
    input_listing: input_channels::InputListing,
    /// The player of a document opened while no device is in use; a test plays it by hand.
    #[cfg(test)]
    parked_player: Option<Player>,
}

impl App {
    /// Opens the window on `session`, the first pattern and channel selected and `panels`
    /// open, in a window of `window_size`, in song mode when `song` (the session must play the
    /// same mode). `audio` is the live output, or why there is none; the toolbar then says so.
    /// `start` names the document (and holds its import report); `host` says what the app may
    /// touch outside the window.
    #[expect(
        clippy::too_many_arguments,
        reason = "the app's start-up inputs, each made on its own by main and by the tests that \
                  boot the app; a struct for them would be built only to be taken apart here"
    )]
    pub fn boot(
        mut session: Session,
        audio: Result<Output, audio::StartError>,
        panels: Panels,
        window_size: Size,
        song: bool,
        script: headless::Script,
        script_failed: Arc<AtomicBool>,
        host: Host,
        start: Start,
    ) -> (Self, Task<Message>) {
        let (audio, audio_problem) = match audio {
            Ok(output) => (Some(output), None),
            Err(error) => (None, Some(Cow::Owned(format!("Audio off: {error}")))),
        };
        let audio_note = audio.as_ref().and_then(Output::notice);
        let notices = session.take_notices();
        let recent = Recent::load(host.recent_file.clone());
        let project = session.project();
        let step = dawsome_engine::time::step_to_ticks(1, project.ppq);
        let tracks_size = playlist_layout::tracks_size_alone(window_size, layout::TOOLBAR_HEIGHT);
        let mut browse = browse::Browse::new(&host, None);
        let mut ports = MidiPorts::new(host.midi_ports.clone());
        let mut settings = Settings::load(host.settings_file.as_deref());
        let midi_monitor = host.midi_monitor;
        if midi_monitor {
            // Every port the monitor's source lists (its one) plays; a scripted run saves none
            // of this.
            ports.monitor();
            settings.midi_inputs = crate::settings::MidiInputs::All;
        }
        browse.library.folded = settings.library_folded();
        let first = crate::settings::WindowPlace::first;
        let instrument_place = (settings.instrument_window)
            .unwrap_or_else(|| first(crate::desktop::PopOut::Instrument));
        let settings_place =
            (settings.settings_window).unwrap_or_else(|| first(crate::desktop::PopOut::Settings));
        let typing = typing::Typing::new(settings.keyboard_plays_notes(), settings.note_layout());
        let keys = settings.keys();
        let waiting = waiting_note(&settings);
        let callout = (host.callouts && waiting.is_some()).then_some(Callout::Due);
        let callout_note = waiting.unwrap_or(CalloutNote::Keyboard);
        let midi_out = midi_outs::start(&host);
        let midi_output_names = host.midi_outputs.names();
        let mut app = Self {
            pattern: project.patterns.keys().next().copied(),
            patterns: patterns::PatternUi::default(),
            selected_channel: project.channels.keys().next().copied(),
            roll: Roll::new(step, piano_layout::grid_size_alone(window_size))
                .with_lane_height(settings.lane_height()),
            clipboard: None,
            quantize_form: None,
            live_notes: crate::ui::live_notes::LiveNotes::default(),
            capture: crate::capture::Capture::default(),
            rehearsal: None,
            live_wake: None,
            live_lost: 0,
            search_typing: false,
            arranger: Arranger::new(project.ppq, tracks_size, settings.playlist_lanes()),
            session,
            audio,
            mode: if song {
                PlayMode::Song
            } else {
                PlayMode::Pattern
            },
            transport_on: false,
            playing: false,
            playhead: None,
            drag: None,
            divider: None,
            modifiers: Modifiers::default(),
            audio_problem,
            audio_note,
            output_refused: None,
            error: None,
            rack_cache: canvas::Cache::new(),
            takes_open: false,
            panels,
            roll_grid_cache: canvas::Cache::new(),
            roll_lane_cache: canvas::Cache::new(),
            ruler_cache: canvas::Cache::new(),
            tracks_cache: canvas::Cache::new(),
            mixer: MixerState::new(window_size.width, settings.kept_heights().mixer_height()),
            mixer_cache: canvas::Cache::new(),
            picker: None,
            panel: None,
            panel_plugin: None,
            scan: None,
            captures: BTreeMap::new(),
            render_hosts: Vec::new(),
            big_states: std::collections::BTreeSet::new(),
            window_size,
            hover: None,
            instrument: None,
            main_window: None,
            main_native: None,
            instrument_window: None,
            focused_window: None,
            windows_above: false,
            instrument_place,
            instrument_width: None,
            instrument_caches: crate::ui::instrument_window::Caches::new(),
            settings_window: None,
            settings_place,
            help: help_window::Help::new(settings.help_window, crate::help::folder()),
            params: param_window::ParamWindow::new(settings.params_window),
            sample_editor: None,
            editor: sample_editor::EditorWindow::new(settings.sample_editor_window),
            channel_rename: None,
            note_preview: None,
            adder_dump: None,
            sets_found: false,
            hints_flag: None,
            maximized: false,
            restored_size: window_size,
            plugin_sent: None,
            song_marker: Ticks(0),
            focus: first_focus(panels),
            edit_menu: false,
            tempo_edit: None,
            metronome: false,
            latency: None,
            listed_outputs: Ok(Vec::new()),
            panel_use: PanelUse::default(),
            displaced: panels::Displaced::default(),
            scale_factor: 1.0,
            headless: headless::Runner::new(script.clone(), Arc::clone(&script_failed)),
            scripted_plugin: script.open_plugin,
            params_dump: script.dump_params.clone(),
            host,
            document: start.document.clone(),
            files: files::Files::default(),
            recent,
            notice: None,
            plugin_notice: None,
            browse,
            looks: browse::Looks::default(),
            recording: recording::Recording::default(),
            automate: None,
            sampler_menu: None,
            rack_steps: 0.0,
            rack_width: 0.0,
            automated: automate::Automated::new(),
            typing,
            keys,
            settings_page: crate::ui::settings_layout::SettingsPage::General,
            rebinding: None,
            callout,
            callout_note,
            controller_queue: ControllerQueue::default(),
            midi_activity: crate::midi::Activity::default(),
            lit_ports: std::collections::BTreeSet::new(),
            midi_monitor,
            links: LinkDriver::default(),
            controllers: controllers::Controllers::default(),
            ports,
            midi_out: None,
            midi_routes: crate::midi_out::Routes::new(),
            midi_output_names,
            settings,
            underruns: 0,
            dropped_notes: 0,
            live_inputs: crate::live_input::LiveInputs::default(),
            live_meters_at: Instant::now(),
            input_listing: input_channels::InputListing::default(),
            #[cfg(test)]
            parked_player: None,
        };
        app.arranger.refresh(app.session.project());
        app.show_project_folder();
        app.share_library();
        app.refresh_looks();
        app.read_automation();
        app.connect_ports();
        if app.midi_monitor {
            println!(
                "midi-monitor: client {:?}, connected to {:?}{}",
                crate::midi::this_client(),
                app.ports.connected_names(),
                app.ports
                    .problem()
                    .map_or_else(String::new, |problem| format!(" ({problem})"))
            );
        }
        if panels.piano_roll {
            app.show_roll();
        }
        app.select_channel_insert();
        app.notice = files::summarize_notices(&notices);
        match midi_out {
            Ok(thread) => {
                app.midi_out = Some(thread);
                app.open_midi_out();
            }
            Err(problem) => app.notice = Some(problem),
        }
        app.show_plugin_notices();
        app.sync_live_inputs(true);
        let focus = app.start_plugins(&script, &script_failed);
        app.list_demo_on_first_launch();
        app.sweep_partial_saves();
        let sets = app.scan_sets();
        let outputs = if panels.settings {
            app.list_outputs()
        } else {
            Task::none()
        };
        if let Some(opened) = app.document.path.clone() {
            app.remember_opened(&opened);
        }
        app.startup_recovery(&start);
        if script.play {
            app.play();
        }
        if let Some(path) = &script.rehearse {
            app.start_rehearsal(path);
        }
        // The main window is the only one yet: the app starts once it is open.
        let scale_factor = window::oldest()
            .and_then(window::scale_factor)
            .map(Message::ScaleFactor);
        let monitor = window::oldest()
            .and_then(window::monitor_size)
            .map(Message::MonitorSized);
        let native = window::oldest()
            .and_then(|id| window::run(id, desktop_drop::native_window))
            .map(Message::MainNative);
        let scale_factor = Task::batch([scale_factor, monitor, native]);
        (app, Task::batch([scale_factor, focus, sets, outputs]))
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
        // Why after every message: the focus moves wherever a panel is opened or pressed, and
        // the panel that has it is the one used last.
        self.panel_use.touch(self.focus);
        // Why after every message too: Settings closes or leaves its Keys page on several paths
        // (its keys and button, its tabs, the make-room rule), and a row left waiting where no
        // page shows it would take the next key pressed anywhere as its chord.
        let keys_page = crate::ui::settings_layout::SettingsPage::Keys;
        if !(self.panels.settings && self.settings_page == keys_page) {
            self.rebinding = None;
        }
        // Why after every message too: a panel closes on several paths (its key, its button,
        // the make-room rule), and a scroll bar's thumb held as it went gets no release, which
        // would keep its view from following the playhead.
        if !self.panels.piano_roll {
            self.roll.let_go_of_bar();
        }
        if !self.panels.playlist {
            self.arranger.let_go_of_bar();
        }
        // Why after every message too: the instrument window opens and closes on many paths
        // (a channel's menu, the Library, Esc, an undo taking its channel, a new document), and
        // so does the Settings window (its keys and button, the notices, Esc, its X); their own
        // windows follow that state here, in one place; on an input channel the instrument
        // window lists the input devices for its picker.
        let listing = self.list_inputs_for_window();
        Task::batch([task, self.sync_pop_outs(), listing])
    }

    /// The main window is the one with `id`: the app's scripted runs and window rules are
    /// told it by main as it opens it.
    pub fn set_main_window(&mut self, id: window::Id) {
        self.main_window = Some(id);
    }

    /// A scripted run's screenshot is of `window`.
    pub fn shoot(&mut self, window: headless::ShotWindow) {
        self.headless.shoot(window);
    }

    /// The window `id` shows: the instrument window's, the Settings window's, the Help
    /// window's, the parameter window's, or the main window's.
    pub fn view_window(&self, id: window::Id) -> Element<'_, Message> {
        if self.instrument_window == Some(id) {
            self.instrument_window_view()
        } else if self.settings_window == Some(id) {
            self.settings_window_view()
        } else if self.help.window == Some(id) {
            self.help_window_view()
        } else if self.params.window == Some(id) {
            self.param_window_view()
        } else if self.editor.window == Some(id) {
            self.sample_editor_view()
        } else {
            self.view()
        }
    }

    /// The title of window `id`: the instrument window's names its channel and sound, the
    /// parameter window's its place and plugin.
    pub fn window_title(&self, id: window::Id) -> String {
        if self.instrument_window == Some(id) {
            self.instrument_title()
        } else if self.settings_window == Some(id) {
            self.settings_title()
        } else if self.help.window == Some(id) {
            self.help_title()
        } else if self.params.window == Some(id) {
            self.param_title()
        } else if self.editor.window == Some(id) {
            self.sample_editor_title()
        } else {
            self.title()
        }
    }

    /// Whether `id` is the main window: the one main opened (any window but the pop-out
    /// windows in a test's app, which opens none).
    fn is_main_window(&self, id: window::Id) -> bool {
        self.instrument_window != Some(id)
            && self.settings_window != Some(id)
            && self.help.window != Some(id)
            && self.params.window != Some(id)
            && self.editor.window != Some(id)
            && self.main_window.is_none_or(|main| main == id)
    }

    /// A task on the main window: by the id main opened it with, else (a test's app) the
    /// oldest window, which is the main one.
    fn on_main_window(&self) -> Task<Option<window::Id>> {
        match self.main_window {
            Some(id) => Task::done(Some(id)),
            None => window::oldest(),
        }
    }

    /// Sorts out a message naming the window it came from: the pop-out windows' own events are
    /// taken here (their size and place kept, their close buttons and their closing end them,
    /// their Esc closes them); the main window's become the messages they always were. `None`
    /// when taken. A window that is none of these (a pop-out window closing) is passed over.
    fn window_message(&mut self, message: Message) -> Option<Message> {
        let instrument = self.instrument_window;
        let settings = self.settings_window;
        let help = self.help.window;
        let params = self.params.window;
        let editor = self.editor.window;
        match message {
            // Either window's keyboard focus: the instrument window floats above the main one
            // while the app has it. A window losing it ends the typed notes, as ever.
            Message::Windowed(id, WindowChange::Focus(focused)) => {
                if focused {
                    self.focused_window = Some(id);
                } else if self.focused_window == Some(id) {
                    self.focused_window = None;
                }
                (!focused).then_some(Message::WindowUnfocused)
            }
            Message::Windowed(id, change) if instrument == Some(id) => {
                self.instrument_window_changed(change);
                None
            }
            Message::Windowed(id, change) if settings == Some(id) => {
                self.settings_window_changed(change);
                None
            }
            Message::Windowed(id, change) if help == Some(id) => {
                self.help_window_changed(change);
                None
            }
            Message::Windowed(id, change) if params == Some(id) => {
                self.param_window_changed(change);
                None
            }
            Message::Windowed(id, _)
            | Message::WindowCloseRequested(id)
            | Message::WindowClosed(id)
            | Message::Escape(id)
                if editor == Some(id) =>
            {
                self.sample_editor_window_message(&message);
                None
            }
            Message::Windowed(id, change) if self.is_main_window(id) => match change {
                WindowChange::Rescaled(factor) => Some(Message::ScaleFactor(factor)),
                WindowChange::Resized(size) => Some(Message::WindowResized(size)),
                WindowChange::Opened { .. } | WindowChange::Moved(_) | WindowChange::Focus(_) => {
                    None
                }
            },
            Message::WindowCloseRequested(id) if instrument == Some(id) => {
                self.close_instrument();
                None
            }
            Message::WindowCloseRequested(id) if settings == Some(id) => {
                self.close_settings();
                None
            }
            Message::WindowCloseRequested(id) if help == Some(id) => {
                self.close_help();
                None
            }
            Message::WindowCloseRequested(id) if params == Some(id) => {
                self.close_params();
                None
            }
            Message::WindowCloseRequested(id) if self.is_main_window(id) => {
                Some(Message::File(FileMessage::CloseRequested))
            }
            Message::WindowClosed(id) if instrument == Some(id) => {
                // Gone already: only the state is left to end.
                self.instrument_window = None;
                self.instrument_width = None;
                self.close_instrument();
                None
            }
            Message::WindowClosed(id) if settings == Some(id) => {
                // Gone already: Settings is closed and its place kept.
                self.settings_window = None;
                self.close_settings();
                self.remember_settings_place();
                None
            }
            Message::WindowClosed(id) if help == Some(id) => {
                // Gone already: Help is closed and its place kept.
                self.help.window = None;
                self.close_help();
                self.remember_help_place();
                None
            }
            Message::WindowClosed(id) if params == Some(id) => {
                // Gone already: the list is closed and its place kept.
                self.params.window = None;
                self.params.size = None;
                self.close_params();
                self.remember_param_place();
                None
            }
            // The main window gone without its close asking (the system closed it): the app
            // closes with it, the instrument window too, rather than run on without it.
            Message::WindowClosed(id) if self.main_window == Some(id) => {
                Some(Message::File(FileMessage::MainWindowClosed))
            }
            // As its X: whatever the main window has up, a dialog too.
            Message::Escape(id) if instrument == Some(id) => {
                self.close_instrument();
                None
            }
            Message::Escape(id) if settings == Some(id) => {
                self.close_settings();
                None
            }
            Message::Escape(id) if help == Some(id) => {
                self.close_help();
                None
            }
            Message::Escape(id) if params == Some(id) => {
                self.escape_params();
                None
            }
            Message::Escape(_) => Some(Message::ClearSelection),
            // A key's message is sorted out as if it came on its own: a typed Esc is its
            // window's Esc (found in use: it reached no window's Esc, so the
            // instrument window never closed on it, nor did the main window's Esc chain run).
            Message::Key(input) => {
                let message = self.key_message(input)?;
                self.window_message(message)
            }
            Message::FileDroppedOn(id, _)
                if [instrument, settings, help, params, editor].contains(&Some(id)) =>
            {
                None
            }
            Message::FileDroppedOn(_, path) => {
                Some(Message::Media(MediaMessage::FileDropped(path)))
            }
            Message::Windowed(..) | Message::WindowCloseRequested(_) | Message::WindowClosed(_) => {
                None
            }
            other => Some(other),
        }
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        crate::window_stats::updated();
        let Some(message) = self.window_message(message) else {
            return Task::none();
        };
        // While a dialog is up, only its answers and the timers get through.
        if !self.files.lets_through(&message) {
            return Task::none();
        }
        // The Library's search box holds the keys from its typing until anything else is done
        // in the window (a press outside it, which iced takes its focus away on, or Esc, which
        // does too): the same messages that close the menus, timers, hovers and key releases
        // passing it by. Here, ahead of Esc's early returns.
        if closes_menus(&message) && !matches!(message, Message::Browser(BrowserMessage::Search(_)))
        {
            self.search_typing = false;
        }
        if matches!(message, Message::ClearSelection) {
            // A menu over a panel first: drawn on the window's top layer, it is above
            // everything the rest of this chain closes, the instrument window's work included.
            if self.close_panel_menu() {
                return Task::none();
            }
            if self.callout == Some(Callout::Shown) {
                self.put_away_callout();
                return Task::none();
            }
            if self.files.modal.is_some() {
                return self.escape_dialog();
            }
            if self.files.menu {
                self.files.menu = false;
                return Task::none();
            }
            if self.edit_menu {
                self.edit_menu = false;
                return Task::none();
            }
            if self.tempo_edit.take().is_some() {
                return Task::none();
            }
            // The picker over everything, then the parameter window, then the instrument
            // window.
            if self.picker.take().is_some() || self.panel.take().is_some() {
                return Task::none();
            }
            if self.close_sample_editor() {
                return Task::none();
            }
            if self.close_instrument() {
                return Task::none();
            }
        }
        if closes_menus(&message) {
            self.automate = None;
            self.sampler_menu = None;
            // Its own button toggles it below.
            if !matches!(message, Message::Roll(RollMessage::MenuButton)) {
                self.roll.menu = None;
            }
            // Its own button toggles it below.
            if !matches!(message, Message::ToggleEditMenu) {
                self.edit_menu = false;
            }
        }
        // Why end a rename on every message that closes the menus (all but those
        // `closes_menus` lists, such as the timers, resizes, cursor moves, the knob's menu and
        // every file and sound message): the text box would otherwise stay open, renaming
        // whichever pattern it was opened on, behind whatever came next. Esc ends it below,
        // and takes the key.
        let renaming = matches!(
            message,
            Message::Pattern(PatternMessage::RenameEdited(_) | PatternMessage::RenameSubmitted)
                | Message::ClearSelection
        );
        if !renaming && closes_menus(&message) {
            self.patterns.rename = None;
        }
        // Anything else done in the window closes the tempo's box unapplied, as a rename's.
        let typing_tempo = matches!(
            message,
            Message::TempoEdited(_) | Message::TempoSubmitted | Message::DragEnded
        );
        if !typing_tempo && closes_menus(&message) {
            self.tempo_edit = None;
        }
        let renaming_channel = matches!(
            message,
            Message::Channel(
                ChannelCommand::RenameEdited(_)
                    | ChannelCommand::RenameSubmitted
                    | ChannelCommand::Rename(_)
            )
        );
        if !renaming_channel && closes_menus(&message) && self.channel_rename.take().is_some() {
            // Anything else done in the window ends the rename unsaved; Esc ends it below.
            if matches!(message, Message::ClearSelection) {
                return Task::none();
            }
        }
        // Why clear on every message but the timers': the piano roll shows the document, its
        // own state and the held keys (Alt changes a drag's snap), and nearly every message
        // can change one of them. The playhead is drawn outside the cache. A timer that
        // changes the document (a recorded note, a controller move) clears them itself.
        if !matches!(
            message,
            Message::Tick
                | Message::AudioCheck
                | Message::RetryAudio
                | Message::Headless(_)
                | Message::PluginTick
                | Message::PanelTick
                | Message::Params(ParamMessage::Scrolled(_))
                | Message::SampleEditor(SampleEditorMessage::Tick)
                | Message::ControllerTick
                | Message::PortsTick
                | Message::NotePreviewTick
                | Message::Hover(_)
                | Message::Callout(CalloutMessage::Due)
        ) {
            self.clear_panel_caches();
        }
        match message {
            Message::Play => self.play(),
            Message::Stop => self.stop(),
            Message::TogglePlay if self.transport_on => self.stop(),
            Message::TogglePlay => self.play(),
            // A take of notes is one undo step once it stops; stepping through the history
            // under it would pull edits out from under the notes it is writing.
            Message::Undo | Message::Redo if self.recording.taking_notes() => {
                self.notice = Some(recording::STOP_RECORDING_FIRST.to_owned());
            }
            Message::Undo => {
                self.finish_drag();
                self.finish_instrument_drag();
                self.settle_controllers();
                self.roll.cancel_gesture();
                self.arranger.cancel_gesture();
                let result = self.session.undo().map(drop);
                self.after_edit(result);
            }
            Message::Redo => {
                self.finish_drag();
                self.finish_instrument_drag();
                self.settle_controllers();
                self.roll.cancel_gesture();
                self.arranger.cancel_gesture();
                let result = self.session.redo().map(drop);
                self.after_edit(result);
            }
            Message::StepClicked { channel, step } => {
                self.focus = Panel::Rack;
                if let Some(pattern) = self.pattern {
                    // One note goes whatever its key; a chord loses only its note at the
                    // default key, never the rest.
                    match step_grid::step_press(self.session.project(), pattern, channel, step) {
                        Some(edit) => self.commit(edit),
                        None => self.notice = Some(MELODY_STEP.to_owned()),
                    }
                }
            }
            Message::MuteToggled(channel) => {
                self.focus = Panel::Rack;
                if let Some(shown) = self.session.project().channels.get(&channel) {
                    let value = !shown.mute;
                    self.commit(Edit::SetChannelMute { channel, value });
                }
            }
            Message::ChannelSelected(channel) => {
                self.focus = Panel::Rack;
                self.press_channel(channel);
            }
            Message::KnobPressed { channel, knob } => {
                self.focus = Panel::Rack;
                self.start_drag(DragTarget::Knob { channel, knob });
            }
            Message::TempoPressed => self.start_drag(DragTarget::Tempo),
            Message::TempoScrolled(delta) => {
                // Up raises the tempo one step per scroll event, whatever its size.
                let rise = match delta {
                    mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => y,
                };
                if rise != 0.0 {
                    let steps = if rise > 0.0 { 1 } else { -1 };
                    let mbpm = self.step_tempo(self.session.project().tempo, steps);
                    self.commit(Edit::SetTempo { mbpm });
                }
            }
            Message::DragMoved(along) => self.drag_to(along),
            Message::DragEnded => {
                // A press on the tempo let go of where it was types it instead.
                let click = self.drag.as_ref().is_some_and(|drag| {
                    drag.target == DragTarget::Tempo
                        && drag.travel < CLICK_TRAVEL
                        && self.drag_value(drag.target) == Some(drag.start)
                });
                self.finish_drag();
                if click {
                    self.tempo_edit = Some(toolbar::format_tempo(self.session.project().tempo));
                    let input = Id::new(TEMPO_INPUT);
                    return operation::focus(input.clone()).chain(operation::select_all(input));
                }
            }
            Message::TempoEdited(typed) => {
                if let Some(edit) = &mut self.tempo_edit {
                    *edit = typed;
                }
            }
            Message::TempoSubmitted => {
                let typed = self.tempo_edit.clone().unwrap_or_default();
                match toolbar::parse_tempo(&typed) {
                    Ok(mbpm) => {
                        self.tempo_edit = None;
                        self.commit(Edit::SetTempo { mbpm });
                    }
                    Err(reason) => self.notice = Some(reason),
                }
            }
            Message::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers;
                self.restart_mixer_drag();
            }
            Message::PatternSelected(pattern) => {
                if Some(pattern) != self.pattern {
                    self.keep_take_for_another_pattern();
                }
                self.select_pattern(pattern);
            }
            Message::Pattern(message) => return self.pattern_message(message),
            Message::ModeSelected(mode) => self.select_mode(mode),
            Message::ToggleMode => self.select_mode(match self.mode {
                PlayMode::Pattern => PlayMode::Song,
                PlayMode::Song => PlayMode::Pattern,
            }),
            Message::ToggleLoop => {
                if let Some(range) = self.session.project().playlist.loop_range {
                    let value = Some(LoopRange {
                        on: !range.on,
                        ..range
                    });
                    self.commit(Edit::SetLoop { value });
                }
            }
            Message::TogglePanel(panel) => {
                self.panels.toggle(panel);
                if !self.panels.is_open(panel) {
                    return self.bring_back(panel);
                }
                // Opened by its key or button, the roll edits the pattern, not a clip of it.
                if panel == Panel::PianoRoll {
                    self.roll.forget_clip();
                }
                self.focus = panel;
                self.make_room_for(panel);
                return self.panel_opened(panel);
            }
            Message::Playlist(message) => return self.playlist_message(message),
            Message::ClipLink(link) => self.clip_link(link),
            Message::Mixer(message) => return self.mixer_message(message),
            Message::Divider(message) => self.divider_message(message),
            Message::RouteToSelectedInsert => {
                if let Some(channel) = self.selected_channel {
                    let insert = self.mixer.selected;
                    self.commit(Edit::RouteChannel { channel, insert });
                }
            }
            Message::Roll(message) => {
                // A new layout size is not a press: it must not take the keys.
                if !matches!(message, RollMessage::GridResized(_)) {
                    self.focus = Panel::PianoRoll;
                }
                let target =
                    Target::of(self.session.project(), self.pattern, self.selected_channel);
                let preview = self.note_to_preview(&message, target.as_ref());
                let lane_let_go = message == RollMessage::HandleReleased;
                let commit = self.roll.update(message, target.as_ref(), self.modifiers);
                if let Some(commit) = commit {
                    self.commit_notes(commit);
                }
                if lane_let_go {
                    self.settings.set_lane_height(self.roll.lane_height());
                    self.save_settings();
                }
                if let Some((key, velocity)) = preview {
                    self.preview_note(key, velocity);
                }
            }
            // The piano roll first while it has the keys (the cursor over it with notes selected
            // beats the playlist's being the last pressed), then the playlist.
            Message::Delete if self.roll_has_keys() => self.note_command(Roll::delete_selected),
            Message::Duplicate if self.roll_has_keys() => {
                self.note_command(Roll::duplicate_selected);
            }
            Message::Delete if self.playlist_has_keys() => {
                if let Some(edit) = self.arranger.delete_selected() {
                    self.commit(edit);
                }
            }
            Message::Duplicate if self.playlist_has_keys() => {
                if let Some(edit) = self.arranger.duplicate_selected(self.session.project()) {
                    self.commit(edit);
                }
            }
            Message::Delete | Message::Duplicate => {}
            Message::RollKey(edit) if self.roll_has_keys() => self.note_edit(edit),
            Message::RollKey(_) => {}
            Message::NoteEdit(edit) => {
                self.roll.menu = None;
                self.note_edit(edit);
            }
            Message::Quantize(message) => self.quantize_message(message),
            Message::OutputsListed(listed) => self.listed_outputs = listed,
            Message::InputsListed(listed) => self.input_listing.devices = Some(listed),
            Message::ToggleMetronome => {
                self.metronome = !self.metronome;
                let result = self.session.send(Control::Metronome(self.metronome));
                self.report(result);
            }
            Message::ToggleEditMenu => {
                self.edit_menu = !self.edit_menu;
                self.files.menu = false;
            }
            Message::CloseEditMenu => {}
            Message::SelectAllNotes => {
                let target =
                    Target::of(self.session.project(), self.pattern, self.selected_channel);
                if let (true, Some(target)) = (self.roll_has_keys(), target) {
                    self.roll.select_all(&target);
                }
            }
            Message::ClearSelection => {
                if self.patterns.escape() {
                    // The pattern name's menu or rename took the key.
                } else if self.cancel_learning() {
                    // A control waiting for its controller took the key.
                } else if self.cancel_browser_drag() || self.stop_preview() {
                    // A file held from the browser, or the preview playing, took the key.
                } else if self.panels.mixer && self.mixer.escape() {
                    // A mixer rename or menu took the key.
                } else if self.arranger.rename.is_some() || self.playlist_has_keys() {
                    self.arranger.escape();
                } else if self.panels.piano_roll {
                    self.roll.selection.clear();
                }
            }
            Message::Tick => {
                // First, so Follow already knows the view was moved by the drag.
                self.scroll_playlist_at_edge();
                self.read_playhead();
            }
            Message::AudioCheck => self.check_audio(),
            Message::RetryAudio => self.retry_audio(),
            Message::NoticePressed => {
                return self.show_settings(crate::ui::settings_layout::SettingsPage::General);
            }
            Message::ToggleHelp => self.toggle_help(),
            Message::Help(message) => return self.help_message(message),
            Message::ScaleFactor(factor) => self.scale_factor = factor,
            Message::MainNative(native) => self.main_native = native,
            Message::Headless(message) => {
                let windows = headless::Windows {
                    main: self.main_window,
                    instrument: self.instrument_window,
                    settings: self.settings_window,
                    help: self.help.window,
                    params: self.params.window,
                    sample_editor: self.editor.window,
                };
                return self
                    .headless
                    .update(message, windows)
                    .map(Message::Headless);
            }
            Message::File(message) => return self.file_message(message),
            Message::OpenPicker(target) => return self.open_picker(target),
            Message::Picker(message) => return self.picker_message(message),
            Message::Params(message) => self.param_message(message),
            Message::OpenSampleEditor(channel) => self.open_sample_editor(channel),
            Message::SampleEditor(message) => return self.sample_editor_message(message),
            Message::ChannelOpened(channel) => {
                // A double press on a name: a plugin channel's own window (its instrument
                // window is in its menu); any other channel's instrument window, or an
                // instrument set's load report when the set left something out.
                self.focus = Panel::Rack;
                let plugin = self
                    .session
                    .project()
                    .channels
                    .get(&channel)
                    .is_some_and(|channel| channel.kind == ChannelKind::Plugin);
                if plugin {
                    self.open_plugin(ProcessorOwner::Channel(channel));
                } else if !self.show_instrument_report(channel) {
                    self.open_instrument_chosen(channel);
                }
            }
            Message::ChannelParams(channel) => {
                self.focus = Panel::Rack;
                self.open_panel(ProcessorOwner::Channel(channel));
            }
            Message::OpenSelectedChannel if self.browser_has_keys() => {
                return self.browser_message(BrowserMessage::Key(Nav::Enter));
            }
            Message::OpenSelectedChannel => {
                if let (Panel::Rack, Some(channel)) = (self.focus, self.selected_channel) {
                    self.open_plugin(ProcessorOwner::Channel(channel));
                }
            }
            Message::PluginTick => self.plugin_tick(),
            Message::PanelTick => self.refresh_panel(),
            Message::ScriptedPluginDue => return self.open_scripted_plugin(),
            Message::WindowResized(size) => {
                self.window_size = size;
                return self
                    .on_main_window()
                    .and_then(window::is_maximized)
                    .map(Message::WindowMaximized);
            }
            Message::MonitorSized(Some(monitor)) if !self.maximized => {
                // A window larger than its monitor (a layout saved on a larger one) fits it.
                if let Some(fitted) = crate::settings::fit_to_monitor(self.window_size, monitor) {
                    return self
                        .on_main_window()
                        .and_then(move |id| window::resize(id, fitted));
                }
            }
            Message::MonitorSized(_) => {}
            Message::WindowMaximized(maximized) => {
                self.maximized = maximized;
                if !maximized {
                    self.restored_size = self.window_size;
                }
            }
            Message::Browser(BrowserMessage::Key(_)) if !self.browser_has_keys() => {}
            Message::Browser(message) => return self.browser_message(message),
            Message::Media(message) => return self.media_message(message),
            Message::Record(message) => self.record_message(message),
            Message::KnobMenu { channel, knob, at } => {
                self.focus = Panel::Rack;
                self.sampler_menu = None;
                if let Some(shown) = self.session.project().channels.get(&channel) {
                    let value = knob.fraction(knob.value(shown));
                    self.open_automate_menu(knob.automation(channel), value, Panel::Rack, at);
                }
            }
            Message::AutomateChosen => self.automate_chosen(),
            Message::RehearsalTick => self.rehearse(),
            Message::ToggleTakes => {
                self.takes_open = !self.takes_open;
                self.rack_cache.clear();
            }
            Message::ToggleTyping => {
                // The switch the first-run note points at: using it puts the note away.
                if self.callout_shows(CalloutNote::Keyboard) {
                    self.put_away_callout();
                }
                self.toggle_typing();
            }
            Message::Callout(CalloutMessage::Due) => {
                if self.callout == Some(Callout::Due) {
                    self.callout = Some(Callout::Shown);
                }
            }
            Message::Callout(CalloutMessage::GotIt) => self.put_away_callout(),
            Message::Callout(CalloutMessage::OpenKeys) => {
                self.put_away_callout();
                return self.show_settings(crate::ui::settings_layout::SettingsPage::Keys);
            }
            Message::TypingOctave(octaves) => self.move_octave(octaves),
            Message::TypingKey {
                code,
                pressed,
                repeat,
            } => {
                self.live_wake = Some(Instant::now());
                self.typed(code, pressed, repeat);
                // What the player posted so far is read now, not only on the timer, so its
                // ring never fills while the roll is closed.
                self.read_player_events();
            }
            // Only the typing keyboard holds notes the window must end: a MIDI device's notes
            // end with their own messages wherever the keyboard is.
            Message::MidiActivity => {
                self.live_wake = Some(Instant::now());
                self.midi_activity_tick();
                self.read_player_events();
            }
            Message::WindowUnfocused if self.typing.on => self.end_typed_notes(),
            Message::WindowUnfocused => {}
            Message::LinkChosen => self.link_chosen(),
            Message::ControllerTick => self.poll_controllers(),
            Message::Settings(message) => self.settings_message(message),
            Message::PortsTick => self.list_ports(),
            Message::NotePreviewTick => self.note_preview_tick(),
            Message::ToggleNotePreview => self.toggle_note_preview(),
            Message::RackResized(width) => {
                self.rack_width = width;
                self.keep_rack_steps();
            }
            Message::RackBar(crate::ui::scroll::BarMessage::To { offset, .. }) => {
                self.rack_steps = offset as f32;
                self.keep_rack_steps();
                self.rack_cache.clear();
            }
            Message::RackBar(crate::ui::scroll::BarMessage::LetGo) => {}
            Message::ToggleFollow(panel) => {
                let on = !self.settings.follows(panel);
                self.settings.set_follows(panel, on);
                self.save_settings();
            }
            Message::Hover(crate::ui::hint::HoverMessage::Over(hover)) => {
                self.hover = Some(hover);
            }
            Message::Hover(crate::ui::hint::HoverMessage::Left(left)) => {
                if self.hover.as_ref() == Some(&left) {
                    self.hover = None;
                }
            }
            Message::OpenInstrument(channel) => {
                self.focus = Panel::Rack;
                self.open_instrument(channel);
            }
            Message::Instrument(message) => {
                self.live_wake = Some(Instant::now());
                self.instrument_message(message);
                self.read_player_events();
            }
            Message::Channel(command) => return self.channel_command(command),
            // Sorted out by `window_message` before anything else.
            Message::Windowed(..)
            | Message::WindowCloseRequested(_)
            | Message::WindowClosed(_)
            | Message::Escape(_)
            | Message::Key(_)
            | Message::FileDroppedOn(..) => {}
        }
        Task::none()
    }

    /// The toolbar as it shows now.
    fn toolbar(&self) -> Toolbar<'_> {
        let project = self.session.project();
        let (previous_pattern, _) = self.neighboring_patterns();
        Toolbar {
            width: self.window_size.width,
            playing: self.playing,
            tempo: project.tempo,
            pattern_name: self
                .pattern
                .and_then(|id| project.patterns.get(&id))
                .map_or("", |shown| shown.name.as_str()),
            previous_pattern,
            pattern_rename: self
                .patterns
                .rename
                .as_ref()
                .map(|(_, typed)| typed.as_str()),
            mode: self.mode,
            loop_on: project.playlist.loop_range.map(|range| range.on),
            position: self.playhead.unwrap_or(Ticks(0)),
            ppq: project.ppq,
            error: self.error.as_deref(),
            audio_problem: self.audio_problem.as_deref(),
            retry_audio: self.retries_audio(),
            notice: self.notice.as_deref(),
            recording: self.recording_words(),
            menu_open: self.files.menu,
            edit_menu_open: self.edit_menu,
            help_open: self.help.open,
            tempo_edit: self.tempo_edit.as_deref(),
            metronome: self.metronome,
            record: RecordLook {
                armed: self.recording.armed,
                // Four times a second while the count-in plays, by the wall clock: the blink
                // needs no state of its own.
                blink_off: self.session.counting_in()
                    && std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .is_ok_and(|since| since.as_millis() / 250 % 2 == 1),
                input_level: self.input_level(),
                kind: self.recording.kind,
            },
            underruns: self.underruns,
            typing: TypingLook {
                on: self.typing.on,
                octave: typing::octave_name(self.typing.base),
                lower: self.typing.base > typing::LOWEST_BASE,
                higher: self.typing.base < typing::HIGHEST_BASE,
                tip: self.keyboard_tip(),
            },
            panels: self.panels,
            keys: &self.keys,
        }
    }

    /// The keyboard switch's tip: what it plays at which velocity, its keys and its shortcut.
    fn keyboard_tip(&self) -> String {
        let switch = self.keys.first(crate::keys::Action::Keyboard);
        if !self.typing.on {
            return format!(
                "Keyboard plays notes \u{b7} off \u{b7} click or {switch} switches it on"
            );
        }
        let value = self.typing.value_words();
        match self.typing.layout {
            crate::keys::NoteLayout::Piano => format!(
                "Keyboard plays notes \u{b7} {value} \u{b7} {} {} octave, {} {} velocity, Tab sustain \u{b7} click or {switch} switches it off",
                self.typing.labels.label(iced::keyboard::key::Code::KeyZ),
                self.typing.labels.label(iced::keyboard::key::Code::KeyX),
                self.typing.labels.label(iced::keyboard::key::Code::KeyC),
                self.typing.labels.label(iced::keyboard::key::Code::KeyV),
            ),
            crate::keys::NoteLayout::TwoRows => format!(
                "Keyboard plays notes \u{b7} {value} \u{b7} click or {switch} switches it off"
            ),
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let toolbar = self.toolbar();
        let project = self.session.project();

        let panels = responsive(move |area| {
            if !self.panels.browser {
                return self.panels_view(area);
            }
            let rest = Size::new((area.width - browser::WIDTH).max(0.0), area.height);
            row![self.browser_view(), self.panels_view(rest)].into()
        });
        let mut window = column![toolbar.view(), panels];
        if self.shows_hints() {
            window = window.push(self.hint_bar());
        }
        let window = container(window)
            .width(Fill)
            .height(Fill)
            .style(style::window);
        let mut layers = vec![window.into()];
        if let Some(press) = self.browse.press.as_ref().filter(|press| press.dragging) {
            // The dragged file's name follows the cursor, clear of it so it takes no events.
            let ghost = container(text(press.name.clone()).size(12))
                .padding([2, 6])
                .style(style::name_box);
            layers.push(pin(ghost).x(press.at.x + 14.0).y(press.at.y + 10.0).into());
        }
        if let Some(picker) = &self.picker {
            let catalogue = self.host.catalogue();
            layers.push(self.picker_layer(picker, &catalogue));
        }
        if self.files.menu {
            let recent: Vec<(std::path::PathBuf, bool)> = self
                .recent
                .entries()
                .iter()
                .map(|path| (path.clone(), path.exists()))
                .collect();
            layers.push(dialog::file_menu(FileMenu {
                recent,
                mp3_unavailable: export::mp3_unavailable(project.sample_rate),
                keys: &self.keys,
                window_height: self.window_size.height,
                right: self.toolbar().spread().file_menu_right(),
            }));
        }
        if self.edit_menu {
            layers.push(dialog::edit_menu(self.edit_menu_view()));
        }
        if self.patterns.menu {
            layers.push(dialog::pattern_menu(dialog::PatternMenu {
                x: self.toolbar().spread().pattern_name_x(),
                can_delete: self.offers_delete(),
            }));
        }
        if let Some(kind) = self.recording.menu {
            let input = self
                .recording_input_channel()
                .and_then(|id| self.session.project().channels.get(&id))
                .map(|channel| channel.name.clone());
            layers.push(dialog::record_menu(dialog::RecordMenu {
                kind,
                recording: self.recording.kind,
                source: self.recording.source,
                input,
                quantize: self.recording.quantize,
                count_in: self.recording.count_in,
                keys: &self.keys,
                x: self.toolbar().spread().take_column_x(),
            }));
        }
        if self.callout == Some(Callout::Shown) {
            layers.push(self.callout_view());
        }
        if let Some(form) = &self.quantize_form {
            layers.push(dialog::overlay(dialog::quantize(form)));
        }
        if let Some(content) = self.dialog_view() {
            layers.push(dialog::overlay(content));
        }
        stack(layers).into()
    }

    /// The first-run note: under the toolbar's keyboard switch, saying the keyboard plays notes,
    /// which keys do what, which shortcut it takes, and how to switch it off or change it; or
    /// under Record, saying how to record what is played.
    fn callout_view(&self) -> Element<'_, Message> {
        let (anchor, keys) = self.callout_anchor();
        crate::ui::callout::view(anchor, self.window_size.width, self.callout_lines(), keys)
    }

    /// What the first-run note points at, where the toolbar lays it out at its scale, and
    /// whether it offers the Keys settings: the keyboard switch, or Record.
    fn callout_anchor(&self) -> (iced::Rectangle, bool) {
        match self.callout_note {
            CalloutNote::Keyboard => (self.keyboard_switch(), true),
            CalloutNote::Record => (self.toolbar().spread().record_button(), false),
        }
    }

    /// Whether `note` is the first-run note on screen.
    fn callout_shows(&self, note: CalloutNote) -> bool {
        self.callout == Some(Callout::Shown) && self.callout_note == note
    }

    /// The Record button was used: the first-run note pointing at it, on screen, goes.
    pub(super) fn put_away_record_callout(&mut self) {
        if self.callout_shows(CalloutNote::Record) {
            self.put_away_callout();
        }
    }

    /// The first-run note's words: which keys of the layout play, how to switch the keyboard
    /// off or change its keys, and the first shortcut it takes; Record's: arm, press Space,
    /// play, and where the notes land.
    fn callout_lines(&self) -> Vec<String> {
        use crate::keys::Action;
        if self.callout_note == CalloutNote::Record {
            let play = self.keys.first(Action::PlayStop);
            return vec![
                format!("Arm ({}), press {play}, play.", self.keys.first(Action::ArmRecord)),
                "What you play on the keyboard or a MIDI device lands in the pattern as it loops, at the roll's snap.".to_owned(),
                format!("{play} stops the take: one undo step. A second pass adds to the first."),
            ];
        }
        let mut lines = vec![
            self.typing.first_run_words(),
            format!(
                "Turn it off here ({}), or change keys in Settings \u{2192} Keys.",
                self.keys.first(crate::keys::Action::Keyboard)
            ),
        ];
        if let Some(taken) = self.typing.taken_example(&self.keys) {
            lines.push(format!("{taken}."));
        }
        lines
    }

    /// Where the toolbar's keyboard switch lies in the window.
    fn keyboard_switch(&self) -> iced::Rectangle {
        self.toolbar().spread().keyboard_switch()
    }

    /// Puts the first-run note away for good: it shows no more, on this start or the next. The
    /// next note waiting shows in its place.
    fn put_away_callout(&mut self) {
        match self.callout_note {
            CalloutNote::Keyboard => self.settings.put_away_keyboard_callout(),
            CalloutNote::Record => self.settings.put_away_record_callout(),
        }
        self.save_settings();
        self.callout = None;
        if let Some(next) = waiting_note(&self.settings) {
            self.callout_note = next;
            self.callout = Some(Callout::Shown);
        }
    }

    /// Whether the first-run notes show: `--callouts on` shows the first not put away in a
    /// scripted run, `off` hides them.
    pub fn set_callouts(&mut self, on: bool) {
        let waiting = waiting_note(&self.settings);
        self.callout = (on && waiting.is_some()).then_some(Callout::Due);
        self.callout_note = waiting.unwrap_or(CalloutNote::Keyboard);
    }

    /// What the Edit menu offers now: Undo and Redo naming their steps, and Delete, Duplicate
    /// and Select all where they would act (on the panel with the keys, as their keys do).
    fn edit_menu_view(&self) -> dialog::EditMenu<'_> {
        let history = self.session.history();
        let step = |verb: &str, steps: usize, name: Option<&str>| {
            (steps > 0)
                .then(|| name.map_or_else(|| verb.to_owned(), |name| format!("{verb} {name}")))
        };
        let selected = if self.roll_has_keys() {
            !self.roll.selection.is_empty()
        } else {
            self.playlist_has_keys() && !self.arranger.selection.is_empty()
        };
        dialog::EditMenu {
            undo: step("Undo", history.undo_len(), history.undo_name()),
            redo: step("Redo", history.redo_len(), history.redo_name()),
            selected,
            notes: self.roll_has_keys()
                && Target::of(self.session.project(), self.pattern, self.selected_channel)
                    .is_some(),
            keys: &self.keys,
            right: self.toolbar().spread().edit_menu_right(),
        }
    }

    /// The dialog on screen, if any.
    fn dialog_view(&self) -> Option<Element<'_, Message>> {
        use files::Modal;
        Some(match self.files.modal.as_ref()? {
            Modal::Unsaved => dialog::unsaved(&self.document.name),
            Modal::Recover { file, age, .. } => dialog::recover(age, file.is_some()),
            Modal::Report { summary, groups } => dialog::report(summary, groups),
            Modal::Export(_) => dialog::export(self.export_form()?),
        })
    }

    /// The open panels stacked in `area`, the space under the toolbar, top to bottom: the rack,
    /// the playlist, the piano roll and the mixer, each as tall as
    /// [`panels::stack_heights`] says, each under another with the divider over it on its top
    /// edge ([`App::divider_view`]). The last takes whatever the whole pixels above it leave.
    fn panels_view(&self, area: Size) -> Element<'_, Message> {
        let stacked: Vec<Panel> = self.panels.stacked().collect();
        if stacked.is_empty() {
            return well(space().into()).height(Fill).into();
        }
        let heights = panels::stack_heights(
            &stacked,
            self.rack_content(),
            &self.settings.kept_heights(),
            area.height,
        );
        let last = stacked.len() - 1;
        let shown = stacked
            .iter()
            .zip(heights)
            .enumerate()
            .map(|(index, (&panel, height))| {
                // Whole pixels, so the lines of the panels under it stay on whole pixels too.
                let height = if index == last {
                    iced::Length::Fill
                } else {
                    iced::Length::Fixed(height.floor())
                };
                let above = index.checked_sub(1).map(|above| stacked[above]);
                let divider = self.divider_view(above, panel);
                let view = match panel {
                    // The rack is the top panel whenever it is open: no divider over it.
                    Panel::Rack => return self.rack_well().height(height).into(),
                    Panel::Playlist => self.playlist_view(),
                    Panel::PianoRoll => self.piano_roll_view(),
                    Panel::Mixer => self.mixer_view(),
                    Panel::Browser | Panel::Settings => {
                        unreachable!("the Library and Settings are no stacked panels")
                    }
                };
                let view = match divider {
                    Some(divider) => column![divider, view].into(),
                    None => view,
                };
                container(view).height(height).into()
            });
        column(shown).height(Fill).into()
    }

    /// The rack's content height: its header, its rows and, while its steps reach past what
    /// its rows show, the bar under them.
    fn rack_content(&self) -> f32 {
        let bar = if self.steps_overflow() {
            crate::ui::scroll::BAR_HEIGHT
        } else {
            0.0
        };
        layout::RACK_HEADER_HEIGHT + self.rack_size().height + bar
    }

    /// When `opened`, a stacked panel just opened, leaves the open ones no room for their least
    /// heights, keeps the most recently used that fit beside it, closes the rest and says
    /// which in a notice; they come back when `opened` is closed by hand ([`App::bring_back`]).
    /// The Library and a resize close nothing.
    pub(super) fn make_room_for(&mut self, opened: Panel) {
        // Opened, it waits for no panel to close any more.
        self.displaced.forget(opened);
        let closed = panels::closed_to_fit(
            self.panels,
            opened,
            &self.panel_use,
            self.rack_content(),
            self.panels_height(),
        );
        if closed.is_empty() {
            return;
        }
        for &panel in &closed {
            self.panels.set(panel, false);
        }
        self.displaced.record(opened, &closed);
        self.notice = Some(format!(
            "{} closed to make room for {}",
            titles(&closed),
            opened.title()
        ));
    }

    /// `closed` was just closed by its key or button: the panels its opening closed to make
    /// room open again, the most recently used first, each while it fits, and the notice says
    /// which. Why: a digit is a panel's key one row above the notes, so a finger can open one
    /// by accident, and the same digit again must undo all of it.
    fn bring_back(&mut self, closed: Panel) -> Task<Message> {
        let waiting = self.displaced.take(closed);
        let mut back = Vec::new();
        for panel in self.panel_use.order().into_iter().rev() {
            if !waiting.contains(&panel) {
                continue;
            }
            let mut stacked: Vec<Panel> = self.panels.stacked().collect();
            stacked.push(panel);
            if panels::fits(&stacked, self.rack_content(), self.panels_height()) {
                self.panels.set(panel, true);
                back.push(panel);
            }
        }
        if back.is_empty() {
            return Task::none();
        }
        self.notice = Some(format!("{} open again", titles(&back)));
        let opened: Vec<Task<Message>> = back
            .into_iter()
            .map(|panel| self.panel_opened(panel))
            .collect();
        Task::batch(opened)
    }

    /// What a panel needs as it opens: the Library looks at the sets on disk again, the piano
    /// roll shows the selected channel's notes, the mixer reads its meters afresh, the settings
    /// list the output devices again.
    fn panel_opened(&mut self, panel: Panel) -> Task<Message> {
        match panel {
            // The sets on disk may have changed while the Library was closed.
            Panel::Browser => return self.scan_sets(),
            Panel::PianoRoll => self.show_roll(),
            // Peaks gathered while the mixer was closed are not shown.
            Panel::Mixer => self.read_meters(false),
            // The devices plugged in may have changed while the settings were closed.
            Panel::Settings => return Task::batch([self.list_outputs(), self.list_inputs()]),
            Panel::Rack | Panel::Playlist => {}
        }
        Task::none()
    }

    fn playlist_view(&self) -> Element<'_, Message> {
        Playlist {
            project: self.session.project(),
            arranger: &self.arranger,
            marker: self.playlist_marker(),
            scale_factor: self.scale_factor,
            ruler_cache: &self.ruler_cache,
            tracks_cache: &self.tracks_cache,
            files: &self.looks.files,
            names: &self.looks.names,
            stretching: &self.looks.stretching,
            properties: self.clip_properties(),
            browser_drag: self.browse.dragging_to_tracks(),
            armed: self.recording.track,
            follow: self.settings.follows(Panel::Playlist),
            menu_foot: self.hint_bar_height(),
            link: self.selection_link(),
        }
        .view()
    }

    /// Whether the playhead timer runs for the live notes: while the piano roll is open and a
    /// note is held, or a key or port played or let go of one in the last [`LIVE_WAKE`].
    fn live_notes_due(&self) -> bool {
        let recent = self
            .live_wake
            .is_some_and(|when| when.elapsed() < LIVE_WAKE);
        self.panels.piano_roll && (!self.live_notes.is_empty() || recent)
    }

    /// Whether the piano roll's keys (the arrows, Delete, Ctrl+D, C, X, V and A) act on it:
    /// while it is open and was the last panel pressed in, or while the cursor is over it with
    /// notes selected.
    pub(super) fn roll_has_keys(&self) -> bool {
        let over = matches!(self.hover, Some(crate::ui::hint::Hover::Roll(_)));
        self.panels.piano_roll
            && (self.focus == Panel::PianoRoll || (over && !self.roll.selection.is_empty()))
    }

    /// Whether the browser's keys (the arrows, Enter) act on it.
    fn browser_has_keys(&self) -> bool {
        self.focus == Panel::Browser && self.panels.browser
    }

    fn mixer_view(&self) -> Element<'_, Message> {
        MixerPanel {
            project: self.session.project(),
            state: &self.mixer,
            height: self.settings.kept_heights().mixer_height(),
            registry: self.session.registry(),
            channel: self.selected_channel,
            cache: &self.mixer_cache,
            now: Instant::now(),
            automated: &self.automated,
            menu: self.automate_menu_view(Panel::Mixer),
            menu_foot: self.hint_bar_height(),
        }
        .view()
    }

    /// The song position the playlist shows: in song mode, the playhead while playing and the
    /// play-from tick while stopped; none in pattern mode.
    fn playlist_marker(&self) -> Option<Ticks> {
        (self.mode == PlayMode::Song).then(|| self.playhead.unwrap_or(self.song_marker))
    }

    /// The rack canvas's size: every channel row, every step of the pattern, and the row of
    /// the add button.
    fn rack_size(&self) -> Size {
        let project = self.session.project();
        let steps = step_grid::step_count(project, self.pattern);
        let size = layout::rack_canvas_size(self.rack_rows().len(), steps);
        // A sampler without a sound says so right of its steps.
        let hint = project
            .channels
            .values()
            .any(crate::ui::rack::is_empty_sampler);
        if hint {
            Size::new(size.width + crate::ui::rack::HINT_WIDTH, size.height)
        } else {
            size
        }
    }

    /// The rack under its header, in its well, which says when the cursor comes onto it or
    /// leaves (a file dragged from the browser lands there as a new sampler channel).
    fn rack_well(&self) -> container::Container<'_, Message> {
        let header = canvas(self.rack_header())
            .width(Fill)
            .height(layout::RACK_HEADER_HEIGHT);
        let rack = mouse_area(column![header, self.rack_view()])
            .on_enter(Message::Media(MediaMessage::RackHover(true)))
            .on_exit(Message::Media(MediaMessage::RackHover(false)));
        well(rack.into())
    }

    /// The rack's rows, the audio takes under their Recordings header, folded unless opened.
    pub(super) fn rack_rows(&self) -> crate::ui::rack_rows::RackRows {
        crate::ui::rack_rows::RackRows::of(self.session.project(), self.takes_open)
    }

    /// The rack's rows as their canvas draws them and hit-tests presses on them.
    pub(super) fn channel_rack(&self) -> ChannelRack<'_> {
        let project = self.session.project();
        let steps = step_grid::step_count(project, self.pattern);
        ChannelRack {
            project,
            pattern: self.pattern,
            selected: self.selected_channel,
            playhead: self
                .playhead
                .and_then(|position| step_grid::step_at(position, project.ppq, steps)),
            automated: &self.automated,
            cache: &self.rack_cache,
            browser_drag: self.browse.dragging_file(),
            drop_sampler: self.browse.drop_sampler,
            rows: self.rack_rows(),
        }
    }

    /// The rack steps' scroll, in pixels from the first step: the steps (and an empty sampler's
    /// hint) are the content, what the rows' canvas shows right of the knobs the view. Nothing
    /// scrolls until the canvas has said its width.
    fn steps_span(&self) -> crate::ui::scroll::Span {
        let content = f64::from(self.rack_size().width - layout::STEPS_X);
        let view = f64::from(self.rack_width - layout::STEPS_X);
        let known = self.rack_width > layout::STEPS_X;
        let span = crate::ui::scroll::Span {
            content,
            offset: 0.0,
            view: if known { view } else { content },
        };
        crate::ui::scroll::Span {
            offset: f64::from(self.rack_steps).clamp(0.0, span.last_offset()),
            ..span
        }
    }

    /// Keeps the steps' scroll inside their reach once the rows' width is known (a shorter
    /// pattern, a wider window), redrawing the rows when that moved it.
    fn keep_rack_steps(&mut self) {
        if self.rack_width <= layout::STEPS_X {
            return;
        }
        let kept = self.steps_span().offset as f32;
        if kept != self.rack_steps {
            self.rack_steps = kept;
            self.rack_cache.clear();
        }
    }

    /// Whether the rack's steps reach past what its rows show: the bar under them shows then.
    fn steps_overflow(&self) -> bool {
        let span = self.steps_span();
        span.content > span.view || span.offset > 0.0
    }

    fn rack_view(&self) -> Element<'_, Message> {
        let project = self.session.project();
        let size = self.rack_size();
        let steps = self.steps_span();
        let rows = crate::ui::rack::ScrolledRack {
            rack: self.channel_rack(),
            steps,
            known_width: self.rack_width,
            shift: self.modifiers.shift(),
        };
        let rack = canvas(rows).width(Fill).height(size.height);
        let menu = self
            .automate_menu_view(Panel::Rack)
            .or_else(|| self.sampler_menu_view());
        let mut layers: Vec<Element<'_, Message>> = vec![rack.into()];
        layers.extend(menu);
        layers.extend(self.channel_rename_view());
        // A MIDI device held from the Library: which row it is over, outlined.
        if self.browse.dragging_device() {
            let over = DeviceDrop {
                project,
                over: self.browse.drop_channel,
                rows: self.rack_rows(),
            };
            layers.push(canvas(over).width(Fill).height(size.height).into());
        }
        // A sound, a set or a plugin held (what lands on a playlist lane): the header it would
        // replace the sound of, outlined.
        if self.browse.dragging_to_tracks() {
            let over = SoundDrop {
                project,
                over: self.browse.drop_header,
                rows: self.rack_rows(),
            };
            layers.push(canvas(over).width(Fill).height(size.height).into());
        }
        let rack: Element<'_, Message> = match layers.len() {
            1 => layers.remove(0),
            _ => stack(layers).into(),
        };
        // The rows scroll up and down as before; along, only their steps do, by the bar under
        // them, the light, name and knobs staying put. The bar's row is there at no height
        // while the steps fit. Why always: iced keeps a widget's state only while the view
        // keeps its shape, and the rows' scrollable keeps how far they are scrolled down.
        let rows = scrollable(rack)
            .direction(Direction::Vertical(Scrollbar::default()))
            .width(Fill)
            .height(Fill);
        let height = if self.steps_overflow() {
            crate::ui::scroll::BAR_HEIGHT
        } else {
            0.0
        };
        let bar = canvas(crate::ui::scroll::Scrollbar {
            span: steps,
            message: Message::RackBar,
        })
        .width(Fill)
        .height(height);
        let under_knobs = space().width(layout::STEPS_X).height(height);
        column![rows, row![under_knobs, bar]].into()
    }

    fn piano_roll_view(&self) -> Element<'_, Message> {
        PianoRoll {
            project: self.session.project(),
            pattern: self.pattern,
            channel: self.selected_channel,
            roll: &self.roll,
            modifiers: self.modifiers,
            playhead: self.playhead,
            origin: self.roll_origin(),
            grid_cache: &self.roll_grid_cache,
            lane_cache: &self.roll_lane_cache,
            preview: self.settings.preview_notes(),
            follow: self.settings.follows(Panel::PianoRoll),
            clipboard: self.clipboard.is_some(),
            menu_foot: self.hint_bar_height(),
            live: &self.live_notes,
            recording_notes: self.recording.taking_notes(),
        }
        .view()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let meters_moving = self.panels.mixer && self.mixer.animating(Instant::now());
        let playhead = if self.transport_on
            || self.playing
            || meters_moving
            || self.input_moving()
            || self.measuring_latency()
            || self.live_meters_shown()
            || self.live_notes_due()
            || self.playlist_edge_scrolls()
        {
            time::every(PLAYHEAD_PERIOD).map(|_| Message::Tick)
        } else {
            Subscription::none()
        };
        let audio = if self.audio.is_some() || self.plugins_busy() {
            time::every(AUDIO_CHECK_PERIOD).map(|_| Message::AudioCheck)
        } else {
            Subscription::none()
        };
        let retry = if self.retries_audio() {
            time::every(AUDIO_RETRY_PERIOD).map(|_| Message::RetryAudio)
        } else {
            Subscription::none()
        };
        let drag = match &self.drag {
            Some(drag) if drag.target.horizontal() => event::listen_with(slider_drag_message),
            Some(_) => event::listen_with(drag_message),
            None => Subscription::none(),
        };
        let autosave = if self.host.autosaves() {
            time::every(files::AUTOSAVE_PERIOD).map(|_| Message::File(FileMessage::AutosaveTick))
        } else {
            Subscription::none()
        };
        let export = if self.exporting() {
            time::every(export::PROGRESS_PERIOD)
                .map(|_| Message::File(FileMessage::ExportForm(ExportMessage::Poll)))
        } else {
            Subscription::none()
        };
        // With nothing due by the clock, the plugins wake the window only by asking.
        let plugins = if self.plugins_due() {
            time::every(PLUGIN_PERIOD).map(|_| Message::PluginTick)
        } else if let Some(doorbells) = self.plugin_doorbells() {
            Subscription::run_with(doorbells, plugins::Doorbells::rung)
                .map(|()| Message::PluginTick)
        } else {
            Subscription::none()
        };
        let panel = if self.panel.is_some() {
            time::every(PANEL_PERIOD).map(|_| Message::PanelTick)
        } else {
            Subscription::none()
        };
        // The sample editor's: Play's position and the sounds it makes in the background, and
        // the Gain readout's drag, whichever window the cursor is over.
        let editor = if self.editor_ticks() {
            time::every(PLAYHEAD_PERIOD).map(|_| Message::SampleEditor(SampleEditorMessage::Tick))
        } else {
            Subscription::none()
        };
        let editor_gain = if self.editor_gain_held() {
            event::listen_with(editor_gain_message)
        } else {
            Subscription::none()
        };
        let media = if self.browse.waiting() || self.looks.waiting() || self.session.loading() {
            time::every(browse::MEDIA_PERIOD).map(|_| Message::Media(MediaMessage::Tick))
        } else {
            Subscription::none()
        };
        // A burst of moves (and a control waiting to be linked) runs on the timer, which also
        // times the burst's end; with a port connected and nothing under way, moves wake the
        // window themselves.
        let controllers = if self.controllers_busy() {
            time::every(controllers::CONTROLLER_PERIOD).map(|_| Message::ControllerTick)
        } else if self.ports.connected() {
            Subscription::run_with(self.controller_queue.clone(), ControllerQueue::moves)
                .map(|()| Message::ControllerTick)
        } else {
            Subscription::none()
        };
        let ports = match self.listing_period() {
            Some(period) => time::every(period).map(|_| Message::PortsTick),
            None => Subscription::none(),
        };
        // The lights: a port sending after a quiet spell wakes the window, which then looks on
        // a timer while one is lit; with nothing sent, the window sleeps.
        let lights = if self.midi_lights_due() {
            time::every(MIDI_LIGHT_PERIOD).map(|_| Message::MidiActivity)
        } else if self.ports.connected() {
            Subscription::run_with(self.midi_activity.clone(), crate::midi::Activity::arrivals)
                .map(|()| Message::MidiActivity)
        } else {
            Subscription::none()
        };
        let scripted_plugin = if self.scripted_plugin.is_some() {
            window::open_events().map(|_| Message::ScriptedPluginDue)
        } else {
            Subscription::none()
        };
        // One listener, on or off: the app decides what a key means (`App::key_message`).
        let keys = event::listen_with(key_input::key_event);
        // The first-run note shows once the window has drawn a frame.
        let callout = if self.callout == Some(Callout::Due) {
            window::frames().map(|_| Message::Callout(CalloutMessage::Due))
        } else {
            Subscription::none()
        };
        let note_preview = if self.note_preview.is_some() {
            time::every(note_preview::PERIOD).map(|_| Message::NotePreviewTick)
        } else {
            Subscription::none()
        };
        let rehearsal = if self.rehearsing() {
            time::every(std::time::Duration::from_millis(REHEARSAL_MS))
                .map(|_| Message::RehearsalTick)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            rehearsal,
            self.headless.subscription().map(Message::Headless),
            note_preview,
            callout,
            keys,
            event::listen_with(rescale_message),
            crate::window_stats::frames(),
            window::close_requests().map(Message::WindowCloseRequested),
            window::close_events().map(Message::WindowClosed),
            playhead,
            audio,
            retry,
            drag,
            autosave,
            export,
            plugins,
            panel,
            editor,
            editor_gain,
            media,
            controllers,
            ports,
            lights,
            scripted_plugin,
        ])
    }

    /// Plays; while Record is armed, records a take instead.
    fn play(&mut self) {
        // The measurement plays clicks of its own; Stop ends it.
        if self.measuring_latency() {
            return;
        }
        // A device chosen in Settings since the stream opened plays from now.
        self.apply_output_choice();
        if self.recording.armed {
            if !self.recording.recording() {
                self.transport_on = self.start_take();
            }
            return;
        }
        let result = self.session.send(Control::Play);
        self.transport_on = result.is_ok();
        self.report(result);
    }

    /// Stops, and completes a take being recorded.
    fn stop(&mut self) {
        self.cancel_latency();
        self.transport_on = false;
        // Read before Stop moves the playhead back: held notes end where the take stopped.
        let note_end = self.note_take_end();
        let result = self.session.send(Control::Stop);
        self.report(result);
        self.forget_live_notes();
        self.finish_take();
        if let Some(tick) = note_end {
            self.finish_note_take(tick);
        }
        self.settle_controllers();
    }

    /// Reads the player's state and, while the mixer shows, its meters; and hands the player
    /// any plan or value that waited for room.
    fn read_playhead(&mut self) {
        // A take whose writer died stops now, with its error, rather than at the next Stop; so
        // does a pattern's take that holds its one pass.
        if self.recording.take_failed() || self.recording.take_full() {
            self.stop();
        }
        self.poll_latency();
        self.session.flush();
        self.playing = self.session.playing();
        // The playhead too, read after the events. Before the views follow: a song take grows
        // its pattern to hold the playhead as it hears the player (found in use: the roll
        // followed the old length, a bar short, and jumped back at every bar), the same
        // playhead the views then follow.
        self.read_player_events();
        self.follow_playhead();
        self.underruns = self.session.underruns();
        self.read_input_meter();
        self.read_live_meters();
        self.read_automation();
        if self.panels.mixer {
            self.read_meters(true);
        }
    }

    /// Scrolls each view that scrolls along time to its bar's end, as a drag of its thumb there
    /// would: `--scroll end`, for a screenshot of the views scrolled.
    pub fn scroll_to_end(&mut self) {
        let target = Target::of(self.session.project(), self.pattern, self.selected_channel);
        let offset = self.roll.span(target.as_ref()).last_offset();
        let to = crate::ui::scroll::BarMessage::To {
            offset,
            held: false,
        };
        let _ = self.update(Message::Roll(RollMessage::Bar(to)));
        let offset = self.arranger.span(self.session.project()).last_offset();
        let to = crate::ui::scroll::BarMessage::To {
            offset,
            held: false,
        };
        let _ = self.update(Message::Playlist(PlaylistMessage::Bar(to)));
        // As far as the steps reach, once the rows say their width.
        self.rack_steps = f32::MAX;
        self.keep_rack_steps();
        // The parameter window's list too, once its window has opened.
        self.params.to_end = true;
    }

    /// Whether a playlist drag holds the pointer past the lanes' left or right edge with room
    /// to scroll that way: the ticks then run, stopped or playing, and each moves the view.
    fn playlist_edge_scrolls(&self) -> bool {
        self.panels.playlist && self.arranger.edge_scroll(self.session.project()).is_some()
    }

    /// One tick of a playlist drag past an edge: the view moves towards it, and what it draws
    /// is drawn again.
    fn scroll_playlist_at_edge(&mut self) {
        if self.panels.playlist && self.arranger.scroll_at_edge(self.session.project()) {
            self.ruler_cache.clear();
            self.tracks_cache.clear();
        }
    }

    /// While playing, each open view whose Follow is on jumps a page when the playhead leaves
    /// it: the piano roll by where the playhead falls in its pattern, the playlist by the song
    /// position it shows (in song mode).
    fn follow_playhead(&mut self) {
        let Some(tick) = self.playhead else {
            return;
        };
        let song = self.playlist_marker().filter(|_| self.panels.playlist);
        if let (Some(at), true) = (song, self.settings.follows(Panel::Playlist)) {
            if self.arranger.follow(self.session.project(), at) {
                self.ruler_cache.clear();
                self.tracks_cache.clear();
            }
        }
        if self.panels.piano_roll && self.settings.follows(Panel::PianoRoll) {
            let target = Target::of(self.session.project(), self.pattern, self.selected_channel);
            let at = Ticks(tick.0 - self.roll_origin().0);
            if self.roll.follow(target.as_ref(), at) {
                self.roll_grid_cache.clear();
                self.roll_lane_cache.clear();
            }
        }
    }

    /// Takes every insert's peak since the last read, and shows it when `show`.
    fn read_meters(&mut self, show: bool) {
        let peaks: Vec<[f32; 2]> = self
            .session
            .project()
            .mixer
            .ids()
            .map(|id| self.session.take_peak(id))
            .collect();
        if show {
            self.mixer.meters.update(&peaks, Instant::now());
        }
    }

    /// Shows what the output stream has reported since it started, and lets a failed stream
    /// go, its player parked. Why park it: the stream never calls it again, but the document
    /// still has the plugins in it, which must not go with the stream.
    fn check_audio(&mut self) {
        self.show_dropped_notes();
        self.check_live_inputs();
        let Some(output) = &self.audio else {
            return;
        };
        let (problem, failed) = (output.problem(), output.failed());
        self.show_stream_problem(problem);
        if failed {
            self.audio_note = None;
            if let Some(player) = self.audio.take().and_then(Output::stop) {
                self.session.park(player);
            }
        }
    }

    /// Whether the player is parked without a device, so the device is tried again.
    fn retries_audio(&self) -> bool {
        self.audio.is_none() && self.session.is_parked()
    }

    /// Tries the output device again for the parked player. Once it opens, the player, with
    /// every processor and plugin in it as it was, plays through it; else it is parked again
    /// and the toolbar says why.
    fn retry_audio(&mut self) {
        if !self.retries_audio() {
            return;
        }
        let Some(player) = self.session.unpark() else {
            return;
        };
        let rate = self.session.project().sample_rate;
        self.start_audio(player, rate);
    }

    fn show_stream_problem(&mut self, problem: Option<&'static str>) {
        self.audio_problem = problem.map(Cow::Borrowed);
    }

    /// Applies `edit` as one undo step. A send the model refuses because it would close a loop
    /// flashes its route button and is explained in the mixer's header.
    fn commit(&mut self, edit: Edit) {
        self.finish_drag();
        let result = self.session.apply(edit);
        if let Err(SessionError::Edit(error @ EditError::RoutingCycle { to, .. })) = &result {
            let mut message = error.to_string();
            // The header starts the sentence.
            if let Some(first) = message.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            self.mixer.refuse(*to, message, Instant::now());
        }
        self.after_edit(result);
    }

    /// As [`commit`](Self::commit), as part of the newest undo step: one undo takes back both.
    fn commit_into_newest(&mut self, edit: Edit) {
        self.finish_drag();
        let result = self.session.apply_into_newest(edit);
        self.after_edit(result);
    }

    /// Applies a piano-roll gesture's edit as one undo step and, once it went through, holds
    /// the selection it leaves.
    fn commit_notes(&mut self, commit: Commit) {
        self.finish_drag();
        let result = self.session.apply(commit.edit);
        if let (Ok(()), Some(selection)) = (&result, commit.selection) {
            self.roll.selection = selection;
        }
        self.after_edit(result);
    }

    /// Runs a piano-roll key command (Delete, Ctrl+B) while the piano roll is open.
    fn note_command(&mut self, command: fn(&Roll, &Target) -> Option<Commit>) {
        if !self.panels.piano_roll {
            return;
        }
        self.roll.cancel_gesture();
        let target = Target::of(self.session.project(), self.pattern, self.selected_channel);
        if let Some(commit) = target.and_then(|target| command(&self.roll, &target)) {
            self.commit_notes(commit);
        }
    }

    /// Shows the selected channel in the piano roll from its top again.
    fn show_roll(&mut self) {
        let target = Target::of(self.session.project(), self.pattern, self.selected_channel);
        self.roll.show(target.as_ref());
    }

    fn start_drag(&mut self, target: DragTarget) {
        self.finish_drag();
        self.plugin_sent = None;
        if let Some(start) = self.drag_value(target) {
            self.drag = Some(Drag {
                target,
                start,
                anchor_y: None,
                travel: 0.0,
                undo: None,
            });
        }
    }

    /// Sets the dragged value for the cursor at `along` (window y, or x for a slider), live and
    /// without an undo step.
    fn drag_to(&mut self, along: f32) {
        let Some(drag) = &mut self.drag else {
            return;
        };
        let anchor = *drag.anchor_y.get_or_insert(along);
        drag.travel = drag.travel.max((along - anchor).abs());
        let (target, start) = (drag.target, drag.start);
        let rise = if target.horizontal() {
            along - anchor
        } else {
            anchor - along
        };
        let project = self.session.project();
        let (value, edit, live) = match target {
            DragTarget::Knob { channel, knob } => {
                let range = knob.range();
                let span = range.end() - range.start();
                let value = start + rise / KNOB_DRAG_PIXELS * span;
                let value = value.clamp(*range.start(), *range.end());
                (value, knob.edit(channel, value), Some(knob.target(channel)))
            }
            DragTarget::Tempo => {
                let steps = (rise / TEMPO_DRAG_PIXELS).round() as i64;
                // `start` holds a tempo in milli-BPM, which an f32 represents exactly.
                let mbpm = self.step_tempo(MilliBpm(start as u32), steps);
                (mbpm.0 as f32, Edit::SetTempo { mbpm }, None)
            }
            DragTarget::Mixer(control) => {
                let range = control.range(project, self.session.registry());
                let fine = if self.modifiers.shift() {
                    FINE_DRAG
                } else {
                    1.0
                };
                let moved = rise / control.sweep_pixels() * (range.end() - range.start()) * fine;
                let value = control.clamp(start + moved, project, self.session.registry());
                let Some(edit) = control.edit(value, project, self.session.registry()) else {
                    return;
                };
                (value, edit, Some(control.target()))
            }
            DragTarget::Plugin(plugin) => {
                let fine = if self.modifiers.shift() {
                    FINE_DRAG
                } else {
                    1.0
                };
                let value = plugin.moved(start, rise, fine);
                if self.drag_value(target) != Some(value) {
                    self.set_plugin_param(plugin, value);
                }
                return;
            }
        };
        // Why skip a move that keeps the value (the first move, which only sets the anchor, or
        // a sideways one): applying it would send the player a value it already has.
        if self.drag_value(target) == Some(value) {
            return;
        }
        let result = match live {
            Some(param) => self.session.apply_param(edit, param, value),
            None => self.session.apply_live(edit),
        };
        if let (Ok(inverse), Some(drag)) = (&result, &mut self.drag) {
            drag.undo.get_or_insert_with(|| inverse.clone());
        }
        self.after_edit(result.map(drop));
    }

    /// Ends a drag: sends the one plan that carries its value and records it as one undo step,
    /// if it left the value changed.
    fn finish_drag(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if let DragTarget::Plugin(plugin) = drag.target {
            // One undo step per gesture, by the state the plugin has at its end.
            if self.drag_value(drag.target) != Some(drag.start) {
                self.capture_soon(plugin.owner);
            }
            self.plugin_sent = None;
            return;
        }
        if let Err(error) = self.session.commit_params() {
            // The player keeps the old plan, so the document goes back to it.
            if let Some(undo) = drag.undo {
                let _ = self.session.apply_live(undo);
            }
            self.after_edit(Err(error));
            return;
        }
        // Why compare with the start: a drag that comes back to it changed nothing, and
        // recording it would still empty the redo stack. A control whose channel, insert or
        // effect is gone has nothing left to restore.
        let changed = self
            .drag_value(drag.target)
            .is_some_and(|now| now != drag.start);
        if !changed {
            return;
        }
        if let Some(undo) = drag.undo {
            self.session.record(undo);
        }
    }

    /// Fine control switched on or off half way through a mixer drag: the drag goes on from
    /// where it is, at the new speed, rather than jumping.
    fn restart_mixer_drag(&mut self) {
        let Some(drag) = &self.drag else {
            return;
        };
        if !matches!(drag.target, DragTarget::Mixer(_)) {
            return;
        }
        let now = self.drag_value(drag.target);
        if let (Some(now), Some(drag)) = (now, &mut self.drag) {
            drag.start = now;
            drag.anchor_y = None;
        }
    }

    /// The dragged control's value now, as a knob value, milli-BPM or the mixer control's
    /// value; `None` once its channel, insert or effect is gone.
    fn drag_value(&self, target: DragTarget) -> Option<f32> {
        let project = self.session.project();
        match target {
            DragTarget::Knob { channel, knob } => project
                .channels
                .get(&channel)
                .map(|shown| knob.value(shown)),
            // Tempos stay below 2^24 milli-BPM, so the f32 is exact.
            DragTarget::Tempo => Some(project.tempo.0 as f32),
            DragTarget::Mixer(control) => control.value(project, self.session.registry()),
            DragTarget::Plugin(plugin) => self.plugin_value(plugin),
        }
    }

    /// `tempo` moved by `steps` whole-BPM steps, or 0.001 BPM steps while Shift is held.
    fn step_tempo(&self, tempo: MilliBpm, steps: i64) -> MilliBpm {
        let step = if self.modifiers.shift() {
            TEMPO_FINE_STEP
        } else {
            TEMPO_STEP
        };
        nudged_tempo(tempo, steps, step)
    }

    /// Switches the play mode: the next plan plays the selected pattern or the arrangement,
    /// and the playhead goes to tick 0 for a pattern or to the song marker for the song.
    fn select_mode(&mut self, mode: PlayMode) {
        if mode == self.mode {
            return;
        }
        self.keep_take("Take kept: the play mode changed");
        self.mode = mode;
        self.send_mode();
        let from = match mode {
            PlayMode::Pattern => Ticks(0),
            PlayMode::Song => self.song_marker,
        };
        let result = self.session.send(Control::Seek(from));
        self.report(result);
    }

    /// Whether the editing keys act on the playlist.
    fn playlist_has_keys(&self) -> bool {
        self.focus == Panel::Playlist && self.panels.playlist
    }

    /// Hands a playlist message to its state and carries out what that asks for.
    fn playlist_message(&mut self, message: PlaylistMessage) -> Task<Message> {
        // A new layout size or place is not a press: it must not take the keys.
        if !matches!(
            message,
            PlaylistMessage::TracksResized(_) | PlaylistMessage::Placed(_)
        ) {
            self.focus = Panel::Playlist;
        }
        let project = self.session.project();
        let lanes = self.arranger.lanes();
        let command = self
            .arranger
            .update(message, project, self.pattern, self.modifiers);
        if self.arranger.lanes() != lanes {
            self.settings.set_playlist_lanes(self.arranger.lanes());
            self.save_settings();
        }
        match command {
            Some(arranger::Command::Edit(edit)) => self.commit(edit),
            Some(arranger::Command::Seek(tick)) => self.move_song_marker(tick),
            Some(arranger::Command::FocusRename) => {
                let id = Id::new(RENAME_INPUT);
                return operation::focus(id.clone()).chain(operation::select_all(id));
            }
            Some(arranger::Command::Arm(track)) => self.arm_track(track),
            Some(arranger::Command::Stretch { clip, end }) => self.stretch_clip(clip, end),
            Some(arranger::Command::StretchMode { clip, mode }) => self.set_clip_mode(clip, mode),
            Some(arranger::Command::OpenInRoll(clip)) => return self.open_clip_in_roll(clip),
            Some(arranger::Command::EditSample(clip)) => self.open_sample_editor_on_clip(clip),
            Some(arranger::Command::Link(link)) => self.clip_link(link),
            None => {}
        }
        Task::none()
    }

    /// A ruler click: song mode plays from `tick` now (jumping there while playing) and Stop
    /// returns there; in pattern mode the marker waits for the switch to song mode.
    fn move_song_marker(&mut self, tick: Ticks) {
        self.song_marker = tick;
        if self.mode == PlayMode::Song {
            let result = self.session.send(Control::Seek(tick));
            self.report(result);
        }
    }

    /// Plays the selected pattern or the arrangement from the next plan on.
    fn send_mode(&mut self) {
        let result = self.session.set_mode(self.plan_mode());
        self.after_edit(result);
    }

    /// Drops what the piano roll, the playlist and the mixer have drawn, so they draw the
    /// document again.
    fn clear_panel_caches(&mut self) {
        self.roll_grid_cache.clear();
        self.roll_lane_cache.clear();
        self.ruler_cache.clear();
        self.tracks_cache.clear();
        self.mixer_cache.clear();
        self.instrument_caches.clear();
        self.editor.caches.wave.clear();
    }

    /// Keeps the roll on its clip (or forgets it), the selected pattern and channel ones that
    /// exist, redraws the rack after a change, keeps the piano roll's selection to notes that
    /// still exist, lets the mixer drop what is gone, and shows a failure in the toolbar.
    fn after_edit(&mut self, result: Result<(), SessionError>) {
        let kept = self
            .keep_roll_clip()
            .and(self.keep_pattern())
            .and(self.keep_channel());
        let result = result.and(kept);
        self.keep_instrument();
        self.keep_sample_editor();
        self.keep_rack_steps();
        self.rack_cache.clear();
        let target = Target::of(self.session.project(), self.pattern, self.selected_channel);
        self.roll.refresh(target.as_ref());
        self.arranger.refresh(self.session.project());
        self.mixer.refresh(self.session.project());
        self.refresh_looks();
        self.read_automation();
        self.forget_gone_plugins();
        self.show_plugin_notices();
        self.route_ports();
        // A MIDI out set, changed or gone ends the notes of its old route.
        self.route_midi_out();
        // An input channel added, changed or gone opens or closes its device.
        self.sync_live_inputs(false);
        // The Library shows each device's channel by name.
        self.browse.cache.clear();
        self.report(result);
    }

    /// Selects `channel`: the rack outlines it, the piano roll shows its notes, the mixer its
    /// insert, and live notes play it. Every selection the app makes goes through here (a click
    /// on a name, an add, a sound replaced, the picker), and an open instrument window follows
    /// it, so the window always shows the channel the keys play. A channel chosen ends the
    /// Library's preview, so the keys play the rack again.
    fn select_channel(&mut self, channel: ChannelId) {
        self.press_channel(channel);
        self.instrument_follows(channel);
    }

    /// Selects `channel` as a press on its rack name does: as [`select_channel`] says, but an
    /// open instrument window stays where it is; a double press ([`Message::ChannelOpened`])
    /// takes the window there. The keys play the selected channel all the same
    /// ([`App::live_target`]).
    ///
    /// [`select_channel`]: Self::select_channel
    fn press_channel(&mut self, channel: ChannelId) {
        let result = self
            .end_library_preview()
            .and(self.change_selected_channel(Some(channel)));
        self.report(result);
    }

    /// An open instrument window shows `channel` from now on, as a click on its name would take
    /// it there; nothing while the window is closed or on it already.
    fn instrument_follows(&mut self, channel: ChannelId) {
        if self.instrument_on().is_some() {
            self.open_instrument_chosen(channel);
        }
    }

    /// Selects `channel`, or none, as [`select_channel`](Self::select_channel) says; returns what
    /// moving the live notes there said.
    fn change_selected_channel(&mut self, channel: Option<ChannelId>) -> Result<(), SessionError> {
        self.selected_channel = channel;
        self.rack_cache.clear();
        self.show_roll();
        self.select_channel_insert();
        // The ports no link routes play the instrument window's channel only while it is the
        // selected one.
        self.route_ports();
        // Record → Input records the selected input channel, else the default input.
        self.update_input();
        // The Library's device rows say where those ports' notes go: the selected channel.
        self.browse.cache.clear();
        // Live notes play the selected channel.
        self.session.set_live_channel(channel)
    }

    /// Keeps the selection on a channel that exists: when an edit or an undo took the selected
    /// one away (or a project with none gained one), the first is selected, so live notes never
    /// play a channel that is gone (which plays nothing). An open instrument window follows,
    /// unless its own channel went too: [`App::keep_instrument`] then closes it, as Esc does.
    fn keep_channel(&mut self) -> Result<(), SessionError> {
        let channels = &self.session.project().channels;
        if self
            .selected_channel
            .is_some_and(|id| channels.contains_key(&id))
        {
            return Ok(());
        }
        let first = channels.keys().next().copied();
        if first == self.selected_channel {
            return Ok(());
        }
        let window_stays = self
            .instrument_on()
            .is_some_and(|id| channels.contains_key(&id));
        let result = self.change_selected_channel(first);
        if let Some(first) = first.filter(|_| window_stays) {
            self.instrument_follows(first);
        }
        result
    }

    /// Selects the selected channel's insert in the mixer, as selecting a channel does.
    fn select_channel_insert(&mut self) {
        let project = self.session.project();
        let insert = self
            .selected_channel
            .and_then(|id| project.channels.get(&id))
            .map(|channel| channel.insert);
        if let Some(insert) = insert {
            self.mixer.select(insert, project);
        }
    }

    /// Closes the menu open over a panel, if one is: a channel's, an automation menu, a
    /// playlist track's or its curve menu, a mixer slot's, a Library row's. Whether one was.
    fn close_panel_menu(&mut self) -> bool {
        let closed = [
            self.roll.menu.take().is_some(),
            self.quantize_form.take().is_some(),
            self.sampler_menu.take().is_some(),
            self.automate.take().is_some(),
            self.arranger.track_menu.take().is_some(),
            self.arranger.curve_menu.take().is_some(),
            self.arranger.clip_menu.take().is_some(),
            self.mixer.menu.take().is_some(),
            self.browse.menu.take().is_some(),
        ];
        closed.contains(&true)
    }

    /// Selects mixer insert `insert` and opens the menu of its slot `slot`, as a press on that
    /// slot does: the add menu under an empty slot (a left press), its effect's menu under a
    /// filled one (a right press). `--slot-menu`, for a screenshot of the menu. Whether it
    /// opened.
    pub fn open_slot_menu(&mut self, insert: dawsome_engine::project::InsertId, slot: u8) -> bool {
        let project = self.session.project();
        let Some(shown) = project.mixer.get(insert) else {
            return false;
        };
        let button = if shown.effects.iter().any(|effect| effect.slot == slot) {
            mouse::Button::Right
        } else {
            mouse::Button::Left
        };
        self.mixer.select(insert, project);
        let at = mixer_layout::slot_rect(slot, self.mixer.size().width).center();
        let pressed = MixerMessage::Pressed {
            at,
            button,
            double: false,
        };
        let _ = self.mixer_message(pressed);
        self.mixer.menu.is_some()
    }

    /// Hands a mixer message to its state and carries out what that asks for.
    fn mixer_message(&mut self, message: MixerMessage) -> Task<Message> {
        // A new layout size is not a press: it must not take the keys.
        if !matches!(message, MixerMessage::Resized(_)) {
            self.focus = Panel::Mixer;
        }
        let context = Context {
            project: self.session.project(),
            registry: self.session.registry(),
            channel: self.selected_channel,
            modifiers: self.modifiers,
        };
        match self.mixer.update(message, &context) {
            Some(MixerCommand::Edit(edit)) => self.commit(edit),
            Some(MixerCommand::Drag(control)) => self.start_drag(DragTarget::Mixer(control)),
            Some(MixerCommand::FocusRename) => {
                let id = Id::new(MIXER_RENAME_INPUT);
                return operation::focus(id.clone()).chain(operation::select_all(id));
            }
            Some(MixerCommand::PickPlugin { insert, slot }) => {
                return self.open_picker(PickerTarget::Effect { insert, slot });
            }
            Some(MixerCommand::OpenPlugin(effect)) => {
                self.open_plugin(ProcessorOwner::Effect(effect));
            }
            Some(MixerCommand::PluginParams(effect)) => {
                self.open_panel(ProcessorOwner::Effect(effect));
            }
            Some(MixerCommand::Automate { target, value, at }) => {
                self.open_automate_menu(target, value, Panel::Mixer, at);
            }
            Some(MixerCommand::AutomatePlugin { effect, index, at }) => {
                let owner = ProcessorOwner::Effect(effect);
                self.open_plugin_automate_menu((owner, index), Panel::Mixer, false, at);
            }
            Some(MixerCommand::DragPlugin { effect, index }) => {
                if let Some(plugin) = self.plugin_drag(ProcessorOwner::Effect(effect), index) {
                    self.start_drag(DragTarget::Plugin(plugin));
                }
            }
            None => {}
        }
        Task::none()
    }

    /// A channel menu's row, or the rename box's typing: Rename opens the box over the name,
    /// Enter renames (one undo step), Clone adds a copy at the end of the rack (selected),
    /// Delete takes the channel and its notes out, One note at a time switches the channel's
    /// `cut_itself` (one undo step each).
    fn channel_command(&mut self, command: ChannelCommand) -> Task<Message> {
        self.focus = Panel::Rack;
        match command {
            ChannelCommand::Rename(channel) => {
                self.sampler_menu = None;
                let name = self.session.project().channels.get(&channel);
                if let Some(name) = name.map(|shown| shown.name.clone()) {
                    self.channel_rename = Some((channel, name));
                    let id = Id::new(CHANNEL_RENAME_INPUT);
                    return operation::focus(id.clone()).chain(operation::select_all(id));
                }
            }
            ChannelCommand::RenameEdited(typed) => {
                if let Some((_, name)) = &mut self.channel_rename {
                    *name = typed;
                }
            }
            ChannelCommand::RenameSubmitted => {
                if let Some((id, name)) = self.channel_rename.take() {
                    let name = name.trim().to_owned();
                    if !name.is_empty() {
                        self.commit(Edit::RenameChannel { id, name });
                    }
                }
            }
            ChannelCommand::Clone(from) => {
                self.sampler_menu = None;
                let id = self.session.project().next_channel_id;
                self.commit(Edit::CloneChannel { id, from });
                if self.error.is_none() {
                    self.select_channel(id);
                }
            }
            ChannelCommand::Delete(id) => {
                self.sampler_menu = None;
                self.commit(Edit::DeleteChannel { id });
            }
            ChannelCommand::OneNoteAtATime(channel) => {
                self.sampler_menu = None;
                // A sound setting, so a sampler's or a set's alone (the kinds the menu shows
                // the row for; the project refuses it on the others).
                let shown = self.session.project().channels.get(&channel);
                if let Some(shown) = shown.filter(|shown| {
                    crate::ui::hint::ChannelRow::OneNoteAtATime.shown_for(shown.kind)
                }) {
                    let sound = dawsome_engine::project::Sound {
                        cut_itself: !shown.sound.cut_itself,
                        ..shown.sound
                    };
                    self.commit(Edit::SetChannelSound { channel, sound });
                }
            }
            ChannelCommand::MixerInsert(channel) => {
                self.sampler_menu = None;
                return self.give_mixer_insert(channel);
            }
        }
        Task::none()
    }

    /// "Mixer insert…" on `channel` (its menu's row, its instrument window's Out): it plays into
    /// an insert of its own ([`mixer_state::own_insert`]: the first free one, one undo step, when
    /// it plays into the master or shares its insert), selected with the channel as a click on
    /// its name selects it, and the mixer opens there. With every insert taken, a notice and
    /// nothing changed.
    pub(super) fn give_mixer_insert(&mut self, channel: ChannelId) -> Task<Message> {
        use crate::ui::mixer_state::{own_insert, OwnInsert};
        match own_insert(self.session.project(), channel) {
            None => return Task::none(),
            Some(OwnInsert::NoneFree) => {
                self.notice =
                    Some("Every insert is in use: free one in the mixer first".to_owned());
                return Task::none();
            }
            Some(OwnInsert::Free(insert)) => {
                self.commit(Edit::RouteChannel { channel, insert });
                if self.error.is_some() {
                    return Task::none();
                }
            }
            Some(OwnInsert::Own(_)) => {}
        }
        self.select_channel(channel);
        self.show_panel(Panel::Mixer)
    }

    /// Opens `panel` as its key does while it is closed; an open one stays where it is laid out.
    /// Either way it takes the keys.
    fn show_panel(&mut self, panel: Panel) -> Task<Message> {
        if self.panels.is_open(panel) {
            self.focus = panel;
            return Task::none();
        }
        self.update(Message::TogglePanel(panel))
    }

    /// The rename box over the name of the channel being renamed.
    fn channel_rename_view(&self) -> Option<Element<'_, Message>> {
        let (id, typed) = self.channel_rename.as_ref()?;
        let row = self.rack_rows().row_of(*id)?;
        let rect = layout::name_button_rect(row);
        let input = iced::widget::text_input("", typed)
            .id(Id::new(CHANNEL_RENAME_INPUT))
            .on_input(|typed| Message::Channel(ChannelCommand::RenameEdited(typed)))
            .on_submit(Message::Channel(ChannelCommand::RenameSubmitted))
            .size(13)
            .padding([4, 6])
            .width(rect.width + 60.0);
        Some(pin(input).x(rect.x).y(rect.y).into())
    }

    /// Whether the hint bar shows: as `--hints` says, else as the settings ask, never in a
    /// scripted run.
    fn shows_hints(&self) -> bool {
        self.hints_flag
            .unwrap_or(self.host.hint_bar && self.settings.show_hints())
    }

    /// The panels' height: the window under the toolbar, above the hint bar while it shows.
    fn panels_height(&self) -> f32 {
        self.window_size.height - layout::TOOLBAR_HEIGHT - self.hint_bar_height()
    }

    /// The hint bar's height at the window's foot: none while it is hidden. The menus over the
    /// panels keep clear of it ([`crate::ui::menu::floating`]).
    pub(super) fn hint_bar_height(&self) -> f32 {
        if self.shows_hints() {
            layout::HINT_BAR_HEIGHT
        } else {
            0.0
        }
    }

    /// What the hint bar's words read from the window.
    fn hint_context(&self) -> crate::ui::hint::Context<'_> {
        let project = self.session.project();
        crate::ui::hint::Context {
            project,
            pattern: self.pattern,
            insert: self.mixer.selected,
            loop_on: project.playlist.loop_range.map(|range| range.on),
            preview: self.settings.preview_notes(),
            octave: typing::octave_name(self.typing.base),
            keyboard: self.keyboard_words(),
            keys: &self.keys,
            notices: self.toolbar().notice_text(),
            resampling: self.audio_note.as_deref(),
            takes_open: self.takes_open,
            link: self.selection_link(),
            latencies: self.plugin_latencies(),
        }
    }

    /// How late each of the document's plugins that says so plays its sound, by `ProcessorOwner`.
    fn plugin_latencies(&self) -> std::collections::BTreeMap<ProcessorOwner, u32> {
        let project = self.session.project();
        let channels = project
            .channels
            .iter()
            .filter(|(_, channel)| channel.processor.is_some())
            .map(|(&id, _)| ProcessorOwner::Channel(id));
        let effects = project
            .mixer
            .inserts
            .iter()
            .flat_map(|insert| &insert.effects)
            .map(|effect| ProcessorOwner::Effect(effect.id));
        channels
            .chain(effects)
            .filter_map(|owner| {
                let frames = plugins::host_of(&self.session, owner)?.latency(owner)?;
                (frames > 0).then_some((owner, frames))
            })
            .collect()
    }

    /// The toolbar switch's value in the hint bar: off, or which key plays which note and the
    /// first shortcut the keyboard takes ("A is C5, velocity 100; L plays D6 now; Shift+L
    /// switches pattern/song").
    fn keyboard_words(&self) -> String {
        if !self.typing.on {
            return "off".to_owned();
        }
        let value = self.typing.value_words();
        match self.typing.taken_example(&self.keys) {
            Some(taken) => format!("{value}; {taken}"),
            None => value,
        }
    }

    /// The hint bar: what the cursor is over, its value and what it does, on one line cut with
    /// an ellipsis; with nothing under the cursor, that the typing keyboard is on while it is.
    fn hint_bar(&self) -> Element<'_, Message> {
        let words = self.hint_words();
        let line = responsive(move |size| {
            let shown = crate::ui::text_fit::fit_width(&words, size.width, HINT_TEXT_SIZE)
                .map(std::borrow::Cow::into_owned)
                .unwrap_or_default();
            container(
                text(shown)
                    .size(HINT_TEXT_SIZE)
                    .color(style::TEXT_DIM)
                    .wrapping(iced::widget::text::Wrapping::None),
            )
            .height(Fill)
            .align_y(iced::Center)
            .into()
        });
        container(line)
            .padding([0.0, layout::TOOLBAR_SIDE_PADDING])
            .width(Fill)
            .height(layout::HINT_BAR_HEIGHT)
            .style(style::panel)
            .into()
    }

    /// The hint bar's words now. The instrument window's and the Settings window's controls are
    /// named on their own foot lines.
    fn hint_words(&self) -> String {
        match &self.hover {
            Some(hover)
                if !matches!(
                    hover,
                    crate::ui::hint::Hover::Sound(_) | crate::ui::hint::Hover::SampleEditor(_)
                ) && !hover.in_settings() =>
            {
                crate::ui::hint::words(hover, &self.hint_context())
            }
            _ if self.browse.model.playing.is_some() => library_preview::preview_hint(&self.keys),
            _ if self.typing.on => self.typing.summary(&self.keys),
            _ => String::new(),
        }
    }

    /// The window opened maximized (or not), its size under that the one it booted with.
    pub fn opened_maximized(&mut self, maximized: bool) {
        self.maximized = maximized;
    }

    /// The window's layout now, as the settings keep it for the next start: the panels open,
    /// the size when not maximized, and whether it is.
    fn window_layout(&self) -> crate::settings::WindowLayout {
        crate::settings::WindowLayout::of(self.panels, self.restored_size, self.maximized)
    }

    /// Keeps the window's layout in the settings for the next start, and the pop-out windows'
    /// places when they are open as the app closes; a run that saves no settings (a scripted
    /// one) keeps nothing.
    fn save_window_layout(&mut self) {
        let layout = Some(self.window_layout());
        let place = match self.instrument_window {
            Some(_) => Some(self.instrument_place),
            None => self.settings.instrument_window,
        };
        let settings_place = match self.settings_window {
            Some(_) => Some(self.settings_place),
            None => self.settings.settings_window,
        };
        let help_place = match self.help.window {
            Some(_) => Some(self.help.place),
            None => self.settings.help_window,
        };
        let params_place = match self.params.window {
            Some(_) => Some(self.params.place),
            None => self.settings.params_window,
        };
        let editor_place = match self.editor.window {
            Some(_) => Some(self.editor.place),
            None => self.settings.sample_editor_window,
        };
        if self.settings.window == layout
            && self.settings.instrument_window == place
            && self.settings.settings_window == settings_place
            && self.settings.help_window == help_place
            && self.settings.params_window == params_place
            && self.settings.sample_editor_window == editor_place
        {
            return;
        }
        self.settings.window = layout;
        self.settings.instrument_window = place;
        self.settings.settings_window = settings_place;
        self.settings.help_window = help_place;
        self.settings.params_window = params_place;
        self.settings.sample_editor_window = editor_place;
        self.save_settings();
    }

    /// Shows why an action failed, or clears the last failure once one succeeds.
    fn report(&mut self, result: Result<(), SessionError>) {
        self.error = result.err().map(|error| error.to_string());
    }

    /// The patterns before and after the selected one, in ID order.
    fn neighboring_patterns(&self) -> (Option<PatternId>, Option<PatternId>) {
        let Some(current) = self.pattern else {
            return (None, None);
        };
        let patterns = &self.session.project().patterns;
        let previous = patterns.range(..current).next_back();
        let next = patterns
            .range((Bound::Excluded(current), Bound::Unbounded))
            .next();
        (previous.map(|(&id, _)| id), next.map(|(&id, _)| id))
    }
}

/// Whether `message` is something done in the window (a press, a key), which closes the
/// automation and sampler menus, rather than a timer, a layout report or background work. A
/// key let go of is none: every release reaches the keyboard whatever has the keys, the release
/// of a key typed into a text box too, and must leave the box open.
fn closes_menus(message: &Message) -> bool {
    !matches!(
        message,
        Message::Tick
            | Message::AudioCheck
            | Message::RetryAudio
            | Message::Headless(_)
            | Message::PluginTick
            | Message::PanelTick
            // A scroll of the parameter window's list is its own: it closes the menu over a
            // row there (`App::param_message`) and nothing in the main window.
            | Message::Params(ParamMessage::Scrolled(_))
            | Message::SampleEditor(
                SampleEditorMessage::Tick
                    | SampleEditorMessage::GainMoved(_)
                    | SampleEditorMessage::Saved(_)
                    | SampleEditorMessage::Copied { .. }
            )
            | Message::ScaleFactor(_)
            | Message::MainNative(_)
            | Message::MonitorSized(_)
            | Message::WindowResized(_)
            | Message::WindowMaximized(_)
            | Message::ModifiersChanged(_)
            | Message::KnobMenu { .. }
            | Message::AutomateChosen
            | Message::LinkChosen
            | Message::ControllerTick
            | Message::PortsTick
            | Message::NotePreviewTick
            | Message::Hover(_)
            | Message::Media(_)
            | Message::Roll(RollMessage::GridResized(_))
            | Message::Playlist(
                PlaylistMessage::TracksResized(_)
                    | PlaylistMessage::Placed(_)
                    | PlaylistMessage::BrowserHover(_)
            )
            | Message::Mixer(MixerMessage::Resized(_))
            | Message::RackResized(_)
            | Message::Browser(BrowserMessage::Moved(_))
            | Message::WindowUnfocused
            | Message::OutputsListed(_)
            | Message::InputsListed(_)
            | Message::Callout(CalloutMessage::Due)
            | Message::TypingKey { pressed: false, .. }
    )
}

/// The names of `panels` as a notice says them: "Playlist and Mixer".
fn titles(panels: &[Panel]) -> String {
    let names: Vec<&str> = panels.iter().map(|panel| panel.title()).collect();
    names.join(" and ")
}

/// A panel's recessed background, full width.
fn well<'a>(content: Element<'a, Message>) -> container::Container<'a, Message> {
    container(content).width(Fill).style(style::rack_well)
}

/// The panel that has the editing keys when the window opens: the piano roll when it is open,
/// else the playlist when that is, else the rack.
fn first_focus(panels: Panels) -> Panel {
    [Panel::PianoRoll, Panel::Playlist]
        .into_iter()
        .find(|&panel| panels.is_open(panel))
        .unwrap_or(Panel::Rack)
}

/// `tempo` moved by `steps` of `step` milli-BPM, kept inside the accepted range.
fn nudged_tempo(tempo: MilliBpm, steps: i64, step: u32) -> MilliBpm {
    let moved = i64::from(tempo.0).saturating_add(steps.saturating_mul(i64::from(step)));
    let clamped = moved.clamp(
        i64::from(*TEMPO_RANGE.start()),
        i64::from(*TEMPO_RANGE.end()),
    );
    // The clamp keeps it inside the u32 tempo range.
    MilliBpm(u32::try_from(clamped).unwrap_or(*TEMPO_RANGE.start()))
}

/// What a key press runs with the default keys and the computer keyboard off: the shortcuts'
/// known answers, through the table the window runs ([`key_input::bound_message`]).
#[cfg(test)]
pub(crate) fn shortcut(key: Key<&str>, modifiers: Modifiers, repeat: bool) -> Option<Message> {
    key_input::bound_message(
        &crate::keys::Keys::default(),
        key,
        modifiers,
        repeat,
        crate::keys::Scope::Window,
    )
}

/// A new scale factor, when a window moves to a screen with another one, a window's new size or
/// place, files dropped on a window from the desktop (unless a sampler row took the file), and
/// a window losing the keyboard.
fn rescale_message(event: Event, status: event::Status, window: window::Id) -> Option<Message> {
    let changed = |change| Some(Message::Windowed(window, change));
    match event {
        Event::Window(window::Event::Rescaled(factor)) => changed(WindowChange::Rescaled(factor)),
        Event::Window(window::Event::Resized(size)) => changed(WindowChange::Resized(size)),
        Event::Window(window::Event::Moved(at)) => changed(WindowChange::Moved(at)),
        Event::Window(window::Event::Opened { position, size }) => {
            changed(WindowChange::Opened { size, position })
        }
        Event::Window(window::Event::FileDropped(path)) if status == event::Status::Ignored => {
            Some(Message::FileDroppedOn(window, path))
        }
        Event::Window(window::Event::Focused) => changed(WindowChange::Focus(true)),
        Event::Window(window::Event::Unfocused) => changed(WindowChange::Focus(false)),
        _ => None,
    }
}

/// Cursor moves and the button release, while the sample editor's Gain readout is held.
fn editor_gain_message(
    event: Event,
    _status: event::Status,
    _window: window::Id,
) -> Option<Message> {
    let message = match event {
        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            SampleEditorMessage::GainMoved(position.y)
        }
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
            SampleEditorMessage::GainReleased
        }
        _ => return None,
    };
    Some(Message::SampleEditor(message))
}

/// Cursor moves and the button release, while an up-and-down drag is held.
fn drag_message(event: Event, _status: event::Status, _window: window::Id) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            Some(Message::DragMoved(position.y))
        }
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(Message::DragEnded),
        _ => None,
    }
}

/// Cursor moves and the button release, while a slider drag is held.
fn slider_drag_message(
    event: Event,
    _status: event::Status,
    _window: window::Id,
) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            Some(Message::DragMoved(position.x))
        }
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(Message::DragEnded),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use dawsome_engine::project::Project;
    use dawsome_engine::realtime::Player;

    use std::collections::BTreeSet;

    use dawsome_engine::project::ClipId;

    use super::*;
    use crate::project_file;
    use crate::ui::clip_gesture::ClipSnap;
    use crate::ui::step_grid::StepGrid;
    use dawsome_engine::project::{ChannelKind, TrackId};

    const F1: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/f1_four_on_floor.json"
    );

    /// An app on `path` with no audio device. The returned player stands in for the audio
    /// callback: a test calls `process` where the device would.
    pub(super) fn app_on(path: &Path) -> (App, Player) {
        let project = project_file::open(path).expect("project opens").project;
        app_with(project, path)
    }

    fn app_with(project: Project, path: &Path) -> (App, Player) {
        app_hosted(project, path, Host::offline())
    }

    /// As [`app_with`], with `host` saying what the app may touch outside its window and how
    /// the session plays its files.
    pub(super) fn app_hosted(project: Project, path: &Path, host: Host) -> (App, Player) {
        let mode = Session::mode_for(&project, false);
        let (session, player) = Session::open_playing(
            project,
            project_file::samples_dir(path),
            mode,
            dawsome_engine::processor::Registry::native(),
            host.playback.clone(),
        )
        .expect("project compiles");
        let document = if path.as_os_str().is_empty() {
            Document::untitled()
        } else {
            Document::saved_at(path)
        };
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
        (app, player)
    }

    /// The computer keyboard as it was before the piano row became the default: the two rows,
    /// off until switched on. The typing tests written then start from it.
    pub(super) fn legacy_keyboard(app: &mut App) {
        app.typing = typing::Typing::new(false, crate::keys::NoteLayout::TwoRows);
    }

    /// The channels live notes play now, once the player has taken every plan waiting: a note
    /// from a MIDI device on no link (so the selected channel's), then one from the typing
    /// keyboard, in the order the player starts them (the keyboard's controls first).
    pub(super) fn live_note_channels(app: &mut App, player: &mut Player) -> Vec<ChannelId> {
        // The player takes one waiting plan per call, as the stream's calls do.
        for _ in 0..16 {
            player.process(&mut [[0.0; 2]; 64]);
        }
        let _ = app.session.take_events();
        let device = crate::midi::Router::new(
            app.session.midi_inbox(),
            crate::midi::ControllerQueue::default(),
        );
        device.deliver(&[0x90, 60, 64]);
        device.deliver(&[0x80, 60, 0]);
        let typing = app.typing.on;
        if !typing {
            let _ = app.update(Message::ToggleTyping);
        }
        // The layout's first key: a note in each.
        let first = app.typing.layout.first_key();
        for pressed in [true, false] {
            let _ = app.update(Message::TypingKey {
                code: first,
                pressed,
                repeat: false,
            });
        }
        player.process(&mut [[0.0; 2]; 64]);
        if !typing {
            let _ = app.update(Message::ToggleTyping);
        }
        app.session
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

    // A channel removed while selected (here the newest, a sampler just added and selected)
    // leaves the first channel selected: the rack, the piano roll, the mixer and live notes,
    // from a device and the typing keyboard, all follow it, as a click on its name does.
    #[test]
    fn removing_the_selected_channel_selects_the_first() {
        let (mut app, mut player) = f1_app();
        let id = app.session.project().next_channel_id;
        app.commit(Edit::AddSamplerChannel {
            id,
            name: "added".to_owned(),
            sample: "impulse.wav".into(),
        });
        let _ = app.update(Message::ChannelSelected(id));
        assert_eq!(live_note_channels(&mut app, &mut player), [id, id]);
        app.commit(Edit::RemoveChannel { id });
        assert_eq!(app.error, None);
        assert_eq!(app.selected_channel, Some(ChannelId(1)));
        assert_eq!(
            live_note_channels(&mut app, &mut player),
            [ChannelId(1), ChannelId(1)]
        );
    }

    fn f1_app() -> (App, Player) {
        app_on(Path::new(F1))
    }

    fn lit_steps(app: &App, channel: ChannelId) -> Vec<u32> {
        let grid = StepGrid::new(app.session.project(), app.pattern);
        (0..grid.steps())
            .filter(|&s| grid.is_lit(channel, s))
            .collect()
    }

    fn volume(app: &App) -> f32 {
        app.session.project().channels[&ChannelId(1)].volume
    }

    /// The notices the toolbar shows, left to right.
    fn notices(app: &App) -> Vec<&str> {
        [app.error.as_deref(), app.audio_problem.as_deref()]
            .into_iter()
            .flatten()
            .collect()
    }

    /// One device buffer: the player renders, then the playhead timer fires.
    fn buffer(app: &mut App, player: &mut Player) {
        player.process(&mut [[0.0; 2]; 512]);
        let _ = app.update(Message::Tick);
    }

    #[test]
    fn boot_selects_the_first_pattern_and_channel_and_names_missing_audio() {
        let (app, _player) = f1_app();
        assert_eq!(app.pattern, Some(PatternId(1)));
        assert_eq!(app.selected_channel, Some(ChannelId(1)));
        assert!(!app.playing);
        assert_eq!(app.mode, PlayMode::Pattern);
        assert_eq!(notices(&app), ["Audio off: no audio output device"]);
    }

    // A stream that failed never calls its player again, so the output goes and its player is
    // parked: it takes each plan as it comes, so more step clicks than an idle player could
    // hold plans for (72) all go through. A dropout plays on, so its output stays.
    #[test]
    fn a_failed_stream_is_dropped_and_a_dropout_is_not() {
        let (mut app, player) = f1_app();
        app.audio = Some(Output::holding(player));
        let report = |app: &App, kind| app.audio.as_ref().expect("an output").report(kind);
        report(&app, cpal::ErrorKind::Xrun);
        let _ = app.update(Message::AudioCheck);
        assert!(app.audio.is_some());
        assert_eq!(notices(&app), ["Audio dropped out briefly and played on"]);
        report(&app, cpal::ErrorKind::DeviceNotAvailable);
        let _ = app.update(Message::AudioCheck);
        assert!(app.audio.is_none());
        assert_eq!(notices(&app), ["Audio stopped: the output device failed"]);
        for _ in 0..100 {
            let _ = app.update(Message::StepClicked {
                channel: ChannelId(1),
                step: 2,
            });
        }
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    #[test]
    fn a_step_click_edits_the_document_and_undo_redo_walk_it() {
        let (mut app, _player) = f1_app();
        let original = app.session.project().clone();
        let _ = app.update(Message::StepClicked {
            channel: ChannelId(1),
            step: 2,
        });
        assert_eq!(lit_steps(&app, ChannelId(1)), [0, 2, 4, 8, 12]);
        let _ = app.update(Message::Undo);
        assert_eq!(app.session.project(), &original);
        let _ = app.update(Message::Redo);
        assert_eq!(lit_steps(&app, ChannelId(1)), [0, 2, 4, 8, 12]);
    }

    #[test]
    fn a_step_click_without_a_pattern_changes_nothing() {
        let no_pattern = Project::new(44_100, MilliBpm(120_000), 96);
        let (mut app, _player) = app_with(no_pattern, Path::new(""));
        let before = app.session.project().clone();
        let _ = app.update(Message::StepClicked {
            channel: ChannelId(0),
            step: 0,
        });
        assert_eq!(app.session.project(), &before);
        assert_eq!(notices(&app), ["Audio off: no audio output device"]);
    }

    #[test]
    fn play_and_stop_follow_the_player() {
        let (mut app, mut player) = f1_app();
        let _ = app.update(Message::TogglePlay);
        assert!(app.transport_on, "Space asks for play");
        assert!(!app.playing, "nothing has played yet");
        buffer(&mut app, &mut player);
        assert!(app.playing);
        // At 44.1 kHz and 120 BPM tick t starts at floor(t·229.6875), so after 512 frames the
        // playhead is in tick 2 (samples 459..689).
        assert_eq!(app.playhead, Some(Ticks(2)));
        let _ = app.update(Message::TogglePlay);
        buffer(&mut app, &mut player);
        assert!(!app.playing && !app.transport_on);
        assert_eq!(app.playhead, None);
    }

    #[test]
    fn a_knob_drag_applies_live_and_undoes_in_one_step() {
        let (mut app, _player) = f1_app();
        let _ = app.update(Message::KnobPressed {
            channel: ChannelId(1),
            knob: Knob::Volume,
        });
        // Measured from the first move: 50 pixels up is a quarter of the sweep, gain +1.0.
        let _ = app.update(Message::DragMoved(300.0));
        let _ = app.update(Message::DragMoved(250.0));
        assert_eq!(volume(&app), 2.0);
        let _ = app.update(Message::DragMoved(100_000.0));
        assert_eq!(volume(&app), 0.0, "pinned at the left stop");
        let _ = app.update(Message::DragEnded);
        let _ = app.update(Message::Undo);
        assert_eq!(volume(&app), 1.0);
        let _ = app.update(Message::Undo);
        assert_eq!(volume(&app), 1.0, "the drag was the only undo step");
    }

    #[test]
    fn the_tempo_scrolls_in_whole_or_fine_steps() {
        let (mut app, _player) = f1_app();
        let tempo = |app: &App| app.session.project().tempo;
        let up = mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 };
        let _ = app.update(Message::TempoScrolled(up));
        assert_eq!(tempo(&app), MilliBpm(121_000));
        let _ = app.update(Message::ModifiersChanged(Modifiers::SHIFT));
        let down = mouse::ScrollDelta::Pixels { x: 0.0, y: -3.0 };
        let _ = app.update(Message::TempoScrolled(down));
        assert_eq!(tempo(&app), MilliBpm(120_999));
        let _ = app.update(Message::Undo);
        let _ = app.update(Message::Undo);
        assert_eq!(tempo(&app), MilliBpm(120_000));
    }

    #[test]
    fn the_mute_light_toggles_mute() {
        let (mut app, _player) = f1_app();
        let muted = |app: &App| app.session.project().channels[&ChannelId(1)].mute;
        let _ = app.update(Message::MuteToggled(ChannelId(1)));
        assert!(muted(&app));
        let _ = app.update(Message::MuteToggled(ChannelId(1)));
        assert!(!muted(&app));
    }

    fn bars(app: &App) -> u32 {
        app.session.project().patterns[&PatternId(1)].length_bars
    }

    fn nudge_bars(app: &mut App, by: i32) {
        let _ = app.update(Message::Pattern(PatternMessage::Bars(by)));
    }

    // The rack header's "−" and "+" set the selected pattern's length a bar at a time, each one
    // undo step, from 1 bar to 32 (512 steps); the rack shows 16 steps a bar. The header shows
    // the count, and dims "−" at 1 bar and "+" at 32.
    #[test]
    fn the_rack_header_sets_the_pattern_length_in_bars() {
        let (mut app, _player) = f1_app();
        let header = |app: &App| {
            let header = app.rack_header();
            (header.bars, header.shorter, header.longer)
        };
        assert_eq!(header(&app), (Some(1), false, true));
        nudge_bars(&mut app, 1);
        assert_eq!(bars(&app), 2);
        assert_eq!(header(&app), (Some(2), true, true));
        assert_eq!(
            step_grid::step_count(app.session.project(), app.pattern),
            32
        );
        nudge_bars(&mut app, -1);
        nudge_bars(&mut app, -1);
        assert_eq!(bars(&app), 1, "a pattern keeps one bar");
        let _ = app.update(Message::Undo);
        assert_eq!(bars(&app), 2, "the refused \"−\" took no undo step");
        for _ in 0..40 {
            nudge_bars(&mut app, 1);
        }
        assert_eq!(bars(&app), 32);
        assert_eq!(header(&app), (Some(32), true, false));
        nudge_bars(&mut app, -1);
        assert_eq!(bars(&app), 31);
    }

    // "+ Sampler…" makes a sampler channel of the file picked, as a file dropped on the rack
    // does: at the end of the rack, named after the file, one undo step; a canceled pick
    // adds nothing.
    #[test]
    fn the_sampler_button_adds_a_channel_playing_the_file_picked() {
        let (mut app, _player) = f1_app();
        let file = Path::new(F1).with_file_name("hold2000.wav");
        let picked = |file| Message::Media(MediaMessage::SamplerPicked(file));
        let _ = app.update(picked(Some(file)));
        let added = &app.session.project().channels[&ChannelId(2)];
        assert_eq!(
            (added.name.as_str(), added.kind),
            ("hold2000", ChannelKind::Sampler)
        );
        assert_eq!(
            added.sample,
            Path::new("hold2000.wav"),
            "beside the project, so relative"
        );
        let _ = app.update(picked(None));
        assert_eq!(app.session.project().channels.len(), 2);
        let _ = app.update(Message::Undo);
        assert_eq!(app.session.project().channels.len(), 1);
    }

    // A right press on F1's kick opens its menu; "Change sample…" closes it and asks for a file
    // (no dialog in a test). The file picked becomes the kick's sound, named relative to the
    // project, one undo step; a canceled pick changes nothing.
    #[test]
    fn the_sampler_menu_gives_the_kick_the_file_picked() {
        let (mut app, _player) = f1_app();
        let before = document(&app);
        let at = iced::Point::new(60.0, 20.0);
        let _ = app.update(Message::Media(MediaMessage::SamplerMenu {
            channel: ChannelId(1),
            at,
        }));
        assert_eq!(app.sampler_menu, Some((ChannelId(1), at)));
        assert!(app.sampler_menu_view().is_some(), "the menu shows");
        let _ = app.update(Message::Media(MediaMessage::ChangeSample(ChannelId(1))));
        assert_eq!(app.sampler_menu, None, "closed by its row");
        let picked = |file| {
            Message::Media(MediaMessage::SampleChosen {
                channel: ChannelId(1),
                file,
            })
        };
        let _ = app.update(picked(None));
        assert_eq!(document(&app), before, "canceled");
        let _ = app.update(picked(Some(Path::new(F1).with_file_name("hold2000.wav"))));
        assert_eq!(
            document(&app),
            before.replace(r#""sample":"impulse.wav""#, r#""sample":"hold2000.wav""#)
        );
        assert_eq!(app.session.history().undo_len(), 1);
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), before);
    }

    // The sampler menu closes on anything else done in the window, as the automation menu does,
    // and opening either closes the other.
    #[test]
    fn the_sampler_menu_closes_on_anything_else() {
        let (mut app, _player) = f1_app();
        let menu = |app: &mut App| {
            let _ = app.update(Message::Media(MediaMessage::SamplerMenu {
                channel: ChannelId(1),
                at: iced::Point::new(60.0, 20.0),
            }));
        };
        menu(&mut app);
        let _ = app.update(Message::ClearSelection);
        assert_eq!(app.sampler_menu, None, "Esc");
        menu(&mut app);
        let _ = app.update(Message::KnobMenu {
            channel: ChannelId(1),
            knob: Knob::Volume,
            at: iced::Point::new(182.0, 30.0),
        });
        assert_eq!(app.sampler_menu, None, "the knob's menu instead");
        assert!(app.automate.is_some());
        menu(&mut app);
        assert!(app.automate.is_none(), "the sampler's menu instead");
    }

    // Asked for in use, 2026-10-26: the switch is the channel menu's
    // "One note at a time", shown on a sampler's and a set's menu (the kinds with sound
    // settings), after "Change sample…" and "Edit sample…", and not on an input channel's. A press switches the
    // channel's `cut_itself` in one undo step named as the other sound settings and closes the
    // menu; opened again, the row is ticked. The file then names "cut_itself":true last in the
    // sampler's sound at the version it had; pressed again it is off and the file as before;
    // undo takes each back. On an input channel the command changes nothing.
    #[test]
    fn the_channel_menu_switches_one_note_at_a_time() {
        use crate::ui::hint::ChannelRow as R;
        use dawsome_engine::project::{InputSource, Sound};
        let (mut app, _player) = f1_app();
        let set = app.session.project().next_channel_id;
        app.commit(Edit::AddInstrumentChannel {
            id: set,
            name: "Sine".to_owned(),
            instrument: std::path::PathBuf::from("p26_sine/sine.sfz"),
        });
        let input = app.session.project().next_channel_id;
        app.commit(Edit::AddInputChannel {
            id: input,
            name: "Line in".to_owned(),
            input: InputSource::default_device(),
        });
        let added = app.session.history().undo_len();
        assert_eq!(added, 2, "{:?}", app.error);
        let before = document(&app);
        let menu = |app: &mut App, channel| {
            let _ = app.update(Message::Media(MediaMessage::SamplerMenu {
                channel,
                at: iced::Point::new(60.0, 20.0),
            }));
            app.channel_menu_rows()
        };
        let sound = |app: &App, channel| app.session.project().channels[&channel].sound;
        assert_eq!(
            menu(&mut app, ChannelId(1)),
            [
                (R::InstrumentWindow, None),
                (R::MixerInsert, None),
                (R::ChangeSample, None),
                (R::EditSample, None),
                (R::OneNoteAtATime, Some(false)),
                (R::Rename, None),
                (R::Clone, None),
                (R::Delete, None)
            ],
            "a sampler's"
        );
        assert_eq!(
            menu(&mut app, set),
            [
                (R::InstrumentWindow, None),
                (R::MixerInsert, None),
                (R::OneNoteAtATime, Some(false)),
                (R::Rename, None),
                (R::Clone, None),
                (R::Delete, None)
            ],
            "a set's"
        );
        assert_eq!(
            menu(&mut app, input),
            [
                (R::InstrumentWindow, None),
                (R::MixerInsert, None),
                (R::Rename, None),
                (R::Clone, None),
                (R::Delete, None)
            ],
            "an input channel's"
        );
        let _ = app.update(Message::Channel(ChannelCommand::OneNoteAtATime(input)));
        assert_eq!(
            app.session.history().undo_len(),
            added,
            "nothing on an input"
        );
        assert_eq!(document(&app), before);
        assert_eq!(app.error, None);

        menu(&mut app, ChannelId(1));
        let _ = app.update(Message::Channel(ChannelCommand::OneNoteAtATime(ChannelId(
            1,
        ))));
        assert_eq!(app.sampler_menu, None, "closed by its row");
        assert!(sound(&app, ChannelId(1)).cut_itself);
        assert!(
            !sound(&app, ChannelId(1)).reverse,
            "the other settings kept"
        );
        assert_eq!(app.session.history().undo_len(), added + 1, "one step");
        assert_eq!(app.session.history().undo_name(), Some("Sound settings"));
        assert_eq!(
            menu(&mut app, ChannelId(1))[4],
            (R::OneNoteAtATime, Some(true)),
            "ticked"
        );
        let json = document(&app);
        assert_eq!(
            json.split(',').next(),
            before.split(',').next(),
            "a sound setting keeps the file's version (the input channel's 6 here)"
        );
        assert!(
            json.contains(r#""resonance":0.0,"cut_itself":true}"#),
            "{json}"
        );
        let _ = app.update(Message::Channel(ChannelCommand::OneNoteAtATime(ChannelId(
            1,
        ))));
        assert_eq!(sound(&app, ChannelId(1)), Sound::default());
        assert_eq!(app.session.history().undo_len(), added + 2);
        assert_eq!(
            menu(&mut app, ChannelId(1))[4],
            (R::OneNoteAtATime, Some(false))
        );
        let _ = app.update(Message::Undo);
        assert!(sound(&app, ChannelId(1)).cut_itself);
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), before);

        menu(&mut app, set);
        let _ = app.update(Message::Channel(ChannelCommand::OneNoteAtATime(set)));
        assert!(sound(&app, set).cut_itself, "a set's too");
        assert_eq!(menu(&mut app, set)[2], (R::OneNoteAtATime, Some(true)));
    }

    // The channel menu's "Mixer insert…", clicked through the widgets as its other rows are:
    // f1's kick (on the master of a new mixer) plays into Insert 1, the first free one, in one
    // undo step; the mixer opens with Insert 1 selected and the kick the selected channel. Undo
    // gives the document back. A snare added and given a strip the same way plays into Insert 2
    // (Insert 1 is the kick's; the header's + puts a channel on the master into Insert 1). The
    // row again on the kick, which has its own, changes nothing and opens the mixer on Insert 1.
    #[test]
    fn the_channel_menus_mixer_insert_gives_a_strip_of_its_own() {
        use crate::app::plugins::app_tests::channel_menu_row;
        use crate::ui::hint::ChannelRow;
        use dawsome_engine::project::InsertId;
        let (mut app, _player) = f1_app();
        let kick = ChannelId(1);
        let insert = |app: &App, channel| app.session.project().channels[&channel].insert;
        assert_eq!(insert(&app, kick), InsertId::MASTER);
        assert!(!app.panels.is_open(Panel::Mixer));
        let before = document(&app);
        let mixer_insert = |app: &mut App, channel| {
            let message = channel_menu_row(app, channel, ChannelRow::MixerInsert);
            assert!(
                matches!(message, Message::Channel(ChannelCommand::MixerInsert(c)) if c == channel),
                "{message:?}"
            );
            let _ = app.update(message);
            assert_eq!(app.sampler_menu, None, "closed by its row");
        };
        mixer_insert(&mut app, kick);
        assert_eq!(insert(&app, kick), InsertId(1));
        assert!(app.panels.is_open(Panel::Mixer));
        assert_eq!(app.focus, Panel::Mixer);
        assert_eq!(app.mixer.selected, InsertId(1));
        assert_eq!(app.selected_channel, Some(kick));
        assert_eq!(app.session.history().undo_len(), 1);
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), before);

        mixer_insert(&mut app, kick);
        let snare = app.session.project().next_channel_id;
        app.commit(Edit::AddSamplerChannel {
            id: snare,
            name: "Snare".to_owned(),
            sample: std::path::PathBuf::from("snare.wav"),
        });
        assert_eq!(insert(&app, snare), InsertId::MASTER);
        let _ = app.update(Message::TogglePanel(Panel::Mixer));
        assert!(!app.panels.is_open(Panel::Mixer));
        mixer_insert(&mut app, snare);
        assert_eq!(insert(&app, snare), InsertId(2));
        assert_eq!(app.mixer.selected, InsertId(2));
        assert_eq!(app.selected_channel, Some(snare));
        assert!(app.panels.is_open(Panel::Mixer));
        let steps = app.session.history().undo_len();
        mixer_insert(&mut app, kick);
        assert_eq!(app.session.history().undo_len(), steps, "its own already");
        assert_eq!(insert(&app, kick), InsertId(1));
        assert_eq!(app.mixer.selected, InsertId(1));
        assert_eq!(app.selected_channel, Some(kick));
        assert!(app.panels.is_open(Panel::Mixer), "kept open");
    }

    #[test]
    fn tempo_nudges_stay_in_range() {
        assert_eq!(nudged_tempo(MilliBpm(120_000), 1, 1000), MilliBpm(121_000));
        assert_eq!(nudged_tempo(MilliBpm(120_000), -3, 1), MilliBpm(119_997));
        assert_eq!(nudged_tempo(MilliBpm(521_500), 1, 1000), MilliBpm(522_000));
        assert_eq!(nudged_tempo(MilliBpm(10_500), -1, 1000), MilliBpm(10_000));
        assert_eq!(
            nudged_tempo(MilliBpm(120_000), i64::MIN, 1000),
            MilliBpm(10_000)
        );
    }

    #[test]
    fn keys_map_to_transport_and_history() {
        let space = Key::Named(key::Named::Space);
        assert!(matches!(
            shortcut(space.clone(), Modifiers::empty(), false),
            Some(Message::TogglePlay)
        ));
        assert!(shortcut(space, Modifiers::empty(), true).is_none());
        let z = Key::Character("z");
        assert!(matches!(
            shortcut(z.clone(), Modifiers::CTRL, false),
            Some(Message::Undo)
        ));
        assert!(matches!(
            shortcut(
                Key::Character("Z"),
                Modifiers::CTRL | Modifiers::SHIFT,
                false
            ),
            Some(Message::Redo)
        ));
        assert!(shortcut(z, Modifiers::empty(), false).is_none());
    }

    // Ctrl+Y redoes, as Ctrl+Shift+Z does; Y alone is no shortcut.
    #[test]
    fn ctrl_y_redoes_beside_ctrl_shift_z() {
        assert!(matches!(
            shortcut(Key::Character("y"), Modifiers::CTRL, false),
            Some(Message::Redo)
        ));
        assert!(matches!(
            shortcut(Key::Character("Y"), Modifiers::CTRL, true),
            Some(Message::Redo)
        ));
        assert!(shortcut(Key::Character("y"), Modifiers::empty(), false).is_none());
        let (mut app, _player) = f1_app();
        let original = app.session.project().clone();
        let _ = app.update(Message::StepClicked {
            channel: ChannelId(1),
            step: 2,
        });
        let stepped = app.session.project().clone();
        let _ = app.update(Message::Undo);
        assert_eq!(app.session.project(), &original);
        let redo = shortcut(Key::Character("y"), Modifiers::CTRL, false).expect("a shortcut");
        let _ = app.update(redo);
        assert_eq!(app.session.project(), &stepped);
    }

    /// What a key press makes while the typing keyboard these tests were written for (the two
    /// rows) is on.
    fn typed_press(key: Key, code: key::Code, modifiers: Modifiers) -> Option<Message> {
        let (mut app, _player) = f1_app();
        legacy_keyboard(&mut app);
        let _ = app.update(Message::ToggleTyping);
        app.key_message(KeyInput {
            window: window::Id::unique(),
            key,
            code: Some(code),
            modifiers,
            pressed: true,
            repeat: false,
        })
    }

    // While the typing keyboard is on, a key is its key by its place, whatever it types; Space,
    // the function keys and Ctrl chords stay shortcuts, and the R of Record is a note.
    #[test]
    fn the_typing_keyboard_takes_single_keys_and_leaves_chords() {
        let character = |text: &str| Key::Character(text.into());
        let none = Modifiers::empty();
        assert!(matches!(
            typed_press(character("y"), key::Code::KeyZ, none),
            Some(Message::TypingKey {
                code: key::Code::KeyZ,
                pressed: true,
                repeat: false
            })
        ));
        assert!(matches!(
            typed_press(character("r"), key::Code::KeyR, none),
            Some(Message::TypingKey { .. })
        ));
        assert!(matches!(
            typed_press(character("z"), key::Code::KeyZ, Modifiers::CTRL),
            Some(Message::Undo)
        ));
        assert!(matches!(
            typed_press(character("t"), key::Code::KeyT, Modifiers::CTRL),
            Some(Message::ToggleTyping)
        ));
        assert!(matches!(
            typed_press(Key::Named(key::Named::Space), key::Code::Space, none),
            Some(Message::TogglePlay)
        ));
        assert!(matches!(
            typed_press(Key::Named(key::Named::F9), key::Code::F9, none),
            Some(Message::TogglePanel(Panel::Mixer))
        ));
        assert!(matches!(
            shortcut(Key::Character("t"), Modifiers::CTRL, false),
            Some(Message::ToggleTyping)
        ));
    }

    // While the typing keyboard is on, single-key shortcuts are off. Each key below is a shortcut
    // while it is off (L the mode, R Record, Delete, Enter, an arrow); on, each goes to the typing
    // keyboard, which plays a note on the mapped ones and swallows the rest.
    #[test]
    fn single_key_shortcuts_are_off_while_the_typing_keyboard_is_on() {
        let character = |text: &str| Key::Character(text.into());
        let none = Modifiers::empty();
        for (key, code) in [
            (character("l"), key::Code::KeyL),
            (character("r"), key::Code::KeyR),
            (character("2"), key::Code::Digit2),
            (Key::Named(key::Named::Delete), key::Code::Delete),
            (Key::Named(key::Named::Enter), key::Code::Enter),
            (Key::Named(key::Named::ArrowDown), key::Code::ArrowDown),
        ] {
            assert!(
                shortcut(key.as_ref(), none, false).is_some(),
                "{key:?} is a shortcut with the typing keyboard off"
            );
            assert!(
                matches!(
                    typed_press(key.clone(), code, none),
                    Some(Message::TypingKey { code: typed, pressed: true, .. }) if typed == code
                ),
                "{key:?} goes to the typing keyboard while it is on"
            );
        }
    }

    #[test]
    fn only_patterns_in_the_project_can_be_selected() {
        let (mut app, _player) = f1_app();
        let selected = app.pattern;
        let _ = app.update(Message::PatternSelected(PatternId(999)));
        assert_eq!(app.pattern, selected);
        assert_eq!(app.neighboring_patterns(), (None, None));
    }

    #[test]
    fn the_mode_toggle_switches_between_pattern_and_song() {
        let (mut app, _player) = f1_app();
        let _ = app.update(Message::ModeSelected(PlayMode::Song));
        assert_eq!(app.mode, PlayMode::Song);
        let _ = app.update(Message::ModeSelected(PlayMode::Pattern));
        assert_eq!(app.mode, PlayMode::Pattern);
        assert_eq!(notices(&app), ["Audio off: no audio output device"]);
    }

    /// A click whose one move only sets the anchor, and a drag that comes back to where it
    /// started, change no value, so neither may take an undo step or empty the redo stack.
    #[test]
    fn a_drag_that_changes_nothing_keeps_the_history() {
        let knob = Message::KnobPressed {
            channel: ChannelId(1),
            knob: Knob::Volume,
        };
        let gestures: [(Message, &[f32]); 4] = [
            (knob.clone(), &[300.0]),
            (knob, &[300.0, 250.0, 300.0]),
            (Message::TempoPressed, &[300.0]),
            (Message::TempoPressed, &[300.0, 250.0, 300.0]),
        ];
        for (press, moves) in gestures {
            let (mut app, _player) = f1_app();
            let _ = app.update(Message::StepClicked {
                channel: ChannelId(1),
                step: 2,
            });
            let _ = app.update(Message::Undo);
            let before = app.session.project().clone();
            let undo_len = app.session.history().undo_len();
            let gesture = format!("{press:?}, moves {moves:?}");
            let _ = app.update(press);
            for &y in moves {
                let _ = app.update(Message::DragMoved(y));
            }
            let _ = app.update(Message::DragEnded);
            assert_eq!(app.session.project(), &before, "{gesture}");
            assert_eq!(app.session.history().undo_len(), undo_len, "{gesture}");
            assert_eq!(app.session.history().redo_len(), 1, "{gesture}");
            let _ = app.update(Message::Redo);
            assert_eq!(
                lit_steps(&app, ChannelId(1)),
                [0, 2, 4, 8, 12],
                "{gesture}: redo brings step 2 back"
            );
        }
    }

    // F1's pattern is one bar of 16 steps, so step 16 does not exist and the toggle is refused.
    #[test]
    fn a_failed_edit_shows_beside_the_audio_notice_until_an_edit_succeeds() {
        let (mut app, _player) = f1_app();
        let _ = app.update(Message::StepClicked {
            channel: ChannelId(1),
            step: 16,
        });
        let shown = notices(&app);
        assert!(
            shown
                .iter()
                .any(|notice| notice.starts_with("edit refused")),
            "{shown:?}"
        );
        assert!(
            shown.contains(&"Audio off: no audio output device"),
            "{shown:?}"
        );
        let _ = app.update(Message::StepClicked {
            channel: ChannelId(1),
            step: 2,
        });
        assert_eq!(notices(&app), ["Audio off: no audio output device"]);
    }

    // ---------------------------------------------------------------------------------------
    // Piano-roll known answers: gestures sent as the canvases send them, in canvas coordinates, at
    // the default zoom on p3_roll (top key 69: key 60's row is y 108..120, key 64's 60..72; A spans
    // x 48..80, B 112..176). Expected documents are the fixtures tests/fixtures/gen_p3_expected.py
    // wrote; the command line's known-answer tests render those same files.
    // ---------------------------------------------------------------------------------------

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    /// A fixture's text as `to_json` writes it (the files end with a newline).
    fn fixture(name: &str) -> String {
        let path = Path::new(FIXTURES).join(name);
        let text = std::fs::read_to_string(&path).expect("fixture reads");
        text.trim_end().to_owned()
    }

    fn document(app: &App) -> String {
        app.session.project().to_json().expect("serializes")
    }

    /// p3_roll with the piano roll opened by F7, channel 1 ("lead") selected.
    fn p3_app() -> (App, Player) {
        let (mut app, player) = app_on(&Path::new(FIXTURES).join("p3_roll.json"));
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        (app, player)
    }

    fn roll(app: &mut App, message: RollMessage) {
        let _ = app.update(Message::Roll(message));
    }

    fn at(x: f32, y: f32) -> iced::Point {
        iced::Point::new(x, y)
    }

    /// A left-button drag on the grid from `from` to `to`, through the point half way.
    fn grid_drag(app: &mut App, from: iced::Point, to: iced::Point) {
        roll(
            app,
            RollMessage::GridPressed {
                at: from,
                button: mouse::Button::Left,
                double: false,
            },
        );
        let half = at((from.x + to.x) / 2.0, (from.y + to.y) / 2.0);
        roll(app, RollMessage::GridDragged(half));
        roll(app, RollMessage::GridDragged(to));
        roll(app, RollMessage::GridReleased(to));
    }

    /// Checks `app` holds `expected`, that the gesture was one undo step, and that one undo
    /// gives p3_roll's own bytes back.
    fn one_undo_restores_p3_roll(app: &mut App, expected: &str) {
        assert_eq!(document(app), fixture(expected));
        assert_eq!(
            app.session.history().undo_len(),
            1,
            "{expected}: one undo step"
        );
        let _ = app.update(Message::Undo);
        assert_eq!(document(app), fixture("p3_roll.json"), "{expected}: undone");
    }

    #[test]
    fn f7_opens_the_piano_roll_on_the_selected_channel() {
        let (app, _player) = p3_app();
        assert!(app.panels.rack && app.panels.piano_roll);
        assert_eq!(app.selected_channel, Some(ChannelId(1)));
        assert_eq!(app.roll.view.top, 69, "5 keys above the highest note, 64");
        assert_eq!(document(&app), fixture("p3_roll.json"));
    }

    // (a) Press A's body at x 56 (tick 12), release at 76 (tick 42): raw start 30, snapped 24.
    #[test]
    fn p3_a_dragging_a_note_moves_it_to_the_nearest_step() {
        let (mut app, _player) = p3_app();
        grid_drag(&mut app, at(56.0, 114.0), at(76.0, 114.0));
        let moved = BTreeSet::from([(ChannelId(1), Ticks(24), 60)]);
        assert_eq!(app.roll.selection, moved, "the moved note stays selected");
        one_undo_restores_p3_roll(&mut app, "p3_a_move.json");
    }

    // (b) Press 2 px inside A's right edge, release at 112: raw end 99, snapped 96.
    #[test]
    fn p3_b_dragging_a_right_edge_resizes_the_note() {
        let (mut app, _player) = p3_app();
        grid_drag(&mut app, at(78.0, 114.0), at(112.0, 114.0));
        assert_eq!(
            app.roll.draw_length,
            Ticks(96),
            "a resize sets the draw length"
        );
        one_undo_restores_p3_roll(&mut app, "p3_b_resize.json");
    }

    // (c) Press A's stem in the lane at y 20 (value 88), release at y 32: velocity 64/128.
    // (Pins moved: the lane is 64 px by default, two velocity steps a pixel,
    // so y 40 and 64 of the 128 px lane are y 20 and 32 now; the answers are the same.)
    #[test]
    fn p3_c_dragging_a_stem_sets_the_velocity() {
        let (mut app, _player) = p3_app();
        roll(&mut app, RollMessage::LanePressed(at(50.0, 20.0)));
        assert_eq!(app.roll.dragged_velocity(), Some(88));
        roll(&mut app, RollMessage::LaneDragged(at(50.0, 35.0)));
        roll(&mut app, RollMessage::LaneReleased(at(50.0, 32.0)));
        assert_eq!(app.roll.dragged_velocity(), None);
        one_undo_restores_p3_roll(&mut app, "p3_c_velocity.json");
    }

    // (d) Ctrl+A, Ctrl+B: A and B span 0..192, so the copies land 192 later, and are the
    // selection. (d2) Ctrl+B again copies the copies to 384 and 480 and grows the pattern to 2
    // bars in the same step.
    #[test]
    fn p3_d_duplicating_copies_one_extent_later() {
        let (mut app, _player) = p3_app();
        let _ = app.update(Message::SelectAllNotes);
        let _ = app.update(Message::Duplicate);
        assert_eq!(document(&app), fixture("p3_d_duplicate.json"));
        let copies = BTreeSet::from([
            (ChannelId(1), Ticks(192), 60),
            (ChannelId(1), Ticks(288), 64),
        ]);
        assert_eq!(app.roll.selection, copies);
        let _ = app.update(Message::Duplicate);
        assert_eq!(document(&app), fixture("p3_d2_duplicate_again.json"));
        assert_eq!(app.session.history().undo_len(), 2);
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), fixture("p3_d_duplicate.json"));
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), fixture("p3_roll.json"));
    }

    // With nothing selected, Ctrl+B copies every note of the channel.
    #[test]
    fn p3_d_duplicating_with_nothing_selected_copies_the_whole_channel() {
        let (mut app, _player) = p3_app();
        assert!(app.roll.selection.is_empty());
        let _ = app.update(Message::Duplicate);
        assert_eq!(document(&app), fixture("p3_d_duplicate.json"));
    }

    // (e) A click on empty grid at x 330 (tick 423, snapped down to 408), key 60's row: a note
    // of the draw length (24) at velocity 100/128, and the pattern grows to 2 bars, one step.
    // The note is drawn as the press is let go of in place (a press there
    // that moves draws a box), so the press alone holds and changes nothing.
    #[test]
    fn p3_e_drawing_past_the_end_grows_the_pattern() {
        let (mut app, _player) = p3_app();
        roll(
            &mut app,
            RollMessage::GridPressed {
                at: at(330.0, 114.0),
                button: mouse::Button::Left,
                double: false,
            },
        );
        assert!(app.roll.grid_held(), "the press waits for its release");
        assert_eq!(document(&app), fixture("p3_roll.json"));
        roll(&mut app, RollMessage::GridReleased(at(331.0, 115.0)));
        assert!(
            app.roll.selection.is_empty(),
            "drawing clears the selection"
        );
        assert!(!app.roll.grid_held(), "a drawn note commits at the release");
        let steps = StepGrid::new(app.session.project(), app.pattern).steps();
        assert_eq!(steps, 32, "the rack shows the grown pattern");
        one_undo_restores_p3_roll(&mut app, "p3_e_grow.json");
    }

    // (f1) B dragged from (120, 66) onto A's tick and key: A is replaced; undo restores both.
    #[test]
    fn p3_f1_a_note_moved_onto_another_replaces_it() {
        let (mut app, _player) = p3_app();
        grid_drag(&mut app, at(120.0, 66.0), at(56.0, 114.0));
        one_undo_restores_p3_roll(&mut app, "p3_f1_replace.json");
    }

    // A right press on B opens the notes menu with B selected, and its Delete takes B out;
    // Ctrl+A and Delete take the rest; one step each, both undone to p3_roll's bytes.
    #[test]
    fn p3_delete_and_the_menus_delete_remove_notes_in_one_step_each() {
        let (mut app, _player) = p3_app();
        roll(
            &mut app,
            RollMessage::GridPressed {
                at: at(120.0, 66.0),
                button: mouse::Button::Right,
                double: false,
            },
        );
        assert!(app.roll.menu.is_some(), "the notes menu");
        let _ = app.update(Message::NoteEdit(crate::ui::roll::NoteEdit::Delete));
        assert_eq!(app.roll.menu, None, "a row closes it");
        let lead: Vec<i64> = app.session.project().patterns[&PatternId(1)]
            .notes
            .iter()
            .filter(|note| note.channel == ChannelId(1))
            .map(|note| note.start.0)
            .collect();
        assert_eq!(lead, [0], "B is gone");
        let _ = app.update(Message::SelectAllNotes);
        let _ = app.update(Message::Delete);
        assert!(app.roll.selection.is_empty());
        assert_eq!(app.session.history().undo_len(), 2);
        let _ = app.update(Message::Undo);
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), fixture("p3_roll.json"));
    }

    // With A and B selected, a right press on B keeps both selected, so the menu acts on both;
    // any other message (here Esc) closes the menu unused.
    #[test]
    fn p3_a_right_press_among_the_selection_keeps_it_for_the_menu() {
        let (mut app, _player) = p3_app();
        let _ = app.update(Message::SelectAllNotes);
        roll(
            &mut app,
            RollMessage::GridPressed {
                at: at(120.0, 66.0),
                button: mouse::Button::Right,
                double: false,
            },
        );
        assert_eq!(app.roll.selection.len(), 2);
        let _ = app.update(Message::ClearSelection);
        assert_eq!(app.roll.menu, None);
        assert_eq!(app.roll.selection.len(), 2, "Esc closed the menu first");
        assert_eq!(document(&app), fixture("p3_roll.json"));
    }

    // B clicked, then A's unselected stem dragged as in (c): A alone takes the value and
    // becomes the selection, as a press on an unselected note in the grid makes it. (Pins
    // moved with (c)'s: y 20 and 32 of the 64 px lane.)
    #[test]
    fn p3_dragging_an_unselected_stem_selects_its_note() {
        let (mut app, _player) = p3_app();
        grid_drag(&mut app, at(120.0, 66.0), at(120.0, 66.0));
        let b = BTreeSet::from([(ChannelId(1), Ticks(96), 64)]);
        assert_eq!(app.roll.selection, b);
        roll(&mut app, RollMessage::LanePressed(at(50.0, 20.0)));
        roll(&mut app, RollMessage::LaneReleased(at(50.0, 32.0)));
        assert_eq!(
            document(&app),
            fixture("p3_c_velocity.json"),
            "only A changed"
        );
        let a = BTreeSet::from([(ChannelId(1), Ticks(0), 60)]);
        assert_eq!(app.roll.selection, a);
    }

    // P2 through the playhead timer: p3_roll's one-bar pattern zoomed to 64 px a step shows 357
    // of its 384 ticks (952 px at 0.375 ticks a pixel). Playing, the view stays put until the
    // playhead passes tick 357, then jumps to start at it (a bar is wider than the view); when
    // the pattern wraps it jumps back to 0. With Follow switched off (written to the settings),
    // the view stays where it is.
    #[test]
    fn the_piano_roll_follows_the_playhead_a_page_at_a_time() {
        let (mut app, mut player) = p3_app();
        app.roll.view.zoom = 8;
        let _ = app.update(Message::Play);
        let mut jumped = None;
        let mut wrapped = false;
        for _ in 0..400 {
            buffer(&mut app, &mut player);
            let (Some(tick), scroll) = (app.playhead, app.roll.view.scroll) else {
                continue;
            };
            let at = tick.0.rem_euclid(384);
            match jumped {
                None if scroll != Ticks(0) => {
                    assert!((357..384).contains(&at), "jumped at tick {at}");
                    assert_eq!(scroll, Ticks(at), "the view starts at the playhead");
                    jumped = Some(scroll);
                }
                None => assert!(at < 357, "the playhead left the view at {at} unfollowed"),
                Some(_) if scroll == Ticks(0) => {
                    assert!(at < 357, "back to the pattern's start");
                    wrapped = true;
                    break;
                }
                Some(_) => {}
            }
        }
        assert!(jumped.is_some() && wrapped, "{jumped:?}, wrapped {wrapped}");
        let _ = app.update(Message::ToggleFollow(Panel::PianoRoll));
        assert!(!app.settings.follows(Panel::PianoRoll));
        for _ in 0..200 {
            buffer(&mut app, &mut player);
            assert_eq!(app.roll.view.scroll, Ticks(0), "Follow is off");
        }
    }

    // P6 through the window's messages, by hand on p15_long_song's two-bar pattern (its steps
    // reach 1012 px past x 222): in rows 1000 px wide 778 px show, so the bar asks for at most
    // 234 px and the rack's content grows by the bar's 10 px; 1400 px wide they fit, the scroll
    // goes back to 0 and the bar goes; a pattern made one bar long (508 px of steps) fits 1000
    // px too.
    #[test]
    fn the_racks_steps_scroll_within_their_reach() {
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p15_long_song.json"));
        let content = app.rack_content();
        let _ = app.update(Message::RackResized(1000.0));
        let span = app.steps_span();
        assert_eq!((span.content, span.view, span.offset), (1012.0, 778.0, 0.0));
        assert_eq!(app.rack_content(), content + crate::ui::scroll::BAR_HEIGHT);
        let to = |offset| {
            Message::RackBar(crate::ui::scroll::BarMessage::To {
                offset,
                held: false,
            })
        };
        let _ = app.update(to(500.0));
        assert_eq!(app.rack_steps, 234.0, "kept inside the reach");
        let _ = app.update(to(100.0));
        assert_eq!(app.steps_span().offset, 100.0);
        let _ = app.update(Message::RackResized(1400.0));
        assert_eq!((app.rack_steps, app.rack_content()), (0.0, content));
        let _ = app.update(Message::RackResized(1000.0));
        let _ = app.update(to(234.0));
        let _ = app.update(Message::Pattern(PatternMessage::Bars(-1)));
        assert_eq!(app.steps_span().content, 508.0);
        assert_eq!(app.rack_steps, 0.0, "one bar fits");
    }

    /// Where iced keeps the state of the first widget tagged `tag` in `tree`, depth first: the
    /// same address after a diff is the same state kept, a new one a state made anew.
    fn state_address(
        tree: &iced::advanced::widget::Tree,
        tag: iced::advanced::widget::tree::Tag,
    ) -> Option<usize> {
        use iced::advanced::widget::tree::State;
        if tree.tag == tag {
            return match &tree.state {
                State::Some(state) => {
                    Some(std::ptr::from_ref(state.as_ref()).cast::<()>() as usize)
                }
                State::None => None,
            };
        }
        tree.children
            .iter()
            .find_map(|child| state_address(child, tag))
    }

    // The rack's rows keep their scroll up and down
    // when the step bar under them comes and goes (a resize across the steps' reach, Bars −):
    // the view keeps one shape, the bar's row there at no height while the steps fit, so iced
    // keeps the rows' scrollable and its state. Before, the scrollable was the whole view
    // without the bar and inside a column with it, and iced made it anew at the change,
    // scrolled back to the first row. p15_long_song's two bars of steps fit 1400 px, not 1000.
    #[test]
    fn the_racks_rows_keep_their_scroll_when_the_step_bar_comes_and_goes() {
        use iced::advanced::widget::Tree;
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p15_long_song.json"));
        let probe: Element<'_, Message> = scrollable(space()).into();
        let tag = probe.as_widget().tag();
        let _ = app.update(Message::RackResized(1400.0));
        assert!(!app.steps_overflow());
        let mut tree = Tree::new(app.rack_view().as_widget());
        let kept = state_address(&tree, tag).expect("the rows' scrollable");
        let _ = app.update(Message::RackResized(1000.0));
        assert!(app.steps_overflow());
        tree.diff(app.rack_view().as_widget());
        assert_eq!(state_address(&tree, tag), Some(kept), "the bar came");
        let _ = app.update(Message::Pattern(PatternMessage::Bars(-1)));
        assert!(!app.steps_overflow());
        tree.diff(app.rack_view().as_widget());
        assert_eq!(state_address(&tree, tag), Some(kept), "the bar went");
    }

    // A scroll bar whose panel closes while its
    // thumb is held lets go of it. Its canvas is gone, so no release comes, and Follow stayed
    // off until the bar was touched again. The piano roll zoomed to 64 px a step shows 357 of
    // p3_roll's 384 ticks, so the playhead at 370 pages it; the playlist at its closest zoom
    // shows less than the 4 bars past the song's end its content reaches.
    #[test]
    fn a_bar_held_when_its_panel_closes_is_let_go_of() {
        let (mut app, _player) = p3_app();
        app.roll.view.zoom = 8;
        let held = crate::ui::scroll::BarMessage::To {
            offset: 0.0,
            held: true,
        };
        roll(&mut app, RollMessage::Bar(held));
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        let target = Target::of(app.session.project(), app.pattern, app.selected_channel);
        assert!(
            app.roll.follow(target.as_ref(), Ticks(370)),
            "the roll follows"
        );
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        app.arranger.grid.zoom = crate::ui::playlist_layout::ZOOM_LEVELS.len() - 1;
        let _ = app.update(Message::Playlist(PlaylistMessage::Bar(held)));
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        let span = app.arranger.span(app.session.project());
        assert!(span.view < span.content, "{span:?}");
        let past = Ticks(span.view as i64 + 1);
        assert!(
            app.arranger.follow(app.session.project(), past),
            "the playlist follows"
        );
    }

    // The playlist's time scroll bar is at the
    // panel's foot, under the tracks, as every other view's. The five one-bar clips with the
    // playlist alone in a 1000 x 560 window, the hint bar off: the tracks canvas reports
    // 560 − 64 − 24 − 28 − 10 = 434 px (426 under the 72 px toolbar), the bar's 10 px under it; at 256 px a bar its 880 px
    // timeline shows 1320 ticks of the 9 bars (3456 ticks) the bar reaches, so a press in the
    // trough right of the thumb (the bar's middle row, y 555) pages the view to 1320; a press
    // where the bar was, at the header's right end, moves nothing.
    #[test]
    fn the_playlists_time_bar_is_at_its_foot() {
        use crate::ui::playlist_layout::{HEADER_HEIGHT, TIME_BAR_HEIGHT, ZOOM_LEVELS};
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p20_five_clips.json"));
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        let _ = app.update(Message::TogglePanel(Panel::Rack));
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 560.0)));
        app.hints_flag = Some(false);
        assert!(app.panels.playlist && !app.panels.rack && !app.panels.piano_roll);
        app.arranger.grid.zoom = ZOOM_LEVELS.len() - 1;
        let mut window = widget_events::Window::new(&app);
        window.mouse(
            &mut app,
            mouse::Event::CursorLeft,
            mouse::Cursor::Unavailable,
        );
        assert_eq!(app.arranger.tracks_size(), Size::new(1000.0, 434.0));
        let span = app.arranger.span(app.session.project());
        assert_eq!((span.content, span.view), (3456.0, 1320.0));
        let header = Point::new(990.0, layout::TOOLBAR_HEIGHT + HEADER_HEIGHT / 2.0);
        window.click(&mut app, header);
        assert_eq!(
            app.arranger.grid.scroll,
            Ticks(0),
            "nothing where the bar was"
        );
        let foot = Point::new(990.0, 560.0 - TIME_BAR_HEIGHT / 2.0);
        window.click(&mut app, foot);
        assert_eq!(app.arranger.grid.scroll, Ticks(1320), "a page on");
    }

    // Undo of a move drops the moved note from the selection: it no longer exists.
    #[test]
    fn p3_undo_drops_selected_notes_that_are_gone() {
        let (mut app, _player) = p3_app();
        grid_drag(&mut app, at(56.0, 114.0), at(76.0, 114.0));
        assert_eq!(app.roll.selection.len(), 1);
        let _ = app.update(Message::Undo);
        assert!(app.roll.selection.is_empty());
    }

    #[test]
    fn piano_roll_keys_do_nothing_while_it_is_closed() {
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p3_roll.json"));
        let _ = app.update(Message::Duplicate);
        let _ = app.update(Message::Delete);
        assert_eq!(document(&app), fixture("p3_roll.json"));
        assert_eq!(app.session.history().undo_len(), 0);
    }

    #[test]
    fn changing_channel_clears_the_selection_and_shows_its_notes_from_the_top() {
        let (mut app, _player) = p3_app();
        let _ = app.update(Message::SelectAllNotes);
        assert_eq!(app.roll.selection.len(), 2);
        let _ = app.update(Message::ChannelSelected(ChannelId(2)));
        assert!(app.roll.selection.is_empty());
        assert_eq!(app.roll.view.top, 65, "the kicks are key 60");
    }

    #[test]
    fn digits_one_to_six_toggle_the_panels_in_working_order() {
        let none = Modifiers::empty();
        for (digit, panel) in [
            ("1", Panel::Rack),
            ("2", Panel::Playlist),
            ("3", Panel::PianoRoll),
            ("4", Panel::Mixer),
            ("5", Panel::Browser),
            ("6", Panel::Settings),
        ] {
            assert!(
                matches!(shortcut(Key::Character(digit), none, false),
                    Some(Message::TogglePanel(toggled)) if toggled == panel),
                "digit {digit}"
            );
            assert!(
                shortcut(Key::Character(digit), Modifiers::CTRL, false).is_none(),
                "Ctrl+{digit} is no shortcut"
            );
            assert!(
                shortcut(Key::Character(digit), none, true).is_none(),
                "a held {digit} toggles once"
            );
        }
        assert!(shortcut(Key::Character("7"), none, false).is_none());
        assert!(shortcut(Key::Character("0"), none, false).is_none());
    }

    #[test]
    fn function_keys_and_note_keys_map_to_panels_and_commands() {
        let named = |name| Key::Named(name);
        let message = |key, modifiers| shortcut(key, modifiers, false);
        assert!(matches!(
            message(named(key::Named::F7), Modifiers::empty()),
            Some(Message::TogglePanel(Panel::PianoRoll))
        ));
        assert!(matches!(
            message(named(key::Named::F6), Modifiers::empty()),
            Some(Message::TogglePanel(Panel::Rack))
        ));
        assert!(matches!(
            message(named(key::Named::Delete), Modifiers::empty()),
            Some(Message::Delete)
        ));
        assert!(matches!(
            message(named(key::Named::Escape), Modifiers::empty()),
            Some(Message::ClearSelection)
        ));
        assert!(matches!(
            message(Key::Character("b"), Modifiers::CTRL),
            Some(Message::Duplicate)
        ));
        assert!(matches!(
            message(Key::Character("a"), Modifiers::CTRL),
            Some(Message::SelectAllNotes)
        ));
        assert!(message(Key::Character("b"), Modifiers::empty()).is_none());
        assert!(shortcut(Key::Character("b"), Modifiers::CTRL, true).is_none());
    }

    #[test]
    fn a_stream_problem_shows_after_a_failed_edit() {
        let stopped = "Audio stopped: the output device failed";
        let (mut app, _player) = f1_app();
        let _ = app.update(Message::StepClicked {
            channel: ChannelId(1),
            step: 16,
        });
        app.show_stream_problem(Some(stopped));
        let shown = notices(&app);
        assert!(shown.contains(&stopped), "{shown:?}");
        assert!(
            shown
                .iter()
                .any(|notice| notice.starts_with("edit refused")),
            "{shown:?}"
        );
    }

    // ---------------------------------------------------------------------------------------
    // Playlist gestures sent as its canvases send them, in canvas coordinates, at the default zoom
    // (64 px per bar: x = 120 + tick/6, tick = (x - 120)·6) on 32 px rows with a 1 px gap.
    // p4_two_clips holds clip 1 (pattern 1, ticks 0..384, x 120..184) and clip 2 (pattern 1,
    // 768..1152, x 248..312) on track 1.
    // ---------------------------------------------------------------------------------------

    /// `fixture` with the playlist opened by F5.
    fn p4_app(fixture: &str) -> (App, Player) {
        let (mut app, player) = app_on(&Path::new(FIXTURES).join(fixture));
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        (app, player)
    }

    fn playlist(app: &mut App, message: PlaylistMessage) {
        let _ = app.update(Message::Playlist(message));
    }

    fn tracks_press(app: &mut App, at: (f32, f32), button: mouse::Button, double: bool) {
        playlist(
            app,
            PlaylistMessage::TracksPressed {
                at: iced::Point::new(at.0, at.1),
                button,
                double,
            },
        );
    }

    /// A left drag on the tracks from `from` to `to`, through the point half way.
    fn tracks_drag(app: &mut App, from: (f32, f32), to: (f32, f32)) {
        tracks_press(app, from, mouse::Button::Left, false);
        let half = at((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
        playlist(app, PlaylistMessage::TracksDragged(half));
        playlist(app, PlaylistMessage::TracksDragged(at(to.0, to.1)));
        playlist(app, PlaylistMessage::TracksReleased(at(to.0, to.1)));
    }

    // Stretch to fit (by hand in clip_gesture's tests): an Alt drag of track 2's unstretched clip
    // (x 120..376, y 33..65: its 44100-frame file, then silence to the end of four bars) from its
    // right edge back by two bars ends it on bar 3 (tick 768, 176400 samples) and time-stretches
    // its file to fill it: 44100 · 176400 / 44100 = 176400 frames, four times, one undo step that
    // gives the document back. With the Beat snap, dragged to x 168 (tick 300, the nearest beat
    // 288: 66150 samples), the ratio is 66150 / 44100 = 1.5: p9_stretch's own 66150 frames, whose
    // render is gen_p9's 440 Hz tone. Dragged onto the start, the ratio would pass a quarter:
    // refused with a notice, nothing changed.
    #[test]
    fn p9_an_alt_drag_of_an_audio_clips_edge_stretches_it_to_fit() {
        use dawsome_engine::project::{ClipItem, ClipStretch, StretchMode};
        let (mut app, _player) = p4_app("p9_stretch_ui.json");
        let before = app.session.project().clone();
        let _ = app.update(Message::ModifiersChanged(Modifiers::ALT));
        let stretched = |app: &App| {
            let clip = &app.session.project().playlist.clips[&ClipId(2)];
            let ClipItem::Audio {
                stretch,
                frame_offset,
                ..
            } = clip.item
            else {
                panic!("an audio clip");
            };
            (clip.length, stretch, frame_offset)
        };
        let of = |frames| {
            Some(ClipStretch {
                mode: StretchMode::Stretch,
                frames,
                pitch: 0,
            })
        };
        tracks_drag(&mut app, (374.0, 49.0), (246.0, 49.0));
        assert_eq!(stretched(&app), (Ticks(768), of(176_400), 0));
        let _ = app.update(Message::Undo);
        assert_eq!(app.session.project(), &before);
        playlist(&mut app, PlaylistMessage::SnapSelected(ClipSnap::Beat));
        tracks_drag(&mut app, (374.0, 49.0), (168.0, 49.0));
        assert_eq!(stretched(&app), (Ticks(288), of(66_150), 0));
        let p9 = Project::load_json(include_str!("../../../tests/fixtures/p9_stretch.json"))
            .expect("p9_stretch loads");
        let ClipItem::Audio { stretch, .. } = p9.playlist.clips[&ClipId(1)].item else {
            panic!("p9_stretch's audio clip");
        };
        assert_eq!(stretched(&app).1, stretch, "p9_stretch's stretch");
        let _ = app.update(Message::Undo);
        assert_eq!(app.session.project(), &before);
        tracks_drag(&mut app, (374.0, 49.0), (121.0, 49.0));
        assert_eq!(app.session.project(), &before);
        assert_eq!(
            app.notice.as_deref(),
            Some("A clip stretches from a quarter to four times its file")
        );
    }

    // A cut far back along a clip minutes long, through the window's widgets: the clip's end in
    // view, dragged back out of the window on the left. By hand on p9_stretch_ui with
    // track 2's clip made 40 bars long (15360 ticks), the playlist alone in a 1000 px window:
    // the view at bar 29 (10752) shows the clip's end at x 120 + 4608/6 = 888. Pressed at x 886
    // (tick 766·6 + 10752 = 15348), a tick moves nothing while the pointer is over the lanes; at
    // x −12, out of the window and 132 px left of the lanes, the ticks run at the fast rate, 40
    // px (240 ticks) each: four of them take the view to 9792. Brought back to x 184 (tick 384 +
    // 9792 = 10176) and let go of, the end moves by 10176 − 15348 = −5172 to 10188, on the
    // nearest bar 10368: one trim, one undo step.
    #[test]
    fn p9_an_edge_dragged_out_of_the_window_scrolls_the_view_and_cuts_under_the_pointer() {
        use crate::ui::playlist_layout::{HEADER_HEIGHT, RULER_HEIGHT};
        let path = Path::new(FIXTURES).join("p9_stretch_ui.json");
        let mut project = project_file::open(&path)
            .expect("p9_stretch_ui opens")
            .project;
        project
            .playlist
            .clips
            .get_mut(&ClipId(2))
            .expect("track 2's clip")
            .length = Ticks(15360);
        let (mut app, _player) = app_with(project, &path);
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        let _ = app.update(Message::TogglePanel(Panel::Rack));
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 560.0)));
        app.hints_flag = Some(false);
        assert!(app.panels.playlist && !app.panels.rack && !app.panels.browser);
        let mut window = widget_events::Window::new(&app);
        window.mouse(
            &mut app,
            mouse::Event::CursorLeft,
            mouse::Cursor::Unavailable,
        );
        assert_eq!(app.arranger.tracks_size().width, 1000.0);
        let before = app.session.project().clone();
        playlist(
            &mut app,
            PlaylistMessage::Bar(crate::ui::scroll::BarMessage::To {
                offset: 10752.0,
                held: false,
            }),
        );
        let grid = app.arranger.grid;
        let y = layout::TOOLBAR_HEIGHT
            + HEADER_HEIGHT
            + RULER_HEIGHT
            + grid.row_top(1)
            + grid.lane / 2.0;
        let mut pointer = |app: &mut App, event, x: f32| {
            window.mouse(app, event, mouse::Cursor::Available(at(x, y)));
        };
        let moved = |x| mouse::Event::CursorMoved { position: at(x, y) };
        pointer(
            &mut app,
            mouse::Event::ButtonPressed(mouse::Button::Left),
            886.0,
        );
        assert!(app.arranger.tracks_held());
        pointer(&mut app, moved(500.0), 500.0);
        assert!(!app.playlist_edge_scrolls());
        let _ = app.update(Message::Tick);
        assert_eq!(app.arranger.grid.scroll, Ticks(10752), "over the lanes");
        pointer(&mut app, moved(-12.0), -12.0);
        assert!(
            app.playlist_edge_scrolls(),
            "the ticks run while it rests there"
        );
        for _ in 0..4 {
            let _ = app.update(Message::Tick);
        }
        assert_eq!(app.arranger.grid.scroll, Ticks(9792));
        pointer(&mut app, moved(184.0), 184.0);
        assert!(!app.playlist_edge_scrolls());
        pointer(
            &mut app,
            mouse::Event::ButtonReleased(mouse::Button::Left),
            184.0,
        );
        let clip = &app.session.project().playlist.clips[&ClipId(2)];
        assert_eq!((clip.start, clip.length), (Ticks(0), Ticks(10368)));
        let _ = app.update(Message::Undo);
        assert_eq!(app.session.project(), &before);
    }

    // A double press on track 1's stretched clip opens its strip in the header: it shows Stretch at
    // 66150 / 44100 = 150%; Resample drops the pitch stepper's effect, Stretch brings it back and
    // the stepper raises the pitch, None ends the stretch. Each choice is one undo step; × closes
    // the strip.
    #[test]
    fn p9_the_clip_properties_strip_chooses_the_stretch_and_pitch() {
        use crate::ui::playlist::ClipProperties;
        use dawsome_engine::project::{ClipItem, StretchMode};
        let (mut app, _player) = p4_app("p9_stretch_ui.json");
        let before = app.session.project().clone();
        tracks_press(&mut app, (200.0, 24.0), mouse::Button::Left, true);
        assert_eq!(app.arranger.properties, Some(ClipId(1)));
        let shown = |app: &App| app.clip_properties();
        assert_eq!(
            shown(&app),
            Some(ClipProperties {
                mode: Some(StretchMode::Stretch),
                pitch: 0,
                percent: Some(150)
            })
        );
        let stretch = |app: &App| match app.session.project().playlist.clips[&ClipId(1)].item {
            ClipItem::Audio { stretch, .. } => stretch.map(|s| (s.mode, s.frames, s.pitch)),
            _ => panic!("an audio clip"),
        };
        playlist(
            &mut app,
            PlaylistMessage::ClipMode(Some(StretchMode::Resample)),
        );
        assert_eq!(stretch(&app), Some((StretchMode::Resample, 66_150, 0)));
        playlist(&mut app, PlaylistMessage::ClipPitch(1));
        assert_eq!(
            stretch(&app),
            Some((StretchMode::Resample, 66_150, 0)),
            "no pitch"
        );
        playlist(
            &mut app,
            PlaylistMessage::ClipMode(Some(StretchMode::Stretch)),
        );
        playlist(&mut app, PlaylistMessage::ClipPitch(1));
        assert_eq!(stretch(&app), Some((StretchMode::Stretch, 66_150, 1)));
        playlist(&mut app, PlaylistMessage::ClipMode(None));
        assert_eq!(stretch(&app), None);
        assert_eq!(shown(&app).and_then(|p| p.percent), Some(100));
        for _ in 0..4 {
            let _ = app.update(Message::Undo);
        }
        assert_eq!(app.session.project(), &before);
        playlist(&mut app, PlaylistMessage::CloseProperties);
        assert_eq!(app.clip_properties(), None);
    }

    fn ruler_press(app: &mut App, at: (f32, f32), button: mouse::Button) {
        playlist(
            app,
            PlaylistMessage::RulerPressed {
                at: iced::Point::new(at.0, at.1),
                button,
            },
        );
    }

    /// The canonical JSON of one clip as the playlist writes it.
    fn clip_json(id: u32, track: u32, start: i64, length: i64, pattern: u32) -> String {
        format!(
            r#"{{"id":{id},"track":{track},"start":{start},"length":{length},"kind":"pattern","pattern":{pattern}}}"#
        )
    }

    #[test]
    fn f5_opens_the_playlist_and_gives_it_the_keys() {
        let (app, _player) = p4_app("p4_two_clips.json");
        assert!(app.panels.rack && app.panels.playlist && !app.panels.piano_roll);
        assert_eq!(app.focus, Panel::Playlist);
        assert_eq!(
            app.playlist_marker(),
            None,
            "pattern mode draws no song position"
        );
        assert_eq!(document(&app), fixture("p4_two_clips.json"));
    }

    // Pattern 2 as the brush, a press on track 2 (8 px into its lane) at x 330 (tick 1260)
    // places it at bar 4 (tick 1152), a bar long, as clip 3; one undo takes it out. (The
    // lanes fit the panel now, so the press finds track 2's row in the grid; it was
    // y 40 in 32 px lanes.)
    #[test]
    fn p4_a_press_on_an_empty_lane_places_the_selected_pattern() {
        let (mut app, _player) = p4_app("p4_two_clips.json");
        let _ = app.update(Message::PatternSelected(PatternId(2)));
        let track_2 = app.arranger.grid.row_top(1) + 8.0;
        tracks_press(&mut app, (330.0, track_2), mouse::Button::Left, false);
        let saved = document(&app);
        assert!(saved.contains(&clip_json(3, 2, 1152, 384, 2)), "{saved}");
        assert!(saved.contains(r#""next_clip_id":4"#), "{saved}");
        assert_eq!(app.session.history().undo_len(), 1);
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), fixture("p4_two_clips.json"));
    }

    // A body drag of 64 px right and 33 px down moves clip 1 to bar 2 on track 2; a right-edge
    // drag (press 3 px inside, x 181) of 128 px stretches it to 1152 ticks. One undo step each.
    #[test]
    fn p4_dragging_moves_a_clip_and_its_right_edge_resizes_it() {
        let (mut app, _player) = p4_app("p4_two_clips.json");
        tracks_drag(&mut app, (140.0, 16.0), (204.0, 49.0));
        assert!(document(&app).contains(&clip_json(1, 2, 384, 384, 1)));
        assert_eq!(app.arranger.selection, BTreeSet::from([ClipId(1)]));
        assert_eq!(app.session.history().undo_len(), 1);
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), fixture("p4_two_clips.json"));
        tracks_drag(&mut app, (181.0, 16.0), (309.0, 16.0));
        assert!(document(&app).contains(&clip_json(1, 1, 0, 1152, 1)));
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), fixture("p4_two_clips.json"));
    }

    // A Ctrl box over both clips selects them; Ctrl+B copies them one selection length (0..1152)
    // later, to 1152 and 1920, as clips 3 and 4, and selects the copies; a right press on copy 4 (x
    // 450) deletes the selection it is in. Undo walks back through both steps; redo brings the
    // copies back.
    #[test]
    fn p4_box_select_duplicate_and_delete_walk_the_history() {
        let (mut app, _player) = p4_app("p4_two_clips.json");
        let _ = app.update(Message::ModifiersChanged(Modifiers::CTRL));
        tracks_drag(&mut app, (150.0, 50.0), (300.0, 10.0));
        let _ = app.update(Message::ModifiersChanged(Modifiers::empty()));
        assert_eq!(
            app.arranger.selection,
            BTreeSet::from([ClipId(1), ClipId(2)])
        );
        assert_eq!(
            app.session.history().undo_len(),
            0,
            "a box select is no edit"
        );
        let _ = app.update(Message::Duplicate);
        let duplicated = document(&app);
        for copy in [clip_json(3, 1, 1152, 384, 1), clip_json(4, 1, 1920, 384, 1)] {
            assert!(duplicated.contains(&copy), "{duplicated}");
        }
        assert_eq!(
            app.arranger.selection,
            BTreeSet::from([ClipId(3), ClipId(4)])
        );
        tracks_press(&mut app, (450.0, 16.0), mouse::Button::Right, false);
        // Both copies are gone; their IDs stay used.
        let deleted =
            fixture("p4_two_clips.json").replace(r#""next_clip_id":3"#, r#""next_clip_id":5"#);
        assert_eq!(document(&app), deleted);
        assert!(app.arranger.selection.is_empty());
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), duplicated);
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), fixture("p4_two_clips.json"));
        assert!(app.arranger.selection.is_empty(), "the copies are gone");
        let _ = app.update(Message::Redo);
        assert_eq!(document(&app), duplicated);
    }

    // In song mode a click on the ruler's numbers at x 258 (tick 828) moves the marker to bar 3
    // (768, sample 176400). Play runs from there: after 512 frames the playhead is in tick 770
    // (176912 / 229.6875 = 770.2); Stop returns it to 768. Back in pattern mode the playhead is at
    // 0 and there is no marker.
    #[test]
    fn p4_a_ruler_click_sets_where_song_mode_plays_from_and_stop_returns() {
        let (mut app, mut player) = p4_app("p4_two_clips.json");
        let _ = app.update(Message::ModeSelected(PlayMode::Song));
        assert_eq!(app.playlist_marker(), Some(Ticks(0)));
        ruler_press(&mut app, (258.0, 8.0), mouse::Button::Left);
        assert_eq!(app.playlist_marker(), Some(Ticks(768)));
        let _ = app.update(Message::TogglePlay);
        buffer(&mut app, &mut player);
        assert_eq!(app.playhead, Some(Ticks(770)));
        assert_eq!(app.playlist_marker(), Some(Ticks(770)));
        let _ = app.update(Message::TogglePlay);
        buffer(&mut app, &mut player);
        let at_768 = dawsome_engine::realtime::Position {
            samples: 176_400,
            ticks: Ticks(768),
        };
        assert_eq!(app.session.position(), at_768);
        assert_eq!(app.playlist_marker(), Some(Ticks(768)));
        let _ = app.update(Message::ToggleMode);
        assert_eq!(app.mode, PlayMode::Pattern);
        buffer(&mut app, &mut player);
        assert_eq!(app.session.position().samples, 0);
        assert_eq!(app.playlist_marker(), None);
    }

    // p4_two_clips has no loop: a strip drag from x 194 (tick 444) to 258 (828) draws bars 2
    // and 3 (384..1152), switched on. On p4_loop (384..768) the toolbar's Loop button turns it
    // off and on, and a right press in the strip clears it; each is one undo step.
    #[test]
    fn p4_the_loop_strip_and_button_draw_switch_and_clear_the_range() {
        let (mut app, _player) = p4_app("p4_two_clips.json");
        ruler_press(&mut app, (194.0, 20.0), mouse::Button::Left);
        playlist(&mut app, PlaylistMessage::RulerReleased(at(258.0, 20.0)));
        assert!(document(&app).contains(r#""loop":{"start":384,"end":1152,"on":true}"#));

        let (mut app, _player) = p4_app("p4_loop.json");
        let _ = app.update(Message::ToggleLoop);
        assert!(document(&app).contains(r#""loop":{"start":384,"end":768,"on":false}"#));
        let _ = app.update(Message::ToggleLoop);
        assert_eq!(document(&app), fixture("p4_loop.json"));
        ruler_press(&mut app, (300.0, 20.0), mouse::Button::Right);
        assert!(!document(&app).contains(r#""loop""#));
        assert_eq!(app.session.history().undo_len(), 3);
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), fixture("p4_loop.json"));
    }

    /// A left drag on `name`'s loop strip from x `press` through `through` to `release`: the
    /// press and the drag leave the document as it was, and the release sets `start..end` as
    /// one undo step.
    fn assert_loop_drag_commits_on_release(
        name: &str,
        [press, through, release]: [f32; 3],
        (start, end): (i64, i64),
    ) {
        let (mut app, _player) = p4_app(name);
        ruler_press(&mut app, (press, 20.0), mouse::Button::Left);
        assert_eq!(document(&app), fixture(name), "after the press");
        playlist(&mut app, PlaylistMessage::RulerDragged(at(through, 20.0)));
        assert_eq!(document(&app), fixture(name), "after the drag");
        playlist(&mut app, PlaylistMessage::RulerReleased(at(release, 20.0)));
        let range = format!(r#""loop":{{"start":{start},"end":{end},"on":true}}"#);
        assert!(document(&app).contains(&range), "{}", document(&app));
        assert_eq!(app.session.history().undo_len(), 1);
    }

    // A strip drag from x 194 (tick 444) through 230 (660) to 258 (828) draws 384..1152.
    #[test]
    fn p4_a_loop_strip_drag_commits_one_undo_step_on_release() {
        assert_loop_drag_commits_on_release(
            "p4_two_clips.json",
            [194.0, 230.0, 258.0],
            (384, 1152),
        );
    }

    // On p4_loop (384..768) a press at 250 takes the end edge (x 248) and drags it through 313
    // (tick 1158, nearest bar 1152) to 376 (1536).
    #[test]
    fn p4_a_loop_edge_drag_commits_one_undo_step_on_release() {
        assert_loop_drag_commits_on_release("p4_loop.json", [250.0, 313.0, 376.0], (384, 1536));
    }

    // The light of track 1 (x 10, y 16) mutes it; a double press on its name and Enter renames
    // it; Esc while renaming track 2 leaves it as it was; the "+ Track" button under track 8
    // adds track 9. (The button is found where the fitted lanes put it, under
    // row 8; it was y 270..290 under 32 px lanes.)
    #[test]
    fn p4_track_headers_mute_rename_and_add_tracks() {
        let (mut app, _player) = p4_app("p4_two_clips.json");
        tracks_press(&mut app, (10.0, 16.0), mouse::Button::Left, false);
        assert!(document(&app).contains(r#"{"id":1,"name":"Track 1","mute":true}"#));
        tracks_press(&mut app, (60.0, 16.0), mouse::Button::Left, true);
        playlist(&mut app, PlaylistMessage::RenameEdited("Drums".into()));
        playlist(&mut app, PlaylistMessage::RenameSubmitted);
        assert!(document(&app).contains(r#"{"id":1,"name":"Drums","mute":true}"#));
        tracks_press(&mut app, (60.0, 49.0), mouse::Button::Left, true);
        playlist(&mut app, PlaylistMessage::RenameEdited("Bass".into()));
        let _ = app.update(Message::ClearSelection);
        assert_eq!(app.arranger.rename, None);
        assert!(document(&app).contains(r#"{"id":2,"name":"Track 2","mute":false}"#));
        let add = crate::ui::playlist_layout::add_track_rect(&app.arranger.grid, 8).center();
        tracks_press(&mut app, (add.x, add.y), mouse::Button::Left, false);
        assert!(document(&app).contains(r#"{"id":9,"name":"Track 9","mute":false}"#));
        assert_eq!(app.session.history().undo_len(), 3);
    }

    // The lanes fit the tracks. A new document's 8 tracks and the "+ Track" row
    // in a 436 px track area are 9 rows of 48 px: 47 px lanes; with a track added, 10 rows of
    // 43: 42. Alt+wheel up steps to 48 and keeps it (adding a track leaves it 48) and the
    // settings keep it; the header's Fit fits again (11 rows of 39: 38) and the settings forget
    // it. In a 300 px area 11 rows would be 27 px: the lanes keep 32 and the tracks scroll.
    #[test]
    fn the_playlist_lanes_fit_the_tracks_until_a_height_is_chosen() {
        use crate::ui::playlist_layout::{add_track_rect, Lanes};
        let (mut app, _player) = app_with(project_file::new_document(), Path::new(""));
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        playlist(
            &mut app,
            PlaylistMessage::TracksResized(Size::new(1000.0, 436.0)),
        );
        assert_eq!(app.session.project().playlist.tracks.len(), 8);
        assert_eq!(app.arranger.lanes(), Lanes::Fit);
        assert_eq!(app.arranger.grid.lane, 47.0);
        let add = |app: &mut App| {
            let tracks = app.session.project().playlist.tracks.len();
            let at = add_track_rect(&app.arranger.grid, tracks).center();
            tracks_press(app, (at.x, at.y), mouse::Button::Left, false);
        };
        add(&mut app);
        assert_eq!(app.session.project().playlist.tracks.len(), 9);
        assert_eq!(app.arranger.grid.lane, 42.0);
        app.modifiers = Modifiers::ALT;
        playlist(
            &mut app,
            PlaylistMessage::TracksScrolled {
                at: at(300.0, 20.0),
                delta: mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
            },
        );
        app.modifiers = Modifiers::empty();
        assert_eq!(app.arranger.lanes(), Lanes::Fixed(48.0));
        assert_eq!(app.settings.playlist_lanes(), Lanes::Fixed(48.0));
        add(&mut app);
        assert_eq!(app.arranger.grid.lane, 48.0, "a height chosen stays");
        playlist(&mut app, PlaylistMessage::ToggleFit);
        assert_eq!(app.arranger.lanes(), Lanes::Fit);
        assert_eq!(app.arranger.grid.lane, 38.0);
        assert_eq!(app.settings.playlist_lanes(), Lanes::Fit);
        playlist(
            &mut app,
            PlaylistMessage::TracksResized(Size::new(1000.0, 300.0)),
        );
        assert_eq!(app.arranger.grid.lane, 32.0);
        // The Fit switch off keeps the height the lanes have.
        playlist(&mut app, PlaylistMessage::ToggleFit);
        assert_eq!(app.arranger.lanes(), Lanes::Fixed(32.0));
    }

    // Delete and Ctrl+B act on the panel last opened or pressed: with the playlist and the
    // piano roll both open, Delete takes the selected clip out while the playlist has the keys,
    // and the piano roll's notes once it has them.
    #[test]
    fn the_editing_keys_act_on_the_panel_that_has_them() {
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p4_two_clips.json"));
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        assert_eq!(app.focus, Panel::Playlist);
        tracks_press(&mut app, (140.0, 16.0), mouse::Button::Left, false);
        playlist(&mut app, PlaylistMessage::TracksReleased(at(140.0, 16.0)));
        let _ = app.update(Message::Delete);
        let without_clip_1 = document(&app);
        assert!(
            !without_clip_1.contains(r#""id":1,"track""#),
            "{without_clip_1}"
        );
        let notes = |app: &App| app.session.project().patterns[&PatternId(1)].notes.len();
        assert_eq!(notes(&app), 4);
        roll(
            &mut app,
            RollMessage::SnapSelected(crate::ui::note_gesture::Snap::Step),
        );
        assert_eq!(app.focus, Panel::PianoRoll);
        let _ = app.update(Message::SelectAllNotes);
        let _ = app.update(Message::Delete);
        assert_eq!(notes(&app), 0);
        assert!(document(&app).contains(&clip_json(2, 1, 768, 384, 1)));
    }

    // A canvas that is laid out at a new size has not been pressed: opening the piano roll
    // shrinks the playlist, and the roll keeps the keys.
    #[test]
    fn a_resized_panel_does_not_take_the_keys() {
        let (mut app, _player) = p4_app("p4_two_clips.json");
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        assert_eq!(app.focus, Panel::PianoRoll);
        playlist(
            &mut app,
            PlaylistMessage::TracksResized(Size::new(1000.0, 200.0)),
        );
        assert_eq!(app.focus, Panel::PianoRoll);
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        assert_eq!(app.focus, Panel::Playlist);
        roll(&mut app, RollMessage::GridResized(Size::new(1000.0, 150.0)));
        assert_eq!(app.focus, Panel::Playlist);
    }

    #[test]
    fn f5_and_l_map_to_the_playlist_and_the_mode() {
        let message = |key, modifiers| shortcut(key, modifiers, false);
        assert!(matches!(
            message(Key::Named(key::Named::F5), Modifiers::empty()),
            Some(Message::TogglePanel(Panel::Playlist))
        ));
        assert!(matches!(
            message(Key::Character("l"), Modifiers::empty()),
            Some(Message::ToggleMode)
        ));
        // Ctrl+L is not the mode toggle: it routes the selected channel to the mixer's selected
        // insert.
        assert!(matches!(
            message(Key::Character("l"), Modifiers::CTRL),
            Some(Message::RouteToSelectedInsert)
        ));
        assert!(shortcut(Key::Character("l"), Modifiers::empty(), true).is_none());
        let (mut app, _player) = p4_app("p4_two_clips.json");
        let _ = app.update(Message::ToggleMode);
        assert_eq!(app.mode, PlayMode::Song);
        assert_eq!(notices(&app), ["Audio off: no audio output device"]);
    }

    // --song opens the window in song mode, the marker on tick 0.
    #[test]
    fn the_window_can_open_in_song_mode() {
        let path = Path::new(FIXTURES).join("p4_loop.json");
        let project = project_file::open(&path).expect("project opens").project;
        let (session, mut player) =
            Session::open(project, project_file::samples_dir(&path), true).expect("compiles");
        let start = Start {
            document: Document::saved_at(&path),
        };
        let (mut app, _) = App::boot(
            session,
            Err(audio::StartError::NoDevice),
            Panels::default(),
            layout::DEFAULT_WINDOW_SIZE,
            true,
            headless::Script::default(),
            Arc::new(AtomicBool::new(false)),
            Host::offline(),
            start,
        );
        assert_eq!(app.mode, PlayMode::Song);
        assert_eq!(app.playlist_marker(), Some(Ticks(0)));
        // The song plays: bar 1 of p4_loop is pattern 1's four beats.
        let _ = app.update(Message::Play);
        let mut out = vec![[0.0f32; 2]; 22_051];
        player.process(&mut out);
        let sounding: Vec<usize> = (0..out.len()).filter(|&i| out[i][0] != 0.0).collect();
        assert_eq!(sounding, [0, 22_050]);
    }

    // ---------------------------------------------------------------------------------------
    // Mixer gestures sent as the canvas sends them, in canvas coordinates from ui/mixer_layout.rs,
    // on the p5 fixtures tests/fixtures/gen_p5_expected.py wrote. p5_gain plays channel 1 ("max")
    // into insert 3 (fader 1.0), channel 2 into 1, 3 into 2 (fader 0.63496), 4 into 4 (sending half
    // to 5), 5 into 6 (fader 0.63496, sending half to 7). p5_delay_sync holds a delay (effect 1) in
    // slot 0 of insert 1, where channel 1 plays.
    // ---------------------------------------------------------------------------------------

    use dawsome_engine::project::{EffectId, InsertId};

    use crate::ui::mixer_layout::{self as mix, strip_left};
    use crate::ui::mixer_state::{effect_values, Menu};

    fn p5_app(fixture: &str) -> (App, Player) {
        let (mut app, player) = app_on(&Path::new(FIXTURES).join(fixture));
        let _ = app.update(Message::TogglePanel(Panel::Mixer));
        (app, player)
    }

    fn press_at(app: &mut App, at: iced::Point, button: mouse::Button, double: bool) {
        let _ = app.update(Message::Mixer(MixerMessage::Pressed { at, button, double }));
    }

    fn left_click(app: &mut App, at: iced::Point) {
        press_at(app, at, mouse::Button::Left, false);
    }

    /// A press on row `row` of the open slot menu: the message that row sends from the window's
    /// top layer.
    fn choose(app: &mut App, row: usize) {
        let _ = app.update(Message::Mixer(MixerMessage::MenuChosen(row)));
    }

    fn insert_of(app: &App, id: u16) -> &dawsome_engine::project::Insert {
        app.session
            .project()
            .mixer
            .get(InsertId(id))
            .expect("insert exists")
    }

    /// The center of insert `id`'s fader cap, unscrolled.
    fn cap(app: &App, id: u16) -> iced::Point {
        let fader = insert_of(app, id).fader;
        let desk = app.mixer.size().height;
        mix::cap_rect(strip_left(InsertId(id), 0.0), fader, desk).center()
    }

    /// A point on insert `id`'s strip, `offset` in and at mixer y `y`. (The tests'
    /// strip presses moved from y 400 to 300, inside the default 394 px desk's strips.)
    fn strip_point(id: u16, offset: f32, y: f32) -> iced::Point {
        iced::Point::new(strip_left(InsertId(id), 0.0) + offset, y)
    }

    /// One undo gives the fixture's own bytes back, and it was the only step.
    fn one_undo_restores(app: &mut App, name: &str) {
        assert_eq!(app.session.history().undo_len(), 1, "one undo step");
        let _ = app.update(Message::Undo);
        assert_eq!(document(app), fixture(name));
    }

    #[test]
    fn f9_opens_the_mixer_on_the_selected_channels_insert() {
        let (mut app, _player) = p5_app("p5_gain.json");
        assert!(app.panels.mixer);
        assert_eq!(app.focus, Panel::Mixer);
        assert_eq!(
            app.mixer.selected,
            InsertId(3),
            "channel 1 plays into insert 3"
        );
        let _ = app.update(Message::ChannelSelected(ChannelId(2)));
        assert_eq!(
            app.mixer.selected,
            InsertId(1),
            "channel 2 plays into insert 1"
        );
        assert!(matches!(
            shortcut(Key::Named(key::Named::F9), Modifiers::empty(), false),
            Some(Message::TogglePanel(Panel::Mixer))
        ));
        assert!(matches!(
            shortcut(Key::Character("l"), Modifiers::CTRL, false),
            Some(Message::RouteToSelectedInsert)
        ));
        let _ = app.update(Message::TogglePanel(Panel::Mixer));
        assert!(!app.panels.mixer);
    }

    // A fader drag is live (no plan while it moves) and one undo step: 20 px up on insert 2
    // moves it a tenth of the way, 0.63496 → 0.73496; Shift moves ten times finer.
    #[test]
    fn a_fader_drag_is_live_one_plan_and_one_undo_step() {
        let (mut app, _player) = p5_app("p5_gain.json");
        let plans = app.session.plans_sent();
        let at = cap(&app, 2);
        left_click(&mut app, at);
        assert_eq!(
            app.mixer.selected,
            InsertId(2),
            "pressing a fader selects its strip"
        );
        let _ = app.update(Message::DragMoved(300.0));
        let _ = app.update(Message::DragMoved(280.0));
        assert!((insert_of(&app, 2).fader - 0.734_96).abs() < 1e-6);
        let _ = app.update(Message::ModifiersChanged(Modifiers::SHIFT));
        let _ = app.update(Message::DragMoved(300.0));
        let _ = app.update(Message::DragMoved(280.0));
        assert!(
            (insert_of(&app, 2).fader - 0.744_96).abs() < 1e-6,
            "fine: a hundredth"
        );
        assert_eq!(
            app.session.plans_sent(),
            plans,
            "no plan while the fader moves"
        );
        let _ = app.update(Message::DragEnded);
        assert_eq!(app.session.plans_sent(), plans + 1, "one plan on release");
        one_undo_restores(&mut app, "p5_gain.json");
    }

    // A double click puts a fader back to unity and a pan back to the center.
    #[test]
    fn double_clicks_reset_the_fader_to_unity_and_the_pan_to_the_center() {
        let (mut app, _player) = p5_app("p5_gain.json");
        let at = cap(&app, 3);
        press_at(&mut app, at, mouse::Button::Left, true);
        assert_eq!(insert_of(&app, 3).fader, 0.8);
        one_undo_restores(&mut app, "p5_gain.json");
        let (mut app, _player) = p5_app("p5_pan.json");
        let pan = mix::pan_rect(strip_left(InsertId(1), 0.0)).center();
        press_at(&mut app, pan, mouse::Button::Left, true);
        assert_eq!(insert_of(&app, 1).pan, 0.0);
        one_undo_restores(&mut app, "p5_pan.json");
    }

    // The light mutes on a left click; a right click solos the insert alone (again: no solo);
    // Shift + right click adds or removes one insert.
    #[test]
    fn the_light_mutes_and_solos() {
        let (mut app, _player) = p5_app("p5_gain.json");
        let light = |id| mix::led_center(strip_left(InsertId(id), 0.0));
        let solo = |app: &App| -> Vec<u16> {
            app.session
                .project()
                .mixer
                .solo
                .iter()
                .map(|id| id.0)
                .collect()
        };
        left_click(&mut app, light(2));
        assert!(insert_of(&app, 2).mute);
        left_click(&mut app, light(2));
        assert!(!insert_of(&app, 2).mute);
        press_at(&mut app, light(4), mouse::Button::Right, false);
        assert_eq!(solo(&app), [4]);
        let _ = app.update(Message::ModifiersChanged(Modifiers::SHIFT));
        press_at(&mut app, light(6), mouse::Button::Right, false);
        assert_eq!(solo(&app), [4, 6]);
        press_at(&mut app, light(4), mouse::Button::Right, false);
        assert_eq!(solo(&app), [6]);
        let _ = app.update(Message::ModifiersChanged(Modifiers::empty()));
        press_at(&mut app, light(6), mouse::Button::Right, false);
        assert!(
            solo(&app).is_empty(),
            "soloing the only soloed insert clears the solo"
        );
        let _ = app.update(Message::ModifiersChanged(Modifiers::CTRL));
        left_click(&mut app, light(5));
        assert_eq!(solo(&app), [5], "Ctrl + click solos too");
        assert_eq!(
            app.session.history().undo_len(),
            7,
            "one undo step per click"
        );
    }

    // The route button sends the selected insert's whole output (again: removes the send); a
    // send that would close a loop is refused, flashes the button and says why.
    #[test]
    fn route_buttons_toggle_sends_and_a_loop_is_refused() {
        let (mut app, _player) = p5_app("p5_gain.json");
        let desk = app.mixer.size().height;
        let route = |id| mix::route_rect(strip_left(InsertId(id), 0.0), desk).center();
        left_click(&mut app, route(4));
        assert_eq!(insert_of(&app, 3).routes.get(&InsertId(4)), Some(&1.0));
        let after_send = document(&app);
        // Insert 4 selected: its button to insert 3 would make 4 → 3 while 3 → 4.
        left_click(&mut app, strip_point(4, 30.0, 300.0));
        assert_eq!(app.mixer.selected, InsertId(4));
        left_click(&mut app, route(3));
        assert_eq!(document(&app), after_send, "the loop is refused");
        let now = Instant::now();
        assert_eq!(app.mixer.flashing(now), Some(InsertId(3)));
        assert_eq!(
            app.mixer.notice(now),
            Some("Routing loop: Insert 3 already feeds Insert 4")
        );
        // Back on insert 3, the lit button takes the send away again.
        left_click(&mut app, strip_point(3, 30.0, 300.0));
        left_click(&mut app, route(4));
        assert!(!insert_of(&app, 3).routes.contains_key(&InsertId(4)));
        assert_eq!(document(&app), fixture("p5_gain.json"));
    }

    // A lit send knob drags the send's amount: insert 4 sends 0.5 to insert 5, and 20 px up
    // adds a tenth of the range, 0.6.
    #[test]
    fn a_send_knob_drags_the_amount() {
        let (mut app, _player) = p5_app("p5_gain.json");
        let _ = app.update(Message::ChannelSelected(ChannelId(4)));
        assert_eq!(app.mixer.selected, InsertId(4));
        let knob = mix::send_rect(strip_left(InsertId(5), 0.0), app.mixer.size().height).center();
        left_click(&mut app, knob);
        let _ = app.update(Message::DragMoved(100.0));
        let _ = app.update(Message::DragMoved(80.0));
        let _ = app.update(Message::DragEnded);
        assert_eq!(insert_of(&app, 4).routes.get(&InsertId(5)), Some(&0.6));
        one_undo_restores(&mut app, "p5_gain.json");
    }

    // The add menu's rows 4 and 5 are the ported compressor and saturator, at their own defaults,
    // and row 6 the limiter.
    #[test]
    fn the_add_menu_offers_the_compressor_the_saturator_and_the_limiter() {
        let (mut app, _player) = p5_app("p5_delay_sync.json");
        let width = app.mixer.size().width;
        let names: Vec<&str> = crate::ui::mixer_state::ADD_MENU
            .iter()
            .map(|id| app.session.registry().get(id).expect("native").name())
            .collect();
        assert_eq!(
            names,
            [
                "EQ",
                "Delay",
                "Reverb",
                "Compressor (Pressure4)",
                "Saturator (Density)",
                "Limiter"
            ]
        );
        for (slot, row, id) in [
            (1, 3, "dawsome.pressure4"),
            (2, 4, "dawsome.density"),
            (3, 5, "dawsome.limiter"),
        ] {
            left_click(&mut app, mix::slot_rect(slot, width).center());
            choose(&mut app, row);
            let effect = insert_of(&app, 1)
                .effects
                .iter()
                .find(|effect| effect.slot == slot)
                .expect("added");
            assert_eq!(effect.processor.id, id);
        }
    }

    // The slot panel: an empty slot opens the add menu, a choice adds the effect; a filled
    // slot's menu removes or moves it. Each choice is one undo step.
    #[test]
    fn slots_add_move_and_remove_effects() {
        let (mut app, _player) = p5_app("p5_delay_sync.json");
        assert_eq!(app.mixer.selected, InsertId(1));
        let width = app.mixer.size().width;
        let slot = |n| mix::slot_rect(n, width).center();
        left_click(&mut app, slot(1));
        assert_eq!(app.mixer.menu, Some(Menu::Add { slot: 1 }));
        choose(&mut app, 2);
        let effects = |app: &App| -> Vec<(u32, u8, String)> {
            insert_of(app, 1)
                .effects
                .iter()
                .map(|e| (e.id.0, e.slot, e.processor.id.clone()))
                .collect()
        };
        assert_eq!(
            effects(&app),
            [
                (1, 0, "dawsome.delay".to_owned()),
                (2, 1, "dawsome.reverb".to_owned())
            ]
        );
        assert_eq!(app.mixer.selected_effect, Some(EffectId(2)));
        press_at(&mut app, slot(0), mouse::Button::Right, false);
        assert_eq!(
            app.mixer.menu,
            Some(Menu::Effect {
                effect: EffectId(1),
                slot: 0
            })
        );
        choose(&mut app, 2);
        assert_eq!(
            effects(&app),
            [
                (2, 0, "dawsome.reverb".to_owned()),
                (1, 1, "dawsome.delay".to_owned())
            ],
            "Move down swaps it with the reverb"
        );
        press_at(&mut app, slot(1), mouse::Button::Right, false);
        choose(&mut app, 0);
        assert_eq!(effects(&app), [(2, 0, "dawsome.reverb".to_owned())]);
        assert_eq!(app.session.history().undo_len(), 3);
        for _ in 0..3 {
            let _ = app.update(Message::Undo);
        }
        assert_eq!(document(&app), fixture("p5_delay_sync.json"));
    }

    // The slot menu is drawn on
    // the window's top layer, so the mixer's foot never cuts it. p5_gain with the mixer alone at
    // 1000 x 400 and the hint bar on, insert 12 (the last strip in view, running under the slot
    // panel from x 800) selected and its empty slot 9 pressed: the add menu, seven rows of 26 px
    // on 4 px of ground, 188 x 190, opens under the slot at (800 + 28, 64 + 6 + 52 + 22·10) =
    // (828, 342). Down from there it would end at 532, past the hint bar's top at 378, so it
    // flips up to start at 152; it would end at 1016, so it moves left to end at the window's
    // edge, from 812. Each of its rows, clicked there, sends its place in the menu. (Pin
    // moved: 328 and 164 under the 56 px toolbar and no mixer handle; the master
    // limiter, a seventh row: 164 + 26 = 190 high, from 350 − 190 = 160; 8 px up under the
    // 64 px toolbar, from 342 − 190 = 152.)
    #[test]
    fn a_slot_menu_shows_whole_at_the_window_foot() {
        let (mut app, _player) = p5_app("p5_gain.json");
        let _ = app.update(Message::TogglePanel(Panel::Rack));
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 400.0)));
        app.hints_flag = Some(true);
        assert!(app.open_slot_menu(InsertId(12), 9));
        assert_eq!(app.mixer.selected, InsertId(12));
        assert_eq!(app.mixer.menu, Some(Menu::Add { slot: 9 }));
        let mut window = widget_events::Window::new(&app);
        let menus = window.overlays(&app);
        let [menu] = menus[..] else {
            panic!("one menu on the window's top layer: {menus:?}");
        };
        assert_eq!(
            menu,
            iced::Rectangle::new(Point::new(812.0, 152.0), Size::new(188.0, 190.0))
        );
        for row in 0..7 {
            let at = Point::new(menu.center_x(), menu.y + 4.0 + 26.0 * (row as f32 + 0.5));
            let published = window.click_overlay(&app, at);
            assert!(
                matches!(
                    published.as_slice(),
                    [Message::Mixer(MixerMessage::MenuChosen(chosen))] if *chosen == row
                ),
                "row {row}: {published:?}"
            );
        }
    }

    // A parameter slider drags left to right: 12 px of the 120 px slider is a tenth of the
    // delay's feedback range (0..0.95), so 0.5 becomes 0.595. One plan, one undo step.
    #[test]
    fn a_parameter_slider_drags_the_effect_value() {
        let (mut app, _player) = p5_app("p5_delay_sync.json");
        let width = app.mixer.size().width;
        left_click(&mut app, mix::slot_rect(0, width).center());
        assert_eq!(app.mixer.selected_effect, Some(EffectId(1)));
        let plans = app.session.plans_sent();
        left_click(&mut app, mix::slider_rect(2, width).center());
        let _ = app.update(Message::DragMoved(500.0));
        let _ = app.update(Message::DragMoved(512.0));
        let feedback = |app: &App| insert_of(app, 1).effects[0].processor.params["feedback"];
        assert!((feedback(&app) - 0.595).abs() < 1e-6, "{}", feedback(&app));
        assert_eq!(app.session.plans_sent(), plans, "no plan while it moves");
        let _ = app.update(Message::DragEnded);
        assert_eq!(app.session.plans_sent(), plans + 1);
        one_undo_restores(&mut app, "p5_delay_sync.json");
    }

    // The mixer's handle is the divider over the mixer now: its test is
    // `dividers::tests::the_divider_over_the_mixer_sets_its_height_between_its_least_and_the_room`.

    // The default desk (394 px) shows five of the EQ's ten parameter rows; the
    // wheel over them scrolls a row a notch, down to the sixth parameter in the first row (the
    // tenth in the last) and no further. Scrolled to the fifth parameter, "Mid gain" (6 dB in
    // p5_eq_3k), a double press on the first row resets it to 0 dB, one undo step.
    #[test]
    fn the_wheel_scrolls_parameter_rows_the_desk_has_no_room_for() {
        let (mut app, _player) = p5_app("p5_eq_3k.json");
        let width = app.mixer.size().width;
        left_click(&mut app, mix::slot_rect(0, width).center());
        assert_eq!(app.mixer.selected_effect, Some(EffectId(1)));
        assert_eq!(mix::param_rows(app.mixer.size().height), 5);
        let first_row = mix::param_row_rect(0, width).center();
        let wheel = |app: &mut App, y: f32| {
            let delta = mouse::ScrollDelta::Lines { x: 0.0, y };
            let _ = app.update(Message::Mixer(MixerMessage::Scrolled {
                at: first_row,
                delta,
            }));
        };
        for _ in 0..8 {
            wheel(&mut app, -1.0);
        }
        assert_eq!(
            app.mixer.params_top(),
            5,
            "the tenth parameter in the last row"
        );
        wheel(&mut app, 1.0);
        assert_eq!(app.mixer.params_top(), 4);
        let gain = |app: &App| {
            let effect = &insert_of(app, 1).effects[0];
            effect_values(effect, app.session.registry()).expect("values")[4]
        };
        assert_eq!(gain(&app), 6.0);
        press_at(&mut app, first_row, mouse::Button::Left, true);
        assert_eq!(gain(&app), 0.0);
        assert_eq!(app.session.history().undo_len(), 1);
        // Another effect selected shows its first parameter first.
        app.mixer.selected_effect = None;
        assert_eq!(app.mixer.params_top(), 0);
    }

    // The header's arrows and Ctrl+L route the selected channel; the selected insert follows.
    #[test]
    fn the_header_and_ctrl_l_route_the_selected_channel() {
        let (mut app, _player) = p5_app("p5_gain.json");
        let channel_insert = |app: &App| app.session.project().channels[&ChannelId(1)].insert;
        left_click(&mut app, mix::route_up_rect().center());
        assert_eq!(channel_insert(&app), InsertId(4));
        assert_eq!(app.mixer.selected, InsertId(4));
        left_click(&mut app, mix::route_down_rect().center());
        left_click(&mut app, mix::route_down_rect().center());
        assert_eq!(channel_insert(&app), InsertId(2));
        left_click(&mut app, strip_point(9, 30.0, 300.0));
        let _ = app.update(Message::RouteToSelectedInsert);
        assert_eq!(channel_insert(&app), InsertId(9));
        assert_eq!(app.session.history().undo_len(), 4);
    }

    // A double click on a name renames the insert; Enter commits, as one undo step.
    #[test]
    fn a_double_clicked_name_renames_the_insert() {
        let (mut app, _player) = p5_app("p5_gain.json");
        let name = mix::name_rect(strip_left(InsertId(2), 0.0)).center();
        press_at(&mut app, name, mouse::Button::Left, true);
        assert_eq!(app.mixer.rename, Some((InsertId(2), "Insert 2".to_owned())));
        let _ = app.update(Message::Mixer(MixerMessage::RenameEdited("Drums".into())));
        let _ = app.update(Message::Mixer(MixerMessage::RenameSubmitted));
        assert_eq!(insert_of(&app, 2).name, "Drums");
        one_undo_restores(&mut app, "p5_gain.json");
        press_at(&mut app, name, mouse::Button::Left, true);
        let _ = app.update(Message::ClearSelection);
        assert_eq!(app.mixer.rename, None, "Esc ends the rename");
        assert_eq!(app.session.history().undo_len(), 0);
    }

    // A click that changes nothing records nothing: Ctrl+L to the insert the channel already
    // plays into, a double click on a fader already at unity and a rename to the same name
    // leave the document, both history stacks and the saved state as they were. Recorded, each
    // would empty the redo stack and make the saved document dirty.
    #[test]
    fn clicks_that_change_nothing_record_no_undo_step() {
        let (mut app, _player) = p5_app("p5_gain.json");
        // A real change undone, so there is a redo step to lose: insert 3's fader 1.0 → unity.
        let at = cap(&app, 3);
        press_at(&mut app, at, mouse::Button::Left, true);
        let _ = app.update(Message::Undo);
        let history = |app: &App| {
            let history = app.session.history();
            (history.undo_len(), history.redo_len())
        };
        assert_eq!(history(&app), (0, 1));
        assert!(!app.session.is_dirty());
        let (before, state) = (document(&app), app.session.state());

        assert_eq!(app.mixer.selected, InsertId(3), "channel 1 plays into it");
        let _ = app.update(Message::RouteToSelectedInsert);
        assert_eq!(insert_of(&app, 1).fader, 0.8);
        let at = cap(&app, 1);
        press_at(&mut app, at, mouse::Button::Left, true);
        let name = mix::name_rect(strip_left(InsertId(2), 0.0)).center();
        press_at(&mut app, name, mouse::Button::Left, true);
        let _ = app.update(Message::Mixer(MixerMessage::RenameSubmitted));

        assert_eq!(document(&app), before);
        assert_eq!(history(&app), (0, 1), "no undo step, the redo step kept");
        assert_eq!(app.session.state(), state);
        assert!(!app.session.is_dirty());
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    // The rack's knobs drag live too: the kick panned hard left is heard on the left alone (√2, the
    // +3 dB of the circular law) before the drag ends, and the drag is one plan and one undo step.
    #[test]
    fn rack_knob_drags_are_live_and_pan_is_heard() {
        let (mut app, mut player) = f1_app();
        let plans = app.session.plans_sent();
        let _ = app.update(Message::KnobPressed {
            channel: ChannelId(1),
            knob: Knob::Pan,
        });
        let _ = app.update(Message::DragMoved(300.0));
        let _ = app.update(Message::DragMoved(500.0));
        assert_eq!(app.session.project().channels[&ChannelId(1)].pan, -1.0);
        assert_eq!(
            app.session.plans_sent(),
            plans,
            "no plan while the knob turns"
        );
        let _ = app.update(Message::Play);
        let mut out = vec![[0.0f32; 2]; 512];
        player.process(&mut out);
        assert_eq!(out[0], [std::f32::consts::SQRT_2, 0.0]);
        let _ = app.update(Message::DragEnded);
        assert_eq!(app.session.plans_sent(), plans + 1);
        assert_eq!(app.session.history().undo_len(), 1);
    }

    // The meters show each insert's peak after the player has played: the impulse through
    // insert 3 at fader 1.0 peaks at 1.953125 on insert 3 and on the master, which lights the
    // clip light; a click on it clears it.
    #[test]
    fn meters_follow_the_player_and_the_clip_light_latches() {
        let (mut app, mut player) = p5_app("p5_gain.json");
        let _ = app.update(Message::Play);
        player.process(&mut [[0.0; 2]; 512]);
        let _ = app.update(Message::Tick);
        assert_eq!(app.mixer.meters.level(InsertId(3)), [1.953_125; 2]);
        assert_eq!(app.mixer.meters.level(InsertId::MASTER), [1.953_125; 2]);
        assert!(app.mixer.meters.clip);
        let clip = iced::Point::new(
            strip_left(InsertId::MASTER, 0.0) + mix::CLIP_OFFSET,
            mix::CLIP_Y,
        );
        left_click(&mut app, clip);
        assert!(!app.mixer.meters.clip);
    }

    /// p8_ui with the playlist and the browser open on tests/fixtures/browser (rows: the root
    /// "browser", "drums", "tone.wav").
    fn p8_app() -> (App, Player) {
        let path = Path::new(FIXTURES).join("p8_ui.json");
        let project = project_file::open(&path).expect("p8_ui opens").project;
        let host = Host {
            browser_root: Some(Path::new(FIXTURES).join("browser")),
            ..Host::offline()
        };
        let (mut app, player) = app_hosted(project, &path, host);
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        let _ = app.update(Message::TogglePanel(Panel::Browser));
        (app, player)
    }

    fn browser(app: &mut App, message: BrowserMessage) {
        let _ = app.update(Message::Browser(message));
    }

    fn press_row(app: &mut App, row: usize) {
        let at = iced::Point::new(60.0, 118.0 + 20.0 * row as f32);
        browser(
            app,
            BrowserMessage::Pressed {
                row,
                button: mouse::Button::Left,
                at,
            },
        );
    }

    // "tone.wav" (one frame at 44.1 kHz) dragged from the browser onto track 3 at window x 424
    // (tracks canvas x 184, the playlist starting at window x 240) lands at tick (184 − 120) · 384
    // / 64 = 384 with Bar snap: one new audio channel "tone" and one clip of it, one tick long
    // (tick 384 is sample 88200 and tick 385 sample 88429, so one tick holds the frame). One undo
    // takes both out again.
    #[test]
    fn p8_a_file_dragged_onto_track_3_adds_one_audio_clip_and_one_undo_removes_it() {
        let (mut app, _player) = p8_app();
        let before = document(&app);
        playlist(&mut app, PlaylistMessage::SnapSelected(ClipSnap::Bar));
        press_row(&mut app, 2);
        browser(
            &mut app,
            BrowserMessage::Moved(iced::Point::new(424.0, 190.0)),
        );
        let track_3 = app.arranger.grid.row_top(2) + 10.0;
        playlist(
            &mut app,
            PlaylistMessage::BrowserHover(Some(iced::Point::new(184.0, track_3))),
        );
        browser(&mut app, BrowserMessage::Released);
        let project = app.session.project();
        assert_eq!(app.session.history().undo_len(), 1, "one edit");
        let clip = &project.playlist.clips[&ClipId(3)];
        assert_eq!(
            (clip.track, clip.start, clip.length, clip.audio_channel()),
            (TrackId(3), Ticks(384), Ticks(1), Some(ChannelId(3)))
        );
        let channel = &project.channels[&ChannelId(3)];
        assert_eq!(
            (
                channel.name.as_str(),
                channel.kind,
                channel.sample.as_path()
            ),
            ("tone", ChannelKind::Audio, Path::new("browser/tone.wav"))
        );
        assert_eq!(project.playlist.clips.len(), 3);
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), before);
    }

    // A file let go of anywhere but a lane or the rack adds nothing; one let go of over the
    // rack is a new sampler channel.
    #[test]
    fn p8_a_file_dropped_on_the_rack_is_a_sampler_channel_and_elsewhere_nothing() {
        let (mut app, _player) = p8_app();
        let before = document(&app);
        press_row(&mut app, 2);
        browser(
            &mut app,
            BrowserMessage::Moved(iced::Point::new(600.0, 500.0)),
        );
        browser(&mut app, BrowserMessage::Released);
        assert_eq!(document(&app), before, "dropped on nothing");
        press_row(&mut app, 2);
        browser(
            &mut app,
            BrowserMessage::Moved(iced::Point::new(300.0, 80.0)),
        );
        let _ = app.update(Message::Media(MediaMessage::RackHover(true)));
        browser(&mut app, BrowserMessage::Released);
        let channel = &app.session.project().channels[&ChannelId(3)];
        assert_eq!(
            (channel.name.as_str(), channel.kind),
            ("tone", ChannelKind::Sampler)
        );
        assert_eq!(app.session.project().playlist.clips.len(), 2, "no clip");
    }

    // A file let go of over a sampler row (p8_ui's channel 2, "click") becomes that sampler's
    // sound, named relative to the project as the browser's drop names it: no new channel, no
    // clip, one undo step that gives impulse.wav back. Held over the audio row (channel 1) the
    // rack names no sampler, and the drop adds a channel as before.
    #[test]
    fn p8_a_file_dropped_on_a_sampler_row_becomes_its_sound() {
        let (mut app, _player) = p8_app();
        let before = document(&app);
        press_row(&mut app, 2);
        browser(
            &mut app,
            BrowserMessage::Moved(iced::Point::new(300.0, 120.0)),
        );
        let _ = app.update(Message::Media(MediaMessage::RackHover(true)));
        let _ = app.update(Message::Media(MediaMessage::OverSampler(Some(ChannelId(
            2,
        )))));
        browser(&mut app, BrowserMessage::Released);
        assert_eq!(app.browse.drop_sampler, None, "the drag is over");
        let expected = before.replace(
            r#""name":"click","kind":"sampler","sample":"impulse.wav""#,
            r#""name":"click","kind":"sampler","sample":"browser/tone.wav""#,
        );
        assert_ne!(expected, before);
        assert_eq!(document(&app), expected);
        assert_eq!(app.session.history().undo_len(), 1);
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), before);

        press_row(&mut app, 2);
        browser(
            &mut app,
            BrowserMessage::Moved(iced::Point::new(300.0, 80.0)),
        );
        let _ = app.update(Message::Media(MediaMessage::RackHover(true)));
        let _ = app.update(Message::Media(MediaMessage::OverSampler(None)));
        browser(&mut app, BrowserMessage::Released);
        let added = &app.session.project().channels[&ChannelId(3)];
        assert_eq!(
            (added.name.as_str(), added.kind),
            ("tone", ChannelKind::Sampler)
        );
    }

    // A file from the desktop that the rack took on a sampler row takes the same path as the
    // browser's drop; the window does not also place it on the playlist.
    #[test]
    fn p8_a_desktop_file_on_a_sampler_row_becomes_its_sound() {
        let (mut app, _player) = p8_app();
        let before = document(&app);
        let file = Path::new(FIXTURES).join("browser/tone.wav");
        let dropped = window::Event::FileDropped(file.clone());
        let id = window::Id::unique();
        assert!(
            rescale_message(Event::Window(dropped.clone()), event::Status::Captured, id).is_none()
        );
        // The drop carries its window, so the instrument window's are
        // left out (`window_message` makes the main window's `MediaMessage::FileDropped`).
        assert!(matches!(
            rescale_message(Event::Window(dropped), event::Status::Ignored, id),
            Some(Message::FileDroppedOn(window, _)) if window == id
        ));
        let _ = app.update(Message::Media(MediaMessage::DroppedOnSampler {
            channel: ChannelId(2),
            file,
        }));
        let expected = before.replace(
            r#""sample":"impulse.wav""#,
            r#""sample":"browser/tone.wav""#,
        );
        assert_eq!(document(&app), expected);
        assert_eq!(app.session.history().undo_len(), 1);
    }

    /// p8_app as the desktop runs it: work in the background, files loading off the UI
    /// thread, streams never waiting.
    fn p8_live_app() -> (App, Player) {
        let path = Path::new(FIXTURES).join("p8_ui.json");
        let project = project_file::open(&path).expect("p8_ui opens").project;
        let host = Host {
            browser_root: Some(Path::new(FIXTURES).join("browser")),
            background: true,
            ..live_host(None)
        };
        let (mut app, player) = app_hosted(project, &path, host);
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        let _ = app.update(Message::TogglePanel(Panel::Browser));
        (app, player)
    }

    // A preview's sound is made ready on a thread of its own, however short the file: the press
    // returns with nothing handed to the player yet (the UI thread decodes nothing), and the
    // media timer starts it: tone.wav's one frame of 1.0 through the master.
    #[test]
    fn p8_a_preview_is_made_ready_off_the_ui_thread() {
        let (mut app, mut player) = p8_live_app();
        press_row(&mut app, 2);
        browser(&mut app, BrowserMessage::Released);
        let mut out = [[0.0f32; 2]; 64];
        player.process(&mut out);
        assert_eq!(out[0], [0.0; 2], "nothing decoded during the press");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while out[0] == [0.0; 2] {
            assert!(std::time::Instant::now() < deadline, "the preview starts");
            std::thread::sleep(std::time::Duration::from_millis(2));
            let _ = app.update(Message::Media(MediaMessage::Tick));
            player.process(&mut out);
        }
        assert_eq!(out[0], [1.0; 2]);
    }

    // A file dropped on the rack becomes its sampler channel at once, its sound loading off the
    // UI thread until the media timer takes it.
    #[test]
    fn p8_a_file_dropped_on_the_rack_loads_off_the_ui_thread() {
        let (mut app, _player) = p8_live_app();
        press_row(&mut app, 2);
        browser(
            &mut app,
            BrowserMessage::Moved(iced::Point::new(300.0, 80.0)),
        );
        let _ = app.update(Message::Media(MediaMessage::RackHover(true)));
        browser(&mut app, BrowserMessage::Released);
        let sample = app.session.project().channels[&ChannelId(3)].sample.clone();
        assert!(app.session.is_loading(&sample), "loading, not decoded");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.session.is_loading(&sample) {
            assert!(std::time::Instant::now() < deadline, "the file loads");
            std::thread::sleep(std::time::Duration::from_millis(2));
            let _ = app.update(Message::Media(MediaMessage::Tick));
        }
    }

    // A sound given to a sampler row loads as a dropped channel's does: off the UI thread,
    // until the media timer takes it.
    #[test]
    fn p8_a_sound_given_to_a_sampler_row_loads_off_the_ui_thread() {
        let (mut app, _player) = p8_live_app();
        let _ = app.update(Message::Media(MediaMessage::DroppedOnSampler {
            channel: ChannelId(2),
            file: Path::new(FIXTURES).join("browser/tone.wav"),
        }));
        let sample = app.session.project().channels[&ChannelId(2)].sample.clone();
        assert_eq!(sample, Path::new("browser/tone.wav"));
        assert!(app.session.is_loading(&sample), "loading, not decoded");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.session.is_loading(&sample) {
            assert!(std::time::Instant::now() < deadline, "the file loads");
            std::thread::sleep(std::time::Duration::from_millis(2));
            let _ = app.update(Message::Media(MediaMessage::Tick));
        }
    }

    // A press on a file previews it and a second one stops it; the document does not change
    // by a byte, and nothing goes on the undo stack.
    #[test]
    fn p8_a_preview_click_changes_no_byte_of_the_document() {
        let (mut app, _player) = p8_app();
        let before = document(&app);
        press_row(&mut app, 2);
        browser(&mut app, BrowserMessage::Released);
        assert!(app.browse.model.previewing.is_some());
        press_row(&mut app, 2);
        browser(&mut app, BrowserMessage::Released);
        assert_eq!(app.browse.model.previewing, None);
        assert_eq!(document(&app), before);
        assert_eq!(app.session.history().undo_len(), 0);
        assert!(!app.session.is_dirty());
    }

    // Folders open on a press: "drums" lists kick.wav under it.
    #[test]
    fn p8_a_press_on_a_folder_opens_it() {
        let (mut app, _player) = p8_app();
        press_row(&mut app, 1);
        let names: Vec<String> = app
            .browse
            .model
            .rows()
            .into_iter()
            .map(|row| row.name)
            .collect();
        assert_eq!(names, ["browser", "drums", "kick.wav", "tone.wav"]);
    }

    /// p8_rec copied into a temp folder with the impulse it plays, so a recording's file goes
    /// there.
    fn p8_rec_app(test: &str) -> (App, Player, std::path::PathBuf) {
        let dir = std::env::temp_dir()
            .join(format!("dawsome-app-record-{}", std::process::id()))
            .join(test);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        for name in ["p8_rec.json", "impulse.wav"] {
            std::fs::copy(Path::new(FIXTURES).join(name), dir.join(name)).expect("copies");
        }
        let (app, player) = app_on(&dir.join("p8_rec.json"));
        (app, player, dir)
    }

    // An audio take of the master in song mode: two bars (176400 frames at 44.1 kHz and 120
    // BPM) without count-in become "samples/p8_rec take 1.wav" and, on Stop, channel 2 "Take 1"
    // and clip 2 on track 2 (track 1 holds the pattern clip), from tick 0 for 768 ticks:
    // gen_p8_expected.py's known answer (8) through the window. Recording v2: pattern mode no
    // longer switches to the song (a pattern's take records one pass of it), and the take is
    // named after the project (it was rec-0001).
    #[test]
    fn p8_an_audio_take_in_song_mode_lands_the_take() {
        let (mut app, mut player, dir) = p8_rec_app("master");
        let _ = app.update(Message::ModeSelected(PlayMode::Song));
        let record = |app: &mut App, message| {
            let _ = app.update(Message::Record(message));
        };
        record(
            &mut app,
            RecordMessage::Source(crate::play_to::RecordSource::Master),
        );
        record(&mut app, RecordMessage::ToggleCountIn);
        record(&mut app, RecordMessage::ToggleArm);
        let _ = app.update(Message::Play);
        assert_eq!(app.mode, PlayMode::Song);
        assert!(app.recording_take());
        for _ in 0..441 {
            player.process(&mut [[0.0; 2]; 400]);
        }
        let _ = app.update(Message::Stop);
        assert!(!app.recording_take());
        let project = app.session.project();
        let clip = &project.playlist.clips[&ClipId(2)];
        assert_eq!(
            (clip.track, clip.start, clip.length, clip.audio_channel()),
            (TrackId(2), Ticks(0), Ticks(768), Some(ChannelId(2)))
        );
        let channel = &project.channels[&ChannelId(2)];
        assert_eq!(
            (channel.name.as_str(), channel.sample.as_path()),
            ("Take 1", Path::new("samples/p8_rec take 1.wav"))
        );
        let reader =
            hound::WavReader::open(dir.join("samples/p8_rec take 1.wav")).expect("written");
        assert_eq!((reader.duration(), reader.spec().channels), (176_400, 2));
    }

    // A take whose writer cannot write (a full disk: /dev/full takes the file and refuses every
    // flush) is stopped by the playhead timer with a notice, the transport stops, and nothing
    // lands in the document. Linux only: Windows has no file that takes a create and refuses
    // every write.
    #[cfg(target_os = "linux")]
    #[test]
    fn p8_a_take_on_a_full_disk_stops_with_a_notice() {
        let (mut app, mut player, _dir) = p8_rec_app("full_disk");
        let before = document(&app);
        let _ = app.update(Message::Record(RecordMessage::Source(
            crate::play_to::RecordSource::Master,
        )));
        let _ = app.update(Message::Record(RecordMessage::ToggleCountIn));
        app.transport_on = app.start_take_into(std::path::PathBuf::from("/dev/full"));
        assert!(app.recording_take());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while app.recording_take() {
            assert!(
                std::time::Instant::now() < deadline,
                "the timer stops the take"
            );
            player.process(&mut [[0.0; 2]; 512]);
            std::thread::sleep(std::time::Duration::from_millis(1));
            let _ = app.update(Message::Tick);
        }
        let notice = app.notice.clone().unwrap_or_default();
        assert!(
            notice.starts_with("Recording failed") && notice.contains("space"),
            "{notice}"
        );
        player.process(&mut [[0.0; 2]; 512]);
        let _ = app.update(Message::Tick);
        assert!(!app.playing && !app.transport_on, "the transport stopped");
        assert_eq!(document(&app), before);
    }

    // A take stopped before anything played is empty: its file is deleted and nothing lands.
    #[test]
    fn p8_an_empty_take_leaves_nothing() {
        let (mut app, _player, dir) = p8_rec_app("empty");
        let before = document(&app);
        let _ = app.update(Message::Record(RecordMessage::Source(
            crate::play_to::RecordSource::Master,
        )));
        let _ = app.update(Message::Record(RecordMessage::ToggleArm));
        let _ = app.update(Message::Play);
        let _ = app.update(Message::Stop);
        assert_eq!(document(&app), before);
        assert!(!dir.join("samples/p8_rec take 1.wav").exists());
    }

    // A right press on the kick's volume knob offers "Create automation clip": two points at
    // the knob's value (volume 1.0 of 0..4: 0.25), on track 2 below the pattern clip; then the
    // knob is automated, so the same press offers "Show automation clip".
    #[test]
    fn p8_a_right_press_on_a_knob_creates_its_automation_clip() {
        let (mut app, _player, _dir) = p8_rec_app("automate");
        let at = iced::Point::new(182.0, 30.0);
        let knob_menu = Message::KnobMenu {
            channel: ChannelId(1),
            knob: Knob::Volume,
            at,
        };
        let _ = app.update(knob_menu.clone());
        assert!(app.automate.is_some());
        let _ = app.update(Message::AutomateChosen);
        let project = app.session.project();
        let automation = &project.automation.list[&dawsome_engine::project::AutomationId(1)];
        assert_eq!(
            automation.target,
            dawsome_engine::project::AutomationTarget::ChannelVolume(ChannelId(1))
        );
        assert_eq!(
            automation
                .points
                .iter()
                .map(|p| (p.tick, p.value))
                .collect::<Vec<_>>(),
            [(Ticks(0), 0.25), (Ticks(1536), 0.25)]
        );
        let clip = &project.playlist.clips[&ClipId(2)];
        assert_eq!(
            (clip.track, clip.start, clip.length),
            (TrackId(2), Ticks(0), Ticks(1536))
        );
        assert_eq!(app.session.history().undo_len(), 1);
        let _ = app.update(knob_menu);
        let _ = app.update(Message::AutomateChosen);
        assert_eq!(app.session.history().undo_len(), 1, "shown, not made again");
        assert!(app.panels.playlist && app.arranger.selection.contains(&ClipId(2)));
    }

    /// A host whose session plays as live playback does: streams never wait and files load in
    /// the background; files of more than `stream_threshold` frames stream.
    fn live_host(stream_threshold: Option<u64>) -> Host {
        Host {
            playback: dawsome_media::Playback {
                stream_threshold,
                waiting: dawsome_media::Waiting::Never,
                convert_dir: None,
                stretch_dir: None,
                edited_dir: None,
            },
            ..Host::offline()
        }
    }

    // The player's events become notices: a streamed clip whose blocks came too late is named.
    // p8_stream with every clip streamed live; a Seek while stopped to tick 96 puts the "saw"
    // clip 1's stream on frame 22050, past its first cache, and Play right after finds it still
    // filling.
    #[test]
    fn p8_a_stream_that_falls_behind_is_named_in_a_notice() {
        let path = Path::new(FIXTURES).join("p8_stream.json");
        let project = project_file::open(&path).expect("p8_stream opens").project;
        let (mut app, mut player) = app_hosted(project, &path, live_host(Some(0)));
        let _ = app.update(Message::ModeSelected(PlayMode::Song));
        app.session.send(Control::Seek(Ticks(96))).expect("room");
        player.process(&mut []);
        app.session.send(Control::Play).expect("room");
        player.process(&mut [[0.0; 2]; 512]);
        assert!(app.session.underruns() > 0, "the stream was not ready");
        let _ = app.update(Message::Tick);
        assert_eq!(
            app.notice.as_deref(),
            Some("The disk fell behind: saw played silence")
        );
    }

    // An automated knob shows the automation's value while it plays, and its own when the
    // automation lets go: p8_ui's ramp sets the click's volume 4 → 0 over bar 1.
    #[test]
    fn p8_an_automated_knob_follows_its_automation() {
        let (mut app, mut player) = p8_app();
        let target = dawsome_engine::project::AutomationTarget::ChannelVolume(ChannelId(2));
        assert_eq!(
            app.automated.get(&target),
            Some(&None),
            "automated, not governing"
        );
        let _ = app.update(Message::ModeSelected(PlayMode::Song));
        let _ = app.update(Message::Play);
        player.process(&mut [[0.0; 2]; 512]);
        let _ = app.update(Message::Tick);
        let value = app.automated[&target].expect("governs while it plays");
        assert!(
            (3.9..4.0).contains(&value),
            "near the top of the ramp: {value}"
        );
        let _ = app.update(Message::Stop);
        player.process(&mut [[0.0; 2]; 512]);
        let _ = app.update(Message::Tick);
        assert_eq!(app.automated.get(&target), Some(&None));
    }

    // A press on a panel button toggles its panel as its keys do; closing the window keeps
    // the panels open, the size it had when last not maximized and whether it is, in the
    // settings, which the next start opens with.
    // A right press on a track's name deletes nothing by itself: it
    // opens the track's menu there, whose Rename opens the name's box and Delete deletes the
    // track with its clips (one undo step); Esc and any other press close the menu.
    #[test]
    fn a_right_press_on_a_track_name_opens_its_menu() {
        use crate::ui::hint::TrackRow;
        use dawsome_engine::project::TrackId;
        let (mut app, _player) = p4_app("p4_two_clips.json");
        let before = document(&app);
        let name = at(40.0, 10.0);
        tracks_press(&mut app, (name.x, name.y), mouse::Button::Right, false);
        assert_eq!(document(&app), before, "nothing deleted by the press");
        assert_eq!(
            app.arranger.track_menu.map(|(track, _)| track),
            Some(TrackId(1))
        );
        let _ = app.update(Message::ClearSelection);
        assert_eq!(app.arranger.track_menu, None, "Esc closes it");
        tracks_press(&mut app, (name.x, name.y), mouse::Button::Right, false);
        playlist(&mut app, PlaylistMessage::TrackMenu(TrackRow::Rename));
        assert_eq!(
            app.arranger
                .rename
                .as_ref()
                .map(|(track, name)| (*track, name.as_str())),
            Some((TrackId(1), "Track 1"))
        );
        assert_eq!(app.arranger.track_menu, None);
        tracks_press(&mut app, (name.x, name.y), mouse::Button::Right, false);
        // A right press on an empty lane, which changes nothing else.
        tracks_press(&mut app, (300.0, 100.0), mouse::Button::Right, false);
        assert_eq!(app.arranger.track_menu, None, "another press closes it");
        assert_eq!(document(&app), before);
        tracks_press(&mut app, (name.x, name.y), mouse::Button::Right, false);
        playlist(&mut app, PlaylistMessage::TrackMenu(TrackRow::Delete));
        assert!(!app
            .session
            .project()
            .playlist
            .tracks
            .contains_key(&TrackId(1)));
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), before);
    }

    // C4: on a chord written in the piano roll (C5, E5 and G5 on step 0 of F1's row, the C5
    // its own step), a step press removes the C5 alone, one undo step ("Delete note"); on a
    // chord without C5 it removes nothing and says to use the piano roll.
    #[test]
    fn a_step_press_never_deletes_a_chord() {
        use dawsome_engine::project::Note;
        let (mut app, _player) = f1_app();
        let chord: Vec<Note> = [(0, 64), (0, 67), (48, 64), (48, 67)]
            .into_iter()
            .map(|(tick, key)| Note {
                channel: ChannelId(1),
                start: Ticks(tick),
                length: Ticks(24),
                velocity: 0.5,
                key,
            })
            .collect();
        app.commit(Edit::ReplaceNotes {
            pattern: PatternId(1),
            remove: Vec::new(),
            add: chord,
        });
        let keys_at = |app: &App, tick: i64| -> Vec<u8> {
            app.session.project().patterns[&PatternId(1)]
                .notes
                .iter()
                .filter(|note| note.start == Ticks(tick))
                .map(|note| note.key)
                .collect()
        };
        assert_eq!(keys_at(&app, 0), [60, 64, 67]);
        let _ = app.update(Message::StepClicked {
            channel: ChannelId(1),
            step: 0,
        });
        assert_eq!(keys_at(&app, 0), [64, 67], "only C5 goes");
        assert_eq!(app.session.history().undo_name(), Some("Delete note"));
        let _ = app.update(Message::StepClicked {
            channel: ChannelId(1),
            step: 2,
        });
        assert_eq!(keys_at(&app, 48), [64, 67], "a chord without C5 stays");
        assert_eq!(app.notice.as_deref(), Some(MELODY_STEP));
        // The hint says so before the press.
        let step = crate::ui::hint::Hover::Rack {
            row: 0,
            part: crate::ui::hint::RackPart::Step(2),
        };
        let _ = app.update(Message::Hover(crate::ui::hint::HoverMessage::Over(step)));
        assert_eq!(
            app.hint_words(),
            "Step 3 \u{b7} a chord \u{b7} the piano roll changes it"
        );
        // A plain step still switches off and on.
        let _ = app.update(Message::StepClicked {
            channel: ChannelId(1),
            step: 4,
        });
        assert_eq!(keys_at(&app, 96), Vec::<u8>::new());
    }

    // A hit recorded off C5 (the typing keyboard's Z, key 48) on step 1
    // of F1's kick row goes with a press on its step, one undo step ("Step"), as before C4, and
    // the hint says the press switches it; a note starting between steps (tick 54, inside step
    // 2) stays when step 2 is switched on beside it.
    #[test]
    fn a_step_press_removes_a_single_note_at_any_key() {
        use dawsome_engine::project::Note;
        let (mut app, _player) = f1_app();
        let note = |tick: i64, key: u8| Note {
            channel: ChannelId(1),
            start: Ticks(tick),
            length: Ticks(24),
            velocity: 0.5,
            key,
        };
        app.commit(Edit::ReplaceNotes {
            pattern: PatternId(1),
            remove: Vec::new(),
            add: vec![note(24, 48), note(54, 50)],
        });
        let keys_at = |app: &App, tick: i64| -> Vec<u8> {
            app.session.project().patterns[&PatternId(1)]
                .notes
                .iter()
                .filter(|note| note.start == Ticks(tick))
                .map(|note| note.key)
                .collect()
        };
        let step = crate::ui::hint::Hover::Rack {
            row: 0,
            part: crate::ui::hint::RackPart::Step(1),
        };
        let _ = app.update(Message::Hover(crate::ui::hint::HoverMessage::Over(step)));
        assert_eq!(
            app.hint_words(),
            "Step 2 \u{b7} on \u{b7} click switches it"
        );
        let _ = app.update(Message::StepClicked {
            channel: ChannelId(1),
            step: 1,
        });
        assert_eq!(keys_at(&app, 24), Vec::<u8>::new(), "the C4 hit goes");
        assert_eq!(app.session.history().undo_name(), Some("Step"));
        assert_eq!(app.notice, None);
        let _ = app.update(Message::StepClicked {
            channel: ChannelId(1),
            step: 2,
        });
        assert_eq!(keys_at(&app, 48), [60], "step 2 switched on");
        assert_eq!(keys_at(&app, 54), [50], "the note between steps stays");
    }

    // C3: a click on the tempo (a press let go of where it was, the tempo unchanged) opens its
    // box with the tempo in it; Enter applies what is typed, one undo step; a tempo out of range
    // is refused with a notice and the box stays; Esc closes it unchanged; a drag still drags
    // and opens nothing.
    #[test]
    fn a_click_on_the_tempo_types_it() {
        let (mut app, _player) = f1_app();
        let click = |app: &mut App| {
            let _ = app.update(Message::TempoPressed);
            let _ = app.update(Message::DragEnded);
        };
        click(&mut app);
        assert_eq!(app.tempo_edit.as_deref(), Some("120.000"));
        let _ = app.update(Message::TempoEdited("87.5".into()));
        let _ = app.update(Message::TempoSubmitted);
        assert_eq!(app.session.project().tempo, MilliBpm(87_500));
        assert_eq!(app.tempo_edit, None);
        assert_eq!(app.session.history().undo_name(), Some("Tempo"));
        click(&mut app);
        let _ = app.update(Message::TempoEdited("900".into()));
        let _ = app.update(Message::TempoSubmitted);
        assert_eq!(app.session.project().tempo, MilliBpm(87_500));
        assert_eq!(app.notice.as_deref(), Some("Tempo goes from 10 to 522 BPM"));
        assert!(app.tempo_edit.is_some(), "the box stays to be put right");
        let _ = app.update(Message::ClearSelection);
        assert_eq!(app.tempo_edit, None, "Esc closes it");
        assert_eq!(app.session.project().tempo, MilliBpm(87_500));
        let _ = app.update(Message::TempoPressed);
        let _ = app.update(Message::DragMoved(300.0));
        let _ = app.update(Message::DragMoved(290.0));
        let _ = app.update(Message::DragEnded);
        assert_eq!(app.tempo_edit, None, "a drag opens no box");
        assert_ne!(app.session.project().tempo, MilliBpm(87_500), "and drags");
    }

    // C1: the toolbar's metronome switches the player's metronome on and off, and a new
    // document's player gets it on again.
    #[test]
    fn the_metronome_switch_reaches_the_player_and_a_new_documents() {
        let metronome = |app: &App| -> Vec<bool> {
            app.session
                .controls_sent()
                .iter()
                .filter_map(|control| match control {
                    Control::Metronome(on) => Some(*on),
                    _ => None,
                })
                .collect()
        };
        let (mut app, _player) = f1_app();
        let _ = app.update(Message::ToggleMetronome);
        assert!(app.metronome);
        assert_eq!(metronome(&app), [true]);
        let _ = app.update(Message::File(FileMessage::New));
        assert_eq!(metronome(&app), [true], "the new player hears it on");
        let _ = app.update(Message::ToggleMetronome);
        assert_eq!(metronome(&app), [true, false]);
    }

    // B4: the Edit menu names the steps Undo and Redo take. F1: a step switched on ("Step"),
    // a volume knob dragged ("Channel volume", the whole drag one step), the channel cloned
    // ("Clone channel"); Undo from the menu takes the clone back and closes the menu, Redo then
    // names it. Before any step Undo is dim ("Undo", nothing to send). Esc closes the menu, and
    // the File menu and the Edit menu never show together.
    #[test]
    fn the_edit_menu_names_the_steps_undo_and_redo_take() {
        let (mut app, _player) = f1_app();
        let menu = app.edit_menu_view();
        assert_eq!((menu.undo, menu.redo), (None, None));
        let _ = app.update(Message::StepClicked {
            channel: ChannelId(1),
            step: 1,
        });
        assert_eq!(app.edit_menu_view().undo.as_deref(), Some("Undo Step"));
        let _ = app.update(Message::KnobPressed {
            channel: ChannelId(1),
            knob: Knob::Volume,
        });
        let _ = app.update(Message::DragMoved(200.0));
        let _ = app.update(Message::DragMoved(180.0));
        let _ = app.update(Message::DragEnded);
        assert_eq!(
            app.edit_menu_view().undo.as_deref(),
            Some("Undo Channel volume")
        );
        let _ = app.update(Message::Channel(ChannelCommand::Clone(ChannelId(1))));
        let _ = app.update(Message::ToggleEditMenu);
        assert!(app.edit_menu);
        let menu = app.edit_menu_view();
        assert_eq!(
            (menu.undo.as_deref(), menu.redo.as_deref()),
            (Some("Undo Clone channel"), None)
        );
        let _ = app.update(Message::Undo);
        assert!(!app.edit_menu, "an entry closes the menu");
        assert_eq!(app.session.project().channels.len(), 1);
        let menu = app.edit_menu_view();
        assert_eq!(
            (menu.undo.as_deref(), menu.redo.as_deref()),
            (Some("Undo Channel volume"), Some("Redo Clone channel"))
        );
        let _ = app.update(Message::ToggleEditMenu);
        let _ = app.update(Message::ClearSelection);
        assert!(!app.edit_menu, "Esc closes it");
        let _ = app.update(Message::ToggleEditMenu);
        let _ = app.update(Message::File(FileMessage::ToggleMenu));
        assert!(app.files.menu && !app.edit_menu);
        let _ = app.update(Message::ToggleEditMenu);
        assert!(app.edit_menu && !app.files.menu);
    }

    // B2: in a 1000 × 560 window (504 px under the toolbar) F1's rack (70 px least), the
    // playlist (180) and the piano roll (224) fit; the mixer (180) opened then does not, so the
    // most recently used of the others stay while they fit beside it: the roll and the rack
    // (474 px), not the playlist, which closes, named in a notice, whether or not the rack was
    // pressed after it.
    #[test]
    fn a_panel_opened_without_room_closes_the_least_recently_used_with_a_notice() {
        let open_three = |press_rack: bool| {
            let (mut app, player) = f1_app();
            let _ = app.update(Message::TogglePanel(Panel::Playlist));
            let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
            if press_rack {
                let _ = app.update(Message::StepClicked {
                    channel: ChannelId(1),
                    step: 1,
                });
            }
            assert!(app.panels.rack && app.panels.playlist && app.panels.piano_roll);
            assert_eq!(app.notice, None);
            let _ = app.update(Message::TogglePanel(Panel::Mixer));
            (app, player)
        };
        let (app, _player) = open_three(true);
        assert!(app.panels.rack && !app.panels.playlist && app.panels.piano_roll);
        assert!(app.panels.mixer);
        assert_eq!(
            app.notice.as_deref(),
            Some("Playlist closed to make room for Mixer")
        );
        let (app, _player) = open_three(false);
        assert!(app.panels.rack && !app.panels.playlist);
        assert!(app.panels.piano_roll && app.panels.mixer);
        assert_eq!(
            app.notice.as_deref(),
            Some("Playlist closed to make room for Mixer")
        );
        // In a taller window all four fit and nothing closes.
        let (mut app, _player) = f1_app();
        let _ = app.update(Message::WindowResized(Size::new(1280.0, 800.0)));
        for panel in [Panel::Playlist, Panel::PianoRoll, Panel::Mixer] {
            let _ = app.update(Message::TogglePanel(panel));
        }
        assert_eq!(app.panels.stacked().count(), 4);
        assert_eq!(app.notice, None);
    }

    // F1 at 1000 × 560 with the playlist and the piano roll open, the
    // window made 420 px tall (364 px under the toolbar, less than the three panels' 474 px of
    // least heights): the resize closes nothing, and neither does opening the Library, a
    // column beside the stack, which squeezes on.
    #[test]
    fn opening_the_library_or_resizing_the_window_closes_no_panel() {
        let (mut app, _player) = f1_app();
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        let three = [Panel::Rack, Panel::Playlist, Panel::PianoRoll];
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 420.0)));
        assert_eq!(app.panels.stacked().collect::<Vec<_>>(), three);
        let _ = app.update(Message::TogglePanel(Panel::Browser));
        assert!(app.panels.browser);
        assert_eq!(app.panels.stacked().collect::<Vec<_>>(), three);
        assert_eq!(app.notice, None);
    }

    // F1's stacked panels opened in turn at 1000 × 560 (504 px): each
    // panel opened keeps the most recently used of the others that fit beside it, and the
    // notice names only those that closed: the mixer closes the playlist (the rack stays, the
    // roll and the mixer leaving it room). Settings then opens its window and
    // closes nothing (stacked, it closed the piano roll).
    #[test]
    fn a_panel_opened_without_room_keeps_the_most_recently_used_that_fit() {
        let (mut app, _player) = f1_app();
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        assert_eq!(app.notice, None);
        let _ = app.update(Message::TogglePanel(Panel::Mixer));
        assert_eq!(
            app.panels.stacked().collect::<Vec<_>>(),
            [Panel::Rack, Panel::PianoRoll, Panel::Mixer]
        );
        assert_eq!(
            app.notice.as_deref(),
            Some("Playlist closed to make room for Mixer")
        );
        let _ = app.update(Message::TogglePanel(Panel::Settings));
        assert!(app.panels.settings && app.settings_window.is_some());
        assert_eq!(
            app.panels.stacked().collect::<Vec<_>>(),
            [Panel::Rack, Panel::PianoRoll, Panel::Mixer]
        );
        assert_eq!(
            app.notice.as_deref(),
            Some("Playlist closed to make room for Mixer"),
            "no new notice"
        );
    }

    // 6d review 4: a stray digit is undone by the same digit. F1 at 1000 x 560 with the rack,
    // the playlist and the piano roll open and the keyboard on: 4, a finger reaching past E and
    // R, opens the mixer, which closes the playlist to make room (B2); 4 again closes the mixer
    // and brings the playlist back, and the notice says so. A panel opened by hand meanwhile
    // stays as it is (here 2 reopens the playlist, which closes the roll; 4 then closes the
    // mixer alone, and 2 again brings the roll back), and one with no room left stays closed
    // (the window made 420 px tall). (The playlist's digit is 2, was 3.)
    #[test]
    fn the_same_digit_again_brings_back_what_its_panel_closed() {
        let open = |app: &App| {
            let panels = app.panels;
            (
                panels.rack,
                panels.playlist,
                panels.piano_roll,
                panels.mixer,
            )
        };
        let digit = |app: &mut App, typed: &str, code: key::Code| {
            for pressed in [true, false] {
                let _ = app.update(Message::Key(KeyInput {
                    window: window::Id::unique(),
                    key: Key::Character(typed.into()),
                    code: Some(code),
                    modifiers: Modifiers::empty(),
                    pressed,
                    repeat: false,
                }));
            }
        };
        let three_open = || {
            let (mut app, player) = f1_app();
            assert!(app.typing.on);
            let _ = app.update(Message::TogglePanel(Panel::Playlist));
            let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
            assert_eq!(open(&app), (true, true, true, false));
            digit(&mut app, "4", key::Code::Digit4);
            assert_eq!(open(&app), (true, false, true, true));
            assert_eq!(
                app.notice.as_deref(),
                Some("Playlist closed to make room for Mixer")
            );
            (app, player)
        };
        let (mut app, _player) = three_open();
        digit(&mut app, "4", key::Code::Digit4);
        assert_eq!(open(&app), (true, true, true, false));
        assert_eq!(app.notice.as_deref(), Some("Playlist open again"));

        let (mut app, _player) = three_open();
        digit(&mut app, "2", key::Code::Digit2);
        assert_eq!(open(&app), (true, true, false, true));
        let roll_closed = Some("Piano roll closed to make room for Playlist");
        assert_eq!(app.notice.as_deref(), roll_closed);
        digit(&mut app, "4", key::Code::Digit4);
        assert_eq!(open(&app), (true, true, false, false));
        assert_eq!(app.notice.as_deref(), roll_closed);
        digit(&mut app, "2", key::Code::Digit2);
        assert_eq!(open(&app), (true, false, true, false));
        assert_eq!(app.notice.as_deref(), Some("Piano roll open again"));

        let (mut app, _player) = three_open();
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 420.0)));
        digit(&mut app, "4", key::Code::Digit4);
        assert_eq!(open(&app), (true, false, true, false), "no room");
        assert_eq!(
            app.notice.as_deref(),
            Some("Playlist closed to make room for Mixer")
        );
    }

    #[test]
    fn closing_keeps_the_window_layout_for_the_next_start() {
        let dir = std::env::temp_dir().join(format!("dawsome-layout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("dawsome/settings.json");
        let mut host = Host::offline();
        host.settings_file = Some(file.clone());
        host.saves_settings = true;
        let (mut app, _player) = app_hosted(project_file::new_document(), Path::new(""), host);
        let _ = app.update(Message::TogglePanel(Panel::Browser));
        let _ = app.update(Message::TogglePanel(Panel::Mixer));
        let _ = app.update(Message::WindowResized(Size::new(1100.0, 700.0)));
        let _ = app.update(Message::WindowMaximized(false));
        let _ = app.update(Message::WindowResized(Size::new(1920.0, 1080.0)));
        let _ = app.update(Message::WindowMaximized(true));
        let _ = app.update(Message::File(FileMessage::CloseRequested));
        let saved = crate::settings::Settings::load(Some(&file));
        let start = crate::settings::start_window(saved.window.as_ref());
        let mut panels = Panels::default();
        panels.set(Panel::Browser, true);
        panels.set(Panel::Mixer, true);
        assert_eq!(
            start,
            crate::settings::StartWindow {
                panels,
                size: Size::new(1100.0, 700.0),
                maximized: true,
            }
        );
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    /// The live notes the session sent the player, in order.
    fn live_notes_sent(app: &App) -> Vec<Control> {
        app.session
            .controls_sent()
            .iter()
            .filter(|control| {
                matches!(
                    control,
                    Control::LiveNoteOn { .. } | Control::LiveNoteOff { .. }
                )
            })
            .cloned()
            .collect()
    }

    // Known answers from p3_roll's layout (top key 69, 12 px rows, 16 px steps): a press on note
    // A (key 60, velocity 0.78125) at (56, 114) plays it; dragging it into key 61's row ends
    // that and plays 61; a click on empty grid at y 150 (key 57) draws there and plays 57 at a
    // step's velocity as it is let go of (a press there may become a box);
    // one on the key column at y 66 plays 64. With the strip's Preview switched off nothing
    // plays, and the setting says so.
    #[test]
    fn the_piano_roll_plays_the_notes_drawn_pressed_and_moved() {
        use dawsome_engine::edit::STEP_VELOCITY;
        let (mut app, _player) = p3_app();
        let press = |app: &mut App, x: f32, y: f32| {
            roll(
                app,
                RollMessage::GridPressed {
                    at: at(x, y),
                    button: mouse::Button::Left,
                    double: false,
                },
            );
        };
        press(&mut app, 56.0, 114.0);
        roll(&mut app, RollMessage::GridDragged(at(60.0, 104.0)));
        roll(&mut app, RollMessage::GridReleased(at(60.0, 104.0)));
        press(&mut app, 200.0, 150.0);
        roll(&mut app, RollMessage::GridReleased(at(200.0, 150.0)));
        press(&mut app, 10.0, 66.0);
        assert_eq!(
            live_notes_sent(&app),
            [
                Control::LiveNoteOn {
                    key: 60,
                    velocity: 0.78125
                },
                Control::LiveNoteOff { key: 60 },
                Control::LiveNoteOn {
                    key: 61,
                    velocity: 0.78125
                },
                Control::LiveNoteOff { key: 61 },
                Control::LiveNoteOn {
                    key: 57,
                    velocity: STEP_VELOCITY
                },
                Control::LiveNoteOff { key: 57 },
                Control::LiveNoteOn {
                    key: 64,
                    velocity: STEP_VELOCITY
                },
            ]
        );
        let _ = app.update(Message::ToggleNotePreview);
        assert!(!app.settings.preview_notes());
        let sent = live_notes_sent(&app).len();
        assert_eq!(
            live_notes_sent(&app).last(),
            Some(&Control::LiveNoteOff { key: 64 }),
            "switching off ends the note"
        );
        press(&mut app, 200.0, 162.0);
        assert_eq!(live_notes_sent(&app).len(), sent, "nothing plays while off");
    }

    // The hint bar names one control of each panel as the cursor comes onto it (the rack's
    // through its canvas's own hover report), and forgets it when the cursor leaves it, unless
    // another control already took its place.
    #[test]
    fn the_hint_bar_names_what_the_cursor_is_over_in_each_panel() {
        use crate::ui::hint::{Hover, HoverMessage, LibraryPart, RackPart, RollPart, Tool};
        use crate::ui::library::Section;
        use crate::ui::mixer_layout::Hit as MixerHit;
        use crate::ui::playlist_layout::{HeaderHit, RulerBand};
        use crate::ui::settings_layout::Hit as SettingHit;
        let (mut app, _player) = p3_app();
        // The computer keyboard switched off, as it was by default when this was written: on,
        // its line fills the bar while nothing is under the cursor.
        let _ = app.update(Message::ToggleTyping);
        let over = |app: &mut App, hover: Hover| {
            let _ = app.update(Message::Hover(HoverMessage::Over(hover)));
            app.hint_words()
        };
        assert_eq!(app.hint_words(), "");
        let rack = Hover::Rack {
            row: 1,
            part: RackPart::Step(4),
        };
        assert_eq!(
            over(&mut app, rack.clone()),
            "Step 5 \u{b7} off \u{b7} click switches it"
        );
        let panels = [
            (Hover::Tool(Tool::Play), "Play \u{b7} Space plays and stops"),
            (
                Hover::Roll(RollPart::Note {
                    key: 64,
                    velocity: 64,
                    edge: false,
                }),
                "E5 \u{b7} velocity 64 \u{b7} drag moves the selection; Shift+click adds; double-click deletes; right-click: notes menu",
            ),
            (
                Hover::Ruler(RulerBand::LoopStrip),
                "Loop range \u{b7} drag sets a loop range; drag its edges; right-click removes it",
            ),
            (
                Hover::Track(HeaderHit::AddTrack),
                "+ Track \u{b7} adds a track under the last",
            ),
            (
                Hover::Mixer(MixerHit::Fader(dawsome_engine::project::InsertId::MASTER)),
                "Fader \u{b7} 0.0 dB \u{b7} drag (Shift: fine); double-click resets; right-click to automate or link a controller",
            ),
            (
                Hover::Library(LibraryPart::Header(Section::Samples)),
                "Samples \u{b7} click opens or closes it",
            ),
        ];
        for (hover, words) in panels {
            assert_eq!(over(&mut app, hover), words);
        }
        // Leaving a control another already took the place of keeps the newer one.
        let samples = Hover::Library(LibraryPart::Header(Section::Samples));
        let _ = app.update(Message::Hover(HoverMessage::Left(rack)));
        assert!(app.hint_words().starts_with("Samples"));
        let _ = app.update(Message::Hover(HoverMessage::Left(samples)));
        assert_eq!(app.hint_words(), "");
        // Settings' controls are named on the Settings window's own foot
        // line, not here.
        let hints = Hover::Setting(SettingHit::ShowHints);
        assert_eq!(over(&mut app, hints.clone()), "");
        app.hints_flag = Some(true);
        assert_eq!(
            app.settings_hint(),
            "Show hints \u{b7} this line, naming what is under the cursor and what it does"
        );
        let _ = app.update(Message::Hover(HoverMessage::Left(hints)));
        legacy_keyboard(&mut app);
        let _ = app.update(Message::ToggleTyping);
        assert!(app.hint_words().starts_with("Typing keyboard on"));
    }

    // A channel's menu renames it (Enter takes the name, Esc keeps the old), clones it (the
    // copy selected) and deletes it with its notes, one undo step each; a right press on a
    // track's name opens its menu, whose Delete deletes the track with its clips, and one undo
    // puts them back.
    #[test]
    fn channels_rename_clone_and_delete_and_tracks_delete() {
        let (mut app, _player) = p4_app("p4_two_clips.json");
        let before = document(&app);
        let channel = |message| Message::Channel(message);
        let _ = app.update(channel(ChannelCommand::Rename(ChannelId(1))));
        let _ = app.update(channel(ChannelCommand::RenameEdited("Kick drum".into())));
        let _ = app.update(Message::ClearSelection);
        assert_eq!(document(&app), before, "Esc keeps the old name");
        let _ = app.update(channel(ChannelCommand::Rename(ChannelId(1))));
        let _ = app.update(channel(ChannelCommand::RenameEdited(" Kick drum ".into())));
        let _ = app.update(channel(ChannelCommand::RenameSubmitted));
        assert_eq!(
            app.session.project().channels[&ChannelId(1)].name,
            "Kick drum"
        );
        let _ = app.update(channel(ChannelCommand::Clone(ChannelId(2))));
        assert_eq!(app.session.project().channels[&ChannelId(3)].name, "hat #2");
        assert_eq!(app.selected_channel, Some(ChannelId(3)));
        let _ = app.update(channel(ChannelCommand::Delete(ChannelId(1))));
        assert!(!app.session.project().channels.contains_key(&ChannelId(1)));
        assert_eq!(app.session.history().undo_len(), 3);
        for _ in 0..3 {
            let _ = app.update(Message::Undo);
        }
        assert_eq!(document(&app), before);
        let name = at(40.0, 10.0);
        tracks_press(&mut app, (name.x, name.y), mouse::Button::Right, false);
        playlist(
            &mut app,
            PlaylistMessage::TrackMenu(crate::ui::hint::TrackRow::Delete),
        );
        assert!(!app
            .session
            .project()
            .playlist
            .tracks
            .contains_key(&dawsome_engine::project::TrackId(1)));
        assert!(
            app.session.project().playlist.clips.is_empty(),
            "its clips with it"
        );
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), before);
    }

    /// F1 in a window that shows its first-run notes, with a settings file of its own in a temp
    /// folder.
    fn first_run_app(dir: &Path) -> (App, Player) {
        let mut host = Host::offline();
        host.callouts = true;
        host.settings_file = Some(dir.join("dawsome/settings.json"));
        host.saves_settings = true;
        let project = project_file::open(Path::new(F1)).expect("opens").project;
        app_hosted(project, Path::new(F1), host)
    }

    // 6d K4: on a first start the keyboard's note waits for the window's first frame, then
    // shows; Got it puts it away for good, written to the settings. The
    // note over Record ("Arm, press Space, play") shows next, its Got it puts it away too, so
    // the next start shows none.
    #[test]
    fn the_keyboard_note_shows_after_the_first_frame_once_per_install() {
        let dir = std::env::temp_dir().join(format!("dawsome-callout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (mut app, _player) = first_run_app(&dir);
        assert_eq!(
            app.callout,
            Some(Callout::Due),
            "not before the first frame"
        );
        let _ = app.update(Message::Callout(CalloutMessage::Due));
        assert_eq!(app.callout, Some(Callout::Shown));
        assert_eq!(app.callout_note, CalloutNote::Keyboard);
        let _ = app.update(Message::Callout(CalloutMessage::GotIt));
        assert_eq!(
            (app.callout, app.callout_note),
            (Some(Callout::Shown), CalloutNote::Record)
        );
        assert_eq!(app.callout_lines()[0], "Arm (R), press Space, play.");
        let _ = app.update(Message::Callout(CalloutMessage::GotIt));
        assert_eq!(app.callout, None);
        let saved = std::fs::read_to_string(dir.join("dawsome/settings.json")).expect("saved");
        assert!(saved.contains(r#""keyboard_shown": true"#), "{saved}");
        assert!(saved.contains(r#""record_shown": true"#), "{saved}");
        let (again, _player) = first_run_app(&dir);
        assert_eq!(again.callout, None, "once per install");
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // Esc puts the note away and does nothing else; a press on the keyboard switch it points
    // at puts it away and switches the keyboard; Open Keys settings opens Settings on Keys. A
    // scripted run shows none unless --callouts on. Each time the note over
    // Record takes its place; Esc puts that one away too, as does a press on Record, which
    // arms it; then none shows.
    #[test]
    fn the_note_goes_on_esc_on_its_switch_or_to_the_keys_page() {
        use crate::ui::settings_layout::SettingsPage;
        let dir = std::env::temp_dir().join(format!("dawsome-callout2-{}", std::process::id()));
        let shown = |dir: &Path| {
            let _ = std::fs::remove_dir_all(dir);
            let (mut app, player) = first_run_app(dir);
            let _ = app.update(Message::Callout(CalloutMessage::Due));
            (app, player)
        };
        let record_next = |app: &App| app.callout_shows(CalloutNote::Record);
        let (mut app, _player) = shown(&dir);
        let _ = app.update(Message::ClearSelection);
        assert!(record_next(&app));
        assert!(app.panels.rack, "Esc closed nothing else");
        let _ = app.update(Message::ClearSelection);
        assert_eq!(app.callout, None);
        let (mut app, _player) = shown(&dir);
        let _ = app.update(Message::ToggleTyping);
        assert!(record_next(&app) && !app.typing.on);
        let _ = app.update(Message::ToggleTyping);
        assert!(
            record_next(&app),
            "the keyboard switch leaves Record's note"
        );
        let _ = app.update(Message::Record(RecordMessage::ToggleArm));
        assert_eq!((app.callout, app.recording.armed), (None, true));
        let (mut app, _player) = shown(&dir);
        let _ = app.update(Message::Callout(CalloutMessage::OpenKeys));
        assert!(record_next(&app));
        assert!(app.panels.settings);
        assert_eq!(app.settings_page, SettingsPage::Keys);
        std::fs::remove_dir_all(&dir).expect("removed");
        let (mut scripted, _player) = f1_app();
        assert_eq!(scripted.callout, None);
        scripted.set_callouts(true);
        assert_eq!(scripted.callout, Some(Callout::Due));
    }

    // 6d review 7: the note's first line names the keys of the layout saved: the piano row's on
    // a first start, the two rows' when the settings file chose them (Z X C V white, S D G H J
    // black, Q W E R an octave up, the octave moved by the buttons beside the switch).
    #[test]
    fn the_note_names_the_keys_of_the_layout_saved() {
        let dir = std::env::temp_dir().join(format!("dawsome-callout3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (app, _player) = first_run_app(&dir);
        assert_eq!(
            app.callout_lines()[0],
            "Your computer keyboard plays notes. A S D F\u{2026} white keys, W E T Y U black. Z and X move the octave."
        );
        let file = dir.join("dawsome/settings.json");
        std::fs::create_dir_all(file.parent().expect("a folder")).expect("folder made");
        std::fs::write(&file, r#"{"note_layout":"two-rows"}"#).expect("written");
        let (app, _player) = first_run_app(&dir);
        assert_eq!(app.typing.layout, crate::keys::NoteLayout::TwoRows);
        assert_eq!(
            app.callout_lines()[0],
            "Your computer keyboard plays notes. Z X C V\u{2026} white keys, S D G H J black; Q W E R\u{2026} an octave up. The + and \u{2212} beside the switch move the octave."
        );
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // A menu over a panel flips against the hint bar
    // while it shows, so it never covers the bar where its own rows' hints are written. The
    // case: p5_gain at 1000 x 400 with the rack alone and the hint bar on; channel 5's
    // menu, opened as a right press on its name does, is laid out above the bar (it ran to y
    // 392, over the bar from 378). With the bar hidden, it flips against the window's foot.
    #[test]
    fn a_menu_keeps_clear_of_the_hint_bar() {
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p5_gain.json"));
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 400.0)));
        app.open_channel_menu(ChannelId(5));
        for (hints, foot) in [(true, 400.0 - layout::HINT_BAR_HEIGHT), (false, 400.0)] {
            app.hints_flag = Some(hints);
            let mut window = widget_events::Window::new(&app);
            let menus = window.overlays(&app);
            let [menu] = menus[..] else {
                panic!("one menu: {menus:?}");
            };
            println!("hints {hints}: the menu at {menu:?}");
            assert!(
                menu.y >= 0.0 && menu.y + menu.height <= foot,
                "hints {hints}: {menu:?} past {foot}"
            );
        }
    }

    // The playlist's curve menu is drawn on the
    // window's top layer like the other panel menus, so the tracks' edge never cuts it: p8_ui's
    // automation clip (track 2, one segment, its handle at the bar's middle) with the playlist
    // alone in a window so short that the menu would pass the tracks' foot, the hint bar on;
    // the menu opened on the handle is laid out whole, above the hint bar.
    #[test]
    fn the_curve_menu_shows_whole_at_the_tracks_edge() {
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p8_ui.json"));
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        let _ = app.update(Message::TogglePanel(Panel::Rack));
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 260.0)));
        app.hints_flag = Some(true);
        assert!(app.open_curve_menu(dawsome_engine::project::AutomationId(1)));
        let mut window = widget_events::Window::new(&app);
        let menus = window.overlays(&app);
        let [menu] = menus[..] else {
            panic!("one menu on the window's top layer: {menus:?}");
        };
        let foot = 260.0 - layout::HINT_BAR_HEIGHT;
        println!("the curve menu at {menu:?}");
        assert!(
            menu.y >= 0.0 && menu.y + menu.height <= foot,
            "{menu:?} past {foot}"
        );
    }

    // A click on the note's words reaches nothing under
    // them. F1's rack at 1000 x 560 with the note shown over its 15th step: a click there
    // switches no step and changes nothing, where the same click with the note put away
    // switches the step.
    #[test]
    fn a_click_on_the_notes_words_reaches_nothing_under_them() {
        use crate::ui::callout::{bubble, BUBBLE_WIDTH};
        let dir = std::env::temp_dir().join(format!("dawsome-callout4-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (mut app, _player) = first_run_app(&dir);
        let _ = app.update(Message::Callout(CalloutMessage::Due));
        let mut window = widget_events::Window::new(&app);
        let corner = bubble(app.keyboard_switch(), app.window_size.width);
        let step = layout::rack_in_window(layout::step_rect(0, 14).center());
        assert!(
            (corner.x + 12.0..corner.x + BUBBLE_WIDTH - 12.0).contains(&step.x)
                && step.y > corner.y + 12.0,
            "the words lie over the step: {corner:?}, {step:?}"
        );
        let stepped = |published: &[Message]| {
            published
                .iter()
                .any(|message| matches!(message, Message::StepClicked { step: 14, .. }))
        };
        let before = app.session.project().clone();
        let published = window.click(&mut app, step);
        assert!(!stepped(&published), "{published:?}");
        assert_eq!(app.session.project(), &before);
        assert_eq!(app.callout, Some(Callout::Shown));
        let _ = app.update(Message::Callout(CalloutMessage::GotIt));
        assert!(
            stepped(&window.click(&mut app, step)),
            "the step lies under the words"
        );
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // The note's words name the keys as the layout and the bindings have them, and the first
    // shortcut the keyboard takes; it points at the toolbar's switch.
    #[test]
    fn the_note_says_which_keys_play_and_points_at_the_switch() {
        let (app, _player) = f1_app();
        let switch = app.keyboard_switch();
        assert!(switch.width > 0.0 && switch.y >= 0.0);
        assert!(switch.y + switch.height <= layout::TOOLBAR_HEIGHT);
        let taken = app.typing.taken_example(&app.keys);
        assert_eq!(
            taken.as_deref(),
            Some("L plays D6 now; Shift+L switches pattern/song")
        );
    }

    // Toolbar scale: at 1920 px, a full screen's 2259 px and 4510 px the controls have
    // grown to the most (1.6), and each takes its click where the toolbar's spread lays it out:
    // the middle of Play, Record, the Audio side of the switch beside it, the mixer's panel
    // button, the keyboard switch, File and Edit each publishes what that control does, and
    // the cursor coming onto the mixer's button tells the hint bar so; a click in a gap, between
    // the keyboard's group and the line after it, publishes nothing. The first-run notes ring
    // the grown keyboard switch and Record, and a click in the middle of Record's ring arms it.
    // At 2259 and 4510 px the groups stand left, 45 px apart, and at each width a
    // click in the notices' cell past their text and Retry, halfway from there to File (the first
    // of the menus since 2026-10-27; Edit was),
    // publishes nothing too.
    #[test]
    fn the_grown_toolbars_controls_take_their_clicks_where_it_lays_them_out() {
        use crate::ui::hint::{Hover, HoverMessage, Tool};
        use crate::ui::toolbar::{panel_button_rects, retry_edit_file_and_help, Group};
        for width in [1920.0, 2259.0, 4510.0] {
            let (mut app, _player) = f1_app();
            let _ = app.update(Message::WindowResized(Size::new(width, 560.0)));
            let spread = app.toolbar().spread();
            assert_eq!(spread.sizes().scale, 1.6, "{width} px");
            let y = layout::TOOLBAR_HEIGHT / 2.0;
            // At 0.9 of the size (asked 2026-10-27): 38 → 61 px buttons (were 42 → 67),
            // the Audio row y 23..37 (9 + 14; was 26..42), 16 + 2 × 6 px panel buttons (were
            // 19 + 2 × 6).
            let play = spread.left(Group::Transport).expect("shown") + 61.0 / 2.0;
            let record = spread.record_button();
            assert_eq!(record.width, 61.0, "the grown Record");
            let audio = Point::new(spread.take_column_x() + 20.0, 30.0);
            let (_, mixer) = panel_button_rects(&spread)
                .into_iter()
                .find(|&(panel, _)| panel == Panel::Mixer)
                .expect("the panel buttons show");
            assert_eq!(mixer.width, 28.0, "the grown panel button");
            let switch = spread.keyboard_switch();
            let [_, edit, file, help] = retry_edit_file_and_help(&spread);
            let menus_start = file.start;
            let middle =
                |range: std::ops::Range<f32>| Point::new((range.start + range.end) / 2.0, y);
            type Wanted = fn(&Message) -> bool;
            let clicks: [(&str, Point, Wanted); 8] = [
                ("Play", Point::new(play, y), |m| matches!(m, Message::Play)),
                ("Record", record.center(), |m| {
                    matches!(m, Message::Record(RecordMessage::ToggleArm))
                }),
                ("Audio", audio, |m| {
                    matches!(m, Message::Record(RecordMessage::Kind(TakeKind::Audio)))
                }),
                ("the mixer's button", mixer.center(), |m| {
                    matches!(m, Message::TogglePanel(Panel::Mixer))
                }),
                ("Keyboard", switch.center(), |m| {
                    matches!(m, Message::ToggleTyping)
                }),
                ("File", middle(file), |m| {
                    matches!(m, Message::File(FileMessage::ToggleMenu))
                }),
                ("Edit", middle(edit), |m| {
                    matches!(m, Message::ToggleEditMenu)
                }),
                ("Help", middle(help), |m| matches!(m, Message::ToggleHelp)),
            ];
            let mut window = widget_events::Window::new(&app);
            for (what, at, wanted) in clicks {
                let published = window.click(&mut app, at);
                println!("{width} px, {what} at {at:?}: {published:?}");
                assert!(
                    published.iter().any(wanted),
                    "{width} px, {what}: {published:?}"
                );
                app.files.menu = false;
                app.edit_menu = false;
            }
            let keyboard_end = spread.left(Group::Keyboard).expect("shown") + spread.groups()[5].1;
            let gap = Point::new(keyboard_end + 10.0, y);
            let published = window.click(&mut app, gap);
            assert!(published.is_empty(), "{width} px, the gap: {published:?}");
            let panels_end = spread.span(Group::Panels).expect("shown").end;
            // At 1920 px the notices of a window with no audio device and Retry leave the gaps
            // less than that to share.
            if width > 2000.0 {
                assert_eq!(
                    keyboard_end + 45.0,
                    spread.left(Group::Panels).expect("shown"),
                    "{width} px: the groups 45 px apart"
                );
            }
            let notices_end = spread
                .span(Group::Status)
                .map_or(panels_end, |status| status.end);
            let cell = Point::new((notices_end + menus_start) / 2.0, y);
            let published = window.click(&mut app, cell);
            assert!(published.is_empty(), "{width} px, the cell: {published:?}");
            let hovered = window.mouse(
                &mut app,
                mouse::Event::CursorMoved {
                    position: mixer.center(),
                },
                mouse::Cursor::Available(mixer.center()),
            );
            assert!(
                hovered.iter().any(|m| matches!(
                    m,
                    Message::Hover(HoverMessage::Over(Hover::Tool(Tool::Panel(Panel::Mixer))))
                )),
                "{width} px: {hovered:?}"
            );
            for (note, anchor) in [
                (CalloutNote::Keyboard, switch),
                (CalloutNote::Record, record),
            ] {
                app.callout_note = note;
                assert_eq!(app.callout_anchor().0, anchor, "{width} px");
            }
            app.callout = Some(Callout::Shown);
            let mut window = widget_events::Window::new(&app);
            let armed = app.recording.armed;
            let published = window.click(&mut app, record.center());
            assert!(
                published
                    .iter()
                    .any(|m| matches!(m, Message::Record(RecordMessage::ToggleArm))),
                "{width} px, Record under its note: {published:?}"
            );
            assert_ne!(app.recording.armed, armed);
        }
    }

    // Toolbar scale: F1's window (or the project DAWSOME_UI_PROJECT names: a pattern's long
    // name) drawn offscreen at each width DAWSOME_UI_WIDTHS names (880, 1000, 1400, 1920, 2259,
    // 2560 and 4510 by default), 160 px tall, into DAWSOME_UI_OUT as toolbar_<width>.png with no
    // notice and toolbar_<width>_notice.png with a long one. Why: the app fits its window to its
    // monitor, so a bar wider than any monitor here (a full screen's 2259 px, 2560, 4510) is
    // looked at this way.
    #[test]
    #[ignore = "writes PNGs to look at: DAWSOME_UI_OUT=<folder>"]
    fn toolbar_shots() {
        let out = std::env::var_os("DAWSOME_UI_OUT").expect("DAWSOME_UI_OUT names a folder");
        let widths = std::env::var("DAWSOME_UI_WIDTHS")
            .unwrap_or_else(|_| "880,1000,1400,1920,2259,2560,4510".to_owned());
        let (mut app, _player) = match std::env::var_os("DAWSOME_UI_PROJECT") {
            Some(project) => app_on(Path::new(&project)),
            None => f1_app(),
        };
        for width in widths.split(',') {
            let width: f32 = width.trim().parse().expect("a width");
            let _ = app.update(Message::WindowResized(Size::new(width, 160.0)));
            for (suffix, error) in [
                ("", None),
                (
                    "_notice",
                    Some("sample samples/an-ambient-pad.wav cannot be read; it plays silence"),
                ),
            ] {
                app.audio_problem = None;
                app.error = error.map(str::to_owned);
                let shot = widget_events::Window::new(&app).snapshot(&app);
                let path = Path::new(&out).join(format!("toolbar_{width}{suffix}.png"));
                crate::headless::save_png(&path, &shot).expect("saved");
                println!("{}", path.display());
            }
        }
    }
}
