//! A second song take of notes would not record: the keys sounded but nothing landed, and selecting
//! another channel and back made it record again. The keys played the Library's preview (a single
//! click on an instrument row), a channel the plans alone have; a take of notes keeps only notes of
//! the document's channels, so it dropped every one and took its pattern out at Stop. Selecting a
//! rack channel ends the preview, which is why switching instruments seemed to fix it. A take of
//! notes now ends the preview as it starts: the keys play the selected channel, which the take is
//! named after and records. Also pinned: the sequence the fault showed in (take, Stop, place it on
//! a track, take again, in song mode, struck with the typing keyboard's L) records every take.

use std::path::Path;

use dawsome_engine::edit::Edit;
use dawsome_engine::project::{Clip, PatternId, TrackId};
use dawsome_engine::realtime::Player;
use dawsome_engine::time::Ticks;
use iced::keyboard::key::Code;
use iced::keyboard::{Key, Modifiers};
use iced::window;

use super::super::key_input::KeyInput;
use super::super::{App, Message, PlayMode};
use super::RecordMessage;
use crate::session::PREVIEW_CHANNEL;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");
/// The typing keyboard's L in the piano layout at its first octave: 60 + 14.
pub(super) const L_KEY: u8 = 74;
/// 512-frame calls in the one-bar count-in at 120 BPM, 44.1 kHz: 88 200 / 512 = 172.3.
pub(super) const COUNT_IN_CALLS: usize = 173;

/// p17_song (channel 1 "Kick", a sampler; Pattern 1 on Track 1 over bars 1 to 4) in song
/// mode, Record armed for notes with its count-in (as a new window has it).
pub(super) fn armed_in_song_mode(test: &str) -> (App, Player) {
    let dir = std::env::temp_dir()
        .join(format!(
            "dawsome-library-preview-take-{}",
            std::process::id()
        ))
        .join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    for file in ["p17_song.json", "impulse.wav"] {
        std::fs::copy(Path::new(FIXTURES).join(file), dir.join(file)).expect("copies");
    }
    let (mut app, player) = super::super::tests::app_on(&dir.join("p17_song.json"));
    let _ = app.update(Message::ModeSelected(PlayMode::Song));
    let _ = app.update(Message::Record(RecordMessage::ToggleArm));
    assert!(app.recording.count_in, "a new window counts in");
    (app, player)
}

/// The typing keyboard's L pressed or let go of, through the window's key path.
pub(super) fn l_key(app: &mut App, pressed: bool) {
    let _ = app.update(Message::Key(KeyInput {
        window: window::Id::unique(),
        key: Key::Character("l".into()),
        code: Some(Code::KeyL),
        modifiers: Modifiers::empty(),
        pressed,
        repeat: false,
    }));
}

pub(super) fn calls(app: &mut App, player: &mut Player, count: usize) {
    for _ in 0..count {
        player.process(&mut vec![[0.0; 2]; 512]);
        let _ = app.update(Message::Tick);
    }
}

/// Play while armed, the count-in and 10 calls of bar 1, L held for 10 calls, Stop: returns
/// the take's pattern.
pub(super) fn take_of_l(app: &mut App, player: &mut Player) -> PatternId {
    let _ = app.update(Message::Play);
    assert!(app.recording.taking_notes(), "a take of notes runs");
    let pattern = app.pattern.expect("the pattern box shows the take");
    calls(app, player, COUNT_IN_CALLS + 10);
    l_key(app, true);
    calls(app, player, 10);
    l_key(app, false);
    calls(app, player, 10);
    let _ = app.update(Message::Stop);
    calls(app, player, 2);
    pattern
}

/// The take's notes as (channel, key), or none when the take left no pattern.
pub(super) fn notes(app: &App, pattern: PatternId) -> Option<Vec<(u32, u8)>> {
    let pattern = app.session.project().patterns.get(&pattern)?;
    Some(
        pattern
            .notes
            .iter()
            .map(|note| (note.channel.0, note.key))
            .collect(),
    )
}

/// `pattern` placed from bar 1 on `track`, as a press on its lane does.
fn place(app: &mut App, pattern: PatternId, track: u32) {
    let bars = app.session.project().patterns[&pattern].length_bars;
    let first = app.session.project().playlist.next_clip_id;
    app.commit(Edit::PlaceClips {
        first,
        clips: vec![Clip::of_pattern(
            pattern,
            TrackId(track),
            Ticks(0),
            Ticks(384 * i64::from(bars)),
        )],
    });
}

/// A set clicked in the Library, as a single press on its row previews it.
fn preview_a_set(app: &mut App) {
    app.preview_set_at_start(Path::new("sets/pad/pad.sfz"));
    assert_eq!(app.live_target(), Some(PREVIEW_CHANNEL), "the keys play it");
}

// The order the fault showed in: take 1, Stop, placed on a track, the Library previewing, take 2
// struck with L. It sounded and recorded nothing (the take's pattern was taken out at Stop).
#[test]
fn a_song_take_records_the_keys_the_library_was_previewing() {
    let (mut app, mut player) = armed_in_song_mode("song");
    let one = take_of_l(&mut app, &mut player);
    assert_eq!(notes(&app, one), Some(vec![(1, L_KEY)]), "take 1 records L");
    place(&mut app, one, 2);
    preview_a_set(&mut app);
    let two = take_of_l(&mut app, &mut player);
    assert_eq!(
        notes(&app, two),
        Some(vec![(1, L_KEY)]),
        "take 2 records L on the selected channel, Kick"
    );
    assert_eq!(app.live_target(), None, "the preview ended with the take");
    assert!(
        app.browse.model.playing.is_none(),
        "its row is marked no more"
    );
}

// Pattern mode the same: the selected pattern's take kept nothing the preview played.
#[test]
fn a_pattern_take_records_the_keys_the_library_was_previewing() {
    let (mut app, mut player) = armed_in_song_mode("pattern");
    let _ = app.update(Message::ModeSelected(PlayMode::Pattern));
    preview_a_set(&mut app);
    let pattern = take_of_l(&mut app, &mut player);
    assert_eq!(pattern, PatternId(1));
    let landed: Vec<_> = notes(&app, pattern)
        .expect("the pattern stays")
        .into_iter()
        .filter(|&(_, key)| key == L_KEY)
        .collect();
    assert_eq!(landed, [(1, L_KEY)], "L lands on Kick in Pattern 1");
}

// The same steps with no preview: three takes, each placed on a track before the
// next, every one records its L (cleared suspects: placing a take, the pattern box on a placed
// take, the take's guard after Stop).
#[test]
fn takes_placed_on_tracks_between_takes_all_record() {
    let (mut app, mut player) = armed_in_song_mode("placed");
    for (index, track) in [2, 3, 4].into_iter().enumerate() {
        let take = take_of_l(&mut app, &mut player);
        assert_eq!(
            notes(&app, take),
            Some(vec![(1, L_KEY)]),
            "take {} records L",
            index + 1
        );
        place(&mut app, take, track);
    }
}
