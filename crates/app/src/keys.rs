//! The computer keyboard: the one table of the window's shortcuts ([`Keys`]: every [`Action`]
//! and the key chords that run it, the defaults, the user's own from `keys` in the settings
//! file), and the note layouts it plays while the keyboard plays notes ([`NoteLayout`], by the
//! keys' physical places). The shortcuts are matched by what a key types (Ctrl+Z is where the
//! user's keyboard prints Z), the notes by where a key lies (A S D F stay under the hand on any
//! layout). Esc is no action of the table: it cancels whatever waits for a key, rebinding too.

mod notes;

use std::collections::BTreeMap;

use iced::keyboard::key::{Code, Named};
use iced::keyboard::{Key, Modifiers};

pub use notes::{step_velocity, Claim, KeyControl, Labels, NoteLayout, DEFAULT_VELOCITY};

use crate::ui::panels::Panel;

/// Something a key chord runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    PlayStop,
    ToggleMode,
    ArmRecord,
    CountIn,
    Capture,
    Keyboard,
    Undo,
    Redo,
    Delete,
    Duplicate,
    SelectAll,
    Route,
    OpenSelected,
    Panel(Panel),
    /// The Help window, opened or closed.
    Help,
    LibraryUp,
    LibraryDown,
    LibraryClose,
    LibraryOpen,
    TransposeUp,
    TransposeDown,
    OctaveUp,
    OctaveDown,
    NudgeLeft,
    NudgeRight,
    Copy,
    Cut,
    Paste,
    New,
    Open,
    Save,
    SaveAs,
}

impl Action {
    /// Every action, in the order the Keys page lists them; a chord two actions share runs the
    /// one listed first.
    pub const ALL: [Action; 37] = [
        Action::PlayStop,
        Action::ToggleMode,
        Action::ArmRecord,
        Action::CountIn,
        Action::Capture,
        Action::Keyboard,
        Action::Undo,
        Action::Redo,
        Action::Delete,
        Action::Duplicate,
        Action::SelectAll,
        Action::Route,
        Action::OpenSelected,
        Action::Panel(Panel::Rack),
        Action::Panel(Panel::Playlist),
        Action::Panel(Panel::PianoRoll),
        Action::Panel(Panel::Mixer),
        Action::Panel(Panel::Browser),
        Action::Panel(Panel::Settings),
        Action::Help,
        Action::LibraryUp,
        Action::LibraryDown,
        Action::LibraryClose,
        Action::LibraryOpen,
        Action::TransposeUp,
        Action::TransposeDown,
        Action::OctaveUp,
        Action::OctaveDown,
        Action::NudgeLeft,
        Action::NudgeRight,
        Action::Copy,
        Action::Cut,
        Action::Paste,
        Action::New,
        Action::Open,
        Action::Save,
        Action::SaveAs,
    ];

    /// Its name in the settings file.
    pub fn id(self) -> &'static str {
        match self {
            Action::PlayStop => "play",
            Action::ToggleMode => "mode",
            Action::ArmRecord => "record",
            Action::CountIn => "count_in",
            Action::Capture => "capture",
            Action::Keyboard => "keyboard",
            Action::Undo => "undo",
            Action::Redo => "redo",
            Action::Delete => "delete",
            Action::Duplicate => "duplicate",
            Action::SelectAll => "select_all",
            Action::TransposeUp => "transpose_up",
            Action::TransposeDown => "transpose_down",
            Action::OctaveUp => "octave_up",
            Action::OctaveDown => "octave_down",
            Action::NudgeLeft => "nudge_left",
            Action::NudgeRight => "nudge_right",
            Action::Copy => "copy",
            Action::Cut => "cut",
            Action::Paste => "paste",
            Action::Route => "route",
            Action::OpenSelected => "open_selected",
            Action::Panel(Panel::Rack) => "rack",
            Action::Panel(Panel::PianoRoll) => "piano_roll",
            Action::Panel(Panel::Playlist) => "playlist",
            Action::Panel(Panel::Mixer) => "mixer",
            Action::Panel(Panel::Browser) => "library",
            Action::Panel(Panel::Settings) => "settings",
            Action::Help => "help",
            Action::LibraryUp => "library_up",
            Action::LibraryDown => "library_down",
            Action::LibraryClose => "library_close",
            Action::LibraryOpen => "library_open",
            Action::New => "new",
            Action::Open => "open",
            Action::Save => "save",
            Action::SaveAs => "save_as",
        }
    }

    /// The action of `id`; none for one this build does not know.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.id() == id)
    }

    /// Its name on the Keys page.
    pub fn label(self) -> &'static str {
        match self {
            Action::PlayStop => "Play / stop",
            Action::ToggleMode => "Pattern / song mode",
            Action::ArmRecord => "Arm Record",
            Action::CountIn => "Count-in",
            Action::Capture => "Capture",
            Action::Keyboard => "Keyboard plays notes",
            Action::Undo => "Undo",
            Action::Redo => "Redo",
            Action::Delete => "Delete the selection",
            Action::Duplicate => "Duplicate the selection",
            Action::SelectAll => "Select all notes",
            Action::TransposeUp => "Notes: up a semitone",
            Action::TransposeDown => "Notes: down a semitone",
            Action::OctaveUp => "Notes: up an octave",
            Action::OctaveDown => "Notes: down an octave",
            Action::NudgeLeft => "Notes: earlier by the snap",
            Action::NudgeRight => "Notes: later by the snap",
            Action::Copy => "Copy notes",
            Action::Cut => "Cut notes",
            Action::Paste => "Paste notes",
            Action::Route => "Route to the selected insert",
            Action::OpenSelected => "Open or add what is selected",
            Action::Panel(panel) => panel.title(),
            Action::Help => "Help",
            Action::LibraryUp => "Library: up",
            Action::LibraryDown => "Library: down",
            Action::LibraryClose => "Library: close a folder",
            Action::LibraryOpen => "Library: open a folder",
            Action::New => "New project",
            Action::Open => "Open\u{2026}",
            Action::Save => "Save",
            Action::SaveAs => "Save As\u{2026}",
        }
    }

    /// What it does, after its keys in a hint: "switches pattern/song".
    pub fn does(self) -> &'static str {
        match self {
            Action::PlayStop => "plays and stops",
            Action::ToggleMode => "switches pattern/song",
            Action::ArmRecord => "arms Record",
            Action::CountIn => "switches the count-in",
            Action::Capture => "turns the notes played in the last 30 s into notes",
            Action::Keyboard => "switches the keyboard's notes",
            Action::Undo => "undoes",
            Action::Redo => "redoes",
            Action::Delete => "deletes the selection",
            Action::Duplicate => "duplicates the selection",
            Action::SelectAll => "selects all notes",
            Action::TransposeUp => "moves the selected notes up a semitone",
            Action::TransposeDown => "moves the selected notes down a semitone",
            Action::OctaveUp => "moves the selected notes up an octave",
            Action::OctaveDown => "moves the selected notes down an octave",
            Action::NudgeLeft => "moves the selected notes earlier by the snap",
            Action::NudgeRight => "moves the selected notes later by the snap",
            Action::Copy => "copies the selected notes",
            Action::Cut => "cuts the selected notes",
            Action::Paste => "pastes the copied notes",
            Action::Route => "routes the channel",
            Action::OpenSelected => "opens or adds what is selected",
            Action::Panel(_) => "shows or hides a panel",
            Action::Help => "opens or closes the Help window",
            Action::LibraryUp | Action::LibraryDown => "walks the Library",
            Action::LibraryClose => "closes a Library folder",
            Action::LibraryOpen => "opens a Library folder",
            Action::New => "starts a new project",
            Action::Open => "opens a project",
            Action::Save => "saves",
            Action::SaveAs => "saves under a new name",
        }
    }

    /// Where its keys act: anywhere in the window, or only while a panel has the keys.
    pub fn scope(self) -> Scope {
        match self {
            Action::LibraryUp
            | Action::LibraryDown
            | Action::LibraryClose
            | Action::LibraryOpen => Scope::Library,
            Action::TransposeUp
            | Action::TransposeDown
            | Action::OctaveUp
            | Action::OctaveDown
            | Action::NudgeLeft
            | Action::NudgeRight
            | Action::Copy
            | Action::Cut
            | Action::Paste => Scope::Roll,
            _ => Scope::Window,
        }
    }

    /// Whether it edits the piano roll's notes while the roll has the keys: its own actions,
    /// and Delete, Duplicate and Select all, which act on the panel with the keys. These take
    /// their keys back from the note layout while the roll has them.
    pub fn edits_notes(self) -> bool {
        self.scope() == Scope::Roll
            || matches!(self, Action::Delete | Action::Duplicate | Action::SelectAll)
    }

    /// Whether its keys act on the sample editor while its window has the keyboard: Delete,
    /// Select all and Play / stop, on its selection and its sound.
    pub fn in_sample_editor(self) -> bool {
        matches!(self, Action::Delete | Action::SelectAll | Action::PlayStop)
    }

    /// Whether a key held down runs it again with each repeat.
    pub fn repeats(self) -> bool {
        matches!(
            self,
            Action::Undo
                | Action::Redo
                | Action::Delete
                | Action::SelectAll
                | Action::TransposeUp
                | Action::TransposeDown
                | Action::OctaveUp
                | Action::OctaveDown
                | Action::NudgeLeft
                | Action::NudgeRight
                | Action::LibraryUp
                | Action::LibraryDown
                | Action::LibraryClose
                | Action::LibraryOpen
        )
    }

    /// The chords it has before the user changes them, written as the settings file writes
    /// them. Shift+Del deletes too: of the modifier variants the shortcuts ran before the table
    /// matched chords exactly, the one a hand is likely to have learned (6d review).
    fn defaults(self) -> &'static [&'static str] {
        match self {
            Action::PlayStop => &["Space"],
            Action::ToggleMode => &["L"],
            Action::ArmRecord => &["R"],
            Action::CountIn => &["Ctrl+P"],
            Action::Capture => &["Ctrl+Shift+R"],
            Action::Keyboard => &["Ctrl+T"],
            Action::Undo => &["Ctrl+Z"],
            Action::Redo => &["Ctrl+Shift+Z", "Ctrl+Y"],
            Action::Delete => &["Del", "Shift+Del"],
            Action::Duplicate => &["Ctrl+B", "Ctrl+D"],
            Action::SelectAll => &["Ctrl+A"],
            // The piano roll's keys: the arrows while the roll has the keys; the
            // Library's walk has them while it does (see [`Scope`]).
            Action::TransposeUp => &["Up"],
            Action::TransposeDown => &["Down"],
            Action::OctaveUp => &["Shift+Up"],
            Action::OctaveDown => &["Shift+Down"],
            Action::NudgeLeft => &["Left"],
            Action::NudgeRight => &["Right"],
            Action::Copy => &["Ctrl+C"],
            Action::Cut => &["Ctrl+X"],
            Action::Paste => &["Ctrl+V"],
            Action::Route => &["Ctrl+L"],
            Action::OpenSelected => &["Enter"],
            // The digits follow the panels' order down the window: the rack, the playlist, the
            // piano roll, the mixer. The function keys keep the usual channel-rack places.
            Action::Panel(Panel::Rack) => &["1", "F6"],
            Action::Panel(Panel::Playlist) => &["2", "F5"],
            Action::Panel(Panel::PianoRoll) => &["3", "F7"],
            Action::Panel(Panel::Mixer) => &["4", "F9"],
            Action::Panel(Panel::Browser) => &["5", "F8"],
            Action::Panel(Panel::Settings) => &["6", "F10"],
            // F1, help in most programs; free while the keyboard plays notes.
            Action::Help => &["F1"],
            Action::LibraryUp => &["Up"],
            Action::LibraryDown => &["Down"],
            Action::LibraryClose => &["Left"],
            Action::LibraryOpen => &["Right"],
            Action::New => &["Ctrl+N"],
            Action::Open => &["Ctrl+O"],
            Action::Save => &["Ctrl+S"],
            Action::SaveAs => &["Ctrl+Shift+S"],
        }
    }
}

/// Where an action's keys act. Two panels' actions may share a chord, since only one panel has
/// the keys at a time: the arrows walk the Library while it has them and move the piano roll's
/// notes while the roll has them (the cursor over it with notes selected, or after a click in
/// it). A window action sharing a chord with any other is a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Window,
    Library,
    Roll,
    /// The sample editor's window, while it has the keyboard: Delete deletes its selection,
    /// Select all selects its whole sound and Play / stop plays its selection there
    /// ([`Action::in_sample_editor`]), as the roll's actions win while the roll has the keys.
    SampleEditor,
}

impl Scope {
    /// Whether actions of the two scopes can both want a key at once.
    fn overlaps(self, other: Scope) -> bool {
        self == Scope::Window || other == Scope::Window || self == other
    }
}

/// The named keys a chord can hold, and how each is written.
const NAMED: [(Named, &str); 26] = [
    (Named::Space, "Space"),
    (Named::Enter, "Enter"),
    (Named::Delete, "Del"),
    (Named::Backspace, "Backspace"),
    (Named::Tab, "Tab"),
    (Named::Insert, "Insert"),
    (Named::Home, "Home"),
    (Named::End, "End"),
    (Named::PageUp, "PageUp"),
    (Named::PageDown, "PageDown"),
    (Named::ArrowUp, "Up"),
    (Named::ArrowDown, "Down"),
    (Named::ArrowLeft, "Left"),
    (Named::ArrowRight, "Right"),
    (Named::F1, "F1"),
    (Named::F2, "F2"),
    (Named::F3, "F3"),
    (Named::F4, "F4"),
    (Named::F5, "F5"),
    (Named::F6, "F6"),
    (Named::F7, "F7"),
    (Named::F8, "F8"),
    (Named::F9, "F9"),
    (Named::F10, "F10"),
    (Named::F11, "F11"),
    (Named::F12, "F12"),
];

/// The key of a chord: a character, as the user's keyboard types it without modifiers (an
/// ASCII letter kept in lower case, so it matches either case; any other character as typed, so
/// it reads back as it was written: ß upper-cased is SS), or a named key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChordKey {
    Character(String),
    Named(Named),
}

/// A key with the modifiers held exactly: Ctrl+Shift+Z is not Ctrl+Z.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
    pub key: ChordKey,
}

impl Chord {
    /// The chord of `key` pressed with `modifiers`; none for a key no chord holds (a modifier
    /// alone, Esc, a key with no name here).
    pub fn of(key: Key<&str>, modifiers: Modifiers) -> Option<Self> {
        let key = match key {
            Key::Character(typed) if !typed.trim().is_empty() => {
                ChordKey::Character(typed.to_ascii_lowercase())
            }
            Key::Named(named) => ChordKey::Named(
                NAMED
                    .iter()
                    .find(|(known, _)| *known == named)
                    .map(|&(known, _)| known)?,
            ),
            _ => return None,
        };
        Some(Self {
            ctrl: modifiers.control(),
            alt: modifiers.alt(),
            shift: modifiers.shift(),
            logo: modifiers.logo(),
            key,
        })
    }

    /// Reads a chord as [`Chord::label`] writes it ("Ctrl+Shift+Z"; the modifiers in any order
    /// and case); none for text that is no chord.
    pub fn parse(text: &str) -> Option<Self> {
        let mut chord = Self {
            ctrl: false,
            alt: false,
            shift: false,
            logo: false,
            key: ChordKey::Named(Named::Space),
        };
        let mut rest = text.trim();
        loop {
            let Some((modifier, after)) =
                rest.split_once('+').filter(|(_, after)| !after.is_empty())
            else {
                break;
            };
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" => chord.ctrl = true,
                "alt" => chord.alt = true,
                "shift" => chord.shift = true,
                "super" => chord.logo = true,
                _ => break,
            }
            rest = after;
        }
        chord.key = match NAMED
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(rest))
        {
            Some(&(named, _)) => ChordKey::Named(named),
            None if rest.eq_ignore_ascii_case("Delete") => ChordKey::Named(Named::Delete),
            None if rest.chars().count() == 1 && rest != " " => {
                ChordKey::Character(rest.to_ascii_lowercase())
            }
            None => return None,
        };
        Some(chord)
    }

    /// The chord as people read it and the settings file keeps it: "Ctrl+Shift+Z", "Del", "L".
    pub fn label(&self) -> String {
        let mut words = String::new();
        for (held, name) in [
            (self.ctrl, "Ctrl+"),
            (self.alt, "Alt+"),
            (self.shift, "Shift+"),
            (self.logo, "Super+"),
        ] {
            if held {
                words.push_str(name);
            }
        }
        match &self.key {
            ChordKey::Character(typed) => words.push_str(&typed.to_ascii_uppercase()),
            ChordKey::Named(named) => {
                let name = NAMED
                    .iter()
                    .find(|(known, _)| known == named)
                    .map_or("?", |(_, name)| name);
                words.push_str(name);
            }
        }
        words
    }

    /// Whether Ctrl, Alt or the logo key is held: a chord the computer keyboard never takes.
    pub fn is_command(&self) -> bool {
        self.ctrl || self.alt || self.logo
    }

    /// The same chord with Shift held or not.
    fn shifted(&self, shift: bool) -> Self {
        Self {
            shift,
            ..self.clone()
        }
    }

    /// Where the chord's key lies: where the user's keyboard was seen typing it, else where a
    /// US keyboard has it.
    pub fn code(&self, labels: &Labels) -> Option<Code> {
        match &self.key {
            ChordKey::Named(named) => named_code(*named),
            ChordKey::Character(typed) => labels.code_of(&typed.to_uppercase()),
        }
    }
}

/// Where a named key lies.
fn named_code(named: Named) -> Option<Code> {
    Some(match named {
        Named::Space => Code::Space,
        Named::Enter => Code::Enter,
        Named::Delete => Code::Delete,
        Named::Backspace => Code::Backspace,
        Named::Tab => Code::Tab,
        Named::Insert => Code::Insert,
        Named::Home => Code::Home,
        Named::End => Code::End,
        Named::PageUp => Code::PageUp,
        Named::PageDown => Code::PageDown,
        Named::ArrowUp => Code::ArrowUp,
        Named::ArrowDown => Code::ArrowDown,
        Named::ArrowLeft => Code::ArrowLeft,
        Named::ArrowRight => Code::ArrowRight,
        Named::F1 => Code::F1,
        Named::F2 => Code::F2,
        Named::F3 => Code::F3,
        Named::F4 => Code::F4,
        Named::F5 => Code::F5,
        Named::F6 => Code::F6,
        Named::F7 => Code::F7,
        Named::F8 => Code::F8,
        Named::F9 => Code::F9,
        Named::F10 => Code::F10,
        Named::F11 => Code::F11,
        Named::F12 => Code::F12,
        _ => return None,
    })
}

/// What an action's keys do while the computer keyboard plays notes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhilePlaying {
    /// Its keys work as with the keyboard off.
    Unchanged,
    /// The keys that work instead, some with Shift: "Shift+L".
    Instead(Vec<Chord>),
    /// None of its keys work.
    Suspended,
}

/// The shortcuts: each action's chords, in [`Action::ALL`]'s order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keys {
    bound: Vec<(Action, Vec<Chord>)>,
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            bound: Action::ALL
                .into_iter()
                .map(|action| (action, default_chords(action)))
                .collect(),
        }
    }
}

fn default_chords(action: Action) -> Vec<Chord> {
    action
        .defaults()
        .iter()
        .filter_map(|text| Chord::parse(text))
        .collect()
}

impl Keys {
    /// The table the settings file's `keys` give: each action named there takes the chords
    /// written (several a comma apart), the rest keep their defaults. A name this build does not
    /// know, and a value that holds no chord, are passed over.
    pub fn from_saved(saved: &BTreeMap<String, String>) -> Self {
        let mut keys = Self::default();
        for (id, text) in saved {
            let Some(action) = Action::from_id(id) else {
                continue;
            };
            let chords: Vec<Chord> = split_chords(text).filter_map(Chord::parse).collect();
            if !chords.is_empty() {
                keys.set(action, chords);
            }
        }
        keys
    }

    /// The actions bound otherwise than by default, as the settings file keeps them.
    pub fn saved(&self) -> BTreeMap<String, String> {
        self.bound
            .iter()
            .filter(|(action, chords)| *chords != default_chords(*action))
            .map(|(action, chords)| (action.id().to_owned(), join(chords, ", ")))
            .collect()
    }

    /// The chords that run `action`.
    pub fn chords(&self, action: Action) -> &[Chord] {
        self.bound
            .iter()
            .find(|(bound, _)| *bound == action)
            .map_or(&[], |(_, chords)| chords.as_slice())
    }

    /// The action `chord` runs, a repeat only for an action that repeats; the first listed when
    /// two share it.
    pub fn action(&self, chord: &Chord, repeat: bool) -> Option<Action> {
        self.bound
            .iter()
            .find(|(_, chords)| chords.contains(chord))
            .map(|&(action, _)| action)
            .filter(|action| !repeat || action.repeats())
    }

    /// The action `chord` runs while `scope` has the keys (a repeat only for an action that
    /// repeats): that panel's own first, else a window action; never another panel's.
    pub fn action_in(&self, chord: &Chord, repeat: bool, scope: Scope) -> Option<Action> {
        let bound = |wanted: Scope| {
            self.bound
                .iter()
                .find(|(action, chords)| action.scope() == wanted && chords.contains(chord))
                .map(|&(action, _)| action)
        };
        let own = (scope != Scope::Window).then(|| bound(scope)).flatten();
        own.or_else(|| bound(Scope::Window))
            .filter(|action| !repeat || action.repeats())
    }

    /// The action the key `key` pressed with `modifiers` runs.
    pub fn action_of(&self, key: Key<&str>, modifiers: Modifiers, repeat: bool) -> Option<Action> {
        self.action(&Chord::of(key, modifiers)?, repeat)
    }

    /// `action` runs on `chord` alone from now on.
    pub fn bind(&mut self, action: Action, chord: Chord) {
        self.set(action, vec![chord]);
    }

    fn set(&mut self, action: Action, chords: Vec<Chord>) {
        if let Some((_, bound)) = self.bound.iter_mut().find(|(bound, _)| *bound == action) {
            *bound = chords;
        }
    }

    /// The actions other than `action` that share one of its chords and can want it at the same
    /// time (a window action, or one of the same panel).
    pub fn conflicts(&self, action: Action) -> Vec<Action> {
        let mine = self.chords(action);
        self.bound
            .iter()
            .filter(|(other, chords)| {
                *other != action
                    && action.scope().overlaps(other.scope())
                    && chords.iter().any(|chord| mine.contains(chord))
            })
            .map(|&(other, _)| other)
            .collect()
    }

    /// `action`'s chords as a hint or a menu names them, the first only: "Ctrl+Shift+Z"; "no
    /// key" when it has none.
    pub fn first(&self, action: Action) -> String {
        self.chords(action)
            .first()
            .map_or_else(|| "no key".to_owned(), Chord::label)
    }

    /// Every chord of `action`, `between` apart: "2 or F5".
    pub fn all(&self, action: Action, between: &str) -> String {
        join(self.chords(action), between)
    }

    /// What `action`'s chords do while the keyboard plays `layout`: a chord of Ctrl, Alt or the
    /// logo key, or of a key outside the note area, works as ever; one with Shift on a key of
    /// the note area works too (a Shift chord is its own); a plain key of the note area is
    /// taken, and Shift runs it instead unless Shift with it runs another action. The piano
    /// roll's own actions work as ever: the roll takes its keys back while it has them.
    pub fn while_playing(
        &self,
        action: Action,
        layout: NoteLayout,
        labels: &Labels,
    ) -> WhilePlaying {
        if action.scope() == Scope::Roll {
            return WhilePlaying::Unchanged;
        }
        let chords = self.chords(action);
        let working: Vec<Chord> = chords
            .iter()
            .filter_map(|chord| {
                let claimed = !chord.is_command()
                    && !chord.shift
                    && chord
                        .code(labels)
                        .map_or(layout.claim_unplaced(), |code| layout.claim(code))
                        != Claim::Free;
                if !claimed {
                    return Some(chord.clone());
                }
                let shifted = chord.shifted(true);
                (self.action_in(&shifted, false, action.scope()).is_none()).then_some(shifted)
            })
            .collect();
        if working.is_empty() {
            WhilePlaying::Suspended
        } else if working == chords {
            WhilePlaying::Unchanged
        } else {
            WhilePlaying::Instead(working)
        }
    }
}

/// The chords of a settings value, a comma and a space apart (a lone "," is the comma key).
fn split_chords(text: &str) -> impl Iterator<Item = &str> {
    text.split(", ").filter(|part| !part.trim().is_empty())
}

fn join(chords: &[Chord], between: &str) -> String {
    chords
        .iter()
        .map(Chord::label)
        .collect::<Vec<String>>()
        .join(between)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(text: &str) -> Chord {
        Chord::parse(text).unwrap_or_else(|| panic!("{text:?} is a chord"))
    }

    // Every default the window had before the table, written by hand from its shortcuts: each
    // present, each running its action, and no two actions sharing one. Shift+Del, a default
    // since then, is in the table with them. The digits 2 and 3 were swapped since too (the
    // playlist 2, the piano roll 3, as the panels stack). The piano roll's editing keys came
    // after: Ctrl+D duplicates beside Ctrl+B, Shift+Up/Down move notes an
    // octave, Ctrl+C/X/V copy, cut and paste notes; none was bound before.
    #[test]
    fn every_default_chord_is_present_and_runs_its_action() {
        let keys = Keys::default();
        let table = [
            ("Space", Action::PlayStop),
            ("L", Action::ToggleMode),
            ("R", Action::ArmRecord),
            ("Ctrl+P", Action::CountIn),
            // Recording v2: Capture, Ctrl+Shift+R.
            ("Ctrl+Shift+R", Action::Capture),
            ("Ctrl+T", Action::Keyboard),
            ("Ctrl+Z", Action::Undo),
            ("Ctrl+Shift+Z", Action::Redo),
            ("Ctrl+Y", Action::Redo),
            ("Del", Action::Delete),
            ("Shift+Del", Action::Delete),
            ("Ctrl+B", Action::Duplicate),
            ("Ctrl+A", Action::SelectAll),
            ("Ctrl+L", Action::Route),
            ("Enter", Action::OpenSelected),
            ("1", Action::Panel(Panel::Rack)),
            ("F6", Action::Panel(Panel::Rack)),
            ("2", Action::Panel(Panel::Playlist)),
            ("F5", Action::Panel(Panel::Playlist)),
            ("3", Action::Panel(Panel::PianoRoll)),
            ("F7", Action::Panel(Panel::PianoRoll)),
            ("4", Action::Panel(Panel::Mixer)),
            ("F9", Action::Panel(Panel::Mixer)),
            ("5", Action::Panel(Panel::Browser)),
            ("F8", Action::Panel(Panel::Browser)),
            ("6", Action::Panel(Panel::Settings)),
            ("F10", Action::Panel(Panel::Settings)),
            // Help guide H1: F1 opens the Help window.
            ("F1", Action::Help),
            ("Up", Action::LibraryUp),
            ("Down", Action::LibraryDown),
            ("Left", Action::LibraryClose),
            ("Right", Action::LibraryOpen),
            ("Ctrl+D", Action::Duplicate),
            ("Shift+Up", Action::OctaveUp),
            ("Shift+Down", Action::OctaveDown),
            ("Ctrl+C", Action::Copy),
            ("Ctrl+X", Action::Cut),
            ("Ctrl+V", Action::Paste),
            ("Ctrl+N", Action::New),
            ("Ctrl+O", Action::Open),
            ("Ctrl+S", Action::Save),
            ("Ctrl+Shift+S", Action::SaveAs),
        ];
        for (text, action) in table {
            assert_eq!(keys.action(&chord(text), false), Some(action), "{text}");
        }
        // The arrows are the Library's walk and the piano roll's moves, each while its panel has
        // the keys; while neither has them they name the Library's walk,
        // which then does nothing, as before.
        let in_roll = [
            ("Up", Action::TransposeUp),
            ("Down", Action::TransposeDown),
            ("Left", Action::NudgeLeft),
            ("Right", Action::NudgeRight),
        ];
        for (text, action) in in_roll {
            assert_eq!(
                keys.action_in(&chord(text), false, Scope::Roll),
                Some(action),
                "{text}"
            );
        }
        assert_eq!(
            keys.action_in(&chord("Up"), false, Scope::Library),
            Some(Action::LibraryUp)
        );
        assert_eq!(keys.action_in(&chord("Up"), false, Scope::Window), None);
        assert_eq!(
            keys.action_in(&chord("Ctrl+Z"), false, Scope::Roll),
            Some(Action::Undo),
            "the window's actions work in a panel"
        );
        let bound: usize = Action::ALL
            .iter()
            .map(|&action| keys.chords(action).len())
            .sum();
        assert_eq!(
            bound,
            table.len() + in_roll.len(),
            "no default beside these"
        );
        for action in Action::ALL {
            assert!(keys.conflicts(action).is_empty(), "{action:?}");
            assert_eq!(Action::from_id(action.id()), Some(action));
        }
        assert_eq!(keys.saved(), BTreeMap::new(), "the defaults write nothing");
    }

    // Chords read back as they are written, modifiers in any order and case; the modifiers
    // held must be exactly the chord's; a repeat runs only an action that repeats.
    #[test]
    fn chords_read_back_and_match_exactly() {
        for text in [
            "Ctrl+Shift+Z",
            "Del",
            "L",
            "F10",
            "Ctrl+,",
            ",",
            "Alt+Up",
            "Super+K",
        ] {
            assert_eq!(chord(text).label(), text);
        }
        assert_eq!(chord("shift+ctrl+z"), chord("Ctrl+Shift+Z"));
        assert_eq!(chord("Delete"), chord("Del"));
        for text in ["", "Ctrl+", "Ctrl+Bogus", "Hyper+Z", "  "] {
            assert_eq!(Chord::parse(text), None, "{text:?}");
        }
        let keys = Keys::default();
        let pressed =
            |text: &str, modifiers, repeat| keys.action_of(Key::Character(text), modifiers, repeat);
        assert_eq!(pressed("z", Modifiers::CTRL, false), Some(Action::Undo));
        assert_eq!(
            pressed("Z", Modifiers::CTRL | Modifiers::SHIFT, false),
            Some(Action::Redo)
        );
        assert_eq!(pressed("z", Modifiers::CTRL | Modifiers::ALT, false), None);
        assert_eq!(
            pressed("y", Modifiers::CTRL, true),
            Some(Action::Redo),
            "repeats"
        );
        assert_eq!(pressed("b", Modifiers::CTRL, true), None, "no repeat");
        assert_eq!(pressed("l", Modifiers::empty(), true), None);
        assert_eq!(
            keys.action_of(Key::Named(Named::Escape), Modifiers::empty(), false),
            None,
            "Esc is no action"
        );
    }

    // 6d review 6: a key typing a letter outside ASCII (ß on a German keyboard, é on a French
    // one, ü) is kept as typed: its chord is written as typed, reads back the same and runs
    // its action from what the key types. Only ASCII letters match either case.
    #[test]
    fn chords_on_keys_outside_ascii_are_kept_as_typed() {
        let typed = |text, modifiers| Chord::of(Key::Character(text), modifiers).expect("a chord");
        let mut keys = Keys::default();
        keys.bind(Action::Save, typed("ß", Modifiers::CTRL));
        keys.bind(Action::Open, typed("é", Modifiers::empty()));
        keys.bind(Action::New, typed("ü", Modifiers::ALT));
        let saved = keys.saved();
        assert_eq!(
            saved,
            BTreeMap::from([
                ("new".to_owned(), "Alt+ü".to_owned()),
                ("open".to_owned(), "é".to_owned()),
                ("save".to_owned(), "Ctrl+ß".to_owned()),
            ])
        );
        assert_eq!(Keys::from_saved(&saved), keys);
        for (text, modifiers, action) in [
            ("ß", Modifiers::CTRL, Action::Save),
            ("é", Modifiers::empty(), Action::Open),
            ("ü", Modifiers::ALT, Action::New),
        ] {
            assert_eq!(
                keys.action_of(Key::Character(text), modifiers, false),
                Some(action)
            );
        }
        assert_eq!(
            keys.action_of(Key::Character("É"), Modifiers::empty(), false),
            None
        );
        assert_eq!(
            chord("ctrl+s"),
            chord("Ctrl+S"),
            "ASCII letters, either case"
        );
    }

    // The settings file's `keys`: a rebound action is written by its name, and read back; a
    // name this build does not know and a value that holds no chord are passed over.
    #[test]
    fn rebound_keys_round_trip_through_the_settings_names() {
        let mut keys = Keys::default();
        keys.bind(Action::ToggleMode, chord("Ctrl+M"));
        keys.bind(Action::Panel(Panel::Mixer), chord("M"));
        let saved = keys.saved();
        assert_eq!(
            saved,
            BTreeMap::from([
                ("mode".to_owned(), "Ctrl+M".to_owned()),
                ("mixer".to_owned(), "M".to_owned()),
            ])
        );
        assert_eq!(Keys::from_saved(&saved), keys);
        let mut odd = saved.clone();
        odd.insert("teleport".to_owned(), "Ctrl+K".to_owned());
        odd.insert("undo".to_owned(), "nonsense+".to_owned());
        odd.insert("redo".to_owned(), "Ctrl+R, Ctrl+Shift+R".to_owned());
        let read = Keys::from_saved(&odd);
        assert_eq!(
            read.chords(Action::Undo),
            [chord("Ctrl+Z")],
            "kept its default"
        );
        assert_eq!(read.all(Action::Redo, " or "), "Ctrl+R or Ctrl+Shift+R");
        assert_eq!(read.action(&chord("Ctrl+K"), false), None);
    }

    // The Library's walk and the roll's moves share the arrows without a
    // conflict, as only one panel has the keys at a time; a window action on an arrow is a
    // conflict with both, and runs where neither panel's own action is bound. While the
    // keyboard plays the two rows (which take the arrows), the roll's actions work as ever and
    // the Library's walk moves to Shift, which runs no Library action.
    #[test]
    fn panels_share_the_arrows_and_a_window_action_on_one_conflicts() {
        let mut keys = Keys::default();
        assert!(keys.conflicts(Action::TransposeUp).is_empty());
        assert!(keys.conflicts(Action::LibraryUp).is_empty());
        keys.bind(Action::Save, chord("Up"));
        assert_eq!(
            keys.conflicts(Action::Save),
            [Action::LibraryUp, Action::TransposeUp]
        );
        assert_eq!(keys.conflicts(Action::TransposeUp), [Action::Save]);
        assert_eq!(
            keys.action_in(&chord("Up"), false, Scope::Roll),
            Some(Action::TransposeUp)
        );
        assert_eq!(
            keys.action_in(&chord("Up"), false, Scope::Window),
            Some(Action::Save)
        );
        assert_eq!(
            keys.action_in(&chord("Up"), true, Scope::Roll),
            Some(Action::TransposeUp),
            "an arrow held moves on"
        );
        assert_eq!(keys.action_in(&chord("Ctrl+V"), true, Scope::Roll), None);
        let defaults = Keys::default();
        let labels = Labels::default();
        let rows = |action| defaults.while_playing(action, NoteLayout::TwoRows, &labels);
        assert_eq!(rows(Action::TransposeUp), WhilePlaying::Unchanged);
        assert_eq!(rows(Action::Paste), WhilePlaying::Unchanged);
        assert_eq!(
            rows(Action::LibraryUp),
            WhilePlaying::Instead(vec![chord("Shift+Up")])
        );
        assert!(Action::Delete.edits_notes() && Action::Paste.edits_notes());
        assert!(!Action::LibraryUp.edits_notes() && !Action::Undo.edits_notes());
    }

    // A chord two actions share is a conflict each names; it runs the one listed first.
    #[test]
    fn a_chord_two_actions_share_is_a_conflict_on_both() {
        let mut keys = Keys::default();
        keys.bind(Action::Save, chord("Space"));
        assert_eq!(keys.conflicts(Action::Save), [Action::PlayStop]);
        assert_eq!(keys.conflicts(Action::PlayStop), [Action::Save]);
        assert_eq!(keys.action(&chord("Space"), false), Some(Action::PlayStop));
        keys.bind(Action::Save, chord("Ctrl+S"));
        assert!(keys.conflicts(Action::PlayStop).is_empty());
    }

    // While the piano row plays: L and R are taken and Shift gives them back; Enter, the
    // digits, Delete and the chords work as ever (Enter since its swallowing was reversed,
    // and this expectation moved with it). The two rows take the digits and
    // Delete too.
    #[test]
    fn the_keys_page_says_what_works_while_the_keyboard_plays() {
        let keys = Keys::default();
        let labels = Labels::default();
        let piano = |action| keys.while_playing(action, NoteLayout::Piano, &labels);
        assert_eq!(
            piano(Action::ToggleMode),
            WhilePlaying::Instead(vec![chord("Shift+L")])
        );
        assert_eq!(
            piano(Action::ArmRecord),
            WhilePlaying::Instead(vec![chord("Shift+R")])
        );
        for action in [
            Action::OpenSelected,
            Action::PlayStop,
            Action::Delete,
            Action::Undo,
            Action::Panel(Panel::Mixer),
            Action::LibraryDown,
        ] {
            assert_eq!(piano(action), WhilePlaying::Unchanged, "{action:?}");
        }
        let rows = |action| keys.while_playing(action, NoteLayout::TwoRows, &labels);
        assert_eq!(
            rows(Action::Panel(Panel::Mixer)),
            WhilePlaying::Instead(vec![chord("Shift+4"), chord("F9")])
        );
        assert_eq!(
            rows(Action::Delete),
            WhilePlaying::Instead(vec![chord("Shift+Del")])
        );
        let mut taken = Keys::default();
        taken.bind(Action::Save, chord("Shift+L"));
        assert_eq!(
            taken.while_playing(Action::ToggleMode, NoteLayout::Piano, &labels),
            WhilePlaying::Suspended,
            "Shift+L runs Save"
        );
    }
}
