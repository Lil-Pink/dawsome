//! The 4-point Hermite read in the player: pitched zones read across their loops' seams give the
//! frames tests/fixtures/gen_p25_expected.py computes from the rule it states
//! (p25_expected.json), however a render is split into blocks (1, a prime, 64, 1000, 4096, then
//! seeded sizes), without touching the heap.

use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::{ClipHint, ClipSound, Compiler, Sounds};
use dawsome_engine::dsp::instrument::{Instrument, ZoneSample};
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::Touched;
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::project::{PatternId, Project};
use dawsome_engine::realtime::{self, Player, RealtimeLink};
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
    let text = fs::read_to_string(fixtures().join("p25_expected.json"))
        .expect("p25_expected.json (run gen_p25_expected.py --write tests/fixtures)");
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

/// The project in `fixture`, compiled and playing pattern 1.
fn playing(fixture: &str) -> (RealtimeLink, Player) {
    let text = fs::read_to_string(fixtures().join(fixture)).expect("a p25 fixture");
    let project = Project::load_json(&text).expect("loads");
    let update: PlanUpdate = Compiler::new()
        .compile_with(
            &project,
            &Touched::default(),
            PlayMode::Pattern(PatternId(1)),
            &mut FixtureSets,
        )
        .expect("compiles");
    let (mut link, player) = realtime::link(update, project.tempo);
    link.send(Control::Play).expect("room");
    (link, player)
}

/// `frames` frames of `fixture`, each process call as long as `sizes` says.
fn render(fixture: &str, frames: usize, sizes: impl IntoIterator<Item = usize>) -> Vec<[f32; 2]> {
    let (_link, mut player) = playing(fixture);
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
    assert_eq!(done, frames, "enough block sizes");
    out
}

/// Edge-case sizes (1, a prime, 64, a round number, 4096), then seeded ones (xorshift64, so a
/// failure reproduces exactly).
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

/// The SHA-256 of `frames` as 32-bit floats, little-endian, left then right (the generator's).
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

// (5) A loop's seams, a sustain loop's offset inside its loop and its release after wrapping:
// the generator's frames in 512-frame calls and at seeded splits, bit for bit.
#[test]
fn p25_loops_read_the_same_frames_however_a_render_is_split() {
    let want = expected();
    for name in want["splits"]["cases"].as_array().expect("cases") {
        let name = name.as_str().expect("a case");
        let answer = &want["renders"][name];
        let fixture = answer["fixture"].as_str().expect("fixture");
        let frames = answer["frames"].as_u64().expect("frames") as usize;
        let whole = render(fixture, frames, std::iter::repeat(512));
        assert_eq!(
            frames_hash(&whole),
            answer["sha256"].as_str().expect("sha256"),
            "{name}: the generator's frames"
        );
        for seed in [1, 0x9E37_79B9_7F4A_7C15] {
            let split = render(fixture, frames, block_sizes(seed));
            assert!(
                split
                    .iter()
                    .map(|f| f.map(f32::to_bits))
                    .eq(whole.iter().map(|f| f.map(f32::to_bits))),
                "{name}: a split render differs (seed {seed})"
            );
        }
    }
}

// The audio thread never allocates: pitched zones reading across their seams and a pitched
// stereo sampler, a bar of 512-frame calls with no heap call.
#[test]
fn p25_pitched_reads_never_touch_the_heap() {
    for fixture in [
        "p25_loop.json",
        "p25_sustain.json",
        "p25_four.json",
        "p25_reverse.json",
    ] {
        let (_link, mut player) = playing(fixture);
        let mut buffer = vec![[0.0f32; 2]; 512];
        player.process(&mut buffer);
        let calls = heap_calls_during(|| {
            for _ in 0..172 {
                player.process(&mut buffer);
            }
        });
        assert_eq!(calls, 0, "{fixture}: the read touched the heap");
    }
}
