//! The external input channel in the engine: format version 6 (its files save back to their own
//! bytes, older versions keep theirs), the edits that add one and change its source with exact
//! inverses, and the player: a fed input plays through any block split to the frames
//! tests/fixtures/gen_p16_expected.py computed, is heard while stopped, takes no note, and its
//! path never touches the heap.

use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, Edit, Touched};
use dawsome_engine::input::{InputFeed, InputFeeds};
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::project::{
    Channel, ChannelId, ChannelKind, InputChannels, InputSource, LoadError, PatternId, Project,
    Sound, SoundSource, INPUT_FORMAT_VERSION,
};
use dawsome_engine::realtime::{self, Player, RealtimeLink};
use dawsome_engine::transport::Control;
use sha2::{Digest, Sha256};

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn text(name: &str) -> String {
    fs::read_to_string(fixtures().join(name)).expect("reads the fixture")
}

fn load(name: &str) -> Project {
    Project::load_json(&text(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn expected() -> serde_json::Value {
    serde_json::from_str(&text("p16_expected.json")).expect("parses")
}

// ---------------------------------------------------------------------------------------------
// The project file
// ---------------------------------------------------------------------------------------------

const P16: [&str; 5] = [
    "p16_unity.json",
    "p16_half_left.json",
    "p16_mono_left.json",
    "p16_muted.json",
    "p16_insert.json",
];

// The generator writes each project as the file shape states it: loading and saving gives the
// same bytes, version 6 first.
#[test]
fn a_project_with_an_input_channel_saves_back_to_its_own_bytes() {
    for name in P16 {
        let text = text(name);
        let project = load(name);
        assert!(text.starts_with(r#"{"format_version":6,"#), "{name}");
        assert_eq!(
            project.to_json().expect("serializes"),
            text.trim_end_matches('\n'),
            "{name}"
        );
    }
    let unity = load("p16_unity.json");
    let channel = &unity.channels[&ChannelId(1)];
    assert_eq!(channel.kind, ChannelKind::Input);
    assert_eq!(
        channel.input,
        Some(InputSource {
            device: "USB Audio Line Input".into(),
            channels: InputChannels::Stereo(0, 1),
        })
    );
    assert_eq!(INPUT_FORMAT_VERSION, 6);
}

// A project without an input channel keeps writing its old version: the muted case with its
// input channel deleted (and its notes, none) writes version 2.
#[test]
fn without_an_input_channel_the_old_version_is_written() {
    let mut project = load("p16_muted.json");
    edit::apply(&mut project, Edit::DeleteChannel { id: ChannelId(1) }).expect("deletes");
    let saved = project.to_json().expect("serializes");
    assert!(saved.starts_with(r#"{"format_version":2,"#), "{saved}");
}

// A version this build does not know is refused by its number, and the message names the
// versions it reads, 6 among them. (9: version 8 is a sound's edits since the sample editor.)
#[test]
fn a_newer_version_is_refused_by_name() {
    let newer =
        text("p16_unity.json").replacen(r#""format_version":6"#, r#""format_version":9"#, 1);
    let error = Project::load_json(&newer).expect_err("refused");
    assert!(matches!(error, LoadError::UnsupportedVersion(9)), "{error}");
    assert!(error.to_string().contains('6'), "{error}");
}

// What reading refuses: a sampler naming an input, an input channel naming a sample or no input.
// An input channel's MIDI link is left out with a note, as an audio channel's is: it plays no
// notes.
#[test]
fn an_input_channel_names_its_input_and_nothing_else() {
    let base = text("p16_unity.json");
    let source = r#""input":{"device":"USB Audio Line Input","channels":{"stereo":[0,1]}},"#;
    let refused = [
        ("no input", base.replacen(source, "", 1)),
        (
            "a sample too",
            base.replacen(source, &format!(r#"{source}"sample":"x.wav","#), 1),
        ),
        (
            "a sampler with an input",
            base.replacen(
                r#""kind":"input""#,
                r#""kind":"sampler","sample":"x.wav""#,
                1,
            ),
        ),
    ];
    for (what, text) in refused {
        assert_ne!(text, base, "{what}: the case changes the file");
        assert!(Project::load_json(&text).is_err(), "{what}: accepted");
    }
    let linked = base.replacen(
        r#""mute":false}"#,
        r#""mute":false,"midi_input":"Oriole"}"#,
        1,
    );
    assert_ne!(linked, base);
    let (project, notes) = Project::load_json_noting(&linked).expect("reads");
    assert_eq!(project.channels[&ChannelId(1)].midi_input, None);
    assert_eq!(notes.len(), 1, "{notes:?}");
}

// ---------------------------------------------------------------------------------------------
// Edits
// ---------------------------------------------------------------------------------------------

fn oriole() -> InputSource {
    InputSource {
        device: "USB Audio Line Input".into(),
        channels: InputChannels::Stereo(0, 1),
    }
}

// Adding an input channel is one edit whose inverse takes it out again, byte for byte; the
// channel is as `Channel::input` makes it.
#[test]
fn an_input_channel_adds_with_an_exact_inverse() {
    let mut project = load("p16_muted.json");
    edit::apply(&mut project, Edit::DeleteChannel { id: ChannelId(1) }).expect("deletes");
    let before = project.to_json().expect("serializes");
    let id = project.next_channel_id;
    let applied = edit::apply(
        &mut project,
        Edit::AddInputChannel {
            id,
            name: "Oriole".into(),
            input: oriole(),
        },
    )
    .expect("adds");
    assert_eq!(project.channels[&id], Channel::input("Oriole", oriole()));
    assert!(project
        .to_json()
        .expect("serializes")
        .starts_with(r#"{"format_version":6,"#));
    assert_eq!(applied.inverse, Edit::RemoveChannel { id });
    let redo = edit::apply(&mut project, applied.inverse).expect("removes");
    assert_eq!(project.to_json().expect("serializes"), before);
    assert!(matches!(redo.inverse, Edit::AddInputChannel { .. }));
}

// Changing an input channel's device or channels is a `ReplaceChannelSound` with an input
// source: its volume, pan, mute and routing stay, and the inverse gives the file back byte for
// byte. A sampler given an input becomes an input channel (its settings go: an input takes
// none); the input given a sampler back is the sampler it was.
#[test]
fn an_input_channels_source_changes_with_an_exact_inverse() {
    let mut project = load("p16_insert.json");
    let before = project.to_json().expect("serializes");
    let pair = InputSource {
        device: "Generic USB Audio".into(),
        channels: InputChannels::Mono(1),
    };
    let applied = edit::apply(
        &mut project,
        Edit::ReplaceChannelSound {
            channel: ChannelId(1),
            name: "Oriole".into(),
            source: Box::new(SoundSource::input(pair.clone())),
            sound: Sound::default(),
        },
    )
    .expect("replaces");
    let channel = &project.channels[&ChannelId(1)];
    assert_eq!(channel.input, Some(pair));
    assert_eq!(channel.insert.0, 1, "routing kept");
    edit::apply(&mut project, applied.inverse).expect("undoes");
    assert_eq!(project.to_json().expect("serializes"), before);

    let mut project = load("p16_muted.json");
    let before = project.to_json().expect("serializes");
    let sampler = project.channels[&ChannelId(2)].clone();
    let applied = edit::apply(
        &mut project,
        Edit::ReplaceChannelSound {
            channel: ChannelId(2),
            name: sampler.name.clone(),
            source: Box::new(SoundSource::input(oriole())),
            sound: Sound::default(),
        },
    )
    .expect("a sampler takes an input");
    let channel = &project.channels[&ChannelId(2)];
    assert_eq!(channel.kind, ChannelKind::Input);
    assert!(channel.sample.as_os_str().is_empty());
    edit::apply(&mut project, applied.inverse).expect("undoes");
    assert_eq!(project.to_json().expect("serializes"), before);

    // Settings an input cannot take are refused, as for a plugin.
    let tuned = Sound {
        semitones: 2,
        ..Sound::default()
    };
    let refused = edit::apply(
        &mut project,
        Edit::ReplaceChannelSound {
            channel: ChannelId(2),
            name: "hit".into(),
            source: Box::new(SoundSource::input(oriole())),
            sound: tuned,
        },
    );
    assert!(refused.is_err(), "an input takes no sound settings");
    assert_eq!(project.to_json().expect("serializes"), before);
}

// A channel linked to a MIDI port given an input (which plays no
// notes) lets the port go in the same step: the project stays valid, saves and loads back as
// it is, and undo gives the channel its sound and its link back, byte for byte.
#[test]
fn a_linked_channel_given_an_input_lets_its_port_go_in_the_same_step() {
    let mut project = load("p16_muted.json");
    edit::apply(&mut project, Edit::DeleteChannel { id: ChannelId(1) }).expect("deletes");
    edit::apply(
        &mut project,
        Edit::SetChannelMidiInput {
            channel: ChannelId(2),
            port: Some("Port A".into()),
        },
    )
    .expect("links");
    project.validate().expect("valid with the link");
    let before = project.to_json().expect("serializes");
    let applied = edit::apply(
        &mut project,
        Edit::ReplaceChannelSound {
            channel: ChannelId(2),
            name: "External input".into(),
            source: Box::new(SoundSource::input(InputSource::default_device())),
            sound: Sound::default(),
        },
    )
    .expect("replaces");
    project.validate().expect("valid after");
    let channel = &project.channels[&ChannelId(2)];
    assert_eq!(
        (channel.kind, channel.midi_input.as_deref()),
        (ChannelKind::Input, None)
    );
    let saved = project.to_json().expect("serializes");
    let (reloaded, notes) = Project::load_json_noting(&saved).expect("loads");
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(reloaded.to_json().expect("serializes"), saved);
    edit::apply(&mut project, applied.inverse).expect("undoes");
    assert_eq!(project.to_json().expect("serializes"), before);
}

// ---------------------------------------------------------------------------------------------
// The player
// ---------------------------------------------------------------------------------------------

/// The feed file's frames, as the decoder reads 16-bit PCM: q / 32768.
fn feed_frames() -> Vec<[f32; 2]> {
    let mut reader = hound::WavReader::open(fixtures().join("p16_input.wav")).expect("opens");
    let spec = reader.spec();
    assert_eq!((spec.channels, spec.sample_rate), (2, 48_000));
    let values: Vec<i16> = reader.samples::<i16>().map(|v| v.expect("reads")).collect();
    values
        .chunks_exact(2)
        .map(|pair| [f32::from(pair[0]) / 32768.0, f32::from(pair[1]) / 32768.0])
        .collect()
}

/// A feed of frames in memory: silence past its last one. Counts its drops in `dropped`.
struct TestFeed {
    frames: Arc<[[f32; 2]]>,
    channels: usize,
    next: usize,
    dropped: Arc<AtomicUsize>,
}

impl InputFeed for TestFeed {
    fn channels(&self) -> usize {
        self.channels
    }

    fn read(&mut self, out: &mut [[f32; 2]]) {
        for slot in out {
            *slot = self.frames.get(self.next).copied().unwrap_or([0.0; 2]);
            self.next += 1;
        }
    }
}

impl Drop for TestFeed {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

/// The feed channel `id` of `project` gets from the feed file: its picked sides, a mono pick on
/// both.
fn feed_for(project: &Project, id: ChannelId, dropped: &Arc<AtomicUsize>) -> TestFeed {
    let frames = feed_frames();
    let input = project.channels[&id]
        .input
        .clone()
        .expect("an input channel");
    let (frames, channels): (Vec<[f32; 2]>, usize) = match input.channels {
        InputChannels::Stereo(l, r) => (
            frames
                .iter()
                .map(|f| [f[usize::from(l)], f[usize::from(r)]])
                .collect(),
            2,
        ),
        InputChannels::Mono(n) => (
            frames
                .iter()
                .map(|f| [f[usize::from(n)], f[usize::from(n)]])
                .collect(),
            1,
        ),
    };
    TestFeed {
        frames: frames.into(),
        channels,
        next: 0,
        dropped: Arc::clone(dropped),
    }
}

fn compiled(project: &Project) -> PlanUpdate {
    // The sampler's sound (p16_impulse.wav): one frame of 1.0.
    let impulse = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(vec![1.0]));
    Compiler::new()
        .compile(
            project,
            &Touched::default(),
            PlayMode::Pattern(PatternId(1)),
            impulse,
        )
        .expect("compiles")
}

/// A player of `project` with each input channel fed from the feed file, playing unless
/// `stopped`.
fn fed_player(
    project: &Project,
    stopped: bool,
    dropped: &Arc<AtomicUsize>,
) -> (RealtimeLink, Player) {
    let (mut link, player) = realtime::link(compiled(project), project.tempo);
    let mut feeds = InputFeeds::new();
    for (&id, channel) in &project.channels {
        if channel.kind == ChannelKind::Input {
            feeds.insert(id, Box::new(feed_for(project, id, dropped)));
        }
    }
    assert!(link.set_input_feeds(feeds).is_ok(), "room for the feeds");
    if !stopped {
        link.send(Control::Play).expect("room");
    }
    (link, player)
}

fn render(
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

/// 1, a prime, a power of two, the bus size, a large block, then seeded sizes from 1 to 5000.
fn block_sizes(seed: u64) -> impl Iterator<Item = usize> {
    let mut state = seed;
    let random = std::iter::repeat_with(move || {
        // xorshift64, so a failure reproduces exactly.
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        usize::try_from(state % 5000).expect("below 5000") + 1
    });
    [1, 7, 64, 512, 4096].into_iter().chain(random)
}

fn hash(frames: &[[f32; 2]]) -> String {
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

// Known answer 5 at the player: one block, blocks of 1, 7, 64 and 4096, and seeded random splits
// give the same bits, which are the generator's frames for every render.
#[test]
fn a_fed_input_plays_the_known_answers_through_any_block_split() {
    let want = expected();
    for (name, answer) in want["renders"].as_object().expect("renders") {
        let project = load(answer["fixture"].as_str().expect("fixture"));
        let frames = usize::try_from(answer["frames"].as_u64().expect("frames")).expect("fits");
        let dropped = Arc::new(AtomicUsize::new(0));
        let (_link, mut player) = fed_player(&project, false, &dropped);
        let whole = render(&mut player, frames, [frames]);
        assert_eq!(
            hash(&whole),
            answer["sha256"].as_str().expect("a hash"),
            "{name}: not the generator's frames"
        );
        for split in [vec![1], vec![7], vec![64], vec![4096]] {
            let (_link, mut player) = fed_player(&project, false, &dropped);
            let sizes = std::iter::repeat(split[0]);
            assert!(
                render(&mut player, frames, sizes) == whole,
                "{name}: blocks of {} differ",
                split[0]
            );
        }
        for seed in [1, 0x5eed, 0xdead_beef] {
            let (_link, mut player) = fed_player(&project, false, &dropped);
            assert!(
                render(&mut player, frames, block_sizes(seed)) == whole,
                "{name}: seed {seed:#x} differs"
            );
        }
    }
}

// Monitoring: while stopped the input is heard through its channel, and Play takes the feed on
// from the frame it reached: 1000 frames stopped, then playing, are the feed in order, frame for
// frame (unity: the output is the feed).
#[test]
fn an_input_is_heard_while_stopped_and_play_reads_on() {
    let project = load("p16_unity.json");
    let dropped = Arc::new(AtomicUsize::new(0));
    let (mut link, mut player) = fed_player(&project, true, &dropped);
    let mut out = render(&mut player, 1000, std::iter::repeat(333));
    link.send(Control::Play).expect("room");
    out.extend(render(&mut player, 47_000, std::iter::repeat(512)));
    let feed = feed_frames();
    assert!(out == feed, "the feed in order, stopped and playing");
}

// A muted input channel is silent but its feed is read on, so unmuting (a new plan) hears the
// frame that plays then, never an old one; a live note on an input channel plays nothing.
#[test]
fn a_muted_input_reads_on_and_notes_on_an_input_play_nothing() {
    let mut project = load("p16_unity.json");
    project.channels.get_mut(&ChannelId(1)).expect("ch 1").mute = true;
    let dropped = Arc::new(AtomicUsize::new(0));
    let (mut link, mut player) = fed_player(&project, false, &dropped);
    let muted = render(&mut player, 1000, std::iter::repeat(100));
    assert!(muted.iter().all(|frame| *frame == [0.0; 2]), "muted");
    project.channels.get_mut(&ChannelId(1)).expect("ch 1").mute = false;
    link.check_room().expect("room");
    link.push_plan(compiled(&project)).expect("same time base");
    link.send(Control::ChannelNoteOn {
        channel: ChannelId(1),
        key: 60,
        velocity: 1.0,
    })
    .expect("room");
    let heard = render(&mut player, 2000, std::iter::repeat(512));
    assert!(
        heard == feed_frames()[1000..3000],
        "frames 1000.. of the feed"
    );
}

// The input's audio-thread path never touches the heap: reading feeds playing and stopped
// (heard and not), taking a new set of feeds in place of the old (which goes back to be dropped
// here), muted and unmuted plans, over 1000 calls of odd sizes. The old sets are dropped on this
// thread, outside the measurement.
#[test]
fn the_input_path_never_touches_the_heap() {
    let control = heap_calls_during(|| drop(std::hint::black_box(vec![0u8; 64])));
    assert!(control > 0, "the counting allocator is not counting");
    let mut project = load("p16_muted.json");
    let dropped = Arc::new(AtomicUsize::new(0));
    let (mut link, mut player) = fed_player(&project, true, &dropped);
    let mut out = vec![[0.0; 2]; 1500];
    let mut calls = 0;
    for call in 0..1000usize {
        if call % 100 == 50 {
            let mut feeds = InputFeeds::new();
            feeds.insert(
                ChannelId(1),
                Box::new(feed_for(&project, ChannelId(1), &dropped)),
            );
            assert!(link.set_input_feeds(feeds).is_ok(), "room for the feeds");
        }
        if call == 200 {
            link.send(Control::Play).expect("room");
        }
        if call == 400 || call == 600 {
            let channel = project.channels.get_mut(&ChannelId(1)).expect("ch 1");
            channel.mute = !channel.mute;
            link.check_room().expect("room");
            link.push_plan(compiled(&project)).expect("same time base");
        }
        if call == 800 {
            link.send(Control::Stop).expect("room");
        }
        let len = 1 + (call * 37) % out.len();
        calls += heap_calls_during(|| player.process(&mut out[..len]));
        link.drain_retired();
    }
    assert_eq!(calls, 0, "Player::process made {calls} heap calls");
    assert_eq!(
        dropped.load(Ordering::Relaxed),
        10,
        "every replaced feed came back and was dropped here"
    );
}
