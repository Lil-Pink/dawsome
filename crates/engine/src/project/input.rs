//! An input channel's source: which audio input device it plays, and which of the device's
//! channels.

use serde::{Deserialize, Serialize};

/// The audio input an input channel plays: a device by its name and the channels of it taken.
/// The file writes `{"device":"...","channels":{"stereo":[0,1]}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputSource {
    /// The input device's name as the system lists it; empty for the system's default input.
    pub device: String,
    pub channels: InputChannels,
}

/// Which channels of an input device a channel plays, by their place on the device (0 is the
/// first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputChannels {
    /// One channel, a mono sound (through the channel pan law).
    Mono(u16),
    /// Two channels, left then right, a stereo sound (through the balance law).
    Stereo(u16, u16),
}

impl InputChannels {
    /// How many channels the sound has: 1 or 2.
    pub fn count(self) -> usize {
        match self {
            InputChannels::Mono(_) => 1,
            InputChannels::Stereo(..) => 2,
        }
    }

    /// The device channel each side of the sound takes: a mono sound's one channel on both.
    pub fn sides(self) -> [u16; 2] {
        match self {
            InputChannels::Mono(channel) => [channel, channel],
            InputChannels::Stereo(left, right) => [left, right],
        }
    }
}

impl InputSource {
    /// The system's default input, its first two channels as a stereo sound: what a new input
    /// channel plays until another is chosen.
    pub fn default_device() -> Self {
        Self {
            device: String::new(),
            channels: InputChannels::Stereo(0, 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The file's shape: the device by name, the channels by kind and place.
    #[test]
    fn a_source_writes_its_device_and_channels() {
        let source = InputSource {
            device: "USB Audio Line Input".into(),
            channels: InputChannels::Stereo(0, 1),
        };
        let text = serde_json::to_string(&source).expect("writes");
        assert_eq!(
            text,
            r#"{"device":"USB Audio Line Input","channels":{"stereo":[0,1]}}"#
        );
        assert_eq!(
            serde_json::from_str::<InputSource>(&text).expect("reads"),
            source
        );
        let mono: InputChannels = serde_json::from_str(r#"{"mono":3}"#).expect("reads");
        assert_eq!((mono.count(), mono.sides()), (1, [3, 3]));
    }
}
