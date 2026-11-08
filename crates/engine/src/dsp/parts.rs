//! The parts native effects are built from, each a piece of state run one frame at a time: a
//! delay line read between frames ([`DelayLine`]), a low-frequency oscillator's phase ([`Lfo`])
//! and a state-variable filter ([`Svf`]). They work in f64 where state recurs; the effect that
//! holds them rounds to f32 once, at its output.
//!
//! Every operation's order is fixed here, and the Python generators that state the effects'
//! known answers restate it, so Linux, Windows and the generators agree.

use std::f64::consts::{PI, TAU};

use super::eq::MAX_FREQ_FRACTION;
use super::sampler::hermite;
use super::snap64;

/// A stereo delay line, read any number of frames back, between frames by the sampler's
/// 4-point cubic Hermite ([`hermite`]).
///
/// Each frame reads first, then writes its input. Why the shortest delay is 2 frames: the read
/// takes the frame before the delayed point as well, which must already be written. Why
/// Hermite and not a straight line between two frames: a linear read dulls the highs, and the
/// dulling moves as an LFO moves the delay (Niemitalo's 4-point, 3rd-order Hermite).
pub struct DelayLine {
    frames: Box<[[f32; 2]]>,
    /// The slot the next frame is written to, which holds the oldest frame.
    write: usize,
}

impl DelayLine {
    /// A silent line that reads up to `max_ms` (above 0) back at `sample_rate`:
    /// ceil(max_ms·rate/1000) + 4 frames, the 4 for the points around the longest read. Runs
    /// on the UI thread, since it allocates.
    pub fn new(max_ms: f64, sample_rate: u32) -> Self {
        let length = (max_ms * f64::from(sample_rate) / 1000.0).ceil() as usize + 4;
        Self {
            frames: vec![[0.0; 2]; length].into_boxed_slice(),
            write: 0,
        }
    }

    /// Silence, as if nothing had been written.
    pub fn reset(&mut self) {
        self.frames.fill([0.0; 2]);
        self.write = 0;
    }

    /// Side `side` (0 left, 1 right) `delay` frames before the frame about to be written,
    /// `delay` clamped to [2, L − 3] for a line of L frames (NaN reads at 2). At `delay` = i + f
    /// for a whole i: the Hermite from the frames i − 1, i, i + 1 and i + 2 back, at f. A whole
    /// delay reads its frame's bits.
    pub fn read(&self, side: usize, delay: f64) -> f32 {
        let length = self.frames.len();
        // `max` before `min`, so NaN (which `max` passes over) reads at the shortest delay.
        let delay = delay.max(2.0).min((length - 3) as f64);
        let whole = delay.floor();
        let fraction = delay - whole;
        let back = whole as usize;
        // The frame `frames` before the one about to be written; `frames` is at most L − 1.
        let at =
            |frames: usize| f64::from(self.frames[(self.write + length - frames) % length][side]);
        hermite(
            [at(back - 1), at(back), at(back + 1), at(back + 2)],
            fraction,
        )
    }

    /// Writes this frame's input, after its reads.
    pub fn write(&mut self, frame: [f32; 2]) {
        self.frames[self.write] = frame;
        self.write = (self.write + 1) % self.frames.len();
    }
}

/// A low-frequency oscillator's phase in radians: 0 at reset, the same for a frame's every
/// read, then advanced by 2π·rate/sr and kept below 2π. Its user takes sin(phase + offset).
pub struct Lfo {
    phase: f64,
    step: f64,
}

impl Lfo {
    /// At phase 0, running at `rate_hz` in a project at `sample_rate`.
    pub fn new(rate_hz: f64, sample_rate: u32) -> Self {
        let mut lfo = Self {
            phase: 0.0,
            step: 0.0,
        };
        lfo.set_rate(rate_hz, sample_rate);
        lfo
    }

    /// Runs at `rate_hz` from the next step on; the phase carries on from where it is.
    pub fn set_rate(&mut self, rate_hz: f64, sample_rate: u32) {
        self.step = TAU * rate_hz / f64::from(sample_rate);
    }

    /// This frame's phase, in [0, 2π).
    pub fn phase(&self) -> f64 {
        self.phase
    }

    /// To the next frame.
    pub fn advance(&mut self) {
        self.phase += self.step;
        if self.phase >= TAU {
            self.phase -= TAU;
        }
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
    }
}

/// An LFO offset of `degrees`, in radians: (degrees·π)/180. The flanger's and the phaser's
/// Spread, the right side's sweep that far ahead of the left's.
pub fn lfo_offset(degrees: f32) -> f64 {
    (f64::from(degrees) * PI) / 180.0
}

/// Which output of the [`Svf`] a frame takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    LowPass,
    HighPass,
    /// Unity gain at the cutoff, so resonance narrows the band without raising it.
    BandPass,
    Notch,
}

/// The cutoff a [`Svf`] goes no lower than.
const MIN_CUTOFF_HZ: f64 = 20.0;

/// A 12 dB/octave state-variable filter per side, solved by trapezoidal integration (Andrew
/// Simper, "Solving the continuous SVF equations using trapezoidal integration and equivalent
/// currents", Cytomic, 2013/2016; re-derived from the paper, no code copied). Why this and not
/// the EQ's cookbook biquad: it stays stable and free of clicks while its cutoff moves.
///
/// At its cutoff a sine comes out of the low- and high-pass Q times as loud, out of the
/// band-pass as loud, out of the notch not at all; elsewhere each output follows the analog
/// prototype at the prewarped frequency.
#[derive(Debug, Clone, Copy)]
pub struct Svf {
    k: f64,
    a1: f64,
    a2: f64,
    a3: f64,
    /// Per side, the two integrators' equivalent currents (ic1, ic2).
    memory: [[f64; 2]; 2],
}

impl Svf {
    /// A silent filter at `cutoff_hz` and `q` in a project at `sample_rate`.
    pub fn new(cutoff_hz: f64, q: f64, sample_rate: u32) -> Self {
        let mut filter = Self {
            k: 0.0,
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            memory: [[0.0; 2]; 2],
        };
        filter.set(cutoff_hz, q, sample_rate);
        filter
    }

    /// Moves the cutoff (clamped to 20 Hz .. 0.45·rate, as the EQ's bands are) and the Q (above
    /// 0); the memory carries on.
    pub fn set(&mut self, cutoff_hz: f64, q: f64, sample_rate: u32) {
        let rate = f64::from(sample_rate);
        let cutoff = cutoff_hz.min(MAX_FREQ_FRACTION * rate).max(MIN_CUTOFF_HZ);
        let g = (PI * cutoff / rate).tan();
        self.k = 1.0 / q;
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    pub fn reset(&mut self) {
        self.memory = [[0.0; 2]; 2];
    }

    /// The filter's `response` to `input` on side `side`, advancing that side one frame.
    pub fn process(&mut self, side: usize, input: f64, response: Response) -> f64 {
        let [ic1, ic2] = &mut self.memory[side];
        let v3 = input - *ic2;
        let v1 = self.a1 * *ic1 + self.a2 * v3;
        let v2 = *ic2 + self.a2 * *ic1 + self.a3 * v3;
        *ic1 = snap64(2.0 * v1 - *ic1);
        *ic2 = snap64(2.0 * v2 - *ic2);
        match response {
            Response::LowPass => v2,
            Response::HighPass => input - self.k * v1 - v2,
            Response::BandPass => self.k * v1,
            Response::Notch => input - self.k * v1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frame n's input on both sides of the line tests: different on each side and each frame.
    fn ramp(n: usize) -> [f32; 2] {
        [n as f32 * 0.1 - 0.5, -(n as f32) * 0.25 - 1.0 / 3.0]
    }

    /// `frames` frames through `line`, each reading side `side` at `delay` before writing ramp.
    fn echoes(line: &mut DelayLine, delay: f64, frames: usize) -> Vec<[f32; 2]> {
        (0..frames)
            .map(|n| {
                let out = [line.read(0, delay), line.read(1, delay)];
                line.write(ramp(n));
                out
            })
            .collect()
    }

    // A whole delay of d frames gives frame n − d's bits on both sides, and 0 before frame d
    // (never written since the line was made). 1 ms at 8 kHz is a line of ceil(8) + 4 = 12
    // frames, so 40 frames wrap it three times.
    #[test]
    fn a_whole_delay_echoes_the_lines_bits() {
        let mut line = DelayLine::new(1.0, 8_000);
        let out = echoes(&mut line, 5.0, 40);
        for (n, frame) in out.iter().enumerate() {
            let want = if n < 5 { [0.0; 2] } else { ramp(n - 5) };
            assert_eq!(frame.map(f32::to_bits), want.map(f32::to_bits), "frame {n}");
        }
    }

    // The 12-frame line reads from 2 to 12 − 3 = 9 frames back: 0, 1 and NaN read 2 back, 100
    // reads 9 back.
    #[test]
    fn a_delay_reads_from_2_frames_to_the_lines_length_less_3() {
        for (delay, back) in [(0.0, 2), (1.0, 2), (f64::NAN, 2), (100.0, 9), (9.0, 9)] {
            let mut line = DelayLine::new(1.0, 8_000);
            let out = echoes(&mut line, delay, 30);
            for (n, frame) in out.iter().enumerate() {
                let want = if n < back { [0.0; 2] } else { ramp(n - back) };
                assert_eq!(frame, &want, "delay {delay}, frame {n}");
            }
        }
    }

    // 20 ms at 44.1 kHz is ceil(882) + 4 = 886 frames, so the longest read is 883 frames back:
    // an impulse written on frame 0 comes out on frame 883 and nowhere else.
    #[test]
    fn the_longest_read_is_the_max_delay_and_a_frame_more() {
        let mut line = DelayLine::new(20.0, 44_100);
        for n in 0..2000 {
            let out = line.read(0, 1e9);
            assert_eq!(out, if n == 883 { 1.0 } else { 0.0 }, "frame {n}");
            line.write(if n == 0 { [1.0; 2] } else { [0.0; 2] });
        }
    }

    // Half way between frames the Hermite weighs its four points (−1, 9, 9, −1)/16 (the
    // sampler's tests derive it). At 2.5 frames back, frame n's points are the frames 1, 2, 3
    // and 4 back, so an impulse on frame 0 reads −1/16, 9/16, 9/16, −1/16 on frames 1 to 4.
    #[test]
    fn half_a_frame_reads_the_hermite_half_way() {
        let mut line = DelayLine::new(1.0, 8_000);
        let mut out = Vec::new();
        for n in 0..8 {
            out.push(line.read(1, 2.5));
            line.write(if n == 0 { [0.0, 1.0] } else { [0.0; 2] });
        }
        assert_eq!(out, [0.0, -0.0625, 0.5625, 0.5625, -0.0625, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn reset_silences_the_line() {
        let mut line = DelayLine::new(1.0, 8_000);
        echoes(&mut line, 3.0, 20);
        line.reset();
        let out = echoes(&mut line, 3.0, 6);
        assert_eq!(&out[..3], [[0.0; 2]; 3]);
        assert_eq!(out[3..], [ramp(0), ramp(1), ramp(2)]);
    }

    /// How far `phase` is from 0 around the circle.
    fn from_zero(phase: f64) -> f64 {
        phase.min(TAU - phase)
    }

    // 1 Hz at 44.1 kHz turns 2π in 44100 steps of 2π/44100: back to 0 (either side of it, by
    // rounding), past π/2 after a quarter of them and π after half; always in [0, 2π).
    #[test]
    fn a_1_hz_lfo_turns_once_in_a_second() {
        let mut lfo = Lfo::new(1.0, 44_100);
        assert_eq!(lfo.phase(), 0.0);
        for step in 1..=44_100 {
            lfo.advance();
            let phase = lfo.phase();
            assert!((0.0..TAU).contains(&phase), "step {step}: {phase}");
            if step == 11_025 {
                assert!((phase - PI / 2.0).abs() < 1e-9, "{phase}");
            }
            if step == 22_050 {
                assert!((phase - PI).abs() < 1e-9, "{phase}");
            }
        }
        assert!(from_zero(lfo.phase()) < 1e-9, "{}", lfo.phase());
    }

    // A new rate takes effect from the next step, from where the phase is; reset goes to 0.
    #[test]
    fn a_new_rate_carries_the_phase_on() {
        let mut lfo = Lfo::new(1.0, 44_100);
        for _ in 0..100 {
            lfo.advance();
        }
        let before = lfo.phase();
        lfo.set_rate(2.0, 44_100);
        assert_eq!(lfo.phase(), before);
        lfo.advance();
        assert_eq!(lfo.phase(), before + TAU * 2.0 / 44_100.0);
        lfo.reset();
        assert_eq!(lfo.phase(), 0.0);
    }

    /// The amplitude of side 0's `response` to a unit sine of `period` frames at 44.1 kHz,
    /// after a second to settle, by projection on the sine and cosine over 10 whole periods
    /// (gen_p5's way). Side 1 gets silence and must stay silent.
    fn amplitude(filter: &mut Svf, response: Response, period: usize) -> f64 {
        let sine = |n: usize| (TAU * (n % period) as f64 / period as f64).sin();
        for n in 0..44_100 {
            filter.process(0, sine(n), response);
            assert_eq!(filter.process(1, 0.0, response), 0.0);
        }
        let (mut on_sine, mut on_cosine) = (0.0, 0.0);
        let frames = 10 * period;
        for n in 44_100..44_100 + frames {
            let out = filter.process(0, sine(n), response);
            let angle = TAU * (n % period) as f64 / period as f64;
            on_sine += out * angle.sin();
            on_cosine += out * angle.cos();
        }
        let scale = 2.0 / frames as f64;
        (on_sine * scale).hypot(on_cosine * scale)
    }

    const RESPONSES: [Response; 4] = [
        Response::LowPass,
        Response::HighPass,
        Response::BandPass,
        Response::Notch,
    ];

    // At the cutoff the prototype's denominator 1 − W² + jW/Q is j/Q (W = 1): the low- and
    // high-pass give Q, the band-pass (W/Q)/(1/Q) = 1, the notch (1 − W²) = 0. 441 Hz at
    // 44.1 kHz is a period of 100 frames.
    #[test]
    fn at_the_cutoff_a_sine_comes_out_q_times_as_loud_through_the_low_and_high_pass() {
        let q = 4.0;
        for (response, want) in RESPONSES.into_iter().zip([q, q, 1.0, 0.0]) {
            let mut filter = Svf::new(441.0, q, 44_100);
            let got = amplitude(&mut filter, response, 100);
            assert!((got - want).abs() < 1e-9, "{response:?}: {got}, not {want}");
        }
    }

    // Away from the cutoff each output is the analog prototype's at the prewarped ratio
    // W = tan(πf/sr)/tan(πfc/sr): low W⁰, high W², band W/Q, notch |1 − W²|, each over
    // |1 − W² + jW/Q|. 882 Hz (a period of 50 frames) through a 441 Hz filter at Q 1/√2,
    // and 441 Hz through an 882 Hz one at Q 2.
    #[test]
    fn away_from_the_cutoff_each_output_follows_the_prototype() {
        for (cutoff, period, q) in [
            (441.0, 50, std::f64::consts::FRAC_1_SQRT_2),
            (882.0, 100, 2.0),
        ] {
            let f = 44_100.0 / period as f64;
            let w = (PI * f / 44_100.0).tan() / (PI * cutoff / 44_100.0).tan();
            let below = (1.0 - w * w).hypot(w / q);
            let wants = [1.0, w * w, w / q, (1.0 - w * w).abs()].map(|top| top / below);
            for (response, want) in RESPONSES.into_iter().zip(wants) {
                let mut filter = Svf::new(cutoff, q, 44_100);
                let got = amplitude(&mut filter, response, period);
                assert!(
                    (got - want).abs() < 1e-9,
                    "{cutoff} Hz, {response:?}: {got}, not {want}"
                );
            }
        }
    }

    // A cutoff outside 20 Hz .. 0.45·rate sounds as the end it is past: 5 Hz as 20 Hz and
    // 30 kHz at 44.1 kHz as 19845 Hz, the same bits frame for frame.
    #[test]
    fn the_cutoff_stays_between_20_hz_and_0_45_of_the_rate() {
        for (asked, end) in [(5.0, 20.0), (30_000.0, 0.45 * 44_100.0)] {
            let mut outside = Svf::new(asked, 0.7, 44_100);
            let mut inside = Svf::new(end, 0.7, 44_100);
            for n in 0..256 {
                let input = if n == 0 { 1.0 } else { 0.0 };
                let got = outside.process(0, input, Response::LowPass);
                let want = inside.process(0, input, Response::LowPass);
                assert_eq!(got.to_bits(), want.to_bits(), "{asked} Hz, frame {n}");
            }
        }
    }

    // Reset clears the memory: an impulse after it rings exactly as through a new filter.
    #[test]
    fn reset_clears_the_filters_memory() {
        let mut used = Svf::new(1000.0, 2.0, 44_100);
        for n in 0..100 {
            used.process(0, (n as f64 * 0.3).sin(), Response::BandPass);
            used.process(1, (n as f64 * 0.7).cos(), Response::BandPass);
        }
        used.reset();
        let mut fresh = Svf::new(1000.0, 2.0, 44_100);
        for n in 0..64 {
            let input = if n == 0 { 1.0 } else { 0.0 };
            for side in 0..2 {
                let got = used.process(side, input, Response::HighPass);
                let want = fresh.process(side, input, Response::HighPass);
                assert_eq!(got.to_bits(), want.to_bits(), "side {side}, frame {n}");
            }
        }
    }
}
