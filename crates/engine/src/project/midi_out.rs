//! An input channel's MIDI out: the MIDI port its notes are sent to, and how they are sent.

use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

use super::is_false;

/// The MIDI channels a MIDI out sends on, as people count them (the status byte holds one less).
pub const MIDI_OUT_CHANNELS: RangeInclusive<u8> = 1..=16;

/// The programs a MIDI out selects: a program change's data byte (shown 1 to 128).
pub const PROGRAM_RANGE: RangeInclusive<u8> = 0..=127;

/// How far ahead of its time a note may be sent, in milliseconds: the device's own delay from a
/// note arriving to its sound coming back.
pub const LATENCY_RANGE: RangeInclusive<i32> = 0..=200;

/// Where an input channel's notes go: the channel's sound comes from an outside instrument
/// (its audio input), and this sends that instrument the channel's notes. The file writes
/// `{"port":"...","channel":1,"latency_ms":0}`, with `"program"` only when one is set and
/// `"echo":true` only when on.
///
/// Why on the channel and not in its [`super::InputSource`]: the window reopens the audio input
/// whenever the source changes, and a MIDI setting changing must not cut the sound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiOut {
    /// The MIDI output port by its stable name (see [`super::stable_port_name`]); never empty.
    pub port: String,
    /// The MIDI channel the notes go out on, in [`MIDI_OUT_CHANNELS`].
    pub channel: u8,
    /// The program the port is set to, in [`PROGRAM_RANGE`]; `None` leaves the instrument's
    /// own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<u8>,
    /// How many milliseconds ahead each note is sent, in [`LATENCY_RANGE`], so the instrument's
    /// sound comes back on time.
    pub latency_ms: i32,
    /// Whether notes the channel takes from its linked MIDI input go out too. Off by default:
    /// an instrument linked to its own channel already hears its keys, and echoing them would
    /// play every note twice.
    #[serde(default, skip_serializing_if = "is_false")]
    pub echo: bool,
}

impl MidiOut {
    /// The first setting that is out of range or missing, by its file name; `None` when every
    /// one is in range.
    pub fn problem(&self) -> Option<&'static str> {
        if self.port.is_empty() {
            Some("port")
        } else if !MIDI_OUT_CHANNELS.contains(&self.channel) {
            Some("channel")
        } else if self
            .program
            .is_some_and(|program| !PROGRAM_RANGE.contains(&program))
        {
            Some("program")
        } else if !LATENCY_RANGE.contains(&self.latency_ms) {
            Some("latency_ms")
        } else {
            None
        }
    }

    /// The latency in whole frames at `sample_rate`: `floor(latency_ms * sample_rate / 1000)`,
    /// 0 for a latency below 0. Every frame count of the latency comes from here.
    pub fn latency_frames(&self, sample_rate: u32) -> u64 {
        let ms = u64::try_from(self.latency_ms).unwrap_or(0);
        ms * u64::from(sample_rate) / 1000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out() -> MidiOut {
        MidiOut {
            port: "Synth:Synth MIDI 1".into(),
            channel: 1,
            program: None,
            latency_ms: 32,
            echo: false,
        }
    }

    // The file's shape: the program and echo left out at their defaults, written when set.
    #[test]
    fn a_midi_out_writes_its_settings_and_leaves_out_the_defaults() {
        let plain = serde_json::to_string(&out()).expect("writes");
        assert_eq!(
            plain,
            r#"{"port":"Synth:Synth MIDI 1","channel":1,"latency_ms":32}"#
        );
        let set = MidiOut {
            program: Some(5),
            echo: true,
            ..out()
        };
        let text = serde_json::to_string(&set).expect("writes");
        assert_eq!(
            text,
            r#"{"port":"Synth:Synth MIDI 1","channel":1,"program":5,"latency_ms":32,"echo":true}"#
        );
        assert_eq!(serde_json::from_str::<MidiOut>(&text).expect("reads"), set);
    }

    // By hand: 32 ms at 44.1 kHz is 1411.2 frames, floored; at 48 kHz exactly 1536.
    #[test]
    fn the_latency_is_whole_frames_rounded_down() {
        assert_eq!(out().latency_frames(44_100), 1411);
        assert_eq!(out().latency_frames(48_000), 1536);
        let negative = MidiOut {
            latency_ms: -5,
            ..out()
        };
        assert_eq!(negative.latency_frames(48_000), 0);
    }

    #[test]
    fn each_setting_out_of_range_is_named() {
        assert_eq!(out().problem(), None);
        let cases: [(MidiOut, &str); 6] = [
            (
                MidiOut {
                    port: String::new(),
                    ..out()
                },
                "port",
            ),
            (
                MidiOut {
                    channel: 0,
                    ..out()
                },
                "channel",
            ),
            (
                MidiOut {
                    channel: 17,
                    ..out()
                },
                "channel",
            ),
            (
                MidiOut {
                    program: Some(128),
                    ..out()
                },
                "program",
            ),
            (
                MidiOut {
                    latency_ms: -1,
                    ..out()
                },
                "latency_ms",
            ),
            (
                MidiOut {
                    latency_ms: 201,
                    ..out()
                },
                "latency_ms",
            ),
        ];
        for (midi_out, setting) in cases {
            assert_eq!(midi_out.problem(), Some(setting), "{midi_out:?}");
        }
    }
}
