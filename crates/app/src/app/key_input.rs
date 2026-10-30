//! Every key either window hears, as one message ([`KeyInput`]), and what the app makes of it
//! ([`App::key_message`]): a shortcut, a note or a control of the computer keyboard, Esc, or
//! nothing.
//!
//! While the keyboard plays notes, every key of its layout's note area is the keyboard's: a
//! note, a control (octave, velocity, sustain, all notes off) or nothing at all (a gap such as
//! R, so Record is never armed by a finger reaching for F#). The keys outside it keep their
//! shortcuts (Space, Esc, Delete, the arrows, the function keys and, on the piano row, Enter and
//! the digits 1 to 6), and so does every chord with Ctrl, Alt or the logo
//! key. Shift with a key of the note area runs that key's single-key shortcut, so Shift+L
//! switches pattern and song and Shift+R arms Record. With the keyboard off every key is a
//! shortcut, as before it played notes. A text box open (the tempo's, a name being typed) has
//! the keys typed into it: the window passes on none it took but Esc, a key it did not take
//! plays nothing, and a key let go of closes nothing. Which chord runs which action is the one
//! table [`Keys`], which the Keys page rebinds: a row clicked there takes the next chord
//! pressed.

use iced::keyboard::{self, key, Key, Modifiers};
use iced::{event, window, Event};

use super::{App, FileMessage, Message, RecordMessage};
use crate::keys::{Action, Chord, Claim, Keys, Scope};
use crate::ui::browser::{BrowserMessage, Nav};
use crate::ui::roll::NoteEdit;

/// What the Keys page says while it waits for a key the system gives no name for.
pub const UNBINDABLE: &str = "That key cannot run an action: press another, or Esc";

/// The message `action` sends.
pub fn message_of(action: Action) -> Message {
    let library = |nav| Message::Browser(BrowserMessage::Key(nav));
    match action {
        Action::PlayStop => Message::TogglePlay,
        Action::ToggleMode => Message::ToggleMode,
        Action::ArmRecord => Message::Record(RecordMessage::ToggleArm),
        Action::CountIn => Message::Record(RecordMessage::ToggleCountIn),
        Action::Capture => Message::Record(RecordMessage::Capture),
        Action::Keyboard => Message::ToggleTyping,
        Action::Undo => Message::Undo,
        Action::Redo => Message::Redo,
        Action::Delete => Message::Delete,
        Action::Duplicate => Message::Duplicate,
        Action::SelectAll => Message::SelectAllNotes,
        Action::Route => Message::RouteToSelectedInsert,
        Action::OpenSelected => Message::OpenSelectedChannel,
        Action::Panel(panel) => Message::TogglePanel(panel),
        Action::Help => Message::ToggleHelp,
        Action::LibraryUp => library(Nav::Up),
        Action::LibraryDown => library(Nav::Down),
        Action::LibraryClose => library(Nav::Left),
        Action::LibraryOpen => library(Nav::Right),
        Action::TransposeUp => Message::RollKey(NoteEdit::Transpose(1)),
        Action::TransposeDown => Message::RollKey(NoteEdit::Transpose(-1)),
        Action::OctaveUp => Message::RollKey(NoteEdit::Transpose(12)),
        Action::OctaveDown => Message::RollKey(NoteEdit::Transpose(-12)),
        Action::NudgeLeft => Message::RollKey(NoteEdit::Nudge(-1)),
        Action::NudgeRight => Message::RollKey(NoteEdit::Nudge(1)),
        Action::Copy => Message::RollKey(NoteEdit::Copy),
        Action::Cut => Message::RollKey(NoteEdit::Cut),
        Action::Paste => Message::RollKey(NoteEdit::Paste),
        Action::New => Message::File(FileMessage::New),
        Action::Open => Message::File(FileMessage::Open),
        Action::Save => Message::File(FileMessage::Save),
        Action::SaveAs => Message::File(FileMessage::SaveAs),
    }
}

/// The message `action` sends in the sample editor's window ([`Action::in_sample_editor`]).
fn editor_message(action: Action) -> Message {
    use crate::ui::sample_editor::{EditorOp, SampleEditorMessage as Editor};
    Message::SampleEditor(match action {
        Action::Delete => Editor::Op(EditorOp::Delete),
        Action::SelectAll => Editor::SelectAll,
        _ => Editor::PlayStop,
    })
}

/// What `key` pressed with `modifiers` runs by `keys` while `scope` has the keys: Esc clears
/// the selection (the window's listener names its window first), every other key the action
/// its chord is bound to there ([`Keys::action_in`]), else the first listed, as before panels
/// shared keys (an arrow names the Library's walk, which does nothing while the Library lacks
/// the keys).
pub fn bound_message(
    keys: &Keys,
    key: Key<&str>,
    modifiers: Modifiers,
    repeat: bool,
    scope: Scope,
) -> Option<Message> {
    if key == Key::Named(key::Named::Escape) {
        return Some(Message::ClearSelection);
    }
    let chord = Chord::of(key, modifiers)?;
    keys.action_in(&chord, repeat, scope)
        .or_else(|| keys.action(&chord, repeat))
        .map(message_of)
}

/// Whether `key` is a modifier alone, which a chord being pressed starts with.
fn is_modifier(key: &Key) -> bool {
    use key::Named;
    matches!(
        key,
        Key::Named(
            Named::Shift
                | Named::Control
                | Named::Alt
                | Named::AltGraph
                | Named::Super
                | Named::Meta
                | Named::Hyper
                | Named::Fn
                | Named::CapsLock
                | Named::NumLock
        )
    )
}

/// A key pressed or let go of in one of the app's windows.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyInput {
    pub window: window::Id,
    /// What the key types without modifiers, as the user's layout has it ("l" with Shift held
    /// too), or its name.
    pub key: Key,
    /// Where the key lies on the keyboard; none when the system could not say.
    pub code: Option<key::Code>,
    pub modifiers: Modifiers,
    pub pressed: bool,
    /// A press the system repeats while the key is held.
    pub repeat: bool,
}

/// The windows' keys and the modifiers held, which the tempo and the drags read. A press a text
/// box took means nothing to the app but Esc (it ends a rename too), so only Esc is passed on
/// then; every release is, so a note never outlives its key.
pub(super) fn key_event(
    event: Event,
    status: event::Status,
    window: window::Id,
) -> Option<Message> {
    let Event::Keyboard(event) = event else {
        return None;
    };
    let place = |physical: key::Physical| match physical {
        key::Physical::Code(code) => Some(code),
        key::Physical::Unidentified(_) => None,
    };
    let input = match event {
        keyboard::Event::ModifiersChanged(modifiers) => {
            return Some(Message::ModifiersChanged(modifiers))
        }
        keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            repeat,
            ..
        } => {
            let taken = status == event::Status::Captured;
            if taken && key != Key::Named(key::Named::Escape) {
                return None;
            }
            KeyInput {
                window,
                key,
                code: place(physical_key),
                modifiers,
                pressed: true,
                repeat,
            }
        }
        keyboard::Event::KeyReleased {
            key,
            physical_key,
            modifiers,
            ..
        } => KeyInput {
            window,
            key,
            code: place(physical_key),
            modifiers,
            pressed: false,
            repeat: false,
        },
    };
    Some(Message::Key(input))
}

impl App {
    /// What `input` does now: the message it stands for, or none.
    pub(super) fn key_message(&mut self, input: KeyInput) -> Option<Message> {
        if let (Some(code), Key::Character(typed)) = (input.code, &input.key) {
            self.typing.labels.saw(code, typed);
        }
        if !input.pressed {
            // Every release reaches the keyboard while it plays notes, whatever has the keys.
            return self.typing.on.then_some(Message::TypingKey {
                code: input.code?,
                pressed: false,
                repeat: false,
            });
        }
        // The Keys page is the Settings window's: a key typed in another window does what it
        // does there, and the row waits on (it was the main window's, while
        // Settings was a panel of it).
        let on_page = self.settings_window == Some(input.window);
        if let Some(action) = self.rebinding.filter(|_| on_page) {
            self.rebind(action, &input);
            return None;
        }
        if input.key == Key::Named(key::Named::Escape) {
            return Some(Message::Escape(input.window));
        }
        let modifiers = input.modifiers;
        let chord = modifiers.control() || modifiers.alt() || modifiers.logo();
        let scope = if self.editor.window == Some(input.window) {
            Scope::SampleEditor
        } else {
            self.key_scope()
        };
        // The sample editor's keys win in its window, over the typing keyboard's notes too, as
        // the roll's actions win while it has the keys.
        if scope == Scope::SampleEditor {
            let action = Chord::of(input.key.as_ref(), modifiers)
                .and_then(|chord| self.keys.action_in(&chord, input.repeat, scope))
                .filter(|action| action.in_sample_editor());
            if let Some(action) = action {
                return Some(editor_message(action));
            }
        }
        // A text box open has the keys typed: no key edits the roll's
        // notes then, whatever path it would take.
        if scope == Scope::Roll && (self.text_box_open() || self.search_typing) {
            let edits = Chord::of(input.key.as_ref(), modifiers)
                .and_then(|chord| self.keys.action_in(&chord, input.repeat, Scope::Roll))
                .is_some_and(Action::edits_notes);
            if edits {
                return None;
            }
        }
        if self.typing.on && !chord {
            let layout = self.typing.layout;
            let claim = input
                .code
                .map_or(layout.claim_unplaced(), |code| layout.claim(code));
            if claim != Claim::Free {
                // The piano roll takes its keys back from the layout while it has them (an
                // arrow or Delete the two rows swallow; a note key bound to a roll action).
                let roll_key = Chord::of(input.key.as_ref(), modifiers)
                    .filter(|_| scope == Scope::Roll)
                    .and_then(|chord| self.keys.action_in(&chord, input.repeat, Scope::Roll))
                    .filter(|action| action.edits_notes());
                if let Some(action) = roll_key {
                    return Some(message_of(action));
                }
                let bound = |modifiers| {
                    let chord = Chord::of(input.key.as_ref(), modifiers)?;
                    self.keys
                        .action_in(&chord, input.repeat, scope)
                        .map(message_of)
                };
                // Shift+key: a shortcut of its own, else the one the keyboard took the key from.
                let shifted = modifiers
                    .shift()
                    .then(|| bound(modifiers).or_else(|| bound(Modifiers::empty())))
                    .flatten();
                if shifted.is_some() {
                    return shifted;
                }
                // A text box open has the keys typed: one it did not take (Tab, a key typed
                // after a click took its focus) plays nothing and leaves it open.
                if self.text_box_open() {
                    return None;
                }
                // Said in the hint bar, so a key taken from its shortcut is never taken quietly.
                self.typing.stolen = (!modifiers.shift() && !input.repeat)
                    .then(|| {
                        let action = self.keys.action_of(input.key.as_ref(), modifiers, false)?;
                        Some((input.code?, action))
                    })
                    .flatten();
                return input.code.map(|code| Message::TypingKey {
                    code,
                    pressed: true,
                    repeat: input.repeat,
                });
            }
        }
        bound_message(
            &self.keys,
            input.key.as_ref(),
            modifiers,
            input.repeat,
            scope,
        )
    }

    /// Which panel's keys run now: the piano roll's while it is open and has the keys (it was
    /// the last panel pressed in, or the cursor is over it with notes selected), else the
    /// Library's while it was the last pressed in, else the window's alone.
    pub(super) fn key_scope(&self) -> Scope {
        if self.roll_has_keys() {
            Scope::Roll
        } else if self.browser_has_keys() {
            Scope::Library
        } else {
            Scope::Window
        }
    }

    /// Whether one of the window's text boxes is open: the tempo's, or a pattern's, a
    /// channel's, an insert's or a track's name being typed. Each takes the keys as it opens.
    fn text_box_open(&self) -> bool {
        self.tempo_edit.is_some()
            || self.patterns.rename.is_some()
            || self.channel_rename.is_some()
            || self.mixer.rename.is_some()
            || self.arranger.rename.is_some()
    }

    /// The Keys page waits for `action`'s new chord: Esc gives up, a modifier alone is part of
    /// a chord being pressed, any other key is the chord (one the system gives no name for is
    /// refused, and the page waits on).
    fn rebind(&mut self, action: Action, input: &KeyInput) {
        if input.key == Key::Named(key::Named::Escape) {
            self.rebinding = None;
            return;
        }
        if is_modifier(&input.key) {
            return;
        }
        match Chord::of(input.key.as_ref(), input.modifiers) {
            Some(chord) => {
                self.rebinding = None;
                self.keys.bind(action, chord);
                self.keys_changed();
            }
            None => self.notice = Some(UNBINDABLE.to_owned()),
        }
    }

    /// The shortcuts changed: kept in the settings, which are saved.
    pub(super) fn keys_changed(&mut self) {
        self.settings.set_keys(&self.keys);
        self.save_settings();
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use dawsome_engine::transport::Control;
    use iced::keyboard::key::{Code, Named};

    use super::super::tests::app_on;
    use super::super::{Panel, PlayMode};
    use super::*;
    use crate::keys::Keys;
    use crate::ui::browser::{BrowserMessage, Nav};

    const F1: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/f1_four_on_floor.json"
    );

    /// A key of `code` typing `typed` (a letter, or a named key), pressed or let go of with
    /// `modifiers` held, in a window of the app.
    fn input(typed: Key, code: Code, modifiers: Modifiers, pressed: bool) -> KeyInput {
        KeyInput {
            window: window::Id::unique(),
            key: typed,
            code: Some(code),
            modifiers,
            pressed,
            repeat: false,
        }
    }

    fn letter(text: &str) -> Key {
        Key::Character(text.into())
    }

    /// Presses and lets go of the key, through the window's own path.
    fn tap(app: &mut App, typed: Key, code: Code, modifiers: Modifiers) {
        for pressed in [true, false] {
            let _ = app.update(Message::Key(input(typed.clone(), code, modifiers, pressed)));
        }
    }

    /// The live notes sent since `from`.
    fn notes_from(app: &App, from: usize) -> Vec<Control> {
        app.session.controls_sent()[from..]
            .iter()
            .filter(|control| {
                matches!(
                    control,
                    Control::LiveNoteOn { .. } | Control::LiveNoteOff { .. }
                )
            })
            .cloned()
            .collect()
    }

    // Found in use: Esc typed in a window is that window's Esc, through the
    // windows' own key path. In the main window it closes the picker (its Esc chain runs, as
    // Message::Escape's does); in the instrument window it closes that window. (A typed Esc
    // became the window's Esc message, which nothing then sorted out: the instrument window
    // stayed open, and the main window's chain ran for none of its steps.)
    #[test]
    fn a_typed_esc_is_its_windows_esc() {
        use dawsome_engine::project::ChannelId;
        let (mut app, _player) = app_on(Path::new(F1));
        let main = window::Id::unique();
        app.set_main_window(main);
        let esc = |app: &mut App, id| {
            let typed = input(
                Key::Named(Named::Escape),
                Code::Escape,
                Modifiers::empty(),
                true,
            );
            let _ = app.update(Message::Key(KeyInput {
                window: id,
                ..typed
            }));
        };
        let _ = app.update(Message::OpenPicker(
            crate::ui::picker::PickerTarget::AddInstrument,
        ));
        assert!(app.picker.is_some());
        esc(&mut app, main);
        assert!(
            app.picker.is_none(),
            "Esc in the main window closes its picker"
        );
        let _ = app.update(Message::OpenInstrument(ChannelId(1)));
        let instrument = app.instrument_window.expect("open");
        esc(&mut app, instrument);
        assert_eq!(
            app.instrument_window, None,
            "Esc in it closes the instrument window"
        );
    }

    // While the keyboard plays notes (as a new window does), L is the piano row's D an octave
    // up (60 + 14 = 74) and never switches the mode; Shift+L switches it and plays nothing;
    // with the keyboard off, L switches it as before.
    #[test]
    fn a_typed_l_plays_a_note_and_never_switches_the_mode_while_the_keyboard_plays() {
        let (mut app, _player) = app_on(Path::new(F1));
        assert!(app.typing.on);
        let sent = app.session.controls_sent().len();
        tap(&mut app, letter("l"), Code::KeyL, Modifiers::empty());
        assert_eq!(app.mode, PlayMode::Pattern, "L plays, it does not switch");
        let velocity = super::super::typing::VELOCITY;
        assert_eq!(
            notes_from(&app, sent),
            [
                Control::LiveNoteOn { key: 74, velocity },
                Control::LiveNoteOff { key: 74 }
            ]
        );
        let sent = app.session.controls_sent().len();
        tap(&mut app, letter("l"), Code::KeyL, Modifiers::SHIFT);
        assert_eq!(app.mode, PlayMode::Song, "Shift+L switches");
        assert!(notes_from(&app, sent).is_empty(), "and plays nothing");
        let _ = app.update(Message::ToggleTyping);
        let sent = app.session.controls_sent().len();
        tap(&mut app, letter("l"), Code::KeyL, Modifiers::empty());
        assert_eq!(app.mode, PlayMode::Pattern, "off, L switches as before");
        assert!(notes_from(&app, sent).is_empty());
    }

    // R lies between E and T, a gap of the black row: while the keyboard plays notes it never
    // arms Record and plays nothing; Shift+R arms it; with the keyboard off R arms it as before.
    #[test]
    fn r_never_arms_record_while_the_keyboard_plays() {
        let (mut app, _player) = app_on(Path::new(F1));
        let sent = app.session.controls_sent().len();
        tap(&mut app, letter("r"), Code::KeyR, Modifiers::empty());
        assert!(!app.recording.armed);
        assert!(notes_from(&app, sent).is_empty(), "a gap plays nothing");
        tap(&mut app, letter("r"), Code::KeyR, Modifiers::SHIFT);
        assert!(app.recording.armed, "Shift+R arms Record");
        let _ = app.update(Message::ToggleTyping);
        tap(&mut app, letter("r"), Code::KeyR, Modifiers::empty());
        assert!(!app.recording.armed, "off, R switches it as before");
    }

    // Outside the note area the shortcuts work while the keyboard plays notes: Space, Delete,
    // the arrows, the function keys, the digits 1 to 6 and every Ctrl chord; Esc names its
    // window; a chord with a note key is its shortcut, not the note. Enter, beside the last
    // white key, opens or adds what is selected and plays nothing (its swallowing was
    // reversed, and this expectation moved with it).
    #[test]
    fn the_keys_outside_the_notes_keep_their_shortcuts_while_the_keyboard_plays() {
        let (mut app, _player) = app_on(Path::new(F1));
        let none = Modifiers::empty();
        let mut press = |typed: Key, code: Code, modifiers: Modifiers| {
            app.key_message(input(typed, code, modifiers, true))
        };
        assert!(matches!(
            press(Key::Named(Named::Space), Code::Space, none),
            Some(Message::TogglePlay)
        ));
        assert!(matches!(
            press(Key::Named(Named::Delete), Code::Delete, none),
            Some(Message::Delete)
        ));
        assert!(matches!(
            press(Key::Named(Named::ArrowDown), Code::ArrowDown, none),
            Some(Message::Browser(BrowserMessage::Key(Nav::Down)))
        ));
        assert!(matches!(
            press(Key::Named(Named::F5), Code::F5, none),
            Some(Message::TogglePanel(Panel::Playlist))
        ));
        for (digit, code, panel) in [
            ("1", Code::Digit1, Panel::Rack),
            ("6", Code::Digit6, Panel::Settings),
        ] {
            assert!(matches!(
                press(letter(digit), code, none),
                Some(Message::TogglePanel(toggled)) if toggled == panel
            ));
        }
        assert!(matches!(
            press(letter("z"), Code::KeyZ, Modifiers::CTRL),
            Some(Message::Undo)
        ));
        assert!(matches!(
            press(letter("l"), Code::KeyL, Modifiers::CTRL),
            Some(Message::RouteToSelectedInsert)
        ));
        assert!(matches!(
            press(letter("t"), Code::KeyT, Modifiers::CTRL),
            Some(Message::ToggleTyping)
        ));
        assert!(matches!(
            press(Key::Named(Named::Escape), Code::Escape, none),
            Some(Message::Escape(_))
        ));
        assert!(matches!(
            press(Key::Named(Named::Enter), Code::Enter, none),
            Some(Message::OpenSelectedChannel)
        ));
        let sent = app.session.controls_sent().len();
        let _ = app.update(Message::Key(input(
            Key::Named(Named::Enter),
            Code::Enter,
            none,
            true,
        )));
        assert!(notes_from(&app, sent).is_empty(), "Enter plays nothing");
    }

    // 6d review 10: Shift+Delete deletes the selection, a second default chord (a habit the
    // literal matches allowed), with the keyboard playing notes and with it off; chords still
    // match exactly, so Ctrl+Alt+Z undoes nothing either way.
    #[test]
    fn shift_delete_deletes_and_other_chords_match_exactly() {
        let (mut app, _player) = app_on(Path::new(F1));
        for on in [true, false] {
            if app.typing.on != on {
                let _ = app.update(Message::ToggleTyping);
            }
            let delete = input(
                Key::Named(Named::Delete),
                Code::Delete,
                Modifiers::SHIFT,
                true,
            );
            assert!(
                matches!(app.key_message(delete), Some(Message::Delete)),
                "keyboard on: {on}"
            );
            let undo = input(
                letter("z"),
                Code::KeyZ,
                Modifiers::CTRL | Modifiers::ALT,
                true,
            );
            assert!(app.key_message(undo).is_none(), "keyboard on: {on}");
        }
    }

    // A held key repeats nothing; its release ends its note even when a dialog is up, and so
    // does losing the window's keys.
    #[test]
    fn a_release_always_ends_the_note_and_a_repeat_plays_nothing() {
        let (mut app, _player) = app_on(Path::new(F1));
        let none = Modifiers::empty();
        let sent = app.session.controls_sent().len();
        let _ = app.update(Message::Key(input(letter("a"), Code::KeyA, none, true)));
        let mut again = input(letter("a"), Code::KeyA, none, true);
        again.repeat = true;
        let _ = app.update(Message::Key(again));
        app.files.modal = Some(super::super::files::Modal::Unsaved);
        let _ = app.update(Message::Key(input(letter("a"), Code::KeyA, none, false)));
        let velocity = super::super::typing::VELOCITY;
        assert_eq!(
            notes_from(&app, sent),
            [
                Control::LiveNoteOn { key: 60, velocity },
                Control::LiveNoteOff { key: 60 }
            ]
        );
        app.files.modal = None;
        let _ = app.update(Message::Key(input(letter("s"), Code::KeyS, none, true)));
        let sent = app.session.controls_sent().len();
        let _ = app.update(Message::WindowUnfocused);
        assert_eq!(app.session.controls_sent()[sent..], [Control::LiveNotesOff]);
    }

    // A key is named by what the user's keyboard types on it: a French keyboard types q on the
    // US A, so the hint bar says Q plays C5.
    #[test]
    fn the_hint_names_the_keys_as_the_users_keyboard_types_them() {
        let (mut app, _player) = app_on(Path::new(F1));
        let none = Modifiers::empty();
        let _ = app.update(Message::Key(input(letter("q"), Code::KeyA, none, true)));
        let _ = app.update(Message::Key(input(letter("q"), Code::KeyA, none, false)));
        assert!(
            app.hint_words()
                .starts_with("Keyboard plays notes: Q is C5"),
            "{}",
            app.hint_words()
        );
    }

    // 6d review 1: while the keyboard plays notes (as a first start has it), the keys typed into
    // a text box are the box's, and letting go of them closes nothing: "90" lands in the
    // tempo's box and Enter sets 90 BPM, "Ve" in the pattern's name, "Ki" in the channel's,
    // each kept by Enter, "ki" in the Library's search. Tab, the sustain key, which a box does
    // not take, leaves the box open. No note plays. Through the window's own widgets and key
    // listener.
    #[test]
    fn the_keys_typed_into_a_text_box_are_the_boxs_while_the_keyboard_plays() {
        use super::super::widget_events::Window;
        use super::super::{
            ChannelCommand, PatternMessage, CHANNEL_RENAME_INPUT, PATTERN_RENAME_INPUT, TEMPO_INPUT,
        };
        use dawsome_engine::project::{ChannelId, PatternId};
        use dawsome_engine::time::MilliBpm;
        let (mut app, _player) = app_on(Path::new(F1));
        assert!(app.typing.on);
        let mut window = Window::new(&app);
        let sent = app.session.controls_sent().len();
        let _ = app.update(Message::TempoPressed);
        let _ = app.update(Message::DragEnded);
        window.focus(&app, TEMPO_INPUT);
        window.type_keys(&mut app, &[('9', Code::Digit9)]);
        assert_eq!(app.tempo_edit.as_deref(), Some("9"));
        window.tap(&mut app, Named::Tab, Code::Tab);
        window.type_keys(&mut app, &[('0', Code::Digit0)]);
        assert_eq!(app.tempo_edit.as_deref(), Some("90"));
        window.tap(&mut app, Named::Enter, Code::Enter);
        assert_eq!(app.tempo_edit, None);
        assert_eq!(app.session.project().tempo, MilliBpm(90_000));

        let _ = app.update(Message::Pattern(PatternMessage::Rename));
        window.focus(&app, PATTERN_RENAME_INPUT);
        window.type_keys(&mut app, &[('V', Code::KeyV), ('e', Code::KeyE)]);
        let typed = |app: &App| app.patterns.rename.as_ref().map(|(_, typed)| typed.clone());
        assert_eq!(typed(&app).as_deref(), Some("Ve"));
        window.tap(&mut app, Named::Enter, Code::Enter);
        assert_eq!(typed(&app), None);
        assert_eq!(app.session.project().patterns[&PatternId(1)].name, "Ve");

        let _ = app.update(Message::Channel(ChannelCommand::Rename(ChannelId(1))));
        window.focus(&app, CHANNEL_RENAME_INPUT);
        window.type_keys(&mut app, &[('K', Code::KeyK), ('i', Code::KeyI)]);
        let typed = |app: &App| app.channel_rename.as_ref().map(|(_, typed)| typed.clone());
        assert_eq!(typed(&app).as_deref(), Some("Ki"));
        window.tap(&mut app, Named::Enter, Code::Enter);
        assert_eq!(typed(&app), None);
        assert_eq!(app.session.project().channels[&ChannelId(1)].name, "Ki");

        let _ = app.update(Message::TogglePanel(Panel::Browser));
        window.focus(&app, crate::ui::browser::SEARCH_INPUT);
        window.type_keys(&mut app, &[('k', Code::KeyK), ('i', Code::KeyI)]);
        assert_eq!(app.browse.model.search, "ki");
        assert!(notes_from(&app, sent).is_empty(), "nothing played");
    }

    /// F1 with a settings file of its own in a temp folder, saved as the window saves it.
    fn saving_app(test: &str) -> (App, dawsome_engine::realtime::Player, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("dawsome-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut host = crate::document::Host::offline();
        host.settings_file = Some(dir.join("dawsome/settings.json"));
        host.saves_settings = true;
        let project = crate::project_file::open(Path::new(F1))
            .expect("opens")
            .project;
        let (app, player) = super::super::tests::app_hosted(project, Path::new(F1), host);
        (app, player, dir)
    }

    fn saved(dir: &Path) -> serde_json::Value {
        let text = std::fs::read_to_string(dir.join("dawsome/settings.json")).expect("saved");
        serde_json::from_str(&text).expect("JSON")
    }

    /// Opens the Settings window on its Keys page through its own tab; its window's id.
    fn open_keys_page(app: &mut App) -> window::Id {
        use crate::ui::settings_layout::{tab, SettingsPage};
        use crate::ui::settings_panel::SettingsMessage;
        let _ = app.update(Message::TogglePanel(Panel::Settings));
        let at = tab(SettingsPage::Keys).center();
        let _ = app.update(Message::Settings(SettingsMessage::Pressed(at)));
        assert_eq!(app.settings_page, SettingsPage::Keys);
        app.settings_window.expect("the Settings window opened")
    }

    /// `input` typed in the window `id`.
    fn typed_in(id: window::Id, mut input: KeyInput) -> KeyInput {
        input.window = id;
        input
    }

    fn press_row(app: &mut App, action: Action) {
        use crate::ui::keys_page::table_row;
        use crate::ui::settings_panel::SettingsMessage;
        let index = Action::ALL
            .iter()
            .position(|&listed| listed == action)
            .expect("listed");
        let at = table_row(index).center();
        let _ = app.update(Message::Settings(SettingsMessage::KeysPressed(at)));
    }

    // 6d K3, through the window's own key events: a press on Pattern / song mode's row waits;
    // Shift and Ctrl alone wait on; a key the system names nothing for is refused; Ctrl+M is
    // the new chord, saved by its name; with the keyboard off L then does nothing and Ctrl+M
    // switches the mode. A row waiting gives up on Esc and keeps its keys, and the window stays
    // open. The keys are typed in the Settings window, where the page now is.
    #[test]
    fn a_row_pressed_takes_the_next_chord_and_keeps_it() {
        let (mut app, _player, dir) = saving_app("rebind");
        let settings = open_keys_page(&mut app);
        press_row(&mut app, Action::ToggleMode);
        assert_eq!(app.rebinding, Some(Action::ToggleMode));
        let none = Modifiers::empty();
        let send = |app: &mut App, typed: Key, code: Code, modifiers: Modifiers| {
            let pressed = typed_in(settings, input(typed, code, modifiers, true));
            let _ = app.update(Message::Key(pressed));
        };
        send(
            &mut app,
            Key::Named(Named::Shift),
            Code::ShiftLeft,
            Modifiers::SHIFT,
        );
        send(
            &mut app,
            Key::Named(Named::Control),
            Code::ControlLeft,
            Modifiers::CTRL,
        );
        assert_eq!(
            app.rebinding,
            Some(Action::ToggleMode),
            "a modifier alone waits"
        );
        send(&mut app, Key::Named(Named::Pause), Code::Pause, none);
        assert_eq!(app.notice.as_deref(), Some(UNBINDABLE));
        assert_eq!(app.rebinding, Some(Action::ToggleMode));
        send(&mut app, letter("m"), Code::KeyM, Modifiers::CTRL);
        assert_eq!(app.rebinding, None);
        assert_eq!(app.keys.first(Action::ToggleMode), "Ctrl+M");
        assert_eq!(saved(&dir)["keys"], serde_json::json!({"mode": "Ctrl+M"}));
        let _ = app.update(Message::ToggleTyping);
        tap(&mut app, letter("l"), Code::KeyL, none);
        assert_eq!(app.mode, PlayMode::Pattern, "L runs nothing now");
        tap(&mut app, letter("m"), Code::KeyM, Modifiers::CTRL);
        assert_eq!(app.mode, PlayMode::Song);
        press_row(&mut app, Action::ArmRecord);
        send(&mut app, Key::Named(Named::Escape), Code::Escape, none);
        assert_eq!(app.rebinding, None);
        assert_eq!(app.keys.first(Action::ArmRecord), "R", "Esc gives up");
        assert!(app.panels.settings, "and closes nothing");
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // A row waiting for its chord gives up once its page is not shown. Settings is a window
    // of its own, so opening the piano roll and the playlist in a
    // 1000 x 560 window no longer closes it to make room, and the row waits on; its window's X
    // closes it, and the row gives up. Space then plays, as ever, and binds nothing.
    #[test]
    fn a_waiting_row_gives_up_when_the_settings_window_closes() {
        let (mut app, _player) = app_on(Path::new(F1));
        let settings = open_keys_page(&mut app);
        press_row(&mut app, Action::Save);
        assert_eq!(app.rebinding, Some(Action::Save));
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        assert!(app.panels.settings, "no panel's room closes it");
        assert_eq!(app.notice, None);
        assert_eq!(app.rebinding, Some(Action::Save));
        let _ = app.update(Message::WindowCloseRequested(settings));
        assert!(!app.panels.settings && app.settings_window.is_none());
        assert_eq!(app.rebinding, None);
        let space = input(
            Key::Named(Named::Space),
            Code::Space,
            Modifiers::empty(),
            true,
        );
        assert!(matches!(app.key_message(space), Some(Message::TogglePlay)));
        assert_eq!(app.keys, Keys::default());
    }

    // A row waiting for its chord takes it from the
    // window its page is in alone (the Settings window; it was the main
    // window). Ctrl+M pressed in the instrument window or in the main window binds nothing and
    // the row waits on; pressed in the Settings window, it is Save's chord.
    #[test]
    fn a_waiting_row_takes_keys_from_the_settings_window_alone() {
        use dawsome_engine::project::ChannelId;
        let (mut app, _player) = app_on(Path::new(F1));
        let main = window::Id::unique();
        app.set_main_window(main);
        let _ = app.update(Message::OpenInstrument(ChannelId(1)));
        let instrument = app.instrument_window.expect("open");
        let settings = open_keys_page(&mut app);
        press_row(&mut app, Action::Save);
        let chord = input(letter("m"), Code::KeyM, Modifiers::CTRL, true);
        for other in [instrument, main] {
            let _ = app.update(Message::Key(typed_in(other, chord.clone())));
            assert_eq!(app.rebinding, Some(Action::Save), "{other:?}");
            assert_eq!(app.keys.first(Action::Save), "Ctrl+S");
        }
        let _ = app.update(Message::Key(typed_in(settings, chord)));
        assert_eq!(app.rebinding, None);
        assert_eq!(app.keys.first(Action::Save), "Ctrl+M");
    }

    // The hints, menus and tips name the keys as bound: Record's hint after R becomes
    // Ctrl+R, the Undo row's after Undo becomes Ctrl+U, the playlist's tip after it becomes
    // F4; the Keys page's Reset gives the defaults back and writes no keys; its layout chooser
    // plays the two rows from Z on C4 and keeps the choice.
    #[test]
    fn the_hints_name_the_bound_keys_and_the_page_resets_and_chooses_the_layout() {
        use crate::ui::hint::{words, EditRow, Hover, MenuRow, Tool};
        use crate::ui::keys_page::{hit, layout_row, reset_button, KeysHit};
        use crate::ui::settings_panel::SettingsMessage;
        let (mut app, _player, dir) = saving_app("rebind-hints");
        let settings = open_keys_page(&mut app);
        for (action, typed, code) in [
            (Action::ArmRecord, "r", Code::KeyR),
            (Action::Undo, "u", Code::KeyU),
        ] {
            press_row(&mut app, action);
            let chord = input(letter(typed), code, Modifiers::CTRL, true);
            let _ = app.update(Message::Key(typed_in(settings, chord)));
        }
        press_row(&mut app, Action::Panel(Panel::Playlist));
        let f4 = input(Key::Named(Named::F4), Code::F4, Modifiers::empty(), true);
        let _ = app.update(Message::Key(typed_in(settings, f4)));
        let context = app.hint_context();
        assert!(words(&Hover::Tool(Tool::Record), &context).contains("click or Ctrl+R arms it"));
        assert!(words(&Hover::Menu(MenuRow::Edit(EditRow::Undo)), &context).ends_with("; Ctrl+U"));
        assert_eq!(
            crate::ui::toolbar::panel_tip(Panel::Playlist, &app.keys),
            "Playlist \u{b7} F4"
        );
        let reset = reset_button().center();
        assert_eq!(hit(reset), Some(KeysHit::Reset));
        let _ = app.update(Message::Settings(SettingsMessage::KeysPressed(reset)));
        assert_eq!(app.keys, Keys::default());
        assert_eq!(saved(&dir).get("keys"), None, "the defaults write nothing");
        let two_rows = layout_row(crate::keys::NoteLayout::TwoRows).center();
        let _ = app.update(Message::Settings(SettingsMessage::KeysPressed(two_rows)));
        assert_eq!(app.typing.layout, crate::keys::NoteLayout::TwoRows);
        assert_eq!(app.typing.base, 48);
        assert_eq!(saved(&dir)["note_layout"], "two-rows");
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // A key taken from its shortcut is said in the hint bar: L plays D6 now and Shift+L
    // switches; a key of the notes with no shortcut says the keyboard's line again. The
    // toolbar switch's hint says it of the first shortcut taken.
    #[test]
    fn a_key_taken_from_its_shortcut_says_so_in_the_hint_bar() {
        use crate::ui::hint::{words, Hover, Tool};
        let (mut app, _player) = app_on(Path::new(F1));
        let none = Modifiers::empty();
        tap(&mut app, letter("l"), Code::KeyL, none);
        assert_eq!(
            app.hint_words(),
            "L plays D6 now; Shift+L switches pattern/song"
        );
        tap(&mut app, letter("r"), Code::KeyR, none);
        assert_eq!(app.hint_words(), "R plays nothing now; Shift+R arms Record");
        tap(&mut app, letter("a"), Code::KeyA, none);
        assert!(app
            .hint_words()
            .starts_with("Keyboard plays notes: A is C5"));
        let switch = words(&Hover::Tool(Tool::Keys), &app.hint_context());
        assert!(
            switch.contains("A is C5, velocity 100; L plays D6 now; Shift+L switches pattern/song"),
            "{switch}"
        );
    }
}
