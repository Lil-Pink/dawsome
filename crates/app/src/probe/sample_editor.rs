//! The sample editor's screenshot probes, as scripts/ui-check.sh shoots it alone
//! (`--screenshot-window sample-editor`) at its first size: p30_fades_stereo's sampler
//! (gen_p30's stereo pair, 1.0 on the left and −0.5 on the right, faded in over frames 0..4 and
//! out over 4..8), frames 2..6 selected (`--sample-editor-select 2-6`). Positions come from the
//! window's constants (`ui/sample_editor.rs`, `ui/sample_view.rs`), colors from
//! [`crate::ui::style`], the frames' values from tests/fixtures/p30_expected.json.

use iced::Point;

use super::tests::{read_ui_shot, rgb8};
use super::{near, Screenshot};
use crate::ui::instrument_window::{HINT_HEIGHT, PADDING};
use crate::ui::sample_editor::{waveform_area, WINDOW_SIZE};
use crate::ui::sample_view::{self, View, RULER_HEIGHT};
use crate::ui::style;

/// How far a filled box may be off its color, and a dot drawn round (its edge smoothed).
const FACE: u8 = 3;
const DOT: u8 = 12;
/// How far a glyph's pixels may be off their text's color.
const GLYPH: u8 = 40;
/// How strongly the selection lights what is under it: the view's own share of the text color.
const LIT: f32 = 0.14;

/// `top` over `base` at `alpha`, channel by channel, as the window blends a shade over the wave.
fn over(base: iced::Color, top: iced::Color, alpha: f32) -> [u8; 3] {
    let mix = |under: f32, above: f32| ((under * (1.0 - alpha) + above * alpha) * 255.0).round();
    [
        mix(base.r, top.r) as u8,
        mix(base.g, top.g) as u8,
        mix(base.b, top.b) as u8,
    ]
}

/// Whether `pixel` is of the accent's warmth: far more red than blue, as nothing else in the
/// waveform is (the edges are a 1 px line between two pixels, so neither is the accent itself).
fn warm(pixel: [u8; 3]) -> bool {
    i32::from(pixel[0]) - i32::from(pixel[2]) > 60
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

// E1: the window at its first size. Under the ruler (the heading's color) two lanes, left
// over right, their ground past the sound's eight frames, the wells' color in the gap between
// them; each frame outside the selection a dot where gen_p30 puts its value (frames 1 and 7,
// both sides); between the selection's edges (frames 2 and 6, 16 px a frame) the ground lit by
// the text color at 0.14, outside them unlit, each edge warm; the foot line's words dim on
// the window's ground.
#[test]
#[ignore = "needs a screenshot of the sample editor (--open-sample-editor channel:1 --sample-editor-select 2-6 --screenshot-window sample-editor on p30_fades_stereo.json): scripts/ui-check.sh takes one and runs this"]
fn e1_the_editor_draws_two_lanes_the_selection_and_its_foot_line() {
    let shot = read_ui_shot();
    let size = (
        shot.width as f32 / shot.scale,
        shot.height as f32 / shot.scale,
    );
    assert_eq!(
        size,
        (WINDOW_SIZE.width, WINDOW_SIZE.height),
        "the editor at its first size"
    );
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/p30_expected.json"
    ))
    .expect("the known answers");
    let answers: serde_json::Value = serde_json::from_str(&text).expect("parses");
    let case = &answers["ops"]["fades_stereo"];
    let out: Vec<f32> = case["out"]
        .as_array()
        .expect("the frames")
        .iter()
        .map(|value| value.as_f64().expect("a value") as f32)
        .collect();
    let frames = case["frames"].as_u64().expect("frames");
    let area = waveform_area(WINDOW_SIZE);
    let view = View::fit(frames, area.width).showing(2, 6);
    let x = |frame: f64| area.x + view.x_of(frame);
    let ruler = shot.pixel(Point::new(area.x + 600.0, area.y + RULER_HEIGHT / 2.0));
    println!("ruler {ruler:02X?}");
    assert!(near(ruler, rgb8(style::HEADER), FACE), "the ruler");
    let lanes = sample_view::lanes(area.height, 2);
    assert_eq!(lanes.len(), 2, "a lane a side");
    for (side, &(top, height)) in lanes.iter().enumerate() {
        let top = area.y + top;
        let ground = shot.pixel(Point::new(area.x + 600.0, top + height / 4.0));
        println!("lane {side}: ground {ground:02X?}");
        assert!(near(ground, rgb8(style::AUDIO_BODY), FACE), "lane {side}");
        for frame in [1, 7] {
            let value = out[frame * 2 + side];
            let at = Point::new(
                x(frame as f64 + 0.5),
                sample_view::value_y(top, height, value),
            );
            let dot = shot.pixel(at);
            println!("lane {side}, frame {frame} ({value}): {dot:02X?} at {at:?}");
            assert!(
                near(dot, rgb8(style::AUDIO_WAVE), DOT),
                "frame {frame}'s dot"
            );
        }
        let low = top + height * 0.9;
        let lit = shot.pixel(Point::new(x(4.0), low));
        let unlit = shot.pixel(Point::new(x(1.0), low));
        println!("lane {side}: selected {lit:02X?}, not {unlit:02X?}");
        assert!(
            near(lit, over(style::AUDIO_BODY, style::TEXT, LIT), FACE),
            "the selection lights lane {side}"
        );
        assert!(near(unlit, rgb8(style::AUDIO_BODY), FACE), "and only it");
        for edge in [2.0, 6.0] {
            let at = x(edge);
            let warmth = shot.count_matching((at - 1.0)..(at + 1.0), low..(low + 1.0), warm);
            assert!(
                warmth >= 1,
                "lane {side}: the edge at frame {edge} is in the accent"
            );
        }
    }
    let (first, second) = (lanes[0], lanes[1]);
    let gap = shot.pixel(Point::new(
        area.x + 600.0,
        area.y + (first.0 + first.1 + second.0) / 2.0,
    ));
    println!("gap {gap:02X?}");
    assert!(
        near(gap, rgb8(style::RACK_WELL), FACE),
        "the gap between the lanes"
    );
    let foot = WINDOW_SIZE.height - HINT_HEIGHT;
    let words = count(
        &shot,
        PADDING..400.0,
        foot..WINDOW_SIZE.height,
        style::TEXT_DIM,
        GLYPH,
    );
    println!("foot line: {words} pixels of dim text");
    assert!(words > 20, "the foot line says how to use it");
    let ground = shot.pixel(Point::new(
        WINDOW_SIZE.width - PADDING,
        foot + HINT_HEIGHT / 2.0,
    ));
    assert!(
        near(ground, rgb8(style::WINDOW), FACE),
        "on the window's ground"
    );
}
