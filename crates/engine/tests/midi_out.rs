//! MIDI out in the engine: format version 7 (the projects of tests/fixtures/gen_p19_expected.py
//! save back to their own bytes, a project without a MIDI out keeps its old version), what
//! `validate` refuses, the edit that sets an input channel's MIDI out with an exact inverse
//! (taking the channel's MIDI input link with it when the channel would take notes no more), and
//! the player: the known messages through seeded block splits, the note offs every jump and plan
//! swap sends, the reserve that keeps room for note offs, live notes sent or kept quiet, and a
//! path that never touches the heap.

use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, Edit, EditError, Touched};
use dawsome_engine::midi_out::{MidiOutEvent, MidiOutReader};
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::project::{
    ChannelId, ChannelKind, InputChannels, InputSource, LoadError, MidiOut, Note, PatternId,
    Project, Sound, SoundSource, ValidationError, MIDI_OUT_FORMAT_VERSION,
};
use dawsome_engine::realtime::{self, EngineEvent, LiveNote, Player, RealtimeLink};
use dawsome_engine::time::Ticks;
use dawsome_engine::transport::Control;

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn text(name: &str) -> String {
    fs::read_to_string(fixtures().join(name)).expect("reads the fixture")
}

fn load(name: &str) -> Project {
    Project::load_json(&text(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn json(project: &Project) -> String {
    project.to_json().expect("serializes")
}

const P19: [&str; 3] = ["p19_pattern.json", "p19_song_latency.json", "p19_rack.json"];

const SYNTH: ChannelId = ChannelId(1);

fn synth_out() -> MidiOut {
    MidiOut {
        port: "Synth:Synth MIDI 1".into(),
        channel: 1,
        program: None,
        latency_ms: 0,
        echo: false,
    }
}

/// `edit` applied to `project`, checked valid, then undone: the document is back byte for byte.
/// Returns the project as the edit left it.
fn applies_and_undoes(project: &Project, edit: Edit) -> Project {
    let before = json(project);
    let mut edited = project.clone();
    let applied = edit::apply(&mut edited, edit).expect("applies");
    edited.validate().expect("valid after the edit");
    let mut undone = edited.clone();
    edit::apply(&mut undone, applied.inverse).expect("undoes");
    assert_eq!(json(&undone), before, "undo gives the bytes back");
    edited
}

// ---------------------------------------------------------------------------------------------
// The project file
// ---------------------------------------------------------------------------------------------

// The generator writes each project as the file shape states it: loading and saving gives the
// same bytes, version 7 first.
#[test]
fn a_project_with_a_midi_out_saves_back_to_its_own_bytes() {
    assert_eq!(MIDI_OUT_FORMAT_VERSION, 7);
    for name in P19 {
        let text = text(name);
        assert!(text.starts_with(r#"{"format_version":7,"#), "{name}");
        assert_eq!(json(&load(name)), text.trim_end_matches('\n'), "{name}");
    }
    let rack = load("p19_rack.json");
    let channel = &rack.channels[&ChannelId(2)];
    assert_eq!(
        channel.midi_out,
        Some(MidiOut {
            channel: 3,
            ..synth_out()
        })
    );
    assert_eq!(rack.channels[&ChannelId(4)].midi_out, None);
}

// Without a MIDI out the project writes version 6 again (it still has input channels), so a
// project never using one keeps its bytes.
#[test]
fn without_a_midi_out_the_old_version_is_written() {
    let mut project = load("p19_pattern.json");
    edit::apply(
        &mut project,
        Edit::SetChannelMidiOut {
            channel: SYNTH,
            out: None,
        },
    )
    .expect("takes it away");
    assert!(json(&project).starts_with(r#"{"format_version":6,"#));
}

// What reading refuses: a MIDI out on a channel of another kind, and settings out of range or
// missing, each named.
#[test]
fn a_file_with_a_midi_out_where_none_can_be_is_refused() {
    let pattern = text("p19_pattern.json");
    let refused = |text: &str| match Project::load_json(text) {
        Err(LoadError::Invalid(error)) => error,
        other => panic!("not refused as invalid: {other:?}"),
    };
    let sampler = pattern.replacen(
        r#""kind":"input","input":{"device":"USB Audio Line Input","channels":{"stereo":[0,1]}}"#,
        r#""kind":"sampler","sample":"impulse.wav""#,
        1,
    );
    assert!(matches!(
        refused(&sampler),
        ValidationError::MidiOutOnOtherKind { .. }
    ));
    let cases = [
        (r#""port":"","channel":1"#, "port"),
        (r#""port":"Synth:Synth MIDI 1","channel":17"#, "channel"),
        (
            r#""port":"Synth:Synth MIDI 1","channel":1,"program":200"#,
            "program",
        ),
    ];
    for (with, setting) in cases {
        let text = pattern.replacen(r#""port":"Synth:Synth MIDI 1","channel":1"#, with, 1);
        match refused(&text) {
            ValidationError::InvalidMidiOut { setting: named, .. } => {
                assert_eq!(named, setting, "{with}");
            }
            other => panic!("{with}: {other}"),
        }
    }
    let late = pattern.replacen(r#""latency_ms":0"#, r#""latency_ms":250"#, 1);
    assert!(matches!(
        refused(&late),
        ValidationError::InvalidMidiOut {
            setting: "latency_ms",
            ..
        }
    ));
}

// An input channel with a MIDI out takes a MIDI input link: a file with one keeps it (no load
// note), and the port finds the channel. Without the MIDI out the link is left out with a note,
// as before.
#[test]
fn an_input_channel_with_a_midi_out_keeps_its_link() {
    let linked = text("p19_pattern.json").replacen(
        r#""mute":false,"midi_out""#,
        r#""mute":false,"midi_input":"Synth:Synth MIDI 1","midi_out""#,
        1,
    );
    let (project, notes) = Project::load_json_noting(&linked).expect("loads");
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(
        project.channel_for_port("Synth:Synth MIDI 1 24:0"),
        Some(SYNTH)
    );
    assert_eq!(json(&project), linked.trim_end_matches('\n'));
    let unsent = linked.replacen(
        r#","midi_out":{"port":"Synth:Synth MIDI 1","channel":1,"latency_ms":0}"#,
        "",
        1,
    );
    let (project, notes) = Project::load_json_noting(&unsent).expect("loads");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(project.channels[&SYNTH].midi_input, None);
}

// ---------------------------------------------------------------------------------------------
// The edit
// ---------------------------------------------------------------------------------------------

// A MIDI out given, changed and taken away, each undone byte for byte; each touches the
// channel, whose plan carries the MIDI channel and the latency.
#[test]
fn a_midi_out_is_set_changed_and_taken_away_with_exact_inverses() {
    let project = load("p19_rack.json");
    let line_in = ChannelId(4);
    let given = applies_and_undoes(
        &project,
        Edit::SetChannelMidiOut {
            channel: line_in,
            out: Some(synth_out()),
        },
    );
    assert_eq!(given.channels[&line_in].midi_out, Some(synth_out()));
    let changed = MidiOut {
        channel: 10,
        program: Some(4),
        latency_ms: 32,
        echo: true,
        ..synth_out()
    };
    let edited = applies_and_undoes(
        &project,
        Edit::SetChannelMidiOut {
            channel: SYNTH,
            out: Some(changed.clone()),
        },
    );
    assert_eq!(edited.channels[&SYNTH].midi_out, Some(changed));
    let gone = applies_and_undoes(
        &project,
        Edit::SetChannelMidiOut {
            channel: SYNTH,
            out: None,
        },
    );
    assert_eq!(gone.channels[&SYNTH].midi_out, None);
    let mut touched = project.clone();
    let applied = edit::apply(
        &mut touched,
        Edit::SetChannelMidiOut {
            channel: SYNTH,
            out: None,
        },
    )
    .expect("applies");
    assert!(applied.touched.channels.contains(&SYNTH));
}

// Refused, changing nothing: a MIDI out on a sampler, a setting out of range, a channel that is
// not there.
#[test]
fn a_midi_out_where_none_can_be_is_refused() {
    let project = load("p19_rack.json");
    let refused = |channel: ChannelId, out: MidiOut| {
        let mut tried = project.clone();
        let error = edit::apply(
            &mut tried,
            Edit::SetChannelMidiOut {
                channel,
                out: Some(out),
            },
        )
        .expect_err("refused");
        assert_eq!(json(&tried), json(&project), "nothing changed");
        error
    };
    assert_eq!(refused(ChannelId(5), synth_out()), EditError::NoMidiOut(5));
    assert_eq!(
        refused(
            SYNTH,
            MidiOut {
                latency_ms: 201,
                ..synth_out()
            }
        ),
        EditError::InvalidMidiOut {
            channel: 1,
            setting: "latency_ms"
        }
    );
    assert_eq!(
        refused(ChannelId(9), synth_out()),
        EditError::UnknownChannel(9)
    );
}

/// `name`'s project with its synth linked to the MIDI input "Keys".
fn linked(name: &str) -> Project {
    let mut project = load(name);
    edit::apply(
        &mut project,
        Edit::SetChannelMidiInput {
            channel: SYNTH,
            port: Some("Keys".into()),
        },
    )
    .expect("an input channel with a MIDI out takes a link");
    project
}

// An input channel takes a link only while it has a MIDI out: taking the MIDI out away takes
// the link with it in one step, and undo gives both back.
#[test]
fn a_linked_channel_losing_its_midi_out_lets_its_port_go_in_the_same_step() {
    let mut line_in = load("p19_rack.json");
    let error = edit::apply(
        &mut line_in,
        Edit::SetChannelMidiInput {
            channel: ChannelId(4),
            port: Some("Keys".into()),
        },
    )
    .expect_err("no MIDI out, no link");
    assert_eq!(error, EditError::PlaysNoNotes(4));
    let project = linked("p19_pattern.json");
    let gone = applies_and_undoes(
        &project,
        Edit::SetChannelMidiOut {
            channel: SYNTH,
            out: None,
        },
    );
    let channel = &gone.channels[&SYNTH];
    assert_eq!((&channel.midi_out, &channel.midi_input), (&None, &None));
}

// Given a sound of another kind, an input channel lets its MIDI out (and so its link) go in the
// same step; given another input, it keeps both. Undo gives everything back byte for byte.
#[test]
fn a_new_sound_keeps_the_midi_out_only_on_an_input() {
    let project = linked("p19_pattern.json");
    let sampler = applies_and_undoes(
        &project,
        Edit::ReplaceChannelSound {
            channel: SYNTH,
            name: "hit".into(),
            source: Box::new(SoundSource::sampler("impulse.wav")),
            sound: Sound::default(),
        },
    );
    let channel = &sampler.channels[&SYNTH];
    assert_eq!(channel.kind, ChannelKind::Sampler);
    assert_eq!((&channel.midi_out, &channel.midi_input), (&None, &None));
    let other_input = InputSource {
        device: "Another Input".into(),
        channels: InputChannels::Mono(1),
    };
    let input = applies_and_undoes(
        &project,
        Edit::ReplaceChannelSound {
            channel: SYNTH,
            name: "Synth".into(),
            source: Box::new(SoundSource::input(other_input.clone())),
            sound: Sound::default(),
        },
    );
    let channel = &input.channels[&SYNTH];
    assert_eq!(channel.input, Some(other_input));
    assert_eq!(channel.midi_out, Some(synth_out()));
    assert_eq!(channel.midi_input.as_deref(), Some("Keys"));
}

// ---------------------------------------------------------------------------------------------
// The player
// ---------------------------------------------------------------------------------------------

/// `project` compiled for `mode` by `compiler`, every sample a one-frame click.
fn compile(
    compiler: &mut Compiler,
    project: &Project,
    touched: &Touched,
    mode: PlayMode,
) -> PlanUpdate {
    let click = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(vec![1.0]));
    compiler
        .compile(project, touched, mode, click)
        .expect("compiles")
}

/// A player of `project` in `mode`, its link and its MIDI-out reader, stopped at frame 0.
struct Rig {
    compiler: Compiler,
    mode: PlayMode,
    link: RealtimeLink,
    player: Player,
    reader: MidiOutReader,
    sent: Vec<MidiOutEvent>,
}

impl Rig {
    fn new(project: &Project, song: bool) -> Self {
        let mode = if song {
            PlayMode::Song
        } else {
            PlayMode::Pattern(PatternId(1))
        };
        let mut compiler = Compiler::new();
        let update = compile(&mut compiler, project, &Touched::default(), mode);
        let (mut link, player) = realtime::link(update, project.tempo);
        let reader = link.take_midi_out().expect("the reader, once");
        assert!(link.take_midi_out().is_none(), "taken once");
        Self {
            compiler,
            mode,
            link,
            player,
            reader,
            sent: Vec::new(),
        }
    }

    fn send(&mut self, control: Control) {
        self.link.send(control).expect("room");
    }

    /// Renders `frames` frames in calls of the sizes `sizes` gives, reading the ring after each.
    fn render(&mut self, frames: u64, sizes: &mut dyn Iterator<Item = usize>) {
        let mut buffer = vec![[0.0f32; 2]; 5000];
        let mut done = 0;
        while done < frames {
            let left = usize::try_from(frames - done).expect("small");
            let len = sizes.next().expect("sizes").min(left).min(buffer.len());
            self.player.process(&mut buffer[..len]);
            self.reader.read_into(&mut self.sent);
            done += len as u64;
        }
    }

    /// Stop, taken by one more call, as the command line's log ends.
    fn stop(&mut self) {
        self.send(Control::Stop);
        self.player.process(&mut []);
        self.reader.read_into(&mut self.sent);
    }

    /// Compiles `project` (after an edit that touched `touched`) and hands the plan over.
    fn swap_to(&mut self, project: &Project, touched: &Touched) {
        self.link.check_room().expect("room");
        let update = compile(&mut self.compiler, project, touched, self.mode);
        self.link.push_plan(update).expect("same time base");
    }

    /// Every message sent so far as (frame, rack channel, bytes in hex).
    fn messages(&self) -> Vec<Message> {
        self.sent.iter().map(message).collect()
    }
}

fn message(event: &MidiOutEvent) -> Message {
    let bytes: Vec<String> = event.bytes().iter().map(|b| format!("{b:02X}")).collect();
    (event.frame, event.channel.0, bytes.join(" "))
}

/// One message as (frame, rack channel, bytes in hex).
type Message = (u64, u32, String);

/// One case of p19_expected.json.
struct Case {
    name: String,
    project: Project,
    song: bool,
    frames: u64,
    events: Vec<Message>,
}

fn expected_cases() -> Vec<Case> {
    let expected: serde_json::Value =
        serde_json::from_str(&text("p19_expected.json")).expect("parses");
    let cases = expected["cases"].as_object().expect("cases");
    cases
        .iter()
        .map(|(name, case)| {
            let events = case["events"]
                .as_array()
                .expect("events")
                .iter()
                .map(|event| {
                    (
                        event["frame"].as_u64().expect("frame"),
                        u32::try_from(event["channel"].as_u64().expect("channel")).expect("id"),
                        event["bytes"].as_str().expect("bytes").to_owned(),
                    )
                })
                .collect();
            Case {
                name: name.clone(),
                project: load(case["fixture"].as_str().expect("fixture")),
                song: case["song"].as_bool().expect("song"),
                frames: case["frames"].as_u64().expect("frames"),
                events,
            }
        })
        .collect()
}

/// Block sizes: `fixed` every call, or seeded random sizes from 1 to 5000 (xorshift64, so a
/// failure reproduces exactly).
fn sizes(fixed: Option<usize>, seed: u64) -> Box<dyn Iterator<Item = usize>> {
    if let Some(size) = fixed {
        return Box::new(std::iter::repeat(size));
    }
    let mut state = seed;
    Box::new(std::iter::repeat_with(move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        usize::try_from(state % 5000).expect("below 5000") + 1
    }))
}

// Each case's messages, the generator's, through calls of 1, 7, 64 and 4096 frames and three
// seeded random splits: the same messages on the same frames every time.
#[test]
fn the_known_messages_come_through_any_block_split() {
    for Case {
        name,
        project,
        song,
        frames,
        events: want,
    } in expected_cases()
    {
        let splits = [Some(1), Some(7), Some(64), Some(4096), None, None, None];
        for (seed, fixed) in (1u64..).zip(splits) {
            let mut rig = Rig::new(&project, song);
            rig.send(Control::Play);
            rig.render(frames, &mut sizes(fixed, seed * 0x9E37_79B9));
            rig.stop();
            assert_eq!(rig.messages(), want, "{name}, split {fixed:?} seed {seed}");
            assert_eq!((rig.reader.dropped(), rig.reader.lost_offs()), (0, 0));
        }
    }
}

fn on(frame: u64, bytes: &str) -> Message {
    (frame, 1, bytes.to_owned())
}

// By hand, pattern A: a Seek on frame 10000 ends the held note 60 there, and play starts again
// from tick 0 on that frame, so note 60 is sent again there and the chord of tick 96 on
// 10000 + s(96) = 30671.
#[test]
fn a_seek_ends_every_sounding_note_and_sends_from_the_new_place() {
    let mut rig = Rig::new(&load("p19_pattern.json"), false);
    rig.send(Control::Play);
    rig.render(10_000, &mut sizes(Some(512), 0));
    rig.send(Control::Seek(Ticks(0)));
    rig.render(30_000, &mut sizes(Some(333), 0));
    assert_eq!(
        rig.messages(),
        [
            on(0, "90 3C 64"),
            on(10_000, "80 3C 40"),
            on(10_000, "90 3C 64"),
            on(30_671, "90 40 40"),
            on(30_671, "90 43 7F"),
        ]
    );
}

// By hand, pattern A swapped on frame 10000: a plan that only changes a volume goes on; one that
// moves the MIDI out to MIDI channel 2 ends note 60 on channel 1 there and sends the chord of
// tick 96 on channel 2 (s(96) = 20671, its end s(144) = 31007); one that takes the MIDI out away
// ends note 60 there and sends nothing more.
#[test]
fn a_new_plan_ends_the_notes_of_a_midi_out_it_changed_or_took_away() {
    let project = load("p19_pattern.json");
    let swapped = |edit: Edit| {
        let mut edited = project.clone();
        let touched = edit::apply(&mut edited, edit).expect("applies").touched;
        let mut rig = Rig::new(&project, false);
        rig.send(Control::Play);
        rig.render(10_000, &mut sizes(Some(512), 0));
        rig.swap_to(&edited, &touched);
        rig.render(30_000, &mut sizes(Some(512), 0));
        rig.messages()
    };
    let louder = swapped(Edit::SetChannelVolume {
        channel: SYNTH,
        value: 0.5,
    });
    assert_eq!(
        louder,
        [
            on(0, "90 3C 64"),
            on(20_671, "90 40 40"),
            on(20_671, "90 43 7F"),
            on(31_007, "80 40 40"),
            on(31_007, "80 43 40"),
        ]
    );
    let moved = swapped(Edit::SetChannelMidiOut {
        channel: SYNTH,
        out: Some(MidiOut {
            channel: 2,
            ..synth_out()
        }),
    });
    assert_eq!(
        moved,
        [
            on(0, "90 3C 64"),
            on(10_000, "80 3C 40"),
            on(20_671, "91 40 40"),
            on(20_671, "91 43 7F"),
            on(31_007, "81 40 40"),
            on(31_007, "81 43 40"),
        ]
    );
    let gone = swapped(Edit::SetChannelMidiOut {
        channel: SYNTH,
        out: None,
    });
    assert_eq!(gone, [on(0, "90 3C 64"), on(10_000, "80 3C 40")]);
}

// A reader that never reads: once the ring is down to its reserve, note ons are dropped and
// counted, but every note on already sent gets its note off, none lost, Stop's included.
#[test]
fn the_ring_keeps_room_for_the_note_offs_of_the_notes_it_sent() {
    let mut project = load("p19_pattern.json");
    // Ten keys on every tick, one tick long: twenty messages a tick.
    let notes = (0..384)
        .flat_map(|tick| {
            (60..70).map(move |key| Note {
                channel: SYNTH,
                start: Ticks(tick),
                length: Ticks(1),
                velocity: 0.5,
                key,
            })
        })
        .collect();
    project
        .patterns
        .get_mut(&PatternId(1))
        .expect("pattern 1")
        .notes = notes;
    project.validate().expect("valid");
    let mut rig = Rig::new(&project, false);
    rig.send(Control::Play);
    let mut buffer = vec![[0.0f32; 2]; 512];
    for _ in 0..(2 * 82_688 / 512) {
        rig.player.process(&mut buffer);
    }
    rig.send(Control::Stop);
    rig.player.process(&mut []);
    rig.reader.read_into(&mut rig.sent);
    assert!(rig.reader.dropped() > 0, "the ring filled");
    assert_eq!(rig.reader.lost_offs(), 0);
    let ons = rig
        .sent
        .iter()
        .filter(|event| event.bytes()[0] == 0x90)
        .count();
    let offs = rig
        .sent
        .iter()
        .filter(|event| event.bytes()[0] == 0x80)
        .count();
    assert!(ons > 1000, "{ons}");
    assert_eq!(ons, offs, "every note on sent was ended");
    assert!(rig
        .sent
        .windows(2)
        .all(|pair| pair[0].frame <= pair[1].frame));
}

// Live notes go to the channel's MIDI out on the call's first frame when handed in to be sent,
// and stay home when handed in quietly (a port whose instrument hears its own keys); both play
// and are recorded. A muted channel sends none.
#[test]
fn live_notes_go_out_unless_handed_in_quietly() {
    let mut rig = Rig::new(&load("p19_rack.json"), false);
    let inbox = rig.link.midi_inbox().for_port();
    let call = |rig: &mut Rig| rig.render(64, &mut sizes(Some(64), 0));
    inbox.push_to(
        SYNTH,
        LiveNote::On {
            key: 50,
            velocity: 1.0,
        },
    );
    call(&mut rig);
    inbox.push_quiet(
        Some(SYNTH),
        LiveNote::On {
            key: 52,
            velocity: 1.0,
        },
    );
    inbox.push_to(
        ChannelId(3),
        LiveNote::On {
            key: 54,
            velocity: 1.0,
        },
    );
    call(&mut rig);
    inbox.push(LiveNote::Off { key: 50 });
    inbox.push(LiveNote::Off { key: 52 });
    inbox.push(LiveNote::Off { key: 54 });
    call(&mut rig);
    assert_eq!(rig.messages(), [on(0, "90 32 7F"), on(128, "80 32 40")]);
    let started: Vec<u8> = rig
        .link
        .take_events()
        .into_iter()
        .filter_map(|event| match event {
            EngineEvent::LiveNoteStarted { key, .. } => Some(key),
            _ => None,
        })
        .collect();
    assert_eq!(started, [50, 52, 54], "every one is heard and recorded");
}

// Notes, live notes, Stop and a plan swap: not one heap call from `process`.
#[test]
fn the_midi_out_path_never_touches_the_heap() {
    let project = load("p19_pattern.json");
    let mut rig = Rig::new(&project, false);
    let inbox = rig.link.midi_inbox();
    let mut edited = project.clone();
    let touched = edit::apply(
        &mut edited,
        Edit::SetChannelMidiOut {
            channel: SYNTH,
            out: Some(MidiOut {
                channel: 2,
                ..synth_out()
            }),
        },
    )
    .expect("applies")
    .touched;
    let mut buffer = vec![[0.0f32; 2]; 512];
    let mut calls = 0;
    let mut process = |rig: &mut Rig| {
        calls += heap_calls_during(|| rig.player.process(&mut buffer));
    };
    rig.send(Control::Play);
    for _ in 0..80 {
        process(&mut rig);
    }
    inbox.push_to(
        SYNTH,
        LiveNote::On {
            key: 50,
            velocity: 1.0,
        },
    );
    process(&mut rig);
    rig.swap_to(&edited, &touched);
    process(&mut rig);
    inbox.push_to(SYNTH, LiveNote::Off { key: 50 });
    rig.send(Control::Stop);
    process(&mut rig);
    assert_eq!(calls, 0, "heap calls on the audio path");
    rig.reader.read_into(&mut rig.sent);
    let messages = rig.messages();
    let live = 80 * 512;
    for want in [
        on(0, "90 3C 64"),
        on(live, "90 32 7F"),
        on(live + 512, "80 3C 40"),
        on(live + 1024, "80 32 40"),
    ] {
        assert!(messages.contains(&want), "{want:?} in {messages:?}");
    }
}
