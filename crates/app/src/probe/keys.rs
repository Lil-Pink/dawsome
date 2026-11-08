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

// The Settings window on its Keys page at its first size, 720 x 560 (the
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
        near(piano, rgb8(style::palette().text), TOLERANCE),
        "the piano row is chosen"
    );
    assert!(
        near(rows, rgb8(style::palette().knob_ring), TOLERANCE),
        "the two rows are not"
    );
    for (code, color) in [
        (Code::KeyA, style::palette().key_white),
        (Code::KeyW, style::palette().key_black),
        (Code::KeyZ, style::palette().button),
        (Code::KeyR, style::palette().knob_face),
        (Code::Digit1, style::palette().button),
    ] {
        let rect = key_rect(code).expect("drawn");
        let at = on_page(Point::new(rect.x + rect.width - 4.0, rect.center_y()));
        let pixel = shot.pixel(at);
        println!("{code:?} at {at:?}: {pixel:02X?}");
        assert!(near(pixel, rgb8(color), TOLERANCE), "{code:?}");
    }
}

// F1 with the rack at 1000 x 560 and the first-run note shown (--callouts on): an orange
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
    let accent = rgb8(style::accent());
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
        near(ground, rgb8(style::palette().panel), TOLERANCE),
        "the bubble's ground"
    );
}

// k6_forty_tracks (forty tracks, more than any window shows) with the Library, the rack and
// the playlist, the window as large as 1920 x 1080 and the hint bar on: the playlist is the
// stack's flexible panel, so its track area runs down to its scroll bar, which sits on the hint
// bar. By hand, the track area starts at y 64 + 132 (the rack: its 24 px header over its two
// channels' rows and the "+ Instrument…" row under them, 8 + 2 · 38 + 24, under half the area)
// + 6 (the divider's band over the playlist, inside its height) + 24 + 28 = 254 and
// ends at the scroll bar, 32 px over the window's foot; the forty tracks' fitted lanes are at
// their 32 px floor (they would share (730 − 25) / 40, 17 px, each), and it shows the rows of
// 32 px it holds whole (n rows take 33n − 1 px) and the well under them. In a 1016 px shot
// (what this desk gives the 1920 x 1080 window) that is 984 − 254 = 730 px, 22 rows (726 − 1
// px) to y 254 + 725 = 979, the well 979..984; at 1080, 794 px, 24 rows to y 1045, the well
// 1045..1048. (Pins moved: from y 230, the rack 108 px with nothing under its rows, 22 rows to
// y 955 in the 1016 px shot, 24 to 1021 at 1080; from y 268, the rack 146 px with the "+" row
// under its rows (8 + 3 · 38), 21 rows to y 960, 23 to 1026 at 1080; before that from y 262, which
// left the divider's band out, 21 rows to y 954; the well's middle row was in the well all
// the same. Rows of 64, 11 and 12, while 64 was the fitted lanes' floor.) Along the row 4 px above the last lane's foot, across the lanes
// right of the track headers, most pixels are the lanes' ground, the lane color or every other
// bar its shade (the rest bar and beat lines);
// across the well's middle row, the well's; 4 px above the hint bar, in the scroll bar, its
// trough or its thumb; 4 px into the hint bar the panels' ground. (The scroll bar moved from
// the playlist's header to its foot, and the lanes ran down to the hint bar itself; then they
// ran down to the scroll bar, the last row cut there, until rows showed whole only.)
#[test]
#[ignore = "needs a screenshot of k6_forty_tracks at 1920 x 1080: scripts/ui-check.sh takes one and runs this"]
fn k6_the_playlist_reaches_the_hint_bar() {
    use crate::ui::layout::{rack_size, HINT_BAR_HEIGHT, RACK_HEADER_HEIGHT, TOOLBAR_HEIGHT};
    use crate::ui::playlist_layout::{
        self, FIT_FLOOR, LANE_GAP, RULER_HEIGHT, TIME_BAR_HEIGHT, TRACK_HEADER_WIDTH,
    };
    let shot = read_ui_shot();
    let (width, height) = (
        shot.width as f32 / shot.scale,
        shot.height as f32 / shot.scale,
    );
    let hints = height - HINT_BAR_HEIGHT;
    let foot = hints - TIME_BAR_HEIGHT;
    let rack = RACK_HEADER_HEIGHT + rack_size(2, 16).height;
    let top = TOOLBAR_HEIGHT
        + rack
        + crate::ui::panels::DIVIDER_BAND
        + playlist_layout::HEADER_HEIGHT
        + RULER_HEIGHT;
    assert_eq!((rack, top), (132.0, 254.0));
    let pitch = FIT_FLOOR + LANE_GAP;
    let rows = ((foot - top + LANE_GAP) / pitch).floor();
    let lanes_foot = top + rows * pitch - LANE_GAP;
    let lanes = crate::ui::browser::WIDTH + TRACK_HEADER_WIDTH + 4.0..width - 4.0;
    let span = ((lanes.end - lanes.start) * shot.scale) as usize;
    // The lanes' ground is the lane color or, every other bar, its shade.
    let (lane, shaded) = (rgb8(style::palette().lane), rgb8(style::palette().lane_alt));
    let ground = shot.count_matching(lanes.clone(), lanes_foot - 4.0..lanes_foot - 3.0, |pixel| {
        near(pixel, lane, TOLERANCE) || near(pixel, shaded, TOLERANCE)
    });
    let middle = ((lanes_foot + foot) / 2.0).floor();
    let well = shot.count_matching(lanes.clone(), middle..middle + 1.0, |pixel| {
        near(pixel, rgb8(style::palette().bg), TOLERANCE)
    });
    let bar = shot.count_matching(lanes.clone(), hints - 4.0..hints - 3.0, |pixel| {
        near(pixel, rgb8(style::palette().bg), TOLERANCE)
            || near(pixel, rgb8(crate::ui::scroll::thumb()), TOLERANCE)
    });
    let under = shot.pixel(Point::new(lanes.start + 40.0, hints + 4.0));
    println!(
        "{width} x {height}: {rows} rows to y {lanes_foot}, {ground} of {span} lane pixels above \
         it, {well} well pixels at y {middle}, {bar} in the scroll bar; the hint bar {under:02X?}"
    );
    assert!(
        foot - lanes_foot >= 1.0,
        "a well under the rows: they end at {lanes_foot}"
    );
    assert!(
        ground * 10 > span * 7,
        "the lanes run down to the last whole row"
    );
    assert!(well * 10 > span * 9, "the well under them, no row cut");
    assert!(
        bar * 10 > span * 9,
        "the scroll bar runs along the hint bar"
    );
    assert!(
        near(under, rgb8(style::palette().panel), TOLERANCE),
        "the hint bar under them"
    );
}
