//! The GUI file's interface (Linux): CLAP's `gui` extension, embedded in an X11 window of the
//! host's, logging every call the host makes into it. Its own window is a child of the host's of
//! its size, on an X connection of its own, mapped when it is shown. It is 400x300 until it is
//! given another size; it says it can be resized unless `dawsome_test_clap_gui_fixed(1)`; its
//! `adjust_size` holds a size to 200..1600 x 150..1200, then to the largest 4:3 size inside that,
//! rounded down (an interface that holds its aspect, as Surge XT's does). Known answers:
//! tests/fixtures/gen_p29_expected.py, its "clap" key.
//!
//! The tests reach it through the library's exports (`dawsome_test_clap_gui_*`): its log, its
//! window, its X windows alive, whether it says it resizes, and a `request_resize` of its own now.
//! One interface at a time: the exports reach the last one created on the calling thread, the
//! host's main thread, which is where the host makes every call into it.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Mutex;

use clack_extensions::gui::{
    GuiApiType, GuiConfiguration, GuiSize, HostGui, PluginGuiImpl, Window,
};
use clack_plugin::host::HostSharedHandle;
use clack_plugin::prelude::PluginError;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, WindowClass,
};
use x11rb::rust_connection::RustConnection;

use crate::MainThread;

/// Its size until it is given another.
const SIZE: (u32, u32) = (400, 300);
/// What `adjust_size` holds a size to, before its aspect.
const LEAST: (u32, u32) = (200, 150);
const MOST: (u32, u32) = (1600, 1200);

/// Every call the host made into it, in turn.
static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// `can_resize` answers no.
static FIXED: AtomicBool = AtomicBool::new(false);
/// Its X windows alive.
static WINDOWS: AtomicI32 = AtomicI32::new(0);

thread_local! {
    /// The interface created last on this thread, for the exports.
    static CURRENT: RefCell<Weak<RefCell<Interface>>> = const { RefCell::new(Weak::new()) };
}

fn log(line: impl Into<String>) {
    if let Ok(mut log) = LOG.lock() {
        log.push(line.into());
    }
}

fn size_text((w, h): (u32, u32)) -> String {
    format!("{w}x{h}")
}

/// A side as a window takes it: 1 to 32767 pixels.
fn side(pixels: u32) -> u16 {
    u16::try_from(pixels.clamp(1, 32_767)).unwrap_or(32_767)
}

/// `adjust_size`'s rule: each side held to [`LEAST`]..[`MOST`], then the largest 4:3 size inside.
fn adjusted((w, h): (u32, u32)) -> (u32, u32) {
    let (w, h) = (w.clamp(LEAST.0, MOST.0), h.clamp(LEAST.1, MOST.1));
    if u64::from(w) * 3 > u64::from(h) * 4 {
        (h * 4 / 3, h)
    } else {
        (w, w * 3 / 4)
    }
}

/// Its window inside the host's, once it has a parent.
struct Own {
    conn: RustConnection,
    window: u32,
}

impl Own {
    fn make(parent: u32, (w, h): (u32, u32)) -> Option<Self> {
        let (conn, _) = RustConnection::connect(None).ok()?;
        let window = conn.generate_id().ok()?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            parent,
            0,
            0,
            side(w),
            side(h),
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new(),
        )
        .ok()?;
        WINDOWS.fetch_add(1, Ordering::AcqRel);
        conn.flush().ok()?;
        Some(Self { conn, window })
    }

    fn resize(&self, (w, h): (u32, u32)) {
        let aux = ConfigureWindowAux::new()
            .width(u32::from(side(w)))
            .height(u32::from(side(h)));
        let _ = self.conn.configure_window(self.window, &aux);
        let _ = self.conn.flush();
    }

    fn map(&self, shown: bool) {
        let _ = if shown {
            self.conn.map_window(self.window)
        } else {
            self.conn.unmap_window(self.window)
        };
        let _ = self.conn.flush();
    }
}

impl Drop for Own {
    fn drop(&mut self) {
        let _ = self.conn.destroy_window(self.window);
        let _ = self.conn.flush();
        WINDOWS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// One instance's interface.
pub(crate) struct Interface {
    /// Its host, for its own `request_resize`. Why a lifetime of its own: the exports reach it
    /// through a thread-local; it is used only while the instance lives (the instance's drop
    /// takes the interface, and with it this handle, away first).
    host: HostSharedHandle<'static>,
    size: (u32, u32),
    own: Option<Own>,
}

/// The interface an instance keeps, shared with the exports while it is created.
pub(crate) struct Gui {
    interface: Rc<RefCell<Interface>>,
}

impl Gui {
    pub(crate) fn new(host: HostSharedHandle<'_>) -> Self {
        // SAFETY: the handle is kept by the instance's own state and by the exports through a
        // `Weak` only, both gone once the instance is (see `Interface::host`).
        let host = unsafe { host.with_arbitrary_lifetime() };
        Self {
            interface: Rc::new(RefCell::new(Interface {
                host,
                size: SIZE,
                own: None,
            })),
        }
    }
}

impl PluginGuiImpl for MainThread<'_> {
    fn is_api_supported(&self, configuration: GuiConfiguration) -> bool {
        log(format!(
            "is_api_supported {} {}",
            configuration.api_type.0.to_string_lossy(),
            if configuration.is_floating {
                "floating"
            } else {
                "embedded"
            }
        ));
        configuration.api_type == GuiApiType::X11 && !configuration.is_floating
    }

    fn get_preferred_api(&self) -> Option<GuiConfiguration<'_>> {
        Some(GuiConfiguration {
            api_type: GuiApiType::X11,
            is_floating: false,
        })
    }

    fn create(&self, configuration: GuiConfiguration) -> Result<(), PluginError> {
        log(format!(
            "create {} {}",
            configuration.api_type.0.to_string_lossy(),
            if configuration.is_floating {
                "floating"
            } else {
                "embedded"
            }
        ));
        let Some(gui) = &self.gui else {
            return Err(PluginError::Message("no interface"));
        };
        if configuration.api_type != GuiApiType::X11 || configuration.is_floating {
            return Err(PluginError::Message("X11 embedded only"));
        }
        CURRENT.with(|current| *current.borrow_mut() = Rc::downgrade(&gui.interface));
        Ok(())
    }

    fn destroy(&self) {
        log("destroy");
        if let Some(gui) = &self.gui {
            let mut interface = gui.interface.borrow_mut();
            interface.own = None;
            interface.size = SIZE;
        }
    }

    fn set_scale(&self, _scale: f64) -> Result<(), PluginError> {
        log("set_scale");
        Err(PluginError::Message("it takes the window system's scale"))
    }

    fn get_size(&self) -> Option<GuiSize> {
        let size = self.gui.as_ref()?.interface.borrow().size;
        log(format!("get_size {}", size_text(size)));
        Some(GuiSize {
            width: size.0,
            height: size.1,
        })
    }

    fn can_resize(&self) -> bool {
        let fixed = FIXED.load(Ordering::Acquire);
        log(if fixed {
            "can_resize no"
        } else {
            "can_resize yes"
        });
        !fixed
    }

    fn adjust_size(&self, size: GuiSize) -> Option<GuiSize> {
        let asked = (size.width, size.height);
        let held = adjusted(asked);
        log(format!(
            "adjust_size {} -> {}",
            size_text(asked),
            size_text(held)
        ));
        Some(GuiSize {
            width: held.0,
            height: held.1,
        })
    }

    fn set_size(&self, size: GuiSize) -> Result<(), PluginError> {
        let size = (size.width, size.height);
        log(format!("set_size {}", size_text(size)));
        let gui = self
            .gui
            .as_ref()
            .ok_or(PluginError::Message("no interface"))?;
        let mut interface = gui.interface.borrow_mut();
        interface.size = size;
        if let Some(own) = &interface.own {
            own.resize(size);
        }
        Ok(())
    }

    fn set_parent(&self, window: Window) -> Result<(), PluginError> {
        log("set_parent");
        let gui = self
            .gui
            .as_ref()
            .ok_or(PluginError::Message("no interface"))?;
        let parent = window
            .as_x11_handle()
            .and_then(|handle| u32::try_from(handle).ok())
            .ok_or(PluginError::Message("not an X11 window"))?;
        let mut interface = gui.interface.borrow_mut();
        let own = Own::make(parent, interface.size)
            .ok_or(PluginError::Message("cannot make its window"))?;
        interface.own = Some(own);
        Ok(())
    }

    fn set_transient(&self, _window: Window) -> Result<(), PluginError> {
        log("set_transient");
        Err(PluginError::Message("embedded only"))
    }

    fn show(&self) -> Result<(), PluginError> {
        log("show");
        let gui = self
            .gui
            .as_ref()
            .ok_or(PluginError::Message("no interface"))?;
        if let Some(own) = &gui.interface.borrow().own {
            own.map(true);
        }
        Ok(())
    }

    fn hide(&self) -> Result<(), PluginError> {
        log("hide");
        let gui = self
            .gui
            .as_ref()
            .ok_or(PluginError::Message("no interface"))?;
        if let Some(own) = &gui.interface.borrow().own {
            own.map(false);
        }
        Ok(())
    }
}

/// The interface created last on this thread, while its instance lives.
fn current() -> Option<Rc<RefCell<Interface>>> {
    CURRENT.with(|current| current.borrow().upgrade())
}

/// The log (newline-joined) into `buffer` of `room` bytes; its whole length.
#[no_mangle]
pub extern "C" fn dawsome_test_clap_gui_log(buffer: *mut u8, room: usize) -> usize {
    let text = LOG.lock().map(|log| log.join("\n")).unwrap_or_default();
    let bytes = text.as_bytes();
    if !buffer.is_null() {
        let n = bytes.len().min(room);
        // SAFETY: the caller gives `room` writable bytes at `buffer`.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, n) };
    }
    bytes.len()
}

#[no_mangle]
pub extern "C" fn dawsome_test_clap_gui_log_clear() {
    if let Ok(mut log) = LOG.lock() {
        log.clear();
    }
}

/// Its X window, 0 while it has none.
#[no_mangle]
pub extern "C" fn dawsome_test_clap_gui_window() -> u32 {
    current()
        .and_then(|interface| interface.borrow().own.as_ref().map(|own| own.window))
        .unwrap_or(0)
}

/// How many X windows of its are alive.
#[no_mangle]
pub extern "C" fn dawsome_test_clap_gui_windows() -> i32 {
    WINDOWS.load(Ordering::Acquire)
}

/// Whether `can_resize` says no (`on` ≠ 0) from now.
#[no_mangle]
pub extern "C" fn dawsome_test_clap_gui_fixed(on: i32) {
    FIXED.store(on != 0, Ordering::Release);
}

/// The interface asks its host for `w` by `h` now (`request_resize`), logged as
/// "request_resize WxH -> true|false"; answered true, it takes that size itself at once (the host
/// need not call `set_size`). 1 for true, 0 for false or no interface.
#[no_mangle]
pub extern "C" fn dawsome_test_clap_gui_resize(w: u32, h: u32) -> i32 {
    let Some(interface) = current() else {
        return 0;
    };
    let host = interface.borrow().host;
    let answered = host
        .get_extension::<HostGui>()
        .is_some_and(|gui| gui.request_resize(&host, w, h).is_ok());
    log(format!(
        "request_resize {} -> {answered}",
        size_text((w, h))
    ));
    if answered {
        let size = (u32::from(side(w)), u32::from(side(h)));
        let mut interface = interface.borrow_mut();
        interface.size = size;
        if let Some(own) = &interface.own {
            own.resize(size);
        }
    }
    i32::from(answered)
}
