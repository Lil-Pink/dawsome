//! Recording v2's known answers through the window: tests/fixtures/gen_p17_expected.py wrote the
//! projects, the MIDI messages (timed by 512-frame call, as `--midi-events` has them) and the
//! documents each case leaves; these tests play them through the app's own path (the Record
//! button's messages, a device's router, the player called where the audio device would) and
//! compare the document byte for byte.

use std::path::{Path, PathBuf};

use dawsome_engine::project::{ClipId, PatternId};
use dawsome_engine::realtime::Player;
use dawsome_engine::time::Ticks;
use iced::keyboard::key::Code;
use iced::keyboard::{Key, Modifiers};
use iced::window;

use super::super::{App, Message, PlayMode};
use super::RecordMessage;
use crate::app::key_input::KeyInput;
use crate::midi::{ControllerQueue, Router};
use crate::play_to::RecordSource;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

/// The frames each call of the player takes, as the events are timed.
const CALL: usize = 512;

fn fixture(name: &str) -> String {
    let text = std::fs::read_to_string(Path::new(FIXTURES).join(name)).expect("the fixture");
    text.trim_end().to_owned()
}

fn expected() -> serde_json::Value {
    serde_json::from_str(&fixture("p17_expected.json")).expect("parses")
}

fn number(value: &serde_json::Value) -> u64 {
    value.as_u64().expect("a number")
}

fn name(value: &serde_json::Value) -> &str {
    value.as_str().expect("a name")
}

/// `project` (and the sound it plays) copied into a folder of the test's own, opened in the
/// window with no audio device; the player stands in for the audio callback.
fn open(test: &str, project: &str) -> (App, Player, PathBuf) {
    let dir = std::env::temp_dir()
        .join(format!("dawsome-p17-{}", std::process::id()))
        .join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    for file in [project, "impulse.wav"] {
        std::fs::copy(Path::new(FIXTURES).join(file), dir.join(file)).expect("copies");
    }
    let (app, player) = super::super::tests::app_on(&dir.join(project));
    (app, player, dir)
}

/// The messages of an events file, by the call they come before.
fn events(file: &str) -> Vec<(u64, Vec<u8>)> {
    let doc: serde_json::Value = serde_json::from_str(&fixture(file)).expect("parses");
    assert_eq!(number(&doc["buffer"]), CALL as u64);
    doc["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|event| {
            let bytes = event["bytes"]
                .as_array()
                .expect("bytes")
                .iter()
                .map(|byte| u8::try_from(number(byte)).expect("a byte"))
                .collect();
            (number(&event["call"]), bytes)
        })
        .collect()
}

fn document(app: &App) -> String {
    app.session.project().to_json().expect("serializes")
}

fn record(app: &mut App, message: RecordMessage) {
    let _ = app.update(Message::Record(message));
}

/// Plays call `call`: its messages through the device first, then the player's 512 frames
/// (`frames` for a shorter last call), then the window's playhead timer.
fn play_call(
    app: &mut App,
    player: &mut Player,
    device: &Router,
    script: &[(u64, Vec<u8>)],
    call: u64,
    frames: usize,
) {
    for (_, bytes) in script.iter().filter(|(at, _)| *at == call) {
        device.deliver(bytes);
    }
    player.process(&mut vec![[0.0; 2]; frames]);
    let _ = app.update(Message::Tick);
}

/// The notes of `pattern` as the project file writes them.
fn notes_json(app: &App, pattern: PatternId) -> serde_json::Value {
    let notes = &app.session.project().patterns[&pattern].notes;
    serde_json::to_value(notes).expect("serializes")
}

/// One undo gives `before` back byte for byte, and its redo `after`.
fn one_undo_step(app: &mut App, before: &str, after: &str) {
    assert_eq!(app.session.history().undo_len(), 1, "one undo step");
    let _ = app.update(Message::Undo);
    assert_eq!(document(app), before, "undone");
    let _ = app.update(Message::Redo);
    assert_eq!(document(app), after, "redone");
}

/// Ctrl+Shift+R pressed and let go of, through the window's keys.
fn capture_keys(app: &mut App) {
    for pressed in [true, false] {
        let _ = app.update(Message::Key(KeyInput {
            window: window::Id::unique(),
            key: Key::Character("r".into()),
            code: Some(Code::KeyR),
            modifiers: Modifiers::CTRL | Modifiers::SHIFT,
            pressed,
            repeat: false,
        }));
    }
}

fn frames_of(path: &Path) -> u32 {
    hound::WavReader::open(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .duration()
}

// Known answer (1) through the window: a take of notes in pattern mode, two passes over the
// one-bar pattern. Each note is in the pattern the moment its end is heard: after 210 calls the
// three notes that ended so far, quantized to the step; at the stop (176400 frames, tick 768)
// the same three unchanged, the second pass's note added over the first (overdub) and the note
// still held ending at the stop's tick. One undo step.
#[test]
fn p17_a_pattern_take_writes_each_note_the_moment_it_ends() {
    let want = &expected()["pattern"];
    let (mut app, mut player, _dir) = open("pattern", name(&want["project"]));
    let before = document(&app);
    record(&mut app, RecordMessage::ToggleCountIn);
    record(&mut app, RecordMessage::ToggleArm);
    let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
    let script = events(name(&want["events"]));
    let _ = app.update(Message::Play);
    assert!(app.recording.taking_notes());
    let frames = usize::try_from(number(&want["frames"])).expect("fits");
    let calls = frames.div_ceil(CALL) as u64;
    for call in 0..calls {
        let left = frames - call as usize * CALL;
        play_call(
            &mut app,
            &mut player,
            &device,
            &script,
            call,
            left.min(CALL),
        );
        if call + 1 == number(&want["dump_call"]) {
            assert_eq!(
                notes_json(&app, PatternId(1)),
                want["dump_notes"],
                "drawn as they end"
            );
        }
    }
    let _ = app.update(Message::Stop);
    assert!(!app.recording.taking_notes());
    let after = fixture(name(&want["document"]));
    assert_eq!(document(&app), after);
    one_undo_step(&mut app, &before, &after);
}

/// Plays a take of notes in song mode from `marker`, `calls` calls long, with the messages of
/// `script`; returns the app, the document before and the bars the take held after each call.
fn song_take(
    test: &str,
    marker: i64,
    calls: u64,
    script: &[(u64, Vec<u8>)],
) -> (App, String, Vec<u32>) {
    let (mut app, mut player, _dir) = open(test, "p17_song.json");
    let before = document(&app);
    record(&mut app, RecordMessage::ToggleCountIn);
    let _ = app.update(Message::ModeSelected(PlayMode::Song));
    app.move_song_marker(Ticks(marker));
    record(&mut app, RecordMessage::ToggleArm);
    let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
    let _ = app.update(Message::Play);
    assert!(app.recording.taking_notes(), "a take of notes in song mode");
    assert_eq!(
        app.pattern,
        Some(PatternId(2)),
        "the pattern box shows the take"
    );
    let mut bars = Vec::new();
    for call in 0..calls {
        play_call(&mut app, &mut player, &device, script, call, CALL);
        let project = app.session.project();
        let held = project.patterns.get(&PatternId(2)).map(|p| p.length_bars);
        if let (Some(held), Some(clip)) = (held, project.playlist.clips.get(&ClipId(2))) {
            assert_eq!(
                clip.length,
                Ticks(i64::from(held) * 384),
                "the clip as the pattern"
            );
        }
        bars.push(held.unwrap_or(0));
    }
    let _ = app.update(Message::Stop);
    assert!(!app.recording.taking_notes());
    (app, before, bars)
}

// Known answers (2) and (5): a take of notes in song mode from tick 384, three notes over 5.5
// bars and one held over the stop (it ends at the stop's tick, 2497). Pattern "Kick take 1" and
// its clip on track 2 (track 1 is taken over the take's first bar) grow by whole bars as the
// playhead goes: after the checkpoint calls 1, 1, 2, 3, 4, 6 bars; at the stop 6 bars, the clip
// 2304 ticks from 384, the song ending at 2688. One undo step gives the document back, its redo
// the take again.
#[test]
fn p17_a_song_take_grows_by_bars_and_is_one_undo_step() {
    let want = &expected()["song"];
    let script = events(name(&want["events"]));
    let (mut app, before, bars) = song_take(
        "song",
        number(&want["marker"]) as i64,
        number(&want["calls"]),
        &script,
    );
    for checkpoint in want["checkpoints"].as_array().expect("checkpoints") {
        let call = number(&checkpoint[0]) as usize;
        assert_eq!(
            u64::from(bars[call]),
            number(&checkpoint[1]),
            "bars after call {call}"
        );
    }
    assert_eq!(before, fixture("p17_song.json"));
    let after = fixture(name(&want["document"]));
    assert_eq!(document(&app), after);
    assert_eq!(
        app.session.project().playlist.end(),
        Ticks(number(&want["song_end"]) as i64)
    );
    assert_eq!(
        app.pattern,
        Some(PatternId(2)),
        "the take's pattern stays shown"
    );
    one_undo_step(&mut app, &before, &after);
}

// Known answer (5): a take in song mode from tick 3072, past the song's end (1536). The player
// plays straight on while it records, so the note lands at its tick; the clip on track 1 (free
// there), 2 bars, the song ending at 3840.
#[test]
fn p17_a_song_take_past_the_songs_end() {
    let want = &expected()["song_past_end"];
    let script = events(name(&want["events"]));
    let (mut app, before, _) = song_take(
        "song_past_end",
        number(&want["marker"]) as i64,
        number(&want["calls"]),
        &script,
    );
    let after = fixture(name(&want["document"]));
    assert_eq!(document(&app), after);
    assert_eq!(
        app.session.project().playlist.end(),
        Ticks(number(&want["song_end"]) as i64)
    );
    one_undo_step(&mut app, &before, &after);
}

// Known answer (5): a take in song mode in which no note is played makes nothing: no empty
// pattern, no clip, no undo step, and the pattern box back on the pattern it showed.
#[test]
fn p17_a_song_take_with_no_note_leaves_nothing() {
    let want = &expected()["song_empty"];
    let (app, before, bars) = song_take(
        "song_empty",
        number(&want["marker"]) as i64,
        number(&want["calls"]),
        &[],
    );
    assert!(bars.iter().any(|&held| held > 1), "it grew while it ran");
    assert_eq!(document(&app), before);
    assert_eq!(app.session.history().undo_len(), 0);
    assert_eq!(app.pattern, Some(PatternId(1)));
}

// Known answer (3), song mode: two takes of the master from tick 192, 1000 frames each at
// 48 kHz, become "p17_audio48 take 1.wav" and "take 2.wav" in samples/ (1000 frames each),
// audio channels "Take 1" and "Take 2" and their clips at 192, 4 ticks long, on tracks 2 and 3.
// Each take is one undo step; undoing both gives the document back and leaves the files.
#[test]
fn p17_audio_takes_in_song_mode_are_named_after_the_project() {
    let want = &expected()["audio_song"];
    let (mut app, mut player, dir) = open("audio_song", name(&want["project"]));
    let before = document(&app);
    record(&mut app, RecordMessage::Source(RecordSource::Master));
    record(&mut app, RecordMessage::ToggleCountIn);
    let _ = app.update(Message::ModeSelected(PlayMode::Song));
    app.move_song_marker(Ticks(number(&want["marker"]) as i64));
    record(&mut app, RecordMessage::ToggleArm);
    let frames = usize::try_from(number(&want["frames"])).expect("fits");
    let takes = want["takes"].as_array().expect("takes");
    for _ in takes {
        let _ = app.update(Message::Play);
        assert!(app.recording.recording(), "a take of the master");
        assert_eq!(app.mode, PlayMode::Song);
        player.process(&mut vec![[0.0; 2]; frames / 2]);
        player.process(&mut vec![[0.0; 2]; frames - frames / 2]);
        let _ = app.update(Message::Stop);
        assert!(!app.recording.recording());
    }
    let after = fixture(name(&want["document"]));
    assert_eq!(document(&app), after);
    for take in takes {
        let file = dir.join(name(&take["file"]));
        assert_eq!(u64::from(frames_of(&file)), number(&take["frames"]));
    }
    // The rack folds the takes under one header; the Library lists their folder as Recordings.
    let rows = app.rack_rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows.takes_header(), Some((1, 2, false)));
    let _ = app.update(Message::ToggleTakes);
    assert_eq!(app.rack_rows().len(), 4, "opened");
    let recordings = app
        .browse
        .model
        .roots
        .iter()
        .find(|root| root.name == crate::ui::browser_model::RECORDINGS)
        .expect("a Recordings root");
    assert_eq!(recordings.path, dir.join("samples"));
    assert_eq!(app.session.history().undo_len(), takes.len());
    for _ in takes {
        let _ = app.update(Message::Undo);
    }
    assert_eq!(document(&app), before);
    for take in takes {
        assert!(dir.join(name(&take["file"])).is_file(), "the file stays");
    }
}

// Known answer (3), pattern mode: a take of the master records one pass of the one-bar pattern,
// 96000 frames at 48 kHz, and stops by itself; its clip lands at the song marker (tick 0) on
// track 2, 384 ticks long.
#[test]
fn p17_an_audio_take_in_pattern_mode_records_one_pass() {
    let want = &expected()["audio_pattern"];
    let (mut app, mut player, dir) = open("audio_pattern", name(&want["project"]));
    let before = document(&app);
    record(&mut app, RecordMessage::Source(RecordSource::Master));
    record(&mut app, RecordMessage::ToggleCountIn);
    record(&mut app, RecordMessage::ToggleArm);
    let _ = app.update(Message::Play);
    assert!(app.recording.recording());
    assert_eq!(app.mode, PlayMode::Pattern, "the pattern plays");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut calls = 0;
    while app.recording.recording() && std::time::Instant::now() < deadline {
        if calls < 400 {
            player.process(&mut [[0.0; 2]; CALL]);
            calls += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
        let _ = app.update(Message::Tick);
    }
    assert!(!app.recording.recording(), "the take stopped by itself");
    let take = &want["take"];
    let after = fixture(name(&want["document"]));
    assert_eq!(document(&app), after);
    assert_eq!(
        u64::from(frames_of(&dir.join(name(&take["file"])))),
        number(&take["frames"])
    );
    one_undo_step(&mut app, &before, &after);
}

/// Plays the capture's events with the transport stopped, then Capture in `mode` (with the song
/// marker at `marker` in song mode).
fn captured(test: &str, mode: PlayMode, marker: i64) -> (App, String) {
    let want = &expected()["capture"];
    let (mut app, mut player, _dir) = open(test, name(&want["project"]));
    let before = document(&app);
    if mode == PlayMode::Song {
        let _ = app.update(Message::ModeSelected(PlayMode::Song));
        app.move_song_marker(Ticks(marker));
    }
    let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
    let script = events(name(&want["events"]));
    for call in 0..number(&want["calls"]) {
        play_call(&mut app, &mut player, &device, &script, call, CALL);
    }
    capture_keys(&mut app);
    (app, before)
}

// Known answer (4), pattern mode: notes played while stopped (one of them more than 30 s before
// the phrase, left out), then Ctrl+Shift+R: the phrase at 120 BPM from the pattern's first bar,
// quantized to the step, the pattern grown to two bars to hold it. One undo step.
#[test]
fn p17_capture_lays_the_last_phrase_into_the_pattern() {
    let want = &expected()["capture"];
    let (mut app, before) = captured("capture", PlayMode::Pattern, 0);
    let after = fixture(name(&want["document"]));
    assert_eq!(document(&app), after);
    one_undo_step(&mut app, &before, &after);
}

// Known answer (4), song mode: the same phrase becomes pattern "Keys take 1" and its clip from
// the bar start nearest the song marker (400 → 384), on track 1. One undo step.
#[test]
fn p17_capture_in_song_mode_makes_a_pattern_and_its_clip() {
    let want = &expected()["capture"];
    let (mut app, before) = captured(
        "capture_song",
        PlayMode::Song,
        number(&want["song_marker"]) as i64,
    );
    let after = fixture(name(&want["song_document"]));
    assert_eq!(document(&app), after);
    one_undo_step(&mut app, &before, &after);
}

/// An edit during a song take: p17_song with an earlier step (channel 1
/// renamed), a song take from tick 384 with a note, and `edit` committed after call 40 while
/// the take runs; then Stop and Ctrl+Z down to nothing. Returns whether `edit` went through,
/// each undo's refusal (none expected) and whether the document came back to the fixture.
fn edit_during_a_song_take(
    test: &str,
    edit: dawsome_engine::edit::Edit,
) -> (bool, Vec<String>, bool) {
    let (mut app, mut player, _dir) = open(test, "p17_song.json");
    let fixture_text = document(&app);
    app.commit(dawsome_engine::edit::Edit::RenameChannel {
        id: dawsome_engine::project::ChannelId(1),
        name: "Kick drum".to_owned(),
    });
    record(&mut app, RecordMessage::ToggleCountIn);
    let _ = app.update(Message::ModeSelected(PlayMode::Song));
    app.move_song_marker(Ticks(384));
    record(&mut app, RecordMessage::ToggleArm);
    let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
    let script = [(10, vec![0x90, 60, 100]), (20, vec![0x80, 60, 0])];
    let _ = app.update(Message::Play);
    assert!(app.recording.taking_notes());
    let mut went_through = false;
    for call in 0..60 {
        play_call(&mut app, &mut player, &device, &script, call, CALL);
        if call == 40 {
            let before = document(&app);
            app.commit(edit.clone());
            went_through = document(&app) != before;
            if !went_through {
                assert_eq!(app.error.as_deref(), Some("Stop the recording first"));
            }
        }
    }
    let _ = app.update(Message::Stop);
    let mut refusals = Vec::new();
    while app.session.history().undo_len() > 0 {
        let left = app.session.history().undo_len();
        let _ = app.update(Message::Undo);
        if app.session.history().undo_len() == left {
            refusals.push(app.error.clone().unwrap_or_default());
            break;
        }
    }
    (went_through, refusals, document(&app) == fixture_text)
}

// While a song take runs, an edit naming its own pattern or clip
// (renaming the pattern, moving or muting the clip) is refused, so every undo after Stop goes
// through and the document comes back to the fixture. Renaming another channel still goes
// through and undoes after the take.
#[test]
fn edits_naming_the_song_takes_pattern_or_clip_are_refused() {
    use dawsome_engine::edit::Edit;
    use dawsome_engine::project::TrackId;
    for (test, edit) in [
        (
            "take_rename",
            Edit::RenamePattern {
                id: PatternId(2),
                name: "Renamed".to_owned(),
            },
        ),
        (
            "take_move",
            Edit::MoveClips {
                moves: vec![(ClipId(2), Ticks(768), TrackId(3))],
            },
        ),
        (
            "take_mute",
            Edit::SetClipMute {
                ids: vec![ClipId(2)],
                muted: true,
            },
        ),
    ] {
        let (went_through, refusals, back) = edit_during_a_song_take(test, edit);
        assert!(!went_through, "{test}: refused");
        assert_eq!(refusals, Vec::<String>::new(), "{test}");
        assert!(back, "{test}: the document back to the fixture");
    }
    let (went_through, refusals, back) = edit_during_a_song_take(
        "take_other_channel",
        Edit::RenameChannel {
            id: dawsome_engine::project::ChannelId(1),
            name: "Kick again".to_owned(),
        },
    );
    assert!(went_through, "another channel's name goes through");
    assert_eq!(refusals, Vec::<String>::new());
    assert!(back);
}

// `--channel-menu` opens a channel's menu on its own row: with the
// takes folded under Recordings and a channel added after them, channel 4 is row 1.
#[test]
fn the_channel_menu_opens_on_the_channels_row() {
    let (mut app, _player, _dir) = open("channel_menu", "p17_audio_song_after.json");
    app.commit(dawsome_engine::edit::Edit::AddSamplerChannel {
        id: dawsome_engine::project::ChannelId(4),
        name: "Snare".to_owned(),
        sample: PathBuf::new(),
    });
    assert_eq!(
        app.rack_rows()
            .row_of(dawsome_engine::project::ChannelId(4)),
        Some(1)
    );
    app.open_channel_menu(dawsome_engine::project::ChannelId(4));
    assert_eq!(
        app.sampler_menu,
        Some((
            dawsome_engine::project::ChannelId(4),
            crate::ui::layout::name_button_rect(1).center()
        ))
    );
}

/// An edit during a song take that adds a track: p17_song with tracks 2..=8 holding a clip over
/// 384..768 (one step), so a song take from 384 adds track 9; `edit` committed after call 40,
/// then Stop and Ctrl+Z down to nothing. As [`edit_during_a_song_take`].
fn edit_during_a_song_take_on_its_own_track(
    test: &str,
    edit: dawsome_engine::edit::Edit,
) -> (bool, Vec<String>, bool) {
    use dawsome_engine::project::{Clip, TrackId};
    let (mut app, mut player, _dir) = open(test, "p17_song.json");
    let fixture_text = document(&app);
    let clips = (2..=8)
        .map(|track| Clip::of_pattern(PatternId(1), TrackId(track), Ticks(384), Ticks(384)))
        .collect();
    app.commit(dawsome_engine::edit::Edit::PlaceClips {
        first: ClipId(2),
        clips,
    });
    record(&mut app, RecordMessage::ToggleCountIn);
    let _ = app.update(Message::ModeSelected(PlayMode::Song));
    app.move_song_marker(Ticks(384));
    record(&mut app, RecordMessage::ToggleArm);
    let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
    let script = [(10, vec![0x90, 60, 100]), (20, vec![0x80, 60, 0])];
    let _ = app.update(Message::Play);
    assert!(app.recording.taking_notes());
    assert_eq!(
        app.session.project().playlist.tracks.len(),
        9,
        "the take added track 9"
    );
    let mut went_through = false;
    for call in 0..60 {
        play_call(&mut app, &mut player, &device, &script, call, CALL);
        if call == 40 {
            let before = document(&app);
            app.commit(edit.clone());
            went_through = document(&app) != before;
            if !went_through {
                assert_eq!(app.error.as_deref(), Some("Stop the recording first"));
            }
        }
    }
    let _ = app.update(Message::Stop);
    let mut refusals = Vec::new();
    while app.session.history().undo_len() > 0 {
        let left = app.session.history().undo_len();
        let _ = app.update(Message::Undo);
        if app.session.history().undo_len() == left {
            refusals.push(app.error.clone().unwrap_or_default());
            break;
        }
    }
    (went_through, refusals, document(&app) == fixture_text)
}

// A song take that had to add a track keeps that track from changing while it
// runs: muting it, moving another clip onto it, renaming it are refused, so every undo after
// Stop goes through and the document comes back to the fixture. Muting track 8 (not the take's)
// still goes through and undoes.
#[test]
fn edits_to_the_track_a_song_take_added_are_refused() {
    use dawsome_engine::edit::Edit;
    use dawsome_engine::project::TrackId;
    for (test, edit) in [
        (
            "added_track_mute",
            Edit::SetTrackMute {
                id: TrackId(9),
                mute: true,
            },
        ),
        (
            "added_track_clip",
            Edit::MoveClips {
                moves: vec![(ClipId(1), Ticks(0), TrackId(9))],
            },
        ),
        (
            "added_track_rename",
            Edit::RenameTrack {
                id: TrackId(9),
                name: "Mine".to_owned(),
            },
        ),
    ] {
        let (went_through, refusals, back) = edit_during_a_song_take_on_its_own_track(test, edit);
        assert!(!went_through, "{test}: refused");
        assert_eq!(refusals, Vec::<String>::new(), "{test}");
        assert!(back, "{test}: the document back to the fixture");
    }
    let (went_through, refusals, back) = edit_during_a_song_take_on_its_own_track(
        "other_track_mute",
        Edit::SetTrackMute {
            id: TrackId(8),
            mute: true,
        },
    );
    assert!(went_through, "another track's mute goes through");
    assert_eq!(refusals, Vec::<String>::new());
    assert!(back);
}

// Found in use ("visual glitches as the piano roll moves" in a song take). Filmed with
// --null-output --rehearse, the roll's playhead and Follow jumped back to the pattern's start
// every bar: they took the song's tick modulo the take's pattern, which grows a bar at a time
// from the song marker, not the tick from the take's start. Here, a take from 384 over 6 bars
// with the roll open at 8 px a step (Follow pages it): after every call the roll's playhead is
// the song's tick less 384, the roll shows the take's pattern at its length now, and the view
// never scrolls back while the playhead goes forward.
#[test]
fn the_roll_follows_a_song_take_from_its_start() {
    let (mut app, mut player, _dir) = open("roll_follows_take", "p17_song.json");
    let _ = app.update(Message::TogglePanel(crate::ui::panels::Panel::PianoRoll));
    app.roll.view.zoom = 8;
    record(&mut app, RecordMessage::ToggleCountIn);
    let _ = app.update(Message::ModeSelected(PlayMode::Song));
    app.move_song_marker(Ticks(384));
    record(&mut app, RecordMessage::ToggleArm);
    let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
    let _ = app.update(Message::Play);
    let mut scroll = app.roll.view.scroll;
    let mut paged = 0;
    for call in 0..1100 {
        play_call(&mut app, &mut player, &device, &[], call, CALL);
        let Some(at) = app.playhead else { continue };
        assert_eq!(
            app.roll_playhead(),
            Some(Ticks(at.0 - 384)),
            "call {call}: the roll's playhead from the take's start"
        );
        let target = crate::ui::note_gesture::Target::of(
            app.session.project(),
            app.pattern,
            app.selected_channel,
        )
        .expect("the take's pattern");
        let bars = app.session.project().patterns[&PatternId(2)].length_bars;
        assert_eq!(target.end(), Ticks(i64::from(bars) * 384), "call {call}");
        assert!(
            app.roll.view.scroll >= scroll,
            "call {call}: the view went back"
        );
        paged += usize::from(app.roll.view.scroll > scroll);
        scroll = app.roll.view.scroll;
    }
    assert!(paged >= 2, "Follow paged the view {paged} times");
    let _ = app.update(Message::Stop);
}

// Found in use (a screencast of a song take, frame by frame at 10 fps): a key struck
// drew, for one frame, a bar across the whole pattern, then its short one at the playhead. The
// window reads what the player posted between its playhead timer's ticks too (a MIDI port's
// arrival, a key typed), and drew a note heard after the playhead was last read: started past
// the playhead, it read as wrapped round the pattern. By hand, 44.1 kHz at 120 BPM (229.6875
// samples a tick), 512 frames a call: from the take's start (tick 384 in song mode, 0 in pattern
// mode) the timer reads the playhead after 100 calls (51 200 samples, tick 222 from the start);
// the player runs one call more on its own (tick 225), then the device's C5 comes and the next
// call hears it at the call's first sample, tick 225 from the start, and ends at 227 (52 224
// samples). The port's arrival is read before the next tick: the roll draws one bar from 225 to
// the playhead read with it, 227 (2 px at 1.5 ticks a pixel, the least bar), never the pattern.
#[test]
fn a_key_heard_between_the_playheads_ticks_draws_from_its_start() {
    for (mode, marker) in [(PlayMode::Song, 384), (PlayMode::Pattern, 0)] {
        let (mut app, mut player, _dir) = open("key_between_ticks", "p17_song.json");
        let _ = app.update(Message::TogglePanel(crate::ui::panels::Panel::PianoRoll));
        record(&mut app, RecordMessage::ToggleCountIn);
        let _ = app.update(Message::ModeSelected(mode));
        app.move_song_marker(Ticks(marker));
        record(&mut app, RecordMessage::ToggleArm);
        let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
        let _ = app.update(Message::Play);
        for call in 0..100 {
            play_call(&mut app, &mut player, &device, &[], call, CALL);
        }
        assert_eq!(app.playhead, Some(Ticks(marker + 222)), "{mode:?}");
        player.process(&mut [[0.0; 2]; CALL]);
        device.deliver(&[0x90, 60, 100]);
        player.process(&mut [[0.0; 2]; CALL]);
        let _ = app.update(Message::MidiActivity);
        let held = app.live_notes.of(app.selected_channel.expect("a channel"));
        assert_eq!(
            held.iter()
                .map(|&(key, held)| (key, held.tick))
                .collect::<Vec<_>>(),
            [(60, Some(Ticks(marker + 225)))],
            "{mode:?}: heard at the call's first sample"
        );
        assert_eq!(app.roll_origin(), Ticks(marker), "{mode:?}");
        let view = app.roll.view;
        let left = view.x_of(Ticks(225));
        let right = view
            .x_of(Ticks(227))
            .max(left + crate::ui::live_notes::LEAST_BAR);
        let row = view.row_top(60);
        let one = iced::Rectangle::new(
            iced::Point::new(left, row),
            iced::Size::new(right - left, crate::ui::piano_layout::ROW_HEIGHT - 1.0),
        );
        assert_eq!(
            app.roll_live_bars(60),
            [one],
            "{mode:?}: playhead {:?}",
            app.playhead
        );
        device.deliver(&[0x80, 60, 0]);
        play_call(&mut app, &mut player, &device, &[], 102, CALL);
        let _ = app.update(Message::Stop);
    }
}
