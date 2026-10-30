//! Every position and size in the window, in logical pixels. The rack canvas draws from these
//! values, its hit test reads them, and the screenshot probe samples the step centers they give,
//! so the drawing and its checks cannot drift apart.
//!
//! Rack coordinates start at the rows' top-left corner, directly under the rack's header, which
//! sits under the toolbar; window coordinates add [`TOOLBAR_HEIGHT`] and [`RACK_HEADER_HEIGHT`]
//! to y ([`rack_in_window`]). Header coordinates start at the header's top-left corner. Each
//! row's controls sit at the row's top edge.
//!
//! Probe coordinates (window, logical): the center of step `i` in row `r` is
//! x = 236 + 30·i + 6·⌊i/4⌋, y = 110 + 38·r. Row 0: x = 236, 266, 296, 326, 362, 392, 422, 452,
//! 488, 518, 548, 578, 614, 644, 674, 704 at y = 110.

use iced::{Point, Rectangle, Size};

/// Window size when `--window-size` is not given.
pub const DEFAULT_WINDOW_SIZE: Size = Size::new(1000.0, 560.0);

/// The toolbar's height: 64 px (was 56 since the first commit; a bigger bar was asked for
/// twice, and growing what sat inside the 56 px was not enough: 72 px, until the bar was asked
/// about 10 % smaller both ways on 2026-10-27; 64.8 would leave its
/// groups a half pixel off center).
pub const TOOLBAR_HEIGHT: f32 = 64.0;

/// The hint bar along the window's foot, while it shows.
pub const HINT_BAR_HEIGHT: f32 = 22.0;

/// Space left and right of the toolbar's contents; the menus under it line up with it.
pub const TOOLBAR_SIDE_PADDING: f32 = 8.0;

/// The rack's header strip: its title, the selected pattern's bar count between "−" and "+",
/// and "+ Sampler…". It stays put while the rows under it scroll.
pub const RACK_HEADER_HEIGHT: f32 = 24.0;

/// Where the header's "Bars" label starts, and its "−", count and "+" (left edge, width).
pub const BARS_LABEL_X: f32 = 108.0;
const BARS_DOWN: (f32, f32) = (140.0, 20.0);
const BARS_COUNT: (f32, f32) = (160.0, 36.0);
const BARS_UP: (f32, f32) = (196.0, 20.0);
/// The header's "+ Sampler…" button, from the steps' left edge.
const ADD_SAMPLER_WIDTH: f32 = 100.0;

/// Space between the rack's header and the first rack row.
pub const RACK_PADDING_TOP: f32 = 8.0;
/// Space after the last step of a row, inside the scrollable rack.
pub const RACK_PADDING_RIGHT: f32 = 12.0;
pub const ROW_HEIGHT: f32 = 36.0;
pub const ROW_GAP: f32 = 2.0;

/// Height of every control in a row, and the side of step buttons and knobs.
pub const CONTROL_SIZE: f32 = 28.0;

pub const MUTE_LED_CENTER_X: f32 = 15.0;
pub const MUTE_LED_RADIUS: f32 = 5.0;
/// Width of the area around the mute light that takes its clicks; the light itself is small.
pub const MUTE_LED_HIT_WIDTH: f32 = 20.0;
pub const NAME_BUTTON_X: f32 = 28.0;
pub const NAME_BUTTON_WIDTH: f32 = 110.0;
/// Left edge of the pan knob.
pub const PAN_KNOB_X: f32 = 148.0;
/// Left edge of the volume knob.
pub const VOLUME_KNOB_X: f32 = 182.0;

/// Left edge of step 0.
pub const STEPS_X: f32 = 222.0;
/// Distance from one step's left edge to the next within a group.
pub const STEP_PITCH: f32 = 30.0;
/// Extra space after every group of steps.
pub const GROUP_GAP: f32 = 6.0;
pub const STEPS_PER_GROUP: u32 = 4;

/// Thickness of the playhead mark along a step's top edge.
pub const PLAYHEAD_EDGE: f32 = 2.0;

// Row controls must not overlap, and all of them must fit left of the steps and inside a row.
const _: () = assert!(
    MUTE_LED_CENTER_X + MUTE_LED_HIT_WIDTH / 2.0 < NAME_BUTTON_X
        && MUTE_LED_RADIUS <= MUTE_LED_HIT_WIDTH / 2.0
        && NAME_BUTTON_X + NAME_BUTTON_WIDTH < PAN_KNOB_X
        && PAN_KNOB_X + CONTROL_SIZE < VOLUME_KNOB_X
        && VOLUME_KNOB_X + CONTROL_SIZE < STEPS_X
        && CONTROL_SIZE <= ROW_HEIGHT
);

// The header's controls follow one another without overlapping, and its button starts with
// the steps.
const _: () = assert!(
    BARS_DOWN.0 + BARS_DOWN.1 <= BARS_COUNT.0
        && BARS_COUNT.0 + BARS_COUNT.1 <= BARS_UP.0
        && BARS_UP.0 + BARS_UP.1 < STEPS_X
);

/// `point`, in rack coordinates, in window coordinates while the rack is the top panel and the
/// browser is closed: where the probe looks for it.
#[cfg(test)]
pub fn rack_in_window(point: Point) -> Point {
    Point::new(point.x, point.y + TOOLBAR_HEIGHT + RACK_HEADER_HEIGHT)
}

/// The header's "−", the bar count and "+", header coordinates.
pub fn bars_down_rect() -> Rectangle {
    header_rect(BARS_DOWN)
}

pub fn bars_count_rect() -> Rectangle {
    header_rect(BARS_COUNT)
}

pub fn bars_up_rect() -> Rectangle {
    header_rect(BARS_UP)
}

/// The header's "+ Sampler…" button.
pub fn add_sampler_rect() -> Rectangle {
    header_rect((STEPS_X, ADD_SAMPLER_WIDTH))
}

/// A header control from its left edge and width, 2 px clear of the strip's top and bottom.
fn header_rect((x, width): (f32, f32)) -> Rectangle {
    Rectangle::new(
        Point::new(x, 2.0),
        Size::new(width, RACK_HEADER_HEIGHT - 4.0),
    )
}

/// The header control a click landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderHit {
    BarsDown,
    BarsUp,
    AddSampler,
}

/// Finds the header control under `point`, header coordinates.
pub fn header_hit_test(point: Point) -> Option<HeaderHit> {
    [
        (bars_down_rect(), HeaderHit::BarsDown),
        (bars_up_rect(), HeaderHit::BarsUp),
        (add_sampler_rect(), HeaderHit::AddSampler),
    ]
    .into_iter()
    .find(|(rect, _)| rect.contains(point))
    .map(|(_, hit)| hit)
}

/// Top edge of `row`, rack coordinates.
pub fn row_top(row: usize) -> f32 {
    RACK_PADDING_TOP + row as f32 * (ROW_HEIGHT + ROW_GAP)
}

/// The row of a rack of `rows` rows whose band, its controls and the gap under them, holds
/// `y` (rack coordinates).
pub fn row_at(y: f32, rows: usize) -> Option<usize> {
    let offset = y - RACK_PADDING_TOP;
    // `contains` is false for NaN too.
    if !(0.0..).contains(&offset) {
        return None;
    }
    // Truncation is the floor here: `offset` is not negative.
    let row = (offset / (ROW_HEIGHT + ROW_GAP)) as usize;
    (row < rows).then_some(row)
}

/// Left edge of `step`, counted across bars (bar 2 starts at step 16).
pub fn step_left(step: u32) -> f32 {
    STEPS_X + step as f32 * STEP_PITCH + (step / STEPS_PER_GROUP) as f32 * GROUP_GAP
}

/// The step button for `step` in `row`, rack coordinates.
pub fn step_rect(row: usize, step: u32) -> Rectangle {
    control_rect(step_left(step), row, CONTROL_SIZE)
}

pub fn name_button_rect(row: usize) -> Rectangle {
    control_rect(NAME_BUTTON_X, row, NAME_BUTTON_WIDTH)
}

pub fn mute_led_center(row: usize) -> Point {
    Point::new(MUTE_LED_CENTER_X, row_top(row) + CONTROL_SIZE / 2.0)
}

/// The area that takes clicks on the mute light of `row`.
pub fn mute_led_rect(row: usize) -> Rectangle {
    control_rect(
        MUTE_LED_CENTER_X - MUTE_LED_HIT_WIDTH / 2.0,
        row,
        MUTE_LED_HIT_WIDTH,
    )
}

/// The square the knob whose left edge is `knob_x` is drawn in.
pub fn knob_rect(knob_x: f32, row: usize) -> Rectangle {
    control_rect(knob_x, row, CONTROL_SIZE)
}

/// Center of the knob whose left edge is `knob_x`.
pub fn knob_center(knob_x: f32, row: usize) -> Point {
    knob_rect(knob_x, row).center()
}

fn control_rect(x: f32, row: usize, width: f32) -> Rectangle {
    Rectangle::new(Point::new(x, row_top(row)), Size::new(width, CONTROL_SIZE))
}

/// The "+ Plugin instrument…" button, in the row under the last channel, from the name
/// button's left edge to the volume knob's right edge.
pub fn add_plugin_rect(rows: usize) -> Rectangle {
    control_rect(
        NAME_BUTTON_X,
        rows,
        VOLUME_KNOB_X + CONTROL_SIZE - NAME_BUTTON_X,
    )
}

/// Size of the rack's canvas: its content ([`rack_size`]) and the add button's row under it.
pub fn rack_canvas_size(rows: usize, steps: u32) -> Size {
    Size::new(rack_size(rows, steps).width, row_top(rows + 1))
}

/// Size of the rack's content for `rows` channels of `steps` steps each.
pub fn rack_size(rows: usize, steps: u32) -> Size {
    let width = match steps.checked_sub(1) {
        Some(last) => step_left(last) + CONTROL_SIZE + RACK_PADDING_RIGHT,
        None => STEPS_X,
    };
    Size::new(width, row_top(rows))
}

/// The header of `row`'s channel: its light, name and knobs, left of its steps, a control tall.
/// A Library sound, set or plugin let go of on it gives the channel that sound.
pub fn channel_header_rect(row: usize) -> Rectangle {
    control_rect(0.0, row, STEPS_X - HEADER_GAP)
}

/// The room between a channel's header and its first step.
const HEADER_GAP: f32 = 4.0;

/// The row of a rack of `rows` rows whose header holds `point` (rack coordinates); none between
/// rows, over the steps, or under the last row (the "+").
pub fn channel_header_at(point: Point, rows: usize) -> Option<usize> {
    (0..rows).find(|&row| channel_header_rect(row).contains(point))
}

/// The control a click in the rack landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RackHit {
    MuteLed {
        row: usize,
    },
    NameButton {
        row: usize,
    },
    PanKnob {
        row: usize,
    },
    VolumeKnob {
        row: usize,
    },
    Step {
        row: usize,
        step: u32,
    },
    /// The "+ Plugin instrument…" button.
    AddPlugin,
}

impl RackHit {
    /// The row hit; none for the add button.
    pub fn row(self) -> Option<usize> {
        match self {
            RackHit::MuteLed { row }
            | RackHit::NameButton { row }
            | RackHit::PanKnob { row }
            | RackHit::VolumeKnob { row }
            | RackHit::Step { row, .. } => Some(row),
            RackHit::AddPlugin => None,
        }
    }
}

/// Finds the control under `point` (rack coordinates) in a rack of `rows` rows of `steps` steps.
pub fn hit_test(point: Point, rows: usize, steps: u32) -> Option<RackHit> {
    let add = || {
        add_plugin_rect(rows)
            .contains(point)
            .then_some(RackHit::AddPlugin)
    };
    (0..rows)
        .find_map(|row| {
            let top = row_top(row);
            if !(top..top + CONTROL_SIZE).contains(&point.y) {
                return None;
            }
            let controls = [
                (mute_led_rect(row), RackHit::MuteLed { row }),
                (name_button_rect(row), RackHit::NameButton { row }),
                (knob_rect(PAN_KNOB_X, row), RackHit::PanKnob { row }),
                (knob_rect(VOLUME_KNOB_X, row), RackHit::VolumeKnob { row }),
            ];
            if let Some((_, hit)) = controls.into_iter().find(|(rect, _)| rect.contains(point)) {
                return Some(hit);
            }
            (0..steps)
                .find(|&step| step_rect(row, step).contains(point))
                .map(|step| RackHit::Step { row, step })
        })
        .or_else(add)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Row r's band runs from its top, 8 + 38r, to the next row's top: rows 36 high and a 2 px
    // gap, so a drop never falls between two rows.
    #[test]
    fn a_rows_band_runs_from_its_top_to_the_next_rows() {
        assert_eq!(row_at(7.9, 2), None, "the padding above the rows");
        assert_eq!(row_at(8.0, 2), Some(0));
        assert_eq!(row_at(45.9, 2), Some(0), "the gap under row 0");
        assert_eq!(row_at(46.0, 2), Some(1));
        assert_eq!(row_at(83.9, 2), Some(1));
        assert_eq!(row_at(84.0, 2), None, "past the last row");
        assert_eq!(row_at(f32::NAN, 2), None);
    }

    // The layout rule, written out by hand: step i's left edge is 222 + 30i + 6⌊i/4⌋, the first
    // row's top is 8 below the 64-pixel toolbar (pins moved 16 px down from
    // the 56-pixel one's to the 72-pixel one, then 8 px up), and a step button is 28 square.
    #[test]
    fn step_centers_follow_the_layout_rule() {
        let expected_x = [
            236.0, 266.0, 296.0, 326.0, 362.0, 392.0, 422.0, 452.0, 488.0, 518.0, 548.0, 578.0,
            614.0, 644.0, 674.0, 704.0,
        ];
        for (step, &x) in (0..16).zip(&expected_x) {
            let center = step_rect(0, step).center();
            assert_eq!(center, Point::new(x, 86.0 - TOOLBAR_HEIGHT), "step {step}");
        }
        assert_eq!(step_rect(1, 0).center().y + TOOLBAR_HEIGHT, 124.0);
        // Bar 2's first step continues the rule: 222 + 30·16 + 6·4 = 726.
        assert_eq!(step_left(16), 726.0);
    }

    #[test]
    fn clicks_land_on_the_control_drawn_there() {
        let at = |x, y| hit_test(Point::new(x, y), 2, 16);
        assert_eq!(at(236.0, 22.0), Some(RackHit::Step { row: 0, step: 0 }));
        assert_eq!(at(362.0, 22.0), Some(RackHit::Step { row: 0, step: 4 }));
        assert_eq!(at(704.0, 22.0), Some(RackHit::Step { row: 0, step: 15 }));
        assert_eq!(at(236.0, 60.0), Some(RackHit::Step { row: 1, step: 0 }));
        assert_eq!(at(80.0, 22.0), Some(RackHit::NameButton { row: 0 }));
        assert_eq!(at(80.0, 60.0), Some(RackHit::NameButton { row: 1 }));
        // The light's click area is 5..25; the knobs are 148..176 and 182..210.
        assert_eq!(at(15.0, 22.0), Some(RackHit::MuteLed { row: 0 }));
        assert_eq!(at(6.0, 60.0), Some(RackHit::MuteLed { row: 1 }));
        assert_eq!(at(162.0, 22.0), Some(RackHit::PanKnob { row: 0 }));
        assert_eq!(at(196.0, 60.0), Some(RackHit::VolumeKnob { row: 1 }));
        assert_eq!(at(26.0, 22.0), None);
        assert_eq!(at(179.0, 22.0), None);
        // Gaps: between steps 0 and 1 (250..252), after a group (340..348), below a row's
        // controls (36..46), past the last step, and above the first row.
        assert_eq!(at(251.0, 22.0), None);
        assert_eq!(at(344.0, 22.0), None);
        assert_eq!(at(236.0, 40.0), None);
        assert_eq!(at(760.0, 22.0), None);
        assert_eq!(at(236.0, 4.0), None);
        // A third row does not exist.
        assert_eq!(at(236.0, 98.0), None);
    }

    // The add button sits in the row after the last channel, over the name and knob columns.
    #[test]
    fn the_add_button_is_under_the_last_row() {
        assert_eq!(
            hit_test(Point::new(80.0, 22.0 + 2.0 * 38.0), 2, 16),
            Some(RackHit::AddPlugin)
        );
        assert_eq!(
            hit_test(Point::new(80.0, 22.0), 0, 16),
            Some(RackHit::AddPlugin)
        );
        assert_eq!(
            hit_test(Point::new(236.0, 98.0), 2, 16),
            None,
            "no step there"
        );
        assert_eq!(rack_canvas_size(1, 16).height, row_top(2));
    }

    // The header's controls, written out: "−" 140..160, the count 160..196, "+" 196..216 and
    // "+ Sampler…" 222..322 (from the steps' left edge), each 2..22 down the 24 px strip.
    #[test]
    fn header_clicks_land_on_the_control_drawn_there() {
        let at = |x, y| header_hit_test(Point::new(x, y));
        assert_eq!(at(150.0, 12.0), Some(HeaderHit::BarsDown));
        assert_eq!(at(206.0, 12.0), Some(HeaderHit::BarsUp));
        assert_eq!(at(223.0, 3.0), Some(HeaderHit::AddSampler));
        assert_eq!(at(321.0, 21.0), Some(HeaderHit::AddSampler));
        assert_eq!(at(178.0, 12.0), None, "the count takes no click");
        assert_eq!(at(219.0, 12.0), None, "between \"+\" and the button");
        assert_eq!(at(150.0, 1.0), None, "above the buttons");
        assert_eq!(at(40.0, 12.0), None, "the title");
    }

    // The rows sit under the toolbar (64) and the header (24): step 1 of row 1 is centered on
    // window (236, 64 + 24 + 8 + 14) = (236, 110), step 16 of row 2 on (704, 148). (Pins
    // moved: 102 and 140 under the 56 px toolbar, 118 and 156 under the 72 px one.)
    #[test]
    fn the_rows_start_under_the_toolbar_and_the_header() {
        assert_eq!(
            rack_in_window(step_rect(0, 0).center()),
            Point::new(236.0, 110.0)
        );
        assert_eq!(
            rack_in_window(step_rect(1, 15).center()),
            Point::new(704.0, 148.0)
        );
    }

    // P4: a channel's header is its light, name and knobs, x 0..218, and a control tall: row 1's
    // runs y 46..74. Between rows (y 74..84), over the steps (x 222) and on the "+" under the
    // last row there is none.
    #[test]
    fn a_header_runs_from_the_light_to_the_knobs() {
        assert_eq!(
            channel_header_rect(1),
            Rectangle::new(Point::new(0.0, 46.0), Size::new(218.0, 28.0))
        );
        let at = |x, y| channel_header_at(Point::new(x, y), 2);
        assert_eq!(at(1.0, 47.0), Some(1));
        assert_eq!(at(80.0, 22.0), Some(0), "the name");
        assert_eq!(at(196.0, 60.0), Some(1), "the volume knob");
        assert_eq!(at(80.0, 78.0), None, "between the rows");
        assert_eq!(at(236.0, 22.0), None, "a step");
        assert_eq!(
            at(80.0, 22.0 + 2.0 * 38.0),
            None,
            "the + under the last row"
        );
    }

    #[test]
    fn rack_size_covers_every_step_and_row() {
        assert_eq!(rack_size(1, 16), Size::new(704.0 + 14.0 + 12.0, 46.0));
        assert_eq!(rack_size(0, 0), Size::new(STEPS_X, RACK_PADDING_TOP));
    }
}
