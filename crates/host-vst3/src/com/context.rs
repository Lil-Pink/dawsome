//! The host context a plugin is given at `initialize` (one per instance) and through
//! `IPluginFactory3::setHostContext` (one per module): `IHostApplication` (the host's name, and
//! the messages and attribute lists a plugin asks for), `IPlugInterfaceSupport`, and on Linux
//! `IRunLoop`, the timers and file descriptors a plugin needs served on its main thread ("On
//! Linux the host has to provide this interface", `iplugview.h`; the portal's "Provide a Runloop
//! on Linux", which a plugin with no window of its own reaches through the host context).
//!
//! The run loop's registrations are served by the host's tick, on the plugins' main thread, and
//! released (no further calls) once the instance that owns the context has terminated.

use std::ffi::c_void;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use vst3::Steinberg::Linux::IRunLoop;
use vst3::Steinberg::Linux::{
    FileDescriptor, IEventHandler, IEventHandlerTrait, IRunLoopTrait, ITimerHandler,
    ITimerHandlerTrait, TimerInterval,
};
use vst3::Steinberg::Vst::{
    IAttributeList_iid, IAudioProcessor_iid, IComponent_iid, IConnectionPoint_iid,
    IEditController_iid, IHostApplication, IHostApplicationTrait, IMessage_iid,
    IPlugInterfaceSupport, IPlugInterfaceSupportTrait, String128,
};
use vst3::Steinberg::{
    kInvalidArgument, kNoInterface, kResultFalse, kResultOk, kResultTrue, tresult, FUnknown, TUID,
};
use vst3::{Class, ComPtr, ComRef, ComWrapper};

use super::message::{AttributeList, Message};
use super::{guarded, write_utf16, Counted};

/// Timer periods below this are rounded up to it: the host's pump runs every 16 ms, the CLAP
/// host's rule too.
pub(crate) const MIN_TIMER_PERIOD: Duration = Duration::from_millis(16);

/// A timer a plugin registered, and when it is next due.
struct Timer {
    handler: ComPtr<ITimerHandler>,
    period: Duration,
    due: Instant,
}

/// What a context's run loop holds.
#[derive(Default)]
struct Registered {
    timers: Vec<Timer>,
    fds: Vec<(FileDescriptor, ComPtr<IEventHandler>)>,
    /// Its instance terminated: nothing more is registered or called.
    closed: bool,
}

/// The timers and file descriptors registered with one context.
#[derive(Default)]
pub(crate) struct RunLoop {
    registered: Mutex<Registered>,
}

impl RunLoop {
    /// Registers `handler` to be called every `milliseconds` (at least [`MIN_TIMER_PERIOD`]),
    /// first at `now` + its period.
    ///
    /// # Safety
    ///
    /// `handler` is null or points to a live COM object that stays live for as long as it is
    /// registered, and the call is made on the main thread.
    pub(crate) unsafe fn register_timer(
        &self,
        handler: *mut ITimerHandler,
        milliseconds: TimerInterval,
        now: Instant,
    ) -> tresult {
        // SAFETY: the caller's: `handler` is null (refused below) or a live COM object, and
        // `to_com_ptr` below keeps a reference of our own.
        let Some(handler) = (unsafe { ComRef::from_raw(handler) }) else {
            return kInvalidArgument;
        };
        if milliseconds == 0 {
            return kInvalidArgument;
        }
        let Ok(mut registered) = self.registered.lock() else {
            return kResultFalse;
        };
        if registered.closed {
            return kResultFalse;
        }
        let period = Duration::from_millis(milliseconds).max(MIN_TIMER_PERIOD);
        registered.timers.push(Timer {
            handler: handler.to_com_ptr(),
            period,
            due: now + period,
        });
        kResultOk
    }

    /// Forgets every timer of `handler`; the references go after the lock is let go of.
    pub(crate) fn unregister_timer(&self, handler: *mut ITimerHandler) -> tresult {
        let Ok(mut registered) = self.registered.lock() else {
            return kResultFalse;
        };
        let (gone, kept): (Vec<Timer>, Vec<Timer>) = std::mem::take(&mut registered.timers)
            .into_iter()
            .partition(|timer| timer.handler.as_ptr() == handler);
        registered.timers = kept;
        drop(registered);
        let found = !gone.is_empty();
        drop(gone);
        if found {
            kResultOk
        } else {
            kInvalidArgument
        }
    }

    /// Registers `handler` to be called when `fd` is readable.
    ///
    /// # Safety
    ///
    /// `handler` is null or points to a live COM object that stays live for as long as it is
    /// registered, and the call is made on the main thread.
    pub(crate) unsafe fn register_fd(
        &self,
        handler: *mut IEventHandler,
        fd: FileDescriptor,
    ) -> tresult {
        // SAFETY: the caller's, as for `register_timer`.
        let Some(handler) = (unsafe { ComRef::from_raw(handler) }) else {
            return kInvalidArgument;
        };
        let Ok(mut registered) = self.registered.lock() else {
            return kResultFalse;
        };
        if registered.closed {
            return kResultFalse;
        }
        registered.fds.push((fd, handler.to_com_ptr()));
        kResultOk
    }

    pub(crate) fn unregister_fd(&self, handler: *mut IEventHandler) -> tresult {
        let Ok(mut registered) = self.registered.lock() else {
            return kResultFalse;
        };
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut registered.fds)
            .into_iter()
            .partition(|(_, known)| known.as_ptr() == handler);
        registered.fds = kept;
        drop(registered);
        if gone.is_empty() {
            kInvalidArgument
        } else {
            kResultOk
        }
    }

    /// How many timers and file descriptors are registered.
    pub(crate) fn registered(&self) -> (usize, usize) {
        self.registered.lock().map_or((0, 0), |registered| {
            (registered.timers.len(), registered.fds.len())
        })
    }

    /// Whether anything is registered: work that comes due without the plugin asking.
    pub(crate) fn is_waiting(&self) -> bool {
        self.registered
            .lock()
            .is_ok_and(|registered| !registered.timers.is_empty() || !registered.fds.is_empty())
    }

    /// Calls the timers due at `now` (each moved on to `now` + its period: a stalled pump fires
    /// no burst) and the handlers of the file descriptors that are readable. The handlers are
    /// called with no lock held, so one may register or unregister as it runs.
    pub(crate) fn serve(&self, now: Instant) {
        let (due, fds) = {
            let Ok(mut registered) = self.registered.lock() else {
                return;
            };
            if registered.closed {
                return;
            }
            let mut due = Vec::new();
            for timer in registered
                .timers
                .iter_mut()
                .filter(|timer| timer.due <= now)
            {
                timer.due = now + timer.period;
                due.push(timer.handler.clone());
            }
            (due, registered.fds.clone())
        };
        for handler in due {
            // SAFETY: a handler the plugin registered, held alive by our reference.
            unsafe { handler.onTimer() };
        }
        for (fd, handler) in ready(fds) {
            // SAFETY: as above.
            unsafe { handler.onFDIsSet(fd) };
        }
    }

    /// Its instance terminated: every registration goes, outside the lock, and nothing more is
    /// taken or called.
    pub(crate) fn close(&self) {
        let gone = self.registered.lock().map(|mut registered| {
            registered.closed = true;
            (
                std::mem::take(&mut registered.timers),
                std::mem::take(&mut registered.fds),
            )
        });
        drop(gone);
    }
}

/// The file descriptors of `fds` that are readable (or hung up or in error, which a read then
/// reports), with their handlers.
#[cfg(unix)]
fn ready(
    fds: Vec<(FileDescriptor, ComPtr<IEventHandler>)>,
) -> Vec<(FileDescriptor, ComPtr<IEventHandler>)> {
    if fds.is_empty() {
        return fds;
    }
    let mut polled: Vec<libc::pollfd> = fds
        .iter()
        .map(|&(fd, _)| libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    let count = libc::nfds_t::try_from(polled.len()).unwrap_or(libc::nfds_t::MAX);
    // SAFETY: `polled` is a live array of `count` pollfd records, and a zero timeout returns at
    // once.
    if unsafe { libc::poll(polled.as_mut_ptr(), count, 0) } <= 0 {
        return Vec::new();
    }
    fds.into_iter()
        .zip(polled)
        .filter(|(_, polled)| polled.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0)
        .map(|(fd, _)| fd)
        .collect()
}

#[cfg(not(unix))]
fn ready(
    _fds: Vec<(FileDescriptor, ComPtr<IEventHandler>)>,
) -> Vec<(FileDescriptor, ComPtr<IEventHandler>)> {
    Vec::new()
}

/// The host context.
pub(crate) struct HostContext {
    run_loop: RunLoop,
    _counted: Counted,
}

#[cfg(target_os = "linux")]
impl Class for HostContext {
    type Interfaces = (IHostApplication, IPlugInterfaceSupport, IRunLoop);
}

#[cfg(not(target_os = "linux"))]
impl Class for HostContext {
    type Interfaces = (IHostApplication, IPlugInterfaceSupport);
}

impl HostContext {
    pub(crate) fn new() -> ComWrapper<Self> {
        ComWrapper::new(Self {
            run_loop: RunLoop::default(),
            _counted: Counted::new(),
        })
    }

    pub(crate) fn run_loop(&self) -> &RunLoop {
        &self.run_loop
    }
}

/// The context as the `FUnknown` a plugin's `initialize` takes.
pub(crate) fn as_unknown(context: &ComWrapper<HostContext>) -> *mut FUnknown {
    context
        .as_com_ref::<FUnknown>()
        .map_or(std::ptr::null_mut(), |unknown| unknown.as_ptr())
}

impl IHostApplicationTrait for HostContext {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        guarded(|| {
            // SAFETY: an out parameter the plugin owns.
            let Some(name) = (unsafe { name.as_mut() }) else {
                return kInvalidArgument;
            };
            write_utf16("Dawsome", name);
            kResultOk
        })
    }

    unsafe fn createInstance(
        &self,
        cid: *mut TUID,
        iid: *mut TUID,
        obj: *mut *mut c_void,
    ) -> tresult {
        guarded(|| {
            if cid.is_null() || iid.is_null() || obj.is_null() {
                return kInvalidArgument;
            }
            // SAFETY: the plugin's 16-byte id, non-null (checked), read for the call.
            let cid = unsafe { *cid };
            // SAFETY: as for `cid`.
            let iid = unsafe { *iid };
            // SAFETY: the plugin's out parameter, non-null (checked).
            unsafe { *obj = std::ptr::null_mut() };
            let made: Option<ComPtr<FUnknown>> = if cid == IMessage_iid && iid == IMessage_iid {
                Message::new().to_com_ptr()
            } else if cid == IAttributeList_iid && iid == IAttributeList_iid {
                AttributeList::new().to_com_ptr()
            } else {
                None
            };
            let Some(made) = made else {
                return kNoInterface;
            };
            let unknown = made.as_ptr();
            // SAFETY: `unknown` is our object, live while `made` holds it; a COM object's first
            // field is its vtable pointer.
            let vtbl = unsafe { (*unknown).vtbl };
            // SAFETY: the vtable of our own class (`vst3::Class`), static and never null.
            let query = unsafe { (*vtbl).queryInterface };
            // SAFETY: our queryInterface on our live object; it adds the reference the plugin
            // gets in `obj`, which is non-null (checked).
            unsafe { query(unknown, &iid, obj) }
        })
    }
}

impl IPlugInterfaceSupportTrait for HostContext {
    unsafe fn isPlugInterfaceSupported(&self, iid: *const TUID) -> tresult {
        guarded(|| {
            if iid.is_null() {
                return kInvalidArgument;
            }
            // SAFETY: the plugin's 16-byte id, non-null (checked), read for the call.
            let iid = unsafe { *iid };
            let used = [
                IComponent_iid,
                IAudioProcessor_iid,
                IEditController_iid,
                IConnectionPoint_iid,
            ];
            if used.contains(&iid) {
                kResultTrue
            } else {
                kResultFalse
            }
        })
    }
}

impl IRunLoopTrait for HostContext {
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use super::*;

    struct Counting {
        calls: Arc<AtomicU32>,
    }

    impl Class for Counting {
        type Interfaces = (ITimerHandler,);
    }

    impl ITimerHandlerTrait for Counting {
        unsafe fn onTimer(&self) {
            self.calls.fetch_add(1, Ordering::AcqRel);
        }
    }

    // 11. A 100 ms timer registered at t0 fires at t0 + 100, 200 and 300 ms of an injected
    // clock and never before; 0 ms is refused; once its context closes it is gone and nothing
    // more registers; an unregistered handler is not called again.
    #[test]
    fn a_timer_fires_on_its_period_and_not_before() {
        let calls = Arc::new(AtomicU32::new(0));
        let handler = ComWrapper::new(Counting {
            calls: Arc::clone(&calls),
        });
        let handler = handler
            .as_com_ref::<ITimerHandler>()
            .expect("a handler")
            .as_ptr();
        let run_loop = RunLoop::default();
        let t0 = Instant::now();
        // SAFETY: `handler` is the live `Counting` object above, non-null, and this test is the main
        // thread; the run loop keeps its own reference while registered.
        let registered = unsafe { run_loop.register_timer(handler, 0, t0) };
        assert_eq!(registered, kInvalidArgument);
        // SAFETY: `handler` is the live `Counting` object above, non-null, and this test is the main
        // thread; the run loop keeps its own reference while registered.
        let registered = unsafe { run_loop.register_timer(handler, 100, t0) };
        assert_eq!(registered, kResultOk);
        assert!(run_loop.is_waiting());
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut fired = Vec::new();
        for ms in [99, 100, 150, 199, 200, 299, 300] {
            let before = calls.load(Ordering::Acquire);
            run_loop.serve(at(ms));
            if calls.load(Ordering::Acquire) > before {
                fired.push(ms);
            }
        }
        assert_eq!(fired, [100, 200, 300]);
        assert_eq!(run_loop.unregister_timer(handler), kResultOk);
        assert_eq!(run_loop.unregister_timer(handler), kInvalidArgument);
        run_loop.serve(at(1000));
        assert_eq!(calls.load(Ordering::Acquire), 3, "unregistered");
        // SAFETY: `handler` is the live `Counting` object above, non-null, and this test is the main
        // thread; the run loop keeps its own reference while registered.
        let registered = unsafe { run_loop.register_timer(handler, 1, t0) };
        assert_eq!(registered, kResultOk);
        run_loop.serve(at(15));
        assert_eq!(calls.load(Ordering::Acquire), 3, "1 ms rounds up to 16");
        run_loop.serve(at(16));
        assert_eq!(calls.load(Ordering::Acquire), 4);
        run_loop.close();
        assert!(!run_loop.is_waiting());
        run_loop.serve(at(2000));
        assert_eq!(calls.load(Ordering::Acquire), 4, "closed");
        // SAFETY: `handler` is the live `Counting` object above, non-null, and this test is the main
        // thread; the run loop keeps its own reference while registered.
        let registered = unsafe { run_loop.register_timer(handler, 100, t0) };
        assert_eq!(registered, kResultFalse);
    }
}
