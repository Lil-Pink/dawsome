//! The app's own settings, kept per user in `dawsome/settings.json` in the settings folder
//! ([`project_file::config_dir`]), not in any project: which MIDI inputs play,
//! `"midi_inputs": "all"` or a list of port names, the window's layout when it last closed
//! (`"window"`), the instrument window's, the Settings window's, the Help window's, the
//! parameter window's and the sample editor's size and position (`"instrument_window"`,
//! `"settings_window"`, `"help_window"`, `"params_window"`, `"sample_editor_window"`), the
//! sample editor's Snap to zero (`"snap_to_zero"`, written only when off), and the
//! audio output device chosen (`"audio_output"`: a sound server's sink as `sink:` and its node
//! name, any other device by its name; none is the system's default; `"audio_output_name"`, the
//! name it was listed by, beside a sink's), the input device new input channels play
//! (`"audio_input"`, its name; none is the system's default input), whether the computer
//! keyboard plays notes (`"keyboard_plays_notes"`, written only when off), its note layout
//! (`"note_layout"`, written only when not the piano row) and the shortcuts bound otherwise
//! than by default (`"keys"`: an action's name to its chords, a comma apart; a name this build
//! does not know, or a value that is no text, is kept as it was and does nothing, and costs no
//! other name its chords), the
//! first-run notes already seen (`"callouts"`, `"keyboard_shown"`) and the piano roll's
//! velocity lane height and Follow switch (`"piano_roll"`: `"lane_height"`, written only when
//! dragged off its default, `"follow"`, written only when off), the playlist's Follow switch
//! and lane height (`"playlist"`: `"follow"`, written only when off, `"lane_height"`, written
//! only while a height was chosen over the lanes fitting the panel) and the mixer's height
//! (`"mixer"`: `"height"`, written only when dragged off its default), and how late the line in
//! records what the output plays (`"input_latency"`: one entry per output and input device,
//! [`InputLatency`], written once one is kept). Keys this
//! build does not know are kept as they were, so an older build saving
//! the file loses nothing a newer one wrote; a value of a key this build knows that it cannot
//! read is the default, without the other keys losing theirs.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use dawsome_engine::project::{same_port, stable_port_name};
use iced::{Point, Size};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::desktop::PopOut;
use crate::keys::{Keys, NoteLayout};
use crate::project_file::{self, SaveError};
use crate::ui::panels::{Kept, Panel, Panels};
use crate::ui::playlist_layout::{self, Lanes};
use crate::ui::{mixer_layout, piano_layout};

/// Which MIDI input ports play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MidiInputs {
    /// Every port, those plugged in later too.
    All,
    /// The ports of these names; none at first. A port is chosen by its name without the
    /// numbers the system appends on Linux ([`stable_port_name`]), and compared so, so a choice
    /// holds when another device is found first.
    Named(BTreeSet<String>),
}

impl Default for MidiInputs {
    fn default() -> Self {
        MidiInputs::Named(BTreeSet::new())
    }
}

impl MidiInputs {
    pub fn plays(&self, port: &str) -> bool {
        match self {
            MidiInputs::All => true,
            MidiInputs::Named(names) => names.iter().any(|name| same_port(name, port)),
        }
    }

    /// "All ports" switched: from all to none, else to all.
    pub fn toggle_all(&mut self) {
        *self = match self {
            MidiInputs::All => MidiInputs::Named(BTreeSet::new()),
            MidiInputs::Named(_) => MidiInputs::All,
        };
    }

    /// The port `name` switched; `listed` are the ports there are, which "all" stands for when
    /// one of them goes. Switched off, every name of the port goes (one stored under former
    /// numbers too); switched on, it is stored without the system's numbers.
    pub fn toggle(&mut self, name: &str, listed: &[String]) {
        let mut names: BTreeSet<String> = match std::mem::take(self) {
            MidiInputs::All => listed
                .iter()
                .map(|port| stable_port_name(port).to_owned())
                .collect(),
            MidiInputs::Named(names) => names,
        };
        let before = names.len();
        names.retain(|kept| !same_port(kept, name));
        if names.len() == before {
            names.insert(stable_port_name(name).to_owned());
        }
        *self = MidiInputs::Named(names);
    }
}

impl Serialize for MidiInputs {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            MidiInputs::All => serializer.serialize_str("all"),
            MidiInputs::Named(names) => serializer.collect_seq(names),
        }
    }
}

impl<'de> Deserialize<'de> for MidiInputs {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Written {
            Word(String),
            Names(BTreeSet<String>),
        }
        match Written::deserialize(deserializer)? {
            Written::Word(word) if word == "all" => Ok(MidiInputs::All),
            Written::Word(word) => Err(serde::de::Error::custom(format!(
                "midi_inputs is \"all\" or a list of port names, not {word:?}"
            ))),
            Written::Names(names) => Ok(MidiInputs::Named(names)),
        }
    }
}

/// The window as it last closed: its panels, its size while not maximized, and whether it was
/// maximized.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowLayout {
    /// The open panels by their command-line names; names this build does not know are
    /// passed over.
    pub panels: Vec<String>,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

/// The size a first start gives the window under the maximized one, which it shows when the
/// window is restored.
pub const FIRST_START_SIZE: Size = Size::new(1280.0, 800.0);

/// The smallest window a saved layout restores: a size smaller than this (a file edited by
/// hand) opens at [`FIRST_START_SIZE`]'s.
const SMALLEST_RESTORED: Size = Size::new(400.0, 300.0);

/// The largest window a saved layout restores, each side on its own (an 8K screen): a size
/// past it (a file edited by hand, or corrupt) opens at it, and the open window is then fitted
/// to its monitor ([`fit_to_monitor`]).
const LARGEST_RESTORED: Size = Size::new(7680.0, 4320.0);

/// The size a window of `size` takes on a monitor of `monitor`: each side no longer than the
/// monitor's; `None` when it fits already.
pub fn fit_to_monitor(size: Size, monitor: Size) -> Option<Size> {
    let fitted = Size::new(
        size.width.min(monitor.width),
        size.height.min(monitor.height),
    );
    (fitted != size).then_some(fitted)
}

impl WindowLayout {
    /// The layout of a window with `panels` open, `size` when not maximized. The Settings
    /// window is left out: like the instrument window it opens when asked for, not at the next
    /// start (it was a panel of the main window's before).
    pub fn of(panels: Panels, size: Size, maximized: bool) -> Self {
        Self {
            panels: panels
                .open()
                .filter(|&panel| panel != Panel::Settings)
                .map(|panel| panel.name().to_owned())
                .collect(),
            width: size.width,
            height: size.height,
            maximized,
        }
    }
}

/// Where a pop-out window (the instrument window, the Settings window) was when it last closed:
/// its size, and its position where the system tells a window its place (X11 and Windows do; a
/// Wayland compositor places every window itself and tells it nothing, so there it is none).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowPlace {
    pub width: f32,
    pub height: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<(f32, f32)>,
}

impl WindowPlace {
    /// The place of a window of `size` at `position`.
    pub fn of(size: Size, position: Option<Point>) -> Self {
        Self {
            width: size.width,
            height: size.height,
            position: position.map(|at| (at.x, at.y)),
        }
    }

    /// The size `window` opens at: as saved, each side at most the largest a window takes; a
    /// size not finite or smaller than the window's least (a hand edit) opens at its first
    /// size.
    pub fn size(&self, window: PopOut) -> Size {
        let least = window.least_size();
        let usable = self.width.is_finite()
            && self.height.is_finite()
            && self.width >= least.width
            && self.height >= least.height;
        if !usable {
            return window.first_size();
        }
        Size::new(
            self.width.min(LARGEST_RESTORED.width),
            self.height.min(LARGEST_RESTORED.height),
        )
    }

    /// Where the window opens: where it was, when the system said; else none, the system's
    /// choice.
    pub fn position(&self) -> Option<Point> {
        self.position
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .map(|(x, y)| Point::new(x, y))
    }
}

impl WindowPlace {
    /// `window`'s first opening: its first size, where the system puts it.
    pub fn first(window: PopOut) -> Self {
        Self::of(window.first_size(), None)
    }
}

/// How the window opens: its panels, its size, and whether it is maximized.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StartWindow {
    pub panels: Panels,
    pub size: Size,
    pub maximized: bool,
}

/// How the window opens with no layout given on the command line: as `saved` left it, or, on a
/// first start (nothing saved), with the Library, the rack and the playlist, maximized over a
/// [`FIRST_START_SIZE`] window. A saved layout naming no panel this build knows opens the
/// rack, so the window is never empty; a saved size too small to use is the first start's.
/// "settings" in a layout an earlier build saved (when Settings was a panel of the main window)
/// is passed over: the Settings window opens when asked for.
pub fn start_window(saved: Option<&WindowLayout>) -> StartWindow {
    let Some(saved) = saved else {
        return StartWindow {
            panels: Panels::FIRST_START,
            size: FIRST_START_SIZE,
            maximized: true,
        };
    };
    let mut panels = Panels::NONE;
    for panel in saved
        .panels
        .iter()
        .filter_map(|name| Panel::from_name(name))
        .filter(|&panel| panel != Panel::Settings)
    {
        panels.set(panel, true);
    }
    if panels == Panels::NONE {
        panels = Panels::default();
    }
    let usable = saved.width.is_finite()
        && saved.height.is_finite()
        && saved.width >= SMALLEST_RESTORED.width
        && saved.height >= SMALLEST_RESTORED.height;
    StartWindow {
        panels,
        size: if usable {
            Size::new(
                saved.width.min(LARGEST_RESTORED.width),
                saved.height.min(LARGEST_RESTORED.height),
            )
        } else {
            FIRST_START_SIZE
        },
        maximized: saved.maximized,
    }
}

/// The settings file's content.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub midi_inputs: MidiInputs,
    /// The window's layout when it last closed; none before the first close.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub window: Option<WindowLayout>,
    /// The instrument window's size and position when it last closed; none before.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub instrument_window: Option<WindowPlace>,
    /// The Settings window's size and position when it last closed; none before.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub settings_window: Option<WindowPlace>,
    /// The Help window's size and position when it last closed; none before.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub help_window: Option<WindowPlace>,
    /// The parameter window's size and position when it last closed; none before.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub params_window: Option<WindowPlace>,
    /// The sample editor's size and position when it last closed; none before.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub sample_editor_window: Option<WindowPlace>,
    /// Whether the sample editor's selection snaps to zero crossings; written only when off.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    snap_to_zero: Option<bool>,
    /// Whether drawing, moving and clicking notes in the piano roll plays them; written only
    /// when off.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    preview_notes: Option<bool>,
    /// Whether the hint bar shows at the window's foot; written only when off.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    show_hints: Option<bool>,
    /// The output device chosen: what Settings lists it by ([`crate::audio::SINK`] and a sink's
    /// node name, else its name); written only when one is (none plays through the system's
    /// default output).
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    audio_output: Option<String>,
    /// The name the chosen output was shown by, kept to name it while it is not there; written
    /// only when it is not what `audio_output` holds (a sink's description).
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    audio_output_name: Option<String>,
    /// The input device a new input channel plays, by name; written only when one is chosen
    /// (none: the system's default input).
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    audio_input: Option<String>,
    /// Whether the computer keyboard plays notes; written only when off.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    keyboard_plays_notes: Option<bool>,
    /// The note layout the keyboard plays, by its id; written only when not the piano row. An
    /// id this build does not know (a newer build's) plays the piano row and is written back.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    note_layout: Option<String>,
    /// The shortcuts bound otherwise than by default: an action's name to its chords
    /// ([`crate::keys::Keys::saved`]). Names this build does not know, and values that are no
    /// text, are kept as they were for the build that wrote them. Why each value apart: read as
    /// one map of texts, one list among them cost every rebound action its keys.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Map::is_empty"
    )]
    keys: Map<String, Value>,
    /// The first-run notes seen and put away; written once one is.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Callouts::is_default"
    )]
    callouts: Callouts,
    /// The channel rack's own settings; written once one is not the default.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "RackSettings::is_default"
    )]
    rack: RackSettings,
    /// The piano roll's own settings; written once one is not the default.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "RollSettings::is_default"
    )]
    piano_roll: RollSettings,
    /// The playlist's own settings; written once one is not the default.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "PlaylistSettings::is_default"
    )]
    playlist: PlaylistSettings,
    /// The mixer's own settings; written once one is not the default.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "MixerSettings::is_default"
    )]
    mixer: MixerSettings,
    /// The Library's own settings; written once one is not the default.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "LibrarySettings::is_default"
    )]
    library: LibrarySettings,
    /// How late the line in records what the output plays, per output and input device; written
    /// once one is kept.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Vec::is_empty"
    )]
    input_latency: Vec<InputLatency>,
    /// What this build does not know, written back as it was read.
    #[serde(flatten)]
    rest: Map<String, Value>,
}

/// How many frames late a take of the line in records what the output plays, when `input`
/// records while `output` plays (device names as Settings lists them, "" for the system's
/// default): measured by Settings → Input latency, or set there by hand. Kept at the rate it was
/// measured at, which a project at another rate converts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputLatency {
    pub output: String,
    pub input: String,
    /// Negative when the input records ahead of what plays.
    pub input_latency_frames: i64,
    pub rate: u32,
    /// Measured (or set by hand).
    pub measured: bool,
}

impl InputLatency {
    /// The latency in frames at `rate`: `input_latency_frames · rate / self.rate`, rounded half
    /// away from zero.
    pub fn frames_at(&self, rate: u32) -> i64 {
        let scaled = i128::from(self.input_latency_frames) * i128::from(rate);
        let from = i128::from(self.rate.max(1));
        let rounded = (scaled.abs() * 2 + from) / (2 * from);
        i64::try_from(rounded * scaled.signum()).unwrap_or(0)
    }

    /// The latency as Settings shows it: milliseconds with one decimal, rounded half up, and
    /// how it was found: "25.7 ms (measured)", "12.0 ms (set by hand)".
    pub fn words(&self) -> String {
        // Tenths of a millisecond are frames at 10 kHz.
        let tenths = self.frames_at(10_000);
        let sign = if tenths < 0 { "-" } else { "" };
        let how = if self.measured {
            "measured"
        } else {
            "set by hand"
        };
        format!(
            "{sign}{}.{} ms ({how})",
            tenths.abs() / 10,
            tenths.abs() % 10
        )
    }
}

/// The channel rack's settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RackSettings {
    /// The panel's height as the divider under it left it, while another panel takes what is
    /// left; written only once one is dragged (none: its rows' height up to half the stack).
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    height: Option<f32>,
    /// The rack's settings this build does not know, written back as they were read.
    #[serde(flatten)]
    rest: Map<String, Value>,
}

impl RackSettings {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The piano roll's settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RollSettings {
    /// The panel's height, the divider over it included, as the dividers left it while the
    /// playlist takes what is left; written only once one is dragged (none: an octave of rows).
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    height: Option<f32>,
    /// The velocity lane's height in logical pixels, as its handle left it; written only when
    /// not the default.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    lane_height: Option<f32>,
    /// Whether the view jumps a page as the playhead leaves it while playing; written only
    /// when off.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    follow: Option<bool>,
    /// The piano roll's settings this build does not know, written back as they were read.
    #[serde(flatten)]
    rest: Map<String, Value>,
}

impl RollSettings {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The playlist's settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistSettings {
    /// Whether the view jumps a page as the song position leaves it while playing; written
    /// only when off.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    follow: Option<bool>,
    /// The lane height chosen (Alt+wheel), while the lanes do not fit the panel; written only
    /// then.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    lane_height: Option<f32>,
    /// The playlist's settings this build does not know, written back as they were read.
    #[serde(flatten)]
    rest: Map<String, Value>,
}

impl PlaylistSettings {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The mixer's settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MixerSettings {
    /// The panel's height as the divider over it left it (its handle's before every panel had
    /// one: the same key); written only when not the default.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    height: Option<f32>,
    /// The mixer's settings this build does not know, written back as they were read.
    #[serde(flatten)]
    rest: Map<String, Value>,
}

impl MixerSettings {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The Library's settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LibrarySettings {
    /// The groups of the Instruments section folded, by name ([`crate::ui::library::fold_key`]:
    /// "Plugins", "Instrument sets/Drums", "Plugins/Example"); written only when one is. A
    /// value that is no list of names folds nothing.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Vec::is_empty"
    )]
    folded: Vec<String>,
    /// The Library's settings this build does not know, written back as they were read.
    #[serde(flatten)]
    rest: Map<String, Value>,
}

impl LibrarySettings {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The first-run notes put away, each once per install.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Callouts {
    /// The note saying the computer keyboard plays notes.
    #[serde(default)]
    pub keyboard_shown: bool,
    /// The note over Record saying how to record notes; written only once put away.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub record_shown: bool,
    /// The notes this build does not know, written back as they were read.
    #[serde(flatten)]
    rest: Map<String, Value>,
}

impl Callouts {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// A value of a key this build knows, read as its default when it cannot be: one bad value
/// then costs that setting alone.
fn lenient<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    let value = Value::deserialize(deserializer)?;
    Ok(T::deserialize(value).unwrap_or_default())
}

impl Settings {
    /// Whether the piano roll plays the notes drawn, moved and clicked: on unless switched off.
    pub fn preview_notes(&self) -> bool {
        self.preview_notes != Some(false)
    }

    /// Switches the piano roll's note preview on or off.
    pub fn set_preview_notes(&mut self, on: bool) {
        self.preview_notes = (!on).then_some(false);
    }

    /// Whether the sample editor's selection snaps to zero crossings: on unless switched off.
    pub fn snap_to_zero(&self) -> bool {
        self.snap_to_zero != Some(false)
    }

    /// Switches the sample editor's Snap to zero on or off.
    pub fn set_snap_to_zero(&mut self, on: bool) {
        self.snap_to_zero = (!on).then_some(false);
    }

    /// Whether the hint bar shows: on unless switched off.
    pub fn show_hints(&self) -> bool {
        self.show_hints != Some(false)
    }

    /// Switches the hint bar on or off.
    pub fn set_show_hints(&mut self, on: bool) {
        self.show_hints = (!on).then_some(false);
    }

    /// The output device chosen, as Settings lists it (`sink:<node name>`, or its name); none
    /// for the system's default.
    pub fn audio_output(&self) -> Option<&str> {
        self.audio_output.as_deref()
    }

    /// The name the chosen output was shown by, when it is not what
    /// [`audio_output`](Self::audio_output) holds.
    pub fn audio_output_name(&self) -> Option<&str> {
        self.audio_output_name.as_deref()
    }

    /// Chooses the output device kept and shown as `name`, or the system's default (none).
    #[cfg(test)]
    pub fn set_audio_output(&mut self, name: Option<String>) {
        self.choose_audio_output(name, None);
    }

    /// Chooses the output device Settings lists as `id` and shows as `name`, or the system's
    /// default (none). The name is kept only when it is not the id.
    pub fn choose_audio_output(&mut self, id: Option<String>, name: Option<String>) {
        self.audio_output_name = name.filter(|name| id.as_ref().is_some_and(|id| id != name));
        self.audio_output = id;
    }

    /// The input device a new input channel plays, by name; none for the system's default.
    pub fn audio_input(&self) -> Option<&str> {
        self.audio_input.as_deref()
    }

    /// Chooses the input device `name` for new input channels, or the system's default (none).
    pub fn set_audio_input(&mut self, name: Option<String>) {
        self.audio_input = name;
    }

    /// Whether the computer keyboard plays notes: on unless switched off.
    pub fn keyboard_plays_notes(&self) -> bool {
        self.keyboard_plays_notes != Some(false)
    }

    /// Switches the computer keyboard's notes on or off.
    pub fn set_keyboard_plays_notes(&mut self, on: bool) {
        self.keyboard_plays_notes = (!on).then_some(false);
    }

    /// Whether the note saying the computer keyboard plays notes was put away.
    pub fn keyboard_callout_shown(&self) -> bool {
        self.callouts.keyboard_shown
    }

    /// The note saying the computer keyboard plays notes was put away: it never shows again.
    pub fn put_away_keyboard_callout(&mut self) {
        self.callouts.keyboard_shown = true;
    }

    /// Whether the note over Record saying how to record notes was put away.
    pub fn record_callout_shown(&self) -> bool {
        self.callouts.record_shown
    }

    /// The note over Record was put away: it never shows again.
    pub fn put_away_record_callout(&mut self) {
        self.callouts.record_shown = true;
    }

    /// The shortcuts: the defaults, with the actions the file rebinds by a text.
    pub fn keys(&self) -> Keys {
        let texts: BTreeMap<String, String> = self
            .keys
            .iter()
            .filter_map(|(name, value)| Some((name.clone(), value.as_str()?.to_owned())))
            .collect();
        Keys::from_saved(&texts)
    }

    /// Keeps `keys`: each action bound otherwise than by default, by its name; the names this
    /// build does not know stay as they were.
    pub fn set_keys(&mut self, keys: &Keys) {
        self.keys
            .retain(|name, _| crate::keys::Action::from_id(name).is_none());
        self.keys.extend(
            keys.saved()
                .into_iter()
                .map(|(name, chords)| (name, Value::String(chords))),
        );
    }

    /// The piano roll's velocity lane height: as its handle left it, kept to the heights the
    /// handle drags it to ([`piano_layout::kept_lane`]), the default until it is dragged.
    pub fn lane_height(&self) -> f32 {
        self.piano_roll
            .lane_height
            .map_or(piano_layout::LANE_HEIGHT, piano_layout::kept_lane)
    }

    /// Keeps the velocity lane `height` tall; the default is written as nothing.
    pub fn set_lane_height(&mut self, height: f32) {
        let height = piano_layout::kept_lane(height);
        self.piano_roll.lane_height = (height != piano_layout::LANE_HEIGHT).then_some(height);
    }

    /// The heights the dividers between the stacked panels left the rack, the piano roll and
    /// the mixer ([`Kept`]): a stored value that is no number is none; the mixer's is never
    /// under the panels' least height, nor any under nothing (the stack keeps each at least at
    /// its least height).
    pub fn kept_heights(&self) -> Kept {
        let number = |height: Option<f32>| height.filter(|height| height.is_finite());
        Kept {
            rack: number(self.rack.height).map(|height| height.max(0.0)),
            piano_roll: number(self.piano_roll.height).map(|height| height.max(0.0)),
            mixer: number(self.mixer.height)
                .map(|height| height.max(crate::ui::panels::MIN_HEIGHT)),
        }
    }

    /// Keeps `kept` as the dividers left it; a height not kept, and the mixer's default, are
    /// written as nothing.
    pub fn set_kept_heights(&mut self, kept: &Kept) {
        self.rack.height = kept.rack;
        self.piano_roll.height = kept.piano_roll;
        self.mixer.height = kept
            .mixer
            .filter(|&height| height != mixer_layout::DEFAULT_HEIGHT);
    }

    /// The Library's groups folded, by name.
    pub fn library_folded(&self) -> std::collections::BTreeSet<String> {
        self.library.folded.iter().cloned().collect()
    }

    /// Keeps `folded` as the Library's groups folded; none is written as nothing.
    pub fn set_library_folded(&mut self, folded: &std::collections::BTreeSet<String>) {
        self.library.folded = folded.iter().cloned().collect();
    }

    /// The playlist's lanes: fitting the panel, the default, or the height chosen.
    pub fn playlist_lanes(&self) -> Lanes {
        self.playlist
            .lane_height
            .filter(|height| height.is_finite())
            .map_or(Lanes::Fit, |height| {
                Lanes::Fixed(playlist_layout::kept_lane(height))
            })
    }

    /// Keeps the playlist's lanes `lanes`; fitting the panel is written as nothing.
    pub fn set_playlist_lanes(&mut self, lanes: Lanes) {
        self.playlist.lane_height = match lanes {
            Lanes::Fit => None,
            Lanes::Fixed(height) => Some(height),
        };
    }

    /// Whether `panel`'s view jumps a page as the playhead leaves it while playing: on unless
    /// switched off, for the piano roll and the playlist; no other panel follows.
    pub fn follows(&self, panel: Panel) -> bool {
        match panel {
            Panel::PianoRoll => self.piano_roll.follow != Some(false),
            Panel::Playlist => self.playlist.follow != Some(false),
            _ => false,
        }
    }

    /// Switches `panel`'s Follow on or off; a panel without one changes nothing.
    pub fn set_follows(&mut self, panel: Panel, on: bool) {
        let off = (!on).then_some(false);
        match panel {
            Panel::PianoRoll => self.piano_roll.follow = off,
            Panel::Playlist => self.playlist.follow = off,
            _ => {}
        }
    }

    /// Chooses the note layout `layout`.
    pub fn set_note_layout(&mut self, layout: NoteLayout) {
        self.note_layout = (layout != NoteLayout::default()).then(|| layout.id().to_owned());
    }

    /// The note layout the keyboard plays: the piano row unless another is chosen.
    pub fn note_layout(&self) -> NoteLayout {
        self.note_layout
            .as_deref()
            .and_then(NoteLayout::from_id)
            .unwrap_or_default()
    }

    /// The latency kept for `input` recording while `output` plays ("" for a system default).
    pub fn input_latency(&self, output: &str, input: &str) -> Option<&InputLatency> {
        self.input_latency
            .iter()
            .find(|kept| kept.output == output && kept.input == input)
    }

    /// Keeps `latency` in place of the one its devices had; `forget` takes theirs out.
    pub fn set_input_latency(&mut self, latency: InputLatency) {
        self.forget_input_latency(&latency.output, &latency.input);
        self.input_latency.push(latency);
    }

    /// Takes out the latency kept for `input` recording while `output` plays.
    pub fn forget_input_latency(&mut self, output: &str, input: &str) {
        self.input_latency
            .retain(|kept| kept.output != output || kept.input != input);
    }

    /// The settings in `file`; the defaults when there is none, or it is no JSON object. A
    /// value this build cannot read (a newer build's, say) is the default, and the keys beside
    /// it are kept.
    pub fn load(file: Option<&Path>) -> Self {
        let Some(text) = file.and_then(|file| fs::read_to_string(file).ok()) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_else(|_| {
            let mut rest: Map<String, Value> = serde_json::from_str(&text).unwrap_or_default();
            rest.remove("midi_inputs");
            // Taken out of the rest too, so it is never written twice.
            let window = rest
                .remove("window")
                .and_then(|value| serde_json::from_value(value).ok());
            let instrument_window = rest
                .remove("instrument_window")
                .and_then(|value| serde_json::from_value(value).ok());
            let settings_window = rest
                .remove("settings_window")
                .and_then(|value| serde_json::from_value(value).ok());
            let help_window = rest
                .remove("help_window")
                .and_then(|value| serde_json::from_value(value).ok());
            let params_window = rest
                .remove("params_window")
                .and_then(|value| serde_json::from_value(value).ok());
            let sample_editor_window = rest
                .remove("sample_editor_window")
                .and_then(|value| serde_json::from_value(value).ok());
            let mut flag = |key: &str| {
                rest.remove(key)
                    .and_then(|value| serde_json::from_value(value).ok())
            };
            let (preview_notes, show_hints, keyboard_plays_notes, snap_to_zero) = (
                flag("preview_notes"),
                flag("show_hints"),
                flag("keyboard_plays_notes"),
                flag("snap_to_zero"),
            );
            let mut text = |key: &str| {
                rest.remove(key)
                    .and_then(|value| serde_json::from_value(value).ok())
            };
            let (audio_output, audio_output_name, audio_input, note_layout) = (
                text("audio_output"),
                text("audio_output_name"),
                text("audio_input"),
                text("note_layout"),
            );
            let mut read = |key: &str| rest.remove(key).unwrap_or(Value::Null);
            let keys = serde_json::from_value(read("keys")).unwrap_or_default();
            let callouts = serde_json::from_value(read("callouts")).unwrap_or_default();
            let rack = serde_json::from_value(read("rack")).unwrap_or_default();
            let piano_roll = serde_json::from_value(read("piano_roll")).unwrap_or_default();
            let playlist = serde_json::from_value(read("playlist")).unwrap_or_default();
            let mixer = serde_json::from_value(read("mixer")).unwrap_or_default();
            let library = serde_json::from_value(read("library")).unwrap_or_default();
            let input_latency = serde_json::from_value(read("input_latency")).unwrap_or_default();
            Self {
                midi_inputs: MidiInputs::default(),
                window,
                instrument_window,
                settings_window,
                help_window,
                params_window,
                sample_editor_window,
                snap_to_zero,
                preview_notes,
                show_hints,
                audio_output,
                audio_output_name,
                audio_input,
                keyboard_plays_notes,
                note_layout,
                keys,
                callouts,
                rack,
                piano_roll,
                playlist,
                mixer,
                library,
                input_latency,
                rest,
            }
        })
    }

    /// Writes the settings to `file`, atomically, making its folder when needed. A file there
    /// that is no JSON object is moved to `<file>.bak` first, and while that name is taken
    /// nothing is written. Why: such a file is a hand edit gone wrong or a format this build
    /// does not know, and overwriting it would lose it.
    pub fn save(&self, file: &Path) -> Result<(), SaveError> {
        let failed = |source| SaveError::Write {
            path: file.to_path_buf(),
            source,
        };
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir).map_err(failed)?;
        }
        let unreadable = match fs::read_to_string(file) {
            Ok(text) => serde_json::from_str::<Map<String, Value>>(&text).is_err(),
            Err(error) => error.kind() != io::ErrorKind::NotFound,
        };
        if unreadable {
            let mut backup = file.as_os_str().to_owned();
            backup.push(".bak");
            let backup = PathBuf::from(backup);
            if backup.exists() {
                return Err(failed(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "it cannot be read, and {} is taken for a copy of it",
                        backup.display()
                    ),
                )));
            }
            fs::rename(file, &backup).map_err(failed)?;
        }
        let mut text =
            serde_json::to_string_pretty(self).map_err(|source| SaveError::Serialize {
                path: file.to_path_buf(),
                source,
            })?;
        text.push('\n');
        project_file::write_atomically(file, text.as_bytes())
    }
}

/// `dawsome/settings.json` in the settings folder.
pub fn settings_file() -> Option<PathBuf> {
    project_file::config_dir().map(|dir| dir.join("dawsome/settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(frames: i64, rate: u32, measured: bool) -> InputLatency {
        InputLatency {
            output: String::new(),
            input: "USB Audio Line Input".to_owned(),
            input_latency_frames: frames,
            rate,
            measured,
        }
    }

    // gen_p23: a latency is kept per output and input device, read back as written beside the
    // keys this build does not know, replaced for the same devices and forgotten alone; a list
    // this build cannot read is none, the other keys keeping theirs. 1134 frames at 44.1 kHz are
    // 1234.28 at 48 kHz: 1234; 1234 at 48 kHz show as 25.7 ms, -960 as -20.0 ms.
    #[test]
    fn an_input_latency_is_kept_per_output_and_input() {
        let dir = std::env::temp_dir().join(format!("dawsome-latency-set-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let file = dir.join("settings.json");
        fs::create_dir_all(&dir).expect("temp dir");
        fs::write(&file, r#"{"newer":1}"#).expect("writes");
        let mut settings = Settings::load(Some(&file));
        settings.set_input_latency(kept(1134, 44_100, true));
        let other = InputLatency {
            output: "Speakers".to_owned(),
            ..kept(512, 48_000, false)
        };
        settings.set_input_latency(other.clone());
        settings.save(&file).expect("saves");
        let text = fs::read_to_string(&file).expect("reads");
        assert!(text.contains(r#""input_latency_frames": 1134"#), "{text}");
        let loaded = Settings::load(Some(&file));
        assert_eq!(loaded, settings);
        assert_eq!(loaded.rest.get("newer"), Some(&Value::from(1)));
        let line = loaded
            .input_latency("", "USB Audio Line Input")
            .expect("kept");
        assert_eq!(line.frames_at(48_000), 1234);
        assert_eq!(
            loaded.input_latency("Speakers", "USB Audio Line Input"),
            Some(&other)
        );
        assert_eq!(loaded.input_latency("", ""), None);
        settings.set_input_latency(kept(-960, 48_000, false));
        assert_eq!(settings.input_latency.len(), 2, "replaced, not added");
        settings.forget_input_latency("", "USB Audio Line Input");
        assert_eq!(settings.input_latency, [other]);
        fs::write(
            &file,
            r#"{"input_latency":[{"output":3}],"show_hints":false}"#,
        )
        .expect("w");
        let unreadable = Settings::load(Some(&file));
        assert!(unreadable.input_latency.is_empty());
        assert!(!unreadable.show_hints());
        assert_eq!(kept(1234, 48_000, true).words(), "25.7 ms (measured)");
        assert_eq!(kept(-960, 48_000, false).words(), "-20.0 ms (set by hand)");
        assert_eq!(kept(-1134, 44_100, true).frames_at(48_000), -1234);
    }

    fn names(list: &[&str]) -> MidiInputs {
        MidiInputs::Named(list.iter().map(|name| (*name).to_owned()).collect())
    }

    // On Linux a port's name ends in the system's client and port numbers, which change from
    // one session to the next. A port chosen under its former numbers still plays; choosing a
    // port stores its name without them; switching the renumbered port off forgets the former
    // entry. A name with no such numbers (every Windows name) is kept whole.
    #[test]
    fn a_port_is_chosen_by_its_name_without_the_systems_numbers() {
        let mut inputs = names(&["USB Keyboard:USB Keyboard MIDI 1 32:0"]);
        assert!(inputs.plays("USB Keyboard:USB Keyboard MIDI 1 36:0"));
        assert!(!inputs.plays("USB Keyboard:USB Keyboard MIDI 2 36:1"));
        let listed = [
            "USB Keyboard:USB Keyboard MIDI 1 36:0".to_owned(),
            "2- Digital Piano".to_owned(),
        ];
        inputs.toggle(&listed[0], &listed);
        assert_eq!(inputs, MidiInputs::default(), "the former entry goes");
        inputs.toggle(&listed[0], &listed);
        assert_eq!(inputs, names(&["USB Keyboard:USB Keyboard MIDI 1"]));
        inputs.toggle(&listed[1], &listed);
        assert_eq!(
            inputs,
            names(&["2- Digital Piano", "USB Keyboard:USB Keyboard MIDI 1"])
        );
        let mut all = MidiInputs::All;
        all.toggle(&listed[1], &listed);
        assert_eq!(all, names(&["USB Keyboard:USB Keyboard MIDI 1"]));
    }

    #[test]
    fn inputs_read_and_write_as_all_or_a_list_and_keep_unknown_keys() {
        let all: Settings =
            serde_json::from_str(r#"{"midi_inputs":"all","theme":"dark"}"#).expect("reads");
        assert_eq!(all.midi_inputs, MidiInputs::All);
        assert_eq!(
            serde_json::to_string(&all).expect("writes"),
            r#"{"midi_inputs":"all","theme":"dark"}"#
        );
        let two: Settings =
            serde_json::from_str(r#"{"midi_inputs":["Port B","Port A"]}"#).expect("reads");
        assert_eq!(two.midi_inputs, names(&["Port A", "Port B"]));
        assert_eq!(
            serde_json::to_string(&two).expect("writes"),
            r#"{"midi_inputs":["Port A","Port B"]}"#
        );
        assert!(serde_json::from_str::<Settings>(r#"{"midi_inputs":"some"}"#).is_err());
        let empty: Settings = serde_json::from_str("{}").expect("reads");
        assert_eq!(empty.midi_inputs, MidiInputs::default());
        assert!(!empty.midi_inputs.plays("Port A"), "no port plays at first");
    }

    #[test]
    fn switching_one_port_from_all_keeps_the_others() {
        let listed = ["Port A".to_owned(), "Port B".to_owned()];
        let mut inputs = MidiInputs::All;
        inputs.toggle("Port A", &listed);
        assert_eq!(inputs, names(&["Port B"]));
        inputs.toggle("Port A", &listed);
        assert_eq!(inputs, names(&["Port A", "Port B"]));
        inputs.toggle_all();
        assert_eq!(inputs, MidiInputs::All);
        inputs.toggle_all();
        assert_eq!(inputs, names(&[]));
    }

    /// A settings file in a temp folder of `test`'s own, holding `text`.
    fn settings_file_with(test: &str, text: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("dawsome-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let file = dir.join("dawsome/settings.json");
        fs::create_dir_all(file.parent().expect("a folder")).expect("folder made");
        fs::write(&file, text).expect("written");
        (dir, file)
    }

    // A file this build cannot read whole, as a newer build may write it: its `midi_inputs` is
    // the default, and the keys this build does not know survive the next save.
    #[test]
    fn a_value_this_build_cannot_read_keeps_the_other_keys() {
        let (dir, file) =
            settings_file_with("settings-newer", r#"{"midi_inputs":"some","theme":"dark"}"#);
        let mut settings = Settings::load(Some(&file));
        assert_eq!(settings.midi_inputs, MidiInputs::default());
        settings.midi_inputs = MidiInputs::All;
        settings.save(&file).expect("saves");
        let saved: Value =
            serde_json::from_str(&fs::read_to_string(&file).expect("reads")).expect("parses");
        assert_eq!(
            saved,
            serde_json::json!({"midi_inputs": "all", "theme": "dark"})
        );
        fs::remove_dir_all(&dir).expect("removed");
    }

    // A file that is no JSON object (a hand edit gone wrong) is not overwritten: a save moves
    // it to settings.json.bak first, and while that is taken it saves nothing.
    #[test]
    fn a_file_that_is_no_json_object_is_kept_aside_before_a_save() {
        let broken = r#"{"midi_inputs": ["Port A",}"#;
        let (dir, file) = settings_file_with("settings-broken", broken);
        let backup = dir.join("dawsome/settings.json.bak");
        let mut settings = Settings::load(Some(&file));
        assert_eq!(settings, Settings::default());
        settings.midi_inputs = MidiInputs::All;
        settings.save(&file).expect("saves");
        assert_eq!(fs::read_to_string(&backup).expect("kept aside"), broken);
        assert_eq!(Settings::load(Some(&file)).midi_inputs, MidiInputs::All);
        fs::write(&file, "not json").expect("written");
        assert!(settings.save(&file).is_err(), "the copy's name is taken");
        assert_eq!(fs::read_to_string(&file).expect("reads"), "not json");
        assert_eq!(fs::read_to_string(&backup).expect("reads"), broken);
        fs::remove_dir_all(&dir).expect("removed");
    }

    // A first start (nothing saved) opens the Library, the rack and the playlist, maximized
    // over a 1280 x 800 window; a saved layout opens as it closed; one naming no panel this
    // build knows opens the rack, and one too small to use opens 1280 x 800. And
    // the Settings window is kept out of the layout, and "settings" an earlier build saved
    // opens nothing.
    #[test]
    fn the_window_opens_as_it_closed_or_as_a_first_start() {
        let first = start_window(None);
        assert_eq!(
            first,
            StartWindow {
                panels: Panels::FIRST_START,
                size: Size::new(1280.0, 800.0),
                maximized: true,
            }
        );
        let mut mixer = Panels::NONE;
        mixer.set(Panel::Mixer, true);
        mixer.set(Panel::PianoRoll, true);
        let saved = WindowLayout::of(mixer, Size::new(1100.0, 700.0), false);
        assert_eq!(saved.panels, ["piano-roll", "mixer"]);
        assert_eq!(
            start_window(Some(&saved)),
            StartWindow {
                panels: mixer,
                size: Size::new(1100.0, 700.0),
                maximized: false,
            }
        );
        let unknown = WindowLayout {
            panels: vec!["timeline".to_owned()],
            width: 10.0,
            height: f32::NAN,
            maximized: true,
        };
        assert_eq!(
            start_window(Some(&unknown)),
            StartWindow {
                panels: Panels::default(),
                size: FIRST_START_SIZE,
                maximized: true,
            }
        );
        let mut with_settings = mixer;
        with_settings.set(Panel::Settings, true);
        let saved = WindowLayout::of(with_settings, Size::new(1100.0, 700.0), false);
        assert_eq!(saved.panels, ["piano-roll", "mixer"]);
        let earlier = WindowLayout {
            panels: vec!["rack".to_owned(), "settings".to_owned()],
            ..saved
        };
        let opened = start_window(Some(&earlier)).panels;
        assert!(opened.rack && !opened.settings, "{opened:?}");
        let alone = WindowLayout {
            panels: vec!["settings".to_owned()],
            ..earlier
        };
        assert_eq!(start_window(Some(&alone)).panels, Panels::default());
    }

    // The layout reads and writes beside the inputs; one this build cannot read is none, and
    // the inputs beside it still read.
    #[test]
    fn the_window_layout_reads_and_writes_beside_the_inputs() {
        let text = r#"{"midi_inputs":"all","window":{"panels":["browser","rack"],"width":1280.0,"height":800.0,"maximized":true}}"#;
        let read: Settings = serde_json::from_str(text).expect("reads");
        assert_eq!(read.midi_inputs, MidiInputs::All);
        assert_eq!(
            read.window,
            Some(WindowLayout {
                panels: vec!["browser".to_owned(), "rack".to_owned()],
                width: 1280.0,
                height: 800.0,
                maximized: true,
            })
        );
        assert_eq!(serde_json::to_string(&read).expect("writes"), text);
        let bad: Settings =
            serde_json::from_str(r#"{"midi_inputs":"all","window":"wide"}"#).expect("reads");
        assert_eq!((bad.midi_inputs, bad.window), (MidiInputs::All, None));
    }

    // The piano roll's note preview is on unless the file says false, and is written only off.
    #[test]
    fn the_note_preview_is_on_unless_switched_off() {
        let mut settings = Settings::default();
        assert!(settings.preview_notes());
        settings.set_preview_notes(false);
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[],"preview_notes":false}"#
        );
        settings.set_preview_notes(true);
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[]}"#
        );
        let odd: Settings = serde_json::from_str(r#"{"preview_notes":"yes"}"#).expect("reads");
        assert!(odd.preview_notes(), "a value it cannot read is the default");
    }

    // The velocity lane's height is written under "piano_roll" only when the
    // handle left it off the default (64); it reads back kept to the handle's limits (32..120),
    // a value it cannot read is the default, and the piano roll's keys this build does not know
    // stay as they were.
    #[test]
    fn the_lane_height_is_kept_only_off_its_default() {
        let mut settings = Settings::default();
        assert_eq!(settings.lane_height(), 64.0);
        settings.set_lane_height(94.0);
        let written = serde_json::to_string(&settings).expect("writes");
        assert_eq!(
            written,
            r#"{"midi_inputs":[],"piano_roll":{"lane_height":94.0}}"#
        );
        let read: Settings = serde_json::from_str(&written).expect("reads");
        assert_eq!(read.lane_height(), 94.0);
        settings.set_lane_height(64.0);
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[]}"#
        );
        let tall: Settings =
            serde_json::from_str(r#"{"piano_roll":{"lane_height":900}}"#).expect("reads");
        // P2, pin moved: 120 once the grid's 10 px scroll bar sits under the grid (130 in P1).
        assert_eq!(tall.lane_height(), 120.0);
        let odd: Settings =
            serde_json::from_str(r#"{"piano_roll":{"lane_height":"tall","zoom":3}}"#)
                .expect("reads");
        assert_eq!(odd.lane_height(), 64.0);
        assert_eq!(
            serde_json::to_string(&odd).expect("writes"),
            r#"{"midi_inputs":[],"piano_roll":{"zoom":3}}"#
        );
        // A file this build cannot read whole still gives its lane height.
        let (dir, file) = settings_file_with(
            "lane_height",
            r#"{"midi_inputs":"some","piano_roll":{"lane_height":40.0}}"#,
        );
        let beside = Settings::load(Some(&file));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(
            beside.lane_height(),
            40.0,
            "kept beside a value it cannot read"
        );
    }

    // The Library's folded groups are written under "library" as a
    // list of names, only while one is folded; read back as a set; a value that is no list
    // folds nothing, and the Library's keys this build does not know stay.
    #[test]
    fn the_library_folds_are_kept_only_while_one_is() {
        let mut settings = Settings::default();
        assert!(settings.library_folded().is_empty());
        let folded = std::collections::BTreeSet::from([
            "Plugins".to_owned(),
            "Instrument sets/Drums".to_owned(),
        ]);
        settings.set_library_folded(&folded);
        let written = serde_json::to_string(&settings).expect("writes");
        assert_eq!(
            written,
            r#"{"midi_inputs":[],"library":{"folded":["Instrument sets/Drums","Plugins"]}}"#
        );
        let read: Settings = serde_json::from_str(&written).expect("reads");
        assert_eq!(read.library_folded(), folded);
        settings.set_library_folded(&std::collections::BTreeSet::new());
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[]}"#
        );
        let mut odd: Settings =
            serde_json::from_str(r#"{"library":{"folded":"Plugins","width":300}}"#).expect("reads");
        assert!(odd.library_folded().is_empty());
        odd.set_library_folded(&std::collections::BTreeSet::from(
            ["Starter kit".to_owned()],
        ));
        assert_eq!(
            serde_json::to_string(&odd).expect("writes"),
            r#"{"midi_inputs":[],"library":{"folded":["Starter kit"],"width":300}}"#
        );
    }

    // The mixer's height is written under "mixer" only when its handle left it
    // off the default (400); it reads back no lower than the panels' least height (180), a value
    // that is no number is the default, and the mixer's keys this build does not know stay. The
    // playlist's lanes fit the panel unless a height was chosen, written only then and read back
    // kept to 32..96; a choice that is no number fits. (Resizable dividers: the mixer's height is
    // one of the kept heights, read and written through them under the same key.)
    #[test]
    fn the_mixer_height_and_the_playlist_lanes_are_kept_only_off_their_defaults() {
        let mixer = |height: f32| Kept {
            mixer: Some(height),
            ..Kept::default()
        };
        let mut settings = Settings::default();
        assert_eq!(settings.kept_heights().mixer_height(), 400.0);
        assert_eq!(settings.playlist_lanes(), Lanes::Fit);
        settings.set_kept_heights(&mixer(312.0));
        settings.set_playlist_lanes(Lanes::Fixed(64.0));
        let written = serde_json::to_string(&settings).expect("writes");
        assert_eq!(
            written,
            r#"{"midi_inputs":[],"playlist":{"lane_height":64.0},"mixer":{"height":312.0}}"#
        );
        let read: Settings = serde_json::from_str(&written).expect("reads");
        assert_eq!(read.kept_heights().mixer_height(), 312.0);
        assert_eq!(read.playlist_lanes(), Lanes::Fixed(64.0));
        settings.set_kept_heights(&mixer(400.0));
        settings.set_playlist_lanes(Lanes::Fit);
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[]}"#
        );
        let odd: Settings = serde_json::from_str(
            r#"{"mixer":{"height":12,"skin":"dark"},"playlist":{"lane_height":900}}"#,
        )
        .expect("reads");
        assert_eq!(odd.kept_heights().mixer_height(), 180.0);
        assert_eq!(odd.playlist_lanes(), Lanes::Fixed(96.0));
        let mut odd: Settings = serde_json::from_str(
            r#"{"mixer":{"height":"tall","skin":"dark"},"playlist":{"lane_height":"x"}}"#,
        )
        .expect("reads");
        assert_eq!(odd.kept_heights().mixer_height(), 400.0);
        assert_eq!(odd.playlist_lanes(), Lanes::Fit);
        odd.set_kept_heights(&mixer(250.0));
        assert_eq!(
            serde_json::to_string(&odd).expect("writes"),
            r#"{"midi_inputs":[],"mixer":{"height":250.0,"skin":"dark"}}"#
        );
    }

    // Resizable dividers: the heights the dividers left are written in each panel's own
    // section, the rack's under "rack", the piano roll's beside its lane's, the mixer's as
    // before, only once kept (none is written as nothing), and read back as written; a value
    // that is no number, or under nothing, keeps none or none less than nothing, and the
    // sections' keys this build does not know stay.
    #[test]
    fn the_kept_panel_heights_round_trip_and_only_once_kept() {
        let kept = Kept {
            rack: Some(150.0),
            piano_roll: Some(264.0),
            mixer: Some(420.0),
        };
        let mut settings = Settings::default();
        assert_eq!(settings.kept_heights(), Kept::default());
        settings.set_kept_heights(&kept);
        let written = serde_json::to_string(&settings).expect("writes");
        assert_eq!(
            written,
            r#"{"midi_inputs":[],"rack":{"height":150.0},"piano_roll":{"height":264.0},"mixer":{"height":420.0}}"#
        );
        let read: Settings = serde_json::from_str(&written).expect("reads");
        assert_eq!(read.kept_heights(), kept);
        settings.set_kept_heights(&Kept::default());
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[]}"#
        );
        let mut odd: Settings = serde_json::from_str(
            r#"{"rack":{"height":"tall","rows":3},"piano_roll":{"height":-5,"lane_height":96}}"#,
        )
        .expect("reads");
        assert_eq!(
            odd.kept_heights(),
            Kept {
                piano_roll: Some(0.0),
                ..Kept::default()
            }
        );
        odd.set_kept_heights(&Kept {
            rack: Some(100.0),
            ..Kept::default()
        });
        assert_eq!(
            serde_json::to_string(&odd).expect("writes"),
            r#"{"midi_inputs":[],"rack":{"height":100.0,"rows":3},"piano_roll":{"lane_height":96.0}}"#
        );
    }

    // P2: the piano roll follows the playhead unless switched off, written only off, beside
    // the lane's height. (P6, pin moved: the playlist has a switch of its own since; the mixer
    // has none.)
    #[test]
    fn the_piano_roll_follows_unless_switched_off() {
        let mut settings = Settings::default();
        assert!(settings.follows(Panel::PianoRoll));
        settings.set_follows(Panel::PianoRoll, false);
        settings.set_lane_height(40.0);
        let written = serde_json::to_string(&settings).expect("writes");
        assert_eq!(
            written,
            r#"{"midi_inputs":[],"piano_roll":{"lane_height":40.0,"follow":false}}"#
        );
        let read: Settings = serde_json::from_str(&written).expect("reads");
        assert!(!read.follows(Panel::PianoRoll));
        settings.set_follows(Panel::PianoRoll, true);
        settings.set_lane_height(64.0);
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[]}"#
        );
        assert!(!settings.follows(Panel::Mixer));
        // P6: the playlist's switch, apart from the roll's.
        settings.set_follows(Panel::Playlist, false);
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[],"playlist":{"follow":false}}"#
        );
        assert!(settings.follows(Panel::PianoRoll) && !settings.follows(Panel::Playlist));
    }

    // B8: the output device chosen is written by name, only when one is; the system default
    // writes nothing; it loads back from a saved file, survives a value of another key this
    // build cannot read, and a value it cannot read itself is the default.
    #[test]
    fn the_audio_output_reads_and_writes_by_name() {
        let mut settings = Settings::default();
        assert_eq!(settings.audio_output(), None);
        settings.set_audio_output(Some("USB Audio Analog Stereo".to_owned()));
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[],"audio_output":"USB Audio Analog Stereo"}"#
        );
        let (dir, file) = settings_file_with("settings-audio", "{}");
        settings.save(&file).expect("saves");
        assert_eq!(Settings::load(Some(&file)), settings);
        fs::write(
            &file,
            r#"{"midi_inputs":"some","audio_output":"USB Audio Analog Stereo"}"#,
        )
        .expect("written");
        assert_eq!(
            Settings::load(Some(&file)).audio_output(),
            Some("USB Audio Analog Stereo")
        );
        fs::remove_dir_all(&dir).expect("removed");
        settings.set_audio_output(None);
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[]}"#
        );
        let odd: Settings = serde_json::from_str(r#"{"audio_output":3}"#).expect("reads");
        assert_eq!(
            odd.audio_output(),
            None,
            "a value it cannot read is the default"
        );
    }

    // A sound server's sink is kept by "sink:" and its node name, with the name it was shown
    // by beside it; both load back, through the lenient reader too; a device kept by its name
    // writes no second name; the system default writes neither.
    #[test]
    fn a_sink_is_kept_by_its_node_and_named_as_it_was_shown() {
        let node = "sink:alsa_output.usb-SteelSeries_Arctis_Nova_7-00.analog-stereo";
        let mut settings = Settings::default();
        settings.choose_audio_output(
            Some(node.to_owned()),
            Some("Arctis Nova 7 Analog Stereo".to_owned()),
        );
        let written = concat!(
            r#"{"midi_inputs":[],"audio_output":"sink:alsa_output.usb-SteelSeries_Arctis_Nova_7-00"#,
            r#".analog-stereo","audio_output_name":"Arctis Nova 7 Analog Stereo"}"#
        );
        assert_eq!(serde_json::to_string(&settings).expect("writes"), written);
        let (dir, file) = settings_file_with("settings-sink", "{}");
        settings.save(&file).expect("saves");
        assert_eq!(Settings::load(Some(&file)), settings);
        let lenient = written.replace(r#""midi_inputs":[]"#, r#""midi_inputs":"some""#);
        fs::write(&file, lenient).expect("written");
        let loaded = Settings::load(Some(&file));
        assert_eq!(
            (loaded.audio_output(), loaded.audio_output_name()),
            (Some(node), Some("Arctis Nova 7 Analog Stereo"))
        );
        fs::remove_dir_all(&dir).expect("removed");
        settings.choose_audio_output(Some("Speakers".to_owned()), Some("Speakers".to_owned()));
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[],"audio_output":"Speakers"}"#
        );
        settings.choose_audio_output(None, Some("System default".to_owned()));
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[]}"#
        );
    }

    // The input device new input channels take: by name, written only when chosen, read back,
    // and a value it cannot read is the default (the system's input).
    #[test]
    fn the_audio_input_reads_and_writes_by_name() {
        let mut settings = Settings::default();
        assert_eq!(settings.audio_input(), None);
        settings.set_audio_input(Some("USB Audio Line Input".to_owned()));
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[],"audio_input":"USB Audio Line Input"}"#
        );
        let (dir, file) = settings_file_with("settings-audio-input", "{}");
        settings.save(&file).expect("saves");
        assert_eq!(Settings::load(Some(&file)), settings);
        fs::write(
            &file,
            r#"{"midi_inputs":"some","audio_input":"USB Audio Line Input"}"#,
        )
        .expect("written");
        assert_eq!(
            Settings::load(Some(&file)).audio_input(),
            Some("USB Audio Line Input"),
            "kept beside a value it cannot read"
        );
        fs::remove_dir_all(&dir).expect("removed");
        let odd: Settings = serde_json::from_str(r#"{"audio_input":[1]}"#).expect("reads");
        assert_eq!(odd.audio_input(), None);
    }

    // A saved size past any screen (a hand edit, a corrupt file) opens
    // within the largest a window takes, 7680 × 4320, each side on its own; a size that fits
    // is kept. Once open, a window larger than its monitor is fitted to it.
    #[test]
    fn a_saved_size_past_any_screen_is_bounded_and_fitted_to_the_monitor() {
        let huge = WindowLayout {
            panels: vec!["rack".to_owned()],
            width: 1e9,
            height: 900.0,
            maximized: false,
        };
        assert_eq!(start_window(Some(&huge)).size, Size::new(7680.0, 900.0));
        let tall = WindowLayout {
            height: f32::MAX,
            ..huge.clone()
        };
        assert_eq!(start_window(Some(&tall)).size, Size::new(7680.0, 4320.0));
        let monitor = Size::new(1920.0, 1080.0);
        assert_eq!(
            fit_to_monitor(Size::new(7680.0, 900.0), monitor),
            Some(Size::new(1920.0, 900.0))
        );
        assert_eq!(fit_to_monitor(Size::new(1280.0, 800.0), monitor), None);
        assert_eq!(fit_to_monitor(Size::new(1920.0, 1080.0), monitor), None);
    }

    // The instrument window opens where it last closed: its size and position read back, the
    // position none where the system gives a window none (Wayland). A size smaller than the
    // window's least (420 x 300) or not a number opens at its first size, 560 x 420; one past
    // any screen within 7680 x 4320, each side on its own; a position not a number is the
    // system's choice. A first start has no place saved and opens at the first size.
    #[test]
    fn the_instrument_windows_place_reads_back_and_is_bounded() {
        let saved: Settings = serde_json::from_str(
            r#"{"midi_inputs":[],"instrument_window":{"width":640.0,"height":480.0,"position":[100.0,-20.0]}}"#,
        )
        .expect("reads");
        let place = saved.instrument_window.expect("a place");
        assert_eq!(place.size(PopOut::Instrument), Size::new(640.0, 480.0));
        assert_eq!(place.position(), Some(Point::new(100.0, -20.0)));
        assert_eq!(
            serde_json::to_string(&saved).expect("writes"),
            r#"{"midi_inputs":[],"instrument_window":{"width":640.0,"height":480.0,"position":[100.0,-20.0]}}"#
        );
        let wayland = WindowPlace::of(Size::new(600.0, 400.0), None);
        assert_eq!(
            serde_json::to_string(&wayland).expect("writes"),
            r#"{"width":600.0,"height":400.0}"#
        );
        assert_eq!(wayland.position(), None);
        let small = WindowPlace::of(Size::new(419.0, 400.0), None);
        assert_eq!(small.size(PopOut::Instrument), Size::new(560.0, 420.0));
        let odd = WindowPlace {
            width: f32::NAN,
            height: 400.0,
            position: Some((f32::INFINITY, 0.0)),
        };
        assert_eq!(odd.size(PopOut::Instrument), Size::new(560.0, 420.0));
        assert_eq!(odd.position(), None);
        let huge = WindowPlace::of(Size::new(1e9, 500.0), Some(Point::new(0.0, 0.0)));
        assert_eq!(huge.size(PopOut::Instrument), Size::new(7680.0, 500.0));
        let first = WindowPlace::first(PopOut::Instrument);
        assert_eq!(first.size(PopOut::Instrument), Size::new(560.0, 420.0));
        assert_eq!(first.position(), None);
        let unreadable: Settings =
            serde_json::from_str(r#"{"instrument_window":"left"}"#).expect("reads");
        assert_eq!(
            unreadable.instrument_window, None,
            "a value it cannot read is the default"
        );
    }

    // 6d K1: the keyboard plays notes unless the file says false, and plays the piano row
    // unless it names the two rows; each is written back as read, the switch only off, an id this
    // build does not know plays the piano row and is written back, and a value it cannot read
    // is the default without the other keys losing theirs.
    #[test]
    fn the_keyboard_plays_the_piano_row_unless_the_file_says_otherwise() {
        let mut settings = Settings::default();
        assert!(settings.keyboard_plays_notes());
        assert_eq!(settings.note_layout(), NoteLayout::Piano);
        settings.set_keyboard_plays_notes(false);
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[],"keyboard_plays_notes":false}"#
        );
        settings.set_keyboard_plays_notes(true);
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[]}"#
        );
        let text = r#"{"midi_inputs":[],"keyboard_plays_notes":false,"note_layout":"two-rows"}"#;
        let read: Settings = serde_json::from_str(text).expect("reads");
        assert!(!read.keyboard_plays_notes());
        assert_eq!(read.note_layout(), NoteLayout::TwoRows);
        assert_eq!(serde_json::to_string(&read).expect("writes"), text);
        let newer: Settings =
            serde_json::from_str(r#"{"note_layout":"isomorphic"}"#).expect("reads");
        assert_eq!(newer.note_layout(), NoteLayout::Piano);
        assert_eq!(
            serde_json::to_string(&newer).expect("writes"),
            r#"{"midi_inputs":[],"note_layout":"isomorphic"}"#
        );
        let (dir, file) = settings_file_with(
            "settings-keyboard",
            r#"{"midi_inputs":"some","keyboard_plays_notes":false,"note_layout":"two-rows"}"#,
        );
        let loaded = Settings::load(Some(&file));
        assert!(!loaded.keyboard_plays_notes());
        assert_eq!(loaded.note_layout(), NoteLayout::TwoRows);
        let odd: Settings =
            serde_json::from_str(r#"{"keyboard_plays_notes":"yes","note_layout":3}"#)
                .expect("reads");
        assert!(odd.keyboard_plays_notes());
        assert_eq!(odd.note_layout(), NoteLayout::Piano);
        fs::remove_dir_all(&dir).expect("removed");
    }

    // 6d K3: the shortcuts bound otherwise are written under `keys` by action name; the
    // defaults write nothing; a name this build does not know is kept through a rebind and a
    // reset; a value that is no map is the default, the other keys keeping theirs; the layout
    // chosen is written only when not the piano row.
    #[test]
    fn rebound_keys_are_kept_by_name_and_unknown_names_survive() {
        use crate::keys::{Action, Chord};
        let mut settings: Settings =
            serde_json::from_str(r#"{"keys":{"teleport":"Ctrl+K"}}"#).expect("reads");
        assert_eq!(
            settings.keys(),
            Keys::default(),
            "an unknown name does nothing"
        );
        let mut keys = settings.keys();
        keys.bind(Action::ToggleMode, Chord::parse("Ctrl+M").expect("a chord"));
        settings.set_keys(&keys);
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[],"keys":{"mode":"Ctrl+M","teleport":"Ctrl+K"}}"#
        );
        let read: Settings =
            serde_json::from_str(&serde_json::to_string(&settings).expect("writes"))
                .expect("reads");
        assert_eq!(read.keys(), keys);
        settings.set_keys(&Keys::default());
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[],"keys":{"teleport":"Ctrl+K"}}"#
        );
        let (dir, file) = settings_file_with(
            "settings-keys",
            r#"{"midi_inputs":"some","keys":{"mode":"Ctrl+M"}}"#,
        );
        assert_eq!(
            Settings::load(Some(&file)).keys().first(Action::ToggleMode),
            "Ctrl+M"
        );
        let odd: Settings = serde_json::from_str(r#"{"keys":"all"}"#).expect("reads");
        assert_eq!(odd.keys(), Keys::default());
        settings.set_note_layout(NoteLayout::TwoRows);
        assert_eq!(settings.note_layout(), NoteLayout::TwoRows);
        settings.set_note_layout(NoteLayout::Piano);
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[],"keys":{"teleport":"Ctrl+K"}}"#
        );
        fs::remove_dir_all(&dir).expect("removed");
    }

    /// What `file` holds, read as JSON.
    fn written(file: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(file).expect("written")).expect("JSON")
    }

    // 6d review 5: a value under `keys` that is no string (a list, as a later build might
    // write one) costs that entry alone: the others bind, and the file is written back with it
    // as it was. A rebind replaces the names this build knows and keeps the rest.
    #[test]
    fn a_keys_value_that_is_no_string_costs_that_entry_alone() {
        use crate::keys::{Action, Chord};
        let text = r#"{"keys":{"later":["Ctrl+K"],"mode":"Ctrl+M","undo":42}}"#;
        let (dir, file) = settings_file_with("settings-keys-odd", text);
        let mut settings = Settings::load(Some(&file));
        let mut keys = settings.keys();
        assert_eq!(keys.first(Action::ToggleMode), "Ctrl+M");
        assert_eq!(keys.first(Action::Undo), "Ctrl+Z", "a number binds nothing");
        settings.save(&file).expect("saves");
        assert_eq!(
            written(&file),
            serde_json::json!({"midi_inputs": [], "keys": {"later": ["Ctrl+K"], "mode": "Ctrl+M", "undo": 42}})
        );
        keys.bind(Action::Save, Chord::parse("Ctrl+W").expect("a chord"));
        settings.set_keys(&keys);
        assert_eq!(
            serde_json::to_string(&settings).expect("writes"),
            r#"{"midi_inputs":[],"keys":{"later":["Ctrl+K"],"mode":"Ctrl+M","save":"Ctrl+W"}}"#
        );
        fs::remove_dir_all(&dir).expect("removed");
    }

    // 6d review 6: a chord on a key that types a letter outside ASCII is written as typed and
    // loads back unchanged: Ctrl+ß (a German keyboard's key right of 0) is still Save's after
    // the file is saved and read again.
    #[test]
    fn a_chord_on_a_key_outside_ascii_loads_back_unchanged() {
        use crate::keys::{Action, Chord};
        use iced::keyboard::{Key, Modifiers};
        let (dir, file) = settings_file_with("settings-keys-sharp-s", "{}");
        let mut settings = Settings::load(Some(&file));
        let mut keys = settings.keys();
        let sharp = Chord::of(Key::Character("ß"), Modifiers::CTRL).expect("a chord");
        keys.bind(Action::Save, sharp);
        settings.set_keys(&keys);
        settings.save(&file).expect("saves");
        assert_eq!(
            written(&file),
            serde_json::json!({"midi_inputs": [], "keys": {"save": "Ctrl+ß"}})
        );
        let read = Settings::load(Some(&file)).keys();
        assert_eq!(read, keys);
        assert_eq!(
            read.action_of(Key::Character("ß"), Modifiers::CTRL, false),
            Some(Action::Save)
        );
        fs::remove_dir_all(&dir).expect("removed");
    }

    // 6d K4: the keyboard's first-run note, once put away, is written so and read back; the
    // notes this build does not know stay; nothing is written before.
    #[test]
    fn the_keyboard_note_put_away_is_kept() {
        let mut settings: Settings =
            serde_json::from_str(r#"{"callouts":{"later":true}}"#).expect("reads");
        assert!(!settings.keyboard_callout_shown());
        settings.put_away_keyboard_callout();
        let text = serde_json::to_string(&settings).expect("writes");
        assert_eq!(
            text,
            r#"{"midi_inputs":[],"callouts":{"keyboard_shown":true,"later":true}}"#
        );
        let read: Settings = serde_json::from_str(&text).expect("reads");
        assert!(read.keyboard_callout_shown());
        assert_eq!(
            serde_json::to_string(&Settings::default()).expect("writes"),
            r#"{"midi_inputs":[]}"#
        );
        let odd: Settings = serde_json::from_str(r#"{"callouts":"seen"}"#).expect("reads");
        assert!(!odd.keyboard_callout_shown());
    }

    #[test]
    fn a_saved_file_loads_back_and_a_missing_one_is_the_default() {
        let dir = std::env::temp_dir().join(format!("dawsome-settings-{}", std::process::id()));
        let file = dir.join("dawsome/settings.json");
        let mut settings = Settings::load(Some(&file));
        assert_eq!(settings, Settings::default());
        settings.midi_inputs = MidiInputs::All;
        settings.save(&file).expect("saves");
        assert_eq!(Settings::load(Some(&file)), settings);
        fs::remove_dir_all(&dir).expect("removed");
    }
}
