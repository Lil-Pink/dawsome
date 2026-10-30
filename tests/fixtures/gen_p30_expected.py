"""Known answers for the sample editor's engine and media slice: a sound's ordered edit list
(crop, delete, silence, fade in, fade out, gain, normalize, reverse) applied before the sound
plays, the clips' frame offsets moved by the ops that change the length, the zero-crossing
search, and the project file that holds the list (format_version 8, written only when a sound
has edits).

Expected values come from the rules written out below (restated from the design's decisions,
never from engine code) and from each signal's construction:
gen_p11_expected.py's exp2 and Hermite and gen_p13_expected.py's sampler read and project
helpers are imported, and the resample read of gen_p9_expected.py is restated. Every input is
authored here: the WAVs and the projects. The ten-minute sound is made by the test from the
formula in "long" below (230 MB has no place in the repository).

The rules (crates/engine/src/dsp/edits.rs states them the same way):
  range      an op's [from, to) in frames of the sound as the ops before it leave it, cut to its
             end: lo = min(from, n), hi = min(to, n) for a sound of n frames; an op whose cut
             range is empty does nothing (one wholly past the end); N = hi - lo, k = 0..N-1
  crop       keeps [lo, hi)
  delete     removes [lo, hi); the rest closes up
  silence    every value in the range 0.0
  fade_in    x * (k / N)
  fade_out   x * ((N - 1 - k) / N), the time-reverse of a fade in
  gain c     x * exp2(c / 100 * log2(10) / 20), c hundredths of a dB in -4800..2400 (exp2 is
             gen_p11's: + - * / only, the same bits everywhere)
  normalize  x * (exp2(t / 100 * log2(10) / 20) / P), P the largest |x| in the range on either
             side, t in -4800..0 (0 when not written); nothing when P = 0; a NaN is no peak
             (passed over, as f64::max passes it over; it stays NaN)
  reverse    frame lo + k takes the old frame hi - 1 - k
  rounding   each product formed in f64 and rounded to f32 once
  frame map  crop [lo, hi): f < lo -> 0, lo <= f < hi -> f - lo, f >= hi -> hi - lo;
             delete [lo, hi): f < lo -> f, lo <= f < hi -> lo, f >= hi -> f - (hi - lo);
             every audio clip of the channel moves its frame_offset so in the same undo step;
             a stretched clip keeps its ratio: frames T' = max(1, floor(T * S' / S + 1/2)) for
             the sound's frames S before and S' after, offset o' = floor(map(floor(o * S / T))
             * T' / S'); clip lengths in ticks stay
  zero cross the sides summed: a frame exactly 0 is a boundary at it, a sign change between
             frames i - 1 and i a boundary at i; an edge moves to the nearest boundary within
             the reach, the earlier of two as near, else stays
  playing    a sampler at key 60, velocity 1, volume 1 and centered plays the edited sound's
             frames as they are, a mono sound on both sides; an audio clip at gain 1 the same;
             at key 67 the sampler's read of gen_p13 over the edited frames; a resampled clip
             gen_p9's read at j * S / T of the edited frames
  the file   a channel's "edits":{"ops":[...]} is written after its sound settings, only when it
             holds an op, and a project holding one writes format_version 8; a project without
             keeps its version and its bytes; the first op appended to a list (an empty one)
             writes "source":{"frames":n,"rate":r} after the ops, the sound's length then and
             the project's rate: a file of another length (or a project at another rate) is
             one the edits were not made on ("stale"), and they still apply

  p30_sound/*.wav     the sounds (32-bit float, 44.1 kHz)
  p30_<case>.json     the render cases' projects ("renders" below)
  p30_remap*.json     an audio clip at frame_offset 48000, and a stretched one, before a crop or
                      a delete ("remap": the documents after)
  p30_undo.json       a sampler and an audio channel with two clips, for the undo round trip
                      ("undo": the document after each op, applied one after another)
  p30_change.json     an edited sampler given another sound ("change": the documents after)
  p30_long.json       an audio clip of the ten-minute sound, edited ("long")
  p30_expected.json   every expected value the tests read: "ops" (the edit list applied to
                      frames), "renders" (through dawsome render), "remap", "undo", "change",
                      "zero_crossings", "long", "editor" (the sample editor window's: a crop of
                      a selection, through the window, and what the window holds of the
                      ten-minute sound), "stale" (that crop's project, its file swapped for
                      another length)

Run:
  python3 -I gen_p30_expected.py              print the known answers
  python3 -I gen_p30_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p30_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import copy, hashlib, importlib.util, json, math, os, struct, sys

HERE = os.path.dirname(os.path.abspath(__file__))


def module(name):
    """A generator beside this one, whose functions restate rules this one shares."""
    sys.dont_write_bytecode = True                   # no __pycache__ beside the fixtures
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, name + ".py"))
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


P11 = module("gen_p11_expected")
P13 = module("gen_p13_expected")
f32, exp2, hermite, J, t2s = P11.f32, P11.exp2, P11.hermite, P11.J, P11.t2s
SR, PPQ, BAR = P11.SR, P11.PPQ, P11.BAR
LOG2_10 = P11.LOG2_10
EDITS_VERSION = 8
BAR_FRAMES = t2s(BAR)

# ---------- the sounds: frames, each a list of its sides ----------
def mono(values): return [[f32(v)] for v in values]
def stereo(left, right): return [[f32(l), f32(r)] for l, r in zip(left, right)]


ONES = mono([1.0] * 8)
PAIR = stereo([1.0] * 8, [-0.5] * 8)
RAMP = mono([k / 65536 for k in range(48000)])           # exact in f32: k < 2^24
LONG = mono([k / 131072 for k in range(72000)])          # the clips' sound
RAMP64 = mono([(i + 1) / 64 for i in range(64)])
PEAK03 = mono([0.1, -0.3, 0.2, 0.25, -0.05, 0.0, 0.15, -0.2])
PEAK17 = stereo([0.5, 1.7, -1.2, 0.0, 0.25, -0.85], [0.25, -0.5, 1.0, 0.75, -1.7, 0.125])
SOUNDS = {"ones8": ONES, "pair8": PAIR, "ramp48000": RAMP, "ramp72000": LONG, "ramp64": RAMP64,
          "peak03": PEAK03, "peak17": PEAK17}


def path_of(sound): return "p30_sound/%s.wav" % sound


def wav_bytes(frames):
    sides = len(frames[0])
    return P11.wav_bytes([[frame[side] for frame in frames] for side in range(sides)])


# ---------- the edit list ----------
def db_gain(centi_db):
    return exp2(centi_db / 100 * LOG2_10 / 20)


def op(kind, start, end, **more):
    return {kind: dict({"from": start, "to": end}, **more)}


def cut(o, n):
    (kind, a), = o.items()
    return kind, a, min(a["from"], n), min(a["to"], n)


def apply(frames, ops):
    """The sound `frames` with `ops` applied in order (the rules above)."""
    frames = [list(f) for f in frames]
    for o in ops:
        kind, a, lo, hi = cut(o, len(frames))
        if lo >= hi: continue
        n = hi - lo
        scale = lambda factor: [[f32(x * factor(k)) for x in frames[lo + k]] for k in range(n)]
        if kind == "crop": frames = frames[lo:hi]
        elif kind == "delete": frames = frames[:lo] + frames[hi:]
        elif kind == "silence": frames[lo:hi] = [[0.0] * len(f) for f in frames[lo:hi]]
        elif kind == "fade_in": frames[lo:hi] = scale(lambda k: k / n)
        elif kind == "fade_out": frames[lo:hi] = scale(lambda k: (n - 1 - k) / n)
        elif kind == "gain":
            g = db_gain(a["centi_db"])
            frames[lo:hi] = scale(lambda k: g)
        elif kind == "normalize":
            peak = max((abs(x) for f in frames[lo:hi] for x in f if x == x), default=0.0)
            if peak == 0.0: continue
            g = db_gain(a.get("centi_db", 0)) / peak
            frames[lo:hi] = scale(lambda k: g)
        elif kind == "reverse": frames[lo:hi] = frames[lo:hi][::-1]
        else: raise ValueError(kind)
    return frames


def frames_after(o, n):
    """The sound's frames after `o` on a sound of `n`."""
    kind, _, lo, hi = cut(o, n)
    if lo >= hi: return n
    return {"crop": hi - lo, "delete": n - (hi - lo)}.get(kind, n)


def map_frame(o, n, f):
    kind, _, lo, hi = cut(o, n)
    if lo >= hi: return f
    if kind == "crop": return 0 if f < lo else (f - lo if f < hi else hi - lo)
    if kind == "delete": return f if f < lo else (lo if f < hi else f - (hi - lo))
    return f


def remapped(doc, channel, o, n):
    """`doc` after appending `o` to `channel`'s list, the clips of the channel moved."""
    doc = copy.deepcopy(doc)
    c = next(c for c in doc["channels"] if c["id"] == channel)
    edits = c.setdefault("edits", {"ops": []})
    first = not edits["ops"]
    edits["ops"].append(o)
    if first: edits["source"] = {"frames": n, "rate": doc["sample_rate"]}
    after = frames_after(o, n)
    for clip in doc.get("playlist", {}).get("clips", []):
        if clip.get("kind") != "audio" or clip["channel"] != channel: continue
        offset = clip.get("frame_offset", 0)
        stretch = clip.get("stretch")
        if stretch is None:
            offset = map_frame(o, n, offset)
        else:
            t = stretch["frames"]
            t2 = max(1, (2 * t * after + n) // (2 * n))
            offset = map_frame(o, n, offset * n // t) * t2 // after
            stretch["frames"] = t2
        if offset: clip["frame_offset"] = offset
        else: clip.pop("frame_offset", None)
    doc["format_version"] = version_of(doc)
    return doc


def version_of(doc):
    """The oldest version that holds `doc` (the project's rule), edits making it 8."""
    if any(c.get("edits") for c in doc["channels"]): return EDITS_VERSION
    if any(c.get("sound") for c in doc["channels"]): return 5
    if any(c["kind"] == "instrument" for c in doc["channels"]): return 4
    return 2


# ---------- projects ----------
def ordered_channel(c):
    """A channel's fields in the file's order: the edits after the sound settings."""
    order = ["id", "name", "kind", "sample", "instrument", "volume", "pan", "mute", "sound", "edits"]
    return {k: c[k] for k in order if k in c}


def finish(doc):
    doc = dict(doc)
    doc["channels"] = [ordered_channel(c) for c in doc["channels"]]
    doc = {k: doc[k] for k in ["format_version", "sample_rate", "tempo_mbpm", "ppq",
                               "next_channel_id", "channels", "patterns", "playlist"] if k in doc}
    return doc


def dump(doc):
    return P13.dump(finish(doc))


def sampler_doc(sound, ops, key=60, settings=None):
    c = P11.sampler(1, "Sound", path_of(sound))
    if settings: c["sound"] = P13.sound_json(settings)
    if ops: c["edits"] = {"ops": ops}
    doc = P11.project([c], [P11.note_json(1, 0, 0, key, 1.0)], version=2)
    doc["format_version"] = version_of(doc)
    return doc


def audio_clip(i, track, channel, offset=0, stretch=None, length=BAR):
    c = {"id": i, "track": track, "start": 0, "length": length, "kind": "audio", "channel": channel}
    if offset: c["frame_offset"] = offset
    if stretch: c["stretch"] = {"mode": "resample", "frames": stretch}
    return c


def audio_doc(channels, clips, tempo=P11.MBPM):
    doc = P11.project(channels, [], version=2)
    doc["tempo_mbpm"] = tempo
    doc["playlist"] = {"next_track_id": 9, "next_clip_id": len(clips) + 1, "tracks": P11.tracks(),
                       "clips": clips}
    doc["format_version"] = version_of(doc)
    return doc


def audio_channel(i, name, sound, ops=None):
    c = {"id": i, "name": name, "kind": "audio", "sample": path_of(sound), "volume": 1.0,
         "pan": 0.0, "mute": False}
    if ops: c["edits"] = {"ops": ops}
    return c


# ---------- the cases ----------
# Ops on frames alone: (sound, ops).
OPS = {
    "fade_in": ("ones8", [op("fade_in", 0, 4)]),
    "fade_out": ("ones8", [op("fade_out", 4, 8)]),
    "fades_stereo": ("pair8", [op("fade_in", 0, 4), op("fade_out", 4, 8)]),
    "fade_past_end": ("ones8", [op("fade_out", 4, 12)]),
    "past_end": ("ones8", [op("silence", 8, 20), op("crop", 9, 12)]),
    "crop": ("ramp48000", [op("crop", 12000, 36000)]),
    "delete": ("ramp48000", [op("delete", 10000, 20000)]),
    "normalize": ("peak03", [op("normalize", 0, 8)]),
    "normalize_hot": ("peak17", [op("normalize", 0, 6)]),
    "normalize_quiet": ("peak03", [op("normalize", 0, 8, centi_db=-100)]),
    "normalize_part": ("peak03", [op("normalize", 2, 5)]),
    "gain": ("ramp48000", [op("gain", 1000, 2000, centi_db=-600)]),
    "gain_up": ("pair8", [op("gain", 0, 8, centi_db=2400)]),
    "silence": ("ramp48000", [op("silence", 5000, 6000)]),
    "reverse": ("ramp64", [op("reverse", 0, 64)]),
    "reverse_twice": ("ramp64", [op("reverse", 0, 64), op("reverse", 0, 64)]),
    "reverse_part": ("pair8", [op("reverse", 2, 6)]),
    "chain": ("ramp64", [op("crop", 8, 56), op("gain", 0, 48, centi_db=-300), op("delete", 0, 4),
                         op("fade_in", 0, 10), op("reverse", 20, 44), op("normalize", 0, 44)]),
}
# The silent sound, with negative zeros: normalize leaves its bits as they are.
SILENT_BITS = [0.0, -0.0, 0.0, -0.0, -0.0, 0.0]
# A NaN in a normalize's range is no peak, first in it or all of it: the rest scales to the
# others' peak and the NaN stays. JSON holds no NaN: null stands for one ("values", "out").
NAN = float("nan")
NAN_CASES = {
    "nan_first": ([NAN, 0.5, -0.25, 0.1], [op("normalize", 0, 4)]),
    "nan_only": ([NAN, NAN, 0.0], [op("normalize", 0, 2)]),
}


def nan_case(name):
    values, ops = NAN_CASES[name]
    out = [x for f in apply(mono(values), ops) for x in f]
    nulled = lambda vs: [None if v != v else v for v in vs]
    return {"values": nulled([f32(v) for v in values]), "edits": {"ops": ops}, "out": nulled(out)}


def ops_case(name):
    sound, ops = OPS[name]
    out = apply(SOUNDS[sound], ops)
    case = {"sound": path_of(sound), "channels": len(SOUNDS[sound][0]), "edits": {"ops": ops},
            "frames": len(out)}
    flat = [x for f in out for x in f]
    if len(flat) <= 64: case["out"] = flat
    case["sha256"] = sha(flat)
    case["first"] = out[0] if out else None
    case["last"] = out[-1] if out else None
    return case


def sha(values):
    return hashlib.sha256(struct.pack("<%df" % len(values), *values)).hexdigest()


# Renders through dawsome render: (project, song mode).
def stereo_render(frames, length=BAR_FRAMES):
    """An audio clip or a key-60 sampler of `frames`: left and right as they are (a mono sound
    on both sides), added to the silence the mix starts from (so a -0.0 plays as 0.0)."""
    left = [0.0] * length; right = [0.0] * length
    for i, f in enumerate(frames[:length]):
        left[i], right[i] = 0.0 + f[0], 0.0 + f[-1]
    return left, right


def resample(src, total):
    """gen_p9_expected.py's resample read, restated: output j reads src at exactly j * S / T,
    frame i and f = r / T (f64), the 4-point Hermite of i - 1 .. i + 2 (outside the sound 0), on
    a frame the frame itself."""
    s = len(src); out = []
    at = lambda k: src[k] if 0 <= k < s else 0.0
    for j in range(total):
        i, r = divmod(j * s, total)
        out.append(at(i) if r == 0 else hermite(at(i - 1), at(i), at(i + 1), at(i + 2), r / total))
    return out


STRETCH_OPS = [op("crop", 8, 56), op("fade_out", 0, 48)]
CLIP_OPS = [op("reverse", 0, 64), op("gain", 0, 32, centi_db=-600)]
RENDERS = {
    "fade_in": (sampler_doc("ones8", OPS["fade_in"][1]), False),
    "fades_stereo": (sampler_doc("pair8", OPS["fades_stereo"][1]), False),
    "crop": (sampler_doc("ramp48000", OPS["crop"][1]), False),
    "delete": (sampler_doc("ramp48000", OPS["delete"][1]), False),
    "normalize": (sampler_doc("peak03", OPS["normalize"][1]), False),
    "normalize_hot": (sampler_doc("peak17", OPS["normalize_hot"][1]), False),
    "gain": (sampler_doc("ramp48000", OPS["gain"][1]), False),
    "silence": (sampler_doc("ramp48000", OPS["silence"][1]), False),
    "reverse60": (sampler_doc("ramp64", OPS["reverse"][1]), False),
    "reverse67": (sampler_doc("ramp64", OPS["reverse"][1], key=67), False),
    # The same sound reversed by its sound setting: the test holds the two renders' bytes equal.
    "reverse60_setting": (sampler_doc("ramp64", [], settings=P13.settings(reverse=True)), False),
    "reverse67_setting": (sampler_doc("ramp64", [], key=67, settings=P13.settings(reverse=True)),
                          False),
    "clip": (audio_doc([audio_channel(1, "Sound", "ramp64", CLIP_OPS)], [audio_clip(1, 1, 1)]),
             True),
    "stretch": (audio_doc([audio_channel(1, "Sound", "ramp64", STRETCH_OPS)],
                          [audio_clip(1, 1, 1, stretch=72)]), True),
}


def render_of(name):
    doc, _ = RENDERS[name]
    c = doc["channels"][0]
    sound = c["sample"][len("p30_sound/"):-len(".wav")]
    ops = c.get("edits", {}).get("ops", [])
    if name == "stretch":
        edited = [f[0] for f in apply(SOUNDS[sound], ops)]
        played = resample(edited, doc["playlist"]["clips"][0]["stretch"]["frames"])
        return stereo_render([[v] for v in played])
    if c["kind"] == "sampler" and len(SOUNDS[sound][0]) == 1:
        key = doc["patterns"][0]["notes"][0].get("key", 60)
        reverse = c.get("sound", {}).get("reverse", False)
        frames = [f[0] for f in apply(SOUNDS[sound], ops)]
        out, _ = P13.sampler_render(frames, P13.settings(reverse=reverse), [(0, 0, key, 1.0)])
        return out, out[:]
    return stereo_render(apply(SOUNDS[sound], ops))


def render_case(name):
    doc, song = RENDERS[name]
    left, right = render_of(name)
    flat = [x for pair in zip(left, right) for x in pair]
    case = {"fixture": "p30_%s.json" % name, "song": song, "frames": len(left), "sha256": sha(flat)}
    nonzero = P11.nonzero(left, right)
    case["sounding"] = len(nonzero)
    case["nonzero" if len(nonzero) <= 300 else "probes"] = (
        nonzero if len(nonzero) <= 300 else nonzero[:3] + nonzero[-3:])
    return case


# ---------- the clips' offsets: a crop and a delete in front of a clip ----------
# 480 BPM, so a bar (the clip) is 22050 frames: from 48000 it plays frames up to 70050 of 72000.
REMAP_TEMPO = 480000
REMAP = audio_doc([audio_channel(1, "Long", "ramp72000")],
                  [audio_clip(1, 1, 1, offset=48000)], tempo=REMAP_TEMPO)
# The stretched clip: resampled to 1.5 times the sound, from stretched frame 30000.
REMAP_STRETCH = audio_doc([audio_channel(1, "Long", "ramp72000")],
                          [audio_clip(1, 1, 1, offset=30000, stretch=108000)], tempo=REMAP_TEMPO)
REMAP_CASES = {
    "crop": ("p30_remap.json", REMAP, op("crop", 1000, 72000)),
    "delete": ("p30_remap.json", REMAP, op("delete", 10000, 20000)),
    "stretch_crop": ("p30_remap_stretch.json", REMAP_STRETCH, op("crop", 1000, 72000)),
    "stretch_delete": ("p30_remap_stretch.json", REMAP_STRETCH, op("delete", 10000, 15000)),
}


def remap_case(name):
    fixture, doc, o = REMAP_CASES[name]
    after = remapped(doc, 1, o, len(LONG))
    clip = after["playlist"]["clips"][0]
    return {"fixture": fixture, "channel": 1, "op": o, "frames": len(LONG),
            "after": dump(after), "frame_offset": clip.get("frame_offset", 0),
            "stretched": clip.get("stretch", {}).get("frames")}


# ---------- undo: every op, one step each, on one project ----------
UNDO = audio_doc([dict(P11.sampler(1, "Ramp", path_of("ramp48000"))),
                  audio_channel(2, "Long", "ramp72000")],
                 [audio_clip(1, 1, 2, offset=48000), audio_clip(2, 2, 2, offset=30000, stretch=108000)])
UNDO["patterns"][0]["notes"] = [P11.note_json(1, 0, 0, 60, 1.0)]
UNDO_OPS = [
    (2, op("crop", 1000, 72000)),
    (2, op("delete", 10000, 20000)),
    (1, op("fade_in", 0, 4800)),
    (1, op("fade_out", 43200, 48000)),
    (1, op("gain", 0, 48000, centi_db=-300)),
    (1, op("normalize", 0, 48000)),
    (1, op("reverse", 0, 48000)),
    (1, op("silence", 1000, 2000)),
    (2, op("gain", 0, 61000, centi_db=600)),
]


def undo_case():
    lengths = {1: len(RAMP), 2: len(LONG)}
    doc, steps = UNDO, []
    for channel, o in UNDO_OPS:
        frames = lengths[channel]
        doc = remapped(doc, channel, o, frames)
        lengths[channel] = frames_after(o, frames)
        steps.append({"channel": channel, "op": o, "frames": frames, "after": dump(doc),
                      "name": OP_NAMES[next(iter(o))]})
    return {"fixture": "p30_undo.json", "steps": steps}


OP_NAMES = {"crop": "Crop", "delete": "Delete part", "silence": "Silence", "fade_in": "Fade in",
            "fade_out": "Fade out", "gain": "Gain", "normalize": "Normalize",
            "reverse": "Reverse"}

# ---------- another sound on an edited channel ----------
CHANGE = sampler_doc("ramp64", [op("reverse", 0, 64), op("fade_in", 0, 16)])


def change_case():
    sample = copy.deepcopy(CHANGE)
    sample["channels"][0]["sample"] = path_of("ones8")
    del sample["channels"][0]["edits"]
    sample["format_version"] = version_of(sample)
    instrument = copy.deepcopy(CHANGE)
    instrument["channels"][0] = P11.instrument(1, "Pad", "instruments/pad/pad.sfz")
    instrument["format_version"] = version_of(instrument)
    clone = copy.deepcopy(CHANGE)
    copied = dict(copy.deepcopy(clone["channels"][0]), id=2, name="Sound #2")
    clone["channels"].append(copied)
    clone["next_channel_id"] = 3
    return {"fixture": "p30_change.json", "sample": {"path": path_of("ones8"), "after": dump(sample)},
            "instrument": {"path": "instruments/pad/pad.sfz", "name": "Pad",
                           "after": dump(instrument)},
            "clone": {"after": dump(clone)}}


# ---------- zero crossings ----------
TRIANGLE = [3, 2, 1, 0, -1, -2, -3, -2, -1, 0, 1, 2, 3, 2, 1, 0, -1, -2]
REACH = SR // 100                                     # 10 ms


def boundaries(values):
    out = []
    for i, v in enumerate(values):
        if v == 0.0 or (i > 0 and values[i - 1] * v < 0.0): out.append(i)
    return out


def snap(values, press, reach):
    near = [b for b in boundaries(values) if abs(b - press) <= reach]
    return min(near, key=lambda b: (abs(b - press), b)) if near else press


def zero_cases():
    tri = [f32(v / 4) for v in TRIANGLE]
    # A 2 kHz wave above 0 all through: no boundary anywhere, so an edge stays.
    above = [f32(0.5 + 0.25 * math.sin(2 * math.pi * 2000 * k / SR)) for k in range(1000)]
    # Stereo, its sides summed: they cancel at frame 2 and the sum crosses between 5 and 6.
    left = [0.5, 0.25, 0.25, 0.5, 0.25, 0.25, -0.75, -0.5]
    right = [0.25, 0.25, -0.25, 0.25, 0.25, -0.125, -0.25, 0.5]
    pair = [l + r for l, r in zip(left, right)]
    cases = []
    for press in (7, 5, 3, 0, 17):
        cases.append({"sound": "triangle", "channels": 1, "values": tri, "press": press,
                      "reach": REACH, "snap": snap(tri, press, REACH)})
    cases.append({"sound": "triangle", "channels": 1, "values": tri, "press": 7, "reach": 1,
                  "snap": snap(tri, 7, 1)})
    cases.append({"sound": "above", "channels": 1, "values": above, "press": 300, "reach": REACH,
                  "snap": snap(above, 300, REACH)})
    interleaved = [x for lr in zip(left, right) for x in lr]
    for press in (1, 4):
        cases.append({"sound": "pair", "channels": 2, "values": interleaved, "press": press,
                      "reach": REACH, "snap": snap(pair, press, REACH)})
    return cases


# ---------- ten minutes of stereo at 48 kHz, through the edited cache ----------
LONG_RATE = 48000
LONG_FRAMES = 10 * 60 * LONG_RATE
LONG_OPS = [op("fade_in", 0, 4800), op("gain", 48000, LONG_FRAMES, centi_db=-600)]


def long_frame(k):
    """The ten-minute sound's frame k: a 4096-frame sawtooth, the right side its half negated
    (exact in f32)."""
    left = ((k % 4096) - 2048) / 4096
    return [left, -left / 2]


def long_case():
    rendered = 96000                                  # one bar at 120 BPM, 48 kHz
    frames = [long_frame(k) for k in range(rendered)]
    # The ops on the frames a bar plays: the fade over the first 4800, the gain from 48000 on.
    out = apply(frames, [LONG_OPS[0], op("gain", 48000, rendered, centi_db=-600)])
    left, right = stereo_render(out, rendered)
    flat = [x for pair in zip(left, right) for x in pair]
    return {"fixture": "p30_long.json", "sound": "p30_long.wav", "rate": LONG_RATE,
            "frames": LONG_FRAMES, "channels": 2, "sawtooth": 4096,
            "cache_bytes": LONG_FRAMES * 2 * 4, "seconds": 120,
            "render": {"frames": rendered, "sha256": sha(flat),
                       "probes": [[k, left[k], right[k]] for k in (0, 1, 2400, 4799, 4800, 47999,
                                                                    48000, 95999)]}}


# ---------- the sample editor window (P1b) ----------
# A selection of frames 1000 to 2000 cropped through the window, on the audio channel of
# p30_undo.json (two clips, one stretched) and on its sampler (no clips): the document after,
# the clips' offsets moved by the crop's frame map, and the frames the window then shows.
EDITOR_CROP = op("crop", 1000, 2000)


def editor_case():
    crops = {}
    for channel, sound in ((1, RAMP), (2, LONG)):
        after = remapped(UNDO, channel, EDITOR_CROP, len(sound))
        clips = [c for c in after["playlist"]["clips"] if c["channel"] == channel]
        frames = apply(sound, [EDITOR_CROP])
        crops[str(channel)] = {
            "fixture": "p30_undo.json", "channel": channel, "selection": [1000, 2000],
            "op": EDITOR_CROP, "frames": len(frames), "sha256": sha([x for f in frames for x in f]),
            "first": frames[0], "last": frames[-1], "after": dump(after),
            "frame_offsets": [c.get("frame_offset", 0) for c in clips],
            "stretched": [c["stretch"]["frames"] for c in clips if "stretch" in c]}
    # What the window holds of the ten-minute sound: its frames as f32, and its peaks, a (min,
    # max) pair of f32s per side per 256 frames (the last pair for what is left).
    pairs = -(-LONG_FRAMES // 256)
    held = {"frames": LONG_FRAMES, "channels": 2, "buffer_bytes": LONG_FRAMES * 2 * 4,
            "peak_pairs": pairs, "peaks_bytes": pairs * 2 * 2 * 4}
    return {"crop": crops, "long": held, "gain_per_pixel": GAIN_PER_PIXEL}


# The Gain readout: a pixel of drag up or down is a tenth of a dB (10 hundredths).
GAIN_PER_PIXEL = 10


# ---------- a file changed since its edits (P1c) ----------
# The editor's crop of the audio channel (its list's source: 72000 frames at 44.1 kHz), its
# file swapped for the 48000-frame ramp: stale, and the crop still applies, by frame, to what
# the file holds now.
def stale_case():
    crop = editor_case()["crop"]["2"]
    frames = apply(RAMP, [EDITOR_CROP])
    return {"after": crop["after"], "channel": 2, "source": {"frames": len(LONG), "rate": SR},
            "sample": path_of("ramp48000"), "file_frames": len(RAMP), "frames": len(frames),
            "first": frames[0], "last": frames[-1]}


LONG_DOC = audio_doc([audio_channel(1, "Long", "ramp64", LONG_OPS)], [audio_clip(1, 1, 1)])
LONG_DOC["sample_rate"] = LONG_RATE
LONG_DOC["channels"][0]["sample"] = "p30_long.wav"


# ---------- the files ----------
def expected():
    return {"ops": {name: ops_case(name) for name in OPS},
            "silent": {"values": SILENT_BITS, "edits": {"ops": [op("normalize", 0, 6)]}},
            "nan": {name: nan_case(name) for name in NAN_CASES},
            "renders": {name: render_case(name) for name in RENDERS},
            "remap": {name: remap_case(name) for name in REMAP_CASES},
            "undo": undo_case(), "change": change_case(), "zero_crossings": zero_cases(),
            "long": long_case(), "editor": editor_case(), "stale": stale_case(),
            "names": OP_NAMES}


def files():
    """Every file --write makes, by its path under DIR, as bytes."""
    out = {}
    for name, frames in SOUNDS.items():
        out[path_of(name)] = wav_bytes(frames)
    for name, (doc, _) in RENDERS.items():
        out["p30_%s.json" % name] = (dump(doc) + "\n").encode()
    out["p30_remap.json"] = (dump(REMAP) + "\n").encode()
    out["p30_remap_stretch.json"] = (dump(REMAP_STRETCH) + "\n").encode()
    out["p30_undo.json"] = (dump(UNDO) + "\n").encode()
    out["p30_change.json"] = (dump(CHANGE) + "\n").encode()
    out["p30_long.json"] = (dump(LONG_DOC) + "\n").encode()
    out["p30_expected.json"] = (J(expected()) + "\n").encode()
    return out


def sanity():
    """What the spec states outright, held here so a rule restated wrong fails at once."""
    crop = apply(RAMP, OPS["crop"][1])
    assert len(crop) == 24000 and crop[0] == [12000 / 65536] and crop[-1] == [35999 / 65536]
    delete = apply(RAMP, OPS["delete"][1])
    assert len(delete) == 38000 and delete[10000] == [20000 / 65536]
    assert [f[0] for f in apply(ONES, OPS["fade_in"][1])] == [0, 0.25, 0.5, 0.75, 1, 1, 1, 1]
    assert [f[0] for f in apply(ONES, OPS["fade_out"][1])][4:] == [0.75, 0.5, 0.25, 0]
    assert min(f[0] for f in apply(PEAK03, OPS["normalize"][1])) == -1.0
    assert max(abs(x) for f in apply(PEAK17, OPS["normalize_hot"][1]) for x in f) == 1.0
    quiet = min(f[0] for f in apply(PEAK03, OPS["normalize_quiet"][1]))
    assert quiet == -f32(exp2(-0.05 * LOG2_10)), quiet
    assert apply(RAMP64, OPS["reverse_twice"][1]) == RAMP64
    assert nan_case("nan_first")["out"] == [None, 1.0, -0.5, f32(0.2)]
    assert nan_case("nan_only")["out"] == [None, None, 0.0]
    assert remap_case("crop")["frame_offset"] == 47000
    assert remap_case("delete")["frame_offset"] == 38000
    # The window's crop of 1000..2000 on the audio channel: the clip at 48000 (past the crop) at
    # its end, 1000; the stretched one (30000 of 108000, the sound's frame 20000) at the crop's
    # end too, its 108000 frames over 72000 now over 1000, so 1500 frames, offset 1500.
    crop = editor_case()["crop"]["2"]
    assert crop["frame_offsets"] == [1000, 1500] and crop["stretched"] == [1500], crop
    assert crop["frames"] == 1000 and crop["first"] == [1000 / 131072]
    assert editor_case()["long"]["buffer_bytes"] == 230400000
    assert editor_case()["long"]["peaks_bytes"] == 1800000
    # The crop's document keeps what its first op found: 72000 frames at 44.1 kHz.
    assert '"source":{"frames":72000,"rate":44100}' in editor_case()["crop"]["2"]["after"]
    assert stale_case()["frames"] == 1000 and stale_case()["first"] == [1000 / 65536]
    left, _ = render_of("reverse67")
    same, _ = render_of("reverse67_setting")
    assert left == same and render_of("reverse60") == render_of("reverse60_setting")
    tri = [f32(v / 4) for v in TRIANGLE]
    assert boundaries(tri) == [3, 9, 15] and snap(tri, 7, REACH) == 9 and snap(tri, 5, REACH) == 3


def main():
    sanity()
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
        if stale: sys.exit("not what gen_p30_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    answers = expected()
    for name, case in answers["ops"].items():
        print("ops %-16s %6d frames, first %s, last %s" % (name, case["frames"], case["first"],
                                                           case["last"]))
    for name, case in answers["renders"].items():
        print("render %-18s %d sounding" % (name, case["sounding"]))
    for name, case in answers["remap"].items():
        print("remap %-15s offset %d, stretched %s" % (name, case["frame_offset"], case["stretched"]))
    print("zero crossings", [(c["press"], c["snap"]) for c in answers["zero_crossings"]])
    print("gain -600: %.17g; -1 dB normalize lands on %.9g" % (db_gain(-600),
                                                             f32(exp2(-0.05 * LOG2_10))))


if __name__ == "__main__":
    main()
