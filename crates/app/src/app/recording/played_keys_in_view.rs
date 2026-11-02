//! The typing keyboard's L seemed not to record in a take: it sounded and no note showed in the
//! piano roll, with no Library click involved. The autosave held L's notes (key 74) in the takes
//! all the same: they were recorded, but the piano roll showed rows only up to 5 keys above the
//! channel's highest note when the channel was selected (Kick's step hits on C5, 60: rows up to
//! 65), and a take never moved it, so every key the piano row plays above that (G H J K L ..., up
//! to 77) landed out of sight. Selecting another channel and back framed the roll again, which is
//! why that seemed to make it record. A take now keeps the row of each key it hears struck on the
//! channel the roll shows in view. Also pinned: L in a take is a note, never Pattern / song mode
//! (Shift+L is). Expected values are the rule (a key played in a take shows in the piano roll) and
//! the piano row's map (A = 60 .. ' = 77, `keys/notes.rs`' researched table), never the code's
//! output.

use dawsome_engine::project::PatternId;
use dawsome_engine::realtime::Player;
use iced::keyboard::key::Code;
use iced::keyboard::{Key, Modifiers};
use iced::{window, Size};

use super::super::key_input::KeyInput;
use super::super::{App, Message, Panel, PlayMode};
use super::library_preview_take::{armed_in_song_mode, calls, l_key, notes, COUNT_IN_CALLS, L_KEY};
use crate::keys::{Labels, NoteLayout};
use crate::ui::piano_layout::ROW_HEIGHT;
use crate::ui::roll::RollMessage;

/// The roll the fault showed in: a 365 px panel less its strip, scroll bar and velocity lane,
/// about 23 rows.
const SEEN_GRID: Size = Size::new(1000.0, 23.0 * ROW_HEIGHT);

/// Whether the piano roll shows `key`'s row whole.
fn in_view(app: &App, key: u8) -> bool {
    let y = app.roll.view.row_top(key);
    (0.0..=app.roll.grid_size().height - ROW_HEIGHT).contains(&y)
}

/// p17_song armed in song mode, the piano roll open on Kick (its one step hit on C5, 60) at the
/// height the fault showed at ([`SEEN_GRID`]).
fn armed_with_the_roll_open(test: &str) -> (App, Player) {
    let (mut app, player) = armed_in_song_mode(test);
    let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
    assert!(app.panels.piano_roll);
    let _ = app.update(Message::Roll(RollMessage::GridResized(SEEN_GRID)));
    assert_eq!(
        app.selected_channel.map(|id| id.0),
        Some(1),
        "Kick selected"
    );
    assert!(
        !in_view(&app, L_KEY),
        "as found: the roll shows 5 keys above Kick's C5, not L's D6"
    );
    (app, player)
}

/// Play while armed and the count-in: a take of notes runs in bar 1. Returns its pattern.
fn start(app: &mut App, player: &mut Player) -> PatternId {
    let _ = app.update(Message::Play);
    assert!(app.recording.taking_notes(), "a take of notes runs");
    calls(app, player, COUNT_IN_CALLS + 10);
    app.pattern.expect("the pattern box shows the take")
}

fn stop(app: &mut App, player: &mut Player) {
    let _ = app.update(Message::Stop);
    calls(app, player, 2);
}

/// The key on `code` pressed or let go of, typing what a US keyboard prints there, through the
/// window's key path.
fn key(app: &mut App, code: Code, modifiers: Modifiers, pressed: bool) {
    let typed = Labels::default().label(code).to_lowercase();
    let _ = app.update(Message::Key(KeyInput {
        window: window::Id::unique(),
        key: Key::Character(typed.as_str().into()),
        code: Some(code),
        modifiers,
        pressed,
        repeat: false,
    }));
}

// The take the fault showed in: song mode, L struck in bar 1. It sounds, it is a note and not
// Pattern / song mode, its row shows while it is held and once it is written, and the take holds it
// on Kick.
#[test]
fn a_song_take_of_l_shows_in_the_piano_roll() {
    let (mut app, mut player) = armed_with_the_roll_open("song");
    let take = start(&mut app, &mut player);
    l_key(&mut app, true);
    calls(&mut app, &mut player, 10);
    assert_eq!(app.mode, PlayMode::Song, "L is a note, not Pattern / song");
    assert!(app.recording.taking_notes(), "the take runs on");
    assert!(
        !app.roll_live_bars(L_KEY).is_empty(),
        "the roll draws L held"
    );
    assert!(in_view(&app, L_KEY), "L's row shows while it is held");
    l_key(&mut app, false);
    calls(&mut app, &mut player, 10);
    stop(&mut app, &mut player);
    assert_eq!(
        notes(&app, take),
        Some(vec![(1, L_KEY)]),
        "the take holds L"
    );
    assert_eq!(app.pattern, Some(take), "the roll shows the take");
    assert!(in_view(&app, L_KEY), "L's note shows in the piano roll");
}

// Pattern mode the same: L lands in Pattern 1 beside Kick's C5, and the roll shows it.
#[test]
fn a_pattern_take_of_l_shows_in_the_piano_roll() {
    let (mut app, mut player) = armed_with_the_roll_open("pattern");
    let _ = app.update(Message::ModeSelected(PlayMode::Pattern));
    let take = start(&mut app, &mut player);
    assert_eq!(take, PatternId(1));
    l_key(&mut app, true);
    calls(&mut app, &mut player, 10);
    assert_eq!(
        app.mode,
        PlayMode::Pattern,
        "L is a note, not Pattern / song"
    );
    assert!(in_view(&app, L_KEY), "L's row shows while it is held");
    l_key(&mut app, false);
    calls(&mut app, &mut player, 10);
    stop(&mut app, &mut player);
    let landed: Vec<_> = notes(&app, take)
        .expect("Pattern 1 stays")
        .into_iter()
        .filter(|&(_, key)| key == L_KEY)
        .collect();
    assert_eq!(landed, [(1, L_KEY)], "L lands on Kick in Pattern 1");
    assert!(in_view(&app, L_KEY), "L's note shows in the piano roll");
}

// The class, not just L: every note key of the piano row, A to ' (60 to 77), struck in one take
// shows its row as it is struck and is recorded; then two octaves down (Z twice) A plays 36,
// below the rows shown, and shows too.
#[test]
fn every_key_a_take_hears_shows_in_the_piano_roll() {
    let (mut app, mut player) = armed_with_the_roll_open("every");
    let take = start(&mut app, &mut player);
    let none = Modifiers::empty();
    let mut played = Vec::new();
    for &(code, offset) in NoteLayout::Piano.notes() {
        let key_played = 60 + offset;
        key(&mut app, code, none, true);
        calls(&mut app, &mut player, 3);
        assert_eq!(app.mode, PlayMode::Song, "{code:?} is a note");
        assert!(
            in_view(&app, key_played),
            "{code:?} ({key_played}) shows held"
        );
        key(&mut app, code, none, false);
        calls(&mut app, &mut player, 3);
        assert!(
            in_view(&app, key_played),
            "{code:?} ({key_played}) shows written"
        );
        played.push((1, key_played));
    }
    for _ in 0..2 {
        key(&mut app, Code::KeyZ, none, true);
        key(&mut app, Code::KeyZ, none, false);
    }
    key(&mut app, Code::KeyA, none, true);
    calls(&mut app, &mut player, 3);
    assert!(in_view(&app, 36), "A two octaves down (36) shows held");
    key(&mut app, Code::KeyA, none, false);
    calls(&mut app, &mut player, 3);
    played.push((1, 36));
    stop(&mut app, &mut player);
    let mut recorded = notes(&app, take).expect("the take stays");
    recorded.sort_unstable();
    played.sort_unstable();
    assert_eq!(recorded, played, "every key struck is recorded");
    assert!(in_view(&app, 36), "the last one struck shows after Stop");
}
