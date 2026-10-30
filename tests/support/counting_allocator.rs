//! Counts heap calls (alloc, realloc, free) that the current thread makes while it measures,
//! and the bytes they leave held. Included as a module by the tests that prove the audio path
//! never touches the heap (the engine's player test, the app's device-rate bridge test) and by
//! the app's measure of what the sample editor holds; it installs itself as the test binary's
//! global allocator. The counts are per thread, so tests running in parallel cannot disturb
//! them.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

/// Forwards every call to the system allocator, counting it while the thread measures.
struct CountingAllocator;

thread_local! {
    static MEASURING: Cell<bool> = const { Cell::new(false) };
    static HEAP_CALLS: Cell<usize> = const { Cell::new(0) };
    /// The bytes this thread's calls took (less those they gave back) while it measures, and
    /// the most they reached.
    static HELD: Cell<i64> = const { Cell::new(0) };
    static MOST: Cell<i64> = const { Cell::new(0) };
}

fn record_heap_call(taken: i64) {
    // `try_with`, because the allocator can still run while thread-local storage is torn down.
    if MEASURING.try_with(Cell::get).unwrap_or(false) {
        let _ = HEAP_CALLS.try_with(|calls| calls.set(calls.get() + 1));
        let _ = HELD.try_with(|held| {
            held.set(held.get() + taken);
            let _ = MOST.try_with(|most| most.set(most.get().max(held.get())));
        });
    }
}

/// A size as the tally counts it.
fn bytes(size: usize) -> i64 {
    i64::try_from(size).unwrap_or(i64::MAX)
}

// SAFETY: every call is forwarded unchanged to the system allocator; counting touches only
// const-initialized thread-locals, which never allocate.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_heap_call(bytes(layout.size()));
        // SAFETY: same contract as our caller's.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record_heap_call(-bytes(layout.size()));
        // SAFETY: `ptr` came from `System` through this allocator, with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_heap_call(bytes(new_size) - bytes(layout.size()));
        // SAFETY: as for `dealloc`; `new_size` is passed through untouched.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Runs `work` and returns how many heap calls this thread made during it.
pub fn heap_calls_during(work: impl FnOnce()) -> usize {
    let before = HEAP_CALLS.with(Cell::get);
    MEASURING.with(|measuring| measuring.set(true));
    work();
    MEASURING.with(|measuring| measuring.set(false));
    HEAP_CALLS.with(Cell::get) - before
}

/// What `work` leaves held on the heap, in bytes, by this thread's calls during it (less what
/// they freed, those of memory taken before it too), and the most it held on the way.
#[allow(
    dead_code,
    reason = "the app's measure alone reads it; the audio tests count calls"
)]
pub fn heap_bytes_during(work: impl FnOnce()) -> (i64, i64) {
    HELD.with(|held| held.set(0));
    MOST.with(|most| most.set(0));
    MEASURING.with(|measuring| measuring.set(true));
    work();
    MEASURING.with(|measuring| measuring.set(false));
    (HELD.with(Cell::get), MOST.with(Cell::get))
}
