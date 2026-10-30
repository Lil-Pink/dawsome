//! The View class's editor (Linux): an `IPlugView` that logs every call its host makes into it
//! and, attached to a window of the host's, is an XEmbed client (XEmbed 0.5): its own window, a
//! child of the host's of its size, with `_XEMBED_INFO` [0, 1] (version 0, MAPPED), which it never
//! maps itself (the embedder does, as the protocol says). It takes its host's events through the
//! frame's run loop alone: its X connection and a pipe's read end are registered with the
//! `IRunLoop` its frame answers, and what they bring is noted, the pipe in the log ("fd pipe"),
//! the X events in a second log, "heard". A frame that answers no `IRunLoop` is refused at
//! `attached`, as DPF's editors refuse one.
//!
//! The tests reach it through the module's exports (`dawsome_test_vst3_view_*`): the two logs,
//! its window, a `resizeView` of its own now, a byte down its pipe, whether it says it resizes,
//! and whether its next `onSize` asks for another size inside.

use std::ffi::{c_void, CStr};
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, Weak};

use vst3::Steinberg::IPlugViewContentScaleSupport_::ScaleFactor;
use vst3::Steinberg::Linux::{
    FileDescriptor, IEventHandler, IEventHandlerTrait, IRunLoop, IRunLoopTrait,
};
use vst3::Steinberg::{
    char16, int16, kInvalidArgument, kResultFalse, kResultOk, kResultTrue, tresult, FIDString,
    IPlugFrame, IPlugFrameTrait, IPlugView, IPlugViewContentScaleSupport,
    IPlugViewContentScaleSupportTrait, IPlugViewTrait, TBool, ViewRect,
};
use vst3::{Class, ComPtr, ComRef, ComWrapper};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::{log, Live};

/// The window type it takes.
const X11: &[u8] = b"X11EmbedWindowID";
/// Its size until it is given another.
const SIZE: (i32, i32) = (400, 300);
/// What `checkSizeConstraint` holds a size to.
const LEAST: (i32, i32) = (200, 150);
const MOST: (i32, i32) = (1600, 1200);
/// What its next `onSize` asks for inside, once `dawsome_test_vst3_view_nested(1)`.
const NESTED: (i32, i32) = (500, 400);
/// XEmbed's opcodes it notes.
const EMBEDDED_NOTIFY: u32 = 0;
const WINDOW_ACTIVATE: u32 = 1;
const WINDOW_DEACTIVATE: u32 = 2;
const FOCUS_IN: u32 = 4;
const FOCUS_OUT: u32 = 5;

/// What it heard of its host's X messages, in turn.
static HEARD: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// `canResize` answers no.
static FIXED: AtomicBool = AtomicBool::new(false);
/// Its next `onSize` asks for [`NESTED`] inside.
static NESTED_NEXT: AtomicBool = AtomicBool::new(false);
/// Its X windows alive.
static WINDOWS: AtomicI32 = AtomicI32::new(0);
/// The editor alive now: its `IPlugView` pointer (compared and handed back to its frame, never
/// called through here) and its state.
static CURRENT: Mutex<Option<(usize, Weak<State>)>> = Mutex::new(None);

fn heard(line: &str) {
    if let Ok(mut heard) = HEARD.lock() {
        heard.push(line.to_owned());
    }
}

/// The heard lines, joined by newlines.
pub(crate) fn heard_text() -> String {
    HEARD
        .lock()
        .map(|heard| heard.join("\n"))
        .unwrap_or_default()
}

pub(crate) fn clear_heard() {
    if let Ok(mut heard) = HEARD.lock() {
        heard.clear();
    }
}

fn size_text((w, h): (i32, i32)) -> String {
    format!("{w}x{h}")
}

fn result_word(result: tresult) -> &'static str {
    match result {
        r if r == kResultTrue => "true",
        r if r == kResultFalse => "false",
        r if r == kInvalidArgument => "invalid",
        _ => "error",
    }
}

/// Its window inside the host's, while attached.
struct Embedded {
    conn: RustConnection,
    window: u32,
    parent: u32,
    xembed: u32,
    /// The pipe: its read end (registered) and its write end (poked).
    pipe: (UnixStream, UnixStream),
    handler: ComWrapper<Handler>,
    run_loop: ComPtr<IRunLoop>,
}

impl Embedded {
    /// Makes its window in `parent` at `size` and registers its two file descriptors; `None` when
    /// it cannot (what it made of it is undone as it drops).
    fn make(
        parent: u32,
        size: (i32, i32),
        run_loop: ComPtr<IRunLoop>,
        state: Weak<State>,
    ) -> Option<Self> {
        let pipe = UnixStream::pair().ok()?;
        pipe.0.set_nonblocking(true).ok()?;
        let (conn, _) = RustConnection::connect(None).ok()?;
        let intern = |name: &[u8]| -> Option<u32> {
            Some(conn.intern_atom(false, name).ok()?.reply().ok()?.atom)
        };
        let (info, xembed) = (intern(b"_XEMBED_INFO")?, intern(b"_XEMBED")?);
        let window = conn.generate_id().ok()?;
        let side = |pixels: i32| u16::try_from(pixels.clamp(1, 32_767)).unwrap_or(1);
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            parent,
            0,
            0,
            side(size.0),
            side(size.1),
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
        )
        .ok()?;
        WINDOWS.fetch_add(1, Ordering::AcqRel);
        let handler = ComWrapper::new(Handler {
            state,
            _live: Live::new(),
        });
        let embedded = Self {
            conn,
            window,
            parent,
            xembed,
            pipe,
            handler,
            run_loop,
        };
        // Version 0, MAPPED: the embedder maps it.
        embedded
            .conn
            .change_property32(PropMode::REPLACE, window, info, info, &[0, 1])
            .ok()?;
        embedded.conn.flush().ok()?;
        let handler = embedded.handler.as_com_ref::<IEventHandler>()?.as_ptr();
        // SAFETY: the frame's run loop, live while the frame is ours; the handler is our object,
        // which the run loop keeps until it is unregistered (as it drops).
        unsafe {
            embedded
                .run_loop
                .registerEventHandler(handler, embedded.conn.stream().as_raw_fd())
        };
        // SAFETY: as above, for the pipe's end.
        unsafe {
            embedded
                .run_loop
                .registerEventHandler(handler, embedded.pipe.0.as_raw_fd())
        };
        Some(embedded)
    }

    /// What its file descriptor `fd` brings: the pipe's bytes, or its X events.
    fn serve(&self, fd: FileDescriptor) {
        if fd == self.pipe.0.as_raw_fd() {
            let mut buffer = [0u8; 64];
            loop {
                match (&self.pipe.0).read(&mut buffer) {
                    Ok(0) => break,
                    Ok(_) => continue,
                    Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            }
            log("fd pipe");
        }
        if fd == self.conn.stream().as_raw_fd() {
            while let Ok(Some(event)) = self.conn.poll_for_event() {
                match event {
                    Event::ClientMessage(message) if message.type_ == self.xembed => {
                        let data = message.data.as_data32();
                        match data[1] {
                            EMBEDDED_NOTIFY if data[3] == self.parent && data[4] == 0 => {
                                heard("embedded_notify");
                            }
                            EMBEDDED_NOTIFY => heard("embedded_notify from another embedder"),
                            WINDOW_ACTIVATE => heard("window_activate"),
                            WINDOW_DEACTIVATE => heard("window_deactivate"),
                            FOCUS_IN => heard("focus_in"),
                            FOCUS_OUT => heard("focus_out"),
                            other => heard(&format!("xembed {other}")),
                        }
                    }
                    Event::MapNotify(map) if map.window == self.window => heard("mapped"),
                    Event::UnmapNotify(unmap) if unmap.window == self.window => heard("unmapped"),
                    _ => {}
                }
            }
        }
    }

    fn resize(&self, (w, h): (i32, i32)) {
        let aux = ConfigureWindowAux::new()
            .width(w.clamp(1, 32_767) as u32)
            .height(h.clamp(1, 32_767) as u32);
        let _ = self.conn.configure_window(self.window, &aux);
        let _ = self.conn.flush();
    }
}

impl Drop for Embedded {
    // Its registrations go first, then its window; the connection and the pipe close as it drops.
    fn drop(&mut self) {
        if let Some(handler) = self.handler.as_com_ref::<IEventHandler>() {
            // SAFETY: as at registration.
            unsafe { self.run_loop.unregisterEventHandler(handler.as_ptr()) };
        }
        let _ = self.conn.destroy_window(self.window);
        let _ = self.conn.flush();
        WINDOWS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The editor's state, which its exports reach too.
#[derive(Default)]
struct State {
    frame: Mutex<Option<ComPtr<IPlugFrame>>>,
    run_loop: Mutex<Option<ComPtr<IRunLoop>>>,
    size: Mutex<Option<(i32, i32)>>,
    embedded: Mutex<Option<Embedded>>,
}

impl State {
    fn size(&self) -> (i32, i32) {
        self.size.lock().ok().and_then(|size| *size).unwrap_or(SIZE)
    }
}

/// Its run loop's handler: what its file descriptors bring goes to the editor.
struct Handler {
    state: Weak<State>,
    _live: Live,
}

impl Class for Handler {
    type Interfaces = (IEventHandler,);
}

impl IEventHandlerTrait for Handler {
    unsafe fn onFDIsSet(&self, fd: FileDescriptor) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let embedded = state.embedded.lock();
        if let Ok(Some(embedded)) = embedded.as_deref() {
            embedded.serve(fd);
        }
    }
}

/// The editor.
pub(crate) struct View {
    state: Arc<State>,
    _live: Live,
}

impl Class for View {
    type Interfaces = (IPlugView, IPlugViewContentScaleSupport);
}

/// A new editor, its one reference the caller's.
pub(crate) fn make() -> *mut IPlugView {
    let state = Arc::new(State::default());
    let view = ComWrapper::new(View {
        state: Arc::clone(&state),
        _live: Live::new(),
    });
    let Some(view) = view.to_com_ptr::<IPlugView>() else {
        return std::ptr::null_mut();
    };
    if let Ok(mut current) = CURRENT.lock() {
        *current = Some((view.as_ptr() as usize, Arc::downgrade(&state)));
    }
    view.into_raw()
}

impl Drop for View {
    fn drop(&mut self) {
        // A host that never called `removed` still takes its window down here.
        let embedded = self.state.embedded.lock().ok().and_then(|mut e| e.take());
        drop(embedded);
        if let Ok(mut current) = CURRENT.lock() {
            if current
                .as_ref()
                .is_some_and(|(_, state)| state.as_ptr() == Arc::as_ptr(&self.state))
            {
                *current = None;
            }
        }
        log("view freed");
    }
}

/// `rect`'s size.
fn size_of(rect: &ViewRect) -> (i32, i32) {
    (rect.right - rect.left, rect.bottom - rect.top)
}

fn rect_of((w, h): (i32, i32)) -> ViewRect {
    ViewRect {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    }
}

impl IPlugViewTrait for View {
    unsafe fn isPlatformTypeSupported(&self, r#type: FIDString) -> tresult {
        let name = if r#type.is_null() {
            &b""[..]
        } else {
            // SAFETY: a non-null platform type is NUL-terminated (`FIDString`), for the call.
            unsafe { CStr::from_ptr(r#type) }.to_bytes()
        };
        log(format!(
            "isPlatformTypeSupported {}",
            String::from_utf8_lossy(name)
        ));
        if name == X11 {
            kResultTrue
        } else {
            kResultFalse
        }
    }

    unsafe fn attached(&self, parent: *mut c_void, r#type: FIDString) -> tresult {
        let run_loop = self
            .state
            .run_loop
            .lock()
            .ok()
            .and_then(|run_loop| run_loop.clone());
        let Some(run_loop) = run_loop else {
            log("attached refused");
            return kInvalidArgument;
        };
        // SAFETY: as in `isPlatformTypeSupported`, behind the null check.
        if r#type.is_null() || unsafe { CStr::from_ptr(r#type) }.to_bytes() != X11 {
            log("attached refused");
            return kInvalidArgument;
        }
        log("attached X11EmbedWindowID");
        let parent = parent as usize as u32;
        let Some(embedded) = Embedded::make(
            parent,
            self.state.size(),
            run_loop,
            Arc::downgrade(&self.state),
        ) else {
            return kResultFalse;
        };
        if let Ok(mut slot) = self.state.embedded.lock() {
            *slot = Some(embedded);
        }
        kResultOk
    }

    unsafe fn removed(&self) -> tresult {
        log("removed");
        let embedded = self.state.embedded.lock().ok().and_then(|mut e| e.take());
        drop(embedded);
        kResultOk
    }

    unsafe fn onWheel(&self, _distance: f32) -> tresult {
        log("onWheel");
        kResultFalse
    }

    unsafe fn onKeyDown(&self, _key: char16, _code: int16, _modifiers: int16) -> tresult {
        log("onKeyDown");
        kResultFalse
    }

    unsafe fn onKeyUp(&self, _key: char16, _code: int16, _modifiers: int16) -> tresult {
        log("onKeyUp");
        kResultFalse
    }

    unsafe fn getSize(&self, size: *mut ViewRect) -> tresult {
        // SAFETY: the host's out parameter (`getSize`), null or live for the call.
        let Some(size) = (unsafe { size.as_mut() }) else {
            return kInvalidArgument;
        };
        let kept = self.state.size();
        log(format!("getSize {}", size_text(kept)));
        *size = rect_of(kept);
        kResultOk
    }

    unsafe fn onSize(&self, new_size: *mut ViewRect) -> tresult {
        // SAFETY: the host's size (`onSize`), null or live for the call.
        let Some(rect) = (unsafe { new_size.as_ref() }) else {
            return kInvalidArgument;
        };
        let size = size_of(rect);
        log(format!("onSize {}", size_text(size)));
        if let Ok(mut kept) = self.state.size.lock() {
            *kept = Some(size);
        }
        if let Ok(embedded) = self.state.embedded.lock() {
            if let Some(embedded) = embedded.as_ref() {
                embedded.resize(size);
            }
        }
        if NESTED_NEXT.swap(false, Ordering::AcqRel) {
            ask_resize(NESTED);
        }
        kResultOk
    }

    unsafe fn onFocus(&self, _state: TBool) -> tresult {
        log("onFocus");
        kResultOk
    }

    unsafe fn setFrame(&self, frame: *mut IPlugFrame) -> tresult {
        // SAFETY: the host's frame (`setFrame`), null or live for the call; the `ComPtr` keeps
        // our own reference.
        let frame = unsafe { ComRef::from_raw(frame) }.map(|frame| frame.to_com_ptr());
        let run_loop = frame.as_ref().and_then(|frame| frame.cast::<IRunLoop>());
        log(match (&frame, &run_loop) {
            (None, _) => "setFrame null",
            (Some(_), Some(_)) => "setFrame frame runloop",
            (Some(_), None) => "setFrame frame",
        });
        // The ones they replace go once the locks are let go of.
        let old_frame = self
            .state
            .frame
            .lock()
            .map(|mut held| std::mem::replace(&mut *held, frame));
        let old_loop = self
            .state
            .run_loop
            .lock()
            .map(|mut held| std::mem::replace(&mut *held, run_loop));
        drop((old_frame, old_loop));
        kResultOk
    }

    unsafe fn canResize(&self) -> tresult {
        if FIXED.load(Ordering::Acquire) {
            log("canResize no");
            kResultFalse
        } else {
            log("canResize yes");
            kResultTrue
        }
    }

    unsafe fn checkSizeConstraint(&self, rect: *mut ViewRect) -> tresult {
        // SAFETY: the host's rectangle (`checkSizeConstraint`), null or live for the call.
        let Some(rect) = (unsafe { rect.as_mut() }) else {
            return kInvalidArgument;
        };
        let asked = size_of(rect);
        let held = (
            asked.0.clamp(LEAST.0, MOST.0),
            asked.1.clamp(LEAST.1, MOST.1),
        );
        log(format!(
            "checkSizeConstraint {} -> {}",
            size_text(asked),
            size_text(held)
        ));
        *rect = rect_of(held);
        kResultTrue
    }
}

impl IPlugViewContentScaleSupportTrait for View {
    unsafe fn setContentScaleFactor(&self, _factor: ScaleFactor) -> tresult {
        log("setContentScaleFactor");
        kResultOk
    }
}

/// The editor alive now: its `IPlugView` pointer and its state.
fn current() -> Option<(*mut IPlugView, Arc<State>)> {
    let current = CURRENT.lock().ok()?;
    let (view, state) = current.as_ref()?;
    Some((*view as *mut IPlugView, state.upgrade()?))
}

/// The editor asks its frame for `size` now, and logs the answer.
fn ask_resize(size: (i32, i32)) -> tresult {
    let Some((view, state)) = current() else {
        return kInvalidArgument;
    };
    let frame = state.frame.lock().ok().and_then(|frame| frame.clone());
    let Some(frame) = frame else {
        return kInvalidArgument;
    };
    let mut rect = rect_of(size);
    // SAFETY: its own frame, held by a reference of ours; `view` is its own live view. No lock of
    // ours is held across the call (the host calls `onSize` inside it).
    let result = unsafe { frame.resizeView(view, &mut rect) };
    log(format!(
        "resizeView {} -> {}",
        size_text(size),
        result_word(result)
    ));
    result
}

/// `resizeView` of `w` by `h` now, as the editor would ask it; the answer (`kResultTrue`,
/// `kResultFalse`, `kInvalidArgument`), logged as "resizeView WxH -> true|false|invalid".
#[no_mangle]
pub extern "C" fn dawsome_test_vst3_view_resize(w: i32, h: i32) -> i32 {
    ask_resize((w, h))
}

/// One byte down its pipe; how many were written.
#[no_mangle]
pub extern "C" fn dawsome_test_vst3_view_poke() -> i32 {
    let Some((_, state)) = current() else {
        return 0;
    };
    let embedded = state.embedded.lock();
    let Ok(Some(embedded)) = embedded.as_deref() else {
        return 0;
    };
    (&embedded.pipe.1).write(&[1]).map_or(0, |n| n as i32)
}

/// Its X window, 0 while it has none.
#[no_mangle]
pub extern "C" fn dawsome_test_vst3_view_window() -> u32 {
    current()
        .and_then(|(_, state)| {
            let embedded = state.embedded.lock().ok()?;
            embedded.as_ref().map(|embedded| embedded.window)
        })
        .unwrap_or(0)
}

/// How many X windows of its are alive.
#[no_mangle]
pub extern "C" fn dawsome_test_vst3_view_windows() -> i32 {
    WINDOWS.load(Ordering::Acquire)
}

/// Whether `canResize` says no (`on` ≠ 0) from now.
#[no_mangle]
pub extern "C" fn dawsome_test_vst3_view_fixed(on: i32) {
    FIXED.store(on != 0, Ordering::Release);
}

/// Whether its next `onSize` asks for 500x400 inside (`on` ≠ 0).
#[no_mangle]
pub extern "C" fn dawsome_test_vst3_view_nested(on: i32) {
    NESTED_NEXT.store(on != 0, Ordering::Release);
}

/// The heard lines (newline-joined) into `buffer` of `room` bytes; their whole length.
#[no_mangle]
pub extern "C" fn dawsome_test_vst3_view_heard(buffer: *mut u8, room: usize) -> usize {
    crate::copy_out(&heard_text(), buffer, room)
}
