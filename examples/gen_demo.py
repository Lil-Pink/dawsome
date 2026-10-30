"""The demo project: an eight-bar song made only of sounds synthesized here (no recorded or
third-party audio), and the known answers its test reads.

  kick, snare, hat, click   16-bit mono 44.1 kHz WAVs in sounds/, each a short decaying body
                            with a sharp first step (the hit) far taller than any step inside it
  demo.json                 the rack (one channel per sound, each on its own insert, the click's
                            insert with a synced delay), pattern 1 "Beat" and pattern 2 "Riff"
                            (piano-roll notes), the playlist (Beat bars 1-8, Riff bars 3-8) and
                            an automation clip fading the master out over bars 7-8, in the
                            project file's canonical form so the app saves it back to its bytes
  demo_expected.json        the first bar of the song: its length in frames, the frames where
                            a hit starts, and the threshold a sample-to-sample step must pass to
                            count as one

The onsets come from the note ticks through the tick-to-sample rule floor(t*60*sr*1000 /
(mbpm*ppq)), never from Dawsome. The threshold sits halfway between the smallest step a hit makes
and the largest step anything else makes in a mix of the first bar computed here (the sounds at
each note's frame, scaled by velocity, channel volume and the channel pan law; centered inserts at
unity change nothing); the script refuses a design whose margin is under MARGIN. It mixes the
whole song the same way, with each note at its key's speed, the click's delay and the master's
fade added, and refuses a design whose loudest sample is over PEAK.

Run:
  python3 -I gen_demo.py              print the known answers
  python3 -I gen_demo.py --write DIR  also write sounds/*.wav, demo.json, demo_expected.json in DIR
"""
import json, math, os, struct, sys

SR = 44100; PPQ = 96; STEP = 24; BAR = 4 * PPQ; MBPM = 124000
MARGIN = 0.1
# The loudest the whole song may get, so it plays with headroom on any device. It peaks at 0.8843
# from bar 3 on, where the click's echoes add to a kick and a snare; bar 1 alone peaks at 0.8521.
PEAK = 0.9

def t2s(t): return t * 60 * SR * 1000 // (MBPM * PPQ)            # the one tick-to-sample rule
def f32(x): return struct.unpack("<f", struct.pack("<f", x))[0]
def jf(x):
    """An f32 written the way the project file writes it: the shortest digits that read back."""
    x = f32(x)
    for p in range(1, 10):
        s = "%.*g" % (p, x)
        if f32(float(s)) == x: break
    return s if ("." in s or "e" in s) else s + ".0"
class F:
    """A number the project file stores as an f32."""
    def __init__(self, x): self.x = x
def J(v):
    """`v` as compact JSON, each F in the project file's f32 form."""
    if isinstance(v, F): return jf(v.x)
    if isinstance(v, dict): return "{" + ",".join(json.dumps(k) + ":" + J(x) for k, x in v.items()) + "}"
    if isinstance(v, list): return "[" + ",".join(J(x) for x in v) + "]"
    if isinstance(v, float): raise TypeError("write %r as an F" % v)
    return json.dumps(v)

# ---------- the sounds: s16 quantized, as the file stores them ----------
def xorshift(x):
    x ^= (x << 13) & 0xFFFFFFFF; x ^= x >> 17; x ^= (x << 5) & 0xFFFFFFFF
    return x & 0xFFFFFFFF
def quantize(values): return [max(-32767, min(32767, round(v * 32768))) for v in values]
def fade_out(values, frames):
    """The last `frames` ramp to silence, so a sound never ends on a step."""
    n = len(values)
    return [v * min(1.0, (n - 1 - i) / frames) for i, v in enumerate(values)]
def hit(values, height, decay):
    """`values` with a sharp first step of `height` decaying over `decay` frames: the hit."""
    return [v + height * math.exp(-i / decay) for i, v in enumerate(values)]

def kick():
    n = int(0.30 * SR); phase = 0.0; out = []
    for i in range(n):
        t = i / SR
        out.append(0.85 * math.exp(-t / 0.12) * math.sin(phase))
        phase += 2 * math.pi * (48 + 110 * math.exp(-t / 0.03)) / SR
    return quantize(fade_out(hit(out, 0.75, 6), 400))
def snare():
    n = int(0.22 * SR); x = 2463534242; low = 0.0; out = []
    for i in range(n):
        t = i / SR
        x = xorshift(x); low += 0.12 * ((x / 2**32 * 2 - 1) - low)    # one-pole low-passed noise
        out.append(0.35 * math.exp(-t / 0.05) * math.sin(2 * math.pi * 185 * t) + 0.5 * math.exp(-t / 0.06) * low)
    return quantize(fade_out(hit(out, 0.7, 5), 300))
def hat():
    n = int(0.07 * SR); out = []
    for i in range(n):
        t = i / SR
        ring = sum(math.sin(2 * math.pi * f * t) for f in (6100, 7300, 8900, 10700))
        out.append(0.03 * math.exp(-t / 0.02) * ring)
    return quantize(fade_out(hit(out, 0.7, 3), 200))
def click():
    n = int(0.06 * SR)
    out = [0.5 * math.exp(-i / SR / 0.012) * math.sin(2 * math.pi * 1000 * i / SR) for i in range(n)]
    return quantize(fade_out(hit(out, 0.6, 4), 150))
SOUNDS = {"kick": kick(), "snare": snare(), "hat": hat(), "click": click()}

def wav_bytes(samples):
    data = struct.pack("<%dh" % len(samples), *samples)
    fmt = struct.pack("<HHIIHH", 1, 1, SR, SR * 2, 2, 16)
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack("<I", len(data)) + data
    return b"RIFF" + struct.pack("<I", len(body)) + body

# ---------- the project ----------
# (id, name, sound, volume, pan, insert)
CHANNELS = [(1, "Kick", "kick", 0.8, 0.0, 1), (2, "Snare", "snare", 0.6, -0.1, 2),
            (3, "Hat", "hat", 0.8, 0.25, 3), (4, "Click", "click", 0.6, 0.3, 4)]
ACCENT, NORMAL, SOFT = 1.0, 100 / 128, 80 / 128
def step(channel, s, velocity, length=0, key=60):
    n = {"channel": channel, "tick": s * STEP, "length": length, "velocity": F(velocity)}
    if key != 60: n["key"] = key
    return n
BEAT = ([step(1, 0, ACCENT)] + [step(1, s, NORMAL) for s in (4, 8, 12)]
        + [step(2, s, 0.9) for s in (4, 12)]
        + [step(3, s, NORMAL) for s in (2, 6, 10, 14)] + [step(3, s, SOFT) for s in (7, 15)])
RIFF = [step(4, s, NORMAL, length=STEP, key=k) for s, k in ((0, 60), (3, 63), (6, 67), (8, 70), (11, 67), (14, 63))]
def ordered(notes): return sorted(notes, key=lambda n: (n["tick"], n["channel"], n.get("key", 60)))

def insert(i, name, effects=()):
    return {"id": i, "name": name, "fader": F(0.8), "pan": F(0.0), "mute": False,
            "routes": [] if i == 0 else [{"to": 0, "amount": F(1.0)}], "effects": list(effects)}
DELAY = {"id": 1, "slot": 0, "processor": {"id": "dawsome.delay",
         "params": {"feedback": F(0.35), "mix": F(0.3), "sync_steps": F(3.0)}}}
NAMES = {1: "Kick", 2: "Snare", 3: "Hat", 4: "Click"}
MIXER = {"next_effect_id": 2, "solo": [],
         "inserts": [insert(0, "Master")] + [insert(i, NAMES.get(i, "Insert %d" % i), [DELAY] if i == 4 else [])
                                             for i in range(1, 17)]}
TRACKS = [{"id": i, "name": {1: "Beat", 2: "Riff", 3: "Fade"}.get(i, "Track %d" % i), "mute": False} for i in range(1, 9)]
CLIPS = [{"id": 1, "track": 1, "start": 0, "length": 8 * BAR, "kind": "pattern", "pattern": 1},
         {"id": 2, "track": 2, "start": 2 * BAR, "length": 6 * BAR, "kind": "pattern", "pattern": 2},
         {"id": 3, "track": 3, "start": 6 * BAR, "length": 2 * BAR, "kind": "automation", "automation": 1}]
AUTOMATION = {"next_id": 2, "list": [{"id": 1, "target": {"insert_fader": 0},
              "points": [{"tick": 0, "value": F(0.8)}, {"tick": 2 * BAR, "value": F(0.0)}]}]}
def channel(i, name, sound, volume, pan, ins):
    return {"id": i, "name": name, "kind": "sampler", "sample": "sounds/%s.wav" % sound,
            "volume": F(volume), "pan": F(pan), "mute": False, "insert": ins}
PROJECT = {"format_version": 2, "sample_rate": SR, "tempo_mbpm": MBPM, "ppq": PPQ,
           "next_channel_id": len(CHANNELS) + 1, "channels": [channel(*c) for c in CHANNELS],
           "patterns": [{"id": 1, "name": "Beat", "length_bars": 1, "notes": ordered(BEAT)},
                        {"id": 2, "name": "Riff", "length_bars": 1, "notes": ordered(RIFF)}],
           "playlist": {"next_track_id": 9, "next_clip_id": 4, "tracks": TRACKS, "clips": CLIPS},
           "mixer": MIXER, "automation": AUTOMATION}

# ---------- the mix, as the player makes it ----------
def pan_gains(pan):
    return [f32(math.sqrt(2) * math.sin((1 + towards) * math.pi / 4)) for towards in (-pan, pan)]
def fader_gain(position): return f32((f32(position) / 0.8) ** 3)  # the cube law, unity at 0.8
SEMITONES = [2 ** (s / 12) for s in range(12)]
def speed(key): return SEMITONES[key % 12] * 2.0 ** (key // 12 - 5)  # 2^((key-60)/12), 1 at 60
def play(sides, n, tick):
    """Note n's sound into sides (left, right) from the frame of `tick`, read at its key's speed
    (linear interpolation between frames, 0 past the last) and scaled by velocity, channel volume
    and the channel pan law; cut where sides end."""
    _, _, sound, volume, pan, _ = CHANNELS[n["channel"] - 1]
    q = SOUNDS[sound]; start = t2s(tick); gains = pan_gains(pan); s = speed(n.get("key", 60))
    for k in range(len(sides[0]) - start):
        at = k * s; i = int(at)
        if i >= len(q): break
        x = (q[i] + (at - i) * ((q[i + 1] if i + 1 < len(q) else 0) - q[i])) / 32768
        for side in (0, 1):
            sides[side][start + k] += x * f32(n["velocity"].x) * f32(volume) * gains[side]
def song_peak():
    """The loudest sample of the whole song: every pattern clip's notes into their channels'
    inserts; the delay's insert through it, per side d[n] = x[n] + feedback*d[n-D] heard as
    (1-mix)*x[n] + mix*d[n-D], D its synced steps by the tick-to-sample rule; the inserts at
    unity (fader 0.8, centered, send 1) into the master, whose fader position the automation
    clip moves in a straight line between its points, holding the last one after them."""
    def unity(i): return i["fader"].x == 0.8 and i["pan"].x == 0.0 and [(r["to"], r["amount"].x) for r in i["routes"]] == [(0, 1.0)]
    if not all(unity(i) for i in MIXER["inserts"][1:]): sys.exit("the song's mix takes every insert at unity")
    frames = t2s(max(clip["start"] + clip["length"] for clip in CLIPS))  # the song's end
    buses = {c[5]: [[0.0] * frames, [0.0] * frames] for c in CHANNELS}
    patterns = {p["id"]: p for p in PROJECT["patterns"]}
    for clip in CLIPS:
        if clip["kind"] != "pattern": continue
        p = patterns[clip["pattern"]]
        for loop in range(0, clip["length"], p["length_bars"] * BAR):
            for n in p["notes"]:
                if loop + n["tick"] < clip["length"]:
                    play(buses[CHANNELS[n["channel"] - 1][5]], n, clip["start"] + loop + n["tick"])
    params = DELAY["processor"]["params"]
    delay = t2s(int(params["sync_steps"].x) * STEP)
    feedback, mix = f32(params["feedback"].x), f32(params["mix"].x)
    for insert in MIXER["inserts"]:
        if DELAY not in insert["effects"]: continue
        for side in buses[insert["id"]]:
            line = [0.0] * frames
            for k in range(frames):
                echo = line[k - delay] if k >= delay else 0.0
                line[k] = side[k] + feedback * echo
                side[k] = (1 - mix) * side[k] + mix * echo
    fade = next(clip for clip in CLIPS if clip["kind"] == "automation")
    first, last = AUTOMATION["list"][0]["points"]
    s0, s1 = t2s(fade["start"] + first["tick"]), t2s(fade["start"] + last["tick"])
    v0, v1 = f32(first["value"].x), f32(last["value"].x)
    start, before = t2s(fade["start"]), fader_gain(MIXER["inserts"][0]["fader"].x)
    def gain(k):
        if k < start: return before
        return fader_gain(v0 + (v1 - v0) * (min(max(k, s0), s1) - s0) / (s1 - s0))
    return max(abs(sum(bus[side][k] for bus in buses.values()) * gain(k))
               for k in range(frames) for side in (0, 1))

# ---------- the first bar's known answers ----------
def expected():
    frames = t2s(BAR)
    onsets = sorted({t2s(n["tick"]) for n in BEAT})
    sides = [[0.0] * frames, [0.0] * frames]
    for n in BEAT: play(sides, n, n["tick"])
    def step_at(i): return max(abs(s[i] - (s[i - 1] if i else 0.0)) for s in sides)
    hits = min(step_at(i) for i in onsets)
    others = max(step_at(i) for i in range(frames) if i not in onsets)
    if hits - others < MARGIN:
        sys.exit("design margin %.3f is under %.1f: hits %.3f, other steps %.3f" % (hits - others, MARGIN, hits, others))
    peak = song_peak()
    if peak > PEAK:
        sys.exit("the song peaks at %.4f, over %g" % (peak, PEAK))
    return {"bar_frames": frames, "onsets": onsets, "threshold": round((hits + others) / 2, 4),
            "smallest_hit_step": round(hits, 4), "largest_other_step": round(others, 4)}

if __name__ == "__main__":
    answers = expected()
    print(json.dumps(answers))
    if len(sys.argv) == 3 and sys.argv[1] == "--write":
        out = sys.argv[2]
        os.makedirs(os.path.join(out, "sounds"), exist_ok=True)
        for name, samples in SOUNDS.items():
            with open(os.path.join(out, "sounds", name + ".wav"), "wb") as f: f.write(wav_bytes(samples))
        with open(os.path.join(out, "demo.json"), "w") as f: f.write(J(PROJECT) + "\n")
        with open(os.path.join(out, "demo_expected.json"), "w") as f: f.write(json.dumps(answers, indent=1) + "\n")
