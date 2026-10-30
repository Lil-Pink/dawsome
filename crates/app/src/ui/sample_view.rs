//! The sample editor's geometry, with no iced types (as `instrument_layout.rs` keeps the
//! instrument window's): which frames the waveform shows across its width ([`View`]: the frame
//! at its left edge and the frames a pixel holds), frame and x one from the other, the zoom
//! around the pointer, the scroll of a wheel notch and of the scroll bar, the view fitted to a
//! selection, the selection a drag makes, and the ruler and lanes the waveform is cut into.
//! The window's canvas and the app read it alike, so a press lands on the frame it is drawn
//! over.

use std::time::Duration;

use crate::ui::scroll::Span;

/// The most pixels a frame takes: the zoom's near end.
pub const MOST_PIXELS_PER_FRAME: f64 = 16.0;
/// A Ctrl+wheel notch halves the frames a pixel holds (in) or doubles them (out).
pub const ZOOM_STEP: f64 = 2.0;
/// The ruler over the lanes, and the gap between the left lane and the right one.
pub const RULER_HEIGHT: f32 = 18.0;
pub const LANE_GAP: f32 = 4.0;
/// From this many frames a pixel on, the waveform is drawn from the sound's peaks (a pair per
/// 256 frames); under it from the frames, as lines, and as dots too once a frame is wider than
/// [`DOT_PIXELS`].
pub const PEAKS_FROM: f64 = 256.0;
pub const DOT_PIXELS: f64 = 8.0;
/// How far a selection's edge looks for a zero crossing: 10 ms.
pub const SNAP_MILLIS: u64 = 10;
/// A press let go of within this many pixels of where it went down is a click: the cursor goes
/// there. Further, it is a drag: a selection.
pub const CLICK_TRAVEL: f32 = 3.0;
/// Two clicks this close in time, and within [`CLICK_TRAVEL`] of each other, select the whole
/// sound.
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// The ruler's marks are at least this far apart.
pub const MARK_SPACING: f32 = 80.0;
/// The steps the ruler's marks can be apart, in milliseconds.
const MARK_STEPS: [u64; 16] = [
    1, 2, 5, 10, 20, 50, 100, 200, 500, 1_000, 2_000, 5_000, 10_000, 20_000, 60_000, 120_000,
];

/// The frames the waveform shows: `first` at its left edge, `per_px` frames to a pixel, across
/// `width` pixels, of a sound `frames` long. `first` and `per_px` are fractional: zoomed in, a
/// frame is many pixels wide and the view starts inside one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub frames: u64,
    pub width: f32,
    pub first: f64,
    pub per_px: f64,
}

impl View {
    /// The whole sound across `width` pixels (Fit), at most [`MOST_PIXELS_PER_FRAME`] to a
    /// frame.
    pub fn fit(frames: u64, width: f32) -> Self {
        let view = Self {
            frames,
            width,
            first: 0.0,
            per_px: 0.0,
        };
        Self {
            per_px: view.fit_per_px(),
            ..view
        }
    }

    /// The fewest frames a pixel holds.
    pub fn least_per_px() -> f64 {
        1.0 / MOST_PIXELS_PER_FRAME
    }

    /// The frames a pixel holds when the whole sound fits the width (never fewer than the
    /// least): the zoom's far end.
    pub fn fit_per_px(&self) -> f64 {
        let width = f64::from(self.width.max(1.0));
        (self.frames as f64 / width).max(Self::least_per_px())
    }

    /// The frames the width shows.
    pub fn shown(&self) -> f64 {
        f64::from(self.width.max(0.0)) * self.per_px
    }

    /// Where `frame` (a boundary: frame `n` is the left edge of frame n) is drawn.
    pub fn x_of(&self, frame: f64) -> f32 {
        ((frame - self.first) / self.per_px) as f32
    }

    /// The frame position under `x`, unrounded.
    pub fn at(&self, x: f32) -> f64 {
        self.first + f64::from(x) * self.per_px
    }

    /// The boundary between frames nearest `x`, within the sound: where a click puts the
    /// cursor and a drag an edge of its selection.
    pub fn edge_at(&self, x: f32) -> u64 {
        let at = self.at(x).round();
        if at <= 0.0 {
            0
        } else {
            (at as u64).min(self.frames)
        }
    }

    /// A Ctrl+wheel notch at `x`: the frames a pixel holds halved (`inward`) or doubled,
    /// within the zoom's ends, the frame under `x` staying under it.
    pub fn zoomed(self, x: f32, inward: bool) -> Self {
        let under = self.at(x);
        let per_px = if inward {
            self.per_px / ZOOM_STEP
        } else {
            self.per_px * ZOOM_STEP
        };
        let per_px = per_px.clamp(Self::least_per_px(), self.fit_per_px());
        Self {
            per_px,
            first: under - f64::from(x) * per_px,
            ..self
        }
        .clamped()
    }

    /// The view as the scroll bar spans it, in frames.
    pub fn span(&self) -> Span {
        Span {
            content: self.frames as f64,
            offset: self.first,
            view: self.shown(),
        }
    }

    /// The view from `offset` (a scroll bar's), kept inside the sound.
    pub fn scrolled_to(self, offset: f64) -> Self {
        Self {
            first: offset,
            ..self
        }
        .clamped()
    }

    /// A wheel notch (or Shift+wheel, or a wheel across): an eighth of the view later
    /// (`forward`) or earlier, as the scroll bar's notch.
    pub fn wheeled(self, forward: bool) -> Self {
        self.scrolled_to(self.span().wheeled(forward))
    }

    /// Zoom to selection: frames `from` to `to` across the width, as near as the zoom's ends
    /// let it (a part narrower than the nearest zoom shows centered).
    pub fn showing(self, from: u64, to: u64) -> Self {
        let (from, to) = (from.min(to) as f64, from.max(to) as f64);
        let width = f64::from(self.width.max(1.0));
        let per_px = ((to - from) / width).clamp(Self::least_per_px(), self.fit_per_px());
        let middle = (from + to) / 2.0;
        Self {
            per_px,
            first: middle - width * per_px / 2.0,
            ..self
        }
        .clamped()
    }

    /// The view on a window now `width` pixels across: the whole sound still when it showed
    /// all of it, else from the same frame at the same zoom (unless the whole sound now takes
    /// fewer frames a pixel).
    pub fn resized(self, width: f32) -> Self {
        let fitted = self.per_px >= self.fit_per_px();
        let view = Self { width, ..self };
        if fitted {
            Self {
                per_px: view.fit_per_px(),
                ..view
            }
            .clamped()
        } else {
            view.clamped()
        }
    }

    /// The view of the sound once an op left it `frames` long: from the same frame, at the
    /// same zoom unless the whole sound now takes fewer frames a pixel.
    pub fn with_frames(self, frames: u64) -> Self {
        Self { frames, ..self }.clamped()
    }

    /// The zoom between its ends and the view inside the sound (from 0 when it all shows).
    fn clamped(self) -> Self {
        let per_px = self.per_px.clamp(Self::least_per_px(), self.fit_per_px());
        let view = Self { per_px, ..self };
        let last = (self.frames as f64 - view.shown()).max(0.0);
        Self {
            first: view.first.clamp(0.0, last),
            ..view
        }
    }

    /// How the waveform is drawn at this zoom.
    pub fn drawn(&self) -> Drawn {
        if self.per_px >= PEAKS_FROM {
            Drawn::Peaks
        } else if self.per_px >= 1.0 {
            Drawn::Columns
        } else if 1.0 / self.per_px > DOT_PIXELS {
            Drawn::Dots
        } else {
            Drawn::Lines
        }
    }

    /// The frames pixel column `column` covers: `[from, to)`, inside the sound.
    pub fn column(&self, column: u32) -> (u64, u64) {
        let edge = |x: f64| {
            let at = (self.first + x * self.per_px).floor();
            if at <= 0.0 {
                0
            } else {
                (at as u64).min(self.frames)
            }
        };
        let x = f64::from(column);
        let (from, to) = (edge(x), edge(x + 1.0));
        (from, to.max(from + 1).min(self.frames))
    }

    /// The frames whose points show (one before the left edge and one past the right, so the
    /// lines run off both edges): `[from, to)`.
    pub fn frames_shown(&self) -> (u64, u64) {
        let from = self.first.floor().max(0.0) as u64;
        let to = (self.first + self.shown()).ceil().max(0.0) as u64 + 1;
        (from.saturating_sub(1), to.min(self.frames))
    }

    /// The ruler's marks: each frame a whole step of milliseconds apart (the least of
    /// [`MARK_STEPS`] at least [`MARK_SPACING`] pixels apart at `rate`) inside the view, with
    /// where it is drawn.
    pub fn marks(&self, rate: u32) -> Vec<(f32, u64)> {
        let rate = u64::from(rate.max(1));
        let spacing = |step: u64| (step * rate) as f64 / 1000.0 / self.per_px;
        let step = MARK_STEPS
            .into_iter()
            .find(|&step| spacing(step) >= f64::from(MARK_SPACING))
            .unwrap_or(MARK_STEPS[MARK_STEPS.len() - 1]);
        let (from, to) = (self.first.max(0.0), self.first + self.shown());
        let first = (from * 1000.0 / rate as f64 / step as f64).ceil() as u64;
        (first..)
            .map(|index| index * step)
            .map(|millis| millis * rate / 1000)
            .take_while(|&frame| (frame as f64) <= to.min(self.frames as f64))
            .map(|frame| (self.x_of(frame as f64), frame))
            .collect()
    }
}

/// How the waveform is drawn: from the peaks, a column of each pixel's least and largest value,
/// from the frames as columns too, as a line through the frames, or as the line and a dot on
/// each frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drawn {
    Peaks,
    Columns,
    Lines,
    Dots,
}

/// A selection: frames `[from, to)`, never empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub from: u64,
    pub to: u64,
}

impl Selection {
    /// The frames between two edges, whichever is first; none when they are one.
    pub fn between(a: u64, b: u64) -> Option<Self> {
        (a != b).then(|| Self {
            from: a.min(b),
            to: a.max(b),
        })
    }

    /// Inside a sound of `frames` frames: cut to its end; none when nothing is left.
    pub fn within(self, frames: u64) -> Option<Self> {
        Self::between(self.from.min(frames), self.to.min(frames))
    }
}

/// The frames of a sound `frames` long that an audio clip plays (its window, bracketed on the
/// ruler while the editor is opened from it): `played` frames of the song from `frame_offset`,
/// or, for a clip stretched to `stretched` frames, those stretched frames taken to the sound's
/// at the same share of its length (floored, as the playlist draws them), cut to the sound;
/// none when nothing of it is left.
pub fn clip_window(
    frame_offset: u64,
    played: u64,
    stretched: Option<u64>,
    frames: u64,
) -> Option<Selection> {
    let end = frame_offset.saturating_add(played);
    let (from, to) = match stretched.filter(|&total| total > 0) {
        Some(total) => {
            let scale = |frame: u64| {
                u64::try_from(u128::from(frame) * u128::from(frames) / u128::from(total))
                    .unwrap_or(u64::MAX)
            };
            (scale(frame_offset), scale(end))
        }
        None => (frame_offset, end),
    };
    Selection::between(from.min(frames), to.min(frames))
}

/// The frames a zero-crossing snap reaches at `rate`: [`SNAP_MILLIS`] of them.
pub fn snap_reach(rate: u32) -> u64 {
    u64::from(rate) * SNAP_MILLIS / 1000
}

/// The lanes under the ruler of a waveform `height` tall for `sides` sides (1 or 2), left over
/// right: each one's top and height.
pub fn lanes(height: f32, sides: u8) -> Vec<(f32, f32)> {
    let sides = sides.clamp(1, 2);
    let room = (height - RULER_HEIGHT).max(0.0);
    let gaps = LANE_GAP * f32::from(sides - 1);
    let lane = ((room - gaps) / f32::from(sides)).max(0.0);
    (0..sides)
        .map(|side| (RULER_HEIGHT + f32::from(side) * (lane + LANE_GAP), lane))
        .collect()
}

/// Where `value` is drawn in a lane at `top`, `height` tall: full scale at its edges, 0 across
/// its middle; a value past full scale beyond its edge (the canvas clips it there).
pub fn value_y(top: f32, height: f32, value: f32) -> f32 {
    top + height / 2.0 - value * height / 2.0
}

/// A time on the ruler and in the header: m:ss.mmm of frame `frame` at `rate` (milliseconds
/// floored).
pub fn time_text(frame: u64, rate: u32) -> String {
    let millis = frame * 1000 / u64::from(rate.max(1));
    format!(
        "{}:{:02}.{:03}",
        millis / 60_000,
        millis / 1000 % 60,
        millis % 1000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // By hand: 48000 frames across 800 px fit at 60 frames a pixel; x 200 is over frame 12000
    // and frame 24000 at x 400. Ctrl+wheel in at x 200 halves it to 30, frame 12000 staying
    // under x 200, so the view starts at 12000 - 200 * 30 = 6000; again in, 15 and
    // 12000 - 3000 = 9000. Out from the fit stays the fit. Ten notches in from 60 reach
    // 60 / 1024 = 0.0586, under the least 1/16: the zoom stops at 16 px a frame.
    #[test]
    fn a_ctrl_wheel_notch_zooms_around_the_pointer() {
        let fit = View::fit(48_000, 800.0);
        assert_eq!((fit.first, fit.per_px), (0.0, 60.0));
        assert_eq!(fit.edge_at(200.0), 12_000);
        assert_eq!(fit.x_of(24_000.0), 400.0);
        let once = fit.zoomed(200.0, true);
        assert_eq!((once.first, once.per_px), (6000.0, 30.0));
        let twice = once.zoomed(200.0, true);
        assert_eq!((twice.first, twice.per_px), (9000.0, 15.0));
        assert_eq!(twice.edge_at(200.0), 12_000, "still under the pointer");
        assert_eq!(fit.zoomed(200.0, false), fit, "out from the fit: the fit");
        assert_eq!(twice.zoomed(200.0, false), once, "out again: as it was");
        let mut deep = fit;
        for _ in 0..10 {
            deep = deep.zoomed(400.0, true);
        }
        assert_eq!(deep.per_px, 1.0 / 16.0);
        // 800 px at 16 px a frame show 50 frames, frame 24000 under x 400: from 23975.
        assert_eq!(deep.first, 23_975.0);
        assert_eq!(deep.drawn(), Drawn::Dots);
    }

    // By hand: at 30 frames a pixel the 800 px show 24000 frames, a wheel notch an eighth of
    // them, 3000: from 6000 to 9000 and back. The view stops at the sound's end, 48000 - 24000
    // = 24000, and at its start. Zoom to selection of 1000..2000 shows 1000 frames over 800
    // px, 1.25 a pixel, from 1000; of 1000..1010, 10 frames would be 80 px each, so it stops at
    // 16 px a frame (50 frames) centered on 1005, from 980.
    #[test]
    fn a_wheel_notch_scrolls_an_eighth_and_zoom_to_selection_fits_the_part() {
        let view = View {
            frames: 48_000,
            width: 800.0,
            first: 6000.0,
            per_px: 30.0,
        };
        assert_eq!(view.shown(), 24_000.0);
        assert_eq!(view.wheeled(true).first, 9000.0);
        assert_eq!(view.wheeled(false).first, 3000.0);
        assert_eq!(
            view.scrolled_to(30_000.0).first,
            24_000.0,
            "the sound's end"
        );
        assert_eq!(view.scrolled_to(-5.0).first, 0.0);
        let part = view.showing(1000, 2000);
        assert_eq!((part.first, part.per_px), (1000.0, 1.25));
        assert_eq!(part.drawn(), Drawn::Columns);
        let narrow = view.showing(1010, 1000);
        assert_eq!((narrow.first, narrow.per_px), (980.0, 1.0 / 16.0));
        // The scroll bar's span is the view in frames.
        let span = view.span();
        assert_eq!(
            (span.content, span.offset, span.view),
            (48_000.0, 6000.0, 24_000.0)
        );
    }

    // A sound shorter than a view at the nearest zoom: 18 frames in 800 px take 16 px a frame
    // (288 px), from 0, and x 112 is the edge before frame 7, x 80 before frame 5, x 120 rounds
    // to 8; past the sound an edge is its end.
    #[test]
    fn a_short_sound_shows_at_the_nearest_zoom_from_its_start() {
        let view = View::fit(18, 800.0);
        assert_eq!((view.first, view.per_px), (0.0, 1.0 / 16.0));
        assert_eq!(view.edge_at(112.0), 7);
        assert_eq!(view.edge_at(80.0), 5);
        assert_eq!(view.edge_at(120.0), 8);
        assert_eq!(view.edge_at(500.0), 18);
        assert_eq!(view.edge_at(-10.0), 0);
        assert_eq!(view.wheeled(true), view, "nothing to scroll");
    }

    // After an op: a crop to 24000 frames keeps the zoom where the sound still fills more than
    // the view (30 frames a pixel, 24000 frames: all of it, from 0); a window made narrower
    // keeps the zoom and the first frame, but a view of the whole sound fits the new width.
    #[test]
    fn a_shorter_sound_or_a_narrower_window_keeps_the_view_inside_it() {
        let view = View {
            frames: 48_000,
            width: 800.0,
            first: 9000.0,
            per_px: 30.0,
        };
        let cropped = view.with_frames(24_000);
        assert_eq!((cropped.first, cropped.per_px), (0.0, 30.0));
        let short = view.with_frames(12_000);
        assert_eq!((short.first, short.per_px), (0.0, 15.0), "the fit of 12000");
        let narrow = view.resized(400.0);
        assert_eq!((narrow.first, narrow.per_px), (9000.0, 30.0));
        // The whole sound shown stays shown: fitted to 876 px, then 800 (60 a pixel).
        let fitted = View::fit(48_000, 876.0).resized(800.0);
        assert_eq!((fitted.first, fitted.per_px), (0.0, 60.0));
    }

    // Pixel columns: at 60 frames a pixel column 3 is frames 180..240; at 1.25, column 0 of a
    // view from 1000 is 1000..1001 (floor of 1001.25), column 1 is 1001..1002. The frames shown
    // at 16 px a frame from 23975: 23974 (one before) to 24026 (one past 24025).
    #[test]
    fn a_columns_frames_and_the_frames_shown() {
        let fit = View::fit(48_000, 800.0);
        assert_eq!(fit.column(3), (180, 240));
        let part = fit.showing(1000, 2000);
        assert_eq!(part.column(0), (1000, 1001));
        assert_eq!(part.column(1), (1001, 1002));
        let deep = View {
            frames: 48_000,
            width: 800.0,
            first: 23_975.0,
            per_px: 1.0 / 16.0,
        };
        assert_eq!(deep.frames_shown(), (23_974, 24_026));
    }

    // The ruler at 44.1 kHz: 48000 frames over 800 px (60 a pixel), 80 px is 4800 frames,
    // 108.8 ms, so the marks are 200 ms (8820 frames, 147 px) apart: 0, 8820, ... 44100. Zoomed
    // to 1000..2000 (1.25 a pixel) 80 px is 100 frames, 2.27 ms: 5 ms apart, from the first
    // after frame 1000 (22.7 ms): 25 ms, frame 1102 (1102.5 floored), then 30 ms, 1323.
    #[test]
    fn the_rulers_marks_are_a_whole_step_of_time_apart() {
        let fit = View::fit(48_000, 800.0);
        let marks = fit.marks(44_100);
        let frames: Vec<u64> = marks.iter().map(|&(_, frame)| frame).collect();
        assert_eq!(frames, [0, 8820, 17_640, 26_460, 35_280, 44_100]);
        assert_eq!(marks[1].0, 147.0);
        let part = fit.showing(1000, 2000).marks(44_100);
        assert_eq!(part[0].1, 1102);
        assert_eq!(part[1].1, 1323);
        assert_eq!(time_text(44_100, 44_100), "0:01.000");
        assert_eq!(time_text(1102, 44_100), "0:00.024");
        assert_eq!(time_text(28_800_000, 48_000), "10:00.000");
    }

    // Two lanes under the 18 px ruler of a 238 px waveform: 220 px less the 4 px gap is 108
    // each, the right from 18 + 112 = 130; one lane takes all 220. Full scale is a lane's top
    // and bottom.
    #[test]
    fn the_lanes_split_the_waveform_under_the_ruler() {
        assert_eq!(lanes(238.0, 2), [(18.0, 108.0), (130.0, 108.0)]);
        assert_eq!(lanes(238.0, 1), [(18.0, 220.0)]);
        assert_eq!(value_y(18.0, 108.0, 1.0), 18.0);
        assert_eq!(value_y(18.0, 108.0, -1.0), 126.0);
        assert_eq!(value_y(18.0, 108.0, 0.0), 72.0);
    }

    #[test]
    fn a_selection_is_between_two_edges_and_inside_the_sound() {
        assert_eq!(
            Selection::between(18_000, 6000),
            Some(Selection {
                from: 6000,
                to: 18_000
            })
        );
        assert_eq!(Selection::between(5, 5), None);
        let selection = Selection {
            from: 900,
            to: 2000,
        };
        assert_eq!(
            selection.within(1000),
            Some(Selection {
                from: 900,
                to: 1000
            })
        );
        assert_eq!(selection.within(800), None);
        assert_eq!(snap_reach(44_100), 441);
        assert_eq!(snap_reach(48_000), 480);
    }

    // By hand: a clip from frame 100 playing 50 frames covers 100..150 of a 1000-frame sound;
    // stretched to 2000 frames (twice the sound), from its frame 300 for 100, it plays the
    // sound's 150..200. p30_undo's clips (a bar at 120 BPM, 44.1 kHz: 88200 frames): the one at
    // 48000 reaches past the 72000-frame sound, so 48000..72000; the one stretched to 108000
    // from 30000 plays the sound's 30000 * 72000 / 108000 = 20000 on, cut at 72000. Past the
    // end (the crop of 1000..2000 leaves them at 1000 of 1000 frames) there is none.
    #[test]
    fn a_clips_window_is_the_frames_it_plays_of_the_sound() {
        let window = |from, to| Some(Selection { from, to });
        assert_eq!(clip_window(100, 50, None, 1000), window(100, 150));
        assert_eq!(clip_window(300, 100, Some(2000), 1000), window(150, 200));
        assert_eq!(
            clip_window(48_000, 88_200, None, 72_000),
            window(48_000, 72_000)
        );
        assert_eq!(
            clip_window(30_000, 88_200, Some(108_000), 72_000),
            window(20_000, 72_000)
        );
        assert_eq!(clip_window(1000, 88_200, None, 1000), None);
        assert_eq!(clip_window(1500, 88_200, Some(1500), 1000), None);
    }
}
