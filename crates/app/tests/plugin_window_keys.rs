//! The typing keyboard in a plugin's window, through the real door on X11 (XWayland on a Wayland
//! desktop): the app run as a program with the GUI test plugin (`dawsome_test_gui.clap`, the
//! sine with an interface, as the CLAP windows tests open it) on its first channel and that
//! plugin's window opened (`--open-plugin channel:1`), its keys and the player's notes printed
//! (`--key-monitor`). This test, the other X11 client, finds our window around the plugin's
//! interface and sends it keys (XSendEvent, as a window manager or a test sends a client
//! events: what the X server would deliver to our window). It never moves the X focus on the
//! desktop's display: a client setting it would take the keyboard from whatever X client the
//! user types in during the run. The second test types as a keyboard does (XTEST: real keys,
//! raw events and all, which XSendEvent cannot make) with the plugin's own window focused, on a
//! nested X server of its own (`tests/support/nested_x.rs`), and skips with a line when there is
//! none.
//!
//! The expected notes are read from the piano row's table (crates/app/src/keys/notes.rs): its
//! first note is 60 (C5, the toolbar's first octave) on A, S two semitones up (62), D four
//! (64). The keys' X keycodes are the evdev codes plus 8, as every X server on evdev numbers
//! them (input-event-codes.h: KEY_ESC 1, KEY_A 30, KEY_S 31, KEY_D 32).
//!
//! Ignored in `cargo test`: it needs an X display and opens windows (class `dawsome-test`, so a
//! window rule can keep them off the user's screen). scripts/ui-check.sh runs it.
#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use dawsome_host_clap::catalogue::{self, FileRecord, FileStatus, PluginFormat};
use dawsome_host_clap::{scan, Catalogue};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, EventMask, InputFocus, KeyButMask,
    KeyPressEvent, Window, KEY_PRESS_EVENT, KEY_RELEASE_EVENT,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

#[path = "../../../tests/support/nested_x.rs"]
mod nested_x;
#[path = "../../../tests/support/test_plugins.rs"]
mod test_plugins;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");
/// The channel the plugin plays on, and so our window's title: "<channel> — <plugin>".
const CHANNEL: &str = "Probe Gui";
const TITLE: &str = "Probe Gui \u{2014} Dawsome Test GUI";

/// X keycodes: evdev's plus 8.
const ESC: u8 = 1 + 8;
const CTRL: u8 = 29 + 8;
const A: u8 = 30 + 8;
const S: u8 = 31 + 8;
const D: u8 = 32 + 8;
/// The piano row's notes on A, S and D (keys/notes.rs: 0, 2 and 4 semitones above 60).
const A_NOTE: u8 = 60;
const S_NOTE: u8 = 62;
const D_NOTE: u8 = 64;
/// X11's modifier bit for Ctrl (ControlMask).
const CONTROL: u16 = 4;

/// The app, killed when the test ends however it ends.
struct App(Child);

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A folder of its own: p7_sine's project with the GUI test plugin on its channel, a catalogue
/// of the GUI file (described in this process, the scan child's work: a scripted run scans
/// nothing), and the per-user folders the app is pointed at, `settings` the settings file's text.
fn folder(name: &str, settings: Option<&str>) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dawsome-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("home/config/dawsome")).expect("temp dir");
    let sine = std::fs::read_to_string(Path::new(FIXTURES).join("p7_sine.json")).expect("p7");
    let project = sine
        .replace(
            r#""name":"Dawsome Test Sine","kind""#,
            &format!(r#""name":"{CHANNEL}","kind""#),
        )
        .replace(
            r#""id":"clap:org.dawsome.test.sine","name":"Dawsome Test Sine""#,
            r#""id":"clap:org.dawsome.test.gui","name":"Dawsome Test GUI""#,
        );
    assert!(project.contains("org.dawsome.test.gui") && project.contains(CHANNEL));
    std::fs::write(dir.join("project.json"), project).expect("writes");
    let file = test_plugins::test_plugin_gui().join("dawsome_test_gui.clap");
    let plugins = scan::describe_file(&file, || {}).expect("the GUI file loads");
    let (size, mtime) = catalogue::stamp(&file).expect("the file is there");
    let catalogue = Catalogue {
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
    };
    catalogue.save(&dir.join("catalogue.json")).expect("saves");
    if let Some(settings) = settings {
        std::fs::write(dir.join("home/config/dawsome/settings.json"), settings).expect("writes");
    }
    dir
}

/// The app on `dir`'s project with the plugin's window opened, on the X display (the Wayland one
/// unset for it alone; `display` when given, a nested one), printing its plugin windows' keys and
/// its notes; its lines come on the channel.
fn start(dir: &Path, display: Option<&str>) -> (App, mpsc::Receiver<String>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dawsome-app"));
    command
        .arg("--load")
        .arg(dir.join("project.json"))
        .arg("--plugin-path")
        .arg(test_plugins::test_plugin_gui())
        .arg("--cache")
        .arg(dir.join("catalogue.json"))
        .args([
            "--null-output",
            "--midi-ports",
            "none",
            "--open-plugin",
            "channel:1",
        ])
        .args(["--quit-after", "60000", "--key-monitor"])
        .env_remove("WAYLAND_DISPLAY")
        .env("HOME", dir.join("home"))
        .env("DAWSOME_APP_ID", "dawsome-test")
        .env("ICED_BACKEND", "tiny-skia")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if let Some(display) = display {
        command.env("DISPLAY", display);
    }
    for (name, folder) in [
        ("CONFIG", "config"),
        ("DATA", "data"),
        ("STATE", "state"),
        ("CACHE", "cache"),
    ] {
        let path = dir.join("home").join(folder);
        command
            .env(format!("XDG_{name}_HOME"), &path)
            .env(format!("DAWSOME_{name}_HOME"), &path);
    }
    let mut child = command.spawn().expect("the app starts");
    let stdout = child.stdout.take().expect("piped");
    let (lines, received) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    (App(child), received)
}

/// The next line the app prints that `wanted` picks, within `seconds`; every line read printed.
fn next_line(
    lines: &mpsc::Receiver<String>,
    seconds: u64,
    mut wanted: impl FnMut(&str) -> bool,
) -> String {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let line = lines
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("the app printed what was waited for");
        println!("plugin window keys: {line}");
        if wanted(&line) {
            return line;
        }
    }
}

/// A note the player started (`true`) or ended, in a `--key-monitor` line: its key.
fn note(line: &str) -> Option<(bool, u8)> {
    let started = if line.starts_with("key-monitor: player started a note") {
        true
    } else if line.starts_with("key-monitor: player ended a note") {
        false
    } else {
        return None;
    };
    let key = line
        .split("key: ")
        .nth(1)?
        .split(',')
        .next()?
        .trim()
        .parse()
        .ok()?;
    Some((started, key))
}

/// The next note the player started or ended.
fn next_note(lines: &mpsc::Receiver<String>) -> (bool, u8) {
    let line = next_line(lines, 5, |line| note(line).is_some());
    note(&line).expect("a note")
}

/// Our window around the plugin's interface: the one whose class is `dawsome-test` and whose
/// title is [`TITLE`], once it is there.
fn find_window(conn: &RustConnection, root: Window) -> Option<Window> {
    let mut pending = vec![root];
    while let Some(window) = pending.pop() {
        let property = |name: AtomEnum| {
            conn.get_property(false, window, name, AtomEnum::STRING, 0, 64)
                .ok()?
                .reply()
                .ok()
        };
        let class = property(AtomEnum::WM_CLASS)
            .is_some_and(|reply| reply.value.starts_with(b"dawsome-test\0"));
        let titled =
            property(AtomEnum::WM_NAME).is_some_and(|reply| reply.value == TITLE.as_bytes());
        if class && titled {
            return Some(window);
        }
        if let Some(tree) = conn
            .query_tree(window)
            .ok()
            .and_then(|tree| tree.reply().ok())
        {
            pending.extend(tree.children);
        }
    }
    None
}

/// Sends `to` a key event of `keycode` (`pressed`, or its release) at `time` with `state` held,
/// reported to `ours` from `child`; `propagate`: it goes up from `to` to the first window that
/// some client hears keys on, as the X server sends a real key.
#[allow(clippy::too_many_arguments)]
fn send_key(
    conn: &RustConnection,
    root: Window,
    (to, propagate): (Window, bool),
    (ours, child): (Window, Window),
    keycode: u8,
    pressed: bool,
    time: u32,
    state: u16,
) {
    let event = KeyPressEvent {
        response_type: if pressed {
            KEY_PRESS_EVENT
        } else {
            KEY_RELEASE_EVENT
        },
        detail: keycode,
        sequence: 0,
        time,
        root,
        event: ours,
        child,
        root_x: 0,
        root_y: 0,
        event_x: 10,
        event_y: 10,
        state: KeyButMask::from(state),
        same_screen: true,
    };
    let mask = if pressed {
        EventMask::KEY_PRESS
    } else {
        EventMask::KEY_RELEASE
    };
    conn.send_event(propagate, to, mask, event).expect("sends");
    conn.flush().expect("flushes");
    std::thread::sleep(Duration::from_millis(60));
}

/// The app started on `dir` (on `display`, else the desktop's), its plugin's window open; our
/// window and the plugin's own inside it.
fn opened(
    conn: &RustConnection,
    root: Window,
    dir: &Path,
    display: Option<&str>,
) -> (App, mpsc::Receiver<String>, Window, Window) {
    let (app, lines) = start(dir, display);
    next_line(&lines, 30, |line| {
        line == "opened Channel(ChannelId(1)): Embedded"
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let ours = loop {
        if let Some(window) = find_window(conn, root) {
            break window;
        }
        assert!(
            Instant::now() < deadline,
            "our window around the plugin's opened"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    let interface = loop {
        let tree = conn
            .query_tree(ours)
            .expect("asks")
            .reply()
            .expect("a tree");
        if let Some(&child) = tree.children.first() {
            break child;
        }
        assert!(
            Instant::now() < deadline,
            "the plugin's interface is inside ours"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    // The app's first frames and ticks.
    std::thread::sleep(Duration::from_millis(500));
    (app, lines, ours, interface)
}

#[test]
#[ignore = "needs an X display; opens windows (scripts/ui-check.sh runs it)"]
fn the_typing_keyboard_plays_while_a_plugin_window_has_the_keys() {
    let Ok((conn, screen)) = RustConnection::connect(None) else {
        println!("plugin window keys: no X display, skipped");
        return;
    };
    let root = conn.setup().roots[screen].root;
    let dir = folder("plugin-window-keys", None);
    let (app, lines, ours, interface) = opened(&conn, root, &dir, None);
    let masks = conn
        .get_window_attributes(interface)
        .expect("asks")
        .reply()
        .expect("attributes");
    println!(
        "plugin window keys: our window {ours:#x}, the interface's {interface:#x}, which every \
         client asks to hear {:#x} of (KEY_PRESS is 1)",
        u32::from(masks.all_event_masks)
    );
    let to_ours = (ours, false);
    let from = (ours, 0);
    // A, pressed and let go of in our window: the note on A, then off.
    send_key(&conn, root, to_ours, from, A, true, 1_000, 0);
    assert_eq!(next_note(&lines), (true, A_NOTE), "A plays its note");
    send_key(&conn, root, to_ours, from, A, false, 1_100, 0);
    assert_eq!(next_note(&lines), (false, A_NOTE), "and lets go of it");
    // Ctrl+A is the plugin's shortcut: no note. S after it plays its own, the next note heard.
    send_key(&conn, root, to_ours, from, A, true, 2_000, CONTROL);
    send_key(&conn, root, to_ours, from, A, false, 2_100, CONTROL);
    send_key(&conn, root, to_ours, from, S, true, 2_200, 0);
    assert_eq!(next_note(&lines), (true, S_NOTE), "Ctrl+A played nothing");
    send_key(&conn, root, to_ours, from, S, false, 2_300, 0);
    assert_eq!(next_note(&lines), (false, S_NOTE));
    // Esc is the plugin's: no note; D after it plays its own.
    send_key(&conn, root, to_ours, from, ESC, true, 3_000, 0);
    send_key(&conn, root, to_ours, from, ESC, false, 3_100, 0);
    send_key(&conn, root, to_ours, from, D, true, 3_200, 0);
    assert_eq!(next_note(&lines), (true, D_NOTE), "Esc played nothing");
    send_key(&conn, root, to_ours, from, D, false, 3_300, 0);
    assert_eq!(next_note(&lines), (false, D_NOTE));
    // A key whose source is the plugin's own window (where the focus or the pointer is once the
    // interface is clicked): its window takes no keys, so the X server hands it up to ours.
    let from_interface = (ours, interface);
    send_key(
        &conn,
        root,
        (interface, true),
        from_interface,
        A,
        true,
        4_000,
        0,
    );
    assert_eq!(
        next_note(&lines),
        (true, A_NOTE),
        "the interface's key reached ours"
    );
    send_key(
        &conn,
        root,
        (interface, true),
        from_interface,
        A,
        false,
        4_100,
        0,
    );
    assert_eq!(next_note(&lines), (false, A_NOTE));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);

    // The keyboard off (the setting a toolbar switch writes): the key comes, no note plays.
    let dir = folder(
        "plugin-window-keys-off",
        Some(r#"{"keyboard_plays_notes":false}"#),
    );
    let (app, lines, ours, _) = opened(&conn, root, &dir, None);
    let from = (ours, 0);
    send_key(&conn, root, (ours, false), from, A, true, 1_000, 0);
    send_key(&conn, root, (ours, false), from, A, false, 1_100, 0);
    next_line(&lines, 5, |line| {
        line.starts_with("key-monitor: a plugin window's key 30") && line.ends_with("released")
    });
    // The player posts a note within a few of its blocks; a second is many times that.
    let deadline = Instant::now() + Duration::from_secs(1);
    while let Ok(line) = lines.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        println!("plugin window keys: {line}");
        assert!(note(&line).is_none(), "the keyboard off played: {line}");
    }
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Types `keycode` as a keyboard does (XTEST: the server makes a real key of it, raw event and
/// all), down (`pressed`) or up, then gives the app a few of its ticks. Only ever on a nested X
/// server ([`nested_x`]).
fn type_key(conn: &RustConnection, root: Window, keycode: u8, pressed: bool) {
    let kind = if pressed {
        KEY_PRESS_EVENT
    } else {
        KEY_RELEASE_EVENT
    };
    conn.xtest_fake_input(kind, keycode, x11rb::CURRENT_TIME, root, 0, 0, 0)
        .expect("types");
    conn.flush().expect("flushes");
    std::thread::sleep(Duration::from_millis(60));
}

/// Gives `window` the X focus, as a plugin's interface takes it when clicked.
fn focus(conn: &RustConnection, window: Window) {
    conn.set_input_focus(InputFocus::PARENT, window, x11rb::CURRENT_TIME)
        .expect("asks")
        .check()
        .expect("focuses");
    let focus = conn
        .get_input_focus()
        .expect("asks")
        .reply()
        .expect("a focus");
    assert_eq!(focus.focus, window, "the interface's window has the focus");
}

/// `keycode` typed down and up while the app's lines are read: the note it started and the one
/// it ended, and how many times the app heard the key go down (`key-monitor: a plugin window's
/// key <scancode> (...) pressed`).
fn played(
    conn: &RustConnection,
    root: Window,
    lines: &mpsc::Receiver<String>,
    keycode: u8,
) -> ((bool, u8), (bool, u8), usize) {
    let pressed = format!("key-monitor: a plugin window's key {} ", keycode - 8);
    let mut presses = 0;
    let mut count = |line: &str| {
        if line.starts_with(&pressed) && line.ends_with(" pressed") {
            presses += 1;
        }
        note(line).is_some()
    };
    type_key(conn, root, keycode, true);
    let on = note(&next_line(lines, 5, &mut count)).expect("a note");
    type_key(conn, root, keycode, false);
    let off = note(&next_line(lines, 5, &mut count)).expect("a note");
    (on, off, presses)
}

// The typing keyboard with the plugin's own window holding the X focus, as an interface takes
// it when clicked, keys typed as a keyboard does on a nested X server of the test's own. First
// the GUI test interface as it is (its window takes no keys): each key's event comes up to our
// window, and its raw event comes as well, and the key is heard once. Then this test hears the
// interface's keys, as JUCE's windows do (Surge XT's): a key event stops there, and only the raw
// events reach ours, which play the same notes. Ctrl+A and Esc play nothing (sentinels S and D
// after them); with the keyboard off nothing plays. Notes from the piano row's table: A 60, S
// 62, D 64.
#[test]
#[ignore = "needs an X display; opens windows (scripts/ui-check.sh runs it)"]
fn the_typing_keyboard_plays_while_the_plugins_own_window_has_the_focus() {
    let nested = match nested_x::NestedX::start() {
        Ok(nested) => nested,
        Err(why) => {
            println!(
                "plugin window keys: skipped the typed keys: {why}; keys are typed on a nested X \
                 server only"
            );
            return;
        }
    };
    let display = nested.display.as_str();
    println!("plugin window keys: typed on the nested X server {display}");
    let (conn, screen) = RustConnection::connect(Some(display)).expect("the nested server");
    let root = conn.setup().roots[screen].root;
    conn.xtest_get_version(2, 2)
        .expect("asks")
        .reply()
        .expect("the nested server types (XTEST)");
    let dir = folder("plugin-window-typed", None);
    let (app, lines, _, interface) = opened(&conn, root, &dir, Some(display));
    focus(&conn, interface);
    assert_eq!(
        played(&conn, root, &lines, A),
        ((true, A_NOTE), (false, A_NOTE), 1),
        "A with the interface focused, its key event and raw event both ours: one note, once"
    );
    let hear_keys =
        ChangeWindowAttributesAux::new().event_mask(EventMask::KEY_PRESS | EventMask::KEY_RELEASE);
    conn.change_window_attributes(interface, &hear_keys)
        .expect("asks")
        .check()
        .expect("hears the interface's keys");
    assert_eq!(
        played(&conn, root, &lines, A),
        ((true, A_NOTE), (false, A_NOTE), 1),
        "A with the key events stopping at the interface: its raw event plays it"
    );
    for (keycode, pressed) in [(CTRL, true), (A, true), (A, false), (CTRL, false)] {
        type_key(&conn, root, keycode, pressed);
    }
    let (on, off, _) = played(&conn, root, &lines, S);
    assert_eq!(
        (on, off),
        ((true, S_NOTE), (false, S_NOTE)),
        "Ctrl+A played nothing"
    );
    type_key(&conn, root, ESC, true);
    type_key(&conn, root, ESC, false);
    let (on, off, _) = played(&conn, root, &lines, D);
    assert_eq!(
        (on, off),
        ((true, D_NOTE), (false, D_NOTE)),
        "Esc played nothing"
    );
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);

    let dir = folder(
        "plugin-window-typed-off",
        Some(r#"{"keyboard_plays_notes":false}"#),
    );
    let (app, lines, _, interface) = opened(&conn, root, &dir, Some(display));
    focus(&conn, interface);
    conn.change_window_attributes(interface, &hear_keys)
        .expect("asks")
        .check()
        .expect("hears the interface's keys");
    type_key(&conn, root, A, true);
    type_key(&conn, root, A, false);
    next_line(&lines, 5, |line| {
        line.starts_with("key-monitor: a plugin window's key 30") && line.ends_with("released")
    });
    let deadline = Instant::now() + Duration::from_secs(1);
    while let Ok(line) = lines.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        println!("plugin window keys: {line}");
        assert!(note(&line).is_none(), "the keyboard off played: {line}");
    }
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}
