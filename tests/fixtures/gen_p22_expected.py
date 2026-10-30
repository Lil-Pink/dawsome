"""Known answers for the sustain pedal, pitch bend and the mod wheel: what a device's CC 64, pitch
bend and CC 1 do to the notes it plays live, through `--play-to --midi-events` renders and the
notes a take records, and what a channel with a MIDI out sends on.

Expected values come from the rules written out below (the design's decisions, restated as this
oracle computes them), never from engine code; zone reading, gains and mixing are
gen_p11_expected.py's rules (imported, as gen_p13_expected.py does). Every input is authored
here: the sine and its set, the projects and the MIDI messages.

The rules:
  time      s(t) = floor(t*60*sr*1000 / (mbpm*ppq)) = floor(t * 229.6875) here (44.1 kHz, 120 BPM,
            ppq 96); the playhead's tick at sample s is the last tick that starts at or before
            it; the player is called with 512 frames and a message timed for call k is taken at
            that call's first frame, F = 512 k: the live notes' rule, and the bend's and the
            wheel's (a change holds from F until the next; nothing depends on how a call is cut)
  sustain   CC 64 at 64 or more is down, below 64 up. While an input's pedal is down on a
            channel, a key of that input let go of there sounds on until the pedal lifts; the
            pedal let go of ends every such key, in the order they were let go of; a key struck
            again while the pedal holds it ends there and starts anew. A note's end, and so a
            take's note, is where it stopped sounding: the pedal's lift
  bend      E0 ll mm: b = ll + 128 mm - 8192 (-8192..8191); semitones st = R*b/8191 for b >= 0,
            R*b/8192 below, R the channel's bend range (2, or its "bend_range" 1..24);
            ratio = exp2(st / 12) (exp2: + - * / only, gen_p11's), exactly 1 at b = 0
  pitch     a live voice reads its sound at p(t) = first + P(t), t its frames since it began:
            P(t) = base + (t - from)*rate + A*sin(w*t), rate = speed*ratio, w = 2*pi*5/sr, the
            sine left out while A = 0; it begins with base = 0, from = 0 and the input's bend and
            wheel as they are then. Where the bend or the wheel moves (frame T of the voice):
            base = P_old(T) - A_new*sin(w*T), from = T, so it reads on from where it was. Only
            live voices follow; a pattern's notes do not
  wheel     CC 1 = v (0..127). On a channel whose low-pass filter is on: the voices' filter
            coefficients from cutoff' = min(cutoff * exp2(4v/127), 20000) Hz (the wheel opens the
            filter by up to 4 octaves; v = 0 is the channel's own cutoff) from the frame it moves
            on, the filter's memory going on. On any other channel: vibrato of c = 50v/127 cents
            at 5 Hz, A = rate*(exp2(c/1200) - 1)/w (rate peaks c cents up)
  filter    the cookbook low-pass of gen_p13 (Q = 1/sqrt 2 at resonance 0), f64 Direct Form I
            on the zone's value; sin and cos make its coefficients, and the vibrato's sine is
            sin: those renders hold to 1e-6, every other to the bit
  forward   on a channel with a MIDI out, the pedal, the bend and the wheel go on as the same
            messages (the same value) on its MIDI channel at F (the live notes' rule: no
            latency), only when they change the value the input holds there (an instrument
            sending back what it was sent stops there); the pedal's lift goes out before the note
            offs of the keys it held, which the engine holds for every channel (a take then
            records the notes as played)
  links     a controller link on CC 1 or CC 64 wins: the message moves the linked parameter
            (min + (max - min)*v/127, gen_p9's rule) and plays nothing else
  mixing    a live zone voice: out += value * (f32(gain * env) * volume * side) in f32, gain 1
            here (velocity byte 127, amp_veltrack 100: (127/127)^2), side the channel pan law's
            gain (exactly 1 centered, 0 and sqrt 2 hard right)

  p22_sine/           the set "sine": one zone over every key, sine.wav (one second of
                      0.5 sin(2 pi 440 i / 44100), f32) rooted on key 69, no loop, no release
  p22_sine.json       channel 1 "Sine" playing the set, a one-bar pattern with no notes
  p22_filter.json     the same with its filter on at 1 kHz
  p22_filter_hand.json  the same with its filter set by hand to 16 kHz
  p22_range.json      the same with a bend range of 12 semitones
  p22_link.json       p22_sine with CC 64 linked to its volume and CC 1 to its pan
  p22_out.json        channel 1 "Synth", an input channel with a MIDI out (MIDI channel 1)
  p22_*_events.json   each case's messages (`--midi-events`)
  p22_expected.json   each case: its inputs and what the tests compare

The cases (the spec's five, the MIDI out's and the links'):
  (1) pedal      pedal down, note on, note off, pedal up: the note ends on the lift; the render
                 equals "held" (the note held to that call); the take holds that length
  (2) repress    pedal down, note on, off, the same key again, off, pedal up: the first voice
                 ends where the key is struck again, the second on the lift; two notes taken
  (3) bend       +8191 then -8192 then 0 under a held note at range 2 (and "range": 12)
  (4) wheel      127 before a note on a filtered channel (equal to the hand-set filter up to
                 where it moves back to 0), and 127 under a held note on an unfiltered one
  (5) saturated  pedal up with no note, a bend with no voice (the next note starts bent), CC 64
                 at 63 (up) and 64 (down)
  (6) out        a MIDI out sends the pedal, the bend and the wheel on; a repeat sends nothing; a
                 quiet port's pedal sends nothing
  (7) link       CC 64 and CC 1 linked: no sustain, no vibrato, the volume and the pan move

Run:
  python3 -I gen_p22_expected.py              print the known answers
  python3 -I gen_p22_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p22_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import hashlib, importlib.util, math, os, struct, sys

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
f32, exp2, J, SR, PPQ, BUF = P11.f32, P11.exp2, P11.J, P11.SR, P11.PPQ, P11.BUF
MBPM = P11.MBPM
BAR = 4 * PPQ
FRAMES = P11.t2s(BAR)                                # one bar: what every render plays
TOLERANCE = 1e-6
DEFAULT_RANGE = 2
VIBRATO_HZ = 5.0
WHEEL_OCTAVES = 4.0
VIBRATO_CENTS = 50.0
TOP_CUTOFF = 20000.0
KEY = 69                                             # the zone's root: speed 1
VELOCITY = 127                                       # byte 127: gain 1


def first_tick_at_or_after(s): return -(-s * MBPM * PPQ // (60 * SR * 1000))
def tick_at(s): return first_tick_at_or_after(s + 1) - 1   # the playhead's tick at sample s


# ---------- the sound and its set ----------
SINE = [f32(0.5 * math.sin(2.0 * math.pi * 440.0 * i / SR)) for i in range(SR)]
SINE_SFZ = "<region> sample=sine.wav pitch_keycenter=69\n"
SET_PATH = "p22_sine/sine.sfz"
ZONE = P11.zone(P11.region("sine.wav", center=KEY), [SINE])

# ---------- the projects ----------
SOUND_FIELDS = ["semitones", "cents", "velocity", "attack", "release", "reverse", "start",
                "filter", "cutoff", "resonance"]
DEFAULT_SOUND = {"semitones": 0, "cents": 0, "velocity": 1.0, "attack": 0.0, "release": 0.0,
                 "reverse": False, "start": 0.0, "filter": False, "cutoff": 20000.0,
                 "resonance": 0.0}


def sound(filter_=False, cutoff=20000.0, bend_range=DEFAULT_RANGE):
    """Sound settings as the file writes them: every field, `bend_range` only when not 2."""
    s = dict(DEFAULT_SOUND, filter=filter_, cutoff=cutoff)
    out = {field: s[field] for field in SOUND_FIELDS}
    if bend_range != DEFAULT_RANGE: out["bend_range"] = bend_range
    return out


def sine_channel(**settings):
    c = P11.instrument(1, "Sine", SET_PATH)
    if settings: c["sound"] = sound(**settings)
    return c


def project(channels, version, links=None):
    d = P11.project(channels, [], version=version)
    if links: d["midi_links"] = links
    return d


SYNTH_PORT = "Synth:Synth MIDI 1"
PROJECTS = {
    "p22_sine.json": project([sine_channel()], 4),
    "p22_filter.json": project([sine_channel(filter_=True, cutoff=1000.0)], 5),
    "p22_filter_hand.json": project([sine_channel(filter_=True, cutoff=16000.0)], 5),
    "p22_range.json": project([sine_channel(bend_range=12)], 5),
    "p22_link.json": project([sine_channel()], 4, links=[
        {"cc": 64, "channel": None, "target": {"channel_volume": 1}},
        {"cc": 1, "channel": None, "target": {"channel_pan": 1}}]),
    "p22_out.json": project([{"id": 1, "name": "Synth", "kind": "input",
                              "input": {"device": "", "channels": {"stereo": [0, 1]}},
                              "volume": 1.0, "pan": 0.0, "mute": False,
                              "midi_out": {"port": SYNTH_PORT, "channel": 1, "latency_ms": 0}}], 7),
}

# ---------- the messages ----------
def on(key=KEY, v=VELOCITY): return [0x90, key, v]
def off(key=KEY): return [0x80, key, 0]
def pedal(v): return [0xB0, 64, v]
def wheel(v): return [0xB0, 1, v]
def bend(b):
    raw = b + 8192
    return [0xE0, raw & 0x7F, raw >> 7]


QUIET = "Synth:Synth MIDI 1 24:0"                   # the MIDI out's own port: echo off, quiet
EVENTS = {
    "pedal": [(2, pedal(127)), (4, on()), (10, off()), (40, pedal(0))],
    "held": [(4, on()), (40, off())],
    "repress": [(2, pedal(127)), (4, on()), (10, off()), (20, on()), (24, off()),
                (40, pedal(0))],
    "bend": [(4, on()), (20, bend(8191)), (40, bend(-8192)), (50, bend(0)), (60, off())],
    "wheel": [(2, wheel(127)), (4, on()), (30, wheel(0)), (60, off())],
    "vibrato": [(4, on()), (10, wheel(127)), (60, off())],
    "saturated": [(1, pedal(0)), (2, bend(4096)), (4, pedal(63)), (6, on()), (10, off()),
                  (12, pedal(64)), (14, on()), (16, off()), (30, pedal(63))],
    "out": [(1, pedal(127)), (2, on(60, 100)), (3, bend(8191)), (4, wheel(100)), (5, off(60)),
            (6, pedal(127)), (7, bend(0)), (8, pedal(0)), (9, wheel(0)), (10, pedal(127), QUIET)],
    "link": [(2, pedal(127)), (2, wheel(127)), (4, on()), (20, off())],
}


def events_json(script):
    rows = []
    for item in script:
        row = {"call": item[0], "bytes": item[1]}
        if len(item) > 2: row["port"] = item[2]
        rows.append(row)
    return {"buffer": BUF, "events": rows}


# ---------- the player, restated ----------
def bend_ratio(b, bend_range):
    if b == 0: return 1.0
    st = float(bend_range) * float(b) / (8191.0 if b >= 0 else 8192.0)
    return exp2(st / 12.0)


OMEGA = 2.0 * math.pi * VIBRATO_HZ / SR


def vibrato_depth(rate, v):
    if v == 0: return 0.0
    cents = VIBRATO_CENTS * float(v) / 127.0
    return rate * (exp2(cents / 1200.0) - 1.0) / OMEGA


def low_pass(cutoff, resonance):
    """gen_p13's cookbook low-pass at `cutoff` Hz (an f64 here: the wheel's is not an f32)."""
    fs = float(SR)
    cutoff = min(cutoff, 0.45 * fs)
    q = 1 / math.sqrt(2) + resonance * (10.0 - 1 / math.sqrt(2))
    w0 = 2.0 * math.pi * cutoff / fs
    sin, cos = math.sin(w0), math.cos(w0)
    alpha = sin / (2.0 * q)
    a0 = 1.0 + alpha
    b1 = 1.0 - cos
    b0 = b1 / 2.0
    return [b0 / a0, b1 / a0, b0 / a0, -2.0 * cos / a0, (1.0 - alpha) / a0]


def wheel_cutoff(cutoff, v):
    return min(float(f32(cutoff)) * exp2(WHEEL_OCTAVES * float(v) / 127.0), TOP_CUTOFF)


class Pitch:
    """Where a live voice reads: P(t) = base + (t - from)*rate + depth*sin(w t)."""

    def __init__(self, rate, depth):
        self.rate, self.base, self.frm, self.depth = rate, 0.0, 0, depth

    def at(self, t):
        p = self.base + float(t - self.frm) * self.rate
        if self.depth != 0.0: p += self.depth * math.sin(OMEGA * float(t))
        return p

    def retarget(self, t, rate, depth):
        here = self.at(t)
        self.base = here - (depth * math.sin(OMEGA * float(t)) if depth != 0.0 else 0.0)
        self.frm, self.rate, self.depth = t, rate, depth


class Voice:
    def __init__(self, start, rate, depth, coefficients):
        self.start, self.off = start, None
        self.pitch = Pitch(rate, depth)
        self.changes = []                 # (t, rate, depth): the bend's and the wheel's moves
        self.coefficients = [(0, coefficients)]     # (t, coefficients or None) from t on


class Channel:
    """One channel's settings as the cases play it."""

    def __init__(self, bend_range=DEFAULT_RANGE, cutoff=None):
        self.bend_range = bend_range
        self.cutoff = cutoff              # its filter's cutoff while the filter is on


def play(script, channel, links=False):
    """The voices `script`'s messages start on one channel from one input (the door with no
    port), as the player's rules above say; with `links`, CC 1 and CC 64 move the linked
    parameters instead. Returns the voices and the notes as heard: (key, on frame, off frame)."""
    voices, held, deferred, heard = [], {}, [], []
    down, b, v = False, 0, 0
    speed = P11.semitone_speed(KEY - ZONE["center"])

    def tone(v):
        if channel.cutoff is None: return None
        return low_pass(wheel_cutoff(channel.cutoff, v), 0.0)

    def depth_of(rate, v):
        return 0.0 if channel.cutoff is not None else vibrato_depth(rate, v)

    def end(key, frame):
        voice, at = held.pop(key)
        voice.off = frame - voice.start
        heard.append((key, at, frame))

    for item in script:
        call, m = item[0], item[1]
        frame = call * BUF
        status, data = m[0] & 0xF0, m[1:]
        if status == 0x90 and data[1] > 0:
            key = data[0]
            if key in held:
                if key in deferred: deferred.remove(key)
                end(key, frame)
            rate = speed * bend_ratio(b, channel.bend_range)
            voice = Voice(frame, rate, depth_of(rate, v), tone(v) if v else None)
            voices.append(voice)
            held[key] = (voice, frame)
        elif status in (0x80, 0x90):
            key = data[0]
            if key in held and key not in deferred:
                if down: deferred.append(key)
                else: end(key, frame)
        elif status == 0xB0 and data[0] in (1, 64) and links:
            continue                      # moves its linked parameter: the window's business
        elif status == 0xB0 and data[0] == 64:
            now = data[1] >= 64
            if down and not now:
                for key in list(deferred): end(key, frame)
                deferred.clear()
            down = now
        elif status == 0xE0 or (status == 0xB0 and data[0] == 1):
            if status == 0xE0: nb, nv = data[0] + 128 * data[1] - 8192, v
            else: nb, nv = b, data[1]
            if (nb, nv) == (b, v): continue
            moved_wheel = nv != v
            b, v = nb, nv
            for voice in voices:
                if voice.off is not None and voice.start + voice.off <= frame: continue
                t = frame - voice.start
                rate = speed * bend_ratio(b, channel.bend_range)
                voice.changes.append((t, rate, depth_of(rate, v)))
                if moved_wheel and channel.cutoff is not None:
                    voice.coefficients.append((t, tone(v) if v else None))
    return voices, heard


def render(voices, channel, volume=1.0, pan=0.0):
    """The voices' sound over one bar, left and right, as gen_p11 mixes a zone voice."""
    base = low_pass(float(f32(channel.cutoff)), 0.0) if channel.cutoff is not None else None
    sides = P11.pan_gains(pan) if pan != 0.0 else [1.0, 1.0]
    left, right = [0.0] * FRAMES, [0.0] * FRAMES
    for voice in voices:
        changes = list(voice.changes)
        switches = list(voice.coefficients)
        memory = [[0.0] * 4, [0.0] * 4]
        coefficients = base
        t = 0
        while voice.start + t < FRAMES:
            if voice.off is not None and t >= voice.off: break   # no release: ends at its end
            while changes and changes[0][0] == t:
                _, rate, depth = changes.pop(0)
                voice.pitch.retarget(t, rate, depth)
            while switches and switches[0][0] == t:
                _, own = switches.pop(0)
                coefficients = own if own is not None else base
            got = P11.value_at(ZONE, ZONE["first"] + voice.pitch.at(t), False, False)
            if got is None: break
            values = list(got)
            if coefficients is not None:
                b0, b1, b2, a1, a2 = coefficients
                for side in (0, 1):
                    x1, x2, y1, y2 = memory[side]
                    y = b0 * values[side] + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2
                    if abs(y) < 1e-30: y = 0.0
                    memory[side] = [values[side], x1, y, y1]
                    values[side] = f32(y)
            gain = f32(f32(1.0 * 1.0) * f32(volume))
            at = voice.start + t
            left[at] = f32(left[at] + f32(values[0] * f32(gain * sides[0])))
            right[at] = f32(right[at] + f32(values[1] * f32(gain * sides[1])))
            t += 1
    return left, right


CHANNELS = {
    "p22_sine.json": Channel(),
    "p22_filter.json": Channel(cutoff=1000.0),
    "p22_filter_hand.json": Channel(cutoff=16000.0),
    "p22_range.json": Channel(bend_range=12),
    "p22_link.json": Channel(),
}

# case: (project, events, tolerance?) for every render the tests compare
RENDERS = {
    "pedal": ("p22_sine.json", "pedal", False),
    "held": ("p22_sine.json", "held", False),
    "repress": ("p22_sine.json", "repress", False),
    "bend": ("p22_sine.json", "bend", False),
    "range": ("p22_range.json", "bend", False),
    "wheel": ("p22_filter.json", "wheel", True),
    "wheel_hand": ("p22_filter_hand.json", "held_long", True),
    "vibrato": ("p22_sine.json", "vibrato", True),
    "saturated": ("p22_sine.json", "saturated", False),
    "link": ("p22_link.json", "link", False),
}
# The hand-set filter's note: on and off where the wheel's note is.
EVENTS["held_long"] = [(4, on()), (60, off())]
# Where the wheel case equals the hand-set filter: up to the call the wheel moves back to 0.
WHEEL_SAME_UNTIL = 30 * BUF


def link_levels():
    """CC 64 = 127 moves the volume to 0 + 4*127/127, CC 1 = 127 the pan to -1 + 2*127/127."""
    return f32(0.0 + (4.0 - 0.0) * 127 / 127.0), f32(-1.0 + (1.0 - -1.0) * 127 / 127.0)


def case_render(name):
    fixture, events, _ = RENDERS[name]
    channel = CHANNELS[fixture]
    links = name == "link"
    voices, heard = play(EVENTS[events], channel, links=links)
    if links:
        volume, pan = link_levels()
        return render(voices, channel, volume, pan), heard
    return render(voices, channel), heard


def sha256_of(left, right):
    data = struct.pack("<%df" % (2 * FRAMES), *[x for pair in zip(left, right) for x in pair])
    return hashlib.sha256(data).hexdigest()


def probes_at(script):
    """Frames worth naming in a render: every 1000th, and each message's frame and the frame
    before and after it."""
    at = set(range(0, FRAMES, 1000)) | {FRAMES - 1}
    for item in script:
        frame = item[0] * BUF
        at |= {frame - 1, frame, frame + 1}
    return sorted(k for k in at if 0 <= k < FRAMES)


def take_notes(heard):
    """The notes a pattern take (no quantize, no count-in) writes for what was heard: start =
    the tick its note on was heard at, length as played, velocity as struck."""
    notes = []
    for key, start, end in heard:
        t0, t1 = tick_at(start), tick_at(end)
        n = {"channel": 1, "tick": t0, "length": t1 - t0, "velocity": VELOCITY / 128}
        if key != 60: n["key"] = key
        notes.append(n)
    return sorted(notes, key=lambda n: (n["tick"], n["channel"], n.get("key", 60)))


def forwarded(script):
    """What the MIDI out of p22_out's channel 1 sends: each message at its call's first frame
    as bytes in hex, as `--midi-out-log` writes them."""
    sent, held, deferred, quiet = [], {}, [], {}
    state = {"pedal": 0, "bend": 0, "wheel": 0}
    hexed = lambda bytes_: " ".join("%02X" % x for x in bytes_)
    for item in script:
        call, m = item[0], item[1]
        frame = call * BUF
        if len(item) > 2:                 # a quiet port: its own state, nothing sent
            quiet[m[1]] = m[2]
            continue
        status, data = m[0] & 0xF0, m[1:]
        if status == 0x90 and data[1] > 0:
            sent.append((frame, hexed([0x90, data[0], data[1]])))
            held[data[0]] = True
        elif status == 0x80:
            if state["pedal"] >= 64: deferred.append(data[0])
            else: sent.append((frame, hexed([0x80, data[0], 0x40])))
        elif status == 0xB0 and data[0] == 64:
            if data[1] != state["pedal"]:
                sent.append((frame, hexed(m)))
                lifted = state["pedal"] >= 64 and data[1] < 64
                state["pedal"] = data[1]
                if lifted:
                    for key in deferred: sent.append((frame, hexed([0x80, key, 0x40])))
                    deferred.clear()
        elif status == 0xB0 and data[0] == 1:
            if data[1] != state["wheel"]:
                sent.append((frame, hexed(m)))
                state["wheel"] = data[1]
        elif status == 0xE0:
            value = data[0] + 128 * data[1] - 8192
            if value != state["bend"]:
                sent.append((frame, hexed(m)))
                state["bend"] = value
    return [{"frame": frame, "channel": 1, "bytes": text} for frame, text in sent]


def expected():
    renders = {}
    for name, (fixture, events, tolerance) in RENDERS.items():
        (left, right), heard = case_render(name)
        answer = {"fixture": fixture, "events": "p22_%s_events.json" % events, "frames": FRAMES,
                  "sounding": sum(1 for l, r in zip(left, right) if l != 0.0 or r != 0.0)}
        if tolerance:
            answer["tolerance"] = TOLERANCE
            answer["probes"] = [[k, left[k], right[k]] for k in range(0, FRAMES, 32)
                                if left[k] != 0.0 or right[k] != 0.0]
        else:
            answer["sha256"] = sha256_of(left, right)
            answer["probes"] = [[k, left[k], right[k]] for k in probes_at(EVENTS[events])]
        if name in ("pedal", "repress"):
            answer["take"] = take_notes(heard)
        renders[name] = answer
    volume, pan = link_levels()
    return {"renders": renders,
            "wheel_same_until": WHEEL_SAME_UNTIL,
            "link": {"volume": volume, "pan": pan},
            "out": {"fixture": "p22_out.json", "events": "p22_out_events.json",
                    "messages": forwarded(EVENTS["out"])},
            "rules": {"bend_ratio_max_range_2": bend_ratio(8191, 2),
                      "bend_ratio_min_range_2": bend_ratio(-8192, 2),
                      "bend_ratio_4096_range_2": bend_ratio(4096, 2),
                      "bend_ratio_max_range_12": bend_ratio(8191, 12),
                      "wheel_cutoff_127_from_1000": wheel_cutoff(1000.0, 127),
                      "vibrato_cents_127": VIBRATO_CENTS}}


def files():
    """Every file --write makes, by its path under DIR, as bytes."""
    out = {"p22_sine/sine.wav": P11.wav_bytes([SINE]),
           "p22_sine/sine.sfz": SINE_SFZ.encode()}
    for name, doc in PROJECTS.items():
        out[name] = (J(doc) + "\n").encode()
    for name, script in EVENTS.items():
        out["p22_%s_events.json" % name] = (J(events_json(script)) + "\n").encode()
    out["p22_expected.json"] = (J(expected()) + "\n").encode()
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
        if stale: sys.exit("not what gen_p22_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    answers = expected()
    for name, r in answers["renders"].items():
        print("%-10s %s + %s: %d sounding%s" % (
            name, r["fixture"], r["events"], r["sounding"],
            "; take %s" % [(n["tick"], n["length"]) for n in r["take"]] if "take" in r else ""))
    print("out:      ", [(m["frame"], m["bytes"]) for m in answers["out"]["messages"]])
    for rule, value in answers["rules"].items():
        print("%-28s %.17g" % (rule, value))


if __name__ == "__main__":
    main()
