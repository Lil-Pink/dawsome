//! Every position in the playlist, in logical pixels. The ruler and track canvases draw from
//! these values, the gestures map the cursor with them, and the screenshot probe samples the
//! points they give, so the drawing and its checks cannot drift apart.
//!
//! The panel, top to bottom: the header strip ([`HEADER_HEIGHT`]), the ruler ([`RULER_HEIGHT`]:
//! the bar-number band over the loop strip), the tracks (the rest), and at the foot the
//! timeline's scroll bar ([`TIME_BAR_HEIGHT`]), its rail the panel's whole width, under the
//! track headers too, so its thumb starts at the panel's left edge at the song's start. Why at
//! the foot: a bar over the tracks read as jarring beside every other view's, which sit under
//! what they scroll; it was in the header's room before. Why the whole width: from x 120 the
//! thumb at the start read as already scrolled.
//!
//! Ruler and track coordinates start at their canvas's top-left corner and share x: the track
//! header column is x 0..120 and the timeline starts at x 120, where tick t is at
//! x = 120 + (t − scroll)·px_per_bar/bar_ticks. Row r, counted from the top visible track,
//! spans y = (h+1)r .. (h+1)r + h for the lane height h, with a 1 px gap under it. By default
//! the lanes fit the tracks there are and the "+ Track" row into the track area, 32 to 106 px
//! ([`fitted_lane`], [`Lanes::Fit`]), the "+ Track" row taking its own 24 px and the tracks the
//! rest; Alt+wheel steps through 32, 48, 64, 85 and 106 and keeps the height chosen
//! ([`Lanes::Fixed`]). [`Grid::lane`] is the height now, whichever it is. Why 106 (96 before,
//! and the "+ Track" row a lane's share): the lanes were asked about a tenth taller, open alone
//! and stacked, where the row under "+ Track" left a lane's height empty.
//! Only the rows the track area holds whole show ([`Grid::rows`]), headers and lanes alike: a
//! row its foot would cut is left as the well, and comes into view whole as the tracks scroll,
//! a row a notch; the "+ Track" row shows when its 24 px fit whole under the last track
//! ([`Grid::shows_add_track`]).
//!
//! The zoom is any number of pixels per bar from 16 to 256 ([`Grid::zoom`], [`ZOOM`]). The lanes
//! draw a bar as four beats by the grid rules the piano roll shares ([`grid_rules`]): beat lines
//! fading in as the beats grow apart ([`grid_rules::beat_alpha`]), bar lines, a stronger line at
//! the start of every four bars ([`grid_rules::strong`]) and every other bar shaded, or every
//! other four bars when far out ([`grid_rules::shaded`]). A scroll along time comes to rest on a
//! whole cell, a beat while beat lines are drawn and else a bar ([`Grid::landed`]), so the view
//! never starts on a sliver of one.
//!
//! Probe coordinates (window, logical, the playlist alone under the 72 px toolbar, 64 px per
//! bar): the ruler's number band is y 96..112 and its loop strip y 112..124; track 1's lane
//! starts at y 124, bar 2 at x 184; in a 1000 x 560 window the tracks end at y 550 and the bar
//! is y 550..560. With the Library open the playlist starts 300 px right.

use std::sync::OnceLock;

use dawsome_engine::project::{Clip, LoopRange};
use dawsome_engine::time::{MeterMap, MeterUnit, SignatureChange, Ticks, TimeSignature};
use iced::{Point, Rectangle, Size};

use crate::ui::add_words;
use crate::ui::fonts::size;
use crate::ui::grid_rules::{self, ZoomRange, NUMBER_BAND_HEIGHT};
use crate::ui::scroll::{self, Span};
use crate::ui::text_fit::text_width;

pub const HEADER_HEIGHT: f32 = 24.0;
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
/// lowest of the heights Alt+wheel chooses ([`LANE_HEIGHTS`]).
pub const LANE_HEIGHT: f32 = 32.0;
/// The least that fitted lanes shrink to before the tracks scroll ([`fitted_lane`]): the
/// ladder's bottom, [`LANE_HEIGHT`], so the lanes squeeze all the way as tracks are added. Why
/// not higher: a floor of 64 px, the lowest tracks scrolling off below it, was tried and the
/// full squeeze preferred; one number, should it be tried again.
pub const FIT_FLOOR: f32 = LANE_HEIGHT;
pub const LANE_GAP: f32 = 1.0;
/// The lane heights Alt+wheel steps through, lowest first; fitted lanes stay between the first
/// and the last. The last is the cap, 106: 96 (the cap before) a tenth taller, 105.6, rounded
/// up; 85 between 64 and it keeps the steps about even (16, 16, 21, 21).
pub const LANE_HEIGHTS: [f32; 5] = [32.0, 48.0, 64.0, 85.0, 106.0];
const TALLEST_LANE: f32 = LANE_HEIGHTS[LANE_HEIGHTS.len() - 1];

/// How tall the playlist's lanes are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Lanes {
    /// The tracks there are and the "+ Track" row fill the track area ([`fitted_lane`]): a
    /// new project's few tracks are tall, up to 106 px, and they shrink as tracks are added,
    /// down to [`FIT_FLOOR`], 32 px, then scroll. The default.
    Fit,
    /// The height chosen with Alt+wheel, kept whatever the tracks and the room.
    Fixed(f32),
}

/// The lane height that fits `tracks` tracks and the "+ Track" row under them into a track
/// area `height` tall: the "+ Track" row takes its own height and a gap, the tracks share the
/// rest, each the whole pixels of its share less the gap under it, kept to
/// [`FIT_FLOOR`]..106: floor((height − 24 − 1) / tracks) − 1. Why not a lane's share for the
/// "+ Track" row (height / (tracks + 1), as before): the 24 px button left the rest of that
/// share empty, about a lane under it, which the lanes now take. Under the floor the tracks
/// scroll instead.
pub fn fitted_lane(tracks: usize, height: f32) -> f32 {
    let room = height - ADD_TRACK_HEIGHT - LANE_GAP;
    let share = room / tracks.max(1) as f32;
    (share.floor() - LANE_GAP).clamp(FIT_FLOOR, TALLEST_LANE)
}

/// `height` kept to the lane heights there are, [`LANE_HEIGHT`]..106 (a stored height that is
/// out of range reads back so).
pub fn kept_lane(height: f32) -> f32 {
    height.clamp(LANE_HEIGHT, TALLEST_LANE)
}

pub const LED_CENTER_X: f32 = 10.0;
pub const LED_RADIUS: f32 = 5.0;
/// The mute light takes clicks this far either side of its center, a 24 px target (was 18); the
/// light itself is small.
pub const LED_HIT_HALF_WIDTH: f32 = 12.0;
/// Track names start here.
pub const NAME_X: f32 = 22.0;
/// The arm light at each header's right end (Record's takes land on its track), its radius,
/// and how far either side of its center a press reaches it: a 24 px target (was 16).
pub const ARM_CENTER_X: f32 = TRACK_HEADER_WIDTH - 12.0;
pub const ARM_RADIUS: f32 = 4.0;
pub const ARM_HIT_HALF_WIDTH: f32 = 12.0;
/// The "+ Track" control under the last track: its words alone ([`add_words`]), centred in the
/// header column, and its row's height, 24 px (was 20): it stands right under the last track's
/// lane and the gap under it, and fitted lanes leave it that room ([`fitted_lane`]). (It stood
/// 6 px down a row a lane tall before, then was a button 6 px in from the column's sides.)
pub const ADD_TRACK_LABEL: &str = "+ Track";
pub const ADD_TRACK_HEIGHT: f32 = add_words::HEIGHT;

/// A clip's title strip, over its body.
pub const TITLE_HEIGHT: f32 = 10.0;
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

/// The zoom, in pixels per bar: 16 to 256, 64 to start. − and + halve and double it
/// ([`ZoomRange::stepped`]), so from the start they reach 16, 32, 128 and 256, the stops the zoom
/// had before it was continuous; a Ctrl+wheel notch moves it a quarter of a doubling
/// ([`ZoomRange::wheeled`]).
pub const ZOOM: ZoomRange = ZoomRange {
    min: 16.0,
    max: 256.0,
    default: 64.0,
};

/// Which part of the timeline and which tracks the playlist shows, and how ticks map to x.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    /// The tick at the timeline's left edge (x = [`TRACK_HEADER_WIDTH`]); never negative.
    pub scroll: Ticks,
    /// Pixels per bar, in [`ZOOM`]'s range.
    pub zoom: f32,
    /// Ticks in one bar of the song's first meter (the project's time signature), at least 1:
    /// the zoom's pixels per bar are this many ticks. The bars and beats drawn are the song's
    /// meter map's ([`visible_bars`]).
    pub bar: Ticks,
    /// Index, in track order, of the track in the top row.
    pub top: usize,
    /// The lane height now, [`LANE_HEIGHT`]..106: fitted, or one of [`LANE_HEIGHTS`] chosen.
    /// The one height the drawing and every hit test read.
    pub lane: f32,
    /// The track area's height, which the rows shown fill whole ([`Grid::rows`],
    /// [`Grid::shows_add_track`]).
    pub area: f32,
}

impl Grid {
    pub fn px_per_bar(&self) -> f32 {
        ZOOM.kept(self.zoom)
    }

    /// Pixels `ticks` take on the timeline.
    pub fn px_of(&self, ticks: Ticks) -> f32 {
        (ticks.0 as f64 * f64::from(self.px_per_bar()) / self.ticks()) as f32
    }

    /// The cell a scroll comes to rest on at `tick`: a beat of the meter there while its beat
    /// lines are drawn (a beat wide enough to show, [`grid_rules::line_alpha`]), else a bar.
    pub fn cell(&self, meter: &MeterMap, tick: Ticks) -> MeterUnit {
        let beat = meter.signature_at(tick).beat_ticks(meter.ppq());
        if grid_rules::line_alpha(self.px_of(beat)) > 0.0 {
            MeterUnit::Beat
        } else {
            MeterUnit::Bar
        }
    }

    /// `tick` moved to the nearest cell's start in `meter`, halfway going on: where a scroll
    /// along time comes to rest, so the view starts on a whole beat or bar.
    pub fn landed(&self, meter: &MeterMap, tick: Ticks) -> Ticks {
        meter.snap_nearest(tick, self.cell(meter, tick))
    }

    /// Where a scroll from the view's start towards `to` comes to rest: [`Grid::landed`],
    /// unless that is no further along than where it started, then the next cell start that
    /// way. Why: a move shorter than half a cell would land back where it was, and the view
    /// could not be moved at all.
    pub fn landed_towards(&self, meter: &MeterMap, to: Ticks) -> Ticks {
        let from = self.scroll;
        let landed = self.landed(meter, to);
        let unit = self.cell(meter, from);
        if to > from && landed <= from {
            meter.snap_next(from, unit)
        } else if to < from && landed >= from {
            let down = meter.snap_down(from, unit);
            let before = if down < from {
                down
            } else {
                meter.snap_down(Ticks(from.0 - 1), self.cell(meter, Ticks(from.0 - 1)))
            };
            Ticks(before.0.max(0))
        } else {
            landed
        }
    }

    /// The start of the cell holding `tick`.
    pub fn cell_start(&self, meter: &MeterMap, tick: Ticks) -> Ticks {
        meter.snap_down(tick, self.cell(meter, tick))
    }

    /// This grid at `px` pixels per bar (kept), scrolled so that what shows at `x` now shows
    /// there still ([`grid_rules::anchored_scroll`]).
    pub fn zoomed_at(&self, px: f32, x: f32) -> Grid {
        let from_left = f64::from(x) - f64::from(TRACK_HEADER_WIDTH);
        let zoomed = Grid {
            zoom: ZOOM.kept(px),
            ..*self
        };
        Grid {
            scroll: grid_rules::anchored_scroll(
                self.scroll,
                from_left,
                self.ticks(),
                self.px_per_bar(),
                zoomed.px_per_bar(),
            ),
            ..zoomed
        }
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

    /// The track index of the row under y, while that row shows ([`Grid::shows`]); none above
    /// the first row or in the strip under the rows shown. A gap belongs to the row above it.
    pub fn shown_index_at(&self, y: f32) -> Option<usize> {
        let index = usize::try_from(self.track_index_at(y)).ok()?;
        self.row_top_of(index).is_some().then_some(index)
    }

    /// The top of the row showing track `index`; `None` for a track not shown: scrolled above
    /// the top row, or under the rows the track area holds whole.
    pub fn row_top_of(&self, index: usize) -> Option<f32> {
        let row = index.checked_sub(self.top)?;
        self.shows(row).then(|| self.row_top(row))
    }

    /// The rows the track area holds whole, counted from the top row ([`Grid::whole_rows`]):
    /// the ones drawn and hit. Why whole rows only: a divider dragged up (or a smaller window)
    /// cut the last track in half under the scroll bar, its header and its lane.
    pub fn rows(&self) -> usize {
        self.whole_rows(self.area)
    }

    /// Whether row `row`, counted from the top row, shows: the track area holds it whole.
    pub fn shows(&self, row: usize) -> bool {
        row < self.rows()
    }

    /// Whether the "+ Track" row under the last of `below` tracks (counted from the top row)
    /// shows: its 24 px end inside the track area, so the tracks above it show too.
    pub fn shows_add_track(&self, below: usize) -> bool {
        self.row_top(below) + ADD_TRACK_HEIGHT <= self.area
    }

    /// From one row's top to the next.
    pub fn row_pitch(&self) -> f32 {
        self.lane + LANE_GAP
    }

    /// The top of row `row`, counted from the top visible track.
    pub fn row_top(&self, row: usize) -> f32 {
        row as f32 * self.row_pitch()
    }

    /// The rows wholly inside a track area `height` tall: each row's lane ends inside it, the
    /// gap under the last need not, so n rows take n·(lane + 1) − 1 px.
    pub fn whole_rows(&self, height: f32) -> usize {
        ((height + LANE_GAP) / self.row_pitch()).floor().max(0.0) as usize
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

/// An audio clip's fade handle: a disc in the accent this wide, and this wide under the pointer.
pub const FADE_HANDLE: f32 = 8.0;
pub const FADE_HANDLE_HOT: f32 = 11.0;
/// The dark ring round a fade handle's disc and a warp marker's triangle. Why the theme's
/// ground: it is the one tone that stands apart from the title strip (2.65:1 or more), the
/// waveform (10:1) and the accent in all five themes; Gilt's lighter clip body reads 1.47:1 on
/// the title strip, and Slate's violet only 1.34:1 there without a ring.
pub const HANDLE_RING: f32 = 1.0;
/// A pointer this close to a fade handle's centre is on it: the hot disc and its ring, so the
/// handle drawn hot is the one a press takes.
pub const FADE_HANDLE_REACH: f32 = FADE_HANDLE_HOT / 2.0 + HANDLE_RING;
/// The narrowest audio clip that shows its fade handles: room for both rings and a gap.
pub const FADE_HANDLES_LEAST: f32 = 24.0;

/// One of an audio clip's handles: its fade in's, its fade out's, or (in warp mode) the warp
/// marker at that index's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipHandle {
    FadeIn,
    FadeOut,
    Warp(usize),
}

/// Whether an audio clip shows its fade handles: while the pointer is over it or it is
/// selected, never in warp mode, where its markers' handles take the top of its body. Why hidden
/// at rest: two light squares on every clip read as notes on the track.
pub fn shows_fade_handles(hovered: bool, selected: bool, warping: bool) -> bool {
    (hovered || selected) && !warping
}

/// How far `frames` of a fade reach into a clip `width` px wide that plays `clip_frames`.
pub fn fade_reach(frames: u64, clip_frames: u64, width: f32) -> f32 {
    if clip_frames == 0 {
        0.0
    } else {
        (width * (frames.min(clip_frames) as f32 / clip_frames as f32)).min(width)
    }
}

/// The centres of an audio clip's two fade handles (the fade in's, the fade out's), hanging
/// from the top edge of the body under `rect`'s title strip (`body_top`), a ringed disc's
/// radius below it: each at its fade's inner end, `reach_in` px from the clip's start and
/// `reach_out` px from its end, kept that radius inside the clip, so at a fade of 0 both sit
/// just inside the top corners; none on a clip narrower than [`FADE_HANDLES_LEAST`]. Why
/// hanging and not centred on the edge: the title's letters fill the strip's lower rows, and a
/// disc there covered the first letters of every name.
pub fn fade_handles(
    rect: Rectangle,
    body_top: f32,
    reach_in: f32,
    reach_out: f32,
) -> Option<(Point, Point)> {
    if rect.width < FADE_HANDLES_LEAST {
        return None;
    }
    let inset = FADE_HANDLE / 2.0 + HANDLE_RING;
    let (first, last) = (rect.x + inset, rect.x + rect.width - inset);
    let at = |x: f32| Point::new(x.clamp(first, last), body_top + inset);
    Some((at(rect.x + reach_in), at(rect.x + rect.width - reach_out)))
}

/// The fade handle of `handles` (centres, as [`fade_handles`] gives them for the fades' reaches
/// `reaches`) the pointer at `at` is on: within [`FADE_HANDLE_REACH`] of its centre, the nearer
/// when both are; when both are as near, the longer fade's, the fade out's when they are as
/// long. Why the longer: a fade that fills the clip puts its handle on the other's, which has
/// no room to move.
pub fn fade_handle_at(
    handles: (Point, Point),
    reaches: (f32, f32),
    at: Point,
) -> Option<ClipHandle> {
    let (head, tail) = handles;
    let mut handles = [(tail, ClipHandle::FadeOut), (head, ClipHandle::FadeIn)];
    if reaches.0 > reaches.1 {
        handles.reverse();
    }
    // `min_by` keeps the first of equals.
    handles
        .into_iter()
        .map(|(centre, handle)| (centre.distance(at), handle))
        .filter(|&(distance, _)| distance <= FADE_HANDLE_REACH)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, handle)| handle)
}

/// The shade over a fade: the part of the body above the fade's line from `from` to `to` (left
/// to right, from the body's foot to its `top` or back), between x `span.0` and `span.1`, as
/// the corners of a polygon: the top's two, then the line's end and start. `None` when none of
/// it is in the span. Why cut here: the GPU renderer drops what a clipped frame draws on the
/// tracks (see [`transient_ticks`]).
pub fn fade_wedge(from: Point, to: Point, top: f32, span: (f32, f32)) -> Option<[Point; 4]> {
    let (start, end) = line_between(from, to, span)?;
    (end.x > start.x).then(|| [Point::new(start.x, top), Point::new(end.x, top), end, start])
}

/// A warp marker's handle: a downward triangle this wide, [`WARP_HANDLE_HEIGHT`] tall, hanging
/// from the top of the body on the marker's line, and this wide under the pointer (its height in
/// proportion). Why a triangle: a fade handle is a disc, and the two must not be taken for each
/// other.
pub const WARP_HANDLE: f32 = 9.0;
pub const WARP_HANDLE_HEIGHT: f32 = 7.0;
pub const WARP_HANDLE_HOT: f32 = 12.0;

/// The corners of a warp marker's triangle `width` wide on x `x` hanging from `top`: the top's
/// left and right, then the point below.
pub fn warp_triangle(x: f32, top: f32, width: f32) -> [Point; 3] {
    let height = width * WARP_HANDLE_HEIGHT / WARP_HANDLE;
    [
        Point::new(x - width / 2.0, top),
        Point::new(x + width / 2.0, top),
        Point::new(x, top + height),
    ]
}
/// A transient's mark: a tick this tall from the top of a selected audio clip's body.
pub const TRANSIENT_TICK: f32 = 4.0;
/// A transient's mark is this many device pixels wide: a hit's peak, in the waveform's colour,
/// reaches the body's top at the same place, and one pixel of accent beside it reads poorly.
pub const TRANSIENT_TICK_DEVICE_PX: f32 = 2.0;

/// The part of the line from `from` to `to` (left to right) between x `span.0` and `span.1`;
/// `None` when none of it is.
pub fn line_between(from: Point, to: Point, span: (f32, f32)) -> Option<(Point, Point)> {
    let (left, right) = span;
    if to.x < left || from.x > right {
        return None;
    }
    let at = |x: f32| {
        if to.x == from.x {
            return Point::new(x, from.y);
        }
        let share = (x - from.x) / (to.x - from.x);
        Point::new(x, from.y + (to.y - from.y) * share)
    };
    let start = if from.x < left { at(left) } else { from };
    let end = if to.x > right { at(right) } else { to };
    Some((start, end))
}

/// The marks of `onsets` (frames of the sound) on a clip drawn as `rect` with its body `body`,
/// playing them through `map`: a [`TRANSIENT_TICK`] tall tick, [`TRANSIENT_TICK_DEVICE_PX`]
/// device pixels wide on whole device pixels, at the top of the body where each plays; only the
/// onsets the clip plays, and only ticks wholly inside `body`. Why the cut here and not by the
/// frame's clip: the GPU renderer reads a clipped frame's bounds against the window rather than
/// the canvas, and drew none of the ticks clipped that way.
pub fn transient_ticks(
    map: &crate::ui::warp_gesture::ClipMap,
    onsets: &[u64],
    rect: Rectangle,
    body: Rectangle,
    scale_factor: f32,
) -> Vec<Rectangle> {
    let scale = if scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let width = TRANSIENT_TICK_DEVICE_PX / scale;
    let size = Size::new(width, TRANSIENT_TICK.min(body.height));
    onsets
        .iter()
        .filter_map(|&onset| {
            let song = map.song_of(map.out_of(onset))?;
            let x = crate::ui::warp_gesture::x_of_frame(
                rect.x, rect.width, map.start, map.played, song,
            );
            let (left, _) = grid_rules::device_line(x, scale);
            (left >= body.x && left + width <= body.x + body.width)
                .then(|| Rectangle::new(Point::new(left, body.y), size))
        })
        .collect()
}

/// The warp markers of a clip drawn as `rect` on `map` that it plays: each marker's x (where its
/// moment plays) and the box its hot triangle ([`WARP_HANDLE_HOT`] wide, hanging from the
/// body's top, `body_top`) fills, which the pointer is on it in; in the markers' order, `None`
/// for a marker outside the part the clip plays.
pub fn warp_handles(
    rect: Rectangle,
    body_top: f32,
    map: &crate::ui::warp_gesture::ClipMap,
) -> Vec<Option<(f32, Rectangle)>> {
    let size = Size::new(
        WARP_HANDLE_HOT,
        WARP_HANDLE_HOT * WARP_HANDLE_HEIGHT / WARP_HANDLE,
    );
    map.markers
        .iter()
        .map(|marker| {
            let song = map.song_of(marker.out)?;
            let x = crate::ui::warp_gesture::x_of_frame(
                rect.x, rect.width, map.start, map.played, song,
            );
            let reach = Rectangle::new(Point::new(x - WARP_HANDLE_HOT / 2.0, body_top), size);
            Some((x, reach))
        })
        .collect()
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

/// The header control under `point` with `rows` tracks from the top row on (the "+ Track"
/// button sits under the last of them), of the rows that show ([`Grid::shows`],
/// [`Grid::shows_add_track`]).
pub fn header_hit(grid: &Grid, point: Point, rows: usize) -> Option<HeaderHit> {
    if !(0.0..TRACK_HEADER_WIDTH).contains(&point.x) || point.y < 0.0 {
        return None;
    }
    if grid.shows_add_track(rows) && add_track_rect(grid, rows).contains(point) {
        return Some(HeaderHit::AddTrack);
    }
    let row = (point.y / grid.row_pitch()).floor() as usize;
    if row >= rows || !grid.shows(row) || point.y - grid.row_top(row) >= grid.lane {
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

/// The "+ Track" control under the last of `rows` rows, where a press takes it: right under its
/// lane and the gap under that, its row's height, as wide as [`add_words::width`] gives,
/// centred in the header column.
pub fn add_track_rect(grid: &Grid, rows: usize) -> Rectangle {
    static WIDTH: OnceLock<f32> = OnceLock::new();
    let width = *WIDTH.get_or_init(|| add_words::width(ADD_TRACK_LABEL));
    Rectangle::new(
        Point::new((TRACK_HEADER_WIDTH - width) / 2.0, grid.row_top(rows)),
        Size::new(width, ADD_TRACK_HEIGHT),
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

/// The bars of `meter` whose line is inside the timeline of a canvas `width` wide, as (bar
/// number counted from 1 across the changes, line x).
pub fn visible_bars(grid: &Grid, meter: &MeterMap, width: f32) -> impl Iterator<Item = (i64, f32)> {
    let (from, to) = (grid.scroll, grid.tick_at(width));
    let first = meter.bar_at(from);
    let first = if first.start < from {
        first.bar + 1
    } else {
        first.bar
    };
    let mut bars = Vec::new();
    for bar in first.. {
        let start = meter.bar_start(bar);
        if start > to {
            break;
        }
        bars.push((bar + 1, grid.x_of(start)));
    }
    bars.into_iter()
}

/// The x spans the shaded bars ([`grid_rules::shaded`], by bar number) of `meter` take on the
/// timeline of a canvas `width` wide, left to right: one a shaded bar, or a shaded four bars far
/// out, cut to the timeline.
pub fn shaded_spans<'a>(
    grid: &'a Grid,
    meter: &'a MeterMap,
    width: f32,
) -> impl Iterator<Item = (f32, f32)> + 'a {
    let px = grid.px_per_bar();
    let group = grid_rules::shade_group(px);
    let first = meter.bar_at(grid.scroll).bar.div_euclid(group);
    let last = meter.bar_at(grid.tick_at(width)).bar.div_euclid(group);
    (first..=last)
        .filter(move |&index| grid_rules::shaded(index * group, px))
        .map(move |index| {
            (
                meter.bar_start(index * group),
                meter.bar_start((index + 1) * group),
            )
        })
        .filter_map(move |(start, end)| {
            let left = grid.x_of(start).max(TRACK_HEADER_WIDTH);
            let right = grid.x_of(end).min(width);
            (right > left).then_some((left, right))
        })
}

/// Room either side of a flag's signature ([`flag_rect`]), and its pole down the bar line.
pub const FLAG_PAD: f32 = 3.0;
pub const FLAG_POLE: f32 = 2.0;

/// The flag of a change of signature at tick `at`, in the ruler's number band: from its bar
/// line, its pole ([`FLAG_POLE`]) then its signature at the numbers' size with [`FLAG_PAD`]
/// either side, the band's height; `None` when its line is off the timeline of a canvas
/// `width` wide.
pub fn flag_rect(
    grid: &Grid,
    at: Ticks,
    signature: TimeSignature,
    width: f32,
) -> Option<Rectangle> {
    let x = grid.x_of(at);
    if !(TRACK_HEADER_WIDTH..width).contains(&x) {
        return None;
    }
    let wide = FLAG_POLE + text_width(&signature.to_string(), size::DENSE) + 2.0 * FLAG_PAD;
    Some(Rectangle::new(
        Point::new(x, 0.0),
        Size::new(wide.ceil(), NUMBER_BAND_HEIGHT),
    ))
}

/// The change whose flag is under ruler point `at` (the latest drawn on top when two meet).
pub fn flag_under(grid: &Grid, changes: &[SignatureChange], at: Point) -> Option<Ticks> {
    changes
        .iter()
        .rev()
        .find(|change| {
            flag_rect(grid, change.at, change.signature, f32::INFINITY)
                .is_some_and(|rect| rect.contains(at))
        })
        .map(|change| change.at)
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
    use crate::ui::grid_rules::{beat_alpha, shaded, strong};

    // p39_plain's clip (song frames 60000..96000, its 24000 frames stretched to 36000, no
    // markers) drawn 360 px wide from x 100, its body from y 30, 26 px tall, at a scale of 1:
    // onsets 1000 and 9000 play at out 1500 and 13500, song frames 61500 and 73500, so x = 100 +
    // 360 · 1500 / 36000 = 115 and 100 + 360 · 13500 / 36000 = 235; each tick 2 px wide and 4
    // tall at y 30. An onset past the sound plays nowhere; at a scale of 2 a tick is 1 px (2
    // device px). With the body starting at x 236, both lie outside it and none is drawn.
    #[test]
    fn a_clips_transients_are_ticks_inside_its_body() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/p39_plain.json"
        ))
        .expect("p39_plain.json");
        let project = dawsome_engine::project::Project::load_json(&text).expect("loads");
        let map = crate::ui::warp_gesture::ClipMap::of(
            &project,
            dawsome_engine::project::ClipId(1),
            Some(24_000),
        )
        .expect("an audio clip");
        let rect = Rectangle::new(Point::new(100.0, 20.0), Size::new(360.0, 36.0));
        let body = Rectangle::new(Point::new(100.0, 30.0), Size::new(360.0, 26.0));
        let ticks = transient_ticks(&map, &[1_000, 9_000, 30_000], rect, body, 1.0);
        let at = |x: f32, width: f32| Rectangle::new(Point::new(x, 30.0), Size::new(width, 4.0));
        assert_eq!(ticks, [at(115.0, 2.0), at(235.0, 2.0)]);
        let sharp = transient_ticks(&map, &[1_000, 9_000], rect, body, 2.0);
        assert_eq!(sharp, [at(115.0, 1.0), at(235.0, 1.0)]);
        let narrow = Rectangle::new(Point::new(236.0, 30.0), Size::new(224.0, 26.0));
        assert!(transient_ticks(&map, &[1_000, 9_000], rect, narrow, 1.0).is_empty());
    }

    // A fade line from (100, 50) to (140, 30) cut to x 110..200 starts a quarter along, at
    // (110, 45); one wholly left of the span is none.
    #[test]
    fn a_line_is_cut_to_a_span_of_columns() {
        let (from, to) = (Point::new(100.0, 50.0), Point::new(140.0, 30.0));
        assert_eq!(
            line_between(from, to, (110.0, 200.0)),
            Some((Point::new(110.0, 45.0), to))
        );
        assert_eq!(line_between(from, to, (150.0, 200.0)), None);
    }

    // A clip drawn from x 100, 200 px wide, its body from y 30: a ringed disc is 8 + 2 · 1 = 10
    // px across, so the handles hang 5 px under the body's top, y 35, and at fades of 0 sit 5 px
    // inside the corners, at (105, 35) and (295, 35); fades reaching 40 and 60 px put them at
    // their inner ends, (140, 35) and (240, 35); a fade reaching 2 px keeps its handle at 105,
    // one filling the clip puts it at 295 (and 105 for a fade out). At the least width, 24 px,
    // the centres are 14 px apart, a 4 px gap between the rings; 23 px wide shows none.
    #[test]
    fn fade_handles_sit_at_the_fades_inner_ends_inside_the_corners() {
        let rect = Rectangle::new(Point::new(100.0, 20.0), Size::new(200.0, 36.0));
        let at = |x: f32| Point::new(x, 35.0);
        let handles = |reach_in, reach_out| fade_handles(rect, 30.0, reach_in, reach_out);
        assert_eq!(handles(0.0, 0.0), Some((at(105.0), at(295.0))));
        assert_eq!(handles(40.0, 60.0), Some((at(140.0), at(240.0))));
        assert_eq!(handles(2.0, 200.0), Some((at(105.0), at(105.0))));
        assert_eq!(handles(200.0, 0.0), Some((at(295.0), at(295.0))));
        let wide = |width: f32| Rectangle::new(rect.position(), Size::new(width, 36.0));
        assert_eq!(
            fade_handles(wide(24.0), 30.0, 0.0, 0.0),
            Some((at(105.0), at(119.0)))
        );
        assert_eq!(fade_handles(wide(23.0), 30.0, 0.0, 0.0), None);
    }

    // The pointer is on a handle within 11 / 2 + 1 = 6.5 px of its centre: on (140, 30)'s at
    // (146.5, 30) and (140, 36.5), off it at (146.6, 30). Centres 8 px apart (200 and 208): the
    // nearer takes the pointer, 203 the fade in's (3 px against 5), 205 the fade out's. Both on
    // 295 (a fade in filling the clip): the fade in's, the longer, which can move; both on 105
    // (a fade out filling it): the fade out's.
    #[test]
    fn the_pointer_is_on_the_nearer_fade_handle_within_its_hot_disc() {
        let at = |x: f32, y: f32| Point::new(x, y);
        let apart = (at(140.0, 30.0), at(240.0, 30.0));
        let on = |handles, reaches, x, y| fade_handle_at(handles, reaches, at(x, y));
        assert_eq!(
            on(apart, (40.0, 60.0), 146.5, 30.0),
            Some(ClipHandle::FadeIn)
        );
        assert_eq!(
            on(apart, (40.0, 60.0), 140.0, 36.5),
            Some(ClipHandle::FadeIn)
        );
        assert_eq!(on(apart, (40.0, 60.0), 146.6, 30.0), None);
        assert_eq!(
            on(apart, (40.0, 60.0), 240.0, 25.0),
            Some(ClipHandle::FadeOut)
        );
        let close = (at(200.0, 30.0), at(208.0, 30.0));
        assert_eq!(
            on(close, (100.0, 92.0), 203.0, 30.0),
            Some(ClipHandle::FadeIn)
        );
        assert_eq!(
            on(close, (100.0, 92.0), 205.0, 30.0),
            Some(ClipHandle::FadeOut)
        );
        let corner = |x| (at(x, 30.0), at(x, 30.0));
        assert_eq!(
            on(corner(295.0), (200.0, 0.0), 295.0, 30.0),
            Some(ClipHandle::FadeIn)
        );
        assert_eq!(
            on(corner(105.0), (0.0, 200.0), 105.0, 30.0),
            Some(ClipHandle::FadeOut)
        );
    }

    #[test]
    fn fade_handles_show_only_on_a_clip_pointed_at_or_selected_out_of_warp_mode() {
        assert!(!shows_fade_handles(false, false, false), "at rest");
        assert!(shows_fade_handles(true, false, false), "pointed at");
        assert!(shows_fade_handles(false, true, false), "selected");
        assert!(!shows_fade_handles(true, true, true), "warp mode");
    }

    // A fade in reaching 40 px on a clip from x 100, its body y 30..56: the shade above its line
    // from (100, 56) to (140, 30) is the triangle (100, 30), (140, 30), (100, 56), the line's end
    // repeating the top's right corner. Cut to the body's columns from x 110 (the clip scrolled
    // under the headers), the line starts a quarter along, at (110, 49.5). A fade out reaching
    // 20 px on a clip ending at x 300: (280, 30), (300, 30), (300, 56), the line's start
    // repeating the top's left corner. Every corner lies inside the body; a span left of the
    // fade has none of it.
    #[test]
    fn a_fades_shade_is_the_body_above_its_line_cut_to_the_body() {
        let at = |x: f32, y: f32| Point::new(x, y);
        let fade_in = (at(100.0, 56.0), at(140.0, 30.0));
        let whole = fade_wedge(fade_in.0, fade_in.1, 30.0, (100.0, 300.0));
        assert_eq!(
            whole,
            Some([
                at(100.0, 30.0),
                at(140.0, 30.0),
                at(140.0, 30.0),
                at(100.0, 56.0)
            ])
        );
        let cut = fade_wedge(fade_in.0, fade_in.1, 30.0, (110.0, 300.0));
        assert_eq!(
            cut,
            Some([
                at(110.0, 30.0),
                at(140.0, 30.0),
                at(140.0, 30.0),
                at(110.0, 49.5)
            ])
        );
        let fade_out = fade_wedge(at(280.0, 30.0), at(300.0, 56.0), 30.0, (100.0, 300.0));
        assert_eq!(
            fade_out,
            Some([
                at(280.0, 30.0),
                at(300.0, 30.0),
                at(300.0, 56.0),
                at(280.0, 30.0)
            ])
        );
        let body = Rectangle::new(at(100.0, 30.0), Size::new(200.0, 26.0));
        let inside = |corner: &Point| {
            (body.x..=body.x + body.width).contains(&corner.x)
                && (body.y..=body.y + body.height).contains(&corner.y)
        };
        for wedge in [whole, cut, fade_out].into_iter().flatten() {
            assert!(wedge.iter().all(inside), "{wedge:?} inside {body:?}");
        }
        assert_eq!(fade_wedge(fade_in.0, fade_in.1, 30.0, (150.0, 300.0)), None);
    }

    // A warp marker's triangle on x 200 under a body top of 30: 9 px wide, 7 tall, (195.5, 30),
    // (204.5, 30), (200, 37); hot, 12 wide and 12 · 7 / 9 = 9.33 tall.
    #[test]
    fn a_warp_markers_handle_is_a_downward_triangle() {
        let at = |x: f32, y: f32| Point::new(x, y);
        assert_eq!(
            warp_triangle(200.0, 30.0, WARP_HANDLE),
            [at(195.5, 30.0), at(204.5, 30.0), at(200.0, 37.0)]
        );
        let hot = warp_triangle(200.0, 30.0, WARP_HANDLE_HOT);
        assert_eq!(hot[..2], [at(194.0, 30.0), at(206.0, 30.0)]);
        assert!((hot[2].y - (30.0 + 28.0 / 3.0)).abs() < 1e-4, "{hot:?}");
    }

    /// A song of one 4/4 at 96 ppq.
    fn common() -> MeterMap {
        MeterMap::single(dawsome_engine::time::TimeSignature::COMMON, 96)
    }

    // Known answers C 6 of gen_p35_expected.py: p35_meter's bars (4/4, 3/4 from 768, 6/8 from
    // 1344, 7/8 from 1920) start at 0, 384, 768, 1056, 1344, 1632, 1920, 2256, 2592; at 64 px a
    // 4/4 bar a tick is 1/6 px, so their lines are at x 120 + t / 6: 120, 184, 248, 296, 344,
    // 392, 440, 496, 552, numbered 1 to 9. Shaded bars 1, 3, 5, 7, 9 (every other by number):
    // 120..184, 248..296, 344..392, 440..496, 552..608 (cut at 610). A scroll near 1100 (in
    // 3/4 from 768, beats of 96 ticks, 16 px, drawn) lands on the nearest beat of that meter:
    // 1100 and 1103 on 1056 (under half of 96 past it), 1104 (half way) on 1152.
    #[test]
    fn the_bars_shading_and_landing_follow_the_meter_map() {
        let p35: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/p35_expected.json"
        )))
        .expect("p35_expected.json");
        let changes: Vec<dawsome_engine::time::SignatureChange> = p35["c"]["changes"]
            .as_array()
            .expect("changes")
            .iter()
            .map(|change| dawsome_engine::time::SignatureChange {
                at: Ticks(change[0].as_i64().expect("a tick")),
                signature: dawsome_engine::time::TimeSignature::new(
                    change[1][0].as_u64().expect("beats") as u8,
                    change[1][1].as_u64().expect("unit") as u8,
                )
                .expect("a signature"),
            })
            .collect();
        let meter = MeterMap::new(dawsome_engine::time::TimeSignature::COMMON, &changes, 96);
        let bars: Vec<(i64, f32)> = visible_bars(&grid(), &meter, 560.0).collect();
        let starts: Vec<i64> = p35["c"]["bars"]
            .as_array()
            .expect("bars")
            .iter()
            .map(|start| start.as_i64().expect("a tick"))
            .collect();
        let want: Vec<(i64, f32)> = (1..)
            .zip(&starts)
            .map(|(number, &start)| (number, 120.0 + start as f32 / 6.0))
            .collect();
        assert_eq!(bars, want);
        let spans: Vec<(f32, f32)> = shaded_spans(&grid(), &meter, 610.0).collect();
        assert_eq!(
            spans,
            [
                (120.0, 184.0),
                (248.0, 296.0),
                (344.0, 392.0),
                (440.0, 496.0),
                (552.0, 608.0)
            ]
        );
        assert_eq!(grid().landed(&meter, Ticks(1100)), Ticks(1056));
        assert_eq!(grid().landed(&meter, Ticks(1103)), Ticks(1056));
        assert_eq!(grid().landed(&meter, Ticks(1104)), Ticks(1152));
    }

    /// The grid as the playlist opens: no scroll, 64 px per bar, a bar of 384 ticks, the 12
    /// rows a 426 px track area holds whole (the playlist alone in a 1000 x 560 window).
    fn grid() -> Grid {
        Grid {
            scroll: Ticks(0),
            zoom: ZOOM.default,
            bar: Ticks(384),
            top: 0,
            lane: LANE_HEIGHT,
            area: 426.0,
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
        let bars: Vec<(i64, f32)> = visible_bars(&grid, &common(), 1000.0).collect();
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
        let zoomed = Grid {
            zoom: 128.0,
            ..then
        };
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
            zoom: 16.0,
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
            &common(),
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
        // The 12 rows shown are tracks 3..=14; track 15 is under them.
        assert_eq!(grid.row_top_of(14), Some(363.0));
        assert_eq!(grid.row_top_of(15), None);
        assert_eq!(grid.shown_index_at(32.5), Some(3));
        assert_eq!(
            grid.shown_index_at(395.0),
            Some(14),
            "row 11's lane: y 363..395"
        );
        assert_eq!(grid.shown_index_at(396.0), None, "row 12 is not shown");
        assert_eq!(grid.shown_index_at(-1.0), None, "above the first row");
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
        // The button sits right under row 7's lane and its gap: y 8 · 33 = 264 .. 288 (it stood
        // 6 px down a row a lane tall, 270..294, before the "+ Track" row took its own height).
        assert_eq!(at(60.0, 263.5), None, "row 7's gap");
        assert_eq!(at(60.0, 264.0), Some(HeaderHit::AddTrack));
        assert_eq!(at(60.0, 275.0), Some(HeaderHit::AddTrack));
        assert_eq!(at(60.0, 288.0), None, "under the button");
        assert_eq!(at(60.0, 300.0), None);
        assert_eq!(at(130.0, 16.0), None, "the timeline");
        assert_eq!(led_center(&grid(), 1), Point::new(10.0, 49.0));
        // Three rows shown (a 118 px track area): row 3's light, its name and the "+ Track"
        // button under it (y 99..123 with three tracks, past the area's foot at 118) are none,
        // as they are not drawn; with two tracks the button is y 66..90, inside it.
        let three = Grid {
            area: 118.0,
            ..grid()
        };
        let at = |x, y, rows| header_hit(&three, Point::new(x, y), rows);
        assert_eq!(at(10.0, 82.0, 8), Some(HeaderHit::Led { row: 2 }));
        assert_eq!(at(10.0, 115.0, 8), None);
        assert_eq!(at(60.0, 115.0, 8), None);
        assert_eq!(at(60.0, 110.0, 3), None);
        assert_eq!(at(60.0, 77.0, 2), Some(HeaderHit::AddTrack), "row 2 shows");
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
    // scroll bar's 10 px under it): 12 rows of 32 px show whole (12 · 33 − 1 = 395 px; 13
    // would take 428). 8 tracks share 426 − 24 − 1 = 401 px in lanes of floor(401 / 8) − 1 =
    // 49 px: 8 rows whole (8 · 50 − 1 = 399; 9 take 449) and the "+ Track" row right under
    // them, y 400..424, inside the 426. (Pins moved: 452 under the 56 px toolbar, then 436 and
    // 14 rows with the bar in the header; then 13 rows, the 13th cut to 30 px, until rows
    // showed whole only; 8 tracks in 46 px lanes, 426 / 9 = 47.3 less the gap, while the
    // "+ Track" row took a lane's share; 64 px lanes, 6 rows, while 64 was the fitted lanes'
    // floor.)
    #[test]
    fn the_track_area_fills_the_panel_under_the_ruler() {
        let size = tracks_size_alone(Size::new(1000.0, 560.0), 72.0);
        assert_eq!(size, Size::new(1000.0, 426.0));
        assert_eq!(grid().whole_rows(426.0), 12);
        assert_eq!(grid().whole_rows(33.0), 1);
        assert_eq!(fitted_lane(8, 426.0), 49.0);
        let fitted = Grid {
            lane: fitted_lane(8, 426.0),
            ..grid()
        };
        assert_eq!(fitted.rows(), 8);
        assert!(fitted.shows_add_track(8), "400 + 24 = 424, inside the 426");
    }

    // The whole-row law by hand, n rows taking n · (lane + 1) − 1 px. A playlist panel h tall
    // has h − 24 (header) − 28 (ruler) − 10 (scroll bar) px of track area. At its least, 180
    // (the roll's divider dragged as far up as it goes): 118 px, 3 rows of 32 (98 px; 4 take
    // 131), where 4 were drawn before, the 4th cut to 118 − 99 = 19 px under the bar. At the
    // divider test's heights (panels.rs), 355, 405 and 481: 293, 343 and 419 px, 8, 10 and 12
    // rows of 32 (263, 329, 395 px; one more takes 296, 362, 428). At 293 px rows of 48 fit 6
    // exactly (6 · 49 − 1 = 293: the gap under the last falls outside), rows of 64 four (259;
    // 5 take 324) and rows of 96 three (290). One pixel short of n rows shows n − 1; less than
    // a lane shows none.
    #[test]
    fn only_rows_the_track_area_holds_whole_show() {
        let lane = |lane| Grid { lane, ..grid() };
        let area = |panel: f32| panel - HEADER_HEIGHT - RULER_HEIGHT - TIME_BAR_HEIGHT;
        let rows_32 = |panel| lane(32.0).whole_rows(area(panel));
        assert_eq!(area(180.0), 118.0);
        assert_eq!(rows_32(180.0), 3);
        assert_eq!(
            [rows_32(355.0), rows_32(405.0), rows_32(481.0)],
            [8, 10, 12]
        );
        assert_eq!(lane(48.0).whole_rows(293.0), 6);
        assert_eq!(lane(64.0).whole_rows(293.0), 4);
        assert_eq!(lane(96.0).whole_rows(293.0), 3);
        assert_eq!(lane(32.0).whole_rows(197.0), 6);
        assert_eq!(lane(32.0).whole_rows(196.0), 5);
        assert_eq!(lane(32.0).whole_rows(31.0), 0);
        assert_eq!(lane(32.0).whole_rows(0.0), 0);
    }

    // By hand, fitted lanes: the "+ Track" row takes its own 24 px and a gap, the tracks share
    // what is left, each floor(share) less the 1 px gap under it, kept to 32..106. In a 567 px
    // area 6 tracks share 567 − 24 − 1 = 542 px: floor(542 / 6 = 90.33) − 1 = 89 px lanes,
    // 11 % taller than the 80 they were when the "+ Track" row took a lane's share,
    // floor(567 / 7 = 81) − 1 (90 px lanes would not fit: 6 · 91 + 24 = 570). Their rows take
    // 6 · 90 = 540 px, the "+ Track" row 540..564, 3 px of the well under it (an 81 px row
    // under the button before, 51 px of it empty). In a 436 px area (411 to share): 5 tracks
    // floor(82.2) − 1 = 81, the "+ Track" row 410..434; 6 tracks floor(68.5) − 1 = 67; 7 tracks
    // floor(58.7) − 1 = 57; 8 tracks floor(51.4) − 1 = 50, as in the 434 px area of the
    // playlist alone in a 1000 × 560 window, floor(409 / 8 = 51.1) − 1 (pin moved: 47 = floor(436 / 9) − 1
    // before, its 9 rows and the "+ Track" row whole); 13 tracks floor(31.6) − 1 = 30 and 30
    // tracks 12, kept to the floor, 32, and they scroll. (7, 8 and 30 tracks were 64 while 64
    // was the floor.) Two tracks, floor(205.5) − 1 = 204, and one, 410, are kept to the 106 cap
    // (96 before; two were 144 when the cap was 160). With no room yet (0 px) the share is
    // negative: 32 (64 under that floor).
    // Alone at 1920 × 1080 (954 px of tracks, the hint bar off) 6 tracks would be
    // floor(929 / 6) − 1 = 153: 106 (96 before: floor(954 / 7) − 1 = 135 kept to the old cap).
    // A height out of range reads back kept to the heights Alt+wheel has: 300 as 106 (was 96),
    // 4 as 32.
    #[test]
    fn fitted_lanes_share_the_track_area_among_the_tracks() {
        let table: [(usize, f32, f32); 12] = [
            (6, 567.0, 89.0),
            (5, 436.0, 81.0),
            (6, 436.0, 67.0),
            (7, 436.0, 57.0),
            (8, 436.0, 50.0),
            (13, 436.0, 32.0),
            (30, 436.0, 32.0),
            (2, 436.0, 106.0),
            (1, 436.0, 106.0),
            (0, 0.0, 32.0),
            (6, 954.0, 106.0),
            (8, 434.0, 50.0),
        ];
        for (tracks, area, lane) in table {
            assert_eq!(
                fitted_lane(tracks, area),
                lane,
                "{tracks} tracks in {area} px"
            );
        }
        let fitted = Grid {
            lane: fitted_lane(6, 567.0),
            area: 567.0,
            ..grid()
        };
        assert_eq!(fitted.rows(), 6, "6 · 90 − 1 = 539 px; 7 take 629");
        assert_eq!(fitted.row_top(6), 540.0);
        assert!(fitted.shows_add_track(6), "540 + 24 = 564, inside the 567");
        assert_eq!(add_track_rect(&fitted, 6).y, 540.0);
        assert_eq!(fitted.track_index_at(89.5), 0, "the gap under row 0");
        // The "+ Track" words alone, a press taking them and 8 px either side: "+ Track" at the
        // labels' 12 px is 43.758 px (Inter's advances, gen_u1_expected.py's rule: 12 × 7468 /
        // 2048), so 59.758 px wide, centred in the 120 px column: x 30.121..89.879 (was the
        // button's 6..114). Its row's band stays: y 540..564.
        let add = add_track_rect(&fitted, 6);
        assert!((add.x - 30.121).abs() < 1e-3, "{add:?}");
        assert!((add.width - 59.758).abs() < 1e-3, "{add:?}");
        assert_eq!((add.y, add.height), (540.0, 24.0));
        assert_eq!(add.center().x, TRACK_HEADER_WIDTH / 2.0);
        for (x, hit) in [(29.0, false), (31.0, true), (89.0, true), (91.0, false)] {
            assert_eq!(
                header_hit(&fitted, Point::new(x, 552.0), 6) == Some(HeaderHit::AddTrack),
                hit,
                "x {x}"
            );
        }
        assert_eq!(fitted.track_index_at(90.0), 1);
        let five = Grid {
            lane: fitted_lane(5, 436.0),
            area: 436.0,
            ..grid()
        };
        assert!(
            five.rows() == 5 && five.shows_add_track(5),
            "410 + 24 = 434"
        );
        assert_eq!((kept_lane(300.0), kept_lane(4.0)), (106.0, 32.0));
        assert_eq!((kept_lane(106.0), kept_lane(107.0)), (106.0, 106.0));
    }

    // By hand: at 64 px a bar a 1000 px tracks canvas shows 880 px of timeline, 13.75 bars
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

    // The scroll bar's rail spans the panel's whole width, under the track headers too, so the
    // thumb's ends are the panel's at the scroll's ends. By hand, a 1000 px panel at 64 px a
    // bar: the 880 px timeline shows 880 · 384 / 64 = 5280 ticks; a song ending with bar 40
    // (tick 15360) reaches 4 bars further, 16896. The thumb is the view's share of the rail,
    // 5280 / 16896 = 0.3125 of 1000 = 312.5 px, with 687.5 px of travel for the 16896 − 5280 =
    // 11616 ticks the view moves: at scroll 0 it starts at the panel's left edge, x 0; at the
    // middle, 5808, at 343.75; at the end, 11616, at 687.5, ending at the panel's right edge,
    // 1000. (Before, the rail started at the timeline's x 120: 880 px, a 275 px thumb from
    // x 120 at scroll 0.)
    #[test]
    fn the_scroll_bars_thumb_runs_the_panels_whole_width() {
        let span_at = |scroll| {
            let grid = Grid {
                scroll: Ticks(scroll),
                ..grid()
            };
            time_span(&grid, Ticks(15360), 1000.0)
        };
        let start = span_at(0);
        assert_eq!((start.content, start.view), (16896.0, 5280.0));
        assert_eq!(start.last_offset(), 11616.0);
        assert_eq!(start.thumb(1000.0), (0.0, 312.5));
        assert_eq!(span_at(5808).thumb(1000.0), (343.75, 312.5));
        let (left, size) = span_at(11616).thumb(1000.0);
        assert_eq!((left, left + size), (687.5, 1000.0));
    }

    // Alt+wheel steps the lanes through 32, 48, 64, 85 and 106 px, rows following: up from 32,
    // four steps reach 106 and a fifth holds there; down from 106, 85, then 64 (pins moved: 96
    // was the highest, and the step under it, while 96 was the cap). From a fitted height
    // between two steps, the next above or below: from 42, 48 or 32; from 89 (six tracks
    // fitted in 567 px), 106 or 85.
    #[test]
    fn lane_heights_step_and_rows_follow_them() {
        let mut grid = grid();
        assert_eq!(grid.stepped_lane(false), 32.0, "already the lowest");
        grid.lane = grid.stepped_lane(true);
        assert_eq!(grid.lane, 48.0);
        assert_eq!((grid.row_top(1), grid.track_index_at(49.0)), (49.0, 1));
        let mut up = vec![grid.lane];
        for _ in 0..4 {
            grid.lane = grid.stepped_lane(true);
            up.push(grid.lane);
        }
        assert_eq!(up, [48.0, 64.0, 85.0, 106.0, 106.0]);
        assert_eq!(grid.stepped_lane(true), 106.0, "already the highest");
        assert_eq!(grid.stepped_lane(false), 85.0);
        assert_eq!((grid.row_top(1), grid.track_index_at(106.5)), (107.0, 0));
        grid.lane = 85.0;
        assert_eq!(grid.stepped_lane(false), 64.0);
        // From a fitted height between two steps, the step above or below.
        grid.lane = 42.0;
        assert_eq!(
            (grid.stepped_lane(true), grid.stepped_lane(false)),
            (48.0, 32.0)
        );
        grid.lane = 89.0;
        assert_eq!(
            (grid.stepped_lane(true), grid.stepped_lane(false)),
            (106.0, 85.0)
        );
    }

    // The zoom's steps by hand: − and + halve and double, held to 16..256, so from 64 + gives
    // 128, then 256, then 256 again, and − gives 32, 16, 16; from a zoom between stops, 100, +
    // doubles to 200 and then holds at 256. A kept value out of range reads back held, one that
    // is no number the default.
    #[test]
    fn the_zoom_buttons_double_and_halve_within_the_ends() {
        let mut zoom = ZOOM.default;
        let mut closer = Vec::new();
        for _ in 0..3 {
            zoom = ZOOM.stepped(zoom, true);
            closer.push(zoom);
        }
        assert_eq!(closer, [128.0, 256.0, 256.0]);
        let mut zoom = ZOOM.default;
        let mut farther = Vec::new();
        for _ in 0..3 {
            zoom = ZOOM.stepped(zoom, false);
            farther.push(zoom);
        }
        assert_eq!(farther, [32.0, 16.0, 16.0]);
        assert_eq!(ZOOM.stepped(100.0, true), 200.0);
        assert_eq!(ZOOM.stepped(200.0, true), 256.0);
        assert_eq!(
            (ZOOM.kept(900.0), ZOOM.kept(2.0), ZOOM.kept(f32::NAN)),
            (256.0, 16.0, 64.0)
        );
    }

    // A Ctrl+wheel notch is a quarter of a doubling, 2^(1/4) = 1.189207: one notch in from 64 is
    // 76.10925, four are 128, four out 32. At either end a notch further holds. A zoom between
    // the steps (100: 4·log2(100/16) = 10.57 quarter-doublings over 16, the nearest 11) moves
    // from the nearest step: one in is 16·2^3 = 128.
    #[test]
    fn a_wheel_notch_zooms_a_quarter_of_a_doubling() {
        assert!((ZOOM.wheeled(64.0, true) - 76.109_25).abs() < 1e-4);
        let notches =
            |from: f32, closer: bool| (0..4).fold(from, |zoom, _| ZOOM.wheeled(zoom, closer));
        assert!((notches(64.0, true) - 128.0).abs() < 1e-5);
        assert!((notches(64.0, false) - 32.0).abs() < 1e-5);
        assert_eq!(ZOOM.wheeled(256.0, true), 256.0);
        assert_eq!(ZOOM.wheeled(16.0, false), 16.0);
        assert!((ZOOM.wheeled(100.0, true) - 128.0).abs() < 1e-5);
    }

    // The anchor law by hand: what shows at x shows there still. At 64 px a bar (6 ticks a
    // pixel), unscrolled, x 248 shows tick (248 − 120)·6 = 768; at 128 px a bar (3 ticks a
    // pixel) it is at 248 again from scroll 768 − 128·3 = 384. At 128 px a bar scrolled to 960,
    // x 200 shows 960 + 80·3 = 1200; out to 32 px a bar (12 ticks a pixel) it stays at x 200 from
    // scroll 1200 − 80·12 = 240. A tick the start cannot keep there (2880 at x 600 out to 16 px a
    // bar would need scroll 2880 − 480·24 < 0) leaves the view at the start.
    #[test]
    fn a_zoom_keeps_the_tick_under_the_cursor_in_place() {
        let zoomed = grid().zoomed_at(128.0, 248.0);
        assert_eq!((zoomed.zoom, zoomed.scroll), (128.0, Ticks(384)));
        assert_eq!(zoomed.x_of(Ticks(768)), 248.0);
        let near = Grid {
            zoom: 128.0,
            scroll: Ticks(960),
            ..grid()
        };
        assert_eq!(near.tick_at(200.0), Ticks(1200));
        let out = near.zoomed_at(32.0, 200.0);
        assert_eq!((out.zoom, out.scroll), (32.0, Ticks(240)));
        assert_eq!(out.x_of(Ticks(1200)), 200.0);
        assert_eq!(grid().zoomed_at(16.0, 600.0).scroll, Ticks(0));
        assert_eq!(grid().zoomed_at(900.0, 600.0).zoom, 256.0, "kept");
    }

    // The fade law by hand, beat = px_per_bar / 4, alpha = (beat − 5)/14 kept to 0..1: 16 px a
    // bar (4 px beats) and 20 (5 px) draw none; 32 is 3/14 = 0.2143, 48 is 7/14 = 0.5, 64 is
    // 11/14 = 0.7857; 76 (19 px beats) and closer draw them whole.
    #[test]
    fn beat_lines_fade_in_as_the_beats_grow_apart() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-6;
        assert_eq!((beat_alpha(16.0, 4), beat_alpha(20.0, 4)), (0.0, 0.0));
        assert!(close(beat_alpha(32.0, 4), 3.0 / 14.0));
        assert!(close(beat_alpha(48.0, 4), 0.5));
        assert!(close(beat_alpha(64.0, 4), 11.0 / 14.0));
        assert_eq!((beat_alpha(76.0, 4), beat_alpha(256.0, 4)), (1.0, 1.0));
    }

    // The shading rule by hand: at 64 px a bar every other bar from bar 1, so bars 1, 3, 5, 7
    // of the first eight; at 24 px a bar (under 32) every other four bars from bar 1, so bars
    // 1-4 and 9-12 of the first twelve; at 32 exactly, every other bar again. The strong lines:
    // bars 1, 5, 9 and 13 of the first thirteen.
    #[test]
    fn every_other_bar_or_four_bars_is_shaded() {
        let bars = |px: f32, count: i64| -> Vec<i64> {
            (0..count)
                .filter(|&index| shaded(index, px))
                .map(|index| index + 1)
                .collect()
        };
        assert_eq!(bars(64.0, 8), [1, 3, 5, 7]);
        assert_eq!(bars(24.0, 12), [1, 2, 3, 4, 9, 10, 11, 12]);
        assert_eq!(bars(32.0, 4), [1, 3]);
        let strong_bars: Vec<i64> = (0..13).filter(|&i| strong(i)).map(|i| i + 1).collect();
        assert_eq!(strong_bars, [1, 5, 9, 13]);
        // On a 1000 px canvas unscrolled, 64 px a bar: bars 1, 3, ..., 13 at x 120 + 64·(bar − 1),
        // 64 px each. At 24 px a bar (16 ticks a pixel, 96 px a four): fours 1, 3, 5, 7 and 9,
        // from x 120 + 192k.
        let spans: Vec<(f32, f32)> = shaded_spans(&grid(), &common(), 1000.0).collect();
        let expected: Vec<(f32, f32)> = (0..7)
            .map(|k| (120.0 + 128.0 * k as f32, 184.0 + 128.0 * k as f32))
            .collect();
        assert_eq!(spans, expected);
        let far = Grid {
            zoom: 24.0,
            ..grid()
        };
        let spans: Vec<(f32, f32)> = shaded_spans(&far, &common(), 1000.0).collect();
        let expected: Vec<(f32, f32)> = (0..5)
            .map(|k| (120.0 + 192.0 * k as f32, 216.0 + 192.0 * k as f32))
            .collect();
        assert_eq!(spans, expected);
        // Scrolled half a bar in at 64 px a bar: bar 1's shading is cut at the timeline's edge.
        let scrolled = Grid {
            scroll: Ticks(192),
            ..grid()
        };
        assert_eq!(
            shaded_spans(&scrolled, &common(), 1000.0).next(),
            Some((120.0, 152.0))
        );
    }

    // Where a scroll comes to rest, by hand. At 256 px a bar beats are drawn, 96 ticks (64 px)
    // each, 1.5 ticks a pixel: 52 px into the beat at 384 (tick 462) is 12 px short of the next
    // beat, so it lands on that one, 480; 20 px in (tick 414) lands back on 384. At 16 px a bar
    // (4 px beats, none drawn) the cell is the bar, 384 ticks, 24 ticks a pixel: 7 px in (tick
    // 168) lands on bar 1, 9 px in (216) on bar 2. At 20 px a bar beats are 5 px and not drawn
    // (bars); at 21 they are (beats).
    #[test]
    fn a_scroll_lands_on_the_nearest_beat_or_bar() {
        let near = Grid {
            zoom: 256.0,
            ..grid()
        };
        assert_eq!(near.cell(&common(), Ticks(0)), MeterUnit::Beat);
        assert_eq!(near.landed(&common(), Ticks(462)), Ticks(480));
        assert_eq!(near.landed(&common(), Ticks(414)), Ticks(384));
        assert_eq!(
            near.landed(&common(), Ticks(432)),
            Ticks(480),
            "halfway goes on"
        );
        let far = Grid {
            zoom: 16.0,
            ..grid()
        };
        assert_eq!(far.cell(&common(), Ticks(0)), MeterUnit::Bar);
        assert_eq!(far.landed(&common(), Ticks(168)), Ticks(0));
        assert_eq!(far.landed(&common(), Ticks(216)), Ticks(384));
        assert_eq!(
            Grid {
                zoom: 20.0,
                ..grid()
            }
            .cell(&common(), Ticks(0)),
            MeterUnit::Bar
        );
        assert_eq!(
            Grid {
                zoom: 21.0,
                ..grid()
            }
            .cell(&common(), Ticks(0)),
            MeterUnit::Beat
        );
        assert_eq!(near.cell_start(&common(), Ticks(470)), Ticks(384));
        // From the beat at 384, a move of 30 ticks either way (under half a beat) still moves a
        // beat: to 480 on, 288 back; from 400 (mid-beat) back to the beat it is in, 384.
        let at = |scroll| Grid {
            scroll: Ticks(scroll),
            ..near
        };
        assert_eq!(at(384).landed_towards(&common(), Ticks(414)), Ticks(480));
        assert_eq!(at(384).landed_towards(&common(), Ticks(354)), Ticks(288));
        assert_eq!(at(400).landed_towards(&common(), Ticks(390)), Ticks(384));
        assert_eq!(
            at(384).landed_towards(&common(), Ticks(500)),
            Ticks(480),
            "nearest"
        );
        assert_eq!(at(0).landed_towards(&common(), Ticks(0)), Ticks(0));
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
