"""Known answers for an audio clip's fades and the declick at its edges, on the playlist.

Expected values come from the rules written out below (restated from the design, never from
engine code) and from each signal's construction. gen_p30_expected.py's resample read (itself
gen_p9's, restated) and its f32 and SHA-256 helpers are imported. Every input is authored here:
the WAVs and the projects.

The rules:
  s(t)       the one tick-to-sample rule, floor(t * 60 * rate * 1000 / (mbpm * ppq)); 120 BPM
             at 48 kHz here (s(t) = 250 t), and at 44.1 kHz for the declick's length
  frames     an audio clip's frames F = s(end) - s(start); it plays sound frame off + k at
             song sample s(start) + k, k = 0 .. F - 1 (silence past the sound's end), the
             stretched file's frames on a stretched clip (gen_p9's read at j * S / T)
  D          the declick's length, round(2 ms * rate) half up: (2 * rate + 500) // 1000, so
             96 frames at 48 kHz and 88 at 44.1 kHz
  cuts in    a clip's start cuts into its sound when off > 0; its end when off + F is below
             the sound's frames (the stretched file's on a stretched clip)
  continued  an edge is not cut when exactly one clip that plays (not muted, its track not muted)
             ends there and exactly one starts, on the clip's track, and the later goes on
             with the earlier's sound: the same channel, stretch and gain, its frame offset
             off + F of the earlier; neither fade at the meeting edge set; and no ramp from
             the far edges reaching it (the earlier's fade in, taken up to D where its start
             cuts in, below its F; the later's fade out, likewise at its end, below its F). A
             split away from both fades, its parts each longer than their far ramps, leaves
             exactly that, so the parts play the uncut clip's bits; a cut inside a fade, or a
             part shorter than its far ramp, ramps at the cut instead
  lengths    N_in = max(fade_in, D) at a start that cuts in, else fade_in; N_out likewise with
             fade_out at the end. Why max: the clip's own fade, when longer, already ramps
  envelope   with k = n - s(start) and m = s(end) - 1 - n, the factor is
             a = min(1, k / N_in) when N_in > 0, else 1, and b = min(1, m / N_out) when
             N_out > 0, else 1, each a double; their product a * b (a double), times the
             sound's value (a double), is rounded once to f32. So a fade in starts at 0 and
             reaches 1 at k = N_in; a fade out is 0 on the clip's last sample, as the sample
             editor's fade (k / N and (N - 1 - k) / N over N frames) gives on its range
  playing    the f32 value times the clip's gain (f32) is added into the bus (f32), a mono
             sound on both sides at the channel's volume 1 and pan 0; clips add in id order
  fades      fade_in and fade_out are frames at the project's rate, written only when not 0;
             a project with one writes format_version 12 (a build that reads up to 11 refuses
             it by that number), one without keeps its version
  set fades  the edit sets fade_in = min(asked in, F), fade_out = min(asked out, F - fade_in);
             its undo gives the document back byte for byte
  split      (gen_p37's rule) the left part keeps fade_in and gets fade_out 0, the right part
             gets fade_in 0 and keeps fade_out

  p38_sound/*.wav      the sounds (32-bit float, mono): "flat" 20000 frames of 0.5 at 48 kHz,
                       "ramp" 60000 frames of k / 2^18 at 48 kHz, "flat44" 20000 of 0.5 at
                       44.1 kHz
  p38_<case>.json      the projects the CLI renders (one bar, the song ending on its bar line)
  p38_expected.json    "renders" (each case's frames, SHA-256 of the interleaved floats, frames
                       sounding and probes), "documents" (the version a project with fades
                       writes, the refusals), "set_fades" (each edit asked, the document after,
                       the bytes its undo gives) and "split" (the document a split of a faded
                       clip leaves)

Run:
  python3 -I gen_p38_expected.py              print the known answers
  python3 -I gen_p38_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p38_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import copy, importlib.util, json, os, struct, sys

HERE = os.path.dirname(os.path.abspath(__file__))


def module(name):
    """A generator beside this one, whose functions restate rules this one shares."""
    sys.dont_write_bytecode = True                   # no __pycache__ beside the fixtures
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, name + ".py"))
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


P30 = module("gen_p30_expected")
f32 = P30.f32
MBPM, PPQ = 120000, 96
BAR = 4 * PPQ


def s(t, rate=48000, mbpm=MBPM): return t * 60 * rate * 1000 // (mbpm * PPQ)


def declick(rate): return (2 * rate + 500) // 1000


def J(d): return json.dumps(d, separators=(",", ":"), ensure_ascii=False)


# ---------- the sounds ----------
SOUNDS = {
    "flat": (48000, [0.5] * 20000),
    "ramp": (48000, [k / 262144 for k in range(60000)]),       # k / 2^18, exact in f32
    "flat44": (44100, [0.5] * 20000),
}


def path_of(sound): return "p38_sound/%s.wav" % sound


def wav_bytes(rate, values):
    """A mono 32-bit float WAV."""
    data = struct.pack("<%df" % len(values), *values)
    fmt = struct.pack("<HHIIHH", 3, 1, rate, rate * 4, 4, 32)
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack(
        "<I", len(data)) + data
    return b"RIFF" + struct.pack("<I", len(body)) + body


# ---------- projects, in the file's field order ----------
def tracks(): return [{"id": i, "name": "Track %d" % i, "mute": False} for i in range(1, 9)]


def channel(i, sound):
    return {"id": i, "name": sound.capitalize(), "kind": "audio", "sample": path_of(sound),
            "volume": 1.0, "pan": 0.0, "mute": False}


ORDER = ["id", "track", "start", "length", "kind", "channel", "frame_offset", "gain",
         "stretch", "fade_in", "fade_out", "muted"]


def audio(i, track, start, length, ch=1, offset=0, gain=1.0, stretch=None, fade_in=0,
          fade_out=0):
    c = {"id": i, "track": track, "start": start, "length": length, "kind": "audio",
         "channel": ch}
    if offset: c["frame_offset"] = offset
    if gain != 1.0: c["gain"] = gain
    if stretch: c["stretch"] = {"mode": "resample", "frames": stretch}
    if fade_in: c["fade_in"] = fade_in
    if fade_out: c["fade_out"] = fade_out
    return c


def ordered(c): return {k: c[k] for k in ORDER if k in c}


def version(clips):
    return 12 if any(c.get("fade_in") or c.get("fade_out") for c in clips) else 2


def project(sounds, clips, rate=48000, mbpm=MBPM):
    return {"format_version": version(clips), "sample_rate": rate, "tempo_mbpm": mbpm,
            "ppq": PPQ, "next_channel_id": len(sounds) + 1,
            "channels": [channel(i + 1, name) for i, name in enumerate(sounds)],
            "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": []}],
            "playlist": {"next_track_id": 9, "next_clip_id": len(clips) + 1, "tracks": tracks(),
                         "clips": [ordered(c) for c in clips]}}


# ---------- what plays, by the rules ----------
def source(doc, c):
    ch = next(x for x in doc["channels"] if x["id"] == c["channel"])
    _, values = SOUNDS[ch["sample"][len("p38_sound/"):-len(".wav")]]
    if c.get("stretch"): values = P30.resample(values, c["stretch"]["frames"])
    return values


def continues(a, b, s):
    """Whether audio clip `b` goes on from audio clip `a`'s end with the same sound, nothing
    faded where they meet; `s` the project's tick-to-sample rule."""
    off = a.get("frame_offset", 0) + s(a["start"] + a["length"]) - s(a["start"])
    return (a["track"] == b["track"] and a["channel"] == b["channel"]
            and a.get("stretch") == b.get("stretch") and a.get("gain", 1.0) == b.get("gain", 1.0)
            and b["start"] == a["start"] + a["length"] and b.get("frame_offset", 0) == off
            and not a.get("fade_out") and not b.get("fade_in"))


def far_ramps_clear(a, b, s, sound_frames, d):
    """Whether no ramp from the far edges of `a` (its start) and `b` (its end) reaches the seam
    where they meet: a's N_in below its frames, b's N_out below its, each N the clip's fade
    taken up to D where that far edge cuts into the sound (whatever meets it there)."""
    frames = lambda x: s(x["start"] + x["length"]) - s(x["start"])
    n_in = a.get("fade_in", 0)
    if a.get("frame_offset", 0) > 0: n_in = max(n_in, d)
    n_out = b.get("fade_out", 0)
    if b.get("frame_offset", 0) + frames(b) < sound_frames: n_out = max(n_out, d)
    return n_in < frames(a) and n_out < frames(b)


def edge_lengths(clips, c, s, sound_frames, rate, muted_tracks=()):
    """(N_in, N_out) of audio clip `c` among the playlist's `clips`, the declick taken in: `s`
    the project's tick-to-sample rule, `sound_frames` the frames of the sound `c` plays (the
    stretched file's when stretched), `rate` the project's, `muted_tracks` the tracks that play
    nothing. A seam is no cut only when exactly one playing audio clip ends there and exactly
    one starts, the one goes on with the other's sound (continues) and no far ramp reaches it.
    Generators whose renders hold trimmed clips import it."""
    playing = [x for x in clips if x.get("kind") == "audio" and not x.get("muted")
               and x["track"] not in muted_tracks]
    end = lambda x: x["start"] + x["length"]
    d = declick(rate)

    def seam(a, b):
        ending = [x for x in playing if x["track"] == a["track"] and end(x) == b["start"]]
        starting = [x for x in playing if x["track"] == a["track"] and x["start"] == b["start"]]
        return (len(ending) == 1 and len(starting) == 1 and continues(a, b, s)
                and far_ramps_clear(a, b, s, sound_frames, d))

    before = [x for x in playing if x["id"] != c["id"] and x["track"] == c["track"]
              and end(x) == c["start"]]
    after = [x for x in playing if x["id"] != c["id"] and x["track"] == c["track"]
             and x["start"] == end(c)]
    off = c.get("frame_offset", 0)
    frames = s(end(c)) - s(c["start"])
    start_cut = off > 0 and not (len(before) == 1 and seam(before[0], c))
    end_cut = off + frames < sound_frames and not (len(after) == 1 and seam(c, after[0]))
    fade_in, fade_out = c.get("fade_in", 0), c.get("fade_out", 0)
    return (max(fade_in, d) if start_cut else fade_in, max(fade_out, d) if end_cut else fade_out)


def lengths(doc, c):
    """(N_in, N_out) of clip `c` of `doc`."""
    rate, mbpm = doc["sample_rate"], doc["tempo_mbpm"]
    muted = [t["id"] for t in doc["playlist"]["tracks"] if t["mute"]]
    return edge_lengths(doc["playlist"]["clips"], c, lambda t: s(t, rate, mbpm),
                        len(source(doc, c)), rate, muted)


def factor(k, m, n_in, n_out):
    a = min(1.0, k / n_in) if n_in > 0 else 1.0
    b = min(1.0, m / n_out) if n_out > 0 else 1.0
    return a * b


def render(doc):
    """Left (= right) of one bar in song mode."""
    rate, mbpm = doc["sample_rate"], doc["tempo_mbpm"]
    total = s(BAR, rate, mbpm)
    out = [0.0] * total
    muted = [t["id"] for t in doc["playlist"]["tracks"] if t["mute"]]
    for c in sorted(doc["playlist"]["clips"], key=lambda c: c["id"]):
        if c.get("muted") or c["track"] in muted: continue
        start, end = s(c["start"], rate, mbpm), s(c["start"] + c["length"], rate, mbpm)
        src = source(doc, c)
        off, gain = c.get("frame_offset", 0), f32(c.get("gain", 1.0))
        n_in, n_out = lengths(doc, c)
        for n in range(start, min(end, total)):
            k = off + n - start
            value = src[k] if k < len(src) else 0.0
            value = f32(value * factor(n - start, end - 1 - n, n_in, n_out))
            out[n] = f32(out[n] + f32(value * gain))
    return out


def render_case(doc):
    left = render(doc)
    flat = [x for v in left for x in (v, v)]
    nonzero = [i for i, v in enumerate(left) if v != 0.0]
    probes = sorted(set(nonzero[:3] + nonzero[-3:] + nonzero[::4000] + EDGES.get(id(doc), [])))
    return {"frames": len(left), "sha256": P30.sha(flat), "sounding": len(nonzero),
            "lengths": [[c["id"]] + list(lengths(doc, c)) for c in doc["playlist"]["clips"]],
            "probes": [[i, left[i], left[i]] for i in probes]}


# ---------- the cases ----------
# Every case's last clip ends on the bar line (tick 384), so one bar is the whole song. flat
# holds 20000 frames (80 ticks at 48 kHz), ramp 60000 (240 ticks).
def split_at(c, t, rate=48000, mbpm=MBPM):
    """gen_p37's split of clip `c` at tick `t`, with the fades kept on the outer edges as they
    are stored (never shortened to a part: the render's min(1, k / N) holds them)."""
    left, right = copy.deepcopy(c), copy.deepcopy(c)
    left["length"] = t - c["start"]
    left.pop("fade_out", None)
    right["id"] = c["id"] + 1
    right["start"], right["length"] = t, c["start"] + c["length"] - t
    right["frame_offset"] = (c.get("frame_offset", 0) + s(t, rate, mbpm)
                             - s(c["start"], rate, mbpm))
    right.pop("fade_in", None)
    return [left, right]


SEAM = audio(1, 1, 200, 184, offset=1234)           # cuts in at both edges: 47234 < 60000
# The review's R1: flat from frame 1000, faded in over 3000 (12 ticks), to the bar line and past
# the sound's end; split 1000 frames in, inside the fade.
R1 = audio(1, 1, 300, 84, offset=1000, fade_in=3000)
# The review's R2: 400 BPM (75 frames a tick), flat from frame 1000, no fades; split a tick in,
# so the left part (75 frames) is shorter than its start's declick (96).
R2_MBPM = 400000
R2 = audio(1, 1, 300, 84, offset=1000)
CASES = {
    # A fade in of 1000 frames on the whole flat sound: no edge cuts in (offset 0, the clip
    # ends on the sound's last frame).
    "fade_in": project(["flat"], [audio(1, 1, 304, 80, fade_in=1000)]),
    # A fade out of 1500 frames on the same.
    "fade_out": project(["flat"], [audio(1, 1, 304, 80, fade_out=1500)]),
    # Both, at gain 0.5: 1000 in, 3000 out.
    "both": project(["flat"], [audio(1, 1, 304, 80, gain=0.5, fade_in=1000, fade_out=3000)]),
    # The fades the edit leaves when asked for more than the clip: 15000 in, 5000 out.
    "clamped": project(["flat"], [audio(1, 1, 304, 80, fade_in=15000, fade_out=5000)]),
    # A start 5000 frames into the sound, to its end: the start declicks over D = 96.
    "declick_start": project(["flat"], [audio(1, 1, 324, 60, offset=5000)]),
    # The same with a fade in of 40 (shorter than D: D ramps) and, on track 2 from 5000 frames
    # later in the bar, one of 200 (longer: its own).
    "short_fade": project(["flat"], [audio(1, 1, 124, 60, offset=5000, fade_in=40),
                                     audio(2, 2, 324, 60, offset=5000, fade_in=200)]),
    # An end 10000 frames into the 20000: the end declicks.
    "declick_end": project(["flat"], [audio(1, 1, 344, 40)]),
    # Offset 0 and past the sound's end: no edge cuts in, so every frame is 0.5 where it plays.
    "untouched": project(["flat"], [audio(1, 1, 284, 100)]),
    # A clip cutting into the ramp at both ends, whole ...
    "seam_whole": project(["ramp"], [SEAM]),
    # ... and split at 290: the parts meet with no declick, the same bits.
    "seam_split": project(["ramp"], split_at(SEAM, 290)),
    # The right part a tick later: a gap, so both inner edges declick.
    "seam_gap": project(["ramp"], [split_at(SEAM, 290)[0],
                                   dict(split_at(SEAM, 290)[1], start=291, length=93)]),
    # The ramp resampled to 90000 frames, the clip all of it, faded 2000 in and 2000 out.
    "stretch": project(["ramp"], [audio(1, 1, 24, 360, stretch=90000, fade_in=2000,
                                        fade_out=2000)]),
    # 44.1 kHz: the start 1625 frames in declicks over D = 88; the clip ends on the sound's end
    # (s(384) - s(304) = 88200 - 69825 = 18375 = 20000 - 1625).
    "rate44": project(["flat44"], [audio(1, 1, 304, 80, offset=1625)], rate=44100),
    # Two clips overlapping 6000 frames on one track, faded across it as Crossfade sets them: the
    # earlier from frame 0 (its end cuts in, its own fade longer), the later from 30000 (its
    # end cuts in at 56000 of 60000: D).
    "crossfade": project(["ramp"], [audio(1, 1, 200, 104, fade_out=6000),
                                    audio(2, 1, 280, 104, offset=30000, fade_in=6000)]),
    # R1 whole, then split inside its fade in: the left part's fade (3000) is longer than the
    # part (1000), so the seam is cut, its two edges declicked: no step, not the whole's bits.
    "r1_whole": project(["flat"], [R1]),
    "r1_split": project(["flat"], split_at(R1, 304)),
    # R2 whole, then split a tick in: the left part's start declick (96) is longer than the
    # part (75), so the seam is cut likewise.
    "r2_whole": project(["flat"], [R2], mbpm=R2_MBPM),
    "r2_split": project(["flat"], split_at(R2, 301, mbpm=R2_MBPM), mbpm=R2_MBPM),
    # Clip 1 ends at 364 where clips 2 and 3, the same, start going on with its sound: not one
    # clip each side, so all three meeting edges declick (clip 1's own start too: offset 1000).
    "stacked": project(["flat"], [audio(1, 1, 344, 20, offset=1000),
                                  audio(2, 1, 364, 20, offset=6000),
                                  audio(3, 1, 364, 20, offset=6000)]),
    # seam_split with its right part muted: nothing goes on at the seam, so the left part's end
    # declicks.
    "seam_muted": project(["ramp"], [split_at(SEAM, 290)[0],
                                     dict(split_at(SEAM, 290)[1], muted=True)]),
}
# Probes on the edges each case is about (by object, as the cases are made once).
EDGES = {
    id(CASES["fade_in"]): [76000, 76001, 76999, 77000],
    id(CASES["fade_out"]): [94499, 94500, 95998, 95999],
    id(CASES["declick_start"]): [81000, 81001, 81095, 81096],
    id(CASES["short_fade"]): [31000, 31039, 31040, 31095, 31096, 81000, 81095, 81199, 81200],
    id(CASES["declick_end"]): [86000, 95903, 95904, 95999],
    id(CASES["seam_whole"]): [50000, 50095, 50096, 72500, 72501, 95904, 95999],
    id(CASES["seam_split"]): [50000, 50095, 50096, 72499, 72500, 72501, 95904, 95999],
    id(CASES["seam_gap"]): [72403, 72499, 72750, 72845, 72846],
    id(CASES["stretch"]): [6000, 7999, 8000, 93999, 94000, 95999],
    id(CASES["rate44"]): [69825, 69826, 69912, 69913],
    id(CASES["crossfade"]): [70000, 70001, 75999, 76000, 82000, 95904, 95999],
    id(CASES["r1_whole"]): [75000, 75999, 76000, 76500, 78000],
    id(CASES["r1_split"]): [75000, 75903, 75999, 76000, 76048, 76500, 78000],
    id(CASES["r2_whole"]): [22500, 22574, 22575, 22590, 22595, 22596],
    id(CASES["r2_split"]): [22500, 22574, 22575, 22590, 22595, 22671],
    id(CASES["stacked"]): [86000, 90903, 90999, 91000, 91048, 91096],
    id(CASES["seam_muted"]): [72403, 72404, 72499, 72500],
}


# ---------- the documents ----------
def dump(doc): return J(doc)


REFUSED_BY_11 = ("format_version 12 is not supported (this build reads versions 1, 2, 4, 5, 6, "
                 "7, 8, 9, 10 and 11)")
REFUSED_13 = ("format_version 13 is not supported (this build reads versions 1, 2, 4, 5, 6, 7, "
              "8, 9, 10, 11 and 12)")


def set_fades(doc, clip, fade_in, fade_out):
    """`doc` after the edit asks clip `clip` for these fades, by the clamp rule."""
    after = copy.deepcopy(doc)
    clips = after["playlist"]["clips"]
    c = next(c for c in clips if c["id"] == clip)
    rate = after["sample_rate"]
    frames = s(c["start"] + c["length"], rate) - s(c["start"], rate)
    got_in = min(fade_in, frames)
    got_out = min(fade_out, frames - got_in)
    c.pop("fade_in", None)
    c.pop("fade_out", None)
    if got_in: c["fade_in"] = got_in
    if got_out: c["fade_out"] = got_out
    after["playlist"]["clips"] = [ordered(x) for x in clips]
    after["format_version"] = version(after["playlist"]["clips"])
    return after


PLAIN = project(["flat"], [audio(1, 1, 304, 80)])
SET_FADES = [
    # (the document, clip, asked in, asked out)
    ("p38_plain.json", PLAIN, 1, 1000, 0),
    ("p38_plain.json", PLAIN, 1, 15000, 9000),         # out clamped to 20000 - 15000
    ("p38_plain.json", PLAIN, 1, 25000, 7),            # in clamped to the clip, out to 0
    ("p38_fade_in.json", CASES["fade_in"], 1, 0, 0),   # both off: back to version 2
]
SPLIT_DOC = project(["ramp"], [audio(1, 1, 200, 184, offset=1234, fade_in=3000,
                                     fade_out=4000)])
SPLIT_TICK = 290


def split_doc(doc=SPLIT_DOC, tick=SPLIT_TICK):
    after = copy.deepcopy(doc)
    after["playlist"]["clips"] = [ordered(c) for c in split_at(doc["playlist"]["clips"][0], tick)]
    after["playlist"]["next_clip_id"] = 3
    return after


# Splits inside a fade in, the stored fades kept: p38_split at 205 (1250 frames in, inside its
# fade in of 3000, with a fade out of 4000) and p38_fade_in at 305 (250 frames in, inside its
# fade in of 1000, no fade out). The left part keeps its fade in whole, longer than itself.
SPLIT_INSIDE = [("p38_split.json", SPLIT_DOC, 205), ("p38_fade_in.json", CASES["fade_in"], 305)]


def expected():
    return {
        "declick": {"48000": declick(48000), "44100": declick(44100)},
        "renders": {name: dict(render_case(doc), fixture="p38_%s.json" % name)
                    for name, doc in CASES.items()},
        "documents": {
            "faded": {"fixture": "p38_both.json", "format_version": 12},
            "declick_only": {"fixture": "p38_declick_start.json", "format_version": 2},
            "refused_by_11": REFUSED_BY_11, "refused_13": REFUSED_13,
        },
        "set_fades": [{"fixture": name, "clip": clip, "fade_in": i, "fade_out": o,
                       "after": dump(set_fades(doc, clip, i, o)), "undo": dump(doc)}
                      for name, doc, clip, i, o in SET_FADES],
        "split": {"fixture": "p38_split.json", "tick": SPLIT_TICK, "clip": 1,
                  "after": dump(split_doc())},
        "split_inside": [{"fixture": name, "tick": tick, "clip": 1,
                          "after": dump(split_doc(doc, tick))}
                         for name, doc, tick in SPLIT_INSIDE],
    }


def files():
    """Every file --write makes, by its path under DIR, as bytes."""
    out = {path_of(name): wav_bytes(rate, values) for name, (rate, values) in SOUNDS.items()}
    for name, doc in CASES.items():
        out["p38_%s.json" % name] = (dump(doc) + "\n").encode()
    out["p38_plain.json"] = (dump(PLAIN) + "\n").encode()
    out["p38_split.json"] = (dump(SPLIT_DOC) + "\n").encode()
    out["p38_expected.json"] = (J(expected()) + "\n").encode()
    return out


def sanity():
    """What the rules state outright, held here so a rule restated wrong fails at once."""
    assert (declick(48000), declick(44100)) == (96, 88)
    assert s(BAR) == 96000 and s(BAR, 44100) == 88200 and s(304, 44100) == 69825
    assert all(f32(v) == v for _, values in SOUNDS.values() for v in values)
    lengths_of = lambda name: [list(lengths(CASES[name], c)) for c in
                               CASES[name]["playlist"]["clips"]]
    assert lengths_of("fade_in") == [[1000, 0]] and lengths_of("untouched") == [[0, 0]]
    assert lengths_of("declick_start") == [[96, 0]] and lengths_of("declick_end") == [[0, 96]]
    assert lengths_of("short_fade") == [[96, 0], [200, 0]]
    assert lengths_of("seam_whole") == [[96, 96]]
    assert lengths_of("seam_split") == [[96, 0], [0, 96]]
    assert lengths_of("seam_gap") == [[96, 96], [96, 96]]
    assert lengths_of("rate44") == [[88, 0]]
    assert lengths_of("crossfade") == [[0, 6000], [6000, 96]]
    assert render(CASES["seam_whole"]) == render(CASES["seam_split"])
    assert lengths_of("r1_whole") == [[3000, 0]] and lengths_of("r1_split") == [[3000, 96], [96, 0]]
    assert lengths_of("r2_whole") == [[96, 96]]
    assert lengths_of("r2_split") == [[96, 96], [96, 96]]
    assert lengths_of("stacked") == [[96, 96], [96, 96], [96, 96]]
    assert lengths_of("seam_muted")[0] == [96, 96]
    r1, r2 = render(CASES["r1_split"]), render(CASES["r2_split"])
    assert r1 != render(CASES["r1_whole"]) and r2 != render(CASES["r2_whole"])
    assert r1[75999] == 0.0 and r1[76000] == 0.0, "both edges of the cut ramp to 0"
    assert r2[22574] == 0.0 and r2[22575] == 0.0
    stacked = render(CASES["stacked"])
    assert stacked[90999] == 0.0 and stacked[91000] == 0.0 and stacked[91096] == 1.0
    inside = [json.loads(x["after"])["playlist"]["clips"] for x in expected()["split_inside"]]
    assert [(l.get("fade_in"), l.get("fade_out"), r.get("fade_in"), r.get("fade_out"))
            for l, r in inside] == [(3000, None, None, 4000), (1000, None, None, None)]
    assert [r["frame_offset"] for _, r in inside] == [1234 + 1250, 250]
    assert render(CASES["seam_whole"]) != render(CASES["seam_gap"])
    flat = render(CASES["untouched"])
    assert set(flat[71000:91000]) == {0.5} and not any(flat[91000:]) and not any(flat[:71000])
    fade = render(CASES["fade_in"])
    assert fade[76000] == 0.0 and fade[76500] == 0.25 and fade[77000] == 0.5
    out = render(CASES["fade_out"])
    assert out[95999] == 0.0 and out[94499] == 0.5 and out[95249] == 0.25
    start = render(CASES["declick_start"])
    assert start[81000] == 0.0 and start[81048] == 0.25 and start[81096] == 0.5
    assert [json.loads(x["after"])["playlist"]["clips"][0].get("fade_out", 0)
            for x in expected()["set_fades"]] == [0, 5000, 0, 0]
    assert [json.loads(x["after"])["format_version"] for x in expected()["set_fades"]] == [
        12, 12, 12, 2]
    left, right = json.loads(expected()["split"]["after"])["playlist"]["clips"]
    assert (left.get("fade_in"), left.get("fade_out"), right.get("fade_in"),
            right.get("fade_out"), right["frame_offset"]) == (3000, None, None, 4000, 23734)


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
        if stale: sys.exit("not what gen_p38_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    answers = expected()
    for name, case in answers["renders"].items():
        print("%-14s lengths %s, %d frames sounding" % (name, case["lengths"], case["sounding"]))
    for step in answers["set_fades"]:
        after = json.loads(step["after"])["playlist"]["clips"][0]
        print("set fades %d/%d on %s: %d/%d" % (step["fade_in"], step["fade_out"],
                                                step["fixture"], after.get("fade_in", 0),
                                                after.get("fade_out", 0)))


if __name__ == "__main__":
    main()
