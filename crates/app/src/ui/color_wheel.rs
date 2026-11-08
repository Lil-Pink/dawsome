//! The accent's colour wheel on Settings' Theme page: the standard HSV picker, a hue ring
//! round a saturation/value square, and the HSV ↔ RGB arithmetic it picks in. Positions are the
//! wheel's own from [`Wheel::at`], its top-left corner, so the drawing, the hit test and the
//! probes read the same numbers, which `tests/fixtures/gen_accent_expected.py` works out from
//! the formulas alone.
//!
//! Hue 0 (red) lies at the ring's right and grows counter-clockwise, as on a colour wheel drawn
//! on paper; the square shows the ring's hue, saturation growing to the right and value upward.
//! Why a ring and a square, not a disc: a disc's centre is white, so its value would need a
//! slider of its own; the square gives the other two in one place.

use iced::widget::canvas::{gradient, Frame, Path, Stroke};
use iced::{Color, Point, Rectangle, Size};

use crate::ui::palette::Rgb;

/// The wheel's width and height. (120 on the General page, where it shared the room with the
/// zoom and the audio devices.)
pub const SIZE: f32 = 160.0;
/// The ring's thickness, in from the wheel's edge: a tenth of the wheel, as at 120 px.
pub const RING: f32 = 16.0;
/// The square's side: the largest in steps of 4 inscribed in the ring with 2 px to spare (its
/// corners 59.40 px from the centre, the ring's inner edge 64; 88 would reach 62.23), so its
/// quarters fall on whole pixels.
pub const SQUARE: f32 = 84.0;
/// The radius the hue's dot sits on: the ring's middle.
const MIDDLE: f32 = SIZE / 2.0 - RING / 2.0;
/// A dot (the hue's on the ring, the pick's in the square): a white circle this far out, in a
/// thin dark one, so it reads on any colour.
const DOT_RADIUS: f32 = 4.5;
/// How far, in degrees, each of the ring's 360 segments reaches into the next, which is drawn
/// over it. Why: two anti-aliased edges meeting leave a faint seam of the ground between them.
const SEGMENT_OVERLAP: f64 = 0.6;

/// A colour as hue (degrees, 0 to under 360), saturation and value (each 0 to 1): what the wheel
/// picks. Kept beside the 8-bit colour it gives because a grey has no hue of its own: a pick
/// keeps its hue while its saturation or its value is 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsv {
    pub hue: f64,
    pub saturation: f64,
    pub value: f64,
}

impl Hsv {
    /// `rgb`'s hue, saturation and value; a grey's hue is 0.
    pub fn of(rgb: Rgb) -> Self {
        let [r, g, b] = rgb.0.map(|channel| f64::from(channel) / 255.0);
        let high = r.max(g).max(b);
        let spread = high - r.min(g).min(b);
        let hue = if spread == 0.0 {
            0.0
        } else if high == r {
            60.0 * ((g - b) / spread).rem_euclid(6.0)
        } else if high == g {
            60.0 * ((b - r) / spread + 2.0)
        } else {
            60.0 * ((r - g) / spread + 4.0)
        };
        let saturation = if high == 0.0 { 0.0 } else { spread / high };
        Self {
            hue,
            saturation,
            value: high,
        }
    }

    /// The 8-bit colour this is, each channel rounded half up.
    pub fn rgb(self) -> Rgb {
        let chroma = self.value * self.saturation;
        let sector = self.hue / 60.0;
        let second = chroma * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = match sector.floor().rem_euclid(6.0) as u8 {
            0 => (chroma, second, 0.0),
            1 => (second, chroma, 0.0),
            2 => (0.0, chroma, second),
            3 => (0.0, second, chroma),
            4 => (second, 0.0, chroma),
            _ => (chroma, 0.0, second),
        };
        let lift = self.value - chroma;
        // Held to 0..=255 before the cast, so it never saturates silently.
        let byte = |part: f64| ((part + lift) * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8;
        Rgb([byte(r), byte(g), byte(b)])
    }

    /// The hue alone at its fullest: what the square shows at its top right.
    fn pure(self) -> Color {
        Hsv {
            saturation: 1.0,
            value: 1.0,
            ..self
        }
        .rgb()
        .color()
    }
}

/// A part of the wheel a press or a drag holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelPart {
    /// The ring: the hue.
    Ring,
    /// The square: the saturation and the value.
    Square,
}

/// Where a wheel lies: its top-left corner; it is [`SIZE`] across.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wheel {
    pub at: Point,
}

impl Wheel {
    pub fn bounds(&self) -> Rectangle {
        Rectangle::new(self.at, Size::new(SIZE, SIZE))
    }

    pub fn center(&self) -> Point {
        Point::new(self.at.x + SIZE / 2.0, self.at.y + SIZE / 2.0)
    }

    /// The saturation/value square, centred in the ring.
    pub fn square(&self) -> Rectangle {
        let center = self.center();
        Rectangle::new(
            Point::new(center.x - SQUARE / 2.0, center.y - SQUARE / 2.0),
            Size::new(SQUARE, SQUARE),
        )
    }

    /// The part a press at `at` lands on: the ring (from its inner edge to the wheel's), the
    /// square, or neither (the gap between them, the corners outside the ring).
    pub fn part_at(&self, at: Point) -> Option<WheelPart> {
        let center = self.center();
        let distance = (at.x - center.x).hypot(at.y - center.y);
        if (SIZE / 2.0 - RING..=SIZE / 2.0).contains(&distance) {
            Some(WheelPart::Ring)
        } else if self.square().contains(at) {
            Some(WheelPart::Square)
        } else {
            None
        }
    }

    /// Where the dot marking `hue` sits: on the ring's middle, at the hue's angle.
    pub fn hue_dot(&self, hue: f64) -> Point {
        let center = self.center();
        let (sin, cos) = hue.to_radians().sin_cos();
        Point::new(
            (f64::from(center.x) + f64::from(MIDDLE) * cos) as f32,
            (f64::from(center.y) - f64::from(MIDDLE) * sin) as f32,
        )
    }

    /// The hue at `at`'s angle round the centre, wherever `at` lies (a drag of the ring leaves
    /// it), 0 to under 360.
    pub fn hue_at(&self, at: Point) -> f64 {
        let center = self.center();
        let (across, up) = (f64::from(at.x - center.x), f64::from(center.y - at.y));
        up.atan2(across).to_degrees().rem_euclid(360.0)
    }

    /// Where the dot marking a saturation and a value sits in the square.
    pub fn pick_dot(&self, saturation: f64, value: f64) -> Point {
        let square = self.square();
        Point::new(
            square.x + (saturation * f64::from(SQUARE)) as f32,
            square.y + ((1.0 - value) * f64::from(SQUARE)) as f32,
        )
    }

    /// The saturation and value at `at`, held to the square's edges (a drag of the square
    /// leaves it).
    pub fn pick_at(&self, at: Point) -> (f64, f64) {
        let square = self.square();
        let along = |offset: f32| (f64::from(offset) / f64::from(SQUARE)).clamp(0.0, 1.0);
        (along(at.x - square.x), 1.0 - along(at.y - square.y))
    }

    /// `pick` with what a press or drag of `part` at `at` sets: the ring the hue, the square
    /// the saturation and the value.
    pub fn picked(&self, part: WheelPart, at: Point, pick: Hsv) -> Hsv {
        match part {
            WheelPart::Ring => Hsv {
                hue: self.hue_at(at),
                ..pick
            },
            WheelPart::Square => {
                let (saturation, value) = self.pick_at(at);
                Hsv {
                    saturation,
                    value,
                    ..pick
                }
            }
        }
    }

    /// The ring, the square in `pick`'s hue, and the two dots marking `pick`.
    pub fn draw(&self, frame: &mut Frame, pick: Hsv) {
        let center = self.center();
        let (inner, outer) = (SIZE / 2.0 - RING, SIZE / 2.0);
        let point = |radius: f32, degrees: f64| {
            let (sin, cos) = degrees.to_radians().sin_cos();
            Point::new(
                center.x + radius * cos as f32,
                center.y - radius * sin as f32,
            )
        };
        for degree in 0..360u16 {
            let from = f64::from(degree);
            let to = from + 1.0 + SEGMENT_OVERLAP;
            let segment = Path::new(|path| {
                path.move_to(point(inner, from));
                path.line_to(point(outer, from));
                path.line_to(point(outer, to));
                path.line_to(point(inner, to));
                path.close();
            });
            let hue = Hsv {
                hue: from + 0.5,
                saturation: 1.0,
                value: 1.0,
            };
            frame.fill(&segment, hue.rgb().color());
        }
        // White to the hue across, then black up from the foot over it: at saturation s and
        // value v that leaves v · ((1 − s) · white + s · hue), the HSV colour.
        let square = self.square();
        let (top_left, top_right, foot) = (
            square.position(),
            Point::new(square.x + square.width, square.y),
            Point::new(square.x, square.y + square.height),
        );
        let across = gradient::Linear::new(top_left, top_right)
            .add_stop(0.0, Color::WHITE)
            .add_stop(1.0, pick.pure());
        frame.fill_rectangle(top_left, square.size(), across);
        let down = gradient::Linear::new(top_left, foot)
            .add_stop(0.0, Color::TRANSPARENT)
            .add_stop(1.0, Color::BLACK);
        frame.fill_rectangle(top_left, square.size(), down);
        dot(frame, self.hue_dot(pick.hue));
        dot(frame, self.pick_dot(pick.saturation, pick.value));
    }
}

/// A dot at `at`: a white circle in a thin dark one.
fn dot(frame: &mut Frame, at: Point) {
    let rim = Color {
        a: 0.6,
        ..Color::BLACK
    };
    frame.stroke(
        &Path::circle(at, DOT_RADIUS + 1.5),
        Stroke::default().with_width(1.0).with_color(rim),
    );
    frame.stroke(
        &Path::circle(at, DOT_RADIUS),
        Stroke::default().with_width(2.0).with_color(Color::WHITE),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `tests/fixtures/gen_accent_expected.py` works out from the formulas alone.
    fn known() -> serde_json::Value {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/accent_expected.json"
        )))
        .expect("accent_expected.json")
    }

    fn number(value: &serde_json::Value) -> f64 {
        value.as_f64().expect("a number")
    }

    fn hsv_of(value: &serde_json::Value) -> Hsv {
        let parts: Vec<f64> = value.as_array().expect("hsv").iter().map(number).collect();
        Hsv {
            hue: parts[0],
            saturation: parts[1],
            value: parts[2],
        }
    }

    fn point_of(value: &serde_json::Value) -> Point {
        Point::new(number(&value[0]) as f32, number(&value[1]) as f32)
    }

    fn rgb_of(value: &serde_json::Value) -> Rgb {
        let hex = value.as_str().expect("a hex colour");
        Rgb::parse(hex).unwrap_or_else(|| panic!("{hex} is a colour"))
    }

    fn close(found: f64, known: f64, within: f64) -> bool {
        (found - known).abs() <= within
    }

    // HSV and back, against the generator's known answers: the primaries, Graphite's accent
    // (33.06°, 0.81, 0.95), a mid grey and black (hue 0, no saturation) and the probes' blue
    // (198.68°): each colour's hue, saturation and value to 1e-6, and back to the same 8-bit
    // colour. Picks made on the wheel give their colours, rounded half up (199°, half and half
    // is #406B80; 359.9° at full is red; no saturation is a grey whatever the hue).
    #[test]
    fn hsv_and_rgb_turn_into_each_other_as_the_formulas_say() {
        let known = known();
        for color in known["colors"].as_array().expect("colors") {
            let rgb = rgb_of(&color["hex"]);
            let found = Hsv::of(rgb);
            let wanted = hsv_of(&color["hsv"]);
            assert!(
                close(found.hue, wanted.hue, 1e-6)
                    && close(found.saturation, wanted.saturation, 1e-6)
                    && close(found.value, wanted.value, 1e-6),
                "{}: {found:?}, known {wanted:?}",
                rgb.hex()
            );
            assert_eq!(found.rgb(), rgb_of(&color["rgb"]), "{} back", rgb.hex());
        }
        for pick in known["picks"].as_array().expect("picks") {
            let hsv = hsv_of(&pick["hsv"]);
            assert_eq!(hsv.rgb(), rgb_of(&pick["rgb"]), "{hsv:?}");
        }
    }

    // The wheel's geometry at its own corner (0, 0), against the generator's: the hue's dot on
    // the ring's middle (radius 72 round 80, 80) at each known hue, to a thousandth of a pixel
    // (hue 0 at the right, 90 at the top: 80, 8); the hue a press gives at each known point
    // (131, 29 is 45°) to 1e-6; and the square's dot of each known colour and the saturation and
    // value at each known point, held to the square (38..122 both ways) when outside it.
    #[test]
    fn the_wheel_places_its_dots_as_the_formulas_say() {
        let known = known();
        let wheel = Wheel { at: Point::ORIGIN };
        let geometry = &known["wheel"];
        assert_eq!(
            (number(&geometry["size"]), number(&geometry["ring"])),
            (f64::from(SIZE), f64::from(RING))
        );
        assert_eq!(number(&geometry["square"]), f64::from(SQUARE));
        assert_eq!(wheel.center(), point_of(&geometry["center"]));
        for dot in geometry["dots"].as_array().expect("dots") {
            let found = wheel.hue_dot(number(&dot["hue"]));
            let (x, y) = (number(&dot["x"]), number(&dot["y"]));
            assert!(
                close(f64::from(found.x), x, 1e-3) && close(f64::from(found.y), y, 1e-3),
                "hue {}: {found:?}, known ({x}, {y})",
                dot["hue"]
            );
        }
        for case in geometry["hue_at"].as_array().expect("hue_at") {
            let at = point_of(&case["at"]);
            let hue = number(&case["hue"]);
            assert!(
                close(wheel.hue_at(at), hue, 1e-6),
                "{at:?}: {}",
                wheel.hue_at(at)
            );
            assert_eq!(
                wheel.part_at(at),
                Some(WheelPart::Ring),
                "{at:?} on the ring"
            );
        }
        for color in known["colors"].as_array().expect("colors") {
            let hsv = hsv_of(&color["hsv"]);
            let found = wheel.pick_dot(hsv.saturation, hsv.value);
            let wanted = point_of(&color["square"]);
            assert!(
                (found.x - wanted.x).abs() <= 1e-3 && (found.y - wanted.y).abs() <= 1e-3,
                "{}: {found:?}, known {wanted:?}",
                color["hex"]
            );
        }
        for case in geometry["sv_at"].as_array().expect("sv_at") {
            let (saturation, value) = wheel.pick_at(point_of(&case["at"]));
            let wanted = &case["sv"];
            assert!(
                close(saturation, number(&wanted[0]), 1e-6)
                    && close(value, number(&wanted[1]), 1e-6),
                "{}: {saturation}, {value}",
                case["at"]
            );
        }
    }

    // What a press lands on, by hand from the 160 px wheel: the ring from its inner edge (radius
    // 64) to the wheel's (80), the square (38..122), and nothing in the gap between them or in a
    // corner outside the ring. A drag of the ring sets the hue alone (8 px in at the left
    // middle: 180°), of the square the saturation and value alone (its top right: both 1): a
    // grey pick keeps its hue.
    #[test]
    fn a_press_holds_the_ring_or_the_square() {
        let wheel = Wheel {
            at: Point::new(100.0, 200.0),
        };
        let at = |x: f32, y: f32| Point::new(100.0 + x, 200.0 + y);
        assert_eq!(wheel.part_at(at(80.0, 1.0)), Some(WheelPart::Ring));
        assert_eq!(wheel.part_at(at(80.0, 16.0)), Some(WheelPart::Ring));
        assert_eq!(wheel.part_at(at(80.0, 17.0)), None, "inside the ring");
        assert_eq!(wheel.part_at(at(80.0, 37.0)), None, "the gap");
        assert_eq!(wheel.part_at(at(80.0, 39.0)), Some(WheelPart::Square));
        assert_eq!(wheel.part_at(at(1.0, 1.0)), None, "a corner");
        let grey = Hsv {
            hue: 120.0,
            saturation: 0.0,
            value: 0.5,
        };
        let turned = wheel.picked(WheelPart::Ring, at(8.0, 80.0), grey);
        assert_eq!(
            (turned.hue, turned.saturation, turned.value),
            (180.0, 0.0, 0.5)
        );
        let lit = wheel.picked(WheelPart::Square, at(122.0, 38.0), grey);
        assert_eq!((lit.hue, lit.saturation, lit.value), (120.0, 1.0, 1.0));
        assert_eq!(lit.rgb(), Rgb([0x00, 0xFF, 0x00]));
    }
}
