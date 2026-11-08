//! The app's own font: Inter 4.1, Regular and SemiBold, built into the programs
//! (`assets/fonts/`, each file pinned in `fonts.lock`), so every window draws the same text on
//! every system. Why: iced 0.14 draws its default sans-serif in whatever the system offers (Noto
//! Sans on one Linux, an unknown face on Windows), so a layout measured on one machine came out
//! wider or narrower on another.
//!
//! [`load`] puts both faces in the toolkit's font database, takes out any face of the same
//! family that came from a file on the system (an Inter the user installed would otherwise answer
//! for "Inter", in its own version), and makes Inter the sans-serif family, so text that names no
//! font draws in it too: a canvas's text defaults to the sans-serif family, which the window's
//! default font ([`UI`], given to `iced::daemon` as `default_font`) does not reach. `main.rs`
//! hands the daemon the same bytes; tests and headless drawing never start it, so what measures
//! or lays out text calls [`load`] first (`text_fit::text_width`, the tests' window).

use std::borrow::Cow;
use std::sync::Once;

use iced::advanced::graphics::text::{cosmic_text::fontdb, font_system};
use iced::font::Weight;
use iced::Font;

/// The family both faces carry (the SemiBold's as its typographic family).
const FAMILY: &str = "Inter";

/// Inter Regular: the window's text. A static, not a const, so every use is one address: the
/// toolkit loads a borrowed font once per address.
pub static REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
/// Inter SemiBold: the text that stands out.
pub static SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");

/// Every window's text.
pub const UI: Font = Font::with_name(FAMILY);
/// Text that stands out: the Help window's headings and bold words.
pub const UI_STRONG: Font = Font {
    weight: Weight::Semibold,
    ..UI
};

/// The type scale: every text size the app draws, in logical pixels (the zoom scales them all).
/// Why a scale and not a size per panel: about ten sizes a pixel apart (9 to 16, and 22) read as
/// noise and could not be checked; a few steps read as a hierarchy, and `scripts/check.sh` refuses
/// a text size anywhere else. Numbers draw in the same face: a readout keeps the width of its
/// widest value and right-aligns (`text_fit::widest`), since the bundled font's digits are
/// proportional.
pub mod size {
    /// Text in a space too small for [`DENSE`], kept from before the scale: the playlist clip's
    /// 10 px title strip (a candidate to grow) and the note names on the instrument window's 14 px
    /// keys.
    pub const TINY: f32 = 9.0;
    /// Numbers and the densest labels: bar numbers, an insert's number, readouts and value tips,
    /// a knob's name, a group's title in capitals, the piano roll's key names.
    pub const DENSE: f32 = 11.0;
    /// Most of the app's words: buttons, list rows, labels, hints, foot lines, captions.
    pub const LABEL: f32 = 12.0;
    /// Running text and the larger rows: the Help window's paragraphs and contents, menus, the
    /// toolbar's text buttons and notices, a rack row's name.
    pub const BODY: f32 = 13.0;
    /// A panel's title in its header strip ("Channel Rack", "Playlist", "Mixer", "Library") and
    /// the piano roll's channel name in its strip: 15 % over [`LABEL`], 12 · 1.15 = 13.8,
    /// rounded to 14. Why not [`BODY`]: a title stands out at 15 % bigger than the labels
    /// beside it, and 13 is 8 % bigger; why not [`TITLE`]: 16 px text (20.8 px of line)
    /// overfills the 24 px header strips.
    pub const PANEL_TITLE: f32 = 14.0;
    /// A window's or a dialog's title, a Help heading, the toolbar's readouts, a section's title
    /// in Settings (13 px until the General page took columns).
    pub const TITLE: f32 = 16.0;
    /// The typing keyboard's octave name beside its switch ("C5"): 5 % under [`TITLE`],
    /// 16 · 0.95 = 15.2, rounded to 15. Why a step of its own: the name at the titles' size read
    /// as heavy as the readouts beside it, so it stands 5 % smaller; nothing else uses it.
    pub const OCTAVE_NAME: f32 = 15.0;
}

/// Loads the bundled faces into the toolkit's font database, once per process; see the module's
/// documentation. Cheap after the first call.
pub fn load() {
    static LOADED: Once = Once::new();
    LOADED.call_once(|| {
        let mut system = font_system()
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let database = system.raw().db_mut();
        let installed: Vec<fontdb::ID> = database
            .faces()
            .filter(|face| !matches!(face.source, fontdb::Source::Binary(_)))
            .filter(|face| face.families.iter().any(|(name, _)| name == FAMILY))
            .map(|face| face.id)
            .collect();
        for id in installed {
            database.remove_face(id);
        }
        database.set_sans_serif_family(FAMILY);
        system.load_font(Cow::Borrowed(REGULAR));
        system.load_font(Cow::Borrowed(SEMIBOLD));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::graphics::text::{cosmic_text, to_attributes};

    /// The known answers `tests/fixtures/gen_u1_expected.py` works out from the font's own
    /// tables.
    fn known() -> serde_json::Value {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/u1_font.json"
        )))
        .expect("u1_font.json")
    }

    /// `fonts.lock`'s `key=` value.
    fn locked(key: &str) -> String {
        include_str!("../../assets/fonts/fonts.lock")
            .lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            .unwrap_or_else(|| panic!("fonts.lock has no {key}="))
            .to_owned()
    }

    /// The name table's version string (name ID 5, Windows Unicode, English US) of a font file.
    fn version_string(data: &[u8]) -> Option<String> {
        let be16 = |at: usize| Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?));
        let be32 = |at: usize| Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?));
        let name = (0..usize::from(be16(4)?))
            .map(|index| 12 + 16 * index)
            .find(|&record| data.get(record..record + 4) == Some(b"name"))
            .and_then(|record| be32(record + 8))? as usize;
        let strings = name + usize::from(be16(name + 4)?);
        (0..usize::from(be16(name + 2)?)).find_map(|index| {
            let at = name + 6 + 12 * index;
            let ids = [be16(at)?, be16(at + 2)?, be16(at + 4)?, be16(at + 6)?];
            if ids != [3, 1, 0x409, 5] {
                return None;
            }
            let start = strings + usize::from(be16(at + 10)?);
            let units: Vec<u16> = data
                .get(start..start + usize::from(be16(at + 8)?))?
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            String::from_utf16(&units).ok()
        })
    }

    /// The width of `content` at `size` px shaped as a canvas shapes its text, in `font`, and the
    /// family of the face its first glyph was drawn from.
    fn shaped(content: &str, size: f32, font: Font) -> (f32, String) {
        load();
        let mut system = font_system().write().expect("the font system");
        let raw = system.raw();
        let mut buffer = cosmic_text::Buffer::new(raw, cosmic_text::Metrics::new(size, size));
        buffer.set_text(
            raw,
            content,
            &to_attributes(font),
            cosmic_text::Shaping::Basic,
            None,
        );
        let run = buffer.layout_runs().next().expect("one line");
        let face = run.glyphs.first().map(|glyph| glyph.font_id);
        let family = face
            .and_then(|id| raw.db().face(id))
            .and_then(|face| face.families.first().map(|(name, _)| name.clone()))
            .unwrap_or_default();
        (run.line_w, family)
    }

    // The face the window resolves for "Inter", Regular and SemiBold, is the bundled file
    // itself, byte for byte, carrying the version string fonts.lock pins (4.1's static faces say
    // "Version 4.001;git-9221beed3"). Why: a user with Inter installed system-wide could
    // otherwise have the font database answer with that copy, in its own version and widths.
    // And text that names no font (the sans-serif family, a canvas's default) draws in it too.
    #[test]
    fn the_ui_font_is_the_bundled_inter() {
        load();
        let version = locked("version");
        assert_eq!(version, known()["version"]);
        let mut system = font_system().write().expect("the font system");
        let database = system.raw().db();
        for (font, bundled) in [(UI, REGULAR), (UI_STRONG, SEMIBOLD)] {
            let attributes = to_attributes(font);
            let query = fontdb::Query {
                families: &[attributes.family],
                weight: attributes.weight,
                stretch: attributes.stretch,
                style: attributes.style,
            };
            let id = database.query(&query).expect("a face for Inter");
            let same = database
                .with_face_data(id, |data, _| data == bundled)
                .expect("the face's data");
            assert!(same, "{font:?} resolves to the bundled file");
            assert_eq!(version_string(bundled).as_deref(), Some(version.as_str()));
        }
        drop(system);
        assert_eq!(shaped("Notes", 12.0, Font::DEFAULT).1, FAMILY);
    }

    // "Notes" at 12 px measures what Inter's own advances give (gen_u1_expected.py: the hmtx
    // advances of N, o, t, e, s, 5716 units of 2048, × 12 = 33.4921875 px), through the toolbar's
    // measure, and as a canvas's text, which names no font, shapes it.
    #[test]
    fn notes_at_12_px_is_as_wide_as_inters_advances_make_it() {
        let want = known()["notes_12px"].as_f64().expect("a width") as f32;
        assert_eq!(want, 33.492_188);
        let measured = crate::ui::text_fit::text_width("Notes", 12.0);
        assert!((measured - want).abs() < 1e-3, "{measured} against {want}");
        let (canvas, family) = shaped("Notes", 12.0, Font::DEFAULT);
        assert_eq!(family, FAMILY);
        assert!((canvas - want).abs() < 1e-3, "{canvas} against {want}");
    }
}
