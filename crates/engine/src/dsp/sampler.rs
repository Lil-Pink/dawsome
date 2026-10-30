//! The sampler: plays a mono or stereo sample from its first frame at a fixed speed (no
//! envelope yet). A speed other than 1 reads between frames by 4-point cubic Hermite
//! interpolation ([`interpolate`]), the one rule every pitched read shares.

use std::sync::Arc;

use super::pitch::Pitch;
use crate::project::DEFAULT_KEY;

/// Decoded audio that a sampler channel or an audio clip plays, mono or stereo: interleaved
/// `f32` values, one per channel per frame. Cloning shares the frames, so channels and clips
/// that play the same file can hold one decoded copy.
#[derive(Debug, Clone)]
pub struct SampleBuffer {
    samples: Arc<[f32]>,
    /// 1 or 2.
    channels: u8,
}

impl SampleBuffer {
    /// Mono audio, one value per frame.
    pub fn mono(frames: Vec<f32>) -> Self {
        Self {
            samples: frames.into(),
            channels: 1,
        }
    }

    /// Stereo audio, left then right for each frame. An unpaired last value is dropped.
    pub fn stereo(mut interleaved: Vec<f32>) -> Self {
        interleaved.truncate(interleaved.len() / 2 * 2);
        Self {
            samples: interleaved.into(),
            channels: 2,
        }
    }

    /// 1 for mono, 2 for stereo.
    pub fn channels(&self) -> u8 {
        self.channels
    }

    /// How many frames it holds.
    pub fn frames(&self) -> u64 {
        (self.samples.len() / usize::from(self.channels)) as u64
    }

    /// The interleaved values.
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// Frame `index` as (left, right), a mono frame on both sides; `None` past the end.
    pub fn frame(&self, index: u64) -> Option<[f32; 2]> {
        let index = usize::try_from(index).ok()?;
        if self.channels == 1 {
            let value = *self.samples.get(index)?;
            Some([value, value])
        } else {
            let at = index.checked_mul(2)?;
            Some([*self.samples.get(at)?, *self.samples.get(at + 1)?])
        }
    }
}

impl From<Vec<f32>> for SampleBuffer {
    /// Mono frames.
    fn from(frames: Vec<f32>) -> Self {
        Self::mono(frames)
    }
}

/// Where a voice reading as `pitch` says reads its sample `elapsed` frames after it started: the
/// frame, and how far it is from there towards the next one (0 on a frame).
///
/// Why from `elapsed` every time rather than a position advanced frame by frame: the read point
/// then depends on nothing but the frame being rendered (and the frames its pitch moved on), so
/// any split of a render into blocks reads the same points, and no rounding accumulates over a
/// long sample.
fn read_point(elapsed: u64, pitch: &Pitch) -> (u64, f64) {
    if pitch.whole_frames() {
        return (elapsed, 0.0);
    }
    // Positions are never negative, so the conversion to an integer is the floor.
    let position = pitch.at(elapsed);
    let frame = position as u64;
    (frame, position - frame as f64)
}

/// The value `fraction` (0 to 1) of the way from `points[1]` to `points[2]`, with `points[0]`
/// the frame before them and `points[3]` the one after, side by side: the 4-point cubic Hermite
/// with Catmull-Rom tangents, in f64, rounded to f32 once. A point a voice does not play (before
/// its first frame, past its last) is given as 0.
///
/// Why four points and not two: a straight line between neighbors dulls a sound's highs and
/// mirrors images of them below, the more the further a note is from its root; the cubic does
/// both far less, for about twice the arithmetic (sinc reads do better still, at many times it). Why f64 with one rounding: the same bits come out of every
/// build and out of the Python generators that restate it (tests/fixtures/gen_p11_expected.py).
pub fn interpolate(points: [[f32; 2]; 4], fraction: f64) -> [f32; 2] {
    let side = |index: usize| {
        let [y0, y1, y2, y3] = points.map(|point| f64::from(point[index]));
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
        (((c3 * fraction + c2) * fraction + c1) * fraction + y1) as f32
    };
    [side(0), side(1)]
}

/// Whether a voice reading as `pitch` says still reads inside `sample` `elapsed` frames after it
/// started. Once it has read past the last frame it never comes back: its rate is positive.
pub(crate) fn is_sounding(sample: &SampleBuffer, elapsed: u64, pitch: &Pitch) -> bool {
    read_point(elapsed, pitch).0 < sample.frames()
}

/// Reads the voice that started `elapsed` frames before the first of `frames` output frames:
/// `sample` as `pitch` says (at speed 1 as recorded, 2 an octave up), until either runs out,
/// handing each output frame's (left, right) value to `add` with its index (a mono frame is on
/// both sides). Between two frames the value is [`interpolate`]d from four, the points before the
/// first frame and past the last counting 0; on a frame it is that frame.
pub(crate) fn read(
    sample: &SampleBuffer,
    elapsed: u64,
    pitch: &Pitch,
    frames: usize,
    mut add: impl FnMut(usize, [f32; 2]),
) {
    if pitch.whole_frames() {
        // The sample's own speed reads whole frames in a run; every key-60 note plays this way.
        let from = usize::try_from(elapsed).unwrap_or(usize::MAX);
        let channels = usize::from(sample.channels);
        let run = sample
            .samples
            .get(from.saturating_mul(channels)..)
            .unwrap_or_default();
        if channels == 1 {
            for (index, &value) in run.iter().take(frames).enumerate() {
                add(index, [value, value]);
            }
        } else {
            for (index, pair) in run.chunks_exact(2).take(frames).enumerate() {
                add(index, [pair[0], pair[1]]);
            }
        }
        return;
    }
    for (index, played) in (elapsed..).take(frames).enumerate() {
        let (at, fraction) = read_point(played, pitch);
        let Some(current) = sample.frame(at) else {
            break;
        };
        let value = if fraction == 0.0 {
            current
        } else {
            let point = |index: Option<u64>| {
                index
                    .and_then(|index| sample.frame(index))
                    .unwrap_or([0.0; 2])
            };
            let points = [
                point(at.checked_sub(1)),
                current,
                point(Some(at + 1)),
                point(Some(at + 2)),
            ];
            interpolate(points, fraction)
        };
        add(index, value);
    }
}

/// Adds the voice that started `elapsed` frames before `out[0]` into `out`, read as [`read`]
/// reads it and scaled by `gains` (left, right): the left value times the left gain, the right
/// times the right. It adds and never clips, so voices that overlap sum exactly.
pub(crate) fn play(
    sample: &SampleBuffer,
    elapsed: u64,
    pitch: &Pitch,
    gains: [f32; 2],
    out: &mut [[f32; 2]],
) {
    let [left, right] = gains;
    read(sample, elapsed, pitch, out.len(), |index, value| {
        let frame = &mut out[index];
        frame[0] += value[0] * left;
        frame[1] += value[1] * right;
    });
}

/// 2^(s/12) for semitones s = 0..12, each correctly rounded to f64.
const SEMITONE_RATIOS: [f64; 12] = [
    1.0,
    1.0594630943592953,
    1.122462048309373,
    1.189207115002721,
    1.2599210498948732,
    1.3348398541700344,
    std::f64::consts::SQRT_2,
    1.4983070768766815,
    1.5874010519681996,
    1.681792830507429,
    1.7817974362806785,
    1.887748625363387,
];

// The default key is a C, so a key's remainder by 12 counts semitones above the C below it.
const _: () = assert!(DEFAULT_KEY.is_multiple_of(12));

/// The speed a sampler plays a note of `key` at: 2^((key − 60)/12). Key 60 plays the sample as
/// recorded, 72 twice as fast (an octave up), 48 at half speed.
///
/// Why a table of semitone ratios times a power of two rather than `powf`: octaves come out
/// exactly (key 60 is exactly 1, so its renders stay bit-identical to the sample), and the
/// result does not depend on the platform's maths library.
pub(crate) fn playback_speed(key: u8) -> f64 {
    let octaves = i32::from(key / 12) - i32::from(DEFAULT_KEY / 12);
    // Multiplying by a power of two only changes the exponent, so it is exact.
    SEMITONE_RATIOS[usize::from(key % 12)] * 2f64.powi(octaves)
}

/// The speed `semitones` up (down, when negative) from a sound's own pitch plays it at:
/// 2^(semitones/12), from the same table of ratios as [`playback_speed`], so a zone rooted on
/// key 60 plays every key at the sampler's speed for it.
pub(crate) fn semitone_speed(semitones: i32) -> f64 {
    let ratio = SEMITONE_RATIOS[semitones.rem_euclid(12) as usize];
    ratio * 2f64.powi(semitones.div_euclid(12))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Hand-computed: frames 1.. are 0.5, 0.25, 0.125; at gain 0.5 that adds 0.25, 0.125, 0.0625.
    #[test]
    fn plays_from_an_offset_into_both_channels_and_adds() {
        let sample = SampleBuffer::from(vec![1.0, 0.5, 0.25, 0.125]);
        let mut out = [[1.0; 2]; 5];
        play(&sample, 1, &Pitch::steady(1.0), [0.5; 2], &mut out);
        assert_eq!(
            out,
            [[1.25; 2], [1.125; 2], [1.0625; 2], [1.0; 2], [1.0; 2]]
        );
    }

    #[test]
    fn past_the_end_adds_nothing() {
        let sample = SampleBuffer::from(vec![1.0; 4]);
        let mut out = [[0.0; 2]; 3];
        play(&sample, 4, &Pitch::steady(1.0), [1.0; 2], &mut out);
        play(&sample, 99, &Pitch::steady(1.0), [1.0; 2], &mut out);
        play(&sample, 8, &Pitch::steady(0.5), [1.0; 2], &mut out);
        assert_eq!(out, [[0.0; 2]; 3]);
        assert!(is_sounding(&sample, 3, &Pitch::steady(1.0)));
        assert!(!is_sounding(&sample, 4, &Pitch::steady(1.0)));
        assert!(
            is_sounding(&sample, 7, &Pitch::steady(0.5)),
            "frame 7 at half speed reads 3.5"
        );
        assert!(!is_sounding(&sample, 8, &Pitch::steady(0.5)));
    }

    const RAMP: [f32; 4] = [1.0, 0.5, 0.25, 0.125];

    /// Plays `RAMP` at `speed` from its start into `frames` frames of silence, left channel.
    fn ramp_at(speed: f64, frames: usize) -> Vec<f32> {
        let mut out = vec![[0.0; 2]; frames];
        play(
            &SampleBuffer::from(RAMP.to_vec()),
            0,
            &Pitch::steady(speed),
            [1.0; 2],
            &mut out,
        );
        out.iter().map(|frame| frame[0]).collect()
    }

    // Hand-computed: twice the speed reads frames 0 and 2 as they are; half the speed reads
    // every half frame, on the odd outputs the 4-point Hermite half way, (-y0 + 9 y1 + 9 y2 -
    // y3)/16, a point before the first frame or past the last counting 0: (9 + 4.5 - 0.25)/16,
    // (-1 + 4.5 + 2.25 - 0.125)/16, (-0.5 + 2.25 + 1.125)/16, (-0.25 + 1.125)/16.
    #[test]
    fn speed_reads_between_frames_by_hermite_interpolation() {
        assert_eq!(ramp_at(2.0, 4), [1.0, 0.25, 0.0, 0.0]);
        assert_eq!(
            ramp_at(0.5, 10),
            [
                1.0,
                0.828_125,
                0.5,
                0.351_562_5,
                0.25,
                0.179_687_5,
                0.125,
                0.054_687_5,
                0.0,
                0.0
            ]
        );
        assert_eq!(ramp_at(1.0, 5), [1.0, 0.5, 0.25, 0.125, 0.0]);
    }

    // Hand-computed: on a straight line the cubic is the line (1, 2, 3, 4 at 0.25 is 2.25); at a
    // quarter of the way the weights are (-9, 111, 29, -3)/128.
    #[test]
    fn the_cubic_keeps_a_line_and_weighs_its_four_points() {
        let line = [[1.0, -1.0], [2.0, -2.0], [3.0, -3.0], [4.0, -4.0]];
        assert_eq!(interpolate(line, 0.25), [2.25, -2.25]);
        let impulse = |at: usize| {
            let mut points = [[0.0; 2]; 4];
            points[at] = [128.0, 128.0];
            interpolate(points, 0.25)[0]
        };
        assert_eq!([0, 1, 2, 3].map(impulse), [-9.0, 111.0, 29.0, -3.0]);
    }

    // A stereo sample reads each side on its own, also between frames.
    #[test]
    fn a_stereo_sample_plays_left_and_right() {
        let sample = SampleBuffer::stereo(vec![1.0, -1.0, 0.5, 0.25, 7.0]);
        assert_eq!(
            (sample.channels(), sample.frames()),
            (2, 2),
            "the odd value is dropped"
        );
        let mut out = [[0.0; 2]; 3];
        play(&sample, 0, &Pitch::steady(1.0), [1.0, 0.5], &mut out);
        assert_eq!(out, [[1.0, -0.5], [0.5, 0.125], [0.0, 0.0]]);
        // Half way, (-y0 + 9 y1 + 9 y2 - y3)/16 side by side, the points outside the two frames
        // 0: (9 + 4.5)/16, (-9 + 2.25)/16; then (-1 + 4.5)/16, (1 + 2.25)/16.
        let mut slow = [[0.0; 2]; 4];
        play(&sample, 0, &Pitch::steady(0.5), [1.0; 2], &mut slow);
        assert_eq!(
            slow,
            [
                [1.0, -1.0],
                [0.843_75, -0.421_875],
                [0.5, 0.25],
                [0.218_75, 0.203_125]
            ]
        );
    }

    // A zone rooted on key 60 plays each key at the sampler's speed for it, bit for bit.
    #[test]
    fn semitone_speeds_match_the_samplers() {
        for key in 0..=127u8 {
            let speed = semitone_speed(i32::from(key) - 60);
            assert_eq!(speed.to_bits(), playback_speed(key).to_bits(), "key {key}");
        }
        assert_eq!(semitone_speed(-13), SEMITONE_RATIOS[11] / 4.0);
    }

    #[test]
    fn a_pitched_voice_split_across_calls_plays_the_same_frames() {
        let sample =
            SampleBuffer::from((0..50).map(|i| 1.0 / (1.0 + i as f32)).collect::<Vec<_>>());
        let speed = 1.059_463_094_359_295_3; // one semitone up
        let mut whole = [[0.0; 2]; 60];
        play(&sample, 0, &Pitch::steady(speed), [0.5; 2], &mut whole);
        let mut split = [[0.0; 2]; 60];
        let (first, rest) = split.split_at_mut(17);
        play(&sample, 0, &Pitch::steady(speed), [0.5; 2], first);
        play(&sample, 17, &Pitch::steady(speed), [0.5; 2], rest);
        assert_eq!(whole, split);
    }
}
