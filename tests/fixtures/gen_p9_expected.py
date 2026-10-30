"""Known answers for MIDI input (record, controller links), SMF import/export, clip
time-stretch, and two effects ported from Airwindows (MIT) with null tests.

Expected values come from the rules written out below, never from engine code. Every input file is authored here: the .mid bytes
from an event list, the WAVs from formulas, the projects from the dicts below.

Run:
  python3 -I gen_p9_expected.py                       print the known answers
  python3 -I gen_p9_expected.py --write DIR           also write the p9 inputs, the p9_*.json
      projects and p9_expected.json (every expected value the tests read) into DIR
  python3 -I gen_p9_expected.py --frequency WAV FROM TO
      print the frequency of WAV's left channel over frames FROM..TO by rising zero crossings,
      linearly interpolated between frames (the stretch tests measure their renders with it; a
      measuring rule, not the read's)
  python3 -I gen_p9_expected.py --reference CHECKOUT --out DIR
      build the original Airwindows C++ (CHECKOUT = a clone of github.com/airwindows/airwindows at
      PINNED_COMMIT) in a temp dir OUTSIDE the repo against our own interface stub, render the
      reference WAVs into DIR/ref/ and write DIR/ref/provenance.json. Needs g++."""
import hashlib, importlib.util, json, os, shutil, struct, subprocess, sys, tempfile

def rules():
    """gen_p11_expected.py beside this script, whose `hermite` is the pitched read's rule."""
    sys.dont_write_bytecode = True                   # no __pycache__ beside the fixtures
    here = os.path.dirname(os.path.abspath(__file__))
    spec = importlib.util.spec_from_file_location("gen_p11_expected", os.path.join(here, "gen_p11_expected.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
hermite = rules().hermite

PPQ = 96; STEP = 24; BAR = 384; SR = 44100; MBPM = 120000; BUF = 512
PINNED_COMMIT = "e68f75154ed0520fc5757fec9e7792dd1747e5ee"   # airwindows HEAD, 2026-10-20

def f32(x): return struct.unpack("<f", struct.pack("<f", x))[0]
def jf(x):
    """An f32 written the way the project file writes it: shortest digits that read back."""
    x = f32(x)
    for p in range(1, 10):
        s = "%.*g" % (p, x)
        if f32(float(s)) == x: break
    return s if ("." in s or "e" in s) else s + ".0"
def t2s(t, sr=SR, mbpm=MBPM): return t * 60 * sr * 1000 // (mbpm * PPQ)            # the one tick-to-sample rule
def tick_at(s, sr=SR, mbpm=MBPM):
    """Playhead tick of absolute sample s: one before the first tick at or after s + 1."""
    return -(-(s + 1) * mbpm * PPQ // (60 * sr * 1000)) - 1
def snap_near(t, u): return (t + u // 2) // u * u                                   # half up
def rescale(t, ppq): return (t * PPQ + ppq // 2) // ppq                             # half up, as the project-file import rounds
def vel(v): return v / 128                                                          # MIDI byte -> stored velocity
def note(tick, length, v, key):
    n = {"channel": 1, "tick": tick, "length": length, "velocity": jf(vel(v)) if isinstance(v, int) else jf(v)}
    if key != 60: n["key"] = key
    return n
def show_notes(ns): return "[" + ",".join(
    '{"channel":1,"tick":%d,"length":%d,"velocity":%s%s}' % (n["tick"], n["length"], n["velocity"],
    "" if "key" not in n else ',"key":%d' % n["key"]) for n in ns) + "]"

# ---------- SMF bytes (the same running-status rule the writer crate uses) ----------
def varlen(n):
    out = [n & 0x7F]; n >>= 7
    while n: out.append(0x80 | (n & 0x7F)); n >>= 7
    return bytes(reversed(out))
def mtrk(events):
    out = b""; last = 0; rs = None
    for t, msg in events:
        out += varlen(t - last); last = t
        if msg[0] == 0xFF: rs = None; out += bytes(msg)                 # meta cancels running status
        elif msg[0] == rs: out += bytes(msg[1:])                         # same status: omit it
        else: rs = msg[0]; out += bytes(msg)
    return b"MTrk" + struct.pack(">I", len(out)) + out
def smf(fmt, division, tracks): return b"MThd" + struct.pack(">IHHH", 6, fmt, len(tracks), division) + b"".join(tracks)
def meta(kind, data): return [0xFF, kind] + list(varlen(len(data))) + list(data)
TEMPO_120 = meta(0x51, [0x07, 0xA1, 0x20]); TS_44 = meta(0x58, [4, 2, 24, 8]); EOT = meta(0x2F, [])

IMPORT_PPQ = 480
IMPORT_T0 = [(0, TEMPO_120), (0, TS_44), (0, meta(0x03, b"conductor")), (0, EOT)]
IMPORT_T1 = [(0, meta(0x03, b"keys")), (0, [0xC0, 5]), (0, [0x90, 60, 100]),
             (77, [0x90, 64, 64]),                    # running status in the file
             (240, [0xB0, 7, 90]), (300, [0xE0, 0, 64]),
             (480, [0x80, 60, 64]), (557, [0x90, 64, 0]),  # velocity-0 note-on is a note-off
             (960, [0x99, 36, 127]), (960, [0x90, 36, 90]),  # same key and tick once merged: dropped
             (1000, [0x80, 36, 0]), (1080, [0x99, 36, 0]),
             (1300, [0x90, 67, 80]),                  # never closed: ends at the end of its track
             (2400, EOT)]

def imported(tracks, ppq):
    """Notes from authored events: pairs each note-off with the oldest open note of its
    (MIDI channel, key), merges every MIDI channel into one rack channel, keeps the first note
    where two land on the same (tick, key), closes open notes at their track's end."""
    notes, dropped, unclosed, ignored = [], 0, 0, 0
    for ti, events in enumerate(tracks):
        open_ = {}; end = events[-1][0]
        for idx, (t, m) in enumerate(events):
            st = m[0]
            if st == 0xFF: continue
            kind, chan = st & 0xF0, st & 0x0F
            if kind == 0x90 and m[2] > 0: open_.setdefault((chan, m[1]), []).append((t, m[2], (ti, idx)))
            elif kind in (0x80, 0x90):
                q = open_.get((chan, m[1]))
                if q: s, v, at = q.pop(0); notes.append((at, s, t - s, v, m[1]))
            else: ignored += 1
        for (chan, key), q in open_.items():
            for s, v, at in q: notes.append((at, s, end - s, v, key)); unclosed += 1
    out, seen = [], set()
    for _, s, length, v, key in sorted(notes):                            # note-on order: track, then event
        tick = rescale(s, ppq)
        if (tick, key) in seen: dropped += 1; continue
        seen.add((tick, key)); out.append(note(tick, rescale(length, ppq), v, key))
    out.sort(key=lambda n: (n["tick"], n.get("key", 60)))
    return out, dropped, unclosed, ignored

def exported(notes, name=b"Pattern 1"):
    ev = []
    for n in notes:
        key = n.get("key", 60); length = n["length"] if n["length"] > 0 else STEP
        vb = min(127, max(1, int(float(n["velocity"]) * 128 + 0.5)))
        ev.append((n["tick"], 1, key, [0x90, key, vb])); ev.append((n["tick"] + length, 0, key, [0x80, key, 64]))
    ev.sort(key=lambda e: (e[0], e[1], e[2]))                              # offs before ons, then key
    events = [(0, TEMPO_120), (0, TS_44), (0, meta(0x03, name))] + [(t, m) for t, _, _, m in ev]
    events.append((events[-1][0], EOT))
    return smf(0, PPQ, [mtrk(events)])

# ---------- live MIDI: record and controller links ----------
REC_EVENTS = [(87, [0x90, 60, 100]), (120, [0x80, 60, 0]), (150, [0x90, 64, 127]),
              (170, [0x90, 64, 0]), (171, [0x90, 67, 64]), (180, [0x80, 67, 64])]
def record(quantize, bars=2):
    on, notes, plays = {}, [], []
    for call, m in REC_EVENTS:
        s = call * BUF; T = tick_at(s)
        if m[0] == 0x90 and m[2] > 0: on[m[1]] = (T, m[2]); plays.append((s, vel(m[2])))
        else:
            T0, v = on.pop(m[1]); pt = T0 % BAR
            if quantize: pt = snap_near(pt, STEP) % BAR                       # wraps at the pattern end
            # By gen_p17_expected.py's rule: a note past the pattern's end is cut there,
            # the rest from tick 0, no longer than its own start.
            first = min(T - T0, BAR - pt)
            notes.append((pt, first, v, m[1], call))
            rest = min(T - T0 - first, pt)
            if rest > 0: notes.append((0, rest, v, m[1], call))
    end = t2s(bars * BAR)
    for pt, length, v, key, call in notes:                                    # heard from the next call on
        for p in range(bars + 1):
            s = t2s(p * BAR + pt)
            if (call + 1) * BUF <= s < end: plays.append((s, vel(v)))
    doc = sorted((note(pt, length, v, key) for pt, length, v, key, _ in notes), key=lambda n: (n["tick"], n.get("key", 60)))
    return doc, sorted(plays)

CC_EVENTS = [(10, [0xB0, 7, 100]), (12, [0xB1, 10, 0]), (14, [0xB0, 10, 127]), (16, [0xB0, 10, 64])]
def cc_value(lo, hi, v): return f32(lo + (hi - lo) * v / 127)

# ---------- clip stretch ----------
def clicks(): return [1.0 if i % 441 == 0 else 0.0 for i in range(22050)]
def resample(src, T):
    """The read at the exact position j*S/T: frame i and f = r/T (f64), the sampler's 4-point
    Hermite (gen_p11_expected.py's `hermite`) of the frames i-1..i+2, those before the first and
    past the end 0; on a frame (r = 0) the frame itself."""
    S = len(src); out = []
    at = lambda k: src[k] if 0 <= k < S else 0.0
    for j in range(T):
        i, r = divmod(j * S, T)
        out.append(at(i) if r == 0 else hermite(at(i - 1), at(i), at(i + 1), at(i + 2), r / T))
    return out

# ---------- effect inputs ----------
FX_FRAMES = 16384
def xorshift(x):
    x ^= (x << 13) & 0xFFFFFFFF; x ^= x >> 17; x ^= (x << 5) & 0xFFFFFFFF; return x
def fx_input():
    import math
    L = [0.0] * FX_FRAMES; R = [0.0] * FX_FRAMES; L[0] = R[0] = 1.0; x = 2463534242
    for i in range(64, 2112):
        x = xorshift(x); L[i] = f32(x / 2**32 * 1.6 - 0.8); x = xorshift(x); R[i] = f32(x / 2**32 * 1.6 - 0.8)
    for i in range(3000, 10000): s = 0.5 * math.sin(2 * math.pi * 1000 * i / SR); L[i] = f32(s); R[i] = f32(0.9 * s)
    for i in range(10500, 12500): s = 3.0 * math.sin(2 * math.pi * 220 * i / SR); L[i] = f32(s); R[i] = f32(-s)
    return L, R
def wav(path, channels, sr=SR):
    n = len(channels[0]); data = struct.pack("<%df" % (n * len(channels)), *[c[i] for i in range(n) for c in channels])
    fmt = struct.pack("<HHIIHH", 3, len(channels), sr, sr * 4 * len(channels), 4 * len(channels), 32)
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack("<I", len(data)) + data
    with open(path, "wb") as f: f.write(b"RIFF" + struct.pack("<I", len(body)) + body)
def sha(b): return hashlib.sha256(b).hexdigest()

# ---------- reference renders of the original C++ ----------
STUB = r"""// Interface stub (Dawsome's own words) so the MIT Airwindows sources build without any vendor SDK.
#ifndef __audioeffect__
#define __audioeffect__
#include <cstdint>
#include <cstring>
#include <cstdio>
#include <cstdlib>
typedef int32_t VstInt32;
typedef void* audioMasterCallback;
enum VstPlugCategory { kPlugCategEffect = 1 };
enum { kVstMaxProgNameLen = 24, kVstMaxParamStrLen = 8, kVstMaxVendorStrLen = 64, kVstMaxProductStrLen = 64, kVstMaxEffectNameLen = 32 };
inline void vst_strncpy(char* d, const char* s, int n) { std::strncpy(d, s, n); d[n] = 0; }
inline void float2string(float v, char* t, int n) { std::snprintf(t, n + 1, "%f", v); }
inline void dB2string(float v, char* t, int n) { std::snprintf(t, n + 1, "%f", v); }
inline void int2string(int v, char* t, int n) { std::snprintf(t, n + 1, "%d", v); }
class AudioEffect { public: virtual ~AudioEffect() {} };
class AudioEffectX : public AudioEffect {
public:
  static double hostRate;
  AudioEffectX(audioMasterCallback, int, int) {}
  void setNumInputs(int) {} void setNumOutputs(int) {} void setUniqueID(long) {}
  void canProcessReplacing() {} void canDoubleReplacing() {} void programsAreChunks(bool = true) {}
  double getSampleRate() { return hostRate; }
};
#endif
"""
HARNESS = r"""#include NAME_H
#include <vector>
#include <fstream>
double AudioEffectX::hostRate = 44100.0;
// harness <in.f32 interleaved stereo> <out.f32> <rate> <block> <A> <B> <C> <D>
int main(int argc, char** argv) {
  AudioEffectX::hostRate = atof(argv[3]); int block = atoi(argv[4]);
  std::ifstream f(argv[1], std::ios::binary); f.seekg(0, std::ios::end); size_t n = f.tellg(); f.seekg(0);
  std::vector<float> buf(n / 4); f.read((char*)buf.data(), n); size_t frames = buf.size() / 2;
  srand(1);                                    // the dither seeds come from the first two rand() calls
  CLS fx(nullptr);
  for (int i = 5; i < argc; ++i) fx.setParameter(i - 5, (float)atof(argv[i]));
  std::vector<float> L(frames), R(frames), oL(frames), oR(frames);
  for (size_t i = 0; i < frames; ++i) { L[i] = buf[2 * i]; R[i] = buf[2 * i + 1]; }
  for (size_t at = 0; at < frames; at += block) {
    int k = (int)std::min<size_t>(block, frames - at);
    float* ins[2] = {&L[at], &R[at]}; float* outs[2] = {&oL[at], &oR[at]};
    fx.processReplacing(ins, outs, k);
  }
  std::ofstream o(argv[2], std::ios::binary);
  for (size_t i = 0; i < frames; ++i) { o.write((char*)&oL[i], 4); o.write((char*)&oR[i], 4); }
}
"""
REF_SETS = [("pressure4", "Pressure4", "a", [0.6, 0.4, 0.7, 0.9]), ("pressure4", "Pressure4", "default", [0.0, 0.2, 1.0, 1.0]),
            ("density", "Density", "a", [0.6, 0.3, 0.8, 0.7]), ("density", "Density", "hot", [1.0, 0.0, 1.0, 1.0])]
CLI_TEMPO = 480000; CLI_FRAMES = 22050                                  # 1 bar at 480 BPM, 44.1 kHz

def reference(checkout, out):
    head = subprocess.run(["git", "-C", checkout, "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    if head != PINNED_COMMIT: sys.exit("checkout is at %s, expected %s" % (head, PINNED_COMMIT))
    refdir = os.path.join(out, "ref"); os.makedirs(refdir, exist_ok=True)
    work = tempfile.mkdtemp(prefix="p9-airwindows-")                    # outside the repo
    with open(os.path.join(work, "audioeffectx.h"), "w") as f: f.write(STUB)
    with open(os.path.join(work, "harness.cpp"), "w") as f: f.write(HARNESS)
    L, R = fx_input()
    with open(os.path.join(work, "in.f32"), "wb") as f: f.write(struct.pack("<%df" % (2 * FX_FRAMES), *[v for p in zip(L, R) for v in p]))
    mono = L + [0.0] * (CLI_FRAMES - FX_FRAMES)
    with open(os.path.join(work, "mono.f32"), "wb") as f: f.write(struct.pack("<%df" % (2 * CLI_FRAMES), *[v for v in mono for _ in (0, 1)]))
    gxx = subprocess.run(["g++", "--version"], capture_output=True, text=True).stdout.splitlines()[0]
    flags = ["-std=c++14", "-O2", "-D__cdecl=", "-w"]
    prov = {"upstream": "https://github.com/airwindows/airwindows", "commit": PINNED_COMMIT, "licence": "MIT",
            "compiler": gxx, "flags": " ".join(flags), "harness_sha256": sha(HARNESS.encode()), "stub_sha256": sha(STUB.encode()),
            "seeds": "srand(1) before construction (glibc): fpdL 2490677913, fpdR 3448036410", "block": BUF,
            "input": "fx_input() in this script, %d frames stereo f32" % FX_FRAMES, "renders": []}
    built = {}
    for key, cls, _, _ in REF_SETS:
        if cls in built: continue
        src = os.path.join(checkout, "plugins/LinuxVST/src", cls)
        files = [cls + ".h", cls + ".cpp", cls + "Proc.cpp"]
        for fn in files: shutil.copy(os.path.join(src, fn), work)
        prov[cls] = {fn: sha(open(os.path.join(src, fn), "rb").read()) for fn in files}
        exe = os.path.join(work, "h_" + cls)
        subprocess.run(["g++"] + flags + ["-I" + work, '-DNAME_H="%s.h"' % cls, "-DCLS=" + cls,
                        os.path.join(work, "harness.cpp"), os.path.join(work, cls + ".cpp"), os.path.join(work, cls + "Proc.cpp"), "-o", exe], check=True)
        built[cls] = exe
    jobs = [(k, c, s, p, rate, "in.f32") for k, c, s, p in REF_SETS for rate in (44100, 48000)]
    jobs.append(("density", "Density", "cli", [0.6, 0.3, 0.8, 0.7], 44100, "mono.f32"))
    for key, cls, sset, params, rate, inp in jobs:
        raw = os.path.join(work, "out.f32")
        subprocess.run([built[cls], os.path.join(work, inp), raw, str(rate), str(BUF)] + [repr(p) for p in params], check=True)
        b = open(raw, "rb").read(); v = struct.unpack("<%df" % (len(b) // 4), b)
        name = "%s_%s_%dk.wav" % (key, sset, rate // 1000)
        wav(os.path.join(refdir, name), [list(v[0::2]), list(v[1::2])], rate)
        prov["renders"].append({"file": name, "class": cls, "rate": rate, "params_ABCD": params, "input": inp,
                                "sha256": sha(open(os.path.join(refdir, name), "rb").read())})
    with open(os.path.join(refdir, "provenance.json"), "w") as f: json.dump(prov, f, indent=1)
    shutil.rmtree(work)
    print("reference: %d renders into %s (commit %s, %s)" % (len(prov["renders"]), refdir, PINNED_COMMIT[:12], gxx))

def write_inputs(out):
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, "p9_import.mid"), "wb") as f: f.write(smf(1, IMPORT_PPQ, [mtrk(IMPORT_T0), mtrk(IMPORT_T1)]))
    ev = lambda es: json.dumps({"buffer": BUF, "events": [{"call": c, "bytes": m} for c, m in es]}, separators=(",", ":"))
    with open(os.path.join(out, "p9_record_events.json"), "w") as f: f.write(ev(REC_EVENTS))
    with open(os.path.join(out, "p9_cc_events.json"), "w") as f: f.write(ev(CC_EVENTS))
    wav(os.path.join(out, "p9_clicks.wav"), [clicks()])
    import math
    wav(os.path.join(out, "p9_tone440.wav"), [[f32(0.5 * math.sin(2 * math.pi * 440 * i / SR)) for i in range(SR)]])
    L, R = fx_input(); wav(os.path.join(out, "p9_fx_in.wav"), [L, R]); wav(os.path.join(out, "p9_fx_in_mono.wav"), [L])

# ---------- the p9 projects (the landed format: canonical field order, defaults left out) ----------
def J(d): return json.dumps(d, separators=(",", ":"))
def tracks(n=8): return [{"id": i, "name": "Track %d" % i, "mute": False} for i in range(1, n + 1)]
def chan(i, name, kind, sample, insert=0):
    c = {"id": i, "name": name, "kind": kind, "sample": sample, "volume": 1.0, "pan": 0.0, "mute": False}
    if insert: c["insert"] = insert
    return c
def aclip(i, tr, start, length, ch, stretch=None):
    c = {"id": i, "track": tr, "start": start, "length": length, "kind": "audio", "channel": ch}
    if stretch:
        mode, frames, pitch = stretch
        c["stretch"] = {"mode": mode, "frames": frames}
        if pitch: c["stretch"]["pitch"] = pitch
    return c
def pattern(notes, bars=1): return [{"id": 1, "name": "Pattern 1", "length_bars": bars, "notes": notes}]
def insert(i, effects=()):
    return {"id": i, "name": "Master" if i == 0 else "Insert %d" % i, "fader": 0.8, "pan": 0.0, "mute": False,
            "routes": [] if i == 0 else [{"to": 0, "amount": 1.0}], "effects": list(effects)}
def project(channels, patterns, clips=None, mbpm=MBPM, mixer=None, links=None):
    d = {"format_version": 2, "sample_rate": SR, "tempo_mbpm": mbpm, "ppq": PPQ, "next_channel_id": len(channels) + 1,
         "channels": channels, "patterns": patterns}
    if clips: d["playlist"] = {"next_track_id": 9, "next_clip_id": len(clips) + 1, "tracks": tracks(), "clips": clips}
    if mixer: d["mixer"] = mixer
    if links: d["midi_links"] = links
    return d
KEYS = [chan(1, "keys", "sampler", "impulse.wav")]
DENSITY_A = {"density": 0.6, "highpass": 0.3, "mix": 0.7, "output": 0.8}           # A B D C, by key
FIX = {
    "p9_import_base": project(KEYS, pattern([{"channel": 1, "tick": 0, "length": 0, "velocity": 1.0}])),
    "p9_record": project(KEYS, pattern([])),
    # cc7 on any MIDI channel -> insert 1's fader; cc10 on MIDI channel 1 (status B0, index 0) ->
    # channel 1's pan. Listed by target, the order the project writes them in.
    "p9_cc": project(KEYS, pattern([]), links=[{"cc": 10, "channel": 0, "target": {"channel_pan": 1}},
                                             {"cc": 7, "channel": None, "target": {"insert_fader": 1}}]),
    # One bar long, so --bars 1 shows the stretched file once, its silent tail after it.
    "p9_resample": project([chan(1, "clicks", "audio", "p9_clicks.wav")], pattern([]),
                           [aclip(1, 1, 0, BAR, 1, ("resample", 44100, 0))]),
    "p9_resample15": project([chan(1, "clicks", "audio", "p9_clicks.wav")], pattern([]),
                             [aclip(1, 1, 0, BAR, 1, ("resample", 33075, 0))]),
    "p9_stretch": project([chan(1, "tone", "audio", "p9_tone440.wav")], pattern([]),
                          [aclip(1, 1, 0, BAR, 1, ("stretch", 66150, 0))]),
    "p9_stretch_pitch": project([chan(1, "tone", "audio", "p9_tone440.wav")], pattern([]),
                                [aclip(1, 1, 0, BAR, 1, ("stretch", 44100, 7))]),
    # The playlist probe: the stretched clip on track 1, the same file unstretched on track 2.
    # Four bars long: at the playlist's default 64 px a bar, a one-bar clip's title strip has no
    # room after the file's name for the stretch's words.
    "p9_stretch_ui": project([chan(1, "tone", "audio", "p9_tone440.wav")], pattern([]),
                             [aclip(1, 1, 0, 4 * BAR, 1, ("stretch", 66150, 0)), aclip(2, 2, 0, 4 * BAR, 1)]),
    # 480 BPM, so one bar is 22050 frames: the mono input through insert 1's Density.
    "p9_density_cli": project([chan(1, "fx", "sampler", "p9_fx_in_mono.wav", insert=1)],
                              pattern([{"channel": 1, "tick": 0, "length": 0, "velocity": 1.0}]), mbpm=CLI_TEMPO,
                              mixer={"next_effect_id": 2, "solo": [], "inserts": [insert(0)]
                                     + [insert(1, [{"id": 1, "slot": 0, "processor": {"id": "dawsome.density", "params": DENSITY_A}}])]
                                     + [insert(i) for i in range(2, 17)]}),
}

def expected():
    """Every value the MIDI, stretch and effect tests read, as p9_expected.json."""
    notes, dropped, unclosed, ignored = imported([IMPORT_T0, IMPORT_T1], IMPORT_PPQ)
    bars = max(1, max(-(-(n["tick"] + max(n["length"], 1)) // BAR) for n in notes))
    ex = exported(notes)
    out = {"import": {"notes": [dict(n, velocity=float(n["velocity"])) for n in notes], "length_bars": bars,
                      "dropped": dropped, "unclosed": unclosed, "ignored": ignored},
           "export": {"hex": ex.hex(), "bytes": len(ex), "sha256_16": sha(ex)[:16]}}
    for q, name in ((True, "on"), (False, "off")):
        doc, plays = record(q)
        out["record_" + name] = {"frames": t2s(2 * BAR), "notes": [dict(n, velocity=float(n["velocity"])) for n in doc],
                                 "nonzero": [[s, vel_value] for s, vel_value in plays]}
    out["cc"] = {"fader": cc_value(0.0, 1.0, 100), "pan": cc_value(-1.0, 1.0, 64), "undo": 2}
    src = clicks()
    for T, key in ((44100, "resample"), (33075, "resample15")):
        nz = [[j, v] for j, v in enumerate(resample(src, T)) if v != 0.0]
        out[key] = {"frames": t2s(BAR), "stretched": T, "nonzero": nz}
    out["stretch"] = {"frames": t2s(BAR), "stretched": 66150, "hz": 440.0, "hz_tolerance": 1.0}
    out["stretch_pitch"] = {"frames": t2s(BAR), "stretched": 44100, "hz": 440 * 2 ** (7 / 12), "cents_tolerance": 10.0}
    out["density_cli"] = {"frames": CLI_FRAMES, "reference": "ref/density_cli_44k.wav"}
    return out

def write_projects(out):
    for name, doc in FIX.items():
        with open(os.path.join(out, name + ".json"), "w") as f: f.write(J(doc) + "\n")
    with open(os.path.join(out, "p9_expected.json"), "w") as f: f.write(J(expected()) + "\n")

# ---------- measuring a render ----------
def read_wav_left(path):
    """The left (first) channel of a 32-bit float WAV, and its rate."""
    with open(path, "rb") as f: b = f.read()
    at, channels, rate, data = 12, 1, SR, None
    while at + 8 <= len(b):
        tag, size = b[at:at + 4], struct.unpack("<I", b[at + 4:at + 8])[0]
        body = b[at + 8:at + 8 + size]
        if tag == b"fmt ":
            fmt, channels, rate = struct.unpack("<HHI", body[:8])
            if fmt == 0xFFFE: fmt = struct.unpack("<H", body[24:26])[0]       # extensible: the subformat's tag
            if fmt != 3 or struct.unpack("<H", body[14:16])[0] != 32: sys.exit("%s is not 32-bit float" % path)
        elif tag == b"data": data = body
        at += 8 + size + (size & 1)
    values = struct.unpack("<%df" % (len(data) // 4), data)
    return list(values[0::channels]), rate
def frequency(x, rate, lo, hi):
    """Rising zero crossings in lo..hi, each placed between its frames by linear interpolation:
    (crossings - 1) periods over the time from the first to the last."""
    first, last, count = None, None, 0
    for i in range(lo, hi - 1):
        if x[i] < 0.0 <= x[i + 1]:
            t = i + (-x[i]) / (x[i + 1] - x[i])
            if first is None: first = t
            else: count += 1
            last = t
    if first is None or count == 0: sys.exit("no full period between frames %d and %d" % (lo, hi))
    return count * rate / (last - first)

def main():
    a = sys.argv[1:]
    if "--reference" in a: reference(a[a.index("--reference") + 1], a[a.index("--out") + 1]); return
    if "--frequency" in a:
        i = a.index("--frequency"); x, rate = read_wav_left(a[i + 1])
        print("%.6f" % frequency(x, rate, int(a[i + 2]), int(a[i + 3]))); return
    if "--write" in a:
        write_inputs(a[a.index("--write") + 1]); write_projects(a[a.index("--write") + 1])
    mid = smf(1, IMPORT_PPQ, [mtrk(IMPORT_T0), mtrk(IMPORT_T1)])
    notes, dropped, unclosed, ignored = imported([IMPORT_T0, IMPORT_T1], IMPORT_PPQ)
    bars = max(1, max(-(-(n["tick"] + max(n["length"], 1)) // BAR) for n in notes))
    print("(1) import p9_import.mid (%d bytes, sha256 %s): notes %s" % (len(mid), sha(mid)[:16], show_notes(notes)))
    print("    length_bars 1 -> %d; report: %d overlapping note dropped, %d note closed at track end, %d other events ignored, tempo 120 BPM not applied"
          % (bars, dropped, unclosed, ignored))
    ex = exported(notes)
    print("(2) export pattern 1 / channel 1: %d bytes sha256 %s\n    hex %s" % (len(ex), sha(ex)[:16], ex.hex()))
    # round trip: decode the exported track's events back through the same pairing rule
    ev = []; t = 0; data = ex[22:]; i = 0; rs = None
    while i < len(data):
        d = 0
        while True:
            c = data[i]; i += 1; d = (d << 7) | (c & 0x7F)
            if c < 0x80: break
        t += d; st = data[i]
        if st == 0xFF: ln = data[i + 2]; ev.append((t, list(data[i:i + 3 + ln]))); i += 3 + ln; rs = None; continue
        if st & 0x80: rs = st; i += 1
        ev.append((t, [rs, data[i], data[i + 1]])); i += 2
    back = imported([ev], PPQ)[0]
    print("    re-import == import: %s; a step (length 0) exports as length %d" % (back == notes, STEP))
    for q in (True, False):
        doc, plays = record(q)
        print("(3) record quantize %s: notes %s" % ("on " if q else "off", show_notes(doc)))
        print("    --play-to 2 bars (%d frames), nonzero frames (both sides): %s" % (t2s(2 * BAR), " ".join("%d:%s" % (s, jf(v)) for s, v in plays)))
    for call, m in REC_EVENTS: print("    call %3d sample %6d tick %3d  %s" % (call, call * BUF, tick_at(call * BUF), m))
    print("(4) links cc7 any -> insert 1 fader [0,1]; cc10 midi ch 1 -> channel 1 pan [-1,1]")
    print("    fader <- cc7=100: %s; cc10 on midi ch 2 ignored; pan <- 127: %s then 64: %s; two undo steps"
          % (jf(cc_value(0.0, 1.0, 100)), jf(cc_value(-1.0, 1.0, 127)), jf(cc_value(-1.0, 1.0, 64))))
    src = clicks()
    for T, ticks in ((44100, 192), (33075, 144)):
        out = resample(src, T); nz = [(j, v) for j, v in enumerate(out) if v != 0.0]
        print("(5) resample clicks 22050 -> %d frames (window %d ticks = %d samples): %d nonzero; first %s; last %s; peaks 1.0 at %s"
              % (T, ticks, t2s(ticks), len(nz), " ".join("%d:%s" % (j, jf(v)) for j, v in nz[:6]),
                 " ".join("%d:%s" % (j, jf(v)) for j, v in nz[-3:]),
                 sorted({j for j, v in nz if v == 1.0})[:4]))
    print("(6) stretch tone440 44100 -> 66150 frames (288 ticks = %d samples), pitch 0: 440 Hz +-1; pitch +7 at 44100: %.3f Hz +-10 cents"
          % (t2s(288), 440 * 2 ** (7 / 12)))
    L, R = fx_input()
    print("(7) effect input: %d frames stereo, sha256 of p9_fx_in.wav data %s; references: run --reference"
          % (FX_FRAMES, sha(struct.pack("<%df" % (2 * FX_FRAMES), *[v for p in zip(L, R) for v in p]))[:16]))

if __name__ == "__main__":
    main()
