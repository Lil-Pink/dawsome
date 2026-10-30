//! The sample editor through the app's messages, against tests/fixtures/gen_p30_expected.py's
//! known answers (p30_expected.json): its window's rules (one of its own, above the app while
//! the app has the keyboard, its place kept, the ways it closes), a selection by drag snapped
//! to the zero crossings the generator finds, zoom and scroll from the layout constants, every
//! op through the window one undo step to the documents the generator writes, the sound shown
//! the bits of `dsp::edits::apply` over the file, Play through the preview, the keys of its
//! window, Save as new file, what it holds of ten minutes, and a player that touches no heap
//! while it edits.

use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::dsp::edits::apply;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::project::{Channel, ChannelId, ClipId, ClipItem, Project, SoundEdits, SoundOp};
use dawsome_engine::realtime::Player;
use dawsome_media::load_channel_sample;
use iced::keyboard::key::{Code, Named};
use iced::keyboard::{Key, Modifiers};
use iced::{window, Point, Size};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::super::plugins::app_tests::channel_menu_row;
use super::super::tests::app_hosted;
use super::super::widget_events::Window;
use super::super::{App, FileMessage, KeyInput, Message, PlayMode, WindowChange};
use crate::counting_allocator::{heap_bytes_during, heap_calls_during};
use crate::desktop::PopOut;
use crate::document::Host;
use crate::project_file;
use crate::settings::{Settings, WindowPlace};
use crate::ui::hint::{self, ChannelRow, Hover, HoverMessage, SoundPart};
use crate::ui::instrument_window::WINDOW_SIZE as INSTRUMENT_WINDOW;
use crate::ui::layout::TOOLBAR_HEIGHT;
use crate::ui::panels::Panel;
use crate::ui::playlist_layout::ClipPart;
use crate::ui::playlist_layout::{HEADER_HEIGHT, RULER_HEIGHT as PLAYLIST_RULER};
use crate::ui::sample_editor::{EditorOp, SampleEditorMessage as M, WINDOW_SIZE};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p30_expected.json")).expect("the answers");
    serde_json::from_str(&text).expect("parses")
}

/// An app on `project`, named as the file at `path`, with `host`'s settings, told its main
/// window's id as main tells it.
fn app_of(project: Project, path: &Path, host: Host) -> (App, Player, window::Id) {
    let (mut app, player) = app_hosted(project, path, host);
    let main = window::Id::unique();
    app.set_main_window(main);
    (app, player, main)
}

/// An app on the fixture `name`.
fn app_on(name: &str) -> (App, Player, window::Id) {
    let path = fixtures().join(name);
    let project = project_file::open(&path).expect("opens").project;
    app_of(project, &path, Host::offline())
}

fn send(app: &mut App, message: M) {
    let _ = app.update(Message::SampleEditor(message));
}

fn open(app: &mut App, channel: u32) -> window::Id {
    let _ = app.update(Message::OpenSampleEditor(ChannelId(channel)));
    app.editor.window.expect("a window of its own")
}

fn shown(app: &App) -> (bool, bool) {
    (app.sample_editor.is_some(), app.editor.window.is_some())
}

fn saved(app: &App) -> String {
    app.session.project().to_json().expect("writes")
}

/// The frames the editor shows.
fn view(app: &App) -> Vec<f32> {
    let open = app.sample_editor.as_ref().expect("open");
    open.sound.as_ref().expect("made").samples().to_vec()
}

/// What the channel the editor is on plays: its file at the project's rate through its list,
/// as `dsp::edits::apply` makes it (its bits are gen_p30's, P1a's tests).
fn played(app: &App) -> SampleBuffer {
    let channel = app.sample_editor_on().expect("open");
    let shown = &app.session.project().channels[&channel];
    let file = load_channel_sample(
        app.session.samples_dir(),
        &shown.sample,
        app.session.project().sample_rate,
    );
    apply(&file.buffer, &shown.edits)
}

fn selection(app: &App) -> Option<(u64, u64)> {
    let open = app.sample_editor.as_ref()?;
    open.selection.map(|part| (part.from, part.to))
}

fn sha(values: &[f32]) -> String {
    let mut hash = Sha256::new();
    for value in values {
        hash.update(value.to_le_bytes());
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn op_of(value: &Value) -> SoundOp {
    serde_json::from_value(value.clone()).expect("an op")
}

/// A key pressed in `window`.
fn key(app: &mut App, window: window::Id, key: Key, code: Option<Code>, modifiers: Modifiers) {
    let _ = app.update(Message::Key(KeyInput {
        window,
        key,
        code,
        modifiers,
        pressed: true,
        repeat: false,
    }));
}

/// A temp folder of this test's own.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dawsome-sample-editor-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("a temp folder");
    dir
}

// The editor is a window of its own, one at a time, titled "<channel> · <file> — Sample
// editor": on p30_undo's sampler "Ramp ·
// ramp48000.wav", then opened on the audio channel the same window, retitled "Long ·
// ramp72000.wav"; a channel playing no file opens nothing and says so. Its Close, Esc in it
// (the selection first, then the window), its X and the system closing it close it alone; Esc
// in the main window closes it after a menu; the main window's X closes the app with it.
#[test]
fn the_sample_editor_is_one_window_of_its_own() {
    let path = fixtures().join("p30_undo.json");
    let mut project = project_file::open(&path).expect("opens").project;
    project
        .channels
        .insert(ChannelId(3), Channel::sampler("Empty", ""));
    project.next_channel_id = ChannelId(4);
    let (mut app, _player, main) = app_of(project, &path, Host::offline());
    assert_eq!(shown(&app), (false, false));
    let id = open(&mut app, 1);
    assert_ne!(Some(id), app.main_window);
    assert_eq!(
        app.window_title(id),
        "Ramp \u{b7} ramp48000.wav \u{2014} Sample editor"
    );
    assert_eq!(app.window_title(main), app.title());
    assert_eq!(view(&app).len(), 48_000, "its sound, made");
    assert_eq!(open(&mut app, 2), id, "the same window");
    assert_eq!(
        app.window_title(id),
        "Long \u{b7} ramp72000.wav \u{2014} Sample editor"
    );
    assert_eq!(app.sample_editor_on(), Some(ChannelId(2)));
    assert_eq!(view(&app).len(), 72_000);
    let _ = app.update(Message::OpenSampleEditor(ChannelId(3)));
    assert_eq!(
        app.sample_editor_on(),
        Some(ChannelId(2)),
        "no file: not opened"
    );
    assert_eq!(
        app.notice.as_deref(),
        Some("Empty plays no sound file to edit")
    );

    send(&mut app, M::Close);
    assert_eq!(shown(&app), (false, false), "its Close");
    let id = open(&mut app, 1);
    send(&mut app, M::Select { from: 10, to: 20 });
    let _ = app.update(Message::Escape(id));
    assert_eq!(shown(&app), (true, true), "Esc: the selection first");
    assert_eq!(selection(&app), None);
    let _ = app.update(Message::Escape(id));
    assert_eq!(shown(&app), (false, false), "then the window");

    let id = open(&mut app, 1);
    let _ = app.update(Message::ToggleEditMenu);
    let _ = app.update(Message::Escape(main));
    assert_eq!(
        app.editor.window,
        Some(id),
        "Esc in the main window: its menu"
    );
    let _ = app.update(Message::Escape(main));
    assert_eq!(shown(&app), (false, false), "then it");

    let id = open(&mut app, 1);
    let _ = app.update(Message::WindowCloseRequested(id));
    assert_eq!(shown(&app), (false, false), "its X");
    assert!(!app.exiting(), "closes it alone");
    let id = open(&mut app, 1);
    let _ = app.update(Message::WindowClosed(id));
    assert_eq!(shown(&app), (false, false), "the system closing it");
    let _ = app.update(Message::WindowCloseRequested(id));
    assert!(!app.exiting(), "a request from a window gone asks nothing");

    let _ = open(&mut app, 1);
    let _ = app.update(Message::WindowCloseRequested(main));
    assert!(app.exiting(), "the main window's X closes the app");
}

// It floats above the main window while one of the app's windows has the keyboard, its own
// too, and is stacked as any window once another program has it.
#[test]
fn the_sample_editor_stays_above_while_the_app_has_the_keyboard() {
    let (mut app, _player, main) = app_on("p30_undo.json");
    let focus = |app: &mut App, id, focused| {
        let _ = app.update(Message::Windowed(id, WindowChange::Focus(focused)));
    };
    focus(&mut app, main, true);
    let id = open(&mut app, 1);
    assert!(app.editor.above, "opened above");
    focus(&mut app, main, false);
    focus(&mut app, id, true);
    assert!(app.editor.above, "its own keyboard: still above");
    focus(&mut app, id, false);
    assert!(!app.editor.above, "another program has it");
    focus(&mut app, main, true);
    assert!(app.editor.above);
}

// Its size and place are kept in the settings when it closes and when the app closes with it
// open, and it opens there again in a later start; its first size is 900 x 440. Snap to zero
// is on until switched off, written only when off.
#[test]
fn the_sample_editors_place_and_snap_are_kept_in_the_settings() {
    let dir = temp_dir("place");
    let file = dir.join("dawsome/settings.json");
    let mut host = Host::offline();
    host.settings_file = Some(file.clone());
    host.saves_settings = true;
    let path = fixtures().join("p30_undo.json");
    let project = project_file::open(&path).expect("opens").project;
    let (mut app, _player, main) = app_of(project.clone(), &path, host.clone());
    assert_eq!(app.editor.place.size(PopOut::SampleEditor), WINDOW_SIZE);
    assert_eq!(WINDOW_SIZE, Size::new(900.0, 440.0));
    let id = open(&mut app, 1);
    let changed = |app: &mut App, change| {
        let _ = app.update(Message::Windowed(id, change));
    };
    changed(&mut app, WindowChange::Resized(Size::new(700.0, 500.0)));
    changed(&mut app, WindowChange::Moved(Point::new(30.0, 40.0)));
    let _ = app.update(Message::Escape(id));
    assert_eq!(
        Settings::load(Some(&file)).sample_editor_window,
        Some(WindowPlace::of(
            Size::new(700.0, 500.0),
            Some(Point::new(30.0, 40.0))
        ))
    );
    assert!(app.settings.snap_to_zero(), "on at first");
    let id = open(&mut app, 1);
    send(&mut app, M::Snap);
    let text = fs::read_to_string(&file).expect("saved");
    assert!(text.contains(r#""snap_to_zero": false"#), "{text}");
    send(&mut app, M::Snap);
    let text = fs::read_to_string(&file).expect("saved");
    assert!(!text.contains("snap_to_zero"), "{text}");
    let _ = app.update(Message::Windowed(
        id,
        WindowChange::Resized(Size::new(640.0, 360.0)),
    ));
    let _ = app.update(Message::WindowCloseRequested(main));
    assert!(app.exiting());
    let (later, _player, _main) = app_of(project, &path, host);
    assert_eq!(
        later.editor.place.size(PopOut::SampleEditor),
        Size::new(640.0, 360.0)
    );
    assert_eq!(later.editor.place.position(), Some(Point::new(30.0, 40.0)));
    fs::remove_dir_all(&dir).expect("removed");
}

// It closes when its channel goes (the sampler deleted; the audio channel's delete is refused
// while it has clips) and when another document comes (File → New).
#[test]
fn the_sample_editor_closes_when_its_channel_goes() {
    let (mut app, _player, _main) = app_on("p30_undo.json");
    let _ = open(&mut app, 1);
    let _ = app.update(Message::Channel(crate::app::ChannelCommand::Delete(
        ChannelId(1),
    )));
    assert_eq!(
        shown(&app),
        (false, false),
        "its channel deleted: {:?}",
        app.error
    );
    let _ = app.update(Message::Undo);
    let _ = open(&mut app, 1);
    let _ = app.update(Message::File(FileMessage::New));
    assert_eq!(shown(&app), (false, false), "another document");
}

// gen_p30's "editor" crop: frames 1000 to 2000 selected on p30_undo's audio channel and
// cropped through the window is one undo step ("Crop") to the document the generator writes:
// the channel's list that one crop, its clip at 48000 moved to 1000 and the stretched one
// (30000 of 108000) to 1500 of 1500 frames. The window then shows 1000 frames, the bits of
// `apply` over the file, whose SHA-256 the generator states; undo gives the file's bytes and
// frames back exactly, redo the crop's. On the sampler, the same crop moves nothing.
#[test]
fn a_selection_cropped_through_the_window_is_one_step_moving_the_clips() {
    let want = expected();
    for channel in ["2", "1"] {
        let case = &want["editor"]["crop"][channel];
        let (mut app, _player, _main) = app_on(case["fixture"].as_str().expect("fixture"));
        let before = saved(&app);
        let id: u32 = channel.parse().expect("an id");
        let _ = open(&mut app, id);
        let file = view(&app);
        let range = case["selection"].as_array().expect("a selection");
        let (from, to) = (
            range[0].as_u64().expect("from"),
            range[1].as_u64().expect("to"),
        );
        send(&mut app, M::Select { from, to });
        send(&mut app, M::Op(EditorOp::Crop));
        assert_eq!(
            saved(&app),
            case["after"].as_str().expect("after"),
            "{channel}"
        );
        assert_eq!(app.session.history().undo_len(), 1, "one step");
        assert_eq!(app.session.history().undo_name(), Some("Crop"));
        let shown = &app.session.project().channels[&ChannelId(id)];
        assert_eq!(shown.edits.ops, [op_of(&case["op"])]);
        let offsets: Vec<u64> = app
            .session
            .project()
            .playlist
            .clips
            .values()
            .filter_map(|clip| match clip.item {
                ClipItem::Audio {
                    channel,
                    frame_offset,
                    ..
                } if channel == ChannelId(id) => Some(frame_offset),
                _ => None,
            })
            .collect();
        let want_offsets: Vec<u64> = case["frame_offsets"]
            .as_array()
            .expect("offsets")
            .iter()
            .map(|offset| offset.as_u64().expect("an offset"))
            .collect();
        assert_eq!(offsets, want_offsets, "{channel}");
        let cropped = view(&app);
        assert_eq!(
            cropped.len() as u64,
            case["frames"].as_u64().expect("frames")
        );
        assert_eq!(sha(&cropped), case["sha256"].as_str().expect("sha256"));
        assert_eq!(cropped, played(&app).samples(), "the bits of apply");
        assert_eq!(selection(&app), None, "a crop leaves no selection");
        let _ = app.update(Message::Undo);
        assert_eq!(saved(&app), before, "undone: the bytes as before");
        assert_eq!(view(&app), file, "and the file's frames");
        let _ = app.update(Message::Redo);
        assert_eq!(saved(&app), case["after"].as_str().expect("after"));
        assert_eq!(view(&app), cropped);
    }
}

// gen_p30's "undo" steps, each through the window's own controls: a selection (or none, for an
// op over the whole sound), then the op's button (Gain by its readout's drag, 10 hundredths of
// a dB a pixel: -300 is 30 px down, +600 60 px up). Each is one undo step, named as the
// generator names it, to the document it writes; the window shows the bits of `apply` after
// each. Undone one by one, each document before comes back byte for byte, the frames with it;
// redone, the last.
#[test]
fn every_op_through_the_window_is_one_undo_step_to_the_generators_document() {
    let want = expected();
    let undo = &want["undo"];
    let (mut app, _player, _main) = app_on(undo["fixture"].as_str().expect("fixture"));
    assert_eq!(
        want["editor"]["gain_per_pixel"].as_f64(),
        Some(f64::from(super::GAIN_PER_PIXEL))
    );
    let mut documents = vec![saved(&app)];
    let steps = undo["steps"].as_array().expect("steps");
    for (index, step) in steps.iter().enumerate() {
        let channel = step["channel"].as_u64().expect("channel") as u32;
        let id = open(&mut app, channel);
        let op = op_of(&step["op"]);
        let frames = step["frames"].as_u64().expect("frames");
        assert_eq!(
            view(&app).len() as u64,
            frames,
            "step {index}: the sound before"
        );
        let (from, to) = op.span();
        if (from, to) == (0, frames) {
            if selection(&app).is_some() {
                let _ = app.update(Message::Escape(id));
            }
        } else {
            send(&mut app, M::Select { from, to });
            assert_eq!(selection(&app), Some((from, to)));
        }
        match op {
            SoundOp::Gain { centi_db, .. } => {
                send(&mut app, M::GainPressed);
                send(&mut app, M::GainMoved(300.0));
                send(&mut app, M::GainMoved(300.0 - centi_db as f32 / 10.0));
                assert_eq!(
                    app.sample_editor.as_ref().map(|open| open.gain),
                    Some(centi_db)
                );
                send(&mut app, M::GainReleased);
            }
            other => {
                let button = match other {
                    SoundOp::Crop { .. } => EditorOp::Crop,
                    SoundOp::Delete { .. } => EditorOp::Delete,
                    SoundOp::Silence { .. } => EditorOp::Silence,
                    SoundOp::FadeIn { .. } => EditorOp::FadeIn,
                    SoundOp::FadeOut { .. } => EditorOp::FadeOut,
                    SoundOp::Normalize { .. } => EditorOp::Normalize,
                    SoundOp::Reverse { .. } => EditorOp::Reverse,
                    SoundOp::Gain { .. } => unreachable!(),
                };
                send(&mut app, M::Op(button));
            }
        }
        let after = step["after"].as_str().expect("after");
        assert_eq!(saved(&app), after, "step {index}: {op:?}");
        assert_eq!(app.session.history().undo_len(), index + 1, "one step each");
        assert_eq!(
            app.session.history().undo_name(),
            step["name"].as_str(),
            "step {index}"
        );
        assert_eq!(
            view(&app),
            played(&app).samples(),
            "step {index}: the bits of apply"
        );
        documents.push(after.to_owned());
    }
    for index in (0..steps.len()).rev() {
        let _ = app.update(Message::Undo);
        assert_eq!(saved(&app), documents[index], "undone to {index}");
        assert_eq!(
            view(&app),
            played(&app).samples(),
            "undone to {index}: the frames"
        );
    }
    for _ in steps {
        let _ = app.update(Message::Redo);
    }
    assert_eq!(&saved(&app), documents.last().expect("the last"));
}

/// A sampler on a sound of `values` (mono, 44.1 kHz) written to `dir`, in p30_undo's project.
fn app_on_values(dir: &Path, values: &[f32]) -> (App, Player, window::Id) {
    let file = dir.join("sound.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 44_100,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&file, spec).expect("a WAV");
    for &value in values {
        writer.write_sample(value).expect("written");
    }
    writer.finalize().expect("finished");
    let path = fixtures().join("p30_undo.json");
    let mut project = project_file::open(&path).expect("opens").project;
    let sampler = project
        .channels
        .get_mut(&ChannelId(1))
        .expect("the sampler");
    sampler.sample = file;
    sampler.edits = SoundEdits::default();
    app_of(project, &path, Host::offline())
}

// A drag selects between its press and its release, each edge snapped to the nearest zero
// crossing within 10 ms (441 frames at 44.1 kHz) while Snap to zero is on: gen_p30's triangle
// (3, 2, 1, 0, -1, ... quartered; zeros at 3, 9 and 15), 18 frames at 16 px a frame (the
// nearest zoom: they fill 288 of the 876 px), so x 112 is frame 7 and x 80 frame 5. Pressed at 7
// and let go of at 5: 3 to 9, as the generator snaps 7 to 9 and 5 to 3. Snap off: 5 to 7. A
// click puts the cursor there and selects nothing; a second click within 400 ms selects it
// all; Shift+click at frame 15 stretches 5..7 to 5..15 (its far edge kept).
#[test]
fn a_drag_selects_and_snaps_its_edges_to_the_zero_crossings() {
    let want = expected();
    let cases = want["zero_crossings"].as_array().expect("cases");
    let triangle = cases
        .iter()
        .find(|case| case["sound"] == "triangle" && case["reach"] == 441)
        .expect("the triangle");
    let values: Vec<f32> = triangle["values"]
        .as_array()
        .expect("values")
        .iter()
        .map(|value| value.as_f64().expect("a value") as f32)
        .collect();
    let snapped = |press: u64| {
        cases
            .iter()
            .find(|case| {
                case["sound"] == "triangle" && case["press"] == press && case["reach"] == 441
            })
            .and_then(|case| case["snap"].as_u64())
            .expect("a case")
    };
    let dir = temp_dir("snap");
    let (mut app, _player, _main) = app_on_values(&dir, &values);
    let _ = open(&mut app, 1);
    let view = app.sample_editor.as_ref().expect("open").view;
    assert_eq!(
        (view.first, view.per_px, view.width),
        (0.0, 1.0 / 16.0, 876.0)
    );
    let drag = |app: &mut App, from: f32, to: f32| {
        send(app, M::Pressed(from));
        send(app, M::Dragged((from + to) / 2.0));
        send(app, M::Dragged(to));
        send(app, M::Released(to));
    };
    drag(&mut app, 112.0, 80.0);
    assert_eq!(selection(&app), Some((snapped(5), snapped(7))));
    assert_eq!(selection(&app), Some((3, 9)));
    send(&mut app, M::Snap);
    drag(&mut app, 112.0, 80.0);
    assert_eq!(selection(&app), Some((5, 7)), "snap off");
    send(&mut app, M::Pressed(112.0));
    send(&mut app, M::Released(112.0));
    assert_eq!(selection(&app), None, "a click selects nothing");
    assert_eq!(app.sample_editor.as_ref().map(|open| open.cursor), Some(7));
    send(&mut app, M::Pressed(112.0));
    send(&mut app, M::Released(112.0));
    assert_eq!(selection(&app), Some((0, 18)), "a double-click: all of it");
    drag(&mut app, 80.0, 112.0);
    let _ = app.update(Message::ModifiersChanged(Modifiers::SHIFT));
    send(&mut app, M::Pressed(240.0));
    send(&mut app, M::Released(240.0));
    assert_eq!(selection(&app), Some((5, 15)), "Shift+click stretches it");
    let _ = fs::remove_dir_all(&dir);
}

// Zoom and scroll through the window, by hand from the layout: the window 824 px wide leaves
// the waveform 800 (12 px of padding a side), so p30_undo's 48000-frame ramp fits at 60 frames
// a pixel. Ctrl+wheel up at x 200: 30 a pixel from 6000 (frame 12000 kept under x 200); the
// wheel down a notch: an eighth of the 24000 shown later, from 9000; Fit: 0 and 60 again;
// frames 1000..2000 selected and zoomed to: 1.25 a pixel from 1000; the scroll bar to 5000.
#[test]
fn the_wheel_zooms_and_scrolls_the_waveform() {
    let (mut app, _player, _main) = app_on("p30_undo.json");
    let id = open(&mut app, 1);
    let _ = app.update(Message::Windowed(
        id,
        WindowChange::Resized(Size::new(824.0, 440.0)),
    ));
    let at = |app: &App| {
        let view = app.sample_editor.as_ref().expect("open").view;
        (view.first, view.per_px)
    };
    assert_eq!(at(&app), (0.0, 60.0));
    let _ = app.update(Message::ModifiersChanged(Modifiers::CTRL));
    send(
        &mut app,
        M::Wheel {
            x: 200.0,
            across: 0.0,
            up: 1.0,
        },
    );
    assert_eq!(at(&app), (6000.0, 30.0));
    let _ = app.update(Message::ModifiersChanged(Modifiers::empty()));
    send(
        &mut app,
        M::Wheel {
            x: 200.0,
            across: 0.0,
            up: -1.0,
        },
    );
    assert_eq!(at(&app), (9000.0, 30.0));
    send(&mut app, M::Fit);
    assert_eq!(at(&app), (0.0, 60.0));
    send(
        &mut app,
        M::Select {
            from: 1000,
            to: 2000,
        },
    );
    send(&mut app, M::ZoomToSelection);
    assert_eq!(at(&app), (1000.0, 1.25));
    send(
        &mut app,
        M::Bar(crate::ui::scroll::BarMessage::To {
            offset: 5000.0,
            held: true,
        }),
    );
    assert_eq!(at(&app), (5000.0, 1.25));
}

// Play plays the selection alone, as it sounds now, through the Library's preview, the song
// stopped: frames 1000..2000 of the ramp (frame k = k/65536) on both sides, then silence; Stop
// stops it. Space in the editor's window plays it too.
#[test]
fn play_plays_the_selection_alone_through_the_preview() {
    let (mut app, mut player, _main) = app_on("p30_undo.json");
    let id = open(&mut app, 1);
    send(
        &mut app,
        M::Select {
            from: 1000,
            to: 2000,
        },
    );
    send(&mut app, M::Play);
    let mut out = vec![[9.0f32; 2]; 1200];
    player.process(&mut out);
    let want: Vec<[f32; 2]> = (0..1200)
        .map(|k| {
            let value = if k < 1000 {
                (1000 + k) as f32 / 65536.0
            } else {
                0.0
            };
            [value, value]
        })
        .collect();
    assert_eq!(out, want);
    send(
        &mut app,
        M::Select {
            from: 0,
            to: 48_000,
        },
    );
    send(&mut app, M::Play);
    player.process(&mut out[..10]);
    assert_ne!(out[5], [0.0; 2], "playing");
    send(&mut app, M::Stop);
    player.process(&mut out);
    assert!(out.iter().all(|&frame| frame == [0.0; 2]), "stopped");
    key(
        &mut app,
        id,
        Key::Named(Named::Space),
        Some(Code::Space),
        Modifiers::empty(),
    );
    player.process(&mut out[..4]);
    assert_eq!(out[1], [1.0 / 65536.0; 2], "Space plays it from its start");
    assert!(!app.transport_on, "the song stays stopped");
}

// The window's own keys: Ctrl+A selects the whole sound, Delete deletes the selection (one
// step, "Delete part"); the same Delete in the main window is the main window's.
#[test]
fn delete_and_select_all_act_on_the_editor_in_its_window() {
    let (mut app, _player, main) = app_on("p30_undo.json");
    let id = open(&mut app, 1);
    key(
        &mut app,
        id,
        Key::Character("a".into()),
        Some(Code::KeyA),
        Modifiers::CTRL,
    );
    assert_eq!(selection(&app), Some((0, 48_000)));
    send(&mut app, M::Select { from: 100, to: 200 });
    key(
        &mut app,
        main,
        Key::Named(Named::Delete),
        None,
        Modifiers::empty(),
    );
    assert_eq!(
        app.session.history().undo_len(),
        0,
        "the main window's Delete"
    );
    key(
        &mut app,
        id,
        Key::Named(Named::Delete),
        None,
        Modifiers::empty(),
    );
    assert_eq!(app.session.history().undo_name(), Some("Delete part"));
    assert_eq!(
        app.session.project().channels[&ChannelId(1)].edits.ops,
        [SoundOp::Delete { from: 100, to: 200 }]
    );
    assert_eq!(view(&app).len(), 47_900);
}

/// A file's SHA-256 and time: what "never written" is checked by.
fn identity(path: &Path) -> (String, std::time::SystemTime) {
    let bytes = fs::read(path).expect("reads");
    let hash = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let time = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .expect("a time");
    (hash, time)
}

/// One bar of `project` rendered by the window's own `--play-to` path, as its WAV's bytes.
fn rendered(project: &Path, out: &Path) -> Vec<u8> {
    let request = crate::play_to::PlayTo {
        out: out.to_path_buf(),
        bars: 1,
        song: true,
        bits: None,
        record: None,
        record_notes: false,
        quantize: true,
        count_in: true,
        save_to: None,
        stream_threshold: None,
        midi_events: None,
        cache_dir: None,
        dump_notes_at: None,
        midi_out_log: None,
    };
    crate::play_to::run(project, &request, None).expect("renders");
    fs::read(out).expect("written")
}

// Save as new file: a copy of p30_undo beside its sounds; the audio channel's sound cropped to
// 1000..2000 and faded in, saved: "samples/ramp72000 (edited).wav" beside the project (where
// takes go), a 32-bit float mono WAV at 44.1 kHz holding exactly the frames shown, the notice
// naming it; saved again: "(edited 2)". The sound files' bytes and times never change, through
// every op and save. The project with its edits renders (song, a bar) the bytes of the same
// project playing the saved file with no edits. A file the project reads is refused as a
// target, and stays as it was.
#[test]
fn save_as_new_file_writes_the_sound_as_it_plays() {
    let dir = temp_dir("save");
    fs::create_dir_all(dir.join("p30_sound")).expect("a folder");
    for sound in ["ramp48000.wav", "ramp72000.wav"] {
        fs::copy(
            fixtures().join("p30_sound").join(sound),
            dir.join("p30_sound").join(sound),
        )
        .expect("copied");
    }
    let path = dir.join("p30_undo.json");
    fs::copy(fixtures().join("p30_undo.json"), &path).expect("copied");
    let project = project_file::open(&path).expect("opens").project;
    let (mut app, _player, _main) = app_of(project, &path, Host::offline());
    let sounds = [
        dir.join("p30_sound/ramp48000.wav"),
        dir.join("p30_sound/ramp72000.wav"),
    ];
    let before: Vec<_> = sounds.iter().map(|sound| identity(sound)).collect();
    let unchanged = |what: &str| {
        let now: Vec<_> = sounds.iter().map(|sound| identity(sound)).collect();
        assert_eq!(now, before, "a sound file was written: {what}");
    };
    let _ = open(&mut app, 2);
    send(
        &mut app,
        M::Select {
            from: 1000,
            to: 2000,
        },
    );
    send(&mut app, M::Op(EditorOp::Crop));
    unchanged("crop");
    send(&mut app, M::Select { from: 0, to: 300 });
    send(&mut app, M::Op(EditorOp::FadeIn));
    unchanged("fade in");
    send(&mut app, M::SaveAs);
    let saved_file = dir.join("samples/ramp72000 (edited).wav");
    assert!(saved_file.is_file(), "{:?}", app.error);
    assert_eq!(
        app.notice.as_deref(),
        Some(
            format!(
                "Saved ramp72000 (edited).wav in {}",
                dir.join("samples").display()
            )
            .as_str()
        )
    );
    unchanged("save");
    let mut reader = hound::WavReader::open(&saved_file).expect("a WAV");
    let spec = reader.spec();
    assert_eq!(
        (spec.channels, spec.sample_rate, spec.bits_per_sample),
        (1, 44_100, 32)
    );
    assert_eq!(spec.sample_format, hound::SampleFormat::Float);
    let written: Vec<f32> = reader
        .samples::<f32>()
        .collect::<Result<_, _>>()
        .expect("decodes");
    assert_eq!(written, view(&app), "exactly the frames shown");
    assert_eq!(written.len(), 1000);
    send(&mut app, M::SaveAs);
    assert!(dir.join("samples/ramp72000 (edited 2).wav").is_file());
    unchanged("save again");

    let edited = dir.join("edited.json");
    fs::write(&edited, saved(&app) + "\n").expect("written");
    let mut swapped = app.session.project().clone();
    let channel = swapped
        .channels
        .get_mut(&ChannelId(2))
        .expect("the audio channel");
    channel.sample = PathBuf::from("samples/ramp72000 (edited).wav");
    channel.edits = SoundEdits::default();
    let plain = dir.join("plain.json");
    fs::write(&plain, swapped.to_json().expect("writes") + "\n").expect("written");
    let with_edits = rendered(&edited, &dir.join("edited.wav"));
    let with_file = rendered(&plain, &dir.join("plain.wav"));
    assert_eq!(
        with_edits, with_file,
        "the saved file plays as the edits do"
    );
    assert!(with_edits.len() > 44);
    unchanged("renders");

    let input = dir.join("p30_sound/ramp72000.wav");
    let refused = app
        .save_job(input.clone(), dir.join("p30_sound"))
        .map(|_| ())
        .expect_err("an input file");
    assert!(refused.contains("input file"), "{refused}");
    unchanged("a refused save");
    let _ = fs::remove_dir_all(&dir);
}

/// Frame `k` of gen_p30's ten-minute sound ("long"): a sawtooth of `period` frames, its right
/// side its left halved and negated.
fn long_frame(k: u64, period: u64) -> [f32; 2] {
    let left = ((k % period) as f64 - (period / 2) as f64) / period as f64;
    [left as f32, (-left / 2.0) as f32]
}

// The spec's 10x bound: the editor holds one buffer of the sound and its peaks. Ten minutes of
// stereo at 48 kHz (gen_p30's "long", its two ops), opened in the window: what it holds is
// 230,400,000 bytes of frames and 1,800,000 of peaks (the generator's figures), and what
// opening it left on the heap is that and under 64 KiB more. One op more (Gain over the whole
// sound) is applied where the frames are: it holds the same, and the op takes nothing more from
// the heap. The most opening it held on the way is printed, for docs/performance.md.
/// gen_p30's ten-minute sound ("long") written into the temp folder `name`, beside a copy of
/// p30_long.json with its clip taken out (the session leaves the sound to the editor: a clip
/// would make it for playback), and an app on that project: the app, its player, the folder
/// and the sound's frames.
fn ten_minutes(name: &str) -> (App, Player, PathBuf, u64) {
    let frames = expected()["long"]["frames"].as_u64().expect("frames");
    long_sound(name, frames)
}

/// As [`ten_minutes`], the sound's first `frames` frames.
fn long_sound(name: &str, frames: u64) -> (App, Player, PathBuf, u64) {
    let want = expected();
    let long = &want["long"];
    let dir = temp_dir(name);
    let period = long["sawtooth"].as_u64().expect("period");
    let rate = long["rate"].as_u64().expect("rate") as u32;
    let sound = dir.join(long["sound"].as_str().expect("sound"));
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&sound, spec).expect("a WAV");
    for k in 0..frames {
        for value in long_frame(k, period) {
            writer.write_sample(value).expect("written");
        }
    }
    writer.finalize().expect("finished");
    let path = dir.join("p30_long.json");
    fs::copy(fixtures().join("p30_long.json"), &path).expect("copied");
    let mut project = project_file::open(&path).expect("opens").project;
    project.playlist.clips.clear();
    let (app, player, _main) = app_of(project, &path, Host::offline());
    (app, player, dir, frames)
}

#[test]
fn the_editor_holds_one_buffer_and_its_peaks_of_ten_minutes() {
    let want = expected();
    let held_answer = &want["editor"]["long"];
    let (mut app, _player, dir, frames) = ten_minutes("long");
    let (held, most) = heap_bytes_during(|| {
        let _ = app.update(Message::OpenSampleEditor(ChannelId(1)));
    });
    let open = app.sample_editor.as_ref().expect("open");
    let sound = open.sound.as_ref().expect("made");
    assert_eq!(sound.frames(), frames);
    let buffer = held_answer["buffer_bytes"].as_u64().expect("bytes") as usize;
    let peaks = held_answer["peaks_bytes"].as_u64().expect("bytes") as usize;
    assert_eq!(
        sound.held_bytes(),
        buffer + peaks,
        "one buffer and its peaks"
    );
    let held = usize::try_from(held).expect("held");
    assert!(
        (buffer + peaks..buffer + peaks + 65_536).contains(&held),
        "opening left {held} bytes held"
    );
    println!("ten minutes in the editor: {held} bytes held, {most} at the most while opening");
    let (op_held, op_most) = heap_bytes_during(|| {
        let open = app.sample_editor.as_mut().expect("open");
        let sound = open.sound.as_mut().expect("made");
        sound.append(SoundOp::Gain {
            from: 0,
            to: frames,
            centi_db: 300,
        });
    });
    let sound = app
        .sample_editor
        .as_ref()
        .and_then(|open| open.sound.as_ref());
    assert_eq!(sound.map(|sound| sound.held_bytes()), Some(buffer + peaks));
    assert!(
        op_most < 65_536,
        "an op in place took {op_most} bytes ({op_held} held)"
    );
    let _ = fs::remove_dir_all(&dir);
}

// P1a's no-heap test through the window: the song playing p30_undo's clips, the editor open,
// a crop of the audio channel's sound, a fade in of the sampler's, an undo and Play of a
// selection, each between two of the player's calls; the player's 300 calls touch the heap not
// once, and every edit sent a plan.
#[test]
fn the_player_touches_no_heap_while_the_editor_edits() {
    let control = heap_calls_during(|| drop(std::hint::black_box(vec![0u8; 64])));
    assert!(control > 0, "the counting allocator is not counting");
    let (mut app, mut player, _main) = app_on("p30_undo.json");
    let _ = app.update(Message::ModeSelected(PlayMode::Song));
    let _ = app.update(Message::Play);
    let _ = open(&mut app, 2);
    let plans = app.session.plans_sent();
    let mut out = vec![[0.0f32; 2]; 512];
    let mut calls = 0;
    for call in 0..300 {
        match call {
            30 => {
                send(
                    &mut app,
                    M::Select {
                        from: 1000,
                        to: 40_000,
                    },
                );
                send(&mut app, M::Op(EditorOp::Crop));
            }
            90 => {
                let _ = open(&mut app, 1);
                send(&mut app, M::Select { from: 0, to: 4800 });
                send(&mut app, M::Op(EditorOp::FadeIn));
            }
            150 => {
                let _ = app.update(Message::Undo);
            }
            210 => send(&mut app, M::Play),
            _ => {}
        }
        app.session.flush();
        calls += heap_calls_during(|| player.process(&mut out));
    }
    assert_eq!(calls, 0, "heap calls in Player::process");
    assert!(
        app.session.plans_sent() >= plans + 3,
        "every edit sent a plan"
    );
    assert_eq!(app.session.history().undo_len(), 1);
    assert_eq!(selection(&app), Some((0, 4800)), "Play kept the selection");
}

/// The point of `points` where the cursor, moved there through `window`'s widgets, comes over
/// `target` (its control says so to the hint bar): how a test finds a button no layout
/// constant places.
fn find_hover(
    window: &mut Window,
    app: &mut App,
    target: &Hover,
    points: impl IntoIterator<Item = Point>,
) -> Option<Point> {
    points.into_iter().find(|&at| {
        let moved = window.mouse(
            app,
            iced::mouse::Event::CursorMoved { position: at },
            iced::mouse::Cursor::Available(at),
        );
        moved.iter().any(
            |message| matches!(message, Message::Hover(HoverMessage::Over(over)) if over == target),
        )
    })
}

/// The top of the playlist's tracks in the main window: under the toolbar, the playlist's
/// header and its ruler.
const TRACKS_TOP: f32 = TOOLBAR_HEIGHT + HEADER_HEIGHT + PLAYLIST_RULER;

// The doors, through the widgets a press reaches. p30_undo with the playlist alone in a 1000 x
// 560 window (as the drop tests lay it out: a bar 64 px from x 120, a row every 48 px, the
// clip's title the top 10 px of its lane): a double-click on clip 1's body (track 1, x 120..184)
// opens the editor on its channel, Long, and the clip's properties strip beside it (the spec's
// default: both). Its window, a bar at 120 BPM and 44.1 kHz (88200 frames) from frame 48000,
// reaches past the 72000-frame sound: 48000..72000 is bracketed and shown across the window
// (876 px). A wheel turn up over the waveform, through the editor's own widgets, scrolls it an
// eighth earlier: 45000. The stretched clip 2 (track 2, its 108000 frames from 30000 over the
// sound's 72000) retargets the bracket to 20000..72000. Edit sample… in a channel's
// right-click menu (rows of a sampler with a file and of the audio channel, none for a sampler
// without one) and in the instrument window's footer (found by the hint its button gives) open
// it on that channel with no bracket. The clip's hint names the editor.
#[test]
fn the_doors_open_the_editor_from_a_clip_the_rack_menu_and_the_instrument_window() {
    let path = fixtures().join("p30_undo.json");
    let mut project = project_file::open(&path).expect("opens").project;
    project
        .channels
        .insert(ChannelId(3), Channel::sampler("Empty", ""));
    project.next_channel_id = ChannelId(4);
    let (mut app, _player, _main) = app_of(project, &path, Host::offline());
    let _ = app.update(Message::TogglePanel(Panel::Playlist));
    let _ = app.update(Message::TogglePanel(Panel::Rack));
    let _ = app.update(Message::WindowResized(Size::new(1000.0, 560.0)));
    app.hints_flag = Some(false);
    let mut window = Window::new(&app);
    let _ = window.mouse(
        &mut app,
        iced::mouse::Event::CursorLeft,
        iced::mouse::Cursor::Unavailable,
    );
    assert_eq!(app.arranger.grid.lane, 47.0);
    let clip_1 = Point::new(150.0, TRACKS_TOP + 30.0);
    let _ = window.click(&mut app, clip_1);
    assert_eq!(shown(&app), (false, false), "one click selects");
    let _ = window.click(&mut app, clip_1);
    assert_eq!(
        app.sample_editor_on(),
        Some(ChannelId(2)),
        "{:?}",
        app.error
    );
    assert!(app.editor.window.is_some());
    assert_eq!(app.arranger.properties, Some(ClipId(1)), "and the strip");
    let bracket = |app: &App| app.clip_window().map(|part| (part.from, part.to));
    assert_eq!(bracket(&app), Some((48_000, 72_000)));
    let view_of = |app: &App| app.sample_editor.as_ref().expect("open").view;
    let shown_view = view_of(&app);
    assert!((shown_view.first - 48_000.0).abs() < 1e-6, "{shown_view:?}");
    assert!(
        (shown_view.shown() - 24_000.0).abs() < 1e-6,
        "{shown_view:?}"
    );
    let editor = app.editor.window.expect("its window");
    let mut widgets = Window::pop_out(&app, editor, WINDOW_SIZE);
    let _ = widgets.wheel(&mut app, Point::new(450.0, 250.0), 1.0);
    assert!(
        (view_of(&app).first - 45_000.0).abs() < 1e-6,
        "{:?}",
        view_of(&app)
    );
    let clip_2 = Point::new(150.0, TRACKS_TOP + 48.0 + 30.0);
    let _ = window.click(&mut app, clip_2);
    let _ = window.click(&mut app, clip_2);
    assert_eq!(app.editor.window, Some(editor), "the same window");
    assert_eq!(bracket(&app), Some((20_000, 72_000)));
    let context = app.hint_context();
    let words = hint::words(
        &Hover::Clip {
            clip: ClipId(1),
            part: ClipPart::Body,
        },
        &context,
    );
    assert!(words.contains("double-click: the sample editor"), "{words}");

    let (mut app, _player, _main) = app_of(
        {
            let mut project = project_file::open(&path).expect("opens").project;
            project
                .channels
                .insert(ChannelId(3), Channel::sampler("Empty", ""));
            project.next_channel_id = ChannelId(4);
            project
        },
        &path,
        Host::offline(),
    );
    for (channel, offered) in [(1, true), (2, true), (3, false)] {
        app.open_channel_menu(ChannelId(channel));
        let rows: Vec<ChannelRow> = app
            .channel_menu_rows()
            .into_iter()
            .map(|row| row.0)
            .collect();
        assert_eq!(rows.contains(&ChannelRow::EditSample), offered, "{channel}");
    }
    let message = channel_menu_row(&mut app, ChannelId(1), ChannelRow::EditSample);
    let _ = app.update(message);
    assert_eq!(
        app.sample_editor_on(),
        Some(ChannelId(1)),
        "the rack's menu"
    );
    assert_eq!(app.clip_window(), None, "no clip bracketed");
    send(&mut app, M::Close);

    let _ = app.update(Message::OpenInstrument(ChannelId(2)));
    let id = app.instrument_window.expect("the instrument window");
    let mut widgets = Window::pop_out(&app, id, INSTRUMENT_WINDOW);
    let right = INSTRUMENT_WINDOW.width - 20.0;
    let footer = find_hover(
        &mut widgets,
        &mut app,
        &Hover::Sound(SoundPart::Close),
        (0..INSTRUMENT_WINDOW.height as u32).map(|y| Point::new(right, y as f32)),
    )
    .expect("the footer's Close");
    let edit = find_hover(
        &mut widgets,
        &mut app,
        &Hover::Sound(SoundPart::EditSample),
        (0..right as u32)
            .rev()
            .map(|x| Point::new(x as f32, footer.y)),
    )
    .expect("Edit sample… left of Close");
    let _ = widgets.click(&mut app, edit);
    assert_eq!(app.sample_editor_on(), Some(ChannelId(2)), "its footer");
    assert_eq!(app.clip_window(), None);
    send(&mut app, M::Close);

    // A scripted run's --open-sample-editor clip:1 with a selection: opened as the clip's
    // double-click opens it, the clip's window shown, the part selected in it.
    let _ = app.open_sample_editor_at_start(crate::args::EditorOn::Clip(1), Some((50_000, 60_000)));
    assert_eq!(app.sample_editor_on(), Some(ChannelId(2)));
    assert_eq!(bracket(&app), Some((48_000, 72_000)));
    assert_eq!(selection(&app), Some((50_000, 60_000)));
    assert!(
        (view_of(&app).first - 48_000.0).abs() < 1e-6,
        "{:?}",
        view_of(&app)
    );
}

// Every reader of a sound's length reads the edited one, against gen_p30's "editor" crop of
// frames 1000 to 2000 on p30_undo's audio channel (1000 frames left, the stretched clip 1500
// frames over them): the playlist's look of the sound (its waveform and the length its clips'
// titles and the strip read) is the peaks of the cropped frames, made at once from the
// editor's own while it shows them, and made from the file through the list when the project
// is opened with the edits; the properties strip of the stretched clip says 1500 / 1000 =
// 150 %, as it said 108000 / 72000 before; Resample chosen on the unstretched clip fits it to
// the cropped 1000 frames, not the file's 72000. Undone, the file's own look is back, 72000.
#[test]
fn every_length_the_playlist_reads_is_the_edited_sounds() {
    use crate::ui::arranger::PlaylistMessage;
    use crate::ui::clips::{self, FileLook};
    use crate::ui::playlist::ClipProperties;
    use dawsome_engine::project::StretchMode;
    use dawsome_media::Peaks;
    let want = expected();
    let case = &want["editor"]["crop"]["2"];
    let frames = case["frames"].as_u64().expect("frames");
    let stretched = case["stretched"][0].as_u64().expect("stretched");
    let (mut app, _player, _main) = app_on("p30_undo.json");
    let look = |app: &App| {
        let channel = &app.session.project().channels[&ChannelId(2)];
        match clips::look_of(&app.looks.files, channel) {
            Some(FileLook::Ready(peaks)) => Some(peaks.clone()),
            _ => None,
        }
    };
    assert_eq!(look(&app).map(|peaks| peaks.frames), Some(72_000));
    let _ = open(&mut app, 2);
    send(
        &mut app,
        M::Select {
            from: 1000,
            to: 2000,
        },
    );
    send(&mut app, M::Op(EditorOp::Crop));
    assert_eq!(saved(&app), case["after"].as_str().expect("after"));
    let cropped = look(&app).expect("made at once from the editor's");
    assert_eq!(cropped.frames, frames);
    assert_eq!(cropped, Peaks::of(&played(&app)), "the peaks of what plays");
    assert_eq!(
        (stretched * 100 / frames, stretched * 100 % frames),
        (150, 0)
    );
    app.arranger.properties = Some(ClipId(2));
    assert_eq!(
        app.clip_properties(),
        Some(ClipProperties {
            mode: Some(StretchMode::Resample),
            pitch: 0,
            percent: Some(150)
        })
    );
    app.arranger.properties = Some(ClipId(1));
    let _ = app.update(Message::Playlist(PlaylistMessage::ClipMode(Some(
        StretchMode::Resample,
    ))));
    let fitted = match app.session.project().playlist.clips[&ClipId(1)].item {
        ClipItem::Audio { stretch, .. } => stretch.map(|stretch| stretch.frames),
        _ => None,
    };
    assert_eq!(fitted, Some(frames), "{:?}", app.notice);
    let _ = app.update(Message::Undo);
    let _ = app.update(Message::Undo);
    assert_eq!(look(&app).map(|peaks| peaks.frames), Some(72_000), "undone");

    // Opened with the edits, no editor: made from the file through the list.
    let project = Project::load_json(case["after"].as_str().expect("after")).expect("loads");
    let (app, _player, _main) = app_of(project, &fixtures().join("p30_undo.json"), Host::offline());
    let made = look(&app).expect("made from the file");
    assert_eq!(made.frames, frames);
    assert_eq!(made, cropped);
}

// gen_p30's "stale": the editor's crop of p30_undo's audio channel, its list's source the
// 72000 frames at 44.1 kHz the crop found. On its own file the editor says nothing; with the
// file swapped for the 48000-frame ramp it says the file changed, and the crop still applies,
// by frame, to what the file holds now (1000 frames, the first 1000/65536), an op more too.
// A list written before sources were kept (p30_crop.json's) never says so.
#[test]
fn a_file_changed_since_its_edits_is_said_and_the_edits_still_apply() {
    let want = expected();
    let stale = &want["stale"];
    let channel = ChannelId(stale["channel"].as_u64().expect("channel") as u32);
    let mut project = Project::load_json(stale["after"].as_str().expect("after")).expect("loads");
    let found = project.channels[&channel].edits.source;
    assert_eq!(
        found.map(|source| (u64::from(source.rate), source.frames)),
        Some((
            stale["source"]["rate"].as_u64().expect("rate"),
            stale["source"]["frames"].as_u64().expect("frames")
        ))
    );
    let path = fixtures().join("p30_undo.json");
    let (mut app, _player, _main) = app_of(project.clone(), &path, Host::offline());
    let _ = open(&mut app, channel.0);
    assert!(!app.editor_file_changed(), "its own file");
    project
        .channels
        .get_mut(&channel)
        .expect("the audio channel")
        .sample = PathBuf::from(stale["sample"].as_str().expect("sample"));
    let (mut app, _player, _main) = app_of(project, &path, Host::offline());
    let _ = open(&mut app, channel.0);
    assert!(app.editor_file_changed(), "another length");
    let shown = view(&app);
    assert_eq!(
        shown.len() as u64,
        stale["frames"].as_u64().expect("frames")
    );
    assert_eq!(
        f64::from(shown[0]),
        stale["first"][0].as_f64().expect("first")
    );
    assert_eq!(shown, played(&app).samples(), "the crop, by frame");
    send(&mut app, M::Select { from: 0, to: 100 });
    send(&mut app, M::Op(EditorOp::Silence));
    assert_eq!(app.session.history().undo_name(), Some("Silence"));
    assert_eq!(view(&app), played(&app).samples());
    assert!(
        app.editor_file_changed(),
        "said until the edits are taken back"
    );
    let (mut app, _player, _main) = app_on("p30_crop.json");
    let _ = open(&mut app, 1);
    assert_eq!(
        app.session.project().channels[&ChannelId(1)].edits.source,
        None
    );
    assert!(!app.editor_file_changed(), "a list that kept nothing");
}

// Save as new file and Play of the whole ten-minute sound (gen_p30's "long", 230,400,000 bytes
// of frames) leave the window's thread at once while the app runs its work in the background:
// each message takes under 64 KiB of that thread's heap (a copy of the frames on it would take
// 230 MB, twice for Play) and under one tick of the editor's timer (33 ms), the editor dim
// meanwhile (no op runs), "Saving…" on its button. The worker's file lands (a 32-bit float
// stereo WAV at 48 kHz of every frame), and the message its work ends with lights the editor
// again and names the file. Play's copy, once made, plays the sound from its first frame; a
// copy let go of by Stop, or one for another Play, is not played.
#[test]
fn saving_and_playing_ten_minutes_leave_the_window_at_once() {
    use super::{Busy, COPY_INLINE_SECONDS};
    use crate::app::editor_sound::part_of;
    use crate::ui::sample_editor::{save_words, Part, SaveDone};
    use std::time::{Duration, Instant};
    const TICK: Duration = Duration::from_millis(33);
    let (mut app, mut player, dir, frames) = ten_minutes("worker");
    let _ = app.update(Message::OpenSampleEditor(ChannelId(1)));
    app.host.background = true;
    let busy = |app: &App| app.sample_editor.as_ref().and_then(|open| open.busy);
    let timed = |app: &mut App, message: M| {
        let started = Instant::now();
        let (_, most) = heap_bytes_during(|| {
            let _ = app.update(Message::SampleEditor(message));
        });
        (started.elapsed(), most)
    };

    let (took, most) = timed(&mut app, M::SaveAs);
    println!("Save as new file: {took:?}, {most} bytes at the most on the window's thread");
    assert!(
        most < 65_536,
        "the save took {most} bytes on the window's thread"
    );
    assert!(took < TICK, "the save held the window {took:?}");
    assert_eq!(busy(&app), Some(Busy::Saving));
    assert_eq!(save_words(true), "Saving\u{2026}");
    assert!(!app.editor_ready(), "the buttons dim");
    send(&mut app, M::Op(EditorOp::Reverse));
    assert_eq!(app.session.history().undo_len(), 0, "no op while it saves");
    let samples = dir.join("samples");
    let target = samples.join("p30_long (edited).wav");
    let deadline = Instant::now() + Duration::from_secs(300);
    while !target.is_file() {
        assert!(Instant::now() < deadline, "the worker's file never landed");
        std::thread::sleep(Duration::from_millis(50));
    }
    let reader = hound::WavReader::open(&target).expect("a WAV");
    let spec = reader.spec();
    assert_eq!(
        (spec.channels, spec.sample_rate, spec.bits_per_sample),
        (2, 48_000, 32)
    );
    assert_eq!(u64::from(reader.duration()), frames);
    drop(reader);
    send(
        &mut app,
        M::Saved(SaveDone {
            target: target.clone(),
            dir: samples.clone(),
            result: Ok(()),
        }),
    );
    assert_eq!(busy(&app), None);
    assert!(app.editor_ready(), "lit again");
    assert_eq!(
        app.notice.as_deref(),
        Some(format!("Saved p30_long (edited).wav in {}", samples.display()).as_str())
    );

    send(&mut app, M::SelectAll);
    assert!(frames > COPY_INLINE_SECONDS * 48_000);
    let (took, most) = timed(&mut app, M::Play);
    println!("Play: {took:?}, {most} bytes at the most on the window's thread");
    assert!(
        most < 65_536,
        "Play took {most} bytes on the window's thread"
    );
    assert!(took < TICK, "Play held the window {took:?}");
    let Some(Busy::Copying {
        token,
        from,
        frames: copied,
    }) = busy(&app)
    else {
        panic!("Play's copy is made on a worker: {:?}", busy(&app));
    };
    assert_eq!((from, copied), (0, frames));
    assert!(!app.editor_ready());
    let sound = view(&app);
    let part = part_of(&sound, 2, 0, frames);
    send(
        &mut app,
        M::Copied {
            token: token + 1,
            part: Some(Part(part.clone())),
        },
    );
    assert!(busy(&app).is_some(), "another Play's copy waits on");
    send(
        &mut app,
        M::Copied {
            token,
            part: Some(Part(part)),
        },
    );
    assert_eq!(busy(&app), None);
    let mut out = vec![[9.0f32; 2]; 4];
    player.process(&mut out);
    let first: Vec<[f32; 2]> = sound
        .chunks(2)
        .take(4)
        .map(|pair| [pair[0], pair[1]])
        .collect();
    assert_eq!(out, first, "plays from its first frame");

    send(&mut app, M::Stop);
    send(
        &mut app,
        M::Select {
            from: 0,
            to: 11 * 48_000,
        },
    );
    let _ = app.update(Message::SampleEditor(M::Play));
    let Some(Busy::Copying { token, .. }) = busy(&app) else {
        panic!("eleven seconds go to a worker: {:?}", busy(&app));
    };
    send(&mut app, M::Stop);
    assert_eq!(busy(&app), None, "Stop lets the copy go");
    let late = part_of(&sound, 2, 0, 11 * 48_000);
    send(
        &mut app,
        M::Copied {
            token,
            part: Some(Part(late)),
        },
    );
    player.process(&mut out);
    assert_eq!(out, [[0.0; 2]; 4], "a copy let go of plays nothing");
    let _ = fs::remove_dir_all(&dir);
}

/// Sends the editor's timer until no sound is being made, as the app's timer does while
/// `editor_ticks` says so.
fn tick_until_made(app: &mut App) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while app.editor_ticks() {
        assert!(
            std::time::Instant::now() < deadline,
            "the sound was never made"
        );
        send(app, M::Tick);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

// A sound made after the editor closed: with the app's work in the background, the editor on
// p30_undo's audio channel closed before its sound was made: the editor's timer runs on until
// the sound lands, which is then let go of (nothing waits in the window: a ten-minute sound
// would be 232 MB held), and the timer stops. Opened, closed and opened again while its sound
// is made, the editor waits for that one: one build, whose 72000 frames it then shows.
#[test]
fn a_sound_made_after_the_editor_closed_is_let_go_of() {
    use std::time::{Duration, Instant};
    let (mut app, _player, _main) = app_on("p30_undo.json");
    app.host.background = true;
    let _ = open(&mut app, 2);
    send(&mut app, M::Close);
    assert_eq!(shown(&app), (false, false));
    assert!(
        app.editor_ticks(),
        "the timer runs while the sound asked for is made"
    );
    tick_until_made(&mut app);
    assert!(
        app.editor.built.1.try_recv().is_err(),
        "a sound waits in the window after the editor closed"
    );
    assert!(!app.editor_ticks(), "the timer stops");

    let _ = open(&mut app, 2);
    send(&mut app, M::Close);
    let _ = open(&mut app, 2);
    // Every sound asked of a worker, taken as it lands (both, were there two: 72000 frames are
    // made in milliseconds), then handed back for the timer to take.
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut landed = Vec::new();
    while landed.is_empty() {
        assert!(Instant::now() < deadline, "the sound was never made");
        landed.extend(app.editor.built.1.try_iter());
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(500));
    landed.extend(app.editor.built.1.try_iter());
    assert_eq!(landed.len(), 1, "one build for the reopened editor");
    for sound in landed {
        app.editor
            .built
            .0
            .send(sound)
            .expect("the window's channel");
    }
    tick_until_made(&mut app);
    assert_eq!(view(&app), played(&app).samples());
    assert_eq!(view(&app).len(), 72_000);
    assert!(!app.editor_ticks());
}

// The app quitting while Save as new file… writes two minutes of stereo (the document clean,
// so the main window's X closes it at once) waits for the write: once the quit is through, the
// samples folder holds the file whole under its name, every frame, and no temporary file.
#[test]
fn a_quit_while_a_sound_saves_waits_for_the_write() {
    use super::Busy;
    let (mut app, _player, dir, frames) = long_sound("quit", 2 * 60 * 48_000);
    let _ = app.update(Message::OpenSampleEditor(ChannelId(1)));
    app.host.background = true;
    send(&mut app, M::SaveAs);
    assert_eq!(
        app.sample_editor.as_ref().and_then(|open| open.busy),
        Some(Busy::Saving)
    );
    let _ = app.update(Message::File(FileMessage::CloseRequested));
    assert!(app.files.exiting, "a clean document closes without asking");
    let samples = dir.join("samples");
    let mut names: Vec<String> = fs::read_dir(&samples)
        .expect("the samples folder")
        .map(|entry| {
            let entry = entry.expect("an entry");
            entry.file_name().to_string_lossy().into_owned()
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["p30_long (edited).wav"],
        "the file whole, nothing else"
    );
    let reader =
        hound::WavReader::open(samples.join("p30_long (edited).wav")).expect("a whole WAV");
    assert_eq!(u64::from(reader.duration()), frames);
    drop(reader);
    let _ = fs::remove_dir_all(&dir);
}

/// Sets the time `path` was last written (a folder's too).
fn set_written(path: &Path, time: std::time::SystemTime) {
    let file = if path.is_dir() {
        fs::File::open(path)
    } else {
        fs::File::options().write(true).open(path)
    };
    file.and_then(|file| file.set_modified(time))
        .expect("dated");
}

/// The names in `dir`, sorted.
fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("a folder")
        .map(|entry| {
            let entry = entry.expect("an entry");
            entry.file_name().to_string_lossy().into_owned()
        })
        .collect();
    names.sort();
    names
}

// What a Save as new file… cut short by more than a quit (the app killed mid-write) left in a
// project's samples folder goes as the project opens, at the app's start and by File → Open:
// files named as the writer names its temporary files ("<name>.wav.<process>-<write>.partial")
// and written over a minute ago. A fresh one (a write under way, maybe another instance's),
// every other name (sounds, another .partial, a process id that is no number, another
// extension, a name after the suffix, no stem) and a folder of such a name stay.
#[test]
fn opening_a_project_sweeps_what_a_killed_save_left() {
    use std::time::{Duration, SystemTime};
    let dir = temp_dir("sweep");
    fs::create_dir_all(dir.join("p30_sound")).expect("a folder");
    for sound in ["ramp48000.wav", "ramp72000.wav"] {
        fs::copy(
            fixtures().join("p30_sound").join(sound),
            dir.join("p30_sound").join(sound),
        )
        .expect("copied");
    }
    let path = dir.join("p30_undo.json");
    fs::copy(fixtures().join("p30_undo.json"), &path).expect("copied");
    let samples = dir.join("samples");
    fs::create_dir_all(&samples).expect("a folder");
    let old = SystemTime::now() - Duration::from_secs(120);
    let stale = "ramp72000 (edited).wav.4242-0.partial";
    let kept = [
        "ramp72000 (edited).wav",
        "take 1.wav",
        "notes.partial",
        "ramp.wav.42a-0.partial",
        "ramp.flac.42-0.partial",
        "ramp.wav.42-0.partial.txt",
        ".wav.42-0.partial",
    ];
    let leave = |name: &str| {
        let file = samples.join(name);
        fs::write(&file, b"RIFF").expect("written");
        set_written(&file, old);
    };
    leave(stale);
    for name in kept {
        leave(name);
    }
    fs::write(
        samples.join("ramp72000 (edited 2).wav.4242-1.partial"),
        b"RIFF",
    )
    .expect("written");
    let folder = samples.join("folder.wav.42-0.partial");
    fs::create_dir_all(&folder).expect("a folder");
    set_written(&folder, old);
    let mut stays: Vec<String> = kept.iter().map(|name| (*name).to_owned()).collect();
    stays.push("ramp72000 (edited 2).wav.4242-1.partial".to_owned());
    stays.push("folder.wav.42-0.partial".to_owned());
    stays.sort();

    let project = project_file::open(&path).expect("opens").project;
    let (mut app, _player, _main) = app_of(project, &path, Host::offline());
    assert_eq!(names_in(&samples), stays, "the app started on the project");
    leave(stale);
    let _ = app.update(Message::File(FileMessage::OpenPicked(Some(path.clone()))));
    assert_eq!(app.error, None);
    assert_eq!(
        names_in(&samples),
        stays,
        "the project opened by File → Open"
    );
    let _ = fs::remove_dir_all(&dir);
}
