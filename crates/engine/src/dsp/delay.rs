//! The stereo delay: per side a feedback comb, `d[n] = x[n − D] + feedback·d[n − D]`, so echo k
//! of a click sits exactly k·D frames after it at feedback^(k−1). D comes from a time in
//! milliseconds, or, when synced, from a number of steps at the song's tempo through the one
//! tick-to-sample rule, so a synced echo lands on the same frame a note on that tick would.
//!
//! When D changes (Time or Sync moved, or the tempo under Sync), the echo fades from the old tap
//! to the new over [`FADE_MS`], `(1 − g)·old + g·new` with g rising by 1/F a frame, instead of
//! jumping to it: a jump between two points of the sound is a click, and a knob being turned
//! makes one per move. Why a straight fade and not an equal-power one: while a knob turns, the
//! two taps are a few ms apart and nearly the same sound, which an equal-power fade would raise
//! by up to 3 dB each move. A change that comes during a fade waits for it to end, then fades
//! from there. With D steady the echo is the one tap, bit for bit.

use super::{blend, snap};
use crate::processor::{ParamInfo, ProcessContext, Processor, ProcessorFactory, Taper};
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
        taper: Taper::Log,
    },
    ParamInfo {
        key: "sync_steps",
        label: "Sync",
        min: 0.0,
        max: 32.0,
        default: 0.0,
        unit: "steps",
        whole: true,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "feedback",
        label: "Feedback",
        min: 0.0,
        max: 0.95,
        default: 0.4,
        unit: "%",
        whole: false,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "mix",
        label: "Mix",
        min: 0.0,
        max: 1.0,
        default: 0.3,
        unit: "%",
        whole: false,
        taper: Taper::Linear,
    },
];

/// How long a change of D takes to fade from the old tap to the new.
const FADE_MS: u64 = 30;

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
        let mut delay = Delay::silent(frames, fade_frames(sample_rate));
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
    /// F: the frames a change of D fades over, at least one.
    fade: usize,
    /// The D the echo is read at; `None` until the first frame after a reset, which takes D as
    /// it is then, with nothing to fade from.
    tap: Option<usize>,
    /// The D being faded to, and how many frames of the fade have played.
    next: Option<(usize, usize)>,
}

/// F at `sample_rate`: [`FADE_MS`] rounded to a frame, at least one.
fn fade_frames(sample_rate: u32) -> usize {
    let frames = (FADE_MS * u64::from(sample_rate) + 500) / 1000;
    usize::try_from(frames.max(1)).unwrap_or(usize::MAX)
}

impl Delay {
    /// A silent line of `frames` frames whose changes of D fade over `fade` frames, at the
    /// default values.
    fn silent(frames: usize, fade: usize) -> Self {
        Self {
            line: vec![[0.0; 2]; frames].into_boxed_slice(),
            write: 0,
            written: 0,
            values: PARAMS.map(|info| info.default),
            fade: fade.max(1),
            tap: None,
            next: None,
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

    /// What the line plays `delay` frames before the frame about to be written.
    fn tap(&self, delay: usize) -> [f32; 2] {
        let len = self.line.len();
        self.heard((self.write + len - delay) % len)
    }

    /// The echo of the frame about to be written, `target` being D now: the tap, or during a
    /// fade the old and new taps weighted by how far it has come; a fade starts when `target`
    /// is not the tap and none is on, and the new tap takes over on its last frame.
    fn echo(&mut self, target: usize) -> [f32; 2] {
        let tap = *self.tap.get_or_insert(target);
        if self.next.is_none() && tap != target {
            self.next = Some((target, 0));
        }
        let old = self.tap(tap);
        let Some((next, done)) = self.next else {
            return old;
        };
        let new = self.tap(next);
        let done = done + 1;
        // At most F, far below 2^24, so both casts are exact; on the last frame g is 1 and the
        // echo the new tap's bits.
        let gain = done as f32 / self.fade as f32;
        if done >= self.fade {
            self.tap = Some(next);
            self.next = None;
        } else {
            self.next = Some((next, done));
        }
        let keep = 1.0 - gain;
        [keep * old[0] + gain * new[0], keep * old[1] + gain * new[1]]
    }
}

impl Processor for Delay {
    fn process(&mut self, context: &ProcessContext, buffer: &mut [[f32; 2]]) {
        let len = self.line.len();
        let delay = self.delay_frames(context);
        let (feedback, mix) = (self.values[FEEDBACK], self.values[MIX]);
        for frame in buffer {
            let echo = self.echo(delay);
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
        self.tap = None;
        self.next = None;
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
        let mut instance = Delay::silent(500, fade_frames(44_100));
        instance.process(&CONTEXT, &mut noise(100));
        instance.line[100..].fill([7.0; 2]);
        instance.reset();
        assert!((0..100).all(|at| instance.heard(at) == [0.0; 2]));
        assert!(instance.line[100..].iter().all(|frame| *frame == [7.0; 2]));

        let mut instance = Delay::silent(500, fade_frames(44_100));
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
            let mut instance = Delay::silent(500, fade_frames(44_100));
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
                let mut delay = Delay::silent(192_000, fade_frames(48_000));
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

    // Old sound never comes back when Time grows: the line holds 4 s whatever Time is, and every
    // frame of it is written each time the line comes round. A click through 10 ms (441 frames)
    // at feedback 0.5 writes 0.5^k on frame 441k, below the 1e-30 snap from k = 100 (0.5^100 ≈
    // 7.9e-31), so nothing but zeros is written from frame 44100 on, and a whole line (176400
    // frames) later every frame of it is zero. Time then raised to 4000 ms, the most the line
    // holds: silence, as with no feedback at all.
    #[test]
    fn time_raised_after_the_echoes_died_out_brings_nothing_old_back() {
        for feedback in [0.0, 0.5] {
            let mut instance = delay([10.0, 0.0, feedback, 1.0]);
            let mut first = impulse(44_100 + 176_400);
            instance.process(&CONTEXT, &mut first);
            instance.set_param(TIME_MS, 4000.0);
            let mut after = vec![[0.0; 2]; 200_000];
            instance.process(&CONTEXT, &mut after);
            assert_eq!(sounding(&after), [], "feedback {feedback}");
        }
    }

    // A Time change fades from the old tap to the new instead of jumping, which clicks. A 440 Hz
    // sine of amplitude 0.5 at 44.1 kHz, all wet, no feedback, Time moved between blocks by 44
    // to 2205 frames (taps far out of phase), once while a fade is still on. The output is
    // (1 − g)·a + g·b of two taps of the input, g rising by 1/F a frame (F = 30 ms = 1323
    // frames), so a step of it is at most the input's largest step s (each tap moves no faster,
    // the weights sum to 1) plus 2·0.5/F (the weights move against taps at most 1.0 apart).
    // Once the last fade is over, the output is the input exactly 150 ms (6615 frames) late.
    #[test]
    fn a_time_change_fades_between_the_taps_without_a_click() {
        let rate = 44_100.0;
        let input: Vec<[f32; 2]> = (0..40_000)
            .map(|n| {
                let value =
                    (0.5 * (std::f64::consts::TAU * 440.0 * f64::from(n) / rate).sin()) as f32;
                [value; 2]
            })
            .collect();
        let step = |frames: &[[f32; 2]]| {
            frames
                .windows(2)
                .map(|pair| (pair[1][0] - pair[0][0]).abs())
                .fold(0.0f32, f32::max)
        };
        let s = step(&input).max(input[0][0].abs());
        assert_eq!(fade_frames(44_100), 1323, "30 ms at 44.1 kHz");
        let fade = 1323.0;
        let bound = s + 2.0 * 0.5 / fade + 1e-6;
        let mut instance = delay([100.0, 0.0, 0.0, 1.0]);
        let mut output = input.clone();
        let times = [100.0, 101.0, 103.0, 106.0, 110.0, 100.0, 150.0];
        let mut at = 0;
        for (block, time) in times.into_iter().enumerate() {
            instance.set_param(TIME_MS, time);
            // The change to 100 ms comes 700 frames after the one before: inside its fade.
            let len = if block == 4 { 700 } else { 2048 };
            instance.process(&CONTEXT, &mut output[at..at + len]);
            at += len;
        }
        instance.process(&CONTEXT, &mut output[at..]);
        let worst = step(&output);
        assert!(worst <= bound, "a step of {worst}, more than {bound}");
        let settled = at + 2 * 1323;
        for n in settled..output.len() {
            assert_eq!(
                output[n].map(f32::to_bits),
                input[n - 6615].map(f32::to_bits),
                "frame {n}"
            );
        }
    }

    // A reset during a fade ends it: the next click echoes on the new D alone, at full level
    // (10 ms then 20 ms: 441 then 882 frames).
    #[test]
    fn a_reset_during_a_fade_leaves_nothing_of_it() {
        let mut instance = delay([10.0, 0.0, 0.0, 1.0]);
        instance.process(&CONTEXT, &mut noise(1000));
        instance.set_param(TIME_MS, 20.0);
        instance.process(&CONTEXT, &mut noise(100));
        instance.reset();
        let mut after = impulse(2000);
        instance.process(&CONTEXT, &mut after);
        assert_eq!(sounding(&after), [(882, 1.0)]);
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
