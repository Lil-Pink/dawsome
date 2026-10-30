//! A channel's sound settings: how its notes play its sound, beyond the sound itself. Tuning,
//! the envelope, how much velocity changes the level, a low-pass filter, for a sampler the
//! direction and the point it starts from, how far a device's pitch bend reaches, and whether a
//! note ends the ones before it (cut itself). The
//! defaults are how every channel played before they existed, and a channel at the defaults
//! writes none of them.

use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

use super::ChannelKind;

/// Whole semitones a channel is tuned by.
pub const SEMITONE_RANGE: RangeInclusive<i8> = -12..=12;
/// Cents a channel is tuned by, beside its semitones.
pub const CENT_RANGE: RangeInclusive<i8> = -100..=100;
/// How much a note's velocity changes its level: 0 not at all, 1 fully.
pub const VELOCITY_RANGE: RangeInclusive<f32> = 0.0..=1.0;
/// The attack's length, in seconds.
pub const ATTACK_RANGE: RangeInclusive<f32> = 0.0..=2.0;
/// The release's length, in seconds.
pub const RELEASE_RANGE: RangeInclusive<f32> = 0.0..=4.0;
/// Where in its sound a sampler starts: 0 at its first frame, 1 past its last.
pub const START_RANGE: RangeInclusive<f32> = 0.0..=1.0;
/// The low-pass filter's cutoff, in Hz.
pub const CUTOFF_RANGE: RangeInclusive<f32> = 20.0..=20_000.0;
/// The filter's resonance: 0 is a Q of 1/√2, the flattest, and 1 a Q of 10.
pub const RESONANCE_RANGE: RangeInclusive<f32> = 0.0..=1.0;
/// How many semitones a pitch bend wheel all the way up (or down) bends a live note.
pub const BEND_RANGE: RangeInclusive<u8> = 1..=24;
/// The bend range of a channel that sets none: a whole tone, MIDI's usual.
pub const DEFAULT_BEND_RANGE: u8 = 2;

/// What a note's end does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    /// The sound plays on to its end; a release longer than 0 fades it from the note's end.
    OneShot,
    /// The note's end releases the sound (with no release, ends it).
    Hold,
}

impl Gate {
    /// How `kind` played before sound settings: a sampler (and an audio channel's notes) to the
    /// sound's end, an instrument set (and a plugin) until the note ends. An input channel plays
    /// no notes; its gate is a set's, which nothing reads.
    pub fn of(kind: ChannelKind) -> Self {
        match kind {
            ChannelKind::Sampler | ChannelKind::Audio => Gate::OneShot,
            ChannelKind::Instrument | ChannelKind::Plugin | ChannelKind::Input => Gate::Hold,
        }
    }
}

/// A channel's sound settings. Field order is the file's.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Sound {
    /// What a note's end does; none is the channel kind's own ([`Gate::of`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate: Option<Gate>,
    pub semitones: i8,
    pub cents: i8,
    /// How much velocity changes the level: the level is 1 − velocity·(1 − v) for a note of
    /// velocity v (an instrument set's own velocity curve gives v).
    pub velocity: f32,
    /// Seconds the level rises over from 0; 0 keeps the sound's own attack.
    pub attack: f32,
    /// Seconds the level falls over after the note's end; 0 keeps the sound's own release.
    pub release: f32,
    /// A sampler plays its sound backwards.
    pub reverse: bool,
    /// Where a sampler starts in its sound.
    pub start: f32,
    /// Whether the low-pass filter is on.
    pub filter: bool,
    pub cutoff: f32,
    pub resonance: f32,
    /// Semitones a device's pitch bend reaches all the way up or down, on the channel's live
    /// notes. Written only when not the default (2), so a file at the default keeps its bytes
    /// and its format version.
    #[serde(skip_serializing_if = "is_default_bend_range")]
    pub bend_range: u8,
    /// A note's start ends the notes the channel is still playing (often called "cut itself"):
    /// each falls over the release its note's end would take from that frame, so a fast 808
    /// line plays one note at a time. Written last and only when on, so a file without it keeps
    /// its bytes and its format version, and a build without it refuses a file with it by name.
    #[serde(skip_serializing_if = "is_off")]
    pub cut_itself: bool,
}

fn is_default_bend_range(range: &u8) -> bool {
    *range == DEFAULT_BEND_RANGE
}

fn is_off(on: &bool) -> bool {
    !*on
}

impl Default for Sound {
    /// How every channel played before sound settings.
    fn default() -> Self {
        Self {
            gate: None,
            semitones: 0,
            cents: 0,
            velocity: 1.0,
            attack: 0.0,
            release: 0.0,
            reverse: false,
            start: 0.0,
            filter: false,
            cutoff: *CUTOFF_RANGE.end(),
            resonance: 0.0,
            bend_range: DEFAULT_BEND_RANGE,
            cut_itself: false,
        }
    }
}

impl Sound {
    /// Whether these are the defaults: the channel plays as before sound settings.
    pub fn is_default(&self) -> bool {
        *self == Sound::default()
    }

    /// Whether these shape no voice: the defaults but for the bend range, which only a live
    /// note's bend reads, and the cut, which the player applies to a voice when a later note
    /// starts. Such a channel's voices take their old paths, bit for bit, until one is cut.
    pub fn shapes_nothing(&self) -> bool {
        Self {
            bend_range: DEFAULT_BEND_RANGE,
            cut_itself: false,
            ..*self
        }
        .is_default()
    }

    /// The gate a channel of `kind` plays with.
    pub fn gate_for(&self, kind: ChannelKind) -> Gate {
        self.gate.unwrap_or(Gate::of(kind))
    }

    /// These settings with the gate `gate` on a channel of `kind`: stored as none when it is the
    /// kind's own, so a channel set back to it is at the defaults again.
    pub fn with_gate(self, gate: Gate, kind: ChannelKind) -> Self {
        Self {
            gate: (gate != Gate::of(kind)).then_some(gate),
            ..self
        }
    }

    /// These settings as a channel whose sound becomes one of `kind` keeps them: all of them on
    /// a sampler; all but reverse and the start on an instrument set (its zones have their
    /// own); none on an audio or a plugin channel (its sound is its clips or the plugin's own,
    /// and a plugin bends by its own range). The gate stays as it was stored.
    pub fn kept_for(self, kind: ChannelKind) -> Self {
        match kind {
            ChannelKind::Sampler => self,
            ChannelKind::Instrument => Self {
                reverse: false,
                start: 0.0,
                ..self
            },
            ChannelKind::Audio | ChannelKind::Plugin | ChannelKind::Input => Self::default(),
        }
    }

    /// The first setting outside its range, or that a channel of `kind` cannot take, by its
    /// name: anything but the defaults on an audio, plugin or input channel (its sound is its
    /// clips, the plugin's own or the input's), reverse or a start on an instrument set (its
    /// zones have their own).
    pub fn problem(&self, kind: ChannelKind) -> Option<&'static str> {
        let ranges = [
            (SEMITONE_RANGE.contains(&self.semitones), "semitones"),
            (CENT_RANGE.contains(&self.cents), "cents"),
            (VELOCITY_RANGE.contains(&self.velocity), "velocity"),
            (ATTACK_RANGE.contains(&self.attack), "attack"),
            (RELEASE_RANGE.contains(&self.release), "release"),
            (START_RANGE.contains(&self.start), "start"),
            (CUTOFF_RANGE.contains(&self.cutoff), "cutoff"),
            (RESONANCE_RANGE.contains(&self.resonance), "resonance"),
            (BEND_RANGE.contains(&self.bend_range), "bend range"),
        ];
        if let Some((_, name)) = ranges.into_iter().find(|(inside, _)| !inside) {
            return Some(name);
        }
        match kind {
            ChannelKind::Audio | ChannelKind::Plugin | ChannelKind::Input if !self.is_default() => {
                Some("kind")
            }
            ChannelKind::Instrument if self.reverse => Some("reverse"),
            ChannelKind::Instrument if self.start != 0.0 => Some("start"),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The defaults write nothing but the braces; a gate set back to the kind's own is the
    // defaults again; a file naming only some settings reads the rest as defaults.
    #[test]
    fn the_defaults_write_nothing_and_read_back() {
        let sound = Sound::default();
        assert!(sound.is_default());
        assert_eq!(
            serde_json::to_string(&sound).expect("writes"),
            r#"{"semitones":0,"cents":0,"velocity":1.0,"attack":0.0,"release":0.0,"reverse":false,"start":0.0,"filter":false,"cutoff":20000.0,"resonance":0.0}"#
        );
        let hold = sound.with_gate(Gate::Hold, ChannelKind::Sampler);
        assert_eq!(hold.gate, Some(Gate::Hold));
        assert_eq!(hold.gate_for(ChannelKind::Sampler), Gate::Hold);
        assert!(hold
            .with_gate(Gate::OneShot, ChannelKind::Sampler)
            .is_default());
        assert!(sound
            .with_gate(Gate::Hold, ChannelKind::Instrument)
            .is_default());
        let read: Sound = serde_json::from_str(r#"{"gate":"hold","cents":-5}"#).expect("reads");
        assert_eq!(
            read,
            Sound {
                gate: Some(Gate::Hold),
                cents: -5,
                ..Sound::default()
            }
        );
        assert!(serde_json::from_str::<Sound>(r#"{"loudness":2}"#).is_err());
    }

    // The bend range is written last and only when not 2; a file without it reads 2; one with
    // it reads it back. A channel set off the default range alone is not at the defaults, but
    // shapes no voice.
    #[test]
    fn the_bend_range_is_written_only_off_its_default() {
        let wide = Sound {
            bend_range: 12,
            ..Sound::default()
        };
        let text = serde_json::to_string(&wide).expect("writes");
        assert!(
            text.ends_with(r#""resonance":0.0,"bend_range":12}"#),
            "{text}"
        );
        assert_eq!(serde_json::from_str::<Sound>(&text).expect("reads"), wide);
        let read: Sound = serde_json::from_str(r#"{"cents":3}"#).expect("reads");
        assert_eq!(read.bend_range, DEFAULT_BEND_RANGE);
        assert!(!wide.is_default() && wide.shapes_nothing());
        assert!(!Sound { cents: 1, ..wide }.shapes_nothing());
    }

    // Cut itself is written last, after the bend range, and only when on; a file without it
    // reads it off. It is no shape of the voices (the player cuts them), so a channel that only
    // cuts plays its notes by their old paths; a sampler and a set take it, a plugin, audio or
    // input channel cannot (its sound is its own), and a channel whose sound becomes one of
    // those lets it go.
    #[test]
    fn cut_itself_is_written_last_only_when_on_and_kept_where_the_kind_takes_it() {
        let cut = Sound {
            cut_itself: true,
            ..Sound::default()
        };
        let text = serde_json::to_string(&cut).expect("writes");
        assert!(
            text.ends_with(r#""resonance":0.0,"cut_itself":true}"#),
            "{text}"
        );
        assert_eq!(serde_json::from_str::<Sound>(&text).expect("reads"), cut);
        let wide = Sound {
            bend_range: 12,
            ..cut
        };
        let text = serde_json::to_string(&wide).expect("writes");
        assert!(
            text.ends_with(r#""bend_range":12,"cut_itself":true}"#),
            "{text}"
        );
        let read: Sound = serde_json::from_str(r#"{"cents":3}"#).expect("reads");
        assert!(!read.cut_itself);
        assert!(!cut.is_default() && cut.shapes_nothing());
        assert!(!Sound { cents: 1, ..cut }.shapes_nothing());
        assert_eq!(cut.problem(ChannelKind::Sampler), None);
        assert_eq!(cut.problem(ChannelKind::Instrument), None);
        for kind in [ChannelKind::Plugin, ChannelKind::Audio, ChannelKind::Input] {
            assert_eq!(cut.problem(kind), Some("kind"), "{kind:?}");
            assert_eq!(cut.kept_for(kind), Sound::default(), "{kind:?}");
        }
        assert_eq!(cut.kept_for(ChannelKind::Sampler), cut);
        assert_eq!(cut.kept_for(ChannelKind::Instrument), cut);
    }

    #[test]
    fn settings_outside_their_ranges_or_kind_are_named() {
        let sound = |change: fn(&mut Sound)| {
            let mut sound = Sound::default();
            change(&mut sound);
            sound
        };
        let sampler = ChannelKind::Sampler;
        assert_eq!(
            sound(|s| s.semitones = 13).problem(sampler),
            Some("semitones")
        );
        assert_eq!(sound(|s| s.cents = -101).problem(sampler), Some("cents"));
        assert_eq!(
            sound(|s| s.velocity = 1.5).problem(sampler),
            Some("velocity")
        );
        assert_eq!(
            sound(|s| s.attack = f32::NAN).problem(sampler),
            Some("attack")
        );
        assert_eq!(sound(|s| s.release = 4.5).problem(sampler), Some("release"));
        assert_eq!(sound(|s| s.cutoff = 10.0).problem(sampler), Some("cutoff"));
        assert_eq!(
            sound(|s| s.bend_range = 0).problem(sampler),
            Some("bend range")
        );
        assert_eq!(
            sound(|s| s.bend_range = 25).problem(sampler),
            Some("bend range")
        );
        assert_eq!(sound(|s| s.bend_range = 24).problem(sampler), None);
        assert_eq!(
            sound(|s| s.bend_range = 12).problem(ChannelKind::Plugin),
            Some("kind")
        );
        assert_eq!(sound(|s| s.reverse = true).problem(sampler), None);
        assert_eq!(
            sound(|s| s.reverse = true).problem(ChannelKind::Instrument),
            Some("reverse")
        );
        assert_eq!(
            sound(|s| s.start = 0.5).problem(ChannelKind::Instrument),
            Some("start")
        );
        assert_eq!(
            sound(|s| s.cents = 3).problem(ChannelKind::Plugin),
            Some("kind")
        );
        assert_eq!(Sound::default().problem(ChannelKind::Audio), None);
    }
}
