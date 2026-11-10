//! Pixel probe for headless screenshots. For the rack it samples the center of every step button in
//! a row and sorts each sample into lit, one of the two unlit tones, or something else; for the
//! piano roll it samples notes, ghosts, rows, lines and stems against their colors; for the
//! playlist, clips, lanes, bar lines, bar numbers and the loop strip; for the mixer, fader caps,
//! strip bodies, the selection outline, route buttons, meters and slot names. The colors are the
//! palette the views draw with. Positions come from [`crate::ui::layout`],
//! [`crate::ui::piano_layout`], [`crate::ui::playlist_layout`] and [`crate::ui::mixer_layout`].

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use iced::Point;

use crate::headless::SCALE_FACTOR_KEYWORD;
use crate::ui::layout;
use crate::ui::piano_layout::GRID_TOP;
use crate::ui::playlist_layout::{HEADER_HEIGHT, RULER_HEIGHT};

mod accent;
mod ease;
mod fixes;
mod help;
mod keys;
mod midi_out;
mod params;
mod roll_strip;
mod sample_editor;
mod synth;
mod theme;

/// The first channel's lit color, the color the rack draws it with.
fn lit_first_channel() -> [u8; 3] {
    rgb(crate::ui::style::CHANNEL_COLORS[0])
}

/// The two unlit tones the rack draws with (Graphite's, the shots' theme). Unlit steps alternate
/// tone by beat: steps 1-4 and 9-12 of a bar use the first tone, steps 5-8 and 13-16 the second.
fn unlit() -> [[u8; 3]; 2] {
    crate::ui::palette::GRAPHITE.step_unlit.map(rgb)
}

fn rgb(color: iced::Color) -> [u8; 3] {
    let [r, g, b, _] = color.into_rgba8();
    [r, g, b]
}
/// Largest per-channel difference still counted as a match: a color-space round trip on the
/// GPU can move a value by one.
const TOLERANCE: u8 = 2;

/// What a probed pixel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    Lit,
    /// Unlit, in the tone with this index into [`unlit`].
    Unlit(usize),
    Other([u8; 3]),
}

/// A decoded screenshot and the scale factor it was taken at.
pub(crate) struct Screenshot {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub rgba: Vec<u8>,
}

impl Screenshot {
    /// Reads a PNG written by the app. Panics with the reason when it cannot: this is test code.
    pub fn read(path: &Path) -> Self {
        let file = File::open(path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
        let mut reader = png::Decoder::new(BufReader::new(file))
            .read_info()
            .expect("PNG header");
        let info = reader.info();
        assert_eq!(
            (info.color_type, info.bit_depth),
            (png::ColorType::Rgba, png::BitDepth::Eight),
            "the app writes 8-bit RGBA"
        );
        let scale = info
            .uncompressed_latin1_text
            .iter()
            .find(|chunk| chunk.keyword == SCALE_FACTOR_KEYWORD)
            .map(|chunk| chunk.text.parse::<f32>().expect("numeric scale factor"))
            .expect("the screenshot records its scale factor");
        let (width, height) = (info.width, info.height);
        let mut rgba = vec![0; reader.output_buffer_size().expect("image fits in memory")];
        reader.next_frame(&mut rgba).expect("PNG data");
        Self {
            width,
            height,
            scale,
            rgba,
        }
    }

    /// The color of the pixel under a logical point.
    pub fn pixel(&self, logical: Point) -> [u8; 3] {
        let x = (logical.x * self.scale) as u32;
        let y = (logical.y * self.scale) as u32;
        assert!(
            x < self.width && y < self.height,
            "{logical:?} lies outside the {}x{} screenshot",
            self.width,
            self.height
        );
        let offset = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.rgba[offset],
            self.rgba[offset + 1],
            self.rgba[offset + 2],
        ]
    }
}

/// Whether every channel of `pixel` is within `tolerance` of `expected`.
fn near(pixel: [u8; 3], expected: [u8; 3], tolerance: u8) -> bool {
    pixel
        .iter()
        .zip(expected)
        .all(|(&value, expected)| value.abs_diff(expected) <= tolerance)
}

/// Where grid point `point` is in the window when the piano roll is the only panel: under the
/// toolbar, the strip and the ruler.
pub(crate) fn roll_grid_in_window(point: Point) -> Point {
    Point::new(point.x, point.y + layout::TOOLBAR_HEIGHT + GRID_TOP)
}

/// Where ruler point `point` is in the window when the playlist is the only panel: under the
/// toolbar and the playlist's header strip.
pub(crate) fn ruler_in_window(point: Point) -> Point {
    Point::new(point.x, point.y + layout::TOOLBAR_HEIGHT + HEADER_HEIGHT)
}

/// Where tracks point `point` is in the window when the playlist is the only panel: under the
/// toolbar, the header strip and the ruler.
pub(crate) fn tracks_in_window(point: Point) -> Point {
    let ruler = ruler_in_window(point);
    Point::new(ruler.x, ruler.y + RULER_HEIGHT)
}

impl Screenshot {
    /// How many pixels inside the logical box `x` by `y` are [`near`] `color`.
    fn count_near(
        &self,
        x: std::ops::Range<f32>,
        y: std::ops::Range<f32>,
        color: [u8; 3],
        tolerance: u8,
    ) -> usize {
        self.count_matching(x, y, |pixel| near(pixel, color, tolerance))
    }

    /// The logical x of the rightmost pixel column inside the box `x` by `y` holding a pixel
    /// `matches` accepts.
    fn rightmost_matching(
        &self,
        x: std::ops::Range<f32>,
        y: std::ops::Range<f32>,
        matches: impl Fn([u8; 3]) -> bool,
    ) -> Option<f32> {
        let columns = ((x.start * self.scale) as u32)..((x.end * self.scale) as u32);
        let rows = ((y.start * self.scale) as u32)..((y.end * self.scale) as u32);
        columns.rev().find_map(|column| {
            rows.clone()
                .any(|row| {
                    let offset = (row as usize * self.width as usize + column as usize) * 4;
                    matches([
                        self.rgba[offset],
                        self.rgba[offset + 1],
                        self.rgba[offset + 2],
                    ])
                })
                .then(|| column as f32 / self.scale)
        })
    }

    /// The logical spans, left to right, of the runs of pixels `matches` accepts along the
    /// pixel row under logical `y`. One pixel it refuses between two it accepts does not end a
    /// run. Why: the software renderer blends a button's dark border into its lighter face
    /// through a column that can match the toolbar's ground (File's left edge at scale 1.7,
    /// 0x2A2D32 on the 0x2B2E33 ground), which split one button into two runs; the gaps
    /// between toolbar items are many pixels wide.
    fn runs_along(&self, y: f32, matches: impl Fn([u8; 3]) -> bool) -> Vec<std::ops::Range<f32>> {
        let (row, width) = ((y * self.scale) as usize, self.width as usize);
        let inside = |column: usize| {
            column < width && {
                let offset = (row * width + column) * 4;
                matches([
                    self.rgba[offset],
                    self.rgba[offset + 1],
                    self.rgba[offset + 2],
                ])
            }
        };
        let mut runs = Vec::new();
        let mut start = None;
        for column in 0..=width {
            match (inside(column), start) {
                (true, None) => start = Some(column),
                (false, Some(first)) if !inside(column + 1) => {
                    runs.push(first as f32 / self.scale..column as f32 / self.scale);
                    start = None;
                }
                _ => {}
            }
        }
        runs
    }

    /// How many pixels inside the logical box `x` by `y` `matches` accepts.
    fn count_matching(
        &self,
        x: std::ops::Range<f32>,
        y: std::ops::Range<f32>,
        matches: impl Fn([u8; 3]) -> bool,
    ) -> usize {
        let pixels = |range: &std::ops::Range<f32>| {
            ((range.start * self.scale) as u32)..((range.end * self.scale) as u32)
        };
        let (columns, rows) = (pixels(&x), pixels(&y));
        rows.flat_map(|row| columns.clone().map(move |column| (column, row)))
            .filter(|&(column, row)| {
                let offset = (row as usize * self.width as usize + column as usize) * 4;
                let pixel = [
                    self.rgba[offset],
                    self.rgba[offset + 1],
                    self.rgba[offset + 2],
                ];
                matches(pixel)
            })
            .count()
    }
}

/// Where the probe samples step `step` of `row`: the button's center, window coordinates.
pub(crate) fn step_center_in_window(row: usize, step: u32) -> Point {
    layout::rack_in_window(layout::step_rect(row, step, 4).center())
}

/// The tone at the center of each of the first `steps` step buttons in `row`.
pub(crate) fn probe_row(shot: &Screenshot, row: usize, steps: u32) -> Vec<Tone> {
    (0..steps)
        .map(|step| tone(shot.pixel(step_center_in_window(row, step))))
        .collect()
}

fn tone(pixel: [u8; 3]) -> Tone {
    let near = |reference: [u8; 3]| near(pixel, reference, TOLERANCE);
    if near(lit_first_channel()) {
        return Tone::Lit;
    }
    match unlit().into_iter().position(near) {
        Some(index) => Tone::Unlit(index),
        None => Tone::Other(pixel),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Step 0 of every group of four lit, as F1 is drawn; the other steps of groups 1 and 3
    /// in the first unlit tone, of groups 2 and 4 in the second.
    fn every_fourth_lit() -> Vec<Tone> {
        let unlit = [Tone::Unlit(0), Tone::Unlit(1)];
        (0..16)
            .map(|step| match step % 4 {
                0 => Tone::Lit,
                _ => unlit[step / 4 % 2],
            })
            .collect()
    }

    // The tones as gen_ui_contrast.py gives them (the first channel #E5533D, Graphite's unlit
    // steps a shade down, #34383E and #2A2E34), each matched within a GPU's ±1 or ±2 a channel; a
    // pixel half way between the two unlit tones (#2F3339, 5 off each), the red 4 off in blue,
    // and the red with its channels swapped are other colors.
    #[test]
    fn classification_tolerates_rounding_but_not_other_colors() {
        assert_eq!(lit_first_channel(), [0xE5, 0x53, 0x3D]);
        assert_eq!(unlit(), [[0x34, 0x38, 0x3E], [0x2A, 0x2E, 0x34]]);
        assert_eq!(tone([0xE5, 0x53, 0x3D]), Tone::Lit);
        assert_eq!(tone([0xE4, 0x54, 0x3F]), Tone::Lit);
        assert_eq!(tone([0x34, 0x38, 0x3E]), Tone::Unlit(0));
        assert_eq!(tone([0x35, 0x37, 0x3F]), Tone::Unlit(0));
        assert_eq!(tone([0x2B, 0x2E, 0x33]), Tone::Unlit(1));
        assert_eq!(tone([0x2F, 0x33, 0x39]), Tone::Other([0x2F, 0x33, 0x39]));
        assert_eq!(tone([0xE5, 0x53, 0x41]), Tone::Other([0xE5, 0x53, 0x41]));
        assert_eq!(tone([0x3D, 0x53, 0xE5]), Tone::Other([0x3D, 0x53, 0xE5]));
    }

    // A row painted by hand at scale 2, on and off per pixel: on, off, on, on, off, off, on,
    // off. One pixel off between two on joins them, two end a run, and so does the row's end:
    // the runs are pixels [0, 4) and [6, 7), logical [0, 2) and [3, 3.5).
    #[test]
    fn a_run_survives_one_refused_pixel_but_not_two() {
        let row = [true, false, true, true, false, false, true, false];
        let rgba = row
            .iter()
            .flat_map(|&on| if on { [255; 4] } else { [0, 0, 0, 255] })
            .collect();
        let shot = Screenshot {
            width: row.len() as u32,
            height: 1,
            scale: 2.0,
            rgba,
        };
        assert_eq!(
            shot.runs_along(0.0, |pixel| pixel[0] == 255),
            [0.0..2.0, 3.0..3.5]
        );
    }

    // A screenshot painted by hand at scale 1.5 (1500x840 pixels for a 1000x560 window): each
    // step button is a box where the rack lays it out, F1's steps lit, step 6 an unknown color.
    #[test]
    fn a_scaled_screenshot_is_probed_at_the_step_centers() {
        let scale = 1.5;
        let (width, height) = (1500_usize, 840_usize);
        let mut rgba = vec![0_u8; width * height * 4];
        for step in 0..16_u32 {
            let color = match step {
                6 => [0, 0, 0xFF],
                s if s % 4 == 0 => lit_first_channel(),
                s => unlit()[(s / 4 % 2) as usize],
            };
            // Step i's box: left edge 222 + 30i + 6⌊i/4⌋ (an extra gap after every four
            // steps), top 96 (under the 64 px toolbar, the 24 px rack header and 8 px of
            // padding; pin moved from 88 under the 56 px one, 104 under the 72 px one), 28
            // square.
            let left = 222.0 + 30.0 * step as f32 + 6.0 * (step / 4) as f32;
            let (x0, x1) = ((left * scale) as usize, ((left + 28.0) * scale) as usize);
            let (y0, y1) = ((96.0 * scale) as usize, (124.0 * scale) as usize);
            for y in y0..y1 {
                for x in x0..x1 {
                    let offset = (y * width + x) * 4;
                    rgba[offset..offset + 3].copy_from_slice(&color);
                }
            }
        }
        let shot = Screenshot {
            width: width as u32,
            height: height as u32,
            scale,
            rgba,
        };
        let mut expected = every_fourth_lit();
        expected[6] = Tone::Other([0, 0, 0xFF]);
        assert_eq!(probe_row(&shot, 0, 16), expected);
    }

    #[test]
    #[ignore = "needs a screenshot of F1: scripts/ui-check.sh takes one and runs this"]
    fn f1_screenshot_lights_steps_1_5_9_13() {
        let path = std::env::var_os("DAWSOME_UI_SHOT")
            .expect("DAWSOME_UI_SHOT names the screenshot to probe");
        let shot = Screenshot::read(Path::new(&path));
        let tones = probe_row(&shot, 0, 16);
        for (step, tone) in (0..16).zip(&tones) {
            let at = step_center_in_window(0, step);
            println!("step {:2} at ({}, {}): {tone:?}", step + 1, at.x, at.y);
        }
        println!(
            "{}x{} px, scale factor {}",
            shot.width, shot.height, shot.scale
        );
        assert_eq!(tones, every_fourth_lit());
    }

    // F1 opened with `--time-signature 6/8` at 1200 x 560, the rack alone: the signature's
    // edit grows F1's one bar to the bars of 288 ticks holding its last hit (tick 288, ending at
    // 289): 2 bars, 2 · 12 = 24 steps, its hits on ticks 0, 96, 192 and 288 lighting steps 0, 4,
    // 8 and 12. The groups are a beat wide, an eighth, 2 steps (gen_p34_expected.py's step
    // grid): step s at x 222 + 30s + 6 · floor(s / 2), its unlit tone by group (the first tone
    // for groups 0, 2, 4, …), a beat line (Graphite's soft line, 2 px) in the middle of each
    // gap between groups, (222 + 30 · (2k − 1) + 6 · (k − 1) + 28 + 222 + 60k + 6k) / 2 = 284
    // for k = 1, 350 for k = 2, and none between the steps of a group (x 251, the rack's
    // ground); no step past step 23 (step 24's place, x 222 + 720 + 72 + 14 = 1028, holds no
    // step's tone).
    #[test]
    #[ignore = "needs a screenshot of F1 in 6/8: scripts/ui-check.sh takes one and runs this"]
    fn f1_in_6_8_draws_two_bars_of_twelve_steps_in_groups_of_two() {
        let shot = read_ui_shot();
        let group = 2;
        let center = |step: u32| layout::rack_in_window(layout::step_rect(0, step, group).center());
        let tones: Vec<Tone> = (0..24).map(|step| tone(shot.pixel(center(step)))).collect();
        let want: Vec<Tone> = (0..24)
            .map(|step| match step {
                0 | 4 | 8 | 12 => Tone::Lit,
                s => Tone::Unlit((s / 2 % 2) as usize),
            })
            .collect();
        assert_eq!(tones, want);
        let y = center(0).y;
        let palette = crate::ui::palette::GRAPHITE;
        let rgb = |color: iced::Color| color.into_rgba8()[..3].try_into().expect("three");
        for x in [284.0, 350.0] {
            let pixel = shot.pixel(Point::new(x, y));
            assert!(
                near(pixel, rgb(palette.line_soft), TOLERANCE),
                "a beat line at x {x}: {pixel:?}"
            );
        }
        let between = shot.pixel(Point::new(251.0, y));
        assert!(
            near(between, rgb(palette.bg), TOLERANCE),
            "the rack's ground inside a group: {between:?}"
        );
        let past = tone(shot.pixel(Point::new(1028.0, y)));
        assert!(matches!(past, Tone::Other(_)), "no step 24: {past:?}");
    }

    // The new document with the rack alone (positions from crate::ui::layout, colors from
    // crate::ui::style): its one row, "Sampler", lights no step, so its 16 steps show the two
    // unlit tones by beat; under it the add row (rack y 46..70): "+ Instrument…" its words
    // alone at the name boxes' x, where a press takes them 28..126.395 (12 · 14062 / 2048 =
    // 82.395 px of words, layout's test, and 8 px either side) by the row's 46..70, in the
    // secondary text's tone with no text-toned pixel (nothing points at them), no face (its
    // corner 2 px in, rack (30, 48), the rack's ground bg), and right of them the rack's well,
    // bg too (rack (140, 58)). In the header above the rows, "−" and "+" are buttons, and the
    // bar count's dark display holds bright text (its "1"). (Pins moved: "+ Instrument…" stood
    // in the header, and the well under the row at rack (30, 48), until it came down into the
    // add row, a button 28..128 by 48..68 for a day, its face at rack (30, 50).)
    #[test]
    #[ignore = "needs a screenshot of the new document: scripts/ui-check.sh takes one and runs this"]
    fn new_document_screenshot_shows_the_rack_header_and_an_unlit_sampler_row() {
        use crate::ui::style;
        let shot = read_ui_shot();
        let unlit: Vec<Tone> = (0..16).map(|step| Tone::Unlit(step / 4 % 2)).collect();
        assert_eq!(
            probe_row(&shot, 0, 16),
            unlit,
            "the Sampler row lights no step"
        );
        let header = |at: Point| Point::new(at.x, at.y + layout::TOOLBAR_HEIGHT);
        let corner = |rect: iced::Rectangle| Point::new(rect.x + 2.0, rect.y + 2.0);
        let button = rgb8(style::palette().button);
        for rect in [layout::bars_down_rect(), layout::bars_up_rect()] {
            let pixel = shot.pixel(header(corner(rect)));
            assert!(near(pixel, button, TOLERANCE), "{rect:?}: {pixel:?}");
        }
        let add = layout::add_instrument_rect(1);
        assert_eq!((add.x, add.y, add.height), (28.0, 46.0, 24.0));
        assert!((add.width - 98.395).abs() < 1e-3, "{add:?}");
        let ground = rgb8(style::palette().bg);
        let face = shot.pixel(layout::rack_in_window(corner(add)));
        assert!(
            near(face, ground, TOLERANCE),
            "+ Instrument…, no face: {face:?}"
        );
        let top = layout::rack_in_window(add.position());
        let words = |color: iced::Color| {
            shot.count_matching(
                top.x..top.x + add.width,
                top.y..top.y + add.height,
                |pixel| near(pixel, rgb8(color), TOLERANCE),
            )
        };
        let (resting, lit) = (
            words(style::palette().text_secondary),
            words(style::palette().text),
        );
        println!("+ Instrument…: {resting} secondary-toned pixels, {lit} text-toned");
        assert!(resting > 5 && lit == 0, "the words at rest");
        let under = shot.pixel(layout::rack_in_window(Point::new(140.0, 58.0)));
        assert!(
            near(under, ground, TOLERANCE),
            "the well right of the words: {under:?}"
        );
        let count = layout::bars_count_rect();
        let ground = shot.pixel(header(corner(count)));
        assert!(
            near(ground, rgb8(style::palette().lcd), TOLERANCE),
            "{ground:?}"
        );
        let top = count.y + layout::TOOLBAR_HEIGHT;
        let lit = shot.count_matching(
            count.x..count.x + count.width,
            top..top + count.height,
            |pixel| pixel.iter().all(|&channel| channel > 0x80),
        );
        println!("bright pixels in the bar count: {lit}");
        assert!(lit > 0, "the bar count shows a number");
        // The pattern's menu opens where its name's box starts, toolbar coordinates (where
        // the spread across the shot's width puts it). Pin moved: the box is joined to the
        // previous arrow, so 4 px left of it is the arrow's face (above its glyph), not the
        // toolbar's ground, which starts a pixel left of the arrow. The piece in one frame: the
        // name on the readouts' dark glass, the frame's edge at the piece's left, 4 px under the
        // groups' top (above the arrow's glyph).
        let width = shot.width as f32 / shot.scale;
        let spread = crate::ui::toolbar::shot_spread(width, false);
        let name = spread.pattern_name_x();
        let y = crate::ui::toolbar::group_rows().start + 4.0;
        let inside = shot.pixel(Point::new(name + 3.0, y));
        assert!(
            near(inside, rgb8(style::palette().lcd), TOLERANCE),
            "{inside:?}"
        );
        let arrow = shot.pixel(Point::new(name - 4.0, y));
        assert!(
            near(arrow, rgb8(style::palette().button), TOLERANCE),
            "{arrow:?}"
        );
        let piece = spread
            .left(crate::ui::toolbar::Group::Pattern)
            .expect("the pattern shows");
        let edge = shot.pixel(Point::new(piece + 0.5, y));
        assert!(
            near(edge, rgb8(style::palette().button_edge), TOLERANCE),
            "{edge:?}"
        );
        let before = shot.pixel(Point::new(piece - 1.0, y));
        assert!(
            near(before, rgb8(style::palette().panel), TOLERANCE),
            "{before:?}"
        );
    }

    /// p3_roll as the piano roll opens on it (top key 69, 16 px per step), its notes A and B,
    /// and the kicks on ticks 192 and 288.
    mod p3 {
        use dawsome_engine::project::{ChannelId, Note};
        use dawsome_engine::time::Ticks;

        use crate::ui::piano_layout::{self, View};

        pub fn view() -> View {
            View {
                top: 69,
                scroll: Ticks(0),
                zoom: piano_layout::ZOOM.default,
                step: Ticks(24),
                signature: dawsome_engine::time::TimeSignature::COMMON,
                rows: crate::ui::piano_layout::Rows::ALL,
            }
        }

        pub fn note(channel: u32, tick: i64, length: i64, velocity: f32, key: u8) -> Note {
            Note {
                channel: ChannelId(channel),
                start: Ticks(tick),
                length: Ticks(length),
                velocity,
                key,
            }
        }
    }

    /// Where lane point `point` is in a window `height` tall when the piano roll is the only
    /// panel: the lane sits at the bottom.
    fn roll_lane_in_window(point: Point, height: f32) -> Point {
        use crate::ui::piano_layout::LANE_HEIGHT;
        Point::new(point.x, point.y + height - LANE_HEIGHT)
    }

    // The piano roll on p3_roll. Expected colors, as tests/fixtures/gen_p3_expected.py prints
    // them: the lead's red #E5533D, the kick's blue at 0.3 over the white row #3C576E, rows #34373D
    // and #2D3035 shaded in bar 1, 5 lighter a channel (#393C42 and #32353A), the bar line
    // #626873, the dimmed row past the end #1F2125, the lane's ground #141619 (the meters'; it
    // was the well's #1F2125 before its lit edges, which lie on its first and last rows, clear of
    // the stem probes), the key label #25282D; past the end the beat line (tick 480, x 368)
    // dimmed as the rows are, #4A4F57 at 0.6, #2C2F34; in the key column C#'s (key 61's) black
    // key #1A1C1F over its first 32 px and the white key #B4B9BF beside it (a shade down from
    // #CFD3D8); on the ruler (y 92
    // under the strip) the beat tick at bar 1's second beat (x 112, its 4 px foot y 104..108)
    // #4A4F57, the beat line's at full strength at 16 px a step, and the ruler's ground #1F2125
    // between ticks (x 144).
    #[test]
    #[ignore = "needs a screenshot of p3_roll with --panels piano-roll: scripts/ui-check.sh takes one and runs this"]
    fn p3_screenshot_shows_the_piano_roll() {
        use crate::ui::grid_rules::{NUMBER_BAND_HEIGHT, NUMBER_TICK_HEIGHT};
        use crate::ui::piano_layout::STRIP_HEIGHT;
        let path = std::env::var_os("DAWSOME_UI_SHOT")
            .expect("DAWSOME_UI_SHOT names the screenshot to probe");
        let shot = Screenshot::read(Path::new(&path));
        let height = shot.height as f32 / shot.scale;
        let view = p3::view();
        let row_center = |key| view.row_top(key) + 6.0;
        let note_center = |note: &dawsome_engine::project::Note| {
            let rect = view.note_rect(note);
            Point::new(rect.center_x(), row_center(note.key))
        };
        let (a, b) = (
            p3::note(1, 0, 48, 0.78125, 60),
            p3::note(1, 96, 96, 0.5, 64),
        );
        let kick = |tick| p3::note(2, tick, 0, 1.0, 60);
        let stem = |note, steps: u8, offset: f32| {
            let rect = view.stem_rect(note, steps).expect("in view");
            roll_lane_in_window(Point::new(rect.center_x(), rect.y + offset), height)
        };
        let end = view.x_of(dawsome_engine::time::Ticks(384));
        let beat_past_end = view.x_of(dawsome_engine::time::Ticks(480));
        let grid = roll_grid_in_window;
        let tick_middle = NUMBER_BAND_HEIGHT - NUMBER_TICK_HEIGHT / 2.0;
        let ruler = |x| Point::new(x, layout::TOOLBAR_HEIGHT + STRIP_HEIGHT + tick_middle);
        let probes: [(&str, Point, [u8; 3]); 17] = [
            ("note A center", grid(note_center(&a)), [0xE5, 0x53, 0x3D]),
            ("note B center", grid(note_center(&b)), [0xE5, 0x53, 0x3D]),
            (
                "ghost kick 192",
                grid(note_center(&kick(192))),
                [0x3C, 0x57, 0x6E],
            ),
            (
                "ghost kick 288",
                grid(note_center(&kick(288))),
                [0x3C, 0x57, 0x6E],
            ),
            (
                "empty white row",
                grid(Point::new(92.0, row_center(60))),
                [0x39, 0x3C, 0x42],
            ),
            (
                "empty black row",
                grid(Point::new(92.0, row_center(61))),
                [0x32, 0x35, 0x3A],
            ),
            (
                "bar line at the end",
                grid(Point::new(end + 0.5, row_center(62))),
                [0x62, 0x68, 0x73],
            ),
            (
                "past the end",
                grid(Point::new(316.0, row_center(60))),
                [0x1F, 0x21, 0x25],
            ),
            (
                "beat line past the end",
                grid(Point::new(beat_past_end + 0.5, row_center(60))),
                [0x2C, 0x2F, 0x34],
            ),
            (
                "black key",
                grid(Point::new(16.0, row_center(61))),
                [0x1A, 0x1C, 0x1F],
            ),
            (
                "white key beside the black",
                grid(Point::new(40.0, row_center(61))),
                [0xB4, 0xB9, 0xBF],
            ),
            ("ruler's beat tick", ruler(112.5), [0x4A, 0x4F, 0x57]),
            ("ruler between ticks", ruler(144.5), [0x1F, 0x21, 0x25]),
            ("stem A inside", stem(&a, 100, 4.0), [0xE5, 0x53, 0x3D]),
            ("above stem A", stem(&a, 100, -4.0), [0x14, 0x16, 0x19]),
            ("stem B inside", stem(&b, 64, 4.0), [0xE5, 0x53, 0x3D]),
            ("above stem B", stem(&b, 64, -4.0), [0x14, 0x16, 0x19]),
        ];
        // The design's window points, written out: the layout must put them there. (The
        // four stem points moved: the lane is 64 px from y 496, a stem half as
        // tall, where they were at y 464, 456, 500 and 492 in the 128 px lane from y 432.
        // The grid's eight points 16 px lower under the 72 px toolbar, then 8 px higher under
        // the 64 px one, as gen_p3_expected.py prints them; the lane's stay, measured from the
        // window's foot. Then the grid's 16 px lower under the ruler, the empty rows and the row
        // past the end read halfway between the faint half-step lines (x 92, 316; were 88, 312),
        // the 1 px bar line on its pixel (304.5, was 305 on the 2 px line), and the stems in the
        // fixed 32 px lane from y 528, a quarter of the 128 px lane's height: y 539, 531, 548 and
        // 540, were 514, 506, 532 and 524 in the 64 px lane. The beat past the end on its pixel,
        // x 48 + 480·16/24 = 368; the keys at x 16 and 40, either side of the black key's edge
        // at 32; the ruler's points in its tick's foot, y 64 + 28 + 14.)
        let spec_points = [
            (64.0, 222.0),
            (144.0, 174.0),
            (184.0, 222.0),
            (248.0, 222.0),
            (92.0, 222.0),
            (92.0, 210.0),
            (304.5, 198.0),
            (316.0, 222.0),
            (368.5, 222.0),
            (16.0, 210.0),
            (40.0, 210.0),
            (112.5, 106.0),
            (144.5, 106.0),
            (50.0, 539.0),
            (50.0, 531.0),
            (114.0, 548.0),
            (114.0, 540.0),
        ];
        let mut wrong = Vec::new();
        for ((name, at, expected), (x, y)) in probes.into_iter().zip(spec_points) {
            assert_eq!((at.x, at.y), (x, y), "{name}: layout moved the probe point");
            let pixel = shot.pixel(at);
            println!(
                "{name} at ({}, {}): {pixel:02X?}, expected {expected:02X?}",
                at.x, at.y
            );
            if !near(pixel, expected, TOLERANCE) {
                wrong.push(name);
            }
        }
        let label = [0x25, 0x28, 0x2D];
        let box_rows = |key| {
            let y = grid(Point::new(0.0, row_center(key))).y;
            (y - 4.0)..(y + 4.0)
        };
        let c5 = shot.count_near(20.0..46.0, box_rows(60), label, 40);
        let d5 = shot.count_near(20.0..46.0, box_rows(62), label, 40);
        println!(
            "label pixels: C5 row {c5}, D5 row {d5}; {}x{} px, scale {}",
            shot.width, shot.height, shot.scale
        );
        assert!(wrong.is_empty(), "wrong color at {wrong:?}");
        assert!(c5 >= 6, "the C5 label shows ({c5} dark pixels)");
        assert_eq!(d5, 0, "the D row has no label");
    }

    /// Probes `points` (name, window point, expected color) on `shot`, printing each, and
    /// returns the names of those off by more than the tolerance.
    fn wrong_colors(shot: &Screenshot, points: &[(String, Point, [u8; 3])]) -> Vec<String> {
        let mut wrong = Vec::new();
        for (name, at, expected) in points {
            let pixel = shot.pixel(*at);
            println!(
                "{name} at ({}, {}): {pixel:02X?}, expected {expected:02X?}",
                at.x, at.y
            );
            if !near(pixel, *expected, TOLERANCE) {
                wrong.push(name.clone());
            }
        }
        wrong
    }

    pub(super) fn read_ui_shot() -> Screenshot {
        let path = std::env::var_os("DAWSOME_UI_SHOT")
            .expect("DAWSOME_UI_SHOT names the screenshot to probe");
        let shot = Screenshot::read(Path::new(&path));
        println!(
            "{}x{} px, scale factor {}",
            shot.width, shot.height, shot.scale
        );
        shot
    }

    /// The playlist as it opens alone in a 1000 x 560 window (64 px per bar, no scroll, track
    /// 1 at the top), its lanes fitted to the 8 tracks of the p4 fixtures in the 434 px track
    /// area under the 64 px toolbar: they share 434 − 24 − 1 = 409 px, floor(409 / 8) − 1 = 50
    /// px lanes, 8 rows whole (8 · 51 − 1 = 407 px; 9 take 458) and the "+ Track" row right
    /// under them, y 408..432. (They were 47 px, floor(434 / 9) − 1, while that row took a
    /// lane's share; 64 px, 6 rows, while 64 was the fitted lanes' floor; 46 px with the bar in
    /// the header, and before that the 32 px every playlist opened with.)
    fn p4_grid() -> crate::ui::playlist_layout::Grid {
        use crate::ui::playlist_layout::{fitted_lane, tracks_size_alone};
        let area = tracks_size_alone(iced::Size::new(1000.0, 560.0), layout::TOOLBAR_HEIGHT);
        let grid = crate::ui::playlist_layout::Grid {
            scroll: dawsome_engine::time::Ticks(0),
            zoom: crate::ui::playlist_layout::ZOOM.default,
            bar: dawsome_engine::time::Ticks(384),
            top: 0,
            lane: fitted_lane(8, area.height),
            area: area.height,
        };
        assert_eq!((area.height, grid.lane, grid.rows()), (434.0, 50.0, 8));
        grid
    }

    // p4_two_clips with the playlist alone and two changes of signature (--signature-change
    // 768:3/4 --signature-change 1344:6/8), Graphite: at 64 px a 4/4 bar a tick is 1/6 px, so
    // the changes' flags stand at x 120 + 768 / 6 = 248 and 120 + 1344 / 6 = 344 in the ruler's
    // number band (window y 88..104 under the 64 px toolbar and the 24 px header): each pole
    // (2 px from the bar line) in the accent's dimmer tone, Graphite's slider fill #8A5A24, the
    // flag's glass right of it the readouts' #141619; none at bar 2's line (x 184). The bars
    // after them are the meter map's (gen_p35_expected.py's rule): 3/4 bars of 288 ticks from 768
    // put bar 4's line at 120 + 1056 / 6 = 296 (#484D55, the lane's bar line), where 4/4 would put
    // none; bar 3 (768..1056, x 248..296) is shaded #393D44 (x 270, between its beat lines at 264
    // and 280), bar 4 (1056..1344) is not: x 304 (tick 1104, no beat) is lane ground #34383F,
    // where 4/4's bar 3 (768..1152) would shade it; 6/8 bars of 288 from 1344 put bar 5's strong
    // line (bars 1, 5, 9) at x 344 (#626873) and bar 6's at 120 + 1632 / 6 = 392, where 4/4 has
    // none.
    #[test]
    #[ignore = "needs a screenshot of p4_two_clips with two signature changes: scripts/ui-check.sh takes one and runs this"]
    fn p4_with_two_changes_flags_the_ruler_and_counts_their_bars() {
        let shot = read_ui_shot();
        let grid = p4_grid();
        let x = |tick: i64| grid.x_of(dawsome_engine::time::Ticks(tick));
        assert_eq!(
            (x(768), x(1056), x(1344), x(1632)),
            (248.0, 296.0, 344.0, 392.0)
        );
        let ruler = |x: f32, y: f32| ruler_in_window(Point::new(x, y));
        let tracks = |x: f32| tracks_in_window(Point::new(x, grid.row_top(1) + 40.0));
        let pole = [0x8A, 0x5A, 0x24];
        let glass = [0x14, 0x16, 0x19];
        let points: Vec<(String, Point, [u8; 3])> = vec![
            ("the 3/4 flag's pole".into(), ruler(x(768) + 1.0, 2.0), pole),
            (
                "the 3/4 flag's glass".into(),
                ruler(x(768) + 3.5, 1.0),
                glass,
            ),
            (
                "the 6/8 flag's pole".into(),
                ruler(x(1344) + 1.0, 2.0),
                pole,
            ),
            (
                "the 6/8 flag's glass".into(),
                ruler(x(1344) + 3.5, 1.0),
                glass,
            ),
            (
                "bar 4's line, 3/4".into(),
                tracks(x(1056) + 0.5),
                [0x48, 0x4D, 0x55],
            ),
            ("bar 4 unshaded".into(), tracks(304.5), [0x34, 0x38, 0x3F]),
            ("bar 3 shaded".into(), tracks(270.0), [0x39, 0x3D, 0x44]),
            (
                "bar 5's strong line, 6/8".into(),
                tracks(x(1344) + 0.5),
                [0x62, 0x68, 0x73],
            ),
            (
                "bar 6's line, 6/8".into(),
                tracks(x(1632) + 0.5),
                [0x48, 0x4D, 0x55],
            ),
        ];
        let wrong = wrong_colors(&shot, &points);
        assert!(wrong.is_empty(), "wrong color at {wrong:?}");
        let bare = shot.pixel(ruler(x(384) + 1.0, 2.0));
        assert!(
            !near(bare, pole, TOLERANCE),
            "no flag at bar 2's line: {bare:?}"
        );
    }

    // p4_two_clips with the playlist alone (colors and window points as gen_p4_expected.py prints
    // them): clip bodies #5B7FA6; the lanes at 64 px a bar, every other bar from bar 1 shaded
    // #393D44 and the others #34383F; bar lines #484D55, but the ones starting bars 1, 5, 9 and
    // 13 #626873 with a companion right of each, #626873 at 45 % over the shaded lane (#4B5059);
    // the lane's color 24 px right of each line; and a bar number 3..15 px right of each bar
    // line, bright (every channel above 0xB4) at bars 1, 5, 9 and 13, in the secondary text at the
    // others (#9FA3A9: drawn, none bright: its brightest channel is 0xA9), none mid-bar. (Before the grid's shaded bars every lane was
    // #34383F, every bar line #484D55 and every number bright.)
    #[test]
    #[ignore = "needs a screenshot of p4_two_clips with --panels playlist: scripts/ui-check.sh takes one and runs this"]
    fn p4_screenshot_shows_the_playlist() {
        use dawsome_engine::time::Ticks;
        let shot = read_ui_shot();
        let grid = p4_grid();
        let bar_x = |bar: i64| grid.x_of(Ticks(bar * 384));
        let lane_center = |row: usize, offset: f32| grid.row_top(row) + offset;
        let tracks = |x: f32, y: f32| tracks_in_window(Point::new(x, y));
        let body = [0x5B, 0x7F, 0xA6];
        let lane = [0x34, 0x38, 0x3F];
        let shaded = [0x39, 0x3D, 0x44];
        let ground = |bar: i64| if bar % 2 == 0 { shaded } else { lane };
        let bar_line = [0x48, 0x4D, 0x55];
        let strong_line = [0x62, 0x68, 0x73];
        let companion = [0x4B, 0x50, 0x59];
        let mut points: Vec<(String, Point, [u8; 3])> = vec![
            (
                "clip 1 body".into(),
                tracks(bar_x(0) + 24.0, lane_center(0, 22.0)),
                body,
            ),
            (
                "clip 2 body".into(),
                tracks(bar_x(2) + 24.0, lane_center(0, 22.0)),
                body,
            ),
            (
                "bar 2 lane".into(),
                tracks(bar_x(1) + 24.0, lane_center(0, 22.0)),
                lane,
            ),
            (
                "track 2 lane, bar 1 shaded".into(),
                tracks(bar_x(0) + 24.0, lane_center(1, 22.0)),
                shaded,
            ),
        ];
        // The design's window points, written out: the layout must put them there. (As
        // gen_p4_expected.py prints them: under the 64 px toolbar, in 50 px lanes a row every 51
        // px, track 2's lane from y 116 + 51 = 167, read 22 px in, 189 (it was y 203 in 64 px
        // lanes while 64 was the fitted lanes' floor, y 186 in 47 px lanes a row every 48, y 194
        // with the bar in the header, y 130 and 163 under the 56 px one in 32 px lanes, and y 146
        // and 193 under the 72 px one in 46 px lanes.)
        assert_eq!(grid.lane, 50.0);
        let spec_points = [
            (144.0, 138.0),
            (272.0, 138.0),
            (208.0, 138.0),
            (144.0, 189.0),
        ];
        for ((name, at, _), (x, y)) in points.iter().zip(spec_points) {
            assert_eq!((at.x, at.y), (x, y), "{name}: layout moved the probe point");
        }
        // The bar lines are read across the last whole row, track 8's (7 · 51 = 357 .. 407 in the
        // tracks, 25 px in, 382, window 116 + 382 = 498). (Pins moved: y 475 in its 47 px row,
        // and track 6's lane, y 473, while 64 px lanes showed 6 rows.)
        let lane_8 = lane_center(7, (grid.lane / 2.0).floor());
        assert!(grid.shows(7) && !grid.shows(8));
        for bar in 1..=13 {
            let line = tracks(bar_x(bar), lane_8);
            assert_eq!((line.x, line.y), (120.0 + 64.0 * bar as f32, 498.0));
            // Why the center of the logical pixel: the screenshot is the last frame the window
            // drew, and a compositor that is not showing the window (a locked screen) lets it
            // draw only its first, before the window reports its scale, so the lines are not on
            // whole device pixels yet. Unsnapped at a fractional scale (such as 1.7), a 1 px
            // line covers device pixels from 1.7x on and the pixel at its left edge is only
            // partly covered, so multisampling blends it with the lane. The pixel under the
            // center is covered snapped or not; `lines_land_on_whole_device_pixels` in
            // ui/playlist.rs pins the snapping itself.
            let center = Point::new(line.x + 0.5, line.y + 0.5);
            let strong = bar % 4 == 0;
            let color = if strong { strong_line } else { bar_line };
            points.push((format!("bar line {}", bar + 1), center, color));
            if strong {
                let next = Point::new(line.x + 1.5, line.y + 0.5);
                points.push((format!("bar line {}'s companion", bar + 1), next, companion));
            }
            points.push((
                format!("lane after bar line {}", bar + 1),
                tracks(line.x + 24.0, lane_8),
                ground(bar),
            ));
        }
        // Each clip's start line (#1A1C1F) at its first pixel, x 120 and 248, and the mark of
        // Pattern 1's tick-0 note (#DDE6F0) right of it, never over it: on the line it read as
        // a note outside the clip. The mark's row: one key in a 50 px lane, so the body under the
        // 10 px title less 2 px padding each side is 36 px, the mark 4 px high in its middle, y
        // 28..32 in the lane, 146 at its middle in the window (the lane's 22 is 138). (Pins
        // moved: 33 px in the 47 px lane, the mark at 144.5; 50 px in a 64 px one, at 153.)
        let mark_y = lane_center(0, 30.0);
        assert_eq!(tracks(0.0, mark_y).y, 146.0);
        for (clip, start) in [(1, bar_x(0)), (2, bar_x(2))] {
            points.push((
                format!("clip {clip} start line"),
                tracks(start + 0.5, mark_y),
                [0x1A, 0x1C, 0x1F],
            ));
            points.push((
                format!("clip {clip} first note mark"),
                tracks(start + 1.5, mark_y),
                [0xDD, 0xE6, 0xF0],
            ));
        }
        let wrong = wrong_colors(&shot, &points);
        // Bar-number boxes [x+3, x+15] x [90, 102] for x = 120 + 64b, b = 0..12, window pixels:
        // text drawn there lifts pixels well off the ruler's #1F2125 (every channel above 0x48);
        // the bright numbers' pixels (#C9CDD3) reach above 0xB4 in every channel, the secondary
        // ones' (#9FA3A9) never can. (Measured: the bright numbers' best pixels 194..200
        // in red, the secondary ones' 149..159.)
        let bright = |pixel: [u8; 3]| pixel.iter().all(|&value| value > 0xB4);
        let drawn = |pixel: [u8; 3]| pixel.iter().all(|&value| value > 0x48);
        let band =
            ruler_in_window(Point::new(0.0, 2.0)).y..ruler_in_window(Point::new(0.0, 15.0)).y;
        assert_eq!((band.start, band.end), (90.0, 103.0));
        let mut unlabeled = Vec::new();
        let mut wrong_tone = Vec::new();
        let mut mid_bar = Vec::new();
        for bar in 0..13 {
            let x = bar_x(bar);
            let label = (x + 3.0)..(x + 16.0);
            let lit = shot.count_matching(label.clone(), band.clone(), bright);
            let text = shot.count_matching(label, band.clone(), drawn);
            let middle = shot.count_matching((x + 35.0)..(x + 48.0), band.clone(), drawn);
            println!(
                "bar {} number box: {text} drawn px, {lit} bright; mid-bar box {middle}",
                bar + 1
            );
            if text == 0 {
                unlabeled.push(bar + 1);
            }
            if (lit > 0) != (bar % 4 == 0) {
                wrong_tone.push(bar + 1);
            }
            if middle > 0 {
                mid_bar.push(bar + 1);
            }
        }
        assert!(wrong.is_empty(), "wrong color at {wrong:?}");
        assert!(unlabeled.is_empty(), "no number for bars {unlabeled:?}");
        assert!(
            wrong_tone.is_empty(),
            "bright numbers are bars 1, 5, 9, 13 alone; wrong at {wrong_tone:?}"
        );
        assert!(
            mid_bar.is_empty(),
            "text pixels mid-bar in bars {mid_bar:?}"
        );
    }

    /// The loop strip's color inside the range (bar 2) and the ruler's outside it (bars 1, 3).
    fn loop_strip_points(inside: [u8; 3]) -> Vec<(String, Point, [u8; 3])> {
        use dawsome_engine::time::Ticks;
        let grid = p4_grid();
        let strip = |bar: i64| {
            let at = ruler_in_window(Point::new(grid.x_of(Ticks(bar * 384)) + 24.0, 22.0));
            assert_eq!(at.y, 110.0, "the loop strip's row");
            at
        };
        let ruler = [0x1F, 0x21, 0x25];
        vec![
            ("strip in the range, bar 2".into(), strip(1), inside),
            ("strip before the range, bar 1".into(), strip(0), ruler),
            ("strip after the range, bar 3".into(), strip(2), ruler),
        ]
    }

    // p4_loop's range (384..768, on) lights the strip #C47A2C at (208, 110) and leaves the ruler
    // #1F2125 at (144, 110) and (272, 110) (16 px lower under the 72 px toolbar than under the
    // 56 px one, then 8 px higher under the 64 px one).
    #[test]
    #[ignore = "needs a screenshot of p4_loop with --panels playlist: scripts/ui-check.sh takes one and runs this"]
    fn p4_loop_screenshot_lights_the_loop_strip() {
        let shot = read_ui_shot();
        let points = loop_strip_points([0xC4, 0x7A, 0x2C]);
        assert_eq!(
            (points[0].1.x, points[1].1.x, points[2].1.x),
            (208.0, 144.0, 272.0)
        );
        let wrong = wrong_colors(&shot, &points);
        assert!(wrong.is_empty(), "wrong color at {wrong:?}");
    }

    // p4_loop_off draws the same range dim, #5E5143 at (208, 110).
    #[test]
    #[ignore = "needs a screenshot of p4_loop_off with --panels playlist: scripts/ui-check.sh takes one and runs this"]
    fn p4_loop_off_screenshot_dims_the_loop_strip() {
        let shot = read_ui_shot();
        let wrong = wrong_colors(&shot, &loop_strip_points([0x5E, 0x51, 0x43]));
        assert!(wrong.is_empty(), "wrong color at {wrong:?}");
    }

    // p5_gain with the mixer alone (points from the mixer's layout at its default height, the
    // colors of its design): channel 1 plays into insert 3, so insert 3 is selected. Fader caps
    // #B8BEC6 at each fader's height (0.8, 0.8, 0.63496, 1.0), 3 px under the cap's center line,
    // strip body #33373D, insert 3's #F29A2E outline, route buttons lit #F29A2E where insert 3
    // sends (the master), #4A4F57 where it does not (insert 4), hidden on insert 3 itself (its
    // body #3D4148), and the stopped meters' ground #141619 on strips 0 to 4. (The
    // points were the design's window points under a 56 px toolbar on a 504 px desk; they
    // follow the layout now, under the 64 px toolbar and the mixer's handle, on its 490 px desk.)
    #[test]
    #[ignore = "needs a screenshot of p5_gain with --panels mixer: scripts/ui-check.sh takes one and runs this"]
    fn p5_screenshot_shows_the_mixer() {
        use dawsome_engine::project::InsertId;

        use crate::ui::mixer_layout::{self as mix, strip_left};

        let shot = read_ui_shot();
        // Alone, the mixer takes the window under the toolbar, and its desk the panel under
        // the handle: 560 − 64 − 6 = 490 px (taller than the 394 px of its default height,
        // which it has when another panel takes the rest).
        let desk = 560.0 - layout::TOOLBAR_HEIGHT - mix::HANDLE_HEIGHT;
        assert_eq!(desk, 490.0);
        let left = |id: u16| strip_left(InsertId(id), 0.0);
        let window = |rect: iced::Rectangle| iced::Rectangle {
            y: mix::in_window(rect.position()).y,
            ..rect
        };
        let window_y = |y: f32| mix::in_window(Point::new(0.0, y)).y;
        let cap = [0xB8, 0xBE, 0xC6];
        let body = [0x33, 0x37, 0x3D];
        let selected = [0x3D, 0x41, 0x48];
        let accent = [0xF2, 0x9A, 0x2E];
        let ground = [0x14, 0x16, 0x19];
        let cap_y = |fader: f32| window_y(mix::cap_center_y(fader, desk)) - 3.0;
        let mut points: Vec<(String, Point, [u8; 3], Option<iced::Rectangle>)> = vec![];
        for (id, fader) in [(0, 0.8), (1, 0.8), (2, 0.634_96), (3, 1.0)] {
            let rect = window(mix::cap_rect(left(id), fader, desk));
            points.push((
                format!("insert {id} fader cap"),
                Point::new(left(id) + 4.0, cap_y(fader)),
                cap,
                Some(rect),
            ));
        }
        let body_2 = window(mix::body_rect(left(2), desk));
        let body_3 = window(mix::body_rect(left(3), desk));
        // Half way down the fader track: inside every body and meter.
        let middle = window_y(mix::FADER_TOP + mix::fader_span(desk) * 0.65);
        let route_y = |id| window(mix::route_rect(left(id), desk)).center_y();
        points.extend([
            (
                "insert 2 body".into(),
                Point::new(left(2) + 4.0, cap_y(0.8)),
                body,
                Some(body_2),
            ),
            (
                "insert 3 outline".into(),
                Point::new(left(3) + 1.0, middle),
                accent,
                Some(body_3),
            ),
            (
                "insert 2 body edge".into(),
                Point::new(left(2) + 1.0, middle),
                body,
                Some(body_2),
            ),
            (
                "master route button, lit".into(),
                Point::new(left(0) + 30.0, route_y(0)),
                accent,
                Some(window(mix::route_rect(left(0), desk))),
            ),
            (
                "insert 4 route button".into(),
                Point::new(left(4) + 30.0, route_y(4)),
                [0x4A, 0x4F, 0x57],
                Some(window(mix::route_rect(left(4), desk))),
            ),
            (
                "insert 3, no route button".into(),
                Point::new(left(3) + 30.0, route_y(3)),
                selected,
                Some(window(mix::route_rect(left(3), desk))),
            ),
        ]);
        for id in 0..=4 {
            let x = left(id) + 39.0;
            let meter = window(mix::meter_rect(left(id), 0, desk));
            points.push((
                format!("strip {id} meter"),
                Point::new(x, middle),
                ground,
                Some(meter),
            ));
        }
        for (name, at, _, rect) in &points {
            let rect = rect.expect("every point names its element");
            assert!(
                rect.contains(*at),
                "{name}: layout moved {rect:?} off {at:?}"
            );
        }
        let colors: Vec<(String, Point, [u8; 3])> = points
            .into_iter()
            .map(|(name, at, color, _)| (name, at, color))
            .collect();
        let wrong = wrong_colors(&shot, &colors);
        assert!(wrong.is_empty(), "wrong color at {wrong:?}");
    }

    // p5_delay_sync with the mixer alone at 1000 x 560 (insert 1 selected, its desk 560 − 64 − 6
    // = 490 px): the effects area starts at 1000 − 332 = 668 (sixteen inserts end at 1068), its
    // view at (674, 28). Insert 1's delay, four knobs, is the first block, 252 x 84 at (674, 28):
    // its name box (698 to 2 px short of its ‹ target, 674 + 252 − 3 × 24 − 2 = 852; under the
    // 64 px toolbar and the 6 px handle window y 98..120) has bright text (every channel above
    // 0xB4), and its light at (674 + 12, 28 + 11), window (686, 109), is lit #6CC24A: the fixture
    // says nothing of it, so it is on. The add tile, 192 x 84, does not fit beside it (674 + 252
    // + 6 + 192 = 1124 > 986) and starts the next line at (674, 28 + 84 + 6 = 118): it is drawn
    // (pixels other than the area's ground) and its words are in the secondary text #9FA3A9 (no
    // bright pixel: its brightest channel is 0xA9; measured, the name's best pixel (200,
    // 204, 210), the tile's (159, 162, 168)), window box [680, 860] × [192, 268]. (Pins moved:
    // with the blocks at 52, 24 px lower; with the head's 16 and 14 px targets the name to 874.)
    #[test]
    #[ignore = "needs a screenshot of p5_delay_sync with --panels mixer: scripts/ui-check.sh takes one and runs this"]
    fn p5_delay_screenshot_shows_the_delays_block_lit() {
        use dawsome_engine::project::InsertId;

        use crate::ui::mixer_layout as mix;

        let shot = read_ui_shot();
        let width = shot.width as f32 / shot.scale;
        let desk = mix::Desk {
            size: iced::Size::new(width, 560.0 - layout::TOOLBAR_HEIGHT - mix::HANDLE_HEIGHT),
            scroll: 0.0,
            last: InsertId(16),
            effects_scroll: 0.0,
        };
        let rects = mix::chain_rects(&[4], desk);
        let [delay, tile] = rects[..] else {
            panic!("the delay and the add tile: {rects:?}");
        };
        let window = |rect: iced::Rectangle| iced::Rectangle {
            y: mix::in_window(rect.position()).y,
            ..rect
        };
        let name = window(mix::effect_name_rect(delay));
        let light = mix::in_window(mix::effect_led_center(delay));
        let tile = window(tile);
        assert_eq!(
            (name.x, name.x + name.width, name.y, name.y + name.height),
            (698.0, 852.0, 98.0, 120.0)
        );
        assert_eq!(light, Point::new(686.0, 109.0));
        assert_eq!((tile.x, tile.y), (674.0, 188.0));
        let bright = |pixel: [u8; 3]| pixel.iter().all(|&value| value > 0xB4);
        let named = shot.count_matching(
            name.x..name.x + name.width,
            name.y..name.y + name.height,
            bright,
        );
        let inside = (tile.x + 6.0)..(tile.x + tile.width - 6.0);
        let rows = (tile.y + 4.0)..(tile.y + tile.height - 4.0);
        let ground = rgb8(crate::ui::style::palette().panel);
        let drawn = shot.count_matching(inside.clone(), rows.clone(), |pixel| {
            !near(pixel, ground, TOLERANCE)
        });
        let loud = shot.count_matching(inside, rows, bright);
        println!("name bright px {named}, tile drawn px {drawn}, tile bright px {loud}");
        assert!(named > 0, "the delay's block names it");
        let lit = vec![(
            "the delay's light, on".to_owned(),
            light,
            rgb8(crate::ui::style::TRACK_LED_ON),
        )];
        let wrong = wrong_colors(&shot, &lit);
        assert!(wrong.is_empty(), "wrong color at {wrong:?}");
        assert!(drawn > 0, "the add tile is drawn");
        assert_eq!(loud, 0, "the add tile's words are in the secondary text");
    }

    // p5_gain with the mixer alone at 1000 px: the effects area starts at 1000 − 332 = 668, so
    // strip 10 (x 632..692) runs under it, and its name must stop at the area's edge. Over the
    // name rows (window y 94..114 under the 64 px toolbar and the mixer's handle: from the
    // header's foot, mixer y 24, past the compact strip's name row 28..44) the area's margin
    // left of its blocks (668..674, the blocks 6 px in) holds only its ground: no strip's name
    // run in; and between the header's foot and the blocks' first line (window y 94..98, mixer
    // 24..28) the whole area to its marks' margin (668..986) does: no title, the selected
    // insert being named by the header's routing control alone. (The title, "Insert 3", stood
    // from x 674 before it went, and the probe looked left of it only. Strip 12 crossed the
    // slot panel's edge at 800 before the effects area took the strips' room. Pins moved: with
    // the compact strip the name row was 32..48, to 118; with the blocks at 52 the whole area
    // held only its ground over the name rows, until the blocks moved up to 28.)
    #[test]
    #[ignore = "needs a screenshot of p5_gain with --panels mixer: scripts/ui-check.sh takes one and runs this"]
    fn p5_strip_names_stop_at_the_effects_area() {
        use dawsome_engine::project::InsertId;

        use crate::ui::mixer_layout as mix;

        let shot = read_ui_shot();
        let width = shot.width as f32 / shot.scale;
        let edge = mix::effects_left(width, InsertId(16));
        let strip = mix::strip_left(InsertId(10), 0.0);
        assert_eq!((width, edge), (1000.0, 668.0));
        assert!(
            strip < edge && strip + mix::STRIP_WIDTH > edge,
            "strip 10 crosses the edge"
        );
        let marks = width - mix::MARKS_WIDTH;
        assert_eq!(marks, 986.0);
        let blocks = edge + mix::EFFECTS_INSET;
        assert_eq!(blocks, 674.0);
        let rows = mix::in_window(Point::new(0.0, mix::HEADER_HEIGHT)).y
            ..mix::in_window(Point::new(0.0, mix::NAME_TOP + mix::NAME_HEIGHT)).y;
        assert_eq!((rows.start, rows.end), (94.0, 114.0));
        let band = rows.start..mix::in_window(Point::new(0.0, mix::BLOCKS_TOP)).y;
        assert_eq!((band.start, band.end), (94.0, 98.0));
        let ground = rgb8(crate::ui::style::palette().panel);
        let not_ground = |pixel| !near(pixel, ground, TOLERANCE);
        let name = shot.count_matching(edge..blocks, rows, not_ground);
        let title = shot.count_matching(edge..marks, band, not_ground);
        println!(
            "pixels other than the area's ground: by its edge {name}, over its blocks {title}"
        );
        assert_eq!(name, 0, "a strip's name in the effects area");
        assert_eq!(title, 0, "a title over the effects' blocks");
    }

    // p8_notice at 1000 px with no audio device: two missing samples with long names and the
    // audio being off make notices far longer than the toolbar's room. They show on one line:
    // the notices' color is on the line through the toolbar's middle (10 px either side of it,
    // window y 22..42) and nowhere above or below it in the toolbar (y 1..18, 46..63: from 14 px
    // off the middle to a pixel from the bar's edges). (Pins moved with the 72 px toolbar's
    // middle, 36: they were 18..38, 1..14 and 42..55 in 56 px, then 26..46, 1..22 and 50..71;
    // from the toolbar's height now.)
    #[test]
    #[ignore = "needs a screenshot of p8_notice with no audio device: scripts/ui-check.sh takes one and runs this"]
    fn p8_notices_stay_on_one_line_in_the_toolbar() {
        let shot = read_ui_shot();
        let width = shot.width as f32 / shot.scale;
        let warning = rgb8(crate::ui::style::WARNING);
        let count = |rows: std::ops::Range<f32>| {
            shot.count_matching(0.0..width, rows, |pixel| near(pixel, warning, 12))
        };
        let middle = layout::TOOLBAR_HEIGHT / 2.0;
        let (above, line, below) = (
            count(1.0..middle - 14.0),
            count(middle - 10.0..middle + 10.0),
            count(middle + 14.0..layout::TOOLBAR_HEIGHT - 1.0),
        );
        println!("notice pixels above {above}, on the line {line}, below {below}");
        assert!(line > 0, "a notice shows on the toolbar's line");
        assert_eq!((above, below), (0, 0), "a notice wrapped out of the line");
    }

    // p8_notice with no audio device at 1000, 960, 930, 900 and 880 px: audio is off, so
    // Retry shows. Along y 12 (pin moved from y 20: the buttons are 40 px tall from y 8
    // now, their 15 px text from about y 21; then 52 px tall from y 10, their 16 px
    // text from about y 26; then 46 px tall from y 9, their 14 px text from about y 23), above
    // the buttons' text, Retry and File are each
    // one box at the
    // place and width the toolbar gives them when its contents fit (Help against the right
    // padding, Edit the menus' spacing before it and File the spacing before Edit, Retry a line
    // before File), with the toolbar's
    // ground on both sides; a
    // notice, whole or cut with an ellipsis, shows left of Retry, and none of it reaches Retry
    // or File.
    #[test]
    #[ignore = "needs screenshots of p8_notice with no audio device: scripts/ui-check.sh takes them and runs this"]
    fn p8_toolbar_keeps_retry_and_file_whole_beside_a_notice() {
        let shot = read_ui_shot();
        let width = shot.width as f32 / shot.scale;
        let spread = crate::ui::toolbar::shot_spread(width, true);
        let [retry, _, file, _] = crate::ui::toolbar::retry_edit_file_and_help(&spread);
        let ground = rgb8(crate::ui::style::palette().panel);
        let boxes = shot.runs_along(12.0, |pixel| !near(pixel, ground, TOLERANCE));
        let last: Vec<_> = boxes.iter().rev().take(2).rev().collect();
        println!("Retry {retry:?}, File {file:?}; the last boxes along y 12: {last:?}");
        let fits = |expected: &std::ops::Range<f32>| {
            boxes.iter().any(|found| {
                (found.start - expected.start).abs() <= 1.0
                    && (found.end - expected.end).abs() <= 1.0
            })
        };
        assert!(fits(&file), "File is not whole against the right padding");
        assert!(fits(&retry), "Retry is not whole before File");
        let warning = rgb8(crate::ui::style::WARNING);
        let count = |x: std::ops::Range<f32>| {
            let middle = layout::TOOLBAR_HEIGHT / 2.0;
            shot.count_matching(x, middle - 10.0..middle + 10.0, |pixel| {
                near(pixel, warning, 12)
            })
        };
        let (notice, over) = (count(0.0..retry.start), count(retry.start..width));
        println!("notice pixels left of Retry {notice}, from Retry on {over}");
        assert!(notice > 0, "no notice shows beside Retry");
        assert_eq!(over, 0, "a notice reaches Retry or File");
    }

    /// An iced color as the 8-bit RGB a screenshot holds.
    pub(super) fn rgb8(color: iced::Color) -> [u8; 3] {
        [color.r, color.g, color.b].map(|channel| (channel * 255.0).round() as u8)
    }

    // The picker opened on p7_catalogue.json's instruments in the default 1000 by 560 window. Its
    // list (positions from crate::ui::picker) shows the three instruments as three bands
    // alternating between the two row tones, then the failed section, whose text is dimmer than a
    // plugin row's.
    #[test]
    #[ignore = "needs a screenshot of the picker: scripts/ui-check.sh takes one and runs this"]
    fn p7_picker_screenshot_lists_three_instruments_and_dims_the_failed_file() {
        use crate::ui::picker::{list_origin, ROW_HEIGHT};
        let shot = read_ui_shot();
        let origin = list_origin(iced::Size::new(1000.0, 560.0));
        let tones = crate::ui::style::palette().row_tones.map(rgb8);
        let row_point = |row: usize, x: f32| {
            Point::new(origin.x + x, origin.y + (row as f32 + 0.5) * ROW_HEIGHT)
        };
        let mut bands = 0;
        for row in 0..8 {
            let pixel = shot.pixel(row_point(row, 5.0));
            println!("row {row} at x + 5: {pixel:02X?}");
            if !near(pixel, tones[row % 2], TOLERANCE) {
                break;
            }
            bands += 1;
        }
        assert_eq!(bands, 3, "one band per instrument, alternating tones");
        // The brightest pixel of a row's text: plugin rows are drawn in the normal text color,
        // the failed file (row 4, after the heading) in the dim one.
        let brightest = |row: usize| -> u32 {
            let top = origin.y + row as f32 * ROW_HEIGHT;
            (0..240)
                .flat_map(|x| (4..22).map(move |y| (x, y)))
                .map(|(x, y)| shot.pixel(Point::new(origin.x + 10.0 + x as f32, top + y as f32)))
                .map(|pixel| pixel.iter().map(|&channel| u32::from(channel)).sum())
                .max()
                .unwrap_or(0)
        };
        let (plugin, failed) = (brightest(0), brightest(4));
        println!("brightest text: plugin row {plugin}, failed file {failed}");
        assert!(failed < plugin, "the failed file's text is dimmer");
        let well: u32 = rgb8(crate::ui::style::palette().bg)
            .iter()
            .map(|&channel| u32::from(channel))
            .sum();
        assert!(failed > well, "its text is drawn");
    }

    // The picker opened on p27_catalogue.json's instruments (CLAP and VST3) in the default 1000 by
    // 560 window: its format column (positions from crate::ui::picker) names each row's format.
    // Rows 0 and 2 (Dawsome Test Fail and Tick, VST3 alone) have the same text there, pixel for
    // pixel where it is drawn, as wide as "VST3" measures; row 1 (Dawsome Test Sine, the CLAP
    // build of a plugin installed in both) has other text, as wide as "CLAP" measures (the widths
    // held to 1 px). Changed with the bundled font: in Noto Sans the two words were 1.5 px
    // apart at 12 px, so the widths told them apart too, and the probe held that; in Inter they
    // are 0.36 px apart (31.14 and 31.49), so only the pixels tell them apart, which the probe
    // has always held (the CLAP row's text other than the VST3 rows', theirs alike).
    #[test]
    #[ignore = "needs a screenshot of the picker over both formats: scripts/ui-check.sh takes one and runs this"]
    fn p27_picker_screenshot_names_each_rows_format() {
        use crate::ui::fonts::size;
        use crate::ui::picker::{list_origin, FORMAT_X, KIND_X, ROW_HEIGHT};
        use crate::ui::text_fit::text_width;
        let shot = read_ui_shot();
        let origin = list_origin(iced::Size::new(1000.0, 560.0));
        let step = 1.0 / shot.scale;
        // Where text is drawn in the format column of `row` (the pixels of its middle 12 px that
        // differ from the row's own band, by their place in the column), and how wide it is.
        let column = |row: usize| -> (Vec<(usize, i32)>, f32) {
            let middle = origin.y + (row as f32 + 0.5) * ROW_HEIGHT;
            let band = shot.pixel(Point::new(origin.x + 5.0, middle));
            let mut drawn = Vec::new();
            let mut column = 0;
            let mut x = origin.x + FORMAT_X - 4.0;
            while x < origin.x + KIND_X - 4.0 {
                for dy in -6..6 {
                    if !near(
                        shot.pixel(Point::new(x, middle + dy as f32)),
                        band,
                        TOLERANCE,
                    ) {
                        drawn.push((column, dy));
                    }
                }
                column += 1;
                x += step;
            }
            let width = match (drawn.first(), drawn.last()) {
                (Some(first), Some(last)) => (last.0 - first.0 + 1) as f32 * step,
                _ => 0.0,
            };
            (drawn, width)
        };
        let (vst3, clap) = (
            text_width("VST3", size::LABEL),
            text_width("CLAP", size::LABEL),
        );
        let ((fail, fail_width), (sine, sine_width), (tick, tick_width)) =
            (column(0), column(1), column(2));
        println!(
            "format column widths: Fail {fail_width}, Sine {sine_width}, Tick {tick_width}; \
             VST3 measures {vst3}, CLAP {clap}"
        );
        assert!(
            !fail.is_empty() && fail == tick,
            "the two VST3 rows' text alike"
        );
        assert!(sine != fail, "the CLAP row's text is another");
        for (name, width, want) in [
            ("Fail", fail_width, vst3),
            ("Sine", sine_width, clap),
            ("Tick", tick_width, vst3),
        ] {
            assert!(
                (width - want).abs() <= 1.0,
                "{name}: {width} wide, not {want}"
            );
        }
    }

    // p8_ui with the Library and the playlist (window points and colors of the Library's and the
    // clips' design): the Library's ground and search well; the Samples tree's rows 0-2
    // ("browser", "drums", "tone.wav", under the Instruments section) hold bright text (every
    // channel above 0x90) and the row after them is the MIDI devices header (notes.txt is not a
    // sound file); the saw clip's title strip, waveform, body (below the center line, the file
    // being positive only, and past the file's end at tick 130) and center line; the automation
    // clip's strip, body and its line at y 170 halfway down the 1 → 0 ramp. Playlist points are
    // given from the playlist's left edge, right of the Library. The run's settings keep the
    // lanes at 32 px (a height chosen; they fit the panel otherwise). (Pins moved
    // 16 px down under the 72 px toolbar, then 8 px up under the 64 px one.)
    #[test]
    #[ignore = "needs a screenshot of p8_ui with --panels browser,playlist: scripts/ui-check.sh takes one and runs this"]
    fn p8_screenshot_shows_the_browser_an_audio_clip_and_an_automation_line() {
        use crate::ui::browser;
        use crate::ui::library::{Contents, Item, Library};
        use dawsome_engine::time::Ticks;
        let shot = read_ui_shot();
        // The playlist to the Library's right: its tracks canvas starts 120 px into it, under
        // the toolbar, the playlist's header strip and the ruler.
        let grid = crate::ui::playlist_layout::Grid {
            lane: crate::ui::playlist_layout::LANE_HEIGHT,
            ..p4_grid()
        };
        let tracks = |point: Point| {
            let window = tracks_in_window(point);
            Point::new(window.x + browser::WIDTH, window.y)
        };
        let playlist = |x: f32, y: f32| Point::new(browser::WIDTH + x, y);
        assert_eq!(
            tracks(Point::new(grid.x_of(Ticks(0)), 0.0)),
            playlist(120.0, 116.0)
        );
        assert_eq!(
            tracks(Point::new(grid.x_of(Ticks(384)), grid.row_top(1))).y,
            149.0
        );
        // The Library as the shot lists it: the kit, no plugin (the run's catalogue is empty),
        // the tree's three rows under its one folder, no MIDI device.
        let items = Library::default().items(&Contents {
            kit: &crate::starter_kit::names(),
            tree: 3,
            folders: 1,
            ..Contents::default()
        });
        let first = items
            .iter()
            .position(|item| *item == Item::Tree(0))
            .expect("the tree is listed");
        let points: Vec<(String, Point, [u8; 3])> = vec![
            (
                "browser ground".into(),
                Point::new(200.0, 300.0),
                [0x2B, 0x2E, 0x33],
            ),
            (
                "search well".into(),
                Point::new(200.0, 100.0),
                [0x1F, 0x21, 0x25],
            ),
            (
                "the header after the tree".into(),
                Point::new(browser::WIDTH - 20.0, library_row(first + 3).center_y()),
                [0x3A, 0x3E, 0x45],
            ),
            (
                "audio title strip".into(),
                playlist(180.0, 121.0),
                [0x3E, 0x6B, 0x57],
            ),
            (
                "waveform".into(),
                playlist(130.0, 132.0),
                [0x9F, 0xD8, 0xBE],
            ),
            (
                "audio body under the center".into(),
                playlist(130.0, 144.0),
                [0x23, 0x33, 0x2C],
            ),
            (
                "audio body past the file".into(),
                playlist(200.0, 132.0),
                [0x23, 0x33, 0x2C],
            ),
            (
                "center line".into(),
                playlist(200.0, 137.0),
                [0x3E, 0x5E, 0x52],
            ),
            // Why y 158 and not 154: at (152, 154) the strip's title "click · Volume" draws the
            // "o" of "Volume"; the strip's last row is clear of the text.
            (
                "automation title strip".into(),
                playlist(152.0, 158.0),
                [0x6B, 0x4A, 0x7A],
            ),
            (
                "automation body".into(),
                playlist(160.0, 164.0),
                [0x2E, 0x25, 0x33],
            ),
        ];
        let wrong = wrong_colors(&shot, &points);
        let line = shot.count_near(
            browser::WIDTH + 151.0..browser::WIDTH + 154.0,
            169.0..172.0,
            [0xE0, 0xB8, 0xF0],
            24,
        );
        let bright = |pixel: [u8; 3]| pixel.iter().all(|&value| value > 0x90);
        let rows: Vec<usize> = (0..3)
            .map(|row| {
                let center = library_row(first + row).center_y();
                shot.count_matching(20.0..200.0, center - 5.0..center + 5.0, bright)
            })
            .collect();
        println!("automation line pixels {line}; bright text per row {rows:?}");
        assert!(wrong.is_empty(), "wrong color at {wrong:?}");
        assert!(
            line > 0,
            "the automation line crosses 152 px into the playlist at y 170"
        );
        assert!(rows.iter().all(|&count| count > 0), "rows 0-2 hold names");
    }

    /// Where row `row` of the Library (counted as its items are) is in the window, unscrolled:
    /// the panel sits at the window's left edge, under the toolbar.
    fn library_row(row: usize) -> iced::Rectangle {
        let rect = crate::ui::browser::row_rect(row, 0).expect("in view");
        iced::Rectangle {
            y: rect.y + layout::TOOLBAR_HEIGHT,
            ..rect
        }
    }

    /// The x range from the Library's text at `x` to the panel's right margin.
    fn library_text(x: f32) -> std::ops::Range<f32> {
        x..crate::ui::browser::WIDTH - 8.0
    }

    /// The y range of the text box of the Library's row `row`.
    fn library_text_rows(row: usize) -> std::ops::Range<f32> {
        let center = library_row(row).center_y();
        center - 6.0..center + 6.0
    }

    /// Checks the Library's section headers in `shot` for `items`: each a band across the panel
    /// (#3A3E45 right of its words), its title and count in bright text (every channel above
    /// 0x90), the count right of "<title> · ". Returns what is wrong.
    fn library_headers(shot: &Screenshot, items: &[crate::ui::library::Item]) -> Vec<String> {
        use crate::ui::browser;
        use crate::ui::library::Item;
        use crate::ui::text_fit::text_width;
        let bright = |pixel: [u8; 3]| pixel.iter().all(|&value| value > 0x90);
        let mut wrong = Vec::new();
        for (row, item) in items.iter().enumerate() {
            let Item::Header { section, count, .. } = item else {
                continue;
            };
            let rect = library_row(row);
            let band = shot.pixel(Point::new(browser::WIDTH - 20.0, rect.center_y()));
            let title = format!("{} \u{b7} ", section.title());
            let words = browser::ROW_LEFT + 12.0;
            let count_x = words + text_width(&title, crate::ui::fonts::size::LABEL);
            let count_end =
                words + text_width(&format!("{title}{count}"), crate::ui::fonts::size::LABEL) + 1.0;
            let ys = library_text_rows(row);
            let title_px = shot.count_matching(words..count_x - 6.0, ys.clone(), bright);
            let count_px = shot.count_matching(count_x..count_end, ys, bright);
            println!(
                "{} header at y {}: band {band:02X?}, title px {title_px}, count px {count_px}",
                section.title(),
                rect.y
            );
            if !near(band, [0x3A, 0x3E, 0x45], TOLERANCE) || title_px == 0 || count_px == 0 {
                wrong.push(section.title().to_owned());
            }
        }
        wrong
    }

    /// Whether `pixel` is text at all (some channel at 0x50 or more on the panel's ground), and
    /// bright text: every channel above 0xB4, which the text (#C9CDD3) reaches and neither the
    /// secondary text (#9FA3A9, its brightest channel 0xA9) nor the disabled (#6E747C) can.
    /// (Some channel at 0x80 or more, before the present secondary text, which reaches it.)
    fn drawn(pixel: [u8; 3]) -> bool {
        pixel.iter().any(|&value| value >= 0x50)
    }

    fn not_dim(pixel: [u8; 3]) -> bool {
        pixel.iter().all(|&value| value > 0xB4)
    }

    // p10_library with the Library alone at 1000 by 908 (--cache p7_catalogue.json, --midi-ports
    // "Port A,Port B", a settings file playing Port A; 44 px over the 800 it was, for the external
    // input's two rows, then 64 more for the Library's 24 px rows (were 22), so every row still
    // shows: 64 + 52 + 32 × 24 = 884, a row to spare): the three section headers sit at the rows
    // the Library model gives, each with its count; Port A's tick is lit (#F29A2E) and its row
    // ends with "→ kick" in the accent, Port B's tick is off (#4A4F57) with no link, and Port C,
    // which "keys" links to but is not plugged in, has an empty tick and only dim text.
    #[test]
    #[ignore = "needs a screenshot of p10_library with --panels browser: scripts/ui-check.sh takes one and runs this"]
    fn p10_library_screenshot_shows_the_sections_counts_and_device_links() {
        use std::collections::BTreeSet;

        use crate::settings::MidiInputs;
        use crate::ui::browser::{self, tick_rect, PORT_NAME_X};
        use crate::ui::library::{self, Contents, Item, Library};
        use crate::ui::text_fit::text_width;
        use dawsome_host_clap::catalogue::PluginKind;
        let shot = read_ui_shot();
        let catalogue = dawsome_host_clap::Catalogue::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/p7_catalogue.json"),
        )
        .expect("the catalogue loads");
        let listed = ["Port A".to_owned(), "Port B".to_owned()];
        let inputs = MidiInputs::Named(BTreeSet::from(["Port A".to_owned()]));
        let links = [
            ("Port A".to_owned(), "kick".to_owned()),
            ("Port C".to_owned(), "keys".to_owned()),
        ];
        let items = Library::default().items(&Contents {
            kit: &crate::starter_kit::names(),
            plugins: catalogue.offered(PluginKind::Instrument),
            tree: 3,
            folders: 1,
            ports: library::ports(&listed, &inputs, &links),
            ..Contents::default()
        });
        let row_of = |name: &str| {
            items
                .iter()
                .position(|item| matches!(item, Item::Port(port) if port.name == name))
                .expect("listed")
        };
        let (a, b, c) = (row_of("Port A"), row_of("Port B"), row_of("Port C"));
        assert_eq!(
            library_row(c).y + library_row(c).height,
            layout::TOOLBAR_HEIGHT + 52.0 + 24.0 * items.len() as f32,
            "every row in a 908 px window"
        );
        let mut wrong = library_headers(&shot, &items);
        let tick = |row| tick_rect(library_row(row)).center();
        wrong.extend(wrong_colors(
            &shot,
            &[
                ("Port A's tick".to_owned(), tick(a), [0xF2, 0x9A, 0x2E]),
                ("Port B's tick".to_owned(), tick(b), [0x4A, 0x4F, 0x57]),
                (
                    "Port C's empty tick".to_owned(),
                    tick(c),
                    [0x2B, 0x2E, 0x33],
                ),
            ],
        ));
        let link_x =
            browser::WIDTH - 8.0 - text_width("\u{2192} kick", crate::ui::fonts::size::LABEL);
        let accent = |row| {
            let xs = link_x - 2.0..browser::WIDTH - 6.0;
            shot.count_near(xs, library_text_rows(row), [0xF2, 0x9A, 0x2E], 40)
        };
        let (link_a, link_b, link_c) = (accent(a), accent(b), accent(c));
        let c_text = shot.count_matching(library_text(PORT_NAME_X), library_text_rows(c), drawn);
        let c_bright =
            shot.count_matching(library_text(PORT_NAME_X), library_text_rows(c), not_dim);
        println!(
            "accent px right of Port A {link_a}, B {link_b}, C {link_c}; Port C text px \
             {c_text}, of them bright {c_bright}"
        );
        assert!(wrong.is_empty(), "wrong at {wrong:?}");
        assert!(link_a > 0, "Port A says it plays kick");
        assert_eq!((link_b, link_c), (0, 0), "no link on B, a dim one on C");
        assert!(c_text > 0 && c_bright == 0, "Port C is written, dim");
    }

    // A new document with the Library alone at 1000 by 860 (800 before the Library's 24 px rows:
    // its 30 rows end at 64 + 52 + 30 × 24 = 836, a row to spare), no plugin catalogue and --midi-ports
    // none: the plugin instruments' and the MIDI devices' empty states are written on their rows,
    // in the secondary text (no bright pixel anywhere in their boxes), Rescan is a button (#3A3E45 inside
    // it), and the section headers sit at their rows with their counts.
    #[test]
    #[ignore = "needs a screenshot of the Library with nothing installed: scripts/ui-check.sh takes one and runs this"]
    fn p10_library_screenshot_hints_what_is_empty() {
        use crate::ui::browser::{INDENT, ROW_LEFT};
        use crate::ui::library::{Contents, Item, Library};
        let shot = read_ui_shot();
        let items = Library::default().items(&Contents {
            kit: &crate::starter_kit::names(),
            tree: 3,
            folders: 1,
            ..Contents::default()
        });
        let mut wrong = library_headers(&shot, &items);
        let hints: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| matches!(item, Item::Hint(_)))
            .map(|(row, _)| row)
            .collect();
        assert_eq!(
            hints.len(),
            6,
            "four lines for the plugins (where CLAP and VST3 ones go), two for the devices"
        );
        for &row in &hints {
            let xs = library_text(ROW_LEFT + INDENT);
            let text = shot.count_matching(xs.clone(), library_text_rows(row), drawn);
            let bright = shot.count_matching(xs, library_text_rows(row), not_dim);
            println!("hint row {row}: text px {text}, bright {bright}");
            if text == 0 || bright > 0 {
                wrong.push(format!("hint row {row}"));
            }
        }
        let rescan = items
            .iter()
            .position(|item| *item == Item::Rescan)
            .expect("listed");
        let inside = Point::new(ROW_LEFT + INDENT + 3.0, library_row(rescan).y + 4.0);
        wrong.extend(wrong_colors(
            &shot,
            &[("Rescan button".to_owned(), inside, [0x3A, 0x3E, 0x45])],
        ));
        assert!(wrong.is_empty(), "wrong at {wrong:?}");
    }

    // A new document with the Library alone at 1000 by 908 (44 px over the 800 it was, for the
    // external input's two rows, then 64 more for the Library's 24 px rows, so the MIDI devices
    // header, row 32, still shows: 64 + 52 + 33 × 24 = 908) and the four test sets
    // of tests/fixtures/instruments in the data folder's instruments/: the Instruments header
    // counts the twelve kit sounds, the four sets and Dawsome Synth; "Instrument sets" is
    // written in the secondary text on its row; each set's row holds its glyph (drawn left of the name) and its
    // name in bright text.
    #[test]
    #[ignore = "needs a screenshot of the Library with the test sets shipped: scripts/ui-check.sh takes one and runs this"]
    fn p11_library_screenshot_lists_the_instrument_sets() {
        use crate::ui::browser::{INDENT, ROW_LEFT};
        use crate::ui::library::{Contents, Item, Library, SHIPPED_SETS};
        let shot = read_ui_shot();
        let sets = crate::sets::scan(
            Some(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../tests/fixtures/instruments"),
            ),
            &[],
        );
        assert_eq!(sets.shipped.len(), 4);
        let library = Library {
            sets,
            ..Library::default()
        };
        let items = library.items(&Contents {
            kit: &crate::starter_kit::names(),
            tree: 3,
            folders: 1,
            ..Contents::default()
        });
        assert!(matches!(items[0], Item::Header { count: 17, .. }));
        let mut wrong = library_headers(&shot, &items);
        let group = items
            .iter()
            .position(|item| *item == Item::Group(SHIPPED_SETS))
            .expect("listed");
        let label = shot.count_matching(library_text(ROW_LEFT), library_text_rows(group), drawn);
        let label_bright =
            shot.count_matching(library_text(ROW_LEFT), library_text_rows(group), not_dim);
        println!("'{SHIPPED_SETS}' row {group}: text px {label}, bright {label_bright}");
        if label == 0 || label_bright > 0 {
            wrong.push(SHIPPED_SETS.to_owned());
        }
        let x = ROW_LEFT + INDENT;
        for (row, item) in items.iter().enumerate() {
            let Item::Set(set) = item else {
                continue;
            };
            let glyph = shot.count_matching(x..x + 10.0, library_text_rows(row), drawn);
            let name = shot.count_matching(library_text(x + 12.0), library_text_rows(row), not_dim);
            println!(
                "{} at row {row}: glyph px {glyph}, bright name px {name}",
                set.name
            );
            if glyph == 0 || name == 0 {
                wrong.push(set.name.clone());
            }
        }
        assert!(wrong.is_empty(), "wrong at {wrong:?}");
    }

    // v2 addition: the Library alone at 1000 by 800 with the four test sets of
    // tests/fixtures/instruments in the data folder and Pad previewed (--library-preview): Pad's
    // row ends in the "playing" mark in the accent (#F29A2E, its text's warm pixels: red well
    // over green, green over blue), the other sets' rows show none there.
    #[test]
    #[ignore = "needs a screenshot of the Library previewing Pad: scripts/ui-check.sh takes one and runs this"]
    fn v2_the_library_marks_the_set_it_previews() {
        use crate::ui::browser::{PLAYING_MARK, WIDTH};
        use crate::ui::library::{Contents, Item, Library};
        use crate::ui::text_fit::text_width;
        let shot = read_ui_shot();
        let sets = crate::sets::scan(
            Some(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../tests/fixtures/instruments"),
            ),
            &[],
        );
        let library = Library {
            sets,
            ..Library::default()
        };
        let items = library.items(&Contents {
            kit: &crate::starter_kit::names(),
            tree: 3,
            folders: 1,
            ..Contents::default()
        });
        let right = WIDTH - 8.0;
        let mark =
            right - text_width(PLAYING_MARK, crate::ui::fonts::size::LABEL) - 1.0..right + 1.0;
        let warm = |[r, g, b]: [u8; 3]| r >= 0x80 && r > g.saturating_add(0x20) && g > b;
        let mut wrong = Vec::new();
        for (row, item) in items.iter().enumerate() {
            let Item::Set(set) = item else {
                continue;
            };
            let marked = shot.count_matching(mark.clone(), library_text_rows(row), warm);
            println!(
                "{} at row {row}: warm px where the mark goes {marked}",
                set.name
            );
            let wanted = set.name == "Pad";
            if (marked > 0) != wanted {
                wrong.push(set.name.clone());
            }
        }
        assert!(wrong.is_empty(), "wrong at {wrong:?}");
    }

    /// Where a part of the Settings window's General page lies in the window's shot: where the
    /// layout puts it, the page unscrolled under its header. (Settings was a
    /// panel under the main window's toolbar, everything 72 px lower.)
    fn settings_in_window(rect: iced::Rectangle) -> iced::Rectangle {
        rect
    }

    fn x_range(rect: iced::Rectangle) -> std::ops::Range<f32> {
        rect.x..rect.x + rect.width
    }

    fn y_range(rect: iced::Rectangle) -> std::ops::Range<f32> {
        rect.y..rect.y + rect.height
    }

    // The Settings window with --midi-ports none (positions from crate::ui::settings_layout,
    // colors of the page's design): the "MIDI input" title box holds bright text (#C9CDD3: every
    // channel above 0xB4); the line saying no port was found holds text, in the secondary text
    // (#9FA3A9, whose brightest channel is 0xA9: no bright pixel in its box); no checkbox is ticked (no #F29A2E pixel in the window
    // but the chosen audio device's dot, in the column of the audio sections' marks, x 12..26
    // from the output's title down, which holds it: the accent's row, whose swatch and wheel
    // show the accent by design, is on the Theme page since, and the band it held, 290..414, is
    // probed too). (Pin moved: no accent pixel anywhere, until the chosen dot took the accent.)
    #[test]
    #[ignore = "needs a screenshot of --panels settings --midi-ports none --screenshot-window settings: scripts/ui-check.sh takes one and runs this"]
    fn p9_settings_screenshot_says_no_port_was_found() {
        use crate::ui::settings_layout::Layout;
        let shot = read_ui_shot();
        let layout = Layout::default();
        let title = settings_in_window(layout.midi_title());
        let message = settings_in_window(layout.empty_message().expect("no port"));
        // Pins moved 16 px down under the 72 px toolbar (were 88 and 130), then 72 px up: the
        // Settings window's own (were 104 and 146); the message 2 px lower with the 24 px rows
        // (52 + 24; was 74).
        assert_eq!((title.y, message.y), (32.0, 76.0), "the page's rows");
        let bright = |pixel: [u8; 3]| pixel.iter().all(|&value| value > 0xB4);
        let drawn = |pixel: [u8; 3]| pixel.iter().any(|&value| value >= 0x50);
        let title_bright = shot.count_matching(x_range(title), y_range(title), bright);
        let message_drawn = shot.count_matching(x_range(message), y_range(message), drawn);
        let message_bright = shot.count_matching(x_range(message), y_range(message), bright);
        let width = shot.width as f32 / shot.scale;
        let height = shot.height as f32 / shot.scale;
        let accent = [0xF2, 0x9A, 0x2E];
        let marks = (12.0..26.0, layout.audio().title().y..height);
        let dot = shot.count_near(marks.0.clone(), marks.1.clone(), accent, TOLERANCE);
        let ticked = shot.count_near(0.0..width, 0.0..height, accent, TOLERANCE) - dot;
        println!(
            "title bright px {title_bright}; message text px {message_drawn}, of them bright \
             {message_bright}; ticked px {ticked}; the chosen device's dot px {dot}"
        );
        assert!(dot > 0, "the chosen output's dot in the accent");
        assert!(title_bright > 0, "the section is titled");
        assert!(message_drawn > 0, "the empty line is written");
        assert_eq!(message_bright, 0, "in the secondary text");
        assert_eq!(ticked, 0, "no checkbox is ticked");
    }

    // The Settings window with --midi-ports "Port A,Port B" and a settings file that plays both:
    // their checkboxes are ticked #F29A2E at their centers, and "All ports" is not (#4A4F57).
    #[test]
    #[ignore = "needs a screenshot of the Settings window with two ports given: scripts/ui-check.sh takes one and runs this"]
    fn p9_settings_screenshot_ticks_the_two_ports_that_play() {
        use crate::ui::settings_layout::Layout;
        let shot = read_ui_shot();
        let layout = Layout {
            ports: 2,
            ..Layout::default()
        };
        let center = |row: usize| settings_in_window(layout.checkbox(row)).center();
        assert_eq!(
            [center(0), center(1), center(2)],
            // Pins moved 16 px down under the 72 px toolbar (were 119, 141, 163), then 72 px up:
            // the Settings window's own (were 135, 157, 179); then the 14 px boxes 5 px into
            // 24 px rows from 52, their centres at x 12 + 7 and y 52 + 24r + 12 (were
            // (18, 63), (18, 85), (18, 107)).
            [
                Point::new(19.0, 64.0),
                Point::new(19.0, 88.0),
                Point::new(19.0, 112.0)
            ],
            "the page's rows"
        );
        let points: Vec<(String, Point, [u8; 3])> = vec![
            (
                "All ports, not ticked".into(),
                center(0),
                [0x4A, 0x4F, 0x57],
            ),
            ("Port A, ticked".into(), center(1), [0xF2, 0x9A, 0x2E]),
            ("Port B, ticked".into(), center(2), [0xF2, 0x9A, 0x2E]),
        ];
        let wrong = wrong_colors(&shot, &points);
        assert!(wrong.is_empty(), "wrong color at {wrong:?}");
    }

    // The Settings window at 720 x 560 with --midi-ports "Port A,Port B" and a settings file
    // choosing an output this machine does not list ("Probe device"), which Settings shows last,
    // marked missing, after the devices the machine lists: so the first choice, "System default",
    // is unchosen, whatever the machine lists. Positions by hand from crate::ui::settings_layout's
    // table (`the_audio_section_follows_the_display`, two ports): the output section's rows from
    // y 388, 24 px each; a mark is the 14 px box 5 px into its row from the 12 px margin, so
    // choice k's centre is (19, 400 + 24k); the dot's radius 3.5, the ring's 7, so 5 px right of
    // a centre is ring. Colors: the unchosen mark all the unticked checkbox's #4A4F57; the chosen
    // one's dot the ticked checkbox's accent #F29A2E, its ring #4A4F57. (The dot was the text's
    // grey, #CFD3D8, on the ring's #4A4F57 before: hard to see.)
    #[test]
    #[ignore = "needs a screenshot of the Settings window choosing an output not listed: scripts/ui-check.sh takes one and runs this"]
    fn p9_settings_marks_the_chosen_output_in_the_accent() {
        use crate::ui::settings_layout::Layout;
        let shot = read_ui_shot();
        let layout = Layout {
            ports: 2,
            outputs: 8,
            ..Layout::default()
        };
        let center = |index: usize| settings_in_window(layout.audio().mark(index)).center();
        assert_eq!(
            [center(0), center(1), center(2)],
            [
                Point::new(19.0, 400.0),
                Point::new(19.0, 424.0),
                Point::new(19.0, 448.0)
            ],
            "the audio section's marks"
        );
        let (accent, unlit) = ([0xF2, 0x9A, 0x2E], [0x4A, 0x4F, 0x57]);
        // The chosen one is last: the first mark down the column from choice 1 with the dot.
        let height = shot.height as f32 / shot.scale;
        let chosen = (1..8)
            .take_while(|&index| center(index).y < height)
            .find(|&index| near(shot.pixel(center(index)), accent, TOLERANCE))
            .expect("a chosen mark with the accent's dot");
        let ring = |index: usize| Point::new(center(index).x + 5.0, center(index).y);
        println!("the chosen output is choice {chosen}");
        let points: Vec<(String, Point, [u8; 3])> = vec![
            ("System default, unchosen".into(), center(0), unlit),
            ("its ring".into(), ring(0), unlit),
            ("the chosen mark's ring".into(), ring(chosen), unlit),
        ];
        let wrong = wrong_colors(&shot, &points);
        assert!(wrong.is_empty(), "wrong color at {wrong:?}");
    }

    // The Settings window, its General page tall enough to show whole (a settings file keeping
    // it 720 x 900, read and not written), with --midi-ports none: the page's last thing is
    // "Reset to defaults" (positions from crate::ui::settings_layout: at the left margin, 132 x
    // 18, x 12..144, a section's gap under the input latency's buttons, whose rows depend on the
    // audio devices this machine lists, so the probe finds the button's row down its column):
    // down x 14, the last run of the button's face (#3A3E45, Graphite's button) is 18 px tall,
    // its row's pixels at x 13 and 143 the face and at x 11 and 145 the page's ground, its words
    // bright (every channel above 0xB4), and under it, down to the window's 22 px foot line,
    // only the ground.
    #[test]
    #[ignore = "needs a screenshot of --panels settings --midi-ports none with the window 720 x 900: scripts/ui-check.sh takes one and runs this"]
    fn p9_settings_general_page_ends_with_the_reset_button() {
        use crate::ui::settings_layout::RESET_WIDTH;
        use crate::ui::style;
        let shot = read_ui_shot();
        let height = shot.height as f32 / shot.scale;
        let face = rgb8(style::palette().button);
        let ground = rgb8(style::palette().bg);
        let at = |x: f32, y: f32| shot.pixel(Point::new(x, y));
        let rows: Vec<f32> = (0..height as usize)
            .map(|y| y as f32 + 0.5)
            .filter(|&y| near(at(14.0, y), face, TOLERANCE))
            .collect();
        let last = *rows.last().expect("a button down the left margin");
        let run = rows
            .iter()
            .rev()
            .zip(0..)
            .take_while(|&(&y, step)| (y - (last - step as f32)).abs() < 0.01)
            .count();
        let top = last - run as f32 + 1.0;
        let middle = top + 9.0;
        println!("the last face down x 14: y {top}..{last} ({run} rows)");
        assert_eq!(run, 18, "18 px tall");
        assert_eq!(12.0 + RESET_WIDTH, 144.0);
        for (x, want, what) in [
            (13.0, face, "its left edge"),
            (143.0, face, "its right edge"),
            (11.0, ground, "left of it"),
            (145.0, ground, "right of it"),
        ] {
            assert!(
                near(at(x, middle), want, TOLERANCE),
                "{what}: {:02X?}",
                at(x, middle)
            );
        }
        let words = shot.count_matching(12.0..144.0, top..top + 18.0, |pixel| {
            pixel.iter().all(|&value| value > 0xB4)
        });
        assert!(words > 20, "its words: {words} px");
        let foot = height - crate::ui::layout::HINT_BAR_HEIGHT;
        let under = shot.count_matching(0.0..400.0, last + 2.0..foot, |pixel| {
            !near(pixel, ground, TOLERANCE)
        });
        assert_eq!(under, 0, "nothing under it down to the window's foot line");
    }

    // The Settings window 1400 px wide (a settings file keeping it 1400 x 700, read and not
    // written), --midi-ports "Port A,Port B": its General page in three columns
    // (crate::ui::settings_layout::Columns: (1400 − 24 − 64) / 3 = 437.333 px each, at x 12,
    // 481.333 and 950.667). Each column's first title at y 32..52 holds bright text (every channel
    // above 0xB4: "MIDI input", "Audio output", "Audio input (new input channels)"), and the 32
    // px between two columns, from the titles down to the reset's row, hold only the page's
    // ground: nothing a column draws runs into the next.
    #[test]
    #[ignore = "needs a screenshot of --panels settings with the window 1400 x 700: scripts/ui-check.sh takes one and runs this"]
    fn p9_settings_general_page_stands_in_three_columns() {
        use crate::ui::settings_layout::{Columns, Layout, Section, COLUMN_GAP};
        use crate::ui::style;
        let shot = read_ui_shot();
        let width = shot.width as f32 / shot.scale;
        let columns = Columns::of(width);
        println!("{width} px: {columns:?}");
        assert_eq!(columns.count, 3, "three columns at {width} px");
        let layout = Layout {
            ports: 2,
            width,
            ..Layout::default()
        };
        let bright = |pixel: [u8; 3]| pixel.iter().all(|&value| value > 0xB4);
        for section in [Section::Midi, Section::Output, Section::Input] {
            let title = settings_in_window(layout.title(section));
            assert_eq!(title.y, 32.0, "{section:?} heads its column");
            let lit = shot.count_matching(x_range(title), y_range(title), bright);
            println!("{section:?} title at x {}: {lit} bright px", title.x);
            assert!(lit > 20, "{section:?}'s title is written in its column");
        }
        let ground = rgb8(style::palette().bg);
        let height = shot.height as f32 / shot.scale - crate::ui::layout::HINT_BAR_HEIGHT;
        for column in 0..2 {
            let from = columns.x(column) + columns.inner;
            let gap = from..from + COLUMN_GAP;
            let drawn = shot.count_matching(gap.clone(), 32.0..height, |pixel| {
                !near(pixel, ground, TOLERANCE)
            });
            println!(
                "between columns {column} and {}: x {gap:?}, {drawn} px",
                column + 1
            );
            assert_eq!(drawn, 0, "nothing between column {column} and the next");
        }
    }

    // p9_stretch_ui with the playlist alone: two four-bar clips of the same file (x 120..376),
    // track 1's stretched, track 2's not. Track 2's title holds the file's name alone, so right of
    // where its text ends (bright: every channel above 0x90), track 1's title holds the stretch's
    // words (" · Stretch 150%", or " · stretching…" while it is made) and track 2's holds nothing.
    #[test]
    #[ignore = "needs a screenshot of p9_stretch_ui with --panels playlist: scripts/ui-check.sh takes one and runs this"]
    fn p9_stretch_screenshot_names_the_stretch_after_the_file() {
        use crate::ui::playlist_layout::TITLE_HEIGHT;
        use dawsome_engine::time::Ticks;
        let shot = read_ui_shot();
        let grid = p4_grid();
        let (left, right) = (grid.x_of(Ticks(0)), grid.x_of(Ticks(1536)));
        assert_eq!((left, right), (120.0, 376.0), "the clips' window x");
        let strip = |row: usize| {
            let top = tracks_in_window(Point::new(0.0, grid.row_top(row))).y;
            top..top + TITLE_HEIGHT
        };
        // Under the 64 px toolbar, in the 50 px lanes the 8 tracks are fitted to (`p4_grid`):
        // track 1's strip at 116, track 2's 51 px lower, 167 (pins moved: 181 in 64 px lanes
        // while 64 was the fitted lanes' floor, 116 and 164 in 47 px lanes, 124 and 172 with the
        // bar in the header, 108 and 141 in 32 px lanes under the 56 px one, and 124 and 171 in
        // 46 px lanes under the 72 px one).
        assert_eq!((strip(0).start, strip(1).start), (116.0, 167.0));
        let bright = |pixel: [u8; 3]| pixel.iter().all(|&value| value > 0x90);
        let name_end = shot
            .rightmost_matching(left..right, strip(1), bright)
            .expect("track 2's clip is titled");
        let words = (name_end + 3.0)..(right - 1.0);
        let stretched = shot.count_matching(words.clone(), strip(0), bright);
        let plain = shot.count_matching(words.clone(), strip(1), bright);
        println!(
            "track 2's name ends at x {name_end}; right of it, track 1 {stretched} bright px, \
             track 2 {plain}"
        );
        assert!(name_end + 40.0 < right, "the strip has room after the name");
        assert!(stretched > 0, "track 1 names its stretch");
        assert_eq!(plain, 0, "track 2 is not stretched");
    }
}
