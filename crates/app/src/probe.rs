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
use crate::ui::piano_layout::{LANE_HEIGHT, STRIP_HEIGHT};
use crate::ui::playlist_layout::{HEADER_HEIGHT, RULER_HEIGHT};

mod ease;
mod fixes;
mod help;
mod keys;
mod midi_out;
mod params;
mod sample_editor;

/// The first channel's lit color and the two unlit tones. Unlit steps alternate tone by beat:
/// steps 1-4 and 9-12 of a bar use the first tone, steps 5-8 and 13-16 the second.
const LIT_FIRST_CHANNEL: [u8; 3] = [0xE5, 0x53, 0x3D];
const UNLIT: [[u8; 3]; 2] = [[0x4A, 0x4F, 0x57], [0x3A, 0x3E, 0x45]];
/// Largest per-channel difference still counted as a match: a color-space round trip on the
/// GPU can move a value by one.
const TOLERANCE: u8 = 2;

/// What a probed pixel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    Lit,
    /// Unlit, in the tone with this index into [`UNLIT`].
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
/// toolbar and the strip.
pub(crate) fn roll_grid_in_window(point: Point) -> Point {
    Point::new(point.x, point.y + layout::TOOLBAR_HEIGHT + STRIP_HEIGHT)
}

/// Where lane point `point` is in a window `height` tall when the piano roll is the only
/// panel: the lane sits at the bottom.
pub(crate) fn roll_lane_in_window(point: Point, height: f32) -> Point {
    Point::new(point.x, point.y + height - LANE_HEIGHT)
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
    layout::rack_in_window(layout::step_rect(row, step).center())
}

/// The tone at the center of each of the first `steps` step buttons in `row`.
pub(crate) fn probe_row(shot: &Screenshot, row: usize, steps: u32) -> Vec<Tone> {
    (0..steps)
        .map(|step| tone(shot.pixel(step_center_in_window(row, step))))
        .collect()
}

fn tone(pixel: [u8; 3]) -> Tone {
    let near = |reference: [u8; 3]| near(pixel, reference, TOLERANCE);
    if near(LIT_FIRST_CHANNEL) {
        return Tone::Lit;
    }
    match UNLIT.into_iter().position(near) {
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

    #[test]
    fn classification_tolerates_rounding_but_not_other_colors() {
        assert_eq!(tone([0xE5, 0x53, 0x3D]), Tone::Lit);
        assert_eq!(tone([0xE4, 0x54, 0x3F]), Tone::Lit);
        assert_eq!(tone([0x4A, 0x4F, 0x57]), Tone::Unlit(0));
        assert_eq!(tone([0x3B, 0x3E, 0x44]), Tone::Unlit(1));
        assert_eq!(tone([0x42, 0x46, 0x4E]), Tone::Other([0x42, 0x46, 0x4E]));
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
                s if s % 4 == 0 => LIT_FIRST_CHANNEL,
                s => UNLIT[(s / 4 % 2) as usize],
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

    // The new document with the rack alone (positions from crate::ui::layout, colors from
    // crate::ui::style): its one row, "Sampler", lights no step, so its 16 steps show the two
    // unlit tones by beat; under it, the "+ Plugin instrument…" button. In the header above the
    // rows, "−", "+" and "+ Sampler…" are buttons, and the bar count's dark display holds bright
    // text (its "1").
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
        let button = rgb8(style::BUTTON);
        for rect in [
            layout::bars_down_rect(),
            layout::bars_up_rect(),
            layout::add_sampler_rect(),
        ] {
            let pixel = shot.pixel(header(corner(rect)));
            assert!(near(pixel, button, TOLERANCE), "{rect:?}: {pixel:?}");
        }
        let plugin = shot.pixel(layout::rack_in_window(corner(layout::add_plugin_rect(1))));
        assert!(
            near(plugin, button, TOLERANCE),
            "+ Plugin instrument: {plugin:?}"
        );
        let count = layout::bars_count_rect();
        let ground = shot.pixel(header(corner(count)));
        assert!(near(ground, rgb8(style::LCD), TOLERANCE), "{ground:?}");
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
        // toolbar's ground, which starts a pixel left of the arrow. The piece in one frame
        // (2026-10-27): the name on the readouts' dark glass, the frame's edge at
        // the piece's left, 4 px under the groups' top (above the arrow's glyph).
        let width = shot.width as f32 / shot.scale;
        let spread = crate::ui::toolbar::shot_spread(width, false);
        let name = spread.pattern_name_x();
        let y = crate::ui::toolbar::group_rows().start + 4.0;
        let inside = shot.pixel(Point::new(name + 3.0, y));
        assert!(near(inside, rgb8(style::LCD), TOLERANCE), "{inside:?}");
        let arrow = shot.pixel(Point::new(name - 4.0, y));
        assert!(near(arrow, rgb8(style::BUTTON), TOLERANCE), "{arrow:?}");
        let piece = spread
            .left(crate::ui::toolbar::Group::Pattern)
            .expect("the pattern shows");
        let edge = shot.pixel(Point::new(piece + 0.5, y));
        assert!(near(edge, rgb8(style::BUTTON_EDGE), TOLERANCE), "{edge:?}");
        let before = shot.pixel(Point::new(piece - 1.0, y));
        assert!(near(before, rgb8(style::PANEL), TOLERANCE), "{before:?}");
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
                zoom: piano_layout::DEFAULT_ZOOM,
                step: Ticks(24),
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

    // The piano roll on p3_roll. Expected colors, as tests/fixtures/gen_p3_expected.py prints
    // them: the lead's red #E5533D, the kick's blue at 0.3 over the white row #3C576E, rows #34373D
    // and #2D3035, the bar line #626873, the dimmed row past the end #1F2125, the lane #1F2125, the
    // key label #25282D.
    #[test]
    #[ignore = "needs a screenshot of p3_roll with --panels piano-roll: scripts/ui-check.sh takes one and runs this"]
    fn p3_screenshot_shows_the_piano_roll() {
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
            let rect = view.stem_rect(note, steps, LANE_HEIGHT);
            roll_lane_in_window(Point::new(rect.center_x(), rect.y + offset), height)
        };
        let end = view.x_of(dawsome_engine::time::Ticks(384));
        let grid = roll_grid_in_window;
        let probes: [(&str, Point, [u8; 3]); 12] = [
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
                grid(Point::new(88.0, row_center(60))),
                [0x34, 0x37, 0x3D],
            ),
            (
                "empty black row",
                grid(Point::new(88.0, row_center(61))),
                [0x2D, 0x30, 0x35],
            ),
            (
                "bar line at the end",
                grid(Point::new(end + 1.0, row_center(62))),
                [0x62, 0x68, 0x73],
            ),
            (
                "past the end",
                grid(Point::new(312.0, row_center(60))),
                [0x1F, 0x21, 0x25],
            ),
            ("stem A inside", stem(&a, 100, 4.0), [0xE5, 0x53, 0x3D]),
            ("above stem A", stem(&a, 100, -4.0), [0x1F, 0x21, 0x25]),
            ("stem B inside", stem(&b, 64, 4.0), [0xE5, 0x53, 0x3D]),
            ("above stem B", stem(&b, 64, -4.0), [0x1F, 0x21, 0x25]),
        ];
        // The design's window points, written out: the layout must put them there. (The
        // four stem points moved: the lane is 64 px from y 496, a stem half as
        // tall, where they were at y 464, 456, 500 and 492 in the 128 px lane from y 432.
        // The grid's eight points 16 px lower under the 72 px toolbar, then 8 px higher under
        // the 64 px one, as gen_p3_expected.py prints them; the lane's stay, measured from the
        // window's foot.)
        let spec_points = [
            (64.0, 206.0),
            (144.0, 158.0),
            (184.0, 206.0),
            (248.0, 206.0),
            (88.0, 206.0),
            (88.0, 194.0),
            (305.0, 182.0),
            (312.0, 206.0),
            (50.0, 514.0),
            (50.0, 506.0),
            (114.0, 532.0),
            (114.0, 524.0),
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
    /// 1 at the top), its lanes fitting the 8 tracks of the p4 fixtures: 46 px over the scroll
    /// bar at the panel's foot (47 px with the bar in the header, and before that
    /// they were the 32 px every playlist opened with).
    fn p4_grid() -> crate::ui::playlist_layout::Grid {
        use crate::ui::playlist_layout::{fitted_lane, tracks_size_alone};
        let area = tracks_size_alone(iced::Size::new(1000.0, 560.0), layout::TOOLBAR_HEIGHT);
        crate::ui::playlist_layout::Grid {
            scroll: dawsome_engine::time::Ticks(0),
            zoom: crate::ui::playlist_layout::DEFAULT_ZOOM,
            bar: dawsome_engine::time::Ticks(384),
            top: 0,
            lane: fitted_lane(8, area.height),
        }
    }

    // p4_two_clips with the playlist alone (colors and window points as gen_p4_expected.py prints
    // them): clip bodies #5B7FA6, empty lanes #34383F, bar lines #484D55 with the lane color 24 px
    // right of each, and a bright bar number (every channel above 0x90) 3..15 px right of each bar
    // line, none mid-bar.
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
        let bar_line = [0x48, 0x4D, 0x55];
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
                "track 2 lane".into(),
                tracks(bar_x(0) + 24.0, lane_center(1, 22.0)),
                lane,
            ),
        ];
        // The design's window points, written out: the layout must put them there. (As
        // gen_p4_expected.py prints them: under the 64 px toolbar, in 47 px lanes a row every 48
        // px over the scroll bar at the panel's foot (track 2's was y 194 in 47 px lanes, y 130
        // and 163 under the 56 px one in 32 px lanes, and y 146 and 193 under the 72 px one in
        // 46 px lanes.)
        assert_eq!(grid.lane, 47.0);
        let spec_points = [
            (144.0, 138.0),
            (272.0, 138.0),
            (208.0, 138.0),
            (144.0, 186.0),
        ];
        for ((name, at, _), (x, y)) in points.iter().zip(spec_points) {
            assert_eq!((at.x, at.y), (x, y), "{name}: layout moved the probe point");
        }
        let lane_8 = lane_center(7, (grid.lane / 2.0).floor());
        for bar in 1..=13 {
            let line = tracks(bar_x(bar), lane_8);
            assert_eq!((line.x, line.y), (120.0 + 64.0 * bar as f32, 475.0));
            // Why the center of the logical pixel: the screenshot is the last frame the window
            // drew, and a compositor that is not showing the window (a locked screen) lets it
            // draw only its first, before the window reports its scale, so the lines are not on
            // whole device pixels yet. Unsnapped at a fractional scale (such as 1.7), a 1 px
            // line covers device pixels from 1.7x on and the pixel at its left edge is only
            // partly covered, so multisampling blends it with the lane. The pixel under the
            // center is covered snapped or not; `lines_land_on_whole_device_pixels` in
            // ui/playlist.rs pins the snapping itself.
            let center = Point::new(line.x + 0.5, line.y + 0.5);
            points.push((format!("bar line {}", bar + 1), center, bar_line));
            points.push((
                format!("lane after bar line {}", bar + 1),
                tracks(line.x + 24.0, lane_8),
                lane,
            ));
        }
        // Each clip's start line (#1A1C1F) at its first pixel, x 120 and 248, and the mark of
        // Pattern 1's tick-0 note (#DDE6F0) right of it, never over it: on the line it read as
        // a note outside the clip. The mark's row: one key in a 47 px lane, so the body under the
        // 10 px title less 2 px padding each side is 33 px, the mark 4 px high in its middle, y
        // 26.5..30.5 in the lane, 144.5 at its middle in the window (the lane's 22 is 138).
        let mark_y = lane_center(0, 28.5);
        assert_eq!(tracks(0.0, mark_y).y, 144.5);
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
        // Bar-number boxes [x+3, x+15] x [90, 102] for x = 120 + 64b, b = 0..12, window pixels.
        let bright = |pixel: [u8; 3]| pixel.iter().all(|&value| value > 0x90);
        let band =
            ruler_in_window(Point::new(0.0, 2.0)).y..ruler_in_window(Point::new(0.0, 15.0)).y;
        assert_eq!((band.start, band.end), (90.0, 103.0));
        let mut unlabeled = Vec::new();
        let mut mid_bar = Vec::new();
        for bar in 0..13 {
            let x = bar_x(bar);
            let label = shot.count_matching((x + 3.0)..(x + 16.0), band.clone(), bright);
            let middle = shot.count_matching((x + 35.0)..(x + 48.0), band.clone(), bright);
            println!(
                "bar {} number box: {label} bright px, mid-bar box {middle}",
                bar + 1
            );
            if label == 0 {
                unlabeled.push(bar + 1);
            }
            if middle > 0 {
                mid_bar.push(bar + 1);
            }
        }
        assert!(wrong.is_empty(), "wrong color at {wrong:?}");
        assert!(unlabeled.is_empty(), "no number for bars {unlabeled:?}");
        assert!(
            mid_bar.is_empty(),
            "bright pixels mid-bar in bars {mid_bar:?}"
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

    // p5_delay_sync with the mixer alone (insert 1 selected): slot 1 holds the delay, so its row
    // box [830, 990] × [124, 142] has bright text (every channel above 0x90); slot 2 is empty, its
    // dim dash leaves the box [830, 990] × [146, 164] dark. (Pins moved 22 px
    // down under the 72 px toolbar and the mixer's 6 px handle: 110 and 132; then 8 px up under
    // the 64 px toolbar.)
    #[test]
    #[ignore = "needs a screenshot of p5_delay_sync with --panels mixer: scripts/ui-check.sh takes one and runs this"]
    fn p5_delay_screenshot_names_the_effect_in_slot_1() {
        use crate::ui::mixer_layout as mix;

        let shot = read_ui_shot();
        let width = shot.width as f32 / shot.scale;
        let row = |slot: u8| {
            let rect = mix::slot_rect(slot, width);
            let top = mix::in_window(rect.position()).y;
            ((rect.x + 30.0)..(rect.x + 190.0), (top + 2.0)..(top + 20.0))
        };
        let bright = |pixel: [u8; 3]| pixel.iter().all(|&value| value > 0x90);
        let (x1, y1) = row(0);
        let (x2, y2) = row(1);
        assert_eq!((x1.start, y1.start, y1.end), (830.0, 124.0, 142.0));
        assert_eq!((x2.start, y2.start, y2.end), (830.0, 146.0, 164.0));
        let filled = shot.count_matching(x1, y1, bright);
        let empty = shot.count_matching(x2, y2, bright);
        println!("slot 1 bright px {filled}, slot 2 bright px {empty}");
        assert!(filled > 0, "slot 1 names its effect");
        assert_eq!(empty, 0, "slot 2 is empty");
    }

    // p5_gain with the mixer alone at 1000 px: strip 12 (x 756..816) runs under the slot panel
    // (from x 800), and its name must stop at the panel's edge rather than run into the
    // panel's title ("Insert 3", from x 806): the panel's ground between the two, over the
    // title and name rows (window y 94..118 under the 64 px toolbar and the mixer's handle;
    // pins moved from 80..104, and 102..126 under the 72 px toolbar), holds no text.
    #[test]
    #[ignore = "needs a screenshot of p5_gain with --panels mixer: scripts/ui-check.sh takes one and runs this"]
    fn p5_strip_names_stop_at_the_slot_panel() {
        use dawsome_engine::project::InsertId;

        use crate::ui::mixer_layout as mix;

        let shot = read_ui_shot();
        let width = shot.width as f32 / shot.scale;
        let edge = mix::slot_panel_left(width);
        let strip = mix::strip_left(InsertId(12), 0.0);
        assert_eq!((width, edge), (1000.0, 800.0));
        assert!(
            strip < edge && strip + mix::STRIP_WIDTH > edge,
            "strip 12 crosses the edge"
        );
        let title_left = edge + mix::SLOT_NUMBER_INSET;
        let rows = mix::in_window(Point::new(0.0, mix::SLOT_NAME_TOP)).y
            ..mix::in_window(Point::new(0.0, mix::NAME_TOP + mix::NAME_HEIGHT)).y;
        assert_eq!((rows.start, rows.end), (94.0, 118.0));
        let ground = rgb8(crate::ui::style::PANEL);
        let text = shot.count_matching(edge..title_left, rows, |pixel| {
            !near(pixel, ground, TOLERANCE)
        });
        println!("pixels other than the panel's ground left of its title: {text}");
        assert_eq!(text, 0, "strip 12's name runs into the slot panel");
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
    // Retry shows. Along y 12 (6d K5, pin moved from y 20: the buttons are 40 px tall from y 8
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
        let ground = rgb8(crate::ui::style::PANEL);
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
        use crate::ui::picker::{list_origin, ROW_HEIGHT, ROW_TONES};
        let shot = read_ui_shot();
        let origin = list_origin(iced::Size::new(1000.0, 560.0));
        let tones = ROW_TONES.map(rgb8);
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
        let well: u32 = rgb8(crate::ui::style::RACK_WELL)
            .iter()
            .map(|&channel| u32::from(channel))
            .sum();
        assert!(failed > well, "its text is drawn");
    }

    // The picker opened on p27_catalogue.json's instruments (CLAP and VST3) in the default 1000 by
    // 560 window: its format column (positions from crate::ui::picker) names each row's format.
    // Rows 0 and 2 (Dawsome Test Fail and Tick, VST3 alone) have the same text there, pixel for
    // pixel where it is drawn, as wide as "VST3" measures; row 1 (Dawsome Test Sine, the CLAP
    // build of a plugin installed in both) has other text, as wide as "CLAP" measures (the two
    // words are 1.5 px apart at 12 px: the widths are held to 1 px).
    #[test]
    #[ignore = "needs a screenshot of the picker over both formats: scripts/ui-check.sh takes one and runs this"]
    fn p27_picker_screenshot_names_each_rows_format() {
        use crate::ui::picker::{list_origin, FORMAT_X, KIND_X, ROW_HEIGHT, TEXT_SIZE};
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
        let (vst3, clap) = (text_width("VST3", TEXT_SIZE), text_width("CLAP", TEXT_SIZE));
        let ((fail, fail_width), (sine, sine_width), (tick, tick_width)) =
            (column(0), column(1), column(2));
        println!(
            "format column widths: Fail {fail_width}, Sine {sine_width}, Tick {tick_width}; \
             VST3 measures {vst3}, CLAP {clap}"
        );
        assert!(
            (vst3 - clap).abs() > 1.0,
            "the two words' widths tell them apart"
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
            let count_x = words + text_width(&title, 12.0);
            let count_end = words + text_width(&format!("{title}{count}"), 12.0) + 1.0;
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
    /// bright text (some channel at 0x80 or more).
    fn drawn(pixel: [u8; 3]) -> bool {
        pixel.iter().any(|&value| value >= 0x50)
    }

    fn not_dim(pixel: [u8; 3]) -> bool {
        pixel.iter().any(|&value| value >= 0x80)
    }

    // p10_library with the Library alone at 1000 by 844 (--cache p7_catalogue.json, --midi-ports
    // "Port A,Port B", a settings file playing Port A; 44 px over the 800 it was, for the external
    // input's two rows, so every row still shows): the three section headers sit at the rows
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
            layout::TOOLBAR_HEIGHT + 52.0 + 22.0 * items.len() as f32,
            "every row in an 844 px window"
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
        let link_x = browser::WIDTH - 8.0 - text_width("\u{2192} kick", 12.0);
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
             {c_text}, of them at 0x80 or more {c_bright}"
        );
        assert!(wrong.is_empty(), "wrong at {wrong:?}");
        assert!(link_a > 0, "Port A says it plays kick");
        assert_eq!((link_b, link_c), (0, 0), "no link on B, a dim one on C");
        assert!(c_text > 0 && c_bright == 0, "Port C is written, dim");
    }

    // A new document with the Library alone at 1000 by 800, no plugin catalogue and --midi-ports
    // none: the plugin instruments' and the MIDI devices' empty states are written on their rows,
    // dim (no channel reaches 0x80 anywhere in their boxes), Rescan is a button (#3A3E45 inside
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
            println!("hint row {row}: text px {text}, at 0x80 or more {bright}");
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

    // A new document with the Library alone at 1000 by 844 (44 px over the 800 it was, for the
    // external input's two rows, so the MIDI devices header still shows) and the four test sets
    // of tests/fixtures/instruments in the data folder's instruments/: the Instruments header
    // counts the twelve kit sounds and the four sets; "Instrument sets" is written dim on its
    // row; each set's row holds its glyph (drawn left of the name) and its name in bright text.
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
        assert!(matches!(items[0], Item::Header { count: 16, .. }));
        let mut wrong = library_headers(&shot, &items);
        let group = items
            .iter()
            .position(|item| *item == Item::Group(SHIPPED_SETS))
            .expect("listed");
        let label = shot.count_matching(library_text(ROW_LEFT), library_text_rows(group), drawn);
        let label_bright =
            shot.count_matching(library_text(ROW_LEFT), library_text_rows(group), not_dim);
        println!("'{SHIPPED_SETS}' row {group}: text px {label}, at 0x80 or more {label_bright}");
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
        let mark = right - text_width(PLAYING_MARK, 11.0) - 1.0..right + 1.0;
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

    /// Where settings panel point `point` is in the window when the panel is the only one:
    /// under the toolbar.
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
    // colors of the page's design): the "MIDI input" title box holds bright text (every channel
    // above 0x90); the line saying no port was found holds text, and dim text (no channel reaches
    // 0x80 anywhere in its box); no checkbox is ticked (no #F29A2E pixel anywhere in the window).
    #[test]
    #[ignore = "needs a screenshot of --panels settings --midi-ports none --screenshot-window settings: scripts/ui-check.sh takes one and runs this"]
    fn p9_settings_screenshot_says_no_port_was_found() {
        use crate::ui::settings_layout::Layout;
        let shot = read_ui_shot();
        let layout = Layout { ports: 0, notes: 0 };
        let title = settings_in_window(layout.midi_title());
        let message = settings_in_window(layout.empty_message().expect("no port"));
        // Pins moved 16 px down under the 72 px toolbar (were 88 and 130), then 72 px up: the
        // Settings window's own (were 104 and 146).
        assert_eq!((title.y, message.y), (32.0, 74.0), "the page's rows");
        let bright = |pixel: [u8; 3]| pixel.iter().all(|&value| value > 0x90);
        let drawn = |pixel: [u8; 3]| pixel.iter().any(|&value| value >= 0x50);
        let not_dim = |pixel: [u8; 3]| pixel.iter().any(|&value| value >= 0x80);
        let title_bright = shot.count_matching(x_range(title), y_range(title), bright);
        let message_drawn = shot.count_matching(x_range(message), y_range(message), drawn);
        let message_bright = shot.count_matching(x_range(message), y_range(message), not_dim);
        let width = shot.width as f32 / shot.scale;
        let height = shot.height as f32 / shot.scale;
        let ticked = shot.count_near(0.0..width, 0.0..height, [0xF2, 0x9A, 0x2E], TOLERANCE);
        println!(
            "title bright px {title_bright}; message text px {message_drawn}, of them at 0x80 or \
             more {message_bright}; ticked px {ticked}"
        );
        assert!(title_bright > 0, "the section is titled");
        assert!(message_drawn > 0, "the empty line is written");
        assert_eq!(message_bright, 0, "and dim");
        assert_eq!(ticked, 0, "no checkbox is ticked");
    }

    // The Settings window with --midi-ports "Port A,Port B" and a settings file that plays both:
    // their checkboxes are ticked #F29A2E at their centers, and "All ports" is not (#4A4F57).
    #[test]
    #[ignore = "needs a screenshot of the Settings window with two ports given: scripts/ui-check.sh takes one and runs this"]
    fn p9_settings_screenshot_ticks_the_two_ports_that_play() {
        use crate::ui::settings_layout::Layout;
        let shot = read_ui_shot();
        let layout = Layout { ports: 2, notes: 0 };
        let center = |row: usize| settings_in_window(layout.checkbox(row)).center();
        assert_eq!(
            [center(0), center(1), center(2)],
            // Pins moved 16 px down under the 72 px toolbar (were 119, 141, 163), then 72 px up:
            // the Settings window's own (were 135, 157, 179).
            [
                Point::new(18.0, 63.0),
                Point::new(18.0, 85.0),
                Point::new(18.0, 107.0)
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
        // Under the 64 px toolbar, in the 47 px lanes that fit the 8 tracks over the scroll bar
        // (124 and 172 in 47 px lanes before, 108 and 141 in 32 px lanes under the 56 px
        // one, and 124 and 171 in 46 px lanes under the 72 px one).
        assert_eq!((strip(0).start, strip(1).start), (116.0, 164.0));
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
