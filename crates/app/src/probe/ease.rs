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
    let (accent, text) = (rgb8(style::accent()), rgb8(style::palette().text));
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
    let face = rgb8(style::palette().button);
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

// F1 with the rack alone at 1080 x 560 (the panel buttons fold below 1049.292 px since the time
// signature stands beside the tempo): six panel buttons right of the typing keyboard, the
// rack's lit.
#[test]
#[ignore = "needs a screenshot of F1: scripts/ui-check.sh takes one and runs this"]
fn a1_the_open_panels_button_is_lit_and_the_others_are_not() {
    assert_panel_buttons(&read_ui_shot(), &[Panel::Rack]);
}

// The new document with the Library, the rack, the playlist and the mixer (a first start's
// panels, `Panels::FIRST_START`) at 1080 x 560 (the panel buttons fold at 1000): their four
// buttons lit, the piano roll's and the settings' not.
#[test]
#[ignore = "needs a screenshot with --panels browser,rack,playlist,mixer: scripts/ui-check.sh takes one and runs this"]
fn a1_the_first_start_panels_buttons_are_lit() {
    assert_panel_buttons(
        &read_ui_shot(),
        &[Panel::Browser, Panel::Rack, Panel::Playlist, Panel::Mixer],
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
    // No fifth row: under the four the add row (rack y 8 + 4 · 38 = 160..184), "+ Instrument…"
    // its words alone from the name boxes' x, a press taking 28..126.395 by 160..184 (layout's
    // test), in the secondary text's tone, no face (rack (30, 162), 2 px in, the rack's ground
    // bg), and right of them in the row the rack's well, bg too (rack (140, 172)). (Pins moved:
    // the well stood at rack (30, 162), where a fifth row's name button would start, until the
    // button came down under the rows, (28, 162) 100 x 20 with its face at rack (30, 164), for
    // a day.)
    let add = layout::add_instrument_rect(4);
    assert_eq!((add.x, add.y, add.height), (28.0, 160.0, 24.0));
    let face = shot.pixel(layout::rack_in_window(Point::new(30.0, 162.0)));
    assert!(
        near(face, rgb8(style::palette().bg), TOLERANCE),
        "+ Instrument…, no face: {face:?}"
    );
    let top = layout::rack_in_window(add.position());
    let dim = rgb8(style::palette().text_secondary);
    let words = shot.count_matching(
        top.x..top.x + add.width,
        top.y..top.y + add.height,
        |pixel| near(pixel, dim, TOLERANCE),
    );
    println!("+ Instrument…: {words} secondary-toned pixels");
    assert!(words > 5, "the words at rest");
    let ground = shot.pixel(layout::rack_in_window(Point::new(140.0, 172.0)));
    assert!(
        near(ground, rgb8(style::palette().bg), TOLERANCE),
        "{ground:?}"
    );
}

// The empty document (one silent "Sampler") with the rack alone at 1000 x 560: right of the
// row's last step, its hint in the secondary text color, on the row's line and not above it.
#[test]
#[ignore = "needs a screenshot of the empty document: scripts/ui-check.sh takes one and runs this"]
fn a2_an_empty_sampler_row_shows_its_hint_right_of_its_steps() {
    use crate::ui::layout;
    let shot = read_ui_shot();
    let row = layout::name_button_rect(0);
    let top = layout::rack_in_window(row.position()).y;
    let left = layout::rack_size(1, 16, 4).width;
    let dim = rgb8(style::palette().text_secondary);
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
    use crate::ui::picker::{list_origin, ROW_HEIGHT};
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
        near(title, rgb8(style::palette().bg), TOLERANCE),
        "the title has no band"
    );
    let tones = style::palette().row_tones;
    assert!(near(first, rgb8(tones[0]), TOLERANCE));
    assert!(near(second, rgb8(tones[1]), TOLERANCE));
}

// p3_roll with the piano roll alone at 1000 x 560 and the hint bar on (--hints on): the bar is
// the panels' ground in the last 22 px; the roll above it gives up that room, its velocity lane
// (the meters' ground, #141619; the well's before its lit edges) ending where the bar starts,
// read 3 px above it, over its foot edge. The ground is sampled 3 px under the bar's
// top (6d, pin moved from its middle: the computer keyboard plays notes by default now, and
// the bar holds its line, whose 12 px text keeps clear of the bar's top 5 px).
#[test]
#[ignore = "needs a screenshot of p3_roll with --hints on: scripts/ui-check.sh takes one and runs this"]
fn a5_the_hint_bar_sits_under_the_panels() {
    use crate::ui::layout::HINT_BAR_HEIGHT;
    let shot = read_ui_shot();
    let height = shot.height as f32 / shot.scale;
    let bar = height - HINT_BAR_HEIGHT;
    let (panel, well) = (
        rgb8(style::palette().panel),
        rgb8(style::palette().meter_ground),
    );
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

// p3_roll with the playlist and, at the stack's foot, the piano roll in a 1000 x 418 window
// with the hint bar on: 418 − 64 − 22 = 332 px for 180 + 230 = 410 of least heights, so the
// roll gives way to its least before the stack scrolls, 146 px and its band, 152, from
// y 64 + 180 = 244 to 396 (one pixel taller would fit; at 1000 x 480 it had been cut by 16 px
// under the bar, the stack scrolling). Under the band (244..250), the strip (..278) and the
// ruler (..294) the grid shows its five rows (294..354) over the 10 px scroll bar (..364);
// the lane is y 364..396 and the hint bar 396..418: stem A (velocity 100, 25 px from lane y 7,
// x 48..52) is the lead's red at its middle (y 383.5) and on the row over the lane's foot edge
// (y 394); the lane's ground is the meters' (#141619) at x 970 over that edge; its first row
// (y 364) and its last (y 395) are the lit edge (#5C626B) at x 970 and its last at x 50, over
// the stem, and at x 995 (no scroll bar of the stack's there: it does not scroll); the hint
// bar's first row under it (y 396) and 3 px under the lane's foot are the bar's ground.
#[test]
#[ignore = "needs a screenshot of p3_roll with the playlist and the roll and --hints on: scripts/ui-check.sh takes one and runs this"]
fn a5b_the_roll_at_the_stacks_foot_keeps_its_lane_above_the_hint_bar() {
    use crate::ui::layout::{HINT_BAR_HEIGHT, TOOLBAR_HEIGHT};
    use crate::ui::panels::{stack_heights, Kept, DIVIDER_BAND, MIN_HEIGHT, ROLL_GIVEN_WAY_HEIGHT};
    use crate::ui::piano_layout::{View, LANE_HEIGHT, ZOOM};
    let shot = read_ui_shot();
    let height = shot.height as f32 / shot.scale;
    assert_eq!(height, 418.0, "the shot's window");
    let area = height - TOOLBAR_HEIGHT - HINT_BAR_HEIGHT;
    let stacked = [Panel::Playlist, Panel::PianoRoll];
    let heights = stack_heights(&stacked, 0.0, &Kept::default(), area);
    assert_eq!(heights, [MIN_HEIGHT, ROLL_GIVEN_WAY_HEIGHT + DIVIDER_BAND]);
    assert_eq!(heights, [180.0, 152.0]);
    let bar = height - HINT_BAR_HEIGHT;
    let lane_top = TOOLBAR_HEIGHT + heights[0] + heights[1] - LANE_HEIGHT;
    assert_eq!(
        (lane_top, bar),
        (364.0, 396.0),
        "the lane ends where the bar starts"
    );
    let view = View {
        top: 69,
        scroll: dawsome_engine::time::Ticks(0),
        zoom: ZOOM.default,
        step: dawsome_engine::time::Ticks(24),
        signature: dawsome_engine::time::TimeSignature::COMMON,
        rows: crate::ui::piano_layout::Rows::ALL,
    };
    let a = dawsome_engine::project::Note {
        channel: dawsome_engine::project::ChannelId(1),
        start: dawsome_engine::time::Ticks(0),
        length: dawsome_engine::time::Ticks(48),
        velocity: 0.78125,
        key: 60,
    };
    let stem = view.stem_rect(&a, 100).expect("in view");
    assert_eq!((stem.x, stem.y, stem.height), (48.0, 7.0, 25.0));
    let x = stem.center_x();
    let (red, lane, edge, ground) = (
        [0xE5, 0x53, 0x3D],
        rgb8(style::palette().meter_ground),
        rgb8(style::palette().line_strong),
        rgb8(style::palette().panel),
    );
    assert_eq!((lane, edge), ([0x14, 0x16, 0x19], [0x5C, 0x62, 0x6B]));
    let probes = [
        ("stem A", Point::new(x, lane_top + stem.center_y()), red),
        ("stem A over the foot edge", Point::new(x, bar - 2.0), red),
        ("the lane's ground", Point::new(970.0, bar - 2.0), lane),
        ("the lane's top edge", Point::new(970.0, lane_top), edge),
        ("the foot edge", Point::new(970.0, bar - 1.0), edge),
        ("the foot edge over stem A", Point::new(x, bar - 1.0), edge),
        (
            "the foot edge at the right",
            Point::new(995.0, bar - 1.0),
            edge,
        ),
        ("the hint bar's first row", Point::new(970.0, bar), ground),
        ("the bar under the lane", Point::new(x, bar + 3.0), ground),
    ];
    let mut wrong = Vec::new();
    for (name, at, expected) in probes {
        let pixel = shot.pixel(at);
        println!("{name} at ({}, {}): {pixel:02X?}", at.x, at.y);
        if !near(pixel, expected, TOLERANCE) {
            wrong.push(name);
        }
    }
    assert!(wrong.is_empty(), "wrong color at {wrong:?}");
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
        Map::Pads(_) => (PADDING + width - 5.0, rgb8(style::palette().bg)),
        Map::Keys { .. } => (
            PADDING + KEY_WIDTH / 2.0 * across,
            rgb8(style::palette().key_white),
        ),
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
    assert!(
        near(face, rgb8(style::palette().button), TOLERANCE),
        "the pad"
    );
    assert!(
        near(beside, rgb8(style::palette().bg), TOLERANCE),
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
        near(white, rgb8(style::palette().key_white), TOLERANCE),
        "a white key"
    );
    assert!(
        near(black, rgb8(style::palette().key_black), TOLERANCE),
        "a black key"
    );
    assert!(near(dot, rgb8(style::accent()), 12), "the root's dot");
}

// p3_roll (two channels) with the rack, the playlist, the piano roll and the mixer at
// 1000 x 560 and no hint bar: too short for every least height (70 + 180 + 230 + 235 = 715 of
// 496 px), and for them with the roll given way to its 146 px and band (637), so the others
// show at their least and the stack scrolls, unscrolled in the shot: the roll's 152 px where
// `panels::stack_heights` puts them, from y 64 + 70 + 180 = 314, its grid its five whole rows
// (60 px, 364..424) of the row tones under the divider's 6 px band, its strip and its ruler, in
// bar 1 shaded, 5 lighter a channel (white #393C42, black #32353A), and its lane the meters'
// ground under the grid's scroll bar (434..466, between its lit edges), read at its middle at
// x 970, clear of the stack's scroll bar along the window's right edge (990..1000).
// (Pin moved: five rows over the 128 px lane before; the 64 px lane left the grid 126 px, ten
// rows, and the 10 px scroll bar 116 px, nine. Resizable dividers, pin moved: 224 px before the
// divider's band over the roll, which its least height counts, so the grid keeps its nine rows
// 6 px lower. Since the stack scrolls rather than squeeze, the roll starts 92 px lower,
// under the rack's and the playlist's least heights rather than their squeezed 44.6 and 114.7,
// and the lane is read at x 970, was 990. Then the mixer's least 310 → 225, its compact
// strip: the roll's place unchanged, the stack 85 px shorter and still scrolling. Then the
// lane fixed at 32 px without its handle and the 16 px ruler over the grid: the roll's place and
// height unchanged, its grid eleven rows, its rows the shaded bar's. Then the roll giving way
// before the stack scrolls: 152 px, five rows, its lane 78 px higher. Then the mixer's least
// 225 → 235, its route button a 24 px target: the roll's place unchanged.)
#[test]
#[ignore = "needs a screenshot of p3_roll with four panels: scripts/ui-check.sh takes one and runs this"]
fn b2_the_piano_roll_keeps_its_grid_under_three_other_panels() {
    use crate::ui::layout::{rack_size, RACK_HEADER_HEIGHT, TOOLBAR_HEIGHT};
    use crate::ui::panels::{stack_heights, Kept, DIVIDER_BAND, ROLL_GIVEN_WAY_HEIGHT};
    use crate::ui::piano_layout::{DIVIDER_HEIGHT, GRID_TOP, LANE_HEIGHT, ROW_HEIGHT, UNDER_GRID};
    let shot = read_ui_shot();
    let stacked = [Panel::Rack, Panel::Playlist, Panel::PianoRoll, Panel::Mixer];
    let rack = RACK_HEADER_HEIGHT + rack_size(2, 16, 4).height;
    let heights = stack_heights(&stacked, rack, &Kept::default(), 560.0 - TOOLBAR_HEIGHT);
    assert_eq!(heights[2], ROLL_GIVEN_WAY_HEIGHT + DIVIDER_BAND);
    assert_eq!(
        heights,
        [70.0, 180.0, 152.0, 235.0],
        "the roll given way, the others at their least"
    );
    // The panels above it in whole pixels, as the window lays them out, and the divider's band
    // on its top edge.
    let roll_top = TOOLBAR_HEIGHT + heights[0].floor() + heights[1].floor() + DIVIDER_BAND;
    assert_eq!(roll_top, 320.0);
    let grid = roll_top + GRID_TOP..roll_top + ROLL_GIVEN_WAY_HEIGHT - UNDER_GRID;
    let rows = (grid.end - grid.start) / ROW_HEIGHT;
    let (white, black) = ([0x39, 0x3C, 0x42], [0x32, 0x35, 0x3A]);
    let inside = grid.start + 1.0..grid.end - 1.0;
    let toned = shot.count_matching(60.0..290.0, inside.clone(), |pixel| {
        near(pixel, white, TOLERANCE) || near(pixel, black, TOLERANCE)
    });
    let area = (230.0 * (inside.end - inside.start) * shot.scale * shot.scale) as usize;
    let lane_top = grid.end + DIVIDER_HEIGHT;
    let lane = shot.pixel(Point::new(970.0, lane_top + LANE_HEIGHT / 2.0));
    println!(
        "roll from y {roll_top}, grid y {grid:?} ({rows} rows): {toned} of {area} pixels in \
         the row tones; lane {lane:02X?}"
    );
    assert_eq!(rows, 5.0);
    assert!(toned * 10 > area * 6, "the grid shows its rows");
    assert!(
        near(lane, rgb8(style::palette().meter_ground), TOLERANCE),
        "the lane under the grid"
    );
}

// p8_notice with no audio device at 1000, 960, 930, 900 and 880 px (the p8 shots): along
// y 12 (pin moved from y 20: the buttons are 40 px tall from y 8), Edit is one box where
// the toolbar places it, the menus' spacing right of File and left of Help (File first; Edit
// used to stand a line right of Retry, File after it).
#[test]
#[ignore = "needs screenshots of p8_notice with no audio device: scripts/ui-check.sh takes them and runs this"]
fn b4_the_edit_button_is_whole_between_retry_and_file() {
    let shot = read_ui_shot();
    let width = shot.width as f32 / shot.scale;
    let spread = crate::ui::toolbar::shot_spread(width, true);
    let [_, edit, _, _] = crate::ui::toolbar::retry_edit_file_and_help(&spread);
    let ground = rgb8(style::palette().panel);
    let boxes = shot.runs_along(12.0, |pixel| !near(pixel, ground, TOLERANCE));
    println!("Edit {edit:?}; boxes along y 12: {boxes:?}");
    let whole = boxes.iter().any(|found| {
        (found.start - edit.start).abs() <= 1.0 && (found.end - edit.end).abs() <= 1.0
    });
    assert!(whole, "Edit is not whole between Retry and File");
}

// F1 with the Edit menu shown at start (--show edit-menu) at 1000 x 560: the menu's box in
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
        near(inside, rgb8(style::palette().panel), TOLERANCE),
        "the menu's box"
    );
    assert!(
        near(outside, rgb8(style::palette().bg), TOLERANCE),
        "the menu ends under Edit's right edge"
    );
}
