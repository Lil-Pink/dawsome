//! Player properties that make offline and live playback one code path: `process` never touches the
//! heap (also while plans swap and controls arrive), the output does not depend on how a render is
//! split into blocks, and plan swaps and tempo changes land on the samples that
//! tests/fixtures/gen_p2_expected.py computes for them.

use std::convert::Infallible;

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, Edit, Touched};
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::project::{
    ChannelId, Clip, ClipId, LoopRange, Note, PatternId, Project, TrackId, DEFAULT_KEY,
};
use dawsome_engine::realtime::{self, PlanQueueFull, Player, Position, RealtimeLink};
use dawsome_engine::time::{self, step_to_ticks, MilliBpm, Ticks};
use dawsome_engine::transport::Control;

const F1: &str = include_str!("../../../tests/fixtures/f1_four_on_floor.json");

// The audio thread's rule (no alloc, no free) is checked by counting heap calls; the app's
// device-rate bridge is checked with the same counter.
#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

fn f1_project() -> Project {
    Project::load_json(F1).expect("F1 loads")
}

/// Compiles `project` to play `mode`, giving every channel `frames` instead of its file.
fn plan_with_sample(project: &Project, mode: PlayMode, frames: &[f32]) -> PlanUpdate {
    let same_sample = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(frames.to_vec()));
    Compiler::new()
        .compile(project, &Touched::default(), mode, same_sample)
        .expect("in-memory samples load")
}

/// A player looping pattern 1 of `project` at its tempo, told to play from tick 0, and the UI
/// half of its link (keep it alive, as the app does).
fn playing(project: &Project, frames: &[f32]) -> (RealtimeLink, Player) {
    let plan = plan_with_sample(project, PlayMode::Pattern(PatternId(1)), frames);
    let (mut link, player) = realtime::link(plan, project.tempo);
    link.send(Control::Play)
        .expect("the control ring starts empty");
    (link, player)
}

fn frames_in_bars(project: &Project, bars: u32) -> usize {
    let ticks = time::bars_to_ticks(bars, project.ppq);
    let samples = time::tick_to_sample(ticks, project.sample_rate, project.tempo, project.ppq)
        .expect("bar start fits in u64");
    usize::try_from(samples).expect("render fits in memory")
}

/// Frames of the sample played with [`dense_project`]: longer than its 66017-frame bar, so the
/// 80 notes of a bar all sound at once and the 64-voice pool must take voices over.
const DENSE_SAMPLE_FRAMES: usize = 70_000;

/// The messy case: 174.5 BPM at 48 kHz (no whole number of samples per tick), five channels
/// lighting all 16 steps. Play it with a [`DENSE_SAMPLE_FRAMES`] sample.
fn dense_project() -> Project {
    let mut project = f1_project();
    project.sample_rate = 48_000;
    project.tempo = MilliBpm(174_500);
    let kick = project.channels[&ChannelId(1)].clone();
    for id in 2..=5 {
        project.channels.insert(ChannelId(id), kick.clone());
    }
    project.next_channel_id = ChannelId(6);
    let pattern = project.patterns.get_mut(&PatternId(1)).expect("pattern 1");
    // Step by step, channel by channel: the (tick, channel) order notes are kept in.
    pattern.notes = (0..16)
        .flat_map(|step| {
            (0..5u8).map(move |channel| Note {
                channel: ChannelId(1 + u32::from(channel)),
                start: step_to_ticks(step, 96),
                length: Ticks(0),
                velocity: 0.25 + 0.05 * f32::from(channel),
                key: DEFAULT_KEY,
            })
        })
        .collect();
    project.validate().expect("dense project is valid");
    project
}

/// The dense project with every note on its own key, 48 to 67: twenty playback speeds from half
/// to about 1.5 times the sample's own, so voices read between frames and end on fractions.
fn pitched_project() -> Project {
    let mut project = dense_project();
    let pattern = project.patterns.get_mut(&PatternId(1)).expect("pattern 1");
    for (index, note) in pattern.notes.iter_mut().enumerate() {
        note.key = 48 + u8::try_from(index % 20).expect("below 20");
    }
    project.validate().expect("pitched project is valid");
    project
}

/// Renders `frames` frames of `project`, calling `process` once per size in `block_sizes`.
fn render(
    project: &Project,
    sample: &[f32],
    frames: usize,
    block_sizes: impl IntoIterator<Item = usize>,
) -> Vec<[f32; 2]> {
    let (_link, mut player) = playing(project, sample);
    render_player(&mut player, frames, block_sizes)
}

/// Renders `frames` frames from `player`, calling `process` once per size in `block_sizes`.
fn render_player(
    player: &mut Player,
    frames: usize,
    block_sizes: impl IntoIterator<Item = usize>,
) -> Vec<[f32; 2]> {
    let mut out = vec![[f32::NAN; 2]; frames];
    let mut done = 0;
    for size in block_sizes {
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

/// Edge-case block sizes first (1, a prime, a power of two, a round number, a large power of
/// two, more than a bar), then seeded pseudo-random sizes from 1 to 5000.
fn block_sizes(seed: u64) -> impl Iterator<Item = usize> {
    let mut state = seed;
    let random = std::iter::repeat_with(move || {
        // xorshift64: a tiny deterministic generator, so any failure reproduces exactly.
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

/// The frames of `out` that are not silent.
fn sounding_frames(out: &[[f32; 2]]) -> Vec<usize> {
    let sounding = |(frame, value): (usize, &[f32; 2])| (*value != [0.0; 2]).then_some(frame);
    out.iter().enumerate().filter_map(sounding).collect()
}

#[test]
fn process_never_touches_the_heap() {
    // Control: the counter must see an allocation, or the zero below would prove nothing.
    let control = heap_calls_during(|| drop(std::hint::black_box(vec![0u8; 64])));
    assert!(control > 0, "the counting allocator is not counting");

    for (name, project) in [
        ("F1", f1_project()),
        ("dense", dense_project()),
        ("pitched", pitched_project()),
    ] {
        // A sample longer than a block and a bar keeps voices sounding across block edges and
        // the loop point, and fills the dense plan's voice pool.
        let sample = vec![0.5; DENSE_SAMPLE_FRAMES];
        let (mut link, mut player) = playing(&project, &sample);
        let mut out = vec![[0.0; 2]; 512];
        let (mut calls, mut swaps) = (0, 0);
        for call in 0..1000 {
            // The UI half's work happens between calls, outside the measurement: compiling and
            // boxing a plan allocates, and dropping a retired one frees.
            if call % 100 == 50 {
                let plan = plan_with_sample(&project, PlayMode::Pattern(PatternId(1)), &sample);
                link.push_plan(plan).expect("same time base");
            }
            if call == 500 {
                link.send(Control::SetTempo(MilliBpm(140_000)))
                    .expect("room in the control ring");
            }
            calls += heap_calls_during(|| player.process(&mut out));
            swaps += link.drain_retired();
        }
        assert_eq!(
            calls, 0,
            "{name}: Player::process made {calls} heap calls in 1000 calls"
        );
        assert_eq!(swaps, 10, "{name}: every pushed plan was swapped in");
    }
}

#[test]
fn any_block_split_renders_the_same_bits_as_one_block() {
    let decaying: Vec<f32> = (0..DENSE_SAMPLE_FRAMES)
        .map(|i| 1.0 / (1.0 + i as f32))
        .collect();
    let cases = [
        ("F1 impulse", f1_project(), vec![1.0]),
        ("dense", dense_project(), decaying.clone()),
        ("pitched", pitched_project(), decaying),
    ];
    for (name, project, sample) in &cases {
        let frames = frames_in_bars(project, 3); // three bars: the pattern loops twice
        let whole = bits(&render(project, sample, frames, [frames]));
        assert!(
            whole.iter().any(|frame| *frame != [0, 0]),
            "{name}: rendered silence"
        );
        for seed in [1, 0x5eed, 0xdead_beef] {
            let split = bits(&render(project, sample, frames, block_sizes(seed)));
            if let Some(frame) = whole.iter().zip(&split).position(|(a, b)| a != b) {
                panic!("{name}, seed {seed:#x}: frame {frame} differs from the one-block render");
            }
        }
    }
}

// Expected positions from Python's exact integers (PPQ 96, 48 kHz, 174.5 BPM):
//   t2s = lambda tick: tick * 60 * 48000 * 1000 // (174500 * 96)
//   [t2s(t) for t in (0, 288, 384, 672, 768)] == [0, 49512, 66017, 115530, 132034]
// Hits on steps 0 and 12 (ticks 0 and 288) with a 20000-frame sample of 1.0: the step-12 note
// of pass 0 (49512..69512) is still sounding when pass 1 starts at 66017.
#[test]
fn a_note_crossing_the_loop_point_keeps_sounding() {
    let mut project = f1_project();
    project.sample_rate = 48_000;
    project.tempo = MilliBpm(174_500);
    let pattern = project.patterns.get_mut(&PatternId(1)).expect("pattern 1");
    pattern
        .notes
        .retain(|note| note.start == Ticks(0) || note.start == Ticks(288));
    assert_eq!(frames_in_bars(&project, 2), 132_034);

    let out = render(&project, &[1.0; 20_000], 132_034, block_sizes(7));
    let expected = |frame: usize| match frame {
        0..20_000 => 1.0,
        20_000..49_512 => 0.0,
        49_512..66_017 => 1.0,
        66_017..69_512 => 2.0, // the pass-0 tail plus the pass-1 downbeat
        69_512..86_017 => 1.0,
        86_017..115_530 => 0.0,
        _ => 1.0,
    };
    for (frame, &value) in out.iter().enumerate() {
        assert_eq!(value, [expected(frame); 2], "frame {frame}");
    }
}

// From Python (gen_p2_expected.py, case (d)): F1 at 44.1 kHz, two bars (176400 frames) in 72 calls
// of 2450 frames. Plan B (F1 plus steps 2 and 6) is pushed before call 9, which starts at sample
// 22050. Step 2 of bar 1 (sample 11025) is already behind the playhead and must not fire late; the
// step-4 hit exactly on 22050 fires once; step 6 (33075) fires in bar 1; bar 2 plays all of plan B.
// F1's own hits have velocity 1.0, the switched-on steps 100/128 = 0.78125.
#[test]
fn d_a_plan_swapped_mid_bar_neither_drops_nor_repeats_a_note() {
    const EXPECTED: [(usize, f32); 11] = [
        (0, 1.0),
        (22050, 1.0),
        (33075, 0.78125),
        (44100, 1.0),
        (66150, 1.0),
        (88200, 1.0),
        (99225, 0.78125),
        (110250, 1.0),
        (121275, 0.78125),
        (132300, 1.0),
        (154350, 1.0),
    ];
    let plan_a = f1_project();
    let mut plan_b = plan_a.clone();
    for step in [2, 6] {
        let toggle = Edit::ToggleStep {
            pattern: PatternId(1),
            channel: ChannelId(1),
            step,
        };
        edit::apply(&mut plan_b, toggle).expect("step toggles");
    }
    let (mut link, mut player) = playing(&plan_a, &[1.0]);
    let mut out = vec![[0.0f32; 2]; 176_400];
    let mut calls = 0;
    for (call, block) in out.chunks_mut(2450).enumerate() {
        if call == 9 {
            let plan = plan_with_sample(&plan_b, PlayMode::Pattern(PatternId(1)), &[1.0]);
            link.push_plan(plan).expect("same time base");
        }
        player.process(block);
        calls += 1;
    }
    assert_eq!(calls, 72);
    assert_eq!(link.drain_retired(), 1, "plan A came back to the UI side");
    let frames: Vec<usize> = EXPECTED.iter().map(|&(frame, _)| frame).collect();
    assert_eq!(sounding_frames(&out), frames);
    for (frame, velocity) in EXPECTED {
        // A note triggered twice on one sample would sum to twice its velocity.
        assert_eq!(
            out[frame], [velocity; 2],
            "frame {frame} plays exactly once"
        );
    }
}

/// Renders two bars of F1 in `block`-frame calls, sending SetTempo(140000) before the first call
/// that starts at or after sample 30000. Returns the audio and where ticks 130 and 131 fall
/// afterwards.
fn render_with_tempo_change(block: usize) -> (Vec<[f32; 2]>, [Option<u64>; 2]) {
    let (mut link, mut player) = playing(&f1_project(), &[1.0]);
    let mut out: Vec<[f32; 2]> = Vec::new();
    let mut buffer = vec![[0.0f32; 2]; block];
    let mut sent = false;
    loop {
        if !sent && out.len() >= 30_000 {
            link.send(Control::SetTempo(MilliBpm(140_000)))
                .expect("room in the control ring");
            sent = true;
        }
        // Two bars end where tick 768 falls under the tempo in force, so ask after every call.
        let end = player
            .tick_to_sample(Ticks(768))
            .expect("tick 768 has a sample");
        let left = usize::try_from(end)
            .expect("fits in memory")
            .saturating_sub(out.len());
        if left == 0 {
            break;
        }
        let len = left.min(block);
        player.process(&mut buffer[..len]);
        out.extend_from_slice(&buffer[..len]);
    }
    let ticks = [130, 131].map(|tick| player.tick_to_sample(Ticks(tick)));
    (out, ticks)
}

// From Python (gen_p2_expected.py, case (e)): F1 at 44.1 kHz in 1000-frame calls; SetTempo(140000)
// arrives with the call that starts at sample 30000. t0 = ceil(30000*120000*96 / (60*44100*1000)) =
// 131 is the first tick at or after it, S0 = 131*60*44100*1000 // (120000*96) = 30089, and later
// ticks count from S0 at 140 BPM. Ticks before t0 keep the old rate, so tick 130 is not on the new
// timeline at all.
#[test]
fn e_a_tempo_change_starts_at_the_first_tick_of_the_call_that_receives_it() {
    let (out, [tick_130, tick_131]) = render_with_tempo_change(1000);
    assert_eq!(tick_131, Some(30_089), "S0");
    assert_eq!(tick_130, None, "t0 is 131");
    assert_eq!(out.len(), 155_498, "two bars");
    assert_eq!(
        sounding_frames(&out),
        [0, 22050, 42098, 60998, 79898, 98798, 117698, 136598]
    );
    let (again, _) = render_with_tempo_change(1000);
    assert!(
        bits(&out) == bits(&again),
        "the same buffer size renders the same bits"
    );
}

/// p19_rack.json with pattern 1 holding one note alone, on the sampler "hit" (channel 5) at tick
/// 47, 12 ticks long, velocity 1, looped in pattern mode with every sound a one-frame click: calls
/// of 512 frames up to frame 10000 (the 20th call 272 frames), SetTempo(64000) sent before the call
/// on frame 10000 when `tempo`, then calls of `block` frames up to frame 12000.
fn tick_47_render(tempo: bool, block: usize) -> Vec<[f32; 2]> {
    let text = include_str!("../../../tests/fixtures/p19_rack.json");
    let mut project = Project::load_json(text).expect("p19_rack loads");
    project
        .patterns
        .get_mut(&PatternId(1))
        .expect("pattern 1")
        .notes = vec![Note {
        channel: ChannelId(5),
        start: Ticks(47),
        length: Ticks(12),
        velocity: 1.0,
        key: DEFAULT_KEY,
    }];
    project.validate().expect("valid");
    let (mut link, mut player) = playing(&project, &[1.0]);
    let mut out = vec![[0.0f32; 2]; 12_000];
    let mut done = 0;
    while done < 10_000 {
        let end = (done + 512).min(10_000);
        player.process(&mut out[done..end]);
        done = end;
    }
    if tempo {
        link.send(Control::SetTempo(MilliBpm(64_000)))
            .expect("room in the control ring");
    }
    while done < out.len() {
        let end = (done + block).min(out.len());
        player.process(&mut out[done..end]);
        done = end;
    }
    out
}

// Found while MIDI out was built: a
// live tempo change starts on the first tick at or after the playhead, so the call that brings it
// starts before the new tempo's first sample, and the note on that very tick must still play.
// By hand (Python, exact ints): at 128 BPM, 44.1 kHz, ppq 96, s(47) = 47·60·44100·1000 //
// (128000·96) = 10120; the first tick at or after sample 10000 is ceil(46.44) = 47, so 64 BPM
// starts on tick 47 at sample 10120. Without the change the note sounds on frame 10120 as well;
// either way, in calls of any size after the change.
#[test]
fn a_note_on_the_tick_a_live_tempo_change_starts_on_plays() {
    let steady = tick_47_render(false, 512);
    assert_eq!(sounding_frames(&steady).first(), Some(&10_120), "no change");
    let changed = tick_47_render(true, 512);
    assert_eq!(sounding_frames(&changed).first(), Some(&10_120), "64 BPM");
    for block in [1, 7, 64, 120, 121, 4096] {
        assert!(
            bits(&tick_47_render(true, block)) == bits(&changed),
            "calls of {block} frames render the same bits"
        );
    }
}

// F1 at 44.1 kHz and 120 BPM puts tick t at floor(t·229.6875). Resending 120 BPM before the third
// 512-frame call (sample 1024) must change nothing: a retime would restart the timeline at the
// first tick at or after 1024, tick 5 (sample 1148), and put tick 96 at
// 1148 + floor(91·229.6875) = 22049 instead of floor(96·229.6875) = 22050 (Python, exact ints).
#[test]
fn resending_the_tempo_in_force_moves_no_beat() {
    let project = f1_project();
    let (mut link, mut player) = playing(&project, &[1.0]);
    let mut out = vec![[0.0f32; 2]; 512 * 44];
    for (call, block) in out.chunks_mut(512).enumerate() {
        if call == 2 {
            link.send(Control::SetTempo(project.tempo))
                .expect("room in the control ring");
        }
        player.process(block);
    }
    assert_eq!(sounding_frames(&out), [0, 22050]);
}

#[test]
fn song_mode_renders_silence() {
    let project = f1_project();
    let plan = plan_with_sample(&project, PlayMode::Song, &[1.0]);
    let (mut link, mut player) = realtime::link(plan, project.tempo);
    link.send(Control::Play).expect("room in the control ring");
    let mut out = vec![[1.0f32; 2]; 88_200];
    player.process(&mut out);
    assert_eq!(
        sounding_frames(&out),
        [] as [usize; 0],
        "there is no arrangement yet"
    );
    assert!(link.playing());
    assert_eq!(link.position().samples, 88_200);
}

// Tick 96 (step 4) is sample 22050 at 120 BPM and 44.1 kHz (the tick table). At 229.6875 samples
// per tick, sample 23050 is inside tick 100 (100.35 ticks).
#[test]
fn controls_start_stop_and_seek_the_playhead() {
    let (mut link, mut player) = playing(&f1_project(), &[1.0]);
    let mut out = vec![[0.0f32; 2]; 1000];
    player.process(&mut out);
    assert!(link.playing());
    assert_eq!(link.position().samples, 1000);

    link.send(Control::Stop).expect("room");
    out.fill([1.0; 2]);
    player.process(&mut out);
    assert!(!link.playing());
    let start = Position {
        samples: 0,
        ticks: Ticks(0),
    };
    assert_eq!(link.position(), start);
    assert_eq!(sounding_frames(&out), [] as [usize; 0], "stopped is silent");

    link.send(Control::Seek(Ticks(96))).expect("room");
    link.send(Control::Play).expect("room");
    player.process(&mut out);
    assert_eq!(
        sounding_frames(&out),
        [0],
        "play starts on the step at tick 96"
    );
    let after = Position {
        samples: 23_050,
        ticks: Ticks(100),
    };
    assert_eq!(link.position(), after);
}

// ---------------------------------------------------------------------------------------------
// Song mode. A clip plays pattern event e at every song tick T with start <= T < start + length and
// (T - start + offset) mod L == e; the playhead wraps from the loop end to the loop start (loop on)
// or from the song end to tick 0; voices are timed by an output clock, so sounds ring on across a
// wrap. Expected frames from Python's exact integers: at 44.1 kHz and 120 BPM tick t falls at
// floor(t * 229.6875).
// ---------------------------------------------------------------------------------------------

const P4_TWO_CLIPS: &str = include_str!("../../../tests/fixtures/p4_two_clips.json");
const P4_LOOP: &str = include_str!("../../../tests/fixtures/p4_loop.json");
const P4_OVERLAP: &str = include_str!("../../../tests/fixtures/p4_overlap.json");
const P4_MESSY: &str = include_str!("../../../tests/fixtures/p4_messy.json");

/// A player for `project` in song mode, every channel playing `frames`, told to play.
fn playing_song(project: &Project, frames: &[f32]) -> (RealtimeLink, Player) {
    let plan = plan_with_sample(project, PlayMode::Song, frames);
    let (mut link, player) = realtime::link(plan, project.tempo);
    link.send(Control::Play)
        .expect("the control ring starts empty");
    (link, player)
}

fn render_song(
    project: &Project,
    sample: &[f32],
    frames: usize,
    block_sizes: impl IntoIterator<Item = usize>,
) -> Vec<[f32; 2]> {
    let (_link, mut player) = playing_song(project, sample);
    render_player(&mut player, frames, block_sizes)
}

/// p4_two_clips with no clips, and pattern 1's four notes (data ticks 0, 96, 192, 288) at
/// velocities 0.125, 0.25, 0.375 and 0.5, so each sound names the data tick that made it.
fn tagged_song() -> Project {
    let mut project = Project::load_json(P4_TWO_CLIPS).expect("fixture loads");
    let pattern = project.patterns.get_mut(&PatternId(1)).expect("pattern 1");
    for (note, velocity) in pattern.notes.iter_mut().zip([0.125, 0.25, 0.375, 0.5]) {
        note.velocity = velocity;
    }
    project.playlist.clips.clear();
    project
}

/// Places pattern 1 on track 1 at `start` for `length` ticks, its window starting `offset` into
/// the pattern.
fn add_clip(project: &mut Project, start: i64, length: i64, offset: i64) -> ClipId {
    let id = project.playlist.next_clip_id;
    let clip = Clip {
        offset: Ticks(offset),
        ..Clip::of_pattern(PatternId(1), TrackId(1), Ticks(start), Ticks(length))
    };
    project.playlist.clips.insert(id, clip);
    project.playlist.next_clip_id = ClipId(id.0 + 1);
    id
}

/// Frame -> value for every frame of `out` that sounds, both channels checked equal.
fn sounding(out: &[[f32; 2]]) -> Vec<(usize, f32)> {
    out.iter()
        .enumerate()
        .filter(|(_, frame)| **frame != [0.0; 2])
        .map(|(index, frame)| {
            assert_eq!(frame[0], frame[1], "frame {index}: mono plays centered");
            (index, frame[0])
        })
        .collect()
}

// P1 at start 0, length 384, offset 96 plays data ticks 96/192/288 at song ticks 0/96/192, then
// data tick 0 at 288. At length 768 the window repeats the pattern from there. An offset one
// pattern length further (480) is the same window.
#[test]
fn a_clip_plays_its_window_of_the_pattern_from_its_offset() {
    let mut project = tagged_song();
    add_clip(&mut project, 0, 768, 96);
    project.validate().expect("valid");
    let out = render_song(&project, &[1.0], 176_400, [176_400]);
    let expected = [
        (0, 0.25),
        (22050, 0.375),
        (44100, 0.5),
        (66150, 0.125),
        (88200, 0.25),
        (110_250, 0.375),
        (132_300, 0.5),
        (154_350, 0.125),
    ];
    assert_eq!(sounding(&out), expected);
    let mut further = tagged_song();
    add_clip(&mut further, 0, 768, 480);
    let again = render_song(&further, &[1.0], 176_400, [176_400]);
    assert!(bits(&out) == bits(&again), "offset 480 is offset 96");
}

// A note that starts inside the window plays to its own end; the clip end never cuts a sounding
// voice. P1 for 96 ticks with a 30000-frame sample: the data-tick-0 note (0.125) sounds for frames
// 0..30000, past the clip end at 22050, and the note on the clip end itself (data tick 96) does not
// play. A muted clip at 768..1152 plays nothing but sets the song end, so there is no wrap within
// the bar.
#[test]
fn the_clip_end_never_cuts_a_sounding_note() {
    let mut project = tagged_song();
    add_clip(&mut project, 0, 96, 0);
    let muted = add_clip(&mut project, 768, 384, 0);
    project.playlist.clips.get_mut(&muted).expect("clip").muted = true;
    project.validate().expect("valid");
    let out = render_song(&project, &[1.0; 30_000], 88_200, block_sizes(3));
    let expected: Vec<(usize, f32)> = (0..30_000).map(|frame| (frame, 0.125)).collect();
    assert_eq!(sounding(&out), expected);
}

// Loop 0..96 on wraps every 22050 frames, and each pass starts the data-tick-0 note again while the
// one before still rings (30000 frames): two voices sound during each overlap.
#[test]
fn sounds_ring_on_across_a_wrap() {
    let mut project = tagged_song();
    add_clip(&mut project, 0, 384, 0);
    project.playlist.loop_range = Some(LoopRange {
        start: Ticks(0),
        end: Ticks(96),
        on: true,
    });
    project.validate().expect("valid");
    let out = render_song(&project, &[1.0; 30_000], 88_200, block_sizes(11));
    let voices = |frame: usize| {
        (0..4)
            .filter(|k| (k * 22050..k * 22050 + 30_000).contains(&frame))
            .count()
    };
    for (frame, value) in out.iter().enumerate() {
        let expected = 0.125 * voices(frame) as f32;
        assert_eq!(*value, [expected; 2], "frame {frame}");
    }
}

// The song end of p4_two_clips is tick 1152, sample 264600. A buffer that ends exactly there
// has not wrapped yet: the playhead reports sample 264600 (in tick 1152). The next buffer's
// first frame plays song sample 0, pattern 1's downbeat, and the playhead reports sample 1.
#[test]
fn a_buffer_that_ends_on_the_wrap_point_wraps_on_the_next_buffer() {
    let project = Project::load_json(P4_TWO_CLIPS).expect("fixture loads");
    let (link, mut player) = playing_song(&project, &[1.0]);
    let mut out = vec![[0.0f32; 2]; 264_600];
    player.process(&mut out);
    assert_eq!(
        link.position(),
        Position {
            samples: 264_600,
            ticks: Ticks(1152)
        }
    );
    let mut next = [[0.0f32; 2]; 1];
    player.process(&mut next);
    assert_eq!(
        next,
        [[1.0; 2]],
        "the first frame after the wrap is song sample 0"
    );
    assert_eq!(
        link.position(),
        Position {
            samples: 1,
            ticks: Ticks(0)
        }
    );
}

// Stop returns the playhead to the play-from tick, the last seek (tick 96 is sample 22050), and the
// next Play starts there again: pattern 1's beat on tick 96 is the first frame.
#[test]
fn stop_returns_to_the_last_seek() {
    let project = Project::load_json(P4_TWO_CLIPS).expect("fixture loads");
    let plan = plan_with_sample(&project, PlayMode::Song, &[1.0]);
    let (mut link, mut player) = realtime::link(plan, project.tempo);
    link.send(Control::Seek(Ticks(96))).expect("room");
    link.send(Control::Play).expect("room");
    let mut out = vec![[0.0f32; 2]; 1000];
    player.process(&mut out);
    assert_eq!(out[0], [1.0; 2]);
    link.send(Control::Stop).expect("room");
    player.process(&mut []);
    let at_96 = Position {
        samples: 22050,
        ticks: Ticks(96),
    };
    assert_eq!(link.position(), at_96);
    assert!(!link.playing());
    link.send(Control::Play).expect("room");
    let mut first = [[0.0f32; 2]; 1];
    player.process(&mut first);
    assert_eq!(first, [[1.0; 2]], "play starts on tick 96 again");
}

#[test]
fn song_mode_splits_render_the_same_bits_as_one_buffer() {
    let decaying: Vec<f32> = (0..DENSE_SAMPLE_FRAMES)
        .map(|i| 1.0 / (1.0 + i as f32))
        .collect();
    for (name, text, bars) in [
        ("p4_loop", P4_LOOP, 3),
        ("p4_messy", P4_MESSY, 2),
        ("p4_overlap", P4_OVERLAP, 2),
    ] {
        let project = Project::load_json(text).expect("fixture loads");
        let frames = frames_in_bars(&project, bars);
        let whole = bits(&render_song(&project, &decaying, frames, [frames]));
        assert!(
            whole.iter().any(|frame| *frame != [0, 0]),
            "{name}: rendered silence"
        );
        for seed in [1, 0x5eed, 0xdead_beef] {
            let split = bits(&render_song(&project, &decaying, frames, block_sizes(seed)));
            if let Some(frame) = whole.iter().zip(&split).position(|(a, b)| a != b) {
                panic!("{name}, seed {seed:#x}: frame {frame} differs from the one-buffer render");
            }
        }
    }
}

/// The dense project as a song: its pattern three times over (twice at 0, once at 96, on three
/// tracks), looping over ticks 0..48, a pass of 8252 frames at 174.5 BPM and 48 kHz.
fn dense_song() -> Project {
    let mut project = dense_project();
    for (track, start) in [(1, 0), (2, 0), (3, 96)] {
        let id = project.playlist.next_clip_id;
        let clip = Clip::of_pattern(PatternId(1), TrackId(track), Ticks(start), Ticks(384));
        project.playlist.clips.insert(id, clip);
        project.playlist.next_clip_id = ClipId(id.0 + 1);
    }
    project.playlist.loop_range = Some(LoopRange {
        start: Ticks(0),
        end: Ticks(48),
        on: true,
    });
    project.validate().expect("dense song is valid");
    project
}

#[test]
fn song_mode_process_never_touches_the_heap() {
    let project = dense_song();
    let sample = vec![0.5; DENSE_SAMPLE_FRAMES];
    let (mut link, mut player) = playing_song(&project, &sample);
    let mut out = vec![[0.0; 2]; 512];
    let (mut calls, mut swaps, mut wraps) = (0, 0, 0);
    let mut last = 0;
    for call in 0..1000 {
        if call % 100 == 50 {
            let plan = plan_with_sample(&project, PlayMode::Song, &sample);
            link.push_plan(plan).expect("same time base");
        }
        if call == 500 {
            link.send(Control::SetTempo(MilliBpm(140_000)))
                .expect("room in the control ring");
        }
        calls += heap_calls_during(|| player.process(&mut out));
        swaps += link.drain_retired();
        // A pass is longer than a buffer, so a wrap shows as the playhead moving back.
        let now = link.position().samples;
        wraps += usize::from(now < last);
        last = now;
    }
    assert_eq!(
        calls, 0,
        "Player::process made {calls} heap calls in song mode"
    );
    assert_eq!(swaps, 10, "every pushed plan was swapped in");
    assert!(wraps >= 50, "only {wraps} wraps");
}

// A clip 97 ticks long plays its note on tick 96, its last tick, also when a buffer starts
// exactly there (sample 22050): the clips that can sound in a buffer reach back by the longest
// clip's length. The song ends at tick 97, floor(97 * 229.6875) = 22279, and wraps to 0, so the
// two notes repeat 22279 frames later.
#[test]
fn a_note_on_the_last_tick_of_the_longest_clip_plays_from_a_buffer_start() {
    let mut project = tagged_song();
    add_clip(&mut project, 0, 97, 0);
    project.validate().expect("valid");
    let expected = [(0, 0.125), (22050, 0.25), (22279, 0.125), (44329, 0.25)];
    for sizes in [vec![44_558], vec![22050, 22508], vec![22050, 229, 1, 22278]] {
        let out = render_song(&project, &[1.0], 44_558, sizes.clone());
        assert_eq!(sounding(&out), expected, "buffers {sizes:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// The mixer in the player. Voices play into their channel's insert; inserts run in routing order;
// faders, pans and sends ramp over 256 frames to a new value; effects are created on the UI side
// and move to the player in plan updates, which arrive in order. Expected values from
// tests/fixtures/gen_p5_expected.py or by hand.
// ---------------------------------------------------------------------------------------------

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use dawsome_engine::processor::{ParamInfo, ProcessContext, Processor, ProcessorFactory, Registry};
use dawsome_engine::project::{Effect, EffectId, InsertId, ProcessorRef};
use dawsome_engine::transport::ParamTarget;

const P5_GAIN: &str = include_str!("../../../tests/fixtures/p5_gain.json");
const P5_MUTE: &str = include_str!("../../../tests/fixtures/p5_mute.json");
const P5_DELAY_MS: &str = include_str!("../../../tests/fixtures/p5_delay_ms.json");
const P5_DELAY_SYNC: &str = include_str!("../../../tests/fixtures/p5_delay_sync.json");
const P5_REVERB: &str = include_str!("../../../tests/fixtures/p5_reverb.json");
const P5_REVERB_NONE: &str = include_str!("../../../tests/fixtures/p5_reverb_none.json");
const P5_SATURATED: &str = include_str!("../../../tests/fixtures/p5_saturated.json");

fn p5(text: &str) -> Project {
    Project::load_json(text).expect("fixture loads")
}

#[test]
fn p5_any_buffer_split_renders_the_same_bits_as_one_buffer() {
    for (name, text) in [
        ("p5_gain", P5_GAIN),
        ("p5_delay_sync", P5_DELAY_SYNC),
        ("p5_reverb", P5_REVERB),
    ] {
        let project = p5(text);
        let frames = frames_in_bars(&project, 2);
        let whole = bits(&render(&project, &[1.0], frames, [frames]));
        assert!(
            whole.iter().any(|frame| *frame != [0, 0]),
            "{name}: silence"
        );
        for seed in [1, 0x5eed, 0xdead_beef] {
            let split = bits(&render(&project, &[1.0], frames, block_sizes(seed)));
            if let Some(frame) = whole.iter().zip(&split).position(|(a, b)| a != b) {
                panic!("{name}, seed {seed:#x}: frame {frame} differs from the one-buffer render");
            }
        }
    }
}

/// F1 with only its downbeat, played by a 100000-frame sample of 1.0: a constant 1.0 for the
/// whole first bar (88200 frames), so the output is the master's gain.
fn steady_project() -> Project {
    let mut project = f1_project();
    let pattern = project.patterns.get_mut(&PatternId(1)).expect("pattern 1");
    pattern.notes.truncate(1);
    project
}

const STEADY: [f32; 1] = [1.0];

fn steady_sample() -> Vec<f32> {
    vec![STEADY[0]; 100_000]
}

/// Ten 512-frame buffers of `player`, `before_call` run before call `n` (0-based).
fn ten_buffers(player: &mut Player, mut before_call: impl FnMut(usize)) -> Vec<[f32; 2]> {
    let mut out = vec![[0.0f32; 2]; 5120];
    for (call, buffer) in out.chunks_mut(512).enumerate() {
        before_call(call);
        player.process(buffer);
    }
    out
}

// Fader 0.8 → 0.4 is gain 1.0 → 0.125 ((0.4/0.8)³). Sent before the buffer that starts at frame
// 2048, the ramp keeps 1.0 on its first frame, passes 1 − 0.875·128/256 = 0.5625 half way, and is
// exactly 0.125 from frame 2048 + 256 on.
#[test]
fn a_fader_change_ramps_over_exactly_256_frames() {
    let project = steady_project();
    let sample = steady_sample();
    let (mut link, mut player) = playing(&project, &sample);
    let live = ten_buffers(&mut player, |call| {
        if call == 4 {
            link.send(Control::Param(
                ParamTarget::InsertFader(InsertId::MASTER),
                0.4,
            ))
            .expect("room");
        }
    });
    let left: Vec<f32> = live.iter().map(|frame| frame[0]).collect();
    assert!(
        left[..2048].iter().all(|&v| v == 1.0),
        "unity before the change"
    );
    assert_eq!(left[2048], 1.0, "the first ramp frame is the old gain");
    assert_eq!(left[2048 + 128], 0.5625);
    assert!(
        left[2048..2304].windows(2).all(|pair| pair[1] < pair[0]),
        "the ramp falls every frame"
    );
    assert_ne!(left[2303], 0.125, "still ramping on frame 255");
    assert!(
        left[2304..].iter().all(|&v| v == 0.125),
        "exactly the target after 256 frames"
    );
    assert!(live.iter().all(|frame| frame[0] == frame[1]));

    // The same change as a new plan ramps the same way, bit for bit.
    let mut quieter = project.clone();
    quieter.mixer.inserts[0].fader = 0.4;
    let (mut link, mut player) = playing(&project, &sample);
    let planned = ten_buffers(&mut player, |call| {
        if call == 4 {
            link.push_plan(plan_with_sample(
                &quieter,
                PlayMode::Pattern(PatternId(1)),
                &sample,
            ))
            .expect("same time base");
        }
    });
    assert!(
        bits(&planned) == bits(&live),
        "a plan ramps as a live change does"
    );
}

// A value present when a plan is first installed does not ramp, and a change made while stopped is
// in force at once when play starts.
#[test]
fn first_values_and_changes_while_stopped_do_not_ramp() {
    let sample = steady_sample();
    let mut quieter = steady_project();
    quieter.mixer.inserts[0].fader = 0.4;
    let (_link, mut player) = playing(&quieter, &sample);
    let first = ten_buffers(&mut player, |_| {});
    assert!(first[..2000].iter().all(|frame| *frame == [0.125; 2]));

    let project = steady_project();
    let plan = plan_with_sample(&project, PlayMode::Pattern(PatternId(1)), &sample);
    let (mut link, mut player) = realtime::link(plan, project.tempo);
    link.send(Control::Param(
        ParamTarget::InsertFader(InsertId::MASTER),
        0.4,
    ))
    .expect("room");
    link.send(Control::Play).expect("room");
    let set_first = ten_buffers(&mut player, |_| {});
    assert!(set_first[..2000].iter().all(|frame| *frame == [0.125; 2]));
}

// A channel pan drag is heard at once, no ramp: hard left gives (√2, 0).
#[test]
fn a_live_channel_pan_is_heard_at_once() {
    let project = steady_project();
    let sample = steady_sample();
    let (mut link, mut player) = playing(&project, &sample);
    let out = ten_buffers(&mut player, |call| {
        if call == 2 {
            link.send(Control::Param(ParamTarget::ChannelPan(ChannelId(1)), -1.0))
                .expect("room");
        }
    });
    assert!(out[..1024].iter().all(|frame| *frame == [1.0; 2]));
    assert!(out[1024..]
        .iter()
        .all(|frame| *frame == [std::f32::consts::SQRT_2, 0.0]));
}

// A send and an insert pan dragged while playing ramp to the new value (Python): channel 1 plays
// into insert 1, which reaches the master directly and through a send of 0 to insert 2, so the
// master hears 1.0; the send at 1.0 adds insert 2's 1.0: 2.0 a side. The master panned hard left
// folds the right side in, fold sin(π/2) = 1 and keep sin(0) = 0: (2 + 2, 0) = (4.0, 0.0).
#[test]
fn live_sends_and_insert_pans_are_heard() {
    let mut project = steady_project();
    let channel = project.channels.get_mut(&ChannelId(1)).expect("channel 1");
    channel.insert = InsertId(1);
    project.mixer.inserts[1].routes.insert(InsertId(2), 0.0);
    project.validate().expect("a send of 0 is valid");
    let sample = steady_sample();
    let (mut link, mut player) = playing(&project, &sample);
    let send = ParamTarget::Send {
        from: InsertId(1),
        to: InsertId(2),
    };
    let out = ten_buffers(&mut player, |call| {
        let (target, value) = match call {
            2 => (send, 1.0),
            6 => (ParamTarget::InsertPan(InsertId::MASTER), -1.0),
            _ => return,
        };
        link.send(Control::Param(target, value)).expect("room");
    });
    assert!(out[..1024].iter().all(|frame| *frame == [1.0; 2]));
    assert!(out[1024 + 256..3072].iter().all(|frame| *frame == [2.0; 2]));
    assert!(out[3072 + 256..].iter().all(|frame| *frame == [4.0, 0.0]));
}

/// Writes how many blocks it has processed into every frame, so its output shows whether it is
/// still the same instance, and counts the instances alive.
struct Counter {
    blocks: f32,
    alive: Arc<AtomicUsize>,
}

impl Processor for Counter {
    fn process(&mut self, _context: &ProcessContext, buffer: &mut [[f32; 2]]) {
        self.blocks += 1.0;
        buffer.fill([self.blocks; 2]);
    }

    fn set_param(&mut self, _index: usize, _value: f32) {}

    fn reset(&mut self) {
        self.blocks = 0.0;
    }
}

impl Drop for Counter {
    fn drop(&mut self) {
        self.alive.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct CounterFactory {
    created: Arc<AtomicUsize>,
    alive: Arc<AtomicUsize>,
}

impl ProcessorFactory for CounterFactory {
    fn id(&self) -> &'static str {
        "test.counter"
    }

    fn name(&self) -> &'static str {
        "Counter"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &[]
    }

    fn create(&self, _sample_rate: u32, _values: &[f32]) -> Box<dyn Processor> {
        self.created.fetch_add(1, Ordering::SeqCst);
        self.alive.fetch_add(1, Ordering::SeqCst);
        Box::new(Counter {
            blocks: 0.0,
            alive: Arc::clone(&self.alive),
        })
    }
}

/// A compiler that knows the native effects and the counter, and the counter's tallies.
fn counting_compiler() -> (Compiler, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let factory = CounterFactory::default();
    let (created, alive) = (Arc::clone(&factory.created), Arc::clone(&factory.alive));
    let mut registry = Registry::native();
    registry.register(factory);
    (Compiler::with_registry(registry), created, alive)
}

/// F1 without notes, so the only sound is what insert 1's effects make.
fn silent_f1() -> Project {
    let mut project = f1_project();
    project
        .patterns
        .get_mut(&PatternId(1))
        .expect("pattern 1")
        .notes
        .clear();
    project
}

fn compile_with(compiler: &mut Compiler, project: &Project, touched: &Touched) -> PlanUpdate {
    let sample = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(vec![1.0]));
    compiler
        .compile(project, touched, PlayMode::Pattern(PatternId(1)), sample)
        .expect("compiles")
}

/// Applies `edit` to `project` and compiles what it touched with `compiler`.
fn edited(compiler: &mut Compiler, project: &mut Project, edit: Edit) -> PlanUpdate {
    let applied = edit::apply(project, edit).expect("edit applies");
    compile_with(compiler, project, &applied.touched)
}

/// Adds effect `n + 1`, a `processor`, ten to an insert from insert 1 on.
fn add_nth_effect(n: u32, processor: &str) -> Edit {
    Edit::AddEffect {
        insert: InsertId(1 + u16::try_from(n / 10).expect("small")),
        slot: u8::try_from(n % 10).expect("below 10"),
        id: EffectId(n + 1),
        processor: ProcessorRef::new(processor),
    }
}

// MoveEffect keeps the instance. The counter in slot 0 of insert 1 writes its block count into
// insert 1, which reaches the master at unity: blocks 1, 2, 3, ... After moving it to slot 7 (an EQ
// at 0 dB swapped in front of it passes everything) the count goes on from 5 to 6.
#[test]
fn a_moved_effect_keeps_its_instance_and_its_state() {
    let (mut compiler, created, _alive) = counting_compiler();
    let mut project = silent_f1();
    for (slot, id, kind) in [(0, 1, "test.counter"), (7, 2, "dawsome.eq3")] {
        let add = Edit::AddEffect {
            insert: InsertId(1),
            slot,
            id: EffectId(id),
            processor: ProcessorRef::new(kind),
        };
        edit::apply(&mut project, add).expect("adds");
    }
    let plan = compile_with(&mut compiler, &project, &Touched::default());
    let (mut link, mut player) = realtime::link(plan, project.tempo);
    link.send(Control::Play).expect("room");
    let move_it = Edit::MoveEffect {
        effect: EffectId(1),
        slot: 7,
    };
    let mut update = Some(edited(&mut compiler, &mut project, move_it));
    let out = ten_buffers(&mut player, |call| {
        if call == 5 {
            link.push_plan(update.take().expect("one update"))
                .expect("same time base");
        }
    });
    let per_block: Vec<f32> = out.chunks(512).map(|block| block[0][0]).collect();
    assert_eq!(
        per_block,
        [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]
    );
    assert_eq!(created.load(Ordering::SeqCst), 1, "never created again");
    let strip = &project.mixer.inserts[1];
    let slots: Vec<(u32, u8)> = strip.effects.iter().map(|e| (e.id.0, e.slot)).collect();
    assert_eq!(slots, [(2, 0), (1, 7)]);
}

// An effect id this build has no processor for passes the audio unchanged: p5_delay_sync with its
// delay renamed sounds exactly like p5_reverb_none (the click through insert 1).
#[test]
fn an_unknown_processor_passes_the_audio_through() {
    let unknown = P5_DELAY_SYNC.replace(r#""id":"dawsome.delay""#, r#""id":"vendor.missing""#);
    assert_ne!(unknown, P5_DELAY_SYNC);
    let missing = p5(&unknown);
    let bypassed = render(&missing, &[1.0], 88_200, [512].repeat(173));
    let plain = render(&p5(P5_REVERB_NONE), &[1.0], 88_200, [512].repeat(173));
    assert!(bits(&bypassed) == bits(&plain));
    assert_eq!(sounding_frames(&bypassed), [0]);
}

// The meters hold each insert's peak after its fader, pan and mute until they are taken: the
// saturated mix peaks at 244.140625 on the master and 7.8125 on each insert; a muted insert
// (p5_mute's insert 2) shows nothing although its channel played.
#[test]
fn meters_hold_the_peak_after_the_mute_stage_until_taken() {
    let (link, mut player) = playing(&p5(P5_SATURATED), &[1.0]);
    let mut out = vec![[0.0f32; 2]; 88_200];
    player.process(&mut out);
    // The generator's 244.140625: 16 × 7.8125 = 125, times the master's 1.953125.
    assert_eq!(link.take_peak(InsertId::MASTER), [125.0 * 1.953_125; 2]);
    assert_eq!(link.take_peak(InsertId(1)), [7.8125; 2]);
    assert_eq!(
        link.take_peak(InsertId::MASTER),
        [0.0; 2],
        "taking clears it"
    );
    assert_eq!(link.take_peak(InsertId(600)), [0.0; 2], "no such insert");

    let (link, mut player) = playing(&p5(P5_MUTE), &[1.0]);
    player.process(&mut out);
    assert_eq!(link.take_peak(InsertId(2)), [0.0; 2]);
    assert_eq!(link.take_peak(InsertId(1)), [1.0; 2]);
}

// Stop clears the tails. p5_delay_sync echoes its click 16537 frames later; playing 1000 frames,
// stopping and playing again must sound exactly like a fresh start, with no echo left over from the
// first click at frame 15537.
#[test]
fn stop_clears_effect_tails() {
    let project = p5(P5_DELAY_SYNC);
    let fresh = render(&project, &[1.0], 88_200, [88_200]);
    let (mut link, mut player) = playing(&project, &[1.0]);
    player.process(&mut vec![[0.0f32; 2]; 1000]);
    link.send(Control::Stop).expect("room");
    player.process(&mut []);
    link.send(Control::Play).expect("room");
    let again = render_player(&mut player, 88_200, [88_200]);
    assert!(bits(&again) == bits(&fresh));
    assert_eq!(sounding_frames(&again), [16537, 33074, 49611, 66148, 82685]);
}

// Updates are applied in order, none merged. Twelve updates, more than the ring holds, so several
// wait at once: five counters added and all five removed, then counters 6 and 7 added. Exactly 6
// and 7 must run. Had only the newest waiting update been kept, counters 4 and 5 would never have
// been removed and 6 never installed.
#[test]
fn plan_updates_arrive_in_order_and_none_is_dropped() {
    let (mut compiler, _created, alive) = counting_compiler();
    let mut project = silent_f1();
    let plan = compile_with(&mut compiler, &project, &Touched::default());
    let (mut link, mut player) = realtime::link(plan, project.tempo);
    link.send(Control::Play).expect("room");
    let add = |id: u32| Edit::AddEffect {
        insert: InsertId(1),
        slot: u8::try_from(id).expect("small"),
        id: EffectId(id),
        processor: ProcessorRef::new("test.counter"),
    };
    let remove = |id| Edit::RemoveEffect { id: EffectId(id) };
    let edits = (1..=5)
        .map(add)
        .chain((1..=5).map(remove))
        .chain([add(6), add(7)]);
    for edit in edits {
        let update = edited(&mut compiler, &mut project, edit);
        link.push_plan(update).expect("same time base");
    }
    let mut returned = 0;
    for _ in 0..30 {
        player.process(&mut [[0.0; 2]; 64]);
        returned += link.drain_retired();
        compiler.release(link.take_freed_slots());
    }
    assert_eq!(returned, 12, "every update came back");
    assert_eq!(
        alive.load(Ordering::SeqCst),
        2,
        "counters 6 and 7 run, no other"
    );
    let mut out = [[0.0f32; 2]; 64];
    player.process(&mut out);
    assert!(out.iter().all(|frame| frame[0] > 0.0), "and they are heard");
    let ids: Vec<u32> = project.mixer.inserts[1]
        .effects
        .iter()
        .map(|e| e.id.0)
        .collect();
    assert_eq!(ids, [6, 7]);
}

// With the player gone (the app runs with no audio device), updates have nowhere to go: each is
// dropped at once, processors and all, and its slots come back, so editing with audio off
// holds nothing.
#[test]
fn updates_for_a_player_that_is_gone_are_dropped_and_their_slots_freed() {
    let (mut compiler, created, alive) = counting_compiler();
    let mut project = silent_f1();
    let plan = compile_with(&mut compiler, &project, &Touched::default());
    let (mut link, player) = realtime::link(plan, project.tempo);
    drop(player);
    for id in 1..=10_u32 {
        let add = Edit::AddEffect {
            insert: InsertId(1),
            slot: u8::try_from(id - 1).expect("small"),
            id: EffectId(id),
            processor: ProcessorRef::new("test.counter"),
        };
        let update = edited(&mut compiler, &mut project, add);
        link.push_plan(update).expect("same time base");
    }
    assert_eq!(created.load(Ordering::SeqCst), 10);
    assert_eq!(alive.load(Ordering::SeqCst), 0, "no update is kept");
    assert_eq!(link.take_freed_slots().len(), 10, "every slot comes back");
}

// A player that is alive but never runs takes nothing: 8 updates fill the plan ring and 64 more
// may wait behind it, so the 65th that would wait is refused before it is compiled and nothing
// is built for it. Once the player is gone, the waiting updates are dropped and their slots
// come back; the 8 in the ring go with the link.
#[test]
fn a_player_that_never_runs_is_sent_at_most_64_waiting_updates() {
    let (mut compiler, created, alive) = counting_compiler();
    let mut project = silent_f1();
    let plan = compile_with(&mut compiler, &project, &Touched::default());
    let (mut link, player) = realtime::link(plan, project.tempo);
    for n in 0..72 {
        link.check_room().expect("room for it");
        let update = edited(
            &mut compiler,
            &mut project,
            add_nth_effect(n, "test.counter"),
        );
        link.push_plan(update).expect("same time base");
    }
    assert_eq!(link.check_room(), Err(PlanQueueFull));
    assert_eq!(created.load(Ordering::SeqCst), 72);
    assert_eq!(alive.load(Ordering::SeqCst), 72, "every update is held");

    drop(player);
    link.check_room()
        .expect("nothing waits for a player that is gone");
    assert_eq!(
        link.take_freed_slots().len(),
        64,
        "the waiting updates' slots"
    );
    assert_eq!(alive.load(Ordering::SeqCst), 8, "the updates in the ring");
    drop(link);
    assert_eq!(alive.load(Ordering::SeqCst), 0);
}

// A parameter edit reaches the running delay through the update, and a live value through the
// control ring: at mix 0, p5_delay_ms is only its dry click.
#[test]
fn effect_parameters_reach_the_running_processor() {
    let project = p5(P5_DELAY_MS);
    let mut compiler = Compiler::new();
    let plan = compile_with(&mut compiler, &project, &Touched::default());
    let (mut link, mut player) = realtime::link(plan, project.tempo);
    let mut edited_project = project.clone();
    let dry = Edit::SetEffectParam {
        effect: EffectId(1),
        key: "mix".into(),
        value: Some(0.0),
    };
    link.push_plan(edited(&mut compiler, &mut edited_project, dry))
        .expect("same time base");
    link.send(Control::Play).expect("room");
    let out = render_player(&mut player, 9261, [9261]);
    assert_eq!(sounding_frames(&out), [0]);
    assert_eq!(out[0], [1.0; 2]);

    let (mut link, mut player) = playing(&project, &[1.0]);
    let mix = ParamTarget::Effect {
        effect: EffectId(1),
        param: 3,
    };
    link.send(Control::Param(mix, 0.0)).expect("room");
    let out = render_player(&mut player, 9261, [9261]);
    assert_eq!(sounding_frames(&out), [0]);
}

/// The dense song with a mixer: channels 1 and 2 into insert 1 (EQ, +6 dB at 1 kHz), sending
/// half to insert 2 (a delay synced to 3 steps); channel 3 into insert 2; channel 4 into
/// insert 3 (a reverb). Effect ids 1 to 3.
fn mixed_dense_song() -> Project {
    let mut project = dense_song();
    for (channel, insert) in [(1, 1), (2, 1), (3, 2), (4, 3)] {
        project
            .channels
            .get_mut(&ChannelId(channel))
            .expect("channel")
            .insert = InsertId(insert);
    }
    project.mixer.inserts[1].routes.insert(InsertId(2), 0.5);
    let effect = |id, kind: &str, params: &[(&str, f32)]| Effect {
        id: EffectId(id),
        slot: 0,
        processor: ProcessorRef {
            params: params.iter().map(|&(k, v)| (k.to_owned(), v)).collect(),
            ..ProcessorRef::new(kind)
        },
    };
    project.mixer.inserts[1].effects = vec![effect(1, "dawsome.eq3", &[("mid_gain_db", 6.0)])];
    project.mixer.inserts[2].effects = vec![effect(2, "dawsome.delay", &[("sync_steps", 3.0)])];
    project.mixer.inserts[3].effects = vec![effect(3, "dawsome.reverb", &[])];
    project.mixer.next_effect_id = EffectId(4);
    project.validate().expect("mixed song is valid");
    project
}

// The audio thread's rule with the mixer in play: three effects, twenty plan updates (effects
// added and removed, parameters, faders, sends, a move), live parameter values, a Stop and
// more than 50 song wraps, and still no heap call inside Player::process.
#[test]
fn process_never_touches_the_heap_with_the_mixer_in_play() {
    let mut project = mixed_dense_song();
    let mut compiler = Compiler::new();
    let sample = vec![0.5; DENSE_SAMPLE_FRAMES];
    let compile = |compiler: &mut Compiler, project: &Project, touched: &Touched| {
        let same = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(sample.clone()));
        compiler
            .compile(project, touched, PlayMode::Song, same)
            .expect("compiles")
    };
    let plan = compile(&mut compiler, &project, &Touched::default());
    let (mut link, mut player) = realtime::link(plan, project.tempo);
    link.send(Control::Play).expect("room");
    let edits = [
        Edit::SetEffectParam {
            effect: EffectId(1),
            key: "mid_gain_db".into(),
            value: Some(-4.0),
        },
        Edit::AddEffect {
            insert: InsertId(4),
            slot: 2,
            id: EffectId(4),
            processor: ProcessorRef::new("dawsome.delay"),
        },
        Edit::SetInsertVolume {
            insert: InsertId(2),
            fader: 0.6,
        },
        Edit::UnaddEffect { id: EffectId(4) },
        Edit::SetSend {
            from: InsertId(1),
            to: InsertId(2),
            amount: Some(0.9),
        },
        Edit::MoveEffect {
            effect: EffectId(3),
            slot: 4,
        },
        Edit::RemoveEffect { id: EffectId(2) },
        Edit::SetInsertPan {
            insert: InsertId(3),
            pan: -0.5,
        },
        Edit::RestoreEffect {
            insert: InsertId(2),
            slot: 0,
            id: EffectId(2),
            processor: ProcessorRef::new("dawsome.delay"),
        },
        Edit::SetEffectParam {
            effect: EffectId(3),
            key: "room".into(),
            value: Some(0.9),
        },
    ];
    let params = [
        ParamTarget::InsertFader(InsertId(1)),
        ParamTarget::InsertPan(InsertId(2)),
        ParamTarget::Send {
            from: InsertId(1),
            to: InsertId(2),
        },
        ParamTarget::ChannelVolume(ChannelId(3)),
        ParamTarget::ChannelPan(ChannelId(4)),
        ParamTarget::Effect {
            effect: EffectId(1),
            param: 4,
        },
        // The delay's feedback.
        ParamTarget::Effect {
            effect: EffectId(2),
            param: 2,
        },
    ];
    let mut out = vec![[0.0; 2]; 512];
    let (mut calls, mut pushed, mut returned, mut wraps, mut last) = (0, 0, 0, 0, 0);
    for call in 0..1000 {
        // The UI half's work between calls, outside the measurement.
        if call % 50 == 25 {
            let edit = edits[pushed % edits.len()].clone();
            let applied = edit::apply(&mut project, edit).expect("edit applies");
            let update = compile(&mut compiler, &project, &applied.touched);
            link.push_plan(update).expect("same time base");
            pushed += 1;
        }
        if call % 7 == 3 {
            // In turn: `call % params.len()` would pick one target forever whenever the list's
            // length shares a factor with 7.
            let target = params[(call / 7) % params.len()];
            let value = if call % 2 == 0 { 0.3 } else { 0.7 };
            link.send(Control::Param(target, value)).expect("room");
        }
        if call == 600 {
            link.send(Control::Stop).expect("room");
        }
        if call == 601 {
            link.send(Control::Play).expect("room");
        }
        calls += heap_calls_during(|| player.process(&mut out));
        returned += link.drain_retired();
        compiler.release(link.take_freed_slots());
        let now = link.position().samples;
        wraps += usize::from(now < last);
        last = now;
    }
    assert_eq!(calls, 0, "Player::process made {calls} heap calls");
    assert_eq!((pushed, returned), (20, 20), "every update was applied");
    assert!(wraps >= 50, "only {wraps} wraps");
}

// Play, Stop and Seek reset every processor inside one audio callback, which at 48 kHz has
// 10.67 ms for 512 frames. A probe, run by hand in release and in dev:
// cargo test [--release] -p dawsome-engine --test player -- --ignored --nocapture delay_resets
#[test]
#[ignore = "timing probe, run by hand"]
fn delay_resets_fit_in_an_audio_callback() {
    use std::time::{Duration, Instant};

    for count in [16, 160] {
        let mut project = silent_f1();
        project.sample_rate = 48_000;
        for n in 0..count {
            edit::apply(&mut project, add_nth_effect(n, "dawsome.delay")).expect("adds");
        }
        let plan = compile_with(&mut Compiler::new(), &project, &Touched::default());
        let (mut link, mut player) = realtime::link(plan, project.tempo);
        let mut stop_after = |frames: usize| {
            link.send(Control::Play).expect("room");
            let mut buffer = [[0.0f32; 2]; 512];
            for _ in 0..frames.div_ceil(512) {
                player.process(&mut buffer);
            }
            link.send(Control::Stop).expect("room");
            let start = Instant::now();
            player.process(&mut []);
            start.elapsed()
        };
        let short = (0..20).map(|_| stop_after(512)).max().expect("measured");
        // Past the 192000 frames (4 s) a line holds, so every frame of it was written.
        let whole = (0..3).map(|_| stop_after(200_000)).max().expect("measured");
        println!("{count} delays: Stop after 512 frames {short:?}, after a whole line {whole:?}");
        assert!(
            short < Duration::from_millis(1),
            "{count} delays: {short:?}"
        );
    }
}
