//! Export: a dialog for what plays (the pattern on screen or the song), the bars and the
//! format, the desktop's save dialog, then an offline
//! render on a worker thread through the `--play-to` path (its own session, compiler, rings and
//! player), so the live stream plays on untouched. Its plugins are its own instances, made and
//! served on the window's thread, their main thread as the window's own instances' (the worker
//! plays them: [`play_to::RenderHosts`]). Progress shows in the dialog; Cancel stops the render
//! and removes the partial file.
//!
//! A 16-bit file is dithered unless the dialog's Dither is set Off (TPDF by default, as
//! `dawsome render` writes; 24-bit and 32-bit float never are).
//!
//! Stems go the same way: the dialog asks the bit depth alone (the song plays, its whole bars),
//! the desktop's folder dialog where, then each playlist track's file and the master's render in
//! turn into `<project> stems/` (`dawsome_media::export::stems`), the progress counting all of
//! them; Cancel removes every file the export wrote. The export ends on its report.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use dawsome_engine::plan;
use dawsome_engine::project::Project;
use dawsome_engine::time;
use dawsome_media::export::{self, Report, Stems};
use dawsome_media::{BitDepth, Dither, Encoding, MP3_RATES};
use iced::Task;

use super::files::{pick_folder, pick_save, with_extension, FileMessage, Modal};
use super::{App, Message, PlayMode};
use crate::play_to::{self, MainThread, Render, RenderHosts, RenderOptions};
use crate::ui::dialog::ExportForm;

/// How often a running export's progress is read.
pub const PROGRESS_PERIOD: Duration = Duration::from_millis(100);

/// What the File menu exports to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportKind {
    Wav,
    Mp3,
    /// A WAV file per playlist track, and the master's.
    Stems,
}

#[derive(Debug, Clone)]
pub enum ExportMessage {
    /// What plays: the pattern on screen or the song.
    Plays(PlayMode),
    Bars(String),
    Depth(BitDepth),
    /// Whether a 16-bit file is dithered.
    Dither(Dither),
    /// Export…: ask where.
    Start,
    /// The save dialog's answer.
    Picked(Option<PathBuf>),
    Cancel,
    /// Time to read the progress.
    Poll,
}

/// The export dialog's settings and, once started, the render.
pub(super) struct ExportState {
    kind: ExportKind,
    /// What plays; the toolbar's mode when the dialog opened.
    plays: PlayMode,
    bars: String,
    depth: BitDepth,
    /// On (TPDF) when the dialog opens, every time: the setting is the export's, not kept.
    dither: Dither,
    error: Option<String>,
    running: Option<Running>,
}

/// A render on its worker thread.
struct Running {
    /// The file it writes; a stems export's folder.
    out: PathBuf,
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    /// How it ended, or why it failed.
    finished: mpsc::Receiver<Result<Finished, String>>,
    /// The failures this thread hears from the export's plugins, when it serves them.
    heard: Option<Rc<RefCell<Vec<String>>>>,
}

/// How a render on its worker ended.
struct Finished {
    /// `false` when Cancel stopped it.
    done: bool,
    /// The plugin failures it reported, each once.
    failures: Vec<String>,
    /// A stems export's report, shown when it is done.
    report: Option<Report>,
}

/// Why MP3 export is not offered for a project at `rate`.
pub fn mp3_unavailable(rate: u32) -> Option<&'static str> {
    (!MP3_RATES.contains(&rate)).then_some("needs 32, 44.1 or 48 kHz")
}

/// Bars an export renders unless changed: the song to its end, or the pattern's length.
fn default_bars(project: &Project, mode: plan::PlayMode) -> u32 {
    match mode {
        plan::PlayMode::Song => {
            let bar = time::bars_to_ticks(1, project.ppq).0;
            let end = project.playlist.end().0.max(0);
            u32::try_from((end + bar - 1) / bar)
                .unwrap_or(u32::MAX)
                .max(1)
        }
        plan::PlayMode::Pattern(id) => project
            .patterns
            .get(&id)
            .map_or(1, |pattern| pattern.length_bars.max(1)),
        plan::PlayMode::Nothing => 1,
    }
}

impl App {
    /// The document's name without its extension, which an export's file is named after.
    pub(super) fn document_stem(&self) -> String {
        Path::new(&self.document.name).file_stem().map_or_else(
            || "Untitled".to_owned(),
            |stem| stem.to_string_lossy().into_owned(),
        )
    }

    /// What an export choosing `plays` plays: the pattern on screen, or the song (also when no
    /// pattern is on screen).
    fn export_mode(&self, plays: PlayMode) -> plan::PlayMode {
        match (plays, self.pattern) {
            (PlayMode::Pattern, Some(pattern)) => plan::PlayMode::Pattern(pattern),
            _ => plan::PlayMode::Song,
        }
    }

    /// What the dialog chooses first: what the toolbar's mode plays.
    fn first_plays(&self) -> PlayMode {
        match self.export_mode(self.mode) {
            plan::PlayMode::Pattern(_) => PlayMode::Pattern,
            plan::PlayMode::Song | plan::PlayMode::Nothing => PlayMode::Song,
        }
    }

    pub(super) fn open_export(&mut self, kind: ExportKind) {
        let project = self.session.project();
        if kind == ExportKind::Mp3 {
            if let Some(reason) = mp3_unavailable(project.sample_rate) {
                self.notice = Some(format!("MP3 export {reason}"));
                return;
            }
        }
        if kind == ExportKind::Stems && project.playlist.clips.is_empty() {
            self.notice = Some("Export stems needs a song: the playlist has no clip".to_owned());
            return;
        }
        // Stems play the song, whatever the toolbar plays.
        let plays = match kind {
            ExportKind::Stems => PlayMode::Song,
            ExportKind::Wav | ExportKind::Mp3 => self.first_plays(),
        };
        let bars = default_bars(project, self.export_mode(plays));
        self.files.modal = Some(Modal::Export(ExportState {
            kind,
            plays,
            bars: bars.to_string(),
            depth: BitDepth::Int24,
            dither: Dither::Tpdf,
            error: None,
            running: None,
        }));
    }

    /// The dialog's view data, while the export dialog is up.
    pub(super) fn export_form(&self) -> Option<ExportForm<'_>> {
        let Some(Modal::Export(state)) = &self.files.modal else {
            return None;
        };
        let project = self.session.project();
        let pattern = self
            .pattern
            .and_then(|id| project.patterns.get(&id))
            .map(|pattern| pattern.name.as_str());
        let progress = state.running.as_ref().map(|running| {
            let total = running.total.load(Ordering::Relaxed).max(1);
            running.done.load(Ordering::Relaxed) as f32 / total as f32
        });
        Some(ExportForm {
            mp3: state.kind == ExportKind::Mp3,
            stems: (state.kind == ExportKind::Stems).then_some(project.playlist.tracks.len()),
            bars: &state.bars,
            depth: state.depth,
            dither: state.dither,
            rate: project.sample_rate,
            plays: state.plays,
            pattern,
            progress,
            error: state.error.as_deref(),
        })
    }

    /// Whether an export is rendering, so the progress timer runs.
    pub(super) fn exporting(&self) -> bool {
        matches!(&self.files.modal, Some(Modal::Export(state)) if state.running.is_some())
    }

    pub(super) fn export_message(&mut self, message: ExportMessage) -> Task<Message> {
        if let ExportMessage::Plays(plays) = message {
            self.choose_export_plays(plays);
            return Task::none();
        }
        let Some(Modal::Export(state)) = &mut self.files.modal else {
            return Task::none();
        };
        match message {
            ExportMessage::Plays(_) => {}
            ExportMessage::Bars(text) => state.bars = text,
            ExportMessage::Depth(depth) => state.depth = depth,
            ExportMessage::Dither(dither) => state.dither = dither,
            ExportMessage::Start => {
                if state.running.is_some() {
                    return Task::none();
                }
                if bars(&state.bars).is_none() {
                    state.error = Some("Bars must be a whole number from 1".into());
                    return Task::none();
                }
                state.error = None;
                if !self.host.background {
                    return Task::none();
                }
                let (title, extension, filter) = match state.kind {
                    ExportKind::Wav => ("Export WAV", "wav", ("WAV audio (*.wav)", "wav")),
                    ExportKind::Mp3 => ("Export MP3", "mp3", ("MP3 audio (*.mp3)", "mp3")),
                    ExportKind::Stems => {
                        let dialog = pick_folder("Export stems: the folder", self.document_dir());
                        return Task::perform(dialog, |path| {
                            Message::File(FileMessage::ExportForm(ExportMessage::Picked(path)))
                        });
                    }
                };
                let stem = self.document_stem();
                let dialog = pick_save(
                    title,
                    self.document_dir(),
                    format!("{stem}.{extension}"),
                    filter,
                );
                return Task::perform(dialog, |path| {
                    Message::File(FileMessage::ExportForm(ExportMessage::Picked(path)))
                });
            }
            ExportMessage::Picked(None) => {}
            ExportMessage::Picked(Some(path)) if state.kind == ExportKind::Stems => {
                self.start_stems_export(&path);
            }
            ExportMessage::Picked(Some(path)) => self.start_export(&path),
            ExportMessage::Cancel => match &state.running {
                // The worker stops at its next block and removes the partial file.
                Some(running) => running.cancel.store(true, Ordering::Relaxed),
                None => self.files.modal = None,
            },
            ExportMessage::Poll => {
                // Under the dialog the plugin tick does not run: its poll serves the export's
                // plugins.
                self.serve_render_hosts();
                self.poll_export();
            }
        }
        Task::none()
    }

    /// The dialog plays `plays` from now on; bars still at the last choice's length follow the
    /// new one's, bars typed stay.
    fn choose_export_plays(&mut self, plays: PlayMode) {
        let Some(Modal::Export(state)) = &self.files.modal else {
            return;
        };
        if state.running.is_some() || state.plays == plays || state.kind == ExportKind::Stems {
            return;
        }
        let project = self.session.project();
        let was = default_bars(project, self.export_mode(state.plays)).to_string();
        let now = default_bars(project, self.export_mode(plays)).to_string();
        if let Some(Modal::Export(state)) = &mut self.files.modal {
            if state.bars.trim() == was {
                state.bars = now;
            }
            state.plays = plays;
        }
    }

    /// Renders to `path` on a worker thread (or here, when nothing runs in the background).
    fn start_export(&mut self, path: &Path) {
        // The document as it is, tweaks to states too big for an undo step included.
        self.capture_big_states();
        let Some(Modal::Export(state)) = &self.files.modal else {
            return;
        };
        let mode = self.export_mode(state.plays);
        // The document's own file: an input the export may not overwrite.
        let document = self.document.path.clone();
        let project = self.session.project().clone();
        let samples_dir = self.session.samples_dir().to_path_buf();
        let background = self.host.background;
        let plugins = self.host.plugins.clone();
        let cache_dir = self.host.playback.stretch_dir.clone();
        let library = self.browse.model.user_folders().folders;
        let (main_thread, heard) = self.render_hosts_for(&project).unzip();
        let Some(Modal::Export(state)) = &mut self.files.modal else {
            return;
        };
        let Some(bars) = bars(&state.bars) else {
            return;
        };
        let (extension, encoding) = match state.kind {
            ExportKind::Wav | ExportKind::Stems => {
                ("wav", Encoding::Wav(state.depth, state.dither))
            }
            ExportKind::Mp3 => ("mp3", Encoding::Mp3),
        };
        let render = Render {
            out: with_extension(path, extension),
            bars,
            mode,
            encoding,
        };
        let (done, total, cancel) = (
            Arc::new(AtomicU64::new(0)),
            Arc::new(AtomicU64::new(0)),
            Arc::new(AtomicBool::new(false)),
        );
        let (sender, finished) = mpsc::channel();
        state.running = Some(Running {
            out: render.out.clone(),
            done: Arc::clone(&done),
            total: Arc::clone(&total),
            cancel: Arc::clone(&cancel),
            finished,
            heard,
        });
        let work = move || {
            let mut progress = |frames: u64, of: u64| {
                done.store(frames, Ordering::Relaxed);
                total.store(of, Ordering::Relaxed);
                !cancel.load(Ordering::Relaxed)
            };
            // In the background the export's plugins are the window's thread's, which this
            // worker plays; with nothing in the background this is the window's thread, where
            // the export makes its own hosts, and the render takes a thread of its own.
            let options = play_to::RenderOptions {
                document: document.as_deref(),
                plugins: plugins.as_ref(),
                cache_dir,
                library,
                main_thread: main_thread.as_ref(),
                ..play_to::RenderOptions::default()
            };
            let result =
                play_to::render_session(project, samples_dir, &render, &mut progress, &options)
                    .map(|outcome| Finished {
                        done: outcome.finished,
                        failures: outcome.failures,
                        report: None,
                    })
                    .map_err(|error| error.to_string());
            let _ = sender.send(result);
        };
        if background {
            std::thread::spawn(work);
        } else {
            work();
            self.poll_export();
        }
    }

    /// Reads a finished export's result: done closes the dialog with a notice (naming any plugin
    /// that failed on the way), a failure shows in it.
    fn poll_export(&mut self) {
        let Some(Modal::Export(state)) = &mut self.files.modal else {
            return;
        };
        let Some(running) = &state.running else {
            return;
        };
        let result = match running.finished.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("the export stopped before it finished".to_owned())
            }
        };
        let out = running.out.clone();
        let heard = running.heard.clone();
        state.running = None;
        // A failure its plugins raised before the worker answered is heard now.
        if heard.is_some() {
            self.serve_render_hosts();
        }
        let heard = heard.map(|heard| heard.take()).unwrap_or_default();
        let with_heard = |mut failures: Vec<String>| {
            for failure in &heard {
                if !failures.contains(failure) {
                    failures.push(failure.clone());
                }
            }
            failures
        };
        match result {
            Ok(Finished {
                done: true,
                report: Some(mut report),
                ..
            }) => {
                report.lines = with_heard(report.lines);
                self.files.modal = None;
                self.show_export_report(&report);
            }
            Ok(Finished {
                done: true,
                failures,
                report: None,
            }) => {
                self.files.modal = None;
                let exported = format!("Exported {}", out.display());
                let notice: Vec<String> = std::iter::once(exported)
                    .chain(with_heard(failures))
                    .collect();
                self.notice = Some(notice.join("; "));
            }
            Ok(Finished { done: false, .. }) => self.files.modal = None,
            Err(error) => {
                if let Some(Modal::Export(state)) = &mut self.files.modal {
                    state.error = Some(error);
                }
            }
        }
    }

    /// The hosts an export of `project` plays its plugins in, made here when the export runs in
    /// the background and the project names plugins, and kept in `render_hosts` until the export
    /// and its instances are done: the worker's way to them, and the failures they report. `None`
    /// with nothing in the background, where the export runs here and makes its own hosts here.
    fn render_hosts_for(
        &mut self,
        project: &Project,
    ) -> Option<(MainThread, Rc<RefCell<Vec<String>>>)> {
        if !self.host.background || !play_to::names_plugins(project) {
            return None;
        }
        let (hosts, main) = RenderHosts::new(project, &self.host.catalogue());
        let heard = hosts.failures();
        self.render_hosts.push(hosts);
        Some((main, heard))
    }

    /// The exports' plugins' work here (what their workers ask of them, their timers, file
    /// descriptors and callbacks); the hosts whose export is done go.
    pub(super) fn serve_render_hosts(&mut self) {
        for hosts in &mut self.render_hosts {
            hosts.serve();
        }
        self.render_hosts.retain(|hosts| !hosts.is_spent());
    }

    /// Renders the stems into `<project> stems/` inside `dir`, on a worker thread (or here,
    /// when nothing runs in the background), as [`App::start_export`] renders one file.
    fn start_stems_export(&mut self, dir: &Path) {
        self.capture_big_states();
        let project = self.session.project().clone();
        let Some(stems) = export::stems(&project) else {
            return;
        };
        let folder = export::stems_folder(dir, &self.document_stem());
        let lines = export::stems_lines(&project);
        let document = self.document.path.clone();
        let samples_dir = self.session.samples_dir().to_path_buf();
        let background = self.host.background;
        let plugins = self.host.plugins.clone();
        let cache_dir = self.host.playback.stretch_dir.clone();
        let library = self.browse.model.user_folders().folders;
        let (main_thread, heard) = self.render_hosts_for(&project).unzip();
        let Some(Modal::Export(state)) = &mut self.files.modal else {
            return;
        };
        let encoding = Encoding::Wav(state.depth, state.dither);
        let (done, total, cancel) = (
            Arc::new(AtomicU64::new(0)),
            Arc::new(AtomicU64::new(0)),
            Arc::new(AtomicBool::new(false)),
        );
        let (sender, finished) = mpsc::channel();
        state.running = Some(Running {
            out: folder.clone(),
            done: Arc::clone(&done),
            total: Arc::clone(&total),
            cancel: Arc::clone(&cancel),
            finished,
            heard,
        });
        let work = move || {
            let mut progress = |frames: u64, of: u64| {
                done.store(frames, Ordering::Relaxed);
                total.store(of, Ordering::Relaxed);
                !cancel.load(Ordering::Relaxed)
            };
            let options = RenderOptions {
                document: document.as_deref(),
                plugins: plugins.as_ref(),
                cache_dir,
                library,
                main_thread: main_thread.as_ref(),
                ..RenderOptions::default()
            };
            let result = render_stems(
                stems,
                &folder,
                &samples_dir,
                encoding,
                &options,
                &mut progress,
            )
            .map(|(done, failures)| {
                let mut lines = lines;
                lines.extend(failures.iter().cloned());
                Finished {
                    done,
                    failures,
                    report: Some(Report {
                        exported: folder.clone(),
                        lines,
                    }),
                }
            });
            let _ = sender.send(result);
        };
        if background {
            std::thread::spawn(work);
        } else {
            work();
            self.poll_export();
        }
    }

    /// The window is closing: a running export stops and its partial file goes now.
    pub(super) fn abandon_export(&mut self) {
        if let Some(Modal::Export(ExportState {
            running: Some(running),
            ..
        })) = &self.files.modal
        {
            running.cancel.store(true, Ordering::Relaxed);
            let _ = std::fs::remove_file(&running.out);
        }
    }
}

/// Renders each file of `stems` into `folder` in turn (sample paths relative to
/// `samples_dir`), calling `progress` with the frames done of all of them; returns whether it
/// finished, and the plugin failures, each once. A Cancel or a failure removes every file it
/// wrote, and the folder when nothing else is in it.
fn render_stems(
    stems: Stems,
    folder: &Path,
    samples_dir: &Path,
    encoding: Encoding,
    options: &RenderOptions,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<(bool, Vec<String>), String> {
    fs::create_dir_all(folder)
        .map_err(|error| format!("cannot make {}: {error}", folder.display()))?;
    let count = stems.files.len() as u64;
    let mut written = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    let give_up = |written: &[PathBuf]| {
        for path in written {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_dir(folder);
    };
    for (index, file) in (0u64..).zip(stems.files) {
        let render = Render {
            out: folder.join(&file.name),
            bars: stems.bars,
            mode: plan::PlayMode::Song,
            encoding,
        };
        // Every file is as long as the next, so the frames done of all of them are this many.
        let mut each = |done: u64, of: u64| progress(index * of + done, count * of);
        let rendered = play_to::render_session(
            file.project,
            samples_dir.to_path_buf(),
            &render,
            &mut each,
            options,
        );
        match rendered {
            Ok(outcome) if outcome.finished => {
                written.push(render.out);
                for failure in outcome.failures {
                    if !failures.contains(&failure) {
                        failures.push(failure);
                    }
                }
            }
            Ok(_) => {
                give_up(&written);
                return Ok((false, failures));
            }
            Err(error) => {
                give_up(&written);
                return Err(error.to_string());
            }
        }
    }
    Ok((true, failures))
}

/// The bars field as a count of at least 1.
fn bars(text: &str) -> Option<u32> {
    text.trim().parse().ok().filter(|&bars| bars >= 1)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;
    use std::time::Instant;

    use dawsome_engine::project::Project;
    use dawsome_engine::realtime::Player;
    use dawsome_host_clap::catalogue::PluginFormat;
    use dawsome_host_clap::{scan, Catalogue, ClapHost, PluginSetup};
    use dawsome_host_vst3::Vst3Host;

    use super::*;
    use crate::app::plugins::app_tests::{
        app_hosted, catalogue_of, test_plugin_file, variants_file,
    };
    use crate::app::PLUGIN_PERIOD;
    use crate::document::Host;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    /// Set in the child [`in_temp_home`] starts.
    const CHILD: &str = "DAWSOME_EXPORT_TEST_CHILD";

    /// Whether this is the child that does `test`'s work (`test` its path, `dir` its folder). In
    /// the test binary's own run it is not: `test` runs again in a child of this binary, alone,
    /// with HOME and the XDG folders under `dir`, and must pass there. Why: Surge XT reads and
    /// makes user folders (`~/Documents/Surge XT`), and its JUCE keeps one message thread per
    /// process, which a process of the test's own then decides alone (as host-vst3's real-plugin
    /// tests run).
    fn in_temp_home(test: &str, dir: &Path) -> bool {
        if std::env::var_os(CHILD).is_some() {
            return true;
        }
        let _ = fs::remove_dir_all(dir);
        fs::create_dir_all(dir).expect("temp dir");
        let output = Command::new(std::env::current_exe().expect("the test binary"))
            .args([test, "--exact", "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .env_remove("CLAP_PATH")
            .env_remove("VST3_PATH")
            .env("HOME", dir.join("home"))
            .env("XDG_CONFIG_HOME", dir.join("home/config"))
            .env("XDG_DATA_HOME", dir.join("home/data"))
            .env("XDG_CACHE_HOME", dir.join("home/cache"))
            .env("XDG_STATE_HOME", dir.join("home/state"))
            .output()
            .expect("run the test again");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{test} failed in its child ({}):\n{stdout}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            stdout.contains("1 passed"),
            "{test} did not run in its child:\n{stdout}"
        );
        false
    }

    /// The bundled plugin's folder, when `scripts/fetch-surge.sh` has fetched it: `<target
    /// dir>/surge/linux-x86_64`, the target dir found from this test binary's own place,
    /// `<target dir>/debug/deps/<binary>`.
    fn surge_xt() -> Option<PathBuf> {
        let exe = std::env::current_exe().expect("the test binary has a path");
        let dir = exe.ancestors().nth(3)?.join("surge/linux-x86_64");
        dir.join("Surge XT.clap").is_file().then_some(dir)
    }

    /// The command line, beside this package's binary, where a workspace build puts it.
    fn cli_binary() -> PathBuf {
        let exe = std::env::current_exe().expect("the test binary has a path");
        let path = exe
            .ancestors()
            .nth(2)
            .expect("inside the target dir")
            .join(format!("dawsome{}", std::env::consts::EXE_SUFFIX));
        assert!(
            path.is_file(),
            "{} is missing: run the workspace tests (scripts/check.sh) so the command line is built",
            path.display()
        );
        path
    }

    /// A song of 16 bars at 48 kHz in `dir`, with what the frozen export played: a sampler (the
    /// impulse on every beat), an instrument set (the pad), a take's audio clip on track 2 from
    /// bar 2, and Surge XT, muted: it runs, unheard (its own sound differs from run to run).
    fn juce_song(dir: &Path) -> PathBuf {
        let fixtures = Path::new(FIXTURES);
        fs::create_dir_all(dir.join("samples")).expect("dirs");
        fs::create_dir_all(dir.join("instruments/pad")).expect("dirs");
        fs::copy(fixtures.join("impulse.wav"), dir.join("impulse.wav")).expect("copies");
        fs::copy(
            fixtures.join("hold2000.wav"),
            dir.join("samples/song take 1.wav"),
        )
        .expect("copies");
        for file in ["pad.sfz", "ramp.wav", "set.json"] {
            fs::copy(
                fixtures.join("instruments/pad").join(file),
                dir.join("instruments/pad").join(file),
            )
            .expect("copies");
        }
        let step = |channel: u32, tick: i64| {
            serde_json::json!({"channel": channel, "tick": tick, "length": 0,
                "velocity": 1.0})
        };
        let note = |channel: u32, tick: i64, length: i64, key: u8| {
            serde_json::json!({"channel": channel, "tick": tick, "length": length,
                "velocity": 0.8, "key": key})
        };
        let mut clips: Vec<serde_json::Value> = (0..16)
            .map(|bar| {
                serde_json::json!({"id": bar + 1, "track": 1, "start": bar * 384, "length": 384,
                    "kind": "pattern", "pattern": 1})
            })
            .collect();
        let take_clip = serde_json::json!({"id": 17, "track": 2, "start": 384, "length": 384,
            "kind": "audio", "channel": 3});
        clips.push(take_clip);
        let channel = |id: u32, name: &str, kind: &str| {
            serde_json::json!({"id": id, "name": name, "kind": kind, "volume": 0.5625,
                "pan": 0.0, "mute": false})
        };
        let mut sampler = channel(1, "Kick", "sampler");
        sampler["sample"] = "impulse.wav".into();
        let mut pad = channel(2, "Pad", "instrument");
        pad["instrument"] = "instruments/pad/pad.sfz".into();
        let mut take = channel(3, "Take 1", "audio");
        take["sample"] = "samples/song take 1.wav".into();
        let mut surge = channel(4, "Surge XT", "plugin");
        surge["processor"] =
            serde_json::json!({"id": "clap:org.surge-synth-team.surge-xt", "name": "Surge XT"});
        surge["mute"] = true.into();
        let project = serde_json::json!({
            "format_version": 4, "sample_rate": 48000, "tempo_mbpm": 120000, "ppq": 96,
            "next_channel_id": 5,
            "channels": [sampler, pad, take, surge],
            "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": [
                step(1, 0), note(2, 0, 96, 60), note(4, 0, 192, 60), step(1, 96), step(1, 192),
                note(4, 192, 96, 64), step(1, 288)]}],
            "playlist": {"next_track_id": 3, "next_clip_id": 18,
                "tracks": [{"id": 1, "name": "Track 1", "mute": false},
                    {"id": 2, "name": "Track 2", "mute": false}],
                "clips": clips},
        });
        let path = dir.join("song.json");
        fs::write(&path, project.to_string()).expect("writes");
        path
    }

    // Build d5f9f02: Export WAV of a song with Surge XT stopped at 23 % and never finished, and
    // Export stems the same. The window's own Surge XT had made the window's thread the
    // plugin's message thread (JUCE keeps one per process); the export's instance,
    // made and pumped on the export's worker, waited in its timer for that thread's lock, which
    // nothing there gives. Through the dialog as the window runs it (work in the background, the
    // document's own Surge XT made on this thread, the plugin tick whenever `plugins_due` says
    // and the progress poll, as the window's subscriptions send them), the export finishes, its
    // dialog closes on "Exported", and its file is the command line's render of the project
    // (`dawsome render --song --bars 16 --bits 24`) byte for byte: 16 bars of 96000 frames.
    #[test]
    fn an_export_beside_the_windows_own_juce_plugin_finishes() {
        let Some(surge) = surge_xt() else {
            println!("skipped: Surge XT is not fetched (scripts/fetch-surge.sh)");
            return;
        };
        let dir = std::env::temp_dir().join("dawsome-app-export-juce");
        let test = "app::export::tests::an_export_beside_the_windows_own_juce_plugin_finishes";
        if !in_temp_home(test, &dir) {
            return;
        }
        let work = dir.join("work");
        let project = juce_song(&work);
        let cli_wav = work.join("cli.wav");
        let rendered = Command::new(cli_binary())
            .arg("render")
            .arg(&project)
            .arg(&cli_wav)
            .args(["--song", "--bars", "16", "--bits", "24", "--plugin-path"])
            .arg(&surge)
            .arg("--cache")
            .arg(work.join("cli-catalogue.json"))
            .output()
            .expect("the command line starts");
        assert!(
            rendered.status.success(),
            "{}",
            String::from_utf8_lossy(&rendered.stderr)
        );
        let catalogue = catalogue_of(surge.join("Surge XT.clap"));
        let cache = work.join("catalogue.json");
        catalogue.save(&cache).expect("saves");
        let host = Host {
            background: true,
            clap: ClapHost::new(catalogue.clone()),
            vst3: Vst3Host::new(catalogue),
            plugins: Some(PluginSetup {
                dirs: vec![surge.clone()],
                cache,
                child: scan::ChildCommand {
                    program: PathBuf::from("false"),
                    args: Vec::new(),
                },
                formats: PluginFormat::CLAP,
            }),
            ..Host::offline()
        };
        let (mut app, _player) = app_hosted(&project, host);
        let export = |message| Message::File(FileMessage::ExportForm(message));
        let _ = app.update(Message::File(FileMessage::Export(ExportKind::Wav)));
        let _ = app.update(export(ExportMessage::Plays(PlayMode::Song)));
        let form = app.export_form().expect("the dialog is up");
        assert_eq!(
            (form.bars, form.depth, form.dither),
            ("16", BitDepth::Int24, Dither::Tpdf)
        );
        let _ = app.update(export(ExportMessage::Picked(Some(work.join("song")))));
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut polled = Instant::now();
        while let Some(form) = app.export_form() {
            assert!(
                Instant::now() < deadline,
                "the export never finished: at {:?} for a minute",
                form.progress
            );
            std::thread::sleep(PLUGIN_PERIOD);
            if app.plugins_due() {
                let _ = app.update(Message::PluginTick);
            }
            if polled.elapsed() >= PROGRESS_PERIOD {
                polled = Instant::now();
                let _ = app.update(export(ExportMessage::Poll));
            }
        }
        let notice = app.notice.clone().unwrap_or_default();
        assert!(notice.starts_with("Exported "), "{notice}");
        let exported = work.join("song.wav");
        let reader = hound::WavReader::open(&exported).expect("exported");
        assert_eq!(reader.duration(), 16 * 96_000);
        assert!(
            fs::read(&exported).ok() == fs::read(&cli_wav).ok(),
            "the export and the command line's render differ"
        );
    }

    // Cancel during the second stem: the first, already written, goes too, and so does the
    // folder the export made; the export reports it did not finish.
    #[test]
    fn a_canceled_stems_export_leaves_nothing_behind() {
        let fixtures = Path::new(FIXTURES);
        let text = fs::read_to_string(fixtures.join("p18_stems.json")).expect("fixture");
        let project = Project::load_json(&text).expect("loads");
        let stems = export::stems(&project).expect("a song");
        let dir =
            std::env::temp_dir().join(format!("dawsome-app-stems-cancel-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let folder = dir.join("p18_stems stems");
        let mut calls = Vec::new();
        let finished = render_stems(
            stems,
            &folder,
            fixtures,
            Encoding::Wav(BitDepth::Float32, Dither::Off),
            &RenderOptions::default(),
            &mut |done, of| {
                calls.push((done, of));
                // Stop once the second file has started: past one file's frames.
                done <= of / 9
            },
        )
        .expect("no failure");
        assert_eq!(finished, (false, Vec::new()));
        assert!(calls.iter().any(|&(done, _)| done > 0), "it rendered");
        assert!(!folder.exists(), "the folder and its files are gone");
        let _ = fs::remove_dir_all(&dir);
    }

    /// An app on `path` as the window runs it (work in the background), hosting `catalogue`'s
    /// plugins.
    fn background_app(path: &Path, catalogue: Catalogue) -> (App, Player) {
        let host = Host {
            background: true,
            clap: ClapHost::new(catalogue.clone()),
            vst3: Vst3Host::new(catalogue),
            ..Host::offline()
        };
        app_hosted(path, host)
    }

    /// The window's subscriptions while the export dialog is up: the plugin tick whenever
    /// `plugins_due` says, the export's poll every [`PROGRESS_PERIOD`], until the dialog closes;
    /// `stop` (given the progress) may send Cancel. A minute at most.
    fn run_dialog(app: &mut App, mut stop: impl FnMut(f32) -> bool) {
        let export = |message| Message::File(FileMessage::ExportForm(message));
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut polled = Instant::now();
        while let Some(form) = app.export_form() {
            let progress = form.progress.unwrap_or(0.0);
            assert!(
                Instant::now() < deadline,
                "the export never finished: at {progress}"
            );
            if stop(progress) {
                let _ = app.update(export(ExportMessage::Cancel));
            }
            std::thread::sleep(PLUGIN_PERIOD);
            if app.plugins_due() {
                let _ = app.update(Message::PluginTick);
            }
            if polled.elapsed() >= PROGRESS_PERIOD {
                polled = Instant::now();
                let _ = app.update(export(ExportMessage::Poll));
            }
        }
    }

    /// The plugin tick as the window sends it, until the exports' hosts are gone (their
    /// instances with them): ten seconds at most.
    fn tick_until_render_hosts_go(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !app.render_hosts.is_empty() {
            assert!(
                Instant::now() < deadline,
                "the export's plugin hosts stay with no dialog up: due {}",
                app.plugins_due()
            );
            std::thread::sleep(PLUGIN_PERIOD);
            if app.plugins_due() {
                let _ = app.update(Message::PluginTick);
            }
        }
    }

    /// The left samples of the float WAV at `path`.
    fn left_of(path: &Path) -> Vec<f32> {
        let mut reader = hound::WavReader::open(path).expect("exported");
        reader
            .samples::<f32>()
            .step_by(2)
            .collect::<Result<_, _>>()
            .expect("samples")
    }

    // gen_p7_expected.py's known answer 1 through the window's export in the background: the
    // test sine, its instance made and served on this thread and played on the worker, sounds
    // on frame 6000 at its state's gain of 0.5 (5999 and 30000 silent), one bar of 96000
    // frames; the export's hosts go once it is done.
    #[test]
    fn a_background_export_plays_its_plugin_from_the_windows_thread() {
        let catalogue = catalogue_of(test_plugin_file());
        let (mut app, _player) =
            background_app(&Path::new(FIXTURES).join("p7_sine.json"), catalogue);
        let dir =
            std::env::temp_dir().join(format!("dawsome-app-export-bg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let export = |message| Message::File(FileMessage::ExportForm(message));
        let _ = app.update(Message::File(FileMessage::Export(ExportKind::Wav)));
        let _ = app.update(export(ExportMessage::Depth(BitDepth::Float32)));
        let _ = app.update(export(ExportMessage::Picked(Some(dir.join("sine")))));
        assert_eq!(app.render_hosts.len(), 1, "its hosts are this thread's");
        run_dialog(&mut app, |_| false);
        let notice = app.notice.clone().unwrap_or_default();
        assert!(notice.starts_with("Exported "), "{notice}");
        let left = left_of(&dir.join("sine.wav"));
        assert_eq!(left.len(), 96_000);
        assert_eq!((left[5999], left[6000], left[30_000]), (0.0, 0.5, 0.0));
        tick_until_render_hosts_go(&mut app);
        let _ = fs::remove_dir_all(&dir);
    }

    // In the background a plugin that fails part way (the fail variant, from its fifth block
    // on) is named once beside "Exported": its failure is heard on this thread, where its
    // instance lives.
    #[test]
    fn a_background_export_names_a_plugin_that_failed_once() {
        let dir =
            std::env::temp_dir().join(format!("dawsome-app-export-bg-fail-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let text = fs::read_to_string(Path::new(FIXTURES).join("p7_sine.json"))
            .expect("reads")
            .replace(
                r#""id":"clap:org.dawsome.test.sine","name":"Dawsome Test Sine""#,
                r#""id":"clap:org.dawsome.test.fail","name":"Dawsome Test Fail""#,
            );
        let path = dir.join("fail.json");
        fs::write(&path, text).expect("writes");
        let (mut app, _player) = background_app(&path, catalogue_of(variants_file()));
        let export = |message| Message::File(FileMessage::ExportForm(message));
        let _ = app.update(Message::File(FileMessage::Export(ExportKind::Wav)));
        let _ = app.update(export(ExportMessage::Bars("20".into())));
        let _ = app.update(export(ExportMessage::Picked(Some(dir.join("fail")))));
        run_dialog(&mut app, |_| false);
        let notice = app.notice.clone().unwrap_or_default();
        assert!(notice.starts_with("Exported "), "{notice}");
        let named = "Plugin failed: Dawsome Test Fail: it failed while processing";
        assert_eq!(notice.matches(named).count(), 1, "{notice}");
        tick_until_render_hosts_go(&mut app);
        let _ = fs::remove_dir_all(&dir);
    }

    // Cancel during a background export of 2000 bars with the test sine: the dialog closes, the
    // partial file goes, and so do the export's hosts.
    #[test]
    fn a_canceled_background_export_lets_its_plugins_go() {
        let catalogue = catalogue_of(test_plugin_file());
        let (mut app, _player) =
            background_app(&Path::new(FIXTURES).join("p7_sine.json"), catalogue);
        let dir = std::env::temp_dir().join(format!(
            "dawsome-app-export-bg-cancel-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let export = |message| Message::File(FileMessage::ExportForm(message));
        let _ = app.update(Message::File(FileMessage::Export(ExportKind::Wav)));
        let _ = app.update(export(ExportMessage::Bars("2000".into())));
        let _ = app.update(export(ExportMessage::Picked(Some(dir.join("long")))));
        let mut canceled = false;
        run_dialog(&mut app, |progress| {
            let now = progress > 0.0 && !canceled;
            canceled |= now;
            now
        });
        assert!(canceled, "it rendered before the cancel");
        assert!(!dir.join("long.wav").exists(), "the partial file is gone");
        assert_eq!(app.notice, None);
        tick_until_render_hosts_go(&mut app);
        let _ = fs::remove_dir_all(&dir);
    }

    // Stems in the background, p7_sine_song (pattern 1 as a bar's clip on track 1), 32-bit
    // float: a session per file, each with its own sine made on this thread; "01 Track 1.wav"
    // and master.wav both hold gen_p7's known answer 1 (0.5 at frame 6000, 5999 and 30000
    // silent, 96000 frames). The report waits; once it closes, the export's hosts go.
    #[test]
    fn background_stems_play_their_plugin_from_the_windows_thread() {
        let catalogue = catalogue_of(test_plugin_file());
        let (mut app, _player) =
            background_app(&Path::new(FIXTURES).join("p7_sine_song.json"), catalogue);
        let dir = std::env::temp_dir().join(format!(
            "dawsome-app-export-bg-stems-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let export = |message| Message::File(FileMessage::ExportForm(message));
        let _ = app.update(Message::File(FileMessage::Export(ExportKind::Stems)));
        let _ = app.update(export(ExportMessage::Depth(BitDepth::Float32)));
        let _ = app.update(export(ExportMessage::Picked(Some(dir.clone()))));
        run_dialog(&mut app, |_| false);
        assert!(
            matches!(app.files.modal, Some(Modal::Report { .. })),
            "the report shows"
        );
        let folder = dir.join("p7_sine_song stems");
        for file in ["01 Track 1.wav", "master.wav"] {
            let left = left_of(&folder.join(file));
            assert_eq!(left.len(), 96_000, "{file}");
            assert_eq!(
                (left[5999], left[6000], left[30_000]),
                (0.0, 0.5, 0.0),
                "{file}"
            );
        }
        let _ = app.update(Message::File(FileMessage::CloseReport));
        tick_until_render_hosts_go(&mut app);
        let _ = fs::remove_dir_all(&dir);
    }
}
