//! MIDI out across a plan swapped away and back within the latency (notes sent ahead of their
//! natural frame): a note sent before the swaps and still sounding is struck once, not ended and
//! struck again where the plan came back (found 2026-10-25).
//!
//! Expected frames are worked by hand from the rules (s(t) = floor(t·60·sr·1000/(mbpm·ppq)),
//! 44100 Hz, 128 BPM, ppq 96, so 200 ms = 8820 frames): a note is sent its latency before its
//! natural frame but never before the start of the call that finds it, and ends its latency before
//! its natural end; one whose note on waited for the call's start keeps its whole length.

use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, Edit, Touched};
use dawsome_engine::midi_out::{MidiOutEvent, MidiOutReader};
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::project::{ChannelId, LoopRange, Note, Pattern, PatternId, Project};
use dawsome_engine::realtime::{self, Player, RealtimeLink};
use dawsome_engine::time::Ticks;
use dawsome_engine::transport::Control;

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

/// The synth's MIDI out of `project`, `latency_ms` late.
fn with_latency(mut project: Project, latency_ms: i32) -> Project {
    let out = project
        .channels
        .get_mut(&SYNTH)
        .and_then(|channel| channel.midi_out.as_mut())
        .expect("the synth's MIDI out");
    out.latency_ms = latency_ms;
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
// Another pattern and back
// ---------------------------------------------------------------------------------------------

/// p19_pattern at `latency_ms`, pattern 1 holding key 60 on tick 96 (48 long), pattern 2 key 62
/// on tick 96 and key 64 on tick 192: pattern 1 played to frame 12138, pattern 2 from there to
/// 12650, pattern 1 again from there to 45000, with no jump, then stopped.
fn pattern_and_back(latency_ms: i32) -> Rig {
    let mut project = with_latency(load("p19_pattern.json"), latency_ms);
    project
        .patterns
        .get_mut(&PatternId(1))
        .expect("pattern 1")
        .notes = vec![note(96, 48, 60)];
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
    rig.render_to(12_138);
    rig.swap_to(&project, PlayMode::Pattern(PatternId(2)));
    rig.render_to(12_650);
    rig.swap_to(&project, PlayMode::Pattern(PatternId(1)));
    rig.render_to(45_000);
    rig.stop();
    rig
}

// By hand, 200 ms: key 60 (tick 96, natural frame 20671, 10336 frames long) goes out on
// 20671 − 8820 = 11851 and ends on 20671 + 10336 − 8820 = 22187. Pattern 2's key 62, due on 20671
// too, goes out on 12138, the first frame its plan is known, and keeps its length: ended on
// 12138 + 10336 = 22474. Back on pattern 1 at 12650, key 60 sounds already: no note off and no
// second note on there, it ends when it was due.
#[test]
fn another_pattern_and_back_within_the_latency_strikes_the_note_once() {
    let rig = pattern_and_back(LATENCY_MS);
    assert_eq!(
        rig.messages(),
        [
            on(11_851, "90 3C 40"),
            on(12_138, "90 3E 40"),
            on(22_187, "80 3C 40"),
            on(22_474, "80 3E 40"),
        ]
    );
}

// The same swaps with no latency: nothing went out before them, and key 60 plays on its natural
// frame, 20671, to 31007.
#[test]
fn another_pattern_and_back_with_no_latency_plays_the_note_on_its_tick() {
    let rig = pattern_and_back(0);
    assert_eq!(
        rig.messages(),
        [on(20_671, "90 3C 40"), on(31_007, "80 3C 40")]
    );
}

// ---------------------------------------------------------------------------------------------
// The loop off and on again
// ---------------------------------------------------------------------------------------------

fn loop_0_192(on: bool) -> Option<LoopRange> {
    Some(LoopRange {
        start: Ticks(0),
        end: Ticks(192),
        on,
    })
}

/// p19_song_latency at `latency_ms` (key 50 on tick 0, 96 long, velocity 0x60; key 55 on tick
/// 192; key 57 on tick 336; MIDI channel 2), looping ticks 0 to 192: played to frame 36000, the
/// loop switched off there, on again after one frame, played on to 100000, then stopped.
fn loop_off_and_on(latency_ms: i32) -> Rig {
    let mut looped = with_latency(load("p19_song_latency.json"), latency_ms);
    looped.playlist.loop_range = loop_0_192(true);
    looped.validate().expect("valid");
    let mut unlooped = looped.clone();
    edit::apply(
        &mut unlooped,
        Edit::SetLoop {
            value: loop_0_192(false),
        },
    )
    .expect("applies");
    let mut rig = Rig::new(&looped, PlayMode::Song);
    rig.send(Control::Play);
    rig.render_to(36_000);
    rig.swap_to(&unlooped, PlayMode::Song);
    rig.render_to(36_001);
    rig.swap_to(&looped, PlayMode::Song);
    rig.render_to(100_000);
    rig.stop();
    rig
}

// By hand, 200 ms: a round of ticks 0 to 192 is 41343 frames, key 50 20671 frames long. The first
// round's key 50 waits for Play (frame 0) and keeps its length, to 20671. The second round's
// (natural 41343) goes out on 32523 and ends on 41343 + 20671 − 8820 = 53194. The loop off on
// 36000 sends key 55 (tick 192, natural 41343, s(480) − s(192) = 62016 frames long) there, at
// once, so it keeps its length: ended on 36000 + 62016 = 98016. Back on at 36001, key 50 of the
// second round sounds already: nothing on 36001, struck once, ended once on 53194. The third
// round's goes out on 82686 − 8820 = 73866 and ends on 82686 + 20671 − 8820 = 94537.
#[test]
fn the_loop_off_and_on_within_the_latency_strikes_its_round_once() {
    let rig = loop_off_and_on(LATENCY_MS);
    let messages = rig.messages();
    assert_eq!(
        messages,
        [
            on(0, "91 32 60"),
            on(20_671, "81 32 40"),
            on(32_523, "91 32 60"),
            on(36_000, "91 37 40"),
            on(53_194, "81 32 40"),
            on(73_866, "91 32 60"),
            on(94_537, "81 32 40"),
            on(98_016, "81 37 40"),
        ]
    );
    assert_paired(&messages);
}

// The same switches with no latency: key 50 plays each round on its natural frame, 0, 41343 and
// 82686, and key 55 never (the playhead never reached tick 192 while the loop was off).
#[test]
fn the_loop_off_and_on_with_no_latency_plays_each_round_on_its_tick() {
    let rig = loop_off_and_on(0);
    assert_eq!(
        rig.ons(),
        [
            on(0, "91 32 60"),
            on(41_343, "91 32 60"),
            on(82_686, "91 32 60"),
        ]
    );
    assert_paired(&rig.messages());
}
