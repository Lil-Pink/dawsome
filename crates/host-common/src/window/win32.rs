//! Plugin windows on Windows: top-level Win32 windows of ours. A window belongs to the thread that
//! made it, here the plugins' main thread (the app's window thread), and that thread's message
//! loop, the toolkit's own, hands the window's messages to [`window_proc`], which queues what the
//! user did for [`Display::events`]. Every display of the thread (one per plugin format's host)
//! shares that queue and takes its own windows' events from it.
//!
//! Keys: a key goes to the window that has the keyboard focus, which is the plugin's own window
//! (a child of ours) once it is clicked, so our window procedure never sees it. While any plugin
//! window is open, a keyboard hook on this thread (WH_KEYBOARD) looks at each key as the message
//! loop takes it from the queue and queues it for our window when the focus is inside one of
//! ours, and always passes it on unchanged. Our window procedure queues the keys it is sent while
//! our window itself has the focus. A plugin may post the keys it does not use to its parent,
//! our window (JUCE's plugins do, Surge XT among them): those come with the focus still on the
//! plugin's window and the hook has them already, so the window procedure leaves them alone.

use std::cell::RefCell;
use std::io;
use std::ptr;

use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{GetStockObject, ScreenToClient, BLACK_BRUSH};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, MapVirtualKeyW, MAPVK_VK_TO_CHAR, MAPVK_VK_TO_VSC_EX,
    MAPVK_VSC_TO_VK_EX, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, GetAncestor, GetClientRect,
    GetCursorPos, GetWindowRect, IsWindow, LoadCursorW, RegisterClassExW, SetWindowPos,
    SetWindowsHookExW, ShowWindow, UnhookWindowsHookEx, CW_USEDEFAULT, GA_ROOT, HC_ACTION, HHOOK,
    HWND_NOTOPMOST, HWND_TOPMOST, IDC_ARROW, SIZE_MINIMIZED, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SW_SHOWNORMAL, WA_INACTIVE, WH_KEYBOARD, WM_ACTIVATE, WM_CLOSE,
    WM_KEYDOWN, WM_KEYUP, WM_NCDESTROY, WM_SIZE, WM_SYSKEYDOWN, WM_SYSKEYUP, WNDCLASSEXW,
    WS_CLIPCHILDREN, WS_MAXIMIZEBOX, WS_OVERLAPPEDWINDOW, WS_THICKFRAME,
};

use super::keys::{typed_of_vk, win32_key};
use super::{clamp_side, take_own, Typed, WindowEvent};

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
    /// This thread's plugin windows, every display's, and the keyboard hook while there is one.
    static KEYS: RefCell<Watch> = const {
        RefCell::new(Watch {
            windows: Vec::new(),
            hook: ptr::null_mut(),
        })
    };
}

/// The plugin windows whose keys the hook looks for, and the hook (null: none installed).
struct Watch {
    windows: Vec<WindowId>,
    hook: HHOOK,
}

/// `window` is a plugin window of this thread from now; the keyboard hook goes in with the first
/// one. A hook the system refuses leaves the window procedure's keys alone (see `window_proc`),
/// and the next window opened asks again.
fn watch_keys(window: WindowId) {
    let unhooked = KEYS.with(|keys| {
        let mut keys = keys.borrow_mut();
        keys.windows.push(window);
        keys.hook.is_null()
    });
    if unhooked {
        // SAFETY: asks for this thread's id.
        let thread = unsafe { GetCurrentThreadId() };
        // SAFETY: a hook for this thread alone, whose procedure is this program's own (so no
        // module is named), removed with this thread's last plugin window.
        let hook =
            unsafe { SetWindowsHookExW(WH_KEYBOARD, Some(keyboard_hook), ptr::null_mut(), thread) };
        KEYS.with(|keys| keys.borrow_mut().hook = hook);
    }
}

/// `window` is gone; the hook goes out with this thread's last plugin window.
fn unwatch_keys(window: WindowId) {
    let hook = KEYS.with(|keys| {
        let mut keys = keys.borrow_mut();
        keys.windows.retain(|&own| own != window);
        if keys.windows.is_empty() {
            std::mem::replace(&mut keys.hook, ptr::null_mut())
        } else {
            ptr::null_mut()
        }
    });
    if !hook.is_null() {
        // SAFETY: the hook `watch_keys` installed, removed once.
        unsafe { UnhookWindowsHookEx(hook) };
    }
}

/// Whether the keyboard hook is in.
fn hooked() -> bool {
    KEYS.with(|keys| !keys.borrow().hook.is_null())
}

/// The plugin window of ours whose inside holds the focus `focus`: the plugin's window or one
/// inside it. None for our window itself (its window procedure hears its keys) and for a window
/// of anyone else's.
fn holding(focus: HWND) -> Option<WindowId> {
    if focus.is_null() {
        return None;
    }
    // SAFETY: asks for a window's top-level ancestor; any handle may be asked.
    let root = unsafe { GetAncestor(focus, GA_ROOT) };
    let id = root as WindowId;
    let ours = root != focus && KEYS.with(|keys| keys.borrow().windows.contains(&id));
    ours.then_some(id)
}

/// The thread's keyboard hook: each keyboard message as the message loop takes it from the queue
/// (HC_ACTION; a message only looked at, HC_NOREMOVE, comes again). One for a window inside a
/// plugin window of ours is queued for that window. Never swallowed: the next hook is always
/// called and its answer given back.
///
/// # Safety
/// Called by the system alone, as this thread's WH_KEYBOARD hook: `wparam` is the key's virtual
/// key, `lparam` its flags.
unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: asks this thread which window has the keyboard focus.
        let focus = unsafe { GetFocus() };
        if let Some(window) = holding(focus) {
            queue_key(window, wparam as u16, lparam, true);
        }
    }
    // SAFETY: the arguments the system passed in; a null hook handle is what the call takes.
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

/// Whether the key of virtual key `vk` is down now, as the thread's input has it.
fn held(vk: u16) -> bool {
    // SAFETY: reads the thread's key state; any virtual key may be asked.
    let state = unsafe { GetKeyState(i32::from(vk)) };
    // The high bit: down.
    state < 0
}

/// Queues the key of a keyboard message (virtual key `vk`, flags `lparam`) for `window`, unless
/// it is the press of a chord with Ctrl, Alt or the logo key (the plugin's shortcut). A message
/// whose device gave no scan code (some media keys) is given the virtual key's, as winit does.
fn queue_key(window: WindowId, vk: u16, lparam: LPARAM, plugin_focused: bool) {
    let key = win32_key(lparam);
    let chord = key.alt || held(VK_MENU) || held(VK_CONTROL) || held(VK_LWIN) || held(VK_RWIN);
    if key.pressed && chord {
        return;
    }
    let scancode = if key.scancode == 0 {
        // SAFETY: maps a virtual key to its scan code; any value may be asked.
        unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC_EX) }
    } else {
        key.scancode
    };
    let event = WindowEvent::Key {
        window,
        scancode,
        pressed: key.pressed,
        repeat: key.repeat,
        plugin_focused,
    };
    EVENTS.with(|events| events.borrow_mut().push(event));
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
        watch_keys(window as WindowId);
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

    /// What the key of `scancode` (as [`WindowEvent::Key`] has it) types on the keyboard layout
    /// of this thread with no modifier held: its virtual key's name, or its character. `None`
    /// for a key that types nothing this knows.
    pub fn typed(&self, scancode: u32) -> Option<Typed> {
        // SAFETY: maps a scan code to its virtual key; any value may be asked.
        let vk = unsafe { MapVirtualKeyW(scancode, MAPVK_VSC_TO_VK_EX) };
        let vk = u16::try_from(vk).ok().filter(|&vk| vk != 0)?;
        // SAFETY: maps a virtual key to the character it types; any value may be asked.
        let character = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_CHAR) };
        typed_of_vk(vk, character)
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

/// What our windows do with their messages: the close button, a new inside size, a key while our
/// window itself has the focus and the window losing the activation are queued for the host's
/// tick; everything else, and every key too, is the system's as before. Why the close button
/// does not close: the host takes the plugin's interface down first and only then destroys the
/// window, as it does on every other way out.
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
        WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP => {
            // Only while our window has the focus itself: with the plugin's window holding it,
            // the hook has the key already, and a key the plugin posted up to us would come
            // twice. Without the hook, every key we are sent.
            // SAFETY: asks this thread which window has the keyboard focus.
            if unsafe { GetFocus() } == window || !hooked() {
                queue_key(window as WindowId, wparam as u16, lparam, false);
            }
            None
        }
        // Clicked away from (another window activated): a key held now goes up elsewhere.
        WM_ACTIVATE if (wparam & 0xFFFF) as u32 == WA_INACTIVE => {
            Some(WindowEvent::KeysLost(window as WindowId))
        }
        // The last message a window gets.
        WM_NCDESTROY => {
            unwatch_keys(window as WindowId);
            None
        }
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
