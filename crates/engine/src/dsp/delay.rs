//! The stereo delay: per side a feedback comb, `d[n] = x[n − D] + feedback·d[n − D]`, so echo k
//! of a click sits exactly k·D frames after it at feedback^(k−1). D comes from a time in
//! milliseconds, or, when synced, from a number of steps at the song's tempo through the one
//! tick-to-sample rule, so a synced echo lands on the same frame a note on that tick would.

use super::{blend, snap};
use crate::processor::{ParamInfo, ProcessContext, Processor, ProcessorFactory};
use crate::time;

pub const ID: &str = "dawsome.delay";

/// The longest delay the line holds.
const MAX_TIME_MS: u32 = 4000;

const PARAMS: [ParamInfo; 4] = [
    ParamInfo {
        key: "time_ms",
        label: "Time",
        min: 1.0,
        max: MAX_TIME_MS as f32,
        default: 250.0,
        unit: "ms",
        whole: false,
    },
    ParamInfo {
        key: "sync_steps",
        label: "Sync",
        min: 0.0,
        max: 32.0,
        default: 0.0,
        unit: "steps",
        whole: true,
    },
    ParamInfo {
        key: "feedback",
        label: "Feedback",
        min: 0.0,
        max: 0.95,
        default: 0.4,
        unit: "%",
        whole: false,
    },
    ParamInfo {
        key: "mix",
        label: "Mix",
        min: 0.0,
        max: 1.0,
        default: 0.3,
        unit: "%",
        whole: false,
    },
];

const TIME_MS: usize = 0;
const SYNC_STEPS: usize = 1;
const FEEDBACK: usize = 2;
const MIX: usize = 3;

pub(super) struct Factory;

impl ProcessorFactory for Factory {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Delay"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn create(&self, sample_rate: u32, values: &[f32]) -> Box<dyn Processor> {
        // At least one frame, so a delay of one frame always fits.
        let frames = (u64::from(MAX_TIME_MS) * u64::from(sample_rate) / 1000).max(1);
        let frames = usize::try_from(frames).unwrap_or(usize::MAX);
        let mut delay = Delay::silent(frames);
        for (index, &value) in values.iter().enumerate() {
            delay.set_param(index, value);
        }
        Box::new(delay)
    }
}

struct Delay {
    /// What went into the line, `x + feedback·echo`, for the last `line.len()` frames.
    line: Box<[[f32; 2]]>,
    /// Where the next frame goes.
    write: usize,
    /// How many frames from the line's start were written since the last reset, at most all of
    /// them: writing starts at frame 0 again after a reset, and a frame at or past this count
    /// holds what played before it, which reads as silence.
    written: usize,
    values: [f32; 4],
}

impl Delay {
    /// A silent line of `frames` frames, at the default values.
    fn silent(frames: usize) -> Self {
        Self {
            line: vec![[0.0; 2]; frames].into_boxed_slice(),
            write: 0,
            written: 0,
            values: PARAMS.map(|info| info.default),
        }
    }

    /// D, in frames, clamped to what the line holds.
    fn delay_frames(&self, context: &ProcessContext) -> usize {
        let steps = self.values[SYNC_STEPS];
        let frames = if steps > 0.0 {
            // Whole and at most 32 after clamping, so the cast is exact.
            let tick = time::step_to_ticks(steps as u32, context.ppq);
            time::tick_to_sample(tick, context.sample_rate, context.tempo, context.ppq)
                .and_then(|frames| usize::try_from(frames).ok())
                .unwrap_or(usize::MAX)
        } else {
            let ms = f64::from(self.values[TIME_MS]);
            // Positive and at most 4000 ms of the rate, so the cast is exact.
            (ms * f64::from(context.sample_rate) / 1000.0).round() as usize
        };
        frames.clamp(1, self.line.len())
    }

    /// What frame `at` of the line plays back: what was written there since the last reset,
    /// silence before.
    fn heard(&self, at: usize) -> [f32; 2] {
        if at < self.written {
            self.line[at]
        } else {
            [0.0; 2]
        }
    }
}

impl Processor for Delay {
    fn process(&mut self, context: &ProcessContext, buffer: &mut [[f32; 2]]) {
        let len = self.line.len();
        let delay = self.delay_frames(context);
        let (feedback, mix) = (self.values[FEEDBACK], self.values[MIX]);
        for frame in buffer {
            let echo = self.heard((self.write + len - delay) % len);
            self.line[self.write] = [
                snap(frame[0] + feedback * echo[0]),
                snap(frame[1] + feedback * echo[1]),
            ];
            self.write = (self.write + 1) % len;
            self.written = (self.written + 1).min(len);
            *frame = blend(*frame, echo, mix);
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(info) = PARAMS.get(index) {
            self.values[index] = info.clamp(value);
        }
    }

    fn reset(&mut self) {
        // Why the line is not cleared: every processor resets inside one audio callback, and
        // clearing 160 lines of 4 s (1.5 MB each at 48 kHz) took 8 ms of a 10.7 ms buffer.
        // Reading past `written` gives silence instead.
        self.write = 0;
        self.written = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::tests::{bits, impulse, noise, whole_and_split, CONTEXT};

    fn delay(values: [f32; 4]) -> Box<dyn Processor> {
        Factory.create(44_100, &values)
    }

    /// The frames of `buffer` that are not silent, with their left value (both sides equal).
    fn sounding(buffer: &[[f32; 2]]) -> Vec<(usize, f32)> {
        buffer
            .iter()
            .enumerate()
            .filter(|(_, frame)| **frame != [0.0; 2])
            .map(|(at, frame)| {
                assert_eq!(frame[0], frame[1], "frame {at}");
                (at, frame[0])
            })
            .collect()
    }

    // 10 ms at 44.1 kHz is round(441.0) = 441 frames; feedback 0.5, mix 0.5: the dry click at
    // 0.5, then 0.5^k at 441k.
    #[test]
    fn echoes_sit_a_whole_delay_apart_and_fall_by_the_feedback() {
        let mut buffer = impulse(2000);
        delay([10.0, 0.0, 0.5, 0.5]).process(&CONTEXT, &mut buffer);
        assert_eq!(
            sounding(&buffer),
            [
                (0, 0.5),
                (441, 0.5),
                (882, 0.25),
                (1323, 0.125),
                (1764, 0.0625)
            ]
        );
    }

    // Python: 72 ticks at 120 BPM, 44.1 kHz, ppq 96 is 72·60·44100·1000 // (120000·96) = 16537.
    #[test]
    fn a_synced_delay_takes_its_time_from_the_tick_rule() {
        let mut buffer = impulse(40_000);
        delay([250.0, 3.0, 0.5, 1.0]).process(&CONTEXT, &mut buffer);
        assert_eq!(sounding(&buffer), [(16537, 1.0), (33074, 0.5)]);
    }

    #[test]
    fn values_are_clamped_and_steps_are_whole() {
        let mut instance = delay([10.0, 0.0, 0.5, 1.0]);
        instance.set_param(FEEDBACK, 5.0);
        instance.set_param(SYNC_STEPS, 2.6);
        instance.set_param(99, 1.0);
        let mut buffer = impulse(40_000);
        instance.process(&CONTEXT, &mut buffer);
        // Three steps (16537 frames) at the top feedback, 0.95.
        assert_eq!(sounding(&buffer), [(16537, 1.0), (33074, 0.95)]);
    }

    #[test]
    fn mix_0_passes_the_input_and_any_split_gives_the_same_bits() {
        let input = noise(3000);
        let mut output = input.clone();
        delay([5.0, 0.0, 0.9, 0.0]).process(&CONTEXT, &mut output);
        assert_eq!(bits(&output), bits(&input));
        let mut make = || delay([3.0, 0.0, 0.9, 0.6]);
        let (whole, split) = whole_and_split(&mut make, &input, &[1, 131, 7, 1000]);
        assert_eq!(bits(&whole), bits(&split));
    }

    // Play, Stop and Seek reset every processor in one audio callback, so a reset silences
    // only what was written since the last: after 100 frames those 100, the rest of the line
    // (marked here) untouched; after more frames than the line holds, all of it.
    #[test]
    fn a_reset_clears_only_the_frames_written_since_the_last() {
        let mut instance = Delay::silent(500);
        instance.process(&CONTEXT, &mut noise(100));
        instance.line[100..].fill([7.0; 2]);
        instance.reset();
        assert!((0..100).all(|at| instance.heard(at) == [0.0; 2]));
        assert!(instance.line[100..].iter().all(|frame| *frame == [7.0; 2]));

        let mut instance = Delay::silent(500);
        instance.process(&CONTEXT, &mut noise(700));
        assert!((0..500).all(|at| instance.heard(at) != [0.0; 2]));
        instance.reset();
        assert!((0..500).all(|at| instance.heard(at) == [0.0; 2]));
    }

    // A reset costs the same however much of the line was written: it touches no frame of it,
    // and what the line still holds reads as silence until it is written again. Here a 44-frame
    // delay, and one of 4000 ms (clamped to the line's 500 frames, so each frame reads the one
    // about to be overwritten), at full feedback and mix, reset after the line was filled with
    // noise, echo nothing from it.
    #[test]
    fn a_reset_touches_none_of_the_line_and_its_old_frames_read_as_silence() {
        for time_ms in [1.0, 4000.0] {
            let mut instance = Delay::silent(500);
            instance.process(&CONTEXT, &mut noise(700));
            let held = instance.line.clone();
            instance.reset();
            assert!(bits(&instance.line) == bits(&held), "the line is as it was");
            instance.set_param(TIME_MS, time_ms);
            instance.set_param(FEEDBACK, 0.95);
            instance.set_param(MIX, 1.0);
            let mut after = vec![[0.0; 2]; 1500];
            instance.process(&CONTEXT, &mut after);
            assert_eq!(sounding(&after), [], "{time_ms} ms");
        }
    }

    // What Stop costs with 160 delays whose 4 s lines (48 kHz) are written through; run with
    // `cargo test -p dawsome-engine [--release] -- --ignored --nocapture delay_reset_cost`.
    #[test]
    #[ignore = "a timing, not a check: prints what resetting 160 full delays takes"]
    fn delay_reset_cost() {
        let mut delays: Vec<Delay> = (0..160)
            .map(|_| {
                let mut delay = Delay::silent(192_000);
                delay.line.fill([0.25; 2]);
                delay.written = delay.line.len();
                delay
            })
            .collect();
        let start = std::time::Instant::now();
        for delay in &mut delays {
            delay.reset();
        }
        let elapsed = start.elapsed();
        println!("160 full 4 s delays at 48 kHz reset in {elapsed:?}");
    }

    #[test]
    fn reset_clears_the_line() {
        let mut instance = delay([10.0, 0.0, 0.9, 1.0]);
        let mut first = impulse(100);
        instance.process(&CONTEXT, &mut first);
        instance.reset();
        let mut after = vec![[0.0; 2]; 2000];
        instance.process(&CONTEXT, &mut after);
        assert_eq!(sounding(&after), []);
    }
}
