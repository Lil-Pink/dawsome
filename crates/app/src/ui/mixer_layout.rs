//! Every position in the mixer, in logical pixels. The mixer canvas draws from these values, its
//! hit test reads them, and the screenshot probe samples the points they give, so the drawing
//! and its checks cannot drift apart.
//!
//! Mixer coordinates start at the desk's top-left corner: the canvas under the panel's 6 px
//! top edge ([`HANDLE_HEIGHT`], the divider over it while a panel is above it). Alone in the
//! window the panel sits under the toolbar, so window y = mixer y + 72 + 6 ([`in_window`]).
//!
//! The desk is as tall as the mixer's height less its top edge ([`desk_height`]: never under
//! [`LEAST_DESK`], so every control and slot stays reachable, the panel scrolling it when it
//! is shorter), or as the panel under the handle when that is taller (the mixer alone, or
//! sharing the window with the settings: the scrollable stretches its content to its view).
//! The canvas reports the height it got, which every hit test uses. Its strips' fader track
//! takes what the height leaves ([`fader_bottom`]): 200 px at the old 504 px panel, 190 at the
//! 400 px default (the controls keep their sizes).
//!
//! The desk, top to bottom, mixer y at a 504 px desk: the header (0..24) with the channel
//! routing control at x 560..790; then the strips (24..504) left of the slot panel, which takes
//! the last 200 px of the width (x 800..1000 in a 1000 px window).
//!
//! A strip is 60 px wide: the master at x 4, insert k at x 12 + 62k, all moved left by the
//! scroll. Within a strip at x (mixer y, a 504 px desk): body 28..500, name 32..48, mute light
//! centered at (x+12, 60), pan knob 28 px from (x+16, 70), fader track x+12..x+18 over 114..314
//! with the cap centered on y = 314 − 200·fader, meters x+36..x+42 and x+45..x+51 over 114..314,
//! the master's clip light at (x+44, 106), route button x+16..x+44 over 328..342, send knob
//! 24 px from (x+18, 348), label 480..496. A shorter desk moves the fader's foot, the route
//! button, the send knob, the label and the body's foot up together.
//!
//! The slot panel (mixer y): the insert's name 24..48, ten 22 px slot rows from 52, the
//! effect's parameters in 20 px rows from 284, as many as the desk has room for; the wheel over
//! them scrolls through the rest.

use dawsome_engine::project::InsertId;
use iced::{Point, Rectangle, Size};

/// The mixer panel's height until the divider over it is dragged: its top edge and the desk. 400 px
/// (was 504, the whole window under the toolbar, which was asked to be smaller). Kept in
/// `settings.json` as `mixer.height` when dragged off it.
pub const DEFAULT_HEIGHT: f32 = 400.0;

/// The panel's top edge: the divider over it ([`crate::ui::panels::DIVIDER_BAND`] is this
/// band; dragged up the mixer grows, down it shrinks), drawn plain while it is alone.
pub const HANDLE_HEIGHT: f32 = 6.0;

/// The desk's least height: the slot panel's ten slots and one parameter row (284 + 20), and
/// a strip with a 100 px fader track. A shorter panel scrolls its desk.
pub const LEAST_DESK: f32 = PARAMS_TOP + PARAM_HEIGHT;

/// The desk under the top edge of a mixer panel `height` tall.
pub fn desk_height(height: f32) -> f32 {
    (height - HANDLE_HEIGHT).max(LEAST_DESK)
}

pub const HEADER_HEIGHT: f32 = 24.0;

/// The routing control in the header: the "−" button, the label, the "+" button.
pub const ROUTE_CONTROL_LEFT: f32 = 560.0;
pub const ROUTE_CONTROL_RIGHT: f32 = 790.0;
pub const ROUTE_ARROW_WIDTH: f32 = 20.0;

/// Width of the slot panel at the right edge.
pub const SLOT_PANEL_WIDTH: f32 = 200.0;

pub const STRIP_WIDTH: f32 = 60.0;
pub const MASTER_LEFT: f32 = 4.0;
/// Insert k's left edge is `INSERTS_LEFT + STRIP_PITCH·k`.
pub const INSERTS_LEFT: f32 = 12.0;
pub const STRIP_PITCH: f32 = 62.0;

/// Mixer y of the strip body's top; its foot is 4 px above the desk's ([`body_bottom`]).
pub const BODY_TOP: f32 = 28.0;
pub const SELECTED_OUTLINE: f32 = 2.0;

pub const NAME_TOP: f32 = 32.0;
pub const NAME_HEIGHT: f32 = 16.0;
pub const NAME_SIZE: f32 = 12.0;

pub const LED_OFFSET: f32 = 12.0;
pub const LED_Y: f32 = 60.0;
pub const LED_RADIUS: f32 = 5.0;
/// The light takes clicks this far either side of its center.
pub const LED_HIT_HALF: f32 = 8.0;

pub const PAN_OFFSET: f32 = 16.0;
pub const PAN_TOP: f32 = 70.0;
pub const PAN_SIZE: f32 = 28.0;

/// The fader track: from fader 1 at the top to 0 at its foot, at most 200 px tall
/// ([`fader_bottom`]).
pub const FADER_TRACK_OFFSET: f32 = 12.0;
pub const FADER_TRACK_WIDTH: f32 = 6.0;
pub const FADER_TOP: f32 = 114.0;
pub const MAX_FADER_SPAN: f32 = 200.0;
/// What a strip needs under its fader's foot: the route button 14 px down, the send knob 34 px
/// down and 24 tall, 8 px, the label (16), the body's 4 px and the desk's 4 px under it.
const UNDER_FADER: f32 = 90.0;
pub const CAP_OFFSET: f32 = 1.0;
pub const CAP_WIDTH: f32 = 28.0;
pub const CAP_HEIGHT: f32 = 10.0;
pub const CAP_LINE: f32 = 2.0;

/// The two meters, left then right, over the fader's height.
pub const METER_OFFSETS: [f32; 2] = [36.0, 45.0];
pub const METER_WIDTH: f32 = 6.0;
/// The meter shows −60 dB at the bottom to +6 dB at the top.
pub const METER_FLOOR_DB: f32 = -60.0;
pub const METER_RANGE_DB: f32 = 66.0;

pub const CLIP_OFFSET: f32 = 44.0;
pub const CLIP_Y: f32 = 106.0;
pub const CLIP_RADIUS: f32 = 3.0;

/// The route button and the send knob, this far under the fader's foot.
pub const ROUTE_OFFSET: f32 = 16.0;
const ROUTE_UNDER_FADER: f32 = 14.0;
pub const ROUTE_WIDTH: f32 = 28.0;
pub const ROUTE_HEIGHT: f32 = 14.0;

pub const SEND_OFFSET: f32 = 18.0;
const SEND_UNDER_FADER: f32 = 34.0;
pub const SEND_SIZE: f32 = 24.0;

/// The insert's number, 24 px above the desk's foot.
pub const LABEL_HEIGHT: f32 = 16.0;

/// Slot panel rows, mixer y.
pub const SLOT_NAME_TOP: f32 = HEADER_HEIGHT;
pub const SLOT_NAME_HEIGHT: f32 = 24.0;
pub const SLOTS_TOP: f32 = 52.0;
pub const SLOT_HEIGHT: f32 = 22.0;
pub const SLOT_NUMBER_INSET: f32 = 6.0;
pub const SLOT_NAME_INSET: f32 = 32.0;
pub const PARAMS_TOP: f32 = 284.0;
pub const PARAM_HEIGHT: f32 = 20.0;
pub const PARAM_LABEL_INSET: f32 = 6.0;
pub const SLIDER_INSET: f32 = 64.0;
pub const SLIDER_WIDTH: f32 = 120.0;
pub const SLIDER_HEIGHT: f32 = 12.0;

/// A slot's menu opens under the slot pressed, this far in from the slot panel's left edge.
pub const MENU_INSET: f32 = 28.0;
/// The slot menu's rows' width: its longest row, "Compressor (Pressure4)", and a margin.
pub const MENU_WIDTH: f32 = 180.0;

pub const TEXT_SIZE: f32 = 11.0;

/// Pixels the wheel scrolls the strips per notch: one strip.
pub const SCROLL_PER_NOTCH: f32 = STRIP_PITCH;

/// Where the panel's strips start: the slot panel's left edge in a panel `width` wide.
pub fn slot_panel_left(width: f32) -> f32 {
    (width - SLOT_PANEL_WIDTH).max(0.0)
}

/// Strip `id`'s left edge, mixer x, with the strips scrolled `scroll` px.
pub fn strip_left(id: InsertId, scroll: f32) -> f32 {
    let unscrolled = if id.is_master() {
        MASTER_LEFT
    } else {
        INSERTS_LEFT + STRIP_PITCH * f32::from(id.0)
    };
    unscrolled - scroll
}

/// How far the strips of a mixer with `last` as its highest insert can scroll in a panel
/// `width` wide: until the last strip's right edge reaches the slot panel.
pub fn max_scroll(last: InsertId, width: f32) -> f32 {
    let right = strip_left(last, 0.0) + STRIP_WIDTH + MASTER_LEFT;
    (right - slot_panel_left(width)).max(0.0)
}

/// The fader track's foot, mixer y, in a desk `desk` tall: what the controls under it leave,
/// at most [`MAX_FADER_SPAN`] under [`FADER_TOP`]. (314 at a 504 px desk, 304 at the default
/// 394, 214 at the least desk.)
pub fn fader_bottom(desk: f32) -> f32 {
    (desk.max(LEAST_DESK) - UNDER_FADER).min(FADER_TOP + MAX_FADER_SPAN)
}

/// The fader track's length in a desk `desk` tall.
pub fn fader_span(desk: f32) -> f32 {
    fader_bottom(desk) - FADER_TOP
}

/// The strip body's foot, mixer y, in a desk `desk` tall.
pub fn body_bottom(desk: f32) -> f32 {
    desk.max(LEAST_DESK) - 4.0
}

/// The insert number's top, mixer y, in a desk `desk` tall.
pub fn label_top(desk: f32) -> f32 {
    body_bottom(desk) - 20.0
}

/// The parameter rows a desk `desk` tall shows under the slots: (504 − 284) / 20 = 11 at the
/// old 504 px, 5 at the default.
pub fn param_rows(desk: f32) -> usize {
    ((desk - PARAMS_TOP) / PARAM_HEIGHT).floor().max(0.0) as usize
}

/// The mixer y the fader cap is centered on at position `fader`, in a desk `desk` tall.
pub fn cap_center_y(fader: f32, desk: f32) -> f32 {
    fader_bottom(desk) - fader_span(desk) * fader.clamp(0.0, 1.0)
}

/// The cap of strip at `left` with fader `fader`, in a desk `desk` tall.
pub fn cap_rect(left: f32, fader: f32, desk: f32) -> Rectangle {
    let center = cap_center_y(fader, desk);
    Rectangle::new(
        Point::new(left + CAP_OFFSET, center - CAP_HEIGHT / 2.0),
        Size::new(CAP_WIDTH, CAP_HEIGHT),
    )
}

/// The area a press on strip `left`'s fader lands in: the cap's width over the whole track.
pub fn fader_rect(left: f32, desk: f32) -> Rectangle {
    Rectangle::new(
        Point::new(left + CAP_OFFSET, FADER_TOP - CAP_HEIGHT / 2.0),
        Size::new(CAP_WIDTH, fader_span(desk) + CAP_HEIGHT),
    )
}

pub fn led_center(left: f32) -> Point {
    Point::new(left + LED_OFFSET, LED_Y)
}

pub fn pan_rect(left: f32) -> Rectangle {
    Rectangle::new(
        Point::new(left + PAN_OFFSET, PAN_TOP),
        Size::new(PAN_SIZE, PAN_SIZE),
    )
}

pub fn route_rect(left: f32, desk: f32) -> Rectangle {
    Rectangle::new(
        Point::new(left + ROUTE_OFFSET, fader_bottom(desk) + ROUTE_UNDER_FADER),
        Size::new(ROUTE_WIDTH, ROUTE_HEIGHT),
    )
}

pub fn send_rect(left: f32, desk: f32) -> Rectangle {
    Rectangle::new(
        Point::new(left + SEND_OFFSET, fader_bottom(desk) + SEND_UNDER_FADER),
        Size::new(SEND_SIZE, SEND_SIZE),
    )
}

pub fn name_rect(left: f32) -> Rectangle {
    Rectangle::new(
        Point::new(left, NAME_TOP),
        Size::new(STRIP_WIDTH, NAME_HEIGHT),
    )
}

pub fn body_rect(left: f32, desk: f32) -> Rectangle {
    Rectangle::new(
        Point::new(left, BODY_TOP),
        Size::new(STRIP_WIDTH, body_bottom(desk) - BODY_TOP),
    )
}

/// Meter `side`'s column of strip `left`, beside the fader track.
pub fn meter_rect(left: f32, side: usize, desk: f32) -> Rectangle {
    Rectangle::new(
        Point::new(left + METER_OFFSETS[side], FADER_TOP),
        Size::new(METER_WIDTH, fader_span(desk)),
    )
}

/// How much of a meter fills for a peak of `level` (linear): (dB + 60)/66, 0 at −60 dB and
/// below, 1 at +6 dB and above.
pub fn meter_fraction(level: f32) -> f32 {
    if level <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * level.log10();
    ((db - METER_FLOOR_DB) / METER_RANGE_DB).clamp(0.0, 1.0)
}

/// How high a meter `span` tall fills for a peak of `level`: 200·(dB + 60)/66 px at 200 px.
pub fn meter_height(level: f32, span: f32) -> f32 {
    span * meter_fraction(level)
}

/// Slot row `slot`'s rectangle in a panel `width` wide.
pub fn slot_rect(slot: u8, width: f32) -> Rectangle {
    Rectangle::new(
        Point::new(
            slot_panel_left(width),
            SLOTS_TOP + SLOT_HEIGHT * f32::from(slot),
        ),
        Size::new(SLOT_PANEL_WIDTH, SLOT_HEIGHT),
    )
}

/// Parameter row `index`'s slider track in a panel `width` wide.
pub fn slider_rect(index: usize, width: f32) -> Rectangle {
    let top = PARAMS_TOP + PARAM_HEIGHT * index as f32 + (PARAM_HEIGHT - SLIDER_HEIGHT) / 2.0;
    Rectangle::new(
        Point::new(slot_panel_left(width) + SLIDER_INSET, top),
        Size::new(SLIDER_WIDTH, SLIDER_HEIGHT),
    )
}

/// Parameter row `index` in a panel `width` wide.
pub fn param_row_rect(index: usize, width: f32) -> Rectangle {
    Rectangle::new(
        Point::new(
            slot_panel_left(width),
            PARAMS_TOP + PARAM_HEIGHT * index as f32,
        ),
        Size::new(SLOT_PANEL_WIDTH, PARAM_HEIGHT),
    )
}

/// Where the menu of slot `slot` opens in a panel `width` wide: under the slot, [`MENU_INSET`]
/// in. The menu is drawn on the window's top layer from there ([`crate::ui::menu`]).
pub fn menu_anchor(slot: u8, width: f32) -> Point {
    let under = slot_rect(slot, width);
    Point::new(under.x + MENU_INSET, under.y + under.height)
}

/// The header's "−" and "+" buttons and the label between them.
pub fn route_down_rect() -> Rectangle {
    header_rect(ROUTE_CONTROL_LEFT, ROUTE_ARROW_WIDTH)
}

pub fn route_up_rect() -> Rectangle {
    header_rect(ROUTE_CONTROL_RIGHT - ROUTE_ARROW_WIDTH, ROUTE_ARROW_WIDTH)
}

pub fn route_label_rect() -> Rectangle {
    let left = ROUTE_CONTROL_LEFT + ROUTE_ARROW_WIDTH;
    header_rect(left, ROUTE_CONTROL_RIGHT - ROUTE_ARROW_WIDTH - left)
}

fn header_rect(x: f32, width: f32) -> Rectangle {
    Rectangle::new(Point::new(x, 2.0), Size::new(width, HEADER_HEIGHT - 4.0))
}

/// A mixer point in the window, when the mixer is the only panel: under the toolbar and the
/// panel's top edge.
#[cfg(test)]
pub fn in_window(point: Point) -> Point {
    Point::new(
        point.x,
        point.y + crate::ui::layout::TOOLBAR_HEIGHT + HANDLE_HEIGHT,
    )
}

/// What a press at a mixer point lands on, ignoring menus (the state checks those first).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    RouteDown,
    RouteUp,
    RouteLabel,
    Led(InsertId),
    ClipLight,
    Pan(InsertId),
    Fader(InsertId),
    Route(InsertId),
    Send(InsertId),
    Name(InsertId),
    Strip(InsertId),
    Slot(u8),
    Param(usize),
}

/// What a press needs to know of the desk beyond the point: its size, how far its strips are
/// scrolled, the highest insert, and the first parameter row shown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Desk {
    pub size: Size,
    pub scroll: f32,
    pub last: InsertId,
    /// The parameter shown in the first row: the wheel scrolls them when they do not all fit.
    pub params_top: usize,
}

/// The control under `point` on `desk`. Strips under the slot panel are not hit; a parameter
/// row is hit as the parameter it shows.
pub fn hit_test(point: Point, desk: Desk) -> Option<Hit> {
    let Desk {
        size,
        scroll,
        last,
        params_top,
    } = desk;
    let (width, height) = (size.width, size.height);
    let slots_left = slot_panel_left(width);
    if point.y < HEADER_HEIGHT {
        return [
            (route_down_rect(), Hit::RouteDown),
            (route_up_rect(), Hit::RouteUp),
            (route_label_rect(), Hit::RouteLabel),
        ]
        .into_iter()
        .find(|(rect, _)| rect.contains(point))
        .map(|(_, hit)| hit);
    }
    if point.x >= slots_left {
        let slot = (0..10).find(|&slot| slot_rect(slot, width).contains(point));
        let param = (0..param_rows(height)).find(|&row| param_row_rect(row, width).contains(point));
        return slot
            .map(Hit::Slot)
            .or(param.map(|row| Hit::Param(params_top + row)));
    }
    let strip = (0..=last.0).map(InsertId).find(|&id| {
        let left = strip_left(id, scroll);
        (left..left + STRIP_WIDTH).contains(&point.x)
    })?;
    let left = strip_left(strip, scroll);
    if !body_rect(left, height).contains(point) {
        return None;
    }
    if led_center(left).distance(point) <= LED_HIT_HALF {
        return Some(Hit::Led(strip));
    }
    let clip = Point::new(left + CLIP_OFFSET, CLIP_Y);
    if strip.is_master() && clip.distance(point) <= CLIP_RADIUS + 3.0 {
        return Some(Hit::ClipLight);
    }
    let controls = [
        (pan_rect(left), Hit::Pan(strip)),
        (fader_rect(left, height), Hit::Fader(strip)),
        (route_rect(left, height), Hit::Route(strip)),
        (send_rect(left, height), Hit::Send(strip)),
        (name_rect(left), Hit::Name(strip)),
    ];
    let control = controls
        .into_iter()
        .find(|(rect, _)| rect.contains(point))
        .map(|(_, hit)| hit);
    Some(control.unwrap_or(Hit::Strip(strip)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: f32 = 1000.0;
    /// The desk the design was drawn at: the old 504 px panel, which had no handle.
    const DESIGN: f32 = 504.0;

    /// The design's window point (logical, the mixer alone under the design's 56 px toolbar) as
    /// a mixer point. The toolbar is 72 px and the panel has a 6 px handle
    /// over the desk now, so these tests keep the design's points and say where they are.
    fn window(x: f32, y: f32) -> Point {
        Point::new(x, y - 56.0)
    }

    /// A mixer point as the design's window point.
    fn design(point: Point) -> Point {
        Point::new(point.x, point.y + 56.0)
    }

    fn desk(height: f32, last: InsertId) -> Desk {
        Desk {
            size: Size::new(WIDTH, height),
            scroll: 0.0,
            last,
            params_top: 0,
        }
    }

    // The design's window layout, written out: master at x 4, insert k at 12 + 62k; the cap of a
    // fader at 0.8 is centered on window y 370 − 160 = 210, at 1.0 on 170.
    #[test]
    fn strips_and_their_parts_sit_where_the_design_puts_them() {
        assert_eq!(strip_left(InsertId::MASTER, 0.0), 4.0);
        assert_eq!(strip_left(InsertId(1), 0.0), 74.0);
        assert_eq!(strip_left(InsertId(3), 0.0), 198.0);
        assert_eq!(strip_left(InsertId(3), 62.0), 136.0);
        assert_eq!(design(Point::new(0.0, cap_center_y(0.8, DESIGN))).y, 210.0);
        assert_eq!(design(Point::new(0.0, cap_center_y(1.0, DESIGN))).y, 170.0);
        let cap = cap_rect(4.0, 0.8, DESIGN);
        assert_eq!((cap.x, cap.width, cap.height), (5.0, 28.0, 10.0));
        assert_eq!(
            design(route_rect(4.0, DESIGN).position()),
            Point::new(20.0, 384.0)
        );
        assert_eq!(
            design(send_rect(4.0, DESIGN).position()),
            Point::new(22.0, 404.0)
        );
        assert_eq!(
            design(meter_rect(4.0, 1, DESIGN).position()),
            Point::new(49.0, 170.0)
        );
        assert_eq!(
            design(slot_rect(0, WIDTH).position()),
            Point::new(800.0, 108.0)
        );
        assert_eq!(design(slot_rect(1, WIDTH).position()).y, 130.0);
        assert_eq!(design(param_row_rect(0, WIDTH).position()).y, 340.0);
        assert_eq!(slider_rect(0, WIDTH).width, 120.0);
        assert_eq!((label_top(DESIGN), body_bottom(DESIGN)), (480.0, 500.0));
        assert_eq!(param_rows(DESIGN), 11);
    }

    // By hand: the default panel is 400 px, its desk 394 under the 6 px
    // handle. The controls under the fader keep their sizes and move up with its foot: the
    // label at 394 − 4 − 20 = 370, the send knob's foot 8 px above it (370 − 8 − 24 = 338 is
    // its top), the route button 20 px above that (318), so the fader's foot is 304 and its
    // track 190 px (114..304). Five parameter rows fit: (394 − 284) / 20 = 5.5. The least desk,
    // 304 px (the ten slots and one parameter row), keeps a 100 px track (114..214), the send
    // knob's foot at 272 and the label at 280..296; a shorter panel scrolls that desk.
    #[test]
    fn a_shorter_desk_shortens_the_fader_and_keeps_the_controls() {
        let default = desk_height(DEFAULT_HEIGHT);
        assert_eq!(default, 394.0);
        assert_eq!((label_top(default), body_bottom(default)), (370.0, 390.0));
        let send = send_rect(4.0, default);
        assert_eq!((send.y, send.y + send.height), (338.0, 362.0));
        assert_eq!(route_rect(4.0, default).y, 318.0);
        assert_eq!((fader_bottom(default), fader_span(default)), (304.0, 190.0));
        assert_eq!(cap_center_y(1.0, default), FADER_TOP);
        assert_eq!(param_rows(default), 5);
        assert_eq!(desk_height(180.0), LEAST_DESK);
        assert_eq!(LEAST_DESK, 304.0);
        assert_eq!(fader_span(LEAST_DESK), 100.0);
        let least = send_rect(4.0, LEAST_DESK);
        assert_eq!(least.y + least.height, 272.0);
        assert_eq!(label_top(LEAST_DESK), 280.0);
        assert_eq!(param_rows(LEAST_DESK), 1);
        // Taller than the design, the track stays 200 px and the parameters take the room.
        assert_eq!(fader_span(700.0), 200.0);
        assert_eq!(param_rows(700.0), 20);
    }

    #[test]
    fn meters_fill_from_minus_60_to_plus_6_db() {
        let span = 200.0;
        assert_eq!(meter_height(0.0, span), 0.0);
        assert_eq!(
            meter_height(0.000_1, span),
            0.0,
            "−80 dB is below the floor"
        );
        assert!((meter_height(1.0, span) - 200.0 * 60.0 / 66.0).abs() < 1e-4);
        assert_eq!(meter_height(100.0, span), 200.0, "+40 dB pins at the top");
        assert!((meter_height(0.5, span) - 200.0 * (60.0 - 6.0206) / 66.0).abs() < 0.01);
        assert_eq!(meter_fraction(100.0), 1.0);
        assert!((meter_height(1.0, 100.0) - 100.0 * 60.0 / 66.0).abs() < 1e-4);
    }

    #[test]
    fn presses_land_on_the_control_drawn_there() {
        let last = InsertId(16);
        let at = |x, y| hit_test(window(x, y), desk(DESIGN, last));
        assert_eq!(at(570.0, 68.0), Some(Hit::RouteDown));
        assert_eq!(at(780.0, 68.0), Some(Hit::RouteUp));
        assert_eq!(at(670.0, 68.0), Some(Hit::RouteLabel));
        assert_eq!(at(100.0, 68.0), None);
        // Insert 3 at x 198: light at (210, 116), pan 214..242 × 126..154, fader 199..227.
        assert_eq!(at(210.0, 116.0), Some(Hit::Led(InsertId(3))));
        assert_eq!(at(228.0, 140.0), Some(Hit::Pan(InsertId(3))));
        assert_eq!(at(213.0, 210.0), Some(Hit::Fader(InsertId(3))));
        assert_eq!(at(213.0, 370.0), Some(Hit::Fader(InsertId(3))));
        assert_eq!(at(228.0, 391.0), Some(Hit::Route(InsertId(3))));
        assert_eq!(at(228.0, 415.0), Some(Hit::Send(InsertId(3))));
        assert_eq!(at(228.0, 96.0), Some(Hit::Name(InsertId(3))));
        assert_eq!(at(240.0, 500.0), Some(Hit::Strip(InsertId(3))));
        assert_eq!(at(48.0, 162.0), Some(Hit::ClipLight));
        // The gap between strips, and the slot panel's rows.
        assert_eq!(at(259.0, 300.0), None);
        assert_eq!(at(900.0, 115.0), Some(Hit::Slot(0)));
        assert_eq!(at(900.0, 140.0), Some(Hit::Slot(1)));
        assert_eq!(at(900.0, 350.0), Some(Hit::Param(0)));
        // Insert 12 (x 756..816) is covered by the slot panel past x 800: its name row there is
        // the slot panel's title, which takes no presses.
        assert_eq!(at(790.0, 300.0), Some(Hit::Strip(InsertId(12))));
        assert_eq!(at(790.0, 96.0), Some(Hit::Name(InsertId(12))));
        assert_eq!(at(805.0, 96.0), None);
    }

    // On the default desk (394 px) the route button is at mixer y 318..332 and the send knob at
    // 338..362; the sixth parameter row (284 + 5·20 = 384) is past the rows shown, and with the
    // parameters scrolled by 3 the first row shows the fourth parameter.
    #[test]
    fn presses_on_a_shorter_desk_find_its_controls_and_scrolled_parameters() {
        let last = InsertId(16);
        let default = desk_height(DEFAULT_HEIGHT);
        let at = |x, y, params_top| {
            let desk = Desk {
                params_top,
                ..desk(default, last)
            };
            hit_test(Point::new(x, y), desk)
        };
        assert_eq!(at(228.0, 325.0, 0), Some(Hit::Route(InsertId(3))));
        assert_eq!(at(228.0, 350.0, 0), Some(Hit::Send(InsertId(3))));
        assert_eq!(at(213.0, 300.0, 0), Some(Hit::Fader(InsertId(3))));
        assert_eq!(at(900.0, 294.0, 0), Some(Hit::Param(0)));
        assert_eq!(at(900.0, 374.0, 0), Some(Hit::Param(4)));
        assert_eq!(at(900.0, 388.0, 0), None, "a row the desk has no room for");
        assert_eq!(at(900.0, 294.0, 3), Some(Hit::Param(3)));
    }

    #[test]
    fn the_strips_scroll_until_the_last_one_meets_the_slot_panel() {
        // Insert 16's right edge is 12 + 62·16 + 60 + 4 = 1068; the slot panel starts at 800.
        assert_eq!(max_scroll(InsertId(16), WIDTH), 268.0);
        assert_eq!(max_scroll(InsertId(2), WIDTH), 0.0);
    }
}
