//! The parameter changes of a block (`IParameterChanges`, one `IParamValueQueue` per
//! parameter), in fixed room: [`QUEUES`] parameters of [`POINTS`] points each, made with the
//! audio half, so a block allocates nothing. The host fills the input changes before `process`;
//! the plugin fills the output changes inside it, which the host reads after.

use std::cell::{Cell, UnsafeCell};

use vst3::Steinberg::Vst::{
    IParamValueQueue, IParamValueQueueTrait, IParameterChanges, IParameterChangesTrait, ParamID,
    ParamValue,
};
use vst3::Steinberg::{int32, kInvalidArgument, kResultFalse, kResultOk, tresult};
use vst3::{Class, ComWrapper};

use super::{guard, guarded, Counted};

/// Parameters one block can change.
pub(crate) const QUEUES: usize = 64;
/// Points one parameter can have in a block.
pub(crate) const POINTS: usize = 4;

pub(crate) struct ParamValueQueue {
    id: Cell<ParamID>,
    points: UnsafeCell<[(int32, ParamValue); POINTS]>,
    len: Cell<usize>,
    _counted: Counted,
}

// SAFETY: as `EventList`'s: one thread at a time, the audio half's, and the plugin inside the
// `process` call it makes.
unsafe impl Send for ParamValueQueue {}
// SAFETY: as above.
unsafe impl Sync for ParamValueQueue {}

impl Class for ParamValueQueue {
    type Interfaces = (IParamValueQueue,);
}

impl ParamValueQueue {
    fn new() -> ComWrapper<Self> {
        ComWrapper::new(Self {
            id: Cell::new(0),
            points: UnsafeCell::new([(0, 0.0); POINTS]),
            len: Cell::new(0),
            _counted: Counted::new(),
        })
    }

    fn reset(&self, id: ParamID) {
        self.id.set(id);
        self.len.set(0);
    }

    /// Adds a point, kept in offset order; a point on an offset already there replaces it.
    /// False when there is no room.
    fn add(&self, offset: int32, value: ParamValue) -> Option<usize> {
        // SAFETY: one thread at a time; no reference into the points outlives this method.
        let points = unsafe { &mut *self.points.get() };
        let len = self.len.get();
        let at = points[..len]
            .iter()
            .position(|&(known, _)| known >= offset)
            .unwrap_or(len);
        if at < len && points[at].0 == offset {
            points[at].1 = value;
            return Some(at);
        }
        if len >= POINTS {
            return None;
        }
        points.copy_within(at..len, at + 1);
        points[at] = (offset, value);
        self.len.set(len + 1);
        Some(at)
    }

    /// The last point's value, which the parameter has at the block's end.
    pub(crate) fn last(&self) -> Option<ParamValue> {
        let len = self.len.get();
        // SAFETY: as in `add`.
        (len > 0).then(|| unsafe { (&*self.points.get())[len - 1].1 })
    }

    pub(crate) fn id(&self) -> ParamID {
        self.id.get()
    }
}

impl IParamValueQueueTrait for ParamValueQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id.get()
    }

    unsafe fn getPointCount(&self) -> int32 {
        self.len.get() as int32
    }

    unsafe fn getPoint(&self, index: int32, offset: *mut int32, value: *mut ParamValue) -> tresult {
        guarded(|| {
            let Ok(index) = usize::try_from(index) else {
                return kInvalidArgument;
            };
            if index >= self.len.get() || offset.is_null() || value.is_null() {
                return kInvalidArgument;
            }
            // SAFETY: as in `add`: one thread at a time, and this reference ends with the line.
            let (at, to) = unsafe { &*self.points.get() }[index];
            // SAFETY: the plugin's out parameter, non-null (checked).
            unsafe { *offset = at };
            // SAFETY: as for `offset`.
            unsafe { *value = to };
            kResultOk
        })
    }

    unsafe fn addPoint(&self, offset: int32, value: ParamValue, index: *mut int32) -> tresult {
        guarded(|| match self.add(offset, value) {
            Some(at) => {
                if !index.is_null() {
                    // SAFETY: the plugin's out parameter.
                    unsafe { *index = at as int32 };
                }
                kResultOk
            }
            None => kResultFalse,
        })
    }
}

pub(crate) struct ParameterChanges {
    queues: Vec<ComWrapper<ParamValueQueue>>,
    used: Cell<usize>,
    _counted: Counted,
}

// SAFETY: as `ParamValueQueue`'s.
unsafe impl Send for ParameterChanges {}
// SAFETY: as above.
unsafe impl Sync for ParameterChanges {}

impl Class for ParameterChanges {
    type Interfaces = (IParameterChanges,);
}

impl ParameterChanges {
    pub(crate) fn new() -> ComWrapper<Self> {
        ComWrapper::new(Self {
            queues: (0..QUEUES).map(|_| ParamValueQueue::new()).collect(),
            used: Cell::new(0),
            _counted: Counted::new(),
        })
    }

    pub(crate) fn clear(&self) {
        self.used.set(0);
    }

    /// The queue of parameter `id`, taken from the free ones when it has none yet; `None` when
    /// every queue is taken.
    fn queue_of(&self, id: ParamID) -> Option<(usize, &ComWrapper<ParamValueQueue>)> {
        let used = self.used.get();
        if let Some(found) = self.queues[..used]
            .iter()
            .position(|queue| queue.id() == id)
        {
            return Some((found, &self.queues[found]));
        }
        let queue = self.queues.get(used)?;
        queue.reset(id);
        self.used.set(used + 1);
        Some((used, queue))
    }

    /// Parameter `id` takes `value` from the block's first frame; false when there is no room.
    pub(crate) fn set_at_start(&self, id: ParamID, value: ParamValue) -> bool {
        self.set_at(id, 0, value)
    }

    /// Parameter `id` takes `value` from frame `offset` of the block, replacing a point already
    /// there; false when there is no room.
    pub(crate) fn set_at(&self, id: ParamID, offset: u32, value: ParamValue) -> bool {
        let offset = int32::try_from(offset).unwrap_or(int32::MAX);
        self.queue_of(id)
            .and_then(|(_, queue)| queue.add(offset, value))
            .is_some()
    }

    /// Each changed parameter and the value it ends the block on.
    pub(crate) fn last_values(&self) -> impl Iterator<Item = (ParamID, ParamValue)> + '_ {
        self.queues[..self.used.get()]
            .iter()
            .filter_map(|queue| Some((queue.id(), queue.last()?)))
    }
}

impl IParameterChangesTrait for ParameterChanges {
    unsafe fn getParameterCount(&self) -> int32 {
        self.used.get() as int32
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        guard(std::ptr::null_mut(), || {
            // No reference added: the changes keep their queues, as the SDK's do.
            usize::try_from(index)
                .ok()
                .filter(|&index| index < self.used.get())
                .and_then(|index| self.queues[index].as_com_ref::<IParamValueQueue>())
                .map_or(std::ptr::null_mut(), |queue| queue.as_ptr())
        })
    }

    unsafe fn addParameterData(
        &self,
        id: *const ParamID,
        index: *mut int32,
    ) -> *mut IParamValueQueue {
        guard(std::ptr::null_mut(), || {
            // SAFETY: the plugin's id, read for the call.
            let Some(&id) = (unsafe { id.as_ref() }) else {
                return std::ptr::null_mut();
            };
            let Some((at, queue)) = self.queue_of(id) else {
                return std::ptr::null_mut();
            };
            if !index.is_null() {
                // SAFETY: the plugin's out parameter.
                unsafe { *index = at as int32 };
            }
            queue
                .as_com_ref::<IParamValueQueue>()
                .map_or(std::ptr::null_mut(), |queue| queue.as_ptr())
        })
    }
}

#[cfg(test)]
mod tests {
    use vst3::ComRef;

    use super::*;

    // A parameter set twice in a block has one queue, its newer value at offset 0; points come
    // back in offset order, a point on a taken offset replacing it; the changes have room for
    // QUEUES parameters, the queues for POINTS points.
    #[test]
    fn points_keep_their_order_and_the_room_is_fixed() {
        let changes = ParameterChanges::new();
        assert!(changes.set_at_start(7, 0.5));
        assert!(changes.set_at_start(7, 0.25));
        assert_eq!(changes.last_values().collect::<Vec<_>>(), [(7, 0.25)]);
        let as_plugin: ComRef<'_, IParameterChanges> = changes.as_com_ref().expect("changes");
        // Every call below is into our own changes or queue through its interface, with
        // pointers to locals of this test that outlive the call.
        // SAFETY: as just said.
        assert_eq!(unsafe { as_plugin.getParameterCount() }, 1);
        let (mut index, id) = (0, 9);
        // SAFETY: as said at the top.
        let added = unsafe { as_plugin.addParameterData(&id, &mut index) };
        // SAFETY: the changes keep their queues while `changes` lives.
        let queue = unsafe { ComRef::from_raw(added) }.expect("a queue");
        assert_eq!(index, 1);
        for (offset, value) in [(30, 0.3), (10, 0.1), (20, 0.2), (10, 0.15), (40, 0.4)] {
            assert_eq!(
                // SAFETY: as said at the top.
                unsafe { queue.addPoint(offset, value, &mut index) },
                kResultOk
            );
        }
        // SAFETY: as said at the top.
        let full = unsafe { queue.addPoint(50, 0.5, &mut index) };
        assert_eq!(full, kResultFalse, "no room");
        // SAFETY: as said at the top.
        let count = unsafe { queue.getPointCount() };
        let points: Vec<(int32, ParamValue)> = (0..count)
            .map(|at| {
                let (mut offset, mut value) = (0, 0.0);
                // SAFETY: as said at the top.
                unsafe { queue.getPoint(at, &mut offset, &mut value) };
                (offset, value)
            })
            .collect();
        assert_eq!(points, [(10, 0.15), (20, 0.2), (30, 0.3), (40, 0.4)]);
        changes.clear();
        for id in 0..QUEUES as u32 {
            assert!(changes.set_at_start(id, 1.0));
        }
        assert!(
            !changes.set_at_start(QUEUES as u32, 1.0),
            "every queue taken"
        );
    }
}
