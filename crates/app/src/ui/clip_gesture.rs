//! Playlist gestures as edits: pure functions from the playlist and a gesture's ticks and track
//! rows to the [`Edit`] that commits it, and to the targets a drag shows while it is held. No
//! iced types, so every rule is unit-tested directly, and a drag draws the very targets its
//! release commits.

use std::collections::BTreeSet;
use std::fmt;

use dawsome_engine::edit::Edit;
use dawsome_engine::project::{
    Clip, ClipId, ClipItem, ClipStretch, LoopRange, PatternId, Project, StretchMode, TrackId,
    STRETCH_PITCH_RANGE,
};
use dawsome_engine::time::{self, Ticks};

use crate::ui::note_gesture::{snap_down, snap_nearest};
use crate::ui::playlist_layout::LoopEdge;

/// What playlist positions snap to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ClipSnap {
    #[default]
    Bar,
    Beat,
    Step,
    /// Single ticks.
    None,
}

impl ClipSnap {
    /// Every choice, in the order the header shows them.
    pub const ALL: [ClipSnap; 4] = [
        ClipSnap::Bar,
        ClipSnap::Beat,
        ClipSnap::Step,
        ClipSnap::None,
    ];

    /// The snap unit in ticks at `ppq`: a bar, a beat, a step (a sixteenth) or one tick.
    pub fn unit(self, ppq: u16) -> Ticks {
        match self {
            ClipSnap::Bar => time::bars_to_ticks(1, ppq),
            ClipSnap::Beat => Ticks(i64::from(ppq)),
            ClipSnap::Step => time::step_to_ticks(1, ppq),
            ClipSnap::None => Ticks(1),
        }
    }
}

impl fmt::Display for ClipSnap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ClipSnap::Bar => "Bar",
            ClipSnap::Beat => "Beat",
            ClipSnap::Step => "Step",
            ClipSnap::None => "None",
        })
    }
}

/// The track at `index` in track order.
pub fn track_at(project: &Project, index: usize) -> Option<TrackId> {
    project.playlist.tracks.keys().nth(index).copied()
}

/// Where `track` is in track order.
pub fn index_of(project: &Project, track: TrackId) -> Option<usize> {
    project.playlist.tracks.keys().position(|&id| id == track)
}

/// A press on empty lane space: `pattern` placed on the track at `index`, at `tick` snapped
/// down to the grid, as long as the pattern. A new clip of a 1-bar pattern is one bar long.
pub fn place(
    project: &Project,
    pattern: PatternId,
    index: usize,
    tick: Ticks,
    unit: Ticks,
) -> Option<Edit> {
    let track = track_at(project, index)?;
    let bars = project.patterns.get(&pattern)?.length_bars;
    let start = snap_down(tick, unit);
    if start.0 < 0 {
        return None;
    }
    let length = time::bars_to_ticks(bars, project.ppq);
    Some(Edit::PlaceClips {
        first: project.playlist.next_clip_id,
        clips: vec![Clip::of_pattern(pattern, track, start, length)],
    })
}

/// Where a drag of `ticks` and `rows` puts each of `ids`, as (clip, start, track index) in
/// `ids` order: the tick offset rounded to the nearest whole unit, and both offsets kept so no
/// clip lands before tick 0 or off the tracks. The group moves as one, so an off-grid clip
/// stays as far off the grid as it was.
pub fn move_targets(
    project: &Project,
    ids: &[ClipId],
    ticks: Ticks,
    rows: i64,
    unit: Ticks,
) -> Vec<(ClipId, Ticks, usize)> {
    let clips: Vec<(ClipId, &Clip, usize)> = ids
        .iter()
        .filter_map(|&id| {
            let clip = project.playlist.clips.get(&id)?;
            Some((id, clip, index_of(project, clip.track)?))
        })
        .collect();
    let (Some(earliest), Some(top), Some(bottom)) = (
        clips.iter().map(|(_, clip, _)| clip.start.0).min(),
        clips.iter().map(|&(_, _, index)| index).min(),
        clips.iter().map(|&(_, _, index)| index).max(),
    ) else {
        return Vec::new();
    };
    let unit = unit.0.max(1);
    // The furthest left the group can go in whole units: the earliest start rounded down.
    let lowest = -(earliest.div_euclid(unit) * unit);
    let ticks = snap_nearest(ticks, Ticks(unit)).0.max(lowest);
    let last = project.playlist.tracks.len().saturating_sub(1);
    let rows = rows.clamp(-(top as i64), last.saturating_sub(bottom) as i64);
    clips
        .into_iter()
        .map(|(id, clip, index)| {
            // Clamped above to stay within 0..=last, so the row fits a usize.
            let index = (index as i64 + rows) as usize;
            (id, Ticks(clip.start.0 + ticks), index)
        })
        .collect()
}

/// The one edit that moves clips to `targets`, or `None` when none of them moves.
pub fn moved(project: &Project, targets: &[(ClipId, Ticks, usize)]) -> Option<Edit> {
    let moves: Vec<(ClipId, Ticks, TrackId)> = targets
        .iter()
        .map(|&(id, start, index)| Some((id, start, track_at(project, index)?)))
        .collect::<Option<_>>()?;
    let changed = moves.iter().any(|&(id, start, track)| {
        project
            .playlist
            .clips
            .get(&id)
            .is_some_and(|clip| (clip.start, clip.track) != (start, track))
    });
    changed.then_some(Edit::MoveClips { moves })
}

/// The length a right-edge drag of `ticks` gives `clip`: its end moved by the drag and floored
/// to the grid, and at least one grid line past the cell the clip starts in, so the end stays
/// on the grid even for a clip that starts off it (an on-grid clip stays a unit long). A press
/// without a drag keeps the length, even for an end off the grid.
pub fn resized_length(clip: &Clip, ticks: Ticks, unit: Ticks) -> Ticks {
    if ticks.0 == 0 {
        return clip.length;
    }
    let unit = Ticks(unit.0.max(1));
    let end = snap_down(Ticks(clip.end().0.saturating_add(ticks.0)), unit);
    let shortest = snap_down(clip.start, unit).0 + unit.0;
    Ticks(end.0.max(shortest) - clip.start.0)
}

/// The edit for a right-edge drag of `ticks` on clip `id`, or `None` when its length stays.
pub fn resized(project: &Project, id: ClipId, ticks: Ticks, unit: Ticks) -> Option<Edit> {
    let clip = project.playlist.clips.get(&id)?;
    let length = resized_length(clip, ticks, unit);
    (length != clip.length).then_some(Edit::ResizeClip { id, length })
}

/// Where an Alt drag of `ticks` on a clip's right edge puts its end: on the snap line nearest
/// the dragged end, at least a tick after the start.
pub fn stretched_end(clip: &Clip, ticks: Ticks, unit: Ticks) -> Ticks {
    let end = snap_nearest(Ticks(clip.end().0.saturating_add(ticks.0)), unit);
    Ticks(end.0.max(clip.start.0 + 1))
}

/// Why a stretch gesture changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StretchRefused {
    /// The file would be stretched beyond a quarter or four times its length.
    Ratio,
    /// The stretched file would be shorter than the `shortest` frames a time-stretch can make.
    TooShort { shortest: u64 },
}

/// `value · num / den` rounded half up, in exact integers; `None` past `u64::MAX` or for a
/// `den` of 0.
fn scaled_half_up(value: u64, num: u64, den: u64) -> Option<u64> {
    let den = u128::from(den);
    if den == 0 {
        return None;
    }
    // Why u128 with checks: two u64 factors always fit, and only their doubling can overflow,
    // which is refused rather than wrapped.
    let twice = u128::from(value)
        .checked_mul(u128::from(num))?
        .checked_mul(2)?;
    u64::try_from((twice + den) / (den * 2)).ok()
}

/// The edit an Alt drag of audio clip `id`'s right edge to `end` commits, stretch to fit: the
/// clip ends there and the audio it plays now fills it. In samples by the one tick rule, with
/// T_old the stretched length, or the file's `source_frames` when the clip is not stretched yet
/// (which makes it a time-stretch at its own pitch): the clip plays
/// played = min(len_old, T_old − offset) frames of it (a clip longer than its audio plays
/// silence after it; one trimmed shorter shows only a part), and
/// T_new = round-half-up(T_old · len_new / played), the frame offset scaled the same way, so
/// those frames take len_new: the silence after the audio goes, a part trimmed to keeps its
/// window. `Ok(None)` when the length stays or the clip plays none of its audio. A ratio past a
/// quarter or four times the file, or a time-stretch shorter than `shortest` frames, is refused.
/// Why fit and not the clip's length scaled: a clip dragged longer first (its edge trims, so
/// that adds silence) kept that silence in every stretch after.
pub fn stretched(
    project: &Project,
    id: ClipId,
    end: Ticks,
    source_frames: u64,
    shortest: u64,
) -> Result<Option<Edit>, StretchRefused> {
    let Some(clip) = project.playlist.clips.get(&id) else {
        return Ok(None);
    };
    let ClipItem::Audio {
        frame_offset,
        stretch,
        ..
    } = clip.item
    else {
        return Ok(None);
    };
    let sample = |tick: Ticks| {
        time::tick_to_sample(
            Ticks(tick.0.max(0)),
            project.sample_rate,
            project.tempo,
            project.ppq,
        )
    };
    let lengths = sample(clip.start).and_then(|start| {
        let old = sample(clip.end())?.checked_sub(start)?;
        let new = sample(end)?.checked_sub(start)?;
        Some((old, new))
    });
    let Some((old, new)) = lengths else {
        return Ok(None);
    };
    if old == new || end <= clip.start {
        return Ok(None);
    }
    let from = stretch.map_or(source_frames, |stretch| stretch.frames);
    let played = old.min(from.saturating_sub(frame_offset));
    if played == 0 {
        return Ok(None);
    }
    let (Some(frames), Some(offset)) = (
        scaled_half_up(from, new, played),
        scaled_half_up(frame_offset, new, played),
    ) else {
        return Err(StretchRefused::Ratio);
    };
    let stretch = ClipStretch {
        mode: stretch.map_or(StretchMode::Stretch, |stretch| stretch.mode),
        frames,
        pitch: stretch.map_or(0, |stretch| stretch.pitch),
    };
    if frames == 0 || !stretch.fits(source_frames) {
        return Err(StretchRefused::Ratio);
    }
    if stretch.mode == StretchMode::Stretch && frames < shortest {
        return Err(StretchRefused::TooShort { shortest });
    }
    Ok(Some(Edit::SetClipStretch {
        clip: id,
        stretch: Some(stretch),
        frame_offset: offset,
        length: Ticks(end.0 - clip.start.0),
    }))
}

/// The properties strip's Stretch choice: audio clip `id` played as its file (`None`),
/// resampled or time-stretched, its stretched length kept (a clip not stretched yet starts at
/// its file's `source_frames`). Ending a stretch puts the frame offset back in the file's
/// frames. Resampling has no pitch of its own. `Ok(None)` when the mode stays; a time-stretch
/// shorter than `shortest` frames is refused.
pub fn with_mode(
    project: &Project,
    id: ClipId,
    mode: Option<StretchMode>,
    source_frames: u64,
    shortest: u64,
) -> Result<Option<Edit>, StretchRefused> {
    let Some((clip, frame_offset, stretch)) = audio_clip(project, id) else {
        return Ok(None);
    };
    if stretch.map(|stretch| stretch.mode) == mode {
        return Ok(None);
    }
    let edit = |stretch, frame_offset| {
        Ok(Some(Edit::SetClipStretch {
            clip: id,
            stretch,
            frame_offset,
            length: clip.length,
        }))
    };
    let Some(mode) = mode else {
        let frames = stretch.map_or(source_frames, |stretch| stretch.frames);
        let offset =
            scaled_half_up(frame_offset, source_frames, frames).ok_or(StretchRefused::Ratio)?;
        return edit(None, offset);
    };
    let new = ClipStretch {
        mode,
        frames: stretch.map_or(source_frames, |stretch| stretch.frames),
        pitch: match mode {
            StretchMode::Stretch => stretch.map_or(0, |stretch| stretch.pitch),
            StretchMode::Resample => 0,
        },
    };
    if !new.fits(source_frames) {
        return Err(StretchRefused::Ratio);
    }
    if mode == StretchMode::Stretch && new.frames < shortest {
        return Err(StretchRefused::TooShort { shortest });
    }
    edit(Some(new), frame_offset)
}

/// The properties strip's pitch stepper: a time-stretched clip `semitones` higher, inside
/// [`STRETCH_PITCH_RANGE`]; `None` for a clip that is not time-stretched or a pitch that stays.
pub fn with_pitch(project: &Project, id: ClipId, semitones: i8) -> Option<Edit> {
    let (clip, frame_offset, stretch) = audio_clip(project, id)?;
    let stretch = stretch.filter(|stretch| stretch.mode == StretchMode::Stretch)?;
    let pitch = stretch
        .pitch
        .saturating_add(semitones)
        .clamp(*STRETCH_PITCH_RANGE.start(), *STRETCH_PITCH_RANGE.end());
    (pitch != stretch.pitch).then_some(Edit::SetClipStretch {
        clip: id,
        stretch: Some(ClipStretch { pitch, ..stretch }),
        frame_offset,
        length: clip.length,
    })
}

/// Audio clip `id`, its frame offset and its stretch.
fn audio_clip(project: &Project, id: ClipId) -> Option<(&Clip, u64, Option<ClipStretch>)> {
    let clip = project.playlist.clips.get(&id)?;
    match clip.item {
        ClipItem::Audio {
            frame_offset,
            stretch,
            ..
        } => Some((clip, frame_offset, stretch)),
        ClipItem::Pattern { .. } | ClipItem::Automation { .. } => None,
    }
}

/// Deletes `ids`, or `None` when there are none.
pub fn deleted(ids: &BTreeSet<ClipId>) -> Option<Edit> {
    (!ids.is_empty()).then(|| Edit::DeleteClips {
        ids: ids.iter().copied().collect(),
    })
}

/// Copies `ids` one selection length later, and the IDs the copies will have.
pub fn duplicated(project: &Project, ids: &BTreeSet<ClipId>) -> Option<(Edit, BTreeSet<ClipId>)> {
    if ids.is_empty() {
        return None;
    }
    let first = project.playlist.next_clip_id;
    let count = u32::try_from(ids.len()).ok()?;
    let copies = (first.0..first.0.checked_add(count)?).map(ClipId).collect();
    let edit = Edit::DuplicateClips {
        ids: ids.iter().copied().collect(),
        first,
    };
    Some((edit, copies))
}

/// A loop range drawn from tick `press` to tick `cursor`: every grid cell from the one pressed
/// to the one under the cursor, switched on, and at least `step` long so a pass is never empty.
pub fn drawn_loop(press: Ticks, cursor: Ticks, unit: Ticks, step: Ticks) -> LoopRange {
    let unit = Ticks(unit.0.max(1));
    let start = snap_down(Ticks(press.0.min(cursor.0)), unit).0.max(0);
    let end = snap_down(Ticks(press.0.max(cursor.0)), unit).0 + unit.0;
    LoopRange {
        start: Ticks(start),
        end: Ticks(end.max(start + step.0)),
        on: true,
    }
}

/// `range` with `edge` moved to the grid line nearest tick `tick`, never closer than `step` to
/// the other edge nor before tick 0. The range stays on or off as it was.
pub fn moved_loop_edge(
    range: LoopRange,
    edge: LoopEdge,
    tick: Ticks,
    unit: Ticks,
    step: Ticks,
) -> LoopRange {
    let at = snap_nearest(tick, unit).0.max(0);
    match edge {
        LoopEdge::Start => LoopRange {
            start: Ticks(at.min(range.end.0 - step.0).max(0)),
            ..range
        },
        LoopEdge::End => LoopRange {
            end: Ticks(at.max(range.start.0 + step.0)),
            ..range
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P4_TWO_CLIPS: &str = include_str!("../../../../tests/fixtures/p4_two_clips.json");
    const P4_MESSY: &str = include_str!("../../../../tests/fixtures/p4_messy.json");
    const P9_STRETCH_UI: &str = include_str!("../../../../tests/fixtures/p9_stretch_ui.json");

    /// p9_tone440.wav is one second at 44.1 kHz.
    const TONE: u64 = 44_100;
    /// Twice the stretcher's latency at 44.1 kHz, mono.
    const SHORTEST: u64 = 2646;

    // Stretch to fit, by hand at 120 BPM, 44.1 kHz: s(t) = floor(t · 229.6875), so a bar is
    // 88200 samples and the clips' four bars 352800; both clips are longer than their audio.
    // Clip 2 (unstretched: N = the file's 44100 frames, then silence) dragged to 288 (L = 66150
    // samples): ratio L/N = 1.5, T = 44100 · 66150 / 44100 = 66150, p9_stretch's own stretch
    // (whose render is gen_p9's 440 Hz tone over 66150 frames); to 768 (176400): T = 176400,
    // four times, the most. Clip 1 (a stretch to 66150, then silence) to 576 (132300): T =
    // 66150 · 132300 / 66150 = 132300. A clip trimmed to the file's first half (clip 2 at 96
    // ticks, 22050 samples) dragged to 192 (44100): that half fills it, T = 44100 · 44100 /
    // 22050 = 88200. From an offset of 11025 (33075 frames played) to 288: T = 44100 · 66150 /
    // 33075 = 88200, the offset 11025 · 66150 / 33075 = 22050, and 88200 − 22050 = 66150.
    #[test]
    fn an_alt_drag_stretches_the_clips_audio_to_fill_its_new_length() {
        let mut project = load(P9_STRETCH_UI);
        let stretch = |frames| {
            Some(ClipStretch {
                mode: StretchMode::Stretch,
                frames,
                pitch: 0,
            })
        };
        let set = |clip, frames, frame_offset, length| {
            Ok(Some(Edit::SetClipStretch {
                clip: ClipId(clip),
                stretch: stretch(frames),
                frame_offset,
                length: Ticks(length),
            }))
        };
        let p9: serde_json::Value =
            serde_json::from_str(include_str!("../../../../tests/fixtures/p9_expected.json"))
                .expect("p9_expected.json parses");
        assert_eq!(p9["stretch"]["stretched"], 66_150);
        assert_eq!(
            stretched(&project, ClipId(2), Ticks(288), TONE, SHORTEST),
            set(2, 66_150, 0, 288)
        );
        assert_eq!(
            stretched(&project, ClipId(2), Ticks(768), TONE, SHORTEST),
            set(2, 176_400, 0, 768)
        );
        assert_eq!(
            stretched(&project, ClipId(1), Ticks(576), TONE, SHORTEST),
            set(1, 132_300, 0, 576)
        );
        assert_eq!(
            stretched(&project, ClipId(2), Ticks(1536), TONE, SHORTEST),
            Ok(None),
            "the same length"
        );
        let clip = project.playlist.clips.get_mut(&ClipId(2)).expect("clip 2");
        clip.length = Ticks(96);
        assert_eq!(
            stretched(&project, ClipId(2), Ticks(192), TONE, SHORTEST),
            set(2, 88_200, 0, 192),
            "a part trimmed to keeps its window"
        );
        let clip = project.playlist.clips.get_mut(&ClipId(2)).expect("clip 2");
        clip.length = Ticks(1536);
        if let ClipItem::Audio { frame_offset, .. } = &mut clip.item {
            *frame_offset = 11_025;
        }
        assert_eq!(
            stretched(&project, ClipId(2), Ticks(288), TONE, SHORTEST),
            set(2, 88_200, 22_050, 288)
        );
        if let Some(ClipItem::Audio { frame_offset, .. }) = project
            .playlist
            .clips
            .get_mut(&ClipId(2))
            .map(|clip| &mut clip.item)
        {
            *frame_offset = 44_100;
        }
        assert_eq!(
            stretched(&project, ClipId(2), Ticks(288), TONE, SHORTEST),
            Ok(None),
            "it plays none of its audio"
        );
    }

    // Both lengths scale half up: from an offset of 4100 (40000 frames played) to a bar (88200):
    // T = 44100 · 88200 / 40000 = 97240.5 → 97241 and the offset 4100 · 88200 / 40000 = 9040.5
    // → 9041. Past four times the file (to 20 bars: 1764000, T = 1944810 of 44100) is refused,
    // as is a time-stretch below the shortest (a 4000-frame file to 11 ticks, 2526 samples:
    // T = 2526 < 2646). Clip 1 to 3072 (705600 samples) would be sixteen times its file:
    // refused. An end lands on the nearest bar line.
    // The strip's choices by hand: clip 1 (a time-stretch to 66150) resampled keeps its
    // length and loses its pitch; its stretch ended puts an offset of 1500 stretched frames
    // back at 1500 · 44100 / 66150 = 1000 file frames; clip 2 time-stretched starts at its
    // file's 44100. A 2000-frame file is too short for the stretcher (2646).
    #[test]
    fn the_strip_switches_modes_keeping_the_stretched_length() {
        let mut project = load(P9_STRETCH_UI);
        let clip_1 = project.playlist.clips.get_mut(&ClipId(1)).expect("clip 1");
        if let ClipItem::Audio {
            frame_offset,
            stretch: Some(stretch),
            ..
        } = &mut clip_1.item
        {
            *frame_offset = 1500;
            stretch.pitch = 7;
        }
        let set = |clip, stretch, frame_offset| {
            Ok(Some(Edit::SetClipStretch {
                clip: ClipId(clip),
                stretch,
                frame_offset,
                length: Ticks(1536),
            }))
        };
        let of = |mode, frames, pitch| {
            Some(ClipStretch {
                mode,
                frames,
                pitch,
            })
        };
        let mode =
            |project: &Project, clip, mode| with_mode(project, ClipId(clip), mode, TONE, SHORTEST);
        assert_eq!(
            mode(&project, 1, Some(StretchMode::Resample)),
            set(1, of(StretchMode::Resample, 66_150, 0), 1500)
        );
        assert_eq!(mode(&project, 1, None), set(1, None, 1000));
        assert_eq!(mode(&project, 1, Some(StretchMode::Stretch)), Ok(None));
        assert_eq!(
            mode(&project, 2, Some(StretchMode::Stretch)),
            set(2, of(StretchMode::Stretch, 44_100, 0), 0)
        );
        assert_eq!(mode(&project, 2, None), Ok(None));
        assert_eq!(
            with_mode(
                &project,
                ClipId(2),
                Some(StretchMode::Stretch),
                2000,
                SHORTEST
            ),
            Err(StretchRefused::TooShort { shortest: SHORTEST })
        );
    }

    // Time-stretched clips step their pitch inside ±24 semitones; other clips have none.
    #[test]
    fn the_pitch_steps_only_a_time_stretch_and_stays_in_range() {
        let mut project = load(P9_STRETCH_UI);
        let Some(Edit::SetClipStretch {
            stretch: Some(stretch),
            ..
        }) = with_pitch(&project, ClipId(1), 1)
        else {
            panic!("a pitch step");
        };
        assert_eq!(stretch.pitch, 1);
        assert_eq!(with_pitch(&project, ClipId(2), 1), None, "not stretched");
        if let Some(clip) = project.playlist.clips.get_mut(&ClipId(1)) {
            if let ClipItem::Audio {
                stretch: Some(stretch),
                ..
            } = &mut clip.item
            {
                stretch.pitch = 24;
            }
        }
        assert_eq!(with_pitch(&project, ClipId(1), 1), None, "at the top");
        let Some(Edit::SetClipStretch {
            stretch: Some(stretch),
            ..
        }) = with_pitch(&project, ClipId(1), -1)
        else {
            panic!("a pitch step");
        };
        assert_eq!(stretch.pitch, 23);
    }

    #[test]
    fn a_stretch_rounds_half_up_and_refuses_what_it_cannot_make() {
        let mut project = load(P9_STRETCH_UI);
        let clip = project.playlist.clips.get_mut(&ClipId(2)).expect("clip 2");
        if let ClipItem::Audio { frame_offset, .. } = &mut clip.item {
            *frame_offset = 4100;
        }
        let Ok(Some(Edit::SetClipStretch {
            stretch: Some(stretch),
            frame_offset,
            ..
        })) = stretched(&project, ClipId(2), Ticks(384), TONE, SHORTEST)
        else {
            panic!("a stretch");
        };
        assert_eq!((stretch.frames, frame_offset), (97_241, 9041));
        assert_eq!(
            stretched(&project, ClipId(2), Ticks(7680), TONE, SHORTEST),
            Err(StretchRefused::Ratio)
        );
        assert_eq!(
            stretched(&project, ClipId(1), Ticks(3072), TONE, SHORTEST),
            Err(StretchRefused::Ratio)
        );
        let clip = project.playlist.clips.get_mut(&ClipId(2)).expect("clip 2");
        if let ClipItem::Audio { frame_offset, .. } = &mut clip.item {
            *frame_offset = 0;
        }
        assert_eq!(
            stretched(&project, ClipId(2), Ticks(11), 4000, SHORTEST),
            Err(StretchRefused::TooShort { shortest: SHORTEST })
        );
        let clip = project.playlist.clips[&ClipId(2)].clone();
        assert_eq!(
            stretched_end(&clip, Ticks(-2000), BAR),
            Ticks(1),
            "a tick at least"
        );
        assert_eq!(stretched_end(&clip, Ticks(200), BAR), Ticks(1920));
        assert_eq!(stretched_end(&clip, Ticks(150), BAR), Ticks(1536));
    }

    fn load(text: &str) -> Project {
        Project::load_json(text).expect("fixture loads")
    }

    const BAR: Ticks = Ticks(384);

    #[test]
    fn snap_units_follow_the_ppq() {
        let units: Vec<i64> = ClipSnap::ALL.iter().map(|snap| snap.unit(96).0).collect();
        assert_eq!(units, [384, 96, 24, 1]);
        assert_eq!(ClipSnap::default(), ClipSnap::Bar);
        assert_eq!(ClipSnap::Step.to_string(), "Step");
    }

    // p4_two_clips has clips 1 and 2 (next id 3). Pattern 2 at tick 1300 on track index 1
    // snaps down to bar 4 (1152) on track 2, one bar long.
    #[test]
    fn a_placed_clip_starts_on_the_grid_below_the_cursor() {
        let project = load(P4_TWO_CLIPS);
        let placed = place(&project, PatternId(2), 1, Ticks(1300), BAR);
        let clip = Clip::of_pattern(PatternId(2), TrackId(2), Ticks(1152), BAR);
        assert_eq!(
            placed,
            Some(Edit::PlaceClips {
                first: ClipId(3),
                clips: vec![clip]
            })
        );
        assert_eq!(place(&project, PatternId(9), 1, Ticks(0), BAR), None);
        assert_eq!(
            place(&project, PatternId(1), 8, Ticks(0), BAR),
            None,
            "no 9th track"
        );
        assert_eq!(place(&project, PatternId(1), 0, Ticks(-5), BAR), None);
    }

    // Clip 1 (0..384, track index 0) and clip 2 (768.., index 0). A drag of 500 ticks rounds to
    // one bar; of 200 to none (nearest); left of tick 0 the group stops at 0; rows stay on the
    // 8 tracks.
    #[test]
    fn a_move_rounds_to_whole_units_and_stays_on_the_playlist() {
        let project = load(P4_TWO_CLIPS);
        let both = [ClipId(1), ClipId(2)];
        assert_eq!(
            move_targets(&project, &both, Ticks(500), 1, BAR),
            [(ClipId(1), Ticks(384), 1), (ClipId(2), Ticks(1152), 1)]
        );
        assert_eq!(
            move_targets(&project, &both, Ticks(200), 0, BAR),
            [(ClipId(1), Ticks(384), 0), (ClipId(2), Ticks(1152), 0)]
        );
        assert_eq!(
            move_targets(&project, &both, Ticks(-1000), -3, BAR),
            [(ClipId(1), Ticks(0), 0), (ClipId(2), Ticks(768), 0)]
        );
        assert_eq!(
            move_targets(&project, &[ClipId(2)], Ticks(0), 20, BAR),
            [(ClipId(2), Ticks(768), 7)]
        );
        let targets = move_targets(&project, &[ClipId(1)], Ticks(150), 0, BAR);
        assert_eq!(
            moved(&project, &targets),
            None,
            "150 ticks rounds to no move"
        );
        let targets = move_targets(&project, &[ClipId(1)], Ticks(0), 2, BAR);
        assert_eq!(
            moved(&project, &targets),
            Some(Edit::MoveClips {
                moves: vec![(ClipId(1), Ticks(0), TrackId(3))]
            })
        );
    }

    // p4_messy's clip 1 starts off the grid at 37: a bar move keeps it 37 past the line, and
    // the furthest left it can go is to 37 (0 whole bars back).
    #[test]
    fn an_off_grid_clip_moves_by_whole_units() {
        let project = load(P4_MESSY);
        assert_eq!(
            move_targets(&project, &[ClipId(1)], Ticks(400), 0, BAR),
            [(ClipId(1), Ticks(421), 0)]
        );
        assert_eq!(
            move_targets(&project, &[ClipId(1)], Ticks(-400), 0, BAR),
            [(ClipId(1), Ticks(37), 0)]
        );
        let step = ClipSnap::Step.unit(96);
        assert_eq!(
            move_targets(&project, &[ClipId(1)], Ticks(-30), 0, step),
            [(ClipId(1), Ticks(13), 0)],
            "-30 rounds to one step back"
        );
    }

    // A right-edge drag floors the new end to the grid, at least one grid line past the start's
    // cell: clip 1 (0..384) dragged 768 ticks ends at 1152; 300 ticks floors back to 384 (no
    // change); far left keeps one bar. p4_messy's clip 1 (37..287) dragged 100 ticks floors
    // 387 to 384 (length 347), and dragged far left still ends on 384.
    #[test]
    fn a_resize_floors_the_end_to_the_grid() {
        let project = load(P4_TWO_CLIPS);
        assert_eq!(
            resized(&project, ClipId(1), Ticks(768), BAR),
            Some(Edit::ResizeClip {
                id: ClipId(1),
                length: Ticks(1152)
            })
        );
        assert_eq!(resized(&project, ClipId(1), Ticks(300), BAR), None);
        let clip = &project.playlist.clips[&ClipId(1)];
        assert_eq!(resized_length(clip, Ticks(-5000), BAR), BAR);
        let messy = load(P4_MESSY);
        let off_grid = &messy.playlist.clips[&ClipId(1)];
        assert_eq!(resized_length(off_grid, Ticks(100), BAR), Ticks(347));
        assert_eq!(
            resized_length(off_grid, Ticks(0), BAR),
            Ticks(250),
            "a press alone"
        );
        assert_eq!(
            resized_length(off_grid, Ticks(-100), BAR),
            Ticks(347),
            "ends on bar 2"
        );
    }

    #[test]
    fn delete_and_duplicate_name_the_selection_and_the_new_ids() {
        let project = load(P4_TWO_CLIPS);
        let both = BTreeSet::from([ClipId(2), ClipId(1)]);
        assert_eq!(
            deleted(&both),
            Some(Edit::DeleteClips {
                ids: vec![ClipId(1), ClipId(2)]
            })
        );
        assert_eq!(deleted(&BTreeSet::new()), None);
        let (edit, copies) = duplicated(&project, &both).expect("a selection");
        assert_eq!(
            edit,
            Edit::DuplicateClips {
                ids: vec![ClipId(1), ClipId(2)],
                first: ClipId(3)
            }
        );
        assert_eq!(copies, BTreeSet::from([ClipId(3), ClipId(4)]));
        assert_eq!(duplicated(&project, &BTreeSet::new()), None);
    }

    // A drawn range covers the cells from the one pressed to the one under the cursor, either
    // way round; with no snap it is at least a step long.
    #[test]
    fn a_drawn_loop_covers_the_cells_between_press_and_cursor() {
        let step = Ticks(24);
        let range = |start, end| LoopRange {
            start: Ticks(start),
            end: Ticks(end),
            on: true,
        };
        assert_eq!(
            drawn_loop(Ticks(444), Ticks(828), BAR, step),
            range(384, 1152)
        );
        assert_eq!(
            drawn_loop(Ticks(828), Ticks(444), BAR, step),
            range(384, 1152)
        );
        assert_eq!(
            drawn_loop(Ticks(444), Ticks(444), BAR, step),
            range(384, 768)
        );
        assert_eq!(
            drawn_loop(Ticks(100), Ticks(105), Ticks(1), step),
            range(100, 124)
        );
        assert_eq!(drawn_loop(Ticks(-50), Ticks(10), BAR, step), range(0, 384));
    }

    // Loop 384..768: the end dragged to tick 1500 lands on the nearest bar line, 1536; the start
    // dragged past the end stops a step before it; the on/off state is kept.
    #[test]
    fn a_loop_edge_moves_to_the_nearest_line_and_keeps_a_step() {
        let range = LoopRange {
            start: Ticks(384),
            end: Ticks(768),
            on: false,
        };
        let step = Ticks(24);
        let end = moved_loop_edge(range, LoopEdge::End, Ticks(1500), BAR, step);
        assert_eq!(
            (end.start, end.end, end.on),
            (Ticks(384), Ticks(1536), false)
        );
        let start = moved_loop_edge(range, LoopEdge::Start, Ticks(2000), BAR, step);
        assert_eq!((start.start, start.end), (Ticks(744), Ticks(768)));
        let end = moved_loop_edge(range, LoopEdge::End, Ticks(0), BAR, step);
        assert_eq!(end.end, Ticks(408));
        let start = moved_loop_edge(range, LoopEdge::Start, Ticks(-900), BAR, step);
        assert_eq!(start.start, Ticks(0));
    }
}
