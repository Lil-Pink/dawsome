//! The desktop drop's door on X11 (XWayland on a Wayland desktop): the app run as a program on
//! the X display, its window found, a file dropped on it over XDND by this test as the other X11
//! client (as a file manager drops one), and what `--drop-monitor` prints of the drop checked:
//! the pointer the app asked of the X server is the one this test asks for that window, in the
//! window's device pixels. It never moves the pointer: wherever the pointer is, the app must say
//! where it is over its window, and where the drop lands follows from that.
//!
//! Ignored in `cargo test`: it needs an X display and opens a window (class `dawsome-test`, so a
//! window rule can keep it off the user's screen). scripts/ui-check.sh runs it.
#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask, PropMode,
    SelectionNotifyEvent, Window, WindowClass, SELECTION_NOTIFY_EVENT,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

/// The app, killed when the test ends however it ends.
struct App(Child);

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A folder of its own holding p8_ui.json, the sounds its channels play and `sounds/tone.wav`,
/// and the per-user folders the app is pointed at, so it reads and writes none of the user's.
fn folder() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dawsome-desktop-drop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sounds")).expect("temp dir");
    for name in ["p8_ui.json", "p8_saw.wav", "impulse.wav"] {
        std::fs::copy(Path::new(FIXTURES).join(name), dir.join(name)).expect("copies");
    }
    let tone = Path::new(FIXTURES).join("browser/tone.wav");
    std::fs::copy(tone, dir.join("sounds/tone.wav")).expect("copies");
    dir
}

/// The app on `dir`'s p8_ui, the playlist alone in a 1000 × 560 window on the X display (the
/// Wayland one unset for it alone), printing each drop; its printed lines come on the channel.
fn start(dir: &Path) -> (App, mpsc::Receiver<String>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dawsome-app"));
    command
        .arg("--load")
        .arg(dir.join("p8_ui.json"))
        .args(["--panels", "playlist", "--window-size", "1000x560"])
        .args(["--quit-after", "60000", "--drop-monitor"])
        .env_remove("WAYLAND_DISPLAY")
        .env("DAWSOME_APP_ID", "dawsome-test")
        .env("ICED_BACKEND", "tiny-skia")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
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

fn atom(conn: &RustConnection, name: &str) -> u32 {
    conn.intern_atom(false, name.as_bytes())
        .expect("asks")
        .reply()
        .expect("an atom")
        .atom
}

/// The app's window: the one under the root whose _NET_WM_PID is `pid`, whose class is
/// `dawsome-test` and which takes drops (XdndAware), once it is there.
fn find_window(conn: &RustConnection, root: Window, pid: u32) -> Option<Window> {
    let pid_atom = atom(conn, "_NET_WM_PID");
    let aware = atom(conn, "XdndAware");
    let mut pending = vec![root];
    while let Some(window) = pending.pop() {
        let property = |name, kind| {
            conn.get_property(false, window, name, kind, 0, 64)
                .ok()?
                .reply()
                .ok()
        };
        let owned = property(pid_atom, u32::from(AtomEnum::CARDINAL))
            .and_then(|reply| reply.value32()?.next())
            == Some(pid);
        let class = property(u32::from(AtomEnum::WM_CLASS), u32::from(AtomEnum::STRING))
            .is_some_and(|reply| reply.value.starts_with(b"dawsome-test\0"));
        let takes_drops =
            property(aware, u32::from(AtomEnum::ATOM)).is_some_and(|reply| reply.value_len > 0);
        if owned && class && takes_drops {
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

/// Where the pointer is over `window`, in its pixels: as the X server says now.
fn pointer_over(conn: &RustConnection, window: Window) -> (i32, i32) {
    let pointer = conn
        .query_pointer(window)
        .expect("asks")
        .reply()
        .expect("the pointer");
    (i32::from(pointer.win_x), i32::from(pointer.win_y))
}

fn send(conn: &RustConnection, to: Window, kind: u32, data: [u32; 5]) {
    let message = ClientMessageEvent::new(32, to, kind, data);
    conn.send_event(false, to, EventMask::NO_EVENT, message)
        .expect("sends");
    conn.flush().expect("flushes");
}

/// Drops `file` on `target` over XDND, as a file manager does: enter, a position (whose
/// coordinates the toolkit does not read), the file's URI handed over when the target asks for
/// it, then the drop, answered by the target's finish.
fn drop_over_xdnd(conn: &RustConnection, root: Window, target: Window, file: &Path) {
    let [selection, enter, position, status, dropped, finished, copy, uri_list] = [
        "XdndSelection",
        "XdndEnter",
        "XdndPosition",
        "XdndStatus",
        "XdndDrop",
        "XdndFinished",
        "XdndActionCopy",
        "text/uri-list",
    ]
    .map(|name| atom(conn, name));
    let source = conn.generate_id().expect("an id");
    conn.create_window(
        COPY_DEPTH_FROM_PARENT,
        source,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new(),
    )
    .expect("creates the source window");
    conn.set_selection_owner(source, selection, CURRENT_TIME)
        .expect("owns the drag's selection");
    send(conn, target, enter, [source, 5 << 24, uri_list, 0, 0]);
    send(conn, target, position, [source, 0, 0, CURRENT_TIME, copy]);
    let uri = format!("file://{}\r\n", file.display());
    let deadline = Instant::now() + Duration::from_secs(10);
    let (mut served, mut accepted) = (false, false);
    while !(served && accepted) {
        assert!(
            Instant::now() < deadline,
            "the app took the drag (served {served}, status {accepted})"
        );
        match conn.poll_for_event().expect("events") {
            Some(Event::SelectionRequest(request)) if request.selection == selection => {
                conn.change_property8(
                    PropMode::REPLACE,
                    request.requestor,
                    request.property,
                    uri_list,
                    uri.as_bytes(),
                )
                .expect("hands over the URI");
                let notify = SelectionNotifyEvent {
                    response_type: SELECTION_NOTIFY_EVENT,
                    sequence: 0,
                    time: request.time,
                    requestor: request.requestor,
                    selection: request.selection,
                    target: request.target,
                    property: request.property,
                };
                conn.send_event(false, request.requestor, EventMask::NO_EVENT, notify)
                    .expect("says so");
                conn.flush().expect("flushes");
                served = true;
            }
            Some(Event::ClientMessage(message)) if message.type_ == status => accepted = true,
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    // The toolkit reads the URI when it is handed over; a moment for it to come in.
    std::thread::sleep(Duration::from_millis(300));
    send(conn, target, dropped, [source, 0, CURRENT_TIME, 0, 0]);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < deadline, "the app finished the drop");
        match conn.poll_for_event().expect("events") {
            Some(Event::ClientMessage(message)) if message.type_ == finished => break,
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    conn.destroy_window(source).expect("destroys");
    conn.flush().expect("flushes");
}

/// The pointer in a `--drop-monitor` line: "pointer X,Y px = ...".
fn printed_pointer(line: &str) -> Option<(i32, i32)> {
    let rest = line.split("pointer ").nth(1)?;
    let (pixels, _) = rest.split_once(" px")?;
    let (x, y) = pixels.split_once(',')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

#[test]
#[ignore = "needs an X display; opens a window (scripts/ui-check.sh runs it)"]
fn a_file_dropped_over_xdnd_lands_where_the_x_server_says_the_pointer_is() {
    let Ok((conn, screen)) = RustConnection::connect(None) else {
        println!("desktop drop: no X display, skipped");
        return;
    };
    let root = conn.setup().roots[screen].root;
    let dir = folder();
    let (app, lines) = start(&dir);
    let deadline = Instant::now() + Duration::from_secs(30);
    let target = loop {
        if let Some(window) = find_window(&conn, root, app.0.id()) {
            break window;
        }
        assert!(Instant::now() < deadline, "the app's window opened");
        std::thread::sleep(Duration::from_millis(100));
    };
    // Its first frames: the playlist reports where it is laid out.
    std::thread::sleep(Duration::from_secs(2));
    let tone = dir.join("sounds/tone.wav");
    let before = pointer_over(&conn, target);
    drop_over_xdnd(&conn, root, target, &tone);
    let deadline = Instant::now() + Duration::from_secs(10);
    let line = loop {
        let line = lines
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("the app prints the drop");
        if line.starts_with("dawsome-app: drop ") {
            break line;
        }
    };
    let after = pointer_over(&conn, target);
    println!("desktop drop: {line}");
    println!("desktop drop: the X server's pointer over the window: {before:?}, then {after:?}");
    let printed = printed_pointer(&line).unwrap_or_else(|| panic!("a pointer asked: {line}"));
    assert!(
        printed == before || printed == after,
        "the app's pointer {printed:?} is the X server's ({before:?}, {after:?})"
    );
    // A clip, when the drop landed on a lane, is printed next.
    if line.contains("lands on the lanes") {
        let clip = lines
            .recv_timeout(Duration::from_secs(5))
            .expect("the clip is printed");
        println!("desktop drop: {clip}");
        assert!(
            clip.starts_with("dawsome-app: drop: clip 3 on track "),
            "{clip}"
        );
    }
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}
