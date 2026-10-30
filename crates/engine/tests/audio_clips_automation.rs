//! Audio clips and automation in the engine: the audio clip and automation edits with their exact
//! inverses and refusals, and what the player does with them: automation set per sample or on the
//! 32-sample control grid, a stream that is not ready, a preview, a count-in and the record tap,
//! and none of it touching the heap on the audio thread. Expected values come from
//! tests/fixtures/p8_expected.json (tests/fixtures/gen_p8_expected.py) or are written out by hand
//! beside each test.

use std::convert::Infallible;
use std::path::Path;
use std::sync::{Arc, Mutex};

use dawsome_engine::compile::{ClipHint, ClipSound, Compiler, Sounds};
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, Edit, EditError, Touched};
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::processor::{ParamInfo, ProcessContext, Processor, ProcessorFactory, Registry};
use dawsome_engine::project::{
    AutomationId, AutomationPoint, AutomationTarget, ChannelId, Clip, ClipId, Curve, Effect,
    EffectId, InsertId, Pattern, PatternId, ProcessorRef, Project, TrackId,
};
use dawsome_engine::realtime::{self, EngineEvent, Player, RealtimeLink};
use dawsome_engine::source::{ClipSource, MemorySource, SourceRead};
use dawsome_engine::time::{MilliBpm, Ticks};
use dawsome_engine::transport::{Control, ParamTarget};

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

fn fixture(name: &str) -> Project {
    let text = std::fs::read_to_string(Path::new(FIXTURES).join(name)).expect("fixture reads");
    Project::load_json(&text).expect("fixture loads")
}

fn json(project: &Project) -> String {
    project.to_json().expect("serializes")
}

fn expected() -> serde_json::Value {
    let text = std::fs::read_to_string(Path::new(FIXTURES).join("p8_expected.json"))
        .expect("p8_expected.json");
    serde_json::from_str(&text).expect("parses")
}

/// Applies `edit`, then its inverse, and checks the document comes back byte for byte; returns
/// what the edit touched.
fn round_trip(project: &mut Project, edit: Edit) -> Touched {
    let before = json(project);
    let applied = edit::apply(project, edit.clone()).unwrap_or_else(|e| panic!("{edit:?}: {e}"));
    assert_ne!(json(project), before, "{edit:?} changed nothing");
    edit::apply(project, applied.inverse).expect("the inverse applies");
    assert_eq!(
        json(project),
        before,
        "{edit:?}: the inverse restores the bytes"
    );
    applied.touched
}

fn ramp(frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|k| ((k % 1000) + 1) as f32 / 1024.0)
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Edits
// ---------------------------------------------------------------------------------------------

#[test]
fn every_audio_clip_edit_has_an_exact_inverse() {
    let mut project = fixture("p8_trim.json");
    let add = Edit::AddAudioClip {
        channel: ChannelId(2),
        name: "vox".into(),
        sample: "samples/vox.wav".into(),
        clip: ClipId(4),
        track: TrackId(4),
        start: Ticks(384),
        length: Ticks(96),
    };
    let touched = round_trip(&mut project, add.clone());
    assert!(touched.playlist && touched.channels.contains(&ChannelId(2)));
    // The add makes exactly the channel and the clip written out here.
    let mut added = project.clone();
    edit::apply(&mut added, add).expect("adds");
    let text = json(&added);
    assert!(text.contains(r#"{"id":2,"name":"vox","kind":"audio","sample":"samples/vox.wav","volume":1.0,"pan":0.0,"mute":false}"#), "{text}");
    assert!(
        text.contains(r#"{"id":4,"track":4,"start":384,"length":96,"kind":"audio","channel":2}"#),
        "{text}"
    );
    for edit in [
        Edit::TrimAudioClip {
            clip: ClipId(1),
            start: Ticks(24),
            length: Ticks(72),
            frame_offset: 100 + 5512,
        },
        Edit::SetClipGain {
            clip: ClipId(1),
            gain: 4.0,
        },
        Edit::SetClipMute {
            ids: vec![ClipId(1), ClipId(3)],
            muted: true,
        },
        Edit::SetClipMute {
            ids: vec![ClipId(3), ClipId(2)],
            muted: false,
        },
    ] {
        assert!(round_trip(&mut project, edit).playlist);
    }
}

#[test]
fn every_automation_edit_has_an_exact_inverse() {
    let mut project = fixture("p8_auto_curves.json");
    let id = AutomationId(1);
    let point = |tick, value| AutomationPoint::linear(Ticks(tick), value);
    for edit in [
        Edit::AddAutomationPoint {
            automation: id,
            point: point(192, 0.75),
        },
        Edit::MoveAutomationPoint {
            automation: id,
            from_tick: Ticks(384),
            tick: Ticks(400),
            value: 0.9,
        },
        Edit::SetSegmentCurve {
            automation: id,
            tick: Ticks(0),
            curve: Curve::Exp(-0.3),
        },
        Edit::DeleteAutomationPoint {
            automation: id,
            tick: Ticks(768),
        },
        Edit::DeleteAutomationClip { automation: id },
        Edit::AddAutomationClip {
            automation: AutomationId(2),
            clip: ClipId(3),
            target: AutomationTarget::ChannelPan(ChannelId(1)),
            track: TrackId(3),
            start: Ticks(0),
            length: Ticks(1536),
            points: vec![point(0, 0.5), point(1536, 0.5)],
        },
    ] {
        let touched = round_trip(&mut project, edit.clone());
        assert!(!touched.automation.is_empty(), "{edit:?}");
    }
}

// Refused edits change nothing: a second automation on one target, the last point deleted, a point
// moved past its neighbor, gain 5, and a tick offset on an audio clip.
#[test]
fn refused_audio_clip_and_automation_edits_change_nothing() {
    let curves = fixture("p8_auto_curves.json");
    let trim = fixture("p8_trim.json");
    let one_point = {
        let mut project = fixture("p8_auto_linear.json");
        edit::apply(
            &mut project,
            Edit::DeleteAutomationPoint {
                automation: AutomationId(1),
                tick: Ticks(384),
            },
        )
        .expect("one point left");
        project
    };
    let mut offset = Clip::of_audio(ChannelId(1), TrackId(4), Ticks(0), Ticks(96));
    offset.offset = Ticks(24);
    let cases = [
        (
            &curves,
            Edit::AddAutomationClip {
                automation: AutomationId(2),
                clip: ClipId(3),
                target: AutomationTarget::ChannelVolume(ChannelId(1)),
                track: TrackId(3),
                start: Ticks(0),
                length: Ticks(384),
                points: vec![AutomationPoint::linear(Ticks(0), 0.5)],
            },
            EditError::AlreadyAutomated,
        ),
        (
            &one_point,
            Edit::DeleteAutomationPoint {
                automation: AutomationId(1),
                tick: Ticks(0),
            },
            EditError::LastPoint,
        ),
        (
            &curves,
            Edit::MoveAutomationPoint {
                automation: AutomationId(1),
                from_tick: Ticks(384),
                tick: Ticks(768),
                value: 0.5,
            },
            EditError::PointOutOfPlace { tick: 768 },
        ),
        (
            &trim,
            Edit::SetClipGain {
                clip: ClipId(1),
                gain: 5.0,
            },
            EditError::InvalidGain(5.0),
        ),
        (
            &trim,
            Edit::PlaceClips {
                first: ClipId(4),
                clips: vec![offset],
            },
            EditError::Invalid(
                dawsome_engine::project::ValidationError::AudioClipTickOffset {
                    clip: 4,
                    offset: 24,
                },
            ),
        ),
        (
            &trim,
            Edit::SetClipGain {
                clip: ClipId(1),
                gain: f32::NAN,
            },
            EditError::InvalidGain(f32::NAN),
        ),
    ];
    for (project, edit, error) in cases {
        let mut project = project.clone();
        let before = json(&project);
        let refused = edit::apply(&mut project, edit.clone());
        match (&refused, &error) {
            // NaN is never equal to itself, so its refusal is matched by kind.
            (Err(EditError::InvalidGain(got)), EditError::InvalidGain(want)) if want.is_nan() => {
                assert!(got.is_nan())
            }
            _ => assert_eq!(refused.map(drop), Err(error), "{edit:?}"),
        }
        assert_eq!(json(&project), before, "{edit:?} changed the document");
    }
}

// A drop on an existing channel id, or one that is not the next clip id, is refused; so is an
// add whose new clip would land past the song's end, and the channel does not stay behind.
#[test]
fn an_audio_clip_add_takes_the_next_ids_or_nothing() {
    let mut project = fixture("p8_trim.json");
    let before = json(&project);
    let add = |channel, clip, start| Edit::AddAudioClip {
        channel: ChannelId(channel),
        name: "vox".into(),
        sample: "vox.wav".into(),
        clip: ClipId(clip),
        track: TrackId(1),
        start: Ticks(start),
        length: Ticks(96),
    };
    assert!(edit::apply(&mut project, add(1, 4, 0)).is_err());
    assert!(edit::apply(&mut project, add(2, 9, 0)).is_err());
    assert!(edit::apply(&mut project, add(2, 4, 38_400_000)).is_err());
    assert_eq!(json(&project), before);
}

// ---------------------------------------------------------------------------------------------
// Playback
// ---------------------------------------------------------------------------------------------

/// Compiles `project` to play `mode`, every sound file `frames`.
fn compiled(
    compiler: &mut Compiler,
    project: &Project,
    mode: PlayMode,
    frames: &[f32],
) -> PlanUpdate {
    let same = |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(frames.to_vec()));
    compiler
        .compile(project, &Touched::default(), mode, same)
        .expect("compiles")
}

/// A player of `project` in song mode with every file `frames`, told to play.
fn song_player(project: &Project, frames: &[f32]) -> (RealtimeLink, Player) {
    let plan = compiled(&mut Compiler::new(), project, PlayMode::Song, frames);
    let (mut link, player) = realtime::link(plan, project.tempo);
    link.send(Control::Play).expect("room");
    (link, player)
}

/// `frames` frames from `player`, `block` at a time.
fn render(player: &mut Player, frames: usize, block: usize) -> Vec<[f32; 2]> {
    let mut out = vec![[f32::NAN; 2]; frames];
    for piece in out.chunks_mut(block) {
        player.process(piece);
    }
    out
}

fn bits(frames: &[[f32; 2]]) -> Vec<[u32; 2]> {
    frames.iter().map(|frame| frame.map(f32::to_bits)).collect()
}

// A constant automation equal to the stored value renders exactly what no automation does
// (0.125 of the 0..4 range is the stored 0.5).
#[test]
fn a_constant_automation_at_the_stored_value_changes_no_bit() {
    let click = [1.0];
    let mut automated = fixture("p8_auto_linear.json");
    let points = &mut automated
        .automation
        .list
        .get_mut(&AutomationId(1))
        .expect("automation 1")
        .points;
    *points = vec![AutomationPoint::linear(Ticks(0), 0.125)];
    let mut plain = automated.clone();
    plain.playlist.clips.remove(&ClipId(2));
    let frames = 264_600;
    let (_a, mut with) = song_player(&automated, &click);
    let (_b, mut without) = song_player(&plain, &click);
    assert_eq!(
        bits(&render(&mut with, frames, 512)),
        bits(&render(&mut without, frames, 512))
    );
}

// Where two windows of one automation overlap, the one that starts later rules. The points
// hold 0.25 from tick 0 and step to 0.75 at 384. Window 1 (0..384, offset 0) shows the 0.25;
// window 2 (192..384, offset 384) shows 0.75: steps 8..15 (ticks 192..360) play 4 · 0.75 = 3.0,
// steps 0..7 play 1.0, and bar 2 holds the end of window 2, the one of the two that started
// later (both end at 384): 3.0.
#[test]
fn where_windows_overlap_the_later_one_rules() {
    let mut project = fixture("p8_auto_linear.json");
    let automation = project
        .automation
        .list
        .get_mut(&AutomationId(1))
        .expect("automation 1");
    automation.points = vec![AutomationPoint {
        curve: Curve::Hold,
        ..AutomationPoint::linear(Ticks(0), 0.25)
    }];
    let clips = &mut project.playlist.clips;
    clips.insert(
        ClipId(2),
        Clip::of_automation(AutomationId(1), TrackId(2), Ticks(0), Ticks(384)),
    );
    let mut later = Clip::of_automation(AutomationId(1), TrackId(3), Ticks(192), Ticks(192));
    later.offset = Ticks(384);
    clips.insert(ClipId(3), later);
    project.playlist.next_clip_id = ClipId(4);
    let automation = project
        .automation
        .list
        .get_mut(&AutomationId(1))
        .expect("1");
    automation
        .points
        .push(AutomationPoint::linear(Ticks(384), 0.75));
    project.validate().expect("valid");
    let (_link, mut player) = song_player(&project, &[1.0]);
    let out = render(&mut player, 176_400, 512);
    // Step k at 120 BPM and 44.1 kHz is floor(k · 24 · 229.6875) = floor(5512.5 k).
    let step = |k: u64| (k * 11_025 / 2) as usize;
    for k in 0..8 {
        assert_eq!(out[step(k)], [1.0; 2], "step {k}");
    }
    for k in 8..16 {
        assert_eq!(out[step(k)], [3.0; 2], "step {k}");
    }
    for k in 16..32 {
        assert_eq!(
            out[step(k)],
            [3.0; 2],
            "step {k}: the later window's end holds"
        );
    }
}

// A tension too small for the exponential formula (below about 1.85e-17 it is 0/0) plays the
// straight line, finite: p8_auto_curves with exp(1e-20) on bar 3 runs 0.25 → 0.0 linearly over
// 176400..264600, so step 40 at frame 220500 (x = 0.5) plays 4 · 0.125 = 0.5.
#[test]
fn a_vanishing_exponential_tension_plays_the_straight_line() {
    let text = std::fs::read_to_string(Path::new(FIXTURES).join("p8_auto_curves.json"))
        .expect("fixture reads")
        .replace(r#"{"exp":0.5}"#, r#"{"exp":1e-20}"#);
    let project = Project::load_json(&text).expect("a tiny tension is valid");
    let (_link, mut player) = song_player(&project, &[1.0]);
    let out = render(&mut player, 264_600, 512);
    let bad = out
        .iter()
        .position(|frame| frame.iter().any(|v| !v.is_finite()));
    assert_eq!(bad, None, "first frame that is not finite");
    assert_eq!(out[220_500], [0.5; 2]);
}

/// p8_auto_linear (44.1 kHz, 120 BPM) with 64 channels, each with a volume automation of 21
/// points over the 10 s the song plays (a point every beat, curves in turn), so 64 automations
/// are worked out frame by frame.
fn sixty_four_automations() -> Project {
    let mut project = fixture("p8_auto_linear.json");
    project.playlist.clips.remove(&ClipId(2));
    project.automation = Default::default();
    let curves = [Curve::Linear, Curve::Smooth, Curve::Exp(0.4), Curve::Hold];
    for n in 0..64u32 {
        let channel = project.next_channel_id;
        let add = Edit::AddSamplerChannel {
            id: channel,
            name: format!("ch {n}"),
            sample: "impulse.wav".into(),
        };
        edit::apply(&mut project, add).expect("adds a channel");
        let points = (0..21)
            .map(|beat| AutomationPoint {
                curve: curves[(beat + n as usize) % 4],
                ..AutomationPoint::linear(Ticks(96 * beat as i64), (beat % 2) as f32 * 0.8 + 0.1)
            })
            .collect();
        let automate = Edit::AddAutomationClip {
            automation: project.automation.next_id,
            clip: project.playlist.next_clip_id,
            target: AutomationTarget::ChannelVolume(channel),
            track: TrackId(2),
            start: Ticks(0),
            length: Ticks(96 * 20),
            points,
        };
        edit::apply(&mut project, automate).expect("automates");
    }
    project
}

// Audio-rate automation is cheap enough for many: 64 of them render 10 s of audio (441000
// frames, 512 a block) in under 5 % of realtime. Measured 2026-10-21 on this workspace's build
// machine: 115 ms in the test build, 103 ms with --release; 1088 ms and 1008 ms when every
// frame worked out its segment's bounds again.
#[test]
fn sixty_four_audio_rate_automations_render_well_under_realtime() {
    let project = sixty_four_automations();
    let (_link, mut player) = song_player(&project, &[1.0]);
    let mut block = [[0.0f32; 2]; 512];
    let started = std::time::Instant::now();
    for _ in 0..441_000 / 512 {
        player.process(&mut block);
    }
    let took = started.elapsed();
    eprintln!("64 audio-rate automations, 10 s of audio: {took:?}");
    assert!(
        took < std::time::Duration::from_millis(500),
        "10 s of audio took {took:?}"
    );
}

/// A test effect with one parameter, "mix" 0..1, that records every value it is given with
/// the frames it has processed by then (the song sample, when play starts at 0 and never
/// wraps).
struct Recorder {
    processed: u64,
    calls: Arc<Mutex<Vec<(u64, f32)>>>,
}

impl Processor for Recorder {
    fn process(&mut self, _context: &ProcessContext, buffer: &mut [[f32; 2]]) {
        self.processed += buffer.len() as u64;
    }

    fn set_param(&mut self, _index: usize, value: f32) {
        self.calls
            .lock()
            .expect("lock")
            .push((self.processed, value));
    }

    fn reset(&mut self) {}
}

struct RecorderFactory(Arc<Mutex<Vec<(u64, f32)>>>);

const MIX: ParamInfo = ParamInfo {
    key: "mix",
    label: "Mix",
    min: 0.0,
    max: 1.0,
    default: 0.3,
    unit: "%",
    whole: false,
};

impl ProcessorFactory for RecorderFactory {
    fn id(&self) -> &'static str {
        "test.recorder"
    }

    fn name(&self) -> &'static str {
        "Recorder"
    }

    fn params(&self) -> &'static [ParamInfo] {
        std::slice::from_ref(&MIX)
    }

    fn create(&self, _sample_rate: u32, _values: &[f32]) -> Box<dyn Processor> {
        Box::new(Recorder {
            processed: 0,
            calls: Arc::clone(&self.0),
        })
    }
}

/// 44.1 kHz at 120 BPM: an empty pattern clip over bar 1 (so the song does not wrap), the
/// recorder in insert 1, and its mix automated 0 → 1 over ticks 0..96.
fn grid_project() -> Project {
    let mut project = Project::new(44_100, MilliBpm(120_000), 96);
    project.patterns.insert(
        PatternId(1),
        Pattern {
            name: "Pattern 1".into(),
            length_bars: 1,
            copied_from: None,
            notes: Vec::new(),
        },
    );
    project.mixer.inserts[1].effects.push(Effect {
        id: EffectId(1),
        slot: 0,
        processor: ProcessorRef::new("test.recorder"),
    });
    project.mixer.next_effect_id = EffectId(2);
    edit::apply(
        &mut project,
        Edit::PlaceClips {
            first: ClipId(1),
            clips: vec![Clip::of_pattern(
                PatternId(1),
                TrackId(1),
                Ticks(0),
                Ticks(384),
            )],
        },
    )
    .expect("places");
    edit::apply(
        &mut project,
        Edit::AddAutomationClip {
            automation: AutomationId(1),
            clip: ClipId(2),
            target: AutomationTarget::EffectParam {
                effect: EffectId(1),
                key: "mix".into(),
            },
            track: TrackId(2),
            start: Ticks(0),
            length: Ticks(96),
            points: vec![
                AutomationPoint::linear(Ticks(0), 0.0),
                AutomationPoint::linear(Ticks(96), 1.0),
            ],
        },
    )
    .expect("automates");
    project
}

/// The values the recorder got over half a bar, rendered `block` frames at a time (or in the
/// seeded random sizes when `block` is 0).
fn grid_calls(block: usize) -> Vec<(u64, f32)> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut registry = Registry::native();
    registry.register(RecorderFactory(Arc::clone(&calls)));
    let project = grid_project();
    let plan = compiled(
        &mut Compiler::with_registry(registry),
        &project,
        PlayMode::Song,
        &[],
    );
    let (mut link, mut player) = realtime::link(plan, project.tempo);
    link.send(Control::Play).expect("room");
    let mut out = vec![[0.0; 2]; 44_100];
    let mut rest: &mut [[f32; 2]] = &mut out;
    let mut seed = 0x2545_f491_u32;
    while !rest.is_empty() {
        let size = if block > 0 {
            block
        } else {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            [1, 7, 31, 32, 33, 64, 500, 4097][seed as usize % 8]
        };
        let (piece, later) = rest.split_at_mut(size.min(rest.len()));
        player.process(piece);
        rest = later;
    }
    let calls = calls.lock().expect("lock").clone();
    calls
}

// (6) A control-rate target is set on every song sample divisible by 32 and on its segment and
// window edges, only when the value changes: the 691 calls the generator lists, the last at
// the window's end, 22050, and none after; the same calls for any block sizes.
#[test]
fn a_processor_parameter_is_set_on_the_32_sample_grid() {
    let expected: Vec<(u64, f32)> = expected()["grid"]["calls"]
        .as_array()
        .expect("calls")
        .iter()
        .map(|call| {
            (
                call[0].as_u64().expect("a sample"),
                call[1].as_f64().expect("a value") as f32,
            )
        })
        .collect();
    assert_eq!(expected.len(), 691);
    assert_eq!(grid_calls(512), expected);
    assert_eq!(grid_calls(0), expected, "random block sizes");
}

/// A stream whose `fail_on`th read gives `fault`: not ready for that read, or failed for good
/// (silent from then on).
struct Flaky {
    frames: Vec<f32>,
    reads: usize,
    fail_on: usize,
    fault: SourceRead,
}

impl ClipSource for Flaky {
    fn frames(&self) -> u64 {
        self.frames.len() as u64
    }

    fn channels(&self) -> u8 {
        1
    }

    fn read(&mut self, frame: u64, out: &mut [[f32; 2]]) -> SourceRead {
        self.reads += 1;
        if self.reads == self.fail_on {
            out.fill([0.0; 2]);
            return self.fault;
        }
        if self.fault == SourceRead::Failed && self.reads > self.fail_on {
            out.fill([0.0; 2]);
            return SourceRead::Ready;
        }
        for (at, slot) in (frame as usize..).zip(out.iter_mut()) {
            let value = self.frames.get(at).copied().unwrap_or(0.0);
            *slot = [value; 2];
        }
        SourceRead::Ready
    }
}

/// [`Sounds`] whose every clip streams from a [`Flaky`] source.
struct FlakySounds {
    frames: Vec<f32>,
    fail_on: usize,
    fault: SourceRead,
}

impl Sounds for FlakySounds {
    type Error = Infallible;

    fn sample(&mut self, _path: &Path) -> Result<Option<SampleBuffer>, Infallible> {
        Ok(Some(SampleBuffer::from(self.frames.clone())))
    }

    fn clip(&mut self, _path: &Path, hint: ClipHint) -> Result<ClipSound, Infallible> {
        assert_eq!(hint.first_frame, 0);
        Ok(ClipSound::Stream(Box::new(Flaky {
            frames: self.frames.clone(),
            reads: 0,
            fail_on: self.fail_on,
            fault: self.fault,
        })))
    }
}

/// One audio channel ("saw") with one clip of it over bar 1, at 44.1 kHz and 120 BPM.
fn one_clip_project() -> Project {
    let mut project = fixture("p8_trim.json");
    project.playlist.clips.retain(|id, _| *id == ClipId(1));
    let clip = project.playlist.clips.get_mut(&ClipId(1)).expect("clip 1");
    *clip = Clip::of_audio(ChannelId(1), TrackId(1), Ticks(0), Ticks(384));
    project.validate().expect("valid");
    project
}

// A stream that is not ready silences its clip for that block only, counts one underrun, posts
// one event, and the next block plays the frames its position names.
#[test]
fn a_stream_that_is_not_ready_silences_one_block_and_is_counted() {
    let project = one_clip_project();
    let frames = ramp(30_000);
    let mut sounds = FlakySounds {
        frames: frames.clone(),
        fail_on: 2,
        fault: SourceRead::NotReady,
    };
    let plan = Compiler::new()
        .compile_with(&project, &Touched::default(), PlayMode::Song, &mut sounds)
        .expect("compiles");
    let (mut link, mut player) = realtime::link(plan, project.tempo);
    link.send(Control::Play).expect("room");
    let out = render(&mut player, 1536, 512);
    for (frame, value) in out.iter().enumerate() {
        let want = if (512..1024).contains(&frame) {
            0.0
        } else {
            frames[frame]
        };
        assert_eq!(*value, [want; 2], "frame {frame}");
    }
    assert_eq!(link.underruns(), 1);
    assert_eq!(
        link.take_events(),
        [EngineEvent::StreamUnderrun { clip: ClipId(1) }]
    );
}

// A stream that fails for good on its second read (its reader thread stopped) is silent from
// that block on, is named once by a StreamFailed event, and is no underrun.
#[test]
fn a_stream_that_fails_is_silent_from_then_on_and_named_once() {
    let project = one_clip_project();
    let frames = ramp(30_000);
    let mut sounds = FlakySounds {
        frames: frames.clone(),
        fail_on: 2,
        fault: SourceRead::Failed,
    };
    let plan = Compiler::new()
        .compile_with(&project, &Touched::default(), PlayMode::Song, &mut sounds)
        .expect("compiles");
    let (mut link, mut player) = realtime::link(plan, project.tempo);
    link.send(Control::Play).expect("room");
    let out = render(&mut player, 2048, 512);
    for (frame, value) in out.iter().enumerate() {
        let want = if frame < 512 { frames[frame] } else { 0.0 };
        assert_eq!(*value, [want; 2], "frame {frame}");
    }
    assert_eq!(link.underruns(), 0);
    assert_eq!(
        link.take_events(),
        [EngineEvent::StreamFailed { clip: ClipId(1) }]
    );
}

// While an automation governs, the knob reads its value back; Pattern mode plays the stored
// value and Stop gives the knob back to it.
#[test]
fn automated_values_read_back_while_they_govern() {
    let project = fixture("p8_auto_linear.json");
    let (mut link, mut player) = song_player(&project, &[1.0]);
    render(&mut player, 88_200, 512);
    assert_eq!(link.automation_value(0), None, "bar 1 is before the window");
    render(&mut player, 44_100, 512);
    let (id, value) = link.automation_value(0).expect("governs in bar 2");
    assert_eq!(id, AutomationId(1));
    // The last frame rendered is song sample 132299 of the line 1.0 → 0.0 over 88200..176400.
    let x = (132_299.0 - 88_200.0) / 88_200.0;
    assert_eq!(value, (4.0 * (0.25 - 0.25 * x)) as f32);
    link.send(Control::Stop).expect("room");
    render(&mut player, 512, 512);
    assert_eq!(
        link.automation_value(0),
        None,
        "Stop returns to the stored value"
    );

    let plan = compiled(
        &mut Compiler::new(),
        &project,
        PlayMode::Pattern(PatternId(1)),
        &[1.0],
    );
    let (mut link, mut player) = realtime::link(plan, project.tempo);
    link.send(Control::Play).expect("room");
    let out = render(&mut player, 176_400, 512);
    assert_eq!(out[88_200], [0.5; 2], "pattern mode keeps the stored 0.5");
    assert_eq!(link.automation_value(0), None);
}

// While an automation governs a target, a live value for it is ignored, so it cannot stick once
// the automation lets go: p8_auto_linear's channel-1 volume, stored at 0.5, is automated over
// bar 2. A drag to 3.0 sent inside bar 2, then Stop and Play from tick 0: bar 1 is before the
// window, so its first step plays the stored 0.5, not 3.0.
#[test]
fn a_live_value_does_not_stick_while_automation_governs() {
    let project = fixture("p8_auto_linear.json");
    let (mut link, mut player) = song_player(&project, &[1.0]);
    render(&mut player, 100_000, 512);
    assert!(link.automation_value(0).is_some(), "the automation governs");
    let drag = ParamTarget::ChannelVolume(ChannelId(1));
    link.send(Control::Param(drag, 3.0)).expect("room");
    render(&mut player, 512, 512);
    link.send(Control::Stop).expect("room");
    link.send(Control::Play).expect("room");
    let out = render(&mut player, 512, 512);
    assert_eq!(out[0], [0.5; 2], "the stored volume, not the dragged one");
}

// The preview plays its file from frame 0 at unity through the master, also while stopped, and
// StopPreview ends it.
#[test]
fn a_preview_plays_through_the_master_while_stopped() {
    let project = fixture("p8_trim.json");
    let plan = compiled(&mut Compiler::new(), &project, PlayMode::Song, &[0.0]);
    let (mut link, mut player) = realtime::link(plan, project.tempo);
    let tone = ramp(700);
    let source = Box::new(MemorySource::new(SampleBuffer::from(tone.clone())));
    assert!(link.preview(source).is_ok());
    let out = render(&mut player, 1000, 256);
    for (frame, value) in out.iter().enumerate() {
        let want = tone.get(frame).copied().unwrap_or(0.0);
        assert_eq!(*value, [want; 2], "frame {frame}");
    }
    assert!(
        link.take_peak(InsertId::MASTER)[0] > 0.0,
        "the preview meters"
    );
    let source = Box::new(MemorySource::new(SampleBuffer::from(tone.clone())));
    assert!(link.preview(source).is_ok());
    render(&mut player, 100, 100);
    link.send(Control::StopPreview).expect("room");
    let after = render(&mut player, 100, 100);
    assert!(after.iter().all(|frame| *frame == [0.0; 2]), "stopped");
}

// Known answer 8 in the engine: Record with a count-in plays one bar of bursts (0.25, then
// 0.125 on beats 2 to 4) while the playhead waits, then the song, which the record tap receives
// frame for frame from where it began; the loop and the song-end wrap are off.
#[test]
fn a_count_in_precedes_the_recorded_song_and_the_tap_gets_the_song() {
    let rec = &expected()["rec"];
    let project = fixture("p8_rec.json");
    let count_in = rec["count_in"].as_u64().expect("frames") as usize;
    let song_frames = rec["recorded_frames"].as_u64().expect("frames") as usize;
    let (mut link, mut player) = {
        let plan = compiled(&mut Compiler::new(), &project, PlayMode::Song, &[1.0]);
        realtime::link(plan, project.tempo)
    };
    let mut tap = link.take_record_tap().expect("the tap");
    link.send(Control::Record {
        count_in: true,
        tap_master: true,
    })
    .expect("room");
    let out = render(&mut player, count_in + song_frames, 512);
    let mut clicks = vec![0.0f32; count_in];
    for burst in rec["bursts"].as_array().expect("bursts") {
        let at = burst[0].as_u64().expect("a frame") as usize;
        for (k, value) in burst[1].as_array().expect("values").iter().enumerate() {
            clicks[at + k] = value.as_f64().expect("a number") as f32;
        }
    }
    for (frame, &want) in clicks.iter().enumerate() {
        assert_eq!(out[frame], [want; 2], "count-in frame {frame}");
    }
    assert_eq!(link.record_clock().recording_from(), Some(count_in as u64));
    let (_plain, mut plain) = song_player(&project, &[1.0]);
    let song = render(&mut plain, song_frames, 512);
    assert_eq!(
        bits(&out[count_in..]),
        bits(&song),
        "the song follows the count-in"
    );
    let mut recorded = Vec::new();
    tap.read_into(&mut recorded);
    let recorded: Vec<[f32; 2]> = recorded
        .chunks_exact(2)
        .map(|pair| [pair[0], pair[1]])
        .collect();
    assert_eq!(bits(&recorded), bits(&song), "the tap holds the song");
    assert_eq!(tap.dropped(), 0);
    link.send(Control::Stop).expect("room");
    render(&mut player, 1, 1);
    assert_eq!(link.record_clock().recording_from(), None);
}

// The audio thread's rule with every clip and automation path in play: audio clips from memory,
// volume (per sample) and effect (on the grid) automation, a preview, the record tap after a
// count-in, and twenty plan updates that install and remove clip sources.
#[test]
fn process_never_touches_the_heap_with_audio_clips_and_automation_in_play() {
    let mut project = fixture("p8_stream.json");
    project.mixer.inserts[0].effects.push(Effect {
        id: EffectId(1),
        slot: 0,
        processor: ProcessorRef::new("dawsome.delay"),
    });
    project.mixer.next_effect_id = EffectId(2);
    let first = project.playlist.next_clip_id;
    for edit in [
        Edit::AddAutomationClip {
            automation: AutomationId(1),
            clip: first,
            target: AutomationTarget::ChannelVolume(ChannelId(1)),
            track: TrackId(3),
            start: Ticks(0),
            length: Ticks(1536),
            points: vec![
                AutomationPoint::linear(Ticks(0), 0.25),
                AutomationPoint {
                    curve: Curve::Exp(0.5),
                    ..AutomationPoint::linear(Ticks(700), 0.1)
                },
                AutomationPoint::linear(Ticks(1536), 0.2),
            ],
        },
        Edit::AddAutomationClip {
            automation: AutomationId(2),
            clip: ClipId(first.0 + 1),
            target: AutomationTarget::EffectParam {
                effect: EffectId(1),
                key: "mix".into(),
            },
            track: TrackId(4),
            start: Ticks(48),
            length: Ticks(1000),
            points: vec![
                AutomationPoint::linear(Ticks(0), 0.0),
                AutomationPoint::linear(Ticks(900), 1.0),
            ],
        },
    ] {
        edit::apply(&mut project, edit).expect("automates");
    }
    let saw = ramp(30_000);
    let mut compiler = Compiler::new();
    let plan = compiled(&mut compiler, &project, PlayMode::Song, &saw);
    let (mut link, mut player) = realtime::link(plan, project.tempo);
    let _tap = link.take_record_tap().expect("the tap");
    link.send(Control::Record {
        count_in: true,
        tap_master: true,
    })
    .expect("room");
    let mut out = vec![[0.0; 2]; 512];
    let (mut calls, mut pushed) = (0, 0);
    for call in 0..1200 {
        if call % 50 == 25 && pushed < 20 {
            // In turn: a new clip of the file (a new source) and the newest clip deleted.
            let edit = if pushed % 2 == 0 {
                Edit::PlaceClips {
                    first: project.playlist.next_clip_id,
                    clips: vec![Clip::of_audio(
                        ChannelId(1),
                        TrackId(5),
                        Ticks(96),
                        Ticks(192),
                    )],
                }
            } else {
                Edit::UnplaceClips {
                    first: ClipId(project.playlist.next_clip_id.0 - 1),
                    count: 1,
                }
            };
            let applied = edit::apply(&mut project, edit).expect("applies");
            let same = |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(saw.clone()));
            let update = compiler
                .compile(&project, &applied.touched, PlayMode::Song, same)
                .expect("compiles");
            link.push_plan(update).expect("same time base");
            pushed += 1;
        }
        if call == 300 {
            let preview = Box::new(MemorySource::new(SampleBuffer::from(saw.clone())));
            assert!(link.preview(preview).is_ok());
        }
        calls += heap_calls_during(|| player.process(&mut out));
        link.drain_retired();
        compiler.release(link.take_freed_slots());
        compiler.release_sources(link.take_freed_sources());
    }
    assert_eq!(calls, 0, "Player::process made {calls} heap calls");
    assert_eq!(pushed, 20);
}
