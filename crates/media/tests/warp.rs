//! Warp markers and transients in media: (7) the onsets gen_p39_expected.py finds in each of its
//! sounds, and (8) the time-stretch mode through markers, by its properties (its output is not
//! bit-predictable): exactly T frames, a click at a marker's source near its out, no step at a
//! segment's edge, and no markers the same bits as the uniform stretch.

use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::project::{ClipStretch, StretchMode};
use dawsome_engine::warp::WarpMarker;
use dawsome_media::{load_channel_sample, onsets, stretch};
use serde_json::Value;
use signalsmith_stretch::Stretch;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p39_expected.json")).expect("p39_expected");
    serde_json::from_str(&text).expect("parses")
}

fn stretch_of(frames: u64) -> ClipStretch {
    ClipStretch {
        mode: StretchMode::Stretch,
        frames,
        pitch: 0,
    }
}

fn marker(source: u64, out: u64) -> WarpMarker {
    WarpMarker { source, out }
}

#[test]
fn p39_onsets_are_the_generators() {
    for (name, case) in expected()["onsets"].as_object().expect("onsets") {
        let file = case["file"].as_str().expect("file");
        let loaded = load_channel_sample(&fixtures(), Path::new(file), 48_000);
        assert!(loaded.file_rate.is_some(), "{name}: {file} reads");
        let want: Vec<u64> = case["frames"]
            .as_array()
            .expect("frames")
            .iter()
            .map(|frame| frame.as_u64().expect("a frame"))
            .collect();
        assert_eq!(onsets::of(&loaded.buffer), want, "{name}");
    }
}

/// One second at 48 kHz of silence with a click (1.0) at each of `at`.
fn clicks(at: &[u64]) -> SampleBuffer {
    let mut values = vec![0.0; 48_000];
    for &frame in at {
        values[frame as usize] = 1.0;
    }
    SampleBuffer::mono(values)
}

/// The frame of the loudest value of `buffer` (left) within `radius` frames of `around`.
fn loudest_near(buffer: &SampleBuffer, around: u64, radius: u64) -> u64 {
    let (from, to) = (
        around.saturating_sub(radius),
        (around + radius).min(buffer.frames()),
    );
    (from..to)
        .max_by(|&a, &b| {
            let level = |frame| buffer.frame(frame).map_or(0.0, |pair| pair[0].abs());
            level(a).total_cmp(&level(b))
        })
        .expect("a frame")
}

// The library's analysis interval at 48 kHz: 30 ms, 1440 frames. A marker moves the moment the
// stretch reads, which the library places to within an interval; that is the tolerance held.
const TOLERANCE: u64 = 1440;

#[test]
fn p39_a_time_stretch_through_markers_is_exactly_its_length() {
    let tone = SampleBuffer::mono(
        (0..48_000)
            .map(|k| 0.5 * (k as f32 * 0.057_595_87).sin())
            .collect(),
    );
    let pair = SampleBuffer::stereo(
        (0..48_000)
            .flat_map(|k| [k as f32 / 48_000.0, 0.25])
            .collect(),
    );
    for (sound, frames, markers) in [
        (&tone, 60_000, vec![marker(12_000, 30_000)]),
        (
            &tone,
            40_000,
            vec![marker(10_000, 8_000), marker(30_000, 20_000)],
        ),
        (
            &pair,
            50_001,
            vec![marker(1_000, 2_000), marker(47_000, 49_000)],
        ),
        (&tone, 48_000, vec![]),
    ] {
        let out = stretch::render(sound, 48_000, stretch_of(frames), &markers).expect("stretches");
        assert_eq!(out.frames(), frames, "{markers:?}");
        assert_eq!(out.channels(), sound.channels());
    }
}

#[test]
fn p39_a_click_at_a_markers_source_lands_near_its_out() {
    let at = [9_000, 21_000, 33_000];
    let markers = vec![
        marker(9_000, 15_000),
        marker(21_000, 24_000),
        marker(33_000, 33_000),
    ];
    let out = stretch::render(&clicks(&at), 48_000, stretch_of(48_000), &markers).expect("fits");
    for marker in &markers {
        let landed = loudest_near(&out, marker.out, 4 * TOLERANCE);
        assert!(
            landed.abs_diff(marker.out) <= TOLERANCE,
            "the click at {} landed at {landed}, its out {}",
            marker.source,
            marker.out
        );
    }
}

// A steady tone through three markers of different ratios: no value steps further from the one
// before it than the tone's own steepest step, with room for the library's smoothing, anywhere
// and in particular at a segment's edge.
#[test]
fn p39_a_time_stretch_through_markers_has_no_step_at_a_segments_edge() {
    let rate = 48_000.0_f32;
    let step = 2.0 * std::f32::consts::PI * 220.0 / rate;
    let tone = SampleBuffer::mono((0..48_000).map(|k| 0.5 * (k as f32 * step).sin()).collect());
    let markers = vec![
        marker(12_000, 18_000),
        marker(24_000, 26_000),
        marker(36_000, 44_000),
    ];
    let out = stretch::render(&tone, 48_000, stretch_of(52_000), &markers).expect("fits");
    let steepest = 0.5 * step;
    let samples = out.samples();
    let worst = samples
        .windows(2)
        .skip(4_000)
        .take(44_000)
        .map(|pair| (pair[1] - pair[0]).abs())
        .fold(0.0_f32, f32::max);
    assert!(
        worst <= 1.25 * steepest,
        "a step of {worst}, the tone's {steepest}"
    );
}

// No markers: the very bits of the uniform stretch, the library's whole-file call.
#[test]
fn p39_no_markers_is_the_uniform_stretch_bit_for_bit() {
    let tone = SampleBuffer::mono(
        (0..30_000)
            .map(|k| ((k * 7919) % 4096) as f32 / 8192.0)
            .collect(),
    );
    for frames in [24_000, 45_000] {
        let out = stretch::render(&tone, 48_000, stretch_of(frames), &[]).expect("fits");
        let mut library = Stretch::preset_default(1, 48_000);
        library.set_transpose_factor_semitones(0.0, None);
        let mut uniform = vec![0.0; frames as usize];
        assert!(library.exact(tone.samples(), &mut uniform));
        assert_eq!(out.samples(), uniform.as_slice(), "{frames}");
    }
}
