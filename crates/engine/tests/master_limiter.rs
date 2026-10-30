//! The master limiter in the engine: a project with the Limiter in the master's first slot,
//! played by `Player::process`, gives the frames tests/fixtures/gen_p24_expected.py computed
//! (p24_expected.json) however the calls are cut, without touching the heap.

use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::{ClipHint, ClipSound, Compiler, Sounds};
use dawsome_engine::dsp::instrument::{Instrument, ZoneSample};
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::Touched;
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::processor::Registry;
use dawsome_engine::project::{PatternId, Project};
use dawsome_engine::realtime::{self, Player};
use dawsome_engine::sfz;
use dawsome_engine::transport::Control;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p24_expected.json"))
        .expect("p24_expected.json (run gen_p24_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("parses")
}

fn wav(path: &Path) -> ZoneSample {
    let mut reader = hound::WavReader::open(path).expect("a test sample");
    let spec = reader.spec();
    let values: Vec<f32> = reader
        .samples::<f32>()
        .map(|value| value.expect("reads"))
        .collect();
    assert_eq!(spec.channels, 1, "the p24 sounds are mono");
    ZoneSample {
        buffer: SampleBuffer::mono(values),
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
    let text = fs::read_to_string(fixtures().join(name)).expect("a p24 fixture");
    Project::load_json(&text).expect("loads")
}

fn compiled(project: &Project) -> PlanUpdate {
    Compiler::new()
        .compile_with(
            project,
            &Touched::default(),
            PlayMode::Pattern(PatternId(1)),
            &mut FixtureSets,
        )
        .expect("compiles")
}

fn playing(project: &Project) -> (realtime::RealtimeLink, Player) {
    let (mut link, player) = realtime::link(compiled(project), project.tempo);
    link.send(Control::Play).expect("room");
    (link, player)
}

/// `frames` frames of `project`, each process call as long as `sizes` says.
fn render(
    project: &Project,
    frames: usize,
    sizes: impl IntoIterator<Item = usize>,
) -> Vec<[f32; 2]> {
    let (_link, mut player) = playing(project);
    let mut out = vec![[f32::NAN; 2]; frames];
    let mut done = 0;
    let mut sizes = sizes.into_iter();
    while done < frames {
        let end = (done + sizes.next().expect("enough block sizes")).min(frames);
        player.process(&mut out[done..end]);
        done = end;
    }
    out
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

// Known answers (gen_p24_expected.py): the burst, the chord and the spike through the player in
// 512-frame calls, then cut at seeded random sizes: the generator's hash either way.
#[test]
fn p24_the_limiter_gives_the_same_bits_however_the_calls_are_cut() {
    let want = expected();
    for name in ["burst", "chord", "spike", "hard"] {
        let answer = &want["renders"][name];
        let project = load(answer["fixture"].as_str().expect("fixture"));
        let frames = answer["frames"].as_u64().expect("frames") as usize;
        let sha = answer["sha256"].as_str().expect("sha256");
        let whole = render(&project, frames, std::iter::repeat(512));
        assert_eq!(frames_hash(&whole), sha, "{name} in 512-frame calls");
        for seed in [1, 0x9E37_79B9_7F4A_7C15] {
            let split = render(&project, frames, block_sizes(seed));
            assert_eq!(
                frames_hash(&split),
                sha,
                "{name}: a split render (seed {seed})"
            );
        }
    }
}

// The limiter's parameters and name are the spec's (gen_p24_expected.py restates them).
#[test]
fn p24_the_limiter_is_a_native_effect_with_the_specs_parameters() {
    let want = &expected()["limiter"];
    let registry = Registry::native();
    let factory = registry
        .get(want["id"].as_str().expect("id"))
        .expect("registered");
    assert_eq!(factory.id(), want["id"].as_str().expect("id"));
    assert_eq!(factory.name(), want["name"].as_str().expect("name"));
    let params = want["params"].as_array().expect("params");
    assert_eq!(factory.params().len(), params.len());
    for (info, param) in factory.params().iter().zip(params) {
        assert_eq!(info.key, param["key"].as_str().expect("key"));
        assert_eq!(
            info.default,
            param["default"].as_f64().expect("default") as f32
        );
        assert_eq!(f64::from(info.min), param["min"].as_f64().expect("min"));
        assert_eq!(f64::from(info.max), param["max"].as_f64().expect("max"));
        assert_eq!(info.unit, param["unit"].as_str().expect("unit"));
        assert!(!info.whole);
    }
}

// The audio thread never touches the heap, the limiter pulling its gain down included.
#[test]
fn the_limiters_path_never_touches_the_heap() {
    let project = load("p24_chord.json");
    let (_link, mut player) = playing(&project);
    let mut buffer = vec![[0.0f32; 2]; 512];
    player.process(&mut buffer);
    let calls = heap_calls_during(|| {
        for _ in 0..20 {
            player.process(&mut buffer);
        }
    });
    assert_eq!(calls, 0, "the limiter's path touched the heap");
    let peak = buffer
        .iter()
        .flatten()
        .fold(0.0f32, |peak, v| peak.max(v.abs()));
    assert!(peak > 0.5, "the chord was playing: {peak}");
}
