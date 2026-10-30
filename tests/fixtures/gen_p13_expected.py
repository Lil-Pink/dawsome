"""Known answers for a channel's sound settings: tuning, the envelope, the velocity's weight,
reverse, the start point, one-shot against hold, the low-pass filter, and an instrument set's
own release replaced. Each case is one channel playing one note; its render says, sample for
sample, how the settings shaped it.

Expected values come from the rules written out below (restated from the design's decisions,
never engine code) and from gen_p11_expected.py's rules for an instrument set's zones (imported,
as gen_p12_expected.py does). Every input is authored here: the WAVs and the projects.

The rules (crates/engine/src/dsp/shape.rs states them the same way):
  tune       speed = 2^((key - 60)/12) from the sampler's semitone table, times 2^(semitones/12)
             from the same table, times exp2(cents / 1200) when cents is not 0 (EXP2 below)
  velocity   level = 1 - w(1 - v) in f32 for a weight w other than 1 (w = 1: v); a note at
             velocity 0 plays nothing, on a sampler as on a set, whatever the weight
  envelope   A = round(attack * rate), R = round(release * rate) frames (0: the sound's own,
             none for a sampler); t / A while t < A, else 1; from the note's end at frame off
             (heard when held, or with R > 0) level(off) * exp2(-15 r / R) for r = t - off < R,
             then the voice ends (R = 0: at off)
  reading    S = floor(start * frames); p = S + t * speed, k = floor(p), f = p - k; value =
             s[k] at f = 0, else gen_p11's 4-point Hermite of s[k-1], s[k], s[k+1], s[k+2],
             a point before S or past the last frame 0; a position past the last frame ends
             the voice; reversed, s[i] is the sound's frame (frames - 1 - i)
  filter     the cookbook's low-pass biquad at the cutoff (below 0.45 * rate), Q = 1/sqrt 2 +
             resonance * (10 - 1/sqrt 2), Direct Form I in f64 on the value, then f32. Its
             coefficients come from sin and cos, whose last bit may differ between maths
             libraries: the test holds that case to 1e-6, every other case to the bit
  mixing     out += value * f32(f32(f32(level_v * f32(env)) * 1.0) * 1.0): the channel at volume 1
             and centered (the pan law gives exactly 1 on both sides)
  note's end a pattern note ends after its length (a step hit after one step):
             off = s(tick + length) - s(tick), at least 1
  a set's    a zone that loops always hears the note's end; any other hears it when the
  gate       channel holds (gate "hold") or sets a release, not when the channel is
             "one_shot" without a release; with neither set, the zone's own loop_mode decides:
             a one_shot zone plays on to its end, any other is held. The channel's gate wins
             over the zone's one_shot

  p13_semitones.json  impulses every 10 frames, +12 semitones: twice the speed
  p13_cents.json      the same, +50 cents: read between frames
  p13_envelope.json   a tone held for 2 ticks, attack 1 ms, release 2 ms
  p13_velocity.json   a tone at velocity 0.5 with the weight 0.5: level 0.75
  p13_velocity_zero.json  the weight 0: a note at velocity 0 plays nothing, one at velocity 1
                      half a bar later plays the tone at level 1
  p13_reverse.json    a ramp backwards
  p13_start.json      a ramp from half way
  p13_oneshot.json    a tone 2 ticks long with an attack: plays to the tone's end
  p13_hold.json       the same tone held: cut at the note's end
  p13_filter.json     a step through the low-pass at 1 kHz
  p13_set.json        the test set "pad" (tests/fixtures/instruments) at -100 cents with its
                      release of 4 ms replaced by 10 ms
  p13_shot.json       the set "shot" (p13_shot/: one loop_mode=one_shot zone over every key on
                      the tone) with a 2-tick note and no settings: plays to the tone's end
  p13_shot_hold.json  the same held: cut at the note's end
  p13_shot_release.json  the same with a release of 2 ms: falls from the note's end
  p13_expected.json   each render's frames and its sounding frames (the filter's: every 8th of
                      its first 2000, to 1e-6)

Run:
  python3 -I gen_p13_expected.py              print the known answers
  python3 -I gen_p13_expected.py --write DIR  also write the files above (and sound/*.wav) into DIR
  python3 -I gen_p13_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import importlib.util, json, math, os, sys

HERE = os.path.dirname(os.path.abspath(__file__))


def rules():
    """gen_p11_expected.py, whose functions restate the reader's and the player's rules."""
    sys.dont_write_bytecode = True                   # no __pycache__ beside the fixtures
    spec = importlib.util.spec_from_file_location("gen_p11_expected",
                                                  os.path.join(HERE, "gen_p11_expected.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


P11 = rules()
f32, exp2, semitone_speed, t2s = P11.f32, P11.exp2, P11.semitone_speed, P11.t2s
SR, STEP, J = P11.SR, P11.STEP, P11.J
BAR_FRAMES = t2s(4 * P11.PPQ)

# ---------- the sounds ----------
IMPULSES = P11.impulses(200, range(0, 200, 10), 1.0)
RAMP = [(i + 1) / 64 for i in range(64)]
TONE = [0.5] * 1000
STEP_UP = [1.0] * 2000
SOUNDS = {"impulses": IMPULSES, "ramp": RAMP, "tone": TONE, "step": STEP_UP}

# ---------- the settings ----------
DEFAULT = {"gate": None, "semitones": 0, "cents": 0, "velocity": 1.0, "attack": 0.0,
           "release": 0.0, "reverse": False, "start": 0.0, "filter": False,
           "cutoff": 20000.0, "resonance": 0.0}
FIELDS = list(DEFAULT)


def settings(**changed):
    return dict(DEFAULT, **changed)


# case: (sound or set, settings, pattern notes (tick, length, key, velocity))
CASES = {
    "semitones": ("impulses", settings(semitones=12), [(0, 0, 60, 1.0)]),
    "cents": ("impulses", settings(cents=50), [(0, 0, 60, 1.0)]),
    "envelope": ("tone", settings(gate="hold", attack=0.001, release=0.002), [(0, 2, 60, 1.0)]),
    "velocity": ("tone", settings(velocity=0.5), [(0, 0, 60, 0.5)]),
    "velocity_zero": ("tone", settings(velocity=0.0), [(0, 0, 60, 0.0), (192, 0, 60, 1.0)]),
    "reverse": ("ramp", settings(reverse=True), [(0, 0, 60, 1.0)]),
    "start": ("ramp", settings(start=0.5), [(0, 0, 60, 1.0)]),
    "oneshot": ("tone", settings(attack=0.001), [(0, 2, 60, 1.0)]),
    "hold": ("tone", settings(gate="hold"), [(0, 2, 60, 1.0)]),
    "filter": ("step", settings(filter=True, cutoff=1000.0), [(0, 0, 60, 1.0)]),
    "set": ("pad", settings(cents=-100, release=0.01), [(0, 2, 60, 1.0)]),
    "shot": ("shot", settings(), [(0, 2, 60, 1.0)]),
    "shot_hold": ("shot", settings(gate="hold"), [(0, 2, 60, 1.0)]),
    "shot_release": ("shot", settings(release=0.002), [(0, 2, 60, 1.0)]),
}
# The sets the cases play: "pad" of tests/fixtures/instruments, and "shot", written here.
SHOT_SFZ = "<region> sample=tone.wav loop_mode=one_shot\n"
SET_ZONES = {
    "pad": lambda: P11.zones_of("pad"),
    "shot": lambda: [P11.zone(P11.region("tone.wav", mode="one_shot"), [TONE])],
}
SET_PATHS = {"pad": "instruments/pad/pad.sfz", "shot": "p13_shot/shot.sfz"}
SET_NAMES = {"pad": "Pad", "shot": "Shot"}
FILTER_TOLERANCE = 1e-6


def frames_of(seconds):
    """Seconds (an f32, as the file holds it) as frames, rounded half away from zero."""
    return math.floor(f32(seconds) * SR + 0.5)


def tune(s):
    t = semitone_speed(s["semitones"])
    return t * (exp2(f32(s["cents"]) / 1200.0) if s["cents"] != 0 else 1.0)


def envelope(attack, release, t, off):
    rise = lambda t: 1.0 if attack == 0 or t >= attack else t / attack
    if off is None or t < off: return rise(t)
    r = t - off
    if r >= release: return None
    return rise(off) * exp2(-(15 * r / release))


def low_pass(cutoff, resonance):
    fs = float(SR)
    cutoff = min(f32(cutoff), 0.45 * fs)
    q = 1 / math.sqrt(2) + f32(resonance) * (10.0 - 1 / math.sqrt(2))
    w0 = 2.0 * math.pi * cutoff / fs
    sin, cos = math.sin(w0), math.cos(w0)
    alpha = sin / (2.0 * q)
    a0 = 1.0 + alpha
    b1 = 1.0 - cos
    b0 = b1 / 2.0
    return [b0 / a0, b1 / a0, b0 / a0, -2.0 * cos / a0, (1.0 - alpha) / a0]


def sampler_render(sound, s, notes):
    """A sampler channel playing `sound` (mono) with the settings `s`."""
    out = [0.0] * BAR_FRAMES
    n = len(sound)
    attack, release = frames_of(s["attack"]), frames_of(s["release"])
    hears = s["gate"] == "hold" or release > 0
    weight = f32(s["velocity"])
    ratio = tune(s)
    coefficients = low_pass(s["cutoff"], s["resonance"]) if s["filter"] else None
    for start, off, key, v in P11.pattern_notes(notes):
        if v <= 0.0: continue                        # velocity 0 plays nothing
        speed = semitone_speed(key - 60)
        if ratio != 1.0: speed = speed * ratio
        level_v = v if weight == 1.0 else f32(1.0 - f32(weight * f32(1.0 - v)))
        first = math.floor(f32(s["start"]) * n)
        value_at = lambda i: (sound[n - 1 - i] if s["reverse"] else sound[i]) if first <= i < n else 0.0
        memory = [0.0, 0.0, 0.0, 0.0]
        t = 0
        while start + t < BAR_FRAMES:
            env = envelope(attack, release, t, off if hears else None)
            if env is None: break
            p = first + t * speed
            k = math.floor(p)
            if k >= n: break
            frac = p - k
            cur = value_at(k)
            value = cur if frac == 0.0 else P11.hermite(value_at(k - 1), cur, value_at(k + 1),
                                                       value_at(k + 2), frac)
            if coefficients:
                b0, b1, b2, a1, a2 = coefficients
                x1, x2, y1, y2 = memory
                y = b0 * value + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2
                if abs(y) < 1e-30: y = 0.0
                memory = [value, x1, y, y1]
                value = f32(y)
            amp = f32(f32(level_v * f32(env)) * 1.0)
            out[start + t] = f32(out[start + t] + f32(value * f32(amp * 1.0)))
            t += 1
    return out, out[:]


def hears_end(mode, s):
    """Whether a set's zone of loop mode `mode` hears the note's end on a channel with the
    settings `s` (the rule "a set's gate" above)."""
    if mode in ("loop_continuous", "loop_sustain"): return True
    if s["gate"] == "hold" or frames_of(s["release"]) > 0: return True
    if s["gate"] == "one_shot": return False
    return mode != "one_shot"


def set_render(name, s, notes):
    """The set `name` with the settings `s`: its zones' speed tuned, where `s` sets a release
    or an attack, it in place of the zone's own, and the note's end heard as the gate rule
    says (a one_shot zone that hears it reads as a zone held to its end)."""
    zones = SET_ZONES[name]()
    left, right = [0.0] * BAR_FRAMES, [0.0] * BAR_FRAMES
    ratio = tune(s)
    counters = {}
    for start, off, key, v in P11.pattern_notes(notes):
        for z, speed, gain in P11.start_voices(zones, counters, key, v):
            z = dict(z)
            if frames_of(s["release"]) > 0: z["release"] = frames_of(s["release"])
            if frames_of(s["attack"]) > 0: z["attack"] = frames_of(s["attack"])
            if ratio != 1.0: speed = speed * ratio
            heard = hears_end(z["mode"], s)
            if heard and z["mode"] == "one_shot": z["mode"] = "no_loop"
            held = off if heard else None
            t = 0
            while start + t < BAR_FRAMES:
                env = P11.level(z, t, held)
                if env is None: break
                got = P11.read(z, t, held, speed)
                if got is None: break
                (l, r), _ = got
                amp = f32(gain * f32(env))
                left[start + t] = f32(left[start + t] + f32(l * f32(amp * 1.0)))
                right[start + t] = f32(right[start + t] + f32(r * f32(amp * 1.0)))
                t += 1
    return left, right


def render(name):
    sound, s, notes = CASES[name]
    if sound in SET_ZONES: return set_render(sound, s, notes)
    return sampler_render(SOUNDS[sound], s, notes)


# ---------- the projects ----------
class Raw(str): pass


def sound_json(s):
    """The settings as the project file writes them: every field, the gate only when set."""
    out = {}
    for field in FIELDS:
        value = s[field]
        if field == "gate":
            if value is not None: out["gate"] = value
        elif isinstance(value, bool) or isinstance(value, int): out[field] = value
        else: out[field] = Raw(f32_text(value))
    return out


def f32_text(x):
    """An f32 as the project file writes it: its shortest digits that read back, in plain
    notation (20000.0, not 2e+04)."""
    return repr(float(P11.jf(x)))


def channel(name, case):
    sound, s, _ = CASES[case]
    if sound in SET_ZONES:
        c = P11.instrument(1, SET_NAMES[sound], SET_PATHS[sound])
    else:
        c = P11.sampler(1, name, "sound/%s.wav" % sound)
    c["sound"] = sound_json(s)
    return c


def case_project(name):
    _, _, notes = CASES[name]
    return P11.project([channel(name, name)], [P11.note_json(1, *n) for n in notes], version=5)


def dump(d):
    """Canonical JSON with f32 values written as the project file writes them."""
    text = J(d)
    raws = {n["velocity"] for p in d["patterns"] for n in p["notes"]}
    raws |= {v for c in d["channels"] for v in c.get("sound", {}).values() if isinstance(v, Raw)}
    for raw in sorted(raws, key=len, reverse=True):
        text = text.replace('":"%s"' % raw, '":%s' % raw)
    return text


def expected():
    renders = {}
    for name in CASES:
        left, right = render(name)
        answer = {"fixture": "p13_%s.json" % name, "frames": BAR_FRAMES}
        if name == "filter":
            answer["tolerance"] = FILTER_TOLERANCE
            answer["probes"] = [[i, left[i], right[i]] for i in range(0, 2000, 8)]
            answer["sounding"] = sum(1 for l in left if l != 0.0)
        else:
            answer["nonzero"] = P11.nonzero(left, right)
        renders[name] = answer
    return {"renders": renders}


def files():
    """Every file --write makes, by its path under DIR, as bytes."""
    out = {}
    for name, frames in SOUNDS.items():
        out["sound/%s.wav" % name] = P11.wav_bytes([frames])
    out["p13_shot/shot.sfz"] = SHOT_SFZ.encode()
    out["p13_shot/tone.wav"] = P11.wav_bytes([TONE])
    for name in CASES:
        out["p13_%s.json" % name] = (dump(case_project(name)) + "\n").encode()
    out["p13_expected.json"] = (J(expected()) + "\n").encode()
    return out


def main():
    args = sys.argv[1:]
    if len(args) == 2 and args[0] in ("--write", "--check"):
        made = files()
        stale = []
        for path, content in sorted(made.items()):
            target = os.path.join(args[1], path)
            if args[0] == "--write":
                os.makedirs(os.path.dirname(target), exist_ok=True)
                with open(target, "wb") as f: f.write(content)
            else:
                try:
                    with open(target, "rb") as f: same = f.read() == content
                except OSError: same = False
                if not same: stale.append(path)
        if stale: sys.exit("not what gen_p13_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    answers = expected()["renders"]
    for name, r in answers.items():
        if "nonzero" in r:
            first = r["nonzero"][:3]
            last = r["nonzero"][-1] if r["nonzero"] else None
            print("%-9s %d sounding; first %s; last %s" % (
                name, len(r["nonzero"]), ", ".join("%d: %.9g" % (f, l) for f, l, _ in first),
                last and "%d: %.9g" % (last[0], last[1])))
        else:
            print("%-9s %d sounding; probes %s" % (name, r["sounding"], r["probes"][:3]))
    print("cents speed %.17g, attack %d frames, release %d, note end %d" % (
        exp2(50 / 1200.0), frames_of(0.001), frames_of(0.002), t2s(2)))


if __name__ == "__main__":
    main()
