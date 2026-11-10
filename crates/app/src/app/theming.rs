//! The theme and the colours of the user's own in the app ([`crate::ui::palette`]): a theme's
//! key on Settings' Theme page ([`crate::ui::theme_page`]) sets the theme, kept in the settings
//! at once, and every window draws in it from the next frame. The toolkit draws every window
//! after every message, the main one, the pop-outs and the Help window alike, so their views
//! follow by themselves; what a canvas kept of an earlier frame is cleared, so no drawing keeps
//! the old colours. Nothing moves: a theme changes colours only.
//!
//! The page's colour wheel sets a colour of the user's own for what its menu chooses
//! ([`ColorTarget`]): the accent over every theme's, or the piano roll's notes over each
//! channel's. A press or a drag on the wheel repaints every window as the cursor moves and
//! writes the settings file once, when let go of; a hex typed in its box and entered, or the key
//! giving the colour back ("Theme's own", "Channel's own"), is kept at once. A hex that is no
//! colour is refused, the reason under the row.

use dawsome_engine::project::ChannelId;
use iced::Point;

use super::App;
use crate::ui::color_wheel::{Hsv, Wheel, WheelPart};
use crate::ui::palette::{self, Rgb, ThemeId};
use crate::ui::settings_layout::Hit;
use crate::ui::style;
use crate::ui::theme_page::{self, ColorTarget, ThemePage};

/// Why a typed hex was refused, shown under the colour's row.
const HEX_REFUSED: &str = "Not a colour: type # and six hex digits (0-9, A-F), as #F29A2E";

/// The colour wheel's editing in Settings.
#[derive(Debug, Default)]
pub(super) struct ColorEdit {
    /// What the wheel edits: the accent until the menu chooses the notes.
    target: ColorTarget,
    /// The last pick on the wheel, held while it is the colour drawn. Why: a grey or black has
    /// no hue of its own, so the wheel would lose the one it was dragged to.
    pick: Option<Hsv>,
    /// The hex box's text while it is typed, until Enter keeps it or the colour changes.
    typed: Option<String>,
    /// Why the hex entered last was refused, until more is typed or the colour changes.
    refused: Option<String>,
}

impl App {
    /// What the Theme page shows: the theme, what the wheel edits, its colour drawn and as the
    /// wheel holds it, the hex box's text and why a hex was refused.
    pub(super) fn theme_page(&self) -> ThemePage {
        let target = self.color_edit.target;
        ThemePage {
            theme: self.settings.theme(),
            target,
            drawn: self.color_drawn(target).color(),
            color: self.color_pick(),
            own: self.own_color(target).is_some(),
            hex: self.hex_text(),
            hex_refused: self.hex_refused(),
        }
    }

    /// A press on the Theme page at `at` (the header's corner the origin): a theme's key, the
    /// colour wheel or the key giving the colour back. The hex box and the menu take their own
    /// presses.
    pub(super) fn theme_page_pressed(&mut self, at: Point) {
        match theme_page::hit(at, self.color_edit.target) {
            Some(Hit::Theme(theme)) => self.set_theme(theme),
            Some(Hit::Wheel(_, part)) => self.pick_color(theme_page::wheel(), part, at),
            Some(Hit::OwnColor(target)) => self.set_color(target, None),
            _ => {}
        }
    }

    /// Sets the theme: kept in the settings, every window drawn in it from the next frame, every
    /// drawing the app keeps cleared.
    pub(super) fn set_theme(&mut self, theme: ThemeId) {
        self.settings.set_theme(theme);
        self.save_settings();
        palette::set_theme(theme);
        self.clear_every_cache();
    }

    /// The menu chose what the wheel edits: it shows that colour, and what was typed in the hex
    /// box for the other is dropped.
    pub(super) fn choose_color_target(&mut self, target: ColorTarget) {
        self.color_edit = ColorEdit {
            target,
            ..ColorEdit::default()
        };
    }

    /// The colour of the user's own for `target`, none while the theme's (the accent) or each
    /// channel's (the notes) is drawn.
    fn own_color(&self, target: ColorTarget) -> Option<Rgb> {
        match target {
            ColorTarget::Accent => palette::own_accent(),
            ColorTarget::Notes => self.settings.note_color(),
        }
    }

    /// The colour `target` draws in now: the user's own, else the theme's accent, or the
    /// selected channel's colour, which the piano roll draws its notes in (the first channel's
    /// with none selected).
    fn color_drawn(&self, target: ColorTarget) -> Rgb {
        match target {
            ColorTarget::Accent => Rgb::of(palette::accent()),
            ColorTarget::Notes => self.settings.note_color().unwrap_or_else(|| {
                Rgb::of(style::channel_color(
                    self.rack_row_of(self.selected_channel),
                ))
            }),
        }
    }

    /// The rack row of `channel`, which picks its colour; the first with none.
    fn rack_row_of(&self, channel: Option<ChannelId>) -> usize {
        let channels = &self.session.project().channels;
        channel
            .and_then(|id| channels.keys().position(|&listed| listed == id))
            .unwrap_or(0)
    }

    /// The colour as the wheel holds it: the last pick while it is the colour drawn, else the
    /// colour's own hue, saturation and value.
    pub(super) fn color_pick(&self) -> Hsv {
        let drawn = self.color_drawn(self.color_edit.target);
        self.color_edit
            .pick
            .filter(|pick| pick.rgb() == drawn)
            .unwrap_or_else(|| Hsv::of(drawn))
    }

    /// The hex box's text: as typed, else the colour drawn, `#RRGGBB`.
    pub(super) fn hex_text(&self) -> String {
        self.color_edit
            .typed
            .clone()
            .unwrap_or_else(|| self.color_drawn(self.color_edit.target).hex())
    }

    /// Why the hex entered last was refused, while it stands.
    pub(super) fn hex_refused(&self) -> Option<String> {
        self.color_edit.refused.clone()
    }

    /// A press on `wheel`'s `part` at `at`, or a drag of it there: every window draws the
    /// colour picked from the next frame, the settings keep it in memory, and the file is
    /// written when the press is let go of ([`App::color_released`]).
    pub(super) fn pick_color(&mut self, wheel: Wheel, part: WheelPart, at: Point) {
        let pick = wheel.picked(part, at, self.color_pick());
        self.show_color(self.color_edit.target, Some(pick.rgb()));
        self.color_edit.pick = Some(pick);
    }

    /// The wheel let go of: the colour picked written to the settings file, once.
    pub(super) fn color_released(&mut self) {
        self.save_settings();
    }

    /// The hex box's text, as typed: shown as it is; a refusal standing goes.
    pub(super) fn hex_typed(&mut self, typed: String) {
        self.color_edit.typed = Some(typed);
        self.color_edit.refused = None;
    }

    /// Enter in the hex box: a colour typed is the target's, kept at once; anything else is
    /// refused, the text left as typed to correct, the reason under the row. With nothing
    /// typed, nothing changes.
    pub(super) fn hex_entered(&mut self) {
        let Some(typed) = self.color_edit.typed.as_deref() else {
            return;
        };
        match Rgb::parse(typed) {
            Some(color) => self.set_color(self.color_edit.target, Some(color)),
            None => self.color_edit.refused = Some(HEX_REFUSED.to_owned()),
        }
    }

    /// Sets `target`'s colour of the user's own, none for the theme's or each channel's again:
    /// every window drawn in it from the next frame and kept in the settings at once.
    pub(super) fn set_color(&mut self, target: ColorTarget, color: Option<Rgb>) {
        self.show_color(target, color);
        self.save_settings();
    }

    /// The Settings window closed: what was typed in the hex box and not entered is dropped,
    /// so it opens on the colour drawn.
    pub(super) fn forget_hex_typed(&mut self) {
        self.color_edit.typed = None;
        self.color_edit.refused = None;
    }

    /// Draws `color` for `target` in every window from the next frame on and keeps it in the
    /// settings in memory; the hex box shows it again.
    fn show_color(&mut self, target: ColorTarget, color: Option<Rgb>) {
        match target {
            ColorTarget::Accent => {
                self.settings.set_accent(color);
                palette::set_accent(color);
            }
            ColorTarget::Notes => self.settings.set_note_color(color),
        }
        self.forget_hex_typed();
        self.clear_every_cache();
    }

    /// A theme name the settings keep that this build does not know (a newer build's, a hand
    /// edit): the default, Slate, shows, and the toolbar says so.
    pub(super) fn show_unknown_theme(&mut self) {
        let Some(name) = self.settings.unknown_theme() else {
            return;
        };
        let shown = ThemeId::default().title();
        let said = format!("Theme \"{name}\" unknown: {shown} shown");
        self.notice = Some(match self.notice.take() {
            Some(notice) => format!("{notice}; {said}"),
            None => said,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use dawsome_engine::project::ChannelId;
    use iced::{window, Color, Point, Size};

    use super::super::tests::app_on;
    use super::super::widget_events::Window;
    use super::super::Message;
    use super::*;
    use crate::ui::console::Shades;
    use crate::ui::palette::Rgb;
    use crate::ui::palette::GRAPHITE;
    use crate::ui::panels::Panel;
    use crate::ui::settings_layout::{tab, SettingsPage};
    use crate::ui::settings_panel::SettingsMessage;

    const SYNTH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/p33_sine.json"
    );
    const MAIN: Size = Size::new(1000.0, 560.0);
    /// Graphite's accent, the theme's own.
    const GRAPHITE_ORANGE: Color = Color::from_rgb8(0xF2, 0x9A, 0x2E);
    const INSTRUMENT: Size = Size::new(924.0, 516.0);

    /// Puts `app` in Graphite, the set these tests' colours were worked out in, as a scripted
    /// run's `--theme graphite` does: an app starts in the default, Slate.
    fn in_graphite(app: &mut App) {
        app.set_theme(ThemeId::Graphite);
    }

    /// The pixel at `at` of a window drawn offscreen.
    fn pixel(window: &mut Window, app: &App, at: Point) -> [u8; 3] {
        let shot = window.snapshot(app);
        let width = shot.size.width as usize;
        let offset = (at.y as usize * width + at.x as usize) * 4;
        let rgba = &shot.rgba[offset..offset + 3];
        [rgba[0], rgba[1], rgba[2]]
    }

    fn rgb(color: Color) -> [u8; 3] {
        let [r, g, b, _] = color.into_rgba8();
        [r, g, b]
    }

    /// Whether every channel of `pixel` is within `tolerance` of `color`.
    fn near(pixel: [u8; 3], color: Color, tolerance: u8) -> bool {
        pixel
            .iter()
            .zip(rgb(color))
            .all(|(&value, wanted)| value.abs_diff(wanted) <= tolerance)
    }

    // Settings → Theme, end to end: the Theme tab, then a press on each other theme's key on
    // the page keeps its name and draws every window in it from the next
    // frame, with the same widgets and caches as before (what the toolkit keeps between frames):
    // the main window's toolbar ground (Graphite 2B2E33, then the theme's panel: Gilt's 2E201B),
    // the synth window's chassis near its top, the window's ground (25282D, then Gilt's
    // 2B1E18), which only its size redraws otherwise, and the Help window's page, the well's
    // ground (1F2125, then Gilt's 251812). Graphite again brings Graphite back and keeps
    // nothing.
    #[test]
    fn a_theme_chosen_in_settings_repaints_every_window() {
        let (mut app, _player) = app_on(Path::new(SYNTH));
        in_graphite(&mut app);
        app.set_main_window(window::Id::unique());
        let _ = app.update(Message::WindowResized(MAIN));
        let _ = app.update(Message::OpenInstrument(ChannelId(1)));
        let _ = app.update(Message::ToggleHelp);
        let _ = app.update(Message::TogglePanel(Panel::Settings));
        let instrument_id = app.instrument_window.expect("the synth's window");
        let help_id = app.help.window.expect("the Help window");
        let mut main = Window::new(&app);
        let mut instrument = Window::pop_out(&app, instrument_id, INSTRUMENT);
        let mut help = Window::pop_out(&app, help_id, crate::ui::help_window::WINDOW_SIZE);
        let toolbar = Point::new(MAIN.width - 20.0, 4.0);
        let chassis = Point::new(8.0, 8.0);
        let page = Point::new(crate::ui::help_window::WINDOW_SIZE.width - 40.0, 12.0);
        let look = |main: &mut Window, instrument: &mut Window, help: &mut Window, app: &App| {
            (
                pixel(main, app, toolbar),
                pixel(instrument, app, chassis),
                pixel(help, app, page),
            )
        };
        let graphite = look(&mut main, &mut instrument, &mut help, &app);
        assert_eq!(graphite.0, rgb(GRAPHITE.panel), "the toolbar");
        assert!(
            near(graphite.1, Shades::of(&GRAPHITE).chassis_top, 3),
            "the chassis: {:02X?}",
            graphite.1
        );
        assert_eq!(graphite.2, rgb(GRAPHITE.bg), "the Help page");

        let theme_tab = tab(SettingsPage::Theme).center();
        let _ = app.update(Message::Settings(SettingsMessage::Pressed(theme_tab)));
        assert_eq!(app.settings_page, SettingsPage::Theme);
        let press = |app: &mut App, theme: ThemeId| {
            let key = theme_page::theme_key(theme).center();
            let _ = app.update(Message::Settings(SettingsMessage::Pressed(key)));
        };
        for theme in ThemeId::ALL
            .into_iter()
            .filter(|&theme| theme != ThemeId::Graphite)
        {
            press(&mut app, theme);
            assert_eq!(app.settings.theme(), theme, "kept");
            let set = theme.palette();
            assert_eq!(palette::palette(), set);
            let shown = look(&mut main, &mut instrument, &mut help, &app);
            assert_eq!(shown.0, rgb(set.panel), "{theme:?}'s toolbar");
            assert!(
                near(shown.1, Shades::of(set).chassis_top, 3),
                "{theme:?}'s chassis redrawn: {:02X?}",
                shown.1
            );
            assert_eq!(shown.2, rgb(set.bg), "{theme:?}'s Help page");
        }

        press(&mut app, ThemeId::Graphite);
        assert_eq!(app.settings.theme(), ThemeId::Graphite);
        assert_eq!(app.settings.unknown_theme(), None);
        assert_eq!(look(&mut main, &mut instrument, &mut help, &app), graphite);
    }

    /// How many pixels of `rect` in a window drawn offscreen lie within 12 of `color` in every
    /// channel.
    fn count_near(window: &mut Window, app: &App, rect: iced::Rectangle, color: Color) -> usize {
        let shot = window.snapshot(app);
        let width = shot.size.width as usize;
        let rows = rect.y as usize..(rect.y + rect.height) as usize;
        let columns = rect.x as usize..(rect.x + rect.width) as usize;
        rows.flat_map(|y| columns.clone().map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let offset = (y * width + x) * 4;
                let rgba = &shot.rgba[offset..offset + 3];
                near([rgba[0], rgba[1], rgba[2]], color, 12)
            })
            .count()
    }

    // Settings → Theme → Accent, end to end through the Settings window's own widgets (its Theme
    // tab clicked first), its settings file written as a user's is. A press on the wheel's ring
    // at its right (72 px from the centre, the ring's middle) and a drag
    // through the centre to its left repaints every window as the cursor moves: the picks are
    // Graphite's orange (33.06°, 0.81, 0.95, the generator's) turned to 0° and then 180°, worked
    // by hand with c = v · s and m = v − c as (c, 0, 0) + m, #F22E2E, and (0, c, c) + m,
    // #2EF2F2. The swatch and the main window's
    // toolbar (its lit rack button) show it, no orange is left anywhere in the main window, and
    // the file holds no accent until the press is let go of, then "#2EF2F2". Typed in the hex
    // box, "#3fa7d6" and Enter is the accent at once and kept as "#3FA7D6", and the keys typed
    // play no note; "#12" and Enter is refused, the accent kept, the reason under the row in the
    // warning colour. #767676 shows the low-contrast note (2.9996 on Graphite's panel), #777777
    // (3.0425) none. "Theme's own" puts the orange back and the file keeps no accent.
    #[test]
    fn an_accent_picked_on_the_wheel_repaints_every_window_and_is_kept_when_let_go() {
        use iced::keyboard::key::{Code, Named};
        use iced::mouse;

        use crate::settings::Settings;
        use crate::ui::palette::Rgb;
        use crate::ui::settings_panel::WINDOW_SIZE;
        use crate::ui::{layout::TOOLBAR_HEIGHT, style};

        let dir = std::env::temp_dir().join(format!("dawsome-accent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("dawsome/settings.json");
        let mut host = crate::document::Host::offline();
        host.settings_file = Some(file.clone());
        host.saves_settings = true;
        let f1 = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/f1_four_on_floor.json"
        );
        let project = crate::project_file::open(Path::new(f1))
            .expect("opens")
            .project;
        let (mut app, _player) = super::super::tests::app_hosted(project, Path::new(f1), host);
        in_graphite(&mut app);
        app.set_main_window(window::Id::unique());
        let _ = app.update(Message::WindowResized(MAIN));
        let _ = app.update(Message::TogglePanel(Panel::Settings));
        let settings_id = app.settings_window.expect("the Settings window");
        let mut main = Window::new(&app);
        let mut settings = Window::pop_out(&app, settings_id, WINDOW_SIZE);
        let _ = settings.click(&mut app, tab(SettingsPage::Theme).center());
        assert_eq!(app.settings_page, SettingsPage::Theme);
        let wheel = theme_page::wheel();
        let toolbar = iced::Rectangle::new(Point::ORIGIN, Size::new(MAIN.width, TOOLBAR_HEIGHT));
        let whole = iced::Rectangle::new(Point::ORIGIN, MAIN);
        let swatch = theme_page::color_swatch();
        let orange = GRAPHITE_ORANGE;
        let cyan = Rgb([0x2E, 0xF2, 0xF2]);
        assert!(
            count_near(&mut main, &app, toolbar, orange) > 4,
            "the lit button"
        );
        assert_eq!(count_near(&mut main, &app, whole, cyan.color()), 0);

        let center = wheel.center();
        let right = Point::new(center.x + 72.0, center.y);
        let left = Point::new(center.x - 72.0, center.y);
        let at = mouse::Cursor::Available;
        let left_button = mouse::Button::Left;
        settings.mouse(
            &mut app,
            mouse::Event::ButtonPressed(left_button),
            at(right),
        );
        let red = Rgb([0xF2, 0x2E, 0x2E]);
        assert_eq!(palette::own_accent(), Some(red), "the press picks hue 0");
        for to in [center, left] {
            let moved = mouse::Event::CursorMoved { position: to };
            settings.mouse(&mut app, moved, at(to));
        }
        assert_eq!(palette::own_accent(), Some(cyan), "live");
        assert_eq!(
            Settings::load(Some(&file)).accent(),
            None,
            "not written while held"
        );
        let swatch_pixel = pixel(&mut settings, &app, swatch.center());
        assert_eq!(swatch_pixel, cyan.0, "the swatch");
        assert!(
            count_near(&mut main, &app, toolbar, cyan.color()) > 4,
            "the lit button"
        );
        assert_eq!(
            count_near(&mut main, &app, whole, orange),
            0,
            "no orange left"
        );
        settings.mouse(
            &mut app,
            mouse::Event::ButtonReleased(left_button),
            at(left),
        );
        assert_eq!(
            Settings::load(Some(&file)).accent(),
            Some(cyan),
            "written when let go"
        );

        let sent = app.session.controls_sent().len();
        let typed = |text: &str| -> Vec<(char, Code)> {
            text.chars()
                .map(|typed| {
                    let code = match typed {
                        '#' => Code::Digit3,
                        '1' => Code::Digit1,
                        '2' => Code::Digit2,
                        '3' => Code::Digit3,
                        '6' => Code::Digit6,
                        '7' => Code::Digit7,
                        'a' => Code::KeyA,
                        'd' => Code::KeyD,
                        'f' => Code::KeyF,
                        _ => Code::Digit0,
                    };
                    (typed, code)
                })
                .collect()
        };
        let enter = |app: &mut App, settings: &mut Window, text: &str| {
            settings.focus(app, theme_page::HEX_INPUT);
            settings.type_keys(app, &typed(text));
            settings.tap(app, Named::Enter, Code::Enter);
        };
        enter(&mut app, &mut settings, "#3fa7d6");
        let blue = Rgb([0x3F, 0xA7, 0xD6]);
        assert_eq!(palette::own_accent(), Some(blue));
        assert_eq!(Settings::load(Some(&file)).accent(), Some(blue));
        assert_eq!(app.hex_text(), "#3FA7D6", "the box shows it as kept");
        assert_eq!(
            app.session.controls_sent().len(),
            sent,
            "the keys played no note"
        );

        let note = theme_page::color_note();
        let warning =
            |settings: &mut Window, app: &App| count_near(settings, app, note, style::WARNING);
        assert_eq!(warning(&mut settings, &app), 0);
        enter(&mut app, &mut settings, "#12");
        assert_eq!(palette::own_accent(), Some(blue), "refused");
        assert_eq!(Settings::load(Some(&file)).accent(), Some(blue));
        assert!(app.hex_refused().is_some());
        assert!(
            warning(&mut settings, &app) > 10,
            "the reason under the row"
        );

        let dim = palette::palette().text_secondary;
        for (grey, low) in [("#767676", true), ("#777777", false)] {
            enter(&mut app, &mut settings, grey);
            let words = count_near(&mut settings, &app, note, dim);
            assert_eq!(words > 10, low, "{grey}: {words} px of the note");
        }

        let key = theme_page::own_color_key().center();
        let _ = settings.click(&mut app, key);
        assert_eq!(palette::own_accent(), None, "the theme's own");
        assert_eq!(palette::accent(), orange);
        assert_eq!(Settings::load(Some(&file)).accent(), None);
        assert!(
            count_near(&mut main, &app, toolbar, orange) > 4,
            "the lit button"
        );
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // Settings → Theme with the menu on "Piano roll notes", end to end through the Settings
    // window's widgets on p3_roll with the rack and the piano roll open, its settings file
    // written as a user's is. Chosen, the page's swatch shows the colour the roll draws the
    // lead's notes in now, the first channel's #E5533D, and the hex box says so; the key reads
    // "Channel's own", on a button's face. "#e8f04a" typed and entered is the notes' colour at
    // once, kept as "#E8F04A" (the accent stays the theme's): the main window draws the lead's
    // notes and stems in it (more than 50 pixels within 12 of it, none before) and the kick's
    // ghosts keep their blue. #7F7F7F shows the note under the row (2.9801 on the white keys'
    // row, the generator's pair), #808080 (3.0214) none. "Channel's own" takes the colour away,
    // the file keeping none, and the menu back on "Accent" shows the accent again.
    #[test]
    fn a_notes_colour_set_in_settings_repaints_the_piano_roll_and_is_kept() {
        use iced::keyboard::key::{Code, Named};

        use crate::settings::Settings;
        use crate::ui::settings_panel::WINDOW_SIZE;
        use crate::ui::style;

        let dir = std::env::temp_dir().join(format!("dawsome-note-colour-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("dawsome/settings.json");
        let mut host = crate::document::Host::offline();
        host.settings_file = Some(file.clone());
        host.saves_settings = true;
        let p3 = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/p3_roll.json"
        );
        let project = crate::project_file::open(Path::new(p3))
            .expect("opens")
            .project;
        let (mut app, _player) = super::super::tests::app_hosted(project, Path::new(p3), host);
        in_graphite(&mut app);
        app.set_main_window(window::Id::unique());
        let _ = app.update(Message::WindowResized(MAIN));
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        let _ = app.update(Message::TogglePanel(Panel::Settings));
        let settings_id = app.settings_window.expect("the Settings window");
        let mut main = Window::new(&app);
        let mut settings = Window::pop_out(&app, settings_id, WINDOW_SIZE);
        let _ = settings.click(&mut app, tab(SettingsPage::Theme).center());
        let _ = app.update(Message::Settings(SettingsMessage::ColorTargetChosen(
            ColorTarget::Notes,
        )));
        let red = style::CHANNEL_COLORS[0];
        let swatch = theme_page::color_swatch();
        assert_eq!(pixel(&mut settings, &app, swatch.center()), rgb(red));
        assert_eq!(app.hex_text(), "#E5533D");
        let key = theme_page::own_color_key();
        let face = Point::new(key.x + 3.0, key.center_y());
        assert_eq!(pixel(&mut settings, &app, face), rgb(GRAPHITE.button));

        let yellow_green = Color::from_rgb8(0xE8, 0xF0, 0x4A);
        let whole = iced::Rectangle::new(Point::ORIGIN, MAIN);
        assert_eq!(count_near(&mut main, &app, whole, yellow_green), 0);
        let ghost = Color::from_rgb8(0x3C, 0x57, 0x6E);
        let ghosts = count_near(&mut main, &app, whole, ghost);
        assert!(ghosts > 20, "the kick's ghosts: {ghosts}");
        let typed = |text: &str| -> Vec<(char, Code)> {
            text.chars()
                .map(|typed| {
                    let code = match typed {
                        '#' => Code::Digit3,
                        '4' => Code::Digit4,
                        '7' => Code::Digit7,
                        '8' => Code::Digit8,
                        'a' => Code::KeyA,
                        'e' => Code::KeyE,
                        'f' => Code::KeyF,
                        _ => Code::Digit0,
                    };
                    (typed, code)
                })
                .collect()
        };
        let enter = |app: &mut App, settings: &mut Window, text: &str| {
            settings.focus(app, theme_page::HEX_INPUT);
            settings.type_keys(app, &typed(text));
            settings.tap(app, Named::Enter, Code::Enter);
        };
        enter(&mut app, &mut settings, "#e8f04a");
        let kept = Rgb([0xE8, 0xF0, 0x4A]);
        assert_eq!(app.settings.note_color(), Some(kept));
        let read = Settings::load(Some(&file));
        assert_eq!((read.note_color(), read.accent()), (Some(kept), None));
        assert_eq!(app.hex_text(), "#E8F04A");
        assert!(
            count_near(&mut main, &app, whole, yellow_green) > 50,
            "the lead's notes and stems"
        );
        assert_eq!(
            count_near(&mut main, &app, whole, ghost),
            ghosts,
            "the ghosts keep theirs"
        );
        assert_eq!(palette::accent(), GRAPHITE_ORANGE, "the accent untouched");

        let note = theme_page::color_note();
        let dim = palette::palette().text_secondary;
        for (grey, low) in [("#7f7f7f", true), ("#808080", false)] {
            enter(&mut app, &mut settings, grey);
            let words = count_near(&mut settings, &app, note, dim);
            assert_eq!(words > 10, low, "{grey}: {words} px of the note");
        }

        let _ = settings.click(&mut app, key.center());
        assert_eq!(app.settings.note_color(), None, "each channel's own");
        assert_eq!(Settings::load(Some(&file)).note_color(), None);
        assert_eq!(count_near(&mut main, &app, whole, yellow_green), 0);
        let _ = app.update(Message::Settings(SettingsMessage::ColorTargetChosen(
            ColorTarget::Accent,
        )));
        assert_eq!(app.hex_text(), "#F29A2E", "the accent again");
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // The Theme page drawn offscreen in the Settings window: each key's swatch shows its set's
    // panel and its own accent (the squares' centres, 4 px in from the swatch's left and 12 px
    // in, 5 px down), the theme drawn now's key on a button's face (3 px in from its right
    // end, clear of its name) and the others on the page's ground. A press on Gilt's key lights
    // it in Gilt's button colour and leaves Graphite's on Gilt's ground; an accent of the user's
    // own leaves every swatch as its set has it.
    #[test]
    fn the_theme_page_shows_each_sets_swatch_and_lights_the_theme_shown() {
        use crate::ui::settings_panel::WINDOW_SIZE;
        let (mut app, _player) = app_on(Path::new(SYNTH));
        in_graphite(&mut app);
        app.set_main_window(window::Id::unique());
        let _ = app.update(Message::TogglePanel(Panel::Settings));
        let id = app.settings_window.expect("the Settings window");
        let mut settings = Window::pop_out(&app, id, WINDOW_SIZE);
        let _ = settings.click(&mut app, tab(SettingsPage::Theme).center());
        let read = |shot: &window::Screenshot, at: Point| {
            let offset = (at.y as usize * shot.size.width as usize + at.x as usize) * 4;
            [
                shot.rgba[offset],
                shot.rgba[offset + 1],
                shot.rgba[offset + 2],
            ]
        };
        let swatches = |shot: &window::Screenshot| {
            ThemeId::ALL.map(|theme| {
                let swatch = theme_page::set_swatch(theme);
                [4.0, 12.0].map(|x| read(shot, Point::new(swatch.x + x, swatch.y + 5.0)))
            })
        };
        let face = |shot: &window::Screenshot, theme: ThemeId| {
            let key = theme_page::theme_key(theme);
            read(shot, Point::new(key.x + key.width - 3.0, key.center_y()))
        };
        let own = ThemeId::ALL.map(|theme| theme.swatch().map(rgb));
        let shot = settings.snapshot(&app);
        assert_eq!(swatches(&shot), own, "each set's own");
        assert_eq!(face(&shot, ThemeId::Graphite), rgb(GRAPHITE.button));
        assert_eq!(face(&shot, ThemeId::Slate), rgb(GRAPHITE.bg));

        let _ = settings.click(&mut app, theme_page::theme_key(ThemeId::Gilt).center());
        assert_eq!(app.settings.theme(), ThemeId::Gilt);
        let gilt = ThemeId::Gilt.palette();
        let shot = settings.snapshot(&app);
        assert_eq!(face(&shot, ThemeId::Gilt), rgb(gilt.button));
        assert_eq!(face(&shot, ThemeId::Graphite), rgb(gilt.bg));
        assert_eq!(swatches(&shot), own, "under Gilt");
        app.set_color(ColorTarget::Accent, Some(Rgb([0x3F, 0xA7, 0xD6])));
        let shot = settings.snapshot(&app);
        assert_eq!(swatches(&shot), own, "under an accent of one's own");
    }

    // A settings file naming a theme this build does not know ("oak", a newer build's): the app
    // starts in the default, Slate, and the toolbar says so, the name kept for the build that
    // wrote it.
    #[test]
    fn an_unknown_theme_starts_in_slate_and_says_so() {
        let dir = std::env::temp_dir().join(format!("dawsome-theme-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("dawsome/settings.json");
        std::fs::create_dir_all(dir.join("dawsome")).expect("temp dir");
        std::fs::write(&file, r#"{"ui_theme":"oak"}"#).expect("writes");
        palette::set_theme(ThemeId::Gilt);
        let mut host = crate::document::Host::offline();
        host.settings_file = Some(file);
        let project = crate::project_file::new_document();
        let (app, _player) = super::super::tests::app_hosted(project, Path::new(""), host);
        assert_eq!(
            palette::palette(),
            ThemeId::Slate.palette(),
            "set at the start"
        );
        assert_eq!(app.settings.unknown_theme(), Some("oak"));
        let notices = app.toolbar().notice_text().expect("a notice");
        assert!(
            notices.contains("Theme \"oak\" unknown: Slate shown"),
            "{notices}"
        );
        std::fs::remove_dir_all(&dir).expect("removed");
    }
}
