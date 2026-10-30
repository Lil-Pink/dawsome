//! Plugin windows on Windows: top-level Win32 windows of ours. A window belongs to the thread that
//! made it, here the plugins' main thread (the app's window thread), and that thread's message
//! loop, the toolkit's own, hands the window's messages to [`window_proc`], which queues what the
//! user did for [`Display::events`]. Every display of the thread (one per plugin format's host)
//! shares that queue and takes its own windows' events from it.

use std::cell::RefCell;
use std::io;
use std::ptr;

use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{GetStockObject, ScreenToClient, BLACK_BRUSH};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, GetWindowRect,
    IsWindow, LoadCursorW, RegisterClassExW, SetWindowPos, ShowWindow, CW_USEDEFAULT,
    HWND_NOTOPMOST, HWND_TOPMOST, IDC_ARROW, SIZE_MINIMIZED, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SW_SHOWNORMAL, WM_CLOSE, WM_SIZE, WNDCLASSEXW, WS_CLIPCHILDREN,
    WS_MAXIMIZEBOX, WS_OVERLAPPEDWINDOW, WS_THICKFRAME,
};

use super::{clamp_side, take_own, WindowEvent};

/// What shows instead of a plugin window when no window of ours can be made.
pub const UNAVAILABLE: &str = "Plugin windows cannot be opened";

/// A window handle (`HWND`), kept as a number.
pub type WindowId = isize;

/// Where the pointer is over `window` (the app's own main window, for one), asked of the system
/// now: its pixels from the top-left corner of the window's inside, which lie outside it when the
/// pointer does. `None` when the system cannot say (a window that is gone, a secure desktop).
pub fn pointer_in(window: WindowId) -> Option<(i32, i32)> {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: `point` is ours to write.
    let found = unsafe { GetCursorPos(&mut point) } != 0
        // SAFETY: as above; any handle may be passed, a window that is gone fails the call.
        && unsafe { ScreenToClient(window as HWND, &mut point) } != 0;
    found.then_some((point.x, point.y))
}

/// The name our plugin windows' class is registered under, once per process.
const CLASS: &str = "DawsomePluginWindow";

thread_local! {
    /// What the user did to this thread's plugin windows since [`Display::events`] last looked.
    static EVENTS: RefCell<Vec<WindowEvent>> = const { RefCell::new(Vec::new()) };
}

/// The window class plugin windows are made from, and the windows made from it here.
pub struct Display {
    class: Vec<u16>,
    module: HINSTANCE,
    /// The windows this display opened: whose events [`Display::events`] takes.
    own: RefCell<Vec<WindowId>>,
}

impl Display {
    /// Registers the window class (once per process); fails with the reason to show.
    pub fn connect() -> Result<Self, String> {
        let class = wide(CLASS);
        // SAFETY: a null name asks for this program's own module, which lives as long as the
        // process does.
        let module = unsafe { GetModuleHandleW(ptr::null()) };
        let description = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: module,
            // SAFETY: a null module and a predefined id load one of the system's cursors.
            hCursor: unsafe { LoadCursorW(ptr::null_mut(), IDC_ARROW) },
            // SAFETY: stock objects belong to the system and are never freed.
            hbrBackground: unsafe { GetStockObject(BLACK_BRUSH) },
            lpszClassName: class.as_ptr(),
            ..WNDCLASSEXW::default()
        };
        // SAFETY: `description` and the name it points to live through the call, which copies
        // what it needs.
        let registered = unsafe { RegisterClassExW(&description) } != 0;
        // SAFETY: reads this thread's last error, set by the call above.
        if !registered && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS {
            return Err(format!("{UNAVAILABLE}: {}", io::Error::last_os_error()));
        }
        Ok(Self {
            class,
            module,
            own: RefCell::new(Vec::new()),
        })
    }

    /// Opens a window titled `title` whose inside is `width` by `height` pixels. A window that
    /// is not `resizable` has no sizing border and no maximize button.
    pub fn open(
        &self,
        title: &str,
        width: u32,
        height: u32,
        resizable: bool,
    ) -> Result<WindowId, String> {
        let frame = if resizable {
            WS_OVERLAPPEDWINDOW
        } else {
            WS_OVERLAPPEDWINDOW & !(WS_THICKFRAME | WS_MAXIMIZEBOX)
        };
        // The plugin's interface is a child window that paints itself; ours must not paint
        // over it.
        let style = frame | WS_CLIPCHILDREN;
        let title = wide(title);
        // SAFETY: the class is registered, both names are NUL-terminated and live through the
        // call, and a top-level window takes null parent, menu and creation data.
        let window = unsafe {
            CreateWindowExW(
                0,
                self.class.as_ptr(),
                title.as_ptr(),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                i32::from(clamp_side(width)),
                i32::from(clamp_side(height)),
                ptr::null_mut(),
                ptr::null_mut(),
                self.module,
                ptr::null(),
            )
        };
        if window.is_null() {
            return Err(format!(
                "cannot open the plugin window: {}",
                io::Error::last_os_error()
            ));
        }
        self.own.borrow_mut().push(window as WindowId);
        fit_inside(window, width, height);
        // SAFETY: `window` is a live window of ours.
        unsafe { ShowWindow(window, SW_SHOWNORMAL) };
        Ok(window as WindowId)
    }

    /// [`open`](Self::open): a plugin's view is a child window of ours on Windows, with no
    /// protocol between the two windows to keep.
    pub fn open_embedder(
        &self,
        title: &str,
        width: u32,
        height: u32,
        resizable: bool,
    ) -> Result<WindowId, String> {
        self.open(title, width, height, resizable)
    }

    /// `window`, for holding apart from the display.
    pub fn handle(&self, window: WindowId) -> WindowHandle {
        WindowHandle { window }
    }

    /// Gives `window`'s inside the size a plugin asked for.
    pub fn resize(&self, window: WindowId, width: u32, height: u32) {
        fit_inside(window as HWND, width, height);
    }

    pub fn close(&self, window: WindowId) {
        self.own.borrow_mut().retain(|&own| own != window);
        destroy(window);
    }

    /// Keeps `window` above other windows (`above`: the topmost band), or stacks it as any
    /// window, without moving, sizing or activating it.
    pub fn set_above(&self, window: WindowId, above: bool) {
        let after = if above { HWND_TOPMOST } else { HWND_NOTOPMOST };
        // SAFETY: restacks a window of ours; a window that is gone fails the call.
        unsafe {
            SetWindowPos(
                window as HWND,
                after,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        };
    }

    /// What happened to this display's windows since the last call; another display's events
    /// stay queued for it.
    pub fn events(&self) -> Vec<WindowEvent> {
        // A window closed another way than `close` is gone from the list too. Asked outside every
        // borrow, as the queue's rule is (see `window_proc`).
        let alive: Vec<WindowId> = self
            .own
            .borrow()
            .iter()
            .copied()
            .filter(|&window| {
                // SAFETY: asks whether a handle names a window; any value may be asked.
                unsafe { IsWindow(window as HWND) != 0 }
            })
            .collect();
        *self.own.borrow_mut() = alive.clone();
        EVENTS.with(|events| take_own(&mut events.borrow_mut(), |window| alive.contains(&window)))
    }
}

/// One window of ours, for code that holds no display (a plugin's frame, called by the plugin,
/// and the view it closes as it goes): its size and its closing. A window handle is a number; the
/// calls are the window thread's, as every plugin call is.
#[derive(Debug, Clone)]
pub struct WindowHandle {
    window: WindowId,
}

impl WindowHandle {
    pub fn id(&self) -> WindowId {
        self.window
    }

    /// Gives the window's inside the size its plugin asked for.
    pub fn resize(&self, width: u32, height: u32) {
        fit_inside(self.window as HWND, width, height);
    }

    /// The same as [`resize`](Self::resize): a window that keeps its size has no sizing border
    /// on Windows, so nothing else holds it.
    pub fn fix(&self, width: u32, height: u32) {
        self.resize(width, height);
    }

    pub fn close(&self) {
        destroy(self.window);
    }
}

/// Destroys `window`, and what it queued that no display has taken yet goes with it.
fn destroy(window: WindowId) {
    // A window that is already gone has nothing to destroy.
    // SAFETY: destroying a window of ours, or failing on a handle that is gone.
    unsafe { DestroyWindow(window as HWND) };
    EVENTS.with(|events| events.borrow_mut().retain(|event| event.window() != window));
}

/// Sizes `window` so that its inside is `width` by `height`. Why measured rather than computed
/// from the style: the frame around the inside is whatever the theme and the screen's scale make
/// it, and the window's outer size less its inside is exactly that.
fn fit_inside(window: HWND, width: u32, height: u32) {
    let (mut outer, mut inner) = (RECT::default(), RECT::default());
    // SAFETY: `outer` is ours to write; any handle may be passed, a window that is gone fails
    // the call.
    let measured = unsafe { GetWindowRect(window, &mut outer) } != 0
        // SAFETY: as above, for `inner`.
        && unsafe { GetClientRect(window, &mut inner) } != 0;
    if !measured {
        return;
    }
    let frame_width = (outer.right - outer.left) - (inner.right - inner.left);
    let frame_height = (outer.bottom - outer.top) - (inner.bottom - inner.top);
    // SAFETY: moves nothing but the size of a window of ours; a window that is gone fails it.
    unsafe {
        SetWindowPos(
            window,
            ptr::null_mut(),
            0,
            0,
            i32::from(clamp_side(width)) + frame_width,
            i32::from(clamp_side(height)) + frame_height,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
}

/// What our windows do with their messages: the close button and a new inside size are queued
/// for the host's tick, everything else is the system's. Why the close button does not close:
/// the host takes the plugin's interface down first and only then destroys the window, as it
/// does on every other way out.
///
/// # Safety
/// Called by the system alone (it is our class's `lpfnWndProc`), with the arguments of a message
/// for `window`: `DefWindowProcW` reads pointers some messages carry in `lparam`.
unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let event = match message {
        WM_CLOSE => Some(WindowEvent::Closed(window as WindowId)),
        // A minimized window has no inside to fit the plugin to.
        WM_SIZE if wparam != SIZE_MINIMIZED as WPARAM => Some(WindowEvent::Resized {
            window: window as WindowId,
            // The new inside's width and height are the low and high 16 bits.
            width: (lparam & 0xFFFF) as u32,
            height: ((lparam >> 16) & 0xFFFF) as u32,
        }),
        _ => None,
    };
    if let Some(event) = event {
        // The queue is borrowed only here and in `events`, never across a call into the
        // system, so a message sent while one of our calls runs finds it free.
        EVENTS.with(|events| events.borrow_mut().push(event));
        if message == WM_CLOSE {
            return 0;
        }
    }
    // SAFETY: the arguments are the ones the system passed in (see `# Safety`).
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// `text` as a NUL-terminated UTF-16 string.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
