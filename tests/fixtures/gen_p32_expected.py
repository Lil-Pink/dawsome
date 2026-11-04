"""Known answers for the seven native effects of Phase A (§3, §4 A.1-A.9):
chorus, flanger, phaser, distortion, filter, gate and stereo width, each in insert 1 of a
project whose one sampler channel plays an input file at tick 0, rendered by `dawsome render`
to 32-bit float.

Expected values come from the spec's formulas restated below, never from engine code (the Rust
did not exist yet: this file is the ground truth the Rust follows). gen_p11_expected.py's
`f32`, `exp2`, `hermite`, `LOG2_10`, `LN_2` and `jf` are imported, as gen_p24_expected.py does.
Every input is authored here: the sounds, the projects and p32_expected.json.

Notation: all arithmetic is f64 unless written f32(...); a parameter is the f32 the project file
holds (the effect gets f32 values), widened to f64 where an f64 formula uses it; sr = 44100.0;
TAU = 2 pi (std TAU, Python 2.0 * math.pi); PI = std PI; TINY = 1e-30.
Products and sums are evaluated LEFT TO RIGHT exactly as written in the comments below (each
"order fixed" note is one the spec left open; the Rust follows this file).

The render path (what lies between the effect and the file, read from render.rs and laws.rs):
  sampler   a step hit (length 0) on key 60 at velocity 1.0, volume 1, centered: the sampler's
            default gate is one_shot (project/sound.rs:51), so the whole file plays; key 60
            reads whole frames (sampler.rs `whole_frames`); the voice adds value * (1.0 * 1.0)
            (mono: times channel_pan_gains(0) = [1.0, 1.0], laws.rs test) into insert 1's bus,
            zeroed when first fed: the bus holds the file's frames exactly, then 0.0
  insert 1  the effect in slot 0 processes the bus in place; fader 0.8 gives fader_gain = 1.0
            exactly (laws.rs test) and apply_fader skips unity; pan 0 is skipped by apply_pan
  send      the master's bus (zeroed when first fed) += insert 1 * 1.0: out = 0.0 + y, which
            is y except that -0.0 becomes +0.0 (`bus` below)
  master    no effects (a loaded file has only what it lists; the limiter is a new document's
            only), fader 0.8 = unity, pan 0: copied to the output; 32-bit float is never
            dithered (cli main.rs `Encoding::for_path`), written as is
  So the render is the effect's output with negative zeros made positive. The five "dry"
  projects (no effect) prove that path through the real door: their hash is the input's.

The shared parts (§3):
  blend      dsp.rs's: mix == 0 -> dry, mix == 1 -> wet (the bits), else keep = f32(1 - mix),
             f32(f32(keep * dry) + f32(mix * wet))
  blend_power dsp.rs's equal-power mix (chorus, flanger, phaser): mix == 0
             -> dry, mix == 1 -> wet (the bits), else f32((sqrt(1 - mix) * dry) + (sqrt(mix) *
             wet)) in f64, mix the f32 widened. sqrt is correctly rounded (IEEE 754), so the gains
             are the same bits everywhere; the squares of the two gains sum to 1 (equal power), and
             at 50% each is sqrt(0.5), the same as cos/sin of 45 degrees
  snap/64    0.0 when |v| < 1e-30 (an f32 compared as f64; an f64)
  lin(dB)    exp2(dB / 20 * LOG2_10)  [(dB / 20) * LOG2_10; f64, not rounded to f32]
  P1 line    L = ceil(max_ms * sr / 1000) + 4 [(max_ms * sr) / 1000]; read(s, tau): tau clamped
             to [2, L - 3], i = floor(tau), f = tau - i, y0..y3 = v[n-i+1], v[n-i], v[n-i-1],
             v[n-i-2] (0 before the first write), gen_p11's hermite(y0, y1, y2, y3, f); each
             frame reads first, then writes
  P2 LFO     phase = 0 at reset; step = TAU * rate / sr [(TAU * rate) / sr]; frame n uses phase,
             then phase += step, if phase >= TAU: phase -= TAU
  P3 SVF     fc clamped to [20, 0.45 * sr]; g = tan(PI * fc / sr) [(PI * fc) / sr]; k = 1 / Q;
             a1 = 1 / (1 + g * (g + k)); a2 = g * a1; a3 = g * a2; per sample v3 = v0 - ic2;
             v1 = a1 * ic1 + a2 * v3; v2 = (ic2 + a2 * ic1) + a3 * v3; ic1 = snap64(2 v1 - ic1);
             ic2 = snap64(2 v2 - ic2); LP v2, HP (v0 - k v1) - v2, BP k v1, notch v0 - k v1
  P4 follower coefficient(ms) = 0 at ms = 0, else exp2(-LOG2_E / ((ms / 1000) * sr));
             p > level: level = a * level + (1 - a) * p, else r * level + (1 - r) * p;
             level = snap64(level)

The effects (§4; the parameter tables are EFFECTS below; the chorus's sum, high-pass and
defaults, the flanger's and phaser's Spread, the equal-power mix, Auto gain, the filter's Slope
and level compensation, the gate's RMS detector and Bass mono came from a comparison of the
effects with professional ones):
  chorus     tau = (15 + 5 * depth * sin(phase + v * TAU / 3 + s * TAU / 4)) * sr / 1000, i.e.
             ((15 + (5 * depth) * sin((phase + (v * TAU) / 3) + (s * TAU) / 4)) * sr) / 1000,
             r_vs = read(s, tau); the voices summed over sqrt 3, not averaged, so
             three uncorrelated voices keep the input's power, and the sum high-passed (P3
             high-pass at 150 Hz, Q 1/sqrt 2, its own memory per side) so the lows stay dry:
             wet_s = f32(HP_s(((r_0s + r_1s) + r_2s) / SQRT_3)), SQRT_3 = sqrt(3.0) (std has
             no constant; 1.7320508075688772); write x; LFO; blend_power. Defaults
             0.5 Hz, depth 30%: 1.5 ms of sweep, about 8 cents of wobble. Line max 20 ms
             (L 886)
  flanger    a Spread (0..180 degrees, default 90) puts the right side's sweep that
             far ahead: offset = (spread * PI) / 180, angle_0 = phase + 0.0 (= phase), angle_1 =
             phase + offset; per side tau_s = max(2, ((delay_ms * sr) / 1000) * (1 - (depth *
             (1 - sin(angle_s))) / 2)), d = read(s, tau_s), write snap(f32(x + feedback * d)),
             wet = d; LFO; blend_power. Line max 10 ms (L 445)
  phaser     center_hz = 100 * exp2(center * (2 + LOG2_10)); Spread as the
             flanger's, per side f_s = clamp(center_hz * exp2((depth / 2) * sin(angle_s)), 20,
             0.45 * sr); t_s = tan((PI * f_s) / sr); a_s = (t_s - 1) / (t_s + 1) (both sides'
             a first, then the chains, left then right); per side u = x + feedback * last;
             stage j: y = ((a_s * u) + x1_j) - (a_s * y1_j), x1_j = snap64(u), y1_j = snap64(y),
             u = y; last = snap64(u); wet = f32(u); LFO; blend_power.
             N = 4 + 2 * stages
  distortion u = lin(drive_db) * x; F1(u) = (|u| + log1p(exp(-2 |u|))) - LN_2; d = u - u1:
             |d| < 1e-5: y = tanh((u + u1) / 2), else (F1(u) - F1(u1)) / d; u1 = u;
             Auto gain (on by default) scales the shaper's output by makeup =
             lin(-(drive_db / (1 + drive_db / 40))) (the fit of the measured loudness
             rise), 1.0 when off: y = y * makeup; then
             fc = 1000 * exp2(tone * (1 + LOG2_10)); c = 1 - exp2(-(((TAU * fc) / sr) * LOG2_E));
             lp = snap64(lp + c * (y - lp)); wet = f32(lin(output_db) * lp); blend
  filter     fc = 20 * exp2(cutoff * (3 * LOG2_10)) [the constant 3 * LOG2_10 first, decision
             6], Q = 0.5 * exp2(resonance * (2 + LOG2_10)); Slope 12 dB (one P3
             stage at Q) or 24 dB (two P3 stages in series, the same cutoff and the same Q on
             both, sqrt(Q) each, so the pair's gain at the cutoff is Q as the 12 dB stage's is);
             the low- and high-pass, whose resonance rises above the input, are scaled above the
             default Q by comp = min(1, sqrt(Q0 / Q)), Q0 = filter_q(f32(0.1)) = 0.7231 (the
             resonance control's default through the same formula, so comp is exactly 1 at the
             default and below it; sqrt(Q0 / 20) = -14.4 dB at 100%), the band-pass and notch,
             which never rise above the input, by 1.0: out = f32(y * comp)
  gate       theta_o = lin(T), theta_c = lin(T - 3); the detector is the RMS of the
             louder side, a one-pole mean of its square with one 10 ms time both ways:
             ms = snap64(c * ms + (1 - c) * max(l * l, r * r)), c = coefficient(10), e =
             sqrt(ms) (correctly rounded, so still the exact tier); the default threshold
             stays -40 dB (the RMS reads hiss as it is, so -40 already closes on -50 dBFS hiss,
             with no loss on the hits). hold_frames = floor((hold_ms / 1000) * sr + 0.5); g = floor =
             lin(range_db); per frame
             if e >= theta_o or (open and e >= theta_c): open, hold_left =
             hold_frames; elif open and hold_left > 0: hold_left -= 1; else: closed;
             target = 1 if open else floor; c = coefficient(attack_ms) if target > g else
             coefficient(release_ms); g = c * g + (1 - c) * target; out = (f32(l g), f32(r g))
  width      bass mono off (its control 0, the default): w == 1: the input's bits; else
             M = (l + r) / 2, S = ((l - r) / 2) * w, out = (f32(M + S), f32(M - S));
             bass mono on (control v > 0): fc = 50 * exp2(v * LOG2_10) (50 to
             500 Hz), the sides high-passed before the width, M = (l + r) / 2,
             S = HP((l - r) / 2) * w (P3 high-pass at fc, Q 1/sqrt 2, side 0's memory), out as
             above: below fc the sides go, so the lows are mono; the middle is untouched

The inputs (44.1 kHz mono 32-bit float, 0.5 s = 22050 frames; stereo.wav two channels):
  p32_impulse.wav   1.0 on frame 0, then 0.0
  p32_sine440.wav   f32(0.5 sin(TAU * 440 * n / sr))
  p32_two_tone.wav  f32(0.25 sin(TAU * 220 * n / sr) + 0.25 sin(TAU * 3520 * n / sr))
  p32_stereo.wav    left f32(0.5 sin(TAU * 440 n / sr)), right f32(0.25 sin(TAU * 660 n / sr))
  p32_bursts.wav    frames [0, 4410) f32(0.5 sin(TAU * 440 n / sr)), [4410, 13230)
                    f32(lin(-60) sin(...)), [13230, 17640) 0.5 again, [17640, 22050) 0.0
                    (n counts from the file's start throughout, so the phase runs on)
The render: one bar at 120 BPM (88200 frames, `--bars 1`), so 1.5 s follow each file.

Tiers (A.9): "exact" cases compare the SHA-256 of the render's sample bytes (left, right
interleaved, little-endian f32: gen_p24's sha256_of, the WAV's data chunk); "close" cases compare
the probed frames (every frame in [0, 512), then every 32nd; p32_expected.json "probe_frames")
within 1e-6 and carry the generator's own SHA-256 too, which Linux's libm will likely match;
"amplitude" entries are closed forms measured by projection over whole periods (gen_p5's
amplitude) within 1e-4.

The cases (p32_<case>.json; A.9's table, both columns, and mix 0 for every effect with a mix):
  dry_<input>        no effect: the input's bits (proves the sampler path above), exact
  chorus_default     sine440, defaults, close; chorus_extreme: depth 1, rate 5, mix 1, close,
                     frames [0, 441) exactly 0; chorus_mix0: the dry hash; chorus_still:
                     depth 0, mix 1, every voice 661.5 frames back: amplitude
                     0.5 sqrt3 |Hherm| |Hhp| at 440 Hz (Hherm the half-frame Hermite, 9/8
                     cos(w/2) - 1/8 cos(3w/2); Hhp the 150 Hz high-pass's prewarped analog form);
                     chorus_still_half: the same at mix 0.5, amplitude 0.5 |sqrt(0.5) +
                     sqrt(0.5) sqrt3 Hherm Hhp e^(-j w 661.5)| (the equal-power law)
  flanger_default    sine440, defaults, close; flanger_comb_pos/_neg: impulse, depth 0, delay
                     10 ms, feedback +-0.95, mix 1: frame 441k is d_k = f32(fb d_(k-1)), d_1 = 1,
                     every other frame 0 (exact: depth 0 takes sin out of tau); flanger_mix0;
                     flanger_feedback70: sine440, feedback 0.7, close; flanger_feedback70_still:
                     the same at depth 0, exact (pins the line's f64 feedback sum; sqrt(0.5) is
                     correctly rounded, so blend_power keeps it exact); flanger_spread0:
                     spread 0, L == R bit for bit; flanger_lead: rate 5 Hz, spread 90:
                     the sides differ, and the right side leads the left by a quarter of the
                     LFO's period (2205 frames, 22 whole periods of 440 Hz), so right[n] ==
                     left[n + 2205] within 1e-5 once settled (the "lead" check)
  phaser_default     sine440, defaults, close; phaser_still: depth 0, 6 stages, center 0.5:
                     amplitude 0.5 |sqrt(0.5) + sqrt(0.5) e^(-j 6 theta)| = sqrt(2) 0.5 |cos(3
                     theta)| at 440 Hz (was 0.5 |cos(3 theta)| under blend);
                     phaser_feedback: 8 stages, feedback -0.95, close; phaser_mix0; phaser_spread0
                     and phaser_lead as the flanger's
  distortion_default sine440, defaults (auto gain on), close, and the "scaled"
                     check: every probe is makeup times distortion_auto_off's within 1e-6 (all
                     after the shaper is linear); distortion_auto_off: sine440, auto gain off,
                     close (the formula before Auto gain); distortion_hot: two_tone, drive
                     40 dB, close, every |frame| <= 1, exactly 0 from zero_from (the tone
                     filter's tail snaps soon after the file); distortion_mix0
  filter_default     sine440, LP at 632 Hz, Q 0.723 (comp 1): amplitude 0.5 / |1 - W^2 + jW/Q|,
                     close; filter_<lp|hp|bp|notch>_q20: resonance 1, cutoff the f32 control
                     nearest 440 Hz (440.000044 Hz): amplitude 0.5 Q comp (= 0.5 sqrt(Q0 Q)),
                     the same, 0.5, < 1e-4, close; filter_lp24_default: slope 24,
                     amplitude 0.5 comp |H(sqrt Q)|^2; filter_lp24_q20: slope 24 at 440 Hz,
                     resonance 1: 0.5 Q comp, filter_lp_q20's (the slopes meet at the cutoff);
                     filter_lp24_far: slope 24, cutoff the control nearest 110 Hz, two octaves
                     under the sine: 0.5 comp |H|^2, about -48 dB where 12 dB gives -24
  gate_default       bursts, defaults (RMS detector; threshold -40 dB), exact;
                     gate_range0: the dry hash; gate_hold0: exact; gate_release_1ms (not in A.9:
                     README inconsistency 2): exact, the -60 dB stretch is f32(x * floor) from
                     stretch.at_floor_from to its end; gate_rms_closed and gate_rms_open:
                     bursts at threshold -8 and -10 dB, exact: the 0.5 sine peaks at
                     -6.02 dBFS (a peak detector opens at both) but its RMS is -9.03 dBFS, so
                     the gate never opens at -8 and opens at -10 (self-checks on the trace)
  width_200          stereo, 2.0: exact, hand_frames; width_0: exact, L == R bit for bit;
                     width_100: the dry hash; width_bass_mono: stereo, bass mono 1
                     (500 Hz), width 1: close, amplitude at 440 Hz per side 0.25 |1 + Hhp| left,
                     0.25 |1 - Hhp| right; width_bass_mono_mono: sine440 (L == R), bass mono
                     0.5, width 2: the sides are 0, so the dry hash (a mono sound passes)
Each case's "asserts" lists what the Rust test checks for it.

Run:
  python3 -I gen_p32_expected.py              run the self-checks, print the known answers
  python3 -I gen_p32_expected.py --write DIR  also write the inputs, the p32_*.json projects and
                                              p32_expected.json into DIR
  python3 -I gen_p32_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import cmath, hashlib, importlib.util, json, math, os, struct, sys

HERE = os.path.dirname(os.path.abspath(__file__))


def rules():
    """gen_p11_expected.py, whose f32, exp2 and hermite restate the engine's."""
    sys.dont_write_bytecode = True                   # no __pycache__ beside the fixtures
    spec = importlib.util.spec_from_file_location("gen_p11_expected",
                                                  os.path.join(HERE, "gen_p11_expected.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


P11 = rules()
f32, exp2, hermite, jf, J = P11.f32, P11.exp2, P11.hermite, P11.jf, P11.J
LOG2_10, LN_2 = P11.LOG2_10, P11.LN_2
LOG2_E = 1.4426950408889634                          # std LOG2_E (gen_p24's)
TAU = 2.0 * math.pi                                  # std TAU
PI = math.pi                                         # std PI
TINY = 1e-30
SR = 44100
SRF = float(SR)
PPQ = 96
MBPM = 120000                                        # 120 BPM: one bar is 88200 frames
FRAMES = 4 * PPQ * 60 * SR * 1000 // (MBPM * PPQ)    # the tick rule: the render's length
FILE_FRAMES = SR // 2                                # every input: 0.5 s
CLOSE_TOLERANCE = 1e-6                               # gen_p24's LOOP_TOLERANCE
AMPLITUDE_TOLERANCE = 1e-4                           # gen_p5's EQ tolerance
PROBE_FRAMES = list(range(512)) + list(range(512, FRAMES, 32))
SVF_MAX = 0.45 * SRF                                 # the EQ's MAX_FREQ_FRACTION of the rate


# ---------- the shared parts (§3) ----------
def snap(v):
    """dsp.rs `snap` on an f32 (compared as f64)."""
    return 0.0 if abs(v) < TINY else v


snap64 = snap                                        # the same rule on an f64


def lin(db):
    return exp2(db / 20.0 * LOG2_10)                 # (dB / 20) * LOG2_10


def blend(dry, wet, mix):
    """dsp.rs `blend`: exact at mix 0 and 1, f32 arithmetic between."""
    if mix == 0.0: return dry
    if mix == 1.0: return wet
    keep = f32(1.0 - mix)
    return f32(f32(keep * dry) + f32(mix * wet))


def power_gains(mix):
    """blend_power's (dry, wet) gains: sqrt(1 - mix), sqrt(mix), correctly rounded."""
    return math.sqrt(1.0 - mix), math.sqrt(mix)


def blend_power(dry, wet, mix):
    """dsp.rs `blend_power`: exact at mix 0 and 1, equal power between, in f64."""
    if mix == 0.0: return dry
    if mix == 1.0: return wet
    gd, gw = power_gains(mix)
    return f32(gd * dry + gw * wet)                  # (gd * dry) + (gw * wet)


def bus(v):
    """The master's send: a zeroed bus plus the insert's frame times 1.0 (-0.0 -> +0.0)."""
    return 0.0 + v


class Line:
    """P1: per side the frames written since reset, read before the frame's write."""

    def __init__(self, max_ms):
        self.length = math.ceil(max_ms * SRF / 1000.0) + 4
        self.top = float(self.length - 3)
        self.pad = self.length + 4                   # frames before reset read as 0
        self.v = [[0.0] * self.pad, [0.0] * self.pad]

    def read(self, side, n, tau):
        tau = min(max(tau, 2.0), self.top)
        i = math.floor(tau)
        f = tau - i
        k = n - i + self.pad                         # y1 = v[n - i]
        h = self.v[side]
        return hermite(h[k + 1], h[k], h[k - 1], h[k - 2], f)

    def write(self, side, value):
        self.v[side].append(value)


def lfo_step(rate):
    return TAU * rate / SRF                          # (TAU * rate) / sr


def coefficient(ms):
    """P4 / the limiter's release rule: the 1/e time `ms`."""
    if ms == 0.0: return 0.0
    return exp2(-LOG2_E / (ms / 1000.0 * SRF))       # -LOG2_E / ((ms / 1000) * sr)


def svf_coefficients(fc, q):
    fc = min(max(fc, 20.0), SVF_MAX)
    g = math.tan(PI * fc / SRF)                      # (PI * fc) / sr
    k = 1.0 / q
    a1 = 1.0 / (1.0 + g * (g + k))
    a2 = g * a1
    a3 = g * a2
    return fc, g, k, a1, a2, a3


LOW, HIGH, BAND, NOTCH = range(4)                    # P3's outputs, the filter's type order


class Svf:
    """P3 with its memory (ic1, ic2) per side, run one value at a time."""

    def __init__(self, fc, q):
        _, _, self.k, self.a1, self.a2, self.a3 = svf_coefficients(fc, q)
        self.ic = [[0.0, 0.0], [0.0, 0.0]]

    def process(self, side, v0, kind):
        ic1, ic2 = self.ic[side]
        v3 = v0 - ic2
        v1 = self.a1 * ic1 + self.a2 * v3
        v2 = ic2 + self.a2 * ic1 + self.a3 * v3      # (ic2 + a2 * ic1) + a3 * v3
        self.ic[side] = [snap64(2.0 * v1 - ic1), snap64(2.0 * v2 - ic2)]
        k = self.k
        if kind == LOW: return v2
        if kind == HIGH: return v0 - k * v1 - v2     # (v0 - k * v1) - v2
        if kind == BAND: return k * v1
        return v0 - k * v1


FRAC_1_SQRT_2 = 0.7071067811865476                   # std FRAC_1_SQRT_2: Butterworth's Q
SQRT_3 = 1.7320508075688772                          # the f64 nearest sqrt 3 (std has none)
CHORUS_LOW_CUT_HZ = 150.0                            # the chorus's wet high-pass


# ---------- the controls' mappings (decision 6) ----------
def phaser_center_hz(v): return 100.0 * exp2(v * (2.0 + LOG2_10))
def distortion_tone_hz(v): return 1000.0 * exp2(v * (1.0 + LOG2_10))
def filter_cutoff_hz(v): return 20.0 * exp2(v * (3.0 * LOG2_10))
def filter_q(r): return 0.5 * exp2(r * (2.0 + LOG2_10))
def width_bass_hz(v): return 50.0 * exp2(v * LOG2_10)   # 50 Hz at 0+, 500 Hz at 1
def spread_offset(spread): return spread * PI / 180.0   # (spread * PI) / 180, radians


FILTER_Q0 = filter_q(f32(0.1))                       # the resonance control's default's Q


def filter_comp(kind, q):
    """The filter's level compensation: for the low- and high-pass, whose
    resonance rises Q times above the input at the cutoff, min(1, sqrt(Q0 / Q)) with Q0 the
    default's Q, so 1 exactly at the default and below it; 1 for the band-pass and the notch,
    which never rise above the input."""
    return min(1.0, math.sqrt(FILTER_Q0 / q)) if kind in (LOW, HIGH) else 1.0


def distortion_makeup(drive_db, auto):
    """Auto gain's scale: lin(-(d / (1 + d / 40))) dB when on, else 1."""
    return lin(-(drive_db / (1.0 + drive_db / 40.0))) if auto else 1.0


# ---------- the effects (§4) ----------
def chorus(inl, inr, p):
    rate, depth, mix = p["rate"], p["depth"], p["mix"]
    line = Line(20.0)
    low_cut = Svf(CHORUS_LOW_CUT_HZ, FRAC_1_SQRT_2)
    step, phase = lfo_step(rate), 0.0
    voice = [v * TAU / 3 for v in range(3)]          # (v * TAU) / 3
    side = [s * TAU / 4 for s in range(2)]           # (s * TAU) / 4
    sin, f32_, read = math.sin, f32, line.read
    out_l, out_r = [], []
    for n in range(FRAMES):
        x = (inl[n], inr[n])
        wet = [0.0, 0.0]
        for s in (0, 1):
            r0, r1, r2 = (read(s, n, (15.0 + 5.0 * depth * sin(phase + voice[v] + side[s]))
                               * SRF / 1000.0) for v in range(3))
            wet[s] = f32_(low_cut.process(s, (r0 + r1 + r2) / SQRT_3, HIGH))
        line.write(0, x[0]); line.write(1, x[1])
        phase += step
        if phase >= TAU: phase -= TAU
        out_l.append(blend_power(x[0], wet[0], mix))
        out_r.append(blend_power(x[1], wet[1], mix))
    return out_l, out_r


def flanger(inl, inr, p):
    delay_ms, depth, rate, fb, spread, mix = (p[k] for k in ("delay_ms", "depth", "rate",
                                                             "feedback", "spread", "mix"))
    line = Line(10.0)
    step, phase = lfo_step(rate), 0.0
    base = delay_ms * SRF / 1000.0                   # (delay_ms * sr) / 1000
    offsets = (0.0, spread_offset(spread))
    out_l, out_r = [], []
    for n in range(FRAMES):
        x = (inl[n], inr[n])
        wet = [0.0, 0.0]
        for s in (0, 1):
            tau = max(2.0, base * (1.0 - depth * (1.0 - math.sin(phase + offsets[s])) / 2.0))
            d = line.read(s, n, tau)
            line.write(s, snap(f32(x[s] + fb * d)))
            wet[s] = d
        phase += step
        if phase >= TAU: phase -= TAU
        out_l.append(blend_power(x[0], wet[0], mix))
        out_r.append(blend_power(x[1], wet[1], mix))
    return out_l, out_r


def phaser(inl, inr, p):
    stages, rate, depth, center, fb, spread, mix = (p[k] for k in (
        "stages", "rate", "depth", "center", "feedback", "spread", "mix"))
    count = 4 + 2 * int(stages)
    center_hz = phaser_center_hz(center)
    step, phase = lfo_step(rate), 0.0
    offsets = (0.0, spread_offset(spread))
    x1 = [[0.0] * count, [0.0] * count]
    y1 = [[0.0] * count, [0.0] * count]
    last = [0.0, 0.0]
    out_l, out_r = [], []
    for n in range(FRAMES):
        coefficient = []
        for s in (0, 1):
            f = center_hz * exp2(depth / 2.0 * math.sin(phase + offsets[s]))
            f = min(max(f, 20.0), SVF_MAX)
            t = math.tan(PI * f / SRF)
            coefficient.append((t - 1.0) / (t + 1.0))
        x = (inl[n], inr[n])
        wet = [0.0, 0.0]
        for s in (0, 1):
            a = coefficient[s]
            u = x[s] + fb * last[s]
            xs, ys = x1[s], y1[s]
            for j in range(count):
                y = a * u + xs[j] - a * ys[j]
                xs[j] = snap64(u)
                ys[j] = snap64(y)
                u = y
            last[s] = snap64(u)
            wet[s] = f32(u)
        phase += step
        if phase >= TAU: phase -= TAU
        out_l.append(blend_power(x[0], wet[0], mix))
        out_r.append(blend_power(x[1], wet[1], mix))
    return out_l, out_r


def antiderivative(u):
    """ln cosh u, overflow-safe."""
    a = abs(u)
    return a + math.log1p(math.exp(-2.0 * a)) - LN_2


def distortion(inl, inr, p):
    drive, outdb, mix = lin(p["drive_db"]), lin(p["output_db"]), p["mix"]
    makeup = distortion_makeup(p["drive_db"], p["auto_gain"] == 1.0)
    fc = distortion_tone_hz(p["tone"])
    c = 1.0 - exp2(-(TAU * fc / SRF * LOG2_E))      # 1 - exp2(-(((TAU * fc) / sr) * LOG2_E))
    u1, lp = [0.0, 0.0], [0.0, 0.0]
    outs = ([], [])
    for n in range(FRAMES):
        x = (inl[n], inr[n])
        for s in (0, 1):
            u = drive * x[s]
            d = u - u1[s]
            if abs(d) < 1e-5:
                y = math.tanh((u + u1[s]) / 2.0)
            else:
                y = (antiderivative(u) - antiderivative(u1[s])) / d
            u1[s] = u
            y = y * makeup
            lp[s] = snap64(lp[s] + c * (y - lp[s]))
            outs[s].append(blend(x[s], f32(outdb * lp[s]), mix))
    return outs


FILTER_TYPES = ["Low-pass", "High-pass", "Band-pass", "Notch"]
FILTER_SLOPES = ["12 dB", "24 dB"]


def filter_(inl, inr, p):
    kind, stages = int(p["type"]), 1 + int(p["slope"])
    q = filter_q(p["resonance"])
    stage_q = q if stages == 1 else math.sqrt(q)
    fc = filter_cutoff_hz(p["cutoff"])
    chain = [Svf(fc, stage_q) for _ in range(stages)]
    comp = filter_comp(kind, q)
    outs = ([], [])
    for s, xs in enumerate((inl, inr)):
        out = outs[s]
        for v0 in xs:
            y = v0
            for stage in chain:
                y = stage.process(s, y, kind)
            out.append(f32(y * comp))
    return outs


def gate(inl, inr, p, trace=None):
    t_db, attack, hold_ms, release, range_db = (p[k] for k in ("threshold_db", "attack_ms",
                                                               "hold_ms", "release_ms", "range_db"))
    opens, closes = lin(t_db), lin(t_db - 3.0)
    floor = lin(range_db)
    c_attack, c_release = coefficient(attack), coefficient(release)
    detector = coefficient(10.0)                     # the mean square's time, both ways
    hold_frames = math.floor(hold_ms / 1000.0 * SRF + 0.5)
    square, is_open, hold_left, g = 0.0, False, 0, floor
    out_l, out_r = [], []
    for n in range(FRAMES):
        l, r = inl[n], inr[n]
        square = snap64(detector * square + (1.0 - detector) * max(l * l, r * r))
        level = math.sqrt(square)
        if level >= opens or (is_open and level >= closes):
            is_open, hold_left = True, hold_frames
        elif is_open and hold_left > 0:
            hold_left -= 1
        else:
            is_open = False
        target = 1.0 if is_open else floor
        c = c_attack if target > g else c_release
        g = c * g + (1.0 - c) * target
        out_l.append(f32(l * g))
        out_r.append(f32(r * g))
        if trace is not None: trace.append((is_open, g, level))
    return out_l, out_r


def width(inl, inr, p):
    w, bass = p["width"], p.get("bass_mono", 0.0)
    if bass == 0.0 and w == 1.0: return list(inl), list(inr)
    low_cut = Svf(width_bass_hz(bass), FRAC_1_SQRT_2) if bass != 0.0 else None
    out_l, out_r = [], []
    for l, r in zip(inl, inr):
        m = (l + r) / 2.0
        if low_cut is None:
            s = (l - r) / 2.0 * w                    # ((l - r) / 2) * w
        else:
            s = low_cut.process(0, (l - r) / 2.0, HIGH) * w
        out_l.append(f32(m + s))
        out_r.append(f32(m - s))
    return out_l, out_r


# ---------- the parameter tables (§4 A.1-A.7, A.8) ----------
def P(key, label, lo, hi, default, unit, whole=False):
    return {"key": key, "label": label, "min": lo, "max": hi, "default": default, "unit": unit,
            "whole": whole}


EFFECTS = {
    "dawsome.chorus": ("Chorus", chorus, [
        P("rate", "Rate", 0.1, 5.0, 0.5, "Hz"),           # was 0.8
        P("depth", "Depth", 0.0, 1.0, 0.3, "%"),          # was 0.5
        P("mix", "Mix", 0.0, 1.0, 0.5, "%")]),
    "dawsome.flanger": ("Flanger", flanger, [
        P("delay_ms", "Delay", 0.1, 10.0, 2.0, "ms"),
        P("depth", "Depth", 0.0, 1.0, 0.5, "%"),
        P("rate", "Rate", 0.05, 5.0, 0.3, "Hz"),
        P("feedback", "Feedback", -0.95, 0.95, 0.5, "%"),
        P("spread", "Spread", 0.0, 180.0, 90.0, "\u00b0"),
        P("mix", "Mix", 0.0, 1.0, 0.5, "%")]),
    "dawsome.phaser": ("Phaser", phaser, [
        P("stages", "Stages", 0.0, 2.0, 1.0, "", True),
        P("rate", "Rate", 0.05, 5.0, 0.5, "Hz"),
        P("depth", "Depth", 0.0, 4.0, 2.0, "oct"),
        P("center", "Center", 0.0, 1.0, 0.5, "Hz"),
        P("feedback", "Feedback", -0.95, 0.95, 0.0, "%"),
        P("spread", "Spread", 0.0, 180.0, 90.0, "\u00b0"),
        P("mix", "Mix", 0.0, 1.0, 0.5, "%")]),
    "dawsome.distortion": ("Distortion", distortion, [
        P("drive_db", "Drive", 0.0, 40.0, 12.0, "dB"),
        P("tone", "Tone", 0.0, 1.0, 1.0, "Hz"),
        P("output_db", "Output", -24.0, 6.0, 0.0, "dB"),
        P("auto_gain", "Auto gain", 0.0, 1.0, 1.0, "", True),
        P("mix", "Mix", 0.0, 1.0, 1.0, "%")]),
    "dawsome.filter": ("Filter", filter_, [
        P("type", "Type", 0.0, 3.0, 0.0, "", True),
        P("slope", "Slope", 0.0, 1.0, 0.0, "", True),
        P("cutoff", "Cutoff", 0.0, 1.0, 0.5, "Hz"),
        P("resonance", "Resonance", 0.0, 1.0, 0.1, "%")]),
    "dawsome.gate": ("Gate", gate, [
        P("threshold_db", "Threshold", -80.0, 0.0, -40.0, "dB"),
        P("attack_ms", "Attack", 0.1, 100.0, 1.0, "ms"),
        P("hold_ms", "Hold", 0.0, 500.0, 20.0, "ms"),
        P("release_ms", "Release", 1.0, 1000.0, 100.0, "ms"),
        P("range_db", "Range", -80.0, 0.0, -80.0, "dB")]),
    "dawsome.width": ("Stereo width", width, [
        P("width", "Width", 0.0, 2.0, 1.0, "%"),
        P("bass_mono", "Bass mono", 0.0, 1.0, 0.0, "Hz")]),
}
VALUE_NAMES = {"dawsome.phaser": {"stages": ["4 stages", "6 stages", "8 stages"]},
               "dawsome.distortion": {"auto_gain": ["Off", "On"]},
               "dawsome.filter": {"type": FILTER_TYPES, "slope": FILTER_SLOPES},
               # Only 0 has a name (Off); 1 is a frequency (null: no name).
               "dawsome.width": {"bass_mono": ["Off", None]}}
SHOWN = {("dawsome.phaser", "center"): phaser_center_hz,
         ("dawsome.distortion", "tone"): distortion_tone_hz,
         ("dawsome.filter", "cutoff"): filter_cutoff_hz,
         ("dawsome.width", "bass_mono"): width_bass_hz}
ADD_MENU_NEW_ROWS = ["dawsome.filter", "dawsome.distortion", "dawsome.chorus", "dawsome.flanger",
                     "dawsome.phaser", "dawsome.gate", "dawsome.width"]


def values_of(eid, given):
    """Every parameter's value as the effect gets it: the given one or the default, clamped
    (all in range here), as an f32."""
    out = {}
    for info in EFFECTS[eid][2]:
        v = f32(given.get(info["key"], info["default"]))
        if info["whole"]: v = math.copysign(math.floor(abs(v) + 0.5), v)   # f32::round
        out[info["key"]] = v
    return out


def metadata():
    out = {}
    for eid, (name, _, params) in EFFECTS.items():
        entry = {"id": eid, "name": name,
                 "params": [dict(info, min=f32(info["min"]), max=f32(info["max"]),
                                 default=f32(info["default"])) for info in params]}
        if eid in VALUE_NAMES: entry["value_names"] = VALUE_NAMES[eid]
        shown = [[key, [[f32(v), fn(f32(v))] for v in (0.0, 0.5, 1.0)]]
                 for (e, key), fn in SHOWN.items() if e == eid]
        if shown: entry["shown"] = {key: pts for key, pts in shown}
        out[eid] = entry
    return out


# ---------- the inputs ----------
def tone(amplitude, hz, n): return amplitude * math.sin(TAU * hz * n / SRF)


MINUS_60 = lin(-60.0)
B1, B2, B3 = SR // 10, 3 * SR // 10, 4 * SR // 10   # 100 ms, then 200 ms, then 100 ms


def burst(n):
    if n < B1: return f32(tone(0.5, 440.0, n))
    if n < B2: return f32(tone(MINUS_60, 440.0, n))
    if n < B3: return f32(tone(0.5, 440.0, n))
    return 0.0


INPUTS = {
    "p32_impulse.wav": [[1.0] + [0.0] * (FILE_FRAMES - 1)],
    "p32_sine440.wav": [[f32(tone(0.5, 440.0, n)) for n in range(FILE_FRAMES)]],
    "p32_two_tone.wav": [[f32(tone(0.25, 220.0, n) + tone(0.25, 3520.0, n))
                          for n in range(FILE_FRAMES)]],
    "p32_stereo.wav": [[f32(tone(0.5, 440.0, n)) for n in range(FILE_FRAMES)],
                       [f32(tone(0.25, 660.0, n)) for n in range(FILE_FRAMES)]],
    "p32_bursts.wav": [[burst(n) for n in range(FILE_FRAMES)]],
}


def wav_bytes(channels):
    """32-bit float WAV at 44.1 kHz (gen_p24's writer)."""
    n = len(channels[0])
    data = struct.pack("<%df" % (n * len(channels)), *[c[i] for i in range(n) for c in channels])
    fmt = struct.pack("<HHIIHH", 3, len(channels), SR, SR * 4 * len(channels),
                      4 * len(channels), 32)
    body = (b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data"
            + struct.pack("<I", len(data)) + data)
    return b"RIFF" + struct.pack("<I", len(body)) + body


def bus_input(name):
    """Insert 1's bus: the file's frames (a mono frame on both sides), then 0.0."""
    chans = INPUTS[name]
    left = chans[0] + [0.0] * (FRAMES - FILE_FRAMES)
    right = (chans[1] if len(chans) > 1 else chans[0]) + [0.0] * (FRAMES - FILE_FRAMES)
    return left, right


# ---------- the projects ----------
def insert(i, effects=()):
    return {"id": i, "name": "Master" if i == 0 else "Insert %d" % i, "fader": 0.8, "pan": 0.0,
            "mute": False, "routes": [] if i == 0 else [{"to": 0, "amount": 1.0}],
            "effects": list(effects)}


def project(sample, eid=None, params=None):
    """gen_p5's shape: one sampler channel into insert 1, one step hit at tick 0; the effect,
    if any, in insert 1's slot 0 as gen_p5's effect(eid, slot, kind, params). Format version 2:
    what the engine writes for sampler channels with default sound settings and no edits
    (project.rs FORMAT_VERSION). A project with no effect writes no mixer, as gen_p5's do: its
    mixer is the default one (the master and 16 inserts, each routed to the master at unity),
    which a load fills in and a save leaves out, so every project here saves back to its own
    bytes (engine tests/round_trip.rs)."""
    doc = {"format_version": 2, "sample_rate": SR, "tempo_mbpm": MBPM, "ppq": PPQ,
           "next_channel_id": 2,
           "channels": [{"id": 1, "name": "Input", "kind": "sampler", "sample": sample,
                         "volume": 1.0, "pan": 0.0, "mute": False, "insert": 1}],
           "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": 1,
                         "notes": [{"channel": 1, "tick": 0, "length": 0, "velocity": 1.0}]}]}
    if eid is not None:
        given = {k: float(jf(v)) for k, v in sorted((params or {}).items())}
        effects = [{"id": 1, "slot": 0, "processor": {"id": eid, "params": given}}]
        doc["mixer"] = {"next_effect_id": 2, "solo": [],
                        "inserts": [insert(0), insert(1, effects)]
                        + [insert(i) for i in range(2, 17)]}
    return doc


def cutoff_for(hz):
    """The f32 cutoff control whose frequency is nearest `hz` (an input value: log is fine)."""
    v = f32(math.log(hz / 20.0) / math.log(1000.0))
    near = [v, f32(v * (1 + 2 ** -23)), f32(v * (1 - 2 ** -23))]
    return min(near, key=lambda c: abs(filter_cutoff_hz(c) - hz))


CUTOFF_440 = cutoff_for(440.0)
CUTOFF_110 = cutoff_for(110.0)
LEAD = SR // 20                                      # a quarter of a 5 Hz LFO's period, 2205
                                                     # frames: 22 whole periods of 440 Hz

# case: (effect id or None for the dry render, input, the params the project writes, tier)
CASES = {
    "dry_impulse": (None, "p32_impulse.wav", {}, "exact"),
    "dry_sine440": (None, "p32_sine440.wav", {}, "exact"),
    "dry_two_tone": (None, "p32_two_tone.wav", {}, "exact"),
    "dry_stereo": (None, "p32_stereo.wav", {}, "exact"),
    "dry_bursts": (None, "p32_bursts.wav", {}, "exact"),
    "chorus_default": ("dawsome.chorus", "p32_sine440.wav", {}, "close"),
    "chorus_extreme": ("dawsome.chorus", "p32_sine440.wav",
                       {"depth": 1.0, "rate": 5.0, "mix": 1.0}, "close"),
    "chorus_mix0": ("dawsome.chorus", "p32_sine440.wav", {"mix": 0.0}, "exact"),
    # Depth 0 holds every voice at 661.5 frames, so the closed forms see the level law
    # (sum / sqrt 3), the wet high-pass and, at mix 0.5, the equal-power mix.
    "chorus_still": ("dawsome.chorus", "p32_sine440.wav", {"depth": 0.0, "mix": 1.0}, "close"),
    "chorus_still_half": ("dawsome.chorus", "p32_sine440.wav", {"depth": 0.0}, "close"),
    "flanger_default": ("dawsome.flanger", "p32_sine440.wav", {}, "close"),
    "flanger_comb_pos": ("dawsome.flanger", "p32_impulse.wav",
                         {"depth": 0.0, "delay_ms": 10.0, "feedback": 0.95, "mix": 1.0}, "exact"),
    "flanger_comb_neg": ("dawsome.flanger", "p32_impulse.wav",
                         {"depth": 0.0, "delay_ms": 10.0, "feedback": -0.95, "mix": 1.0}, "exact"),
    "flanger_mix0": ("dawsome.flanger", "p32_sine440.wav", {"mix": 0.0}, "exact"),
    # Feedback 0.7 (not a power of two) on a sounding line, for the line's write, x + fb * d
    # summed in f64. An f32 sum changes this render's hash but moves no probe by more than
    # 3e-8, inside the close tolerance: the probes alone do not pin that sum.
    "flanger_feedback70": ("dawsome.flanger", "p32_sine440.wav", {"feedback": 0.7}, "close"),
    # Its exact twin: depth 0 takes the sine out of the delay read (depth * (1 - sin) is exactly
    # 0, so tau is 88.2 frames whatever libm returns), so the hash is the same on every
    # platform, and it pins the f64 feedback sum, which flanger_feedback70's probes cannot.
    "flanger_feedback70_still": ("dawsome.flanger", "p32_sine440.wav",
                                 {"feedback": 0.7, "depth": 0.0}, "exact"),
    # Spread 0 is the mono flange (one phase); at 5 Hz the right side, 90 degrees
    # ahead, is the left a quarter period (LEAD frames) later.
    "flanger_spread0": ("dawsome.flanger", "p32_sine440.wav", {"spread": 0.0}, "close"),
    "flanger_lead": ("dawsome.flanger", "p32_sine440.wav", {"rate": 5.0}, "close"),
    "phaser_default": ("dawsome.phaser", "p32_sine440.wav", {}, "close"),
    "phaser_still": ("dawsome.phaser", "p32_sine440.wav", {"depth": 0.0, "stages": 1.0,
                                                          "center": 0.5}, "close"),
    "phaser_feedback": ("dawsome.phaser", "p32_sine440.wav", {"stages": 2.0, "feedback": -0.95},
                        "close"),
    "phaser_mix0": ("dawsome.phaser", "p32_sine440.wav", {"mix": 0.0}, "exact"),
    "phaser_spread0": ("dawsome.phaser", "p32_sine440.wav", {"spread": 0.0}, "close"),
    "phaser_lead": ("dawsome.phaser", "p32_sine440.wav", {"rate": 5.0}, "close"),
    "distortion_default": ("dawsome.distortion", "p32_sine440.wav", {}, "close"),
    "distortion_auto_off": ("dawsome.distortion", "p32_sine440.wav", {"auto_gain": 0.0}, "close"),
    "distortion_hot": ("dawsome.distortion", "p32_two_tone.wav", {"drive_db": 40.0}, "close"),
    "distortion_mix0": ("dawsome.distortion", "p32_sine440.wav", {"mix": 0.0}, "exact"),
    "filter_default": ("dawsome.filter", "p32_sine440.wav", {}, "close"),
    "filter_lp_q20": ("dawsome.filter", "p32_sine440.wav",
                      {"type": 0.0, "cutoff": CUTOFF_440, "resonance": 1.0}, "close"),
    "filter_hp_q20": ("dawsome.filter", "p32_sine440.wav",
                      {"type": 1.0, "cutoff": CUTOFF_440, "resonance": 1.0}, "close"),
    "filter_bp_q20": ("dawsome.filter", "p32_sine440.wav",
                      {"type": 2.0, "cutoff": CUTOFF_440, "resonance": 1.0}, "close"),
    "filter_notch_q20": ("dawsome.filter", "p32_sine440.wav",
                         {"type": 3.0, "cutoff": CUTOFF_440, "resonance": 1.0}, "close"),
    "filter_lp24_default": ("dawsome.filter", "p32_sine440.wav", {"slope": 1.0}, "close"),
    "filter_lp24_q20": ("dawsome.filter", "p32_sine440.wav",
                        {"slope": 1.0, "cutoff": CUTOFF_440, "resonance": 1.0}, "close"),
    "filter_lp24_far": ("dawsome.filter", "p32_sine440.wav",
                        {"slope": 1.0, "cutoff": CUTOFF_110}, "close"),
    "gate_default": ("dawsome.gate", "p32_bursts.wav", {}, "exact"),
    "gate_range0": ("dawsome.gate", "p32_bursts.wav", {"range_db": 0.0}, "exact"),
    "gate_hold0": ("dawsome.gate", "p32_bursts.wav", {"hold_ms": 0.0}, "exact"),
    # Not in A.9's table: the default release (1/e in 100 ms) never brings the -60 dB stretch
    # to the floor inside its 200 ms (README, inconsistency 2); at 1 ms it does.
    "gate_release_1ms": ("dawsome.gate", "p32_bursts.wav", {"release_ms": 1.0}, "exact"),
    # The RMS detector. The 0.5 sine's RMS (-9.03 dBFS) lies between -10 and -8,
    # its peak (-6.02) above both: open at -10, never at -8.
    "gate_rms_closed": ("dawsome.gate", "p32_bursts.wav", {"threshold_db": -8.0}, "exact"),
    "gate_rms_open": ("dawsome.gate", "p32_bursts.wav", {"threshold_db": -10.0}, "exact"),
    "width_200": ("dawsome.width", "p32_stereo.wav", {"width": 2.0}, "exact"),
    "width_0": ("dawsome.width", "p32_stereo.wav", {"width": 0.0}, "exact"),
    "width_100": ("dawsome.width", "p32_stereo.wav", {"width": 1.0}, "exact"),
    "width_bass_mono": ("dawsome.width", "p32_stereo.wav", {"bass_mono": 1.0}, "close"),
    "width_bass_mono_mono": ("dawsome.width", "p32_sine440.wav",
                             {"bass_mono": 0.5, "width": 2.0}, "exact"),
}


def fixture(case): return "p32_%s.json" % case


PROJECTS = {fixture(c): project(inp, eid, params) for c, (eid, inp, params, _) in CASES.items()}


# ---------- the renders ----------
_RENDERS = {}


def render(case):
    """(left, right) of the file `dawsome render` writes for `case`, and the effect's values."""
    if case not in _RENDERS:
        eid, inp, params, _ = CASES[case]
        inl, inr = bus_input(inp)
        if eid is None:
            values, (yl, yr) = {}, (inl, inr)
        else:
            values = values_of(eid, params)
            yl, yr = EFFECTS[eid][1](inl, inr, values)
        _RENDERS[case] = ([bus(v) for v in yl], [bus(v) for v in yr]), values
    return _RENDERS[case]


def sha256_of(left, right):
    data = struct.pack("<%df" % (2 * len(left)), *[x for pair in zip(left, right) for x in pair])
    return hashlib.sha256(data).hexdigest()


def amplitude(y, hz, lo, hi):
    """gen_p5's: projection on sin/cos over [lo, hi), a whole number of periods."""
    s = sum(y[n] * math.sin(TAU * hz * n / SRF) for n in range(lo, hi))
    c = sum(y[n] * math.cos(TAU * hz * n / SRF) for n in range(lo, hi))
    return 2.0 * math.hypot(s, c) / (hi - lo)


def whole_periods(lo, hi, hz):
    assert (hi - lo) * hz % SR == 0, (lo, hi, hz)    # 4410 frames hold 44 periods of 440 Hz


def first_zero_tail(left, right):
    """The first frame from which every frame is exactly 0.0 on both sides."""
    k = len(left)
    while k > 0 and left[k - 1] == 0.0 and right[k - 1] == 0.0: k -= 1
    return k


def svf_closed_complex(kind, hz, fc, q):
    """H of the prewarped analog SVF at hz, for the input e^(+j w n): the trapezoidal SVF is the
    bilinear transform of the analog one, s = j W, W = tan(PI hz / sr) / tan(PI fc / sr)."""
    w = math.tan(PI * hz / SRF) / math.tan(PI * fc / SRF)
    d = complex(1.0 - w * w, w / q)
    num = [1.0, -w * w, complex(0.0, w / q), 1.0 - w * w][kind]
    return num / d


def svf_closed(kind, hz, fc, q):
    """|H| of the prewarped analog SVF at hz (the trapezoidal SVF's exact steady state)."""
    return abs(svf_closed_complex(kind, hz, fc, q))


def phaser_closed(amplitude_in, count, hz, fc, mix):
    """A sine of amplitude_in at hz through `count` still stages at fc, mixed by blend_power:
    each stage lags theta = 2 atan(tan(PI hz / sr) / tan(PI fc / sr)) (90 degrees at fc)."""
    theta = 2.0 * math.atan(math.tan(PI * hz / SRF) / math.tan(PI * fc / SRF))
    gd, gw = power_gains(mix)
    return amplitude_in * abs(gd + gw * cmath.exp(complex(0.0, -count * theta))), theta


def chorus_still_closed(amplitude_in, hz, mix):
    """A sine through the still chorus (depth 0): each voice reads 15 ms back, 661.5 frames at
    44.1 kHz, the half-frame Hermite over the frames 660..663 back, (-1, 9, 9, -1) / 16: a
    delay of 661.5 frames times 9/8 cos(w/2) - 1/8 cos(3w/2); the three summed over sqrt 3
    (sqrt 3 times one voice); the 150 Hz high-pass; then blend_power with the dry sine."""
    w = TAU * hz / SRF
    hermite_gain = 9.0 / 8.0 * math.cos(w / 2.0) - 1.0 / 8.0 * math.cos(3.0 * w / 2.0)
    delay = 15.0 * SRF / 1000.0
    wet = (math.sqrt(3.0) * hermite_gain * svf_closed_complex(HIGH, hz, CHORUS_LOW_CUT_HZ,
                                                              FRAC_1_SQRT_2)
           * cmath.exp(complex(0.0, -w * delay)))
    if mix == 1.0: return amplitude_in * abs(wet)
    gd, gw = power_gains(mix)
    return amplitude_in * abs(gd + gw * wet)


WINDOW = (4410, FILE_FRAMES)                         # 176 periods of 440 Hz, inside the file
WINDOW_Q20 = (13230, FILE_FRAMES)                    # 88 periods, the Q 20 ring (640 frames)
                                                     # down by e^-20 first
LEAD_TOLERANCE = 1e-5                                # right[n] against left[n + LEAD]
LEAD_SPAN = (4410, FILE_FRAMES - LEAD)               # settled, and both inside the file
DIFFER_BY = 0.01                                     # "the sides differ": by more than this


def amplitude_answer(left, right, expected, window=WINDOW, **extra):
    whole_periods(*window, 440)
    return dict({"hz": 440.0, "window": list(window), "expected": expected,
                 "tolerance": AMPLITUDE_TOLERANCE,
                 "measured": [amplitude(left, 440.0, *window), amplitude(right, 440.0, *window)]},
                **extra)


def case_answer(case):
    eid, inp, params, tier = CASES[case]
    (left, right), values = render(case)
    a = {"fixture": fixture(case), "effect": eid, "input": inp, "params": values,
         "tier": tier, "frames": FRAMES, "sha256": sha256_of(left, right)}
    if tier == "close":
        a["probes"] = {"left": [float(jf(left[k])) for k in PROBE_FRAMES],
                       "right": [float(jf(right[k])) for k in PROBE_FRAMES]}
    passes = (values.get("mix") == 0.0 or values.get("range_db") == 0.0
              or (values.get("width") == 1.0 and values.get("bass_mono") == 0.0)
              or case == "width_bass_mono_mono")     # a mono input has no sides to change
    if eid is not None and passes:
        a["same_as"] = "dry_" + inp[4:-4]
    tail = first_zero_tail(left, right)
    a["zero_from"] = tail
    if case == "chorus_extreme":
        first = next(k for k in range(FRAMES) if left[k] != 0.0 or right[k] != 0.0)
        a["zero_before"] = 441                       # (15 - 5) ms at 44.1 kHz
        a["first_sounding"] = first
    if case in ("chorus_still", "chorus_still_half"):
        a["amplitude"] = amplitude_answer(left, right, chorus_still_closed(0.5, 440.0,
                                                                            values["mix"]))
    if case.startswith("flanger_comb"):
        a["sounding"] = [[k, left[k]] for k in range(FRAMES) if left[k] != 0.0 or right[k] != 0.0]
    if case == "phaser_still":
        count = 4 + 2 * int(values["stages"])
        want, theta = phaser_closed(0.5, count, 440.0, phaser_center_hz(values["center"]),
                                    values["mix"])
        a["amplitude"] = amplitude_answer(left, right, want, theta=theta)
    if case.endswith("_spread0"):
        a["left_equals_right"] = True
    if case.endswith("_lead"):
        lo, hi = LEAD_SPAN
        a["lead"] = {"frames": LEAD, "from": lo, "to": hi, "tolerance": LEAD_TOLERANCE,
                     "measured": max(abs(right[n] - left[n + LEAD]) for n in range(lo, hi))}
        a["sides_differ"] = {"by_more_than": DIFFER_BY,
                             "measured": max(abs(x - y) for x, y in zip(left, right))}
    if case == "distortion_default":
        other = render("distortion_auto_off")[0][0]
        factor = distortion_makeup(values["drive_db"], True)
        a["scaled"] = {"of": "distortion_auto_off", "factor": factor,
                       "tolerance": CLOSE_TOLERANCE,
                       "measured": max(abs(left[k] - factor * other[k]) for k in PROBE_FRAMES)}
    if eid == "dawsome.filter":
        kind, stages = int(values["type"]), 1 + int(values["slope"])
        q = filter_q(values["resonance"])
        stage_q = q if stages == 1 else math.sqrt(q)
        fc, *_ = svf_coefficients(filter_cutoff_hz(values["cutoff"]), stage_q)
        comp = filter_comp(kind, q)
        window = WINDOW_Q20 if values["resonance"] == 1.0 else WINDOW
        want = 0.5 * comp * svf_closed(kind, 440.0, fc, stage_q) ** stages
        a["amplitude"] = amplitude_answer(left, right, want, window, fc=fc, q=q, comp=comp,
                                          stage_q=stage_q)
    if case == "distortion_hot":
        a["peak"] = max(max(abs(x), abs(y)) for x, y in zip(left, right))
        a["peak_at_most"] = 1.0
    if case.startswith("gate"):
        closed, at, end_gain, floor = gate_settle(case)
        a["stretch"] = {"frames": [B1, B2], "floor": floor, "closed_at": closed,
                        "gain_at_end": end_gain, "at_floor_from": at}
    if case == "width_200":
        inl, inr = bus_input(inp)
        a["hand_frames"] = [[k, inl[k], inr[k], left[k], right[k]]
                            for k in (0, 1, 2, 25, 100, 1000, FILE_FRAMES - 1, FILE_FRAMES)]
    if case == "width_0":
        a["left_equals_right"] = True
    if case == "width_bass_mono":
        h = svf_closed_complex(HIGH, 440.0, width_bass_hz(values["bass_mono"]), FRAC_1_SQRT_2)
        a["amplitude"] = amplitude_answer(left, right, [0.25 * abs(1.0 + h), 0.25 * abs(1.0 - h)])
    # What the Rust test asserts for this case (README "asserts").
    asserts = ["sha256"] if tier == "exact" else ["probes"]
    for key in ("same_as", "zero_before", "sounding", "amplitude", "peak_at_most", "hand_frames",
                "left_equals_right", "lead", "sides_differ", "scaled"):
        if key in a: asserts.append(key)
    if case == "distortion_hot": asserts.append("zero_from")
    if case == "gate_release_1ms": asserts.append("stretch.at_floor_from")
    a["asserts"] = asserts
    return a


def expected():
    cases = {case: case_answer(case) for case in CASES}
    return {"rate": SR, "frames": FRAMES, "bars": 1, "tempo_mbpm": MBPM,
            "file_frames": FILE_FRAMES,
            "inputs": {name: {"channels": len(chans), "frames": len(chans[0])}
                       for name, chans in INPUTS.items()},
            "tolerances": {"close": CLOSE_TOLERANCE, "amplitude": AMPLITUDE_TOLERANCE},
            "probe_frames": PROBE_FRAMES,
            "effects": metadata(),
            "add_menu_new_rows": ADD_MENU_NEW_ROWS,
            "cases": cases}


# ---------- self-checks ----------
CHECKS = []
NOTES = []                                           # spec statements that do not hold


def check(name, ok, detail):
    CHECKS.append((name, ok, detail))


def svf_steady(kind, hz, fc, q):
    return abs(svf_steady_complex(kind, hz, fc, q))


def svf_steady_complex(kind, hz, fc, q):
    """The SVF's exact steady-state response at hz from its own recurrence: with v0 = z^n,
    ic1 = I1 z^n, ic2 = I2 z^n, the update gives V1 = (z + 1) I1 / 2, V2 = (z + 1) I2 / 2, and
    V1 = a1 I1 + a2 (1 - I2), V2 = I2 + a2 I1 + a3 (1 - I2): a 2x2 complex system."""
    _, _, k, a1, a2, a3 = svf_coefficients(fc, q)
    z = cmath.exp(complex(0.0, TAU * hz / SRF))
    h = (z + 1.0) / 2.0
    # (h - a1) I1 + a2 I2 = a2 ; -a2 I1 + (h - 1 + a3) I2 = a3
    m11, m12, m21, m22 = h - a1, a2, -a2, h - 1.0 + a3
    det = m11 * m22 - m12 * m21
    i1 = (a2 * m22 - m12 * a3) / det
    i2 = (m11 * a3 - m21 * a2) / det
    v1 = h * i1
    v2 = h * i2
    return [v2, 1.0 - k * v1 - v2, k * v1, 1.0 - k * v1][kind]


def svf_lp_gap(hz, fc, q):
    """|steady state of the recurrence's LP| - |1 / (1 - W^2 + jW/Q)| at hz, both evaluated in
    exact rationals (svf_steady's 2x2 system) from the f64 coefficients and t = f64 tan(PI hz /
    sr), with z = (1 + jt) / (1 - jt) exactly on the unit circle (so h = (z + 1) / 2 =
    1 / (1 - jt)) and W = t / g: what remains is the coefficients' own rounding."""
    from fractions import Fraction as F
    _, g, k, a1, a2, a3 = svf_coefficients(fc, q)
    a1, a2, a3 = F(a1), F(a2), F(a3)
    mul = lambda x, y: (x[0] * y[0] - x[1] * y[1], x[0] * y[1] + x[1] * y[0])
    sub = lambda x, y: (x[0] - y[0], x[1] - y[1])
    def div(x, y):
        d = y[0] * y[0] + y[1] * y[1]
        return ((x[0] * y[0] + x[1] * y[1]) / d, (x[1] * y[0] - x[0] * y[1]) / d)
    t = F(math.tan(PI * hz / SRF))
    h = div((F(1), F(0)), (F(1), -t))
    m11, m12, m21, m22 = (h[0] - a1, h[1]), (a2, F(0)), (-a2, F(0)), (h[0] - 1 + a3, h[1])
    det = sub(mul(m11, m22), mul(m12, m21))
    i2 = div(sub(mul(m11, (a3, F(0))), mul(m21, (a2, F(0)))), det)
    v2 = mul(h, i2)
    rec2 = v2[0] * v2[0] + v2[1] * v2[1]
    ww = t / F(g)
    closed2 = 1 / ((1 - ww * ww) ** 2 + (ww / F(q)) ** 2)
    return float(abs(rec2 - closed2)) / (math.sqrt(float(rec2)) + math.sqrt(float(closed2)))


def gate_settle(case):
    """(frame the gate closes in the -60 dB stretch, first frame from which every frame to the
    stretch's end is exactly f32(x * floor) or None, gain at the stretch's end, floor)."""
    values = render(case)[1]
    trace = []
    inl, inr = bus_input(CASES[case][1])
    left, _ = gate(inl, inr, values, trace)
    floor = lin(values["range_db"])
    closed = next(k for k in range(B1, B2) if not trace[k][0])
    at = B2
    while at > closed and left[at - 1] == f32(inl[at - 1] * floor): at -= 1
    return closed, (at if at < B2 else None), trace[B2 - 1][1], floor


def svf_simulated(kind, hz, fc, q, lo=8800, hi=13200):
    """The SVF run on an f64 sine at hz (no f32 rounding), measured by projection."""
    _, _, k, a1, a2, a3 = svf_coefficients(fc, q)
    ic1 = ic2 = 0.0
    y = []
    for n in range(hi):
        v0 = math.sin(TAU * hz * n / SRF)
        v3 = v0 - ic2
        v1 = a1 * ic1 + a2 * v3
        v2 = ic2 + a2 * ic1 + a3 * v3
        ic1 = snap64(2.0 * v1 - ic1)
        ic2 = snap64(2.0 * v2 - ic2)
        y.append([v2, v0 - k * v1 - v2, k * v1, v0 - k * v1][kind])
    return amplitude(y, hz, lo, hi)


def self_checks(answers):
    # P3: a sine at fc (441 Hz, 100 frames a period), Q 3: LP and HP x Q, BP x 1, notch ~0.
    fc, q = 441.0, 3.0
    for kind, want in ((0, q), (1, q), (2, 1.0)):
        got = svf_simulated(kind, fc, fc, q)
        check("SVF %s at fc x%g (simulated)" % (FILTER_TYPES[kind], want), abs(got - want) < 1e-12,
              "%.17g (error %.2g)" % (got, abs(got - want)))
    notch = svf_simulated(3, fc, fc, q)
    check("SVF notch at fc < 1e-13 (simulated)", notch < 1e-13, "%.3g" % notch)
    worst = worst_float = 0.0
    for hz in (110.0, 220.0, 882.0, 1764.0, 4410.0, 11025.0):
        for qq in (0.5, 0.7071067811865476, 3.0, 20.0):
            worst = max(worst, svf_lp_gap(hz, fc, qq))
            worst_float = max(worst_float, abs(svf_steady(0, hz, fc, qq)
                                               - svf_closed(0, hz, fc, qq)))
    check("SVF LP off fc = 1/|1 - W^2 + jW/Q| to 1e-15 (the recurrence's steady state, exact "
          "rationals)", worst <= 1e-15, "largest difference %.3g (%.3g evaluated in floats)"
          % (worst, worst_float))
    lp_sim = svf_simulated(0, 882.0, fc, q)
    check("SVF LP off fc, simulated by projection (rounding-limited)",
          abs(lp_sim - svf_closed(0, 882.0, fc, q)) < 1e-12,
          "difference %.3g" % abs(lp_sim - svf_closed(0, 882.0, fc, q)))
    # Flanger +-95%: frame 441k is d_k = f32(fb d_(k-1)), d_1 = 1, every other frame 0.
    for case in ("flanger_comb_pos", "flanger_comb_neg"):
        fb = f32(CASES[case][2]["feedback"])
        chain, d = [], 1.0
        for k in range(1, FRAMES // 441 + 1):
            if 441 * k < FRAMES: chain.append([441 * k, d])
            d = f32(fb * d)
        (left, right), _ = render(case)
        got = [[k, left[k]] for k in range(FRAMES) if left[k] != 0.0 or right[k] != 0.0]
        same_sides = left == right
        check("flanger %s: 441k frames are the f32 chain, the rest exactly 0" % case[-3:],
              got == chain and same_sides,
              "%d echoes, last %r at %d" % (len(chain), chain[-1][1], chain[-1][0]))
    # The complex closed form (phase too) is the recurrence's own steady state: the chorus's
    # and the width's closed forms add the high-passed sound to the dry one.
    worst_complex = max(abs(svf_steady_complex(kind, hz, fcc, qq) - svf_closed_complex(kind, hz, fcc, qq))
                        for kind in (LOW, HIGH) for hz in (110.0, 440.0, 1764.0)
                        for fcc in (CHORUS_LOW_CUT_HZ, 500.0) for qq in (FRAC_1_SQRT_2, 3.0))
    check("SVF H(jW) with its phase = the recurrence's steady state", worst_complex < 1e-12,
          "largest difference %.3g" % worst_complex)
    # Every closed form (amplitude) within 1e-4, per side when it names two.
    for case, a in answers["cases"].items():
        if "amplitude" not in a: continue
        amp = a["amplitude"]
        want = amp["expected"] if isinstance(amp["expected"], list) else [amp["expected"]] * 2
        err = max(abs(m - w) for m, w in zip(amp["measured"], want))
        check("%s: amplitude closed form within 1e-4" % case, err < AMPLITUDE_TOLERANCE,
              "expected %s, error %.2g" % (", ".join("%.9f" % w for w in want), err))
    # Phaser: 6 stages, depth 0, mix 50%: sqrt(2) 0.5 |cos(3 theta)| at 440 Hz (blend_power).
    amp = answers["cases"]["phaser_still"]["amplitude"]
    by_cos = math.sqrt(2.0) * 0.5 * abs(math.cos(3.0 * amp["theta"]))
    check("phaser 6 stages depth 0, mix 50%: |sqrt(.5) + sqrt(.5) e^(-j 6 theta)| = sqrt(2)|cos 3 "
          "theta|", abs(amp["expected"] - by_cos) < 1e-12, "%.12f %.12f" % (amp["expected"], by_cos))
    # Filter: the four types at Q 20 at 440 Hz, compensated: LP and HP
    # 0.5 Q sqrt(Q0 / Q) = 0.5 sqrt(Q0 Q), BP 0.5, notch < 1e-4; the 24 dB slope's pair at
    # sqrt(Q) each meets the 12 dB stage at the cutoff (440 Hz is 1e-7 under the cutoff's
    # 440.000044 Hz, so they meet to about 1e-7 there: |H| near the cutoff goes as Q / W per
    # stage, Q / W against Q / W^2).
    q20 = {c: answers["cases"]["filter_%s_q20" % c]["amplitude"]["expected"]
           for c in ("lp", "hp", "bp", "notch", "lp24")}
    qv = answers["cases"]["filter_lp_q20"]["amplitude"]["q"]
    half = 0.5 * math.sqrt(FILTER_Q0 * qv)
    check("filter Q 20 at 440 Hz: LP, HP 0.5 sqrt(Q0 Q), BP 0.5, notch < 1e-4; 24 dB LP = 12 dB LP",
          abs(q20["lp"] - half) < 1e-4 and abs(q20["hp"] - half) < 1e-4
          and abs(q20["bp"] - 0.5) < 1e-4 and q20["notch"] < 1e-4
          and abs(q20["lp24"] - q20["lp"]) < 1e-6,
          "Q %.9f: LP %.9f HP %.9f BP %.9f notch %.3g LP24 %.9f" % (
              qv, q20["lp"], q20["hp"], q20["bp"], q20["notch"], q20["lp24"]))
    check("filter comp: exactly 1 at the default Q and below; sqrt(Q0 / 20) at 100%%: %.6f = %.2f dB"
          % (filter_comp(LOW, filter_q(1.0)), 20.0 * math.log10(filter_comp(LOW, filter_q(1.0)))),
          filter_comp(LOW, filter_q(f32(0.1))) == 1.0 and filter_comp(HIGH, 0.5) == 1.0
          and answers["cases"]["filter_default"]["amplitude"]["comp"] == 1.0
          and abs(filter_comp(LOW, filter_q(1.0)) - math.sqrt(FILTER_Q0 / 20.0)) < 1e-12, "")
    far24 = answers["cases"]["filter_lp24_far"]["amplitude"]
    far12 = 0.5 * far24["comp"] * svf_closed(LOW, 440.0, far24["fc"], far24["q"])
    check("filter 24 dB two octaves above the cutoff: about 24 dB under the 12 dB slope",
          -26.0 < 20.0 * math.log10(far24["expected"] / far12) < -22.0,
          "24 dB %.6f, 12 dB %.6f: %.1f dB" % (far24["expected"], far12,
                                              20.0 * math.log10(far24["expected"] / far12)))
    # Chorus: still voices are one sound three times, so their sum over sqrt 3 is
    # sqrt 3 times one voice (+4.8 dB); moving voices are uncorrelated and their powers add, so
    # the same sum keeps the input's level (averaged, it was -4.8 dB). Through
    # the high-pass (-0.06 dB at 440 Hz) and the Hermite (-0.0005 dB): 0.5 sqrt3 |H| = 0.860.
    still = answers["cases"]["chorus_still"]["amplitude"]["expected"]
    check("chorus still, mix 100%: 0.5 sqrt3 |Hherm Hhp|, 4.7 dB over the dry 0.5",
          0.85 < still < 0.87, "%.6f (%+.2f dB)" % (still, 20.0 * math.log10(still / 0.5)))
    # Flanger and phaser: spread 0 is mono; spread 90 at 5 Hz leads by LEAD frames.
    for case in ("flanger_lead", "phaser_lead"):
        a = answers["cases"][case]
        check("%s: right[n] = left[n + %d] within %g (measured %.3g), the sides differ by %.3f"
              % (case, LEAD, LEAD_TOLERANCE, a["lead"]["measured"], a["sides_differ"]["measured"]),
              a["lead"]["measured"] < LEAD_TOLERANCE / 10
              and a["sides_differ"]["measured"] > DIFFER_BY, "")
    for case in ("flanger_spread0", "phaser_spread0"):
        (left, right), _ = render(case)
        check("%s: L == R bit for bit" % case, left == right, "")
    # Distortion: auto gain is a scale after the shaper, so the default render is
    # makeup times the render without it; makeup at 12 dB is lin(-12 / 1.3) = -9.23 dB.
    sc = answers["cases"]["distortion_default"]["scaled"]
    check("distortion auto gain: default = %.9f x auto off, within %.3g" % (sc["factor"],
                                                                           sc["measured"]),
          sc["measured"] <= sc["tolerance"] and abs(sc["factor"] - 10 ** (-12.0 / 1.3 / 20)) < 1e-12,
          "")
    # blend_power: the ends' bits; equal power between; its constants.
    gd, gw = power_gains(f32(0.3))
    check("blend_power: exact at 0 and 1, gains squared sum to 1, sqrt(0.5) twice at 50%",
          blend_power(-0.0, 0.25, 0.0) == -0.0 and blend_power(0.7, -0.0, 1.0) == -0.0
          and abs(gd * gd + gw * gw - 1.0) < 1e-15 and power_gains(0.5) == (math.sqrt(0.5),) * 2,
          "")
    check("SQRT_3, FRAC_1_SQRT_2 are the correctly rounded f64s",
          SQRT_3 == math.sqrt(3.0) and FRAC_1_SQRT_2 == math.sqrt(0.5), "")
    # Gate: the -60 dB stretch once closed comes out at the floor (x1e-4). At the default
    # release (1/e in 100 ms) it does not within the stretch: a note, not a check (README).
    closed, at, end_gain, floor = gate_settle("gate_default")
    NOTES.append("gate_default: the spec's '-60 dB stretch comes out x1e-4 once closed' does not "
                 "hold: closes at frame %d, gain %.6g at the stretch's end (frame %d), floor %.6g; "
                 "%s" % (closed, end_gain, B2 - 1, floor,
                         "never exactly x floor" if at is None else "x floor from %d" % at))
    closed, at, end_gain, floor = gate_settle("gate_release_1ms")
    check("gate_release_1ms: the -60 dB stretch comes out exactly f32(x floor) once settled",
          at is not None and at - closed < SR // 20 and B2 - at > SR // 20,
          "closes at frame %d, every frame from %d to the stretch's end %d is f32(x %.17g)"
          % (closed, at, B2, floor))
    # Gate: the RMS detector reads the 0.5 sine at -9.03 dBFS, between -10 and -8
    # dB, where a peak detector (0.5 = -6.02 dBFS) opens at both.
    for case, opens in (("gate_rms_closed", False), ("gate_rms_open", True)):
        values = render(case)[1]
        trace = []
        gate(*bus_input(CASES[case][1]), values, trace)
        top = max(level for _, _, level in trace[B1 // 2:B1])
        check("%s: threshold %g dB, the RMS reads at most %.3f dBFS in the burst; %s" % (
              case, values["threshold_db"], 20.0 * math.log10(top),
              "opens" if opens else "never opens"),
              any(t[0] for t in trace) == opens and 0.5 >= lin(values["threshold_db"]), "")
    # Width: 0% is L == R bit for bit; the hand answer.
    (left, right), _ = render("width_0")
    check("width 0%: L == R bit for bit", struct.pack("<%df" % FRAMES, *left)
          == struct.pack("<%df" % FRAMES, *right), "%d frames" % FRAMES)
    hand2 = width([0.5], [0.25], {"width": 2.0})
    hand0 = width([0.5], [0.25], {"width": 0.0})
    check("width hand answer: (0.5, 0.25) -> (0.625, 0.125) at 2, (0.375, 0.375) at 0",
          hand2 == ([0.625], [0.125]) and hand0 == ([0.375], [0.375]), "%r %r" % (hand2, hand0))
    # Distortion: two-tone at 40 dB never past 1.0; exact 0 after the file's tail.
    hot = answers["cases"]["distortion_hot"]
    check("distortion two-tone 40 dB: every |frame| <= 1.0", hot["peak"] <= 1.0,
          "peak %r" % hot["peak"])
    check("distortion two-tone 40 dB: exactly 0 from shortly after the file",
          FILE_FRAMES < hot["zero_from"] <= FILE_FRAMES + 64,
          "every frame from %d on is 0.0 (the file ends at %d)" % (hot["zero_from"], FILE_FRAMES))
    # Chorus: frames before 10 ms exactly 0 at depth 100%, mix 100%.
    ch = answers["cases"]["chorus_extreme"]
    check("chorus depth 100%: frames [0, 441) exactly 0", ch["first_sounding"] >= 441,
          "first sounding frame %d" % ch["first_sounding"])
    # Every 'same_as' case is the dry render's bits; the dry renders are the inputs' bits.
    for case, a in answers["cases"].items():
        if "same_as" in a:
            dry = answers["cases"][a["same_as"]]["sha256"]
            check("%s = %s" % (case, a["same_as"]), a["sha256"] == dry, a["sha256"][:16])
    # Exact-tier claims with no libm in the path.
    hermite_zero = hermite(0.3, f32(0.7), -0.2, 0.9, 0.0) == f32(0.7)
    check("Hermite at f = 0 is y1's bits", hermite_zero, "")
    check("lin(0) = 1 exactly; coefficient(0) = 0", lin(0.0) == 1.0 and coefficient(0.0) == 0.0, "")
    failed = [c for c in CHECKS if not c[1]]
    return failed


# ---------- files ----------
def files(answers):
    out = {name: wav_bytes(chans) for name, chans in INPUTS.items()}
    for name, doc in PROJECTS.items():
        out[name] = (J(doc) + "\n").encode()
    out["p32_expected.json"] = (J(answers) + "\n").encode()
    return out


def main():
    args = sys.argv[1:]
    if args and not (len(args) == 2 and args[0] in ("--write", "--check")): sys.exit(__doc__)
    answers = expected()
    failed = self_checks(answers)
    if args:
        stale = []
        for path, content in sorted(files(answers).items()):
            target = os.path.join(args[1], path)
            if args[0] == "--write":
                os.makedirs(os.path.dirname(target) or ".", exist_ok=True)
                with open(target, "wb") as f: f.write(content)
            else:
                try:
                    with open(target, "rb") as f: same = f.read() == content
                except OSError: same = False
                if not same: stale.append(path)
        if stale: sys.exit("not what gen_p32_expected.py writes: " + ", ".join(stale))
    for name, ok, detail in CHECKS:
        print("%s  %s%s" % ("ok  " if ok else "FAIL", name, ": " + detail if detail else ""))
    for note in NOTES:
        print("NOTE  " + note)
    for case, a in answers["cases"].items():
        print("%-20s %-6s %s %s zero from %d" % (case, a["tier"], a["fixture"], a["sha256"][:16],
                                                 a["zero_from"]))
    if failed: sys.exit("%d self-check(s) failed" % len(failed))


if __name__ == "__main__":
    main()
