//! Stop ends every live note: a key held on a plugin channel when Stop is pressed is sent its
//! note-off, and nothing is struck again when the song plays next, so a key the player no longer
//! holds (one whose release never arrived) cannot sound on. Play and Seek still strike a held key
//! again after their reset, as before, and a pedal down at Stop lifts.
//!
//! Expected events come from the spec, not from running the engine: a live note is sent on frame
//! 0 of the call it is handed in before; Stop ends it in the call after; a jump resets the
//! processor before the call's events reach it; the plugin's clock counts the frames rendered
//! since the last Play, Seek or Stop and stands still while stopped.

use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::Touched;
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::processor::{
    Captured, Editor, HostError, HostNotice, PluginParam, PortEvent, ProcessContext, Processor,
    ProcessorHost, Registry,
};
use dawsome_engine::project::{
    Channel, ChannelId, PatternId, ProcessorOwner, ProcessorRef, Project,
};
use dawsome_engine::realtime::{self, EngineEvent, LiveControl, LiveNote, Player, RealtimeLink};
use dawsome_engine::time::{MilliBpm, Ticks};
use dawsome_engine::transport::Control;

/// What the stand-in plugin saw, in order: each note event with the block's clock, and each reset.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Seen {
    Event(u64, PortEvent),
    Reset,
}

type Log = Arc<Mutex<Vec<Seen>>>;

struct Recorder {
    log: Log,
}

impl Processor for Recorder {
    fn process(&mut self, context: &ProcessContext<'_>, buffer: &mut [[f32; 2]]) {
        let clock = context.transport.steady_time;
        let mut log = self.log.lock().expect("log");
        for &event in context.events {
            log.push(Seen::Event(clock, event));
        }
        buffer.fill([0.0; 2]);
    }

    fn set_param(&mut self, _index: usize, _value: f32) {}

    fn reset(&mut self) {
        self.log.lock().expect("log").push(Seen::Reset);
    }

    fn queue_param(&mut self, _id: u32, _value: f32) {}
}

#[derive(Default)]
struct RecordingHost {
    log: Log,
}

impl ProcessorHost for RecordingHost {
    fn format(&self) -> &'static str {
        "mock"
    }

    fn create(
        &self,
        _owner: ProcessorOwner,
        reference: &ProcessorRef,
        _sample_rate: u32,
    ) -> Result<Box<dyn Processor>, HostError> {
        match reference.id.as_str() {
            "mock:recorder" => Ok(Box::new(Recorder {
                log: Arc::clone(&self.log),
            })),
            _ => Err(HostError::Missing),
        }
    }

    fn release(&self, _owner: ProcessorOwner) {}

    fn sync_state(&self, _owner: ProcessorOwner, _reference: &ProcessorRef) {}

    fn params(&self, _owner: ProcessorOwner) -> Vec<PluginParam> {
        Vec::new()
    }

    fn value(&self, _owner: ProcessorOwner, _id: u32) -> Option<f64> {
        None
    }

    fn value_text(&self, _owner: ProcessorOwner, _id: u32, _value: f64) -> Option<String> {
        None
    }

    fn capture_state(&self, _owner: ProcessorOwner) -> Result<Captured, HostError> {
        Err(HostError::Missing)
    }

    fn open_editor(&self, _owner: ProcessorOwner, _title: &str) -> Result<Editor, HostError> {
        Ok(Editor::Generic)
    }

    fn tick(&self) -> Vec<HostNotice> {
        Vec::new()
    }

    fn has_instances(&self) -> bool {
        true
    }
}

const SYNTH: ChannelId = ChannelId(1);
const BLOCK: usize = 512;

/// 48 kHz, 120 BPM: channel 1 plays `mock:recorder`, selected for live notes, in pattern mode
/// with an empty one-bar pattern.
fn synth_project() -> Project {
    let mut project = Project::new(48_000, MilliBpm(120_000), 96);
    project.channels.insert(
        SYNTH,
        Channel::plugin("Synth", ProcessorRef::plugin("mock:recorder", "Synth")),
    );
    project.next_channel_id = ChannelId(2);
    project.patterns.insert(
        PatternId(1),
        dawsome_engine::project::Pattern {
            name: "Pattern 1".into(),
            length_bars: 1,
            copied_from: None,
            notes: Vec::new(),
        },
    );
    project.validate().expect("valid");
    project
}

/// The project linked to a stopped player, with the plugin's log.
fn stopped() -> (RealtimeLink, Player, Log) {
    let host = RecordingHost::default();
    let log = Arc::clone(&host.log);
    let mut registry = Registry::native();
    registry.add_host(Box::new(host));
    let mut compiler = Compiler::with_registry(registry);
    compiler.set_live_channel(Some(SYNTH));
    let project = synth_project();
    let silent = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(vec![0.0]));
    let update: PlanUpdate = compiler
        .compile(
            &project,
            &Touched::default(),
            PlayMode::Pattern(PatternId(1)),
            silent,
        )
        .expect("compiles");
    let (link, player) = realtime::link(update, project.tempo);
    (link, player, log)
}

/// One call of `BLOCK` frames; returns what the plugin saw in it.
fn call(player: &mut Player, log: &Log) -> Vec<Seen> {
    let mut out = vec![[0.0; 2]; BLOCK];
    player.process(&mut out);
    std::mem::take(&mut *log.lock().expect("log"))
}

fn on(key: u8) -> LiveNote {
    LiveNote::On { key, velocity: 0.5 }
}

fn off(key: u8) -> LiveNote {
    LiveNote::Off { key }
}

fn note_on(clock: u64, key: u8) -> Seen {
    Seen::Event(
        clock,
        PortEvent::NoteOn {
            frame: 0,
            key,
            velocity: 0.5,
        },
    )
}

fn note_off(clock: u64, key: u8) -> Seen {
    Seen::Event(clock, PortEvent::NoteOff { frame: 0, key })
}

fn ended(key: u8, frame: u64) -> EngineEvent {
    EngineEvent::LiveNoteEnded {
        channel: SYNTH,
        key,
        tick: None,
        frame,
    }
}

// A key held when Stop is pressed: the call after sends its note-off (after the reset every
// jump makes) and reports the note ended; Play then strikes nothing again, the key's own
// release, coming late, ends nothing, and a key pressed after plays.
#[test]
fn stop_ends_a_held_key_and_play_does_not_strike_it_again() {
    let (mut link, mut player, log) = stopped();
    let inbox = link.midi_inbox();
    assert!(inbox.push(on(60)));
    assert_eq!(call(&mut player, &log), [note_on(0, 60)]);
    link.take_events();

    link.send(Control::Stop).expect("room");
    assert_eq!(call(&mut player, &log), [Seen::Reset, note_off(0, 60)]);
    assert_eq!(link.take_events(), [ended(60, BLOCK as u64)]);

    link.send(Control::Play).expect("room");
    assert_eq!(
        call(&mut player, &log),
        [Seen::Reset],
        "nothing struck again"
    );
    assert_eq!(call(&mut player, &log), []);

    assert!(inbox.push(off(60)));
    assert_eq!(call(&mut player, &log), [], "its release ends nothing");
    assert_eq!(link.take_events(), []);

    assert!(inbox.push(on(64)));
    assert_eq!(call(&mut player, &log), [note_on(3 * BLOCK as u64, 64)]);
}

// The behavior kept: Seek and Play reset the plugin and strike a key still held again, once,
// so a note a player holds through them sounds on.
#[test]
fn seek_and_play_strike_a_held_key_again() {
    let (mut link, mut player, log) = stopped();
    let inbox = link.midi_inbox();
    assert!(inbox.push(on(60)));
    assert_eq!(call(&mut player, &log), [note_on(0, 60)]);

    link.send(Control::Seek(Ticks(0))).expect("room");
    assert_eq!(call(&mut player, &log), [Seen::Reset, note_on(0, 60)]);

    link.send(Control::Play).expect("room");
    assert_eq!(call(&mut player, &log), [Seen::Reset, note_on(0, 60)]);
    assert_eq!(call(&mut player, &log), []);
}

// A pedal down holds a key let go of; Stop lifts it: the held note ends, and a key let go of
// after ends at once, in the same call its note-on went in.
#[test]
fn stop_lifts_the_pedal() {
    let (mut link, mut player, log) = stopped();
    let inbox = link.midi_inbox();
    assert!(inbox.push_control(Some(SYNTH), LiveControl::Sustain(127), true));
    assert!(inbox.push(on(60)));
    assert!(inbox.push(off(60)));
    assert_eq!(
        call(&mut player, &log),
        [note_on(0, 60)],
        "held by the pedal"
    );

    link.send(Control::Stop).expect("room");
    assert_eq!(call(&mut player, &log), [Seen::Reset, note_off(0, 60)]);

    assert!(inbox.push(on(62)));
    assert!(inbox.push(off(62)));
    assert_eq!(
        call(&mut player, &log),
        [note_on(0, 62), note_off(0, 62)],
        "the pedal is up"
    );
}
