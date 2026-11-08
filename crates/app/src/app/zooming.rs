//! The zoom in the app ([`crate::zoom`]): the keys' and the Settings stepper's steps, the
//! toolbar's notice, and every size the app holds in a window's own pixels kept true when the
//! zoom changes. The toolkit reads [`App::zoom`] after every message (`main.rs`'s scale factor)
//! and redraws each window at it; a window keeps its size on the screen, so its own logical
//! pixels grow or shrink, and the toolkit tells no window so: the app works the new sizes out
//! itself ([`App::rezoom`]).

use std::time::{Duration, Instant};

use iced::{window, Size, Task};

use super::{App, Message, WindowChange};
use crate::desktop::PopOut;
use crate::settings::WindowPlace;
use crate::zoom::{Step, Zoom};

/// How long the toolbar says "Zoom 125 %" after a step.
pub(super) const ZOOM_NOTICE: Duration = Duration::from_secs(2);

/// How soon a second press on the Settings window's zoom value sets 100 %, as the app's other
/// double presses.
const DOUBLE_PRESS: Duration = Duration::from_millis(400);

impl App {
    /// The zoom every window draws at: the settings' (a scripted run's `--zoom` for the run).
    pub fn zoom(&self) -> Zoom {
        self.settings.zoom()
    }

    /// The main window's device pixels per its own logical pixel: the system's scale times the
    /// zoom. What its 1-px lines snap to, and what a desktop drop's pointer is divided by.
    pub(super) fn device_scale(&self) -> f32 {
        self.os_scale * self.zoom().factor()
    }

    /// A key's or the stepper's step.
    pub(super) fn zoom_by(&mut self, step: Step) -> Task<Message> {
        self.set_zoom(self.zoom().stepped(step))
    }

    /// A press on the Settings window's zoom value: a second one soon after sets 100 %.
    pub(super) fn zoom_value_pressed(&mut self) -> Task<Message> {
        let now = Instant::now();
        let double = self
            .zoom_pressed
            .replace(now)
            .is_some_and(|before| now.duration_since(before) < DOUBLE_PRESS);
        if double {
            self.zoom_pressed = None;
            return self.zoom_by(Step::Reset);
        }
        Task::none()
    }

    /// Sets the zoom: kept in the settings, every size the app holds in a window's pixels
    /// worked out anew, each pop-out window's least size asked again in its new pixels, and the
    /// toolbar saying "Zoom 125 %" for [`ZOOM_NOTICE`] (also when a step went nowhere, at an
    /// end: the notice then says where it stays).
    pub(super) fn set_zoom(&mut self, zoom: Zoom) -> Task<Message> {
        let was = self.zoom();
        self.zoom_notice = Some((zoom, Instant::now()));
        // The timer is made when the task first runs, on the app's runtime (a test's app runs
        // no task).
        let due = Task::perform(async { tokio::time::sleep(ZOOM_NOTICE).await }, |()| {
            Message::ZoomNoticeDue
        });
        if zoom == was {
            return due;
        }
        self.settings.set_zoom(zoom);
        self.save_settings();
        self.rezoom(was, zoom);
        Task::batch([due, self.pop_out_least_sizes(), self.window_sizes()])
    }

    /// Every size the app holds in a window's own pixels, from zoom `was` to `now`: the main
    /// window's size and its size under maximized, each pop-out window's place (open or not)
    /// and the sizes the open ones last said. Why by hand: the toolkit keeps each window's size
    /// on the screen and redraws it at the new zoom, but sends no resize.
    fn rezoom(&mut self, was: Zoom, now: Zoom) {
        let by = was.factor() / now.factor();
        let rezoomed = |place: WindowPlace| place.system(was).zoomed(now);
        self.window_size = self.window_size * by;
        self.restored_size = self.restored_size * by;
        self.instrument_place = rezoomed(self.instrument_place);
        self.settings_place = rezoomed(self.settings_place);
        self.help.place = rezoomed(self.help.place);
        self.params.place = rezoomed(self.params.place);
        self.editor.place = rezoomed(self.editor.place);
        self.panel_windows.rezoom(was, now);
        self.instrument_width = self.instrument_width.map(|width| width * by);
        self.params.size = self.params.size.map(|size| size * by);
        self.editor.size = self.editor.size.map(|size| size * by);
        self.lay_out_mixer_desk();
    }

    /// The pop-out windows open now, with what each is.
    fn open_pop_outs(&self) -> Vec<(window::Id, PopOut)> {
        [
            (self.instrument_window, PopOut::Instrument),
            (self.settings_window, PopOut::Settings),
            (self.help.window, PopOut::Help),
            (self.params.window, PopOut::Params),
            (self.editor.window, PopOut::SampleEditor),
        ]
        .into_iter()
        .chain(self.panel_windows.open())
        .filter_map(|(id, window)| Some((id?, window)))
        .collect()
    }

    /// Each open pop-out window's least size asked again: the toolkit holds a window to a least
    /// size in its pixels at the time it is asked (opening, it takes it in the system's, which
    /// `desktop::pop_out_settings` multiplies by the zoom).
    fn pop_out_least_sizes(&self) -> Task<Message> {
        Task::batch(
            self.open_pop_outs()
                .into_iter()
                .map(|(id, window)| window::set_min_size(id, Some(window.least_size()))),
        )
    }

    /// Every open window's size asked of the toolkit, as it lays them out at the new zoom (it
    /// answers after it has taken the zoom up), each answer a resize as the window reports one:
    /// the sizes [`App::rezoom`] worked out hold until then, and the answers end the rounding
    /// that steps one after another leave in them.
    fn window_sizes(&self) -> Task<Message> {
        let windows = self
            .main_window
            .into_iter()
            .chain(self.open_pop_outs().into_iter().map(|(id, _)| id));
        Task::batch(windows.map(|id| {
            window::size(id).map(move |size| Message::Windowed(id, WindowChange::Resized(size)))
        }))
    }

    /// A pop-out window just opened: its monitor's size asked, so one larger than its monitor
    /// (a first size at a large zoom: the synth's 924 × 516 at 200 %) is made to fit it.
    pub(super) fn fit_opened_pop_out(&self, message: &Message) -> Task<Message> {
        match *message {
            Message::Windowed(id, WindowChange::Opened { size, .. })
                if !self.is_main_window(id) =>
            {
                window::monitor_size(id).map(move |monitor| Message::PopOutMonitor {
                    window: id,
                    size,
                    monitor,
                })
            }
            _ => Task::none(),
        }
    }

    /// A pop-out window `size` big on a monitor of `monitor`: made to fit it when larger.
    pub(super) fn fit_pop_out(
        window: window::Id,
        size: Size,
        monitor: Option<Size>,
    ) -> Task<Message> {
        match monitor.and_then(|monitor| crate::settings::fit_to_monitor(size, monitor)) {
            Some(fitted) => window::resize(window, fitted),
            None => Task::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use iced::keyboard::key::Code;
    use iced::keyboard::{Key, Modifiers};
    use iced::{Point, Size};

    use super::super::key_input::KeyInput;
    use super::super::tests::app_on;
    use super::*;
    use crate::ui::panels::Panel;
    use crate::ui::settings_panel::SettingsMessage;

    const F1: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/f1_four_on_floor.json"
    );

    fn f1_app() -> App {
        let (mut app, _player) = app_on(Path::new(F1));
        app.set_main_window(window::Id::unique());
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 560.0)));
        app
    }

    fn zoomed(app: &mut App, step: Step) {
        let _ = app.update(Message::Zoom(step));
    }

    /// `is` and `want` alike to a thousandth of a pixel: three steps of float division leave a
    /// little (the toolkit's own answer, which a test's app gets none of, ends it in the app).
    fn near(is: Size, want: Size) -> bool {
        (is.width - want.width).abs() < 1e-3 && (is.height - want.height).abs() < 1e-3
    }

    // Known answers worked by hand: a window keeps its size on the screen, so at a zoom of 125 %
    // a main window of 1000 x 560 holds 800 x 448 of its own pixels (1000 / 1.25, 560 / 1.25),
    // and the layout kept for the next start is 1000 x 560 again (800 × 1.25); a pop-out
    // window's place of 560 x 420 becomes 448 x 336. The toolbar says "Zoom 125 %" first among
    // its notices. Back at 100 % every size is what it was.
    #[test]
    fn a_zoom_keeps_every_window_its_size_on_the_screen() {
        let mut app = f1_app();
        app.instrument_place = WindowPlace::of(Size::new(560.0, 420.0), None);
        for _ in 0..3 {
            zoomed(&mut app, Step::Larger);
        }
        assert_eq!(app.zoom().percent(), 125, "100, 110, 120, 125");
        assert!(
            near(app.window_size, Size::new(800.0, 448.0)),
            "{:?}",
            app.window_size
        );
        let layout = app.window_layout();
        let kept = Size::new(layout.width, layout.height);
        assert!(near(kept, Size::new(1000.0, 560.0)), "{kept:?}");
        let place = app.instrument_place;
        let place = Size::new(place.width, place.height);
        assert!(near(place, Size::new(448.0, 336.0)), "{place:?}");
        let notices = app.toolbar().notice_text().expect("a notice");
        assert!(notices.starts_with("Zoom 125 %"), "{notices}");
        zoomed(&mut app, Step::Reset);
        assert_eq!(app.zoom(), Zoom::DEFAULT);
        assert!(near(app.window_size, Size::new(1000.0, 560.0)));
        let place = app.instrument_place;
        assert!(near(
            Size::new(place.width, place.height),
            Size::new(560.0, 420.0)
        ));
        assert!(app
            .toolbar()
            .notice_text()
            .is_some_and(|text| text.starts_with("Zoom 100 %")));
    }

    // The zoom holds at its ends: Ctrl+- at 50 % stays 50 % (and says so), Ctrl+= at 200 %
    // stays 200 %. Each step is a stop: from 100 down 90, 80, 75, 70, 60, 50.
    #[test]
    fn the_zoom_holds_at_its_ends() {
        let mut app = f1_app();
        let mut seen = Vec::new();
        for _ in 0..8 {
            zoomed(&mut app, Step::Smaller);
            seen.push(app.zoom().percent());
        }
        assert_eq!(seen, [90, 80, 75, 70, 60, 50, 50, 50]);
        for _ in 0..25 {
            zoomed(&mut app, Step::Larger);
        }
        assert_eq!(app.zoom().percent(), 200);
        assert!(app
            .toolbar()
            .notice_text()
            .is_some_and(|text| text.starts_with("Zoom 200 %")));
    }

    // The Settings window's stepper: + a stop larger, − a stop smaller, two presses on the
    // value soon after each other 100 %; one press alone changes nothing.
    #[test]
    fn the_settings_stepper_steps_the_zoom_and_a_double_press_resets_it() {
        let mut app = f1_app();
        let _ = app.update(Message::TogglePanel(Panel::Settings));
        let layout = app.settings_panel().layout();
        let press = |app: &mut App, at: Point| {
            let _ = app.update(Message::Settings(SettingsMessage::Pressed(at)));
        };
        press(&mut app, layout.zoom_up().center());
        press(&mut app, layout.zoom_up().center());
        assert_eq!(app.zoom().percent(), 120);
        press(&mut app, layout.zoom_down().center());
        assert_eq!(app.zoom().percent(), 110);
        press(&mut app, layout.zoom_value().center());
        assert_eq!(app.zoom().percent(), 110, "one press");
        press(&mut app, layout.zoom_value().center());
        assert_eq!(app.zoom(), Zoom::DEFAULT, "a double press");
    }

    // Ctrl+= in a pop-out window (the Settings window) zooms as in the main window, Ctrl+0
    // sets 100 % back, and every window follows the one zoom.
    #[test]
    fn the_zoom_keys_work_from_a_pop_out_window() {
        let mut app = f1_app();
        let _ = app.update(Message::TogglePanel(Panel::Settings));
        let settings = app.settings_window.expect("open");
        let key = |app: &mut App, typed: &str, code: Code| {
            let input = KeyInput {
                window: settings,
                key: Key::Character(typed.into()),
                code: Some(code),
                modifiers: Modifiers::CTRL,
                pressed: true,
                repeat: false,
            };
            let _ = app.update(Message::Key(input));
        };
        key(&mut app, "=", Code::Equal);
        assert_eq!(app.zoom().percent(), 110);
        key(&mut app, "-", Code::Minus);
        key(&mut app, "-", Code::Minus);
        assert_eq!(app.zoom().percent(), 90);
        key(&mut app, "0", Code::Digit0);
        assert_eq!(app.zoom(), Zoom::DEFAULT);
    }
}
