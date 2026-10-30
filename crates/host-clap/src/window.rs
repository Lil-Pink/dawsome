//! Plugin windows: host-common's (`dawsome_host_common::window`), which every plugin format's
//! host shares. What is CLAP's own is here: the window API a plugin's interface is asked to use,
//! and our window as the parent it embeds into.

#[cfg(unix)]
use std::ffi::c_ulong;
#[cfg(windows)]
use std::ffi::c_void;

use clack_extensions::gui::{GuiApiType, Window};

pub(crate) use dawsome_host_common::window::{Display, WindowEvent, WindowId, UNAVAILABLE};

/// The window API plugin interfaces are asked to use.
#[cfg(unix)]
pub(crate) const API: GuiApiType<'static> = GuiApiType::X11;
#[cfg(windows)]
pub(crate) const API: GuiApiType<'static> = GuiApiType::WIN32;

/// `window` as the parent a plugin's interface is embedded into.
#[cfg(unix)]
pub(crate) fn as_parent(window: WindowId) -> Window<'static, 'static> {
    Window::from_x11_handle(c_ulong::from(window))
}

/// `window` as the parent a plugin's interface is embedded into.
#[cfg(windows)]
pub(crate) fn as_parent(window: WindowId) -> Window<'static, 'static> {
    // SAFETY: the handle reaches the plugin only through `set_parent`, whose caller keeps the
    // window until the plugin's interface is destroyed (`ClapHost::open` and `close`).
    unsafe { Window::from_win32_hwnd(window as *mut c_void) }
}
