//! Text cut to fit a width in the window's font: whole when it fits, else its longest start
//! that fits with an ellipsis after it. Why the views cut text themselves rather than leave it
//! to a canvas frame's clip: the software renderer the toolkit falls back to without a usable
//! GPU draws the text of a clipped frame whole, past the clip, so a cut made only by the clip
//! holds on the GPU renderer alone.

use std::borrow::Cow;

use iced::{Renderer, Size};

use crate::ui::fonts;

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
    fonts::load();
    let paragraph = <Renderer as core_text::Renderer>::Paragraph::with_text(core_text::Text {
        content,
        bounds: Size::INFINITE,
        size: size.into(),
        line_height: core_text::LineHeight::default(),
        font: fonts::UI,
        align_x: core_text::Alignment::Default,
        align_y: iced::alignment::Vertical::Top,
        shaping: core_text::Shaping::default(),
        wrapping: core_text::Wrapping::None,
    });
    paragraph.min_width()
}

/// `content` in lines of `room` at text size `size`, broken between words; a word wider than
/// `room` alone takes a line of its own (cut by [`fit_width`] where drawn).
pub fn wrap(content: &str, room: f32, size: f32) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start: Option<usize> = None;
    let mut end = 0;
    for (at, word) in content.split(' ').scan(0, |at, word| {
        let here = *at;
        *at += word.len() + 1;
        Some((here, word))
    }) {
        let word_end = at + word.len();
        match start {
            Some(from) if text_width(&content[from..word_end], size) > room => {
                lines.push(&content[from..end]);
                start = Some(at);
            }
            Some(_) => {}
            None => start = Some(at),
        }
        end = word_end;
    }
    if let Some(from) = start {
        lines.push(&content[from..end]);
    }
    lines
}

/// The width of the widest of `values` at text size `size`, rounded up to a whole pixel: the
/// room a readout keeps whatever it shows, so what follows it stays put as its value changes.
/// Why measured and not a digit count: the bundled font's digits are proportional (its '1' is
/// narrower than its '4').
pub fn widest<S: AsRef<str>>(values: impl IntoIterator<Item = S>, size: f32) -> f32 {
    values
        .into_iter()
        .map(|value| text_width(value.as_ref(), size))
        .fold(0.0, f32::max)
        .ceil()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Lines break between words at the room: the whole text when it fits, else each line the
    // most words that fit, none lost or doubled; a word wider than the room is a line alone.
    #[test]
    fn text_wraps_between_words_at_its_room() {
        let text = "No MIDI input found. Plug a keyboard in, then restart";
        let whole = text_width(text, 12.0);
        assert_eq!(wrap(text, whole + 1.0, 12.0), vec![text]);
        let lines = wrap(text, whole * 0.6, 12.0);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines.join(" "), text);
        for line in &lines {
            assert!(text_width(line, 12.0) <= whole * 0.6, "{line:?} fits");
        }
        let next = lines[1].split(' ').next().expect("a word");
        assert!(
            text_width(&format!("{} {next}", lines[0]), 12.0) > whole * 0.6,
            "the first line takes every word that fits"
        );
        assert_eq!(
            wrap("a verylongword b", 1.0, 12.0),
            ["a", "verylongword", "b"]
        );
        assert!(wrap("", 100.0, 12.0) == [""]);
    }

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
