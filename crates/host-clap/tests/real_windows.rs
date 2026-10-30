//! Surge XT's CLAP interface, the plugin the programs ship (`scripts/fetch-surge.sh`), in a window
//! of ours: opened embedded, its window mapped inside ours, then our window asked by another
//! client for a wider size, and a taller one, as the user drags it (what `xdotool windowsize`
//! asks): each time our window ends at the size of Surge's own window inside it, with no black
//! beside it (the case seen in use, 2026-10-28: dragged wider, the interface stayed at its width with
//! black to its right). The sizes are printed for the record. Skipped with a line when Surge XT is
//! not fetched.
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

use dawsome_engine::processor::{Editor, ProcessorHost};
use dawsome_engine::project::{ChannelId, ProcessorOwner, ProcessorRef};
use dawsome_host_clap::catalogue::{self, FileRecord, FileStatus, PluginFormat};
use dawsome_host_clap::{scan, Catalogue, ClapHost};
use dawsome_host_common::window;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt as _, MapState};
use x11rb::rust_connection::RustConnection;

#[path = "../../../tests/support/test_plugins.rs"]
mod test_plugins;

/// Set in the child [`in_temp_home`] starts.
const CHILD: &str = "DAWSOME_REAL_WINDOW_CHILD";
const OWNER: ProcessorOwner = ProcessorOwner::Channel(ChannelId(1));

/// Whether this is the child that does `test`'s work: else the test runs again in a child of this
/// binary with temp HOME and XDG folders, whose result is this one's.
fn in_temp_home(test: &str) -> bool {
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
    assert_eq!(host.open_editor(OWNER, "Surge XT"), Ok(Editor::Embedded));
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
