//! Settings → General's "Reset to defaults" in the app: the looks and the layout as a first
//! start has them ([`crate::settings::Settings::reset_looks_and_layout`] says which and what
//! stays), applied at once in every window and saved. The app has no dialogs, so a first press
//! arms the button ("Press again to reset") for [`RESET_ARMED`] or until the pointer leaves it,
//! and a second press while it is armed resets. Why a second press: a reset throws away a
//! layout made by hand, and one stray click must not.

use std::time::{Duration, Instant};

use iced::Task;

use super::{first_focus, App, Message};
use crate::ui::hint::Hover;
use crate::ui::palette;
use crate::ui::panels::{Displaced, Panel, Panels};
use crate::ui::piano_layout;
use crate::ui::settings_layout::Hit;
use crate::ui::theme_page::ColorTarget;
use crate::zoom::Zoom;

/// How long a first press keeps the button armed.
pub(super) const RESET_ARMED: Duration = Duration::from_secs(3);

impl App {
    /// A press on "Reset to defaults": the first arms it, a second while it is armed resets.
    pub(super) fn reset_pressed(&mut self) -> Task<Message> {
        if self.reset_armed.take().is_some() {
            return self.reset_looks_and_layout();
        }
        self.reset_armed = Some(Instant::now());
        // The timer is made when the task first runs, on the app's runtime (a test's app runs
        // no task).
        Task::perform(async { tokio::time::sleep(RESET_ARMED).await }, |()| {
            Message::ResetArmedDue
        })
    }

    /// The armed button's time is up: it reads "Reset to defaults" again.
    pub(super) fn reset_armed_due(&mut self) {
        if self
            .reset_armed
            .is_some_and(|armed| armed.elapsed() >= RESET_ARMED)
        {
            self.reset_armed = None;
        }
    }

    /// The pointer is over `hover` now (none: over nothing): anywhere but the reset button
    /// disarms it.
    pub(super) fn reset_hover(&mut self, hover: Option<&Hover>) {
        if hover != Some(&Hover::Setting(Hit::ResetDefaults)) {
            self.reset_armed = None;
        }
    }

    /// The looks and the layout back to a first start's, at once in every window, and saved:
    /// the zoom by the zoom's own path (every size held in a window's pixels worked out
    /// anew), the theme and the accent as Settings → Theme sets them, the views' zooms and the
    /// playlist's lanes, the Library's groups unfolded, and the first start's panels, none
    /// popped (their windows close, their places kept).
    fn reset_looks_and_layout(&mut self) -> Task<Message> {
        let zoom = (self.zoom() != Zoom::DEFAULT).then(|| self.set_zoom(Zoom::DEFAULT));
        self.settings.reset_looks_and_layout();
        palette::set_theme(self.settings.theme());
        palette::set_accent(self.settings.accent());
        self.choose_color_target(ColorTarget::Accent);
        self.roll.view.zoom = piano_layout::ZOOM.default;
        self.arranger.reset_view(self.session.project());
        self.browse.library.folded = self.settings.library_folded();
        let opened = self.open_first_start_panels();
        self.clear_every_cache();
        self.lay_out_mixer_desk();
        self.save_settings();
        Task::batch(zoom.into_iter().chain(opened))
    }

    /// The first start's panels open in the stack and no other (the Settings window stays as
    /// it is), none popped; what a panel just opened needs, for each.
    fn open_first_start_panels(&mut self) -> Vec<Task<Message>> {
        let first = Panels {
            settings: self.panels.settings,
            ..Panels::FIRST_START
        };
        let opened: Vec<Panel> = Panel::ALL
            .into_iter()
            .filter(|&panel| first.is_open(panel) && !self.panels.is_open(panel))
            .collect();
        self.panels = first;
        self.displaced = Displaced::default();
        self.focus = first_focus(self.panels);
        opened
            .into_iter()
            .map(|panel| self.panel_opened(panel))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use dawsome_engine::realtime::Player;
    use iced::{window, Size};

    use super::super::{App, Message};
    use super::RESET_ARMED;
    use crate::settings::Settings;
    use crate::ui::hint::{Hover, HoverMessage};
    use crate::ui::palette::{self, Rgb, ThemeId};
    use crate::ui::panels::{Kept, Panel, Panels};
    use crate::ui::playlist_layout::Lanes;
    use crate::ui::settings_layout::{Hit, SettingsPage};
    use crate::ui::settings_panel::SettingsMessage;
    use crate::ui::theme_page::ColorTarget;
    use crate::zoom::Zoom;

    /// F1 with its settings file written as a user's is, the Settings window open on General.
    fn app_with_file(test: &str) -> (App, Player, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("dawsome-reset-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("dawsome/settings.json");
        let mut host = crate::document::Host::offline();
        host.settings_file = Some(file);
        host.saves_settings = true;
        let f1 = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/f1_four_on_floor.json"
        );
        let project = crate::project_file::open(Path::new(f1))
            .expect("opens")
            .project;
        let (mut app, player) = super::super::tests::app_hosted(project, Path::new(f1), host);
        app.set_main_window(window::Id::unique());
        let _ = app.update(Message::WindowResized(Size::new(1600.0, 900.0)));
        (app, player, dir)
    }

    fn press_reset(app: &mut App) {
        let at = app.settings_panel().latency_layout().reset().center();
        let _ = app.update(Message::Settings(SettingsMessage::Pressed(at)));
    }

    fn over(app: &mut App, hover: Hover) {
        let _ = app.update(Message::Hover(HoverMessage::Over(hover)));
    }

    // "Reset to defaults", end to end through the General page: with the looks and the layout
    // made by hand (zoom 150 %, Gilt, an accent and a notes' colour of the user's own, the piano
    // roll and the mixer open with the mixer popped out, a divider height kept, the views
    // zoomed, the playlist's lanes a height of their own, the roll's Follow off, a Library group
    // folded, the hints off) and a MIDI input and the playlist's Follow set, a first press only
    // arms the button (the page's button reads "Press again to reset"; nothing changes); a
    // second resets, at once and in the file: zoom 100 %, Slate and its own accent, each
    // channel's colour, the first start's panels (the Library, the rack, the playlist, the
    // mixer) with none popped and no height kept, the views at 64 px a bar and 16 px a step, the lanes fitting,
    // Follow on, nothing folded, the hints shown; the MIDI input and the playlist's Follow kept.
    #[test]
    fn a_second_press_resets_the_looks_and_the_layout() {
        let (mut app, _player, dir) = app_with_file("second");
        let file = dir.join("dawsome/settings.json");
        let _ = app.update(Message::TogglePanel(Panel::Settings));
        app.settings.set_zoom(Zoom::nearest(150));
        app.set_theme(ThemeId::Gilt);
        app.set_color(ColorTarget::Accent, Some(Rgb([0x3F, 0xA7, 0xD6])));
        app.set_color(ColorTarget::Notes, Some(Rgb([0xE8, 0xF0, 0x4A])));
        for panel in [Panel::PianoRoll, Panel::Mixer] {
            let _ = app.update(Message::TogglePanel(panel));
        }
        let _ = app.update(Message::PopOutPanel(Panel::Mixer));
        app.settings.set_kept_heights(&Kept {
            piano_roll: Some(300.0),
            ..Kept::default()
        });
        app.roll.view.zoom = 32.0;
        app.settings.set_roll_zoom(32.0);
        app.arranger.grid.zoom = 128.0;
        app.settings.set_playlist_zoom(128.0);
        app.settings.set_playlist_lanes(Lanes::Fixed(48.0));
        app.settings.set_follows(Panel::PianoRoll, false);
        app.settings.set_follows(Panel::Playlist, false);
        app.settings
            .set_library_folded(&std::collections::BTreeSet::from(["Plugins".to_owned()]));
        app.settings.set_show_hints(false);
        app.settings.midi_inputs = crate::settings::MidiInputs::All;
        app.save_settings();
        assert!(app.panels.popped.has(Panel::Mixer));
        let before = Settings::load(Some(&file));

        over(&mut app, Hover::Setting(Hit::ResetDefaults));
        press_reset(&mut app);
        assert!(app.settings_panel().reset_armed, "armed");
        assert_eq!(Settings::load(Some(&file)), before, "nothing reset yet");
        assert_eq!(app.settings.theme(), ThemeId::Gilt);

        press_reset(&mut app);
        assert!(!app.settings_panel().reset_armed, "done");
        let saved = Settings::load(Some(&file));
        for settings in [&app.settings, &saved] {
            assert_eq!(settings.zoom(), Zoom::DEFAULT);
            assert_eq!(settings.theme(), ThemeId::Slate);
            assert_eq!((settings.accent(), settings.note_color()), (None, None));
            assert_eq!(settings.kept_heights(), Kept::default());
            assert_eq!(
                (settings.roll_zoom(), settings.playlist_zoom()),
                (16.0, 64.0)
            );
            assert_eq!(settings.playlist_lanes(), Lanes::Fit);
            assert!(settings.follows(Panel::PianoRoll));
            assert!(settings.library_folded().is_empty());
            assert!(settings.show_hints());
            assert_eq!(
                settings.midi_inputs,
                crate::settings::MidiInputs::All,
                "kept"
            );
            assert!(!settings.follows(Panel::Playlist), "kept");
        }
        assert_eq!(app.zoom(), Zoom::DEFAULT);
        assert_eq!(palette::palette(), ThemeId::Slate.palette());
        assert_eq!(palette::own_accent(), None);
        assert_eq!(
            app.panels,
            Panels {
                settings: true,
                ..Panels::FIRST_START
            }
        );
        assert_eq!((app.roll.view.zoom, app.arranger.grid.zoom), (16.0, 64.0));
        assert_eq!(app.arranger.lanes(), Lanes::Fit);
        assert!(app.browse.library.folded.is_empty());
        assert_eq!(app.settings_page, SettingsPage::General);
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // An armed button disarms when the pointer goes to anything else, or leaves it for nothing,
    // and when its three seconds are up (a press at `now − 3 s`, its timer's message then): a
    // press after either arms it again and resets nothing. A timer of an earlier press, due
    // while a later one still waits, leaves it armed.
    #[test]
    fn the_armed_button_lets_go_when_left_or_after_three_seconds() {
        let (mut app, _player, dir) = app_with_file("disarm");
        let _ = app.update(Message::TogglePanel(Panel::Settings));
        app.set_theme(ThemeId::Gilt);
        let reset = Hover::Setting(Hit::ResetDefaults);
        over(&mut app, reset.clone());
        press_reset(&mut app);
        over(&mut app, Hover::Setting(Hit::ZoomUp));
        assert!(app.reset_armed.is_none(), "another control");
        over(&mut app, reset.clone());
        press_reset(&mut app);
        let _ = app.update(Message::Hover(HoverMessage::Left(reset.clone())));
        assert!(app.reset_armed.is_none(), "left for nothing");
        over(&mut app, reset.clone());
        press_reset(&mut app);
        let _ = app.update(Message::ResetArmedDue);
        assert!(app.reset_armed.is_some(), "a timer early");
        app.reset_armed = Some(Instant::now() - RESET_ARMED - Duration::from_millis(1));
        let _ = app.update(Message::ResetArmedDue);
        assert!(app.reset_armed.is_none(), "three seconds up");
        press_reset(&mut app);
        assert_eq!(app.settings.theme(), ThemeId::Gilt, "a first press again");
        std::fs::remove_dir_all(&dir).expect("removed");
    }
}
