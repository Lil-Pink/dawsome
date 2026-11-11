"""Known answers for Jam: a pattern written for the instruments on the rack from a genre's table,
the project's key and a seed. `dawsome jam` runs each request below and must print exactly the
pattern this file works out, and save the project with it.

Expected values come from these rules, never from engine code. Every genre's numbers (the
tables the rules draw from) are read from crates/engine/src/jam/genres.json, the file the engine
is built with: the one place they are tuned, so a number is never written twice.
  PRNG       xorshift64*, its state s = splitmix64(seed ^ salt) (splitmix64(x): z = x +
             0x9E3779B97F4A7C15; z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9; z = (z ^ (z >> 27)) *
             0x94D049BB133111EB; z ^ (z >> 31), all mod 2^64), the salt (the genre's place in lofi,
             boombap, trap, drill, house, phonk, afrobeats, from 1) * 0x9E3779B97F4A7C15, and
             0x9E3779B97F4A7C15 in the one case splitmix gives 0 (xorshift would stay at 0);
             next_u32: s ^= s >> 12; s ^= s << 25 (64 bits); s ^= s >> 27;
             (s * 0x2545F4914F6CDD1D mod 2^64) >> 32. below(n) = next_u32() % n;
             chance(p) = below(100) < p; jitter(r) = below(r)
  order      the progression first (below(count) over the key family's progressions), then the
             tempo (low + below(high - low + 1) BPM over the genre's tempo range), then one
             variant of each part in the order kick, snare, clap, closed hat, open hat, perc,
             chords, bass, melody (below(n) over its variants; no draw for a part with one),
             then each channel in rack order, each bar left to right; drums: the voices kick,
             snare, clap, closed hat, open hat, perc (a voice the channel's map lacks draws
             nothing); chords, bass and melody: their own draws per bar. No draw for a value the
             table fixes
  hits       a part's variant is a list of parts: a list of hits; "either" (one chance draw for the
             bar picks one of two lists); "last bar" (only on the pattern's last bar). Each hit
             draws, in the list's order: its chance (hit when chance(p)) or its drop (no hit when
             chance(p)), nothing when fixed; then, when it plays, its velocity: base - jitter(16)
             hundredths, or a fixed velocity with no draw. Every velocity is then held to
             0.30..1.00
  grid       16 steps a bar of 4/4, a step ppq/4 ticks (24 at 96), a half step ppq/8 (12); a
             Jam needs a PPQ that is a multiple of 8, so a half step is whole ticks (the edit and
             the command refuse any other, and every project here is at 96)
  key        scale steps: major 0 2 4 5 7 9 11, minor 0 2 3 5 7 8 10, dorian 0 2 3 5 7 9 10,
             mixolydian 0 2 4 5 7 9 10, harmonic minor 0 2 3 5 7 8 11, pentatonic major 0 2 4 7 9,
             pentatonic minor 0 3 5 7 10. A scale with a minor third (3) is minor-family
  chords     stacked thirds on degree d of the chord scale (the key's, or the parent seven-note
             scale of a pentatonic: major, minor): degrees d, d+2, d+4 (+ d+6 with sevenths), a
             degree past the scale's length an octave up; the root in 48..59, every note above 71
             an octave down; written low to high. One chord a bar: progression[bar mod 4]
  bass       the chord root in 36..47; the fifth = root pitch class + 7 in 36..47; an octave up
             = root + 12; an octave down = root - 12 when that is 24 or more, else the root; the
             next degree = the root plus the step to the chord scale's next degree
  melody     the pool: the key family's pentatonic on the key's root, keeping only the notes in
             the key's scale. Per bar: the count (drawn: min + below(spread); then rest-light:
             chance(p) caps it), starts picked by below(len) with a repeat drawn again, sorted
             (or every start of the table, no draws), then each start one step late on chance(p)
             when the table says so, in start order; each note lasts the gap to the next start in
             the bar, or to the bar's end for the last, at most the genre's max_length steps. Per
             note in time order: its pitch (the channel's first note is the pool note in the
             genre's band low..high nearest the bar's chord root put in the band's lowest octave,
             ties to the lower; every next one moves below(5) - 2 places along the pool from the
             one before, then folds into the band by octaves), then its velocity
             0.55 + jitter(20)/100
  sustained  a melody channel on a sustaining instrument (keys, pads, strings) holds its notes:
             after the draws, a note drawn 2 steps or longer lasts to the next start in its bar,
             or to the bar's end for the last (no max_length cap, no rest between); one drawn
             shorter stays as drawn; starts, keys and velocities never move. Chords and bass keep
             their lengths, and a plucked channel (every other) is as written
  range      a channel may carry the lowest and highest key its instrument plays. After its
             draws, a chords, bass or melody part is fitted to it, in two steps: (1) the whole part
             moves by one octave offset, 0, -12, +12, -24 or +24 tried in that order, the first
             that puts the most notes inside winning; (2) each note still outside moves the
             fewest whole octaves toward the range (up while below its low key, down while above
             its high key), and one that cannot reach it so (the range narrower than an octave,
             its pitch class not in it) is left out and counted as dropped. Drums never move, and
             a channel without a range is as written, so fitting changes no draw
  one key    on one channel, a later note on the same tick and key replaces the earlier one
             (house_1_shared_key: the snare's hit on steps 4 and 12 replaces the kick's there)
  output     each channel's notes sorted by (tick, key); a drum hit is a note of length 0. The
             saved project: the pattern "Jam N" (the lowest N no pattern has) after the others,
             its bars, its swing (written when not 500), every note sorted by (tick, channel,
             key) with the key written when not 60; "key":{"root":R,"scale":S} after "ppq",
             which makes the file format_version 11; tempo the genre's with --auto-tempo
  CLI range  N=LO-HI (--range), the keys inclusive; the requests list them as the flags spell them
  CLI roles  N=drums (the GM map: kick 36, snare 38, clap 39, closed hat 42, open hat 46, perc
             37), N=drums:kick=K,snare=K,clap=K,chat=K,ohat=K,perc=K (only those voices),
             N=kick|snare|clap|chat|ohat|perc (that voice alone on key 60), N=bass|chords|melody;
             channels go in rack order whatever order the flags come in

Choices where the spec's table is loose (the Rust follows them):
  - "(v)" is a base velocity (jittered) unless the spec says fixed; a hit without one takes its
    voice's default base, chance hits too (boom bap's kick 7 @40 is 0.90).
  - Lo-fi's last-bar snare fills (0.50, 0.60) are bases; its ghosts' 0.35 is fixed, and so are
    boom bap's ghosts. No genre but lo-fi has the last-bar fill.
  - Boom bap's "every even step as lo-fi" closed hat is lo-fi's whole closed hat (the drops and
    the odd-step fills too). Other genres' hats draw no drop chance.
  - The velocity floor 0.30 (trap's 0.40 and phonk's 0.35 hats, drill's and afrobeats' added
    0.40 hats can jitter below the pin otherwise); the draw still happens.
  - Trap's roll: one draw after the bar's 16 hats; on a hit, four hats a half step after steps
    12, 13, 14 and 15, fixed 0.35.
  - Drill's triplet feel: its draw comes first; on a hit the hats are the even steps but 2 and 10
    plus 3 and 11 (base 0.40), in step order. Its even steps take the default 0.60/0.45 accents.
  - House chords: the chance-50 hit on step 0 draws first (step order), then the stabs.
  - Drill's slide (root + 1 scale degree) stays above the root (it may reach 49) and is a passing
    note, not a root or fifth. Afrobeats' unstated lengths: step 7 root 3 steps, step 14 fifth 2.
  - The melody's pool drops pentatonic notes outside the key (harmonic minor keeps 4 notes), and
    its first note is the nearest pool note to the chord root (house minor starts on degree 6,
    which the minor pentatonic lacks).
  - Trap's (and so drill's) melody keeps lo-fi's rest-light draw; afrobeats' has none; house
    draws no count, no rest-light and no starts, only the four late draws.
  - The progression draw happens even when no channel plays harmony.

Cases (each request p36_<genre>_<seed>[_<what>].json names its project, flags and roles):
  lofi_1            A minor, 4 bars, the band (drums, bass, chords, melody), --auto-tempo
  lofi_2            C major, seed 2
  lofi_3            A minor, 8 bars (the progression twice)
  lofi_4            E harmonic minor (the melody's 4-note pool)
  lofi_5            Bb pentatonic major (chords from the parent major scale)
  lofi_7_keyed      no key flags: the project's own key, D dorian (p36_keyed.json, version 11)
  lofi_1_no_melody  lofi_1 without the melody: the other channels as lofi_1's, note for note
  lofi_6_kit        three sample channels, one voice each (kick, snare, closed hat), D minor
  boombap_1, trap_1, drill_1, house_1 (--auto-tempo), phonk_1 (a drum map of its own with all
  six voices), afrobeats_1: A minor, seed 1, the band
  lofi_1_ranged     lofi_1 with ranges: the bass on 26..45 (the Electric Bass's keys), the chords
                    on 40..64, the melody on 60..76 (lo-fi's melody band, 60..72, lies inside it;
                    one chord note, 67, moves alone to 55)
  lofi_1_narrow_bass lofi_1 with the bass on 40..46, narrower than an octave: its D drops
  lofi_10_electric_bass seed 10, A minor, the bass on 26..45: its walking bass (B on 47, the
                    octave above on 57 and 59) does not fit as it is; an octave down all but one
                    note does, and that one moves down another octave (the app rolls the same
                    from p36_band_bass.json, whose Bass set spans 26..45)
  lofi_1_sustained  lofi_1 with the melody (channel 4) sustained: the same notes, since lo-fi's
                    12-step cap already lets each of that roll's reach the next
  trap_1_sustained  trap_1 with the melody sustained (trap's melody is the choppiest)
  house_1_shared_key  house, A minor, seed 1, one drum channel whose kick and snare share key 36
                    (kick=36,snare=36,chat=42): the only case where two hits meet on a tick and key
Files:
  p36_band.json     the band: an instrument set "Drums" (p36_sets/drums: a hit on keys 24 to 46),
                    "Bass" and "Keys" on p36_sets/tone (one tone on every key), synth "Lead"
  p36_band_bass.json the band with "Bass" on p36_sets/bass (one zone, keys 26 to 45)
  p36_band_strings.json the band with "Strings" on p36_sets/strings (set.json's category
                    Strings, one tone on every key) where the synth "Lead" was: the app plays it
                    sustained, as lofi_1_sustained
  p36_kit.json      samplers "Kick", "Snare", "Hat closed" on p36_sets/drums/hit.wav
  p36_keyed.json    the band with the key D dorian; p36_waltz.json the band in 3/4 (refused)
  p36_expected.json each case's request file, the pattern --print gives and the saved project's
                    version, tempo, key and Jam pattern; the PRNG's first ten values from seeds 1
                    and 0; the harmony table (chords, bass, melody pool) for A and C in every scale

Run:
  python3 -I gen_p36_expected.py              print the known answers
  python3 -I gen_p36_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p36_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import json, os, struct, sys
from collections import namedtuple

PPQ = 96; STEP = PPQ // 4; HALF = STEP // 2; BAR = 16 * STEP
SR = 48000
DEFAULT_KEY = 60
MASK = (1 << 64) - 1


def J(d): return json.dumps(d, separators=(",", ":"))


# ---------- the PRNG ----------
GOLDEN = 0x9E3779B97F4A7C15
GENRE_ORDER = ["lofi", "boombap", "trap", "drill", "house", "phonk", "afrobeats"]
def splitmix64(x):
    z = (x + GOLDEN) & MASK
    z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
    z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
    return z ^ (z >> 31)
def jam_state(genre, seed):
    salt = ((GENRE_ORDER.index(genre) + 1) * GOLDEN) & MASK
    return splitmix64(seed ^ salt) or GOLDEN
class Rng:
    def __init__(self, state): self.s = state
    def next_u32(self):
        s = self.s
        s ^= s >> 12
        s ^= (s << 25) & MASK
        s ^= s >> 27
        self.s = s
        return ((s * 0x2545F4914F6CDD1D) & MASK) >> 32
    def below(self, n): return self.next_u32() % n
    def chance(self, p): return self.below(100) < p
    def jitter(self, r): return self.below(r)


# ---------- keys and harmony ----------
SCALES = {"major": [0, 2, 4, 5, 7, 9, 11], "minor": [0, 2, 3, 5, 7, 8, 10],
          "dorian": [0, 2, 3, 5, 7, 9, 10], "mixolydian": [0, 2, 4, 5, 7, 9, 10],
          "harmonic_minor": [0, 2, 3, 5, 7, 8, 11], "pentatonic_major": [0, 2, 4, 7, 9],
          "pentatonic_minor": [0, 3, 5, 7, 10]}
PARENT = {"pentatonic_major": "major", "pentatonic_minor": "minor"}
ROOTS = {"C": 0, "C#": 1, "Db": 1, "D": 2, "D#": 3, "Eb": 3, "E": 4, "F": 5, "F#": 6, "Gb": 6,
         "G": 7, "G#": 8, "Ab": 8, "A": 9, "A#": 10, "Bb": 10, "B": 11}
CLI_SCALES = {"major": "major", "minor": "minor", "dorian": "dorian", "mixolydian": "mixolydian",
              "harmonic-minor": "harmonic_minor", "pentatonic-major": "pentatonic_major",
              "pentatonic-minor": "pentatonic_minor"}


def minor_family(scale): return 3 in SCALES[scale]
def chord_steps(scale): return SCALES[PARENT.get(scale, scale)]
def offset(steps, degree):
    """Semitones from the key's root to 1-based degree `degree`, past the scale an octave up."""
    i = degree - 1
    return steps[i % len(steps)] + 12 * (i // len(steps))
def chord_root_pc(root, scale, degree): return (root + offset(chord_steps(scale), degree)) % 12
def chord(root, scale, degree, sevenths):
    steps = chord_steps(scale)
    low = 48 + chord_root_pc(root, scale, degree)
    degrees = [degree, degree + 2, degree + 4] + ([degree + 6] if sevenths else [])
    notes = [low + offset(steps, d) - offset(steps, degree) for d in degrees]
    return sorted(n - 12 if n > 71 else n for n in notes)
def bass_note(root, scale, degree, pitch):
    pc = chord_root_pc(root, scale, degree)
    low = 36 + pc
    if pitch == "root": return low
    if pitch == "fifth": return 36 + (pc + 7) % 12
    if pitch == "octave_up": return low + 12
    if pitch == "octave_down": return low - 12 if low - 12 >= 24 else low
    if pitch == "next_degree":
        steps = chord_steps(scale)
        return low + offset(steps, degree + 1) - offset(steps, degree)
    raise ValueError(pitch)
def melody_pool(root, scale):
    pent = "pentatonic_minor" if minor_family(scale) else "pentatonic_major"
    in_key = {(root + s) % 12 for s in SCALES[scale]}
    return sorted(pc for pc in ((root + s) % 12 for s in SCALES[pent]) if pc in in_key)
def ladder_index(pool, note): return (note // 12) * len(pool) + pool.index(note % 12)
def ladder_note(pool, k): return 12 * (k // len(pool)) + pool[k % len(pool)]
def fold(note, low, high):
    while note > high: note -= 12
    while note < low: note += 12
    return note
def nearest(pool, target, low, high):
    return min((n for n in range(low, high + 1) if n % 12 in pool), key=lambda n: (abs(n - target), n))


# ---------- the tables ----------
# Every genre's numbers live in the engine's crates/engine/src/jam/genres.json, the one place
# they are tuned; this file reads them from there. The rules that draw from them (the PRNG, the
# order, the harmony, the fit) stay this file's own.
GENRES_JSON = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "crates",
                           "engine", "src", "jam", "genres.json")
Hit = namedtuple("Hit", "step half length pitch when vel")
VOICES = ["kick", "snare", "clap", "closed_hat", "open_hat", "perc"]
# How far below its base a velocity may fall when a part does not say: jitter(16) hundredths.
DEFAULT_JITTER = 16
def load_hit(d):
    if "chance" in d: when = ("chance", d["chance"])
    elif "unless" in d: when = ("unless", d["unless"])
    else: when = ("always",)
    vel = ("jitter", d["base"]) if "base" in d else ("exact", d["exact"])
    return Hit(d["step"], d.get("half", False), d.get("length", 0), d.get("pitch", "root"), when, vel)
def load_part(d):
    """A part as (kind, ..., jitter): ("hits", hits, j), ("either", p, then, otherwise, j) or
    ("last_bar", hits, j)."""
    jitter = d.get("jitter", DEFAULT_JITTER)
    if "hits" in d: return ("hits", [load_hit(h) for h in d["hits"]], jitter)
    if "last_bar" in d: return ("last_bar", [load_hit(h) for h in d["last_bar"]], jitter)
    return ("either", d["either"], [load_hit(h) for h in d["then"]],
            [load_hit(h) for h in d["otherwise"]], jitter)
def load_melody(d):
    if d["count"] == "every": count, rest = ("every",), None
    else:
        drawn = d["count"]["drawn"]
        count = ("draw", drawn["least"], drawn["spread"])
        rest = (drawn["rest_light"]["chance"], drawn["rest_light"]["cap"]) if "rest_light" in drawn else None
    return {"count": count, "rest_light": rest, "starts": d["starts"], "late": d.get("late"),
            "low": d.get("low", 60), "high": d.get("high", 72), "max_length": d["max_length"]}
PARTS = VOICES + ["chords", "bass", "melody"]
def load_genres(path):
    with open(path, encoding="utf-8") as f: data = json.load(f)["genres"]
    out = {}
    for gid, t in data.items():
        table = {"name": t["name"], "tempo": t["tempo_bpm"], "range": t["tempo_range"],
                 "swing": t["swing_tenths"], "sevenths": t["sevenths"],
                 "minor": t["minor_progressions"], "major": t["major_progressions"],
                 "melody": [load_melody(m) for m in t["melody"]]}
        # Each part a list of variants: a voice's, the chords' and the bass's each a list of
        # parts, the melody's each a melody table.
        for part in VOICES + ["chords", "bass"]:
            table[part] = [[load_part(p) for p in v["parts"]] for v in t[part]]
        out[gid] = table
    return out
GENRES = load_genres(GENRES_JSON)
CLI_VOICES = {"kick": "kick", "snare": "snare", "clap": "clap", "chat": "closed_hat",
              "ohat": "open_hat", "perc": "perc"}
GM = {"kick": 36, "snare": 38, "clap": 39, "closed_hat": 42, "open_hat": 46, "perc": 37}
VELOCITY_FLOOR, VELOCITY_CEILING = 30, 100
MELODY_BASE, MELODY_SPREAD = 55, 20


# ---------- the writer ----------
def play_parts(rng, parts, last):
    """The hits a voice's parts play in one bar, with their velocities, drawing as they go."""
    out = []
    for part in parts:
        if part[0] == "hits": listed = part[1]
        elif part[0] == "either": listed = part[2] if rng.chance(part[1]) else part[3]
        else:
            if not last: continue
            listed = part[1]
        for h in listed:
            if h.when[0] == "chance" and not rng.chance(h.when[1]): continue
            if h.when[0] == "unless" and rng.chance(h.when[1]): continue
            v = h.vel[1] if h.vel[0] == "exact" else h.vel[1] - rng.jitter(part[-1])
            out.append((h, max(VELOCITY_FLOOR, min(VELOCITY_CEILING, v))))
    return out


def tick_of(bar, h): return bar * BAR + h.step * STEP + (HALF if h.half else 0)


OCTAVE_OFFSETS = [0, -12, 12, -24, 24]
def fit(notes, key_range):
    """A pitched part's notes fitted to `key_range` (lo, hi), and how many were left out."""
    if key_range is None: return notes, 0
    lo, hi = key_range
    inside = lambda k: lo <= k <= hi
    best = OCTAVE_OFFSETS[0]
    for k in OCTAVE_OFFSETS:
        if sum(inside(n[2] + k) for n in notes) > sum(inside(n[2] + best) for n in notes): best = k
    fitted = {}; dropped = 0
    for t, l, key, v in notes:
        moved = key + best
        if moved < lo: moved += 12 * ((lo - moved + 11) // 12)
        elif moved > hi: moved -= 12 * ((moved - hi + 11) // 12)
        if inside(moved): fitted[(t, moved)] = (t, l, moved, v)
        else: dropped += 1
    return sorted(fitted.values(), key=lambda n: (n[0], n[2])), dropped


def write(genre, root, scale, bars, seed, channels, ranges=None, sustained=()):
    """The Jam: (progression, [(channel, role, [(tick, length, key, velocity hundredths)])],
    [(channel, the note replaced, the velocity that replaced it)], {channel: notes dropped},
    the tempo in BPM, {part: the variant picked, from 0})."""
    ranges = ranges or {}
    dropped = {}
    assert PPQ % 8 == 0, "a Jam needs a PPQ that is a multiple of 8"
    replaced = []
    table = GENRES[genre]
    rng = Rng(jam_state(genre, seed))
    progressions = table["minor"] if minor_family(scale) else table["major"]
    progression = progressions[rng.below(len(progressions))]
    low, high = table["range"]
    tempo = low + rng.below(high - low + 1)
    picks = {part: rng.below(len(table[part])) if len(table[part]) > 1 else 0 for part in PARTS}
    picked = {part: table[part][picks[part]] for part in PARTS}
    degree = lambda bar: progression[bar % len(progression)]
    parts = []
    for channel, role in channels:
        notes = {}
        def put(tick, length, key, v):
            if (tick, key) in notes: replaced.append((channel, notes[(tick, key)], v))
            notes[(tick, key)] = (tick, length, key, v)
        if role[0] == "drums":
            drum_map = role[1]
            for bar in range(bars):
                for voice in VOICES:
                    key = drum_map.get(voice)
                    if key is None: continue
                    for h, v in play_parts(rng, picked[voice], bar == bars - 1):
                        put(tick_of(bar, h), 0, key, v)
        elif role[0] == "chords":
            for bar in range(bars):
                stack = chord(root, scale, degree(bar), table["sevenths"])
                for h, v in play_parts(rng, picked["chords"], bar == bars - 1):
                    for key in stack: put(tick_of(bar, h), h.length * STEP, key, v)
        elif role[0] == "bass":
            for bar in range(bars):
                for h, v in play_parts(rng, picked["bass"], bar == bars - 1):
                    put(tick_of(bar, h), h.length * STEP, bass_note(root, scale, degree(bar), h.pitch), v)
        elif role[0] == "melody":
            m = picked["melody"]; pool = melody_pool(root, scale); previous = None
            for bar in range(bars):
                if m["count"][0] == "draw":
                    count = m["count"][1] + rng.below(m["count"][2])
                    if m["rest_light"] and rng.chance(m["rest_light"][0]):
                        count = min(count, m["rest_light"][1])
                    count = min(count, len(m["starts"]))
                    starts = []
                    while len(starts) < count:
                        s = m["starts"][rng.below(len(m["starts"]))]
                        if s not in starts: starts.append(s)
                    starts.sort()
                else:
                    starts = list(m["starts"])
                if m["late"] is not None:
                    starts = [s + 1 if rng.chance(m["late"]) else s for s in starts]
                for i, s in enumerate(starts):
                    end = starts[i + 1] if i + 1 < len(starts) else 16
                    length = min(m["max_length"], end - s)
                    # Sustained: held to the next start (or the bar's end), drawn shorts kept.
                    if channel in sustained and length >= 2: length = end - s
                    if previous is None:
                        pc = chord_root_pc(root, scale, degree(bar))
                        key = nearest(pool, m["low"] + (pc - m["low"]) % 12, m["low"], m["high"])
                    else:
                        key = fold(ladder_note(pool, ladder_index(pool, previous) + rng.below(5) - 2),
                                   m["low"], m["high"])
                    v = MELODY_BASE + rng.jitter(MELODY_SPREAD)
                    put(bar * BAR + s * STEP, length * STEP, key, v)
                    previous = key
        written = sorted(notes.values())
        if role[0] != "drums": written, dropped[channel] = fit(written, ranges.get(channel))
        else: dropped[channel] = 0
        parts.append((channel, role, written))
    return progression, parts, replaced, dropped, tempo, picks


# ---------- requests ----------
def parse_role(text):
    channel, what = text.split("=", 1)
    if what == "drums": return int(channel), ("drums", dict(GM))
    if what.startswith("drums:"):
        drum_map = {}
        for pair in what[len("drums:"):].split(","):
            voice, key = pair.split("=")
            drum_map[CLI_VOICES[voice]] = int(key)
        return int(channel), ("drums", drum_map)
    if what in CLI_VOICES: return int(channel), ("drums", {CLI_VOICES[what]: DEFAULT_KEY})
    return int(channel), (what,)


def parse_range(text):
    channel, keys = text.split("=", 1)
    lo, hi = keys.split("-")
    return int(channel), (int(lo), int(hi))


BAND_ROLES = ["1=drums", "2=bass", "3=chords", "4=melody"]
def request(project, genre, seed, key=None, scale=None, bars=4, roles=BAND_ROLES, auto=False,
            ranges=None, sustained=None):
    r = {"project": project, "genre": genre, "bars": bars, "seed": seed, "roles": roles,
         "auto_tempo": auto}
    if key is not None: r["key"] = key; r["scale"] = scale
    if ranges: r["ranges"] = ranges
    if sustained: r["sustained"] = sustained
    return r


KEYED = ("D", "dorian")
CASES = {
    "lofi_1": request("p36_band.json", "lofi", 1, "A", "minor", auto=True),
    "lofi_2": request("p36_band.json", "lofi", 2, "C", "major"),
    "lofi_3": request("p36_band.json", "lofi", 3, "A", "minor", bars=8),
    "lofi_4": request("p36_band.json", "lofi", 4, "E", "harmonic-minor"),
    "lofi_5": request("p36_band.json", "lofi", 5, "Bb", "pentatonic-major"),
    "lofi_7_keyed": request("p36_keyed.json", "lofi", 7),
    # The flags in another order than the rack's: the channels still go in rack order.
    "lofi_1_no_melody": request("p36_band.json", "lofi", 1, "A", "minor",
                                roles=["3=chords", "1=drums", "2=bass"], auto=True),
    "lofi_6_kit": request("p36_kit.json", "lofi", 6, "D", "minor",
                          roles=["1=kick", "2=snare", "3=chat"]),
    "boombap_1": request("p36_band.json", "boombap", 1, "A", "minor"),
    "trap_1": request("p36_band.json", "trap", 1, "A", "minor"),
    "drill_1": request("p36_band.json", "drill", 1, "A", "minor"),
    "house_1": request("p36_band.json", "house", 1, "A", "minor", auto=True),
    "phonk_1": request("p36_band.json", "phonk", 1, "A", "minor",
                       roles=["1=drums:kick=24,snare=31,clap=29,chat=40,ohat=42,perc=33",
                              "2=bass", "3=chords", "4=melody"]),
    "afrobeats_1": request("p36_band.json", "afrobeats", 1, "A", "minor"),
    "lofi_1_ranged": request("p36_band.json", "lofi", 1, "A", "minor", auto=True,
                             ranges=["2=26-45", "3=40-64", "4=60-76"]),
    "lofi_10_electric_bass": request("p36_band.json", "lofi", 10, "A", "minor",
                                    ranges=["2=26-45"]),
    "lofi_1_narrow_bass": request("p36_band.json", "lofi", 1, "A", "minor", auto=True,
                                  ranges=["2=40-46"]),
    "lofi_1_sustained": request("p36_band.json", "lofi", 1, "A", "minor", auto=True, sustained=[4]),
    "trap_1_sustained": request("p36_band.json", "trap", 1, "A", "minor", sustained=[4]),
    "house_1_shared_key": request("p36_band.json", "house", 1, "A", "minor",
                                  roles=["1=drums:kick=36,snare=36,chat=42"]),
}


def file_name(case):
    genre_seed = "_".join(case.split("_")[:2])
    rest = case[len(genre_seed):]
    return "p36_%s%s.json" % (genre_seed, rest)


# ---------- projects and sets ----------
def wav_bytes(samples):
    data = struct.pack("<%df" % len(samples), *samples)
    fmt = struct.pack("<HHIIHH", 3, 1, SR, SR * 4, 4, 32)
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack("<I", len(data)) + data
    return b"RIFF" + struct.pack("<I", len(body)) + body


# A tenth of a second at a quarter, a level any render hears.
HIT = [0.25] * 4800
DRUM_KEYS = sorted(set(GM.values()) | {24, 29, 31, 33, 40})
DRUMS_SFZ = "".join("<region> sample=hit.wav key=%d\n" % k for k in DRUM_KEYS)
TONE_SFZ = "<region> sample=tone.wav lokey=0 hikey=127 pitch_keycenter=60\n"
# The keys of the Electric Bass, the set a range is for: 26 (D1) to 45 (A2).
BASS_SFZ = "<region> sample=tone.wav lokey=26 hikey=45 pitch_keycenter=36\n"
def set_json(name, category=None):
    d = {"name": name, "maker": "Dawsome tests", "licence": "CC0-1.0"}
    if category: d["category"] = category
    return d
def instrument(i, name, path):
    return {"id": i, "name": name, "kind": "instrument", "instrument": path, "volume": 1.0,
            "pan": 0.0, "mute": False}
def sampler(i, name, sample):
    return {"id": i, "name": name, "kind": "sampler", "sample": sample, "volume": 1.0, "pan": 0.0,
            "mute": False}
BAND = [instrument(1, "Drums", "p36_sets/drums/drums.sfz"),
        instrument(2, "Bass", "p36_sets/tone/tone.sfz"),
        instrument(3, "Keys", "p36_sets/tone/tone.sfz"),
        {"id": 4, "name": "Lead", "kind": "synth", "processor": {"id": "dawsome.synth", "params": {}},
         "volume": 1.0, "pan": 0.0, "mute": False}]
KIT = [sampler(1, "Kick", "p36_sets/drums/hit.wav"), sampler(2, "Snare", "p36_sets/drums/hit.wav"),
       sampler(3, "Hat closed", "p36_sets/drums/hit.wav")]
MBPM = 120000
def project(version, channels, key=None, signature=None):
    d = {"format_version": version, "sample_rate": SR, "tempo_mbpm": MBPM, "ppq": PPQ}
    if signature: d["time_signature"] = {"beats": signature[0], "unit": signature[1]}
    if key: d["key"] = key
    d.update({"next_channel_id": len(channels) + 1, "channels": channels,
              "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": []}]})
    return d
def key_json(root, scale): return {"root": root, "scale": scale}
# The synth makes the band version 9; the key 11 and the 3/4 signature 10; the samplers 2.
BAND_BASS = [dict(c, instrument="p36_sets/bass/bass.sfz") if c["name"] == "Bass" else c
             for c in BAND]
BAND_STRINGS = BAND[:3] + [instrument(4, "Strings", "p36_sets/strings/strings.sfz")]
PROJECTS = {"p36_band.json": project(9, BAND),
            "p36_band_bass.json": project(9, BAND_BASS),
            # No synth, an instrument set where it was: version 4.
            "p36_band_strings.json": project(4, BAND_STRINGS),
            "p36_keyed.json": project(11, BAND, key=key_json(ROOTS[KEYED[0]], KEYED[1])),
            "p36_waltz.json": project(10, BAND, signature=(3, 4)),
            "p36_kit.json": project(2, KIT)}


# ---------- the answers ----------
ROLE_NAMES = {"drums": "drums", "bass": "bass", "chords": "chords", "melody": "melody"}
def answer(case):
    req = CASES[case]
    proj = PROJECTS[req["project"]]
    if "key" in req: root, scale = ROOTS[req["key"]], CLI_SCALES[req["scale"]]
    else: root, scale = proj["key"]["root"], proj["key"]["scale"]
    channels = sorted(parse_role(r) for r in req["roles"])
    table = GENRES[req["genre"]]
    ranges = dict(parse_range(r) for r in req.get("ranges", []))
    sustained = set(req.get("sustained", []))
    progression, parts, replaced, dropped, tempo, picks = write(
        req["genre"], root, scale, req["bars"], req["seed"], channels, ranges, sustained)
    printed = {
        "genre": req["genre"], "key": key_json(root, scale), "bars": req["bars"],
        "seed": req["seed"], "progression": progression, "tempo_mbpm": tempo * 1000,
        "swing_tenths": table["swing"],
        # Each part's variant, counted from 1 as the file lists them.
        "variants": {part: picks[part] + 1 for part in PARTS},
        "channels": [{"channel": c, "role": ROLE_NAMES[role[0]],
                      "notes": [{"tick": t, "length": l, "key": k, "velocity": v / 100}
                                for t, l, k, v in notes], "dropped": dropped[c]}
                     for c, role, notes in parts],
    }
    saved_notes = sorted((t, c, k, l, v) for c, _, notes in parts for t, l, k, v in notes)
    def note_json(t, c, k, l, v):
        n = {"channel": c, "tick": t, "length": l, "velocity": v / 100}
        if k != DEFAULT_KEY: n["key"] = k
        return n
    pattern = {"id": 2, "name": "Jam 1", "length_bars": req["bars"]}
    if table["swing"] != 500: pattern["swing_tenths"] = table["swing"]
    pattern["notes"] = [note_json(*n) for n in saved_notes]
    saved = {"format_version": 11, "key": key_json(root, scale),
             "tempo_mbpm": tempo * 1000 if req["auto_tempo"] else proj["tempo_mbpm"],
             "pattern": pattern}
    return ({"request": file_name(case), "print": printed, "saved": saved},
            (root, scale, parts, replaced, ranges, dropped))


def harmony():
    """Chords per degree with and without sevenths, bass pitches and the melody's pool, for the
    roots A and C in every scale."""
    out = {}
    for root_name in ("A", "C"):
        root = ROOTS[root_name]
        for scale in SCALES:
            out["%s %s" % (root_name, scale)] = {
                "triads": [chord(root, scale, d, False) for d in range(1, 8)],
                "sevenths": [chord(root, scale, d, True) for d in range(1, 8)],
                "bass": [{p: bass_note(root, scale, d, p) for p in
                          ("root", "fifth", "octave_up", "octave_down", "next_degree")}
                         for d in range(1, 8)],
                "melody_pool": melody_pool(root, scale),
            }
    return out


def rng_values(genre, seed):
    r = Rng(jam_state(genre, seed))
    return [r.next_u32() for _ in range(10)]


def expected():
    cases = {case: answer(case)[0] for case in CASES}
    return {"rng": {"lofi_seed_1": rng_values("lofi", 1), "boombap_seed_1": rng_values("boombap", 1),
                    "splitmix64": [splitmix64(x) for x in (0, 1, GOLDEN)]},
            "harmony": harmony(), "cases": cases}


def files():
    w = lambda d: (J(d) + "\n").encode()
    out = {name: w(d) for name, d in PROJECTS.items()}
    for case, req in CASES.items(): out[file_name(case)] = w(req)
    out["p36_sets/drums/drums.sfz"] = DRUMS_SFZ.encode()
    out["p36_sets/drums/hit.wav"] = wav_bytes(HIT)
    out["p36_sets/drums/set.json"] = w(set_json("Jam drums"))
    out["p36_sets/tone/tone.sfz"] = TONE_SFZ.encode()
    out["p36_sets/tone/tone.wav"] = wav_bytes(HIT)
    out["p36_sets/tone/set.json"] = w(set_json("Jam tone"))
    out["p36_sets/bass/bass.sfz"] = BASS_SFZ.encode()
    out["p36_sets/bass/tone.wav"] = wav_bytes(HIT)
    out["p36_sets/bass/set.json"] = w(set_json("Jam bass"))
    out["p36_sets/strings/strings.sfz"] = TONE_SFZ.encode()
    out["p36_sets/strings/tone.wav"] = wav_bytes(HIT)
    out["p36_sets/strings/set.json"] = w(set_json("Jam strings", "Strings"))
    out["p36_expected.json"] = w(expected())
    return out


# ---------- the pins ----------
def sanity():
    """The spec's invariants over every case, and a few values worked by hand."""
    # Music theory by hand: A minor i7 is A3 C4 E4 G4; C major V7 is G3 B3 D4 F4; E harmonic
    # minor V7 is B3 D#4 F#4 A4; Bb pentatonic major chords come from Bb major (IV = Eb G Bb).
    assert chord(9, "minor", 1, True) == [57, 60, 64, 67]
    assert chord(0, "major", 5, True) == [55, 59, 62, 65]
    assert chord(4, "harmonic_minor", 5, True) == [59, 63, 66, 69]
    assert chord(10, "pentatonic_major", 4, False) == [51, 55, 58]
    assert melody_pool(4, "harmonic_minor") == [4, 7, 9, 11]
    assert melody_pool(9, "minor") == [0, 2, 4, 7, 9]
    assert bass_note(9, "minor", 1, "fifth") == 40 and bass_note(9, "minor", 6, "root") == 41
    # The PRNG's xorshift steps from seed 1, by hand: s = 1 -> 1 ^ (1 << 25) = 0x2000001 ->
    # ^ (0x2000001 >> 27 = 0) -> 0x2000001; times the multiplier, the high 32 bits.
    assert Rng(1).next_u32() == ((0x2000001 * 0x2545F4914F6CDD1D) & MASK) >> 32
    # SplitMix64's published first output from state 0, and two genres' seeds part at once.
    assert splitmix64(0) == 0xE220A8397B1DCDAF
    assert rng_values("lofi", 1) != rng_values("boombap", 1)
    assert jam_state("lofi", 1) != jam_state("lofi", 2) and jam_state("lofi", 0) != 0
    answers = {case: answer(case) for case in CASES}
    for case, (ans, (root, scale, parts, _, ranges, _)) in answers.items():
        variant = {part: n - 1 for part, n in ans["print"]["variants"].items()}
        req = CASES[case]; table = GENRES[req["genre"]]
        bars = req["bars"]; end = bars * BAR
        progression = ans["print"]["progression"]
        key_pcs = {(root + s) % 12 for s in SCALES[scale]}
        chord_pcs = {(root + s) % 12 for s in chord_steps(scale)}
        for channel, role, notes in parts:
            assert notes, (case, channel)
            fitted = ranges.get(channel) if role[0] != "drums" else None
            for t, l, k, v in notes:
                assert 0 <= t < end and t + l <= end, (case, channel, t, l)
                assert VELOCITY_FLOOR <= v <= VELOCITY_CEILING, (case, v)
                # A fitted note lies in its range; an unfitted one in its band.
                if fitted: assert fitted[0] <= k <= fitted[1], (case, channel, k)
                tune = table["melody"][variant["melody"]]
                band = tune["low"] <= k <= tune["high"]
                if role[0] == "melody": assert k % 12 in key_pcs and (fitted or band), (case, k)
                held = channel in req.get("sustained", [])
                if role[0] == "melody" and not held: assert l <= tune["max_length"] * STEP, (case, l)
                if role[0] == "chords": assert k % 12 in chord_pcs and (fitted or 48 <= k <= 71), (case, k)
                if role[0] == "bass":
                    pc = chord_root_pc(root, scale, progression[(t // BAR) % len(progression)])
                    # The root (any octave), its fifth, or the passing note a degree above.
                    step_up = bass_note(root, scale, progression[(t // BAR) % len(progression)], "next_degree") % 12
                    assert k % 12 in (pc, (pc + 7) % 12, step_up), (case, t, k)
                if role[0] == "drums": assert l == 0 and k in role[1].values(), (case, k)
    # Seeds 1 and 2 differ in at least 8 drum hits.
    drums = lambda case: {(t, k, v) for c, role, notes in answers[case][1][2] if role[0] == "drums"
                          for t, l, k, v in notes}
    assert len(drums("lofi_1") ^ drums("lofi_2")) >= 8
    # Leaving out the last channel changes nothing before it.
    assert answers["lofi_1_no_melody"][1][2] == answers["lofi_1"][1][2][:3]
    # 8 bars play the progression twice; lo-fi's last-bar fill draws only on bar 8.
    assert len(answers["lofi_3"][0]["print"]["channels"][0]["notes"]) > 0
    # Trap's roll and drill's triplet feel came up at least once in their cases (so the cases
    # pin them): a hat on a half step, a hat on step 3 of a bar.
    trap_hats = [t for t, l, k, v in answers["trap_1"][1][2][0][2] if k == 42]
    assert any(t % STEP == HALF for t in trap_hats)
    drill_hats = [t for t, l, k, v in answers["drill_1"][1][2][0][2] if k == 42]
    assert any(t % BAR == 3 * STEP for t in drill_hats)
    # House's melody: four notes a bar, each on 0, 4, 8, 12 or a step later.
    house_melody = answers["house_1"][1][2][3][2]
    house_starts = GENRES["house"]["melody"][answers["house_1"][0]["print"]["variants"]["melody"] - 1]["starts"]
    assert len(house_melody) == 16 and all((t % BAR) // STEP in {s + late for s in house_starts for late in (0, 1)}
                                           for t, l, k, v in house_melody)
    # One tick and key on one channel holds one note: a later hit replaces the earlier. The
    # shared-key case has such hits (house's kick and snare both on beats 2 and 4, one key);
    # a voice's own fill can also land on its ghost (lo-fi's soft snare), which the rule covers.
    for case, ans in answers.items():
        for c in ans[0]["print"]["channels"]:
            at = [(n["tick"], n["key"]) for n in c["notes"]]
            assert len(at) == len(set(at)), (case, c["channel"])
    assert answers["house_1_shared_key"][1][3], "the shared key meets"
    # Every project a case runs on is at a PPQ the Jam takes.
    assert all(p["ppq"] % 8 == 0 for p in PROJECTS.values())
    # Ranges: a case without one drops nothing. A ranged case is its unranged twin fitted: the
    # same drums; each fitted part inside its range, its notes on the twin's ticks and pitch
    # classes (moved by octaves only), each note kept or counted as dropped.
    for case, ans in answers.items():
        if "ranges" not in CASES[case]:
            assert all(c["dropped"] == 0 for c in ans[0]["print"]["channels"]), case
    def unranged(case):
        req = dict(CASES[case]); req.pop("ranges")
        root, scale = ROOTS[req["key"]], CLI_SCALES[req["scale"]]
        return write(req["genre"], root, scale, req["bars"], req["seed"],
                     sorted(parse_role(r) for r in req["roles"]))[1]
    for case in ("lofi_1_ranged", "lofi_1_narrow_bass", "lofi_10_electric_bass"):
        ans, (_, _, fitted, _, ranges, dropped) = answers[case]
        plain = unranged(case)
        for (c, role, notes), (_, _, twin) in zip(fitted, plain):
            if c not in ranges or role[0] == "drums":
                assert notes == twin, (case, c)
                continue
            lo, hi = ranges[c]
            assert all(lo <= k <= hi for t, l, k, v in notes), (case, c)
            assert {(t, k % 12) for t, l, k, v in notes} <= {(t, k % 12) for t, l, k, v in twin}
            assert len(notes) + dropped[c] == len(twin), (case, c)
    # The narrow bass (40..46 holds E to A#) cannot play every root of A minor: it drops some.
    assert answers["lofi_1_narrow_bass"][0]["print"]["channels"][1]["dropped"] > 0
    # The electric bass case's seed writes a bass the Electric Bass's 26..45 cannot hold as it
    # is; fitted, every note sounds (the range is an octave or more, so nothing drops).
    eight_plain = unranged("lofi_10_electric_bass")[1][2]
    assert any(not 26 <= k <= 45 for t, l, k, v in eight_plain)
    assert answers["lofi_10_electric_bass"][0]["print"]["channels"][1]["dropped"] == 0
    # Sustained: the same Jam as its plucked twin but the melody's lengths: each note drawn 2
    # steps or longer now ends where the next starts in its bar, or at the bar's end; a shorter
    # one as drawn; nothing else moves. Lo-fi's seed 1 changes nothing (its 12-step cap already
    # lets every note of that roll reach the next); trap's (an 8-step cap, rests) lengthens.
    for case, twin in (("lofi_1_sustained", "lofi_1"), ("trap_1_sustained", "trap_1")):
        held, plain = answers[case][1][2], answers[twin][1][2]
        assert [p for p in held if p[1][0] != "melody"] == [p for p in plain if p[1][0] != "melody"]
        (_, _, held_notes), = [p for p in held if p[1][0] == "melody"]
        (_, _, plain_notes), = [p for p in plain if p[1][0] == "melody"]
        assert [(t, k, v) for t, l, k, v in held_notes] == [(t, k, v) for t, l, k, v in plain_notes]
        longer = 0
        for i, ((t, l, k, v), (_, drawn, _, _)) in enumerate(zip(held_notes, plain_notes)):
            bar_end = (t // BAR + 1) * BAR
            later = [n[0] for n in held_notes[i + 1:] if n[0] < bar_end]
            gap = (later[0] if later else bar_end) - t
            assert l == (gap if drawn >= 2 * STEP else drawn), (case, t, l, drawn, gap)
            longer += l > drawn
        assert (longer > 0) == (case == "trap_1_sustained"), (case, longer)
    # The fit by hand: a part all above moves down an octave whole; a note 30 above a range
    # an octave wide moves the fewest octaves that reach it, three; one 30 below, three up; a
    # pitch class a range narrower than an octave lacks is dropped.
    assert fit([(0, 0, 80, 50), (0, 0, 84, 50)], (60, 76)) == ([(0, 0, 68, 50), (0, 0, 72, 50)], 0)
    assert fit([(0, 0, 50, 50), (24, 0, 90, 50)], (48, 60)) == ([(0, 0, 50, 50), (24, 0, 54, 50)], 0)
    assert fit([(0, 0, 30, 50)], (60, 76)) == ([(0, 0, 66, 50)], 0)
    assert fit([(0, 0, 38, 50)], (40, 46)) == ([], 1)
    # The kit's channels each play their one voice on key 60.
    for c, role, notes in answers["lofi_6_kit"][1][2]:
        assert {k for t, l, k, v in notes} == {DEFAULT_KEY}


def main():
    sanity()
    args = sys.argv[1:]
    if len(args) == 2 and args[0] in ("--write", "--check"):
        stale = []
        for path, content in sorted(files().items()):
            target = os.path.join(args[1], path)
            if args[0] == "--write":
                os.makedirs(os.path.dirname(target), exist_ok=True)
                with open(target, "wb") as f: f.write(content)
            else:
                try:
                    with open(target, "rb") as f: same = f.read() == content
                except OSError: same = False
                if not same: stale.append(path)
        if stale: sys.exit("not what gen_p36_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    want = expected()
    print("PRNG lo-fi seed 1:", want["rng"]["lofi_seed_1"])
    print("PRNG boom bap seed 1:", want["rng"]["boombap_seed_1"])
    for case, ans in want["cases"].items():
        p = ans["print"]
        print("%-17s %s %s, progression %s: %s" % (
            case, p["key"]["root"], p["key"]["scale"], p["progression"],
            ", ".join("%d %s %d notes" % (c["channel"], c["role"], len(c["notes"]))
                      for c in p["channels"])))


if __name__ == "__main__":
    main()
