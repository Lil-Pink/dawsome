//! The rack's and the playlist's own edits: a channel renamed, deleted with its notes and put
//! back, cloned and the clone taken out; a track deleted with its clips and put back. Each
//! expected document is the fixture with the change written out by hand, and each inverse
//! gives the fixture's bytes back.

use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::edit::{self, Edit, EditError};
use dawsome_engine::project::{ChannelId, Project, TrackId};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn fixture(name: &str) -> (String, Project) {
    let text = fs::read_to_string(fixtures().join(name)).expect("a fixture");
    let project = Project::load_json(&text).expect("loads");
    (text.trim_end().to_owned(), project)
}

/// Applies `edit`, checks the document is `expected`, then that the inverse gives `before`'s
/// bytes back, and that the inverse's inverse gives `expected` again (a redo).
fn round_trip(project: &mut Project, edit: Edit, before: &str, expected: &str) {
    let applied = edit::apply(project, edit).expect("applies");
    assert_eq!(project.to_json().expect("writes"), expected);
    project.validate().expect("valid");
    let redo = edit::apply(project, applied.inverse).expect("undoes");
    assert_eq!(
        project.to_json().expect("writes"),
        before,
        "undone byte for byte"
    );
    edit::apply(project, redo.inverse).expect("redoes");
    assert_eq!(project.to_json().expect("writes"), expected, "redone");
}

// p4_two_clips: channel 1 "kick" renamed "Kick drum"; then channel 1 deleted, its six notes in
// patterns 1 and 2 with it, the counter still 3.
#[test]
fn a_channel_renames_and_deletes_with_its_notes() {
    let (before, mut project) = fixture("p4_two_clips.json");
    let renamed = before.replacen(r#""name":"kick""#, r#""name":"Kick drum""#, 1);
    round_trip(
        &mut project,
        Edit::RenameChannel {
            id: ChannelId(1),
            name: "Kick drum".to_owned(),
        },
        &before,
        &renamed,
    );
    let (before, mut project) = fixture("p4_two_clips.json");
    let kick = r#"{"id":1,"name":"kick","kind":"sampler","sample":"impulse.wav","volume":1.0,"pan":0.0,"mute":false},"#;
    let one_notes = r#"{"channel":1,"tick":0,"length":0,"velocity":1.0},{"channel":1,"tick":96,"length":0,"velocity":1.0},{"channel":1,"tick":192,"length":0,"velocity":1.0},{"channel":1,"tick":288,"length":0,"velocity":1.0}"#;
    let two_notes = r#"{"channel":1,"tick":48,"length":0,"velocity":1.0},{"channel":1,"tick":144,"length":0,"velocity":1.0}"#;
    let deleted = before
        .replacen(kick, "", 1)
        .replacen(one_notes, "", 1)
        .replacen(two_notes, "", 1);
    assert!(deleted.contains(r#""next_channel_id":3"#));
    round_trip(
        &mut project,
        Edit::DeleteChannel { id: ChannelId(1) },
        &before,
        &deleted,
    );
}

// p4_two_clips: channel 2 "hat" cloned as channel 3 "hat #2", its settings and no note, the
// counter 4; taking the clone out gives the counter back.
#[test]
fn a_channel_clones_without_its_notes() {
    let (before, mut project) = fixture("p4_two_clips.json");
    let hat = r#"{"id":2,"name":"hat","kind":"sampler","sample":"impulse.wav","volume":1.0,"pan":0.0,"mute":false}"#;
    let clone = r#"{"id":3,"name":"hat #2","kind":"sampler","sample":"impulse.wav","volume":1.0,"pan":0.0,"mute":false}"#;
    let cloned = before
        .replacen(r#""next_channel_id":3"#, r#""next_channel_id":4"#, 1)
        .replacen(hat, &format!("{hat},{clone}"), 1);
    round_trip(
        &mut project,
        Edit::CloneChannel {
            id: ChannelId(3),
            from: ChannelId(2),
        },
        &before,
        &cloned,
    );
    assert_eq!(
        edit::apply(
            &mut project,
            Edit::CloneChannel {
                id: ChannelId(9),
                from: ChannelId(2)
            }
        ),
        Err(EditError::NotNextChannelId {
            got: 9,
            expected: 4
        })
    );
}

// A channel an audio clip plays is refused, changing nothing; p8_ui's audio clips play
// channel 2.
#[test]
fn a_channel_with_audio_clips_is_not_deleted() {
    let (before, mut project) = fixture("p8_ui.json");
    let audio = project
        .playlist
        .clips
        .values()
        .find_map(|clip| clip.audio_channel())
        .expect("an audio clip");
    assert_eq!(
        edit::apply(&mut project, Edit::DeleteChannel { id: audio }),
        Err(EditError::ChannelPlaysClips(audio.0))
    );
    assert_eq!(project.to_json().expect("writes"), before);
}

// p4_two_clips: track 1 deleted with its two clips (bars 1 and 3), the counters where they
// were.
#[test]
fn a_track_deletes_with_its_clips() {
    let (before, mut project) = fixture("p4_two_clips.json");
    let track = r#"{"id":1,"name":"Track 1","mute":false},"#;
    let clips: Vec<_> = project
        .playlist
        .clips
        .iter()
        .filter(|(_, clip)| clip.track == TrackId(1))
        .map(|(&id, _)| id)
        .collect();
    assert_eq!(clips.len(), 2, "two clips on track 1");
    let mut expected = project.clone();
    expected.playlist.tracks.remove(&TrackId(1));
    for clip in &clips {
        expected.playlist.clips.remove(clip);
    }
    let expected = expected.to_json().expect("writes");
    assert!(!expected.contains(track) && expected.contains(r#""next_track_id":9"#));
    round_trip(
        &mut project,
        Edit::DeleteTrack { id: TrackId(1) },
        &before,
        &expected,
    );
}
