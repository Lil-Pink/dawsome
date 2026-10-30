//! The media halves that run on the audio thread never touch the heap: a streamed clip's
//! reads (live playback, which never waits for the disk) and the input callback's copy into
//! its ring, overflow included.

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use std::path::Path;
use std::time::{Duration, Instant};

use counting_allocator::heap_calls_during;
use dawsome_engine::source::{ClipSource, SourceRead};
use dawsome_media::record::{input_ring, Feed};
use dawsome_media::{StreamingSource, Waiting};

#[test]
fn a_streamed_clip_reads_without_the_heap() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/p8_saw.wav");
    let mut source = StreamingSource::open(&path, 0, None, Waiting::Never).expect("opens");
    let mut out = [[0.0f32; 2]; 512];
    // The reader thread fills the cache first; what is measured is the audio thread's side.
    let deadline = Instant::now() + Duration::from_secs(10);
    while source.read(0, &mut out) != SourceRead::Ready {
        assert!(Instant::now() < deadline, "the stream fills");
        std::thread::sleep(Duration::from_millis(1));
    }
    let calls = heap_calls_during(|| {
        for block in 1..20u64 {
            source.read(block * 512, &mut out);
        }
        // A jump back to the cached start, as a loop wrap does.
        source.read(0, &mut out);
        source.prepare(4096);
    });
    assert_eq!(calls, 0, "heap calls on the audio thread");
}

// One second of stereo at 48 kHz holds 48000 frames: pushing 200 buffers of 512 frames keeps
// the first 48000 and counts the other 54400 as dropped, without the heap.
#[test]
fn the_input_callback_copies_into_its_ring_without_the_heap() {
    let (mut writer, mut feed, peak) = input_ring(2, 48_000, 1);
    let buffer = vec![0.25f32; 1024];
    let calls = heap_calls_during(|| {
        for _ in 0..200 {
            writer.push(&buffer, 2, Some(0));
        }
    });
    assert_eq!(calls, 0, "heap calls on the input callback");
    let mut taken = Vec::new();
    assert_eq!(feed.read_into(&mut taken), 96_000);
    assert_eq!(feed.dropped(), 54_400);
    assert_eq!(
        f32::from_bits(peak.load(std::sync::atomic::Ordering::Relaxed)),
        0.25
    );
}

// The player's side of an input channel's monitor ring reads without the heap, live (waiting
// for its cushion, running dry, dropping what runs ahead, then watching level stretches, its
// cushion shrinking and draining) and offline. The live rate is low, so its 10 s stretches end
// within the measurement.
#[test]
fn the_monitor_feed_reads_without_the_heap() {
    use std::sync::atomic::Ordering;

    use dawsome_engine::input::InputFeed;
    use dawsome_engine::project::InputChannels;
    use dawsome_media::record::{monitor_ring, Pacing};

    let live = Some(Pacing {
        output_block: 512,
        rate: 1_000,
    });
    for pacing in [live, None] {
        let (mut writer, mut feed, _) = monitor_ring(InputChannels::Stereo(1, 0), 8192, pacing);
        let stats = feed.stats();
        let buffer = vec![0.25f32; 3 * 256];
        let mut out = [[0.0f32; 2]; 512];
        let mut grown = 0;
        let calls = heap_calls_during(|| {
            for round in 0..200 {
                // Bursts and gaps: the input runs ahead, then dry.
                let pushes = if round % 10 < 5 { 4 } else { 0 };
                for _ in 0..pushes {
                    writer.push(&buffer, 3, Some(0));
                }
                feed.read(&mut out);
            }
            grown = stats.cushion.load(Ordering::Relaxed);
            // Then the clocks match: two 256-frame buffers for each 512-frame read.
            for _ in 0..2000 {
                writer.push(&buffer, 3, Some(0));
                writer.push(&buffer, 3, Some(0));
                feed.read(&mut out);
            }
        });
        assert_eq!(calls, 0, "heap calls reading the monitor feed ({pacing:?})");
        if pacing.is_some() {
            let now = stats.cushion.load(Ordering::Relaxed);
            assert!(
                now < grown,
                "the cushion shrank while measured: {grown} -> {now}"
            );
        }
    }
}
