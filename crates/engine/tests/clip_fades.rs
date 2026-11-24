//! An audio clip's fades as edits and in the file: gen_p38_expected.py's fade edits (asked
//! lengths clamped to the clip, the document after, the undo's bytes), the version a faded
//! project writes and the refusals around it, and the audio clip limit on every edit that adds
//! audio clips.

use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::edit::{self, Edit, EditError};
use dawsome_engine::project::{
    ChannelId, Clip, ClipId, ClipItem, LoadError, Project, TrackId, ValidationError,
    MAX_AUDIO_CLIPS,
};
use dawsome_engine::time::Ticks;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn text(name: &str) -> String {
    fs::read_to_string(fixtures().join(name)).unwrap_or_else(|_| panic!("{name}"))
}

fn expected() -> serde_json::Value {
    serde_json::from_str(&text("p38_expected.json")).expect("p38_expected.json parses")
}

fn number(value: &serde_json::Value) -> u64 {
    value.as_u64().expect("a number")
}

// Each fade edit gen_p38 asks: the document after is the generator's (fades clamped so both fit
// the clip, written only when set, the version 12 while one is and the file's own otherwise),
// and its inverse gives back the file's bytes.
#[test]
fn p38_a_fade_edit_clamps_to_the_clip_and_undoes_to_the_bytes() {
    for case in expected()["set_fades"].as_array().expect("set_fades") {
        let before = text(case["fixture"].as_str().expect("fixture"));
        let mut project = Project::load_json(&before).expect("loads");
        let change = Edit::SetClipFades {
            clip: ClipId(number(&case["clip"]) as u32),
            fade_in: number(&case["fade_in"]),
            fade_out: number(&case["fade_out"]),
        };
        let applied = edit::apply(&mut project, change).expect("applies");
        assert_eq!(
            project.to_json().expect("saves"),
            case["after"].as_str().expect("after"),
            "{case}"
        );
        edit::apply(&mut project, applied.inverse).expect("undoes");
        let undone = project.to_json().expect("saves");
        assert_eq!(undone, case["undo"].as_str().expect("undo"), "{case}");
        assert_eq!(undone + "\n", before, "{case}");
    }
}

// Fades on a pattern clip or a clip that is not there are refused, the project unchanged.
#[test]
fn a_fade_edit_needs_an_audio_clip() {
    let mut project = Project::load_json(&text("p37_cut_two.json")).expect("loads");
    let before = project.to_json().expect("saves");
    let pattern = Edit::SetClipFades {
        clip: ClipId(1),
        fade_in: 10,
        fade_out: 0,
    };
    assert!(matches!(
        edit::apply(&mut project, pattern),
        Err(EditError::NotAnAudioClip(1))
    ));
    let gone = Edit::SetClipFades {
        clip: ClipId(99),
        fade_in: 10,
        fade_out: 0,
    };
    assert!(matches!(
        edit::apply(&mut project, gone),
        Err(EditError::UnknownClip(99))
    ));
    assert_eq!(project.to_json().expect("saves"), before);
}

// A faded project writes version 12 and reads back to the same bytes; one whose clips only cut
// into their sound (the declick, no field) keeps version 2. A build that reads up to 11 refuses
// version 12 by its number, as this build's tests held before it knew 12, and one that reads up
// to 12 refuses 13 so; this build refuses 14, naming 13.
#[test]
fn p38_a_faded_project_is_version_12_and_a_newer_one_is_refused() {
    let want = expected();
    let documents = &want["documents"];
    for kind in ["faded", "declick_only"] {
        let case = &documents[kind];
        let file = text(case["fixture"].as_str().expect("fixture"));
        let project = Project::load_json(&file).expect("loads");
        let written = project.to_json().expect("saves");
        assert_eq!(written.clone() + "\n", file, "{kind}: the same bytes");
        let version = format!("{{\"format_version\":{},", case["format_version"]);
        assert!(written.starts_with(&version), "{kind}: {written}");
    }
    let faded = text("p38_both.json");
    assert!(faded.starts_with(r#"{"format_version":12,"#));
    let refused = documents["refused_by_11"].as_str().expect("refused_by_11");
    assert!(refused.starts_with("format_version 12 is not supported"));
    let by_12 = documents["refused_13"].as_str().expect("refused_13");
    assert!(by_12.starts_with("format_version 13 is not supported"));
    let newer = faded.replacen(r#""format_version":12"#, r#""format_version":14"#, 1);
    let error = Project::load_json(&newer).expect_err("refused");
    assert!(
        matches!(error, LoadError::UnsupportedVersion(14)),
        "{error}"
    );
    assert_eq!(
        error.to_string(),
        "format_version 14 is not supported (this build reads versions 1, 2, 4, 5, 6, 7, 8, 9, \
         10, 11, 12 and 13)"
    );
}

/// p37_cut_two with audio clips placed (copies of clip 4 on track 6) until the song holds
/// `count` of them.
fn with_audio_clips(count: usize) -> Project {
    let mut project = Project::load_json(&text("p37_cut_two.json")).expect("loads");
    let model = project.playlist.clips[&ClipId(4)].clone();
    let held = |project: &Project| {
        let clips = project.playlist.clips.values();
        clips.filter(|clip| clip.audio_channel().is_some()).count()
    };
    let missing = count - held(&project);
    let clips = (0..missing)
        .map(|k| Clip {
            track: TrackId(6),
            start: Ticks(k as i64 % 200),
            ..model.clone()
        })
        .collect();
    let first = project.playlist.next_clip_id;
    edit::apply(&mut project, Edit::PlaceClips { first, clips }).expect("places");
    assert_eq!(held(&project), count);
    project
}

// At 4096 audio clips, placing, duplicating or adding one more is refused with the limit's
// error and the project unchanged; a pattern clip still places. Up to the limit, all go in.
#[test]
fn edits_that_add_audio_clips_stop_at_the_limit() {
    let mut project = with_audio_clips(MAX_AUDIO_CLIPS);
    let before = project.to_json().expect("saves");
    let audio = project.playlist.clips[&ClipId(4)].clone();
    let first = project.playlist.next_clip_id;
    let refused = |result: Result<edit::Applied, EditError>| {
        matches!(
            result,
            Err(EditError::Invalid(ValidationError::TooManyAudioClips(4097)))
        )
    };
    let place = Edit::PlaceClips {
        first,
        clips: vec![audio.clone()],
    };
    assert!(refused(edit::apply(&mut project, place)), "placed");
    let duplicate = Edit::DuplicateClips {
        ids: vec![ClipId(4)],
        first,
    };
    assert!(refused(edit::apply(&mut project, duplicate)), "duplicated");
    let add = Edit::AddAudioClip {
        channel: project.next_channel_id,
        name: "more".into(),
        sample: "p37_sound/ramp.wav".into(),
        clip: first,
        track: TrackId(7),
        start: Ticks(0),
        length: Ticks(96),
    };
    assert!(refused(edit::apply(&mut project, add)), "added");
    assert_eq!(project.to_json().expect("saves"), before, "unchanged");
    let pattern = project.playlist.clips[&ClipId(1)].clone();
    let place = Edit::PlaceClips {
        first,
        clips: vec![pattern],
    };
    edit::apply(&mut project, place).expect("a pattern clip places");
    let mut under = with_audio_clips(MAX_AUDIO_CLIPS - 1);
    let first = under.playlist.next_clip_id;
    let place = Edit::PlaceClips {
        first,
        clips: vec![audio],
    };
    edit::apply(&mut under, place).expect("the 4096th places");
    let ClipItem::Audio { channel, .. } = under.playlist.clips[&first].item else {
        panic!("an audio clip");
    };
    assert_eq!(channel, ChannelId(2));
}

/// Clip 1's (fade in, fade out, length in ticks).
fn fades_of(project: &Project) -> (u64, u64, i64) {
    let clip = &project.playlist.clips[&ClipId(1)];
    let ClipItem::Audio {
        fade_in, fade_out, ..
    } = clip.item
    else {
        panic!("an audio clip");
    };
    (fade_in, fade_out, clip.length.0)
}

/// `project` with clip 1 resized to `ticks`: a resize keeps the fades as they are stored, so
/// they can be longer than the clip.
fn resized(mut project: Project, ticks: i64) -> Project {
    let resize = Edit::ResizeClip {
        id: ClipId(1),
        length: Ticks(ticks),
    };
    edit::apply(&mut project, resize).expect("resizes");
    project
}

// p38_fade_in's clip (fade in 1000 frames) resized to 2 ticks (500 frames), its fades taken off,
// then undone: the fade in is 1000 again, as stored, not cut to the clip's 500.
#[test]
fn undoing_a_fade_edit_restores_fades_longer_than_the_clip() {
    let mut project = resized(
        Project::load_json(&text("p38_fade_in.json")).expect("loads"),
        2,
    );
    assert_eq!(fades_of(&project), (1000, 0, 2));
    let before = project.to_json().expect("saves");
    let off = Edit::SetClipFades {
        clip: ClipId(1),
        fade_in: 0,
        fade_out: 0,
    };
    let applied = edit::apply(&mut project, off).expect("applies");
    assert_eq!(fades_of(&project), (0, 0, 2));
    edit::apply(&mut project, applied.inverse).expect("undoes");
    assert_eq!(fades_of(&project), (1000, 0, 2));
    assert_eq!(project.to_json().expect("saves"), before);
}

// Every edit phases 1 and 2 use on an audio clip, applied to clips whose fades are longer than
// themselves (p38_fade_in at 2 ticks; p38_split, fades 3000 and 4000, at 5 ticks: 1250
// frames), then undone, gives the project's bytes back; and the undo undone (redo) gives the
// edit's bytes back.
#[test]
fn every_clip_edit_undoes_to_the_bytes_with_fades_longer_than_the_clip() {
    let cases = [
        resized(
            Project::load_json(&text("p38_fade_in.json")).expect("loads"),
            2,
        ),
        resized(
            Project::load_json(&text("p38_split.json")).expect("loads"),
            5,
        ),
    ];
    for project in cases {
        let (fade_in, fade_out, ticks) = fades_of(&project);
        let clip = &project.playlist.clips[&ClipId(1)];
        let ClipItem::Audio { frame_offset, .. } = clip.item else {
            panic!("an audio clip");
        };
        let edits = [
            Edit::SetClipFades {
                clip: ClipId(1),
                fade_in: 0,
                fade_out: 0,
            },
            Edit::SetClipFades {
                clip: ClipId(1),
                fade_in: 100,
                fade_out: 100,
            },
            Edit::SetClipFades {
                clip: ClipId(1),
                fade_in: fade_in * 2,
                fade_out: fade_out * 2,
            },
            Edit::TrimAudioClip {
                clip: ClipId(1),
                start: clip.start,
                length: clip.length,
                frame_offset: frame_offset + 77,
            },
            Edit::ResizeClip {
                id: ClipId(1),
                length: Ticks(ticks + 40),
            },
            Edit::SetClipGain {
                clip: ClipId(1),
                gain: 0.5,
            },
            Edit::DeleteClips {
                ids: vec![ClipId(1)],
            },
        ];
        for change in edits {
            let mut edited = project.clone();
            let before = edited.to_json().expect("saves");
            let applied = edit::apply(&mut edited, change.clone()).expect("applies");
            let after = edited.to_json().expect("saves");
            let undone = edit::apply(&mut edited, applied.inverse).expect("undoes");
            assert_eq!(
                edited.to_json().expect("saves"),
                before,
                "{change:?} undone"
            );
            edit::apply(&mut edited, undone.inverse).expect("redoes");
            assert_eq!(edited.to_json().expect("saves"), after, "{change:?} redone");
        }
    }
}
