"""Known answers for the master limiter (and the loop fixes found beside it): what the native
Limiter on the master does to a mix that runs past its ceiling, through `dawsome render` (32-bit
float, so nothing past full scale is lost before the comparison), where an instrument zone's
loop wraps when its file is at another rate than the project, and what such a zone's file above
the project's rate keeps of its band.

Expected values come from the rules written out below (the design's decisions, restated as this
oracle computes them), never from engine code; zone reading, gains and mixing are
gen_p11_expected.py's rules (imported, as gen_p22_expected.py does). Every input is authored
here: the sounds, their sets and the projects.

The rules:
  ceiling   C = f32(exp2(dB / 20 * log2 10)) for the ceiling parameter dB (an f32, -12..0,
            default -0.3); exp2 from + - * / alone (gen_p11's), so 0 dB is exactly 1
  release   r = exp2(-log2(e) / (ms / 1000 * sr)) for the release parameter ms (an f32,
            1..1000, default 100): the gain's reduction falls to 1/e of itself in that time
  limiter   per stereo frame (l, r) of its input, in f64, from a reduction d = 0:
              d = d * r, set to 0 when below 1e-30 (the engine's TINY rule)
              g = 1 - d
              p = max(|l|, |r|)
              if p * g > C: g = C / p, d = 1 - g
              out = (f32(l * g), f32(r * g))
            one gain for both sides (linked), no lookahead (no latency): the first frame past C
            comes out exactly on C, a frame at C exactly or under passes as it is (g = 1 then
            multiplies by exactly 1), and silence stays silence; Play resets d to 0
  master    the Limiter in the master's first slot runs on the master's bus before its fader
            (unity, which changes no bit) and pan (centered, the same): the render is the
            limiter's output
  mixing    gen_p11's: a zone voice adds value * f32(f32(gain * env) * volume) into the bus in
            f32, gain 1 here (byte 127, amp_veltrack 100), volume 1 or 2 (exact either way)
  time      44.1 kHz, 120 BPM, ppq 96: one bar is 88200 frames, what every render plays
  loops     a zone of a file at file rate F in a project at rate R: frame k of the file is
            frame round_half_up(k * R / F); the loop [ls, le] (le inclusive, the frames from ls
            up to the start of le + 1) is [at(ls), at(le + 1) - 1], so a loop of N file frames
            is N * R / F frames long when that is whole (found while the 808 sets were made:
            at(le) made every loop one frame short at 88.2/96 kHz)
  last      a loop that ends on its file's last frame is converted to the project's rate as the
            voice plays it: the frames after the last are the loop's start again, never silence
            (found while the 808 sets were made: the conversion bent the loop's last frames, a
            -50 dB tick per wrap; a sine loop of whole periods then reads as the sine, to 1e-6)
  not whole a loop of N file frames that is no whole number of frames at the project's rate
            (N * R / F not whole) is not converted: the zone plays its file's own frames at F,
            its speed times F / R (one f64 division), its loop points and offsets as written, so
            it wraps every N file frames as it does at F (found rendering FreePats' 44.1 kHz
            loops at 48 kHz: a loop rounded to whole frames slipped 0.26 to 0.66 frame a wrap,
            a click 4 to 16 times the loop's own second difference); everything else gen_p11's
            reading, speed = semitone_speed * tune * F / R
  band      such a zone's file above the project's rate (F > R) is low-passed once, on load,
            at F, its frames as many and its points as written: read at F / R times its speed a
            frame holds everything up to F / 2, and above R / 2 that folds down (found by the
            review of the not-whole rule: a 96 kHz file's 32 kHz tone came out at 16 kHz at its
            full level, where the converter that rule bypasses left it at -167 dBFS). The taps h,
            2H + 1 of them (H their half), from + - * / and sqrt alone (sin and the Bessel I0
            by their series), as band_taps() restates them:
              ratio = R / F, cut = 0.475 * ratio, width = 2 pi 0.05 ratio (the band from
              0.45 R to 0.5 R, in radians a frame of the file), A = 96 dB, beta =
              0.1102 (A - 8.7), H = min(ceil((A - 7.95) / (2.285 width) / 2), 1024)
              h[j] = I0(beta sqrt(1 - (k / H)^2)) / I0(beta) * (2 cut when k = 0, else
              sin(2 pi cut k) / (pi k)), k = j - H, then each h[j] over their sum (0 dB at DC)
            a Kaiser-windowed sinc: designed for 96 dB down from 0.5 R up (at least 95 dB at
            every file rate from 44.8 to 384 kHz over 44.1 and 48 kHz), within 0.0002 dB to
            0.45 R (H at most 1024, reached past 17 times the project's rate: a header's rate
            must not make the load's cost, frames times taps, unbounded); linear phase, applied
            centered (frame n from frames n - H to n + H), so it
            moves no frame. Each frame of the loop is the taps over the loop played on and on
            (circularly, frame ls + ((m - ls) mod N) for any m), every other frame over the
            file's own frames (silence before the first and past the last); each sum in f64,
            j from 0 up, to f32 once. So the loop's frames, read end to end, are exactly the
            filter over the loop as the voice plays it: the wrap is as smooth as the loop's own
            (filtered over the file's frames instead, a loop whose file goes on with another
            sound past its end would carry that sound's ringing into every wrap: 40 times the
            loop's own in seam96). A file at or below the project's rate is read as it is

  p24_tones/       the set "tones": one zone per key, each at its root (speed 1, no reading
                   between frames): key 60 burst.wav, 62 spike.wav, 64 dc.wav
  p24_chord/       the set "chord": one zone over every key, sine.wav (1 s of
                   f32(0.89125 sin(2 pi 220 i / 44100)), -1 dBFS, one full-velocity note's peak
                   as the shipped sets are normalized) rooted on key 60, no loop
  p24_burst.json   key 60 at volume 2: 0.1 s of a 440 Hz sine at +6 dBFS, then 0.4 s at -20 dBFS
                   (the gain's recovery heard), the Limiter at its defaults
  p24_chord.json   keys 60 and 67 together on the chord set (about +5 dBFS summed), the Limiter
  p24_under.json   key 60 alone on the chord set (-1 dBFS: under the ceiling throughout), the
                   Limiter; p24_under_plain.json the same with no limiter
  p24_spike.json   key 62: one frame at 10.0 (+20 dBFS) then 0.5 s of 0.5 (the curve itself)
  p24_dc.json      key 64: 0.1 s of silence, then 0.25 s of C exactly (a step to the ceiling)
  p24_hard.json    p24_burst with the ceiling at 0 dBFS and the release at 1 ms
  p24_silence.json the chord set with no notes, the Limiter
  p24_loop96/      a 44.1 kHz file (20 periods of a 100 Hz sine, 441 frames each) looping 5
                   periods [4410, 6614]; p24_loop96.json plays it at 96 kHz for one bar
  p24_looplast/    a 48 kHz file (20 periods of a 100 Hz sine, 480 frames each) looping its last
                   10 [4800, 9599], the file's last frame; p24_looplast.json plays it at 44.1 kHz
  p24_loop48/      a 44.1 kHz file (40 periods of a 441 Hz sine, 100 frames each) looping 3
                   periods [2000, 2299], 326.53 frames at 48 kHz; p24_loop48.json plays it at
                   48 kHz for one bar
  p24_loop441/     a 48 kHz file (40 periods of a 480 Hz sine, 100 frames each) looping the same
                   frames, 275.625 at 44.1 kHz; p24_loop441.json plays it at 44.1 kHz (band
                   limited first: its file is above the project's rate)
  p24_alias96/     a 96 kHz file (267 periods of 15 frames: f32(0.25 sin(2 pi 19200 i / 96000) +
                   0.25 sin(2 pi 32000 i / 96000))) looping [2000, 3004], 1005 frames, 502.5 at
                   48 kHz; p24_alias96.json plays it at 48 kHz (the review's probe)
  p24_seam96/      a 96 kHz file of 4200 frames: 0.5 sin(2 pi i / 105) up to the loop's end, 0.5
                   held after it (the file goes on with another sound); looping 3 periods
                   [2100, 2414], 315 frames, 157.5 at 48 kHz; p24_seam96.json plays it at 48 kHz
  p24_expected.json  each case: its inputs and what the tests compare

The cases (the spec's five, and the loops'):
  (1) burst     every frame at C or under, the first frame past C exactly on C, the recovery
                frame by frame (hash and probes), and the same with ceiling 0 dB, release 1 ms
  (2) chord     two full-scale notes summed, then limited: to the bit
  (3) under     a mix under the ceiling throughout: bit for bit the render without the limiter
  (4) saturated the spike (the gain snaps to C/10 and recovers), the DC step at C (no change),
                hard (0 dB, 1 ms), silence (every frame +0.0)
  (5) blocks    burst, chord and spike at --block 1, 7, 64 and 4096 give the 512 render's bytes
  (6) loop96    the render repeats every N * 96000/44100 = 4800 frames from the loop's start
                (9600), to the bit
  (7) looplast  the render is 0.5 sin(2 pi 100 t / 44100) to 1e-6 from the loop's start (4410),
                at every frame around each wrap and every 50th elsewhere
  (8) loop48, loop441  the render, to the bit (its SHA-256), at --block 1, 7, 64 and 4096 too;
                and what it means, measured on the render: at every wrap but the last (the
                frame the read point first reaches the loop's end + 1 + k N) the largest |second
                difference| of the 4 frames from 2 before the wrap is at most 1.5 times the
                largest in the middle half of the lap that follows (a slip of s frames a wrap
                gives about s / w times it, w the sine's radians a frame: 9.2 at both rates when
                the loop is rounded to 326 and 275 frames), and the sine's pitch, by its rising zero
                crossings from the first wrap on, is the file's (441 Hz, 480 Hz) within 1 cent
                (2.8 and 3.9 cents sharp when rounded)
  (9) alias96, seam96  as (8), the pitch within 0.5 cent (19.2 kHz; 96000 / 105 Hz); and in
                alias96's render, over frames [8000, 40010) (2134 times 15 frames: whole periods
                of both tones at 48 kHz, so each one's level is its own DFT bin's, exactly), the
                19.2 kHz tone within 0.5 dB of its 0.25 (-12.04 dBFS) and the 32 kHz tone's fold
                at 48 - 32 = 16 kHz below -90 dBFS (unfiltered: -12.04 dBFS, its full level, as
                a read at speed 2 keeps every other frame)

Run:
  python3 -I gen_p24_expected.py              print the known answers
  python3 -I gen_p24_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p24_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import hashlib, importlib.util, math, os, struct, sys

HERE = os.path.dirname(os.path.abspath(__file__))


def rules():
    """gen_p11_expected.py, whose functions restate the reader's and the player's rules."""
    sys.dont_write_bytecode = True                   # no __pycache__ beside the fixtures
    spec = importlib.util.spec_from_file_location("gen_p11_expected",
                                                  os.path.join(HERE, "gen_p11_expected.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


P11 = rules()
f32, exp2, J, SR, PPQ = P11.f32, P11.exp2, P11.J, P11.SR, P11.PPQ
LOG2_10 = P11.LOG2_10
LOG2_E = 1.4426950408889634                          # log2(e), the f64 nearest
TINY = 1e-30
FRAMES = P11.t2s(4 * PPQ)                            # one bar: what every render plays
LIMITER = "dawsome.limiter"
DEFAULT_CEILING_DB = -0.3
DEFAULT_RELEASE_MS = 100.0
BLOCKS = [1, 7, 64, 4096]
LOOP_TOLERANCE = 1e-6


# ---------- the limiter, restated ----------
def ceiling_of(db):
    return float(f32(exp2(float(f32(db)) / 20.0 * LOG2_10)))


def release_of(ms, rate):
    return exp2(-LOG2_E / (float(f32(ms)) / 1000.0 * rate))


F32_MAX = 3.4028234663852886e38  # f32::MAX


def limit(left, right, db=DEFAULT_CEILING_DB, ms=DEFAULT_RELEASE_MS, rate=SR):
    c, r = ceiling_of(db), release_of(ms, rate)
    d = 0.0
    out_l, out_r = [], []
    for x, y in zip(left, right):
        d = d * r
        if abs(d) < TINY: d = 0.0
        g = 1.0 - d
        # an infinite side is held to f32's largest value first (the engine's rule), so it
        # leaves at the ceiling instead of as NaN
        x, y = (max(-F32_MAX, min(F32_MAX, v)) for v in (x, y))
        p = max(abs(x), abs(y))
        if p * g > c:
            g = c / p
            d = 1.0 - g
        out_l.append(f32(x * g))
        out_r.append(f32(y * g))
    return out_l, out_r


# ---------- the sounds ----------
def wav_bytes(channels, rate):
    """32-bit float WAV at `rate` (gen_p11's writer, which is fixed at 44.1 kHz)."""
    n = len(channels[0])
    data = struct.pack("<%df" % (n * len(channels)), *[c[i] for i in range(n) for c in channels])
    fmt = struct.pack("<HHIIHH", 3, len(channels), rate, rate * 4 * len(channels),
                      4 * len(channels), 32)
    body = (b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data"
            + struct.pack("<I", len(data)) + data)
    return b"RIFF" + struct.pack("<I", len(body)) + body


def sine(amplitude, hz, i, rate=SR): return f32(amplitude * math.sin(2.0 * math.pi * hz * i / rate))


C_DEFAULT = ceiling_of(DEFAULT_CEILING_DB)
LOUD_FRAMES, SOFT_FRAMES = SR // 10, 2 * SR // 5     # 440 Hz: 44 whole periods, a zero crossing
BURST = ([sine(1.0, 440.0, i) for i in range(LOUD_FRAMES)]
         + [sine(0.05, 440.0, i) for i in range(LOUD_FRAMES, LOUD_FRAMES + SOFT_FRAMES)])
SPIKE_AT = 1000
SPIKE = [0.0] * SPIKE_AT + [10.0] + [0.5] * (SR // 2)
DC = [0.0] * (SR // 10) + [C_DEFAULT] * (SR // 4)
CHORD_PEAK = 0.89125
CHORD_SINE = [sine(CHORD_PEAK, 220.0, i) for i in range(SR)]

TONES_SFZ = ("<region> sample=burst.wav key=60\n"
             "<region> sample=spike.wav key=62\n"
             "<region> sample=dc.wav key=64\n")
CHORD_SFZ = "<region> sample=sine.wav pitch_keycenter=60\n"
TONES_SET, CHORD_SET = "p24_tones/tones.sfz", "p24_chord/chord.sfz"
TONES = {60: P11.zone(P11.region("burst.wav", 60, 60, 60), [BURST]),
         62: P11.zone(P11.region("spike.wav", 62, 62, 62), [SPIKE]),
         64: P11.zone(P11.region("dc.wav", 64, 64, 64), [DC])}
CHORD = P11.zone(P11.region("sine.wav", center=60), [CHORD_SINE])

# The loops: 100 Hz is 441 frames at 44.1 kHz, 480 at 48 kHz and 960 at 96 kHz, so a loop of whole
# periods is a whole number of frames at every rate here. Each file is one period's values
# repeated, so every period holds the same bits.
LOOP_HZ, LOOP_AMPLITUDE = 100.0, 0.5
def periods(rate, count):
    one = [sine(LOOP_AMPLITUDE, LOOP_HZ, i, rate) for i in range(int(rate / LOOP_HZ))]
    return one * count
UP_FILE_RATE, UP_RATE = 44100, 96000
UP = periods(UP_FILE_RATE, 20)
UP_LOOP = (441 * 10, 441 * 15 - 1)                   # 5 periods, 2205 frames
LAST_FILE_RATE = 48000
LAST = periods(LAST_FILE_RATE, 20)
LAST_LOOP = (480 * 10, len(LAST) - 1)                # the last 10 periods, to the last frame
# The loops no rate here holds whole (see "not whole"): a sine of 100 frames a period at its
# file's rate, 40 periods, looping 3 of them [2000, 2299]: 300 frames, 326.53 at 48 kHz from
# 44.1 kHz and 275.625 at 44.1 kHz from 48 kHz.
SLIP_PERIOD, SLIP_PERIODS = 100, 40
SLIP_LOOP = (20 * SLIP_PERIOD, 23 * SLIP_PERIOD - 1)
SLIP_BOUND, SLIP_CENTS = 1.5, 1.0
SLIP = {"loop48": (44100, 48000), "loop441": (48000, SR)}   # case: (file rate, project rate)
def slip_file(file_rate):
    one = [sine(LOOP_AMPLITUDE, file_rate / SLIP_PERIOD, i, file_rate) for i in range(SLIP_PERIOD)]
    return one * SLIP_PERIODS
# The band's cases (see "band"), both 96 kHz files at 48 kHz, both loops an odd number of frames
# (half a frame over a whole number at 48 kHz). alias96: two tones whose periods are 5 and 3
# frames, 15 frames repeated; seam96: a sine of 105 frames, the file another sound past the
# loop's end.
BAND_FILE_RATE, BAND_RATE, BAND_CENTS = 96000, 48000, 0.5
ALIAS_TONE, ALIAS_HIGH, ALIAS_AMPLITUDE = 19200.0, 32000.0, 0.25
ALIAS_FOLD = BAND_RATE - ALIAS_HIGH                  # 16 kHz: where 32 kHz lands at 48 kHz
ALIAS_LOOP = (2000, 3004)
ALIAS_WINDOW = (8000, 2134 * 15)                     # first frame, frames
ALIAS_TONE_DB = 20.0 * math.log10(ALIAS_AMPLITUDE)
ALIAS_DB_TOLERANCE, ALIAS_FOLD_BELOW_DB = 0.5, -90.0
def alias_file():
    two = lambda i: f32(ALIAS_AMPLITUDE * math.sin(2.0 * math.pi * ALIAS_TONE * i / BAND_FILE_RATE)
                        + ALIAS_AMPLITUDE * math.sin(2.0 * math.pi * ALIAS_HIGH * i / BAND_FILE_RATE))
    return [two(i) for i in range(15)] * 267
SEAM_PERIOD, SEAM_LOOP, SEAM_FRAMES = 105, (2100, 2414), 4200
def seam_file():
    return [sine(LOOP_AMPLITUDE, BAND_FILE_RATE / SEAM_PERIOD, i, BAND_FILE_RATE)
            if i <= SEAM_LOOP[1] else LOOP_AMPLITUDE for i in range(SEAM_FRAMES)]
# case: (its file's frames, its loop, the sine's Hz)
BAND = {"alias96": (alias_file(), ALIAS_LOOP, ALIAS_TONE),
        "seam96": (seam_file(), SEAM_LOOP, BAND_FILE_RATE / SEAM_PERIOD)}
def loop_sfz(loop, sample="sine.wav"):
    return ("<region> sample=%s pitch_keycenter=60 loop_mode=loop_continuous "
            "loop_start=%d loop_end=%d\n" % ((sample,) + tuple(loop)))
def at_rate(k, file_rate, rate):
    """Frame k of a file at file_rate as a frame at rate, rounded half up (exact integers)."""
    return (k * rate * 2 + file_rate) // (2 * file_rate)
def converted_loop(loop, file_rate, rate):
    ls, le = loop
    return at_rate(ls, file_rate, rate), at_rate(le + 1, file_rate, rate) - 1


# The band (see "band"): the files above the project's rate a zone reads as they are.
BAND_DB, BAND_MOST_HALF = 96.0, 1024
def sin_pi(x):
    """sin(pi x) from + - * / alone: x less its nearest whole n, 14 terms of the Taylor series
    there (|pi r| <= pi / 2), negated for an odd n."""
    n = math.floor(x + 0.5); r = x - n; t = math.pi * r; t2 = t * t
    term = t; total = t
    for j in range(1, 14):
        term = -term * t2 / ((2 * j) * (2 * j + 1))
        total += term
    return -total if n % 2 else total
def bessel_i0(x):
    """I0(x) by 41 terms of its series, the sum of ((x / 2)^j / j!)^2."""
    half = x / 2.0; term = 1.0; total = 1.0
    for j in range(1, 41):
        term = term * half / j
        total += term * term
    return total
def band_taps(file_rate, rate):
    ratio = rate / file_rate
    cut = 0.475 * ratio
    width = 2.0 * math.pi * 0.05 * ratio
    half = min(math.ceil((BAND_DB - 7.95) / (2.285 * width) / 2.0), BAND_MOST_HALF)
    beta = 0.1102 * (BAND_DB - 8.7)
    i0_beta = bessel_i0(beta)
    raw = []
    for j in range(2 * half + 1):
        k = j - half
        q = k / half
        window = bessel_i0(beta * math.sqrt(1.0 - q * q)) / i0_beta
        ideal = 2.0 * cut if k == 0 else sin_pi(2.0 * cut * k) / (math.pi * k)
        raw.append(window * ideal)
    total = 0.0
    for v in raw: total += v
    return [v / total for v in raw]
def taps_over(frames, h):
    """Each window of len(h) frames in `frames`, the taps over it: f64, j from 0 up, f32 once."""
    n = len(h)
    out = []
    for i in range(len(frames) - n + 1):
        acc = 0.0
        for j in range(n): acc += h[j] * frames[i + j]
        out.append(f32(acc))
    return out
def band_limited(values, file_rate, rate, loop):
    """One channel's frames as a zone on its file's own frames holds them: as they are at or
    below the project's rate, else the band's filter over them, the loop's circularly."""
    if file_rate <= rate: return list(values)
    h = band_taps(file_rate, rate)
    half = (len(h) - 1) // 2
    out = taps_over([0.0] * half + list(values) + [0.0] * half, h)
    ls, le = loop
    n = le - ls + 1
    out[ls:le + 1] = taps_over([values[ls + (i - half) % n] for i in range(n + 2 * half)], h)
    return out


# ---------- the projects ----------
def insert(i, effects=()):
    return {"id": i, "name": "Master" if i == 0 else "Insert %d" % i, "fader": 0.8, "pan": 0.0,
            "mute": False, "routes": [] if i == 0 else [{"to": 0, "amount": 1.0}],
            "effects": list(effects)}


def limiter_effect(params=None):
    return {"id": 1, "slot": 0, "processor": {"id": LIMITER, "params": dict(params or {})}}


def mixer(master_effects):
    return {"next_effect_id": 2, "solo": [],
            "inserts": [insert(0, master_effects)] + [insert(i) for i in range(1, 17)]}


def project(set_path, notes, volume=1.0, params=None, limiter=True, rate=SR):
    channel = P11.instrument(1, "Tone", set_path)
    channel["volume"] = volume
    d = {"format_version": 4, "sample_rate": rate, "tempo_mbpm": P11.MBPM, "ppq": PPQ,
         "next_channel_id": 2, "channels": [channel],
         "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": 1,
                       "notes": [P11.note_json(1, 0, 4 * PPQ, key, 1.0) for key in notes]}]}
    if limiter: d["mixer"] = mixer([limiter_effect(params)])
    return d


HARD = {"ceiling_db": 0.0, "release_ms": 1.0}
PROJECTS = {
    "p24_burst.json": project(TONES_SET, [60], volume=2.0),
    "p24_chord.json": project(CHORD_SET, [60, 67]),
    "p24_under.json": project(CHORD_SET, [60]),
    "p24_under_plain.json": project(CHORD_SET, [60], limiter=False),
    "p24_spike.json": project(TONES_SET, [62]),
    "p24_dc.json": project(TONES_SET, [64]),
    "p24_hard.json": project(TONES_SET, [60], volume=2.0, params=HARD),
    "p24_silence.json": project(CHORD_SET, []),
    "p24_loop96.json": project("p24_loop96/sine.sfz", [60], limiter=False, rate=UP_RATE),
    "p24_looplast.json": project("p24_looplast/sine.sfz", [60], limiter=False),
    "p24_loop48.json": project("p24_loop48/sine.sfz", [60], limiter=False, rate=48000),
    "p24_loop441.json": project("p24_loop441/sine.sfz", [60], limiter=False),
    "p24_alias96.json": project("p24_alias96/tones.sfz", [60], limiter=False, rate=BAND_RATE),
    "p24_seam96.json": project("p24_seam96/sine.sfz", [60], limiter=False, rate=BAND_RATE),
}


# ---------- the renders ----------
def mixed(zones, keys, volume, frames=FRAMES, file_speed=None):
    """The master's bus: each key's voice from frame 0, held for the bar (`frames` long),
    gen_p11's mixing; `file_speed` F / R for zones that play their file's own frames."""
    left, right = [0.0] * frames, [0.0] * frames
    counters = {}
    for key in keys:
        for z, speed, gain in P11.start_voices(zones, counters, key, 1.0):
            if file_speed is not None: speed = speed * file_speed
            t = 0
            while t < frames:
                env = P11.level(z, t, frames)
                if env is None: break
                got = P11.read(z, t, frames, speed)
                if got is None: break
                (l, r), _ = got
                amp = f32(f32(gain * f32(env)) * volume)
                left[t] = f32(left[t] + f32(l * amp))
                right[t] = f32(right[t] + f32(r * amp))
                t += 1
    return left, right


TONE_ZONES = [TONES[k] for k in sorted(TONES)]


def bus(name):
    if name in ("burst", "hard"): return mixed(TONE_ZONES, [60], 2.0)
    if name == "chord": return mixed([CHORD], [60, 67], 1.0)
    if name == "under": return mixed([CHORD], [60], 1.0)
    if name == "spike": return mixed(TONE_ZONES, [62], 1.0)
    if name == "dc": return mixed(TONE_ZONES, [64], 1.0)
    if name == "silence": return [0.0] * FRAMES, [0.0] * FRAMES
    raise KeyError(name)


# case: (project, ceiling dB, release ms)
RENDERS = {
    "burst": ("p24_burst.json", DEFAULT_CEILING_DB, DEFAULT_RELEASE_MS),
    "chord": ("p24_chord.json", DEFAULT_CEILING_DB, DEFAULT_RELEASE_MS),
    "under": ("p24_under.json", DEFAULT_CEILING_DB, DEFAULT_RELEASE_MS),
    "spike": ("p24_spike.json", DEFAULT_CEILING_DB, DEFAULT_RELEASE_MS),
    "dc": ("p24_dc.json", DEFAULT_CEILING_DB, DEFAULT_RELEASE_MS),
    "hard": ("p24_hard.json", HARD["ceiling_db"], HARD["release_ms"]),
    "silence": ("p24_silence.json", DEFAULT_CEILING_DB, DEFAULT_RELEASE_MS),
}


def sha256_of(left, right):
    data = struct.pack("<%df" % (2 * len(left)), *[x for pair in zip(left, right) for x in pair])
    return hashlib.sha256(data).hexdigest()


def first_over(left, right, c):
    for k, (x, y) in enumerate(zip(left, right)):
        if max(abs(x), abs(y)) > c: return k
    return None


def probes(left, right, around):
    """Every 1000th frame, the last, and the frames around each of `around`."""
    at = set(range(0, FRAMES, 1000)) | {FRAMES - 1}
    for k in around:
        at |= set(range(k - 3, k + 4))
    return [[k, left[k], right[k]] for k in sorted(k for k in at if 0 <= k < FRAMES)]


def case_render(name):
    _, db, ms = RENDERS[name]
    left, right = bus(name)
    return (left, right), limit(left, right, db, ms)


def render_answer(name):
    fixture, db, ms = RENDERS[name]
    (inl, inr), (left, right) = case_render(name)
    c = ceiling_of(db)
    over = first_over(inl, inr, c)
    marks = [over] if over is not None else []
    marks += [LOUD_FRAMES] if name in ("burst", "hard") else []
    marks += [SPIKE_AT] if name == "spike" else []
    marks += [SR // 10] if name == "dc" else []
    answer = {"fixture": fixture, "frames": FRAMES, "ceiling": c, "release": release_of(ms, SR),
              "sha256": sha256_of(left, right),
              "sounding": sum(1 for l, r in zip(left, right) if l != 0.0 or r != 0.0),
              "peak_in": max(max(abs(x), abs(y)) for x, y in zip(inl, inr)),
              "peak_out": max(max(abs(x), abs(y)) for x, y in zip(left, right)),
              "first_over": None if over is None else [over, left[over], right[over]],
              "unchanged": (left, right) == (inl, inr),
              "probes": probes(left, right, marks)}
    return answer


BAR_AT_96K = 4 * PPQ * 60 * UP_RATE * 1000 // (P11.MBPM * PPQ)   # the tick rule at 96 kHz


def loop96_answer():
    ls, le = converted_loop(UP_LOOP, UP_FILE_RATE, UP_RATE)
    return {"fixture": "p24_loop96.json", "rate": UP_RATE, "frames": BAR_AT_96K,
            "file_loop": list(UP_LOOP), "loop": [ls, le], "period": le - ls + 1,
            "period_exact": (UP_LOOP[1] - UP_LOOP[0] + 1) * UP_RATE // UP_FILE_RATE}


def looplast_answer():
    ls, le = converted_loop(LAST_LOOP, LAST_FILE_RATE, SR)
    period = le - ls + 1
    wraps = list(range(le + 1, FRAMES, period))
    at = set(range(ls, FRAMES, 50)) | {FRAMES - 1}
    for w in wraps:
        at |= set(range(max(ls, w - 40), min(FRAMES, w + 40)))
    ideal = lambda t: LOOP_AMPLITUDE * math.sin(2.0 * math.pi * LOOP_HZ * t / SR)
    return {"fixture": "p24_looplast.json", "frames": FRAMES, "file_loop": list(LAST_LOOP),
            "file_frames": len(LAST), "loop": [ls, le], "period": period, "wraps": wraps,
            "tolerance": LOOP_TOLERANCE,
            "probes": [[t, ideal(t)] for t in sorted(at)]}


def bar_at(rate): return 4 * PPQ * 60 * rate * 1000 // (P11.MBPM * PPQ)   # the tick rule


def slip_wraps(speed, frames, loop=SLIP_LOOP):
    """The frames where a voice reading the file at `speed` from frame 0 first reaches the
    loop's end + 1 + k N, read as the engine reads (position = 0 + t * speed, in f64)."""
    ls, le = loop
    n = le - ls + 1
    laps = lambda t: -1 if t * speed < le + 1 else math.floor((t * speed - ls) / n)
    return [t for t in range(1, frames) if laps(t) > laps(t - 1)]


def second_differences(y):
    return [abs(y[k + 1] - 2.0 * y[k] + y[k - 1]) for k in range(1, len(y) - 1)]


def seam_ratio(y, wraps):
    """The largest, over every wrap but the last, of its |second difference| (the largest of the
    4 frames from 2 before it) over the largest in the middle half of the lap after it, up to
    the next wrap: d[k] is frame k's."""
    d = [0.0] + second_differences(y) + [0.0]
    def lap(w, n):
        quarter = (n - w) // 4
        return max(d[k] for k in range(w + quarter, n - quarter))
    return max(max(d[k] for k in range(w - 2, w + 2)) / lap(w, n)
               for w, n in zip(wraps, wraps[1:]))


def crossing_hz(y, rate, start):
    """The frequency of y's rising zero crossings from frame `start` on, each placed between its
    frames by the straight line through them."""
    at = [k + y[k] / (y[k] - y[k + 1]) for k in range(start, len(y) - 1) if y[k] < 0.0 <= y[k + 1]]
    return (len(at) - 1) * rate / (at[-1] - at[0])


def own_frames_render(file_rate, rate, wave, loop, filtered=True):
    """Key 60 held for a bar on a zone that reads its file's own frames, `wave`, looping
    `loop`: the render, as the zone holds its frames (band limited unless `filtered` is False,
    which only the printout's comparisons ask for), the speed and the wraps."""
    frames = bar_at(rate)
    held = band_limited(wave, file_rate, rate, loop) if filtered else wave
    z = P11.zone(P11.region("sine.wav", center=60, mode="loop_continuous", loop=list(loop)),
                 [held])
    file_speed = file_rate / rate
    left, right = mixed([z], [60], 1.0, frames, file_speed)
    speed = P11.semitone_speed(0) * z["tune"] * file_speed
    return (left, right), speed, slip_wraps(speed, frames, loop)


def own_frames_answer(case, file_rate, rate, wave, loop, ideal, cents):
    (left, right), speed, wraps = own_frames_render(file_rate, rate, wave, loop)
    y = [float(v) for v in left]
    hz = crossing_hz(y, rate, wraps[0])
    n = loop[1] - loop[0] + 1
    return {"fixture": "p24_%s.json" % case, "file_rate": file_rate, "rate": rate,
            "frames": len(left), "file_loop": list(loop), "loop_at_rate": n * rate / file_rate,
            "speed": speed, "sha256": sha256_of(left, right), "wraps": wraps,
            "seam_bound": SLIP_BOUND, "seam_ratio": seam_ratio(y, wraps),
            "hz": ideal, "cents_tolerance": cents,
            "cents": 1200.0 * math.log2(hz / ideal), "blocks": BLOCKS}


def slip_answer(case):
    file_rate, rate = SLIP[case]
    answer = own_frames_answer(case, file_rate, rate, slip_file(file_rate), SLIP_LOOP,
                               file_rate / SLIP_PERIOD, SLIP_CENTS)
    if file_rate > rate: answer["band_taps"] = len(band_taps(file_rate, rate))
    return answer


def bin_db(y, hz, rate, window):
    """The level of y's component at hz over `window` (first frame, frames: whole periods of
    it), dBFS of its amplitude: 2/n |sum of y[t] e^(-i 2 pi hz t / rate)|."""
    start, n = window
    re = im = 0.0
    for t in range(start, start + n):
        a = 2.0 * math.pi * hz * t / rate
        re += y[t] * math.cos(a)
        im -= y[t] * math.sin(a)
    amplitude = 2.0 * math.hypot(re, im) / n
    return 20.0 * math.log10(amplitude) if amplitude > 0.0 else -math.inf


def band_answer(case):
    wave, loop, ideal = BAND[case]
    answer = own_frames_answer(case, BAND_FILE_RATE, BAND_RATE, wave, loop, ideal, BAND_CENTS)
    answer["band_taps"] = len(band_taps(BAND_FILE_RATE, BAND_RATE))
    if case == "alias96":
        answer["levels"] = {"window": list(ALIAS_WINDOW),
                            "tone": {"hz": ALIAS_TONE, "db": ALIAS_TONE_DB,
                                     "tolerance_db": ALIAS_DB_TOLERANCE},
                            "fold": {"hz": ALIAS_FOLD, "below_db": ALIAS_FOLD_BELOW_DB}}
    return answer


def new_document_master():
    """The master insert a new document (File > New, the window opened with no file, the
    starter document) saves: the Limiter at its defaults in slot 0, effect id 1."""
    return {"insert": insert(0, [limiter_effect()]), "next_effect_id": 2}


def expected():
    renders = {name: render_answer(name) for name in RENDERS}
    return {"renders": renders,
            "blocks": {"sizes": BLOCKS, "cases": ["burst", "chord", "spike"]},
            "same_as": {"under": "p24_under_plain.json"},
            "limiter": {"id": LIMITER, "name": "Limiter",
                        "params": [{"key": "ceiling_db", "default": DEFAULT_CEILING_DB,
                                    "min": -12.0, "max": 0.0, "unit": "dB"},
                                   {"key": "release_ms", "default": DEFAULT_RELEASE_MS,
                                    "min": 1.0, "max": 1000.0, "unit": "ms"}],
                        "ceiling_default": C_DEFAULT, "ceiling_0": ceiling_of(0.0),
                        "ceiling_min": ceiling_of(-12.0),
                        "release_default_44100": release_of(DEFAULT_RELEASE_MS, SR),
                        "release_1ms_44100": release_of(1.0, SR)},
            "loop96": loop96_answer(),
            "looplast": looplast_answer(),
            **{case: slip_answer(case) for case in SLIP},
            **{case: band_answer(case) for case in BAND},
            "new_document_master": new_document_master()}


def files():
    """Every file --write makes, by its path under DIR, as bytes."""
    out = {"p24_tones/tones.sfz": TONES_SFZ.encode(),
           "p24_tones/burst.wav": wav_bytes([BURST], SR),
           "p24_tones/spike.wav": wav_bytes([SPIKE], SR),
           "p24_tones/dc.wav": wav_bytes([DC], SR),
           "p24_chord/chord.sfz": CHORD_SFZ.encode(),
           "p24_chord/sine.wav": wav_bytes([CHORD_SINE], SR),
           "p24_loop96/sine.sfz": loop_sfz(UP_LOOP).encode(),
           "p24_loop96/sine.wav": wav_bytes([UP], UP_FILE_RATE),
           "p24_looplast/sine.sfz": loop_sfz(LAST_LOOP).encode(),
           "p24_looplast/sine.wav": wav_bytes([LAST], LAST_FILE_RATE)}
    for case, (file_rate, _) in SLIP.items():
        out["p24_%s/sine.sfz" % case] = loop_sfz(SLIP_LOOP).encode()
        out["p24_%s/sine.wav" % case] = wav_bytes([slip_file(file_rate)], file_rate)
    out["p24_alias96/tones.sfz"] = loop_sfz(ALIAS_LOOP, "tones.wav").encode()
    out["p24_alias96/tones.wav"] = wav_bytes([BAND["alias96"][0]], BAND_FILE_RATE)
    out["p24_seam96/sine.sfz"] = loop_sfz(SEAM_LOOP).encode()
    out["p24_seam96/sine.wav"] = wav_bytes([BAND["seam96"][0]], BAND_FILE_RATE)
    for name, doc in PROJECTS.items():
        out[name] = (P11.dump(doc) + "\n").encode()
    out["p24_expected.json"] = (J(expected()) + "\n").encode()
    return out


def main():
    args = sys.argv[1:]
    if len(args) == 2 and args[0] in ("--write", "--check"):
        made = files()
        stale = []
        for path, content in sorted(made.items()):
            target = os.path.join(args[1], path)
            if args[0] == "--write":
                os.makedirs(os.path.dirname(target), exist_ok=True)
                with open(target, "wb") as f: f.write(content)
            else:
                try:
                    with open(target, "rb") as f: same = f.read() == content
                except OSError: same = False
                if not same: stale.append(path)
        if stale: sys.exit("not what gen_p24_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    answers = expected()
    for name, r in answers["renders"].items():
        print("%-8s %s: peak in %.6g out %.9g (C %.9g), first over %s, %d sounding%s" % (
            name, r["fixture"], r["peak_in"], r["peak_out"], r["ceiling"],
            r["first_over"][0] if r["first_over"] else None, r["sounding"],
            ", unchanged" if r["unchanged"] else ""))
    lim = answers["limiter"]
    print("ceiling -0.3 dB %.17g, 0 dB %.17g, -12 dB %.17g" % (
        lim["ceiling_default"], lim["ceiling_0"], lim["ceiling_min"]))
    print("release 100 ms %.17g, 1 ms %.17g (44.1 kHz)" % (
        lim["release_default_44100"], lim["release_1ms_44100"]))
    up, last = answers["loop96"], answers["looplast"]
    print("loop96:   file loop %s -> %s at 96 kHz, period %d (exact %d)" % (
        up["file_loop"], up["loop"], up["period"], up["period_exact"]))
    print("looplast: file loop %s of %d frames -> %s at 44.1 kHz, period %d, %d wraps" % (
        last["file_loop"], last["file_frames"], last["loop"], last["period"], len(last["wraps"])))
    for case in list(SLIP) + list(BAND):
        a = answers[case]
        print("%-9s %d Hz file at %d Hz: loop %s is %.3f frames, %d wraps, seam %.3f (bound %.1f), "
              "pitch %+.4f cents%s" % (case + ":", a["file_rate"], a["rate"], a["file_loop"],
                                       a["loop_at_rate"], len(a["wraps"]), a["seam_ratio"],
                                       a["seam_bound"], a["cents"],
                                       ", %d taps" % a["band_taps"] if "band_taps" in a else ""))
    # What the band's choices are measured against, in the printout only: alias96 read without
    # the filter (as before it), and seam96 filtered over the file's own frames, the loop's too.
    (left, _), _, _ = own_frames_render(BAND_FILE_RATE, BAND_RATE, BAND["alias96"][0], ALIAS_LOOP,
                                        filtered=False)
    (fl, _), _, _ = own_frames_render(BAND_FILE_RATE, BAND_RATE, BAND["alias96"][0], ALIAS_LOOP)
    for name, y in (("unfiltered", left), ("filtered", fl)):
        print("alias96 %s: %g Hz at %.2f dBFS, %g Hz at %.2f dBFS" % (
            name, ALIAS_TONE, bin_db(y, ALIAS_TONE, BAND_RATE, ALIAS_WINDOW),
            ALIAS_FOLD, bin_db(y, ALIAS_FOLD, BAND_RATE, ALIAS_WINDOW)))
    wave, loop, _ = BAND["seam96"]
    h = band_taps(BAND_FILE_RATE, BAND_RATE)
    half = (len(h) - 1) // 2
    linear = taps_over([0.0] * half + wave + [0.0] * half, h)
    z = P11.zone(P11.region("sine.wav", center=60, mode="loop_continuous", loop=list(loop)),
                 [linear])
    frames = bar_at(BAND_RATE)
    ll, _ = mixed([z], [60], 1.0, frames, BAND_FILE_RATE / BAND_RATE)
    print("seam96 filtered over the file's own frames: seam %.3f (circularly %.3f)" % (
        seam_ratio([float(v) for v in ll], answers["seam96"]["wraps"]),
        answers["seam96"]["seam_ratio"]))


if __name__ == "__main__":
    main()
