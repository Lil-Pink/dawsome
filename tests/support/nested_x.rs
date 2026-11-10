//! An X server of the test's own, nested (Xvfb, else Xephyr), for the tests that type as a
//! keyboard does (XTEST, which makes the X server generate real device events and so raw key
//! events, which XSendEvent cannot). Keys are never typed so on the desktop's own display. Why:
//! a key typed so lands in whatever window has the focus, and under a Wayland desktop Xwayland
//! hands XTEST to the compositor, which asks the person at the desk for remote control (KDE
//! Plasma does). Included as a module by the tests that type.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// The nested server, stopped when dropped.
pub struct NestedX {
    child: Child,
    /// Its display, ":<number>", for `DISPLAY`.
    pub display: String,
}

impl Drop for NestedX {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `name` in a folder of `PATH`.
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|file| file.is_file())
}

impl NestedX {
    /// A nested server taking clients, or why there is none (the test skips with it).
    pub fn start() -> Result<Self, String> {
        let (program, args): (PathBuf, &[&str]) = if let Some(xvfb) = on_path("Xvfb") {
            (xvfb, &["-screen", "0", "1280x800x24"])
        } else if let Some(xephyr) = on_path("Xephyr") {
            // A window on the desktop: it never grabs the desktop's keyboard.
            (xephyr, &["-screen", "1280x800", "-no-host-grab"])
        } else {
            return Err("no nested X server (Xvfb or Xephyr) on PATH".into());
        };
        // -displayfd 1: the server takes a free display and writes its number to its standard
        // output once it takes clients.
        let mut child = Command::new(&program)
            .args(args)
            .args(["-displayfd", "1", "-nolisten", "tcp"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("{} does not start: {error}", program.display()))?;
        let stdout = child.stdout.take().expect("piped");
        let (number, received) = mpsc::channel();
        std::thread::spawn(move || {
            let mut lines = BufReader::new(stdout).lines();
            let first = lines.next().and_then(Result::ok).unwrap_or_default();
            let _ = number.send(first);
            // Read to its end, so the server never writes into a closed pipe.
            for _ in lines {}
        });
        let line = received
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_default();
        let number = line.trim();
        if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{} gave no display", program.display()));
        }
        Ok(Self {
            child,
            display: format!(":{number}"),
        })
    }
}
