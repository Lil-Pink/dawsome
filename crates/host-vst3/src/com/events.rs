//! The note events a block sends a plugin (`IEventList`): a list of fixed room, filled by the
//! audio half before each `process` and read by the plugin inside it, so a block allocates
//! nothing. The list a plugin may write its own events into is one too, with no room: its events
//! are not used.

use std::cell::{Cell, UnsafeCell};

use vst3::Steinberg::Vst::{Event, IEventList, IEventListTrait};
use vst3::Steinberg::{int32, kInvalidArgument, kResultFalse, kResultOk, tresult};
use vst3::{Class, ComWrapper};

use super::{guarded, Counted};

pub(crate) struct EventList {
    events: UnsafeCell<Vec<Event>>,
    len: Cell<usize>,
    room: usize,
    _counted: Counted,
}

// SAFETY: the list is used by one thread at a time: made on the main thread, then only the audio
// half's thread fills it and the plugin reads it inside the `process` call that thread makes
// (the format allows no other use of it). It is handed over whole when the audio half moves.
unsafe impl Send for EventList {}
// SAFETY: as above; `Sync` only so the wrapper may move with the audio half.
unsafe impl Sync for EventList {}

impl Class for EventList {
    type Interfaces = (IEventList,);
}

impl EventList {
    /// A list with room for `room` events, all made now.
    pub(crate) fn new(room: usize) -> ComWrapper<Self> {
        // SAFETY: an `Event` is plain data (numbers and a union of plain structs), for which
        // all zero bytes are a value.
        let blank: Event = unsafe { std::mem::zeroed() };
        ComWrapper::new(Self {
            events: UnsafeCell::new(vec![blank; room]),
            len: Cell::new(0),
            room,
            _counted: Counted::new(),
        })
    }

    /// Empties the list for the next block.
    pub(crate) fn clear(&self) {
        self.len.set(0);
    }

    /// Adds `event`; false when there is no room.
    pub(crate) fn push(&self, event: Event) -> bool {
        let len = self.len.get();
        if len >= self.room {
            return false;
        }
        // SAFETY: one thread at a time (see the `Send` impl), and no reference into the list
        // outlives a method.
        unsafe { (&mut *self.events.get())[len] = event };
        self.len.set(len + 1);
        true
    }
}

impl IEventListTrait for EventList {
    unsafe fn getEventCount(&self) -> int32 {
        self.len.get() as int32
    }

    unsafe fn getEvent(&self, index: int32, event: *mut Event) -> tresult {
        guarded(|| {
            let Ok(index) = usize::try_from(index) else {
                return kInvalidArgument;
            };
            if index >= self.len.get() || event.is_null() {
                return kInvalidArgument;
            }
            // SAFETY: as in `push`: one thread at a time, and this reference ends with the line.
            let found = unsafe { &*self.events.get() }[index];
            // SAFETY: `event` is the plugin's out parameter, non-null (checked).
            unsafe { *event = found };
            kResultOk
        })
    }

    unsafe fn addEvent(&self, event: *mut Event) -> tresult {
        guarded(|| {
            // SAFETY: the plugin's event, read for the call.
            match unsafe { event.as_ref() } {
                Some(event) if self.push(*event) => kResultOk,
                Some(_) => kResultFalse,
                None => kInvalidArgument,
            }
        })
    }
}
