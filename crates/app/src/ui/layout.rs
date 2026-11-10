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

use std::sync::OnceLock;

use dawsome_engine::time::TimeSignature;
use iced::{Point, Rectangle, Size};

use crate::ui::add_words;
use crate::ui::panels::Panel;

/// Window size when `--window-size` is not given.
pub const DEFAULT_WINDOW_SIZE: Size = Size::new(1000.0, 560.0);

/// The toolbar's height: 64 px (was 56, too small even with what sat inside it grown; then
/// 72 px, until the bar was made about 10 % smaller both ways; 64.8 would leave its groups a
/// half pixel off center).
pub const TOOLBAR_HEIGHT: f32 = 64.0;

/// The hint bar along the window's foot, while it shows.
pub const HINT_BAR_HEIGHT: f32 = 22.0;

/// Space left and right of the toolbar's contents; the menus under it line up with it.
pub const TOOLBAR_SIDE_PADDING: f32 = 8.0;

/// The rack's header strip: its title, the pattern's swing knob with its readout, and the
/// selected pattern's bar count between "−" and "+". It stays put while the rows under it
/// scroll; "+ Instrument…" stands under the rows ([`add_instrument_rect`]).
pub const RACK_HEADER_HEIGHT: f32 = 24.0;

/// Where the header's title starts: the strips' side padding, as the other headers'.
pub const RACK_TITLE_X: f32 = 12.0;

/// The space between the header's groups: the swing's readout and the "Bars" label (12, what
/// stood between "+ Instrument…" and the swing while the button was the header's).
const HEADER_GROUP_GAP: f32 = 12.0;

/// Where the header's "Swing" label starts, its first control: [`TITLE_GAP`] past the title
/// ([`Panel::after_title`]), 12 + 90.952 ("Channel Rack" at 14 px) + 48 = 150.952. The bars
/// follow a group's gap after the swing. Why the bars last (they came first, the title gap
/// before them): the pattern's feel first, its length after it.
pub fn swing_label_x() -> f32 {
    Panel::Rack.after_title(RACK_TITLE_X)
}

/// The swing's knob and readout (left edge from the "Swing" label's start, width). Why the
/// readout beside the knob, not under it: the strip is 24 px tall.
const SWING_KNOB: (f32, f32) = (42.0, 20.0);
const SWING_COUNT: (f32, f32) = (66.0, 48.0);

/// Where the header's "Bars" label starts: a group's gap past the swing's readout.
pub fn bars_label_x() -> f32 {
    swing_label_x() + SWING_COUNT.0 + SWING_COUNT.1 + HEADER_GROUP_GAP
}

/// The header's "−", count and "+" (left edge from the "Bars" label's start, width): "−" 30 px
/// on, 4.56 px past the label's 25.44 at the labels' size, the count 32 px wide. Three digits
/// fit the count with room.
const BARS_DOWN: (f32, f32) = (30.0, 20.0);
const BARS_COUNT: (f32, f32) = (50.0, 32.0);
const BARS_UP: (f32, f32) = (82.0, 20.0);
/// The pattern's own time signature, right of "+", when it has one: as wide as "24/16" at the
/// numbers' size and its glass (the toolbar's readout is 41 px for the same).
const SIGNATURE: (f32, f32) = (110.0, 41.0);

/// Space between the rack's header and the first rack row.
pub const RACK_PADDING_TOP: f32 = 8.0;
/// The row under the rack's last row holding "+ Instrument…" ([`add_instrument_rect`]), a
/// [`ROW_GAP`] after it as one row after another; with no rows, the rack's only row. The
/// words-alone row's height, as "+ Track"'s under the tracks.
pub const ADD_ROW_HEIGHT: f32 = add_words::HEIGHT;
/// The words under the rows that add a channel: one picker over the sets, the starter kit, the
/// plugin instruments and a sound file of the user's.
pub const ADD_INSTRUMENT_LABEL: &str = "+ Instrument\u{2026}";
/// Space after the last step of a row, inside the scrollable rack.
pub const RACK_PADDING_RIGHT: f32 = 12.0;
pub const ROW_HEIGHT: f32 = 36.0;
pub const ROW_GAP: f32 = 2.0;

/// Height of every control in a row, and the side of step buttons and knobs.
pub const CONTROL_SIZE: f32 = 28.0;

pub const MUTE_LED_CENTER_X: f32 = 15.0;
pub const MUTE_LED_RADIUS: f32 = 5.0;
/// Width of the area around the mute light that takes its clicks, a 24 px target (was 20); the
/// light itself is small.
pub const MUTE_LED_HIT_WIDTH: f32 = 24.0;
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
/// Extra space after every group of steps (none when a group is one step).
pub const GROUP_GAP: f32 = 6.0;

/// Steps in one of a rack row's groups: a beat of the time signature (4 for a quarter, 2 for an
/// eighth, 1 for a 16th). The unlit steps' tones alternate by group, a beat line stands between
/// groups, and a gap follows each group of more than one step.
pub fn steps_per_group(signature: TimeSignature) -> u32 {
    signature.steps_per_beat().max(1)
}

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

// Within each group the header's controls follow one another without overlapping (the labels
// before their controls are the header test's: their widths are measured).
const _: () = assert!(
    BARS_DOWN.0 + BARS_DOWN.1 <= BARS_COUNT.0
        && BARS_COUNT.0 + BARS_COUNT.1 <= BARS_UP.0
        && SWING_KNOB.0 + SWING_KNOB.1 <= SWING_COUNT.0
);

/// `point`, in rack coordinates, in window coordinates while the rack is the top panel and the
/// browser is closed: where the probe looks for it.
#[cfg(test)]
pub fn rack_in_window(point: Point) -> Point {
    Point::new(point.x, point.y + TOOLBAR_HEIGHT + RACK_HEADER_HEIGHT)
}

/// The header's "−", the bar count and "+", header coordinates.
pub fn bars_down_rect() -> Rectangle {
    bars_rect(BARS_DOWN)
}

pub fn bars_count_rect() -> Rectangle {
    bars_rect(BARS_COUNT)
}

pub fn bars_up_rect() -> Rectangle {
    bars_rect(BARS_UP)
}

/// The pattern's own time signature's readout right of "+" (shown only when it has one).
pub fn signature_rect() -> Rectangle {
    bars_rect(SIGNATURE)
}

/// A control of the bars' group from its offset past the "Bars" label's start and its width.
fn bars_rect((offset, width): (f32, f32)) -> Rectangle {
    header_rect((bars_label_x() + offset, width))
}

/// "+ Instrument…" under the last of `rows` rows, where a press takes it, rack coordinates: its
/// words alone ([`add_words`], as "+ Track"), as wide as [`add_words::width`] gives, the add
/// row's whole height, from the rows' name boxes' left edge ([`NAME_BUTTON_X`]), so the words
/// start [`add_words::PAD`] in, where the button's label did. Why under the rows (it was the
/// header's): a channel added where the channels are, as the playlist's "+ Track" stands under
/// its tracks; why words alone (it was a button 100 x 20): the two "+" rows alike.
pub fn add_instrument_rect(rows: usize) -> Rectangle {
    static WIDTH: OnceLock<f32> = OnceLock::new();
    let width = *WIDTH.get_or_init(|| add_words::width(ADD_INSTRUMENT_LABEL));
    Rectangle::new(
        Point::new(NAME_BUTTON_X, row_top(rows)),
        Size::new(width, ADD_ROW_HEIGHT),
    )
}

/// A control of the swing's group from its offset past the "Swing" label's start and its width.
fn swing_part((offset, width): (f32, f32)) -> Rectangle {
    header_rect((swing_label_x() + offset, width))
}

/// The header's swing knob, a control tall and as wide: square.
pub fn swing_knob_rect() -> Rectangle {
    swing_part(SWING_KNOB)
}

/// The swing's readout, right of its knob.
pub fn swing_count_rect() -> Rectangle {
    swing_part(SWING_COUNT)
}

/// What a press or the wheel on the swing takes: its knob and its readout.
pub fn swing_rect() -> Rectangle {
    swing_part((SWING_KNOB.0, SWING_COUNT.0 + SWING_COUNT.1 - SWING_KNOB.0))
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
    /// The pattern's own time signature's readout.
    Signature,
    /// The swing's knob or its readout.
    Swing,
}

/// Finds the header control under `point`, header coordinates.
pub fn header_hit_test(point: Point) -> Option<HeaderHit> {
    [
        (bars_down_rect(), HeaderHit::BarsDown),
        (bars_up_rect(), HeaderHit::BarsUp),
        (signature_rect(), HeaderHit::Signature),
        (swing_rect(), HeaderHit::Swing),
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

/// Left edge of `step` in groups of `group` steps ([`steps_per_group`]), counted across bars
/// (in 4/4 bar 2 starts at step 16).
pub fn step_left(step: u32, group: u32) -> f32 {
    let gaps = if group > 1 { step / group } else { 0 };
    STEPS_X + step as f32 * STEP_PITCH + gaps as f32 * GROUP_GAP
}

/// The step button for `step` in `row`, in groups of `group` steps, rack coordinates.
pub fn step_rect(row: usize, step: u32, group: u32) -> Rectangle {
    control_rect(step_left(step, group), row, CONTROL_SIZE)
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

/// Size of the rack's canvas for `rows` channels of `steps` steps each: its rows, the padding
/// above them and the add row under the last row's band ([`ADD_ROW_HEIGHT`], "+ Instrument…").
/// A sound let go of anywhere on the rack but a channel's header adds a channel (the rack's
/// well, [`crate::app`]'s rack hover), the add row among it. (A wide "+" row, 36 px, stood
/// under the rows until the header's "+ Instrument…" made it a second way to one place; the
/// button later came down into a row of its own.)
pub fn rack_size(rows: usize, steps: u32, group: u32) -> Size {
    let width = match steps.checked_sub(1) {
        Some(last) => step_left(last, group) + CONTROL_SIZE + RACK_PADDING_RIGHT,
        None => STEPS_X,
    };
    Size::new(width, row_top(rows) + ADD_ROW_HEIGHT)
}

/// The header of `row`'s channel: its light, name and knobs, left of its steps, a control tall.
/// A Library sound, set or plugin let go of on it gives the channel that sound.
pub fn channel_header_rect(row: usize) -> Rectangle {
    control_rect(0.0, row, STEPS_X - HEADER_GAP)
}

/// The room between a channel's header and its first step.
const HEADER_GAP: f32 = 4.0;

/// The row of a rack of `rows` rows whose header holds `point` (rack coordinates); none between
/// rows, over the steps, or under the last row.
pub fn channel_header_at(point: Point, rows: usize) -> Option<usize> {
    (0..rows).find(|&row| channel_header_rect(row).contains(point))
}

/// The control a click in the rack landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RackHit {
    MuteLed { row: usize },
    NameButton { row: usize },
    PanKnob { row: usize },
    VolumeKnob { row: usize },
    Step { row: usize, step: u32 },
}

impl RackHit {
    /// The row hit.
    pub fn row(self) -> usize {
        match self {
            RackHit::MuteLed { row }
            | RackHit::NameButton { row }
            | RackHit::PanKnob { row }
            | RackHit::VolumeKnob { row }
            | RackHit::Step { row, .. } => row,
        }
    }
}

/// Finds the control under `point` (rack coordinates) in a rack of `rows` rows of `steps` steps
/// in groups of `group`.
pub fn hit_test(point: Point, rows: usize, steps: u32, group: u32) -> Option<RackHit> {
    (0..rows).find_map(|row| {
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
            .find(|&step| step_rect(row, step, group).contains(point))
            .map(|step| RackHit::Step { row, step })
    })
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
            let center = step_rect(0, step, 4).center();
            assert_eq!(center, Point::new(x, 86.0 - TOOLBAR_HEIGHT), "step {step}");
        }
        assert_eq!(step_rect(1, 0, 4).center().y + TOOLBAR_HEIGHT, 124.0);
        // Bar 2's first step continues the rule: 222 + 30·16 + 6·4 = 726.
        assert_eq!(step_left(16, 4), 726.0);
    }

    #[test]
    fn clicks_land_on_the_control_drawn_there() {
        let at = |x, y| hit_test(Point::new(x, y), 2, 16, 4);
        assert_eq!(at(236.0, 22.0), Some(RackHit::Step { row: 0, step: 0 }));
        assert_eq!(at(362.0, 22.0), Some(RackHit::Step { row: 0, step: 4 }));
        assert_eq!(at(704.0, 22.0), Some(RackHit::Step { row: 0, step: 15 }));
        assert_eq!(at(236.0, 60.0), Some(RackHit::Step { row: 1, step: 0 }));
        assert_eq!(at(80.0, 22.0), Some(RackHit::NameButton { row: 0 }));
        assert_eq!(at(80.0, 60.0), Some(RackHit::NameButton { row: 1 }));
        // The light's click area is 15 ± 12 = 3..27 (24 px, WCAG 2.2's target size; was 5..25), up to the
        // name button's 28; the knobs are 148..176 and 182..210.
        assert_eq!(at(15.0, 22.0), Some(RackHit::MuteLed { row: 0 }));
        assert_eq!(at(6.0, 60.0), Some(RackHit::MuteLed { row: 1 }));
        assert_eq!(at(4.0, 22.0), Some(RackHit::MuteLed { row: 0 }));
        assert_eq!(at(26.0, 22.0), Some(RackHit::MuteLed { row: 0 }));
        assert_eq!(at(162.0, 22.0), Some(RackHit::PanKnob { row: 0 }));
        assert_eq!(at(196.0, 60.0), Some(RackHit::VolumeKnob { row: 1 }));
        assert_eq!(at(27.5, 22.0), None);
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

    // The rows' hit test finds no row under the last one: a press in the band under two rows
    // (y 84 + 14, where "+ Instrument…" now stands, which the rack takes itself), in an empty
    // rack, or over the steps there lands on no row; the canvas ends with the add row, 8 + 38 ·
    // rows + 24: 32 with none, 70 with one row, 108 with two (8, 46 and 84 without it; 46, 84
    // and 122 with the old 38 px "+" row).
    #[test]
    fn no_row_stands_under_the_last_row() {
        assert_eq!(
            hit_test(Point::new(80.0, 22.0 + 2.0 * 38.0), 2, 16, 4),
            None
        );
        assert_eq!(hit_test(Point::new(80.0, 22.0), 0, 16, 4), None);
        assert_eq!(hit_test(Point::new(236.0, 98.0), 2, 16, 4), None);
        let heights = [0, 1, 2].map(|rows| rack_size(rows, 16, 4).height);
        assert_eq!(heights, [32.0, 70.0, 108.0]);
    }

    // The header's controls, written out: the title "Channel Rack" from 12, 90.952 px at 14 px
    // (Inter's advances, as tests/fixtures/gen_u1_expected.py reads them: 14 × 13305 / 2048),
    // then, the title gap (48) past it, the "Swing" label at 150.952 (the knob and readout in the
    // swing's test, the readout to 264.952); the "Bars" label a group's gap (12) on, at
    // 276.952; "−" 30 on, 306.952..326.952, the count 326.952..358.952, "+" 358.952..378.952,
    // each 2..22 down the 24 px strip. The "Bars" label, 25.441 px at 12 px, ends 4.56 px
    // before "−"; "Swing", 34.875, ends 7.125 before its knob. The header ends left of step 16's
    // edge (222 + 16 · 30 + 4 · 6 = 726), so it never sits past the rack's 16 steps. Nothing in
    // it adds a channel: "+ Instrument…" stands under the rows. (Pins moved: "+ Instrument…"
    // stood first at 150.952..250.952, the swing's label at 262.952 and the bars' at 388.952,
    // until the button went under the rows; the bars stood first at 150.952, "−"
    // 180.952..200.952, the count 200.952..232.952, "+" 232.952..252.952, the add button at
    // 258..358, until they moved after the swing; before the title gap the label at 108, "−"
    // 140..160, the count 160..196, "+" 196..216.)
    #[test]
    fn header_clicks_land_on_the_control_drawn_there() {
        let near = |found: f32, wanted: f32| (found - wanted).abs() < 1e-3;
        assert!(near(swing_label_x(), 150.952), "{}", swing_label_x());
        assert!(near(bars_label_x(), 276.952), "{}", bars_label_x());
        let down = bars_down_rect();
        let count = bars_count_rect();
        let up = bars_up_rect();
        assert!(near(down.x, 306.952) && near(down.x + down.width, 326.952));
        assert!(near(count.x, 326.952) && near(count.x + count.width, 358.952));
        assert!(near(up.x, 358.952) && near(up.x + up.width, 378.952));
        assert!((up.y, up.height) == (2.0, 20.0));
        assert!(
            up.x + up.width < step_left(16, 4),
            "inside the rack's 16 steps"
        );
        let width = |words| crate::ui::text_fit::text_width(words, crate::ui::fonts::size::LABEL);
        let bars_end = bars_label_x() + width("Bars");
        assert!(bars_end < down.x, "the label before \"−\": {bars_end}");
        let swing_end = swing_label_x() + width("Swing");
        assert!(
            swing_end < swing_knob_rect().x,
            "the label before the knob: {swing_end}"
        );
        let at = |x, y| header_hit_test(Point::new(x, y));
        assert_eq!(at(312.0, 12.0), Some(HeaderHit::BarsDown));
        assert_eq!(at(368.0, 12.0), Some(HeaderHit::BarsUp));
        assert_eq!(at(342.0, 12.0), None, "the count takes no click");
        assert_eq!(at(285.0, 12.0), None, "the \"Bars\" label");
        assert_eq!(at(270.0, 12.0), None, "between the swing and the bars");
        assert_eq!(at(312.0, 1.0), None, "above the buttons");
        assert_eq!(at(40.0, 12.0), None, "the title");
        assert_eq!(at(130.0, 12.0), None, "the title's gap");
        assert_eq!(
            at(200.0, 12.0),
            Some(HeaderHit::Swing),
            "where the add button stood"
        );
    }

    // The swing, written out, from its label at 150.952: the knob 42 on,
    // 192.952..212.952 (its middle at 202.952, 12), the readout 66 on, 216.952..264.952, one hit
    // over both, 2..22 down the strip; the label takes no press. (Pins moved: the label stood at
    // 262.952 after "+ Instrument…", and at 370 before the bars moved after it; 36 px right of
    // that with the 12 px title gap.)
    #[test]
    fn the_swing_takes_presses_on_its_knob_and_its_readout() {
        let near = |found: f32, wanted: f32| (found - wanted).abs() < 1e-3;
        let at = |x, y| header_hit_test(Point::new(x, y));
        assert_eq!(
            at(194.0, 3.0),
            Some(HeaderHit::Swing),
            "the knob's top left"
        );
        assert_eq!(at(203.0, 12.0), Some(HeaderHit::Swing), "the knob's center");
        assert_eq!(
            at(214.0, 12.0),
            Some(HeaderHit::Swing),
            "between knob and readout"
        );
        assert_eq!(
            at(264.0, 21.0),
            Some(HeaderHit::Swing),
            "the readout's bottom right"
        );
        assert_eq!(at(192.0, 12.0), None, "left of the knob");
        assert_eq!(at(266.0, 12.0), None, "right of the readout");
        assert_eq!(at(240.0, 1.0), None, "above the readout");
        assert_eq!(at(240.0, 23.0), None, "under the readout");
        assert_eq!(at(160.0, 12.0), None, "the label");
        let knob = swing_knob_rect();
        assert!(near(knob.center().x, 202.952) && knob.center().y == 12.0);
        assert_eq!((knob.width, knob.height), (20.0, 20.0));
        let count = swing_count_rect();
        assert!(near(count.x, 216.952) && count.width == 48.0, "{count:?}");
    }

    // "+ Instrument…" under the rows, written out: the add row a row's gap under the last row,
    // at row_top(rows) = 8 + 38 · rows, 24 px tall; a press takes the words and 8 px either side
    // across the whole row, from the name boxes' x, 28. The words at the labels' 12 px, by
    // Inter's advances (gen_u1_expected.py's rule): + 1355, space 576, I 550, n 1210, s 1081,
    // t 670, r 771, u 1211, m 1794, e 1194, n 1210, t 670, … 1770 = 14062 of its 2048 units, and
    // its GPOS kerns no pair of them (the "…" has the text shaped with kerning), so
    // 12 · 14062 / 2048 = 82.395 px, 98.395 with the pad: x 28..126.395. With no rows at y 8, the
    // add row the rack's only row (8..32); with two at y 84 (8 + 76), 84..108; the rack's canvas
    // ends at the add row's foot (the test above). The rows' hit test finds nothing in the add
    // row: the rack takes its presses itself. (Pins moved: a button 100 x 20, 2 px down the
    // row, at (28, 10) and (28, 86).)
    #[test]
    fn the_add_words_stand_under_the_rows() {
        let near = |rect: Rectangle, (x, y, width): (f32, f32, f32)| {
            (rect.x, rect.y, rect.height) == (x, y, ADD_ROW_HEIGHT)
                && (rect.width - width).abs() < 1e-3
        };
        assert!(
            near(add_instrument_rect(0), (28.0, 8.0, 98.395)),
            "{:?}",
            add_instrument_rect(0)
        );
        let add = add_instrument_rect(2);
        assert!(near(add, (28.0, 84.0, 98.395)), "{add:?}");
        assert_eq!(add.x, NAME_BUTTON_X);
        assert_eq!(hit_test(Point::new(60.0, 96.0), 2, 16, 4), None);
        for (at, inside, what) in [
            (Point::new(60.0, 96.0), true, "the words"),
            (Point::new(29.0, 96.0), true, "1 px inside the left edge"),
            (Point::new(27.0, 96.0), false, "1 px outside the left edge"),
            (Point::new(126.0, 96.0), true, "inside the right edge"),
            (Point::new(127.0, 96.0), false, "past the right edge"),
            (Point::new(60.0, 83.0), false, "the gap over the row"),
            (Point::new(60.0, 107.0), true, "the row's foot"),
        ] {
            assert_eq!(add.contains(at), inside, "{what}: {at:?}");
        }
    }

    // The rows sit under the toolbar (64) and the header (24): step 1 of row 1 is centered on
    // window (236, 64 + 24 + 8 + 14) = (236, 110), step 16 of row 2 on (704, 148). (Pins
    // moved: 102 and 140 under the 56 px toolbar, 118 and 156 under the 72 px one.)
    #[test]
    fn the_rows_start_under_the_toolbar_and_the_header() {
        assert_eq!(
            rack_in_window(step_rect(0, 0, 4).center()),
            Point::new(236.0, 110.0)
        );
        assert_eq!(
            rack_in_window(step_rect(1, 15, 4).center()),
            Point::new(704.0, 148.0)
        );
    }

    // A channel's header is its light, name and knobs, x 0..218, and a control tall: row 1's
    // runs y 46..74. Between rows (y 74..84), over the steps (x 222) and under the last row
    // there is none.
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
        assert_eq!(at(80.0, 22.0 + 2.0 * 38.0), None, "under the last row");
    }

    #[test]
    fn rack_size_covers_every_step_and_row() {
        assert_eq!(
            rack_size(1, 16, 4),
            Size::new(704.0 + 14.0 + 12.0, 46.0 + 24.0)
        );
        assert_eq!(
            rack_size(0, 0, 4),
            Size::new(STEPS_X, RACK_PADDING_TOP + ADD_ROW_HEIGHT)
        );
    }
}
