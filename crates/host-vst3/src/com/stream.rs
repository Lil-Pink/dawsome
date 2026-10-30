//! A stream over bytes in memory (`IBStream` and `ISizeableStream`), what a plugin writes its
//! state into and reads it back from. Used on the main thread only.

use std::ffi::c_void;
use std::slice;
use std::sync::Mutex;

use vst3::Steinberg::IBStream_::IStreamSeekMode_;
use vst3::Steinberg::{
    int32, int64, kInvalidArgument, kResultFalse, kResultOk, tresult, IBStream, IBStreamTrait,
    ISizeableStream, ISizeableStreamTrait,
};
use vst3::{Class, ComWrapper};

use super::{guarded, Counted};

/// The most a stream holds: 256 MiB. A write past it, or a size over it, is refused
/// (`kResultFalse`) and leaves the stream as it was. Why: a plugin's state is a preset's worth of
/// bytes (tens of MiB with samples inside), and a misbehaving plugin asking for 2^40 would make
/// the host allocate that; a failed allocation aborts the process, which [`guarded`]'s unwind
/// guard cannot catch.
pub(crate) const MOST_BYTES: usize = 256 << 20;

/// The bytes and the place the next read or write starts at.
#[derive(Debug, Default)]
struct Bytes {
    data: Vec<u8>,
    at: usize,
}

pub(crate) struct MemoryStream {
    bytes: Mutex<Bytes>,
    _counted: Counted,
}

impl Class for MemoryStream {
    type Interfaces = (IBStream, ISizeableStream);
}

impl MemoryStream {
    /// A stream to read `data` from, from its start (or to write over it).
    pub(crate) fn new(data: Vec<u8>) -> ComWrapper<Self> {
        ComWrapper::new(Self {
            bytes: Mutex::new(Bytes { data, at: 0 }),
            _counted: Counted::new(),
        })
    }

    /// What the stream holds now.
    pub(crate) fn data(&self) -> Vec<u8> {
        self.bytes
            .lock()
            .map(|bytes| bytes.data.clone())
            .unwrap_or_default()
    }

    /// Back to its start, for the next reader.
    pub(crate) fn rewind(&self) {
        if let Ok(mut bytes) = self.bytes.lock() {
            bytes.at = 0;
        }
    }
}

impl IBStreamTrait for MemoryStream {
    unsafe fn read(&self, buffer: *mut c_void, count: int32, read: *mut int32) -> tresult {
        guarded(|| {
            let Ok(mut bytes) = self.bytes.lock() else {
                return kInvalidArgument;
            };
            let Ok(wanted) = usize::try_from(count) else {
                return kInvalidArgument;
            };
            if buffer.is_null() && wanted > 0 {
                return kInvalidArgument;
            }
            let from = bytes.at.min(bytes.data.len());
            let taken = wanted.min(bytes.data.len() - from);
            if taken > 0 {
                // SAFETY: the plugin hands a buffer of `count` bytes (`IBStream::read`), and
                // `taken` is at most that.
                unsafe {
                    slice::from_raw_parts_mut(buffer.cast::<u8>(), taken)
                        .copy_from_slice(&bytes.data[from..from + taken]);
                }
            }
            bytes.at = from + taken;
            if !read.is_null() {
                // SAFETY: an out parameter the plugin owns, as the interface lays it out.
                unsafe { *read = taken as int32 };
            }
            kResultOk
        })
    }

    unsafe fn write(&self, buffer: *mut c_void, count: int32, written: *mut int32) -> tresult {
        guarded(|| {
            let Ok(mut bytes) = self.bytes.lock() else {
                return kInvalidArgument;
            };
            let Ok(count) = usize::try_from(count) else {
                return kInvalidArgument;
            };
            if buffer.is_null() && count > 0 {
                return kInvalidArgument;
            }
            let source: &[u8] = if count == 0 {
                &[]
            } else {
                // SAFETY: the plugin hands `count` bytes to write (`IBStream::write`).
                unsafe { slice::from_raw_parts(buffer.cast::<u8>(), count) }
            };
            let at = bytes.at;
            let Some(end) = at.checked_add(count).filter(|&end| end <= MOST_BYTES) else {
                return kResultFalse;
            };
            if bytes.data.len() < end {
                bytes.data.resize(end, 0);
            }
            bytes.data[at..end].copy_from_slice(source);
            bytes.at = end;
            if !written.is_null() {
                // SAFETY: an out parameter the plugin owns.
                unsafe { *written = count as int32 };
            }
            kResultOk
        })
    }

    unsafe fn seek(&self, pos: int64, mode: int32, result: *mut int64) -> tresult {
        guarded(|| {
            let Ok(mut bytes) = self.bytes.lock() else {
                return kInvalidArgument;
            };
            let base = match mode {
                mode if mode == IStreamSeekMode_::kIBSeekSet as int32 => 0,
                mode if mode == IStreamSeekMode_::kIBSeekCur as int32 => bytes.at as int64,
                mode if mode == IStreamSeekMode_::kIBSeekEnd as int32 => bytes.data.len() as int64,
                _ => return kInvalidArgument,
            };
            let to = base.saturating_add(pos).max(0);
            bytes.at = usize::try_from(to).unwrap_or(usize::MAX);
            if !result.is_null() {
                // SAFETY: an out parameter the plugin owns.
                unsafe { *result = to };
            }
            kResultOk
        })
    }

    unsafe fn tell(&self, pos: *mut int64) -> tresult {
        guarded(|| {
            let Ok(bytes) = self.bytes.lock() else {
                return kInvalidArgument;
            };
            if pos.is_null() {
                return kInvalidArgument;
            }
            // SAFETY: an out parameter the plugin owns.
            unsafe { *pos = bytes.at as int64 };
            kResultOk
        })
    }
}

impl ISizeableStreamTrait for MemoryStream {
    unsafe fn getStreamSize(&self, size: *mut int64) -> tresult {
        guarded(|| {
            let Ok(bytes) = self.bytes.lock() else {
                return kInvalidArgument;
            };
            if size.is_null() {
                return kInvalidArgument;
            }
            // SAFETY: an out parameter the plugin owns.
            unsafe { *size = bytes.data.len() as int64 };
            kResultOk
        })
    }

    unsafe fn setStreamSize(&self, size: int64) -> tresult {
        guarded(|| {
            let (Ok(mut bytes), Ok(size)) = (self.bytes.lock(), usize::try_from(size)) else {
                return kInvalidArgument;
            };
            if size > MOST_BYTES {
                return kResultFalse;
            }
            bytes.data.resize(size, 0);
            bytes.at = bytes.at.min(size);
            kResultOk
        })
    }
}

#[cfg(test)]
mod tests {
    use vst3::ComRef;

    use super::*;

    // A size over MOST_BYTES, and a write that would end past it (after a seek far out, or at
    // the end of the address space), are refused and change nothing; a size within it is kept.
    #[test]
    fn sizes_and_writes_past_the_most_bytes_are_refused() {
        let wrapper = MemoryStream::new(b"OTP1".to_vec());
        let stream: ComRef<'_, IBStream> = wrapper.as_com_ref().expect("a stream");
        let sizeable: ComRef<'_, ISizeableStream> = wrapper.as_com_ref().expect("sizeable");
        let (mut size, mut at, mut written) = (0i64, 0i64, 0i32);
        let mut data = *b"abcd";
        let most = MOST_BYTES as i64;
        let set = IStreamSeekMode_::kIBSeekSet as int32;
        // Every call below is into our own stream through its interface, with out parameters
        // and the 4-byte `data` of this test, which outlive the call.
        // SAFETY: as just said.
        assert_eq!(unsafe { sizeable.setStreamSize(1 << 40) }, kResultFalse);
        // SAFETY: as said at the top.
        assert_eq!(unsafe { sizeable.setStreamSize(most + 1) }, kResultFalse);
        // SAFETY: as said at the top.
        assert_eq!(unsafe { sizeable.getStreamSize(&mut size) }, kResultOk);
        assert_eq!(size, 4);
        // SAFETY: as said at the top.
        assert_eq!(unsafe { stream.seek(1 << 40, set, &mut at) }, kResultOk);
        assert_eq!(
            // SAFETY: as said at the top.
            unsafe { stream.write(data.as_mut_ptr().cast(), 4, &mut written) },
            kResultFalse
        );
        // SAFETY: as said at the top.
        assert_eq!(unsafe { stream.seek(most - 3, set, &mut at) }, kResultOk);
        assert_eq!(
            // SAFETY: as said at the top.
            unsafe { stream.write(data.as_mut_ptr().cast(), 4, &mut written) },
            kResultFalse
        );
        // SAFETY: as said at the top.
        assert_eq!(unsafe { stream.seek(i64::MAX, set, &mut at) }, kResultOk);
        assert_eq!(
            // SAFETY: as said at the top.
            unsafe { stream.write(data.as_mut_ptr().cast(), 4, &mut written) },
            kResultFalse
        );
        // SAFETY: as said at the top.
        assert_eq!(unsafe { sizeable.getStreamSize(&mut size) }, kResultOk);
        assert_eq!(size, 4);
        // SAFETY: as said at the top.
        assert_eq!(unsafe { sizeable.setStreamSize(16) }, kResultOk);
        // SAFETY: as said at the top.
        assert_eq!(unsafe { sizeable.getStreamSize(&mut size) }, kResultOk);
        assert_eq!(size, 16);
        assert_eq!(wrapper.data()[..4], *b"OTP1");
    }

    // Reads past the end give what is left and then nothing; writes go where the last seek put
    // them, growing the stream; the size and the place follow.
    #[test]
    fn reads_writes_and_seeks_follow_one_place() {
        let wrapper = MemoryStream::new(b"OTP1".to_vec());
        let stream: ComRef<'_, IBStream> = wrapper.as_com_ref().expect("a stream");
        let (mut buffer, mut read) = ([0u8; 8], 0);
        // Every call below is into our own stream through its interface, with the 8-byte
        // `buffer` and out parameters of this test, which outlive the call.
        assert_eq!(
            // SAFETY: as just said.
            unsafe { stream.read(buffer.as_mut_ptr().cast(), 8, &mut read) },
            kResultOk
        );
        assert_eq!((read, &buffer[..4]), (4, &b"OTP1"[..]));
        assert_eq!(
            // SAFETY: as said at the top.
            unsafe { stream.read(buffer.as_mut_ptr().cast(), 8, &mut read) },
            kResultOk
        );
        assert_eq!(read, 0, "at the end");
        let mut at = 0;
        assert_eq!(
            // SAFETY: as said at the top.
            unsafe { stream.seek(2, IStreamSeekMode_::kIBSeekSet as int32, &mut at) },
            kResultOk
        );
        let mut written = 0;
        let more = *b"xyz";
        assert_eq!(
            // SAFETY: as said at the top; the stream only reads the 3 bytes of `more`.
            unsafe { stream.write(more.as_ptr().cast_mut().cast(), 3, &mut written) },
            kResultOk
        );
        assert_eq!(written, 3);
        assert_eq!(wrapper.data(), b"OTxyz");
        // SAFETY: as said at the top.
        assert_eq!(unsafe { stream.tell(&mut at) }, kResultOk);
        assert_eq!(at, 5);
        assert_eq!(
            // SAFETY: as said at the top.
            unsafe { stream.seek(-1, IStreamSeekMode_::kIBSeekEnd as int32, &mut at) },
            kResultOk
        );
        assert_eq!(at, 4);
    }
}
