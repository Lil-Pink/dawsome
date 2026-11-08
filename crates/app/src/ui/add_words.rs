//! The "+" control under a list's last row, drawn as its words alone: "+ Track" under the
//! playlist's tracks and "+ Instrument…" under the channel rack's channels, so the two look and
//! answer alike. No face and no edge: the secondary text tone at rest, the text tone under the
//! pointer, the disabled tone while nothing more can be added. A press takes the words and
//! [`PAD`] either side, across the whole [`HEIGHT`] row. Why words alone: such a row shows and
//! hides as the room changes, and a face coming and going drew the eye; the words come and go
//! quietly, the row keeping its place.

use iced::alignment::{Horizontal, Vertical};
use iced::widget::canvas::{Frame, Text};
use iced::Rectangle;

use crate::ui::fonts::size;
use crate::ui::style;
use crate::ui::text_fit::text_width;

/// The room a press takes either side of the words.
pub const PAD: f32 = 8.0;
/// The row the words stand in, and the height a press takes.
pub const HEIGHT: f32 = 24.0;

/// How the words look: at rest, lit under the pointer, or dim while nothing more can be added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Resting,
    Lit,
    Disabled,
}

/// The width a press takes for `label`: its words at the labels' size and [`PAD`] either side.
/// Measured each call; a caller asking on every pointer move keeps the answer.
pub fn width(label: &str) -> f32 {
    text_width(label, size::LABEL) + 2.0 * PAD
}

/// `label`'s words centred in `rect`, the place a press takes them, in `look`'s tone.
pub fn draw(frame: &mut Frame, label: &str, rect: Rectangle, look: Look) {
    let palette = style::palette();
    let color = match look {
        Look::Resting => palette.text_secondary,
        Look::Lit => palette.text,
        Look::Disabled => palette.text_disabled,
    };
    frame.fill_text(Text {
        content: label.into(),
        position: rect.center(),
        color,
        size: size::LABEL.into(),
        align_x: Horizontal::Center.into(),
        align_y: Vertical::Center,
        ..Text::default()
    });
}
