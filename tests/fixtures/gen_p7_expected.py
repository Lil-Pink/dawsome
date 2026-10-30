"""Plugin known answers. Stdlib only: python3 -I tests/fixtures/gen_p7_expected.py tests/fixtures
Writes the p7_*.json fixtures into <out_dir> and prints every expected value. Expected values come
from these formulas, never from Dawsome.

The test plugins (crates/test-plugin) are our own code, so their formulas are exact:
  sine instrument: y[S0+n] = f32(gain * velocity * cos(TAU * f * n / sr)) for S0 <= S0+n < note-off,
                   f = 440 * 2**((key-69)/12), n counts samples since the note-on frame; 0 elsewhere.
  gain effect:     y = f32(f64(x) * gain).
  state blob:      b"OTP1" + gain as f64 little-endian (12 bytes), base64 (standard, padded) in JSON.
"""
import base64, json, math, os, struct, sys

OUT = sys.argv[1] if len(sys.argv) > 1 else "."
os.makedirs(OUT, exist_ok=True)

def f32(x):
    return struct.unpack("<f", struct.pack("<f", x))[0]

def t2s(tick, sr, mbpm, ppq=96):
    # the project's one tick->sample rule, exact integer math
    return tick * 60 * sr * 1000 // (mbpm * ppq)

def blob(gain):
    return base64.b64encode(b"OTP1" + struct.pack("<d", gain)).decode()

def sine(n, gain, vel=1.0, key=69, sr=48000):
    f = 440.0 * 2.0 ** ((key - 69) / 12.0)
    return f32(gain * vel * math.cos(math.tau * f * n / sr))

# --- fixtures, in the model's canonical JSON (compact, fields in the order Dawsome writes) -----
def write(name, obj):
    with open(os.path.join(OUT, name), "w") as f:
        f.write(json.dumps(obj, separators=(",", ":")))

def mixer(effects_on_1, next_effect_id):
    inserts = [{"id": 0, "name": "Master", "fader": 0.8, "pan": 0.0, "mute": False, "routes": [], "effects": []}]
    for k in range(1, 17):
        inserts.append({"id": k, "name": f"Insert {k}", "fader": 0.8, "pan": 0.0, "mute": False,
                        "routes": [{"to": 0, "amount": 1.0}], "effects": effects_on_1 if k == 1 else []})
    return {"next_effect_id": next_effect_id, "solo": [], "inserts": inserts}

def project(sr, channels, notes, next_channel_id, mix=None):
    p = {"format_version": 2, "sample_rate": sr, "tempo_mbpm": 120000, "ppq": 96,
         "next_channel_id": next_channel_id, "channels": channels,
         "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": notes}]}
    if mix is not None:
        p["mixer"] = mix
    return p

def plugin_channel(cid, name, pid, state):
    return {"id": cid, "name": name, "kind": "plugin",
            "processor": {"id": pid, "name": name, "state": state},
            "volume": 1.0, "pan": 0.0, "mute": False}

def kick(cid, insert):
    c = {"id": cid, "name": "kick", "kind": "sampler", "sample": "impulse.wav", "volume": 1.0, "pan": 0.0, "mute": False}
    if insert:
        c["insert"] = insert
    return c

def note(channel, tick, length=0, key=None):
    n = {"channel": channel, "tick": tick, "length": length, "velocity": 1.0}
    if key is not None:
        n["key"] = key
    return n

write("p7_sine.json", project(48000, [plugin_channel(1, "Dawsome Test Sine", "clap:org.dawsome.test.sine", blob(0.5))],
                              [note(1, 24, 96, 69)], 2))
# The same sine in song mode: pattern 1 as one clip of a bar on track 1, so an automation clip
# the window adds (on track 2) governs its gain.
sine_song = project(48000, [plugin_channel(1, "Dawsome Test Sine", "clap:org.dawsome.test.sine", blob(0.5))],
                    [note(1, 24, 96, 69)], 2)
sine_song["playlist"] = {"next_track_id": 9, "next_clip_id": 2,
                         "tracks": [{"id": i, "name": f"Track {i}", "mute": False} for i in range(1, 9)],
                         "clips": [{"id": 1, "track": 1, "start": 0, "length": 384, "kind": "pattern", "pattern": 1}]}
write("p7_sine_song.json", sine_song)
missing_fx = [{"id": 1, "slot": 0, "processor": {"id": "clap:org.example.missing-fx", "name": "Missing FX", "state": "BAUG"}}]
write("p7_missing.json", project(44100, [kick(1, 1), plugin_channel(2, "Missing Synth", "clap:org.example.missing", "AAECAw==")],
                                 [note(1, 0), note(2, 48), note(1, 96), note(2, 144), note(1, 192), note(1, 288)], 3,
                                 mixer(missing_fx, 2)))
def gain_fx(gain):
    return [{"id": 1, "slot": 0, "processor": {"id": "clap:org.dawsome.test.gain", "name": "Dawsome Test Gain", "state": blob(gain)}}]
write("p7_fx.json", project(44100, [kick(1, 1)], [note(1, 0)], 2, mixer(gain_fx(0.5), 2)))
write("p7_fx_unity.json", project(44100, [kick(1, 1)], [note(1, 0)], 2, mixer(gain_fx(1.0), 2)))
write("p7_fx_none.json", project(44100, [kick(1, 1)], [note(1, 0)], 2, mixer([], 2)))

def plugin(pid, name, kind, vendor="Dawsome"):
    return {"id": pid, "name": name, "vendor": vendor, "version": "1.0.0", "kind": kind, "gui": "none", "params": 1}
catalogue = {"version": 1, "files": [
    {"file": "/plugins/broken.clap", "size": 1024, "mtime": 1, "status": "failed", "reason": "crashed: signal 6", "plugins": []},
    {"file": "/plugins/example.clap", "size": 2048, "mtime": 1, "status": "ok", "plugins": [
        plugin("org.example.bells", "Bell Keys", "instrument", "Example"),
        plugin("org.example.lead", "Analog Lead", "instrument", "Example")]},
    {"file": "/plugins/dawsome_test_plugin.clap", "size": 4096, "mtime": 1, "status": "ok", "plugins": [
        plugin("org.dawsome.test.gain", "Dawsome Test Gain", "effect"),
        plugin("org.dawsome.test.sine", "Dawsome Test Sine", "instrument")]}]}
# In the order a saved catalogue has: files by path, each file's plugins by id.
catalogue["files"].sort(key=lambda f: f["file"])
for f in catalogue["files"]:
    f["plugins"].sort(key=lambda p: p["id"])
with open(os.path.join(OUT, "p7_catalogue.json"), "w") as f:
    f.write(json.dumps(catalogue, separators=(",", ":")))

# --- expected values ---------------------------------------------------------------------------
SR, BPM = 48000, 120000
s0, s1 = t2s(24, SR, BPM), t2s(120, SR, BPM)
bar = t2s(384, SR, BPM)
print(f"1 sine p7_sine.json 48 kHz 120 BPM: tick 24 -> {s0}, tick 120 -> {s1}, 1 bar = {bar} frames")
print(f"  state gain 0.5 = {blob(0.5)!r}; frames {s0-1}: 0.0  " +
      "  ".join(f"{s0+n}: {sine(n, 0.5):.9g}" for n in (0, 1, 2, 3)))
print(f"  {s0+1200}: {sine(1200, 0.5):.9g} (n=1200, 11 whole cycles)  {s1-1}: {sine(s1-1-s0, 0.5):.9g}  {s1}: 0.0")
vals = [sine(n, 0.5) for n in range(s1 - s0)]
rms = math.sqrt(sum(v * v for v in vals) / len(vals))
print(f"  RMS[{s0},{s1}) = {rms:.7f} ({(s1-s0)*440/SR:g} whole cycles; 0.5/sqrt2 = {0.5/math.sqrt(2):.7f}); peak {max(abs(v) for v in vals):.9g}")

k = 19; at = k * 512
print(f"2 param: 512-frame calls; Param(gain 0.25) sent before call {k} (0-based) -> first affected frame {at}")
print(f"  {at-1}: {sine(at-1-s0, 0.5):.9g} (gain 0.5)  {at}: {sine(at-s0, 0.25):.9g} (gain 0.25)  {at+1}: {sine(at+1-s0, 0.25):.9g}")
print(f"  committed state after release = {blob(0.25)!r}")
print(f"3 state: load {blob(0.5)!r} -> capture -> {blob(0.5)!r}; bytes {(b'OTP1'+struct.pack('<d',0.5)).hex()}")
print(f"3b automation p7_sine_song.json, song mode: the window's automation clip of the sine's Gain (param id 0, "
      f"range 0..1) held at 0.25 over bars 1-4 sets the gain from play start, so {s0-1}: 0.0  " +
      "  ".join(f"{s0+n}: {sine(n, 0.25):.9g}" for n in (0, 1, 2, 3)) + f"  {s0+1200}: {sine(1200, 0.25):.9g}")

SR2 = 44100
imp = [t2s(t, SR2, BPM) for t in (0, 96, 192, 288)]
print(f"4 missing p7_missing.json 44.1 kHz: 1 bar = {t2s(384, SR2, BPM)} frames; nonzero only at {imp} = 1.0 "
      f"(missing synth notes at ticks 48,144 -> samples {t2s(48,SR2,BPM)},{t2s(144,SR2,BPM)} stay 0.0)")
print(f"  missing blobs kept verbatim: {base64.b64encode(bytes([0,1,2,3])).decode()!r} {base64.b64encode(bytes([4,5,6])).decode()!r}")
print("5 scan: catalogue = test-plugin file ok (ids org.dawsome.test.gain effect, org.dawsome.test.sine instrument)"
      " + crash file failed 'crashed: signal 6' (SIGABRT); dawsome scan exits 0")
print(f"6 determinism: p7_sine twice at --block 512 -> cmp equal; --block 1,7,64,4096 equal too (test plugin is split-invariant)")
print(f"7 effect p7_fx.json 44.1 kHz: impulse -> insert 1 [test gain {blob(0.5)!r}] -> frame 0 = {f32(1.0*0.5)}, rest 0.0; "
      f"gain {blob(1.0)!r} renders cmp-equal to no effect")
ins = sorted((p["name"] for f in catalogue["files"] for p in f["plugins"] if p["kind"] == "instrument"))
failed = [(os.path.basename(f["file"]), f["reason"]) for f in catalogue["files"] if f["status"] == "failed"]
print("8 picker p7_catalogue.json, instruments: rows " + " | ".join(ins) +
      f" | Could not load ({len(failed)}) | " + " | ".join(f"{name}: {reason}" for name, reason in failed) +
      f"; {len(ins)} row bands")
