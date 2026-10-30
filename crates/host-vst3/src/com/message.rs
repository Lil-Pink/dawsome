//! Messages between a plugin's component and its controller (`IMessage`) and the attribute
//! lists they carry (`IAttributeList`: integers, floats, UTF-16 strings, binary data), made by
//! the host when a plugin asks for one (`IHostApplication::createInstance`).

use std::collections::BTreeMap;
use std::ffi::{c_void, CStr, CString};
use std::ptr;
use std::slice;
use std::sync::Mutex;

use vst3::Steinberg::Vst::IAttributeList_::AttrID;
use vst3::Steinberg::Vst::{IAttributeList, IAttributeListTrait, IMessage, IMessageTrait, TChar};
use vst3::Steinberg::{
    int64, kInvalidArgument, kResultFalse, kResultOk, tresult, uint32, FIDString,
};
use vst3::{Class, ComWrapper};

use super::{guard, guarded, Counted};

/// One attribute's value.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Int(i64),
    Float(f64),
    /// UTF-16, without its ending 0.
    String(Vec<TChar>),
    Binary(Vec<u8>),
}

pub(crate) struct AttributeList {
    values: Mutex<BTreeMap<Vec<u8>, Value>>,
    _counted: Counted,
}

impl Class for AttributeList {
    type Interfaces = (IAttributeList,);
}

impl AttributeList {
    pub(crate) fn new() -> ComWrapper<Self> {
        ComWrapper::new(Self {
            values: Mutex::new(BTreeMap::new()),
            _counted: Counted::new(),
        })
    }

    /// Stores `value` under attribute id `id`.
    ///
    /// # Safety
    ///
    /// `id` is null or points to a NUL-terminated string that is live for the call.
    unsafe fn set(&self, id: AttrID, value: Value) -> tresult {
        // SAFETY: the caller's: `id` is null or a NUL-terminated string live for the call.
        let Some(key) = (unsafe { key(id) }) else {
            return kInvalidArgument;
        };
        let Ok(mut values) = self.values.lock() else {
            return kInvalidArgument;
        };
        values.insert(key, value);
        kResultOk
    }

    /// Reads the attribute id `id` through `take`.
    ///
    /// # Safety
    ///
    /// `id` is null or points to a NUL-terminated string that is live for the call.
    unsafe fn get<T>(&self, id: AttrID, take: impl FnOnce(&Value) -> Option<T>) -> Option<T> {
        // SAFETY: the caller's, as for `set`.
        let key = unsafe { key(id) }?;
        let values = self.values.lock().ok()?;
        take(values.get(&key)?)
    }
}

/// The bytes of attribute id `id`.
unsafe fn key(id: AttrID) -> Option<Vec<u8>> {
    if id.is_null() {
        return None;
    }
    // SAFETY: the caller's: `id` is a NUL-terminated string.
    Some(unsafe { CStr::from_ptr(id) }.to_bytes().to_vec())
}

impl IAttributeListTrait for AttributeList {
    unsafe fn setInt(&self, id: AttrID, value: int64) -> tresult {
        guarded(|| {
            // SAFETY: `id` is what the plugin handed this `IAttributeList` method: null or a NUL-terminated
            // string live for the call (`AttrID`).
            unsafe { self.set(id, Value::Int(value)) }
        })
    }

    unsafe fn getInt(&self, id: AttrID, value: *mut int64) -> tresult {
        guarded(|| {
            // SAFETY: `id` is what the plugin handed this `IAttributeList` method: null or a NUL-terminated
            // string live for the call (`AttrID`).
            let found = unsafe {
                self.get(id, |value| match value {
                    Value::Int(int) => Some(*int),
                    _ => None,
                })
            };
            match (found, value.is_null()) {
                (Some(found), false) => {
                    // SAFETY: an out parameter the plugin owns.
                    unsafe { *value = found };
                    kResultOk
                }
                _ => kResultFalse,
            }
        })
    }

    unsafe fn setFloat(&self, id: AttrID, value: f64) -> tresult {
        guarded(|| {
            // SAFETY: `id` is what the plugin handed this `IAttributeList` method: null or a NUL-terminated
            // string live for the call (`AttrID`).
            unsafe { self.set(id, Value::Float(value)) }
        })
    }

    unsafe fn getFloat(&self, id: AttrID, value: *mut f64) -> tresult {
        guarded(|| {
            // SAFETY: `id` is what the plugin handed this `IAttributeList` method: null or a NUL-terminated
            // string live for the call (`AttrID`).
            let found = unsafe {
                self.get(id, |value| match value {
                    Value::Float(float) => Some(*float),
                    _ => None,
                })
            };
            match (found, value.is_null()) {
                (Some(found), false) => {
                    // SAFETY: an out parameter the plugin owns.
                    unsafe { *value = found };
                    kResultOk
                }
                _ => kResultFalse,
            }
        })
    }

    unsafe fn setString(&self, id: AttrID, string: *const TChar) -> tresult {
        guarded(|| {
            if string.is_null() {
                return kInvalidArgument;
            }
            // SAFETY: `string` is non-null (checked) and NUL-terminated UTF-16, live for the call
            // (`IAttributeList::setString`), so every unit up to its 0 may be read.
            let len = (0..)
                .take_while(|&at| unsafe { *string.wrapping_add(at) } != 0)
                .count();
            // SAFETY: the `len` units before the 0, just counted.
            let units = unsafe { slice::from_raw_parts(string, len) };
            // SAFETY: `id` is what the plugin handed this `IAttributeList` method: null or a NUL-terminated
            // string live for the call (`AttrID`).
            unsafe { self.set(id, Value::String(units.to_vec())) }
        })
    }

    unsafe fn getString(&self, id: AttrID, string: *mut TChar, size_in_bytes: uint32) -> tresult {
        guarded(|| {
            // SAFETY: `id` is what the plugin handed this `IAttributeList` method: null or a NUL-terminated
            // string live for the call (`AttrID`).
            let found = unsafe {
                self.get(id, |value| match value {
                    Value::String(units) => Some(units.clone()),
                    _ => None,
                })
            };
            let room = size_in_bytes as usize / std::mem::size_of::<TChar>();
            match found {
                Some(units) if !string.is_null() && room > 0 => {
                    let len = units.len().min(room - 1);
                    // SAFETY: `string` is non-null (checked) and the plugin's buffer of
                    // `size_in_bytes` bytes, `room` units (`IAttributeList::getString`); `len + 1`
                    // is at most `room`.
                    let to = unsafe { slice::from_raw_parts_mut(string, len + 1) };
                    to[..len].copy_from_slice(&units[..len]);
                    to[len] = 0;
                    kResultOk
                }
                _ => kResultFalse,
            }
        })
    }

    unsafe fn setBinary(&self, id: AttrID, data: *const c_void, size_in_bytes: uint32) -> tresult {
        guarded(|| {
            let size = size_in_bytes as usize;
            if data.is_null() && size > 0 {
                return kInvalidArgument;
            }
            let bytes = if size == 0 {
                Vec::new()
            } else {
                // SAFETY: the plugin hands `size_in_bytes` bytes (`IAttributeList::setBinary`).
                unsafe { slice::from_raw_parts(data.cast::<u8>(), size) }.to_vec()
            };
            // SAFETY: `id` is what the plugin handed this `IAttributeList` method: null or a NUL-terminated
            // string live for the call (`AttrID`).
            unsafe { self.set(id, Value::Binary(bytes)) }
        })
    }

    unsafe fn getBinary(
        &self,
        id: AttrID,
        data: *mut *const c_void,
        size_in_bytes: *mut uint32,
    ) -> tresult {
        guarded(|| {
            if data.is_null() || size_in_bytes.is_null() {
                return kInvalidArgument;
            }
            // The data stays where it is until the attribute is set again or the list goes, as
            // the SDK's list keeps it: the pointer handed out points into it.
            // SAFETY: `id` is what the plugin handed this `IAttributeList` method: null or a NUL-terminated
            // string live for the call (`AttrID`).
            let found = unsafe {
                self.get(id, |value| match value {
                    Value::Binary(bytes) => Some((bytes.as_ptr(), bytes.len())),
                    _ => None,
                })
            };
            match found {
                Some((at, len)) => {
                    // SAFETY: `data` is the plugin's out parameter, non-null (checked).
                    unsafe { *data = at.cast() };
                    // SAFETY: as for `data`.
                    unsafe { *size_in_bytes = len as uint32 };
                    kResultOk
                }
                None => kResultFalse,
            }
        })
    }
}

pub(crate) struct Message {
    id: Mutex<Option<CString>>,
    attributes: ComWrapper<AttributeList>,
    _counted: Counted,
}

impl Class for Message {
    type Interfaces = (IMessage,);
}

impl Message {
    pub(crate) fn new() -> ComWrapper<Self> {
        ComWrapper::new(Self {
            id: Mutex::new(None),
            attributes: AttributeList::new(),
            _counted: Counted::new(),
        })
    }
}

impl IMessageTrait for Message {
    unsafe fn getMessageID(&self) -> FIDString {
        guard(ptr::null(), || {
            // The text stays where it is until the id is set again: the pointer handed out
            // points into it, as the SDK's message keeps it.
            self.id
                .lock()
                .ok()
                .and_then(|id| id.as_ref().map(|id| id.as_ptr()))
                .unwrap_or(ptr::null())
        })
    }

    unsafe fn setMessageID(&self, id: FIDString) {
        guard((), || {
            let text = if id.is_null() {
                None
            } else {
                // SAFETY: a message id is a NUL-terminated string (`FIDString`).
                Some(unsafe { CStr::from_ptr(id) }.to_owned())
            };
            if let Ok(mut held) = self.id.lock() {
                *held = text;
            }
        });
    }

    unsafe fn getAttributes(&self) -> *mut IAttributeList {
        guard(ptr::null_mut(), || {
            // No reference added: the message keeps its list, as the SDK's does.
            self.attributes
                .as_com_ref::<IAttributeList>()
                .map_or(ptr::null_mut(), |list| list.as_ptr())
        })
    }
}

/// The text of a message's id, for tests.
///
/// # Safety
/// `message` keeps the `IMessage` contract: its id is null or a NUL-terminated string.
#[cfg(test)]
pub(crate) unsafe fn id_of(message: &vst3::ComPtr<IMessage>) -> Option<String> {
    // SAFETY: `message` is live (a `ComPtr` holds a reference).
    let id = unsafe { message.getMessageID() };
    // SAFETY: the caller's: a non-null id is NUL-terminated, kept by the message.
    (!id.is_null()).then(|| unsafe { CStr::from_ptr(id) }.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use vst3::ComRef;

    use super::*;

    // Each kind of attribute reads back as it was set and only as that kind; a string longer
    // than the reader's buffer is cut and ended; binary data comes back by pointer.
    #[test]
    fn attributes_read_back_as_they_were_set() {
        let message = Message::new();
        let message = message.to_com_ptr::<IMessage>().expect("a message");
        // Every call below is into our own message or list through its interface, and every
        // pointer passed is to a constant or a local of this test that outlives the call.
        // SAFETY: as just said.
        unsafe { message.setMessageID(c"hello".as_ptr()) };
        // SAFETY: our message keeps the `IMessage` contract.
        assert_eq!(unsafe { id_of(&message) }.as_deref(), Some("hello"));
        // SAFETY: as said at the top.
        let attributes = unsafe { message.getAttributes() };
        // SAFETY: the message keeps its list alive while `message` lives.
        let list = unsafe { ComRef::from_raw(attributes) }.expect("a list");
        // SAFETY: as said at the top.
        assert_eq!(unsafe { list.setInt(c"n".as_ptr(), 7) }, kResultOk);
        let mut n = 0;
        // SAFETY: as said at the top.
        assert_eq!(unsafe { list.getInt(c"n".as_ptr(), &mut n) }, kResultOk);
        assert_eq!(n, 7);
        let mut float = 0.0;
        assert_eq!(
            // SAFETY: as said at the top.
            unsafe { list.getFloat(c"n".as_ptr(), &mut float) },
            kResultFalse,
            "not a float"
        );
        // SAFETY: as said at the top.
        assert_eq!(unsafe { list.setFloat(c"f".as_ptr(), 0.25) }, kResultOk);
        assert_eq!(
            // SAFETY: as said at the top.
            unsafe { list.getFloat(c"f".as_ptr(), &mut float) },
            kResultOk
        );
        assert_eq!(float, 0.25);
        let text: Vec<TChar> = "abc\0".encode_utf16().collect();
        assert_eq!(
            // SAFETY: as said at the top; `text` ends with its 0.
            unsafe { list.setString(c"s".as_ptr(), text.as_ptr()) },
            kResultOk
        );
        let mut short = [0 as TChar; 3];
        assert_eq!(
            // SAFETY: as said at the top; `short` is 6 bytes.
            unsafe { list.getString(c"s".as_ptr(), short.as_mut_ptr(), 6) },
            kResultOk
        );
        assert_eq!(short, ['a' as TChar, 'b' as TChar, 0]);
        let bytes = [1u8, 2, 3];
        assert_eq!(
            // SAFETY: as said at the top; `bytes` is 3 bytes.
            unsafe { list.setBinary(c"b".as_ptr(), bytes.as_ptr().cast(), 3) },
            kResultOk
        );
        let (mut at, mut size) = (ptr::null(), 0);
        assert_eq!(
            // SAFETY: as said at the top.
            unsafe { list.getBinary(c"b".as_ptr(), &mut at, &mut size) },
            kResultOk
        );
        assert_eq!(
            // SAFETY: `getBinary` pointed `at` at the list's `size` bytes, kept while the list lives.
            unsafe { slice::from_raw_parts(at.cast::<u8>(), size as usize) },
            bytes
        );
        assert_eq!(
            // SAFETY: as said at the top.
            unsafe { list.getInt(c"missing".as_ptr(), &mut n) },
            kResultFalse
        );
    }
}
