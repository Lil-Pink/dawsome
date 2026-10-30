//! Instrument channels in the engine: the SFZ reader gives the regions each test set means,
//! and the player renders the sets' notes to the frames tests/fixtures/gen_p11_expected.py
//! states, however a render is split into blocks, without touching the heap. Expected values
//! come from tests/fixtures/p11_expected.json, never from the engine.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::{ClipHint, ClipSound, Compiler, Sounds};
use dawsome_engine::dsp::instrument::{Instrument, ZoneSample};
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, Edit, Touched};
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::project::{
    Channel, ChannelId, ChannelKind, Note, PatternId, Project, FORMAT_VERSION,
    INSTRUMENT_FORMAT_VERSION,
};
use dawsome_engine::realtime::{self, Player, RealtimeLink};
use dawsome_engine::sfz::{self, LoopMode, Region};
use dawsome_engine::time::Ticks;
use dawsome_engine::transport::Control;
use serde_json::{json, Value};

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p11_expected.json"))
        .expect("p11_expected.json (run gen_p11_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("parses")
}

/// The set whose main file is `sfz` (relative to the fixtures), read as a loader reads it:
/// includes and samples relative to the main file's folder.
fn read_set(sfz: &Path) -> sfz::Sfz {
    let main = fixtures().join(sfz);
    let dir = main.parent().expect("a folder").to_path_buf();
    let text = fs::read_to_string(&main).expect("the set's file");
    let name = main
        .file_name()
        .expect("a name")
        .to_string_lossy()
        .into_owned();
    sfz::parse(&name, &text, &mut |path| {
        fs::read_to_string(dir.join(path)).map_err(|error| error.to_string())
    })
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

/// The set whose main file is `sfz`, built at 44.1 kHz from the WAVs beside it.
fn build_set(sfz: &Path) -> Instrument {
    let dir = fixtures()
        .join(sfz)
        .parent()
        .expect("a folder")
        .to_path_buf();
    let read = read_set(sfz);
    let (instrument, problems) =
        Instrument::build(&read.regions, |path| wav(&dir.join(path)), 44_100);
    assert_eq!(problems, Vec::<String>::new(), "{}", sfz.display());
    instrument
}

/// Sounds from the fixtures folder: instrument sets as `build_set` builds them.
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
        Ok(Some(build_set(path)))
    }
}

fn load(name: &str) -> Project {
    let text = fs::read_to_string(fixtures().join(name)).expect("a p11 fixture");
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

fn bits(frames: &[[f32; 2]]) -> Vec<[u32; 2]> {
    frames.iter().map(|frame| frame.map(f32::to_bits)).collect()
}

/// Asserts `out` holds exactly the expected render: its sounding frames, all else silence.
fn assert_render(name: &str, out: &[[f32; 2]], want: &Value) {
    assert_eq!(
        out.len() as u64,
        want["frames"].as_u64().expect("frames"),
        "{name}"
    );
    let mut sounding: BTreeMap<usize, [f32; 2]> = BTreeMap::new();
    for entry in want["nonzero"].as_array().expect("a list") {
        let frame = entry[0].as_u64().expect("a frame") as usize;
        let value = |side: usize| entry[side].as_f64().expect("a value") as f32;
        sounding.insert(frame, [value(1), value(2)]);
    }
    assert!(!sounding.is_empty(), "{name}: the answer sounds");
    for (frame, got) in out.iter().enumerate() {
        let want = sounding.get(&frame).copied().unwrap_or([0.0; 2]);
        assert_eq!(
            bits(&[*got]),
            bits(&[want]),
            "{name}, frame {frame}: {got:?} != {want:?}"
        );
    }
}

fn mode_name(mode: Option<LoopMode>) -> Value {
    match mode {
        None => Value::Null,
        Some(LoopMode::NoLoop) => json!("no_loop"),
        Some(LoopMode::OneShot) => json!("one_shot"),
        Some(LoopMode::Continuous) => json!("loop_continuous"),
        Some(LoopMode::Sustain) => json!("loop_sustain"),
    }
}

/// A region in the generator's terms.
fn region_json(region: &Region) -> Value {
    let pair = |a: Option<u64>, b: Option<u64>| match (a, b) {
        (Some(a), Some(b)) => json!([a, b]),
        _ => Value::Null,
    };
    json!({
        "sample": region.sample,
        "lokey": region.lokey,
        "hikey": region.hikey,
        "pitch_keycenter": region.pitch_keycenter,
        "lovel": region.lovel,
        "hivel": region.hivel,
        "loop_mode": mode_name(region.loop_mode),
        "loop": pair(region.loop_start, region.loop_end),
        "ampeg_attack": region.ampeg_attack,
        "ampeg_release": region.ampeg_release,
        "amp_veltrack": region.amp_veltrack,
        "volume": region.volume,
        "tune": region.tune,
        "transpose": region.transpose,
        "pan": region.pan,
        "offset": region.offset,
        "end": region.end,
        "seq": (region.seq_length > 1).then(|| json!([region.seq_length, region.seq_position])),
    })
}

/// Known answer: each set's text gives the regions the generator states, field for field:
/// inheritance from <global>, <master> and <group>, key= setting three opcodes, note names,
/// #define and #include, default_path, comments and a sample name with a space.
#[test]
fn the_reader_gives_the_regions_each_set_means() {
    let want = expected();
    for (name, set) in want["sets"].as_object().expect("sets") {
        let sfz = Path::new("instruments")
            .join(set["dir"].as_str().expect("dir"))
            .join(set["sfz"].as_str().expect("sfz"));
        let read = read_set(&sfz);
        assert_eq!(read.problems, [], "{name}");
        let got: Vec<Value> = read.regions.iter().map(region_json).collect();
        let regions = want["regions"][name].as_array().expect("regions").clone();
        assert_eq!(got, regions, "{name}");
    }
}

/// Known answer: the set "formats" (what real libraries write) gives the regions and the
/// problems the generator states.
#[test]
fn the_reader_gives_what_real_libraries_write_the_regions_it_means() {
    let want = &expected()["formats"];
    let sfz =
        Path::new(want["dir"].as_str().expect("dir")).join(want["sfz"].as_str().expect("sfz"));
    let read = read_set(&sfz);
    let problems: Vec<Value> = read.problems.iter().map(|p| json!(p.to_string())).collect();
    assert_eq!(Value::Array(problems), want["problems"]);
    let got: Vec<Value> = read.regions.iter().map(region_json).collect();
    assert_eq!(Value::Array(got), want["regions"]);
}

/// Known answer: macro's opcodes and header the reader leaves out, each once with how often
/// and where it first appears.
#[test]
fn the_reader_lists_what_it_leaves_out() {
    let read = read_set(Path::new("instruments/macro/macro.sfz"));
    let got: Vec<Value> = read
        .ignored
        .iter()
        .map(|ignored| {
            json!({"name": ignored.name, "count": ignored.count, "file": ignored.first.file,
                   "line": ignored.first.line})
        })
        .collect();
    assert_eq!(Value::Array(got), expected()["ignored"]["macro"]);
    let layers = read_set(Path::new("instruments/layers/layers.sfz"));
    assert_eq!(
        layers.ignored,
        [],
        "a set of supported opcodes leaves nothing out"
    );
}

/// Known answers: each set's notes render to the generator's frames (which zone, at what
/// pitch and gain, how it loops, releases and takes turns), in one block and split into the
/// edge-case and seeded block sizes: loops and releases cross block edges, bit for bit.
#[test]
fn every_set_renders_its_known_answer_however_it_is_split() {
    let want = expected();
    for name in ["layers", "pad", "rr", "macro"] {
        let answer = &want["renders"][name];
        let project = load(answer["fixture"].as_str().expect("fixture"));
        let frames = answer["frames"].as_u64().expect("frames") as usize;
        let (_link, mut player) = playing(&project);
        let whole = render_player(&mut player, frames, [frames]);
        assert_render(name, &whole, answer);
        for seed in [1, 0x5eed, 0xdead_beef] {
            let (_link, mut player) = playing(&project);
            let split = render_player(&mut player, frames, block_sizes(seed));
            assert_eq!(bits(&split), bits(&whole), "{name}, seed {seed:#x}");
        }
    }
}

/// Known answer: a live note on the pad (held for one 512-frame call, released over its
/// 176 frames) renders the generator's frames; it is timed by the output frames, so the
/// transport's Play takes nothing from it.
#[test]
fn a_live_note_holds_its_loop_and_releases_at_note_off() {
    let answer = &expected()["renders"]["live"];
    let project = load("p11_live.json");
    let frames = answer["frames"].as_u64().expect("frames") as usize;
    let (mut link, mut player) = playing(&project);
    let mut out = vec![[0.0; 2]; frames];
    link.send(Control::LiveNoteOn {
        key: 60,
        velocity: 127.0 / 128.0,
    })
    .expect("room");
    player.process(&mut out[..512]);
    link.send(Control::LiveNoteOff { key: 60 }).expect("room");
    for block in out[512..].chunks_mut(512) {
        player.process(block);
    }
    assert_render("live", &out, answer);
}

/// The round robin starts again on Play: a second Play renders the first's frames; and a new
/// plan of the same set (an edit elsewhere) carries the count on, so a render with one swapped
/// in renders the same as one without.
#[test]
fn round_robins_restart_on_play_and_carry_across_plans() {
    let project = load("p11_rr.json");
    let bar = 88_200;
    let (mut link, mut player) = playing(&project);
    let first = render_player(&mut player, bar, [512; 200]);
    link.send(Control::Play).expect("room");
    let again = render_player(&mut player, bar, [512; 200]);
    assert_eq!(
        bits(&again),
        bits(&first),
        "Play counts from the first note again"
    );

    let (_link, mut player) = playing(&project);
    let two_bars = render_player(&mut player, 2 * bar, [512; 400]);
    let mut compiler = Compiler::new();
    let plan = |compiler: &mut Compiler| {
        compiler
            .compile_with(
                &project,
                &Touched::default(),
                PlayMode::Pattern(PatternId(1)),
                &mut FixtureSets,
            )
            .expect("compiles")
    };
    let (mut link, mut player) = realtime::link(plan(&mut compiler), project.tempo);
    link.send(Control::Play).expect("room");
    let mut swapped = render_player(&mut player, bar + 1000, [512; 200]);
    link.push_plan(plan(&mut compiler)).expect("same time base");
    swapped.extend(render_player(&mut player, bar - 1000, [512; 200]));
    assert_eq!(
        bits(&swapped),
        bits(&two_bars),
        "the count goes on across the new plan"
    );
}

/// A dense instrument project: every set on a channel of its own, a note on every step with
/// long gates so loops, releases, round robins and layers overlap; plus live notes.
fn dense_project() -> Project {
    let mut project = load("p11_layers.json");
    for (id, (name, path)) in [
        ("pad", "instruments/pad/pad.sfz"),
        ("rr", "instruments/rr/rr.sfz"),
        ("macro", "instruments/macro/macro.sfz"),
    ]
    .into_iter()
    .enumerate()
    {
        let id = ChannelId(2 + id as u32);
        project.channels.insert(id, Channel::instrument(name, path));
    }
    project.next_channel_id = ChannelId(5);
    let pattern = project.patterns.get_mut(&PatternId(1)).expect("pattern 1");
    pattern.notes = (0..16)
        .flat_map(|step| {
            (1..=4u32).map(move |channel| Note {
                channel: ChannelId(channel),
                start: Ticks(i64::from(step) * 24),
                length: Ticks(30),
                velocity: 0.25 + 0.05 * channel as f32,
                key: 48 + u8::try_from((step * 5 + channel) % 30).expect("small"),
            })
        })
        .collect();
    project.validate().expect("valid");
    project
}

#[test]
fn process_never_touches_the_heap_with_instruments_playing() {
    let control = heap_calls_during(|| drop(std::hint::black_box(vec![0u8; 64])));
    assert!(control > 0, "the counting allocator is not counting");
    let project = dense_project();
    let (mut link, mut player) = playing(&project);
    let mut out = vec![[0.0; 2]; 512];
    let mut calls = 0;
    for call in 0..800 {
        if call % 100 == 50 {
            link.push_plan(compiled(&project)).expect("same time base");
        }
        if call % 7 == 3 {
            link.send(Control::LiveNoteOn {
                key: 60 + (call % 12) as u8,
                velocity: 0.8,
            })
            .expect("room");
        }
        if call % 7 == 5 {
            link.send(Control::LiveNoteOff {
                key: 60 + ((call - 2) % 12) as u8,
            })
            .expect("room");
        }
        if call == 400 {
            link.send(Control::Stop).expect("room");
        }
        if call == 450 {
            link.send(Control::Play).expect("room");
        }
        calls += heap_calls_during(|| player.process(&mut out));
        link.drain_retired();
    }
    assert_eq!(calls, 0, "Player::process made {calls} heap calls");
}

/// The dense project renders the same bits however it is split.
#[test]
fn a_dense_instrument_render_does_not_depend_on_the_split() {
    let project = dense_project();
    let frames = 3 * 88_200;
    let (_link, mut player) = playing(&project);
    let whole = render_player(&mut player, frames, [frames]);
    assert!(whole.iter().any(|frame| *frame != [0.0; 2]), "it sounds");
    for seed in [3, 0xfeed] {
        let (_link, mut player) = playing(&project);
        let split = render_player(&mut player, frames, block_sizes(seed));
        if let Some(frame) = bits(&whole)
            .iter()
            .zip(bits(&split))
            .position(|(a, b)| *a != b)
        {
            panic!("seed {seed:#x}: frame {frame} differs from the one-block render");
        }
    }
}

/// The p11 fixtures are canonical version 4 files: each saves back to its own bytes.
#[test]
fn instrument_projects_save_back_to_their_own_bytes() {
    for name in [
        "p11_layers.json",
        "p11_pad.json",
        "p11_rr.json",
        "p11_macro.json",
        "p11_shipped.json",
        "p11_live.json",
        "p11_after_set.json",
    ] {
        let text = fs::read_to_string(fixtures().join(name)).expect("fixture");
        assert!(text.starts_with(r#"{"format_version":4"#), "{name}");
        let saved = Project::load_json(&text)
            .expect("loads")
            .to_json()
            .expect("saves");
        assert_eq!(saved, text.trim_end_matches('\n'), "{name}");
    }
}

/// Migration: a version 2 project loads as it is; adding an instrument channel (at the volume
/// a channel that plays notes is added at, 9/16) makes it write version 4, which loads back to
/// the same project, and undoing the add writes version 2 again, byte for byte. A version 2 file
/// names no instrument, so the shape needs no change.
#[test]
fn an_instrument_channel_moves_a_project_to_version_4_and_its_undo_back() {
    let base = fs::read_to_string(fixtures().join("p11_base.json")).expect("fixture");
    assert!(base.starts_with(&format!(r#"{{"format_version":{FORMAT_VERSION}"#)));
    let mut project = Project::load_json(&base).expect("loads");
    let add = Edit::AddInstrumentChannel {
        id: project.next_channel_id,
        name: "Pad".to_owned(),
        instrument: PathBuf::from("instruments/pad/pad.sfz"),
    };
    let applied = edit::apply(&mut project, add).expect("adds");
    let saved = project.to_json().expect("saves");
    assert!(saved.starts_with(&format!(
        r#"{{"format_version":{INSTRUMENT_FORMAT_VERSION}"#
    )));
    assert!(saved.contains(
        r#"{"id":2,"name":"Pad","kind":"instrument","instrument":"instruments/pad/pad.sfz","volume":0.5625,"pan":0.0,"mute":false}"#
    ));
    assert_eq!(Project::load_json(&saved).expect("loads back"), project);
    edit::apply(&mut project, applied.inverse).expect("undoes");
    assert_eq!(project.to_json().expect("saves") + "\n", base);
}

/// An instrument channel names its set and nothing else: a sample or a processor beside it,
/// or a set on another kind, is refused.
#[test]
fn an_instrument_channel_names_its_set_alone() {
    let base = fs::read_to_string(fixtures().join("p11_layers.json")).expect("fixture");
    let set = r#""instrument":"instruments/layers/layers.sfz""#;
    assert!(base.contains(set));
    for wrong in [
        format!(r#""sample":"a.wav",{set}"#),
        r#""sample":"a.wav""#.to_owned(),
    ] {
        assert!(
            Project::load_json(&base.replace(set, &wrong)).is_err(),
            "{wrong}"
        );
    }
    let sampler = base.replace(r#""kind":"instrument""#, r#""kind":"sampler""#);
    assert!(
        Project::load_json(&sampler).is_err(),
        "a sampler with a set"
    );
    let project = Project::load_json(&base).expect("loads");
    assert_eq!(
        project.channels[&ChannelId(1)].kind,
        ChannelKind::Instrument
    );
}

/// A set the sounds cannot give plays silence: its channel is silent and nothing is refused.
#[test]
fn an_empty_set_is_silent() {
    let mut project = load("p11_layers.json");
    project
        .channels
        .get_mut(&ChannelId(1))
        .expect("ch")
        .instrument = PathBuf::from("none.sfz");
    struct Empty;
    impl Sounds for Empty {
        type Error = Infallible;
        fn sample(&mut self, _: &Path) -> Result<Option<SampleBuffer>, Infallible> {
            Ok(None)
        }
        fn clip(&mut self, _: &Path, _: ClipHint) -> Result<ClipSound, Infallible> {
            Ok(ClipSound::Loading)
        }
    }
    let update = Compiler::new()
        .compile_with(
            &project,
            &Touched::default(),
            PlayMode::Pattern(PatternId(1)),
            &mut Empty,
        )
        .expect("compiles");
    let (mut link, mut player) = realtime::link(update, project.tempo);
    link.send(Control::Play).expect("room");
    let out = render_player(&mut player, 88_200, [512; 200]);
    assert!(out.iter().all(|frame| *frame == [0.0; 2]));
    assert_eq!(Instrument::default().zones(), 0);
}
