"""Known answers for VST3 in the app. Both formats in one list (a plugin
installed as both listed once, as its CLAP build), pitch bend and the mod wheel through the
plugin's MIDI mapping, stepped and mapped parameters, restarts, and the latency the app shows.

Expected values come from the rules and the test plugins' formulas restated here, never from
running Dawsome. The test plugins are our own code (crates/test-plugin-vst3, crates/test-plugin);
their sound is gen_p27_expected.py's and gen_p7_expected.py's formulas, whose helpers are copied
in below (`python3 -I` cannot import a sibling file). The p27 fixtures and answers these tests
also play are gen_p27_expected.py's files, read from DIR.

The rules:
  list      one list over both formats: the ok files' plugins of a kind, a plugin in two files of
            one format once (the file found first), sorted by name, then format (CLAP first),
            then id. A VST3 plugin is not offered when a CLAP plugin of the same kind, name and
            maker is ("twins"): names trimmed, lower-cased and their spaces collapsed; makers
            the same with the whole words CLAP, VST3 and VST dropped. That CLAP row is "twinned".
            A failed file offers nothing, so the VST3 twin of a failed CLAP file is listed. The
            failed files of both formats are listed under "Could not load (n)", by path.
  dump      the picker's headless dump writes a VST3 row as "<name> (VST3)", a CLAP row as its
            name, the failed files as "<file name>: <reason>"
  bend      the host asks the plugin's controller which parameter MIDI bus 0, channel 0,
            controller 129 (pitch bend) and 1 (the mod wheel) move. A device's bend b (-8192 to
            8191, held there) reaches that parameter as a point at the bend's frame of
            v = (b + 8192) / 16383; the wheel w as v = (w & 0x7F) / 127 (CLAP's MIDI bytes carry
            the same values); a later control on one frame replaces the earlier; the controller
            is given each value back. v decodes back to b exactly through round(v * 16383) and
            min(16383, floor(v * 16384)), both for every b (checked below); / 16384 would not.
            A plugin with no mapping is sent neither. A parameter any mapping names (bus 0, MIDI
            channels 0-15, controllers 0-129) is hidden from the panel.
  sine      the VST3 test sine's controller maps the bend to parameter 1 "Bend" and the wheel
            to 2 "Wheel"; its processor bends as the CLAP test sine does: at a point v of
            parameter 1 on frame k, b = round(v * 16383) - 8192, r = 2^(b/49152), and the note
            reads on from where its phase is: y = f32(gain * velocity * cos(tau * S / sr)),
            S = S_k + f * r * (n - k), S_k the phase sum at frame k (f * k while unbent); unbent,
            y = f32(gain * velocity * cos(tau * f * n / sr)). A point v of parameter 2 is
            reported back as read-only parameter 3 "Wheel heard".
  steps     the gain effect's parameter 1 "Steps" has 4 steps (0..4 in the engine, default 4);
            a value from the engine is rounded to the nearest step (half away from zero), held
            to 0..4, NaN taken as 0, and sent as k / 4; the plugin reports what it got as
            read-only parameter 2 "Steps heard"; its text is "k/4", k = min(4, floor(v * 5)).
            A drag of the panel's slider moves the value by (pixels / 120) * 4, rounded to a step.
  latency   "delays its sound by N frames (M ms); not compensated", M in tenths of a ms by
            integer half-up: (20000 N + rate) div (2 rate); the Latency class plays its input 64
            frames late, 128 once asked to restart with kLatencyChanged.
  restart   the test module's `dawsome_test_vst3_restart(flags)`: every controller made on the
            caller's thread asks its host for restartComponent(flags); kParamTitlesChanged
            renames parameter 0 "Level" first, kMidiCCAssignmentChanged makes the sine map
            nothing, kLatencyChanged makes the Latency class 128 frames late.

Run:
  python3 -I gen_p28_expected.py              print the known answers
  python3 -I gen_p28_expected.py --write DIR  also write p28_expected.json into DIR
  python3 -I gen_p28_expected.py --check DIR  exit 1 unless DIR's p28_expected.json is what
                                              --write writes
"""
import base64, hashlib, json, math, os, struct, sys


def f32(x):
    return struct.unpack("<f", struct.pack("<f", x))[0]


def t2s(tick, sr, mbpm, ppq=96):
    # the project's one tick->sample rule, exact integer math
    return tick * 60 * sr * 1000 // (mbpm * ppq)


def J(d):
    return json.dumps(d, separators=(",", ":"))


def class_id(name):
    raw = name.encode("ascii")
    assert len(raw) == 16, name
    return raw.hex().upper()


SINE = class_id("DawsomeTestSine1")
GAIN = class_id("DawsomeTestGain1")
FAIL = class_id("DawsomeTestFail1")
TICK = class_id("DawsomeTestTick1")
LATE = class_id("DawsomeTestLate1")
VIEW = class_id("DawsomeTestView1")


def component_state(gain):
    return b"OTP1" + struct.pack("<d", gain)


def preset(cid, comp, cont):
    """The .vstpreset bytes (gen_p27's layout)."""
    header_size = 4 + 4 + 32 + 8
    data, entries, offset = b"", [], header_size
    for tag, bytes_ in [(b"Comp", comp), (b"Cont", cont)]:
        entries.append(tag + struct.pack("<qq", offset, len(bytes_)))
        data += bytes_
        offset += len(bytes_)
    header = b"VST3" + struct.pack("<i", 1) + cid.encode("ascii") + struct.pack("<q", offset)
    return header + data + b"List" + struct.pack("<i", len(entries)) + b"".join(entries)


def b64(raw):
    return base64.b64encode(raw).decode()


def sha256_of(frames):
    """The SHA-256 of frames as a 32-bit float WAV holds them: left then right (alike here),
    little-endian (gen_p27's)."""
    sha = hashlib.sha256()
    for value in frames:
        sha.update(struct.pack("<ff", value, value))
    return sha.hexdigest()


# --- the twin rule -------------------------------------------------------------------------
FORMAT_WORDS = ("clap", "vst3", "vst")


def name_key(name):
    return " ".join(name.split()).lower()


def maker_key(vendor):
    return " ".join(word for word in vendor.lower().split() if word not in FORMAT_WORDS)


def twin_key(plugin):
    return (plugin["kind"], name_key(plugin["name"]), maker_key(plugin["vendor"]))


def offered(files, kind):
    """What the list offers of `kind` from `files` (each {"format", "status", "plugins"}, in
    search order): [(format, plugin, twinned)]."""
    by_format = {"clap": [], "vst3": []}
    seen = set()
    for file in files:
        if file["status"] != "ok":
            continue
        for plugin in file["plugins"]:
            if (file["format"], plugin["id"]) in seen:
                continue
            seen.add((file["format"], plugin["id"]))
            if plugin["kind"] == kind:
                by_format[file["format"]].append(plugin)
    clap_keys = {twin_key(p) for p in by_format["clap"]}
    vst3_keys = {twin_key(p) for p in by_format["vst3"]}
    rows = [("clap", p, twin_key(p) in vst3_keys) for p in by_format["clap"]]
    rows += [("vst3", p, False) for p in by_format["vst3"] if twin_key(p) not in clap_keys]
    rows.sort(key=lambda row: (row[1]["name"], row[0] != "clap", row[1]["id"]))
    return rows


def plugin(id_, name, vendor, kind):
    return {"id": id_, "name": name, "vendor": vendor, "version": "1.0.0", "kind": kind, "gui": "none",
            "params": 0}


def twin_case(name, clap, vst3, clap_failed=False):
    """A catalogue of one CLAP file and one VST3 file, and what each kind offers of it."""
    files = [{"file": "/clap/a.clap", "format": "clap", "status": "failed" if clap_failed else "ok",
              "plugins": [] if clap_failed else clap},
             {"file": "/vst3/a.vst3", "format": "vst3", "status": "ok", "plugins": vst3}]
    if clap_failed:
        files[0]["reason"] = "crashed: signal 6"
    return {"case": name, "files": files,
            "offered": {kind: [{"format": f, "id": p["id"], "twin": t} for f, p, t in offered(files, kind)]
                        for kind in ("instrument", "effect")}}


TWINS = [
    twin_case("LSP names its maker by format",
              [plugin("in.lsp-plug.ab_tester_x2_mono", "A/B Tester x2 Mono", "LSP CLAP", "effect")],
              [plugin("4C5350414254584D4F4E4F0000000000", "A/B Tester x2 Mono", "LSP VST3", "effect")]),
    twin_case("the same maker in both",
              [plugin("org.surge-synth-team.surge-xt", "Surge XT", "Surge Synth Team", "instrument")],
              [plugin("ABCDEF019182FAEB566D624153675854", "Surge XT", "Surge Synth Team", "instrument")]),
    twin_case("another maker",
              [plugin("com.acme.reverb", "Reverb", "Acme", "effect")],
              [plugin("00000000000000000000000052657662", "Reverb", "Other", "effect")]),
    twin_case("another kind",
              [plugin("com.acme.pad", "Pad", "Acme", "instrument")],
              [plugin("00000000000000000000000000506164", "Pad", "Acme", "effect")]),
    twin_case("its CLAP file failed",
              [plugin("org.surge-synth-team.surge-xt", "Surge XT", "Surge Synth Team", "instrument")],
              [plugin("ABCDEF019182FAEB566D624153675854", "Surge XT", "Surge Synth Team", "instrument")],
              clap_failed=True),
    twin_case("spaces and case",
              [plugin("com.acme.lead", "  Big   Lead ", "ACME vst", "instrument")],
              [plugin("000000000000000000000000004C6561", "big lead", "Acme", "instrument")]),
    twin_case("a word that only starts like a format",
              [plugin("com.clapton.delay", "Delay", "Clapton Audio", "effect")],
              [plugin("00000000000000000000000044656C61", "Delay", "Audio", "effect")]),
    twin_case("VSTi is a word of its own",
              [plugin("com.v.synth", "Synth", "VSTi Works", "instrument")],
              [plugin("00000000000000000000000053796E74", "Synth", "VSTi Works VST3", "instrument")]),
]
# The spec's cases, by hand: twins, twins, both, both, the VST3 listed, twins, both, twins.
assert [[r["twin"] for r in c["offered"]["instrument"] + c["offered"]["effect"]] for c in TWINS] == [
    [True], [True], [False, False], [False, False], [False], [True], [False, False], [True]], TWINS


# --- the test plugins' catalogue as the app lists it (K1) ------------------------------------
CLAP_FILE = "dawsome_test_plugin.clap"
CRASH = "Dawsome Test VST3 Crash.vst3"
CLAP_SCANNED = [plugin("org.dawsome.test.gain", "Dawsome Test Gain", "Dawsome", "effect"),
                plugin("org.dawsome.test.sine", "Dawsome Test Sine", "Dawsome", "instrument")]
VST3_SCANNED = [plugin(SINE, "Dawsome Test Sine", "Dawsome", "instrument"),
                plugin(GAIN, "Dawsome Test Gain", "Dawsome", "effect"),
                plugin(FAIL, "Dawsome Test Fail", "Dawsome", "instrument"),
                plugin(TICK, "Dawsome Test Tick", "Dawsome", "instrument"),
                plugin(LATE, "Dawsome Test Latency", "Dawsome", "effect"),
                plugin(VIEW, "Dawsome Test View", "Dawsome", "instrument")]


def dump(files, kind):
    rows = offered(files, kind)
    lines = [p["name"] + (" (VST3)" if f == "vst3" else "") for f, p, _ in rows]
    failed = sorted((f for f in files if f["status"] == "failed"), key=lambda f: f["file"])
    if failed:
        lines.append("Could not load (%d)" % len(failed))
        lines += ["%s: %s" % (os.path.basename(f["file"]), f["reason"]) for f in failed]
    return lines


BOTH = [{"file": "/t/" + CLAP_FILE, "format": "clap", "status": "ok", "plugins": CLAP_SCANNED},
        {"file": "/t/" + CRASH, "format": "vst3", "status": "failed", "reason": "crashed: signal 6",
         "plugins": []},
        {"file": "/t/Dawsome Test VST3.vst3", "format": "vst3", "status": "ok", "plugins": VST3_SCANNED}]
VST3_ALONE = [BOTH[2]]


def library_rows(files):
    """Library › Instruments › Plugins, by maker: each row's name, format and hint value."""
    makers = {}
    for f, p, twin in offered(files, "instrument"):
        note = "VST3" if f == "vst3" else ("also installed as VST3" if twin else None)
        makers.setdefault(p["vendor"], []).append({"name": p["name"], "format": f, "note": note})
    return makers


LISTS = {
    "both": {"instruments": dump(BOTH, "instrument"), "effects": dump(BOTH, "effect"),
             "library": library_rows(BOTH)},
    "vst3_alone": {"instruments": dump(VST3_ALONE, "instrument"), "library": library_rows(VST3_ALONE),
                   "added": {"channel": 2, "id": "vst3:" + SINE, "name": "Dawsome Test Sine"},
                   "first_capture": b64(preset(SINE, component_state(1.0), b"OTC1" + struct.pack("<I", 0)))},
}
# By hand (the spec's K1): the CLAP sine twins the VST3 one; the VST3 Fail, Tick, Latency and View
# (the plugin windows' sixth class) have no CLAP build; the crash module is the one failed file.
assert LISTS["both"]["instruments"] == ["Dawsome Test Fail (VST3)", "Dawsome Test Sine", "Dawsome Test Tick (VST3)",
                                        "Dawsome Test View (VST3)", "Could not load (1)",
                                        CRASH + ": crashed: signal 6"]
assert LISTS["both"]["effects"] == ["Dawsome Test Gain", "Dawsome Test Latency (VST3)", "Could not load (1)",
                                    CRASH + ": crashed: signal 6"]
assert [r["note"] for r in LISTS["both"]["library"]["Dawsome"]] == ["VST3", "also installed as VST3", "VST3", "VST3"]

# What a scan child prints for the test module (`--scan-child` of the app, `scan --child` of the
# command line): a progress line per audio class, then the result line, the records by id with
# the parameter counts of the grown oracle (Sine 5, Gain 3, View 5; gen_p27_expected.py's
# SCANNED).
SCAN_PARAMS = {SINE: 5, GAIN: 3, FAIL: 1, TICK: 2, LATE: 8, VIEW: 5}
SCAN_RECORDS = sorted([dict(p, params=SCAN_PARAMS[p["id"]]) for p in VST3_SCANNED], key=lambda p: p["id"])
SCAN_CHILD = "".join("dawsome-scan-progress %d\n" % n for n in range(1, len(VST3_SCANNED) + 1)) + \
    "dawsome-scan-result " + J({"plugins": SCAN_RECORDS}) + "\n"


# --- bend and wheel (K2) -----------------------------------------------------------------------
BEND_PARAM, WHEEL_PARAM, WHEEL_HEARD = 1, 2, 3
NO_PARAM = 0xFFFFFFFF


def bend_value(b):
    return (min(max(b, -8192), 8191) + 8192) / 16383


def wheel_value(w):
    return (w & 0x7F) / 127


def decoded(v):
    """b as the test sine decodes a point v (round half away from zero; v * 16383 >= 0)."""
    return math.floor(v * 16383 + 0.5) - 8192


for b in range(-8192, 8192):
    v = bend_value(b)
    assert decoded(v) == b and min(16383, math.floor(v * 16384)) - 8192 == b, b
assert sum(1 for b in range(-8192, 8192) if math.floor((b + 8192) / 16384 * 16383 + 0.5) - 8192 != b) == 8191


def play_bent(calls, bends, block=512, sr=48000, gain=0.5, velocity=0.5, key=69):
    """A live note from frame 0 of call 0, bent at the first frame of each call `bends` names
    ({call: b}); the CLAP test sine's arithmetic, operation for operation."""
    f = 440.0 * 2.0 ** ((key - 69) / 12.0)
    out, bent = [], None
    for n in range(calls * block):
        call, at = divmod(n, block)
        if at == 0 and call in bends:
            r = 2.0 ** (decoded(bend_value(bends[call])) / 49152.0)
            base = f * n if bent is None else bent[1] + f * bent[0] * (n - bent[2])
            bent = (r, base, n)
        if bent is None:
            phase = math.tau * f * n / sr
        else:
            phase = math.tau * (bent[1] + f * bent[0] * (n - bent[2])) / sr
        out.append(f32(gain * velocity * math.cos(phase)))
    return out


BENDS = {1: 8191, 2: 0, 3: -8192}
BENT = play_bent(4, BENDS)
UNBENT = play_bent(4, {})
PROBES = [511, 512, 1023, 1024, 1536, 2047]
# The spec's values (vst3-host-phase2.md, K2), to ten digits.
assert ["%.10g" % BENT[k] for k in PROBES] == ["-0.1004869416", "-0.08714301139", "0.2382726669", "0.2426635325",
                                              "-0.1409350932", "0.1173973233"]
assert all(BENT[n] == UNBENT[n] for n in range(512))


def splits(seed=28, count=64):
    """Seeded piece sizes 1..97 (an LCG, so every language gets the same ones), cycled over a
    call and cut at its end."""
    sizes, x = [], seed
    for _ in range(count):
        x = (1103515245 * x + 12345) % 2 ** 31
        sizes.append(1 + x % 97)
    return sizes


def midi_bend(b):
    raw = b + 8192
    return [0xE0, raw & 0x7F, raw >> 7]


BEND = {
    "rate": 48000, "block": 512, "calls": 4, "key": 69, "velocity": 0.5, "gain": 0.5,
    "note_on": [0x90, 69, 64],
    "bends": [[call, b] for call, b in sorted(BENDS.items())],
    "midi": [[call, midi_bend(b)] for call, b in sorted(BENDS.items())],
    "frames": len(BENT), "sha256": sha256_of(BENT), "probes": [[k, BENT[k]] for k in PROBES],
    "unbent_sha256": sha256_of(UNBENT), "unbent_probes": [[k, UNBENT[k]] for k in PROBES],
    "param": BEND_PARAM, "wheel_param": WHEEL_PARAM, "wheel_heard": WHEEL_HEARD, "no_param": NO_PARAM,
    # After the center is sent (before call 2) and a tick: the controller holds 8192/16383.
    "center_value": bend_value(0),
    "values": {str(b): bend_value(b) for b in (-8192, 0, 8191)},
    "wheel": {"value": 100, "heard": wheel_value(100)},
    "masked_wheel": {"value": 200, "heard": wheel_value(200)},
    "clamped": {"value": 30000, "as": 8191, "sent": bend_value(30000)},
    "three_bends": [100, -3000, 8191],
    "splits": [[1], [7], [64], splits()],
    # The Tick class has no MIDI mapping: played at gain 0.5 it is the unbent sine.
    "tick_preset": b64(preset(TICK, component_state(0.5), b"")),
}
assert BEND["center_value"] == 0.5000305194408838 and BEND["wheel"]["heard"] == 0.7874015748031497
assert BEND["masked_wheel"]["heard"] == 72 / 127 == 0.5669291338582677 and BEND["clamped"]["sent"] == 1.0


# --- stepped and hidden parameters (K3) --------------------------------------------------------
STEPS = 4
SLIDER_WIDTH = 120


def step_round(x):
    """The engine's value of a parameter of 4 steps as the host sends it: the nearest step, half
    away from zero, held to 0..4, NaN as 0, over 4."""
    if x is None or x != x:
        return 0.0
    k = math.floor(abs(x) + 0.5) * (1 if x >= 0 else -1)
    return min(max(k, 0), STEPS) / STEPS


def step_text(v):
    return "%d/%d" % (min(STEPS, math.floor(v * (STEPS + 1))), STEPS)


def dragged(start, pixels):
    value = start + pixels / SLIDER_WIDTH * STEPS
    return float(math.floor(min(max(value, 0), STEPS) + 0.5))


RING = [2.4, 2.5, 2.6, 7.0, -1.0, None]
STEPPED = {
    "param": 1, "heard_param": 2, "name": "Steps",
    "shape": {"min": 0.0, "max": float(STEPS), "stepped": True, "default": float(STEPS)},
    "default_text": step_text(1.0),
    "gain_rows": ["Gain", "Steps", "Steps heard"],
    "sine_rows": ["Gain", "Wheel heard", "Linked"],
    "sine_params": ["Gain", "Bend", "Wheel", "Wheel heard", "Linked"],
    "drag": {"from": float(STEPS), "pixels": -90, "slider_width": SLIDER_WIDTH, "sent": dragged(STEPS, -90),
             "heard": step_round(dragged(STEPS, -90)), "value": dragged(STEPS, -90),
             "text": step_text(step_round(dragged(STEPS, -90)))},
    # The ring sends f32 values; null stands for NaN.
    "ring": [[x, step_round(None if x is None else f32(x))] for x in RING],
    "capture": b64(preset(GAIN, component_state(0.5), b"")),
}
assert STEPPED["default_text"] == "4/4" and STEPPED["drag"]["sent"] == 1.0
assert (STEPPED["drag"]["heard"], STEPPED["drag"]["text"]) == (0.25, "1/4")
assert [h for _, h in STEPPED["ring"]] == [0.5, 0.75, 0.75, 1.0, 0.0, 0.0]


# --- restarts and latency (K4) -----------------------------------------------------------------
FLAGS = {"reload": 1, "io": 2, "values": 4, "latency": 8, "titles": 16, "midi_cc": 32}


def latency_text(frames, rate):
    tenths = (20000 * frames + rate) // (2 * rate)
    return "delays its sound by %d frames (%d.%d ms); not compensated" % (frames, tenths // 10, tenths % 10)


def latency_render(delay, gain=0.5, sr=44100):
    out = [0.0] * t2s(384, sr, 120000)
    out[delay] = f32(1.0 * gain)
    return out


LATE_128 = latency_render(128)
LATENCY = {
    "rate": 44100, "before": 64, "after": 128,
    "foot_before": latency_text(64, 44100)[0].upper() + latency_text(64, 44100)[1:],
    "foot_after": latency_text(128, 44100)[0].upper() + latency_text(128, 44100)[1:],
    "slot_hint": "VST3 · " + latency_text(128, 44100),
    "rack_hint": "VST3",
    "texts": [[n, rate, latency_text(n, rate)] for n, rate in
              [(64, 44100), (128, 44100), (12, 48000), (24, 48000), (1, 48000), (441000, 44100)]],
    "render": {"fixture": "p27_vst3_latency.json", "frames": len(LATE_128), "sha256": sha256_of(LATE_128),
               "probes": [[k, LATE_128[k]] for k in (0, 64, 127, 128, 129)]},
}
assert LATENCY["foot_before"] == "Delays its sound by 64 frames (1.5 ms); not compensated"
assert LATENCY["texts"][1][2] == "delays its sound by 128 frames (2.9 ms); not compensated"
assert LATENCY["texts"][2][2].startswith("delays its sound by 12 frames (0.3 ms)")  # 0.25 ms, half up

RESTART = {"flags": FLAGS, "renamed": "Level", "export": "dawsome_test_vst3_restart"}


def expected():
    return {"ids": {"sine": SINE, "gain": GAIN, "fail": FAIL, "tick": TICK, "latency": LATE, "view": VIEW},
            "twins": TWINS, "lists": LISTS, "scan_child": SCAN_CHILD, "bend": BEND, "steps": STEPPED,
            "latency": LATENCY, "restart": RESTART}


def files():
    return {"p28_expected.json": (J(expected()) + "\n").encode()}


def main():
    args = sys.argv[1:]
    if len(args) == 2 and args[0] in ("--write", "--check"):
        stale = []
        for path, content in sorted(files().items()):
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
            sys.exit("not what gen_p28_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check":
            return
    elif args:
        sys.exit(__doc__)
    answers = expected()
    for name, lines in answers["lists"]["both"].items():
        print("%-12s %s" % (name, lines))
    b = answers["bend"]
    print("bend: sha256 %s..., probes %s" % (b["sha256"][:16], ", ".join("%d: %.10g" % (k, v) for k, v in b["probes"])))
    print("unbent: sha256 %s..." % b["unbent_sha256"][:16])
    print("steps: %s" % answers["steps"]["ring"])
    print("latency: %s / %s" % (answers["latency"]["foot_before"], answers["latency"]["foot_after"]))


if __name__ == "__main__":
    main()
