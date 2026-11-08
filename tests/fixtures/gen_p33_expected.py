"""Known answers for Dawsome Synth: the synth channel kind, its parameters, the voice pool and the
amp envelope; the oscillators' waves, width and tuning, oscillator 2 and the mix, unison and
noise; the filter, its envelope, key tracking and velocity → cutoff, the LFO and the mod wheel,
and glide; the DC-free pulse, free-running oscillator phases and the Drive.
The pattern cases are projects of one synth channel rendered by `dawsome render` to 32-bit
float; the live cases are MIDI messages handed to the player beside a project of one synth
channel, as gen_p22_expected.py's are.

Expected values come from the formulas restated below, never from engine code (the synth
did not exist yet: this file is the ground truth its Rust follows). gen_p11_expected.py's `f32`,
`exp2`, `jf`, `J` and `LOG2_10` are imported, as gen_p24_expected.py does, and
gen_p32_expected.py's restatement of P3 (the SVF the Filter effect runs: its coefficients, its
memory, the prewarped analog closed form) and the Filter's Q law, so the synth's filter is the
effect's math. Every input is authored here: the projects, the MIDI messages,
p33_expected.json and p33_laws.json.

Notation: all arithmetic is f64 unless written f32(...); a parameter is the f32 the project file
holds, widened to f64; sr = 44100.0; TAU = 2 pi (std TAU). Sums and products are evaluated LEFT
TO RIGHT exactly as written; each order the laws leave open is fixed below (the Rust follows
them).

The parameters (37, then the Drive, the 38th, after key_track: it is
additive, a missing key reading 0, so no format version moves): PARAMS below, in
the order the instrument window's groups will show them (Osc 1, Osc 2 with the mix, Unison
with the noise, Filter, Filter env, Amp env, LFO, Play). Log controls go by these laws:
  time       t(v) = 0.001 * exp2(v * (4 * LOG2_10)) s [the constant 4 * LOG2_10 first, as
             log2 10000 = 4 LOG2_10]: 1 ms at 0, 10 ms at 0.25, 100 ms at 0.5, 1 s at 0.75,
             10 s at 1; shown in ms as exp2(v * (4 * LOG2_10))
  cutoff     20 * exp2(v * (3 * LOG2_10)) Hz [the constant 3 * LOG2_10 first]
  resonance  Q = 0.5 * exp2(r * (2 + LOG2_10)), the Filter effect's (gen_p32's filter_q)
  lfo_rate   0.01 * exp2(v * (1 + 3 * LOG2_10)) Hz

The voice:
  events     a block's events in (frame, arrival) order; one at frame f applies before frame f
             renders. Note{key, velocity, length}: a voice whose gate closes `length` frames on
             (off_in = length, counted down after each frame; the frame that finds it 0 starts
             the release). NoteOn: a voice held until a NoteOff of its key, which releases every
             unreleased NoteOn voice of that key. Bend: bend = value / 4096 semitones. Wheel:
             w = value / 127. A bend, a wheel or a parameter change retunes every voice on its
             next frame
  voices     16. A note takes the free voice of lowest index, else steals the oldest: the
             smallest start count (a count of the notes started since the last reset, first
             note 0). A stolen voice restarts its glide, keeps its filter memory and attacks
             both envelopes from their values (no click); a free voice starts its filter
             envelope at 0 and its filter memory clear. Neither touches its phases or its
             Drive memory
  unison     U = the unison control at the note's start (it keeps it for its life); voice k of
             U: det_k = detune * (k / (U - 1) - 0.5) cents, pan_k = spread * ((2 k) / (U - 1)
             - 1), both 0 for U = 1; its pan gains laws::channel_pan_gains(f32(pan_k))
             (gen_p11's pan_gains), widened
  phases     free-running: a reset puts slot s's unison voice k, on both
             oscillators, at frac(s PHI + k / 3), PHI = 0.6180339887498949 [(s PHI) + (k / 3)
             first]; a note never sets them (it starts where its slot's are). The law before had
             every note start them at k / U
  glide      a note started while another voice is held (attack, decay or sustain, its gate not
             closing on this frame, and sounding since an earlier frame: a chord struck from
             nothing does not glide into itself) and glide_ms > 0 glides from the previously
             started note's pitch now: g0 = (key_prev + glide_prev) - key semitones, over
             N = floor(glide_ms / 1000 * sr + 0.5) frames (both fixed at its start; N = 0: no
             glide); on its m-th frame glide = g0 * (1 - m / N) for m < N, else 0
  LFO        one, free-running from reset: phase t = 0 at reset; each frame its value from t,
             then t = t + rate / sr, if t >= 1: t = t - 1. Sine sin(TAU t), triangle the
             oscillator's, square t < 0.5 ? 1 : -1, saw 2 t - 1. A frame where nothing uses
             it (pitch depth lfo_pitch + w = 0, and no cutoff depth on a running filter) takes
             0.0 for its value without computing it; its phase steps all the same.
             vibrato = lfo * (lfo_pitch + w)
  pitch      per oscillator o and unison voice k: p = (key - 69) + bend, then + glide,
             + vibrato, + 12 * oct_o, + semi_o, + cents_o / 100, + det_k / 100, one term at a
             time in that order; f = min(440 * exp2(p / 12), 0.45 * sr); dt = f / sr (kept per
             voice while nothing moves: worked out again on a frame that follows a retune, on
             every frame of a glide up to and with m = N, and on every frame while lfo_pitch +
             w is not 0: the same bits as every frame)
  waves      phase t; value from t, then t = t + dt, if t >= 1: t = t - 1. blep(t, dt): t < dt:
             x = t / dt, ((2 x) - (x x)) - 1; t > 1 - dt: x = (t - 1) / dt, ((x x) + (2 x)) + 1;
             else 0. Sine sin(TAU t); triangle 1 - 4 |frac(t + 0.25) - 0.5|; saw ((2 t) - 1) -
             blep(t, dt); square (((t < wd ? 1 : -1) + blep(t, dt)) - blep(frac((t + 1) - wd),
             dt)) - dc, wd the f32 width widened, dc = 2 wd - 1 its mean over a period (DC-free,
             +0.0 at wd 0.5, which leaves every bit); frac(x) = x - floor(x)
  noise      a xorshift32 state per voice slot, seeded slot + 1 at reset (not at a note's
             start: a slot's next note goes on from it); each frame the voice sounds,
             s ^= s << 13, s ^= s >> 17, s ^= s << 5 (u32), then nz = s / 2^31 - 1
  mix        a = (1 - osc_mix) * level1, b = osc_mix * level2; per side s: acc = 0.0, acc = acc
             + g_k,s * (a * o1_k + b * o2_k) for k in order; osc_s = acc / U + noise * nz
  envelope   coef(K, v) = exp2(-K / (t(v) * sr)); K_A = 2.115477217419936 (log2(1.3/0.3)) for
             the attack, K_DR = 13.287856641840545 (log2(1.0001/0.0001)) for decay and release
             attack   env = 1.3 * (1 - a) + env * a; env >= 1: env = 1, then decay
             decay    env = (S - 0.0001) * (1 - d) + env * d; env <= S: env = S, then sustain
             sustain  env = S (the parameter as it is now)
             release  env = -0.0001 * (1 - r) + env * r; env <= 0: env = 0 (the amp envelope's
                      0 frees the voice; the filter envelope's stays at 0)
             The amp envelope and the filter envelope (fenv) share it, each with its controls.
  filter     P3's low-pass per voice and side (gen_p32's Svf), Q = filter_q(resonance) (no level
             compensation). It is open (its output not taken: the voice hears
             osc_s exactly) while cutoff = 1 and filter_env, key_track, lfo_cutoff and
             vel_cutoff are all 0; its memory runs on every frame all the same, at the open
             cutoff, so a cutoff moved off 100% mid-note goes on from a filter that was
             listening. Its cutoff on a frame:
               semis = filter_env * fenv * 96 + key_track * (key - 60) + lfo * lfo_cutoff * 48
                       + vel_cutoff * (vel - 1) * 48   (left to right; key - 60 exact)
               fc = 20 * exp2(cutoff * (3 * LOG2_10) + semis / 12), open: 20 * exp2(cutoff *
                    (3 * LOG2_10)); clamped to [20, 0.45 sr] by the coefficients (P3)
             The coefficients are worked out again when fc's bits differ from the last ones
             (and after a parameter change, as Q may move): the same bits as every frame
  drive      d = 1 + drive * 5, tanh(d) and g = gen_p32's
             distortion_makeup(20 log10(d), on) (the Distortion's auto gain at the drive in dB
             whose gain is d) once per settings. Per voice and side after heard_s,
             before env: drive 0: heard_s itself, u1 = heard_s; else u = d heard_s,
             y = gen_p32's adaa_tanh(u, u1) (the Distortion's antialiased tanh), u1 = u,
             heard_s = (y / tanh(d)) * g. u1 per voice side: 0 at reset, kept across notes
  a frame    the LFO's value; then per sounding voice, in slot order: the gate check (off_in 0:
             both envelopes release), the amp envelope's step, the filter envelope's (each new
             value is this frame's), the pitch when it moves, each oscillator's value and phase
             step (osc 1's voices, then osc 2's), the noise step, the off_in count, the
             filter's cutoff, then per side osc_s, its filtered y_s = Svf(osc_s),
             voice_s = (drive(heard_s) * env) * (1 - vel_amp * (1 - vel)), heard_s = osc_s
             while the filter is open, else y_s; then the voice's frame count m; then the
             LFO's step
  output     per side, sum = 0.0, sum = sum + voice_s for each sounding voice in slot order;
             out_s = f32(sum * lin(volume_db)), lin(dB) = exp2(dB / 20 * LOG2_10)
The render path after the synth (read from render.rs): the channel's volume 1.0 and centred pan
give gains (1.0, 1.0); its sound is added into the master's bus, zeroed when first fed (0.0 + y
* 1.0: only -0.0 becomes +0.0); the master has no effects (a loaded file lists none), fader
unity, pan 0; 32-bit float is written as is. So the render is the synth's output with negative
zeros made positive.

The live path (gen_p22's rules): a message timed for call k is taken at that call's first frame,
512 k, and reaches the synth as an event at that frame. A note on of byte velocity v plays
velocity f32(v / 128). CC 64 at 64 or more holds back the note offs of the keys let go of while
it is down; its lift sends them, at the lift's frame, in the order the keys were let go of. Pitch
bend E0 ll mm is ll + 128 mm - 8192. CC 1 is the mod wheel, sent on when it moves off the value
the device holds (0 to start with), as a plugin's is: a Wheel event at that frame.

Tiers: a case with a sine wave in its path is "close": the probed frames
(every frame in [0, 512), every 32nd frame after, and the frames around the case's events)
within 1e-6, with the generator's own SHA-256 as a hint that Linux's libm will likely match.
Closed forms: the rectangular projection over whole periods (gen_p5's amplitude) within 1e-4;
for tones beside others (the steal, the bends) the projection under a Hann window, whose
leakage from tones many bins away is far under 1e-4 (a rectangular window leaks ~1e-3 from a
tone 300 Hz away over 36400 frames). A case with no sine wave is "exact", its SHA-256 (saw,
square_width, unison, noise; the first three tightened from 1e-6, as p32's flanger combs
were): its only libm is the pan law's sin at pan 0 or +-1, and f32(sqrt 2 sin(pi/4)) = 1,
f32(sqrt 2 sin(pi/2)) = f32(sqrt 2) and sin(0) = 0 whatever libm's last bit. A running filter
takes tan (libm) every time its cutoff moves, so a case whose filter runs is "close"; an open
filter's memory runs on tan too, but its output is never heard, so it leaves a case exact. The
Drive above 0 takes tanh, exp and log1p (libm), so a driven case is close; at 0 it takes none.
Zero crossings: the upward ones (a frame below 0, the next at or above it) in a
window, one for one with the frames worked out by hand within 1 frame.

The cases:
  sine       osc 1 sine, the default envelope (A 1 ms, D 100 ms, S 100%, R 100 ms), key 69 at
             velocity 1 for the whole bar: probes; the 440 Hz amplitude over [4410, 88200) (836
             periods, all sustain) is lin(-6) within 1e-4
  adsr       A 0.25 (10 ms), D 0.5, S 0.5, R 0.5, key 69 for 0.25 s (48 ticks, 11025 frames):
             probes; the envelope reaches 1 at frame 441 +- 1 (the generator's frame, 440, is
             "env_one_at"; the synth's own unit test holds it to 441 +- 1), its release
             starts on frame 11025 and every frame from the one it reaches 0 on is exactly 0.0
             ("zero_from"; the frame before it is not)
  steal      18 notes 100 frames apart (275.625 BPM: one tick is 100 frames), each held to the
             bar's end (38400 frames): note 1 key 33 (55 Hz), note 2 key 45 (110 Hz), notes 3
             to 18 keys 69 to 84. Note 17 steals note 1's voice (the oldest; also the lowest
             slot), note 18 steals note 2's (the oldest, slot 1, where the lowest slot would be
             note 17's): probes; over [2000, 38400) 55 Hz and 110 Hz are gone (Hann amplitude
             under 1e-4) and keys 69 to 84 sound at lin(-6) (within 1e-4). 17 notes would not
             tell "oldest" from "lowest slot"; the 18th does
  pedal      (live) pedal down (call 2), key 60 on (4), off (10), pedal up (53): probes; the
             render is "held"'s bit for bit and "unreleased"'s up to the lift's frame 27136,
             not on it (the release starts on the lift's frame)
  held       (live) key 60 on (4), off (53): probes
  unreleased (live) key 60 on (4), never off: probes
  bend       (live) key 69 on (4), bend +8191 (20), -8192 (40), 0 (50), off (60): probes; each
             span between bends sounds 440 * 2^(b/4096/12) Hz at lin(-6) * 127/128
The oscillators' (pattern cases, rendered by the command line):
  saw        the defaults (osc 1 the PolyBLEP saw, osc 2's square mixed out), key 81: probes
  square_width  osc 1 square width 0.05, osc 2 square width 0.95, mix 0.3 (unequal, so the
             widths swapped would not sum the same); key 105 (3520 Hz: dt 0.0798 > 0.05, the
             narrow pulse's two corrections overlap) for half the bar, then key 57 (220 Hz):
             probes; each half's mean over whole periods by hand, 0.7 (2 * 0.05 - 1) + 0.3 (2 *
             0.95 - 1) of the volume (a pulse averages 2w - 1), within 1e-4
  tuning     osc 1 sine octave +1, semitones -3, cents +30 (752.9 Hz) at 0.7 of the volume;
             osc 2 triangle octave -1 (220 Hz) at level 0.6, mix 0.3: probes; both tones by hand
             (the triangle's fundamental 8/pi^2), Hann, within 1e-4
  unison     osc 1 saw, U 3, detune 50, spread 1, noise 0.2, key 69: probes, the sides
             differ
  unison_tones  the same voices as sines, no noise: on each side its outer voice at
             f32(sqrt 2)/3 and the middle one at 1/3 of the volume, the other side's outer
             voice absent (Hann, 1e-4): the detune and the pan law by hand
  noise      osc 1 at level 0, noise 1: note A (slot 0), B (slot 1, while A sounds), C (slot 0
             again, after both are free): exact; the self-checks pin slot 0's and slot 1's first
             values by hand and that C goes on from slot 0's state
The filter's, LFO's and glide's (pattern cases, rendered by the command line, unless "live"):
  filter     the saw (the defaults) at key 57 for half the bar, cutoff 0.5, resonance 1
             (Q 20), filter_env 1 (the default filter envelope: A 1 ms, D 100 ms, S 0, R 100
             ms), key_track 1: probes
  cutoff_tones  key tracking and velocity -> cutoff by hand: osc 1 sine, cutoff 0.5, resonance
             0.5, key_track 0.5, vel_cutoff 1, vel_amp 0 (so velocity moves the cutoff alone);
             keys 57 and 69 at velocity 1, key 81 at 0.5, held: each tone at lin(-6) |H| of the
             prewarped analog low-pass at its own cutoff, 632.46 Hz * 2^((0.5 (key - 60) + 48
             (vel - 1)) / 12) (Hann, 1e-4)
  lfo_pitch  exact: the saw at key 69 with the triangle LFO at 20 Hz to pitch, 2 semitones:
             probes, the hash, and its zero crossings (the saw's, at phase 1/2) by hand
  lfo_cutoff the square LFO at 0.956 Hz to the cutoff, 50% (+-2 octaves), of a sine at 220 Hz
             through cutoff 880 Hz, resonance 0.5: the LFO starts high (t = 0, +1), so the filter
             sits at 3520 Hz, then 220 Hz, then 3520 Hz: each half's tone at lin(-6) |H| by hand
  wheel      (live) key 69 on (4), the mod wheel to 100 (20), back to 0 (120), off (160): the
             default sine LFO (2.99 Hz) swings the pitch +-100/127 semitone while the wheel is
             up: probes, and the zero crossings by hand over the whole note
  glide      200 ms glide, key 60 then key 72 a beat on, 60 held one tick past 72's
             start (1 ms release, so 72 sounds alone soon after): 72's zero crossings by hand,
             its pitch 72 - 12 (1 - m / 8820) semitones on its m-th frame, then 72; probes
  glide_separated  the same notes, 60 ending on the frame 72 starts (its gate closes there: not
             held, the note schedule's note-offs-first order): 72 does not glide, its crossings
             at 523.25 Hz from its start
The pulse's, the phases' and the Drive's (pattern cases, rendered by the command line):
  repeat     Init, C4 for a beat, then C4 again on the same (freed) slot two beats on: the
             second does not play the first's frames ("differs"; with phases set at each note
             it played them bit for bit), and starts its saw where the first left it, by hand: exact
  chord      Init, six notes from C3 struck together: the first 50 ms peak below that of
             every note starting from phase 0 ("peak_below", the old law's peak computed
             here): exact
  drive_half, drive_full  the Saw Lead preset's voice at drive 0.5 and 1, A3 then E4 on the
             same slot after A3 is free (its Drive memory carried over): probes, sides differ
  drive_tone  a sine at A3, drive 0.2 (d = 2), filter open: its 220 and 660 Hz harmonics
             LIN_6 b_n(2) / tanh(2) g by hand, b_n the sine series of tanh(2 sin x), g the auto
             gain 10^(-(D / (1 + D / 40)) / 20) at D = 20 log10 2 (Hann, 1e-4)
  drive_zero  the saw case with "drive": 0.0 written: the saw's bits ("same_as"): exact
And "synth": the factory's id, name, instrument flag and every parameter's key, label, range,
default, unit and whole flag; the names the choices' values go by; and what the log controls'
0, 0.5 and 1 stand for (exp2 alone, so exact).

Run:
  python3 -I gen_p33_expected.py              run the self-checks, print the known answers
  python3 -I gen_p33_expected.py --write DIR  also write the p33_*.json projects, the messages,
                                              p33_expected.json and p33_laws.json into DIR
  python3 -I gen_p33_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import cmath, hashlib, importlib.util, math, os, statistics, struct, sys

HERE = os.path.dirname(os.path.abspath(__file__))


def rules(name):
    """A generator beside this one, loaded as a module: gen_p11_expected.py, whose f32, exp2 and
    jf restate the engine's, and gen_p32_expected.py, whose Svf restates P3."""
    sys.dont_write_bytecode = True                   # no __pycache__ beside the fixtures
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


P11 = rules("gen_p11_expected")
P32 = rules("gen_p32_expected")
f32, exp2, jf, J = P11.f32, P11.exp2, P11.jf, P11.J
LOG2_10 = P11.LOG2_10
TAU = 2.0 * math.pi                                  # std TAU
SR = 44100
SRF = float(SR)
PPQ = 96
MBPM = 120000                                        # 120 BPM: one bar is 88200 frames
BAR = 4 * PPQ
BUF = 512                                            # the calls the live messages are timed by
FORMAT_VERSION = 9                                   # a project with a synth channel
CLOSE_TOLERANCE = 1e-6                               # gen_p24's LOOP_TOLERANCE
AMPLITUDE_TOLERANCE = 1e-4                           # gen_p5's EQ tolerance
MAX_PITCH = 0.45 * SRF                               # the EQ's MAX_FREQ_FRACTION of the rate


def t2s(tick, mbpm=MBPM):
    """The one tick-to-sample rule."""
    return tick * 60 * SR * 1000 // (mbpm * PPQ)


# ---------- the parameters ----------
ID, NAME = "dawsome.synth", "Dawsome Synth"


def P(key, label, lo, hi, default, unit, whole=False):
    return {"key": key, "label": label, "min": lo, "max": hi, "default": default, "unit": unit,
            "whole": whole}


def osc(n, wave):
    return [P("osc%d_wave" % n, "Osc %d wave" % n, 0.0, 3.0, wave, "", True),
            P("osc%d_width" % n, "Osc %d width" % n, 0.05, 0.95, 0.5, "%"),
            P("osc%d_octave" % n, "Osc %d octave" % n, -2.0, 2.0, 0.0, "oct", True),
            P("osc%d_semitones" % n, "Osc %d semi" % n, -12.0, 12.0, 0.0, "st", True),
            P("osc%d_cents" % n, "Osc %d fine" % n, -100.0, 100.0, 0.0, "ct"),
            P("osc%d_level" % n, "Osc %d level" % n, 0.0, 1.0, 1.0, "%")]


def adsr(prefix, label, attack, decay, sustain, release):
    return [P(prefix + "_attack", label + " attack", 0.0, 1.0, attack, "ms"),
            P(prefix + "_decay", label + " decay", 0.0, 1.0, decay, "ms"),
            P(prefix + "_sustain", label + " sustain", 0.0, 1.0, sustain, "%"),
            P(prefix + "_release", label + " release", 0.0, 1.0, release, "ms")]


PARAMS = (osc(1, 2.0)                                # Saw
          + osc(2, 3.0)                              # Square
          + [P("osc_mix", "Osc mix", 0.0, 1.0, 0.0, "%"),
             P("unison", "Unison", 1.0, 3.0, 1.0, "", True),
             P("detune", "Detune", 0.0, 50.0, 10.0, "ct"),
             P("spread", "Spread", 0.0, 1.0, 0.5, "%"),
             P("noise", "Noise", 0.0, 1.0, 0.0, "%"),
             P("cutoff", "Cutoff", 0.0, 1.0, 1.0, "Hz"),
             P("resonance", "Resonance", 0.0, 1.0, 0.1, "%"),
             P("filter_env", "Env amount", -1.0, 1.0, 0.0, "%"),
             P("key_track", "Key track", 0.0, 1.0, 0.0, "%"),
             P("drive", "Drive", 0.0, 1.0, 0.0, "%")]   # additive: a missing key reads 0
          + adsr("filter", "Filter", 0.0, 0.5, 0.0, 0.5)
          + adsr("amp", "Amp", 0.0, 0.5, 1.0, 0.5)
          + [P("lfo_rate", "LFO rate", 0.0, 1.0, 0.75, "Hz"),
             P("lfo_shape", "LFO shape", 0.0, 3.0, 0.0, "", True),
             P("lfo_pitch", "LFO → pitch", 0.0, 12.0, 0.0, "st"),
             P("lfo_cutoff", "LFO → cutoff", 0.0, 1.0, 0.0, "%"),
             P("glide_ms", "Glide", 0.0, 1000.0, 0.0, "ms"),
             P("volume_db", "Volume", -48.0, 6.0, -6.0, "dB"),
             P("vel_amp", "Velocity → level", 0.0, 1.0, 1.0, "%"),
             P("vel_cutoff", "Velocity → cutoff", 0.0, 1.0, 0.0, "%")])
WAVES = ["Sine", "Triangle", "Saw", "Square"]
# The names a choice's values go by, from its min up (unison's from 1).
VALUE_NAMES = {"osc1_wave": WAVES, "osc2_wave": WAVES,
               "unison": ["1 voice", "2 voices", "3 voices"],
               "lfo_shape": ["Sine", "Triangle", "Square", "Saw"]}
FOUR_LOG2_10 = 4.0 * LOG2_10                         # log2 10000, formed first


def time_seconds(v): return 0.001 * exp2(v * FOUR_LOG2_10)
def time_ms(v): return exp2(v * FOUR_LOG2_10)
cutoff_hz = P32.filter_cutoff_hz                    # 20 * exp2(v * (3 * LOG2_10)), the Filter's
def lfo_rate_hz(v): return 0.01 * exp2(v * (1.0 + 3.0 * LOG2_10))


TIMES = ("filter_attack", "filter_decay", "filter_release", "amp_attack", "amp_decay",
         "amp_release")
SHOWN = dict([(key, time_ms) for key in TIMES] + [("cutoff", cutoff_hz), ("lfo_rate", lfo_rate_hz)])


def values_of(given):
    """Every parameter's value as the synth gets it: the given one or the default, as an f32,
    clamped (all in range here) and whole where it must be (f32::round, half away from 0)."""
    out = {}
    for info in PARAMS:
        v = f32(given.get(info["key"], info["default"]))
        if info["whole"]: v = math.copysign(math.floor(abs(v) + 0.5), v)
        out[info["key"]] = v
    return out


def metadata():
    entry = {"id": ID, "name": NAME, "instrument": True,
             "params": [dict(info, min=f32(info["min"]), max=f32(info["max"]),
                             default=f32(info["default"])) for info in PARAMS],
             "value_names": VALUE_NAMES,
             "shown": {key: [[f32(v), fn(f32(v))] for v in (0.0, 0.5, 1.0)]
                       for key, fn in SHOWN.items()}}
    return entry


# ---------- the synth ----------
K_A = 2.115477217419936                              # log2(1.3 / 0.3), pinned on both sides
K_DR = 13.287856641840545                            # log2(1.0001 / 0.0001)
VOICES = 16
ATTACK, DECAY, SUSTAIN, RELEASE, IDLE = range(5)


def lin(db): return exp2(db / 20.0 * LOG2_10)       # (dB / 20) * LOG2_10
def coefficient(k, v): return exp2(-k / (time_seconds(v) * SRF))


class Adsr:
    """An envelope's settings: its three coefficients and its sustain level."""

    def __init__(self, values, prefix):
        self.a = coefficient(K_A, values[prefix + "_attack"])
        self.d = coefficient(K_DR, values[prefix + "_decay"])
        self.r = coefficient(K_DR, values[prefix + "_release"])
        self.s = values[prefix + "_sustain"]


class Env:
    def __init__(self):
        self.value, self.stage = 0.0, IDLE

    def release(self):
        if self.stage != IDLE: self.stage = RELEASE

    def advance(self, e):
        v = self.value
        if self.stage == ATTACK:
            v = 1.3 * (1.0 - e.a) + v * e.a
            if v >= 1.0: v, self.stage = 1.0, DECAY
        elif self.stage == DECAY:
            v = (e.s - 0.0001) * (1.0 - e.d) + v * e.d
            if v <= e.s: v, self.stage = e.s, SUSTAIN
        elif self.stage == SUSTAIN:
            v = e.s
        elif self.stage == RELEASE:
            v = -0.0001 * (1.0 - e.r) + v * e.r
            if v <= 0.0: v, self.stage = 0.0, IDLE
        self.value = v
        return v


SINE_WAVE, TRIANGLE_WAVE, SAW_WAVE, SQUARE_WAVE = range(4)   # osc n wave's values
UNISON_MAX = 3
PHI = 0.6180339887498949                             # the golden ratio's fraction, (sqrt 5 - 1)
                                                     # / 2, this literal on both sides


def start_phase(slot, k):
    """Where a reset leaves slot `slot`'s unison voice k on both oscillators:
    frac(slot * PHI + k / 3), the slots spread round the cycle by the golden ratio and each
    slot's three unison voices a third apart (3, the most there are: see the notes on U)."""
    return frac(float(slot) * PHI + float(k) / float(UNISON_MAX))


def blep(t, dt):
    """The degree-2 PolyBLEP residual of a unit step at phase 0 (Valimaki and Huovilainen)."""
    if t < dt:
        x = t / dt
        return 2.0 * x - x * x - 1.0                 # ((2 x) - (x x)) - 1
    if t > 1.0 - dt:
        x = (t - 1.0) / dt
        return x * x + 2.0 * x + 1.0                 # ((x x) + (2 x)) + 1
    return 0.0


def frac(x): return x - math.floor(x)


def triangle(t): return 1.0 - 4.0 * abs(frac(t + 0.25) - 0.5)   # in phase with the sine


def square_dc(width):
    """A pulse of width w's mean over a period, 2 w - 1 (the DC-free pulse takes it off), w the f32 widened."""
    return 2.0 * width - 1.0


def wave(kind, t, dt, width, dc):
    """An oscillator's value at phase t, stepping dt a frame ("waves" above). The square
    less its mean `dc` (DC-free); with dc None, the square before it was DC-free (a
    self-check's, which proves the subtraction of 0.0 at width 0.5 leaves every bit)."""
    if kind == SINE_WAVE: return math.sin(TAU * t)
    if kind == TRIANGLE_WAVE: return triangle(t)
    if kind == SAW_WAVE: return 2.0 * t - 1.0 - blep(t, dt)   # ((2 t) - 1) - blep
    naive = 1.0 if t < width else -1.0                  # the width compared as the f32 widened
    square = naive + blep(t, dt) - blep(frac(t + 1.0 - width), dt)   # frac((t + 1) - width)
    return square if dc is None else square - dc


LFO_SINE, LFO_TRIANGLE, LFO_SQUARE, LFO_SAW = range(4)   # lfo_shape's values


def lfo_value(shape, t):
    """The LFO's value at phase t ("LFO" above): no correction, it moves too slowly to alias."""
    if shape == LFO_SINE: return math.sin(TAU * t)
    if shape == LFO_TRIANGLE: return triangle(t)
    if shape == LFO_SQUARE: return 1.0 if t < 0.5 else -1.0
    return 2.0 * t - 1.0


def detune_cents(detune, k, u):
    """det_k = detune * (k / (U - 1) - 0.5), 0 for one voice."""
    return 0.0 if u == 1 else detune * (float(k) / float(u - 1) - 0.5)


def unison_pan(spread, k, u):
    """pan_k = spread * ((2 k) / (U - 1) - 1), 0 for one voice."""
    return 0.0 if u == 1 else spread * (2.0 * float(k) / float(u - 1) - 1.0)


def pan_table(spread):
    """Each unison count's voices' pan gains (left, right): laws::channel_pan_gains of the f32
    nearest pan_k (gen_p11's pan_gains, which restates it), widened."""
    return {u: [P11.pan_gains(f32(unison_pan(spread, k, u))) for k in range(u)]
            for u in range(1, UNISON_MAX + 1)}


def xorshift32(s):
    s ^= (s << 13) & 0xFFFFFFFF
    s ^= s >> 17
    s ^= (s << 5) & 0xFFFFFFFF
    return s


class Osc:
    """One oscillator's settings, as its parameters hold them."""

    def __init__(self, values, n):
        p = "osc%d_" % n
        self.wave, self.width = int(values[p + "wave"]), values[p + "width"]
        self.dc = square_dc(self.width)              # once per settings, as the Rust does
        self.octave, self.semitones = values[p + "octave"], values[p + "semitones"]
        self.cents, self.level = values[p + "cents"], values[p + "level"]


def pitch(key, bend, glide, vibrato, osc, det):
    """p in semitones from A4: (key - 69) + bend, then each term in the order "pitch" above gives, one addition
    at a time."""
    p = float(key - 69) + bend                       # (key - 69) + bend
    p = p + glide
    p = p + vibrato
    p = p + 12.0 * osc.octave
    p = p + osc.semitones
    p = p + osc.cents / 100.0
    p = p + det / 100.0
    return p


def glide_offset(start, m, frames):
    """A glide from `start` semitones on its m-th frame of `frames`: start * (1 - m / N), 0 from
    frame N on (and with no glide, N = 0)."""
    return start * (1.0 - m / frames) if m < frames else 0.0


ENV_SEMITONES = 96.0                                 # filter_env 1 at fenv 1: 8 octaves
DRIVE_RANGE = 5.0                                    # d = 1 + 5 drive
LFO_CUTOFF_SEMITONES = 48.0                          # lfo_cutoff 1 at lfo +-1: 4 octaves
VEL_CUTOFF_SEMITONES = 48.0                          # vel_cutoff 1 from velocity 1 to 0
KEY_TRACK_FROM = 60                                  # key tracking holds C4's cutoff
LOG2_1000 = 3.0 * LOG2_10                            # log2 1000, formed first
WHEEL_TOP = 127.0                                    # the wheel's value at the top: w = 1


def cutoff_semitones(f, fenv, key, lfo, vel):
    """The cutoff's modulation in semitones, its four terms left to right ("filter" above)."""
    return (f["filter_env"] * fenv * ENV_SEMITONES
            + f["key_track"] * float(key - KEY_TRACK_FROM)
            + lfo * f["lfo_cutoff"] * LFO_CUTOFF_SEMITONES
            + f["vel_cutoff"] * (vel - 1.0) * VEL_CUTOFF_SEMITONES)


def modulated_cutoff_hz(f, fenv, key, lfo, vel):
    """fc = 20 * exp2(cutoff * (3 * LOG2_10) + semis / 12), before P3's clamp."""
    return 20.0 * exp2(f["cutoff"] * LOG2_1000 + cutoff_semitones(f, fenv, key, lfo, vel) / 12.0)


FILTER_KEYS = ("cutoff", "filter_env", "key_track", "lfo_cutoff", "vel_cutoff")


def filter_open(f):
    """The filter is skipped (open) at cutoff 1 with nothing moving it."""
    return f["cutoff"] == 1.0 and all(f[k] == 0.0 for k in FILTER_KEYS[1:])


def svf_set(svf, hz, q):
    """P3's coefficients for cutoff hz (clamped to [20, 0.45 sr]) and q; the memory goes on."""
    _, _, svf.k, svf.a1, svf.a2, svf.a3 = P32.svf_coefficients(hz, q)


def svf_clear(svf):
    svf.ic = [[0.0, 0.0], [0.0, 0.0]]


class Voice:
    def __init__(self, index):
        self.env, self.fenv = Env(), Env()
        self.key, self.vel, self.started, self.off_in, self.live = 0, 0.0, 0, None, False
        self.u = 1
        # [osc][unison voice], free-running: set here (a reset) and never at a note's start
        self.phase = [[start_phase(index, k) for k in range(UNISON_MAX)] for _ in (0, 1)]
        self.step = [[0.0] * UNISON_MAX, [0.0] * UNISON_MAX]
        self.noise = index + 1                       # the slot's xorshift32 state, from reset
        self.svf = P32.Svf(20.0, 1.0)                # its coefficients set on its first frame
        self.svf_hz = math.nan                       # the cutoff they are for (none yet)
        self.driven = [0.0, 0.0]                     # the Drive's u1 per side: 0 at reset,
                                                     # kept across notes (free or stolen)
        self.glide_from, self.glide_n, self.m = 0.0, 0, 0
        self.retune = False

    def held(self):
        """Its key held: unreleased, its gate not closing on this frame, sounding since an
        earlier frame."""
        return self.env.stage in (ATTACK, DECAY, SUSTAIN) and self.off_in != 0 and self.m > 0

    def glide(self): return glide_offset(self.glide_from, self.m, self.glide_n)


def spec_held(v): return v.held()


class Synth:
    """The synth as the laws above state it, one frame at a time. `steal` picks the voice a note
    takes when none is free (by the rule: the oldest) and `held` tells a held voice; the
    self-checks swap in wrong ones."""

    def __init__(self, values, steal=None, held=None):
        self.amp = Adsr(values, "amp")
        self.filt = Adsr(values, "filter")
        self.gain = lin(values["volume_db"])
        self.vel_amp = values["vel_amp"]
        self.osc = [Osc(values, 1), Osc(values, 2)]
        mix = values["osc_mix"]
        self.weights = ((1.0 - mix) * self.osc[0].level, mix * self.osc[1].level)
        self.unison, self.detune = int(values["unison"]), values["detune"]
        self.pans = pan_table(values["spread"])
        self.noise = values["noise"]
        self.f = {k: values[k] for k in FILTER_KEYS}
        self.open = filter_open(self.f)
        self.open_hz = cutoff_hz(values["cutoff"])
        self.q = P32.filter_q(values["resonance"])
        self.drive = values["drive"]
        self.drive_d = 1.0 + self.drive * DRIVE_RANGE   # d = 1 + 5 drive: 6 at the top
        self.drive_norm = math.tanh(self.drive_d)       # tanh(d), once per settings
        # Auto gain: the Distortion effect's, at the drive in dB whose gain is d
        self.drive_makeup = P32.distortion_makeup(20.0 * math.log10(self.drive_d), True)
        self.lfo_shape = int(values["lfo_shape"])
        self.lfo_step = lfo_rate_hz(values["lfo_rate"]) / SRF
        self.lfo_pitch = values["lfo_pitch"]
        self.glide_n = math.floor(values["glide_ms"] / 1000.0 * SRF + 0.5)
        self.voices = [Voice(i) for i in range(VOICES)]
        self.bend, self.wheel, self.lfo_t = 0.0, 0.0, 0.0
        self.starts = 0
        self.last = None                             # the slot of the note started last
        self.steal = steal or (lambda voices: min(range(VOICES), key=lambda i: voices[i].started))
        self.held = held or spec_held
        self.trace = None                            # voice 0's envelope per frame, when traced
        self.cutoffs = None                          # voice 0's cutoff per frame, when traced
        self.phases_at_start = False                 # the phases' law before (a self-check's)

    def retune_voice(self, v, vibrato):
        glide = v.glide()
        v.step = [[min(440.0 * exp2(pitch(v.key, self.bend, glide, vibrato, self.osc[o],
                                          detune_cents(self.detune, k, v.u)) / 12.0),
                       MAX_PITCH) / SRF if k < v.u else 0.0
                   for k in range(UNISON_MAX)] for o in (0, 1)]

    def start(self, key, vel, length, live):
        # The glide first, from the voices as they are before this note takes one.
        glide_from, glide_n = 0.0, 0
        if self.glide_n > 0 and self.last is not None and any(self.held(v) for v in self.voices):
            prev = self.voices[self.last]
            glide_from, glide_n = (float(prev.key) + prev.glide()) - float(key), self.glide_n
        free = [i for i, v in enumerate(self.voices) if v.env.stage == IDLE]
        slot = free[0] if free else self.steal(self.voices)
        v = self.voices[slot]
        if free:                                     # a free voice starts its filter from rest
            v.fenv = Env()
            svf_clear(v.svf)
            v.svf_hz = math.nan
        v.key, v.vel, v.started, v.off_in, v.live = key, vel, self.starts, length, live
        self.starts += 1
        v.u = self.unison                            # the count at the note's start
        if self.phases_at_start:                     # the law before: every note from k / U
            v.phase = [[float(k) / float(v.u) if k < v.u else 0.0 for k in range(UNISON_MAX)]
                       for _ in (0, 1)]
        v.glide_from, v.glide_n, v.m = glide_from, glide_n, 0
        v.retune = True
        v.env.stage = ATTACK                         # from the value it has
        v.fenv.stage = ATTACK
        self.last = slot

    def off(self, key):
        for v in self.voices:
            if v.live and v.key == key and v.env.stage not in (RELEASE, IDLE):
                v.env.release()
                v.fenv.release()

    def retune_all(self):
        for v in self.voices: v.retune = True

    def set_bend(self, value):
        self.bend = value / 4096.0
        self.retune_all()

    def set_wheel(self, value):
        self.wheel = value / WHEEL_TOP
        self.retune_all()

    def frame(self):
        total = [0.0, 0.0]
        a, b = self.weights
        depth = self.lfo_pitch + self.wheel          # lfo_pitch + w
        used = depth != 0.0 or (not self.open and self.f["lfo_cutoff"] != 0.0)
        lfo = lfo_value(self.lfo_shape, self.lfo_t) if used else 0.0
        vibrato = lfo * depth
        for i, v in enumerate(self.voices):
            if v.env.stage == IDLE: continue
            if v.off_in == 0:
                v.off_in = None
                v.env.release()
                v.fenv.release()
            env = v.env.advance(self.amp)
            fenv = v.fenv.advance(self.filt)
            if self.trace is not None and i == 0: self.trace.append(env)
            if v.retune or (v.glide_n > 0 and v.m <= v.glide_n) or depth != 0.0:
                self.retune_voice(v, vibrato)
                v.retune = False
            values = [[0.0] * v.u, [0.0] * v.u]
            for o in (0, 1):
                osc = self.osc[o]
                for k in range(v.u):
                    t, dt = v.phase[o][k], v.step[o][k]
                    values[o][k] = wave(osc.wave, t, dt, osc.width, osc.dc)
                    t += dt
                    if t >= 1.0: t -= 1.0
                    v.phase[o][k] = t
            v.noise = xorshift32(v.noise)
            nz = v.noise / 2147483648.0 - 1.0        # s / 2^31 - 1
            if v.off_in is not None: v.off_in -= 1
            hz = self.open_hz if self.open else modulated_cutoff_hz(self.f, fenv, v.key, lfo, v.vel)
            if self.cutoffs is not None and i == 0: self.cutoffs.append(hz)
            if not hz == v.svf_hz:                   # bits: the cutoff is never 0 or NaN
                svf_set(v.svf, hz, self.q)
                v.svf_hz = hz
            gain = 1.0 - self.vel_amp * (1.0 - v.vel)
            pans = self.pans[v.u]
            for s in (0, 1):
                acc = 0.0
                for k in range(v.u):
                    acc = acc + pans[k][s] * (a * values[0][k] + b * values[1][k])
                osc_s = acc / float(v.u) + self.noise * nz
                y = v.svf.process(s, osc_s, P32.LOW)
                heard = osc_s if self.open else y
                if self.drive == 0.0:                # not shaped at all: the bits before the Drive
                    v.driven[s] = heard              # u1 follows the side (d is 1 here)
                else:
                    u = self.drive_d * heard         # d x
                    shaped = P32.adaa_tanh(u, v.driven[s])
                    v.driven[s] = u
                    heard = shaped / self.drive_norm * self.drive_makeup   # (y / tanh d) * g
                total[s] += heard * env * gain       # (heard * env) * gain
            v.m += 1
        self.lfo_t += self.lfo_step
        if self.lfo_t >= 1.0: self.lfo_t -= 1.0
        return [0.0 + f32(x * self.gain) for x in total]


def play(values, events, frames, steal=None, trace=None, held=None, cutoffs=None, law=None):
    """The synth's output over `frames` frames for `events` (frame, kind, args) in arrival
    order, as the render writes it: (left, right), -0.0 made +0.0 by the master's bus. `law`,
    when given, changes the synth before it plays (a self-check's older law)."""
    synth = Synth(values, steal, held)
    if law: law(synth)
    synth.trace = trace
    synth.cutoffs = cutoffs
    order = sorted(range(len(events)), key=lambda i: (events[i][0], i))
    left, right, k = [], [], 0
    for n in range(frames):
        while k < len(order) and events[order[k]][0] == n:
            _, kind, args = events[order[k]]
            if kind == "note": synth.start(*args, live=False)
            elif kind == "on": synth.start(*args, None, live=True)
            elif kind == "off": synth.off(*args)
            elif kind == "bend": synth.set_bend(*args)
            elif kind == "wheel": synth.set_wheel(*args)
            k += 1
        l, r = synth.frame()
        left.append(l)
        right.append(r)
    return left, right


# ---------- the pattern cases ----------
def project(params, notes, mbpm=MBPM):
    given = {k: float(jf(v)) for k, v in sorted(params.items())}
    return {"format_version": FORMAT_VERSION, "sample_rate": SR, "tempo_mbpm": mbpm, "ppq": PPQ,
            "next_channel_id": 2,
            "channels": [{"id": 1, "name": NAME, "kind": "synth",
                          "processor": {"id": ID, "params": given},
                          "volume": 1.0, "pan": 0.0, "mute": False}],
            "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": notes}]}


def note(tick, length, key, velocity=1.0):
    n = {"channel": 1, "tick": tick, "length": length, "velocity": velocity}
    if key != 60: n["key"] = key
    return n


def note_events(notes, mbpm):
    """The Note events the player sends: each at its tick's frame, sounding until its end's."""
    return [(t2s(n["tick"], mbpm), "note",
             (n.get("key", 60), f32(n["velocity"]), t2s(n["tick"] + n["length"], mbpm)
              - t2s(n["tick"], mbpm)))
            for n in notes]


SINE = {"osc1_wave": 0.0}                            # the first cases play osc 1's sine
# The Saw Lead factory preset (assets/synth-presets/saw-lead.json), restated.
SAW_LEAD = {"osc2_wave": 2.0, "osc2_cents": 7.0, "osc_mix": 0.5, "unison": 2.0, "detune": 12.0,
            "cutoff": 0.7, "resonance": 0.25, "filter_env": 0.3, "glide_ms": 60.0}
DRIVE_NOTES = [note(0, PPQ, 57), note(2 * PPQ, PPQ, 64)]
STEAL_MBPM = 275625                                  # one tick is exactly 100 frames
STEAL_KEYS = [33, 45] + list(range(69, 85))          # 55 Hz, 110 Hz, then 440 Hz upward
PATTERN_CASES = {
    "sine": (SINE, [note(0, BAR, 69)], MBPM),
    "adsr": (dict(SINE, amp_attack=0.25, amp_decay=0.5, amp_sustain=0.5, amp_release=0.5),
             [note(0, PPQ // 2, 69)], MBPM),
    "steal": (SINE, [note(i, BAR - i, key) for i, key in enumerate(STEAL_KEYS)], STEAL_MBPM),
    # The oscillators. The defaults: osc 1 a saw (osc 2's square mixed out), A5 for the bar.
    "saw": ({}, [note(0, BAR, 81)], MBPM),
    # Both squares at their extreme widths, mixed 0.7 and 0.3 (equal weights would sum the
    # same with the widths swapped, hiding a duty turned round): A7 (3520 Hz, dt 0.0798, so
    # the 0.05-wide pulse's two corrections overlap) for half the bar, then A3 (220 Hz).
    "square_width": ({"osc1_wave": 3.0, "osc1_width": 0.05, "osc2_width": 0.95, "osc_mix": 0.3},
                     [note(0, BAR // 2, 105), note(BAR // 2, BAR // 2, 57)], MBPM),
    # The tuning, a level and the mix: osc 1 a sine an octave up, 3 semitones down, 30 cents
    # up (9.3 semitones: 752.7 Hz); osc 2 a triangle an octave down (220 Hz) at level 0.6;
    # mix 0.3.
    "tuning": ({"osc1_wave": 0.0, "osc1_octave": 1.0, "osc1_semitones": -3.0,
                "osc1_cents": 30.0, "osc2_wave": 1.0, "osc2_octave": -1.0, "osc2_level": 0.6,
                "osc_mix": 0.3}, [note(0, BAR, 69)], MBPM),
    # Three saws 50 cents wide, spread fully, and noise.
    "unison": ({"unison": 3.0, "detune": 50.0, "spread": 1.0, "noise": 0.2},
               [note(0, BAR, 69)], MBPM),
    # The same three voices as sines, no noise: the detune and the spread by their tones.
    "unison_tones": ({"osc1_wave": 0.0, "unison": 3.0, "detune": 50.0, "spread": 1.0},
                     [note(0, BAR, 69)], MBPM),
    # Noise alone (osc 1's level 0), exact: note A on slot 0 (seed 1), B on slot 1 (seed 2)
    # while A sounds, and C on slot 0 again after both are free, its noise going on from where
    # slot 0's stopped (not seeded again).
    "noise": ({"osc1_level": 0.0, "noise": 1.0},
              [note(0, PPQ, 60), note(PPQ // 4, 3 * PPQ // 4, 64), note(2 * PPQ, PPQ, 67)],
              MBPM),
    # The filter, the LFO and glide. The filter: the saw at A3 for half the bar through Q 20, the filter envelope's
    # full 8 octaves and key tracking 1:1 (the cutoff at 632 Hz 3 semitones down once the
    # envelope has decayed to its sustain, 0).
    "filter": ({"cutoff": 0.5, "resonance": 1.0, "filter_env": 1.0, "key_track": 1.0},
               [note(0, BAR // 2, 57)], MBPM),
    # Key tracking at 50% and velocity -> cutoff at 100% by their tones: three sines, each
    # through its own voice's filter (vel_amp 0: velocity moves the cutoff alone).
    "cutoff_tones": ({"osc1_wave": 0.0, "cutoff": 0.5, "resonance": 0.5, "key_track": 0.5,
                      "vel_cutoff": 1.0, "vel_amp": 0.0},
                     [note(0, BAR, 57), note(0, BAR, 69), note(0, BAR, 81, 0.5)], MBPM),
    # The triangle LFO at 20 Hz, 2 semitones to the pitch of the saw: exact (no libm).
    "lfo_pitch": ({"lfo_shape": 1.0, "lfo_pitch": 2.0, "lfo_rate": 1.0}, [note(0, BAR, 69)], MBPM),
    # The square LFO at 0.956 Hz, +-2 octaves to the cutoff (880 Hz) of a 220 Hz sine: 3520 Hz
    # while the LFO is high (it starts there), 220 Hz (on the tone, ringing Q) while it is low.
    "lfo_cutoff": ({"osc1_wave": 0.0, "cutoff": P32.cutoff_for(880.0), "resonance": 0.5,
                    "lfo_cutoff": 0.5, "lfo_shape": 2.0, "lfo_rate": 0.6},
                   [note(0, BAR, 57)], MBPM),
    # The glide: C4 held one tick into C5's start, so C5 glides 12 semitones up over 200 ms;
    # a 1 ms release, so C5 sounds alone soon after C4 lets go.
    "glide": ({"osc1_wave": 0.0, "glide_ms": 200.0, "amp_release": 0.0},
              [note(0, PPQ + 1, 60), note(PPQ, 2 * PPQ, 72)], MBPM),
    # The same notes, C4 ending on the frame C5 starts: not held there, so no glide.
    "glide_separated": ({"osc1_wave": 0.0, "glide_ms": 200.0, "amp_release": 0.0},
                        [note(0, PPQ, 60), note(PPQ, 2 * PPQ, 72)], MBPM),
    # The free-running phases. The same note twice on one slot, the first freed (its 100 ms release
    # over) long before the second starts: the second goes on from the phase the first left,
    # so it does not play the first's frames again.
    "repeat": ({}, [note(0, PPQ, 60), note(2 * PPQ, PPQ, 60)], MBPM),
    # Six Init notes struck at once: each slot starts its saw where a reset left it, spread
    # round the cycle, so their first cycles stack less.
    "chord": ({}, [note(0, PPQ, key) for key in (48, 52, 55, 60, 64, 67)], MBPM),
    # Ruling 28: the Saw Lead factory preset's voice (two detuned saws, two unison voices
    # spread, the filter at 0.7 with its envelope) driven half way and fully, A3 then E4 on the
    # same slot after A3 is free (the Drive's memory goes on from A3's last frame).
    "drive_half": (dict(SAW_LEAD, drive=0.5), DRIVE_NOTES, MBPM),
    "drive_full": (dict(SAW_LEAD, drive=1.0), DRIVE_NOTES, MBPM),
    # By hand: a sine at A3 driven at 0.2 (d = 2), filter open: its harmonics are tanh(2 sin)'s
    # over tanh(2).
    "drive_tone": ({"osc1_wave": 0.0, "drive": 0.2}, [note(0, BAR, 57)], MBPM),
    # Drive written as 0 in the file: the saw case's bits (the voice not shaped at all).
    "drive_zero": ({"drive": 0.0}, [note(0, BAR, 81)], MBPM),
}
STEREO = ("unison", "unison_tones", "drive_half", "drive_full")   # the cases whose sides differ
# The cases whose bits no libm decides, compared by hash: no sine wave, and the pan law's sin
# only at pan 0 (one voice) or +-1 (spread 1), where f32(sqrt 2 sin(pi/4)) = 1, f32(sqrt 2
# sin(pi/2)) = f32(sqrt 2) and sin(0) = 0 whatever libm's last bit (Windows' CI runs them too).
EXACT = ("saw", "square_width", "unison", "noise", "lfo_pitch", "repeat", "chord", "drive_zero")

# ---------- the live cases ----------
def on(key, v=127): return [0x90, key, v]
def off(key): return [0x80, key, 0]
def pedal(v): return [0xB0, 64, v]
def wheel(v): return [0xB0, 1, v]
def bend(b):
    raw = b + 8192
    return [0xE0, raw & 0x7F, raw >> 7]


LIFT_CALL = 53                                       # 0.5 s after the note off (call 10)
LIVE_CASES = {
    "pedal": [(2, pedal(127)), (4, on(60)), (10, off(60)), (LIFT_CALL, pedal(0))],
    "held": [(4, on(60)), (LIFT_CALL, off(60))],
    "unreleased": [(4, on(60))],
    "bend": [(4, on(69)), (20, bend(8191)), (40, bend(-8192)), (50, bend(0)), (60, off(69))],
    "wheel": [(4, on(69)), (20, wheel(100)), (120, wheel(0)), (160, off(69))],
}
LIVE_PROJECT = "p33_live.json"


def live_events(script):
    """What the player hands the synth for `script`'s messages (gen_p22's rules restated)."""
    events, down, deferred, wheel_at = [], False, [], 0
    for call, m in script:
        frame = call * BUF
        status, data = m[0] & 0xF0, m[1:]
        if status == 0x90 and data[1] > 0:
            events.append((frame, "on", (data[0], f32(data[1] / 128.0))))
        elif status in (0x80, 0x90):
            if down: deferred.append(data[0])
            else: events.append((frame, "off", (data[0],)))
        elif status == 0xB0 and data[0] == 64:
            now = data[1] >= 64
            if down and not now:
                events.extend((frame, "off", (key,)) for key in deferred)
                deferred.clear()
            down = now
        elif status == 0xB0 and data[0] == 1:
            if data[1] != wheel_at:                  # sent on when it moves
                events.append((frame, "wheel", (data[1],)))
                wheel_at = data[1]
        elif status == 0xE0:
            events.append((frame, "bend", (data[0] + 128 * data[1] - 8192,)))
    return events


def events_json(script):
    return {"buffer": BUF, "events": [{"call": c, "bytes": m} for c, m in script]}


# ---------- the renders and their checks ----------
FRAMES = t2s(BAR)                                    # 88200: one bar at 120 BPM
_RENDERS = {}


def case_inputs(case):
    """(values, events, frames) of `case`."""
    if case in PATTERN_CASES:
        params, notes, mbpm = PATTERN_CASES[case]
        return values_of(params), note_events(notes, mbpm), t2s(BAR, mbpm)
    return values_of(SINE), live_events(LIVE_CASES[case]), FRAMES


def render(case):
    if case not in _RENDERS:
        _RENDERS[case] = play(*case_inputs(case))
    return _RENDERS[case]


def sha256_of(left, right):
    data = struct.pack("<%df" % (2 * len(left)), *[x for pair in zip(left, right) for x in pair])
    return hashlib.sha256(data).hexdigest()


def amplitude(y, hz, lo, hi):
    """gen_p5's: projection on sin/cos over [lo, hi), a whole number of periods."""
    s = sum(y[n] * math.sin(TAU * hz * n / SRF) for n in range(lo, hi))
    c = sum(y[n] * math.cos(TAU * hz * n / SRF) for n in range(lo, hi))
    return 2.0 * math.hypot(s, c) / (hi - lo)


def hann_amplitude(y, hz, lo, hi):
    """The amplitude of the `hz` tone in y[lo:hi) under a periodic Hann window:
    2 |sum w_n y_n e^(-j w n)| / sum w_n, w_n = 0.5 - 0.5 cos(TAU (n - lo) / (hi - lo))."""
    width = hi - lo
    total, weight = 0j, 0.0
    for n in range(lo, hi):
        w = 0.5 - 0.5 * math.cos(TAU * (n - lo) / width)
        total += w * y[n] * cmath.exp(complex(0.0, -TAU * hz * n / SRF))
        weight += w
    return 2.0 * abs(total) / weight


def pitch_hz(key, b=0): return min(440.0 * exp2((float(key - 69) + b / 4096.0) / 12.0), MAX_PITCH)


def probes_around(*frames, span=16):
    return [k for f in frames for k in range(f - span, f + span)]


BASE_PROBES = list(range(512)) + list(range(512, FRAMES, 32))
LIFT = LIFT_CALL * BUF
SINE_WINDOW = (4410, FRAMES)                         # 836 periods of 440 Hz, all sustain
STEAL_WINDOW = (2000, t2s(BAR, STEAL_MBPM))          # after note 18's start and attack
TONE_TOLERANCE = AMPLITUDE_TOLERANCE


TUNING_WINDOW = (4410, FRAMES)                       # all sustain
UNISON_WINDOW = (4410, FRAMES)
LIN_6 = 10.0 ** (-6.0 / 20.0)                        # the default volume, by hand
TRIANGLE_FUNDAMENTAL = 8.0 / math.pi ** 2            # a triangle's first harmonic, by hand
DIFFER_BY = 0.01                                     # "the sides differ": by more than this
MEAN_TOLERANCE = AMPLITUDE_TOLERANCE
SQUARE_WINDOWS = [(2205, 44100), (52920, FRAMES)]    # 19 and 16 times 2205 frames: whole
                                                     # periods of A7 and A3, after A7's attack
                                                     # and A3's start, each in its note


def mean_answer(y, lo, hi, expected):
    assert (hi - lo) % 2205 == 0
    return {"window": [lo, hi], "expected": expected, "tolerance": MEAN_TOLERANCE,
            "measured": sum(y[lo:hi]) / (hi - lo)}


# ---------- the filter's, the LFO's and glide's answers by hand ----------
# Every value here is worked out from the voice's laws with math.pow and the libm, never by the
# voice above: the pitch paths a phase follows, the cutoffs the filter sits at.
CROSSING_TOLERANCE = 1                               # the glide's zero crossings: +-1 frame
Q_HALF = 0.5 * 40.0 ** 0.5                           # resonance 0.5: Q = 0.5 * 40^0.5
FC_HALF = 20.0 * 1000.0 ** 0.5                       # cutoff 0.5: 632.46 Hz


def note_hz(semitones): return 440.0 * 2.0 ** (semitones / 12.0)   # from A4
def lfo_rate_by_hand(v): return 0.01 * 2000.0 ** f32(v)             # 0.01 Hz * 2000^v
def triangle_by_hand(t): return 2.0 / math.pi * math.asin(math.sin(TAU * t))


def crossings_of(y, lo, hi):
    """The upward zero crossings in y[lo:hi): the frames n with y[n - 1] < 0 <= y[n]."""
    return [n for n in range(max(lo, 1), hi) if y[n - 1] < 0.0 <= y[n]]


def crossings_by_hand(start, hz_at, end, at=0.0, phase=0.0):
    """A wave whose phase is `phase` on frame `start` and moves hz_at(n) / sr on frame n: the
    frames before `end` on which its phase first reaches k + at, k whole (at 0 a sine's upward
    zero crossings, at 1/2 the saw's)."""
    out, k = [], at + math.ceil(phase - at)
    for n in range(start, end):
        if phase >= k:
            out.append(n)
            k += 1.0
        phase += hz_at(n) / SRF
    return out


def crossing_answer(y, by_hand, lo, hi):
    """`by_hand`'s crossings in [lo, hi), the window's ends moved half way to the crossings
    beside them (so none sits near an end), and the most the render's own there are off by
    ("measured": None when they are not as many)."""
    inside = [i for i, n in enumerate(by_hand) if lo <= n < hi]
    first, last = inside[0], inside[-1]
    lo = (by_hand[first - 1] + by_hand[first]) // 2
    hi = (by_hand[last] + by_hand[last + 1]) // 2
    frames = by_hand[first:last + 1]
    got = crossings_of(y, lo, hi)
    off = max(abs(g - w) for g, w in zip(got, frames)) if len(got) == len(frames) else None
    return {"window": [lo, hi], "frames": frames, "tolerance": CROSSING_TOLERANCE,
            "measured": off}


GLIDE_START = t2s(PPQ)                               # C5's first frame, 22050
C5_PHASE = PHI                                       # C5 takes slot 1 (C4 sounds on 0), which a
                                                     # reset left at frac(1 PHI + 0)
GLIDE_FRAMES = 8820                                  # 200 ms at 44.1 kHz
GLIDE_CUT = t2s(PPQ + 1)                             # C4's gate closes in "glide", 22279
GLIDE_END = t2s(3 * PPQ)                             # C5's gate closes, 66150


def glide_hz(n, hz_linear=False):
    """C5's frequency on frame n: from C4 by 72 - 12 (1 - m / N) semitones on its m-th frame
    (the glide law); `hz_linear`, the wrong form: a straight line in Hz from C4 to C5."""
    m = n - GLIDE_START
    if m >= GLIDE_FRAMES: return note_hz(3.0)
    if hz_linear: return note_hz(-9.0) + (note_hz(3.0) - note_hz(-9.0)) * m / GLIDE_FRAMES
    return note_hz(3.0 - 12.0 * (1.0 - m / GLIDE_FRAMES))


def lfo_pitch_hz(n, phase=0.0):
    """lfo_pitch's frequency on frame n: A4 swung 2 semitones by the triangle at 20 Hz, its
    phase `phase` at reset (0, where a reset leaves it)."""
    t = math.fmod(phase + n * lfo_rate_by_hand(1.0) / SRF, 1.0)
    return note_hz(2.0 * triangle_by_hand(t))


WHEEL_ON, WHEEL_OFF = 20 * BUF, 120 * BUF            # the wheel at 100, then back at 0
WHEEL_DEPTH = 100.0 / 127.0                          # w: semitones of vibrato
NOTE_ON, NOTE_OFF = 4 * BUF, 160 * BUF


def wheel_hz(n, depth=WHEEL_DEPTH):
    """The wheel case's frequency on frame n: A4, swung by the sine LFO (2.99 Hz, phase 0 at
    reset) times w semitones while the wheel is up."""
    if not WHEEL_ON <= n < WHEEL_OFF: return 440.0
    lfo = math.sin(TAU * math.fmod(n * lfo_rate_by_hand(0.75) / SRF, 1.0))
    return note_hz(lfo * depth)


def lfo_cutoff_windows():
    """The square LFO's halves inside the bar, each from 50 ms after its jump (the filter's
    ring at Q 3.16 is gone by then), with the cutoff it holds there by hand: the LFO high
    (+2 octaves) from its start, then low (-2), and so on."""
    step = lfo_rate_by_hand(0.6) / SRF
    jumps = [0] + [math.ceil(k * 0.5 / step) for k in range(1, 4)]
    base = 20.0 * 1000.0 ** PATTERN_CASES["lfo_cutoff"][0]["cutoff"]
    out = []
    for k, (a, b) in enumerate(zip(jumps, jumps[1:])):
        lo, hi = a + 2205, min(b, FRAMES)
        if hi - lo >= 4410: out.append((lo, hi, base * 4.0 ** (1 if k % 2 == 0 else -1)))
    return out


def cutoff_tones_by_hand():
    """Each note of cutoff_tones: (hz, its cutoff by hand): 632.46 Hz moved 0.5 (key - 60)
    semitones by key tracking and 48 (vel - 1) by velocity."""
    out = []
    for n in PATTERN_CASES["cutoff_tones"][1]:
        key, vel = n.get("key", 60), n["velocity"]
        out.append((note_hz(key - 69.0), FC_HALF * 2.0 ** ((0.5 * (key - 60) + 48.0 * (vel - 1.0))
                                                         / 12.0)))
    return out


def lowpass_tone(y, hz, fc, lo, hi):
    """A sine of lin(-6) at hz through the low-pass at fc, Q 3.16 (the prewarped analog closed
    form gen_p32 states P3's steady state by), as a tone check."""
    return tone(y, hz, lo, hi, LIN_6 * P32.svf_closed(P32.LOW, hz, fc, Q_HALF))


def b3_answer(case, a, y, asserts):
    """The filter, LFO and glide checks of `case` into its answer `a`; the frames to probe around."""
    extra = []
    if case == "filter":
        extra += probes_around(t2s(BAR // 2))
    if case == "cutoff_tones":
        lo, hi = 4410, FRAMES
        a["tones"] = [lowpass_tone(y, hz, fc, lo, hi) for hz, fc in cutoff_tones_by_hand()]
        asserts.append("tones")
    if case == "lfo_pitch":
        hand = crossings_by_hand(0, lfo_pitch_hz, FRAMES, at=0.5)
        a["crossings"] = crossing_answer(y, hand, 500, FRAMES - 100)
        asserts.append("crossings")
    if case == "lfo_cutoff":
        windows = lfo_cutoff_windows()
        a["tones"] = [lowpass_tone(y, 220.0, fc, lo, hi) for lo, hi, fc in windows]
        extra += probes_around(*(lo - 2205 for lo, _, _ in windows[1:]))
        asserts.append("tones")
    if case == "wheel":
        hand = crossings_by_hand(NOTE_ON, wheel_hz, NOTE_OFF)
        a["crossings"] = crossing_answer(y, hand, NOTE_ON + 200, NOTE_OFF - 200)
        extra += probes_around(WHEEL_ON, WHEEL_OFF, NOTE_OFF)
        asserts.append("crossings")
    if case in ("glide", "glide_separated"):
        hz_at = glide_hz if case == "glide" else (lambda n: note_hz(3.0))
        hand = crossings_by_hand(GLIDE_START, hz_at, GLIDE_END, phase=C5_PHASE)
        a["crossings"] = crossing_answer(y, hand, GLIDE_CUT + 100, GLIDE_END - 100)
        extra += probes_around(GLIDE_START, GLIDE_CUT, GLIDE_START + GLIDE_FRAMES, GLIDE_END)
        asserts.append("crossings")
    return extra


REPEAT_GATE = t2s(PPQ)                               # each of repeat's notes: 22050 frames
REPEAT_SECOND = t2s(2 * PPQ)                         # the second's first frame, 44100
CHORD_WINDOW = (0, 2205)                             # the chord's first 50 ms: under the old law
                                                     # its saws stack at the chord's common
                                                     # period (~31 ms for C E G), after the attack


def peak(y, lo, hi): return max(abs(v) for v in y[lo:hi])


def b5_answer(case, a, y, asserts):
    """The pulse, phase and Drive checks of `case` into its answer `a`; the frames to probe around."""
    extra = []
    if case == "repeat":
        n = REPEAT_GATE
        a["differs"] = {"first": [0, n], "second": [REPEAT_SECOND, REPEAT_SECOND + n],
                        "by_more_than": DIFFER_BY,
                        "measured": max(abs(y[k] - y[REPEAT_SECOND + k]) for k in range(n))}
        extra += probes_around(REPEAT_GATE, REPEAT_SECOND, REPEAT_SECOND + n)
        asserts.append("differs")
    if case == "chord":
        lo, hi = CHORD_WINDOW
        stacked = play(*case_inputs(case), law=phases_at_start)[0]
        a["peak_below"] = {"window": [lo, hi], "before": peak(stacked, lo, hi),
                           "measured": peak(y, lo, hi)}
        extra += probes_around(t2s(PPQ))             # the notes' gates close
        asserts.append("peak_below")
    if case in ("drive_half", "drive_full"):
        extra += probes_around(t2s(PPQ), t2s(2 * PPQ), t2s(3 * PPQ))
    if case == "drive_tone":
        lo, hi = DRIVE_TONE_WINDOW
        a["tones"] = [tone(y, 220.0 * n, lo, hi, LIN_6 * tanh_harmonic(DRIVE_TONE_D, n)
                           / math.tanh(DRIVE_TONE_D) * DRIVE_TONE_GAIN) for n in (1, 3)]
        asserts.append("tones")
    if case == "drive_zero":
        a["same_as"] = "saw"
        asserts.append("same_as")
    return extra


DRIVE_TONE_D = 1.0 + 5.0 * 0.2                       # d at drive 0.2, by hand: 2
DRIVE_TONE_DB = 20.0 * math.log10(2.0)               # that gain in dB: 6.02
# Auto gain by hand (the Distortion's law): 10^(-(D / (1 + D / 40)) / 20), -5.23 dB
DRIVE_TONE_GAIN = 10.0 ** (-(DRIVE_TONE_DB / (1.0 + DRIVE_TONE_DB / 40.0)) / 20.0)
DRIVE_TONE_WINDOW = (4410, FRAMES)                   # all sustain


def tanh_harmonic(d, n, points=4096):
    """By hand: the nth harmonic of tanh(d sin x), its sine series' b_n =
    (1/pi) integral over a period of tanh(d sin x) sin(n x) dx, by the trapezoid rule (a smooth
    periodic integrand: exact to the last digits long before 4096 points)."""
    return 2.0 / points * math.fsum(math.tanh(d * math.sin(TAU * j / points))
                                    * math.sin(n * TAU * j / points) for j in range(points))


def release_zero(case):
    """The frame the release reaches 0 on (the envelope's value 0 and the voice free)."""
    trace = []
    play(*case_inputs(case), trace=trace)
    one = trace.index(1.0)
    zero = next(k for k in range(one, len(trace)) if trace[k] == 0.0)
    return one, zero


def case_answer(case):
    values, events, frames = case_inputs(case)
    left, right = render(case)
    y = left
    live = case in LIVE_CASES
    exact = case in EXACT
    a = {"live": live, "params": values, "frames": frames,
         "tier": "exact" if exact else "close", "sha256": sha256_of(left, right)}
    if live:
        a["project"], a["events"] = LIVE_PROJECT, "p33_%s_events.json" % case
    else:
        a["fixture"] = "p33_%s.json" % case
    extra = []
    # The probes first, so a miss names its frame; an exact case's hash then pins every bit.
    asserts = ["probes", "sha256"] if exact else ["probes"]
    if case in STEREO:
        a["sides_differ"] = {"by_more_than": DIFFER_BY,
                             "measured": max(abs(l - r) for l, r in zip(left, right))}
        asserts.append("sides_differ")
    else:
        asserts.append("left_equals_right")
    if case == "sine":
        lo, hi = SINE_WINDOW
        assert (hi - lo) * 440 % SR == 0
        a["amplitude"] = {"hz": 440.0, "window": [lo, hi], "expected": lin(-6.0),
                          "tolerance": AMPLITUDE_TOLERANCE, "measured": amplitude(y, 440.0, lo, hi)}
        asserts.append("amplitude")
    if case == "adsr":
        one, zero = release_zero(case)
        a["env_one_at"], a["release_at"], a["zero_from"] = one, t2s(PPQ // 2), zero
        extra += probes_around(one, a["release_at"], zero)
        asserts.append("zero_from")
    if case == "steal":
        lo, hi = STEAL_WINDOW
        starts = [t2s(i, STEAL_MBPM) for i in range(len(STEAL_KEYS))]
        extra += probes_around(starts[16], starts[17])
        gone = [pitch_hz(key) for key in STEAL_KEYS[:2]]
        kept = [pitch_hz(key) for key in STEAL_KEYS[2:]]
        a["tones"] = ([tone(y, hz, lo, hi, 0.0) for hz in gone]
                      + [tone(y, hz, lo, hi, lin(-6.0)) for hz in kept])
        asserts.append("tones")
    if case == "pedal":
        a["same_as"] = "held"
        a["prefix_of"] = {"case": "unreleased", "frames": LIFT}
        extra += probes_around(LIFT)
        asserts += ["same_as", "prefix_of"]
    if case == "held":
        extra += probes_around(LIFT)
    if case == "bend":
        spans = [(20, 40, 8191), (40, 50, -8192), (50, 60, 0)]
        gain = lin(-6.0) * (1.0 - 1.0 * (1.0 - f32(127 / 128.0)))
        a["tones"] = [tone(y, pitch_hz(69, b), c0 * BUF, c1 * BUF, gain) for c0, c1, b in spans]
        extra += probes_around(*(c * BUF for c in (20, 40, 50, 60)))
        asserts.append("tones")
    if case == "square_width":
        extra += probes_around(t2s(BAR // 2))
        # By hand: a pulse of width w averages 2w - 1 over whole periods (its corrections
        # average 0), and the DC-free pulse takes that off, so each averages 0 and
        # so does the mix, each half of the bar (A7's periods are 2205 / 176 frames, A3's
        # 2205 / 11). With the mean left in, the mix averaged 0.7 (2 * 0.05 - 1) + 0.3 (2 * 0.95 - 1)
        # = -0.36 of the volume (b2_checks holds that law apart).
        a["means"] = [mean_answer(left, lo, hi, 0.0) for lo, hi in SQUARE_WINDOWS]
        asserts.append("means")
    if case == "tuning":
        # By hand: osc 1 at 440 * 2^((12 - 3 + 0.3) / 12), (1 - 0.3) * 1 of the volume; osc 2's
        # triangle at 220 Hz, its fundamental 8/pi^2 of 0.3 * 0.6 of it.
        lo, hi = TUNING_WINDOW
        a["tones"] = [tone(y, 440.0 * 2.0 ** (9.3 / 12.0), lo, hi, 0.7 * LIN_6),
                      tone(y, 220.0, lo, hi, 0.3 * 0.6 * TRIANGLE_FUNDAMENTAL * LIN_6)]
        asserts.append("tones")
    if case == "unison_tones":
        # By hand: voices 25 cents down, on pitch and 25 cents up, panned hard left, centred and
        # hard right: each side holds its own outer voice at sqrt 2 (the pan law's f32 of it)
        # and the middle one at 1, both over 3, and not the other side's outer voice.
        lo, hi = UNISON_WINDOW
        down, up = 440.0 * 2.0 ** (-0.25 / 12.0), 440.0 * 2.0 ** (0.25 / 12.0)
        outer, middle = f32(math.sqrt(2.0)) / 3.0 * LIN_6, LIN_6 / 3.0
        a["tones"] = [tone(left, down, lo, hi, outer), tone(left, 440.0, lo, hi, middle),
                      tone(left, up, lo, hi, 0.0), tone(right, up, lo, hi, outer, 1),
                      tone(right, 440.0, lo, hi, middle, 1), tone(right, down, lo, hi, 0.0, 1)]
        asserts.append("tones")
    if case == "noise":
        extra += probes_around(t2s(PPQ // 4), t2s(2 * PPQ))
    extra += b3_answer(case, a, y, asserts)
    extra += b5_answer(case, a, y, asserts)
    probes = sorted(set(BASE_PROBES + [k for k in extra if 0 <= k < frames]))
    probes = [k for k in probes if k < frames]
    a["probe_frames"] = probes
    a["probes"] = {"left": [float(jf(left[k])) for k in probes],
                   "right": [float(jf(right[k])) for k in probes]}
    a["asserts"] = asserts
    return a


def tone(y, hz, lo, hi, expected, side=0):
    return {"hz": hz, "side": side, "window": [lo, hi], "expected": expected,
            "tolerance": TONE_TOLERANCE, "measured": hann_amplitude(y, hz, lo, hi)}


CASES = list(PATTERN_CASES) + list(LIVE_CASES)


def expected():
    return {"rate": SR, "bars": 1, "tolerances": {"close": CLOSE_TOLERANCE,
                                                  "amplitude": AMPLITUDE_TOLERANCE},
            "format_version": FORMAT_VERSION, "synth": metadata(),
            "cases": {case: case_answer(case) for case in CASES}}


# ---------- self-checks ----------
CHECKS = []


def check(name, ok, detail=""):
    CHECKS.append((name, ok, detail))


def lowest_slot(voices): return 0
def newest(voices): return max(range(VOICES), key=lambda i: voices[i].started)


def self_checks(answers):
    cases = answers["cases"]
    check("38 parameters, keys unique", len(PARAMS) == 38
          and len({p["key"] for p in PARAMS}) == 38, "%d" % len(PARAMS))
    check("K_A is log2(1.3/0.3)", abs(K_A - math.log2(1.3 / 0.3)) <= 4.5e-16,
          "%.17g" % math.log2(1.3 / 0.3))
    check("K_DR is log2(1.0001/0.0001)", abs(K_DR - math.log2(1.0001 / 0.0001)) <= 2e-15,
          "%.17g" % math.log2(1.0001 / 0.0001))
    for v, want in ((0.0, 0.001), (0.25, 0.01), (0.5, 0.1), (0.75, 1.0), (1.0, 10.0)):
        got = time_seconds(f32(v))
        check("time control %g is %g s" % (v, want), abs(got / want - 1.0) < 1e-12, "%.17g" % got)
    check("LFO rate 0.75 is 2.99 Hz", abs(lfo_rate_hz(f32(0.75)) - 2.99) < 0.005,
          "%.6g" % lfo_rate_hz(f32(0.75)))
    # The attack from 0 reaches 1 t_a after the start: 441 frames at 10 ms (held to 441 +- 1).
    one = cases["adsr"]["env_one_at"]
    check("adsr: the envelope reaches 1 at frame 441 +- 1", abs(one - 441) <= 1, "frame %d" % one)
    y = render("adsr")[0]
    zero = cases["adsr"]["zero_from"]
    check("adsr: exactly 0.0 from the frame the release reaches 0",
          all(v == 0.0 for v in y[zero:]) and y[zero - 1] != 0.0, "from frame %d" % zero)
    check("adsr: the release starts on frame 11025 (0.25 s)", cases["adsr"]["release_at"] == 11025)
    sine = cases["sine"]["amplitude"]
    check("sine: the sustain's 440 Hz amplitude is lin(-6) within 1e-4",
          abs(sine["measured"] - sine["expected"]) <= AMPLITUDE_TOLERANCE,
          "%.9g against %.9g" % (sine["measured"], sine["expected"]))
    for case in ("steal", "bend", "tuning", "unison_tones"):
        for t in cases[case]["tones"]:
            check("%s: tone %.3f Hz (side %d) over %s is %.6g within %g"
                  % (case, t["hz"], t["side"], t["window"], t["expected"], t["tolerance"]),
                  abs(t["measured"] - t["expected"]) <= t["tolerance"], "%.3g off"
                  % abs(t["measured"] - t["expected"]))
    # The steal case tells the right order from the wrong ones: the lowest slot keeps note 2
    # (110 Hz), the newest keeps note 1 (55 Hz) or more.
    lo, hi = STEAL_WINDOW
    for name, rule, hz in (("lowest slot", lowest_slot, pitch_hz(45)),
                           ("newest", newest, pitch_hz(33))):
        wrong = play(*case_inputs("steal"), steal=rule)[0]
        heard = hann_amplitude(wrong, hz, lo, hi)
        check("steal: stealing the %s leaves %.0f Hz sounding" % (name, hz), heard > 0.1,
              "amplitude %.3g" % heard)
    check("pedal: the render is held's bit for bit", render("pedal") == render("held"))
    p, u = render("pedal")[0], render("unreleased")[0]
    check("pedal: unreleased's frames up to the lift, not on it",
          p[:LIFT] == u[:LIFT] and p[LIFT] != u[LIFT], "lift frame %d" % LIFT)
    first = next(k for k, v in enumerate(render("sine")[0]) if v != 0.0)
    check("sine: the first sounding frame is 1 (sin 0 is 0)", first == 1, "%d" % first)
    b2_checks(cases)
    b3_checks(cases)
    b5_checks(cases)
    drive_checks(cases)
    return [c for c in CHECKS if not c[1]]


def b2_checks(cases):
    # PolyBLEP: -1 at the jump's start and +1 just before it (the residual cancels the naive
    # wave's step there), 0 outside its two frames' windows.
    dt = 0.01
    check("blep(0) is -1, blep(1-) tends to +1, 0 between",
          blep(0.0, dt) == -1.0 and abs(blep(1.0 - 1e-12, dt) - 1.0) < 1e-9
          and blep(dt, dt) == 0.0 and blep(1.0 - dt, dt) == 0.0 and blep(0.5, dt) == 0.0)
    # The saw corrected is continuous across the wrap: its value just before 1 meets its value
    # at 0 (both 0), where the naive saw jumps from +1 to -1.
    check("the PolyBLEP saw meets itself at the wrap",
          wave(SAW_WAVE, 0.0, dt, 0.5, 0.0) == 0.0
          and abs(wave(SAW_WAVE, 1.0 - 1e-12, dt, 0.5, 0.0)) < 1e-9)
    check("the square at width 0.5 is low from its width on: 0 (its edge's middle) at t = width",
          wave(SQUARE_WAVE, 0.5, dt, 0.5, square_dc(0.5)) == 0.0
          and wave(SQUARE_WAVE, 0.25, dt, 0.5, square_dc(0.5)) == 1.0)
    # DC-free by hand at width 0.3: its mean 2 (0.3) - 1 = -0.4 taken off, so 1.4
    # high, -0.6 low, and its edge's middle 0.4.
    check("the DC-free square at width 0.3 is 1.4 high, -0.6 low and 0.4 on its edge",
          [wave(SQUARE_WAVE, t, dt, 0.3, square_dc(0.3)) for t in (0.15, 0.65, 0.3)]
          == [1.0 + 0.4, -1.0 + 0.4, 0.4])
    check("the triangle is in phase with the sine",
          wave(TRIANGLE_WAVE, 0.0, dt, 0.5, 0.0) == 0.0
          and wave(TRIANGLE_WAVE, 0.25, dt, 0.5, 0.0) == 1.0
          and wave(TRIANGLE_WAVE, 0.75, dt, 0.5, 0.0) == -1.0)
    dt_a7 = pitch_hz(105) / SRF
    check("square_width: at A7 the narrow pulse's corrections overlap (dt > 0.05)",
          dt_a7 > f32(0.05), "dt %.4f" % dt_a7)
    check("unison detune is -25, 0, +25 cents at 50", [detune_cents(50.0, k, 3)
                                                       for k in range(3)] == [-25.0, 0.0, 25.0])
    check("unison pans are -1, 0, +1 at spread 1", [unison_pan(1.0, k, 3)
                                                     for k in range(3)] == [-1.0, 0.0, 1.0])
    for m in cases["square_width"]["means"]:
        check("square_width: the mean over %s is %.6g within %g" % (m["window"], m["expected"],
                                                                    m["tolerance"]),
              abs(m["measured"] - m["expected"]) <= m["tolerance"],
              "%.3g off" % abs(m["measured"] - m["expected"]))
    # Between its edges the mix sits at 0.7 (-1 - (2 * 0.05 - 1)) + 0.3 (1 - (2 * 0.95 - 1)) =
    # -0.04 of the volume, by hand (74% of A7's frames and 98% of A3's: the median), where a
    # duty turned round (the widths swapped) sits at +0.04. The mean cannot tell them apart now
    # (both 0); with the mean left in, the mix averaged -0.36.
    params, notes, mbpm = PATTERN_CASES["square_width"]
    level = (0.7 * (-1.0 - (2.0 * 0.05 - 1.0)) + 0.3 * (1.0 - (2.0 * 0.95 - 1.0))) * LIN_6
    left = render("square_width")[0]
    swapped = dict(params, osc1_width=params["osc2_width"], osc2_width=params["osc1_width"])
    other = play(values_of(swapped), note_events(notes, mbpm), FRAMES)[0]
    for lo, hi in SQUARE_WINDOWS:
        mine, theirs = statistics.median(left[lo:hi]), statistics.median(other[lo:hi])
        check("square_width: between its edges over %s it sits at %.6g by hand, the widths "
              "swapped at the other sign" % ([lo, hi], level),
              abs(mine - level) < 1e-6 and abs(theirs + level) < 1e-6,
              "%.9g and %.9g" % (mine, theirs))
    before = play(values_of(params), note_events(notes, mbpm), FRAMES, law=pulse_with_dc)[0]
    lo, hi = SQUARE_WINDOWS[1]
    check("square_width: with the mean left in, the mix averaged -0.36 of the volume (2w - 1 each)",
          abs(sum(before[lo:hi]) / (hi - lo) - (-0.36 * LIN_6)) < MEAN_TOLERANCE,
          "%.6g" % (sum(before[lo:hi]) / (hi - lo)))
    # The subtraction of 0.0 leaves every bit: a square of width 0.5 heard alone renders the
    # square's bits from before the pulse was DC-free (the cases' width-0.5 squares keep their answers).
    half = values_of({"osc1_wave": 3.0})
    events = note_events([note(0, PPQ // 2, 69)], MBPM)
    now, then = play(half, events, 12000), play(half, events, 12000, law=pulse_with_dc)
    check("a width-0.5 square renders its bits from before the pulse was DC-free", now == then
          and any(v != 0.0 for v in now[0]), "%d frames" % len(now[0]))
    for case in STEREO:
        d = cases[case]["sides_differ"]
        check("%s: the sides differ" % case, d["measured"] > d["by_more_than"],
              "by %.3g" % d["measured"])
    # The noise by hand: slot 0's first value from seed 1, slot 1's (note B, frame 5512)
    # from seed 2, through the attack's first step and the volume; note C (frame 44100) on
    # slot 0 again goes on from where slot 0 stopped, so it is not seed 1's first value.
    left = render("noise")[0]
    attack = 1.3 * (1.0 - coefficient(K_A, f32(0.0)))
    first = lambda seed: f32(((0.0 + xorshift32(seed) / 2147483648.0 - 1.0) * attack) * LIN_6)
    b_start, c_start = t2s(PPQ // 4), t2s(2 * PPQ)
    check("noise: frame 0 is seed 1's first value", left[0] == first(1),
          "%.9g" % left[0])
    a_at_b = render_without_b()[b_start]
    check("noise: note B adds seed 2's first value on its frame",
          abs(left[b_start] - a_at_b - first(2)) < 1e-7,
          "%.9g" % (left[b_start] - a_at_b))
    check("noise: note C goes on from slot 0's noise (not seed 1 again)",
          left[c_start] != first(1) and left[c_start - 1] == 0.0, "%.9g" % left[c_start])


def phases_at_start(synth):
    """The oscillators as they were before the free-running phases: every note starting its phases at k / U
    (from a reset at 0)."""
    synth.phases_at_start = True
    for v in synth.voices: v.phase = [[0.0] * UNISON_MAX, [0.0] * UNISON_MAX]


def pulse_with_dc(synth):
    """The pulse as it was before it was DC-free: its mean left in."""
    for osc in synth.osc: osc.dc = None


def b5_checks(cases):
    """The free-running phases' cases tell their law from the one before it."""
    check("PHI is the golden ratio's fraction, (sqrt 5 - 1) / 2, to the bit",
          PHI == (math.sqrt(5.0) - 1.0) / 2.0, "%.17g" % ((math.sqrt(5.0) - 1.0) / 2.0))
    # By hand: slot 0's voices at 0, 1/3, 2/3; slot 1's 0.618.., 0.951.., 0.285...
    check("a reset spreads slot 0's unison voices by thirds and starts slot 1 at PHI",
          [start_phase(0, k) for k in range(3)] == [0.0, 1.0 / 3.0, 2.0 / 3.0]
          and start_phase(1, 0) == PHI and abs(start_phase(1, 2) - (PHI + 2.0 / 3.0 - 1.0)) < 1e-15)
    d = cases["repeat"]["differs"]
    check("repeat: the second note does not play the first's frames (by more than %g)"
          % d["by_more_than"], d["measured"] > d["by_more_than"], "by %.3g" % d["measured"])
    y = play(*case_inputs("repeat"), law=phases_at_start)[0]
    (a0, a1), (b0, b1) = d["first"], d["second"]
    check("repeat: with phases set at each note the second note played the first's frames bit for bit",
          y[a0:a1] == y[b0:b1] and any(v != 0.0 for v in y[a0:a1]))
    # By hand: the first note (slot 0, from phase 0) stepped its saw once a frame it sounded,
    # to the frame its release reached 0; the second starts from frac(frames x f / sr) there,
    # its first frame (2t - 1) times the attack's first step, 1.3 (1 - a), and lin(-6).
    trace = []
    play(*case_inputs("repeat"), trace=trace)
    sounded = trace.index(0.0, trace.index(1.0)) + 1
    t = math.fmod(sounded * note_hz(60 - 69.0) / SRF, 1.0)
    first = (2.0 * t - 1.0) * 1.3 * (1.0 - coefficient(K_A, f32(0.0))) * LIN_6
    got = render("repeat")[0][REPEAT_SECOND]
    check("repeat: the second note starts its saw at phase %.6f, where the first left it "
          "(%d frames sounded), by hand" % (t, sounded), abs(got - first) < 1e-6,
          "%.9g against %.9g" % (got, first))
    # The window is 50 ms, not 10 ms: over the first 10 ms this
    # chord peaks higher now (2.178 against 1.675), as the old law's stack comes later, once the
    # saws come round together at the chord's common period; over 50 ms (and the whole note)
    # it is lower.
    p = cases["chord"]["peak_below"]
    check("chord: the first 50 ms peak %.4f, below %.4f with every note from phase 0 (the old "
          "law): %.2f dB" % (p["measured"], p["before"],
                                   20.0 * math.log10(p["measured"] / p["before"])),
          p["measured"] < p["before"])


def drive_checks(cases):
    """Ruling 28's: the drive by hand, the normalisation it asks for, drive 0 as nothing."""
    for t in cases["drive_tone"]["tones"]:
        check("drive_tone: harmonic %.0f Hz of tanh(2 sin) over tanh(2) is %.6g within %g, by "
              "hand" % (t["hz"], t["expected"], t["tolerance"]),
              abs(t["measured"] - t["expected"]) <= t["tolerance"],
              "%.3g off" % abs(t["measured"] - t["expected"]))
    unscaled = LIN_6 * tanh_harmonic(DRIVE_TONE_D, 1) * DRIVE_TONE_GAIN
    right = cases["drive_tone"]["tones"][0]["expected"]
    check("drive_tone: without the 1/tanh(d) the fundamental would be %.5f, not %.5f"
          % (unscaled, right), abs(unscaled - right) > 0.005)
    loud = LIN_6 * tanh_harmonic(DRIVE_TONE_D, 1) / math.tanh(DRIVE_TONE_D)
    check("drive_tone: without the auto gain the fundamental would be %.5f, not %.5f"
          % (loud, right), abs(loud - right) > 0.1)
    y = render("drive_tone")[0]
    top = peak(y, *DRIVE_TONE_WINDOW)
    want = LIN_6 * DRIVE_TONE_GAIN
    check("drive_tone: a full-scale sine peaks at the auto gain (%.6f) of the volume "
          "(tanh(2)/tanh(2) = 1 before it), ADAA a hair under" % DRIVE_TONE_GAIN,
          want * 0.999 < top <= want, "%.9g against %.9g" % (top, want))
    check("drive_zero: drive written as 0 renders the saw case's bits",
          render("drive_zero") == render("saw"))
    for case in ("drive_half", "drive_full"):
        left, right = render(case)
        check("%s: the render is finite and sounds" % case,
              all(math.isfinite(v) for v in left + right) and peak(left, 0, FRAMES) > 0.1,
              "peak %.4g" % peak(left, 0, FRAMES))
    full, half = render("drive_full")[0], render("drive_half")[0]
    check("drive_full and drive_half differ", full != half)


def render_without_b():
    """The noise case without note B: note A's frames alone, to take B's first value out."""
    params, notes, mbpm = PATTERN_CASES["noise"]
    return play(values_of(params), note_events([notes[0], notes[2]], mbpm), t2s(BAR, mbpm))[0]


def b3_checks(cases):
    """The filter, LFO and glide cases: each by-hand answer holds on the generator's own render,
    and each case tells the law from a wrong one (the cases would pass a wrong synth otherwise)."""
    for case in ("cutoff_tones", "lfo_cutoff"):
        for t in cases[case]["tones"]:
            check("%s: tone %.3f Hz over %s is %.6g within %g"
                  % (case, t["hz"], t["window"], t["expected"], t["tolerance"]),
                  abs(t["measured"] - t["expected"]) <= t["tolerance"], "%.3g off"
                  % abs(t["measured"] - t["expected"]))
    for case in ("lfo_pitch", "wheel", "glide", "glide_separated"):
        c = cases[case]["crossings"]
        check("%s: the render's %d zero crossings over %s are by hand's within %d frame"
              % (case, len(c["frames"]), c["window"], c["tolerance"]),
              c["measured"] is not None and c["measured"] <= c["tolerance"],
              "%s off" % c["measured"])
    # The filter case's cutoff by hand: the full envelope throws it past 0.45 sr (P3 clamps it);
    # once the envelope sits at its sustain (0) it is 632.46 Hz key tracked 3 semitones down.
    cutoffs = []
    play(*case_inputs("filter"), cutoffs=cutoffs)
    settled = FC_HALF * 2.0 ** (-3.0 / 12.0)
    check("filter: the envelope's peak throws the cutoff past 0.45 sr; it settles at %.2f Hz"
          % settled, max(cutoffs) > MAX_PITCH and abs(cutoffs[t2s(BAR // 2) - 1] / settled - 1.0)
          < 1e-12, "%.6g Hz, then %.9g Hz" % (max(cutoffs), cutoffs[t2s(BAR // 2) - 1]))
    y = render("filter")[0]
    check("filter: the render is finite and rings past the saw's peak (Q 20)",
          all(math.isfinite(v) for v in y) and max(map(abs, y)) > LIN_6,
          "peak %.4g" % max(map(abs, y)))
    # Each case tells the law from a wrong one, by hand.
    lo, hi = 4410, FRAMES
    no_velocity = FC_HALF * 2.0 ** (0.5 * (81 - 60) / 12.0)
    wrong = LIN_6 * P32.svf_closed(P32.LOW, 880.0, no_velocity, Q_HALF)
    right = cases["cutoff_tones"]["tones"][2]["expected"]
    check("cutoff_tones: without velocity -> cutoff the 880 Hz tone would be %.4f, not %.4f"
          % (wrong, right), abs(wrong - right) > 0.1)
    from_a4 = FC_HALF * 2.0 ** (0.5 * (57 - 69) / 12.0)
    wrong = LIN_6 * P32.svf_closed(P32.LOW, 220.0, from_a4, Q_HALF)
    right = cases["cutoff_tones"]["tones"][0]["expected"]
    check("cutoff_tones: key tracking from A4 (not C4) would give 220 Hz %.4f, not %.4f"
          % (wrong, right), abs(wrong - right) > 0.01)
    up, down = [t["expected"] for t in cases["lfo_cutoff"]["tones"][:2]]
    check("lfo_cutoff: the high and low halves differ by hand (an LFO started half way round "
          "swaps them)", abs(up - down) > 0.5, "%.4f against %.4f" % (up, down))
    for name, hz_at, start, end, at, phase, case in (
            ("a glide straight in Hz", lambda n: glide_hz(n, True), GLIDE_START, GLIDE_END, 0.0,
             C5_PHASE, "glide"),
            ("the LFO started a quarter round", lambda n: lfo_pitch_hz(n, 0.25), 0, FRAMES, 0.5,
             0.0, "lfo_pitch"),
            ("no vibrato from the wheel", lambda n: wheel_hz(n, 0.0), NOTE_ON, NOTE_OFF, 0.0,
             0.0, "wheel")):
        c = cases[case]["crossings"]
        lo, hi = c["window"]
        other = [n for n in crossings_by_hand(start, hz_at, end, at, phase) if lo <= n < hi]
        apart = (len(other) != len(c["frames"])
                 or max(abs(x - w) for x, w in zip(other, c["frames"])) > 10)
        check("%s: %s puts the crossings more than 10 frames off" % (case, name), apart)
    # glide_separated: a synth that counted a gate closing on this frame as held would glide.
    loose = lambda v: v.env.stage in (ATTACK, DECAY, SUSTAIN) and v.m > 0
    y = play(*case_inputs("glide_separated"), held=loose)[0]
    c = cases["glide_separated"]["crossings"]
    got = crossings_of(y, *c["window"])
    check("glide_separated: counting a closing gate as held glides (crossings off)",
          len(got) != len(c["frames"])
          or max(abs(g - w) for g, w in zip(got, c["frames"])) > 10)
    pins = laws()
    for name, orders in (("cutoff", CUTOFF_ORDERS), ("pitch", PITCH_ORDERS)):
        for label, other, out in orders:
            check("%s law: the pins tell its order from %s" % (name, label),
                  any(other(row) != row[out] for row in pins[name]))


# ---------- the laws, pinned to the bit (p33_laws.json) ----------
# The orders the voice's laws leave open move a value by an ulp or so, which the f32 output hides (a render
# almost never changes a bit), so the laws' f64 values are pinned themselves: a test of the
# synth's own functions holds them to these bits. Inputs are messy on purpose (every sum
# rounds), drawn from a fixed xorshift32 stream.
def draws(seed):
    """A fixed stream of numbers in [0, 1): xorshift32 from `seed`."""
    state = seed
    while True:
        state = xorshift32(state)
        yield state / 4294967296.0


PLAIN_ROWS = 4                                       # rows taken as they come


def pick_rows(make, width, seed, orders):
    """Rows of a law from a fixed stream: the first PLAIN_ROWS, then for each order of
    `orders` no row yet tells apart, the next row that does. Another order often rounds the
    same: the velocity term (f32 x f32 x 48) is exact in f64, so where it goes in the sum moves
    a bit only when the sum before it is far smaller."""
    source = draws(seed)
    take = lambda: make([next(source) for _ in range(width)])
    rows = [take() for _ in range(PLAIN_ROWS)]
    for _, other, out in orders:
        for _ in range(100000):
            if any(other(r) != r[out] for r in rows): break
            rows.append(take())
        else:
            sys.exit("no row of the stream tells an order apart")
    return rows


def cutoff_row(u):
    # filter_env and key_track scaled down by up to 2^-9, so the sum before the LFO's and the
    # velocity's terms ranges from their size to far below it.
    f = {"cutoff": f32(u[0]), "filter_env": f32((2.0 * u[1] - 1.0) * 2.0 ** -int(u[9] * 10)),
         "key_track": f32(u[2] * 2.0 ** -int(u[10] * 10)), "lfo_cutoff": f32(u[3]),
         "vel_cutoff": f32(u[4])}
    fenv, key, lfo, vel = u[5], int(u[6] * 128), 2.0 * u[7] - 1.0, f32(max(u[8], 1.0 / 128))
    row = dict(f, fenv=fenv, key=key, lfo=lfo, velocity=vel)
    row["semitones"] = cutoff_semitones(f, fenv, key, lfo, vel)
    row["hz"] = modulated_cutoff_hz(f, fenv, key, lfo, vel)
    return row


class PinOsc:
    def __init__(self, octave, semitones, cents):
        self.octave, self.semitones, self.cents = octave, semitones, cents


def pitch_row(u):
    frames = 1 + int(u[3] * 44100)
    row = {"key": int(u[0] * 128), "bend": int(u[1] * 16384) - 8192,
           "glide_from": (u[2] - 0.5) * 48.0, "m": int(u[4] * frames), "glide_frames": frames}
    row["glide"] = glide_offset(row["glide_from"], row["m"], frames)
    lfo, lfo_pitch, wheel_value = 2.0 * u[5] - 1.0, f32(u[6] * 12.0), int(u[7] * 128)
    row.update(lfo=lfo, lfo_pitch=lfo_pitch, wheel=wheel_value,
               vibrato=lfo * (lfo_pitch + wheel_value / WHEEL_TOP))
    row.update(octave=float(round(u[8] * 4.0 - 2.0)), semitones=float(round(u[9] * 24.0 - 12.0)),
               cents=f32(u[10] * 200.0 - 100.0),
               detune=detune_cents(f32(u[11] * 50.0), int(u[12] * 3), 3))
    osc = PinOsc(row["octave"], row["semitones"], row["cents"])
    row["pitch"] = pitch(row["key"], row["bend"] / 4096.0, row["glide"], row["vibrato"], osc,
                         row["detune"])
    return row


def laws():
    return {"cutoff": pick_rows(cutoff_row, 11, 1000, CUTOFF_ORDERS),
            "pitch": pick_rows(pitch_row, 13, 2000, PITCH_ORDERS)}


def cutoff_terms(r):
    return (r["filter_env"] * r["fenv"] * ENV_SEMITONES,
            r["key_track"] * float(r["key"] - KEY_TRACK_FROM),
            r["lfo"] * r["lfo_cutoff"] * LFO_CUTOFF_SEMITONES,
            r["vel_cutoff"] * (r["velocity"] - 1.0) * VEL_CUTOFF_SEMITONES)


def pitch_terms(r):
    return (float(r["key"] - 69), r["bend"] / 4096.0, r["glide"], r["vibrato"],
            12.0 * r["octave"], r["semitones"], r["cents"] / 100.0, r["detune"] / 100.0)


def summed(terms):
    total = terms[0]
    for t in terms[1:]: total = total + t
    return total


def reordered(terms, order): return summed([terms[i] for i in order])


def glide_by(form):
    return lambda r: form(r["glide_from"], r["m"], r["glide_frames"]) if r["m"] < r["glide_frames"] \
        else 0.0


# Other orders one might write: (label, its value, the row's key it is held to).
# Swapping a sum's first two terms is not one: a + b is b + a in IEEE arithmetic.
CUTOFF_ORDERS = [
    ("its terms reversed", lambda r: reordered(cutoff_terms(r), (3, 2, 1, 0)), "semitones"),
    ("velocity before the LFO", lambda r: reordered(cutoff_terms(r), (0, 1, 3, 2)), "semitones"),
    ("pairs summed first", lambda r: (cutoff_terms(r)[0] + cutoff_terms(r)[1])
     + (cutoff_terms(r)[2] + cutoff_terms(r)[3]), "semitones"),
    ("the LFO's term first", lambda r: reordered(cutoff_terms(r), (2, 0, 1, 3)), "semitones"),
]
PITCH_ORDERS = [
    ("the vibrato before the glide", lambda r: reordered(pitch_terms(r), (0, 1, 3, 2, 4, 5, 6, 7)),
     "pitch"),
    ("the glide and vibrato last", lambda r: reordered(pitch_terms(r), (0, 1, 4, 5, 6, 7, 2, 3)),
     "pitch"),
    ("the bend and glide summed first", lambda r: summed(
        [pitch_terms(r)[0], pitch_terms(r)[1] + pitch_terms(r)[2]] + list(pitch_terms(r)[3:])),
     "pitch"),
    ("the glide as g0 - g0 m / N", glide_by(lambda g, m, n: g - g * m / n), "glide"),
    ("the glide as g0 (N - m) / N", glide_by(lambda g, m, n: g * (n - m) / n), "glide"),
]


def laws_json():
    """p33_laws.json: the f32 parameters as the project file writes them, every f64 as its
    shortest digits (read back exactly by a correctly rounded parser)."""
    rows = laws()
    f32_keys = {"cutoff", "filter_env", "key_track", "lfo_cutoff", "vel_cutoff", "velocity",
                "lfo_pitch", "cents"}
    return {law: [{k: (float(jf(v)) if k in f32_keys else v) for k, v in row.items()}
                  for row in table] for law, table in rows.items()}


# ---------- the files ----------
def files(answers):
    out = {}
    for case, (params, notes, mbpm) in PATTERN_CASES.items():
        out["p33_%s.json" % case] = (J(project(params, notes, mbpm)) + "\n").encode()
    out[LIVE_PROJECT] = (J(project(SINE, [])) + "\n").encode()
    for case, script in LIVE_CASES.items():
        out["p33_%s_events.json" % case] = (J(events_json(script)) + "\n").encode()
    out["p33_expected.json"] = (J(answers) + "\n").encode()
    out["p33_laws.json"] = (J(laws_json()) + "\n").encode()
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
        if stale: sys.exit("not what gen_p33_expected.py writes: " + ", ".join(stale))
    for name, ok, detail in CHECKS:
        print("%s  %s%s" % ("ok  " if ok else "FAIL", name, ": " + detail if detail else ""))
    for case, a in answers["cases"].items():
        print("%-11s %-5s %s %s %d probes" % (case, a["tier"], a.get("fixture", a.get("events")),
                                             a["sha256"][:16], len(a["probe_frames"])))
    if failed: sys.exit("%d self-check(s) failed" % len(failed))


if __name__ == "__main__":
    main()
