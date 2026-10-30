//! Dawsome desktop app: a toolbar over the loaded project's panels (the channel rack, the
//! playlist, the piano roll, the mixer), playing through the default audio output.
//!
//! The app owns the project document in a [`session::Session`]; every edit goes from there to
//! the audio callback, which renders with the engine's player and nothing else. `--play-to`
//! runs the same path into a WAV file instead of the device.

// A release build on Windows opens no console window beside its own; a debug build keeps one for
// what the app prints. Scan children print to a pipe either way.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod args;
mod audio;
mod capture;
// The heap-call counter the audio tests use; it is the test binary's global allocator.
#[cfg(test)]
#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;
mod crash;
mod demo;
mod desktop;
mod document;
mod edit_names;
mod executor;
mod former_folders;
mod headless;
mod help;
mod keys;
mod links;
mod live_input;
mod midi;
mod midi_out;
mod notes;
mod play_to;
#[cfg(test)]
mod probe;
mod project_file;
mod recent;
mod record;
mod session;
mod sets;
mod settings;
mod starter_kit;
mod ui;
mod window_stats;

use std::cell::Cell;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use app::{App, Start};
use args::{Args, PluginFlags};
use dawsome_engine::project::ChannelId;
use dawsome_host_common::catalogue::PluginFormat;
use dawsome_host_common::scan::ChildCommand;
use dawsome_host_common::PluginSetup;
use document::{Document, Host};
use iced::{window, Task};
use project_file::Opened;
use session::Session;

/// The flag a plugin scan child of this app is started with; the file to describe follows.
const SCAN_CHILD: &str = "--scan-child";

/// The flag, given alone, that prints the instrument sets and shipped folders of sounds the
/// Library finds where a desktop run looks for them, and opens no window: a headless check of
/// an install. It only reads.
const LIST_INSTRUMENTS: &str = "--list-instruments";

/// The flag, given alone, that prints the folders a plugin scan looks in when no
/// `--plugin-path` is given, a line each, and opens no window: a headless check that an
/// install's bundled plugins are found. It only reads.
const LIST_PLUGIN_FOLDERS: &str = "--list-plugin-folders";

/// The flag, given alone, that prints the help folder the Help window reads its pictures from,
/// then each picture the guide names, found there or missing, a line each, and opens no window:
/// a headless check that an install's pictures are where the app looks. It only reads.
const LIST_HELP: &str = "--list-help";

/// The flag, given alone, that prints the output devices Settings offers, as it lists them
/// (the one the settings chose marked), and opens no window and no device: a check by hand of
/// what the system's audio host lists. It only reads.
const LIST_OUTPUTS: &str = "--list-outputs";

/// The flag, given alone, that prints the program's name and version and opens no window.
const VERSION: &str = "--version";

/// The flags, either given alone, that print the usage and open no window, as `dawsome --help`
/// does.
const HELP: [&str; 2] = ["--help", "-h"];

/// Where plugins of both formats are found and the catalogue kept, with this app as the scan
/// child.
fn plugin_setup(flags: &PluginFlags) -> Option<PluginSetup> {
    let child = ChildCommand {
        program: std::env::current_exe().ok()?,
        args: vec![SCAN_CHILD.into()],
    };
    PluginSetup::for_formats(
        PluginFormat::ALL,
        flags.dirs.clone(),
        flags.cache.clone(),
        child,
    )
}

/// A scan child's work on `file`, as its format's host loads it (what `dawsome scan --child`
/// does): the exit code.
fn scan_child(file: &std::path::Path) -> i32 {
    match PluginFormat::of_path(file) {
        Some(PluginFormat::Vst3) => dawsome_host_vst3::scan::run_child(file),
        _ => dawsome_host_clap::scan::run_child(file),
    }
}

fn main() -> ExitCode {
    crash::install_hook();
    let mut raw: Vec<String> = std::env::args().skip(1).collect();
    if let [flag, file] = raw.as_slice() {
        if flag == SCAN_CHILD {
            return ExitCode::from(u8::try_from(scan_child(file.as_ref())).unwrap_or(1));
        }
    }
    if raw == [VERSION] {
        println!("dawsome-app {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if let [flag] = raw.as_slice() {
        if HELP.contains(&flag.as_str()) {
            println!("{}", args::USAGE);
            return ExitCode::SUCCESS;
        }
    }
    if raw == [LIST_INSTRUMENTS] {
        let shipped: Vec<PathBuf> = [
            project_file::instruments_dir(),
            dawsome_media::paths::installed_instruments(),
        ]
        .into_iter()
        .flatten()
        .collect();
        let added = ui::browser_model::Folders::load(project_file::browser_file().as_deref());
        print!(
            "{}",
            sets::listing(&sets::scan_roots(&shipped, &added.folders))
        );
        return ExitCode::SUCCESS;
    }
    if raw == [LIST_HELP] {
        let folder = help::folder();
        print!(
            "{}",
            help::listing(folder.as_deref(), &help::Guide::shipped())
        );
        return ExitCode::SUCCESS;
    }
    if raw == [LIST_OUTPUTS] {
        let settings = settings::Settings::load(settings::settings_file().as_deref());
        let chosen = settings
            .audio_output()
            .map(|id| audio::Chosen::new(id, settings.audio_output_name()));
        let (choices, problem) = audio::output_choices(&audio::SystemOutputs, chosen);
        print!("{}", audio::listing(&choices, problem.as_deref()));
        return ExitCode::SUCCESS;
    }
    if raw == [LIST_PLUGIN_FOLDERS] {
        for dir in dawsome_host_common::catalogue::search_paths_for(PluginFormat::ALL) {
            println!("{}", dir.display());
        }
        return ExitCode::SUCCESS;
    }
    let extras = match args::Extras::take(&mut raw) {
        Ok(extras) => extras,
        Err(error) => {
            eprintln!("dawsome-app: {error}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let shown = match args::Shown::take(&mut raw) {
        Ok(shown) => shown,
        Err(error) => {
            eprintln!("dawsome-app: {error}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let library_preview = match args::take_library_preview(&mut raw) {
        Ok(preview) => preview,
        Err(error) => {
            eprintln!("dawsome-app: {error}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let callouts = match args::take_callouts(&mut raw) {
        Ok(callouts) => callouts,
        Err(error) => {
            eprintln!("dawsome-app: {error}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let settings_page = match args::take_settings_page(&mut raw) {
        Ok(page) => page,
        Err(error) => {
            eprintln!("dawsome-app: {error}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let scroll_to_end = match args::take_scroll(&mut raw) {
        Ok(at_end) => at_end,
        Err(error) => {
            eprintln!("dawsome-app: {error}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let drag_over = match args::take_drag_over(&mut raw) {
        Ok(channel) => channel,
        Err(error) => {
            eprintln!("dawsome-app: {error}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let channel_menu = match args::take_channel_menu(&mut raw) {
        Ok(channel) => channel,
        Err(error) => {
            eprintln!("dawsome-app: {error}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let select_clip = match args::take_select_clip(&mut raw) {
        Ok(clip) => clip,
        Err(error) => {
            eprintln!("dawsome-app: {error}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let curve_menu = match args::take_curve_menu(&mut raw) {
        Ok(automation) => automation,
        Err(error) => {
            eprintln!("dawsome-app: {error}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let slot_menu = match args::take_slot_menu(&mut raw) {
        Ok(menu) => menu,
        Err(error) => {
            eprintln!("dawsome-app: {error}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let library_menu = match args::take_library_menu(&mut raw) {
        Ok(port) => port,
        Err(error) => {
            eprintln!("dawsome-app: {error}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let midi_monitor = match args::take_midi_monitor(&mut raw) {
        Ok(port) => port,
        Err(error) => {
            eprintln!("dawsome-app: {error}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let drop_monitor = args::take_drop_monitor(&mut raw);
    let sample_editor = match args::take_sample_editor(&mut raw) {
        Ok(editor) => editor,
        Err(error) => {
            eprintln!("dawsome-app: {error}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let shot_window = match args::take_shot_window(&mut raw) {
        Ok(window) => window,
        Err(error) => {
            eprintln!("dawsome-app: {error}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    let layout_given = args::layout_given(&raw);
    let args = match Args::parse(raw) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("dawsome-app: {error}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    // With --play-to, --open-instrument alone of the window's flags means something (with
    // --library-preview, taken apart above): the MIDI notes no link routes play its channel, as
    // while its window is open.
    let other_window_flags = args::Extras {
        open_instrument: None,
        ..extras.clone()
    };
    if args.play_to.is_some()
        && (other_window_flags != args::Extras::default()
            || shown.is_some()
            || shot_window.is_some()
            || scroll_to_end
            || drag_over.is_some()
            || channel_menu.is_some()
            || select_clip.is_some()
            || curve_menu.is_some()
            || slot_menu.is_some()
            || library_menu.is_some()
            || midi_monitor.is_some()
            || drop_monitor
            || sample_editor.is_some())
    {
        eprintln!(
            "dawsome-app: --open-adder, --hints, --show, --scroll, --drag-over, --channel-menu, \
             --select-clip, --curve-menu, --slot-menu, --library-menu, --midi-monitor, \
             --drop-monitor, --open-sample-editor and --screenshot-window are for the window, \
             not --play-to\n\n{}",
            args::USAGE
        );
        return ExitCode::from(2);
    }
    let lists_params = args.script.open_plugin.is_some() || args.script.open_params.is_some();
    if args.script.dump_params.is_some() && !lists_params {
        eprintln!(
            "dawsome-app: --dump-params writes the window --open-plugin or --open-params \
             opens\n\n{}",
            args::USAGE
        );
        return ExitCode::from(2);
    }
    if let Some(window) = shot_window {
        let refused = if args.script.screenshot.is_none() {
            Some("--screenshot-window goes with --screenshot")
        } else if window == headless::ShotWindow::Instrument && extras.open_instrument.is_none() {
            Some("--screenshot-window instrument needs the window --open-instrument opens")
        } else if window == headless::ShotWindow::Settings && !args.panels.settings {
            Some("--screenshot-window settings needs the window --panels settings opens")
        } else if window == headless::ShotWindow::Help
            && !matches!(shown, Some(args::Shown::Help(_)))
        {
            Some("--screenshot-window help needs the window --show help opens")
        } else if window == headless::ShotWindow::Params && !lists_params {
            Some("--screenshot-window params needs the window --open-plugin or --open-params opens")
        } else if window == headless::ShotWindow::SampleEditor && sample_editor.is_none() {
            Some("--screenshot-window sample-editor needs the window --open-sample-editor opens")
        } else {
            None
        };
        if let Some(refused) = refused {
            eprintln!("dawsome-app: {refused}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    }
    let scripted = args.script != headless::Script::default()
        || extras.scripted()
        || shown.is_some()
        || scroll_to_end
        || drag_over.is_some()
        || channel_menu.is_some()
        || select_clip.is_some()
        || curve_menu.is_some()
        || slot_menu.is_some()
        || library_menu.is_some()
        || library_preview.is_some()
        || midi_monitor.is_some()
        || drop_monitor
        || sample_editor.is_some();
    // Before the first thread (the audio stream, the plugin hosts, the toolkit): on a Wayland
    // session the windows open on X11 through XWayland, so the pop-out windows can stay above
    // the main one; DAWSOME_WINDOW_SYSTEM=wayland keeps them on Wayland.
    #[cfg(target_os = "linux")]
    desktop::open_windows_on_chosen_system(scripted);
    // Before anything reads a per-user file; scripted runs and --play-to write none.
    if !scripted && args.play_to.is_none() {
        for (folder, error) in former_folders::carry_over(&former_folders::kept()) {
            eprintln!(
                "dawsome-app: {} could not be made from the earlier per-user folder: {error}",
                folder.display()
            );
        }
    }
    let plugins = plugin_setup(&args.plugins);
    if let (Some(request), Some(project)) = (&args.play_to, &args.project) {
        let window = extras.open_instrument.map(ChannelId);
        let live = play_to::Live {
            instrument_window: window,
            library_preview: library_preview.as_deref(),
        };
        return match play_to::run_with(project, request, plugins.as_ref(), live) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("dawsome-app: {error}");
                ExitCode::FAILURE
            }
        };
    }
    let mut host = Host::desktop(
        args.device_rate,
        scripted,
        plugins,
        args.stream_threshold,
        args.cache_dir.clone(),
    );
    host.browser_root = args.browser_root.clone();
    if let Some(names) = args.midi_ports.clone() {
        if midi_monitor.is_some() {
            eprintln!(
                "dawsome-app: --midi-monitor connects system ports; --midi-ports lists names \
                 that never connect\n\n{}",
                args::USAGE
            );
            return ExitCode::from(2);
        }
        host.midi_ports = midi::PortSource::Given(names);
    }
    if let Some(names) = args.midi_out_ports.clone() {
        host.midi_outputs = midi_out::MidiOutputs::Given(names);
    }
    // A run of its own that writes nothing (it counts as scripted) yet reaches the one system
    // port it names, never another.
    if let Some(port) = &midi_monitor {
        host.midi_ports = midi::PortSource::Only(port.clone());
        host.midi_monitor = true;
    }
    host.drop_monitor = drop_monitor;
    // The output device the settings choose (what they keep for it and the name it was shown
    // by), read before the window, which reads them again.
    let saved = settings::Settings::load(host.settings_file.as_deref());
    let chosen = saved
        .audio_output()
        .map(|id| (id.to_owned(), saved.audio_output_name().map(str::to_owned)));
    let chosen_output = chosen
        .as_ref()
        .map(|(id, name)| audio::Chosen::new(id, name.as_deref()));
    let (opened, document, load_notes) = match &args.project {
        Some(path) => match project_file::open_noting(path) {
            Ok((opened, notes)) => (opened, Document::saved_at(path), notes),
            Err(error) => {
                eprintln!("dawsome-app: {error}");
                return ExitCode::FAILURE;
            }
        },
        None => {
            // At the rate the output device plays at on its own, asked before its stream opens
            // (a 44.1 kHz document on a 48 kHz device was resampled on the
            // way out and refused the input at 48 kHz).
            let mut project = project_file::first_document(host.starter_kit.as_deref());
            project.sample_rate = project_file::new_document_rate(audio::device_own_rate(
                args.device_rate,
                chosen_output.map(|chosen| chosen.id),
            ));
            let empty = Opened {
                project,
                samples_dir: PathBuf::new(),
            };
            (empty, Document::untitled(), Vec::new())
        }
    };
    let Opened {
        project,
        samples_dir,
    } = opened;
    let sample_rate = project.sample_rate;
    if let Err(error) = starter_kit::install_for(&project, host.starter_kit.as_deref()) {
        eprintln!("dawsome-app: the starter kit was not written: {error}");
    }
    // A file whose playlist holds music opens on it, playing the song (a double-click on a
    // project in a file manager starts here); a scripted run opens as its flags say.
    let (landing, song) = app::landing_at_start(&project, scripted, args.song);
    let mode = Session::mode_for(&project, song);
    let opened = Session::open_with_library(
        project,
        samples_dir,
        mode,
        host.registry(),
        host.playback.clone(),
        app::added_folders(&host),
    );
    let (mut session, player) = match opened {
        Ok(opened) => opened,
        Err(error) => {
            eprintln!("dawsome-app: {error}");
            return ExitCode::FAILURE;
        }
    };
    let audio = if args.script.null_output {
        Ok(audio::Output::null(player, sample_rate))
    } else {
        audio::Output::start(player, sample_rate, args.device_rate, chosen_output)
    };
    let audio = audio.map_err(|failure| {
        eprintln!("dawsome-app: audio off: {}", failure.error);
        if let Some(player) = failure.player {
            session.park(*player);
        }
        failure.error
    });
    // Of the stream opened at start; a stream opened later by a retry is not counted.
    let callback_stats = args
        .stats
        .then(|| audio.as_ref().ok().map(audio::Output::stats));

    // Set by a headless script that could not do its job; read once the window has closed.
    let script_failed = Arc::new(AtomicBool::new(false));
    let failed = Arc::clone(&script_failed);
    // A run with nobody at the keyboard opens as its flags say (the defaults: the rack alone,
    // 1000 x 560), so its screenshots never depend on how the user's window last closed.
    let start_window = if scripted || layout_given {
        settings::StartWindow {
            panels: args.panels,
            size: args.window_size,
            maximized: false,
        }
    } else {
        let saved = settings::Settings::load(host.settings_file.as_deref()).window;
        settings::start_window(saved.as_ref())
    };
    let script = args.script;
    let dump_picker = script.dump_picker.clone();
    let (panels, window_size) = (start_window.panels, start_window.size);
    if args.stats {
        window_stats::count_frames();
    }
    let window_opened = std::time::Instant::now();
    // The session, the stream and the start cannot be cloned, so the boot closure hands them
    // over once.
    let opened = Cell::new(Some((session, audio, Start { document })));
    // Plugin windows carry the application id as their X11 class, as the app's own windows do,
    // so a desktop rule (one that sends `dawsome-test` windows to another screen without the
    // focus) treats them alike.
    #[cfg(target_os = "linux")]
    dawsome_host_common::window::set_class(&desktop::app_id());
    let main_window = window::Settings {
        maximized: start_window.maximized,
        // A close with unsaved changes asks first; the app closes itself once answered.
        exit_on_close_request: false,
        ..desktop::window_settings(start_window.size)
    };
    // A daemon, not an application, for the instrument window beside the main one: iced 0.14's
    // `application` hands every window one view, a daemon's `view` and `title` take the
    // window's id. A daemon opens no window by itself and never exits when its windows close:
    // the main window is opened here, and the app exits itself (`iced::exit`) when it closes.
    let result = iced::daemon(
        move || {
            let (session, audio, start) = opened.take().expect("iced boots the app once");
            // The main window opens first and the app starts once it is open, as iced's
            // `application` runs (iced_winit 0.14 `run`: `window::open(settings)` then the
            // boot task), so the start's tasks find it.
            let (id, open) = window::open(main_window.clone());
            let (mut app, task) = App::boot(
                session,
                audio,
                panels,
                window_size,
                song,
                script.clone(),
                Arc::clone(&failed),
                host.clone(),
                start,
            );
            app.set_main_window(id);
            if let Some(landing) = landing {
                app.land(landing);
            }
            app.show_load_notes(&load_notes);
            app.opened_maximized(start_window.maximized);
            app.shoot(shot_window.unwrap_or_default());
            let extra = app.start_extras(&extras, dump_picker.as_deref());
            if let Some(sfz) = &library_preview {
                app.preview_set_at_start(sfz);
            }
            if let Some(shown) = shown {
                app.show_at_start(shown);
            }
            if let Some(page) = settings_page {
                app.show_settings_page(page);
            }
            if let Some(on) = callouts {
                app.set_callouts(on);
            }
            if scroll_to_end {
                app.scroll_to_end();
            }
            if let Some(channel) = drag_over {
                app.drag_over_header(ChannelId(channel));
            }
            if let Some(channel) = channel_menu {
                app.open_channel_menu(ChannelId(channel));
            }
            let params = match script.open_params {
                Some(owner) => app.open_params_at_start(owner),
                None => Task::none(),
            };
            let editor = match sample_editor {
                Some(editor) => app.open_sample_editor_at_start(editor.on, editor.select),
                None => Task::none(),
            };
            if let Some(clip) = select_clip {
                let clip = dawsome_engine::project::ClipId(clip);
                if !app.select_clip(clip) {
                    eprintln!(
                        "dawsome-app: --select-clip: clip {} is not in the playlist",
                        clip.0
                    );
                }
            }
            if let Some(automation) = curve_menu {
                let automation = dawsome_engine::project::AutomationId(automation);
                if !app.open_curve_menu(automation) {
                    eprintln!(
                        "dawsome-app: --curve-menu: automation {} has no segment handle in view",
                        automation.0
                    );
                }
            }
            if let Some((insert, slot)) = slot_menu {
                let insert = dawsome_engine::project::InsertId(insert);
                if !app.open_slot_menu(insert, slot) {
                    eprintln!(
                        "dawsome-app: --slot-menu: insert {} has no slot {slot}",
                        insert.0
                    );
                }
            }
            if let Some(port) = &library_menu {
                if !app.open_library_menu(port) {
                    eprintln!(
                        "dawsome-app: --library-menu: {port} is no listed device a channel plays"
                    );
                }
            }
            // The pop-out windows --open-instrument, --panels settings, --show help,
            // --open-params and --open-sample-editor opened, after the main window.
            let instrument = app.sync_pop_outs();
            let mut start = Some(Task::batch([task, extra, params, editor, instrument]));
            let start = open.then(move |_| start.take().unwrap_or_else(Task::none));
            (app, start)
        },
        App::update,
        App::view_window,
    )
    .title(App::window_title)
    .theme(iced::Theme::Dark)
    .subscription(App::subscription)
    .executor::<executor::Executor>()
    .run();

    match callback_stats {
        Some(Some(stats)) => println!("{stats}"),
        Some(None) => println!("audio callbacks: no stream opened at start"),
        None => {}
    }
    if args.stats {
        println!("{}", window_stats::Summary::now(window_opened.elapsed()));
        let renderer = window_stats::renderer().unwrap_or("none, no frame was drawn");
        println!("renderer: {renderer}");
    }
    match result {
        Ok(()) if script_failed.load(Ordering::Relaxed) => ExitCode::FAILURE,
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("dawsome-app: {error}");
            ExitCode::FAILURE
        }
    }
}
