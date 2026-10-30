//! Screenshot probes of the Settings window's Keys page, the keyboard's first-run
//! note and the playlist taking the window's height. Positions come from the views' own layout functions, colors from
//! [`crate::ui::style`].

use iced::keyboard::key::Code;
use iced::Point;

use super::tests::{read_ui_shot, rgb8};
use super::{near, TOLERANCE};
use crate::keys::NoteLayout;
use crate::ui::settings_layout::HEADER_HEIGHT;
use crate::ui::style;

/// A point of the Keys page in the Settings window's shot, the page unscrolled under the
/// window's header. (Under the main window's toolbar too before.)
fn on_page(point: Point) -> Point {
    Point::new(point.x, point.y + HEADER_HEIGHT)
}

// K3: the Settings window on its Keys page at its first size, 720 x 560 (the
// settings panel alone in a 1000 x 800 main window before): the piano row chosen (a dot in
// its mark, none in the two rows'); the drawing's A a white key, W a black one, Z an orange
// control on a key's face, R the darkest (it plays nothing), 1 a key's face (its shortcut).
// Each key is sampled at its right edge's middle, clear of its letter and its note.
#[test]
#[ignore = "needs a screenshot of the Keys page: scripts/ui-check.sh takes one and runs this"]
fn k3_the_keys_page_draws_the_piano_row() {
    use crate::ui::keys_page::{key_rect, layout_mark};
    let shot = read_ui_shot();
    let dot = |layout| shot.pixel(on_page(layout_mark(layout).center()));
    let (piano, rows) = (dot(NoteLayout::Piano), dot(NoteLayout::TwoRows));
    println!("marks: piano {piano:02X?}, two rows {rows:02X?}");
    assert!(
        near(piano, rgb8(style::TEXT), TOLERANCE),
        "the piano row is chosen"
    );
    assert!(
        near(rows, rgb8(style::KNOB_RING), TOLERANCE),
        "the two rows are not"
    );
    for (code, color) in [
        (Code::KeyA, style::KEY_WHITE),
        (Code::KeyW, style::KEY_BLACK),
        (Code::KeyZ, style::BUTTON),
        (Code::KeyR, style::KNOB_FACE),
        (Code::Digit1, style::BUTTON),
    ] {
        let rect = key_rect(code).expect("drawn");
        let at = on_page(Point::new(rect.x + rect.width - 4.0, rect.center_y()));
        let pixel = shot.pixel(at);
        println!("{code:?} at {at:?}: {pixel:02X?}");
        assert!(near(pixel, rgb8(color), TOLERANCE), "{code:?}");
    }
}

// K4: F1 with the rack at 1000 x 560 and the first-run note shown (--callouts on): an orange
// ring around the toolbar's keyboard switch (its left side, 1 px in), the arrow under it, and
// the bubble under that: its orange border at its left edge and the panels' ground inside.
#[test]
#[ignore = "needs a screenshot with --callouts on: scripts/ui-check.sh takes one and runs this"]
fn k4_the_keyboard_note_rings_the_switch_and_points_at_it() {
    use crate::ui::callout::{bubble, ring};
    let shot = read_ui_shot();
    let width = shot.width as f32 / shot.scale;
    let anchor = crate::ui::toolbar::shot_spread(width, false).keyboard_switch();
    let ring = ring(anchor);
    let accent = rgb8(style::ACCENT);
    let side = shot.count_matching(
        ring.x..ring.x + 2.0,
        ring.y + 8.0..ring.y + ring.height - 8.0,
        |pixel| near(pixel, accent, 24),
    );
    let arrow = shot.pixel(Point::new(anchor.center_x(), ring.y + ring.height + 7.0));
    let at = bubble(anchor, width);
    let border = shot.pixel(Point::new(at.x + 1.0, at.y + 30.0));
    let ground = shot.pixel(Point::new(at.x + 6.0, at.y + 30.0));
    println!(
        "switch {anchor:?}: ring side {side} accent pixels, arrow {arrow:02X?}; bubble at \
         {at:?}: border {border:02X?}, ground {ground:02X?}"
    );
    assert!(side > 8, "the ring's side");
    assert!(near(arrow, accent, 24), "the arrow under the switch");
    assert!(near(border, accent, 24), "the bubble's border");
    assert!(
        near(ground, rgb8(style::PANEL), TOLERANCE),
        "the bubble's ground"
    );
}

// K6: k6_forty_tracks (forty tracks, more than any window shows) with the Library, the rack and
// the playlist, the window as large as 1920 x 1080 and the hint bar on: the playlist is the
// stack's flexible panel, so its lanes run down to its scroll bar, which sits on the hint bar.
// Along the row 4 px above the scroll bar, across the lanes right of the track headers, most
// pixels are the lanes' ground (the rest bar and beat lines); 4 px above the hint bar, in the
// scroll bar, they are its trough or its thumb; 4 px into the hint bar it is the panels'
// ground. (The scroll bar moved from the playlist's header to its foot; the lanes
// ran down to the hint bar itself.)
#[test]
#[ignore = "needs a screenshot of k6_forty_tracks at 1920 x 1080: scripts/ui-check.sh takes one and runs this"]
fn k6_the_playlist_reaches_the_hint_bar() {
    use crate::ui::layout::HINT_BAR_HEIGHT;
    use crate::ui::playlist_layout::{TIME_BAR_HEIGHT, TRACK_HEADER_WIDTH};
    let shot = read_ui_shot();
    let (width, height) = (
        shot.width as f32 / shot.scale,
        shot.height as f32 / shot.scale,
    );
    let hints = height - HINT_BAR_HEIGHT;
    let foot = hints - TIME_BAR_HEIGHT;
    let lanes = crate::ui::browser::WIDTH + TRACK_HEADER_WIDTH + 4.0..width - 4.0;
    let span = ((lanes.end - lanes.start) * shot.scale) as usize;
    let lane = rgb8(style::LANE);
    let ground = shot.count_matching(lanes.clone(), foot - 4.0..foot - 3.0, |pixel| {
        near(pixel, lane, TOLERANCE)
    });
    let bar = shot.count_matching(lanes.clone(), hints - 4.0..hints - 3.0, |pixel| {
        near(pixel, rgb8(style::RACK_WELL), TOLERANCE)
            || near(pixel, rgb8(crate::ui::scroll::THUMB), TOLERANCE)
    });
    let under = shot.pixel(Point::new(lanes.start + 40.0, hints + 4.0));
    println!(
        "{width} x {height}: {ground} of {span} lane pixels above the scroll bar, {bar} in it; \
         the hint bar {under:02X?}"
    );
    assert!(
        ground * 10 > span * 7,
        "the lanes run down to the scroll bar"
    );
    assert!(
        bar * 10 > span * 9,
        "the scroll bar runs along the hint bar"
    );
    assert!(
        near(under, rgb8(style::PANEL), TOLERANCE),
        "the hint bar under them"
    );
}
