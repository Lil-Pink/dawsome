//! Dawsome Synth against tests/fixtures/gen_p33_expected.py's known answers (p33_expected.json):
//! the pattern cases through the command line (`dawsome render` to 32-bit float), the live ones
//! (the sustain pedal, pitch bend, the mod wheel) as MIDI messages handed to the player beside a
//! synth channel, as a MIDI port's are (gen_p22's door); every case checked as its "asserts"
//! list says. Then
//! what holds whatever the case: the parameters gen_p33 lists, the same bits however the player
//! is called, no heap touched on the audio path, the synth channel's file format and the version
//! an older build refuses it by, a parameter edit reaching the running synth, and a project at
//! another rate getting its processors made again.

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use std::collections::BTreeMap;
use std::f64::consts::TAU;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use counting_allocator::heap_calls_during;
use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, Edit, Touched};
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::processor::Registry;
use dawsome_engine::project::{
    AutomationTarget, Channel, ChannelId, ChannelKind, LoadError, PatternId, Project,
    SYNTH_FORMAT_VERSION,
};
use dawsome_engine::realtime::{self, LiveControl, LiveNote, MidiInbox, Player, RealtimeLink};
use dawsome_engine::time::MilliBpm;
use dawsome_engine::transport::Control;
use serde_json::value::RawValue;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// The cases this test plays, by name in p33_expected.json: the projects the command line
/// renders, then the live cases. Every case of the answers is here
/// (`p33_every_case_of_the_answers_is_played`).
const CASES: [&str; 26] = [
    "sine",
    "adsr",
    "steal",
    "saw",
    "square_width",
    "tuning",
    "unison",
    "unison_tones",
    "noise",
    "filter",
    "cutoff_tones",
    "lfo_pitch",
    "lfo_cutoff",
    "glide",
    "glide_separated",
    "repeat",
    "chord",
    "drive_half",
    "drive_full",
    "drive_tone",
    "drive_zero",
    "pedal",
    "held",
    "unreleased",
    "bend",
    "wheel",
];

/// Frames per call, as the live cases' messages are timed.
const CALL: usize = 512;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> PathBuf {
    repo_root().join("tests/fixtures")
}

/// Fresh empty directory under cargo's per-target test directory.
fn temp_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("synth")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn answers_text() -> String {
    fs::read_to_string(fixtures().join("p33_expected.json"))
        .expect("p33_expected.json (run gen_p33_expected.py --write tests/fixtures)")
}

fn expected() -> Value {
    serde_json::from_str(&answers_text()).expect("parses")
}

fn load(name: &str) -> Project {
    let text = fs::read_to_string(fixtures().join(name)).expect("a p33 fixture");
    Project::load_json(&text).expect("loads")
}

fn number(value: &Value) -> f64 {
    value.as_f64().expect("a number")
}

fn index(value: &Value) -> usize {
    usize::try_from(value.as_u64().expect("a frame")).expect("fits")
}

/// An f32 as the generator writes it: the shortest digits that read back as that f32.
fn sample(value: &Value) -> f32 {
    number(value) as f32
}

/// `dawsome render <project> <dir>/<name>.wav --bits 32f <args>`, every per-user folder under
/// `dir`; the frames it wrote, left and right.
fn render_cli(project: &Path, dir: &Path, name: &str, args: &[&str]) -> Vec<[f32; 2]> {
    let out = dir.join(format!("{name}.wav"));
    let home = dir.join("home");
    let output = Command::new(env!("CARGO_BIN_EXE_dawsome"))
        .current_dir(repo_root())
        .env("DAWSOME_DATA_HOME", home.join("data"))
        .env("DAWSOME_CACHE_HOME", home.join("cache"))
        .env("DAWSOME_CONFIG_HOME", home.join("config"))
        .env("DAWSOME_STATE_HOME", home.join("state"))
        .env("XDG_DATA_HOME", home.join("xdg-data"))
        .env("XDG_CACHE_HOME", home.join("xdg-cache"))
        .env("XDG_CONFIG_HOME", home.join("xdg-config"))
        .env("XDG_STATE_HOME", home.join("xdg-state"))
        .arg("render")
        .arg(project)
        .arg(&out)
        .args(["--bits", "32f"])
        .args(args)
        .output()
        .expect("run dawsome");
    assert!(
        output.status.success(),
        "dawsome render {} failed: {}",
        project.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut reader = hound::WavReader::open(&out).expect("open rendered wav");
    assert_eq!(reader.spec().channels, 2);
    let values: Vec<f32> = reader
        .samples::<f32>()
        .map(|v| v.expect("decodes"))
        .collect();
    values
        .chunks_exact(2)
        .map(|pair| [pair[0], pair[1]])
        .collect()
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

// ---------- the player, driven as the app's audio callback drives it ----------

/// `project` compiled to play its pattern 1, its first channel taking the live notes.
fn compiled(compiler: &mut Compiler, project: &Project, touched: &Touched) -> PlanUpdate {
    compiler.set_live_channel(project.channels.keys().next().copied());
    compiler
        .compile(
            project,
            touched,
            PlayMode::Pattern(PatternId(1)),
            |path: &Path| Err::<SampleBuffer, PathBuf>(path.to_path_buf()),
        )
        .expect("compiles")
}

/// `project`'s player, told to play.
fn playing(project: &Project) -> (RealtimeLink, Player) {
    let update = compiled(&mut Compiler::new(), project, &Touched::default());
    let (mut link, player) = realtime::link(update, project.tempo);
    link.send(Control::Play).expect("room");
    (link, player)
}

/// One message of an events file: the call it comes before, and its bytes.
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
/// status bytes, restated (never the app's parser): 9n kk vv a note on (velocity vv / 128;
/// vv 0 a note off), 8n kk vv a note off, Bn 40 vv the pedal, Bn 01 vv the mod wheel, En ll mm
/// pitch bend, ll + 128 mm − 8192.
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
        [status, 1, value] if status & 0xF0 == 0xB0 => {
            inbox.push_control(None, LiveControl::Wheel(value), true)
        }
        [status, low, high] if status & 0xF0 == 0xE0 => {
            let value = i16::from(low) + 128 * i16::from(high) - 8192;
            inbox.push_control(None, LiveControl::Bend(value), true)
        }
        _ => panic!("a message the cases do not send: {bytes:02X?}"),
    };
    assert!(taken, "the inbox had room");
}

/// Plays `frames` frames of `project` with `script`'s messages, each process call `sizes` says
/// long but cut where a message is taken (on its call's first frame), so the messages reach the
/// player exactly where 512-frame calls would hand them over.
fn render_live(
    project: &Project,
    script: &[Timed],
    frames: usize,
    sizes: impl IntoIterator<Item = usize>,
) -> Vec<[f32; 2]> {
    let (link, mut player) = playing(project);
    let inbox = link.midi_inbox();
    let mut next = 0;
    let mut hand_in = |call: usize| {
        while let Some((at, bytes)) = script.get(next) {
            if usize::try_from(*at).expect("a call") > call {
                break;
            }
            deliver(&inbox, bytes);
            next += 1;
        }
    };
    let message_frames: Vec<usize> = script
        .iter()
        .map(|(call, _)| usize::try_from(*call).expect("a call") * CALL)
        .collect();
    let mut out = vec![[f32::NAN; 2]; frames];
    let mut done = 0;
    let mut sizes = sizes.into_iter();
    hand_in(0);
    while done < frames {
        let size = sizes.next().expect("enough block sizes");
        let mut end = (done + size).min(frames);
        if let Some(&at) = message_frames.iter().find(|&&at| at > done) {
            end = end.min(at);
        }
        player.process(&mut out[done..end]);
        done = end;
        if done % CALL == 0 {
            hand_in(done / CALL);
        }
    }
    out
}

/// Plays `frames` frames of `project`'s pattern in calls `sizes` long.
fn render_pattern(
    project: &Project,
    frames: usize,
    sizes: impl IntoIterator<Item = usize>,
) -> Vec<[f32; 2]> {
    render_live(project, &[], frames, sizes)
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

/// The render of case `case`: its project through the command line, or its messages through
/// the player in 512-frame calls.
fn render_case(case: &str, answer: &Value, dir: &Path) -> Vec<[f32; 2]> {
    let frames = index(&answer["frames"]);
    if answer["live"].as_bool().expect("live") {
        let project = load(answer["project"].as_str().expect("project"));
        let script = events(answer["events"].as_str().expect("events"));
        render_live(&project, &script, frames, calls())
    } else {
        let fixture = fixtures().join(answer["fixture"].as_str().expect("fixture"));
        render_cli(&fixture, dir, case, &[])
    }
}

// ---------- the checks ----------

/// The amplitude of the `hz` component of `side` over `[lo, hi)`, a whole number of periods at
/// `rate`: the projection onto sine and cosine (gen_p5's `amplitude`).
fn amplitude(side: &[f32], hz: f64, rate: f64, lo: usize, hi: usize) -> f64 {
    let (mut s, mut c) = (0.0, 0.0);
    for (n, &y) in side.iter().enumerate().take(hi).skip(lo) {
        let phase = TAU * hz * n as f64 / rate;
        s += f64::from(y) * phase.sin();
        c += f64::from(y) * phase.cos();
    }
    2.0 * s.hypot(c) / (hi - lo) as f64
}

/// The amplitude of the `hz` tone in `side` over `[lo, hi)` under a periodic Hann window (the
/// generator's `hann_amplitude`): 2·|Σ w·y·e^(−jωn)| / Σ w.
fn hann_amplitude(side: &[f32], hz: f64, rate: f64, lo: usize, hi: usize) -> f64 {
    let width = (hi - lo) as f64;
    let (mut re, mut im, mut weight) = (0.0, 0.0, 0.0);
    for (n, &y) in side.iter().enumerate().take(hi).skip(lo) {
        let w = 0.5 - 0.5 * (TAU * (n - lo) as f64 / width).cos();
        let phase = TAU * hz * n as f64 / rate;
        re += w * f64::from(y) * phase.cos();
        im -= w * f64::from(y) * phase.sin();
        weight += w;
    }
    2.0 * re.hypot(im) / weight
}

fn side(frames: &[[f32; 2]], side: usize) -> Vec<f32> {
    frames.iter().map(|frame| frame[side]).collect()
}

/// Asserts `frames`, the render of `case`, holds check `check` of its answer; `renders` holds
/// the other cases' renders, for the checks that compare with one.
fn assert_check(
    case: &str,
    check: &str,
    frames: &[[f32; 2]],
    answer: &Value,
    want: &Value,
    renders: &BTreeMap<&str, Vec<[f32; 2]>>,
) {
    let rate = number(&want["rate"]);
    match check {
        "sha256" => assert_eq!(
            frames_hash(frames),
            answer["sha256"].as_str().expect("sha256"),
            "{case}: the hash of every frame"
        ),
        "probes" => {
            let tolerance = number(&want["tolerances"]["close"]);
            let at = answer["probe_frames"].as_array().expect("probe frames");
            for (place, key) in ["left", "right"].into_iter().enumerate() {
                let values = answer["probes"][key].as_array().expect("probes");
                assert_eq!(values.len(), at.len(), "{case}: {key} probes");
                for (frame, value) in at.iter().map(index).zip(values) {
                    let (got, ideal) = (frames[frame][place], sample(value));
                    assert!(
                        (f64::from(got) - f64::from(ideal)).abs() <= tolerance,
                        "{case}, frame {frame} {key}: {got} is not {ideal}"
                    );
                }
            }
        }
        "left_equals_right" => {
            let differ = frames
                .iter()
                .position(|[left, right]| left.to_bits() != right.to_bits());
            assert_eq!(differ, None, "{case}: left and right differ");
        }
        "means" => {
            let left = side(frames, 0);
            for mean in answer["means"].as_array().expect("means") {
                let (lo, hi) = (index(&mean["window"][0]), index(&mean["window"][1]));
                let got =
                    left[lo..hi].iter().map(|&y| f64::from(y)).sum::<f64>() / (hi - lo) as f64;
                let ideal = number(&mean["expected"]);
                assert!(
                    (got - ideal).abs() <= number(&mean["tolerance"]),
                    "{case}: the mean over [{lo}, {hi}) is {got}, expected {ideal}"
                );
            }
        }
        "sides_differ" => {
            let by = number(&answer["sides_differ"]["by_more_than"]);
            let most = frames
                .iter()
                .map(|[left, right]| (f64::from(*left) - f64::from(*right)).abs())
                .fold(0.0, f64::max);
            assert!(most > by, "{case}: the sides differ by at most {most}");
        }
        "amplitude" => {
            let amp = &answer["amplitude"];
            let (lo, hi) = (index(&amp["window"][0]), index(&amp["window"][1]));
            let ideal = number(&amp["expected"]);
            for place in 0..2 {
                let got = amplitude(&side(frames, place), number(&amp["hz"]), rate, lo, hi);
                assert!(
                    (got - ideal).abs() <= number(&amp["tolerance"]),
                    "{case}, side {place}: amplitude {got}, expected {ideal}"
                );
            }
        }
        "tones" => {
            let sides = [side(frames, 0), side(frames, 1)];
            for tone in answer["tones"].as_array().expect("tones") {
                let (lo, hi) = (index(&tone["window"][0]), index(&tone["window"][1]));
                let hz = number(&tone["hz"]);
                let place = index(&tone["side"]);
                let got = hann_amplitude(&sides[place], hz, rate, lo, hi);
                let ideal = number(&tone["expected"]);
                assert!(
                    (got - ideal).abs() <= number(&tone["tolerance"]),
                    "{case}: {hz} Hz (side {place}) over [{lo}, {hi}) is {got}, expected {ideal}"
                );
            }
        }
        "crossings" => {
            let crossings = &answer["crossings"];
            let window = &crossings["window"];
            let (lo, hi) = (index(&window[0]), index(&window[1]));
            let tolerance = index(&crossings["tolerance"]);
            let by_hand: Vec<usize> = crossings["frames"]
                .as_array()
                .expect("frames")
                .iter()
                .map(index)
                .collect();
            // Upward: a frame below 0, the next at or above it.
            let left = side(frames, 0);
            let got: Vec<usize> = (lo.max(1)..hi)
                .filter(|&n| left[n - 1] < 0.0 && left[n] >= 0.0)
                .collect();
            assert_eq!(
                got.len(),
                by_hand.len(),
                "{case}: zero crossings over [{lo}, {hi})"
            );
            for (frame, hand) in got.into_iter().zip(by_hand) {
                assert!(
                    frame.abs_diff(hand) <= tolerance,
                    "{case}: a zero crossing on frame {frame}, by hand on {hand}"
                );
            }
        }
        "differs" => {
            let differs = &answer["differs"];
            let span = |key: &str| index(&differs[key][0])..index(&differs[key][1]);
            let (first, second) = (&frames[span("first")], &frames[span("second")]);
            assert_eq!(first.len(), second.len(), "{case}: spans alike");
            let most = first
                .iter()
                .zip(second)
                .flat_map(|(a, b)| [a[0] - b[0], a[1] - b[1]])
                .fold(0.0f32, |most, d| most.max(d.abs()));
            let by = number(&differs["by_more_than"]);
            assert!(
                f64::from(most) > by,
                "{case}: the second span is the first's within {most}"
            );
        }
        "peak_below" => {
            let peak = &answer["peak_below"];
            let (lo, hi) = (index(&peak["window"][0]), index(&peak["window"][1]));
            let got = frames[lo..hi]
                .iter()
                .flatten()
                .fold(0.0f32, |most, v| most.max(v.abs()));
            let before = number(&peak["before"]);
            assert!(
                f64::from(got) < before,
                "{case}: peak {got} over [{lo}, {hi}), not below {before}"
            );
        }
        "zero_from" => {
            let from = index(&answer["zero_from"]);
            assert!(from < frames.len(), "{case}: a tail to check");
            let first = frames[from..].iter().position(|frame| *frame != [0.0; 2]);
            assert_eq!(first, None, "{case}: a frame from {from} is not 0");
            assert_ne!(
                frames[from - 1],
                [0.0; 2],
                "{case}: frame {} sounds",
                from - 1
            );
        }
        "same_as" => {
            let other = answer["same_as"].as_str().expect("a case");
            assert_eq!(
                frames_hash(frames),
                frames_hash(&renders[other]),
                "{case}: the bits of {other}"
            );
        }
        "prefix_of" => {
            let other = answer["prefix_of"]["case"].as_str().expect("a case");
            let to = index(&answer["prefix_of"]["frames"]);
            let theirs = &renders[other];
            assert_eq!(
                frames_hash(&frames[..to]),
                frames_hash(&theirs[..to]),
                "{case}: {other}'s frames before {to}"
            );
            assert_ne!(frames[to], theirs[to], "{case}: frame {to} is {other}'s");
        }
        other => panic!("{case}: no check for \"{other}\""),
    }
}

/// Known answers (gen_p33_expected.py): every listed case, the pattern cases rendered through the
/// command line to 32-bit float and the live ones played through the player's MIDI inbox in
/// 512-frame calls, holds every check its "asserts" list names: every probed frame within 1e-6
/// (sin and tan are the platform's) and, for the cases with neither, the hash of every frame;
/// both sides alike, or apart for spread unison; the sine's sustain at lin(−6 dB) by
/// projection, the envelope's release ending on exactly 0.0, the stolen notes' tones gone and
/// the others sounding, the bends' tones, the tuning's and the unison voices' tones by hand, the
/// pedal's note held to the lift and released on it; the filter's tones by hand under key
/// tracking, velocity → cutoff and the square LFO, and the zero crossings of the glide, of the
/// separated pair that does not glide, of the LFO's and the mod wheel's vibrato, by hand; the
/// same note twice on one slot sounding different the second time (its oscillators run free)
/// and a six-note chord's first 50 ms peaking below its peak when every note started its
/// phases at 0; the Drive's harmonics of a driven sine by hand, and drive written as 0 giving the
/// saw's bits. A listed case missing from the answers, or a check this test does not know,
/// fails it.
#[test]
fn p33_the_synth_renders_its_known_answers() {
    let want = expected();
    let dir = temp_dir("renders");
    let mut renders = BTreeMap::new();
    for case in CASES {
        let answer = &want["cases"][case];
        assert!(
            answer.is_object(),
            "{case} is not a case of p33_expected.json"
        );
        let frames = render_case(case, answer, &dir);
        assert_eq!(frames.len(), index(&answer["frames"]), "{case}: frames");
        renders.insert(case, frames);
    }
    for case in CASES {
        let answer = &want["cases"][case];
        let checks = answer["asserts"].as_array().expect("asserts");
        assert!(!checks.is_empty(), "{case}: nothing to check");
        for check in checks {
            let check = check.as_str().expect("a check");
            assert_check(case, check, &renders[case], answer, &want, &renders);
        }
    }
}

/// Known answers (gen_p33_expected.py): the cases this test plays are every case of
/// p33_expected.json, by count and by name, so none can be left out unseen.
#[test]
fn p33_every_case_of_the_answers_is_played() {
    let want = expected();
    let mut answers: Vec<&str> = want["cases"]
        .as_object()
        .expect("cases")
        .keys()
        .map(String::as_str)
        .collect();
    let mut listed = CASES.to_vec();
    assert_eq!(listed.len(), answers.len(), "how many cases");
    answers.sort_unstable();
    listed.sort_unstable();
    assert_eq!(listed, answers, "the cases' names");
}

/// The number at `path` (object keys and array places) under `raw`, read from the text the
/// generator wrote by std's correctly rounded parser (serde_json's own reads some 17-digit
/// numbers an ulp off).
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

/// Known answers (gen_p33_expected.py's): the synth's id, name and instrument
/// flag, all 38 parameters' keys, labels, ranges, defaults, units and whole flags in order, the
/// names the choices' values go by (none for the others), and what the log controls' 0, 0.5
/// and 1 stand for (exp2 alone, so exact).
#[test]
fn p33_the_synth_has_the_specs_parameters() {
    let want = &expected()["synth"];
    let text = answers_text();
    let root: BTreeMap<&str, &RawValue> = serde_json::from_str(&text).expect("parses");
    let raw = root["synth"];
    let registry = Registry::native();
    let id = want["id"].as_str().expect("id");
    let factory = registry.get(id).expect("registered");
    assert_eq!(factory.name(), want["name"].as_str().expect("name"));
    assert_eq!(Some(factory.is_instrument()), want["instrument"].as_bool());
    let params = want["params"].as_array().expect("params");
    assert_eq!(factory.params().len(), 38);
    assert_eq!(factory.params().len(), params.len());
    for (place, (info, param)) in factory.params().iter().zip(params).enumerate() {
        let key = param["key"].as_str().expect("key");
        let at = place.to_string();
        assert_eq!(info.key, key, "parameter {place}");
        assert_eq!(info.label, param["label"].as_str().expect("label"), "{key}");
        assert_eq!(
            f64::from(info.min),
            exact(raw, &["params", &at, "min"]),
            "{key}"
        );
        assert_eq!(
            f64::from(info.max),
            exact(raw, &["params", &at, "max"]),
            "{key}"
        );
        assert_eq!(info.default, number(&param["default"]) as f32, "{key}");
        assert_eq!(info.unit, param["unit"].as_str().expect("unit"), "{key}");
        assert_eq!(Some(info.whole), param["whole"].as_bool(), "{key}");
        match want["value_names"][key].as_array() {
            Some(names) => {
                // From the parameter's min up, one name per whole value.
                for (step, name) in names.iter().enumerate() {
                    let value = info.min + step as f32;
                    assert_eq!(
                        factory.value_name(place, value),
                        name.as_str(),
                        "{key} at {value}"
                    );
                }
            }
            None => assert_eq!(factory.value_name(place, info.default), None, "{key}"),
        }
        match want["shown"][key].as_array() {
            Some(points) => {
                for (point, values) in points.iter().enumerate() {
                    let value = number(&values[0]) as f32;
                    let shown = exact(raw, &["shown", key, &point.to_string(), "1"]);
                    assert_eq!(
                        factory.shown(place, value, 44_100),
                        shown,
                        "{key} at {value}"
                    );
                }
            }
            None => assert_eq!(
                factory.shown(place, info.max, 44_100),
                f64::from(info.max),
                "{key}"
            ),
        }
    }
}

/// Each case, through the player in 512-frame calls and again cut at seeded random sizes (the
/// live messages still taken where 512-frame calls would take them), gives the same bits: no
/// voice depends on where a block starts. A pattern case's render is also the command line's
/// bits.
#[test]
fn p33_the_synth_gives_the_same_bits_however_the_calls_are_cut() {
    let want = expected();
    let dir = temp_dir("calls");
    for (seed, case) in (1u64..).zip(CASES) {
        let answer = &want["cases"][case];
        let frames = index(&answer["frames"]);
        let live = answer["live"].as_bool().expect("live");
        let (project, script) = if live {
            let project = load(answer["project"].as_str().expect("project"));
            (project, events(answer["events"].as_str().expect("events")))
        } else {
            (
                load(answer["fixture"].as_str().expect("fixture")),
                Vec::new(),
            )
        };
        let whole = render_live(&project, &script, frames, calls());
        let cut = render_live(&project, &script, frames, block_sizes(seed));
        assert_eq!(
            frames_hash(&cut),
            frames_hash(&whole),
            "{case}: seeded calls"
        );
        if !live {
            let fixture = fixtures().join(answer["fixture"].as_str().expect("fixture"));
            let door = render_cli(&fixture, &dir, case, &[]);
            assert_eq!(
                frames_hash(&door),
                frames_hash(&whole),
                "{case}: the command line"
            );
        }
    }
}

/// The steal case's 18 notes (16 voices, then two steals) played as three unison voices of a
/// square and a triangle with noise, through a filter its envelope, key tracking, velocity and
/// the LFO move every frame, the LFO on the pitch too, each note gliding from the one before, a
/// bend and the mod wheel from a device, and every one of the synth's 38 parameters moved at
/// once (a preset's worth, the Drive to 0.5 among them) through a plan: the player's calls touch
/// no heap.
#[test]
fn the_synths_path_never_touches_the_heap() {
    let mut project = load("p33_steal.json");
    let processor = project
        .channels
        .get_mut(&ChannelId(1))
        .and_then(|channel| channel.processor.as_mut())
        .expect("the synth");
    for (key, value) in [
        ("osc1_wave", 3.0),
        ("osc2_wave", 1.0),
        ("osc_mix", 0.5),
        ("unison", 3.0),
        ("spread", 1.0),
        ("noise", 0.2),
        ("cutoff", 0.4),
        ("resonance", 0.6),
        ("filter_env", 0.5),
        ("key_track", 0.5),
        ("vel_cutoff", 0.3),
        ("lfo_cutoff", 0.2),
        ("lfo_pitch", 0.5),
        ("glide_ms", 150.0),
    ] {
        processor.params.insert(key.to_owned(), value);
    }
    let mut compiler = Compiler::new();
    let update = compiled(&mut compiler, &project, &Touched::default());
    let (mut link, mut player) = realtime::link(update, project.tempo);
    link.send(Control::Play).expect("room");
    let inbox = link.midi_inbox();
    let mut edited = project.clone();
    let registry = Registry::native();
    let factory = registry.get("dawsome.synth").expect("registered");
    let preset = factory
        .params()
        .iter()
        .map(|info| Edit::SetChannelParam {
            channel: ChannelId(1),
            key: info.key.to_owned(),
            value: Some(info.clamp((info.min + info.max) / 2.0)),
        })
        .collect();
    let applied = edit::apply(&mut edited, Edit::Transaction(preset)).expect("applies");
    let mut moved = Some(compiled(&mut compiler, &edited, &applied.touched));
    let mut buffer = vec![[0.0f32; 2]; CALL];
    let mut calls = 0;
    let mut loudest = 0.0f32;
    for call in 0..12 {
        match call {
            2 => assert!(inbox.push_control(None, LiveControl::Bend(8191), true)),
            3 => assert!(inbox.push_control(None, LiveControl::Wheel(100), true)),
            5 => link
                .push_plan(moved.take().expect("once"))
                .expect("same time base"),
            _ => {}
        }
        calls += heap_calls_during(|| player.process(&mut buffer));
        loudest = buffer
            .iter()
            .flatten()
            .fold(loudest, |peak, value| peak.max(value.abs()));
    }
    assert_eq!(calls, 0, "the synth's path touched the heap");
    // One note alone peaks at 0.63 at most here (its unison voices averaged, then the Q 4.6
    // filter ringing past lin(−6 dB); each of the 18 played alone through gen_p33's synth over
    // the first five calls): past 0.75, several were sounding. (Before the oscillators ran free
    // the notes' phases started together and the busy render passed 1.)
    assert!(loudest > 0.75, "many voices were sounding: peak {loudest}");
}

/// Every p33 project reads and saves back to its own bytes, version 9 (a synth channel); a
/// synth channel made in code saves and reads back as itself; deleting it, the project writes
/// the version it would without one.
#[test]
fn a_synth_channel_saves_as_version_9_and_reads_back() {
    assert_eq!(SYNTH_FORMAT_VERSION, 9);
    let mut names: Vec<String> = fs::read_dir(fixtures())
        .expect("fixtures")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.starts_with("p33_") && name.ends_with(".json"))
        .filter(|name| !name.ends_with("_events.json"))
        .filter(|name| name != "p33_expected.json" && name != "p33_laws.json")
        .collect();
    names.sort();
    assert_eq!(names.len(), 22, "{names:?}");
    for name in &names {
        let text = fs::read_to_string(fixtures().join(name)).expect("reads");
        assert!(text.starts_with(r#"{"format_version":9,"#), "{name}");
        let project = Project::load_json(&text).expect("loads");
        assert_eq!(
            project.channels[&ChannelId(1)].kind,
            ChannelKind::Synth,
            "{name}"
        );
        assert_eq!(
            project.to_json().expect("serializes"),
            text.trim_end_matches('\n'),
            "{name}"
        );
    }
    let mut project = Project::new(44_100, MilliBpm(120_000), 96);
    let add = Edit::AddSynthChannel {
        id: ChannelId(1),
        name: "Lead".into(),
    };
    edit::apply(&mut project, add).expect("adds");
    let saved = project.to_json().expect("serializes");
    assert!(
        saved.contains(
            r#""name":"Lead","kind":"synth","processor":{"id":"dawsome.synth","params":{}}"#
        ),
        "{saved}"
    );
    assert_eq!(Project::load_json(&saved).expect("reads"), project);
    let removed = edit::apply(&mut project, Edit::RemoveChannel { id: ChannelId(1) });
    assert_eq!(
        removed.expect("removes").inverse,
        Edit::AddSynthChannel {
            id: ChannelId(1),
            name: "Lead".into()
        }
    );
    let without = project.to_json().expect("serializes");
    assert!(without.starts_with(r#"{"format_version":2,"#), "{without}");
}

/// Why version 9: a build reads a file's version before anything else, and refuses one it does
/// not know by its number, naming the versions it reads, whatever the file holds. So a build of
/// version 8 refuses a synth project as "format_version 9 is not supported (this build reads
/// versions 1, 2, 4, 5, 6, 7 and 8)". Shown here one version on: a kind no build knows, in a
/// version this one does not know (10), is refused by that number, the message naming 9; the
/// same kind in a version this one reads (9) fails on the kind, naming no version, which is what
/// version 8 would have done with a synth project written as 8.
#[test]
fn an_older_build_refuses_a_synth_project_by_its_version() {
    let text = fs::read_to_string(fixtures().join("p33_sine.json")).expect("reads");
    let unknown = text.replacen(r#""kind":"synth""#, r#""kind":"theremin""#, 1);
    assert_ne!(unknown, text);
    let newer = unknown.replacen(r#""format_version":9"#, r#""format_version":10"#, 1);
    let error = Project::load_json(&newer).expect_err("refused");
    assert!(
        matches!(error, LoadError::UnsupportedVersion(10)),
        "{error}"
    );
    assert_eq!(
        error.to_string(),
        "format_version 10 is not supported (this build reads versions 1, 2, 4, 5, 6, 7, 8 and 9)"
    );
    let error = Project::load_json(&unknown).expect_err("refused");
    assert!(matches!(error, LoadError::Json(_)), "{error}");
    assert!(!error.to_string().contains("format_version"), "{error}");
}

/// A synth parameter edit reaches the running synth: the sine case played, its
/// volume edited from −6 to −12 dB after 2048 frames through the compile that follows the edit
/// (a plan with the changed value), plays from there the bits of the same project made at
/// −12 dB from the start (the volume scales the output alone, so the voices are the same);
/// before it, the −6 dB bits. The edit's inverse puts −6 dB back.
#[test]
fn a_synth_parameter_edit_reaches_the_running_synth() {
    let project = load("p33_sine.json");
    let mut compiler = Compiler::new();
    let update = compiled(&mut compiler, &project, &Touched::default());
    let (mut link, mut player) = realtime::link(update, project.tempo);
    link.send(Control::Play).expect("room");
    let mut edited = project.clone();
    let quieter = Edit::SetChannelParam {
        channel: ChannelId(1),
        key: "volume_db".into(),
        value: Some(-12.0),
    };
    let applied = edit::apply(&mut edited, quieter).expect("applies");
    assert_eq!(applied.touched.channels, [ChannelId(1)].into());
    let mut out = vec![[0.0f32; 2]; 8192];
    player.process(&mut out[..2048]);
    let moved = compiled(&mut compiler, &edited, &applied.touched);
    link.push_plan(moved).expect("same time base");
    player.process(&mut out[2048..]);
    let at_minus_12 = render_pattern(&edited, 8192, calls());
    let at_minus_6 = render_pattern(&project, 8192, calls());
    assert_eq!(frames_hash(&out[..2048]), frames_hash(&at_minus_6[..2048]));
    assert_eq!(frames_hash(&out[2048..]), frames_hash(&at_minus_12[2048..]));
    assert_ne!(
        at_minus_6[4000], at_minus_12[4000],
        "the edit changed the sound"
    );
    let mut undone = edited.clone();
    edit::apply(&mut undone, applied.inverse).expect("undoes");
    assert_eq!(undone, project);
}

/// A compiler handed its project at another rate makes every processor again, at that rate:
/// the sine case compiled at 44.1 kHz, then at 48 kHz by the same compiler, plays
/// 440 Hz at lin(−6 dB) at 48 kHz (by projection over 440 whole periods, within 1e-4). A synth
/// kept from the first compile would be in no slot of the new player, which would be silent.
#[test]
fn a_project_at_another_rate_gets_its_processors_made_again() {
    let mut project = load("p33_sine.json");
    let mut compiler = Compiler::new();
    let first = compiled(&mut compiler, &project, &Touched::default());
    drop(realtime::link(first, project.tempo));
    project.sample_rate = 48_000;
    let again = compiled(&mut compiler, &project, &Touched::default());
    let (mut link, mut player) = realtime::link(again, project.tempo);
    link.send(Control::Play).expect("room");
    let mut out = vec![[0.0f32; 2]; 52_800];
    player.process(&mut out);
    let left = side(&out, 0);
    let got = amplitude(&left, 440.0, 48_000.0, 4800, 52_800);
    let lin_minus_6 = 10f64.powf(-6.0 / 20.0);
    assert!((got - lin_minus_6).abs() <= 1e-4, "amplitude {got}");
}

/// A synth parameter's automation target is written `{"channel_param":{"channel":1,
/// "param":"cutoff"}}`, beside a plugin's `"param_id"`, and reads back as itself; one naming both
/// or neither is refused.
#[test]
fn a_synth_parameters_target_reads_and_writes_by_its_key() {
    let target = AutomationTarget::InstrumentParam {
        channel: ChannelId(1),
        key: "cutoff".into(),
    };
    let text = serde_json::to_string(&target).expect("writes");
    assert_eq!(text, r#"{"channel_param":{"channel":1,"param":"cutoff"}}"#);
    let read: AutomationTarget = serde_json::from_str(&text).expect("reads");
    assert_eq!(read, target);
    let plugin: AutomationTarget =
        serde_json::from_str(r#"{"channel_param":{"channel":2,"param_id":7}}"#).expect("reads");
    assert_eq!(
        plugin,
        AutomationTarget::ChannelParam {
            channel: ChannelId(2),
            param_id: 7
        }
    );
    for refused in [
        r#"{"channel_param":{"channel":1,"param":"cutoff","param_id":7}}"#,
        r#"{"channel_param":{"channel":1}}"#,
    ] {
        assert!(
            serde_json::from_str::<AutomationTarget>(refused).is_err(),
            "{refused}"
        );
    }
    let project = load("p33_sine.json");
    assert!(project.has_target(&target));
    let mut sampler = project.clone();
    sampler
        .channels
        .insert(ChannelId(1), Channel::sampler("s", "x.wav"));
    assert!(
        !sampler.has_target(&target),
        "a sampler has no synth parameters"
    );
}

/// Every factory preset (assets/synth-presets, read here as plain JSON rather than through the
/// app's reader) loaded into the sine case's synth channel with `Edit::SetChannelParams` and
/// rendered by `dawsome render`: Init (an empty map: every default) sounds unlike the sine it
/// replaced, and each other preset unlike Init, every render finite and sounding (a peak over
/// 1e-3 in its bar: the Pad's 1.6 s attack too). The load's inverse gives back the sine's map
/// exactly, so an undo restores the sound before it.
#[test]
fn every_factory_preset_renders_unlike_init_through_the_command_line() {
    let dir = temp_dir("factory_presets");
    let sine = load("p33_sine.json");
    let mut files: Vec<PathBuf> = fs::read_dir(repo_root().join("assets/synth-presets"))
        .expect("the presets folder")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    assert_eq!(files.len(), 8, "the eight factory presets");
    let render = |stem: &str, params: BTreeMap<String, f32>| {
        let mut loaded = sine.clone();
        let load = Edit::SetChannelParams {
            channel: ChannelId(1),
            params: params.clone(),
        };
        let applied = edit::apply(&mut loaded, load).expect("loads");
        assert_eq!(applied.touched.channels, [ChannelId(1)].into());
        let processor = loaded.channels[&ChannelId(1)].processor.as_ref();
        assert_eq!(processor.map(|processor| &processor.params), Some(&params));
        let mut undone = loaded.clone();
        edit::apply(&mut undone, applied.inverse).expect("undoes");
        assert_eq!(undone, sine, "{stem}: the undo gives the sine back");
        let path = dir.join(format!("{stem}.json"));
        fs::write(&path, loaded.to_json().expect("writes")).expect("saves");
        let frames = render_cli(&path, &dir, stem, &[]);
        let peak = frames
            .iter()
            .flatten()
            .fold(0.0f32, |peak, value| peak.max(value.abs()));
        assert!(
            frames.iter().flatten().all(|value| value.is_finite()),
            "{stem}"
        );
        assert!(peak > 1e-3, "{stem} sounds: peak {peak}");
        frames_hash(&frames)
    };
    let mut hashes = BTreeMap::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("a preset file");
        let preset: Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(preset["synth"], "dawsome.synth");
        let params: BTreeMap<String, f32> = preset["params"]
            .as_object()
            .expect("a map")
            .iter()
            .map(|(key, value)| (key.clone(), number(value) as f32))
            .collect();
        let stem = file
            .file_stem()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        let hash = render(&stem, params);
        hashes.insert(stem, hash);
    }
    let init = &hashes["init"];
    let sine_frames = render_cli(&fixtures().join("p33_sine.json"), &dir, "sine", &[]);
    assert_ne!(&frames_hash(&sine_frames), init, "Init replaces the sine");
    for (stem, hash) in &hashes {
        if stem != "init" {
            assert_ne!(hash, init, "{stem} sounds unlike Init");
        }
    }
}
