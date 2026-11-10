//! Time signatures per pattern and along the song: a pattern's own signature (its file, its
//! edit, its copy, the metronome and count-in while it plays alone) and the song's meter map
//! (its bars, snaps, metronome, count-ins, the change edits and the rule that settles the
//! changes after one, the file that keeps them). Expected values come from
//! tests/fixtures/gen_p35_expected.py (p35_expected.json and its projects), never from the
//! engine.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, copies, Edit, History, Touched};
use dawsome_engine::plan::PlayMode;
use dawsome_engine::project::{ClipId, PatternId, Project};
use dawsome_engine::realtime::{self, Player, RealtimeLink};
use dawsome_engine::time::{MeterUnit, SignatureChange, Ticks, TimeSignature};
use dawsome_engine::transport::Control;
use serde_json::Value;

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p35_expected.json"))
        .expect("p35_expected.json (run gen_p35_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("parses")
}

fn text(name: &str) -> String {
    fs::read_to_string(fixtures().join(name)).expect("a p35 fixture")
}

fn load(name: &str) -> Project {
    Project::load_json(&text(name)).expect("loads")
}

fn signature(value: &Value) -> TimeSignature {
    let beats = value[0].as_u64().expect("beats") as u8;
    let unit = value[1].as_u64().expect("unit") as u8;
    TimeSignature::new(beats, unit).expect("a signature the generator lists")
}

fn ticks(value: &Value) -> Ticks {
    Ticks(value.as_i64().expect("a tick"))
}

/// The play mode a case names: "song" or "pattern N".
fn mode(value: &Value) -> PlayMode {
    match value.as_str().expect("a mode") {
        "song" => PlayMode::Song,
        named => PlayMode::Pattern(PatternId(
            named
                .strip_prefix("pattern ")
                .and_then(|id| id.parse().ok())
                .expect("pattern N"),
        )),
    }
}

/// A player of `project` in `mode`, its files one frame of silence, the metronome on as
/// `metronome` says.
fn player(project: &Project, mode: PlayMode, metronome: bool) -> (RealtimeLink, Player) {
    let silence = |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(vec![0.0]));
    let update = Compiler::new()
        .compile(project, &Touched::default(), mode, silence)
        .expect("compiles");
    let (mut link, player) = realtime::link(update, project.tempo);
    link.send(Control::Metronome(metronome)).expect("room");
    (link, player)
}

/// `frames` frames of `player`, a call per size of `sizes` (the last repeated).
fn render(player: &mut Player, frames: usize, sizes: &[usize]) -> Vec<[f32; 2]> {
    let mut out = vec![[f32::NAN; 2]; frames];
    let mut done = 0;
    let mut sizes = sizes.iter().cycle();
    while done < frames {
        let end = (done + sizes.next().copied().unwrap_or(512)).min(frames);
        player.process(&mut out[done..end]);
        done = end;
    }
    out
}

/// The frames `want` says sound, by frame, both sides the value.
fn sounding(want: &Value) -> BTreeMap<usize, f32> {
    want["nonzero"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|entry| {
            let at = entry[0].as_u64().expect("a frame") as usize;
            (at, entry[1].as_f64().expect("a value") as f32)
        })
        .collect()
}

/// `player` renders `frames` frames whose only sound is the clicks `want` lists, to the bit,
/// for each way of cutting the render into calls.
fn assert_clicks(make: &dyn Fn() -> (RealtimeLink, Player), want: &Value, name: &str) {
    let frames = want["frames"].as_u64().expect("frames") as usize;
    let clicks = sounding(want);
    for sizes in [&[frames][..], &[1], &[64], &[4096], &[13, 1, 997, 2, 64]] {
        let (_link, mut player) = make();
        let out = render(&mut player, frames, sizes);
        for (at, got) in out.iter().enumerate() {
            let value = clicks.get(&at).copied().unwrap_or(0.0);
            assert_eq!(
                got.map(f32::to_bits),
                [value.to_bits(); 2],
                "{name}, blocks {sizes:?}, frame {at}"
            );
        }
    }
}

/// The count-in's clicks a case lists: the burst at each, accented on the first.
fn count_in_clicks(answer: &Value, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; frames];
    for (index, at) in answer["clicks"]
        .as_array()
        .expect("clicks")
        .iter()
        .enumerate()
    {
        let at = at.as_u64().expect("a frame") as usize;
        let amplitude = if index == 0 { 0.25 } else { 0.125 };
        for n in 0..64 {
            let sign = if (n / 8) % 2 == 0 { 1.0 } else { -1.0 };
            out[at + n] += amplitude * (1.0 - n as f32 / 64.0) * sign;
        }
    }
    out
}

/// Record with a count-in from `answer`'s start tick in `mode`: its clicks, the silent song
/// after them and the frame the recording starts at.
fn assert_count_in(project: &Project, mode: PlayMode, answer: &Value, name: &str) {
    let start = answer["start"].as_u64().expect("a start") as usize;
    let (mut link, mut player) = self::player(project, mode, false);
    link.send(Control::Seek(ticks(&answer["start_tick"])))
        .expect("room");
    link.send(Control::Record {
        count_in: true,
        tap_master: false,
    })
    .expect("room");
    let out = render(&mut player, start + 1000, &[512]);
    let clicks = count_in_clicks(answer, start);
    for (at, &value) in clicks.iter().enumerate() {
        assert_eq!(out[at], [value; 2], "{name}: count-in frame {at}");
    }
    assert!(
        out[start..].iter().all(|frame| *frame == [0.0; 2]),
        "{name}: the song after it"
    );
    assert_eq!(
        link.record_clock().recording_from(),
        Some(start as u64),
        "{name}: the recording starts where the count-in's bar ends"
    );
    // 1000 frames at 120 BPM and 44.1 kHz are 4.35 ticks: the song plays on from the tick
    // Record started from, not from the bar after it.
    let from = ticks(&answer["start_tick"]).0;
    let now = link.position().ticks.0;
    assert!(
        (from..from + 24).contains(&now),
        "{name}: the song plays on from {from}, now {now}"
    );
}

// ---------------- Phase B: a pattern's own signature ----------------

// The file: pattern 2's own 3/4 right after its bars, pattern 1 (the project's) without a field,
// version 10; load and save gives the bytes back; the accessor gives each its signature and its
// length in ticks (288 and 384).
#[test]
fn a_patterns_own_signature_is_kept_in_the_file() {
    let bytes = text("p35_pattern_34.json");
    let project = load("p35_pattern_34.json");
    assert_eq!(project.to_json().expect("saves"), bytes.trim_end());
    assert!(bytes.contains(r#""format_version":10,"#));
    assert!(bytes.contains(
        r#""name":"Pattern 2","length_bars":1,"time_signature":{"beats":3,"unit":4},"notes""#
    ));
    assert!(bytes.contains(r#""name":"Pattern 1","length_bars":1,"notes""#));
    let three_four = TimeSignature::new(3, 4).expect("3/4");
    assert_eq!(project.pattern_signature(PatternId(2)), three_four);
    assert_eq!(
        project.pattern_signature(PatternId(1)),
        TimeSignature::COMMON
    );
    assert_eq!(project.pattern_ticks(PatternId(2)), Ticks(288));
    assert_eq!(project.pattern_ticks(PatternId(1)), Ticks(384));
    let mut bad = bytes.replace(r#"{"beats":3,"unit":4}"#, r#"{"beats":3,"unit":2}"#);
    let error = Project::load_json(&bad)
        .expect_err("a pattern signature out of range is refused")
        .to_string();
    assert!(
        error.contains("3/2") && error.contains("Pattern 2"),
        "{error}"
    );
    bad = bytes.replace(r#""format_version":10"#, r#""format_version":11"#);
    assert!(
        Project::load_json(&bad).is_err(),
        "a later version is refused"
    );
}

// Known answers B 4: pattern 1 given 3/4 grows from 1 to 2 bars (its hit at 300 ends at 301,
// ceil(301 / 288) = 2), its field written; undo gives the first bytes back, redo the second's;
// 3/4 again touches nothing; the project's signature back keeps 2 bars (never shorter).
#[test]
fn a_patterns_signature_grows_it_never_shortens_and_undoes() {
    let want = &expected()["b"]["edit"];
    let before = text(want["before"].as_str().expect("a file"));
    let after = text(want["after"].as_str().expect("a file"));
    let none = text(want["none"].as_str().expect("a file"));
    let pattern = PatternId(want["pattern"].as_u64().expect("a pattern") as u32);
    let sig = Some(signature(&want["signature"]));
    let mut project = Project::load_json(&before).expect("loads");
    let mut history = History::new();
    let touched = history
        .apply(
            &mut project,
            Edit::SetPatternTimeSignature {
                pattern,
                signature: sig,
            },
        )
        .expect("applies");
    assert_eq!(project.to_json().expect("saves"), after.trim_end());
    assert_eq!(touched.patterns, [pattern].into());
    history.undo(&mut project).expect("undoes");
    assert_eq!(project.to_json().expect("saves"), before.trim_end());
    history.redo(&mut project).expect("redoes");
    assert_eq!(project.to_json().expect("saves"), after.trim_end());
    let again = edit::apply(
        &mut project,
        Edit::SetPatternTimeSignature {
            pattern,
            signature: sig,
        },
    )
    .expect("applies");
    assert!(
        again.touched.is_empty(),
        "the same signature touches nothing"
    );
    history
        .apply(
            &mut project,
            Edit::SetPatternTimeSignature {
                pattern,
                signature: None,
            },
        )
        .expect("applies");
    assert_eq!(project.to_json().expect("saves"), none.trim_end());
    history.undo(&mut project).expect("undoes");
    assert_eq!(project.to_json().expect("saves"), after.trim_end());
    let refused = edit::apply(
        &mut project,
        Edit::SetPatternTimeSignature {
            pattern: PatternId(9),
            signature: sig,
        },
    );
    assert!(refused.is_err(), "no pattern 9");
}

// The project's signature leaves a pattern with its own alone: p35_pattern_34 given 6/8 keeps
// pattern 2's 3/4 and its one bar.
#[test]
fn the_projects_signature_leaves_a_pattern_with_its_own() {
    let mut project = load("p35_pattern_34.json");
    edit::apply(
        &mut project,
        Edit::SetTimeSignature {
            signature: TimeSignature::new(6, 8).expect("6/8"),
        },
    )
    .expect("applies");
    assert_eq!(
        project.pattern_signature(PatternId(2)),
        TimeSignature::new(3, 4).expect("3/4")
    );
    assert_eq!(project.patterns[&PatternId(2)].length_bars, 1);
    assert_eq!(
        project.pattern_signature(PatternId(1)),
        TimeSignature::new(6, 8).expect("6/8")
    );
}

// A clip given its own copy keeps its pattern's signature (and its bars); undo takes it out.
#[test]
fn a_copy_keeps_its_patterns_signature() {
    let mut project = load("p35_pattern_34.json");
    let before = project.to_json().expect("saves");
    let unlink = copies::unlink(&project, ClipId(1)).expect("clip 1 plays pattern 2");
    let mut history = History::new();
    history.apply(&mut project, unlink.edit).expect("applies");
    let copy = &project.patterns[&unlink.copy];
    assert_eq!(copy.time_signature, TimeSignature::new(3, 4));
    assert_eq!(copy.length_bars, 1);
    assert_eq!(project.pattern_ticks(unlink.copy), Ticks(288));
    history.undo(&mut project).expect("undoes");
    assert_eq!(project.to_json().expect("saves"), before);
}

// Known answers B 4: the metronome of pattern 2 played alone accents every 288 ticks (its 3/4),
// of the song every 384 (the song's 4/4); to the bit, whole and split.
#[test]
fn the_metronome_counts_the_pattern_alone_and_the_song_by_their_own_bars() {
    let want = expected();
    let project = load(want["b"]["project"].as_str().expect("a file"));
    for (name, answer) in want["b"]["metronome"].as_object().expect("cases") {
        let mode = mode(&answer["mode"]);
        let make = || {
            let (mut link, player) = player(&project, mode, true);
            link.send(Control::Play).expect("room");
            (link, player)
        };
        assert_clicks(&make, answer, name);
    }
}

// Known answers B 4: Record's count-in is one bar of the meter that plays: three quarters in
// pattern 2 alone, four in the song.
#[test]
fn the_count_in_counts_the_pattern_alone_by_its_own_bar() {
    let want = expected();
    let project = load(want["b"]["project"].as_str().expect("a file"));
    for (name, answer) in want["b"]["count_in"].as_object().expect("cases") {
        assert_count_in(&project, mode(&answer["mode"]), answer, name);
    }
}

// ---------------- Phase C: changes along the song ----------------

// The file: the changes right after the ppq (no time_signature in a 4/4 song), version 10; load
// and save gives the bytes back.
#[test]
fn the_changes_are_kept_in_the_file() {
    let bytes = text("p35_meter.json");
    let project = load("p35_meter.json");
    assert_eq!(project.to_json().expect("saves"), bytes.trim_end());
    assert!(bytes.contains(
        r#""ppq":96,"signature_changes":[{"at":768,"beats":3,"unit":4},{"at":1344,"beats":6,"unit":8},{"at":1920,"beats":7,"unit":8}],"next_channel_id""#
    ));
    assert!(bytes.contains(r#""format_version":10,"#));
    let changes: Vec<(Ticks, TimeSignature)> = expected()["c"]["changes"]
        .as_array()
        .expect("changes")
        .iter()
        .map(|change| (ticks(&change[0]), signature(&change[1])))
        .collect();
    let got: Vec<(Ticks, TimeSignature)> = project
        .signature_changes
        .iter()
        .map(|change| (change.at, change.signature))
        .collect();
    assert_eq!(got, changes);
    assert!(!text("p35_edit_c.json").contains("signature_changes"));
}

// A file whose changes break a rule is refused: at 0, off its bar line, out of order, two on a
// tick, a signature out of range.
#[test]
fn a_file_with_changes_off_their_bar_lines_is_refused() {
    let bytes = text("p35_meter.json");
    let shown = r#"[{"at":768,"beats":3,"unit":4},{"at":1344,"beats":6,"unit":8},{"at":1920,"beats":7,"unit":8}]"#;
    assert!(bytes.contains(shown));
    for bad in expected()["c"]["bad_files"].as_array().expect("cases") {
        let file = bytes.replace(shown, &bad.to_string());
        assert!(Project::load_json(&file).is_err(), "{bad} is refused");
    }
}

// Known answers C 6: the map's bars (numbered on across the changes), the bar each tick is in,
// and its snaps to bars and beats.
#[test]
fn the_meter_map_counts_bars_across_the_changes() {
    let want = &expected()["c"];
    let map = load(want["project"].as_str().expect("a file")).meter_map();
    for (bar, start) in want["bars"].as_array().expect("bars").iter().enumerate() {
        assert_eq!(map.bar_start(bar as i64), ticks(start), "bar {bar}");
    }
    for case in want["bar_at"].as_array().expect("cases") {
        let at = map.bar_at(ticks(&case[0]));
        assert_eq!(
            (at.bar, at.start, at.signature),
            (
                case[1].as_i64().expect("a bar"),
                ticks(&case[2]),
                signature(&case[3])
            ),
            "bar_at({})",
            case[0]
        );
    }
    for case in want["snaps"].as_array().expect("cases") {
        let tick = ticks(&case[0]);
        let unit = match case[1].as_str() {
            Some("bar") => MeterUnit::Bar,
            _ => MeterUnit::Beat,
        };
        assert_eq!(map.snap_down(tick, unit), ticks(&case[2]), "{case}: down");
        assert_eq!(
            map.snap_nearest(tick, unit),
            ticks(&case[3]),
            "{case}: nearest"
        );
        assert_eq!(map.snap_next(tick, unit), ticks(&case[4]), "{case}: next");
    }
}

// Known answers C 6: the song's metronome clicks every beat of the meter it is in (quarters to
// 1344, eighths after) and accents each bar's first, 0, 384, 768, 1056, 1344, 1632, 1920, 2256;
// to the bit, whole and split.
#[test]
fn the_metronome_follows_the_changes() {
    let want = expected();
    let project = load(want["c"]["project"].as_str().expect("a file"));
    let make = || {
        let (mut link, player) = player(&project, PlayMode::Song, true);
        link.send(Control::Play).expect("room");
        (link, player)
    };
    assert_clicks(&make, &want["c"]["metronome"], "the map");
}

// Known answers C 6: Record from 1056 counts in one bar of 3/4 (3 quarters), from 1632 one of
// 6/8 (6 eighths); the song plays on from the tick it started from.
#[test]
fn the_count_in_is_one_bar_of_the_meter_where_recording_starts() {
    let want = expected();
    let project = load(want["c"]["project"].as_str().expect("a file"));
    for (name, answer) in want["c"]["count_in"].as_object().expect("cases") {
        assert_count_in(&project, PlayMode::Song, answer, name);
    }
}

// The audio thread's rule holds across changes: the song's metronome over the map, and a
// count-in in its 6/8, make no heap call.
#[test]
fn the_metronome_and_count_in_over_a_map_never_touch_the_heap() {
    let project = load("p35_meter.json");
    let (mut link, mut player) = player(&project, PlayMode::Song, true);
    link.send(Control::Play).expect("room");
    let mut out = vec![[0.0; 2]; 512];
    let calls: usize = (0..1200)
        .map(|_| heap_calls_during(|| player.process(&mut out)))
        .sum();
    assert_eq!(calls, 0, "the metronome made {calls} heap calls");
    link.send(Control::Stop).expect("room");
    link.send(Control::Seek(Ticks(1632))).expect("room");
    link.send(Control::Record {
        count_in: true,
        tap_master: false,
    })
    .expect("room");
    let calls: usize = (0..200)
        .map(|_| heap_calls_during(|| player.process(&mut out)))
        .sum();
    assert_eq!(calls, 0, "the count-in made {calls} heap calls");
}

fn change_edit(case: &Value) -> Edit {
    if let Some(set) = case.get("set") {
        Edit::SetSignatureChange {
            at: ticks(&set["at"]),
            signature: signature(&set["signature"]),
        }
    } else if let Some(moved) = case.get("move") {
        Edit::MoveSignatureChange {
            from: ticks(&moved["from"]),
            to: ticks(&moved["to"]),
        }
    } else if let Some(removed) = case.get("remove") {
        Edit::RemoveSignatureChange {
            at: ticks(&removed["at"]),
        }
    } else {
        Edit::SetTimeSignature {
            signature: signature(&case["start"]["signature"]),
        }
    }
}

// Known answers C 6: each change edit gives its document byte for byte (the change snapped to
// its bar line, replacing one there, the later changes settled down, or up past the one before);
// undo gives the first bytes back, redo the second's; a move back onto its own bar line changes
// nothing.
#[test]
fn the_change_edits_settle_the_later_changes_and_undo_exactly() {
    for case in expected()["c"]["edits"].as_array().expect("cases") {
        let name = case["name"].as_str().expect("a name");
        let before = text(case["before"].as_str().expect("a file"));
        let after = text(case["after"].as_str().expect("a file"));
        let mut project = Project::load_json(&before).expect("loads");
        let mut history = History::new();
        let touched = history
            .apply(&mut project, change_edit(case))
            .expect("applies");
        assert_eq!(
            project.to_json().expect("saves"),
            after.trim_end(),
            "{name}"
        );
        project.validate().expect("valid");
        if before == after {
            assert!(
                touched.is_empty(),
                "{name}: nothing changed, nothing touched"
            );
            continue;
        }
        assert!(!touched.is_empty(), "{name}: the plan is built again");
        history.undo(&mut project).expect("undoes");
        assert_eq!(
            project.to_json().expect("saves"),
            before.trim_end(),
            "{name}: undo"
        );
        history.redo(&mut project).expect("redoes");
        assert_eq!(
            project.to_json().expect("saves"),
            after.trim_end(),
            "{name}: redo"
        );
    }
}

// The project's signature is the song's meter, which the plan carries (the metronome, the
// count-in, the bar plugins are told): setting it, and undoing that, rebuild the plan even in a
// project with no pattern, where no pattern is there to touch.
#[test]
fn the_projects_signature_rebuilds_the_plan_without_a_pattern() {
    let mut project = load("p35_edit_c.json");
    project.playlist.clips.clear();
    project.patterns.clear();
    project.validate().expect("valid without patterns");
    let applied = edit::apply(
        &mut project,
        Edit::SetTimeSignature {
            signature: TimeSignature::new(3, 4).expect("3/4"),
        },
    )
    .expect("applies");
    assert!(applied.touched.playlist, "the plan is built again");
    let undone = edit::apply(&mut project, applied.inverse).expect("undoes");
    assert!(undone.touched.playlist, "and again on undo");
    assert_eq!(project.time_signature, TimeSignature::COMMON);
}

// A change in the song's first bar is refused (the project's own signature rules there), and so
// is removing or moving a tick with no change; the document stays as it was.
#[test]
fn a_change_at_the_start_or_of_nothing_is_refused() {
    let bytes = text("p35_meter.json");
    for case in expected()["c"]["refused"].as_array().expect("cases") {
        let mut project = Project::load_json(&bytes).expect("loads");
        assert!(
            edit::apply(&mut project, change_edit(case)).is_err(),
            "{case} is refused"
        );
        assert_eq!(
            project.to_json().expect("saves"),
            bytes.trim_end(),
            "{case}"
        );
    }
    let mut project = Project::load_json(&bytes).expect("loads");
    let bad = edit::apply(
        &mut project,
        Edit::RestoreSignatureChanges {
            changes: vec![SignatureChange {
                at: Ticks(800),
                signature: TimeSignature::new(3, 4).expect("3/4"),
            }],
        },
    );
    assert!(bad.is_err(), "a list off its bar lines is not put back");
}
