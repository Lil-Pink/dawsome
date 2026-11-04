//! The gate: a level detector, an open/closed state with hysteresis and hold, and a gain that
//! moves towards 1 (open) or the range's floor (closed) with an attack and a release (textbook:
//! detector, hysteresis, hold, ballistics). No sidechain: Dawsome has no sidechain routing.
//!
//! Fixed, not parameters:
//! - The detector is the RMS of the louder side ([`MeanSquare`]): a one-pole mean of
//!   max(l², r²) with one 10 ms time both ways, then its square root. Why RMS and not the peak:
//!   hiss at −50 dBFS RMS peaks near −40 dBFS every few
//!   milliseconds, so a peak detector read it 10 dB hot and the gate never closed on it; the RMS
//!   reads levels as the meters and other gates do.
//! - Hysteresis 3 dB: the gate opens when the detector reaches the threshold T and stays open
//!   down to T − 3 dB, so a level hovering at T does not chatter.
//!
//! Per frame: the detector's level e; open (and the hold refilled) when e ≥ lin(T), or when
//! already open and e ≥ lin(T − 3); else, while open, one frame of the hold spent; else closed.
//! Then the gain g moves towards its target (1 open, lin(range) closed) by the attack's
//! coefficient when rising and the release's when falling: g = c·g + (1 − c)·target. Each side
//! leaves as l·g, rounded to f32 once.
//!
//! Only + − × ÷, comparisons, `sqrt` (correctly rounded) and the crate's [`exp2`], so every
//! platform renders the same bits.
//! At a range of 0 dB the floor is exactly 1, the gain stays exactly 1, and the gate passes the
//! input's bits.

use std::f64::consts::{LOG2_10, LOG2_E};

use super::{exp2, snap64};
use crate::processor::{ParamInfo, ProcessContext, Processor, ProcessorFactory, Taper};

pub const ID: &str = "dawsome.gate";

const PARAMS: [ParamInfo; 5] = [
    ParamInfo {
        key: "threshold_db",
        label: "Threshold",
        min: -80.0,
        max: 0.0,
        default: -40.0,
        unit: "dB",
        whole: false,
        taper: Taper::Linear,
    },
    ParamInfo {
        key: "attack_ms",
        label: "Attack",
        min: 0.1,
        max: 100.0,
        default: 1.0,
        unit: "ms",
        whole: false,
        taper: Taper::Log,
    },
    ParamInfo {
        key: "hold_ms",
        label: "Hold",
        min: 0.0,
        max: 500.0,
        default: 20.0,
        unit: "ms",
        whole: false,
        taper: Taper::Log,
    },
    ParamInfo {
        key: "release_ms",
        label: "Release",
        min: 1.0,
        max: 1000.0,
        default: 100.0,
        unit: "ms",
        whole: false,
        taper: Taper::Log,
    },
    ParamInfo {
        key: "range_db",
        label: "Range",
        min: -80.0,
        max: 0.0,
        default: -80.0,
        unit: "dB",
        whole: false,
        taper: Taper::Linear,
    },
];

const THRESHOLD_DB: usize = 0;
const ATTACK_MS: usize = 1;
const HOLD_MS: usize = 2;
const RELEASE_MS: usize = 3;
const RANGE_DB: usize = 4;

/// How far below the threshold an open gate stays open.
const HYSTERESIS_DB: f64 = 3.0;

/// The detector's averaging time. Why 10 ms: long enough to ride over a low note's cycles (with
/// the hysteresis and the default hold, notes down to about 50 Hz), short enough that the gate
/// hears a hit's start and its tail's end soon after them (LSP's gate's default reactivity).
const DETECTOR_MS: f64 = 10.0;

/// 10^(dB/20), by [`exp2`].
fn lin(db: f64) -> f64 {
    exp2(db / 20.0 * LOG2_10)
}

/// What a level is multiplied by each frame to fall to 1/e of the way in `ms`:
/// e^(−1/(ms·rate/1000)) by [`exp2`], the limiter's release rule; 0 for 0 ms (no smoothing).
fn coefficient(ms: f64, sample_rate: u32) -> f64 {
    if ms == 0.0 {
        return 0.0;
    }
    exp2(-LOG2_E / (ms / 1000.0 * f64::from(sample_rate.max(1))))
}

/// An RMS detector: a one-pole mean of each frame's square (ms = c·ms + (1 − c)·x², tiny
/// values to 0), its square root the level.
struct MeanSquare {
    square: f64,
    coefficient: f64,
}

impl MeanSquare {
    fn new(ms: f64, sample_rate: u32) -> Self {
        Self {
            square: 0.0,
            coefficient: coefficient(ms, sample_rate),
        }
    }

    /// The level after a frame whose square is `square`.
    fn follow(&mut self, square: f64) -> f64 {
        let c = self.coefficient;
        self.square = snap64(c * self.square + (1.0 - c) * square);
        self.square.sqrt()
    }

    fn reset(&mut self) {
        self.square = 0.0;
    }
}

pub(super) struct Factory;

impl ProcessorFactory for Factory {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Gate"
    }

    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn create(&self, sample_rate: u32, values: &[f32]) -> Box<dyn Processor> {
        let mut gate = Gate {
            detector: MeanSquare::new(DETECTOR_MS, sample_rate),
            opens: 0.0,
            closes: 0.0,
            attack: 0.0,
            release: 0.0,
            hold_frames: 0,
            floor: 0.0,
            open: false,
            hold_left: 0,
            gain: 0.0,
            sample_rate,
        };
        for (index, info) in PARAMS.iter().enumerate() {
            gate.set_param(index, values.get(index).copied().unwrap_or(info.default));
        }
        gate.reset();
        Box::new(gate)
    }
}

struct Gate {
    detector: MeanSquare,
    /// The level that opens the gate, lin(T).
    opens: f64,
    /// The level an open gate stays open down to, lin(T − 3 dB).
    closes: f64,
    /// The gain's coefficients rising (attack) and falling (release).
    attack: f64,
    release: f64,
    /// Frames an open gate stays open once the level drops below `closes`.
    hold_frames: u64,
    /// The closed gate's gain, lin(range).
    floor: f64,
    open: bool,
    hold_left: u64,
    gain: f64,
    sample_rate: u32,
}

impl Processor for Gate {
    fn process(&mut self, _context: &ProcessContext, buffer: &mut [[f32; 2]]) {
        for frame in buffer {
            let [left, right] = frame.map(f64::from);
            let level = self.detector.follow((left * left).max(right * right));
            if level >= self.opens || (self.open && level >= self.closes) {
                self.open = true;
                self.hold_left = self.hold_frames;
            } else if self.open && self.hold_left > 0 {
                self.hold_left -= 1;
            } else {
                self.open = false;
            }
            let target = if self.open { 1.0 } else { self.floor };
            let c = if target > self.gain {
                self.attack
            } else {
                self.release
            };
            self.gain = c * self.gain + (1.0 - c) * target;
            *frame = [(left * self.gain) as f32, (right * self.gain) as f32];
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(info) = PARAMS.get(index) else {
            return;
        };
        let value = f64::from(info.clamp(value));
        match index {
            THRESHOLD_DB => {
                self.opens = lin(value);
                self.closes = lin(value - HYSTERESIS_DB);
            }
            ATTACK_MS => self.attack = coefficient(value, self.sample_rate),
            HOLD_MS => {
                let frames = value / 1000.0 * f64::from(self.sample_rate) + 0.5;
                self.hold_frames = frames.floor() as u64;
            }
            RELEASE_MS => self.release = coefficient(value, self.sample_rate),
            RANGE_DB => self.floor = lin(value),
            _ => {}
        }
    }

    /// Closed, at the floor, the detector silent.
    fn reset(&mut self) {
        self.detector.reset();
        self.open = false;
        self.hold_left = 0;
        self.gain = self.floor;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::tests::{bits, noise, whole_and_split, CONTEXT};

    fn gate(values: [f32; 5]) -> Box<dyn Processor> {
        Factory.create(44_100, &values)
    }

    const DEFAULTS: [f32; 5] = [-40.0, 1.0, 20.0, 100.0, -80.0];

    // A steady level reads as its RMS: a constant as itself (the louder side's), a sine as its
    // amplitude over √2 (to the ripple a 10 ms mean leaves at 440 Hz, under 1%).
    #[test]
    fn the_detector_reads_the_rms_of_the_louder_side() {
        let mut detector = MeanSquare::new(10.0, 44_100);
        let mut level = 0.0;
        for _ in 0..44_100 {
            level = detector.follow((0.25f64 * 0.25).max(-0.1f64 * -0.1));
        }
        assert!((level - 0.25).abs() < 1e-12, "{level}");
        let mut detector = MeanSquare::new(10.0, 44_100);
        let mut top = 0.0f64;
        for n in 0..44_100 {
            let x = 0.5 * (std::f64::consts::TAU * 440.0 * n as f64 / 44_100.0).sin();
            let level = detector.follow(x * x);
            if n > 22_050 {
                top = top.max(level);
            }
        }
        let rms = 0.5 / 2f64.sqrt();
        assert!(top > rms && top < rms * 1.01, "{top}");
    }

    // coefficient(0) is 0 (no smoothing). Otherwise the mean moves 1 − 1/e of the way in the
    // time: 10 ms at 44.1 kHz is 441 frames, so c = e^(−1/441), and a square of 1 from silence
    // is 1 − c^441 = 1 − 1/e after them.
    #[test]
    fn the_coefficient_is_the_1_over_e_rule() {
        assert_eq!(coefficient(0.0, 44_100), 0.0);
        let c = coefficient(10.0, 44_100);
        assert!((c - (-1.0f64 / 441.0).exp()).abs() < 1e-15, "{c}");
        let mut detector = MeanSquare::new(10.0, 44_100);
        let mut level = 0.0;
        for _ in 0..441 {
            level = detector.follow(1.0);
        }
        assert!(
            (level - (1.0 - (-1.0f64).exp()).sqrt()).abs() < 1e-12,
            "{level}"
        );
    }

    /// `got` within a millionth of `want`, relatively.
    fn near(got: f32, want: f64) -> bool {
        (f64::from(got) - want).abs() <= want.abs() * 1e-6
    }

    // A level that stays below the threshold never opens the gate: the output is the input at
    // the floor, −80 dB = 10^(−4) (by exp2, to an f64 step).
    #[test]
    fn a_quiet_input_stays_at_the_floor() {
        let input = vec![[0.001, -0.002]; 100];
        let mut out = input.clone();
        gate(DEFAULTS).process(&CONTEXT, &mut out);
        let floor = lin(-80.0);
        assert!((floor - 1e-4).abs() < 1e-19, "{floor}");
        for [l, r] in out {
            assert!(near(l, 0.001 * 1e-4) && near(r, -0.002 * 1e-4), "{l} {r}");
        }
    }

    // Opened by a loud frame, the gate holds for 20 ms (882 frames at 44.1 kHz) after the
    // detector falls below the closing level, then the gain falls by the release.
    #[test]
    fn a_loud_frame_opens_the_gate_and_it_holds_then_releases() {
        let mut out = vec![[0.0f32; 2]; 3000];
        out[0] = [1.0, 0.5];
        let mut instance = gate([-40.0, 0.1, 20.0, 1.0, -80.0]);
        instance.process(&CONTEXT, &mut out);
        // Rising from the floor by the 0.1 ms attack (c = e^(−1/4.41)): g = 1 − (1 − 1e-4)·c.
        let c = coefficient(0.1, 44_100);
        let gain = c * lin(-80.0) + (1.0 - c);
        assert_eq!(out[0], [gain as f32, (0.5 * gain) as f32]);

        // A constant −50 dB after the hit is below the closing level (−43 dB): the detector's
        // mean square, (1 − c) after the hit (c = e^(−1/441)), falls towards q² and crosses
        // θ² after 10·ln((1 − c − q²)/(θ² − q²)) = 40.3 ms (q = 10^(−50/20), θ = 10^(−43/20));
        // 20 ms of hold later the gain falls from 1 to the floor by the 1 ms release, long
        // before frame 5999 (136 ms).
        let mut instance = gate([-40.0, 0.1, 20.0, 1.0, -80.0]);
        let quiet = lin(-50.0) as f32;
        let mut frames = vec![[quiet; 2]; 6000];
        frames[0] = [1.0; 2];
        instance.process(&CONTEXT, &mut frames);
        let opened = frames[100][0];
        assert!((f64::from(opened) - f64::from(quiet)).abs() < 1e-9, "open");
        let closed = frames[5999][0];
        assert!(
            f64::from(closed) < f64::from(quiet) * 1e-3,
            "closed: {closed}"
        );
    }

    // Range 0 dB: the floor is exactly 1, so the gain never moves and the input passes bit for
    // bit, opened or not.
    #[test]
    fn range_0_db_passes_the_input_bit_for_bit() {
        let input = noise(5000);
        let mut out = input.clone();
        gate([-20.0, 1.0, 0.0, 5.0, 0.0]).process(&CONTEXT, &mut out);
        assert_eq!(bits(&out), bits(&input));
    }

    #[test]
    fn any_split_gives_the_same_bits() {
        let mut bursts = noise(6000);
        for frame in &mut bursts[1500..4500] {
            *frame = frame.map(|v| v * 0.001);
        }
        let mut make = || gate([-30.0, 0.5, 2.0, 3.0, -40.0]);
        let (whole, split) = whole_and_split(&mut make, &bursts, &[1, 7, 512, 1, 999, 1700]);
        assert_eq!(bits(&whole), bits(&split));
    }

    #[test]
    fn reset_closes_the_gate_at_the_floor() {
        let mut instance = gate(DEFAULTS);
        let mut loud = vec![[0.9, 0.9]; 100];
        instance.process(&CONTEXT, &mut loud);
        instance.reset();
        let mut quiet = vec![[0.001, 0.001]; 4];
        instance.process(&CONTEXT, &mut quiet);
        assert!(
            quiet.iter().flatten().all(|&v| near(v, 0.001 * 1e-4)),
            "{quiet:?}"
        );
    }
}
