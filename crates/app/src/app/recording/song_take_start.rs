//! Record in song mode, pressed while the song was already playing, made "Hat closed take N"
//! 17 bars long at once and put the notes after bar 17.
//! The take counted from the song marker, but the player, already playing, refused the record
//! and played on from where it was. Record in song mode now plays from the song marker, where the
//! user sees it, and the take's bar 1 is there: it starts one bar long, grows a bar at a time as
//! the playhead goes, and places no clip (the user places the take's pattern). By hand: 44.1 kHz
//! at 120 BPM and 96 ticks a beat is 229.6875 samples a tick, 88 200 a bar (384 ticks), so 2 760
//! calls of 512 frames (1 413 120 samples, tick 6152) play past the start of bar 17 (tick 6144).

use std::path::Path;

use dawsome_engine::edit::Edit;
use dawsome_engine::project::{Clip, PatternId, Project, TrackId};
use dawsome_engine::realtime::Player;
use dawsome_engine::time::Ticks;

use super::super::{App, Message, PlayMode};
use super::RecordMessage;
use crate::midi::{ControllerQueue, Router};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");
const CALL: usize = 512;
/// Calls of 512 frames that play past tick 6144, the start of bar 17.
const PAST_BAR_17: u64 = 2_760;

fn open(test: &str) -> (App, Player) {
    let dir = std::env::temp_dir()
        .join(format!("dawsome-song-take-start-{}", std::process::id()))
        .join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    for file in ["p17_song.json", "impulse.wav"] {
        std::fs::copy(Path::new(FIXTURES).join(file), dir.join(file)).expect("copies");
    }
    super::super::tests::app_on(&dir.join("p17_song.json"))
}

fn call(app: &mut App, player: &mut Player) {
    player.process(&mut vec![[0.0; 2]; CALL]);
    let _ = app.update(Message::Tick);
}

fn project(app: &App) -> &Project {
    app.session.project()
}

/// One take of C5 from the song marker `marker`, Record pressed while the song plays at tick
/// 6144 or later; returns the take's pattern.
fn take_while_playing(
    app: &mut App,
    player: &mut Player,
    device: &Router,
    marker: i64,
) -> PatternId {
    app.move_song_marker(Ticks(marker));
    let _ = app.update(Message::Play);
    for _ in 0..PAST_BAR_17 {
        call(app, player);
    }
    let at = app.session.position().ticks;
    assert!(
        at >= Ticks(6144),
        "playing past bar 17: at {at:?}, playing {}",
        app.session.playing()
    );
    let clips = project(app).playlist.clips.len();
    let _ = app.update(Message::Record(RecordMessage::ToggleArm));
    let _ = app.update(Message::Play);
    assert!(app.recording.taking_notes(), "a take of notes in song mode");
    let pattern = app.pattern.expect("the pattern box shows the take");
    let bars = |app: &App| project(app).patterns[&pattern].length_bars;
    // The window's timer before the player has heard Record: what it last posted is the old
    // playhead, past bar 17.
    let _ = app.update(Message::Tick);
    assert_eq!(bars(app), 1, "the take starts one bar long");
    for index in 0..20u64 {
        if index == 10 {
            device.deliver(&[0x90, 60, 100]);
        }
        if index == 15 {
            device.deliver(&[0x80, 60, 0]);
        }
        call(app, player);
        assert_eq!(bars(app), 1, "still bar 1 of the take after call {index}");
    }
    let position = app.session.position().ticks;
    assert!(
        position >= Ticks(marker) && position < Ticks(marker + 384),
        "Record played from the song marker {marker}, not on from bar 17: at {position:?}"
    );
    // Through the end of the take's bar 1: the take grows to 2 bars there, not before.
    while app.session.position().ticks < Ticks(marker + 384) {
        assert_eq!(bars(app), 1, "one bar until the playhead leaves it");
        call(app, player);
    }
    assert_eq!(bars(app), 2, "a bar more once the playhead is in bar 2");
    assert_eq!(
        project(app).playlist.clips.len(),
        clips,
        "no clip placed while the take runs"
    );
    let _ = app.update(Message::Stop);
    assert!(!app.recording.taking_notes());
    let _ = app.update(Message::Record(RecordMessage::ToggleArm));
    let notes = &project(app).patterns[&pattern].notes;
    assert_eq!(notes.len(), 1, "the one note played");
    // Heard in call 10 after the take's start (5 120 samples, tick 22 from the marker).
    assert!(
        (Ticks(20)..Ticks(30)).contains(&notes[0].start),
        "the note lands in bar 1 from the take's own start: {:?}",
        notes[0].start
    );
    assert_eq!(
        project(app).playlist.clips.len(),
        clips,
        "no clip placed at Stop"
    );
    pattern
}

#[test]
fn record_in_song_mode_while_playing_starts_one_bar_at_the_marker() {
    let (mut app, mut player) = open("while_playing");
    let _ = app.update(Message::Record(RecordMessage::ToggleCountIn));
    let _ = app.update(Message::ModeSelected(PlayMode::Song));
    // A song 21 bars long (a clip in bar 21), so it plays past bar 17 before it loops.
    let first_clip = project(&app).playlist.next_clip_id;
    app.commit(Edit::PlaceClips {
        first: first_clip,
        clips: vec![Clip::of_pattern(
            PatternId(1),
            TrackId(1),
            Ticks(384 * 20),
            Ticks(384),
        )],
    });
    let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
    let first = take_while_playing(&mut app, &mut player, &device, 0);
    assert_eq!(project(&app).patterns[&first].length_bars, 2);
    // A second take, from bar 3: one bar from its own start again, its note from there.
    let second = take_while_playing(&mut app, &mut player, &device, 768);
    assert_ne!(first, second, "a pattern of its own");
    assert_eq!(
        project(&app).patterns[&first].length_bars,
        2,
        "the first stays"
    );
    assert_eq!(
        app.session.history().undo_len(),
        3,
        "the clip, then one undo step a take"
    );
}
