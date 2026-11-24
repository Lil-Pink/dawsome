//! Warp markers: the moments of a stretched audio clip's sound pinned to moments of its
//! stretched file, so one clip can stretch by different ratios in different parts. A marker
//! pairs a frame of the channel's (edited) sound, S frames long, with a frame of the stretched
//! file, T frames long (the clip's [`ClipStretch::frames`]); the map between the two is the
//! straight line through (0, 0), the markers in order and (S, T).
//!
//! Drawing, gestures, the media render and the known-answer generator all read the one map
//! here, in whole numbers, so a marker drawn at a frame plays at that frame.
//!
//! [`ClipStretch::frames`]: crate::project::ClipStretch::frames

use std::iter;

use serde::{Deserialize, Serialize};

use crate::project::MAX_STRETCH_RATIO;

/// The most markers one clip holds.
pub const MAX_WARP_MARKERS: usize = 256;

/// The fewest frames between neighbouring anchors (the ends (0, 0) and (S, T) included), in the
/// sound and in the stretched file alike. Why 64 and no more: a time-stretch feeds the library
/// segment by segment, and it takes any lengths in and out; its own floor, twice its output
/// latency, is on the whole file, which every stretch already keeps. 64 frames (1.3 ms at 48
/// kHz) keeps a segment a span of audio, not a single sample's jump.
pub const MIN_WARP_SEGMENT: u64 = 64;

/// A frame of the sound (`source`) and the frame of the stretched file it plays at (`out`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WarpMarker {
    pub source: u64,
    pub out: u64,
}

/// A clip's warp markers with the length S of the sound they were placed on, so the document
/// alone holds every anchor of the map, (S, T) included, and its last segment is checked like
/// the others. Why S can be kept: an edit of the sound that changes its length is refused while
/// a clip of its channel has markers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClipWarp {
    /// S: the frames of the channel's (edited) sound when the markers were set.
    pub source_frames: u64,
    pub markers: Vec<WarpMarker>,
}

impl ClipWarp {
    /// Which rule the list breaks on a stretch of `frames` (T) frames ([`problem`]).
    pub fn problem(&self, frames: u64) -> Option<&'static str> {
        problem(&self.markers, self.source_frames, frames)
    }
}

/// The anchors of the map: (0, 0), `markers`, (S, T).
fn anchors(
    markers: &[WarpMarker],
    source_frames: u64,
    frames: u64,
) -> impl Iterator<Item = (u64, u64)> + '_ {
    iter::once((0, 0))
        .chain(markers.iter().map(|marker| (marker.source, marker.out)))
        .chain(iter::once((source_frames, frames)))
}

/// Each segment of the map in order: (source from, out from, source to, out to).
pub fn segments(
    markers: &[WarpMarker],
    source_frames: u64,
    frames: u64,
) -> impl Iterator<Item = (u64, u64, u64, u64)> + '_ {
    anchors(markers, source_frames, frames)
        .zip(anchors(markers, source_frames, frames).skip(1))
        .map(|((s0, o0), (s1, o1))| (s0, o0, s1, o1))
}

/// `n / d` rounded half up, for `d` > 0: the whole part, one more when the rest is at least
/// half of `d` (taken as rest >= d − rest, which no product of two frame counts overflows).
fn half_up(n: u128, d: u128) -> u128 {
    let (whole, rest) = (n / d, n % d);
    whole + u128::from(rest >= d - rest)
}

/// Where `x` of the first column lands in the second on the line through `points` (pairs of the
/// two columns, in order): `x` past the last point counts as it; in the first segment whose end
/// reaches `x`, its start plus the part of the segment `x` is in, half up.
fn mapped(points: impl Iterator<Item = (u64, u64)>, x: u64) -> u64 {
    let mut previous = (0, 0);
    for (index, point) in points.enumerate() {
        let (x1, y1) = point;
        if index > 0 && x <= x1 {
            let (x0, y0) = previous;
            if x1 <= x0 || x < x0 {
                return y0;
            }
            let rise = u128::from(y1.saturating_sub(y0));
            let part = half_up(u128::from(x - x0) * rise, u128::from(x1 - x0));
            return y0.saturating_add(u64::try_from(part).unwrap_or(u64::MAX));
        }
        previous = point;
    }
    previous.1
}

/// The frame of the stretched file (T frames) that frame `source` of the sound (S frames) plays
/// at, through `markers`: exact at each anchor, half up between, T from S on.
pub fn out_of_source(markers: &[WarpMarker], source_frames: u64, frames: u64, source: u64) -> u64 {
    mapped(
        anchors(markers, source_frames, frames),
        source.min(source_frames),
    )
}

/// The frame of the sound (S frames) that frame `out` of the stretched file (T frames) plays,
/// through `markers`: the map the other way, by the same rule; S from T on.
pub fn source_of_out(markers: &[WarpMarker], source_frames: u64, frames: u64, out: u64) -> u64 {
    let swapped = anchors(markers, source_frames, frames).map(|(source, out)| (out, source));
    mapped(swapped, out.min(frames))
}

const SPACING: &str =
    "markers lie at least 64 frames apart, from each other and the ends, in order";
const RATIO: &str =
    "each part between markers stretches to between a quarter and four times its length";

/// Which rule `markers` on a sound of `source_frames` (S) stretched to `frames` (T) breaks, as
/// the file's refusal, an edit's and the render's say it: every segment, the last (from the
/// last marker to (S, T)) included.
pub fn problem(markers: &[WarpMarker], source_frames: u64, frames: u64) -> Option<&'static str> {
    if markers.is_empty() {
        return Some("a clip with no markers holds no list");
    }
    if markers.len() > MAX_WARP_MARKERS {
        return Some("a clip holds at most 256 markers");
    }
    for (s0, o0, s1, o1) in segments(markers, source_frames, frames) {
        if o1 < o0.saturating_add(MIN_WARP_SEGMENT) || s1 < s0.saturating_add(MIN_WARP_SEGMENT) {
            return Some(SPACING);
        }
        let (source, out) = (u128::from(s1 - s0), u128::from(o1 - o0));
        let ratio = u128::from(MAX_STRETCH_RATIO);
        if source > out * ratio || out > source * ratio {
            return Some(RATIO);
        }
    }
    None
}

/// `markers` on a stretch of `old` frames moved to one of `new`: every `out` scaled by
/// new / old, half up; the sources stay.
pub fn scaled(markers: &[WarpMarker], old: u64, new: u64) -> Vec<WarpMarker> {
    markers
        .iter()
        .map(|marker| {
            let out = half_up(
                u128::from(marker.out) * u128::from(new),
                u128::from(old.max(1)),
            );
            WarpMarker {
                source: marker.source,
                out: u64::try_from(out).unwrap_or(u64::MAX),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(source: u64, out: u64) -> WarpMarker {
        WarpMarker { source, out }
    }

    // A list that breaks the order is no map the rules allow, but the map still gives a frame
    // inside the file for it, never a panic.
    #[test]
    fn a_list_out_of_order_still_maps_inside_the_file() {
        let list = [at(500, 900), at(100, 50)];
        for x in [0, 99, 100, 300, 500, 1000, 5000] {
            assert!(out_of_source(&list, 1000, 1000, x) <= 1000, "{x}");
            assert!(source_of_out(&list, 1000, 1000, x) <= 1000, "{x}");
        }
        assert_eq!(out_of_source(&[], 0, 0, 7), 0);
    }

    #[test]
    fn the_rules_name_what_a_list_breaks() {
        assert_eq!(problem(&[at(64, 64)], 128, 128), None);
        assert!(problem(&[at(64, 64)], 128, 127).is_some(), "the end");
        assert!(problem(&[at(63, 64)], 1000, 1000).is_some(), "the start");
        assert!(problem(&[at(64, 257)], 1000, 1000).is_some(), "the ratio");
        assert!(problem(&[], 1000, 1000).is_some(), "an empty list");
        assert_eq!(
            problem(&[at(64, 64)], 127, 1000),
            Some(SPACING),
            "the sound's end"
        );
        assert_eq!(
            problem(&[at(64, 64)], 1000, 128),
            Some(RATIO),
            "the last segment"
        );
        assert_eq!(
            problem(&[at(500, 500)], 1000, 3000),
            Some(RATIO),
            "its ratio"
        );
    }

    #[test]
    fn a_new_length_scales_each_out_half_up() {
        let list = [at(10, 6000), at(20, 21000)];
        assert_eq!(scaled(&list, 30000, 30005), [at(10, 6001), at(20, 21004)]);
    }
}
