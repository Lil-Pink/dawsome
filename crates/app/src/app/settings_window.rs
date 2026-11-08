//! The Settings window's life in the app (it was a panel stacked at the main
//! window's foot, which could not scroll to its lower options and had no close). It is a window
//! of its own beside the main one, as the instrument window is: `App::panels.settings` says
//! whether it is open (the toolbar's Settings button lights by it, and F10, the digit 6, the
//! button and a press on the notices toggle or open it, as they did the panel), and
//! [`App::sync_settings_window`] opens and closes its window to match after every message, so
//! every path (its keys, Esc in it, its X, the system closing it, the main window closing the
//! app) needs no task of its own. It keeps the instrument window's level (above the main window
//! while the app has the keyboard). It opens at a share of the main window's size, which the
//! user cannot change (`settings_panel::window_size_for`; a scripted run's `--window-size` is
//! its size instead), and its position is kept in the settings (`settings_window`) when it
//! closes and when the app closes with it open.
//!
//! Its header (the page tabs) stays at its top while the page under it scrolls, and a line at
//! its foot names the control under the cursor (while hints show) or says how to close it. Keys
//! typed in it do what they do in the main window (the typing keyboard plays, the shortcuts
//! run), as the instrument window's do; a row of the Keys page waiting for its chord takes it
//! from this window alone. Its one text box, the accent's hex box on its Theme page, takes them
//! while it has the keys, as every text box does (the toolkit holds back the presses it takes;
//! Esc still closes the window).

use iced::widget::{column, container, scrollable, text};
use iced::{window, Element, Fill, Task};

use super::instrument::pop_out_level;
use super::{App, Message, WindowChange};
use crate::desktop::{self, PopOut};
use crate::keys::Action;
use crate::settings::{WindowPlace, WindowPosition};
use crate::ui::fonts::size;
use crate::ui::hint;
use crate::ui::layout::{HINT_BAR_HEIGHT, TOOLBAR_SIDE_PADDING};
use crate::ui::panels::Panel;
use crate::ui::settings_layout::SettingsPage;
use crate::ui::settings_panel;
use crate::ui::settings_panel::SettingsHeader;
use crate::ui::style;

impl App {
    /// Opens the Settings window's own window while Settings is open and closes it once it is
    /// not, keeping its place for the next opening; asks it to the app's level again when
    /// `restack`.
    pub(super) fn sync_settings_window(&mut self, restack: bool) -> Task<Message> {
        let level = pop_out_level(self.windows_above);
        match (self.panels.settings, self.settings_window) {
            (true, None) => {
                let settings = self.settings_window_opening(level);
                let (id, opened) = window::open(settings);
                self.settings_window = Some(id);
                opened.discard()
            }
            (false, Some(id)) => {
                self.settings_window = None;
                self.remember_settings_place();
                self.forget_hex_typed();
                window::close(id)
            }
            (true, Some(id)) if restack => window::set_level(id, level),
            _ => Task::none(),
        }
    }

    /// The Settings window about to open at `level`: a scripted run's `--window-size`, else the
    /// main window's share of its last size ([`settings_panel::window_size_for`]), where it
    /// last was; the user cannot resize it. Its place takes that size, which its page's columns
    /// follow.
    pub(super) fn settings_window_opening(&mut self, level: window::Level) -> window::Settings {
        let size = self
            .headless
            .settings_size()
            .unwrap_or_else(|| settings_panel::window_size_for(self.window_size));
        self.settings_place = WindowPlace::of(size, self.settings_place.position());
        window::Settings {
            resizable: false,
            ..desktop::pop_out_settings(
                PopOut::Settings,
                size,
                self.settings_place.position(),
                level,
                self.zoom(),
            )
        }
    }

    /// The Settings window opened, was resized or moved: its place is kept.
    pub(super) fn settings_window_changed(&mut self, change: WindowChange) {
        let place = &mut self.settings_place;
        match change {
            WindowChange::Opened { size, position } => {
                *place = WindowPlace::of(size, position.or(place.position()));
            }
            WindowChange::Resized(size) => *place = WindowPlace::of(size, place.position()),
            WindowChange::Moved(at) => {
                *place = WindowPlace::of(place.size(PopOut::Settings), Some(at));
            }
            WindowChange::Rescaled(_) | WindowChange::Focus(_) => {}
        }
    }

    /// What the settings keep of the Settings window's place: its position, when the system
    /// told it (its size is worked out each time it opens).
    pub(super) fn settings_place_kept(&self) -> Option<WindowPosition> {
        self.settings_place.position().map(WindowPosition::at)
    }

    /// Keeps the Settings window's position in the settings for its next opening.
    pub(super) fn remember_settings_place(&mut self) {
        let place = self.settings_place_kept();
        if self.settings.settings_window != place {
            self.settings.settings_window = place;
            self.save_settings();
        }
    }

    /// Closes the Settings window: its X, Esc in it, or the system closing it.
    pub(super) fn close_settings(&mut self) {
        self.panels.settings = false;
    }

    /// Shows Settings on `page`: opens its window as its key does, or, open already, brings it
    /// to the front (where the window system lets an app ask that).
    pub(super) fn show_settings(&mut self, page: SettingsPage) -> Task<Message> {
        self.show_settings_page(page);
        if !self.panels.settings {
            return self.handle(Message::TogglePanel(Panel::Settings));
        }
        self.settings_window
            .map_or_else(Task::none, window::gain_focus)
    }

    /// What the Settings window shows: the header with the page tabs (General, Keys, Theme), the
    /// page shown scrolling under it (the Keys page across too, in a window narrower than its
    /// table), and the foot line.
    pub(super) fn settings_window_view(&self) -> Element<'_, Message> {
        let header = SettingsHeader {
            page: self.settings_page,
        };
        let page: Element<'_, Message> = match self.settings_page {
            SettingsPage::General => scrollable(self.settings_panel().view()).height(Fill).into(),
            SettingsPage::Keys => {
                let page = crate::ui::keys_page::KeysPage {
                    keys: &self.keys,
                    layout: self.typing.layout,
                    labels: &self.typing.labels,
                    base: self.typing.base,
                    rebinding: self.rebinding,
                };
                scrollable(page.view())
                    .direction(scrollable::Direction::Both {
                        vertical: scrollable::Scrollbar::default(),
                        horizontal: scrollable::Scrollbar::default(),
                    })
                    .width(Fill)
                    .height(Fill)
                    .into()
            }
            SettingsPage::Theme => scrollable(self.theme_page().view()).height(Fill).into(),
        };
        let words = self.settings_hint();
        let foot = iced::widget::responsive(move |size| {
            let room = (size.width - 2.0 * TOOLBAR_SIDE_PADDING).max(0.0);
            let shown = crate::ui::text_fit::fit_width(&words, room, size::LABEL)
                .map(std::borrow::Cow::into_owned)
                .unwrap_or_default();
            container(
                text(shown)
                    .size(size::LABEL)
                    .color(style::palette().text_secondary)
                    .wrapping(iced::widget::text::Wrapping::None),
            )
            .padding([0.0, TOOLBAR_SIDE_PADDING])
            .height(Fill)
            .align_y(iced::Center)
            .into()
        });
        let foot = container(foot)
            .width(Fill)
            .height(HINT_BAR_HEIGHT)
            .style(style::panel);
        container(column![header.view(), page, foot])
            .width(Fill)
            .height(Fill)
            .style(style::rack_well)
            .into()
    }

    /// The Settings window's foot line: what its control under the cursor is and does, as the
    /// main window's hint bar names its own, while hints show; else how to close the window.
    pub(super) fn settings_hint(&self) -> String {
        match &self.hover {
            Some(hover) if hover.in_settings() && self.shows_hints() => {
                hint::words(hover, &self.hint_context())
            }
            _ => format!(
                "Esc or {} closes Settings",
                self.keys.all(Action::Panel(Panel::Settings), " or ")
            ),
        }
    }

    /// The Settings window's title, the main window's way: "Settings - Dawsome".
    pub(super) fn settings_title(&self) -> String {
        "Settings - Dawsome".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use iced::keyboard::key::{Code, Named};
    use iced::keyboard::{Key, Modifiers};
    use iced::{Point, Size};

    use super::super::key_input::KeyInput;
    use super::super::tests::{app_hosted, app_on};
    use super::*;
    use crate::document::Host;
    use crate::ui::settings_layout::{tab, Layout};
    use crate::ui::settings_panel::SettingsMessage;

    const F1: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/f1_four_on_floor.json"
    );

    /// F1 in an app told its main window's id, as main tells it.
    fn f1_app() -> (App, dawsome_engine::realtime::Player, window::Id) {
        let (mut app, player) = app_on(Path::new(F1));
        let main = window::Id::unique();
        app.set_main_window(main);
        (app, player, main)
    }

    fn toggle(app: &mut App) {
        let _ = app.update(Message::TogglePanel(Panel::Settings));
    }

    /// `key` pressed in the window `window`.
    fn press(app: &mut App, window: window::Id, key: Key, code: Code) {
        let input = KeyInput {
            window,
            key,
            code: Some(code),
            modifiers: Modifiers::empty(),
            pressed: true,
            repeat: false,
        };
        let _ = app.update(Message::Key(input));
    }

    fn open(app: &App) -> (bool, bool) {
        (app.panels.settings, app.settings_window.is_some())
    }

    // Settings is a window of its own beside the main one, titled the
    // main window's way. Its button toggles it (a second press closes it), and so do F10 and the
    // digit 6 typed in the main window (the typing keyboard on, as a new window has it: the
    // digits 1 to 6 keep their shortcuts). Esc in it closes it and Esc in the main window does
    // not; its X closes it alone (not the app), and so does the system closing it, after which
    // a close request from it is no request to close the app. A press on the notices opens it
    // on its General page, and a second press keeps it open; the first-run note's "Open Keys
    // settings" opens it on its Keys page. Under a dialog in the main window, its Esc and its X
    // still close it. The main window's X closes the app, the Settings window with it.
    #[test]
    fn the_settings_window_is_a_window_of_its_own() {
        let (mut app, _player, main) = f1_app();
        toggle(&mut app);
        let id = app.settings_window.expect("a window of its own");
        assert!(app.panels.settings);
        assert_ne!(Some(id), app.main_window);
        assert_eq!(app.window_title(id), "Settings - Dawsome");
        assert_eq!(app.window_title(main), app.title());
        toggle(&mut app);
        assert_eq!(open(&app), (false, false), "a second press closes it");
        for (key, code) in [
            (Key::Named(Named::F10), Code::F10),
            (Key::Character("6".into()), Code::Digit6),
        ] {
            press(&mut app, main, key.clone(), code);
            assert_eq!(open(&app), (true, true), "{code:?} opens it");
            press(&mut app, main, key, code);
            assert_eq!(open(&app), (false, false), "{code:?} again closes it");
        }

        toggle(&mut app);
        let id = app.settings_window.expect("open");
        let _ = app.update(Message::Escape(main));
        assert_eq!(open(&app), (true, true), "Esc in the main window leaves it");
        press(&mut app, id, Key::Named(Named::Escape), Code::Escape);
        assert_eq!(open(&app), (false, false), "Esc in it closes it");

        toggle(&mut app);
        let id = app.settings_window.expect("open");
        let _ = app.update(Message::WindowCloseRequested(id));
        assert_eq!(open(&app), (false, false), "its X closes it");
        assert!(!app.exiting(), "and not the app");
        toggle(&mut app);
        let id = app.settings_window.expect("open");
        let _ = app.update(Message::WindowClosed(id));
        assert_eq!(open(&app), (false, false), "the system closed it");
        let _ = app.update(Message::WindowCloseRequested(id));
        assert!(!app.exiting());

        app.settings_page = SettingsPage::Keys;
        let _ = app.update(Message::NoticePressed);
        assert_eq!(open(&app), (true, true), "a notice's press opens it");
        assert_eq!(app.settings_page, SettingsPage::General);
        let _ = app.update(Message::NoticePressed);
        assert_eq!(open(&app), (true, true), "a second keeps it open");
        toggle(&mut app);
        let _ = app.update(Message::Callout(super::super::CalloutMessage::OpenKeys));
        assert_eq!(open(&app), (true, true), "Open Keys settings opens it");
        assert_eq!(app.settings_page, SettingsPage::Keys);

        let id = app.settings_window.expect("open");
        app.files.modal = Some(super::super::files::Modal::Unsaved);
        let _ = app.update(Message::Escape(id));
        assert_eq!(open(&app), (false, false), "Esc in it under a dialog");
        app.files.modal = None;
        toggle(&mut app);
        let id = app.settings_window.expect("open");
        app.files.modal = Some(super::super::files::Modal::Unsaved);
        let _ = app.update(Message::WindowCloseRequested(id));
        assert_eq!(open(&app), (false, false), "its X under a dialog");
        app.files.modal = None;

        toggle(&mut app);
        let _ = app.update(Message::WindowCloseRequested(main));
        assert!(app.exiting(), "the main window's X closes the app");
    }

    // `--panels settings` opens the window at start, as the main window's first sync does
    // (main runs it once the main window is open), on the page `--settings-page` names.
    #[test]
    fn panels_settings_on_the_command_line_opens_the_window() {
        let (mut app, _player, _main) = f1_app();
        app.panels.settings = true;
        app.show_settings_page(SettingsPage::Keys);
        let _ = app.sync_windows();
        assert!(app.settings_window.is_some());
        assert_eq!(app.settings_page, SettingsPage::Keys);
    }

    // It keeps the instrument window's level: asked to stay above every window while one of
    // the app's windows has the keyboard (the main window giving it to the Settings window keeps
    // that), as any window once another program has it, above again when the main window takes
    // it back; each change asks the open window again, and no change asks nothing. The
    // instrument window opens beside it, a window of its own, and Esc in the one closes that
    // one alone. The main window gone without asking closes the app.
    #[test]
    fn the_settings_window_keeps_the_pop_out_level() {
        let (mut app, _player, main) = f1_app();
        let focus = |app: &mut App, id, focused| {
            let _ = app.update(Message::Windowed(id, WindowChange::Focus(focused)));
        };
        focus(&mut app, main, true);
        toggle(&mut app);
        let id = app.settings_window.expect("open");
        assert!(app.windows_above);
        focus(&mut app, main, false);
        focus(&mut app, id, true);
        assert!(app.windows_above, "the keyboard moved to it");
        assert_eq!(app.focused_window, Some(id));
        focus(&mut app, id, false);
        assert!(!app.windows_above, "another program has it");
        focus(&mut app, main, true);
        assert!(app.windows_above);
        assert_eq!(
            app.sync_settings_window(true).units(),
            1,
            "a restack asks it"
        );
        assert_eq!(
            app.sync_settings_window(false).units(),
            0,
            "a steady one not"
        );

        let _ = app.update(Message::OpenInstrument(dawsome_engine::project::ChannelId(
            1,
        )));
        let instrument = app.instrument_window.expect("open beside it");
        assert_ne!(instrument, id);
        assert_eq!(app.settings_window, Some(id));
        let _ = app.update(Message::Escape(instrument));
        assert_eq!(
            (app.instrument_window, app.settings_window),
            (None, Some(id))
        );
        let _ = app.update(Message::Escape(id));
        assert_eq!(app.settings_window, None);

        toggle(&mut app);
        let _ = app.update(Message::WindowClosed(main));
        assert!(app.exiting(), "the main window gone closes the app");
    }

    // It opens at the main window's share of its last size, which the user cannot change, and
    // keeps its position in the settings when it closes and when the app closes with it open,
    // in a later start too; no size is kept. By hand: main 1920 x 1080 → 902 x 724 (0.47 and
    // 0.67, rounded), main 2560 x 1440 → 1203 x 965. A settings file an earlier build wrote,
    // its size beside the position, opens at the position and the share, and the next save
    // drops the size. A scripted run's --window-size (720 x 900 here) is its size instead.
    #[test]
    fn the_settings_window_opens_at_its_share_and_keeps_its_position() {
        use crate::settings::Settings;
        let dir =
            std::env::temp_dir().join(format!("dawsome-settings-place-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("dawsome/settings.json");
        let mut host = Host::offline();
        host.settings_file = Some(file.clone());
        host.saves_settings = true;
        let project = || {
            crate::project_file::open(Path::new(F1))
                .expect("opens")
                .project
        };
        let (mut app, _player) = app_hosted(project(), Path::new(F1), host.clone());
        let main = window::Id::unique();
        app.set_main_window(main);
        let _ = app.update(Message::WindowResized(Size::new(1920.0, 1080.0)));
        let opening = app.settings_window_opening(window::Level::Normal);
        assert_eq!(opening.size, Size::new(902.0, 724.0));
        assert!(!opening.resizable, "the user does not resize it");
        assert!(matches!(opening.position, window::Position::Default));
        toggle(&mut app);
        let id = app.settings_window.expect("open");
        assert_eq!(
            app.settings_place.size(PopOut::Settings),
            Size::new(902.0, 724.0)
        );
        let _ = app.update(Message::Windowed(
            id,
            WindowChange::Moved(Point::new(30.0, 40.0)),
        ));
        let _ = app.update(Message::Escape(id));
        let kept = Some(WindowPosition::at(Point::new(30.0, 40.0)));
        assert_eq!(Settings::load(Some(&file)).settings_window, kept);
        let text = std::fs::read_to_string(&file).expect("reads");
        assert!(
            text.contains(
                "\"settings_window\": {\n    \"position\": [\n      30.0,\n      40.0\n    ]\n  }"
            ),
            "its position alone: {text}"
        );
        let _ = app.update(Message::WindowResized(Size::new(2560.0, 1440.0)));
        toggle(&mut app);
        assert!(app.settings_window.is_some(), "open again");
        assert_eq!(
            app.settings_place,
            WindowPlace::of(Size::new(1203.0, 965.0), Some(Point::new(30.0, 40.0))),
            "the new share, where it was"
        );
        let _ = app.update(Message::WindowCloseRequested(main));
        assert!(app.exiting());
        assert_eq!(Settings::load(Some(&file)).settings_window, kept);

        let earlier = r#"{"settings_window":{"width":600.0,"height":450.0,"position":[5.0,6.0]}}"#;
        std::fs::write(&file, earlier).expect("writes");
        let (mut later, _player) = app_hosted(project(), Path::new(F1), host.clone());
        later.set_main_window(main);
        assert_eq!(later.settings_place.position(), Some(Point::new(5.0, 6.0)));
        let _ = later.update(Message::WindowResized(Size::new(1920.0, 1080.0)));
        toggle(&mut later);
        assert_eq!(
            later.settings_place.size(PopOut::Settings),
            Size::new(902.0, 724.0),
            "the size it kept passed over"
        );
        let id = later.settings_window.expect("open");
        let _ = later.update(Message::Escape(id));
        let text = std::fs::read_to_string(&file).expect("reads");
        assert!(
            text.contains("\"position\"") && !text.contains("\"width\": 600.0"),
            "the size dropped on the save: {text}"
        );

        let session = || {
            crate::session::Session::open(
                project(),
                crate::project_file::samples_dir(Path::new(F1)),
                false,
            )
            .expect("compiles")
            .0
        };
        let (mut scripted, _) = App::boot(
            session(),
            Err(crate::audio::StartError::NoDevice),
            crate::ui::panels::Panels::default(),
            Size::new(1000.0, 560.0),
            false,
            crate::headless::Script {
                settings_size: Some(Size::new(720.0, 900.0)),
                ..crate::headless::Script::default()
            },
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            Host::offline(),
            super::super::files::Start {
                document: crate::document::Document::untitled(),
            },
        );
        let opening = scripted.settings_window_opening(window::Level::Normal);
        assert_eq!(opening.size, Size::new(720.0, 900.0), "its --window-size");
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    /// The General page of two ports, three output devices and two input devices, nothing
    /// dropped and nothing wrong.
    fn general_page<'a>(
        ports: &'a [String],
        inputs: &'a crate::settings::MidiInputs,
    ) -> crate::ui::settings_panel::SettingsPanel<'a> {
        use crate::audio::OutputChoice;
        let named = |name: &str| OutputChoice {
            id: Some(name.to_owned()),
            name: Some(name.to_owned()),
            missing: false,
            chosen: false,
        };
        let default = OutputChoice {
            id: None,
            name: None,
            missing: false,
            chosen: true,
        };
        crate::ui::settings_panel::SettingsPanel {
            ports,
            inputs,
            dropped: 0,
            lost: 0,
            problem: None,
            octave: "C5".to_owned(),
            hints: true,
            zoom: crate::zoom::Zoom::DEFAULT,
            outputs: vec![default.clone(), named("Speakers"), named("USB Audio")],
            playing: "Playing through: Speakers, 48000 Hz, 512 frames".to_owned(),
            resampling: None,
            refused: None,
            outputs_problem: None,
            inputs_offered: vec![default, named("USB Audio Line Input")],
            inputs_problem: None,
            latency_devices: "Line in: System default input; output: System default".to_owned(),
            latency: None,
            measuring: false,
            reset_armed: false,
            width: crate::ui::settings_layout::FIRST_WIDTH,
        }
    }

    // Its pages scroll under the header, which stays. By hand, from the layout's rows (24 px,
    // WCAG 2.2's target size; 22 before), the window's first 720 px one column (under 800), the
    // sections 28 px apart: with two ports the MIDI input's title at 32, its three rows to 124;
    // the typing keyboard's title 28 px under that at 152, its row to 196; the hints' at 224,
    // to 268; the display's at 296, its zoom row to 340; the audio output's title at 368, its
    // three devices' rows 388 to 460, the line saying what plays 460 to 484; the audio input's
    // title at 512, its two rows 532 to 580; the input latency's title at 608, its two lines
    // from 628 and its buttons' row 676 to 700; "Reset to defaults" in the row 28 px under that,
    // 728 to 752, 18 px tall centred in it from the left margin, 12..144 by 731..749 (132 px:
    // "Press again to reset" and 8 px either side): the General page ends 8 px under that, at
    // 760, and its canvas under the 24 px header is 736 px tall (it was the panel's height,
    // which cut the input rows off; 588 and 564 before the reset's row, 620 and 596 with the
    // sections 8 px apart). The Theme page ends at
    // 324 (its note line's foot, 316, and 8 px), its canvas 300 px tall. The Keys
    // page is its table's 680 px and a 12 px margin across, so a window narrower than that
    // scrolls it across, and as tall as its content. The tabs switch the page in the window,
    // whose view builds on each. The least window keeps the tabs (General, Keys, then Theme, to
    // x 262) and the first group (the MIDI input's title, "All ports" and a port, or the line
    // saying none was found, to y 100) above its 22 px foot line.
    #[test]
    fn the_settings_pages_scroll_under_their_header() {
        use iced::Length;
        let ports = ["Port A".to_owned(), "Port B".to_owned()];
        let inputs = crate::settings::MidiInputs::default();
        let page = general_page(&ports, &inputs);
        // Pins moved 50 px: the Display section, its title's gap and height and the zoom's
        // row (8 + 20 + 22), under the hints (were 226, 342, 414, 508 and 484); then 22 px by
        // the theme's row under the zoom's (were 276, 392, 464, 558 and 534); then 146 px by the
        // accent's row (124) and its note's line (22) under the theme's (were 298, 414, 486, 580
        // and 556); then 22 px by the theme's second row of keys (were 444, 560, 632, 726 and
        // 702); then 190 px up when the theme's keys and the accent went to the Theme page (were
        // 466, 582, 654, 748 and 724), the first move's pins again; then down with the 24 px rows (were 276,
        // 392, 464, 558 and 534); then down with the sections 28 px apart (were 288, 412, 488,
        // 580 and 591).
        assert_eq!(page.audio_layout().title().y, 368.0);
        assert_eq!(page.input_layout().title().y, 512.0);
        let latency = page.latency_layout();
        assert_eq!(latency.title().y, 608.0);
        assert_eq!(latency.line(2).y + 24.0, 700.0);
        assert_eq!(
            latency.reset(),
            iced::Rectangle::new(iced::Point::new(12.0, 731.0), iced::Size::new(132.0, 18.0))
        );
        assert_eq!(
            latency.hit(iced::Point::new(20.0, 740.0)),
            Some(crate::ui::settings_layout::Hit::ResetDefaults)
        );
        let reset_words = ["Reset to defaults", "Press again to reset"]
            .map(|words| crate::ui::text_fit::text_width(words, crate::ui::fonts::size::LABEL));
        assert!(
            reset_words.iter().all(|&width| width + 16.0 <= 132.0),
            "{reset_words:?}"
        );
        assert_eq!(page.content_height(), 760.0);
        let element = page.view();
        assert_eq!(element.as_widget().size().height, Length::Fixed(736.0));
        let (mut app, _player, _main) = f1_app();
        let keys = crate::ui::keys_page::KeysPage {
            keys: &app.keys,
            layout: app.typing.layout,
            labels: &app.typing.labels,
            base: app.typing.base,
            rebinding: None,
        };
        let size = keys.view().as_widget().size();
        assert_eq!(size.width, Length::Fixed(692.0));
        assert_eq!(
            size.height,
            Length::Fixed(crate::ui::keys_page::content_height())
        );

        // The Theme page down to its colour's note and a gap, 324 (was 300 before the menu's row
        // over the colour wheel, 24 px).
        assert_eq!(crate::ui::theme_page::content_height(), 324.0);
        let theme = app.theme_page().view();
        assert_eq!(theme.as_widget().size().height, Length::Fixed(300.0));

        toggle(&mut app);
        for page in [
            SettingsPage::Keys,
            SettingsPage::Theme,
            SettingsPage::General,
        ] {
            let _ = app.settings_window_view();
            let at = tab(page).center();
            let _ = app.update(Message::Settings(SettingsMessage::Pressed(at)));
            assert_eq!(app.settings_page, page);
        }

        let least = crate::ui::settings_panel::WINDOW_MIN;
        let tabs = tab(SettingsPage::Theme);
        assert_eq!(tabs.x + tabs.width, 262.0);
        assert!(tabs.x + tabs.width <= least.width);
        let room = least.height - HINT_BAR_HEIGHT;
        for ports in [1, 0] {
            let layout = Layout {
                ports,
                width: least.width,
                ..Layout::default()
            };
            let first_group = layout.row(1);
            assert_eq!(first_group.y + first_group.height, 100.0);
            assert!(first_group.y + first_group.height <= room, "{layout:?}");
            assert!(layout.row(0).width <= least.width);
        }
        let keys = crate::ui::theme_page::keys_row();
        assert_eq!(keys.y + keys.height, 76.0);
        assert!(keys.y + keys.height <= room, "the Theme page's keys");
    }

    // The Theme page closes and comes back as the other pages do: its tab shows it, Esc in the
    // window closes it from there, the digit 6 and F10 in the main window open it again on the
    // Theme page (the page shown last is kept while the app runs), and each typed in the Settings
    // window closes it.
    #[test]
    fn the_theme_page_closes_and_comes_back_as_the_others_do() {
        let (mut app, _player, main) = f1_app();
        toggle(&mut app);
        let at = tab(SettingsPage::Theme).center();
        let _ = app.update(Message::Settings(SettingsMessage::Pressed(at)));
        assert_eq!(app.settings_page, SettingsPage::Theme);
        let id = app.settings_window.expect("open");
        press(&mut app, id, Key::Named(Named::Escape), Code::Escape);
        assert_eq!(open(&app), (false, false), "Esc in it closes it");
        for (key, code) in [
            (Key::Character("6".into()), Code::Digit6),
            (Key::Named(Named::F10), Code::F10),
        ] {
            press(&mut app, main, key.clone(), code);
            assert_eq!(open(&app), (true, true), "{code:?} opens it");
            assert_eq!(
                app.settings_page,
                SettingsPage::Theme,
                "the page shown last"
            );
            let id = app.settings_window.expect("open");
            press(&mut app, id, key, code);
            assert_eq!(open(&app), (false, false), "{code:?} in it closes it");
        }
    }

    // Keys typed in the Settings window do what they do in the main window, as the instrument
    // window's do: with the typing keyboard on, A plays the selected channel's C5. A press a
    // text box took passes nothing on but Esc, the rule every text box keeps, so a text box in
    // Settings would take its keys. With nothing under the cursor, or hints off, its foot line
    // says how to close it.
    #[test]
    fn keys_typed_in_the_settings_window_play_as_in_the_main_window() {
        use dawsome_engine::transport::Control;
        use iced::keyboard::{self, key};
        use iced::{event, Event};
        let (mut app, _player, _main) = f1_app();
        toggle(&mut app);
        let id = app.settings_window.expect("open");
        let pressed = |typed: Key| keyboard::Event::KeyPressed {
            key: typed.clone(),
            modified_key: typed,
            physical_key: key::Physical::Code(Code::KeyA),
            location: keyboard::Location::Standard,
            modifiers: Modifiers::empty(),
            text: None,
            repeat: false,
        };
        let sent = app.session.controls_sent().len();
        let message = super::super::key_input::key_event(
            Event::Keyboard(pressed(Key::Character("a".into()))),
            event::Status::Ignored,
            id,
        )
        .expect("a key of the typing keyboard");
        let _ = app.update(message);
        let velocity = super::super::typing::VELOCITY;
        assert_eq!(
            &app.session.controls_sent()[sent..],
            [Control::LiveNoteOn { key: 60, velocity }]
        );
        let taken = super::super::key_input::key_event(
            Event::Keyboard(pressed(Key::Character("a".into()))),
            event::Status::Captured,
            id,
        );
        assert!(taken.is_none(), "a text box's key goes nowhere else");
        let escape = super::super::key_input::key_event(
            Event::Keyboard(pressed(Key::Named(Named::Escape))),
            event::Status::Captured,
            id,
        );
        assert!(escape.is_some(), "Esc still closes it");
        assert_eq!(app.settings_hint(), "Esc or 6 or F10 closes Settings");
    }
}
