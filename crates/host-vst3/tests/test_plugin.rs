//! The VST3 host with Dawsome's own VST3 test plugins (crates/test-plugin-vst3), through the real
//! link: the compiler creates them, the player runs them, the control ring carries their values,
//! captured states round-trip byte for byte as `.vstpreset` files, undo loads a state into the
//! live instance, the component and its controller talk through the host's connection, the
//! Linux run loop serves a timer, and the host's audio half never touches the heap. Expected
//! values are read from tests/fixtures/p27_expected.json (gen_p27_expected.py).

use std::convert::Infallible;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, Edit, History, Touched};
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::processor::{Editor, HostError, NoticeKind, ProcessorHost, Registry};
use dawsome_engine::project::{
    ChannelId, Effect, EffectId, InsertId, Note, PatternId, ProcessorOwner, ProcessorRef, Project,
};
use dawsome_engine::realtime::{self, Player, RealtimeLink};
use dawsome_engine::time::{MilliBpm, Ticks};
use dawsome_engine::transport::{Control, ParamTarget};
use dawsome_host_common::catalogue::{self, Catalogue, FileRecord, FileStatus, PluginFormat};
use dawsome_host_vst3::{scan, Vst3Host};
use serde_json::Value;

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;
#[path = "../../../tests/support/test_plugins.rs"]
mod test_plugins;

use counting_allocator::heap_calls_during;

const P27_SINE: &str = include_str!("../../../tests/fixtures/p27_vst3_sine.json");
const SINE: ProcessorOwner = ProcessorOwner::Channel(ChannelId(1));
const MODE: PlayMode = PlayMode::Pattern(PatternId(1));
const MODULE: &str = "Dawsome Test VST3.vst3";

fn expected() -> Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/p27_expected.json"))
        .expect("p27_expected.json parses")
}

/// The text of one of the generator's answers.
fn text(value: &Value) -> String {
    value.as_str().expect("a text").to_owned()
}

/// A catalogue of the test module `module`, described in this process (the scan child's work).
fn catalogue_of(module: std::path::PathBuf) -> Catalogue {
    let plugins = scan::describe_module(&module, || {}).expect("the test module loads");
    let (size, mtime) =
        catalogue::module_stamp(PluginFormat::Vst3, &module).expect("the module is there");
    Catalogue {
        files: vec![FileRecord {
            file: module,
            format: PluginFormat::Vst3,
            size,
            mtime,
            status: FileStatus::Ok,
            reason: None,
            plugins,
        }],
        ..Catalogue::default()
    }
}

fn catalogue() -> Catalogue {
    catalogue_of(test_plugins::test_plugins_vst3().join(MODULE))
}

/// A compiler that creates plugins through `host`.
fn compiler(host: &Vst3Host) -> Compiler {
    let mut registry = Registry::native();
    registry.add_host(Box::new(host.clone()));
    Compiler::with_registry(registry)
}

fn compile(compiler: &mut Compiler, project: &Project, touched: &Touched) -> PlanUpdate {
    let silent = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(vec![1.0]));
    compiler
        .compile(project, touched, MODE, silent)
        .expect("compiles")
}

/// The sine fixture with its processor replaced by `id` (with no state) or kept as it is.
fn project_with(id: Option<&str>) -> Project {
    let mut project = Project::load_json(P27_SINE).expect("fixture loads");
    if let Some(id) = id {
        let channel = project.channels.get_mut(&ChannelId(1)).expect("channel 1");
        let processor = channel.processor.as_mut().expect("a plugin channel");
        processor.id = format!("vst3:{id}");
        processor.state = None;
    }
    project
}

/// `project` playing through a fresh host: the host, the compiler, both halves.
fn playing(project: &Project, live: bool) -> (Vst3Host, Compiler, RealtimeLink, Player) {
    let host = Vst3Host::new(catalogue());
    let mut compiler = compiler(&host);
    if live {
        compiler.set_live_channel(Some(ChannelId(1)));
    }
    let update = compile(&mut compiler, project, &Touched::default());
    assert!(compiler.take_notices().is_empty(), "the plugins are found");
    let (link, player) = realtime::link(update, project.tempo);
    (host, compiler, link, player)
}

/// The sine fixture playing through a fresh host.
fn sine() -> (Vst3Host, Compiler, RealtimeLink, Player) {
    playing(&project_with(None), false)
}

/// The sine fixture with `notes` in its pattern in place of its own, its channel playing live
/// notes.
fn live_sine(notes: Vec<Note>) -> (Vst3Host, Compiler, RealtimeLink, Player) {
    let mut project = project_with(None);
    project
        .patterns
        .get_mut(&PatternId(1))
        .expect("pattern 1")
        .notes = notes;
    playing(&project, true)
}

/// `calls` calls of `block` frames, left side only (the sine writes both sides alike).
fn play(player: &mut Player, calls: usize, block: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(calls * block);
    let mut buffer = vec![[0.0f32; 2]; block];
    for _ in 0..calls {
        player.process(&mut buffer);
        assert!(
            buffer.iter().all(|frame| frame[0] == frame[1]),
            "both sides alike"
        );
        out.extend(buffer.iter().map(|frame| frame[0]));
    }
    out
}

fn near(got: f32, want: f64) -> bool {
    (f64::from(got) - want).abs() < 1e-6
}

fn energy(samples: &[f32]) -> f64 {
    samples.iter().map(|&value| f64::from(value).powi(2)).sum()
}

// 1 (the child's half). The test module lists its audio classes, by id, with the names,
// kinds and parameter counts the generator gives; its controller classes are no plugins.
#[test]
fn the_test_module_lists_its_audio_classes() {
    let mut described = 0;
    let plugins = scan::describe_module(&test_plugins::test_plugins_vst3().join(MODULE), || {
        described += 1;
    })
    .expect("loads");
    let listed = serde_json::to_value(&plugins).expect("records");
    let want = &expected()["scan"]["plugins"];
    assert_eq!(listed, *want);
    assert_eq!(
        described,
        want.as_array().expect("plugins").len(),
        "a progress call for each audio class, the controller class none"
    );
}

// 5. 512-frame calls; gain 0.25 sent before call 19 is heard from frame 19·512 = 9728: 9727 at
// gain 0.5, 9728 and 9729 at 0.25. After a tick the controller has it too (the host hands the
// values it sent on to `setParamNormalized`), and capturing stores the 0.25 preset; one undo
// loads the 0.5 preset back into the same instance.
#[test]
fn a_value_travels_over_the_control_ring_and_undo_reloads_the_state() {
    let expected = expected();
    let answer = &expected["param"];
    let (host, mut compiler, mut link, mut player) = sine();
    let mut project = project_with(None);
    link.send(Control::Play).expect("room");
    let call = answer["call"].as_u64().expect("call") as usize;
    let mut out = play(&mut player, call, 512);
    let target = ParamTarget::Plugin { owner: SINE, id: 0 };
    link.send(Control::Param(target, 0.25)).expect("room");
    out.extend(play(&mut player, 2, 512));
    for frame in answer["frames"].as_array().expect("frames") {
        let at = frame[0].as_u64().expect("frame") as usize;
        let want = frame[1].as_f64().expect("value");
        assert!(near(out[at], want), "{at}: {} instead of {want}", out[at]);
    }
    assert_eq!(host.value(SINE, 0), Some(0.5), "not before a tick");
    assert!(host.tick().is_empty(), "values the host sent mark nothing");
    assert_eq!(host.value(SINE, 0), Some(0.25));

    let captured = host.capture_state(SINE).expect("captures");
    assert_eq!(captured.state, text(&answer["captured"]));
    assert_eq!(captured.bytes, 116);
    let mut history = History::new();
    let touched = history
        .apply(
            &mut project,
            Edit::SetProcessorState {
                owner: SINE,
                state: Some(captured.state),
            },
        )
        .expect("stores");
    link.check_room().expect("room");
    link.push_plan(compile(&mut compiler, &project, &touched))
        .expect("same time base");
    assert_eq!(
        host.value(SINE, 0),
        Some(0.25),
        "the capture reloads nothing"
    );
    let undone = history.undo(&mut project).expect("undoes").expect("a step");
    link.check_room().expect("room");
    link.push_plan(compile(&mut compiler, &project, &undone))
        .expect("same time base");
    assert_eq!(host.value(SINE, 0), Some(0.5), "undo loaded the old state");
    assert_eq!(
        host.capture_state(SINE).expect("captures").state,
        text(&expected["presets"]["sine_0.5"])
    );
    assert_eq!(host.created(), 1, "no new instance");
    assert_eq!(project.to_json().expect("serializes"), P27_SINE);
}

// 4. The fixture's preset loads and captures back to the same 116 bytes; a preset holding the
// component's chunk alone loads, and captures back whole. A CLAP state is refused as no VST3
// preset, a preset of another class as another plugin's. An instance made with no state keeps
// its first one (gain 1.0): clearing the document's state (undoing its first capture) loads it.
#[test]
fn a_state_round_trips_and_a_wrong_one_is_refused_by_name() {
    let expected = expected();
    let presets = &expected["presets"];
    let (host, ..) = sine();
    let captured = host.capture_state(SINE).expect("captures");
    assert_eq!(captured.state, text(&presets["sine_0.5"]));
    let bytes = BASE64.decode(&captured.state).expect("base64");
    assert_eq!(bytes.len(), 116);

    let host = Vst3Host::new(catalogue());
    let owner = ProcessorOwner::Channel(ChannelId(9));
    let sine_id = format!("vst3:{}", text(&expected["ids"]["sine"]));
    let mut reference = ProcessorRef::plugin(&sine_id, "Dawsome Test Sine");
    for (preset, reason) in expected["refused"].as_object().expect("refusals") {
        reference.state = Some(text(&presets[preset]));
        let error = host.create(owner, &reference, 48_000).err();
        assert_eq!(error, Some(HostError::Failed(text(reason))), "{preset}");
    }
    reference.state = Some(text(&presets["sine_0.5_component_only"]));
    let _alone = host.create(owner, &reference, 48_000).expect("loads");
    assert_eq!(
        host.capture_state(owner).expect("captures").state,
        text(&presets["sine_0.5"])
    );

    let fresh = ProcessorOwner::Channel(ChannelId(10));
    let _audio = host
        .create(fresh, &ProcessorRef::plugin(&sine_id, "Sine"), 48_000)
        .expect("creates");
    assert_eq!(
        host.capture_state(fresh).expect("captures").state,
        text(&presets["sine_1.0"]),
        "made with no state: gain 1.0"
    );
    let mut quarter = ProcessorRef::plugin(&sine_id, "Sine");
    quarter.state = Some(text(&presets["sine_0.25"]));
    host.sync_state(fresh, &quarter);
    assert_eq!(host.value(fresh, 0), Some(0.25));
    host.sync_state(fresh, &ProcessorRef::plugin(&sine_id, "Sine"));
    assert_eq!(host.value(fresh, 0), Some(1.0), "the first state again");
    assert_eq!(
        host.capture_state(fresh).expect("captures").state,
        text(&presets["sine_1.0"])
    );
    let mut broken = ProcessorRef::plugin(&sine_id, "Sine");
    broken.state = Some(text(&presets["clap_0.5"]));
    host.sync_state(fresh, &broken);
    let notices = host.tick();
    assert!(
        notices.iter().any(|notice| notice.owner == fresh
            && notice.kind == NoticeKind::Failed("it refused the state it was given".into())),
        "{notices:?}"
    );
}

// A value sent while stopped is flushed into the plugin from the audio thread, through the
// stopped player's idle call (a `process` with no buses and no frames), then heard.
#[test]
fn a_value_sent_while_stopped_is_flushed() {
    let (host, _compiler, mut link, mut player) = sine();
    let target = ParamTarget::Plugin { owner: SINE, id: 0 };
    link.send(Control::Param(target, 0.75)).expect("room");
    play(&mut player, 1, 64);
    host.tick();
    assert_eq!(host.value(SINE, 0), Some(0.75));
    let captured = host.capture_state(SINE).expect("captures");
    let bytes = BASE64.decode(captured.state).expect("base64");
    assert_eq!(
        &bytes[48..60],
        [b"OTP1".as_slice(), &0.75f64.to_le_bytes()].concat()
    );
}

// 10. The sine's controller and component are joined through the host: its "hello" reaches the
// component and the component's "ack" reaches back (each through a proxy of ours, as messages
// the host made), so "Linked" reads 1.0; the controller's call back into the host from inside
// the host's own `setComponentState` marked nothing the next tick reports.
#[test]
fn the_component_and_its_controller_talk_through_the_host() {
    let expected = expected();
    let (host, ..) = sine();
    assert!(host.tick().is_empty(), "the host's own load is forgotten");
    let id = expected["linked"]["param"].as_u64().expect("id") as u32;
    assert_eq!(host.value(SINE, id), expected["linked"]["value"].as_f64());
    // The grown oracle's parameters (Bend, Wheel and Wheel heard besides Gain and
    // Linked, gen_p28_expected.py), in its order.
    let names: Vec<String> = host.params(SINE).into_iter().map(|p| p.name).collect();
    assert_eq!(names, ["Gain", "Bend", "Wheel", "Wheel heard", "Linked"]);
    let linked = host.params(SINE).into_iter().find(|p| p.id == id);
    assert!(linked.expect("Linked").read_only);
    assert_eq!(host.value_text(SINE, 0, 0.5).as_deref(), Some("0.50"));
    assert_eq!(host.open_editor(SINE, "Sine"), Ok(Editor::Generic));
}

// 11. The Tick class registers a 100 ms timer with its host context's run loop as it starts:
// ticked every 16 ms for 400 ms the timer has run (its "Ticks" ≥ 0.003) and the host asks for
// ticks while it lives; its controller's edit on the first timer call (0.125, `performEdit`
// between a begin and an end) reaches the processor, which plays the next note at that gain,
// and the end of the gesture marks the state changed. Released, it goes, and the host is quiet.
#[cfg(target_os = "linux")]
#[test]
fn the_run_loop_serves_a_timer_and_a_controllers_edit_is_heard() {
    let expected = expected();
    let timer = &expected["timer"];
    let tick_id = text(&expected["ids"]["tick"]);
    let project = project_with(Some(&tick_id));
    let (host, mut compiler, mut link, mut player) = playing(&project, true);
    assert!(host.needs_ticks(), "a timer is registered");
    let start = std::time::Instant::now();
    let mut dirty = false;
    while start.elapsed() < Duration::from_millis(400) {
        dirty |= host
            .tick()
            .iter()
            .any(|notice| notice.owner == SINE && notice.kind == NoticeKind::Dirty);
        std::thread::sleep(Duration::from_millis(16));
    }
    let ticks_param = timer["ticks_param"].as_u64().expect("id") as u32;
    assert!(host.value(SINE, ticks_param).expect("ticks") >= 0.003);
    assert!(dirty, "the controller's gesture ended");
    link.send(Control::LiveNoteOn {
        key: 69,
        velocity: 1.0,
    })
    .expect("room");
    let out = play(&mut player, 1, 512);
    assert!(
        near(out[0], timer["edit"].as_f64().expect("edit")),
        "frame 0: {}",
        out[0]
    );
    let mut project = project;
    let applied =
        edit::apply(&mut project, Edit::DeleteChannel { id: ChannelId(1) }).expect("deletes");
    link.check_room().expect("room");
    link.push_plan(compile(&mut compiler, &project, &applied.touched))
        .expect("same time base");
    play(&mut player, 1, 64);
    link.drain_retired();
    assert!(host.needs_ticks());
    host.tick();
    assert_eq!(host.instances(), 0);
    assert!(!host.needs_ticks(), "nothing registered any more");
}

// 8 (saturated). 8 kHz at 522 BPM put ticks 0 to 5 in the first block: 600 long notes start
// there, the table of sounding notes holds 512, and the other 88 are not sent but counted.
#[test]
fn notes_past_the_sounding_table_are_dropped_and_counted() {
    let expected = expected();
    let answer = &expected["dropped"];
    let mut project = project_with(None);
    project.sample_rate = answer["rate"].as_u64().expect("rate") as u32;
    project.tempo = MilliBpm(answer["tempo_mbpm"].as_u64().expect("tempo") as u32);
    let pattern = project.patterns.get_mut(&PatternId(1)).expect("pattern 1");
    pattern.notes = (0..6)
        .flat_map(|tick| {
            (0..100).map(move |key| Note {
                channel: ChannelId(1),
                start: Ticks(tick),
                length: Ticks(300),
                velocity: 1.0,
                key,
            })
        })
        .collect();
    project.validate().expect("valid");
    let (host, _compiler, mut link, mut player) = playing(&project, false);
    link.send(Control::Play).expect("room");
    play(&mut player, 1, 512);
    assert_eq!(
        host.dropped_notes(SINE),
        answer["dropped"].as_u64().expect("dropped")
    );
}

// 8 again, after a reset: a Seek back to tick 0 with the table full ends the 512 sounding notes
// on frame 0 of the next call, so the 600 notes starting again find the table empty: 512 are
// sent and 88 more dropped, twice the generator's answer in all. (Review 2026-10-26: the starts
// were recorded before the ended notes left the table, and all 600 were dropped.)
#[test]
fn a_reset_with_the_table_full_makes_room_for_the_next_calls_notes() {
    let expected = expected();
    let answer = &expected["dropped"];
    let mut project = project_with(None);
    project.sample_rate = answer["rate"].as_u64().expect("rate") as u32;
    project.tempo = MilliBpm(answer["tempo_mbpm"].as_u64().expect("tempo") as u32);
    let pattern = project.patterns.get_mut(&PatternId(1)).expect("pattern 1");
    pattern.notes = (0..6)
        .flat_map(|tick| {
            (0..100).map(move |key| Note {
                channel: ChannelId(1),
                start: Ticks(tick),
                length: Ticks(300),
                velocity: 1.0,
                key,
            })
        })
        .collect();
    project.validate().expect("valid");
    let (host, _compiler, mut link, mut player) = playing(&project, false);
    link.send(Control::Play).expect("room");
    play(&mut player, 1, 512);
    let once = answer["dropped"].as_u64().expect("dropped");
    assert_eq!(host.dropped_notes(SINE), once);
    link.send(Control::Seek(Ticks(0))).expect("room");
    play(&mut player, 1, 512);
    assert_eq!(host.dropped_notes(SINE), 2 * once);
}

// A live note on and off in one call is a note at least a frame long: its note-off goes after
// its note-on, so the sine is silent from the next call on.
#[test]
fn a_live_note_on_and_off_in_one_call_ends() {
    let (_host, _compiler, mut link, mut player) = live_sine(Vec::new());
    link.send(Control::LiveNoteOn {
        key: 69,
        velocity: 0.5,
    })
    .expect("room");
    link.send(Control::LiveNoteOff { key: 69 }).expect("room");
    play(&mut player, 1, 512);
    assert_eq!(energy(&play(&mut player, 1, 512)), 0.0, "the note ended");
}

// Play and Seek reset the plugin (its notes end on frame 0 of the next call and processing
// stops and starts again), but a key held across them is still down: its note sounds on in the
// call after each, from frame 0 (gain 0.5 × velocity 0.5 × cos 0), until its note-off. Stop
// ends every live note (since 2026-10-28): the call after it is silent, the key's own release
// then ends nothing, and the key struck again plays.
#[test]
fn a_held_live_note_sounds_on_through_play_and_seek_and_ends_at_stop() {
    let (_host, _compiler, mut link, mut player) = live_sine(Vec::new());
    link.send(Control::LiveNoteOn {
        key: 69,
        velocity: 0.5,
    })
    .expect("room");
    assert!(
        near(play(&mut player, 1, 512)[0], 0.25),
        "held while stopped"
    );
    for control in [Control::Play, Control::Seek(Ticks(96))] {
        link.send(control).expect("room");
        let out = play(&mut player, 1, 512);
        assert!(energy(&out) > 0.0, "silent after {control:?}");
        assert!(near(out[0], 0.25), "after {control:?}, frame 0: {}", out[0]);
    }
    link.send(Control::Stop).expect("room");
    play(&mut player, 1, 512);
    assert_eq!(energy(&play(&mut player, 1, 512)), 0.0, "Stop ended it");
    link.send(Control::LiveNoteOff { key: 69 }).expect("room");
    assert_eq!(energy(&play(&mut player, 1, 512)), 0.0, "released already");
    link.send(Control::LiveNoteOn {
        key: 69,
        velocity: 0.5,
    })
    .expect("room");
    assert!(
        near(play(&mut player, 1, 512)[0], 0.25),
        "struck again after Stop"
    );
    link.send(Control::LiveNoteOff { key: 69 }).expect("room");
    play(&mut player, 1, 512);
    assert_eq!(energy(&play(&mut player, 1, 512)), 0.0, "released");
}

// The call that starts the pattern's A5 at frame 250 (tick 1 at 48 kHz and 120 BPM) also
// carries a live A4 at velocity 0.5: the monophonic sine plays the A4 from frame 0 and the A5
// takes over at frame 250 (by hand: 0.25, then f32(0.25·cos(τ·440·249/48000)) = -0.0506968237
// at 249, 0.5 at 250 and f32(0.5·cos(τ·880/48000)) = 0.496686369 at 251).
#[test]
fn a_live_note_and_a_pattern_note_in_one_call_play_in_time_order() {
    let a5 = Note {
        channel: ChannelId(1),
        start: Ticks(1),
        length: Ticks(96),
        velocity: 1.0,
        key: 81,
    };
    let (_host, _compiler, mut link, mut player) = live_sine(vec![a5]);
    link.send(Control::Play).expect("room");
    link.send(Control::LiveNoteOn {
        key: 69,
        velocity: 0.5,
    })
    .expect("room");
    let out = play(&mut player, 1, 512);
    assert!(
        near(out[0], 0.25),
        "the first sound is at frame 0: {}",
        out[0]
    );
    assert!(near(out[249], -0.050_696_823_7), "249: {}", out[249]);
    assert!(near(out[250], 0.5), "the pattern note at 250: {}", out[250]);
    assert!(near(out[251], 0.496_686_369), "251: {}", out[251]);
}

/// The VST3 effect `id` at `gain` in `slot` of the master, as effect `effect`, with the
/// generator's preset (a class that is its own controller).
fn effect(expected: &Value, preset: &str, class: &str, effect: u32, slot: u8) -> Effect {
    let mut processor = ProcessorRef::plugin(
        format!("vst3:{}", text(&expected["ids"][class])),
        "Dawsome Test Gain",
    );
    processor.state = Some(text(&expected["presets"][preset]));
    Effect {
        id: EffectId(effect),
        slot,
        processor,
    }
}

// The Latency class reports a latency of 64 frames, which the host gives as the instance's
// latency; and the transport it was given in call 9 of 512 frames after Play, at 44.1 kHz and
// 120 BPM, comes back from its own output changes into its controller: the tempo, the song
// position in quarter notes and samples, the bar, playing, the output clock, each divided by the
// generator's 2^24 (exact) so that it is a normalized value, which the host passes on whole.
#[test]
fn the_latency_and_the_transport_a_plugin_is_given_are_reported() {
    let expected = expected();
    let answer = &expected["transport"];
    let mut project = project_with(None);
    project.sample_rate = answer["rate"].as_u64().expect("rate") as u32;
    let mut late = effect(&expected, "gain_0.5", "latency", 1, 0);
    late.processor.state = None;
    project.mixer.inserts[usize::from(InsertId::MASTER.0)]
        .effects
        .push(late);
    project.mixer.next_effect_id = EffectId(2);
    project.validate().expect("valid");
    let (host, _compiler, mut link, mut player) = playing(&project, false);
    let owner = ProcessorOwner::Effect(EffectId(1));
    assert_eq!(
        host.latency(owner),
        Some(expected["latency"].as_u64().expect("latency") as u32)
    );
    assert_eq!(host.latency(SINE), Some(0));
    link.send(Control::Play).expect("room");
    let calls = answer["call"].as_u64().expect("call") as usize + 1;
    play(
        &mut player,
        calls,
        answer["block"].as_u64().expect("block") as usize,
    );
    assert!(host.tick().is_empty(), "read-only values mark nothing");
    // The quarter notes from the generator's tick (the JSON reader rounds a decimal fraction to
    // within one unit in the last place, not exactly): tick / 96, the bar its multiple of 4.
    let tick = answer["tick"].as_u64().expect("tick") as f64;
    let beats = tick / 96.0;
    let number = |key: &str| answer[key].as_f64().expect(key);
    let divisor = number("divisor");
    for (id, key, want) in [
        (10, "tempo", number("tempo")),
        (11, "beats", beats),
        (12, "bar", (beats / 4.0).floor() * 4.0),
        (13, "playing", number("playing")),
        (14, "samples", number("samples")),
        (15, "clock", number("clock")),
    ] {
        assert_eq!(
            host.value(owner, id),
            Some(want / divisor),
            "{key} (parameter {id})"
        );
    }
    assert!(
        (beats - number("beats")).abs() < 1e-15,
        "the generator's beats"
    );
}

// A plugin's output values out of a normalized value's range never reach its controller as they
// are: the Latency class's "Wild" (parameter 16) reports -0.5, 1.5 and NaN on its audio calls
// 0, 1, 2 (crates/test-plugin-vst3's `WILD`), and its controller keeps what it is given, so
// `value` shows what the host passed on. By hand: -0.5 is held to 0.0, 1.5 to 1.0, and a NaN is
// not passed on, so the controller keeps the 1.0 it had. (Review 2026-10-26.)
#[test]
fn a_plugins_output_values_reach_its_controller_inside_0_to_1() {
    let expected = expected();
    let mut project = project_with(None);
    let mut late = effect(&expected, "gain_0.5", "latency", 1, 0);
    late.processor.state = None;
    project.mixer.inserts[usize::from(InsertId::MASTER.0)]
        .effects
        .push(late);
    project.mixer.next_effect_id = EffectId(2);
    project.validate().expect("valid");
    let (host, _compiler, mut link, mut player) = playing(&project, false);
    let owner = ProcessorOwner::Effect(EffectId(1));
    link.send(Control::Play).expect("room");
    let seen: Vec<Option<f64>> = (0..3)
        .map(|_| {
            play(&mut player, 1, 512);
            assert!(host.tick().is_empty(), "read-only values mark nothing");
            host.value(owner, 16)
        })
        .collect();
    assert_eq!(seen, [Some(0.0), Some(1.0), Some(1.0)]);
}

/// The sine with the gain effect at 0.5 in the master's first slot, as effect 1.
fn sine_and_gain(expected: &Value) -> Project {
    let mut project = project_with(None);
    project.mixer.inserts[usize::from(InsertId::MASTER.0)]
        .effects
        .push(effect(expected, "gain_0.5", "gain", 1, 0));
    project.mixer.next_effect_id = EffectId(2);
    project.validate().expect("valid");
    project
}

// An effect removed by an edit comes back from the player, is dropped on this thread, and the
// next tick lets its instance go; only an instance on its way out needs ticks.
#[test]
fn a_removed_instance_is_let_go_on_the_next_tick() {
    let expected = expected();
    let mut project = sine_and_gain(&expected);
    let (host, mut compiler, mut link, mut player) = playing(&project, false);
    assert_eq!(host.instances(), 2);
    host.tick();
    assert!(!host.needs_ticks(), "two quiet instances");
    let bell = host.doorbell().expect("the VST3 host has a doorbell");
    assert!(std::sync::Arc::ptr_eq(
        &bell,
        &host.doorbell().expect("still there")
    ));
    let applied =
        edit::apply(&mut project, Edit::RemoveEffect { id: EffectId(1) }).expect("removes");
    link.check_room().expect("room");
    link.push_plan(compile(&mut compiler, &project, &applied.touched))
        .expect("same time base");
    play(&mut player, 1, 64);
    link.drain_retired();
    assert!(host.needs_ticks(), "the gain waits for a tick to go");
    host.tick();
    assert_eq!(host.instances(), 1, "the gain's instance is gone");
    assert!(!host.needs_ticks(), "the sine alone is quiet again");
}

// A player that goes while its processors are still in the plan takes no instance with it: the
// sine's state can still be captured. The instances a session's own handle created go with
// that session, once their audio halves have come back; the module stays loaded with the host.
#[test]
fn a_sessions_instances_go_with_it_and_the_module_with_the_host() {
    let expected = expected();
    let (host, _compiler, _link, player) = sine();
    drop(player);
    host.tick();
    assert_eq!(host.instances(), 1, "the sine is still there");
    assert_eq!(
        host.capture_state(SINE).expect("captures").state,
        text(&expected["presets"]["sine_0.5"])
    );

    let host = Vst3Host::new(catalogue());
    let session = host.session();
    let mut compiler = compiler(&session);
    let project = project_with(None);
    let update = compile(&mut compiler, &project, &Touched::default());
    let (link, player) = realtime::link(update, project.tempo);
    drop(player);
    host.tick();
    assert_eq!(host.instances(), 1, "the session still has the sine");
    drop((compiler, link, session));
    assert!(host.needs_ticks(), "its session is over: a tick lets it go");
    host.tick();
    assert_eq!(host.instances(), 0, "gone with its session");
    assert_eq!(host.modules(), 1, "the module stays while the host lives");
}

// 8 (changed). A module whose binary changed after its scan is not loaded into this process:
// its record no longer matches the binary, and the host says so.
#[test]
fn a_module_changed_after_its_scan_is_not_loaded() {
    let dir = std::env::temp_dir().join(format!("dawsome-vst3-changed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let module = dir.join(MODULE);
    copy_tree(&test_plugins::test_plugins_vst3().join(MODULE), &module);
    let catalogue = catalogue_of(module.clone());
    let binary = PluginFormat::Vst3.binary(&module);
    std::fs::File::options()
        .write(true)
        .open(&binary)
        .expect("opens")
        .set_modified(std::time::SystemTime::now() + Duration::from_secs(5))
        .expect("touches");
    let host = Vst3Host::new(catalogue);
    let expected = expected();
    let reference =
        ProcessorRef::plugin(format!("vst3:{}", text(&expected["ids"]["sine"])), "Sine");
    let error = host.create(SINE, &reference, 48_000).err();
    assert_eq!(
        error,
        Some(HostError::Failed(
            "its file changed after the last plugin scan".into()
        ))
    );
    assert_eq!(host.modules(), 0);
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).expect("folder");
    for entry in std::fs::read_dir(from).expect("reads") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copies");
        }
    }
}

// Chosen on purpose: a Linux single-file `.vst3` (a module built for older hosts) is loaded, as
// the format's own module finder takes any `.vst3` entry: the test module's binary as
// `Single.vst3` lists its classes and plays.
#[cfg(target_os = "linux")]
#[test]
fn a_single_file_module_loads_on_linux() {
    let dir = std::env::temp_dir().join(format!("dawsome-vst3-single-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("folder");
    let single = dir.join("Single.vst3");
    std::fs::copy(
        PluginFormat::Vst3.binary(&test_plugins::test_plugins_vst3().join(MODULE)),
        &single,
    )
    .expect("copies");
    let catalogue = catalogue_of(single);
    let classes = expected()["scan"]["plugins"]
        .as_array()
        .expect("plugins")
        .len();
    assert_eq!(catalogue.files[0].plugins.len(), classes);
    let host = Vst3Host::new(catalogue);
    let mut compiler = compiler(&host);
    let project = project_with(None);
    let update = compile(&mut compiler, &project, &Touched::default());
    assert!(compiler.take_notices().is_empty(), "found and made");
    let (mut link, mut player) = realtime::link(update, project.tempo);
    link.send(Control::Play).expect("room");
    let out = play(&mut player, 12, 512);
    assert!(near(out[6000], 0.5), "6000: {}", out[6000]);
}

// 12. The host's audio half, with the sine on a channel and the gain in a master slot: notes, a
// value every 10 calls, a live note held across a Stop and Play and released, plan updates that
// add and remove another gain instance, then, stopped, a value every call flushed through the
// idle call. The plugins' own allocations (their own allocator) are not counted; every call
// the host makes on the audio thread is.
#[test]
fn the_hosts_audio_half_never_touches_the_heap() {
    let expected = expected();
    let mut project = sine_and_gain(&expected);
    let pattern = project.patterns.get_mut(&PatternId(1)).expect("pattern 1");
    pattern.notes = (0..16)
        .map(|step| Note {
            channel: ChannelId(1),
            start: Ticks(step * 24),
            length: Ticks(if step % 2 == 0 { 0 } else { 12 }),
            velocity: 0.8,
            key: 57 + u8::try_from(step).expect("small"),
        })
        .collect();
    project.validate().expect("valid");
    let (host, mut compiler, mut link, mut player) = playing(&project, true);
    link.send(Control::Play).expect("room");
    let target = ParamTarget::Plugin { owner: SINE, id: 0 };
    let mut buffer = [[0.0f32; 2]; 512];
    let mut undo = None;
    for call in 0..1000 {
        if call % 10 == 0 {
            let value = f32::from(u8::try_from(call % 7).expect("small")) / 7.0;
            link.send(Control::Param(target, value)).expect("room");
        }
        if call == 400 {
            link.send(Control::LiveNoteOn {
                key: 60,
                velocity: 0.5,
            })
            .expect("room");
        }
        if call == 500 {
            link.send(Control::Stop).expect("room");
            link.send(Control::Play).expect("room");
        }
        if call == 700 {
            link.send(Control::LiveNoteOff { key: 60 }).expect("room");
        }
        let edit = match call {
            300 => {
                let added = effect(&expected, "gain_0.25", "gain", 2, 1);
                Some(Edit::AddEffect {
                    insert: InsertId::MASTER,
                    slot: added.slot,
                    id: added.id,
                    processor: added.processor,
                })
            }
            600 => undo.take(),
            _ => None,
        };
        if let Some(edit) = edit {
            let applied = edit::apply(&mut project, edit).expect("applies");
            if call == 300 {
                undo = Some(applied.inverse);
            }
            link.check_room().expect("room");
            compiler.release(link.take_freed_slots());
            link.push_plan(compile(&mut compiler, &project, &applied.touched))
                .expect("same time base");
        }
        assert_eq!(
            heap_calls_during(|| player.process(&mut buffer)),
            0,
            "call {call}"
        );
        link.drain_retired();
        host.tick();
    }
    // Stopped, each value goes in through the idle call's flush, which touches nothing either.
    link.send(Control::Stop).expect("room");
    for call in 0..100u8 {
        let value = f32::from(call % 7) / 7.0;
        link.send(Control::Param(target, value)).expect("room");
        assert_eq!(
            heap_calls_during(|| player.process(&mut buffer)),
            0,
            "stopped call {call}"
        );
        link.drain_retired();
        host.tick();
    }
    assert_eq!(
        host.value(SINE, 0),
        Some(f64::from(f32::from(99u8 % 7) / 7.0)),
        "flushed and handed on"
    );
    assert_eq!(host.created(), 3);
    assert_eq!(host.instances(), 2, "the added gain is gone again");
    assert!(compiler.take_notices().is_empty());
}
