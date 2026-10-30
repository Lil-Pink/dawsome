//! MIDI out across changes made while the song plays, with a latency (notes sent ahead of their
//! natural frame): a tempo change and a plan that changes the loop or the pattern played send each
//! note once, none twice and none skipped; and a cloned channel does not share its source's MIDI
//! out (cases found 2026-10-25, a pattern switch among them).
//!
//! Expected frames are worked by hand from the rules (s(t) = floor(t·60·sr·1000/(mbpm·ppq)),
//! 44100 Hz, 128 BPM, ppq 96, so 200 ms = 8820 frames): a note is sent its latency before its
//! natural frame but never before the start of the call that finds it; a tempo change while
//! playing starts at the first tick at or after the playhead (here tick 47, sample 10120).

use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, Edit, Touched};
use dawsome_engine::midi_out::{MidiOutEvent, MidiOutReader};
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::project::{ChannelId, LoopRange, Note, Pattern, PatternId, Project};
use dawsome_engine::realtime::{self, LiveNote, Player, RealtimeLink};
use dawsome_engine::time::{MilliBpm, Ticks};
use dawsome_engine::transport::Control;

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

const SYNTH: ChannelId = ChannelId(1);

/// 200 ms at 44100 Hz.
const LATENCY_MS: i32 = 200;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn load(name: &str) -> Project {
    let text = fs::read_to_string(fixtures().join(name)).expect("reads the fixture");
    Project::load_json(&text).unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn json(project: &Project) -> String {
    project.to_json().expect("serializes")
}

/// `project` with the synth's MIDI out `latency_ms` late and pattern 1 holding `notes`.
fn with_notes(mut project: Project, latency_ms: i32, notes: Vec<Note>) -> Project {
    let out = project
        .channels
        .get_mut(&SYNTH)
        .and_then(|channel| channel.midi_out.as_mut())
        .expect("the synth's MIDI out");
    out.latency_ms = latency_ms;
    project
        .patterns
        .get_mut(&PatternId(1))
        .expect("pattern 1")
        .notes = notes;
    project.validate().expect("valid");
    project
}

fn note(tick: i64, length: i64, key: u8) -> Note {
    Note {
        channel: SYNTH,
        start: Ticks(tick),
        length: Ticks(length),
        velocity: 0.5,
        key,
    }
}

/// The late-note layout, from key 40 (keys 40 and 41 are due within the latency of Play, so sent
/// late on frame 0; how long such a note sounds, midi_out_delayed_note.rs pins): key k on
/// tick 24·(k − 40), 12 ticks long, velocity 0.5, keys 42 to 55.
fn ladder(latency_ms: i32) -> Project {
    let notes = (42..=55).map(|key| note(24 * (i64::from(key) - 40), 12, key));
    with_notes(load("p19_pattern.json"), latency_ms, notes.collect())
}

fn compile(compiler: &mut Compiler, project: &Project, mode: PlayMode) -> PlanUpdate {
    let click = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(vec![1.0]));
    compiler
        .compile(project, &Touched::default(), mode, click)
        .expect("compiles")
}

/// A player, its link and its MIDI-out reader, stopped at frame 0.
struct Rig {
    compiler: Compiler,
    link: RealtimeLink,
    player: Player,
    reader: MidiOutReader,
    sent: Vec<MidiOutEvent>,
}

impl Rig {
    fn new(project: &Project, mode: PlayMode) -> Self {
        let mut compiler = Compiler::new();
        let update = compile(&mut compiler, project, mode);
        let (mut link, player) = realtime::link(update, project.tempo);
        let reader = link.take_midi_out().expect("the reader");
        Self {
            compiler,
            link,
            player,
            reader,
            sent: Vec::new(),
        }
    }

    fn send(&mut self, control: Control) {
        self.link.send(control).expect("room");
    }

    /// Renders up to output frame `to` in calls of 512 frames (the last one shorter), reading the
    /// ring after each.
    fn render_to(&mut self, to: u64) {
        let mut buffer = [[0.0f32; 2]; 512];
        let mut done = self.player.output_frames();
        while done < to {
            let len = usize::try_from(to - done).expect("small").min(512);
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

    /// Compiles `project` for `mode` and hands the plan over; taken at the next call's start.
    fn swap_to(&mut self, project: &Project, mode: PlayMode) {
        self.link.check_room().expect("room");
        let update = compile(&mut self.compiler, project, mode);
        self.link.push_plan(update).expect("same time base");
    }

    fn messages(&self) -> Vec<Message> {
        self.sent.iter().map(message).collect()
    }

    /// The note ons sent, in order.
    fn ons(&self) -> Vec<Message> {
        let ons = self
            .sent
            .iter()
            .filter(|event| event.bytes()[0] & 0xF0 == 0x90);
        ons.map(message).collect()
    }
}

/// One message as (frame, rack channel, bytes in hex).
type Message = (u64, u32, String);

fn message(event: &MidiOutEvent) -> Message {
    let bytes: Vec<String> = event.bytes().iter().map(|b| format!("{b:02X}")).collect();
    (event.frame, event.channel.0, bytes.join(" "))
}

fn on(frame: u64, bytes: &str) -> Message {
    (frame, SYNTH.0, bytes.to_owned())
}

/// Every note on is ended by one note off of its key and MIDI channel, on its frame or later and
/// before that key starts again; no note off ends a note that is not sounding.
fn assert_paired(messages: &[Message]) {
    let mut sounding: Vec<(String, u64)> = Vec::new();
    for (frame, _, bytes) in messages {
        let (status, rest) = bytes.split_at(2);
        let key = format!("{}{}", &status[1..], &rest[..3]);
        let held = sounding.iter().position(|(sounding, _)| *sounding == key);
        if status.starts_with('9') {
            assert!(
                held.is_none(),
                "{bytes} on {frame} while it sounds: {messages:?}"
            );
            sounding.push((key, *frame));
        } else {
            let held = held.unwrap_or_else(|| panic!("{bytes} on {frame} ends nothing"));
            assert!(sounding[held].1 <= *frame);
            sounding.remove(held);
        }
    }
    assert!(sounding.is_empty(), "never ended: {sounding:?}");
}

// ---------------------------------------------------------------------------------------------
// A tempo change while playing
// ---------------------------------------------------------------------------------------------

/// The ladder at `latency_ms`, played in pattern mode to frame 10000, the tempo set to `mbpm`
/// there, played on to frame `to`, then stopped.
fn tempo_change(latency_ms: i32, mbpm: u32, to: u64) -> Rig {
    let mut rig = Rig::new(&ladder(latency_ms), PlayMode::Pattern(PatternId(1)));
    rig.send(Control::Play);
    rig.render_to(10_000);
    rig.send(Control::SetTempo(MilliBpm(mbpm)));
    rig.render_to(to);
    rig.stop();
    rig
}

// By hand, 128 → 64 BPM on frame 10000, 200 ms: keys 42 and 43 (ticks 48 and 72, natural frames
// 10335 and 15503) were sent before it, 8820 frames early; every tick before 88 was. At 64 BPM tick
// t ≥ 47 falls at 10120 + floor((t − 47)·430.664), so key 43's tick plays on 20886, but it went out
// already and must not go again; key 44 (tick 96) plays on 31222, sent on 22402, ended on
// 31222 + 5168 − 8820 = 27570; key 45 (tick 120) on 41558 − 8820 = 32738, ended on 37906.
#[test]
fn a_slower_tempo_sends_the_notes_already_sent_ahead_no_second_time() {
    let rig = tempo_change(LATENCY_MS, 64_000, 40_000);
    assert_eq!(
        rig.messages(),
        [
            on(1_515, "90 2A 40"),
            on(4_099, "80 2A 40"),
            on(6_683, "90 2B 40"),
            on(9_267, "80 2B 40"),
            on(22_402, "90 2C 40"),
            on(27_570, "80 2C 40"),
            on(32_738, "90 2D 40"),
            on(37_906, "80 2D 40"),
        ]
    );
}

// By hand, 128 → 256 BPM on frame 10000: tick t ≥ 47 falls at 10120 + floor((t − 47)·107.666), so
// keys 44 and 45 (ticks 96 and 120, natural frames 15395 and 17979 now) are due less than 8820
// frames ahead: sent on 10000, the first frame the new tempo is known (they were not sent before:
// at 128 BPM they were due on 20671 and 25839). Keys 46 to 49 then go 8820 frames ahead of
// 20563, 23147, 25731 and 28315. Each once, each ended.
#[test]
fn a_faster_tempo_sends_the_notes_it_brings_inside_the_latency() {
    let rig = tempo_change(LATENCY_MS, 256_000, 20_000);
    assert_eq!(
        rig.ons(),
        [
            on(1_515, "90 2A 40"),
            on(6_683, "90 2B 40"),
            on(10_000, "90 2C 40"),
            on(10_000, "90 2D 40"),
            on(11_743, "90 2E 40"),
            on(14_327, "90 2F 40"),
            on(16_911, "90 30 40"),
            on(19_495, "90 31 40"),
        ]
    );
    assert_paired(&rig.messages());
}

// The same change with no latency sends every note on its natural frame at the new tempo, as it
// did before tempo changes moved anything: key 42 on 10120 + 430 = 10550, key 43 on 20886, key 44
// on 31222, each 5168 frames long.
#[test]
fn a_tempo_change_with_no_latency_sends_each_note_on_its_tick() {
    let rig = tempo_change(0, 64_000, 40_000);
    assert_eq!(
        rig.messages(),
        [
            on(10_550, "90 2A 40"),
            on(15_718, "80 2A 40"),
            on(20_886, "90 2B 40"),
            on(26_054, "80 2B 40"),
            on(31_222, "90 2C 40"),
            on(36_390, "80 2C 40"),
        ]
    );
}

// With no latency, a note on the very tick the new tempo starts on (tick 47, sample 10120) falls
// inside the first call after the change (10000 to 10512): sent there, on 10120.
#[test]
fn a_note_on_the_tick_a_new_tempo_starts_on_is_sent() {
    let project = with_notes(load("p19_pattern.json"), 0, vec![note(47, 12, 60)]);
    let mut rig = Rig::new(&project, PlayMode::Pattern(PatternId(1)));
    rig.send(Control::Play);
    rig.render_to(10_000);
    rig.send(Control::SetTempo(MilliBpm(64_000)));
    rig.render_to(20_000);
    rig.stop();
    // Its end, tick 59, at 10120 + floor(12·430.664) = 15287.
    assert_eq!(
        rig.messages(),
        [on(10_120, "90 3C 40"), on(15_287, "80 3C 40")]
    );
}

// ---------------------------------------------------------------------------------------------
// A new plan that moves the loop or plays another pattern
// ---------------------------------------------------------------------------------------------

/// p19_song_latency at 200 ms: key 50 on tick 0 (96 long), key 55 on tick 192 (288 long), key 57
/// on tick 336 (96 long), MIDI channel 2, one clip of the 384-tick pattern, 640 ticks long.
fn song_at_200_ms(looped: Option<LoopRange>) -> Project {
    let mut project = load("p19_song_latency.json");
    let out = project
        .channels
        .get_mut(&SYNTH)
        .and_then(|channel| channel.midi_out.as_mut())
        .expect("the synth's MIDI out");
    out.latency_ms = LATENCY_MS;
    project.playlist.loop_range = looped;
    project.validate().expect("valid");
    project
}

fn loop_0_192(on: bool) -> Option<LoopRange> {
    Some(LoopRange {
        start: Ticks(0),
        end: Ticks(192),
        on,
    })
}

// By hand: looping ticks 0 to 192 (41343 frames a round), key 50 of the second round (natural
// 41343) went out on 32523, its off due on 41343 + 20671 − 8820 = 53194. On frame 38000 the loop is
// switched off: the song plays on through tick 192, so key 55 is due on 41343 and must be sent
// (on 38000, the first frame the new plan is known), and key 57 on 72351 − 8820 = 63531, key 50 of
// the clip's second pass (tick 384) on 82687 − 8820 = 73867. Key 50 sent for the round that no
// longer comes still ends when it was due, 53194.
#[test]
fn switching_the_loop_off_sends_the_notes_past_its_end() {
    let looped = song_at_200_ms(loop_0_192(true));
    let mut rig = Rig::new(&looped, PlayMode::Song);
    rig.send(Control::Play);
    rig.render_to(38_000);
    let edited = {
        let mut edited = looped.clone();
        edit::apply(
            &mut edited,
            Edit::SetLoop {
                value: loop_0_192(false),
            },
        )
        .expect("applies");
        edited
    };
    rig.swap_to(&edited, PlayMode::Song);
    rig.render_to(100_000);
    rig.stop();
    assert_eq!(
        rig.ons(),
        [
            on(0, "91 32 60"),
            on(32_523, "91 32 60"),
            on(38_000, "91 37 40"),
            on(63_531, "91 39 40"),
            on(73_867, "91 32 60"),
        ]
    );
    let messages = rig.messages();
    assert!(
        messages.contains(&on(53_194, "81 32 40")),
        "key 50 of the second round ends on time: {messages:?}"
    );
    assert_paired(&messages);
}

// Pattern mode, 200 ms: pattern 1 has key 60 on tick 96 (sent on 20671 − 8820 = 11851); on frame
// 15000 the plan plays pattern 2 instead (key 62 on tick 96, key 64 on tick 192), with no jump.
// Key 62 is due on 20671, less than 8820 frames ahead: sent on 15000. Key 64 on 41343 − 8820 =
// 32523. Key 60, sent before the switch, still ends when it was due (31007 − 8820 = 22187).
#[test]
fn another_pattern_without_a_jump_sends_its_notes_due_within_the_latency() {
    let mut project = with_notes(load("p19_pattern.json"), LATENCY_MS, vec![note(96, 48, 60)]);
    project.patterns.insert(
        PatternId(2),
        Pattern {
            name: "Pattern 2".into(),
            length_bars: 1,
            copied_from: None,
            notes: vec![note(96, 48, 62), note(192, 48, 64)],
        },
    );
    project.validate().expect("valid");
    let mut rig = Rig::new(&project, PlayMode::Pattern(PatternId(1)));
    rig.send(Control::Play);
    rig.render_to(15_000);
    rig.swap_to(&project, PlayMode::Pattern(PatternId(2)));
    rig.render_to(45_000);
    rig.stop();
    assert_eq!(
        rig.ons(),
        [
            on(11_851, "90 3C 40"),
            on(15_000, "90 3E 40"),
            on(32_523, "90 40 40"),
        ]
    );
    let messages = rig.messages();
    assert!(messages.contains(&on(22_187, "80 3C 40")), "{messages:?}");
    assert!(messages.contains(&on(42_859, "80 40 40")), "{messages:?}");
    assert_paired(&messages);
}

// A loop shorter than the latency: ticks 0 to 24 (5167 frames a round at 128 BPM), key 50 on
// tick 12, so notes go out up to two rounds ahead. By hand: on frame 30000 (round 5, tick 19.3)
// the tempo halves from tick 20 on: round 5 ends on 31863, then a round is 10335 frames. Before
// it, round 7's note went out (natural 38752 < 30000 + 8820); at 64 BPM rounds 6 and 7 play their
// key 50 on 37030 and 47365, sent already, so not again; round 8's is due on 57700, sent on 48880,
// round 9's on 68035 − 8820 = 59215. Rounds 6 and 7 went out on 24765 and 29932.
#[test]
fn a_tempo_change_under_a_loop_shorter_than_the_latency_sends_each_round_once() {
    let mut project = song_at_200_ms(Some(LoopRange {
        start: Ticks(0),
        end: Ticks(24),
        on: true,
    }));
    project
        .patterns
        .get_mut(&PatternId(1))
        .expect("pattern 1")
        .notes = vec![note(12, 12, 50)];
    project.validate().expect("valid");
    let mut rig = Rig::new(&project, PlayMode::Song);
    rig.send(Control::Play);
    rig.render_to(30_000);
    rig.send(Control::SetTempo(MilliBpm(64_000)));
    rig.render_to(60_000);
    rig.stop();
    let late: Vec<Message> = rig
        .ons()
        .into_iter()
        .filter(|(frame, _, _)| *frame >= 20_000)
        .collect();
    assert_eq!(
        late,
        [
            on(24_765, "91 32 40"),
            on(29_932, "91 32 40"),
            on(48_880, "91 32 40"),
            on(59_215, "91 32 40"),
        ]
    );
    assert_paired(&rig.messages());
}

// The tempo change and the loop's plan swap work out where notes go without a heap call.
#[test]
fn moving_the_marks_never_touches_the_heap() {
    let looped = song_at_200_ms(loop_0_192(true));
    let mut edited = looped.clone();
    edit::apply(
        &mut edited,
        Edit::SetLoop {
            value: loop_0_192(false),
        },
    )
    .expect("applies");
    let mut rig = Rig::new(&looped, PlayMode::Song);
    let mut buffer = vec![[0.0f32; 2]; 512];
    let mut calls = 0;
    let mut process = |rig: &mut Rig| {
        calls += heap_calls_during(|| rig.player.process(&mut buffer));
    };
    rig.send(Control::Play);
    for _ in 0..70 {
        process(&mut rig);
    }
    rig.send(Control::SetTempo(MilliBpm(64_000)));
    process(&mut rig);
    rig.swap_to(&edited, PlayMode::Song);
    for _ in 0..4 {
        process(&mut rig);
    }
    assert_eq!(calls, 0, "heap calls on the audio path");
}

// ---------------------------------------------------------------------------------------------
// A cloned channel
// ---------------------------------------------------------------------------------------------

// A clone takes no MIDI out, as it takes no MIDI input: its source's port and MIDI channel would
// play every note twice. A live note on the clone sends nothing; the source still sends. The
// clone's undo gives the document back byte for byte.
#[test]
fn a_cloned_channel_sends_nothing_to_its_sources_midi_out() {
    let project = load("p19_pattern.json");
    let clone = project.next_channel_id;
    let before = json(&project);
    let mut cloned = project.clone();
    let applied = edit::apply(
        &mut cloned,
        Edit::CloneChannel {
            id: clone,
            from: SYNTH,
        },
    )
    .expect("clones");
    cloned.validate().expect("valid");
    assert!(
        cloned.channels[&SYNTH].midi_out.is_some(),
        "the source keeps it"
    );
    assert_eq!(cloned.channels[&clone].midi_out, None);
    let mut undone = cloned.clone();
    edit::apply(&mut undone, applied.inverse).expect("undoes");
    assert_eq!(json(&undone), before, "undo gives the bytes back");

    let mut rig = Rig::new(&cloned, PlayMode::Pattern(PatternId(1)));
    let inbox = rig.link.midi_inbox().for_port();
    inbox.push_to(
        clone,
        LiveNote::On {
            key: 62,
            velocity: 0.5,
        },
    );
    rig.render_to(64);
    assert_eq!(rig.messages(), [], "the clone sends nothing");
    inbox.push_to(
        SYNTH,
        LiveNote::On {
            key: 64,
            velocity: 0.5,
        },
    );
    rig.render_to(128);
    assert_eq!(
        rig.messages(),
        [on(64, "90 40 40")],
        "the source still sends"
    );
}
