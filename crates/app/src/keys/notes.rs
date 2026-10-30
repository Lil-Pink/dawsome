//! The note layouts: which key of the computer keyboard plays which note while the keyboard
//! plays notes, which keys beside the notes act on the keyboard itself (octave, velocity,
//! sustain, all notes off), and which do nothing. Keys are taken by their physical places
//! (`key::Code`, the place a US keyboard prints the letter on), so a French or German keyboard
//! plays the same shape; the window draws each key with the letter the user's own layout types
//! there once it has seen it typed ([`Labels`]), the US letter until then.
//!
//! Why the piano row is the default: its white keys lie under the resting hand, its black keys
//! one row up, and its octave, velocity and sustain keys under the left hand without leaving
//! it, as several widely taught programs lay it out; it also costs the fewest of the window's
//! single-key shortcuts (L and R, given back by Shift). The two rows are the layout the window
//! had before, kept for those who learned it.

use std::collections::BTreeMap;

use iced::keyboard::key::Code;

/// The velocities the velocity keys step through, on the 0..=127 scale of a MIDI note.
pub const VELOCITIES: [u8; 7] = [20, 40, 60, 80, 100, 120, 127];

/// The velocity the keyboard plays at first: 100, the drawn notes' 100/128 once scaled as a
/// MIDI device's notes are.
pub const DEFAULT_VELOCITY: u8 = 100;

/// A layout of notes on the computer keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NoteLayout {
    /// One row of white keys from A (C) to ' (F an octave up), the black keys in the row above;
    /// Z and X move the octave, C and V the velocity, Tab holds the sustain, Backspace ends
    /// every note.
    #[default]
    Piano,
    /// Two rows: from Z the octave the toolbar names, its black keys the row above it; from Q
    /// the octave above, the digits its black keys. The layout before the piano row.
    TwoRows,
}

/// What a key does while the keyboard plays notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    /// Plays the note this many semitones above the layout's first key.
    Note(u8),
    /// Acts on the keyboard itself.
    Control(KeyControl),
    /// Lies among the notes and plays none: it does nothing, so no shortcut runs by accident
    /// (R, between E and T, must never arm Record).
    Swallowed,
    /// Lies outside the notes: its shortcut works as with the keyboard off.
    Free,
}

/// A key that acts on the keyboard rather than playing a note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyControl {
    OctaveDown,
    OctaveUp,
    VelocityDown,
    VelocityUp,
    /// Held down, the notes let go of sound on until it is let go of too.
    Sustain,
    /// Every note the keyboard plays ends, sustained ones too.
    AllNotesOff,
}

/// The piano row: semitones above A's C. White keys A S D F G H J K L ; ' (C D E F G A B C D E
/// F), black keys W E T Y U O P (C# D# F# G# A# C# D#).
const PIANO: [(Code, u8); 18] = [
    (Code::KeyA, 0),
    (Code::KeyW, 1),
    (Code::KeyS, 2),
    (Code::KeyE, 3),
    (Code::KeyD, 4),
    (Code::KeyF, 5),
    (Code::KeyT, 6),
    (Code::KeyG, 7),
    (Code::KeyY, 8),
    (Code::KeyH, 9),
    (Code::KeyU, 10),
    (Code::KeyJ, 11),
    (Code::KeyK, 12),
    (Code::KeyO, 13),
    (Code::KeyL, 14),
    (Code::KeyP, 15),
    (Code::Semicolon, 16),
    (Code::Quote, 17),
];

/// The piano row's keys that act on the keyboard.
const PIANO_CONTROLS: [(Code, KeyControl); 6] = [
    (Code::KeyZ, KeyControl::OctaveDown),
    (Code::KeyX, KeyControl::OctaveUp),
    (Code::KeyC, KeyControl::VelocityDown),
    (Code::KeyV, KeyControl::VelocityUp),
    (Code::Tab, KeyControl::Sustain),
    (Code::Backspace, KeyControl::AllNotesOff),
];

/// The piano row's keys among the notes that play nothing: the gaps of the black row (Q, R, I)
/// and the rest of it, the digits right of 6 with - and =, the bottom row right of V, the key
/// left of 1 and the one left of Z. The digits 1 to 6 stay free: they show and hide the panels,
/// one row above the black keys. So does Enter, right of the last white key: it adds the
/// Library row pressed and opens the selected channel, which a finger overshooting ' may do by
/// accident, a risk taken for those (6d review).
const PIANO_SWALLOWED: [Code; 20] = [
    Code::KeyQ,
    Code::KeyR,
    Code::KeyI,
    Code::BracketLeft,
    Code::BracketRight,
    Code::Backslash,
    Code::Digit7,
    Code::Digit8,
    Code::Digit9,
    Code::Digit0,
    Code::Minus,
    Code::Equal,
    Code::KeyB,
    Code::KeyN,
    Code::KeyM,
    Code::Comma,
    Code::Period,
    Code::Slash,
    Code::Backquote,
    Code::IntlBackslash,
];

/// The two rows: semitones above Z's key. Z S X D C V G B H N J M then , L . ; / on the bottom
/// rows; Q 2 W 3 E R 5 T 6 Y 7 U then I 9 O 0 P an octave up.
const TWO_ROWS: [(Code, u8); 34] = [
    (Code::KeyZ, 0),
    (Code::KeyS, 1),
    (Code::KeyX, 2),
    (Code::KeyD, 3),
    (Code::KeyC, 4),
    (Code::KeyV, 5),
    (Code::KeyG, 6),
    (Code::KeyB, 7),
    (Code::KeyH, 8),
    (Code::KeyN, 9),
    (Code::KeyJ, 10),
    (Code::KeyM, 11),
    (Code::Comma, 12),
    (Code::KeyL, 13),
    (Code::Period, 14),
    (Code::Semicolon, 15),
    (Code::Slash, 16),
    (Code::KeyQ, 12),
    (Code::Digit2, 13),
    (Code::KeyW, 14),
    (Code::Digit3, 15),
    (Code::KeyE, 16),
    (Code::KeyR, 17),
    (Code::Digit5, 18),
    (Code::KeyT, 19),
    (Code::Digit6, 20),
    (Code::KeyY, 21),
    (Code::Digit7, 22),
    (Code::KeyU, 23),
    (Code::KeyI, 24),
    (Code::Digit9, 25),
    (Code::KeyO, 26),
    (Code::Digit0, 27),
    (Code::KeyP, 28),
];

/// The keys the two rows leave their shortcuts: as before the piano row came, every other key
/// is the keyboard's while it plays notes.
const TWO_ROWS_FREE: [Code; 14] = [
    Code::Space,
    Code::Escape,
    Code::F1,
    Code::F2,
    Code::F3,
    Code::F4,
    Code::F5,
    Code::F6,
    Code::F7,
    Code::F8,
    Code::F9,
    Code::F10,
    Code::F11,
    Code::F12,
];

impl NoteLayout {
    /// Every layout, as Settings offers them.
    pub const ALL: [NoteLayout; 2] = [NoteLayout::Piano, NoteLayout::TwoRows];

    /// Its name in Settings.
    pub fn name(self) -> &'static str {
        match self {
            NoteLayout::Piano => "Piano (single row)",
            NoteLayout::TwoRows => "Two rows (legacy)",
        }
    }

    /// Its name in the settings file.
    pub fn id(self) -> &'static str {
        match self {
            NoteLayout::Piano => "piano",
            NoteLayout::TwoRows => "two-rows",
        }
    }

    /// The layout of id `id`; none for an id this build does not know.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|layout| layout.id() == id)
    }

    /// The key its first note plays at first: the piano row's A middle C (60, "C5", the
    /// sampler's own pitch), the two rows' Z an octave lower (48, "C4"), as before.
    pub const fn start(self) -> u8 {
        match self {
            NoteLayout::Piano => 60,
            NoteLayout::TwoRows => 48,
        }
    }

    /// The physical key that plays its first note: A or Z.
    pub fn first_key(self) -> Code {
        match self {
            NoteLayout::Piano => Code::KeyA,
            NoteLayout::TwoRows => Code::KeyZ,
        }
    }

    /// Its note keys and the semitones each plays above the first, in the order of [`Claim`]s
    /// a test or a drawing walks.
    pub fn notes(self) -> &'static [(Code, u8)] {
        match self {
            NoteLayout::Piano => &PIANO,
            NoteLayout::TwoRows => &TWO_ROWS,
        }
    }

    /// What a key the toolkit could not place does: the piano row leaves it its shortcut; the
    /// two rows swallow it, as every key but Space, Esc and the function keys before.
    pub fn claim_unplaced(self) -> Claim {
        match self {
            NoteLayout::Piano => Claim::Free,
            NoteLayout::TwoRows => Claim::Swallowed,
        }
    }

    /// What `code` does in this layout while the keyboard plays notes.
    pub fn claim(self, code: Code) -> Claim {
        if let Some(&(_, offset)) = self.notes().iter().find(|(mapped, _)| *mapped == code) {
            return Claim::Note(offset);
        }
        match self {
            NoteLayout::Piano => {
                if let Some(&(_, control)) = PIANO_CONTROLS.iter().find(|(key, _)| *key == code) {
                    Claim::Control(control)
                } else if PIANO_SWALLOWED.contains(&code) {
                    Claim::Swallowed
                } else {
                    Claim::Free
                }
            }
            NoteLayout::TwoRows if TWO_ROWS_FREE.contains(&code) => Claim::Free,
            NoteLayout::TwoRows => Claim::Swallowed,
        }
    }
}

/// The velocity one step from `velocity` on [`VELOCITIES`], up or down, kept at its ends.
pub fn step_velocity(velocity: u8, up: bool) -> u8 {
    if up {
        VELOCITIES
            .into_iter()
            .find(|&step| step > velocity)
            .unwrap_or(VELOCITIES[VELOCITIES.len() - 1])
    } else {
        VELOCITIES
            .into_iter()
            .rev()
            .find(|&step| step < velocity)
            .unwrap_or(VELOCITIES[0])
    }
}

/// The letters the user's keyboard types on each physical key, as the window last saw them
/// typed. Why learned: the toolkit says what a key typed only when it is pressed, never what a
/// layout would type; until a key is pressed it is named by its US letter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Labels(BTreeMap<Code, String>);

impl Labels {
    /// `code` typed `typed` (the character without modifiers): a single printable character
    /// becomes its label, as a capital.
    pub fn saw(&mut self, code: Code, typed: &str) {
        let mut chars = typed.chars();
        if let (Some(character), None) = (chars.next(), chars.next()) {
            if !character.is_control() && !character.is_whitespace() {
                self.0.insert(code, character.to_uppercase().collect());
            }
        }
    }

    /// Where the key that types `label` (a capital) lies: where the user's keyboard was seen
    /// typing it, else where a US keyboard has it.
    pub fn code_of(&self, label: &str) -> Option<Code> {
        self.0
            .iter()
            .find(|(_, seen)| seen.as_str() == label)
            .map(|(&code, _)| code)
            .or_else(|| {
                LABELED
                    .into_iter()
                    .find(|&code| us_label(code) == label && !self.0.contains_key(&code))
            })
    }

    /// The name of `code`: what the user's keyboard types there, else its US letter.
    pub fn label(&self, code: Code) -> String {
        self.0
            .get(&code)
            .cloned()
            .unwrap_or_else(|| us_label(code).to_owned())
    }
}

/// The keys [`us_label`] names.
const LABELED: [Code; 51] = [
    Code::KeyA,
    Code::KeyB,
    Code::KeyC,
    Code::KeyD,
    Code::KeyE,
    Code::KeyF,
    Code::KeyG,
    Code::KeyH,
    Code::KeyI,
    Code::KeyJ,
    Code::KeyK,
    Code::KeyL,
    Code::KeyM,
    Code::KeyN,
    Code::KeyO,
    Code::KeyP,
    Code::KeyQ,
    Code::KeyR,
    Code::KeyS,
    Code::KeyT,
    Code::KeyU,
    Code::KeyV,
    Code::KeyW,
    Code::KeyX,
    Code::KeyY,
    Code::KeyZ,
    Code::Digit1,
    Code::Digit2,
    Code::Digit3,
    Code::Digit4,
    Code::Digit5,
    Code::Digit6,
    Code::Digit7,
    Code::Digit8,
    Code::Digit9,
    Code::Digit0,
    Code::Minus,
    Code::Equal,
    Code::BracketLeft,
    Code::BracketRight,
    Code::Backslash,
    Code::Semicolon,
    Code::Quote,
    Code::Comma,
    Code::Period,
    Code::Slash,
    Code::Backquote,
    Code::IntlBackslash,
    Code::Tab,
    Code::Backspace,
    Code::Enter,
];

/// What a US keyboard prints on `code`, for the keys the layouts and the keyboard's drawing use.
pub fn us_label(code: Code) -> &'static str {
    match code {
        Code::KeyA => "A",
        Code::KeyB => "B",
        Code::KeyC => "C",
        Code::KeyD => "D",
        Code::KeyE => "E",
        Code::KeyF => "F",
        Code::KeyG => "G",
        Code::KeyH => "H",
        Code::KeyI => "I",
        Code::KeyJ => "J",
        Code::KeyK => "K",
        Code::KeyL => "L",
        Code::KeyM => "M",
        Code::KeyN => "N",
        Code::KeyO => "O",
        Code::KeyP => "P",
        Code::KeyQ => "Q",
        Code::KeyR => "R",
        Code::KeyS => "S",
        Code::KeyT => "T",
        Code::KeyU => "U",
        Code::KeyV => "V",
        Code::KeyW => "W",
        Code::KeyX => "X",
        Code::KeyY => "Y",
        Code::KeyZ => "Z",
        Code::Digit1 => "1",
        Code::Digit2 => "2",
        Code::Digit3 => "3",
        Code::Digit4 => "4",
        Code::Digit5 => "5",
        Code::Digit6 => "6",
        Code::Digit7 => "7",
        Code::Digit8 => "8",
        Code::Digit9 => "9",
        Code::Digit0 => "0",
        Code::Minus => "-",
        Code::Equal => "=",
        Code::BracketLeft => "[",
        Code::BracketRight => "]",
        Code::Backslash => "\\",
        Code::Semicolon => ";",
        Code::Quote => "'",
        Code::Comma => ",",
        Code::Period => ".",
        Code::Slash => "/",
        Code::Backquote => "`",
        Code::IntlBackslash => "<",
        Code::Tab => "Tab",
        Code::Backspace => "Backspace",
        Code::Enter => "Enter",
        _ => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Known answers from the layout research's table (The Library, daw-computer-keyboard-layouts
    // 2026-10-22, section 8), written as MIDI keys with A on 60: every white and black key,
    // and the keys between that play nothing. Enter (and the numpad's) keeps its shortcut: the
    // Its swallowing was reversed, and this expectation moved with it.
    #[test]
    fn the_piano_row_plays_the_researched_map_from_a_on_middle_c() {
        let table = [
            (Code::KeyA, 60),
            (Code::KeyS, 62),
            (Code::KeyD, 64),
            (Code::KeyF, 65),
            (Code::KeyG, 67),
            (Code::KeyH, 69),
            (Code::KeyJ, 71),
            (Code::KeyK, 72),
            (Code::KeyL, 74),
            (Code::Semicolon, 76),
            (Code::Quote, 77),
            (Code::KeyW, 61),
            (Code::KeyE, 63),
            (Code::KeyT, 66),
            (Code::KeyY, 68),
            (Code::KeyU, 70),
            (Code::KeyO, 73),
            (Code::KeyP, 75),
        ];
        let layout = NoteLayout::Piano;
        assert_eq!(layout.start(), 60);
        assert_eq!(layout.notes().len(), table.len());
        for (code, key) in table {
            assert_eq!(
                layout.claim(code),
                Claim::Note(key - 60),
                "{code:?} plays key {key}"
            );
        }
        for code in [Code::KeyQ, Code::KeyR, Code::KeyI, Code::KeyB] {
            assert_eq!(layout.claim(code), Claim::Swallowed, "{code:?}");
        }
        for (code, control) in [
            (Code::KeyZ, KeyControl::OctaveDown),
            (Code::KeyX, KeyControl::OctaveUp),
            (Code::KeyC, KeyControl::VelocityDown),
            (Code::KeyV, KeyControl::VelocityUp),
            (Code::Tab, KeyControl::Sustain),
            (Code::Backspace, KeyControl::AllNotesOff),
        ] {
            assert_eq!(layout.claim(code), Claim::Control(control), "{code:?}");
        }
        for code in [
            Code::Space,
            Code::Delete,
            Code::ArrowDown,
            Code::F5,
            Code::Digit1,
            Code::Digit6,
            Code::Escape,
            Code::Enter,
            Code::NumpadEnter,
        ] {
            assert_eq!(layout.claim(code), Claim::Free, "{code:?}");
        }
    }

    // The two rows as before: Z on 48, the digits black keys of the upper row, everything but
    // Space, Esc and the function keys the keyboard's.
    #[test]
    fn the_two_rows_keep_the_map_they_had() {
        let layout = NoteLayout::TwoRows;
        assert_eq!(layout.start(), 48);
        for (code, offset) in [
            (Code::KeyZ, 0),
            (Code::Comma, 12),
            (Code::Digit2, 13),
            (Code::KeyP, 28),
        ] {
            assert_eq!(layout.claim(code), Claim::Note(offset), "{code:?}");
        }
        for code in [
            Code::KeyA,
            Code::Delete,
            Code::Enter,
            Code::Tab,
            Code::Digit1,
        ] {
            assert_eq!(layout.claim(code), Claim::Swallowed, "{code:?}");
        }
        assert_eq!(layout.claim(Code::F10), Claim::Free);
        assert_eq!(layout.claim(Code::Space), Claim::Free);
    }

    // The research's velocity steps, from 100: down 80, 60, 40, 20 and no lower; up 120, 127
    // and no higher.
    #[test]
    fn velocity_steps_through_the_seven_values_and_stops_at_the_ends() {
        let walk = |up: bool| {
            let mut velocity = DEFAULT_VELOCITY;
            (0..5)
                .map(|_| {
                    velocity = step_velocity(velocity, up);
                    velocity
                })
                .collect::<Vec<u8>>()
        };
        assert_eq!(walk(false), [80, 60, 40, 20, 20]);
        assert_eq!(walk(true), [120, 127, 127, 127, 127]);
    }

    #[test]
    fn layouts_read_back_from_their_ids() {
        for layout in NoteLayout::ALL {
            assert_eq!(NoteLayout::from_id(layout.id()), Some(layout));
        }
        assert_eq!(NoteLayout::from_id("qwertz"), None);
        assert_eq!(Labels::default().label(Code::Semicolon), ";");
    }

    // 6d K2: a key is named by what the user's keyboard typed on it (a French keyboard types q
    // on the US A), as a capital; nothing typed, a space, or more than one character leaves
    // the US name.
    #[test]
    fn labels_follow_what_was_typed() {
        let mut labels = Labels::default();
        assert_eq!(labels.label(Code::KeyA), "A");
        labels.saw(Code::KeyA, "q");
        assert_eq!(labels.label(Code::KeyA), "Q");
        labels.saw(Code::KeyS, "");
        labels.saw(Code::Space, " ");
        labels.saw(Code::KeyD, "ab");
        assert_eq!(
            [Code::KeyS, Code::Space, Code::KeyD].map(|code| labels.label(code)),
            ["S", "?", "D"]
        );
        // Back from a letter to its key: Q is where it was seen typed, A (not seen) where a US
        // keyboard has it, unless that key was seen typing something else.
        assert_eq!(labels.code_of("Q"), Some(Code::KeyA));
        assert_eq!(labels.code_of("L"), Some(Code::KeyL));
        assert_eq!(labels.code_of("A"), None, "the US A types Q here");
        assert_eq!(labels.code_of(";"), Some(Code::Semicolon));
    }
}
