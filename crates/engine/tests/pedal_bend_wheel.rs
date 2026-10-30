//! The sustain pedal, pitch bend and the mod wheel in the engine: a device's controls handed to
//! the player's inbox beside its notes play the renders and the note events
//! tests/fixtures/gen_p22_expected.py computed from the rules it states (p22_expected.json),
//! however a render is split into blocks between the messages, without touching the heap.
//!
//! The messages reach the player here as a MIDI port's would, each before the 512-frame call its
//! file times it for; the bytes are read by MIDI 1.0's status bytes (9n note on, 8n note off, Bn
//! 40 the pedal, from 64 down, Bn 01 the mod wheel, En ll mm pitch bend, ll + 128 mm − 8192), a
//! restatement in the test, never the app's parser (the app's own tests run the same files
//! through its door).

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::{ClipHint, ClipSound, Compiler, Sounds};
use dawsome_engine::dsp::instrument::{Instrument, ZoneSample};
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::Touched;
use dawsome_engine::midi_out::{MidiOutEvent, MidiOutReader};
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::project::{
    ChannelId, Note, PatternId, Project, DEFAULT_BEND_RANGE, SOUND_FORMAT_VERSION,
};
use dawsome_engine::realtime::{
    self, EngineEvent, LiveControl, LiveNote, MidiInbox, Player, RealtimeLink,
};
use dawsome_engine::sfz;
use dawsome_engine::time::Ticks;
use dawsome_engine::transport::Control;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

/// Frames per call, as the messages' files time them.
const CALL: usize = 512;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p22_expected.json"))
        .expect("p22_expected.json (run gen_p22_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("parses")
}

fn wav(path: &Path) -> ZoneSample {
    let mut reader = hound::WavReader::open(path).expect("a test sample");
    let spec = reader.spec();
    let values: Vec<f32> = reader
        .samples::<f32>()
        .map(|value| value.expect("reads"))
        .collect();
    let buffer = if spec.channels == 2 {
        SampleBuffer::stereo(values)
    } else {
        SampleBuffer::mono(values)
    };
    ZoneSample {
        buffer,
        file_rate: Some(spec.sample_rate),
        at_file_rate: false,
    }
}

/// Sounds from the fixtures folder: a set read as a loader reads it, its sounds beside it.
struct FixtureSets;

impl Sounds for FixtureSets {
    type Error = Infallible;

    fn sample(&mut self, path: &Path) -> Result<Option<SampleBuffer>, Infallible> {
        Ok(Some(wav(&fixtures().join(path)).buffer))
    }

    fn clip(&mut self, path: &Path, _hint: ClipHint) -> Result<ClipSound, Infallible> {
        Ok(ClipSound::Memory(wav(&fixtures().join(path)).buffer))
    }

    fn instrument(&mut self, path: &Path) -> Result<Option<Instrument>, Infallible> {
        let main = fixtures().join(path);
        let dir = main.parent().expect("a folder").to_path_buf();
        let text = fs::read_to_string(&main).expect("the set's file");
        let read = sfz::parse("set.sfz", &text, &mut |path| {
            fs::read_to_string(dir.join(path)).map_err(|error| error.to_string())
        });
        let (instrument, problems) =
            Instrument::build(&read.regions, |sample| wav(&dir.join(sample)), 44_100);
        assert_eq!(problems, Vec::<String>::new());
        Ok(Some(instrument))
    }
}

fn load(name: &str) -> Project {
    let text = fs::read_to_string(fixtures().join(name)).expect("a p22 fixture");
    Project::load_json(&text).expect("loads")
}

fn compiled(project: &Project) -> PlanUpdate {
    let mut compiler = Compiler::new();
    compiler.set_live_channel(project.channels.keys().next().copied());
    compiler
        .compile_with(
            project,
            &Touched::default(),
            PlayMode::Pattern(PatternId(1)),
            &mut FixtureSets,
        )
        .expect("compiles")
}

/// The project's player, playing, the first channel selected.
fn playing(project: &Project) -> (RealtimeLink, Player) {
    let (mut link, player) = realtime::link(compiled(project), project.tempo);
    link.send(Control::Play).expect("room");
    (link, player)
}

/// One message of an events file: the call it comes before, its bytes, the port it names.
type Timed = (u64, Vec<u8>, Option<String>);

fn events(name: &str) -> Vec<Timed> {
    let text = fs::read_to_string(fixtures().join(name)).expect("an events file");
    let file: Value = serde_json::from_str(&text).expect("parses");
    assert_eq!(file["buffer"].as_u64(), Some(CALL as u64));
    file["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|event| {
            let bytes = event["bytes"]
                .as_array()
                .expect("bytes")
                .iter()
                .map(|byte| u8::try_from(byte.as_u64().expect("a byte")).expect("a byte"))
                .collect();
            let port = event["port"].as_str().map(str::to_owned);
            (event["call"].as_u64().expect("a call"), bytes, port)
        })
        .collect()
}

/// Hands one message to `inbox` as a port routed to the selected channel would: going on to a
/// MIDI out when `out`. MIDI 1.0's status bytes, restated: 9n kk vv a note on (vv 0 a note off),
/// 8n kk vv a note off, Bn 40 vv the pedal, Bn 01 vv the mod wheel, En ll mm pitch bend.
fn deliver(inbox: &MidiInbox, bytes: &[u8], out: bool) {
    let note = |note| {
        if out {
            inbox.push(note)
        } else {
            inbox.push_quiet(None, note)
        }
    };
    let taken = match *bytes {
        [status, key, 0] if status & 0xF0 == 0x90 => note(LiveNote::Off { key }),
        [status, key, velocity] if status & 0xF0 == 0x90 => note(LiveNote::On {
            key,
            velocity: f32::from(velocity) / 128.0,
        }),
        [status, key, _] if status & 0xF0 == 0x80 => note(LiveNote::Off { key }),
        [status, 64, value] if status & 0xF0 == 0xB0 => {
            inbox.push_control(None, LiveControl::Sustain(value), out)
        }
        [status, 1, value] if status & 0xF0 == 0xB0 => {
            inbox.push_control(None, LiveControl::Wheel(value), out)
        }
        [status, low, high] if status & 0xF0 == 0xE0 => {
            let value = i16::from(low) + 128 * i16::from(high) - 8192;
            inbox.push_control(None, LiveControl::Bend(value), out)
        }
        _ => panic!("a message the cases do not send: {bytes:02X?}"),
    };
    assert!(taken, "the inbox had room");
}

/// The door: the messages of `script` handed in before the call each is timed for, from the
/// inbox's own handle, or, for a message naming a port, from that port's handle (a quiet one:
/// the MIDI out's own port, whose instrument hears its keys already).
struct Door {
    script: Vec<Timed>,
    next: usize,
    inbox: MidiInbox,
    ports: BTreeMap<String, MidiInbox>,
}

impl Door {
    fn new(link: &RealtimeLink, script: Vec<Timed>) -> Self {
        Self {
            script,
            next: 0,
            inbox: link.midi_inbox(),
            ports: BTreeMap::new(),
        }
    }

    /// Hands in the messages timed for `call` and before; whether any was.
    fn before(&mut self, call: u64) -> bool {
        let mut any = false;
        while let Some((at, bytes, port)) = self.script.get(self.next).cloned() {
            if at > call {
                break;
            }
            match port {
                Some(port) => {
                    let inbox = self
                        .ports
                        .entry(port)
                        .or_insert_with(|| self.inbox.for_port());
                    deliver(inbox, &bytes, false);
                }
                None => deliver(&self.inbox, &bytes, true),
            }
            self.next += 1;
            any = true;
        }
        any
    }

    /// The frame the next message is taken at, if any is left.
    fn next_frame(&self) -> Option<usize> {
        let (call, _, _) = self.script.get(self.next)?;
        Some(usize::try_from(*call).expect("a call") * CALL)
    }
}

/// Plays `frames` frames of `project` with `script`'s messages, each process call `sizes` says
/// long but cut where a message is taken (on its call's first frame), so the messages reach the
/// player exactly where 512-frame calls would hand them over. Returns the frames and every event
/// the player posted.
fn render(
    project: &Project,
    script: Vec<Timed>,
    frames: usize,
    sizes: impl IntoIterator<Item = usize>,
) -> (Vec<[f32; 2]>, Vec<EngineEvent>) {
    let (mut link, mut player) = playing(project);
    let mut door = Door::new(&link, script);
    let mut out = vec![[f32::NAN; 2]; frames];
    let mut events = Vec::new();
    let mut done = 0;
    let mut sizes = sizes.into_iter();
    door.before(0);
    while done < frames {
        let size = sizes.next().expect("enough block sizes");
        let mut end = (done + size).min(frames);
        if let Some(next) = door.next_frame().filter(|&next| next > done) {
            end = end.min(next);
        }
        player.process(&mut out[done..end]);
        events.extend(link.take_events());
        done = end;
        if done % CALL == 0 {
            door.before((done / CALL) as u64);
        }
    }
    (out, events)
}

/// 512-frame calls, as the app's door and the command line call the player.
fn calls() -> impl Iterator<Item = usize> {
    std::iter::repeat(CALL)
}

/// Edge-case sizes (1, a prime, 64, a round number, 4096), then seeded ones.
fn block_sizes(seed: u64) -> impl Iterator<Item = usize> {
    let mut state = seed;
    let random = std::iter::repeat_with(move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        usize::try_from(state % 5000).expect("below 5000") + 1
    });
    [1, 7, 64, 1000, 4096].into_iter().chain(random)
}

/// The SHA-256 of `frames` as 32-bit floats, little-endian, left then right.
fn frames_hash(frames: &[[f32; 2]]) -> String {
    let mut hasher = Sha256::new();
    for frame in frames {
        hasher.update(frame[0].to_le_bytes());
        hasher.update(frame[1].to_le_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Asserts `frames` is render `name` of the generator: length, sounding frames, probes (to the
/// bit or to its tolerance) and, without a tolerance, the hash of every frame.
fn assert_render(name: &str, frames: &[[f32; 2]], want: &Value) {
    assert_eq!(
        frames.len() as u64,
        want["frames"].as_u64().expect("frames"),
        "{name}"
    );
    let tolerance = want["tolerance"].as_f64();
    for probe in want["probes"].as_array().expect("probes") {
        let at = probe[0].as_u64().expect("a frame") as usize;
        let side = |index: usize| probe[index].as_f64().expect("a value") as f32;
        let wanted = [side(1), side(2)];
        match tolerance {
            Some(tolerance) => {
                for (got, wanted) in frames[at].iter().zip(wanted) {
                    assert!(
                        (f64::from(*got) - f64::from(wanted)).abs() <= tolerance,
                        "{name}, frame {at}: {:?} is not {wanted:?} within {tolerance}",
                        frames[at]
                    );
                }
            }
            None => assert_eq!(
                frames[at].map(f32::to_bits),
                wanted.map(f32::to_bits),
                "{name}, frame {at}: {:?} != {wanted:?}",
                frames[at]
            ),
        }
    }
    let sounding = frames.iter().filter(|frame| **frame != [0.0; 2]).count();
    assert_eq!(
        sounding as u64,
        want["sounding"].as_u64().expect("a count"),
        "{name}: sounding frames"
    );
    if tolerance.is_none() {
        assert_eq!(
            frames_hash(frames),
            want["sha256"].as_str().expect("a hash"),
            "{name}: the frames differ from the generator's"
        );
    }
}

/// Render `name` of p22_expected.json in 512-frame calls, checked; its frames and events.
fn checked(name: &str) -> (Vec<[f32; 2]>, Vec<EngineEvent>) {
    let want = &expected()["renders"][name];
    let project = load(want["fixture"].as_str().expect("fixture"));
    let script = events(want["events"].as_str().expect("events"));
    let frames = want["frames"].as_u64().expect("frames") as usize;
    let (out, events) = render(&project, script, frames, calls());
    assert_render(name, &out, want);
    (out, events)
}

/// Asserts render `name` gives the same bits however its calls are cut between the messages.
fn split_invariant(name: &str, reference: &[[f32; 2]]) {
    let want = &expected()["renders"][name];
    let project = load(want["fixture"].as_str().expect("fixture"));
    for seed in [1, 0x9E37_79B9_7F4A_7C15] {
        let script = events(want["events"].as_str().expect("events"));
        let (out, _) = render(&project, script, reference.len(), block_sizes(seed));
        assert!(
            out.iter()
                .map(|f| f.map(f32::to_bits))
                .eq(reference.iter().map(|f| f.map(f32::to_bits))),
            "{name}: a split render differs (seed {seed})"
        );
    }
}

/// The (key, tick it was heard at, frame) of each live note started and ended, in order.
fn notes_heard(events: &[EngineEvent]) -> Vec<(bool, u8, Option<Ticks>, u64)> {
    events
        .iter()
        .filter_map(|event| match *event {
            EngineEvent::LiveNoteStarted {
                key, tick, frame, ..
            } => Some((true, key, tick, frame)),
            EngineEvent::LiveNoteEnded {
                key, tick, frame, ..
            } => Some((false, key, tick, frame)),
            _ => None,
        })
        .collect()
}

/// The (start, end) ticks of the notes a take would write, as the generator's `take`.
fn take_ticks(want: &Value) -> Vec<(i64, i64)> {
    want.as_array()
        .expect("notes")
        .iter()
        .map(|note| {
            let tick = note["tick"].as_i64().expect("a tick");
            (tick, tick + note["length"].as_i64().expect("a length"))
        })
        .collect()
}

/// The ticks each note was heard from and to, in the order they ended.
fn heard_ticks(events: &[EngineEvent]) -> Vec<(i64, i64)> {
    let mut started = BTreeMap::new();
    let mut spans = Vec::new();
    for (on, key, tick, _) in notes_heard(events) {
        let tick = tick.expect("heard while the song runs").0;
        if on {
            started.insert(key, tick);
        } else if let Some(from) = started.remove(&key) {
            spans.push((from, tick));
        }
    }
    spans
}

// (1) Pedal down, note on, note off, pedal up: the note sounds to the lift (frame 20480, call
// 40), as the note held to that call does, bit for bit; its end is heard there, tick 89, where a
// take ends it.
#[test]
fn p22_a_pedal_holds_a_note_to_its_lift() {
    let (pedal, events) = checked("pedal");
    let (held, _) = checked("held");
    assert!(pedal == held, "the pedal's note is the held note");
    assert_eq!(
        heard_ticks(&events),
        take_ticks(&expected()["renders"]["pedal"]["take"])
    );
    assert_eq!(
        notes_heard(&events)
            .iter()
            .map(|&(on, _, _, frame)| (on, frame))
            .collect::<Vec<_>>(),
        [(true, 2048), (false, 20480)]
    );
    split_invariant("pedal", &pedal);
}

// (2) A key struck again under the pedal ends its first voice there (call 20) and starts anew;
// the lift (call 40) ends the second.
#[test]
fn p22_a_key_struck_again_under_the_pedal_starts_anew() {
    let (out, events) = checked("repress");
    assert_eq!(
        heard_ticks(&events),
        take_ticks(&expected()["renders"]["repress"]["take"])
    );
    split_invariant("repress", &out);
}

/// p10_link.json: "kick" (channel 1, selected) and "keys" (channel 2), samplers of impulse.wav.
fn two_channels() -> Project {
    load("p10_link.json")
}

fn on(key: u8) -> LiveNote {
    LiveNote::On { key, velocity: 0.5 }
}

/// The live notes started and ended in one call, as (started, channel, key).
fn call_events(link: &mut RealtimeLink, player: &mut Player) -> Vec<(bool, ChannelId, u8)> {
    player.process(&mut [[0.0; 2]; 64]);
    link.take_events()
        .into_iter()
        .filter_map(|event| match event {
            EngineEvent::LiveNoteStarted { channel, key, .. } => Some((true, channel, key)),
            EngineEvent::LiveNoteEnded { channel, key, .. } => Some((false, channel, key)),
            _ => None,
        })
        .collect()
}

// One input's pedal holds its own notes only: a second port's key on the same channel ends at
// its note-off while the first port's pedal is down, and the first port's key ends on the lift.
#[test]
fn a_pedal_holds_its_own_inputs_notes_only() {
    let (mut link, mut player) = playing(&two_channels());
    let (pedal, other) = (link.midi_inbox().for_port(), link.midi_inbox().for_port());
    let kick = ChannelId(1);
    assert!(pedal.push_control(None, LiveControl::Sustain(127), true));
    assert!(pedal.push(on(60)) && other.push(on(62)));
    assert!(pedal.push(LiveNote::Off { key: 60 }) && other.push(LiveNote::Off { key: 62 }));
    assert_eq!(
        call_events(&mut link, &mut player),
        [(true, kick, 60), (true, kick, 62), (false, kick, 62)]
    );
    assert!(other.push_control(None, LiveControl::Sustain(0), true));
    assert!(
        call_events(&mut link, &mut player).is_empty(),
        "another port's lift lets go of nothing"
    );
    assert!(pedal.push_control(None, LiveControl::Sustain(0), true));
    assert_eq!(call_events(&mut link, &mut player), [(false, kick, 60)]);
}

// A lift routed to another channel than the press (the selection moved between) still lets go
// of every key the input's pedal held, in the order they were let go of, wherever they play.
#[test]
fn a_lift_lets_go_on_every_channel_the_inputs_pedal_held() {
    let (mut link, mut player) = playing(&two_channels());
    let port = link.midi_inbox().for_port();
    let (kick, keys) = (ChannelId(1), ChannelId(2));
    assert!(port.push_control(Some(keys), LiveControl::Sustain(100), true));
    assert!(port.push_to(keys, on(64)) && port.push_to(keys, on(60)));
    assert!(port.push(LiveNote::Off { key: 64 }) && port.push(LiveNote::Off { key: 60 }));
    assert_eq!(
        call_events(&mut link, &mut player),
        [(true, keys, 64), (true, keys, 60)],
        "both held by the pedal"
    );
    assert!(port.push_control(Some(kick), LiveControl::Sustain(10), true));
    assert_eq!(
        call_events(&mut link, &mut player),
        [(false, keys, 64), (false, keys, 60)],
        "the lift on kick lets go on keys, in the order let go of"
    );
}

// The typing keyboard's sustain key is the same pedal: on the control ring's input, its lift
// ends the notes let go of meanwhile; a note held down when it lifts sounds on.
#[test]
fn the_typing_keyboards_pedal_is_the_engines() {
    let (mut link, mut player) = playing(&two_channels());
    let kick = ChannelId(1);
    let tab = |down: bool| Control::LiveControl {
        channel: None,
        control: LiveControl::Sustain(if down { 127 } else { 0 }),
    };
    for control in [
        tab(true),
        Control::LiveNoteOn {
            key: 60,
            velocity: 0.5,
        },
        Control::LiveNoteOff { key: 60 },
        Control::LiveNoteOn {
            key: 62,
            velocity: 0.5,
        },
    ] {
        link.send(control).expect("room");
    }
    assert_eq!(
        call_events(&mut link, &mut player),
        [(true, kick, 60), (true, kick, 62)]
    );
    link.send(tab(false)).expect("room");
    assert_eq!(
        call_events(&mut link, &mut player),
        [(false, kick, 60)],
        "62 is still down"
    );
    link.send(Control::LiveNoteOff { key: 62 }).expect("room");
    assert_eq!(call_events(&mut link, &mut player), [(false, kick, 62)]);
}

// All notes off ends the notes the pedals hold too and lifts every pedal; a port that goes lifts
// its own, so a later note-off of another port ends at once.
#[test]
fn all_notes_off_and_a_port_that_goes_lift_the_pedal() {
    let (mut link, mut player) = playing(&two_channels());
    let kick = ChannelId(1);
    let port = link.midi_inbox().for_port();
    assert!(port.push_control(None, LiveControl::Sustain(127), true));
    assert!(port.push(on(60)) && port.push(LiveNote::Off { key: 60 }));
    let _ = call_events(&mut link, &mut player);
    link.send(Control::LiveNotesOff).expect("room");
    assert_eq!(call_events(&mut link, &mut player), [(false, kick, 60)]);
    assert!(port.push(on(62)) && port.push(LiveNote::Off { key: 62 }));
    assert_eq!(
        call_events(&mut link, &mut player),
        [(true, kick, 62), (false, kick, 62)],
        "the pedal is up"
    );
    assert!(port.push_control(None, LiveControl::Sustain(127), true));
    assert!(port.push(on(64)) && port.push(LiveNote::Off { key: 64 }));
    let _ = call_events(&mut link, &mut player);
    assert!(port.release());
    assert_eq!(call_events(&mut link, &mut player), [(false, kick, 64)]);
    assert!(port.push(on(65)) && port.push(LiveNote::Off { key: 65 }));
    assert_eq!(
        call_events(&mut link, &mut player),
        [(true, kick, 65), (false, kick, 65)]
    );
}

/// What the MIDI out sent so far, as (frame, channel, bytes).
fn sent(reader: &mut MidiOutReader) -> Vec<(u64, u32, Vec<u8>)> {
    let mut out: Vec<MidiOutEvent> = Vec::new();
    reader.read_into(&mut out);
    out.iter()
        .map(|event| (event.frame, event.channel.0, event.bytes().to_vec()))
        .collect()
}

/// The pedal, wheel and bend messages (Bn and En) the MIDI out sent so far, as [`sent`] gives
/// them.
fn controls_sent(reader: &mut MidiOutReader) -> Vec<(u64, u32, Vec<u8>)> {
    sent(reader)
        .into_iter()
        .filter(|(_, _, bytes)| matches!(bytes[0] & 0xF0, 0xB0 | 0xE0))
        .collect()
}

// By hand from the MIDI 1.0 bytes, p22_out.json's channel 1 (MIDI channel 1): the pedal goes out
// as B0 40 vv at its call's first frame, a repeat of the value sends nothing, the note let go of
// under it sends its note off only after the lift's B0 40 00; a quiet port's pedal sends nothing.
#[test]
fn a_midi_out_sends_the_pedal_on_once_and_its_lift_before_the_notes_it_held() {
    let project = load("p22_out.json");
    let (mut link, mut player) = playing(&project);
    let mut reader = link.take_midi_out().expect("the ring");
    let inbox = link.midi_inbox();
    let quiet = inbox.for_port();
    let call = |player: &mut Player| player.process(&mut [[0.0; 2]; CALL]);
    call(&mut player);
    assert!(inbox.push_control(None, LiveControl::Sustain(127), true));
    call(&mut player);
    assert!(inbox.push(LiveNote::On {
        key: 60,
        velocity: 100.0 / 128.0
    }));
    call(&mut player);
    assert!(inbox.push(LiveNote::Off { key: 60 }));
    call(&mut player);
    assert!(inbox.push_control(None, LiveControl::Sustain(127), true));
    assert!(quiet.push_control(None, LiveControl::Sustain(127), false));
    call(&mut player);
    assert!(inbox.push_control(None, LiveControl::Sustain(0), true));
    call(&mut player);
    assert_eq!(
        sent(&mut reader),
        [
            (512, 1, vec![0xB0, 64, 127]),
            (1024, 1, vec![0x90, 60, 100]),
            (2560, 1, vec![0xB0, 64, 0]),
            (2560, 1, vec![0x80, 60, 0x40]),
        ]
    );
}

// The pedal's whole path, pressed, notes let go of under it, a key struck again, the lift, a
// release and all notes off, never touches the heap on the audio thread.
#[test]
fn the_pedals_path_never_touches_the_heap() {
    let (mut link, mut player) = playing(&two_channels());
    let port = link.midi_inbox().for_port();
    let mut buffer = vec![[0.0f32; 2]; CALL];
    player.process(&mut buffer);
    let _ = link.take_events();
    let calls = heap_calls_during(|| {
        assert!(port.push_control(None, LiveControl::Sustain(127), true));
        assert!(port.push(on(60)) && port.push(LiveNote::Off { key: 60 }));
        player.process(&mut buffer);
        assert!(port.push(on(60)) && port.push(on(62)));
        assert!(port.push(LiveNote::Off { key: 60 }) && port.push(LiveNote::Off { key: 62 }));
        player.process(&mut buffer);
        assert!(port.push_control(None, LiveControl::Sustain(0), true));
        player.process(&mut buffer);
        assert!(port.push_control(None, LiveControl::Sustain(90), true));
        assert!(port.push(on(64)) && port.push(LiveNote::Off { key: 64 }));
        assert!(port.release());
        player.process(&mut buffer);
        player.process(&mut buffer);
    });
    assert_eq!(calls, 0, "the pedal's path touched the heap");
    link.send(Control::Stop).expect("room");
}

// (3) Pitch bend under a held note: +8191 plays 2^(2/12) from its call (frame 10240) on, -8192
// 2^(-2/12) from frame 20480, the center the zone's own speed from 25600, each reading on from
// where the voice was; at a range of 12, +8191 plays twice the speed. However the calls are cut
// between the messages, the same bits.
#[test]
fn p22_a_bend_plays_from_its_call_at_the_bent_rate() {
    let (bend, _) = checked("bend");
    split_invariant("bend", &bend);
    let (range, _) = checked("range");
    split_invariant("range", &range);
}

// (5) Pedal up with no note does nothing; a bend with no voice waits for the next note, which
// starts bent; CC 64 at 63 is up and at 64 down.
#[test]
fn p22_saturated_pedal_and_bend_messages() {
    let (out, _) = checked("saturated");
    split_invariant("saturated", &out);
}

// The bend is the live notes': a pattern's note on the channel plays as it would without it,
// bit for bit, while a live note there bends.
#[test]
fn a_bend_moves_live_notes_only() {
    let mut project = load("p22_sine.json");
    project
        .patterns
        .get_mut(&PatternId(1))
        .expect("pattern 1")
        .notes = vec![Note {
        channel: ChannelId(1),
        start: Ticks(0),
        length: Ticks(96),
        velocity: 1.0,
        key: 69,
    }];
    let frames = 30 * CALL;
    let timed = |call: u64, bytes: &[u8]| (call, bytes.to_vec(), None);
    let bent = vec![timed(4, &[0xE0, 0x7F, 0x7F])];
    let (plain, _) = render(&project, Vec::new(), frames, calls());
    let (with_bend, _) = render(&project, bent, frames, calls());
    assert!(
        plain[..frames] == with_bend[..frames],
        "the pattern note bends not"
    );
    let live = vec![
        timed(2, &[0x90, 72, 127]),
        timed(4, &[0xE0, 0x7F, 0x7F]),
        timed(20, &[0x80, 72, 0]),
    ];
    let (plain, _) = render(&project, live[..1].to_vec(), frames, calls());
    let (bent, _) = render(&project, live, frames, calls());
    assert!(plain[..4 * CALL] == bent[..4 * CALL]);
    assert!(
        plain[4 * CALL + 1..] != bent[4 * CALL + 1..],
        "the live note bends"
    );
}

// By hand from the MIDI 1.0 bytes: a channel with a MIDI out sends the bend on as En ll mm at its
// call's first frame (+8191: E0 7F 7F, the center E0 00 40), a repeat sends nothing, and a port
// that goes centers the bend it sent.
#[test]
fn a_midi_out_sends_the_bend_on_and_a_port_that_goes_centers_it() {
    let project = load("p22_out.json");
    let (mut link, mut player) = playing(&project);
    let mut reader = link.take_midi_out().expect("the ring");
    let port = link.midi_inbox().for_port();
    let call = |player: &mut Player| player.process(&mut [[0.0; 2]; CALL]);
    assert!(port.push_control(None, LiveControl::Bend(8191), true));
    call(&mut player);
    assert!(port.push_control(None, LiveControl::Bend(8191), true));
    call(&mut player);
    assert!(port.push_control(None, LiveControl::Bend(0), true));
    call(&mut player);
    assert!(port.push_control(None, LiveControl::Bend(-8192), true));
    call(&mut player);
    assert!(port.release());
    call(&mut player);
    assert_eq!(
        sent(&mut reader),
        [
            (0, 1, vec![0xE0, 0x7F, 0x7F]),
            (1024, 1, vec![0xE0, 0x00, 0x40]),
            (1536, 1, vec![0xE0, 0x00, 0x00]),
            (2048, 1, vec![0xE0, 0x00, 0x40]),
        ]
    );
}

// A bend range written off its default reads back and keeps the format version sound settings
// have always written (5); a file that never names one, a version 7 file among them, reads 2,
// and a project at the default range writes the bytes it was read from.
#[test]
fn the_bend_range_reads_back_and_moves_no_format_version() {
    let range = load("p22_range.json");
    assert_eq!(range.channels[&ChannelId(1)].sound.bend_range, 12);
    let written = range.to_json().expect("writes");
    assert!(written.starts_with(&format!(r#"{{"format_version":{SOUND_FORMAT_VERSION},"#)));
    assert!(
        written.contains(r#""resonance":0.0,"bend_range":12}"#),
        "{written}"
    );
    assert_eq!(Project::load_json(&written).expect("reads"), range);
    let seven = load("p22_out.json");
    assert_eq!(
        seven.channels[&ChannelId(1)].sound.bend_range,
        DEFAULT_BEND_RANGE
    );
    for name in ["p22_sine.json", "p22_out.json", "p22_filter.json"] {
        let text = fs::read_to_string(fixtures().join(name)).expect("a fixture");
        assert_eq!(load(name).to_json().expect("writes") + "\n", text, "{name}");
    }
}

// A live note bending on an instrument set, the bend handed in, centered, sent to a MIDI out, and
// a port that goes with its bend off the center: no heap call on the audio thread.
#[test]
fn the_bends_path_never_touches_the_heap() {
    let (a, b) = (ChannelId(1), ChannelId(2));
    for name in ["p22_sine.json", "p22_out.json"] {
        let (mut link, mut player) = playing(&with_sine_b(load(name)));
        let mut reader = link.take_midi_out().expect("the ring");
        let port = link.midi_inbox().for_port();
        let mut buffer = vec![[0.0f32; 2]; CALL];
        player.process(&mut buffer);
        let calls = heap_calls_during(|| {
            assert!(port.push(LiveNote::On {
                key: 69,
                velocity: 1.0
            }));
            player.process(&mut buffer);
            assert!(port.push_control(None, LiveControl::Bend(3000), true));
            player.process(&mut buffer);
            assert!(port.push_control(None, LiveControl::Bend(-8192), true));
            assert!(port.push(LiveNote::On {
                key: 72,
                velocity: 1.0
            }));
            player.process(&mut buffer);
            assert!(port.release());
            player.process(&mut buffer);
            link.send(Control::LiveNotesOff).expect("room");
            player.process(&mut buffer);
        });
        assert_eq!(calls, 0, "{name}: the bend's path touched the heap");
        // Found 2026-10-26: bent on A under its note, then
        // on B under B's, the bend rests on A (A's voice re-based, A's MIDI out sent E0 00 40);
        // bent back on A, it rests on B. A heap call nowhere.
        let _ = sent(&mut reader);
        let calls = heap_calls_during(|| {
            assert!(port.push_to(a, on(69)));
            assert!(port.push_control(Some(a), LiveControl::Bend(3000), true));
            player.process(&mut buffer);
            assert!(port.push_to(b, on(72)));
            assert!(port.push_control(Some(b), LiveControl::Bend(-5000), true));
            player.process(&mut buffer);
            assert!(port.push_control(Some(a), LiveControl::Bend(0), true));
            player.process(&mut buffer);
            assert!(port.release());
            player.process(&mut buffer);
        });
        assert_eq!(calls, 0, "{name}: the bend's rest on A touched the heap");
        // By hand: 3000 + 8192 = 11192 = 0x57·128 + 0x38; the center 8192 = 0x40·128.
        let sent = controls_sent(&mut reader);
        let at = sent.first().map_or(0, |event| event.0);
        let want = match name {
            "p22_out.json" => vec![
                (at, 1, vec![0xE0, 0x38, 0x57]),
                (at + CALL as u64, 1, vec![0xE0, 0x00, 0x40]),
            ],
            _ => Vec::new(),
        };
        assert_eq!(sent, want, "{name}: the bend's rest on A's MIDI out");
    }
}

// A sustain loop's release reads on from where its note ended, also once a bend moves it after
// the end: the pad set's loop_sustain zone (key 72, loop 20..29 of a 48-frame ramp, released over
// 176 frames) let go of at frame 64 reads on from frame 24 of its ramp (64 wrapped into the loop)
// to its end; bent at 72, inside that, it reads the same point at 72 as unbent (the bend moves
// what comes after, never where it is) and another from 73 on, with no jump back to the loop.
#[test]
fn a_bend_inside_a_sustain_loops_release_reads_on_from_where_it_was() {
    let mut project = load("p22_sine.json");
    project
        .channels
        .get_mut(&ChannelId(1))
        .expect("channel 1")
        .instrument = PathBuf::from("instruments/pad/pad.sfz");
    let play = |bend: bool| {
        let (link, mut player) = playing(&project);
        let port = link.midi_inbox().for_port();
        let mut out = vec![[0.0f32; 2]; 128];
        assert!(port.push(LiveNote::On {
            key: 72,
            velocity: 1.0
        }));
        player.process(&mut out[..64]);
        assert!(port.push(LiveNote::Off { key: 72 }));
        player.process(&mut out[64..72]);
        if bend {
            assert!(port.push_control(None, LiveControl::Bend(8191), true));
        }
        player.process(&mut out[72..]);
        out
    };
    let (plain, bent) = (play(false), play(true));
    assert!(plain[..73] == bent[..73], "the same up to the bend's frame");
    assert!(plain[73] != bent[73], "bent from the next frame");
    // A frame moves the ramp by 1/64 (bent, 2^(2/12)/64) under the release's fall, about 0.01
    // either way; a jump back to the loop's start, as reading from the wrong end point would,
    // moves it by about 0.12.
    let step = (bent[73][0] - bent[72][0]).abs();
    assert!(step < 0.03, "read on, no jump: {step}");
}

// (4) The mod wheel at 127 before a note on a filtered channel opens its filter four octaves
// (1 kHz to 16 kHz): the note plays as the note with its filter set to 16 kHz by hand, bit for
// bit, up to call 30, where the wheel goes back to 0 and the channel's own filter plays on, its
// memory carried. On an unfiltered channel the wheel swings a vibrato from where the voice is.
// Both, however the calls are cut between the messages, the same bits.
#[test]
fn p22_the_wheel_opens_the_filter_as_set_by_hand_or_plays_vibrato() {
    let (wheel, _) = checked("wheel");
    let (hand, _) = checked("wheel_hand");
    let until = expected()["wheel_same_until"].as_u64().expect("a frame") as usize;
    assert!(
        wheel[..until] == hand[..until],
        "the wheel's filter is the hand-set filter"
    );
    assert!(wheel[until..] != hand[until..]);
    split_invariant("wheel", &wheel);
    let (vibrato, _) = checked("vibrato");
    split_invariant("vibrato", &vibrato);
}

// (6) p22_out's MIDI out sends the pedal, the bend and the wheel on as the generator says: each
// at its call's first frame, a repeat sends nothing, the pedal's lift before the note off it held,
// a quiet port's pedal nothing.
#[test]
fn p22_a_midi_out_sends_pedal_bend_and_wheel_on() {
    let case = &expected()["out"];
    let project = load(case["fixture"].as_str().expect("fixture"));
    let (mut link, mut player) = playing(&project);
    let mut reader = link.take_midi_out().expect("the ring");
    let mut door = Door::new(&link, events(case["events"].as_str().expect("events")));
    for call in 0..20 {
        door.before(call);
        player.process(&mut [[0.0; 2]; CALL]);
    }
    let messages: Vec<Value> = sent(&mut reader)
        .into_iter()
        .map(|(frame, channel, bytes)| {
            let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
            serde_json::json!({"frame": frame, "channel": channel, "bytes": hex.join(" ")})
        })
        .collect();
    assert_eq!(Value::Array(messages), case["messages"]);
}

// A port that goes sets its wheel back to 0 on the MIDI out it moved, as its bend.
#[test]
fn a_port_that_goes_sets_its_wheel_back() {
    let project = load("p22_out.json");
    let (mut link, mut player) = playing(&project);
    let mut reader = link.take_midi_out().expect("the ring");
    let port = link.midi_inbox().for_port();
    assert!(port.push_control(None, LiveControl::Wheel(90), true));
    player.process(&mut [[0.0; 2]; CALL]);
    assert!(port.release());
    player.process(&mut [[0.0; 2]; CALL]);
    assert_eq!(
        sent(&mut reader),
        [(0, 1, vec![0xB0, 1, 90]), (512, 1, vec![0xB0, 1, 0])]
    );
}

// The wheel's paths, the filter opened and moved and the vibrato swung on live notes, a port that
// goes and all notes off: no heap call on the audio thread.
#[test]
fn the_wheels_path_never_touches_the_heap() {
    let (a, b) = (ChannelId(1), ChannelId(2));
    for name in ["p22_sine.json", "p22_filter.json", "p22_out.json"] {
        let (mut link, mut player) = playing(&with_sine_b(load(name)));
        let mut reader = link.take_midi_out().expect("the ring");
        let port = link.midi_inbox().for_port();
        let mut buffer = vec![[0.0f32; 2]; CALL];
        player.process(&mut buffer);
        let calls = heap_calls_during(|| {
            assert!(port.push_control(None, LiveControl::Wheel(127), true));
            assert!(port.push(LiveNote::On {
                key: 69,
                velocity: 1.0
            }));
            player.process(&mut buffer);
            assert!(port.push_control(None, LiveControl::Wheel(30), true));
            assert!(port.push_control(None, LiveControl::Bend(-2000), true));
            player.process(&mut buffer);
            assert!(port.release());
            player.process(&mut buffer);
            assert!(port.push_control(None, LiveControl::Wheel(64), true));
            link.send(Control::LiveNotesOff).expect("room");
            player.process(&mut buffer);
        });
        assert_eq!(calls, 0, "{name}: the wheel's path touched the heap");
        // Found 2026-10-26: moved on A under its note, then
        // on B under B's, the wheel rests on A (A's voice unswung or its filter closed, A's MIDI
        // out sent B0 01 00); moved back on A, it rests on B. A heap call nowhere.
        let _ = sent(&mut reader);
        let calls = heap_calls_during(|| {
            assert!(port.push_to(a, on(69)));
            assert!(port.push_control(Some(a), LiveControl::Wheel(100), true));
            player.process(&mut buffer);
            assert!(port.push_to(b, on(72)));
            assert!(port.push_control(Some(b), LiveControl::Wheel(40), true));
            player.process(&mut buffer);
            assert!(port.push_control(Some(a), LiveControl::Wheel(0), true));
            player.process(&mut buffer);
            assert!(port.release());
            player.process(&mut buffer);
        });
        assert_eq!(calls, 0, "{name}: the wheel's rest on A touched the heap");
        let sent = controls_sent(&mut reader);
        let at = sent.first().map_or(0, |event| event.0);
        let want = match name {
            "p22_out.json" => vec![
                (at, 1, vec![0xB0, 1, 100]),
                (at + CALL as u64, 1, vec![0xB0, 1, 0]),
            ],
            _ => Vec::new(),
        };
        assert_eq!(sent, want, "{name}: the wheel's rest on A's MIDI out");
    }
}

/// `project` (a p22 fixture: channel 1 alone) with a second channel, "Sine B" (2), playing
/// p22_sine's set.
fn with_sine_b(mut project: Project) -> Project {
    let b = dawsome_engine::project::Channel::instrument("Sine B", "p22_sine/sine.sfz");
    project.channels.insert(ChannelId(2), b);
    project.next_channel_id = ChannelId(3);
    project.validate().expect("valid");
    project
}

/// p22_sine with a second channel, "Sine B" (2), playing the same set.
fn two_sines() -> Project {
    with_sine_b(load("p22_sine.json"))
}

/// One move of a device, handed in before the 512-frame call it is timed for: a control for a
/// channel, or a key struck there (velocity 1).
#[derive(Debug, Clone, Copy)]
enum Move {
    Control(ChannelId, LiveControl),
    Strike(ChannelId, u8),
}

/// `frames` frames of `project` with one device's `moves` (in order of their calls), each
/// handed in before the call it names; the calls cut as `sizes` says and also where a move is
/// taken, as [`render`] cuts them.
fn play_moves(
    project: &Project,
    moves: &[(usize, Move)],
    frames: usize,
    sizes: impl IntoIterator<Item = usize>,
) -> Vec<[f32; 2]> {
    let (link, mut player) = playing(project);
    let port = link.midi_inbox().for_port();
    let mut out = vec![[f32::NAN; 2]; frames];
    let mut sizes = sizes.into_iter();
    let (mut next, mut done) = (0, 0);
    while done < frames {
        while let Some(&(_, step)) = moves.get(next).filter(|(call, _)| call * CALL <= done) {
            let taken = match step {
                Move::Control(channel, control) => port.push_control(Some(channel), control, true),
                Move::Strike(channel, key) => {
                    port.push_to(channel, LiveNote::On { key, velocity: 1.0 })
                }
            };
            assert!(taken, "the inbox had room");
            next += 1;
        }
        let mut end = (done + sizes.next().expect("enough block sizes")).min(frames);
        if let Some(&(call, _)) = moves.get(next) {
            end = end.min(call * CALL);
        }
        player.process(&mut out[done..end]);
        done = end;
    }
    out
}

/// Channel A's (1) live A4 held for 40 calls, after `controls` were handed in for the channels
/// they name from one input (a call each): the note's frames.
fn a_note_after(project: &Project, controls: &[(ChannelId, LiveControl)]) -> Vec<[f32; 2]> {
    let mut moves: Vec<(usize, Move)> = controls
        .iter()
        .enumerate()
        .map(|(call, &(channel, control))| (call, Move::Control(channel, control)))
        .collect();
    let note = controls.len();
    moves.push((note, Move::Strike(ChannelId(1), 69)));
    let out = play_moves(project, &moves, (note + 40) * CALL, calls());
    out[note * CALL..].to_vec()
}

// Found 2026-10-26: a device's wheel and bend
// follow the channel its notes play, so a control moved back to rest after the selection moved
// lands on the new channel. Then it rests on every other channel the device held it off rest:
// A's next note plays as one no control ever touched, bit for bit (the case found: 20475 and
// 20479 of 20480 frames differed).
#[test]
fn a_wheel_or_bend_back_at_rest_on_another_channel_rests_the_one_left() {
    let project = two_sines();
    let (a, b) = (ChannelId(1), ChannelId(2));
    let untouched = a_note_after(&project, &[]);
    let wheel = a_note_after(
        &project,
        &[(a, LiveControl::Wheel(100)), (b, LiveControl::Wheel(0))],
    );
    assert!(wheel == untouched, "the wheel left on A");
    let bend = a_note_after(
        &project,
        &[(a, LiveControl::Bend(8191)), (b, LiveControl::Bend(0))],
    );
    assert!(bend == untouched, "the bend left on A");
    // Moved on B (not to rest), the control is B's alone: A rests, B takes it.
    let moved = a_note_after(
        &project,
        &[(a, LiveControl::Bend(8191)), (b, LiveControl::Bend(-3000))],
    );
    assert!(moved == untouched, "A rests when B takes the bend");
}

// Found 2026-10-26: the rest on the channel left gives the
// same bits however the calls are cut: the two cases found (the wheel at 100 or the bend at
// +8191 on A, back to rest on B, then A's note), and the controls hopping between A and B under
// a note sounding on each, so each rest re-bases a voice.
#[test]
fn a_rest_on_the_channel_left_gives_the_same_bits_however_the_calls_are_cut() {
    use LiveControl::{Bend, Wheel};
    use Move::{Control, Strike};
    let project = two_sines();
    let (a, b) = (ChannelId(1), ChannelId(2));
    let cases: [&[(usize, Move)]; 3] = [
        &[
            (0, Control(a, Wheel(100))),
            (1, Control(b, Wheel(0))),
            (2, Strike(a, 69)),
        ],
        &[
            (0, Control(a, Bend(8191))),
            (1, Control(b, Bend(0))),
            (2, Strike(a, 69)),
        ],
        &[
            (0, Strike(a, 69)),
            (1, Control(a, Wheel(100))),
            (2, Control(a, Bend(8191))),
            (3, Strike(b, 72)),
            (4, Control(b, Wheel(30))),
            (6, Control(b, Bend(-3000))),
            (8, Control(a, Bend(2000))),
            (10, Control(a, Wheel(0))),
        ],
    ];
    for moves in cases {
        let reference = play_moves(&project, moves, 40 * CALL, calls());
        for seed in [1, 0x9E37_79B9_7F4A_7C15] {
            let out = play_moves(&project, moves, 40 * CALL, block_sizes(seed));
            assert!(
                out.iter()
                    .map(|f| f.map(f32::to_bits))
                    .eq(reference.iter().map(|f| f.map(f32::to_bits))),
                "{moves:?}: a split render differs (seed {seed})"
            );
        }
    }
}

// The same on MIDI outs, by hand from the MIDI 1.0 bytes: the wheel and the bend moved on A
// (MIDI channel 1) go out there; moved on B (MIDI channel 2), they first rest on A's out
// (B0 01 00, E0 00 40), then go out on B's. A control already at rest on A sends no rest.
#[test]
fn a_control_moved_to_another_channel_rests_on_the_out_it_left() {
    let text = fs::read_to_string(fixtures().join("p22_out.json")).expect("p22_out");
    let mut file: Value = serde_json::from_str(&text).expect("parses");
    let mut b = file["channels"][0].clone();
    b["id"] = 2.into();
    b["name"] = "Synth B".into();
    b["midi_out"]["port"] = "Synth B:Synth B MIDI 1".into();
    b["midi_out"]["channel"] = 2.into();
    file["channels"].as_array_mut().expect("channels").push(b);
    file["next_channel_id"] = 3.into();
    let project = Project::load_json(&file.to_string()).expect("loads");
    let (mut link, mut player) = playing(&project);
    let mut reader = link.take_midi_out().expect("the ring");
    let port = link.midi_inbox().for_port();
    let (a, b) = (ChannelId(1), ChannelId(2));
    for (channel, control) in [
        (a, LiveControl::Wheel(100)),
        (a, LiveControl::Bend(8191)),
        (b, LiveControl::Wheel(50)),
        (b, LiveControl::Bend(100)),
        (a, LiveControl::Wheel(0)),
    ] {
        assert!(port.push_control(Some(channel), control, true));
        player.process(&mut [[0.0; 2]; CALL]);
    }
    assert_eq!(
        sent(&mut reader),
        [
            (0, 1, vec![0xB0, 1, 100]),
            (512, 1, vec![0xE0, 0x7F, 0x7F]),
            (1024, 1, vec![0xB0, 1, 0]),
            (1024, 2, vec![0xB1, 1, 50]),
            (1536, 1, vec![0xE0, 0x00, 0x40]),
            (1536, 2, vec![0xE1, 0x64, 0x40]),
            (2048, 2, vec![0xB1, 1, 0]),
        ]
    );
}
