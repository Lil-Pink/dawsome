"""Known answers for the electronic instrument sets the program ships (assets/instruments/): each
is one region over every key, its WAV rooted at its note, playing until note-off and releasing
over 0.3 s (bass, lead, pluck) or 1 s (pads). The renders below say, sample for sample, that
a note stops with its release after note-off, and that keys away from the root play the sound
pitched. The 808 basses are a region per root, each key played by its nearest, each sustaining
on a loop at its file's end (loop_continuous) while the key is held: their render also says that
a key plays its own root's region and that a held note reads on through its loop's wraps.

Expected values come from the rules written out in gen_p11_expected.py (whose functions this
imports: the reader's rules restated, never engine code; its loop rule is the one a loop plays
by here) and from the sets' own files: each set's .sfz is read here for its regions' keys,
roots, loops and release, each WAV decoded here (24-bit integers over 2^23). The projects run at
48 kHz, the sets' rate, so nothing is resampled.

  p12_pluck.json   Pluck C4 (mono, root 60, release 0.3 s): key 60 for a beat, key 67 at half
                   velocity for a sixteenth, key 48 for a sixteenth
  p12_bass.json    Bass 808 C1 (mono, roots C0 to C3), five bars: key 24 (its own root) held
                   for two bars, on through its loop's wraps; key 38 (root 36, two semitones
                   up) for a sixteenth; key 31 (root 29, two up) held for seven beats, through
                   its loop's wraps at that speed
  p12_pad.json     Pad warm C3 (stereo, root 48, release 1 s), two bars: key 48 for a beat,
                   key 60 at velocity 0.7 (byte 90) for a beat
  p12_expected.json  each render's SHA-256 (the stereo frames as little-endian f32 pairs), its
                   bars and frames, and for each note the root it plays, where its release ends
                   and frames to probe (around its loop's first wrap too, when it wraps)

Each project names its set by its reference, `dawsome:instruments/<set>/<set>.sfz`, and the test
renders it with the sets' folder as the installed sets.

Run:
  python3 -I gen_p12_expected.py              print the known answers
  python3 -I gen_p12_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p12_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import hashlib, importlib.util, json, math, os, re, struct, sys

HERE = os.path.dirname(os.path.abspath(__file__))
SETS = os.path.join(HERE, "..", "..", "assets", "instruments")


def rules():
    """gen_p11_expected.py, whose functions restate the reader's and the player's rules."""
    sys.dont_write_bytecode = True                   # no __pycache__ beside the fixtures
    spec = importlib.util.spec_from_file_location("gen_p11_expected",
                                                  os.path.join(HERE, "gen_p11_expected.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


P11 = rules()
f32, jf, exp2 = P11.f32, P11.jf, P11.exp2
SR = 48000; PPQ = 96; MBPM = 120000; STEP = PPQ // 4; BAR = 4 * PPQ


def t2s(t): return t * 60 * SR * 1000 // (MBPM * PPQ)            # the one tick-to-sample rule


# ---------- the sets as shipped ----------
def wav_channels(path):
    """The frames of a 24-bit integer PCM WAV, a list per channel, each sample over 2^23."""
    with open(path, "rb") as f:
        data = f.read()
    pos, fmt, samples = 12, None, None
    while pos + 8 <= len(data):
        chunk, size = data[pos:pos + 4], int.from_bytes(data[pos + 4:pos + 8], "little")
        if chunk == b"fmt ": fmt = data[pos + 8:pos + 24]
        elif chunk == b"data": samples = data[pos + 8:pos + 8 + size]
        pos += 8 + size + (size & 1)
    tag, channels, rate, _, align, bits = struct.unpack("<HHIIHH", fmt)
    assert (tag, bits, rate) == (1, 24, SR), path
    values = [int.from_bytes(samples[i:i + 3], "little", signed=True) / 8388608.0
              for i in range(0, len(samples), 3)]
    return [values[c::channels] for c in range(channels)]


def regions(sfz):
    """Each region's opcodes of a set's .sfz, as written, in file order."""
    with open(sfz, encoding="utf-8") as f:
        text = "".join(line for line in f if not line.startswith("//"))
    return [dict(re.findall(r"(\w+)=(.*?)(?=\s+\w+=|\s*$)", region.strip()))
            for region in text.split("<region>")[1:]]


def zones(name):
    """The set `name`'s zones, as gen_p11_expected.py's player reads zones: one per region, in
    file order, their key ranges covering every key once, upwards; each plays until note-off
    (no_loop) or loops while it sounds (loop_continuous, with its loop's frames)."""
    out = []
    for ops in regions(os.path.join(SETS, name, name + ".sfz")):
        frames = wav_channels(os.path.join(SETS, name, ops["sample"]))
        mode = ops["loop_mode"]
        assert mode in ("no_loop", "loop_continuous"), name
        assert ("loop_start" in ops and "loop_end" in ops) == (mode == "loop_continuous"), name
        loop = [int(ops["loop_start"]), int(ops["loop_end"])] if mode == "loop_continuous" else None
        out.append({"frames": frames, "first": 0, "last": len(frames[0]) - 1,
                    "keys": (int(ops["lokey"]), int(ops["hikey"])), "vels": (1, 127),
                    "center": int(ops["pitch_keycenter"]), "transpose": 0, "tune": 1.0,
                    "gain": 1.0, "veltrack": 100.0, "pan": 0.0, "mode": mode, "loop": loop,
                    "attack": 0, "release": int(float(ops["ampeg_release"]) * SR + 0.5),
                    "seq": None})
    lows = [z["keys"][0] for z in out]
    assert lows == [0] + [z["keys"][1] + 1 for z in out[:-1]] and out[-1]["keys"][1] == 127, name
    return out


def zone_of(zs, key):
    """The zone that plays `key`: the one whose range holds it."""
    return next(z for z in zs if z["keys"][0] <= key <= z["keys"][1])


def first_wrap(z, speed):
    """The first frame at which a voice reading `z` at `speed` reads past its loop's end, as
    gen_p11_expected.py's position() reads (first + t * speed); None for a zone with no loop."""
    if z["loop"] is None:
        return None
    t = max(0, math.ceil((z["loop"][1] + 1 - z["first"]) / speed) - 2)
    while z["first"] + t * speed < z["loop"][1] + 1:
        t += 1
    return t


# ---------- the cases ----------
# Pattern notes per case: (tick, length, key, velocity); none overlaps another, releases
# included, so each frame holds one voice.
CASES = {
    "pluck": ("pluck-c4", "Pluck C4", 1,
              [(0, PPQ, 60, 1.0), (2 * PPQ, STEP, 67, 0.5), (3 * PPQ, STEP, 48, 1.0)]),
    "bass": ("bass-808-c1", "Bass 808 C1", 5,
             [(0, 2 * BAR, 24, 1.0), (9 * PPQ, STEP, 38, 1.0), (10 * PPQ, 7 * PPQ, 31, 1.0)]),
    "pad": ("pad-warm-c3", "Pad warm C3", 2, [(0, PPQ, 48, 1.0), (BAR, PPQ, 60, 0.7)]),
}


def notes_of(case):
    """(start sample, off frames, key, velocity) of each note of `case`."""
    return [(t2s(tick), t2s(tick + length) - t2s(tick), key, f32(v))
            for tick, length, key, v in CASES[case][3]]


def render(case):
    _, _, bars, _ = CASES[case]
    return P11.render(zones(CASES[case][0]), notes_of(case), t2s(bars * BAR))


def sha256(left, right):
    data = b"".join(struct.pack("<ff", l, r) for l, r in zip(left, right))
    return hashlib.sha256(data).hexdigest()


def answer(case):
    left, right = render(case)
    zs = zones(CASES[case][0])
    releases = {z["release"] for z in zs}
    assert len(releases) == 1, case                          # every region releases alike
    notes = []
    for start, off, key, v in notes_of(case):
        z = zone_of(zs, key)
        speed = P11.semitone_speed(key - z["center"])
        end = start + off + z["release"]                   # the first frame after the release
        probes = {start, start + 1, start + off - 1, start + off,
                  start + off + z["release"] // 2, end - 1, end}
        wrap = first_wrap(z, speed)
        wraps = wrap is not None and start + wrap + 1 < end
        if wraps:
            probes |= {start + wrap - 1, start + wrap, start + wrap + 1}
        note = {"start": start, "off": start + off, "end": end, "key": key, "root": z["center"],
                "byte": P11.velocity_byte(v), "speed": speed}
        if wraps:
            note["wrap"] = start + wrap
        note["probes"] = [[i, left[i], right[i]] for i in sorted(probes)]
        notes.append(note)
    _, _, bars, _ = CASES[case]
    return {"fixture": "p12_%s.json" % case, "set": CASES[case][0], "bars": bars,
            "frames": len(left), "release": releases.pop(), "sha256": sha256(left, right),
            "notes": notes}


# ---------- the projects (the landed format: canonical field order, defaults left out) ----------
def project(case):
    folder, name, bars, cases = CASES[case]
    notes = [P11.note_json(1, tick, length, key, v) for tick, length, key, v in cases]
    channel = P11.instrument(1, name, "dawsome:instruments/%s/%s.sfz" % (folder, folder))
    d = {"format_version": 4, "sample_rate": SR, "tempo_mbpm": MBPM, "ppq": PPQ,
         "next_channel_id": 2, "channels": [channel],
         "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": bars, "notes": notes}]}
    return P11.dump(d)


def files():
    """Every file --write makes, by its name under DIR, as bytes."""
    out = {"p12_%s.json" % case: (project(case) + "\n").encode() for case in CASES}
    expected = {"renders": {case: answer(case) for case in CASES}}
    out["p12_expected.json"] = (P11.J(expected) + "\n").encode()
    return out


def main():
    args = sys.argv[1:]
    if len(args) == 2 and args[0] in ("--write", "--check"):
        stale = []
        for path, content in sorted(files().items()):
            target = os.path.join(args[1], path)
            if args[0] == "--write":
                with open(target, "wb") as f: f.write(content)
                continue
            try:
                with open(target, "rb") as f: same = f.read() == content
            except OSError: same = False
            if not same: stale.append(path)
        if stale: sys.exit("not what gen_p12_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    for case in CASES:
        a = answer(case)
        print("%-6s %s: %d frames, release %d; sha256 %s" % (case, a["set"], a["frames"],
              a["release"], a["sha256"][:16]))
        for n in a["notes"]:
            print("       key %d (root %d) byte %d at %d, off %d, silent from %d (speed %.17g)%s"
                  % (n["key"], n["root"], n["byte"], n["start"], n["off"], n["end"], n["speed"],
                     ", first wrap at %d" % n["wrap"] if "wrap" in n else ""))


if __name__ == "__main__":
    main()
