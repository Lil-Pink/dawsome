//! The CLAP host with Dawsome's own test plugins, through the real link: the compiler creates them,
//! the player runs them, the control ring carries their parameters, captured states round-trip byte
//! for byte, undo loads a state into the live instance, and the host's audio half never touches the
//! heap. Expected values are printed by tests/fixtures/gen_p7_expected.py.

use std::convert::Infallible;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, Edit, History, Touched};
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::processor::{Editor, HostError, ProcessorHost, Registry};
use dawsome_engine::project::{
    ChannelId, Effect, EffectId, InsertId, Note, PatternId, ProcessorOwner, ProcessorRef, Project,
};
use dawsome_engine::realtime::{self, LiveControl, LiveNote, Player, RealtimeLink};
use dawsome_engine::time::{MilliBpm, Ticks};
use dawsome_engine::transport::{Control, ParamTarget};
use dawsome_host_clap::catalogue::{
    self, FileRecord, FileStatus, GuiSupport, PluginFormat, PluginKind,
};
use dawsome_host_clap::{scan, Catalogue, ClapHost};

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;
#[path = "../../../tests/support/test_plugins.rs"]
mod test_plugins;

use counting_allocator::heap_calls_during;

const P7_SINE: &str = include_str!("../../../tests/fixtures/p7_sine.json");
const SINE: ProcessorOwner = ProcessorOwner::Channel(ChannelId(1));
const MODE: PlayMode = PlayMode::Pattern(PatternId(1));

/// A catalogue of the test plugin file, described in this process (the scan child's work).
fn catalogue() -> Catalogue {
    let file = test_plugins::test_plugins().join("dawsome_test_plugin.clap");
    let plugins = scan::describe_file(&file, || {}).expect("the test plugins load");
    let (size, mtime) = catalogue::stamp(&file).expect("the test plugin file is there");
    Catalogue {
        files: vec![FileRecord {
            file,
            format: PluginFormat::Clap,
            size,
            mtime,
            status: FileStatus::Ok,
            reason: None,
            plugins,
        }],
        ..Catalogue::default()
    }
}

/// A compiler that creates plugins through `host`.
fn compiler(host: &ClapHost) -> Compiler {
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

/// The test sine fixture playing through a fresh host: the host, the compiler, both halves.
fn sine() -> (ClapHost, Compiler, RealtimeLink, Player) {
    let host = ClapHost::new(catalogue());
    let mut compiler = compiler(&host);
    let project = Project::load_json(P7_SINE).expect("fixture loads");
    let update = compile(&mut compiler, &project, &Touched::default());
    assert!(compiler.take_notices().is_empty(), "the sine is found");
    let (link, player) = realtime::link(update, project.tempo);
    (host, compiler, link, player)
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

/// The test sine fixture with `notes` in its pattern in place of its own, its channel playing
/// live notes: the host, the compiler, both halves.
fn live_sine(notes: Vec<Note>) -> (ClapHost, Compiler, RealtimeLink, Player) {
    let host = ClapHost::new(catalogue());
    let mut compiler = compiler(&host);
    compiler.set_live_channel(Some(ChannelId(1)));
    let mut project = Project::load_json(P7_SINE).expect("fixture loads");
    project
        .patterns
        .get_mut(&PatternId(1))
        .expect("pattern 1")
        .notes = notes;
    let update = compile(&mut compiler, &project, &Touched::default());
    let (link, player) = realtime::link(update, project.tempo);
    (host, compiler, link, player)
}

fn energy(samples: &[f32]) -> f64 {
    samples.iter().map(|&value| f64::from(value).powi(2)).sum()
}

// A live note on and off in one call is a note at least a frame long: its note-off goes after
// its note-on, so the sine (which a note-on starts and a note-off of its key ends) is silent
// from the next call on.
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

// A plugin hears the sustain pedal as the engine holds it: the note-off of a key let go of under
// the pedal reaches the plugin on the lift's call, frame 0, so the test sine (which a note-off of
// its key ends) sounds through every call the pedal holds it and is silent from the lift on.
#[test]
fn a_pedal_holds_a_plugins_note_off_until_it_lifts() {
    let (_host, _compiler, link, mut player) = live_sine(Vec::new());
    let port = link.midi_inbox().for_port();
    assert!(port.push_control(None, LiveControl::Sustain(127), true));
    assert!(port.push(LiveNote::On {
        key: 69,
        velocity: 0.5
    }));
    assert!(near(play(&mut player, 1, 512)[0], 0.25), "struck");
    assert!(port.push(LiveNote::Off { key: 69 }));
    for call in 0..3 {
        assert!(
            energy(&play(&mut player, 1, 512)) > 0.0,
            "held by the pedal, call {call}"
        );
    }
    assert!(port.push_control(None, LiveControl::Sustain(0), true));
    assert_eq!(energy(&play(&mut player, 1, 512)), 0.0, "lifted: ended");
}

// A device's pitch bend reaches a plugin whose note port takes MIDI as a MIDI pitch bend at frame
// 0 of the call that takes it: the test sine (its note port takes CLAP notes and MIDI) plays A4 at
// 440 Hz for the first call, then from frame 512 on at 440·2^(8191/49152) from where its phase
// was: y = 0.25·cos(τ·(440·512 + 440·r·(n − 512))/48000) (the test plugin's formula, by hand: the
// fixture's gain 0.5 times velocity 0.5); the center plays 440 Hz on from there.
#[test]
fn a_bend_reaches_a_plugin_that_takes_midi_from_its_call() {
    let (_host, _compiler, link, mut player) = live_sine(Vec::new());
    let port = link.midi_inbox().for_port();
    assert!(port.push(LiveNote::On {
        key: 69,
        velocity: 0.5
    }));
    let first = play(&mut player, 1, 512);
    let unbent = |n: f64| 0.25 * (std::f64::consts::TAU * 440.0 * n / 48_000.0).cos();
    for n in [0, 1, 100, 511] {
        assert!(near(first[n], unbent(n as f64)), "frame {n}: {}", first[n]);
    }
    assert!(port.push_control(None, LiveControl::Bend(8191), true));
    let second = play(&mut player, 1, 512);
    let ratio = 2f64.powf(8191.0 / 49_152.0);
    let bent = |n: f64| {
        let sum = 440.0 * 512.0 + 440.0 * ratio * (n - 512.0);
        0.25 * (std::f64::consts::TAU * sum / 48_000.0).cos()
    };
    for j in [0, 1, 7, 200, 511] {
        let n = 512.0 + j as f64;
        assert!(
            near(second[j], bent(n)),
            "frame {n}: {} != {}",
            second[j],
            bent(n)
        );
    }
    assert!(
        !near(second[200], unbent(712.0)),
        "the bend moved the pitch"
    );
    assert!(port.push_control(None, LiveControl::Bend(0), true));
    let third = play(&mut player, 1, 512);
    let centered = |n: f64| {
        let sum = 440.0 * 512.0 + 440.0 * ratio * 512.0 + 440.0 * (n - 1024.0);
        0.25 * (std::f64::consts::TAU * sum / 48_000.0).cos()
    };
    for j in [0, 300] {
        let n = 1024.0 + j as f64;
        assert!(near(third[j], centered(n)), "frame {n}");
    }
}

/// The test sine on two plugin channels, A (1) and B (2, a copy of A), A playing live notes: the
/// project, the host, the compiler, both halves.
fn two_live_sines() -> (Project, ClapHost, Compiler, RealtimeLink, Player) {
    let host = ClapHost::new(catalogue());
    let mut compiler = compiler(&host);
    compiler.set_live_channel(Some(ChannelId(1)));
    let mut project = Project::load_json(P7_SINE).expect("fixture loads");
    project
        .patterns
        .get_mut(&PatternId(1))
        .expect("pattern 1")
        .notes
        .clear();
    let mut b = project.channels[&ChannelId(1)].clone();
    b.name = "Dawsome Test Sine B".into();
    project.channels.insert(ChannelId(2), b);
    project.next_channel_id = ChannelId(3);
    project.validate().expect("valid");
    let update = compile(&mut compiler, &project, &Touched::default());
    assert!(compiler.take_notices().is_empty(), "both sines are found");
    let (link, player) = realtime::link(update, project.tempo);
    (project, host, compiler, link, player)
}

// Found 2026-10-26: a muted plugin channel is still sent a
// device's bend, its rest included, so its plugin holds what the device does when it is
// unmuted. Bend +8191 on A (the test sine keeps a bend for the next note), A muted, the bend
// back at rest (on B, which rests A, or on A itself), A unmuted: A's next A4 plays the unbent
// formula 0.25·cos(τ·440·n/48000) from its note-on (by hand: gain 0.5 × velocity 0.5), not the
// bent one, 0.25·cos(τ·440·2^(8191/49152)·n/48000).
#[test]
fn a_muted_plugin_channel_hears_the_bend_go_back_to_rest() {
    let (a, b) = (ChannelId(1), ChannelId(2));
    let unbent = |n: f64| 0.25 * (std::f64::consts::TAU * 440.0 * n / 48_000.0).cos();
    for rest_on in [b, a] {
        let (mut project, _host, mut compiler, mut link, mut player) = two_live_sines();
        let port = link.midi_inbox().for_port();
        let mut mute = |project: &mut Project, link: &mut RealtimeLink, value| {
            let edit = Edit::SetChannelMute { channel: a, value };
            let applied = edit::apply(project, edit).expect("mutes");
            link.check_room().expect("room");
            link.push_plan(compile(&mut compiler, project, &applied.touched))
                .expect("same time base");
        };
        assert!(port.push_control(Some(a), LiveControl::Bend(8191), true));
        play(&mut player, 1, 512);
        mute(&mut project, &mut link, true);
        play(&mut player, 1, 512);
        assert!(port.push_control(Some(rest_on), LiveControl::Bend(0), true));
        play(&mut player, 1, 512);
        mute(&mut project, &mut link, false);
        play(&mut player, 1, 512);
        assert!(port.push_to(
            a,
            LiveNote::On {
                key: 69,
                velocity: 0.5
            }
        ));
        let out = play(&mut player, 1, 512);
        for n in [0, 1, 100, 511] {
            assert!(
                near(out[n], unbent(n as f64)),
                "rest on {rest_on:?}, frame {n}: {} != {}",
                out[n],
                unbent(n as f64)
            );
        }
    }
}

// Play and Seek reset the plugin, but a key held across them is still down: its note sounds on
// in the call after each, from frame 0 (gain 0.5 × velocity 0.5 × cos 0), until its note-off.
// Stop ends every live note (since 2026-10-28): the call after it is silent, the key's own
// release then ends nothing, and the key struck again plays. A note-on still waiting when Play
// comes sounds once, and its note-off ends it.
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
    link.send(Control::LiveNoteOn {
        key: 72,
        velocity: 0.5,
    })
    .expect("room");
    link.send(Control::Play).expect("room");
    assert!(
        energy(&play(&mut player, 1, 512)) > 0.0,
        "struck before Play"
    );
    link.send(Control::LiveNoteOff { key: 72 }).expect("room");
    play(&mut player, 1, 512);
    assert_eq!(
        energy(&play(&mut player, 1, 512)),
        0.0,
        "the note-off ends it"
    );
}

// The call that starts the pattern's A5 at frame 250 (tick 1: 250 samples a tick at 48 kHz and
// 120 BPM) also carries a live A4 at velocity 0.5. In time order the monophonic sine plays the
// A4 from frame 0, and the A5 takes over at frame 250 (python: 0.25, then
// f32(0.25·cos(τ·440·249/48000)) = -0.0506968237 at 249, 0.5 at 250 and
// f32(0.5·cos(τ·880/48000)) = 0.496686369 at 251).
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

#[test]
fn the_test_plugin_file_lists_both_plugins() {
    let mut described = 0;
    let plugins = scan::describe_file(
        &test_plugins::test_plugins().join("dawsome_test_plugin.clap"),
        || described += 1,
    )
    .expect("loads");
    assert_eq!(described, 2, "a progress call for each plugin described");
    let listed: Vec<(&str, &str, PluginKind, GuiSupport, u32)> = plugins
        .iter()
        .map(|p| (p.id.as_str(), p.name.as_str(), p.kind, p.gui, p.params))
        .collect();
    assert_eq!(
        listed,
        [
            (
                "org.dawsome.test.gain",
                "Dawsome Test Gain",
                PluginKind::Effect,
                GuiSupport::None,
                1
            ),
            (
                "org.dawsome.test.sine",
                "Dawsome Test Sine",
                PluginKind::Instrument,
                GuiSupport::None,
                1
            )
        ]
    );
    assert!(plugins
        .iter()
        .all(|p| p.vendor == "Dawsome" && p.version == "1.0.0"));
}

// 2. 512-frame calls; gain 0.25 sent before call 19 is heard from frame 19·512 = 9728:
// 9727 at gain 0.5, 9728 and 9729 at 0.25. Capturing then stores 'T1RQMQAAAAAAANA/'; one undo
// loads 'T1RQMQAAAAAAAOA/' back into the same instance.
#[test]
fn a_parameter_travels_over_the_control_ring_and_undo_reloads_the_state() {
    let (host, mut compiler, mut link, mut player) = sine();
    let mut project = Project::load_json(P7_SINE).expect("fixture loads");
    link.send(Control::Play).expect("room");
    let mut out = play(&mut player, 19, 512);
    let target = ParamTarget::Plugin { owner: SINE, id: 0 };
    link.send(Control::Param(target, 0.25)).expect("room");
    out.extend(play(&mut player, 2, 512));
    assert!(near(out[9727], 0.256_770_641), "9727: {}", out[9727]);
    assert!(near(out[9728], 0.115_824_006), "9728: {}", out[9728]);
    assert!(near(out[9729], 0.102_878_593), "9729: {}", out[9729]);
    assert_eq!(host.value(SINE, 0), Some(0.25));

    let captured = host.capture_state(SINE).expect("captures");
    assert_eq!(captured.state, "T1RQMQAAAAAAANA/");
    assert_eq!(captured.bytes, 12);
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
        "T1RQMQAAAAAAAOA/"
    );
    assert_eq!(host.created(), 1, "no new instance");
    assert_eq!(project.to_json().expect("serializes"), P7_SINE);
}

// 3. The fixture's state loads and captures back to the same 12 bytes.
#[test]
fn a_state_round_trips_through_the_plugin_byte_for_byte() {
    let (host, ..) = sine();
    let captured = host.capture_state(SINE).expect("captures");
    assert_eq!(captured.state, "T1RQMQAAAAAAAOA/");
    let bytes = BASE64.decode(&captured.state).expect("base64");
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(hex, "4f545031000000000000e03f");
}

// A value sent while stopped is flushed into the plugin from the audio thread, through the stopped
// player's idle call.
#[test]
fn a_parameter_sent_while_stopped_is_flushed() {
    let (host, _compiler, mut link, mut player) = sine();
    let target = ParamTarget::Plugin { owner: SINE, id: 0 };
    link.send(Control::Param(target, 0.75)).expect("room");
    play(&mut player, 1, 64);
    assert_eq!(host.value(SINE, 0), Some(0.75));
}

#[test]
fn a_plugin_the_catalogue_does_not_list_is_missing_and_a_bad_state_fails() {
    let host = ClapHost::new(catalogue());
    let owner = ProcessorOwner::Channel(ChannelId(9));
    let missing = ProcessorRef::plugin("clap:org.example.missing", "Missing Synth");
    assert!(matches!(
        host.create(owner, &missing, 48_000),
        Err(HostError::Missing)
    ));
    let mut refused = ProcessorRef::plugin("clap:org.dawsome.test.sine", "Sine");
    refused.state = Some("AAECAw==".into());
    let error = host.create(owner, &refused, 48_000).err();
    assert_eq!(
        error,
        Some(HostError::Failed("it refused its saved state".into()))
    );
    refused.state = Some("not base64!".into());
    assert!(matches!(
        host.create(owner, &refused, 48_000),
        Err(HostError::Failed(_))
    ));
    assert_eq!(host.created(), 0);
}

/// The gain effect at `gain` in `slot` of `insert`, as effect `id`.
fn gain_effect(id: u32, slot: u8, gain: f64) -> (InsertId, Effect) {
    let mut processor = ProcessorRef::plugin("clap:org.dawsome.test.gain", "Dawsome Test Gain");
    let mut bytes = b"OTP1".to_vec();
    bytes.extend_from_slice(&gain.to_le_bytes());
    processor.state = Some(BASE64.encode(bytes));
    (
        InsertId::MASTER,
        Effect {
            id: EffectId(id),
            slot,
            processor,
        },
    )
}

// An effect removed by an edit comes back from the player, is dropped on this thread, and the
// next tick lets its instance go.
#[test]
fn a_removed_plugin_instance_is_let_go_on_the_next_tick() {
    let host = ClapHost::new(catalogue());
    let mut compiler = compiler(&host);
    let mut project = Project::load_json(P7_SINE).expect("fixture loads");
    let (insert, effect) = gain_effect(1, 0, 0.5);
    project.mixer.inserts[usize::from(insert.0)]
        .effects
        .push(effect);
    project.mixer.next_effect_id = EffectId(2);
    project.validate().expect("valid");
    let update = compile(&mut compiler, &project, &Touched::default());
    let (mut link, mut player) = realtime::link(update, project.tempo);
    assert_eq!(host.instances(), 2);
    let applied =
        edit::apply(&mut project, Edit::RemoveEffect { id: EffectId(1) }).expect("removes");
    link.check_room().expect("room");
    link.push_plan(compile(&mut compiler, &project, &applied.touched))
        .expect("same time base");
    play(&mut player, 1, 64);
    link.drain_retired();
    host.tick();
    assert_eq!(host.instances(), 1, "the gain's instance is gone");
}

// Quiet instances (no timer, no window, nothing waiting) need no ticks: the window then only
// watches the host's doorbell, one for the whole host, the same each time it is asked for. An
// instance released by an edit needs ticks until the tick that lets it go.
#[test]
fn only_an_instance_on_its_way_out_needs_ticks() {
    let host = ClapHost::new(catalogue());
    let mut compiler = compiler(&host);
    let mut project = Project::load_json(P7_SINE).expect("fixture loads");
    let (insert, effect) = gain_effect(1, 0, 0.5);
    project.mixer.inserts[usize::from(insert.0)]
        .effects
        .push(effect);
    project.mixer.next_effect_id = EffectId(2);
    project.validate().expect("valid");
    let update = compile(&mut compiler, &project, &Touched::default());
    let (mut link, mut player) = realtime::link(update, project.tempo);
    host.tick();
    assert!(!host.needs_ticks(), "two quiet instances");
    let bell = host.doorbell().expect("the CLAP host has a doorbell");
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
    assert_eq!(host.instances(), 1);
    assert!(!host.needs_ticks(), "the sine alone is quiet again");
}

// A player that goes while its processors are still in the plan (its stream failed, say) takes
// no instance with it: the sine's state can still be captured and its window opened.
#[test]
fn dropping_the_player_keeps_every_instance() {
    let (host, _compiler, _link, player) = sine();
    drop(player);
    host.tick();
    assert_eq!(host.instances(), 1, "the sine is still there");
    assert_eq!(
        host.capture_state(SINE).expect("captures").state,
        "T1RQMQAAAAAAAOA/"
    );
    assert_eq!(host.open_editor(SINE, "Sine"), Ok(Editor::Generic));
}

// The instances a session's own handle created go with that session (a new document replaced
// it), once their audio halves have come back.
#[test]
fn a_sessions_instances_go_with_it() {
    let host = ClapHost::new(catalogue());
    let session = host.session();
    let mut compiler = compiler(&session);
    let project = Project::load_json(P7_SINE).expect("fixture loads");
    let update = compile(&mut compiler, &project, &Touched::default());
    let (link, player) = realtime::link(update, project.tempo);
    drop(player);
    host.tick();
    assert_eq!(host.instances(), 1, "the session still has the sine");
    drop((compiler, link, session));
    assert!(host.needs_ticks(), "its session is over: a tick lets it go");
    host.tick();
    assert_eq!(host.instances(), 0, "gone with its session");
}

// 8 kHz at 522 BPM put ticks 0 to 5 in the first block: 600 long notes start there, the sine's
// table of sounding notes holds 512, and the other 88 are not sent but counted.
#[test]
fn notes_past_the_sounding_table_are_dropped_and_counted() {
    let host = ClapHost::new(catalogue());
    let mut compiler = compiler(&host);
    let mut project = Project::load_json(P7_SINE).expect("fixture loads");
    project.sample_rate = 8_000;
    project.tempo = MilliBpm(522_000);
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
    let update = compile(&mut compiler, &project, &Touched::default());
    let (mut link, mut player) = realtime::link(update, project.tempo);
    link.send(Control::Play).expect("room");
    play(&mut player, 1, 512);
    assert_eq!(host.dropped_notes(SINE), 88);
}

// The host's audio half, with the test sine on a channel and the test gain in a master slot:
// notes, a plugin value every 10 calls, a live note held across a Stop and Play (struck again
// after them) and released, plan updates that add and remove another gain instance, then,
// stopped, a value every call flushed through the idle call. The plugins' own allocations
// (their own allocator) are not counted; every call the host makes on the audio thread is.
#[test]
fn the_hosts_audio_half_never_touches_the_heap() {
    let host = ClapHost::new(catalogue());
    let mut compiler = compiler(&host);
    compiler.set_live_channel(Some(ChannelId(1)));
    let mut project = Project::load_json(P7_SINE).expect("fixture loads");
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
    let (insert, effect) = gain_effect(1, 0, 0.5);
    project.mixer.inserts[usize::from(insert.0)]
        .effects
        .push(effect);
    project.mixer.next_effect_id = EffectId(2);
    project.validate().expect("valid");
    let update = compile(&mut compiler, &project, &Touched::default());
    let (mut link, mut player) = realtime::link(update, project.tempo);
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
        if call == 650 {
            // The pedal, the bend and the wheel: the bend and the wheel go to the plugin as MIDI,
            // the pedal holds the note-off at call 700 until it lifts at 750.
            link.send(Control::LiveControl {
                channel: None,
                control: LiveControl::Sustain(127),
            })
            .expect("room");
            link.send(Control::LiveControl {
                channel: None,
                control: LiveControl::Bend(-4000),
            })
            .expect("room");
            link.send(Control::LiveControl {
                channel: None,
                control: LiveControl::Wheel(100),
            })
            .expect("room");
        }
        if call == 700 {
            link.send(Control::LiveNoteOff { key: 60 }).expect("room");
        }
        if call == 750 {
            link.send(Control::LiveControl {
                channel: None,
                control: LiveControl::Sustain(0),
            })
            .expect("room");
        }
        let edit = match call {
            300 => {
                let (_, effect) = gain_effect(2, 1, 0.25);
                Some(Edit::AddEffect {
                    insert: InsertId::MASTER,
                    slot: effect.slot,
                    id: effect.id,
                    processor: effect.processor,
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
        Some(f64::from(1.0f32 / 7.0)),
        "flushed"
    );
    assert_eq!(host.created(), 3);
    assert_eq!(host.instances(), 2, "the added gain is gone again");
    assert!(compiler.take_notices().is_empty());
}
