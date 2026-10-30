//! Command-line flags.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::Duration;

use dawsome_engine::project::{ChannelId, EffectId, ProcessorOwner, SAMPLE_RATE_RANGE};
use dawsome_host_clap::catalogue::PluginKind;
use dawsome_media::BitDepth;
use iced::Size;
use thiserror::Error;

use crate::headless::{ScreenshotRequest, Script, ShotWindow};
use crate::play_to::{PlayTo, RecordSource};
use crate::ui::layout::DEFAULT_WINDOW_SIZE;
use crate::ui::panels::{Panel, Panels};

/// Shown after any flag error.
pub const USAGE: &str = "usage: dawsome-app [[--load] <project.dawsome|project.json>]
                   [--window-size <W>x<H>]
                   [--panels <list>] [--song] [--device-rate <Hz>]
                   [--plugin-path <dir>].. [--cache <file>] [--stream-threshold <frames>]
                   [--browser-root <dir>] [--cache-dir <dir>] [--midi-ports none|<names>]
                   [--midi-out-ports none|<names>]
                   [--screenshot <out.png> --screenshot-after <ms>
                   [--screenshot-window main|instrument|settings|help|params|sample-editor]
                   [--screenshot-every <ms>]]
                   [--quit-after <ms>] [--null-output] [--rehearse <events.json>]
                   [--open-picker instrument|effect] [--dump-picker <file>]
                   [--open-plugin channel:<id>|effect:<id>]
                   [--open-params channel:<id>|effect:<id>] [--dump-params <file>]
                   [--play] [--stats]
                   [--open-adder] [--hints on|off] [--open-instrument channel:<id>]
                   [--library-preview <set.sfz>] [--show export|edit-menu|help[:<n>]]
                   [--settings-page general|keys] [--callouts on|off] [--scroll end]
                   [--drag-over channel:<id>] [--channel-menu channel:<id>] [--select-clip clip:<id>]
                   [--curve-menu automation:<id>] [--slot-menu insert:<id>,slot:<n>]
                   [--library-menu device:<port>] [--midi-monitor <port>] [--drop-monitor]
                   [--open-sample-editor channel:<id>|clip:<id>
                   [--sample-editor-select <from>-<to>]]
       dawsome-app --list-instruments
       dawsome-app --list-plugin-folders
       dawsome-app --list-help
       dawsome-app --list-outputs
       dawsome-app --version
       dawsome-app --help
       dawsome-app --load <project> --play-to <out.wav|out.mp3> [--song] [--bars N]
                   [--bits 16|24|32f] [--plugin-path <dir>].. [--cache <file>]
                   [--stream-threshold <frames>] [--record master [--count-in 0|1]]
                   [--midi-events <file.json> [--open-instrument channel:<id>]
                   [--library-preview <set.sfz>]]
                   [--record notes [--count-in 0|1] [--quantize on|off]]
                   [--dump-notes-at <call>] [--cache-dir <dir>] [--save-to <project.json>]
                   [--midi-out-log <file.json>]
  --panels       the panels open at start, comma-separated: rack, playlist, piano-roll,
                 mixer, browser (the Library), settings (the Settings window) (default rack)
  --load         the project to open, as when it is given alone
  --window-size  the main window's size at start, in logical pixels (default 1000x560)
  --screenshot   (a scripted window) save a PNG of the main window, or of the one
                 --screenshot-window names, --screenshot-after <ms> after start; with
                 --screenshot-every <ms>, again that often, each to <stem>-NNN.png beside it
  --quit-after   (a scripted window) close the window this many milliseconds after start
  --midi-ports   list these MIDI input ports, comma-separated, or none, in place of the
                 system's, and connect nothing (a headless check)
  --midi-out-ports  list these MIDI output ports, comma-separated, or none, in place of the
                 system's for the MIDI outs, and open none: what they are sent is kept in
                 memory (a headless check)
  --browser-root the one folder the Library lists, open (instead of the project folder,
                 Music and the folders added)
  --stream-threshold  audio clip files longer than this many frames stream from disk
                 (default 30 seconds of frames)
  --record       with --play-to: record the master into samples/rec-NNNN.wav beside the
                 project while it plays (input has no device offline), after one bar of
                 count-in unless --count-in 0; the recording becomes a clip. `notes` records
                 the notes --midi-events plays into the first pattern instead, quantized to
                 a step unless --quantize off
  --midi-events  with --play-to: play these MIDI messages, each before the 512-frame call
                 the file times it for, as a device's would arrive, from the port it names
                 when it names one (projects without plugins)
  --midi-out-log  with --play-to: write the messages the input channels' MIDI outs send, as
                 JSON (dawsome render's log): each with its output frame, rack channel and
                 bytes, then the note ons dropped and note offs lost; the render ends with
                 Stop. No MIDI port is opened (projects without plugins)
  --dump-notes-at  with --play-to: once this many 512-frame calls have played, print the
                 pattern's notes (the take's, else the one played) as one JSON line,
                 {\"call\":N,\"notes\":[...]}: what a take of notes has written by then
  --null-output  (a scripted window) play through no device: a thread calls the player in
                 real time and drops what it plays
  --rehearse     (a scripted window) Record armed, no count-in, Play from the file's \"from\" tick
                 (song mode, with --song), the file's MIDI messages (--midi-events' format) played
                 by the wall clock: a take filmed with --screenshot-every <ms> (each shot
                 <stem>-NNN.png)
  --cache-dir    keep stretched audio clips here (default: dawsome/stretch in the user's cache
                 folder for the window and --play-to, as dawsome render; in memory only for
                 scripted runs)
  --save-to      with --play-to: write the project, with what the render added, there
  --song         start in song mode: play the playlist instead of the first pattern
  --device-rate  open the audio device at this rate, resampling when it is not the project's
  --play-to      play the project into a WAV file instead of the audio device, with no window
  --bars N       bars to play, counted as played: loop and song-end wraps count (default 1)
  --bits         the WAV sample format of --play-to (default 32f); an .mp3 output is 320 kbps
  --plugin-path  look for CLAP and VST3 plugins here instead of the standard folders
                 (~/.clap and /usr/lib/clap, ~/.vst3 and /usr/lib/vst3 among others;
                 %COMMONPROGRAMFILES%\\CLAP, %COMMONPROGRAMFILES%\\VST3 and their
                 %LOCALAPPDATA%\\Programs\\Common folders on Windows), $CLAP_PATH,
                 $VST3_PATH and the plugins bundled with the program (plugins/ beside it, or
                 lib/dawsome/clap under its install prefix) (repeatable)
  --cache        the plugin catalogue (default: dawsome/clap-catalogue.json in the user's
                 cache folder)
  --open-picker  open the plugin picker at start (a headless check)
  --dump-picker  write the picker's rows, in the order drawn, to a file (a headless check)
  --open-plugin  open a plugin channel's or effect's window at start (a headless check); one
                 without a window of its own opens the parameter window
  --open-params  open a plugin channel's or effect's parameter window at start, as its menu's
                 Parameters… does (a headless check)
  --dump-params  write the parameter window --open-plugin or --open-params opens to a file: its
                 title, then each row's name, a tab and its value's text (a headless check)
  --play         start playing at start (a headless check)
  --list-instruments  print the instrument sets and shipped folders of sounds the Library
                 finds, a line each, and open no window (a headless check of an install)
  --list-help    print the folder the Help window reads its pictures from, then each picture
                 its guide names, found there or missing, and open no window (a headless check
                 of an install)
  --list-plugin-folders  print the folders plugins are looked for in without --plugin-path,
                 a line each, and open no window (a headless check of an install)
  --list-outputs print the output devices Settings offers, a line each (the chosen one
                 marked *, then a tab and what the settings keep for it), and open no window
                 and no device
  --stats        when the window closes, print how long the audio callbacks took, how many
                 frames the window drew and which renderer drew them (wgpu or tiny-skia)
  --open-adder   open the + Instrument picker at start; with --dump-picker, write its
                 rows once the instrument sets are found (a headless check)
  --hints        on: show the hint bar in a scripted run, which hides it otherwise; off: hide
                 it in any run
  --open-instrument  open this channel's instrument window at start (a headless check); with
                 --play-to, the MIDI notes of ports linked to no channel play it, as while its
                 window is open
  --screenshot-window  the window the screenshot is of: the main window (the default), the
                 instrument window --open-instrument opens, the Settings window --panels
                 settings opens, the Help window --show help opens, the parameter window
                 --open-plugin opens (params) or the sample editor --open-sample-editor opens
  --open-sample-editor  open this channel's sample editor at start (a headless check), or the
                 one an audio clip's double-click opens (its channel's, the clip bracketed)
  --sample-editor-select  with --open-sample-editor: select frames <from> to <to> of the sound
                 and show them across the window (opened on a clip, its window shows), once it
                 is loaded
  --library-preview  preview this instrument set as a click on its Library row does (a
                 headless check): the notes no link routes play it; with --play-to, the
                 MIDI notes of ports linked to no channel, as in the window
  --show         show the export dialog (export), the Edit menu (edit-menu) or the Help
                 window on its first section (help), or on section n (help:n), at start (a
                 headless check)
  --settings-page  the Settings window's page at start: general or keys (a headless check)
  --callouts     on: show the first-run notes not yet put away (a scripted run hides them);
                 off: hide them
  --scroll       end: the views that scroll along time start scrolled to their bar's end (a
                 headless check)
  --drag-over    the starter kit's first sound held from the Library over this channel's
                 header, where letting go would replace its sound (a headless check: the
                 run never lets go)
  --channel-menu  open this channel's menu, as a right press on its name does (a headless
                 check)
  --select-clip  select this playlist clip, as a click on it does (a headless check: the Help
                 guide's picture of the link control)
  --curve-menu   open the playlist's curve menu on this automation's first segment handle in
                 view, as a right press there does (a headless check)
  --slot-menu    select this mixer insert and open the menu of its slot n (0 the first), as a
                 press on that slot does (a headless check)
  --library-menu  open the menu of this MIDI device's Library row, as a right press on it does
                 (a headless check; a device a channel plays has one)
  --midi-monitor  connect the system MIDI ports whose names start with this, and no other,
                 and print each message they deliver, where its note goes and what the player
                 plays of it (a check of the real MIDI path; writes no settings)
  --drop-monitor  print where each file dropped on the window lands: the pointer the window
                 system gives, the cursor the window saw, the track and tick (a check of the
                 real drop path; writes no settings)
The user's cache folder is $DAWSOME_CACHE_HOME, else $XDG_CACHE_HOME or ~/.cache on Linux and
%LOCALAPPDATA% on Windows.";

/// The window's flags beside [`Args`]: those added with the hint bar and the "+ Instrument…"
/// picker. Each run takes them out of the command line before [`Args::parse`] reads the rest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extras {
    /// Open the "+ Instrument…" picker at start (a headless check).
    pub open_adder: bool,
    /// Whether the hint bar shows: `--hints on` or `off`; none leaves it to the settings (and
    /// hidden in a scripted run).
    pub hints: Option<bool>,
    /// Open this channel's instrument window at start (a headless check).
    pub open_instrument: Option<u32>,
}

impl Extras {
    /// Takes the flags it reads out of `raw`, leaving the rest in their order.
    pub fn take(raw: &mut Vec<String>) -> Result<Self, ArgsError> {
        let mut extras = Self::default();
        let mut rest = Vec::with_capacity(raw.len());
        let mut words = std::mem::take(raw).into_iter();
        while let Some(word) = words.next() {
            match word.as_str() {
                "--open-adder" if !extras.open_adder => extras.open_adder = true,
                "--open-adder" => return Err(ArgsError::Repeated(word)),
                "--open-instrument" => {
                    let value = words
                        .next()
                        .ok_or_else(|| ArgsError::MissingValue(word.clone()))?;
                    let id = value
                        .strip_prefix("channel:")
                        .and_then(|id| id.parse().ok())
                        .ok_or_else(|| bad_value(&word, "channel:<id>", value.clone()))?;
                    if extras.open_instrument.replace(id).is_some() {
                        return Err(ArgsError::Repeated(word));
                    }
                }
                "--hints" => {
                    let value = words
                        .next()
                        .ok_or_else(|| ArgsError::MissingValue(word.clone()))?;
                    let shown = match value.as_str() {
                        "on" => true,
                        "off" => false,
                        _ => return Err(bad_value(&word, "on or off", value)),
                    };
                    if extras.hints.replace(shown).is_some() {
                        return Err(ArgsError::Repeated(word));
                    }
                }
                _ => rest.push(word),
            }
        }
        *raw = rest;
        Ok(extras)
    }

    /// Whether they make the run a scripted one, as the picker's own flags do.
    pub fn scripted(&self) -> bool {
        self.open_adder || self.open_instrument.is_some()
    }
}

/// What a scripted run shows over the window at start, for its screenshot (`--show`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    /// The export dialog (WAV).
    Export,
    /// The Edit menu.
    EditMenu,
    /// The Help window, on this section (0 the first; `help:<n>` names n from 1).
    Help(usize),
}

impl Shown {
    /// Takes `--show <what>` out of `raw`, leaving the rest in their order. Why apart from
    /// [`Extras`]: a test pins that struct field by field.
    pub fn take(raw: &mut Vec<String>) -> Result<Option<Self>, ArgsError> {
        let Some(at) = raw.iter().position(|word| word == "--show") else {
            return Ok(None);
        };
        let flag = raw.remove(at);
        if at >= raw.len() {
            return Err(ArgsError::MissingValue(flag));
        }
        let value = raw.remove(at);
        let shown = match value.as_str() {
            "export" => Self::Export,
            "edit-menu" => Self::EditMenu,
            "help" => Self::Help(0),
            _ => match value
                .strip_prefix("help:")
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|&n| n >= 1)
            {
                Some(n) => Self::Help(n - 1),
                None => return Err(bad_value(&flag, "export, edit-menu or help[:<n>]", value)),
            },
        };
        if raw.iter().any(|word| word == "--show") {
            return Err(ArgsError::Repeated(flag));
        }
        Ok(Some(shown))
    }
}

/// Takes `--screenshot-window main|instrument|settings|help|params|sample-editor` out of `raw`,
/// leaving the rest in their order.
/// Why apart from [`Args`]: tests pin its screenshot request field by field.
pub fn take_shot_window(raw: &mut Vec<String>) -> Result<Option<ShotWindow>, ArgsError> {
    const FLAG: &str = "--screenshot-window";
    let Some(at) = raw.iter().position(|word| word == FLAG) else {
        return Ok(None);
    };
    let flag = raw.remove(at);
    if at >= raw.len() {
        return Err(ArgsError::MissingValue(flag));
    }
    let value = raw.remove(at);
    let window = match value.as_str() {
        "main" => ShotWindow::Main,
        "instrument" => ShotWindow::Instrument,
        "settings" => ShotWindow::Settings,
        "help" => ShotWindow::Help,
        "params" => ShotWindow::Params,
        "sample-editor" => ShotWindow::SampleEditor,
        _ => {
            return Err(bad_value(
                &flag,
                "main, instrument, settings, help, params or sample-editor",
                value,
            ))
        }
    };
    if raw.iter().any(|word| word == FLAG) {
        return Err(ArgsError::Repeated(flag));
    }
    Ok(Some(window))
}

/// Takes `<flag> <value>` out of `raw` once, leaving the rest in their order: the value as
/// `parse` reads it, or why not (`expected` names what it takes).
fn take_choice<T>(
    raw: &mut Vec<String>,
    flag: &str,
    expected: &'static str,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<Option<T>, ArgsError> {
    let Some(at) = raw.iter().position(|word| word == flag) else {
        return Ok(None);
    };
    let flag = raw.remove(at);
    if at >= raw.len() {
        return Err(ArgsError::MissingValue(flag));
    }
    let value = raw.remove(at);
    let Some(chosen) = parse(&value) else {
        return Err(bad_value(&flag, expected, value));
    };
    if raw.contains(&flag) {
        return Err(ArgsError::Repeated(flag));
    }
    Ok(Some(chosen))
}

/// Takes `--settings-page general|keys` out of `raw`: the Settings window's page a scripted run
/// opens on (with `--panels settings`, which opens the window, for its screenshot).
pub fn take_settings_page(
    raw: &mut Vec<String>,
) -> Result<Option<crate::ui::settings_layout::SettingsPage>, ArgsError> {
    take_choice(
        raw,
        "--settings-page",
        "general or keys",
        crate::ui::settings_layout::SettingsPage::from_name,
    )
}

/// Takes `--callouts on|off` out of `raw`: whether the first-run notes show, which a scripted
/// run leaves hidden otherwise.
pub fn take_callouts(raw: &mut Vec<String>) -> Result<Option<bool>, ArgsError> {
    take_choice(raw, "--callouts", "on or off", |value| match value {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    })
}

/// Takes `--scroll end` out of `raw`: whether the views that scroll along time start at their
/// bar's end (for a screenshot of them scrolled).
pub fn take_scroll(raw: &mut Vec<String>) -> Result<bool, ArgsError> {
    let at_end = take_choice(raw, "--scroll", "end", |value| {
        (value == "end").then_some(())
    })?;
    Ok(at_end.is_some())
}

/// Takes `--drag-over channel:<id>` out of `raw`: the channel whose header a scripted run shows a
/// Library sound held over (for a screenshot of the drop target).
pub fn take_drag_over(raw: &mut Vec<String>) -> Result<Option<u32>, ArgsError> {
    take_choice(raw, "--drag-over", "channel:<id>", |value| {
        value.strip_prefix("channel:")?.parse().ok()
    })
}

/// Takes `--channel-menu channel:<id>` out of `raw`: the channel whose menu a scripted run opens
/// (for a screenshot of a menu over the panel below its panel).
pub fn take_channel_menu(raw: &mut Vec<String>) -> Result<Option<u32>, ArgsError> {
    take_choice(raw, "--channel-menu", "channel:<id>", |value| {
        value.strip_prefix("channel:")?.parse().ok()
    })
}

/// Takes `--select-clip clip:<id>` out of `raw`: the playlist clip a scripted run selects (for
/// the Help guide's picture of the link control, which shows for a selected clip).
pub fn take_select_clip(raw: &mut Vec<String>) -> Result<Option<u32>, ArgsError> {
    take_choice(raw, "--select-clip", "clip:<id>", |value| {
        value.strip_prefix("clip:")?.parse().ok()
    })
}

/// Takes `--curve-menu automation:<id>` out of `raw`: the automation whose segment's curve menu
/// a scripted run opens (for a screenshot of that menu at the tracks' edge).
pub fn take_curve_menu(raw: &mut Vec<String>) -> Result<Option<u32>, ArgsError> {
    take_choice(raw, "--curve-menu", "automation:<id>", |value| {
        value.strip_prefix("automation:")?.parse().ok()
    })
}

/// Takes `--slot-menu insert:<id>,slot:<n>` out of `raw`: the mixer insert a scripted run
/// selects and its slot whose menu it opens (for a screenshot of that menu at the window's
/// foot).
pub fn take_slot_menu(raw: &mut Vec<String>) -> Result<Option<(u16, u8)>, ArgsError> {
    take_choice(raw, "--slot-menu", "insert:<id>,slot:<n>", |value| {
        let (insert, slot) = value.strip_prefix("insert:")?.split_once(",slot:")?;
        Some((insert.parse().ok()?, slot.parse().ok()?))
    })
}

/// Takes `--library-menu device:<port>` out of `raw`: the MIDI device whose Library row's menu
/// a scripted run opens (for a screenshot of that menu at the panel's foot).
pub fn take_library_menu(raw: &mut Vec<String>) -> Result<Option<String>, ArgsError> {
    take_choice(raw, "--library-menu", "device:<port>", |value| {
        let port = value.strip_prefix("device:")?;
        (!port.is_empty()).then(|| port.to_owned())
    })
}

/// Takes `--open-sample-editor channel:<id>|clip:<id>` and `--sample-editor-select
/// <from>-<to>` out of `raw`: the channel whose sound a scripted run opens the sample editor on
/// (or the audio clip whose double-click it opens as), and the frames it selects there and
/// shows across the window (for the editor's screenshots). A selection without the editor is
/// refused.
pub fn take_sample_editor(raw: &mut Vec<String>) -> Result<Option<EditorAtStart>, ArgsError> {
    let on = take_choice(
        raw,
        "--open-sample-editor",
        "channel:<id>|clip:<id>",
        |value| match value.split_once(':')? {
            ("channel", id) => Some(EditorOn::Channel(id.parse().ok()?)),
            ("clip", id) => Some(EditorOn::Clip(id.parse().ok()?)),
            _ => None,
        },
    )?;
    let select = take_choice(raw, "--sample-editor-select", "<from>-<to>", |value| {
        let (from, to) = value.split_once('-')?;
        Some((from.parse().ok()?, to.parse().ok()?))
    })?;
    match (on, select) {
        (None, Some(_)) => Err(ArgsError::MissingValue(
            "--open-sample-editor (--sample-editor-select selects in it)".to_owned(),
        )),
        (on, select) => Ok(on.map(|on| EditorAtStart { on, select })),
    }
}

/// The sample editor a scripted run opens at start: on a channel or as a clip's double-click
/// opens it, frames `select` selected and shown across it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorAtStart {
    pub on: EditorOn,
    pub select: Option<(u64, u64)>,
}

/// What `--open-sample-editor` opens the editor on, by id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorOn {
    Channel(u32),
    Clip(u32),
}

/// Takes `--drop-monitor` out of `raw`: whether the run prints where each file dropped on its
/// window lands (a check of the real drop path; such a run writes no settings).
pub fn take_drop_monitor(raw: &mut Vec<String>) -> bool {
    let before = raw.len();
    raw.retain(|word| word != "--drop-monitor");
    raw.len() != before
}

/// Takes `--midi-monitor <port>` out of `raw`: the start of the name of the system MIDI port a
/// run connects to (and no other), printing what it delivers and what the player plays of it.
pub fn take_midi_monitor(raw: &mut Vec<String>) -> Result<Option<String>, ArgsError> {
    take_choice(
        raw,
        "--midi-monitor",
        "the start of a MIDI port's name",
        |value| (!value.trim().is_empty()).then(|| value.to_owned()),
    )
}

/// Takes `--library-preview <set.sfz>` out of `raw`, leaving the rest in their order. Why apart
/// from [`Extras`]: a test pins that struct field by field.
pub fn take_library_preview(raw: &mut Vec<String>) -> Result<Option<PathBuf>, ArgsError> {
    const FLAG: &str = "--library-preview";
    let Some(at) = raw.iter().position(|word| word == FLAG) else {
        return Ok(None);
    };
    let flag = raw.remove(at);
    if at >= raw.len() {
        return Err(ArgsError::MissingValue(flag));
    }
    let value = PathBuf::from(raw.remove(at));
    if value.extension().is_none_or(|extension| extension != "sfz") {
        return Err(bad_value(
            &flag,
            "an instrument set's .sfz file",
            value.display().to_string(),
        ));
    }
    if raw.iter().any(|word| word == FLAG) {
        return Err(ArgsError::Repeated(flag));
    }
    Ok(Some(value))
}

/// Whether the command line `raw` gives the window's layout itself (`--panels` or
/// `--window-size`), so no saved layout and no first start's apply. Why a scan of the words
/// rather than a field of [`Args`]: the flags keep the defaults they had, which the parsed
/// arguments cannot tell from values given.
pub fn layout_given(raw: &[String]) -> bool {
    raw.iter()
        .any(|word| word == "--panels" || word == "--window-size")
}

/// Everything the command line can ask for.
#[derive(Debug, Clone, PartialEq)]
pub struct Args {
    /// Project to open; an empty document when absent.
    pub project: Option<PathBuf>,
    /// Initial window size in logical pixels.
    pub window_size: Size,
    /// The panels open at start.
    pub panels: Panels,
    /// The rate to open the audio device at, instead of the project's.
    pub device_rate: Option<u32>,
    /// Audio clip files longer than this many frames stream; `None` is 30 s of frames.
    pub stream_threshold: Option<u64>,
    /// The one folder the browser lists, open.
    pub browser_root: Option<PathBuf>,
    /// Where stretched clips are kept, instead of the user's cache.
    pub cache_dir: Option<PathBuf>,
    /// MIDI input port names to list in place of the system's ports, which then are never
    /// opened.
    pub midi_ports: Option<Vec<String>>,
    /// MIDI output port names to list in place of the system's ports, which then are never
    /// opened.
    pub midi_out_ports: Option<Vec<String>>,
    /// Start in song mode.
    pub song: bool,
    /// Print the audio callbacks' timings and the window's updates and frames when it closes.
    pub stats: bool,
    pub script: Script,
    /// Render to a WAV file instead of opening the window.
    pub play_to: Option<PlayTo>,
    pub plugins: PluginFlags,
}

/// Where plugins are looked for and the catalogue is kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginFlags {
    /// `--plugin-path`, each given; none means CLAP's standard places.
    pub dirs: Vec<PathBuf>,
    /// `--cache`; none means the user's cache dir.
    pub cache: Option<PathBuf>,
}

/// A command line that cannot be run.
#[derive(Debug, PartialEq, Error)]
pub enum ArgsError {
    #[error("unknown argument {0:?}")]
    Unknown(String),
    #[error("{0} needs a value")]
    MissingValue(String),
    #[error("{0} was given twice")]
    Repeated(String),
    #[error("{flag} expects {expected}, got {value:?}")]
    BadValue {
        flag: String,
        expected: &'static str,
        value: String,
    },
    #[error("--screenshot and --screenshot-after go together (and --screenshot-every with them)")]
    IncompleteScreenshot,
    #[error("--quit-after ({quit} ms) must be later than --screenshot-after ({shot} ms)")]
    QuitBeforeScreenshot { quit: u128, shot: u128 },
    #[error("--bars goes with --play-to")]
    BarsWithoutPlayTo,
    #[error("--bits goes with --play-to to a WAV file")]
    BitsWithoutWavPlayTo,
    #[error("--play-to needs --load")]
    PlayToWithoutProject,
    #[error("--play-to opens no window, so {0} does not apply")]
    PlayToWithWindowFlag(&'static str),
    #[error("--play-to plays no audio device, so --device-rate does not apply")]
    PlayToWithDeviceRate,
    #[error("{0} goes with --play-to")]
    WithoutPlayTo(&'static str),
    #[error("--count-in goes with --record")]
    CountInWithoutRecord,
    #[error("--record input needs an input device, and --play-to has none: record the master")]
    RecordInputOffline,
    #[error("--quantize goes with --record notes")]
    QuantizeWithoutNotes,
    #[error("--record notes records into the pattern as it loops, so --song does not apply")]
    NotesInSong,
}

/// What `--record` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordFlag {
    Audio(RecordSource),
    Notes,
}

impl Args {
    /// Parses the flags after the program name.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, ArgsError> {
        let mut args = args.into_iter();
        let mut project = None;
        let mut window_size = None;
        let mut screenshot_path = None;
        let mut screenshot_after = None;
        let mut quit_after = None;
        let mut play_to = None;
        let mut bars = None;
        let mut panels = None;
        let mut device_rate = None;
        let mut song = None;
        let mut bits = None;
        let mut plugins = PluginFlags::default();
        let mut open_picker = None;
        let mut dump_picker = None;
        let mut open_plugin = None;
        let mut open_params = None;
        let mut dump_params = None;
        let mut stream_threshold = None;
        let mut browser_root = None;
        let mut record = None;
        let mut count_in = None;
        let mut save_to = None;
        let mut quantize = None;
        let mut midi_events = None;
        let mut dump_notes_at = None;
        let mut cache_dir = None;
        let mut midi_ports = None;
        let mut midi_out_ports = None;
        let mut midi_out_log = None;
        let mut play = None;
        let mut stats = None;
        let mut null_output = None;
        let mut rehearse = None;
        let mut screenshot_every = None;
        while let Some(flag) = args.next() {
            let switch = match flag.as_str() {
                "--song" => Some(&mut song),
                "--play" => Some(&mut play),
                "--stats" => Some(&mut stats),
                "--null-output" => Some(&mut null_output),
                _ => None,
            };
            if let Some(switch) = switch {
                set(switch, &flag, true)?;
                continue;
            }
            // A bare path is the project to open, as a desktop entry passes a file.
            if !flag.starts_with('-') {
                set(&mut project, "--load", PathBuf::from(flag))?;
                continue;
            }
            let value = args
                .next()
                .ok_or_else(|| ArgsError::MissingValue(flag.clone()));
            match flag.as_str() {
                "--load" => set(&mut project, &flag, PathBuf::from(value?))?,
                "--window-size" => set(&mut window_size, &flag, parse_size(&flag, value?)?)?,
                "--screenshot" => set(&mut screenshot_path, &flag, PathBuf::from(value?))?,
                "--screenshot-after" => {
                    set(&mut screenshot_after, &flag, parse_millis(&flag, value?)?)?;
                }
                "--quit-after" => set(&mut quit_after, &flag, parse_millis(&flag, value?)?)?,
                "--screenshot-every" => {
                    set(&mut screenshot_every, &flag, parse_millis(&flag, value?)?)?;
                }
                "--rehearse" => set(&mut rehearse, &flag, PathBuf::from(value?))?,
                "--play-to" => set(&mut play_to, &flag, PathBuf::from(value?))?,
                "--bars" => set(&mut bars, &flag, parse_bars(&flag, value?)?)?,
                "--panels" => set(&mut panels, &flag, parse_panels(&flag, value?)?)?,
                "--device-rate" => set(&mut device_rate, &flag, parse_rate(&flag, value?)?)?,
                "--bits" => set(&mut bits, &flag, parse_bits(&flag, value?)?)?,
                "--plugin-path" => plugins.dirs.push(PathBuf::from(value?)),
                "--cache" => set(&mut plugins.cache, &flag, PathBuf::from(value?))?,
                "--open-picker" => set(&mut open_picker, &flag, parse_kind(&flag, value?)?)?,
                "--dump-picker" => set(&mut dump_picker, &flag, PathBuf::from(value?))?,
                "--open-plugin" => set(&mut open_plugin, &flag, parse_owner(&flag, value?)?)?,
                "--open-params" => set(&mut open_params, &flag, parse_owner(&flag, value?)?)?,
                "--dump-params" => set(&mut dump_params, &flag, PathBuf::from(value?))?,
                "--stream-threshold" => {
                    set(&mut stream_threshold, &flag, parse_frames(&flag, value?)?)?;
                }
                "--browser-root" => set(&mut browser_root, &flag, PathBuf::from(value?))?,
                "--record" => set(&mut record, &flag, parse_record(&flag, value?)?)?,
                "--count-in" => set(&mut count_in, &flag, parse_count_in(&flag, value?)?)?,
                "--save-to" => set(&mut save_to, &flag, PathBuf::from(value?))?,
                "--quantize" => set(&mut quantize, &flag, parse_switch(&flag, value?)?)?,
                "--midi-events" => set(&mut midi_events, &flag, PathBuf::from(value?))?,
                "--dump-notes-at" => set(&mut dump_notes_at, &flag, parse_call(&flag, value?)?)?,
                "--cache-dir" => set(&mut cache_dir, &flag, PathBuf::from(value?))?,
                "--midi-ports" => set(&mut midi_ports, &flag, parse_ports(&flag, value?)?)?,
                "--midi-out-ports" => {
                    set(&mut midi_out_ports, &flag, parse_ports(&flag, value?)?)?;
                }
                "--midi-out-log" => set(&mut midi_out_log, &flag, PathBuf::from(value?))?,
                _ => return Err(ArgsError::Unknown(flag)),
            }
        }
        let screenshot = match (screenshot_path, screenshot_after) {
            (Some(path), Some(after)) => Some(ScreenshotRequest {
                path,
                after,
                every: screenshot_every,
            }),
            (None, None) => None,
            _ => return Err(ArgsError::IncompleteScreenshot),
        };
        if screenshot.is_none() && screenshot_every.is_some() {
            return Err(ArgsError::IncompleteScreenshot);
        }
        if let (Some(shot), Some(quit)) = (&screenshot, quit_after) {
            if quit <= shot.after {
                return Err(ArgsError::QuitBeforeScreenshot {
                    quit: quit.as_millis(),
                    shot: shot.after.as_millis(),
                });
            }
        }
        if play_to.is_none() {
            let offline_only = [
                (record.is_some(), "--record"),
                (count_in.is_some(), "--count-in"),
                (save_to.is_some(), "--save-to"),
                (quantize.is_some(), "--quantize"),
                (midi_events.is_some(), "--midi-events"),
                (dump_notes_at.is_some(), "--dump-notes-at"),
                (midi_out_log.is_some(), "--midi-out-log"),
            ];
            if let Some((_, flag)) = offline_only.into_iter().find(|(given, _)| *given) {
                return Err(ArgsError::WithoutPlayTo(flag));
            }
        }
        let play_to = match (play_to, bars) {
            (Some(out), bars) => {
                let window_flag = [
                    (window_size.is_some(), "--window-size"),
                    (panels.is_some(), "--panels"),
                    (screenshot.is_some(), "--screenshot"),
                    (quit_after.is_some(), "--quit-after"),
                    (open_picker.is_some(), "--open-picker"),
                    (dump_picker.is_some(), "--dump-picker"),
                    (open_plugin.is_some(), "--open-plugin"),
                    (open_params.is_some(), "--open-params"),
                    (dump_params.is_some(), "--dump-params"),
                    (browser_root.is_some(), "--browser-root"),
                    (midi_ports.is_some(), "--midi-ports"),
                    (midi_out_ports.is_some(), "--midi-out-ports"),
                    (play.is_some(), "--play"),
                    (stats.is_some(), "--stats"),
                    (null_output.is_some(), "--null-output"),
                    (rehearse.is_some(), "--rehearse"),
                ]
                .into_iter()
                .find_map(|(given, flag)| given.then_some(flag));
                if let Some(flag) = window_flag {
                    return Err(ArgsError::PlayToWithWindowFlag(flag));
                }
                if device_rate.is_some() {
                    return Err(ArgsError::PlayToWithDeviceRate);
                }
                if project.is_none() {
                    return Err(ArgsError::PlayToWithoutProject);
                }
                let bars = bars.map_or(1, NonZeroU32::get);
                let song = song.unwrap_or(false);
                let mp3 = out
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("mp3"));
                if mp3 && bits.is_some() {
                    return Err(ArgsError::BitsWithoutWavPlayTo);
                }
                if record == Some(RecordFlag::Audio(RecordSource::Input)) {
                    return Err(ArgsError::RecordInputOffline);
                }
                if count_in.is_some() && record.is_none() {
                    return Err(ArgsError::CountInWithoutRecord);
                }
                let record_notes = record == Some(RecordFlag::Notes);
                if quantize.is_some() && !record_notes {
                    return Err(ArgsError::QuantizeWithoutNotes);
                }
                if record_notes && song {
                    return Err(ArgsError::NotesInSong);
                }
                Some(PlayTo {
                    out,
                    bars,
                    song,
                    bits,
                    record: match record {
                        Some(RecordFlag::Audio(source)) => Some(source),
                        Some(RecordFlag::Notes) | None => None,
                    },
                    record_notes,
                    quantize: quantize.unwrap_or(true),
                    count_in: count_in.unwrap_or(true),
                    save_to,
                    stream_threshold,
                    midi_events,
                    cache_dir: cache_dir.clone(),
                    dump_notes_at,
                    midi_out_log,
                })
            }
            (None, Some(_)) => return Err(ArgsError::BarsWithoutPlayTo),
            (None, None) => None,
        };
        if play_to.is_none() && bits.is_some() {
            return Err(ArgsError::BitsWithoutWavPlayTo);
        }
        Ok(Self {
            project,
            window_size: window_size.unwrap_or(DEFAULT_WINDOW_SIZE),
            panels: panels.unwrap_or_default(),
            device_rate,
            stream_threshold,
            browser_root,
            cache_dir,
            midi_ports,
            midi_out_ports,
            song: song.unwrap_or(false),
            stats: stats.unwrap_or(false),
            script: Script {
                screenshot,
                quit_after,
                open_picker,
                dump_picker,
                open_plugin,
                open_params,
                dump_params,
                play: play.unwrap_or(false),
                rehearse,
                null_output: null_output.unwrap_or(false),
            },
            play_to,
            plugins,
        })
    }
}

/// `channel:<id>` or `effect:<id>`.
fn parse_owner(flag: &str, value: String) -> Result<ProcessorOwner, ArgsError> {
    let owner = value.split_once(':').and_then(|(kind, id)| {
        let id = id.parse().ok()?;
        match kind {
            "channel" => Some(ProcessorOwner::Channel(ChannelId(id))),
            "effect" => Some(ProcessorOwner::Effect(EffectId(id))),
            _ => None,
        }
    });
    owner.ok_or_else(|| bad_value(flag, "channel:<id> or effect:<id>", value))
}

/// `instrument` or `effect`.
fn parse_kind(flag: &str, value: String) -> Result<PluginKind, ArgsError> {
    match value.as_str() {
        "instrument" => Ok(PluginKind::Instrument),
        "effect" => Ok(PluginKind::Effect),
        _ => Err(bad_value(flag, "instrument or effect", value)),
    }
}

/// Fills an empty slot, refusing a flag that was already given.
fn set<T>(slot: &mut Option<T>, flag: &str, value: T) -> Result<(), ArgsError> {
    match slot.replace(value) {
        Some(_) => Err(ArgsError::Repeated(flag.to_owned())),
        None => Ok(()),
    }
}

/// `"1000x560"` → 1000 by 560 logical pixels.
fn parse_size(flag: &str, value: String) -> Result<Size, ArgsError> {
    let dimension = |text: &str| text.parse::<u16>().ok().filter(|&pixels| pixels > 0);
    let size = value
        .split_once('x')
        .and_then(|(width, height)| Some((dimension(width)?, dimension(height)?)));
    match size {
        Some((width, height)) => Ok(Size::new(f32::from(width), f32::from(height))),
        None => Err(bad_value(
            flag,
            "<width>x<height> in pixels, e.g. 1000x560",
            value,
        )),
    }
}

/// A delay in whole milliseconds. Zero is refused: a timer needs a period.
fn parse_millis(flag: &str, value: String) -> Result<Duration, ArgsError> {
    match value.parse::<u64>() {
        Ok(millis) if millis > 0 => Ok(Duration::from_millis(millis)),
        _ => Err(bad_value(
            flag,
            "a whole number of milliseconds above 0",
            value,
        )),
    }
}

/// A count of frames, 0 included.
fn parse_frames(flag: &str, value: String) -> Result<u64, ArgsError> {
    value
        .parse()
        .map_err(|_| bad_value(flag, "a whole number of frames", value))
}

/// `master`, `input` or `notes`.
fn parse_record(flag: &str, value: String) -> Result<RecordFlag, ArgsError> {
    match value.as_str() {
        "master" => Ok(RecordFlag::Audio(RecordSource::Master)),
        "input" => Ok(RecordFlag::Audio(RecordSource::Input)),
        "notes" => Ok(RecordFlag::Notes),
        _ => Err(bad_value(flag, "master, input or notes", value)),
    }
}

/// `on` or `off`.
fn parse_switch(flag: &str, value: String) -> Result<bool, ArgsError> {
    match value.as_str() {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(bad_value(flag, "on or off", value)),
    }
}

/// `0` or `1`.
fn parse_count_in(flag: &str, value: String) -> Result<bool, ArgsError> {
    match value.as_str() {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(bad_value(flag, "0 or 1", value)),
    }
}

/// A call of the player: a whole number, 0 included.
fn parse_call(flag: &str, value: String) -> Result<u64, ArgsError> {
    value
        .parse()
        .map_err(|_| bad_value(flag, "a whole number of calls", value))
}

/// `none` → no port; `"Port A,Port B"` → those two, in that order. A name is not empty.
fn parse_ports(flag: &str, value: String) -> Result<Vec<String>, ArgsError> {
    if value == "none" {
        return Ok(Vec::new());
    }
    let names: Vec<String> = value.split(',').map(str::to_owned).collect();
    if names.iter().any(|name| name.trim().is_empty()) {
        return Err(bad_value(
            flag,
            "none, or port names separated by commas",
            value,
        ));
    }
    Ok(names)
}

/// `"rack,piano-roll"` → both panels open. Each name at most once; at least one.
fn parse_panels(flag: &str, value: String) -> Result<Panels, ArgsError> {
    let expected = "a comma-separated list of rack, playlist, piano-roll, mixer, browser and settings, each at most once";
    let mut panels = Panels::NONE;
    for name in value.split(',') {
        match Panel::from_name(name) {
            Some(panel) if !panels.is_open(panel) => panels.set(panel, true),
            _ => return Err(bad_value(flag, expected, value)),
        }
    }
    Ok(panels)
}

/// A sample rate in whole Hz, one the engine accepts (8000 to 384000).
fn parse_rate(flag: &str, value: String) -> Result<u32, ArgsError> {
    match value.parse::<u32>() {
        Ok(rate) if SAMPLE_RATE_RANGE.contains(&rate) => Ok(rate),
        _ => Err(bad_value(
            flag,
            "a sample rate from 8000 to 384000 Hz",
            value,
        )),
    }
}

/// `16`, `24` or `32f`.
fn parse_bits(flag: &str, value: String) -> Result<BitDepth, ArgsError> {
    BitDepth::parse(&value).ok_or_else(|| bad_value(flag, "16, 24 or 32f", value))
}

/// A whole number of bars above 0.
fn parse_bars(flag: &str, value: String) -> Result<NonZeroU32, ArgsError> {
    value
        .parse()
        .map_err(|_| bad_value(flag, "a whole number of bars above 0", value))
}

fn bad_value(flag: &str, expected: &'static str, value: String) -> ArgsError {
    ArgsError::BadValue {
        flag: flag.to_owned(),
        expected,
        value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Result<Args, ArgsError> {
        Args::parse(line.split_whitespace().map(String::from))
    }

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
    }

    // The window's extra flags come out of the line, the rest stays for Args in its order; a
    // flag given twice or a value other than on/off is refused.
    #[test]
    fn the_extra_flags_come_out_and_leave_the_rest() {
        let mut line = words(
            "--load a.json --open-adder --hints on --open-instrument channel:3 --panels rack",
        );
        let extras = Extras::take(&mut line).expect("valid");
        assert_eq!(
            extras,
            Extras {
                open_adder: true,
                hints: Some(true),
                open_instrument: Some(3),
            }
        );
        assert!(Extras::take(&mut words("--open-instrument effect:1")).is_err());
        assert!(extras.scripted());
        assert_eq!(line, words("--load a.json --panels rack"));
        assert_eq!(
            Extras::take(&mut words("")).expect("valid"),
            Extras::default()
        );
        assert_eq!(
            Extras::take(&mut words("--hints maybe")),
            Err(bad_value("--hints", "on or off", "maybe".to_owned()))
        );
        assert_eq!(
            Extras::take(&mut words("--hints")),
            Err(ArgsError::MissingValue("--hints".to_owned()))
        );
        assert_eq!(
            Extras::take(&mut words("--open-adder --open-adder")),
            Err(ArgsError::Repeated("--open-adder".to_owned()))
        );
    }

    // `--show export`, `--show edit-menu` and `--show help` come out of the line; a missing,
    // unknown or repeated value is refused.
    #[test]
    fn show_comes_out_of_the_line() {
        let mut line = words("--load a.json --show export --panels rack");
        assert_eq!(Shown::take(&mut line), Ok(Some(Shown::Export)));
        assert_eq!(line, words("--load a.json --panels rack"));
        assert_eq!(
            Shown::take(&mut words("--show edit-menu")),
            Ok(Some(Shown::EditMenu))
        );
        assert_eq!(
            Shown::take(&mut words("--show help")),
            Ok(Some(Shown::Help(0)))
        );
        assert_eq!(
            Shown::take(&mut words("--show help:5")),
            Ok(Some(Shown::Help(4)))
        );
        assert_eq!(
            Shown::take(&mut words("--show help:0")),
            Err(bad_value(
                "--show",
                "export, edit-menu or help[:<n>]",
                "help:0".to_owned()
            ))
        );
        assert_eq!(Shown::take(&mut words("--load a.json")), Ok(None));
        assert_eq!(
            Shown::take(&mut words("--show")),
            Err(ArgsError::MissingValue("--show".to_owned()))
        );
        assert_eq!(
            Shown::take(&mut words("--show menu")),
            Err(bad_value(
                "--show",
                "export, edit-menu or help[:<n>]",
                "menu".to_owned()
            ))
        );
        assert_eq!(
            Shown::take(&mut words("--show export --show export")),
            Err(ArgsError::Repeated("--show".to_owned()))
        );
    }

    // P2: `--scroll end` comes out of the line; another value, a missing one and a repeat are
    // refused.
    #[test]
    fn scroll_end_comes_out_of_the_line() {
        let mut line = words("--load a.json --scroll end --panels piano-roll");
        assert_eq!(take_scroll(&mut line), Ok(true));
        assert_eq!(line, words("--load a.json --panels piano-roll"));
        assert_eq!(take_scroll(&mut words("--load a.json")), Ok(false));
        assert_eq!(
            take_scroll(&mut words("--scroll start")),
            Err(bad_value("--scroll", "end", "start".to_owned()))
        );
        assert_eq!(
            take_scroll(&mut words("--scroll")),
            Err(ArgsError::MissingValue("--scroll".to_owned()))
        );
        assert_eq!(
            take_scroll(&mut words("--scroll end --scroll end")),
            Err(ArgsError::Repeated("--scroll".to_owned()))
        );
    }

    // P4: `--drag-over channel:<id>` comes out of the line; a value that names no channel id is
    // refused.
    #[test]
    fn drag_over_comes_out_of_the_line() {
        let mut line = words("--load a.json --drag-over channel:2 --panels rack");
        assert_eq!(take_drag_over(&mut line), Ok(Some(2)));
        assert_eq!(line, words("--load a.json --panels rack"));
        assert_eq!(take_drag_over(&mut words("--panels rack")), Ok(None));
        assert_eq!(
            take_drag_over(&mut words("--drag-over 2")),
            Err(bad_value("--drag-over", "channel:<id>", "2".to_owned()))
        );
        let mut line = words("--channel-menu channel:5 --panels rack");
        assert_eq!(take_channel_menu(&mut line), Ok(Some(5)));
        assert_eq!(line, words("--panels rack"));
        assert_eq!(
            take_channel_menu(&mut words("--channel-menu channel:x")),
            Err(bad_value(
                "--channel-menu",
                "channel:<id>",
                "channel:x".to_owned()
            ))
        );
        // Help guide: `--select-clip clip:<id>` likewise.
        let mut line = words("--select-clip clip:3 --panels playlist");
        assert_eq!(take_select_clip(&mut line), Ok(Some(3)));
        assert_eq!(line, words("--panels playlist"));
        assert_eq!(
            take_select_clip(&mut words("--select-clip 3")),
            Err(bad_value("--select-clip", "clip:<id>", "3".to_owned()))
        );
    }

    // `--curve-menu automation:<id>` comes out of the line, the rest kept in
    // order; a value that names no automation is refused.
    #[test]
    fn curve_menu_comes_out_of_the_line() {
        let mut line = words("--load a.json --curve-menu automation:1 --panels playlist");
        assert_eq!(take_curve_menu(&mut line), Ok(Some(1)));
        assert_eq!(line, words("--load a.json --panels playlist"));
        assert_eq!(take_curve_menu(&mut words("--panels playlist")), Ok(None));
        assert_eq!(
            take_curve_menu(&mut words("--curve-menu channel:1")),
            Err(bad_value(
                "--curve-menu",
                "automation:<id>",
                "channel:1".to_owned()
            ))
        );
    }

    // `--slot-menu insert:<id>,slot:<n>` comes out of the line, the rest kept in order; a value
    // missing its insert or its slot, or with a slot past 255, is refused.
    #[test]
    fn slot_menu_comes_out_of_the_line() {
        let mut line = words("--load a.json --slot-menu insert:12,slot:9 --panels mixer");
        assert_eq!(take_slot_menu(&mut line), Ok(Some((12, 9))));
        assert_eq!(line, words("--load a.json --panels mixer"));
        assert_eq!(take_slot_menu(&mut words("--panels mixer")), Ok(None));
        for value in [
            "insert:12",
            "slot:9",
            "insert:12,slot:256",
            "insert:x,slot:1",
        ] {
            assert_eq!(
                take_slot_menu(&mut words(&format!("--slot-menu {value}"))),
                Err(bad_value(
                    "--slot-menu",
                    "insert:<id>,slot:<n>",
                    value.to_owned()
                ))
            );
        }
    }

    // `--library-menu device:<port>` comes out of the line whole, a port name with a space in
    // it too, the rest kept in order; a value that names no device is refused.
    #[test]
    fn library_menu_comes_out_of_the_line() {
        let mut line = vec![
            "--panels".to_owned(),
            "browser".to_owned(),
            "--library-menu".to_owned(),
            "device:Port A".to_owned(),
        ];
        assert_eq!(take_library_menu(&mut line), Ok(Some("Port A".to_owned())));
        assert_eq!(line, words("--panels browser"));
        assert_eq!(take_library_menu(&mut words("--panels browser")), Ok(None));
        for value in ["device:", "channel:1"] {
            assert_eq!(
                take_library_menu(&mut words(&format!("--library-menu {value}"))),
                Err(bad_value(
                    "--library-menu",
                    "device:<port>",
                    value.to_owned()
                ))
            );
        }
    }

    // `--library-preview <set.sfz>` comes out of the line; a missing value, a file that is no
    // set's .sfz and a repeat are refused.
    #[test]
    fn the_library_preview_comes_out_of_the_line() {
        let mut line = words("--load a.json --library-preview sets/pad/pad.sfz --play-to b.wav");
        assert_eq!(
            take_library_preview(&mut line),
            Ok(Some(PathBuf::from("sets/pad/pad.sfz")))
        );
        assert_eq!(line, words("--load a.json --play-to b.wav"));
        assert_eq!(take_library_preview(&mut words("--load a.json")), Ok(None));
        assert_eq!(
            take_library_preview(&mut words("--library-preview")),
            Err(ArgsError::MissingValue("--library-preview".to_owned()))
        );
        assert_eq!(
            take_library_preview(&mut words("--library-preview kick.wav")),
            Err(bad_value(
                "--library-preview",
                "an instrument set's .sfz file",
                "kick.wav".to_owned()
            ))
        );
        assert_eq!(
            take_library_preview(&mut words(
                "--library-preview a.sfz --library-preview b.sfz"
            )),
            Err(ArgsError::Repeated("--library-preview".to_owned()))
        );
    }

    // `--screenshot-window main|instrument|settings|help|params` comes out of the line; a missing,
    // unknown or repeated value is refused; without it the screenshot is of the main window.
    #[test]
    fn the_screenshot_window_comes_out_of_the_line() {
        let mut line = words("--load a.json --screenshot-window instrument --screenshot b.png");
        assert_eq!(
            take_shot_window(&mut line),
            Ok(Some(ShotWindow::Instrument))
        );
        assert_eq!(line, words("--load a.json --screenshot b.png"));
        assert_eq!(
            take_shot_window(&mut words("--screenshot-window main")),
            Ok(Some(ShotWindow::Main))
        );
        assert_eq!(take_shot_window(&mut words("--load a.json")), Ok(None));
        assert_eq!(
            take_shot_window(&mut words("--screenshot-window settings")),
            Ok(Some(ShotWindow::Settings))
        );
        assert_eq!(
            take_shot_window(&mut words("--screenshot-window help")),
            Ok(Some(ShotWindow::Help))
        );
        assert_eq!(
            take_shot_window(&mut words("--screenshot-window params")),
            Ok(Some(ShotWindow::Params))
        );
        assert_eq!(
            take_shot_window(&mut words("--screenshot-window")),
            Err(ArgsError::MissingValue("--screenshot-window".to_owned()))
        );
        assert_eq!(
            take_shot_window(&mut words("--screenshot-window mixer")),
            Err(bad_value(
                "--screenshot-window",
                "main, instrument, settings, help, params or sample-editor",
                "mixer".to_owned()
            ))
        );
        // `--open-sample-editor channel:<id>` or `clip:<id>` with its selection, `<from>-<to>`;
        // another target, a selection alone, or one not two numbers, is refused.
        assert_eq!(
            take_sample_editor(&mut words(
                "--open-sample-editor channel:2 --sample-editor-select 1000-2000"
            )),
            Ok(Some(EditorAtStart {
                on: EditorOn::Channel(2),
                select: Some((1000, 2000))
            }))
        );
        assert_eq!(
            take_sample_editor(&mut words("--open-sample-editor channel:1")),
            Ok(Some(EditorAtStart {
                on: EditorOn::Channel(1),
                select: None
            }))
        );
        assert_eq!(
            take_sample_editor(&mut words("--open-sample-editor clip:3")),
            Ok(Some(EditorAtStart {
                on: EditorOn::Clip(3),
                select: None
            }))
        );
        assert!(take_sample_editor(&mut words("--open-sample-editor track:1")).is_err());
        assert!(take_sample_editor(&mut words("--sample-editor-select 1-2")).is_err());
        assert!(take_sample_editor(&mut words(
            "--open-sample-editor channel:1 --sample-editor-select 1"
        ))
        .is_err());
        assert_eq!(take_sample_editor(&mut words("--load a.json")), Ok(None));
        assert_eq!(
            take_shot_window(&mut words(
                "--screenshot-window main --screenshot-window instrument"
            )),
            Err(ArgsError::Repeated("--screenshot-window".to_owned()))
        );
        assert_eq!(ShotWindow::default(), ShotWindow::Main);
        assert_eq!(
            take_shot_window(&mut words("--screenshot-window sample-editor")),
            Ok(Some(ShotWindow::SampleEditor))
        );
    }

    #[test]
    fn the_headless_screenshot_command_parses() {
        let args = parse(
            "--load tests/fixtures/f1_four_on_floor.json --screenshot /tmp/f1.png \
             --screenshot-after 800 --quit-after 1500",
        )
        .expect("valid");
        assert_eq!(
            args,
            Args {
                project: Some(PathBuf::from("tests/fixtures/f1_four_on_floor.json")),
                window_size: Size::new(1000.0, 560.0),
                panels: Panels::default(),
                device_rate: None,
                stream_threshold: None,
                browser_root: None,
                cache_dir: None,
                midi_ports: None,
                midi_out_ports: None,
                song: false,
                stats: false,
                script: Script {
                    screenshot: Some(ScreenshotRequest {
                        path: PathBuf::from("/tmp/f1.png"),
                        after: Duration::from_millis(800),
                        every: None,
                    }),
                    quit_after: Some(Duration::from_millis(1500)),
                    open_picker: None,
                    dump_picker: None,
                    open_plugin: None,
                    open_params: None,
                    dump_params: None,
                    play: false,
                    rehearse: None,
                    null_output: false,
                },
                play_to: None,
                plugins: PluginFlags::default(),
            }
        );
    }

    // The picker screenshot command, and plugin places for --play-to.
    #[test]
    fn the_picker_flags_parse_and_plugin_places_go_with_play_to() {
        let args = parse(
            "--cache tests/fixtures/p7_catalogue.json --open-picker instrument              --screenshot /tmp/pick.png --screenshot-after 1500 --quit-after 2500              --dump-picker /tmp/pick.txt",
        )
        .expect("valid");
        assert_eq!(args.script.open_picker, Some(PluginKind::Instrument));
        assert_eq!(
            args.script.dump_picker,
            Some(PathBuf::from("/tmp/pick.txt"))
        );
        assert_eq!(
            args.plugins.cache,
            Some(PathBuf::from("tests/fixtures/p7_catalogue.json"))
        );
        let play = parse("--load a.json --play-to a.wav --plugin-path /a --plugin-path /b")
            .expect("valid");
        assert_eq!(
            play.plugins.dirs,
            [PathBuf::from("/a"), PathBuf::from("/b")]
        );
        assert_eq!(
            parse("--load a.json --play-to a.wav --open-picker effect"),
            Err(ArgsError::PlayToWithWindowFlag("--open-picker"))
        );
        assert!(matches!(
            parse("--open-picker synth"),
            Err(ArgsError::BadValue { .. })
        ));
        assert_eq!(
            parse("--open-plugin effect:3")
                .expect("valid")
                .script
                .open_plugin,
            Some(ProcessorOwner::Effect(EffectId(3)))
        );
        assert!(matches!(
            parse("--open-plugin insert:3"),
            Err(ArgsError::BadValue { .. })
        ));
        assert_eq!(
            parse("--open-plugin effect:1 --dump-params /tmp/params.txt")
                .expect("valid")
                .script
                .dump_params,
            Some(PathBuf::from("/tmp/params.txt"))
        );
        assert_eq!(
            parse("--open-params channel:2")
                .expect("valid")
                .script
                .open_params,
            Some(ProcessorOwner::Channel(ChannelId(2)))
        );
        assert_eq!(
            parse("--load a.json --play-to a.wav --open-params channel:1"),
            Err(ArgsError::PlayToWithWindowFlag("--open-params"))
        );
        assert_eq!(
            parse("--load a.json --play-to a.wav --dump-params p.txt"),
            Err(ArgsError::PlayToWithWindowFlag("--dump-params"))
        );
        assert_eq!(
            parse("--cache a --cache b"),
            Err(ArgsError::Repeated("--cache".into()))
        );
    }

    #[test]
    fn the_play_to_command_parses() {
        let args =
            parse("--load tests/fixtures/f1_four_on_floor.json --play-to /tmp/f1.wav --bars 2")
                .expect("valid");
        assert_eq!(
            args.play_to,
            Some(PlayTo {
                out: PathBuf::from("/tmp/f1.wav"),
                bars: 2,
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
            })
        );
        let one_bar = parse("--load f1.json --play-to f1.wav").expect("valid");
        assert_eq!(one_bar.play_to.map(|request| request.bars), Some(1));
    }

    #[test]
    fn bits_pick_the_play_to_wav_format() {
        let args = parse("--load f1.json --play-to f1.wav --bits 24").expect("valid");
        assert_eq!(
            args.play_to.and_then(|request| request.bits),
            Some(BitDepth::Int24)
        );
        assert_eq!(
            parse("--load f1.json --bits 16"),
            Err(ArgsError::BitsWithoutWavPlayTo)
        );
        assert_eq!(
            parse("--load f1.json --play-to f1.mp3 --bits 16"),
            Err(ArgsError::BitsWithoutWavPlayTo)
        );
        assert!(matches!(
            parse("--load f1.json --play-to f1.wav --bits 20"),
            Err(ArgsError::BadValue { .. })
        ));
    }

    #[test]
    fn play_to_runs_without_a_window_and_needs_a_project() {
        assert_eq!(
            parse("--play-to a.wav"),
            Err(ArgsError::PlayToWithoutProject)
        );
        assert_eq!(
            parse("--load a.json --play-to a.wav --quit-after 5"),
            Err(ArgsError::PlayToWithWindowFlag("--quit-after"))
        );
        assert_eq!(
            parse("--load a.json --play-to a.wav --window-size 800x600"),
            Err(ArgsError::PlayToWithWindowFlag("--window-size"))
        );
        assert_eq!(
            parse("--load a.json --bars 2"),
            Err(ArgsError::BarsWithoutPlayTo)
        );
        for bad in ["0", "-1", "two"] {
            assert!(
                matches!(
                    parse(&format!("--load a.json --play-to a.wav --bars {bad}")),
                    Err(ArgsError::BadValue { .. })
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn no_flags_opens_an_empty_window_of_the_default_size() {
        let args = parse("").expect("valid");
        assert_eq!(args.project, None);
        assert_eq!(args.window_size, Size::new(1000.0, 560.0));
        assert_eq!(args.script, Script::default());
    }

    // The piano-roll screenshot command.
    #[test]
    fn panels_are_a_list_of_names_each_at_most_once() {
        let both = parse("--panels rack,piano-roll").expect("valid");
        assert_eq!(
            both.panels,
            Panels {
                rack: true,
                playlist: false,
                piano_roll: true,
                mixer: false,
                browser: false,
                settings: false,
            }
        );
        let roll =
            parse("--load p3_roll.json --panels piano-roll --window-size 1000x560").expect("valid");
        assert_eq!(
            roll.panels,
            Panels {
                rack: false,
                playlist: false,
                piano_roll: true,
                mixer: false,
                browser: false,
                settings: false,
            }
        );
        assert_eq!(parse("").expect("valid").panels, Panels::default());
        // "mixers" stands in as a name no panel has.
        for bad in [
            "",
            "rack,rack",
            "mixers",
            "mixer,mixer",
            "rack,",
            "piano-roll,rack,piano-roll",
        ] {
            assert!(
                matches!(
                    Args::parse(["--panels".to_owned(), bad.to_owned()]),
                    Err(ArgsError::BadValue { .. })
                ),
                "{bad:?}"
            );
        }
        assert_eq!(
            parse("--load a.json --play-to a.wav --panels rack"),
            Err(ArgsError::PlayToWithWindowFlag("--panels"))
        );
    }

    // --song starts the window in song mode and picks the mode --play-to renders; --panels playlist
    // shows the playlist alone for the screenshot.
    #[test]
    fn song_is_a_flag_without_a_value_that_also_sets_the_play_to_mode() {
        let args = parse("--load p4_loop.json --song --play-to a.wav --bars 3").expect("valid");
        assert!(args.song);
        assert_eq!(
            args.play_to,
            Some(PlayTo {
                out: PathBuf::from("a.wav"),
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
            })
        );
        assert!(!parse("").expect("valid").song);
        assert_eq!(
            parse("--song --load a.json").expect("valid").project,
            Some(PathBuf::from("a.json")),
            "the word after --song is a flag again"
        );
        assert_eq!(
            parse("--song --song"),
            Err(ArgsError::Repeated("--song".into()))
        );
        let shot = parse("--load p4_two_clips.json --panels playlist --window-size 1000x560")
            .expect("valid");
        assert_eq!(
            shot.panels,
            Panels {
                rack: false,
                playlist: true,
                piano_roll: false,
                mixer: false,
                browser: false,
                settings: false,
            }
        );
        assert!(matches!(
            parse("--panels playlist,playlist"),
            Err(ArgsError::BadValue { .. })
        ));
    }

    // The mixer screenshot command.
    #[test]
    fn panels_mixer_shows_the_mixer_alone() {
        let args =
            parse("--load p5_gain.json --panels mixer --window-size 1000x560").expect("valid");
        assert_eq!(
            args.panels,
            Panels {
                rack: false,
                playlist: false,
                piano_roll: false,
                mixer: true,
                browser: false,
                settings: false,
            }
        );
        assert!(parse("--panels rack,mixer").expect("valid").panels.mixer);
    }

    // Recording the master through --play-to (input has no device offline), the count-in switch,
    // the project written after, the stream threshold and the browser root.
    #[test]
    fn recording_flags_go_with_play_to() {
        let args = parse(
            "--load p8_rec.json --song --play-to a.wav --bars 2 --record master --save-to out.json",
        )
        .expect("valid");
        let play = args.play_to.expect("a render");
        assert_eq!(
            (play.record, play.count_in, play.save_to),
            (
                Some(RecordSource::Master),
                true,
                Some(PathBuf::from("out.json"))
            )
        );
        let quiet =
            parse("--load a.json --play-to a.wav --record master --count-in 0").expect("valid");
        assert!(!quiet.play_to.expect("a render").count_in);
        assert_eq!(
            parse("--load a.json --play-to a.wav --record input"),
            Err(ArgsError::RecordInputOffline)
        );
        assert_eq!(
            parse("--load a.json --play-to a.wav --count-in 1"),
            Err(ArgsError::CountInWithoutRecord)
        );
        assert_eq!(
            parse("--load a.json --record master"),
            Err(ArgsError::WithoutPlayTo("--record"))
        );
        assert!(matches!(
            parse("--load a.json --play-to a.wav --record both"),
            Err(ArgsError::BadValue { .. })
        ));
        let window = parse(
            "--panels browser,playlist --browser-root tests/fixtures/browser --stream-threshold 0",
        )
        .expect("valid");
        assert!(window.panels.browser && window.panels.playlist);
        assert_eq!(
            window.browser_root,
            Some(PathBuf::from("tests/fixtures/browser"))
        );
        assert_eq!(window.stream_threshold, Some(0));
        assert_eq!(
            parse("--load a.json --play-to a.wav --browser-root x"),
            Err(ArgsError::PlayToWithWindowFlag("--browser-root"))
        );
    }

    // MIDI events, note recording and its quantize switch go with --play-to; the stretch cache goes
    // with both.
    #[test]
    fn midi_and_note_recording_flags_go_with_play_to() {
        let args = parse(
            "--load p9_record.json --play-to a.wav --bars 2 --midi-events ev.json \
             --record notes --count-in 0 --quantize off --save-to s.json --cache-dir /c",
        )
        .expect("valid");
        let play = args.play_to.expect("a render");
        assert_eq!(
            (
                play.record,
                play.record_notes,
                play.quantize,
                play.count_in,
                play.midi_events,
                play.cache_dir
            ),
            (
                None,
                true,
                false,
                false,
                Some(PathBuf::from("ev.json")),
                Some(PathBuf::from("/c"))
            )
        );
        // The song-take film: a scripted window with no device, a rehearsed take, shots taken
        // again; none goes with --play-to, and --screenshot-every needs --screenshot.
        let film = parse(
            "--load a.json --null-output --rehearse ev.json --screenshot s.png \
             --screenshot-after 400 --screenshot-every 300 --quit-after 9000",
        )
        .expect("valid");
        assert!(film.script.null_output);
        assert_eq!(film.script.rehearse, Some(PathBuf::from("ev.json")));
        assert_eq!(
            film.script.screenshot.and_then(|shot| shot.every),
            Some(Duration::from_millis(300))
        );
        for flag in ["--null-output", "--rehearse ev.json"] {
            assert!(matches!(
                parse(&format!("--load a.json --play-to a.wav {flag}")),
                Err(ArgsError::PlayToWithWindowFlag(_))
            ));
        }
        assert_eq!(
            parse("--load a.json --screenshot-every 300").map(|args| args.play_to),
            Err(ArgsError::IncompleteScreenshot)
        );
        // Recording v2: the read-only dump of the pattern's notes after a call.
        let dumped = parse("--load a.json --play-to a.wav --record notes --dump-notes-at 210")
            .expect("valid");
        assert_eq!(dumped.play_to.expect("a render").dump_notes_at, Some(210));
        assert_eq!(
            parse("--load a.json --dump-notes-at 3").map(|args| args.play_to),
            Err(ArgsError::WithoutPlayTo("--dump-notes-at"))
        );
        assert!(matches!(
            parse("--load a.json --play-to a.wav --dump-notes-at soon"),
            Err(ArgsError::BadValue { .. })
        ));
        let quantized = parse("--load a.json --play-to a.wav --record notes").expect("valid");
        assert!(
            quantized.play_to.expect("a render").quantize,
            "on by default"
        );
        assert_eq!(
            parse("--load a.json --play-to a.wav --quantize on"),
            Err(ArgsError::QuantizeWithoutNotes)
        );
        assert_eq!(
            parse("--load a.json --play-to a.wav --record master --quantize on"),
            Err(ArgsError::QuantizeWithoutNotes)
        );
        assert_eq!(
            parse("--load a.json --play-to a.wav --song --record notes"),
            Err(ArgsError::NotesInSong)
        );
        assert_eq!(
            parse("--load a.json --midi-events ev.json"),
            Err(ArgsError::WithoutPlayTo("--midi-events"))
        );
        assert_eq!(
            parse("--load a.json --quantize off"),
            Err(ArgsError::WithoutPlayTo("--quantize"))
        );
        assert!(matches!(
            parse("--load a.json --play-to a.wav --record notes --quantize maybe"),
            Err(ArgsError::BadValue { .. })
        ));
        let window = parse("--load a.json --cache-dir /c").expect("valid");
        assert_eq!(window.cache_dir, Some(PathBuf::from("/c")));
    }

    // The Settings window alone (it was the settings panel before), with given port
    // names or none.
    #[test]
    fn midi_ports_stand_in_for_the_systems_and_settings_is_a_panel() {
        let none = parse("--panels settings --midi-ports none").expect("valid");
        assert!(none.panels.settings && !none.panels.rack);
        assert_eq!(none.midi_ports, Some(Vec::new()));
        let two =
            Args::parse(["--midi-ports".to_owned(), "Port A,Port B".to_owned()]).expect("valid");
        assert_eq!(
            two.midi_ports,
            Some(vec!["Port A".to_owned(), "Port B".to_owned()])
        );
        assert_eq!(parse("").expect("valid").midi_ports, None);
        assert!(matches!(
            Args::parse(["--midi-ports".to_owned(), "A,,B".to_owned()]),
            Err(ArgsError::BadValue { .. })
        ));
        assert_eq!(
            parse("--load a.json --play-to a.wav --midi-ports none"),
            Err(ArgsError::PlayToWithWindowFlag("--midi-ports"))
        );
    }

    // A measured run starts playing by itself and prints what it timed and counted as it
    // closes; neither has a meaning without a window.
    #[test]
    fn play_and_stats_are_window_flags_without_a_value() {
        let args = parse("--load f1.json --play --stats --quit-after 25000").expect("valid");
        assert!(args.script.play && args.stats);
        assert_ne!(args.script, Script::default(), "a playing run is scripted");
        let quiet = parse("--load f1.json").expect("valid");
        assert!(!quiet.script.play && !quiet.stats);
        assert_eq!(
            parse("--stats").expect("valid").script,
            Script::default(),
            "printing timings alone leaves the run unscripted"
        );
        assert_eq!(
            parse("--play --play"),
            Err(ArgsError::Repeated("--play".into()))
        );
        assert_eq!(
            parse("--load a.json --play-to a.wav --play"),
            Err(ArgsError::PlayToWithWindowFlag("--play"))
        );
        assert_eq!(
            parse("--load a.json --play-to a.wav --stats"),
            Err(ArgsError::PlayToWithWindowFlag("--stats"))
        );
    }

    #[test]
    fn the_device_rate_is_a_sample_rate_and_not_for_play_to() {
        let args = parse("--load f1.json --device-rate 48000").expect("valid");
        assert_eq!(args.device_rate, Some(48_000));
        assert_eq!(parse("").expect("valid").device_rate, None);
        for bad in ["0", "7999", "384001", "48k", "-1"] {
            assert!(
                matches!(
                    parse(&format!("--device-rate {bad}")),
                    Err(ArgsError::BadValue { .. })
                ),
                "{bad}"
            );
        }
        assert_eq!(
            parse("--load a.json --play-to a.wav --device-rate 48000"),
            Err(ArgsError::PlayToWithDeviceRate)
        );
    }

    #[test]
    fn window_size_is_width_x_height() {
        let args = parse("--window-size 1280x720").expect("valid");
        assert_eq!(args.window_size, Size::new(1280.0, 720.0));
        for bad in ["1280", "0x720", "1280x", "x720", "-5x5", "1280x720x2"] {
            assert!(
                matches!(
                    parse(&format!("--window-size {bad}")),
                    Err(ArgsError::BadValue { .. })
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn delays_are_positive_whole_milliseconds() {
        for bad in ["0", "-1", "1.5", "soon"] {
            assert!(
                matches!(
                    parse(&format!("--quit-after {bad}")),
                    Err(ArgsError::BadValue { .. })
                ),
                "{bad}"
            );
        }
    }

    // `dawsome-app song.dawsome`, as the desktop entry starts it with a file (a double-click on
    // one in a file manager), is `--load song.dawsome`; a `.json` project is taken the same way.
    #[test]
    fn a_bare_path_is_the_project_to_open() {
        assert_eq!(parse("song.json"), parse("--load song.json"));
        let own = parse("song.dawsome").expect("parses");
        assert_eq!(own.project, Some(PathBuf::from("song.dawsome")));
        assert_eq!(Ok(own), parse("--load song.dawsome"));
        let args = parse("--song song.json --panels mixer").expect("parses");
        assert_eq!(args.project, Some(PathBuf::from("song.json")));
        assert!(args.song);
    }

    #[test]
    fn malformed_command_lines_are_named() {
        assert_eq!(
            parse("--load"),
            Err(ArgsError::MissingValue("--load".into()))
        );
        assert_eq!(parse("--fast 1"), Err(ArgsError::Unknown("--fast".into())));
        assert_eq!(
            parse("--load a.json --load b.json"),
            Err(ArgsError::Repeated("--load".into()))
        );
        assert_eq!(
            parse("a.json --load b.json"),
            Err(ArgsError::Repeated("--load".into()))
        );
        assert_eq!(parse("-x"), Err(ArgsError::Unknown("-x".into())));
        assert_eq!(
            parse("--screenshot a.png"),
            Err(ArgsError::IncompleteScreenshot)
        );
        assert_eq!(
            parse("--screenshot-after 800"),
            Err(ArgsError::IncompleteScreenshot)
        );
    }

    #[test]
    fn quitting_must_come_after_the_screenshot() {
        assert_eq!(
            parse("--screenshot a.png --screenshot-after 800 --quit-after 800"),
            Err(ArgsError::QuitBeforeScreenshot {
                quit: 800,
                shot: 800
            })
        );
        assert!(parse("--screenshot a.png --screenshot-after 800 --quit-after 801").is_ok());
        assert!(parse("--quit-after 5").is_ok());
    }

    // 6d K4: --callouts is on or off, once.
    #[test]
    fn the_callouts_flag_is_on_or_off() {
        assert_eq!(take_callouts(&mut words("--callouts on")), Ok(Some(true)));
        assert_eq!(take_callouts(&mut words("--callouts off")), Ok(Some(false)));
        assert_eq!(take_callouts(&mut words("--hints on")), Ok(None));
        assert_eq!(
            take_callouts(&mut words("--callouts yes")),
            Err(bad_value("--callouts", "on or off", "yes".to_owned()))
        );
    }

    // 6d K3: --settings-page names the settings panel's page a scripted run opens on, once.
    #[test]
    fn the_settings_page_flag_names_a_page() {
        use crate::ui::settings_layout::SettingsPage;
        let mut line = words("--panels settings --settings-page keys");
        assert_eq!(take_settings_page(&mut line), Ok(Some(SettingsPage::Keys)));
        assert_eq!(line, words("--panels settings"));
        assert_eq!(take_settings_page(&mut words("--load a.json")), Ok(None));
        assert_eq!(
            take_settings_page(&mut words("--settings-page audio")),
            Err(bad_value(
                "--settings-page",
                "general or keys",
                "audio".to_owned()
            ))
        );
        assert_eq!(
            take_settings_page(&mut words("--settings-page")),
            Err(ArgsError::MissingValue("--settings-page".to_owned()))
        );
        assert_eq!(
            take_settings_page(&mut words("--settings-page keys --settings-page general")),
            Err(ArgsError::Repeated("--settings-page".to_owned()))
        );
    }
}
