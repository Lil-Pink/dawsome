//! A channel's sound settings in the engine: the player renders each case of
//! tests/fixtures/gen_p13_expected.py to the frames it states (p13_expected.json), however a
//! render is split into blocks and without touching the heap; the settings round-trip through
//! the file, a project without them keeps its bytes and version, and the edit that sets them
//! is undone by its inverse. Expected values come from the generator, never from the engine.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::{ClipHint, ClipSound, Compiler, Sounds};
use dawsome_engine::dsp::instrument::{Instrument, ZoneSample};
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, Edit, EditError, Touched};
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::project::{
    Channel, ChannelId, ChannelKind, Gate, LoadError, Note, PatternId, Project, Sound,
    ValidationError, FORMAT_VERSION, INSTRUMENT_FORMAT_VERSION, SOUND_FORMAT_VERSION,
};
use dawsome_engine::realtime::{self, Player, RealtimeLink};
use dawsome_engine::sfz;
use dawsome_engine::time::Ticks;
use dawsome_engine::transport::Control;
use serde_json::Value;

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p13_expected.json"))
        .expect("p13_expected.json (run gen_p13_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("parses")
}

fn wav(path: &Path) -> ZoneSample {
    let mut reader = hound::WavReader::open(path).expect("a test sound");
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

/// Sounds from the fixtures folder: samples, and instrument sets built at 44.1 kHz from the
/// WAVs beside their .sfz.
struct Fixtures;

impl Sounds for Fixtures {
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
        let read = sfz::parse("set.sfz", &text, &mut |include| {
            fs::read_to_string(dir.join(include)).map_err(|error| error.to_string())
        });
        let (instrument, problems) =
            Instrument::build(&read.regions, |sample| wav(&dir.join(sample)), 44_100);
        assert_eq!(problems, Vec::<String>::new());
        Ok(Some(instrument))
    }
}

fn load(name: &str) -> (String, Project) {
    let text = fs::read_to_string(fixtures().join(name)).expect("a p13 fixture");
    let project = Project::load_json(&text).expect("loads");
    (text, project)
}

fn compiled(project: &Project) -> PlanUpdate {
    let mut compiler = Compiler::new();
    compiler.set_live_channel(project.channels.keys().next().copied());
    compiler
        .compile_with(
            project,
            &Touched::default(),
            PlayMode::Pattern(PatternId(1)),
            &mut Fixtures,
        )
        .expect("compiles")
}

fn playing(project: &Project) -> (RealtimeLink, Player) {
    let (mut link, player) = realtime::link(compiled(project), project.tempo);
    link.send(Control::Play).expect("room");
    (link, player)
}

/// `frames` frames of `player`, one `process` call per size in `sizes`.
fn render_player(
    player: &mut Player,
    frames: usize,
    sizes: impl IntoIterator<Item = usize>,
) -> Vec<[f32; 2]> {
    let mut out = vec![[f32::NAN; 2]; frames];
    let mut done = 0;
    for size in sizes {
        if done == frames {
            break;
        }
        let end = (done + size).min(frames);
        player.process(&mut out[done..end]);
        done = end;
    }
    assert_eq!(done, frames, "ran out of block sizes");
    out
}

/// Edge-case sizes (1, a prime, 64, a round number, 4096, a bar and more), then seeded ones.
fn block_sizes(seed: u64) -> impl Iterator<Item = usize> {
    let mut state = seed;
    let random = std::iter::repeat_with(move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        usize::try_from(state % 5000).expect("below 5000") + 1
    });
    [1, 7, 64, 1000, 4096, 100_000].into_iter().chain(random)
}

/// Asserts `out` is the expected render of `want`: every sounding frame to the bit and all else
/// silence; for the filter, its probes to the case's tolerance and its sounding count.
fn assert_render(name: &str, out: &[[f32; 2]], want: &Value) {
    assert_eq!(
        out.len() as u64,
        want["frames"].as_u64().expect("frames"),
        "{name}"
    );
    if let Some(tolerance) = want["tolerance"].as_f64() {
        for probe in want["probes"].as_array().expect("probes") {
            let at = probe[0].as_u64().expect("a frame") as usize;
            for side in 0..2 {
                let wanted = probe[side + 1].as_f64().expect("a value");
                let got = f64::from(out[at][side]);
                assert!(
                    (got - wanted).abs() <= tolerance,
                    "{name}, frame {at}: {got} != {wanted}"
                );
            }
        }
        let sounding = out.iter().filter(|frame| frame[0] != 0.0).count();
        assert_eq!(
            sounding as u64,
            want["sounding"].as_u64().expect("count"),
            "{name}"
        );
        return;
    }
    let mut sounding: BTreeMap<usize, [f32; 2]> = BTreeMap::new();
    for entry in want["nonzero"].as_array().expect("a list") {
        let at = entry[0].as_u64().expect("a frame") as usize;
        let value = |side: usize| entry[side].as_f64().expect("a value") as f32;
        sounding.insert(at, [value(1), value(2)]);
    }
    assert!(!sounding.is_empty(), "{name}: the answer sounds");
    for (at, got) in out.iter().enumerate() {
        let want = sounding.get(&at).copied().unwrap_or([0.0; 2]);
        assert_eq!(
            got.map(f32::to_bits),
            want.map(f32::to_bits),
            "{name}, frame {at}: {got:?} != {want:?}"
        );
    }
}

/// Known answers (gen_p13_expected.py): semitones and cents tune the speed, the envelope rises
/// and falls in decibels after a held note's end, the velocity's weight lifts a soft note, a
/// sampler plays backwards and from half way, a one-shot plays through its note's end and a
/// held note is cut there, the low-pass filters a step, and an instrument set plays tuned with
/// its release replaced. Each whole, and split into blocks of edge-case and seeded sizes.
#[test]
fn p13_sound_settings_render_their_known_answers_however_split() {
    let want = expected();
    for (name, answer) in want["renders"].as_object().expect("renders") {
        let (_, project) = load(answer["fixture"].as_str().expect("fixture"));
        let frames = answer["frames"].as_u64().expect("frames") as usize;
        let (_link, mut player) = playing(&project);
        let whole = render_player(&mut player, frames, [frames]);
        assert_render(name, &whole, answer);
        for seed in [5, 0xbeef] {
            let (_link, mut player) = playing(&project);
            let split = render_player(&mut player, frames, block_sizes(seed));
            assert!(
                whole
                    .iter()
                    .zip(&split)
                    .all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits)),
                "{name}: a split render differs (seed {seed})"
            );
        }
    }
}

/// Every p13 project reads and writes back to its own bytes, as format version 5.
#[test]
fn sound_settings_round_trip_as_format_version_5() {
    for name in ["envelope", "filter", "reverse", "set"] {
        let (text, project) = load(&format!("p13_{name}.json"));
        assert_eq!(
            project.to_json().expect("writes"),
            text.trim_end(),
            "{name}"
        );
        assert!(text.starts_with(&format!(r#"{{"format_version":{SOUND_FORMAT_VERSION},"#)));
    }
}

/// Migration: a version 2 or 4 file reads with every channel at the default settings and
/// writes back as it was; giving one channel settings writes version 5, and taking them back
/// to the defaults writes the old version and bytes again. A file asking for a setting out of
/// range is refused by name.
#[test]
fn files_before_sound_settings_keep_their_version_and_bytes() {
    for (name, version) in [
        ("f1_four_on_floor.json", 1),
        ("p2_three_patterns.json", FORMAT_VERSION),
        ("p11_pad.json", INSTRUMENT_FORMAT_VERSION),
    ] {
        let (text, mut project) = load(name);
        assert!(project
            .channels
            .values()
            .all(|channel| channel.sound.is_default()));
        let before = project.to_json().expect("writes");
        if version > 1 {
            assert_eq!(before, text.trim_end(), "{name} keeps its bytes");
        }
        let channel = *project.channels.keys().next().expect("a channel");
        let kind = project.channels[&channel].kind;
        let sound = Sound {
            cents: 7,
            ..Sound::default()
        };
        let applied =
            edit::apply(&mut project, Edit::SetChannelSound { channel, sound }).expect("applies");
        let shaped = project.to_json().expect("writes");
        assert!(
            shaped.starts_with(r#"{"format_version":5,"#),
            "{name}: {shaped}"
        );
        assert_eq!(Project::load_json(&shaped).expect("reads back"), project);
        edit::apply(&mut project, applied.inverse).expect("undoes");
        assert_eq!(project.to_json().expect("writes"), before, "{name}: undone");
        assert_eq!(project.channels[&channel].kind, kind);
    }
    let (text, _) = load("p13_hold.json");
    let loud = text.replace(r#""velocity":1.0,"attack""#, r#""velocity":1.5,"attack""#);
    assert!(matches!(
        Project::load_json(&loud),
        Err(LoadError::Invalid(ValidationError::SoundSetting {
            setting: "velocity",
            ..
        }))
    ));
}

/// The edit refuses settings out of range and ones a channel's kind cannot take, changing
/// nothing; its inverse gives the old settings back exactly.
#[test]
fn the_sound_edit_refuses_what_a_channel_cannot_take() {
    let (_, mut project) = load("p13_set.json");
    let before = project.clone();
    let channel = ChannelId(1);
    let reverse = Sound {
        reverse: true,
        ..Sound::default()
    };
    assert_eq!(
        edit::apply(
            &mut project,
            Edit::SetChannelSound {
                channel,
                sound: reverse
            }
        ),
        Err(EditError::InvalidSound {
            channel: 1,
            setting: "reverse"
        })
    );
    let sharp = Sound {
        semitones: 13,
        ..Sound::default()
    };
    assert!(edit::apply(
        &mut project,
        Edit::SetChannelSound {
            channel,
            sound: sharp
        }
    )
    .is_err());
    assert_eq!(project, before);
    let hold = project.channels[&channel].sound;
    assert_eq!(hold.gate_for(ChannelKind::Instrument), Gate::Hold);
    let one_shot = hold.with_gate(Gate::OneShot, ChannelKind::Instrument);
    let applied = edit::apply(
        &mut project,
        Edit::SetChannelSound {
            channel,
            sound: one_shot,
        },
    )
    .expect("applies");
    assert_eq!(
        applied.inverse,
        Edit::SetChannelSound {
            channel,
            sound: hold
        }
    );
}

/// A busy project of shaped voices: a filtered, tuned, reversed sampler held with a release and
/// the pad set filtered and released, notes on every step, live notes on and off, plans
/// swapped, Stop and Play: `Player::process` never touches the heap.
#[test]
fn process_never_touches_the_heap_with_shaped_voices_playing() {
    let control = heap_calls_during(|| drop(std::hint::black_box(vec![0u8; 64])));
    assert!(control > 0, "the counting allocator is not counting");
    let (_, mut project) = load("p13_filter.json");
    let sampler = project
        .channels
        .get_mut(&ChannelId(1))
        .expect("the sampler");
    sampler.sound = Sound {
        gate: Some(Gate::Hold),
        semitones: -5,
        cents: 30,
        release: 0.05,
        reverse: true,
        start: 0.25,
        filter: true,
        cutoff: 800.0,
        resonance: 0.7,
        ..Sound::default()
    };
    let mut set = Channel::instrument("Pad", "instruments/pad/pad.sfz");
    set.sound = Sound {
        attack: 0.01,
        release: 0.1,
        filter: true,
        cutoff: 2000.0,
        ..Sound::default()
    };
    project.channels.insert(ChannelId(2), set);
    project.next_channel_id = ChannelId(3);
    let pattern = project.patterns.get_mut(&PatternId(1)).expect("pattern 1");
    pattern.notes = (0..16)
        .flat_map(|step| {
            (1..=2u32).map(move |channel| Note {
                channel: ChannelId(channel),
                start: Ticks(i64::from(step) * 24),
                length: Ticks(30),
                velocity: 0.3 + 0.2 * channel as f32,
                key: 48 + u8::try_from((step * 5 + channel) % 30).expect("small"),
            })
        })
        .collect();
    project.validate().expect("valid");
    let (mut link, mut player) = playing(&project);
    let mut out = vec![[0.0; 2]; 512];
    let mut calls = 0;
    for call in 0..600 {
        if call % 100 == 50 {
            link.push_plan(compiled(&project)).expect("same time base");
        }
        if call % 7 == 3 {
            let key = 60 + (call % 12) as u8;
            link.send(Control::LiveNoteOn { key, velocity: 0.8 })
                .expect("room");
        }
        if call % 7 == 5 {
            let key = 60 + ((call - 2) % 12) as u8;
            link.send(Control::LiveNoteOff { key }).expect("room");
        }
        if call == 300 {
            link.send(Control::Stop).expect("room");
        }
        if call == 350 {
            link.send(Control::Play).expect("room");
        }
        calls += heap_calls_during(|| player.process(&mut out));
        link.drain_retired();
    }
    assert_eq!(calls, 0, "Player::process made {calls} heap calls");
}
