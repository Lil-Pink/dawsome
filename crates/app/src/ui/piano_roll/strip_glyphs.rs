//! The piano roll strip's words against the bundled font's character map: every character the
//! strip's fold law measures ([`super::StripShape::needs`]) is one Inter Regular maps to a glyph
//! of its own, so the toolkit never draws one in a face the system falls back to, and no width
//! the law sums depends on the system's fonts. Why: the Notes label was "Notes ▾", and Inter has
//! no U+25BE; the system's face drew it, 7.911 px on one Linux and over 0.78 px wider on Windows,
//! and the fold table, pinned on Linux, failed on Windows (CI, 0.1.3). Read from the font file's
//! own cmap, this check gives the same answer on every system, which a measured width cannot.

use super::*;
use crate::ui::fonts;

/// The glyph `character` maps to in `font`'s Windows Unicode BMP subtable (cmap format 4), or 0
/// (the missing glyph) when it maps none: tests/fixtures/gen_u1_expected.py's `glyph_of`.
fn glyph_of(font: &[u8], character: char) -> u16 {
    let be16 = |at: usize| u16::from_be_bytes([font[at], font[at + 1]]);
    let be32 = |at: usize| {
        u32::from_be_bytes([font[at], font[at + 1], font[at + 2], font[at + 3]]) as usize
    };
    let Ok(code) = u16::try_from(u32::from(character)) else {
        return 0;
    };
    let cmap = (0..usize::from(be16(4)))
        .map(|index| 12 + 16 * index)
        .find(|&record| &font[record..record + 4] == b"cmap")
        .map(|record| be32(record + 8))
        .expect("the font has a cmap");
    let table = (0..usize::from(be16(cmap + 2)))
        .map(|index| cmap + 4 + 8 * index)
        .filter(|&record| be16(record) == 3 && be16(record + 2) == 1)
        .map(|record| cmap + be32(record + 4))
        .find(|&table| be16(table) == 4)
        .expect("a format 4 Windows Unicode subtable");
    let pairs = usize::from(be16(table + 6)) / 2;
    let ends = table + 14;
    let starts = ends + 2 * pairs + 2;
    let deltas = starts + 2 * pairs;
    let ranges = deltas + 2 * pairs;
    for k in 0..pairs {
        if code > be16(ends + 2 * k) {
            continue;
        }
        let start = be16(starts + 2 * k);
        if code < start {
            return 0;
        }
        let delta = be16(deltas + 2 * k);
        let offset = usize::from(be16(ranges + 2 * k));
        if offset == 0 {
            return code.wrapping_add(delta);
        }
        let glyph = be16(ranges + 2 * k + offset + 2 * usize::from(code - start));
        return if glyph == 0 {
            0
        } else {
            glyph.wrapping_add(delta)
        };
    }
    0
}

// The reader against gen_u1_expected.py's, on the bundled Regular: "N" is glyph 300, "…" 1503,
// "●" 1853 and "▼" (U+25BC) 1844, and "▾" (U+25BE), the arrow the Notes label held, maps none.
#[test]
fn the_cmap_reader_gives_what_gen_u1_reads() {
    let font = fonts::REGULAR;
    assert_eq!(glyph_of(font, 'N'), 300);
    assert_eq!(glyph_of(font, '\u{2026}'), 1503);
    assert_eq!(glyph_of(font, '\u{25cf}'), 1853);
    assert_eq!(glyph_of(font, '\u{25bc}'), 1844);
    assert_eq!(glyph_of(font, '\u{25be}'), 0, "Inter has no \u{25be}");
}

// Every word the strip's fold law measures, each character in Inter: the text buttons' (Preview,
// Follow, Fold, Notes, Loop), the Rec light's, the Len readout's (any length: its digits and
// colon), the velocity readout's (0 to 127), the snap, root and scale pickers' entries (the
// pickers' arrow is the toolkit's own icon font, as wide as the text is tall, never a text
// glyph), and a cut name's "…". A channel's name is the user's and is not checked.
#[test]
fn the_strips_words_are_all_in_the_bundled_font() {
    let mut words: Vec<String> = [PREVIEW, FOLLOW, FOLD, NOTES, LOOP, REC_NOTES]
        .map(String::from)
        .to_vec();
    words.push(length_words("0123456789:"));
    words.extend((0..=127).map(velocity_words));
    words.extend(Snap::ALL.iter().map(ToString::to_string));
    words.extend(Root::ALL.iter().map(ToString::to_string));
    words.extend(ScaleMode::ALL.iter().map(ToString::to_string));
    words.push(least_name("Bass 808 low").into_owned());
    let missing: Vec<(String, char)> = words
        .iter()
        .flat_map(|word| word.chars().map(move |c| (word.clone(), c)))
        .filter(|&(_, c)| glyph_of(fonts::REGULAR, c) == 0)
        .collect();
    assert!(missing.is_empty(), "not in Inter: {missing:?}");
}
