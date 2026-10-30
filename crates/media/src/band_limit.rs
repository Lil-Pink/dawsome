//! A zone's file above the project's rate, low-passed once on load when the zone reads the file's
//! own frames ([`FileLoop::reads_file_rate`]): read at file_rate / rate times a voice's speed, a
//! frame in two at 96 over 48 kHz, everything the file holds above half the project's rate
//! would fold down at its full level (a 96 kHz file's 32 kHz tone came out at 16 kHz at
//! -12 dBFS, where the converter that path bypasses left it at -167 dBFS). The frames stay as
//! many and where they were, so the loop's points hold as written; the audio thread reads them
//! as before.
//!
//! [`FileLoop::reads_file_rate`]: dawsome_engine::dsp::instrument::FileLoop::reads_file_rate

use std::f64::consts::PI;

/// How far down the taps are designed to hold everything from half the project's rate up, in
/// dB (Kaiser's design rule; at least 95 dB measured at every file rate from 44.8 to 384 kHz
/// over 44.1 and 48 kHz).
const DEPTH_DB: f64 = 96.0;

/// The most taps either side of the center: 1024, which a file reaches past 17 times the
/// project's rate. Why: the load costs frames times taps, and a file's header names its rate.
const MOST_HALF: usize = 1024;

/// Frames each pass over the taps makes at once. Each frame's sum still adds its taps in order,
/// so the bits are the plain sum's; the frames side by side let the compiler use vector
/// instructions.
const BLOCK: usize = 64;

/// The taps: a Kaiser-windowed sinc at `file_rate`, flat to 0.45 times `rate` (within 0.0002 dB)
/// and down by [`DEPTH_DB`] from 0.5 times it, an odd number of them, symmetric (linear phase,
/// applied centered: no frame moves), summing to 1 (0 dB at DC).
///
/// From + − × ÷ and the square root alone, as gen_p24_expected.py's `band_taps` restates them
/// bit for bit: sin and the Bessel I0 by their series, since the platform's maths library
/// differs in its last bit between platforms.
fn taps(file_rate: u32, rate: u32) -> Vec<f64> {
    let ratio = f64::from(rate) / f64::from(file_rate);
    let cut = 0.475 * ratio;
    let width = 2.0 * PI * 0.05 * ratio;
    let half = (((DEPTH_DB - 7.95) / (2.285 * width) / 2.0).ceil() as usize).min(MOST_HALF);
    let beta = 0.1102 * (DEPTH_DB - 8.7);
    let i0_beta = bessel_i0(beta);
    let raw: Vec<f64> = (0..=2 * half)
        .map(|j| {
            let k = j as f64 - half as f64;
            let q = k / half as f64;
            let window = bessel_i0(beta * (1.0 - q * q).sqrt()) / i0_beta;
            let ideal = if j == half {
                2.0 * cut
            } else {
                sin_pi(2.0 * cut * k) / (PI * k)
            };
            window * ideal
        })
        .collect();
    let total = raw.iter().fold(0.0, |total, &tap| total + tap);
    raw.iter().map(|&tap| tap / total).collect()
}

/// sin(πx): x less its nearest whole number n, 14 terms of the Taylor series there (|πr| ≤ π/2,
/// where they reach the last bit), negated for an odd n.
fn sin_pi(x: f64) -> f64 {
    let whole = (x + 0.5).floor();
    let t = PI * (x - whole);
    let t2 = t * t;
    let (mut term, mut total) = (t, t);
    for j in 1..=13_u32 {
        term = -term * t2 / f64::from(2 * j * (2 * j + 1));
        total += term;
    }
    if whole.rem_euclid(2.0) == 1.0 {
        -total
    } else {
        total
    }
}

/// The modified Bessel function I0(x) by 41 terms of its series, Σ ((x/2)^j / j!)², which reach
/// the last bit for the window's x (up to 0.1102 · (96 − 8.7) = 9.62).
fn bessel_i0(x: f64) -> f64 {
    let half = x / 2.0;
    let (mut term, mut total) = (1.0, 1.0);
    for j in 1..=40_u32 {
        term = term * half / f64::from(j);
        total += term * term;
    }
    total
}

/// Each window of `taps.len()` frames in `frames`, the taps over it, into `out` (one frame per
/// window): the sum in f64 from the first tap up, rounded to f32 once.
fn taps_over(frames: &[f64], taps: &[f64], out: &mut [f32]) {
    for (block, chunk) in out.chunks_mut(BLOCK).enumerate() {
        let at = block * BLOCK;
        let mut sums = [0.0_f64; BLOCK];
        let sums = &mut sums[..chunk.len()];
        for (j, &tap) in taps.iter().enumerate() {
            let window = &frames[at + j..at + j + sums.len()];
            for (sum, &value) in sums.iter_mut().zip(window) {
                *sum += tap * value;
            }
        }
        for (frame, &sum) in chunk.iter_mut().zip(sums.iter()) {
            *frame = sum as f32;
        }
    }
}

/// `samples` (`channels` interleaved values a frame) at `file_rate`, low-passed for a project at
/// `rate` by [`taps`], each channel alone; nothing changes at or below the project's rate.
/// `looped` is the loop's first and last frame: its frames are the taps over the loop played on
/// and on (frame start + ((m − start) mod its length) for any m), so read end to end, as the
/// voice reads them, they are exactly the filter over what it plays, and the wrap is as smooth
/// as the loop's own. Every other frame is the taps over the file's own frames, silence before
/// the first and past the last. Why not the file's own frames for the loop's too: past its end
/// a file may go on with another sound, whose ringing would then sound at every wrap (40 times
/// the loop's own second difference in gen_p24's seam96).
pub(crate) fn band_limit(
    samples: &mut [f32],
    channels: usize,
    file_rate: u32,
    rate: u32,
    looped: Option<(usize, usize)>,
) {
    let channels = channels.max(1);
    let frames = samples.len() / channels;
    if rate == 0 || file_rate <= rate || frames == 0 {
        return;
    }
    let taps = taps(file_rate, rate);
    let half = taps.len() / 2;
    let looped = looped.filter(|&(start, end)| start <= end && end < frames);
    let mut out = vec![0.0_f32; frames];
    let mut padded = vec![0.0_f64; frames + 2 * half];
    for channel in 0..channels {
        let own = samples[channel..].iter().step_by(channels).take(frames);
        for (to, &value) in padded[half..half + frames].iter_mut().zip(own) {
            *to = f64::from(value);
        }
        taps_over(&padded, &taps, &mut out);
        if let Some((start, end)) = looped {
            let length = end - start + 1;
            let own = &padded[half..half + frames];
            // Frame i of the loop played on and on, from `half` frames before its start.
            let lap: Vec<f64> = (0..length + 2 * half)
                .map(|i| own[start + (i + length - half % length) % length])
                .collect();
            taps_over(&lap, &taps, &mut out[start..=end]);
        }
        let own = samples[channel..].iter_mut().step_by(channels);
        for (value, &filtered) in own.zip(&out) {
            *value = filtered;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seeded noise in [−0.5, 0.5), so every frame differs.
    fn noise(frames: usize, seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..frames)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (f64::from(state) / f64::from(u32::MAX) - 0.5) as f32
            })
            .collect()
    }

    /// The filter's gain at `nu` cycles a frame.
    fn gain(taps: &[f64], nu: f64) -> f64 {
        let half = (taps.len() / 2) as f64;
        let (re, im) = taps
            .iter()
            .enumerate()
            .fold((0.0, 0.0), |(re, im), (j, tap)| {
                let angle = 2.0 * PI * nu * (j as f64 - half);
                (re + tap * angle.cos(), im - tap * angle.sin())
            });
        re.hypot(im)
    }

    /// The taps over `frames` (silence around them), one frame at a time: the definition
    /// [`taps_over`]'s blocks must give bit for bit.
    fn plain(frames: &[f32], taps: &[f64]) -> Vec<f32> {
        let half = (taps.len() / 2) as isize;
        (0..frames.len() as isize)
            .map(|n| {
                let mut sum = 0.0_f64;
                for (j, &tap) in taps.iter().enumerate() {
                    let m = n - half + j as isize;
                    let value = usize::try_from(m)
                        .ok()
                        .and_then(|m| frames.get(m))
                        .map_or(0.0, |&v| f64::from(v));
                    sum += tap * value;
                }
                sum as f32
            })
            .collect()
    }

    // The spec's numbers, at the rates libraries ship (44.8 kHz, just above, to 384 kHz) over
    // 44.1 and 48 kHz: an odd number of taps, symmetric to the bit, 0 dB at DC, flat to
    // 0.45 · rate within 0.001 dB and at least 90 dB down from 0.5 · rate to the file's half.
    #[test]
    fn the_taps_pass_to_045_of_the_rate_and_hold_90_db_down_from_half_of_it() {
        for rate in [44_100_u32, 48_000] {
            for file_rate in [44_800_u32, 48_000, 50_000, 88_200, 96_000, 192_000, 384_000] {
                if file_rate <= rate {
                    continue;
                }
                let taps = taps(file_rate, rate);
                let n = taps.len();
                assert_eq!(n % 2, 1, "{file_rate} over {rate}");
                assert!(taps.iter().eq(taps.iter().rev()), "{file_rate} over {rate}");
                assert!((gain(&taps, 0.0) - 1.0).abs() < 1e-12);
                let ratio = f64::from(rate) / f64::from(file_rate);
                for i in 0..=200 {
                    let nu = 0.45 * ratio * f64::from(i) / 200.0;
                    let db = 20.0 * gain(&taps, nu).log10();
                    assert!(db.abs() < 0.001, "{file_rate} over {rate}: {db} dB at {nu}");
                }
                for i in 0..=1000 {
                    let nu = 0.5 * ratio + (0.5 - 0.5 * ratio) * f64::from(i) / 1000.0;
                    let db = 20.0 * gain(&taps, nu).log10();
                    assert!(db < -90.0, "{file_rate} over {rate}: {db} dB at {nu}");
                }
            }
        }
    }

    // The series against the platform's own: sin(πx) to 1e-13 (the platform's rounds πx first,
    // up to 77 here: 1e-14 off) and I0 against its tables
    // (I0(1) = 1.2660658777520083, I0(5) = 27.239871823604442; I0(9.6) = 1927.4787689622,
    // by (1/π) ∫ e^(9.6 cos θ) dθ over [0, π], 20000 steps).
    #[test]
    fn the_series_are_the_functions() {
        for i in -2000..=2000 {
            let x = f64::from(i) * 0.0123;
            assert!((sin_pi(x) - (PI * x).sin()).abs() < 1e-13, "{x}");
        }
        // Whole periods and halves move no bit: the taps' symmetry rests on it.
        assert!((sin_pi(0.5) - 1.0).abs() <= f64::EPSILON);
        assert_eq!(sin_pi(-1.5), sin_pi(0.5));
        assert_eq!(sin_pi(2.5), sin_pi(0.5));
        assert_eq!(sin_pi(1.5), -sin_pi(0.5));
        assert_eq!(sin_pi(-0.3), -sin_pi(0.3));
        assert!((bessel_i0(1.0) - 1.266_065_877_752_008_3).abs() < 1e-15);
        assert!((bessel_i0(5.0) - 27.239_871_823_604_442).abs() < 1e-12);
        assert!((bessel_i0(9.6) / 1_927.478_768_962_2 - 1.0).abs() < 1e-12);
    }

    // A 96 kHz file of noise for 48 kHz, its loop 301 frames: every frame outside the loop is
    // the plain sum over the file (silence around it), and the loop's frames, laid end to end
    // four times over, are the plain sum over the loop laid end to end so, in the laps the sum
    // reaches no edge of; the same of a loop of 7 frames, far shorter than the taps (247).
    #[test]
    fn a_loops_frames_are_the_taps_over_the_loop_played_on_and_on() {
        let taps = taps(96_000, 48_000);
        assert_eq!(taps.len(), 247);
        for (start, end) in [(1000, 1300), (1500, 1506)] {
            let file = noise(3000, 0x9e37_79b9);
            let mut held = file.clone();
            band_limit(&mut held, 1, 96_000, 48_000, Some((start, end)));
            let linear = plain(&file, &taps);
            for n in (0..start).chain(end + 1..file.len()) {
                assert_eq!(held[n].to_bits(), linear[n].to_bits(), "frame {n}");
            }
            let laps = 2 + 2 * 247 / (end - start + 1);
            let lap = &file[start..=end];
            let on: Vec<f32> = lap.iter().copied().cycle().take(lap.len() * laps).collect();
            let filtered = plain(&on, &taps);
            let middle = &filtered[lap.len() * (laps / 2)..][..lap.len()];
            let bits = |frames: &[f32]| frames.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
            assert_eq!(
                bits(&held[start..=end]),
                bits(middle),
                "loop {start}..={end}"
            );
        }
    }

    // Each channel is filtered alone (silence on the right stays silence), and a file at or
    // below the project's rate, or one with no frames, keeps its bits.
    #[test]
    fn channels_are_filtered_alone_and_a_file_at_or_below_the_rate_keeps_its_bits() {
        let left = noise(2000, 7);
        let stereo: Vec<f32> = left.iter().flat_map(|&l| [l, 0.0]).collect();
        let mut held = stereo.clone();
        band_limit(&mut held, 2, 96_000, 44_100, Some((500, 1500)));
        let mut mono = left.clone();
        band_limit(&mut mono, 1, 96_000, 44_100, Some((500, 1500)));
        assert!(held.iter().skip(1).step_by(2).all(|&r| r == 0.0));
        let held_left: Vec<f32> = held.iter().copied().step_by(2).collect();
        assert_eq!(held_left, mono);
        assert_ne!(mono, left);
        for (file_rate, rate) in [(48_000, 48_000), (44_100, 48_000), (96_000, 0)] {
            let mut same = stereo.clone();
            band_limit(&mut same, 2, file_rate, rate, Some((500, 1500)));
            assert_eq!(same, stereo, "{file_rate} at {rate}");
        }
        band_limit(&mut [], 2, 96_000, 48_000, Some((0, 0)));
    }
}
