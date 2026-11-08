//! Dawsome Synth: a polyphonic subtractive synth a synth channel plays, sent its notes, the
//! sustain pedal's note-offs, pitch bend and the mod wheel as a plugin instrument is.
//!
//! Its 38 parameters are all here (the instrument window and the presets save every one by
//! key): two oscillators with unison and noise, a resonant low-pass per voice with its own
//! envelope, key tracking, velocity → cutoff and a drive, one LFO to the pitch and the cutoff
//! (the mod wheel adds vibrato), glide, and the amp envelope.
//!
//! - Voices: 16. A note takes the free voice of lowest slot, else steals the oldest (the
//!   smallest count of notes started since the last reset). A stolen voice starts its glide
//!   again, keeps its filter's memory and attacks both envelopes from their values, so a steal
//!   does not click; a free voice starts its filter envelope at 0 and its filter's memory
//!   clear, so an old note's ring never sounds under a new one. Either way its oscillators'
//!   phases and its drive's memory go on from where they were.
//! - Unison: a note plays U voices of each oscillator, U the unison control at its start;
//!   voice k is detuned detune·(k/(U − 1) − 0.5) cents and panned spread·(2k/(U − 1) − 1)
//!   by the channel pan law, both 0 for one voice.
//! - Pitch, per oscillator and unison voice: p = (key − 69) + bend + glide + vibrato +
//!   12·octave + semitones + cents/100 + detune/100 semitones from A4; f = min(440·2^(p/12),
//!   0.45·rate). The bend is the device's value / 4096: ±2 semitones. The vibrato is
//!   lfo·(lfo_pitch + w), w the mod wheel's value / 127: the wheel at the top adds a semitone.
//! - Glide: a note started while another is held (unreleased, its gate not closing on this
//!   frame, sounding since an earlier frame) slides from the pitch the previous note has now:
//!   glide = g0·(1 − m/N) semitones on its m-th frame, g0 that pitch less its key, over
//!   N = ⌊glide_ms/1000·rate + ½⌋ frames, fixed at its start. A straight line in semitones, so
//!   every octave takes the same time. Its envelopes start as any note's (polyphonic legato
//!   glide); key tracking follows its key, not the slide.
//! - Oscillators: a phase t in [0, 1); the value from t, then t += f/rate, less 1 once it
//!   reaches 1. Sine sin(2π·t); triangle 1 − 4·|frac(t + ¼) − ½| (in phase with the sine);
//!   saw 2t − 1 − blep(t); square (t < width ? 1 : −1) + blep(t) − blep(frac(t + 1 − width))
//!   − (2·width − 1), its mean over a period taken off, so the pulse carries no DC at any width
//!   (at 0.5 that is +0.0, which leaves every bit). blep is the degree-2 PolyBLEP residual
//!   (Välimäki and Huovilainen, IEEE SPM 24(2), 2007, re-derived), which takes the step out of
//!   the saw's wrap and the square's two edges.
//! - Phases run free, as an analog polysynth's oscillators do: a reset puts slot s's unison
//!   voice k at frac(s·φ + k/3) on both oscillators (φ = 0.6180339887498949, the golden ratio's
//!   fraction: the slots spread round the cycle, a slot's three voices a third apart), and a
//!   note, free or stolen, starts from wherever its slot's phases are. So a chord's notes do not
//!   start in phase (their first cycles stack less) and a repeated note does not repeat its
//!   frames. A unison voice runs only while a note plays it.
//! - Noise: a xorshift32 per voice slot, seeded slot + 1 at reset, stepped each frame the voice
//!   sounds (a slot's next note goes on from it): s/2^31 − 1, the same on both sides.
//! - Mix, per side: Σ_k gain_k·((1 − mix)·level1·osc1_k + mix·level2·osc2_k) / U + noise·nz.
//! - Filter: a low-pass per voice and side, the Filter effect's state-variable filter
//!   ([`Svf`], Simper's) and its resonance law, Q = 0.5·2^(r·log2 40) from 0.5 to 20, with no
//!   level compensation. Its cutoff each frame is 20·2^(cutoff·log2 1000 + s/12) Hz,
//!   s = env_amount·fenv·96 + key_track·(key − 60) + lfo·lfo_cutoff·48 + vel_cutoff·(vel − 1)·48
//!   semitones (8 octaves of envelope, key tracking 1:1 from C4, 4 octaves of LFO and of
//!   velocity), clamped to 20 Hz .. 0.45·rate. At cutoff 100% with nothing moving it the filter
//!   is open: the voice hears its mix exactly. Its memory runs on every frame all the same, so
//!   a cutoff moved off 100% mid-note starts from a filter that was listening, not from silence.
//! - Drive, per voice and side, after the filter (or the open filter's mix) and before the
//!   envelope: x becomes the Distortion effect's antialiased tanh ([`adaa_tanh`]: the mean of tanh
//!   between the frame before's d·x and this one's) of d·x, over tanh(d), d = 1 + 5·drive
//!   (6 at the top), so a full-scale voice peaks at 1, then times the Distortion's auto gain
//!   ([`auto_gain`], lin(−D/(1 + D/40))) at D = 20·log10(d) dB, the drive whose gain is d (−5.2 dB
//!   at drive 0.2, −11.2 dB at the top), so the knob moves the tone more than the level. Per voice,
//!   so a chord's notes do not intermodulate. Its memory, the frame before's d·x, is 0 at reset and
//!   kept across notes (one frame of state: nothing rings on). At drive 0 the voice is not shaped
//!   at all, bit for bit the voice before the Drive, and the memory follows x (d is 1 there), so a
//!   drive turned up mid-note starts from the sound.
//! - Envelopes: exponential segments (Redmon's "Envelope generators" math, re-derived): each
//!   time control v is t = 0.001·2^(v·4·log2 10) s (1 ms to 10 s) and moves the level by
//!   c = 2^(−K/(t·rate)) a frame. Attack: env = 1.3·(1 − c) + env·c until it reaches 1, which
//!   from 0 takes t (K = log2(1.3/0.3)); decay: env = (S − 0.0001)·(1 − c) + env·c down to the
//!   sustain level S; sustain: S; release: env = −0.0001·(1 − c) + env·c down to 0, where the
//!   amp envelope frees the voice (K = log2(1.0001/0.0001) for both). The overshoot targets
//!   (1.3, S − 0.0001, −0.0001) end each segment in finite time. The filter's envelope (fenv)
//!   runs the same segments on its own controls.
//! - LFO: one, free-running from reset, where its phase is 0: a phase t in [0, 1) moving
//!   rate/sample rate a frame. Sine sin(2π·t), triangle the oscillator's, square (t < ½ ? 1 :
//!   −1), saw 2t − 1.
//! - Output, per side: each sounding voice gives driven·env·(1 − vel_amp·(1 − velocity));
//!   their sum in slot order times 10^(volume/20), rounded to f32 once.
//!
//! A block's events are taken in frame order (those on one frame in the order they came), and each
//! one acts before its frame renders, so a voice's state never depends on where blocks are cut.
//! Operation order (tests/fixtures/gen_p33_expected.py states it, and this follows): the constants
//! 4·log2 10 and 3·log2 10 first; the pitch's terms added one at a time in the order above; the
//! cutoff's four terms summed left to right, then /12, then added to cutoff·log2 1000; blep
//! ((2x − x·x) − 1) before the jump, ((x·x + 2x) + 1) after it; the square's width compared as the
//! f32 widened, its mean 2·width − 1 formed once a settings change and taken off last; a reset's
//! phase (slot·φ + k/3), then frac; the drive's d = 1 + drive·5, tanh(d) and its auto gain
//! (20·log10 d, then the Distortion's law) formed once a settings change; a frame is the LFO's
//! value (0, not computed, when nothing uses it), then per voice in slot order the gate's end (the
//! frame that finds its count at 0 releases both envelopes), the amp envelope's step and the filter
//! envelope's (each new value is the frame's), the pitch when it moves (after a retune, through a
//! glide up to its last frame, under vibrato), each oscillator's values and phase steps (oscillator
//! 1's voices, then 2's), the noise's step, the gate's count, the cutoff (its exp2 and the
//! coefficients worked out again only when its semitones' bits change), per side the unison sum
//! from 0 in voice order, then / U, then + noise·nz, the filter, the drive (u = d·x, the mean of
//! tanh from the memory to u, the memory = u, then / tanh(d), then ·auto gain), then ·env, then
//! ·velocity gain; the voice's frame count; then the LFO's step. sum = 0, then + each voice;
//! out = f32(sum·lin(volume)).
//!
//! Only + − × ÷, comparisons, the crate's [`exp2`], `floor`, `abs`, `sin` and `tan` (the
//! platform's, in the sine wave, the sine LFO, the pan law and the filter's coefficients: the known
//! answers hold them to 1e-6), and at a drive above 0 the platform's `tanh`, `exp`, `ln_1p` and
//! `log10` (the Distortion's shaper and the auto gain's dB, held to 1e-6 too), with every value in
//! f64 and one f32 rounding per output frame.

use std::f64::consts::{LOG2_10, TAU};

use super::distortion::{adaa_tanh, auto_gain};
use super::eq::MAX_FREQ_FRACTION;
use super::filter::{cutoff_hz, q as filter_q, LOG2_1000};
use super::laws::channel_pan_gains;
use super::parts::{Response, Svf};
use super::{exp2, lin};
use crate::processor::{
    ParamGroup, ParamInfo, PortEvent, ProcessContext, Processor, ProcessorFactory, Taper,
};
use crate::render::NOTES_PER_PLUGIN;

/// The registry key a synth channel names it by; permanent, whatever the synth is called.
pub const ID: &str = "dawsome.synth";

/// The name the Library and the channel show.
pub const NAME: &str = "Dawsome Synth";

/// Voices sounding at once; a note past them steals the oldest.
const VOICES: usize = 16;

/// The attack's K: log2(1.3/0.3), so that from 0 the level reaches 1 in the attack's time.
/// Written as a literal on both sides (here and in the generator) so no log call can differ.
const K_ATTACK: f64 = 2.115477217419936;

/// The decay's and the release's K: log2(1.0001/0.0001), the time to fall from 1 to within
/// 0.0001 of the target.
const K_DECAY: f64 = 13.287856641840545;

/// The attack's target, past 1 so the segment ends.
const ATTACK_TARGET: f64 = 1.3;

/// How far past its end the decay and the release aim, so each ends.
const UNDERSHOOT: f64 = 0.0001;

/// One semitone of pitch bend per this many steps of the device's value: ±2 semitones.
const BEND_STEPS_PER_SEMITONE: f64 = 4096.0;

/// The mod wheel's value at the top: w = value / 127, so the top adds a semitone of vibrato.
const WHEEL_TOP: f64 = 127.0;

/// How far the filter envelope moves the cutoff at env amount 100%: 8 octaves, in semitones.
const ENV_SEMITONES: f64 = 96.0;

/// How far the LFO at LFO → cutoff 100%, and velocity at velocity → cutoff 100%, move the
/// cutoff: 4 octaves each, in semitones.
const LFO_CUTOFF_SEMITONES: f64 = 48.0;
const VELOCITY_SEMITONES: f64 = 48.0;

/// The key whose cutoff key tracking leaves where it is: C4.
const KEY_TRACK_FROM: f64 = 60.0;

/// How much harder than 1 the Drive's top drives a voice into tanh: d = 1 + 5·drive, 6 at
/// the top.
const DRIVE_RANGE: f64 = 5.0;

/// A parameter of the synth: a linear slider in `min..max` (the log controls map their 0..1
/// through [`ProcessorFactory::shown`]).
const fn param(
    key: &'static str,
    label: &'static str,
    min: f32,
    max: f32,
    default: f32,
    unit: &'static str,
    whole: bool,
) -> ParamInfo {
    ParamInfo {
        key,
        label,
        min,
        max,
        default,
        unit,
        whole,
        taper: Taper::Linear,
    }
}

/// The parameters, in the order the instrument window groups them: oscillator 1, oscillator 2
/// with the mix, unison with the noise, the filter, its envelope, the amp envelope, the LFO,
/// and how it plays.
const PARAMS: [ParamInfo; 38] = [
    param("osc1_wave", "Osc 1 wave", 0.0, 3.0, 2.0, "", true),
    param("osc1_width", "Osc 1 width", 0.05, 0.95, 0.5, "%", false),
    param("osc1_octave", "Osc 1 octave", -2.0, 2.0, 0.0, "oct", true),
    param("osc1_semitones", "Osc 1 semi", -12.0, 12.0, 0.0, "st", true),
    param("osc1_cents", "Osc 1 fine", -100.0, 100.0, 0.0, "ct", false),
    param("osc1_level", "Osc 1 level", 0.0, 1.0, 1.0, "%", false),
    param("osc2_wave", "Osc 2 wave", 0.0, 3.0, 3.0, "", true),
    param("osc2_width", "Osc 2 width", 0.05, 0.95, 0.5, "%", false),
    param("osc2_octave", "Osc 2 octave", -2.0, 2.0, 0.0, "oct", true),
    param("osc2_semitones", "Osc 2 semi", -12.0, 12.0, 0.0, "st", true),
    param("osc2_cents", "Osc 2 fine", -100.0, 100.0, 0.0, "ct", false),
    param("osc2_level", "Osc 2 level", 0.0, 1.0, 1.0, "%", false),
    param("osc_mix", "Osc mix", 0.0, 1.0, 0.0, "%", false),
    param("unison", "Unison", 1.0, 3.0, 1.0, "", true),
    param("detune", "Detune", 0.0, 50.0, 10.0, "ct", false),
    param("spread", "Spread", 0.0, 1.0, 0.5, "%", false),
    param("noise", "Noise", 0.0, 1.0, 0.0, "%", false),
    param("cutoff", "Cutoff", 0.0, 1.0, 1.0, "Hz", false),
    param("resonance", "Resonance", 0.0, 1.0, 0.1, "%", false),
    param("filter_env", "Env amount", -1.0, 1.0, 0.0, "%", false),
    param("key_track", "Key track", 0.0, 1.0, 0.0, "%", false),
    param("drive", "Drive", 0.0, 1.0, 0.0, "%", false),
    param("filter_attack", "Filter attack", 0.0, 1.0, 0.0, "ms", false),
    param("filter_decay", "Filter decay", 0.0, 1.0, 0.5, "ms", false),
    param(
        "filter_sustain",
        "Filter sustain",
        0.0,
        1.0,
        0.0,
        "%",
        false,
    ),
    param(
        "filter_release",
        "Filter release",
        0.0,
        1.0,
        0.5,
        "ms",
        false,
    ),
    param("amp_attack", "Amp attack", 0.0, 1.0, 0.0, "ms", false),
    param("amp_decay", "Amp decay", 0.0, 1.0, 0.5, "ms", false),
    param("amp_sustain", "Amp sustain", 0.0, 1.0, 1.0, "%", false),
    param("amp_release", "Amp release", 0.0, 1.0, 0.5, "ms", false),
    param("lfo_rate", "LFO rate", 0.0, 1.0, 0.75, "Hz", false),
    param("lfo_shape", "LFO shape", 0.0, 3.0, 0.0, "", true),
    param("lfo_pitch", "LFO → pitch", 0.0, 12.0, 0.0, "st", false),
    param("lfo_cutoff", "LFO → cutoff", 0.0, 1.0, 0.0, "%", false),
    param("glide_ms", "Glide", 0.0, 1000.0, 0.0, "ms", false),
    param("volume_db", "Volume", -48.0, 6.0, -6.0, "dB", false),
    param("vel_amp", "Velocity → level", 0.0, 1.0, 1.0, "%", false),
    param("vel_cutoff", "Velocity → cutoff", 0.0, 1.0, 0.0, "%", false),
];

/// Where each oscillator's six parameters start, and their places from there.
const OSC1: usize = 0;
const OSC2: usize = 6;
const WAVE: usize = 0;
const WIDTH: usize = 1;
const OCTAVE: usize = 2;
const SEMITONES: usize = 3;
const CENTS: usize = 4;
const LEVEL: usize = 5;

const OSC1_WAVE: usize = OSC1 + WAVE;
const OSC2_WAVE: usize = OSC2 + WAVE;
const OSC_MIX: usize = 12;
const UNISON: usize = 13;
const DETUNE: usize = 14;
const SPREAD: usize = 15;
const NOISE: usize = 16;
const CUTOFF: usize = 17;
const RESONANCE: usize = 18;
const ENV_AMOUNT: usize = 19;
const KEY_TRACK: usize = 20;
const DRIVE: usize = 21;
/// Each envelope's four controls run attack, decay, sustain, release from its first.
const FILTER_ATTACK: usize = 22;
const FILTER_DECAY: usize = 23;
const FILTER_RELEASE: usize = 25;
const AMP_ATTACK: usize = 26;
const AMP_DECAY: usize = 27;
const AMP_RELEASE: usize = 29;
const LFO_RATE: usize = 30;
const LFO_SHAPE: usize = 31;
const LFO_PITCH: usize = 32;
const LFO_CUTOFF: usize = 33;
const GLIDE_MS: usize = 34;
const VOLUME_DB: usize = 35;
const VEL_AMP: usize = 36;
const VEL_CUTOFF: usize = 37;

/// The most unison voices a note plays (the unison control's top).
const UNISON_MAX: usize = 3;

/// The golden ratio's fraction, (√5 − 1)/2, by which a reset spreads the voice slots' phases
/// round the cycle. Written as a literal on both sides (here and in the generator).
const PHI: f64 = 0.618_033_988_749_894_9;

/// The parameters set by a time control (0..1 for 1 ms to 10 s).
const TIMES: [usize; 6] = [
    FILTER_ATTACK,
    FILTER_DECAY,
    FILTER_RELEASE,
    AMP_ATTACK,
    AMP_DECAY,
    AMP_RELEASE,
];

/// The headings the instrument window shows the parameters under, in their order: each
/// oscillator (the second with the mix), unison with the noise, the filter, its envelope, the
/// amp envelope, the LFO, and how it plays.
const GROUPS: [ParamGroup; 8] = [
    ("Osc 1", OSC1..OSC2),
    ("Osc 2", OSC2..UNISON),
    ("Unison", UNISON..CUTOFF),
    ("Filter", CUTOFF..FILTER_ATTACK),
    ("Filter env", FILTER_ATTACK..AMP_ATTACK),
    ("Amp env", AMP_ATTACK..LFO_RATE),
    ("LFO", LFO_RATE..GLIDE_MS),
    ("Play", GLIDE_MS..PARAMS.len()),
];

/// Each parameter's name on its knob, under its heading: the label without what the heading
/// says ("Attack" under "Amp env", "Wave" under "Osc 1").
const KNOB_LABELS: [&str; 38] = [
    "Wave",
    "Width",
    "Octave",
    "Semi",
    "Fine",
    "Level",
    "Wave",
    "Width",
    "Octave",
    "Semi",
    "Fine",
    "Level",
    "Mix",
    "Voices",
    "Detune",
    "Spread",
    "Noise",
    "Cutoff",
    "Resonance",
    "Env amt",
    "Key track",
    "Drive",
    "Attack",
    "Decay",
    "Sustain",
    "Release",
    "Attack",
    "Decay",
    "Sustain",
    "Release",
    "Rate",
    "Shape",
    "\u{2192} Pitch",
    "\u{2192} Cutoff",
    "Glide",
    "Volume",
    "Vel\u{2192}level",
    "Vel\u{2192}cutoff",
];

const WAVES: [&str; 4] = ["Sine", "Triangle", "Saw", "Square"];
const UNISON_VOICES: [&str; 3] = ["1 voice", "2 voices", "3 voices"];
const LFO_SHAPES: [&str; 4] = ["Sine", "Triangle", "Square", "Saw"];

/// A time control's milliseconds, 2^(v·4·log2 10): 1 ms at 0, 10 ms at 0.25, 100 ms at 0.5,
/// 1 s at 0.75 and 10 s at 1. The constant 4·log2 10 (log2 10000) is formed first, so the
/// generator gets its bits.
fn time_ms(value: f32) -> f64 {
    exp2(f64::from(value) * (4.0 * LOG2_10))
}

/// A time control's seconds, as the envelope takes them: 0.001·2^(v·4·log2 10).
fn time_seconds(value: f32) -> f64 {
    0.001 * time_ms(value)
}

/// The LFO rate control's frequency: 0.01 Hz at 0 to 20 Hz at 1, evenly in octaves.
fn lfo_rate_hz(value: f32) -> f64 {
    0.01 * exp2(f64::from(value) * (1.0 + 3.0 * LOG2_10))
}

/// What a level is multiplied by each frame on a segment of `k` over the time control `value`.
fn coefficient(k: f64, value: f32, rate: f64) -> f64 {
    exp2(-k / (time_seconds(value) * rate))
}

/// The value name of choice `value` among `names`, `value` being the parameter `index`'s.
fn named(names: &'static [&'static str], index: usize, value: f32) -> Option<&'static str> {
    let first = PARAMS[index].min;
    let place = PARAMS[index].clamp(value) - first;
    names.get(place as usize).copied()
}

/// The degree-2 PolyBLEP residual of a unit step at phase 0, for a phase `t` moving `dt` a
/// frame: ((2x − x·x) − 1) for x = t/dt just after the step, ((x·x + 2x) + 1) for
/// x = (t − 1)/dt just before it, 0 elsewhere. It runs from −1 to 1 across the step, so
/// subtracting it takes a saw's −2 jump out and adding it a square's +2.
fn blep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let x = t / dt;
        2.0 * x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + 2.0 * x + 1.0
    } else {
        0.0
    }
}

/// x − ⌊x⌋.
fn frac(x: f64) -> f64 {
    x - x.floor()
}

/// Where a reset leaves slot `slot`'s unison voice `k`, on both oscillators: frac(slot·φ + k/3),
/// the slots spread round the cycle by the golden ratio and a slot's three voices a third apart.
/// A note never sets them (they run free), so a chord's notes do not start in phase.
fn start_phase(slot: usize, k: usize) -> f64 {
    frac(slot as f64 * PHI + k as f64 / UNISON_MAX as f64)
}

/// sin(2π·t): the sine oscillator's and the sine LFO's value at phase t.
fn sine(t: f64) -> f64 {
    (TAU * t).sin()
}

/// 1 − 4·|frac(t + ¼) − ½|: the triangle in phase with the sine, the oscillator's and the
/// LFO's.
fn triangle(t: f64) -> f64 {
    1.0 - 4.0 * (frac(t + 0.25) - 0.5).abs()
}

/// One step of the xorshift32 generator (Marsaglia's 13, 17, 5).
fn xorshift32(mut state: u32) -> u32 {
    state ^= state << 13;
    state ^= state >> 17;
    state ^= state << 5;
    state
}

/// Unison voice `k` of `count`'s detune in cents: detune·(k/(U − 1) − 0.5), 0 for one voice.
fn detune_cents(detune: f64, k: usize, count: usize) -> f64 {
    if count == 1 {
        0.0
    } else {
        detune * (k as f64 / (count - 1) as f64 - 0.5)
    }
}

/// Unison voice `k` of `count`'s pan: spread·(2k/(U − 1) − 1), 0 for one voice.
fn unison_pan(spread: f64, k: usize, count: usize) -> f64 {
    if count == 1 {
        0.0
    } else {
        spread * (2.0 * k as f64 / (count - 1) as f64 - 1.0)
    }
}

/// An oscillator's wave, as its wave control's values name them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wave {
    Sine,
    Triangle,
    Saw,
    Square,
}

impl Wave {
    /// The wave a whole value of the control (0 to 3) names.
    fn of(value: f32) -> Self {
        match value as u8 {
            0 => Wave::Sine,
            1 => Wave::Triangle,
            2 => Wave::Saw,
            _ => Wave::Square,
        }
    }
}

/// One oscillator's settings, as its parameters hold them.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Osc {
    wave: Wave,
    /// The square's width: the part of a period it is high for.
    width: f64,
    /// The square's mean over a period, 2·width − 1, which its value has taken off (0 at
    /// width 0.5, where the subtraction leaves every bit).
    dc: f64,
    octave: f64,
    semitones: f64,
    cents: f64,
}

impl Osc {
    /// The oscillator whose parameters start at `base` in `values`.
    fn new(values: &[f32; PARAMS.len()], base: usize) -> Self {
        let width = f64::from(values[base + WIDTH]);
        Self {
            wave: Wave::of(values[base + WAVE]),
            width,
            dc: 2.0 * width - 1.0,
            octave: f64::from(values[base + OCTAVE]),
            semitones: f64::from(values[base + SEMITONES]),
            cents: f64::from(values[base + CENTS]),
        }
    }

    /// Its value at phase `t`, the phase moving `dt` a frame.
    fn value(&self, t: f64, dt: f64) -> f64 {
        match self.wave {
            Wave::Sine => sine(t),
            Wave::Triangle => triangle(t),
            Wave::Saw => 2.0 * t - 1.0 - blep(t, dt),
            Wave::Square => {
                let naive = if t < self.width { 1.0 } else { -1.0 };
                naive + blep(t, dt) - blep(frac(t + 1.0 - self.width), dt) - self.dc
            }
        }
    }
}

/// The pitch in semitones from A4 of `key` on oscillator `osc`, detuned `detune` cents, under
/// `bend`, `glide` and `vibrato` (semitones): its terms added one at a time, in this
/// order (the bits depend on it).
fn pitch(key: u8, bend: f64, glide: f64, vibrato: f64, osc: &Osc, detune: f64) -> f64 {
    let mut pitch = (f64::from(key) - 69.0) + bend;
    pitch += glide;
    pitch += vibrato;
    pitch += 12.0 * osc.octave;
    pitch += osc.semitones;
    pitch += osc.cents / 100.0;
    pitch += detune / 100.0;
    pitch
}

/// A glide from `from` semitones on its frame `age` of `frames`: from·(1 − age/frames), a
/// straight line in semitones that reaches 0 on frame `frames` and stays there (0 throughout
/// with no glide, 0 frames).
fn glide_offset(from: f64, age: u64, frames: u64) -> f64 {
    if age < frames {
        from * (1.0 - age as f64 / frames as f64)
    } else {
        0.0
    }
}

/// The LFO's shape, as the shape control's values name them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LfoShape {
    Sine,
    Triangle,
    Square,
    Saw,
}

impl LfoShape {
    /// The shape a whole value of the control (0 to 3) names.
    fn of(value: f32) -> Self {
        match value as u8 {
            0 => LfoShape::Sine,
            1 => LfoShape::Triangle,
            2 => LfoShape::Square,
            _ => LfoShape::Saw,
        }
    }

    /// Its value at phase `t`, uncorrected: it moves far too slowly to alias.
    fn value(self, t: f64) -> f64 {
        match self {
            LfoShape::Sine => sine(t),
            LfoShape::Triangle => triangle(t),
            LfoShape::Square => {
                if t < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoShape::Saw => 2.0 * t - 1.0,
        }
    }
}

/// The LFO's settings.
#[derive(Debug, Clone, Copy, PartialEq)]
struct LfoSettings {
    shape: LfoShape,
    /// What its phase moves by each frame: its rate over the sample rate.
    step: f64,
    /// How far it swings the pitch, in semitones, before the mod wheel's share.
    pitch: f64,
}

/// What the filter's cutoff follows, and its Q.
#[derive(Debug, Clone, Copy, PartialEq)]
struct FilterSettings {
    /// The cutoff control's octaves above 20 Hz: cutoff·log2 1000.
    octaves: f64,
    q: f64,
    env_amount: f64,
    key_track: f64,
    lfo_depth: f64,
    velocity_depth: f64,
    /// At cutoff 100% with nothing to move it the filter is open: its output is not taken, so
    /// the voice hears its mix exactly.
    open: bool,
}

impl FilterSettings {
    fn new(values: &[f32; PARAMS.len()]) -> Self {
        let cutoff = values[CUTOFF];
        let depths = [ENV_AMOUNT, KEY_TRACK, LFO_CUTOFF, VEL_CUTOFF].map(|i| f64::from(values[i]));
        let [env_amount, key_track, lfo_depth, velocity_depth] = depths;
        Self {
            octaves: f64::from(cutoff) * LOG2_1000,
            q: filter_q(values[RESONANCE]),
            env_amount,
            key_track,
            lfo_depth,
            velocity_depth,
            open: cutoff == PARAMS[CUTOFF].max && depths.iter().all(|&depth| depth == 0.0),
        }
    }

    /// How far a voice on `key` at `velocity`, its filter envelope at `envelope` and the LFO at
    /// `lfo`, moves the cutoff, in semitones: the four terms summed left to right; 0 while the
    /// filter is open (nothing moves it then).
    fn semitones(&self, envelope: f64, key: u8, lfo: f64, velocity: f64) -> f64 {
        if self.open {
            return 0.0;
        }
        self.env_amount * envelope * ENV_SEMITONES
            + self.key_track * (f64::from(key) - KEY_TRACK_FROM)
            + lfo * self.lfo_depth * LFO_CUTOFF_SEMITONES
            + self.velocity_depth * (velocity - 1.0) * VELOCITY_SEMITONES
    }

    /// The cutoff in Hz `semitones` from the control's, before the filter's clamp:
    /// 20·2^(octaves + semitones/12). At 0 it is the control's own, the open filter's.
    fn hz(&self, semitones: f64) -> f64 {
        20.0 * exp2(self.octaves + semitones / 12.0)
    }

    /// Whether the LFO moves the cutoff.
    fn follows_lfo(&self) -> bool {
        !self.open && self.lfo_depth != 0.0
    }
}

/// How hard each voice side is driven into tanh: the Distortion effect's antialiased shaper
/// ([`adaa_tanh`]) on d·x, over tanh(d) so a full-scale voice peaks at 1, then times the
/// Distortion's auto gain at the drive in dB whose gain is d ([`auto_gain`]), so the knob
/// changes the tone rather than the level.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Drive {
    /// The control, 0 to 1: at 0 the voice is not shaped at all.
    amount: f64,
    /// d = 1 + 5·drive.
    gain: f64,
    /// tanh(d).
    ceiling: f64,
    /// The auto gain: the Distortion's at 20·log10(d) dB.
    makeup: f64,
}

impl Drive {
    fn new(amount: f32) -> Self {
        let amount = f64::from(amount);
        let gain = 1.0 + amount * DRIVE_RANGE;
        Self {
            amount,
            gain,
            ceiling: gain.tanh(),
            makeup: auto_gain(20.0 * gain.log10()),
        }
    }

    /// Side value `x` driven, `before` the driven value of the frame before, moved on to this
    /// one's: the mean of tanh between it and d·x, over tanh(d), times the auto gain. At drive
    /// 0, `x` itself (the voice as it was before the Drive, bit for bit), `before` following it
    /// (d is 1 there), so a drive turned up mid-note starts from the sound, not a stale value.
    fn shape(&self, x: f64, before: &mut f64) -> f64 {
        if self.amount == 0.0 {
            *before = x;
            return x;
        }
        let u = self.gain * x;
        let y = adaa_tanh(u, *before);
        *before = u;
        y / self.ceiling * self.makeup
    }
}

/// Each unison count's voices' pan gains (left, right), by count − 1 and voice.
type Pans = [[[f64; 2]; UNISON_MAX]; UNISON_MAX];

/// What the parameters set, worked out when one changes, so a frame only reads them.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Settings {
    sample_rate: u32,
    rate: f64,
    amp: Adsr,
    filter_env: Adsr,
    filter: FilterSettings,
    drive: Drive,
    lfo: LfoSettings,
    /// How many frames a glide takes: ⌊glide_ms/1000·rate + ½⌋ (0: no glide).
    glide_frames: u64,
    osc: [Osc; 2],
    /// The oscillators' weights in the mix: (1 − mix)·level1 and mix·level2.
    weights: [f64; 2],
    /// The unison control: how many voices a note started now plays.
    unison: usize,
    detune: f64,
    pans: Pans,
    noise: f64,
    vel_amp: f64,
    /// lin(volume), the output's gain.
    gain: f64,
}

impl Settings {
    fn new(values: &[f32; PARAMS.len()], sample_rate: u32) -> Self {
        let rate = f64::from(sample_rate);
        let mix = f64::from(values[OSC_MIX]);
        let spread = f64::from(values[SPREAD]);
        let pans = std::array::from_fn(|place| {
            let count = place + 1;
            std::array::from_fn(|k| {
                if k < count {
                    channel_pan_gains(unison_pan(spread, k, count) as f32).map(f64::from)
                } else {
                    [0.0; 2]
                }
            })
        });
        Self {
            sample_rate,
            rate,
            amp: Adsr::new(values, AMP_ATTACK, rate),
            filter_env: Adsr::new(values, FILTER_ATTACK, rate),
            filter: FilterSettings::new(values),
            drive: Drive::new(values[DRIVE]),
            lfo: LfoSettings {
                shape: LfoShape::of(values[LFO_SHAPE]),
                step: lfo_rate_hz(values[LFO_RATE]) / rate,
                pitch: f64::from(values[LFO_PITCH]),
            },
            glide_frames: (f64::from(values[GLIDE_MS]) / 1000.0 * rate + 0.5).floor() as u64,
            osc: [Osc::new(values, OSC1), Osc::new(values, OSC2)],
            weights: [
                (1.0 - mix) * f64::from(values[OSC1 + LEVEL]),
                mix * f64::from(values[OSC2 + LEVEL]),
            ],
            unison: values[UNISON] as usize,
            detune: f64::from(values[DETUNE]),
            pans,
            noise: f64::from(values[NOISE]),
            vel_amp: f64::from(values[VEL_AMP]),
            gain: lin(f64::from(values[VOLUME_DB])),
        }
    }

    /// What a phase moves by each frame at `pitch` semitones from A4: the frequency, kept
    /// below 0.45 of the rate, over the rate.
    fn step(&self, pitch: f64) -> f64 {
        let hz = (440.0 * exp2(pitch / 12.0)).min(MAX_FREQ_FRACTION * self.rate);
        hz / self.rate
    }
}

pub(super) struct Factory;

impl ProcessorFactory for Factory {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn shown(&self, index: usize, value: f32, _sample_rate: u32) -> f64 {
        match index {
            CUTOFF => cutoff_hz(value),
            LFO_RATE => lfo_rate_hz(value),
            index if TIMES.contains(&index) => time_ms(value),
            _ => f64::from(value),
        }
    }

    fn value_name(&self, index: usize, value: f32) -> Option<&'static str> {
        match index {
            OSC1_WAVE | OSC2_WAVE => named(&WAVES, index, value),
            UNISON => named(&UNISON_VOICES, index, value),
            LFO_SHAPE => named(&LFO_SHAPES, index, value),
            _ => None,
        }
    }

    fn groups(&self) -> &'static [ParamGroup] {
        &GROUPS
    }

    fn knob_label(&self, index: usize) -> &'static str {
        KNOB_LABELS.get(index).copied().unwrap_or("")
    }

    fn is_instrument(&self) -> bool {
        true
    }

    fn create(&self, sample_rate: u32, values: &[f32]) -> Box<dyn Processor> {
        Box::new(Synth::new(sample_rate, values))
    }
}

/// An envelope's settings: the coefficients of its three timed segments and its sustain level.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Adsr {
    attack: f64,
    decay: f64,
    sustain: f64,
    release: f64,
}

impl Adsr {
    /// The envelope of the parameter `values` at `rate` whose four controls (attack, decay,
    /// sustain, release, in that order) start at `first`.
    fn new(values: &[f32; PARAMS.len()], first: usize, rate: f64) -> Self {
        Self {
            attack: coefficient(K_ATTACK, values[first], rate),
            decay: coefficient(K_DECAY, values[first + 1], rate),
            sustain: f64::from(values[first + 2]),
            release: coefficient(K_DECAY, values[first + 3], rate),
        }
    }
}

/// Where an envelope is. `Idle` is a voice that is free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Stage {
    #[default]
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// An envelope's level and segment.
#[derive(Debug, Clone, Copy, Default)]
struct Envelope {
    level: f64,
    stage: Stage,
}

impl Envelope {
    /// Starts the attack from the level it has (0 for a voice that was free).
    fn attack(&mut self) {
        self.stage = Stage::Attack;
    }

    /// Starts the release, unless it is free.
    fn release(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    /// One frame on: the level this frame has.
    fn advance(&mut self, adsr: &Adsr) -> f64 {
        let level = self.level;
        let (level, stage) = match self.stage {
            Stage::Idle => (level, Stage::Idle),
            Stage::Attack => {
                let c = adsr.attack;
                let level = ATTACK_TARGET * (1.0 - c) + level * c;
                if level >= 1.0 {
                    (1.0, Stage::Decay)
                } else {
                    (level, Stage::Attack)
                }
            }
            Stage::Decay => {
                let (c, sustain) = (adsr.decay, adsr.sustain);
                let level = (sustain - UNDERSHOOT) * (1.0 - c) + level * c;
                if level <= sustain {
                    (sustain, Stage::Sustain)
                } else {
                    (level, Stage::Decay)
                }
            }
            Stage::Sustain => (adsr.sustain, Stage::Sustain),
            Stage::Release => {
                let c = adsr.release;
                let level = -UNDERSHOOT * (1.0 - c) + level * c;
                if level <= 0.0 {
                    (0.0, Stage::Idle)
                } else {
                    (level, Stage::Release)
                }
            }
        };
        self.level = level;
        self.stage = stage;
        level
    }
}

/// What moves every voice on a frame.
#[derive(Debug, Clone, Copy)]
struct Modulation {
    /// The pitch bend, in semitones.
    bend: f64,
    /// The LFO's value: 0 on a frame where nothing uses it.
    lfo: f64,
    /// lfo·(lfo_pitch + w), in semitones.
    vibrato: f64,
    /// Whether the vibrato's depth (lfo_pitch + w) is not 0, so every pitch moves each frame.
    vibrating: bool,
}

/// One voice: the note it plays, its oscillators, its noise, its filter and its envelopes.
#[derive(Debug, Clone, Copy)]
struct Voice {
    amp: Envelope,
    filter_env: Envelope,
    key: u8,
    velocity: f64,
    /// Which note it plays, counted from the last reset: the smallest is the oldest.
    started: u64,
    /// Frames until a pattern note's gate closes; `None` for a live note (it waits for its
    /// note-off) and once the gate has closed.
    gate_left: Option<u64>,
    /// Started by a live note-on, so a note-off of its key releases it.
    live: bool,
    /// How many unison voices it plays: the unison control at its start.
    unison: usize,
    /// Each oscillator's phase per unison voice, in [0, 1): from [`start_phase`] at reset,
    /// then free-running (a note starts from where they are).
    phases: [[f64; UNISON_MAX]; 2],
    /// What those phases move by each frame: the pitch over the rate.
    steps: [[f64; UNISON_MAX]; 2],
    /// The slot's noise generator, seeded with its slot + 1 at reset.
    noise: u32,
    /// Its low-pass: both sides' memory and the coefficients for `filter_semitones`.
    filter: Svf,
    /// The cutoff the filter's coefficients are for, in semitones from the control's (the
    /// input to its exp2); NaN when they must be worked out again.
    filter_semitones: f64,
    /// Per side, the Drive's driven value on the frame before: 0 at reset, then kept across
    /// notes (one frame of state, no ring to carry over, so a free voice keeps it too).
    driven: [f64; 2],
    /// Its glide: from `glide_from` semitones over `glide_frames` frames (0: none).
    glide_from: f64,
    glide_frames: u64,
    /// Frames it has played since its note started.
    age: u64,
    /// Its steps are worked out again on its next frame: its note started, or the bend, the
    /// wheel or a parameter moved.
    retune: bool,
}

impl Voice {
    /// The voice of slot `slot` as a reset leaves it: free, its phases at their start, its noise
    /// seeded, its filter's memory clear (its coefficients are worked out on its first frame).
    fn seeded(slot: usize, sample_rate: u32) -> Self {
        Self {
            amp: Envelope::default(),
            filter_env: Envelope::default(),
            key: 0,
            velocity: 0.0,
            started: 0,
            gate_left: None,
            live: false,
            unison: 1,
            phases: [std::array::from_fn(|k| start_phase(slot, k)); 2],
            steps: [[0.0; UNISON_MAX]; 2],
            noise: u32::try_from(slot).map_or(u32::MAX, |slot| slot + 1),
            filter: Svf::new(1000.0, 1.0, sample_rate),
            filter_semitones: f64::NAN,
            driven: [0.0; 2],
            glide_from: 0.0,
            glide_frames: 0,
            age: 0,
            retune: false,
        }
    }

    fn sounding(&self) -> bool {
        self.amp.stage != Stage::Idle
    }

    /// Whether its key counts as held for a glide: unreleased, its gate not closing on this
    /// frame (a note ending where another starts lets go first, as a plugin's note-offs go
    /// before its note-ons), and sounding since an earlier frame (so a chord struck from
    /// nothing does not glide into itself).
    fn held(&self) -> bool {
        matches!(
            self.amp.stage,
            Stage::Attack | Stage::Decay | Stage::Sustain
        ) && self.gate_left != Some(0)
            && self.age > 0
    }

    /// Its glide on this frame, in semitones.
    fn glide(&self) -> f64 {
        glide_offset(self.glide_from, self.age, self.glide_frames)
    }

    /// Whether its glide moves its pitch on this frame (up to the frame it reaches 0).
    fn glides(&self) -> bool {
        self.glide_frames > 0 && self.age <= self.glide_frames
    }

    /// Sets every unison voice's step for the note's key under `bend`, its glide and `vibrato`.
    fn tune(&mut self, settings: &Settings, bend: f64, vibrato: f64) {
        let glide = self.glide();
        for (osc, steps) in settings.osc.iter().zip(&mut self.steps) {
            for (k, step) in steps.iter_mut().enumerate().take(self.unison) {
                let detune = detune_cents(settings.detune, k, self.unison);
                *step = settings.step(pitch(self.key, bend, glide, vibrato, osc, detune));
            }
        }
    }

    /// This frame of the voice on each side, before the volume:
    /// filtered mix·env·(1 − vel_amp·(1 − velocity)).
    fn next(&mut self, settings: &Settings, modulation: &Modulation) -> [f64; 2] {
        if self.gate_left == Some(0) {
            self.gate_left = None;
            self.amp.release();
            self.filter_env.release();
        }
        let env = self.amp.advance(&settings.amp);
        let filter_env = self.filter_env.advance(&settings.filter_env);
        if self.retune || self.glides() || modulation.vibrating {
            self.tune(settings, modulation.bend, modulation.vibrato);
            self.retune = false;
        }
        let count = self.unison;
        let mut values = [[0.0; UNISON_MAX]; 2];
        let oscillators = settings.osc.iter().zip(&mut self.phases).zip(&self.steps);
        for (((osc, phases), steps), values) in oscillators.zip(&mut values) {
            let voices = phases.iter_mut().zip(steps).zip(values.iter_mut());
            for ((phase, &dt), value) in voices.take(count) {
                let t = *phase;
                *value = osc.value(t, dt);
                let next = t + dt;
                *phase = if next >= 1.0 { next - 1.0 } else { next };
            }
        }
        self.noise = xorshift32(self.noise);
        let noise = f64::from(self.noise) / 2_147_483_648.0 - 1.0;
        if let Some(left) = &mut self.gate_left {
            *left -= 1;
        }
        // The coefficients again only when the cutoff moved: exp2 and tan are skipped while it
        // holds still (in sustain, or open), for the same bits.
        let (lfo, velocity) = (modulation.lfo, self.velocity);
        let semitones = settings
            .filter
            .semitones(filter_env, self.key, lfo, velocity);
        if semitones.to_bits() != self.filter_semitones.to_bits() {
            let hz = settings.filter.hz(semitones);
            self.filter.set(hz, settings.filter.q, settings.sample_rate);
            self.filter_semitones = semitones;
        }
        let gain = 1.0 - settings.vel_amp * (1.0 - self.velocity);
        let [a, b] = settings.weights;
        let pans = &settings.pans[count - 1];
        let [first, second] = &values;
        let svf = &mut self.filter;
        let driven = &mut self.driven;
        let heard = [0, 1].map(|side| {
            let sum = pans
                .iter()
                .zip(first.iter().zip(second))
                .take(count)
                .fold(0.0, |sum, (pan, (one, two))| {
                    sum + pan[side] * (a * one + b * two)
                });
            let mixed = sum / count as f64 + settings.noise * noise;
            // The memory runs while the filter is open too, so it is ready when the cutoff
            // moves off 100%.
            let filtered = svf.process(side, mixed, Response::LowPass);
            let heard = if settings.filter.open {
                mixed
            } else {
                filtered
            };
            settings.drive.shape(heard, &mut driven[side]) * env * gain
        });
        self.age += 1;
        heard
    }
}

struct Synth {
    values: [f32; PARAMS.len()],
    settings: Settings,
    voices: [Voice; VOICES],
    /// The pitch bend, in semitones.
    bend: f64,
    /// The mod wheel, 0 to 1: w, the semitones of vibrato it adds.
    wheel: f64,
    /// The LFO's phase in [0, 1): 0 at reset, then free-running.
    lfo_phase: f64,
    /// Notes started since the last reset.
    starts: u64,
    /// The slot of the note started last, which a glide starts from.
    last: Option<usize>,
    /// The block's events with their frame and arrival, sorted; sized in `create`.
    events: Vec<(u32, usize, PortEvent)>,
}

impl Synth {
    /// A synth at `sample_rate` with `values` (one per parameter, in order; missing ones at
    /// their defaults), no voice sounding.
    fn new(sample_rate: u32, values: &[f32]) -> Self {
        let sample_rate = sample_rate.max(1);
        let defaults = PARAMS.map(|info| info.default);
        let mut synth = Self {
            values: defaults,
            settings: Settings::new(&defaults, sample_rate),
            voices: std::array::from_fn(|slot| Voice::seeded(slot, sample_rate)),
            bend: 0.0,
            wheel: 0.0,
            lfo_phase: 0.0,
            starts: 0,
            last: None,
            events: Vec::with_capacity(NOTES_PER_PLUGIN),
        };
        for (index, &value) in values.iter().enumerate() {
            synth.set_param(index, value);
        }
        synth
    }

    /// The glide a note on `key` starts with, (from, frames): from the pitch the note started
    /// last has now, less `key`, when the glide is on and a voice is held; none otherwise.
    fn glide_into(&self, key: u8) -> (f64, u64) {
        let frames = self.settings.glide_frames;
        match self.last {
            Some(last) if frames > 0 && self.voices.iter().any(Voice::held) => {
                let previous = &self.voices[last];
                let from = (f64::from(previous.key) + previous.glide()) - f64::from(key);
                (from, frames)
            }
            _ => (0.0, 0),
        }
    }

    /// Starts a note on `key`: in the free voice of lowest slot, else in the oldest's place.
    /// `gate` is a pattern note's length in frames; `None` for a live note.
    fn start(&mut self, key: u8, velocity: f32, gate: Option<u64>) {
        // The glide first, from the voices as they are before this note takes one.
        let (glide_from, glide_frames) = self.glide_into(key);
        let free = self.voices.iter().position(|voice| !voice.sounding());
        let slot = free
            .or_else(|| (0..VOICES).min_by_key(|&slot| self.voices[slot].started))
            .unwrap_or(0);
        let count = self.settings.unison;
        let voice = &mut self.voices[slot];
        if free.is_some() {
            // From rest: no old note's filter envelope or ring under the new one. A stolen
            // voice keeps both going, so the steal does not click.
            voice.filter_env = Envelope::default();
            voice.filter.reset();
            voice.filter_semitones = f64::NAN;
        }
        voice.key = key;
        voice.velocity = f64::from(velocity);
        voice.started = self.starts;
        voice.gate_left = gate;
        voice.live = gate.is_none();
        // Its phases go on from where its last note left them (free-running oscillators).
        voice.unison = count;
        voice.glide_from = glide_from;
        voice.glide_frames = glide_frames;
        voice.age = 0;
        voice.retune = true;
        voice.amp.attack();
        voice.filter_env.attack();
        self.last = Some(slot);
        self.starts += 1;
    }

    /// Every voice works its pitch out again on its next frame, under the bend, the wheel and
    /// the parameters as they are then.
    fn retune(&mut self) {
        for voice in &mut self.voices {
            voice.retune = true;
        }
    }

    fn apply(&mut self, event: PortEvent) {
        match event {
            PortEvent::Note {
                key,
                velocity,
                length,
                ..
            } => self.start(key, velocity, Some(length)),
            PortEvent::NoteOn { key, velocity, .. } => self.start(key, velocity, None),
            PortEvent::NoteOff { key, .. } => {
                for voice in &mut self.voices {
                    if voice.live && voice.key == key {
                        voice.amp.release();
                        voice.filter_env.release();
                    }
                }
            }
            PortEvent::Bend { value, .. } => {
                self.bend = f64::from(value) / BEND_STEPS_PER_SEMITONE;
                self.retune();
            }
            PortEvent::Wheel { value, .. } => {
                self.wheel = f64::from(value) / WHEEL_TOP;
                self.retune();
            }
        }
    }

    /// What moves every voice on this frame: the bend, and the LFO's value at its phase now
    /// with the vibrato it gives. A frame nothing listens to the LFO on takes 0 for it without
    /// computing it (the same bits: it is multiplied by a depth of 0).
    fn modulation(&self) -> Modulation {
        let depth = self.settings.lfo.pitch + self.wheel;
        let vibrating = depth != 0.0;
        let lfo = if vibrating || self.settings.filter.follows_lfo() {
            self.settings.lfo.shape.value(self.lfo_phase)
        } else {
            0.0
        };
        Modulation {
            bend: self.bend,
            lfo,
            vibrato: lfo * depth,
            vibrating,
        }
    }

    /// Renders `frames` with no event among them.
    fn render(&mut self, frames: &mut [[f32; 2]]) {
        for frame in frames {
            let modulation = self.modulation();
            let mut sum = [0.0; 2];
            for voice in &mut self.voices {
                if voice.sounding() {
                    let [left, right] = voice.next(&self.settings, &modulation);
                    sum[0] += left;
                    sum[1] += right;
                }
            }
            *frame = sum.map(|side| (side * self.settings.gain) as f32);
            let next = self.lfo_phase + self.settings.lfo.step;
            self.lfo_phase = if next >= 1.0 { next - 1.0 } else { next };
        }
    }
}

/// The frame an event falls on in its block.
fn frame_of(event: &PortEvent) -> u32 {
    match *event {
        PortEvent::Note { frame, .. }
        | PortEvent::NoteOn { frame, .. }
        | PortEvent::NoteOff { frame, .. }
        | PortEvent::Bend { frame, .. }
        | PortEvent::Wheel { frame, .. } => frame,
    }
}

impl Processor for Synth {
    fn process(&mut self, context: &ProcessContext<'_>, buffer: &mut [[f32; 2]]) {
        self.events.clear();
        let room = self.events.capacity();
        for (arrival, event) in context.events.iter().enumerate().take(room) {
            self.events.push((frame_of(event), arrival, *event));
        }
        self.events
            .sort_unstable_by_key(|&(frame, arrival, _)| (frame, arrival));
        let mut done = 0;
        for index in 0..self.events.len() {
            let (frame, _, event) = self.events[index];
            let at = usize::try_from(frame).map_or(buffer.len(), |at| at.min(buffer.len()));
            self.render(&mut buffer[done..at]);
            done = at;
            self.apply(event);
        }
        self.render(&mut buffer[done..]);
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(info) = PARAMS.get(index) else {
            return;
        };
        self.values[index] = info.clamp(value);
        self.settings = Settings::new(&self.values, self.settings.sample_rate);
        // A sounding voice's pitch follows the tuning, the detune and the LFO's depth, and its
        // filter's coefficients the Q (the same bits as before, when they have not moved).
        self.retune();
        for voice in &mut self.voices {
            voice.filter_semitones = f64::NAN;
        }
    }

    fn reset(&mut self) {
        let sample_rate = self.settings.sample_rate;
        self.voices = std::array::from_fn(|slot| Voice::seeded(slot, sample_rate));
        self.bend = 0.0;
        self.wheel = 0.0;
        self.lfo_phase = 0.0;
        self.starts = 0;
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::value::RawValue;

    use super::*;
    use crate::dsp::tests::{bits, CONTEXT};

    /// The default values with `changes` made, by key.
    fn values(changes: &[(&str, f32)]) -> Vec<f32> {
        PARAMS
            .iter()
            .map(|info| {
                changes
                    .iter()
                    .find(|(key, _)| *key == info.key)
                    .map_or(info.default, |&(_, value)| value)
            })
            .collect()
    }

    /// Plays `events` (absolute frames) over `frames` frames in blocks cut at `cuts` (then one
    /// block for the rest), each event handed to the block it falls in at its frame there.
    fn play(
        changes: &[(&str, f32)],
        events: &[PortEvent],
        frames: usize,
        cuts: &[usize],
    ) -> Vec<[f32; 2]> {
        let mut synth = Factory.create(44_100, &values(changes));
        let mut out = vec![[f32::NAN; 2]; frames];
        let mut start = 0;
        let ends = cuts
            .iter()
            .scan(0, |at, &cut| {
                *at += cut;
                Some(*at)
            })
            .filter(|&end| end < frames)
            .chain([frames]);
        let mut block_events = Vec::new();
        for end in ends {
            block_events.clear();
            for event in events {
                let frame = frame_of(event) as usize;
                if (start..end).contains(&frame) {
                    block_events.push(moved(*event, (frame - start) as u32));
                }
            }
            let context = ProcessContext {
                events: &block_events,
                ..CONTEXT
            };
            synth.process(&context, &mut out[start..end]);
            start = end;
        }
        out
    }

    /// `event` on frame `frame` instead.
    fn moved(event: PortEvent, frame: u32) -> PortEvent {
        match event {
            PortEvent::Note {
                key,
                velocity,
                length,
                ..
            } => PortEvent::Note {
                frame,
                key,
                velocity,
                length,
            },
            PortEvent::NoteOn { key, velocity, .. } => PortEvent::NoteOn {
                frame,
                key,
                velocity,
            },
            PortEvent::NoteOff { key, .. } => PortEvent::NoteOff { frame, key },
            PortEvent::Bend { value, .. } => PortEvent::Bend { frame, value },
            PortEvent::Wheel { value, .. } => PortEvent::Wheel { frame, value },
        }
    }

    fn note(frame: u32, key: u8, length: u64) -> PortEvent {
        PortEvent::Note {
            frame,
            key,
            velocity: 1.0,
            length,
        }
    }

    // Each named index is the parameter it names.
    #[test]
    fn the_named_indices_are_their_parameters() {
        let named = [
            (OSC1_WAVE, "osc1_wave"),
            (OSC1 + WIDTH, "osc1_width"),
            (OSC1 + OCTAVE, "osc1_octave"),
            (OSC1 + SEMITONES, "osc1_semitones"),
            (OSC1 + CENTS, "osc1_cents"),
            (OSC1 + LEVEL, "osc1_level"),
            (OSC2_WAVE, "osc2_wave"),
            (OSC2 + WIDTH, "osc2_width"),
            (OSC2 + OCTAVE, "osc2_octave"),
            (OSC2 + SEMITONES, "osc2_semitones"),
            (OSC2 + CENTS, "osc2_cents"),
            (OSC2 + LEVEL, "osc2_level"),
            (OSC_MIX, "osc_mix"),
            (UNISON, "unison"),
            (DETUNE, "detune"),
            (SPREAD, "spread"),
            (NOISE, "noise"),
            (CUTOFF, "cutoff"),
            (RESONANCE, "resonance"),
            (ENV_AMOUNT, "filter_env"),
            (KEY_TRACK, "key_track"),
            (DRIVE, "drive"),
            (FILTER_ATTACK, "filter_attack"),
            (FILTER_DECAY, "filter_decay"),
            (FILTER_RELEASE, "filter_release"),
            (AMP_ATTACK, "amp_attack"),
            (AMP_DECAY, "amp_decay"),
            (AMP_RELEASE, "amp_release"),
            (LFO_RATE, "lfo_rate"),
            (LFO_SHAPE, "lfo_shape"),
            (LFO_PITCH, "lfo_pitch"),
            (LFO_CUTOFF, "lfo_cutoff"),
            (GLIDE_MS, "glide_ms"),
            (VOLUME_DB, "volume_db"),
            (VEL_AMP, "vel_amp"),
            (VEL_CUTOFF, "vel_cutoff"),
        ];
        for (index, key) in named {
            assert_eq!(PARAMS[index].key, key);
        }
        // `Adsr::new` reads an envelope's four controls from its first on, in this order.
        for (first, prefix) in [(FILTER_ATTACK, "filter"), (AMP_ATTACK, "amp")] {
            let keys: Vec<&str> = PARAMS[first..first + 4]
                .iter()
                .map(|info| info.key)
                .collect();
            let want = ["attack", "decay", "sustain", "release"].map(|s| format!("{prefix}_{s}"));
            assert_eq!(keys, want, "{prefix}");
        }
    }

    // The window's eight headings, by hand: their names in order, each
    // run starting where the one before ended, from the first parameter to the last; the
    // parameters under each by key. Each knob's name is its label less what its heading says.
    #[test]
    fn the_parameters_show_under_eight_headings_covering_each_once() {
        let groups = Factory.groups();
        let names: Vec<&str> = groups.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            [
                "Osc 1",
                "Osc 2",
                "Unison",
                "Filter",
                "Filter env",
                "Amp env",
                "LFO",
                "Play"
            ]
        );
        let mut next = 0;
        for (name, run) in groups {
            assert_eq!(run.start, next, "{name} starts where the one before ended");
            assert!(run.end > run.start, "{name} holds a parameter");
            next = run.end;
        }
        assert_eq!(next, 38, "every parameter is under a heading");
        let keys = |index: usize| -> Vec<&str> {
            groups[index]
                .1
                .clone()
                .map(|param| PARAMS[param].key)
                .collect()
        };
        assert_eq!(
            keys(1).last(),
            Some(&"osc_mix"),
            "the mix with oscillator 2"
        );
        assert_eq!(keys(2), ["unison", "detune", "spread", "noise"]);
        assert_eq!(
            keys(3),
            ["cutoff", "resonance", "filter_env", "key_track", "drive"]
        );
        assert_eq!(keys(7), ["glide_ms", "volume_db", "vel_amp", "vel_cutoff"]);
        let shown: Vec<&str> = [0, 12, 13, 21, 22, 29, 32, 36]
            .map(|index| Factory.knob_label(index))
            .to_vec();
        assert_eq!(
            shown,
            [
                "Wave",
                "Mix",
                "Voices",
                "Drive",
                "Attack",
                "Release",
                "\u{2192} Pitch",
                "Vel\u{2192}level"
            ]
        );
        assert_eq!(Factory.knob_label(38), "", "no parameter, no name");
    }

    // An attack of 10 ms (its control at 0.25) from 0 reaches 1 at frame 441 ± 1 at
    // 44.1 kHz: 1.3·(1 − c^n) = 1 at c^n = 0.3/1.3, n = 441.
    #[test]
    fn a_10_ms_attack_reaches_1_at_frame_441_within_1() {
        let set: [f32; 38] = values(&[("amp_attack", 0.25)])
            .try_into()
            .expect("one per parameter");
        let adsr = Adsr::new(&set, AMP_ATTACK, 44_100.0);
        let mut env = Envelope::default();
        env.attack();
        let first = (0..1000)
            .position(|_| env.advance(&adsr) == 1.0)
            .expect("reaches 1");
        assert!((440..=442).contains(&first), "frame {first}");
        assert_eq!(env.stage, Stage::Decay);
    }

    // The release falls to exactly 0 and frees the voice; at sustain 100% the decay ends at
    // once on exactly 1.
    #[test]
    fn the_release_ends_on_exactly_0_and_the_voice_is_free() {
        let adsr = Adsr::new(&PARAMS.map(|info| info.default), AMP_ATTACK, 44_100.0);
        let mut env = Envelope::default();
        env.attack();
        while env.stage == Stage::Attack {
            env.advance(&adsr);
        }
        assert_eq!(env.advance(&adsr), 1.0, "the decay at sustain 1");
        assert_eq!(env.stage, Stage::Sustain);
        env.release();
        let frames = (0..100_000)
            .position(|_| env.advance(&adsr) == 0.0)
            .expect("reaches 0");
        assert_eq!(env.stage, Stage::Idle);
        // 100 ms from 1 down to within 0.0001 of −0.0001: about 4410 frames.
        assert!((4400..4420).contains(&frames), "{frames}");
    }

    // Seventeen notes held: the seventeenth takes the first one's voice; an eighteenth takes
    // the second's (the oldest), not the seventeenth's (the lowest slot).
    #[test]
    fn a_note_past_16_steals_the_oldest_voice() {
        let events: Vec<PortEvent> = (0..18)
            .map(|n| note(n * 10, 40 + n as u8, 10_000))
            .collect();
        let mut synth = Synth::new(44_100, &[]);
        let context = ProcessContext {
            events: &events,
            ..CONTEXT
        };
        synth.process(&context, &mut [[0.0; 2]; 200]);
        let keys: Vec<u8> = synth.voices.iter().map(|voice| voice.key).collect();
        let mut want: Vec<u8> = (40..56).collect();
        want[0] = 56;
        want[1] = 57;
        assert_eq!(keys, want);
    }

    // Any cut of the blocks, with notes, live notes, a bend and the mod wheel among them, gives
    // the same bits: events act on their own frame wherever the block starts.
    #[test]
    fn any_split_gives_the_same_bits() {
        let events = [
            note(0, 69, 3000),
            PortEvent::NoteOn {
                frame: 7,
                key: 72,
                velocity: 0.5,
            },
            note(511, 60, 20),
            PortEvent::Bend {
                frame: 512,
                value: 8191,
            },
            PortEvent::Wheel {
                frame: 600,
                value: 64,
            },
            PortEvent::NoteOff {
                frame: 999,
                key: 72,
            },
            PortEvent::Bend {
                frame: 1500,
                value: -8192,
            },
        ];
        let sine = [
            ("osc1_wave", 0.0),
            ("amp_attack", 0.2),
            ("amp_sustain", 0.6),
        ];
        // Both oscillators (a narrow square and a triangle an octave down), three unison
        // voices spread wide, and noise.
        let full = [
            ("osc1_wave", 3.0),
            ("osc1_width", 0.2),
            ("osc2_wave", 1.0),
            ("osc2_octave", -1.0),
            ("osc_mix", 0.4),
            ("unison", 3.0),
            ("detune", 30.0),
            ("spread", 0.8),
            ("noise", 0.3),
        ];
        // The filter moved by its envelope, key tracking, the saw LFO and velocity, the LFO on
        // the pitch with the wheel, and glide (the NoteOn and the note at 511 start over held
        // notes), so glides and the LFO's phase run across the cuts.
        let moving = [
            ("cutoff", 0.4),
            ("resonance", 0.7),
            ("filter_env", 0.6),
            ("key_track", 0.5),
            ("filter_attack", 0.2),
            ("filter_sustain", 0.3),
            ("lfo_cutoff", 0.3),
            ("vel_cutoff", 0.4),
            ("lfo_pitch", 1.5),
            ("lfo_shape", 3.0),
            ("lfo_rate", 0.9),
            ("glide_ms", 120.0),
        ];
        for changes in [&sine[..], &full, &moving] {
            let whole = play(changes, &events, 8000, &[]);
            for cuts in [
                &[1, 7, 512, 1, 999][..],
                &[64; 100],
                &[4096],
                &[3, 5, 7, 11, 13],
            ] {
                let split = play(changes, &events, 8000, cuts);
                assert_eq!(bits(&split), bits(&whole), "{changes:?} {cuts:?}");
            }
            assert!(whole.iter().any(|frame| frame[0] != 0.0), "it sounds");
        }
    }

    // PolyBLEP by hand: −1 at the step's first frame, tending to +1 just before it, 0 outside
    // a frame either side. The saw it corrects is 0 at the wrap from both sides (the naive saw
    // jumps from +1 to −1); the triangle is in phase with the sine (0, +1, 0, −1 at the
    // quarters); a square is ±1 away from its edges less its mean 2·width − 1 (DC-free: at
    // width 0.3, 1.4 high and −0.6 low).
    #[test]
    fn the_waves_by_hand() {
        let dt = 0.01;
        assert_eq!(blep(0.0, dt), -1.0);
        assert!((blep(1.0 - 1e-12, dt) - 1.0).abs() < 1e-9);
        for t in [dt, 0.5, 1.0 - dt] {
            assert_eq!(blep(t, dt), 0.0, "at {t}");
        }
        let osc = |wave, width: f64| Osc {
            wave,
            width,
            dc: 2.0 * width - 1.0,
            octave: 0.0,
            semitones: 0.0,
            cents: 0.0,
        };
        let saw = osc(Wave::Saw, 0.5);
        assert_eq!(saw.value(0.0, dt), 0.0);
        assert!(saw.value(1.0 - 1e-12, dt).abs() < 1e-9);
        assert_eq!(saw.value(0.25, dt), -0.5);
        let triangle = osc(Wave::Triangle, 0.5);
        let quarters = [0.0, 0.25, 0.5, 0.75].map(|t| triangle.value(t, dt));
        assert_eq!(quarters, [0.0, 1.0, 0.0, -1.0]);
        let square = osc(Wave::Square, 0.3);
        assert_eq!(square.value(0.15, dt), 1.0 + 0.4);
        assert_eq!(square.value(0.65, dt), -1.0 + 0.4);
        // On its falling edge exactly (t = width, low there: t < width is high) it is the
        // edge's middle, 0, as the saw is at its wrap (less its mean); high there it would be 2.
        assert_eq!(square.value(0.3, dt), 0.4);
        let half = osc(Wave::Square, 0.5);
        assert_eq!(
            [0.25, 0.5, 0.75].map(|t| half.value(t, dt)),
            [1.0, 0.0, -1.0]
        );
        assert_eq!(Wave::of(0.0), Wave::Sine);
        assert_eq!(Wave::of(3.0), Wave::Square);
    }

    // Ruling 26 by hand: at width 0.5 the square is the one before it to the bit (2·0.5 − 1 is
    // +0.0, and x − 0.0 is x), at every phase and step, edges included; at width 0.2 a period of
    // it averages 0 (stepped 1/1000, so the mean is the edges' share alone), where before it
    // averaged 2·0.2 − 1 = −0.6.
    #[test]
    fn the_pulse_is_dc_free_and_unchanged_at_width_half() {
        let square = |width: f32| {
            Osc::new(
                &values(&[("osc1_wave", 3.0), ("osc1_width", width)])
                    .try_into()
                    .expect("one per parameter"),
                OSC1,
            )
        };
        let half = square(0.5);
        assert_eq!(half.dc.to_bits(), 0.0f64.to_bits());
        for dt in [1e-3, 0.0798, 0.3] {
            for step in 0..1000 {
                let t = f64::from(step) / 1000.0;
                let before = (if t < 0.5 { 1.0 } else { -1.0 }) + blep(t, dt)
                    - blep(frac(t + 1.0 - 0.5), dt);
                assert_eq!(half.value(t, dt).to_bits(), before.to_bits(), "{t} {dt}");
            }
        }
        let narrow = square(0.2);
        let dt = 1e-3;
        let mean = (0..1000)
            .map(|step| narrow.value(f64::from(step) * dt, dt))
            .sum::<f64>()
            / 1000.0;
        assert!(mean.abs() < 1e-9, "{mean}");
        assert!((narrow.dc - -0.6).abs() < 1e-7, "{}", narrow.dc);
    }

    // Ruling 27 by hand: a reset leaves slot s's unison voice k at frac(s·φ + k/3) on both
    // oscillators (slot 0 at 0, 1/3, 2/3; slot 1 at φ, φ + 1/3, φ + 2/3 − 1; slot 15 at
    // 15·φ − 9), and a note does not move them: the same note twice on slot 0, the first freed
    // before the second starts, sounds different the second time (it goes on from the phase the
    // first left), where every note used to start from k/U and played the same frames again.
    #[test]
    fn a_reset_spreads_the_phases_and_a_note_goes_on_from_them() {
        let synth = Synth::new(44_100, &[]);
        let start = |slot: usize| synth.voices[slot].phases;
        assert_eq!(start(0), [[0.0, 1.0 / 3.0, 2.0 / 3.0]; 2]);
        let one = [
            0.618_033_988_749_894_9,
            0.951_367_322_083_228_2,
            0.284_700_655_416_561_4,
        ];
        assert_eq!(start(1), [one; 2]);
        assert_eq!(start(15)[1][0], 0.270_509_831_248_423_54);
        let events = [note(0, 60, 1000), note(10_000, 60, 1000)];
        let out = play(&[], &events, 11_000, &[]);
        let (first, second) = (&out[..1000], &out[10_000..]);
        assert!(first.iter().any(|frame| frame[0] != 0.0), "it sounds");
        assert!(
            out[9000..10_000].iter().all(|frame| *frame == [0.0; 2]),
            "freed"
        );
        assert_ne!(
            bits(first),
            bits(second),
            "the second goes on from the first's phases"
        );
    }

    // Rulings 28 and 28b by hand: d = 1 + 5·drive, 6 at the top; a held value x settles on
    // tanh(d·x)/tanh(d) (the mean of tanh over a step of none is tanh itself) times the auto
    // gain, the Distortion's 10^(−(D/(1 + D/40))/20) at D = 20·log10(d) dB (15.56 dB of drive at
    // the top: −11.2 dB, 0.2754). So a full-scale 1 settles on the auto gain itself at any drive,
    // and 0.5 at the top on tanh(3)/tanh(6) of it; at drive 0 a value passes bit for bit (−0.0
    // too) and the memory follows it.
    #[test]
    fn the_drive_by_hand() {
        let by_hand = |d: f64| {
            let db = 20.0 * d.log10();
            10f64.powf(-(db / (1.0 + db / 40.0)) / 20.0)
        };
        let full = Drive::new(1.0);
        assert_eq!((full.gain, full.ceiling), (6.0, 6f64.tanh()));
        assert!(
            (full.makeup - by_hand(6.0)).abs() < 1e-15,
            "{}",
            full.makeup
        );
        assert!((full.makeup - 0.2754).abs() < 1e-4, "{}", full.makeup);
        let mut before = 0.0;
        let settled = (0..4).fold(0.0, |_, _| full.shape(0.5, &mut before));
        assert_eq!(settled, 3f64.tanh() / 6f64.tanh() * full.makeup);
        for amount in [0.1, 0.5, 1.0] {
            let drive = Drive::new(amount);
            let mut before = drive.gain;
            assert_eq!(drive.shape(1.0, &mut before), drive.makeup, "{amount}");
            let want = by_hand(drive.gain);
            assert!((drive.makeup - want).abs() < 1e-15, "{amount}: {want}");
        }
        let off = Drive::new(0.0);
        let mut before = 0.25;
        for x in [0.3, -0.0, -1.7] {
            assert_eq!(off.shape(x, &mut before).to_bits(), x.to_bits());
            assert_eq!(before.to_bits(), x.to_bits());
        }
    }

    // xorshift32 by hand: from 1, 1 ^ 1<<13 = 8193, ^ >>17 leaves it, ^ <<5 = 8193 ^ 262176 =
    // 270369; from 2, twice that, 540738. A reset seeds slot s with s + 1.
    #[test]
    fn the_noise_steps_by_xorshift32_from_each_slots_seed() {
        assert_eq!(xorshift32(1), 270_369);
        assert_eq!(xorshift32(2), 540_738);
        let synth = Synth::new(44_100, &[]);
        let seeds: Vec<u32> = synth.voices.iter().map(|voice| voice.noise).collect();
        assert_eq!(seeds, (1..=16).collect::<Vec<u32>>());
    }

    // Unison by hand at detune 50 and spread 1: −25, 0 and +25 cents, panned hard left,
    // centred and hard right by the channel pan law (√2 on its side, 0 on the other, f32).
    #[test]
    fn three_unison_voices_detune_and_spread_by_hand() {
        let detunes = [0, 1, 2].map(|k| detune_cents(50.0, k, 3));
        assert_eq!(detunes, [-25.0, 0.0, 25.0]);
        assert_eq!([0, 1, 2].map(|k| unison_pan(1.0, k, 3)), [-1.0, 0.0, 1.0]);
        assert_eq!(detune_cents(50.0, 0, 1), 0.0);
        assert_eq!(unison_pan(1.0, 0, 1), 0.0);
        let changed: [f32; 38] = values(&[("unison", 3.0), ("spread", 1.0)])
            .try_into()
            .expect("one per parameter");
        let settings = Settings::new(&changed, 44_100);
        let root2 = f64::from(std::f32::consts::SQRT_2);
        assert_eq!(settings.pans[2][0], [root2, 0.0]);
        assert_eq!(settings.pans[2][1], [1.0, 1.0]);
        assert_eq!(settings.pans[2][2], [0.0, root2]);
        assert_eq!(settings.pans[0][0], [1.0, 1.0]);
        assert_eq!(settings.unison, 3);
    }

    // Reset frees every voice, the bend and the wheel, and puts the LFO back at phase 0: the
    // same notes after a reset give the same bits.
    #[test]
    fn a_reset_plays_as_new() {
        let changes = [("osc1_wave", 0.0), ("lfo_pitch", 1.0)];
        let mut synth = Factory.create(44_100, &values(&changes));
        let events = [
            PortEvent::Bend {
                frame: 0,
                value: 4000,
            },
            PortEvent::Wheel {
                frame: 0,
                value: 90,
            },
            note(3, 69, 500),
        ];
        let context = ProcessContext {
            events: &events,
            ..CONTEXT
        };
        let mut first = vec![[0.0; 2]; 1000];
        synth.process(&context, &mut first);
        synth.reset();
        let mut again = vec![[0.0; 2]; 1000];
        synth.process(
            &ProcessContext {
                events: &events[2..],
                ..CONTEXT
            },
            &mut again,
        );
        let mut fresh = vec![[0.0; 2]; 1000];
        Factory.create(44_100, &values(&changes)).process(
            &ProcessContext {
                events: &events[2..],
                ..CONTEXT
            },
            &mut fresh,
        );
        assert_eq!(bits(&again), bits(&fresh));
        assert_ne!(bits(&first), bits(&fresh), "the bend moved the first");
    }

    // The log controls by hand: 1 ms, 10 ms, 100 ms, 1 s and 10 s; 20 Hz to 20 kHz; 0.01 Hz to
    // 20 Hz (2.99 Hz at 0.75); the choices by name.
    #[test]
    fn the_controls_show_their_units_and_choices_their_names() {
        let close = |a: f64, b: f64| (a / b - 1.0).abs() < 1e-12;
        for (value, ms) in [
            (0.0, 1.0),
            (0.25, 10.0),
            (0.5, 100.0),
            (0.75, 1000.0),
            (1.0, 1e4),
        ] {
            assert!(
                close(Factory.shown(AMP_RELEASE, value, 44_100), ms),
                "{value}"
            );
        }
        assert!(close(Factory.shown(CUTOFF, 0.0, 44_100), 20.0));
        assert!(close(Factory.shown(CUTOFF, 1.0, 44_100), 20_000.0));
        assert!(close(Factory.shown(LFO_RATE, 1.0, 44_100), 20.0));
        assert!((Factory.shown(LFO_RATE, 0.75, 44_100) - 2.99).abs() < 0.005);
        assert_eq!(Factory.shown(VOLUME_DB, -6.0, 44_100), -6.0);
        assert_eq!(Factory.value_name(OSC1_WAVE, 0.0), Some("Sine"));
        assert_eq!(Factory.value_name(OSC2_WAVE, 3.0), Some("Square"));
        assert_eq!(Factory.value_name(UNISON, 1.0), Some("1 voice"));
        assert_eq!(Factory.value_name(UNISON, 3.0), Some("3 voices"));
        assert_eq!(Factory.value_name(LFO_SHAPE, 2.0), Some("Square"));
        assert_eq!(Factory.value_name(VOLUME_DB, 0.0), None);
        assert!(Factory.is_instrument());
    }

    /// p33_laws.json (tests/fixtures/gen_p33_expected.py): the cutoff's and the pitch's laws
    /// pinned to the bit, from inputs on which each other order the generator lists rounds
    /// differently.
    const LAWS: &str = include_str!("../../../../tests/fixtures/p33_laws.json");

    type Row = BTreeMap<&'static str, &'static RawValue>;

    fn law(name: &str) -> Vec<Row> {
        let mut laws: BTreeMap<&str, Vec<Row>> = serde_json::from_str(LAWS).expect("parses");
        let rows = laws.remove(name).expect("a law");
        assert!(rows.len() >= 4, "{name}: {} rows", rows.len());
        rows
    }

    /// Field `key` of `row`, read from the text the generator wrote by std's correctly rounded
    /// parser (serde_json's own reads some 17-digit numbers an ulp off).
    fn read<T: std::str::FromStr>(row: &Row, key: &str) -> T
    where
        T::Err: std::fmt::Debug,
    {
        row[key].get().parse().expect(key)
    }

    // The cutoff's modulation, its four terms summed left to right, and its frequency, to the
    // bit. Why pinned here: another order moves the f64 by an ulp, which the f32 output hides.
    #[test]
    fn the_cutoff_law_is_the_generators_to_the_bit() {
        for (n, row) in law("cutoff").iter().enumerate() {
            let changes = [
                "cutoff",
                "filter_env",
                "key_track",
                "lfo_cutoff",
                "vel_cutoff",
            ]
            .map(|key| (key, read::<f32>(row, key)));
            let set: [f32; 38] = values(&changes).try_into().expect("one per parameter");
            let filter = Settings::new(&set, 44_100).filter;
            assert!(!filter.open, "row {n}");
            let (fenv, key, lfo) = (read(row, "fenv"), read(row, "key"), read(row, "lfo"));
            let velocity = f64::from(read::<f32>(row, "velocity"));
            let semitones: f64 = read(row, "semitones");
            let hz: f64 = read(row, "hz");
            let got = filter.semitones(fenv, key, lfo, velocity);
            assert_eq!(
                got.to_bits(),
                semitones.to_bits(),
                "row {n}: {got} not {semitones}"
            );
            let got = filter.hz(filter.semitones(fenv, key, lfo, velocity));
            assert_eq!(got.to_bits(), hz.to_bits(), "row {n}: {got} Hz not {hz}");
        }
    }

    // The glide's offset and the pitch's terms added one at a time, to the bit.
    #[test]
    fn the_pitch_and_glide_laws_are_the_generators_to_the_bit() {
        for (n, row) in law("pitch").iter().enumerate() {
            let glide = glide_offset(
                read(row, "glide_from"),
                read(row, "m"),
                read(row, "glide_frames"),
            );
            let want: f64 = read(row, "glide");
            assert_eq!(glide.to_bits(), want.to_bits(), "row {n}: glide {glide}");
            let osc = Osc {
                wave: Wave::Sine,
                width: 0.5,
                dc: 0.0,
                octave: read(row, "octave"),
                semitones: read(row, "semitones"),
                cents: f64::from(read::<f32>(row, "cents")),
            };
            let bend = f64::from(read::<i16>(row, "bend")) / BEND_STEPS_PER_SEMITONE;
            let vibrato = read(row, "vibrato");
            let got = pitch(
                read(row, "key"),
                bend,
                glide,
                vibrato,
                &osc,
                read(row, "detune"),
            );
            let want: f64 = read(row, "pitch");
            assert_eq!(got.to_bits(), want.to_bits(), "row {n}: {got} not {want}");
        }
    }

    // The LFO's shapes by hand at the quarters: the sine and triangle 0, 1, 0, −1 (in phase);
    // the square 1 for the first half; the saw −1 up to 1.
    #[test]
    fn the_lfo_shapes_by_hand() {
        let quarters = |shape: LfoShape| [0.0, 0.25, 0.5, 0.75].map(|t| shape.value(t));
        assert_eq!(quarters(LfoShape::Triangle), [0.0, 1.0, 0.0, -1.0]);
        assert_eq!(quarters(LfoShape::Square), [1.0, 1.0, -1.0, -1.0]);
        assert_eq!(quarters(LfoShape::Saw), [-1.0, -0.5, 0.0, 0.5]);
        let sine = quarters(LfoShape::Sine);
        for (got, want) in sine.into_iter().zip([0.0, 1.0, 0.0, -1.0]) {
            assert!((got - want).abs() < 1e-15, "{sine:?}");
        }
        assert_eq!(LfoShape::of(0.0), LfoShape::Sine);
        assert_eq!(LfoShape::of(2.0), LfoShape::Square);
        assert_eq!(LfoShape::of(3.0), LfoShape::Saw);
    }

    /// A synth with `changes` after `events` over `frames` frames, in one block.
    fn played(changes: &[(&str, f32)], events: &[PortEvent], frames: usize) -> Synth {
        let mut synth = Synth::new(44_100, &values(changes));
        let context = ProcessContext { events, ..CONTEXT };
        synth.process(&context, &mut vec![[0.0; 2]; frames]);
        synth
    }

    /// Each voice's glide (from, frames), by slot.
    fn glides(synth: &Synth, slots: usize) -> Vec<(f64, u64)> {
        synth.voices[..slots]
            .iter()
            .map(|voice| (voice.glide_from, voice.glide_frames))
            .collect()
    }

    // Glide by hand at 200 ms (⌊0.2·44100 + ½⌋ = 8820 frames): a chord struck from nothing
    // does not glide into itself; a note started over it glides from the note started last
    // (G4 to C5: −5 semitones); one started half way through that glide starts from where it
    // has got to (72 − 2.5, so −6.5 to E5); a note whose gate closes on the frame another starts
    // is let go first (no glide), one a frame longer is held (−12); with the glide off, nothing.
    #[test]
    fn a_note_glides_only_from_a_held_one() {
        let on = [("glide_ms", 200.0)];
        let chord = [note(0, 60, 9000), note(0, 64, 9000), note(0, 67, 9000)];
        let then = [&chord[..], &[note(500, 72, 9000), note(4910, 76, 9000)]].concat();
        let synth = played(&on, &then, 5000);
        let want = vec![(0.0, 0), (0.0, 0), (0.0, 0), (-5.0, 8820), (-6.5, 8820)];
        assert_eq!(glides(&synth, 5), want);
        let touching = played(&on, &[note(0, 60, 100), note(100, 72, 100)], 200);
        assert_eq!(glides(&touching, 2), [(0.0, 0), (0.0, 0)]);
        let overlapping = played(&on, &[note(0, 60, 101), note(100, 72, 100)], 200);
        assert_eq!(glides(&overlapping, 2), [(0.0, 0), (-12.0, 8820)]);
        let off = played(&[], &then, 5000);
        assert!(glides(&off, 5).iter().all(|&glide| glide == (0.0, 0)));
    }

    // A free voice starts its filter from rest: a note on a slot whose last note was freed
    // while its filter envelope (a 10 s release) was still high and its Q 20 filter ringing
    // plays the bits of the same note on a new synth whose slot's phases are put where the
    // used one's are (they run free). (No noise and no LFO, which run on.)
    #[test]
    fn a_free_voice_starts_its_filter_from_rest() {
        let changes = [
            ("cutoff", 0.3),
            ("resonance", 1.0),
            ("filter_env", 0.5),
            ("filter_sustain", 1.0),
            ("filter_release", 1.0),
            ("amp_release", 0.0),
        ];
        let mut used = Synth::new(44_100, &values(&changes));
        let mut fresh = Synth::new(44_100, &values(&changes));
        let mut a = vec![[0.0; 2]; 4000];
        let mut b = vec![[0.0; 2]; 4000];
        let first = [note(0, 60, 1000)];
        let context = |events| ProcessContext { events, ..CONTEXT };
        used.process(&context(&first), &mut a[..3000]);
        fresh.process(&context(&[]), &mut b[..3000]);
        assert!(
            a[1500..3000].iter().all(|frame| *frame == [0.0; 2]),
            "the first is free"
        );
        assert_ne!(fresh.voices[0].phases, used.voices[0].phases, "it ran");
        fresh.voices[0].phases = used.voices[0].phases;
        let second = [note(0, 72, 1000)];
        used.process(&context(&second), &mut a[3000..]);
        fresh.process(&context(&second), &mut b[3000..]);
        assert_eq!(bits(&a[3000..]), bits(&b[3000..]));
    }

    // While open the filter's memory runs all the same: a saw whose cutoff moves off 100%
    // mid-note sounds, over the next 64 frames, within 0.01 of the same note filtered from the
    // start, where a filter starting from silence is off by more than 0.1 (its first frame
    // passes 70% of the input at 18.6 kHz).
    #[test]
    fn an_open_filter_keeps_listening() {
        let events = [note(0, 57, 10_000)];
        let context = ProcessContext {
            events: &events,
            ..CONTEXT
        };
        let lower = 0.99;
        let mut filtered = vec![[0.0; 2]; 2064];
        Synth::new(44_100, &values(&[("cutoff", lower)])).process(&context, &mut filtered);
        let off = |silenced: bool| {
            let mut synth = Synth::new(44_100, &[]);
            let mut out = vec![[0.0; 2]; 2064];
            synth.process(&context, &mut out[..2000]);
            let ran = synth.voices[0].filter_semitones;
            assert_eq!(
                ran, 0.0,
                "the open filter's memory runs at the control's cutoff"
            );
            if silenced {
                synth.voices[0].filter.reset();
            }
            synth.set_param(CUTOFF, lower);
            synth.process(
                &ProcessContext {
                    events: &[],
                    ..CONTEXT
                },
                &mut out[2000..],
            );
            out[2000..]
                .iter()
                .zip(&filtered[2000..])
                .map(|(got, want)| (got[0] - want[0]).abs())
                .fold(0.0f32, f32::max)
        };
        let listening = off(false);
        let silent = off(true);
        assert!(listening < 0.01, "{listening}");
        assert!(silent > 0.1, "{silent}");
    }
}
