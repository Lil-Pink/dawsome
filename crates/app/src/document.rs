//! The open document as the window names it, and what the app may touch outside its window:
//! the audio device, where settings and autosaves live, and whether file work runs in the
//! background.

use std::cell::Ref;
use std::path::{Path, PathBuf};

use dawsome_engine::processor::Registry;
use dawsome_engine::project::ProcessorOwner;
#[cfg(test)]
use dawsome_engine::realtime::Player;
use dawsome_host_clap::{Catalogue, ClapHost, PluginSetup};
use dawsome_host_common::window::WindowId;
use dawsome_host_vst3::Vst3Host;
use dawsome_media::{Playback, Waiting};

#[cfg(test)]
use crate::audio::{Output, StartFailure};
use crate::live_input::InputDevices;
use crate::midi::PortSource;
use crate::midi_out::MidiOutputs;
use crate::project_file;
use crate::settings;
use crate::ui::browser_model;

/// Where the open document came from and where Save writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// The file it was opened from and Save writes; `None` for a new document, which Save asks
    /// a name for.
    pub path: Option<PathBuf>,
    /// The name the title shows.
    pub name: String,
}

impl Document {
    pub fn untitled() -> Self {
        Self {
            path: None,
            name: "Untitled".to_owned(),
        }
    }

    pub fn saved_at(path: &Path) -> Self {
        Self {
            path: Some(path.to_path_buf()),
            name: file_name(path),
        }
    }

    /// `song - Dawsome` for `song.dawsome` or `song.json` (the name without its extension),
    /// with `*` after the name when there are unsaved changes: `song* - Dawsome`,
    /// `Untitled - Dawsome`.
    pub fn title(&self, dirty: bool) -> String {
        let star = if dirty { "*" } else { "" };
        format!("{}{star} - Dawsome", stem_of(&self.name))
    }

    /// The name Save As suggests: the file's name without its extension, "Untitled" for a
    /// document with none, and `.dawsome`: `song.json` → `song.dawsome`.
    pub fn save_as_name(&self) -> String {
        let stem = self
            .path
            .as_deref()
            .map_or_else(|| "Untitled".to_owned(), |path| stem_of(&file_name(path)));
        format!("{stem}.{}", project_file::EXTENSION)
    }
}

/// `name` without its extension: `song.json` → `song`, `my.song.dawsome` → `my.song`.
fn stem_of(name: &str) -> String {
    Path::new(name).file_stem().map_or_else(
        || name.to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// How the window gets its sound out.
#[derive(Debug, Clone, Copy)]
pub enum AudioChoice {
    /// The default output device, at this rate when given (`--device-rate`).
    Device { rate: Option<u32> },
    /// No device: a test, which plays the player by hand.
    #[cfg(test)]
    Off,
    /// A device a test stands in, opened by this function.
    #[cfg(test)]
    Fake(fn(Player) -> Result<Output, StartFailure>),
    /// Devices a test stands in, the one named (none: the default) opened by this function.
    #[cfg(test)]
    FakeNamed(fn(Player, Option<&str>) -> Result<Output, StartFailure>),
    /// As [`FakeNamed`](Self::FakeNamed), each stand-in's own rate told by the second function
    /// without opening it, as [`crate::audio::device_own_rate`] tells a real device's.
    #[cfg(test)]
    FakeRated(
        fn(Player, Option<&str>) -> Result<Output, StartFailure>,
        fn(Option<&str>) -> Option<u32>,
    ),
}

/// Where Settings' list of output devices comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputSource {
    /// The system's output devices, listed in the background when Settings opens.
    System,
    /// These devices only: a scripted run lists none, so its screenshots never depend on the
    /// machine's devices, and a test names its own.
    Given(Vec<crate::audio::ListedOutput>),
}

/// What the app may touch outside its window.
#[derive(Debug, Clone)]
pub struct Host {
    pub audio: AudioChoice,
    /// The recent-files list; `None` keeps it in memory only.
    pub recent_file: Option<PathBuf>,
    /// Where a document with no file of its own autosaves; `None` turns autosave (and the
    /// recovery questions) off.
    pub untitled_autosave: Option<PathBuf>,
    /// File dialogs, autosave writes and exports run as background work; off, they run at once
    /// on the calling thread, and dialogs do not open (a test sends their result instead).
    pub background: bool,
    /// The CLAP host every session's CLAP plugins run in; its thread is the plugins' main
    /// thread.
    pub clap: ClapHost,
    /// The VST3 host every session's VST3 plugins run in, on the same thread, over the same
    /// catalogue.
    pub vst3: Vst3Host,
    /// Where plugins are scanned and the catalogue is kept; `None` finds no plugins.
    pub plugins: Option<PluginSetup>,
    /// Whether the app scans for plugins and writes the catalogue (a scripted run does not).
    pub scans: bool,
    /// How the window's sessions play their files: live, never waiting for the disk, with
    /// long files at another rate converted into the cache (not in a scripted run).
    pub playback: Playback,
    /// The file the browser keeps the folders added to it in; `None` keeps them in memory.
    pub browser_folders: Option<PathBuf>,
    /// The user's Music folder, a root of the browser.
    pub music: Option<PathBuf>,
    /// The one folder the browser lists instead of its roots (`--browser-root`).
    pub browser_root: Option<PathBuf>,
    /// Where a document with no file of its own records to; `None` records nothing until it
    /// is saved.
    pub recordings: Option<PathBuf>,
    /// Where the demo project is put the first time it is needed; `None` puts it nowhere, and
    /// File → Open demo says so.
    pub demo_dir: Option<PathBuf>,
    /// Where the starter kit's sounds are put the first time one is previewed or dragged;
    /// `None` puts them nowhere, and the Library says so when one is.
    pub starter_kit: Option<PathBuf>,
    /// The data folder's instrument sets, which the Library lists (it only reads there); `None`
    /// lists none.
    pub instruments: Option<PathBuf>,
    /// The instrument sets installed with the program, listed after the data folder's (a set
    /// folder of the same name there shadows one here); `None` lists none.
    pub installed_instruments: Option<PathBuf>,
    /// The app's settings file, read at start; `None` keeps the defaults.
    pub settings_file: Option<PathBuf>,
    /// Whether changed settings are written back to it (a scripted run only reads it).
    pub saves_settings: bool,
    /// Where the MIDI input ports come from: the system's only for a run nobody scripts, or a
    /// `--midi-monitor` run's one port.
    pub midi_ports: PortSource,
    /// A `--midi-monitor` run: what the ports deliver, and what the player plays of it, is
    /// printed.
    pub midi_monitor: bool,
    /// Where Settings' output devices come from: the system's only for a run nobody scripts.
    pub audio_outputs: OutputSource,
    /// Where the MIDI output ports the input channels' MIDI outs send to come from: the
    /// system's (listed and opened) only for a run nobody scripts; a scripted run and the tests
    /// name theirs, and what goes to them is recorded, never sent.
    pub midi_outputs: MidiOutputs,
    /// The recording a test reads what its MIDI outs sent from.
    #[cfg(test)]
    pub midi_sent: Option<crate::midi_out::Recording>,
    /// Where the input channels' devices come from: the system's (listed and opened) only for
    /// a run nobody scripts; a scripted run and the tests name theirs, which open no stream.
    pub audio_inputs: InputDevices,
    /// Whether the hint bar shows when the settings ask for it: not in a scripted run, whose
    /// screenshots pin the panels' heights as they were before it (`--hints on` shows it).
    pub hint_bar: bool,
    /// Whether the first-run notes show until put away: not in a scripted run, whose
    /// screenshots show the window under them (`--callouts on` shows them).
    pub callouts: bool,
    /// Where the pointer is over the main window (its X11 id or Win32 handle), in the window's
    /// device pixels: what a file dropped from the desktop lands under, the drop itself carrying
    /// no position. The window system's in a desktop session; a test gives its own answer, and
    /// `None` leaves the drop to the cursor the window last saw.
    pub pointer: fn(WindowId) -> Option<(i32, i32)>,
    /// A `--drop-monitor` run: where each file dropped on the window lands is printed.
    pub drop_monitor: bool,
}

impl Host {
    /// A desktop session, with plugins found as `plugins` says and stretched clips kept in
    /// `cache_dir` when given. A scripted run (a screenshot, a timed quit) keeps no recent
    /// files, writes no autosave, runs no plugin scan and keeps stretched clips only where
    /// `cache_dir` says, so running it changes nothing on disk outside its output.
    pub fn desktop(
        device_rate: Option<u32>,
        scripted: bool,
        plugins: Option<PluginSetup>,
        stream_threshold: Option<u64>,
        cache_dir: Option<PathBuf>,
    ) -> Self {
        let catalogue = plugins
            .as_ref()
            .and_then(|setup| Catalogue::load(&setup.cache).ok())
            .unwrap_or_default();
        Self {
            audio: AudioChoice::Device { rate: device_rate },
            recent_file: (!scripted).then(project_file::recent_file).flatten(),
            untitled_autosave: (!scripted).then(project_file::untitled_autosave).flatten(),
            background: true,
            clap: ClapHost::new(catalogue.clone()),
            vst3: Vst3Host::new(catalogue),
            plugins,
            scans: !scripted,
            playback: Playback {
                stream_threshold,
                waiting: Waiting::Never,
                convert_dir: (!scripted)
                    .then(dawsome_media::sounds::convert_dir)
                    .flatten(),
                stretch_dir: cache_dir
                    .or_else(|| (!scripted).then(dawsome_media::stretch_dir).flatten()),
                edited_dir: (!scripted).then(dawsome_media::edited_dir).flatten(),
            },
            browser_folders: (!scripted).then(project_file::browser_file).flatten(),
            music: browser_model::music_dir(),
            browser_root: None,
            recordings: (!scripted).then(project_file::recordings_dir).flatten(),
            demo_dir: (!scripted).then(project_file::demo_dir).flatten(),
            starter_kit: (!scripted).then(project_file::starter_kit_dir).flatten(),
            instruments: project_file::instruments_dir(),
            installed_instruments: dawsome_media::paths::installed_instruments(),
            settings_file: settings::settings_file(),
            saves_settings: !scripted,
            hint_bar: !scripted,
            callouts: !scripted,
            midi_ports: if scripted {
                PortSource::Given(Vec::new())
            } else {
                PortSource::Devices
            },
            midi_monitor: false,
            audio_outputs: if scripted {
                OutputSource::Given(Vec::new())
            } else {
                OutputSource::System
            },
            midi_outputs: if scripted {
                MidiOutputs::Given(Vec::new())
            } else {
                MidiOutputs::System
            },
            #[cfg(test)]
            midi_sent: None,
            audio_inputs: if scripted {
                InputDevices::Given(Vec::new())
            } else {
                InputDevices::System
            },
            pointer: dawsome_host_common::window::pointer_in,
            drop_monitor: false,
        }
    }

    /// No device, no settings, nothing in the background, no plugins.
    #[cfg(test)]
    pub fn offline() -> Self {
        Self {
            audio: AudioChoice::Off,
            recent_file: None,
            untitled_autosave: None,
            background: false,
            clap: ClapHost::new(Catalogue::default()),
            vst3: Vst3Host::new(Catalogue::default()),
            plugins: None,
            scans: false,
            playback: Playback::offline(None),
            browser_folders: None,
            music: None,
            browser_root: None,
            recordings: None,
            demo_dir: None,
            starter_kit: None,
            instruments: None,
            installed_instruments: None,
            settings_file: None,
            saves_settings: false,
            hint_bar: false,
            callouts: false,
            midi_ports: PortSource::Given(Vec::new()),
            midi_monitor: false,
            audio_outputs: OutputSource::Given(Vec::new()),
            midi_outputs: MidiOutputs::Given(Vec::new()),
            midi_sent: None,
            audio_inputs: InputDevices::Given(Vec::new()),
            pointer: |_| None,
            drop_monitor: false,
        }
    }

    /// No device, no settings, nothing in the background, the plugins `catalogue` lists in
    /// both hosts.
    #[cfg(test)]
    pub fn with_catalogue(catalogue: Catalogue) -> Self {
        Self {
            clap: ClapHost::new(catalogue.clone()),
            vst3: Vst3Host::new(catalogue),
            ..Self::offline()
        }
    }

    pub fn autosaves(&self) -> bool {
        self.untitled_autosave.is_some()
    }

    /// The plugin catalogue the lists show: the CLAP host's copy, which a finished scan gives
    /// both hosts.
    pub fn catalogue(&self) -> Ref<'_, Catalogue> {
        self.clap.catalogue()
    }

    /// A finished scan's catalogue, for both hosts. Their running instances are not touched.
    pub fn set_catalogue(&self, catalogue: Catalogue) {
        self.vst3.set_catalogue(catalogue.clone());
        self.clap.set_catalogue(catalogue);
    }

    /// Notes `owner`'s plugin was not sent, in whichever host plays it.
    pub fn dropped_notes(&self, owner: ProcessorOwner) -> u64 {
        self.clap.dropped_notes(owner) + self.vst3.dropped_notes(owner)
    }

    /// What a session of this app creates processors from: the native effects and both plugin
    /// hosts, each through a handle of the session's own, so its plugin instances go with it.
    pub fn registry(&self) -> Registry {
        let mut registry = Registry::native();
        registry.add_host(Box::new(self.clap.session()));
        registry.add_host(Box::new(self.vst3.session()));
        registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The title names the file without its extension, `.dawsome` or `.json` alike.
    #[test]
    fn titles_name_the_document_and_mark_unsaved_changes() {
        let song = Document::saved_at(Path::new("/music/song.json"));
        assert_eq!(song.title(false), "song - Dawsome");
        assert_eq!(song.title(true), "song* - Dawsome");
        let own = Document::saved_at(Path::new("/music/my.song.dawsome"));
        assert_eq!(own.title(false), "my.song - Dawsome");
        assert_eq!(Document::untitled().title(false), "Untitled - Dawsome");
    }

    // Save As suggests the name with `.dawsome`, whatever the document was opened from.
    #[test]
    fn save_as_suggests_the_name_with_the_dawsome_extension() {
        assert_eq!(Document::untitled().save_as_name(), "Untitled.dawsome");
        let cases = [
            Document::saved_at(Path::new("/music/song.json")),
            Document::saved_at(Path::new("/music/song.dawsome")),
        ];
        for document in cases {
            assert_eq!(document.save_as_name(), "song.dawsome", "{document:?}");
        }
        assert_eq!(
            Document::saved_at(Path::new("/music/my.song.dawsome")).save_as_name(),
            "my.song.dawsome"
        );
    }

    // The starter kit goes into the data folder only for a run nobody scripts.
    #[test]
    fn a_scripted_run_writes_no_starter_kit() {
        assert_eq!(
            Host::desktop(None, true, None, None, None).starter_kit,
            None
        );
    }

    #[test]
    fn a_scripted_run_keeps_no_settings() {
        let host = Host::desktop(None, true, None, None, None);
        assert!(!host.autosaves());
        assert_eq!(host.playback.stretch_dir, None, "nor stretched clips");
        let given = Host::desktop(None, true, None, None, Some(PathBuf::from("/c")));
        assert_eq!(given.playback.stretch_dir, Some(PathBuf::from("/c")));
        assert!(!host.scans, "no plugin scan either");
        assert_eq!((host.recent_file, host.untitled_autosave), (None, None));
        assert_eq!(
            host.playback.convert_dir, None,
            "nor converted files in the cache"
        );
        assert_eq!(host.browser_folders, None, "nor the browser's folders");
        assert_eq!(
            host.recordings, None,
            "nor recordings of untitled documents"
        );
        assert_eq!(host.demo_dir, None, "nor the demo");
        assert!(
            !host.saves_settings,
            "it reads the settings, never writes them"
        );
        assert_eq!(
            host.midi_ports,
            PortSource::Given(Vec::new()),
            "nor opens a MIDI port"
        );
        assert_eq!(
            host.midi_outputs,
            MidiOutputs::Given(Vec::new()),
            "nor a MIDI output"
        );
    }
}
