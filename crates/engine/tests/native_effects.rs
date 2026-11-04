//! The native effects in the engine: each one's id, name and parameters are what
//! tests/fixtures/gen_p32_expected.py pins (p32_expected.json); the p32 projects played by
//! `Player::process` give the same bits however the calls are cut (the generator's hash for the
//! exact tier); and the player runs them all on one insert without touching the heap.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::{ClipHint, ClipSound, Compiler, Sounds};
use dawsome_engine::dsp::instrument::Instrument;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::Touched;
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::processor::Registry;
use dawsome_engine::project::{InsertId, PatternId, Project};
use dawsome_engine::realtime::{self, Player};
use dawsome_engine::transport::Control;
use serde_json::value::RawValue;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

/// The effects these tests cover, by id: the seven of p32, in the add menu's order.
const EFFECTS: [&str; 7] = [
    "dawsome.filter",
    "dawsome.distortion",
    "dawsome.chorus",
    "dawsome.flanger",
    "dawsome.phaser",
    "dawsome.gate",
    "dawsome.width",
];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p32_expected.json"))
        .expect("p32_expected.json (run gen_p32_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("parses")
}

/// A p32 input file, mono or stereo.
fn wav(path: &Path) -> SampleBuffer {
    let mut reader = hound::WavReader::open(path).expect("a test sound");
    let channels = reader.spec().channels;
    let values: Vec<f32> = reader
        .samples::<f32>()
        .map(|value| value.expect("reads"))
        .collect();
    match channels {
        1 => SampleBuffer::mono(values),
        2 => SampleBuffer::stereo(values),
        other => panic!("{other} channels"),
    }
}

/// Sounds from the fixtures folder.
struct FixtureSounds;

impl Sounds for FixtureSounds {
    type Error = Infallible;

    fn sample(&mut self, path: &Path) -> Result<Option<SampleBuffer>, Infallible> {
        Ok(Some(wav(&fixtures().join(path))))
    }

    fn clip(&mut self, path: &Path, _hint: ClipHint) -> Result<ClipSound, Infallible> {
        Ok(ClipSound::Memory(wav(&fixtures().join(path))))
    }

    fn instrument(&mut self, _path: &Path) -> Result<Option<Instrument>, Infallible> {
        Ok(None)
    }
}

fn load(name: &str) -> Project {
    let text = fs::read_to_string(fixtures().join(name)).expect("a p32 fixture");
    Project::load_json(&text).expect("loads")
}

fn playing(project: &Project) -> (realtime::RealtimeLink, Player) {
    let update: PlanUpdate = Compiler::new()
        .compile_with(
            project,
            &Touched::default(),
            PlayMode::Pattern(PatternId(1)),
            &mut FixtureSounds,
        )
        .expect("compiles");
    let (mut link, player) = realtime::link(update, project.tempo);
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

fn number(value: &Value) -> f64 {
    value.as_f64().expect("a number")
}

/// The number at `path` (object keys and array places) under `raw`, read from the text the
/// generator wrote by std's correctly rounded parser. Why not `Value::as_f64` where a test
/// compares an f64 exactly: serde_json's default parser reads some 17-digit numbers an ulp off
/// (the phaser's center at 1, 3999.9999999999973, comes back as 3999.9999999999977).
fn exact(raw: &RawValue, path: &[&str]) -> f64 {
    let mut at = raw;
    for step in path {
        at = match step.parse::<usize>() {
            Ok(place) => serde_json::from_str::<Vec<&RawValue>>(at.get()).expect("an array")[place],
            Err(_) => serde_json::from_str::<BTreeMap<&str, &RawValue>>(at.get())
                .expect("an object")[step],
        };
    }
    at.get().parse().expect("a number")
}

// Known answers (gen_p32_expected.py restates the spec's tables): each effect's id, name, and
// every parameter's key, label, range, default, unit and whole flag; the names a choice's values
// go by; and what a log control's 0, 0.5 and 1 stand for (exp2 alone, so exact).
#[test]
fn p32_each_effect_has_the_specs_parameters() {
    let want = &expected()["effects"];
    let text = fs::read_to_string(fixtures().join("p32_expected.json")).expect("the answers");
    let root: BTreeMap<&str, &RawValue> = serde_json::from_str(&text).expect("parses");
    let effects: BTreeMap<&str, &RawValue> =
        serde_json::from_str(root["effects"].get()).expect("the effects");
    let registry = Registry::native();
    for id in EFFECTS {
        let spec = &want[id];
        let raw = effects[id];
        let factory = registry.get(id).expect("registered");
        assert_eq!(factory.name(), spec["name"].as_str().expect("name"), "{id}");
        let params = spec["params"].as_array().expect("params");
        assert_eq!(factory.params().len(), params.len(), "{id}");
        for (index, (info, param)) in factory.params().iter().zip(params).enumerate() {
            let key = param["key"].as_str().expect("key");
            let place = index.to_string();
            assert_eq!(info.key, key, "{id}");
            assert_eq!(
                info.label,
                param["label"].as_str().expect("label"),
                "{id} {key}"
            );
            let bound = |end| exact(raw, &["params", &place, end]);
            assert_eq!(f64::from(info.min), bound("min"), "{id} {key}");
            assert_eq!(f64::from(info.max), bound("max"), "{id} {key}");
            assert_eq!(info.default, number(&param["default"]) as f32, "{id} {key}");
            assert_eq!(
                info.unit,
                param["unit"].as_str().expect("unit"),
                "{id} {key}"
            );
            assert_eq!(Some(info.whole), param["whole"].as_bool(), "{id} {key}");
            match spec["value_names"][key].as_array() {
                Some(names) => {
                    for (value, name) in names.iter().enumerate() {
                        assert_eq!(
                            factory.value_name(index, value as f32),
                            name.as_str(),
                            "{id} {key} at {value}"
                        );
                    }
                }
                None => assert_eq!(factory.value_name(index, info.default), None, "{id} {key}"),
            }
            match spec["shown"][key].as_array() {
                Some(points) => {
                    for (at, point) in points.iter().enumerate() {
                        let value = number(&point[0]) as f32;
                        let shown = exact(raw, &["shown", key, &at.to_string(), "1"]);
                        assert_eq!(
                            factory.shown(index, value, 44_100),
                            shown,
                            "{id} {key} at {value}"
                        );
                    }
                }
                None => assert_eq!(
                    factory.shown(index, info.max, 44_100),
                    f64::from(info.max),
                    "{id} {key}"
                ),
            }
        }
    }
}

// Known answers (gen_p32_expected.py): every case of each effect here, through the player in
// 512-frame calls and then cut at seeded random sizes, gives the same bits; the exact tier's are
// the generator's hash.
#[test]
fn p32_the_effects_give_the_same_bits_however_the_calls_are_cut() {
    let want = expected();
    let frames = usize::try_from(want["frames"].as_u64().expect("frames")).expect("fits");
    for id in EFFECTS {
        let cases: Vec<(&String, &Value)> = want["cases"]
            .as_object()
            .expect("cases")
            .iter()
            .filter(|(_, answer)| answer["effect"].as_str() == Some(id))
            .collect();
        assert!(!cases.is_empty(), "{id}: no cases");
        for (name, answer) in cases {
            let project = load(answer["fixture"].as_str().expect("fixture"));
            let whole = frames_hash(&render(&project, frames, std::iter::repeat(512)));
            if answer["tier"] == "exact" {
                assert_eq!(whole, answer["sha256"].as_str().expect("sha256"), "{name}");
            }
            for seed in [1, 0x9E37_79B9_7F4A_7C15] {
                let split = frames_hash(&render(&project, frames, block_sizes(seed)));
                assert_eq!(split, whole, "{name}: a split render (seed {seed})");
            }
        }
    }
}

// The audio thread never touches the heap with every effect here on one insert, in order.
#[test]
fn the_effects_path_never_touches_the_heap() {
    let mut document: Value =
        serde_json::from_str(&fs::read_to_string(fixtures().join("p32_dry_stereo.json")).unwrap())
            .expect("parses");
    let effects: Vec<Value> = EFFECTS
        .iter()
        .enumerate()
        .map(|(slot, id)| serde_json::json!({"id": slot + 1, "slot": slot, "processor": {"id": id, "params": {}}}))
        .collect();
    let mut inserts = vec![
        serde_json::json!({"id": 0, "name": "Master", "fader": 0.8, "pan": 0.0,
        "mute": false, "routes": [], "effects": []}),
    ];
    inserts.push(
        serde_json::json!({"id": 1, "name": "Insert 1", "fader": 0.8, "pan": 0.0,
        "mute": false, "routes": [{"to": 0, "amount": 1.0}], "effects": effects}),
    );
    document["mixer"] = serde_json::json!({"next_effect_id": EFFECTS.len() + 1, "solo": [],
        "inserts": inserts});
    let project = Project::load_json(&document.to_string()).expect("loads");
    let insert = project.mixer.get(InsertId(1)).expect("insert 1");
    assert_eq!(
        insert.effects.len(),
        EFFECTS.len(),
        "every effect on insert 1"
    );
    let (_link, mut player) = playing(&project);
    let mut buffer = vec![[0.0f32; 2]; 512];
    player.process(&mut buffer);
    let calls = heap_calls_during(|| {
        for _ in 0..20 {
            player.process(&mut buffer);
        }
    });
    assert_eq!(calls, 0, "the effects' path touched the heap");
    let peak = buffer
        .iter()
        .flatten()
        .fold(0.0f32, |peak, v| peak.max(v.abs()));
    assert!(peak > 0.05, "the input was playing: {peak}");
}
