//! What a plugin's controller tells its host (`IComponentHandler`, `IComponentHandler2`): the
//! edits of its own parameters, gestures, the restarts it asks for and a changed state. Each
//! becomes a flag in [`Signals`] or a value on the instance's edit ring, never a call into the
//! host's tables, so a plugin may make these calls in the middle of one the host makes into it.
//! The calls are documented as made on the plugins' main thread; made from another, they still
//! neither block nor allocate (the edit ring is taken only if free).

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use vst3::Steinberg::Vst::{
    IComponentHandler, IComponentHandler2, IComponentHandler2Trait, IComponentHandlerTrait,
    ParamID, ParamValue, RestartFlags_,
};
use vst3::Steinberg::{int32, kResultFalse, kResultOk, tresult, FIDString, TBool};
use vst3::{Class, ComWrapper};

use super::{guarded, Counted};

/// What one instance has asked for since the host last looked: flags any thread can set and
/// the main-thread pump reads and clears.
#[derive(Debug, Default)]
pub(crate) struct Signals {
    /// Its state changed: an edit outside a gesture, a gesture's end, `setDirty`, new values.
    pub(crate) dirty: AtomicBool,
    /// The restart flags it asked for and the host has not acted on.
    pub(crate) restart: AtomicI32,
    /// Its `process` failed.
    pub(crate) failed: AtomicBool,
    /// Parameter gestures begun and not ended.
    pub(crate) gestures: AtomicI32,
    /// Notes it was not sent because its table of sounding notes was full.
    pub(crate) dropped_notes: AtomicU64,
    /// Values the audio half could not hand back (its ring to the main thread was full).
    pub(crate) dropped_values: AtomicU64,
    /// Rung with every request the main thread acts on, the audio thread's values included; one
    /// bell for all of a host's instances.
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

    /// Sets `flag` and rings the doorbell: two stores, so any thread may, the audio thread
    /// included.
    pub(crate) fn raise(&self, flag: &AtomicBool) {
        flag.store(true, Ordering::Release);
        self.ring();
    }

    pub(crate) fn ring(&self) {
        self.doorbell.store(true, Ordering::Release);
    }

    /// Whether a request the main thread acts on is still waiting. Why the flags themselves and
    /// not only the doorbell: the window clears the bell as it looks, so a tick it then could
    /// not take would leave the request waiting until the plugin asked again (the CLAP host's
    /// rule).
    pub(crate) fn pending(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
            || self.failed.load(Ordering::Acquire)
            || self.restart.load(Ordering::Acquire) != 0
    }

    /// Forgets what the host's own state load made the plugin say: a changed state, and new
    /// values. Why: capturing that would put the plugin's own reading of the document's state
    /// back into the document as a change the user never made (the CLAP host's
    /// `forget_own_load`).
    pub(crate) fn forget_own_load(&self) {
        Self::take(&self.dirty);
        self.restart
            .fetch_and(!RestartFlags_::kParamValuesChanged, Ordering::AcqRel);
    }

    /// Forgets every request a new instance made before it was activated (its state load and
    /// connection included). Why all of them: activation reads its buses, latency, parameters and
    /// MIDI mapping afresh, so none is left to act on, and a reload asked inside the load that
    /// made it would otherwise have it made again, and again.
    pub(crate) fn forget_before_activation(&self) {
        Self::take(&self.dirty);
        self.restart.store(0, Ordering::Release);
    }
}

/// An edit the controller made, on its way to the audio half.
pub(crate) type Edit = (ParamID, ParamValue);

/// Room for the controller's edits between two blocks.
pub(crate) const EDITS: usize = 256;

pub(crate) struct ComponentHandler {
    signals: Arc<Signals>,
    edits: Mutex<rtrb::Producer<Edit>>,
    _counted: Counted,
}

impl Class for ComponentHandler {
    type Interfaces = (IComponentHandler, IComponentHandler2);
}

impl ComponentHandler {
    pub(crate) fn new(signals: Arc<Signals>, edits: rtrb::Producer<Edit>) -> ComWrapper<Self> {
        ComWrapper::new(Self {
            signals,
            edits: Mutex::new(edits),
            _counted: Counted::new(),
        })
    }
}

impl IComponentHandlerTrait for ComponentHandler {
    unsafe fn beginEdit(&self, _id: ParamID) -> tresult {
        guarded(|| {
            self.signals.gestures.fetch_add(1, Ordering::AcqRel);
            kResultOk
        })
    }

    unsafe fn performEdit(&self, id: ParamID, value: ParamValue) -> tresult {
        guarded(|| {
            // Never waits: a ring busy on another thread (a plugin calling from two at once) or
            // full drops the edit.
            if let Ok(mut edits) = self.edits.try_lock() {
                let _ = edits.push((id, value));
            }
            if self.signals.gestures.load(Ordering::Acquire) <= 0 {
                self.signals.raise(&self.signals.dirty);
            }
            kResultOk
        })
    }

    unsafe fn endEdit(&self, _id: ParamID) -> tresult {
        guarded(|| {
            self.signals.gestures.fetch_sub(1, Ordering::AcqRel);
            self.signals.raise(&self.signals.dirty);
            kResultOk
        })
    }

    unsafe fn restartComponent(&self, flags: int32) -> tresult {
        guarded(|| {
            self.signals.restart.fetch_or(flags, Ordering::AcqRel);
            self.signals.ring();
            kResultOk
        })
    }
}

impl IComponentHandler2Trait for ComponentHandler {
    unsafe fn setDirty(&self, state: TBool) -> tresult {
        guarded(|| {
            if state != 0 {
                self.signals.raise(&self.signals.dirty);
            }
            kResultOk
        })
    }

    // The plugin's own window is view.rs's.
    unsafe fn requestOpenEditor(&self, _name: FIDString) -> tresult {
        kResultFalse
    }

    unsafe fn startGroupEdit(&self) -> tresult {
        kResultOk
    }

    unsafe fn finishGroupEdit(&self) -> tresult {
        kResultOk
    }
}

#[cfg(test)]
mod tests {
    use vst3::ComRef;

    use super::*;

    // An edit outside a gesture goes to the audio half and marks the state changed; inside one,
    // only the gesture's end does. A restart rings the bell and keeps its flags; the host's own
    // load forgets the changed state and new values it made the plugin report, not a reload.
    #[test]
    fn edits_reach_the_ring_and_requests_ring_the_bell() {
        let doorbell = Arc::new(AtomicBool::new(false));
        let signals = Arc::new(Signals::ringing(Arc::clone(&doorbell)));
        let (producer, mut consumer) = rtrb::RingBuffer::new(EDITS);
        let handler = ComponentHandler::new(Arc::clone(&signals), producer);
        let handler: ComRef<'_, IComponentHandler> = handler.as_com_ref().expect("a handler");
        // Every call below is into our own handler through its interface, with plain values.
        // SAFETY: as just said.
        unsafe { handler.beginEdit(0) };
        // SAFETY: as said at the top.
        unsafe { handler.performEdit(0, 0.25) };
        assert!(!signals.dirty.load(Ordering::Acquire), "inside a gesture");
        // SAFETY: as said at the top.
        unsafe { handler.endEdit(0) };
        assert!(Signals::take(&signals.dirty));
        // SAFETY: as said at the top.
        unsafe { handler.performEdit(1, 0.5) };
        assert!(Signals::take(&signals.dirty));
        assert_eq!(consumer.pop(), Ok((0, 0.25)));
        assert_eq!(consumer.pop(), Ok((1, 0.5)));
        assert!(doorbell.swap(false, Ordering::AcqRel));
        let reload = RestartFlags_::kReloadComponent | RestartFlags_::kParamValuesChanged;
        // SAFETY: as said at the top.
        unsafe { handler.restartComponent(reload) };
        assert!(doorbell.swap(false, Ordering::AcqRel));
        assert!(signals.pending());
        signals.forget_own_load();
        assert_eq!(
            signals.restart.load(Ordering::Acquire),
            RestartFlags_::kReloadComponent
        );
        // Before activation every request is forgotten, a reload included: activation reads it
        // all afresh, and a reload asked inside the load that made the instance would otherwise
        // make it again, and again.
        signals.raise(&signals.dirty);
        // SAFETY: as said at the top.
        unsafe { handler.restartComponent(RestartFlags_::kLatencyChanged) };
        signals.forget_before_activation();
        assert!(!signals.pending(), "nothing left to act on");
    }
}
