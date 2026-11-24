"""Known answers for slip and split on the playlist: an audio clip's audio moved inside it (its
start and length kept), and clips cut in two at a tick, every cut of one command one undo step.

Expected values come from the rules written out below (restated from the design, never from
engine code) and from each signal's construction. gen_p30_expected.py's resample read (itself
gen_p9's, restated) is imported. Every input is authored here: the WAVs and the projects.

The rules:
  s(t)       the one tick-to-sample rule, floor(t * 60 * rate * 1000 / (mbpm * ppq)); here 48 kHz
             at 174.5 BPM, where s(a) - s(b) is not always s(a - b): each case is chosen so
             the shortcut would be a frame off (sanity() holds it)
  split      a clip (start, length, item) is cut at tick t only when start < t < end. The left
             part keeps its start and is t - start long. The right part is a new clip on the same
             track from t, end - t long, muted as the clip was: an audio clip's frame_offset
             becomes off + s(t) - s(start) (in the stretched file's frames when stretched; the
             channel, gain and stretch kept); a pattern or automation clip's offset becomes
             offset + (t - start) in ticks, the same pattern (a linked clip stays linked). Why
             not reduced by the pattern's length: the right part then plays what the whole clip
             played after t even once the pattern is made longer or shorter
  ids        the right parts take next_clip_id, next_clip_id + 1, ... in the ascending order of
             the cut clips' ids (not their starts); next_clip_id moves past them
  one step   every cut of one command is one edit: the lefts resized, then the rights placed;
             its undo gives back the document byte for byte
  slip       a drag of d ticks (already snapped) moves the audio, the clip staying put:
             F = s(start + d) - s(start) for d >= 0, F = -(s(start + |d|) - s(start)) for d < 0
             (the mirror, so a slip and its opposite cancel), and the frame offset becomes
             clamp(off - F, 0, last): last the stretched file's frames - 1 on a stretched clip,
             else the sound's frames - 1 when known, else no upper stop. Dragging right plays
             earlier audio. An offset that stays is no edit
  playing    an audio clip plays sound frame off + n - s(start) at song sample n of the clip,
             times its gain, a mono sound on both sides; a resampled one the stretched file
             (gen_p9's read at j * S / T) the same way. An edge that cuts into the sound ramps
             over the 2 ms declick (96 frames here) unless a clip going on with the same sound
             meets it, as gen_p38_expected.py states and computes (its edge_lengths and factor):
             so a split's parts play the uncut clip's bits, the ramps on the outer edges alone. A sampler with no sound settings plays
             its whole sound from a note's start at key 60 and velocity 1, whatever the note's
             length or the clip's end: so a cut through a note leaves what plays as it was.
             Song tick T of a pattern clip plays the note on data tick (T - start + offset)
             mod L. Every value here is a multiple of 2^-18 below 1, so sums are exact in f32.
             Each project ends on its first bar line (song mode starts again at the song's end)

  p37_sound/*.wav        the sounds (32-bit float, 48 kHz, mono)
  p37_cut_<case>.json    a project before a split; p37_cut_<case>_after.json the same after it
                         (the CLI renders both)
  p37_slip.json          two audio clips, one resampled, for the slips
  p37_expected.json      "cuts" (the clips cut, the tick, the new ids, the document after and the
                         render both documents give) and "slip" (each drag, the frames it moves,
                         the offset and the document after)

Run:
  python3 -I gen_p37_expected.py              print the known answers
  python3 -I gen_p37_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p37_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import copy, hashlib, importlib.util, json, os, struct, sys

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
f32 = P30.f32
SR, MBPM, PPQ = 48000, 174500, 96
BAR = 4 * PPQ


def s(t): return t * 60 * SR * 1000 // (MBPM * PPQ)


BAR_FRAMES = s(BAR)


def J(d): return json.dumps(d, separators=(",", ":"), ensure_ascii=False)


# ---------- the sounds ----------
RAMP = [k / 262144 for k in range(60000)]                  # k / 2^18, exact in f32
BURST = [(1 + k % 32) / 256 for k in range(4000)]
SOUNDS = {"ramp": RAMP, "burst": BURST}


def path_of(sound): return "p37_sound/%s.wav" % sound


def wav_bytes(values):
    """A mono 32-bit float WAV at 48 kHz."""
    data = struct.pack("<%df" % len(values), *values)
    fmt = struct.pack("<HHIIHH", 3, 1, SR, SR * 4, 4, 32)
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack(
        "<I", len(data)) + data
    return b"RIFF" + struct.pack("<I", len(body)) + body


# ---------- projects, in the file's field order ----------
def tracks(): return [{"id": i, "name": "Track %d" % i, "mute": False} for i in range(1, 9)]


def channel(i, name, kind, sound):
    return {"id": i, "name": name, "kind": kind, "sample": path_of(sound), "volume": 1.0,
            "pan": 0.0, "mute": False}


def note(tick, length): return {"channel": 1, "tick": tick, "length": length, "velocity": 1.0}


def audio(i, track, start, length, ch, offset=0, gain=1.0, stretch=None, muted=False):
    c = {"id": i, "track": track, "start": start, "length": length, "kind": "audio",
         "channel": ch}
    if offset: c["frame_offset"] = offset
    if gain != 1.0: c["gain"] = gain
    if stretch: c["stretch"] = {"mode": "resample", "frames": stretch}
    if muted: c["muted"] = True
    return c


def pattern_clip(i, track, start, length, offset=0, muted=False):
    c = {"id": i, "track": track, "start": start, "length": length, "kind": "pattern",
         "pattern": 1}
    if offset: c["offset"] = offset
    if muted: c["muted"] = True
    return c


def project(channels, notes, clips):
    return {"format_version": 2, "sample_rate": SR, "tempo_mbpm": MBPM, "ppq": PPQ,
            "next_channel_id": len(channels) + 1, "channels": channels,
            "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": notes}],
            "playlist": {"next_track_id": 9, "next_clip_id": len(clips) + 1, "tracks": tracks(),
                         "clips": clips}}


def item_order(c):
    """A clip's fields in the file's order."""
    order = ["id", "track", "start", "length", "kind", "pattern", "channel", "frame_offset",
             "gain", "stretch", "offset", "muted"]
    return {k: c[k] for k in order if k in c}


# ---------- the split, by the rule ----------
def split(doc, ids, t):
    """`doc` with each clip of `ids` that `t` falls strictly inside cut there: the document and
    the new clips' ids."""
    doc = copy.deepcopy(doc)
    playlist = doc["playlist"]
    first = playlist["next_clip_id"]
    rights = []
    for c in sorted((c for c in playlist["clips"] if c["id"] in ids), key=lambda c: c["id"]):
        start, end = c["start"], c["start"] + c["length"]
        if not start < t < end: continue
        right = copy.deepcopy(c)
        right["id"] = first + len(rights)
        right["start"], right["length"] = t, end - t
        if c["kind"] == "audio":
            offset = c.get("frame_offset", 0) + s(t) - s(start)
            if offset: right["frame_offset"] = offset
        else:
            offset = c.get("offset", 0) + (t - start)
            if offset: right["offset"] = offset
        c["length"] = t - start
        rights.append(item_order(right))
    playlist["clips"] += rights
    playlist["next_clip_id"] = first + len(rights)
    return doc, [r["id"] for r in rights]


# ---------- the slip, by the rule ----------
def slip(doc, clip, d, sound_frames):
    """`doc` after a slip of `d` ticks on `clip`: the frames moved, the offset, the document
    (None when the offset stays)."""
    c = next(c for c in doc["playlist"]["clips"] if c["id"] == clip)
    start, off = c["start"], c.get("frame_offset", 0)
    moved = s(start + abs(d)) - s(start)
    frames = moved if d >= 0 else -moved
    stretch = c.get("stretch")
    last = stretch["frames"] - 1 if stretch else (
        sound_frames - 1 if sound_frames is not None else None)
    offset = max(0, off - frames)
    if last is not None: offset = min(offset, last)
    if offset == off: return frames, offset, None
    after = copy.deepcopy(doc)
    c = next(c for c in after["playlist"]["clips"] if c["id"] == clip)
    c.pop("frame_offset", None)
    if offset: c["frame_offset"] = offset
    after["playlist"]["clips"] = [item_order(c) for c in after["playlist"]["clips"]]
    return frames, offset, after


# ---------- what plays, by the rule ----------
def render(doc):
    """Left (= right) of one bar in song mode."""
    out = [0.0] * BAR_FRAMES
    notes = doc["patterns"][0]["notes"]
    for c in doc["playlist"]["clips"]:
        if c.get("muted"): continue
        start, end = c["start"], c["start"] + c["length"]
        if c["kind"] == "audio":
            ch = next(x for x in doc["channels"] if x["id"] == c["channel"])
            src = SOUNDS[ch["sample"][len("p37_sound/"):-len(".wav")]]
            if c.get("stretch"): src = P30.resample(src, c["stretch"]["frames"])
            off, gain = c.get("frame_offset", 0), c.get("gain", 1.0)
            n_in, n_out = P38.edge_lengths(doc["playlist"]["clips"], c, s, len(src), SR)
            for n in range(s(start), min(s(end), BAR_FRAMES)):
                k = off + n - s(start)
                if k < len(src):
                    ramp = P38.factor(n - s(start), s(end) - 1 - n, n_in, n_out)
                    out[n] += f32(src[k] * ramp) * gain
        else:
            offset = c.get("offset", 0)
            for p in range(-1, (end - start + offset) // BAR + 1):
                for x in notes:
                    at = start + p * BAR + x["tick"] - offset
                    if not start <= at < end: continue
                    for k, v in enumerate(BURST):
                        if s(at) + k < BAR_FRAMES: out[s(at) + k] += v
    return [f32(v) for v in out]


def render_case(doc):
    left = render(doc)
    flat = [x for v in left for x in (v, v)]
    nonzero = [i for i, v in enumerate(left) if v != 0.0]
    probes = sorted(set(nonzero[:3] + nonzero[-3:] + nonzero[::5000]))
    return {"frames": BAR_FRAMES, "sha256": P30.sha(flat), "sounding": len(nonzero),
            "probes": [[i, left[i], left[i]] for i in probes]}


# ---------- the cases ----------
RAMP_AUDIO = channel(1, "Ramp", "audio", "ramp")
BURST_SAMPLER = channel(1, "Burst", "sampler", "burst")
CUTS = {
    # (1) An audio clip at gain 0.5 from frame 1234, cut by the pointer or the marker at 216:
    # s(216) - s(40) = 30258, one more than s(176) = 30257.
    "audio": (project([RAMP_AUDIO], [], [audio(1, 1, 40, 344, 1, offset=1234, gain=0.5)]),
              [1], 216),
    # (2) A clip resampled to 90000 frames (1.5 times its 60000), from stretched frame 777, cut
    # at 173: s(173) - s(20) = 26304, one more than s(153).
    "stretch": (project([RAMP_AUDIO], [], [audio(1, 1, 20, 364, 1, offset=777, stretch=90000)]),
                [1], 173),
    # (3) A pattern clip from data tick 48 (its notes on 96, 192 ticks long, and on 288), cut at
    # 120: through the first note, which sounds from song tick 48 to 240.
    "pattern": (project([BURST_SAMPLER], [note(96, 192), note(288, 0)],
                        [pattern_clip(1, 1, 0, 384, offset=48)]), [1], 120),
    # (4) Four clips selected and the marker at 150: clip 2 (audio, 100..384) and clip 5
    # (pattern, 25..325) are cut, clip 3 (0..96) ends before it, clip 4 starts on it; clip 1
    # crosses it unselected. The rights are 6 (of clip 2) and 7 (of clip 5), though clip 5
    # starts first. s(150) - s(100) = 8596, one more than s(50).
    "two": (project([BURST_SAMPLER, channel(2, "Ramp", "audio", "ramp")],
                    [note(0, 0), note(192, 0)],
                    [pattern_clip(1, 1, 0, 384), audio(2, 2, 100, 284, 2, offset=5000),
                     pattern_clip(3, 3, 0, 96), audio(4, 4, 150, 100, 2),
                     pattern_clip(5, 5, 25, 300)]), [2, 3, 4, 5], 150),
}
SLIP = project([RAMP_AUDIO], [], [audio(1, 1, 100, 200, 1, offset=20000),
                                  audio(2, 2, 100, 200, 1, offset=500, stretch=90000)])
# (5) Slips: (clip, ticks, the sound's frames as the window knows them, or None).
SLIPS = [
    (1, 96, 60000),        # a beat right: 16505 frames earlier, 3495
    (1, -96, 60000),       # a beat left: the mirror, 36505
    (1, 192, 60000),       # two beats right: past frame 0, so 0
    (1, -2000, 60000),     # far left: stops on the sound's last frame, 59999
    (1, -2000, None),      # far left with the sound not loaded yet: no stop, 363840
    (2, -2000, None),      # the resampled clip: stops on the stretched file's last, 89999
    (1, 0, 60000),         # no drag: no edit
]


def dump(doc): return J(doc)


def cut_case(name):
    doc, ids, t = CUTS[name]
    after, new = split(doc, ids, t)
    return {"fixture": "p37_cut_%s.json" % name, "cut": "p37_cut_%s_after.json" % name,
            "ids": ids, "tick": t, "new": new, "after": dump(after), "render": render_case(doc)}


def slip_case():
    steps = []
    for clip, d, frames in SLIPS:
        moved, offset, after = slip(SLIP, clip, d, frames)
        steps.append({"clip": clip, "ticks": d, "sound_frames": frames, "frames": moved,
                      "frame_offset": offset, "after": dump(after) if after else None})
    return {"fixture": "p37_slip.json", "steps": steps}


def expected():
    return {"cuts": {name: cut_case(name) for name in CUTS}, "slip": slip_case()}


def files():
    """Every file --write makes, by its path under DIR, as bytes."""
    out = {path_of(name): wav_bytes(values) for name, values in SOUNDS.items()}
    for name, (doc, ids, t) in CUTS.items():
        out["p37_cut_%s.json" % name] = (dump(doc) + "\n").encode()
        out["p37_cut_%s_after.json" % name] = (dump(split(doc, ids, t)[0]) + "\n").encode()
    out["p37_slip.json"] = (dump(SLIP) + "\n").encode()
    out["p37_expected.json"] = (J(expected()) + "\n").encode()
    return out


def sanity():
    """What the rules state outright, held here so a rule restated wrong fails at once."""
    assert BAR_FRAMES == 66017
    for (a, b) in [(216, 40), (173, 20), (150, 100)]:
        assert s(a) - s(b) == s(a - b) + 1, (a, b)
    assert all(f32(v) == v for v in RAMP + BURST)
    cut = lambda name: split(*CUTS[name])
    doc, new = cut("audio")
    assert new == [2] and doc["playlist"]["clips"][1]["frame_offset"] == 1234 + 30258
    doc, _ = cut("stretch")
    assert doc["playlist"]["clips"][1]["frame_offset"] == 777 + 26304
    doc, _ = cut("pattern")
    assert doc["playlist"]["clips"][1]["offset"] == 168
    doc, new = cut("two")
    assert new == [6, 7] and [c["id"] for c in doc["playlist"]["clips"]] == [1, 2, 3, 4, 5, 6, 7]
    assert doc["playlist"]["clips"][5]["frame_offset"] == 5000 + 8596
    assert doc["playlist"]["next_clip_id"] == 8
    for name, (before, ids, t) in CUTS.items():
        assert render(before) == render(split(before, ids, t)[0]), name
        assert any(v != 0.0 for v in render(before)), name
    offsets = [slip(SLIP, clip, d, n)[1] for clip, d, n in SLIPS]
    assert offsets == [3495, 36505, 0, 59999, 363840, 89999, 20000], offsets
    assert slip(SLIP, 1, 0, 60000)[2] is None


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
        if stale: sys.exit("not what gen_p37_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    answers = expected()
    for name, case in answers["cuts"].items():
        print("cut %-8s at %d: clips %s -> new %s, %d frames sounding" % (
            name, case["tick"], case["ids"], case["new"], case["render"]["sounding"]))
    for step in answers["slip"]["steps"]:
        print("slip clip %d by %+d ticks: %+d frames, offset %d" % (
            step["clip"], step["ticks"], step["frames"], step["frame_offset"]))


if __name__ == "__main__":
    main()
