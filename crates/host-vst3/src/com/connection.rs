//! The connection between a plugin's component and its controller when they are two objects:
//! each is connected to a proxy of ours that passes its messages on to the other, as the SDK's
//! host does (`connectionproxy.cpp`), and only on the plugins' main thread. Why a proxy and not
//! the two connected straight: a message sent from the audio thread (some plugins send one every
//! block) would run the other side's code there, which may allocate or lock; the SDK's proxy
//! drops it, and so does this one.

use std::sync::Mutex;

use vst3::Steinberg::Vst::{IConnectionPoint, IConnectionPointTrait, IMessage};
use vst3::Steinberg::{kInvalidArgument, kResultFalse, kResultOk, tresult};
use vst3::{Class, ComPtr, ComWrapper};

use super::{guarded, is_main_thread, Counted};

pub(crate) struct ConnectionProxy {
    /// Where messages go; none once disconnected.
    to: Mutex<Option<ComPtr<IConnectionPoint>>>,
    _counted: Counted,
}

impl Class for ConnectionProxy {
    type Interfaces = (IConnectionPoint,);
}

impl ConnectionProxy {
    /// A proxy that passes messages on to `to`.
    pub(crate) fn new(to: ComPtr<IConnectionPoint>) -> ComWrapper<Self> {
        ComWrapper::new(Self {
            to: Mutex::new(Some(to)),
            _counted: Counted::new(),
        })
    }

    /// Lets go of where messages went: nothing is passed on any more.
    pub(crate) fn cut(&self) {
        let gone = self.to.lock().ok().and_then(|mut to| to.take());
        drop(gone);
    }
}

impl IConnectionPointTrait for ConnectionProxy {
    // The host connects the parts itself; the plugin calling these on the proxy changes
    // nothing.
    unsafe fn connect(&self, _other: *mut IConnectionPoint) -> tresult {
        kResultOk
    }

    unsafe fn disconnect(&self, _other: *mut IConnectionPoint) -> tresult {
        kResultOk
    }

    unsafe fn notify(&self, message: *mut IMessage) -> tresult {
        guarded(|| {
            if message.is_null() {
                return kInvalidArgument;
            }
            if !is_main_thread() {
                return kResultFalse;
            }
            // Passed on with no lock held, so the other side may answer from inside its notify.
            let Some(to) = self.to.lock().ok().and_then(|to| to.clone()) else {
                return kResultFalse;
            };
            // SAFETY: `to` is the other part's live connection point; the message is the
            // plugin's, live for the call.
            unsafe { to.notify(message) }
        })
    }
}
