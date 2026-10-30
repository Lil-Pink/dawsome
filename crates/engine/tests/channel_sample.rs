//! A sampler channel takes a sound file after it was made: one edit, whose inverse gives the
//! old file back (an empty path too) byte for byte. Expected documents are written out by hand.

use std::collections::BTreeSet;
use std::path::PathBuf;

use dawsome_engine::edit::{self, Edit, EditError, Touched};
use dawsome_engine::project::{Channel, ChannelId, Project};
use dawsome_engine::time::MilliBpm;

const F1: &str = include_str!("../../../tests/fixtures/f1_four_on_floor.json");
const P7_SINE: &str = include_str!("../../../tests/fixtures/p7_sine.json");
const P8_UI: &str = include_str!("../../../tests/fixtures/p8_ui.json");

fn json(project: &Project) -> String {
    project.to_json().expect("serializes")
}

fn set(id: u32, path: &str) -> Edit {
    Edit::SetChannelSample {
        id: ChannelId(id),
        path: PathBuf::from(path),
    }
}

/// Applies `edit`, checks the document against `expected`, then that its inverse gives the
/// document back byte for byte and that the redo makes `expected` again.
fn walks(project: &mut Project, edit: Edit, expected: &str) -> Touched {
    let before = json(project);
    let applied = edit::apply(project, edit).expect("applies");
    assert_eq!(json(project), expected);
    let undone = edit::apply(project, applied.inverse).expect("undoes");
    assert_eq!(json(project), before, "undo restores the bytes");
    edit::apply(project, undone.inverse).expect("redoes");
    assert_eq!(json(project), expected, "redo makes it again");
    applied.touched
}

// The new document's "Sampler" has no file: given impulse.wav it plays it, and undo takes it
// back to no file at all. The channel is all the compiler has to look at again.
#[test]
fn the_empty_sampler_takes_a_file_and_undo_empties_it_again() {
    let mut project = Project::new(44_100, MilliBpm(120_000), 96);
    project
        .channels
        .insert(ChannelId(1), Channel::sampler("Sampler", ""));
    project.next_channel_id = ChannelId(2);
    let channel = |sample: &str| {
        format!(
            r#"{{"id":1,"name":"Sampler","kind":"sampler","sample":"{sample}","volume":1.0,"pan":0.0,"mute":false}}"#
        )
    };
    let document = |sample: &str| {
        format!(
            r#"{{"format_version":2,"sample_rate":44100,"tempo_mbpm":120000,"ppq":96,"next_channel_id":2,"channels":[{}],"patterns":[]}}"#,
            channel(sample)
        )
    };
    assert_eq!(json(&project), document(""));
    let touched = walks(
        &mut project,
        set(1, "impulse.wav"),
        &document("impulse.wav"),
    );
    assert_eq!(
        touched,
        Touched {
            channels: BTreeSet::from([ChannelId(1)]),
            ..Touched::default()
        }
    );
}

// F1's kick changes file the same way, and keeps its name: a sampler named after its first file
// is the user's to rename.
#[test]
fn a_sampler_with_a_file_takes_another() {
    let mut project = Project::load_json(F1).expect("loads");
    let expected =
        json(&project).replace(r#""sample":"impulse.wav""#, r#""sample":"hold2000.wav""#);
    walks(&mut project, set(1, "hold2000.wav"), &expected);
}

// Only a sampler takes a file this way: a plugin channel plays its plugin, and an audio
// channel's clips are measured in its file's frames. A refusal changes nothing.
#[test]
fn only_a_sampler_takes_a_file() {
    for (text, edit, error) in [
        (P7_SINE, set(1, "impulse.wav"), EditError::NotASampler(1)),
        (P8_UI, set(1, "impulse.wav"), EditError::NotASampler(1)),
        (F1, set(9, "impulse.wav"), EditError::UnknownChannel(9)),
    ] {
        let mut project = Project::load_json(text).expect("loads");
        let before = json(&project);
        assert_eq!(
            edit::apply(&mut project, edit.clone()),
            Err(error),
            "{edit:?}"
        );
        assert_eq!(json(&project), before, "{edit:?} changed the project");
    }
}
