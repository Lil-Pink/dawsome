//! CLAP plugin windows, with the GUI file's test interface (`org.dawsome.test.gui`,
//! crates/test-plugin/src/gui.rs), which logs every call the host makes into it and makes its own
//! window, a child of ours. Expected logs, sizes and words are read from
//! tests/fixtures/p29_expected.json, its "clap" key (gen_p29_expected.py).
//!
//! They open real windows, so they are ignored: `scripts/ui-check.sh` runs each alone (`--exact`)
//! in a graphical session with `DAWSOME_APP_ID=dawsome-test` and a temp HOME. Each takes the
//! binary's lock (one window at a time), gives its windows the class `dawsome-test` (so a desktop
//! rule keeps them off the user's screen and focus), waits on X with ticks every 16 ms for at
//! most a second, and closes every window it opened, which it checks on its own connection.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![deny(clippy::multiple_unsafe_ops_per_block)]
#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use dawsome_engine::processor::{Editor, HostNotice, NoticeKind, Processor, ProcessorHost};
use dawsome_engine::project::{ChannelId, ProcessorOwner, ProcessorRef};
use dawsome_host_clap::catalogue::{self, FileRecord, FileStatus, PluginFormat};
use dawsome_host_clap::{scan, Catalogue, ClapHost};
use dawsome_host_common::window;
use serde_json::Value;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConfigureNotifyEvent, ConfigureWindowAux, ConnectionExt as _,
    EventMask, MapState, CONFIGURE_NOTIFY_EVENT,
};
use x11rb::protocol::ErrorKind;
use x11rb::rust_connection::RustConnection;

#[path = "../../../tests/support/test_plugins.rs"]
mod test_plugins;

const OWNER: ProcessorOwner = ProcessorOwner::Channel(ChannelId(1));
/// The class the windows of these tests carry (what `DAWSOME_APP_ID` names for the app's).
const CLASS: &str = "dawsome-test";

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

fn file() -> PathBuf {
    test_plugins::test_plugin_gui().join("dawsome_test_gui.clap")
}

/// A catalogue of the GUI file, described in this process (the scan child's work).
fn catalogue() -> Catalogue {
    let file = file();
    let plugins = scan::describe_file(&file, || {}).expect("the GUI file loads");
    let (size, mtime) = catalogue::stamp(&file).expect("the file is there");
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

/// The GUI file's exports, read through the image the host loads (a second `dlopen` of one path
/// is that image).
struct Oracle {
    library: libloading::Library,
}

impl Oracle {
    fn open() -> Self {
        // SAFETY: the test plugin, our own code; opening it again runs no entry (only the host
        // calls `clap_entry.init`).
        let library = unsafe { libloading::Library::new(file()) }.expect("opens");
        Self { library }
    }

    /// The export `name`.
    fn get<T: Copy>(&self, name: &[u8]) -> T {
        // SAFETY: the library exports `name` with the signature `T` names
        // (crates/test-plugin/src/gui.rs). The pointer copied out is used only while `self`
        // holds the library open (every caller calls it at once).
        unsafe { *self.library.get::<T>(name).expect("exported") }
    }

    fn log(&self) -> Vec<String> {
        let read: extern "C" fn(*mut u8, usize) -> usize = self.get(b"dawsome_test_clap_gui_log\0");
        let len = read(std::ptr::null_mut(), 0);
        let mut buffer = vec![0u8; len];
        read(buffer.as_mut_ptr(), len);
        let text = String::from_utf8(buffer).expect("UTF-8");
        text.lines().map(str::to_owned).collect()
    }

    fn clear(&self) {
        let clear: extern "C" fn() = self.get(b"dawsome_test_clap_gui_log_clear\0");
        clear();
    }

    fn window(&self) -> u32 {
        let window: extern "C" fn() -> u32 = self.get(b"dawsome_test_clap_gui_window\0");
        window()
    }

    fn windows(&self) -> i32 {
        let windows: extern "C" fn() -> i32 = self.get(b"dawsome_test_clap_gui_windows\0");
        windows()
    }

    fn fixed(&self, on: bool) {
        let fixed: extern "C" fn(i32) = self.get(b"dawsome_test_clap_gui_fixed\0");
        fixed(i32::from(on));
    }

    /// The interface's own `request_resize` now; its answer.
    fn resize(&self, (w, h): (u32, u32)) -> bool {
        let resize: extern "C" fn(u32, u32) -> i32 = self.get(b"dawsome_test_clap_gui_resize\0");
        resize(w, h) == 1
    }
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

    /// The close button pressed, as a window manager says so.
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

    /// Asks the window manager, from this other client, for `window` at `size`: what
    /// `xdotool windowsize` asks (a ConfigureWindow of the width and height, which the window
    /// manager applies as it would a user's resize and tells the window's own client of).
    fn ask_size(&self, window: u32, (width, height): (u32, u32)) {
        let aux = ConfigureWindowAux::new().width(width).height(height);
        self.conn.configure_window(window, &aux).expect("asks");
        self.conn.flush().expect("flushes");
    }
}

/// Ticks `host` every 16 ms until `done`, for at most a second; the notices its ticks gave.
fn until(host: &ClapHost, what: &str, mut done: impl FnMut() -> bool) -> Vec<HostNotice> {
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
fn settle(host: &ClapHost, ms: u64) -> Vec<HostNotice> {
    let start = Instant::now();
    let mut notices = Vec::new();
    while start.elapsed() < Duration::from_millis(ms) {
        notices.extend(host.tick());
        std::thread::sleep(Duration::from_millis(16));
    }
    notices
}

/// The GUI sine made and its interface open in a window of ours, its own window mapped there.
struct Opened {
    host: ClapHost,
    audio: Option<Box<dyn Processor>>,
    oracle: Oracle,
    x: X,
    /// Our window and the interface's.
    ours: u32,
    child: u32,
}

impl Opened {
    /// The GUI sine made, its interface opened (`fixed`: it says it does not resize), and the
    /// log of the opening.
    fn new(fixed: bool) -> (Self, Vec<String>) {
        window::set_class(CLASS);
        let p29 = p29();
        let oracle = Oracle::open();
        let catalogue = catalogue();
        oracle.fixed(fixed);
        oracle.clear();
        let host = ClapHost::new(catalogue);
        let id = p29["clap"]["id"].as_str().expect("an id");
        let name = p29["clap"]["name"].as_str().expect("a name");
        let reference = ProcessorRef::plugin(format!("clap:{id}"), name);
        let audio = host.create(OWNER, &reference, 48_000).expect("creates");
        oracle.clear();
        assert_eq!(host.open_editor(OWNER, name), Ok(Editor::Embedded));
        let log = oracle.log();
        let child = oracle.window();
        assert_ne!(child, 0, "the interface made its window");
        let x = X::connect();
        let ours = x
            .parent(child)
            .expect("the interface's window has a parent");
        let opened = Self {
            host,
            audio: Some(audio),
            oracle,
            x,
            ours,
            child,
        };
        until(&opened.host, "the interface's window mapped", || {
            opened.x.viewable(opened.child)
        });
        (opened, log)
    }

    /// The test's end: the audio half, then the host (the app quitting with the interface
    /// open); the interface destroyed and both windows gone.
    fn finish(mut self) {
        drop(self.audio.take());
        self.oracle.clear();
        let Opened {
            host,
            x,
            ours,
            child,
            oracle,
            ..
        } = self;
        drop(host);
        assert_eq!(oracle.log(), texts(&p29()["clap"]["close_log"]));
        assert_closed(&x, &oracle, ours, child);
    }
}

/// Our window and the interface's gone (BadWindow on the test's connection), and none of the
/// interface's windows alive, which every test ends with.
fn assert_closed(x: &X, oracle: &Oracle, ours: u32, child: u32) {
    let start = Instant::now();
    while !(x.gone(ours) && x.gone(child)) {
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "a window was left open: ours {ours:#x}, the interface's {child:#x}"
        );
        std::thread::sleep(Duration::from_millis(16));
    }
    assert_eq!(oracle.windows(), 0, "none of the interface's windows alive");
}

/// `until` our window and the interface's are `ours` and `child`, then a settling, after which
/// they still are.
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
            "waited a second for ours {ours:?} and the interface's {child:?}: {now:?}; log {:?}",
            opened.oracle.log()
        );
        std::thread::sleep(Duration::from_millis(16));
    }
    settle(&opened.host, 300);
    assert_eq!(
        (opened.x.size(opened.ours), opened.x.size(opened.child)),
        (Some(ours), Some(child)),
        "after the settling; log {:?}",
        opened.oracle.log()
    );
}

// C1. Opened: Embedded; the interface's calls in the format's order (the open log); its window
// is our window's child, viewable, both 400x300; our window carries the title (both encodings)
// and the class; the host asks for ticks; a second open is Embedded again and calls nothing. The
// close button: the interface destroyed, then our window; a Closed notice.
#[test]
#[ignore = "opens a window: scripts/ui-check.sh runs it in a graphical session"]
fn c1_an_interface_opens_embedded_in_our_window() {
    let _turn = turn();
    let p29 = p29();
    let clap = &p29["clap"];
    let (opened, log) = Opened::new(false);
    assert_eq!(log, texts(&clap["open_log"]));
    let x = &opened.x;
    let want = size(&clap["size"]);
    assert_eq!(x.size(opened.ours), Some(want));
    assert_eq!(x.size(opened.child), Some(want));
    let title = clap["name"].as_str().expect("a name").as_bytes().to_vec();
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
    assert!(opened.host.needs_ticks(), "an open interface is ticked");
    opened.oracle.clear();
    let name = clap["name"].as_str().expect("a name");
    assert_eq!(opened.host.open_editor(OWNER, name), Ok(Editor::Embedded));
    assert!(opened.oracle.log().is_empty(), "nothing called again");
    opened.x.close(opened.ours);
    let notices = until(&opened.host, "the interface closed", || {
        opened.oracle.log() == texts(&clap["close_log"])
    });
    assert!(notices
        .iter()
        .any(|notice| notice.owner == OWNER && notice.kind == NoticeKind::Closed));
    assert_closed(&opened.x, &opened.oracle, opened.ours, opened.child);
    assert!(!opened.host.needs_ticks(), "closed: nothing to tick");
    let Opened { host, audio, .. } = opened;
    drop(audio);
    drop(host);
}

// C2. The plugin's own request_resize: answered true, our window takes the size and the
// interface its own (no adjust_size or set_size); the same size again changes nothing. A fixed
// interface's window keeps its size as its least and most (WM_NORMAL_HINTS), which follow a
// resize it asks.
#[test]
#[ignore = "opens a window: scripts/ui-check.sh runs it in a graphical session"]
fn c2_a_plugins_request_resizes_our_window() {
    let _turn = turn();
    let p29 = p29();
    let clap = &p29["clap"];
    let (opened, _) = Opened::new(false);
    for step in clap["plugin_requests"].as_array().expect("steps") {
        opened.oracle.clear();
        assert!(opened.oracle.resize(size(&step["asked"])), "{step}");
        sized(&opened, size(&step["window"]), size(&step["child"]));
        assert_eq!(opened.oracle.log(), texts(&step["log"]), "{step}");
    }
    opened.finish();
    let fixed = &clap["fixed"];
    let (opened, log) = Opened::new(true);
    assert_eq!(log, texts(&fixed["open_log"]));
    let hints = opened.x.atom("WM_NORMAL_HINTS");
    assert_eq!(
        opened.x.words(opened.ours, hints),
        words(&fixed["hints_before"])
    );
    opened.oracle.clear();
    assert!(opened.oracle.resize(size(&fixed["asked"])));
    sized(&opened, size(&fixed["window"]), size(&fixed["window"]));
    assert_eq!(opened.oracle.log(), texts(&fixed["log"]));
    assert_eq!(
        opened.x.words(opened.ours, hints),
        words(&fixed["hints_after"])
    );
    opened.finish();
}

// C3. A user's resize, as the window manager tells it: held by the interface's adjust_size (to
// its 4:3 inside 200..1600 x 150..1200); a size held to the one kept puts our window back alone
// (Surge XT's case seen in use: dragged wider, 600x300, back to 400x300, nothing told the
// interface); another is set_size and our window's; a fixed interface is not asked.
#[test]
#[ignore = "opens a window: scripts/ui-check.sh runs it in a graphical session"]
fn c3_a_users_resize_is_held_by_the_interface_and_our_window_follows() {
    let _turn = turn();
    let p29 = p29();
    let cases = p29["clap"]["user_resizes"].as_array().expect("cases");
    for fixed in [false, true] {
        let (opened, _) = Opened::new(fixed);
        for case in cases.iter().filter(|case| case["fixed"] == fixed) {
            settle(&opened.host, 200);
            opened.oracle.clear();
            opened.x.configure(opened.ours, size(&case["size"]));
            sized(&opened, size(&case["window"]), size(&case["child"]));
            assert_eq!(opened.oracle.log(), texts(&case["log"]), "{case}");
        }
        opened.finish();
    }
}

// C4. A resize asked of the window manager by another client (what `xdotool windowsize` asks):
// the size it gives comes back to the host as the user's, and our window ends where the
// interface holds it, the interface's window its size: dragged wider than its aspect, back to
// 400x300 with no set_size; to a size it keeps as given; to one it holds smaller, that one. A
// window manager may tell one size twice: the log is the case's, an adjust_size line repeated
// at most.
#[test]
#[ignore = "opens a window: scripts/ui-check.sh runs it in a graphical session"]
fn c4_a_resize_asked_of_the_window_manager_ends_where_the_interface_holds_it() {
    let _turn = turn();
    let p29 = p29();
    let (opened, _) = Opened::new(false);
    for case in p29["clap"]["real_resizes"].as_array().expect("cases") {
        settle(&opened.host, 200);
        opened.oracle.clear();
        opened.x.ask_size(opened.ours, size(&case["size"]));
        // The window manager's answer heard first: before it, our window still has the size
        // the case may end at.
        let heard = texts(&case["log"])[0].clone();
        until(&opened.host, "the window manager's size", || {
            opened.oracle.log().contains(&heard)
        });
        sized(&opened, size(&case["window"]), size(&case["child"]));
        let mut log = opened.oracle.log();
        log.dedup();
        assert_eq!(log, texts(&case["log"]), "{case}");
    }
    opened.finish();
}
