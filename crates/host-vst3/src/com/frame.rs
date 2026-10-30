//! The frame a plugin's editor is given (`IPlugFrame`): the one way the editor asks for another
//! size, answered inside the call as the VST 3 portal's "Resize View Call Sequence" orders it (our
//! window resized, the editor's `onSize`, then the answer), and on Linux the run loop the
//! editor's timers and file descriptors are served by (`IRunLoop`, which an editor asks its frame
//! for: "On Linux the host has to provide this interface", `iplugview.h`; DPF's editors refuse a
//! frame without one).
//!
//! The run loop is the frame's own, not the instance context's: what an editor registers ends
//! with it, so an editor that forgets to unregister is never called into once it is gone. The
//! frame holds no reference to the editor (a raw pointer, compared, and called through only
//! while the host keeps the editor: a reference would make a cycle, each holding the other) and
//! nothing of the host's but our window's handle and atomics.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use dawsome_host_common::window::{clamp_side, Settle, WindowHandle};
#[cfg(target_os = "linux")]
use vst3::Steinberg::Linux::IRunLoop;
use vst3::Steinberg::Linux::{
    FileDescriptor, IEventHandler, IRunLoopTrait, ITimerHandler, TimerInterval,
};
use vst3::Steinberg::{
    kInvalidArgument, kResultFalse, kResultTrue, tresult, IPlugFrame, IPlugFrameTrait, IPlugView,
    IPlugViewTrait, ViewRect,
};
use vst3::{Class, ComPtr, ComWrapper};

use super::context::RunLoop;
use super::{guarded, is_main_thread, Counted};

/// A side of `rect`, from `from` to `to`, as a window takes it: 1 to 32767 pixels.
fn side(from: i32, to: i32) -> u32 {
    let pixels = (i64::from(to) - i64::from(from)).clamp(0, i64::from(u32::MAX));
    u32::from(clamp_side(pixels as u32))
}

/// `rect`'s size, each side held to what a window takes.
pub(crate) fn size_of(rect: &ViewRect) -> (u32, u32) {
    (side(rect.left, rect.right), side(rect.top, rect.bottom))
}

/// The rectangle of `size` at the origin, as `onSize` and `checkSizeConstraint` take one.
pub(crate) fn rect_of((width, height): (u32, u32)) -> ViewRect {
    ViewRect {
        left: 0,
        top: 0,
        right: width as i32,
        bottom: height as i32,
    }
}

/// What `resizeView` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resize {
    /// Answered with this code, nothing done.
    Refused(tresult),
    /// The size is the one kept: true, nothing done.
    Kept,
    /// Our window takes this size, it is kept, and the editor's `onSize` is called with it.
    To((u32, u32)),
}

/// What an editor's `resizeView(view, rect)` does (S6): off the main thread false; a null or
/// another view invalid; while its editor is closing, or inside its own `resizeView` (an `onSize`
/// asking again), false; the size held to what a window takes equal to the one `kept`, nothing;
/// else that size.
pub(crate) fn decide(
    main: bool,
    asked: usize,
    own: usize,
    closing: bool,
    inside: bool,
    rect: Option<ViewRect>,
    kept: (u32, u32),
) -> Resize {
    if !main {
        return Resize::Refused(kResultFalse);
    }
    let Some(rect) = rect.filter(|_| asked != 0 && asked == own) else {
        return Resize::Refused(kInvalidArgument);
    };
    if closing || inside {
        return Resize::Refused(kResultFalse);
    }
    let size = size_of(&rect);
    if size == kept {
        Resize::Kept
    } else {
        Resize::To(size)
    }
}

fn pack((width, height): (u32, u32)) -> u64 {
    (u64::from(width) << 32) | u64::from(height)
}

fn unpack(packed: u64) -> (u32, u32) {
    ((packed >> 32) as u32, packed as u32)
}

/// The frame.
pub(crate) struct PlugFrame {
    /// The editor's `IPlugView`, what `resizeView` must name; 0 once the editor is let go.
    view: AtomicUsize,
    window: WindowHandle,
    /// The size kept: what our window and the editor were last given.
    size: AtomicU64,
    /// The editor said it does not resize: our window keeps its size, and a user's resize is
    /// not asked of it.
    fixed: bool,
    /// Its editor is being closed: no `resizeView` is answered.
    closing: AtomicBool,
    /// Inside its own `resizeView`.
    inside: AtomicBool,
    /// Whether our window has the size kept, as the window manager last told us.
    settle: Mutex<Settle>,
    run_loop: RunLoop,
    _counted: Counted,
}

#[cfg(target_os = "linux")]
impl Class for PlugFrame {
    type Interfaces = (IPlugFrame, IRunLoop);
}

#[cfg(not(target_os = "linux"))]
impl Class for PlugFrame {
    type Interfaces = (IPlugFrame,);
}

impl PlugFrame {
    pub(crate) fn new(
        view: *mut IPlugView,
        window: WindowHandle,
        size: (u32, u32),
        fixed: bool,
    ) -> ComWrapper<Self> {
        ComWrapper::new(Self {
            view: AtomicUsize::new(view as usize),
            window,
            size: AtomicU64::new(pack(size)),
            fixed,
            closing: AtomicBool::new(false),
            inside: AtomicBool::new(false),
            settle: Mutex::new(Settle::new(size, Instant::now())),
            run_loop: RunLoop::default(),
            _counted: Counted::new(),
        })
    }

    /// The frame as the `IPlugFrame` an editor's `setFrame` takes.
    pub(crate) fn as_frame(frame: &ComWrapper<Self>) -> *mut IPlugFrame {
        frame
            .as_com_ref::<IPlugFrame>()
            .map_or(std::ptr::null_mut(), |frame| frame.as_ptr())
    }

    pub(crate) fn window(&self) -> &WindowHandle {
        &self.window
    }

    pub(crate) fn fixed(&self) -> bool {
        self.fixed
    }

    /// The size kept.
    pub(crate) fn size(&self) -> (u32, u32) {
        unpack(self.size.load(Ordering::Acquire))
    }

    pub(crate) fn keep(&self, size: (u32, u32)) {
        self.size.store(pack(size), Ordering::Release);
    }

    /// Our window takes `size`, kept from now (a fixed editor's window keeps it as its least and
    /// most size too).
    pub(crate) fn size_window(&self, size: (u32, u32)) {
        if self.fixed {
            self.window.fix(size.0, size.1);
        } else {
            self.window.resize(size.0, size.1);
        }
        self.keep(size);
        self.asked(Instant::now());
    }

    /// Our window asked back to `size` (the one kept), the editor not told: it has that size.
    fn put_back(&self, size: (u32, u32), now: Instant) {
        self.window.resize(size.0, size.1);
        self.asked(now);
    }

    fn asked(&self, now: Instant) {
        if let Ok(mut settle) = self.settle.lock() {
            settle.asked(now);
        }
    }

    /// The window manager told us at `now` that our window is `size`.
    fn told(&self, size: (u32, u32), now: Instant) {
        if let Ok(mut settle) = self.settle.lock() {
            settle.told(size, now);
        }
    }

    /// Asks our window again for the size kept when the window manager left it at another and
    /// has been quiet a while ([`Settle::due`]); a fixed editor's window is left to its hints.
    pub(crate) fn settle(&self, now: Instant) {
        if self.fixed {
            return;
        }
        let kept = self.size();
        let due = self
            .settle
            .lock()
            .ok()
            .and_then(|mut settle| settle.due(kept, now));
        if let Some((width, height)) = due {
            self.window.resize(width, height);
        }
    }

    pub(crate) fn run_loop(&self) -> &RunLoop {
        &self.run_loop
    }

    /// Its editor is being closed: no `resizeView` is answered from now.
    pub(crate) fn close(&self) {
        self.closing.store(true, Ordering::Release);
    }

    /// Its editor was removed and let go of its frame: its pointer is forgotten, and what it
    /// registered with the run loop goes.
    pub(crate) fn forget_view(&self) {
        self.view.store(0, Ordering::Release);
        self.run_loop.close();
    }

    /// Serves the editor's run loop.
    pub(crate) fn serve(&self, now: Instant) {
        self.run_loop.serve(now);
    }
}

impl IPlugFrameTrait for PlugFrame {
    unsafe fn resizeView(&self, view: *mut IPlugView, new_size: *mut ViewRect) -> tresult {
        guarded(|| {
            // SAFETY: the editor's out parameter, read once; null is refused by `decide`.
            let rect = unsafe { new_size.as_ref() }.copied();
            let decided = decide(
                is_main_thread(),
                view as usize,
                self.view.load(Ordering::Acquire),
                self.closing.load(Ordering::Acquire),
                self.inside.load(Ordering::Acquire),
                rect,
                self.size(),
            );
            let size = match decided {
                Resize::Refused(code) => return code,
                Resize::Kept => return kResultTrue,
                Resize::To(size) => size,
            };
            self.inside.store(true, Ordering::Release);
            self.size_window(size);
            let mut rect = rect_of(size);
            // SAFETY: `view` is the editor the host keeps (it is this frame's own, just compared,
            // and the host forgets it before letting it go), so it is live and the reference is
            // borrowed for this call alone.
            let view = unsafe { vst3::ComRef::from_raw(view) };
            if let Some(view) = view {
                // SAFETY: the live editor (as above), on the main thread inside its own call, as
                // the portal's resize sequence has it; `rect` is ours.
                unsafe { view.onSize(&mut rect) };
            }
            self.inside.store(false, Ordering::Release);
            kResultTrue
        })
    }
}

impl IRunLoopTrait for PlugFrame {
    unsafe fn registerEventHandler(
        &self,
        handler: *mut IEventHandler,
        fd: FileDescriptor,
    ) -> tresult {
        guarded(|| {
            // SAFETY: the plugin handed us this pointer through the VST3 interface, which promises a valid COM
            // object until it is unregistered; null is checked inside, and the call is on the main thread.
            unsafe { self.run_loop.register_fd(handler, fd) }
        })
    }

    unsafe fn unregisterEventHandler(&self, handler: *mut IEventHandler) -> tresult {
        guarded(|| self.run_loop.unregister_fd(handler))
    }

    unsafe fn registerTimer(
        &self,
        handler: *mut ITimerHandler,
        milliseconds: TimerInterval,
    ) -> tresult {
        guarded(|| {
            // SAFETY: the plugin handed us this pointer through the VST3 interface, which promises a valid COM
            // object until it is unregistered; null is checked inside, and the call is on the main thread.
            unsafe {
                self.run_loop
                    .register_timer(handler, milliseconds, Instant::now())
            }
        })
    }

    unsafe fn unregisterTimer(&self, handler: *mut ITimerHandler) -> tresult {
        guarded(|| self.run_loop.unregister_timer(handler))
    }
}

/// A user's resize of our window to `size` at `now` (S7): the size kept (our own resize's echo),
/// or an editor that keeps its size, nothing; else the editor's `checkSizeConstraint` gives the
/// size kept, our window takes it when it differs, and `onSize`; refused, our window goes back to
/// the size kept. A size the editor holds to the one it has already (a window manager that will
/// not give our window the size asked, and says so twice) only puts our window back: the editor
/// is not told a size it has. Whatever our window is asked for here is asked again while the
/// window manager does not give it ([`PlugFrame::settle`]).
pub(crate) fn user_resized(
    view: &ComPtr<IPlugView>,
    frame: &PlugFrame,
    size: (u32, u32),
    now: Instant,
) {
    frame.told(size, now);
    let kept = frame.size();
    if frame.fixed() || size == kept {
        return;
    }
    let mut rect = rect_of(size);
    // SAFETY: the editor is live (the host holds it); called on the main thread; `rect` is ours.
    if unsafe { view.checkSizeConstraint(&mut rect) } != kResultTrue {
        frame.put_back(kept, now);
        return;
    }
    let held = size_of(&rect);
    if held == kept {
        frame.put_back(held, now);
        return;
    }
    if held == size {
        frame.keep(held);
    } else {
        frame.size_window(held);
    }
    let mut rect = rect_of(held);
    // SAFETY: as above.
    unsafe { view.onSize(&mut rect) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(right: i32, bottom: i32) -> Option<ViewRect> {
        Some(ViewRect {
            left: 0,
            top: 0,
            right,
            bottom,
        })
    }

    // S6's rules in their order: off the main thread false; null rect or another (or no) view
    // invalid; closing or inside its own call false; the kept size nothing; another size taken,
    // each side held to 1..32767 (0x0 is 1x1, a rect turned over too).
    #[test]
    fn a_resize_is_refused_kept_or_taken_by_the_rules() {
        let kept = (400, 300);
        let decide_main =
            |asked, rect, closing, inside| decide(true, asked, 7, closing, inside, rect, kept);
        assert_eq!(
            decide(false, 7, 7, false, false, rect(300, 200), kept),
            Resize::Refused(kResultFalse)
        );
        assert_eq!(
            decide_main(7, None, false, false),
            Resize::Refused(kInvalidArgument)
        );
        assert_eq!(
            decide_main(8, rect(300, 200), false, false),
            Resize::Refused(kInvalidArgument)
        );
        assert_eq!(
            decide(true, 0, 0, false, false, rect(300, 200), kept),
            Resize::Refused(kInvalidArgument),
            "no editor kept"
        );
        assert_eq!(
            decide_main(7, rect(300, 200), true, false),
            Resize::Refused(kResultFalse)
        );
        assert_eq!(
            decide_main(7, rect(300, 200), false, true),
            Resize::Refused(kResultFalse)
        );
        assert_eq!(decide_main(7, rect(400, 300), false, false), Resize::Kept);
        assert_eq!(
            decide_main(7, rect(300, 200), false, false),
            Resize::To((300, 200))
        );
        assert_eq!(decide_main(7, rect(0, 0), false, false), Resize::To((1, 1)));
        assert_eq!(
            decide_main(7, rect(-5, 99_999), false, false),
            Resize::To((1, 32_767))
        );
        let far = ViewRect {
            left: i32::MIN,
            top: 0,
            right: i32::MAX,
            bottom: 10,
        };
        assert_eq!(size_of(&far), (32_767, 10), "no overflow");
    }
}
