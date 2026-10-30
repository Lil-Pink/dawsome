"""Known answers for Cut itself: on a channel whose sound settings set "cut_itself", a note start
ends the notes the channel is still playing, so a fast 808 line plays one note at a time instead
of stacking. Renders through `dawsome render` (the pattern's notes) and through the player's MIDI
inbox (live notes, the sustain pedal), sample for sample.

Expected values come from the rules written out below (the design's decisions, restated as this
oracle computes them), never from engine code; zone reading, gains and mixing are
gen_p11_expected.py's rules and the sound settings' envelope gen_p13_expected.py's (restated
here, as gen_p22 does). Every input is authored here: the sine, its sets and the projects.

The rules:
  time      s(t) = floor(t*60*sr*1000 / (mbpm*ppq)) = floor(t * 229.6875) (44.1 kHz, 120 BPM, ppq
            96); a pattern note ends after its length: off = s(tick + length) - s(tick); a live
            note starts and ends on the first frame of the call its message comes before
            (F = 512 k), as gen_p22's
  the cut   a note start at frame F on a channel that cuts itself (one that plays it: velocity
            above 0, a channel that is not muted or silent for a pattern note) cuts every voice
            of that channel that began before F, in the pattern's pools and the live ones alike
            (one channel, one sound: a live note cuts a pattern note, a pattern note a live one);
            voices that begin on F itself (a chord on one tick) are kept. A voice keeps the
            first cut it is given, c = F - its start. The note's end the voice hears (`off`: a
            one-shot sampler hears none, nor a one-shot zone; gen_p13's gate rules) stands when
            it comes before c; otherwise the cut is its end:
              level(t) = rise(t)                                    for t < c
                       = rise(c) * exp2(-15 (t - c) / R)            for t - c < R, R > 0
                       = rise(c) * (D - (t - c)) / D                for t - c < D, R = 0
            then the voice ends; R is the release its note's end would fall over (the channel's
            release setting, else the zone's own ampeg_release, else 0: none for a sampler) and
            D = round(0.002 * sr) = 88 frames, a straight fall over 2 ms. Why D: a release of 0
            ends a voice on one frame, a step of up to the wave's height, which clicks
            (reproduced on 800af0b: a held sine ended from 0.295 to 0 in one frame, 12 times the
            wave's own largest step). rise(t) = t / A while t < A, else 1 (no attack here)
  pedal     a voice the sustain pedal holds is cut as any other; its key's note-off at the lift
            then changes nothing (the cut came first). The notes as played (the player's live
            note events, which a take records) are the same with the cut or without
  slots     each pool takes the first slot free at the note's frame (empty, or its voice ended
            by then: a cut voice still fading is not free), else the one that began first; on a
            channel that cuts itself a live sampler note never takes over its own key's voice in
            place (that would end it on one frame) but cuts it and takes a free slot
  mixing    out += value * f32(f32(f32(gain * f32(level)) * volume) * side) per voice, in f32, gain
            1 here (a set's zone at velocity byte 127, amp_veltrack 100; a sampler note's velocity
            1), the channel at volume 1 and centered (side exactly 1); each frame adds the voices
            pool by pool (the pattern's samplers, the pattern's zones, the live samplers, the live
            zones), slot by slot

  p26_sine/sine.wav     one second of 0.5 sin(2 pi 440 i / 44100), f32
  p26_sine/sine.sfz     one zone over every key on it, rooted on key 60, no loop, no release
  p26_sine/sine_release.sfz  the same with ampeg_release=0.01 (441 frames)
  p26_<case>.json       each case's project; p26_<case>_events.json its MIDI messages
  p26_expected.json     each render: its frames, SHA-256 (f32 little-endian, left then right),
                        sounding frames, probes, peak and most voices sounding at once

The cases (sequence ones through `dawsome render`, live ones through the player's inbox):
  overlap        two overlapping notes on a set, release 0.01: the first falls over 441 frames
                 from the second's start (overlap_nocut: the same without the flag, both ring)
  zone_release   the same on the set whose zone releases over 0.01 s, the channel at its
                 defaults but for the cut: the zone's own release
  chord          a three-note chord on one tick (all kept), then a note that cuts all three
  release0       the overlap with no release anywhere: the 2 ms straight fall
  sampler        a sampler (one-shot, no settings but the cut): a step that plays on past its
                 end until the next note cuts it, over 2 ms
  sampler_release  a sampler with a release of 0.01: the shaped sampler cut over 441 frames
  restrike       a note struck again on its own key: cut, then started anew, never doubled
  row            64 notes in a row, a 16th apart, each a beat long (row_nocut: four at a time)
  pedal          pedal down, note on, off (held by the pedal), another note: the held one is cut
                 (pedal_nocut: it sounds to the lift); the notes as played are the same
  cross          a live note cut by a pattern note, which a later live note cuts
  live_restrike  a live sampler key struck again: the first cut over 2 ms, not replaced

Run:
  python3 -I gen_p26_expected.py              print the known answers
  python3 -I gen_p26_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p26_expected.py --check DIR  exit 1 unless the files in DIR are what --write
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
f32, exp2, J, SR, PPQ, BUF, t2s = P11.f32, P11.exp2, P11.J, P11.SR, P11.PPQ, P11.BUF, P11.t2s
BAR = 4 * PPQ
BAR_FRAMES = t2s(BAR)
FADE = math.floor(0.002 * SR + 0.5)                  # D: 88 frames
RELEASE = 0.01                                       # every release here: 441 frames

# ---------- the sound and its sets ----------
SINE = [f32(0.5 * math.sin(2.0 * math.pi * 440.0 * i / SR)) for i in range(SR)]
SINE_SFZ = "<region> sample=sine.wav pitch_keycenter=60\n"
SINE_RELEASE_SFZ = "<region> sample=sine.wav pitch_keycenter=60 ampeg_release=0.01\n"
SET = "p26_sine/sine.sfz"
SET_RELEASE = "p26_sine/sine_release.sfz"
SAMPLE = "p26_sine/sine.wav"
ZONES = {SET: P11.zone(P11.region("sine.wav", center=60), [SINE]),
         SET_RELEASE: P11.zone(P11.region("sine.wav", center=60, release=RELEASE), [SINE]),
         # A sampler reads its sound as a zone rooted on key 60 over all of it reads it.
         SAMPLE: P11.zone(P11.region("sine.wav", center=60), [SINE])}

# ---------- the projects ----------
DEFAULT_SOUND = {"semitones": 0, "cents": 0, "velocity": 1.0, "attack": 0.0, "release": 0.0,
                 "reverse": False, "start": 0.0, "filter": False, "cutoff": 20000.0,
                 "resonance": 0.0}


class Channel:
    """One channel as a case plays it: its kind, its sound (a set's main file or a sample), its
    release setting (seconds, an f32 as the file holds it) and whether it cuts itself."""

    def __init__(self, kind, sound, release=0.0, cut=True):
        self.kind, self.sound, self.release, self.cut = kind, sound, release, cut

    def json(self):
        c = (P11.instrument(1, "Sine", self.sound) if self.kind == "instrument"
             else P11.sampler(1, "Sine", self.sound))
        settings = dict(DEFAULT_SOUND, release=self.release)
        if settings != DEFAULT_SOUND or self.cut:
            out = dict(settings)
            if self.cut: out["cut_itself"] = True
            c["sound"] = out
        return c

    def shaped(self):
        """Whether its settings shape its voices: any but the cut (which the voices' paths take
        apart, so a channel that only cuts plays its notes as before until one is cut)."""
        return self.release != 0.0

    def version(self):
        base = 4 if self.kind == "instrument" else 2
        return 5 if self.release != 0.0 or self.cut else base


def project(channel, notes, bars=1):
    d = P11.project([channel.json()], [P11.note_json(1, *n) for n in notes],
                    version=channel.version())
    d["patterns"][0]["length_bars"] = bars
    return d


def instrument(sfz=SET, release=RELEASE, cut=True): return Channel("instrument", sfz, release, cut)
def sampler(release=0.0, cut=True): return Channel("sampler", SAMPLE, release, cut)


# Pattern notes: (tick, length, key, velocity). Keys off 440 Hz's grid: a cut lands mid-wave.
OVERLAP = [(0, 96, 64, 1.0), (48, 48, 67, 1.0)]
ROW = [(24 * i, 96, 60 + i % 8, 1.0) for i in range(64)]
# case: (channel, pattern notes, bars, live messages or None)
CASES = {
    "overlap": (instrument(), OVERLAP, 1, None),
    "overlap_nocut": (instrument(cut=False), OVERLAP, 1, None),
    "zone_release": (instrument(SET_RELEASE, release=0.0), OVERLAP, 1, None),
    "chord": (instrument(), [(0, 192, 64, 1.0), (0, 192, 67, 1.0), (0, 192, 71, 1.0),
                             (96, 48, 62, 1.0)], 1, None),
    "release0": (instrument(release=0.0), OVERLAP, 1, None),
    "sampler": (sampler(), [(0, 24, 64, 1.0), (48, 48, 67, 1.0)], 1, None),
    "sampler_release": (sampler(release=RELEASE), OVERLAP, 1, None),
    "restrike": (instrument(), [(0, 96, 64, 1.0), (48, 48, 64, 1.0)], 1, None),
    "row": (instrument(), ROW, 4, None),
    "row_nocut": (instrument(cut=False), ROW, 4, None),
}


# ---------- the messages ----------
def on(key, v=127): return [0x90, key, v]
def off(key): return [0x80, key, 0]
def pedal(v): return [0xB0, 64, v]


PEDAL = [(2, pedal(127)), (4, on(64)), (10, off(64)), (20, on(67)), (24, off(67)),
         (40, pedal(0))]
LIVE = {
    "pedal": (instrument(), [], PEDAL),
    "pedal_nocut": (instrument(cut=False), [], PEDAL),
    "cross": (instrument(), [(24, 168, 67, 1.0)],
              [(1, on(64)), (20, on(71)), (30, off(71)), (60, off(64))]),
    "live_restrike": (sampler(), [], [(2, on(60)), (10, on(60)), (50, off(60))]),
}
# The projects each file holds: one per case, the live cases' patterns their own.
PROJECTS = {name: project(c, notes, bars) for name, (c, notes, bars, _) in CASES.items()}
PROJECTS.update({name: project(c, notes) for name, (c, notes, _) in LIVE.items()})


def events_json(script):
    return {"buffer": BUF, "events": [{"call": call, "bytes": m} for call, m in script]}


# ---------- the player, restated ----------
POOLS = {"pattern_sampler": (0, 64), "pattern_zone": (1, 128), "live_sampler": (2, 16),
         "live_zone": (3, 32)}


def rise(attack, t): return 1.0 if attack == 0 or t >= attack else t / attack


def envelope(attack, release, t, off):
    """gen_p13's: the attack's rise, then from the note's end `off` a fall of 15 halvings over
    `release` frames; None once released (at `off` itself with no release)."""
    if off is None or t < off: return rise(attack, t)
    r = t - off
    if r >= release: return None
    return rise(attack, off) * exp2(-(15 * r / release))


def cut_stands(off, cut): return cut is not None and (off is None or cut <= off)


def level(attack, release, t, off, cut):
    """The level of a voice `t` frames in whose heard note's end is `off` and which was cut at
    `cut` (None: never), as the rule above; None once it has ended."""
    if not cut_stands(off, cut): return envelope(attack, release, t, off)
    if t < cut or release > 0: return envelope(attack, release, t, cut)
    r = t - cut
    if r >= FADE: return None
    return rise(attack, cut) * (FADE - r) / FADE


class Voice:
    def __init__(self, channel, pool, slot, start, key, velocity, off):
        self.channel, self.pool, self.slot, self.start, self.key = channel, pool, slot, start, key
        self.off, self.cut = off, None
        self.zone = ZONES[channel.sound]
        self.speed = P11.semitone_speed(key - self.zone["center"])
        # A zone's gain at velocity byte 127 is exactly 1; a sampler plays at the velocity.
        self.gain = 1.0 if channel.kind == "instrument" else f32(velocity)

    def heard_off(self):
        """The note's end it hears: a zone with no loop (no_loop) hears it, a one-shot sampler
        only with a release (gen_p13's gate rules; no gate is set here)."""
        if self.channel.kind == "instrument": return self.off
        return self.off if self.channel.release != 0.0 else None

    def envelope_frames(self):
        z = self.zone
        if self.channel.kind == "instrument":
            mine = P11.frames_of(f32(self.channel.release))
            return z["attack"], (mine if mine > 0 else z["release"])
        return 0, P11.frames_of(f32(self.channel.release))

    def at(self, t):
        """(value, level) at `t` frames in, or None once it has ended."""
        attack, release = self.envelope_frames()
        off = self.heard_off()
        env = level(attack, release, t, off, self.cut)
        if env is None: return None
        end = self.cut if cut_stands(off, self.cut) else off
        got = P11.read(self.zone, t, end, self.speed)
        if got is None: return None
        return got[0], env

    def sounds_at(self, frame): return self.at(frame - self.start) is not None


class Player:
    """The voices a case's notes and messages start, slot by slot, and the notes as played."""

    def __init__(self, channel):
        self.channel = channel
        self.slots = {pool: [None] * size for pool, (_, size) in POOLS.items()}
        self.voices, self.heard = [], []

    def start(self, pool, frame, key, velocity, off):
        if self.channel.cut:
            for voice in self.voices:
                if voice.start < frame and voice.cut is None: voice.cut = frame - voice.start
        slots = self.slots[pool]
        free = [i for i, v in enumerate(slots) if v is None or not v.sounds_at(frame)]
        assert free, "no case fills a pool"
        voice = Voice(self.channel, pool, free[0], frame, key, velocity, off)
        slots[free[0]] = voice
        self.voices.append(voice)
        return voice

    def play(self, notes, script):
        """Pattern notes from the first pass and live messages, in time order (a live message
        first on a frame both fall on: it is taken before the call renders)."""
        pool = "pattern_zone" if self.channel.kind == "instrument" else "pattern_sampler"
        live_pool = "live_zone" if self.channel.kind == "instrument" else "live_sampler"
        timed = [(start, 1, ("note", key, v, gate)) for start, gate, key, v in P11.pattern_notes(notes)]
        timed += [(call * BUF, 0, ("live", m)) for call, m in (script or [])]
        held, deferred, down = {}, [], False

        def end(key, frame):
            voice = held.pop(key)
            voice.off = frame - voice.start
            self.heard.append((key, voice.start, frame))

        for frame, _, what in sorted(timed, key=lambda e: (e[0], e[1])):
            if what[0] == "note":
                self.start(pool, frame, what[1], what[2], what[3])
                continue
            m = what[1]
            status, data = m[0] & 0xF0, m[1:]
            if status == 0x90 and data[1] > 0:
                key = data[0]
                if key in held:
                    if key in deferred: deferred.remove(key)
                    end(key, frame)
                held[key] = self.start(live_pool, frame, key, data[1] / 128.0, None)
            elif status in (0x80, 0x90):
                key = data[0]
                if key in held and key not in deferred:
                    if down: deferred.append(key)
                    else: end(key, frame)
            elif status == 0xB0 and data[0] == 64:
                now = data[1] >= 64
                if down and not now:
                    for key in list(deferred): end(key, frame)
                    deferred.clear()
                down = now
        return self

    def render(self, frames):
        left, right = [0.0] * frames, [0.0] * frames
        counts = [0] * frames
        order = sorted(self.voices, key=lambda v: (POOLS[v.pool][0], v.slot, v.start))
        for voice in order:
            t = 0
            while voice.start + t < frames:
                got = voice.at(t)
                if got is None: break
                (l, r), env = got
                gain = f32(f32(f32(voice.gain * f32(env)) * 1.0) * 1.0)
                k = voice.start + t
                left[k] = f32(left[k] + f32(l * gain))
                right[k] = f32(right[k] + f32(r * gain))
                counts[k] += 1
                t += 1
        return left, right, max(counts)


def sha256_of(left, right):
    data = struct.pack("<%df" % (2 * len(left)), *[x for pair in zip(left, right) for x in pair])
    return hashlib.sha256(data).hexdigest()


def probes_at(player, frames):
    """Frames worth naming: every 1000th, the last, and around each voice's start, cut, end of
    note and where its cut's release or fall ends."""
    at = set(range(0, frames, 1000)) | {frames - 1}
    for v in player.voices:
        marks = [v.start]
        if v.cut is not None: marks += [v.start + v.cut, v.start + v.cut + FADE,
                                        v.start + v.cut + 441]
        if v.off is not None: marks.append(v.start + v.off)
        for m in marks: at |= {m - 1, m, m + 1}
    return sorted(k for k in at if 0 <= k < frames)


def case_answer(name, channel, notes, bars, script):
    frames = bars * BAR_FRAMES
    player = Player(channel).play(notes, script)
    left, right, most = player.render(frames)
    answer = {"fixture": "p26_%s.json" % name, "frames": frames, "bars": bars,
              "sha256": sha256_of(left, right),
              "sounding": sum(1 for l, r in zip(left, right) if l != 0.0 or r != 0.0),
              "peak": max(max(abs(x) for x in left), max(abs(x) for x in right)),
              "voices_most": most,
              "cuts": [[v.start, v.cut] for v in player.voices if v.cut is not None],
              "probes": [[k, left[k], right[k]] for k in probes_at(player, frames)]}
    if script is not None:
        answer["events"] = "p26_%s_events.json" % name.replace("_nocut", "")
        answer["heard"] = [list(n) for n in player.heard]
    return answer


def expected():
    renders = {name: case_answer(name, c, notes, bars, None)
               for name, (c, notes, bars, _) in CASES.items()}
    live = {name: case_answer(name, c, notes, 1, script)
            for name, (c, notes, script) in LIVE.items()}
    return {"renders": renders, "live": live,
            "rules": {"fade_frames": FADE, "release_frames": P11.frames_of(f32(RELEASE))}}


def files():
    """Every file --write makes, by its path under DIR, as bytes."""
    out = {"p26_sine/sine.wav": P11.wav_bytes([SINE]),
           "p26_sine/sine.sfz": SINE_SFZ.encode(),
           "p26_sine/sine_release.sfz": SINE_RELEASE_SFZ.encode()}
    for name, doc in PROJECTS.items():
        out["p26_%s.json" % name] = (P11.dump(doc) + "\n").encode()
    for name, (_, _, script) in LIVE.items():
        if not name.endswith("_nocut"):
            out["p26_%s_events.json" % name] = (J(events_json(script)) + "\n").encode()
    out["p26_expected.json"] = (J(expected()) + "\n").encode()
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
        if stale: sys.exit("not what gen_p26_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    answers = expected()
    print("fade %d frames, release %d frames" % (answers["rules"]["fade_frames"],
                                                 answers["rules"]["release_frames"]))
    for group in ("renders", "live"):
        for name, r in answers[group].items():
            print("%-16s %6d sounding, peak %.6f, at most %d voices, cuts %s" % (
                name, r["sounding"], r["peak"], r["voices_most"],
                [tuple(c) for c in r["cuts"]][:4] + (["..."] if len(r["cuts"]) > 4 else [])))


if __name__ == "__main__":
    main()
