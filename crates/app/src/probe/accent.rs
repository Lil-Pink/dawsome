//! Colours of the user's own (scripts/ui-check.sh, the software renderer). An accent
//! (`--accent`): the main window draws its lit controls in it and keeps its content colours,
//! and the Settings window's Theme page shows it on the swatch and on the wheel, while each
//! theme's key keeps its set's own colours. The piano roll's notes' colour (`--note-color`): the
//! channel's notes and stems take it, the other channels' ghosts keep theirs. The colours and
//! the wheel's geometry are `tests/fixtures/gen_accent_expected.py`'s (its probe blue, #3FA7D6,
//! and its notes' yellow-green, #E8F04A, the ones the script passes); positions come from the
//! toolbar's, the Theme page's and the piano roll's layouts.

use iced::Point;

use super::tests::{read_ui_shot, rgb8};
use super::{near, TOLERANCE};
use crate::ui::palette::Rgb;
use crate::ui::panels::Panel;
use crate::ui::style;

/// The generator's known answers.
fn known() -> serde_json::Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/accent_expected.json"
    )))
    .expect("accent_expected.json")
}

/// The probe's blue, as the generator names it.
fn blue(known: &serde_json::Value) -> [u8; 3] {
    let hex = known["probe"]["hex"].as_str().expect("a hex colour");
    Rgb::parse(hex).expect("a colour").0
}

// F1 with the Library, the rack and the playlist at 1920 x 1080, --accent #3FA7D6: the rack's
// panel button, open, draws its icon in the blue (more than 4 pixels within 12 of it, as the themes' probe
// counts its accent) and none in Graphite's orange; the Record button, a content colour, keeps
// its dot RECORD_OFF (not armed: more than 20 pixels within 12 of #6E3030) with no blue in it.
#[test]
#[ignore = "needs a shot of F1 taken with --accent #3FA7D6: scripts/ui-check.sh takes one and runs this"]
fn ac1_the_main_window_draws_the_users_accent_and_keeps_its_content_colours() {
    let shot = read_ui_shot();
    let blue = blue(&known());
    // This thread draws in Graphite with no accent of its own: Graphite's orange.
    let orange = rgb8(crate::ui::palette::accent());
    let width = shot.width as f32 / shot.scale;
    let spread = crate::ui::toolbar::shot_spread(width, false);
    let (_, rack) = crate::ui::toolbar::panel_button_rects(&spread)
        .into_iter()
        .find(|&(panel, _)| panel == Panel::Rack)
        .expect("the rack's button");
    let inside = |rect: iced::Rectangle, color: [u8; 3]| {
        shot.count_matching(
            rect.x + 3.0..rect.x + rect.width - 3.0,
            rect.y + 3.0..rect.y + rect.height - 3.0,
            |pixel| near(pixel, color, 12),
        )
    };
    let record = spread.record_button();
    let (lit, old, dot, dot_blue) = (
        inside(rack, blue),
        inside(rack, orange),
        inside(record, rgb8(style::RECORD_OFF)),
        inside(record, blue),
    );
    println!(
        "rack's button: {lit} blue px, {old} orange px; Record: {dot} RECORD_OFF px, {dot_blue} \
         blue px"
    );
    assert!(lit > 4, "the accent on a lit button");
    assert_eq!(old, 0, "no orange left on it");
    assert!(dot > 20, "Record keeps its own colour");
    assert_eq!(dot_blue, 0, "and takes none of the accent");
}

// The Settings window on its Theme page with --midi-ports none and --accent #3FA7D6
// (positions from crate::ui::theme_page, the page unscrolled at the window's first size: the
// menu choosing what the wheel sets at y 107, the colour's row a row under it at y 128, the
// swatch 12..48 by 131..149, the 160 px wheel at 60, 131, its centre 140, 211): the swatch's
// centre is the blue, and the hue's dot, a white circle on the ring's middle, sits at the
// blue's hue, 198.68°: by the generator's x = cx + 72 cos h, y = cy − 72 sin h (hue 0 at the
// right, counter-clockwise), its offset from the centre is (−68.21, 23.05), so its centre is
// (71.79, 234.05), pixel (71, 234). The centroid of the near-white pixels on the ring (62 to
// 82 px from the centre, clear of the square, whose corners are 59.40 out and white at its top
// left) is that point to within a pixel. (Pins moved with the row to the Theme page and the
// wheel from 120 to 160 px: the row was at 290 on the General page, the dot at (166, 369), the
// band 46 to 62 px; then 3 px down with the Settings rows 24 px: the row at 102,
// the swatch at 104, the centre at 184, the dot's pixel (71, 207); then 24 px down under the
// menu's row: the row at 104, the swatch at 107, the centre at 187, the dot's pixel (71, 210).)
#[test]
#[ignore = "needs a shot of --panels settings --settings-page theme --midi-ports none --accent #3FA7D6 --screenshot-window settings: scripts/ui-check.sh takes one and runs this"]
fn ac2_the_settings_window_shows_the_users_accent_on_the_swatch_and_the_wheel() {
    use crate::ui::theme_page::{color_row, color_swatch, wheel};
    let shot = read_ui_shot();
    let known = known();
    let blue = blue(&known);
    let swatch = color_swatch();
    let center = wheel().center();
    assert_eq!(
        (color_row().y, swatch.y, center),
        (128.0, 131.0, Point::new(140.0, 211.0)),
        "the colour's row"
    );
    let swatch_pixel = shot.pixel(swatch.center());
    let offset = &known["probe"]["offset"];
    let number = |value: &serde_json::Value| value.as_f64().expect("a number") as f32;
    let expected = Point::new(center.x + number(&offset[0]), center.y + number(&offset[1]));
    assert_eq!(
        (expected.x.floor(), expected.y.floor()),
        (71.0, 234.0),
        "the dot's pixel"
    );
    let (mut sum_x, mut sum_y, mut count) = (0.0, 0.0, 0.0);
    for y in (center.y - 84.0) as u32..(center.y + 84.0) as u32 {
        for x in (center.x - 84.0) as u32..(center.x + 84.0) as u32 {
            let (middle_x, middle_y) = (x as f32 + 0.5, y as f32 + 0.5);
            let out = (middle_x - center.x).hypot(middle_y - center.y);
            let white = shot
                .pixel(Point::new(x as f32, y as f32))
                .iter()
                .all(|&value| value >= 0xE0);
            if (62.0..=82.0).contains(&out) && white {
                sum_x += middle_x;
                sum_y += middle_y;
                count += 1.0;
            }
        }
    }
    let found = Point::new(sum_x / count, sum_y / count);
    println!(
        "swatch {swatch_pixel:02X?}; the hue's dot: {count} white px round {found:?}, expected \
         {expected:?}"
    );
    assert!(near(swatch_pixel, blue, TOLERANCE), "the swatch");
    assert!(count > 10.0, "a dot on the ring");
    assert!(
        (found.x - expected.x).abs() <= 1.0 && (found.y - expected.y).abs() <= 1.0,
        "the dot at the blue's hue"
    );
}

// The same shot: each theme's key on the Theme page keeps its set's own swatch whatever accent
// is drawn, its panel square and its accent square (their centres, 4 and 12 px in from the
// swatch's left, 5 px down: Graphite's at (24, 64) and (32, 64), the keys' row 24 px now,
// were (24, 63) and (32, 63)) the set's colours, so none the blue; and Graphite's
// key, the theme shown, on the button's face 3 px in from its right end (97, 64), Slate's on the
// page's ground there (169, 64).
#[test]
#[ignore = "needs a shot of --panels settings --settings-page theme --midi-ports none --accent #3FA7D6 --screenshot-window settings: scripts/ui-check.sh takes one and runs this"]
fn ac3_the_theme_keys_keep_their_sets_own_colours() {
    use crate::ui::palette::ThemeId;
    use crate::ui::theme_page::{set_swatch, theme_key};
    let shot = read_ui_shot();
    let swatch = set_swatch(ThemeId::Graphite);
    assert_eq!(
        (swatch.x + 4.0, swatch.y + 5.0),
        (24.0, 64.0),
        "Graphite's swatch"
    );
    let mut wrong = Vec::new();
    for theme in ThemeId::ALL {
        let swatch = set_swatch(theme);
        for (x, color) in [4.0, 12.0].into_iter().zip(theme.swatch()) {
            let pixel = shot.pixel(Point::new(swatch.x + x, swatch.y + 5.0));
            println!("{theme:?}'s square at {x}: {pixel:02X?}");
            if !near(pixel, rgb8(color), TOLERANCE) {
                wrong.push(format!("{theme:?} at {x}: {pixel:02X?}"));
            }
        }
    }
    let face = |theme| {
        let key = theme_key(theme);
        shot.pixel(Point::new(key.x + key.width - 3.0, key.center_y()))
    };
    let (lit, unlit) = (face(ThemeId::Graphite), face(ThemeId::Slate));
    println!("Graphite's key {lit:02X?}, Slate's {unlit:02X?}");
    assert!(wrong.is_empty(), "the swatches: {wrong:?}");
    assert!(
        near(lit, rgb8(style::palette().button), TOLERANCE),
        "Graphite's key lit"
    );
    assert!(
        near(unlit, rgb8(style::palette().bg), TOLERANCE),
        "Slate's key not"
    );
}

// p3_roll with the piano roll alone at 1000 x 560 and --note-color #E8F04A (the generator's
// probe colour, a yellow-green no channel has): the lead's notes A (tick 0, key 60) and B (tick
// 96, key 64) and their velocity stems are drawn in it, at the points the roll's own probe reads
// them (crate::probe::tests::p3_screenshot_shows_the_piano_roll: the notes' centres (64, 222)
// and (144, 174), the stems' insides (50, 539) and (114, 548)); the kick's ghosts on channel 2
// keep its blue at 0.3 over the white row, #3C576E, at (184, 222) and (248, 222), and none of
// them takes the yellow-green.
#[test]
#[ignore = "needs a shot of p3_roll with --panels piano-roll --note-color #E8F04A: scripts/ui-check.sh takes one and runs this"]
fn ac4_the_piano_roll_draws_its_notes_and_stems_in_the_users_colour() {
    let shot = read_ui_shot();
    let hex = known()["note"]["probe"]["hex"]
        .as_str()
        .expect("a hex colour")
        .to_owned();
    let note = Rgb::parse(&hex).expect("a colour").0;
    let ghost = [0x3C, 0x57, 0x6E];
    let mut wrong = Vec::new();
    for (name, x, y, want) in [
        ("note A", 64.0, 222.0, note),
        ("note B", 144.0, 174.0, note),
        ("stem A", 50.0, 539.0, note),
        ("stem B", 114.0, 548.0, note),
        ("ghost kick 192", 184.0, 222.0, ghost),
        ("ghost kick 288", 248.0, 222.0, ghost),
    ] {
        let pixel = shot.pixel(Point::new(x, y));
        println!("{name} at ({x}, {y}): {pixel:02X?}, want {want:02X?}");
        if !near(pixel, want, TOLERANCE) {
            wrong.push(format!("{name}: {pixel:02X?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}
