//! The timeline grid's rules, shared by the playlist and the piano roll so the two panels zoom,
//! scroll and draw their time alike: a zoom kept to a range, halved and doubled by a button and
//! moved a quarter of a doubling by a wheel notch ([`ZoomRange`]); lines that fade in as they
//! grow apart ([`line_alpha`]); a stronger line every four bars ([`strong`]); every other bar
//! shaded ([`shaded`]); a scroll that comes to rest on a whole cell ([`cell`], [`landed`]); a
//! zoom that keeps what is under the cursor there ([`anchored_scroll`]); lines on whole device
//! pixels ([`device_line`]); and the ruler's bar numbers ([`draw_bar_numbers`]). Each panel
//! keeps its own units and its own mapping to pixels (the playlist pixels per bar, the roll
//! pixels per step) and calls these with them.

use std::ops::RangeInclusive;

use dawsome_engine::time::{Ticks, BEATS_PER_BAR};
use iced::alignment::Vertical;
use iced::widget::canvas::{Frame, Text};
use iced::{Point, Size};

use crate::ui::fonts::size;
use crate::ui::style;

/// Ctrl+wheel notches to a doubling of the zoom.
pub const NOTCHES_PER_DOUBLING: f32 = 4.0;

/// Pixels of timeline Shift+wheel (or a wheel across) scrolls per notch, at any zoom.
pub const PIXELS_PER_NOTCH: f32 = 64.0;

/// Lines show from this many pixels apart on, fainter the closer together they are, and fully
/// from [`LINE_FADE_FROM`] + [`LINE_FADE_SPAN`] = 19 px apart ([`line_alpha`]). Why a fade:
/// lines that vanished at one zoom stop made zooming jump, and lines a few pixels apart are
/// only noise.
pub const LINE_FADE_FROM: f32 = 5.0;
pub const LINE_FADE_SPAN: f32 = 14.0;

/// Bars counted in fours, from bar 1: the line starting each four (bars 1, 5, 9, …) is drawn
/// stronger, and below [`SHADE_FOURS_BELOW`] the shading alternates by fours of bars.
pub const BARS_PER_GROUP: i64 = 4;
/// Pixels per bar under which every other four bars is shaded instead of every other bar
/// ([`shaded`]). Why (the design chosen): a bar this narrow shaded alone reads as a thin stripe,
/// four of them together as a block.
pub const SHADE_FOURS_BELOW: f32 = 32.0;

/// The line right of every strong bar line ([`strong`]): the bar line's color at this alpha
/// over what it lies on, so the line reads as a double, heavier one.
pub const STRONG_COMPANION_ALPHA: f32 = 0.45;

/// The ruler's band of bar numbers: its height, how far right of its bar line a number starts,
/// and the numbers' size.
pub const NUMBER_BAND_HEIGHT: f32 = 16.0;
pub const NUMBER_INSET: f32 = 3.0;
/// The tick under each numbered bar, at the band's foot (the piano roll's ruler draws a tick of
/// this height at each beat too).
pub const NUMBER_TICK_HEIGHT: f32 = 4.0;
/// Pixels a bar number needs per digit, plus a margin, so numbers never run into each other.
const NUMBER_DIGIT_WIDTH: f32 = 7.0;
const NUMBER_MARGIN: f32 = 6.0;

/// A panel's zoom, in its own pixels per unit (the playlist's per bar, the roll's per step):
/// the farthest out, the closest in, and where it starts. Any value between the ends shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoomRange {
    pub min: f32,
    pub max: f32,
    pub default: f32,
}

impl ZoomRange {
    /// `px` held to the ends; a value that is no number (a settings file's) is the default.
    pub fn kept(self, px: f32) -> f32 {
        if px.is_finite() {
            px.clamp(self.min, self.max)
        } else {
            self.default
        }
    }

    /// The zoom − (`closer` false) or + leads to from `px`: half or twice it, kept.
    pub fn stepped(self, px: f32, closer: bool) -> f32 {
        self.kept(if closer { px * 2.0 } else { px / 2.0 })
    }

    /// The zoom one Ctrl+wheel notch leads to from `px`: 2^(1/4) times closer or farther, kept,
    /// so four notches double or halve it. Counted in quarter-doublings from the farthest out
    /// and raised again rather than multiplied on. Why: four notches from the playlist's 64 then
    /// land on 128 exactly, not on 127.99999, which an octave further out would sit under
    /// [`SHADE_FOURS_BELOW`] and shade otherwise than the 32 the − button gives. A zoom off those
    /// steps (a settings file's) takes the nearest one first.
    pub fn wheeled(self, px: f32, closer: bool) -> f32 {
        let steps = (NOTCHES_PER_DOUBLING * (self.kept(px) / self.min).log2()).round();
        let steps = steps + if closer { 1.0 } else { -1.0 };
        self.kept(self.min * (steps / NOTCHES_PER_DOUBLING).exp2())
    }
}

/// How strongly lines `apart` pixels apart show, 0 (not drawn) to 1: (apart − 5)/14, kept to
/// 0..1.
pub fn line_alpha(apart: f32) -> f32 {
    ((apart - LINE_FADE_FROM) / LINE_FADE_SPAN).clamp(0.0, 1.0)
}

/// How strongly beat lines show at `px_per_bar`: [`line_alpha`] of a beat's width.
pub fn beat_alpha(px_per_bar: f32) -> f32 {
    line_alpha(px_per_bar / f32::from(BEATS_PER_BAR))
}

/// Whether bar `index` (from 0: bar 1) is shaded at `px_per_bar`: every other bar from bar 1
/// (1, 3, 5, …), or below [`SHADE_FOURS_BELOW`] every other four bars from bar 1 (1-4, 9-12, …).
/// Why shade: the bars then read as blocks however faint their lines are.
pub fn shaded(index: i64, px_per_bar: f32) -> bool {
    index.div_euclid(shade_group(px_per_bar)).rem_euclid(2) == 0
}

/// The bars the shading alternates by at `px_per_bar`: one, or four far out.
fn shade_group(px_per_bar: f32) -> i64 {
    if px_per_bar < SHADE_FOURS_BELOW {
        BARS_PER_GROUP
    } else {
        1
    }
}

/// The shaded bars ([`shaded`]; fours of them far out) from the one holding tick `from` to the
/// one holding tick `to`, each as the ticks it starts and ends at, left to right.
pub fn shaded_ticks(
    from: Ticks,
    to: Ticks,
    bar: Ticks,
    px_per_bar: f32,
) -> impl Iterator<Item = (Ticks, Ticks)> {
    let bar = bar.0.max(1);
    let group = shade_group(px_per_bar);
    let first = from.0.div_euclid(bar).div_euclid(group);
    let last = to.0.div_euclid(bar).div_euclid(group);
    (first..=last)
        .filter(move |&index| shaded(index * group, px_per_bar))
        .map(move |index| (Ticks(index * group * bar), Ticks((index + 1) * group * bar)))
}

/// Whether bar `index`'s line (from 0: bar 1) is a strong one, starting a four: bars 1, 5, 9, …
pub fn strong(index: i64) -> bool {
    index.rem_euclid(BARS_PER_GROUP) == 0
}

/// The bars, counted from 0 (bar 1), whose line lies from tick `from` to tick `to`.
pub fn bar_lines(from: Ticks, to: Ticks, bar: Ticks) -> RangeInclusive<i64> {
    let bar = bar.0.max(1);
    let first = from.0.div_euclid(bar) + i64::from(from.0.rem_euclid(bar) != 0);
    first..=to.0.div_euclid(bar)
}

/// The cell a scroll along time comes to rest on: a `beat` while beat lines are drawn at
/// `px_per_bar` ([`beat_alpha`] above 0), else a `bar`; at least a tick.
pub fn cell(beat: Ticks, bar: Ticks, px_per_bar: f32) -> Ticks {
    if beat_alpha(px_per_bar) > 0.0 {
        Ticks(beat.0.max(1))
    } else {
        Ticks(bar.0.max(1))
    }
}

/// `tick` moved to the nearest start of a `cell` (halfway goes on): where a scroll along time
/// comes to rest, so the view starts on a whole beat or bar, never on a sliver of one.
pub fn landed(tick: Ticks, cell: Ticks) -> Ticks {
    let cell = cell.0.max(1);
    Ticks((tick.0 + cell / 2).div_euclid(cell) * cell)
}

/// Where a scroll from `from` towards `to` comes to rest: [`landed`], unless that is no further
/// along than where it started, then the next cell start that way. Why: a move shorter than
/// half a cell (a wheel notch over the bar of a narrow view) would land back where it was, and
/// the view could not be moved at all.
pub fn landed_towards(from: Ticks, to: Ticks, cell: Ticks) -> Ticks {
    let (from, size) = (from.0, cell.0.max(1));
    let landed = landed(to, cell);
    if to.0 > from && landed.0 <= from {
        Ticks((from.div_euclid(size) + 1) * size)
    } else if to.0 < from && landed.0 >= from {
        Ticks((((from + size - 1).div_euclid(size) - 1) * size).max(0))
    } else {
        landed
    }
}

/// The start of the `cell` holding `tick`.
pub fn cell_start(tick: Ticks, cell: Ticks) -> Ticks {
    let cell = cell.0.max(1);
    Ticks(tick.0.div_euclid(cell) * cell)
}

/// The scroll that keeps what shows `from_left` pixels into the timeline there when the zoom
/// goes from `before` to `after` pixels per `unit` ticks (the panel's bar or step): to the
/// nearest whole tick, and never before the start.
pub fn anchored_scroll(scroll: Ticks, from_left: f64, unit: f64, before: f32, after: f32) -> Ticks {
    let under = scroll.0 as f64 + from_left * unit / f64::from(before);
    let scroll = under - from_left * unit / f64::from(after);
    Ticks(scroll.round().max(0.0) as i64)
}

/// The first multiple of `unit` (at least 1) at or after `tick`.
pub fn first_at_or_after(tick: i64, unit: i64) -> i64 {
    let unit = unit.max(1);
    (tick + unit - 1).div_euclid(unit) * unit
}

/// A 1 px line at logical x, as drawn: its left edge moved down onto a device pixel, and one
/// device pixel per whole scale step wide (2 at scale 1.7), so it covers whole pixels.
///
/// Why: lines a logical pixel wide at a fractional scale straddle device pixels, and the
/// renderer's multisampling then blends them into the lane color, lighter or darker from line
/// to line. Lines on whole device pixels look the same everywhere, in the window and in a
/// screenshot.
pub fn device_line(x: f32, scale_factor: f32) -> (f32, f32) {
    let scale = if scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let left = (x * scale).floor() / scale;
    let width = scale.round().max(1.0) / scale;
    (left, width)
}

/// Every how many bars a number is written at `px_per_bar`, so that numbers up to `largest` fit
/// between their lines: every bar when there is room, else every 2nd, 4th, ...
pub fn label_interval(px_per_bar: f32, largest: i64) -> i64 {
    let digits = largest.max(1).to_string().len() as f32;
    let needed = digits * NUMBER_DIGIT_WIDTH + NUMBER_MARGIN;
    let mut every = 1;
    while (every as f32) * px_per_bar < needed {
        every *= 2;
    }
    every
}

/// The ruler's bar numbers in a band [`NUMBER_BAND_HEIGHT`] tall from the frame's top, for the
/// `bars` (number counted from 1, line x) in view at `px_per_bar`: a number and a tick under it
/// every [`label_interval`]th bar, the numbers of bars that start a four (1, 5, 9, …, over the
/// grid's strong lines) bright, the others dim.
pub fn draw_bar_numbers(
    frame: &mut Frame,
    bars: &[(i64, f32)],
    px_per_bar: f32,
    scale_factor: f32,
) {
    let every = label_interval(px_per_bar, bars.last().map_or(1, |&(number, _)| number));
    for &(number, x) in bars
        .iter()
        .filter(|&&(number, _)| (number - 1) % every == 0)
    {
        let (line, width) = device_line(x, scale_factor);
        frame.fill_rectangle(
            Point::new(line, NUMBER_BAND_HEIGHT - NUMBER_TICK_HEIGHT),
            Size::new(width, NUMBER_TICK_HEIGHT),
            style::palette().lane_bar_line,
        );
        let color = if strong(number - 1) {
            style::palette().text
        } else {
            style::palette().text_secondary
        };
        frame.fill_text(Text {
            content: number.to_string(),
            position: Point::new(x + NUMBER_INSET, NUMBER_BAND_HEIGHT / 2.0),
            color,
            size: size::DENSE.into(),
            align_y: Vertical::Center,
            ..Text::default()
        });
    }
}
