//! The parameter window's screenshot probes, as scripts/ui-check.sh shoots it alone
//! (`--screenshot-window params`) at its first size. Positions come from the window's own
//! constants (`ui/param_window.rs`), colors from [`crate::ui::style`]; the list's foot, which iced
//! lays out under the list, is found as the end of the scroller's rail down the window's right
//! edge.

use iced::Point;

use super::tests::{read_ui_shot, rgb8};
use super::{near, Screenshot};
use crate::ui::instrument_window::{HINT_HEIGHT, PADDING};
use crate::ui::param_window::{
    HEADING_HEIGHT, NAME_WIDTH, ROW_HEIGHT, SLIDER_HEIGHT, SLIDER_WIDTH, SLIDER_X, WINDOW_SIZE,
};
use crate::ui::style;

/// How far a filled box may be off its color.
const FACE: u8 = 6;
/// How far a glyph's brightest pixels may be off their text's color.
const GLYPH: u8 = 40;

/// The middle of row `row` of a list scrolled to its top.
fn row_middle(row: usize) -> f32 {
    HEADING_HEIGHT + ROW_HEIGHT * (row as f32 + 0.5)
}

/// How many pixels of the box `x` by `y` are near `color` (within `tolerance`).
fn count(
    shot: &Screenshot,
    x: std::ops::Range<f32>,
    y: std::ops::Range<f32>,
    color: iced::Color,
    tolerance: u8,
) -> usize {
    let color = rgb8(color);
    shot.count_matching(x, y, |pixel| near(pixel, color, tolerance))
}

/// The shot's size in logical pixels.
fn size(shot: &Screenshot) -> (f32, f32) {
    (
        shot.width as f32 / shot.scale,
        shot.height as f32 / shot.scale,
    )
}

// P1: p27_vst3_fx's VST3 test gain on Insert 1, opened by --open-plugin effect:1 (it has no
// editor). The heading's place is drawn in the text's color; row 0 (Gain, 0.5) has its name in
// the text's color, its slider filled to its middle and its value written right of it; row 1
// (Steps, 4 of 0..4) is filled to its end; row 2 (Steps heard, read-only, 0) has its name dim and
// none in the text's color, and an empty slider. Under the three rows the window's ground; the
// footer's Close button and the foot line on the window's ground at its foot.
#[test]
#[ignore = "needs a screenshot of the parameter window (--open-plugin effect:1 --screenshot-window params on p27_vst3_fx.json): scripts/ui-check.sh takes one and runs this"]
fn p1_the_parameter_window_heads_its_list_and_draws_its_rows() {
    let shot = read_ui_shot();
    let (width, height) = size(&shot);
    assert_eq!(
        (width, height),
        (WINDOW_SIZE.width, WINDOW_SIZE.height),
        "the parameter window at its first size"
    );
    let names = PADDING..PADDING + NAME_WIDTH;
    let heading = count(&shot, names.clone(), 8.0..32.0, style::TEXT, GLYPH);
    println!("heading: {heading} pixels of text");
    assert!(heading > 10, "the place is written");
    let band = |row: usize| {
        let middle = row_middle(row);
        (middle - ROW_HEIGHT / 2.0 + 2.0)..(middle + ROW_HEIGHT / 2.0 - 2.0)
    };
    let gain_name = count(&shot, names.clone(), band(0), style::TEXT, GLYPH);
    let gain_value = count(
        &shot,
        (SLIDER_X + SLIDER_WIDTH + 8.0)..(SLIDER_X + SLIDER_WIDTH + 60.0),
        band(0),
        style::TEXT,
        GLYPH,
    );
    println!("Gain: name {gain_name} pixels, value {gain_value} pixels");
    assert!(gain_name > 5, "row 0's name");
    assert!(gain_value > 5, "row 0's value");
    let slider = |row: usize, along: f32| {
        shot.pixel(Point::new(SLIDER_X + SLIDER_WIDTH * along, row_middle(row)))
    };
    let (fill, track) = (rgb8(style::SLIDER_FILL), rgb8(style::FADER_TRACK));
    println!(
        "sliders: {:02X?} {:02X?} {:02X?} {:02X?}",
        slider(0, 0.25),
        slider(0, 0.75),
        slider(1, 0.95),
        slider(2, 0.25)
    );
    assert!(near(slider(0, 0.25), fill, FACE), "Gain filled to 0.5");
    assert!(near(slider(0, 0.75), track, FACE), "and no further");
    assert!(near(slider(1, 0.95), fill, FACE), "Steps filled to its end");
    assert!(near(slider(2, 0.25), track, FACE), "Steps heard at 0");
    let top_of_slider = row_middle(0) - SLIDER_HEIGHT / 2.0;
    assert!(
        near(
            shot.pixel(Point::new(SLIDER_X + 2.0, top_of_slider - 3.0)),
            rgb8(style::PANEL),
            FACE
        ),
        "the slider is {SLIDER_HEIGHT} px tall in its row"
    );
    let bright = count(&shot, names.clone(), band(2), style::TEXT, 30);
    let dim = count(&shot, names, band(2), style::TEXT_DIM, GLYPH);
    println!("Steps heard: {bright} bright and {dim} dim pixels");
    assert_eq!(bright, 0, "a read-only row is dim");
    assert!(dim > 5, "its name is written");
    let under = Point::new(SLIDER_X + SLIDER_WIDTH / 2.0, row_middle(5));
    assert!(
        near(shot.pixel(under), rgb8(style::PANEL), FACE),
        "no row 3"
    );
    let foot = height - HINT_HEIGHT;
    let close = count(
        &shot,
        (width - 100.0)..(width - PADDING),
        (foot - 34.0)..foot,
        style::BUTTON,
        FACE,
    );
    println!("Close: {close} pixels of the button's face");
    assert!(close > 200, "the footer's Close");
    assert!(
        near(
            shot.pixel(Point::new(width - 4.0, height - HINT_HEIGHT / 2.0)),
            rgb8(style::WINDOW),
            FACE
        ),
        "the foot line"
    );
}

// P2: Surge XT's VST3 (775 parameters) with --open-params channel:1 --scroll end: the list starts
// at its end, so the scroller's thumb sits at the foot of its rail (the rail's last pixel is
// lighter than its first), and the rows are built there: slider bands (track or fill) run all
// the way down the list, at least 14 of them, the last within a row of the list's foot.
#[test]
#[ignore = "needs a screenshot of Surge XT's parameter window scrolled to its end (--open-params channel:1 --scroll end --screenshot-window params): scripts/ui-check.sh takes one when Surge XT's VST3 is fetched and runs this"]
fn p2_a_long_list_scrolled_to_its_end_builds_its_last_rows() {
    let shot = read_ui_shot();
    let (width, height) = size(&shot);
    assert_eq!((width, height), (WINDOW_SIZE.width, WINDOW_SIZE.height));
    let ground = rgb8(style::PANEL);
    let edge = width - 3.0;
    let rail: Vec<f32> = (0..(height * 2.0) as usize)
        .map(|half| half as f32 / 2.0)
        .filter(|&y| y >= HEADING_HEIGHT && y < height - HINT_HEIGHT)
        .filter(|&y| !near(shot.pixel(Point::new(edge, y)), ground, FACE))
        .collect();
    let (Some(&top), Some(&bottom)) = (rail.first(), rail.last()) else {
        panic!("no scroller rail down the right edge");
    };
    let luminance = |y: f32| {
        let [r, g, b] = shot.pixel(Point::new(edge, y));
        u32::from(r) + u32::from(g) + u32::from(b)
    };
    let (first, last) = (luminance(top + 1.0), luminance(bottom - 1.0));
    println!("rail {top}..{bottom}: {first} at its top, {last} at its foot");
    assert!(last > first + 30, "the thumb at the rail's foot");
    let (fill, track) = (rgb8(style::SLIDER_FILL), rgb8(style::FADER_TRACK));
    let x = SLIDER_X + SLIDER_WIDTH - 2.0;
    let mut bands: Vec<(f32, f32)> = Vec::new();
    let mut y = top;
    while y <= bottom {
        let pixel = shot.pixel(Point::new(x, y));
        if near(pixel, fill, FACE) || near(pixel, track, FACE) {
            match bands.last_mut() {
                Some(band) if y - band.1 <= 0.5 => band.1 = y,
                _ => bands.push((y, y)),
            }
        }
        y += 0.5;
    }
    println!("{} slider bands: {bands:?}", bands.len());
    let whole: Vec<&(f32, f32)> = bands
        .iter()
        .filter(|band| band.1 - band.0 >= SLIDER_HEIGHT - 2.0)
        .collect();
    assert!(whole.len() >= 14, "the rows in view are built");
    let lowest = bands.last().map_or(0.0, |band| band.1);
    assert!(
        lowest > bottom - ROW_HEIGHT,
        "the last row at the list's foot"
    );
}
