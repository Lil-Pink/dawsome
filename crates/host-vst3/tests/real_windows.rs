//! Real VST3 plugins' editors in windows of ours: Surge XT 1.3.4's
//! VST3 (JUCE 7, `scripts/fetch-surge.sh --vst3`), Dexed (JUCE), JS80P (its own toolkit) and
//! DPF-Plugins' 3-Band EQ (DPF, which needs its frame's run loop) as `scripts/fetch-vst3-test-
//! plugins.sh` pins them, and LSP's Limiter Stereo from the system's VST3 folder. Each: its editor
//! opens embedded, a window of its is mapped inside ours within 2 s, three times opened and closed
//! by the close button, then the host goes with it open (the app quitting), with no crash. What
//! each is (an XEmbed client or not, what it registers with the frame's run loop) is printed for
//! the record. A plugin that is not there is skipped with a line.
//!
//! Ignored: they open windows, so `scripts/ui-check.sh` runs each alone in a graphical session.
//! The plugins run in this process and read and make user folders, so each runs again in a child
//! of this test binary whose HOME and XDG folders are temp ones ([`in_temp_home`]). Never
//! `/usr/lib/vst3/Surge XT.vst3`: the fetched copy alone.

#![cfg(target_os = "linux")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use dawsome_engine::processor::{Editor, HostNotice, NoticeKind, ProcessorHost};
use dawsome_engine::project::{ChannelId, EffectId, ProcessorOwner, ProcessorRef};
use dawsome_host_common::catalogue::{
    self, Catalogue, FileRecord, FileStatus, GuiSupport, PluginFormat, PluginKind, PluginRecord,
};
use dawsome_host_common::window;
use dawsome_host_vst3::{scan, Vst3Host};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, MapState,
};
use x11rb::rust_connection::RustConnection;

#[path = "../../../tests/support/test_plugins.rs"]
mod test_plugins;

/// Set in the child [`in_temp_home`] starts.
const CHILD: &str = "DAWSOME_REAL_WINDOW_CHILD";

/// Whether this is the child that does `test`'s work (see real_plugins.rs's twin).
fn in_temp_home(test: &str) -> bool {
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("vst3-real-windows")
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
        .env_remove("VST3_PATH")
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

fn record_of(module: &Path, plugins: Vec<PluginRecord>) -> Catalogue {
    let (size, mtime) =
        catalogue::module_stamp(PluginFormat::Vst3, module).expect("the module is there");
    Catalogue {
        files: vec![FileRecord {
            file: module.to_owned(),
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

/// A catalogue of `module`, described in this process (the scan child's work).
fn described(module: &Path) -> Catalogue {
    let plugins = scan::describe_module(module, || {}).expect("the module loads");
    record_of(module, plugins)
}

struct X {
    conn: RustConnection,
    wm_protocols: u32,
    wm_delete_window: u32,
    xembed_info: u32,
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
        let (wm_protocols, wm_delete_window, xembed_info) = (
            intern(b"WM_PROTOCOLS"),
            intern(b"WM_DELETE_WINDOW"),
            intern(b"_XEMBED_INFO"),
        );
        Self {
            conn,
            wm_protocols,
            wm_delete_window,
            xembed_info,
        }
    }

    /// The children of `window` that are viewable.
    fn mapped_children(&self, window: u32) -> Vec<u32> {
        let Some(tree) = self
            .conn
            .query_tree(window)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
        else {
            return Vec::new();
        };
        tree.children
            .into_iter()
            .filter(|&child| {
                self.conn
                    .get_window_attributes(child)
                    .ok()
                    .and_then(|cookie| cookie.reply().ok())
                    .is_some_and(|attributes| attributes.map_state == MapState::VIEWABLE)
            })
            .collect()
    }

    /// Whether `child` asks to be an XEmbed client.
    fn xembed_client(&self, child: u32) -> bool {
        self.conn
            .get_property(false, child, self.xembed_info, AtomEnum::ANY, 0, 2)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|reply| reply.value_len > 0)
    }

    fn gone(&self, window: u32) -> bool {
        self.conn
            .get_window_attributes(window)
            .ok()
            .is_some_and(|cookie| cookie.reply().is_err())
    }

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
}

/// Ticks `host` every 16 ms until `done`, for at most `seconds`; the notices its ticks gave.
fn until(
    host: &Vst3Host,
    seconds: u64,
    what: &str,
    mut done: impl FnMut() -> bool,
) -> Vec<HostNotice> {
    let start = Instant::now();
    let mut notices = Vec::new();
    loop {
        notices.extend(host.tick());
        if done() {
            return notices;
        }
        assert!(
            start.elapsed() < Duration::from_secs(seconds),
            "waited {seconds} s for {what}"
        );
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// `plugin` of `catalogue` (`kind`): its editor three times opened, mapped inside ours and closed
/// by the close button, then opened once more and the host dropped with it open.
fn three_cycles_and_quit(catalogue: Catalogue, name: &str, kind: PluginKind) {
    window::set_class("dawsome-test");
    let record = catalogue.files[0]
        .plugins
        .iter()
        .find(|plugin| plugin.name == name)
        .unwrap_or_else(|| panic!("{name} in its module"))
        .clone();
    let owner = match kind {
        PluginKind::Instrument => ProcessorOwner::Channel(ChannelId(1)),
        PluginKind::Effect => ProcessorOwner::Effect(EffectId(1)),
    };
    let x = X::connect();
    let host = Vst3Host::new(catalogue);
    let reference = ProcessorRef::plugin(format!("vst3:{}", record.id), &record.name);
    let audio = host.create(owner, &reference, 48_000).expect("creates");
    let mut windows = Vec::new();
    for cycle in 1..=4 {
        let opened = Instant::now();
        assert_eq!(
            host.open_editor(owner, name),
            Ok(Editor::Embedded),
            "{name}, cycle {cycle}"
        );
        let ours = host.editor(owner).expect("open").window;
        let mut children = Vec::new();
        until(&host, 2, &format!("{name}'s window mapped in ours"), || {
            children = x.mapped_children(ours);
            !children.is_empty()
        });
        let shown = opened.elapsed();
        settle(&host, 300);
        let editor = host.editor(owner).expect("still open");
        let clients: Vec<bool> = children
            .iter()
            .map(|&child| x.xembed_client(child))
            .collect();
        println!(
            "{name}, cycle {cycle}: mapped in {} ms, its windows in ours {children:#x?}, XEmbed \
             client {clients:?}, the frame's run loop: {} timers, {} file descriptors",
            shown.as_millis(),
            editor.timers,
            editor.fds
        );
        windows.push(ours);
        windows.extend(children);
        if cycle == 4 {
            break;
        }
        x.close(ours);
        let notices = until(&host, 2, &format!("{name}'s window closed"), || {
            host.editor(owner).is_none()
        });
        assert!(
            notices
                .iter()
                .any(|notice| notice.owner == owner && notice.kind == NoticeKind::Closed),
            "{name}: closed by its close button"
        );
    }
    // The app quitting with the editor open: the audio half goes, then the host.
    drop(audio);
    drop(host);
    let start = Instant::now();
    while !windows.iter().all(|&window| x.gone(window)) {
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{name}: a window was left open"
        );
        std::thread::sleep(Duration::from_millis(16));
    }
    println!("{name}: quit with its editor open, every window gone");
}

fn settle(host: &Vst3Host, ms: u64) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(ms) {
        host.tick();
        std::thread::sleep(Duration::from_millis(16));
    }
}

fn fetched(folder: &str, binary: &str, fetch: &str) -> Option<PathBuf> {
    let found = test_plugins::fetched(folder, binary);
    if found.is_none() {
        println!("skipped: {folder} is not fetched ({fetch})");
    }
    found
}

#[test]
#[ignore = "opens a plugin's window: scripts/ui-check.sh runs it in a graphical session"]
fn surge_xt_opens_its_editor() {
    let Some(dir) = fetched(
        "surge/linux-x86_64-vst3",
        "Surge XT.vst3/Contents/x86_64-linux/Surge XT.so",
        "scripts/fetch-surge.sh --vst3",
    ) else {
        return;
    };
    if !in_temp_home("surge_xt_opens_its_editor") {
        return;
    }
    let module = dir.join("Surge XT.vst3");
    three_cycles_and_quit(described(&module), "Surge XT", PluginKind::Instrument);
}

#[test]
#[ignore = "opens a plugin's window: scripts/ui-check.sh runs it in a graphical session"]
fn dexed_opens_its_editor() {
    let Some(dir) = fetched(
        "vst3-test/dexed",
        "Dexed.vst3/Contents/x86_64-linux/Dexed.so",
        "scripts/fetch-vst3-test-plugins.sh",
    ) else {
        return;
    };
    if !in_temp_home("dexed_opens_its_editor") {
        return;
    }
    three_cycles_and_quit(
        described(&dir.join("Dexed.vst3")),
        "Dexed",
        PluginKind::Instrument,
    );
}

#[test]
#[ignore = "opens a plugin's window: scripts/ui-check.sh runs it in a graphical session"]
fn js80p_opens_its_editor() {
    let Some(dir) = fetched(
        "vst3-test/js80p",
        "js80p.vst3/Contents/x86_64-linux/js80p.so",
        "scripts/fetch-vst3-test-plugins.sh",
    ) else {
        return;
    };
    if !in_temp_home("js80p_opens_its_editor") {
        return;
    }
    three_cycles_and_quit(
        described(&dir.join("js80p.vst3")),
        "JS80P",
        PluginKind::Instrument,
    );
}

#[test]
#[ignore = "opens a plugin's window: scripts/ui-check.sh runs it in a graphical session"]
fn dpf_3_band_eq_opens_its_editor() {
    let Some(dir) = fetched(
        "vst3-test/dpf",
        "3BandEQ.vst3/Contents/x86_64-linux/3BandEQ.so",
        "scripts/fetch-vst3-test-plugins.sh",
    ) else {
        return;
    };
    if !in_temp_home("dpf_3_band_eq_opens_its_editor") {
        return;
    }
    let catalogue = described(&dir.join("3BandEQ.vst3"));
    let name = catalogue.files[0].plugins[0].name.clone();
    three_cycles_and_quit(catalogue, &name, PluginKind::Effect);
}

// LSP's 198 classes take some 8 s to describe, so the Limiter Stereo's record is written here as
// its moduleinfo.json names it (its id and name; the host loads the module by its stamp).
#[test]
#[ignore = "opens a plugin's window: scripts/ui-check.sh runs it in a graphical session"]
fn lsp_limiter_stereo_opens_its_editor() {
    let module = Path::new("/usr/lib/vst3/lsp-plugins.vst3");
    if !module
        .join("Contents/x86_64-linux/lsp-plugins.so")
        .is_file()
    {
        println!("skipped: LSP's VST3 plugins are not installed in /usr/lib/vst3");
        return;
    }
    if !in_temp_home("lsp_limiter_stereo_opens_its_editor") {
        return;
    }
    let limiter = PluginRecord {
        id: "64737020623173202020202072667563".to_owned(),
        name: "Limiter Stereo".to_owned(),
        vendor: "LSP VST3".to_owned(),
        version: "1.0.37".to_owned(),
        kind: PluginKind::Effect,
        gui: GuiSupport::None,
        params: 0,
    };
    three_cycles_and_quit(
        record_of(module, vec![limiter]),
        "Limiter Stereo",
        PluginKind::Effect,
    );
}
