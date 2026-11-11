//! A project's key: a root pitch class and a scale. A project setting, written only when set,
//! which makes the file version [`super::KEY_FORMAT_VERSION`]. A Jam writes its chords, bass and
//! melody in it.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A pitch class: 0 is C, 1 C sharp, up to 11, B. Only those twelve exist, so a file naming
/// another is refused as it is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct PitchClass(u8);

/// Why a pitch class or a scale was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KeyError {
    #[error("pitch class {0} is not one of 0 (C) to 11 (B)")]
    PitchClassOutOfRange(u8),
    #[error("'{0}' is not a key: a letter A to G, then # or b for sharp or flat (C, F#, Bb)")]
    UnknownRoot(String),
    #[error("'{0}' is not a scale: major, minor, dorian, mixolydian, harmonic-minor, pentatonic-major or pentatonic-minor")]
    UnknownScale(String),
}

impl PitchClass {
    pub const C: PitchClass = PitchClass(0);

    /// Each pitch class's name, sharps for the black keys.
    pub const NAMES: [&'static str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];

    /// The pitch class `value` (0 is C), or `None` past 11.
    pub const fn new(value: u8) -> Option<Self> {
        if value < 12 {
            Some(Self(value))
        } else {
            None
        }
    }

    /// The pitch class of MIDI key `key` (60 is C).
    pub const fn of_key(key: u8) -> Self {
        Self(key % 12)
    }

    pub const fn get(self) -> u8 {
        self.0
    }

    pub fn name(self) -> &'static str {
        Self::NAMES[usize::from(self.0)]
    }
}

impl TryFrom<u8> for PitchClass {
    type Error = KeyError;

    fn try_from(value: u8) -> Result<Self, KeyError> {
        Self::new(value).ok_or(KeyError::PitchClassOutOfRange(value))
    }
}

impl From<PitchClass> for u8 {
    fn from(class: PitchClass) -> u8 {
        class.0
    }
}

impl FromStr for PitchClass {
    type Err = KeyError;

    /// A letter A to G (either case), then nothing, `#` (sharp) or `b` (flat): "C", "F#", "Bb".
    fn from_str(text: &str) -> Result<Self, KeyError> {
        let refused = || KeyError::UnknownRoot(text.to_owned());
        let mut chars = text.chars();
        let natural: u8 = match chars.next().map(|c| c.to_ascii_uppercase()) {
            Some('C') => 0,
            Some('D') => 2,
            Some('E') => 4,
            Some('F') => 5,
            Some('G') => 7,
            Some('A') => 9,
            Some('B') => 11,
            _ => return Err(refused()),
        };
        let class = match chars.as_str() {
            "" => natural,
            "#" => (natural + 1) % 12,
            "b" => (natural + 11) % 12,
            _ => return Err(refused()),
        };
        Ok(Self(class))
    }
}

/// A scale: which pitch classes above the root belong to the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scale {
    Major,
    /// The natural minor.
    Minor,
    Dorian,
    Mixolydian,
    HarmonicMinor,
    PentatonicMajor,
    PentatonicMinor,
}

impl Scale {
    pub const ALL: [Scale; 7] = [
        Scale::Major,
        Scale::Minor,
        Scale::Dorian,
        Scale::Mixolydian,
        Scale::HarmonicMinor,
        Scale::PentatonicMajor,
        Scale::PentatonicMinor,
    ];

    /// Semitones from the root of each of the scale's notes, low to high.
    pub const fn steps(self) -> &'static [u8] {
        match self {
            Scale::Major => &[0, 2, 4, 5, 7, 9, 11],
            Scale::Minor => &[0, 2, 3, 5, 7, 8, 10],
            Scale::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Scale::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            Scale::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            Scale::PentatonicMajor => &[0, 2, 4, 7, 9],
            Scale::PentatonicMinor => &[0, 3, 5, 7, 10],
        }
    }

    /// The seven-note scale chords are stacked in: the scale itself, or for a pentatonic the
    /// scale it is taken from. Why: thirds stacked in five notes are not thirds.
    pub const fn chord_scale(self) -> Scale {
        match self {
            Scale::PentatonicMajor => Scale::Major,
            Scale::PentatonicMinor => Scale::Minor,
            other => other,
        }
    }

    /// Whether the scale has a minor third (minor, dorian, harmonic minor, pentatonic minor),
    /// which picks the minor progressions and the minor pentatonic for a melody.
    pub fn is_minor_family(self) -> bool {
        self.steps().contains(&3)
    }

    /// The name shown to the user.
    pub const fn name(self) -> &'static str {
        match self {
            Scale::Major => "Major",
            Scale::Minor => "Minor",
            Scale::Dorian => "Dorian",
            Scale::Mixolydian => "Mixolydian",
            Scale::HarmonicMinor => "Harmonic minor",
            Scale::PentatonicMajor => "Pentatonic major",
            Scale::PentatonicMinor => "Pentatonic minor",
        }
    }

    /// The word the command line takes.
    pub const fn id(self) -> &'static str {
        match self {
            Scale::Major => "major",
            Scale::Minor => "minor",
            Scale::Dorian => "dorian",
            Scale::Mixolydian => "mixolydian",
            Scale::HarmonicMinor => "harmonic-minor",
            Scale::PentatonicMajor => "pentatonic-major",
            Scale::PentatonicMinor => "pentatonic-minor",
        }
    }
}

impl FromStr for Scale {
    type Err = KeyError;

    /// The command line's word ([`Scale::id`]), in either case, `_` taken for `-`.
    fn from_str(text: &str) -> Result<Self, KeyError> {
        let word = text.to_ascii_lowercase().replace('_', "-");
        Scale::ALL
            .into_iter()
            .find(|scale| scale.id() == word)
            .ok_or_else(|| KeyError::UnknownScale(text.to_owned()))
    }
}

/// A key: its root and its scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeySignature {
    pub root: PitchClass,
    pub scale: Scale,
}

impl KeySignature {
    /// Whether MIDI key `key` is one of the key's notes.
    pub fn contains(self, key: u8) -> bool {
        let above_root = (PitchClass::of_key(key).get() + 12 - self.root.get()) % 12;
        self.scale.steps().contains(&above_root)
    }
}

impl fmt::Display for KeySignature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scale = self.scale.name().to_ascii_lowercase();
        write!(f, "{} {scale}", self.root.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_read_sharps_and_flats_and_refuse_the_rest() {
        let read = |text: &str| text.parse::<PitchClass>().map(PitchClass::get);
        assert_eq!(read("C"), Ok(0));
        assert_eq!(read("A"), Ok(9));
        assert_eq!(read("Bb"), Ok(10));
        assert_eq!(read("bb"), Ok(10));
        assert_eq!(read("F#"), Ok(6));
        assert_eq!(read("Cb"), Ok(11));
        assert_eq!(read("B#"), Ok(0));
        for bad in ["", "H", "C##", "Cx", "#"] {
            assert!(read(bad).is_err(), "{bad}");
        }
        assert_eq!(PitchClass::new(12), None);
        assert!(serde_json::from_str::<PitchClass>("12").is_err());
        assert_eq!(
            serde_json::from_str::<PitchClass>("11")
                .map(PitchClass::get)
                .ok(),
            Some(11)
        );
    }

    #[test]
    fn scales_read_their_words_and_write_snake_case() {
        for scale in Scale::ALL {
            assert_eq!(scale.id().parse::<Scale>(), Ok(scale));
        }
        assert_eq!("Harmonic_Minor".parse::<Scale>(), Ok(Scale::HarmonicMinor));
        assert!("lydian".parse::<Scale>().is_err());
        let key = KeySignature {
            root: PitchClass::new(9).expect("A"),
            scale: Scale::HarmonicMinor,
        };
        let text = serde_json::to_string(&key).expect("writes");
        assert_eq!(text, r#"{"root":9,"scale":"harmonic_minor"}"#);
        assert_eq!(serde_json::from_str::<KeySignature>(&text).ok(), Some(key));
        assert_eq!(key.to_string(), "A harmonic minor");
    }

    // Membership by the spec's step lists: A minor holds A B C D E F G, not C# or G#; C
    // pentatonic major holds C D E G A only; the minor family is the scales with a minor third.
    #[test]
    fn a_key_holds_its_scales_notes() {
        let a_minor = KeySignature {
            root: PitchClass::new(9).expect("A"),
            scale: Scale::Minor,
        };
        let held: Vec<u8> = (57..69).filter(|&k| a_minor.contains(k)).collect();
        assert_eq!(held, [57, 59, 60, 62, 64, 65, 67]);
        let c_pent = KeySignature {
            root: PitchClass::C,
            scale: Scale::PentatonicMajor,
        };
        let held: Vec<u8> = (60..72).filter(|&k| c_pent.contains(k)).collect();
        assert_eq!(held, [60, 62, 64, 67, 69]);
        let minor: Vec<Scale> = Scale::ALL
            .into_iter()
            .filter(|s| s.is_minor_family())
            .collect();
        assert_eq!(
            minor,
            [
                Scale::Minor,
                Scale::Dorian,
                Scale::HarmonicMinor,
                Scale::PentatonicMinor
            ]
        );
        for scale in Scale::ALL {
            assert_eq!(scale.chord_scale().steps().len(), 7, "{scale:?}");
        }
    }
}
