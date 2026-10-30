//! How long a sound's edits take on the spec's 10x load: a ten-minute stereo sound at 48 kHz
//! (28,800,000 frames, 230 MB as f32) through 200 ops, applied from scratch (what an undo, a
//! redo or a project's opening costs) and one op at a time on the sound held (what appending an
//! op costs), then the long result written into an edited cache folder and found there again,
//! with the file's size. Numbers for the performance figures, not a test.
//!
//! ```text
//! cargo run --release -p dawsome-media --example edit_time -- <scratch folder>
//! ```
//!
//! The scratch folder gets the source WAV and the cache (about 460 MB); it is left there.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use dawsome_engine::dsp::edits::{apply_in_place, apply_ops};
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::project::{SoundEdits, SoundOp};
use dawsome_media::edited::{EditJob, Edited};

const RATE: u32 = 48_000;
const FRAMES: u64 = 10 * 60 * RATE as u64;

/// 200 ops over most of the sound: every kind, the length kept close to ten minutes.
fn ops() -> Vec<SoundOp> {
    let mut ops = Vec::new();
    let mut frames = FRAMES;
    for index in 0..200u64 {
        let op = match index % 8 {
            0 => SoundOp::Gain {
                from: 0,
                to: frames,
                centi_db: -100,
            },
            1 => SoundOp::FadeIn {
                from: 0,
                to: 48_000,
            },
            2 => SoundOp::Normalize {
                from: 0,
                to: frames,
                centi_db: -30,
            },
            3 => SoundOp::Reverse {
                from: 0,
                to: frames,
            },
            4 => SoundOp::Silence {
                from: index * 1000,
                to: index * 1000 + 480,
            },
            5 => SoundOp::FadeOut {
                from: frames - 48_000,
                to: frames,
            },
            6 => SoundOp::Crop {
                from: 10,
                to: frames - 10,
            },
            _ => SoundOp::Delete {
                from: frames / 2,
                to: frames / 2 + 100,
            },
        };
        frames = op.frames_after(frames);
        ops.push(op);
    }
    ops
}

fn main() -> ExitCode {
    let Some(dir) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: edit_time <scratch folder>");
        return ExitCode::FAILURE;
    };
    let samples: Vec<f32> = (0..FRAMES)
        .flat_map(|k| {
            let left = ((k % 4096) as f32 - 2048.0) / 4096.0;
            [left, -left / 2.0]
        })
        .collect();
    let sound = SampleBuffer::stereo(samples);
    let ops = ops();

    let started = Instant::now();
    let whole = apply_ops(&sound, &ops);
    let scratch = started.elapsed();
    println!(
        "from scratch: {} ops on {} frames in {:.2} s ({:.1} ms an op)",
        ops.len(),
        FRAMES,
        scratch.as_secs_f64(),
        scratch.as_secs_f64() * 1000.0 / ops.len() as f64
    );

    // One op at a time on a new buffer each (what media's path makes of an op past a sound it
    // holds), and in place on frames held for editing (what the editor's view can do).
    let mut held = sound.clone();
    let mut slowest = (0.0f64, "");
    let started = Instant::now();
    for op in &ops {
        let one = Instant::now();
        held = apply_ops(&held, std::slice::from_ref(op));
        let took = one.elapsed().as_secs_f64();
        if took > slowest.0 {
            slowest = (took, op.name());
        }
    }
    let appended = started.elapsed();
    assert_eq!(
        held.samples(),
        whole.samples(),
        "op by op gives the bits of the whole"
    );
    println!(
        "one op at a time, a new buffer each: {:.1} ms an op on average, the slowest {:.1} ms ({})",
        appended.as_secs_f64() * 1000.0 / ops.len() as f64,
        slowest.0 * 1000.0,
        slowest.1
    );
    let mut frames = sound.samples().to_vec();
    let mut slowest = (0.0f64, "");
    let started = Instant::now();
    for op in &ops {
        let one = Instant::now();
        apply_in_place(&mut frames, 2, op);
        let took = one.elapsed().as_secs_f64();
        if took > slowest.0 {
            slowest = (took, op.name());
        }
    }
    let in_place = started.elapsed();
    assert_eq!(
        frames.as_slice(),
        whole.samples(),
        "in place gives the same bits"
    );
    println!(
        "one op at a time, in place: {:.1} ms an op on average, the slowest {:.1} ms ({})",
        in_place.as_secs_f64() * 1000.0 / ops.len() as f64,
        slowest.0 * 1000.0,
        slowest.1
    );

    let file = dir.join("ten-minutes.wav");
    if let Err(error) = std::fs::create_dir_all(&dir).and_then(|()| {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(&file, spec).map_err(std::io::Error::other)?;
        for &value in sound.samples() {
            writer.write_sample(value).map_err(std::io::Error::other)?;
        }
        writer.finalize().map_err(std::io::Error::other)
    }) {
        eprintln!("edit_time: cannot write {}: {error}", file.display());
        return ExitCode::FAILURE;
    }
    let cache = dir.join("edited");
    let _ = std::fs::remove_dir_all(&cache);
    let job = EditJob {
        file: file.clone(),
        rate: RATE,
        edits: SoundEdits { ops, source: None },
        cache: Some(cache.clone()),
        stream_threshold: 30 * u64::from(RATE),
        streams: true,
        base: None,
    };
    let started = Instant::now();
    let made = job.run();
    let first = started.elapsed();
    let Edited::Cached(path) = &made.result else {
        eprintln!("edit_time: the result was not cached");
        return ExitCode::FAILURE;
    };
    let bytes = std::fs::metadata(path).map_or(0, |metadata| metadata.len());
    let started = Instant::now();
    let again = job.run();
    let found = started.elapsed();
    println!(
        "the job (decode, 200 ops, write): {:.2} s, {bytes} bytes cached; found again in {:.1} ms (wrote: {})",
        first.as_secs_f64(),
        found.as_secs_f64() * 1000.0,
        again.wrote
    );
    ExitCode::SUCCESS
}
