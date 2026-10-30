//! A MIDI-out note whose note on had to wait for the jump frame (Play, Seek) because it was due
//! less than its channel's latency after it keeps its full length: its note off comes its gate
//! after the note on, not its latency before its end (which could leave it a one-frame blip). A
//! rule decided on 2026-10-25 (the generator tests/fixtures/gen_p19_expected.py states it, table
//! B's first off follows it).
//!
//! By hand (44100 Hz, 128 BPM, ppq 96, s(t) = floor(t·60·44100·1000/(128000·96)), 200 ms = 8820
//! frames): key 41 on tick 24, 12 ticks long (s(36) − s(24) = 7751 − 5167 = 2584 frames, 58 ms),
//! is due on 5167, less than 8820 frames after the jump.

use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::Touched;
use dawsome_engine::midi_out::MidiOutEvent;
use dawsome_engine::plan::PlayMode;
use dawsome_engine::project::{ChannelId, Note, PatternId, Project};
use dawsome_engine::realtime;
use dawsome_engine::time::Ticks;
use dawsome_engine::transport::Control;

const SYNTH: ChannelId = ChannelId(1);

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// p19_pattern with its MIDI out 200 ms late and pattern 1 holding key 41 on tick 24, 12 long.
fn short_note_project() -> Project {
    let text = fs::read_to_string(fixtures().join("p19_pattern.json")).expect("reads");
    let mut project = Project::load_json(&text).expect("loads");
    let out = project
        .channels
        .get_mut(&SYNTH)
        .and_then(|channel| channel.midi_out.as_mut())
        .expect("the synth's MIDI out");
    out.latency_ms = 200;
    project
        .patterns
        .get_mut(&PatternId(1))
        .expect("pattern 1")
        .notes = vec![Note {
        channel: SYNTH,
        start: Ticks(24),
        length: Ticks(12),
        velocity: 0.5,
        key: 41,
    }];
    project.validate().expect("valid");
    project
}

fn message(event: &MidiOutEvent) -> (u64, String) {
    let bytes: Vec<String> = event.bytes().iter().map(|b| format!("{b:02X}")).collect();
    (event.frame, bytes.join(" "))
}

// Play on frame 0: on 0, off 0 + 2584 (not 1). A Seek to tick 0 on frame 10000: the note is due on
// 15167, sent on 10000, ended on 12584 (not 10001).
#[test]
fn a_note_delayed_to_the_jump_frame_keeps_its_full_length() {
    let project = short_note_project();
    let mut compiler = Compiler::new();
    let click = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(vec![1.0]));
    let mode = PlayMode::Pattern(PatternId(1));
    let update = compiler
        .compile(&project, &Touched::default(), mode, click)
        .expect("compiles");
    let (mut link, mut player) = realtime::link(update, project.tempo);
    let mut reader = link.take_midi_out().expect("the reader");
    let mut sent = Vec::new();
    let mut buffer = [[0.0f32; 2]; 500];
    link.send(Control::Play).expect("room");
    for call in 0..40 {
        if call == 20 {
            link.send(Control::Seek(Ticks(0))).expect("room");
        }
        player.process(&mut buffer);
        reader.read_into(&mut sent);
    }
    let messages: Vec<(u64, String)> = sent.iter().map(message).collect();
    let want = [
        (0, "90 29 40"),
        (2_584, "80 29 40"),
        (10_000, "90 29 40"),
        (12_584, "80 29 40"),
    ];
    let want: Vec<(u64, String)> = want
        .iter()
        .map(|&(frame, bytes)| (frame, bytes.to_owned()))
        .collect();
    assert_eq!(messages, want);
}
