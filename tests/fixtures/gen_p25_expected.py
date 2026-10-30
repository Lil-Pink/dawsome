"""Known answers for the sampler's interpolation and the export's dither: a voice that reads between
two frames reads four of them by cubic Hermite interpolation (the sampler, a channel's sound
settings and an instrument set's zones, loops included), and a 16-bit file is dithered.

Expected values come from the rules written out below (the design's decisions, restated as this
oracle computes them), never from engine code; tick timing, zones, gains and mixing are
gen_p11_expected.py's rules (imported, as gen_p24_expected.py does). Every input is authored
here: the sounds, their sets and the projects.

The rules:
  hermite   a voice at position p reads frame k = floor(p) and f = p - k (f64); at f = 0 the frame
            itself, a copy (speed 1 and every whole-frame read keep their bits); else, with y1
            frame k, y2 the frame after it, y0 the frame before and y3 the frame after y2, the
            4-point cubic Hermite with Catmull-Rom tangents, in f64 from the f32 frames:
              c1 = 0.5 (y2 - y0)
              c2 = y0 - 2.5 y1 + 2 y2 - 0.5 y3
              c3 = 0.5 (y3 - y0) + 1.5 (y1 - y2)
              y  = ((c3 f + c2) f + c1) f + y1
            rounded to f32 once, each side on its own
  points    the frames a voice plays run from its first (a sampler's frame 0, or the start frame
            its sound settings give, S = floor(start * frames); a zone's offset) to its last (the
            sound's, or a zone's end); a point outside them counts 0, before the first as past
            the last (the linear rule counted the frame past the last as 0)
  loops     a zone's loop [ls, le] (le inclusive): while the loop is in force (always for
            loop_continuous, until note-off for loop_sustain) the frame after le is ls; once a
            voice has wrapped (its unwrapped position reached le + 1, for a released
            loop_sustain voice: where it read at note-off) the frame before ls is le, and every
            other frame's the one below it, its offset no edge any more; so a seam interpolates
            across the wrap both ways and a looping voice never reads the frames after le
  reversed  a reversed sampler reads frame i as the sound's frame (frames - 1 - i), its points
            the same way
  dither    a 16-bit WAV: each value v (f32; left then right, frame by frame) is written as
              q = round_half_away(v * 32768 + (r1 + r2 - 1)), clamped to -32768..32767
            r1, r2 the next two values of one LCG run per file from SEED:
              x' = (1664525 x + 1013904223) mod 2^32,  r = (x' >> 8) / 2^24, in [0, 1)
            (TPDF dither, one LSB either way at most, no noise shaping; two draws for every value
            written, whatever it is, so the stream is the file's frame count's and no block's);
            silence is dithered too: a silent 16-bit render is -1, 0 or 1 at every value
  plain     --no-dither, and every 24-bit WAV: q = round_half_away(v * 2^(bits-1)) clamped, the
            rule before this change; 32-bit float and MP3 are written as before

  p25_sine/      the set "sine": one zone over every key rooted on 60, sine.wav: 0.5 s of
                 0.5 sin(2 pi 440 i/sr) + 0.25 sin(2 pi 7000 i/sr) (a high partial, where
                 interpolation shows)
  p25_up.json    key 61 on it: speed 2^(1/12)
  p25_down.json  key 53: speed 2^(-7/12)
  p25_root.json  key 60: speed 1, the file's frames copied
  p25_loop/      the set "loop": loop.wav, 64 frames of 0.8 sin(2 pi i/11 + 0.3) (a period that
                 does not fit the loop, so its seam is a step), a loop_continuous zone (keys
                 0..63, root 60, loop 8..23) and a loop_sustain one (keys 64..127, root 72, loop
                 8..23, offset 12, inside the loop, release 2 ms)
  p25_loop.json  keys 55 and 62 on the continuous zone, half a bar each
  p25_sustain.json  keys 77 and 66 on the sustain zone, a step each: wraps while held, then
                 reads on past the loop's end released
  p25_four.wav   stereo, 4 frames (1, 0.5, 0.25, 0.125 / -0.25, 0.75, -1, 0.5); p25_four.json
                 plays it on a sampler at key 48 (half speed: both edges)
  p25_one.wav    one frame of 0.8; p25_one.json plays it at key 41
  p25_reverse.json  p25_four.wav reversed, from a quarter in (S = 1), 5 semitones down
  p25_quiet.wav  0.25 s: eight saturated frames (1, -1, 2, -2, half an LSB up and down, 1.5 LSB,
                 0), then 0.001 sin(2 pi 1000 i/sr) (-60 dBFS); p25_quiet.json plays it at key
                 60 (speed 1: no interpolation), the rest of the bar silence
  p25_silence.json  the same channel and no notes
  f1_four_on_floor.json  (an older fixture: 1.0 at frames 0, 22050, 44100, 66150) at 16 bits,
                 dithered: the tops stay 32767, the rest the dither alone
  p25_expected.json  each case: its inputs and what the tests compare

The cases (the spec's):
  (1) pitched   up and down to the bit, through the command line (32-bit float renders)
  (2) loop      the seams wrap through the loop (continuous, and sustain with an offset inside
                its loop, released after wrapping), to the bit
  (3) speed 1   root is the file's frames then silence (the copy); and the pinned renders of
                fixtures that play at speed 1 keep their bytes (the test pins them)
  (4) saturated four (a 4-frame stereo sound at half speed: before its first frame and past its
                last), one (a 1-frame sound), reverse (start frame and reversed points)
  (5) blocks    up, loop, sustain and four at --block 1, 7, 64, 4096 give the 512 render's bytes;
                loop and sustain at seeded splits in the engine, the same frames
  (6) dither    quiet at 16 bits equals the rule's values; --no-dither the plain rounding; 24
                bits plain with or without it; silence dithers to -1, 0, 1

Run:
  python3 -I gen_p25_expected.py              print the known answers
  python3 -I gen_p25_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p25_expected.py --check DIR  exit 1 unless the files in DIR are what --write
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
f32, J, SR, PPQ, t2s = P11.f32, P11.J, P11.SR, P11.PPQ, P11.t2s
FRAMES = t2s(4 * PPQ)                                # one bar: what every render plays
BLOCKS = [1, 7, 64, 4096]


# ---------- the read ----------
# The zones' read is gen_p11's (`read`, `value_at`, `hermite`): the rules above are its rules, a
# set's renders its `render`.
hermite = P11.hermite


def sampler_render(channels, notes, start=0.0, reverse=False, semitones=0, frames=FRAMES):
    """A sampler channel at volume 1, centered, playing `channels` (one list: mono): each note
    (start sample, key, velocity 1) from the start frame to the sound's end, whatever its length
    (a sampler is one-shot), at speed 2^((key - 60)/12) times 2^(semitones/12)."""
    n = len(channels[0])
    first = math.floor(f32(start) * n)

    def frame(i):
        if i < first or i >= n: return [0.0, 0.0]
        j = n - 1 - i if reverse else i
        return [channels[0][j], channels[-1][j]]

    left, right = [0.0] * frames, [0.0] * frames
    for at, key in notes:
        speed = P11.semitone_speed(key - 60)
        if semitones: speed = speed * P11.semitone_speed(semitones)
        t = 0
        while at + t < frames:
            p = first + t * speed
            k = math.floor(p)
            if k >= n: break
            f = p - k
            points = [frame(k - 1), frame(k), frame(k + 1), frame(k + 2)]
            value = [points[1][s] if f == 0.0 else
                     hermite(points[0][s], points[1][s], points[2][s], points[3][s], f)
                     for s in (0, 1)]
            left[at + t] = f32(left[at + t] + value[0])
            right[at + t] = f32(right[at + t] + value[1])
            t += 1
    return left, right


# ---------- the dither, restated ----------
SEED = 0x0D175EED
LCG_A, LCG_C = 1664525, 1013904223


def round_away(v):
    """Round half away from zero, exactly (v - floor(v) is exact for a float)."""
    whole = math.floor(v)
    rest = v - whole
    if rest > 0.5 or (rest == 0.5 and v > 0): return whole + 1
    return whole


def plain(values, bits):
    full = float(1 << (bits - 1))
    return [max(-full, min(full - 1.0, round_away(v * full))) for v in values]


def dithered(values):
    """The 16-bit values of a file holding `values` (interleaved), dithered from SEED."""
    x, out = SEED, []
    for v in values:
        x = (LCG_A * x + LCG_C) % 2 ** 32
        r1 = (x >> 8) / 2.0 ** 24
        x = (LCG_A * x + LCG_C) % 2 ** 32
        r2 = (x >> 8) / 2.0 ** 24
        out.append(max(-32768.0, min(32767.0, round_away(v * 32768.0 + (r1 + r2 - 1.0)))))
    return out


# ---------- the sounds ----------
def wav_bytes(channels):
    """32-bit float WAV at 44.1 kHz (gen_p11's writer)."""
    return P11.wav_bytes(channels)


TONE = [f32(0.5 * math.sin(2.0 * math.pi * 440.0 * i / SR)
            + 0.25 * math.sin(2.0 * math.pi * 7000.0 * i / SR)) for i in range(SR // 2)]
LOOP = [f32(0.8 * math.sin(2.0 * math.pi * i / 11.0 + 0.3)) for i in range(64)]
FOUR = [[1.0, 0.5, 0.25, 0.125], [-0.25, 0.75, -1.0, 0.5]]
ONE = [f32(0.8)]
LSB = 1.0 / 32768.0
QUIET = ([1.0, -1.0, 2.0, -2.0, 0.5 * LSB, -0.5 * LSB, 1.5 * LSB, 0.0]
         + [f32(0.001 * math.sin(2.0 * math.pi * 1000.0 * i / SR)) for i in range(8, SR // 4)])

SINE_SFZ = "<region> sample=sine.wav pitch_keycenter=60\n"
LOOP_SFZ = ("<region> sample=loop.wav lokey=0 hikey=63 pitch_keycenter=60 "
            "loop_mode=loop_continuous loop_start=8 loop_end=23\n"
            "<region> sample=loop.wav lokey=64 hikey=127 pitch_keycenter=72 "
            "loop_mode=loop_sustain loop_start=8 loop_end=23 offset=12 ampeg_release=0.002\n")
SINE_SET, LOOP_SET = "p25_sine/sine.sfz", "p25_loop/loop.sfz"
SINE_ZONES = [P11.zone(P11.region("sine.wav", center=60), [TONE])]
LOOP_ZONES = [P11.zone(P11.region("loop.wav", 0, 63, 60, mode="loop_continuous", loop=[8, 23]),
                       [LOOP]),
              P11.zone(P11.region("loop.wav", 64, 127, 72, mode="loop_sustain", loop=[8, 23],
                                  offset=12, release=0.002), [LOOP])]


# ---------- the projects ----------
# Notes: (tick, length, key), velocity 1.
SET_CASES = {
    "up": ("p25_up.json", SINE_SET, SINE_ZONES, [(0, 4 * PPQ, 61)]),
    "down": ("p25_down.json", SINE_SET, SINE_ZONES, [(0, 4 * PPQ, 53)]),
    "root": ("p25_root.json", SINE_SET, SINE_ZONES, [(0, 4 * PPQ, 60)]),
    "loop": ("p25_loop.json", LOOP_SET, LOOP_ZONES, [(0, 2 * PPQ, 55), (2 * PPQ, 2 * PPQ, 62)]),
    "sustain": ("p25_sustain.json", LOOP_SET, LOOP_ZONES,
                [(0, PPQ // 4, 77), (2 * PPQ, PPQ // 4, 66)]),
}
# (project, sound, channels, notes (tick, key), sound settings or None)
SAMPLER_CASES = {
    "four": ("p25_four.json", "p25_four.wav", FOUR, [(0, 48)], None),
    "one": ("p25_one.json", "p25_one.wav", [ONE], [(0, 41)], None),
    "reverse": ("p25_reverse.json", "p25_four.wav", FOUR, [(0, 60)],
                {"semitones": -5, "reverse": True, "start": 0.25}),
}
DITHER_PROJECTS = {"p25_quiet.json": [(0, 60)], "p25_silence.json": []}
SOUND_DEFAULT = {"semitones": 0, "cents": 0, "velocity": 1.0, "attack": 0.0, "release": 0.0,
                 "reverse": False, "start": 0.0, "filter": False, "cutoff": 20000.0,
                 "resonance": 0.0}


def set_project(path, notes):
    return P11.project([P11.instrument(1, "Tone", path)],
                       [P11.note_json(1, tick, length, key, 1.0) for tick, length, key in notes])


def sampler_project(sound, notes, settings=None):
    channel = P11.sampler(1, "Sound", sound)
    if settings:
        channel["sound"] = dict(SOUND_DEFAULT, **settings)
    return P11.project([channel], [P11.note_json(1, tick, 0, key, 1.0) for tick, key in notes],
                       version=5 if settings else 4)


def projects():
    out = {}
    for fixture, path, _, notes in SET_CASES.values():
        out[fixture] = set_project(path, notes)
    for fixture, sound, _, notes, settings in SAMPLER_CASES.values():
        out[fixture] = sampler_project(sound, notes, settings)
    for fixture, notes in DITHER_PROJECTS.items():
        out[fixture] = sampler_project("p25_quiet.wav", notes)
    return out


# ---------- the answers ----------
def sha256_of(left, right):
    data = struct.pack("<%df" % (2 * len(left)), *[x for pair in zip(left, right) for x in pair])
    return hashlib.sha256(data).hexdigest()


def sha256_ints(values):
    """The SHA-256 of integer samples, each a 32-bit little-endian integer, in file order."""
    return hashlib.sha256(struct.pack("<%di" % len(values), *[int(v) for v in values])).hexdigest()


def case_render(name):
    if name in SET_CASES:
        _, _, zones, notes = SET_CASES[name]
        return P11.render(zones, P11.pattern_notes([(t, l, k, 1.0) for t, l, k in notes]), FRAMES)
    _, _, channels, notes, settings = SAMPLER_CASES[name]
    settings = settings or {}
    return sampler_render(channels, [(t2s(tick), key) for tick, key in notes],
                          settings.get("start", 0.0), settings.get("reverse", False),
                          settings.get("semitones", 0))


def note_starts(name):
    if name in SET_CASES: return [t2s(tick) for tick, _, _ in SET_CASES[name][3]]
    return [t2s(tick) for tick, _ in SAMPLER_CASES[name][3]]


def probes(left, right, starts):
    """The first 40 frames from each note's start, every 997th frame and the last."""
    at = set(range(0, FRAMES, 997)) | {FRAMES - 1}
    for s in starts:
        at |= set(range(s, min(FRAMES, s + 40)))
    return [[k, left[k], right[k]] for k in sorted(at)]


def render_answer(name):
    fixture = SET_CASES[name][0] if name in SET_CASES else SAMPLER_CASES[name][0]
    left, right = case_render(name)
    return {"fixture": fixture, "frames": FRAMES, "sha256": sha256_of(left, right),
            "sounding": sum(1 for l, r in zip(left, right) if l != 0.0 or r != 0.0),
            "probes": probes(left, right, note_starts(name))}


def root_answer():
    """Speed 1 is a copy: the file's frames, then silence."""
    left, right = case_render("root")
    copy = TONE + [0.0] * (FRAMES - len(TONE))
    assert left == copy and right == copy, "speed 1 reads the file's frames"
    return dict(render_answer("root"), file="p25_sine/sine.wav", file_frames=len(TONE))


def quiet_values():
    """The 32-bit float render of p25_quiet.json, interleaved: the sound copied (speed 1)."""
    mono = QUIET + [0.0] * (FRAMES - len(QUIET))
    return [v for v in mono for _ in (0, 1)]


def int_answer(fixture, values, extra):
    return dict({"fixture": fixture, "frames": FRAMES, "sha256": sha256_ints(values),
                 "first": [int(v) for v in values[:32]],
                 "nonzero": sum(1 for v in values if v != 0)}, **extra)


# f1_four_on_floor.json's render, its own known answer (crates/cli/tests/known_answers.rs (a)):
# 1.0 on both sides at frames 0, 22050, 44100 and 66150 of one bar, 0 elsewhere.
F1_BEATS = [0, 22050, 44100, 66150]


def f1_values():
    mono = [1.0 if k in F1_BEATS else 0.0 for k in range(FRAMES)]
    return [v for v in mono for _ in (0, 1)]


def dither_answer():
    quiet = quiet_values()
    f1 = dithered(f1_values())
    assert all(f1[2 * k] == f1[2 * k + 1] == 32767 for k in F1_BEATS), "full scale stays the top"
    silent = [0.0] * (2 * FRAMES)
    hiss = dithered(silent)
    counts = {str(level): sum(1 for v in hiss if v == level) for level in (-1, 0, 1)}
    assert sum(counts.values()) == len(hiss), "silence dithers to -1, 0 or 1"
    return {"seed": SEED, "lcg": [LCG_A, LCG_C],
            "quiet16": int_answer("p25_quiet.json", dithered(quiet), {"bits": 16}),
            "quiet16_plain": int_answer("p25_quiet.json", plain(quiet, 16), {"bits": 16}),
            "quiet24": int_answer("p25_quiet.json", plain(quiet, 24), {"bits": 24}),
            "silence16": int_answer("p25_silence.json", hiss, {"bits": 16, "counts": counts}),
            "silence16_plain": int_answer("p25_silence.json", plain(silent, 16), {"bits": 16}),
            "f1_16": int_answer("f1_four_on_floor.json", f1, {"bits": 16, "beats": F1_BEATS})}


def expected():
    renders = {name: render_answer(name) for name in list(SET_CASES) + list(SAMPLER_CASES)
               if name != "root"}
    return {"renders": renders,
            "root": root_answer(),
            "blocks": {"sizes": BLOCKS, "cases": ["up", "loop", "sustain", "four"]},
            "splits": {"cases": ["loop", "sustain"]},
            "dither": dither_answer()}


def files():
    """Every file --write makes, by its path under DIR, as bytes."""
    out = {"p25_sine/sine.sfz": SINE_SFZ.encode(),
           "p25_sine/sine.wav": wav_bytes([TONE]),
           "p25_loop/loop.sfz": LOOP_SFZ.encode(),
           "p25_loop/loop.wav": wav_bytes([LOOP]),
           "p25_four.wav": wav_bytes(FOUR),
           "p25_one.wav": wav_bytes([ONE]),
           "p25_quiet.wav": wav_bytes([QUIET])}
    for name, doc in projects().items():
        out[name] = (P11.dump(doc) + "\n").encode()
    out["p25_expected.json"] = (J(expected()) + "\n").encode()
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
        if stale: sys.exit("not what gen_p25_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    answers = expected()
    for name, r in answers["renders"].items():
        first = [p for p in r["probes"] if p[1] != 0.0 or p[2] != 0.0][:3]
        print("%-8s %s: %d sounding, sha256 %s; first %s" % (
            name, r["fixture"], r["sounding"], r["sha256"][:16],
            ", ".join("%d: %.9g/%.9g" % tuple(p) for p in first)))
    root = answers["root"]
    print("root     %s: the file's %d frames copied, sha256 %s" % (
        root["fixture"], root["file_frames"], root["sha256"][:16]))
    d = answers["dither"]
    print("dither   seed 0x%08X, LCG %d x + %d" % (d["seed"], d["lcg"][0], d["lcg"][1]))
    for key in ("quiet16", "quiet16_plain", "quiet24", "silence16", "silence16_plain", "f1_16"):
        a = d[key]
        print("  %-15s %d nonzero, sha256 %s; first %s" % (
            key, a["nonzero"], a["sha256"][:16], a["first"][:16]))
    print("  silence16 counts", d["silence16"]["counts"])


if __name__ == "__main__":
    main()
