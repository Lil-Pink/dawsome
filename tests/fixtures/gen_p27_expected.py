"""Known answers for the VST3 host: the scan's catalogue, renders through `dawsome render`
of Dawsome's own VST3 test plugins as a channel and as an effect, sample-accurate parameter
changes, notes with pitch and velocity, the state as `.vstpreset` bytes and its round trip, and the
latency a plugin reports.

Expected values come from the test plugins' formulas and the documented `.vstpreset` layout,
restated here, never from running Dawsome. The test plugins (crates/test-plugin-vst3) are our own
code; their formulas are the CLAP test plugins' (gen_p7_expected.py), so the same frames come out:

  sine      y[S0+n] = f32(gain * velocity * cos(TAU * f * n / sr)) from the note-on frame S0 to the
            frame before its note-off, f = 440 * 2**((key-69)/12), n counted from the note-on, 0
            elsewhere; monophonic (a note-on replaces the note sounding; a note-off ends the note
            whose id it names). The Fail class is the sine failing every `process` after 4 calls.
            The View class (plugin windows, gen_p29_expected.py) is the sine with a controller of
            its own that has an editor: its sound, parameters (5) and state are the sine's.
  gain      y = f32(f64(x) * gain); the Latency class is the gain on the input 64 frames late,
            y[n] = f32(f64(x[n - 64]) * gain), 0 for n < 64, and reports a latency of 64 frames;
            it also reports the transport it was given through six read-only parameters
            (ids 10-15: tempo, beats, bar, playing, samples, clock), each divided by 2**24
            (exact) so that it is a normalized value, and a seventh, id 16 "Wild", that reports
            values out of range on purpose (the host's clamp), so it has 8.
  parameter id 0 "Gain", normalized value = gain, 0..1, default 1.0; a point changes it at its
            sample offset. The sine's controller adds id 9 "Linked" (read-only, 1.0 once its link
            to the component answered) and, for VST3 in the app (gen_p28_expected.py), ids 1
            "Bend", 2 "Wheel" and 3 "Wheel heard", so it has 5; the gain adds 1 "Steps" and 2 "Steps
            heard", so it has 3 (neither changes its sound or state); the Tick class adds id 8
            "Ticks" (read-only).
  state     component: b"OTP1" + gain as f64 little-endian (12 bytes), anything else refused;
            the sine's controller: b"OTC1" + u32 0 little-endian (8 bytes); a class that is its
            own controller writes no controller state (0 bytes).
  preset    the SDK's .vstpreset file (vstpresetfile.cpp): b"VST3", int32 1, the class id as 32
            ASCII characters, int64 the offset of the chunk list; the component's data, then the
            controller's; b"List", int32 the count, then per chunk its id (b"Comp", b"Cont"),
            int64 offset and int64 size; little-endian; standard padded base64 in the project.
  class id  the ASCII of a 16-byte name, so its text is that ASCII in hex (FUID::toString's 32
            uppercase digits) on every platform.

The mixer path of these projects is unity (channel volume 1, centered, faders at 0.8, the
render's master untouched), as gen_p7's known answers show.

Run:
  python3 -I gen_p27_expected.py              print the known answers
  python3 -I gen_p27_expected.py --write DIR  also write the files below into DIR
  python3 -I gen_p27_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes

Files:
  p27_vst3_sine.json      p7_sine with the VST3 sine and its gain-0.5 preset
  p27_vst3_fx.json        p7_fx with the VST3 gain at 0.5 in insert 1 (p27_vst3_fx_unity: 1.0)
  p27_vst3_missing.json   p7_missing with VST3 ids nothing provides
  p27_vst3_auto.json      the sine in song mode, its Gain automated: 0.5 held to tick 48, then 0.25
  p27_vst3_notes.json     A4 velocity 1 on ticks 24-48, then A5 velocity 0.5 on ticks 48-96
  p27_vst3_fail.json      the Fail class at gain 0.5, one note over the whole bar
  p27_vst3_latency.json   p7_fx with the Latency class at 0.5 in insert 1
  p27_catalogue.json      a catalogue holding CLAP and VST3 records, as Dawsome writes one
  p27_expected.json       every expected value the tests read
"""
import base64, hashlib, json, math, os, struct, sys


def f32(x):
    return struct.unpack("<f", struct.pack("<f", x))[0]


def t2s(tick, sr, mbpm, ppq=96):
    # the project's one tick->sample rule, exact integer math
    return tick * 60 * sr * 1000 // (mbpm * ppq)


def J(d):
    return json.dumps(d, separators=(",", ":"))


# --- the test plugins --------------------------------------------------------------------------
def class_id(name):
    """A class id named by 16 ASCII characters: its 16 bytes, and its text."""
    raw = name.encode("ascii")
    assert len(raw) == 16, name
    return raw.hex().upper()


SINE = class_id("DawsomeTestSine1")
CTRL = class_id("DawsomeTestCtrl1")
GAIN = class_id("DawsomeTestGain1")
FAIL = class_id("DawsomeTestFail1")
TICK = class_id("DawsomeTestTick1")
LATE = class_id("DawsomeTestLate1")
VIEW = class_id("DawsomeTestView1")
LATENCY = 64
FAIL_AFTER = 4
TIMER_MS = 100

COMPONENT_MAGIC = b"OTP1"
CONTROLLER_STATE = b"OTC1" + struct.pack("<I", 0)


def component_state(gain):
    return COMPONENT_MAGIC + struct.pack("<d", gain)


def preset(cid, comp, cont=None):
    """The .vstpreset bytes of class `cid` holding the component's `comp` and, when given, the
    controller's `cont`."""
    header_size = 4 + 4 + 32 + 8
    chunks = [(b"Comp", comp)] + ([(b"Cont", cont)] if cont is not None else [])
    data, entries, offset = b"", [], header_size
    for tag, bytes_ in chunks:
        entries.append(tag + struct.pack("<qq", offset, len(bytes_)))
        data += bytes_
        offset += len(bytes_)
    header = b"VST3" + struct.pack("<i", 1) + cid.encode("ascii") + struct.pack("<q", offset)
    return header + data + b"List" + struct.pack("<i", len(entries)) + b"".join(entries)


def b64(raw):
    return base64.b64encode(raw).decode()


def sine_preset(gain, cid=SINE):
    return preset(cid, component_state(gain), CONTROLLER_STATE)


def own_controller_preset(cid, gain):
    """A class that is its own controller: its controller writes nothing."""
    return preset(cid, component_state(gain), b"")


# The spec's precomputed gain-0.5 sine preset (vst3-host-phase1.md, known answer 4): the
# generator's layout must give exactly it.
SPEC_SINE_HALF = ("VlNUMwEAAAA0NDYxNzc3MzZGNkQ2NTU0NjU3Mzc0NTM2OTZFNjUzMUQAAAAAAAAAT1RQMQAAAAAAAOA/T1RDMQ"
                  "AAAABMaXN0AgAAAENvbXAwAAAAAAAAAAwAAAAAAAAAQ29udDwAAAAAAAAACAAAAAAAAAA=")
assert b64(sine_preset(0.5)) == SPEC_SINE_HALF and len(sine_preset(0.5)) == 116


def sine(n, gain, vel=1.0, key=69, sr=48000):
    f = 440.0 * 2.0 ** ((key - 69) / 12.0)
    return f32(gain * vel * math.cos(math.tau * f * n / sr))


# --- fixtures, in the model's canonical JSON (compact, fields in the order Dawsome writes) -----
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
    c = {"id": cid, "name": "kick", "kind": "sampler", "sample": "impulse.wav", "volume": 1.0,
         "pan": 0.0, "mute": False}
    if insert:
        c["insert"] = insert
    return c


def note(channel, tick, length=0, key=None, velocity=1.0):
    n = {"channel": channel, "tick": tick, "length": length, "velocity": velocity}
    if key is not None:
        n["key"] = key
    return n


def vst3(cid):
    return "vst3:" + cid


def effect(cid, name, state):
    return [{"id": 1, "slot": 0, "processor": {"id": vst3(cid), "name": name, "state": state}}]


SINE_HALF = b64(sine_preset(0.5))
FIX = {}
FIX["p27_vst3_sine"] = project(48000, [plugin_channel(1, "Dawsome Test Sine", vst3(SINE), SINE_HALF)],
                               [note(1, 24, 96, 69)], 2)
FIX["p27_vst3_fx"] = project(44100, [kick(1, 1)], [note(1, 0)], 2,
                             mixer(effect(GAIN, "Dawsome Test Gain", b64(own_controller_preset(GAIN, 0.5))), 2))
FIX["p27_vst3_fx_unity"] = project(44100, [kick(1, 1)], [note(1, 0)], 2,
                                   mixer(effect(GAIN, "Dawsome Test Gain", b64(own_controller_preset(GAIN, 1.0))), 2))
MISSING_SYNTH = "ABCDEF0123456789ABCDEF0123456789"
MISSING_FX = "0123456789ABCDEF0123456789ABCDEF"
missing_fx = [{"id": 1, "slot": 0, "processor": {"id": vst3(MISSING_FX), "name": "Missing FX", "state": "BAUG"}}]
FIX["p27_vst3_missing"] = project(44100, [kick(1, 1), plugin_channel(2, "Missing Synth", vst3(MISSING_SYNTH), "AAECAw==")],
                                  [note(1, 0), note(2, 48), note(1, 96), note(2, 144), note(1, 192), note(1, 288)], 3,
                                  mixer(missing_fx, 2))
auto = project(48000, [plugin_channel(1, "Dawsome Test Sine", vst3(SINE), SINE_HALF)], [note(1, 24, 96, 69)], 2)
auto["playlist"] = {"next_track_id": 9, "next_clip_id": 3,
                    "tracks": [{"id": i, "name": f"Track {i}", "mute": False} for i in range(1, 9)],
                    "clips": [{"id": 1, "track": 1, "start": 0, "length": 384, "kind": "pattern", "pattern": 1},
                              {"id": 2, "track": 2, "start": 0, "length": 384, "kind": "automation", "automation": 1}]}
auto["automation"] = {"next_id": 2, "list": [{"id": 1, "target": {"channel_param": {"channel": 1, "param_id": 0}},
                                              "points": [{"tick": 0, "value": 0.5, "curve": "hold"},
                                                         {"tick": 48, "value": 0.25}]}]}
FIX["p27_vst3_auto"] = auto
FIX["p27_vst3_notes"] = project(48000, [plugin_channel(1, "Dawsome Test Sine", vst3(SINE), SINE_HALF)],
                                [note(1, 24, 24, 69), note(1, 48, 48, 81, 0.5)], 2)
FIX["p27_vst3_fail"] = project(48000, [plugin_channel(1, "Dawsome Test Fail", vst3(FAIL),
                                                      b64(own_controller_preset(FAIL, 0.5)))],
                               [note(1, 0, 384, 69)], 2)
FIX["p27_vst3_latency"] = project(44100, [kick(1, 1)], [note(1, 0)], 2,
                                  mixer(effect(LATE, "Dawsome Test Latency", b64(own_controller_preset(LATE, 0.5))), 2))


def record(id_, name, kind, params):
    return {"id": id_, "name": name, "vendor": "Dawsome", "version": "1.0.0", "kind": kind, "gui": "none",
            "params": params}


# What a scan of the test module lists, by id (the controller class is no plugin).
SCANNED = sorted([record(SINE, "Dawsome Test Sine", "instrument", 5),
                  record(GAIN, "Dawsome Test Gain", "effect", 3),
                  record(FAIL, "Dawsome Test Fail", "instrument", 1),
                  record(TICK, "Dawsome Test Tick", "instrument", 2),
                  record(LATE, "Dawsome Test Latency", "effect", 8),
                  record(VIEW, "Dawsome Test View", "instrument", 5)], key=lambda r: r["id"])

CLAP_PLUGINS = [
    {"id": "org.dawsome.test.gain", "name": "Dawsome Test Gain", "vendor": "Dawsome", "version": "1.0.0",
     "kind": "effect", "gui": "none", "params": 1},
    {"id": "org.dawsome.test.sine", "name": "Dawsome Test Sine", "vendor": "Dawsome", "version": "1.0.0",
     "kind": "instrument", "gui": "none", "params": 1}]
# A catalogue of both formats, in the order a saved one has: files by path, each file's plugins
# by id, a VST3 record naming its format right after its file, ranks by path.
CATALOGUE = {"version": 1, "files": [
    {"file": "/plugins/dawsome_test_plugin.clap", "size": 4096, "mtime": 1, "status": "ok", "plugins": CLAP_PLUGINS},
    {"file": "/vst3/Dawsome Test VST3 Crash.vst3", "format": "vst3", "size": 512, "mtime": 2,
     "status": "failed", "reason": "crashed: signal 6", "plugins": []},
    {"file": "/vst3/Dawsome Test VST3.vst3", "format": "vst3", "size": 8192, "mtime": 3, "status": "ok",
     "plugins": SCANNED}],
    "ranks": {"/plugins/dawsome_test_plugin.clap": 0, "/vst3/Dawsome Test VST3 Crash.vst3": 1,
              "/vst3/Dawsome Test VST3.vst3": 1}}


# --- the renders, frame by frame ---------------------------------------------------------------
def frames_of(sr, bars=1, mbpm=120000):
    return t2s(384 * bars, sr, mbpm)


def render_sine():
    sr = 48000
    out = [0.0] * frames_of(sr)
    s0, s1 = t2s(24, sr, 120000), t2s(120, sr, 120000)
    for k in range(s0, s1):
        out[k] = sine(k - s0, 0.5)
    return sr, out


def render_impulse(gain, delay=0):
    sr = 44100
    out = [0.0] * frames_of(sr)
    out[delay] = f32(1.0 * gain)
    return sr, out


def render_auto():
    sr = 48000
    out = [0.0] * frames_of(sr)
    s0, s1, change = t2s(24, sr, 120000), t2s(120, sr, 120000), t2s(48, sr, 120000)
    for k in range(s0, s1):
        out[k] = sine(k - s0, 0.5 if k < change else 0.25)
    return sr, out


def render_notes():
    sr = 48000
    out = [0.0] * frames_of(sr)
    a4, a5, end = t2s(24, sr, 120000), t2s(48, sr, 120000), t2s(96, sr, 120000)
    for k in range(a4, a5):
        out[k] = sine(k - a4, 0.5)
    for k in range(a5, end):
        out[k] = sine(k - a5, 0.5, 0.5, 81)
    return sr, out


def render_fail(block=512):
    sr = 48000
    out = [0.0] * frames_of(sr)
    for k in range(FAIL_AFTER * block):
        out[k] = sine(k, 0.5)
    return sr, out


def sha256_of(frames):
    """The SHA-256 of a render's data as a 32-bit float WAV holds it: each frame left then right
    (the two sides alike here), little-endian."""
    sha = hashlib.sha256()
    for value in frames:
        sha.update(struct.pack("<ff", value, value))
    return sha.hexdigest()


def render_answer(fixture, made, probes, extra=None):
    sr, frames = made
    answer = {"fixture": fixture + ".json", "rate": sr, "frames": len(frames), "sha256": sha256_of(frames),
              "nonzero": sum(1 for v in frames if v != 0.0),
              "probes": [[k, frames[k]] for k in probes]}
    if extra:
        answer.update(extra)
    return answer


def transport_answer(sr, mbpm, block, call):
    """What the Latency class reports of the transport of call `call` (0-based) of `block`
    frames after Play: the output clock at the call's first frame, the song tick that frame is
    in (the last tick whose sample is at or before it), and the tick as quarter notes, its bar's
    start (4/4) and its sample; each is reported divided by `divisor`."""
    clock = call * block
    tick = 0
    while t2s(tick + 1, sr, mbpm) <= clock:
        tick += 1
    beats = tick / 96
    return {"rate": sr, "tempo_mbpm": mbpm, "block": block, "call": call,
            "tempo": mbpm / 1000, "beats": beats, "bar": math.floor(beats / 4) * 4.0,
            "playing": 1.0, "samples": t2s(tick, sr, mbpm), "clock": clock, "tick": tick,
            "divisor": 2 ** 24}


def expected():
    sr = 48000
    s0, s1 = t2s(24, sr, 120000), t2s(120, sr, 120000)
    vals = [sine(n, 0.5) for n in range(s1 - s0)]
    rms = math.sqrt(sum(v * v for v in vals) / len(vals))
    change = t2s(48, sr, 120000)
    call = 19
    at = call * 512
    renders = {
        "sine": render_answer("p27_vst3_sine", render_sine(), [s0 - 1, s0, s0 + 1, s0 + 2, s0 + 3, s0 + 1200,
                                                                s1 - 1, s1],
                              {"rms": [s0, s1, round(rms, 7)]}),
        "fx": render_answer("p27_vst3_fx", render_impulse(0.5), [0, 1]),
        "fx_unity": render_answer("p27_vst3_fx_unity", render_impulse(1.0), [0, 1]),
        "auto": render_answer("p27_vst3_auto", render_auto(), [change - 1, change, change + 1]),
        "notes": render_answer("p27_vst3_notes", render_notes(),
                               [change - 1, change, change + 1, t2s(96, sr, 120000) - 1, t2s(96, sr, 120000)]),
        "fail": render_answer("p27_vst3_fail", render_fail(), [0, 1, FAIL_AFTER * 512 - 1, FAIL_AFTER * 512],
                              {"block": 512, "notice": "Plugin failed: Dawsome Test Fail: it failed while processing"}),
        "latency": render_answer("p27_vst3_latency", render_impulse(0.5, LATENCY), [0, LATENCY - 1, LATENCY],
                                 {"latency": LATENCY}),
    }
    presets = {
        "sine_0.5": SINE_HALF,
        "sine_0.25": b64(sine_preset(0.25)),
        "sine_1.0": b64(sine_preset(1.0)),
        "sine_0.5_as_gain": b64(sine_preset(0.5, GAIN)),
        "sine_0.5_component_only": b64(preset(SINE, component_state(0.5))),
        "gain_0.5": b64(own_controller_preset(GAIN, 0.5)),
        "gain_0.25": b64(own_controller_preset(GAIN, 0.25)),
        "clap_0.5": b64(component_state(0.5)),
    }
    return {
        "ids": {"sine": SINE, "controller": CTRL, "gain": GAIN, "fail": FAIL, "tick": TICK, "latency": LATE,
                "view": VIEW},
        "presets": presets,
        "preset_bytes": {name: len(base64.b64decode(text)) for name, text in presets.items()},
        "refused": {"clap_0.5": "its saved state is not a VST3 preset",
                    "sine_0.5_as_gain": "its saved state is for another plugin"},
        "scan": {"plugins": SCANNED, "crash": "Dawsome Test VST3 Crash.vst3", "crash_reason": "crashed: signal 6",
                 "module": "Dawsome Test VST3.vst3", "first": "2 files: 2 scanned, 0 unchanged",
                 "second": "2 files: 0 scanned, 2 unchanged", "touched": "2 files: 1 scanned, 1 unchanged"},
        "missing": ["Missing plugin: Missing Synth (vst3:%s)" % MISSING_SYNTH,
                    "Missing plugin: Missing FX (vst3:%s)" % MISSING_FX],
        "missing_impulses": [t2s(t, 44100, 120000) for t in (0, 96, 192, 288)],
        "renders": renders,
        "param": {"block": 512, "call": call, "first_frame": at, "gain": 0.25,
                  "frames": [[at - 1, sine(at - 1 - s0, 0.5)], [at, sine(at - s0, 0.25)],
                             [at + 1, sine(at + 1 - s0, 0.25)]],
                  "captured": b64(sine_preset(0.25))},
        "dropped": {"rate": 8000, "tempo_mbpm": 522000, "notes": 600, "sent": 512, "dropped": 88},
        "latency": LATENCY,
        "linked": {"param": 9, "value": 1.0},
        "transport": transport_answer(44100, 120000, 512, 9),
        "timer": {"period_ms": TIMER_MS, "fires_at_ms": [100, 200, 300], "ticks_param": 8,
                  "edit": 0.125},
        "class_id_text": {"uid": [0x44617773, 0x6F6D6554, 0x65737453, 0x696E6531], "text": SINE},
    }


def files():
    """Every file --write makes, by its name under DIR, as bytes."""
    out = {name + ".json": J(doc).encode() for name, doc in FIX.items()}
    out["p27_catalogue.json"] = J(CATALOGUE).encode()
    out["p27_expected.json"] = (J(expected()) + "\n").encode()
    return out


def main():
    args = sys.argv[1:]
    if len(args) == 2 and args[0] in ("--write", "--check"):
        made = files()
        stale = []
        for path, content in sorted(made.items()):
            target = os.path.join(args[1], path)
            if args[0] == "--write":
                with open(target, "wb") as f:
                    f.write(content)
            else:
                try:
                    with open(target, "rb") as f:
                        same = f.read() == content
                except OSError:
                    same = False
                if not same:
                    stale.append(path)
        if stale:
            sys.exit("not what gen_p27_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check":
            return
    elif args:
        sys.exit(__doc__)
    answers = expected()
    print("class ids: " + ", ".join("%s %s" % item for item in answers["ids"].items()))
    for name, text in answers["presets"].items():
        print("preset %-24s %3d bytes  %s" % (name, answers["preset_bytes"][name], text))
    for name, r in answers["renders"].items():
        print("%-9s %-24s %6d frames, %6d nonzero, sha256 %s..., probes %s" % (
            name, r["fixture"], r["frames"], r["nonzero"], r["sha256"][:16],
            ", ".join("%d: %.10g" % (k, v) for k, v in r["probes"])))
    p = answers["param"]
    print("param: gain %g sent before call %d of %d frames: " % (p["gain"], p["call"], p["block"]) +
          ", ".join("%d: %.10g" % (k, v) for k, v in p["frames"]))
    print("scan: " + ", ".join("%s %s (%s, %d)" % (r["id"], r["name"], r["kind"], r["params"])
                               for r in answers["scan"]["plugins"]))


if __name__ == "__main__":
    main()
