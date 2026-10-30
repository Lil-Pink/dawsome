"""Known answers for multisampled instruments: four hand-authored SFZ sets, the zones each one
means, and the renders that playing them must give, sample for sample.

Expected values come from the rules written out below, never from engine code. Every input is
authored here: the .sfz text, the WAVs (impulses and ramps whose frames say which zone played,
at what pitch and gain, how it looped and how long it released), and the projects.

The rules (the design's decisions, restated as the oracle computes them):
  zone pick     every region whose key range and velocity range hold the note plays, in file
                order; velocity byte b = round(v * 128) kept in 1..127 (v is the stored 0..1)
  round robin   each region with seq_length > 1 counts the notes that fall in its ranges (from 0,
                again on Play); the n-th plays when n mod seq_length == seq_position - 1
  pitch         speed = 2^((key - pitch_keycenter + transpose)/12), from the sampler's table of
                semitone ratios times a power of two, times exp2(tune / 1200) (EXP2 below)
  gain          exp2(volume / 20 * log2 10) * (1 - t(1 - (b/127)^2)) for t = amp_veltrack/100 >= 0
                (t < 0: |t|(1 - (b/127)^2)), rounded to f32 once
  reading       position p(t) = offset + t * speed, frame k = floor(p), f = p - k (f64); at
                f = 0 the frame s[k] itself, else the 4-point cubic Hermite (Catmull-Rom) of
                y0..y3 = the frames before k, k, after k and after that: c1 = 0.5 (y2 - y0),
                c2 = y0 - 2.5 y1 + 2 y2 - 0.5 y3, c3 = 0.5 (y3 - y0) + 1.5 (y1 - y2),
                ((c3 f + c2) f + c1) f + y1 in f64, to f32 once (`hermite`); inside a loop
                [ls, le] (le inclusive) a position past le wraps to ls + fmod(p - ls, le - ls +
                1), the frame after le is ls while the loop is in force, and once the voice has
                wrapped (its unwrapped position reached le + 1; a released loop_sustain voice:
                where it read at note-off) the frame before ls is le; a frame outside the zone's
                own (before its offset, unwrapped; past its last, `end` inclusive) counts as 0,
                and a position past the last ends the voice
  loops         loop_continuous loops always; loop_sustain loops until note-off, then reads on
                from where it was; one_shot ignores note-off; no_loop stops at note-off
  envelope      attack: t / A (A = round(ampeg_attack * rate) frames); after note-off at frame
                off: level(off) * EXP2(-15 r / R) for r = t - off < R (R = round(ampeg_release *
                rate)), then the voice ends (R = 0: at note-off)
  note-off      a pattern note ends after its length (a step hit after one step):
                off = s(tick + length) - s(tick), at least 1; a live note at its note-off frame
  mixing        out += value * f32(f32(gain * env) * volume) with the channel at volume 1 (the
                Library's preview and a channel an add makes: 9/16, NEW_CHANNEL_VOLUME) and centered
                (the pan law gives exactly 1 on both sides); a mono zone panned p plays the
                channel pan law's sqrt2*sin((1 -+ p) pi/4) per side, as a stereo frame

The set "formats" (p11_formats/, apart from instruments/, whose four sets the Library's tests
list) holds what real libraries write that the four leave out, one sound per key at its root
(speed 1: each frame as decoded) at amp_veltrack 0 (gain 1), a step each:
  wav fmt       a WAV's fmt chunk of a size its format's spec does not give (20 bytes: the 16 of
                WAVEFORMAT, cbSize 0 and two zero bytes, as FreePats' vowel choir writes its
                float WAVs) reads as its first 16 bytes when they agree with themselves (block
                align = channels * bits / 8, byte rate = rate * block align, cbSize inside the
                chunk): float20.wav (IEEE float) decodes as float16.wav, its 16-byte twin, and
                pcm20.wav (16-bit PCM) as its integers over 2^15
  aiff          AIFF (big-endian integers: 16-bit mono aiff16.aiff, 24-bit stereo aiff24.aif,
                8-bit aiff8.aif) and AIFC (16-bit little-endian sowt.aifc, 32-bit big-endian
                float fl32.aifc, uncompressed none.aifc), the rate an 80-bit extended float:
                integers over 2^(bits - 1), floats as they are, stereo interleaved
  volume        a volume past SFZ's range, -144 to 6 dB, plays at the range's nearer end and is
                reported ("volume=9: outside -144 to 6; it plays at 6"): float20.wav on key 74
                at volume=9 plays at exp2(6 / 20 * log2 10)
  aliases       loop_mode=continuous (Karoryfer's) is loop_continuous: loop.wav on key 76 loops
                all its frames while held; a default_path written with a leading backslash
                (Windows-made sets: default_path=\\sounds\\) names the folder beside the file,
                so every sound here is read from sounds/
p11_expected.json's "formats" says what the reader gives (regions, problems) and what each sound
decodes to (rate, channels, values interleaved); "renders"/"formats" the render.

Run:
  python3 -I gen_p11_expected.py              print the known answers
  python3 -I gen_p11_expected.py --write DIR  also write the sets into DIR/instruments/, the
                                              p11_*.json projects and p11_expected.json into DIR
  python3 -I gen_p11_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import json, math, os, struct, sys

SR = 44100; PPQ = 96; MBPM = 120000; STEP = PPQ // 4; BAR = 4 * PPQ; BUF = 512

def t2s(t): return t * 60 * SR * 1000 // (MBPM * PPQ)            # the one tick-to-sample rule
def f32(x): return struct.unpack("<f", struct.pack("<f", x))[0]
def jf(x):
    """An f32 written the way the project file writes it: shortest digits that read back."""
    x = f32(x)
    for p in range(1, 10):
        s = "%.*g" % (p, x)
        if f32(float(s)) == x: break
    return s if ("." in s or "e" in s) else s + ".0"

# 2^(s/12) for s = 0..11, the sampler's table (f64, correctly rounded).
RATIOS = [1.0, 1.0594630943592953, 1.122462048309373, 1.189207115002721, 1.2599210498948732,
          1.3348398541700344, 1.4142135623730951, 1.4983070768766815, 1.5874010519681996,
          1.681792830507429, 1.7817974362806785, 1.887748625363387]
LN_2 = 0.6931471805599453; LOG2_10 = 3.321928094887362
def semitone_speed(s): return RATIOS[s % 12] * 2.0 ** (s // 12)
def exp2(x):
    """2^x from + - * / alone: 2^floor(x) times the Taylor series of e^(f ln 2), 17 terms."""
    whole = math.floor(x); frac = x - whole; y = frac * LN_2
    term = 1.0; total = 1.0
    for k in range(1, 18):
        term = term * y / k
        total += term
    return math.ldexp(total, int(whole))
def velocity_byte(v):
    return max(1, min(127, math.floor(v * 128 + 0.5)))
def velocity_gain(b, veltrack):
    t = veltrack / 100.0; n = b / 127.0; g = n * n; x = abs(t) * (1.0 - g)
    return x if t < 0 else 1.0 - x
def frames_of(seconds): return math.floor(seconds * SR + 0.5)
def pan_gains(pan):
    side = lambda towards: f32(math.sqrt(2) * math.sin((1.0 + towards) * math.pi / 4))
    return [side(-pan), side(pan)]

# ---------- WAVs: 32-bit float ----------
def wav_bytes(channels):
    n = len(channels[0])
    data = struct.pack("<%df" % (n * len(channels)), *[c[i] for i in range(n) for c in channels])
    fmt = struct.pack("<HHIIHH", 3, len(channels), SR, SR * 4 * len(channels), 4 * len(channels), 32)
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack("<I", len(data)) + data
    return b"RIFF" + struct.pack("<I", len(body)) + body
def impulses(length, at, value):
    s = [0.0] * length
    for i in at: s[i] = value
    return s

# ---------- the sets ----------
# layers: three key ranges, two velocity layers each, and a click layered over the top range.
# Zone i's WAV holds two impulses 40 frames apart from frame 4 + 8i, of value (i + 1)/8, so a
# render says which zone played (where and how loud) and at what pitch (the spacing).
LAYER_NAMES = ["a_soft", "b_soft", "c_soft", "a_loud", "b_loud", "c_loud", "click"]
def layer_wav(i): return impulses(100, [4 + 8 * i, 44 + 8 * i], (i + 1) / 8)
LAYERS_SFZ = """// Three key ranges, two velocity layers each, and a click over the top range.
<global> amp_veltrack=0
<group> lovel=1 hivel=64
<region> sample=a_soft.wav lokey=0 hikey=54 pitch_keycenter=48
<region> sample=b_soft.wav lokey=55 hikey=66 pitch_keycenter=60
<region> sample=c_soft.wav lokey=67 hikey=127 pitch_keycenter=72
<group> lovel=65 hivel=127
<region> sample=a_loud.wav lokey=0 hikey=54 pitch_keycenter=48
<region> sample=b_loud.wav lokey=55 hikey=66 pitch_keycenter=60
<region> sample=c_loud.wav lokey=67 hikey=127 pitch_keycenter=72
<group>
<region> sample=click.wav lokey=67 hikey=127 pitch_keycenter=72
"""
# pad: one ramp, looped two ways: continuously below key 64, while held from 64 up.
RAMP = [(i + 1) / 64 for i in range(48)]
PAD_SFZ = """<global> loop_start=20 loop_end=29 ampeg_release=0.004
<region> sample=ramp.wav lokey=0 hikey=63 pitch_keycenter=60 loop_mode=loop_continuous
<region> sample=ramp.wav lokey=64 hikey=127 pitch_keycenter=72 loop_mode=loop_sustain ampeg_attack=0.001
"""
# rr: a round robin pair on key 60 at the default velocity tracking.
RR_SFZ = """<group> seq_length=2 key=60
<region> seq_position=1 sample=rr1.wav
<region> seq_position=2 sample=rr2.wav
"""
# macro: <control> default_path, #define, #include, note names, comments, volume, tune,
# transpose, pan, offset, end, a sample name with a space, and opcodes and a header this
# reader leaves out.
TONE_L = [i / 16 for i in range(16)]; TONE_R = [-i / 16 for i in range(16)]
LOW = impulses(60, [0, 20, 40], 1.0)
MACRO_SFZ = """/* Everything the reader does besides the plain opcodes. */
<control> default_path=sounds/
#define $ROOT 60
#define $VOL -20
<global> amp_veltrack=0 xfin_lovel=0 // velocity crossfades are left out
#include "regions.sfz"
<effect> type=reverb
"""
MACRO_REGIONS = """<group> volume=$VOL lokey=c4 hikey=b4 pitch_keycenter=$ROOT
<region> sample=tone a.wav offset=2 end=11
<region> sample=low.wav lokey=0 hikey=59 pitch_keycenter=c3 transpose=12 tune=-100 pan=-100 xfout_hivel=127
"""
def set_json(name): return {"name": name, "maker": "Dawsome tests", "licence": "CC0-1.0"}
SETS = {
    "layers": {"layers.sfz": LAYERS_SFZ, "set.json": set_json("Layers"),
               **{n + ".wav": [layer_wav(i)] for i, n in enumerate(LAYER_NAMES)}},
    "pad": {"pad.sfz": PAD_SFZ, "set.json": set_json("Pad"), "ramp.wav": [RAMP]},
    "rr": {"rr.sfz": RR_SFZ, "set.json": set_json("Round robin"),
           "rr1.wav": [impulses(20, [5], 1.0)], "rr2.wav": [impulses(20, [9], 1.0)]},
    "macro": {"macro.sfz": MACRO_SFZ, "regions.sfz": MACRO_REGIONS, "set.json": set_json("Macro"),
              "sounds/tone a.wav": [TONE_L, TONE_R], "sounds/low.wav": [LOW]},
}

# formats: one sound per key at its root, amp_veltrack 0; each sound's values (interleaved) are
# what it must decode to.
def wav_fmt_bytes(tag, channels, bits, frames_bytes, fmt_size, frames):
    """A WAV of format `tag` (1 PCM, 3 IEEE float) at SR whose fmt chunk is `fmt_size` bytes:
    WAVEFORMAT's 16, then (from 18) cbSize 0 and zeros."""
    align = channels * bits // 8
    fmt = struct.pack("<HHIIHH", tag, channels, SR, SR * align, align, bits)
    if fmt_size >= 18: fmt += struct.pack("<H", 0) + bytes(fmt_size - 18)
    body = (b"WAVE" + b"fmt " + struct.pack("<I", len(fmt)) + fmt + b"data"
            + struct.pack("<I", len(frames_bytes)) + frames_bytes)
    assert len(frames_bytes) == frames * align
    return b"RIFF" + struct.pack("<I", len(body)) + body
FLOAT_VALUES = [0.25, -0.5, 0.875, -0.125, 0.0625]
PCM16_INTS = [16384, -8192, 32767, -32768, 1]
def float_wav(fmt_size):
    return wav_fmt_bytes(3, 1, 32, struct.pack("<%df" % len(FLOAT_VALUES), *FLOAT_VALUES), fmt_size,
                         len(FLOAT_VALUES))
FORMATS_SOUNDS = {
    # name: (file bytes, rate, channels, values interleaved)
    "float20.wav": (float_wav(20), SR, 1, FLOAT_VALUES),
    "float16.wav": (float_wav(16), SR, 1, FLOAT_VALUES),
    "pcm20.wav": (wav_fmt_bytes(1, 1, 16, struct.pack("<%dh" % len(PCM16_INTS), *PCM16_INTS), 20,
                                len(PCM16_INTS)), SR, 1, [v / 32768.0 for v in PCM16_INTS]),
}
def extended(rate):
    """`rate`, a positive whole number, as AIFF's 80-bit IEEE extended float, big-endian."""
    exponent = rate.bit_length() - 1
    return struct.pack(">HQ", 16383 + exponent, rate << (63 - exponent))
def iff_chunk(tag, body):
    return tag + struct.pack(">I", len(body)) + body + (b"\0" if len(body) % 2 else b"")
def aiff_bytes(channels, bits, data, frames, compression=None):
    """An AIFF file at SR (an AIFC one, with its FVER chunk and an empty compression name, when
    `compression` is a four-byte type) holding `data`, `frames` frames."""
    assert len(data) == frames * channels * bits // 8
    comm = struct.pack(">hIh", channels, frames, bits) + extended(SR)
    chunks = b""
    if compression:
        comm += compression + b"\0\0"
        chunks += iff_chunk(b"FVER", struct.pack(">I", 0xA2805140))
    chunks += iff_chunk(b"COMM", comm) + iff_chunk(b"SSND", struct.pack(">II", 0, 0) + data)
    return b"FORM" + struct.pack(">I", 4 + len(chunks)) + (b"AIFC" if compression else b"AIFF") + chunks
def ints(values, bits, order="big"):
    return b"".join(v.to_bytes(bits // 8, order, signed=True) for v in values)
AIFF16 = [12288, -24576, 32767, -32768, 2]
AIFF24 = [4194304, -8388608, -1, 2097152, 8388607, 0]          # left, right, interleaved
AIFF8 = [64, -128, 127, -1]
SOWT = [1000, -1000, 30000, -30000]
FL32 = [0.75, -0.375, 0.125, 1.5]
NONE16 = [100, -200, 300]
FORMATS_SOUNDS.update({
    "aiff16.aiff": (aiff_bytes(1, 16, ints(AIFF16, 16), 5), SR, 1, [v / 32768.0 for v in AIFF16]),
    "aiff24.aif": (aiff_bytes(2, 24, ints(AIFF24, 24), 3), SR, 2, [v / 8388608.0 for v in AIFF24]),
    "aiff8.aif": (aiff_bytes(1, 8, ints(AIFF8, 8), 4), SR, 1, [v / 128.0 for v in AIFF8]),
    "sowt.aifc": (aiff_bytes(1, 16, ints(SOWT, 16, "little"), 4, b"sowt"), SR, 1,
                  [v / 32768.0 for v in SOWT]),
    "fl32.aifc": (aiff_bytes(1, 32, struct.pack(">4f", *FL32), 4, b"fl32"), SR, 1, FL32),
    "none.aifc": (aiff_bytes(1, 16, ints(NONE16, 16), 3, b"NONE"), SR, 1,
                  [v / 32768.0 for v in NONE16]),
})
LOOP_RAMP = [0.125, 0.25, 0.375, 0.5]
FORMATS_SOUNDS["loop.wav"] = (wav_bytes([LOOP_RAMP]), SR, 1, LOOP_RAMP)
FORMATS_FOLDER = "sounds/"
FORMATS_TWINS = [[FORMATS_FOLDER + "float20.wav", FORMATS_FOLDER + "float16.wav"]]
FORMATS_SFZ = """// What real libraries write: sound files in other forms, a volume past the range, Karoryfer's
// loop mode and a Windows-made default_path.
<control> default_path=\\sounds\\
<global> amp_veltrack=0
<region> sample=float20.wav key=60
<region> sample=pcm20.wav key=62
<region> sample=aiff16.aiff key=64
<region> sample=aiff24.aif key=65
<region> sample=aiff8.aif key=67
<region> sample=sowt.aifc key=69
<region> sample=fl32.aifc key=71
<region> sample=none.aifc key=72
<region> sample=float20.wav key=74 volume=9
<region> sample=loop.wav key=76 loop_mode=continuous
"""
FORMATS_DIR = "p11_formats"
FORMATS_KEYS = [60, 62, 64, 65, 67, 69, 71, 72, 74]
# A step each; the loop held one tick (229 frames: its 4 frames 57 times and one more).
FORMATS_CASE = [(24 * i, 0, key, 1.0) for i, key in enumerate(FORMATS_KEYS)] + [(216, 1, 76, 1.0)]
def formats_line(text):
    """The line of formats.sfz holding `text`, counted from 1."""
    return next(n for n, line in enumerate(FORMATS_SFZ.splitlines(), 1) if text in line)

# ---------- what each set means: the regions the reader must give (its known answer) ----------
def region(sample, lokey=0, hikey=127, center=60, lovel=1, hivel=127, mode=None, loop=None,
           attack=0.0, release=0.0, veltrack=100.0, volume=0.0, tune=0.0, transpose=0, pan=0.0,
           offset=0, end=None, seq=None):
    return {"sample": sample, "lokey": lokey, "hikey": hikey, "pitch_keycenter": center,
            "lovel": lovel, "hivel": hivel, "loop_mode": mode, "loop": loop,
            "ampeg_attack": attack, "ampeg_release": release, "amp_veltrack": veltrack,
            "volume": volume, "tune": tune, "transpose": transpose, "pan": pan,
            "offset": offset, "end": end, "seq": seq}
KEYS = [(0, 54, 48), (55, 66, 60), (67, 127, 72)]
REGIONS = {
    "layers": [region(n + ".wav", lo, hi, c, lovel=v[0], hivel=v[1], veltrack=0.0)
               for v, names in (((1, 64), LAYER_NAMES[:3]), ((65, 127), LAYER_NAMES[3:6]))
               for n, (lo, hi, c) in zip(names, KEYS)]
              + [region("click.wav", 67, 127, 72, veltrack=0.0)],
    "pad": [region("ramp.wav", 0, 63, 60, mode="loop_continuous", loop=[20, 29], release=0.004),
            region("ramp.wav", 64, 127, 72, mode="loop_sustain", loop=[20, 29], release=0.004,
                   attack=0.001)],
    "rr": [region("rr1.wav", 60, 60, 60, seq=[2, 1]), region("rr2.wav", 60, 60, 60, seq=[2, 2])],
    "macro": [region("sounds/tone a.wav", 60, 71, 60, veltrack=0.0, volume=-20.0, offset=2, end=11),
              region("sounds/low.wav", 0, 59, 48, veltrack=0.0, volume=-20.0, transpose=12,
                     tune=-100.0, pan=-100.0)],
}
FORMATS_REGIONS = [region(FORMATS_FOLDER + name, key, key, key, veltrack=0.0)
                   for name, key in [("float20.wav", 60), ("pcm20.wav", 62), ("aiff16.aiff", 64),
                                     ("aiff24.aif", 65), ("aiff8.aif", 67), ("sowt.aifc", 69),
                                     ("fl32.aifc", 71), ("none.aifc", 72)]] + [
                   region(FORMATS_FOLDER + "float20.wav", 74, 74, 74, veltrack=0.0, volume=6.0),
                   region(FORMATS_FOLDER + "loop.wav", 76, 76, 76, veltrack=0.0,
                          mode="loop_continuous")]
# What the reader reports of formats (each with where it is).
FORMATS_PROBLEMS = ["formats.sfz:%d: volume=9: outside -144 to 6; it plays at 6"
                    % formats_line("volume=9")]
# What the reader reports leaving out of macro: each opcode or header with how often and where
# it first appears (the file, as the set names it, and the line).
MACRO_IGNORED = [{"name": "xfin_lovel", "count": 1, "file": "macro.sfz", "line": 5},
                 {"name": "xfout_hivel", "count": 1, "file": "regions.sfz", "line": 3},
                 {"name": "<effect>", "count": 1, "file": "macro.sfz", "line": 7}]

def samples_of(name):
    """Each sample path the set's regions name, as its frames (one list per channel)."""
    files = SETS[name]
    return {path: files[path] for path in files if path.endswith(".wav")}

# ---------- zones: a region with its sample, at the project's rate ----------
def zone(r, frames):
    n = len(frames[0])
    last = n - 1 if r["end"] is None else min(r["end"], n - 1)
    mode = r["loop_mode"] or ("loop_continuous" if r["loop"] else "no_loop")
    return {"frames": frames, "first": r["offset"], "last": last,
            "keys": (r["lokey"], r["hikey"]), "vels": (r["lovel"], r["hivel"]),
            "center": r["pitch_keycenter"], "transpose": r["transpose"],
            "tune": exp2(r["tune"] / 1200.0) if r["tune"] != 0 else 1.0,
            "gain": exp2(r["volume"] / 20.0 * LOG2_10), "veltrack": r["amp_veltrack"],
            "pan": r["pan"] / 100.0, "mode": mode, "loop": r["loop"],
            "attack": frames_of(r["ampeg_attack"]), "release": frames_of(r["ampeg_release"]),
            "seq": r["seq"]}
def zones_of(name):
    wavs = samples_of(name)
    return [zone(r, wavs[r["sample"]]) for r in REGIONS[name]]
def formats_zones():
    """formats' zones, each sound's frames as decoded (a loop to the default end: its last)."""
    out = []
    for r in FORMATS_REGIONS:
        _, _, channels, values = FORMATS_SOUNDS[r["sample"].split("/")[-1]]
        z = zone(r, [values[c::channels] for c in range(channels)])
        if z["mode"] in ("loop_continuous", "loop_sustain") and not z["loop"]:
            z["loop"] = [0, z["last"]]
        out.append(z)
    return out

def looping(z, t, off):
    if z["mode"] == "loop_continuous": return True
    return z["mode"] == "loop_sustain" and (off is None or t < off)
def wrap(z, p):
    ls, le = z["loop"]
    return ls + math.fmod(p - ls, le - ls + 1) if p >= le + 1 else p
def position(z, t, off, speed):
    """Where the voice reads at t, and whether it has wrapped there (for a released sustain
    loop: whether it had where it read at note-off)."""
    if z["mode"] == "loop_continuous" or (z["mode"] == "loop_sustain" and (off is None or t < off)):
        q = z["first"] + t * speed
        return wrap(z, q), q >= z["loop"][1] + 1
    if z["mode"] == "loop_sustain":
        q = z["first"] + off * speed
        return wrap(z, q) + (t - off) * speed, q >= z["loop"][1] + 1
    return z["first"] + t * speed, False
def level(z, t, off):
    """The envelope at t, or None once the voice has ended by its release."""
    att = lambda t: 1.0 if z["attack"] == 0 or t >= z["attack"] else t / z["attack"]
    if z["mode"] == "one_shot" or off is None or t < off: return att(t)
    r = t - off
    if r >= z["release"]: return None
    return att(off) * exp2(-(15 * r / z["release"]))
def frame_value(z, k, side):
    channels = z["frames"]
    return channels[min(side, len(channels) - 1)][k]
def hermite(y0, y1, y2, y3, f):
    """The 4-point cubic Hermite (Catmull-Rom) between y1 and y2 at f, in f64, to f32 once."""
    c1 = 0.5 * (y2 - y0)
    c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3
    c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2)
    return f32(((c3 * f + c2) * f + c1) * f + y1)
def value_at(z, p, held, wrapped):
    """The zone's [left, right] at position p (before its pan), the loop in force when `held`,
    `wrapped` once the voice has wrapped; None past its last frame."""
    k = math.floor(p)
    if k > z["last"]: return None
    f = p - k
    ls, le = z["loop"] if z["loop"] else (None, None)
    def after(i):
        if i is None: return None
        if held and i == le: return ls
        return i + 1 if i < z["last"] else None
    def before(i):
        if wrapped: return le if i == ls else i - 1
        return i - 1 if i > z["first"] else None
    frames = [before(k), k, after(k), after(after(k))]
    value = []
    for side in (0, 1):
        y = [0.0 if i is None else frame_value(z, i, side) for i in frames]
        value.append(y[1] if f == 0.0 else hermite(y[0], y[1], y[2], y[3], f))
    return value
def read(z, t, off, speed):
    """The zone's frame at t as [left, right] and whether it is stereo, or None once ended."""
    p, wrapped = position(z, t, off, speed)
    value = value_at(z, p, looping(z, t, off), wrapped)
    if value is None: return None
    stereo = len(z["frames"]) == 2 or z["pan"] != 0
    if z["pan"] != 0 and len(z["frames"]) == 1:
        g = pan_gains(z["pan"])
        value = [f32(value[0] * g[0]), f32(value[1] * g[1])]
    return value, stereo

# ---------- rendering notes ----------
def start_voices(zones, counters, key, v):
    """The voices a note of key and velocity v starts: (zone, speed, gain) in file order."""
    b = velocity_byte(v)
    out = []
    for i, z in enumerate(zones):
        if not (z["keys"][0] <= key <= z["keys"][1] and z["vels"][0] <= b <= z["vels"][1]): continue
        if z["seq"] and z["seq"][0] > 1:
            n = counters.get(i, 0); counters[i] = n + 1
            if n % z["seq"][0] != z["seq"][1] - 1: continue
        speed = semitone_speed(key - z["center"] + z["transpose"]) * z["tune"]
        gain = f32(z["gain"] * velocity_gain(b, z["veltrack"]))
        out.append((z, speed, gain))
    return out
# The volume a channel that plays notes is added at: 9/16, -5.0 dB.
NEW_CHANNEL_VOLUME = 0.5625
def render(zones, notes, frames, volume=1.0):
    """notes: (start sample, off frames or None, key, velocity), in the order they start; the
    channel at `volume`, centered (each frame's level f32(f32(gain * env) * volume))."""
    left = [0.0] * frames; right = [0.0] * frames
    counters = {}
    for start, off, key, v in notes:
        for z, speed, gain in start_voices(zones, counters, key, v):
            held = None if z["mode"] == "one_shot" else off
            t = 0
            while start + t < frames:
                env = level(z, t, held)
                if env is None: break
                got = read(z, t, held, speed)
                if got is None: break
                (l, r), _ = got
                amp = f32(gain * f32(env))
                left[start + t] = f32(left[start + t] + f32(l * f32(amp * volume)))
                right[start + t] = f32(right[start + t] + f32(r * f32(amp * volume)))
                t += 1
    return left, right
def pattern_notes(notes):
    """Pattern notes (tick, length, key, velocity) as (start, off, key, velocity)."""
    out = []
    for tick, length, key, v in notes:
        gate = length if length > 0 else STEP
        out.append((t2s(tick), max(1, t2s(tick + gate) - t2s(tick)), key, v))
    return out
def nonzero(left, right):
    return [[i, l, r] for i, (l, r) in enumerate(zip(left, right)) if l != 0.0 or r != 0.0]

# ---------- the projects (the landed format: canonical field order, defaults left out) ----------
def J(d): return json.dumps(d, separators=(",", ":"), ensure_ascii=False)
def tracks(n=8): return [{"id": i, "name": "Track %d" % i, "mute": False} for i in range(1, n + 1)]
def instrument(i, name, path):
    return {"id": i, "name": name, "kind": "instrument", "instrument": path, "volume": 1.0,
            "pan": 0.0, "mute": False}
def sampler(i, name, sample):
    return {"id": i, "name": name, "kind": "sampler", "sample": sample, "volume": 1.0, "pan": 0.0,
            "mute": False}
class Raw(str): pass
def note_json(channel, tick, length, key, v):
    n = {"channel": channel, "tick": tick, "length": length, "velocity": Raw(jf(v))}
    if key != 60: n["key"] = key
    return n
def project(channels, notes, version=4, playlist=None):
    d = {"format_version": version, "sample_rate": SR, "tempo_mbpm": MBPM, "ppq": PPQ,
         "next_channel_id": max(c["id"] for c in channels) + 1, "channels": channels,
         "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": notes}]}
    if playlist: d["playlist"] = playlist
    return d
def dump(d):
    """Canonical JSON with velocities written as the project file writes f32s."""
    text = J(d)
    for raw in sorted({n["velocity"] for p in d["patterns"] for n in p["notes"]}, key=len, reverse=True):
        text = text.replace('"velocity":"%s"' % raw, '"velocity":%s' % raw)
    return text

# Pattern notes per case: (tick, length, key, velocity). Each sounds well apart from the others.
CASES = {
    "layers": [(0, 0, 60, 100 / 128), (96, 0, 60, 40 / 128), (192, 0, 47, 1.0), (288, 0, 74, 0.25)],
    "pad": [(0, 2, 60, 1.0), (192, 1, 67, 1.0), (288, 1, 72, 1.0)],
    "rr": [(0, 0, 60, 1.0), (96, 0, 60, 0.5), (192, 0, 60, 1.0), (288, 0, 60, 0.25)],
    "macro": [(0, 0, 64, 1.0), (192, 0, 48, 1.0)],
}
# Velocities that are no whole number of 128ths, as a hand-written or version 1 file holds them:
# the byte is the nearest, not the one below. layers: 0.505 * 128 = 64.64 is byte 65, the loud
# layer (64 would be the soft one); pad (amp_veltrack 100): 0.7 * 128 = 89.6 is byte 90, gain
# (90/127)^2 (89 would be (89/127)^2).
VELOCITY = {"layers": [(0, 0, 60, 0.505)], "pad": [(0, 2, 60, 0.7)]}
SET_NAMES = {"layers": "Layers", "pad": "Pad", "rr": "Round robin", "macro": "Macro"}
SFZ_FILES = {"layers": "layers.sfz", "pad": "pad.sfz", "rr": "rr.sfz", "macro": "macro.sfz"}
def sfz_path(name): return "instruments/%s/%s" % (name, SFZ_FILES[name])
def case_project(name, path=None, cases=CASES):
    notes = [note_json(1, *n) for n in cases[name]]
    return project([instrument(1, SET_NAMES[name], path or sfz_path(name))], notes)

# The portable reference: the same layers, named inside the per-user data folder.
SHIPPED = "dawsome:instruments/layers/layers.sfz"
# Live: the pad's continuous zone held from call 0 to call 1 by MIDI messages, on a channel
# with no notes; a live note starts on the first frame of the call it comes before.
LIVE_ON, LIVE_OFF = 0, 1
def live_events():
    return {"buffer": BUF, "events": [{"call": LIVE_ON, "bytes": [0x90, 60, 127]},
                                      {"call": LIVE_OFF, "bytes": [0x80, 60, 0]}]}
# The Library drop: the set "Pad" (a copy of pad/ in the project's sets/ folder) dropped on
# track 1 at bar 2 of p11_base: AddInstrumentChannel (the channel at the volume a channel that
# plays notes is added at) then PlaceClips of pattern 1, a bar long.
BEAT = [{"channel": 1, "tick": 0, "length": 0, "velocity": Raw("1.0")}]
BASE = project([sampler(1, "beat", "impulse.wav")], BEAT, version=2)
def after_set():
    track, bar = 1, 2
    start = (bar - 1) * BAR
    clip = {"id": 1, "track": track, "start": start, "length": BAR, "kind": "pattern", "pattern": 1}
    added = dict(instrument(2, "Pad", "sets/pad/pad.sfz"), volume=NEW_CHANNEL_VOLUME)
    doc = project(BASE["channels"] + [added], BEAT,
                  playlist={"next_track_id": 9, "next_clip_id": 2, "tracks": tracks(), "clips": [clip]})
    return doc, {"set": "Pad", "track": track, "bar": bar, "tick": start, "selected_channel": 2,
                 "document": "p11_after_set.json"}

def renders():
    out = {}
    for name in CASES:
        left, right = render(zones_of(name), pattern_notes(CASES[name]), t2s(BAR))
        out[name] = {"fixture": "p11_%s.json" % name, "frames": t2s(BAR), "nonzero": nonzero(left, right)}
    for name in VELOCITY:
        notes = [(t, l, k, f32(v)) for t, l, k, v in VELOCITY[name]]
        left, right = render(zones_of(name), pattern_notes(notes), t2s(BAR))
        out["velocity_" + name] = {"fixture": "p11_velocity_%s.json" % name, "frames": t2s(BAR),
                                   "bytes": [velocity_byte(n[3]) for n in notes],
                                   "nonzero": nonzero(left, right)}
    live = [(LIVE_ON * BUF, (LIVE_OFF - LIVE_ON) * BUF, 60, 127 / 128)]
    left, right = render(zones_of("pad"), live, t2s(BAR))
    out["live"] = {"fixture": "p11_live.json", "events": "p11_live_events.json", "frames": t2s(BAR),
                   "nonzero": nonzero(left, right)}
    # The same notes on the pad as the Library previews it: at the volume a channel is added at.
    left, right = render(zones_of("pad"), live, t2s(BAR), NEW_CHANNEL_VOLUME)
    out["live_added"] = {"events": "p11_live_events.json", "frames": t2s(BAR),
                         "volume": NEW_CHANNEL_VOLUME, "nonzero": nonzero(left, right)}
    left, right = render(formats_zones(), pattern_notes(FORMATS_CASE), t2s(BAR))
    out["formats"] = {"fixture": "p11_formats.json", "frames": t2s(BAR),
                      "nonzero": nonzero(left, right)}
    return out

def regions_json(name):
    return [dict(r) for r in REGIONS[name]]
def expected():
    return {"renders": renders(),
            "regions": {name: regions_json(name) for name in REGIONS},
            "ignored": {"macro": MACRO_IGNORED},
            "sets": {name: {"dir": name, "sfz": SFZ_FILES[name], "name": SET_NAMES[name]} for name in SETS},
            "shipped": {"fixture": "p11_shipped.json", "reference": SHIPPED, "same_as": "layers"},
            "set_drop": after_set()[1],
            "formats": {"dir": FORMATS_DIR, "sfz": "formats.sfz",
                        "regions": [dict(r) for r in FORMATS_REGIONS],
                        "problems": FORMATS_PROBLEMS,
                        "sounds": {FORMATS_FOLDER + name: {"rate": rate, "channels": channels,
                                                           "values": values}
                                   for name, (_, rate, channels, values) in FORMATS_SOUNDS.items()},
                        "twins": FORMATS_TWINS}}

def files():
    """Every file --write makes, by its path under DIR, as bytes."""
    out = {}
    for name, entries in SETS.items():
        for path, content in entries.items():
            key = "instruments/%s/%s" % (name, path)
            if path.endswith(".wav"): out[key] = wav_bytes(content)
            elif path.endswith(".json"): out[key] = (J(content) + "\n").encode()
            else: out[key] = content.encode()
    for name in CASES:
        out["p11_%s.json" % name] = (dump(case_project(name)) + "\n").encode()
    out["p11_shipped.json"] = (dump(case_project("layers", SHIPPED)) + "\n").encode()
    for name in VELOCITY:
        doc = case_project(name, cases=VELOCITY)
        out["p11_velocity_%s.json" % name] = (dump(doc) + "\n").encode()
    live = project([instrument(1, "Pad", sfz_path("pad"))], [])
    out["p11_live.json"] = (dump(live) + "\n").encode()
    out["p11_live_events.json"] = (J(live_events()) + "\n").encode()
    out["p11_base.json"] = (dump(BASE) + "\n").encode()
    out["p11_after_set.json"] = (dump(after_set()[0]) + "\n").encode()
    out[FORMATS_DIR + "/formats.sfz"] = FORMATS_SFZ.encode()
    for name, (content, _, _, _) in FORMATS_SOUNDS.items():
        out[FORMATS_DIR + "/" + FORMATS_FOLDER + name] = content
    formats = project([instrument(1, "Formats", FORMATS_DIR + "/formats.sfz")],
                      [note_json(1, *n) for n in FORMATS_CASE])
    out["p11_formats.json"] = (dump(formats) + "\n").encode()
    out["p11_expected.json"] = (J(expected()) + "\n").encode()
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
        if stale: sys.exit("not what gen_p11_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    answers = expected()
    for name, r in answers["renders"].items():
        first = r["nonzero"][:3]
        print("%-7s %d frames, %d sounding; first %s" % (name, r["frames"], len(r["nonzero"]),
              ", ".join("%d: %.9g/%.9g" % (f, l, rr) for f, l, rr in first)))
    print("layers: velocity bytes", [velocity_byte(n[3]) for n in CASES["layers"]],
          "-> b_loud, b_soft, a_loud at speed %.17g, c_soft + click at %.17g"
          % (semitone_speed(-1), semitone_speed(2)))
    print("rr:     gains", [f32(velocity_gain(velocity_byte(n[3]), 100.0)) for n in CASES["rr"]],
          "zones 1, 2, 1, 2")
    print("pad:    release %d frames, attack %d; offs" % (frames_of(0.004), frames_of(0.001)),
          [o for _, o, _, _ in pattern_notes(CASES["pad"])])
    print("macro:  gain %.9g, tone speed %.17g, low speed %.17g"
          % (f32(exp2(-20 / 20.0 * LOG2_10)), semitone_speed(4), semitone_speed(12) * exp2(-100 / 1200.0)))
    print("drop:   %s on track %d at bar %d: channel 2, clip at tick %d" % (
        answers["set_drop"]["set"], answers["set_drop"]["track"], answers["set_drop"]["bar"],
        answers["set_drop"]["tick"]))

if __name__ == "__main__":
    main()
