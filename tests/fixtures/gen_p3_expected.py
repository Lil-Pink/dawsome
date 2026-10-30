"""Piano-roll known answers (note editing, pitch from key, the device resampler).

Expected values come from these rules, never from engine code:
  pitch rule: a note plays its sample at speed 2^((key-60)/12), between frames by the 4-point
              cubic Hermite of gen_p11_expected.py (`hermite`: f64, rounded to f32 once), the
              points before the sample's first frame and past its end reading 0; key 60 plays
              the sample unchanged.
  copy rule:  Ctrl+B shifts the copies by the selection's extent (min start to max drawn end,
              a zero-length note drawn 24 ticks long) rounded up to the snap unit.

Run: python3 -I gen_p3_expected.py [--write DIR]
--write DIR also writes every fixture (p3_*.json, hold2000.wav) into DIR, which must hold
impulse.wav and ramp4.wav (the expected documents name those files)."""
import importlib.util, json, os, struct, sys
from decimal import Decimal, getcontext
from fractions import Fraction

def rules():
    """gen_p11_expected.py beside this script, whose `hermite` is the pitched read's rule."""
    sys.dont_write_bytecode = True                   # no __pycache__ beside the fixtures
    here = os.path.dirname(os.path.abspath(__file__))
    spec = importlib.util.spec_from_file_location("gen_p11_expected", os.path.join(here, "gen_p11_expected.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
hermite = rules().hermite

PPQ = 96; STEP = 24; BEAT = 96; BAR = 384; SR = 44100; MBPM = 120000

def t2s(tick, sr=SR, mbpm=MBPM): return tick * 60 * sr * 1000 // (mbpm * PPQ)  # the one tick-to-sample rule
def J(d): return json.dumps(d, separators=(',', ':'))
def ch(i, name, sample): return {"id": i, "name": name, "kind": "sampler", "sample": sample,
                                 "volume": 1.0, "pan": 0.0, "mute": False}
def note(c, tick, length, vel, key=60):
    n = {"channel": c, "tick": tick, "length": length, "velocity": vel}
    if key != 60: n["key"] = key
    return n
def key_of(n): return n.get("key", 60)
def nkey(n): return (n["tick"], n["channel"], key_of(n))           # the order notes are kept in
def doc(channels, notes, bars=1):
    return {"format_version": 2, "sample_rate": SR, "tempo_mbpm": MBPM, "ppq": PPQ,
            "next_channel_id": len(channels) + 1, "channels": channels,
            "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": bars,
                          "notes": sorted(notes, key=nkey)}]}
V100 = 100 / 128                     # velocity 100 on the 0..128 scale = 0.78125
A = note(1, 0, 48, V100); B = note(1, 96, 96, 0.5, 64)
KICK = [note(2, 192, 0, 1.0), note(2, 288, 0, 1.0)]
def roll(lead, bars=1): return doc([ch(1, "lead", "impulse.wav"), ch(2, "kick", "impulse.wav")], lead + KICK, bars)
def bars_needed(notes): return max(1, max(-(-(n["tick"] + max(n["length"], 1)) // BAR) for n in notes))

# --- piano-roll geometry (grid canvas px: key column 48 wide, rows 12 high, default 16 px/step) ---
LEFT, ROW, PXSTEP = 48, 12, 16
def tick_at(x, pxs=PXSTEP, h=0): return Fraction(x - LEFT) * STEP // pxs + h   # floor
def key_at(y, top): return top - y // ROW
def snap_down(t, u): return t // u * u
def snap_near(t, u): return (t + u // 2) // u * u                              # half up
def drawn_len(n): return n["length"] if n["length"] > 0 else STEP
TOP = max(key_of(n) for n in (A, B)) + 5                                       # top key on open

# --- render model: impulse = 1 frame of 1.0; length never ends a voice; 64 voices; a new note
#     takes the first free voice, else the one that started first (lowest slot on a tie), cut
#     at the new note's sample. A voice reads position n*r after n frames (the pitch rule). ---
getcontext().prec = 50
SEMITONE = [float(Decimal(2) ** (Decimal(s) / 12)) for s in range(12)]       # correctly rounded
def speed(key):
    octave, semitone = divmod(key - 60, 12)
    return SEMITONE[semitone] * 2.0 ** octave                                  # exact power of two
MARGIN = 1e-9   # every position used below stays this far from a whole frame unless it is one
def position(n, r):
    p = n * r; i = int(p); frac = p - i
    assert frac == 0 or MARGIN < frac < 1 - MARGIN, "too close to a frame boundary to be a known answer"
    return i, frac
def read(data, n, r):
    i, frac = position(n, r)
    if i >= len(data): return None
    if frac == 0: return data[i]
    at = lambda k: data[k] if 0 <= k < len(data) else 0.0
    return hermite(at(i - 1), data[i], at(i + 1), at(i + 2), frac)
def voice_frames(data, r):
    n = 0
    while position(n, r)[0] < len(data): n += 1
    return n
def render(d, bars, samples):
    p = d["patterns"][0]; L = p["length_bars"] * BAR; total = t2s(bars * BAR)
    gain = {c["id"]: (0.0 if c["mute"] else c["volume"]) for c in d["channels"]}
    smp = {c["id"]: samples[c["sample"]] for c in d["channels"]}
    ev = [n for n in p["notes"] if 0 <= n["tick"] < L]
    starts = [(t2s(k * L + n["tick"]), n) for k in range(bars * BAR // L + 1) for n in ev]
    voices, slots = [], [None] * 64                                            # [start, note, end]
    for s, n in [(s, n) for s, n in starts if s < total]:                      # passes, then notes
        if gain[n["channel"]] == 0.0: continue
        free = [i for i, v in enumerate(slots) if v is None or voices[v][2] <= s]
        i = free[0] if free else min(range(64), key=lambda j: (voices[slots[j]][0], j))
        if slots[i] is not None: voices[slots[i]][2] = min(voices[slots[i]][2], s)
        end = s + voice_frames(smp[n["channel"]], speed(key_of(n)))
        voices.append([s, n, end]); slots[i] = len(voices) - 1
    out = [0.0] * total
    for s, n, e in voices:
        data, r = smp[n["channel"]], speed(key_of(n))
        for f in range(s, min(e, total)):
            out[f] += read(data, f - s, r) * n["velocity"] * gain[n["channel"]]
    return out
def nz(out): return [(i, v) for i, v in enumerate(out) if v != 0.0]
IMP = {"impulse.wav": [1.0]}
RAMP4 = [1.0, 0.5, 0.25, 0.125]
HOLD = [0.25] * 2000
def show(tag, d, bars, samples=IMP):
    o = render(d, bars, samples); print(f"{tag} render {bars} bar(s): frames {len(o)} nonzero {nz(o)}")

written = {}                                                                   # file name -> doc
base = roll([A, B]); written["p3_roll.json"] = base
print("base p3_roll.json:", J(base)); show("base", base, 1)
print("geometry: top key on open =", TOP, "| key 60 row y", (TOP - 60) * ROW, "| key 64 row y", (TOP - 64) * ROW,
      "| A x", LEFT, "..", LEFT + 48 * PXSTEP // STEP, "| B x", LEFT + 96 * PXSTEP // STEP, "..", LEFT + 192 * PXSTEP // STEP)
y60 = (TOP - 60) * ROW + 6; y64 = (TOP - 64) * ROW + 6

# (a) move A by dragging its body (56,114) -> (76,114), snap Step (nearest)
dt = tick_at(76) - tick_at(56); new = snap_near(A["tick"] + dt, STEP)
a = roll([dict(A, tick=new), B]); written["p3_a_move.json"] = a
print(f"(a) press (56,{y60}) drag (76,{y60}): tick_at {tick_at(56)}->{tick_at(76)} raw start {A['tick']+dt} snapped {new}")
print("(a) doc:", J(a)); show("(a)", a, 1)

# (b) resize A by its right edge (78,114) -> (112,114): end snapped nearest
end = snap_near(A["tick"] + A["length"] + tick_at(112) - tick_at(78), STEP)
b = roll([dict(A, length=end - A["tick"]), B]); written["p3_b_resize.json"] = b
print(f"(b) resize: raw end {A['tick']+A['length']+tick_at(112)-tick_at(78)} snapped {end} length {end-A['tick']}")
print("(b) doc:", J(b)); show("(b)", b, 1); print("(b) render must equal the base render byte for byte (length does not end a voice)")
for length in (1, 96):
    h = doc([ch(1, "hold", "hold2000.wav")], [note(1, 0, length, 1.0)])
    if length == 1: written["p3_hold.json"] = h; print("(b) p3_hold.json:", J(h))
    else: written["p3_hold_96.json"] = h
    o = render(h, 1, {"hold2000.wav": HOLD})
    print(f"(b) hold note length {length} tick(s) = {Fraction(length*60*SR*1000, MBPM*PPQ)} samples nominal;",
          f"frames 0..{max(i for i,v in nz(o))} = {sorted(set(v for i,v in nz(o)))}, first silent frame {len(nz(o))}, total {len(o)}")

# (c) velocity lane (64 px tall, two steps a pixel): press A's stem (50,20) drag to (50,32);
# value = round((64 - y) * 128 / 64)
LANE = 64
def lane_value(y): return max(0, min(128, round((LANE - y) * 128 / LANE)))
val = lane_value(32); c = roll([dict(A, velocity=val / 128), B]); written["p3_c_velocity.json"] = c
print(f"(c) lane value at press {lane_value(20)}, at release {val} -> velocity {val/128}"); print("(c) doc:", J(c)); show("(c)", c, 1)

# (d) Ctrl+A, Ctrl+B at snap Step: offset = extent rounded up to the snap unit (the copy rule);
# (d2) Ctrl+B again: the copies are the selection, so they are copied once more, and the
# pattern grows to hold them, in the same undo step.
def duplicate(sel, u):
    lo = min(n["tick"] for n in sel); hi = max(n["tick"] + drawn_len(n) for n in sel)
    off = -(-(hi - lo) // u) * u
    return off, [dict(n, tick=n["tick"] + off) for n in sel]
off, copies = duplicate([A, B], STEP); dn = [A, B] + copies
d = roll(dn, bars_needed(dn + KICK)); written["p3_d_duplicate.json"] = d
print(f"(d) extent 0..{max(n['tick'] + drawn_len(n) for n in (A, B))} offset {off} copies at {[n['tick'] for n in copies]} bars -> {d['patterns'][0]['length_bars']}")
print("(d) doc:", J(d)); show("(d)", d, 1)
off2, copies2 = duplicate(copies, STEP); dn2 = dn + copies2
d2 = roll(dn2, bars_needed(dn2 + KICK)); written["p3_d2_duplicate_again.json"] = d2
print(f"(d2) offset {off2} copies at {[n['tick'] for n in copies2]} bars -> {d2['patterns'][0]['length_bars']}")
print("(d2) doc:", J(d2)); show("(d2)", d2, 2)

# (e) draw on empty (330,114): floor snap, draw length 24, velocity 100/128, pattern grows
t = snap_down(tick_at(330), STEP); en = note(1, t, STEP, V100, key_at(y60, TOP)); e_notes = [A, B, en]
e = roll(e_notes, bars_needed(e_notes + KICK)); written["p3_e_grow.json"] = e
print(f"(e) tick_at(330) {tick_at(330)} -> {t}, key {key_at(y60, TOP)}, bars -> {e['patterns'][0]['length_bars']}")
print("(e) doc:", J(e)); show("(e)", e, 2)

# (f1) move B body (120,66) -> (56,114): lands on A's (tick,key) -> A replaced
dtk = tick_at(56) - tick_at(120); dk = key_at(y60, TOP) - key_at(y64, TOP)
moved = dict(B, tick=snap_near(B["tick"] + dtk, STEP)); moved["key"] = 64 + dk
if moved["key"] == 60: moved.pop("key")
f1 = roll([moved]); written["p3_f1_replace.json"] = f1
print(f"(f1) press (120,{y64}) drag (56,{y60}): dt {dtk} dkey {dk} -> tick {moved['tick']} key {64+dk}; A replaced")
print("(f1) doc:", J(f1)); show("(f1)", f1, 1)
ov = roll([A, note(1, 24, 48, V100), B]); written["p3_f1_overlap.json"] = ov
print("(f1) partial same-key overlap is valid:", J(ov["patterns"][0]["notes"])); show("(f1) overlap", ov, 1)

# (f2) 65-note chord on one channel at tick 0, keys 0..64, velocity (k+1)/128. With pitch from
# key (the pitch rule) keys below 60 slow the one-frame impulse into a falling ramp, so frames after
# 0 sound too; frame 0 alone tells the voice rule apart.
chord = doc([ch(1, "chord", "impulse.wav")], [note(1, 0, 0, (k + 1) / 128, k) for k in range(65)])
written["p3_chord.json"] = chord
print("(f2) p3_chord.json:", J(chord)); show("(f2)", chord, 1)
print("(f2) frame 0 if the 65th note were dropped:", sum((k + 1) / 128 for k in range(64)),
      "| if it took the newest voice (key 63):", sum((k + 1) / 128 for k in range(65) if k != 63))

# (g) pitch from key on ramp4.wav (the pitch rule): rate 2^((key-60)/12), Hermite between frames
for k in (72, 48):
    g = doc([ch(1, "ramp", "ramp4.wav")], [note(1, 0, 0, 1.0, k)]); written[f"p3_g_key{k}.json"] = g
    print(f"(g) p3_g_key{k}.json:", J(g)); show(f"(g) key {k}", g, 1, {"ramp4.wav": RAMP4})
print("(g) key 60: [1.0, 0.5, 0.25, 0.125] (F3 plays it at velocity 0.5, unchanged)")

# (h) device resampler 44100 -> 48000 (Fft, fixed input chunk 512)
r = Fraction(48000, SR)
print("(h) ratio", r, "| impulse spacing", 22050 * r, "| bar", t2s(BAR) * r, "| 2 bars", t2s(2 * BAR) * r,
      "| peaks at d +", [int(s * r) for s in (0, 22050, 44100, 66150, 88200, 110250, 132300, 154350)],
      "| 22050 % 147 =", 22050 % 147)

# --- unit-level answers for the gesture mapping ---
print("snap_near(12, 24) =", snap_near(12, 24), "| snap_near(11, 24) =", snap_near(11, 24),
      "| snap_down(423, 24) =", snap_down(423, 24))
print("tick_at at 6 px/step (4 ticks/px): x 48, 49, 50, 73 ->", [tick_at(x, 6) for x in (48, 49, 50, 73)],
      "| at 64 px/step (0.375 ticks/px): x 48, 50, 51, 56, 60 ->", [tick_at(x, 64) for x in (48, 50, 51, 56, 60)])

# --- UI probe (window logical px, --panels piano-roll, 1000x560): grid canvas at (0, 64+28) ---
# (the toolbar is 64 px; it was 56, then 72)
GY = 64 + 28; LANE_TOP = 560 - LANE
def hx(c): return tuple(int(c[i:i + 2], 16) for i in (1, 3, 5))
def mix(c, bg, a): return tuple(int(a * x + (1 - a) * y + 0.5) for x, y in zip(c, bg))
WHITE_ROW, BLACK_ROW = hx("#34373D"), hx("#2D3035"); LEAD, KICKC = hx("#E5533D"), hx("#4DA3E0")
LANE_BG = hx("#1F2125"); BAR_LINE = hx("#626873")
def row_y(k): return GY + (TOP - k) * ROW + 6
def xt(t): return LEFT + t * PXSTEP // STEP
probes = [
    ("note A center", (xt(24), row_y(60)), LEAD), ("note B center", (xt(144), row_y(64)), LEAD),
    ("ghost kick 192 center", (xt(192) + 8, row_y(60)), mix(KICKC, WHITE_ROW, 0.3)),
    ("ghost kick 288 center", (xt(288) + 8, row_y(60)), mix(KICKC, WHITE_ROW, 0.3)),
    ("empty white row (key 60)", (88, row_y(60)), WHITE_ROW), ("empty black row (key 61)", (88, row_y(61)), BLACK_ROW),
    ("bar line at pattern end", (xt(384) + 1, row_y(62)), BAR_LINE),
    ("past end, white row key 60", (312, row_y(60)), mix((0, 0, 0), WHITE_ROW, 0.4)),
    # a stem is velocity * LANE / 128 tall from the lane's bottom: 50 px for A (100), 32 for B (64)
    ("lane stem A inside", (50, 560 - 100 * LANE // 128 + 4), LEAD), ("lane above stem A", (50, 560 - 100 * LANE // 128 - 4), LANE_BG),
    ("lane stem B inside", (114, 560 - 64 * LANE // 128 + 4), LEAD), ("lane above stem B", (114, 560 - 64 * LANE // 128 - 4), LANE_BG),
]
for name, xy, col in probes: print(f"probe {name}: {xy} = #{''.join(f'{v:02X}' for v in col)}")
print(f"probe label boxes x 20..46: C5 row (key 60) y {row_y(60)-4}..{row_y(60)+4} has >= 6 px near #25282D;",
      f"D row (key 62) y {row_y(62)-4}..{row_y(62)+4} has 0; grid {GY}..{560-6-LANE}, lane {LANE_TOP}..560")

if "--write" in sys.argv:
    out = sys.argv[sys.argv.index("--write") + 1]; os.makedirs(out, exist_ok=True)
    for name, dd in written.items():
        with open(os.path.join(out, name), "w") as f: f.write(J(dd) + "\n")
    data = struct.pack("<2000f", *HOLD)  # mono 32-bit float, 44100 Hz
    fmt = struct.pack("<HHIIHH", 3, 1, SR, SR * 4, 4, 32)
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack("<I", len(data)) + data
    with open(os.path.join(out, "hold2000.wav"), "wb") as f: f.write(b"RIFF" + struct.pack("<I", len(body)) + body)
    print("wrote", len(written) + 1, "fixtures to", out)
