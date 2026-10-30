//! What the playlist shows of audio and automation clips besides the document: the waveform of
//! each audio channel's file (or that it is still loading, or missing), an automation's name,
//! and the curve its points draw.

use dawsome_engine::processor::Registry;
use std::collections::BTreeMap;
use std::path::PathBuf;

use dawsome_engine::project::{
    Automation, AutomationTarget, Channel, Clip, ClipStretch, InsertId, ProcessorOwner, Project,
    SoundEdits, StretchMode,
};
use dawsome_engine::time::Ticks;
use dawsome_media::Peaks;

/// The audio channels' sounds as their clips draw them: by the file the project names, then by
/// the edit list it plays through (none for the file as it is), so a clip draws what it plays.
pub type FileLooks = BTreeMap<PathBuf, BTreeMap<SoundEdits, FileLook>>;

/// The look of the sound `channel` plays: its file through its edits.
pub fn look_of<'a>(looks: &'a FileLooks, channel: &Channel) -> Option<&'a FileLook> {
    looks.get(&channel.sample)?.get(&channel.edits)
}

/// An audio channel's sound (its file through its edits) as its clips draw it.
#[derive(Debug, Clone, PartialEq)]
pub enum FileLook {
    /// Decoding (or converting) still.
    Loading,
    /// It cannot be read.
    Missing,
    Ready(Peaks),
}

/// What an automation sets, as its clips are titled: "kick · Volume", "Insert 2 · Pan",
/// "Insert 1 → Master · Send", "Delay · Mix", or the plugin's own parameter name; `None` when
/// the target is gone.
pub fn automation_name(
    project: &Project,
    registry: &Registry,
    target: &AutomationTarget,
) -> Option<String> {
    let channel = |id| project.channels.get(id).map(|channel| channel.name.clone());
    let insert = |id: InsertId| project.mixer.get(id).map(|insert| insert.name.clone());
    Some(match target {
        AutomationTarget::ChannelVolume(id) => format!("{} · Volume", channel(id)?),
        AutomationTarget::ChannelPan(id) => format!("{} · Pan", channel(id)?),
        AutomationTarget::InsertFader(id) => format!("{} · Fader", insert(*id)?),
        AutomationTarget::InsertPan(id) => format!("{} · Pan", insert(*id)?),
        AutomationTarget::Send { from, to } => {
            project.mixer.get(*from)?.routes.get(to)?;
            format!("{} → {} · Send", insert(*from)?, insert(*to)?)
        }
        AutomationTarget::EffectParam { effect, key } => {
            let (_, placed) = project.mixer.effect(*effect)?;
            let factory = registry.get(&placed.processor.id)?;
            let info = factory.params().iter().find(|info| info.key == key)?;
            format!("{} · {}", factory.name(), info.label)
        }
        AutomationTarget::EffectPlugin { effect, param_id } => {
            let (_, placed) = project.mixer.effect(*effect)?;
            plugin_param_name(
                registry,
                ProcessorOwner::Effect(*effect),
                &placed.processor.id,
                *param_id,
                placed.processor.display_name(),
            )?
        }
        AutomationTarget::ChannelParam {
            channel: id,
            param_id,
        } => {
            let processor = project.channels.get(id)?.processor.as_ref()?;
            let owner = ProcessorOwner::Channel(*id);
            plugin_param_name(registry, owner, &processor.id, *param_id, &channel(id)?)?
        }
    })
}

fn plugin_param_name(
    registry: &Registry,
    owner: ProcessorOwner,
    id: &str,
    param_id: u32,
    what: &str,
) -> Option<String> {
    let host = registry.host(id)?;
    let param = host
        .params(owner)
        .into_iter()
        .find(|param| param.id == param_id)?;
    Some(format!("{what} · {}", param.name))
}

/// The words an audio clip's title adds after its file's name: " · stretching…" while its
/// stretched sound is being made, else " · Stretch 150% +7 st" or " · Resample 200% −12 st",
/// the stretched length over the file's `file_frames` and (resampling) the pitch that length
/// gives, when the file is loaded; none for a clip that is not stretched.
pub fn stretch_words(
    stretch: Option<ClipStretch>,
    file_frames: Option<u64>,
    stretching: bool,
) -> String {
    let Some(stretch) = stretch else {
        return String::new();
    };
    if stretching {
        return " \u{b7} stretching\u{2026}".to_owned();
    }
    let mode = match stretch.mode {
        StretchMode::Stretch => "Stretch",
        StretchMode::Resample => "Resample",
    };
    let mut words = format!(" \u{b7} {mode}");
    let source = file_frames.filter(|&frames| frames > 0);
    if let Some(source) = source {
        words.push_str(&format!(" {}%", stretch_percent(stretch.frames, source)));
    }
    let tenths = match stretch.mode {
        StretchMode::Stretch => i64::from(stretch.pitch) * 10,
        StretchMode::Resample => source.map_or(0, |source| resample_shift(stretch.frames, source)),
    };
    words.push_str(&shift_words(tenths));
    words
}

/// The pitch `source` frames resampled to `frames` are heard at, in tenths of a semitone,
/// rounded: −12·log2(frames / source), so twice as long is an octave down.
fn resample_shift(frames: u64, source: u64) -> i64 {
    let ratio = frames as f64 / source.max(1) as f64;
    (-120.0 * ratio.log2()).round() as i64
}

/// A pitch shift in tenths of a semitone as the title writes it: " +7 st", " −3.9 st"; nothing
/// for none.
fn shift_words(tenths: i64) -> String {
    let sign = match tenths.signum() {
        0 => return String::new(),
        1 => "+",
        _ => "\u{2212}",
    };
    let (whole, tenth) = (tenths.unsigned_abs() / 10, tenths.unsigned_abs() % 10);
    if tenth == 0 {
        format!(" {sign}{whole} st")
    } else {
        format!(" {sign}{whole}.{tenth} st")
    }
}

/// `frames` as a share of `source` frames, in whole percent rounded half up: 66150 of 44100
/// is 150. A `source` of 0 counts as one frame.
pub fn stretch_percent(frames: u64, source: u64) -> u128 {
    let source = u128::from(source.max(1));
    (u128::from(frames) * 200 + source) / (2 * source)
}

/// A gain as the clip's readout writes it: "+3.5 dB", "−6.0 dB", "−inf dB" for silence.
pub fn gain_text(gain: f32) -> String {
    if gain <= 0.0 {
        return "\u{2212}inf dB".to_owned();
    }
    let db = 20.0 * f64::from(gain).log10();
    let rounded = (db * 10.0).round() / 10.0;
    if rounded < 0.0 {
        format!("\u{2212}{:.1} dB", -rounded)
    } else {
        format!("+{rounded:.1} dB")
    }
}

/// The gain `db` decibels from `gain`, kept inside the clip gain range.
pub fn gain_moved(gain: f32, db: f32) -> f32 {
    let from = if gain > 0.0 {
        20.0 * f64::from(gain).log10()
    } else {
        -96.0
    };
    let moved = 10f64.powf((from + f64::from(db)) / 20.0) as f32;
    moved.clamp(0.0, *dawsome_engine::project::CLIP_GAIN_RANGE.end())
}

/// The normalized value `automation` shows at song tick `tick` in `clip` (one of its clips): the
/// curve of the segment the tick falls in, the first point's value before it, the last one's
/// after.
pub fn value_at(automation: &Automation, clip: &Clip, tick: Ticks) -> f32 {
    let data = tick.0 - clip.start.0 + clip.offset.0;
    let points = &automation.points;
    let Some(first) = points.first() else {
        return 0.0;
    };
    let after = points.partition_point(|point| point.tick.0 <= data);
    let Some(index) = after.checked_sub(1) else {
        return first.value;
    };
    let Some(next) = points.get(index + 1) else {
        return points[index].value;
    };
    let point = &points[index];
    let span = (next.tick.0 - point.tick.0) as f64;
    let x = (data - point.tick.0) as f64 / span;
    let (v0, v1) = (f64::from(point.value), f64::from(next.value));
    (v0 + (v1 - v0) * point.curve.shape(x)) as f32
}

/// The tension handle's tension for the cursor at fraction `fraction` of the way between a
/// segment's two values: k = ln(1/r − 1)/3, so the curve's midpoint lands on the cursor,
/// kept within −1..1 and rounded to 0.01.
pub fn tension_for(fraction: f32) -> f32 {
    let r = f64::from(fraction).clamp(1e-6, 1.0 - 1e-6);
    let k = ((1.0 / r - 1.0).ln() / 3.0).clamp(-1.0, 1.0);
    ((k * 100.0).round() / 100.0) as f32
}

#[cfg(test)]
mod tests {
    use dawsome_engine::project::{AutomationPoint, Curve};

    use super::*;

    #[test]
    fn a_stretched_clip_names_its_mode_ratio_and_pitch() {
        let stretch = |mode, frames, pitch| {
            Some(ClipStretch {
                mode,
                frames,
                pitch,
            })
        };
        let file = Some(44_100);
        assert_eq!(stretch_words(None, file, false), "");
        assert_eq!(
            stretch_words(stretch(StretchMode::Stretch, 66_150, 7), file, false),
            " \u{b7} Stretch 150% +7 st"
        );
        // Resampling shifts the pitch by −12·log2(ratio), shown to a tenth of a semitone:
        // twice as long is an octave down, half as long an octave up (python: 125% is
        // −3.863 st, 75% +4.980, 33.3% +19.020).
        assert_eq!(
            stretch_words(stretch(StretchMode::Resample, 88_200, 0), file, false),
            " \u{b7} Resample 200% \u{2212}12 st"
        );
        assert_eq!(
            stretch_words(stretch(StretchMode::Resample, 22_050, 0), file, false),
            " \u{b7} Resample 50% +12 st"
        );
        assert_eq!(
            stretch_words(stretch(StretchMode::Resample, 55_125, 0), file, false),
            " \u{b7} Resample 125% \u{2212}3.9 st"
        );
        assert_eq!(
            stretch_words(stretch(StretchMode::Resample, 44_100, 0), file, false),
            " \u{b7} Resample 100%",
            "no shift at the file's own length"
        );
        assert_eq!(
            stretch_words(stretch(StretchMode::Resample, 88_200, 0), None, false),
            " \u{b7} Resample",
            "no ratio and no shift before the file is loaded"
        );
        assert_eq!(
            stretch_words(stretch(StretchMode::Stretch, 44_100, -5), None, false),
            " \u{b7} Stretch \u{2212}5 st",
            "no ratio before the file is loaded"
        );
        assert_eq!(
            stretch_words(stretch(StretchMode::Stretch, 66_150, 7), file, true),
            " \u{b7} stretching\u{2026}"
        );
        // 33075 of 44100 is 75%; 100 of 300 is 33.3%, shown 33%.
        assert_eq!(
            stretch_words(stretch(StretchMode::Resample, 33_075, 0), file, false),
            " \u{b7} Resample 75% +5 st"
        );
        assert_eq!(
            stretch_words(stretch(StretchMode::Resample, 100, 0), Some(300), false),
            " \u{b7} Resample 33% +19 st"
        );
    }

    #[test]
    fn gains_read_in_decibels() {
        assert_eq!(gain_text(0.5), "\u{2212}6.0 dB");
        assert_eq!(gain_text(2.0), "+6.0 dB");
        assert_eq!(gain_text(1.0), "+0.0 dB");
        assert_eq!(gain_text(0.0), "\u{2212}inf dB");
        assert!((gain_moved(1.0, -6.0206) - 0.5).abs() < 1e-4);
        assert_eq!(gain_moved(3.9, 6.0), 4.0, "kept at the top of the range");
    }

    // The tension handle lands on the cursor: at k the curve's midpoint is 1/(e^(3k) + 1),
    // which tension_for inverts; the center is the straight line.
    #[test]
    fn the_tension_follows_the_cursor() {
        assert_eq!(tension_for(0.5), 0.0);
        let k = tension_for(0.25);
        let midpoint = Curve::Exp(k).shape(0.5);
        assert!((midpoint - 0.25).abs() < 0.01, "{k} gives {midpoint}");
        assert_eq!(tension_for(0.0), 1.0, "kept within the range");
        assert_eq!(tension_for(1.0), -1.0);
    }

    #[test]
    fn a_clip_shows_its_window_of_the_points() {
        let automation = Automation {
            target: AutomationTarget::ChannelVolume(dawsome_engine::project::ChannelId(1)),
            points: vec![
                AutomationPoint::linear(Ticks(0), 1.0),
                AutomationPoint::linear(Ticks(384), 0.0),
            ],
        };
        let mut clip = Clip::of_automation(
            dawsome_engine::project::AutomationId(1),
            dawsome_engine::project::TrackId(1),
            Ticks(0),
            Ticks(768),
        );
        assert_eq!(value_at(&automation, &clip, Ticks(192)), 0.5);
        assert_eq!(
            value_at(&automation, &clip, Ticks(600)),
            0.0,
            "holds past the last"
        );
        clip.offset = Ticks(192);
        assert_eq!(
            value_at(&automation, &clip, Ticks(0)),
            0.5,
            "from its offset"
        );
    }
}
