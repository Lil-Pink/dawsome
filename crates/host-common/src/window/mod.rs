//! Plugin windows: top-level windows of ours that a plugin's interface embeds into, X11 windows
//! on Linux and Win32 windows on Windows, for every plugin format's host. Each has the title the
//! app gives, closes with its title bar's close button, and keeps the plugin's size unless the
//! plugin can be resized. How a plugin's interface is put into one is the format's own (its
//! host's crate); the window is the same for all of them.

use std::sync::Mutex;

mod settle;
#[cfg(windows)]
mod win32;
#[cfg(unix)]
mod x11;
#[cfg(unix)]
mod xembed;

pub use settle::{Settle, SETTLE_AFTER, SETTLE_TRIES};
#[cfg(windows)]
pub use win32::{pointer_in, Display, WindowHandle, WindowId, UNAVAILABLE};
#[cfg(unix)]
pub use x11::{pointer_in, Display, WindowHandle, WindowId, UNAVAILABLE};

/// The application id plugin windows opened from now carry as their X11 class.
static CLASS: Mutex<Option<String>> = Mutex::new(None);

/// Gives every plugin window opened from now the X11 class `id` (WM_CLASS "<id>\0<id>\0"): the
/// application id the app's own windows go by (`dawsome`, or `dawsome-test` for the scripted runs
/// and tests that open windows). Why: a desktop's window rule finds a window by its class, and a
/// plugin window without one could not be kept off the user's screen and focus as the app's test
/// windows are. Never called (the command line, tests that set none): no class, as before.
/// Windows has no such class; there it changes nothing.
pub fn set_class(id: &str) {
    if let Ok(mut class) = CLASS.lock() {
        *class = Some(id.to_owned());
    }
}

/// The class [`set_class`] gave, if any.
#[cfg(unix)]
fn class() -> Option<String> {
    CLASS.lock().ok().and_then(|class| class.clone())
}

/// Something the user did to one of our windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowEvent {
    /// The close button was pressed.
    Closed(WindowId),
    /// The window's inside, where the plugin's interface sits, now has this size.
    Resized {
        window: WindowId,
        width: u32,
        height: u32,
    },
}

impl WindowEvent {
    /// The window it happened to.
    pub fn window(self) -> WindowId {
        match self {
            WindowEvent::Closed(window) | WindowEvent::Resized { window, .. } => window,
        }
    }
}

/// The events of `queue` whose window `own` names, taken out in their order; the others stay,
/// in theirs. Why: on Windows every window of a thread queues its messages in one place, and
/// each plugin format's host has a display of its own on that thread, so a display that took the
/// whole queue would take (and drop) another host's windows' events.
#[cfg(any(windows, test))]
fn take_own(queue: &mut Vec<WindowEvent>, own: impl Fn(WindowId) -> bool) -> Vec<WindowEvent> {
    let (mine, others) = std::mem::take(queue)
        .into_iter()
        .partition(|event| own(event.window()));
    *queue = others;
    mine
}

/// A side length both window systems take: 1 to 32767 pixels.
pub fn clamp_side(pixels: u32) -> u16 {
    u16::try_from(pixels.clamp(1, 32_767)).unwrap_or(32_767)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_sides_stay_in_the_range_the_window_systems_take() {
        assert_eq!(clamp_side(0), 1);
        assert_eq!(clamp_side(640), 640);
        assert_eq!(clamp_side(u32::MAX), 32_767);
    }

    // Two displays on one queue: each takes its own windows' events, in their order, and leaves
    // the other's; one display owning every window takes the whole queue, as before.
    #[test]
    fn a_display_takes_only_its_own_windows_events() {
        let resized = |window| WindowEvent::Resized {
            window,
            width: 10,
            height: 20,
        };
        let all = vec![
            WindowEvent::Closed(1),
            resized(2),
            resized(1),
            WindowEvent::Closed(3),
            resized(2),
        ];
        let mut queue = all.clone();
        let first = take_own(&mut queue, |window| window == 1 || window == 3);
        assert_eq!(
            first,
            [WindowEvent::Closed(1), resized(1), WindowEvent::Closed(3)]
        );
        assert_eq!(queue, [resized(2), resized(2)], "the other display's, kept");
        assert_eq!(
            take_own(&mut queue, |window| window == 2),
            [resized(2), resized(2)]
        );
        assert!(queue.is_empty());
        let mut queue = all.clone();
        assert_eq!(
            take_own(&mut queue, |_| true),
            all,
            "one display: everything"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_class_set_is_the_one_windows_get() {
        set_class("dawsome-test");
        assert_eq!(class().as_deref(), Some("dawsome-test"));
    }
}
