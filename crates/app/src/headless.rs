//! Scripted runs with nobody at the keyboard: save a screenshot of a window after a delay,
//! quit after a delay, or both. The screenshot is the toolkit's own capture of one window (the
//! main window, or the instrument, Settings, Help or parameter window with
//! `--screenshot-window`), so it works under any compositor and never includes other windows.
//! It is the last frame the window drew: a compositor that is not showing the window (a locked
//! screen) lets it draw only the first, before the window has reported its scale.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dawsome_engine::project::ProcessorOwner;
use dawsome_host_clap::catalogue::PluginKind;
use iced::{time, window, Subscription, Task};
use thiserror::Error;

/// The PNG text keyword under which a screenshot records its scale factor, so a probe can map
/// logical layout coordinates to pixels.
pub const SCALE_FACTOR_KEYWORD: &str = "Scale Factor";

/// What a headless run does. The default does nothing and leaves the window open.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Script {
    pub screenshot: Option<ScreenshotRequest>,
    /// Close the window this long after start.
    pub quit_after: Option<Duration>,
    /// Open the plugin picker at start, listing this kind.
    pub open_picker: Option<PluginKind>,
    /// Write the picker's row labels, in the order drawn, to this file once it is open.
    pub dump_picker: Option<PathBuf>,
    /// Open this plugin's window once the app's window is open, as a double-click on it would,
    /// and print what opened.
    pub open_plugin: Option<ProcessorOwner>,
    /// Open this plugin's parameter window at start, as its channel menu's Parameters… does.
    pub open_params: Option<ProcessorOwner>,
    /// Write the parameter window's title and rows to this file when `open_plugin` or
    /// `open_params` opens it.
    pub dump_params: Option<PathBuf>,
    /// Start the transport at start, as Play would.
    pub play: bool,
    /// Rehearse a take at start: these MIDI messages played into a take (`--rehearse`).
    pub rehearse: Option<PathBuf>,
    /// Play through no device: a thread calls the player in real time (`--null-output`).
    pub null_output: bool,
}

/// Save the window as a PNG at `path`, `after` start.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenshotRequest {
    pub path: PathBuf,
    pub after: Duration,
    /// Shoot again this often, each to `<path's stem>-NNN.png` beside it, until the run quits
    /// (`--screenshot-every`).
    pub every: Option<Duration>,
}

/// Which of the app's windows a screenshot is of.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ShotWindow {
    #[default]
    Main,
    Instrument,
    /// The Settings window.
    Settings,
    /// The Help window.
    Help,
    /// The parameter window.
    Params,
    /// The sample editor.
    SampleEditor,
}

/// The app's windows a screenshot can be of, as far as they are open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Windows {
    /// The main window; none in a test's app, which opens none.
    pub main: Option<window::Id>,
    /// The instrument window, while it is open.
    pub instrument: Option<window::Id>,
    /// The Settings window, while it is open.
    pub settings: Option<window::Id>,
    /// The Help window, while it is open.
    pub help: Option<window::Id>,
    /// The parameter window, while it is open.
    pub params: Option<window::Id>,
    /// The sample editor's window, while it is open.
    pub sample_editor: Option<window::Id>,
}

/// Steps of a headless run.
#[derive(Debug, Clone)]
pub enum Message {
    CaptureDue,
    Captured(window::Screenshot),
    QuitDue,
}

/// Where the screenshot is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Capture {
    Waiting,
    InFlight,
    Saved,
}

/// Plays a [`Script`] against the open window and records whether it failed.
pub struct Runner {
    script: Script,
    /// The window the screenshot is of.
    window: ShotWindow,
    capture: Capture,
    /// Screenshots saved so far (`--screenshot-every` numbers them).
    saved: u32,
    /// Set when the script could not do its job; `main` turns it into the exit status.
    failed: Arc<AtomicBool>,
}

/// Why a screenshot could not be saved.
#[derive(Debug, Error)]
pub(crate) enum SaveError {
    #[error("PNG encoding failed: {0}")]
    Encode(#[from] png::EncodingError),
    #[error(transparent)]
    Write(#[from] std::io::Error),
}

impl Runner {
    pub fn new(script: Script, failed: Arc<AtomicBool>) -> Self {
        Self {
            script,
            window: ShotWindow::Main,
            capture: Capture::Waiting,
            saved: 0,
            failed,
        }
    }

    /// The screenshot is of `window`.
    pub fn shoot(&mut self, window: ShotWindow) {
        self.window = window;
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let capture = match (&self.script.screenshot, self.capture) {
            (Some(request), Capture::Waiting) => {
                let wait = match (request.every, self.saved) {
                    (Some(every), 1..) => every,
                    _ => request.after,
                };
                time::every(wait).map(|_| Message::CaptureDue)
            }
            _ => Subscription::none(),
        };
        let quit = match self.script.quit_after {
            Some(after) => time::every(after).map(|_| Message::QuitDue),
            None => Subscription::none(),
        };
        Subscription::batch([capture, quit])
    }

    /// Takes the next step, with `windows` the app's windows now.
    pub fn update(&mut self, message: Message, windows: Windows) -> Task<Message> {
        match message {
            Message::CaptureDue if self.capture == Capture::Waiting => {
                let shot = match (self.window, windows.main) {
                    (ShotWindow::Main, Some(id)) => Task::done(Some(id)),
                    // Unknown, the main window is the oldest: it opens first and the app exits
                    // rather than close it (the toolkit keeps windows in id order, and ids
                    // count up).
                    (ShotWindow::Main, None) => window::oldest(),
                    (ShotWindow::Instrument, _) => match windows.instrument {
                        Some(id) => Task::done(Some(id)),
                        None => {
                            return self
                                .fail("there is no instrument window to take a screenshot of")
                        }
                    },
                    (ShotWindow::Settings, _) => match windows.settings {
                        Some(id) => Task::done(Some(id)),
                        None => {
                            return self.fail("there is no Settings window to take a screenshot of")
                        }
                    },
                    (ShotWindow::Help, _) => match windows.help {
                        Some(id) => Task::done(Some(id)),
                        None => {
                            return self.fail("there is no Help window to take a screenshot of")
                        }
                    },
                    (ShotWindow::Params, _) => match windows.params {
                        Some(id) => Task::done(Some(id)),
                        None => {
                            return self
                                .fail("there is no parameter window to take a screenshot of")
                        }
                    },
                    (ShotWindow::SampleEditor, _) => match windows.sample_editor {
                        Some(id) => Task::done(Some(id)),
                        None => {
                            return self.fail("there is no sample editor to take a screenshot of")
                        }
                    },
                };
                self.capture = Capture::InFlight;
                shot.and_then(window::screenshot).map(Message::Captured)
            }
            Message::CaptureDue => Task::none(),
            Message::Captured(shot) => {
                let Some(request) = &self.script.screenshot else {
                    return Task::none();
                };
                let path = match request.every {
                    Some(_) => numbered(&request.path, self.saved),
                    None => request.path.clone(),
                };
                if let Err(error) = save_png(&path, &shot) {
                    return self.fail(&format!(
                        "cannot save the screenshot to {}: {error}",
                        path.display()
                    ));
                }
                println!(
                    "wrote {} ({}x{} px, scale factor {})",
                    path.display(),
                    shot.size.width,
                    shot.size.height,
                    shot.scale_factor
                );
                self.saved += 1;
                if request.every.is_some() {
                    self.capture = Capture::Waiting;
                    return Task::none();
                }
                self.capture = Capture::Saved;
                match self.script.quit_after {
                    Some(_) => Task::none(),
                    None => iced::exit(),
                }
            }
            Message::QuitDue => {
                if self.script.screenshot.is_some() && self.saved == 0 {
                    return self.fail("quit time came before the screenshot was saved");
                }
                iced::exit()
            }
        }
    }

    /// Reports why the script could not do its job, marks the run failed and closes the window.
    pub fn fail(&self, reason: &str) -> Task<Message> {
        eprintln!("dawsome-app: {reason}");
        self.failed.store(true, Ordering::Relaxed);
        iced::exit()
    }
}

/// `path` with `-NNN` (the shot's number, from 0) after its stem: `shot.png` → `shot-007.png`.
fn numbered(path: &Path, number: u32) -> PathBuf {
    let stem = path
        .file_stem()
        .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
    path.with_file_name(format!("{stem}-{number:03}.png"))
}

/// Encodes the screenshot as an 8-bit RGBA PNG, its scale factor in a text chunk, and writes it.
pub(crate) fn save_png(path: &Path, shot: &window::Screenshot) -> Result<(), SaveError> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, shot.size.width, shot.size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.add_text_chunk(
        SCALE_FACTOR_KEYWORD.to_owned(),
        shot.scale_factor.to_string(),
    )?;
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&shot.rgba)?;
    writer.finish()?;
    std::fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use iced::Size;

    use super::*;
    use crate::probe::Screenshot as Decoded;

    #[test]
    fn a_saved_screenshot_reads_back_with_its_pixels_and_scale() {
        let rgba: Vec<u8> = vec![0xE5, 0x53, 0x3D, 0xFF, 0x4A, 0x4F, 0x57, 0xFF];
        let shot = window::Screenshot::new(rgba.clone(), Size::new(2, 1), 2.0);
        let path = std::env::temp_dir().join(format!("dawsome-app-{}.png", std::process::id()));
        save_png(&path, &shot).expect("saved");
        let decoded = Decoded::read(&path);
        std::fs::remove_file(&path).expect("removed");
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!(decoded.scale, 2.0);
        assert_eq!(decoded.rgba, rgba);
    }

    // A screenshot of the instrument window while none is open fails the run (its window
    // closes, the exit status says so) rather than taking the main window instead. So
    // does one of the Settings window.
    #[test]
    fn a_shot_of_the_instrument_window_with_none_open_fails_the_run() {
        let script = Script {
            screenshot: Some(ScreenshotRequest {
                path: PathBuf::from("unused.png"),
                after: Duration::from_millis(10),
                every: None,
            }),
            ..Script::default()
        };
        let failed = Arc::new(AtomicBool::new(false));
        let mut runner = Runner::new(script.clone(), Arc::clone(&failed));
        runner.shoot(ShotWindow::Instrument);
        let _ = runner.update(Message::CaptureDue, Windows::default());
        assert!(failed.load(Ordering::Relaxed));
        let failed = Arc::new(AtomicBool::new(false));
        let mut runner = Runner::new(script.clone(), Arc::clone(&failed));
        runner.shoot(ShotWindow::Instrument);
        let open = Windows {
            instrument: Some(window::Id::unique()),
            ..Windows::default()
        };
        let _ = runner.update(Message::CaptureDue, open);
        assert!(!failed.load(Ordering::Relaxed), "one open is taken");
        // The Settings window's likewise.
        let failed = Arc::new(AtomicBool::new(false));
        let mut runner = Runner::new(script.clone(), Arc::clone(&failed));
        runner.shoot(ShotWindow::Settings);
        let _ = runner.update(Message::CaptureDue, open);
        assert!(
            failed.load(Ordering::Relaxed),
            "the instrument window is not it"
        );
        let failed = Arc::new(AtomicBool::new(false));
        let mut runner = Runner::new(script.clone(), Arc::clone(&failed));
        runner.shoot(ShotWindow::Settings);
        let open = Windows {
            settings: Some(window::Id::unique()),
            ..Windows::default()
        };
        let _ = runner.update(Message::CaptureDue, open);
        assert!(
            !failed.load(Ordering::Relaxed),
            "the Settings window open is taken"
        );
        // The Help window's likewise: none open fails, the one open is taken.
        for (help, fails) in [(None, true), (Some(window::Id::unique()), false)] {
            let failed = Arc::new(AtomicBool::new(false));
            let mut runner = Runner::new(script.clone(), Arc::clone(&failed));
            runner.shoot(ShotWindow::Help);
            let open = Windows { help, ..open };
            let _ = runner.update(Message::CaptureDue, open);
            assert_eq!(failed.load(Ordering::Relaxed), fails, "{help:?}");
        }
        // The parameter window's likewise.
        for (params, fails) in [(None, true), (Some(window::Id::unique()), false)] {
            let failed = Arc::new(AtomicBool::new(false));
            let mut runner = Runner::new(script.clone(), Arc::clone(&failed));
            runner.shoot(ShotWindow::Params);
            let open = Windows { params, ..open };
            let _ = runner.update(Message::CaptureDue, open);
            assert_eq!(failed.load(Ordering::Relaxed), fails, "{params:?}");
        }
        // The sample editor's likewise.
        for (sample_editor, fails) in [(None, true), (Some(window::Id::unique()), false)] {
            let failed = Arc::new(AtomicBool::new(false));
            let mut runner = Runner::new(script.clone(), Arc::clone(&failed));
            runner.shoot(ShotWindow::SampleEditor);
            let open = Windows {
                sample_editor,
                ..open
            };
            let _ = runner.update(Message::CaptureDue, open);
            assert_eq!(failed.load(Ordering::Relaxed), fails, "{sample_editor:?}");
        }
    }

    // `--screenshot-every`: each shot beside the path asked for, numbered from 0.
    #[test]
    fn shots_taken_again_are_numbered() {
        assert_eq!(
            numbered(Path::new("/out/take.png"), 7),
            PathBuf::from("/out/take-007.png")
        );
    }

    #[test]
    fn saving_into_a_missing_directory_fails() {
        let shot = window::Screenshot::new(vec![0; 4], Size::new(1, 1), 1.0);
        let result = save_png(Path::new("/nonexistent/dir/shot.png"), &shot);
        assert!(matches!(result, Err(SaveError::Write(_))), "{result:?}");
    }
}
