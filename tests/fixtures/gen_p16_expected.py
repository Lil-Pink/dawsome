"""Known answers for the external input channel: a channel whose sound is an audio input (the
frames a device delivers, or in an offline render the file given with --input-feed), through the
channel's volume, pan, mute and mixer insert like any channel's sound.

Expected values come from the rules written out below, never from engine code:
  feed      output frame k of a render (counted from its first frame) plays frame k of the feed:
            no delay is added offline (the feed delay is 0 frames; a live input's own delay is
            its ring's fill, which never reaches an offline render); past the feed's last frame
            every frame is silence (an underrun plays zeros, never a frame again)
  pick      the feed file stands in for the device: an input channel's channels pick its
            frames' sides, {"stereo":[l,r]} sides l and r (0 = the first), {"mono":n} side n
            played as a mono sound
  volume    linear, the channel's volume times each frame (f32), 0 while muted
  pan       the existing laws (src/dsp/laws.rs): a mono sound through the channel pan law,
            gains f32(sqrt(2)*sin((1 -+ pan)*pi/4)) for left and right, the gain applied as
            v * f32(volume * side); a stereo sound through the balance law, both sides times
            the volume first, then at a pan other than 0 the insert pan's crossfeed:
            fold = f32(sin(|pan|*pi/2)), keep = f32(sin((1-|pan|)*pi/2)); panned left
            (L + R*fold, R*keep), right (L*keep, R + L*fold)
  insert    the channel plays into its insert's bus (zeroed, then added into); an insert's
            fader by the cube law (position/0.8)^3 (0.4 gives 0.125 exactly), its pan 0
            changing nothing, then its route to the master at amount 1.0 added in
  notes     notes on an input channel play nothing (steps on it here change no frame)
  sampler   the 48 kHz one-frame impulse at velocity 1.0, volume 1.0, pan 0 plays 1.0 on both
            sides on its note's first sample, s(t) = floor(t*60*48000*1000 / (mbpm*96)); at
            240 BPM s(t) = 125*t
  version   format_version 6 for a project holding an input channel

  p16_input.wav     the feed: 1 s at 48 kHz, 16-bit stereo, left a 440 Hz sine at -6 dBFS
                    (q = round(10^(-6/20)*sin(2*pi*440*n/48000)*32768), read as q/32768),
                    right a 660 Hz sine at -12 dBFS the same way (so a stereo pick's
                    crossfeed folds a sounding right side into the left)
  p16_impulse.wav   one frame of 1.0, 32-bit float mono at 48 kHz
  p16_unity.json    an input channel {"stereo":[0,1]} at volume 1.0, pan 0, with four steps:
                    the output is the feed
  p16_half_left.json   the same at volume 0.5, pan -1: the balance law, (L/2 + R/2, 0)
  p16_mono_left.json   {"mono":0} at volume 0.5, pan -1: the channel pan law, (L*0.70710677, 0)
  p16_muted.json    the input channel muted, and a sampler of p16_impulse.wav with four steps:
                    only the sampler's four hits
  p16_insert.json   the input channel into insert 1 (fader 0.4: gain 0.125), which routes to
                    the master: (L/8, R/8)
  p16_expected.json each render: its project, bars, frames, the SHA-256 of its frames (32-bit
                    float, little-endian, left then right), how many frames sound, and probes
                    (frame, left, right) every 1000 frames, at the sine's first peak, the
                    feed's last frame, the first after it and the sampler's hits

Renders are 240 BPM at 48 kHz, so a bar is 48000 frames, the feed's length: "unity_2_bars" is
the feed then a bar of silence.

Run:
  python3 -I gen_p16_expected.py              print the known answers
  python3 -I gen_p16_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p16_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import hashlib, json, math, os, struct, sys

SR = 48000; PPQ = 96; MBPM = 240000
FEED_FRAMES = SR                                  # 1 s
SINE_HZ = 440; SINE_DB = -6.0                     # the left side
TONE_HZ = 660; TONE_DB = -12.0                    # the right side
BAR = 4 * PPQ


def f32(x): return struct.unpack("<f", struct.pack("<f", x))[0]
def J(d): return json.dumps(d, separators=(",", ":"))
def s(t): return t * 60 * SR * 1000 // (MBPM * PPQ)


def feed_values():
    """The feed's frames as 16-bit values (left, right); the decoder reads each as q/32768."""
    def side(n, hz, db):
        q = round(10 ** (db / 20) * math.sin(2 * math.pi * hz * n / SR) * 32768)
        return max(-32768, min(32767, q))
    return [(side(n, SINE_HZ, SINE_DB), side(n, TONE_HZ, TONE_DB)) for n in range(FEED_FRAMES)]


def wav_s16_stereo(frames):
    data = b"".join(struct.pack("<hh", l, r) for l, r in frames)
    fmt = struct.pack("<HHIIHH", 1, 2, SR, SR * 4, 4, 16)
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack("<I", len(data)) + data
    return b"RIFF" + struct.pack("<I", len(body)) + body


def wav_f32_mono(values):
    data = struct.pack("<%df" % len(values), *values)
    fmt = struct.pack("<HHIIHH", 3, 1, SR, SR * 4, 4, 32)
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack("<I", len(data)) + data
    return b"RIFF" + struct.pack("<I", len(body)) + body


# --- the laws, from their formulas -----------------------------------------------------------

def channel_pan_gains(pan):
    side = lambda towards: f32(math.sqrt(2) * math.sin((1.0 + towards) * math.pi / 4))
    return side(-pan), side(pan)


def insert_pan_gains(pan):
    away = abs(pan)
    return f32(math.sin(away * math.pi / 2)), f32(math.sin((1.0 - away) * math.pi / 2))


def fader_gain(position):
    ratio = f32(position) / 0.8                      # the f32 position widened, over the decimal
    return f32(ratio * ratio * ratio)


def mono_through(v, volume, pan):
    left, right = channel_pan_gains(pan)
    return f32(v * f32(volume * left)), f32(v * f32(volume * right))


def stereo_through(l, r, volume, pan):
    sl, sr = f32(l * volume), f32(r * volume)
    if pan == 0.0:
        return sl, sr
    fold, keep = insert_pan_gains(pan)
    if pan < 0.0:
        return f32(sl + f32(sr * fold)), f32(sr * keep)
    return f32(sl * keep), f32(sr + f32(sl * fold))


def bus_add(bus, value):
    """A frame added into a bus that starts at +0.0 (IEEE: 0.0 + -0.0 is 0.0)."""
    return f32(bus[0] + value[0]), f32(bus[1] + value[1])


# --- the projects ----------------------------------------------------------------------------

def input_channel(i, channels, volume=1.0, pan=0.0, mute=False, insert=None):
    c = {"id": i, "name": "Oriole", "kind": "input",
         "input": {"device": "USB Audio Line Input", "channels": channels},
         "volume": volume, "pan": pan, "mute": mute}
    if insert is not None: c["insert"] = insert
    return c


def sampler_channel(i, sample):
    return {"id": i, "name": "hit", "kind": "sampler", "sample": sample,
            "volume": 1.0, "pan": 0.0, "mute": False}


def steps(cid):
    return [{"channel": cid, "tick": t, "length": 0, "velocity": 1.0} for t in (0, 96, 192, 288)]


def project(channels, notes, mixer=None):
    notes = sorted(notes, key=lambda n: (n["tick"], n["channel"]))
    d = {"format_version": 6, "sample_rate": SR, "tempo_mbpm": MBPM, "ppq": PPQ,
         "next_channel_id": max(c["id"] for c in channels) + 1, "channels": channels,
         "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": notes}]}
    if mixer is not None: d["mixer"] = mixer
    return d


def insert_mixer():
    """The master and insert 1 (fader 0.4), which routes to the master."""
    def insert(i, name, fader, routes):
        return {"id": i, "name": name, "fader": fader, "pan": 0.0, "mute": False,
                "routes": routes, "effects": []}
    return {"next_effect_id": 1, "solo": [],
            "inserts": [insert(0, "Master", 0.8, []),
                        insert(1, "Insert 1", 0.4, [{"to": 0, "amount": 1.0}])]}


STEREO = {"stereo": [0, 1]}
MONO_LEFT = {"mono": 0}

PROJECTS = {
    "p16_unity.json": project([input_channel(1, STEREO)], steps(1)),
    "p16_half_left.json": project([input_channel(1, STEREO, volume=0.5, pan=-1.0)], []),
    "p16_mono_left.json": project([input_channel(1, MONO_LEFT, volume=0.5, pan=-1.0)], []),
    "p16_muted.json": project([input_channel(1, STEREO, mute=True),
                               sampler_channel(2, "p16_impulse.wav")], steps(2)),
    "p16_insert.json": project([input_channel(1, STEREO, insert=1)], [], insert_mixer()),
}


# --- the renders -----------------------------------------------------------------------------

def render(name, frames):
    """The frames the rules above give for render `name`."""
    feed = [(q / 32768, r / 32768) for q, r in feed_values()]
    out = []
    for k in range(frames):
        l, r = feed[k] if k < len(feed) else (0.0, 0.0)
        master = (0.0, 0.0)
        if name in ("unity", "unity_2_bars"):
            master = bus_add(master, stereo_through(l, r, 1.0, 0.0))
        elif name == "half_left":
            master = bus_add(master, stereo_through(l, r, 0.5, -1.0))
        elif name == "mono_left":
            master = bus_add(master, mono_through(l, 0.5, -1.0))
        elif name == "muted":
            if k in [s(t) for t in (0, 96, 192, 288)]:
                master = bus_add(master, (1.0, 1.0))
        elif name == "insert":
            bus = bus_add((0.0, 0.0), stereo_through(l, r, 1.0, 0.0))
            gain = fader_gain(0.4)
            bus = (f32(bus[0] * gain), f32(bus[1] * gain))
            master = bus_add(master, (f32(bus[0] * 1.0), f32(bus[1] * 1.0)))
        out.append(master)
    return out


# render name: (project, bars)
RENDERS = {
    "unity": ("p16_unity.json", 1),
    "unity_2_bars": ("p16_unity.json", 2),
    "half_left": ("p16_half_left.json", 1),
    "mono_left": ("p16_mono_left.json", 1),
    "muted": ("p16_muted.json", 1),
    "insert": ("p16_insert.json", 1),
}


def expected():
    renders = {}
    for name, (fixture, bars) in RENDERS.items():
        frames = s(bars * BAR)
        out = render(name, frames)
        data = struct.pack("<%df" % (2 * frames), *[v for frame in out for v in frame])
        at = sorted(set(list(range(0, frames, 1000)) + [frames - 1, 27, FEED_FRAMES - 1]
                        + [s(t) for t in (0, 96, 192, 288)]
                        + ([FEED_FRAMES] if frames > FEED_FRAMES else [])))
        renders[name] = {"fixture": fixture, "bars": bars, "frames": frames,
                         "sha256": hashlib.sha256(data).hexdigest(),
                         "sounding": sum(1 for frame in out if frame != (0.0, 0.0)),
                         "probes": [[k, out[k][0], out[k][1]] for k in at if k < frames]}
    return {"input": "p16_input.wav", "rate": SR, "input_frames": FEED_FRAMES,
            "feed_delay": 0, "renders": renders}


def files():
    out = {"p16_input.wav": wav_s16_stereo(feed_values()),
           "p16_impulse.wav": wav_f32_mono([1.0])}
    for name, doc in PROJECTS.items():
        out[name] = (J(doc) + "\n").encode()
    out["p16_expected.json"] = (J(expected()) + "\n").encode()
    return out


def main():
    args = sys.argv[1:]
    if len(args) == 2 and args[0] in ("--write", "--check"):
        stale = []
        for path, content in sorted(files().items()):
            target = os.path.join(args[1], path)
            if args[0] == "--write":
                with open(target, "wb") as f: f.write(content)
            else:
                try:
                    with open(target, "rb") as f: same = f.read() == content
                except OSError: same = False
                if not same: stale.append(path)
        if stale: sys.exit("not what gen_p16_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    for name, r in expected()["renders"].items():
        probes = {k: (l, rr) for k, l, rr in r["probes"]}
        print("%s: %s, %d bar(s), %d frames, %d sounding, frame 27 %r, sha256 %s" % (
            name, r["fixture"], r["bars"], r["frames"], r["sounding"], probes.get(27),
            r["sha256"][:16]))


if __name__ == "__main__":
    main()
