"""Mixer known answers. Stdlib only: python3 -I tests/fixtures/gen_p5_expected.py tests/fixtures
Writes the p5_*.json fixtures and two sine WAVs into <out_dir>, prints every expected value.
Expected values come from the mixer's formulas below, never from Dawsome."""
import json, math, os, struct, sys

OUT = sys.argv[1] if len(sys.argv) > 1 else "."
os.makedirs(OUT, exist_ok=True)

def f32(x):
    return struct.unpack("<f", struct.pack("<f", x))[0]

def tick_to_sample(tick, sr, mbpm, ppq=96):
    return tick * 60 * sr * 1000 // (mbpm * ppq)

# --- the rules under test ---------------------------------------------------------------------
UNITY = 0.8  # fader position of 0 dB (f64 constant)
def fader_gain(p):  # cube law, computed in f64 from the f32 position, then rounded to f32
    r = f32(p) / UNITY
    return f32(r * r * r)

def channel_pan(pan):  # circular law normalized to unity at center (sines only, so ends are exact)
    return (f32(math.sqrt(2) * math.sin((1 - pan) * math.pi / 4)),
            f32(math.sqrt(2) * math.sin((1 + pan) * math.pi / 4)))

def insert_pan(p, l, r):  # crossfeed: the far side folds into the near side
    if p == 0:
        return l, r
    if p < 0:
        s, c = f32(math.sin(-p * math.pi / 2)), f32(math.sin((1 + p) * math.pi / 2))
        return f32(l + f32(r * s)), f32(r * c)
    s, c = f32(math.sin(p * math.pi / 2)), f32(math.sin((1 - p) * math.pi / 2))
    return f32(l * c), f32(r + f32(l * s))

# --- file writers ---------------------------------------------------------------------------
def write_float_wav(path, sr, frames):
    data = b"".join(struct.pack("<f", v) for v in frames)
    fmt = struct.pack("<HHIIHH", 3, 1, sr, sr * 4, 4, 32)
    body = b"WAVE" + b"fmt " + struct.pack("<I", len(fmt)) + fmt + b"data" + struct.pack("<I", len(data)) + data
    with open(path, "wb") as f:
        f.write(b"RIFF" + struct.pack("<I", len(body)) + body)

def sine(freq, sr, n, amp=0.5):
    return [f32(amp * math.sin(2 * math.pi * freq * i / sr)) for i in range(n)]

def channel(cid, name, sample, insert=0, volume=1.0, pan=0.0):
    c = {"id": cid, "name": name, "kind": "sampler", "sample": sample, "volume": volume, "pan": pan, "mute": False}
    if insert:
        c["insert"] = insert
    return c

def mixer(n=16, overrides=None, solo=(), effects=None, next_effect_id=1):
    inserts = []
    for i in range(n + 1):
        ins = {"id": i, "name": "Master" if i == 0 else f"Insert {i}", "fader": UNITY, "pan": 0.0,
               "mute": False, "routes": [] if i == 0 else [{"to": 0, "amount": 1.0}], "effects": []}
        ins.update((overrides or {}).get(i, {}))
        ins["effects"] = (effects or {}).get(i, [])
        inserts.append(ins)
    return {"next_effect_id": next_effect_id, "solo": list(solo), "inserts": inserts}

def routes(*pairs):
    return [{"to": t, "amount": a} for t, a in sorted(pairs)]

def effect(eid, slot, kind, params):
    return {"id": eid, "slot": slot, "processor": {"id": kind, "params": dict(sorted(params.items()))}}

def project(name, sr, chans, notes, bars, mix):
    notes = sorted(notes, key=lambda n: (n[1], n[0]))
    doc = {"format_version": 2, "sample_rate": sr, "tempo_mbpm": 120000, "ppq": 96,
           "next_channel_id": len(chans) + 1, "channels": chans,
           "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": bars,
                         "notes": [{"channel": c, "tick": t, "length": 0, "velocity": 1.0} for c, t in notes]}]}
    if mix is not None:
        doc["mixer"] = mix
    with open(os.path.join(OUT, name), "w") as f:
        f.write(json.dumps(doc, separators=(",", ":")) + "\n")
    return doc

def rms(xs):
    return math.sqrt(sum(x * x for x in xs) / len(xs))

# --- 1 pan law ------------------------------------------------------------------------------
S1K = sine(1000, 48000, 24000)
write_float_wav(os.path.join(OUT, "sine1k_48k.wav"), 48000, S1K)
write_float_wav(os.path.join(OUT, "sine3k_48k.wav"), 48000, sine(3000, 48000, 24000))
project("p5_pan.json", 48000,
        [channel(1, "left", "sine1k_48k.wav", pan=-1.0), channel(2, "center", "sine1k_48k.wav"),
         channel(3, "right", "sine1k_48k.wav", pan=1.0), channel(4, "bus", "sine1k_48k.wav", insert=1)],
        [(1, 0), (2, 384), (3, 768), (4, 1152)], 4, mixer(overrides={1: {"pan": -1.0}}))
print("1 pan  p5_pan.json 48 kHz --bars 4: frames", tick_to_sample(4 * 384, 48000, 120000))
for bar, (pan, ins) in enumerate([(-1.0, 0), (0.0, 0), (1.0, 0), (0.0, -1.0)]):
    gl, gr = channel_pan(pan)
    L = [f32(s * f32(f32(1.0 * 1.0) * gl)) for s in S1K]
    R = [f32(s * f32(f32(1.0 * 1.0) * gr)) for s in S1K]
    if ins:
        L, R = zip(*(insert_pan(ins, l, r) for l, r in zip(L, R)))
    start = bar * 96000
    print(f"  bar {bar+1} frames [{start},{start+24000}) chan pan {pan:+} insert pan {ins:+}: gains ({gl!r},{gr!r})"
          f" RMS L {rms(L):.7f} R {rms(R):.7f}; frames [{start+24000},{start+96000}) all 0.0")

# --- 2 fader and sends ----------------------------------------------------------------------
P_HALF = 0.63496
g_half = fader_gain(P_HALF)
project("p5_gain.json", 44100,
        [channel(1, "max", "impulse.wav", insert=3), channel(2, "unity", "impulse.wav", insert=1),
         channel(3, "half", "impulse.wav", insert=2), channel(4, "send", "impulse.wav", insert=4),
         channel(5, "postfader", "impulse.wav", insert=6)],
        [(1, 0), (5, 48), (2, 96), (3, 192), (4, 288)], 1,
        mixer(overrides={2: {"fader": P_HALF}, 3: {"fader": 1.0}, 4: {"routes": routes((0, 1.0), (5, 0.5))},
                         6: {"fader": P_HALF, "routes": routes((0, 1.0), (7, 0.5))}}))
post = f32(g_half + f32(f32(g_half * 0.5) * fader_gain(UNITY)))
print(f"2 gain p5_gain.json 44.1 kHz --bars 1: 88200 frames; fader gains unity {fader_gain(UNITY)!r} "
      f"p={P_HALF} -> {g_half!r} ({20*math.log10(g_half):.4f} dB) p=1.0 -> {fader_gain(1.0)!r} "
      f"({20*math.log10(fader_gain(1.0)):.4f} dB)")
print(f"  nonzero frames (both sides): 0:{fader_gain(1.0)!r} 11025:{post!r} 22050:1.0 44100:{g_half!r} 66150:1.5")

# --- 3 mute / solo --------------------------------------------------------------------------
ms_chans = [channel(i, f"c{i}", "impulse.wav", insert=i) for i in range(1, 6)]
ms_notes = [(1, 0), (5, 48), (2, 96), (3, 192), (4, 288)]
ms_over = {2: {"mute": True}, 3: {"routes": routes((0, 1.0), (4, 1.0))}, 5: {"routes": routes((3, 1.0))}}
project("p5_mute.json", 44100, ms_chans, ms_notes, 1, mixer(overrides=ms_over))
project("p5_solo.json", 44100, ms_chans, ms_notes, 1, mixer(overrides=ms_over, solo=[3]))
print("3 mute p5_mute.json: 0:1.0 11025:2.0 22050:0.0 44100:2.0 66150:1.0 (insert 2 muted; 5->3 only; 3->master+4)")
print("  solo p5_solo.json (solo [3]): 0:0.0 11025:2.0 22050:0.0 44100:2.0 66150:1.0 (5 upstream, 4 downstream)")

# --- 4 EQ (RBJ peaking) ---------------------------------------------------------------------
def peaking(f0, gain_db, q, fs):
    A = 10 ** (gain_db / 40); w0 = 2 * math.pi * f0 / fs; al = math.sin(w0) / (2 * q); c = math.cos(w0)
    b = [1 + al * A, -2 * c, 1 - al * A]; a = [1 + al / A, -2 * c, 1 - al / A]
    return [x / a[0] for x in b], [x / a[0] for x in a]

def mag(b, a, f, fs):
    z = complex(math.cos(2 * math.pi * f / fs), -math.sin(2 * math.pi * f / fs))
    return abs((b[0] + b[1] * z + b[2] * z * z) / (a[0] + a[1] * z + a[2] * z * z))

def amplitude(y, f, fs, lo, hi):  # projection on sin/cos over a whole number of periods
    s = sum(y[n] * math.sin(2 * math.pi * f * n / fs) for n in range(lo, hi))
    c = sum(y[n] * math.cos(2 * math.pi * f * n / fs) for n in range(lo, hi))
    return 2 * math.hypot(s, c) / (hi - lo)

b, a = peaking(1000, 6.0, 1.0, 48000)
eq = {"mid_freq": 1000.0, "mid_gain_db": 6.0, "mid_q": 1.0}
for name, wav, f in [("p5_eq_f0.json", "sine1k_48k.wav", 1000), ("p5_eq_3k.json", "sine3k_48k.wav", 3000)]:
    project(name, 48000, [channel(1, "sine", wav, insert=1)], [(1, 0)], 1,
            mixer(effects={1: [effect(1, 0, "dawsome.eq3", eq)]}, next_effect_id=2))
    x = sine(f, 48000, 24000)
    y, x1, x2, y1, y2 = [], 0.0, 0.0, 0.0, 0.0
    for v in x:  # direct form I in f64, a check on the measuring window only
        out = b[0] * v + b[1] * x1 + b[2] * x2 - a[1] * y1 - a[2] * y2
        x2, x1, y2, y1 = x1, v, y1, out
        y.append(f32(out))
    print(f"4 eq   {name}: |H({f})| = {mag(b, a, f, 48000):.7f}; expected amplitude in frames [6000,18000) "
          f"= {0.5 * mag(b, a, f, 48000):.7f} (simulated {amplitude(y, f, 48000, 6000, 18000):.7f}); both sides equal")
print(f"  coefficients b={['%.10f' % v for v in b]} a={['%.10f' % v for v in a]}")

# --- 5 delay --------------------------------------------------------------------------------
D_MS = round(10.0 * 44100 / 1000)
D_SYNC = tick_to_sample(3 * 24, 44100, 120000)
project("p5_delay_ms.json", 44100, [channel(1, "click", "impulse.wav", insert=1)], [(1, 0)], 1,
        mixer(effects={1: [effect(1, 0, "dawsome.delay", {"time_ms": 10.0, "feedback": 0.5, "mix": 0.5})]}, next_effect_id=2))
project("p5_delay_sync.json", 44100, [channel(1, "click", "impulse.wav", insert=1)], [(1, 0)], 1,
        mixer(effects={1: [effect(1, 0, "dawsome.delay", {"sync_steps": 3.0, "feedback": 0.5, "mix": 1.0})]}, next_effect_id=2))
print(f"5 delay p5_delay_ms.json: D = round(10 ms * 44100/1000) = {D_MS}; frame 0 = 0.5; frame {D_MS}k = 0.5^k for k=1..20: "
      + ", ".join(f"{D_MS*k}:{0.5**k!r}" for k in range(1, 5)) + f", ... {D_MS*20}:{0.5**20!r}; other frames < {D_MS*21} are 0.0")
print(f"  p5_delay_sync.json: 3 steps = tick 72 -> D = {D_SYNC}; echoes (not tick 144's {tick_to_sample(144,44100,120000)}) "
      + ", ".join(f"{D_SYNC*k}:{0.5**(k-1)!r}" for k in range(1, 6) if D_SYNC * k < 88200) + "; frame 0 = 0.0; all else 0.0")

# --- 6 reverb (structural) ------------------------------------------------------------------
for name, m, fx in [("p5_reverb.json", 1.0, True), ("p5_reverb_dry.json", 0.0, True), ("p5_reverb_none.json", None, False)]:
    eff = {1: [effect(1, 0, "dawsome.reverb", {"mix": m})]} if fx else {}
    project(name, 44100, [channel(1, "click", "impulse.wav", insert=1)], [(1, 0)], 1,
            mixer(effects=eff, next_effect_id=2 if fx else 1, overrides={1: {"name": "Verb"}}))
print("6 reverb p5_reverb.json: frames [0, 1102) exactly 0 (shortest comb >= 25 ms); some |y| > 1e-4 at frame >= 22050;"
      " RMS[44100,66150) < RMS[4410,22050); L != R somewhere. p5_reverb_dry.json == p5_reverb_none.json (cmp)")

# --- 7 routing order and cycle --------------------------------------------------------------
project("p5_chain.json", 44100, [channel(1, "click", "impulse.wav", insert=3)], [(1, 0)], 1,
        mixer(overrides={3: {"routes": routes((2, 1.0))}, 2: {"routes": routes((1, 1.0))}}))
project("p5_cycle.json", 44100, [channel(1, "click", "impulse.wav", insert=1)], [(1, 0)], 1,
        mixer(overrides={1: {"routes": routes((0, 1.0), (2, 1.0))}, 2: {"routes": routes((3, 1.0))}, 3: {"routes": routes((1, 1.0))}}))
print("7 order p5_chain.json (3->2->1->master): frame 0 = 1.0 (index order would give 0.0); p5_cycle.json (1->2->3->1): "
      "render exits 1, stderr names 'routing cycle' and inserts 1, 2, 3; no WAV written")

# --- 8 saturated ----------------------------------------------------------------------------
project("p5_saturated.json", 44100, [channel(i, f"c{i}", "impulse.wav", insert=i, volume=4.0) for i in range(1, 17)],
        [(i, 0) for i in range(1, 17)], 1,
        mixer(overrides={i: {"fader": 1.0} for i in range(17)}))
acc = 0.0
for i in range(16):
    acc = f32(acc + f32(4.0 * fader_gain(1.0)))
print(f"8 saturated p5_saturated.json: insert out {f32(4.0*fader_gain(1.0))!r} x16 = {acc!r}; master x{fader_gain(1.0)!r}"
      f" -> frame 0 = {f32(acc*fader_gain(1.0))!r} both sides, written unclamped; master meter peak = same; rest 0.0")
print("checks: center pan gains", channel_pan(0.0), "unity", fader_gain(UNITY), "f32(0.8)/0.8 cubed", (f32(0.8)/0.8)**3)
