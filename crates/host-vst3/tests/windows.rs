//! VST3 plugin windows, with the test module's View class, whose editor logs every call
//! the host makes into it and is an XEmbed client (crates/test-plugin-vst3/src/view.rs). Expected
//! logs, sizes and words are read from tests/fixtures/p29_expected.json (gen_p29_expected.py).
//!
//! W2 runs everywhere (in a child of this test binary without `DISPLAY`). The others open real
//! windows, so they are ignored: `scripts/ui-check.sh` runs each alone (`--exact`) in a graphical
//! session with `DAWSOME_APP_ID=dawsome-test` and temp HOME and XDG folders. Each takes the
//! binary's lock (one window at a time), gives its windows the class `dawsome-test` (so a desktop
//! rule keeps them off the user's screen and focus), waits on X with ticks every 16 ms for at
//! most a second, and closes every window it opened, which it checks on its own connection.
//! A test binary of its own: the module's counts and its log are the process's.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![deny(clippy::multiple_unsafe_ops_per_block)]
#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use dawsome_engine::processor::{
    Editor, HostError, HostNotice, NoticeKind, Processor, ProcessorHost,
};
use dawsome_engine::project::{ChannelId, EffectId, ProcessorOwner, ProcessorRef, Project};
use dawsome_host_common::catalogue::{self, Catalogue, FileRecord, FileStatus, PluginFormat};
use dawsome_host_common::window;
use dawsome_host_vst3::{host_objects, scan, Vst3Host};
use serde_json::Value;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConfigureNotifyEvent, ConnectionExt as _, EventMask,
    FocusInEvent, MapState, NotifyDetail, NotifyMode, CONFIGURE_NOTIFY_EVENT, FOCUS_IN_EVENT,
    FOCUS_OUT_EVENT,
};
use x11rb::protocol::ErrorKind;
use x11rb::rust_connection::RustConnection;

#[path = "../../../tests/support/test_plugins.rs"]
mod test_plugins;

const MODULE: &str = "Dawsome Test VST3.vst3";
const VIEW: ProcessorOwner = ProcessorOwner::Channel(ChannelId(1));
/// The class the windows of these tests carry (what `DAWSOME_APP_ID` names for the app's).
const CLASS: &str = "dawsome-test";
/// Set in W2's child.
const CHILD: &str = "DAWSOME_WINDOW_TEST_CHILD";

/// One window at a time.
static TURN: Mutex<()> = Mutex::new(());

fn turn() -> std::sync::MutexGuard<'static, ()> {
    TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn p29() -> Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/p29_expected.json"))
        .expect("p29_expected.json parses")
}

fn texts(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("a list")
        .iter()
        .map(|text| text.as_str().expect("a text").to_owned())
        .collect()
}

fn size(value: &Value) -> (u32, u32) {
    (
        value[0].as_u64().expect("a width") as u32,
        value[1].as_u64().expect("a height") as u32,
    )
}

fn words(value: &Value) -> Vec<u32> {
    value
        .as_array()
        .expect("words")
        .iter()
        .map(|word| word.as_u64().expect("a word") as u32)
        .collect()
}

fn module() -> PathBuf {
    test_plugins::test_plugins_vst3().join(MODULE)
}

/// A catalogue of the test module, described in this process (the scan child's work).
fn catalogue() -> Catalogue {
    let plugins = scan::describe_module(&module(), || {}).expect("the test module loads");
    let (size, mtime) =
        catalogue::module_stamp(PluginFormat::Vst3, &module()).expect("the module is there");
    Catalogue {
        files: vec![FileRecord {
            file: module(),
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

/// The View's channel as p29_vst3_view.json has it: its id and its gain-0.5 preset.
fn view_reference() -> ProcessorRef {
    let project = Project::load_json(include_str!("../../../tests/fixtures/p29_vst3_view.json"))
        .expect("the fixture loads");
    project.channels[&ChannelId(1)]
        .processor
        .clone()
        .expect("a plugin channel")
}

/// The test module's exports, read through the image the host loads (a second `dlopen` of one
/// path is that image).
struct Oracle {
    library: libloading::Library,
}

impl Oracle {
    fn open() -> Self {
        let binary = PluginFormat::Vst3.binary(&module());
        // SAFETY: the test module, our own code; opening it again runs no entry (only the host
        // calls `ModuleEntry`).
        let library = unsafe { libloading::Library::new(binary) }.expect("opens");
        Self { library }
    }

    /// The export `name`.
    fn get<T: Copy>(&self, name: &[u8]) -> T {
        // SAFETY: the module exports `name` with the signature `T` names (crates/test-plugin-vst3:
        // lib.rs and view.rs). The pointer copied out is used only while `self` holds the
        // library open (every caller calls it at once).
        unsafe { *self.library.get::<T>(name).expect("exported") }
    }

    fn text(&self, name: &[u8]) -> Vec<String> {
        let read: extern "C" fn(*mut u8, usize) -> usize = self.get(name);
        let len = read(std::ptr::null_mut(), 0);
        let mut buffer = vec![0u8; len];
        read(buffer.as_mut_ptr(), len);
        let text = String::from_utf8(buffer).expect("UTF-8");
        text.lines().map(str::to_owned).collect()
    }

    fn log(&self) -> Vec<String> {
        self.text(b"dawsome_test_vst3_view_log\0")
    }

    fn heard(&self) -> Vec<String> {
        self.text(b"dawsome_test_vst3_view_heard\0")
    }

    fn clear(&self) {
        let clear: extern "C" fn() = self.get(b"dawsome_test_vst3_view_log_clear\0");
        clear();
    }

    fn window(&self) -> u32 {
        let window: extern "C" fn() -> u32 = self.get(b"dawsome_test_vst3_view_window\0");
        window()
    }

    fn windows(&self) -> i32 {
        let windows: extern "C" fn() -> i32 = self.get(b"dawsome_test_vst3_view_windows\0");
        windows()
    }

    /// The editor's own `resizeView` now.
    fn resize(&self, (w, h): (u32, u32)) -> i32 {
        let resize: extern "C" fn(i32, i32) -> i32 = self.get(b"dawsome_test_vst3_view_resize\0");
        resize(w as i32, h as i32)
    }

    fn poke(&self) -> i32 {
        let poke: extern "C" fn() -> i32 = self.get(b"dawsome_test_vst3_view_poke\0");
        poke()
    }

    fn fixed(&self, on: bool) {
        let fixed: extern "C" fn(i32) = self.get(b"dawsome_test_vst3_view_fixed\0");
        fixed(i32::from(on));
    }

    fn nested(&self, on: bool) {
        let nested: extern "C" fn(i32) = self.get(b"dawsome_test_vst3_view_nested\0");
        nested(i32::from(on));
    }

    fn live(&self) -> isize {
        let live: extern "C" fn() -> isize = self.get(b"dawsome_test_vst3_live_objects\0");
        live()
    }
}

// W2. Without X11 (in a child of this binary with no DISPLAY): the classes without an editor
// open their parameter window (Generic) and no display is made (one would fail here); the View,
// whose editor takes our window type, is the reason X11 is missing, its editor let go at once.
// Released and ticked, nothing of theirs is left; with the host gone, nothing of the module's or
// the host's.
#[test]
fn without_x11_an_editor_is_its_reason_and_the_others_their_panel() {
    if std::env::var_os(CHILD).is_none() {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("vst3-windows-w2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let test = "without_x11_an_editor_is_its_reason_and_the_others_their_panel";
        let output = Command::new(std::env::current_exe().expect("the test binary"))
            .args([test, "--exact", "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .env_remove("DISPLAY")
            .env("HOME", &dir)
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .env("XDG_DATA_HOME", dir.join("data"))
            .env("XDG_CACHE_HOME", dir.join("cache"))
            .env("XDG_STATE_HOME", dir.join("state"))
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
    let p29 = p29();
    let oracle = Oracle::open();
    let catalogue = catalogue();
    let ids =
        &serde_json::from_str::<Value>(include_str!("../../../tests/fixtures/p27_expected.json"))
            .expect("p27_expected.json parses")["ids"];
    let ours = host_objects();
    {
        let host = Vst3Host::new(catalogue);
        let no_editor = texts(&p29["no_editor"]);
        let mut halves: Vec<Box<dyn Processor>> = Vec::new();
        let mut owners = Vec::new();
        for (index, class) in no_editor.iter().enumerate() {
            let key = class.to_lowercase();
            let id = ids[key.as_str()].as_str().expect("an id");
            let owner = if class == "Gain" || class == "Latency" {
                ProcessorOwner::Effect(EffectId(index as u32 + 1))
            } else {
                ProcessorOwner::Channel(ChannelId(index as u32 + 2))
            };
            let reference = ProcessorRef::plugin(format!("vst3:{id}"), class);
            halves.push(host.create(owner, &reference, 48_000).expect("creates"));
            assert_eq!(
                host.open_editor(owner, class),
                Ok(Editor::Generic),
                "{class}"
            );
            owners.push(owner);
        }
        halves.push(
            host.create(VIEW, &view_reference(), 48_000)
                .expect("creates"),
        );
        owners.push(VIEW);
        oracle.clear();
        let reason = p29["no_display"]["reason"].as_str().expect("a reason");
        assert_eq!(
            host.open_editor(VIEW, "Dawsome Test View"),
            Err(HostError::Failed(reason.to_owned()))
        );
        assert_eq!(oracle.log(), texts(&p29["no_display"]["log"]));
        assert!(host.editor(VIEW).is_none());
        drop(halves);
        for owner in owners {
            host.release(owner);
        }
        host.tick();
        assert_eq!(host.instances(), 0);
    }
    assert_eq!(oracle.live(), 0, "nothing of the module's alive");
    assert_eq!(host_objects(), ours, "nothing of the host's alive");
}

/// The test's own X connection, which checks what the host's windows are.
struct X {
    conn: RustConnection,
    wm_protocols: u32,
    wm_delete_window: u32,
}

impl X {
    fn connect() -> Self {
        let (conn, _) = RustConnection::connect(None).expect("X11 (a graphical session)");
        let intern = |name: &[u8]| {
            conn.intern_atom(false, name)
                .expect("asks")
                .reply()
                .expect("an atom")
                .atom
        };
        let (wm_protocols, wm_delete_window) =
            (intern(b"WM_PROTOCOLS"), intern(b"WM_DELETE_WINDOW"));
        Self {
            conn,
            wm_protocols,
            wm_delete_window,
        }
    }

    fn atom(&self, name: &str) -> u32 {
        self.conn
            .intern_atom(false, name.as_bytes())
            .expect("asks")
            .reply()
            .expect("an atom")
            .atom
    }

    /// `window`'s size, `None` once it is gone.
    fn size(&self, window: u32) -> Option<(u32, u32)> {
        let geometry = self.conn.get_geometry(window).ok()?.reply().ok()?;
        Some((u32::from(geometry.width), u32::from(geometry.height)))
    }

    fn parent(&self, window: u32) -> Option<u32> {
        Some(self.conn.query_tree(window).ok()?.reply().ok()?.parent)
    }

    fn viewable(&self, window: u32) -> bool {
        self.conn
            .get_window_attributes(window)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|attributes| attributes.map_state == MapState::VIEWABLE)
    }

    /// Whether `window` is gone: the server answers BadWindow for it.
    fn gone(&self, window: u32) -> bool {
        match self
            .conn
            .get_window_attributes(window)
            .expect("asks")
            .reply()
        {
            Err(x11rb::errors::ReplyError::X11Error(error)) => {
                error.error_kind == ErrorKind::Window
            }
            _ => false,
        }
    }

    fn bytes(&self, window: u32, property: u32) -> Vec<u8> {
        self.conn
            .get_property(false, window, property, AtomEnum::ANY, 0, 1024)
            .expect("asks")
            .reply()
            .expect("a property")
            .value
    }

    fn words(&self, window: u32, property: u32) -> Vec<u32> {
        let reply = self
            .conn
            .get_property(false, window, property, AtomEnum::ANY, 0, 1024)
            .expect("asks")
            .reply()
            .expect("a property");
        let words: Vec<u32> = reply.value32().map(Iterator::collect).unwrap_or_default();
        words
    }

    /// The close button pressed, as a window manager says so: WM_PROTOCOLS's WM_DELETE_WINDOW to
    /// the window's own client.
    fn close(&self, window: u32) {
        let message = ClientMessageEvent::new(
            32,
            window,
            self.wm_protocols,
            [self.wm_delete_window, 0, 0, 0, 0],
        );
        self.conn
            .send_event(false, window, EventMask::NO_EVENT, message)
            .expect("sends");
        self.conn.flush().expect("flushes");
    }

    /// `window` told it now has `size`, as a window manager tells a client its new size (the
    /// user's resize, whatever the desktop's own constraints would make of it).
    fn configure(&self, window: u32, (width, height): (u32, u32)) {
        let event = ConfigureNotifyEvent {
            response_type: CONFIGURE_NOTIFY_EVENT,
            sequence: 0,
            event: window,
            window,
            above_sibling: 0,
            x: 0,
            y: 0,
            width: width as u16,
            height: height as u16,
            border_width: 0,
            override_redirect: false,
        };
        self.conn
            .send_event(false, window, EventMask::NO_EVENT, event)
            .expect("sends");
        self.conn.flush().expect("flushes");
    }

    /// `window` told it gained (`focused`) or lost the focus, X11's focus `detail` saying how.
    fn focus(&self, window: u32, focused: bool, detail: u8) {
        let event = FocusInEvent {
            response_type: if focused {
                FOCUS_IN_EVENT
            } else {
                FOCUS_OUT_EVENT
            },
            detail: NotifyDetail::from(detail),
            sequence: 0,
            event: window,
            mode: NotifyMode::NORMAL,
        };
        self.conn
            .send_event(false, window, EventMask::NO_EVENT, event)
            .expect("sends");
        self.conn.flush().expect("flushes");
    }
}

/// Ticks `host` every 16 ms until `done`, for at most a second; the notices its ticks gave.
fn until(host: &Vst3Host, what: &str, mut done: impl FnMut() -> bool) -> Vec<HostNotice> {
    let start = Instant::now();
    let mut notices = Vec::new();
    loop {
        notices.extend(host.tick());
        if done() {
            return notices;
        }
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "waited a second for {what}"
        );
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Ticks `host` every 16 ms for `ms`, the notices it gave.
fn settle(host: &Vst3Host, ms: u64) -> Vec<HostNotice> {
    let start = Instant::now();
    let mut notices = Vec::new();
    while start.elapsed() < Duration::from_millis(ms) {
        notices.extend(host.tick());
        std::thread::sleep(Duration::from_millis(16));
    }
    notices
}

/// The View made and its editor open in a window of ours, mapped: the test's state.
struct Opened {
    host: Vst3Host,
    audio: Option<Box<dyn Processor>>,
    oracle: Oracle,
    x: X,
    /// Our window and the editor's.
    ours: u32,
    child: u32,
}

impl Opened {
    /// The View made, its editor opened (`fixed`: it says it does not resize) and its window
    /// mapped by ours, and the log of the opening.
    fn new(fixed: bool) -> (Self, Vec<String>) {
        window::set_class(CLASS);
        let p29 = p29();
        let oracle = Oracle::open();
        // The scan's own instance of the View logs its parts' end: cleared after it.
        let catalogue = catalogue();
        oracle.clear();
        oracle.fixed(fixed);
        oracle.nested(false);
        let host = Vst3Host::new(catalogue);
        let audio = host
            .create(VIEW, &view_reference(), 48_000)
            .expect("creates");
        let title = p29["title"].as_str().expect("a title");
        assert_eq!(host.open_editor(VIEW, title), Ok(Editor::Embedded));
        let log = oracle.log();
        let ours = host.editor(VIEW).expect("open").window;
        let child = oracle.window();
        assert_ne!(child, 0, "the editor made its window");
        let x = X::connect();
        let opened = Self {
            host,
            audio: Some(audio),
            oracle,
            x,
            ours,
            child,
        };
        until(
            &opened.host,
            "the editor's window mapped and heard so",
            || {
                opened.oracle.heard().iter().any(|line| line == "mapped")
                    && opened.x.viewable(opened.child)
            },
        );
        (opened, log)
    }

    /// Both windows gone.
    fn assert_closed(&self) {
        assert_closed(&self.x, self.ours, self.child);
    }

    /// The test's end: the audio half, then the host; both windows gone.
    fn finish(mut self) {
        drop(self.audio.take());
        let Opened {
            host,
            x,
            ours,
            child,
            ..
        } = self;
        drop(host);
        assert_closed(&x, ours, child);
    }
}

/// Our window and the editor's gone (BadWindow on the test's connection), which every test ends
/// with: a window left open on the desktop is a failure.
fn assert_closed(x: &X, ours: u32, child: u32) {
    let start = Instant::now();
    while !(x.gone(ours) && x.gone(child)) {
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "a window was left open: ours {ours:#x}, the editor's {child:#x}"
        );
        std::thread::sleep(Duration::from_millis(16));
    }
}

// W3. Opened: Embedded; the editor's calls in the format's order (the open log); it heard that it
// is embedded (EMBEDDED_NOTIFY naming our window, version 0) and was mapped; on the test's own
// connection its window is our window's child, viewable, both 400x300; our window carries the
// title (both encodings) and the class; the host asks for ticks; a second open is Embedded again
// and calls nothing.
#[test]
#[ignore = "opens a window: scripts/ui-check.sh runs it in a graphical session"]
fn w3_an_editor_opens_embedded_in_our_window() {
    let _turn = turn();
    let p29 = p29();
    let (opened, log) = Opened::new(false);
    assert_eq!(log, texts(&p29["open_log"]));
    assert_eq!(opened.oracle.heard(), texts(&p29["open_heard"]));
    let x = &opened.x;
    assert_eq!(x.parent(opened.child), Some(opened.ours));
    assert!(x.viewable(opened.child));
    let want = size(&p29["size"]);
    assert_eq!(x.size(opened.ours), Some(want));
    assert_eq!(x.size(opened.child), Some(want));
    let title = p29["title"].as_str().expect("a title").as_bytes().to_vec();
    assert_eq!(x.bytes(opened.ours, u32::from(AtomEnum::WM_NAME)), title);
    assert_eq!(x.bytes(opened.ours, x.atom("_NET_WM_NAME")), title);
    let wm_class: Vec<u8> = words(&p29["pins"]["wm_class"])
        .into_iter()
        .map(|byte| byte as u8)
        .collect();
    assert_eq!(
        x.bytes(opened.ours, u32::from(AtomEnum::WM_CLASS)),
        wm_class
    );
    assert!(opened.host.needs_ticks(), "an open editor is ticked");
    opened.oracle.clear();
    assert_eq!(opened.host.open_editor(VIEW, "again"), Ok(Editor::Embedded));
    assert!(opened.oracle.log().is_empty(), "nothing called again");
    opened.finish();
}

/// `until` our window and the editor's are `ours` and `child`, then a settling: the log is then
/// whole (no echo of ours brought anything).
fn sized(opened: &Opened, ours: (u32, u32), child: (u32, u32)) {
    let start = Instant::now();
    loop {
        opened.host.tick();
        let now = (opened.x.size(opened.ours), opened.x.size(opened.child));
        if now == (Some(ours), Some(child)) {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "waited a second for ours {ours:?} and the editor's {child:?}: {now:?}; log {:?}",
            opened.oracle.log()
        );
        std::thread::sleep(Duration::from_millis(16));
    }
    settle(&opened.host, 200);
}

/// W4's 0x0: the frame asks for 1x1 (the step's log) and the editor's own window, a child no
/// window manager sizes, is 1x1. Our window is a managed one: a window manager may give it more
/// (KWin: at least 2x48), and what it gives comes back as a user's resize, which the editor's
/// constraint holds (each "checkSizeConstraint" line its rule's, one onSize of the size held), and
/// both windows end at that size.
fn smallest(opened: &Opened, p29: &Value, step: &Value) {
    let start = Instant::now();
    while opened.x.size(opened.child) != Some((1, 1)) {
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "the editor's window at 1x1"
        );
        std::thread::sleep(Duration::from_millis(16));
    }
    settle(&opened.host, 400);
    let log = opened.oracle.log();
    let want = texts(&step["log"]);
    assert_eq!(log[..want.len()], want[..], "{log:?}");
    let rest = &log[want.len()..];
    if opened.x.size(opened.ours) == Some((1, 1)) {
        assert!(rest.is_empty(), "{log:?}");
        return;
    }
    let (least, most) = (
        size(&p29["constraint"]["least"]),
        size(&p29["constraint"]["most"]),
    );
    let parse = |text: &str| -> (u32, u32) {
        let (w, h) = text.split_once('x').expect("WxH");
        (w.parse().expect("a width"), h.parse().expect("a height"))
    };
    let mut held = None;
    let mut sized_to = Vec::new();
    for line in rest {
        if let Some(sizes) = line.strip_prefix("checkSizeConstraint ") {
            let (given, answer) = sizes.split_once(" -> ").expect("a constraint line");
            let given = parse(given);
            let rule = (
                given.0.clamp(least.0, most.0),
                given.1.clamp(least.1, most.1),
            );
            assert_eq!(parse(answer), rule, "the editor's rule: {line}");
            assert_ne!(given, (1, 1), "the window manager's size, not ours");
            println!(
                "0x0: the window manager gave our window {}x{}",
                given.0, given.1
            );
            held = Some(rule);
        } else {
            let held = held.expect("a constraint before an onSize");
            assert_eq!(*line, format!("onSize {}x{}", held.0, held.1), "{log:?}");
            sized_to.push(held);
        }
    }
    let held = held.expect("the window manager's size came back as a user's resize");
    assert_eq!(sized_to, [held], "one onSize of the size held: {log:?}");
    sized(opened, held, held);
}

// W4. The editor's own resizes (resizeView): a new size resizes our window and calls onSize
// inside the call, then true; the same again is true and nothing else; an onSize that asks again
// inside is refused (false) and the first goes on; 0x0 is our window at 1x1. A fixed editor's
// window keeps its size as its least and most (WM_NORMAL_HINTS), which follow a resize it asks.
#[test]
#[ignore = "opens a window: scripts/ui-check.sh runs it in a graphical session"]
fn w4_an_editor_resizes_its_window_inside_its_call() {
    let _turn = turn();
    let p29 = p29();
    let (opened, _) = Opened::new(false);
    for step in p29["plugin_resizes"].as_array().expect("steps") {
        opened.oracle.clear();
        opened
            .oracle
            .nested(step["nested"].as_bool().expect("nested"));
        opened.oracle.resize(size(&step["asked"]));
        assert_eq!(opened.oracle.log(), texts(&step["log"]), "{step}");
        if size(&step["window"]) == (1, 1) {
            smallest(&opened, &p29, step);
            continue;
        }
        sized(&opened, size(&step["window"]), size(&step["child"]));
        assert_eq!(
            opened.oracle.log(),
            texts(&step["log"]),
            "nothing more: {step}"
        );
    }
    opened.finish();
    let fixed = &p29["fixed"];
    let (opened, log) = Opened::new(true);
    assert_eq!(log, texts(&fixed["open_log"]));
    let hints = opened.x.atom("WM_NORMAL_HINTS");
    assert_eq!(
        opened.x.words(opened.ours, hints),
        words(&p29["fixed_hints"]["before"])
    );
    opened.oracle.clear();
    opened.oracle.resize(size(&fixed["asked"]));
    assert_eq!(opened.oracle.log(), texts(&fixed["log"]));
    sized(&opened, size(&fixed["window"]), size(&fixed["window"]));
    assert_eq!(
        opened.x.words(opened.ours, hints),
        words(&p29["fixed_hints"]["after"])
    );
    opened.finish();
}

// W5. A user's resize of our window: held by the editor's checkSizeConstraint (200..1600 x
// 150..1200), our window takes the size it gives, onSize, and the echo of our own resize brings
// nothing more; a fixed editor is not asked.
#[test]
#[ignore = "opens a window: scripts/ui-check.sh runs it in a graphical session"]
fn w5_a_users_resize_is_held_by_the_editors_constraint() {
    let _turn = turn();
    let p29 = p29();
    let cases = p29["user_resizes"].as_array().expect("cases");
    let (opened, _) = Opened::new(false);
    for case in cases.iter().filter(|case| case["fixed"] == false) {
        settle(&opened.host, 200);
        opened.oracle.clear();
        opened.x.configure(opened.ours, size(&case["size"]));
        let want = size(&case["window"]);
        sized(&opened, want, want);
        assert_eq!(opened.oracle.log(), texts(&case["log"]), "{case}");
    }
    opened.finish();
    let (opened, _) = Opened::new(true);
    for case in cases.iter().filter(|case| case["fixed"] == true) {
        settle(&opened.host, 200);
        opened.oracle.clear();
        opened.x.configure(opened.ours, size(&case["size"]));
        settle(&opened.host, 300);
        assert_eq!(opened.oracle.log(), texts(&case["log"]), "{case}");
        assert_eq!(opened.x.size(opened.ours), Some(size(&case["window"])));
    }
    opened.finish();
}

// W6. The frame's run loop serves what the editor registered with it: a byte down its pipe is
// heard at the next tick, once ("fd pipe"), and not before a tick.
#[test]
#[ignore = "opens a window: scripts/ui-check.sh runs it in a graphical session"]
fn w6_the_frames_run_loop_serves_the_editors_file_descriptors() {
    let _turn = turn();
    let p29 = p29();
    let run_loop = &p29["run_loop"];
    let (opened, _) = Opened::new(false);
    let registered = opened.host.editor(VIEW).expect("open");
    assert_eq!(
        (registered.timers, registered.fds),
        (0, 2),
        "its X connection and its pipe"
    );
    settle(&opened.host, 100);
    opened.oracle.clear();
    assert_eq!(
        i64::from(opened.oracle.poke()),
        run_loop["poked"].as_i64().expect("bytes")
    );
    assert!(opened.oracle.log().is_empty(), "no tick yet");
    opened.host.tick();
    assert_eq!(opened.oracle.log(), texts(&run_loop["log"]));
    opened.host.tick();
    assert_eq!(opened.oracle.log(), texts(&run_loop["log"]), "once");
    opened.finish();
}

// W7. Our window's focus reaches the embedded editor as XEmbed's messages: in, WINDOW_ACTIVATE
// then FOCUS_IN; out, FOCUS_OUT then WINDOW_DEACTIVATE; the focus moving inside our window, or
// to the pointer's, nothing. The host calls none of the editor's key, wheel, focus or scale
// methods.
#[test]
#[ignore = "opens a window: scripts/ui-check.sh runs it in a graphical session"]
fn w7_our_focus_reaches_the_editor_as_xembed_messages() {
    let _turn = turn();
    let p29 = p29();
    let focus = &p29["focus"];
    let (opened, _) = Opened::new(false);
    let mut whole_log = opened.oracle.log();
    for (case, focused) in [
        ("in", true),
        ("out", false),
        ("out_inferior", false),
        ("in_pointer", true),
    ] {
        settle(&opened.host, 100);
        whole_log.extend(opened.oracle.log());
        opened.oracle.clear();
        let detail = focus[case]["detail"].as_u64().expect("detail") as u8;
        opened.x.focus(opened.ours, focused, detail);
        let want = texts(&focus[case]["heard"]);
        if want.is_empty() {
            settle(&opened.host, 300);
        } else {
            until(&opened.host, case, || {
                opened.oracle.heard().len() >= want.len()
            });
            settle(&opened.host, 100);
        }
        assert_eq!(opened.oracle.heard(), want, "{case}");
    }
    whole_log.extend(opened.oracle.log());
    for never in texts(&focus["never"]) {
        assert!(
            !whole_log.contains(&never),
            "{never} was called: {whole_log:?}"
        );
    }
    opened.finish();
}

// W8. Every way an editor closes: the close button (WM_DELETE_WINDOW, as a window manager sends
// it) gives one Closed notice and removes, unframes and lets go of the editor, both windows gone,
// the host quiet; released with its audio half gone, the parts then terminate; the host dropped
// (audio half first), the same; the host dropped under a live audio half, the editor still
// closes and the parts are kept.
#[test]
#[ignore = "opens a window: scripts/ui-check.sh runs it in a graphical session"]
fn w8_an_editor_closes_on_every_way_out() {
    let _turn = turn();
    let p29 = p29();
    let close = texts(&p29["close"]);
    let terminated = [close.clone(), texts(&p29["terminated"])].concat();

    let (opened, _) = Opened::new(false);
    settle(&opened.host, 100);
    opened.oracle.clear();
    opened.x.close(opened.ours);
    let mut closed = until(&opened.host, "the editor closed", || {
        opened.host.editor(VIEW).is_none()
    });
    closed.extend(settle(&opened.host, 100));
    let notices: Vec<&HostNotice> = closed
        .iter()
        .filter(|notice| notice.kind == NoticeKind::Closed)
        .collect();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].owner, VIEW);
    assert_eq!(opened.oracle.log(), close);
    opened.assert_closed();
    assert_eq!(opened.oracle.window(), 0);
    assert!(!opened.host.needs_ticks(), "nothing open, nothing waiting");
    opened.finish();

    let (mut opened, _) = Opened::new(false);
    settle(&opened.host, 100);
    opened.oracle.clear();
    drop(opened.audio.take());
    opened.host.release(VIEW);
    opened.host.tick();
    assert_eq!(opened.oracle.log(), terminated, "released");
    opened.assert_closed();
    opened.finish();

    let (mut opened, _) = Opened::new(false);
    settle(&opened.host, 100);
    opened.oracle.clear();
    drop(opened.audio.take());
    let Opened {
        host,
        oracle,
        x,
        ours,
        child,
        ..
    } = opened;
    drop(host);
    assert_eq!(oracle.log(), terminated, "the host dropped");
    assert_closed(&x, ours, child);

    // Last: the parts it keeps for good keep the module loaded.
    let (opened, _) = Opened::new(false);
    settle(&opened.host, 100);
    opened.oracle.clear();
    let Opened {
        host,
        audio,
        oracle,
        x,
        ours,
        child,
    } = opened;
    drop(host);
    assert_eq!(oracle.log(), close, "under a live audio half");
    drop(audio);
    assert_eq!(oracle.log(), close, "its parts kept");
    assert_closed(&x, ours, child);
}

/// This process's open file descriptors.
fn open_fds() -> usize {
    std::fs::read_dir("/proc/self/fd").expect("/proc").count()
}

// W9. Twenty cycles of open, mapped, closed by the close button and ticked: twenty attached,
// twenty removed, every window either side gone after its cycle, no file descriptor more after
// the twentieth than after the first; released, nothing of the module's, the host's or the
// editor's windows is left.
#[test]
#[ignore = "opens windows: scripts/ui-check.sh runs it in a graphical session"]
fn w9_twenty_editors_opened_and_closed_leave_nothing() {
    let _turn = turn();
    let p29 = p29();
    let cycles = p29["cycles"].as_u64().expect("cycles") as usize;
    window::set_class(CLASS);
    let oracle = Oracle::open();
    let catalogue = catalogue();
    oracle.clear();
    oracle.fixed(false);
    oracle.nested(false);
    let ours_before = host_objects();
    let x = X::connect();
    let mut after_first = None;
    {
        let host = Vst3Host::new(catalogue);
        let audio = host
            .create(VIEW, &view_reference(), 48_000)
            .expect("creates");
        for cycle in 1..=cycles {
            assert_eq!(
                host.open_editor(VIEW, "Dawsome Test View"),
                Ok(Editor::Embedded)
            );
            let ours = host.editor(VIEW).expect("open").window;
            let child = oracle.window();
            until(&host, "mapped", || {
                oracle
                    .heard()
                    .iter()
                    .filter(|line| *line == "mapped")
                    .count()
                    == cycle
            });
            x.close(ours);
            until(&host, "closed", || host.editor(VIEW).is_none());
            let gone = Instant::now();
            while !(x.gone(ours) && x.gone(child)) {
                assert!(
                    gone.elapsed() < Duration::from_secs(1),
                    "cycle {cycle}: a window left"
                );
                host.tick();
                std::thread::sleep(Duration::from_millis(16));
            }
            if cycle == 1 {
                after_first = Some(open_fds());
            }
        }
        assert_eq!(Some(open_fds()), after_first, "no file descriptor kept");
        let log = oracle.log();
        let count = |line: &str| log.iter().filter(|logged| *logged == line).count();
        assert_eq!(count("attached X11EmbedWindowID"), cycles);
        assert_eq!(count("removed"), cycles);
        assert_eq!(count("view freed"), cycles);
        drop(audio);
        host.release(VIEW);
        host.tick();
        assert_eq!(host.instances(), 0);
    }
    assert_eq!(oracle.live(), 0, "nothing of the module's alive");
    assert_eq!(host_objects(), ours_before, "nothing of the host's alive");
    assert_eq!(oracle.windows(), 0, "no window of the editor's alive");
}
