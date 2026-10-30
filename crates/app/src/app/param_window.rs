//! The parameter window's life in the app (it was a 360 px strip drawn over the
//! main window's right edge, under the toolbar, which covered the panels there and could be
//! neither moved nor made larger). It is a window of its own beside the main one, as the Help
//! window is: [`App::panel`] holds its list while it is open, and [`App::sync_param_window`]
//! opens and closes its window to match after every message, so every path that opens the list
//! (a plugin channel's menu's Parameters…, a mixer slot's menu, a plugin with no window of its
//! own, windows that cannot open) or closes it (its Close, Esc in it or in the main window, its
//! X, the system closing it, its plugin gone, another document) needs no task of its own. One
//! at a time: opening it on another plugin shows that plugin's list in the same window, from its
//! top. It keeps the pop-out windows' level (above the main window while the app has the
//! keyboard), and its size and place are kept in the settings (`params_window`) when it closes
//! and when the app closes with it open. Keys typed in it do what they do in the main window.

use std::time::Instant;

use dawsome_engine::project::ProcessorOwner;
use iced::widget::operation::{self, AbsoluteOffset};
use iced::{window, Element, Size, Task};

use super::instrument::pop_out_level;
use super::{App, DragTarget, Message, WindowChange};
use crate::desktop::{self, PopOut};
use crate::settings::WindowPlace;
use crate::ui::panels::Panel;
use crate::ui::param_window::{self as view, ParamMessage, SliderPress};

/// The parameter window's own window.
pub(super) struct ParamWindow {
    /// Its window while it is open or opening.
    pub window: Option<window::Id>,
    /// Where it opens: its size and position as it last was.
    pub place: WindowPlace,
    /// Its size as it last said, while it is open.
    pub size: Option<Size>,
    /// Whether its window was last asked to stay above every window.
    above: bool,
    /// The list was built anew (another plugin, or its rows changed): its scrollable is set to
    /// the list's scroll at the next sync, the widget keeping its own offset otherwise.
    scroll_due: bool,
    /// `--scroll end`: the list starts at its end once its window has opened.
    pub to_end: bool,
}

impl ParamWindow {
    /// Closed, opening at `place` (its first place when none is kept).
    pub fn new(place: Option<WindowPlace>) -> Self {
        Self {
            window: None,
            place: place.unwrap_or_else(|| WindowPlace::first(PopOut::Params)),
            size: None,
            above: false,
            scroll_due: false,
            to_end: false,
        }
    }

    /// The list was built anew: its scroll is the one to show.
    pub fn rebuilt(&mut self) {
        self.scroll_due = true;
    }
}

impl App {
    /// Opens the parameter window's own window while its list is open and closes it once it is
    /// not, keeping its place for the next opening; asks it to the app's level again when the
    /// app's windows were last asked to another; scrolls its list where the list says after the
    /// list was built anew.
    pub(super) fn sync_param_window(&mut self) -> Task<Message> {
        let level = pop_out_level(self.windows_above);
        let restack = self.params.above != self.windows_above;
        self.params.above = self.windows_above;
        let scroll = match (&self.panel, self.params.scroll_due) {
            // `--scroll end` waits for the window, whose list the scroll is of.
            (Some(_), true) if self.params.to_end && self.params.size.is_none() => Task::none(),
            (Some(_), true) if self.params.to_end => {
                self.params.to_end = false;
                self.params.scroll_due = false;
                operation::snap_to_end(view::LIST)
            }
            (Some(panel), true) => {
                self.params.scroll_due = false;
                operation::scroll_to(
                    view::LIST,
                    AbsoluteOffset {
                        x: None,
                        y: Some(panel.scroll),
                    },
                )
            }
            _ => Task::none(),
        };
        let window = match (self.panel.is_some(), self.params.window) {
            (true, None) => {
                let settings = desktop::pop_out_settings(
                    PopOut::Params,
                    self.params.place.size(PopOut::Params),
                    self.params.place.position(),
                    level,
                );
                let (id, opened) = window::open(settings);
                self.params.window = Some(id);
                opened.discard()
            }
            (false, Some(id)) => {
                self.params.window = None;
                self.params.size = None;
                self.remember_param_place();
                window::close(id)
            }
            (true, Some(id)) if restack => window::set_level(id, level),
            _ => Task::none(),
        };
        Task::batch([window, scroll])
    }

    /// The parameter window opened, was resized or moved: its place is kept.
    pub(super) fn param_window_changed(&mut self, change: WindowChange) {
        let place = &mut self.params.place;
        match change {
            WindowChange::Opened { size, position } => {
                self.params.size = Some(size);
                *place = WindowPlace::of(size, position.or(place.position()));
            }
            WindowChange::Resized(size) => {
                self.params.size = Some(size);
                *place = WindowPlace::of(size, place.position());
            }
            WindowChange::Moved(at) => {
                *place = WindowPlace::of(place.size(PopOut::Params), Some(at));
            }
            WindowChange::Rescaled(_) | WindowChange::Focus(_) => {}
        }
    }

    /// Keeps the parameter window's place in the settings for its next opening.
    pub(super) fn remember_param_place(&mut self) {
        let place = Some(self.params.place);
        if self.settings.params_window != place {
            self.settings.params_window = place;
            self.save_settings();
        }
    }

    /// Closes the parameter window: its Close, Esc, its X, or the system closing it.
    pub(super) fn close_params(&mut self) {
        self.panel = None;
    }

    /// Esc in the parameter window: the automation menu open over its list first, else the
    /// window.
    pub(super) fn escape_params(&mut self) {
        if self.automate.as_ref().is_some_and(|menu| menu.over_params) {
            self.automate = None;
        } else {
            self.close_params();
        }
    }

    /// Something the parameter window reported.
    pub(super) fn param_message(&mut self, message: ParamMessage) {
        let Some(panel) = &mut self.panel else {
            return;
        };
        match message {
            ParamMessage::Scrolled(scroll) => {
                if scroll != panel.scroll {
                    panel.scroll = scroll;
                    // The rows coming into view show their values as they are now.
                    self.refresh_panel();
                    // The menu over a row belongs to that row, which the scroll moves.
                    if self.automate.as_ref().is_some_and(|menu| menu.over_params) {
                        self.automate = None;
                    }
                }
            }
            ParamMessage::RightPressed(index) => {
                let owner = panel.owner;
                let Some(at) = panel.menu_at(index) else {
                    return;
                };
                // The menu belongs to the panel the plugin's control lives in.
                let home = match owner {
                    ProcessorOwner::Channel(_) => Panel::Rack,
                    ProcessorOwner::Effect(_) => Panel::Mixer,
                };
                self.open_plugin_automate_menu((owner, index), home, true, at);
            }
            ParamMessage::Pressed(index) => {
                let Some(SliderPress { index, double }) = panel.press(index, Instant::now()) else {
                    return;
                };
                let owner = panel.owner;
                let default = panel.params.get(index).map(|param| param.default as f32);
                let Some(drag) = self.plugin_drag(owner, index) else {
                    return;
                };
                match (double, default) {
                    (true, Some(default)) => {
                        self.set_plugin_param(drag, default);
                        self.capture_soon(owner);
                    }
                    _ => self.start_drag(DragTarget::Plugin(drag)),
                }
            }
            ParamMessage::Close => self.close_params(),
        }
    }

    /// What the parameter window shows: its list while it is open, nothing in the moment
    /// between its closing and its window going.
    pub(super) fn param_window_view(&self) -> Element<'_, Message> {
        let Some(panel) = &self.panel else {
            return iced::widget::container(iced::widget::space())
                .width(iced::Fill)
                .height(iced::Fill)
                .style(crate::ui::style::panel)
                .into();
        };
        let size = self.param_window_size();
        view::view(view::View {
            panel,
            menu: self.params_automate_menu_view(crate::ui::instrument_window::HINT_HEIGHT),
            size,
            hint: self.param_hint(),
        })
    }

    /// The parameter window's size: as it last said, else (before it says) the size it opens at.
    pub(super) fn param_window_size(&self) -> Size {
        self.params
            .size
            .unwrap_or_else(|| self.params.place.size(PopOut::Params))
    }

    /// The parameter window's foot line: its gestures, and how to close it.
    pub(super) fn param_hint(&self) -> String {
        "Drag a slider (Shift: fine) \u{b7} double-click: its default \u{b7} right-click: automate \
         \u{b7} Esc closes"
            .to_owned()
    }

    /// `--open-params`: the parameter window opened on `owner`'s plugin at start, as the channel
    /// menu's Parameters… opens it (its window follows at the start's sync); the run fails when
    /// `owner` plays no plugin.
    pub fn open_params_at_start(&mut self, owner: ProcessorOwner) -> Task<Message> {
        self.open_panel(owner);
        if self.panel.is_none() {
            let refused = format!("--open-params: {owner:?} has no plugin to list");
            return self.headless.fail(&refused).map(Message::Headless);
        }
        self.dump_params()
    }

    /// `--dump-params`: the parameter window's list written out, while it is open; the run
    /// fails when the file cannot be written.
    pub(super) fn dump_params(&self) -> Task<Message> {
        let Some((path, panel)) = self.params_dump.as_ref().zip(self.panel.as_ref()) else {
            return Task::none();
        };
        match std::fs::write(path, panel.dump()) {
            Ok(()) => Task::none(),
            Err(error) => {
                let refused = format!("cannot write {}: {error}", path.display());
                self.headless.fail(&refused).map(Message::Headless)
            }
        }
    }

    /// The parameter window's title, the one the plugin's own window carries:
    /// "<channel or slot> — <plugin>".
    pub(super) fn param_title(&self) -> String {
        self.panel
            .as_ref()
            .map(|panel| panel.title.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use dawsome_engine::processor::{
        Captured, Editor, HostError, HostNotice, PluginParam, Processor, ProcessorHost,
    };
    use dawsome_engine::project::{ChannelId, InsertId, ProcessorRef};
    use dawsome_engine::realtime::Player;
    use dawsome_host_clap::catalogue::PluginKind;
    use dawsome_host_clap::ClapHost;
    use iced::Point;

    use super::super::plugins::app_tests::{
        app_hosted, catalogue_of, channel_menu_row, test_plugin_file,
    };
    use super::super::plugins::host_of;
    use super::super::ChannelCommand;
    use super::*;
    use crate::document::Host;
    use crate::ui::hint::{ChannelRow, Hover, RackPart};
    use crate::ui::picker::PickerTarget;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");
    const SINE: ProcessorOwner = ProcessorOwner::Channel(ChannelId(1));

    /// An app on `fixture` hosting the CLAP test plugins, offline, with `host`'s settings.
    fn app_on(fixture: &str, host: Host) -> (App, Player) {
        let host = Host {
            clap: ClapHost::new(catalogue_of(test_plugin_file())),
            ..host
        };
        app_hosted(&Path::new(FIXTURES).join(fixture), host)
    }

    /// The test sine's app told its main window's id, as main tells it.
    fn sine_app() -> (App, Player, window::Id) {
        let (mut app, player) = app_on("p7_sine.json", Host::offline());
        let main = window::Id::unique();
        app.set_main_window(main);
        (app, player, main)
    }

    /// The sine channel's menu's "Parameters…", clicked through the main window's top layer.
    fn open_from_menu(app: &mut App) -> window::Id {
        let message = channel_menu_row(app, ChannelId(1), ChannelRow::Parameters);
        let _ = app.update(message);
        app.params.window.expect("a window of its own")
    }

    fn open(app: &App) -> (bool, bool) {
        (app.panel.is_some(), app.params.window.is_some())
    }

    /// The test gain added to the master's first slot, as the picker adds it: its effect.
    fn add_gain(app: &mut App) -> ProcessorOwner {
        let gain = app
            .host
            .catalogue()
            .offered(PluginKind::Effect)
            .into_iter()
            .find(|plugin| plugin.plugin.name == "Dawsome Test Gain")
            .expect("listed");
        let id = app.session.project().mixer.next_effect_id;
        let target = PickerTarget::Effect {
            insert: InsertId(0),
            slot: 0,
        };
        app.add_plugin(target, &gain);
        assert_eq!(app.error, None);
        ProcessorOwner::Effect(id)
    }

    // The parameter window is a window of its own, one at a time, titled as the
    // plugin's own window is ("<channel or slot> — <plugin>") and headed by the place and
    // "<plugin> · by <maker>". Opened on another plugin (an effect's, from its slot's menu) it
    // is the same window, retitled, its list from the top. Its Close, Esc in it, its X and the
    // system closing it each close it alone; Esc in the main window closes the picker first,
    // then it. The main window's X closes the app (nothing unsaved), the window with it.
    #[test]
    fn the_parameter_window_is_one_window_of_its_own() {
        let (mut app, _player, main) = sine_app();
        assert_eq!(open(&app), (false, false));
        let id = open_from_menu(&mut app);
        assert_ne!(Some(id), app.main_window);
        assert_eq!(
            app.window_title(id),
            "Dawsome Test Sine \u{2014} Dawsome Test Sine"
        );
        assert_eq!(app.window_title(main), app.title());
        let panel = app.panel.as_ref().expect("its list");
        assert_eq!(
            (panel.place.as_str(), panel.about.as_str()),
            ("Dawsome Test Sine", "Dawsome Test Sine \u{b7} by Dawsome")
        );
        assert_eq!(panel.labels, ["Gain"]);
        let _ = app.update(Message::Params(ParamMessage::Scrolled(48.0)));
        assert_eq!(app.panel.as_ref().map(|panel| panel.scroll), Some(48.0));

        let gain = add_gain(&mut app);
        app.open_panel(gain);
        let _ = app.update(Message::PanelTick);
        assert_eq!(app.params.window, Some(id), "the same window");
        assert_eq!(
            app.window_title(id),
            "Master slot 1 \u{2014} Dawsome Test Gain"
        );
        let panel = app.panel.as_ref().expect("the gain's list");
        assert_eq!(panel.owner, gain);
        assert_eq!(panel.place, "Master slot 1");
        assert_eq!(panel.scroll, 0.0, "from its top");

        let _ = app.update(Message::Params(ParamMessage::Close));
        assert_eq!(open(&app), (false, false), "its Close");
        let id = open_from_menu(&mut app);
        let _ = app.update(Message::Escape(id));
        assert_eq!(open(&app), (false, false), "Esc in it");

        let id = open_from_menu(&mut app);
        let _ = app.update(Message::OpenPicker(PickerTarget::AddInstrument));
        let _ = app.update(Message::Escape(main));
        assert!(app.picker.is_none(), "Esc in the main window: its picker");
        assert_eq!(app.params.window, Some(id), "left open");
        let _ = app.update(Message::Escape(main));
        assert_eq!(open(&app), (false, false), "then it");

        let id = open_from_menu(&mut app);
        let _ = app.update(Message::WindowCloseRequested(id));
        assert_eq!(open(&app), (false, false), "its X");
        assert!(!app.exiting(), "closes it alone");
        let id = open_from_menu(&mut app);
        let _ = app.update(Message::WindowClosed(id));
        assert_eq!(open(&app), (false, false), "the system closing it");
        let _ = app.update(Message::WindowCloseRequested(id));
        assert!(!app.exiting(), "a request from a window gone asks nothing");
        assert!(app.files.modal.is_none());

        let (mut app, _player, main) = sine_app();
        let _ = open_from_menu(&mut app);
        let _ = app.update(Message::WindowCloseRequested(main));
        assert!(app.exiting(), "the main window's X closes the app");
    }

    // It floats above the main window while one of the app's windows has the keyboard, the
    // parameter window's own too, and is stacked as any window once another program has it.
    #[test]
    fn the_parameter_window_stays_above_while_the_app_has_the_keyboard() {
        let (mut app, _player, main) = sine_app();
        let focus = |app: &mut App, id, focused| {
            let _ = app.update(Message::Windowed(id, WindowChange::Focus(focused)));
        };
        focus(&mut app, main, true);
        let id = open_from_menu(&mut app);
        assert!(app.params.above, "opened above");
        focus(&mut app, main, false);
        focus(&mut app, id, true);
        assert!(app.params.above, "its own keyboard: still above");
        focus(&mut app, id, false);
        assert!(!app.params.above, "another program has it");
        focus(&mut app, main, true);
        assert!(app.params.above);
    }

    // Its size and place are kept in the settings when it closes and when the app closes with it
    // open, and it opens there again, in a later start too.
    #[test]
    fn the_parameter_windows_size_and_place_are_kept_in_the_settings() {
        use crate::settings::Settings;
        let dir = std::env::temp_dir().join(format!("dawsome-params-place-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("dawsome/settings.json");
        let mut host = Host::offline();
        host.settings_file = Some(file.clone());
        host.saves_settings = true;
        let (mut app, _player) = app_on("p7_sine.json", host.clone());
        assert_eq!(
            app.params.place.size(PopOut::Params),
            view::WINDOW_SIZE,
            "its first size"
        );
        let main = window::Id::unique();
        app.set_main_window(main);
        let id = open_from_menu(&mut app);
        let changed = |app: &mut App, change| {
            let _ = app.update(Message::Windowed(id, change));
        };
        changed(&mut app, WindowChange::Resized(Size::new(600.0, 700.0)));
        changed(&mut app, WindowChange::Moved(Point::new(30.0, 40.0)));
        assert_eq!(app.params.size, Some(Size::new(600.0, 700.0)));
        let _ = app.update(Message::Escape(id));
        assert_eq!(
            Settings::load(Some(&file)).params_window,
            Some(WindowPlace::of(
                Size::new(600.0, 700.0),
                Some(Point::new(30.0, 40.0))
            ))
        );
        let id = open_from_menu(&mut app);
        let _ = app.update(Message::Windowed(
            id,
            WindowChange::Resized(Size::new(640.0, 520.0)),
        ));
        let _ = app.update(Message::WindowCloseRequested(main));
        assert!(app.exiting());
        let saved = Settings::load(Some(&file)).params_window;
        assert_eq!(
            saved.map(|place| place.size(PopOut::Params)),
            Some(Size::new(640.0, 520.0))
        );
        let (later, _player) = app_on("p7_sine.json", host);
        assert_eq!(
            later.params.place.size(PopOut::Params),
            Size::new(640.0, 520.0)
        );
        assert_eq!(later.params.place.position(), Some(Point::new(30.0, 40.0)));
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // It closes when its plugin goes: its channel deleted, and the document replaced (File →
    // New; another document's channel 1 is another plugin or none).
    #[test]
    fn the_parameter_window_closes_when_its_plugin_goes() {
        let (mut app, _player, _main) = sine_app();
        let _ = open_from_menu(&mut app);
        let _ = app.update(Message::Channel(ChannelCommand::Delete(ChannelId(1))));
        assert_eq!(open(&app), (false, false), "its channel deleted");
        let _ = app.update(Message::Undo);
        let _ = open_from_menu(&mut app);
        let _ = app.update(Message::File(crate::app::FileMessage::New));
        assert_eq!(open(&app), (false, false), "another document");
    }

    // A right press on a row opens its automation menu over the list, under the row's slider
    // (gen_p7's sine: "Create automation clip" for channel 1's param 0). Esc in the window
    // closes the menu first, then the window; a scroll of the list closes it, a report of the
    // offset it has already leaves it.
    #[test]
    fn a_rows_automation_menu_opens_over_the_list() {
        let (mut app, _player, _main) = sine_app();
        let id = open_from_menu(&mut app);
        let _ = app.update(Message::Params(ParamMessage::RightPressed(0)));
        let menu = app.automate.as_ref().expect("the row's menu");
        assert!(menu.over_params);
        assert_eq!(
            Some(menu.at),
            app.panel.as_ref().and_then(|panel| panel.menu_at(0))
        );
        assert!(app.params_automate_menu_view(0.0).is_some());
        assert!(
            app.automate_menu_view(Panel::Rack).is_none(),
            "not in the main window"
        );
        let _ = app.update(Message::Escape(id));
        assert!(app.automate.is_none(), "Esc: the menu first");
        assert_eq!(open(&app), (true, true));
        let _ = app.update(Message::Params(ParamMessage::RightPressed(0)));
        let _ = app.update(Message::Params(ParamMessage::Scrolled(0.0)));
        assert!(app.automate.is_some(), "no scroll: kept");
        let _ = app.update(Message::Params(ParamMessage::Scrolled(24.0)));
        assert!(app.automate.is_none(), "scrolled: closed");
    }

    // A double press on a slider sets its default: the sine's Gain at 0.25
    // ('T1RQMQAAAAAAANA/', one undo step) pressed twice within 400 ms is sent its default, 1
    // (crates/test-plugin's doc), shown at once, and captured as one more undo step:
    // 'T1RQMQAAAAAAAPA/', "OTP1" and 1.0 as a little-endian f64, by hand (Python's struct and
    // base64).
    #[test]
    fn a_double_press_on_a_slider_sets_its_default() {
        let (mut app, mut player, _main) = sine_app();
        let _ = open_from_menu(&mut app);
        let gain = app.plugin_drag(SINE, 0).expect("Gain moves");
        app.set_plugin_param(gain, 0.25);
        app.capture_soon(SINE);
        settle(&mut app, &mut player);
        assert_eq!(app.session.history().undo_len(), 1);
        let state = |app: &App| app.plugin_reference(SINE).and_then(|r| r.state.clone());
        assert_eq!(state(&app).as_deref(), Some("T1RQMQAAAAAAANA/"));
        for _ in 0..2 {
            let _ = app.update(Message::Params(ParamMessage::Pressed(0)));
            let _ = app.update(Message::DragEnded);
        }
        assert_eq!(app.panel.as_ref().map(|panel| panel.values[0]), Some(1.0));
        settle(&mut app, &mut player);
        assert_eq!(state(&app).as_deref(), Some("T1RQMQAAAAAAAPA/"));
        assert_eq!(app.session.history().undo_len(), 2);
        let host = host_of(&app.session, SINE).expect("its host");
        assert_eq!(host.value(SINE, 0), Some(1.0));
    }

    /// The stopped player's idle call hands the plugin the values sent, then the captures due
    /// are taken.
    fn settle(app: &mut App, player: &mut Player) {
        player.process(&mut [[0.0; 2]; 64]);
        for when in app.captures.values_mut() {
            *when = Instant::now();
        }
        let _ = app.update(Message::PluginTick);
    }

    // A right-click on a plugin channel's name opens the channel's menu,
    // as a sampler's does, with "Plugin window…" and "Parameters…" after "Instrument window…";
    // its hint says so. "Plugin window…" does what a double press on the name does: the test
    // sine has no window of its own, so its parameter window opens. An audio channel's menu is
    // the rows every kind has.
    #[test]
    fn a_plugin_channels_menu_offers_its_window_and_its_parameters() {
        use ChannelRow as R;
        let (mut app, _player, _main) = sine_app();
        app.open_channel_menu(ChannelId(1));
        let rows: Vec<ChannelRow> = app
            .channel_menu_rows()
            .into_iter()
            .map(|row| row.0)
            .collect();
        assert_eq!(
            rows,
            [
                R::InstrumentWindow,
                R::PluginWindow,
                R::Parameters,
                R::MixerInsert,
                R::Rename,
                R::Clone,
                R::Delete
            ]
        );
        let name = Hover::Rack {
            row: 0,
            part: RackPart::Name,
        };
        let words = crate::ui::hint::words(&name, &app.hint_context());
        assert!(
            words.ends_with("double-click: its plugin's window \u{b7} right-click: menu (its instrument window there)"),
            "{words}"
        );
        let message = channel_menu_row(&mut app, ChannelId(1), R::PluginWindow);
        assert!(
            matches!(message, Message::ChannelOpened(ChannelId(1))),
            "{message:?}"
        );
        let _ = app.update(message);
        assert_eq!(app.panel.as_ref().map(|panel| panel.owner), Some(SINE));
        assert!(app.params.window.is_some());

        let (mut app, _player) = app_on("p8_ui.json", Host::offline());
        app.open_channel_menu(ChannelId(1));
        let rows: Vec<ChannelRow> = app
            .channel_menu_rows()
            .into_iter()
            .map(|row| row.0)
            .collect();
        // An audio channel's: its sound in the sample editor too.
        assert_eq!(
            rows,
            [
                R::InstrumentWindow,
                R::MixerInsert,
                R::EditSample,
                R::Rename,
                R::Clone,
                R::Delete
            ]
        );
    }

    /// A plugin's parameters as a host lists them: as many as it holds (ids from 0, each 0..1
    /// and at 0.25), each one's text its id and value ("7: 0.25"), but the text of every seventh
    /// (ids 6, 13, ...) refused.
    struct Listed(u32);

    impl ProcessorHost for Listed {
        fn format(&self) -> &'static str {
            "listed"
        }

        fn create(
            &self,
            _owner: ProcessorOwner,
            _reference: &ProcessorRef,
            _sample_rate: u32,
        ) -> Result<Box<dyn Processor>, HostError> {
            Err(HostError::Missing)
        }

        fn release(&self, _owner: ProcessorOwner) {}

        fn sync_state(&self, _owner: ProcessorOwner, _reference: &ProcessorRef) {}

        fn params(&self, _owner: ProcessorOwner) -> Vec<PluginParam> {
            (0..self.0)
                .map(|id| PluginParam {
                    id,
                    name: format!("Param {id}"),
                    group: String::new(),
                    min: 0.0,
                    max: 1.0,
                    default: 0.5,
                    stepped: false,
                    hidden: false,
                    read_only: false,
                })
                .collect()
        }

        fn value(&self, _owner: ProcessorOwner, id: u32) -> Option<f64> {
            (id < self.0).then_some(0.25)
        }

        fn value_text(&self, _owner: ProcessorOwner, id: u32, value: f64) -> Option<String> {
            (id % 7 != 6).then(|| format!("{id}: {value}"))
        }

        fn capture_state(&self, _owner: ProcessorOwner) -> Result<Captured, HostError> {
            Err(HostError::Missing)
        }

        fn open_editor(&self, _owner: ProcessorOwner, _title: &str) -> Result<Editor, HostError> {
            Ok(Editor::Generic)
        }

        fn tick(&self) -> Vec<HostNotice> {
            Vec::new()
        }

        fn has_instances(&self) -> bool {
            false
        }
    }

    /// The parameter window opened on a list of `count` parameters ([`Listed`]), at its first
    /// size, and its widgets; the list's channel plays no plugin, so the app reads no values.
    fn listed_window(count: u32) -> (App, Player, super::super::widget_events::Window) {
        let (mut app, player, _main) = sine_app();
        let heading = view::Heading {
            title: "Listed \u{2014} Listed".into(),
            place: "Listed".into(),
            about: "Listed \u{b7} by Nobody".into(),
        };
        let owner = ProcessorOwner::Channel(ChannelId(9));
        app.panel = Some(view::ParamPanel::new(
            owner,
            heading,
            &Listed(count),
            48_000,
        ));
        app.params.rebuilt();
        let _ = app.update(Message::PanelTick);
        let id = app.params.window.expect("its window");
        let opened = WindowChange::Opened {
            size: view::WINDOW_SIZE,
            position: None,
        };
        let _ = app.update(Message::Windowed(id, opened));
        let widgets = super::super::widget_events::Window::pop_out(&app, id, view::WINDOW_SIZE);
        (app, player, widgets)
    }

    /// The scrolls the window reported among `messages`.
    fn scrolls(messages: &[Message]) -> Vec<f32> {
        messages
            .iter()
            .filter_map(|message| match message {
                Message::Params(ParamMessage::Scrolled(scroll)) => Some(*scroll),
                _ => None,
            })
            .collect()
    }

    // The list under the wheel, through the window's own widgets (the CPU renderer), each turn
    // three lines (iced's 60 px a line: 180 px), 200 turns down then 200 up, with the window's
    // timer between (a guard written hunting the 2026-10-27 crash "opened the parameters, scrolled
    // down", which none of these reproduced). None: the list's line, and no scroll. Two rows fit:
    // no scroll reported. 1,150 rows (27,600 px; JS80P lists that many before its MIDI mapping
    // hides most): 180, 360, ... by 180 px a turn, stopping short of 27,600 by less than the
    // window's 480 px, the last report building the last row; more turns there report nothing;
    // back up to 0. Row 6's text the plugin refuses: its value to three places, "0.250"; row 7's
    // its own, "7: 0.25".
    #[test]
    fn the_wheel_scrolls_the_parameter_list_to_its_end_and_back() {
        let over_list = Point::new(200.0, view::HEADING_HEIGHT + 100.0);
        for count in [0u32, 2] {
            let (mut app, _player, mut widgets) = listed_window(count);
            let mut reported = Vec::new();
            for turn in 0..400 {
                let lines = if turn < 200 { -3.0 } else { 3.0 };
                reported.extend(scrolls(&widgets.wheel(&mut app, over_list, lines)));
                let _ = app.update(Message::PanelTick);
            }
            assert!(reported.is_empty(), "{count} rows: {reported:?}");
            assert_eq!(app.panel.as_ref().map(|panel| panel.scroll), Some(0.0));
        }

        let (mut app, _player, mut widgets) = listed_window(1_150);
        let panel = app.panel.as_ref().expect("its list");
        assert_eq!(panel.items.len(), 1_150);
        assert_eq!(
            (panel.texts[6].as_str(), panel.texts[7].as_str()),
            ("0.250", "7: 0.25")
        );
        let mut down = Vec::new();
        for _ in 0..200 {
            down.extend(scrolls(&widgets.wheel(&mut app, over_list, -3.0)));
            let _ = app.update(Message::PanelTick);
        }
        let end = *down.last().expect("it scrolled");
        assert_eq!(down[..3], [180.0, 360.0, 540.0]);
        let whole = 1_150.0 * view::ROW_HEIGHT;
        assert!(
            end < whole && end > whole - view::WINDOW_SIZE.height,
            "{end}"
        );
        let (last, turns) = down.split_last().expect("reports");
        let whole_turns = turns.windows(2).all(|pair| pair[1] - pair[0] == 180.0);
        let short_turn = last - turns.last().copied().unwrap_or_default();
        assert!(whole_turns && short_turn <= 180.0, "{down:?}");
        assert_eq!(app.panel.as_ref().map(|panel| panel.scroll), Some(end));
        let built = view::shown(1_150, end, view::WINDOW_SIZE.height);
        assert_eq!(built.end, 1_150, "the last row built");
        assert!(
            scrolls(&widgets.wheel(&mut app, over_list, -3.0)).is_empty(),
            "past its end"
        );
        let mut up = Vec::new();
        for _ in 0..200 {
            up.extend(scrolls(&widgets.wheel(&mut app, over_list, 3.0)));
            let _ = app.update(Message::PanelTick);
        }
        assert_eq!(up.last(), Some(&0.0));
        assert_eq!(app.panel.as_ref().map(|panel| panel.scroll), Some(0.0));
    }
}
