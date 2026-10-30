//! What a CLAP plugin can call on its host, per instance: the host-side callbacks and the host
//! extensions Dawsome offers (parameters, state, GUI, timers, file descriptors on Linux, thread
//! checks, latency, audio and note ports). Requests from the plugin become flags in
//! [`Signals`], which the main-thread pump and the audio half read; each one the main thread
//! must act on also rings the host's doorbell. Nothing here blocks or allocates on the audio
//! thread.
//!
//! No log extension: a plugin logging from its audio thread would make the host take a lock or
//! allocate there.

use std::cell::{Cell, RefCell};
#[cfg(unix)]
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clack_extensions::audio_ports::{AudioPortRescanFlags, HostAudioPorts, HostAudioPortsImpl};
use clack_extensions::gui::{GuiSize, HostGui, HostGuiImpl};
use clack_extensions::latency::{HostLatency, HostLatencyImpl};
use clack_extensions::note_ports::{
    HostNotePorts, HostNotePortsImpl, NoteDialects, NotePortRescanFlags,
};
use clack_extensions::params::{
    HostParams, HostParamsImplMainThread, HostParamsImplShared, ParamClearFlags, ParamRescanFlags,
};
// File descriptors are a POSIX extension of CLAP, which plugins use on Linux only.
#[cfg(unix)]
use clack_extensions::posix_fd::{FdFlags, HostPosixFd, HostPosixFdImpl};
use clack_extensions::state::{HostState, HostStateImpl};
use clack_extensions::thread_check::{HostThreadCheck, HostThreadCheckImpl};
use clack_extensions::timer::{HostTimer, HostTimerImpl, TimerId};
use clack_host::prelude::{
    HostError, HostExtensions, HostHandlers, MainThreadHandler, SharedHandler,
};

thread_local! {
    /// Set on the thread that made the host: the plugins' main thread.
    pub(crate) static IS_MAIN: Cell<bool> = const { Cell::new(false) };
    /// Set while a plugin's processor is being called on the audio thread.
    pub(crate) static IN_AUDIO: Cell<bool> = const { Cell::new(false) };
}

/// Timer periods below this are rounded up to it: the pump runs every 16 ms, as the CLAP timer
/// extension allows a host to choose.
const MIN_TIMER_PERIOD: Duration = Duration::from_millis(16);

/// What one plugin instance has asked for since the host last looked, as flags any thread can
/// set and the main-thread pump and the processor read and clear.
#[derive(Debug, Default)]
pub(crate) struct Signals {
    /// `on_main_thread` should be called.
    pub(crate) callback: AtomicBool,
    /// The plugin wants its parameters flushed (applied while it does not process).
    pub(crate) flush: AtomicBool,
    /// The plugin asks to be deactivated and created again (or its ports changed).
    pub(crate) restart: AtomicBool,
    /// Its state changed: `mark_dirty`, a parameter rescan, or a value it changed itself.
    pub(crate) dirty: AtomicBool,
    /// Parameter gestures the plugin has begun and not ended.
    pub(crate) gestures: AtomicI32,
    /// The plugin said its interface closed, as when the user closes its floating window.
    pub(crate) closed: AtomicBool,
    /// A size it asked its window to take, packed as [`GuiSize::pack_to_u64`]; 0 for none.
    pub(crate) resize: AtomicU64,
    /// Its `process` failed.
    pub(crate) failed: AtomicBool,
    /// Notes it was not sent because its table of sounding notes was full.
    pub(crate) dropped_notes: AtomicU64,
    /// Rung with every request the main thread acts on; one bell for all of a host's
    /// instances, so one look tells whether any of them asked.
    pub(crate) doorbell: Arc<AtomicBool>,
}

impl Signals {
    /// No request yet; requests ring `doorbell`.
    pub(crate) fn ringing(doorbell: Arc<AtomicBool>) -> Self {
        Self {
            doorbell,
            ..Self::default()
        }
    }

    /// Takes a flag: whether it was set, clearing it.
    pub(crate) fn take(flag: &AtomicBool) -> bool {
        flag.swap(false, Ordering::AcqRel)
    }

    /// Sets `flag`, one of these signals, and rings the doorbell: two stores, so any thread
    /// may, the audio thread included.
    pub(crate) fn raise(&self, flag: &AtomicBool) {
        flag.store(true, Ordering::Release);
        self.ring();
    }

    /// Rings the doorbell, after a request stored some other way.
    pub(crate) fn ring(&self) {
        self.doorbell.store(true, Ordering::Release);
    }

    /// Whether a request the main thread acts on is still waiting: a callback, a changed
    /// state, a restart, a failure, a closed interface or a size asked for. Why the flags
    /// themselves and not only the doorbell: the window clears the bell as it looks, so a tick
    /// it then could not take (a dialog was up) would leave the request waiting until the
    /// plugin happened to ask again.
    pub(crate) fn pending(&self) -> bool {
        [
            &self.callback,
            &self.dirty,
            &self.restart,
            &self.failed,
            &self.closed,
        ]
        .into_iter()
        .any(|flag| flag.load(Ordering::Acquire))
            || self.resize.load(Ordering::Acquire) != 0
    }
}

/// The host callbacks' types.
pub(crate) struct DawsomeHost;

impl HostHandlers for DawsomeHost {
    type Shared<'a> = SharedState;
    type MainThread<'a> = MainState<'a>;
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &SharedState) {
        builder
            .register::<HostParams>()
            .register::<HostState>()
            .register::<HostGui>()
            .register::<HostTimer>()
            .register::<HostThreadCheck>()
            .register::<HostLatency>()
            .register::<HostAudioPorts>()
            .register::<HostNotePorts>();
        #[cfg(unix)]
        builder.register::<HostPosixFd>();
    }
}

/// The callbacks any thread may make.
pub(crate) struct SharedState {
    pub(crate) signals: Arc<Signals>,
}

impl SharedHandler<'_> for SharedState {
    fn request_restart(&self) {
        self.signals.raise(&self.signals.restart);
    }

    // Dawsome calls `process` on every block while playing, so there is nothing to wake.
    fn request_process(&self) {}

    fn request_callback(&self) {
        self.signals.raise(&self.signals.callback);
    }
}

impl HostParamsImplShared for SharedState {
    // The processor takes it on its next call, played or idle: no main-thread work, no ring.
    fn request_flush(&self) {
        self.signals.flush.store(true, Ordering::Release);
    }
}

/// Whether a plugin's resize request names a size a window can have: both sides over zero.
pub(crate) fn resize_has_both_sides(width: u32, height: u32) -> bool {
    width > 0 && height > 0
}

impl HostGuiImpl for SharedState {
    fn resize_hints_changed(&self) {}

    fn request_resize(&self, new_size: GuiSize) -> Result<(), HostError> {
        // A zero side is refused rather than taken: packed, 0x0 is the word that means "no
        // request", so a yes here would be a request that never arrives (found by review,
        // 2026-10-28), and a window has no zero side to give.
        if !resize_has_both_sides(new_size.width, new_size.height) {
            return Err(HostError::Message("a plugin window has no zero side"));
        }
        self.signals
            .resize
            .store(new_size.pack_to_u64(), Ordering::Release);
        self.signals.ring();
        Ok(())
    }

    fn request_show(&self) -> Result<(), HostError> {
        Ok(())
    }

    fn request_hide(&self) -> Result<(), HostError> {
        Ok(())
    }

    fn closed(&self, _was_destroyed: bool) {
        self.signals.raise(&self.signals.closed);
    }
}

impl HostThreadCheckImpl for SharedState {
    fn is_main_thread(&self) -> bool {
        IS_MAIN.with(Cell::get)
    }

    fn is_audio_thread(&self) -> bool {
        IN_AUDIO.with(Cell::get)
    }
}

/// A timer a plugin registered, and when it is next due.
#[derive(Debug, Clone, Copy)]
struct Timer {
    id: TimerId,
    period: Duration,
    due: Instant,
}

/// The callbacks only the main thread makes, and what they registered: timers and file
/// descriptors, which the main-thread pump serves.
pub(crate) struct MainState<'a> {
    shared: &'a SharedState,
    timers: RefCell<Vec<Timer>>,
    next_timer: Cell<u32>,
    #[cfg(unix)]
    fds: RefCell<Vec<(RawFd, FdFlags)>>,
}

impl<'a> MainState<'a> {
    pub(crate) fn new(shared: &'a SharedState) -> Self {
        Self {
            shared,
            timers: RefCell::new(Vec::new()),
            next_timer: Cell::new(0),
            #[cfg(unix)]
            fds: RefCell::new(Vec::new()),
        }
    }

    /// The timers due at `now`, each moved on to its next time.
    pub(crate) fn due_timers(&self, now: Instant) -> Vec<TimerId> {
        let mut due = Vec::new();
        for timer in self.timers.borrow_mut().iter_mut() {
            if timer.due <= now {
                due.push(timer.id);
                // From now, not from the missed time, so a stalled pump does not fire a burst.
                timer.due = now + timer.period;
            }
        }
        due
    }

    /// The file descriptors registered, with the events each waits for.
    #[cfg(unix)]
    pub(crate) fn fds(&self) -> Vec<(RawFd, FdFlags)> {
        self.fds.borrow().clone()
    }

    /// Whether a timer or a file descriptor is registered: work that comes due without the
    /// plugin asking, so the pump must look on its own.
    pub(crate) fn waits_on_the_clock(&self) -> bool {
        #[cfg(unix)]
        let fds = !self.fds.borrow().is_empty();
        #[cfg(not(unix))]
        let fds = false;
        !self.timers.borrow().is_empty() || fds
    }
}

impl<'a> MainThreadHandler<'a> for MainState<'a> {}

impl HostTimerImpl for MainState<'_> {
    fn register_timer(&self, period_ms: u32) -> Result<TimerId, HostError> {
        let id = TimerId(self.next_timer.get());
        self.next_timer.set(id.0.wrapping_add(1));
        let period = Duration::from_millis(u64::from(period_ms)).max(MIN_TIMER_PERIOD);
        self.timers.borrow_mut().push(Timer {
            id,
            period,
            due: Instant::now() + period,
        });
        Ok(id)
    }

    fn unregister_timer(&self, timer_id: TimerId) -> Result<(), HostError> {
        let mut timers = self.timers.borrow_mut();
        let before = timers.len();
        timers.retain(|timer| timer.id != timer_id);
        if timers.len() == before {
            return Err(HostError::Message("no such timer"));
        }
        Ok(())
    }
}

#[cfg(unix)]
impl HostPosixFdImpl for MainState<'_> {
    fn register_fd(&self, fd: RawFd, flags: FdFlags) -> Result<(), HostError> {
        let mut fds = self.fds.borrow_mut();
        if fds.iter().any(|&(known, _)| known == fd) {
            return Err(HostError::Message("file descriptor already registered"));
        }
        fds.push((fd, flags));
        Ok(())
    }

    fn modify_fd(&self, fd: RawFd, flags: FdFlags) -> Result<(), HostError> {
        let mut fds = self.fds.borrow_mut();
        let entry = fds
            .iter_mut()
            .find(|(known, _)| *known == fd)
            .ok_or(HostError::Message("file descriptor not registered"))?;
        entry.1 = flags;
        Ok(())
    }

    fn unregister_fd(&self, fd: RawFd) -> Result<(), HostError> {
        let mut fds = self.fds.borrow_mut();
        let before = fds.len();
        fds.retain(|&(known, _)| known != fd);
        if fds.len() == before {
            return Err(HostError::Message("file descriptor not registered"));
        }
        Ok(())
    }
}

impl HostParamsImplMainThread for MainState<'_> {
    // A new list or new values: the parameter window reads them again, and the state is
    // captured, which is how the document learns of the change.
    fn rescan(&self, _flags: ParamRescanFlags) {
        self.shared.signals.raise(&self.shared.signals.dirty);
    }

    // Dawsome keeps no automation or modulation of plugin parameters yet.
    fn clear(&self, _param_id: clack_host::utils::ClapId, _flags: ParamClearFlags) {}
}

impl HostStateImpl for MainState<'_> {
    fn mark_dirty(&self) {
        self.shared.signals.raise(&self.shared.signals.dirty);
    }
}

impl HostLatencyImpl for MainState<'_> {
    // Latency is not compensated, so a change needs nothing.
    fn changed(&self) {}
}

impl HostAudioPortsImpl for MainState<'_> {
    // Every change is handled the same way, by creating the plugin again.
    fn is_rescan_flag_supported(&self, _flag: AudioPortRescanFlags) -> bool {
        true
    }

    fn rescan(&self, _flags: AudioPortRescanFlags) {
        self.shared.signals.raise(&self.shared.signals.restart);
    }
}

impl HostNotePortsImpl for MainState<'_> {
    fn supported_dialects(&self) -> NoteDialects {
        NoteDialects::CLAP | NoteDialects::MIDI
    }

    fn rescan(&self, _flags: NotePortRescanFlags) {
        self.shared.signals.raise(&self.shared.signals.restart);
    }
}

#[cfg(test)]
mod tests {
    // A plugin's resize request with a zero side is refused (packed, 0x0 reads as no
    // request); any size with both sides is taken.
    #[test]
    fn a_resize_request_needs_both_sides() {
        assert!(!super::resize_has_both_sides(0, 0));
        assert!(!super::resize_has_both_sides(0, 20));
        assert!(!super::resize_has_both_sides(400, 0));
        assert!(super::resize_has_both_sides(1, 1));
        assert!(super::resize_has_both_sides(400, 300));
    }

    use super::*;

    #[test]
    fn timers_round_up_fire_when_due_and_can_be_removed() {
        let shared = SharedState {
            signals: Arc::default(),
        };
        let main = MainState::new(&shared);
        let fast = main.register_timer(1).expect("registers");
        let slow = main.register_timer(1000).expect("registers");
        assert_ne!(fast, slow);
        let start = Instant::now();
        assert!(main.due_timers(start).is_empty(), "not due yet");
        let after = start + Duration::from_millis(20);
        assert_eq!(main.due_timers(after), [fast], "1 ms rounds up to 16 ms");
        assert!(main.due_timers(after).is_empty(), "moved on");
        main.unregister_timer(fast).expect("removes");
        assert!(main.unregister_timer(fast).is_err());
        assert_eq!(main.due_timers(start + Duration::from_secs(2)), [slow]);
    }

    // The pump looks on its own only while a timer or a file descriptor is registered.
    #[test]
    fn a_timer_or_a_file_descriptor_keeps_the_pump_on_the_clock() {
        let shared = SharedState {
            signals: Arc::default(),
        };
        let main = MainState::new(&shared);
        assert!(!main.waits_on_the_clock());
        let timer = main.register_timer(100).expect("registers");
        assert!(main.waits_on_the_clock());
        main.unregister_timer(timer).expect("removes");
        assert!(!main.waits_on_the_clock());
        // File descriptors are registered on Linux only (see `declare_extensions`).
        #[cfg(unix)]
        {
            main.register_fd(5, FdFlags::READ).expect("registers");
            assert!(main.waits_on_the_clock());
            main.unregister_fd(5).expect("removes");
            assert!(!main.waits_on_the_clock());
        }
    }

    // Every request the main thread acts on rings the one bell its host shares; a flush, which
    // the processor takes itself, does not.
    #[test]
    fn requests_ring_the_shared_doorbell_and_a_flush_does_not() {
        let doorbell = Arc::new(AtomicBool::new(false));
        let shared = SharedState {
            signals: Arc::new(Signals::ringing(Arc::clone(&doorbell))),
        };
        shared.request_flush();
        assert!(
            !doorbell.load(Ordering::Relaxed),
            "a flush is the processor's"
        );
        shared.request_callback();
        assert!(doorbell.swap(false, Ordering::Relaxed));
        assert!(Signals::take(&shared.signals.callback));
        shared.request_restart();
        assert!(doorbell.swap(false, Ordering::Relaxed));
        let main = MainState::new(&shared);
        main.mark_dirty();
        assert!(doorbell.swap(false, Ordering::Relaxed));
        assert!(Signals::take(&shared.signals.dirty));
    }

    // Linux only: file descriptors are a POSIX extension, offered to plugins there alone.
    #[cfg(unix)]
    #[test]
    fn file_descriptors_register_once_and_change_their_events() {
        let shared = SharedState {
            signals: Arc::default(),
        };
        let main = MainState::new(&shared);
        main.register_fd(5, FdFlags::READ).expect("registers");
        assert!(main.register_fd(5, FdFlags::WRITE).is_err());
        main.modify_fd(5, FdFlags::WRITE).expect("modifies");
        assert_eq!(main.fds(), [(5, FdFlags::WRITE)]);
        main.unregister_fd(5).expect("removes");
        assert!(main.unregister_fd(5).is_err());
        assert!(main.fds().is_empty());
    }
}
