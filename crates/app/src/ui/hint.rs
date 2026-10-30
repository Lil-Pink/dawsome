//! The hint bar's words: what the cursor is over ([`Hover`], which each canvas works out with
//! its own hit test and each toolbar widget says on entering), its value, and what pressing,
//! dragging or right-clicking it does: "Volume · 80% · drag up or down; right-click to automate
//! or link a controller". One table, [`words`], so every control's hint is in one place and a
//! test walks every hit each panel's hit test gives.

use std::collections::BTreeMap;

use dawsome_engine::dsp::laws::fader_gain;
use dawsome_engine::project::{ChannelKind, ClipId, InsertId, PatternId, ProcessorOwner, Project};
use dawsome_engine::time::MilliBpm;

use crate::app::SelectionLink;
use crate::keys::{Action, Keys};
use crate::ui::instrument_window::{Control, Flag};
use crate::ui::key_names::key_name;
use crate::ui::keys_page::KeysHit;
use crate::ui::layout::HeaderHit;
use crate::ui::library::{Port, Section};
use crate::ui::mixer_layout::Hit as MixerHit;
use crate::ui::mixer_state::{fader_text, pan_text, percent_text};
use crate::ui::panels::Panel;
use crate::ui::playlist_layout::{ClipPart, HeaderHit as TrackHit, RulerBand};
use crate::ui::rack_rows::{RackRow, RackRows};
use crate::ui::sample_editor::{EditorOp, EditorPart};
use crate::ui::settings_layout::Hit as SettingHit;
use crate::ui::step_grid::{step_content, StepContent, StepGrid};
use crate::ui::toolbar::panel_tip;

/// The dot between a hint's parts.
const DOT: &str = " \u{b7} ";

/// What a Follow switch does, the piano roll's and the playlist's.
const FOLLOW_WORDS: &str =
    "while playing, the view jumps a page when the playhead leaves it; click switches it on or off";

/// What the cursor is over.
#[derive(Debug, Clone, PartialEq)]
pub enum Hover {
    Tool(Tool),
    /// The rack header's controls.
    RackHeader(HeaderHit),
    /// A control of the rack's row `row`.
    Rack {
        row: usize,
        part: RackPart,
    },
    /// The "+" under the rack's rows.
    RackAdd,
    Roll(RollPart),
    Ruler(RulerBand),
    Track(TrackHit),
    Clip {
        clip: ClipId,
        part: ClipPart,
    },
    /// An empty part of a playlist lane.
    Lane,
    PlaylistTool(PlaylistTool),
    Mixer(MixerHit),
    /// The divider between two stacked panels, `upper` over `lower` (the mixer's top edge is
    /// one).
    Divider {
        upper: Panel,
        lower: Panel,
    },
    Library(LibraryPart),
    Setting(SettingHit),
    /// A control of Settings' Keys page.
    Keys(KeysHit),
    /// An output device Settings offers, by the words of its row.
    Output(String),
    /// An input device Settings offers new input channels, by the words of its row.
    Input(String),
    /// A row of the File, Edit or channel menu.
    Menu(MenuRow),
    /// A row or control of the plugin picker or the "+ Instrument…" picker.
    Picker(PickerPart),
    /// A control of an audio clip's properties strip in the playlist's header.
    ClipControl(ClipControl),
    /// A control of the instrument window.
    Sound(SoundPart),
    /// A control of the sample editor.
    SampleEditor(EditorPart),
    /// A view's scroll bar along time (the piano roll's, the playlist's, the rack's steps').
    ScrollBar,
    /// A control of the piano roll's Quantize dialog.
    Quantize(QuantizePart),
}

impl Hover {
    /// Whether it is something of the Settings window's, which names it on its own foot line
    /// (the main window's hint bar leaves it out, as it leaves the instrument window's).
    pub fn in_settings(&self) -> bool {
        matches!(
            self,
            Hover::Setting(_) | Hover::Keys(_) | Hover::Output(_) | Hover::Input(_)
        )
    }
}

/// A control of the Quantize dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantizePart {
    Grid,
    Strength,
    Ends,
    Apply,
}

impl QuantizePart {
    /// Every control the dialog has, for the hint table's test.
    #[cfg(test)]
    pub const ALL: [QuantizePart; 4] = [
        QuantizePart::Grid,
        QuantizePart::Strength,
        QuantizePart::Ends,
        QuantizePart::Apply,
    ];
}

/// A row of a menu. Each menu's view builds its rows from its list (`ALL`), so every row it
/// shows is one the hint table's test walks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuRow {
    File(FileRow),
    Edit(EditRow),
    Channel(ChannelRow),
    Track(TrackRow),
    Notes(NoteRow),
    Record(RecordRow),
    Clip(ClipRow),
}

/// A row of a Record menu: Notes' (a right press on Record while it records notes, or on the
/// switch's Notes) or Audio's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordRow {
    Notes,
    Audio,
    Quantize,
    Capture,
    Master,
    LineIn,
    CountIn,
}

impl RecordRow {
    /// Every row, as the hint table's test walks them.
    #[cfg(test)]
    pub const ALL: [RecordRow; 7] = [
        RecordRow::Notes,
        RecordRow::Audio,
        RecordRow::Quantize,
        RecordRow::Capture,
        RecordRow::Master,
        RecordRow::LineIn,
        RecordRow::CountIn,
    ];

    /// Its name and what it does, in the hint bar.
    fn parts(self) -> (&'static str, &'static str) {
        match self {
            RecordRow::Notes => (
                "Record notes",
                "takes record the notes played into the piano roll: in pattern mode over the pattern as it loops (they add; delete first to replace), in song mode into a new pattern that grows as you play",
            ),
            RecordRow::Audio => (
                "Record audio",
                "takes record what you hear, or the line in, into a sound file: one pass of the pattern, or until Stop in song mode",
            ),
            RecordRow::Quantize => (
                "Quantize notes to snap",
                "a recorded note starts on the nearest snap line; its length stays as played",
            ),
            RecordRow::Capture => (
                "Capture",
                "the notes played in the last 30 s, while not recording, become notes: one undo",
            ),
            RecordRow::Master => (
                "What you hear (master)",
                "takes record the master into the project's recordings, \"<project> take N.wav\"; Undo takes the clip away and keeps the file",
            ),
            RecordRow::LineIn => (
                "Line in",
                "takes record the selected external input channel's device, else the default input",
            ),
            RecordRow::CountIn => (
                "Count-in",
                "a bar of clicks before a take starts",
            ),
        }
    }
}

/// A row of the piano roll's notes menu (a right press on the grid, or the strip's Notes
/// button).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteRow {
    Cut,
    Copy,
    Paste,
    Duplicate,
    Delete,
    Quantize,
    Chop,
    Glue,
    Legato,
    Reverse,
    Invert,
    Humanize,
}

impl NoteRow {
    pub const ALL: [NoteRow; 12] = [
        NoteRow::Cut,
        NoteRow::Copy,
        NoteRow::Paste,
        NoteRow::Duplicate,
        NoteRow::Delete,
        NoteRow::Quantize,
        NoteRow::Chop,
        NoteRow::Glue,
        NoteRow::Legato,
        NoteRow::Reverse,
        NoteRow::Invert,
        NoteRow::Humanize,
    ];

    /// Its words in the menu.
    pub fn label(self) -> &'static str {
        match self {
            NoteRow::Cut => "Cut",
            NoteRow::Copy => "Copy",
            NoteRow::Paste => "Paste",
            NoteRow::Duplicate => "Duplicate",
            NoteRow::Delete => "Delete",
            NoteRow::Quantize => "Quantize\u{2026}",
            NoteRow::Chop => "Chop at snap",
            NoteRow::Glue => "Glue",
            NoteRow::Legato => "Legato",
            NoteRow::Reverse => "Reverse",
            NoteRow::Invert => "Invert",
            NoteRow::Humanize => "Humanize",
        }
    }

    /// What it does, in the hint bar.
    pub fn does(self) -> &'static str {
        match self {
            NoteRow::Cut => "takes the selected notes out to the clipboard",
            NoteRow::Copy => "copies the selected notes to the clipboard",
            NoteRow::Paste => "puts the clipboard's notes at the playhead or the last click",
            NoteRow::Duplicate => "copies the notes after their end",
            NoteRow::Delete => "takes the selected notes out",
            NoteRow::Quantize => "moves the notes toward a grid, as far as you choose",
            NoteRow::Chop => "cuts the notes at every snap line",
            NoteRow::Glue => "joins touching notes of one key",
            NoteRow::Legato => "makes each note end where the next starts",
            NoteRow::Reverse => "mirrors the notes in time",
            NoteRow::Invert => "mirrors the notes' keys",
            NoteRow::Humanize => "moves the notes and their velocities a little at random",
        }
    }
}

/// A row of a playlist track's menu (a right press on its name).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackRow {
    Rename,
    Delete,
}

impl TrackRow {
    pub const ALL: [TrackRow; 2] = [TrackRow::Rename, TrackRow::Delete];
}

/// A row of a pattern clip's menu (Ctrl+right-click on the clip): it shows the rows that apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipRow {
    /// "Unlink (make unique)": shown while other clips play the clip's pattern too.
    Unlink,
    /// "Link back to <name>": shown while the clip plays a copy of a pattern that still exists.
    LinkBack,
}

impl ClipRow {
    /// Every row, as the hint table's test walks them.
    #[cfg(test)]
    pub const ALL: [ClipRow; 2] = [ClipRow::Unlink, ClipRow::LinkBack];
}

/// A row of the File menu; `Recent` stands for each recent file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRow {
    New,
    NewEmpty,
    Open,
    OpenDemo,
    ImportAudio,
    ImportMidi,
    Recent,
    Save,
    SaveAs,
    ExportWav,
    ExportMp3,
    ExportMidi,
    ExportSongMidi,
    ExportStems,
    ExportDawproject,
}

impl FileRow {
    pub const ALL: [FileRow; 15] = [
        FileRow::New,
        FileRow::NewEmpty,
        FileRow::Open,
        FileRow::OpenDemo,
        FileRow::ImportAudio,
        FileRow::ImportMidi,
        FileRow::Recent,
        FileRow::Save,
        FileRow::SaveAs,
        FileRow::ExportWav,
        FileRow::ExportMp3,
        FileRow::ExportMidi,
        FileRow::ExportSongMidi,
        FileRow::ExportStems,
        FileRow::ExportDawproject,
    ];
}

/// A row of the Edit menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditRow {
    Undo,
    Redo,
    Delete,
    Duplicate,
    SelectAll,
    Settings,
}

impl EditRow {
    pub const ALL: [EditRow; 6] = [
        EditRow::Undo,
        EditRow::Redo,
        EditRow::Delete,
        EditRow::Duplicate,
        EditRow::SelectAll,
        EditRow::Settings,
    ];
}

/// A row of a channel's menu (a right-click on a channel's name); each kind's menu shows the
/// rows [`ChannelRow::shown_for`] gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelRow {
    InstrumentWindow,
    /// Plugin channels only: the plugin's own window, as a double-click on the name opens it.
    PluginWindow,
    /// Plugin channels only: the parameter window.
    Parameters,
    /// Every kind: an insert of the channel's own, the first free one, and the mixer opened
    /// there.
    MixerInsert,
    /// Samplers only.
    ChangeSample,
    /// Samplers and audio channels (those that play a file): the sample editor on its sound.
    EditSample,
    /// The `cut_itself` switch, ticked when on: samplers and sets, the kinds with sound
    /// settings.
    OneNoteAtATime,
    Rename,
    Clone,
    Delete,
}

impl ChannelRow {
    pub const ALL: [ChannelRow; 10] = [
        ChannelRow::InstrumentWindow,
        ChannelRow::PluginWindow,
        ChannelRow::Parameters,
        ChannelRow::MixerInsert,
        ChannelRow::ChangeSample,
        ChannelRow::EditSample,
        ChannelRow::OneNoteAtATime,
        ChannelRow::Rename,
        ChannelRow::Clone,
        ChannelRow::Delete,
    ];

    /// Whether a channel of `kind`'s menu shows it.
    pub fn shown_for(self, kind: ChannelKind) -> bool {
        match self {
            ChannelRow::PluginWindow | ChannelRow::Parameters => kind == ChannelKind::Plugin,
            ChannelRow::ChangeSample => kind == ChannelKind::Sampler,
            ChannelRow::EditSample => {
                matches!(kind, ChannelKind::Sampler | ChannelKind::Audio)
            }
            ChannelRow::OneNoteAtATime => {
                matches!(kind, ChannelKind::Sampler | ChannelKind::Instrument)
            }
            ChannelRow::InstrumentWindow
            | ChannelRow::MixerInsert
            | ChannelRow::Rename
            | ChannelRow::Clone
            | ChannelRow::Delete => true,
        }
    }
}

/// What the cursor is over in a picker: a kind of row of its list, or one of its controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerPart {
    Plugin,
    Set,
    KitSound,
    SampleFile,
    /// A section's title.
    Title,
    /// The files that could not be loaded, and their heading.
    Failed,
    Filter,
    Rescan,
    Close,
}

impl PickerPart {
    /// The picker's controls beside its list's rows (whose hovers its list's hit test gives).
    #[cfg(test)]
    pub const CONTROLS: [PickerPart; 3] =
        [PickerPart::Filter, PickerPart::Rescan, PickerPart::Close];
}

/// A control of an audio clip's properties strip. The strip builds its controls from
/// [`ClipControl::ALL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipControl {
    NoStretch,
    Resample,
    Stretch,
    PitchDown,
    PitchUp,
    Close,
}

impl ClipControl {
    pub const ALL: [ClipControl; 6] = [
        ClipControl::NoStretch,
        ClipControl::Resample,
        ClipControl::Stretch,
        ClipControl::PitchDown,
        ClipControl::PitchUp,
        ClipControl::Close,
    ];
}

/// A control of the instrument window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundPart {
    Close,
    Gate,
    Compare,
    /// A pad or key of the sound map.
    Key(u8),
    /// Edit sample…: the sample editor on the channel's sound (samplers and audio channels).
    EditSample,
    Control(Control),
    Flag(Flag),
    More,
    OpenPlugin,
    /// The output chooser: the mixer insert the channel plays into.
    Insert,
    /// An input channel's device picker.
    InputDevice,
    /// The input devices listed again.
    InputRefresh,
    /// An input channel's channel picker.
    InputChannels,
    /// An input channel's meter.
    InputLevel,
    /// The MIDI OUT group's port picker.
    MidiOutPort,
    /// The MIDI OUT group's MIDI channel picker.
    MidiOutChannel,
    /// The MIDI OUT group's program picker.
    MidiOutProgram,
    /// The MIDI OUT group's latency slider.
    MidiOutLatency,
    /// The MIDI OUT group's Echo switch.
    MidiOutEcho,
}

impl SoundPart {
    /// Every control of the window, as the hint table's test walks them.
    #[cfg(test)]
    pub fn all() -> Vec<SoundPart> {
        let mut parts = vec![
            SoundPart::Close,
            SoundPart::Gate,
            SoundPart::Compare,
            SoundPart::Key(60),
            SoundPart::EditSample,
            SoundPart::Flag(Flag::Reverse),
            SoundPart::Flag(Flag::Filter),
            SoundPart::More,
            SoundPart::OpenPlugin,
            SoundPart::Insert,
            SoundPart::InputDevice,
            SoundPart::InputRefresh,
            SoundPart::InputChannels,
            SoundPart::InputLevel,
            SoundPart::MidiOutPort,
            SoundPart::MidiOutChannel,
            SoundPart::MidiOutProgram,
            SoundPart::MidiOutLatency,
            SoundPart::MidiOutEcho,
        ];
        parts.extend(Control::ALL.map(SoundPart::Control));
        parts
    }
}

/// A toolbar control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Play,
    Stop,
    Record,
    /// The switch beside Record: Notes, Audio.
    TakeNotes,
    TakeAudio,
    Capture,
    Metronome,
    Tempo,
    PreviousPattern,
    PatternName,
    NextPattern,
    PatternMode,
    SongMode,
    Loop,
    Position,
    OctaveDown,
    Keys,
    OctaveUp,
    Panel(Panel),
    /// The notices' line.
    Notices,
    Retry,
    Edit,
    File,
    /// The Help window's button, right of File and Edit.
    Help,
}

impl Tool {
    /// Every toolbar control, as the hint table's test walks them.
    #[cfg(test)]
    pub fn all() -> Vec<Tool> {
        let mut tools = vec![
            Tool::Play,
            Tool::Stop,
            Tool::Record,
            Tool::TakeNotes,
            Tool::TakeAudio,
            Tool::Capture,
            Tool::Metronome,
            Tool::Tempo,
            Tool::PreviousPattern,
            Tool::PatternName,
            Tool::NextPattern,
            Tool::PatternMode,
            Tool::SongMode,
            Tool::Loop,
            Tool::Position,
            Tool::OctaveDown,
            Tool::Keys,
            Tool::OctaveUp,
            Tool::Notices,
            Tool::Retry,
            Tool::Edit,
            Tool::File,
            Tool::Help,
        ];
        tools.extend(Panel::ALL.map(Tool::Panel));
        tools
    }
}

/// A control of a rack row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RackPart {
    Mute,
    Name,
    Pan,
    Volume,
    Step(u32),
}

/// What the piano roll's canvases have under the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollPart {
    /// A note of `key` at `velocity` on the 0..=128 scale; `edge` on its right edge.
    Note {
        key: u8,
        velocity: u8,
        edge: bool,
    },
    /// Empty grid on `key`'s row.
    Empty {
        key: u8,
    },
    /// The key column's `key`.
    Keys {
        key: u8,
    },
    /// A velocity stem.
    Stem,
    Snap,
    Preview,
    /// The drag handle between the grid and the velocity lane.
    LaneHandle,
    /// The strip's Follow switch.
    Follow,
    /// The strip's Notes button: the notes' menu.
    NotesMenu,
    /// The strip's light while a take of notes runs.
    RecNotes,
    /// The strip's scale chooser, its root and the Fold switch.
    Scale,
    ScaleRoot,
    Fold,
}

/// The playlist header's controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistTool {
    Snap,
    ZoomOut,
    ZoomIn,
    /// The header's Follow switch.
    Follow,
    /// The header's Fit switch: the lanes fit the tracks into the panel.
    Fit,
    /// The header's link control, right of Fit, while selected pattern clips share their
    /// patterns or play copies (`Context::link`).
    Link,
}

/// What a Library row is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryPart {
    Header(Section),
    /// A starter kit sound, an instrument set or a plugin instrument, by name.
    Sound(String),
    Set(String),
    /// A plugin instrument, by name, and what its row's hint says of its format
    /// (`browser::plugin_note`).
    Plugin {
        name: String,
        note: Option<&'static str>,
    },
    /// "External input".
    ExternalInput,
    /// A group's label inside Instruments ("Plugins", a kind, a maker), by its label.
    Group(String),
    Rescan,
    Folder,
    File(String),
    AddFolder,
    /// A MIDI device's row, as the Library lists it (its light left out).
    Device(Port),
    /// A MIDI device's tick.
    Tick(Port),
}

/// What the words need to know beyond the hover itself.
pub struct Context<'a> {
    pub project: &'a Project,
    /// The pattern the rack shows.
    pub pattern: Option<PatternId>,
    /// The mixer's selected insert, whose sends the send knobs show.
    pub insert: InsertId,
    /// The loop range's state: on, off, or none.
    pub loop_on: Option<bool>,
    /// Whether the piano roll plays the notes drawn.
    pub preview: bool,
    /// The typing keyboard's bottom octave, "C4".
    pub octave: String,
    /// What the computer keyboard plays: "A is C5, velocity 100", or "off".
    pub keyboard: String,
    /// The shortcuts, whose keys the hints name.
    pub keys: &'a Keys,
    /// The toolbar's notices whole, as one line; none while there are none.
    pub notices: Option<String>,
    /// How the output runs when that is worth knowing: "Resampling 44100 → 48000 Hz".
    pub resampling: Option<&'a str>,
    /// The rack's Recordings header is open: its audio takes are rows.
    pub takes_open: bool,
    /// How the playlist's selected pattern clips share their patterns: what the header's link
    /// control shows.
    pub link: Option<SelectionLink>,
    /// How late each plugin that says so plays its sound, in frames, by `ProcessorOwner`.
    pub latencies: BTreeMap<ProcessorOwner, u32>,
}

/// What a plugin's hint value says of its format: "VST3", or "VST3 · delays its sound by N
/// frames (M ms); not compensated" when it says so, for `owner`'s VST3 plugin; nothing for any
/// other.
fn plugin_value(context: &Context, owner: ProcessorOwner) -> Option<String> {
    let project = context.project;
    let reference = match owner {
        ProcessorOwner::Channel(id) => project.channels.get(&id)?.processor.as_ref()?,
        ProcessorOwner::Effect(id) => &project.mixer.effect(id)?.1.processor,
    };
    if reference.format() != Some("vst3") {
        return None;
    }
    Some(match context.latencies.get(&owner) {
        Some(&frames) if frames > 0 => format!(
            "VST3{DOT}{}",
            crate::ui::param_window::latency_text(frames, project.sample_rate)
        ),
        _ => "VST3".to_owned(),
    })
}

/// The hint for `hover`: its name, its value where it has one, and what it does, a dot apart.
pub fn words(hover: &Hover, context: &Context) -> String {
    let parts = parts(hover, context);
    parts.into_iter().flatten().collect::<Vec<_>>().join(DOT)
}

/// The hint's name, value and gestures.
fn parts(hover: &Hover, context: &Context) -> [Option<String>; 3] {
    let three = |name: &str, value: Option<String>, does: &str| {
        [Some(name.to_owned()), value, Some(does.to_owned())]
    };
    let project = context.project;
    match hover {
        Hover::Tool(tool) => tool_parts(*tool, context),
        Hover::RackHeader(HeaderHit::BarsDown) => {
            three("Bars", bars(context), "shortens the pattern by a bar")
        }
        Hover::RackHeader(HeaderHit::BarsUp) => {
            three("Bars", bars(context), "lengthens the pattern by a bar")
        }
        Hover::RackHeader(HeaderHit::AddSampler) | Hover::RackAdd => three(
            "+ Instrument\u{2026}",
            None,
            "adds an instrument set, a starter kit sound, a plugin or a sound file of yours",
        ),
        Hover::Rack { row, part } => {
            let rows = RackRows::of(project, context.takes_open);
            if let Some(RackRow::Takes { count, open }) = rows.get(*row) {
                let takes = if count == 1 {
                    "1 audio take".to_owned()
                } else {
                    format!("{count} audio takes")
                };
                let does = if open {
                    "click folds them away; each take's file is in the Library under Recordings"
                } else {
                    "click shows them; each take's file is in the Library under Recordings"
                };
                return three(crate::ui::rack::TAKES_HEADER, Some(takes), does);
            }
            let Some((id, channel)) = rows
                .channel(*row)
                .and_then(|id| Some((id, project.channels.get(&id)?)))
            else {
                return [None, None, None];
            };
            match part {
                RackPart::Mute => three(
                    "Mute",
                    Some(on_off(channel.mute).to_owned()),
                    "click mutes or unmutes the channel",
                ),
                RackPart::Name => {
                    let does = match channel.kind {
                        ChannelKind::Sampler if channel.sample.as_os_str().is_empty() => {
                            "click: select \u{b7} double-click: choose its sound \u{b7} right-click: menu"
                        }
                        ChannelKind::Sampler | ChannelKind::Instrument => {
                            "click: select \u{b7} double-click: instrument window \u{b7} right-click: menu"
                        }
                        ChannelKind::Plugin => {
                            "click: select \u{b7} double-click: its plugin's window \u{b7} right-click: menu (its instrument window there)"
                        }
                        ChannelKind::Audio => {
                            "click: select \u{b7} double-click: instrument window \u{b7} its clips play its file \u{b7} right-click: menu"
                        }
                        ChannelKind::Input => {
                            "click: select \u{b7} double-click: instrument window (its device and channels) \u{b7} right-click: menu"
                        }
                    };
                    // A channel playing a VST3 plugin says so (and its latency), and a channel
                    // that plays one note at a time (its menu's switch) says that.
                    let one = channel
                        .sound
                        .cut_itself
                        .then(|| "one note at a time".to_owned());
                    let said: Vec<String> = plugin_value(context, ProcessorOwner::Channel(id))
                        .into_iter()
                        .chain(one)
                        .collect();
                    let value = (!said.is_empty()).then(|| said.join(DOT));
                    three(&channel.name, value, does)
                }
                RackPart::Pan => three(
                    "Pan",
                    Some(pan_text(channel.pan)),
                    "drag up or down; right-click to automate or link a controller",
                ),
                RackPart::Volume => three(
                    "Volume",
                    Some(percent_text(channel.volume)),
                    "drag up or down; right-click to automate or link a controller",
                ),
                RackPart::Step(step) => {
                    let lit = StepGrid::new(project, context.pattern).is_lit(id, *step);
                    let name = format!("Step {}", step + 1);
                    let content = context
                        .pattern
                        .map(|pattern| step_content(project, pattern, id, *step));
                    match content {
                        Some(StepContent::Melody { default: true }) => three(
                            &name,
                            Some("a chord".to_owned()),
                            &format!(
                                "click removes only its {}; the piano roll changes the rest",
                                key_name(dawsome_engine::project::DEFAULT_KEY)
                            ),
                        ),
                        Some(StepContent::Melody { default: false }) => three(
                            &name,
                            Some("a chord".to_owned()),
                            "the piano roll changes it",
                        ),
                        _ => three(&name, Some(on_off(lit).to_owned()), "click switches it"),
                    }
                }
            }
        }
        Hover::Roll(part) => roll_parts(*part, context),
        Hover::Ruler(RulerBand::Numbers) => three(
            "Bars",
            None,
            "click sets where the song plays from; Stop comes back there",
        ),
        Hover::Ruler(RulerBand::LoopStrip) => three(
            "Loop range",
            None,
            "drag sets a loop range; drag its edges; right-click removes it",
        ),
        Hover::Track(hit) => track_parts(*hit, project),
        Hover::Clip { clip, part } => clip_parts(*clip, *part, context),
        Hover::Lane => three(
            "Lane",
            None,
            "click places the selected pattern; Ctrl+drag selects clips",
        ),
        Hover::PlaylistTool(PlaylistTool::Snap) => {
            three("Snap", None, "what clips land on when placed or moved")
        }
        Hover::PlaylistTool(PlaylistTool::ZoomOut) => {
            three("Zoom out", None, "shows more bars; Ctrl+wheel zooms too")
        }
        Hover::PlaylistTool(PlaylistTool::ZoomIn) => {
            three("Zoom in", None, "shows fewer bars; Ctrl+wheel zooms too")
        }
        Hover::PlaylistTool(PlaylistTool::Follow) => three("Follow", None, FOLLOW_WORDS),
        Hover::PlaylistTool(PlaylistTool::Fit) => three(
            "Fit",
            None,
            "lit: the tracks share the playlist's height; Alt+wheel over them sets a height",
        ),
        Hover::PlaylistTool(PlaylistTool::Link) => link_parts(context),
        Hover::Mixer(hit) => mixer_parts(*hit, context),
        Hover::Divider { upper, lower } => [
            Some(format!("{} / {}", upper.title(), lower.title())),
            None,
            Some("drag to resize the panels above and below; double-click resets".to_owned()),
        ],
        Hover::Library(part) => library_parts(part, context.keys),
        Hover::Setting(hit) => setting_parts(*hit),
        Hover::Keys(hit) => keys_parts(*hit, context.keys),
        Hover::Menu(row) => menu_parts(*row, context.keys),
        Hover::Picker(part) => picker_parts(*part),
        Hover::ClipControl(control) => clip_control_parts(*control),
        Hover::Output(device) => [
            Some("Audio output".to_owned()),
            Some(device.clone()),
            Some("click plays through it from the next Play".to_owned()),
        ],
        Hover::Input(device) => [
            Some("Audio input".to_owned()),
            Some(device.clone()),
            Some("click: a new input channel plays it (the Library's External input)".to_owned()),
        ],
        Hover::Sound(part) => sound_parts(*part),
        Hover::SampleEditor(part) => editor_parts(*part, context.keys),
        Hover::ScrollBar => three(
            "Scroll bar",
            None,
            "drag the thumb; click beside it for a page; the wheel scrolls it too",
        ),
        Hover::Quantize(part) => match part {
            QuantizePart::Grid => three(
                "Grid",
                None,
                "the lines the notes move toward: the roll's snap or a note value",
            ),
            QuantizePart::Strength => three(
                "Strength",
                None,
                "how far toward the grid: 100 % all the way, 50 % half way",
            ),
            QuantizePart::Ends => three("Moves", None, "the notes' starts only, or their ends too"),
            QuantizePart::Apply => three(
                "Quantize",
                None,
                "moves the selected notes (all, with none selected); one undo step",
            ),
        },
    }
}

fn menu_parts(row: MenuRow, keys: &Keys) -> [Option<String>; 3] {
    let two = |name: &str, does: &str| [Some(name.to_owned()), None, Some(does.to_owned())];
    let keyed = |name: &str, does: &str, action: Action| {
        [
            Some(name.to_owned()),
            None,
            Some(format!("{does}; {}", keys.first(action))),
        ]
    };
    match row {
        MenuRow::File(row) => match row {
            FileRow::New => keyed(
                "New",
                "a new project with four starter kit rows",
                Action::New,
            ),
            FileRow::NewEmpty => two("New empty project", "a new project with one empty sampler"),
            FileRow::Open => keyed(
                "Open",
                "a Dawsome project",
                Action::Open,
            ),
            FileRow::OpenDemo => two("Open demo", "a song to play and pull apart"),
            FileRow::ImportAudio => two("Import audio", "sound files onto the playlist"),
            FileRow::ImportMidi => two("Import MIDI", "a MIDI file's notes into the pattern"),
            FileRow::Recent => two("Recent project", "opens it again"),
            FileRow::Save => keyed("Save", "writes the project", Action::Save),
            FileRow::SaveAs => keyed("Save As", "writes it under a new name", Action::SaveAs),
            FileRow::ExportWav => two("Export WAV", "renders the pattern or the song to a file"),
            FileRow::ExportMp3 => two("Export MP3", "renders the pattern or the song to an MP3"),
            FileRow::ExportMidi => two("Export pattern as MIDI", "the pattern's notes to a file"),
            FileRow::ExportSongMidi => two(
                "Export song as MIDI",
                "the playlist's notes in one file, a track per channel",
            ),
            FileRow::ExportStems => two(
                "Export stems",
                "a WAV file per playlist track, and the master",
            ),
            FileRow::ExportDawproject => two(
                "Export DAWproject",
                "the project for Bitwig Studio, Studio One or Cubase",
            ),
        },
        MenuRow::Edit(row) => match row {
            EditRow::Undo => keyed("Undo", "takes back the step it names", Action::Undo),
            EditRow::Redo => keyed("Redo", "does the step it names again", Action::Redo),
            EditRow::Delete => keyed("Delete", "the selected clips or notes", Action::Delete),
            EditRow::Duplicate => keyed(
                "Duplicate",
                "the selection, after itself",
                Action::Duplicate,
            ),
            EditRow::SelectAll => keyed("Select all notes", "in the piano roll", Action::SelectAll),
            EditRow::Settings => keyed(
                "Settings\u{2026}",
                "the audio and MIDI devices, and the keys",
                Action::Panel(Panel::Settings),
            ),
        },
        MenuRow::Channel(row) => match row {
            ChannelRow::InstrumentWindow => two(
                "Instrument window",
                "plays the channel and shapes its sound",
            ),
            ChannelRow::PluginWindow => two(
                "Plugin window",
                "the plugin's own interface; its parameters where it has none",
            ),
            ChannelRow::Parameters => two(
                "Parameters",
                "the plugin's settings as a list of sliders in a window of their own; right-click one to automate it",
            ),
            ChannelRow::MixerInsert => two(
                "Mixer insert",
                "gives the channel a mixer strip of its own (the first free insert) and opens the mixer there",
            ),
            ChannelRow::ChangeSample => two("Change sample", "another sound file for the row"),
            ChannelRow::EditSample => two(
                "Edit sample",
                "the sample editor: crop, fade, gain or reverse the sound; its file stays as it is",
            ),
            ChannelRow::OneNoteAtATime => two(
                "One note at a time",
                "a new note ends the one before it, like plucking a bass string; on for the 808s",
            ),
            ChannelRow::Rename => two("Rename", "a box over the name; Enter takes it"),
            ChannelRow::Clone => two("Clone", "a copy of the channel without its notes"),
            ChannelRow::Delete => two("Delete", "the channel and its notes; Undo brings it back"),
        },
        MenuRow::Track(row) => match row {
            TrackRow::Rename => two("Rename", "a box over the track's name; Enter takes it"),
            TrackRow::Delete => two("Delete", "the track and its clips; Undo brings them back"),
        },
        MenuRow::Notes(row) => two(row.label(), row.does()),
        MenuRow::Clip(row) => match row {
            ClipRow::Unlink => two(
                "Unlink (make unique)",
                "this clip gets its own copy of the pattern; the other clips keep playing it",
            ),
            ClipRow::LinkBack => two(
                "Link back",
                "this clip plays the pattern its copy was made from again; the copy goes when no other clip plays it",
            ),
        },
        MenuRow::Record(row) => {
            let (name, does) = row.parts();
            two(name, does)
        }
    }
}

fn picker_parts(part: PickerPart) -> [Option<String>; 3] {
    let two = |name: &str, does: &str| [Some(name.to_owned()), None, Some(does.to_owned())];
    match part {
        PickerPart::Plugin => two("Plugin", "click selects it; click again or Enter adds it"),
        PickerPart::Set => two(
            "Instrument set",
            "click selects it; click again or Enter adds it",
        ),
        PickerPart::KitSound => two("Starter kit sound", "click selects it; click again adds it"),
        PickerPart::SampleFile => two("Sample file", "click again to choose a sound file of yours"),
        PickerPart::Title => two("Section", "what the rows under it are"),
        PickerPart::Failed => two("Could not load", "plugin files the scan could not read"),
        PickerPart::Filter => two("Filter", "type part of a name or a maker"),
        PickerPart::Rescan => two("Rescan", "looks for plugins again"),
        PickerPart::Close => two("Close", "closes the picker; Esc too"),
    }
}

fn clip_control_parts(control: ClipControl) -> [Option<String>; 3] {
    let two = |name: &str, does: &str| [Some(name.to_owned()), None, Some(does.to_owned())];
    match control {
        ClipControl::NoStretch => two("No stretch", "the clip plays its file as it is"),
        ClipControl::Resample => two("Resample", "to the clip's length, its pitch with it"),
        ClipControl::Stretch => two("Stretch", "to the clip's length, its pitch kept"),
        ClipControl::PitchDown => two("Pitch", "a semitone down, while stretched"),
        ClipControl::PitchUp => two("Pitch", "a semitone up, while stretched"),
        ClipControl::Close => two("Close", "hides these settings"),
    }
}

fn editor_parts(part: EditorPart, keys: &Keys) -> [Option<String>; 3] {
    let two = |name: &str, does: String| [Some(name.to_owned()), None, Some(does)];
    let whole = "with nothing selected, the whole sound";
    match part {
        EditorPart::Waveform => two(
            "Waveform",
            format!(
                "drag to select; click: Play starts there; Shift+click stretches the selection; \
                 double-click or {} selects it all; Ctrl+wheel zooms, the wheel scrolls",
                keys.first(Action::SelectAll)
            ),
        ),
        EditorPart::Play => two(
            "Play",
            format!(
                "plays the selection (or from the cursor on) alone, as it sounds now; {} too",
                keys.first(Action::PlayStop)
            ),
        ),
        EditorPart::Stop => two("Stop", "stops Play".to_owned()),
        EditorPart::Fit => two("Fit", "shows the whole sound".to_owned()),
        EditorPart::ZoomToSelection => two(
            "Zoom to selection",
            "shows the selection across the window".to_owned(),
        ),
        EditorPart::Snap => two(
            "Snap to zero",
            "lit: a selection's edges move to where the wave crosses zero, so a cut does not click"
                .to_owned(),
        ),
        EditorPart::Op(EditorOp::Crop) => two(
            "Crop",
            "keeps only the selection; the channel's clips keep playing what they played"
                .to_owned(),
        ),
        EditorPart::Op(EditorOp::Delete) => two(
            "Delete",
            format!(
                "takes the selection out and closes the gap; {} too",
                keys.first(Action::Delete)
            ),
        ),
        EditorPart::Op(EditorOp::Silence) => two(
            "Silence",
            format!("makes the selection quiet, its length kept; {whole}"),
        ),
        EditorPart::Op(EditorOp::FadeIn) => two(
            "Fade in",
            format!("rises from silence over the selection; {whole}"),
        ),
        EditorPart::Op(EditorOp::FadeOut) => two(
            "Fade out",
            format!("falls to silence over the selection; {whole}"),
        ),
        EditorPart::Op(EditorOp::Normalize) => two(
            "Normalize",
            format!("as loud as it can be, its loudest point at 0 dB; {whole}"),
        ),
        EditorPart::Op(EditorOp::Reverse) => {
            two("Reverse", format!("plays the selection backwards; {whole}"))
        }
        EditorPart::Gain => two(
            "Gain",
            format!("drag up (louder) or down (quieter), let go to apply; {whole}"),
        ),
        EditorPart::Undo => two(
            "Undo",
            format!("each change is one step; {}", keys.first(Action::Undo)),
        ),
        EditorPart::Redo => two("Redo", keys.first(Action::Redo)),
        EditorPart::Revert => two(
            "Revert",
            "the sound as its file is, every change taken back in one step".to_owned(),
        ),
        EditorPart::SaveAs => two(
            "Save as new file\u{2026}",
            "writes the sound as it is now to a new WAV file beside the project's takes; your \
             file is never changed"
                .to_owned(),
        ),
        EditorPart::Close => two("Close", "closes the sample editor; Esc too".to_owned()),
    }
}

fn sound_parts(part: SoundPart) -> [Option<String>; 3] {
    let three = |name: String, does: &str| [Some(name), None, Some(does.to_owned())];
    match part {
        SoundPart::Close => three("Close".to_owned(), "closes the instrument window; Esc too"),
        SoundPart::Gate => three(
            "One-shot / Hold".to_owned(),
            "one-shot plays the whole sound; hold stops it when the note ends",
        ),
        SoundPart::Compare => three(
            "Store A and A/B".to_owned(),
            "Store A keeps the settings as they are; A/B switches between them and your changes",
        ),
        SoundPart::Key(key) => three(key_name(key), "click plays it; lower on the key is louder"),
        SoundPart::Control(Control::BendRange) => three(
            Control::BendRange.label().to_owned(),
            "how far a MIDI keyboard's pitch-bend wheel bends live notes, up and down; drag up or \
             down; double-click resets (\u{b1}2 st)",
        ),
        SoundPart::Control(control) => three(
            control.label().to_owned(),
            "drag up or down (Shift: fine); double-click resets; one undo step a drag",
        ),
        SoundPart::Flag(Flag::Reverse) => three("Reverse".to_owned(), "plays the sound backwards"),
        SoundPart::Flag(Flag::Filter) => three(
            "Low-pass filter".to_owned(),
            "on, it takes away what is above the cutoff",
        ),
        SoundPart::More => three("Zones".to_owned(), "lists the set's sounds and their keys"),
        SoundPart::EditSample => three(
            "Edit sample".to_owned(),
            "the sample editor: crop, fade, gain or reverse the sound; its file stays as it is",
        ),
        SoundPart::OpenPlugin => {
            three("Open plugin window".to_owned(), "the plugin's own controls")
        }
        SoundPart::Insert => three(
            "Output".to_owned(),
            "the mixer insert it plays into; pick another to route it there",
        ),
        SoundPart::InputDevice => three(
            "Input device".to_owned(),
            "the audio input it plays live (a synth cabled into your interface); pick another, one undo step",
        ),
        SoundPart::InputRefresh => three(
            "Refresh".to_owned(),
            "lists the input devices and MIDI outs again, after one is plugged in",
        ),
        SoundPart::InputChannels => three(
            "Channels".to_owned(),
            "which of the device's inputs it plays: a pair as stereo, one alone as mono",
        ),
        SoundPart::InputLevel => three(
            "Input level".to_owned(),
            "how loud the input comes in, before the channel's volume; red is past full scale",
        ),
        SoundPart::MidiOutPort => three(
            "MIDI out port".to_owned(),
            "the instrument the channel's notes are sent to (the synth whose sound comes back on its input); None sends nothing; one undo step",
        ),
        SoundPart::MidiOutChannel => three(
            "MIDI channel".to_owned(),
            "the MIDI channel the notes go out on, 1 to 16, as the instrument listens; one undo step",
        ),
        SoundPart::MidiOutProgram => three(
            "Program".to_owned(),
            "the sound the instrument is set to, sent when you pick it and when the project opens, never on Play; Off leaves its own",
        ),
        SoundPart::MidiOutLatency => three(
            "Latency".to_owned(),
            "how many ms early each note is sent, so the instrument's sound comes back on the beat; drag until it lands with the drums; one undo step a drag",
        ),
        SoundPart::MidiOutEcho => three(
            "Echo".to_owned(),
            "on: notes played on the instrument's own keys are sent back to it too; off when it already plays them itself",
        ),
    }
}

fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

/// The rack's pattern's length in bars, as the header shows it.
fn bars(context: &Context) -> Option<String> {
    let pattern = context.project.patterns.get(&context.pattern?)?;
    Some(pattern.length_bars.to_string())
}

fn tempo_text(tempo: MilliBpm) -> String {
    format!("{}.{:03} BPM", tempo.0 / 1000, tempo.0 % 1000)
}

fn tool_parts(tool: Tool, context: &Context) -> [Option<String>; 3] {
    let three = |name: &str, value: Option<String>, does: &str| {
        [Some(name.to_owned()), value, Some(does.to_owned())]
    };
    let pattern_name = context
        .pattern
        .and_then(|id| context.project.patterns.get(&id))
        .map(|pattern| pattern.name.clone());
    let key = |action| context.keys.first(action);
    match tool {
        Tool::Play => three(
            "Play",
            None,
            &format!("{} plays and stops", key(Action::PlayStop)),
        ),
        Tool::Stop => three(
            "Stop",
            None,
            &format!(
                "{} plays and stops; Stop goes back to the start and ends every note",
                key(Action::PlayStop)
            ),
        ),
        Tool::Record => three(
            "Record",
            None,
            &format!(
                "click or {} arms it, then Play records what the switch beside it says; right-click: its menu",
                key(Action::ArmRecord)
            ),
        ),
        Tool::TakeNotes => three(
            "Notes",
            None,
            "Record takes the notes played into the piano roll, drawn as each ends: the pattern loops and each pass adds (pattern mode), or a new pattern grows as you play (song mode); right-click: quantize, count-in, Capture",
        ),
        Tool::TakeAudio => three(
            "Audio",
            None,
            "Record takes what you hear, or the line in, into \"<project> take N.wav\": one pass of the pattern (pattern mode) or until Stop (song mode), a clip from the song marker; right-click: which",
        ),
        Tool::Capture => three(
            "Capture",
            None,
            &format!(
                "click or {}: the notes played in the last 30 s while not recording become notes, from a bar's start: one undo",
                key(Action::Capture)
            ),
        ),
        Tool::Metronome => three(
            "Metronome",
            None,
            "click switches it: a click every beat while playing, a bar's first louder; never recorded",
        ),
        Tool::Tempo => three(
            "Tempo",
            Some(tempo_text(context.project.tempo)),
            "drag up or down, or scroll; Shift for 0.001 steps",
        ),
        Tool::PreviousPattern => three("Previous pattern", None, "shows the pattern before"),
        Tool::PatternName => [
            pattern_name,
            None,
            Some("the pattern the rack shows; right-click: rename or delete".to_owned()),
        ],
        Tool::NextPattern => three(
            "Next pattern",
            None,
            "shows the pattern after; past the last one it makes a new pattern",
        ),
        Tool::PatternMode => three(
            "Pattern mode",
            None,
            &format!(
                "plays the selected pattern; {} switches",
                key(Action::ToggleMode)
            ),
        ),
        Tool::SongMode => three(
            "Song mode",
            None,
            &format!("plays the playlist; {} switches", key(Action::ToggleMode)),
        ),
        Tool::Loop => three(
            "Loop",
            Some(context.loop_on.map_or("no range", on_off).to_owned()),
            "switches the loop range; drag in the playlist's ruler to set one",
        ),
        Tool::Position => {
            let does = match context.resampling {
                Some(resampling) => format!("bar : step : tick of what plays{DOT}{resampling}"),
                None => "bar : step : tick of what plays".to_owned(),
            };
            three("Position", None, &does)
        }
        Tool::Notices => three(
            "Notices",
            context.notices.clone(),
            "click opens the Settings window, where the audio devices are",
        ),
        Tool::OctaveDown => three(
            "Typing keyboard octave",
            Some(context.octave.clone()),
            "an octave down",
        ),
        Tool::OctaveUp => three(
            "Typing keyboard octave",
            Some(context.octave.clone()),
            "an octave up",
        ),
        Tool::Keys => three(
            "Keyboard plays notes",
            Some(context.keyboard.clone()),
            &format!(
                "click or {} switches it on or off; Settings \u{2192} Keys chooses its layout",
                key(Action::Keyboard)
            ),
        ),
        // Settings is a window of its own.
        Tool::Panel(Panel::Settings) => [
            Some(panel_tip(Panel::Settings, context.keys)),
            None,
            Some("opens the Settings window; again closes it".to_owned()),
        ],
        Tool::Panel(panel) => [
            Some(panel_tip(panel, context.keys)),
            None,
            Some("shows or hides it".to_owned()),
        ],
        Tool::Retry => three("Retry audio", None, "opens the output device again"),
        Tool::Edit => three(
            "Edit",
            None,
            "undo and redo, each naming its step; delete, duplicate, select all; settings",
        ),
        Tool::File => three("File", None, "new, open, save, import and export"),
        Tool::Help => three(
            "Help",
            None,
            &format!(
                "click or {} opens the guide to Dawsome, for someone new to making music; again closes it",
                key(Action::Help)
            ),
        ),
    }
}

fn roll_parts(part: RollPart, context: &Context) -> [Option<String>; 3] {
    let three = |name: String, value: Option<String>, does: &str| {
        [Some(name), value, Some(does.to_owned())]
    };
    match part {
        RollPart::Note {
            key,
            velocity,
            edge: false,
        } => three(
            key_name(key),
            Some(format!("velocity {velocity}")),
            "drag moves the selection; Shift+click adds; double-click deletes; right-click: notes menu",
        ),
        RollPart::Note {
            key, edge: true, ..
        } => three(
            key_name(key),
            None,
            "drag the edge to change the note's length",
        ),
        RollPart::Empty { key } => three(
            key_name(key),
            None,
            "click draws a note; drag selects with a box; right-click: notes menu; Alt: off the snap",
        ),
        RollPart::Keys { key } => three(key_name(key), None, "click plays it"),
        RollPart::Stem => three(
            "Velocity".to_owned(),
            None,
            "drag sets velocity (across stems paints them); Alt+drag draws a ramp",
        ),
        RollPart::Snap => three(
            "Snap".to_owned(),
            None,
            "what notes land on when drawn or moved",
        ),
        RollPart::Preview => three(
            "Preview".to_owned(),
            Some(on_off(context.preview).to_owned()),
            "plays the notes you draw, press and move",
        ),
        RollPart::LaneHandle => three(
            "Velocity lane".to_owned(),
            None,
            "drag up or down to make the lane taller or shorter",
        ),
        RollPart::Follow => three("Follow".to_owned(), None, FOLLOW_WORDS),
        RollPart::Scale => three(
            "Scale".to_owned(),
            None,
            "keys outside the scale shade darker; Scale off shades none",
        ),
        RollPart::ScaleRoot => three(
            "Root".to_owned(),
            None,
            "the note the scale starts on",
        ),
        RollPart::Fold => three(
            "Fold".to_owned(),
            None,
            "lit: only rows for keys with notes and, with a scale, keys in it",
        ),
        RollPart::RecNotes => three(
            "Rec notes".to_owned(),
            None,
            "a take runs: what you play lands here, one undo step when it stops",
        ),
        RollPart::NotesMenu => three(
            "Notes".to_owned(),
            None,
            "quantize, chop, glue, legato, reverse, invert, humanize; on the selection, else all",
        ),
    }
}

/// The header's link control in the hint bar: one selected clip in the words the roll's control
/// had, several counted.
fn link_parts(context: &Context) -> [Option<String>; 3] {
    let three =
        |name: &str, value: String, does: String| [Some(name.to_owned()), Some(value), Some(does)];
    match &context.link {
        Some(SelectionLink::Linked { clips, places }) if clips.len() == 1 => three(
            "Linked",
            format!("this pattern plays in {places} places"),
            "edits change all of them; click to unlink this clip".to_owned(),
        ),
        Some(SelectionLink::Linked { clips, .. }) => three(
            "Linked",
            format!("{} selected clips share their patterns", clips.len()),
            "edits change every clip playing them; click to give each its own copy".to_owned(),
        ),
        Some(SelectionLink::Unlinked { clips, original }) if clips.len() == 1 => three(
            "Unlinked",
            "this clip has its own copy".to_owned(),
            format!(
                "click to link it back to {}",
                pattern_name(context.project, *original)
            ),
        ),
        Some(SelectionLink::Unlinked { clips, .. }) => three(
            "Unlinked",
            format!("{} selected clips have their own copies", clips.len()),
            "click to link each back to the pattern it was copied from".to_owned(),
        ),
        None => [
            Some("Link".to_owned()),
            None,
            Some(
                "shows while a selected pattern clip shares its pattern or plays a copy".to_owned(),
            ),
        ],
    }
}

/// The link control's tip, in the spec's words: what the state means and what a click does,
/// for one selected clip or several.
pub fn link_tip(project: &Project, link: &SelectionLink) -> String {
    match link {
        SelectionLink::Linked { clips, places } if clips.len() == 1 => format!(
            "Linked: this pattern plays in {places} places; edits change all of them. Click to unlink this clip."
        ),
        SelectionLink::Linked { clips, .. } => format!(
            "Linked: {} selected clips share their patterns; edits change every clip playing them. Click to give each its own copy.",
            clips.len()
        ),
        SelectionLink::Unlinked { clips, original } if clips.len() == 1 => format!(
            "Unlinked: this clip has its own copy. Click to link it back to {}.",
            pattern_name(project, *original)
        ),
        SelectionLink::Unlinked { clips, .. } => format!(
            "Unlinked: {} selected clips have their own copies. Click to link each back to the pattern it was copied from.",
            clips.len()
        ),
    }
}

/// Pattern `id`'s name, empty when it is gone.
fn pattern_name(project: &Project, id: PatternId) -> &str {
    project
        .patterns
        .get(&id)
        .map_or("", |pattern| pattern.name.as_str())
}

fn track_parts(hit: TrackHit, project: &Project) -> [Option<String>; 3] {
    let three = |name: String, does: &str| [Some(name), None, Some(does.to_owned())];
    let name = |row: usize| {
        project
            .playlist
            .tracks
            .values()
            .nth(row)
            .map_or_else(|| format!("Track {}", row + 1), |track| track.name.clone())
    };
    match hit {
        TrackHit::Led { row } => three(name(row), "click mutes or unmutes the track"),
        TrackHit::Name { row } => three(
            name(row),
            "double-click renames it; right-click: rename or delete it",
        ),
        TrackHit::Arm { row } => [
            Some("Record arm".to_owned()),
            Some(name(row)),
            Some("click arms the track to record audio onto; Record starts the take".to_owned()),
        ],
        TrackHit::AddTrack => three("+ Track".to_owned(), "adds a track under the last"),
    }
}

fn clip_parts(clip: ClipId, part: ClipPart, context: &Context) -> [Option<String>; 3] {
    let Some(shown) = context.project.playlist.clips.get(&clip) else {
        return [None, None, None];
    };
    let (name, does) = if shown.audio_channel().is_some() {
        (
            "Audio clip",
            match part {
                ClipPart::Body => {
                    "drag moves; double-click: the sample editor and stretch settings; right-click deletes; Shift+right-click mutes"
                }
                ClipPart::LeftEdge => "drag: trim",
                ClipPart::RightEdge => "drag: trim \u{b7} Alt+drag: stretch to fit",
            }
            .to_owned(),
        )
    } else if shown.automation().is_some() {
        (
            "Automation clip",
            "double-click adds a point; drag a point moves it; right-click a segment: its curve"
                .to_owned(),
        )
    } else {
        (
            "Pattern clip",
            match part {
                ClipPart::RightEdge => "drag resizes it".to_owned(),
                ClipPart::Body | ClipPart::LeftEdge => format!(
                    "drag moves; double-click opens it in the piano roll; right-click deletes; Shift+right-click mutes; Ctrl+right-click: unlink or link back; {} duplicates",
                    context.keys.first(Action::Duplicate)
                ),
            },
        )
    };
    [Some(name.to_owned()), None, Some(does)]
}

fn mixer_parts(hit: MixerHit, context: &Context) -> [Option<String>; 3] {
    let mixer = &context.project.mixer;
    let three = |name: String, value: Option<String>, does: &str| {
        [Some(name), value, Some(does.to_owned())]
    };
    let name = |id: InsertId| {
        mixer
            .get(id)
            .map_or_else(String::new, |insert| insert.name.clone())
    };
    match hit {
        MixerHit::RouteDown => three(
            "Route".to_owned(),
            None,
            &format!(
                "plays the selected channel into the insert before; {} into the one selected",
                context.keys.first(Action::Route)
            ),
        ),
        MixerHit::RouteUp => three(
            "Route".to_owned(),
            None,
            &format!(
                "plays the selected channel into the insert after; {} into the one selected",
                context.keys.first(Action::Route)
            ),
        ),
        MixerHit::RouteLabel => three("Route".to_owned(), None, "where the selected channel plays"),
        MixerHit::Led(id) => three(
            name(id),
            None,
            "click mutes; right-click solos (Shift+right-click for more than one)",
        ),
        MixerHit::ClipLight => three("Clip light".to_owned(), None, "click clears it"),
        MixerHit::Pan(id) => three(
            "Pan".to_owned(),
            mixer.get(id).map(|insert| pan_text(insert.pan)),
            "drag; double-click centers; right-click to automate or link a controller",
        ),
        MixerHit::Fader(id) => three(
            "Fader".to_owned(),
            mixer
                .get(id)
                .map(|insert| fader_text(fader_gain(insert.fader))),
            "drag (Shift: fine); double-click resets; right-click to automate or link a controller",
        ),
        MixerHit::Route(id) => three(
            format!("Send to {}", name(id)),
            None,
            "click sends the selected insert here, or stops it",
        ),
        MixerHit::Send(id) => {
            let amount = mixer
                .get(context.insert)
                .filter(|_| id != context.insert)
                .and_then(|from| from.routes.get(&id).copied())
                .map(percent_text);
            three(
                format!("Send to {}", name(id)),
                amount,
                "drag sets how much the selected insert sends",
            )
        }
        MixerHit::Name(id) => three(name(id), None, "double-click renames it"),
        MixerHit::Strip(id) => three(name(id), None, "click selects it"),
        MixerHit::Slot(slot) => {
            // A slot holding a VST3 plugin says so, and its latency.
            let value = mixer
                .get(context.insert)
                .and_then(|insert| insert.effects.iter().find(|effect| effect.slot == slot))
                .and_then(|effect| plugin_value(context, ProcessorOwner::Effect(effect.id)));
            three(
                format!("Effect slot {}", slot + 1),
                value,
                "click adds an effect or shows its settings; right-click: remove or move",
            )
        }
        MixerHit::Param(index) => three(
            format!("Effect setting {}", index + 1),
            None,
            "drag; double-click resets; right-click to automate or link a controller",
        ),
    }
}

fn library_parts(part: &LibraryPart, keys: &Keys) -> [Option<String>; 3] {
    let three = |name: &str, does: &str| [Some(name.to_owned()), None, Some(does.to_owned())];
    let enter = keys.first(Action::OpenSelected);
    match part {
        LibraryPart::Header(section) => three(section.title(), "click opens or closes it"),
        LibraryPart::Group(name) => three(
            name,
            &format!("click or {enter} folds or opens it; Left folds, Right opens"),
        ),
        LibraryPart::Sound(name) | LibraryPart::Set(name) => three(
            name,
            &format!("click previews; double-click or {enter} adds it to the rack; right-click adds it and opens its instrument window; drag onto the rack or a track"),
        ),
        LibraryPart::Plugin { name, note } => [
            Some(name.clone()),
            note.map(str::to_owned),
            Some(format!(
                "double-click or {enter} adds it to the rack; drag onto the rack or a track"
            )),
        ],
        LibraryPart::ExternalInput => three(
            "External input",
            &format!("a channel playing an audio input live (a synth cabled into your interface); double-click or {enter} adds it; right-click adds it and opens its window; drag onto the rack or a track"),
        ),
        LibraryPart::Rescan => three("Rescan", "looks for plugins again"),
        LibraryPart::Folder => three("Folder", "click opens or closes it; the arrows walk the tree"),
        LibraryPart::File(name) => three(
            name,
            "click previews; drag onto the rack, a sampler's row or a track",
        ),
        LibraryPart::AddFolder => three("+ Add folder\u{2026}", "adds a folder of your sounds"),
        LibraryPart::Device(port) => device_parts(port),
        LibraryPart::Tick(port) if port.loopback => three(
            &port.name,
            "a system loopback, not a device; tick it only for a program that sends through it",
        ),
        LibraryPart::Tick(port) => three(&port.name, "tick to play with it"),
    }
}

/// A MIDI device's row: where its notes go now after the arrow, and what to do with it.
fn device_parts(port: &Port) -> [Option<String>; 3] {
    let goes = port.arrow();
    let does = if port.loopback {
        "a system loopback, not a device: it echoes what a program sends it".to_owned()
    } else if let Some(silent) = &port.silent {
        format!("{silent}; select a channel with a sound, or drag the device onto one")
    } else if port.channel.is_some() {
        "plays the linked channel, its pedal, bend and mod wheel too; drag onto another rack \
         row to move it; right-click: Unlink"
            .to_owned()
    } else if port.plays {
        "plays the selected channel, its pedal, bend and mod wheel too; its light blinks as it \
         sends; drag onto a rack row to keep it on that channel"
            .to_owned()
    } else {
        "tick it to play with it; drag onto a rack row to play that channel with it".to_owned()
    };
    [Some(port.name.clone()), goes, Some(does)]
}

fn setting_parts(hit: SettingHit) -> [Option<String>; 3] {
    let three = |name: &str, does: &str| [Some(name.to_owned()), None, Some(does.to_owned())];
    match hit {
        SettingHit::AllPorts => three(
            "All ports",
            "lets every MIDI input in, those plugged in later too",
        ),
        SettingHit::Port(_) => three(
            "MIDI input",
            "tick a device to play with it: its keys, sustain pedal, pitch bend and mod wheel",
        ),
        SettingHit::OctaveDown => three("Typing keyboard octave", "an octave down"),
        SettingHit::OctaveUp => three("Typing keyboard octave", "an octave up"),
        SettingHit::ShowHints => three(
            "Show hints",
            "this line, naming what is under the cursor and what it does",
        ),
        SettingHit::Page(page) => three(page.title(), "shows this page of the settings"),
        SettingHit::Measure => three(
            "Measure input latency",
            "plays clicks through the output and finds them in the line in: loop a cable from \
             the output into the input first",
        ),
        SettingHit::LatencyDown => three("Input latency", "a millisecond less, set by hand"),
        SettingHit::LatencyUp => three("Input latency", "a millisecond more, set by hand"),
        SettingHit::LatencyClear => three(
            "Clear input latency",
            "forgets the figure: takes of the line in land late, as recorded",
        ),
    }
}

fn keys_parts(hit: KeysHit, keys: &Keys) -> [Option<String>; 3] {
    match hit {
        KeysHit::Layout(layout) => [
            Some(layout.name().to_owned()),
            None,
            Some("the notes the computer keyboard plays; the drawing shows them".to_owned()),
        ],
        KeysHit::Reset => [
            Some("Reset keys to defaults".to_owned()),
            None,
            Some("every shortcut back to the keys it had at first".to_owned()),
        ],
        KeysHit::Action(action) => [
            Some(action.label().to_owned()),
            Some(keys.all(action, ", ")),
            Some("click, then press its new keys; Esc gives up".to_owned()),
        ],
    }
}

/// What a canvas says when the cursor moved from what it was over last (`last`) onto `now`:
/// that it is over `now`, or, moving off everything, that it left `last`; nothing while it
/// stays on the same control. `last` takes `now`.
pub fn hover_change<M: From<HoverMessage>>(
    last: &mut Option<Hover>,
    now: Option<Hover>,
) -> Option<iced::widget::canvas::Action<M>> {
    if *last == now {
        return None;
    }
    let message = match (&now, last.take()) {
        (Some(hover), _) => HoverMessage::Over(hover.clone()),
        (None, Some(left)) => HoverMessage::Left(left),
        (None, None) => return None,
    };
    *last = now;
    Some(iced::widget::canvas::Action::publish(M::from(message)))
}

/// A canvas's or widget's word about the cursor.
#[derive(Debug, Clone, PartialEq)]
pub enum HoverMessage {
    /// The cursor is over this now.
    Over(Hover),
    /// The cursor left this; what it is over now said so itself, or nothing is under it.
    Left(Hover),
}

/// `content` saying when the cursor comes onto it and leaves it: a toolbar or strip widget's
/// hover.
pub fn hover_area<'a>(
    content: impl Into<iced::Element<'a, crate::app::Message>>,
    hover: Hover,
) -> iced::Element<'a, crate::app::Message> {
    use crate::app::Message;
    iced::widget::mouse_area(content)
        .on_enter(Message::Hover(HoverMessage::Over(hover.clone())))
        .on_exit(Message::Hover(HoverMessage::Left(hover)))
        .into()
}

/// `content` with a tip under it holding `words`: on the controls whose use a beginner cannot
/// read off them, so it shows even with the hint bar switched off.
pub fn with_tip<'a>(
    content: impl Into<iced::Element<'a, crate::app::Message>>,
    words: impl iced::widget::text::IntoFragment<'a>,
) -> iced::Element<'a, crate::app::Message> {
    use iced::widget::{container, text, tooltip};
    let tip = container(text(words).size(12))
        .padding([4, 8])
        .style(crate::ui::style::panel);
    tooltip(content, tip, tooltip::Position::Bottom).into()
}

/// A canvas's state when it tracks a drag and says what the cursor is over: where the cursor
/// was last seen during a drag (for a release with no cursor position), and the hover.
#[derive(Debug, Default)]
pub struct Pointer {
    pub last: Option<iced::Point>,
    pub hover: Option<Hover>,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use dawsome_engine::project::ChannelId;
    use dawsome_engine::time::Ticks;
    use iced::{Point, Size};

    use super::*;
    use crate::ui::browser_model::{Row, RowKind};
    use crate::ui::library::{InstrumentSet, Item, Port};
    use crate::ui::{layout, mixer_layout, piano_layout, playlist_layout, settings_layout};

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    fn project(name: &str) -> Project {
        crate::project_file::open(&Path::new(FIXTURES).join(name))
            .expect("opens")
            .project
    }

    /// The default shortcuts, which the hints these tests pin name.
    static KEYS: std::sync::LazyLock<Keys> = std::sync::LazyLock::new(Keys::default);

    fn context(project: &Project) -> Context<'_> {
        Context {
            project,
            pattern: project.patterns.keys().next().copied(),
            insert: InsertId::MASTER,
            loop_on: None,
            preview: true,
            octave: "C4".to_owned(),
            keyboard: "A is C5, velocity 100".to_owned(),
            keys: &KEYS,
            notices: None,
            resampling: None,
            takes_open: false,
            link: None,
            latencies: BTreeMap::new(),
        }
    }

    // One note at a time (`cut_itself`): a rack channel with it on says so beside its name; one
    // without it, nothing.
    #[test]
    fn a_channel_playing_one_note_at_a_time_says_so_on_its_rack_name() {
        let mut project = project("p26_overlap.json");
        let name = Hover::Rack {
            row: 0,
            part: RackPart::Name,
        };
        assert_eq!(
            words(&name, &context(&project)),
            "Sine \u{b7} one note at a time \u{b7} click: select \u{b7} double-click: instrument window \u{b7} right-click: menu"
        );
        project
            .channels
            .get_mut(&ChannelId(1))
            .expect("channel 1")
            .sound
            .cut_itself = false;
        assert_eq!(
            words(&name, &context(&project)),
            "Sine \u{b7} click: select \u{b7} double-click: instrument window \u{b7} right-click: menu"
        );
    }

    // Recording v2: the rack's Recordings header names its takes and what a click does; a take
    // under it, opened, is hinted as the audio channel it is.
    #[test]
    fn the_recordings_header_says_what_it_folds() {
        let project = project("p17_audio_song_after.json");
        let header = Hover::Rack {
            row: 1,
            part: RackPart::Name,
        };
        assert_eq!(
            words(&header, &context(&project)),
            "Recordings \u{b7} 2 audio takes \u{b7} click shows them; each take's file is in the Library under Recordings"
        );
        let open = Context {
            takes_open: true,
            ..context(&project)
        };
        let take = Hover::Rack {
            row: 2,
            part: RackPart::Name,
        };
        assert!(words(&take, &open).starts_with("Take 1 \u{b7} click"));
    }

    // Over the position the hint bar says how the output runs while it
    // is resampled (that left the toolbar); over the notices it holds them whole and says that a
    // click opens Settings (the Settings window, where the audio devices are;
    // it said "Settings → Audio", a page there is not).
    #[test]
    fn the_position_tells_the_resampling_and_the_notices_are_whole() {
        let f1 = project("p3_roll.json");
        let mut context = context(&f1);
        let position = Hover::Tool(Tool::Position);
        assert_eq!(
            words(&position, &context),
            "Position \u{b7} bar : step : tick of what plays"
        );
        context.resampling = Some("Resampling 44100 \u{2192} 48000 Hz");
        assert_eq!(
            words(&position, &context),
            "Position \u{b7} bar : step : tick of what plays \u{b7} Resampling 44100 \u{2192} 48000 Hz"
        );
        context.notices =
            Some("Oriole: Line offers no 32-bit float input \u{b7} Underruns: 2".to_owned());
        assert_eq!(
            words(&Hover::Tool(Tool::Notices), &context),
            "Notices \u{b7} Oriole: Line offers no 32-bit float input \u{b7} Underruns: 2 \u{b7} \
             click opens the Settings window, where the audio devices are"
        );
    }

    /// Every point of a `size` area, 2 px apart.
    fn sweep(size: Size) -> impl Iterator<Item = Point> {
        let columns = (size.width / 2.0) as usize;
        let rows = (size.height / 2.0) as usize;
        (0..columns)
            .flat_map(move |x| (0..rows).map(move |y| Point::new(x as f32 * 2.0, y as f32 * 2.0)))
    }

    /// Checks `hover` has a hint: a name and what it does, a dot apart; returns its words.
    fn hinted(hover: &Hover, context: &Context) -> String {
        let words = words(hover, context);
        assert!(
            words.split(DOT).count() >= 2 && words.split(DOT).all(|part| !part.is_empty()),
            "{hover:?} has no hint: {words:?}"
        );
        words
    }

    // The table: every control each panel's own hit test finds, swept 2 px apart over the
    // panel, and every toolbar control, has a hint naming it and what it does. A control whose
    // hit maps to no hover, or whose hover has no words, fails here.
    #[test]
    fn every_control_of_every_panel_has_a_hint() {
        let f1 = project("p3_roll.json");
        let context = context(&f1);
        for tool in Tool::all() {
            hinted(&Hover::Tool(tool), &context);
        }
        // The rack and its header.
        let mut kinds = BTreeSet::new();
        let size = layout::rack_canvas_size(f1.channels.len(), 16);
        for at in sweep(size) {
            if let Some(hit) = layout::hit_test(at, f1.channels.len(), 16) {
                kinds.insert(format!("{:?}", std::mem::discriminant(&hit)));
                hinted(&crate::ui::rack::hover_of(hit), &context);
            }
        }
        assert_eq!(
            kinds.len(),
            6,
            "mute, name, pan, volume, step and add were all found"
        );
        let mut header = BTreeSet::new();
        for at in sweep(Size::new(400.0, layout::RACK_HEADER_HEIGHT)) {
            if let Some(hit) = layout::header_hit_test(at) {
                header.insert(format!("{hit:?}"));
                hinted(&Hover::RackHeader(hit), &context);
            }
        }
        assert_eq!(header.len(), 3);
        // The piano roll: the key column, notes (bodies and edges), empty grid, the stems.
        let view = piano_layout::View {
            top: 69,
            scroll: Ticks(0),
            zoom: piano_layout::DEFAULT_ZOOM,
            step: Ticks(24),
            rows: crate::ui::piano_layout::Rows::ALL,
        };
        let notes: Vec<_> = f1.patterns[&dawsome_engine::project::PatternId(1)]
            .notes
            .iter()
            .filter(|note| note.channel == ChannelId(1))
            .cloned()
            .collect();
        let mut roll = BTreeSet::new();
        for at in sweep(Size::new(600.0, 300.0)) {
            if let Some(hover) = crate::ui::piano_roll::grid_hover(&view, notes.iter(), at) {
                let Hover::Roll(part) = &hover else {
                    panic!("{hover:?}")
                };
                roll.insert(format!("{:?}", std::mem::discriminant(part)));
                hinted(&hover, &context);
            }
            if piano_layout::stem_under(&view, notes.iter(), at.x).is_some() {
                hinted(&Hover::Roll(RollPart::Stem), &context);
            }
        }
        assert_eq!(roll.len(), 3, "keys, notes and empty grid");
        for part in [
            RollPart::Snap,
            RollPart::Preview,
            RollPart::NotesMenu,
            RollPart::RecNotes,
            RollPart::Scale,
            RollPart::ScaleRoot,
            RollPart::Fold,
        ] {
            hinted(&Hover::Roll(part), &context);
        }
        // The playlist: track headers, the ruler's bands, the clips of every kind and part,
        // empty lanes and the header's controls.
        let grid = playlist_layout::Grid {
            scroll: Ticks(0),
            zoom: playlist_layout::DEFAULT_ZOOM,
            bar: Ticks(384),
            top: 0,
            lane: playlist_layout::LANE_HEIGHT,
        };
        let mut tracks = BTreeSet::new();
        for at in sweep(Size::new(playlist_layout::TRACK_HEADER_WIDTH, 400.0)) {
            if let Some(hit) = playlist_layout::header_hit(&grid, at, 8) {
                tracks.insert(format!("{:?}", std::mem::discriminant(&hit)));
                hinted(&Hover::Track(hit), &context);
            }
        }
        assert_eq!(tracks.len(), 4, "light, name, arm and + Track");
        for y in [2.0, playlist_layout::RULER_HEIGHT - 2.0] {
            let band = playlist_layout::ruler_band(y).expect("a band");
            hinted(&Hover::Ruler(band), &context);
        }
        for fixture in ["p4_two_clips.json", "p8_ui.json"] {
            let project = project(fixture);
            let context = super::tests::context(&project);
            for &clip in project.playlist.clips.keys() {
                for part in [ClipPart::Body, ClipPart::LeftEdge, ClipPart::RightEdge] {
                    hinted(&Hover::Clip { clip, part }, &context);
                }
            }
        }
        hinted(&Hover::Lane, &context);
        for tool in [
            PlaylistTool::Snap,
            PlaylistTool::ZoomOut,
            PlaylistTool::ZoomIn,
            PlaylistTool::Link,
        ] {
            hinted(&Hover::PlaylistTool(tool), &context);
        }
        // The mixer, over its whole panel.
        let mixed = project("p5_gain.json");
        let context = super::tests::context(&mixed);
        let last = mixed.mixer.ids().last().expect("inserts");
        let mut mixer = BTreeSet::new();
        let desk = mixer_layout::Desk {
            size: Size::new(1000.0, 504.0),
            scroll: 0.0,
            last,
            params_top: 0,
        };
        for at in sweep(desk.size) {
            if let Some(hit) = mixer_layout::hit_test(at, desk) {
                mixer.insert(format!("{:?}", std::mem::discriminant(&hit)));
                hinted(&Hover::Mixer(hit), &context);
            }
        }
        assert!(mixer.len() >= 11, "the mixer's controls: {mixer:?}");
        // The Library: a row of every kind.
        let rows = [
            (
                "music",
                RowKind::Root {
                    user: false,
                    open: true,
                },
            ),
            ("drums", RowKind::Folder { open: false }),
            ("kick.wav", RowKind::File),
            ("", RowKind::AddFolder),
        ]
        .map(|(name, kind)| Row {
            depth: 0,
            name: name.to_owned(),
            path: name.into(),
            kind,
            parent: None,
        });
        let set = InstrumentSet {
            name: "Pad".to_owned(),
            maker: String::new(),
            sfz: "pad.sfz".into(),
            bytes: None,
        };
        let port = Port {
            name: "Port A".to_owned(),
            plays: true,
            goes: Some("selected: kick".to_owned()),
            ..Port::default()
        };
        let items = [
            Item::Header {
                section: Section::Instruments,
                count: 1,
                open: true,
            },
            Item::Group(crate::ui::library::PLUGINS),
            Item::Kind(crate::ui::library::Kind::Drums),
            Item::Maker("Example".to_owned()),
            Item::Sound(0),
            Item::Set(set),
            Item::Rescan,
            Item::ExternalInput,
            Item::Tree(0),
            Item::Tree(1),
            Item::Tree(2),
            Item::Tree(3),
            Item::Port(port),
        ];
        for item in &items {
            for on_tick in [false, true] {
                let hover = crate::ui::browser::hover_of_item(item, &rows, on_tick)
                    .unwrap_or_else(|| panic!("{item:?} has a hover"));
                hinted(&hover, &context);
            }
        }
        // The settings panel.
        let settings = settings_layout::Layout { ports: 2, notes: 1 };
        let mut hits = BTreeSet::new();
        for at in sweep(Size::new(400.0, 260.0)) {
            if let Some(hit) = settings.hit(at) {
                hits.insert(format!("{:?}", std::mem::discriminant(&hit)));
                hinted(&Hover::Setting(hit), &context);
            }
        }
        assert_eq!(
            hits.len(),
            5,
            "all ports, a port, the stepper's two and the hints box"
        );
        let latency = settings.audio(2).below(1, 2).latency_below(0);
        let mut buttons = BTreeSet::new();
        for at in sweep(Size::new(400.0, latency.bottom())) {
            if let Some(hit) = latency.hit(at) {
                buttons.insert(format!("{hit:?}"));
                hinted(&Hover::Setting(hit), &context);
            }
        }
        assert_eq!(
            buttons.len(),
            4,
            "Measure, less, more and Clear: {buttons:?}"
        );
        // The instrument window.
        for part in SoundPart::all() {
            hinted(&Hover::Sound(part), &context);
        }
    }

    // v2: every control the instrument window's strip finds under the cursor, swept 2 px
    // apart over the strip of a sampler, a set, a plugin and an input channel, has a hint naming
    // it and what it does, as do its widgets (the output chooser, the footer's buttons, the map,
    // an input channel's pickers, Refresh and meter).
    #[test]
    fn every_control_of_the_instrument_window_has_a_hint() {
        use crate::ui::instrument_layout::{Strip, STRIP_HEIGHT};
        use crate::ui::instrument_window::hover_of;
        let f1 = project("p3_roll.json");
        let context = context(&f1);
        for kind in [
            ChannelKind::Sampler,
            ChannelKind::Instrument,
            ChannelKind::Plugin,
            ChannelKind::Input,
        ] {
            let strip = Strip::of(kind, 536.0);
            let mut found = BTreeSet::new();
            for at in sweep(Size::new(strip.width, STRIP_HEIGHT)) {
                if let Some(placed) = strip.hit(at) {
                    found.insert(format!("{:?}", placed.part));
                    hinted(&hover_of(placed.part), &context);
                }
            }
            let parts = strip
                .parts()
                .filter(|placed| placed.part != crate::ui::instrument_layout::Part::Insert)
                .count();
            assert_eq!(
                found.len(),
                parts,
                "{kind:?}: every control is found: {found:?}"
            );
        }
        for part in SoundPart::all() {
            hinted(&Hover::Sound(part), &context);
        }
        assert_eq!(
            words(&Hover::Sound(SoundPart::Insert), &context),
            "Output \u{b7} the mixer insert it plays into; pick another to route it there"
        );
    }

    // The widget controls outside the canvases. Every row of the File,
    // Edit and channel menus (each menu builds its rows from these lists), every kind of row
    // the pickers' own hit test finds over "+ Instrument…"'s sections (sets, the kit, plugins,
    // the files that failed, "Sample file…" and the sections' titles), the pickers' filter,
    // Rescan and Close, and every control of an audio clip's properties strip (built from its
    // list) has a hint naming it and what it does.
    #[test]
    fn every_menu_row_picker_row_and_clip_control_has_a_hint() {
        use crate::ui::picker;
        let f1 = project("p3_roll.json");
        let context = context(&f1);
        let rows = FileRow::ALL
            .map(MenuRow::File)
            .into_iter()
            .chain(EditRow::ALL.map(MenuRow::Edit))
            .chain(ChannelRow::ALL.map(MenuRow::Channel))
            .chain(TrackRow::ALL.map(MenuRow::Track))
            .chain(NoteRow::ALL.map(MenuRow::Notes))
            .chain(RecordRow::ALL.map(MenuRow::Record))
            .chain(ClipRow::ALL.map(MenuRow::Clip));
        for row in rows {
            hinted(&Hover::Menu(row), &context);
        }
        assert_eq!(
            words(
                &Hover::Menu(MenuRow::Channel(ChannelRow::OneNoteAtATime)),
                &context
            ),
            "One note at a time \u{b7} a new note ends the one before it, like plucking a bass string; on for the 808s"
        );
        let catalogue = dawsome_host_clap::Catalogue::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/p7_catalogue.json"),
        )
        .expect("the catalogue loads");
        let sets = crate::ui::library::Sets {
            shipped: vec![crate::ui::library::InstrumentSet {
                name: "Piano".to_owned(),
                maker: "Dawsome".to_owned(),
                sfz: "piano.sfz".into(),
                bytes: None,
            }],
            mine: Vec::new(),
        };
        let lines = picker::add_lines(&sets, &["Kick"], &catalogue, "");
        let height = lines.len() as f32 * picker::ROW_HEIGHT;
        let mut kinds = BTreeSet::new();
        for at in sweep(Size::new(picker::LIST_WIDTH, height)) {
            if let Some(hover) = picker::hover_at(&lines, 0.0, at) {
                kinds.insert(format!("{hover:?}"));
                hinted(&hover, &context);
            }
        }
        assert_eq!(kinds.len(), 6, "{kinds:?}");
        for part in PickerPart::CONTROLS {
            hinted(&Hover::Picker(part), &context);
        }
        for control in ClipControl::ALL {
            hinted(&Hover::ClipControl(control), &context);
        }
        for part in QuantizePart::ALL {
            hinted(&Hover::Quantize(part), &context);
        }
        for part in crate::ui::sample_editor::EditorPart::all() {
            hinted(&Hover::SampleEditor(part), &context);
        }
    }

    // Known answers, written by hand: a value with its unit between the name and the gestures.
    #[test]
    fn a_hint_names_the_control_its_value_and_its_gestures() {
        let f1 = project("p3_roll.json");
        let context = context(&f1);
        assert_eq!(
            words(&Hover::Rack { row: 0, part: RackPart::Volume }, &context),
            "Volume \u{b7} 100% \u{b7} drag up or down; right-click to automate or link a controller"
        );
        assert_eq!(
            words(&Hover::Tool(Tool::Tempo), &context),
            "Tempo \u{b7} 120.000 BPM \u{b7} drag up or down, or scroll; Shift for 0.001 steps"
        );
        assert_eq!(
            words(&Hover::Roll(RollPart::Keys { key: 60 }), &context),
            "C5 \u{b7} click plays it"
        );
        assert_eq!(key_name(61), "C#5");
        assert_eq!(key_name(0), "C0");
        assert_eq!(key_name(127), "G10");
    }

    // 6d K3: every control of Settings' Keys page, swept 2 px apart, and each page's tab has a
    // hint naming it and what it does; every action's row is found.
    // The velocity lane's handle, Follow, the scroll bars, Fit and the dividers each have a hint
    // naming them and what they do.
    #[test]
    fn the_lane_follow_scroll_and_divider_controls_have_hints() {
        let f1 = project("p3_roll.json");
        let context = context(&f1);
        let handle = hinted(&Hover::Roll(RollPart::LaneHandle), &context);
        assert_eq!(
            handle,
            "Velocity lane \u{b7} drag up or down to make the lane taller or shorter"
        );
        for hover in [
            Hover::Roll(RollPart::Follow),
            Hover::ScrollBar,
            Hover::PlaylistTool(PlaylistTool::Follow),
            Hover::PlaylistTool(PlaylistTool::Fit),
        ] {
            hinted(&hover, &context);
        }
        // The mixer's handle says what a drag does, as the lane's does. Pin moved: the handle
        // is the divider over the mixer, as every
        // line between two stacked panels is; each names the two panels and the double-click.
        assert_eq!(
            hinted(
                &Hover::Divider {
                    upper: Panel::PianoRoll,
                    lower: Panel::Mixer
                },
                &context
            ),
            "Piano roll / Mixer \u{b7} drag to resize the panels above and below; double-click \
             resets"
        );
        for (upper, lower) in [
            (Panel::Rack, Panel::Playlist),
            (Panel::Playlist, Panel::PianoRoll),
            (Panel::Rack, Panel::PianoRoll),
            (Panel::Playlist, Panel::Mixer),
            (Panel::Rack, Panel::Mixer),
        ] {
            hinted(&Hover::Divider { upper, lower }, &context);
        }
        // P3: a channel's name says what a click, a double-click and a right-click do.
        let name = Hover::Rack {
            row: 0,
            part: RackPart::Name,
        };
        assert_eq!(
            words(&name, &context),
            "lead \u{b7} click: select \u{b7} double-click: instrument window \u{b7} right-click: menu"
        );
        let empty = crate::project_file::new_document();
        assert_eq!(
            words(&name, &super::tests::context(&empty)),
            "Sampler \u{b7} click: select \u{b7} double-click: \
             choose its sound \u{b7} right-click: menu"
        );
    }

    #[test]
    fn every_control_of_the_keys_page_has_a_hint() {
        use crate::ui::keys_page::{content_height, hit, ROW_WIDTH};
        use crate::ui::settings_layout::SettingsPage;
        let f1 = project("p3_roll.json");
        let context = context(&f1);
        let mut rows = BTreeSet::new();
        for at in sweep(Size::new(ROW_WIDTH, content_height())) {
            if let Some(found) = hit(at) {
                rows.insert(format!("{found:?}"));
                hinted(&Hover::Keys(found), &context);
            }
        }
        assert_eq!(
            rows.len(),
            Action::ALL.len() + 3,
            "every row, two layouts, Reset"
        );
        for page in SettingsPage::ALL {
            hinted(&Hover::Setting(SettingHit::Page(page)), &context);
        }
    }
}
