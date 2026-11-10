//! On Windows the main window takes the keyboard focus when it is activated, so the keys typed
//! into it never fall to the system's error beep.
//!
//! Why: a user on Windows heard the error beep on every key typed at first open, before any
//! click, and it stopped at the first click in the Library; it was not reproduced (a clean first
//! start on a laptop). On Win32 a key typed while the active window has no focused window inside
//! it is sent to the active window as WM_SYSKEYDOWN, as with Alt held, which the message loop
//! turns into WM_SYSCHAR; the default window procedure answers a WM_SYSCHAR no menu takes with
//! the beep. So the main window's procedure is wrapped, as a guard against that class of beep
//! whatever left the focus unset:
//! 1. Activated (WM_ACTIVATE, its low word not WA_INACTIVE): once the toolkit's procedure has
//!    run, the window is given the focus (SetFocus) when it is on no window, or on one that is
//!    neither the window nor inside it.
//! 2. A WM_SYSCHAR with Alt not held is answered 0, as the toolkit answers the ones it knows, and
//!    reaches nothing else.
//!
//! Everything else goes to the toolkit's procedure unchanged, WM_SYSKEYDOWN and WM_SYSKEYUP too:
//! F10 is a shortcut of ours (Settings) and must reach the toolkit. The wrapper takes itself out
//! as the window goes (WM_NCDESTROY).
//!
//! The wrapping is the classic kind (the window procedure replaced through SetWindowLongPtrW, the
//! toolkit's called through CallWindowProcW for every message). Why not SetWindowSubclass:
//! comctl32 exports it by ordinal before Common Controls 6, which this program's manifest does
//! not ask for (the Windows SDK's own ComCtl32.Lib imports it by ordinal), while windows-sys
//! imports it by name: the program could fail to start. The pop-out windows are not wrapped:
//! each opens from its own place in the app, and the beep was heard in the main window at first
//! open.

/// winuser.h's numbers for the messages and activation states the table reads (the Windows
/// build holds them to windows-sys's below).
const WM_ACTIVATE: u32 = 0x0006;
const WM_SYSCHAR: u32 = 0x0106;
const WA_INACTIVE: usize = 0;

/// Where the keyboard focus is, as the main window's wrapper sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// On no window.
    Nowhere,
    /// On the window itself or a window inside it.
    Ours,
    /// On another window.
    Elsewhere,
}

/// What the wrapper does with a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// The toolkit's procedure answers it.
    Pass,
    /// The toolkit's procedure answers it, then the window is given the focus.
    Focus,
    /// Answered 0; nothing else sees it.
    Swallow,
}

/// What the wrapper does with `message` (`wparam` its word parameter), Alt held or not
/// (`alt_down`), the focus at `focus` (read once the toolkit's procedure has run, for an
/// activation).
pub fn act(message: u32, wparam: usize, alt_down: bool, focus: Focus) -> Act {
    match message {
        WM_ACTIVATE if wparam & 0xFFFF != WA_INACTIVE && focus != Focus::Ours => Act::Focus,
        WM_SYSCHAR if !alt_down => Act::Swallow,
        _ => Act::Pass,
    }
}

#[cfg(windows)]
pub use win32::wrap;

#[cfg(windows)]
mod win32 {
    use std::cell::RefCell;

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetFocus, GetKeyState, SetFocus, VK_MENU,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, IsChild, SetWindowLongPtrW, GWLP_WNDPROC, WM_NCDESTROY, WNDPROC,
    };

    use super::{act, Act, Focus};

    // The table's numbers are winuser.h's, as windows-sys has them.
    const _: () =
        assert!(super::WM_ACTIVATE == windows_sys::Win32::UI::WindowsAndMessaging::WM_ACTIVATE);
    const _: () =
        assert!(super::WM_SYSCHAR == windows_sys::Win32::UI::WindowsAndMessaging::WM_SYSCHAR);
    const _: () = assert!(
        super::WA_INACTIVE as u32 == windows_sys::Win32::UI::WindowsAndMessaging::WA_INACTIVE
    );

    /// The type of a window procedure.
    type Procedure = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

    thread_local! {
        /// The windows wrapped on this thread, each with the toolkit's procedure it replaced.
        static WRAPPED: RefCell<Vec<(isize, WNDPROC)>> = const { RefCell::new(Vec::new()) };
    }

    /// Wraps the main window `window` (its Win32 handle, as the toolkit gave it), once. Called
    /// on the window's own thread, the toolkit's, as the handle arrives.
    pub fn wrap(window: isize) {
        if WRAPPED.with(|wrapped| wrapped.borrow().iter().any(|&(own, _)| own == window)) {
            return;
        }
        let ours: Procedure = procedure;
        // SAFETY: replaces the procedure of a window of this thread (a handle that is gone fails
        // the call and answers 0); the one it answers is kept and called for every message.
        let previous =
            unsafe { SetWindowLongPtrW(window as HWND, GWLP_WNDPROC, ours as usize as isize) };
        if previous == 0 {
            return;
        }
        // SAFETY: a window procedure's value as the system keeps it (a function or the handle
        // CallWindowProcW takes in its place), handed to nothing but CallWindowProcW.
        let previous = Some(unsafe { std::mem::transmute::<isize, Procedure>(previous) });
        WRAPPED.with(|wrapped| wrapped.borrow_mut().push((window, previous)));
    }

    /// The toolkit's procedure for `window`, while it is wrapped.
    fn previous_of(window: HWND) -> Option<WNDPROC> {
        let id = window as isize;
        WRAPPED.with(|wrapped| {
            wrapped
                .borrow()
                .iter()
                .find(|&&(own, _)| own == id)
                .map(|&(_, previous)| previous)
        })
    }

    /// Where the focus is, for `window`.
    fn focus_of(window: HWND) -> Focus {
        // SAFETY: asks this thread which window has the keyboard focus.
        let focus = unsafe { GetFocus() };
        if focus.is_null() {
            Focus::Nowhere
        // SAFETY: asks whether one window is inside another; any handles may be asked.
        } else if focus == window || unsafe { IsChild(window, focus) } != 0 {
            Focus::Ours
        } else {
            Focus::Elsewhere
        }
    }

    /// The wrapped window's procedure (see the module's doc).
    ///
    /// # Safety
    /// Called by the system alone, as the window's procedure, with a message's arguments.
    unsafe extern "system" fn procedure(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let Some(previous) = previous_of(window) else {
            return 0;
        };
        // SAFETY: reads the thread's key state for Alt.
        let alt_down = unsafe { GetKeyState(i32::from(VK_MENU)) } < 0;
        // The focus decides nothing before the toolkit's procedure has run.
        if act(message, wparam, alt_down, Focus::Ours) == Act::Swallow {
            return 0;
        }
        if message == WM_NCDESTROY {
            let id = window as isize;
            let previous_value = previous.map_or(0, |procedure| procedure as usize as isize);
            // SAFETY: gives a window of ours back the procedure it had; it is going.
            unsafe { SetWindowLongPtrW(window, GWLP_WNDPROC, previous_value) };
            WRAPPED.with(|wrapped| wrapped.borrow_mut().retain(|&(own, _)| own != id));
        }
        // SAFETY: the toolkit's procedure for this window, with the arguments the system passed.
        let answer = unsafe { CallWindowProcW(previous, window, message, wparam, lparam) };
        if act(message, wparam, alt_down, focus_of(window)) == Act::Focus {
            // SAFETY: gives the focus to a window of this thread.
            unsafe { SetFocus(window) };
        }
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // winuser.h's numbers by hand: WM_ACTIVATE 0x0006 with WA_INACTIVE 0, WA_ACTIVE 1 and
    // WA_CLICKACTIVE 2 in the low word (the high word non-zero when minimized), WM_KEYDOWN
    // 0x0100, WM_CHAR 0x0102, WM_SYSKEYDOWN 0x0104, WM_SYSKEYUP 0x0105, WM_SYSCHAR 0x0106. An
    // activation with the focus on no window or another's gives the window the focus, one with
    // the focus already ours leaves it; a deactivation never does. A WM_SYSCHAR with Alt up (the
    // beep's case) is swallowed, with Alt held it is the toolkit's (Alt+letter); WM_SYSKEYDOWN
    // and WM_SYSKEYUP always pass (F10, our Settings key, comes as WM_SYSKEYDOWN without Alt),
    // as do the plain keys, whatever the focus.
    #[test]
    fn the_wrappers_table() {
        use Focus::{Elsewhere, Nowhere, Ours};
        let cases = [
            (0x0006, 1, false, Nowhere, Act::Focus),
            (0x0006, 2, false, Elsewhere, Act::Focus),
            (0x0006, 0x0001_0001, false, Nowhere, Act::Focus),
            (0x0006, 1, false, Ours, Act::Pass),
            (0x0006, 0, false, Nowhere, Act::Pass),
            (0x0006, 0x0001_0000, false, Elsewhere, Act::Pass),
            (0x0106, 0x41, false, Nowhere, Act::Swallow),
            (0x0106, 0x41, false, Ours, Act::Swallow),
            (0x0106, 0x41, true, Ours, Act::Pass),
            (0x0104, 0x79, false, Nowhere, Act::Pass),
            (0x0105, 0x79, false, Ours, Act::Pass),
            (0x0104, 0x41, true, Ours, Act::Pass),
            (0x0100, 0x41, false, Nowhere, Act::Pass),
            (0x0102, 0x61, false, Elsewhere, Act::Pass),
        ];
        for (message, wparam, alt, focus, want) in cases {
            assert_eq!(
                act(message, wparam, alt, focus),
                want,
                "{message:#06x} {wparam:#x} alt {alt} {focus:?}"
            );
        }
    }
}
