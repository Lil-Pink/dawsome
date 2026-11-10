"""Known answers for time signatures per pattern (Phase B) and changes along the song (Phase C):
a pattern's own signature (its length, its clips, its step grid, its MIDI file, the readout,
metronome, count-in and plugin bar while it plays alone, the edit that sets it, the file that
keeps it) and the song's meter map (its bars, the readout, the metronome, the count-in, the bar a
plugin is given, the song's MIDI file and DAWproject, `--bars`, the stems' end, the edits that
add, replace, remove and move a change and the rule that settles the changes after it, the file
that keeps them).

Expected values come from these rules, never from engine code:
  signature     beats/unit as in gen_p34: beat = 96*4/unit ticks, bar = beats*beat, a step 24
                ticks in every meter; tempo milli-BPM of quarter notes
  tick->sample  s(t) = floor(t*60*sr*1000 / (mbpm*96))
  pattern       a pattern's signature is its own when it has one, else the project's (the
                meter from tick 0); its length is length_bars bars of it, wherever its clips
                lie; a clip plays pattern tick (T - start) mod L; looping it alone (pattern
                mode) the transport's bars are the pattern's: the metronome, the count-in, the
                readout and a plugin's bar follow its signature
  meter map     the song's bars: the project's signature from tick 0, then each change
                (at, signature) from its tick on; bars are numbered on across changes, bar n
                (from 0) of a meter that starts at tick S with bar n0 starts at
                S + (n - n0)*bar; a tick's bar is the last bar start at or before it; a beat
                likewise within the bar's meter
  readout       bar:step:tick from 1:01:00 with the map's bar: bar number + 1, the step
                (t - bar start)//24 + 1 in two digits, the tick (t - bar start) mod 24
  metronome     a click from the first sample of every beat of the meter the tick is in,
                accented on a bar's first; gen_p14's burst (64 frames, A = 0.25 accented, 0.125
                otherwise, frame n = A*(1 - n/64)*(+1 if (n//8) even else -1))
  count-in      one bar of the meter at the tick Record starts from: `beats` clicks k*beat
                apart from the count-in's first frame, the first accented; the song (and the
                recording) starts s(bar) frames later, from that tick
  plugins       the bar the block's first tick is in: its number from 0 and its start in
                quarter notes (start ticks / 96), numerator and denominator of its meter
  MIDI          the song file's conductor track: tempo and the time signature at tick 0, then
                FF 58 at each change's tick (FF 58 04 nn dd cc bb as gen_p34: dd = log2 unit,
                cc = 96/unit, bb 8); it ends on the song's last note-off, or on its last change
                when that is later; a pattern file carries its pattern's signature
  DAWproject    the Transport's TimeSignature is the meter from tick 0, given id="time-signature"
                when the song has changes; then, after the arrangement's Lanes, a
                TimeSignatureAutomation (timeUnit="beats") whose Target names that id and whose
                TimeSignaturePoint (time, numerator, denominator) lists the meter at 0 and each
                change, times in quarter notes
  render        `--bars N`: the start of bar N of the meter that plays (the map in song mode,
                the pattern's own in pattern mode)
  stems         every file ends where the bar after the song's end starts (the song's end
                itself when it is a bar line): the song's end rounded up to the map's bar
  file          a pattern's "time_signature":{"beats":B,"unit":U} right after "length_bars",
                only when it has one of its own; the project's "signature_changes":[{"at":T,
                "beats":B,"unit":U},...] right after "time_signature" (or "ppq"), only when
                there is a change; either makes the file format_version 10
  edits         a pattern's signature: the pattern grows to the whole bars of its new signature
                that hold the end of every note it played (start + length, a step hit a tick),
                never shorter, nothing else changes; undo puts back its signature and bars.
                A change: the requested tick goes down to the bar line at or before it in the
                map without the change being moved (a set: the map as it is); a change already
                there is replaced; tick 0 is refused (the start's meter is the project's
                signature). Then every change, in tick order, is settled: k = floor((at - P) /
                bar), P the settled change before it (or tick 0) and bar that one's meter's;
                at = P + k*bar when k >= 1 (down to its bar line), else P + bar (up to the first
                bar line after P). The project's own signature settles the changes the same way.
                Undo gives back every change as it was.

Rules this phase had to decide (the spec's section 7 says why): the count-in's "recording from"
is the song tick the bar after the start would begin at, while the recording itself starts
s(bar) output frames after the count-in's first frame and the song plays on from the start tick;
a moved change snaps in the map without it, and when it lands on another change it replaces
that one; a change pushed past a later one by "up" keeps the order (every settled change lies
after the one before it); a change between a move's ends is settled like the rest; the song
MIDI's conductor ends at its last change when that is past the last note.

Mutation checks (each breaks a known answer here): the pattern's length taken from the project's
signature (pattern 2's song render, its MIDI file, the edit's growth); the pattern mode metronome
accented on the project's bar; the readout's bar from tick 0's meter alone (4:02:20 becomes
3:...); the map's bar numbers restarting at a change; the accent on every bar of tick 0's
meter; the count-in of tick 0's meter at 1056; the plugin's bar start a multiple of the first
meter's bar; no FF 58 at a change; the conductor's EOT at the last note before a later change;
the settle rule rounding to the nearest line (1920 would go to 2016) or never up (1344 would
collide with 1152); a moved change snapped in the map with it (1300 would land at 1056); the
stems' end rounded to a 4/4 bar (2304).

Files:
  p35_pattern_34.json          4/4 song: pattern 1 (4/4) and pattern 2 (3/4, its own), a hit at 0
                               each; clip of pattern 2 at 0 for 384, of pattern 1 at 384 for 384
  p35_pattern_34_p2.mid        smf-export --pattern 2 of it
  p35_pattern_34_song.mid      export-midi of it
  p35_edit_b.json, p35_edit_b_34.json, p35_edit_b_none.json
                               pattern 1 given 3/4 (grows 1 -> 2 bars), then the project's again
  p35_meter.json               the meter map song: 4/4, 3/4 from 768, 6/8 from 1344, 7/8 from 1920
  p35_meter_song.mid, p35_meter.project.xml   its MIDI file and DAWproject
  p35_meter_stems.json         the map, its last clip ending at 1700
  p35_edit_c.json, p35_edit_c_set.json        a change set at 800 lands at 768
  p35_meter_{set_54,moved,moved_past,removed_1344,removed_768,start_34}.json
                               the edits on p35_meter.json
  p35_expected.json            the cases, renders, readouts, bars, snaps, edits

Run:
  python3 -I gen_p35_expected.py              print the known answers
  python3 -I gen_p35_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p35_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import json, os, re, struct, sys

PPQ = 96; SR = 44100; STEP = 24
CLICK_FRAMES = 64


def J(d): return json.dumps(d, separators=(",", ":"), ensure_ascii=False)
def s(t, mbpm=120000): return t * 60 * SR * 1000 // (mbpm * PPQ)
def beat_of(sig): return PPQ * 4 // sig[1]
def bar_of(sig): return sig[0] * beat_of(sig)


def version():
    """The workspace's version, as Cargo.toml states it: what DAWproject's Application names."""
    here = os.path.dirname(os.path.abspath(__file__))
    with open(os.path.join(here, "..", "..", "Cargo.toml"), encoding="utf-8") as f:
        text = f.read()
    section = text.split("[workspace.package]", 1)[1]
    return re.search(r'^version = "([^"]+)"', section, re.M).group(1)


# 120 BPM at 44.1 kHz: s(t) = t*60*44100*1000 / (120000*96) = 229.6875 t, exact for every
# multiple of 16: s(48) = 11025, s(96) = 22050, s(288) = 66150, s(384) = 88200, s(576) = 132300,
# s(768) = 176400, s(1920) = 441000, s(2592) = 595350.
assert (s(48), s(96), s(288), s(384), s(576), s(768), s(1920), s(2592)) == \
    (11025, 22050, 66150, 88200, 132300, 176400, 441000, 595350)


# ---------- the projects ----------
def sampler(i, name):
    return {"id": i, "name": name, "kind": "sampler", "sample": "impulse.wav", "volume": 1.0,
            "pan": 0.0, "mute": False}


def note(channel, tick, length, velocity, key=None):
    d = {"channel": channel, "tick": tick, "length": length, "velocity": velocity}
    if key is not None: d["key"] = key
    return d


def head(signature=(4, 4), changes=(), version_10=False):
    """The project's first fields in the file's order: the version (10 when anything of a meter
    other than one 4/4 is written, else 2), the signature after the ppq only when not 4/4, the
    changes after it only when there are some."""
    ten = version_10 or tuple(signature) != (4, 4) or bool(changes)
    d = {"format_version": 10 if ten else 2, "sample_rate": SR, "tempo_mbpm": 120000,
         "ppq": PPQ}
    if tuple(signature) != (4, 4):
        d["time_signature"] = {"beats": signature[0], "unit": signature[1]}
    if changes:
        d["signature_changes"] = [{"at": at, "beats": b, "unit": u} for at, (b, u) in changes]
    d["next_channel_id"] = 2
    return d


def pattern(i, name, bars, notes, signature=None):
    """A pattern in the file's order: its own signature right after "length_bars", only when
    it has one."""
    d = {"id": i, "name": name, "length_bars": bars}
    if signature is not None:
        d["time_signature"] = {"beats": signature[0], "unit": signature[1]}
    d["notes"] = notes
    return d


def playlist(clips, tracks=1):
    return {"next_track_id": tracks + 1, "next_clip_id": len(clips) + 1,
            "tracks": [{"id": i, "name": "Track %d" % i, "mute": False}
                       for i in range(1, tracks + 1)], "clips": clips}


def pclip(i, track, start, length, pattern_id):
    return {"id": i, "track": track, "start": start, "length": length, "kind": "pattern",
            "pattern": pattern_id}


def project(patterns, clips=None, signature=(4, 4), changes=(), tracks=1):
    d = head(signature, changes, any("time_signature" in p for p in patterns))
    d["channels"] = [sampler(1, "kick")]
    d["patterns"] = patterns
    if clips is not None: d["playlist"] = playlist(clips, tracks)
    return d


# =================== Phase B: a pattern's own signature ===================
# A 4/4 song. Pattern 1 (the project's 4/4, 384 ticks a bar) and pattern 2 (3/4 of its own,
# 288), one bar each, a step hit at tick 0 (velocity 1, impulse.wav at unit gain). Clip 1 plays
# pattern 2 from 0 for 384 ticks (1 1/3 passes: pattern ticks 0..288 then 0..96), clip 2 pattern
# 1 from 384 for 384.
P34 = project([pattern(1, "Pattern 1", 1, [note(1, 0, 0, 1.0)]),
               pattern(2, "Pattern 2", 1, [note(1, 0, 0, 1.0)], (3, 4))],
              [pclip(1, 1, 0, 384, 2), pclip(2, 1, 384, 384, 1)])
assert P34["format_version"] == 10 and "time_signature" not in P34
# Renders (`dawsome render`):
#   --song --bars 2: two bars of the song's 4/4 = 768 ticks = s(768) = 176400 frames; hits at
#     song ticks 0 (clip 1, pass 1), 288 (clip 1, pass 2: pattern 2 is 288 long, not 384) and
#     384 (clip 2): frames 0, s(288) = 66150, s(384) = 88200
#   --pattern "Pattern 2" --bars 2: two bars of pattern 2's own 3/4 = 576 ticks = 132300
#     frames; the pattern loops every 288: hits at 0 and 66150
B_RENDERS = [
    {"args": ["--song", "--bars", "2"], "frames": 176400, "onsets": [0, 66150, 88200]},
    {"args": ["--pattern", "Pattern 2", "--bars", "2"], "frames": 132300, "onsets": [0, 66150]},
]
assert B_RENDERS[0]["frames"] == s(2 * 384) and B_RENDERS[0]["onsets"] == [s(0), s(288), s(384)]
assert B_RENDERS[1]["frames"] == s(2 * 288) and B_RENDERS[1]["onsets"] == [s(0), s(288)]


def click(n, accented):
    amplitude = 0.25 if accented else 0.125
    sign = 1.0 if (n // 8) % 2 == 0 else -1.0
    return amplitude * (1.0 - n / CLICK_FRAMES) * sign


def clicks_render(frames, beats):
    out = [0.0] * frames
    for at, accented in beats:
        for n in range(CLICK_FRAMES):
            if at + n < frames:
                out[at + n] += click(n, accented)
    return [[i, v] for i, v in enumerate(out) if v != 0.0]


# The metronome of p35_pattern_34 (its sounds loaded as silence, so only the clicks sound):
#   pattern mode on pattern 2, two of its bars (576 ticks, 132300 frames): a click every 96
#     ticks (a quarter), accented every 288 (its bar): ticks 0, 96, ..., 480, accents 0 and 288
#   song mode, the song's 768 ticks (176400 frames): a click every 96, accented every 384 (the
#     song's 4/4): accents at 0 and 384
B_METRONOME = {
    "pattern_2": {"mode": "pattern 2", "frames": 132300,
                  "beats": [[s(t), t % 288 == 0] for t in range(0, 576, 96)]},
    "song": {"mode": "song", "frames": 176400,
             "beats": [[s(t), t % 384 == 0] for t in range(0, 768, 96)]},
}
assert [a for _, a in B_METRONOME["pattern_2"]["beats"]] == [True, False, False, True, False, False]
assert [a for _, a in B_METRONOME["song"]["beats"]] == [True, False, False, False] * 2
# The count-in from tick 0: pattern mode on pattern 2, one bar of its 3/4: 3 clicks at s(0),
# s(96), s(192) = 0, 22050, 44100, the song from s(288) = 66150; song mode, one bar of the song's
# 4/4: 4 clicks at 0, 22050, 44100, 66150, from s(384) = 88200.
B_COUNT_INS = {
    "pattern_2": {"mode": "pattern 2", "start_tick": 0, "clicks": [0, 22050, 44100],
                  "start": 66150},
    "song": {"mode": "song", "start_tick": 0, "clicks": [0, 22050, 44100, 66150],
             "start": 88200},
}
# A plugin's bar at tick 300 (the VST3 test plugin's readouts, CLAP's transport event):
#   pattern mode on pattern 2 (bar 288 ticks = 3 quarters): 300 // 288 = 1 -> bar 1, starting
#     at 288 / 96 = 3.0, 3/4
#   song mode (4/4, bar 384): 300 // 384 = 0 -> bar 0, 0.0, 4/4
B_TRANSPORT = [
    {"mode": "pattern 2", "tick": 300, "numerator": 3, "denominator": 4, "bar_start": 3.0,
     "bar_number": 1},
    {"mode": "song", "tick": 300, "numerator": 4, "denominator": 4, "bar_start": 0.0,
     "bar_number": 0},
]
# The readout at tick 300: in pattern mode on pattern 2 (bar 288): bar 300 // 288 + 1 = 2,
# 300 - 288 = 12 -> step 12 // 24 + 1 = 1 -> "01", tick 12 -> "2:01:12"; in song mode (bar 384):
# bar 1, step 300 // 24 + 1 = 13, tick 300 mod 24 = 12 -> "1:13:12".
B_READOUTS = [
    {"mode": "pattern 2", "tick": 300, "position": "2:01:12"},
    {"mode": "song", "tick": 300, "position": "1:13:12"},
]
assert "%d:%02d:%02d" % (300 // 288 + 1, 300 % 288 // 24 + 1, 300 % 288 % 24) == "2:01:12"
assert "%d:%02d:%02d" % (300 // 384 + 1, 300 % 384 // 24 + 1, 300 % 384 % 24) == "1:13:12"
# The step grid: pattern 2 (3/4, one bar) has 3*16/4 = 12 steps in groups of 4; pattern 1 16.
B_STEPS = [{"pattern": 1, "steps": 16, "group": 4}, {"pattern": 2, "steps": 12, "group": 4}]


# ---------- MIDI of p35_pattern_34 ----------
def varlen(n):
    out = [n & 0x7F]; n >>= 7
    while n: out.append(0x80 | (n & 0x7F)); n >>= 7
    return bytes(reversed(out))


def mtrk(events):
    out = b""; last = 0; rs = None
    for t, msg in events:
        out += varlen(t - last); last = t
        if msg[0] == 0xFF: rs = None; out += bytes(msg)                 # meta cancels running status
        elif msg[0] == rs: out += bytes(msg[1:])                         # same status: left out
        else: rs = msg[0]; out += bytes(msg)
    return b"MTrk" + struct.pack(">I", len(out)) + out


def smf0(events):
    return b"MThd" + struct.pack(">IHHH", 6, 0, 1, PPQ) + mtrk(events)


def smf1(tracks):
    return b"MThd" + struct.pack(">IHHH", 6, 1, len(tracks), PPQ) + b"".join(mtrk(t) for t in tracks)


def meta(kind, data): return [0xFF, kind] + list(varlen(len(data))) + list(data)
def name(text): return meta(0x03, text.encode("utf-8"))
def on(key, vel): return [0x90, key, vel]
def off(key): return [0x80, key, 64]
EOT = meta(0x2F, [])
TEMPO_120 = meta(0x51, list((500000).to_bytes(3, "big")))       # 6e10 / 120000 = 500000 us
# FF 58 04 nn dd cc bb, by hand (dd = log2 unit, cc = 96 / unit, bb = 8):
#   4/4: 04 02 18 08; 3/4: 03 02 18 08; 6/8: 06 03 0C 08; 7/8: 07 03 0C 08
TS = {(4, 4): [0xFF, 0x58, 0x04, 0x04, 0x02, 0x18, 0x08],
      (3, 4): [0xFF, 0x58, 0x04, 0x03, 0x02, 0x18, 0x08],
      (6, 8): [0xFF, 0x58, 0x04, 0x06, 0x03, 0x0C, 0x08],
      (7, 8): [0xFF, 0x58, 0x04, 0x07, 0x03, 0x0C, 0x08]}
for (b, u), event in TS.items():
    assert event[3:] == [b, u.bit_length() - 1, 96 // u, 8]
# smf-export --pattern 2: tempo, pattern 2's own 3/4, its name; the hit at 0 one step long; the
# track ends on its last note-off, 24 (gen_p18's rule: on the last event).
P34_PATTERN_2 = smf0([(0, TEMPO_120), (0, TS[(3, 4)]), (0, name("Pattern 2")),
                      (0, on(60, 127)), (STEP, off(60)), (STEP, EOT)])
# export-midi: the conductor keeps the song's 4/4; kick plays 0-24, 288-312 (pattern 2's second
# pass inside clip 1), 384-408 (clip 2); every track ends on the last note-off, 408.
P34_SONG = smf1([
    [(0, TEMPO_120), (0, TS[(4, 4)]), (408, EOT)],
    [(0, name("kick")), (0, on(60, 127)), (24, off(60)), (288, on(60, 127)), (312, off(60)),
     (384, on(60, 127)), (408, off(60)), (408, EOT)],
])

# ---------- the edit ----------
# p35_edit_b (4/4): pattern 1, one bar, a step hit at 300; pattern 2, two bars, a note at 0 for
# 96 ticks. Pattern 1 given 3/4 (bar 288): the hit ends at 301, ceil(301 / 288) = 2 bars -> grows
# to 2 (576 holds it), its field written, the file version 10; pattern 2 untouched. Undo gives
# the first bytes back; 3/4 again changes nothing. Then given the project's (None) again: bar
# 384, ceil(301 / 384) = 1 <= 2: it stays 2 bars (never shorter), no field, version 2 again.
EDIT_B = [pattern(1, "Pattern 1", 1, [note(1, 300, 0, 1.0)]),
          pattern(2, "Pattern 2", 2, [note(1, 0, 96, 0.5)])]
EDIT_B_34 = [pattern(1, "Pattern 1", 2, [note(1, 300, 0, 1.0)], (3, 4)), EDIT_B[1]]
EDIT_B_NONE = [pattern(1, "Pattern 1", 2, [note(1, 300, 0, 1.0)]), EDIT_B[1]]
assert -(-301 // 288) == 2 and -(-301 // 384) == 1

# =================== Phase C: changes along the song ===================
# The meter map: 4/4 from 0, 3/4 from 768 (bar 3), 6/8 from 1344 (bar 5), 7/8 from 1920 (bar 7).
MAP = [(0, (4, 4)), (768, (3, 4)), (1344, (6, 8)), (1920, (7, 8))]
CHANGES = MAP[1:]


def bar_starts(meter, upto):
    """Bar starts (bar 1 first) up to `upto`, by walking each meter's bars to the next change."""
    out = []
    for i, (start, sig) in enumerate(meter):
        end = meter[i + 1][0] if i + 1 < len(meter) else upto
        t = start
        while t < end:
            out.append(t); t += bar_of(sig)
    return out + [t]


# By hand: 4/4 bars of 384 from 0: 0, 384; 3/4 bars of 288 from 768: 768, 1056; 6/8 bars of 288
# from 1344: 1344, 1632; 7/8 bars of 336 from 1920: 1920, 2256; bar 9 starts at 2592.
BARS = [0, 384, 768, 1056, 1344, 1632, 1920, 2256, 2592]
assert bar_starts(MAP, 2592) == BARS


def bar_at(meter, t):
    """(bar number from 0, bar start, meter) of tick t >= 0."""
    i = max(k for k, (start, _) in enumerate(meter) if start <= t)
    start, sig = meter[i]
    first = bar_starts(meter, start).index(start) if i else 0
    n = (t - start) // bar_of(sig)
    return first + n, start + n * bar_of(sig), sig


# bar_at, by hand: 1100 is 44 past 1056, bar 4 (index 3), 3/4; 1700 is 68 past 1632, bar 6
# (index 5), 6/8; 0 bar 1 (0); 767 bar 2 (index 1, start 384); 768 bar 3 (index 2, start 768,
# 3/4); 2300 is 44 past 2256, bar 8 (index 7), 7/8; 2592 bar 9 (index 8), 7/8.
BAR_AT = [[1100, 3, 1056, [3, 4]], [1700, 5, 1632, [6, 8]], [0, 0, 0, [4, 4]],
          [767, 1, 384, [4, 4]], [768, 2, 768, [3, 4]], [2300, 7, 2256, [7, 8]],
          [2592, 8, 2592, [7, 8]]]
for t, n, start, sig in BAR_AT:
    assert bar_at(MAP, t) == (n, start, tuple(sig))

# The readout, by hand: 1100 -> bar 4, 44 in: step 44 // 24 + 1 = 2 -> "02", 44 mod 24 = 20 ->
# "4:02:20"; 1700 -> bar 6, 68 in: step 3, tick 20 -> "6:03:20"; 2300 -> bar 8, 44 in -> "8:02:20";
# 768 -> "3:01:00"; 1343 -> bar 4 (1056), 287 in: step 287 // 24 + 1 = 12, 287 mod 24 = 23 ->
# "4:12:23".
C_READOUTS = [[1100, "4:02:20"], [1700, "6:03:20"], [2300, "8:02:20"], [768, "3:01:00"],
              [1343, "4:12:23"]]
for t, want in C_READOUTS:
    n, start, _ = bar_at(MAP, t)
    assert want == "%d:%02d:%02d" % (n + 1, (t - start) // STEP + 1, (t - start) % STEP)

# Snaps: [tick, "bar"|"beat", down (the line at or before), nearest (the nearer of it and the
# next line, half way up), next (the line after the one at or before)], by hand:
#   1100 bar: 3/4 bars from 768: 1056 | next 1344; 1100 - 1056 = 44 < 288 / 2 -> 1056
#   1300 bar: 1056 | 1344; 244 >= 144 -> 1344
#   1200 bar: 1056 | 1344; 144 = half -> up, 1344
#   1343 bar: 1056 | 1344 -> 1344
#   1700 beat: 6/8 beats of 48 from 1344: 1344 + 7*48 = 1680 | 1728; 20 < 24 -> 1680
#   1343 beat: 3/4 beats of 96 from 768: 768 + 5*96 = 1248 | 1344; 95 >= 48 -> 1344
#   383 beat: 4/4 beats: 288 | 384 -> 384
#   2592 bar: 7/8 bars of 336 from 1920: 1920 + 2*336 = 2592 (on the line) | 2928 -> 2592
#   1919 beat: 6/8 beats from 1344: 1344 + 11*48 = 1872 | 1920; 47 >= 24 -> 1920
SNAPS = [[1100, "bar", 1056, 1056, 1344], [1300, "bar", 1056, 1344, 1344],
         [1200, "bar", 1056, 1344, 1344], [1343, "bar", 1056, 1344, 1344],
         [1700, "beat", 1680, 1680, 1728], [1343, "beat", 1248, 1344, 1344],
         [383, "beat", 288, 384, 384], [2592, "bar", 2592, 2592, 2928],
         [1919, "beat", 1872, 1920, 1920]]
for t, unit, down, nearest, after in SNAPS:
    n, start, sig = bar_at(MAP, t)
    if unit == "bar":
        lo, step = start, bar_of(sig)
    else:
        lo, step = start + (t - start) // beat_of(sig) * beat_of(sig), beat_of(sig)
    assert (lo, lo + step, lo if 2 * (t - lo) < step else lo + step) == (down, after, nearest)

# The metronome over eight bars (2592 ticks, s(2592) = 595350 frames), song mode: beats every 96
# ticks from 0 to 1344 (4/4 then 3/4), every 48 from 1344 (6/8) and from 1920 (7/8); accented on
# the bar starts 0, 384, 768, 1056, 1344, 1632, 1920, 2256.
C_BEATS = list(range(0, 1344, 96)) + list(range(1344, 1920, 48)) + list(range(1920, 2592, 48))
assert len(C_BEATS) == 14 + 12 + 14
C_METRONOME = {"frames": 595350, "beats": [[s(t), t in BARS] for t in C_BEATS]}
assert sum(1 for _, a in C_METRONOME["beats"] if a) == 8
# The count-in, Record started from a tick (the metronome off, so the song after it is silent):
#   from 1056 (3/4, bar 288): 3 clicks 96 ticks apart: 0, 22050, 44100; the song (and the
#     recording) s(288) = 66150 frames on, from 1056 (the bar's end would be tick 1344)
#   from 1632 (6/8, bar 288): 6 clicks 48 apart: 11025 k, k = 0..5; from 66150 (tick 1920)
C_COUNT_INS = {
    "at_1056": {"start_tick": 1056, "clicks": [0, 22050, 44100], "start": 66150,
                "bar_end_tick": 1344},
    "at_1632": {"start_tick": 1632, "clicks": [11025 * k for k in range(6)], "start": 66150,
                "bar_end_tick": 1920},
}
for case in C_COUNT_INS.values():
    _, _, sig = bar_at(MAP, case["start_tick"])
    assert case["clicks"] == [s(k * beat_of(sig)) for k in range(sig[0])]
    assert case["start"] == s(bar_of(sig))
    assert case["bar_end_tick"] == case["start_tick"] + bar_of(sig)
# A plugin's bar (song mode): [tick, numerator, denominator, bar start in quarters, bar number]:
#   1100: bar 4 (number 3) of 3/4 starts at 1056 / 96 = 11.0; 1700: bar 6 (5) of 6/8 at 17.0;
#   2300: bar 8 (7) of 7/8 at 2256 / 96 = 23.5; 1344: bar 5 (4) of 6/8 at 14.0; 767: bar 2 (1)
#   of 4/4 at 4.0; 0: bar 1 (0) of 4/4 at 0.0
C_TRANSPORT = [[1100, 3, 4, 11.0, 3], [1700, 6, 8, 17.0, 5], [2300, 7, 8, 23.5, 7],
               [1344, 6, 8, 14.0, 4], [767, 4, 4, 4.0, 1], [0, 4, 4, 0.0, 0]]
for t, b, u, start, n in C_TRANSPORT:
    assert bar_at(MAP, t) == (n, start * 96, (b, u))

# p35_meter.json: the map, "kick", pattern 1 (the project's 4/4, one bar, a hit at 0), one clip
# of it from 0 for 2592 ticks (eight bars of the map): the pattern plays every 384 ticks (its own
# length, whatever the song's meter), hits at 0, 384, ..., 2304 (2688 is past the clip).
METER = project([pattern(1, "Pattern 1", 1, [note(1, 0, 0, 1.0)])],
                [pclip(1, 1, 0, 2592, 1)], changes=CHANGES)
assert METER["format_version"] == 10
HITS = list(range(0, 2592, 384))
assert HITS == [0, 384, 768, 1152, 1536, 1920, 2304]
# render --song --bars 6: bar 7 starts at 1920 -> s(1920) = 441000 frames; the hits before it:
# 0, 384, 768, 1152, 1536 -> frames 0, 88200, 176400, 264600, 352800.
C_RENDER = {"args": ["--song", "--bars", "6"], "frames": 441000,
            "onsets": [s(t) for t in HITS if t < 1920]}
assert C_RENDER["frames"] == s(BARS[6]) and C_RENDER["onsets"] == [0, 88200, 176400, 264600, 352800]
# p35_meter_stems.json: the map, the clip from 0 for 1700 ticks (hits 0 ... 1536): the song ends
# at 1700, in bar 6 (1632..1920), so every stem is s(1920) = 441000 frames (a 4/4 grid would
# give s(2304) = 529200).
STEMS = project([pattern(1, "Pattern 1", 1, [note(1, 0, 0, 1.0)])],
                [pclip(1, 1, 0, 1700, 1)], changes=CHANGES)
C_STEMS = {"files": ["01 Track 1.wav", "master.wav"], "frames": 441000}
assert bar_at(MAP, 1700)[1] == 1632 and BARS[6] == 1920 and s(1920) == 441000

# The song's MIDI file: the conductor holds tempo + 4/4 at 0 and FF 58 at 768 (3/4), 1344 (6/8),
# 1920 (7/8), and ends with the song's last note-off at 2304 + 24 = 2328 (after every change);
# kick plays each hit one step long.
METER_SONG = smf1([
    [(0, TEMPO_120), (0, TS[(4, 4)]), (768, TS[(3, 4)]), (1344, TS[(6, 8)]),
     (1920, TS[(7, 8)]), (2328, EOT)],
    [(0, name("kick"))] + [ev for t in HITS for ev in ((t, on(60, 127)), (t + 24, off(60)))]
    + [(2328, EOT)],
])


def daw_note(time, duration, key, vel):
    return ('              <Note time="%s" duration="%s" channel="0" key="%d" vel="%s"/>\n'
            % (time, duration, key, vel))


# DAWproject of p35_meter.json: the Transport's 4/4 with id="time-signature", the one clip at
# 0.0 for 2592 / 96 = 27.0 quarters, its notes at the hits / 96 = 0, 4, ..., 24, each a step
# (0.25) long; then the TimeSignatureAutomation: points at 0 / 96 = 0.0 (4/4), 768 / 96 = 8.0
# (3/4), 1344 / 96 = 14.0 (6/8), 1920 / 96 = 20.0 (7/8).
METER_XML = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0">
  <Application name="Dawsome" version="{version}"/>
  <Transport>
    <Tempo unit="bpm" value="120.0"/>
    <TimeSignature numerator="4" denominator="4" id="time-signature"/>
  </Transport>
  <Structure>
    <Track contentType="notes" id="track1" name="kick">
      <Channel audioChannels="2" destination="master" role="regular" solo="false" id="channel1">
        <Mute value="false"/>
        <Pan unit="normalized" value="0.5" min="0.0" max="1.0"/>
        <Volume unit="linear" value="1.0" min="0.0" max="4.0"/>
      </Channel>
    </Track>
    <Track contentType="audio notes" id="master-track" name="Master">
      <Channel audioChannels="2" role="master" solo="false" id="master">
        <Mute value="false"/>
        <Pan unit="normalized" value="0.5" min="0.0" max="1.0"/>
        <Volume unit="linear" value="1.0" min="0.0" max="4.0"/>
      </Channel>
    </Track>
  </Structure>
  <Arrangement>
    <Lanes timeUnit="beats">
      <Lanes track="track1">
        <Clips>
          <Clip time="0.0" duration="27.0" name="Pattern 1">
            <Notes>
""" + "".join(daw_note("%d.0" % (t // 96), "0.25", 60, "1.0") for t in HITS) + """            </Notes>
          </Clip>
        </Clips>
      </Lanes>
    </Lanes>
    <TimeSignatureAutomation timeUnit="beats">
      <Target parameter="time-signature"/>
      <TimeSignaturePoint time="0.0" numerator="4" denominator="4"/>
      <TimeSignaturePoint time="8.0" numerator="3" denominator="4"/>
      <TimeSignaturePoint time="14.0" numerator="6" denominator="8"/>
      <TimeSignaturePoint time="20.0" numerator="7" denominator="8"/>
    </TimeSignatureAutomation>
  </Arrangement>
</Project>
"""
SAMPLER_LINE = ("kick: a sampler playing impulse.wav; DAWproject has no sampler, so its track "
                "has no instrument")
assert [t // 96 for t, _ in MAP] == [0, 8, 14, 20]


# ---------- the edits ----------
def settle(first, changes):
    """The settle rule: each change in tick order to its bar line after the one before it."""
    out, p, sig = [], 0, first
    for at, c in sorted(changes):
        k = (at - p) // bar_of(sig)
        at = p + k * bar_of(sig) if k >= 1 else p + bar_of(sig)
        out.append((at, c)); p, sig = at, c
    return out


def snap_in(first, changes, t):
    """The bar line at or before t in the map of `first` and `changes` as they are."""
    return bar_at([(0, first)] + sorted(changes), t)[1]


def set_change(first, changes, at, sig):
    at = snap_in(first, changes, at)
    assert at > 0, "tick 0 is refused"
    return settle(first, [c for c in changes if c[0] != at] + [(at, sig)])


def move_change(first, changes, frm, to):
    moved = dict(changes)[frm]
    rest = [c for c in changes if c[0] != frm]
    to = snap_in(first, rest, to)
    assert to > 0, "tick 0 is refused"
    return settle(first, [c for c in rest if c[0] != to] + [(to, moved)])


def remove_change(first, changes, at):
    return settle(first, [c for c in changes if c[0] != at])


# By hand (C section 6):
#   set 3/4 at 800 on a 4/4 song with no change: the bar line at or before 800 is 768
#     (2 * 384): [768 3/4]
#   set 5/4 at 1000 on the map: 1000 is in the bar from 768 (3/4: 768 + 0 * 288), where the 3/4
#     change is: replaced by 5/4 (bar 480); then 1344: k = floor((1344 - 768) / 480) = 1 ->
#     1248; 1920 (6/8 from 1248, bar 288): k = floor(672 / 288) = 2 -> 1824
#   move 768 to 1000: in the map without it (4/4 from 0, the others as they are) 1000 -> 768:
#     the same map, nothing changes
#   move 768 to 1300: 1300 -> 1152 (3 * 384); 1344 (3/4 from 1152): k = floor(192 / 288) = 0 ->
#     up to 1152 + 288 = 1440; 1920 (6/8 from 1440): k = floor(480 / 288) = 1 -> 1728 (down,
#     though 2016 is nearer)
#   move 768 to 2000: without it 2000 lies in the 7/8 from 1920 (its first bar): 1920, where the
#     7/8 change is: replaced by the moved 3/4; then 1344 (4/4 from 0): k = 3 -> 1152; 1920
#     (6/8 from 1152): k = floor(768 / 288) = 2 -> 1728: [1152 6/8, 1728 3/4]
#   remove 1344: [768 3/4, 1920 7/8]: 1920 - 768 = 1152 = 4 * 288, on its line, stays
#   remove 768: 1344 (4/4 from 0): k = 3 -> 1152; 1920 (6/8 from 1152): k = 2 -> 1728
#   the project's own signature 3/4: 768 (3/4 from 0): k = floor(768 / 288) = 2 -> 576; 1344
#     (3/4 from 576): k = floor(768 / 288) = 2 -> 1152; 1920 (6/8 from 1152): k = 2 -> 1728;
#     pattern 1 (the project's, a hit at 0 ending at 1) stays one bar
C_EDITS = {
    "set_800": ((4, 4), [], ("set", 800, (3, 4)), [(768, (3, 4))]),
    "set_54": ((4, 4), CHANGES, ("set", 1000, (5, 4)), [(768, (5, 4)), (1248, (6, 8)),
                                                         (1824, (7, 8))]),
    "move_1000": ((4, 4), CHANGES, ("move", 768, 1000), CHANGES),
    "moved": ((4, 4), CHANGES, ("move", 768, 1300), [(1152, (3, 4)), (1440, (6, 8)),
                                                     (1728, (7, 8))]),
    "moved_past": ((4, 4), CHANGES, ("move", 768, 2000), [(1152, (6, 8)), (1728, (3, 4))]),
    "removed_1344": ((4, 4), CHANGES, ("remove", 1344), [(768, (3, 4)), (1920, (7, 8))]),
    "removed_768": ((4, 4), CHANGES, ("remove", 768), [(1152, (6, 8)), (1728, (7, 8))]),
    "start_34": ((4, 4), CHANGES, ("start", (3, 4)), [(576, (3, 4)), (1152, (6, 8)),
                                                      (1728, (7, 8))]),
}
for key, (first, changes, edit, want) in C_EDITS.items():
    if edit[0] == "set": got = set_change(first, changes, edit[1], edit[2])
    elif edit[0] == "move": got = move_change(first, changes, edit[1], edit[2])
    elif edit[0] == "remove": got = remove_change(first, changes, edit[1])
    else: got = settle(edit[1], changes)
    assert got == want, (key, got)
# Refused: a set that lands on tick 0 (at 100: the first bar's line is 0), a move to the first
# bar, a remove or a move of a tick with no change.
C_REFUSED = [("set", 100, (3, 4)), ("move", 768, 200), ("remove", 800), ("move", 800, 1300)]
# Files a build refuses: a change at 0, off its bar line (800 in 4/4), out of order, two on one
# tick, a signature out of range, a pattern signature out of range.
C_BAD_FILES = [
    [{"at": 0, "beats": 3, "unit": 4}],
    [{"at": 800, "beats": 3, "unit": 4}],
    [{"at": 1344, "beats": 6, "unit": 8}, {"at": 768, "beats": 3, "unit": 4}],
    [{"at": 768, "beats": 3, "unit": 4}, {"at": 768, "beats": 6, "unit": 8}],
    [{"at": 768, "beats": 3, "unit": 2}],
]


C_EDIT_FILES = {
    "set_800": ("p35_edit_c.json", "p35_edit_c_set.json"),
    "set_54": ("p35_meter.json", "p35_meter_set_54.json"),
    "move_1000": ("p35_meter.json", "p35_meter.json"),
    "moved": ("p35_meter.json", "p35_meter_moved.json"),
    "moved_past": ("p35_meter.json", "p35_meter_moved_past.json"),
    "removed_1344": ("p35_meter.json", "p35_meter_removed_1344.json"),
    "removed_768": ("p35_meter.json", "p35_meter_removed_768.json"),
    "start_34": ("p35_meter.json", "p35_meter_start_34.json"),
}


def edit_json(edit):
    if edit[0] == "set": return {"set": {"at": edit[1], "signature": list(edit[2])}}
    if edit[0] == "move": return {"move": {"from": edit[1], "to": edit[2]}}
    if edit[0] == "remove": return {"remove": {"at": edit[1]}}
    return {"start": {"signature": list(edit[1])}}


def expected():
    return {
        "b": {
            "project": "p35_pattern_34.json",
            "renders": B_RENDERS,
            "metronome": {k: {"mode": v["mode"], "frames": v["frames"],
                              "nonzero": clicks_render(v["frames"], v["beats"])}
                          for k, v in B_METRONOME.items()},
            "count_in": B_COUNT_INS,
            "transport": B_TRANSPORT,
            "readouts": B_READOUTS,
            "steps": B_STEPS,
            "midi": {"pattern": ["--pattern", "2", "p35_pattern_34_p2.mid"],
                     "song": "p35_pattern_34_song.mid",
                     "pattern_time_signature": TS[(3, 4)], "song_time_signature": TS[(4, 4)]},
            "edit": {"before": "p35_edit_b.json", "after": "p35_edit_b_34.json",
                     "none": "p35_edit_b_none.json", "pattern": 1, "signature": [3, 4]},
        },
        "c": {
            "project": "p35_meter.json",
            "changes": [[at, list(sig)] for at, sig in CHANGES],
            "bars": BARS,
            "bar_at": BAR_AT,
            "readouts": C_READOUTS,
            "snaps": SNAPS,
            "metronome": {"frames": C_METRONOME["frames"],
                          "nonzero": clicks_render(C_METRONOME["frames"],
                                                   C_METRONOME["beats"])},
            "count_in": C_COUNT_INS,
            "transport": C_TRANSPORT,
            "render": C_RENDER,
            "stems": dict(C_STEMS, project="p35_meter_stems.json"),
            "midi": {"song": "p35_meter_song.mid",
                     "time_signatures": [[at, TS[sig]] for at, sig in MAP]},
            "dawproject": {"entries": [["project.xml", "p35_meter.project.xml"],
                                       ["metadata.xml", None]],
                           "metadata": ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
                                        '<MetaData>\n  <Title>p35_meter</Title>\n</MetaData>\n'),
                           "report": [SAMPLER_LINE]},
            "edits": [dict(edit_json(C_EDITS[k][2]), name=k, before=b, after=a)
                      for k, (b, a) in C_EDIT_FILES.items()],
            "refused": [edit_json(e) for e in C_REFUSED],
            "bad_files": C_BAD_FILES,
        },
    }


def files():
    w = lambda d: (J(d) + "\n").encode()
    out = {
        "p35_pattern_34.json": w(P34),
        "p35_pattern_34_p2.mid": P34_PATTERN_2,
        "p35_pattern_34_song.mid": P34_SONG,
        "p35_edit_b.json": w(project(EDIT_B)),
        "p35_edit_b_34.json": w(project(EDIT_B_34)),
        "p35_edit_b_none.json": w(project(EDIT_B_NONE)),
        "p35_meter.json": w(METER),
        "p35_meter_song.mid": METER_SONG,
        "p35_meter.project.xml": METER_XML.replace("{version}", version()).encode("utf-8"),
        "p35_meter_stems.json": w(STEMS),
        "p35_edit_c.json": w(project([pattern(1, "Pattern 1", 1, [note(1, 0, 0, 1.0)])],
                                     [pclip(1, 1, 0, 2592, 1)])),
    }
    out["p35_edit_c_set.json"] = w(project([pattern(1, "Pattern 1", 1, [note(1, 0, 0, 1.0)])],
                                           [pclip(1, 1, 0, 2592, 1)],
                                           changes=C_EDITS["set_800"][3]))
    for key, (_, after) in C_EDIT_FILES.items():
        if key in ("set_800", "move_1000"): continue
        first = (3, 4) if key == "start_34" else (4, 4)
        out[after] = w(project(METER["patterns"], METER["playlist"]["clips"], signature=first,
                               changes=C_EDITS[key][3]))
    out["p35_expected.json"] = w(expected())
    return out


def main():
    args = sys.argv[1:]
    if len(args) == 2 and args[0] in ("--write", "--check"):
        stale = []
        for path, content in sorted(files().items()):
            target = os.path.join(args[1], path)
            if args[0] == "--write":
                with open(target, "wb") as f: f.write(content)
            else:
                try:
                    with open(target, "rb") as f: same = f.read() == content
                except OSError: same = False
                if not same: stale.append(path)
        if stale: sys.exit("not what gen_p35_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    print("bars of the map: %s" % BARS)
    for t, want in C_READOUTS: print("readout %d: %s" % (t, want))
    for key, (first, changes, edit, want) in C_EDITS.items():
        print("%s: %s -> %s" % (key, edit, want))
    print("p35_pattern_34 pattern 2 MIDI: %s" % P34_PATTERN_2.hex())
    print("p35_meter song MIDI: %s" % METER_SONG.hex())


if __name__ == "__main__":
    main()
