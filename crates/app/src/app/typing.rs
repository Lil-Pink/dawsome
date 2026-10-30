//! The computer keyboard as an instrument: while it plays notes (on unless switched off with
//! Ctrl+T or the toolbar's switch, a setting kept for the next start), the keys of the note
//! layout Settings chooses ([`NoteLayout`]: the piano row, or the two rows it had before) play
//! the selected channel live (the Library's preview while there is one, else the instrument
//! window's while it is open), by the keys' physical places so any layout plays the same notes.
//! A key held down repeats nothing; letting it go ends its note, unless the sustain key (Tab)
//! is held, which holds the notes let go of until it is let go of too: the engine's sustain pedal
//! (the typing keyboard's own, as a device's CC 64 is that device's), so a take, the roll and the
//! player agree on where a held note ends. The piano row's Z and X move the octave, C and V step
//! the velocity, Backspace ends every note.

use dawsome_engine::realtime::LiveControl;
use dawsome_engine::transport::Control;
use iced::keyboard::key::Code;

use super::App;
use crate::keys::{
    self, Action, Chord, Claim, KeyControl, Keys, Labels, NoteLayout, DEFAULT_VELOCITY,
};
use crate::midi::VELOCITY_SCALE;

/// The velocity typed notes play at first, as the player takes it: 100/128, a drawn note's.
#[cfg(test)]
pub const VELOCITY: f32 = DEFAULT_VELOCITY as f32 / VELOCITY_SCALE;

/// The lowest and highest key a layout's first note can move to: an octave name from "C2" to
/// "C8".
pub const LOWEST_BASE: u8 = 24;
pub const HIGHEST_BASE: u8 = 96;

const OCTAVE: u8 = 12;

/// "C4" for the octave starting on key 48: the name of its first key, a C.
pub fn octave_name(base: u8) -> String {
    crate::ui::key_names::key_name(base)
}

/// The keyboard's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Typing {
    pub on: bool,
    pub layout: NoteLayout,
    /// The key the layout's first note plays: a C from [`LOWEST_BASE`] to [`HIGHEST_BASE`].
    pub base: u8,
    /// The velocity typed notes play at, on the 0..=127 scale: one of the velocity keys' seven
    /// steps.
    pub velocity: u8,
    /// What the user's keyboard types on each key, as seen so far.
    pub labels: Labels,
    /// The keys held down and the note each started, so a release ends the note it played even
    /// after the octave moved.
    held: Vec<(Code, u8)>,
    /// Whether the sustain key is held down: the engine's pedal is down for the keyboard.
    sustain: bool,
    /// The key last pressed among the notes whose shortcut the keyboard took, and that
    /// shortcut: the hint bar says so until a key of the notes with none is pressed.
    pub stolen: Option<(Code, Action)>,
}

impl Typing {
    /// The keyboard as a window opens it: on or off, playing `layout` from its first key.
    pub fn new(on: bool, layout: NoteLayout) -> Self {
        Self {
            on,
            layout,
            base: layout.start(),
            velocity: DEFAULT_VELOCITY,
            labels: Labels::default(),
            held: Vec::new(),
            sustain: false,
            stolen: None,
        }
    }

    /// The velocity typed notes play at, as the player takes it.
    fn note_velocity(&self) -> f32 {
        f32::from(self.velocity) / VELOCITY_SCALE
    }

    /// What the hint bar says while the keyboard plays notes and the cursor is over nothing:
    /// what the key just pressed took from its shortcut, else which key plays which note at
    /// which velocity, and how to switch it off.
    pub fn summary(&self, keys: &Keys) -> String {
        if let Some((code, action)) = self.stolen {
            return self.taken_words(code, action, keys);
        }
        let label = |code: Code| self.labels.label(code);
        let off = keys.first(Action::Keyboard);
        match self.layout {
            NoteLayout::Piano => format!(
                "Keyboard plays notes: {} is {}, velocity {}; {} {} octave, {} {} velocity, Tab sustain, Backspace all notes off; {off} switches it off",
                label(Code::KeyA),
                octave_name(self.base),
                self.velocity,
                label(Code::KeyZ),
                label(Code::KeyX),
                label(Code::KeyC),
                label(Code::KeyV),
            ),
            NoteLayout::TwoRows => format!(
                "Typing keyboard on: the letter keys play notes, {} is {}; {off} switches it off",
                label(Code::KeyZ),
                octave_name(self.base)
            ),
        }
    }

    /// The first-run note's first line, as the layout plays: which keys play the white notes
    /// and the black, and what moves the octave (the two rows have no octave keys: the
    /// toolbar's buttons do).
    pub fn first_run_words(&self) -> String {
        let names = |codes: &[Code]| {
            let names: Vec<String> = codes.iter().map(|&code| self.labels.label(code)).collect();
            names.join(" ")
        };
        match self.layout {
            NoteLayout::Piano => format!(
                "Your computer keyboard plays notes. {}\u{2026} white keys, {} black. {} and {} move the octave.",
                names(&[Code::KeyA, Code::KeyS, Code::KeyD, Code::KeyF]),
                names(&[Code::KeyW, Code::KeyE, Code::KeyT, Code::KeyY, Code::KeyU]),
                self.labels.label(Code::KeyZ),
                self.labels.label(Code::KeyX),
            ),
            NoteLayout::TwoRows => format!(
                "Your computer keyboard plays notes. {}\u{2026} white keys, {} black; {}\u{2026} an octave up. The + and \u{2212} beside the switch move the octave.",
                names(&[Code::KeyZ, Code::KeyX, Code::KeyC, Code::KeyV]),
                names(&[Code::KeyS, Code::KeyD, Code::KeyG, Code::KeyH, Code::KeyJ]),
                names(&[Code::KeyQ, Code::KeyW, Code::KeyE, Code::KeyR]),
            ),
        }
    }

    /// What `code` does now that the keyboard took it from `action`, and how to run that:
    /// "L plays D6 now; Shift+L switches pattern/song".
    fn taken_words(&self, code: Code, action: Action, keys: &Keys) -> String {
        let label = self.labels.label(code);
        let plays = match self.layout.claim(code) {
            Claim::Note(offset) => format!(
                "plays {}",
                crate::ui::key_names::key_name(self.base.saturating_add(offset))
            ),
            Claim::Control(_) => "moves the keyboard".to_owned(),
            Claim::Swallowed | Claim::Free => "plays nothing".to_owned(),
        };
        let shifted =
            Chord::parse(&format!("Shift+{label}")).and_then(|chord| keys.action(&chord, false));
        let instead = match shifted {
            Some(other) if other != action => format!(
                "{} switches the keyboard off, then {label} {}",
                keys.first(Action::Keyboard),
                action.does()
            ),
            _ => format!("Shift+{label} {}", action.does()),
        };
        format!("{label} {plays} now; {instead}")
    }

    /// The first shortcut the keyboard takes from a note key, said as [`Typing::summary`] says
    /// a taken key: what the toolbar's switch and the first-run note warn of.
    pub fn taken_example(&self, keys: &Keys) -> Option<String> {
        Action::ALL.into_iter().find_map(|action| {
            keys.chords(action).iter().find_map(|chord| {
                if chord.is_command() || chord.shift {
                    return None;
                }
                let code = chord.code(&self.labels)?;
                matches!(self.layout.claim(code), Claim::Note(_))
                    .then(|| self.taken_words(code, action, keys))
            })
        })
    }

    /// The toolbar switch's value in the hint bar: "A is C5, velocity 100".
    pub fn value_words(&self) -> String {
        let first = self.labels.label(self.layout.first_key());
        match self.layout {
            NoteLayout::Piano => format!(
                "{first} is {}, velocity {}",
                octave_name(self.base),
                self.velocity
            ),
            NoteLayout::TwoRows => format!("{first} is {}", octave_name(self.base)),
        }
    }
}

impl App {
    /// Ctrl+T or the toolbar's switch: on or off, kept for the next start. Off, every live note
    /// ends.
    pub(super) fn toggle_typing(&mut self) {
        self.typing.on = !self.typing.on;
        if !self.typing.on {
            self.end_typed_notes();
        }
        self.settings.set_keyboard_plays_notes(self.typing.on);
        self.save_settings();
    }

    /// The octave buttons and keys: the layout's first key moves `octaves` octaves, inside its
    /// range.
    pub(super) fn move_octave(&mut self, octaves: i8) {
        let moved = i16::from(self.typing.base) + i16::from(octaves) * i16::from(OCTAVE);
        let clamped = moved.clamp(i16::from(LOWEST_BASE), i16::from(HIGHEST_BASE));
        self.typing.base = u8::try_from(clamped).unwrap_or(self.typing.layout.start());
    }

    /// A key went down (`pressed`) or up while the keyboard plays notes.
    pub(super) fn typed(&mut self, code: Code, pressed: bool, repeat: bool) {
        if !self.typing.on {
            return;
        }
        let claim = self.typing.layout.claim(code);
        if !pressed {
            self.typed_release(code, claim);
            return;
        }
        if repeat {
            return;
        }
        match claim {
            Claim::Note(offset) => self.typed_note(code, self.typing.base + offset),
            Claim::Control(control) => self.typed_control(control),
            Claim::Swallowed | Claim::Free => {}
        }
    }

    /// `code` went down on the note `key`: it plays, unless that key is held down already.
    fn typed_note(&mut self, code: Code, key: u8) {
        if self.typing.held.iter().any(|&(held, _)| held == code) {
            return;
        }
        // A note the pedal holds is ended by the player before it is struck again.
        self.typing.held.push((code, key));
        let velocity = self.typing.note_velocity();
        // The Library's preview, else the instrument window's channel while it is open,
        // whatever the rack selects; the note-off ends the note wherever it started.
        let control = match self.live_target() {
            Some(channel) => Control::ChannelNoteOn {
                channel,
                key,
                velocity,
            },
            None => Control::LiveNoteOn { key, velocity },
        };
        self.send_live(control);
    }

    fn typed_control(&mut self, control: KeyControl) {
        match control {
            KeyControl::OctaveDown => self.move_octave(-1),
            KeyControl::OctaveUp => self.move_octave(1),
            KeyControl::VelocityDown => {
                self.typing.velocity = keys::step_velocity(self.typing.velocity, false);
            }
            KeyControl::VelocityUp => {
                self.typing.velocity = keys::step_velocity(self.typing.velocity, true);
            }
            KeyControl::Sustain => self.press_sustain(true),
            KeyControl::AllNotesOff => self.end_typed_notes(),
        }
    }

    /// The sustain key went down or up: the keyboard's pedal in the engine, on the channel its
    /// notes play. Let go of, the notes it held whose keys are up end there.
    fn press_sustain(&mut self, down: bool) {
        if self.typing.sustain == down {
            return;
        }
        self.typing.sustain = down;
        let control = Control::LiveControl {
            channel: self.live_target(),
            control: LiveControl::Sustain(if down { 127 } else { 0 }),
        };
        self.send_live(control);
    }

    /// `code` went up: its note's key is let go of (the player holds it on while the pedal is
    /// down); the sustain key let go of lifts the pedal.
    fn typed_release(&mut self, code: Code, claim: Claim) {
        if claim == Claim::Control(KeyControl::Sustain) {
            self.press_sustain(false);
            return;
        }
        let Some(index) = self.typing.held.iter().position(|&(held, _)| held == code) else {
            return;
        };
        let (_, key) = self.typing.held.remove(index);
        self.send_live(Control::LiveNoteOff { key });
    }

    fn send_live(&mut self, control: Control) {
        let result = self.session.send(control);
        self.report(result);
    }

    /// Ends every live note: the keyboard went off, Backspace was pressed, or the window lost
    /// the keys and will not see their releases. Every pedal lifts too.
    pub(super) fn end_typed_notes(&mut self) {
        self.forget_live_notes();
        self.send_live(Control::LiveNotesOff);
    }

    /// Stop ends every live note in the player (every pedal lifts), so what the app holds for
    /// them goes too: the keyboard's keys and sustain, the instrument window's pressed key and
    /// the roll's preview. A key still down then plays again when pressed next, and its release
    /// ends nothing; the sustain key still down presses the pedal again when pressed next.
    pub(super) fn forget_live_notes(&mut self) {
        self.typing.held.clear();
        self.typing.sustain = false;
        if let Some(open) = &mut self.instrument {
            open.forget_held();
        }
        self.note_preview = None;
    }

    /// The note layout `layout` chosen: the notes held end, and it plays from its first key.
    pub(super) fn choose_note_layout(&mut self, layout: NoteLayout) {
        if layout == self.typing.layout {
            return;
        }
        self.release_typed_keys();
        // The other layout may have no sustain key to let the pedal go with.
        self.press_sustain(false);
        self.typing.stolen = None;
        self.typing.layout = layout;
        self.typing.base = layout.start();
        self.settings.set_note_layout(layout);
        self.save_settings();
    }

    /// Ends the notes the keys held down and the sustain play, and no other: a device's go on.
    /// The pedal lifts first (ending what it held), then the keys still down let go; with the
    /// sustain key still down, the pedal goes down again, on the channel the notes play now.
    pub(super) fn release_typed_keys(&mut self) {
        let down = self.typing.sustain;
        self.press_sustain(false);
        for (_, key) in std::mem::take(&mut self.typing.held) {
            self.send_live(Control::LiveNoteOff { key });
        }
        if down {
            self.press_sustain(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use dawsome_engine::project::ChannelId;
    use dawsome_engine::realtime::{EngineEvent, Player};

    use super::super::Message;
    use super::*;

    const F1: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/f1_four_on_floor.json"
    );

    fn f1_app() -> (App, Player) {
        super::super::tests::app_on(Path::new(F1))
    }

    /// F1 with the keyboard as the tests written before the piano row knew it: the two rows,
    /// off until switched on.
    fn legacy_app() -> (App, Player) {
        let (mut app, player) = f1_app();
        super::super::tests::legacy_keyboard(&mut app);
        (app, player)
    }

    fn key(app: &mut App, code: Code, pressed: bool, repeat: bool) {
        let _ = app.update(Message::TypingKey {
            code,
            pressed,
            repeat,
        });
    }

    /// What the player reports after one buffer: (started, key, velocity) for each live note.
    fn heard(app: &mut App, player: &mut Player) -> Vec<(bool, u8, f32)> {
        player.process(&mut [[0.0; 2]; 64]);
        app.session
            .take_events()
            .into_iter()
            .filter_map(|event| match event {
                EngineEvent::LiveNoteStarted {
                    channel: ChannelId(1),
                    key,
                    velocity,
                    tick: None,
                    ..
                } => Some((true, key, velocity)),
                EngineEvent::LiveNoteEnded {
                    channel: ChannelId(1),
                    key,
                    tick: None,
                    ..
                } => Some((false, key, 0.0)),
                _ => None,
            })
            .collect()
    }

    // By hand: base 48, Z = 48, , = 48 + 12, 2 = 48 + 13 (Q-row C#), P = 48 + 28; all at the drawn
    // velocity 100/128.
    #[test]
    fn the_rows_play_their_keys_at_the_drawn_velocity() {
        let (mut app, mut player) = legacy_app();
        key(&mut app, Code::KeyZ, true, false);
        assert!(heard(&mut app, &mut player).is_empty(), "off: Z is no note");
        let _ = app.update(Message::ToggleTyping);
        for (code, want) in [
            (Code::KeyZ, 48),
            (Code::Comma, 60),
            (Code::Digit2, 61),
            (Code::KeyP, 76),
        ] {
            key(&mut app, code, true, false);
            assert_eq!(
                heard(&mut app, &mut player),
                [(true, want, 0.78125)],
                "{code:?}"
            );
        }
        key(&mut app, Code::KeyZ, true, true);
        key(&mut app, Code::KeyZ, true, false);
        assert!(heard(&mut app, &mut player).is_empty(), "held: no repeat");
        key(&mut app, Code::KeyZ, false, false);
        assert_eq!(heard(&mut app, &mut player), [(false, 48, 0.0)]);
        key(&mut app, Code::KeyA, true, false);
        assert!(heard(&mut app, &mut player).is_empty(), "A plays nothing");
    }

    #[test]
    fn the_octave_buttons_move_the_rows_inside_their_range() {
        let (mut app, mut player) = legacy_app();
        let _ = app.update(Message::ToggleTyping);
        let _ = app.update(Message::TypingOctave(-1));
        key(&mut app, Code::KeyZ, true, false);
        assert_eq!(heard(&mut app, &mut player), [(true, 36, 0.78125)]);
        // A release after the octave moved ends the note the press started.
        let _ = app.update(Message::TypingOctave(1));
        key(&mut app, Code::KeyZ, false, false);
        assert_eq!(heard(&mut app, &mut player), [(false, 36, 0.0)]);
        for _ in 0..10 {
            let _ = app.update(Message::TypingOctave(-1));
        }
        assert_eq!(app.typing.base, LOWEST_BASE);
        for _ in 0..10 {
            let _ = app.update(Message::TypingOctave(1));
        }
        assert_eq!(app.typing.base, HIGHEST_BASE);
        assert_eq!(octave_name(NoteLayout::TwoRows.start()), "C4");
        assert_eq!(octave_name(HIGHEST_BASE), "C8");
    }

    // Ctrl+T off and the window losing the keys end every live note at once.
    #[test]
    fn switching_off_or_losing_focus_ends_every_live_note() {
        for off in [Message::ToggleTyping, Message::WindowUnfocused] {
            let (mut app, mut player) = legacy_app();
            let _ = app.update(Message::ToggleTyping);
            key(&mut app, Code::KeyZ, true, false);
            key(&mut app, Code::KeyX, true, false);
            let _ = heard(&mut app, &mut player);
            let sent = app.session.controls_sent().len();
            let _ = app.update(off.clone());
            assert_eq!(
                app.session.controls_sent()[sent..],
                [Control::LiveNotesOff],
                "{off:?}"
            );
            assert_eq!(
                heard(&mut app, &mut player),
                [(false, 48, 0.0), (false, 50, 0.0)]
            );
            key(&mut app, Code::KeyZ, false, false);
            assert!(heard(&mut app, &mut player).is_empty(), "nothing held");
        }
    }

    // 6d K1, by hand from the layout research's table: a new window plays the piano row, on,
    // with A on middle C (60) at velocity 100 (100/128 to the player); W and ' are 61 and 77,
    // Q plays nothing; Z and X move the octave, C and V step the velocity (80 is 80/128 =
    // 0.625, 127 is 0.9921875).
    #[test]
    fn a_new_window_plays_the_piano_row_from_a_on_middle_c() {
        let (mut app, mut player) = f1_app();
        assert!(app.typing.on, "on by default");
        assert_eq!(
            (app.typing.layout, app.typing.base),
            (NoteLayout::Piano, 60)
        );
        let mut tap = |app: &mut App, code: Code| {
            key(app, code, true, false);
            key(app, code, false, false);
            heard(app, &mut player)
        };
        assert_eq!(
            tap(&mut app, Code::KeyA),
            [(true, 60, 0.78125), (false, 60, 0.0)]
        );
        assert_eq!(tap(&mut app, Code::KeyW)[0], (true, 61, 0.78125));
        assert_eq!(tap(&mut app, Code::Quote)[0], (true, 77, 0.78125));
        assert!(tap(&mut app, Code::KeyQ).is_empty(), "Q is a gap");
        assert!(tap(&mut app, Code::KeyZ).is_empty(), "Z plays no note");
        assert_eq!(tap(&mut app, Code::KeyA)[0], (true, 48, 0.78125));
        let _ = tap(&mut app, Code::KeyX);
        let _ = tap(&mut app, Code::KeyX);
        assert_eq!(tap(&mut app, Code::KeyA)[0], (true, 72, 0.78125));
        let _ = tap(&mut app, Code::KeyC);
        assert_eq!(tap(&mut app, Code::KeyA)[0], (true, 72, 0.625));
        for _ in 0..3 {
            let _ = tap(&mut app, Code::KeyV);
        }
        assert_eq!(tap(&mut app, Code::KeyA)[0], (true, 72, 0.992_187_5));
        assert_eq!(app.typing.velocity, 127);
    }

    // Stop ends every live note in the player, so the keys the keyboard holds are forgotten: A's
    // release after it sends nothing, A pressed again plays, and Tab held through Stop sends the
    // pedal down again on its next press (2026-10-28: a plugin note whose release never arrived
    // came back on every Play).
    #[test]
    fn stop_forgets_the_keys_held_and_they_play_again_when_pressed() {
        let (mut app, mut player) = f1_app();
        key(&mut app, Code::Tab, true, false);
        key(&mut app, Code::KeyA, true, false);
        assert_eq!(heard(&mut app, &mut player), [(true, 60, 0.78125)]);
        let sent = app.session.controls_sent().len();
        let _ = app.update(Message::Stop);
        assert_eq!(app.session.controls_sent()[sent..], [Control::Stop]);
        assert!(app.typing.held.is_empty() && !app.typing.sustain);
        assert_eq!(
            heard(&mut app, &mut player),
            [(false, 60, 0.0)],
            "Stop ended it"
        );
        let sent = app.session.controls_sent().len();
        key(&mut app, Code::KeyA, false, false);
        key(&mut app, Code::Tab, false, false);
        assert_eq!(app.session.controls_sent()[sent..], [], "ended already");
        key(&mut app, Code::Tab, true, false);
        key(&mut app, Code::KeyA, true, false);
        assert_eq!(
            app.session.controls_sent()[sent..],
            [
                Control::LiveControl {
                    channel: None,
                    control: LiveControl::Sustain(127)
                },
                Control::LiveNoteOn {
                    key: 60,
                    velocity: 0.78125
                }
            ]
        );
        assert_eq!(heard(&mut app, &mut player), [(true, 60, 0.78125)]);
    }

    // Tab held down holds the notes let go of until Tab is let go of; a held note struck again
    // ends and starts anew; Backspace ends every note and lets the sustain go. Tab is the engine's
    // pedal for the keyboard (127 down, 0 up), sent once however long it is held; the keys send
    // their note-offs as they go up and the player holds them.
    #[test]
    fn tab_sustains_while_held_and_backspace_ends_every_note() {
        let (mut app, mut player) = f1_app();
        let sent = app.session.controls_sent().len();
        key(&mut app, Code::Tab, true, false);
        key(&mut app, Code::Tab, true, true);
        assert_eq!(
            app.session.controls_sent()[sent..],
            [Control::LiveControl {
                channel: None,
                control: LiveControl::Sustain(127)
            }]
        );
        key(&mut app, Code::KeyA, true, false);
        key(&mut app, Code::KeyA, false, false);
        key(&mut app, Code::KeyS, true, false);
        key(&mut app, Code::KeyS, false, false);
        assert_eq!(
            heard(&mut app, &mut player),
            [(true, 60, 0.78125), (true, 62, 0.78125)],
            "let go of under the sustain, they sound on"
        );
        key(&mut app, Code::KeyA, true, false);
        assert_eq!(
            heard(&mut app, &mut player),
            [(false, 60, 0.0), (true, 60, 0.78125)],
            "struck again: ended, then anew"
        );
        let sent = app.session.controls_sent().len();
        key(&mut app, Code::Tab, false, false);
        assert_eq!(
            app.session.controls_sent()[sent..],
            [Control::LiveControl {
                channel: None,
                control: LiveControl::Sustain(0)
            }]
        );
        assert_eq!(
            heard(&mut app, &mut player),
            [(false, 62, 0.0)],
            "the sustain let go: S ends, A is still held"
        );
        key(&mut app, Code::Tab, true, false);
        key(&mut app, Code::KeyD, true, false);
        let _ = heard(&mut app, &mut player);
        let sent = app.session.controls_sent().len();
        key(&mut app, Code::Backspace, true, false);
        assert_eq!(app.session.controls_sent()[sent..], [Control::LiveNotesOff]);
        assert_eq!(
            heard(&mut app, &mut player),
            [(false, 60, 0.0), (false, 64, 0.0)]
        );
        for code in [Code::KeyA, Code::KeyD, Code::Tab] {
            key(&mut app, code, false, false);
        }
        assert!(heard(&mut app, &mut player).is_empty(), "nothing hangs on");
    }

    // A held key repeats nothing, and its release ends the note it started after the octave
    // moved under it.
    #[test]
    fn a_held_key_repeats_nothing_and_its_release_ends_its_own_note() {
        let (mut app, mut player) = f1_app();
        key(&mut app, Code::KeyA, true, false);
        key(&mut app, Code::KeyA, true, true);
        key(&mut app, Code::KeyX, true, false);
        key(&mut app, Code::KeyX, false, false);
        key(&mut app, Code::KeyA, false, false);
        assert_eq!(
            heard(&mut app, &mut player),
            [(true, 60, 0.78125), (false, 60, 0.0)]
        );
        assert_eq!(app.typing.base, 72);
    }

    // Switched off, the keyboard stays off at the next start: the settings file says so.
    #[test]
    fn the_keyboard_switched_off_stays_off_at_the_next_start() {
        let dir = std::env::temp_dir().join(format!("dawsome-typing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("dawsome/settings.json");
        let host = || {
            let mut host = crate::document::Host::offline();
            host.settings_file = Some(file.clone());
            host.saves_settings = true;
            host
        };
        let project = || {
            crate::project_file::open(Path::new(F1))
                .expect("opens")
                .project
        };
        let (mut app, _player) = super::super::tests::app_hosted(project(), Path::new(F1), host());
        assert!(app.typing.on);
        let _ = app.update(Message::ToggleTyping);
        let saved = std::fs::read_to_string(&file).expect("saved");
        assert!(
            saved.contains(r#""keyboard_plays_notes": false"#),
            "{saved}"
        );
        let (again, _player) = super::super::tests::app_hosted(project(), Path::new(F1), host());
        assert!(!again.typing.on);
        std::fs::remove_dir_all(&dir).expect("removed");
    }
}
