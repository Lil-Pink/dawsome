//! The piano roll's player as the transport plays it (`PlayMode::Channel`): one channel's notes
//! of one pattern, looped or played once. Played once, the transport stops by itself where the
//! pattern's pass ends, exactly there however a render is split into blocks, as Stop stops it
//! (nothing rings on), and without touching the heap; looped, it plays on. The figures (the
//! pattern's frames a pass, the last note's frame) come from tests/fixtures/gen_roll_player.py,
//! never from the engine. Changed late in a pass played once (Loop switched on, the pattern
//! grown), a MIDI out still sends what the pass then plays on to.

use std::convert::Infallible;

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::Touched;
use dawsome_engine::midi_out::MidiOutEvent;
use dawsome_engine::plan::PlayMode;
use dawsome_engine::project::{ChannelId, Note, PatternId, Project};
use dawsome_engine::realtime::{self, Player, RealtimeLink};
use dawsome_engine::time::Ticks;
use dawsome_engine::transport::Control;
use serde_json::Value;

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

const PROJECT: &str = include_str!("../../../tests/fixtures/rp_keys_drums.json");
const EXPECTED: &str = include_str!("../../../tests/fixtures/rp_expected.json");

/// A sound longer than the generator's tone (100 frames): the pattern's last note, on its last
/// tick, rings on past the pattern's end with it, so what happens there is heard.
const RINGING: usize = 1000;

/// The generator's figures for the player: (pattern, channel, the pattern's frames a pass, the
/// last note's frame).
fn figures() -> (PatternId, ChannelId, usize, usize) {
    let want: Value = serde_json::from_str(EXPECTED).expect("rp_expected.json parses");
    let player = &want["player"];
    let number = |key: &str| player[key].as_u64().unwrap_or_else(|| panic!("{key}"));
    (
        PatternId(u32::try_from(number("pattern_id")).expect("an id")),
        ChannelId(u32::try_from(number("channel_id")).expect("an id")),
        usize::try_from(number("pattern_frames")).expect("frames"),
        usize::try_from(number("last_note_frame")).expect("a frame"),
    )
}

/// A player of the roll's channel of its pattern, once or looped, every sound [`RINGING`]
/// frames of 0.25, told to play from tick 0, and the UI half of its link.
fn playing(once: bool) -> (RealtimeLink, Player) {
    let project = Project::load_json(PROJECT).expect("rp_keys_drums loads");
    let (pattern, channel, _, _) = figures();
    let ringing = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(vec![0.25; RINGING]));
    let mode = PlayMode::Channel {
        pattern,
        channel,
        once,
    };
    let plan = Compiler::new()
        .compile(&project, &Touched::default(), mode, ringing)
        .expect("in-memory sounds load");
    let (mut link, player) = realtime::link(plan, project.tempo);
    link.send(Control::Play)
        .expect("the control ring starts empty");
    (link, player)
}

/// `frames` frames from `player`, `block` frames a call (fewer for the last).
fn render(player: &mut Player, frames: usize, block: usize) -> Vec<[f32; 2]> {
    let mut out = vec![[0.0; 2]; frames];
    for piece in out.chunks_mut(block) {
        player.process(piece);
    }
    out
}

// Once: playing after one frame short of the pass, stopped after the pass, the playhead back
// on the play-from tick (0); in the frames after it nothing sounds, though the last note's
// sound would ring on 750 frames past the end (it starts 250 frames before it). Looped, the
// transport plays on and that ring is heard over the next pass's first note.
#[test]
fn played_once_the_transport_stops_where_the_pass_ends() {
    let (_, _, pass, last_note) = figures();
    let rings_past = last_note + RINGING - pass;
    assert_eq!(rings_past, 750, "the generator's figures");

    let (link, mut player) = playing(true);
    render(&mut player, pass - 1, 512);
    assert!(link.playing(), "one frame short of the pass end it plays");
    render(&mut player, 1, 512);
    assert!(!link.playing(), "stopped on the pass's end");
    assert_eq!(
        link.position().ticks,
        Ticks(0),
        "back on the play-from tick"
    );
    let after = render(&mut player, 2 * rings_past, 512);
    assert!(
        after.iter().all(|frame| *frame == [0.0; 2]),
        "nothing rings on after the stop"
    );

    let (link, mut player) = playing(false);
    let looped = render(&mut player, pass + rings_past, 512);
    assert!(link.playing(), "looped, it plays on");
    assert_eq!(looped[last_note], [0.25; 2], "the last note");
    assert_eq!(
        looped[pass], [0.5; 2],
        "the next pass's first note over the last note's ring"
    );
}

// The stop is a frame of the timeline, not of a call: one-frame, prime and 4096-frame calls
// stop on the same frame and put out the same bits, two passes' worth of frames long.
#[test]
fn the_stop_lands_on_the_same_frame_however_a_render_is_split() {
    let (_, _, pass, _) = figures();
    let frames = pass + 3000;
    let (_, mut player) = playing(true);
    let whole = render(&mut player, frames, frames);
    for block in [1, 509, 4096] {
        let (link, mut player) = playing(true);
        let split = render(&mut player, frames, block);
        assert!(!link.playing(), "block {block}: stopped");
        let bits = |frames: &[[f32; 2]]| -> Vec<[u32; 2]> {
            frames.iter().map(|f| f.map(f32::to_bits)).collect()
        };
        assert_eq!(bits(&split), bits(&whole), "block {block}");
    }
}

// The stop at the pass's end is the audio thread's own: Play, the pass and the stop in calls
// that never touch the heap.
#[test]
fn stopping_at_the_pass_end_never_touches_the_heap() {
    let control = heap_calls_during(|| drop(std::hint::black_box(vec![0u8; 64])));
    assert!(control > 0, "the counting allocator is not counting");
    let (_, _, pass, _) = figures();
    let (link, mut player) = playing(true);
    let mut out = vec![[0.0; 2]; 512];
    let mut calls = 0;
    for _ in 0..pass.div_ceil(512) + 4 {
        calls += heap_calls_during(|| player.process(&mut out));
    }
    assert!(!link.playing(), "the pass ended");
    assert_eq!(calls, 0, "Player::process made {calls} heap calls");
}

const MIDI_OUT: &str = include_str!("../../../tests/fixtures/rp_midi_out.json");

/// One MIDI-out message as the generator writes it: (frame, rack channel, bytes in hex).
fn message(event: &MidiOutEvent) -> (u64, u64, String) {
    let bytes: Vec<String> = event.bytes().iter().map(|b| format!("{b:02X}")).collect();
    (event.frame, u64::from(event.channel.0), bytes.join(" "))
}

fn messages(want: &Value) -> Vec<(u64, u64, String)> {
    let events = want.as_array().expect("events").iter();
    events
        .map(|event| {
            (
                event["frame"].as_u64().expect("frame"),
                event["channel"].as_u64().expect("channel"),
                event["bytes"].as_str().expect("bytes").to_owned(),
            )
        })
        .collect()
}

// rp_midi_out (one bar, 96000 frames a pass; the synth's MIDI out 480 frames early), its synth
// played once in 512-frame calls; in the call that starts on frame 95744 (inside the pass's
// last 480 frames and a call) the plan changes: Loop switched on, or the pattern grown to two
// bars with key 67 on tick 384, once kept. Either way the note the pass now plays on to (the
// next pass's downbeat, bar 2's key 67) goes out on 95744 and ends its whole gate later,
// 119744: the generator's frames, read from the ring after every call up to frame 120320. (Found
// in review: a pass played once had already counted those frames as sent, so the note never
// went out.)
#[test]
fn changed_late_in_a_pass_played_once_a_midi_out_sends_what_plays_on() {
    let want: Value = serde_json::from_str(EXPECTED).expect("rp_expected.json parses");
    let late = &want["midi_out"]["late"];
    let frame = |key: &str| late[key].as_u64().unwrap_or_else(|| panic!("{key}"));
    let (swap, until) = (frame("swap_frame"), frame("until_frame"));
    let synth = ChannelId(1);
    let once = |once| PlayMode::Channel {
        pattern: PatternId(1),
        channel: synth,
        once,
    };
    let silent = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(vec![0.0]));
    for case in ["loop_on", "grown"] {
        let mut project = Project::load_json(MIDI_OUT).expect("rp_midi_out loads");
        let mut compiler = Compiler::new();
        let first = compiler
            .compile(&project, &Touched::default(), once(true), silent)
            .expect("compiles");
        let (mut link, mut player) = realtime::link(first, project.tempo);
        let mut reader = link.take_midi_out().expect("the reader");
        link.send(Control::Play).expect("room");
        let mut sent = Vec::new();
        let mut out = [[0.0f32; 2]; 512];
        let mut done = 0;
        while done < until {
            if done == swap {
                let (mode, touched) = if case == "loop_on" {
                    (once(false), Touched::default())
                } else {
                    let grown = &late["grown_note"];
                    let number = |index: usize| grown[index].as_f64().expect("a number");
                    let pattern = project.patterns.get_mut(&PatternId(1)).expect("pattern 1");
                    pattern.length_bars = 2;
                    pattern.notes.push(Note {
                        channel: synth,
                        start: Ticks(number(0) as i64),
                        length: Ticks(number(1) as i64),
                        velocity: number(2) as f32,
                        key: number(3) as u8,
                    });
                    project.validate().expect("valid");
                    let touched = Touched {
                        patterns: [PatternId(1)].into(),
                        ..Touched::default()
                    };
                    (once(true), touched)
                };
                link.check_room().expect("room");
                let update = compiler
                    .compile(&project, &touched, mode, silent)
                    .expect("compiles");
                link.push_plan(update).expect("same time base");
            }
            player.process(&mut out);
            reader.read_into(&mut sent);
            done += 512;
        }
        let got: Vec<_> = sent.iter().map(message).collect();
        assert_eq!(got, messages(&late[case]), "{case}");
    }
}
