//! Warp markers as the playlist's gestures make them: pure functions from the project, an audio
//! clip and frames to the edit that commits a marker added, moved or removed, or a quantize. No
//! iced types, so every rule is unit-tested directly, and a drag draws the very list its release
//! commits. Every frame is mapped through [`dawsome_engine::warp`] alone.
//!
//! The frames: a marker's `source` counts the channel's sound (S frames), its `out` the clip's
//! stretched file (T frames); the clip plays stretched frame `frame_offset + k` at song frame
//! `s(start) + k`. A clip not stretched yet maps as one stretched to its own S (T = S): the map a
//! first marker starts from, which [`edit::start_warp`] then makes the clip's.

use dawsome_engine::edit::{self, Edit};
use dawsome_engine::project::{ClipId, ClipItem, Project};
use dawsome_engine::time::{self, MeterMap, Ticks};
use dawsome_engine::warp::{self, ClipWarp, WarpMarker, MAX_WARP_MARKERS, MIN_WARP_SEGMENT};

use crate::ui::clip_gesture::{ClipSnap, SnapGrid};

/// How far from the pointer, in pixels, a double press takes a transient for a new marker.
pub const TRANSIENT_REACH: f32 = 8.0;

/// The lines Quantize audio moves hits to: the snap's, a sixteenth (a step) when it is None.
/// Why not single ticks: every hit would stay where it is.
pub fn quantize_grid(snap: ClipSnap, meter: &MeterMap) -> SnapGrid<'_> {
    match snap {
        ClipSnap::None => ClipSnap::Step.grid(meter),
        other => other.grid(meter),
    }
}

/// Why a marker gesture changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarpRefused {
    /// The first marker turns a time-stretch on, which cannot make fewer than `shortest` frames.
    TooShort { shortest: u64 },
    /// The marker would lie closer than [`MIN_WARP_SEGMENT`] to another or to an end.
    Crowded,
    /// The clip holds [`MAX_WARP_MARKERS`] already.
    Full,
}

/// The map an audio clip plays its sound through, and where it plays it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipMap {
    /// S: the frames of the sound (those its list was set on, once it has markers).
    pub source_frames: u64,
    /// T: the frames of the stretched file (S on a clip not stretched).
    pub frames: u64,
    pub markers: Vec<WarpMarker>,
    pub frame_offset: u64,
    /// The song frame the clip starts at, s(start).
    pub start: u64,
    /// The song frames it plays, s(end) − s(start).
    pub played: u64,
    /// Whether it is stretched already (a first marker on one that is not stretches it).
    pub stretched: bool,
}

impl ClipMap {
    /// Audio clip `id`'s map, its sound `sound_frames` long when known; a clip with markers maps
    /// by the frames its list was set on, which every marker edit keeps. `None` for a clip that
    /// is not audio, or one not stretched whose sound's length is not known yet.
    pub fn of(project: &Project, id: ClipId, sound_frames: Option<u64>) -> Option<Self> {
        let clip = project.playlist.clips.get(&id)?;
        let ClipItem::Audio {
            frame_offset,
            stretch,
            ..
        } = clip.item
        else {
            return None;
        };
        let held = project.playlist.warps.get(&id);
        let source_frames = match held {
            Some(warp) => warp.source_frames,
            None => sound_frames?,
        };
        let sample = |tick: Ticks| {
            time::tick_to_sample(tick, project.sample_rate, project.tempo, project.ppq)
        };
        let start = sample(clip.start)?;
        Some(Self {
            source_frames,
            frames: stretch.map_or(source_frames, |stretch| stretch.frames),
            markers: held.map(|warp| warp.markers.clone()).unwrap_or_default(),
            frame_offset,
            start,
            played: sample(clip.end())?.saturating_sub(start),
            stretched: stretch.is_some(),
        })
    }

    /// The stretched frame sound frame `source` plays at.
    pub fn out_of(&self, source: u64) -> u64 {
        warp::out_of_source(&self.markers, self.source_frames, self.frames, source)
    }

    /// The sound frame stretched frame `out` plays.
    pub fn source_of(&self, out: u64) -> u64 {
        warp::source_of_out(&self.markers, self.source_frames, self.frames, out)
    }

    /// The song frame stretched frame `out` plays at, when the clip plays it.
    pub fn song_of(&self, out: u64) -> Option<u64> {
        let k = out.checked_sub(self.frame_offset)?;
        (k < self.played).then(|| self.start + k)
    }

    /// The stretched frame the clip plays at song frame `song` (its first frame before its start).
    pub fn out_at(&self, song: u64) -> u64 {
        self.frame_offset + song.saturating_sub(self.start)
    }
}

/// The x of song frame `song` on a clip drawn from `left`, `width` px wide, that plays `played`
/// frames from song frame `start` (as [`crate::ui::playlist_layout::fade_reach`] measures a fade).
pub fn x_of_frame(left: f32, width: f32, start: u64, played: u64, song: u64) -> f32 {
    if played == 0 {
        return left;
    }
    left + width * (song.saturating_sub(start) as f64 / played as f64) as f32
}

/// The song frame at x `x` on such a clip, inside it.
pub fn frame_at_x(left: f32, width: f32, start: u64, played: u64, x: f32) -> u64 {
    if width <= 0.0 {
        return start;
    }
    let share = f64::from(((x - left) / width).clamp(0.0, 1.0));
    start + (share * played as f64).floor() as u64
}

/// The sound frame a double press at song frame `song` pins: the transient among `onsets` that
/// plays nearest it within `reach` frames, else the frame playing there.
pub fn pinned_source(map: &ClipMap, onsets: &[u64], song: u64, reach: u64) -> u64 {
    onsets
        .iter()
        .filter_map(|&onset| {
            let at = map.song_of(map.out_of(onset))?;
            let distance = at.abs_diff(song);
            (distance <= reach).then_some((distance, onset))
        })
        .min()
        .map_or_else(|| map.source_of(map.out_at(song)), |(_, onset)| onset)
}

/// The edit that gives audio clip `id` the marker list `markers` on its map `map`: the marker
/// edit on a stretched clip, the first marker's [`edit::start_warp`] on one that is not (a
/// time-stretch, refused below `shortest` frames). `Ok(None)` when the list stays.
pub fn with_markers(
    project: &Project,
    id: ClipId,
    map: &ClipMap,
    markers: Vec<WarpMarker>,
    shortest: u64,
) -> Result<Option<Edit>, WarpRefused> {
    if markers == map.markers {
        return Ok(None);
    }
    if markers.len() > MAX_WARP_MARKERS {
        return Err(WarpRefused::Full);
    }
    if !markers.is_empty() && warp::problem(&markers, map.source_frames, map.frames).is_some() {
        return Err(WarpRefused::Crowded);
    }
    if !map.stretched && map.source_frames < shortest {
        return Err(WarpRefused::TooShort { shortest });
    }
    if map.stretched {
        let warp = (!markers.is_empty()).then_some(ClipWarp {
            source_frames: map.source_frames,
            markers,
        });
        return Ok(Some(Edit::SetClipWarp { clip: id, warp }));
    }
    Ok(edit::start_warp(project, id, map.source_frames, markers).ok())
}

/// A marker that pins sound frame `source` of audio clip `id` where it plays now, so the audio
/// stays where it is: `Ok(None)` when one pins it already.
pub fn added(
    project: &Project,
    id: ClipId,
    map: &ClipMap,
    source: u64,
    shortest: u64,
) -> Result<Option<Edit>, WarpRefused> {
    if map.markers.iter().any(|marker| marker.source == source) {
        return Ok(None);
    }
    if map.markers.len() >= MAX_WARP_MARKERS {
        return Err(WarpRefused::Full);
    }
    let mut markers = map.markers.clone();
    markers.push(WarpMarker {
        source,
        out: map.out_of(source),
    });
    markers.sort_unstable();
    with_markers(project, id, map, markers, shortest)
}

/// The list without its marker `index`; `None` when there is no such marker. Taking a marker out
/// joins its two parts into one, whose ratio lies between theirs, so it keeps every rule.
pub fn removed(project: &Project, id: ClipId, index: usize) -> Option<Edit> {
    let warp = project.playlist.warps.get(&id)?;
    let mut markers = warp.markers.clone();
    (index < markers.len()).then(|| markers.remove(index))?;
    let warp = (!markers.is_empty()).then(|| ClipWarp {
        markers,
        ..warp.clone()
    });
    Some(Edit::SetClipWarp { clip: id, warp })
}

/// The `out` frames marker `index` may move between, its source and its neighbours kept: each
/// of its two parts at least [`MIN_WARP_SEGMENT`] frames long and from a quarter to four times
/// its sound's frames (a uniform stretch's range). `None` for no such marker.
pub fn out_range(map: &ClipMap, index: usize) -> Option<(u64, u64)> {
    let marker = map.markers.get(index)?;
    let previous = index
        .checked_sub(1)
        .map_or((0, 0), |at| (map.markers[at].source, map.markers[at].out));
    let next = map
        .markers
        .get(index + 1)
        .map_or((map.source_frames, map.frames), |next| {
            (next.source, next.out)
        });
    let ratio = dawsome_engine::project::MAX_STRETCH_RATIO;
    let (before, after) = (
        marker.source.saturating_sub(previous.0),
        next.0.saturating_sub(marker.source),
    );
    let low = (previous.1 + MIN_WARP_SEGMENT.max(before.div_ceil(ratio)))
        .max(next.1.saturating_sub(after * ratio));
    let high = (previous.1 + before * ratio).min(
        next.1
            .saturating_sub(MIN_WARP_SEGMENT.max(after.div_ceil(ratio))),
    );
    Some((low, high))
}

/// The list with marker `index` moved to `out`, stopped inside [`out_range`]; the list as it is
/// when the marker cannot move at all.
pub fn dragged(map: &ClipMap, index: usize, out: u64) -> Vec<WarpMarker> {
    let mut markers = map.markers.clone();
    if let (Some((low, high)), Some(marker)) = (out_range(map, index), markers.get_mut(index)) {
        if low <= high {
            marker.out = out.clamp(low, high);
        }
    }
    if warp::problem(&markers, map.source_frames, map.frames).is_some() {
        return map.markers.clone();
    }
    markers
}

/// What a quantize of one clip gives: its hits (the transients inside the part it plays), how
/// many were placed on a line, and the marker list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quantized {
    pub hits: usize,
    pub kept: usize,
    pub markers: Vec<WarpMarker>,
}

/// Quantize audio on clip map `map`: each of `onsets` the clip plays is a hit, in rising order;
/// the line of `grid` nearest where it plays (in frames, half way going on) is its target, its
/// marker (the onset, frame_offset + s(line) − s(start)), one already at the onset replaced. A hit
/// whose line an earlier one took is skipped, and so is one whose marker breaks a rule of the
/// list built so far. gen_p39_expected.py's "quantize" restates it.
pub fn quantized(project: &Project, map: &ClipMap, onsets: &[u64], grid: SnapGrid) -> Quantized {
    let sample =
        |tick: Ticks| time::tick_to_sample(tick, project.sample_rate, project.tempo, project.ppq);
    let last_tick = |frame: u64| {
        let after = frame.checked_add(1)?;
        let tick =
            time::first_tick_at_or_after(after, project.sample_rate, project.tempo, project.ppq)?;
        Some(Ticks(tick.0 - 1))
    };
    let hits: Vec<(u64, u64)> = onsets
        .iter()
        .filter_map(|&onset| Some((onset, map.song_of(map.out_of(onset))?)))
        .collect();
    let mut built: Vec<WarpMarker> = map
        .markers
        .iter()
        .filter(|marker| !hits.iter().any(|&(onset, _)| onset == marker.source))
        .copied()
        .collect();
    let mut taken = Vec::new();
    let mut kept = 0;
    for &(onset, song) in &hits {
        let line = last_tick(song).and_then(|at| {
            let (down, up) = (grid.down(at), grid.next(at));
            let (low, high) = (sample(down)?, sample(up)?);
            Some(if song - low < high - song {
                (down, low)
            } else {
                (up, high)
            })
        });
        let Some((line, line_frame)) = line else {
            continue;
        };
        let out = (map.frame_offset + line_frame).checked_sub(map.start);
        let Some(out) = out.filter(|_| !taken.contains(&line)) else {
            continue;
        };
        let mut trial = built.clone();
        trial.push(WarpMarker { source: onset, out });
        trial.sort_unstable();
        if trial.len() > MAX_WARP_MARKERS
            || warp::problem(&trial, map.source_frames, map.frames).is_some()
        {
            continue;
        }
        built = trial;
        taken.push(line);
        kept += 1;
    }
    Quantized {
        hits: hits.len(),
        kept,
        markers: built,
    }
}

#[cfg(test)]
mod tests {
    use dawsome_engine::project::StretchMode;

    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    fn load(name: &str) -> Project {
        let text = std::fs::read_to_string(std::path::Path::new(FIXTURES).join(name))
            .unwrap_or_else(|_| panic!("{name}"));
        Project::load_json(&text).expect("loads")
    }

    fn p39() -> serde_json::Value {
        let text =
            std::fs::read_to_string(std::path::Path::new(FIXTURES).join("p39_expected.json"))
                .expect("p39_expected.json");
        serde_json::from_str(&text).expect("parses")
    }

    fn frames(value: &serde_json::Value) -> Vec<u64> {
        value
            .as_array()
            .expect("a list")
            .iter()
            .map(|frame| frame.as_u64().expect("a frame"))
            .collect()
    }

    fn at(source: u64, out: u64) -> WarpMarker {
        WarpMarker { source, out }
    }

    fn apply(project: &mut Project, edit: Edit) -> String {
        dawsome_engine::edit::apply(project, edit).expect("applies");
        project.to_json().expect("saves")
    }

    // gen_p39's quantize: the clicks of p39_quantize's clip with the snap None (a sixteenth),
    // through the engine's onsets as the generator states them; the hits, the six kept (the
    // second of the two clicks that want 72000 skipped), the marker list and the document the
    // unstretched clip's first warp leaves.
    #[test]
    fn p39_quantize_gives_the_generators_markers() {
        let want = p39();
        let case = &want["quantize"];
        let mut project = load(case["fixture"].as_str().expect("fixture"));
        let onsets = frames(&want["onsets"]["qclicks"]["frames"]);
        let id = ClipId(1);
        let map = ClipMap::of(&project, id, Some(96_000)).expect("an audio clip");
        assert_eq!(case["snap"], "none");
        let meter = project.meter_map();
        let result = quantized(
            &project,
            &map,
            &onsets,
            quantize_grid(ClipSnap::None, &meter),
        );
        assert_eq!(result.hits, frames(&case["hits"]).len());
        assert_eq!(result.kept as u64, case["kept"].as_u64().expect("kept"));
        let markers: Vec<WarpMarker> = case["markers"]
            .as_array()
            .expect("markers")
            .iter()
            .map(|pair| at(pair[0].as_u64().expect("s"), pair[1].as_u64().expect("o")))
            .collect();
        assert_eq!(result.markers, markers);
        let edit = with_markers(&project, id, &map, result.markers, 0)
            .expect("fits")
            .expect("a change");
        assert_eq!(
            apply(&mut project, edit),
            case["after"].as_str().expect("after")
        );
    }

    // A marker added at a frame of p39_plain's clip (S 24000 stretched to T 36000, no markers):
    // source 12000 plays at out 18000 (12000 · 1.5), so the marker is (12000, 18000) and the map
    // is the line it was. One added twice changes nothing.
    #[test]
    fn an_added_marker_pins_its_frame_where_it_plays() {
        let project = load("p39_plain.json");
        let map = ClipMap::of(&project, ClipId(1), Some(24_000)).expect("audio");
        let edit = added(&project, ClipId(1), &map, 12_000, 0);
        assert_eq!(
            edit,
            Ok(Some(Edit::SetClipWarp {
                clip: ClipId(1),
                warp: Some(ClipWarp {
                    source_frames: 24_000,
                    markers: vec![at(12_000, 18_000)],
                }),
            }))
        );
        let warped = load("p39_warp1.json");
        let map = ClipMap::of(&warped, ClipId(1), Some(24_000)).expect("audio");
        assert_eq!(added(&warped, ClipId(1), &map, 12_000, 0), Ok(None));
        // 63 frames from the sound's start: too close to it.
        assert_eq!(
            added(&warped, ClipId(1), &map, 63, 0),
            Err(WarpRefused::Crowded)
        );
    }

    // On p39_unstretched (S 24000, no stretch) the first marker stretches the clip to its own
    // 24000 frames in the mode that keeps the pitch (gen_p39's "start" edit, (12000, 12000)).
    #[test]
    fn the_first_marker_on_an_unstretched_clip_starts_the_warp() {
        let want = p39();
        let case = want["edits"]
            .as_array()
            .expect("edits")
            .iter()
            .find(|case| case["name"] == "start")
            .expect("the start edit")
            .clone();
        let mut project = load("p39_unstretched.json");
        let map = ClipMap::of(&project, ClipId(1), Some(24_000)).expect("audio");
        assert!(!map.stretched && map.frames == 24_000);
        let edit = added(&project, ClipId(1), &map, 12_000, 2646)
            .expect("long enough")
            .expect("a change");
        assert_eq!(
            apply(&mut project, edit),
            case["after"].as_str().expect("after")
        );
        let ClipItem::Audio { stretch, .. } = project.playlist.clips[&ClipId(1)].item else {
            panic!("audio");
        };
        assert_eq!(
            stretch.map(|stretch| stretch.mode),
            Some(StretchMode::Stretch)
        );
    }

    // The suspicion of the 3a review: a first marker on a sound shorter than the time-stretch's
    // shortest (here 16448 frames against a shortest of 20000) would make a stretch the render
    // refuses, a silent clip; the gesture refuses it instead and changes nothing.
    #[test]
    fn a_first_marker_on_a_sound_too_short_to_stretch_is_refused() {
        let mut project = load("p39_unstretched.json");
        if let Some(channel) = project.channels.values_mut().next() {
            channel.sample = "p39_sound/short.wav".into();
        }
        let map = ClipMap::of(&project, ClipId(1), Some(16_448)).expect("audio");
        assert_eq!(
            added(&project, ClipId(1), &map, 8_000, 20_000),
            Err(WarpRefused::TooShort { shortest: 20_000 })
        );
    }

    // p39_warp3 (S 24000, T 30000; markers (4000, 6000), (10000, 12000), (20000, 21000)): the
    // middle marker moves between its neighbours, each part at least 64 frames and a quarter to
    // four times its sound: its parts hold 6000 and 10000 frames of sound, so out - 6000 lies
    // in 1500..=24000 and 21000 - out in 2500..=40000: 7500..=18500.
    #[test]
    fn a_dragged_marker_stops_at_its_neighbours_and_the_ratios() {
        let project = load("p39_warp3.json");
        let map = ClipMap::of(&project, ClipId(1), None).expect("warped");
        assert_eq!(out_range(&map, 1), Some((7_500, 18_500)));
        let moved = |out| dragged(&map, 1, out)[1].out;
        assert_eq!(moved(15_000), 15_000);
        assert_eq!(moved(1), 7_500);
        assert_eq!(moved(29_000), 18_500);
        // The first marker: (0, 0) before it, 4000 frames of sound: 1000..=16000, and the
        // next part 6000 of sound, 12000 - out in 1500..=24000: up to 10500.
        assert_eq!(out_range(&map, 0), Some((1_000, 10_500)));
        // The last: 10000 of sound before (12000 + 2500..=40000), 4000 to (24000, 30000):
        // 30000 - out in 1000..=16000, so 14500..=29000.
        assert_eq!(out_range(&map, 2), Some((14_500, 29_000)));
        assert_eq!(out_range(&map, 3), None);
    }

    // A double press on a marker of p39_warp3 takes it out; the last one out takes the list
    // away (the clip stays stretched).
    #[test]
    fn a_removed_marker_leaves_its_neighbours() {
        let project = load("p39_warp3.json");
        let Some(Edit::SetClipWarp {
            warp: Some(warp), ..
        }) = removed(&project, ClipId(1), 1)
        else {
            panic!("a marker edit");
        };
        assert_eq!(warp.markers, [at(4_000, 6_000), at(20_000, 21_000)]);
        assert_eq!(warp.source_frames, 24_000);
        let one = load("p39_warp1.json");
        assert_eq!(
            removed(&one, ClipId(1), 0),
            Some(Edit::SetClipWarp {
                clip: ClipId(1),
                warp: None
            })
        );
        assert_eq!(removed(&one, ClipId(1), 1), None);
    }

    // The transient a double press takes: p39_plain's clip (ticks 240..384 at 250 frames a
    // tick: song frames 60000..96000, out 0..36000, the source 2/3 of the out). Onsets at sound
    // frames 1000 and 9000 play at song frames 61500 and 73500; a press at 73000 with a reach
    // of 600 takes 9000, one with a reach of 400 the frame under it, out 13000: 8666.67, half
    // up 8667.
    #[test]
    fn a_double_press_takes_the_nearest_transient_within_reach() {
        let project = load("p39_plain.json");
        let map = ClipMap::of(&project, ClipId(1), Some(24_000)).expect("audio");
        assert_eq!((map.start, map.played), (60_000, 36_000));
        assert_eq!(pinned_source(&map, &[1_000, 9_000], 73_000, 600), 9_000);
        assert_eq!(pinned_source(&map, &[1_000, 9_000], 73_000, 400), 8_667);
    }

    #[test]
    fn frames_and_pixels_map_both_ways_on_a_clip() {
        // A clip 360 px wide playing 36000 frames from song frame 60000, drawn from x 100.
        assert_eq!(x_of_frame(100.0, 360.0, 60_000, 36_000, 78_000), 280.0);
        assert_eq!(frame_at_x(100.0, 360.0, 60_000, 36_000, 280.0), 78_000);
        assert_eq!(frame_at_x(100.0, 360.0, 60_000, 36_000, 0.0), 60_000);
        assert_eq!(frame_at_x(100.0, 360.0, 60_000, 36_000, 900.0), 96_000);
    }
}
