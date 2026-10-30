//! The Help window's life in the app: a window of its own beside the main one, as the Settings
//! window is. [`Help::open`] says whether it is open (F1 and the toolbar's Help button toggle
//! it, its button lit while it is), and [`App::sync_help_window`] opens and closes its window to
//! match after every message, so every path (F1, the button, Esc in it, its X, the system
//! closing it, the main window closing the app) needs no task of its own. It keeps the pop-out
//! windows' level (above the main window while the app has the keyboard), and its size and
//! place are kept in the settings (`help_window`) when it closes and when the app closes with
//! it open.
//!
//! It shows the guide (`help.rs`): the contents on the left, one section a page on the right,
//! which scrolls. Why one section a page and not one long page the contents scroll: a jump then
//! always lands on the section's heading, whatever the window's width does to the lines above
//! it, and the page's foot leads on to the next section, so the guide still reads in order.
//! Its pictures are read from the help folder when their section first shows (a picture is a
//! few hundred kilobytes decoded; the window never reads them all at once), kept for the run.
//! It opens on the first section, and again on the section last read while the app runs.
//! Keys typed in it do what they do in the main window, as in the Settings window.

use std::collections::BTreeMap;
use std::path::PathBuf;

use iced::widget::{image, operation};
use iced::{window, Element, Task};

use super::instrument::pop_out_level;
use super::{App, Message, WindowChange};
use crate::desktop::{self, PopOut};
use crate::help::{self, Block, Guide};
use crate::keys::Action;
use crate::settings::WindowPlace;
use crate::ui::help_window::{HelpMessage, HelpPage, PAGE};

/// The Help window's state.
pub(super) struct Help {
    /// Whether it is open: its window follows.
    pub open: bool,
    /// The section its page shows.
    pub section: usize,
    /// Its own window while it is open or opening.
    pub window: Option<window::Id>,
    /// Where it opens: its size and position as it last was.
    pub place: WindowPlace,
    /// Whether its window was last asked to stay above every window (see
    /// [`App::sync_help_window`]).
    above: bool,
    pub guide: Guide,
    /// Where its pictures are read from; none when no help folder was found.
    pub folder: Option<PathBuf>,
    /// The pictures read so far, by file: none for one missing or unreadable.
    pub pictures: BTreeMap<String, Option<image::Handle>>,
}

impl Help {
    /// Closed, on the first section, opening at `place`, its pictures in `folder`.
    pub fn new(place: Option<WindowPlace>, folder: Option<PathBuf>) -> Self {
        Self {
            open: false,
            section: 0,
            window: None,
            place: place.unwrap_or_else(|| WindowPlace::first(PopOut::Help)),
            above: false,
            guide: Guide::shipped(),
            folder,
            pictures: BTreeMap::new(),
        }
    }

    /// Reads the pictures of the section shown that are not read yet.
    fn read_shown(&mut self) {
        let Some(section) = self.guide.sections.get(self.section) else {
            return;
        };
        for block in &section.blocks {
            let Block::Picture { file, .. } = block else {
                continue;
            };
            if self.pictures.contains_key(file) {
                continue;
            }
            let picture = help::load_picture(self.folder.as_deref(), file).map(|picture| {
                image::Handle::from_rgba(picture.width, picture.height, picture.rgba)
            });
            self.pictures.insert(file.clone(), picture);
        }
    }
}

impl App {
    /// The pop-out windows follow their state ([`App::sync_windows`]), the Help window, the
    /// parameter window and the sample editor too: after every message, and once at start for
    /// the windows the command line opened.
    pub fn sync_pop_outs(&mut self) -> Task<Message> {
        let others = self.sync_windows();
        let help = self.sync_help_window();
        let editor = self.sync_sample_editor_window();
        Task::batch([others, help, self.sync_param_window(), editor])
    }

    /// Opens the Help window's own window while Help is open and closes it once it is not,
    /// keeping its place for the next opening; asks it to the app's level again when the
    /// app's windows were last asked to another (`windows_above`, which
    /// [`App::sync_windows`] keeps). Why apart from that function: it lives beside the
    /// instrument window's, in a file another feature owns.
    pub(super) fn sync_help_window(&mut self) -> Task<Message> {
        let level = pop_out_level(self.windows_above);
        let restack = self.help.above != self.windows_above;
        self.help.above = self.windows_above;
        match (self.help.open, self.help.window) {
            (true, None) => {
                self.help.read_shown();
                let settings = desktop::pop_out_settings(
                    PopOut::Help,
                    self.help.place.size(PopOut::Help),
                    self.help.place.position(),
                    level,
                );
                let (id, opened) = window::open(settings);
                self.help.window = Some(id);
                opened.discard()
            }
            (false, Some(id)) => {
                self.help.window = None;
                self.remember_help_place();
                window::close(id)
            }
            (true, Some(id)) if restack => window::set_level(id, level),
            _ => Task::none(),
        }
    }

    /// The Help window opened, was resized or moved: its place is kept.
    pub(super) fn help_window_changed(&mut self, change: WindowChange) {
        let place = &mut self.help.place;
        match change {
            WindowChange::Opened { size, position } => {
                *place = WindowPlace::of(size, position.or(place.position()));
            }
            WindowChange::Resized(size) => *place = WindowPlace::of(size, place.position()),
            WindowChange::Moved(at) => {
                *place = WindowPlace::of(place.size(PopOut::Help), Some(at));
            }
            WindowChange::Rescaled(_) | WindowChange::Focus(_) => {}
        }
    }

    /// Keeps the Help window's place in the settings for its next opening.
    pub(super) fn remember_help_place(&mut self) {
        let place = Some(self.help.place);
        if self.settings.help_window != place {
            self.settings.help_window = place;
            self.save_settings();
        }
    }

    /// F1 or the toolbar's Help button: Help opens, or closes when open.
    pub(super) fn toggle_help(&mut self) {
        self.help.open = !self.help.open;
    }

    /// Closes the Help window: its X, Esc in it, or the system closing it.
    pub(super) fn close_help(&mut self) {
        self.help.open = false;
    }

    /// Something the Help window reported: a section chosen shows from its top.
    pub(super) fn help_message(&mut self, message: HelpMessage) -> Task<Message> {
        match message {
            HelpMessage::Show(section) if section < self.help.guide.sections.len() => {
                self.help.section = section;
                self.help.read_shown();
                operation::snap_to(PAGE, operation::RelativeOffset::START)
            }
            HelpMessage::Show(_) => Task::none(),
        }
    }

    /// What the Help window shows.
    pub(super) fn help_window_view(&self) -> Element<'_, Message> {
        HelpPage {
            guide: &self.help.guide,
            section: self.help.section,
            pictures: &self.help.pictures,
            foot: self.help_foot(),
        }
        .view()
    }

    /// The Help window's foot line: how to close it, by the keys as bound.
    pub(super) fn help_foot(&self) -> String {
        let keys = self.keys.all(Action::Help, " or ");
        let closes = if keys.is_empty() {
            "Esc closes Help".to_owned()
        } else {
            format!("Esc or {keys} closes Help")
        };
        format!("{closes} \u{b7} the contents on the left jump to a section")
    }

    /// The Help window's title, the main window's way: "Help - Dawsome".
    pub(super) fn help_title(&self) -> String {
        "Help - Dawsome".to_owned()
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
    use super::super::widget_events;
    use super::*;
    use crate::document::Host;
    use crate::ui::help_window::entry_label;

    const F1: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/f1_four_on_floor.json"
    );

    /// The shipped help folder, the one the install copies.
    const FOLDER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/help");

    /// F1 in an app told its main window's id, as main tells it.
    fn f1_app() -> (App, dawsome_engine::realtime::Player, window::Id) {
        let (mut app, player) = app_on(Path::new(F1));
        let main = window::Id::unique();
        app.set_main_window(main);
        (app, player, main)
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
        (app.help.open, app.help.window.is_some())
    }

    // H1, H2, H5: the toolbar's Help button, right of File and Edit, opens the Help window (a click on
    // it through the window's widgets), lit while it is open, and a second click closes it;
    // F1 typed in the main window does the same, with the typing keyboard on as a new window has
    // it (F1 is no key of its notes). The window is one of its own, titled the main window's
    // way. Esc in it closes it and Esc in the main window does not; its X closes it alone (not
    // the app), and so does the system closing it. The main window's X closes the app, the Help
    // window with it.
    #[test]
    fn the_help_window_opens_from_its_button_and_f1_and_closes_with_esc() {
        let (mut app, _player, main) = f1_app();
        assert!(app.typing.on, "the keyboard plays notes");
        let spread = app.toolbar().spread();
        let [_, edit, file, help] = crate::ui::toolbar::retry_edit_file_and_help(&spread);
        assert!(file.end < help.start, "Help right of File");
        assert!(
            file.end < edit.start && edit.end < help.start,
            "File, Edit, Help"
        );
        let middle = Point::new(
            (help.start + help.end) / 2.0,
            crate::ui::layout::TOOLBAR_HEIGHT / 2.0,
        );
        let mut widgets = widget_events::Window::new(&app);
        let published = widgets.click(&mut app, middle);
        assert!(
            published
                .iter()
                .any(|message| matches!(message, Message::ToggleHelp)),
            "{published:?}"
        );
        let id = app.help.window.expect("a window of its own");
        assert!(app.help.open && app.toolbar().help_open, "its button lit");
        assert_ne!(Some(id), app.main_window);
        assert_eq!(app.window_title(id), "Help - Dawsome");
        let _ = widgets.click(&mut app, middle);
        assert_eq!(open(&app), (false, false), "a second click closes it");

        press(&mut app, main, Key::Named(Named::F1), Code::F1);
        assert_eq!(open(&app), (true, true), "F1 opens it");
        press(&mut app, main, Key::Named(Named::F1), Code::F1);
        assert_eq!(open(&app), (false, false), "F1 again closes it");

        let _ = app.update(Message::ToggleHelp);
        let id = app.help.window.expect("open");
        let _ = app.update(Message::Escape(main));
        assert_eq!(open(&app), (true, true), "Esc in the main window leaves it");
        press(&mut app, id, Key::Named(Named::Escape), Code::Escape);
        assert_eq!(open(&app), (false, false), "Esc in it closes it");

        let _ = app.update(Message::ToggleHelp);
        let id = app.help.window.expect("open");
        let _ = app.update(Message::WindowCloseRequested(id));
        assert_eq!(open(&app), (false, false), "its X closes it");
        assert!(!app.exiting(), "and not the app");
        let _ = app.update(Message::ToggleHelp);
        let id = app.help.window.expect("open");
        let _ = app.update(Message::WindowClosed(id));
        assert_eq!(open(&app), (false, false), "the system closed it");
        let _ = app.update(Message::WindowCloseRequested(id));
        assert!(!app.exiting());

        let _ = app.update(Message::ToggleHelp);
        assert!(app.help.window.is_some());
        let _ = app.update(Message::WindowCloseRequested(main));
        assert!(app.exiting(), "the main window's X closes the app");
    }

    // It keeps the pop-out windows' level beside the Settings window: both open at once, each
    // a window of its own, and Esc in the one closes that one alone.
    #[test]
    fn the_help_window_opens_beside_the_settings_window() {
        let (mut app, _player, main) = f1_app();
        let _ = app.update(Message::Windowed(main, WindowChange::Focus(true)));
        let _ = app.update(Message::ToggleHelp);
        let _ = app.update(Message::TogglePanel(crate::ui::panels::Panel::Settings));
        let (help, settings) = (app.help.window, app.settings_window);
        assert!(help.is_some() && settings.is_some() && help != settings);
        assert!(app.windows_above);
        let _ = app.update(Message::Escape(help.expect("open")));
        assert_eq!((app.help.window, app.settings_window), (None, settings));
    }

    // H2, H5: it opens on the first section; each entry of the contents shows its section, from
    // its heading (the page snaps back to its top), and the page's foot leads to the sections
    // either side; a section past the last changes nothing. The page's title is the entry's
    // title, so every entry reaches its heading.
    #[test]
    fn every_section_in_the_contents_reaches_its_heading() {
        let (mut app, _player, _main) = f1_app();
        let _ = app.update(Message::ToggleHelp);
        assert_eq!(app.help.section, 0, "it opens on the first section");
        let count = app.help.guide.sections.len();
        assert_eq!(count, 14);
        for section in (0..count).rev() {
            let task = app.help_message(HelpMessage::Show(section));
            assert_eq!(task.units(), 1, "the page scrolls back to its top");
            assert_eq!(app.help.section, section);
            let title = &app.help.guide.sections[section].title;
            assert!(entry_label(section, title).ends_with(title.as_str()));
            let _ = app.help_window_view();
        }
        let _ = app.update(Message::Help(HelpMessage::Show(count)));
        assert_eq!(app.help.section, 0, "no section past the last");
        let _ = app.update(Message::Help(HelpMessage::Show(3)));
        let _ = app.update(Message::ToggleHelp);
        let _ = app.update(Message::ToggleHelp);
        assert_eq!(app.help.section, 3, "it opens again where it was left");
    }

    // H3: the guide's sections in the spec's order, each opening with words saying what its
    // subject is before any heading, list or picture.
    #[test]
    fn the_guide_has_its_fourteen_sections_in_order() {
        let guide = Guide::shipped();
        let titles: Vec<&str> = guide
            .sections
            .iter()
            .map(|section| section.title.as_str())
            .collect();
        assert_eq!(
            titles,
            [
                "What Dawsome does",
                "The main window",
                "Adding an instrument",
                "Editing a sound",
                "Playing notes",
                "Patterns and the playlist",
                "The piano roll",
                "Recording",
                "The mixer and effects",
                "Saving, exporting and sharing",
                "A quick tune in ten minutes",
                "A melody, step by step",
                "From loop to song",
                "Tips and words to know",
            ]
        );
        for section in &guide.sections {
            assert!(
                matches!(section.blocks.first(), Some(Block::Paragraph(_))),
                "{} starts with what it is",
                section.title
            );
        }
    }

    // H4, H5: every picture the guide names is in the help folder the install ships (its
    // pictures are read from there, not built in), a PNG the window can read, no wider than
    // the page has room for in the window's first size.
    #[test]
    fn every_picture_the_guide_names_is_in_the_help_folder() {
        let guide = Guide::shipped();
        let pictures = guide.pictures();
        assert!(pictures.len() >= 6, "{pictures:?}");
        let room =
            crate::ui::help_window::WINDOW_SIZE.width - crate::ui::help_window::CONTENTS_WIDTH;
        for file in pictures {
            let picture = help::load_picture(Some(Path::new(FOLDER)), file)
                .unwrap_or_else(|| panic!("{file} is not in assets/help"));
            assert!(
                (picture.width as f32) <= room,
                "{file} is {} px wide",
                picture.width
            );
        }
    }

    // H4: with no help folder (an install without it), a section's pictures are none and its
    // page shows their captions alone; nothing fails.
    #[test]
    fn a_missing_picture_leaves_its_caption() {
        let (mut app, _player, _main) = f1_app();
        app.help.folder = None;
        let with_picture = app
            .help
            .guide
            .sections
            .iter()
            .position(|section| {
                section
                    .blocks
                    .iter()
                    .any(|block| matches!(block, Block::Picture { .. }))
            })
            .expect("a section with a picture");
        let _ = app.update(Message::ToggleHelp);
        let _ = app.update(Message::Help(HelpMessage::Show(with_picture)));
        assert!(!app.help.pictures.is_empty());
        assert!(app.help.pictures.values().all(Option::is_none));
        let _ = app.help_window_view();
        let (mut shipped, _player, _main) = f1_app();
        shipped.help.folder = Some(Path::new(FOLDER).to_path_buf());
        let _ = shipped.update(Message::ToggleHelp);
        let _ = shipped.update(Message::Help(HelpMessage::Show(with_picture)));
        assert!(shipped.help.pictures.values().all(Option::is_some));
    }

    // Its size and place are kept in the settings when it closes, and it opens there again in
    // a later start; its foot line names its keys as bound.
    #[test]
    fn the_help_windows_size_and_place_are_kept_in_the_settings() {
        use crate::settings::Settings;
        let dir = std::env::temp_dir().join(format!("dawsome-help-place-{}", std::process::id()));
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
        app.set_main_window(window::Id::unique());
        assert_eq!(
            app.help.place.size(PopOut::Help),
            crate::ui::help_window::WINDOW_SIZE
        );
        assert_eq!(
            app.help_foot(),
            "Esc or F1 closes Help \u{b7} the contents on the left jump to a section"
        );
        let _ = app.update(Message::ToggleHelp);
        let id = app.help.window.expect("open");
        let _ = app.update(Message::Windowed(
            id,
            WindowChange::Resized(Size::new(700.0, 500.0)),
        ));
        let _ = app.update(Message::Windowed(
            id,
            WindowChange::Moved(Point::new(30.0, 40.0)),
        ));
        let _ = app.update(Message::Escape(id));
        let place = WindowPlace::of(Size::new(700.0, 500.0), Some(Point::new(30.0, 40.0)));
        assert_eq!(Settings::load(Some(&file)).help_window, Some(place));
        let (later, _player) = app_hosted(project(), Path::new(F1), host);
        assert_eq!(later.help.place, place);
        std::fs::remove_dir_all(&dir).expect("removed");
    }
}
