//! Surge XT's CLAP interface, the plugin the programs ship (`scripts/fetch-surge.sh`), in a window
//! of ours: opened embedded, its window mapped inside ours, then our window asked by another
//! client for a wider size, and a taller one, as the user drags it (what `xdotool windowsize`
//! asks): each time our window ends at the size of Surge's own window inside it, with no black
//! beside it (the case seen in use: dragged wider, the interface stayed at its width with black
//! to its right). The sizes are printed for the record. Skipped with a line when Surge XT is
//! not fetched. And the keys that reach our host while Surge's own window has them: sent to a
//! window (XSendEvent) on the desktop's display, and typed as a keyboard does (XTEST) on a nested
//! X server of the test's own (`tests/support/nested_x.rs`), skipped with a line when there is
//! none.
//!
//! Ignored: it opens a window, so `scripts/ui-check.sh` runs it alone in a graphical session. The
//! plugin runs in this process and reads and makes user folders, so it runs again in a child of
//! this test binary whose HOME and XDG folders are temp ones ([`in_temp_home`]). Never
//! `/usr/lib/clap/Surge XT.clap`: the fetched copy alone.

#![cfg(target_os = "linux")]

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use dawsome_engine::processor::{Editor, HostNotice, NoticeKind, ProcessorHost};
use dawsome_engine::project::{ChannelId, ProcessorOwner, ProcessorRef};
use dawsome_host_clap::catalogue::{self, FileRecord, FileStatus, PluginFormat};
use dawsome_host_clap::{scan, Catalogue, ClapHost};
use dawsome_host_common::window;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt as _, EventMask, InputFocus,
    KeyButMask, KeyPressEvent, MapState, KEY_PRESS_EVENT, KEY_RELEASE_EVENT,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

#[path = "../../../tests/support/nested_x.rs"]
mod nested_x;
#[path = "../../../tests/support/test_plugins.rs"]
mod test_plugins;

/// Set in the child [`in_temp_home`] starts.
const CHILD: &str = "DAWSOME_REAL_WINDOW_CHILD";
/// The nested X server's display, set in a child that types keys (its `DISPLAY` too).
const NESTED: &str = "DAWSOME_NESTED_DISPLAY";
const OWNER: ProcessorOwner = ProcessorOwner::Channel(ChannelId(1));
/// The scale the plugins' windows are opened at: a desktop at 100 %, as these tests have always
/// opened them (Dawsome told no scale before), so what they check stays what they saw.
const SCALE: f64 = 1.0;

/// Whether this is the child that does `test`'s work: else the test runs again in a child of this
/// binary with temp HOME and XDG folders, whose result is this one's.
fn in_temp_home(test: &str) -> bool {
    in_temp_home_with(test, &[])
}

/// [`in_temp_home`], the child given `env` besides (a nested X server's `DISPLAY`).
fn in_temp_home_with(test: &str, env: &[(&str, &str)]) -> bool {
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("clap-real-windows")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let output = Command::new(std::env::current_exe().expect("the test binary"))
        .args([
            test,
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .env_remove("CLAP_PATH")
        .env("HOME", dir.join("home"))
        .env("XDG_CONFIG_HOME", dir.join("home/config"))
        .env("XDG_DATA_HOME", dir.join("home/data"))
        .env("XDG_CACHE_HOME", dir.join("home/cache"))
        .env("XDG_STATE_HOME", dir.join("home/state"))
        .envs(env.iter().copied())
        .output()
        .expect("run the test again");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{test} failed in its child ({}):\n{stdout}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("1 passed"),
        "{test} did not run in its child:\n{stdout}"
    );
    print!("{stdout}");
    false
}

/// A catalogue of `file`, described in this process (the scan child's work).
fn described(file: &Path) -> Catalogue {
    let plugins = scan::describe_file(file, || {}).expect("the plugin file loads");
    let (size, mtime) = catalogue::stamp(file).expect("the file is there");
    Catalogue {
        files: vec![FileRecord {
            file: file.to_owned(),
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

struct X {
    conn: RustConnection,
}

impl X {
    fn connect() -> Self {
        let (conn, _) = RustConnection::connect(None).expect("X11 (a graphical session)");
        Self { conn }
    }

    fn size(&self, window: u32) -> Option<(u32, u32)> {
        let geometry = self.conn.get_geometry(window).ok()?.reply().ok()?;
        Some((u32::from(geometry.width), u32::from(geometry.height)))
    }

    /// The largest viewable child of `window` and its size: the plugin's own window.
    fn interface(&self, window: u32) -> Option<(u32, (u32, u32))> {
        let tree = self.conn.query_tree(window).ok()?.reply().ok()?;
        tree.children
            .into_iter()
            .filter(|&child| {
                self.conn
                    .get_window_attributes(child)
                    .ok()
                    .and_then(|cookie| cookie.reply().ok())
                    .is_some_and(|attributes| attributes.map_state == MapState::VIEWABLE)
            })
            .filter_map(|child| Some((child, self.size(child)?)))
            .max_by_key(|&(_, (w, h))| u64::from(w) * u64::from(h))
    }

    fn gone(&self, window: u32) -> bool {
        self.conn
            .get_window_attributes(window)
            .ok()
            .is_some_and(|cookie| cookie.reply().is_err())
    }

    /// Every window under `window`, itself first.
    fn tree(&self, window: u32) -> Vec<u32> {
        let mut all = vec![window];
        let mut at = 0;
        while let Some(&next) = all.get(at) {
            if let Some(tree) = self
                .conn
                .query_tree(next)
                .ok()
                .and_then(|tree| tree.reply().ok())
            {
                all.extend(tree.children);
            }
            at += 1;
        }
        all
    }

    /// What every client together asks to hear of `window` (GetWindowAttributes'
    /// all-event-masks).
    fn heard(&self, window: u32) -> u32 {
        self.conn
            .get_window_attributes(window)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map_or(0, |attributes| u32::from(attributes.all_event_masks))
    }

    /// Sends `to` the press and the release of `keycode`, reported to `ours` from `child`;
    /// `propagate`: each goes up from `to` to the first window some client hears keys on, as
    /// the X server sends a real key.
    fn key(&self, (to, propagate): (u32, bool), (ours, child): (u32, u32), keycode: u8) {
        let root = self.conn.setup().roots[0].root;
        for (pressed, time) in [(true, 1_000), (false, 1_100)] {
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
                state: KeyButMask::from(0u16),
                same_screen: true,
            };
            let mask = if pressed {
                EventMask::KEY_PRESS
            } else {
                EventMask::KEY_RELEASE
            };
            self.conn
                .send_event(propagate, to, mask, event)
                .expect("sends");
            self.conn.flush().expect("flushes");
        }
    }

    /// Types `keycode` as a keyboard does (XTEST: the server makes a real key of it, raw event
    /// and all), down (`pressed`) or up. Only ever on a nested X server ([`nested_x`]).
    fn type_key(&self, keycode: u8, pressed: bool) {
        let root = self.conn.setup().roots[0].root;
        let kind = if pressed {
            KEY_PRESS_EVENT
        } else {
            KEY_RELEASE_EVENT
        };
        self.conn
            .xtest_fake_input(kind, keycode, x11rb::CURRENT_TIME, root, 0, 0, 0)
            .expect("types");
        self.conn.flush().expect("flushes");
    }

    /// The key events this client heard on `window` since it last asked, in order: pressed or
    /// not, and the keycode. A release with a press of the same key at the same time right after
    /// it is left out. Why: Xvfb 21.1.25 sends one before every key XTEST presses (a release
    /// from the server, not a client, at the press's own time; seen for Esc and A alike), which a
    /// real keyboard never does. It is X11's shape of a held key's repeat, and our host drops it
    /// by the same rule (host-common's x11.rs, `KeyState::raw`).
    fn keys_heard(&self, window: u32) -> Vec<(bool, u8)> {
        let mut heard = Vec::new();
        while let Ok(Some(event)) = self.conn.poll_for_event() {
            match event {
                Event::KeyPress(key) if key.event == window => {
                    heard.push((true, key.detail, key.time));
                }
                Event::KeyRelease(key) if key.event == window => {
                    heard.push((false, key.detail, key.time));
                }
                _ => {}
            }
        }
        let repeat_shape = |at: usize| {
            let (pressed, keycode, time) = heard[at];
            !pressed && heard.get(at + 1) == Some(&(true, keycode, time))
        };
        (0..heard.len())
            .filter(|&at| !repeat_shape(at))
            .map(|at| (heard[at].0, heard[at].1))
            .collect()
    }

    /// Asks the window manager, from this other client, for `window` at `size` (what
    /// `xdotool windowsize` asks).
    fn ask_size(&self, window: u32, (width, height): (u32, u32)) {
        let aux = ConfigureWindowAux::new().width(width).height(height);
        self.conn.configure_window(window, &aux).expect("asks");
        self.conn.flush().expect("flushes");
    }
}

fn settle(host: &ClapHost, ms: u64) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(ms) {
        host.tick();
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Ticks `host` every 16 ms until our window `ours` has had the size of the interface's window
/// inside it for 300 ms, for at most `seconds`; that size.
fn held_to_its_interface(
    host: &ClapHost,
    x: &X,
    ours: u32,
    seconds: u64,
    what: &str,
) -> (u32, u32) {
    let start = Instant::now();
    let mut since: Option<(Instant, (u32, u32))> = None;
    loop {
        host.tick();
        let now = (x.size(ours), x.interface(ours).map(|(_, size)| size));
        match now {
            (Some(window), Some(inside)) if window == inside => match since {
                Some((from, size)) if size == window => {
                    if from.elapsed() >= Duration::from_millis(300) {
                        return window;
                    }
                }
                _ => since = Some((Instant::now(), window)),
            },
            _ => since = None,
        }
        assert!(
            start.elapsed() < Duration::from_secs(seconds),
            "{what}: waited {seconds} s for our window at its interface's size: ours {:?}, the \
             interface's {:?}",
            now.0,
            now.1
        );
        std::thread::sleep(Duration::from_millis(16));
    }
}

#[test]
#[ignore = "opens a plugin's window: scripts/ui-check.sh runs it in a graphical session"]
fn surge_xt_holds_our_window_to_its_interface() {
    let Some(dir) = test_plugins::fetched("surge/linux-x86_64", "Surge XT.clap") else {
        println!("skipped: surge/linux-x86_64 is not fetched (scripts/fetch-surge.sh)");
        return;
    };
    if !in_temp_home("surge_xt_holds_our_window_to_its_interface") {
        return;
    }
    window::set_class("dawsome-test");
    let catalogue = described(&dir.join("Surge XT.clap"));
    let record = catalogue.files[0]
        .plugins
        .iter()
        .find(|plugin| plugin.name == "Surge XT")
        .expect("Surge XT in its file")
        .clone();
    let x = X::connect();
    let host = ClapHost::new(catalogue);
    let reference = ProcessorRef::plugin(format!("clap:{}", record.id), &record.name);
    let audio = host.create(OWNER, &reference, 48_000).expect("creates");
    assert_eq!(
        host.open_editor(OWNER, "Surge XT", SCALE),
        Ok(Editor::Embedded)
    );
    let ours = host.editor_window(OWNER).expect("open in a window of ours");
    let opened = held_to_its_interface(&host, &x, ours, 5, "opened");
    let (interface, _) = x.interface(ours).expect("its window inside ours");
    println!("Surge XT: opened at {opened:?}, its window {interface:#x} inside ours {ours:#x}");
    let mut windows = vec![ours, interface];
    let mut from = opened;
    for (what, wider, taller) in [("dragged wider", 300, 0), ("dragged taller", 0, 200)] {
        let asked = (from.0 + wider, from.1 + taller);
        x.ask_size(ours, asked);
        // The window manager's answer first: until it comes, our window keeps the size it had.
        let start = Instant::now();
        while x.size(ours) == Some(from) && start.elapsed() < Duration::from_secs(1) {
            host.tick();
            std::thread::sleep(Duration::from_millis(16));
        }
        let held = held_to_its_interface(&host, &x, ours, 5, what);
        println!("Surge XT: {what}, {asked:?} asked: our window and its interface at {held:?}");
        windows.extend(x.interface(ours).map(|(window, _)| window));
        from = held;
    }
    settle(&host, 300);
    drop(audio);
    drop(host);
    let start = Instant::now();
    while !windows.iter().all(|&window| x.gone(window)) {
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "Surge XT: a window was left open"
        );
        std::thread::sleep(Duration::from_millis(16));
    }
    println!("Surge XT: quit with its interface open, every window gone");
}

/// Ticks `host` every 16 ms for `ms`; the notices it gave.
fn notices_for(host: &ClapHost, ms: u64) -> Vec<HostNotice> {
    let start = Instant::now();
    let mut notices = Vec::new();
    while start.elapsed() < Duration::from_millis(ms) {
        notices.extend(host.tick());
        std::thread::sleep(Duration::from_millis(16));
    }
    notices
}

/// The keys among `notices`.
fn keys(notices: &[HostNotice]) -> Vec<&NoticeKind> {
    notices
        .iter()
        .map(|notice| &notice.kind)
        .filter(|kind| matches!(kind, NoticeKind::Key { .. }))
        .collect()
}

// What reaches our window on X11 while Surge XT's interface has the keys, recorded. JUCE, which
// Surge XT is built on, asks to hear keys on its windows (juce_XWindowSystem_linux.cpp's
// getAllEventsMask: KeyPressMask | KeyReleaseMask) and takes the X focus for them when clicked
// (grabFocus: XSetInputFocus). The X server reports a key to the focus window, or to the window
// under the pointer when that lies inside it, and from there to the first window up the tree a
// client hears keys on: so a key event whose source is Surge's window is Surge's alone. A key
// sent there (XSendEvent, a propagated key) gives our host no key: a sent event has no raw
// event, which is what a real key reaches us by then (the nested test below types one). A key
// sent to our window itself is ours (the wrapper focused, the pointer outside Surge's window).
// Keycode 38 is evdev's KEY_A (30) plus 8.
#[test]
#[ignore = "opens a plugin's window: scripts/ui-check.sh runs it in a graphical session"]
fn surge_xt_hears_its_keys_itself_and_ours_only_reach_our_window() {
    let Some(dir) = test_plugins::fetched("surge/linux-x86_64", "Surge XT.clap") else {
        println!("skipped: surge/linux-x86_64 is not fetched (scripts/fetch-surge.sh)");
        return;
    };
    if !in_temp_home("surge_xt_hears_its_keys_itself_and_ours_only_reach_our_window") {
        return;
    }
    window::set_class("dawsome-test");
    let catalogue = described(&dir.join("Surge XT.clap"));
    let record = catalogue.files[0]
        .plugins
        .iter()
        .find(|plugin| plugin.name == "Surge XT")
        .expect("Surge XT in its file")
        .clone();
    let x = X::connect();
    let host = ClapHost::new(catalogue);
    let reference = ProcessorRef::plugin(format!("clap:{}", record.id), &record.name);
    let audio = host.create(OWNER, &reference, 48_000).expect("creates");
    assert_eq!(
        host.open_editor(OWNER, "Surge XT", SCALE),
        Ok(Editor::Embedded)
    );
    let ours = host.editor_window(OWNER).expect("open in a window of ours");
    held_to_its_interface(&host, &x, ours, 5, "opened");
    let (interface, _) = x.interface(ours).expect("its window inside ours");
    let windows = x.tree(ours);
    let key_press = u32::from(EventMask::KEY_PRESS);
    for &window in &windows {
        println!(
            "Surge XT: window {window:#x} is heard for {:#x}{}",
            x.heard(window),
            if window == ours { " (ours)" } else { "" }
        );
    }
    assert_ne!(
        x.heard(interface) & key_press,
        0,
        "Surge's window takes keys (JUCE's getAllEventsMask)"
    );
    notices_for(&host, 200);
    x.key((interface, true), (ours, interface), 38);
    let from_surge = notices_for(&host, 500);
    assert!(
        keys(&from_surge).is_empty(),
        "a key from Surge's window reached ours: {from_surge:?}"
    );
    println!("Surge XT: a key sent with its window as the source stays its own: none reached ours");
    x.key((ours, false), (ours, 0), 38);
    let to_ours = notices_for(&host, 500);
    assert_eq!(
        keys(&to_ours),
        [
            &NoticeKind::Key {
                scancode: 30,
                pressed: true,
                repeat: false
            },
            &NoticeKind::Key {
                scancode: 30,
                pressed: false,
                repeat: false
            }
        ],
        "a key sent to our window is ours"
    );
    println!("Surge XT: a key sent to our window itself is ours: A pressed and let go of");
    drop(audio);
    drop(host);
    let start = Instant::now();
    while !windows.iter().all(|&window| x.gone(window)) {
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "Surge XT: a window was left open"
        );
        std::thread::sleep(Duration::from_millis(16));
    }
}

// Surge XT's window holding the X focus, as JUCE leaves it once clicked: keys typed as a
// keyboard does (XTEST, on a nested X server of the test's own) reach our host by their raw
// events, while their key events still go to Surge's window, which this test hears as well (any
// client may ask for a window's keys). A press with Ctrl held is the plugin's shortcut: of
// Ctrl+A only the releases come. Esc comes on (the app keeps it from playing, not the host).
// Keycodes: evdev's plus 8 (KEY_ESC 1, KEY_LEFTCTRL 29, KEY_A 30).
#[test]
#[ignore = "opens a plugin's window: scripts/ui-check.sh runs it in a graphical session"]
fn surge_xt_keys_reach_us_while_its_window_has_the_focus() {
    const TEST: &str = "surge_xt_keys_reach_us_while_its_window_has_the_focus";
    const ESC: u8 = 1 + 8;
    const CTRL: u8 = 29 + 8;
    const A: u8 = 30 + 8;
    let Some(dir) = test_plugins::fetched("surge/linux-x86_64", "Surge XT.clap") else {
        println!("skipped: surge/linux-x86_64 is not fetched (scripts/fetch-surge.sh)");
        return;
    };
    if std::env::var_os(CHILD).is_none() {
        match nested_x::NestedX::start() {
            Ok(nested) => {
                println!(
                    "Surge XT: keys typed on the nested X server {}",
                    nested.display
                );
                let display = nested.display.as_str();
                in_temp_home_with(TEST, &[("DISPLAY", display), (NESTED, display)]);
            }
            Err(why) => println!("skipped: {why}; keys are typed on a nested X server only"),
        }
        return;
    }
    // Typed only on the nested server the parent started, never on a display that is not.
    let nested = std::env::var(NESTED).expect("started by its parent, on a nested X server");
    assert_eq!(
        std::env::var("DISPLAY").ok(),
        Some(nested),
        "DISPLAY is the nested server"
    );
    window::set_class("dawsome-test");
    let catalogue = described(&dir.join("Surge XT.clap"));
    let record = catalogue.files[0]
        .plugins
        .iter()
        .find(|plugin| plugin.name == "Surge XT")
        .expect("Surge XT in its file")
        .clone();
    let x = X::connect();
    x.conn
        .xtest_get_version(2, 2)
        .expect("asks")
        .reply()
        .expect("the nested server types (XTEST)");
    let host = ClapHost::new(catalogue);
    let reference = ProcessorRef::plugin(format!("clap:{}", record.id), &record.name);
    let audio = host.create(OWNER, &reference, 48_000).expect("creates");
    assert_eq!(
        host.open_editor(OWNER, "Surge XT", SCALE),
        Ok(Editor::Embedded)
    );
    let ours = host.editor_window(OWNER).expect("open in a window of ours");
    held_to_its_interface(&host, &x, ours, 5, "opened");
    let (interface, _) = x.interface(ours).expect("its window inside ours");
    let windows = x.tree(ours);
    let hear_keys =
        ChangeWindowAttributesAux::new().event_mask(EventMask::KEY_PRESS | EventMask::KEY_RELEASE);
    x.conn
        .change_window_attributes(interface, &hear_keys)
        .expect("asks")
        .check()
        .expect("hears Surge's window's keys too");
    x.conn
        .set_input_focus(InputFocus::PARENT, interface, x11rb::CURRENT_TIME)
        .expect("asks")
        .check()
        .expect("focuses");
    let focus = x
        .conn
        .get_input_focus()
        .expect("asks")
        .reply()
        .expect("a focus");
    assert_eq!(focus.focus, interface, "Surge's window has the focus");
    notices_for(&host, 200);
    // Each key typed, then the host ticked: what is held is asked as a tick reads a press.
    let typed = |keys: &[(u8, bool)]| {
        let mut notices = Vec::new();
        for &(keycode, pressed) in keys {
            x.type_key(keycode, pressed);
            notices.extend(notices_for(&host, 100));
        }
        notices.extend(notices_for(&host, 300));
        notices
    };
    let key = |scancode: u32, pressed: bool| NoticeKind::Key {
        scancode,
        pressed,
        repeat: false,
    };
    let notices = typed(&[(A, true), (A, false)]);
    assert_eq!(
        keys(&notices),
        [&key(30, true), &key(30, false)],
        "A typed with Surge's window focused reached our host"
    );
    assert_eq!(
        x.keys_heard(interface),
        [(true, A), (false, A)],
        "and Surge's window had its key events"
    );
    println!("Surge XT: A typed with its window focused: ours heard it, and its window did");
    let notices = typed(&[(CTRL, true), (A, true), (A, false), (CTRL, false)]);
    assert_eq!(
        keys(&notices),
        [&key(30, false), &key(29, false)],
        "Ctrl+A: no press, both releases"
    );
    let notices = typed(&[(ESC, true), (ESC, false)]);
    assert_eq!(keys(&notices), [&key(1, true), &key(1, false)], "Esc");
    println!("Surge XT: Ctrl+A gave releases alone; Esc came on to the app");
    drop(audio);
    drop(host);
    let start = Instant::now();
    while !windows.iter().all(|&window| x.gone(window)) {
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "Surge XT: a window was left open"
        );
        std::thread::sleep(Duration::from_millis(16));
    }
}
