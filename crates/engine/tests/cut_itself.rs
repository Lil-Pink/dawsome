//! Cut itself in the engine: a note start on a channel that cuts itself ends the notes the
//! channel still plays, in the pattern's pools and the live ones, under the sustain pedal too,
//! as tests/fixtures/gen_p26_expected.py computed from the rules it states (p26_expected.json);
//! the same bits however a render is split into blocks, without touching the heap; and the
//! setting read and written as the file holds it.
//!
//! Live messages reach the player as a MIDI port's would, each before the 512-frame call its
//! file times it for; the bytes are read by MIDI 1.0's status bytes (9n note on, 8n note off, Bn
//! 40 the pedal), a restatement in the test, never the app's parser (the app's own test runs the
//! pedal's file through its door).

use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::{ClipHint, ClipSound, Compiler, Sounds};
use dawsome_engine::dsp::instrument::{Instrument, ZoneSample};
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::Touched;
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::project::{PatternId, Project, SOUND_FORMAT_VERSION};
use dawsome_engine::realtime::{self, EngineEvent, LiveControl, LiveNote, MidiInbox};
use dawsome_engine::sfz;
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
    let text = fs::read_to_string(fixtures().join("p26_expected.json"))
        .expect("p26_expected.json (run gen_p26_expected.py --write tests/fixtures)");
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
    let text = fs::read_to_string(fixtures().join(name)).expect("a p26 fixture");
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

/// One message of an events file: the call it comes before and its bytes.
type Timed = (u64, Vec<u8>);

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
            (event["call"].as_u64().expect("a call"), bytes)
        })
        .collect()
}

/// Hands one message to `inbox` as a port routed to the selected channel would. MIDI 1.0's
/// status bytes, restated: 9n kk vv a note on (vv 0 a note off), 8n kk vv a note off, Bn 40 vv
/// the pedal.
fn deliver(inbox: &MidiInbox, bytes: &[u8]) {
    let taken = match *bytes {
        [status, key, 0] if status & 0xF0 == 0x90 => inbox.push(LiveNote::Off { key }),
        [status, key, velocity] if status & 0xF0 == 0x90 => inbox.push(LiveNote::On {
            key,
            velocity: f32::from(velocity) / 128.0,
        }),
        [status, key, _] if status & 0xF0 == 0x80 => inbox.push(LiveNote::Off { key }),
        [status, 64, value] if status & 0xF0 == 0xB0 => {
            inbox.push_control(None, LiveControl::Sustain(value), true)
        }
        _ => panic!("a message the cases do not send: {bytes:02X?}"),
    };
    assert!(taken, "the inbox had room");
}

/// Plays `frames` frames of `project` with `script`'s messages, each process call `sizes` says
/// long but cut where a message is taken (on its call's first frame), so the messages reach the
/// player exactly where 512-frame calls would hand them over. Returns the frames and every event
/// the player posted.
fn render(
    project: &Project,
    script: &[Timed],
    frames: usize,
    sizes: impl IntoIterator<Item = usize>,
) -> (Vec<[f32; 2]>, Vec<EngineEvent>) {
    let (mut link, mut player) = realtime::link(compiled(project), project.tempo);
    link.send(Control::Play).expect("room");
    let inbox = link.midi_inbox();
    let mut out = vec![[f32::NAN; 2]; frames];
    let mut events = Vec::new();
    let mut next = 0;
    let mut done = 0;
    let mut sizes = sizes.into_iter();
    let hand_in = |done: usize, next: &mut usize| {
        while let Some((call, bytes)) = script.get(*next) {
            if usize::try_from(*call).expect("a call") * CALL > done {
                break;
            }
            deliver(&inbox, bytes);
            *next += 1;
        }
    };
    hand_in(0, &mut next);
    while done < frames {
        let size = sizes.next().expect("enough block sizes");
        let mut end = (done + size).min(frames);
        if let Some((call, _)) = script.get(next) {
            let at = usize::try_from(*call).expect("a call") * CALL;
            if at > done {
                end = end.min(at);
            }
        }
        player.process(&mut out[done..end]);
        events.extend(link.take_events());
        done = end;
        hand_in(done, &mut next);
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

/// Asserts `frames` is render `name` of the generator: its length, its probes to the bit, its
/// sounding frames and the hash of every frame.
fn assert_render(name: &str, frames: &[[f32; 2]], want: &Value) {
    assert_eq!(
        frames.len() as u64,
        want["frames"].as_u64().expect("frames"),
        "{name}"
    );
    for probe in want["probes"].as_array().expect("probes") {
        let at = probe[0].as_u64().expect("a frame") as usize;
        let side = |index: usize| probe[index].as_f64().expect("a value") as f32;
        assert_eq!(
            frames[at].map(f32::to_bits),
            [side(1), side(2)].map(f32::to_bits),
            "{name}, frame {at}: {:?}",
            frames[at]
        );
    }
    let sounding = frames.iter().filter(|frame| **frame != [0.0; 2]).count();
    assert_eq!(
        sounding as u64,
        want["sounding"].as_u64().expect("a count"),
        "{name}: sounding frames"
    );
    assert_eq!(
        frames_hash(frames),
        want["sha256"].as_str().expect("a hash"),
        "{name}: the frames differ from the generator's"
    );
}

/// The case `name` of the generator's `group` ("live" or "renders"): its project, its messages
/// (none for a pattern case) and its frames.
fn case(group: &str, name: &str) -> (Value, Project, Vec<Timed>, usize) {
    let want = expected()[group][name].clone();
    let project = load(want["fixture"].as_str().expect("fixture"));
    let script = want["events"].as_str().map(events).unwrap_or_default();
    let frames = want["frames"].as_u64().expect("frames") as usize;
    (want, project, script, frames)
}

/// Renders case `name` of `group` in 512-frame calls, checked; its frames and events.
fn checked(group: &str, name: &str) -> (Vec<[f32; 2]>, Vec<EngineEvent>) {
    let (want, project, script, frames) = case(group, name);
    let (out, events) = render(&project, &script, frames, calls());
    assert_render(name, &out, &want);
    (out, events)
}

/// Asserts case `name` of `group` gives the same bits however its calls are cut between the
/// messages.
fn split_invariant(group: &str, name: &str, reference: &[[f32; 2]]) {
    let (_, project, script, frames) = case(group, name);
    for seed in [1, 0x9E37_79B9_7F4A_7C15] {
        let (out, _) = render(&project, &script, frames, block_sizes(seed));
        assert!(
            out.iter()
                .map(|f| f.map(f32::to_bits))
                .eq(reference.iter().map(|f| f.map(f32::to_bits))),
            "{name}: a split render differs (seed {seed})"
        );
    }
}

/// The (key, frame started, frame ended) of each live note as the player reports it, in the
/// order they ended.
fn notes_heard(events: &[EngineEvent]) -> Vec<(u8, u64, u64)> {
    let mut started = Vec::new();
    let mut heard = Vec::new();
    for event in events {
        match *event {
            EngineEvent::LiveNoteStarted { key, frame, .. } => started.push((key, frame)),
            EngineEvent::LiveNoteEnded { key, frame, .. } => {
                let at = started
                    .iter()
                    .position(|&(on, _)| on == key)
                    .expect("ended after it started");
                let (_, on) = started.remove(at);
                heard.push((key, on, frame));
            }
            _ => {}
        }
    }
    heard
}

fn heard_of(want: &Value) -> Vec<(u8, u64, u64)> {
    want["heard"]
        .as_array()
        .expect("heard")
        .iter()
        .map(|note| {
            let at = |index: usize| note[index].as_u64().expect("a number");
            (u8::try_from(at(0)).expect("a key"), at(1), at(2))
        })
        .collect()
}

// Pedal down, note on, its note-off held by the pedal, another note: the held note falls over
// the release from the second's start (frame 10240), where without the cut it sounds to the lift
// (frame 20480). The notes as played, which a take records, are the same either way: the cut is
// playback alone. However the calls are cut between the messages, the same bits.
#[test]
fn p26_a_note_the_pedal_holds_is_cut_and_the_notes_as_played_stay() {
    let (cut, cut_events) = checked("live", "pedal");
    let (_, whole_events) = checked("live", "pedal_nocut");
    let want = &expected()["live"]["pedal"];
    assert_eq!(want["cuts"][0], serde_json::json!([2048, 8192]));
    assert_eq!(notes_heard(&cut_events), heard_of(want));
    assert_eq!(notes_heard(&cut_events), notes_heard(&whole_events));
    split_invariant("live", "pedal", &cut);
}

// Both ways between the pools: a live note held from frame 512 is cut by a pattern note at tick
// 24 (frame 5512), which a live note at frame 10240 cuts in turn; split anywhere, the same bits.
#[test]
fn p26_a_pattern_note_and_a_live_note_cut_each_other() {
    let (cross, _) = checked("live", "cross");
    assert_eq!(
        expected()["live"]["cross"]["cuts"],
        serde_json::json!([[512, 5000], [5512, 4728]])
    );
    split_invariant("live", "cross", &cross);
}

// A live key struck again on a sampler that cuts itself: the first note falls over 2 ms from the
// second's start instead of ending on one frame, and the second starts in a slot of its own.
#[test]
fn p26_a_live_key_struck_again_is_cut_not_replaced() {
    let (restrike, _) = checked("live", "live_restrike");
    split_invariant("live", "live_restrike", &restrike);
}

// A pattern case through the player at seeded block sizes: the cut lands on its frame whatever
// the split (the command line's tests take blocks of 1 and 7).
#[test]
fn p26_a_pattern_cut_is_the_same_bits_at_seeded_splits() {
    for name in ["overlap", "chord", "sampler"] {
        let (reference, _) = checked("renders", name);
        split_invariant("renders", name, &reference);
    }
}

// A note cut on a pattern (sampler and set), a live note cut by another and under the pedal,
// and a pattern note cut by a live one: no heap call on the audio thread.
#[test]
fn the_cuts_path_never_touches_the_heap() {
    for name in ["p26_overlap.json", "p26_sampler.json", "p26_cross.json"] {
        let project = load(name);
        let (mut link, mut player) = realtime::link(compiled(&project), project.tempo);
        link.send(Control::Play).expect("room");
        let port = link.midi_inbox().for_port();
        let mut buffer = vec![[0.0f32; 2]; CALL];
        player.process(&mut buffer);
        let calls = heap_calls_during(|| {
            for call in 1..48 {
                match call {
                    4 => assert!(port.push_control(None, LiveControl::Sustain(127), true)),
                    6 => assert!(port.push(LiveNote::On {
                        key: 64,
                        velocity: 1.0
                    })),
                    8 => assert!(port.push(LiveNote::Off { key: 64 })),
                    10 | 12 => assert!(port.push(LiveNote::On {
                        key: 67,
                        velocity: 1.0
                    })),
                    14 => assert!(port.push_control(None, LiveControl::Sustain(0), true)),
                    _ => {}
                }
                player.process(&mut buffer);
            }
            assert!(port.release());
            player.process(&mut buffer);
        });
        assert_eq!(calls, 0, "{name}: the cut's path touched the heap");
    }
}

// The setting as the file holds it: every p26 project reads and writes back byte for byte, at
// the sound settings' format version; "cut_itself" is written only when on (true), so a channel
// without it keeps its bytes.
#[test]
fn cut_itself_is_written_only_when_on_and_reads_back() {
    let mut read = 0;
    for entry in fs::read_dir(fixtures()).expect("the fixtures") {
        let name = entry.expect("an entry").file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("p26_") && name.ends_with(".json"))
            || name.ends_with("_events.json")
            || name == "p26_expected.json"
        {
            continue;
        }
        let text = fs::read_to_string(fixtures().join(&*name)).expect("a fixture");
        assert_eq!(
            load(&name).to_json().expect("writes") + "\n",
            text,
            "{name}"
        );
        assert_eq!(
            text.contains(r#""cut_itself":true"#),
            !name.contains("nocut"),
            "{name}"
        );
        assert!(!text.contains(r#""cut_itself":false"#), "{name}");
        assert!(
            text.starts_with(&format!(r#"{{"format_version":{SOUND_FORMAT_VERSION},"#)),
            "{name}"
        );
        read += 1;
    }
    assert_eq!(read, 14, "every case's project");
}
