//! A theme on the main window: F1 with the Library, the rack and the playlist at 1920 x
//! 1080 taken with `--theme <name>` (scripts/ui-check.sh, the software renderer), five of the
//! palette's colours sampled where the views draw them, against that theme's palette
//! ([`crate::ui::palette`]), which `DAWSOME_UI_THEME` names. Positions come from the toolbar's
//! and the playlist's layout.

use iced::Point;

use super::tests::{read_ui_shot, rgb8};
use super::{near, TOLERANCE};
use crate::ui::layout::{RACK_HEADER_HEIGHT, TOOLBAR_HEIGHT};
use crate::ui::palette::ThemeId;
use crate::ui::panels::Panel;
use crate::ui::playlist_layout::{self, TRACK_HEADER_WIDTH};

// The toolbar's ground over its groups (y 4) at the window's right; the rack's title row, the
// first raised band under the toolbar, at its right; the playlist's strong line at bar 5 (four
// bars of the default 64 px right of the lanes' left edge, past the Library and the track
// heads), solid down a whole lane (a run of at least 32 px, a lane's least height); the
// toolbar's words in the text colour (more than 100 pixels within 12 of it, of the groups' band
// y 9..55); and the rack's panel button, open, its icon in the accent (more than 4 pixels within
// 12). Graphite's shot probed as any other theme fails on its toolbar's ground.
#[test]
#[ignore = "needs a shot of F1 taken with --theme: scripts/ui-check.sh takes one and runs this"]
fn th1_the_main_window_draws_its_themes_colours() {
    let name = std::env::var("DAWSOME_UI_THEME").expect("DAWSOME_UI_THEME names the theme");
    let theme = ThemeId::from_name(&name).expect("a theme's name");
    // This thread draws in the theme, so its accent is the theme's own (no --accent was given).
    crate::ui::palette::set_theme(theme);
    let palette = crate::ui::palette::palette();
    let shot = read_ui_shot();
    let (width, height) = (
        shot.width as f32 / shot.scale,
        shot.height as f32 / shot.scale,
    );
    let ground = shot.pixel(Point::new(width - 20.0, 4.0));
    let title = shot.pixel(Point::new(
        width - 20.0,
        TOOLBAR_HEIGHT + RACK_HEADER_HEIGHT / 2.0,
    ));
    let bar_five =
        crate::ui::browser::WIDTH + TRACK_HEADER_WIDTH + 4.0 * playlist_layout::ZOOM.default;
    let line = (TOOLBAR_HEIGHT as u32..height as u32)
        .map(|y| {
            let pixel = shot.pixel(Point::new(bar_five, y as f32));
            near(pixel, rgb8(palette.bar_line), TOLERANCE)
        })
        .fold((0, 0), |(run, longest), on| {
            if on {
                (run + 1, longest.max(run + 1))
            } else {
                (0, longest)
            }
        })
        .1;
    let words = shot.count_matching(0.0..width, 9.0..55.0, |pixel| {
        near(pixel, rgb8(palette.text), 12)
    });
    let spread = crate::ui::toolbar::shot_spread(width, false);
    let (_, rack) = crate::ui::toolbar::panel_button_rects(&spread)
        .into_iter()
        .find(|&(panel, _)| panel == Panel::Rack)
        .expect("the rack's button");
    let lit = shot.count_matching(
        rack.x + 3.0..rack.x + rack.width - 3.0,
        rack.y + 3.0..rack.y + rack.height - 3.0,
        |pixel| near(pixel, rgb8(crate::ui::palette::accent()), 12),
    );
    println!(
        "{name}: toolbar {ground:02X?}, title row {title:02X?}, bar 5's line {line} px long at x \
         {bar_five}, {words} text px, {lit} accent px on the rack's button"
    );
    assert!(near(ground, rgb8(palette.panel), TOLERANCE), "the panel");
    assert!(near(title, rgb8(palette.raised), TOLERANCE), "raised");
    assert!(line >= 32, "the 4-bar line");
    assert!(words > 100, "the toolbar's text");
    assert!(lit > 4, "the accent on a lit button");
}
