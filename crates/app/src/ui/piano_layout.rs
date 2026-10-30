//! Every position in the piano roll, in logical pixels. The grid and lane canvases draw from
//! these values, the gestures map the cursor with them, and the screenshot probe samples the
//! points they give, so the drawing and its checks cannot drift apart.
//!
//! The panel, top to bottom: the strip ([`STRIP_HEIGHT`]), the grid (the rest), the divider
//! ([`DIVIDER_HEIGHT`]: the grid's scroll bar along time, right of the key column, and the
//! lane's drag handle) and the velocity lane ([`LANE_HEIGHT`] tall until the handle is dragged,
//! then anywhere from [`LANE_MIN_HEIGHT`] to [`LANE_MAX_HEIGHT`]).
//!
//! Grid coordinates start at the grid canvas's top-left corner. The key column is x 0..48 and
//! the timeline starts at x 48: tick t is at x = 48 + (t − scroll)·px_per_step/step_ticks. The
//! row of key k spans y = (top − k)·12 .. +12, where `top` is the key of the top row. Lane
//! coordinates start at the lane's top-left corner and share the grid's x; a lane h tall shows
//! velocity 128 at its top and 0 at its bottom.
//!
//! Probe coordinates (window, logical, piano roll alone in a 1000x560 window, p3_roll, the lane
//! at its default height): the grid starts at y 84 and the lane at y 496; the top key is 69, so
//! key 60's row is y 192..204.

use dawsome_engine::project::{Note, MAX_KEY};
use dawsome_engine::time::{Ticks, BEATS_PER_BAR};
use iced::{Point, Rectangle, Size};

use crate::ui::layout::TOOLBAR_HEIGHT;
use crate::ui::scroll::{self, Span};

/// The strip of controls above the grid.
pub const STRIP_HEIGHT: f32 = 28.0;
/// The lane's drag handle, between the grid and the velocity lane: dragged up it makes the
/// lane taller, down shorter.
pub const HANDLE_HEIGHT: f32 = 6.0;
/// What lies between the grid and the velocity lane: the grid's scroll bar and the lane's drag
/// handle.
pub const DIVIDER_HEIGHT: f32 = scroll::BAR_HEIGHT + HANDLE_HEIGHT;
/// The velocity lane's height until its handle is dragged: half the 128 px it had, so the grid
/// shows more rows.
pub const LANE_HEIGHT: f32 = 64.0;
/// The most the divider and the lane take under the grid: 136 px, what they took before the
/// lane halved. Why fixed: the piano roll's least and natural heights keep their values, so the
/// grid takes the room the lane gave up rather than the panels around the roll.
pub const UNDER_GRID: f32 = 136.0;
/// The tallest and the shortest the handle drags the lane. At its tallest the lane takes all of
/// [`UNDER_GRID`] the divider leaves, so the roll's least height still keeps its grid rows.
pub const LANE_MAX_HEIGHT: f32 = UNDER_GRID - DIVIDER_HEIGHT;
pub const LANE_MIN_HEIGHT: f32 = 32.0;

pub const KEY_COLUMN_WIDTH: f32 = 48.0;
pub const ROW_HEIGHT: f32 = 12.0;
/// C labels end at this x, right-aligned.
pub const LABEL_RIGHT: f32 = 44.0;
pub const LABEL_SIZE: f32 = 9.0;

/// Pixels per step at each zoom level, from farthest out to closest in.
pub const ZOOM_LEVELS: [f32; 9] = [4.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0];
/// 16 pixels per step.
pub const DEFAULT_ZOOM: usize = 4;

/// A press this close to a note's right edge resizes the note, if the note is at least
/// [`RESIZE_MIN_WIDTH`] wide; anywhere else on it moves it.
pub const RESIZE_EDGE: f32 = 4.0;
pub const RESIZE_MIN_WIDTH: f32 = 12.0;

/// Width of a velocity stem, drawn from its note's start.
pub const STEM_WIDTH: f32 = 4.0;
/// A lane press picks the stem whose center is at most this far away.
pub const STEM_PICK_DISTANCE: f32 = 6.0;

pub const LINE_WIDTH: f32 = 1.0;
pub const BAR_LINE_WIDTH: f32 = 2.0;
pub const PLAYHEAD_WIDTH: f32 = 2.0;
pub const NOTE_BORDER: f32 = 1.0;
pub const SELECTED_BORDER: f32 = 2.0;

/// How far the view reaches past the pattern's end.
pub const BARS_PAST_END: i64 = 4;

/// The keys the grid has rows for: every key, or those a fold keeps,
/// one bit a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rows(u128);

impl Default for Rows {
    fn default() -> Self {
        Rows::ALL
    }
}

impl Rows {
    /// Every key has a row.
    pub const ALL: Rows = Rows(u128::MAX);

    /// Rows for `keys` alone (keys past 127 are passed over); every key when that is none.
    pub fn of(keys: impl IntoIterator<Item = u8>) -> Rows {
        let bits = keys
            .into_iter()
            .filter(|&key| key <= MAX_KEY)
            .fold(0u128, |bits, key| bits | 1 << key);
        if bits == 0 {
            Rows::ALL
        } else {
            Rows(bits)
        }
    }

    /// Whether `key` has a row.
    pub fn shows(self, key: u8) -> bool {
        key <= MAX_KEY && self.0 & 1 << key != 0
    }

    /// The keys with rows, highest first.
    fn keys_down(self) -> impl Iterator<Item = u8> {
        (0..=MAX_KEY).rev().filter(move |&key| self.shows(key))
    }

    /// How many keys have rows in `low..=high`.
    fn count(self, low: u8, high: u8) -> i32 {
        (low..=high).filter(|&key| self.shows(key)).count() as i32
    }
}

/// The part of the timeline and keyboard the grid shows, and how ticks map to pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View {
    /// The key whose row is at the top of the grid.
    pub top: u8,
    /// The tick at the timeline's left edge (x = [`KEY_COLUMN_WIDTH`]); never negative.
    pub scroll: Ticks,
    /// Index into [`ZOOM_LEVELS`].
    pub zoom: usize,
    /// Ticks in one step (ppq / 4), at least 1.
    pub step: Ticks,
    /// The keys with rows: every key unless folded.
    pub rows: Rows,
}

impl View {
    pub fn px_per_step(&self) -> f32 {
        ZOOM_LEVELS[self.zoom.min(ZOOM_LEVELS.len() - 1)]
    }

    /// Ticks in one beat and in one bar.
    pub fn beat(&self) -> Ticks {
        Ticks(self.step.0 * 4)
    }

    pub fn bar(&self) -> Ticks {
        Ticks(self.beat().0 * i64::from(BEATS_PER_BAR))
    }

    /// The finest grid line, which follows the zoom: half a step from 32 pixels per step,
    /// a step from 8, a beat below that.
    pub fn line_unit(&self) -> Ticks {
        let pixels = self.px_per_step();
        if pixels >= 32.0 {
            Ticks((self.step.0 / 2).max(1))
        } else if pixels >= 8.0 {
            self.step
        } else {
            self.beat()
        }
    }

    /// The grid x where `tick` starts.
    pub fn x_of(&self, tick: Ticks) -> f32 {
        let ticks = (tick.0 - self.scroll.0) as f64;
        let x = f64::from(KEY_COLUMN_WIDTH) + ticks * f64::from(self.px_per_step()) / self.ticks();
        x as f32
    }

    /// The tick under grid x: floor((x − 48)·step_ticks / px_per_step) + scroll.
    pub fn tick_at(&self, x: f32) -> Ticks {
        let ticks = (f64::from(x) - f64::from(KEY_COLUMN_WIDTH)) * self.ticks()
            / f64::from(self.px_per_step());
        Ticks(ticks.floor() as i64 + self.scroll.0)
    }

    /// The tick nearest grid x: round((x − 48)·step_ticks / px_per_step) + scroll. Where a
    /// point stands for a tick (a stem's middle less half its width), so a tick read back from
    /// its own x is itself.
    pub fn tick_nearest(&self, x: f32) -> Ticks {
        let ticks = (f64::from(x) - f64::from(KEY_COLUMN_WIDTH)) * self.ticks()
            / f64::from(self.px_per_step());
        Ticks(ticks.round() as i64 + self.scroll.0)
    }

    /// The grid y of the top of `key`'s row. Folded, rows count only the keys that have them,
    /// and a key without one is where its row would be, under the next key above with one.
    pub fn row_top(&self, key: u8) -> f32 {
        if self.rows == Rows::ALL {
            return f32::from(i16::from(self.top) - i16::from(key)) * ROW_HEIGHT;
        }
        let rows = match key.cmp(&self.top) {
            std::cmp::Ordering::Equal => 0,
            // Below the top: the rows of the keys from just above it up to the top.
            std::cmp::Ordering::Less => self.rows.count(key + 1, self.top),
            std::cmp::Ordering::Greater => -self.rows.count(self.top + 1, key),
        };
        rows as f32 * ROW_HEIGHT
    }

    /// The key whose row holds grid y. Above the top of the keyboard or below its bottom this
    /// is outside 0..=127.
    pub fn key_at(&self, y: f32) -> i32 {
        let row = (y / ROW_HEIGHT).floor() as i32;
        if self.rows == Rows::ALL {
            return i32::from(self.top) - row;
        }
        let top = i32::from(self.top);
        if row >= 0 {
            let below: Vec<u8> = self
                .rows
                .keys_down()
                .filter(|&key| i32::from(key) <= top)
                .collect();
            match below.get(row as usize) {
                Some(&key) => i32::from(key),
                None => -1 - (row - below.len() as i32),
            }
        } else {
            let above: Vec<u8> = self
                .rows
                .keys_down()
                .filter(|&key| i32::from(key) > top)
                .collect();
            let wanted = (-row) as usize;
            match above.len().checked_sub(wanted) {
                Some(index) => i32::from(above[index]),
                None => i32::from(MAX_KEY) + (wanted - above.len()) as i32,
            }
        }
    }

    /// `top` kept where the keyboard fills a grid `grid_height` tall ([`clamp_top`]); folded,
    /// on a key with a row, no lower than the one that puts the last such key on the last
    /// whole row, and the highest when they all fit.
    pub fn clamp_top(&self, top: i32, grid_height: f32) -> u8 {
        if self.rows == Rows::ALL {
            return clamp_top(top, grid_height);
        }
        let keys: Vec<u8> = self.rows.keys_down().collect();
        let fit = usize::from(visible_rows(grid_height)).max(1);
        let lowest = keys.len().saturating_sub(fit);
        let wanted = keys
            .iter()
            .position(|&key| i32::from(key) <= top)
            .unwrap_or(keys.len().saturating_sub(1));
        keys.get(wanted.min(lowest)).copied().unwrap_or(MAX_KEY)
    }

    /// The top moved `rows` rows up (or down), kept by [`View::clamp_top`].
    pub fn top_moved(&self, rows: i32, grid_height: f32) -> u8 {
        if self.rows == Rows::ALL {
            return clamp_top(i32::from(self.top) + rows, grid_height);
        }
        let keys: Vec<u8> = self.rows.keys_down().collect();
        let at = keys.iter().position(|&key| key <= self.top).unwrap_or(0) as i32;
        let moved = (at - rows).clamp(0, keys.len() as i32 - 1) as usize;
        self.clamp_top(i32::from(keys[moved]), grid_height)
    }

    /// The note as drawn: from its start to its drawn end, its row less the bottom pixel.
    pub fn note_rect(&self, note: &Note) -> Rectangle {
        let left = self.x_of(note.start);
        let right = self.x_of(Ticks(note.start.0 + drawn_length(note, self.step).0));
        Rectangle::new(
            Point::new(left, self.row_top(note.key)),
            Size::new(right - left, ROW_HEIGHT - 1.0),
        )
    }

    /// The velocity stem of `note` in a lane `lane` pixels tall, lane coordinates:
    /// [`STEM_WIDTH`] wide from the note's start, `steps`/128 of the lane tall from its bottom.
    pub fn stem_rect(&self, note: &Note, steps: u8, lane: f32) -> Rectangle {
        let height = f32::from(steps) * lane / VELOCITY_STEPS;
        Rectangle::new(
            Point::new(self.x_of(note.start), lane - height),
            Size::new(STEM_WIDTH, height),
        )
    }

    fn ticks(&self) -> f64 {
        self.step.0.max(1) as f64
    }
}

/// How long a note is drawn: its length, or a step for a zero-length (step) note.
pub fn drawn_length(note: &Note, step: Ticks) -> Ticks {
    if note.length.0 > 0 {
        note.length
    } else {
        step
    }
}

/// What part of a note a grid press landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotePart {
    Body,
    /// The resize handle along its right edge.
    RightEdge,
}

/// The note under grid point (`x`, `y`): the last of `notes` (the one drawn on top) whose
/// columns and row hold the point, and which part of it.
pub fn note_under<'a>(
    view: &View,
    notes: impl DoubleEndedIterator<Item = &'a Note>,
    x: f32,
    y: f32,
) -> Option<(&'a Note, NotePart)> {
    let key = view.key_at(y);
    notes.rev().find_map(|note| {
        let rect = view.note_rect(note);
        let inside = i32::from(note.key) == key && (rect.x..rect.x + rect.width).contains(&x);
        inside.then(|| {
            let edge = rect.width >= RESIZE_MIN_WIDTH && rect.x + rect.width - x <= RESIZE_EDGE;
            (
                note,
                if edge {
                    NotePart::RightEdge
                } else {
                    NotePart::Body
                },
            )
        })
    })
}

/// Whether `note`'s drawn rectangle meets the box between grid points `a` and `b`.
pub fn meets_box(view: &View, note: &Note, a: Point, b: Point) -> bool {
    let rect = view.note_rect(note);
    let (left, right) = (a.x.min(b.x), a.x.max(b.x));
    let (top, bottom) = (a.y.min(b.y), a.y.max(b.y));
    rect.x <= right
        && left <= rect.x + rect.width
        && rect.y <= bottom
        && top <= rect.y + rect.height
}

/// The stem nearest lane x among `notes` within [`STEM_PICK_DISTANCE`] of its center; the
/// leftmost on a tie.
pub fn stem_under<'a>(
    view: &View,
    notes: impl Iterator<Item = &'a Note>,
    x: f32,
) -> Option<&'a Note> {
    let distance = |note: &Note| (view.x_of(note.start) + STEM_WIDTH / 2.0 - x).abs();
    notes
        .filter(|note| distance(note) <= STEM_PICK_DISTANCE)
        .min_by(|a, b| {
            let nearer = distance(a).total_cmp(&distance(b));
            nearer.then(a.start.cmp(&b.start))
        })
}

/// The top of the velocity lane's scale: 128 steps, the scale the stems and notes use.
const VELOCITY_STEPS: f32 = 128.0;

/// The velocity a lane press or drag at lane y sets in a lane `lane` pixels tall, on the 0..=128
/// scale: 128 at the lane's top, 0 at its bottom, rounded and clamped.
pub fn lane_steps(y: f32, lane: f32) -> u8 {
    let lane = lane.max(1.0);
    // Clamped into 0..=128 first, so the conversion is exact.
    ((lane - y) * VELOCITY_STEPS / lane)
        .round()
        .clamp(0.0, VELOCITY_STEPS) as u8
}

/// A lane height kept to what the handle drags it to: [`LANE_MIN_HEIGHT`] to
/// [`LANE_MAX_HEIGHT`], the default for a value that is no number.
pub fn kept_lane(height: f32) -> f32 {
    if height.is_finite() {
        height.clamp(LANE_MIN_HEIGHT, LANE_MAX_HEIGHT)
    } else {
        LANE_HEIGHT
    }
}

/// The lane's height while its handle, pressed at window y `from` with the lane `start` tall,
/// is at window y `to`: taller by what the handle went up, kept by [`kept_lane`].
pub fn dragged_lane(start: f32, from: f32, to: f32) -> f32 {
    kept_lane(start + from - to)
}

/// Whole rows that fit in a grid `height` pixels tall, at most the 128 keys.
pub fn visible_rows(height: f32) -> u8 {
    (height / ROW_HEIGHT).floor().clamp(0.0, 128.0) as u8
}

/// `top` kept where the keyboard fills the grid: no higher than key 127, and high enough that
/// key 0 is not above the grid's last whole row.
pub fn clamp_top(top: i32, grid_height: f32) -> u8 {
    let lowest = (i32::from(visible_rows(grid_height)) - 1).max(0);
    let highest = i32::from(MAX_KEY);
    // Both bounds lie in 0..=127, so the clamped key fits a u8.
    top.clamp(lowest.min(highest), highest) as u8
}

/// The largest scroll that keeps the view inside the pattern plus [`BARS_PAST_END`] bars.
pub fn max_scroll(view: &View, pattern_end: Ticks, grid_width: f32) -> Ticks {
    let view_end = pattern_end.0 + BARS_PAST_END * view.bar().0;
    Ticks((view_end - visible_ticks(view, grid_width)).max(0))
}

/// The whole ticks the timeline of a grid `grid_width` wide shows.
fn visible_ticks(view: &View, grid_width: f32) -> i64 {
    let timeline = f64::from((grid_width - KEY_COLUMN_WIDTH).max(0.0));
    (timeline * view.ticks() / f64::from(view.px_per_step())).floor() as i64
}

/// The grid's scroll along time, in ticks, for notes that end by `end`: the content reaches
/// [`BARS_PAST_END`] bars past it (where [`max_scroll`] stops), the view is what the timeline
/// shows.
pub fn time_span(view: &View, end: Ticks, grid_width: f32) -> Span {
    Span {
        content: (end.0 + BARS_PAST_END * view.bar().0) as f64,
        offset: view.scroll.0 as f64,
        view: visible_ticks(view, grid_width) as f64,
    }
}

/// The grid canvas's size when the piano roll is the only panel in a window of `window` size,
/// its lane at the default height.
pub fn grid_size_alone(window: Size) -> Size {
    let height = window.height - TOOLBAR_HEIGHT - STRIP_HEIGHT - DIVIDER_HEIGHT - LANE_HEIGHT;
    Size::new(window.width, height.max(0.0))
}

/// The rack's height above an open piano roll: its content, up to half the panel area.
pub fn rack_height_above_roll(content: f32, area: f32) -> f32 {
    content.min(area / 2.0)
}

#[cfg(test)]
mod tests {
    use dawsome_engine::project::ChannelId;

    use super::*;

    /// p3_roll as the piano roll opens on it: top key 69, no scroll, 16 px per step, ppq 96.
    fn p3_view() -> View {
        View {
            top: 69,
            scroll: Ticks(0),
            zoom: DEFAULT_ZOOM,
            step: Ticks(24),
            rows: crate::ui::piano_layout::Rows::ALL,
        }
    }

    fn note(tick: i64, length: i64, key: u8) -> Note {
        Note {
            channel: ChannelId(1),
            start: Ticks(tick),
            length: Ticks(length),
            velocity: 0.5,
            key,
        }
    }

    // The design's geometry: key 60's row is y 108..120 and key 64's 60..72; A (tick 0, 48 ticks)
    // spans x 48..80 and B (tick 96, 96 ticks) 112..176; tick 384 (the pattern end) is x 304.
    #[test]
    fn p3_roll_lands_where_the_design_draws_it() {
        let view = p3_view();
        assert_eq!(view.row_top(60), 108.0);
        assert_eq!(view.row_top(64), 60.0);
        let a = view.note_rect(&note(0, 48, 60));
        assert_eq!(
            (a.x, a.x + a.width, a.y, a.height),
            (48.0, 80.0, 108.0, 11.0)
        );
        let b = view.note_rect(&note(96, 96, 64));
        assert_eq!((b.x, b.x + b.width), (112.0, 176.0));
        assert_eq!(view.x_of(Ticks(384)), 304.0);
        // A zero-length (step) note is drawn one step long: the kick at 192 spans 176..192.
        let kick = view.note_rect(&note(192, 0, 60));
        assert_eq!((kick.x, kick.width), (176.0, 16.0));
        // Alone in a 1000x560 window the grid is 388 tall: 560 less the toolbar (64), the
        // strip (28), the grid's scroll bar (10), the lane's handle (6) and the lane (64).
        // (Pin moved: 340 under an 8 px divider and a 128 px lane; the half lane and handle
        // made it 406, the scroll bar 396, the 72 px toolbar 380, the 64 px one 388.)
        assert_eq!(
            grid_size_alone(Size::new(1000.0, 560.0)),
            Size::new(1000.0, 388.0)
        );
    }

    // tick_at(x) = floor((x − 48)·24/pxs) + scroll. At 6 px per step a pixel is 4 ticks; at 64 px
    // per step it is 0.375 ticks (gen_p3_expected.py prints the same values).
    #[test]
    fn tick_at_floors_the_ticks_under_a_pixel() {
        let at = |zoom: usize, x: f32| View { zoom, ..p3_view() }.tick_at(x).0;
        let six = 1; // ZOOM_LEVELS[1] = 6
        let sixty_four = 8;
        assert_eq!([48.0, 49.0, 50.0, 73.0].map(|x| at(six, x)), [0, 4, 8, 100]);
        assert_eq!(
            [48.0, 50.0, 51.0, 56.0, 60.0].map(|x| at(sixty_four, x)),
            [0, 0, 1, 3, 4]
        );
        assert_eq!(at(DEFAULT_ZOOM, 56.0), 12);
        assert_eq!(at(DEFAULT_ZOOM, 330.0), 423);
        assert_eq!(at(DEFAULT_ZOOM, 47.0), -2, "left of the timeline");
        let scrolled = View {
            scroll: Ticks(96),
            ..p3_view()
        };
        assert_eq!(scrolled.tick_at(56.0), Ticks(108));
        assert_eq!(scrolled.x_of(Ticks(108)), 56.0);
    }

    // A6, by hand: folded to keys 60, 64 and 67 with 67 on top, the rows are 67 (y 0..12), 64
    // (12..24) and 60 (24..36), and a note on 60 is drawn on the third; under them is below
    // the keyboard (−1, −2), above the top above it (128). Three keys in a grid of 10 rows keep
    // the highest on top whatever is asked. Folded to C major's keys, three rows up from 72
    // is 77 (74, 76, 77). Unfolded, every key keeps its row as before.
    #[test]
    fn a_fold_keeps_rows_only_for_its_keys() {
        let folded = View {
            top: 67,
            rows: Rows::of([60, 64, 67]),
            ..p3_view()
        };
        assert_eq!(
            [67, 64, 60].map(|key| folded.row_top(key)),
            [0.0, 12.0, 24.0]
        );
        assert_eq!(
            [5.0, 13.0, 30.0, 36.0, 50.0, -1.0].map(|y| folded.key_at(y)),
            [67, 64, 60, -1, -2, 128]
        );
        assert_eq!(folded.note_rect(&note(0, 48, 60)).y, 24.0);
        assert_eq!(folded.clamp_top(50, 120.0), 67);
        assert_eq!(folded.clamp_top(127, 120.0), 67);
        assert!(folded.rows.shows(64) && !folded.rows.shows(65));
        let c_major = [0u8, 2, 4, 5, 7, 9, 11];
        let scale = View {
            top: 72,
            rows: Rows::of((0..=127).filter(|key: &u8| c_major.contains(&(key % 12)))),
            ..p3_view()
        };
        assert_eq!(scale.top_moved(3, 340.0), 77);
        assert_eq!(scale.top_moved(-2, 340.0), 69);
        assert_eq!(Rows::of([]), Rows::ALL, "a fold of nothing keeps every key");
        let open = p3_view();
        for key in 0..=127u8 {
            assert_eq!(open.row_top(key), f32::from(69 - i16::from(key)) * 12.0);
            assert_eq!(open.key_at(open.row_top(key) + 6.0), i32::from(key));
        }
        assert_eq!(open.clamp_top(200, 340.0), 127);
        assert_eq!(open.top_moved(3, 340.0), 72);
    }

    #[test]
    fn rows_map_to_keys_from_the_top() {
        let view = p3_view();
        assert_eq!(view.key_at(114.0), 60);
        assert_eq!(view.key_at(66.0), 64);
        assert_eq!(view.key_at(0.0), 69);
        assert_eq!(view.key_at(11.9), 69);
        assert_eq!(view.key_at(12.0), 68);
        assert_eq!(view.key_at(-1.0), 70);
    }

    #[test]
    fn the_line_unit_follows_the_zoom() {
        let unit = |zoom| View { zoom, ..p3_view() }.line_unit().0;
        // 4, 6 px: a beat; 8..24 px: a step; 32..64 px: half a step.
        assert_eq!(
            (0..9).map(unit).collect::<Vec<_>>(),
            [96, 96, 24, 24, 24, 24, 12, 12, 12]
        );
    }

    // (a) presses A's body at x 56; (b) presses 2 px from its right edge, inside the 4 px zone.
    #[test]
    fn a_press_finds_the_note_and_the_part_under_it() {
        let view = p3_view();
        let notes = [note(0, 48, 60), note(96, 96, 64)];
        let under = |x, y| note_under(&view, notes.iter(), x, y).map(|(n, part)| (n.start.0, part));
        assert_eq!(under(56.0, 114.0), Some((0, NotePart::Body)));
        assert_eq!(under(78.0, 114.0), Some((0, NotePart::RightEdge)));
        assert_eq!(under(76.0, 114.0), Some((0, NotePart::RightEdge)));
        assert_eq!(under(75.9, 114.0), Some((0, NotePart::Body)));
        assert_eq!(under(120.0, 66.0), Some((96, NotePart::Body)));
        assert_eq!(under(80.0, 114.0), None, "the right edge itself is outside");
        assert_eq!(under(56.0, 102.0), None, "the row above");
        assert_eq!(under(330.0, 114.0), None);
        // Under 12 px wide a note has no resize zone.
        let narrow = [note(0, 12, 60)];
        let hit = note_under(&view, narrow.iter(), 55.0, 114.0).map(|(_, part)| part);
        assert_eq!(hit, Some(NotePart::Body));
    }

    #[test]
    fn a_box_selects_the_notes_it_meets() {
        let view = p3_view();
        let a = note(0, 48, 60);
        let b = note(96, 96, 64);
        let (from, to) = (Point::new(40.0, 100.0), Point::new(60.0, 130.0));
        assert!(meets_box(&view, &a, from, to));
        assert!(!meets_box(&view, &b, from, to));
        assert!(meets_box(
            &view,
            &b,
            Point::new(300.0, 0.0),
            Point::new(120.0, 65.0)
        ));
    }

    // Stem centers are x(tick) + 2: A at 50, B at 114.
    #[test]
    fn a_lane_press_picks_the_nearest_stem_within_6_px() {
        let view = p3_view();
        // Out of tick order on purpose: the tie rule must not depend on the order given.
        let notes = [note(6, 0, 62), note(96, 96, 64), note(0, 48, 60)];
        let pick = |x| stem_under(&view, notes.iter(), x).map(|n| n.start.0);
        assert_eq!(pick(50.0), Some(0));
        assert_eq!(pick(56.0), Some(6), "54 is nearer than 50");
        assert_eq!(pick(52.0), Some(0), "a tie goes to the leftmost");
        assert_eq!(pick(114.0), Some(96));
        assert_eq!(pick(121.0), None);
        // The lane's height is an argument now; 128 px is the old lane.
        let stem = view.stem_rect(&notes[2], 100, 128.0);
        assert_eq!(
            (stem.x, stem.y, stem.width, stem.height),
            (48.0, 28.0, 4.0, 100.0)
        );
    }

    // (c): a press at lane y 40 is velocity 88, the release at 64 sets 64. (The
    // lane's height is an argument now; these answers are the 128 px lane's.)
    #[test]
    fn lane_height_is_the_velocity_scale() {
        assert_eq!(lane_steps(40.0, 128.0), 88);
        assert_eq!(lane_steps(64.0, 128.0), 64);
        assert_eq!(lane_steps(63.6, 128.0), 64);
        assert_eq!(lane_steps(-30.0, 128.0), 128);
        assert_eq!(lane_steps(500.0, 128.0), 0);
    }

    #[test]
    fn the_top_key_stays_where_the_keyboard_fills_the_grid() {
        // 340 px hold 28 whole rows: key 0 is on the last of them when the top is 27.
        assert_eq!(visible_rows(340.0), 28);
        assert_eq!(clamp_top(69, 340.0), 69);
        assert_eq!(clamp_top(10, 340.0), 27);
        assert_eq!(clamp_top(200, 340.0), 127);
        // A grid taller than the keyboard keeps key 127 at the top; one with no whole row
        // keeps any key.
        assert_eq!(clamp_top(50, 5000.0), 127);
        assert_eq!(clamp_top(-5, 4.0), 0);
        assert_eq!(clamp_top(30, 4.0), 30);
    }

    // 1000 px: 952 px of timeline at 16 px per step is 1428 ticks; the view ends 4 bars past a
    // 1-bar pattern, at tick 1920.
    #[test]
    fn scrolling_stops_four_bars_past_the_end() {
        let view = p3_view();
        assert_eq!(max_scroll(&view, Ticks(384), 1000.0), Ticks(1920 - 1428));
        let far_out = View { zoom: 0, ..view };
        assert_eq!(max_scroll(&far_out, Ticks(384), 1000.0), Ticks(0));
    }

    #[test]
    fn an_open_roll_leaves_the_rack_its_content_up_to_half_the_area() {
        assert_eq!(rack_height_above_roll(84.0, 504.0), 84.0);
        assert_eq!(rack_height_above_roll(400.0, 504.0), 252.0);
    }

    // By hand: the default lane is 64 px, half the old 128, so a step of
    // velocity is half a pixel. Velocity 88 is (64 − y)·128/64 = 88 at y 20, 64 at y 32, the
    // top (y 0) 128 and the bottom 0. Stem A (velocity 100) is 100·64/128 = 50 px tall, from y
    // 14; stem B (64) 32 px, from y 32.
    #[test]
    fn the_default_lane_is_half_as_tall_on_the_same_scale() {
        assert_eq!(LANE_HEIGHT, 64.0);
        assert_eq!(lane_steps(20.0, LANE_HEIGHT), 88);
        assert_eq!(lane_steps(32.0, LANE_HEIGHT), 64);
        assert_eq!(lane_steps(0.0, LANE_HEIGHT), 128);
        assert_eq!(lane_steps(64.0, LANE_HEIGHT), 0);
        assert_eq!(lane_steps(31.9, LANE_HEIGHT), 64, "64.2 rounds to 64");
        let view = p3_view();
        let a = view.stem_rect(&note(0, 48, 60), 100, LANE_HEIGHT);
        assert_eq!((a.x, a.y, a.width, a.height), (48.0, 14.0, 4.0, 50.0));
        let b = view.stem_rect(&note(96, 96, 64), 64, LANE_HEIGHT);
        assert_eq!((b.x, b.y, b.height), (112.0, 32.0, 32.0));
    }

    // The handle drags the lane by what it moves: up 30 px from a 64 px lane is 94, down 20 px
    // 44; it stops at 32 and at 120 (136 under the grid less the 10 px scroll bar and the 6 px
    // handle), and a lane height that is no number reads as the default.
    #[test]
    fn the_handle_drags_the_lane_between_its_limits() {
        assert_eq!(dragged_lane(64.0, 400.0, 370.0), 94.0);
        assert_eq!(dragged_lane(64.0, 400.0, 420.0), 44.0);
        assert_eq!(dragged_lane(64.0, 400.0, 100.0), LANE_MAX_HEIGHT);
        assert_eq!(LANE_MAX_HEIGHT, 120.0);
        assert_eq!(dragged_lane(64.0, 400.0, 700.0), LANE_MIN_HEIGHT);
        assert_eq!(kept_lane(f32::NAN), LANE_HEIGHT);
        assert_eq!(kept_lane(f32::INFINITY), LANE_HEIGHT);
        assert_eq!(kept_lane(500.0), LANE_MAX_HEIGHT);
    }

    // P2: the grid's scroll along time, by hand. The 1000 px grid shows 952 px of timeline,
    // 1428 ticks at 16 px a step; notes ending at bar 1 (384) let it reach 4 bars further, 1920.
    // Scrolled to 492 (`max_scroll`), the view ends at the content's end.
    #[test]
    fn the_time_span_reaches_four_bars_past_the_notes() {
        let view = p3_view();
        let span = time_span(&view, Ticks(384), 1000.0);
        assert_eq!(
            (span.content, span.offset, span.view),
            (1920.0, 0.0, 1428.0)
        );
        let end = View {
            scroll: max_scroll(&view, Ticks(384), 1000.0),
            ..view
        };
        let span = time_span(&end, Ticks(384), 1000.0);
        assert_eq!(span.offset + span.view, span.content);
        assert_eq!(span.last_offset(), 492.0);
    }
}
