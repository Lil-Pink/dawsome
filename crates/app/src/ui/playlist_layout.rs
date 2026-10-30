//! Every position in the playlist, in logical pixels. The ruler and track canvases draw from
//! these values, the gestures map the cursor with them, and the screenshot probe samples the
//! points they give, so the drawing and its checks cannot drift apart.
//!
//! The panel, top to bottom: the header strip ([`HEADER_HEIGHT`]), the ruler ([`RULER_HEIGHT`]:
//! the bar-number band over the loop strip), the tracks (the rest), and at the foot the
//! timeline's scroll bar ([`TIME_BAR_HEIGHT`]), under the lanes from x 120, the well under the
//! track headers. Why at the foot: a bar over the tracks read as jarring beside every other
//! view's, which sit under what they scroll; it was in the header's room before.
//!
//! Ruler and track coordinates start at their canvas's top-left corner and share x: the track
//! header column is x 0..120 and the timeline starts at x 120, where tick t is at
//! x = 120 + (t − scroll)·px_per_bar/bar_ticks. Row r, counted from the top visible track,
//! spans y = (h+1)r .. (h+1)r + h for the lane height h, with a 1 px gap under it. By default
//! the lanes fit the tracks there are and the "+ Track" row into the track area, 32 to 96 px
//! ([`fitted_lane`], [`Lanes::Fit`]); Alt+wheel steps through 32, 48, 64 and 96 and keeps the
//! height chosen ([`Lanes::Fixed`]). [`Grid::lane`] is the height now, whichever it is.
//!
//! Probe coordinates (window, logical, the playlist alone under the 72 px toolbar, 64 px per
//! bar): the ruler's number band is y 96..112 and its loop strip y 112..124; track 1's lane
//! starts at y 124, bar 2 at x 184; in a 1000 x 560 window the tracks end at y 550 and the bar
//! is y 550..560. With the Library open the playlist starts 300 px right.

use dawsome_engine::project::{Clip, LoopRange};
use dawsome_engine::time::Ticks;
use iced::{Point, Rectangle, Size};

use crate::ui::scroll::{self, Span};

pub const HEADER_HEIGHT: f32 = 24.0;
pub const NUMBER_BAND_HEIGHT: f32 = 16.0;
pub const LOOP_STRIP_HEIGHT: f32 = 12.0;
pub const RULER_HEIGHT: f32 = NUMBER_BAND_HEIGHT + LOOP_STRIP_HEIGHT;
/// The timeline's scroll bar under the tracks: the one bar every scrolling view has.
pub const TIME_BAR_HEIGHT: f32 = scroll::BAR_HEIGHT;

pub const TRACK_HEADER_WIDTH: f32 = 120.0;
/// A drag held over the lanes' last pixels on the right moves the view along as one past their
/// right edge does ([`crate::ui::scroll::edge_step`]). Why inside the lanes: a window that
/// fills the screen ends where the pointer stops, so it can never go past that edge. The left
/// needs none: the track headers are past it.
pub const EDGE_SCROLL_INSET: f32 = 2.0;
/// The lowest lane: the height a lane has before the playlist knows its tracks' room, and the
/// least that fitted lanes shrink to before the tracks scroll.
pub const LANE_HEIGHT: f32 = 32.0;
pub const LANE_GAP: f32 = 1.0;
/// The lane heights Alt+wheel steps through, lowest first; fitted lanes stay between the first
/// and the last.
pub const LANE_HEIGHTS: [f32; 4] = [32.0, 48.0, 64.0, 96.0];
const TALLEST_LANE: f32 = LANE_HEIGHTS[LANE_HEIGHTS.len() - 1];

/// How tall the playlist's lanes are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Lanes {
    /// The tracks there are and the "+ Track" row fill the track area ([`fitted_lane`]): a
    /// new project's few tracks are tall, and they shrink as tracks are added, down to
    /// [`LANE_HEIGHT`], then scroll. The default.
    Fit,
    /// The height chosen with Alt+wheel, kept whatever the tracks and the room.
    Fixed(f32),
}

/// The lane height that fits `tracks` tracks and the "+ Track" row under them into a track
/// area `height` tall: the whole pixels of each row's share less the gap under it, kept to
/// [`LANE_HEIGHT`]..96.
pub fn fitted_lane(tracks: usize, height: f32) -> f32 {
    let rows = (tracks + 1) as f32;
    kept_lane((height / rows).floor() - LANE_GAP)
}

/// `height` kept to the lane heights there are, [`LANE_HEIGHT`]..96 (a stored height that is
/// out of range reads back so).
pub fn kept_lane(height: f32) -> f32 {
    height.clamp(LANE_HEIGHT, TALLEST_LANE)
}

pub const LED_CENTER_X: f32 = 10.0;
pub const LED_RADIUS: f32 = 5.0;
/// The mute light takes clicks this far either side of its center; the light itself is small.
pub const LED_HIT_HALF_WIDTH: f32 = 9.0;
/// Track names start here.
pub const NAME_X: f32 = 22.0;
/// The arm light at each header's right end (Record's takes land on its track), its radius,
/// and how far either side of its center a press reaches it.
pub const ARM_CENTER_X: f32 = TRACK_HEADER_WIDTH - 12.0;
pub const ARM_RADIUS: f32 = 4.0;
pub const ARM_HIT_HALF_WIDTH: f32 = 8.0;
pub const NAME_SIZE: f32 = 12.0;
/// The "+ Track" button under the last track: its inset from the column's edges and height.
pub const ADD_TRACK_INSET: f32 = 6.0;
pub const ADD_TRACK_HEIGHT: f32 = 20.0;

/// Bar numbers start this far right of their bar line.
pub const LABEL_INSET: f32 = 3.0;
pub const LABEL_SIZE: f32 = 11.0;

/// A clip's title strip, over its body.
pub const TITLE_HEIGHT: f32 = 10.0;
pub const TITLE_SIZE: f32 = 9.0;
pub const NOTE_MARK_WIDTH: f32 = 2.0;
pub const SELECTED_OUTLINE: f32 = 2.0;

/// A press this close to a clip's right edge resizes the clip (an audio clip's left edge too:
/// it trims), if it is at least [`RESIZE_MIN_WIDTH`] wide; anywhere else on it selects and
/// moves it.
pub const RESIZE_EDGE: f32 = 6.0;
pub const RESIZE_MIN_WIDTH: f32 = 12.0;

/// An audio clip's gain readout: this wide at the right end of its title strip.
pub const GAIN_READOUT_WIDTH: f32 = 44.0;

/// An automation clip's value line runs this far inside its body: value 1 at the body's top
/// plus this, value 0 at its bottom minus this.
pub const AUTOMATION_INSET: f32 = 2.0;
/// Its line, points and tension handles.
pub const AUTOMATION_LINE: f32 = 1.5;
pub const POINT_RADIUS: f32 = 3.5;
pub const HANDLE_RADIUS: f32 = 3.0;
/// A press this close to a point or a handle takes it.
pub const POINT_REACH: f32 = 6.0;
/// A loop-strip press this close to an edge of the loop range moves that edge.
pub const LOOP_EDGE: f32 = 4.0;

/// The song-position marker: a downward triangle this wide in the number band.
pub const MARKER_WIDTH: f32 = 7.0;

/// Pixels per bar at each zoom level, from farthest out to closest in.
pub const ZOOM_LEVELS: [f32; 5] = [16.0, 32.0, 64.0, 128.0, 256.0];
/// 64 pixels per bar.
pub const DEFAULT_ZOOM: usize = 2;

/// Beat lines are drawn only when beats are at least this far apart.
pub const MIN_BEAT_SPACING: f32 = 8.0;

/// Which part of the timeline and which tracks the playlist shows, and how ticks map to x.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    /// The tick at the timeline's left edge (x = [`TRACK_HEADER_WIDTH`]); never negative.
    pub scroll: Ticks,
    /// Index into [`ZOOM_LEVELS`].
    pub zoom: usize,
    /// Ticks in one bar, at least 1.
    pub bar: Ticks,
    /// Index, in track order, of the track in the top row.
    pub top: usize,
    /// The lane height now, [`LANE_HEIGHT`]..96: fitted, or one of [`LANE_HEIGHTS`] chosen.
    /// The one height the drawing and every hit test read.
    pub lane: f32,
}

impl Grid {
    pub fn px_per_bar(&self) -> f32 {
        ZOOM_LEVELS[self.zoom.min(ZOOM_LEVELS.len() - 1)]
    }

    /// Ticks in one beat (a quarter of a bar), at least 1.
    pub fn beat(&self) -> Ticks {
        Ticks((self.bar.0 / 4).max(1))
    }

    /// The x where `tick` starts.
    pub fn x_of(&self, tick: Ticks) -> f32 {
        let ticks = (tick.0 - self.scroll.0) as f64;
        let x = f64::from(TRACK_HEADER_WIDTH) + ticks * f64::from(self.px_per_bar()) / self.ticks();
        x as f32
    }

    /// The tick under x: floor((x − 120)·bar_ticks/px_per_bar) + scroll.
    pub fn tick_at(&self, x: f32) -> Ticks {
        let ticks = (f64::from(x) - f64::from(TRACK_HEADER_WIDTH)) * self.ticks()
            / f64::from(self.px_per_bar());
        Ticks(ticks.floor() as i64 + self.scroll.0)
    }

    /// The x where `now` shows what this grid shows at `x`: the same place on the timeline,
    /// however `now` has scrolled or zoomed since; `x` itself when it has not.
    pub fn x_in(&self, x: f32, now: &Grid) -> f32 {
        if (self.scroll, self.zoom, self.bar) == (now.scroll, now.zoom, now.bar) {
            return x;
        }
        let header = f64::from(TRACK_HEADER_WIDTH);
        let tick = (f64::from(x) - header) * self.ticks() / f64::from(self.px_per_bar())
            + self.scroll.0 as f64;
        let x = header + (tick - now.scroll.0 as f64) * f64::from(now.px_per_bar()) / now.ticks();
        x as f32
    }

    /// The track index of the row under y (a gap belongs to the row above it); negative above
    /// the first row.
    pub fn track_index_at(&self, y: f32) -> i64 {
        self.top as i64 + (y / self.row_pitch()).floor() as i64
    }

    /// The top of the row showing track `index`, or `None` for a track scrolled above the top.
    pub fn row_top_of(&self, index: usize) -> Option<f32> {
        index.checked_sub(self.top).map(|row| self.row_top(row))
    }

    /// From one row's top to the next.
    pub fn row_pitch(&self) -> f32 {
        self.lane + LANE_GAP
    }

    /// The top of row `row`, counted from the top visible track.
    pub fn row_top(&self, row: usize) -> f32 {
        row as f32 * self.row_pitch()
    }

    /// Rows at least partly inside a track area `height` tall.
    pub fn visible_rows(&self, height: f32) -> usize {
        (height / self.row_pitch()).ceil().max(0.0) as usize
    }

    /// The next of [`LANE_HEIGHTS`] above the lane height now (`taller`) or below it, staying
    /// within them: from a fitted 42 px, 48 or 32.
    pub fn stepped_lane(&self, taller: bool) -> f32 {
        let next = if taller {
            LANE_HEIGHTS.into_iter().find(|&height| height > self.lane)
        } else {
            LANE_HEIGHTS
                .into_iter()
                .rev()
                .find(|&height| height < self.lane)
        };
        next.unwrap_or(if taller { TALLEST_LANE } else { LANE_HEIGHT })
    }

    fn ticks(&self) -> f64 {
        self.bar.0.max(1) as f64
    }
}

/// `clip` drawn in the row whose top is `top`: from its start to its end, a lane tall.
pub fn clip_rect(grid: &Grid, clip: &Clip, top: f32) -> Rectangle {
    let left = grid.x_of(clip.start);
    let right = grid.x_of(clip.end());
    Rectangle::new(Point::new(left, top), Size::new(right - left, grid.lane))
}

/// What part of a clip a press landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipPart {
    Body,
    /// The resize handle along its right edge.
    RightEdge,
    /// An audio clip's trim handle along its left edge.
    LeftEdge,
}

/// The part of `rect` (a drawn clip) under x: the right edge within [`RESIZE_EDGE`] of it (and
/// the left one, when `left_edge`), if the clip is wide enough to have them, else the body;
/// `None` outside it.
pub fn clip_part(rect: Rectangle, x: f32, left_edge: bool) -> Option<ClipPart> {
    if !(rect.x..rect.x + rect.width).contains(&x) {
        return None;
    }
    let wide = rect.width >= RESIZE_MIN_WIDTH;
    Some(if wide && rect.x + rect.width - x <= RESIZE_EDGE {
        ClipPart::RightEdge
    } else if wide && left_edge && x - rect.x < RESIZE_EDGE {
        ClipPart::LeftEdge
    } else {
        ClipPart::Body
    })
}

/// The y of normalized automation value `value` in a clip body from `top` to `bottom`.
pub fn automation_y(top: f32, bottom: f32, value: f32) -> f32 {
    let (high, low) = (top + AUTOMATION_INSET, bottom - AUTOMATION_INSET);
    low - value.clamp(0.0, 1.0) * (low - high)
}

/// The normalized automation value at y in a clip body from `top` to `bottom`, kept in 0..1.
pub fn automation_value(top: f32, bottom: f32, y: f32) -> f32 {
    let (high, low) = (top + AUTOMATION_INSET, bottom - AUTOMATION_INSET);
    if low <= high {
        return 0.0;
    }
    ((low - y) / (low - high)).clamp(0.0, 1.0)
}

/// Whether `rect` meets the box between points `a` and `b`.
pub fn meets_box(rect: Rectangle, a: Point, b: Point) -> bool {
    let (left, right) = (a.x.min(b.x), a.x.max(b.x));
    let (top, bottom) = (a.y.min(b.y), a.y.max(b.y));
    rect.x <= right
        && left <= rect.x + rect.width
        && rect.y <= bottom
        && top <= rect.y + rect.height
}

/// What a press in the track header column landed on, by row from the top.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderHit {
    Led {
        row: usize,
    },
    Name {
        row: usize,
    },
    /// The arm light at the header's right end.
    Arm {
        row: usize,
    },
    AddTrack,
}

/// The header control under `point` with `rows` tracks shown from the top row on (the "+
/// Track" button sits under the last of them).
pub fn header_hit(grid: &Grid, point: Point, rows: usize) -> Option<HeaderHit> {
    if !(0.0..TRACK_HEADER_WIDTH).contains(&point.x) || point.y < 0.0 {
        return None;
    }
    if add_track_rect(grid, rows).contains(point) {
        return Some(HeaderHit::AddTrack);
    }
    let row = (point.y / grid.row_pitch()).floor() as usize;
    if row >= rows || point.y - grid.row_top(row) >= grid.lane {
        return None;
    }
    let led = (point.x - LED_CENTER_X).abs() <= LED_HIT_HALF_WIDTH;
    let arm = (point.x - ARM_CENTER_X).abs() <= ARM_HIT_HALF_WIDTH;
    Some(if led {
        HeaderHit::Led { row }
    } else if arm {
        HeaderHit::Arm { row }
    } else {
        HeaderHit::Name { row }
    })
}

/// The center of row `row`'s arm light.
pub fn arm_center(grid: &Grid, row: usize) -> Point {
    Point::new(ARM_CENTER_X, grid.row_top(row) + grid.lane / 2.0)
}

/// Where the tracks canvas lies in the panel laid out at `panel` (window coordinates): under the
/// header strip and the ruler, over the scroll bar, the panel's width.
pub fn tracks_rect(panel: Rectangle) -> Rectangle {
    let top = HEADER_HEIGHT + RULER_HEIGHT;
    Rectangle::new(
        Point::new(panel.x, panel.y + top),
        Size::new(panel.width, (panel.height - top - TIME_BAR_HEIGHT).max(0.0)),
    )
}

/// The "+ Track" button under the last of `rows` rows.
pub fn add_track_rect(grid: &Grid, rows: usize) -> Rectangle {
    Rectangle::new(
        Point::new(ADD_TRACK_INSET, grid.row_top(rows) + ADD_TRACK_INSET),
        Size::new(TRACK_HEADER_WIDTH - 2.0 * ADD_TRACK_INSET, ADD_TRACK_HEIGHT),
    )
}

/// The center of row `row`'s mute light.
pub fn led_center(grid: &Grid, row: usize) -> Point {
    Point::new(LED_CENTER_X, grid.row_top(row) + grid.lane / 2.0)
}

/// Which part of the ruler a y falls in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RulerBand {
    Numbers,
    LoopStrip,
}

/// The ruler band under y, `None` outside the ruler.
pub fn ruler_band(y: f32) -> Option<RulerBand> {
    if (0.0..NUMBER_BAND_HEIGHT).contains(&y) {
        Some(RulerBand::Numbers)
    } else if (NUMBER_BAND_HEIGHT..RULER_HEIGHT).contains(&y) {
        Some(RulerBand::LoopStrip)
    } else {
        None
    }
}

/// An end of the loop range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopEdge {
    Start,
    End,
}

/// The edge of `range` within [`LOOP_EDGE`] of x, the nearer one when both are.
pub fn loop_edge_under(grid: &Grid, range: &LoopRange, x: f32) -> Option<LoopEdge> {
    let start = (grid.x_of(range.start) - x).abs();
    let end = (grid.x_of(range.end) - x).abs();
    match (start <= LOOP_EDGE, end <= LOOP_EDGE) {
        (true, true) if end < start => Some(LoopEdge::End),
        (true, _) => Some(LoopEdge::Start),
        (false, true) => Some(LoopEdge::End),
        (false, false) => None,
    }
}

/// The bars whose line is inside the timeline of a canvas `width` wide, as (bar number counted
/// from 1, line x).
pub fn visible_bars(grid: &Grid, width: f32) -> impl Iterator<Item = (i64, f32)> + '_ {
    let bar = grid.bar.0.max(1);
    let first = grid.scroll.0.div_euclid(bar) + i64::from(grid.scroll.0 % bar != 0);
    let last = grid.tick_at(width).0.div_euclid(bar);
    (first..=last).map(move |index| (index + 1, grid.x_of(Ticks(index * bar))))
}

/// How far past the song's end the scroll bar reaches.
pub const BARS_PAST_END: i64 = 4;

/// The timeline's scroll, in ticks, for a song that ends at `end`, on a tracks canvas `width`
/// wide: the content reaches [`BARS_PAST_END`] bars past the end, the view is the whole ticks
/// the timeline shows.
pub fn time_span(grid: &Grid, end: Ticks, width: f32) -> Span {
    let timeline = f64::from((width - TRACK_HEADER_WIDTH).max(0.0));
    let view = (timeline * grid.ticks() / f64::from(grid.px_per_bar())).floor();
    Span {
        content: (end.0 + BARS_PAST_END * grid.bar.0) as f64,
        offset: grid.scroll.0 as f64,
        view,
    }
}

/// The playlist's track area when the playlist is the only panel in a window of `window` size:
/// what the header, the ruler and the scroll bar under the tracks leave.
pub fn tracks_size_alone(window: Size, toolbar: f32) -> Size {
    let height = window.height - toolbar - HEADER_HEIGHT - RULER_HEIGHT - TIME_BAR_HEIGHT;
    Size::new(window.width, height.max(0.0))
}

#[cfg(test)]
mod tests {
    use dawsome_engine::project::{PatternId, TrackId};

    use super::*;

    /// The grid as the playlist opens: no scroll, 64 px per bar, a bar of 384 ticks.
    fn grid() -> Grid {
        Grid {
            scroll: Ticks(0),
            zoom: DEFAULT_ZOOM,
            bar: Ticks(384),
            top: 0,
            lane: LANE_HEIGHT,
        }
    }

    fn clip(start: i64, length: i64) -> Clip {
        Clip::of_pattern(PatternId(1), TrackId(1), Ticks(start), Ticks(length))
    }

    // The design's geometry: x = 120 + tick·64/384, so bar b's line is at 120 + 64b; track rows are
    // 32 px tall with a 1 px gap, so track 8's lane is y 231..263 (window 339..371).
    #[test]
    fn ticks_and_rows_land_where_the_design_draws_them() {
        let grid = grid();
        assert_eq!(grid.x_of(Ticks(0)), 120.0);
        assert_eq!(grid.x_of(Ticks(384)), 184.0);
        assert_eq!(grid.x_of(Ticks(768)), 248.0);
        assert_eq!(grid.x_of(Ticks(96)), 136.0, "a beat is 16 px");
        assert_eq!(grid.row_top(1), 33.0);
        assert_eq!(grid.row_top(7), 231.0);
        let bars: Vec<(i64, f32)> = visible_bars(&grid, 1000.0).collect();
        assert_eq!(bars.first(), Some(&(1, 120.0)));
        assert_eq!(bars[12], (13, 888.0));
        assert_eq!(bars.last(), Some(&(14, 952.0)));
        let rect = clip_rect(&grid, &clip(768, 384), grid.row_top(0));
        assert_eq!(
            (rect.x, rect.width, rect.y, rect.height),
            (248.0, 64.0, 0.0, 32.0)
        );
    }

    // By hand: x 200 at no scroll is tick (200 − 120)·6 = 480. Scrolled a bar on (384), the
    // view shows it at 120 + (480 − 384)/6 = 136; zoomed to 128 px a bar (3 ticks a pixel) at no
    // scroll, at 120 + 480/3 = 280; an unchanged view keeps x as it is.
    #[test]
    fn a_place_pressed_is_found_again_in_a_view_scrolled_or_zoomed() {
        let then = grid();
        let scrolled = Grid {
            scroll: Ticks(384),
            ..then
        };
        assert_eq!(then.x_in(200.0, &scrolled), 136.0);
        assert_eq!(scrolled.x_in(136.0, &then), 200.0);
        let zoomed = Grid { zoom: 3, ..then };
        assert_eq!(then.x_in(200.0, &zoomed), 280.0);
        assert_eq!(then.x_in(200.3, &then), 200.3);
        let lower = Grid { top: 4, ..then };
        assert_eq!(then.x_in(200.3, &lower), 200.3, "rows are not time");
    }

    // tick_at(x) = floor((x − 120)·384/64) + scroll: 6 ticks a pixel at 64 px per bar.
    #[test]
    fn tick_at_floors_the_ticks_under_a_pixel() {
        let grid = grid();
        assert_eq!(grid.tick_at(120.0), Ticks(0));
        assert_eq!(grid.tick_at(181.0), Ticks(366));
        assert_eq!(grid.tick_at(309.0), Ticks(1134));
        assert_eq!(grid.tick_at(119.0), Ticks(-6), "left of the timeline");
        let scrolled = Grid {
            scroll: Ticks(768),
            zoom: 0,
            ..grid
        };
        // 16 px per bar is 24 ticks a pixel.
        assert_eq!(scrolled.tick_at(130.0), Ticks(768 + 240));
        assert_eq!(scrolled.x_of(Ticks(1008)), 130.0);
        let bars: Vec<i64> = visible_bars(
            &Grid {
                scroll: Ticks(400),
                ..grid
            },
            300.0,
        )
        .map(|(number, _)| number)
        .collect();
        assert_eq!(bars, [3, 4], "bar 2's line (tick 384) is scrolled off");
    }

    #[test]
    fn rows_map_to_track_indices_from_the_top_one() {
        let grid = Grid { top: 3, ..grid() };
        assert_eq!(grid.track_index_at(0.0), 3);
        assert_eq!(
            grid.track_index_at(32.5),
            3,
            "the gap belongs to the row above"
        );
        assert_eq!(grid.track_index_at(33.0), 4);
        assert_eq!(grid.track_index_at(-1.0), 2);
        assert_eq!(grid.row_top_of(4), Some(33.0));
        assert_eq!(grid.row_top_of(2), None);
    }

    // A clip 64 px wide (x 120..184): its last 6 px are the resize handle.
    #[test]
    fn a_press_finds_the_clip_part_under_it() {
        let rect = clip_rect(&grid(), &clip(0, 384), 0.0);
        assert_eq!(clip_part(rect, 120.0, false), Some(ClipPart::Body));
        assert_eq!(clip_part(rect, 177.9, false), Some(ClipPart::Body));
        assert_eq!(clip_part(rect, 178.0, false), Some(ClipPart::RightEdge));
        assert_eq!(clip_part(rect, 183.9, false), Some(ClipPart::RightEdge));
        assert_eq!(
            clip_part(rect, 184.0, false),
            None,
            "the right edge itself is outside"
        );
        assert_eq!(clip_part(rect, 119.0, false), None);
        // Under 12 px wide a clip has no resize handle: 48 ticks is 8 px.
        let narrow = clip_rect(&grid(), &clip(0, 48), 0.0);
        assert_eq!(clip_part(narrow, 127.0, false), Some(ClipPart::Body));
        // An audio clip trims at its left edge too: its first 6 px.
        assert_eq!(clip_part(rect, 125.9, true), Some(ClipPart::LeftEdge));
        assert_eq!(clip_part(rect, 126.0, true), Some(ClipPart::Body));
        assert_eq!(clip_part(narrow, 121.0, true), Some(ClipPart::Body));
    }

    // The probe coordinates above: the playlist alone in a 1000 x 560 window is the panel from
    // y 72 under the toolbar, 488 px tall; its tracks start at y 124 and end at y 550, over the
    // 10 px bar.
    #[test]
    fn the_tracks_lie_between_the_ruler_and_the_bar() {
        let panel = Rectangle::new(Point::new(0.0, 72.0), Size::new(1000.0, 488.0));
        assert_eq!(
            tracks_rect(panel),
            Rectangle::new(Point::new(0.0, 124.0), Size::new(1000.0, 426.0))
        );
    }

    #[test]
    fn header_presses_find_the_light_the_name_and_the_add_button() {
        let at = |x, y| header_hit(&grid(), Point::new(x, y), 8);
        assert_eq!(at(10.0, 16.0), Some(HeaderHit::Led { row: 0 }));
        assert_eq!(at(18.0, 49.0), Some(HeaderHit::Led { row: 1 }));
        assert_eq!(at(60.0, 49.0), Some(HeaderHit::Name { row: 1 }));
        assert_eq!(at(60.0, 32.5), None, "the gap under a row");
        // The button sits under row 8: y 264 + 6 .. + 20.
        assert_eq!(at(60.0, 275.0), Some(HeaderHit::AddTrack));
        assert_eq!(at(60.0, 300.0), None);
        assert_eq!(at(130.0, 16.0), None, "the timeline");
        assert_eq!(led_center(&grid(), 1), Point::new(10.0, 49.0));
    }

    // Loop 384..768 is drawn x 184..248; presses within 4 px of an edge move it.
    #[test]
    fn loop_presses_find_the_edge_near_them() {
        let range = LoopRange {
            start: Ticks(384),
            end: Ticks(768),
            on: true,
        };
        let grid = grid();
        assert_eq!(loop_edge_under(&grid, &range, 186.0), Some(LoopEdge::Start));
        assert_eq!(loop_edge_under(&grid, &range, 250.0), Some(LoopEdge::End));
        assert_eq!(loop_edge_under(&grid, &range, 244.0), Some(LoopEdge::End));
        assert_eq!(loop_edge_under(&grid, &range, 208.0), None);
        // A range 6 px wide: both edges are near; the nearer one wins.
        let narrow = LoopRange {
            start: Ticks(384),
            end: Ticks(420),
            ..range
        };
        assert_eq!(loop_edge_under(&grid, &narrow, 189.0), Some(LoopEdge::End));
        assert_eq!(
            loop_edge_under(&grid, &narrow, 186.0),
            Some(LoopEdge::Start)
        );
        assert_eq!(ruler_band(5.0), Some(RulerBand::Numbers));
        assert_eq!(ruler_band(16.0), Some(RulerBand::LoopStrip));
        assert_eq!(ruler_band(28.0), None);
    }

    #[test]
    fn a_box_meets_the_clips_it_touches() {
        let rect = clip_rect(&grid(), &clip(0, 384), 0.0);
        assert!(meets_box(
            rect,
            Point::new(100.0, 40.0),
            Point::new(130.0, 10.0)
        ));
        assert!(!meets_box(
            rect,
            Point::new(190.0, 0.0),
            Point::new(300.0, 30.0)
        ));
        assert!(!meets_box(
            rect,
            Point::new(130.0, 33.0),
            Point::new(140.0, 60.0)
        ));
    }

    // Alone in a 1000x560 window the track area is 560 − 72 − 24 − 28 − 10 = 426 tall (the
    // scroll bar's 10 px under it): 13 rows of 32 px show (12.9), and 8 tracks fit in 46 px
    // lanes (426 / 9 = 47.3, less the gap). (Pins moved: 452 under the 56 px toolbar,
    // then 436 and 14 rows with the bar in the header.)
    #[test]
    fn the_track_area_fills_the_panel_under_the_ruler() {
        let size = tracks_size_alone(Size::new(1000.0, 560.0), 72.0);
        assert_eq!(size, Size::new(1000.0, 426.0));
        assert_eq!(grid().visible_rows(426.0), 13);
        assert_eq!(grid().visible_rows(33.0), 1);
        assert_eq!(fitted_lane(8, 426.0), 46.0);
    }

    // By hand: lanes that fit share the track area among the tracks and the
    // "+ Track" row, whole pixels less the 1 px gap. The 436 px area holds 8 tracks and the row
    // in 9 rows of 48 px (436 / 9 = 48.4): 47 px lanes. Two tracks in 3 rows would be 144 px,
    // kept to 96; 30 tracks in 31 rows (14 px) keep 32 and scroll. 13 tracks in 14 rows of
    // 31 px (436 / 14 = 31.1) would be 30, kept to 32. A height out of range reads back kept.
    #[test]
    fn fitted_lanes_share_the_track_area_among_the_tracks() {
        assert_eq!(fitted_lane(8, 436.0), 47.0);
        assert_eq!(fitted_lane(2, 436.0), 96.0);
        assert_eq!(fitted_lane(30, 436.0), 32.0);
        assert_eq!(fitted_lane(13, 436.0), 32.0);
        assert_eq!(fitted_lane(0, 0.0), 32.0, "no room yet");
        let fitted = Grid {
            lane: fitted_lane(8, 436.0),
            ..grid()
        };
        assert_eq!(
            fitted.row_top(9),
            432.0,
            "the + Track row ends inside the area"
        );
        assert_eq!(fitted.track_index_at(47.5), 0, "the gap under row 0");
        assert_eq!(fitted.track_index_at(48.0), 1);
        assert_eq!((kept_lane(300.0), kept_lane(4.0)), (96.0, 32.0));
    }

    // P6, by hand: at 64 px a bar a 1000 px tracks canvas shows 880 px of timeline, 13.75 bars
    // (5280 ticks); a song ending at bar 3 (tick 1152) reaches 4 bars further, 2688.
    #[test]
    fn the_time_span_reaches_four_bars_past_the_song() {
        let span = time_span(&grid(), Ticks(1152), 1000.0);
        assert_eq!(
            (span.content, span.offset, span.view),
            (2688.0, 0.0, 5280.0)
        );
        let scrolled = Grid {
            scroll: Ticks(3000),
            ..grid()
        };
        assert_eq!(time_span(&scrolled, Ticks(1152), 1000.0).extent(), 8280.0);
    }

    // Alt+wheel steps the lanes through 32, 48, 64 and 96 px, rows following.
    #[test]
    fn lane_heights_step_and_rows_follow_them() {
        let mut grid = grid();
        assert_eq!(grid.stepped_lane(false), 32.0, "already the lowest");
        grid.lane = grid.stepped_lane(true);
        assert_eq!(grid.lane, 48.0);
        assert_eq!((grid.row_top(1), grid.track_index_at(49.0)), (49.0, 1));
        grid.lane = 96.0;
        assert_eq!(grid.stepped_lane(true), 96.0, "already the highest");
        assert_eq!(grid.stepped_lane(false), 64.0);
        // From a fitted height between two steps, the step above or below.
        grid.lane = 42.0;
        assert_eq!(
            (grid.stepped_lane(true), grid.stepped_lane(false)),
            (48.0, 32.0)
        );
    }

    // The value line: 1 at the body's top + 2, 0 at its bottom − 2, and back.
    #[test]
    fn automation_values_map_to_the_body_and_back() {
        assert_eq!(automation_y(151.0, 173.0, 1.0), 153.0);
        assert_eq!(automation_y(151.0, 173.0, 0.0), 171.0);
        assert_eq!(automation_y(151.0, 173.0, 0.5), 162.0);
        assert_eq!(automation_value(151.0, 173.0, 162.0), 0.5);
        assert_eq!(automation_value(151.0, 173.0, 100.0), 1.0, "kept in range");
    }
}
