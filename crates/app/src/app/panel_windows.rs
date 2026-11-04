//! The stacked panels popped out of the main window, each shown in a window of its own beside
//! it: a divider dragged sideways pops the panel under it ([`super::dividers`]). The panel stays
//! open: `App::panels.popped` says which are popped, the stack lays out the others as though it
//! were closed, and [`App::sync_panel_windows`] opens and closes each panel's window to match
//! after every message, as the other pop-out windows' do, at their level (above the main window
//! while the app has the keyboard: on X11 and Windows; a Wayland compositor stacks them as it
//! likes, `desktop::window_system`). It shows the panel's own view, as docked, at the window's
//! size; keys typed in it do what they do in the main window, its panel taking the editing keys
//! while it has the keyboard.
//!
//! Closing its window (its X, the system closing it, or the panel's key or toolbar button while
//! that window has the keyboard) puts the panel back in the stack at the height kept for it;
//! the key or button from another window brings it to the front instead. Nothing else docks it.
//! Its size and position are kept in the settings (`panel_windows`) when it closes and when the
//! app closes, and a panel popped as the app closes opens popped at the next start (the window
//! layout's `popped`).

use iced::widget::{column, container};
use iced::{window, Element, Fill, Point, Size, Task};

use super::instrument::pop_out_level;
use super::{App, Message, WindowChange};
use crate::desktop::{self, PopOut};
use crate::settings::{Settings, WindowPlace};
use crate::ui::panels::{self, Panel};
use crate::ui::{browser, mixer_layout, style};

/// A popped panel's own window.
#[derive(Debug, Clone, Copy)]
struct PanelWindow {
    /// Its window while it is open or opening.
    window: Option<window::Id>,
    /// Where it opens: its size and position as it last was.
    place: WindowPlace,
}

/// The four stacked panels' own windows, in [`Panel::STACKED`]'s order.
pub(super) struct PanelWindows {
    windows: [PanelWindow; 4],
    /// Whether the open ones were last asked to stay above every window (see
    /// [`App::sync_panel_windows`]).
    above: bool,
}

impl PanelWindows {
    /// Closed, each opening where `settings` kept it, else at its first size where the system
    /// puts it.
    pub fn new(settings: &Settings) -> Self {
        let windows = Panel::STACKED.map(|panel| PanelWindow {
            window: None,
            place: settings
                .panel_window(panel)
                .unwrap_or_else(|| WindowPlace::first(PopOut::Panel(panel))),
        });
        Self {
            windows,
            above: false,
        }
    }

    fn of(&self, panel: Panel) -> Option<&PanelWindow> {
        self.windows.get(panel.stacked_index()?)
    }

    fn of_mut(&mut self, panel: Panel) -> Option<&mut PanelWindow> {
        self.windows.get_mut(panel.stacked_index()?)
    }

    /// The panel whose window is `id`, if one's is.
    pub fn panel_of(&self, id: window::Id) -> Option<Panel> {
        Panel::STACKED
            .into_iter()
            .zip(&self.windows)
            .find(|(_, kept)| kept.window == Some(id))
            .map(|(panel, _)| panel)
    }

    /// `panel`'s window, while it is open or opening.
    pub fn window(&self, panel: Panel) -> Option<window::Id> {
        self.of(panel)?.window
    }

    /// Each stacked panel's window, in [`Panel::STACKED`]'s order.
    pub fn ids(&self) -> [Option<window::Id>; 4] {
        self.windows.map(|kept| kept.window)
    }

    /// Where `panel`'s window opens, or is now.
    pub fn place(&self, panel: Panel) -> Option<WindowPlace> {
        Some(self.of(panel)?.place)
    }
}

impl App {
    /// Opens each popped panel's window while it is popped and closes it once it is not,
    /// keeping its place for the next opening; asks the open ones to the app's level again when
    /// the app's windows were last asked to another (`windows_above`, which
    /// [`App::sync_windows`] keeps).
    pub(super) fn sync_panel_windows(&mut self) -> Task<Message> {
        let level = pop_out_level(self.windows_above);
        let restack = self.panel_windows.above != self.windows_above;
        self.panel_windows.above = self.windows_above;
        let mut tasks = Vec::new();
        for panel in Panel::STACKED {
            let popped = self.panels.popped.has(panel);
            let Some(kept) = self.panel_windows.of_mut(panel) else {
                continue;
            };
            let task = match (popped, kept.window) {
                (true, None) => {
                    let pop_out = PopOut::Panel(panel);
                    let settings = desktop::pop_out_settings(
                        pop_out,
                        kept.place.size(pop_out),
                        kept.place.position(),
                        level,
                    );
                    let (id, opened) = window::open(settings);
                    kept.window = Some(id);
                    opened.discard()
                }
                (false, Some(id)) => {
                    kept.window = None;
                    self.remember_panel_place(panel);
                    window::close(id)
                }
                (true, Some(id)) if restack => window::set_level(id, level),
                _ => Task::none(),
            };
            tasks.push(task);
        }
        Task::batch(tasks)
    }

    /// `panel`'s window opened, was resized or moved: its place is kept, and a popped mixer's
    /// desk is laid out at once from its new height.
    pub(super) fn panel_window_changed(&mut self, panel: Panel, change: WindowChange) {
        let Some(kept) = self.panel_windows.of_mut(panel) else {
            return;
        };
        let place = &mut kept.place;
        match change {
            WindowChange::Opened { size, position } => {
                *place = WindowPlace::of(size, position.or(place.position()));
            }
            WindowChange::Resized(size) => *place = WindowPlace::of(size, place.position()),
            WindowChange::Moved(at) => {
                *place = WindowPlace::of(place.size(PopOut::Panel(panel)), Some(at));
            }
            WindowChange::Rescaled(_) | WindowChange::Focus(_) => return,
        }
        if panel == Panel::Mixer {
            self.lay_out_mixer_desk();
        }
    }

    /// `panel`'s window was asked to close (its X), or is gone already (`gone`: the system
    /// closed it): the panel goes back in the stack and its window's place is kept.
    pub(super) fn panel_window_closed(&mut self, panel: Panel, gone: bool) {
        if gone {
            if let Some(kept) = self.panel_windows.of_mut(panel) {
                kept.window = None;
            }
            self.remember_panel_place(panel);
        }
        self.dock(panel);
    }

    /// Keeps `panel`'s window's place in the settings for its next opening.
    fn remember_panel_place(&mut self, panel: Panel) {
        let Some(place) = self.panel_windows.place(panel) else {
            return;
        };
        if self.settings.panel_window(panel) != Some(place) {
            self.settings.set_panel_window(panel, place);
            self.save_settings();
        }
    }

    /// The popped panels' windows' places, as the settings keep them while the app closes:
    /// whether any changed.
    pub(super) fn keep_panel_places(&mut self) -> bool {
        let mut changed = false;
        for panel in Panel::STACKED {
            let open = self.panel_windows.window(panel).is_some();
            let place = self.panel_windows.place(panel);
            if let (true, Some(place)) = (open, place) {
                if self.settings.panel_window(panel) != Some(place) {
                    self.settings.set_panel_window(panel, place);
                    changed = true;
                }
            }
        }
        changed
    }

    /// Pops `panel`, open in the stack, out into its own window, the window's top-left corner
    /// at main window point `at` (where a divider was let go of) on the screen, where the system
    /// tells the app where the main window is (X11 and Windows; on Wayland the compositor
    /// places it). Its size: as it last was, else as the stack showed the panel. The others
    /// share the stack as though it were closed.
    pub(super) fn pop_out(&mut self, panel: Panel, at: Point) {
        if !self.panels.in_stack(panel) {
            return;
        }
        let pop_out = PopOut::Panel(panel);
        let size = match self.settings.panel_window(panel) {
            Some(kept) => kept.size(pop_out),
            None => self.docked_size(panel),
        };
        let position = self.main_position.map(|main| main + (at - Point::ORIGIN));
        if let Some(kept) = self.panel_windows.of_mut(panel) {
            kept.place = WindowPlace::of(size, position);
        }
        self.panels.pop_out(panel);
        self.focus = panel;
        self.panel_moved();
    }

    /// Puts the popped `panel` back in the stack, where it stacks and at the height kept for
    /// it: its window closes ([`App::sync_panel_windows`]). The others make room as for any
    /// height kept; none closes (a stack too short scrolls).
    pub(super) fn dock(&mut self, panel: Panel) {
        if !self.panels.popped.has(panel) {
            return;
        }
        self.panels.popped.set(panel, false);
        self.focus = panel;
        self.panel_moved();
    }

    /// The key or toolbar button of the popped `panel`: from its own window, which has the
    /// keyboard, the panel goes back in the stack; from any other window, its window comes to
    /// the front (where the window system lets an app ask that) and the panel takes the keys.
    pub(super) fn toggle_popped(&mut self, panel: Panel) -> Task<Message> {
        let window = self.panel_windows.window(panel);
        if window.is_some() && window == self.focused_window {
            self.dock(panel);
            return Task::none();
        }
        self.focus = panel;
        window.map_or_else(Task::none, window::gain_focus)
    }

    /// A panel moved between the stack and a window of its own: what the panels drew is drawn
    /// again at their new sizes, and the mixer's desk is laid out from where it shows now.
    fn panel_moved(&mut self) {
        self.clear_panel_caches();
        self.rack_cache.clear();
        self.lay_out_mixer_desk();
    }

    /// Lays the mixer's desk out from the height it shows now, so a press before its canvas
    /// reports its size finds what is drawn.
    fn lay_out_mixer_desk(&mut self) {
        let desk = mixer_layout::desk_height(self.mixer_panel_height());
        self.mixer.set_desk_height(desk);
    }

    /// The size the stack shows `panel` at: the stack's width, and the height it is laid out
    /// at; none's height when it is not in the stack.
    fn docked_size(&self, panel: Panel) -> Size {
        let stacked: Vec<Panel> = self.panels.stacked().collect();
        let heights = panels::stack_heights(
            &stacked,
            self.rack_content(),
            &self.settings.kept_heights(),
            self.panels_height(),
        );
        let height = stacked
            .iter()
            .zip(heights)
            .find(|&(&shown, _)| shown == panel)
            .map_or(0.0, |(_, height)| height.floor());
        Size::new(self.stack_width(), height)
    }

    /// The stack's width: the window's, less the Library's column while it is open.
    fn stack_width(&self) -> f32 {
        let library = if self.panels.browser {
            browser::WIDTH
        } else {
            0.0
        };
        (self.window_size.width - library).max(0.0)
    }

    /// The height of the popped mixer's window, its top edge and desk: what its desk is laid
    /// out from; none while the mixer is not popped.
    pub(super) fn popped_mixer_height(&self) -> Option<f32> {
        if !self.panels.popped.has(Panel::Mixer) {
            return None;
        }
        let place = self.panel_windows.place(Panel::Mixer)?;
        Some(place.size(PopOut::Panel(Panel::Mixer)).height)
    }

    /// What `panel`'s own window shows: the panel's view as docked, filling the window; the
    /// mixer under its plain top edge, as when it is alone in the stack.
    pub(super) fn panel_window_view(&self, panel: Panel) -> Element<'_, Message> {
        let view = match panel {
            Panel::Rack => self.rack_well().height(Fill).into(),
            Panel::Playlist => self.playlist_view(),
            Panel::PianoRoll => self.piano_roll_view(),
            Panel::Mixer => {
                let height = self.mixer_panel_height();
                let mut view = column![];
                view = view.extend(self.divider_view(None, Panel::Mixer));
                view.push(self.mixer_view(height)).into()
            }
            Panel::Browser | Panel::Settings => {
                unreachable!("the Library and Settings are never popped")
            }
        };
        container(view)
            .width(Fill)
            .height(Fill)
            .style(style::window)
            .into()
    }

    /// `panel`'s window's title, the other windows' way: "Piano roll - Dawsome".
    pub(super) fn panel_window_title(panel: Panel) -> String {
        format!("{} - Dawsome", panel.title())
    }

    /// How far above the window's foot the menus over `panel` stop: the hint bar's height in the
    /// main window, nothing in a popped panel's own window, which has no hint bar.
    pub(super) fn menu_foot(&self, panel: Panel) -> f32 {
        if self.panels.popped.has(panel) {
            0.0
        } else {
            self.hint_bar_height()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use dawsome_engine::realtime::Player;
    use iced::keyboard::key::{Code, Named};
    use iced::{mouse, window, Point, Size};

    use super::super::tests::app_on;
    use super::super::widget_events::Window;
    use super::super::{App, Message, Start, WindowChange};
    use crate::desktop::PopOut;
    use crate::document::{Document, Host};
    use crate::settings::{start_window, Settings, WindowPlace};
    use crate::ui::layout::TOOLBAR_HEIGHT;
    use crate::ui::mixer_layout;
    use crate::ui::panels::{stack_heights, Kept, Panel, Panels};

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    /// The heights the stack shows now, top to bottom.
    fn shown(app: &App) -> Vec<f32> {
        let stacked: Vec<Panel> = app.panels.stacked().collect();
        let kept = app.settings.kept_heights();
        stack_heights(&stacked, app.rack_content(), &kept, app.panels_height())
    }

    /// p3_roll with the playlist, the piano roll and the mixer open in a 1000 × 1000 window,
    /// no hint bar (936 px under the toolbar: 355, 356, 225), the main window known to the app
    /// and holding the keyboard.
    fn three_panels() -> (App, Player, window::Id) {
        let (mut app, player) = app_on(&Path::new(FIXTURES).join("p3_roll.json"));
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 1000.0)));
        for panel in [Panel::Rack, Panel::Playlist, Panel::PianoRoll, Panel::Mixer] {
            let _ = app.update(Message::TogglePanel(panel));
        }
        app.hints_flag = Some(false);
        let main = window::Id::unique();
        app.set_main_window(main);
        let _ = app.update(Message::Windowed(main, WindowChange::Focus(true)));
        (app, player, main)
    }

    /// The divider over `panel` (its line at window y `line`) dragged 60 px sideways and let
    /// go of, through the main window's widgets: its window.
    fn pop_by_divider(app: &mut App, line: f32, panel: Panel) -> window::Id {
        let mut window = Window::new(app);
        let from = Point::new(500.0, line + 3.0);
        window.drag(app, from, Point::new(from.x + 60.0, from.y));
        assert!(app.panels.popped.has(panel), "{:?}", app.panels);
        app.panel_windows.window(panel).expect("its window")
    }

    // The roll's height kept by its divider (dragged 50 px down: the playlist 405, the roll
    // 306), the roll popped (the playlist 711, the mixer 225), then its window's X: the roll
    // is back between them at its kept 306 (405, 306, 225), its window closed and its place
    // kept in the settings (as the stack showed it, 1000 × 306; no position in a test's app).
    // Popped again, the system closing its window docks it likewise.
    #[test]
    fn closing_a_popped_panels_window_puts_it_back_at_its_kept_height() {
        let (mut app, _player, _main) = three_panels();
        let mut window = Window::new(&app);
        let line = TOOLBAR_HEIGHT + 355.0;
        let from = Point::new(500.0, line + 3.0);
        window.drag(&mut app, from, Point::new(from.x, from.y + 50.0));
        assert_eq!(shown(&app), [405.0, 306.0, 225.0]);
        let id = pop_by_divider(&mut app, TOOLBAR_HEIGHT + 405.0, Panel::PianoRoll);
        assert_eq!(shown(&app), [711.0, 225.0]);
        let place = WindowPlace::of(Size::new(1000.0, 306.0), None);
        assert_eq!(app.panel_windows.place(Panel::PianoRoll), Some(place));
        let _ = app.update(Message::WindowCloseRequested(id));
        assert!(app.panels.piano_roll && !app.panels.popped.has(Panel::PianoRoll));
        assert_eq!(app.panel_windows.window(Panel::PianoRoll), None);
        assert_eq!(shown(&app), [405.0, 306.0, 225.0]);
        assert_eq!(app.settings.kept_heights().piano_roll, Some(306.0));
        assert_eq!(app.settings.panel_window(Panel::PianoRoll), Some(place));
        let id = pop_by_divider(&mut app, TOOLBAR_HEIGHT + 405.0, Panel::PianoRoll);
        let _ = app.update(Message::WindowClosed(id));
        assert!(!app.panels.popped.has(Panel::PianoRoll));
        assert_eq!(app.panel_windows.window(Panel::PianoRoll), None);
        assert_eq!(shown(&app), [405.0, 306.0, 225.0]);
    }

    // The roll's key (F7) while it is popped and the main window has the keyboard: the roll
    // stays popped, its window the same, and it takes the editing keys (its window is asked to
    // the front). Once its own window has the keyboard (which gives it the keys too), the same
    // key puts it back in the stack and its window closes. Its toolbar button sends the same
    // message.
    #[test]
    fn a_popped_panels_key_raises_its_window_or_docks_it_from_there() {
        let (mut app, _player, main) = three_panels();
        let id = pop_by_divider(&mut app, TOOLBAR_HEIGHT + 355.0, Panel::PianoRoll);
        app.focus = Panel::Playlist;
        let mut window = Window::new(&app);
        window.tap(&mut app, Named::F7, Code::F7);
        assert!(
            app.panels.popped.has(Panel::PianoRoll),
            "raised, not docked"
        );
        assert_eq!(app.panel_windows.window(Panel::PianoRoll), Some(id));
        assert_eq!(app.focus, Panel::PianoRoll);
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        assert!(
            app.panels.popped.has(Panel::PianoRoll),
            "the button raises too"
        );
        app.focus = Panel::Playlist;
        let _ = app.update(Message::Windowed(main, WindowChange::Focus(false)));
        let _ = app.update(Message::Windowed(id, WindowChange::Focus(true)));
        assert_eq!(app.focus, Panel::PianoRoll);
        window.tap(&mut app, Named::F7, Code::F7);
        assert!(app.panels.piano_roll && !app.panels.popped.has(Panel::PianoRoll));
        assert_eq!(app.panel_windows.window(Panel::PianoRoll), None);
        assert_eq!(shown(&app), [355.0, 356.0, 225.0]);
    }

    // The popped roll's window shows the roll and takes its presses: a click on its grid,
    // delivered through that window's own widgets, is the roll's (it takes the editing keys).
    // The popped mixer's desk is laid out from its window's height, under its 6 px top edge:
    // opened 1000 × 400, 394; resized to 1000 × 300, 294. Its window's Esc clears the
    // selection as the main window's does, closing nothing.
    #[test]
    fn a_popped_panels_window_shows_it_and_takes_its_events() {
        let (mut app, _player, _main) = three_panels();
        let roll = pop_by_divider(&mut app, TOOLBAR_HEIGHT + 355.0, Panel::PianoRoll);
        app.focus = Panel::Playlist;
        let size = Size::new(1000.0, 356.0);
        let mut window = Window::pop_out(&app, roll, size);
        let published = window.click(&mut app, Point::new(600.0, 200.0));
        assert!(
            published
                .iter()
                .any(|message| matches!(message, Message::Roll(_))),
            "{published:?}"
        );
        assert_eq!(app.focus, Panel::PianoRoll);
        let mixer = pop_by_divider(&mut app, TOOLBAR_HEIGHT + 711.0, Panel::Mixer);
        let opened = WindowChange::Opened {
            size: Size::new(1000.0, 400.0),
            position: Some(Point::new(30.0, 40.0)),
        };
        let _ = app.update(Message::Windowed(mixer, opened));
        assert_eq!(app.mixer.size().height, 394.0);
        let _ = app.update(Message::Windowed(
            mixer,
            WindowChange::Resized(Size::new(1000.0, 300.0)),
        ));
        assert_eq!(app.mixer.size().height, 294.0);
        assert_eq!(
            app.panel_windows.place(Panel::Mixer),
            Some(WindowPlace::of(
                Size::new(1000.0, 300.0),
                Some(Point::new(30.0, 40.0))
            ))
        );
        let mut window = Window::pop_out(&app, mixer, Size::new(1000.0, 300.0));
        window.mouse(
            &mut app,
            mouse::Event::CursorLeft,
            mouse::Cursor::Unavailable,
        );
        assert_eq!(app.mixer.size().height, mixer_layout::desk_height(300.0));
        let _ = app.update(Message::Escape(mixer));
        assert!(app.panels.popped.has(Panel::Mixer), "Esc closes no panel");
        assert_eq!(app.panel_windows.window(Panel::Mixer), Some(mixer));
    }

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("dawsome-app-panel-windows-{}", std::process::id()))
            .join(test);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    // The app closing with the mixer popped keeps it in the settings file: the layout names it
    // popped, and its window's place (as it was moved and sized) is kept by its name. The next
    // start from that file (the layout as main reads it, booted as main boots) opens the mixer
    // popped, out of the stack, its window opened at once where it was.
    #[test]
    fn a_panel_popped_as_the_app_closes_opens_popped_where_it_was() {
        let dir = temp_dir("popped_at_close");
        let file = dir.join("dawsome/settings.json");
        let host = Host {
            settings_file: Some(file.clone()),
            saves_settings: true,
            ..Host::offline()
        };
        let path = Path::new(FIXTURES).join("p3_roll.json");
        let project = crate::project_file::open(&path).expect("opens").project;
        let (mut app, _player) = super::super::tests::app_hosted(project, &path, host.clone());
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 1000.0)));
        for panel in [Panel::Playlist, Panel::Mixer] {
            let _ = app.update(Message::TogglePanel(panel));
        }
        app.hints_flag = Some(false);
        let line = TOOLBAR_HEIGHT + shown(&app)[..2].iter().sum::<f32>();
        let id = pop_by_divider(&mut app, line, Panel::Mixer);
        let _ = app.update(Message::Windowed(
            id,
            WindowChange::Moved(Point::new(1930.0, 120.0)),
        ));
        let _ = app.update(Message::Windowed(
            id,
            WindowChange::Resized(Size::new(1200.0, 420.0)),
        ));
        let place = WindowPlace::of(Size::new(1200.0, 420.0), Some(Point::new(1930.0, 120.0)));
        app.save_window_layout();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).expect("saved")).expect("JSON");
        assert_eq!(saved["window"]["popped"], serde_json::json!(["mixer"]));
        assert_eq!(
            saved["panel_windows"],
            serde_json::json!({"mixer": {"width": 1200.0, "height": 420.0, "position": [1930.0, 120.0]}})
        );
        let settings = Settings::load(Some(&file));
        let start = start_window(settings.window.as_ref());
        assert!(start.panels.mixer && start.panels.popped.has(Panel::Mixer));
        let project = crate::project_file::open(&path).expect("opens").project;
        let (mut next, _player) = boot(project, &path, host, start.panels);
        assert_eq!(
            next.panels.stacked().collect::<Vec<_>>(),
            [Panel::Rack, Panel::Playlist]
        );
        let _ = next.sync_pop_outs();
        assert!(next.panel_windows.window(Panel::Mixer).is_some());
        assert_eq!(next.panel_windows.place(Panel::Mixer), Some(place));
        assert_eq!(
            place.size(PopOut::Panel(Panel::Mixer)),
            Size::new(1200.0, 420.0)
        );
        assert_eq!(next.settings.kept_heights(), Kept::default());
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    /// The app booted as main boots it, with `panels` open.
    fn boot(
        project: dawsome_engine::project::Project,
        path: &Path,
        host: Host,
        panels: Panels,
    ) -> (App, Player) {
        let mode = crate::session::Session::mode_for(&project, false);
        let (session, player) = crate::session::Session::open_playing(
            project,
            crate::project_file::samples_dir(path),
            mode,
            dawsome_engine::processor::Registry::native(),
            host.playback.clone(),
        )
        .expect("project compiles");
        let start = Start {
            document: Document::saved_at(path),
        };
        let (app, _) = App::boot(
            session,
            Err(crate::audio::StartError::NoDevice),
            panels,
            crate::ui::layout::DEFAULT_WINDOW_SIZE,
            false,
            crate::headless::Script::default(),
            Arc::new(AtomicBool::new(false)),
            host,
            start,
        );
        (app, player)
    }
}
