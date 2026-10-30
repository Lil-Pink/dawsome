//! A sound's edits in the engine (the sample editor, P1a): each list of
//! tests/fixtures/gen_p30_expected.py gives the frames it states (p30_expected.json), the
//! zero-crossing search snaps where it says, every p30 project reads and writes back byte for
//! byte at its version (8 only with edits), an op appended through `sound_edits::append` moves
//! the channel's clips to the offsets stated and is one undo step back to the exact document and
//! render, another sound on an edited sampler carries its edits away and back, and the player
//! plays an edited sampler and an edited clip without touching the heap. Expected values come
//! from the generator, never from the engine.

use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::{ClipHint, ClipSound, Compiler, EditedSound, Sounds};
use dawsome_engine::dsp::edits::{apply, zero_crossing};
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, sound_edits, Edit, EditError, History, Touched};
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::project::{
    ChannelId, ClipItem, LoadError, PatternId, Project, SoundEdits, SoundOp, SoundSource,
    ValidationError, EDITS_FORMAT_VERSION,
};
use dawsome_engine::realtime;
use dawsome_engine::transport::Control;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p30_expected.json"))
        .expect("p30_expected.json (run gen_p30_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("parses")
}

fn wav(path: &Path) -> SampleBuffer {
    let mut reader = hound::WavReader::open(path).expect("a test sound");
    let channels = reader.spec().channels;
    let values: Vec<f32> = reader
        .samples::<f32>()
        .map(|value| value.expect("reads"))
        .collect();
    if channels == 2 {
        SampleBuffer::stereo(values)
    } else {
        SampleBuffer::mono(values)
    }
}

fn text(name: &str) -> String {
    fs::read_to_string(fixtures().join(name)).expect("a p30 fixture")
}

fn load(name: &str) -> Project {
    Project::load_json(&text(name)).expect("loads")
}

fn saved(project: &Project) -> String {
    project.to_json().expect("writes")
}

fn edits_of(value: &Value) -> SoundEdits {
    serde_json::from_value(value.clone()).expect("an edit list")
}

fn op_of(value: &Value) -> SoundOp {
    serde_json::from_value(value.clone()).expect("an op")
}

fn sha_of(values: &[f32]) -> String {
    let mut hash = Sha256::new();
    for value in values {
        hash.update(value.to_le_bytes());
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn floats(value: &Value) -> Vec<f32> {
    value
        .as_array()
        .expect("a list")
        .iter()
        .map(|value| value.as_f64().expect("a number") as f32)
        .collect()
}

// (1)-(6) Every list on its sound: the frames, their bits (by the generator's SHA-256 of the
// values as little-endian f32s) and, for the short ones, every value: fades, crops, deletes,
// normalize (to -1.0 exactly, a hot 32f file down to 1.0, -1 dB), gain, silence, reverse (twice
// is the original), ranges past the end, and a chain whose positions are each in the sound the
// ops before left.
#[test]
fn p30_each_list_gives_the_frames_its_rules_state() {
    let want = expected();
    for (name, case) in want["ops"].as_object().expect("ops") {
        let sound = wav(&fixtures().join(case["sound"].as_str().expect("a sound")));
        assert_eq!(
            u64::from(sound.channels()),
            case["channels"].as_u64().expect("channels"),
            "{name}"
        );
        let out = apply(&sound, &edits_of(&case["edits"]));
        assert_eq!(
            out.frames(),
            case["frames"].as_u64().expect("frames"),
            "{name}"
        );
        if case.get("out").is_some() {
            let wanted: Vec<u32> = floats(&case["out"]).iter().map(|v| v.to_bits()).collect();
            let got: Vec<u32> = out.samples().iter().map(|v| v.to_bits()).collect();
            assert_eq!(got, wanted, "{name}");
        }
        assert_eq!(
            sha_of(out.samples()),
            case["sha256"].as_str().expect("sha256"),
            "{name}"
        );
    }
    // Normalize leaves silence as it is, its negative zeros too.
    let silent = &want["silent"];
    let values = floats(&silent["values"]);
    let out = apply(
        &SampleBuffer::mono(values.clone()),
        &edits_of(&silent["edits"]),
    );
    let bits = |values: &[f32]| values.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(out.samples()), bits(&values));
    assert!(
        values.iter().any(|v| v.is_sign_negative()),
        "the case has a -0.0"
    );
    // A NaN in a normalize's range is no peak, first in it or all of it: the rest lands on the
    // others' peak and the NaN stays (null stands for a NaN, which JSON cannot hold).
    let nan_or = |value: &Value| value.as_f64().map_or(f32::NAN, |value| value as f32);
    for (name, case) in want["nan"].as_object().expect("the NaN cases") {
        let list = |key: &str| -> Vec<f32> {
            case[key]
                .as_array()
                .expect("a list")
                .iter()
                .map(nan_or)
                .collect()
        };
        let out = apply(
            &SampleBuffer::mono(list("values")),
            &edits_of(&case["edits"]),
        );
        let shown = |values: &[f32]| {
            values
                .iter()
                .map(|v| if v.is_nan() { None } else { Some(v.to_bits()) })
                .collect::<Vec<_>>()
        };
        assert_eq!(shown(out.samples()), shown(&list("out")), "{name}");
    }
}

// (11) The zero-crossing search: the triangle 3, 2, 1, 0, -1, ... (quartered) snaps 7 to 9 and 5
// to 3; nothing in reach leaves the edge; a 2 kHz wave above 0 has no crossing; stereo sides
// are summed, and of two crossings as near the earlier wins.
#[test]
fn p30_an_edge_snaps_to_the_crossing_the_generator_finds() {
    for case in expected()["zero_crossings"].as_array().expect("cases") {
        let values = floats(&case["values"]);
        let sound = if case["channels"] == 2 {
            SampleBuffer::stereo(values)
        } else {
            SampleBuffer::mono(values)
        };
        let press = case["press"].as_u64().expect("press");
        let reach = case["reach"].as_u64().expect("reach");
        assert_eq!(
            zero_crossing(&sound, press, reach),
            case["snap"].as_u64().expect("snap"),
            "{} pressed at {press}, reach {reach}",
            case["sound"]
        );
    }
}

/// Every p30 project, by name.
fn p30_projects() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(fixtures())
        .expect("the fixtures")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.starts_with("p30_") && name.ends_with(".json"))
        .filter(|name| name != "p30_expected.json")
        .collect();
    names.sort();
    names
}

// (10) Every p30 project reads and writes back byte for byte: version 8 exactly when a channel
// has edits, its old version without (the sound settings' 5, else 2), and the edits after the
// sound settings.
#[test]
fn p30_projects_read_and_write_back_at_their_version() {
    let names = p30_projects();
    assert_eq!(names.len(), 19, "{names:?}");
    for name in &names {
        let text = text(name);
        let project = Project::load_json(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(saved(&project) + "\n", text, "{name}");
        let edited = project.channels.values().any(|c| !c.edits.is_empty());
        let version = if edited {
            EDITS_FORMAT_VERSION
        } else if text.contains(r#""sound":{"#) {
            5
        } else {
            2
        };
        assert!(
            text.starts_with(&format!(r#"{{"format_version":{version},"#)),
            "{name}"
        );
        // A project whose edits are all taken back writes the version it had before them.
        if edited {
            let mut plain = project.clone();
            for channel in plain.channels.values_mut() {
                channel.edits = SoundEdits::default();
            }
            assert!(!saved(&plain).contains(r#""edits""#), "{name}");
            assert!(
                !saved(&plain).starts_with(r#"{"format_version":8,"#),
                "{name}"
            );
        }
    }
}

// (10) What reading refuses: an op of a later build, by its name; edits on a channel that plays
// no file; an empty range; a gain out of range; more than 1000 ops. What it reads past: a field
// beside the ops (a later build's markers), which plays nothing.
#[test]
fn p30_reading_refuses_what_no_sound_can_hold() {
    let crop = text("p30_crop.json");
    let warble = crop.replacen(r#"{"crop":"#, r#"{"warble":"#, 1);
    let error = Project::load_json(&warble).expect_err("an unknown op");
    assert!(error.to_string().contains("warble"), "{error}");
    let marked = crop.replacen(
        r#""to":36000}}]}"#,
        r#""to":36000}}],"markers":[100,200]}"#,
        1,
    );
    assert_ne!(marked, crop);
    assert_eq!(
        Project::load_json(&marked).expect("markers are read past"),
        load("p30_crop.json")
    );
    let refused = |text: &str| match Project::load_json(text) {
        Err(LoadError::Invalid(error)) => error,
        other => panic!("not refused as invalid: {other:?}"),
    };
    let empty = crop.replacen(
        r#""from":12000,"to":36000"#,
        r#""from":12000,"to":12000"#,
        1,
    );
    assert!(matches!(
        refused(&empty),
        ValidationError::EmptyEditRange { index: 0, .. }
    ));
    let gain = text("p30_gain.json").replacen(r#""centi_db":-600"#, r#""centi_db":2401"#, 1);
    assert!(matches!(
        refused(&gain),
        ValidationError::EditGainOutOfRange { centi_db: 2401, .. }
    ));
    let op = r#"{"reverse":{"from":0,"to":8}}"#;
    let many = crop.replacen(
        r#"{"crop":{"from":12000,"to":36000}}"#,
        &vec![op; 1001].join(","),
        1,
    );
    assert!(matches!(
        refused(&many),
        ValidationError::TooManySoundEdits { count: 1001, .. }
    ));
    let set = text("p30_change.json").replacen(
        r#""kind":"sampler","sample":"p30_sound/ramp64.wav""#,
        r#""kind":"instrument","instrument":"instruments/pad/pad.sfz""#,
        1,
    );
    assert!(matches!(
        refused(&set),
        ValidationError::EditsOnOtherKind { .. }
    ));
}

// (7) A crop in front of a clip at frame 48000 moves it to 47000, a delete of 10000..20000 to
// 38000; a stretched clip keeps its ratio (108000 over 72000 frames: 106500 over 71000 after the
// crop) and its place in the sound. Each is one edit whose inverse gives the document back.
#[test]
fn p30_a_crop_or_a_delete_moves_the_clips_as_stated() {
    for (name, case) in expected()["remap"].as_object().expect("remap") {
        let fixture = case["fixture"].as_str().expect("fixture");
        let before = text(fixture);
        let mut project = load(fixture);
        let channel = ChannelId(case["channel"].as_u64().expect("channel") as u32);
        let frames = case["frames"].as_u64().expect("frames");
        let step =
            sound_edits::append(&project, channel, op_of(&case["op"]), frames).expect("an op");
        let applied = edit::apply(&mut project, step).expect("applies");
        assert_eq!(
            saved(&project),
            case["after"].as_str().expect("after"),
            "{name}"
        );
        let ClipItem::Audio {
            frame_offset,
            stretch,
            ..
        } = project.playlist.clips.values().next().expect("a clip").item
        else {
            panic!("{name}: an audio clip");
        };
        assert_eq!(
            frame_offset,
            case["frame_offset"].as_u64().expect("offset"),
            "{name}"
        );
        assert_eq!(
            stretch.map(|stretch| stretch.frames),
            case["stretched"].as_u64(),
            "{name}"
        );
        edit::apply(&mut project, applied.inverse).expect("undoes");
        assert_eq!(saved(&project) + "\n", before, "{name}: undone");
    }
}

/// `frames` of `project` playing `mode`, its sounds decoded from the fixtures (the edits
/// applied by the compiler's own default), one `process` call per size in `sizes`.
fn render(project: &Project, mode: PlayMode, frames: usize, sizes: &[usize]) -> Vec<[f32; 2]> {
    let (mut link, mut player) = realtime::link(compiled(project, mode), project.tempo);
    link.send(Control::Play).expect("room");
    let mut out = vec![[f32::NAN; 2]; frames];
    let mut done = 0;
    for &size in sizes.iter().cycle() {
        if done == frames {
            break;
        }
        let end = (done + size).min(frames);
        player.process(&mut out[done..end]);
        done = end;
    }
    out
}

fn compiled(project: &Project, mode: PlayMode) -> PlanUpdate {
    let load = |path: &Path| Ok::<_, Infallible>(wav(&fixtures().join(path)));
    Compiler::new()
        .compile(project, &Touched::default(), mode, load)
        .expect("compiles")
}

/// The project's pattern and its song, a bar each, as bits.
fn renders(project: &Project) -> Vec<Vec<[u32; 2]>> {
    let bits = |frames: Vec<[f32; 2]>| frames.iter().map(|f| f.map(f32::to_bits)).collect();
    [PlayMode::Pattern(PatternId(1)), PlayMode::Song]
        .into_iter()
        .map(|mode| bits(render(project, mode, 88_200, &[512])))
        .collect()
}

// (8) Every op as the editor commits it (`sound_edits::append`, recorded as one step named by
// the op): the document after each is the generator's; undoing them all gives back each document
// before and at last the project's own bytes and renders; redoing them all the documents and the
// renders after.
#[test]
fn p30_every_op_is_one_undo_step_back_to_the_exact_project() {
    let want = expected();
    let undo = &want["undo"];
    let fixture = undo["fixture"].as_str().expect("fixture");
    let before = text(fixture);
    let mut project = load(fixture);
    let first_renders = renders(&project);
    let mut history = History::new();
    let mut documents = vec![saved(&project)];
    for step in undo["steps"].as_array().expect("steps") {
        let channel = ChannelId(step["channel"].as_u64().expect("channel") as u32);
        let op = op_of(&step["op"]);
        assert_eq!(op.name(), step["name"].as_str().expect("a name"));
        let frames = step["frames"].as_u64().expect("frames");
        let edit = sound_edits::append(&project, channel, op, frames).expect("an op");
        let applied = edit::apply(&mut project, edit).expect("applies");
        history.record_as(applied.inverse, op.name());
        assert_eq!(history.undo_name(), Some(op.name()));
        let after = saved(&project);
        assert_eq!(after, step["after"].as_str().expect("after"), "{op:?}");
        documents.push(after);
    }
    let last_renders = renders(&project);
    assert_ne!(last_renders, first_renders, "the edits change what plays");
    for document in documents.iter().rev().skip(1) {
        history.undo(&mut project).expect("undoes");
        assert_eq!(&saved(&project), document);
    }
    assert_eq!(saved(&project) + "\n", before);
    assert_eq!(
        renders(&project),
        first_renders,
        "undone, it plays as before"
    );
    for document in documents.iter().skip(1) {
        history.redo(&mut project).expect("redoes");
        assert_eq!(&saved(&project), document);
    }
    assert_eq!(renders(&project), last_renders, "redone, it plays as after");
}

// (8) Another sound on an edited sampler: another file lets its edits go in the same step, a set
// in its place too (`SoundSource::instrument` holds none), a clone copies them; each undone is
// the document byte for byte.
#[test]
fn p30_another_sound_carries_the_edits_away_and_back() {
    let want = expected();
    let change = &want["change"];
    let fixture = change["fixture"].as_str().expect("fixture");
    let before = text(fixture);
    let cases = [
        (
            "sample",
            Edit::SetChannelSample {
                id: ChannelId(1),
                path: change["sample"]["path"].as_str().expect("path").into(),
            },
        ),
        (
            "instrument",
            Edit::ReplaceChannelSound {
                channel: ChannelId(1),
                name: change["instrument"]["name"].as_str().expect("name").into(),
                source: Box::new(SoundSource::instrument(
                    change["instrument"]["path"].as_str().expect("path"),
                )),
                sound: Default::default(),
            },
        ),
        (
            "clone",
            Edit::CloneChannel {
                id: ChannelId(2),
                from: ChannelId(1),
            },
        ),
    ];
    for (name, edit) in cases {
        let mut project = load(fixture);
        let applied = edit::apply(&mut project, edit).expect("applies");
        assert_eq!(
            saved(&project),
            change[name]["after"].as_str().expect("after"),
            "{name}"
        );
        edit::apply(&mut project, applied.inverse).expect("undoes");
        assert_eq!(saved(&project) + "\n", before, "{name}: undone");
    }
    // The same file again keeps the edits: nothing about the sound changed.
    let mut project = load(fixture);
    let same = Edit::SetChannelSample {
        id: ChannelId(1),
        path: "p30_sound/ramp64.wav".into(),
    };
    edit::apply(&mut project, same).expect("applies");
    assert_eq!(saved(&project) + "\n", before);
}

// What the edit and `sound_edits::append` refuse: edits on a channel that plays no file, a list no sound
// holds, a delete leaving no frame; and the list taken back to the file (`revert`) moves the
// clips back to where they played.
#[test]
fn p30_the_edit_refuses_and_revert_moves_the_clips_back() {
    let mut project = load("p30_change.json");
    let set = Edit::ReplaceChannelSound {
        channel: ChannelId(1),
        name: "Pad".into(),
        source: Box::new(SoundSource::instrument("instruments/pad/pad.sfz")),
        sound: Default::default(),
    };
    edit::apply(&mut project, set).expect("a set");
    let reverse = SoundOp::Reverse { from: 0, to: 8 };
    assert_eq!(
        sound_edits::append(&project, ChannelId(1), reverse, 64),
        Err(EditError::NoSoundFile(1))
    );
    let on_set = Edit::SetChannelEdits {
        channel: ChannelId(1),
        edits: SoundEdits {
            ops: vec![reverse],
            source: None,
        },
    };
    assert_eq!(
        edit::apply(&mut project, on_set),
        Err(EditError::NoSoundFile(1))
    );
    let mut project = load("p30_change.json");
    let shown = saved(&project);
    let empty = Edit::SetChannelEdits {
        channel: ChannelId(1),
        edits: SoundEdits {
            ops: vec![SoundOp::Crop { from: 5, to: 5 }],
            source: None,
        },
    };
    assert!(matches!(
        edit::apply(&mut project, empty),
        Err(EditError::Invalid(ValidationError::EmptyEditRange { .. }))
    ));
    assert_eq!(saved(&project), shown, "a refused edit changes nothing");
    let all = SoundOp::Delete { from: 0, to: 64 };
    assert_eq!(
        sound_edits::append(&project, ChannelId(1), all, 64),
        Err(EditError::NoFramesLeft(1))
    );

    // A crop and a delete, then revert: the clip at 48000 plays from 48000 again.
    let mut project = load("p30_remap.json");
    let before = saved(&project);
    for (op, frames) in [
        (
            SoundOp::Crop {
                from: 1000,
                to: 72000,
            },
            72_000,
        ),
        (
            SoundOp::Delete {
                from: 10000,
                to: 20000,
            },
            71_000,
        ),
    ] {
        let step = sound_edits::append(&project, ChannelId(1), op, frames).expect("an op");
        edit::apply(&mut project, step).expect("applies");
    }
    let revert = sound_edits::revert(&project, ChannelId(1), 72_000).expect("revert");
    edit::apply(&mut project, revert).expect("applies");
    assert_eq!(saved(&project), before);
}

// (13) Through a plan swap that brings an edited sampler and an edited clip in, the player plays
// on without the heap.
#[test]
fn p30_an_edited_sound_plays_without_the_heap() {
    let control = heap_calls_during(|| drop(std::hint::black_box(vec![0u8; 64])));
    assert!(control > 0, "the counting allocator is not counting");
    let plain = load("p30_undo.json");
    let mut edited = plain.clone();
    for (channel, op, frames) in [
        (1, SoundOp::Reverse { from: 0, to: 48000 }, 48_000),
        (
            2,
            SoundOp::Crop {
                from: 1000,
                to: 72000,
            },
            72_000,
        ),
        (2, SoundOp::FadeIn { from: 0, to: 30000 }, 71_000),
    ] {
        let step = sound_edits::append(&edited, ChannelId(channel), op, frames).expect("an op");
        edit::apply(&mut edited, step).expect("applies");
    }
    let load = |path: &Path| Ok::<_, Infallible>(wav(&fixtures().join(path)));
    let touched = Touched {
        channels: [ChannelId(1), ChannelId(2)].into(),
        playlist: true,
        ..Touched::default()
    };
    for mode in [PlayMode::Pattern(PatternId(1)), PlayMode::Song] {
        // One compiler, as the app keeps: its sources and slots carry from plan to plan.
        let mut compiler = Compiler::new();
        let first = compiler
            .compile(&plain, &Touched::default(), mode, load)
            .expect("compiles");
        let (mut link, mut player) = realtime::link(first, plain.tempo);
        link.send(Control::Play).expect("room");
        let mut out = vec![[0.0; 2]; 512];
        let (mut calls, mut swaps) = (0, 0);
        for call in 0..200 {
            // The UI half's work happens between calls, outside the measurement.
            if call % 50 == 25 {
                compiler.release(link.take_freed_slots());
                compiler.release_sources(link.take_freed_sources());
                let project = if call % 100 == 25 { &edited } else { &plain };
                let update = compiler
                    .compile(project, &touched, mode, load)
                    .expect("compiles");
                link.push_plan(update).expect("same time base");
            }
            calls += heap_calls_during(|| player.process(&mut out));
            swaps += link.drain_retired();
        }
        assert_eq!(calls, 0, "{mode:?}: heap calls in Player::process");
        assert_eq!(swaps, 4, "{mode:?}: every plan swapped in");
    }
}

// (12) An edited project renders the same bits whatever the blocks: 1, 7, 64, 4096 and seeded
// sizes against one block of 512.
#[test]
fn p30_an_edited_project_renders_the_same_bits_in_any_blocks() {
    let mut project = load("p30_undo.json");
    for (channel, op, frames) in [
        (1, SoundOp::FadeOut { from: 0, to: 48000 }, 48_000),
        (
            2,
            SoundOp::Delete {
                from: 10000,
                to: 20000,
            },
            72_000,
        ),
        (
            2,
            SoundOp::Reverse {
                from: 30000,
                to: 50000,
            },
            62_000,
        ),
    ] {
        let step = sound_edits::append(&project, ChannelId(channel), op, frames).expect("an op");
        edit::apply(&mut project, step).expect("applies");
    }
    let mut state = 0x5eed_u64;
    let seeded: Vec<usize> = (0..400)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            1 + (state % 900) as usize
        })
        .collect();
    for mode in [PlayMode::Pattern(PatternId(1)), PlayMode::Song] {
        let whole = render(&project, mode, 88_200, &[512]);
        assert!(whole.iter().any(|f| f[0] != 0.0), "{mode:?}: sounds");
        for sizes in [vec![1], vec![7], vec![64], vec![4096], seeded.clone()] {
            let split = render(&project, mode, 88_200, &sizes);
            let bits = |frames: &[[f32; 2]]| -> Vec<[u32; 2]> {
                frames.iter().map(|f| f.map(f32::to_bits)).collect()
            };
            assert!(
                bits(&split) == bits(&whole),
                "{mode:?}: blocks {:?} differ",
                &sizes[..sizes.len().min(3)]
            );
        }
    }
}

// (10) Every project the repository holds, of every version, that reads at all writes back its
// own bytes past the version, and a version no higher than its own (8 exactly when it has
// edits): the edits field writes nothing for a sound without edits, so no project made before
// them changes on disk.
#[test]
fn p30_every_project_without_edits_keeps_its_bytes_and_version() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut checked = 0;
    for dir in ["tests/fixtures", "examples"] {
        for entry in fs::read_dir(root.join(dir)).expect("a folder") {
            let path = entry.expect("an entry").path();
            let name = path
                .file_name()
                .expect("a name")
                .to_string_lossy()
                .into_owned();
            if !name.ends_with(".json") {
                continue;
            }
            let text = fs::read_to_string(&path).expect("reads");
            let Some(version) = text
                .strip_prefix(r#"{"format_version":"#)
                .and_then(|rest| rest.split(',').next())
                .and_then(|version| version.parse::<u32>().ok())
            else {
                continue;
            };
            // Version 1 is migrated (written anew as 2), and a few fixtures are refused by design.
            // p13_shot writes its sound settings at the defaults, which a save leaves out (so on
            // f7d1a73, before edits, too).
            if version == 1 || name == "p13_shot.json" {
                continue;
            }
            let Ok(project) = Project::load_json(&text) else {
                continue;
            };
            // Past the version, byte for byte; the version the oldest that holds the project,
            // never above the file's (gen_p25 writes four of its projects at 4, holding 2's).
            let saved = saved(&project);
            let (written, rest) = saved
                .strip_prefix(r#"{"format_version":"#)
                .and_then(|rest| rest.split_once(','))
                .expect("a version first");
            let file_rest = text
                .trim_end_matches('\n')
                .split_once(',')
                .expect("fields")
                .1;
            assert_eq!(rest, file_rest, "{name}");
            let written: u32 = written.parse().expect("a number");
            assert!(written <= version, "{name}: {written} above {version}");
            let edited = project.channels.values().any(|c| !c.edits.is_empty());
            assert_eq!(edited, written == EDITS_FORMAT_VERSION, "{name}");
            assert_eq!(edited, version == EDITS_FORMAT_VERSION, "{name}");
            checked += 1;
        }
    }
    eprintln!("{checked} projects keep their bytes");
    assert!(checked >= 200, "only {checked} projects read");
}

// The compiler keys a sampler's sound by its file and list: two channels of one file, one
// edited, play two sounds; the unedited one plays the file's own frames.
#[test]
fn p30_an_edited_and_a_plain_channel_of_one_file_play_their_own_sounds() {
    let mut project = load("p30_reverse60.json");
    let clone = Edit::CloneChannel {
        id: ChannelId(2),
        from: ChannelId(1),
    };
    edit::apply(&mut project, clone).expect("clones");
    let plain = Edit::SetChannelEdits {
        channel: ChannelId(2),
        edits: SoundEdits::default(),
    };
    edit::apply(&mut project, plain).expect("plain");
    let mut notes = project.patterns[&PatternId(1)].notes.clone();
    notes.push(dawsome_engine::project::Note {
        channel: ChannelId(2),
        ..notes[0].clone()
    });
    project
        .patterns
        .get_mut(&PatternId(1))
        .expect("a pattern")
        .notes = notes;
    let out = render(&project, PlayMode::Pattern(PatternId(1)), 64, &[512]);
    // The reversed ramp 64/64 .. 1/64 and the ramp 1/64 .. 64/64 together: 65/64 every frame.
    for (i, frame) in out.iter().enumerate() {
        assert_eq!(frame[0], 65.0 / 64.0, "frame {i}");
    }
}

/// Sounds whose edited ones are being made until `ready`: meanwhile each gives its file as it
/// is, for now, as media does while its worker edits.
struct Making {
    ready: bool,
    asked: usize,
}

impl Sounds for Making {
    type Error = Infallible;

    fn sample(&mut self, path: &Path) -> Result<Option<SampleBuffer>, Infallible> {
        Ok(Some(wav(&fixtures().join(path))))
    }

    fn clip(&mut self, path: &Path, _hint: ClipHint) -> Result<ClipSound, Infallible> {
        Ok(ClipSound::Memory(wav(&fixtures().join(path))))
    }

    fn edited_sample(
        &mut self,
        path: &Path,
        edits: &SoundEdits,
    ) -> Result<Option<EditedSound<SampleBuffer>>, Infallible> {
        self.asked += 1;
        let file = wav(&fixtures().join(path));
        Ok(Some(if self.ready {
            EditedSound::ready(apply(&file, edits))
        } else {
            EditedSound::for_now(file)
        }))
    }

    fn edited_clip(
        &mut self,
        path: &Path,
        edits: &SoundEdits,
        _hint: ClipHint,
    ) -> Result<EditedSound<ClipSound>, Infallible> {
        self.asked += 1;
        let file = wav(&fixtures().join(path));
        Ok(if self.ready {
            EditedSound::ready(ClipSound::Memory(apply(&file, edits)))
        } else {
            EditedSound::for_now(ClipSound::Memory(file))
        })
    }
}

// While an edited sound is made the channel plays the sound it had (here its file), never kept:
// each compile asks again, and a clip keeps the source it plays until the edited one is in; then
// the edited sound plays and is kept (no more asks).
#[test]
fn p30_a_sound_for_now_plays_until_the_edited_one_is_in() {
    for (fixture, mode) in [
        ("p30_reverse60.json", PlayMode::Pattern(PatternId(1))),
        ("p30_clip.json", PlayMode::Song),
    ] {
        let project = load(fixture);
        let edits = project.channels[&ChannelId(1)].edits.clone();
        let file = wav(&fixtures().join(&project.channels[&ChannelId(1)].sample));
        let edited = apply(&file, &edits);
        let mut sounds = Making {
            ready: false,
            asked: 0,
        };
        let mut compiler = Compiler::new();
        let compile = |compiler: &mut Compiler, sounds: &mut Making| {
            compiler
                .compile_with(&project, &Touched::default(), mode, sounds)
                .expect("compiles")
        };
        let first = compile(&mut compiler, &mut sounds);
        let (mut link, mut player) = realtime::link(first, project.tempo);
        link.send(Control::Play).expect("room");
        // The first 64 frames from the start, left side.
        let heard = |link: &mut realtime::RealtimeLink, player: &mut realtime::Player| {
            link.send(Control::Seek(dawsome_engine::time::Ticks(0)))
                .expect("room");
            let mut out = vec![[0.0f32; 2]; 512];
            player.process(&mut out);
            link.drain_retired();
            out[..64].iter().map(|frame| frame[0]).collect::<Vec<f32>>()
        };
        let left = |buffer: &SampleBuffer| -> Vec<f32> {
            (0..64)
                .map(|i| buffer.frame(i).expect("a frame")[0])
                .collect()
        };
        assert_eq!(
            heard(&mut link, &mut player),
            left(&file),
            "{fixture}: for now"
        );
        assert_eq!(sounds.asked, 1, "{fixture}");
        let again = compile(&mut compiler, &mut sounds);
        assert_eq!(sounds.asked, 2, "{fixture}: asked again");
        link.push_plan(again).expect("same time base");
        assert_eq!(
            heard(&mut link, &mut player),
            left(&file),
            "{fixture}: still"
        );
        sounds.ready = true;
        let ready = compile(&mut compiler, &mut sounds);
        link.push_plan(ready).expect("same time base");
        assert_eq!(
            heard(&mut link, &mut player),
            left(&edited),
            "{fixture}: edited"
        );
        assert_eq!(sounds.asked, 3, "{fixture}");
        let kept = compile(&mut compiler, &mut sounds);
        assert_eq!(sounds.asked, 3, "{fixture}: kept, not asked");
        link.push_plan(kept).expect("same time base");
        assert_eq!(
            heard(&mut link, &mut player),
            left(&edited),
            "{fixture}: kept"
        );
    }
}
