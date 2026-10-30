//! The objects this host hands VST3 plugins, each a COM class of its own (`vst3`'s `Class` and
//! `ComWrapper`): the host context with its run loop, messages and their attribute lists, memory
//! streams, the audio half's event list and parameter changes, the component handler, the
//! connection proxy, and the frame an editor sits in (with a run loop of its own). The SDK's headers and its hosting samples were the reference (read for the
//! protocol only; nothing copied).
//!
//! Every method body runs inside [`guard`], so a panic of ours becomes `kInternalError` instead
//! of unwinding into a plugin's C++ (which aborts the process). None of these objects touches the
//! host's instance table: a plugin may call any of them in the middle of a call the host makes
//! into it (a `restartComponent` inside `setComponentState`, a `notify` inside `connect`), and
//! every lock they take is their own, held only for the length of the method.

pub(crate) mod connection;
pub(crate) mod context;
pub(crate) mod events;
pub(crate) mod frame;
pub(crate) mod handler;
pub(crate) mod message;
pub(crate) mod params;
pub(crate) mod stream;

use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicIsize, Ordering};

use vst3::Steinberg::Vst::TChar;
use vst3::Steinberg::{kInternalError, tresult};

thread_local! {
    /// Set on the plugins' main thread: the thread that made the host (the UI thread, the
    /// command line's main thread, a scan child's only thread).
    static IS_MAIN: Cell<bool> = const { Cell::new(false) };
}

/// Marks this thread as the plugins' main thread.
pub(crate) fn mark_main_thread() {
    IS_MAIN.with(|flag| flag.set(true));
}

/// Whether this is the plugins' main thread.
pub(crate) fn is_main_thread() -> bool {
    IS_MAIN.with(Cell::get)
}

/// Runs a COM method's `body`; a panic in it returns `fallback` instead of unwinding into the
/// plugin's code.
pub(crate) fn guard<T>(fallback: T, body: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or(fallback)
}

/// [`guard`] for a method that returns a result code.
pub(crate) fn guarded(body: impl FnOnce() -> tresult) -> tresult {
    guard(kInternalError, body)
}

static HOST_OBJECTS: AtomicIsize = AtomicIsize::new(0);

/// How many of this host's COM objects are alive: what a test of reference counting holds to 0
/// once every plugin is gone (a plugin that keeps one of ours past its own end leaks it).
pub fn host_objects() -> isize {
    HOST_OBJECTS.load(Ordering::Acquire)
}

/// One of this host's COM objects alive, counted from its making to its drop.
#[derive(Debug)]
pub(crate) struct Counted;

impl Counted {
    pub(crate) fn new() -> Self {
        HOST_OBJECTS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        HOST_OBJECTS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// `text` into the UTF-16 buffer `to`, cut to fit and ended with a 0.
pub(crate) fn write_utf16(text: &str, to: &mut [TChar]) {
    let room = to.len().saturating_sub(1);
    let mut len = 0;
    for (slot, unit) in to.iter_mut().take(room).zip(text.encode_utf16()) {
        *slot = unit;
        len += 1;
    }
    if let Some(end) = to.get_mut(len) {
        *end = 0;
    }
}

/// The UTF-16 text in `from`, up to its first 0.
pub(crate) fn read_utf16(from: &[TChar]) -> String {
    let len = from
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(from.len());
    String::from_utf16_lossy(&from[..len])
}

/// The text in the `char8` buffer `from`, up to its first 0.
pub(crate) fn read_utf8(from: &[std::ffi::c_char]) -> String {
    let len = from
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(from.len());
    let bytes: Vec<u8> = from[..len].iter().map(|&unit| unit as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}
