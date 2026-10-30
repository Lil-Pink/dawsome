//! The piano roll's scale help: a root and a scale (major, minor, the
//! church modes, harmonic and melodic minor, the pentatonics, the blues). Keys outside it shade
//! darker in the grid and the key column; Fold keeps rows only for the keys that hold notes and,
//! with a scale chosen, the keys in it. Off by default, and UI state only: never in the file.

use std::fmt;

/// The note names of a scale's root, from C.
pub const ROOT_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// A scale chosen in the strip, or none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScaleMode {
    #[default]
    Off,
    Major,
    Minor,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Locrian,
    HarmonicMinor,
    MelodicMinor,
    MajorPentatonic,
    MinorPentatonic,
    Blues,
}

impl ScaleMode {
    /// Every choice, as the strip lists them.
    pub const ALL: [ScaleMode; 13] = [
        ScaleMode::Off,
        ScaleMode::Major,
        ScaleMode::Minor,
        ScaleMode::Dorian,
        ScaleMode::Phrygian,
        ScaleMode::Lydian,
        ScaleMode::Mixolydian,
        ScaleMode::Locrian,
        ScaleMode::HarmonicMinor,
        ScaleMode::MelodicMinor,
        ScaleMode::MajorPentatonic,
        ScaleMode::MinorPentatonic,
        ScaleMode::Blues,
    ];

    /// The semitones above the root it holds; none for no scale.
    pub fn steps(self) -> &'static [u8] {
        match self {
            ScaleMode::Off => &[],
            ScaleMode::Major => &[0, 2, 4, 5, 7, 9, 11],
            ScaleMode::Minor => &[0, 2, 3, 5, 7, 8, 10],
            ScaleMode::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            ScaleMode::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            ScaleMode::Lydian => &[0, 2, 4, 6, 7, 9, 11],
            ScaleMode::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            ScaleMode::Locrian => &[0, 1, 3, 5, 6, 8, 10],
            ScaleMode::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            ScaleMode::MelodicMinor => &[0, 2, 3, 5, 7, 9, 11],
            ScaleMode::MajorPentatonic => &[0, 2, 4, 7, 9],
            ScaleMode::MinorPentatonic => &[0, 3, 5, 7, 10],
            ScaleMode::Blues => &[0, 3, 5, 6, 7, 10],
        }
    }
}

impl fmt::Display for ScaleMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ScaleMode::Off => "Scale off",
            ScaleMode::Major => "Major",
            ScaleMode::Minor => "Minor",
            ScaleMode::Dorian => "Dorian",
            ScaleMode::Phrygian => "Phrygian",
            ScaleMode::Lydian => "Lydian",
            ScaleMode::Mixolydian => "Mixolydian",
            ScaleMode::Locrian => "Locrian",
            ScaleMode::HarmonicMinor => "Harmonic minor",
            ScaleMode::MelodicMinor => "Melodic minor",
            ScaleMode::MajorPentatonic => "Major pentatonic",
            ScaleMode::MinorPentatonic => "Minor pentatonic",
            ScaleMode::Blues => "Blues",
        })
    }
}

/// A scale's root, 0 (C) to 11 (B), as the strip's chooser lists it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Root(pub u8);

impl Root {
    pub const ALL: [Root; 12] = [
        Root(0),
        Root(1),
        Root(2),
        Root(3),
        Root(4),
        Root(5),
        Root(6),
        Root(7),
        Root(8),
        Root(9),
        Root(10),
        Root(11),
    ];
}

impl fmt::Display for Root {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(ROOT_NAMES[usize::from(self.0 % 12)])
    }
}

/// A scale on a root.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Scale {
    pub root: Root,
    pub mode: ScaleMode,
}

impl Scale {
    /// Whether a scale is chosen.
    pub fn is_on(self) -> bool {
        self.mode != ScaleMode::Off
    }

    /// Whether `key` is in it; every key is while no scale is chosen.
    pub fn holds(self, key: u8) -> bool {
        if !self.is_on() {
            return true;
        }
        let above_root = (key % 12 + 12 - self.root.0 % 12) % 12;
        self.mode.steps().contains(&above_root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pitch classes (0 = C) of the keys from 60 to 71 `scale` holds.
    fn classes(scale: Scale) -> Vec<u8> {
        (60..72)
            .filter(|&key| scale.holds(key))
            .map(|key| key - 60)
            .collect()
    }

    // From the scales as music theory spells them: C major C D E F G A B; A minor the same
    // notes (A B C D E F G); D dorian the same again; E blues E G A A# B D; F# major
    // pentatonic F# G# A# C# D#; no scale holds every key.
    #[test]
    fn scales_hold_the_notes_theory_spells() {
        let scale = |root, mode| Scale {
            root: Root(root),
            mode,
        };
        assert_eq!(classes(scale(0, ScaleMode::Major)), [0, 2, 4, 5, 7, 9, 11]);
        assert_eq!(classes(scale(9, ScaleMode::Minor)), [0, 2, 4, 5, 7, 9, 11]);
        assert_eq!(classes(scale(2, ScaleMode::Dorian)), [0, 2, 4, 5, 7, 9, 11]);
        assert_eq!(classes(scale(4, ScaleMode::Blues)), [2, 4, 7, 9, 10, 11]);
        assert_eq!(
            classes(scale(6, ScaleMode::MajorPentatonic)),
            [1, 3, 6, 8, 10]
        );
        assert_eq!(
            classes(scale(0, ScaleMode::HarmonicMinor)),
            [0, 2, 3, 5, 7, 8, 11]
        );
        assert_eq!(classes(Scale::default()).len(), 12, "off: every key");
        let c_major = scale(0, ScaleMode::Major);
        assert!(c_major.holds(0) && c_major.holds(127), "C0 and G10");
        assert!(!c_major.holds(1) && !c_major.holds(126), "C#0 and F#10");
        assert_eq!(Root(10).to_string(), "A#");
        assert_eq!(ScaleMode::HarmonicMinor.to_string(), "Harmonic minor");
    }
}
