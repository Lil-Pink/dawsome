//! VST3 in the app through the real link, with Dawsome's own VST3 test plugins: a
//! device's pitch bend and mod wheel reach the plugin through its MIDI mapping (the sine's
//! controller maps them to parameters 1 and 2), its stepped parameters hear whole steps, and
//! the restarts and the latency a plugin asks for are acted on. Expected values are read from
//! tests/fixtures/p28_expected.json (gen_p28_expected.py) and p27_expected.json.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![deny(clippy::multiple_unsafe_ops_per_block)]

use std::convert::Infallible;

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::Touched;
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::processor::{
    NoticeKind, PortEvent, ProcessContext, ProcessorHost, Registry, TransportInfo,
};
use dawsome_engine::project::{
    ChannelId, Effect, EffectId, InsertId, PatternId, ProcessorOwner, ProcessorRef, Project,
};
use dawsome_engine::realtime::{self, LiveControl, LiveNote, Player, RealtimeLink};
use dawsome_engine::time::{MilliBpm, Ticks};
use dawsome_engine::transport::{Control, ParamTarget};
use dawsome_host_common::catalogue::{self, Catalogue, FileRecord, FileStatus, PluginFormat};
use dawsome_host_vst3::{scan, Vst3Host};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;
#[path = "../../../tests/support/test_plugins.rs"]
mod test_plugins;

use counting_allocator::heap_calls_during;

const P27_SINE: &str = include_str!("../../../tests/fixtures/p27_vst3_sine.json");
const P27_LATENCY: &str = include_str!("../../../tests/fixtures/p27_vst3_latency.json");
const SINE: ProcessorOwner = ProcessorOwner::Channel(ChannelId(1));
const SLOT: ProcessorOwner = ProcessorOwner::Effect(EffectId(1));
const MODE: PlayMode = PlayMode::Pattern(PatternId(1));
const MODULE: &str = "Dawsome Test VST3.vst3";

fn p28() -> Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/p28_expected.json"))
        .expect("p28_expected.json parses")
}

fn p27() -> Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/p27_expected.json"))
        .expect("p27_expected.json parses")
}

fn text(value: &Value) -> String {
    value.as_str().expect("a text").to_owned()
}

fn number(value: &Value) -> f64 {
    value.as_f64().expect("a number")
}

fn catalogue() -> Catalogue {
    let module = test_plugins::test_plugins_vst3().join(MODULE);
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

/// The test module's `dawsome_test_vst3_restart(flags)`: every controller this thread made asks
/// its host for `restartComponent(flags)`; how many did.
fn restart(flags: i32) -> i32 {
    let binary = PluginFormat::Vst3.binary(&test_plugins::test_plugins_vst3().join(MODULE));
    // SAFETY: the test module, our own code, already loaded by the host (a second open of one
    // path is that image; it runs no entry).
    let library = unsafe { libloading::Library::new(binary) }.expect("opens");
    // SAFETY: the module exports `dawsome_test_vst3_restart(i32) -> i32`
    // (crates/test-plugin-vst3/src/lib.rs); `call` is used while `library` is open.
    let call = unsafe { library.get::<extern "C" fn(i32) -> i32>(b"dawsome_test_vst3_restart\0") }
        .expect("exported");
    call(flags)
}

fn compile(compiler: &mut Compiler, project: &Project) -> PlanUpdate {
    let silent = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(vec![1.0]));
    compiler
        .compile(project, &Touched::default(), MODE, silent)
        .expect("compiles")
}

/// `project` playing through a fresh host, channel 1 taking live notes.
fn playing(project: &Project) -> (Vst3Host, Compiler, RealtimeLink, Player) {
    let host = Vst3Host::new(catalogue());
    let mut registry = Registry::native();
    registry.add_host(Box::new(host.clone()));
    let mut compiler = Compiler::with_registry(registry);
    compiler.set_live_channel(Some(ChannelId(1)));
    let update = compile(&mut compiler, project);
    assert!(compiler.take_notices().is_empty(), "the plugins are found");
    let (link, player) = realtime::link(update, project.tempo);
    (host, compiler, link, player)
}

/// p27's sine with no notes of its own, its plugin `class` (the sine's when `None`) with
/// `state`.
fn quiet_sine(class: Option<&str>, state: Option<String>) -> Project {
    let mut project = Project::load_json(P27_SINE).expect("fixture loads");
    project
        .patterns
        .get_mut(&PatternId(1))
        .expect("pattern 1")
        .notes
        .clear();
    if let Some(class) = class {
        let processor = project
            .channels
            .get_mut(&ChannelId(1))
            .and_then(|channel| channel.processor.as_mut())
            .expect("a plugin channel");
        processor.id = format!("vst3:{class}");
        processor.state = state;
    }
    project
}

/// One call of `block` frames, cut into pieces of `pieces` (cycled, the last cut at the call's
/// end): the frames, left side (the sine writes both alike).
fn call(player: &mut Player, block: usize, pieces: &[usize]) -> Vec<f32> {
    let mut out = vec![[0.0f32; 2]; block];
    let mut at = 0;
    for &size in pieces.iter().cycle() {
        if at == block {
            break;
        }
        let end = (at + size).min(block);
        player.process(&mut out[at..end]);
        at = end;
    }
    assert!(
        out.iter().all(|frame| frame[0] == frame[1]),
        "both sides alike"
    );
    out.iter().map(|frame| frame[0]).collect()
}

fn sha256(frames: &[f32]) -> String {
    let mut sha = Sha256::new();
    for value in frames {
        sha.update(value.to_le_bytes());
        sha.update(value.to_le_bytes());
    }
    format!("{:x}", sha.finalize())
}

/// The bend case: a live A4 at velocity 0.5 from call 0, then before each call the bends
/// `bends` gives for it, each call cut as `pieces` says; the frames, and after each call a tick.
fn bend_run(
    project: &Project,
    bends: &dyn Fn(usize) -> Vec<i16>,
    pieces: &[usize],
) -> (Vst3Host, Vec<f32>) {
    let answer = &p28()["bend"];
    let (host, _compiler, link, mut player) = playing(project);
    let port = link.midi_inbox().for_port();
    assert!(port.push(LiveNote::On {
        key: 69,
        velocity: number(&answer["velocity"]) as f32,
    }));
    let block = answer["block"].as_u64().expect("block") as usize;
    let mut frames = Vec::new();
    for at in 0..answer["calls"].as_u64().expect("calls") as usize {
        for bend in bends(at) {
            assert!(port.push_control(None, LiveControl::Bend(bend), true));
        }
        frames.extend(call(&mut player, block, pieces));
    }
    (host, frames)
}

/// The generator's bends, before calls 1, 2 and 3.
fn the_bends(call: usize) -> Vec<i16> {
    p28()["bend"]["bends"]
        .as_array()
        .expect("bends")
        .iter()
        .filter(|bend| bend[0].as_u64() == Some(call as u64))
        .map(|bend| bend[1].as_i64().expect("b") as i16)
        .collect()
}

// K2. Bend +8191 before call 1, 0 before call 2 and -8192 before call 3 reach the VST3 sine as
// points of its "Bend" parameter, (b + 8192)/16383, at the first frame of the call that takes
// them; it bends as the CLAP sine does, so its 2,048 frames are the generator's to the bit (its
// probes and SHA-256), however each call is cut after the bend is taken (1, 7, 64 and seeded
// pieces). After the center is sent and a tick, the controller holds 8192/16383, and nothing is
// marked changed.
#[test]
fn a_bend_reaches_the_vst3_sine_through_its_midi_mapping() {
    let answer = &p28()["bend"];
    let project = quiet_sine(None, None);
    let want = text(&answer["sha256"]);
    let (host, frames) = bend_run(&project, &the_bends, &[512]);
    for probe in answer["probes"].as_array().expect("probes") {
        let at = probe[0].as_u64().expect("frame") as usize;
        // An f32 the generator printed: the JSON reader parses it to within an ULP of the f64,
        // which is the f32 again.
        assert_eq!(frames[at], number(&probe[1]) as f32, "frame {at}");
    }
    assert_eq!(sha256(&frames), want);
    assert!(host.tick().is_empty(), "values the host sent mark nothing");
    assert_eq!(
        host.value(SINE, answer["param"].as_u64().expect("id") as u32),
        Some(bend_value(-8192))
    );
    for pieces in answer["splits"].as_array().expect("splits") {
        let pieces: Vec<usize> = pieces
            .as_array()
            .expect("pieces")
            .iter()
            .map(|size| size.as_u64().expect("size") as usize)
            .collect();
        let (_, frames) = bend_run(&project, &the_bends, &pieces);
        assert_eq!(sha256(&frames), want, "cut in {pieces:?}");
    }
    // The center's value, after the call that took it and a tick.
    let (host, _compiler, link, mut player) = playing(&project);
    let port = link.midi_inbox().for_port();
    assert!(port.push_control(None, LiveControl::Bend(0), true));
    call(&mut player, 512, &[512]);
    assert!(host.tick().is_empty(), "not marked changed");
    assert_eq!(host.value(SINE, 1), Some(number(&answer["center_value"])));
}

fn bend_value(b: i16) -> f64 {
    number(&p28()["bend"]["values"][b.to_string()])
}

// K2 (messy). Three bends before one call (100, -3000, 8191) play as 8191 alone: a later point
// on the same frame replaces the earlier, as the last MIDI message wins in CLAP.
#[test]
fn three_bends_before_one_call_play_as_the_last() {
    let answer = &p28()["bend"];
    let three: Vec<i16> = answer["three_bends"]
        .as_array()
        .expect("bends")
        .iter()
        .map(|b| b.as_i64().expect("b") as i16)
        .collect();
    let bends = |at: usize| {
        if at == 1 {
            three.clone()
        } else {
            the_bends(at)
        }
    };
    let (_, frames) = bend_run(&quiet_sine(None, None), &bends, &[512]);
    assert_eq!(sha256(&frames), text(&answer["sha256"]));
}

// K2 (no mapping). The Tick class's controller has no MIDI mapping: it is sent no bend, so at
// gain 0.5 it plays the unbent sine (the generator's unbent SHA-256).
#[test]
fn a_plugin_without_a_mapping_is_sent_no_bend() {
    let answer = &p28()["bend"];
    let tick = text(&p28()["ids"]["tick"]);
    let project = quiet_sine(Some(&tick), Some(text(&answer["tick_preset"])));
    let (_, frames) = bend_run(&project, &the_bends, &[512]);
    assert_eq!(sha256(&frames), text(&answer["unbent_sha256"]));
}

// K2 (mapping changed). Once the sine's controller maps nothing (its restart with
// kMidiCCAssignmentChanged) and the host has ticked, the bends go nowhere: the unbent sine.
#[test]
fn a_mapping_that_goes_takes_the_bend_with_it() {
    let expected = p28();
    let answer = &expected["bend"];
    let project = quiet_sine(None, None);
    let (host, _compiler, link, mut player) = playing(&project);
    host.tick();
    let flag = expected["restart"]["flags"]["midi_cc"]
        .as_i64()
        .expect("flag") as i32;
    assert!(restart(flag) >= 1, "the sine's controller asked");
    host.tick();
    let port = link.midi_inbox().for_port();
    assert!(port.push(LiveNote::On {
        key: 69,
        velocity: 0.5
    }));
    let mut frames = Vec::new();
    for at in 0..4 {
        for bend in the_bends(at) {
            assert!(port.push_control(None, LiveControl::Bend(bend), true));
        }
        frames.extend(call(&mut player, 512, &[512]));
    }
    assert_eq!(sha256(&frames), text(&answer["unbent_sha256"]));
}

/// A context for one stopped block of the audio half called by hand, with `events`.
fn context(events: &[PortEvent]) -> ProcessContext<'_> {
    ProcessContext {
        sample_rate: 48_000,
        tempo: MilliBpm(120_000),
        ppq: 96,
        transport: TransportInfo {
            playing: false,
            song_tick: Ticks(0),
            steady_time: 0,
        },
        events,
    }
}

// K2 (messy, the audio half called by hand). The wheel at 100 reaches "Wheel" as 100/127, and
// the sine reports it as "Wheel heard"; a wheel of 200 is masked as CLAP's message is (72); a
// bend of 30000 is held to 8191 (the point 1.0).
#[test]
fn a_wheel_is_heard_and_out_of_range_controls_are_held() {
    let answer = &p28()["bend"];
    let host = Vst3Host::new(catalogue());
    let reference = ProcessorRef::plugin(format!("vst3:{}", text(&p28()["ids"]["sine"])), "Sine");
    let mut audio = host.create(SINE, &reference, 48_000).expect("creates");
    let heard = answer["wheel_heard"].as_u64().expect("id") as u32;
    let mut buffer = [[0.0f32; 2]; 64];
    for (wheel, key) in [("wheel", "heard"), ("masked_wheel", "heard")] {
        let value = answer[wheel]["value"].as_u64().expect("wheel") as u8;
        let events = [PortEvent::Wheel { frame: 0, value }];
        audio.process(&context(&events), &mut buffer);
        host.tick();
        assert_eq!(
            host.value(SINE, heard),
            Some(number(&answer[wheel][key])),
            "{wheel}"
        );
    }
    let clamped = &answer["clamped"];
    let events = [PortEvent::Bend {
        frame: 0,
        value: clamped["value"].as_i64().expect("bend") as i16,
    }];
    audio.process(&context(&events), &mut buffer);
    host.tick();
    assert_eq!(host.value(SINE, 1), Some(number(&clamped["sent"])));
    drop(audio);
}

/// The sine fixture with p27's gain effect (its 0.5 preset) in the master's first slot.
fn with_gain(mut project: Project) -> Project {
    let mut gain = ProcessorRef::plugin(
        format!("vst3:{}", text(&p28()["ids"]["gain"])),
        "Dawsome Test Gain",
    );
    gain.state = Some(text(&p27()["presets"]["gain_0.5"]));
    project.mixer.inserts[usize::from(InsertId::MASTER.0)]
        .effects
        .push(Effect {
            id: EffectId(1),
            slot: 0,
            processor: gain,
        });
    project.mixer.next_effect_id = EffectId(2);
    project.validate().expect("valid");
    project
}

// K3. The gain's "Steps" has 4 steps (0..4, default 4, text "4/4"); a value from the engine
// reaches the plugin as the nearest whole step over 4, whatever the control ring carried (2.4,
// 2.5, 2.6, 7, -1, NaN), flushed while stopped: its "Steps heard" is the generator's. The
// sine's parameters the MIDI mapping names (Bend, Wheel) are hidden from the panels.
#[test]
fn a_stepped_parameter_hears_whole_steps_and_mapped_ones_are_hidden() {
    let answer = &p28()["steps"];
    let (host, _compiler, mut link, mut player) = playing(&with_gain(quiet_sine(None, None)));
    let id = answer["param"].as_u64().expect("id") as u32;
    let steps = host
        .params(SLOT)
        .into_iter()
        .find(|param| param.id == id)
        .expect("Steps");
    let shape = &answer["shape"];
    assert_eq!(steps.name, text(&answer["name"]));
    assert_eq!(
        (steps.min, steps.max, steps.stepped, steps.default),
        (
            number(&shape["min"]),
            number(&shape["max"]),
            shape["stepped"].as_bool().expect("stepped"),
            number(&shape["default"])
        )
    );
    assert_eq!(
        host.value_text(SLOT, id, steps.default).as_deref(),
        Some(text(&answer["default_text"]).as_str())
    );
    let heard = answer["heard_param"].as_u64().expect("id") as u32;
    let target = ParamTarget::Plugin { owner: SLOT, id };
    for case in answer["ring"].as_array().expect("ring") {
        let sent = case[0].as_f64().map_or(f32::NAN, |value| value as f32);
        link.send(Control::Param(target, sent)).expect("room");
        call(&mut player, 64, &[64]);
        host.tick();
        assert_eq!(host.value(SLOT, heard), Some(number(&case[1])), "{sent}");
    }
    let shown: Vec<String> = host
        .params(SINE)
        .into_iter()
        .filter(|param| !param.hidden)
        .map(|param| param.name)
        .collect();
    let rows: Vec<String> = answer["sine_rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(text)
        .collect();
    assert_eq!(shown, rows);
}

// K4. A restart with kReloadComponent (and with kIoChanged) is a Restart notice for its `owner`
// after a tick; kLatencyChanged makes the Latency class report 128 frames, which the host
// reads again at the tick, and play its input 128 frames late (p28's render of
// p27_vst3_latency, to the bit).
#[test]
fn restarts_are_noticed_and_a_new_latency_is_heard() {
    let expected = p28();
    let flags = &expected["restart"]["flags"];
    let flag = |name: &str| flags[name].as_i64().expect("flag") as i32;
    let (host, ..) = playing(&quiet_sine(None, None));
    host.tick();
    for name in ["reload", "io"] {
        assert!(restart(flag(name)) >= 1);
        let notices = host.tick();
        assert!(
            notices
                .iter()
                .any(|notice| notice.owner == SINE && notice.kind == NoticeKind::Restart),
            "{name}: {notices:?}"
        );
    }
    drop(host);

    let answer = &expected["latency"];
    let project = Project::load_json(P27_LATENCY).expect("fixture loads");
    let (host, _compiler, mut link, mut player) = playing(&project);
    let late = ProcessorOwner::Effect(EffectId(1));
    assert_eq!(
        host.latency(late),
        answer["before"].as_u64().map(|n| n as u32)
    );
    assert!(restart(flag("latency")) >= 1);
    host.tick();
    assert_eq!(
        host.latency(late),
        answer["after"].as_u64().map(|n| n as u32)
    );
    let render = &answer["render"];
    link.send(Control::Play).expect("room");
    let total = render["frames"].as_u64().expect("frames") as usize;
    let mut frames = Vec::new();
    while frames.len() < total {
        frames.extend(call(&mut player, 512.min(total - frames.len()), &[512]));
    }
    for probe in render["probes"].as_array().expect("probes") {
        let at = probe[0].as_u64().expect("frame") as usize;
        // An f32 the generator printed: the JSON reader parses it to within an ULP of the f64,
        // which is the f32 again.
        assert_eq!(frames[at], number(&probe[1]) as f32, "frame {at}");
    }
    assert_eq!(sha256(&frames), text(&render["sha256"]));
}

// K4. A restart with kParamTitlesChanged (the gain renames its parameter 0 "Level") is a Params
// notice after a tick, and the parameters are read again; with kMidiCCAssignmentChanged (the
// sine maps nothing) too, and the sine's Bend and Wheel are no longer left out of the panels.
// Neither creates anything again. What a plugin asks inside the host's own state load (the
// sine's controller restarts inside `setComponentState`) is forgotten: opening the project makes
// one instance and the first tick reports nothing.
#[test]
fn renamed_and_remapped_parameters_are_a_params_notice() {
    let expected = p28();
    let flags = &expected["restart"]["flags"];
    let flag = |name: &str| flags[name].as_i64().expect("flag") as i32;
    let (host, ..) = playing(&with_gain(quiet_sine(None, None)));
    assert_eq!(host.created(), 2, "the sine and the gain, once each");
    assert!(host.tick().is_empty(), "the host's own loads are forgotten");
    assert!(restart(flag("titles")) >= 2);
    let notices = host.tick();
    for owner in [SINE, SLOT] {
        assert!(
            notices.contains(&dawsome_engine::processor::HostNotice {
                owner,
                kind: NoticeKind::Params
            }),
            "{owner:?}: {notices:?}"
        );
    }
    assert!(!notices
        .iter()
        .any(|notice| notice.kind == NoticeKind::Restart));
    assert_eq!(
        host.params(SLOT)[0].name,
        text(&expected["restart"]["renamed"])
    );
    assert!(restart(flag("midi_cc")) >= 1);
    let notices = host.tick();
    assert!(notices
        .iter()
        .any(|notice| notice.owner == SINE && notice.kind == NoticeKind::Params));
    let shown: Vec<String> = host
        .params(SINE)
        .into_iter()
        .filter(|param| !param.hidden)
        .map(|param| param.name)
        .collect();
    let all: Vec<String> = expected["steps"]["sine_params"]
        .as_array()
        .expect("names")
        .iter()
        .map(text)
        .map(|name| {
            if name == "Gain" {
                text(&expected["restart"]["renamed"])
            } else {
                name
            }
        })
        .collect();
    assert_eq!(shown, all, "nothing mapped, nothing left out");
    assert_eq!(host.created(), 2, "nothing made again");
}

// The audio half with a bend and a wheel every call and the mapping read again between calls
// (a restart with kMidiCCAssignmentChanged, served by the tick): nothing on the heap.
#[test]
fn bends_and_wheels_every_call_never_touch_the_heap() {
    let (host, _compiler, link, mut player) = playing(&quiet_sine(None, None));
    let port = link.midi_inbox().for_port();
    assert!(port.push(LiveNote::On {
        key: 69,
        velocity: 0.5
    }));
    let mut buffer = [[0.0f32; 2]; 512];
    for at in 0..400i16 {
        assert!(port.push_control(None, LiveControl::Bend(at * 40 - 8000), true));
        assert!(port.push_control(None, LiveControl::Wheel((at % 128) as u8), true));
        assert_eq!(
            heap_calls_during(|| player.process(&mut buffer)),
            0,
            "call {at}"
        );
        host.tick();
        if at == 200 {
            restart(32);
            host.tick();
        }
    }
}
