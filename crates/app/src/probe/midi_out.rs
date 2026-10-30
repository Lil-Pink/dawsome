//! The MIDI OUT group's screenshot probe: an input channel's instrument window (p19_echo's
//! "Synth", its MIDI out set) as scripts/ui-check.sh shoots it beside a main window of 1000 and
//! of 1920 px, the instrument window 560 and 960 px wide. Positions come from the window's own
//! constants and colors from [`crate::ui::style`]; the rows' heights, which iced lays out on its
//! own, are found by walking down a column of the shot.

use iced::Point;

use super::tests::{read_ui_shot, rgb8};
use super::{near, Screenshot};
use crate::ui::instrument_layout::STRIP_HEIGHT;
use crate::ui::instrument_window::{HINT_HEIGHT, INPUT_LABEL, MIDI_CHANNEL_WIDTH, PADDING};
use crate::ui::style;

/// How far a button's or picker's face may be off its color.
const FACE: u8 = 6;

/// The runs of rows (logical, half a pixel apart, so a scale of 2 skips none) from `from` to
/// `to` at `x` whose color is within `tolerance` of `color`: each its first and last row.
fn runs(
    shot: &Screenshot,
    x: f32,
    (from, to): (f32, f32),
    color: [u8; 3],
    tolerance: u8,
) -> Vec<(f32, f32)> {
    let mut found: Vec<(f32, f32)> = Vec::new();
    let mut y = from;
    while y < to {
        if near(shot.pixel(Point::new(x, y)), color, tolerance) {
            match found.last_mut() {
                Some(run) if y - run.1 <= 0.5 => run.1 = y,
                _ => found.push((y, y)),
            }
        }
        y += 0.5;
    }
    found
}

/// The middle of a run.
fn middle(run: (f32, f32)) -> f32 {
    (run.0 + run.1) / 2.0
}

// The group shows under the input's rows and lines up with them: its accent rule at the right
// edge, under it the Port picker filling the row to the padding and no further, the Channel
// picker under the Port picker after the same column of labels and MIDI_CHANNEL_WIDTH wide,
// Echo ending the latency row at the right edge, and the footer's Close under it, all above the
// foot line.
#[test]
#[ignore = "needs a screenshot of p19_echo's instrument window (--open-instrument channel:1 --screenshot-window instrument --midi-out-ports ...): scripts/ui-check.sh takes two and runs this"]
fn m1_the_midi_out_group_lines_up_under_the_input() {
    let shot = read_ui_shot();
    let (width, height) = (
        shot.width as f32 / shot.scale,
        shot.height as f32 / shot.scale,
    );
    let foot = height - HINT_HEIGHT;
    let right = width - PADDING - 3.0;
    // Under the channel's name (15 px words) and its strip: nothing of the group is higher.
    let top = PADDING + 15.0 + STRIP_HEIGHT;
    let rule = runs(&shot, right, (top, foot), rgb8(style::ACCENT), 12);
    let &(_, rule_end) = rule
        .first()
        .expect("the MIDI OUT group's accent rule at the window's right edge");
    let face = rgb8(style::BUTTON);
    let edge = runs(&shot, right, (rule_end + 1.0, foot), face, FACE);
    println!("width {width}, rule {rule:?}, faces at the right edge {edge:?}");
    let [port, echo, close] = edge[..] else {
        panic!("the Port picker, Echo and Close at the right edge under the rule: {edge:?}");
    };
    assert!(
        port.0 - rule_end < 20.0,
        "the Port picker right under the rule"
    );
    assert!(
        port.1 < echo.0 && echo.1 < close.0,
        "Port, then Echo, then Close"
    );
    let past = shot.pixel(Point::new(width - PADDING / 2.0, middle(port)));
    assert!(
        !near(past, face, FACE),
        "the Port picker runs into the padding: {past:02X?}"
    );
    let column = PADDING + INPUT_LABEL + 6.0 + 3.0;
    let pickers = runs(&shot, column, (rule_end + 1.0, echo.0), face, FACE);
    println!("faces at the labels' column {pickers:?}");
    let [first, channel] = pickers[..] else {
        panic!("the Port and Channel pickers after the labels' column: {pickers:?}");
    };
    assert!(
        (first.0 - port.0).abs() < 1.0,
        "the Port picker is one face from the labels' column to the right edge"
    );
    let before = shot.pixel(Point::new(column - 6.0, middle(channel)));
    assert!(
        !near(before, face, FACE),
        "the Channel picker starts after the labels"
    );
    let after = shot.pixel(Point::new(
        column - 3.0 + MIDI_CHANNEL_WIDTH + 3.0,
        middle(channel),
    ));
    assert!(
        !near(after, face, FACE),
        "the Channel picker is {MIDI_CHANNEL_WIDTH} px wide"
    );
    assert!(
        close.1 < foot,
        "the group and the footer above the foot line"
    );
}
