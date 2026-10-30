//! First use's screenshot probes: the toolbar's panel buttons, the instrument
//! window, the hint bar and the starter document. Positions come from the views' own layout
//! functions, colors from [`crate::ui::style`].

use iced::Point;

use super::tests::{read_ui_shot, rgb8};
use super::{near, Screenshot, TOLERANCE};
use crate::ui::panels::Panel;
use crate::ui::style;

/// How many pixels of each color the icon of `panel`'s button holds: the accent (lit) and the
/// text color (unlit).
fn icon_pixels(shot: &Screenshot, rect: iced::Rectangle) -> (usize, usize) {
    let (accent, text) = (rgb8(style::ACCENT), rgb8(style::TEXT));
    let x = rect.x + 3.0..rect.x + rect.width - 3.0;
    let y = rect.y + 3.0..rect.y + rect.height - 3.0;
    (
        shot.count_matching(x.clone(), y.clone(), |pixel| near(pixel, accent, 12)),
        shot.count_matching(x, y, |pixel| near(pixel, text, 12)),
    )
}

/// The panel buttons of a shot whose panels `open` are open: each a button's face where the
/// toolbar places it, its icon in the accent when its panel is open and in the text color when
/// it is not.
fn assert_panel_buttons(shot: &Screenshot, open: &[Panel]) {
    let face = rgb8(style::BUTTON);
    let width = shot.width as f32 / shot.scale;
    let spread = crate::ui::toolbar::shot_spread(width, false);
    let rects = crate::ui::toolbar::panel_button_rects(&spread);
    assert_eq!(rects.len(), 6, "the panel buttons show");
    for (panel, rect) in rects {
        let corner = shot.pixel(Point::new(rect.x + 3.0, rect.y + 3.0));
        let (lit, unlit) = icon_pixels(shot, rect);
        println!("{panel:?} at {rect:?}: corner {corner:02X?}, accent {lit}, text {unlit}");
        assert!(near(corner, face, TOLERANCE), "{panel:?}'s button face");
        if open.contains(&panel) {
            assert!(lit > 4 && unlit == 0, "{panel:?} is open: its icon is lit");
        } else {
            assert!(
                unlit > 4 && lit == 0,
                "{panel:?} is closed: its icon is unlit"
            );
        }
    }
}

// F1 with the rack alone at 1000 x 560: six panel buttons right of the typing keyboard, the
// rack's lit.
#[test]
#[ignore = "needs a screenshot of F1: scripts/ui-check.sh takes one and runs this"]
fn a1_the_open_panels_button_is_lit_and_the_others_are_not() {
    assert_panel_buttons(&read_ui_shot(), &[Panel::Rack]);
}

// The new document with the Library, the rack and the playlist (a first start's panels) at
// 1000 x 560: their three buttons lit, the piano roll's, the mixer's and the settings' not.
#[test]
#[ignore = "needs a screenshot with --panels browser,rack,playlist: scripts/ui-check.sh takes one and runs this"]
fn a1_the_first_start_panels_buttons_are_lit() {
    assert_panel_buttons(
        &read_ui_shot(),
        &[Panel::Browser, Panel::Rack, Panel::Playlist],
    );
}

// The starter document (tests/fixtures/a6_starter.json, the kit copied into the data folder)
// with the rack alone at 1000 x 560: four rows, each name drawn bright on its button and none
// of its 16 steps lit.
#[test]
#[ignore = "needs a screenshot of the starter document: scripts/ui-check.sh takes one and runs this"]
fn a6_the_starter_document_shows_four_kit_rows_with_no_step_lit() {
    use super::{probe_row, Tone};
    use crate::ui::layout;
    let shot = read_ui_shot();
    let unlit: Vec<Tone> = (0..16).map(|step| Tone::Unlit(step / 4 % 2)).collect();
    for row in 0..4 {
        assert_eq!(probe_row(&shot, row, 16), unlit, "row {row} lights no step");
        let name = layout::name_button_rect(row);
        let top = layout::rack_in_window(name.position());
        let bright = shot.count_matching(
            top.x..top.x + name.width,
            top.y..top.y + name.height,
            |pixel| pixel.iter().all(|&channel| channel > 0x90),
        );
        println!("row {row}: {bright} bright pixels on its name");
        assert!(bright > 10, "row {row}'s name is drawn");
    }
    // No fifth row: the add button's row follows the four.
    let below = layout::rack_in_window(layout::add_plugin_rect(4).position());
    let face = shot.pixel(Point::new(below.x + 2.0, below.y + 2.0));
    assert!(near(face, rgb8(style::BUTTON), TOLERANCE), "{face:?}");
}

// The empty document (one silent "Sampler") with the rack alone at 1000 x 560: right of the
// row's last step, its hint in the dim text color, on the row's line and not above it.
#[test]
#[ignore = "needs a screenshot of the empty document: scripts/ui-check.sh takes one and runs this"]
fn a2_an_empty_sampler_row_shows_its_hint_right_of_its_steps() {
    use crate::ui::layout;
    let shot = read_ui_shot();
    let row = layout::name_button_rect(0);
    let top = layout::rack_in_window(row.position()).y;
    let left = layout::rack_size(1, 16).width;
    let dim = rgb8(style::TEXT_DIM);
    let on_line = shot.count_matching(left..1000.0, top..top + row.height, |pixel| {
        near(pixel, dim, 24)
    });
    let above = shot.count_matching(left..1000.0, top - 8.0..top, |pixel| near(pixel, dim, 24));
    println!("hint pixels right of the steps: {on_line}, above the row: {above}");
    assert!(on_line > 40, "the hint shows");
    assert_eq!(above, 0);
}

// "+ Instrument…" opened over tests/fixtures/p7_catalogue.json and the four test sets at
// 1000 x 560 (positions from crate::ui::picker): the sets' title on the list's ground, then
// the first set's band in the first row tone and the second's in the other.
#[test]
#[ignore = "needs a screenshot of the instrument picker: scripts/ui-check.sh takes one and runs this"]
fn a2_the_instrument_picker_shows_its_sections() {
    use crate::ui::picker::{list_origin, ROW_HEIGHT, ROW_TONES};
    let shot = read_ui_shot();
    let origin = list_origin(iced::Size::new(1000.0, 560.0));
    let at = |row: usize| {
        shot.pixel(Point::new(
            origin.x + 5.0,
            origin.y + (row as f32 + 0.5) * ROW_HEIGHT,
        ))
    };
    let (title, first, second) = (at(0), at(1), at(2));
    println!("title row {title:02X?}, set rows {first:02X?} {second:02X?}");
    assert!(
        near(title, rgb8(style::RACK_WELL), TOLERANCE),
        "the title has no band"
    );
    assert!(near(first, rgb8(ROW_TONES[0]), TOLERANCE));
    assert!(near(second, rgb8(ROW_TONES[1]), TOLERANCE));
}

// p3_roll with the piano roll alone at 1000 x 560 and the hint bar on (--hints on): the bar is
// the panels' ground in the last 22 px; the roll above it gives up that room, its velocity lane
// (the well's color) ending where the bar starts. The ground is sampled 3 px under the bar's
// top (6d, pin moved from its middle: the computer keyboard plays notes by default now, and
// the bar holds its line, whose 12 px text keeps clear of the bar's top 5 px).
#[test]
#[ignore = "needs a screenshot of p3_roll with --hints on: scripts/ui-check.sh takes one and runs this"]
fn a5_the_hint_bar_sits_under_the_panels() {
    use crate::ui::layout::HINT_BAR_HEIGHT;
    let shot = read_ui_shot();
    let height = shot.height as f32 / shot.scale;
    let bar = height - HINT_BAR_HEIGHT;
    let (panel, well) = (rgb8(style::PANEL), rgb8(style::RACK_WELL));
    for x in [5.0, 500.0, 990.0] {
        let inside = shot.pixel(Point::new(x, bar + 3.0));
        assert!(
            near(inside, panel, TOLERANCE),
            "the bar at x {x}: {inside:02X?}"
        );
    }
    let lane = shot.pixel(Point::new(990.0, bar - 3.0));
    println!("bar from y {bar}; the lane above it {lane:02X?}");
    assert!(
        near(lane, well, TOLERANCE),
        "the lane ends where the bar starts"
    );
}

/// The instrument window's sound map of the kind of `map`, in a screenshot of that window
/// alone (`--screenshot-window instrument`): its left edge (the window's padding), its top, and
/// how much wider than its own layout it is drawn to fill the window across. The top is the
/// first of three rows down the window showing the map: for pads, its ground near its right
/// end (a map of one pad leaves the rest bare); for keys, which fill it, the first white key's
/// face (a set over that key). Nothing above the map is drawn in either color: the window's
/// ground, the strip of controls, buttons and edges differ by more than the tolerance, and
/// text is never brighter than the text color, which is darker than a white key.
fn sound_map(shot: &Screenshot, map: &crate::ui::instrument_window::Map) -> (Point, f32) {
    use crate::ui::instrument_window::{span, Map, KEY_WIDTH, PADDING};
    let width = shot.width as f32 / shot.scale - 2.0 * PADDING;
    let across = width / span(map);
    let (probe, color) = match map {
        Map::Pads(_) => (PADDING + width - 5.0, rgb8(style::RACK_WELL)),
        Map::Keys { .. } => (PADDING + KEY_WIDTH / 2.0 * across, rgb8(style::KEY_WHITE)),
    };
    let bottom = shot.height as f32 / shot.scale;
    let shows = |y: f32| near(shot.pixel(Point::new(probe, y)), color, TOLERANCE);
    let mut y = PADDING;
    while y + 3.0 < bottom && !(shows(y) && shows(y + 1.0) && shows(y + 2.0)) {
        y += 1.0;
    }
    println!("the sound map's top at y {y}, {width} px across");
    assert!(y + 3.0 < bottom, "the window shows its sound map");
    (Point::new(PADDING, y), across)
}

/// A pad map of one pad: the pad's face where the window lays the first pad, the map's ground
/// beside it.
fn assert_one_pad(shot: &Screenshot) {
    use crate::ui::instrument_window::{pad_rect, Map, PAD, PAD_GAP};
    let (map, across) = sound_map(shot, &Map::Pads(Vec::new()));
    let pad = pad_rect(0);
    let face = shot.pixel(Point::new(
        map.x + (pad.x + 30.0) * across,
        map.y + pad.y + 26.0,
    ));
    // Where the second pad would start.
    let beside = shot.pixel(Point::new(
        map.x + (PAD.width + PAD_GAP + 20.0) * across,
        map.y + 10.0,
    ));
    println!("pad {face:02X?}, beside it {beside:02X?}");
    assert!(near(face, rgb8(style::BUTTON), TOLERANCE), "the pad");
    assert!(
        near(beside, rgb8(style::RACK_WELL), TOLERANCE),
        "one pad alone"
    );
}

// The test set "Round robin" (two zones on key 60: a drum) in its instrument window, opened at
// start in a window of its own and shot alone: one pad. Twice, with the renderer the toolkit
// picks and with the software one forced.
#[test]
#[ignore = "needs a screenshot of p11_rr's instrument window (--open-instrument channel:1 --screenshot-window instrument): scripts/ui-check.sh takes one and runs this"]
fn a4_a_drum_set_shows_pads() {
    assert_one_pad(&read_ui_shot());
}

// The starter kit's Kick 909 (the starter document's first row) in its instrument window: one
// pad, as a kit drum.
#[test]
#[ignore = "needs a screenshot of a6_starter's instrument window (--open-instrument channel:1 --screenshot-window instrument): scripts/ui-check.sh takes one and runs this"]
fn a4_a_kit_sound_shows_a_pad() {
    assert_one_pad(&read_ui_shot());
}

// The test set "Layers" (roots 48, 60 and 72) in its instrument window: a keyboard from key 36,
// its first white key lit as inside the set, the root 48's dot on its white key (the eighth),
// and a black key over the first two.
#[test]
#[ignore = "needs a screenshot of p11_layers's instrument window (--open-instrument channel:1 --screenshot-window instrument): scripts/ui-check.sh takes one and runs this"]
fn a4_a_pitched_set_shows_a_keyboard_with_its_roots() {
    use crate::ui::instrument_window::{Map, KEY_WIDTH, MAP_HEIGHT};
    let shot = read_ui_shot();
    let keys = Map::Keys {
        low: 36,
        zones: Vec::new(),
        roots: Vec::new(),
    };
    let (map, across) = sound_map(&shot, &keys);
    let white = shot.pixel(Point::new(
        map.x + KEY_WIDTH / 2.0 * across,
        map.y + MAP_HEIGHT - 30.0,
    ));
    let black = shot.pixel(Point::new(map.x + KEY_WIDTH * across, map.y + 20.0));
    let dot = shot.pixel(Point::new(
        map.x + (7.0 * KEY_WIDTH + KEY_WIDTH / 2.0) * across,
        map.y + MAP_HEIGHT - 8.0,
    ));
    println!("white {white:02X?}, black {black:02X?}, root dot {dot:02X?}");
    assert!(
        near(white, rgb8(style::KEY_WHITE), TOLERANCE),
        "a white key"
    );
    assert!(
        near(black, rgb8(style::KEY_BLACK), TOLERANCE),
        "a black key"
    );
    assert!(near(dot, rgb8(style::ACCENT), 12), "the root's dot");
}

// B2: p3_roll (two channels) with the rack, the playlist, the piano roll and the mixer at
// 1000 x 560 and no hint bar: too short for every least height, so the roll keeps its 230 px
// where `panels::stack_heights` puts it, its grid nine whole rows of the row tones (white
// #34373D, black #2D3035) under the divider's 6 px band and its strip, and its lane the well's
// color under the grid. (Pin moved: five rows over the 128 px lane before; the 64 px lane
// left the grid 126 px, ten rows, and the 10 px scroll bar 116 px, nine.
// Resizable dividers, pin moved: 224 px before the divider's band over the roll, which its
// least height counts, so the grid keeps its nine rows 6 px lower.)
#[test]
#[ignore = "needs a screenshot of p3_roll with four panels: scripts/ui-check.sh takes one and runs this"]
fn b2_the_piano_roll_keeps_its_grid_under_three_other_panels() {
    use crate::ui::layout::{rack_canvas_size, RACK_HEADER_HEIGHT, TOOLBAR_HEIGHT};
    use crate::ui::panels::{stack_heights, Kept, DIVIDER_BAND, MIN_ROLL_HEIGHT};
    use crate::ui::piano_layout::{DIVIDER_HEIGHT, LANE_HEIGHT, ROW_HEIGHT, STRIP_HEIGHT};
    let shot = read_ui_shot();
    let stacked = [Panel::Rack, Panel::Playlist, Panel::PianoRoll, Panel::Mixer];
    let rack = RACK_HEADER_HEIGHT + rack_canvas_size(2, 16).height;
    let heights = stack_heights(&stacked, rack, &Kept::default(), 560.0 - TOOLBAR_HEIGHT);
    assert_eq!(heights[2], MIN_ROLL_HEIGHT + DIVIDER_BAND);
    // The panels above it in whole pixels, as the window lays them out, and the divider's band
    // on its top edge.
    let roll_top = TOOLBAR_HEIGHT + heights[0].floor() + heights[1].floor() + DIVIDER_BAND;
    let grid = roll_top + STRIP_HEIGHT..roll_top + MIN_ROLL_HEIGHT - DIVIDER_HEIGHT - LANE_HEIGHT;
    let rows = (grid.end - grid.start) / ROW_HEIGHT;
    let (white, black) = ([0x34, 0x37, 0x3D], [0x2D, 0x30, 0x35]);
    let inside = grid.start + 1.0..grid.end - 1.0;
    let toned = shot.count_matching(60.0..290.0, inside.clone(), |pixel| {
        near(pixel, white, TOLERANCE) || near(pixel, black, TOLERANCE)
    });
    let area = (230.0 * (inside.end - inside.start) * shot.scale * shot.scale) as usize;
    let lane_top = grid.end + DIVIDER_HEIGHT;
    let lane = shot.pixel(Point::new(990.0, lane_top + LANE_HEIGHT / 2.0));
    println!(
        "roll from y {roll_top}, grid y {grid:?} ({rows} rows): {toned} of {area} pixels in \
         the row tones; lane {lane:02X?}"
    );
    assert_eq!(rows.floor(), 9.0);
    assert!(toned * 10 > area * 6, "the grid shows its rows");
    assert!(
        near(lane, rgb8(style::RACK_WELL), TOLERANCE),
        "the lane under the grid"
    );
}

// B4: p8_notice with no audio device at 1000, 960, 930, 900 and 880 px (the p8 shots): along
// y 12 (6d K5, pin moved from y 20: the buttons are 40 px tall from y 8), Edit is one box where
// the toolbar places it, the menus' spacing right of File and left of Help (File first since
// 2026-10-27: Edit stood a line right of Retry, File after it).
#[test]
#[ignore = "needs screenshots of p8_notice with no audio device: scripts/ui-check.sh takes them and runs this"]
fn b4_the_edit_button_is_whole_between_retry_and_file() {
    let shot = read_ui_shot();
    let width = shot.width as f32 / shot.scale;
    let spread = crate::ui::toolbar::shot_spread(width, true);
    let [_, edit, _, _] = crate::ui::toolbar::retry_edit_file_and_help(&spread);
    let ground = rgb8(style::PANEL);
    let boxes = shot.runs_along(12.0, |pixel| !near(pixel, ground, TOLERANCE));
    println!("Edit {edit:?}; boxes along y 12: {boxes:?}");
    let whole = boxes.iter().any(|found| {
        (found.start - edit.start).abs() <= 1.0 && (found.end - edit.end).abs() <= 1.0
    });
    assert!(whole, "Edit is not whole between Retry and File");
}

// B4: F1 with the Edit menu shown at start (--show edit-menu) at 1000 x 560: the menu's box in
// the panels' ground over the rack's well (60 px under the toolbar, below the rack's header),
// its right edge under Edit's, the well right of it.
#[test]
#[ignore = "needs a screenshot of F1 with --show edit-menu: scripts/ui-check.sh takes one and runs this"]
fn b4_the_edit_menu_opens_under_the_edit_button() {
    use crate::ui::layout::TOOLBAR_HEIGHT;
    let shot = read_ui_shot();
    let width = shot.width as f32 / shot.scale;
    let right = width - crate::ui::toolbar::shot_spread(width, false).edit_menu_right();
    let y = TOOLBAR_HEIGHT + 60.0;
    let inside = shot.pixel(Point::new(right - 4.0, y));
    let outside = shot.pixel(Point::new(right + 4.0, y));
    println!("menu's right edge at x {right}: inside {inside:02X?}, outside {outside:02X?}");
    assert!(
        near(inside, rgb8(style::PANEL), TOLERANCE),
        "the menu's box"
    );
    assert!(
        near(outside, rgb8(style::RACK_WELL), TOLERANCE),
        "the menu ends under Edit's right edge"
    );
}
