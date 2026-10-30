//! The Settings window's Keys page: the note layout chooser with Reset to defaults, a drawing of
//! the computer keyboard showing what each key does while it plays notes (the note it plays,
//! the octave, velocity, sustain and all-notes-off keys, the keys among the notes that play
//! nothing), and the table of every shortcut: its action, its keys (red, naming the other
//! action, when two share one) and what works while the keyboard plays notes. A press on a row
//! waits for the next chord pressed in the Settings window, which becomes the row's (Esc gives
//! up). Positions are in the page's own coordinates, its top under the window's header; the
//! drawing, the hit test and the screenshot probe read them.

use iced::alignment::{Horizontal, Vertical};
use iced::keyboard::key::Code;
use iced::widget::canvas::{self, Event, Frame, Geometry, Text};
use iced::{mouse, Color, Element, Point, Rectangle, Renderer, Size, Theme};

use crate::app::Message;
use crate::keys::{Action, Claim, KeyControl, Keys, Labels, NoteLayout, WhilePlaying};
use crate::ui::key_names::key_name;
use crate::ui::settings_layout::{CHECKBOX, INSET, ROW_HEIGHT, SECTION_TITLE_HEIGHT};
use crate::ui::settings_panel::SettingsMessage;
use crate::ui::style;
use crate::ui::text_fit::fit_width;

pub const TITLE_SIZE: f32 = 13.0;
pub const TEXT_SIZE: f32 = 12.0;
/// The size of the letters and notes on the drawn keys.
const KEY_TEXT_SIZE: f32 = 10.0;

/// The note layout chooser: its title, then a row per layout.
const LAYOUT_TOP: f32 = 8.0;
/// Where a row's words start, after its round mark.
const LABEL_X: f32 = INSET + CHECKBOX + 8.0;
const CHOICE_WIDTH: f32 = 300.0;
/// Reset to defaults, right of the chooser.
const RESET: Rectangle = Rectangle {
    x: 340.0,
    y: LAYOUT_TOP + SECTION_TITLE_HEIGHT + 2.0,
    width: 170.0,
    height: 20.0,
};

/// The drawn keyboard: its title, then four rows of keys a unit apart.
const PICTURE_TITLE: f32 = 80.0;
const PICTURE_TOP: f32 = PICTURE_TITLE + SECTION_TITLE_HEIGHT;
/// A key's pitch and side: 32 px from one to the next, 30 px each.
pub const KEY_UNIT: f32 = 32.0;
const KEY_GAP: f32 = 2.0;
/// The line under the drawing saying what its colors mean.
const LEGEND_TOP: f32 = PICTURE_TOP + 4.0 * KEY_UNIT + 4.0;

/// The table: its title, the header row, then a row per action and Esc's.
const TABLE_TITLE: f32 = LEGEND_TOP + ROW_HEIGHT + 8.0;
const TABLE_HEAD: f32 = TABLE_TITLE + SECTION_TITLE_HEIGHT;
const ROWS_TOP: f32 = TABLE_HEAD + ROW_HEIGHT;
/// The columns: action, keys, what works while the keyboard plays notes; the row's width.
pub const ACTION_X: f32 = INSET;
pub const KEYS_X: f32 = 230.0;
pub const PLAYING_X: f32 = 470.0;
pub const ROW_WIDTH: f32 = 680.0;
/// The page's width: the table's rows (the widest it draws) and a margin as wide as its left.
/// Fixed, so the page scrolls across a Settings window narrower than it.
pub const PAGE_WIDTH: f32 = ROW_WIDTH + INSET;

/// The keys the drawing shows, a row each, as a US keyboard lays them out: each key's place
/// and width in units from the row's start.
const PICTURE: [&[(Code, f32, f32)]; 4] = [
    &[
        (Code::Backquote, 0.0, 1.0),
        (Code::Digit1, 1.0, 1.0),
        (Code::Digit2, 2.0, 1.0),
        (Code::Digit3, 3.0, 1.0),
        (Code::Digit4, 4.0, 1.0),
        (Code::Digit5, 5.0, 1.0),
        (Code::Digit6, 6.0, 1.0),
        (Code::Digit7, 7.0, 1.0),
        (Code::Digit8, 8.0, 1.0),
        (Code::Digit9, 9.0, 1.0),
        (Code::Digit0, 10.0, 1.0),
        (Code::Minus, 11.0, 1.0),
        (Code::Equal, 12.0, 1.0),
        (Code::Backspace, 13.0, 2.0),
    ],
    &[
        (Code::Tab, 0.0, 1.5),
        (Code::KeyQ, 1.5, 1.0),
        (Code::KeyW, 2.5, 1.0),
        (Code::KeyE, 3.5, 1.0),
        (Code::KeyR, 4.5, 1.0),
        (Code::KeyT, 5.5, 1.0),
        (Code::KeyY, 6.5, 1.0),
        (Code::KeyU, 7.5, 1.0),
        (Code::KeyI, 8.5, 1.0),
        (Code::KeyO, 9.5, 1.0),
        (Code::KeyP, 10.5, 1.0),
        (Code::BracketLeft, 11.5, 1.0),
        (Code::BracketRight, 12.5, 1.0),
        (Code::Backslash, 13.5, 1.5),
    ],
    &[
        (Code::KeyA, 1.75, 1.0),
        (Code::KeyS, 2.75, 1.0),
        (Code::KeyD, 3.75, 1.0),
        (Code::KeyF, 4.75, 1.0),
        (Code::KeyG, 5.75, 1.0),
        (Code::KeyH, 6.75, 1.0),
        (Code::KeyJ, 7.75, 1.0),
        (Code::KeyK, 8.75, 1.0),
        (Code::KeyL, 9.75, 1.0),
        (Code::Semicolon, 10.75, 1.0),
        (Code::Quote, 11.75, 1.0),
        (Code::Enter, 12.75, 2.25),
    ],
    &[
        (Code::KeyZ, 2.25, 1.0),
        (Code::KeyX, 3.25, 1.0),
        (Code::KeyC, 4.25, 1.0),
        (Code::KeyV, 5.25, 1.0),
        (Code::KeyB, 6.25, 1.0),
        (Code::KeyN, 7.25, 1.0),
        (Code::KeyM, 8.25, 1.0),
        (Code::Comma, 9.25, 1.0),
        (Code::Period, 10.25, 1.0),
        (Code::Slash, 11.25, 1.0),
    ],
];

/// What a press on the page lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeysHit {
    Layout(NoteLayout),
    Reset,
    /// An action's row of the table.
    Action(Action),
}

/// The note layout chooser's title.
pub fn layout_title() -> Rectangle {
    title_at(LAYOUT_TOP)
}

/// The row of `layout` in the chooser: its round mark, then its name.
pub fn layout_row(layout: NoteLayout) -> Rectangle {
    let index = NoteLayout::ALL
        .iter()
        .position(|&shown| shown == layout)
        .unwrap_or(0);
    Rectangle::new(
        Point::new(
            0.0,
            LAYOUT_TOP + SECTION_TITLE_HEIGHT + ROW_HEIGHT * index as f32,
        ),
        Size::new(LABEL_X + CHOICE_WIDTH - 20.0, ROW_HEIGHT),
    )
}

/// The round mark of `layout`'s row, filled while it is the one chosen.
pub fn layout_mark(layout: NoteLayout) -> Rectangle {
    let row = layout_row(layout);
    Rectangle::new(
        Point::new(INSET, row.y + (ROW_HEIGHT - CHECKBOX) / 2.0),
        Size::new(CHECKBOX, CHECKBOX),
    )
}

pub fn reset_button() -> Rectangle {
    RESET
}

/// The drawing's title.
pub fn picture_title() -> Rectangle {
    title_at(PICTURE_TITLE)
}

/// The drawn key of `code`; none for a key the drawing leaves out.
pub fn key_rect(code: Code) -> Option<Rectangle> {
    PICTURE.iter().enumerate().find_map(|(row, keys)| {
        keys.iter()
            .find(|(key, ..)| *key == code)
            .map(|&(_, at, width)| {
                Rectangle::new(
                    Point::new(INSET + at * KEY_UNIT, PICTURE_TOP + row as f32 * KEY_UNIT),
                    Size::new(width * KEY_UNIT - KEY_GAP, KEY_UNIT - KEY_GAP),
                )
            })
    })
}

/// The line saying what the drawing's colors mean.
pub fn legend() -> Rectangle {
    Rectangle::new(
        Point::new(INSET, LEGEND_TOP),
        Size::new(ROW_WIDTH - INSET, ROW_HEIGHT),
    )
}

/// The table's title and its header row.
pub fn table_title() -> Rectangle {
    title_at(TABLE_TITLE)
}

pub fn table_head() -> Rectangle {
    Rectangle::new(
        Point::new(0.0, TABLE_HEAD),
        Size::new(ROW_WIDTH, ROW_HEIGHT),
    )
}

/// The table's row `index`: the actions in [`Action::ALL`]'s order, then Esc's.
pub fn table_row(index: usize) -> Rectangle {
    Rectangle::new(
        Point::new(0.0, ROWS_TOP + ROW_HEIGHT * index as f32),
        Size::new(ROW_WIDTH, ROW_HEIGHT),
    )
}

/// The line under the table saying when the piano roll's keys win over the others'.
pub fn roll_note() -> Rectangle {
    table_row(Action::ALL.len() + 1)
}

/// What [`roll_note`] says.
pub const ROLL_NOTE: &str =
    "Piano roll keys win over it with notes selected, or after a click in it, keyboard on or off";

/// The line under [`roll_note`]: what a MIDI keyboard's controls beside its keys play.
pub fn midi_note() -> Rectangle {
    table_row(Action::ALL.len() + 2)
}

/// What [`midi_note`] says.
pub const MIDI_NOTE: &str = "A MIDI keyboard: its sustain pedal (CC 64) holds notes as Tab does, its pitch bend bends them (Bend in the instrument window), its mod wheel (CC 1) opens a filter that is on, else adds vibrato; a controller link on CC 1 or 64 takes it instead";

/// The page's height: the table's last row (Esc's), the lines under it and a margin.
pub fn content_height() -> f32 {
    midi_note().y + ROW_HEIGHT + 8.0
}

/// What a press at `point` lands on.
pub fn hit(point: Point) -> Option<KeysHit> {
    if let Some(layout) = NoteLayout::ALL
        .into_iter()
        .find(|&layout| layout_row(layout).contains(point))
    {
        return Some(KeysHit::Layout(layout));
    }
    if reset_button().contains(point) {
        return Some(KeysHit::Reset);
    }
    Action::ALL
        .into_iter()
        .enumerate()
        .find(|&(index, _)| table_row(index).contains(point))
        .map(|(_, action)| KeysHit::Action(action))
}

fn title_at(top: f32) -> Rectangle {
    Rectangle::new(
        Point::new(INSET, top),
        Size::new(ROW_WIDTH - INSET, SECTION_TITLE_HEIGHT),
    )
}

/// What a drawn key shows while the keyboard plays notes: its fill, its words' color, and the
/// note or control it names under its letter.
pub fn key_face(claim: Claim, base: u8) -> (Color, Color, String) {
    match claim {
        Claim::Note(offset) => {
            let key = base.saturating_add(offset);
            // The black keys of an octave: C# D# F# G# A#.
            if matches!(key % 12, 1 | 3 | 6 | 8 | 10) {
                (style::KEY_BLACK, style::TEXT, key_name(key))
            } else {
                (style::KEY_WHITE, style::KEY_LABEL, key_name(key))
            }
        }
        Claim::Control(control) => {
            let words = match control {
                KeyControl::OctaveDown => "Oct\u{2212}",
                KeyControl::OctaveUp => "Oct+",
                KeyControl::VelocityDown => "Vel\u{2212}",
                KeyControl::VelocityUp => "Vel+",
                KeyControl::Sustain => "Sustain",
                KeyControl::AllNotesOff => "All off",
            };
            (style::BUTTON, style::ACCENT, words.to_owned())
        }
        // Darker than the page's ground: among the notes, playing none, its shortcut off.
        Claim::Swallowed => (style::KNOB_FACE, style::TEXT_DIM, String::new()),
        // A key's face: its shortcut works as ever.
        Claim::Free => (style::BUTTON, style::TEXT_DIM, String::new()),
    }
}

/// The words of what works while the keyboard plays notes, in an action's row.
pub fn while_playing_words(playing: &WhilePlaying) -> String {
    match playing {
        WhilePlaying::Unchanged => "the same".to_owned(),
        WhilePlaying::Suspended => "off while it plays".to_owned(),
        WhilePlaying::Instead(chords) => chords
            .iter()
            .map(crate::keys::Chord::label)
            .collect::<Vec<String>>()
            .join(", "),
    }
}

/// The words of an action's keys and, when another action shares one, its name.
pub fn keys_words(keys: &Keys, action: Action) -> (String, Option<String>) {
    let bound = keys.all(action, ", ");
    let conflicts = keys.conflicts(action);
    let also = (!conflicts.is_empty()).then(|| {
        let names: Vec<&str> = conflicts.iter().map(|other| other.label()).collect();
        format!("also {}", names.join(", "))
    });
    (
        if bound.is_empty() {
            "no key".to_owned()
        } else {
            bound
        },
        also,
    )
}

/// What the Keys page shows.
pub struct KeysPage<'a> {
    pub keys: &'a Keys,
    pub layout: NoteLayout,
    pub labels: &'a Labels,
    /// The key the layout's first note plays now.
    pub base: u8,
    /// The action waiting for its new chord.
    pub rebinding: Option<Action>,
}

impl<'a> KeysPage<'a> {
    pub fn view(self) -> Element<'a, Message> {
        canvas::Canvas::new(self)
            .width(PAGE_WIDTH)
            .height(content_height())
            .into()
    }

    fn draw_page(&self, frame: &mut Frame) {
        frame.fill_rectangle(Point::ORIGIN, frame.size(), style::RACK_WELL);
        title(frame, layout_title(), "Note layout");
        for layout in NoteLayout::ALL {
            let mark = layout_mark(layout);
            let radius = mark.width / 2.0;
            frame.fill(
                &canvas::Path::circle(mark.center(), radius),
                style::KNOB_RING,
            );
            if layout == self.layout {
                frame.fill(
                    &canvas::Path::circle(mark.center(), radius / 2.0),
                    style::TEXT,
                );
            }
            let row = layout_row(layout);
            text(
                frame,
                layout.name(),
                Point::new(LABEL_X, row.center_y()),
                style::TEXT,
                TEXT_SIZE,
            );
        }
        let reset = reset_button();
        frame.fill_rectangle(reset.position(), reset.size(), style::BUTTON);
        centered(
            frame,
            "Reset keys to defaults",
            reset.center(),
            style::TEXT,
            TEXT_SIZE,
        );
        title(
            frame,
            picture_title(),
            "What each key does while the keyboard plays notes",
        );
        self.draw_picture(frame);
        text(
            frame,
            "White and black: the notes \u{b7} orange: octave, velocity, sustain, all off \u{b7} darkest: nothing, its shortcut off \u{b7} gray: its shortcut",
            Point::new(INSET, legend().center_y()),
            style::TEXT_DIM,
            TEXT_SIZE,
        );
        title(frame, table_title(), "Shortcuts");
        let head = table_head();
        for (x, words) in [
            (ACTION_X, "Action"),
            (KEYS_X, "Keys"),
            (PLAYING_X, "While the keyboard plays notes"),
        ] {
            text(
                frame,
                words,
                Point::new(x, head.center_y()),
                style::TEXT_DIM,
                TEXT_SIZE,
            );
        }
        for (index, action) in Action::ALL.into_iter().enumerate() {
            self.draw_row(frame, index, action);
        }
        let esc = table_row(Action::ALL.len());
        let cells = [
            (ACTION_X, KEYS_X, "Cancel, clear the selection, close"),
            (KEYS_X, PLAYING_X, "Esc (fixed)"),
            (PLAYING_X, ROW_WIDTH, "the same"),
        ];
        for (from, to, words) in cells {
            cell(frame, words, from, to, esc.center_y(), style::TEXT_DIM);
        }
        let note = roll_note();
        cell(
            frame,
            ROLL_NOTE,
            ACTION_X,
            ROW_WIDTH,
            note.center_y(),
            style::TEXT_DIM,
        );
        cell(
            frame,
            MIDI_NOTE,
            ACTION_X,
            ROW_WIDTH,
            midi_note().center_y(),
            style::TEXT_DIM,
        );
    }

    fn draw_picture(&self, frame: &mut Frame) {
        for keys in PICTURE {
            for &(code, ..) in keys {
                let Some(rect) = key_rect(code) else {
                    continue;
                };
                let (fill, ink, names) = key_face(self.layout.claim(code), self.base);
                frame.fill_rectangle(rect.position(), rect.size(), fill);
                let label = self.labels.label(code);
                let corner = Point::new(rect.x + 3.0, rect.y + 2.0);
                frame.fill_text(Text {
                    content: label,
                    position: corner,
                    color: ink,
                    size: KEY_TEXT_SIZE.into(),
                    ..Text::default()
                });
                if let Some(names) = fit_width(&names, rect.width - 2.0, KEY_TEXT_SIZE) {
                    let at = Point::new(rect.center_x(), rect.y + rect.height - 8.0);
                    centered(frame, &names, at, ink, KEY_TEXT_SIZE);
                }
            }
        }
    }

    fn draw_row(&self, frame: &mut Frame, index: usize, action: Action) {
        let row = table_row(index);
        if index % 2 == 1 {
            frame.fill_rectangle(row.position(), row.size(), style::PANEL);
        }
        let y = row.center_y();
        cell(frame, action.label(), ACTION_X, KEYS_X, y, style::TEXT);
        if self.rebinding == Some(action) {
            frame.stroke(
                &canvas::Path::rectangle(row.position(), row.size()),
                canvas::Stroke::default()
                    .with_color(style::ACCENT)
                    .with_width(1.0),
            );
            let waiting = "Press the new keys\u{2026} (Esc gives up)";
            cell(frame, waiting, KEYS_X, ROW_WIDTH, y, style::ACCENT);
            return;
        }
        let (bound, also) = keys_words(self.keys, action);
        let (shown, color) = match &also {
            Some(also) => (format!("{bound} \u{b7} {also}"), style::CONFLICT),
            None => (bound, style::TEXT),
        };
        cell(frame, &shown, KEYS_X, PLAYING_X, y, color);
        let playing = self.keys.while_playing(action, self.layout, self.labels);
        let words = while_playing_words(&playing);
        let color = match playing {
            WhilePlaying::Unchanged => style::TEXT_DIM,
            _ => style::TEXT,
        };
        cell(frame, &words, PLAYING_X, ROW_WIDTH, y, color);
    }
}

/// `words` in the column from `from` to `to`, cut to fit it. Why cut as well as clipped: the
/// software renderer draws a clipped frame's text whole.
fn cell(frame: &mut Frame, words: &str, from: f32, to: f32, y: f32, color: Color) {
    if let Some(words) = fit_width(words, to - from - 8.0, TEXT_SIZE) {
        text(frame, &words, Point::new(from, y), color, TEXT_SIZE);
    }
}

fn title(frame: &mut Frame, rect: Rectangle, words: &str) {
    text(
        frame,
        words,
        Point::new(rect.x, rect.center_y()),
        style::TEXT,
        TITLE_SIZE,
    );
}

fn text(frame: &mut Frame, words: &str, at: Point, color: Color, size: f32) {
    frame.fill_text(Text {
        content: words.to_owned(),
        position: at,
        color,
        size: size.into(),
        align_y: Vertical::Center,
        ..Text::default()
    });
}

fn centered(frame: &mut Frame, words: &str, at: Point, color: Color, size: f32) {
    frame.fill_text(Text {
        content: words.to_owned(),
        position: at,
        color,
        size: size.into(),
        align_x: Horizontal::Center.into(),
        align_y: Vertical::Center,
        ..Text::default()
    });
}

impl canvas::Program<Message> for KeysPage<'_> {
    /// What the cursor was over last.
    type State = Option<crate::ui::hint::Hover>;

    fn update(
        &self,
        hovered: &mut Option<crate::ui::hint::Hover>,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                cursor.position_in(bounds).map(|at| {
                    canvas::Action::publish(Message::Settings(SettingsMessage::KeysPressed(at)))
                        .and_capture()
                })
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let now = cursor
                    .position_in(bounds)
                    .and_then(hit)
                    .map(crate::ui::hint::Hover::Keys);
                crate::ui::hint::hover_change(hovered, now)
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Option<crate::ui::hint::Hover>,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        // Why no cache: drawn while the page is shown, which changes with each key bound.
        let mut frame = Frame::new(renderer, bounds.size());
        self.draw_page(&mut frame);
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &Option<crate::ui::hint::Hover>,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.position_in(bounds).and_then(hit).is_some() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::panels::Panel;

    // By hand from the units: the chooser's rows at 28 and 50, Reset right of them; A is the
    // third row's first key at 12 + 1.75 × 32 = 68, y 100 + 2 × 32 = 164; the table's rows
    // 22 px apart from 304, the first Play / stop, Esc's last and no hit.
    #[test]
    fn the_page_lays_out_the_chooser_the_keyboard_and_the_table() {
        assert_eq!(layout_row(NoteLayout::Piano).y, 28.0);
        assert_eq!(layout_row(NoteLayout::TwoRows).y, 50.0);
        assert_eq!(
            key_rect(Code::KeyA),
            Some(Rectangle::new(
                Point::new(68.0, 164.0),
                Size::new(30.0, 30.0)
            ))
        );
        assert_eq!(key_rect(Code::Backspace).map(|rect| rect.width), Some(62.0));
        assert_eq!(key_rect(Code::F5), None);
        assert_eq!(table_row(0).y, 304.0);
        assert_eq!(
            hit(Point::new(100.0, 310.0)),
            Some(KeysHit::Action(Action::PlayStop))
        );
        let mixer = Action::ALL
            .iter()
            .position(|&action| action == Action::Panel(Panel::Mixer))
            .expect("listed");
        assert_eq!(
            hit(table_row(mixer).center()),
            Some(KeysHit::Action(Action::Panel(Panel::Mixer)))
        );
        assert_eq!(
            hit(table_row(Action::ALL.len()).center()),
            None,
            "Esc's row"
        );
        assert_eq!(hit(reset_button().center()), Some(KeysHit::Reset));
        assert_eq!(
            hit(layout_mark(NoteLayout::TwoRows).center()),
            Some(KeysHit::Layout(NoteLayout::TwoRows))
        );
        // Recording v2: a row more, Capture's (was table_row(36)); the pedal, bend and wheel
        // slice: the MIDI keyboard's line under the roll's (was table_row(37)); help guide H1:
        // a row more, Help's (was table_row(38)).
        assert_eq!(content_height(), table_row(39).y + 30.0);
        assert_eq!(hit(roll_note().center()), None, "the line under the table");
        assert_eq!(hit(midi_note().center()), None, "and the one under it");
    }

    // The keys of the piano row as the drawing shows them from A on C5: A a white C5, W a
    // black C#5, Z the octave down in the accent, R darkest with nothing under its letter, 1 a
    // key's face (its shortcut).
    #[test]
    fn the_drawn_keys_show_what_they_play() {
        let layout = NoteLayout::Piano;
        let face = |code| key_face(layout.claim(code), 60);
        assert_eq!(
            face(Code::KeyA),
            (style::KEY_WHITE, style::KEY_LABEL, "C5".to_owned())
        );
        assert_eq!(
            face(Code::KeyW),
            (style::KEY_BLACK, style::TEXT, "C#5".to_owned())
        );
        assert_eq!(face(Code::KeyZ).2, "Oct\u{2212}");
        assert_eq!(
            face(Code::KeyR),
            (style::KNOB_FACE, style::TEXT_DIM, String::new())
        );
        assert_eq!(face(Code::Digit1).0, style::BUTTON);
        assert_eq!(face(Code::Quote).2, "F6");
    }
}
