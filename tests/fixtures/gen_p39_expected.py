"""Known answers for an audio clip's warp markers and for the transients found in a sound.

Expected values come from the rules written out below (restated from the design, never from
engine code) and from each signal's construction. gen_p30_expected.py's f32, SHA-256 and Hermite
helpers and gen_p38_expected.py's declick rules are imported. Every input is authored here: the
WAVs and the projects.

The rules:
  s(t)       the one tick-to-sample rule, floor(t * 60 * rate * 1000 / (mbpm * ppq)); 120 BPM
             at 48 kHz here (s(t) = 250 t), and at 44.1 kHz
  marker     a pair (source, out): `source` a frame of the channel's (edited) sound, S frames
             long, `out` a frame of the stretched file, T = the stretch's frames long
  anchors    (0, 0), the markers in order, (S, T); the map is the straight line between
             neighbouring anchors
  map        out_of_source(s): s past S counts as S; in the first segment whose end s1 >= s,
             o0 + round_half_up((s - s0) * (o1 - o0) / (s1 - s0)), in whole numbers; and
             source_of_out the same with the columns swapped. Exact at every anchor
  half up    round_half_up(n / d) = (2 n + d) // (2 d)
  rules      at most 256 markers; neighbouring anchors (both ends included) at least 64 frames
             apart in BOTH columns, so each column rises; each segment's ratio out / source
             from 1/4 to 4 (a uniform stretch's range). Each list keeps the S of the sound it
             was placed on, so a load and every edit check each segment, the last (from the last
             marker to (S, T)) too
  file       a project with any marker is format_version 13 (it wins over every other); the
             markers live beside the clips, in the playlist after "clips":
             "warps":[{"clip":<id>,"frames":S,"markers":[{"source":s,"out":o},...]},...], by
             clip id, an entry only for a clip with markers, and only for a stretched audio
             clip; S is the sound's frames when the first marker was set (an edit of the sound
             that changes its length is refused while it has markers, so S stays)
  resample   output frame j of segment i (o_i <= j < o_i+1) reads the sound at
             s_i + (j - o_i) * (s_i+1 - s_i) / (o_i+1 - o_i): frame s_i + n // d and fraction
             (n mod d) / d (f64), the 4-point Hermite of the frames around it (outside the sound
             0), on a frame (n mod d = 0) the frame itself; exactly T frames. With no markers
             it is the uniform stretch's read j * S / T (gen_p9's), bit for bit
  playing    gen_p38's: a clip plays stretched frame off + k at song sample s(start) + k, its
             fades and the 2 ms declick where an edge cuts in; a seam is no cut only when the
             later clip goes on with the earlier's sound, the marker list included (here the
             stretch compared is the stretch and the markers together)
  edits      set warp replaces a clip's list (empty: the entry goes); its undo gives the bytes
             back. A new stretch length T' scales every out by T' / T half up, refused when
             that breaks a rule (the last segment's ratio included); another mode or pitch
             keeps them; no stretch takes them away.
             Starting a warp on an unstretched clip first stretches it, mode "stretch", T = the
             sound's frames S, its offset and length kept. A copy of a clip copies its markers;
             a split gives both parts the whole list. An edit of the sound that changes its
             length is refused while a clip of its channel has markers
  onsets     the energy of each block of 128 frames: the sum of the squares of every side's
             values (f64), frame by frame and side by side in order, the last block what is
             left; a block whose energy is not finite (a NaN or an infinity in it) is no onset
             and stays out of every mean; block b is an onset when its energy is above K = 2.5
             times the mean of the 16 blocks before it (those before the sound, and those not
             finite, counting 0; the sum taken in order, then / 16) plus FLOOR = 1.28e-4 (a
             block at -60 dBFS), and at least 24 blocks after the last onset; its frame is the
             first of the blocks b and b + 1 whose magnitude (the largest |value| of its sides,
             a value not finite counting 0) reaches a quarter of their peak. Why every side
             and not a mix: a click whose sides are in opposite phase sums to nothing

  quantize   on each selected audio clip, every onset s of its sound whose out
             o = out_of_source(s) (T = S and no markers on an unstretched clip) lies in the part
             the clip plays (frame_offset <= o < frame_offset + its song frames) is a hit, in
             rising order; its song frame f = s(start) + o - frame_offset; t0 the last tick with
             s(t0) <= f; the lines either side, down = t0 on the snap's grid (a sixteenth, 24
             ticks, when the snap is None) and up the next; the nearer of s(down) and s(up) to f
             wins, half way going on; the hit's marker is (s, frame_offset + s(line) - s(start)),
             a marker already at s replaced. A hit whose line an earlier hit took is skipped, and
             so is one whose marker breaks a rule of the list built so far (the clip's markers
             and the hits kept before it). An unstretched clip is first stretched as starting a
             warp does (T = S). The notice counts the hits kept and all of them
  p39_sound/*.wav      the sounds (32-bit float, mono unless named): "tone" 24000 frames of the
                       saw ((k * 7919) mod 4096) / 8192 - 0.25 at 48 kHz, "tone44" 22050 of it
                       at 44.1 kHz, "short" 16448 of it at 48 kHz; the onset sounds at 48 kHz:
                       "clicks", "steady" (a triangle of period 128, one block), "close",
                       "step6", "step2", "nan" (silence with a NaN), "qclicks" (clicks off the
                       sixteenths at 120 BPM, for quantize), and of two sides "stereo",
                       "in_phase" and "inverted" (the clicks, the right side the same or
                       turned over)
  p39_<case>.json      the projects (one bar, the song ending on its bar line)
  p39_expected.json    "map" (the table), "renders" (each case's frames, SHA-256 of the
                       interleaved floats, frames sounding and probes), "documents" (the
                       version a warped project writes, the refusal texts, the broken files),
                       "edits" (each edit, the document after and the bytes its undo gives),
                       "refused" (edits that change nothing), "onsets" and "quantize" (the
                       hits, the marker list, the counts and the document after)

Run:
  python3 -I gen_p39_expected.py              print the known answers
  python3 -I gen_p39_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p39_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import copy, importlib.util, json, math, os, struct, sys

HERE = os.path.dirname(os.path.abspath(__file__))


def module(name):
    """A generator beside this one, whose functions restate rules this one shares."""
    sys.dont_write_bytecode = True                   # no __pycache__ beside the fixtures
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, name + ".py"))
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


P30 = module("gen_p30_expected")
P38 = module("gen_p38_expected")
f32, hermite = P30.f32, P30.hermite
MBPM, PPQ = 120000, 96
BAR = 4 * PPQ
MIN_SEGMENT, MAX_MARKERS, MAX_RATIO = 64, 256, 4
WARP_VERSION = 13


def s(t, rate=48000, mbpm=MBPM): return t * 60 * rate * 1000 // (mbpm * PPQ)


def J(d): return json.dumps(d, separators=(",", ":"), ensure_ascii=False)


# ---------- the map ----------
def half_up(n, d): return (2 * n + d) // (2 * d)


def anchors(markers, S, T): return [(0, 0)] + [tuple(m) for m in markers] + [(S, T)]


def out_of_source(markers, S, T, x):
    x = min(x, S)
    a = anchors(markers, S, T)
    for (s0, o0), (s1, o1) in zip(a, a[1:]):
        if x <= s1:
            return o0 + half_up((x - s0) * (o1 - o0), s1 - s0) if s1 > s0 else o0
    return T


def source_of_out(markers, S, T, y):
    swapped = [(o, x) for x, o in markers]
    return out_of_source(swapped, T, S, y)


def problems(markers, T, S):
    """The rules `markers` breaks on a stretch of T frames of a sound of S frames, as words;
    none when it keeps them all."""
    found = []
    if len(markers) > MAX_MARKERS: found.append("too many")
    ends = anchors(markers, S, T)
    for (s0, o0), (s1, o1) in zip(ends, ends[1:]):
        if o1 - o0 < MIN_SEGMENT: found.append("too close")
        if s1 - s0 < MIN_SEGMENT: found.append("too close")
        elif not (s1 - s0 <= MAX_RATIO * (o1 - o0) and o1 - o0 <= MAX_RATIO * (s1 - s0)):
            found.append("ratio")
    return found


MAP_CASES = [
    # (markers, S, T, the sources asked, the outs asked)
    ([(12000, 24000)], 24000, 36000, [0, 1, 6000, 11999, 12000, 12001, 18000, 24000, 30000],
     [0, 1, 2, 12000, 23999, 24000, 24001, 30000, 36000, 40000]),
    # Ties: the first segment 4 -> 6 (s = 1 is 1.5, s = 3 is 4.5: both up), 6 -> 4 back (o = 1
    # is 1.5: up).
    ([(4, 6)], 8, 10, [0, 1, 2, 3, 4, 5, 8], [0, 1, 2, 3, 4, 5, 6, 7, 10]),
    ([(6, 4)], 10, 8, [0, 1, 2, 3, 6, 10], [0, 1, 2, 3, 4, 8]),
    ([(4000, 6000), (10000, 12000), (20000, 21000)], 24000, 30000,
     [0, 3999, 4000, 7000, 10000, 15001, 20000, 22000, 24000],
     [0, 5999, 6000, 9001, 12000, 16500, 21000, 25500, 30000]),
    ([], 24000, 36000, [0, 1, 2, 3, 12000, 23999, 24000], [0, 1, 2, 3, 18000, 35999, 36000]),
]


def map_table():
    return [{"markers": [list(m) for m in markers], "source_frames": S, "frames": T,
             "out_of_source": [[x, out_of_source(markers, S, T, x)] for x in xs],
             "source_of_out": [[y, source_of_out(markers, S, T, y)] for y in ys]}
            for markers, S, T, xs, ys in MAP_CASES]


# ---------- the sounds ----------
def saw(n): return [((k * 7919) % 4096) / 8192 - 0.25 for k in range(n)]   # exact in f32


def triangle(n, height=0.25):
    """Period 128 (one block), from 0 up: every block's energy the same."""
    return [height * (32 - abs((k + 32) % 128 - 64)) / 32 for k in range(n)]


def silence_with(n, at, height=1.0):
    values = [0.0] * n
    for k in at: values[k] = height
    return values


def turned_over(values): return [-v for v in values]


GAIN_6DB = 1.9952623149688795        # 10^(6/20), as a literal so no platform's pow differs
GAIN_2DB = 1.2589254117941673        # 10^(2/20)
STEP = 24064                         # block 188's first frame
CLICKS = [1000, 9000, 20000, 33333, 47000]
# Quantize's clicks, on a clip at tick 10 (song frame 2500) playing the sound from its start: at
# song frames 6700 (700 late), 11100 (900 early), 26999 (2999 late: still the line before),
# 33000 (3000 late: half way, so the line after), 48100, then 69100 and 74900, which both want
# 72000 (the second is skipped), and 95000, past the clip's end (song frame 96000 is the bar's).
QCLICKS = [4200, 8600, 24499, 30500, 45600, 66600, 72400, 95000]
SOUNDS = {
    "tone": (48000, [saw(24000)]),
    "tone44": (44100, [saw(22050)]),
    "short": (48000, [saw(16448)]),
    "clicks": (48000, [silence_with(48000, CLICKS)]),
    "steady": (48000, [triangle(48000)]),
    "close": (48000, [silence_with(48000, [5000, 6000])]),
    "step6": (48000, [triangle(STEP) + [f32(v * GAIN_6DB) for v in triangle(48000)[STEP:]]]),
    "step2": (48000, [triangle(STEP) + [f32(v * GAIN_2DB) for v in triangle(48000)[STEP:]]]),
    "stereo": (48000, [silence_with(48000, [3000, 30000]), [0.0] * 48000]),
    "nan": (48000, [silence_with(48000, [5000], math.nan)]),
    "qclicks": (48000, [silence_with(96000, QCLICKS)]),
    "in_phase": (48000, [silence_with(48000, CLICKS), silence_with(48000, CLICKS)]),
    "inverted": (48000, [silence_with(48000, CLICKS), turned_over(silence_with(48000, CLICKS))]),
}


def path_of(sound): return "p39_sound/%s.wav" % sound


def wav_bytes(rate, sides):
    """A 32-bit float WAV of one or two sides."""
    n = len(sides)
    flat = [v for frame in zip(*sides) for v in frame]
    data = struct.pack("<%df" % len(flat), *flat)
    fmt = struct.pack("<HHIIHH", 3, n, rate, rate * 4 * n, 4 * n, 32)
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack(
        "<I", len(data)) + data
    return b"RIFF" + struct.pack("<I", len(body)) + body


def values_of(sound): return SOUNDS[sound][1][0]


# ---------- projects, in the file's field order ----------
def tracks(): return [{"id": i, "name": "Track %d" % i, "mute": False} for i in range(1, 9)]


def channel(i, sound):
    return {"id": i, "name": sound.capitalize(), "kind": "audio", "sample": path_of(sound),
            "volume": 1.0, "pan": 0.0, "mute": False}


def audio(i, start, length, stretch=None, offset=0, mode="resample", fade_in=0, fade_out=0,
          track=1):
    c = {"id": i, "track": track, "start": start, "length": length, "kind": "audio",
         "channel": 1}
    if offset: c["frame_offset"] = offset
    if stretch: c["stretch"] = {"mode": mode, "frames": stretch}
    if fade_in: c["fade_in"] = fade_in
    if fade_out: c["fade_out"] = fade_out
    return c


def ordered(c): return {k: c[k] for k in P38.ORDER if k in c}


def warp_entries(warps, S):
    """The warps table of marker lists by clip id, each placed on a sound of S frames."""
    return [entry(clip, warps[clip], S) for clip in sorted(warps) if warps[clip]]


def entry(clip, pairs, S):
    return {"clip": clip, "frames": S, "markers": [{"source": x, "out": o} for x, o in pairs]}


def version(clips, warps):
    if any(warps.values()): return WARP_VERSION
    return 12 if any(c.get("fade_in") or c.get("fade_out") for c in clips) else 2


def project(sound, clips, warps=None, rate=48000, edits=None):
    """One audio channel of `sound`; `warps` the marker lists by clip id."""
    warps = warps or {}
    ch = channel(1, sound)
    if edits: ch["edits"] = edits
    playlist = {"next_track_id": 9, "next_clip_id": max(c["id"] for c in clips) + 1 if clips
                else 1, "tracks": tracks(), "clips": [ordered(c) for c in clips]}
    if any(warps.values()): playlist["warps"] = warp_entries(warps, len(values_of(sound)))
    doc = {"format_version": version(clips, warps), "sample_rate": rate, "tempo_mbpm": MBPM,
           "ppq": PPQ, "next_channel_id": 2, "channels": [ch],
           "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": []}],
           "playlist": playlist}
    if edits: doc["format_version"] = max(doc["format_version"], 8)
    return doc


def markers_of(doc, clip):
    for entry in doc["playlist"].get("warps", []):
        if entry["clip"] == clip: return [(m["source"], m["out"]) for m in entry["markers"]]
    return []


# ---------- what plays, by the rules ----------
def warp_resample(src, markers, T):
    """The resample read through the markers: exactly T frames."""
    S = len(src); out = []
    at = lambda k: src[k] if 0 <= k < S else 0.0
    a = anchors(markers, S, T)
    for (s0, o0), (s1, o1) in zip(a, a[1:]):
        d = o1 - o0
        for j in range(o0, o1):
            i, r = divmod((j - o0) * (s1 - s0), d)
            i += s0
            out.append(at(i) if r == 0 else hermite(at(i - 1), at(i), at(i + 1), at(i + 2), r / d))
    return out


def sound_of(doc): return doc["channels"][0]["sample"][len("p39_sound/"):-len(".wav")]


def sound_frames(doc): return len(values_of(sound_of(doc)))


def source(doc, c):
    values = values_of(sound_of(doc))
    if c.get("stretch"): values = warp_resample(values, markers_of(doc, c["id"]),
                                                c["stretch"]["frames"])
    return values


def lengths(doc, c):
    """(N_in, N_out) of clip `c`, by gen_p38's rule with each clip's stretch taken together
    with its markers: what makes the two sides of a seam the same sound."""
    rate = doc["sample_rate"]
    clips = [dict(x, stretch=(json.dumps(x.get("stretch"), sort_keys=True),
                              markers_of(doc, x["id"])))
             for x in doc["playlist"]["clips"]]
    this = next(x for x in clips if x["id"] == c["id"])
    return P38.edge_lengths(clips, this, lambda t: s(t, rate), len(source(doc, c)), rate)


def render(doc):
    """Left (= right) of one bar in song mode."""
    rate = doc["sample_rate"]
    total = s(BAR, rate)
    out = [0.0] * total
    for c in sorted(doc["playlist"]["clips"], key=lambda c: c["id"]):
        start, end = s(c["start"], rate), s(c["start"] + c["length"], rate)
        src = source(doc, c)
        off, gain = c.get("frame_offset", 0), f32(c.get("gain", 1.0))
        n_in, n_out = lengths(doc, c)
        for n in range(start, min(end, total)):
            k = off + n - start
            value = src[k] if k < len(src) else 0.0
            value = f32(value * P38.factor(n - start, end - 1 - n, n_in, n_out))
            out[n] = f32(out[n] + f32(value * gain))
    return out


def render_case(doc):
    left = render(doc)
    flat = [x for v in left for x in (v, v)]
    nonzero = [i for i, v in enumerate(left) if v != 0.0]
    probes = sorted(set(nonzero[:3] + nonzero[-3:] + nonzero[::3000]))
    return {"frames": len(left), "sha256": P30.sha(flat), "sounding": len(nonzero),
            "lengths": [[c["id"]] + list(lengths(doc, c)) for c in doc["playlist"]["clips"]],
            "probes": [[i, left[i], left[i]] for i in probes]}


# ---------- the cases ----------
M1 = [(12000, 24000)]                                      # tone, T 36000: ratios 2 and 1
M3 = [(4000, 6000), (10000, 12000), (20000, 21000)]        # tone, T 30000: 1.5, 1, 0.9, 2.25
M1_44 = [(11025, 22050)]                                   # tone44, T 33075
M3_44 = [(3675, 5512), (9187, 11025), (18375, 19845)]      # tone44, T 33075: last 3.6
# The messy case: 256 markers on the short sound (16448 = 64 * 257 frames), the sources at the
# least spacing, the outs 64 and 128 apart in turn, the last segment 64 -> 64.
MESSY = [(64 * k, sum(64 if j % 2 else 128 for j in range(1, k + 1))) for k in range(1, 257)]
MESSY_T = MESSY[-1][1] + 64
# One marker more than a clip holds, each rule kept else (on plain's 36000 frames).
TOO_MANY = [(64 * k, 64 * k) for k in range(1, MAX_MARKERS + 2)]


def split_at(c, t, new_id, rate=48000):
    """gen_p38's split: the left keeps the fade in, the right the fade out and moves its
    offset by the frames between."""
    left, right = copy.deepcopy(c), copy.deepcopy(c)
    left["length"] = t - c["start"]
    left.pop("fade_out", None)
    right["id"] = new_id
    right["start"], right["length"] = t, c["start"] + c["length"] - t
    right["frame_offset"] = c.get("frame_offset", 0) + s(t, rate) - s(c["start"], rate)
    right.pop("fade_in", None)
    return [left, right]


WARP3 = audio(1, 264, 120, stretch=30000)
WARP3_FADED = audio(1, 264, 120, stretch=30000, fade_in=2000, fade_out=3000)
M3_MOVED = M3[:2] + [(20000, 21064)]
CASES = {
    # No markers: the uniform stretch (gen_p9's read), and the file's bits unchanged.
    "plain": project("tone", [audio(1, 240, 144, stretch=36000)]),
    # One marker: the first half of the sound made twice as long, the second as it is.
    "warp1": project("tone", [audio(1, 240, 144, stretch=36000)], {1: M1}),
    # Three markers, the clip all of its 30000 frames.
    "warp3": project("tone", [WARP3], {1: M3}),
    # The same at 44.1 kHz, one marker and three.
    "warp1_44": project("tone44", [audio(1, 240, 144, stretch=33075)], {1: M1_44}, rate=44100),
    "warp3_44": project("tone44", [audio(1, 240, 144, stretch=33075)], {1: M3_44}, rate=44100),
    # warp3 split at 300: both parts keep the whole list and meet with no declick, the bits of
    # the uncut clip.
    "warp3_split": project("tone", split_at(WARP3, 300, 2), {1: M3, 2: M3}),
    # Faded 2000 in and 3000 out, whole and split.
    "warp3_faded": project("tone", [WARP3_FADED], {1: M3}),
    "warp3_faded_split": project("tone", split_at(WARP3_FADED, 300, 2), {1: M3, 2: M3}),
    # The split's right part with another list, or none: another sound, so the seam declicks.
    "seam_moved": project("tone", split_at(WARP3, 300, 2), {1: M3, 2: M3_MOVED}),
    "seam_unwarped": project("tone", split_at(WARP3, 300, 2), {1: M3}),
    # 256 markers at the least spacing on a short sound.
    "messy": project("short", [audio(1, 285, 99, stretch=MESSY_T)], {1: MESSY}),
}


# ---------- the documents ----------
def dump(doc): return J(doc)


REFUSED_BY_12 = P38.REFUSED_13
REFUSED_14 = ("format_version 14 is not supported (this build reads versions 1, 2, 4, 5, 6, 7, "
              "8, 9, 10, 11, 12 and 13)")


def with_warps(doc, entries):
    """`doc` with its warps table replaced by `entries` as written (a broken one allowed)."""
    broken = copy.deepcopy(doc)
    broken["playlist"]["warps"] = entries
    broken["format_version"] = WARP_VERSION
    return broken


UNSTRETCHED = project("tone", [audio(1, 240, 144)])
LAST_RATIO = [(21000, 20000)]
PATTERN_CLIP = copy.deepcopy(CASES["warp1"])
PATTERN_CLIP["playlist"]["clips"].append({"id": 2, "track": 2, "start": 0, "length": 384,
                                          "kind": "pattern", "pattern": 1})
PATTERN_CLIP["playlist"]["next_clip_id"] = 3
BROKEN = {
    # (the document, what it breaks)
    "no_clip": with_warps(CASES["plain"], [entry(7, M1, 24000)]),
    "pattern_clip": with_warps(PATTERN_CLIP, [entry(1, M1, 24000), entry(2, M1, 24000)]),
    "unstretched": with_warps(UNSTRETCHED, [entry(1, M1, 24000)]),
    "empty_list": with_warps(CASES["plain"], [entry(1, [], 24000)]),
    "twice": with_warps(CASES["plain"], [entry(1, M1, 24000), entry(1, M1, 24000)]),
    "not_rising": with_warps(CASES["plain"],
                             [entry(1, [(12000, 24000), (11000, 30000)], 24000)]),
    "too_close": with_warps(CASES["plain"], [entry(1, [(12000, 24000), (12063, 24100)], 24000)]),
    "past_the_end": with_warps(CASES["plain"], [entry(1, [(12000, 35937)], 24000)]),
    "at_the_start": with_warps(CASES["plain"], [entry(1, [(63, 100)], 24000)]),
    "ratio": with_warps(CASES["plain"], [entry(1, [(1000, 4001)], 24000)]),
    "too_many": with_warps(CASES["plain"], [entry(1, TOO_MANY, 24000)]),
    # Only the last segment breaks a rule: (21000, 20000) to (24000, 36000) is 3000 frames
    # made 16000, more than four times (the first, 21000 made 20000, keeps them).
    "last_ratio": with_warps(CASES["plain"], [entry(1, LAST_RATIO, 24000)]),
    # An entry without the sound's frames.
    "no_frames": with_warps(CASES["plain"], [{"clip": 1, "markers": [{"source": 12000,
                                                                      "out": 24000}]}]),
}


# ---------- the edits ----------
def set_warp(doc, clip, markers):
    after = copy.deepcopy(doc)
    warps = {e["clip"]: [(m["source"], m["out"]) for m in e["markers"]]
             for e in doc["playlist"].get("warps", [])}
    warps[clip] = list(markers)
    after["playlist"].pop("warps", None)
    if any(warps.values()):
        after["playlist"] = dict(after["playlist"], warps=warp_entries(warps, sound_frames(doc)))
        after["playlist"] = {k: after["playlist"][k] for k in
                             ["next_track_id", "next_clip_id", "tracks", "clips", "warps"]}
    after["format_version"] = version(after["playlist"]["clips"], warps)
    return after


def set_stretch(doc, clip, stretch, mode="resample"):
    """The stretch edit with a new mode or length (`None`: none), the offset and length kept;
    the markers scaled by the new length half up, or taken away with the stretch."""
    after = copy.deepcopy(doc)
    c = next(x for x in after["playlist"]["clips"] if x["id"] == clip)
    old = c["stretch"]["frames"]
    markers = markers_of(doc, clip)
    if stretch is None:
        c.pop("stretch")
        markers = []
    else:
        c["stretch"] = {"mode": mode, "frames": stretch}
        markers = [(x, half_up(o * stretch, old)) for x, o in markers]
    return set_warp(after, clip, markers)


def start_warp(doc, clip, sound_frames, markers):
    """A first marker on an unstretched clip: stretched to the sound's own frames in the mode
    that keeps the pitch, then the markers."""
    after = copy.deepcopy(doc)
    c = next(x for x in after["playlist"]["clips"] if x["id"] == clip)
    c["stretch"] = {"mode": "stretch", "frames": sound_frames}
    after["playlist"]["clips"] = [ordered(x) for x in after["playlist"]["clips"]]
    return set_warp(after, clip, markers)


def duplicated(doc, clip, new_id):
    after = copy.deepcopy(doc)
    c = copy.deepcopy(next(x for x in after["playlist"]["clips"] if x["id"] == clip))
    c["id"], c["start"] = new_id, c["start"] + c["length"]
    after["playlist"]["clips"].append(c)
    after["playlist"]["next_clip_id"] = new_id + 1
    return set_warp(after, new_id, markers_of(doc, clip))


def deleted(doc, clip):
    after = copy.deepcopy(doc)
    after["playlist"]["clips"] = [x for x in after["playlist"]["clips"] if x["id"] != clip]
    return set_warp(after, clip, [])


GAIN_OP = {"gain": {"from": 0, "to": 24000, "centi_db": -600}}   # gen_p30's op shape
CROP_OP = {"crop": {"from": 100, "to": 24000}}
def set_warp_edit(clip, markers, sound_frames=24000):
    """The marker edit as a case names it: the list and the frames of the sound it is set on."""
    return {"edit": "set_warp", "clip": clip, "sound_frames": sound_frames, "markers": markers}


EDITS = [
    # (name, fixture, the edit, the document after)
    ("add", "p39_plain.json", set_warp_edit(1, M1),
     set_warp(CASES["plain"], 1, M1)),
    ("replace", "p39_warp1.json",
     set_warp_edit(1, [(6000, 9000)] + M1),
     set_warp(CASES["warp1"], 1, [(6000, 9000)] + M1)),
    ("remove", "p39_warp1.json", set_warp_edit(1, []),
     set_warp(CASES["warp1"], 1, [])),
    ("start", "p39_unstretched.json",
     {"edit": "start_warp", "clip": 1, "sound_frames": 24000, "markers": [(12000, 12000)]},
     start_warp(UNSTRETCHED, 1, 24000, [(12000, 12000)])),
    # 30000 -> 30005: the outs 6000, 12000, 21000 become 6001, 12002 and 21003.5 up to 21004.
    ("scale", "p39_warp3.json", {"edit": "set_stretch", "clip": 1, "mode": "resample",
                                 "frames": 30005},
     set_stretch(CASES["warp3"], 1, 30005)),
    ("scale_down", "p39_warp3.json", {"edit": "set_stretch", "clip": 1, "mode": "resample",
                                      "frames": 20000},
     set_stretch(CASES["warp3"], 1, 20000)),
    ("mode", "p39_warp3.json", {"edit": "set_stretch", "clip": 1, "mode": "stretch",
                                "frames": 30000},
     set_stretch(CASES["warp3"], 1, 30000, mode="stretch")),
    ("unstretch", "p39_warp3.json", {"edit": "set_stretch", "clip": 1, "mode": None,
                                     "frames": 0},
     set_stretch(CASES["warp3"], 1, None)),
    ("delete", "p39_warp3.json", {"edit": "delete", "clip": 1}, deleted(CASES["warp3"], 1)),
    ("duplicate", "p39_warp3.json", {"edit": "duplicate", "clip": 1, "first": 2},
     duplicated(CASES["warp3"], 1, 2)),
    # An edit of the sound that keeps its length goes through on a warped clip's channel.
    ("gain_op", "p39_warp3.json", {"edit": "sound_ops", "channel": 1, "ops": [GAIN_OP]},
     project("tone", [WARP3], {1: M3}, edits={"ops": [GAIN_OP]})),
]
REFUSED = [
    # (name, fixture, the edit): each changes nothing.
    ("unstretched", "p39_unstretched.json", set_warp_edit(1, M1)),
    ("no_clip", "p39_plain.json", set_warp_edit(9, M1)),
    ("not_rising", "p39_plain.json", set_warp_edit(1, [(12000, 24000), (11000, 30000)])),
    ("too_close_source", "p39_plain.json", set_warp_edit(1, [(12000, 24000), (12063, 24200)])),
    ("too_close_out", "p39_plain.json", set_warp_edit(1, [(12000, 24000), (12200, 24063)])),
    ("past_the_end", "p39_plain.json", set_warp_edit(1, [(12000, 35937)])),
    ("ratio", "p39_plain.json", set_warp_edit(1, [(1000, 4001)])),
    ("too_many", "p39_plain.json", set_warp_edit(1, TOO_MANY)),
    # Only the last segment breaks the ratio (the broken file "last_ratio" as an edit).
    ("last_ratio", "p39_plain.json", set_warp_edit(1, LAST_RATIO)),
    # Past the sound: a source 64 frames short of S is the least the last segment allows.
    ("past_the_sound", "p39_plain.json", set_warp_edit(1, [(23937, 30000)])),
    # The messy list scaled to 24000 frames: its 64-frame gaps fall to 62 or 63.
    ("scale_breaks", "p39_messy.json", {"edit": "set_stretch", "clip": 1, "mode": "resample",
                                        "frames": 24000}),
    # warp1 (S 24000, T 36000, marker (12000, 24000)) made 7250 frames long, as an Alt drag of
    # its end to 29 ticks asks: the uniform ratio 0.30 fits, the marker scales to (12000, 4833)
    # and the first segment is 0.40, but the last, 12000 frames made 2417, is under a quarter.
    ("last_segment_ratio", "p39_warp1.json", {"edit": "set_stretch", "clip": 1,
                                              "mode": "resample", "frames": 7250}),
    ("crop_op", "p39_warp3.json", {"edit": "sound_ops", "channel": 1, "ops": [CROP_OP]}),
]


def last_tick_at_or_before(f, rate=48000):
    """The last tick t with s(t) <= f."""
    t = f * MBPM * PPQ // (60 * rate * 1000)
    while s(t + 1, rate) <= f: t += 1
    while s(t, rate) > f: t -= 1
    return t


SIXTEENTH = PPQ // 4


def quantize(doc, clip, onsets, unit=SIXTEENTH):
    """The quantize rule above on clip `clip` of `doc`: (hits, markers kept, the list)."""
    c = next(x for x in doc["playlist"]["clips"] if x["id"] == clip)
    rate, S = doc["sample_rate"], sound_frames(doc)
    T = c["stretch"]["frames"] if c.get("stretch") else S
    off, start = c.get("frame_offset", 0), s(c["start"], rate)
    played = s(c["start"] + c["length"], rate) - start
    old = markers_of(doc, clip)
    hits = [x for x in onsets if off <= out_of_source(old, S, T, x) < off + played]
    built, taken, kept = [m for m in old if m[0] not in hits], set(), 0
    for x in hits:
        f = start + out_of_source(old, S, T, x) - off
        down = last_tick_at_or_before(f, rate) // unit * unit
        up = down + unit
        line = down if f - s(down, rate) < s(up, rate) - f else up
        if line in taken: continue
        trial = sorted(built + [(x, off + s(line, rate) - start)])
        if problems(trial, T, S): continue
        built, kept = trial, kept + 1
        taken.add(line)
    return hits, kept, built


QUANTIZE = project("qclicks", [audio(1, 10, 374)])
QUANTIZE_HITS, QUANTIZE_KEPT, QUANTIZE_MARKERS = quantize(QUANTIZE, 1, QCLICKS)


def onset_frames(sides):
    """The onsets rule, restated."""
    BLOCK, HISTORY, K, FLOOR, GAP = 128, 16, 2.5, 1.28e-4, 24
    frames = list(zip(*sides))
    blocks = (len(frames) + BLOCK - 1) // BLOCK
    energy = []
    for b in range(blocks):
        e = 0.0
        for frame in frames[b * BLOCK:(b + 1) * BLOCK]:
            for v in frame: e += v * v
        energy.append(e)
    magnitude = [max(abs(v) if math.isfinite(v) else 0.0 for v in frame) for frame in frames]
    found, last = [], None
    for b in range(blocks):
        if last is not None and b - last < GAP: continue
        if not math.isfinite(energy[b]): continue
        before = 0.0
        for j in range(b - HISTORY, b):
            if j >= 0 and math.isfinite(energy[j]): before += energy[j]
        if energy[b] > K * (before / HISTORY) + FLOOR:
            window = magnitude[b * BLOCK:(b + 2) * BLOCK]
            quarter = max(window) * 0.25
            found.append(b * BLOCK + next(i for i, m in enumerate(window) if m >= quarter))
            last = b
    return found


ONSET_SOUNDS = ["clicks", "steady", "close", "step6", "step2", "stereo", "nan", "in_phase",
                "inverted", "qclicks"]


def expected():
    return {
        "rules": {"min_segment": MIN_SEGMENT, "max_markers": MAX_MARKERS, "max_ratio": MAX_RATIO},
        "map": map_table(),
        "renders": {name: dict(render_case(doc), fixture="p39_%s.json" % name)
                    for name, doc in CASES.items()},
        "documents": {
            "warped": {"fixture": "p39_warp3_faded.json", "format_version": WARP_VERSION},
            "unwarped": {"fixture": "p39_plain.json", "format_version": 2},
            "refused_by_12": REFUSED_BY_12, "refused_14": REFUSED_14,
            "broken": {name: dump(doc) for name, doc in BROKEN.items()},
        },
        "edits": [dict(edit, name=name, fixture=fixture, after=dump(after),
                       undo=dump(json.loads(files_json()[fixture])))
                  for name, fixture, edit, after in EDITS],
        "refused": [dict(edit, name=name, fixture=fixture) for name, fixture, edit in REFUSED],
        "onsets": {name: {"file": path_of(name), "frames": onset_frames(SOUNDS[name][1])}
                   for name in ONSET_SOUNDS},
        "quantize": {"fixture": "p39_quantize.json", "clip": 1, "snap": "none",
                     "hits": QUANTIZE_HITS, "kept": QUANTIZE_KEPT,
                     "markers": [list(m) for m in QUANTIZE_MARKERS],
                     "after": dump(start_warp(QUANTIZE, 1, sound_frames(QUANTIZE),
                                              QUANTIZE_MARKERS))},
    }


def files_json():
    """Every project --write makes, by its name, as text without the final line break."""
    out = {"p39_%s.json" % name: dump(doc) for name, doc in CASES.items()}
    out["p39_unstretched.json"] = dump(UNSTRETCHED)
    out["p39_quantize.json"] = dump(QUANTIZE)
    return out


def files():
    """Every file --write makes, by its path under DIR, as bytes."""
    out = {path_of(name): wav_bytes(rate, sides) for name, (rate, sides) in SOUNDS.items()}
    for name, text in files_json().items():
        out[name] = (text + "\n").encode()
    out["p39_expected.json"] = (J(expected()) + "\n").encode()
    return out


def sanity():
    """What the rules state outright, held here so a rule restated wrong fails at once."""
    assert all(f32(v) == v or math.isnan(v)
               for _, sides in SOUNDS.values() for side in sides for v in side)
    assert s(BAR) == 96000 and s(240, 44100) == 55125 and s(384, 44100) == 88200
    tone = values_of("tone")
    # No markers is the uniform read, bit for bit.
    assert warp_resample(tone, [], 36000) == P30.resample(tone, 36000)
    # A marker at ratio 1 on both sides reads the sound itself.
    assert warp_resample(tone, [(12000, 12000)], 24000) == tone
    for markers, S, T in [(M1, 24000, 36000), (M3, 24000, 30000), (M1_44, 22050, 33075),
                          (M3_44, 22050, 33075), (MESSY, 16448, MESSY_T)]:
        assert problems(markers, T, S) == [], (markers[:3], S, T)
        assert all(out_of_source(markers, S, T, x) == o and source_of_out(markers, S, T, o) == x
                   for x, o in markers), "exact at every anchor"
        assert len(warp_resample(values_of("tone")[:S], markers, T)) == T
    assert len(MESSY) == MAX_MARKERS and MESSY[-1] == (16384, 24576) and MESSY_T == 24640
    assert out_of_source([(4, 6)], 8, 10, 1) == 2 and out_of_source([(4, 6)], 8, 10, 3) == 5
    assert source_of_out([(6, 4)], 10, 8, 1) == 2
    assert render(CASES["warp3"]) == render(CASES["warp3_split"])
    assert render(CASES["warp3_faded"]) == render(CASES["warp3_faded_split"])
    assert render(CASES["warp3"]) != render(CASES["seam_moved"])
    assert lengths(CASES["warp3_split"], CASES["warp3_split"]["playlist"]["clips"][1]) == (0, 0)
    assert lengths(CASES["seam_moved"], CASES["seam_moved"]["playlist"]["clips"][1]) == (96, 0)
    assert lengths(CASES["seam_unwarped"], CASES["seam_unwarped"]["playlist"]["clips"][0]) == (
        0, 96)
    assert render(CASES["warp1"]) != render(CASES["plain"])
    # Each broken file breaks what its name says (the ones a load alone can see).
    for name in ["not_rising", "too_close", "past_the_end", "at_the_start", "ratio", "too_many",
                 "last_ratio"]:
        pairs = markers_of(BROKEN[name], 1)
        assert problems(pairs, json.loads(J(BROKEN[name]))["playlist"]["clips"][0]["stretch"][
            "frames"], 24000), name
    # "last_ratio" breaks the last segment alone: the list up to its last marker keeps every
    # rule (the marker taken as the end).
    assert problems(LAST_RATIO, 36000, 24000) == ["ratio"]
    assert problems([], 20000, 21000) == []
    assert problems([(23937, 30000)], 36000, 24000) == ["too close"]
    # The refused T change breaks the least spacing; the accepted ones keep every rule.
    assert problems([(x, half_up(o * 24000, MESSY_T)) for x, o in MESSY], 24000, 16448) != []
    # warp1 made 7250 frames: the uniform ratio fits, the scaled marker's last segment does not.
    assert 24000 <= MAX_RATIO * 7250 and half_up(24000 * 7250, 36000) == 4833
    assert problems([(12000, 4833)], 7250, 24000) == ["ratio"]
    assert problems([], 4833, 12000) == []
    assert markers_of(set_stretch(CASES["warp3"], 1, 30005), 1) == [(4000, 6001),
                                                                    (10000, 12002),
                                                                    (20000, 21004)]
    assert problems(markers_of(set_stretch(CASES["warp3"], 1, 20000), 1), 20000, 24000) == []
    assert onset_frames(SOUNDS["clicks"][1]) == CLICKS
    assert onset_frames(SOUNDS["close"][1]) == [5000]
    assert onset_frames(SOUNDS["stereo"][1]) == [3000, 30000]
    steady = onset_frames(SOUNDS["steady"][1])
    assert len(steady) == 1 and steady[0] < 128
    step6, step2 = onset_frames(SOUNDS["step6"][1]), onset_frames(SOUNDS["step2"][1])
    assert step6[0] == steady[0] and len(step6) == 2 and STEP <= step6[1] < STEP + 128, step6
    assert step2 == steady
    # A NaN in silence is no hit; a click in opposite phase on the two sides is found where the
    # same click in phase is, which a mix to one side would have summed to nothing.
    assert onset_frames(SOUNDS["nan"][1]) == []
    assert onset_frames(SOUNDS["in_phase"][1]) == CLICKS == onset_frames(SOUNDS["inverted"][1])
    assert [(l + r) * 0.5 for l, r in zip(*SOUNDS["inverted"][1])] == [0.0] * 48000
    # Quantize, by hand: song frame = 2500 + source, the lines every 6000 song frames, a marker's
    # out = its line - 2500.
    assert onset_frames(SOUNDS["qclicks"][1]) == QCLICKS
    assert QUANTIZE_HITS == QCLICKS[:7] and QUANTIZE_KEPT == 6
    assert QUANTIZE_MARKERS == [(4200, 3500), (8600, 9500), (24499, 21500), (30500, 33500),
                                (45600, 45500), (66600, 69500)]
    assert last_tick_at_or_before(26999) == 107 and last_tick_at_or_before(27000) == 108


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
        if stale: sys.exit("not what gen_p39_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    answers = expected()
    for name, case in answers["renders"].items():
        print("%-18s lengths %s, %d frames sounding" % (name, case["lengths"], case["sounding"]))
    for name, case in answers["onsets"].items():
        print("onsets %-7s %s" % (name, case["frames"]))


if __name__ == "__main__":
    main()
