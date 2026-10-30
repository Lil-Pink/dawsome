//! The plugin side of the processor port, through the real link: plugin channels get their
//! notes on exact frames with lengths that end them on time, a muted one gets none, plugin
//! parameters travel over the control ring, stored states reach the live instance, missing and
//! failing plugins are named, and none of it touches the heap on the audio thread.
//!
//! The host here is a stand-in whose processor writes `+a·velocity` on a note's first frame and
//! `−a·velocity` on the frame its note ends, so a render shows exactly where the engine put each
//! note-on and note-off. Expected frames: 48 kHz at 120 BPM and ppq 96 put tick t on frame 250·t.

use std::cell::{Cell, RefCell};
use std::convert::Infallible;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, Edit, History, Touched};
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::processor::{
    Captured, Editor, HostError, HostNotice, PluginParam, PortEvent, ProcessContext, Processor,
    ProcessorHost, Registry,
};
use dawsome_engine::project::{
    Channel, ChannelId, Clip, ClipId, Note, PatternId, ProcessorOwner, ProcessorRef, Project,
    TrackId,
};
use dawsome_engine::realtime::{self, Player, RealtimeLink};
use dawsome_engine::time::{MilliBpm, Ticks};
use dawsome_engine::transport::{Control, ParamTarget};

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

/// What the stand-in processors report back to the test.
#[derive(Default)]
struct Tally {
    notes: AtomicUsize,
    processed: AtomicUsize,
    idled: AtomicUsize,
    suspended: AtomicUsize,
}

/// Clicks on note starts and ends; parameter 0 sets the click size from the next block.
struct Clicks {
    size: f32,
    queued: Option<f32>,
    /// Pending note ends: output-clock frame and click size. Sized on the UI thread.
    ends: Vec<(u64, f32)>,
    tally: Arc<Tally>,
}

impl Processor for Clicks {
    fn process(&mut self, context: &ProcessContext<'_>, buffer: &mut [[f32; 2]]) {
        self.tally.processed.fetch_add(1, Ordering::Relaxed);
        if let Some(size) = self.queued.take() {
            self.size = size;
        }
        let start = context.transport.steady_time;
        for event in context.events {
            let PortEvent::Note {
                frame,
                velocity,
                length,
                ..
            } = *event
            else {
                continue;
            };
            let click = self.size * velocity;
            let at = frame as usize;
            buffer[at] = [buffer[at][0] + click, buffer[at][1] + click];
            if self.ends.len() < self.ends.capacity() {
                self.ends.push((start + u64::from(frame) + length, click));
            }
            self.tally.notes.fetch_add(1, Ordering::Relaxed);
        }
        let end = start + buffer.len() as u64;
        self.ends.retain(|&(at, click)| {
            if at < end {
                let frame = (at - start) as usize;
                buffer[frame] = [buffer[frame][0] - click, buffer[frame][1] - click];
                false
            } else {
                true
            }
        });
    }

    fn set_param(&mut self, _index: usize, _value: f32) {}

    fn reset(&mut self) {
        self.ends.clear();
    }

    fn queue_param(&mut self, id: u32, value: f32) {
        if id == 0 {
            self.queued = Some(value);
        }
    }

    fn idle(&mut self, _context: &ProcessContext<'_>) {
        if let Some(size) = self.queued.take() {
            self.size = size;
            self.tally.idled.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn suspend(&mut self) {
        self.tally.suspended.fetch_add(1, Ordering::Relaxed);
    }
}

/// The states handed to live instances, in order.
type Synced = Rc<RefCell<Vec<(ProcessorOwner, Option<String>)>>>;

/// A host of the `mock` format: `mock:clicks` makes [`Clicks`], `mock:broken` fails.
#[derive(Default)]
struct MockHost {
    created: Rc<Cell<usize>>,
    synced: Synced,
    tally: Arc<Tally>,
}

impl ProcessorHost for MockHost {
    fn format(&self) -> &'static str {
        "mock"
    }

    fn create(
        &self,
        _owner: ProcessorOwner,
        reference: &ProcessorRef,
        _sample_rate: u32,
    ) -> Result<Box<dyn Processor>, HostError> {
        match reference.id.as_str() {
            "mock:clicks" => {
                self.created.set(self.created.get() + 1);
                Ok(Box::new(Clicks {
                    size: 1.0,
                    queued: None,
                    ends: Vec::with_capacity(2048),
                    tally: Arc::clone(&self.tally),
                }))
            }
            "mock:broken" => Err(HostError::Failed("refused to activate".into())),
            _ => Err(HostError::Missing),
        }
    }

    fn release(&self, _owner: ProcessorOwner) {}

    fn sync_state(&self, owner: ProcessorOwner, reference: &ProcessorRef) {
        self.synced
            .borrow_mut()
            .push((owner, reference.state.clone()));
    }

    fn params(&self, _owner: ProcessorOwner) -> Vec<PluginParam> {
        Vec::new()
    }

    fn value(&self, _owner: ProcessorOwner, _id: u32) -> Option<f64> {
        None
    }

    fn value_text(&self, _owner: ProcessorOwner, _id: u32, _value: f64) -> Option<String> {
        None
    }

    fn capture_state(&self, _owner: ProcessorOwner) -> Result<Captured, HostError> {
        Err(HostError::Missing)
    }

    fn open_editor(&self, _owner: ProcessorOwner, _title: &str) -> Result<Editor, HostError> {
        Ok(Editor::Generic)
    }

    fn tick(&self) -> Vec<HostNotice> {
        Vec::new()
    }

    fn has_instances(&self) -> bool {
        self.created.get() > 0
    }
}

/// The host's tallies, kept by the test while the compiler owns the host.
struct Probe {
    created: Rc<Cell<usize>>,
    synced: Synced,
    tally: Arc<Tally>,
}

fn mock_compiler() -> (Compiler, Probe) {
    let host = MockHost::default();
    let probe = Probe {
        created: Rc::clone(&host.created),
        synced: Rc::clone(&host.synced),
        tally: Arc::clone(&host.tally),
    };
    let mut registry = Registry::native();
    registry.add_host(Box::new(host));
    (Compiler::with_registry(registry), probe)
}

const CLICKS: ChannelId = ChannelId(1);

fn note(start: i64, length: i64, key: u8) -> Note {
    Note {
        channel: CLICKS,
        start: Ticks(start),
        length: Ticks(length),
        velocity: 1.0,
        key,
    }
}

/// 48 kHz, 120 BPM: channel 1 plays `mock:clicks` with `notes` in a one-bar pattern.
fn clicks_project(notes: Vec<Note>) -> Project {
    let mut project = Project::new(48_000, MilliBpm(120_000), 96);
    project.channels.insert(
        CLICKS,
        Channel::plugin("Clicks", ProcessorRef::plugin("mock:clicks", "Clicks")),
    );
    project.next_channel_id = ChannelId(2);
    project.patterns.insert(
        PatternId(1),
        dawsome_engine::project::Pattern {
            name: "Pattern 1".into(),
            length_bars: 1,
            copied_from: None,
            notes,
        },
    );
    project.validate().expect("valid");
    project
}

/// A note at tick 24 lasting 96 ticks, and a step hit at tick 192.
fn two_notes() -> Project {
    clicks_project(vec![note(24, 96, 69), note(192, 0, 60)])
}

fn compile(compiler: &mut Compiler, project: &Project, mode: PlayMode) -> PlanUpdate {
    let silent = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(vec![0.0]));
    compiler
        .compile(project, &Touched::default(), mode, silent)
        .expect("compiles")
}

fn playing(project: &Project, mode: PlayMode) -> (RealtimeLink, Player, Compiler, Probe) {
    let (mut compiler, probe) = mock_compiler();
    let update = compile(&mut compiler, project, mode);
    let (mut link, player) = realtime::link(update, project.tempo);
    link.send(Control::Play).expect("room");
    (link, player, compiler, probe)
}

fn render(player: &mut Player, frames: usize, blocks: impl IntoIterator<Item = usize>) -> Vec<f32> {
    let mut out = vec![[f32::NAN; 2]; frames];
    let mut done = 0;
    for size in blocks.into_iter().chain(std::iter::repeat(frames)) {
        if done == frames {
            break;
        }
        let end = (done + size).min(frames);
        player.process(&mut out[done..end]);
        done = end;
    }
    assert!(
        out.iter().all(|frame| frame[0] == frame[1]),
        "both sides alike"
    );
    out.iter().map(|frame| frame[0]).collect()
}

fn clicks(out: &[f32]) -> Vec<(usize, f32)> {
    out.iter()
        .enumerate()
        .filter(|(_, &value)| value != 0.0)
        .map(|(frame, &value)| (frame, value))
        .collect()
}

// Tick 24 → frame 6000, its end at tick 120 → 30000; the step hit at tick 192 → 48000 lasts one
// step (ppq/4 = 24 ticks), so it ends on tick 216 → 54000.
#[test]
fn notes_start_on_their_frame_and_end_after_their_length_or_one_step() {
    let (_link, mut player, _compiler, probe) =
        playing(&two_notes(), PlayMode::Pattern(PatternId(1)));
    let out = render(&mut player, 96_000, [512; 1]);
    assert_eq!(
        clicks(&out),
        [(6000, 1.0), (30_000, -1.0), (48_000, 1.0), (54_000, -1.0)]
    );
    assert_eq!(probe.created.get(), 1);
    assert_eq!(probe.tally.notes.load(Ordering::Relaxed), 2);
}

#[test]
fn any_split_of_the_render_places_the_notes_the_same() {
    let whole = {
        let (_link, mut player, ..) = playing(&two_notes(), PlayMode::Pattern(PatternId(1)));
        render(&mut player, 192_000, [192_000])
    };
    let mut seed = 0x2545_f491_u32;
    let mut random = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        1 + (seed % 3000) as usize
    };
    let random: Vec<usize> = (0..200).map(|_| random()).collect();
    let splits: [Vec<usize>; 5] = [vec![1; 200_000], vec![7], vec![64], vec![4096], random];
    for blocks in splits {
        let (_link, mut player, ..) = playing(&two_notes(), PlayMode::Pattern(PatternId(1)));
        let first = blocks[0];
        let cycle = blocks.iter().copied().cycle().take(200_000);
        let split = render(&mut player, 192_000, cycle);
        assert_eq!(
            split.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            whole.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            "blocks starting with {first}"
        );
    }
}

// Song mode: the pattern plays from a clip of 60 ticks, which ends the tick-24 note early, at
// tick 60 → frame 15000; the step hit at 192 lies past the clip and never starts.
#[test]
fn a_clip_end_ends_its_notes() {
    let mut project = two_notes();
    project.playlist.clips.insert(
        ClipId(1),
        Clip::of_pattern(PatternId(1), TrackId(1), Ticks(0), Ticks(60)),
    );
    project.playlist.next_clip_id = ClipId(2);
    project.validate().expect("valid");
    let (_link, mut player, ..) = playing(&project, PlayMode::Song);
    let out = render(&mut player, 14_000, [512]);
    assert_eq!(clicks(&out), [(6000, 1.0)]);
    let (_link, mut player, ..) = playing(&project, PlayMode::Song);
    // The song ends with the clip, so playback wraps on frame 15000 too; the clock keeps time,
    // and the note plays again from 15000 + 6000.
    let out = render(&mut player, 22_000, [512]);
    assert_eq!(clicks(&out), [(6000, 1.0), (15_000, -1.0), (21_000, 1.0)]);
}

// A muted plugin channel is sent no notes and outputs silence, but keeps processing.
#[test]
fn a_muted_plugin_channel_gets_no_notes_but_keeps_processing() {
    let mut project = two_notes();
    project.channels.get_mut(&CLICKS).expect("clicks").mute = true;
    let (_link, mut player, _compiler, probe) = playing(&project, PlayMode::Pattern(PatternId(1)));
    let out = render(&mut player, 96_000, [512]);
    assert_eq!(clicks(&out), []);
    assert_eq!(probe.tally.notes.load(Ordering::Relaxed), 0);
    assert!(probe.tally.processed.load(Ordering::Relaxed) >= 96_000 / 512);
}

// A note that is sounding when the channel is muted (frames 10240..40960) still ends while
// muted, unheard, because the processor keeps running; once unmuted, bar 2's note sounds from
// 96000 + 6000 to 96000 + 30000. A processor left out while muted would still hold the stale end.
#[test]
fn muting_mid_note_does_not_lose_its_end() {
    let mut project = clicks_project(vec![note(24, 96, 69)]);
    let (mut compiler, _probe) = mock_compiler();
    let update = compile(&mut compiler, &project, PlayMode::Pattern(PatternId(1)));
    let (mut link, mut player) = realtime::link(update, project.tempo);
    link.send(Control::Play).expect("room");
    let mut out = render(&mut player, 10_240, [512]);
    let applied = edit::apply(
        &mut project,
        Edit::SetChannelMute {
            channel: CLICKS,
            value: true,
        },
    )
    .expect("mutes");
    let silent = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(vec![0.0]));
    let update = compiler
        .compile(
            &project,
            &applied.touched,
            PlayMode::Pattern(PatternId(1)),
            silent,
        )
        .expect("compiles");
    link.check_room().expect("room");
    link.push_plan(update).expect("same time base");
    out.extend(render(&mut player, 30_720, [512]));
    let unmuted = edit::apply(&mut project, applied.inverse).expect("unmutes");
    let update = compiler
        .compile(
            &project,
            &unmuted.touched,
            PlayMode::Pattern(PatternId(1)),
            silent,
        )
        .expect("compiles");
    link.check_room().expect("room");
    link.push_plan(update).expect("same time base");
    out.extend(render(&mut player, 151_040, [512]));
    assert_eq!(out.len(), 192_000);
    assert_eq!(clicks(&out), [(6000, 1.0), (102_000, 1.0), (126_000, -1.0)]);
}

// 8 kHz at 522 BPM: a tick is 9.58 frames, so the first 512-frame block holds ticks 0 to 53.
// 1100 notes on ticks 0..10 (110 keys each) start in it: 1024 reach the plugin, 76 are dropped
// and counted.
#[test]
fn notes_past_a_plugins_block_share_are_dropped_and_counted() {
    let notes = (0..10)
        .flat_map(|tick| (0..110).map(move |key| note(tick, 1, key)))
        .collect();
    let mut project = clicks_project(notes);
    project.sample_rate = 8_000;
    project.tempo = MilliBpm(522_000);
    let (link, mut player, _compiler, probe) = playing(&project, PlayMode::Pattern(PatternId(1)));
    render(&mut player, 512, [512]);
    assert_eq!(probe.tally.notes.load(Ordering::Relaxed), 1024);
    assert_eq!(link.dropped_notes(), 76);
}

// A plugin value travels as f32 over the control ring and is heard from the start of the next
// block; sent while stopped, it reaches the plugin through `idle`.
#[test]
fn plugin_parameters_travel_over_the_control_ring() {
    let target = ParamTarget::Plugin {
        owner: ProcessorOwner::Channel(CLICKS),
        id: 0,
    };
    let (mut compiler, probe) = mock_compiler();
    let project = two_notes();
    let update = compile(&mut compiler, &project, PlayMode::Pattern(PatternId(1)));
    let (mut link, mut player) = realtime::link(update, project.tempo);
    link.send(Control::Param(target, 0.5)).expect("room");
    player.process(&mut [[0.0; 2]; 64]);
    assert_eq!(
        probe.tally.idled.load(Ordering::Relaxed),
        1,
        "applied while stopped"
    );
    link.send(Control::Play).expect("room");
    let mut out = render(&mut player, 12_288, [512]);
    link.send(Control::Param(target, 0.25)).expect("room");
    out.extend(render(&mut player, 83_712, [512]));
    assert_eq!(
        clicks(&out),
        [(6000, 0.5), (30_000, -0.5), (48_000, 0.25), (54_000, -0.25)]
    );
}

// Storing a captured state is an edit; the compile after it, and after its undo, hands the
// document's state to the live instance, which is never created again.
#[test]
fn a_stored_state_reaches_the_live_instance_and_undo_takes_it_back() {
    let (mut compiler, probe) = mock_compiler();
    let mut project = two_notes();
    let silent = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(vec![0.0]));
    let mode = PlayMode::Pattern(PatternId(1));
    compiler
        .compile(&project, &Touched::default(), mode, silent)
        .expect("compiles");
    let mut history = History::new();
    let owner = ProcessorOwner::Channel(CLICKS);
    let touched = history
        .apply(
            &mut project,
            Edit::SetProcessorState {
                owner,
                state: Some("AAEC".into()),
            },
        )
        .expect("stores");
    assert_eq!(touched.processors, [owner].into());
    compiler
        .compile(&project, &touched, mode, silent)
        .expect("compiles");
    let undone = history.undo(&mut project).expect("undoes").expect("a step");
    compiler
        .compile(&project, &undone, mode, silent)
        .expect("compiles");
    assert_eq!(
        *probe.synced.borrow(),
        [(owner, Some("AAEC".to_owned())), (owner, None)]
    );
    assert_eq!(probe.created.get(), 1, "no new instance");
    assert_eq!(project, two_notes());
}

#[test]
fn missing_and_failing_plugins_are_named_and_replaced_by_silence() {
    let mut project = two_notes();
    project.channels.insert(
        ChannelId(2),
        Channel::plugin(
            "Gone",
            ProcessorRef::plugin("clap:org.example.gone", "Gone Synth"),
        ),
    );
    project.channels.insert(
        ChannelId(3),
        Channel::plugin(
            "Broken",
            ProcessorRef::plugin("mock:broken", "Broken Synth"),
        ),
    );
    project.next_channel_id = ChannelId(4);
    project.validate().expect("valid");
    let (mut compiler, _probe) = mock_compiler();
    compile(&mut compiler, &project, PlayMode::Pattern(PatternId(1)));
    assert_eq!(
        compiler.take_notices(),
        [
            "Missing plugin: Gone Synth (clap:org.example.gone)",
            "Plugin failed: Broken Synth: refused to activate"
        ]
    );
    assert!(compiler.take_notices().is_empty(), "taken once");
}

// The plugin channel is added at the volume a channel that plays notes is added at, 9/16.
#[test]
fn adding_a_plugin_channel_undoes_to_the_same_bytes() {
    let mut project = two_notes();
    let before = project.to_json().expect("serializes");
    let add = Edit::AddPluginChannel {
        id: ChannelId(2),
        name: "Lead".into(),
        processor: ProcessorRef::plugin("mock:clicks", "Clicks"),
    };
    let applied = edit::apply(&mut project, add).expect("adds");
    assert_eq!(project.next_channel_id, ChannelId(3));
    assert_eq!(applied.touched.channels, [ChannelId(2)].into());
    let saved = project.to_json().expect("serializes");
    assert!(
        saved.contains(r#"{"id":2,"name":"Lead","kind":"plugin","processor":{"id":"mock:clicks","name":"Clicks"},"volume":0.5625,"pan":0.0,"mute":false}"#),
        "{saved}"
    );
    edit::apply(&mut project, applied.inverse).expect("removes");
    assert_eq!(project.next_channel_id, ChannelId(2));
    assert_eq!(project.to_json().expect("serializes"), before);
}

#[test]
fn a_sampler_has_no_plugin_state_to_set() {
    let mut project = two_notes();
    project
        .channels
        .insert(ChannelId(2), Channel::sampler("kick", "kick.wav"));
    project.next_channel_id = ChannelId(3);
    let set = |owner| Edit::SetProcessorState {
        owner,
        state: Some("AA==".into()),
    };
    let before = project.clone();
    assert!(edit::apply(&mut project, set(ProcessorOwner::Channel(ChannelId(2)))).is_err());
    assert!(edit::apply(&mut project, set(ProcessorOwner::Channel(ChannelId(9)))).is_err());
    assert_eq!(project, before);
}

// The audio thread's rule with a plugin channel: notes, a plugin value every 10 calls, a Stop
// and Play, and plan updates that add and take away a second plugin channel. The stand-in
// processor allocates nothing itself, so every heap call counted would be the engine's.
#[test]
fn the_plugin_path_never_touches_the_heap() {
    let mut project = two_notes();
    let (mut compiler, _probe) = mock_compiler();
    let mode = PlayMode::Pattern(PatternId(1));
    let update = compile(&mut compiler, &project, mode);
    let (mut link, mut player) = realtime::link(update, project.tempo);
    link.send(Control::Play).expect("room");
    let mut buffer = [[0.0f32; 2]; 512];
    let target = ParamTarget::Plugin {
        owner: ProcessorOwner::Channel(CLICKS),
        id: 0,
    };
    let silent = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(vec![0.0]));
    let add = || Edit::AddPluginChannel {
        id: ChannelId(2),
        name: "Second".into(),
        processor: ProcessorRef::plugin("mock:clicks", "Clicks"),
    };
    let mut undo = None;
    for call in 0..1000 {
        if call % 10 == 0 {
            link.send(Control::Param(target, (call % 7) as f32 / 7.0))
                .expect("room");
        }
        if call == 500 {
            link.send(Control::Stop).expect("room");
            link.send(Control::Play).expect("room");
        }
        let edit = match call % 100 {
            25 => Some(add()),
            75 => undo.take(),
            _ => None,
        };
        if let Some(edit) = edit {
            let applied = edit::apply(&mut project, edit).expect("applies");
            if call % 100 == 25 {
                undo = Some(applied.inverse);
            }
            link.check_room().expect("room");
            compiler.release(link.take_freed_slots());
            let update = compiler
                .compile(&project, &applied.touched, mode, silent)
                .expect("compiles");
            link.push_plan(update).expect("same time base");
        }
        assert_eq!(
            heap_calls_during(|| player.process(&mut buffer)),
            0,
            "call {call}"
        );
        link.drain_retired();
    }
}

// The app's close: Suspend stops playback and suspends every processor in the player's next
// call, and the link counts it, so the app knows when the processors' last audio-thread call
// has been made.
#[test]
fn suspend_stops_and_suspends_every_processor_once_counted() {
    let (mut link, mut player, _compiler, probe) =
        playing(&two_notes(), PlayMode::Pattern(PatternId(1)));
    render(&mut player, 1024, [512]);
    let before = link.suspends();
    link.send(Control::Suspend).expect("room");
    let out = render(&mut player, 512, [512]);
    assert!(out.iter().all(|&value| value == 0.0), "stopped");
    assert!(!link.playing());
    assert_eq!(link.suspends(), before + 1);
    assert_eq!(probe.tally.suspended.load(Ordering::Relaxed), 1);
    link.send(Control::Play).expect("room");
    let out = render(&mut player, 7000, [512]);
    assert_eq!(clicks(&out), [(6000, 1.0)], "plays again after a suspend");
}
