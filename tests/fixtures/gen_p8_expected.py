"""Known answers for audio clips, recording, automation and the browser.

Expected values come from these rules, never from engine code:
  tick->sample  s(t) = floor(t*60*sr*1000 / (mbpm*96))            (the one integer rule)
  audio clip    song sample n in [s(start), s(start+length)) plays file frame frame_offset + n - s(start);
                frames past the file end are 0; value = frame * ((gain * channel_gain) * pan_side)
  default clip length L = min{L >= 1 : s(start+L) - s(start) >= frames}
  automation    x = (n - S_i)/(S_{i+1} - S_i) in f64, v = v_i + (v_{i+1} - v_i)*f(x),
                param = min + v*(max - min) in f64, rounded to f32 once; channel volume range 0..4
                f: linear x | hold 0 | smooth x*x*(3-2x) | exp(k): (e^(6k x)-1)/(e^(6k)-1), |k|<1e-8 -> linear
                before the first point: v_0; after the last point: v_last; outside every window: the
                value at the end of the latest window that ended (hold), else the stored value
  count-in      one bar; at each beat b a 64-frame burst A*(1-n/64)*(+1 if (n//8) even else -1),
                A = 0.25 on beat 1, 0.125 on beats 2-4 (the metronome's click); not recorded
  resampling    every decoded file is converted to the project rate on load, by the
                arithmetic in resample_oracle.py; F2 (impulse.wav, 44.1 kHz, in a 48 kHz project)
                therefore plays a two-frame click instead of a one-frame one
Run: python3 -I tests/fixtures/gen_p8_expected.py [--write tests/fixtures]
--write writes the fixtures, the WAVs, browser/ and p8_expected.json (every expected value the
tests read) into DIR, which must already hold impulse.wav."""
import importlib.util, json, math, os, struct, sys

def oracle():
    """resample_oracle.py beside this script, loaded by its path (-I keeps the directory off sys.path)."""
    here = os.path.dirname(os.path.abspath(__file__))
    spec = importlib.util.spec_from_file_location("resample_oracle", os.path.join(here, "resample_oracle.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

PPQ = 96; BAR = 384; BEAT = 96; STEP = 24
def s(t, sr, mbpm): return t * 60 * sr * 1000 // (mbpm * PPQ)
def f32(x): return struct.unpack("<f", struct.pack("<f", x))[0]
def J(d): return json.dumps(d, separators=(',', ':'))
def default_len(start, frames, sr, mbpm):
    L = 1
    while s(start + L, sr, mbpm) - s(start, sr, mbpm) < frames: L += 1
    return L

def wav_bytes(sr, frames, kind):                       # mono; kind 'f32' or 's16'
    if kind == "f32":
        data = struct.pack("<%df" % len(frames), *frames); fmt = struct.pack("<HHIIHH", 3, 1, sr, sr * 4, 4, 32)
    else:
        data = struct.pack("<%dh" % len(frames), *[round(v * 32768) for v in frames]); fmt = struct.pack("<HHIIHH", 1, 1, sr, sr * 2, 2, 16)
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack("<I", len(data)) + data
    return b"RIFF" + struct.pack("<I", len(body)) + body

# --- generated sounds (our own) ---
RAMP48 = [(k + 1) / 1024 for k in range(1000)]                       # p8_ramp48k.wav, f32, 48 kHz
SAW = [((k % 1000) + 1) / 1024 for k in range(30000)]                # p8_saw.wav, s16, 44.1 kHz (exact: (k%1000+1)*32/32768)
IMP48 = [0.0] * 12000; IMP48[1000] = 0.125; IMP48[4800] = 0.5; IMP48[9600] = 0.25   # p8_imp48k.wav, f32, 48 kHz

def tracks(n=8): return [{"id": i, "name": "Track %d" % i, "mute": False} for i in range(1, n + 1)]
def chan(i, name, kind, sample, vol=1.0): return {"id": i, "name": name, "kind": kind, "sample": sample, "volume": vol, "pan": 0.0, "mute": False}
def aclip(i, tr, start, length, ch, off=0, gain=1.0, muted=False):
    c = {"id": i, "track": tr, "start": start, "length": length, "kind": "audio", "channel": ch}
    if off: c["frame_offset"] = off
    if gain != 1.0: c["gain"] = gain
    if muted: c["muted"] = True
    return c
def pclip(i, tr, start, length, pat): return {"id": i, "track": tr, "start": start, "length": length, "kind": "pattern", "pattern": pat}
def mclip(i, tr, start, length, auto): return {"id": i, "track": tr, "start": start, "length": length, "kind": "automation", "automation": auto}
def pt(tick, value, curve="linear"):
    p = {"tick": tick, "value": value}
    if curve != "linear": p["curve"] = curve
    return p
def project(sr, mbpm, channels, patterns, clips, loop=None, automations=None):
    pl = {"next_track_id": 9, "next_clip_id": len(clips) + 1, "tracks": tracks(), "clips": clips}
    if loop: pl["loop"] = loop
    d = {"format_version": 2, "sample_rate": sr, "tempo_mbpm": mbpm, "ppq": PPQ, "next_channel_id": len(channels) + 1,
         "channels": channels, "patterns": patterns, "playlist": pl}
    if automations: d["automation"] = {"next_id": len(automations) + 1, "list": automations}
    return d
def steps(n, ch=1): return [{"channel": ch, "tick": STEP * k, "length": 0, "velocity": 1.0} for k in range(n)]
EMPTY_P1 = [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": []}]

FIX = {}
# (1) placement: 48 kHz, 140 BPM, a 1000-frame ramp at bar 2
FIX["p8_place"] = project(48000, 140000, [chan(1, "ramp", "audio", "p8_ramp48k.wav")], EMPTY_P1, [aclip(1, 1, 384, 384, 1)])
# (2) trim + gain + mute, 44.1 kHz 120 BPM; song end 288 wraps at s(288)
FIX["p8_trim"] = project(44100, 120000, [chan(1, "saw", "audio", "p8_saw.wav")], EMPTY_P1,
                         [aclip(1, 1, 0, 96, 1, off=100), aclip(2, 2, 192, 96, 1, gain=0.5), aclip(3, 3, 96, 96, 1, muted=True)])
# (3) resample on load: 48 kHz file in a 44.1 kHz project
FIX["p8_resample"] = project(44100, 120000, [chan(1, "imp48", "audio", "p8_imp48k.wav")], EMPTY_P1, [aclip(1, 1, 0, 48, 1)])
# (4) linear channel-volume automation; stored volume 0.5; 16 steps per bar, clip repeats 3 bars
P_STEPS = [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": steps(16)}]
CLICK = [chan(1, "click", "sampler", "impulse.wav", vol=0.5)]
FIX["p8_auto_linear"] = project(44100, 120000, CLICK, P_STEPS, [pclip(1, 1, 0, 1152, 1), mclip(2, 2, 384, 384, 1)],
    automations=[{"id": 1, "target": {"channel_volume": 1}, "points": [pt(0, 0.25), pt(384, 0.0)]}])
# (5) hold, smooth, exp(0.5) over three bars
FIX["p8_auto_curves"] = project(44100, 120000, CLICK, P_STEPS, [pclip(1, 1, 0, 1152, 1), mclip(2, 2, 0, 1152, 1)],
    automations=[{"id": 1, "target": {"channel_volume": 1},
                  "points": [pt(0, 0.25, "hold"), pt(384, 0.125, "smooth"), pt(768, 0.25, {"exp": 0.5}), pt(1152, 0.0)]}])
# (7) streaming == memory: same file twice, overlap, loop 480..864 on, and an offset whose time
# in seconds does not give its frame back in f64 (int(1565/44100*44100) = 1564)
FIX["p8_stream"] = project(44100, 120000, [chan(1, "saw", "audio", "p8_saw.wav")], EMPTY_P1,
    [aclip(1, 1, 0, 384, 1), aclip(2, 2, 96, 192, 1, off=5000), aclip(3, 1, 576, 192, 1), aclip(4, 3, 192, 96, 1, off=1565)],
    loop={"start": 480, "end": 864, "on": True})
# (8) recording loopback
FIX["p8_rec"] = project(44100, 120000, [chan(1, "kick", "sampler", "impulse.wav")],
    [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": [{"channel": 1, "tick": t, "length": 0, "velocity": 1.0} for t in (0, 96, 192, 288)]}],
    [pclip(1, 1, 0, 768, 1)])
# UI probe document: saw clip on track 1 (bars 1-2), automation 1.0 -> 0.0 over bar 1 on track 2
FIX["p8_ui"] = project(44100, 120000, [chan(1, "saw", "audio", "p8_saw.wav"), chan(2, "click", "sampler", "impulse.wav")], EMPTY_P1,
    [aclip(1, 1, 0, 768, 1), mclip(2, 2, 0, 384, 1)],
    automations=[{"id": 1, "target": {"channel_volume": 2}, "points": [pt(0, 1.0), pt(384, 0.0)]}])
# UI probe document for the toolbar: two samples that are not there, with long names, so the
# notice saying so is far longer than the toolbar has room for
FIX["p8_notice"] = project(44100, 120000,
    [chan(1, "pad", "sampler", "samples/an-ambient-pad-recorded-at-the-old-studio-take-three-final.wav"),
     chan(2, "keys", "sampler", "samples/electric-piano-chords-in-a-minor-with-the-tremolo-on.wav")],
    EMPTY_P1, [])
del FIX["p8_notice"]["playlist"]                         # the default playlist is not written

def out(*a): print(*a)
EXPECTED = {}                                            # everything the tests read, as p8_expected.json
def f32list(values): return [f32(v) for v in values]
# ---------------- (1) placement ----------------
sr, mb = 48000, 140000
S384, S768 = s(384, sr, mb), s(768, sr, mb)
out("(1) p8_place --song --bars 2: frames", S768, "| first nonzero frame", S384, "= ramp[0] =", RAMP48[0],
    "| last nonzero", S384 + 999, "=", RAMP48[999], "| frame", S384 - 1, "and", S384 + 1000, "= 0.0")
out("    default length of a 1000-frame file dropped at tick 384:", default_len(384, 1000, sr, mb), "ticks")
EXPECTED["place"] = {"frames": S768, "first": S384, "values": f32list(RAMP48), "default_length": default_len(384, 1000, sr, mb)}
# ---------------- (2) trim / gain / mute ----------------
sr, mb = 44100, 120000
def trim_frame(n):
    W = s(288, sr, mb)
    if n >= W: n = n - W                                     # song end 288 wraps to 0
    if n < s(96, sr, mb): return SAW[100 + n]
    if n < s(192, sr, mb): return 0.0                        # clip 3 muted
    if n < s(288, sr, mb): return f32(SAW[n - s(192, sr, mb)] * f32(0.5 * 1.0))
    return 0.0
picks = [0, 22049, 22050, 44099, 44100, 66149, 66150, 88199]
out("(2) p8_trim --song --bars 1 (88200 frames, wrap at", s(288, sr, mb), "):", ", ".join("%d:%r" % (n, trim_frame(n)) for n in picks))
# Every frame as runs: [first output frame, end, first file frame, gain]; the rest is silent.
W = s(288, sr, mb)
EXPECTED["trim"] = {"frames": s(384, sr, mb), "spots": [[n, trim_frame(n)] for n in picks],
                    "runs": [[p * W, p * W + s(96, sr, mb), 100, 1.0] for p in (0, 1)]
                          + [[p * W + s(192, sr, mb), p * W + s(288, sr, mb), 0, f32(0.5 * 1.0)] for p in (0, 1)
                             if p * W + s(192, sr, mb) < s(384, sr, mb)]}
# ---------------- (3) resample ----------------
n_out = -(-12000 * 44100 // 48000)
pk = [(p * 44100 / 48000, v) for p, v in ((1000, 0.125), (4800, 0.5), (9600, 0.25))]
out("(3) p8_resample: loaded frames", n_out, "| default clip length", default_len(0, n_out, sr, mb),
    "| peaks (exact position, nearest frame):", ", ".join("%.2f->%d" % (p, round(p)) for p, _ in pk),
    "| repeats every", s(48, sr, mb), "frames (song end 48), 8 passes in 88200; peak(4410)/peak(8820) = 2")
EXPECTED["resample"] = {"loaded_frames": n_out, "default_length": default_len(0, n_out, sr, mb),
                        "peaks": [round(p) for p, _ in pk], "pass": s(48, sr, mb), "frames": s(384, sr, mb)}
# ---------------- automation helpers ----------------
def curve_f(c, x):
    if c == "linear": return x
    if c == "hold": return 0.0
    if c == "smooth": return x * x * (3.0 - 2.0 * x)
    k = c["exp"]
    if abs(k) < 1e-8: return x                               # TENSION_EPSILON: the formula is 0/0 near 0
    n = 6.0 * k
    return (math.exp(n * x) - 1.0) / (math.exp(n) - 1.0)
def auto_value(points, clip_start, clip_len, stored, n, sr, mb, lo=0.0, hi=4.0):
    """The value and whether an exponential segment gave it."""
    W0, W1 = s(clip_start, sr, mb), s(clip_start + clip_len, sr, mb)
    if n < W0: return stored, False
    m = min(n, W1)                                           # past the window: hold its end value
    S = [s(clip_start + p["tick"], sr, mb) for p in points]
    curve = "linear"
    if m < S[0]: v = points[0]["value"]
    elif m >= S[-1]: v = points[-1]["value"]
    else:
        i = max(j for j in range(len(S)) if S[j] <= m)
        x = (m - S[i]) / (S[i + 1] - S[i])
        v0, v1 = points[i]["value"], points[i + 1]["value"]
        curve = points[i].get("curve", "linear")
        v = v0 + (v1 - v0) * curve_f(curve, x)
    return f32(lo + v * (hi - lo)), isinstance(curve, dict)
def step_frames(bars): return [s(STEP * k, sr, mb) for k in range(16 * bars)]
for name in ("p8_auto_linear", "p8_auto_curves"):
    d = FIX[name]; a = d["automation"]["list"][0]; c = d["playlist"]["clips"][1]
    vals = [(n,) + auto_value(a["points"], c["start"], c["length"], 0.5, n, sr, mb) for n in step_frames(3)]
    out("(%s) %s --song --bars 3 (264600 frames): nonzero only at steps; (frame:value) for steps 0,8,15,16,17,24,31,32,40,47:"
        % ("4" if "linear" in name else "5", name))
    out("   ", ", ".join("%d:%r" % vals[k][:2] for k in (0, 8, 15, 16, 17, 24, 31, 32, 40, 47)))
    out("    sha256-free check: all 48 step values", [v for _, v, _ in vals])
    # Each step: [frame, value, exact]; values an exponential segment gives are compared within 1e-6.
    EXPECTED[name[3:]] = {"frames": s(3 * BAR, sr, mb), "steps": [[n, v, not e] for n, v, e in vals]}
# ---------------- (6) control-rate grid (unit test) ----------------
S1 = s(96, sr, mb)
grid = [n for n in range(0, S1, 32)] + [S1]
gv = [(n, f32(0.0 + (0.0 + (1.0 - 0.0) * (n - 0) / (S1 - 0)) * (1.0 - 0.0))) for n in grid]
out("(6) delay mix automated 0 -> 1 over tick 0..96 (mix range 0..1): set_param calls", len(gv), "first", gv[:3], "last", gv[-2:])
EXPECTED["grid"] = {"calls": [list(c) for c in gv]}
# ---------------- (7) streaming spot values ----------------
def stream_frame(song_n):
    total = 0.0
    for c in FIX["p8_stream"]["playlist"]["clips"]:
        a, b = s(c["start"], sr, mb), s(c["start"] + c["length"], sr, mb)
        if a <= song_n < b:
            k = c.get("frame_offset", 0) + song_n - a
            total += SAW[k] if k < len(SAW) else 0.0
    return total
def song_pos(o):                                         # output frame -> song sample, loop 480..864 on
    W, R = s(864, sr, mb), s(480, sr, mb)
    while o >= W: o = R + (o - W)
    return o
spots = [0, 22050, 29999, 30000, 44100, 132300, 198450, 220500, 308700]
out("(7) p8_stream --song --bars 4 (", s(4 * BAR, sr, mb), "frames): wraps at", s(864, sr, mb), "->", s(480, sr, mb), "|",
    ", ".join("%d:%r" % (o, stream_frame(song_pos(o))) for o in spots))
EXPECTED["stream"] = {"frames": s(4 * BAR, sr, mb), "spots": [[o, stream_frame(song_pos(o))] for o in spots]}
# ---------------- (8) recording ----------------
CI = s(BAR, sr, mb)
def burst(A): return [A * (1 - n / 64) * (1 if (n // 8) % 2 == 0 else -1) for n in range(64)]
b1 = burst(0.25)
out("(8) p8_rec --play-to --bars 2 --record master: a.wav frames", CI + s(768, sr, mb), "| count-in bursts at",
    [s(BEAT * b, sr, mb) for b in range(4)], "frames 0,8,63 of beat 1:", b1[0], b1[8], b1[63], "beat 2 frame 0:", burst(0.125)[0])
out("    song impulses at", [CI + s(t, sr, mb) for t in (0, 96, 192, 288, 384, 480, 576, 672)],
    "| samples/rec-0001.wav frames", s(768, sr, mb), "== dawsome render --song --bars 2 (cmp)")
new_channel = chan(2, "rec-0001", "audio", "samples/rec-0001.wav")
new_clip = aclip(2, 2, 0, default_len(0, s(768, sr, mb), sr, mb), 2)
out("    new channel", J(new_channel), "new clip", J(new_clip))
EXPECTED["rec"] = {"frames": CI + s(768, sr, mb), "count_in": CI,
                   "bursts": [[s(BEAT * b, sr, mb), f32list(burst(0.25 if b == 0 else 0.125))] for b in range(4)],
                   "impulses": [CI + s(t, sr, mb) for t in (0, 96, 192, 288, 384, 480, 576, 672)],
                   "recorded_frames": s(768, sr, mb), "channel": J(new_channel), "clip": J(new_clip)}
# ---------------- F2: a 44.1 kHz click in a 48 kHz project, resampled on load ----------------
click = oracle().resample([1.0], 44100, 48000)
f2_onsets = [s(STEP * k, 48000, 140000) for k in (0, 4, 8, 12)]
out("F2 f2_tempo_140_48k (1 bar, 82285 frames): impulse.wav loads as", len(click), "frames", click,
    "| onsets", f2_onsets, "| frames", [(n + k, click[k]) for n in f2_onsets for k in range(len(click))])
EXPECTED["f2"] = {"frames": s(BAR, 48000, 140000), "onsets": f2_onsets, "click": click}
# ---------------- fixtures ----------------
if "--write" in sys.argv:
    d = sys.argv[sys.argv.index("--write") + 1]
    assert os.path.exists(os.path.join(d, "impulse.wav")), "DIR must hold impulse.wav"
    for name, kind, sr_, fr in (("p8_ramp48k.wav", "f32", 48000, RAMP48), ("p8_saw.wav", "s16", 44100, SAW), ("p8_imp48k.wav", "f32", 48000, IMP48)):
        with open(os.path.join(d, name), "wb") as f: f.write(wav_bytes(sr_, fr, kind))
    for name, doc in FIX.items():
        with open(os.path.join(d, name + ".json"), "w") as f: f.write(J(doc) + "\n")
    with open(os.path.join(d, "p8_expected.json"), "w") as f: f.write(J(EXPECTED) + "\n")
    b = os.path.join(d, "browser"); os.makedirs(os.path.join(b, "drums"), exist_ok=True)
    for p in (os.path.join(b, "drums", "kick.wav"), os.path.join(b, "tone.wav")):
        with open(p, "wb") as f: f.write(wav_bytes(44100, [1.0], "f32"))
    with open(os.path.join(b, "notes.txt"), "w") as f: f.write("not audio\n")
    out("wrote", len(FIX), "projects, 3 wavs, p8_expected.json and browser/ into", d)
