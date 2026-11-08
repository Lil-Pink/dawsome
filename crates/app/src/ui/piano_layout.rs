//! Every position in the piano roll, in logical pixels. The grid and lane canvases draw from
//! these values, the gestures map the cursor with them, and the screenshot probe samples the
//! points they give, so the drawing and its checks cannot drift apart.
//!
//! The panel, top to bottom: the strip ([`STRIP_HEIGHT`]), the ruler ([`RULER_HEIGHT`]: the bar
//! numbers over the timeline, as the playlist's ruler has them), the grid (the rest), the
//! divider ([`DIVIDER_HEIGHT`]: the grid's scroll bar along time, its rail the panel's whole
//! width, under the key column too) and the velocity lane ([`LANE_HEIGHT`], fixed).
//!
//! Grid coordinates start at the grid canvas's top-left corner. The key column is x 0..48, drawn
//! as a keyboard (a black key's row black over its left 32 px, [`black_key_width`], the white
//! key showing beside it), and the timeline starts at x 48: tick t is at
//! x = 48 + (t − scroll)·px_per_step/step_ticks. The
//! row of key k spans y = (top − k)·12 .. +12, where `top` is the key of the top row. Only the
//! rows the grid holds whole show, top-aligned under the ruler; the few pixels under the last
//! are ground ([`rows_bottom`]), and the keys scroll a whole row at a time. Ruler and
//! lane coordinates start at their canvas's top-left corner and share the grid's x; the lane
//! shows velocity 128 at its top and 0 at its bottom.
//!
//! The zoom is any number of pixels per step from 4 to 64 ([`View::zoom`], [`ZOOM`]), and the
//! grid follows the rules the playlist's lanes do ([`grid_rules`]): lines fading in as they grow
//! apart (half steps, steps and beats, [`View::line_levels`]), a stronger line at the start of
//! every four bars, every other bar shaded, a scroll along time that comes to rest on a whole
//! cell ([`View::landed`]) and a zoom that keeps the tick under the cursor there
//! ([`View::zoomed_at`]).
//!
//! Probe coordinates (window, logical, piano roll alone in a 1000x560 window, p3_roll): the
//! ruler starts at y 92, the grid at y 108 and the lane at y 528; the top key is 69, so key 60's
//! row is y 216..228.

use dawsome_engine::project::{Note, MAX_KEY};
use dawsome_engine::time::{Ticks, BEATS_PER_BAR};
use iced::{Point, Rectangle, Size};

use crate::ui::grid_rules::{self, ZoomRange, NUMBER_BAND_HEIGHT};
use crate::ui::layout::TOOLBAR_HEIGHT;
use crate::ui::scroll::{self, Span};

/// The strip of controls above the grid.
pub const STRIP_HEIGHT: f32 = 28.0;
/// The strip's margin at either end.
pub const STRIP_PADDING: f32 = 12.0;
/// The space between the controls of one group in the strip (the snap and the draw length;
/// Preview and Follow; the scale and Fold).
pub const STRIP_SPACING: f32 = 10.0;
/// The space between the strip's groups, the channel's name one of its own: two of
/// [`STRIP_SPACING`]. Why groups: ten controls evenly spaced read as one run, and what belongs
/// together reads so only when the gaps say it.
pub const STRIP_GROUP_GAP: f32 = 2.0 * STRIP_SPACING;
/// The strip's Play / Stop: as tall as its text buttons (a label's line, 15.6, and 2 above and
/// below), its glyph a [`PLAY_ICON`] square with their 8 px either side.
pub const PLAY_BUTTON: iced::Size = iced::Size::new(26.0, 20.0);
pub const PLAY_ICON: f32 = 10.0;
/// The roll's player, Play and Loop: [`PLAYER_GAP`] apart, both Play's 20 px tall, with
/// [`PLAYER_SIDE`] of plain room either side. Why that room: a box drew round the two there (its
/// 1 px edge and 4 px inset either side) and was taken away as it read poorly; the room it took
/// stays, so nothing on the strip moved.
pub const PLAYER_GAP: f32 = 4.0;
pub const PLAYER_SIDE: f32 = 5.0;
/// The strip's Preview and Follow once the strip folds them to glyphs (a speaker, an arrow;
/// [`crate::ui::piano_roll::StripFold`]): a 24 px target as tall as Play, its glyph a
/// [`GLYPH_SWITCH_ICON`] square in the middle.
pub const GLYPH_SWITCH: iced::Size = iced::Size::new(24.0, PLAY_BUTTON.height);
pub const GLYPH_SWITCH_ICON: f32 = 12.0;
/// The Scale chooser's width once the strip folds it, its last fold: 18 px of its face for the
/// name after the arrow and the padding, so it shows the arrow alone (no name's first letter
/// and "…" fit), the root beside it and the hint and the menu saying the scale.
pub const SCALE_FOLDED_WIDTH: f32 = 48.0;
/// The ruler between the strip and the grid: the bar numbers, in a band as tall as the
/// playlist's.
pub const RULER_HEIGHT: f32 = NUMBER_BAND_HEIGHT;
/// The grid's top in the panel: under the strip and the ruler.
pub const GRID_TOP: f32 = STRIP_HEIGHT + RULER_HEIGHT;
/// What lies between the grid and the velocity lane: the grid's scroll bar along time.
pub const DIVIDER_HEIGHT: f32 = scroll::BAR_HEIGHT;
/// The velocity lane's height, fixed: the least its handle could drag it to when it had one.
/// Why fixed: the lane needs room to read and drag a velocity, and the grid is what the panel
/// is for.
pub const LANE_HEIGHT: f32 = 32.0;
/// What the divider and the lane take under the grid.
pub const UNDER_GRID: f32 = DIVIDER_HEIGHT + LANE_HEIGHT;
/// The room the roll's natural height keeps beside the strip and the grid rows it counts
/// ([`crate::ui::panels::natural_height`]): 136 px, what the divider and the lane took when the
/// lane was dragged to its tallest. Why it stayed when the lane became fixed and the ruler came:
/// the roll keeps its natural height in the stack, so the grid takes the room the lane gave up,
/// less the ruler's, rather than the panels around the roll. Its least height counts only what
/// it shows ([`crate::ui::panels::MIN_ROLL_HEIGHT`]).
pub const ROOM_BESIDE_ROWS: f32 = 136.0;

pub const KEY_COLUMN_WIDTH: f32 = 48.0;
pub const ROW_HEIGHT: f32 = 12.0;
/// C labels end at this x, right-aligned in the white key's visible part (a C key's row is
/// white across the column).
pub const LABEL_RIGHT: f32 = 44.0;
/// How much of the key column's width a black key covers, the white key showing beside it as
/// on a keyboard. Why not all of it: black keys across the whole column read as stripes, not
/// keys.
const BLACK_KEY_SHARE: f32 = 2.0 / 3.0;

/// The zoom, in pixels per step: 4 to 64 (the ends of the zoom levels it had before it was
/// continuous), 16 to start. A Ctrl+wheel notch moves it a quarter of a doubling
/// ([`ZoomRange::wheeled`]), as it does the playlist's.
pub const ZOOM: ZoomRange = ZoomRange {
    min: 4.0,
    max: 64.0,
    default: 16.0,
};

/// [`View::line_unit`], the snap Line, is the finest line drawn at least this strong. Why not
/// the finest drawn at all: a line too faint to see is no line to snap to.
pub const SNAP_LINE_ALPHA: f32 = 0.5;

/// A press this close to a note's right edge resizes the note, if the note is at least
/// [`RESIZE_MIN_WIDTH`] wide; anywhere else on it moves it.
pub const RESIZE_EDGE: f32 = 4.0;
pub const RESIZE_MIN_WIDTH: f32 = 12.0;

/// Width of a velocity stem, drawn from its note's start.
pub const STEM_WIDTH: f32 = 4.0;
/// A lane press picks the stem whose center is at most this far away.
pub const STEM_PICK_DISTANCE: f32 = 6.0;

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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// The key whose row is at the top of the grid.
    pub top: u8,
    /// The tick at the timeline's left edge (x = [`KEY_COLUMN_WIDTH`]); never negative.
    pub scroll: Ticks,
    /// Pixels per step, in [`ZOOM`]'s range.
    pub zoom: f32,
    /// Ticks in one step (ppq / 4), at least 1.
    pub step: Ticks,
    /// The keys with rows: every key unless folded.
    pub rows: Rows,
}

impl View {
    pub fn px_per_step(&self) -> f32 {
        ZOOM.kept(self.zoom)
    }

    /// Pixels per bar: the zoom in the playlist's unit, which the grid rules read.
    pub fn px_per_bar(&self) -> f32 {
        self.px_per_step() * (self.bar().0 / self.step.0.max(1)) as f32
    }

    /// Ticks in one beat and in one bar.
    pub fn beat(&self) -> Ticks {
        Ticks(self.step.0 * 4)
    }

    pub fn bar(&self) -> Ticks {
        Ticks(self.beat().0 * i64::from(BEATS_PER_BAR))
    }

    /// The lines between the bars, finest first: half steps, steps and beats, each as its ticks
    /// and how strongly it shows at this zoom ([`grid_rules::line_alpha`] of how far apart its
    /// lines are; 0 is not drawn).
    pub fn line_levels(&self) -> [(Ticks, f32); 3] {
        let px = self.px_per_step();
        [
            (
                Ticks((self.step.0 / 2).max(1)),
                grid_rules::line_alpha(px / 2.0),
            ),
            (self.step, grid_rules::line_alpha(px)),
            (self.beat(), self.beat_alpha()),
        ]
    }

    /// How strongly the beat lines show at this zoom ([`grid_rules::beat_alpha`]); the ruler's
    /// beat ticks fade with them.
    pub fn beat_alpha(&self) -> f32 {
        grid_rules::beat_alpha(self.px_per_bar())
    }

    /// The finest grid line, which follows the zoom (the snap Line): the finest of
    /// [`View::line_levels`] drawn at least [`SNAP_LINE_ALPHA`] strong, a beat at the least.
    pub fn line_unit(&self) -> Ticks {
        self.line_levels()
            .into_iter()
            .find(|&(_, alpha)| alpha >= SNAP_LINE_ALPHA)
            .map_or(self.beat(), |(ticks, _)| ticks)
    }

    /// The cell a scroll along time comes to rest on ([`grid_rules::cell`]): a beat while beat
    /// lines are drawn, which they are at every zoom the roll has, else a bar.
    pub fn cell(&self) -> Ticks {
        grid_rules::cell(self.beat(), self.bar(), self.px_per_bar())
    }

    /// `tick` moved to the nearest cell's start ([`grid_rules::landed`]).
    pub fn landed(&self, tick: Ticks) -> Ticks {
        grid_rules::landed(tick, self.cell())
    }

    /// Where a scroll from the view's start towards `to` comes to rest
    /// ([`grid_rules::landed_towards`]): a short move still moves a cell.
    pub fn landed_towards(&self, to: Ticks) -> Ticks {
        grid_rules::landed_towards(self.scroll, to, self.cell())
    }

    /// The start of the cell holding `tick`.
    pub fn cell_start(&self, tick: Ticks) -> Ticks {
        grid_rules::cell_start(tick, self.cell())
    }

    /// This view at `px` pixels per step (kept), scrolled so that what shows at grid x `x` now
    /// shows there still ([`grid_rules::anchored_scroll`]).
    pub fn zoomed_at(&self, px: f32, x: f32) -> View {
        let from_left = f64::from(x) - f64::from(KEY_COLUMN_WIDTH);
        let zoomed = View {
            zoom: ZOOM.kept(px),
            ..*self
        };
        View {
            scroll: grid_rules::anchored_scroll(
                self.scroll,
                from_left,
                self.ticks(),
                self.px_per_step(),
                zoomed.px_per_step(),
            ),
            ..zoomed
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

    /// Whether `key`'s row shows whole in a grid `grid_height` tall: the key has a row (a fold
    /// keeps it), the top row or one under it, and ends by [`rows_bottom`].
    pub fn shows_row(&self, key: u8, grid_height: f32) -> bool {
        let top = self.row_top(key);
        self.rows.shows(key) && top >= 0.0 && top + ROW_HEIGHT <= rows_bottom(grid_height)
    }

    /// The key whose row holds grid y in a grid `grid_height` tall, while that row shows whole:
    /// none above the grid, in the ground under its whole rows ([`rows_bottom`]) or under the
    /// keyboard. Where a press and the hint bar read the key, so the ground plays and draws
    /// nothing.
    pub fn key_in_rows(&self, y: f32, grid_height: f32) -> Option<u8> {
        if !(0.0..rows_bottom(grid_height)).contains(&y) {
            return None;
        }
        u8::try_from(self.key_at(y))
            .ok()
            .filter(|&key| key <= MAX_KEY)
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

    /// Where `note`'s velocity stem starts across the lane: at the note's start, or, for a
    /// note that starts left of the view and still shows in it, at the timeline's left edge
    /// ([`KEY_COLUMN_WIDTH`]); none for a note that ends at or before the view's start. Why the
    /// edge: a note scrolled half out of view showed in the grid with no stem in the lane (its
    /// start, where the stem stood, was under the key column), and read as a note at the
    /// pattern's start without one.
    pub fn stem_x(&self, note: &Note) -> Option<f32> {
        let end = Ticks(note.start.0 + drawn_length(note, self.step).0);
        (end > self.scroll).then(|| self.x_of(note.start).max(KEY_COLUMN_WIDTH))
    }

    /// The velocity stem of `note`, lane coordinates: [`STEM_WIDTH`] wide from [`View::stem_x`],
    /// `steps`/128 of the lane tall from its bottom; none for a note wholly left of the view.
    pub fn stem_rect(&self, note: &Note, steps: u8) -> Option<Rectangle> {
        let height = f32::from(steps) * LANE_HEIGHT / VELOCITY_STEPS;
        let x = self.stem_x(note)?;
        Some(Rectangle::new(
            Point::new(x, LANE_HEIGHT - height),
            Size::new(STEM_WIDTH, height),
        ))
    }

    fn ticks(&self) -> f64 {
        self.step.0.max(1) as f64
    }
}

/// A black key's width in a key column `column` wide: [`BLACK_KEY_SHARE`] of it, to the whole
/// pixel, so it follows the column: 32 px of the 48 px one.
pub fn black_key_width(column: f32) -> f32 {
    (column * BLACK_KEY_SHARE).round()
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

/// The middle of `note`'s stem across the lane, where a press or a paint finds it
/// ([`View::stem_x`]); none for a note wholly left of the view.
pub fn stem_middle(view: &View, note: &Note) -> Option<f32> {
    view.stem_x(note).map(|x| x + STEM_WIDTH / 2.0)
}

/// The stem nearest lane x among `notes` within [`STEM_PICK_DISTANCE`] of its center; the
/// leftmost on a tie.
pub fn stem_under<'a>(
    view: &View,
    notes: impl Iterator<Item = &'a Note>,
    x: f32,
) -> Option<&'a Note> {
    notes
        .filter_map(|note| Some((note, (stem_middle(view, note)? - x).abs())))
        .filter(|&(_, distance)| distance <= STEM_PICK_DISTANCE)
        .min_by(|(a, from_a), (b, from_b)| from_a.total_cmp(from_b).then(a.start.cmp(&b.start)))
        .map(|(note, _)| note)
}

/// The top of the velocity lane's scale: 128 steps, the scale the stems and notes use.
const VELOCITY_STEPS: f32 = 128.0;

/// The velocity a lane press or drag at lane y sets, on the 0..=128 scale: 128 at the lane's
/// top, 0 at its bottom, rounded and clamped.
pub fn lane_steps(y: f32) -> u8 {
    // Clamped into 0..=128 first, so the conversion is exact.
    ((LANE_HEIGHT - y) * VELOCITY_STEPS / LANE_HEIGHT)
        .round()
        .clamp(0.0, VELOCITY_STEPS) as u8
}

/// Whole rows that fit in a grid `height` pixels tall, at most the 128 keys.
pub fn visible_rows(height: f32) -> u8 {
    (height / ROW_HEIGHT).floor().clamp(0.0, 128.0) as u8
}

/// Where the whole rows of a grid `height` tall end: [`visible_rows`] rows from its top. The
/// rest of the grid under them is ground, the roll's own, pressed on nothing. Why whole rows
/// only: the grid's foot cut its last row to a sliver, a key a few pixels tall with its C's
/// name cut in half, as the playlist cut its last track before it showed whole lanes only.
pub fn rows_bottom(height: f32) -> f32 {
    f32::from(visible_rows(height)) * ROW_HEIGHT
}

/// `top` kept where the keyboard fills the grid: no higher than key 127, and high enough that
/// key 0 is not above the grid's last whole row.
pub fn clamp_top(top: i32, grid_height: f32) -> u8 {
    let lowest = (i32::from(visible_rows(grid_height)) - 1).max(0);
    let highest = i32::from(MAX_KEY);
    // Both bounds lie in 0..=127, so the clamped key fits a u8.
    top.clamp(lowest.min(highest), highest) as u8
}

/// The largest scroll: the one that shows the pattern and [`BARS_PAST_END`] bars past it to
/// their end, landed up to a whole cell ([`View::cell`]), so a view scrolled to the end starts
/// on one too and shows a little more than the bars past the end.
pub fn max_scroll(view: &View, pattern_end: Ticks, grid_width: f32) -> Ticks {
    let view_end = pattern_end.0 + BARS_PAST_END * view.bar().0;
    let reach = (view_end - visible_ticks(view, grid_width)).max(0);
    let cell = view.cell().0;
    Ticks((reach + cell - 1).div_euclid(cell) * cell)
}

/// The whole ticks the timeline of a grid `grid_width` wide shows.
fn visible_ticks(view: &View, grid_width: f32) -> i64 {
    let timeline = f64::from((grid_width - KEY_COLUMN_WIDTH).max(0.0));
    (timeline * view.ticks() / f64::from(view.px_per_step())).floor() as i64
}

/// The grid's scroll along time, in ticks, for notes that end by `end`: the content reaches
/// [`BARS_PAST_END`] bars past it, or on to the end of the view at [`max_scroll`] when that is
/// further (so the thumb dragged to the bar's end lands there), the view is what the timeline
/// shows.
pub fn time_span(view: &View, end: Ticks, grid_width: f32) -> Span {
    let visible = visible_ticks(view, grid_width);
    let past_end = end.0 + BARS_PAST_END * view.bar().0;
    let last_view_end = max_scroll(view, end, grid_width).0 + visible;
    Span {
        content: past_end.max(last_view_end) as f64,
        offset: view.scroll.0 as f64,
        view: visible as f64,
    }
}

/// The bars whose line is inside the timeline of a grid `width` wide, as (bar number counted
/// from 1, line x).
pub fn visible_bars(view: &View, width: f32) -> impl Iterator<Item = (i64, f32)> + '_ {
    let bar = view.bar().0.max(1);
    grid_rules::bar_lines(view.scroll, view.tick_at(width), view.bar())
        .map(move |index| (index + 1, view.x_of(Ticks(index * bar))))
}

/// The beats starting no bar whose line is inside the timeline of a grid `width` wide, as that
/// line's x: where the ruler's beat ticks go, the bars having ticks of their own.
pub fn visible_beats(view: &View, width: f32) -> impl Iterator<Item = f32> + '_ {
    let (beat, bar) = (view.beat().0.max(1), view.bar().0.max(1));
    let first = grid_rules::first_at_or_after(view.scroll.0, beat);
    (first..=view.tick_at(width).0)
        .step_by(beat as usize)
        .filter(move |tick| tick % bar != 0)
        .map(move |tick| view.x_of(Ticks(tick)))
}

/// The x spans the shaded bars ([`grid_rules::shaded`]) take on the timeline of a grid `width`
/// wide, left to right, cut to the timeline.
pub fn shaded_spans(view: &View, width: f32) -> impl Iterator<Item = (f32, f32)> + '_ {
    grid_rules::shaded_ticks(
        view.scroll,
        view.tick_at(width),
        view.bar(),
        view.px_per_bar(),
    )
    .filter_map(move |(start, end)| {
        let left = view.x_of(start).max(KEY_COLUMN_WIDTH);
        let right = view.x_of(end).min(width);
        (right > left).then_some((left, right))
    })
}

/// The grid canvas's size when the piano roll is the only panel in a window of `window` size.
pub fn grid_size_alone(window: Size) -> Size {
    let height = window.height - TOOLBAR_HEIGHT - GRID_TOP - UNDER_GRID;
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
            zoom: ZOOM.default,
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
        // Alone in a 1000x560 window the grid is 410 tall: 560 less the toolbar (64), the
        // strip (28), the ruler (16), the grid's scroll bar (10) and the lane (32).
        // (Pin moved: 340 under an 8 px divider and a 128 px lane; the half lane and handle
        // made it 406, the scroll bar 396, the 72 px toolbar 380, the 64 px one 388; then the
        // fixed 32 px lane without its 6 px handle and the 16 px ruler, 410.)
        assert_eq!(
            grid_size_alone(Size::new(1000.0, 560.0)),
            Size::new(1000.0, 410.0)
        );
    }

    // tick_at(x) = floor((x − 48)·24/pxs) + scroll. At 6 px per step a pixel is 4 ticks; at 64 px
    // per step it is 0.375 ticks (gen_p3_expected.py prints the same values).
    #[test]
    fn tick_at_floors_the_ticks_under_a_pixel() {
        let at = |zoom: f32, x: f32| View { zoom, ..p3_view() }.tick_at(x).0;
        assert_eq!([48.0, 49.0, 50.0, 73.0].map(|x| at(6.0, x)), [0, 4, 8, 100]);
        assert_eq!(
            [48.0, 50.0, 51.0, 56.0, 60.0].map(|x| at(64.0, x)),
            [0, 0, 1, 3, 4]
        );
        assert_eq!(at(ZOOM.default, 56.0), 12);
        assert_eq!(at(ZOOM.default, 330.0), 423);
        assert_eq!(at(ZOOM.default, 47.0), -2, "left of the timeline");
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

    // The snap Line is the finest line drawn at least half strong, (apart − 5)/14 ≥ 0.5, so
    // lines at least 12 px apart: at the old zoom stops 4, 6 and 8 px a step the beat (16 px and
    // more; steps 4 to 8 px), at 12 and 16 the step (half steps 6 and 8 px), from 24 the half
    // step (12 px and more). (Pin moved from the stops' own rule, [96, 96, 24, 24, 24, 24, 12,
    // 12, 12]: 8 px a step now snaps to the beat, its step lines 3/14 strong, and 24 px a step
    // to the half step, its half-step lines 1/2 strong.)
    #[test]
    fn the_line_unit_follows_the_zoom() {
        let unit = |zoom| View { zoom, ..p3_view() }.line_unit().0;
        let stops = [4.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0];
        assert_eq!(stops.map(unit), [96, 96, 96, 24, 24, 12, 12, 12, 12]);
    }

    // The roll's zoom by the playlist's laws, in pixels per step: 4 to 64, 16 to start; a
    // Ctrl+wheel notch in from 16 is 4·2^(9/4) = 19.0273, four are 32, four out 8; at either end
    // a notch further holds. A kept value out of range reads back held, one that is no number
    // the default. 16 px a step is 256 px a bar, 16 steps of it.
    #[test]
    fn the_zoom_is_any_pixels_per_step_from_4_to_64() {
        assert!((ZOOM.wheeled(16.0, true) - 19.027_313).abs() < 1e-4);
        let notches =
            |from: f32, closer: bool| (0..4).fold(from, |zoom, _| ZOOM.wheeled(zoom, closer));
        assert!((notches(16.0, true) - 32.0).abs() < 1e-5);
        assert!((notches(16.0, false) - 8.0).abs() < 1e-5);
        assert_eq!(
            (ZOOM.wheeled(64.0, true), ZOOM.wheeled(4.0, false)),
            (64.0, 4.0)
        );
        assert_eq!(
            (ZOOM.kept(100.0), ZOOM.kept(1.0), ZOOM.kept(f32::NAN)),
            (64.0, 4.0, 16.0)
        );
        assert_eq!(p3_view().px_per_bar(), 256.0);
    }

    // The lines' strength by hand, (apart − 5)/14 kept to 0..1, at 16 px a step: half steps
    // 8 px apart 3/14, steps 16 px apart 11/14, beats 64 px apart whole. At 4 px a step the
    // beats are 16 px apart, 11/14, the steps and half steps none: beats are drawn at every
    // zoom, so a scroll always lands on a beat, 96 ticks: 140 lands on 96, 144 (halfway) on
    // 192.
    #[test]
    fn lines_fade_by_how_far_apart_they_are_and_a_scroll_lands_on_a_beat() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-6;
        let [half, step, beat] = p3_view().line_levels();
        assert_eq!((half.0, step.0, beat.0), (Ticks(12), Ticks(24), Ticks(96)));
        assert!(close(half.1, 3.0 / 14.0) && close(step.1, 11.0 / 14.0) && beat.1 == 1.0);
        let far = View {
            zoom: ZOOM.min,
            ..p3_view()
        };
        let [half, step, beat] = far.line_levels();
        assert!(half.1 == 0.0 && step.1 == 0.0 && close(beat.1, 11.0 / 14.0));
        assert_eq!((far.cell(), p3_view().cell()), (Ticks(96), Ticks(96)));
        assert_eq!(p3_view().landed(Ticks(140)), Ticks(96));
        assert_eq!(p3_view().landed(Ticks(144)), Ticks(192), "halfway goes on");
    }

    // The anchor law by hand: at 16 px a step unscrolled, x 304 shows tick 256·24/16 = 384; at
    // 32 px a step it is there again from scroll 384 − 256·24/32 = 192. Out to 4 px a step at
    // x 600 (tick 552·24/16 = 828), the scroll would be 828 − 552·6 < 0: the view stays at the
    // start.
    #[test]
    fn a_zoom_keeps_the_tick_under_the_cursor_in_place() {
        let zoomed = p3_view().zoomed_at(32.0, 304.0);
        assert_eq!((zoomed.zoom, zoomed.scroll), (32.0, Ticks(192)));
        assert_eq!(zoomed.x_of(Ticks(384)), 304.0);
        assert_eq!(p3_view().zoomed_at(4.0, 600.0).scroll, Ticks(0));
        assert_eq!(p3_view().zoomed_at(900.0, 600.0).zoom, 64.0, "kept");
    }

    // The shading and the ruler's bars by hand, at 16 px a step (256 px a bar) on a 1000 px
    // grid: bars 1 to 4 at x 48, 304, 560 and 816 (tick 1428 is the last in view), bars 1 and 3
    // shaded, x 48..304 and 560..816. Scrolled half a bar (192 ticks, 128 px): bar 1's shading
    // is cut at the timeline's edge, 48..176; bar 3's is 432..688 and bar 5's, from 944, is cut
    // at the grid's right edge; the first number is bar 2's, at 176.
    #[test]
    fn every_other_bar_is_shaded_and_numbered_where_its_line_is() {
        let bars: Vec<(i64, f32)> = visible_bars(&p3_view(), 1000.0).collect();
        assert_eq!(bars, [(1, 48.0), (2, 304.0), (3, 560.0), (4, 816.0)]);
        let spans: Vec<(f32, f32)> = shaded_spans(&p3_view(), 1000.0).collect();
        assert_eq!(spans, [(48.0, 304.0), (560.0, 816.0)]);
        let scrolled = View {
            scroll: Ticks(192),
            ..p3_view()
        };
        let spans: Vec<(f32, f32)> = shaded_spans(&scrolled, 1000.0).collect();
        assert_eq!(spans, [(48.0, 176.0), (432.0, 688.0), (944.0, 1000.0)]);
        assert_eq!(visible_bars(&scrolled, 1000.0).next(), Some((2, 176.0)));
    }

    // The ruler's beat ticks by hand, at 16 px a step (a beat every 96 ticks, 64 px) on a 1000 px
    // grid showing ticks to 1428: the beats at x 48 + t·16/24 that start no bar (bars at 0, 384,
    // 768 and 1152 have ticks of their own), the last at 1344; scrolled half a beat (48 ticks,
    // 32 px) the first is the beat at 96, x 80.
    #[test]
    fn the_ruler_ticks_every_beat_that_starts_no_bar() {
        let beats: Vec<f32> = visible_beats(&p3_view(), 1000.0).collect();
        assert_eq!(
            beats,
            [112.0, 176.0, 240.0, 368.0, 432.0, 496.0, 624.0, 688.0, 752.0, 880.0, 944.0]
        );
        let scrolled = View {
            scroll: Ticks(48),
            ..p3_view()
        };
        assert_eq!(visible_beats(&scrolled, 1000.0).next(), Some(80.0));
    }

    // The ticks' strength is the beat lines', (beat px − 5)/14 kept to 0..1, a beat four steps:
    // at 4 px a step 16 px, 11/14; at 4.5, 18 px, 13/14; at 16, 64 px, whole.
    #[test]
    fn the_beat_ticks_fade_with_the_beat_lines() {
        let alpha = |zoom| View { zoom, ..p3_view() }.beat_alpha();
        assert!((alpha(4.0) - 11.0 / 14.0).abs() < 1e-6);
        assert!((alpha(4.5) - 13.0 / 14.0).abs() < 1e-6);
        assert_eq!(alpha(16.0), 1.0);
        assert_eq!(
            p3_view().line_levels()[2].1,
            alpha(16.0),
            "the beat lines' own"
        );
    }

    // A black key is two thirds of the key column, to the whole pixel: 32 of the 48 px column,
    // 2/3 · 64 = 42.67 → 43 of a 64 px one, 24 of a 36 px one.
    #[test]
    fn a_black_key_is_two_thirds_of_the_column() {
        assert_eq!(black_key_width(KEY_COLUMN_WIDTH), 32.0);
        assert_eq!(black_key_width(64.0), 43.0);
        assert_eq!(black_key_width(36.0), 24.0);
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
        // On the 32 px lane a stem of velocity 100 is 100·32/128 = 25 px tall, from y 7.
        // (Pin moved from the 128 px lane's 100 px from y 28.)
        let stem = view.stem_rect(&notes[2], 100).expect("in view");
        assert_eq!(
            (stem.x, stem.y, stem.width, stem.height),
            (48.0, 7.0, 4.0, 25.0)
        );
    }

    // Every note the grid shows has its stem. By hand at 64 px a step (a tick 64 / 24 = 2.667
    // px): with the view at its start, a note at tick 0 has its stem at the tick's x, 48, and
    // its middle at x + 2 = 50, inside the lane; two notes at one tick (a chord, keys 60 and
    // 64) stand their stems in that one column, on top of each other, on purpose: a chord's
    // velocities read as one stem, the last drawn (the higher key) over the others, the taller
    // showing above a shorter one. With the view scrolled to tick 148 (a Ctrl+wheel zoom keeps
    // the tick under the cursor, so a view can start mid-step; found in a shot at 64 px a step,
    // where notes at tick 144 showed half under the key column with no stem), a note at 144,
    // 24 long, ends at 168, inside the view: its start's x is 48 + (144 − 148) · 2.667 = 37.33,
    // under the key column, so its stem stands at the timeline's left edge, 48..52, and a press
    // at 50 picks it; a note at 120 ending at 144 lies wholly left and has none. A note at 168
    // has its stem at its own x, 48 + 20 · 2.667 = 101.33. (Before: a stem stood only at its
    // note's start, and one left of x 48 was not drawn.)
    #[test]
    fn every_note_in_view_has_a_stem_in_the_lane() {
        let zoomed = View {
            zoom: 64.0,
            ..p3_view()
        };
        let chord = [note(0, 24, 60), note(0, 24, 64)];
        let low = zoomed.stem_rect(&chord[0], 100).expect("in view");
        let high = zoomed.stem_rect(&chord[1], 64).expect("in view");
        assert_eq!((low.x, low.center_x()), (48.0, 50.0));
        assert_eq!(high.x, low.x, "a chord's stems in one column, on purpose");
        assert!(low.x >= KEY_COLUMN_WIDTH && low.x + low.width <= 1000.0);
        let scrolled = View {
            scroll: Ticks(148),
            ..zoomed
        };
        let half_out = note(144, 24, 60);
        assert!((scrolled.x_of(Ticks(144)) - 37.333).abs() < 1e-3);
        let stem = scrolled.stem_rect(&half_out, 100).expect("it shows");
        assert_eq!((stem.x, stem.width), (48.0, 4.0));
        assert_eq!(
            scrolled.stem_rect(&note(120, 24, 60), 100),
            None,
            "wholly left"
        );
        let after = scrolled.stem_x(&note(168, 24, 60)).expect("in view");
        assert!((after - 101.333).abs() < 1e-3, "{after}");
        let notes = [note(120, 24, 62), half_out.clone()];
        let picked = stem_under(&scrolled, notes.iter(), 50.0).map(|n| n.start.0);
        assert_eq!(picked, Some(144));
    }

    // (c): a press at lane y 10 is velocity (32 − 10)·4 = 88, the release at 16 sets 64.
    // (Pins moved from the 128 px lane's y 40 and 64: the 32 px lane is four steps a pixel.)
    #[test]
    fn lane_height_is_the_velocity_scale() {
        assert_eq!(lane_steps(10.0), 88);
        assert_eq!(lane_steps(16.0), 64);
        assert_eq!(lane_steps(15.9), 64, "64.4 rounds to 64");
        assert_eq!(lane_steps(-30.0), 128);
        assert_eq!(lane_steps(500.0), 0);
    }

    // The whole-row rule by hand, rows of 12 px top-aligned under the ruler: a grid h px tall
    // holds floor(h / 12) whole rows (at most the keyboard's 128) and the h − 12·rows px under
    // them are ground. Alone in a 1000 × 560 window the grid is 410 px: 34 rows (408 px; 35
    // take 420), 2 px of ground. At the roll's least height with no divider over it, 224 px,
    // the grid is 224 − 28 (strip) − 16 (ruler) − 10 (scroll bar) − 32 (lane) = 138 px: 11 rows
    // (132), 6 px; given way to 146, 60 px: 5 rows exactly, no ground; at its natural height,
    // 308 (28 + 12 · 12 + 136), 222 px: 18 rows (216), 6 px; 340 px (the roll tests' grid):
    // 28 rows (336), 4 px. Under a row, 11 px: none, all ground; taller than the keyboard,
    // 1600 px: its 128 rows (1536) and 64 px.
    #[test]
    fn the_grid_shows_whole_rows_and_the_rest_is_ground() {
        let table: [(f32, u8, f32); 9] = [
            (410.0, 34, 2.0),
            (138.0, 11, 6.0),
            (60.0, 5, 0.0),
            (222.0, 18, 6.0),
            (340.0, 28, 4.0),
            (11.0, 0, 11.0),
            (12.0, 1, 0.0),
            (0.0, 0, 0.0),
            (1600.0, 128, 64.0),
        ];
        for (height, rows, ground) in table {
            assert_eq!(
                (visible_rows(height), height - rows_bottom(height)),
                (rows, ground),
                "a grid {height} px tall"
            );
        }
        // In the 410 px grid from key 69: keys 69 down to 36 show, key 36's row y 396..408;
        // key 35's would be 408..420, cut to the 2 px of ground, so it shows not and its y is
        // no key's.
        let view = p3_view();
        assert!(view.shows_row(69, 410.0) && view.shows_row(36, 410.0));
        assert!(!view.shows_row(35, 410.0) && !view.shows_row(70, 410.0));
        assert_eq!(view.key_in_rows(407.9, 410.0), Some(36));
        assert_eq!(view.key_in_rows(408.0, 410.0), None, "the ground");
        assert_eq!(view.key_in_rows(-0.5, 410.0), None, "above the grid");
        assert_eq!(view.key_in_rows(0.0, 410.0), Some(69));
        // Folded to three keys, the rows under the third are no key's.
        let folded = View {
            top: 67,
            rows: Rows::of([60, 64, 67]),
            ..view
        };
        assert!(folded.shows_row(60, 410.0) && !folded.shows_row(61, 410.0));
        assert_eq!(folded.key_in_rows(30.0, 410.0), Some(60));
        assert_eq!(folded.key_in_rows(40.0, 410.0), None, "under the keyboard");
    }

    // Where the keys' scroll comes to rest, by hand. The roll scrolls the keys a whole row at a
    // time, the top row's key its scroll, so the top row always starts at the grid's top; asked
    // any key, the top is kept so key 127 is the top row at the highest and key 0 the last
    // whole row at the lowest. In the 410 px grid (34 whole rows, 2 px of ground): asked key
    // −5, the top is 33, key 0's row y (33 − 0) · 12 = 396..408, over the ground's 408..410;
    // asked 200, 127; asked 69, 69. In the 138 px grid (11 rows) the lowest is 10, key 0's row
    // 120..132 over 6 px of ground; in the 60 px grid (5 rows, no ground) 4, key 0's row
    // 48..60. A wheel notch (3 rows) down from 69 rests on 66; down from 34 in the 410 px grid
    // on 33, not 31, which would lift key 0 above the last whole row; up from 126 on 127.
    #[test]
    fn the_keys_scroll_comes_to_rest_on_a_whole_row() {
        let table: [(f32, i32, u8, f32); 6] = [
            (410.0, -5, 33, 408.0),
            (410.0, 200, 127, 408.0),
            (410.0, 69, 69, 408.0),
            (138.0, 0, 10, 132.0),
            (60.0, 2, 4, 60.0),
            (222.0, 3, 17, 216.0),
        ];
        for (height, asked, kept, foot) in table {
            assert_eq!(clamp_top(asked, height), kept, "{height} px, asked {asked}");
            assert_eq!(rows_bottom(height), foot, "{height} px");
            let lowest = clamp_top(0, height);
            let view = View {
                top: lowest,
                ..p3_view()
            };
            assert_eq!(view.row_top(0) + ROW_HEIGHT, foot, "key 0 on the last row");
            assert!(view.shows_row(0, height));
        }
        let at = |top| View { top, ..p3_view() };
        assert_eq!(at(69).top_moved(-3, 410.0), 66);
        assert_eq!(at(34).top_moved(-3, 410.0), 33);
        assert_eq!(at(126).top_moved(3, 410.0), 127);
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

    // 1000 px: 952 px of timeline at 16 px per step is 1428 ticks; 4 bars past a 1-bar pattern
    // is tick 1920, which a view from 1920 − 1428 = 492 ends at; landed up to the next beat,
    // 6·96 = 576. Far out, 4 px a step, the timeline shows 5712 ticks: no scroll.
    #[test]
    fn scrolling_stops_on_the_beat_past_four_bars_past_the_end() {
        let view = p3_view();
        assert_eq!(max_scroll(&view, Ticks(384), 1000.0), Ticks(576));
        let far_out = View {
            zoom: ZOOM.min,
            ..view
        };
        assert_eq!(max_scroll(&far_out, Ticks(384), 1000.0), Ticks(0));
    }

    #[test]
    fn an_open_roll_leaves_the_rack_its_content_up_to_half_the_area() {
        assert_eq!(rack_height_above_roll(84.0, 504.0), 84.0);
        assert_eq!(rack_height_above_roll(400.0, 504.0), 252.0);
    }

    // By hand: the lane is 32 px, a quarter of the old 128, so a pixel is four steps of
    // velocity. Velocity 88 is (32 − y)·128/32 = 88 at y 10, 64 at y 16, the top (y 0) 128 and
    // the bottom 0. Stem A (velocity 100) is 100·32/128 = 25 px tall, from y 7; stem B (64)
    // 16 px, from y 16. (Pins moved from the 64 px default lane: 88 at y 20, A 50 px from y 14,
    // B 32 px from y 32.)
    #[test]
    fn the_lane_is_32_px_on_the_same_scale() {
        assert_eq!(LANE_HEIGHT, 32.0);
        assert_eq!(lane_steps(10.0), 88);
        assert_eq!(lane_steps(16.0), 64);
        assert_eq!(lane_steps(0.0), 128);
        assert_eq!(lane_steps(32.0), 0);
        let view = p3_view();
        let a = view.stem_rect(&note(0, 48, 60), 100).expect("in view");
        assert_eq!((a.x, a.y, a.width, a.height), (48.0, 7.0, 4.0, 25.0));
        let b = view.stem_rect(&note(96, 96, 64), 64).expect("in view");
        assert_eq!((b.x, b.y, b.height), (112.0, 16.0, 16.0));
    }

    // The grid's scroll bar runs the panel's whole width, under the key column too, so the
    // thumb's ends are the panel's at the scroll's ends. By hand, p3 in a 1000 px panel: the
    // view 1428 of the 2004 ticks the grid reaches (below), a thumb of 1428 / 2004 · 1000 =
    // 712.5749 px, with 287.4251 px of travel for the 576 ticks the view moves: at scroll 0
    // from the panel's left edge, x 0; at the middle, 288 (a beat, so a scroll can rest there),
    // from 143.7126; at the end, 576, from 287.4251 to the panel's right edge, 1000. (Before,
    // the rail started at the timeline's x 48: 952 px, a 678.37 px thumb from x 48.)
    #[test]
    fn the_scroll_bars_thumb_runs_the_panels_whole_width() {
        let span_at = |scroll| {
            let view = View {
                scroll: Ticks(scroll),
                ..p3_view()
            };
            time_span(&view, Ticks(384), 1000.0)
        };
        let close = |found: (f32, f32), expected: (f32, f32)| {
            (found.0 - expected.0).abs() < 1e-3 && (found.1 - expected.1).abs() < 1e-3
        };
        let thumb = |scroll| span_at(scroll).thumb(1000.0);
        assert_eq!(span_at(0).last_offset(), 576.0);
        assert!(close(thumb(0), (0.0, 712.574_9)), "{:?}", thumb(0));
        assert!(
            close(thumb(288), (143.712_6, 712.574_9)),
            "{:?}",
            thumb(288)
        );
        let (left, size) = thumb(576);
        assert!(
            close((left, left + size), (287.425_1, 1000.0)),
            "{left} {size}"
        );
    }

    // The grid's scroll along time, by hand. The 1000 px grid shows 952 px of timeline,
    // 1428 ticks at 16 px a step; notes ending at bar 1 (384) let it reach 4 bars further, 1920,
    // and the last view, from the beat 576 (`max_scroll`), on to 576 + 1428 = 2004, which is
    // where the content ends. Scrolled there, the view ends at the content's end. (Pin moved
    // from 1920 and 492, before the last view landed on a beat.)
    #[test]
    fn the_time_span_reaches_four_bars_past_the_notes() {
        let view = p3_view();
        let span = time_span(&view, Ticks(384), 1000.0);
        assert_eq!(
            (span.content, span.offset, span.view),
            (2004.0, 0.0, 1428.0)
        );
        let end = View {
            scroll: max_scroll(&view, Ticks(384), 1000.0),
            ..view
        };
        let span = time_span(&end, Ticks(384), 1000.0);
        assert_eq!(span.offset + span.view, span.content);
        assert_eq!(span.last_offset(), 576.0);
    }
}
