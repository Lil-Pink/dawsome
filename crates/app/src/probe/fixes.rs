//! Screenshot probes of the views' scroll bars along time, at rest and
//! scrolled to their end, and the menus over the panels at their panels' edges. Positions come
//! from the views' own layout functions, the scroll model ([`crate::ui::scroll`]) and the menus'
//! placement ([`crate::ui::menu::place`]), with the hand-worked figures beside them; colors
//! from [`crate::ui::style`].

use dawsome_engine::time::Ticks;
use iced::Point;

use super::tests::{read_ui_shot, rgb8};
use super::{near, roll_grid_in_window, Screenshot, TOLERANCE};
use crate::ui::piano_layout::{self, View, KEY_COLUMN_WIDTH};
use crate::ui::scroll::{self, Span};
use crate::ui::style;

/// p3_roll's view in a 1000 x 560 window with the piano roll alone: top key 69, 16 px a step,
/// scrolled to `scroll`.
fn p3_view(scroll: Ticks) -> View {
    View {
        top: 69,
        scroll,
        zoom: piano_layout::DEFAULT_ZOOM,
        step: Ticks(24),
        rows: crate::ui::piano_layout::Rows::ALL,
    }
}

/// The piano roll's bar along time in a shot of p3_roll alone at 1000 x 560: the thumb where
/// the scroll model puts it for `span` (window x), the trough beside it, sampled down the bar's
/// middle, under the grid of 388 px.
fn assert_roll_bar(shot: &Screenshot, span: Span, thumb_x: std::ops::Range<f32>) {
    let grid = piano_layout::grid_size_alone(iced::Size::new(1000.0, 560.0));
    let y = roll_grid_in_window(Point::new(0.0, grid.height + scroll::BAR_HEIGHT / 2.0)).y;
    // Under the 64 px toolbar the grid starts at 92 and is 388 px tall (at 100 and 380 under
    // the 72 px one), so the bar's middle stays at 485.
    assert_eq!(y, 485.0, "the bar's middle, under the grid at 92 + 388");
    let trough = 1000.0 - KEY_COLUMN_WIDTH;
    let (start, size) = span.thumb(trough);
    let (left, right) = (KEY_COLUMN_WIDTH + start, KEY_COLUMN_WIDTH + start + size);
    println!("thumb from x {left} to {right}");
    assert!(
        (left - thumb_x.start).abs() < 1.0 && (right - thumb_x.end).abs() < 1.0,
        "the thumb is not where the design puts it: {left}..{right}"
    );
    let thumb = shot.count_matching(left + 4.0..right - 4.0, y - 1.0..y + 1.0, |pixel| {
        near(pixel, rgb8(scroll::THUMB), TOLERANCE)
    });
    let width = ((right - left - 8.0) * 2.0 * shot.scale * shot.scale) as usize;
    let beside = if left - KEY_COLUMN_WIDTH > 12.0 {
        Point::new(KEY_COLUMN_WIDTH + 6.0, y)
    } else {
        Point::new(right + 6.0, y)
    };
    let well = shot.pixel(beside);
    println!("thumb pixels {thumb} of {width}; the trough at {beside:?}: {well:02X?}");
    assert!(thumb * 10 >= width * 9, "the thumb is drawn");
    assert!(
        near(well, rgb8(style::RACK_WELL), TOLERANCE),
        "the trough beside it"
    );
}

// P2: p3_roll with the piano roll alone at 1000 x 560: the bar under the grid holds a thumb of
// the view's share, 1428 of the 1920 ticks the grid reaches (952 px at 16 px a step; one bar
// and the four past it), 708 px from the key column's edge, x 48..756.
#[test]
#[ignore = "needs a screenshot of p3_roll with --panels piano-roll: scripts/ui-check.sh takes one and runs this"]
fn p2_the_rolls_bar_shows_the_view_at_its_start() {
    let shot = read_ui_shot();
    let span = piano_layout::time_span(&p3_view(Ticks(0)), Ticks(384), 1000.0);
    assert_eq!((span.content, span.view), (1920.0, 1428.0));
    assert_roll_bar(&shot, span, 48.0..756.05);
}

// P2: p3_roll with --scroll end: the grid shows ticks 492 to 1920, all past the pattern's end
// (its rows dimmed), bar 3's line (tick 768) at x 48 + 276·16/24 = 232, and the thumb at the
// bar's end, x 291.95..1000.
#[test]
#[ignore = "needs a screenshot of p3_roll with --panels piano-roll --scroll end: scripts/ui-check.sh takes one and runs this"]
fn p2_the_roll_scrolled_to_its_end() {
    let shot = read_ui_shot();
    let view = p3_view(Ticks(492));
    let span = piano_layout::time_span(&view, Ticks(384), 1000.0);
    assert_eq!(span.offset, span.last_offset(), "at the end");
    assert_roll_bar(&shot, span, 291.95..1000.0);
    let row = view.row_top(60) + 6.0;
    let line = roll_grid_in_window(Point::new(view.x_of(Ticks(768)) + 1.0, row));
    // Pin moved 16 px down under the 72 px toolbar (was y 198), then 8 px up under the 64 px
    // one (was y 214).
    assert_eq!(line, Point::new(233.0, 206.0));
    let past = roll_grid_in_window(Point::new(300.0, row));
    let (line, past) = (shot.pixel(line), shot.pixel(past));
    println!("bar 3's line {line:02X?}, a row past the end {past:02X?}");
    assert!(near(line, rgb8(style::BAR_LINE), TOLERANCE), "bar 3's line");
    assert!(
        near(past, rgb8(style::dimmed(style::ROLL_ROW_WHITE)), TOLERANCE),
        "the rows past the end, dimmed"
    );
}

// P4: p3_roll with the rack alone and the starter kit's first sound held over channel 2's
// header (--drag-over channel:2): an accent outline runs round row 2's header (its light, name
// and knobs, window x 0..218 from y 134, 28 px tall under the toolbar's 64 and the rack header's
// 24; pins moved from y 126 under the 56 px toolbar and 142 under the 72 px one): along its top
// edge left
// of the name and down its left edge. Row 1's header, the
// selected channel's, has none there (its name's selection outline starts at x 28).
#[test]
#[ignore = "needs a screenshot of p3_roll with --panels rack --drag-over channel:2: scripts/ui-check.sh takes one and runs this"]
fn p4_a_held_sound_outlines_the_header_it_would_replace() {
    use crate::ui::layout::{channel_header_rect, RACK_HEADER_HEIGHT, TOOLBAR_HEIGHT};
    let shot = read_ui_shot();
    let in_window = |row: usize| {
        let rect = channel_header_rect(row);
        iced::Rectangle {
            y: rect.y + TOOLBAR_HEIGHT + RACK_HEADER_HEIGHT,
            ..rect
        }
    };
    let held = in_window(1);
    assert_eq!(
        (held.x, held.y, held.width, held.height),
        (0.0, 134.0, 218.0, 28.0)
    );
    let accent = |pixel| near(pixel, rgb8(style::ACCENT), TOLERANCE);
    let top = |rect: iced::Rectangle| shot.count_matching(2.0..26.0, rect.y..rect.y + 2.0, accent);
    let left = shot.count_matching(0.0..2.0, held.y + 4.0..held.y + held.height - 4.0, accent);
    let (held_top, other_top) = (top(held), top(in_window(0)));
    println!(
        "accent pixels: row 2's top edge {held_top}, its left edge {left}, row 1's {other_top}"
    );
    let edge = (24.0 * 2.0 * shot.scale * shot.scale) as usize;
    assert!(
        held_top * 10 >= edge * 9,
        "row 2's header is outlined along its top"
    );
    assert!(
        left * 10 >= (20.0 * 2.0 * shot.scale * shot.scale) as usize * 9,
        "and its left"
    );
    assert_eq!(other_top, 0, "row 1's header is not");
}

// P5: p5_gain (five samplers) with the rack and the playlist at 1000 x 560 and the last row's
// menu open (--channel-menu channel:5, as a right press on "postfader"'s name). The rack takes
// half the 496 px under the toolbar, so the playlist starts at y 312; the menu opens at the
// name's middle, (83, 262), on the window's top layer: 168 px wide, its eight rows 26 px tall
// from y 266 (inside its 4 px ground), the last six over the playlist. Each row shows whole:
// the menu's ground at its right end, low in the row, and its label's bright text. (Pins
// moved under the 72 px toolbar: the playlist from 308, the menu at 254; then 316 and 270, and
// 312 and 262 under the 64 px one. "One note at a time" made a sampler's menu six rows, five
// before; "Edit sample…" seven and "Mixer insert…" eight.)
#[test]
#[ignore = "needs a screenshot of p5_gain with --panels rack,playlist --channel-menu channel:5: scripts/ui-check.sh takes one and runs this"]
fn p5_the_last_rack_rows_menu_shows_whole_over_the_playlist() {
    use crate::ui::layout::{
        name_button_rect, rack_canvas_size, RACK_HEADER_HEIGHT, TOOLBAR_HEIGHT,
    };
    use crate::ui::menu::{place, PADDING, ROW_HEIGHT};
    use crate::ui::panels::{stack_heights, Panel};
    let shot = read_ui_shot();
    let window = iced::Size::new(1000.0, 560.0);
    let rack = RACK_HEADER_HEIGHT + rack_canvas_size(5, 16).height;
    let heights = stack_heights(
        &[Panel::Rack, Panel::Playlist],
        rack,
        &crate::ui::panels::Kept::default(),
        560.0 - TOOLBAR_HEIGHT,
    );
    let playlist_top = TOOLBAR_HEIGHT + heights[0].floor();
    assert_eq!(playlist_top, 312.0);
    let name = name_button_rect(4).center();
    let anchor = Point::new(name.x, name.y + TOOLBAR_HEIGHT + RACK_HEADER_HEIGHT);
    let size = iced::Size::new(160.0 + 2.0 * PADDING, 8.0 * ROW_HEIGHT + 2.0 * PADDING);
    let at = place(anchor, size, window);
    assert_eq!(
        at,
        Point::new(83.0, 262.0),
        "down and right of the press: it fits"
    );
    let ground = rgb8(style::PANEL);
    let mut over_playlist = 0;
    for row in 0..8 {
        let top = at.y + PADDING + row as f32 * ROW_HEIGHT;
        let edge = shot.pixel(Point::new(at.x + size.width - 12.0, top + ROW_HEIGHT - 4.0));
        let label = shot.count_matching(at.x + 10.0..at.x + 110.0, top + 5.0..top + 21.0, |p| {
            p.iter().all(|&channel| channel > 0x90)
        });
        println!("row {row} from y {top}: ground {edge:02X?}, label pixels {label}");
        assert!(near(edge, ground, TOLERANCE), "row {row}'s ground");
        assert!(label > 8, "row {row}'s label");
        if top >= playlist_top {
            over_playlist += 1;
        }
    }
    assert_eq!(over_playlist, 6, "rows over the playlist from their top on");
    let foot = at.y + size.height - 2.0;
    let under = shot.pixel(Point::new(at.x + 20.0, foot));
    assert!(
        near(under, ground, TOLERANCE),
        "the menu's foot, over the playlist: {under:02X?}"
    );
}

// P6: p15_long_song (clips at bars 1 and 19: the song ends with bar 19, tick 7296) with the
// playlist alone at 1000 x 560 and --scroll end. At 64 px a bar the 880 px timeline shows 5280
// ticks, and the bar reaches 4 bars past the song, 8832: the view starts at 3552, so the
// second clip lies at x 120 + 3360/6 = 680 to 744 on track 1's lane and the 4 bars after it
// are empty lanes. The scroll bar at the panel's foot (y 550..560, its middle row 555) runs
// under the timeline from x 120 to the window's edge, the well under the track headers left
// of it, its thumb at its end, the view's share of the 880 px trough (5280 of 8832).
// (The bar was in the header, its middle 12 px under the toolbar, to x 988, and
// the lanes 47 px in a 436 px track area; now the lanes fit the 8 tracks and the "+ Track" row
// into 434 px, 47 px from y 116. They were 32 px from y 108 under the 56 px
// toolbar, and 46 px from y 124 in 426 px under the 72 px one.)
#[test]
#[ignore = "needs a screenshot of p15_long_song with --panels playlist --scroll end: scripts/ui-check.sh takes one and runs this"]
fn p6_the_playlist_scrolled_past_the_song_end() {
    use crate::ui::layout::TOOLBAR_HEIGHT;
    use crate::ui::playlist_layout::{
        self, Grid, DEFAULT_ZOOM, HEADER_HEIGHT, RULER_HEIGHT, TIME_BAR_HEIGHT, TRACK_HEADER_WIDTH,
    };
    let shot = read_ui_shot();
    let area = playlist_layout::tracks_size_alone(iced::Size::new(1000.0, 560.0), TOOLBAR_HEIGHT);
    let grid = Grid {
        scroll: Ticks(3552),
        zoom: DEFAULT_ZOOM,
        bar: Ticks(384),
        top: 0,
        lane: playlist_layout::fitted_lane(8, area.height),
    };
    assert_eq!(grid.lane, 47.0);
    let lane_middle = TOOLBAR_HEIGHT + HEADER_HEIGHT + RULER_HEIGHT + grid.lane / 2.0;
    let span = playlist_layout::time_span(&grid, Ticks(7296), 1000.0);
    assert_eq!(
        (span.content, span.view, span.last_offset()),
        (8832.0, 5280.0, 3552.0)
    );
    assert_eq!(
        (grid.x_of(Ticks(6912)), grid.x_of(Ticks(7296))),
        (680.0, 744.0)
    );
    let body = shot.pixel(Point::new(700.0, lane_middle));
    let past = shot.pixel(Point::new(900.0, lane_middle));
    println!("the clip {body:02X?}, a lane past the song {past:02X?}");
    assert!(
        near(body, rgb8(style::CLIP_BODY), TOLERANCE),
        "bar 19's clip"
    );
    assert!(
        near(past, rgb8(style::LANE), TOLERANCE),
        "empty lanes past the song"
    );
    let y = 560.0 - TIME_BAR_HEIGHT / 2.0;
    assert_eq!(y, 555.0, "the bar's middle row");
    let bar = shot.runs_along(y, |pixel| {
        near(pixel, rgb8(style::RACK_WELL), TOLERANCE)
            || near(pixel, rgb8(scroll::THUMB), TOLERANCE)
    });
    let bar = bar
        .last()
        .cloned()
        .expect("the scroll bar at the panel's foot");
    let thumb = shot.runs_along(y, |pixel| near(pixel, rgb8(scroll::THUMB), TOLERANCE));
    let thumb = thumb.last().cloned().expect("its thumb");
    let share = thumb.end - thumb.start;
    println!("bar {bar:?}, thumb {thumb:?} ({share} px)");
    assert!(
        (bar.end - 1000.0).abs() <= 1.0,
        "the bar runs to the window's edge"
    );
    assert!(
        bar.start <= TRACK_HEADER_WIDTH + 1.0,
        "the trough from the timeline's start, the well left of it"
    );
    assert!(
        (thumb.end - bar.end).abs() <= 2.0,
        "the thumb at the bar's end"
    );
    let length = 1000.0 - TRACK_HEADER_WIDTH;
    let expected = span.thumb(length).1;
    assert!(
        (share - expected).abs() <= 3.0,
        "the view's share: {expected}"
    );
}

// P6: p15_long_song's two-bar pattern (32 steps, reaching 1012 px past x 222) in the rack alone
// at 1000 x 560 with --scroll end: 778 px of steps show, so they are scrolled 234 px. The name
// and knobs stay where they are; step 8 (lit, its left edge 474) shows at x 240..268 and step 31
// (unlit, 1194) at 960..988; step 7, scrolled to 204..232, is covered left of x 222 (row 1's
// middle at y 110 under the 64 px toolbar; pins moved from y 102, and 118 under the 72 px
// one). The bar
// under the rows (y 550..560, x 222..1000) has its thumb at its end: 778/1012 of 778 px, from
// x 401.9.
#[test]
#[ignore = "needs a screenshot of p15_long_song with --panels rack --scroll end: scripts/ui-check.sh takes one and runs this"]
fn p6_the_racks_steps_scrolled_to_their_end() {
    use crate::ui::layout::{step_left, STEPS_X};
    let shot = read_ui_shot();
    let span = Span {
        content: 1012.0,
        offset: 234.0,
        view: 778.0,
    };
    assert_eq!(span.last_offset(), 234.0);
    assert_eq!((step_left(8), step_left(31)), (474.0, 1194.0));
    let (start, size) = span.thumb(1000.0 - STEPS_X);
    assert!((STEPS_X + start - 401.9).abs() < 0.1, "{start}");
    let at = |x: f32, y: f32| shot.pixel(Point::new(x, y));
    let checks: [(&str, Point, [u8; 3]); 7] = [
        (
            "the name, put",
            Point::new(120.0, 110.0),
            rgb8(style::NAME_BUTTON),
        ),
        (
            "step 8, lit",
            Point::new(254.0, 110.0),
            rgb8(style::channel_color(0)),
        ),
        (
            "step 31",
            Point::new(974.0, 110.0),
            rgb8(style::unlit_step(31)),
        ),
        (
            "step 7 covered",
            Point::new(216.0, 110.0),
            rgb8(style::RACK_WELL),
        ),
        (
            "the trough",
            Point::new(300.0, 555.0),
            rgb8(style::RACK_WELL),
        ),
        (
            "the thumb",
            Point::new(STEPS_X + start + 6.0, 555.0),
            rgb8(scroll::THUMB),
        ),
        (
            "the thumb's end",
            Point::new(STEPS_X + start + size - 6.0, 555.0),
            rgb8(scroll::THUMB),
        ),
    ];
    let mut wrong = Vec::new();
    for (name, point, expected) in checks {
        let pixel = at(point.x, point.y);
        println!("{name} at {point:?}: {pixel:02X?}, expected {expected:02X?}");
        if !near(pixel, expected, TOLERANCE) {
            wrong.push(name);
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// The `rows` rows of a menu `size` big drawn at window point `at` in `shot`, each whole: the
/// menu's ground at its right end, low in the row, and its label's bright text (no other text
/// in the window is that bright).
fn assert_menu_rows(shot: &Screenshot, at: Point, size: iced::Size, rows: usize) {
    use crate::ui::menu::{PADDING, ROW_HEIGHT};
    let ground = rgb8(style::PANEL);
    for row in 0..rows {
        let top = at.y + PADDING + row as f32 * ROW_HEIGHT;
        let edge = shot.pixel(Point::new(at.x + size.width - 12.0, top + ROW_HEIGHT - 4.0));
        let label = shot.count_matching(at.x + 10.0..at.x + 80.0, top + 5.0..top + 21.0, |p| {
            p.iter().all(|&channel| channel > 0x90)
        });
        println!("row {row} from y {top}: ground {edge:02X?}, label pixels {label}");
        assert!(near(edge, ground, TOLERANCE), "row {row}'s ground");
        assert!(label > 8, "row {row}'s label");
    }
}

// A rack menu over the hint bar: p5_gain with the rack alone at 1000 x 400, the hint
// bar on (--hints on) and the last row's menu open (--channel-menu channel:5). The menu opens
// at the name's middle, (83, 262), 168 x 216 (eight rows): down from there it would end at 478,
// over the hint bar from 378, and flipped up it would start at 46, above the rack's rows (from
// y 88, under the toolbar and the rack's header, which covered its first rows there), so it
// sits as low as it fits, y 162..378, over the press, every row whole. The hint bar holds no
// menu label, only its own dim words on its ground. (Pins moved 16 px down under the 72 px
// toolbar: 254 and 116; then 8 px up under the 64 px one, from 270 and 106. The sixth row, "One
// note at a time": 168 x 138 from y 132 before; six rows flipped up to 98; "Edit sample…" and
// "Mixer insert…" made eight.)
#[test]
#[ignore = "needs a screenshot of p5_gain with --panels rack --window-size 1000x400 --hints on --channel-menu channel:5: scripts/ui-check.sh takes one and runs this"]
fn p5_a_menu_stays_above_the_hint_bar() {
    use crate::ui::layout::{
        name_button_rect, HINT_BAR_HEIGHT, RACK_HEADER_HEIGHT, TOOLBAR_HEIGHT,
    };
    use crate::ui::menu::{place_below, PADDING, ROW_HEIGHT};
    let shot = read_ui_shot();
    let name = name_button_rect(4).center();
    let anchor = Point::new(name.x, name.y + TOOLBAR_HEIGHT + RACK_HEADER_HEIGHT);
    let size = iced::Size::new(160.0 + 2.0 * PADDING, 8.0 * ROW_HEIGHT + 2.0 * PADDING);
    let bar = 400.0 - HINT_BAR_HEIGHT;
    let rows_top = TOOLBAR_HEIGHT + RACK_HEADER_HEIGHT;
    let at = place_below(anchor, size, iced::Size::new(1000.0, bar), rows_top);
    assert_eq!(
        (anchor, rows_top, at),
        (Point::new(83.0, 262.0), 88.0, Point::new(83.0, 162.0)),
        "as low as it fits under the rack's header"
    );
    assert_menu_rows(&shot, at, size, 8);
    let labels = shot.count_matching(at.x..at.x + size.width, bar + 1.0..399.0, |p| {
        p.iter().all(|&channel| channel > 0x90)
    });
    let ground = shot.pixel(Point::new(at.x + size.width - 12.0, bar + 3.0));
    println!("the hint bar under the menu's old place: {labels} label pixels, {ground:02X?}");
    assert_eq!(labels, 0, "no menu label over the hint bar");
    assert!(
        near(ground, rgb8(style::PANEL), TOLERANCE),
        "the hint bar's ground"
    );
}

// A menu at the tracks' foot: p8_ui with the playlist alone at 1000 x 260, the
// hint bar on and the curve menu open on automation 1's segment handle (--curve-menu
// automation:1). The handle sits on track 2's clip, its segment's middle: tick 192 at 64 px a
// bar is x 152, value 0.5 the middle of the clip's body. The menu, 128 x 112, would pass the
// hint bar's top, 238 (the tracks end at 228 over the playlist's scroll bar at the panel's foot;
// the tracks canvas used to cut a menu at its foot); drawn on the window's top layer it flips
// up and ends at the handle, every row whole.
#[test]
#[ignore = "needs a screenshot of p8_ui with --panels playlist --window-size 1000x260 --hints on --curve-menu automation:1: scripts/ui-check.sh takes one and runs this"]
fn p5_the_curve_menu_shows_whole_at_the_tracks_foot() {
    use dawsome_engine::project::{AutomationId, ClipId, Project};
    use iced::{Rectangle, Size};

    use super::tracks_in_window;
    use crate::ui::layout::HINT_BAR_HEIGHT;
    use crate::ui::menu::{place, PADDING, ROW_HEIGHT};
    use crate::ui::playlist::{automation_handles, CURVE_MENU_WIDTH};
    use crate::ui::playlist_layout::{self, Grid, DEFAULT_ZOOM, LANE_HEIGHT, TITLE_HEIGHT};
    let shot = read_ui_shot();
    let project = Project::load_json(include_str!("../../../../tests/fixtures/p8_ui.json"))
        .expect("p8_ui loads");
    let grid = Grid {
        scroll: Ticks(0),
        zoom: DEFAULT_ZOOM,
        bar: Ticks(384),
        top: 0,
        lane: LANE_HEIGHT,
    };
    let clip = &project.playlist.clips[&ClipId(2)];
    let automation = &project.automation.list[&AutomationId(1)];
    let top = grid.row_top(1);
    let rect = playlist_layout::clip_rect(&grid, clip, top);
    let body = Rectangle::new(
        Point::new(rect.x, top + TITLE_HEIGHT),
        Size::new(rect.width, grid.lane - TITLE_HEIGHT),
    );
    let handle = automation_handles(&grid, clip, automation, body)
        .first()
        .map(|handle| handle.at)
        .expect("a handle");
    assert_eq!(handle.x, 152.0, "tick 192");
    let anchor = tracks_in_window(handle);
    let size = Size::new(
        CURVE_MENU_WIDTH + 2.0 * PADDING,
        4.0 * ROW_HEIGHT + 2.0 * PADDING,
    );
    let foot = 260.0 - HINT_BAR_HEIGHT;
    let at = place(anchor, size, Size::new(1000.0, foot));
    println!("the handle at {anchor:?}; the menu at {at:?}, {size:?}");
    assert!(
        anchor.y + size.height > foot,
        "down from the handle it would pass the tracks' foot"
    );
    assert_eq!(
        at,
        Point::new(anchor.x, anchor.y - size.height),
        "flipped up"
    );
    assert_menu_rows(&shot, at, size, 4);
}

// The slot menu at the mixer's foot: p5_gain with the mixer alone at
// 1000 x 400, the hint bar on and insert 12 (the last strip in view) selected, its empty slot
// 9's menu open (--slot-menu insert:12,slot:9). The add menu, six rows, 188 x 164, opens under
// the slot at (800 + 28, 64 + 6 + 52 + 22·10) = (828, 342): down from there it would end at
// 506, past the hint bar's top at 378, where the mixer used to cut its rows (two and a half of
// six showed). Drawn on the window's top layer it flips up and moves left to end at the
// window's edge, from (812, 178), every row whole. (Pins moved: the 72 px
// toolbar and the mixer's 6 px handle put the slot 22 px lower than at 328 and 164, at 350 and
// 186; the 64 px toolbar 8 px higher.)
#[test]
#[ignore = "needs a screenshot of p5_gain with --panels mixer --window-size 1000x400 --hints on --slot-menu insert:12,slot:9: scripts/ui-check.sh takes one and runs this"]
fn p5_a_slot_menu_shows_whole_at_the_window_foot() {
    use crate::ui::layout::HINT_BAR_HEIGHT;
    use crate::ui::menu::{place, PADDING, ROW_HEIGHT};
    use crate::ui::mixer_layout::{in_window, menu_anchor, MENU_WIDTH};
    let shot = read_ui_shot();
    let anchor = in_window(menu_anchor(9, 1000.0));
    let size = iced::Size::new(MENU_WIDTH + 2.0 * PADDING, 6.0 * ROW_HEIGHT + 2.0 * PADDING);
    let at = place(
        anchor,
        size,
        iced::Size::new(1000.0, 400.0 - HINT_BAR_HEIGHT),
    );
    println!("the slot opens its menu at {anchor:?}; the menu at {at:?}, {size:?}");
    assert_eq!(
        (anchor, at),
        (Point::new(828.0, 342.0), Point::new(812.0, 178.0)),
        "flipped up and moved left"
    );
    assert_menu_rows(&shot, at, size, 6);
}

// The Library row menu at the panel's foot: p10_library with the
// Library alone at 1000 x 784, the hint bar on, Port A and Port B listed (--midi-ports
// "Port A,Port B", --cache p7_catalogue.json, --browser-root tests/fixtures/browser) and Port
// A's row's menu open (--library-menu "device:Port A"; "kick" plays Port A). Port A is row 28
// of 31: in a panel 784 − 64 − 22 = 698 px tall, (698 − 52) / 22 = 29.4, so 29 rows show whole
// and it is the last, its foot at window y 64 + 52 + 22·29 = 754, 8 px over the hint bar's top,
// where the Library used to cut the whole menu off. Its Unlink menu, 104 x 34, opened at x 40
// under the row, would pass the hint bar, so it flips up on the window's top layer to (40, 720),
// its row whole. (The window 16 px taller than its 724 for the 72 px toolbar, so the panel kept
// its 646 px; the external input: 44 px taller again for its two rows, "Audio input" and
// "External input", which come before the ports, so Port A stays the last whole row; under the
// 64 px toolbar the panel is 8 px taller, not a row more: the row's foot was at 762, the menu at
// 728.)
#[test]
#[ignore = "needs a screenshot of p10_library with --panels browser --window-size 1000x784 --hints on --library-menu \"device:Port A\": scripts/ui-check.sh takes one and runs this"]
fn p5_the_library_row_menu_shows_whole_at_the_panels_foot() {
    use dawsome_host_clap::catalogue::PluginKind;

    use crate::settings::MidiInputs;
    use crate::ui::browser::{self, menu_anchor, MENU_WIDTH};
    use crate::ui::layout::{HINT_BAR_HEIGHT, TOOLBAR_HEIGHT};
    use crate::ui::library::{self, Contents, Item, Library};
    use crate::ui::menu::{place, PADDING, ROW_HEIGHT};
    let shot = read_ui_shot();
    let catalogue = dawsome_host_clap::Catalogue::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/p7_catalogue.json"),
    )
    .expect("the catalogue loads");
    let listed = ["Port A".to_owned(), "Port B".to_owned()];
    let links = [
        ("Port A".to_owned(), "kick".to_owned()),
        ("Port C".to_owned(), "keys".to_owned()),
    ];
    let items = Library::default().items(&Contents {
        kit: &crate::starter_kit::names(),
        plugins: catalogue.offered(PluginKind::Instrument),
        tree: 3,
        folders: 1,
        ports: library::ports(&listed, &MidiInputs::default(), &links),
        ..Contents::default()
    });
    let row = items
        .iter()
        .position(|item| matches!(item, Item::Port(port) if port.name == "Port A"))
        .expect("listed");
    assert_eq!((row, items.len()), (28, 31));
    let panel = 784.0 - TOOLBAR_HEIGHT - HINT_BAR_HEIGHT;
    assert_eq!(browser::visible_rows(panel), row + 1, "the last whole row");
    let under = menu_anchor(row, 0).expect("in view");
    let anchor = Point::new(under.x, under.y + TOOLBAR_HEIGHT);
    let size = iced::Size::new(MENU_WIDTH + 2.0 * PADDING, ROW_HEIGHT + 2.0 * PADDING);
    let at = place(
        anchor,
        size,
        iced::Size::new(1000.0, 784.0 - HINT_BAR_HEIGHT),
    );
    println!("the row opens its menu at {anchor:?}; the menu at {at:?}, {size:?}");
    assert_eq!(
        (anchor, at),
        (Point::new(40.0, 754.0), Point::new(40.0, 720.0)),
        "flipped up, ending at the row's foot"
    );
    assert_menu_rows(&shot, at, size, 1);
}

/// The first and the last pixel row, in logical pixels (the last row's end), of the toolbar
/// above its foot line where any pixel of the logical columns `x` is off the toolbar's ground.
fn drawn_rows(shot: &Screenshot, x: std::ops::Range<f32>) -> Option<(f32, f32)> {
    use crate::ui::layout::TOOLBAR_HEIGHT;
    let ground = rgb8(style::PANEL);
    let columns = (x.start * shot.scale) as usize..(x.end * shot.scale).ceil() as usize;
    let rows = ((TOOLBAR_HEIGHT - 1.0) * shot.scale) as usize;
    let width = shot.width as usize;
    let drawn: Vec<usize> = (0..rows)
        .filter(|&row| {
            columns.clone().any(|column| {
                let at = (row * width + column) * 4;
                let pixel = [shot.rgba[at], shot.rgba[at + 1], shot.rgba[at + 2]];
                !near(pixel, ground, 6)
            })
        })
        .collect();
    let (first, last) = (*drawn.first()?, *drawn.last()?);
    Some((first as f32 / shot.scale, (last + 1) as f32 / shot.scale))
}

// Every group of the toolbar fills its cell. 2 to 4 px inside each
// group's left edge (Play, the tempo, the pattern's previous arrow, Pat over Song, the position,
// the keyboard switch, the panel buttons' first column, Edit) and across each line between two
// groups, what is drawn starts and ends where the toolbar's groups do (`toolbar::group_rows`):
// y 9 to 55, the groups' 46 px centered in the 64 px bar (10 to 62 in the 72 px one), the lines
// as tall (they were 44 px beside 52 px buttons and 60 px stacks). On F1 at
// 1000 px and on the first start's panels at 1920 px, the groups where the spread for the
// shot's width puts them. Toolbar scale: at 1920 px, and on F1's toolbar at 2560 px drawn
// offscreen, the groups have grown (scale 1.6) and the spread's places are the grown ones; the
// heights stay.
#[test]
#[ignore = "needs a screenshot of the toolbar: scripts/ui-check.sh takes F1's, a 1920 px one and a 2560 px one drawn offscreen and runs this"]
fn t1_every_toolbar_group_fills_its_cell() {
    let shot = read_ui_shot();
    let width = shot.width as f32 / shot.scale;
    let spread = crate::ui::toolbar::shot_spread(width, false);
    let pixel = 1.0 / shot.scale;
    let groups = crate::ui::toolbar::group_rows();
    let mut checked = 0;
    let mut check = |what: String, x: std::ops::Range<f32>| {
        let rows = drawn_rows(&shot, x.clone());
        println!("{what} at x {x:?}: drawn rows {rows:?}");
        let (top, bottom) = rows.unwrap_or_else(|| panic!("{what}: nothing drawn"));
        assert!(
            (top - groups.start).abs() <= pixel,
            "{what} starts at y {top}, not {}",
            groups.start
        );
        assert!(
            (bottom - groups.end).abs() <= pixel,
            "{what} ends at y {bottom}, not {}",
            groups.end
        );
        checked += 1;
    };
    for &(group, _) in spread.groups() {
        let left = spread.left(group).expect("shown");
        check(format!("{group:?}"), left + 2.0..left + 4.0);
    }
    for (index, line) in spread.divider_xs().into_iter().enumerate() {
        check(
            format!("the line after group {index}"),
            line - 0.5..line + 1.5,
        );
    }
    assert!(checked >= 15, "every group and line checked: {checked}");
}

// Toolbar scale: the panel buttons' icons grow with the bar. Where the window is wide enough for
// the most scale (1920 px, the widest a test window opens on this desk's side screens, and the
// toolbar at 2560 px drawn offscreen) each of the six icons spans 16 px across (11 × 1.6 = 17.6,
// held to what leaves 3 px over and under it in its 22 px row) in its 28 px button; at 1000 px,
// where the bar has nothing to spare, 11 px in 19 (`toolbar::panel_icon_sizes`; were 19 in 31
// and 12 in 20 before the bar was made 0.9 of its size). What
// is measured: from the first to the last column inside the button (2 px in from its edges)
// holding a pixel off its face, within a pixel (the grid starts between two pixels, so an
// icon's edge columns are partly covered).
#[test]
#[ignore = "needs a screenshot of the toolbar: scripts/ui-check.sh takes F1's, a 1920 px one and a 2560 px one drawn offscreen and runs this"]
fn t3_the_panel_icons_grow_with_the_bar() {
    let shot = read_ui_shot();
    let width = shot.width as f32 / shot.scale;
    let spread = crate::ui::toolbar::shot_spread(width, false);
    let icon = spread.sizes().panel_icon;
    let [one, most] = crate::ui::toolbar::panel_icon_sizes();
    let worked = if width >= 1920.0 {
        Some(most)
    } else if width <= 1200.0 {
        Some(one)
    } else {
        None
    };
    let face = rgb8(style::BUTTON);
    let rects = crate::ui::toolbar::panel_button_rects(&spread);
    assert_eq!(rects.len(), 6, "the panel buttons show");
    for (panel, rect) in rects {
        if let Some((side, button)) = worked {
            assert_eq!(
                (icon, rect.width),
                (side, button),
                "{panel:?} at {width} px"
            );
        }
        let physical = |logical: f32| (logical * shot.scale).round() as usize;
        let rows = physical(rect.y + 2.0)..physical(rect.y + rect.height - 2.0);
        let columns: Vec<usize> = (physical(rect.x + 2.0)..physical(rect.x + rect.width - 2.0))
            .filter(|&column| {
                rows.clone().any(|row| {
                    let at = (row * shot.width as usize + column) * 4;
                    let pixel = [shot.rgba[at], shot.rgba[at + 1], shot.rgba[at + 2]];
                    !near(pixel, face, 8)
                })
            })
            .collect();
        let (first, last) = (columns[0], columns[columns.len() - 1]);
        let drawn = (last + 1 - first) as f32 / shot.scale;
        println!("{panel:?} at {rect:?}: its icon spans {drawn} px (the spread's {icon})");
        assert!(
            (drawn - icon).abs() <= 1.0,
            "{panel:?}: {drawn} px, not {icon}"
        );
    }
}

// The Settings window at its least size, 420 x 240 (a settings file
// keeping that place), with two ports given: its header's tabs show (the General tab lit on
// the button's face, the Keys tab's ground the header's), the first group shows ("All ports"
// unticked with its words beside it, both ports' boxes ticked), the foot line fills its last
// 22 px in the panels' ground, and
// the page, longer than the room under the header, shows a scroller down the window's right
// edge (the toolkit's, off the page's ground) where the page draws nothing.
#[test]
#[ignore = "needs a screenshot of the Settings window at 420 x 240: scripts/ui-check.sh takes one and runs this"]
fn t2_the_least_settings_window_keeps_its_tabs_and_first_group() {
    use crate::ui::layout::HINT_BAR_HEIGHT;
    use crate::ui::settings_layout::{tab, Layout, SettingsPage, HEADER_HEIGHT};
    let shot = read_ui_shot();
    let (width, height) = (
        shot.width as f32 / shot.scale,
        shot.height as f32 / shot.scale,
    );
    assert_eq!((width, height), (420.0, 240.0), "the least size kept");
    let general = tab(SettingsPage::General);
    let keys = tab(SettingsPage::Keys);
    let lit = shot.pixel(Point::new(general.x + 3.0, general.y + 3.0));
    let unlit = shot.pixel(Point::new(keys.x + 3.0, keys.y + 3.0));
    println!("tabs: General {lit:02X?}, Keys {unlit:02X?}");
    assert!(near(lit, rgb8(style::BUTTON), TOLERANCE), "the General tab");
    assert!(near(unlit, rgb8(style::HEADER), TOLERANCE), "the Keys tab");
    let layout = Layout { ports: 2, notes: 0 };
    for (row, color) in [
        (0, style::KNOB_RING),
        (1, style::ACCENT),
        (2, style::ACCENT),
    ] {
        let at = layout.checkbox(row).center();
        let pixel = shot.pixel(at);
        println!("row {row}'s box at {at:?}: {pixel:02X?}");
        assert!(near(pixel, rgb8(color), TOLERANCE), "row {row}");
    }
    // "All ports" beside its box, in its row: a clipped label is drawn in a frame of its own,
    // which once left it a row lower than its box.
    let label = layout.label(0);
    let words = shot.count_matching(
        label.x..label.x + 60.0,
        label.y..label.y + label.height,
        |pixel| pixel.iter().all(|&value| value > 0x90),
    );
    println!("bright text in the first row's label: {words} px");
    assert!(words > 0, "the first row's words beside its box");
    let foot = shot.pixel(Point::new(width - 20.0, height - HINT_BAR_HEIGHT / 2.0));
    assert!(
        near(foot, rgb8(style::PANEL), TOLERANCE),
        "the foot line: {foot:02X?}"
    );
    let ground = rgb8(style::RACK_WELL);
    let edge = shot.count_matching(
        width - 4.0..width - 2.0,
        HEADER_HEIGHT + 2.0..height - HINT_BAR_HEIGHT - 2.0,
        |pixel| !near(pixel, ground, 6),
    );
    let inside = shot.count_matching(
        width - 40.0..width - 38.0,
        HEADER_HEIGHT + 2.0..height - HINT_BAR_HEIGHT - 2.0,
        |pixel| !near(pixel, ground, 6),
    );
    println!("off the page's ground: {edge} px down the right edge, {inside} px left of it");
    assert!(edge > 20, "a scroller down the right edge");
    assert_eq!(inside, 0, "and the page's ground beside it");
}
