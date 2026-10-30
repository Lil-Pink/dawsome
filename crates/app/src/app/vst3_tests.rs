//! VST3 plugins in the app, with Dawsome's own VST3 test module (crates/test-plugin-vst3) and
//! CLAP test plugins: the lists over both formats, a VST3 project opened and played, pitch bend
//! and the mod wheel through the port's door, stepped parameters in the parameter window,
//! restarts and the latency shown. Expected values come from tests/fixtures/p28_expected.json and
//! p27_expected.json (gen_p28_expected.py, gen_p27_expected.py).
//!
//! Every test holds [`VST3`] while it runs: the module's own counters (its entries) are the
//! process's, so the tests that load it take turns, and one that reads them sees its own loads
//! alone.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![deny(clippy::multiple_unsafe_ops_per_block)]

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use dawsome_engine::processor::Editor;
use dawsome_engine::project::{ChannelId, EffectId, InsertId, ProcessorOwner};
use dawsome_engine::realtime::Player;
use dawsome_host_clap::catalogue::{self, Catalogue, FileRecord, FileStatus, PluginFormat};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::plugins::app_tests::{app_hosted, catalogue_of, test_plugin_file};
use super::plugins::host_of;
use super::{App, Message};
use crate::document::Host;
use crate::ui::browser::hover_of_item;
use crate::ui::hint::{Hover, RackPart};
use crate::ui::instrument_window::{self as window, InstrumentMessage};
use crate::ui::library::Item;
use crate::ui::mixer_layout::Hit as MixerHit;
use crate::ui::param_window::ParamMessage;
use crate::ui::picker::{self as picker_view, PickerMessage, PickerTarget};

#[path = "../../../../tests/support/test_plugins.rs"]
mod test_plugins;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");
const MODULE: &str = "Dawsome Test VST3.vst3";
const CRASH: &str = "Dawsome Test VST3 Crash.vst3";
const SINE: ProcessorOwner = ProcessorOwner::Channel(ChannelId(1));
const SLOT: ProcessorOwner = ProcessorOwner::Effect(EffectId(1));

/// Taken by every test here (see the module's note).
static VST3: Mutex<()> = Mutex::new(());

fn turn() -> MutexGuard<'static, ()> {
    VST3.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn p28() -> Value {
    serde_json::from_str(include_str!("../../../../tests/fixtures/p28_expected.json"))
        .expect("p28_expected.json parses")
}

fn p27() -> Value {
    serde_json::from_str(include_str!("../../../../tests/fixtures/p27_expected.json"))
        .expect("p27_expected.json parses")
}

fn text(value: &Value) -> String {
    value.as_str().expect("a text").to_owned()
}

fn module() -> PathBuf {
    test_plugins::test_plugins_vst3().join(MODULE)
}

/// The VST3 test module's record, described in this process (the scan child's work).
fn module_record() -> FileRecord {
    let module = module();
    let plugins =
        dawsome_host_vst3::scan::describe_module(&module, || {}).expect("the test module loads");
    let (size, mtime) =
        catalogue::module_stamp(PluginFormat::Vst3, &module).expect("the module is there");
    FileRecord {
        file: module,
        format: PluginFormat::Vst3,
        size,
        mtime,
        status: FileStatus::Ok,
        reason: None,
        plugins,
    }
}

/// A catalogue of the VST3 test module alone.
fn vst3_catalogue() -> Catalogue {
    Catalogue {
        files: vec![module_record()],
        ..Catalogue::default()
    }
}

/// A catalogue of the CLAP test plugins, the VST3 test module and the module that crashes as it
/// loads (its failed record as a scan writes it, by hand, as p27_catalogue.json has it).
fn both_catalogue() -> Catalogue {
    let mut catalogue = catalogue_of(test_plugin_file());
    catalogue.files.push(module_record());
    catalogue.files.push(FileRecord {
        file: test_plugins::test_plugins_vst3().join(CRASH),
        format: PluginFormat::Vst3,
        size: 512,
        mtime: 2,
        status: FileStatus::Failed,
        reason: Some("crashed: signal 6".to_owned()),
        plugins: Vec::new(),
    });
    catalogue.sorted()
}

fn texts(value: &Value) -> Vec<String> {
    value.as_array().expect("a list").iter().map(text).collect()
}

/// The picker's dump for `target`, opened on `app`.
fn dump(app: &mut App, target: PickerTarget) -> Vec<String> {
    let _ = app.open_picker(target);
    let lines = app.picker_lines();
    app.picker = None;
    picker_view::dump(&lines)
        .lines()
        .map(str::to_owned)
        .collect()
}

/// An app on the fixture `fixture` hosting the plugins `catalogue` lists in both hosts.
fn app_on(fixture: &str, catalogue: Catalogue) -> (App, Player) {
    app_hosted(
        &Path::new(FIXTURES).join(fixture),
        Host::with_catalogue(catalogue),
    )
}

/// The test module's `dawsome_test_vst3_restart(flags)`: every controller this thread made asks
/// its host for `restartComponent(flags)`; how many did.
fn restart(flags: i32) -> i32 {
    let binary = PluginFormat::Vst3.binary(&module());
    // SAFETY: the test module, our own code, already loaded by the app's host (a second open of
    // one path is that image; it runs no entry).
    let library = unsafe { libloading::Library::new(binary) }.expect("opens");
    // SAFETY: the module exports `dawsome_test_vst3_restart(i32) -> i32`
    // (crates/test-plugin-vst3/src/lib.rs); `call` is used while `library` is open.
    let call = unsafe { library.get::<extern "C" fn(i32) -> i32>(b"dawsome_test_vst3_restart\0") }
        .expect("exported");
    call(flags)
}

/// The test module's `dawsome_test_vst3_module_entered()`: its `ModuleEntry` calls less its
/// `ModuleExit` calls, in this process.
fn entered() -> i32 {
    let binary = PluginFormat::Vst3.binary(&module());
    // SAFETY: as in `restart`.
    let library = unsafe { libloading::Library::new(binary) }.expect("opens");
    // SAFETY: the module exports `dawsome_test_vst3_module_entered() -> i32`, a read of a
    // counter; `read` is used while `library` is open.
    let read =
        unsafe { library.get::<extern "C" fn() -> i32>(b"dawsome_test_vst3_module_entered\0") }
            .expect("exported");
    read()
}

/// The restart flag `name` gen_p28 gives.
fn flag(name: &str) -> i32 {
    p28()["restart"]["flags"][name].as_i64().expect("a flag") as i32
}

/// The rows of the parameter window open now, by name.
fn rows(app: &App) -> Vec<String> {
    let panel = app.panel.as_ref().expect("a panel");
    panel
        .params
        .iter()
        .map(|param| param.name.clone())
        .collect()
}

/// `frames` frames played in calls of `block`, both sides.
fn play(player: &mut Player, frames: usize, block: usize) -> Vec<[f32; 2]> {
    let mut out = vec![[0.0f32; 2]; frames];
    for chunk in out.chunks_mut(block) {
        player.process(chunk);
    }
    out
}

fn sha256(frames: &[[f32; 2]]) -> String {
    let mut sha = Sha256::new();
    for [left, right] in frames {
        sha.update(left.to_le_bytes());
        sha.update(right.to_le_bytes());
    }
    format!("{:x}", sha.finalize())
}

/// `frames` against the render answer `answer` of p27_expected.json: its probes (f32s the JSON
/// reader parses to within an ULP) and its SHA-256.
fn assert_render(frames: &[[f32; 2]], answer: &Value) {
    assert_eq!(
        frames.len() as u64,
        answer["frames"].as_u64().expect("frames")
    );
    for probe in answer["probes"].as_array().expect("probes") {
        let at = probe[0].as_u64().expect("frame") as usize;
        let want = probe[1].as_f64().expect("value") as f32;
        assert_eq!(frames[at], [want; 2], "frame {at}");
    }
    assert_eq!(sha256(frames), text(&answer["sha256"]));
}

// K1. A project naming the VST3 sine opens with no notice (before VST3 reached the app it said
// "Missing plugin") and plays p27's frames: the sine from frame 6000 at gain 0.5, every frame
// of the bar the generator's; the VST3 gain in an insert plays p27's impulse at 0.5.
#[test]
fn a_vst3_project_opens_with_no_notice_and_plays_its_formula() {
    let _turn = turn();
    let expected = p27();
    let (mut app, mut player) = app_on("p27_vst3_sine.json", vst3_catalogue());
    assert_eq!(app.notice, None);
    let _ = app.update(Message::Play);
    let answer = &expected["renders"]["sine"];
    let frames = answer["frames"].as_u64().expect("frames") as usize;
    assert_render(&play(&mut player, frames, 512), answer);
    let (mut app, mut player) = app_on("p27_vst3_fx.json", vst3_catalogue());
    assert_eq!(app.notice, None);
    let _ = app.update(Message::Play);
    let answer = &expected["renders"]["fx"];
    let frames = answer["frames"].as_u64().expect("frames") as usize;
    assert_render(&play(&mut player, frames, 512), answer);
}

// K1. With the CLAP test plugins, the VST3 test module and the module that crashes beside them,
// the pickers list both formats as gen_p28 says: the VST3 sine left out for the CLAP one of the
// same name and maker, the VST3 Fail, Tick and Latency marked "(VST3)", the crashed module under
// "Could not load (1)". The Library lists the plugin instruments under their maker, Dawsome:
// Fail, Sine, Tick, the VST3 rows' hints saying "VST3" and the CLAP sine's "also installed as
// VST3".
#[test]
fn both_formats_are_listed_once_each_with_their_format() {
    let _turn = turn();
    let expected = p28();
    let lists = &expected["lists"]["both"];
    let (mut app, _player) = app_on("p7_sine.json", both_catalogue());
    assert_eq!(
        dump(&mut app, PickerTarget::Instrument),
        texts(&lists["instruments"])
    );
    let effect = PickerTarget::Effect {
        insert: dawsome_engine::project::InsertId(1),
        slot: 0,
    };
    assert_eq!(dump(&mut app, effect), texts(&lists["effects"]));
    let items = app.library_items();
    let maker = items
        .iter()
        .position(|item| *item == Item::Maker("Dawsome".to_owned()))
        .expect("the maker's label");
    let rows: Vec<&crate::ui::library::Item> = items[maker + 1..]
        .iter()
        .take_while(|item| matches!(item, Item::Plugin(_)))
        .collect();
    let want = lists["library"]["Dawsome"].as_array().expect("rows");
    assert_eq!(rows.len(), want.len());
    let context = app.hint_context();
    for (row, want) in rows.into_iter().zip(want) {
        let Item::Plugin(plugin) = row else {
            unreachable!("only plugin rows were taken")
        };
        assert_eq!(plugin.plugin.name, text(&want["name"]));
        assert_eq!(plugin.format.name(), text(&want["format"]));
        let hover = hover_of_item(row, &[], false).expect("a hint");
        let words = crate::ui::hint::words(&hover, &context);
        let start = match want["note"].as_str() {
            Some(note) => format!("{} \u{b7} {note} \u{b7} double-click", plugin.plugin.name),
            None => format!("{} \u{b7} double-click", plugin.plugin.name),
        };
        assert!(words.starts_with(&start), "{words}");
    }
}

// K1. With the VST3 module alone, on the VST3 sine's project: the picker lists Fail, Sine and
// Tick, each "(VST3)"; choosing Sine adds channel 2 playing `vst3:<the sine's id>` named
// "Dawsome Test Sine", with no state, selected, one undo step; its first capture is p27's
// sine_1.0 preset (its own state as it starts, gain 1).
#[test]
fn a_vst3_plugin_chosen_in_the_picker_is_a_new_channel() {
    let _turn = turn();
    let expected = p28();
    let lists = &expected["lists"]["vst3_alone"];
    let (mut app, _player) = app_on("p27_vst3_sine.json", vst3_catalogue());
    assert_eq!(
        dump(&mut app, PickerTarget::Instrument),
        texts(&lists["instruments"])
    );
    let _ = app.open_picker(PickerTarget::Instrument);
    let _ = app.picker_message(PickerMessage::FilterEdited("Sine".to_owned()));
    let _ = app.picker_message(PickerMessage::Submit);
    let added = &lists["added"];
    let channel = ChannelId(added["channel"].as_u64().expect("an id") as u32);
    let project = app.session.project();
    let processor = project.channels[&channel]
        .processor
        .as_ref()
        .expect("a plugin channel");
    assert_eq!(processor.id, text(&added["id"]));
    assert_eq!(processor.display_name(), text(&added["name"]));
    assert_eq!(processor.state, None);
    assert_eq!(project.channels[&channel].name, text(&added["name"]));
    assert_eq!(app.selected_channel, Some(channel));
    assert_eq!(app.session.history().undo_len(), 1);
    let owner = ProcessorOwner::Channel(channel);
    let captured = host_of(&app.session, owner)
        .expect("its host")
        .capture_state(owner)
        .expect("captures");
    assert_eq!(captured.state, text(&lists["first_capture"]));
    assert_eq!(
        captured.state,
        text(&p27()["presets"]["sine_1.0"]),
        "p27's preset"
    );
}

// K3. The VST3 gain's parameter window lists Gain, Steps and Steps heard; Steps is stepped from
// 0 to 4, its default 4 ("4/4"). Its slider (120 px, gen_p28's slider width, its 4 steps)
// pressed and moved 90 px left sends 1.0; after a stopped call (the idle flush) and a tick the
// plugin heard 0.25 ("Steps heard"), the window reads 1.0 and "1/4", and the capture is p27's
// gain_0.5 (Steps is not saved), so no undo step. Values sent over the control ring are whole
// steps when heard (gen_p28's ring cases). The VST3 sine's window leaves out Bend and Wheel,
// which its MIDI mapping names.
#[test]
fn a_stepped_vst3_parameter_snaps_in_the_panel_and_mapped_ones_are_left_out() {
    let _turn = turn();
    let expected = p28();
    let answer = &expected["steps"];
    let (mut app, mut player) = app_on("p27_vst3_fx.json", vst3_catalogue());
    app.open_panel(SLOT);
    assert_eq!(rows(&app), texts(&answer["gain_rows"]));
    let panel = app.panel.as_ref().expect("a panel");
    let steps = &panel.params[1];
    let shape = &answer["shape"];
    assert_eq!(
        (steps.min, steps.max, steps.stepped, steps.default),
        (
            shape["min"].as_f64().expect("min"),
            shape["max"].as_f64().expect("max"),
            true,
            shape["default"].as_f64().expect("default")
        )
    );
    assert_eq!(panel.texts[1], text(&answer["default_text"]));
    let drag = &answer["drag"];
    assert_eq!(
        f64::from(crate::ui::param_window::SLIDER_WIDTH),
        drag["slider_width"].as_f64().expect("a width")
    );
    let _ = app.update(Message::Params(ParamMessage::Pressed(1)));
    let pixels = drag["pixels"].as_f64().expect("pixels") as f32;
    let _ = app.update(Message::DragMoved(500.0));
    let _ = app.update(Message::DragMoved(500.0 + pixels));
    assert_eq!(
        app.plugin_sent.map(|(_, id, value)| (id, value)),
        Some((1, drag["sent"].as_f64().expect("sent") as f32))
    );
    player.process(&mut [[0.0; 2]; 64]);
    let _ = app.update(Message::DragEnded);
    for when in app.captures.values_mut() {
        *when = std::time::Instant::now();
    }
    let _ = app.update(Message::PluginTick);
    let _ = app.update(Message::PanelTick);
    let host = host_of(&app.session, SLOT).expect("its host");
    assert_eq!(host.value(SLOT, 2), drag["heard"].as_f64());
    assert_eq!(host.value(SLOT, 1), drag["value"].as_f64());
    let panel = app.panel.as_ref().expect("a panel");
    assert_eq!(panel.texts[1], text(&drag["text"]));
    assert_eq!(
        app.session.history().undo_len(),
        0,
        "the state is as it was"
    );
    assert_eq!(
        app.plugin_reference(SLOT).and_then(|r| r.state.clone()),
        Some(text(&answer["capture"]))
    );
    let steps = app.plugin_drag(SLOT, 1).expect("Steps moves");
    for case in answer["ring"].as_array().expect("ring") {
        let sent = case[0].as_f64().map_or(f32::NAN, |value| value as f32);
        app.set_plugin_param(steps, sent);
        player.process(&mut [[0.0; 2]; 64]);
        let _ = app.update(Message::PluginTick);
        let host = host_of(&app.session, SLOT).expect("its host");
        assert_eq!(host.value(SLOT, 2), case[1].as_f64(), "{sent}");
    }
    let (mut app, _player) = app_on("p27_vst3_sine.json", vst3_catalogue());
    app.open_panel(SINE);
    assert_eq!(rows(&app), texts(&answer["sine_rows"]));
}

// The parameter window's timer reads the rows it builds: the gain's Steps set to 1 (gen_p28's
// "1/4") is read for row 1 when row 1 is among the items read, not when only row 0 is; set back
// to its default 4 ("4/4"), the timer reads it, the window at its first size building all three
// rows.
#[test]
fn the_parameter_windows_timer_reads_the_rows_it_builds() {
    let _turn = turn();
    let answer = &p28()["steps"];
    let (mut app, mut player) = app_on("p27_vst3_fx.json", vst3_catalogue());
    app.open_panel(SLOT);
    let steps = app.plugin_drag(SLOT, 1).expect("Steps moves");
    app.set_plugin_param(steps, 1.0);
    player.process(&mut [[0.0; 2]; 64]);
    let _ = app.update(Message::PluginTick);
    let text_of_steps = |app: &App| app.panel.as_ref().map(|panel| panel.texts[1].clone());
    let default = text(&answer["default_text"]);
    let moved = text(&answer["drag"]["text"]);
    let read = |app: &mut App, items: std::ops::Range<usize>| {
        let host = host_of(&app.session, SLOT).expect("its host");
        if let Some(panel) = app.panel.as_mut() {
            panel.refresh_items(host, items);
        }
    };
    read(&mut app, 0..1);
    assert_eq!(text_of_steps(&app), Some(default.clone()), "row 0 alone");
    read(&mut app, 1..2);
    assert_eq!(text_of_steps(&app), Some(moved), "row 1 read");
    let back = answer["shape"]["default"].as_f64().expect("default") as f32;
    app.set_plugin_param(steps, back);
    player.process(&mut [[0.0; 2]; 64]);
    let _ = app.update(Message::PluginTick);
    let _ = app.update(Message::PanelTick);
    assert_eq!(text_of_steps(&app), Some(default), "the timer");
}

// K4. The VST3 sine asks for a reload (kReloadComponent, then kIoChanged): after a tick the app
// has captured it (p27's sine_0.5, the document's own: no undo step, nothing unsaved) and made it
// again from that state, and the old instance goes once its audio half is back; Play then gives
// p27's frames to the bit. Opening the project made one instance though the sine's controller
// asks the host to restart inside its own state load.
#[test]
fn a_vst3_plugin_asking_for_a_reload_is_made_again_from_its_state() {
    let _turn = turn();
    let expected = p27();
    let (mut app, mut player) = app_on("p27_vst3_sine.json", vst3_catalogue());
    let _ = app.update(Message::PluginTick);
    assert_eq!(
        app.host.vst3.created(),
        1,
        "one instance for the one channel"
    );
    for (round, name) in ["reload", "io"].into_iter().enumerate() {
        assert!(restart(flag(name)) >= 1, "{name}");
        let _ = app.update(Message::PluginTick);
        assert_eq!(app.host.vst3.created(), round + 2, "{name}: made again");
        player.process(&mut [[0.0; 2]; 64]);
        app.session.flush();
        let _ = app.update(Message::PluginTick);
        assert_eq!(app.host.vst3.instances(), 1, "{name}: the old one gone");
        assert_eq!(app.session.history().undo_len(), 0, "{name}");
        assert!(!app.session.is_dirty(), "{name}");
        assert_eq!(
            app.plugin_reference(SINE).and_then(|r| r.state.clone()),
            Some(text(&expected["presets"]["sine_0.5"]))
        );
    }
    let _ = app.update(Message::Play);
    let answer = &expected["renders"]["sine"];
    let frames = answer["frames"].as_u64().expect("frames") as usize;
    assert_render(&play(&mut player, frames, 512), answer);
}

// K4. The VST3 gain renames its parameter 0 "Level" (kParamTitlesChanged) with its parameter
// window open: after a tick the window's first row reads "Level" (its label too), its list
// scrolled as it was, and nothing was made again.
#[test]
fn a_renamed_vst3_parameter_renames_the_open_panels_row() {
    let _turn = turn();
    let (mut app, _player) = app_on("p27_vst3_fx.json", vst3_catalogue());
    app.open_panel(SLOT);
    assert_eq!(rows(&app)[0], "Gain");
    let created = app.host.vst3.created();
    let _ = app.update(Message::Params(ParamMessage::Scrolled(24.0)));
    assert!(restart(flag("titles")) >= 1);
    let _ = app.update(Message::PluginTick);
    assert_eq!(rows(&app)[0], text(&p28()["restart"]["renamed"]));
    let panel = app.panel.as_ref().expect("still open");
    assert_eq!(panel.labels[0], text(&p28()["restart"]["renamed"]));
    assert_eq!(panel.scroll, 24.0, "its scroll kept");
    assert_eq!(app.host.vst3.created(), created);
}

// K4. The VST3 Latency effect (44.1 kHz) says it delays its sound by 64 frames: its parameter
// window's footer says so with its milliseconds, as gen_p28 writes it; asked to restart with
// kLatencyChanged, after a tick it says 128, its slot's hint value too ("VST3 · delays its sound
// by 128 frames (2.9 ms); not compensated"), and Play puts the impulse at 128 (p28's render, to
// the bit). The CLAP sine's window says no latency.
#[test]
fn a_vst3_plugins_latency_is_shown_in_its_panel_and_slot_hint() {
    let _turn = turn();
    let expected = p28();
    let answer = &expected["latency"];
    let (mut app, mut player) = app_on("p27_vst3_latency.json", vst3_catalogue());
    app.open_panel(SLOT);
    let foot = |app: &App| app.panel.as_ref().and_then(|panel| panel.foot.clone());
    assert_eq!(foot(&app), Some(text(&answer["foot_before"])));
    assert!(restart(flag("latency")) >= 1);
    let _ = app.update(Message::PluginTick);
    let _ = app.update(Message::PanelTick);
    assert_eq!(foot(&app), Some(text(&answer["foot_after"])));
    app.mixer.selected = InsertId(1);
    let words = crate::ui::hint::words(&Hover::Mixer(MixerHit::Slot(0)), &app.hint_context());
    assert!(
        words.contains(&format!("\u{b7} {} \u{b7}", text(&answer["slot_hint"]))),
        "{words}"
    );
    let _ = app.update(Message::Play);
    let render = &answer["render"];
    let frames = play(
        &mut player,
        render["frames"].as_u64().expect("frames") as usize,
        512,
    );
    for probe in render["probes"].as_array().expect("probes") {
        let at = probe[0].as_u64().expect("frame") as usize;
        let want = probe[1].as_f64().expect("value") as f32;
        assert_eq!(frames[at], [want; 2], "frame {at}");
    }
    assert_eq!(sha256(&frames), text(&render["sha256"]));
    let (mut app, _player) = app_on("p7_sine.json", catalogue_of(test_plugin_file()));
    app.open_panel(SINE);
    assert_eq!(foot(&app), None, "a CLAP plugin says no latency");
}

// The VST3 sine's channel: its rack name's hint value says "VST3"; its instrument window calls it
// a "VST3 plugin instrument" and its foot line is any plugin channel's (the plugin's own
// window has the rest), and "Open plugin window" opens the parameter window (the test sine has
// no editor).
#[test]
fn a_vst3_channel_says_so_in_the_rack_and_its_window() {
    let _turn = turn();
    let (mut app, _player) = app_on("p27_vst3_sine.json", vst3_catalogue());
    let name = Hover::Rack {
        row: 0,
        part: RackPart::Name,
    };
    let words = crate::ui::hint::words(&name, &app.hint_context());
    assert!(
        words.starts_with("Dawsome Test Sine \u{b7} VST3 \u{b7} click"),
        "{words}"
    );
    let _ = app.update(Message::OpenInstrument(ChannelId(1)));
    assert_eq!(
        app.instrument_title(),
        "Dawsome Test Sine \u{b7} VST3 plugin instrument"
    );
    let channel = &app.session.project().channels[&ChannelId(1)];
    assert_eq!(window::kind_words(channel), "VST3 plugin instrument");
    let plugin = dawsome_engine::project::ChannelKind::Plugin;
    assert_eq!(app.instrument_hint(plugin), window::help(plugin, &app.keys));
    assert!(app
        .instrument_hint(plugin)
        .ends_with("the plugin's own window has the rest."));
    assert!(app.panel.is_none());
    let _ = app.update(Message::Instrument(InstrumentMessage::OpenPlugin));
    assert_eq!(app.panel.as_ref().map(|panel| panel.owner), Some(SINE));
}

/// Set in the child [`a_vst3_editor_without_x11_names_the_reason_and_opens_the_panel`] starts.
const NO_X11_CHILD: &str = "DAWSOME_APP_NO_X11_CHILD";

// The View class, whose editor takes our window type, on a desktop without X11 (a
// child of this test binary with no DISPLAY): opening its window puts the reason in the toolbar
// and opens its parameter window, as before; the test sine, which has no editor, opens its
// parameter window with no notice (gen_p29's no_display reason).
#[test]
fn a_vst3_editor_without_x11_names_the_reason_and_opens_the_panel() {
    let test = "a_vst3_editor_without_x11_names_the_reason_and_opens_the_panel";
    if std::env::var_os(NO_X11_CHILD).is_none() {
        let name = format!("app::vst3_tests::{test}");
        let output = std::process::Command::new(std::env::current_exe().expect("the test binary"))
            .args([name.as_str(), "--exact", "--nocapture", "--test-threads=1"])
            .env(NO_X11_CHILD, "1")
            .env_remove("DISPLAY")
            .output()
            .expect("run the test again");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "{test} in its child ({}):\n{stdout}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let _turn = turn();
    let p29: Value =
        serde_json::from_str(include_str!("../../../../tests/fixtures/p29_expected.json"))
            .expect("p29_expected.json parses");
    let reason = text(&p29["no_display"]["reason"]);
    let (mut app, _player) = app_on("p29_vst3_view.json", vst3_catalogue());
    assert_eq!(app.open_plugin(SINE), Some(Editor::Generic));
    assert_eq!(app.notice.as_deref(), Some(reason.as_str()));
    assert_eq!(app.panel.as_ref().map(|panel| panel.owner), Some(SINE));
    let (mut app, _player) = app_on("p27_vst3_sine.json", vst3_catalogue());
    assert_eq!(app.open_plugin(SINE), Some(Editor::Generic));
    assert_eq!(app.notice, None);
    assert_eq!(app.panel.as_ref().map(|panel| panel.owner), Some(SINE));
}

/// gen_p28's bend case through the app's door: a port delivers A4 at velocity 64 (0.5) before
/// call 0 and the bends' MIDI messages before calls 1, 2 and 3 to the selected channel 1, the
/// player called in 512-frame calls while stopped; the 2,048 frames.
fn bend_through_the_door(app: &App, player: &mut Player) -> Vec<[f32; 2]> {
    let answer = &p28()["bend"];
    let bytes = |value: &Value| -> Vec<u8> {
        value
            .as_array()
            .expect("bytes")
            .iter()
            .map(|byte| byte.as_u64().expect("a byte") as u8)
            .collect()
    };
    app.router().deliver(&bytes(&answer["note_on"]));
    let mut frames = Vec::new();
    for call in 0..answer["calls"].as_u64().expect("calls") {
        for message in answer["midi"].as_array().expect("messages") {
            if message[0].as_u64() == Some(call) {
                app.router().deliver(&bytes(&message[1]));
            }
        }
        frames.extend(play(player, 512, 512));
    }
    frames
}

// K2. A device's pitch bend through the app's door bends the CLAP sine (MIDI bend messages) and
// the VST3 sine (points of the parameter its MIDI mapping names) alike: both play gen_p28's
// 2,048 frames to the bit. The mod wheel at 100 reaches the VST3 sine as 100/127 ("Wheel heard"
// after a call and a tick), and neither control marks the document changed.
#[test]
fn a_devices_bend_plays_alike_in_both_formats() {
    let _turn = turn();
    let answer = &p28()["bend"];
    let want = text(&answer["sha256"]);
    let (app, mut player) = app_on("p7_sine.json", catalogue_of(test_plugin_file()));
    assert_eq!(
        sha256(&bend_through_the_door(&app, &mut player)),
        want,
        "CLAP"
    );
    let (mut app, mut player) = app_on("p27_vst3_sine.json", vst3_catalogue());
    assert_eq!(
        sha256(&bend_through_the_door(&app, &mut player)),
        want,
        "VST3"
    );
    let _ = app.update(Message::PluginTick);
    let host = host_of(&app.session, SINE).expect("its host");
    let param = answer["param"].as_u64().expect("id") as u32;
    assert_eq!(host.value(SINE, param), Some(0.0), "the last bend, -8192");
    let wheel = &answer["wheel"];
    let value = wheel["value"].as_u64().expect("a value") as u8;
    app.router().deliver(&[0xB0, 1, value]);
    play(&mut player, 512, 512);
    let _ = app.update(Message::PluginTick);
    let host = host_of(&app.session, SINE).expect("its host");
    let heard = answer["wheel_heard"].as_u64().expect("id") as u32;
    assert_eq!(host.value(SINE, heard), wheel["heard"].as_f64());
    assert_eq!(app.session.history().undo_len(), 0);
    assert!(!app.session.is_dirty());
}

// An export while the window's host holds the VST3 sine: the export makes hosts of its own, so
// the module is entered again while it renders and left again after, at 1 (the window's), as
// the format's entry counts; the window's sine plays on.
#[test]
fn an_export_enters_the_module_the_window_holds_and_leaves_it_held() {
    let _turn = turn();
    let dir = std::env::temp_dir()
        .join(format!("dawsome-app-vst3-{}", std::process::id()))
        .join("export");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let cache = dir.join("catalogue.json");
    vst3_catalogue().save(&cache).expect("saves");
    let host = Host {
        plugins: Some(dawsome_host_clap::PluginSetup {
            dirs: vec![test_plugins::test_plugins_vst3()],
            cache,
            child: dawsome_host_clap::scan::ChildCommand {
                program: PathBuf::from("false"),
                args: Vec::new(),
            },
            formats: PluginFormat::ALL,
        }),
        ..Host::with_catalogue(vst3_catalogue())
    };
    let (mut app, _player) = app_hosted(&Path::new(FIXTURES).join("p27_vst3_sine.json"), host);
    let held = entered();
    assert_eq!(held, 1, "the window's host holds the module");
    let export = |message| Message::File(super::FileMessage::ExportForm(message));
    let _ = app.update(Message::File(super::FileMessage::Export(
        super::ExportKind::Wav,
    )));
    let _ = app.update(export(super::ExportMessage::Bars("1".into())));
    let _ = app.update(export(super::ExportMessage::Start));
    let _ = app.update(export(super::ExportMessage::Picked(Some(dir.join("sine")))));
    let notice = app.notice.as_deref().unwrap_or_default();
    assert!(notice.starts_with("Exported "), "{notice}");
    assert!(!notice.contains("Missing plugin"), "{notice}");
    assert_eq!(entered(), held, "the export's own entry is left again");
    let host = host_of(&app.session, SINE).expect("its host");
    assert_eq!(host.value(SINE, 0), Some(0.5), "the window's sine is there");
}
