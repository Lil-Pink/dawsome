//! MIDI, stretch and the ported effects in the engine: SMF import and export, controller links and
//! clip stretch in the model with exact inverses, live notes on the player (stopped or playing,
//! muted samplers, the held-note table, the 16-voice pool) and none of it touching the heap, and
//! the two ported effects nulled against the committed renders of the originals. Expected values
//! come from tests/fixtures/p9_expected.json (tests/fixtures/gen_p9_expected.py),
//! tests/fixtures/ref, or are written out by hand beside each test.

use std::convert::Infallible;
use std::path::Path;

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, Edit, Touched};
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::processor::{param_values, ProcessContext, Processor, Registry, TransportInfo};
use dawsome_engine::project::{
    AutomationTarget, ChannelId, ClipId, ClipItem, ClipStretch, Controller, InsertId, Note,
    PatternId, Project, StretchMode, ValidationError,
};
use dawsome_engine::realtime::{self, EngineEvent, LiveNote, Player, RealtimeLink};
use dawsome_engine::smf;
use dawsome_engine::time::{MilliBpm, Ticks};
use dawsome_engine::transport::Control;

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

fn fixture(name: &str) -> Project {
    let text = std::fs::read_to_string(Path::new(FIXTURES).join(name)).expect("fixture reads");
    Project::load_json(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn json(project: &Project) -> String {
    project.to_json().expect("serializes")
}

fn expected() -> serde_json::Value {
    let text = std::fs::read_to_string(Path::new(FIXTURES).join("p9_expected.json"))
        .expect("p9_expected.json (run gen_p9_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("parses")
}

/// The generator writes each f32 as the shortest decimal that names it, so `as f32` is exact.
fn f32_of(value: &serde_json::Value) -> f32 {
    value.as_f64().expect("a number") as f32
}

fn u64_of(value: &serde_json::Value) -> u64 {
    value.as_u64().expect("a whole number")
}

/// Notes as the generator writes them: channel, tick, length, velocity and the key (60 when
/// it is left out).
fn expected_notes(list: &serde_json::Value) -> Vec<Note> {
    list.as_array()
        .expect("notes")
        .iter()
        .map(|note| Note {
            channel: ChannelId(u64_of(&note["channel"]) as u32),
            start: Ticks(u64_of(&note["tick"]) as i64),
            length: Ticks(u64_of(&note["length"]) as i64),
            velocity: f32_of(&note["velocity"]),
            key: note.get("key").map_or(60, |key| u64_of(key) as u8),
        })
        .collect()
}

/// Applies `edit`, then its inverse, and checks the document comes back byte for byte.
fn round_trip(project: &mut Project, edit: Edit) {
    let before = json(project);
    let applied = edit::apply(project, edit.clone()).unwrap_or_else(|e| panic!("{edit:?}: {e}"));
    assert_ne!(json(project), before, "{edit:?} changed nothing");
    edit::apply(project, applied.inverse).expect("the inverse applies");
    assert_eq!(
        json(project),
        before,
        "{edit:?}: the inverse restores bytes"
    );
}

fn import_mid() -> Vec<u8> {
    std::fs::read(Path::new(FIXTURES).join("p9_import.mid")).expect("p9_import.mid")
}

// ---------------------------------------------------------------------------------------------
// SMF
// ---------------------------------------------------------------------------------------------

// (1) The hand-written format-1 file at PPQ 480 replaces channel 1's one step with four notes
// rescaled to 96 (half up), merges both MIDI channels, keeps the first of two notes landing on
// one (tick, key), closes an unclosed note at its track's end and grows the pattern to 2 bars;
// one undo puts the step back.
#[test]
fn an_smf_import_replaces_the_channels_notes_as_the_known_answer_says() {
    let expected = &expected()["import"];
    let mut project = fixture("p9_import_base.json");
    let before = json(&project);
    let imported = smf::import_into(&project, PatternId(1), ChannelId(1), &import_mid())
        .expect("the file imports");
    let report = &imported.report;
    assert_eq!(
        (report.dropped, report.unclosed, report.ignored),
        (
            u64_of(&expected["dropped"]) as u32,
            u64_of(&expected["unclosed"]) as u32,
            u64_of(&expected["ignored"]) as u32
        )
    );
    assert_eq!(report.tempo, Some(MilliBpm(120_000)));
    let summary = report.summary();
    for words in [
        "1 overlapping note dropped",
        "1 note closed at track end",
        "3 other events ignored",
        "tempo 120 BPM not applied",
    ] {
        assert!(summary.contains(words), "{summary}");
    }
    let applied = edit::apply(&mut project, imported.edit).expect("the import applies");
    let pattern = &project.patterns[&PatternId(1)];
    assert_eq!(pattern.notes, expected_notes(&expected["notes"]));
    assert_eq!(
        u64::from(pattern.length_bars),
        u64_of(&expected["length_bars"])
    );
    edit::apply(&mut project, applied.inverse).expect("one undo");
    assert_eq!(json(&project), before, "one undo restores the step");
}

// (2) Format 0, division 96: tempo, 4/4 and the pattern name at tick 0, offs before ons, running
// status, end of track at the last event; the file reads back to the same notes.
#[test]
fn an_smf_export_writes_the_known_bytes_and_reads_back() {
    let expected = expected();
    let mut project = fixture("p9_import_base.json");
    let imported =
        smf::import_into(&project, PatternId(1), ChannelId(1), &import_mid()).expect("imports");
    edit::apply(&mut project, imported.edit).expect("applies");
    let bytes = smf::export(&project, PatternId(1), ChannelId(1)).expect("exports");
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(hex, expected["export"]["hex"].as_str().expect("hex"));
    let mut again = fixture("p9_import_base.json");
    let back = smf::import_into(&again, PatternId(1), ChannelId(1), &bytes).expect("re-imports");
    edit::apply(&mut again, back.edit).expect("applies");
    assert_eq!(
        again.patterns[&PatternId(1)].notes,
        project.patterns[&PatternId(1)].notes,
        "re-import == import"
    );
}

// A step (length 0) exports 24 ticks long, and velocity 1.0 as byte 127.
#[test]
fn a_step_exports_one_step_long_at_velocity_127() {
    let base = fixture("p9_import_base.json");
    let bytes = smf::export(&base, PatternId(1), ChannelId(1)).expect("exports");
    let mut again = base.clone();
    let back = smf::import_into(&again, PatternId(1), ChannelId(1), &bytes).expect("imports");
    edit::apply(&mut again, back.edit).expect("applies");
    let notes = &again.patterns[&PatternId(1)].notes;
    assert_eq!(notes.len(), 1);
    assert_eq!(
        (notes[0].start, notes[0].length, notes[0].velocity),
        (Ticks(0), Ticks(24), 127.0 / 128.0)
    );
}

#[test]
fn smpte_timing_and_garbage_are_refused() {
    let base = fixture("p9_import_base.json");
    let mut smpte = import_mid();
    // The division word: bit 15 set is SMPTE timing (-25 fps, 40 subframes).
    smpte[12] = 0xE7;
    smpte[13] = 0x28;
    assert!(matches!(
        smf::import_into(&base, PatternId(1), ChannelId(1), &smpte),
        Err(smf::SmfError::Smpte)
    ));
    assert!(smf::import_into(&base, PatternId(1), ChannelId(1), b"not a midi file").is_err());
    assert!(smf::import_into(&base, PatternId(9), ChannelId(1), &import_mid()).is_err());
}

/// A format 0 file at PPQ 1 of one note on key 60 from tick 0, `quarters` long: its delta
/// written as a variable-length quantity, the largest a file holds being 0x0FFFFFFF.
fn one_note_mid(quarters: u32) -> Vec<u8> {
    let mut delta = vec![(quarters & 0x7F) as u8];
    let mut rest = quarters >> 7;
    while rest > 0 {
        delta.insert(0, (rest & 0x7F) as u8 | 0x80);
        rest >>= 7;
    }
    let mut track = vec![0x00, 0x90, 0x3C, 0x40];
    track.extend_from_slice(&delta);
    track.extend_from_slice(&[0x80, 0x3C, 0x40, 0x00, 0xFF, 0x2F, 0x00]);
    let mut bytes = b"MThd\0\0\0\x06\0\0\0\x01\0\x01MTrk".to_vec();
    bytes.extend_from_slice(&u32::try_from(track.len()).expect("short").to_be_bytes());
    bytes.extend_from_slice(&track);
    bytes
}

// A 37-byte file whose one note lasts 0x0FFFFFFF quarters would grow the pattern to
// ceil(0x0FFFFFFF · 96 / 384) = 67,108,864 bars. The longest song is MAX_SONG_BARS bars, so
// a note ending on bar 100,000 (400,000 quarters) imports and one a quarter later is refused.
#[test]
fn a_file_reaching_past_the_longest_song_is_refused() {
    let base = fixture("p9_import_base.json");
    let huge = one_note_mid(0x0FFF_FFFF);
    assert_eq!(huge.len(), 37);
    assert!(matches!(
        smf::import_into(&base, PatternId(1), ChannelId(1), &huge),
        Err(smf::SmfError::TooLong)
    ));
    let longest = smf::import_into(&base, PatternId(1), ChannelId(1), &one_note_mid(400_000))
        .expect("a note ending on the last bar imports");
    let Edit::Transaction(edits) = longest.edit else {
        panic!("the pattern grows: {:?}", longest.edit);
    };
    assert!(matches!(
        edits.last(),
        Some(Edit::SetPatternBars { bars: 100_000, .. })
    ));
    assert!(matches!(
        smf::import_into(&base, PatternId(1), ChannelId(1), &one_note_mid(400_001)),
        Err(smf::SmfError::TooLong)
    ));
}

// ---------------------------------------------------------------------------------------------
// The model: controller links and clip stretch
// ---------------------------------------------------------------------------------------------

fn fader_1() -> AutomationTarget {
    AutomationTarget::InsertFader(InsertId(1))
}

fn pan_1() -> AutomationTarget {
    AutomationTarget::ChannelPan(ChannelId(1))
}

fn controller(cc: u8, channel: Option<u8>) -> Option<Controller> {
    Some(Controller { cc, channel })
}

// p9_cc names cc 10 on MIDI channel 1 (index 0) for channel 1's pan and cc 7 on any channel for
// insert 1's fader, in target order, as the file writes them.
#[test]
fn controller_links_load_write_and_undo_exactly() {
    let mut project = fixture("p9_cc.json");
    assert_eq!(project.midi_links.get(&pan_1()), controller(10, Some(0)));
    assert_eq!(project.midi_links.get(&fader_1()), controller(7, None));
    let targets: Vec<AutomationTarget> = project.midi_links.matching(10, 0).collect();
    assert_eq!(targets, [pan_1()]);
    assert_eq!(project.midi_links.matching(10, 1).count(), 0, "status B1");
    assert_eq!(
        project.midi_links.matching(7, 15).collect::<Vec<_>>(),
        [fader_1()]
    );
    // Unlink, link a new target, and move cc 7 from the fader to the volume.
    round_trip(
        &mut project,
        Edit::SetMidiLink {
            target: fader_1(),
            link: None,
        },
    );
    round_trip(
        &mut project,
        Edit::SetMidiLink {
            target: AutomationTarget::InsertPan(InsertId(2)),
            link: controller(1, Some(3)),
        },
    );
    let volume = AutomationTarget::ChannelVolume(ChannelId(1));
    let mut moved = project.clone();
    edit::apply(
        &mut moved,
        Edit::SetMidiLink {
            target: volume.clone(),
            link: controller(7, None),
        },
    )
    .expect("links");
    assert_eq!(moved.midi_links.get(&volume), controller(7, None));
    assert_eq!(moved.midi_links.get(&fader_1()), None, "cc 7 moved");
    round_trip(
        &mut project,
        Edit::SetMidiLink {
            target: volume,
            link: controller(7, None),
        },
    );
}

#[test]
fn a_link_is_refused_past_cc_119_channel_16_or_onto_a_missing_target() {
    let set = |target, cc, channel| {
        let mut project = fixture("p9_cc.json");
        let before = json(&project);
        let result = edit::apply(
            &mut project,
            Edit::SetMidiLink {
                target,
                link: controller(cc, channel),
            },
        );
        if result.is_err() {
            assert_eq!(json(&project), before, "a refused link changes nothing");
        }
        result.map(drop)
    };
    assert!(set(fader_1(), 119, Some(15)).is_ok());
    assert!(set(fader_1(), 120, None).is_err());
    assert!(set(fader_1(), 7, Some(16)).is_err());
    assert!(set(AutomationTarget::InsertFader(InsertId(17)), 7, None).is_err());
    let mut project = fixture("p9_cc.json");
    project.midi_links.insert(
        fader_1(),
        Controller {
            cc: 120,
            channel: None,
        },
    );
    assert!(matches!(
        project.validate(),
        Err(ValidationError::InvalidMidiLink { cc: 120, .. })
    ));
    // One controller drives one target: a file naming cc 7 on any channel twice is refused.
    let text = std::fs::read_to_string(Path::new(FIXTURES).join("p9_cc.json")).expect("reads");
    let twice = text.replace(
        r#"{"cc":10,"channel":0,"target""#,
        r#"{"cc":7,"channel":null,"target""#,
    );
    assert_ne!(twice, text);
    assert!(matches!(
        Project::load_json(&twice),
        Err(dawsome_engine::project::LoadError::Invalid(
            ValidationError::ControllerLinkedTwice { cc: 7, .. }
        ))
    ));
    // A link whose target has gone (its effect removed, say) is inert, as an automation of a
    // gone target is, so the document still loads.
    let gone = text.replace(
        r#"{"insert_fader":1}"#,
        r#"{"effect":{"id":9,"param":"mix"}}"#,
    );
    assert!(Project::load_json(&gone).is_ok());
}

fn stretch(mode: StretchMode, frames: u64, pitch: i8) -> Option<ClipStretch> {
    Some(ClipStretch {
        mode,
        frames,
        pitch,
    })
}

fn clip_stretch(project: &Project, clip: u32) -> Option<ClipStretch> {
    match project.playlist.clips[&ClipId(clip)].item {
        ClipItem::Audio { stretch, .. } => stretch,
        _ => panic!("clip {clip} is not an audio clip"),
    }
}

#[test]
fn a_clip_stretch_loads_writes_and_undoes_exactly() {
    let mut project = fixture("p9_stretch_pitch.json");
    assert_eq!(
        clip_stretch(&project, 1),
        stretch(StretchMode::Stretch, 44_100, 7)
    );
    let text = json(&project);
    assert!(
        text.contains(r#""stretch":{"mode":"stretch","frames":44100,"pitch":7}"#),
        "{text}"
    );
    round_trip(
        &mut project,
        Edit::SetClipStretch {
            clip: ClipId(1),
            stretch: stretch(StretchMode::Resample, 22_050, 0),
            frame_offset: 100,
            length: Ticks(96),
        },
    );
    round_trip(
        &mut project,
        Edit::SetClipStretch {
            clip: ClipId(1),
            stretch: None,
            frame_offset: 0,
            length: Ticks(384),
        },
    );
    // None is left out of the file: an unstretched clip writes what it wrote before stretch
    // existed.
    edit::apply(
        &mut project,
        Edit::SetClipStretch {
            clip: ClipId(1),
            stretch: None,
            frame_offset: 0,
            length: Ticks(384),
        },
    )
    .expect("unstretches");
    assert!(json(&project)
        .contains(r#"{"id":1,"track":1,"start":0,"length":384,"kind":"audio","channel":1}]"#));
}

// Pitch only on Stretch, within ±24; a ratio of 1/4..4 of the source's frames.
#[test]
fn stretch_rules_refuse_pitch_on_resample_and_ratios_past_4() {
    let set = |value| {
        let mut project = fixture("p9_stretch.json");
        let before = json(&project);
        let result = edit::apply(
            &mut project,
            Edit::SetClipStretch {
                clip: ClipId(1),
                stretch: value,
                frame_offset: 0,
                length: Ticks(384),
            },
        );
        if result.is_err() {
            assert_eq!(json(&project), before, "a refused stretch changes nothing");
        }
        result.map(drop)
    };
    assert!(set(stretch(StretchMode::Resample, 44_100, 0)).is_ok());
    assert!(set(stretch(StretchMode::Resample, 44_100, 3)).is_err());
    assert!(set(stretch(StretchMode::Stretch, 44_100, 24)).is_ok());
    assert!(set(stretch(StretchMode::Stretch, 44_100, -25)).is_err());
    assert!(set(stretch(StretchMode::Stretch, 0, 0)).is_err());
    let ratio = |frames| ClipStretch {
        mode: StretchMode::Stretch,
        frames,
        pitch: 0,
    };
    // The source has 44100 frames.
    assert!(ratio(4 * 44_100).fits(44_100));
    assert!(!ratio(5 * 44_100).fits(44_100), "a ratio of 5");
    assert!(ratio(44_100 / 4).fits(44_100));
    assert!(!ratio(44_100 / 4 - 1).fits(44_100));
    assert!(!ratio(1).fits(0), "an empty source stretches to nothing");
}

// ---------------------------------------------------------------------------------------------
// Live notes on the player
// ---------------------------------------------------------------------------------------------

/// Every file loads as `frames` frames of 1.0 (one frame: an impulse).
fn compile(
    compiler: &mut Compiler,
    project: &Project,
    touched: &Touched,
    frames: usize,
) -> PlanUpdate {
    let ones = |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(vec![1.0; frames]));
    compiler
        .compile(project, touched, PlayMode::Pattern(PatternId(1)), ones)
        .expect("compiles")
}

/// p9_record (channel 1 "keys", an empty bar) linked with channel 1 live, its file `frames`
/// frames of 1.0.
fn live(project: &Project, frames: usize) -> (Compiler, RealtimeLink, Player) {
    let mut compiler = Compiler::new();
    compiler.set_live_channel(Some(ChannelId(1)));
    let update = compile(&mut compiler, project, &Touched::default(), frames);
    let (link, player) = realtime::link(update, project.tempo);
    (compiler, link, player)
}

fn call(player: &mut Player) -> Vec<[f32; 2]> {
    let mut out = vec![[9.0; 2]; 512];
    player.process(&mut out);
    out
}

fn on(key: u8, velocity: f32) -> Control {
    Control::LiveNoteOn { key, velocity }
}

// A muted sampler channel still plays live notes, while stopped, at velocity × volume × pan gain,
// from frame 0 of the call; the event says so with no tick.
#[test]
fn a_live_note_sounds_on_a_muted_stopped_sampler_channel() {
    let mut project = fixture("p9_record.json");
    let keys = project.channels.get_mut(&ChannelId(1)).expect("keys");
    keys.mute = true;
    keys.volume = 0.5;
    let (_, mut link, mut player) = live(&project, 1);
    link.send(on(60, 0.5)).expect("room");
    let out = call(&mut player);
    assert_eq!(out[0], [0.25, 0.25]);
    assert!(out[1..].iter().all(|frame| *frame == [0.0; 2]));
    assert_eq!(
        link.take_events(),
        [EngineEvent::LiveNoteStarted {
            channel: ChannelId(1),
            key: 60,
            velocity: 0.5,
            tick: None,
            frame: 0
        }]
    );
}

// Live notes go through the channel's insert: muted, insert 1 is silent; unmuted, it passes.
#[test]
fn live_notes_render_through_the_insert_chain_while_stopped() {
    for muted in [true, false] {
        let mut project = fixture("p9_record.json");
        project
            .channels
            .get_mut(&ChannelId(1))
            .expect("keys")
            .insert = InsertId(1);
        project.mixer.inserts[1].mute = muted;
        let (_, mut link, mut player) = live(&project, 1);
        link.send(on(72, 1.0)).expect("room");
        let out = call(&mut player);
        let want = if muted { 0.0 } else { 1.0 };
        assert_eq!(out[0], [want; 2], "insert 1 muted: {muted}");
    }
}

// The inbox plays like the control ring, after it, from the next call's first frame; while
// playing, the events carry the playhead tick of the call's first sample (Python's tick rule
// at 44.1 kHz and 120 BPM: sample 44544 is in tick 193, sample 45056 in tick 196) and that
// sample's output frame (87 and 88 calls of 512 in: the clock started with the song).
#[test]
fn inbox_notes_while_playing_carry_the_playhead_tick() {
    let project = fixture("p9_record.json");
    let (_, mut link, mut player) = live(&project, 1);
    link.send(Control::Play).expect("room");
    for _ in 0..87 {
        call(&mut player);
    }
    link.take_events();
    assert!(link.midi_inbox().push(LiveNote::On {
        key: 64,
        velocity: 127.0 / 128.0
    }));
    let out = call(&mut player);
    assert_eq!(out[0], [127.0 / 128.0; 2]);
    assert!(link.midi_inbox().push(LiveNote::Off { key: 64 }));
    call(&mut player);
    assert_eq!(
        link.take_events(),
        [
            EngineEvent::LiveNoteStarted {
                channel: ChannelId(1),
                key: 64,
                velocity: 127.0 / 128.0,
                tick: Some(Ticks(193)),
                frame: 44_544
            },
            EngineEvent::LiveNoteEnded {
                channel: ChannelId(1),
                key: 64,
                tick: Some(Ticks(196)),
                frame: 45_056
            }
        ]
    );
}

// A held note remembers its channel: after the selection moves to channel 2 its note-off still
// ends channel 1's note, and the next note plays channel 2.
#[test]
fn a_note_off_after_a_selection_change_ends_the_right_note() {
    let mut project = fixture("p9_record.json");
    edit::apply(
        &mut project,
        Edit::AddSamplerChannel {
            id: ChannelId(2),
            name: "other".into(),
            sample: "impulse.wav".into(),
        },
    )
    .expect("adds");
    let (mut compiler, mut link, mut player) = live(&project, 1);
    link.send(on(60, 1.0)).expect("room");
    call(&mut player);
    compiler.set_live_channel(Some(ChannelId(2)));
    link.push_plan(compile(&mut compiler, &project, &Touched::default(), 1))
        .expect("same time base");
    // The new plan, and with it the new selection, takes effect at the start of a call.
    call(&mut player);
    link.send(Control::LiveNoteOff { key: 60 }).expect("room");
    link.send(on(62, 1.0)).expect("room");
    call(&mut player);
    let events = link.take_events();
    assert_eq!(
        events[1..],
        [
            EngineEvent::LiveNoteEnded {
                channel: ChannelId(1),
                key: 60,
                tick: None,
                frame: 1024
            },
            EngineEvent::LiveNoteStarted {
                channel: ChannelId(2),
                key: 62,
                velocity: 1.0,
                tick: None,
                frame: 1024
            }
        ]
    );
}

// 16 live voices; a repeated (channel, key) restarts its voice, a 17th note takes over the oldest.
// Seek leaves live notes ringing; Stop ends them (since 2026-10-28, as every note), and so does
// LiveNotesOff.
#[test]
fn the_live_pool_holds_16_voices_and_stop_or_live_notes_off_clears_it() {
    let project = fixture("p9_record.json");
    let (_, mut link, mut player) = live(&project, 100_000);
    link.send(on(60, 0.5)).expect("room");
    link.send(on(60, 0.5)).expect("room");
    assert_eq!(
        call(&mut player)[0],
        [0.5; 2],
        "the same key restarts one voice"
    );
    link.send(Control::LiveNotesOff).expect("room");
    call(&mut player);
    for key in 40..57 {
        link.send(on(key, 0.25)).expect("room");
    }
    assert_eq!(
        call(&mut player)[0],
        [16.0 * 0.25; 2],
        "17 notes, 16 voices"
    );
    link.send(Control::Seek(Ticks(96))).expect("room");
    assert_eq!(call(&mut player)[0], [16.0 * 0.25; 2], "Seek keeps them");
    link.send(Control::Stop).expect("room");
    assert!(
        call(&mut player).iter().all(|frame| *frame == [0.0; 2]),
        "Stop ends them"
    );
    for key in 40..56 {
        link.send(on(key, 0.25)).expect("room");
    }
    assert_eq!(call(&mut player)[0], [16.0 * 0.25; 2]);
    link.send(Control::LiveNotesOff).expect("room");
    assert!(call(&mut player).iter().all(|frame| *frame == [0.0; 2]));
}

// The counting-allocator check: 200 inbox notes, 50 live controls, a selection swap and a
// stolen live voice, playing and stopped, make no heap call on the audio thread.
#[test]
fn live_notes_never_touch_the_heap() {
    let mut project = fixture("p9_record.json");
    edit::apply(
        &mut project,
        Edit::AddSamplerChannel {
            id: ChannelId(2),
            name: "other".into(),
            sample: "impulse.wav".into(),
        },
    )
    .expect("adds");
    let (mut compiler, mut link, mut player) = live(&project, 50_000);
    let inbox = link.midi_inbox();
    compiler.set_live_channel(Some(ChannelId(2)));
    let swap = compile(&mut compiler, &project, &Touched::default(), 50_000);
    let mut swap = Some(swap);
    let mut out = vec![[0.0; 2]; 512];
    let mut calls = 0;
    for step in 0..200u8 {
        if step == 10 {
            link.send(Control::Play).expect("room");
        }
        if step == 120 {
            link.send(Control::Stop).expect("room");
        }
        if step == 60 {
            if let Some(update) = swap.take() {
                link.push_plan(update).expect("same time base");
            }
        }
        let key = 30 + step % 40;
        assert!(inbox.push(LiveNote::On { key, velocity: 0.5 }));
        if step % 4 == 0 {
            link.send(on(key.wrapping_add(50), 0.5)).expect("room");
            link.send(Control::LiveNoteOff { key }).expect("room");
        }
        calls += heap_calls_during(|| player.process(&mut out));
        link.drain_retired();
        link.take_events();
    }
    assert_eq!(calls, 0, "Player::process made {calls} heap calls");
}

// ---------------------------------------------------------------------------------------------
// The ported effects
// ---------------------------------------------------------------------------------------------

/// Interleaved stereo frames and the rate of a committed WAV.
fn read_wav(name: &str) -> (Vec<[f32; 2]>, u32) {
    let mut reader = hound::WavReader::open(Path::new(FIXTURES).join(name))
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    let spec = reader.spec();
    assert_eq!(
        (spec.channels, spec.sample_format, spec.bits_per_sample),
        (2, hound::SampleFormat::Float, 32),
        "{name}"
    );
    let values: Vec<f32> = reader
        .samples::<f32>()
        .collect::<Result<_, _>>()
        .expect("reads");
    let frames = values
        .chunks_exact(2)
        .map(|pair| [pair[0], pair[1]])
        .collect();
    (frames, spec.sample_rate)
}

/// A native processor by id at `rate`, its parameters set by key.
fn processor(id: &str, rate: u32, params: &[(&str, f32)]) -> Box<dyn Processor> {
    let registry = Registry::native();
    let factory = registry
        .get(id)
        .unwrap_or_else(|| panic!("{id} is registered"));
    let saved = params
        .iter()
        .map(|&(key, value)| (key.to_owned(), value))
        .collect();
    for (key, _) in params {
        assert!(
            factory.params().iter().any(|info| info.key == *key),
            "{id} has no parameter {key}"
        );
    }
    factory.create(rate, &param_values(factory.params(), &saved))
}

fn context(rate: u32) -> ProcessContext<'static> {
    ProcessContext {
        sample_rate: rate,
        tempo: MilliBpm(120_000),
        ppq: 96,
        transport: TransportInfo {
            playing: true,
            song_tick: Ticks(0),
            steady_time: 0,
        },
        events: &[],
    }
}

/// `input` through `effect` in pieces of the sizes `pieces` gives in turn.
fn run(
    effect: &mut dyn Processor,
    rate: u32,
    input: &[[f32; 2]],
    pieces: &mut dyn FnMut() -> usize,
) -> Vec<[f32; 2]> {
    let mut buffer = input.to_vec();
    let mut rest: &mut [[f32; 2]] = &mut buffer;
    while !rest.is_empty() {
        let (piece, later) = rest.split_at_mut(pieces().clamp(1, rest.len()));
        effect.process(&context(rate), piece);
        rest = later;
    }
    buffer
}

/// The null: the largest difference and how many samples differ at all.
fn null(ours: &[[f32; 2]], reference: &[[f32; 2]]) -> (f64, usize) {
    assert_eq!(ours.len(), reference.len(), "frame count");
    let mut largest = 0.0f64;
    let mut differing = 0;
    for (a, b) in ours.iter().flatten().zip(reference.iter().flatten()) {
        let diff = (f64::from(*a) - f64::from(*b)).abs();
        largest = largest.max(diff);
        differing += usize::from(a != b);
    }
    (largest, differing)
}

/// Seeded piece sizes: 1, 7 and 4096 in turn with xorshift sizes from 1 to 600 between.
fn seeded_pieces() -> impl FnMut() -> usize {
    let mut state = 0x2545_f491_u32;
    let mut turn = 0usize;
    move || {
        turn += 1;
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        match turn % 4 {
            0 => 1,
            1 => 7,
            2 => 4096,
            _ => 1 + (state % 600) as usize,
        }
    }
}

/// The null rule: max |diff| ≤ 2^-20 (about −120 dBFS) and no sample differs, in the originals'
/// 512-frame blocks and in seeded random pieces.
fn assert_nulls(id: &str, params: &[(&str, f32)], reference: &str) {
    let (input, input_rate) = read_wav("p9_fx_in.wav");
    let (expected, rate) = read_wav(reference);
    assert_eq!(input_rate, 44_100);
    let blocks = run(
        processor(id, rate, params).as_mut(),
        rate,
        &input,
        &mut || 512,
    );
    let (largest, differing) = null(&blocks, &expected);
    assert!(
        largest <= 2f64.powi(-20) && differing == 0,
        "{reference}: max |diff| {largest:e}, {differing} samples differ"
    );
    let split = run(
        processor(id, rate, params).as_mut(),
        rate,
        &input,
        &mut seeded_pieces(),
    );
    let (largest, differing) = null(&split, &expected);
    assert!(
        largest <= 2f64.powi(-20) && differing == 0,
        "{reference} in random pieces: max |diff| {largest:e}, {differing} samples differ"
    );
}

const PRESSURE4: &str = "dawsome.pressure4";
const DENSITY: &str = "dawsome.density";

const PRESSURE4_A: [(&str, f32); 4] = [
    ("pressure", 0.6),
    ("speed", 0.4),
    ("mewiness", 0.7),
    ("output", 0.9),
];
const DENSITY_A: [(&str, f32); 4] = [
    ("density", 0.6),
    ("highpass", 0.3),
    ("output", 0.8),
    ("mix", 0.7),
];
const DENSITY_HOT: [(&str, f32); 4] = [
    ("density", 1.0),
    ("highpass", 0.0),
    ("output", 1.0),
    ("mix", 1.0),
];

#[test]
fn pressure4_nulls_against_the_original() {
    for rate in ["44k", "48k"] {
        assert_nulls(
            PRESSURE4,
            &PRESSURE4_A,
            &format!("ref/pressure4_a_{rate}.wav"),
        );
        // Every parameter at its default: {0, 0.2, 1, 1}.
        assert_nulls(PRESSURE4, &[], &format!("ref/pressure4_default_{rate}.wav"));
    }
}

#[test]
fn density_nulls_against_the_original() {
    for rate in ["44k", "48k"] {
        assert_nulls(DENSITY, &DENSITY_A, &format!("ref/density_a_{rate}.wav"));
        assert_nulls(
            DENSITY,
            &DENSITY_HOT,
            &format!("ref/density_hot_{rate}.wav"),
        );
    }
}

// Mix exactly 0 returns the input bit for bit while the state still advances, so turning the mix
// back up continues where the effect would be.
#[test]
fn mix_zero_passes_the_input_bit_for_bit() {
    let (input, _) = read_wav("p9_fx_in.wav");
    for (id, params) in [
        (PRESSURE4, vec![("pressure", 0.9), ("mix", 0.0)]),
        (DENSITY, vec![("density", 1.0), ("mix", 0.0)]),
    ] {
        let out = run(
            processor(id, 44_100, &params).as_mut(),
            44_100,
            &input,
            &mut || 333,
        );
        let bits = |frames: &[[f32; 2]]| -> Vec<u32> {
            frames
                .iter()
                .flatten()
                .map(|value| value.to_bits())
                .collect()
        };
        assert_eq!(bits(&out), bits(&input), "{id} at mix 0");
    }
}

// As for every native effect, reset gives back the constructed state, the dither seeds included, so
// a render after a reset is the render of a fresh instance.
#[test]
fn reset_restores_the_constructed_state() {
    let (input, _) = read_wav("p9_fx_in.wav");
    for (id, params) in [(PRESSURE4, &PRESSURE4_A), (DENSITY, &DENSITY_A)] {
        let mut effect = processor(id, 44_100, params);
        let first = run(effect.as_mut(), 44_100, &input, &mut || 512);
        effect.reset();
        let again = run(effect.as_mut(), 44_100, &input, &mut || 512);
        assert!(first == again, "{id}: a reset render differs");
    }
}

#[test]
fn the_slot_menu_effects_are_registered_by_name() {
    let registry = Registry::native();
    let names: Vec<(&str, &str)> = registry
        .factories()
        .map(|factory| (factory.id(), factory.name()))
        .collect();
    assert!(
        names.contains(&(PRESSURE4, "Compressor (Pressure4)")),
        "{names:?}"
    );
    assert!(
        names.contains(&(DENSITY, "Saturator (Density)")),
        "{names:?}"
    );
    let keys = |id: &str| -> Vec<&str> {
        registry
            .get(id)
            .expect("registered")
            .params()
            .iter()
            .map(|info| info.key)
            .collect()
    };
    assert_eq!(
        keys(PRESSURE4),
        ["pressure", "speed", "mewiness", "output", "mix"]
    );
    assert_eq!(keys(DENSITY), ["density", "highpass", "output", "mix"]);
}
