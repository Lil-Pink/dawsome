//! An audio clip's warp markers as the engine holds them: gen_p39_expected.py's map table, the
//! version a warped project writes and the files it refuses, every marker edit's document and
//! undo bytes, the edits it refuses, and the markers following their clips through every edit
//! that makes or removes clips; and the compiler playing a warped clip's sound for now while its
//! own is made.

use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::{ClipHint, ClipSound, Compiler, Sounds};
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, Edit, Touched};
use dawsome_engine::plan::PlayMode;
use dawsome_engine::project::{
    ClipId, ClipItem, ClipStretch, LoadError, Project, SoundEdits, StretchMode, TrackId,
};
use dawsome_engine::realtime;
use dawsome_engine::time::Ticks;
use dawsome_engine::transport::Control;
use dawsome_engine::warp::{
    out_of_source, source_of_out, ClipWarp, WarpMarker, MAX_WARP_MARKERS, MIN_WARP_SEGMENT,
};
use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn text(name: &str) -> String {
    fs::read_to_string(fixtures().join(name)).unwrap_or_else(|_| panic!("{name}"))
}

fn expected() -> Value {
    serde_json::from_str(&text("p39_expected.json")).expect("p39_expected.json parses")
}

fn number(value: &Value) -> u64 {
    value.as_u64().expect("a number")
}

fn markers(value: &Value) -> Vec<WarpMarker> {
    value
        .as_array()
        .expect("a list of pairs")
        .iter()
        .map(|pair| WarpMarker {
            source: number(&pair[0]),
            out: number(&pair[1]),
        })
        .collect()
}

fn clip_id(value: &Value) -> ClipId {
    ClipId(u32::try_from(number(value)).expect("a clip id"))
}

/// The edit a generator case names, made as the gestures will make it.
fn edit_of(project: &Project, case: &Value) -> Edit {
    let clip = || clip_id(&case["clip"]);
    match case["edit"].as_str().expect("an edit") {
        "set_warp" => {
            let markers = markers(&case["markers"]);
            Edit::SetClipWarp {
                clip: clip(),
                warp: (!markers.is_empty()).then(|| ClipWarp {
                    source_frames: number(&case["sound_frames"]),
                    markers,
                }),
            }
        }
        "start_warp" => edit::start_warp(
            project,
            clip(),
            number(&case["sound_frames"]),
            markers(&case["markers"]),
        )
        .expect("a warp can start on this clip"),
        "set_stretch" => {
            let held = &project.playlist.clips[&clip()];
            let ClipItem::Audio { frame_offset, .. } = held.item else {
                panic!("an audio clip");
            };
            let mode = match case["mode"].as_str() {
                Some("resample") => Some(StretchMode::Resample),
                Some("stretch") => Some(StretchMode::Stretch),
                _ => None,
            };
            Edit::SetClipStretch {
                clip: clip(),
                stretch: mode.map(|mode| ClipStretch {
                    mode,
                    frames: number(&case["frames"]),
                    pitch: 0,
                }),
                frame_offset,
                length: held.length,
            }
        }
        "delete" => Edit::DeleteClips { ids: vec![clip()] },
        "duplicate" => Edit::DuplicateClips {
            ids: vec![clip()],
            first: clip_id(&case["first"]),
        },
        "sound_ops" => {
            let ops = serde_json::json!({ "ops": case["ops"] });
            Edit::SetChannelEdits {
                channel: dawsome_engine::project::ChannelId(
                    u32::try_from(number(&case["channel"])).expect("a channel id"),
                ),
                edits: serde_json::from_value::<SoundEdits>(ops).expect("an edit list"),
            }
        }
        other => panic!("no edit {other}"),
    }
}

// (1) The map both ways, by the generator's table: exact at each anchor, half up between, the
// ends held past them.
#[test]
fn p39_the_map_gives_the_generators_table() {
    for case in expected()["map"].as_array().expect("map") {
        let list = markers(&case["markers"]);
        let (sound, frames) = (number(&case["source_frames"]), number(&case["frames"]));
        for pair in case["out_of_source"].as_array().expect("pairs") {
            let (source, out) = (number(&pair[0]), number(&pair[1]));
            assert_eq!(
                out_of_source(&list, sound, frames, source),
                out,
                "{case} at {source}"
            );
        }
        for pair in case["source_of_out"].as_array().expect("pairs") {
            let (out, source) = (number(&pair[0]), number(&pair[1]));
            assert_eq!(
                source_of_out(&list, sound, frames, out),
                source,
                "{case} at {out}"
            );
        }
    }
}

// The limits the generator states are the engine's.
#[test]
fn p39_the_limits_are_the_generators() {
    let rules = &expected()["rules"];
    assert_eq!(MIN_WARP_SEGMENT, number(&rules["min_segment"]));
    assert_eq!(MAX_WARP_MARKERS as u64, number(&rules["max_markers"]));
    assert_eq!(
        dawsome_engine::project::MAX_STRETCH_RATIO,
        number(&rules["max_ratio"])
    );
}

// (5) A warped project writes version 13 and reads back to its bytes; one without markers keeps
// its version. A build that reads up to 12 refuses 13 by its number (its text pinned in the
// generator); this build refuses 14 so, naming 13. Every broken table is refused on load.
#[test]
fn p39_a_warped_project_is_version_13_and_a_broken_one_is_refused() {
    let want = expected();
    let documents = &want["documents"];
    for kind in ["warped", "unwarped"] {
        let case = &documents[kind];
        let file = text(case["fixture"].as_str().expect("fixture"));
        let project = Project::load_json(&file).expect("loads");
        let written = project.to_json().expect("saves");
        assert_eq!(written.clone() + "\n", file, "{kind}: the same bytes");
        let version = format!("{{\"format_version\":{},", case["format_version"]);
        assert!(written.starts_with(&version), "{kind}: {written}");
    }
    let by_12 = documents["refused_by_12"].as_str().expect("refused_by_12");
    assert!(by_12.starts_with("format_version 13 is not supported"));
    assert!(by_12.ends_with("11 and 12)"), "{by_12}");
    let warped = text("p39_warp3.json");
    let newer = warped.replacen(r#""format_version":13"#, r#""format_version":14"#, 1);
    let error = Project::load_json(&newer).expect_err("refused");
    assert!(
        matches!(error, LoadError::UnsupportedVersion(14)),
        "{error}"
    );
    assert_eq!(
        error.to_string(),
        documents["refused_14"].as_str().expect("refused_14")
    );
    for (name, broken) in documents["broken"].as_object().expect("broken") {
        let file = broken.as_str().expect("a document");
        assert!(Project::load_json(file).is_err(), "{name} loads");
    }
}

// (6) Each marker edit the generator lists: the document after is its, the undo gives the
// fixture's bytes back, and the undo undone gives the edit's bytes again.
#[test]
fn p39_each_marker_edit_gives_its_document_and_undoes_to_the_bytes() {
    for case in expected()["edits"].as_array().expect("edits") {
        let name = case["name"].as_str().expect("name");
        let before = text(case["fixture"].as_str().expect("fixture"));
        let mut project = Project::load_json(&before).expect("loads");
        let change = edit_of(&project, case);
        let applied = edit::apply(&mut project, change).unwrap_or_else(|error| {
            panic!("{name}: {error}");
        });
        let after = project.to_json().expect("saves");
        assert_eq!(after, case["after"].as_str().expect("after"), "{name}");
        let undone = edit::apply(&mut project, applied.inverse).expect("undoes");
        let back = project.to_json().expect("saves");
        assert_eq!(back, case["undo"].as_str().expect("undo"), "{name}");
        assert_eq!(back + "\n", before, "{name}");
        edit::apply(&mut project, undone.inverse).expect("redoes");
        assert_eq!(project.to_json().expect("saves"), after, "{name} redone");
    }
}

// (6) The refused edits: each is an error and the project keeps its bytes.
#[test]
fn p39_a_marker_edit_that_breaks_a_rule_changes_nothing() {
    for case in expected()["refused"].as_array().expect("refused") {
        let name = case["name"].as_str().expect("name");
        let before = text(case["fixture"].as_str().expect("fixture"));
        let mut project = Project::load_json(&before).expect("loads");
        let change = edit_of(&project, case);
        assert!(edit::apply(&mut project, change).is_err(), "{name} applied");
        assert_eq!(project.to_json().expect("saves") + "\n", before, "{name}");
    }
}

/// Every warps entry names a clip that exists, is audio and stretched, and holds markers.
fn assert_warps_follow_clips(project: &Project, after: &str) {
    for (id, warp) in &project.playlist.warps {
        let clip = project.playlist.clips.get(id);
        let stretched = clip.is_some_and(|clip| {
            matches!(
                clip.item,
                ClipItem::Audio {
                    stretch: Some(_),
                    ..
                }
            )
        });
        assert!(
            stretched && !warp.markers.is_empty(),
            "{after}: clip {id:?}"
        );
    }
    project.validate().expect("a valid project");
}

// Every edit that makes or removes clips, on p39_warp3_split (two warped clips) with one more
// track and the song's own audio clip added: the markers go and come back with their clips
// (copies carry them), the project stays valid after each step, and each step undoes and redoes
// to the bytes.
#[test]
fn every_edit_that_makes_or_removes_clips_carries_the_markers() {
    let base = Project::load_json(&text("p39_warp3_split.json")).expect("loads");
    let three = base.playlist.warps[&ClipId(1)].clone();
    let mut added = base.clone();
    let add = Edit::AddAudioClip {
        channel: added.next_channel_id,
        name: "More".into(),
        sample: "p39_sound/tone.wav".into(),
        clip: added.playlist.next_clip_id,
        track: TrackId(3),
        start: Ticks(0),
        length: Ticks(96),
    };
    edit::apply(&mut added, add).expect("adds");
    let newest = ClipId(3);
    let stretch = Edit::SetClipStretch {
        clip: newest,
        stretch: Some(ClipStretch {
            mode: StretchMode::Resample,
            frames: 24000,
            pitch: 0,
        }),
        frame_offset: 0,
        length: Ticks(96),
    };
    let warp = Edit::SetClipWarp {
        clip: newest,
        warp: Some(ClipWarp {
            source_frames: 24000,
            markers: vec![WarpMarker {
                source: 12000,
                out: 12000,
            }],
        }),
    };
    let steps: Vec<(&str, Project, Edit)> = vec![
        (
            "delete one",
            base.clone(),
            Edit::DeleteClips {
                ids: vec![ClipId(2)],
            },
        ),
        (
            "delete both",
            base.clone(),
            Edit::DeleteClips {
                ids: vec![ClipId(1), ClipId(2)],
            },
        ),
        (
            "duplicate",
            base.clone(),
            Edit::DuplicateClips {
                ids: vec![ClipId(1), ClipId(2)],
                first: ClipId(3),
            },
        ),
        (
            "unplace",
            base.clone(),
            Edit::UnplaceClips {
                first: ClipId(2),
                count: 1,
            },
        ),
        (
            "place",
            base.clone(),
            Edit::PlaceClips {
                first: ClipId(3),
                clips: vec![base.playlist.clips[&ClipId(1)].clone()],
            },
        ),
        (
            "delete track",
            base.clone(),
            Edit::DeleteTrack { id: TrackId(1) },
        ),
        (
            "unstretch",
            base.clone(),
            Edit::SetClipStretch {
                clip: ClipId(1),
                stretch: None,
                frame_offset: 0,
                length: Ticks(36),
            },
        ),
        (
            "transaction",
            base.clone(),
            Edit::Transaction(vec![
                Edit::DeleteClips {
                    ids: vec![ClipId(1)],
                },
                Edit::SetClipWarp {
                    clip: ClipId(2),
                    warp: Some(ClipWarp {
                        markers: three.markers[..2].to_vec(),
                        ..three.clone()
                    }),
                },
            ]),
        ),
        (
            "stretch and warp the newest",
            added.clone(),
            Edit::Transaction(vec![stretch.clone(), warp.clone()]),
        ),
    ];
    for (name, project, change) in steps {
        let mut edited = project.clone();
        let before = edited.to_json().expect("saves");
        let applied = edit::apply(&mut edited, change).unwrap_or_else(|error| {
            panic!("{name}: {error}");
        });
        assert_warps_follow_clips(&edited, name);
        let after = edited.to_json().expect("saves");
        let undone = edit::apply(&mut edited, applied.inverse).expect("undoes");
        assert_warps_follow_clips(&edited, name);
        assert_eq!(edited.to_json().expect("saves"), before, "{name} undone");
        edit::apply(&mut edited, undone.inverse).expect("redoes");
        assert_eq!(edited.to_json().expect("saves"), after, "{name} redone");
    }
    // A copy holds the list of the clip it copies; a placed clip none until a marker edit.
    let mut copied = base.clone();
    let duplicate = Edit::DuplicateClips {
        ids: vec![ClipId(2)],
        first: ClipId(3),
    };
    edit::apply(&mut copied, duplicate).expect("duplicates");
    assert_eq!(copied.playlist.warps.get(&ClipId(3)), Some(&three));
    // The newest audio clip, warped, then taken out with its channel: the add's inverse takes
    // its markers too, and putting it back (a redo) brings the clip without them, as added.
    let mut warped = added.clone();
    edit::apply(&mut warped, Edit::Transaction(vec![stretch, warp])).expect("warps");
    let unadd = Edit::Transaction(vec![
        Edit::SetClipWarp {
            clip: newest,
            warp: None,
        },
        Edit::SetClipStretch {
            clip: newest,
            stretch: None,
            frame_offset: 0,
            length: Ticks(96),
        },
        Edit::UnaddAudioClip {
            channel: dawsome_engine::project::ChannelId(2),
            clip: newest,
        },
    ]);
    edit::apply(&mut warped, unadd).expect("takes the newest out");
    assert_eq!(
        warped.to_json().expect("saves"),
        base.to_json().expect("saves")
    );
}

/// Sounds whose warped stretch is being made until `ready`: meanwhile a warped clip gets, for
/// now, the stretch it had (a quarter everywhere), as media gives while its worker stretches;
/// then its own (a half everywhere).
struct Stretching {
    ready: bool,
    asked: usize,
}

impl Sounds for Stretching {
    type Error = Infallible;

    fn sample(&mut self, _path: &Path) -> Result<Option<SampleBuffer>, Infallible> {
        Ok(None)
    }

    fn clip(&mut self, _path: &Path, hint: ClipHint<'_>) -> Result<ClipSound, Infallible> {
        self.asked += 1;
        let frames = hint.stretch.map_or(0, |stretch| stretch.frames) as usize;
        let level = |value| ClipSound::Memory(SampleBuffer::mono(vec![value; frames]));
        Ok(if self.ready || hint.warp.is_empty() {
            level(0.5)
        } else {
            ClipSound::ForNow(Box::new(level(0.25)))
        })
    }
}

// A warped clip whose stretch is still being made plays the one given for now, never kept: each
// compile asks again, the clip keeping that source meanwhile; once its own is in, it plays and
// is kept (no more asks).
#[test]
fn a_warped_clips_sound_for_now_plays_until_its_own_is_in() {
    let project = Project::load_json(&text("p39_warp1.json")).expect("loads");
    let mut sounds = Stretching {
        ready: false,
        asked: 0,
    };
    let mut compiler = Compiler::new();
    let compile = |compiler: &mut Compiler, sounds: &mut Stretching| {
        compiler
            .compile_with(&project, &Touched::default(), PlayMode::Song, sounds)
            .expect("compiles")
    };
    let first = compile(&mut compiler, &mut sounds);
    let (mut link, mut player) = realtime::link(first, project.tempo);
    link.send(Control::Play).expect("room");
    // The clip's first frames: it starts at tick 240.
    let heard = |link: &mut realtime::RealtimeLink, player: &mut realtime::Player| {
        link.send(Control::Seek(Ticks(240))).expect("room");
        let mut out = vec![[0.0f32; 2]; 512];
        player.process(&mut out);
        link.drain_retired();
        out[..64].iter().map(|frame| frame[0]).collect::<Vec<f32>>()
    };
    assert_eq!(heard(&mut link, &mut player), [0.25; 64], "for now");
    assert_eq!(sounds.asked, 1);
    let again = compile(&mut compiler, &mut sounds);
    assert_eq!(sounds.asked, 2, "asked again");
    link.push_plan(again).expect("same time base");
    assert_eq!(heard(&mut link, &mut player), [0.25; 64], "still");
    sounds.ready = true;
    let ready = compile(&mut compiler, &mut sounds);
    link.push_plan(ready).expect("same time base");
    assert_eq!(heard(&mut link, &mut player), [0.5; 64], "its own");
    assert_eq!(sounds.asked, 3);
    let kept = compile(&mut compiler, &mut sounds);
    assert_eq!(sounds.asked, 3, "kept, not asked");
    link.push_plan(kept).expect("same time base");
    assert_eq!(heard(&mut link, &mut player), [0.5; 64], "kept");
}
