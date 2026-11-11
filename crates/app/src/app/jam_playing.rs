//! The Jam rolled while its pattern plays (Roll pressed fast on EDM / house was heard as "some
//! of the notes got stuck"). A roll replaces the pattern's notes under the playhead; every
//! note already sounding carries its own end from the frame it started (a voice's or zone's `off`,
//! a plugin note's length in the note schedule, the synth's gate), so it rings to that end and
//! nothing has to end it at the plan swap. These tests hold the player to that through the Jam
//! window's own messages: nothing outlives its note, and once the old notes have ended the player
//! plays what the last roll wrote and nothing else.

use std::fs;
use std::path::PathBuf;

use dawsome_engine::jam::Genre;
use dawsome_engine::project::{ChannelId, PatternId};
use dawsome_engine::realtime::Player;
use dawsome_host_clap::ClapHost;

use super::plugins::app_tests::{app_hosted, catalogue_of, test_plugin_file};
use super::{App, Message};
use crate::document::Host;
use crate::ui::jam::{GenreChoice, JamMessage, RoleName};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

/// Frames per audio call: the device's usual block.
const BLOCK: usize = 512;

/// A bar at house's 126 BPM is 91 428 frames at 48 kHz: 179 blocks, rounded up.
const BAR: usize = 179;

/// Blocks between two rolls: 5120 frames, 107 ms, a fast click's spacing.
const BETWEEN_ROLLS: usize = 10;

/// How far two runs that played the same notes may differ on a frame. Why not 0: which voice
/// slot a note takes follows what played before it, and the slots' order is the order their
/// floats are summed in, so two histories can round a frame an ulp or two apart (4.8e-7 at a
/// level of 3.3 measured). A note that should have ended differs by its whole
/// level.
const SAME: f32 = 1e-5;

/// A folder of its own for a test's project, removed when dropped.
struct Folder(PathBuf);

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The band's fourth channel, the melody's.
#[derive(Clone, Copy)]
enum Lead {
    /// Dawsome Synth, which sounds while a note's gate is open and then for its release.
    Synth,
    /// The tone set's one zone again. Why: a synth's oscillators run on from its last note, so
    /// two runs with different pasts never match frame for frame, while a set's zones read
    /// their sound from its start.
    Tone,
}

/// A band in a project of its own: drums on the Jam fixtures' drum set, the bass on their tone
/// set, the chords on the CLAP test sine (which sounds from a note's on to its off, so a note
/// never ended would sound for good) and the melody on `lead`; the Jam window open on the four
/// with every part as its name says, EDM / house, seeds from `seed`. Pattern 1 is empty.
fn band(test: &str, lead: Lead, seed: u64) -> (Folder, App, Player) {
    let dir = std::env::temp_dir().join(format!("dawsome-jam-c3-{}-{test}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("a folder");
    let set = |id: u32, name: &str, set: &str| {
        format!(
            r#"{{"id":{id},"name":"{name}","kind":"instrument","instrument":"{FIXTURES}/p36_sets/{set}","volume":1.0,"pan":0.0,"mute":false}}"#
        )
    };
    let sine = r#"{"id":3,"name":"Keys","kind":"plugin","processor":{"id":"clap:org.dawsome.test.sine","name":"Dawsome Test Sine","state":"T1RQMQAAAAAAAOA/"},"volume":1.0,"pan":0.0,"mute":false}"#;
    let lead = match lead {
        Lead::Synth => r#"{"id":4,"name":"Lead","kind":"synth","processor":{"id":"dawsome.synth","params":{}},"volume":1.0,"pan":0.0,"mute":false}"#.to_owned(),
        Lead::Tone => set(4, "Lead", "tone/tone.sfz"),
    };
    let project = format!(
        r#"{{"format_version":9,"sample_rate":48000,"tempo_mbpm":120000,"ppq":96,"next_channel_id":5,"channels":[{},{},{sine},{lead}],"patterns":[{{"id":1,"name":"Pattern 1","length_bars":1,"notes":[]}}]}}"#,
        set(1, "Drums", "drums/drums.sfz"),
        set(2, "Bass", "tone/tone.sfz"),
    );
    let file = dir.join("band.json");
    fs::write(&file, project).expect("the project");
    let host = Host {
        clap: ClapHost::new(catalogue_of(test_plugin_file())),
        ..Host::offline()
    };
    let (mut app, player) = app_hosted(&file, host);
    let _ = app.update(Message::Jam(JamMessage::Start));
    for (channel, role) in [
        (1, RoleName::Drums),
        (2, RoleName::Bass),
        (3, RoleName::Chords),
        (4, RoleName::Melody),
    ] {
        let _ = app.update(Message::Jam(JamMessage::RoleChosen(
            ChannelId(channel),
            role,
        )));
    }
    let _ = app.update(Message::Jam(JamMessage::Next));
    let house = GenreChoice(Genre::House);
    let _ = app.update(Message::Jam(JamMessage::Genre(house)));
    // Auto off, house's 126 typed: Auto draws each roll's tempo, and a run of many rolls would
    // then play on another timeline than the run it is compared with, which rolls once.
    let _ = app.update(Message::Jam(JamMessage::AutoTempo(false)));
    let _ = app.update(Message::Jam(JamMessage::TempoEdited("126".to_owned())));
    app.jam.next_seed = seed;
    (Folder(dir), app, player)
}

/// `blocks` audio calls, the UI's tick after each, as the window's timer runs beside the
/// device: the frames played.
fn play(app: &mut App, player: &mut Player, blocks: usize) -> Vec<[f32; 2]> {
    let mut played = Vec::with_capacity(blocks * BLOCK);
    for _ in 0..blocks {
        let mut out = [[0.0; 2]; BLOCK];
        player.process(&mut out);
        let _ = app.update(Message::Tick);
        played.extend_from_slice(&out);
    }
    played
}

/// Play pressed a click's time after what came before: [`BETWEEN_ROLLS`] audio calls, then
/// Play. Why the wait: a roll sends two plans (its notes, under the selection as it was, then the
/// selection), the player takes one a call, and a Play in the call that takes the first starts
/// the pattern selected before, so the new one's first notes fall behind the playhead (a roll
/// and a Play inside ~10 ms, which no hand gives).
fn press_play(app: &mut App, player: &mut Player) {
    let _ = play(app, player, BETWEEN_ROLLS);
    let _ = app.update(Message::Play);
}

/// The loudest sample of `frames`.
fn peak(frames: &[[f32; 2]]) -> f32 {
    frames
        .iter()
        .map(|[left, right]| left.abs().max(right.abs()))
        .fold(0.0, f32::max)
}

/// The largest difference between two runs' frames.
fn furthest(a: &[[f32; 2]], b: &[[f32; 2]]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(a, b)| (a[0] - b[0]).abs().max((a[1] - b[1]).abs()))
        .fold(0.0, f32::max)
}

/// The seed the last roll took.
fn last_seed(app: &App) -> u64 {
    app.jam
        .session
        .as_ref()
        .and_then(|session| session.seed)
        .expect("rolled")
}

/// Twenty rolls ~107 ms apart while the first roll's pattern plays, two bars with none, the
/// empty Pattern 1 selected and two bars more (the last notes' ends, the synth's release and
/// the drum set's hit are long past by then): the next bar is silent to the last bit. A note
/// one of the swaps left without an end (the sine's, the synth's, a zone's) would still sound.
#[test]
fn twenty_rolls_while_playing_leave_no_note_sounding_past_its_end() {
    let (_folder, mut app, mut player) = band("stuck", Lead::Synth, 1);
    let _ = app.update(Message::Jam(JamMessage::Roll));
    press_play(&mut app, &mut player);
    assert!(
        peak(&play(&mut app, &mut player, BAR)) > 0.1,
        "the Jam plays"
    );
    for _ in 0..20 {
        let _ = app.update(Message::Jam(JamMessage::Roll));
        let _ = play(&mut app, &mut player, BETWEEN_ROLLS);
    }
    assert_eq!(last_seed(&app), 21, "twenty rolls after the first");
    let _ = play(&mut app, &mut player, 2 * BAR);
    app.select_pattern(PatternId(1));
    let _ = play(&mut app, &mut player, 2 * BAR);
    assert_eq!(
        peak(&play(&mut app, &mut player, BAR)),
        0.0,
        "a note sounds on"
    );
}

/// The same twenty rolls, then two bars: the next bar is the one a run that rolled the last
/// seed alone and played as long plays, within [`SAME`]. Notes of several rolls interleaved, a
/// voice piling up per roll, or one never ended would each differ by a note's level.
#[test]
fn after_twenty_rolls_the_player_plays_the_last_roll_and_nothing_else() {
    let (_folder, mut app, mut player) = band("last", Lead::Tone, 1);
    let _ = app.update(Message::Jam(JamMessage::Roll));
    press_play(&mut app, &mut player);
    let _ = play(&mut app, &mut player, BAR);
    for _ in 0..20 {
        let _ = app.update(Message::Jam(JamMessage::Roll));
        let _ = play(&mut app, &mut player, BETWEEN_ROLLS);
    }
    let _ = play(&mut app, &mut player, 2 * BAR);
    let rolled = play(&mut app, &mut player, BAR);
    let jam = app.pattern.expect("the Jam selected");

    let (_alone_folder, mut alone, mut alone_player) = band("last-alone", Lead::Tone, 21);
    let _ = alone.update(Message::Jam(JamMessage::Roll));
    let written = alone.pattern.expect("rolled");
    assert_eq!(
        app.session.project().patterns[&jam].notes,
        alone.session.project().patterns[&written].notes,
        "the document holds the last roll's notes alone"
    );
    press_play(&mut alone, &mut alone_player);
    let _ = play(
        &mut alone,
        &mut alone_player,
        BAR + 20 * BETWEEN_ROLLS + 2 * BAR,
    );
    let reference = play(&mut alone, &mut alone_player, BAR);
    assert!(peak(&reference) > 0.1, "the reference plays");
    let off = furthest(&rolled, &reference);
    assert!(off <= SAME, "differs from the last roll alone by {off}");
}

/// Rolls while stopped play nothing (a roll plays nothing by itself), and Play then plays the
/// last one as a run that rolled it alone does.
#[test]
fn rolls_while_stopped_leave_nothing_sounding_and_play_plays_the_last() {
    let (_folder, mut app, mut player) = band("stopped", Lead::Tone, 1);
    for _ in 0..5 {
        let _ = app.update(Message::Jam(JamMessage::Roll));
        let _ = play(&mut app, &mut player, BETWEEN_ROLLS);
    }
    assert_eq!(
        peak(&play(&mut app, &mut player, BAR)),
        0.0,
        "rolls play nothing"
    );
    press_play(&mut app, &mut player);
    let rolled = play(&mut app, &mut player, 2 * BAR);

    let (_alone_folder, mut alone, mut alone_player) = band("stopped-alone", Lead::Tone, 5);
    let _ = alone.update(Message::Jam(JamMessage::Roll));
    press_play(&mut alone, &mut alone_player);
    let reference = play(&mut alone, &mut alone_player, 2 * BAR);
    assert!(peak(&reference) > 0.1, "the reference plays");
    let off = furthest(&rolled, &reference);
    assert!(off <= SAME, "differs from the last roll alone by {off}");
}

/// Accept while Jam 1 plays, then a roll: it writes Jam 2, selected, and the player moves to it
/// as to any pattern selected (Jam 1's notes ring to their ends); two bars on, it plays Jam 2 as
/// a run that rolled Jam 2's seed alone does, and the empty pattern then falls silent.
#[test]
fn accept_then_a_roll_while_playing_plays_the_new_jam_alone() {
    let (_folder, mut app, mut player) = band("accept", Lead::Tone, 1);
    let _ = app.update(Message::Jam(JamMessage::Roll));
    press_play(&mut app, &mut player);
    let _ = play(&mut app, &mut player, BAR);
    let _ = app.update(Message::Jam(JamMessage::Accept));
    let _ = app.update(Message::Jam(JamMessage::Roll));
    assert_eq!(app.pattern, Some(PatternId(3)), "Jam 2 selected");
    assert_eq!(last_seed(&app), 2);
    let _ = play(&mut app, &mut player, 2 * BAR);
    let rolled = play(&mut app, &mut player, BAR);

    let (_alone_folder, mut alone, mut alone_player) = band("accept-alone", Lead::Tone, 2);
    let _ = alone.update(Message::Jam(JamMessage::Roll));
    press_play(&mut alone, &mut alone_player);
    let _ = play(&mut alone, &mut alone_player, 3 * BAR);
    let reference = play(&mut alone, &mut alone_player, BAR);
    let off = furthest(&rolled, &reference);
    assert!(off <= SAME, "differs from Jam 2 alone by {off}");

    app.select_pattern(PatternId(1));
    let _ = play(&mut app, &mut player, 2 * BAR);
    assert_eq!(
        peak(&play(&mut app, &mut player, BAR)),
        0.0,
        "a note sounds on"
    );
}
