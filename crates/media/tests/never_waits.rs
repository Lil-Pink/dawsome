//! A streamed clip's live reads never wait: not for the disk, not for a lock, not for a sleep.
//! Why the context-switch count: every way a thread can wait (a sleep, a contended lock, a
//! blocking read) gives the CPU up and counts one voluntary switch; a read that only copies
//! counts none.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dawsome_engine::source::{ClipSource, SourceRead};
use dawsome_media::{StreamingSource, Waiting};

fn saw() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/p8_saw.wav")
}

/// The calling thread's voluntary context switches so far, from the kernel (Linux).
#[cfg(target_os = "linux")]
fn voluntary_switches() -> u64 {
    let status = std::fs::read_to_string("/proc/thread-self/status").expect("thread status");
    status
        .lines()
        .find_map(|line| line.strip_prefix("voluntary_ctxt_switches:"))
        .and_then(|count| count.trim().parse().ok())
        .expect("voluntary_ctxt_switches")
}

// Frame 20000 lies past the cache at the clip's start (0..16384): right after the stream opens,
// a live read there is not ready, and says so at once rather than waiting for the disk.
#[test]
fn a_cold_live_read_is_not_ready_at_once() {
    let mut source = StreamingSource::open(&saw(), 0, None, Waiting::Never).expect("opens");
    let mut out = [[0.0f32; 2]; 512];
    let started = Instant::now();
    let read = source.read(20_000, &mut out);
    let took = started.elapsed();
    assert_eq!(read, SourceRead::NotReady);
    assert!(took < Duration::from_millis(5), "a cold read took {took:?}");
}

/// Two thousand live reads of `source`, in order and with jumps.
#[cfg(target_os = "linux")]
fn read_through(source: &mut StreamingSource, out: &mut [[f32; 2]]) {
    for read in 0..2000u64 {
        let frame = if read % 50 == 0 {
            (read * 7919) % 30_000
        } else {
            (read * 512) % 30_000
        };
        source.read(frame, out);
    }
}

// Two thousand live reads, in order and with jumps, while the reader thread fills behind them:
// the reading thread never gives up the CPU. Counted on a second pass: the first faults in the
// code, the output buffer and the stream's first windows. Why: under memory pressure a page
// reloaded from disk blocks the thread and counts as a voluntary switch too, which is the
// machine's wait, not the read path's. Linux only: the count is read from /proc, which other
// systems do not have.
#[cfg(target_os = "linux")]
#[test]
fn live_reads_never_wait() {
    let mut source = StreamingSource::open(&saw(), 0, None, Waiting::Never).expect("opens");
    let mut out = [[0.0f32; 2]; 512];
    read_through(&mut source, &mut out);
    let before = voluntary_switches();
    read_through(&mut source, &mut out);
    let waits = voluntary_switches() - before;
    assert_eq!(waits, 0, "live reads waited {waits} times");
}
