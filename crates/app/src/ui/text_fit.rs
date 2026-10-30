//! Text cut to fit a width in the window's font: whole when it fits, else its longest start
//! that fits with an ellipsis after it. Why the views cut text themselves rather than leave it
//! to a canvas frame's clip: the software renderer the toolkit falls back to without a usable
//! GPU draws the text of a clipped frame whole, past the clip, so a cut made only by the clip
//! holds on the GPU renderer alone.

use std::borrow::Cow;

use iced::{Font, Renderer, Size};

/// What ends a text cut short.
const ELLIPSIS: char = '\u{2026}';

/// A text as it fits a width.
#[derive(Debug, Clone, PartialEq)]
pub struct Fitted<'n> {
    pub text: Cow<'n, str>,
    /// Cut short, so a tooltip can say it whole.
    pub cut: bool,
}

/// `content` as it fits `room`, `measure` giving a text's width: whole, or its longest start
/// that fits with the ellipsis after it; `None` when not one character does.
pub fn fit<'n>(content: &'n str, room: f32, measure: &impl Fn(&str) -> f32) -> Option<Fitted<'n>> {
    if measure(content) <= room {
        return Some(Fitted {
            text: Cow::Borrowed(content),
            cut: false,
        });
    }
    let ends: Vec<usize> = content
        .char_indices()
        .map(|(at, character)| at + character.len_utf8())
        .collect();
    let cut = |count: usize| format!("{}{ELLIPSIS}", content[..ends[count - 1]].trim_end());
    // `kept` characters fit with the ellipsis (none at first), all of them do not: halve
    // the difference until they meet.
    let (mut kept, mut too_many) = (0, ends.len());
    while too_many - kept > 1 {
        let middle = (kept + too_many) / 2;
        if measure(&cut(middle)) <= room {
            kept = middle;
        } else {
            too_many = middle;
        }
    }
    (kept > 0).then(|| Fitted {
        text: Cow::Owned(cut(kept)),
        cut: true,
    })
}

/// `content` as it fits `room` at text size `size` in the window's font: whole, cut short
/// with an ellipsis, or `None` when not one character fits.
pub fn fit_width(content: &str, room: f32, size: f32) -> Option<Cow<'_, str>> {
    fit(content, room, &|text: &str| text_width(text, size)).map(|fitted| fitted.text)
}

/// The width `content` takes on one line at text size `size`, in the window's font.
pub fn text_width(content: &str, size: f32) -> f32 {
    use iced::advanced::text::{self as core_text, Paragraph as _};
    let paragraph = <Renderer as core_text::Renderer>::Paragraph::with_text(core_text::Text {
        content,
        bounds: Size::INFINITE,
        size: size.into(),
        line_height: core_text::LineHeight::default(),
        font: Font::DEFAULT,
        align_x: core_text::Alignment::Default,
        align_y: iced::alignment::Vertical::Top,
        shaping: core_text::Shaping::default(),
        wrapping: core_text::Wrapping::None,
    });
    paragraph.min_width()
}

#[cfg(test)]
mod tests {
    use super::*;

    // At the mixer's name size, in the 40 px a strip at x 756 keeps left of a slot panel at
    // x 800 (its name starts 4 px in): a short name stays whole, a long one is cut to a start
    // of it and an ellipsis that fit the 40 px while one more character would not, and a room
    // narrower than any character holds nothing.
    #[test]
    fn a_name_cut_for_a_room_fits_it_with_nothing_to_spare() {
        let (size, room) = (12.0, 800.0 - (756.0 + 4.0));
        assert_eq!(room, 40.0);
        assert_eq!(fit_width("Bus", room, size).as_deref(), Some("Bus"));
        // No spaces: a cut trims the space before its ellipsis, so "one more character" is
        // then a character further on.
        let name = "Kick-and-snare-room-bus";
        assert!(
            text_width(name, size) > room,
            "the name is longer than the room"
        );
        let cut = fit_width(name, room, size).expect("a start of the name fits");
        let kept = cut.trim_end_matches(ELLIPSIS);
        assert!(cut.ends_with(ELLIPSIS) && name.starts_with(kept) && !kept.is_empty());
        assert!(text_width(&cut, size) <= room);
        let one_more: String = name.chars().take(kept.chars().count() + 1).collect();
        assert!(text_width(&format!("{one_more}{ELLIPSIS}"), size) > room);
        assert_eq!(fit_width(name, 2.0, size), None);
        assert_eq!(fit_width(name, -4.0, size), None);
    }
}
