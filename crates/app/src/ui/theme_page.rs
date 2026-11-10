//! The Settings window's Theme page: the themes' keys in one row, each a small swatch of its
//! set (its panel and its own accent, two squares, [`ThemeId::swatch`]) beside its name, the
//! theme drawn now on a button's face; and under them the colours' section: a menu choosing
//! what the one colour wheel edits ([`ColorTarget`]: the accent, or the piano roll's notes),
//! then the colour's row: the swatch of the colour drawn now, its colour wheel ([`Wheel`]), and
//! beside the wheel its hex box over the key giving the colour back to the theme ("Theme's
//! own") or the channels ("Channel's own"), with a line under the row for a note on it (low
//! contrast, a hex refused). They were rows of the General page's Display section, between the
//! zoom and the audio devices, until they stood in the way of them. Why one wheel and a menu,
//! not a wheel for each: the page fits the Settings window's least height with one, and the
//! two colours are edited the same way.
//!
//! Positions are in logical pixels from the header's top-left corner, as the General page's are
//! ([`crate::ui::settings_layout`]): with the page unscrolled they are the Settings window's
//! own, and a press on the page reaches the app as one on the General page does
//! ([`SettingsMessage::Pressed`]). The drawing, the hit test and the screenshot probe read the
//! same functions.

use std::fmt;

use iced::widget::canvas::{self, Event, Frame, Geometry};
use iced::widget::{pick_list, pin, stack, text_input};
use iced::{mouse, Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme, Vector};

use crate::app::Message;
use crate::ui::color_wheel::{self, Hsv, Wheel, WheelPart};
use crate::ui::fonts::size;
use crate::ui::hint::Hover;
use crate::ui::palette::{ThemeId, ACCENT_LEAST_CONTRAST};
use crate::ui::settings_layout::{
    title_at, Hit, BOX_HEIGHT, HEADER_HEIGHT, INSET, LABEL_X, ROW_HEIGHT, SECTION_GAP,
    SECTION_TITLE_HEIGHT, TEXT_WIDTH,
};
use crate::ui::settings_panel::{centered, page_point, text, SettingsMessage};
use crate::ui::style;
use crate::ui::text_fit::fit_width;

/// The colour's hex box, for the toolkit's focus.
pub const HEX_INPUT: &str = "settings-accent-hex";
/// The hex box's and the menu's line: their 18 px ([`BOX_HEIGHT`]) less 1 px of room above and
/// below.
const HEX_LINE_HEIGHT: f32 = 16.0;

/// What the colour wheel edits, as the menu over it offers them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ColorTarget {
    /// The accent of every window ([`crate::ui::palette::accent`]).
    #[default]
    Accent,
    /// The piano roll's notes and their velocity stems (`ui_note_color`), each channel's own
    /// colour until one is set; the rack's steps and the playlist's clips keep theirs.
    Notes,
}

impl ColorTarget {
    /// The menu's rows, in its order.
    pub const ALL: [ColorTarget; 2] = [ColorTarget::Accent, ColorTarget::Notes];

    /// The key that gives the colour back: to the theme, or to each channel.
    pub fn own_label(self) -> &'static str {
        match self {
            ColorTarget::Accent => "Theme's own",
            ColorTarget::Notes => "Channel's own",
        }
    }

    /// The note under the row when the colour reads poorly where it is drawn: the accent on the
    /// theme's panels, the notes on the piano roll's rows.
    pub fn low_contrast(self) -> &'static str {
        match self {
            ColorTarget::Accent => "Low contrast on this theme's panels",
            ColorTarget::Notes => "Low contrast on the piano roll's rows",
        }
    }

    /// Whether `color` reads poorly where this target draws it, under
    /// [`ACCENT_LEAST_CONTRAST`] (3:1): the accent on the theme's panel; the notes on either of
    /// the roll's row tones, the white keys' and the black keys'.
    pub fn reads_poorly(self, color: Color) -> bool {
        let palette = style::palette();
        let grounds: &[Color] = match self {
            ColorTarget::Accent => &[palette.panel],
            ColorTarget::Notes => &[palette.roll_row_white, palette.roll_row_black],
        };
        grounds
            .iter()
            .any(|&ground| style::contrast(color, ground) < ACCENT_LEAST_CONTRAST)
    }
}

impl fmt::Display for ColorTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ColorTarget::Accent => "Accent",
            ColorTarget::Notes => "Piano roll notes",
        })
    }
}

/// A theme's key: its swatch and its name with this much room either side, [`SET_SWATCH_GAP`]
/// between them.
const KEY_PAD: f32 = 8.0;
/// A set's swatch on its key: two squares side by side, its panel then its accent, in a 1 px
/// edge.
pub const SET_SWATCH: Size = Size::new(2.0 * SET_SQUARE + 2.0, SET_SQUARE + 2.0);
const SET_SQUARE: f32 = 8.0;
const SET_SWATCH_GAP: f32 = 4.0;
/// Each theme's key in [`ThemeId::ALL`]'s order (Slate, Graphite, Gilt, Garnet, Fjord): its
/// swatch and its name with [`KEY_PAD`] either side, rounded up to an even width (Graphite: 8 +
/// 18 + 4 + 48.5 + 8 = 86.5, so 88), a
/// gap between them as between the header's tabs. Why a width each and not one for all: five
/// keys as wide as Graphite's would need 5 × 88 + 4 × 4 = 456 px, more than the Settings
/// window's least width (420); these end at 388, inside the row's text box (x 392), so the five
/// stand in one row. A sixth set: a width here, and a second row once the row's test says it no
/// longer fits.
pub const THEME_KEY_WIDTHS: [f32; ThemeId::ALL.len()] = [68.0, 88.0, 58.0, 78.0, 68.0];
pub const THEME_KEY_GAP: f32 = 4.0;
/// The colour's row: the swatch at the left margin, as tall as a stepper's button; the gap after
/// it and after the wheel; the hex box ("#F29A2E" with room either side) and the own-colour key
/// under it, as wide as its longer words ("Channel's own", 84.09 px) with a key's room either
/// side, rounded up to 4 px (it was 96 for "Theme's own" alone).
pub const SWATCH: Size = Size::new(36.0, 18.0);
pub const ACCENT_GAP: f32 = 12.0;
pub const HEX_BOX_WIDTH: f32 = 72.0;
pub const OWN_COLOR_WIDTH: f32 = 104.0;
/// The menu over the row choosing what the wheel edits: its longest row ("Piano roll notes",
/// 87.41 px) with the menu's padding and arrow (30 px), rounded up to 4 px.
pub const TARGET_BOX_WIDTH: f32 = 120.0;
/// The colour's row is the wheel's height and 2 px above and below it. Why the wheel is 160 px
/// here (120 on the General page): the row and the menu over it fit the page's least room, the
/// Settings window's least height less its header and its foot line (240 − 24 − 22 = 194), so
/// the least window scrolls them into view whole; a 200 px wheel would not fit (204).
pub const ACCENT_ROW_HEIGHT: f32 = color_wheel::SIZE + 4.0;

/// The "Theme" section's title, under the header.
pub fn theme_title() -> Rectangle {
    title_at(HEADER_HEIGHT + SECTION_GAP)
}

/// The keys' row, under their title.
pub fn keys_row() -> Rectangle {
    Rectangle::new(
        Point::new(0.0, theme_title().y + SECTION_TITLE_HEIGHT),
        Size::new(LABEL_X + TEXT_WIDTH, ROW_HEIGHT),
    )
}

/// The key of `theme`, in the order Settings offers them, along the row from the left margin,
/// the row's whole 24 px tall. Why: WCAG 2.2's 24 px target, which the row allows (the keys were
/// 18 tall under its spacing exception).
pub fn theme_key(theme: ThemeId) -> Rectangle {
    let index = ThemeId::ALL
        .iter()
        .position(|&offered| offered == theme)
        .unwrap_or(0);
    let x = INSET
        + THEME_KEY_WIDTHS[..index]
            .iter()
            .map(|width| width + THEME_KEY_GAP)
            .sum::<f32>();
    let row = keys_row();
    Rectangle::new(
        Point::new(x, row.y),
        Size::new(THEME_KEY_WIDTHS[index], ROW_HEIGHT),
    )
}

/// The swatch on `theme`'s key, [`KEY_PAD`] in from its left, centred in its height.
pub fn set_swatch(theme: ThemeId) -> Rectangle {
    let key = theme_key(theme);
    Rectangle::new(
        Point::new(
            key.x + KEY_PAD,
            key.y + (key.height - SET_SWATCH.height) / 2.0,
        ),
        SET_SWATCH,
    )
}

/// Where `theme`'s name starts on its key: after its swatch and a gap.
pub fn name_x(theme: ThemeId) -> f32 {
    let swatch = set_swatch(theme);
    swatch.x + swatch.width + SET_SWATCH_GAP
}

/// The "Colours" section's title, under the keys.
pub fn colors_title() -> Rectangle {
    title_at(keys_row().y + ROW_HEIGHT + SECTION_GAP)
}

/// The menu choosing what the wheel edits, at the left margin of the row under the title,
/// [`BOX_HEIGHT`] tall and centred in the row.
pub fn target_box() -> Rectangle {
    let top = colors_title().y + SECTION_TITLE_HEIGHT;
    Rectangle::new(
        Point::new(INSET, top + (ROW_HEIGHT - BOX_HEIGHT) / 2.0),
        Size::new(TARGET_BOX_WIDTH, BOX_HEIGHT),
    )
}

/// The colour's row, a row under the menu.
pub fn color_row() -> Rectangle {
    Rectangle::new(
        Point::new(INSET, colors_title().y + SECTION_TITLE_HEIGHT + ROW_HEIGHT),
        Size::new(TEXT_WIDTH, ACCENT_ROW_HEIGHT),
    )
}

/// The swatch showing the colour drawn now, at the row's start, centred in its first 24 px.
pub fn color_swatch() -> Rectangle {
    let row = color_row();
    Rectangle::new(
        Point::new(row.x, row.y + (ROW_HEIGHT - SWATCH.height) / 2.0),
        SWATCH,
    )
}

/// The colour wheel, right of the swatch.
pub fn wheel() -> Wheel {
    let swatch = color_swatch();
    Wheel {
        at: Point::new(swatch.x + swatch.width + ACCENT_GAP, swatch.y),
    }
}

/// The hex box, right of the wheel at its top, [`BOX_HEIGHT`] tall.
pub fn hex_box() -> Rectangle {
    let wheel = wheel().bounds();
    Rectangle::new(
        Point::new(wheel.x + wheel.width + ACCENT_GAP, wheel.y),
        Size::new(HEX_BOX_WIDTH, BOX_HEIGHT),
    )
}

/// The key giving the colour back ("Theme's own", "Channel's own"), a row under the hex box.
pub fn own_color_key() -> Rectangle {
    let hex = hex_box();
    Rectangle::new(
        Point::new(hex.x, hex.y + ROW_HEIGHT),
        Size::new(OWN_COLOR_WIDTH, BOX_HEIGHT),
    )
}

/// The line under the colour's row: the note that the colour reads poorly where it is drawn,
/// or why a typed hex was refused.
pub fn color_note() -> Rectangle {
    let row = color_row();
    Rectangle::new(
        Point::new(INSET, row.y + row.height),
        Size::new(TEXT_WIDTH, ROW_HEIGHT),
    )
}

/// The page's height from the header's top: down to the note's line and a section's gap.
pub fn content_height() -> f32 {
    color_note().y + ROW_HEIGHT + SECTION_GAP
}

/// What a press at `point` lands on while the wheel edits `target`: a theme's key, the key
/// giving the colour back, the hex box or the menu (named for the hint line; each takes its own
/// presses) or a part of the wheel.
pub fn hit(point: Point, target: ColorTarget) -> Option<Hit> {
    if let Some(theme) = ThemeId::ALL
        .into_iter()
        .find(|&theme| theme_key(theme).contains(point))
    {
        return Some(Hit::Theme(theme));
    }
    if target_box().contains(point) {
        return Some(Hit::ColorChoice);
    }
    if own_color_key().contains(point) {
        return Some(Hit::OwnColor(target));
    }
    if hex_box().contains(point) {
        return Some(Hit::Hex(target));
    }
    wheel().part_at(point).map(|part| Hit::Wheel(target, part))
}

/// What the Theme page shows.
pub struct ThemePage {
    /// The theme every window draws in.
    pub theme: ThemeId,
    /// What the wheel edits.
    pub target: ColorTarget,
    /// The target's colour drawn now, and as the wheel holds it (a grey keeps the hue it was
    /// picked at), and whether it is the user's own rather than the theme's or the channel's.
    pub drawn: Color,
    pub color: Hsv,
    pub own: bool,
    /// The hex box's text: what is typed into it, else the colour's `#RRGGBB`.
    pub hex: String,
    /// Why the hex typed last was refused, until the colour next changes or more is typed.
    pub hex_refused: Option<String>,
}

/// What the page's canvas keeps between events: what the cursor was over last, and the part of
/// the accent's wheel a press holds until it is let go of.
#[derive(Debug, Default)]
pub struct PageState {
    hovered: Option<Hover>,
    dragging: Option<WheelPart>,
}

impl ThemePage {
    /// The page under the header (which stays over it while it scrolls): a canvas as tall as its
    /// content, the menu choosing what the wheel edits and the colour's hex box over it.
    pub fn view<'a>(self) -> Element<'a, Message> {
        let height = content_height() - HEADER_HEIGHT;
        let choice = target_box();
        let menu = pick_list(ColorTarget::ALL, Some(self.target), |target| {
            Message::Settings(SettingsMessage::ColorTargetChosen(target))
        })
        .text_size(size::LABEL)
        .text_line_height(iced::Pixels(HEX_LINE_HEIGHT))
        .padding([(choice.height - HEX_LINE_HEIGHT) / 2.0, 6.0])
        .width(choice.width)
        .style(style::chooser)
        .menu_style(style::chooser_menu);
        let hex = hex_box();
        let input = text_input("#RRGGBB", &self.hex)
            .id(HEX_INPUT)
            .on_input(|typed| Message::Settings(SettingsMessage::HexTyped(typed)))
            .on_submit(Message::Settings(SettingsMessage::HexEntered))
            .size(size::LABEL)
            .line_height(iced::Pixels(HEX_LINE_HEIGHT))
            .padding([(hex.height - HEX_LINE_HEIGHT) / 2.0, 4.0])
            .width(hex.width)
            .style(style::hex_box);
        let page = canvas::Canvas::new(self).width(Fill).height(height);
        stack![
            page,
            pin(menu).x(choice.x).y(choice.y - HEADER_HEIGHT),
            pin(input).x(hex.x).y(hex.y - HEADER_HEIGHT)
        ]
        .width(Fill)
        .height(height)
        .into()
    }

    /// The "Theme" section: its title, then a key per theme, the one shown on a button's face
    /// and the others dim on the page, as the header's tabs show their pages, each with its
    /// set's swatch before its name. Why not the accent the ticked boxes take: a theme is a
    /// choice of one, and the General page's screenshot probe holds that a scripted run ticks
    /// nothing in the accent.
    fn draw_keys(&self, frame: &mut Frame) {
        let palette = style::palette();
        let title = theme_title();
        let at = Point::new(title.x, title.center_y());
        text(frame, "Theme", at, palette.text, size::TITLE);
        for theme in ThemeId::ALL {
            let key = theme_key(theme);
            let color = if theme == self.theme {
                frame.fill_rectangle(key.position(), key.size(), palette.button);
                palette.text
            } else {
                palette.text_secondary
            };
            // Edged 1 px in the lit edges' colour, so a panel as dark as the page shows on a key
            // lit or not (the buttons' edge, which it was, all but vanished on Graphite's page).
            let swatch = set_swatch(theme);
            frame.fill_rectangle(swatch.position(), swatch.size(), palette.line_strong);
            for (index, fill) in theme.swatch().into_iter().enumerate() {
                let at = Point::new(swatch.x + 1.0 + SET_SQUARE * index as f32, swatch.y + 1.0);
                frame.fill_rectangle(at, Size::new(SET_SQUARE, SET_SQUARE), fill);
            }
            let at = Point::new(name_x(theme), key.center_y());
            text(frame, theme.title(), at, color, size::LABEL);
        }
    }

    /// The "Colours" section: its title (the menu under it is a widget over the canvas,
    /// [`ThemePage::view`]); the swatch in the colour drawn now; the wheel holding it; the key
    /// giving the colour back, on a button's face while the theme's or the channel's own is
    /// drawn, as a theme's key is while its theme is, dim while the user's own is; and the line
    /// under the row: why a typed hex was refused, else whether the colour reads poorly where it
    /// is drawn ([`ColorTarget::reads_poorly`]), a note and no refusal. The hex box is a text
    /// box over the canvas too.
    fn draw_color(&self, frame: &mut Frame) {
        let palette = style::palette();
        let title = colors_title();
        let at = Point::new(title.x, title.center_y());
        text(frame, "Colours", at, palette.text, size::TITLE);
        // Edged 1 px in the buttons' edge colour, so a swatch of a dark colour still shows.
        let swatch = color_swatch();
        frame.fill_rectangle(swatch.position(), swatch.size(), palette.button_edge);
        frame.fill_rectangle(
            Point::new(swatch.x + 1.0, swatch.y + 1.0),
            Size::new(swatch.width - 2.0, swatch.height - 2.0),
            self.drawn,
        );
        wheel().draw(frame, self.color);
        let key = own_color_key();
        let color = if self.own {
            palette.text_secondary
        } else {
            frame.fill_rectangle(key.position(), key.size(), palette.button);
            palette.text
        };
        centered(frame, self.target.own_label(), key.center(), color);
        let low = self.target.reads_poorly(self.drawn);
        let note = match &self.hex_refused {
            Some(why) => Some((why.as_str(), style::WARNING)),
            None if low => Some((self.target.low_contrast(), palette.text_secondary)),
            None => None,
        };
        if let Some((words, color)) = note {
            let line = color_note();
            if let Some(words) = fit_width(words, line.width, size::LABEL) {
                let at = Point::new(line.x, line.center_y());
                text(frame, &words, at, color, size::LABEL);
            }
        }
    }
}

impl canvas::Program<Message> for ThemePage {
    type State = PageState;

    fn update(
        &self,
        state: &mut PageState,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let publish =
            |message| Some(canvas::Action::publish(Message::Settings(message)).and_capture());
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let at = page_point(cursor.position_in(bounds)?);
                state.dragging = match hit(at, self.target) {
                    Some(Hit::Wheel(_, part)) => Some(part),
                    _ => None,
                };
                publish(SettingsMessage::Pressed(at))
            }
            // A drag of the wheel is followed wherever the cursor goes (over the hex box, off
            // the page) until the button is let go of.
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.dragging.is_some() => {
                let at = cursor.land().position_from(bounds.position())?;
                let part = state.dragging?;
                publish(SettingsMessage::WheelDragged(part, page_point(at)))
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if state.dragging.is_some() =>
            {
                state.dragging = None;
                publish(SettingsMessage::WheelReleased)
            }
            // Landed: the cursor over the hex box or the menu above still names it in the hint
            // line.
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let now = cursor
                    .land()
                    .position_in(bounds)
                    .and_then(|at| hit(page_point(at), self.target))
                    .map(Hover::Setting);
                crate::ui::hint::hover_change(&mut state.hovered, now)
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &PageState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        // Why no cache: the wheel's 360 small segments follow every move of its drag, and a
        // theme or an accent chosen here repaints the page at once.
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), style::palette().bg);
        frame.translate(Vector::new(0.0, -HEADER_HEIGHT));
        self.draw_keys(&mut frame);
        self.draw_color(&mut frame);
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &PageState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging.is_some() {
            return mouse::Interaction::Crosshair;
        }
        match cursor
            .position_in(bounds)
            .and_then(|at| hit(page_point(at), self.target))
        {
            Some(Hit::Wheel(..)) => mouse::Interaction::Crosshair,
            Some(_) => mouse::Interaction::Pointer,
            None => mouse::Interaction::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::text_fit::text_width;

    // By hand from the constants: the "Theme" title 8 px under the 24 px header (32..52), the
    // keys' row under it (52..76: 24 px rows; 22 before), each key the row's whole
    // 24 px (52..76; they were 18 tall, 55..73, under WCAG 2.2's spacing
    // exception), from the left
    // margin, 12, each 4 px after the one before at its own width (68, 88, 58, 78, 68):
    // Slate 12..80, Graphite 84..172, Gilt 176..234, Garnet 238..316, Fjord 320..388 (Graphite
    // and Slate swapped places, Graphite first before), one row
    // ending inside the row's text box (12 + 14 + 8 + 360 = 394, the 14 px box's) and so the
    // Settings window's least width (420); each key's swatch 8 px in, centred in its 24 px
    // ((24 − 10) / 2 = 7: Slate's 20..38 by 59..69, Graphite's 92..110, where it was in the 18
    // px key, 3 + 4) and its name 4 px after the swatch
    // (Slate's from 42, Graphite's from 114). A press on each key is its theme, at its top and
    // foot rows too (52, 75); one between two keys, right of Fjord or on the title none.
    #[test]
    fn the_theme_keys_stand_in_one_row_under_their_title() {
        assert_eq!(theme_title().y, 32.0);
        let row = keys_row();
        assert_eq!((row.y, row.height), (52.0, 24.0));
        let spans = ThemeId::ALL.map(|theme| {
            let key = theme_key(theme);
            (key.x, key.x + key.width)
        });
        assert_eq!(
            spans,
            [
                (12.0, 80.0),
                (84.0, 172.0),
                (176.0, 234.0),
                (238.0, 316.0),
                (320.0, 388.0)
            ]
        );
        for theme in ThemeId::ALL {
            let key = theme_key(theme);
            assert_eq!((key.y, key.height), (52.0, 24.0), "{theme:?}");
            for at in [
                key.center(),
                Point::new(key.center_x(), 52.0),
                Point::new(key.center_x(), 75.0),
            ] {
                assert_eq!(
                    hit(at, ColorTarget::Accent),
                    Some(Hit::Theme(theme)),
                    "{theme:?} at {at:?}"
                );
            }
        }
        assert_eq!(
            set_swatch(ThemeId::Slate),
            Rectangle::new(Point::new(20.0, 59.0), Size::new(18.0, 10.0))
        );
        assert_eq!(
            set_swatch(ThemeId::Graphite),
            Rectangle::new(Point::new(92.0, 59.0), Size::new(18.0, 10.0))
        );
        assert_eq!(ThemeId::ALL.map(name_x), [42.0, 114.0, 206.0, 268.0, 350.0]);
        let last = theme_key(ThemeId::Fjord);
        assert!(
            last.x + last.width <= LABEL_X + TEXT_WIDTH,
            "inside the text box"
        );
        let least = crate::ui::settings_panel::WINDOW_MIN.width;
        assert!(LABEL_X + TEXT_WIDTH <= least, "inside the least window");
        for (at, what) in [
            (Point::new(82.0, 64.0), "between Slate and Graphite"),
            (Point::new(392.0, 64.0), "right of Fjord"),
            (Point::new(20.0, 42.0), "the title"),
        ] {
            assert_eq!(hit(at, ColorTarget::Accent), None, "{what}");
        }
    }

    // Every theme's key holds its swatch and its name, in the app's font, with 8 px either side
    // and 4 between them.
    #[test]
    fn every_theme_fits_its_key() {
        for theme in ThemeId::ALL {
            let name = text_width(theme.title(), size::LABEL);
            let needed = KEY_PAD + SET_SWATCH.width + SET_SWATCH_GAP + name + KEY_PAD;
            let width = theme_key(theme).width;
            assert!(needed <= width, "{}: {needed} in {width}", theme.title());
        }
    }

    // By hand: the "Colours" title 8 px under the keys' row (84..104), the menu choosing what
    // the wheel edits in the row under it (104..128), 18 px tall centred in it, 12..132 by
    // 107..125 (120 px: "Piano roll notes" and the menu's 30 px of padding and arrow); the
    // colour's row a row lower, from the left margin, 128..292 (the 160 px wheel and 2 either
    // side): the swatch centred in the row's first 24 px, 12..48 by 131..149, the wheel 12 px
    // right of it at 60, 131 (centre 140, 211; its square 98..182 by 169..253), the hex box 12
    // px right of the wheel, 232..304 by 131..149, the key giving the colour back a row (24)
    // under it, 232..336 by 155..173 (104 px: "Channel's own"), and the note's line under the
    // row, 292..316; the page ends a section's gap under it, at 324. (Pins moved 24 px down with
    // the menu's row: the row started at 104, the swatch at 107, the centre at 187, the note's
    // line at 268, the page's end at 300; the key was 96 wide, for "Theme's own" alone.) A
    // press on the ring (72 px right of the centre, its middle: 212, 211) is the ring, one on
    // the square's centre the square, one on the hex box, the key or the menu those, each
    // naming the target the wheel edits; one on the swatch, the gap inside the ring (48 px over
    // the centre), the note's line or the title nothing.
    #[test]
    fn the_colour_row_follows_the_keys_under_its_menu() {
        assert_eq!(colors_title().y, 84.0);
        assert_eq!(
            target_box(),
            Rectangle::new(Point::new(12.0, 107.0), Size::new(120.0, 18.0))
        );
        let row = color_row();
        assert_eq!((row.x, row.y, row.height), (12.0, 128.0, 164.0));
        assert_eq!(
            color_swatch(),
            Rectangle::new(Point::new(12.0, 131.0), Size::new(36.0, 18.0))
        );
        let wheel = wheel();
        assert_eq!(wheel.at, Point::new(60.0, 131.0));
        assert_eq!(wheel.center(), Point::new(140.0, 211.0));
        assert_eq!(
            wheel.square(),
            Rectangle::new(Point::new(98.0, 169.0), Size::new(84.0, 84.0))
        );
        assert_eq!(
            hex_box(),
            Rectangle::new(Point::new(232.0, 131.0), Size::new(72.0, 18.0))
        );
        assert_eq!(
            own_color_key(),
            Rectangle::new(Point::new(232.0, 155.0), Size::new(104.0, 18.0))
        );
        assert_eq!((color_note().y, color_note().height), (292.0, 24.0));
        assert_eq!(content_height(), 324.0);
        let key = own_color_key();
        assert!(key.x + key.width <= row.x + row.width, "inside the row");
        for target in ColorTarget::ALL {
            let at = |x, y| hit(Point::new(x, y), target);
            assert_eq!(at(212.0, 211.0), Some(Hit::Wheel(target, WheelPart::Ring)));
            assert_eq!(
                at(140.0, 211.0),
                Some(Hit::Wheel(target, WheelPart::Square))
            );
            assert_eq!(at(240.0, 139.0), Some(Hit::Hex(target)));
            assert_eq!(at(240.0, 164.0), Some(Hit::OwnColor(target)));
            assert_eq!(at(20.0, 115.0), Some(Hit::ColorChoice));
            for (x, y, what) in [
                (20.0, 139.0, "the swatch"),
                (140.0, 163.0, "inside the ring"),
                (20.0, 302.0, "the note"),
                (20.0, 94.0, "the title"),
            ] {
                assert_eq!(at(x, y), None, "{what}");
            }
        }
    }

    // The menu and the colour's row (24 + 164 = 188 px) fit the page's least room, the Settings
    // window's least height less its header and its foot line, 240 − 24 − 22 = 194, so a least
    // window scrolls the menu and the whole wheel into view.
    #[test]
    fn the_colour_row_fits_the_least_window() {
        use crate::ui::layout::HINT_BAR_HEIGHT;
        let least = crate::ui::settings_panel::WINDOW_MIN.height - HEADER_HEIGHT - HINT_BAR_HEIGHT;
        assert_eq!(least, 194.0);
        let menu_and_row = ROW_HEIGHT + ACCENT_ROW_HEIGHT;
        assert!(menu_and_row <= least, "{menu_and_row}");
    }

    // The hex box holds any colour's "#RRGGBB" with 4 px either side, the key giving the colour
    // back its words with 8 px either side ("Theme's own" 76.76 px, "Channel's own" 84.09 at
    // 12 px), the menu its longest row with its 30 px of padding and arrow ("Piano roll notes"
    // 87.41) and the note's line either note (209.54 px at most), in the app's font: each hex
    // digit six times over, so the widest digit's six (ASCII is shaped without kerning) fit too.
    #[test]
    fn the_colours_words_fit_their_boxes() {
        for digit in "0123456789ABCDEF".chars() {
            let hex = format!("#{}", digit.to_string().repeat(6));
            let width = text_width(&hex, size::LABEL);
            assert!(width + 8.0 <= HEX_BOX_WIDTH, "{hex}: {width}");
        }
        for target in ColorTarget::ALL {
            let own = text_width(target.own_label(), size::LABEL);
            assert!(
                own + 16.0 <= OWN_COLOR_WIDTH,
                "{}: {own}",
                target.own_label()
            );
            let row = text_width(&target.to_string(), size::LABEL);
            assert!(row + 30.0 <= TARGET_BOX_WIDTH, "{target}: {row}");
            let note = text_width(target.low_contrast(), size::LABEL);
            assert!(note <= TEXT_WIDTH, "{note}");
        }
    }

    // The low-contrast note's rule by hand (tests/fixtures/gen_accent_expected.py's pairs):
    // the accent against Graphite's panel, #767676 2.9996 (a note) and #777777 3.0425 (none);
    // the notes against both of the roll's row tones, the white keys' #34373D binding (it is the
    // lighter), #7F7F7F 2.9801 (a note) and #808080 3.0214 (none); every channel's own colour
    // reads at 3:1 or more on both tones in every theme (the lowest, the first channel's red on
    // Graphite's, 3.203), so "Channel's own" never shows the note.
    #[test]
    fn the_note_under_the_row_follows_what_the_wheel_sets() {
        let known: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/accent_expected.json"
        )))
        .expect("accent_expected.json");
        let colour = |side: &serde_json::Value| {
            crate::ui::palette::Rgb::parse(side["hex"].as_str().expect("hex"))
                .expect("a colour")
                .color()
        };
        for (target, pair) in [
            (ColorTarget::Accent, &known["contrast"]),
            (ColorTarget::Notes, &known["note"]["contrast"]),
        ] {
            assert!(
                target.reads_poorly(colour(&pair["below"])),
                "{target} below"
            );
            assert!(
                !target.reads_poorly(colour(&pair["above"])),
                "{target} above"
            );
        }
        assert_eq!(
            known["targets"],
            serde_json::json!(ColorTarget::ALL.map(|target| target.to_string()))
        );
        for theme in ThemeId::ALL {
            crate::ui::palette::set_theme(theme);
            let rows = &known["note"]["rows"][theme.name()];
            let hex = |color: Color| crate::ui::palette::Rgb::of(color).hex()[1..].to_owned();
            let palette = style::palette();
            assert_eq!(rows["white"], hex(palette.roll_row_white), "{theme:?}");
            assert_eq!(rows["black"], hex(palette.roll_row_black), "{theme:?}");
            for channel in style::CHANNEL_COLORS {
                assert!(!ColorTarget::Notes.reads_poorly(channel), "{theme:?}");
            }
        }
        crate::ui::palette::set_theme(ThemeId::Graphite);
    }
}
