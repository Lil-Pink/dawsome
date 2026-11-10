"""Known answers for time signatures other than 4/4 (one signature per project): the beat and
the bar a signature makes, the metronome and the count-in it clicks, the pattern length and the
render length it sets, the project file that keeps it, the edit that sets it, the MIDI file and
the DAWproject that carry it, the transport a plugin is given, the position readout and the step
grid.

Expected values come from these rules, never from engine code:
  signature     beats/unit, beats 1 to 24, unit 4, 8 or 16 (a beat is a quarter, an eighth or a
                sixteenth note); typed as "3/4", spaces allowed around the slash and the whole,
                digits only; anything else is refused with one message:
                "Type a time signature such as 3/4 or 6/8: 1 to 24 beats over 4, 8 or 16"
  beat, bar     at 96 ppq a quarter note is 96 ticks: beat = 96*4/unit, bar = beats*beat
  steps         a step is a 16th, 24 ticks, in every meter: steps a beat = 16/unit (4, 2, 1),
                steps a bar = beats*16/unit; the rack's groups are a beat wide; the rack holds
                512 steps, so a pattern there has at most 512 // steps-a-bar bars
  tempo         milli-BPM of quarter notes in every meter (never of the beat's note)
  tick->sample  s(t) = floor(t*60*sr*1000 / (mbpm*96))
  metronome     a click from the first sample of every beat the song position plays (a tick
                that is a multiple of the beat), accented where the tick is a multiple of the
                bar; the click is gen_p14's burst: 64 frames, frame n = A*(1 - n/64)*(+1 if
                (n//8) even else -1), A = 0.25 accented, 0.125 otherwise
  count-in      one bar: `beats` clicks, click k at s(k*beat) from the count-in's start, the
                first accented; the song (and the recording) starts at s(bar)
  pattern       a pattern of N bars is N*bar ticks; looped, its pass k starts at s(k*bar);
                `dawsome render --bars N` renders s(N*bar) frames
  file          the field "time_signature":{"beats":B,"unit":U} right after "ppq", written only
                when the signature is not 4/4, and then the file's format_version is 10 (above
                every other version: a build before it refuses the file by its number); a 4/4
                project's bytes are what they were
  edit          setting the signature changes no clip; a pattern grows to the whole bars (of the
                new signature) that hold the end of every note it played (a note that starts
                inside its old length: start + length, a step hit a tick, as a drawn note
                grows a pattern); it never gets shorter; undo puts back the signature and the
                lengths; setting the signature it has changes nothing
  MIDI          at tick 0 the time signature meta event FF 58 04 nn dd cc bb: nn = beats,
                dd = log2(unit), cc = MIDI clocks per metronome click = 24 a quarter * 4/unit
                = 96/unit (the metronome clicks the beat's note), bb = 8 (32nds a quarter); in
                the pattern file (format 0) and in the song file's conductor track (format 1),
                each otherwise as gen_p18 writes them: tempo FF 51, the time signature, the
                pattern's name (FF 03) in the pattern file; a step hit one step (24 ticks)
                long; velocity byte round half up(v*128) to 127; at one tick note-offs (velocity
                64) first; a note at or past the pattern's end is silent and left out; a song
                clip plays pattern tick (T - start) mod L, L the pattern's length in this
                signature; every track ends (FF 2F) on the last note-off
  DAWproject    <TimeSignature numerator="B" denominator="U"/> in the Transport; the rest as
                gen_p18's E3 (times in quarter notes, a pattern clip's notes unrolled with the
                pattern's length in this signature)
  plugins       the bar a block's first tick is in: bar = beats*4/unit quarter notes,
                bar number = floor(tick / bar ticks), bar start = bar number * bar in quarter
                notes; numerator = beats, denominator = unit (CLAP's transport event and VST3's
                ProcessContext)
  readout       a position bar:step:tick from 1:01:00: bar = t // bar + 1, step =
                (t mod bar) // 24 + 1 in two digits, tick = t mod 24 in two digits; a length the
                same counted from 0:00:00 (no +1)

Mutation checks (each breaks a known answer here): a bar taken as 16 steps whatever the
signature (the 3/4 render length, the readout, the step count); cc written 24 whatever the unit
(the 6/8 and 15/16 MIDI bytes); the accent on every fourth beat (the 3/4 and 6/8 clicks); a
count-in of four clicks (the 3/4 and 7/8 count-ins); the field written for 4/4 or version 10
written for 4/4 (the 4/4 file's bytes); the pattern of a clip looping at 384 ticks (the song
MIDI, the DAWproject notes); the bar start a multiple of 4 quarters (the 6/8, 7/8 and 15/16
transport); a pattern shortened by the edit (pattern 2 of p34_edit).

Files:
  p34_click_34.json, p34_click_68.json   the metronome: an empty one-bar pattern at 120 BPM,
                                         44.1 kHz, in 3/4 (two bars of clicks) and 6/8 (four)
  p34_count_78.json                      the count-in in 7/8 (the click projects serve 3/4 and
                                         6/8)
  p34_onset_34.json, p34_onset_78.json   a one-bar pattern with a step hit at tick 0 (impulse.wav,
                                         velocity 1, unit gain) at 130 BPM, in 3/4 and 7/8
  p34_file_44.json, p34_file_34.json     the same project in 4/4 (version 2) and 3/4 (version 10)
  p34_edit.json, p34_edit_34.json        a project before and after setting 3/4
  p34_midi_{34,68,1516}.json (+ _pattern.mid, _song.mid)  the MIDI exports in 3/4, 6/8, 15/16
  p34_daw_68.json, p34_daw_68.project.xml  gen_p18's DAW_PATTERN in 6/8
  p34_expected.json                      the table, the typed texts, the renders, the count-ins,
                                         the transport cases, the readouts and the step grids

Run:
  python3 -I gen_p34_expected.py              print the known answers
  python3 -I gen_p34_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p34_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import json, os, re, struct, sys

PPQ = 96; SR = 44100; STEP = 24
CLICK_FRAMES = 64
MESSAGE = "Type a time signature such as 3/4 or 6/8: 1 to 24 beats over 4, 8 or 16"


def J(d): return json.dumps(d, separators=(",", ":"), ensure_ascii=False)
def s(t, mbpm): return t * 60 * SR * 1000 // (mbpm * PPQ)


def version():
    """The workspace's version, as Cargo.toml states it: what DAWproject's Application names."""
    here = os.path.dirname(os.path.abspath(__file__))
    with open(os.path.join(here, "..", "..", "Cargo.toml"), encoding="utf-8") as f:
        text = f.read()
    section = text.split("[workspace.package]", 1)[1]
    return re.search(r'^version = "([^"]+)"', section, re.M).group(1)


# ---------- 1. the beat and the bar ----------
# (beats, unit): beat ticks, bar ticks, steps a bar, steps a beat (the rack's group), most bars
# in the rack (512 // steps a bar). By hand at 96 ppq:
#   4/4:   beat 96*4/4 = 96,   bar 4*96 = 384,    steps 4*16/4 = 16,  group 16/4 = 4, 512//16 = 32
#   3/4:   beat 96,            bar 3*96 = 288,    steps 3*4 = 12,     group 4,  512//12 = 42 (504)
#   6/8:   beat 96*4/8 = 48,   bar 6*48 = 288,    steps 6*16/8 = 12,  group 16/8 = 2,  42
#   7/8:   beat 48,            bar 7*48 = 336,    steps 7*2 = 14,     group 2,  512//14 = 36 (504)
#   5/4:   beat 96,            bar 5*96 = 480,    steps 5*4 = 20,     group 4,  512//20 = 25 (500)
#   12/8:  beat 48,            bar 12*48 = 576,   steps 12*2 = 24,    group 2,  512//24 = 21 (504)
#   15/16: beat 96*4/16 = 24,  bar 15*24 = 360,   steps 15*1 = 15,    group 16/16 = 1, 512//15 = 34 (510)
#   24/4:  beat 96,            bar 24*96 = 2304,  steps 24*4 = 96,    group 4,  512//96 = 5 (480)
#   1/16:  beat 24,            bar 24,            steps 1,            group 1,  512
TABLE = [
    ([4, 4], 96, 384, 16, 4, 32),
    ([3, 4], 96, 288, 12, 4, 42),
    ([6, 8], 48, 288, 12, 2, 42),
    ([7, 8], 48, 336, 14, 2, 36),
    ([5, 4], 96, 480, 20, 4, 25),
    ([12, 8], 48, 576, 24, 2, 21),
    ([15, 16], 24, 360, 15, 1, 34),
    ([24, 4], 96, 2304, 96, 4, 5),
    ([1, 16], 24, 24, 1, 1, 512),
]
for (b, u), beat, bar, steps, group, most in TABLE:  # the hand values against the rules
    assert beat == PPQ * 4 // u and bar == b * beat and steps == b * 16 // u
    assert group == 16 // u and most == 512 // steps and bar == steps * STEP

# Typed texts: what each reads as, or None for the refusal (MESSAGE). Accepted: the digits,
# spaces around the slash and the whole. Refused: 0 beats, 25 beats, the units 2, 3 and 32 (out
# of the ranges), no slash, an empty side, a letter, two slashes, a sign, a decimal point, a
# number past a byte.
TYPED = [
    ["3/4", [3, 4]], ["6/8", [6, 8]], [" 6 / 8 ", [6, 8]], ["12/8", [12, 8]], ["24/16", [24, 16]],
    ["1/4", [1, 4]], ["15/16", [15, 16]], ["04/4", [4, 4]],
    ["0/4", None], ["25/4", None], ["3/2", None], ["3/3", None], ["3/32", None], ["", None],
    ["3", None], ["3/", None], ["/4", None], ["x/4", None], ["3/4/4", None], ["+3/4", None],
    ["3.0/4", None], ["256/4", None], ["3/-4", None],
]

# ---------- 2. the metronome ----------
# 120 BPM at 44.1 kHz: s(t) = t*60*44100*1000 / (120000*96) = 229.6875 t, so s(96) = 22050,
# s(48) = 11025, s(24) = 5512.5 -> 5512.
#   3/4, two bars (576 ticks, s(576) = 132300 frames): clicks at ticks 0, 96, ..., 480, frames
#        0, 22050, 44100, 66150, 88200, 110250; accented at ticks 0 and 288 (frames 0, 66150).
#   6/8, four bars (1152 ticks, s(1152) = 264600 frames): clicks every 48 ticks, k = 0..23,
#        frames 11025 k; accented where 48k is a multiple of 288: k = 0, 6, 12, 18.
CLICKS = {
    "click_34": {"signature": [3, 4], "bars": 2, "frames": 132300,
                 "beats": [[0, True], [22050, False], [44100, False], [66150, True],
                           [88200, False], [110250, False]]},
    "click_68": {"signature": [6, 8], "bars": 4, "frames": 264600,
                 "beats": [[11025 * k, k % 6 == 0] for k in range(24)]},
}
for case in CLICKS.values():  # the hand frames against s(t)
    (b, u), beat = case["signature"], PPQ * 4 // case["signature"][1]
    bar = b * beat
    assert case["frames"] == s(case["bars"] * bar, 120000)
    assert case["beats"] == [[s(t, 120000), t % bar == 0]
                             for t in range(0, case["bars"] * bar, beat)]


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


# ---------- 3. the count-in ----------
# 120 BPM, as above:
#   3/4: 3 clicks at s(0) = 0, s(96) = 22050, s(192) = 44100; the song starts at s(288) = 66150
#   6/8: 6 clicks at s(48k) = 11025 k for k = 0..5 (0 ... 55125); starts at s(288) = 66150
#   7/8: 7 clicks at 11025 k for k = 0..6 (0 ... 66150); starts at s(336) = 77175
COUNT_INS = {
    "34": {"fixture": "p34_click_34.json", "clicks": [0, 22050, 44100], "start": 66150},
    "68": {"fixture": "p34_click_68.json", "clicks": [11025 * k for k in range(6)],
           "start": 66150},
    "78": {"fixture": "p34_count_78.json", "clicks": [11025 * k for k in range(7)],
           "start": 77175},
}
for name, case in COUNT_INS.items():
    b, u = {"34": (3, 4), "68": (6, 8), "78": (7, 8)}[name]
    beat = PPQ * 4 // u
    assert case["clicks"] == [s(k * beat, 120000) for k in range(b)]
    assert case["start"] == s(b * beat, 120000)

# ---------- 4. the pattern's length and the render's ----------
# 130 BPM at 44.1 kHz: s(t) = floor(t * 2646000000 / 12480000) = floor(212.0192307... t):
#   s(288) = floor(61061.538) = 61061, s(576) = floor(122123.077) = 122123
#   s(336) = floor(71238.462) = 71238, s(672) = floor(142476.923) = 142476
# --bars 1: one pass, the hit at frame 0 alone; --bars 2: the pattern twice, the second hit
# where the second pass starts, s(bar) (not s(384) = 81415 as a 4/4 bar would put it).
ONSETS = {
    "onset_34": {"fixture": "p34_onset_34.json", "signature": [3, 4],
                 "bars": {"1": {"frames": 61061, "onsets": [0]},
                          "2": {"frames": 122123, "onsets": [0, 61061]}}},
    "onset_78": {"fixture": "p34_onset_78.json", "signature": [7, 8],
                 "bars": {"1": {"frames": 71238, "onsets": [0]},
                          "2": {"frames": 142476, "onsets": [0, 71238]}}},
}
for case in ONSETS.values():
    b, u = case["signature"]
    bar = b * (PPQ * 4 // u)
    for n, want in case["bars"].items():
        assert want["frames"] == s(int(n) * bar, 130000)
        assert want["onsets"] == [s(k * bar, 130000) for k in range(int(n))]


# ---------- the projects ----------
def sampler(i, name):
    return {"id": i, "name": name, "kind": "sampler", "sample": "impulse.wav", "volume": 1.0,
            "pan": 0.0, "mute": False}


def note(channel, tick, length, velocity, key=None):
    d = {"channel": channel, "tick": tick, "length": length, "velocity": velocity}
    if key is not None: d["key"] = key
    return d


def head(signature, mbpm=120000, next_channel=2):
    """The project fields in the file's order: the version first, the signature after the ppq
    and only when not 4/4 (and then version 10), else version 2."""
    d = {"format_version": 2 if signature == [4, 4] else 10, "sample_rate": SR,
         "tempo_mbpm": mbpm, "ppq": PPQ}
    if signature != [4, 4]:
        d["time_signature"] = {"beats": signature[0], "unit": signature[1]}
    d["next_channel_id"] = next_channel
    return d


def playlist(clips, tracks=1):
    return {"next_track_id": tracks + 1, "next_clip_id": len(clips) + 1,
            "tracks": [{"id": i, "name": "Track %d" % i, "mute": False}
                       for i in range(1, tracks + 1)], "clips": clips}


def pclip(i, track, start, length, pattern, muted=False):
    d = {"id": i, "track": track, "start": start, "length": length, "kind": "pattern",
         "pattern": pattern}
    if muted: d["muted"] = True
    return d


def simple(signature, mbpm, notes, bars=1, clips=None, name="Pattern 1"):
    d = head(signature, mbpm)
    d["channels"] = [sampler(1, "kick")]
    d["patterns"] = [{"id": 1, "name": name, "length_bars": bars, "notes": notes}]
    if clips is not None: d["playlist"] = playlist(clips)
    return d


CLICK_PROJECTS = {
    "p34_click_34.json": simple([3, 4], 120000, []),
    "p34_click_68.json": simple([6, 8], 120000, []),
    "p34_count_78.json": simple([7, 8], 120000, []),
    "p34_onset_34.json": simple([3, 4], 130000, [note(1, 0, 0, 1.0)]),
    "p34_onset_78.json": simple([7, 8], 130000, [note(1, 0, 0, 1.0)]),
}

# ---------- 5. the file ----------
# One project, a step hit at 0 and at 192 in one bar: in 4/4 version 2 and no field (as every
# 4/4 file was written before the signature existed); in 3/4 version 10 and the field after
# "ppq". Loading either and saving it gives its bytes back; the 4/4 one with its signature set to
# 3/4 saves as the 3/4 one.
FILE_44 = simple([4, 4], 120000, [note(1, 0, 0, 1.0), note(1, 192, 0, 0.5)])
FILE_34 = simple([3, 4], 120000, [note(1, 0, 0, 1.0), note(1, 192, 0, 0.5)])

# ---------- 6. the edit ----------
# Before (4/4, bar 384): pattern 1, 1 bar, a step hit at 300; pattern 2, 2 bars, a note at 0 for
# 96 ticks; pattern 3, 1 bar, a note at 200 for 200 ticks (it ends at 400, past the bar: a bar
# was taken away after it was drawn); a clip of pattern 1 from 0 for 384 ticks.
# Set 3/4 (bar 288):
#   pattern 1: the hit at 300 ends at 301 (a step hit counts a tick); ceil(301 / 288) = 2 bars
#              (576 holds it): grows 1 -> 2
#   pattern 2: the note ends at 96; ceil(96 / 288) = 1 bar, which 2 bars hold: stays 2 (never
#              shorter)
#   pattern 3: starts at 200, inside its old 384, and ends at 400; ceil(400 / 288) = 2: grows to 2
#   the clip keeps its 384 ticks (1 1/3 passes of the 288-tick pattern).
# Undo gives back the first file's bytes; 3/4 again changes nothing.
EDIT_PATTERNS = [
    ({"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": [note(1, 300, 0, 1.0)]}, 2),
    ({"id": 2, "name": "Pattern 2", "length_bars": 2, "notes": [note(1, 0, 96, 0.5)]}, 2),
    ({"id": 3, "name": "Pattern 3", "length_bars": 1, "notes": [note(1, 200, 200, 0.5)]}, 2),
]


def edit_project(signature):
    d = head(signature)
    d["channels"] = [sampler(1, "kick")]
    d["patterns"] = [dict(p, length_bars=p["length_bars"] if signature == [4, 4] else after)
                     for p, after in EDIT_PATTERNS]
    d["playlist"] = playlist([pclip(1, 1, 0, 384, 1)])
    return d


# ---------- 7. MIDI ----------
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
# FF 58 04 nn dd cc bb, by hand:
#   3/4:   03, log2 4 = 2 -> 02, 96/4 = 24 -> 18, 08          FF 58 04 03 02 18 08
#   6/8:   06, log2 8 = 3 -> 03, 96/8 = 12 -> 0C, 08          FF 58 04 06 03 0C 08
#   15/16: 0F, log2 16 = 4 -> 04, 96/16 = 6 -> 06, 08         FF 58 04 0F 04 06 08
TIME_SIGNATURE = {"34": [0xFF, 0x58, 0x04, 0x03, 0x02, 0x18, 0x08],
                  "68": [0xFF, 0x58, 0x04, 0x06, 0x03, 0x0C, 0x08],
                  "1516": [0xFF, 0x58, 0x04, 0x0F, 0x04, 0x06, 0x08]}
# Each project: "kick", pattern 1 "Pattern 1" of one bar: step hits (v1.0, key 60) at 0, at the
# bar's last step (bar - 24) and at the bar itself (past the pattern's end: silent); one clip of
# it from 0 for two bars.
#   3/4 and 6/8 (bar 288): hits at 0, 264, 288 (silent). 15/16 (bar 360): 0, 336, 360 (silent).
# Pattern file: tempo, time signature, name, on 0 / off 24, on bar-24 / off bar; ends at bar.
#   3/4, 6/8: (0 on) (24 off) (264 on) (288 off) EOT 288. 15/16: (0) (24) (336) (360) EOT 360.
# Song file: song tick T plays pattern tick T mod bar for T in 0..2 bar: hits at 0, bar-24, bar,
# 2 bar-24 (the pattern's 288 / 360 hit is never heard), each 24 long; ends at 2 bar.
#   3/4, 6/8: 0-24, 264-288, 288-312, 552-576 (at 288 the note-off first); EOT 576.
#   15/16: 0-24, 336-360, 360-384, 696-720; EOT 720.
MIDI_CASES = {"34": ([3, 4], 288), "68": ([6, 8], 288), "1516": ([15, 16], 360)}


def midi_project(signature, bar):
    notes = [note(1, 0, 0, 1.0), note(1, bar - STEP, 0, 1.0), note(1, bar, 0, 1.0)]
    return simple(signature, 120000, notes, clips=[pclip(1, 1, 0, 2 * bar, 1)])


def midi_files(key):
    _, bar = MIDI_CASES[key]
    ts = TIME_SIGNATURE[key]
    pattern = smf0([(0, TEMPO_120), (0, ts), (0, name("Pattern 1")),
                    (0, on(60, 127)), (STEP, off(60)), (bar - STEP, on(60, 127)), (bar, off(60)),
                    (bar, EOT)])
    song = smf1([
        [(0, TEMPO_120), (0, ts), (2 * bar, EOT)],
        [(0, name("kick")), (0, on(60, 127)), (STEP, off(60)), (bar - STEP, on(60, 127)),
         (bar, off(60)), (bar, on(60, 127)), (bar + STEP, off(60)),
         (2 * bar - STEP, on(60, 127)), (2 * bar, off(60)), (2 * bar, EOT)],
    ])
    return pattern, song


# ---------- 8. DAWproject ----------
# gen_p18's DAW_PATTERN in 6/8 (bar 288): "kick", pattern 1 "Beat" (1 bar): a step hit at 0
# (v1.0) and key 64 at 192 for 96 ticks (v0.5); clip 1 track 1 from 384 for 768 ticks, clip 2
# track 2 from 1152 for 384 ticks, muted. Times in quarter notes (96 ticks each).
#   clip 1 at 384/96 = 4.0 for 768/96 = 8.0: passes start at clip ticks 0, 288, 576:
#     0 -> 0.0 (0.25 long), 192 -> 2.0 (1.0 long, ends 288, inside the clip),
#     288 -> 3.0, 480 -> 5.0 (ends 576), 576 -> 6.0, 768 -> past the clip's end: left out
#   clip 2 at 1152/96 = 12.0 for 384/96 = 4.0, enable="false": 0 -> 0.0, 192 -> 2.0,
#     288 -> 3.0 (ends 312, inside 384); 480 is past its end
DAW_68 = head([6, 8])
DAW_68.update({
    "channels": [sampler(1, "kick")],
    "patterns": [{"id": 1, "name": "Beat", "length_bars": 1,
                  "notes": [note(1, 0, 0, 1.0), note(1, 192, 96, 0.5, 64)]}],
    "playlist": playlist([pclip(1, 1, 384, 768, 1), pclip(2, 2, 1152, 384, 1, muted=True)],
                         tracks=8),
})


def daw_note(time, duration, key, vel):
    return ('              <Note time="%s" duration="%s" channel="0" key="%d" vel="%s"/>\n'
            % (time, duration, key, vel))


DAW_68_XML = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0">
  <Application name="Dawsome" version="{version}"/>
  <Transport>
    <Tempo unit="bpm" value="120.0"/>
    <TimeSignature numerator="6" denominator="8"/>
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
          <Clip time="4.0" duration="8.0" name="Beat">
            <Notes>
""" + (daw_note("0.0", "0.25", 60, "1.0") + daw_note("2.0", "1.0", 64, "0.5")
       + daw_note("3.0", "0.25", 60, "1.0") + daw_note("5.0", "1.0", 64, "0.5")
       + daw_note("6.0", "0.25", 60, "1.0")) + """            </Notes>
          </Clip>
          <Clip time="12.0" duration="4.0" enable="false" name="Beat">
            <Notes>
""" + (daw_note("0.0", "0.25", 60, "1.0") + daw_note("2.0", "1.0", 64, "0.5")
       + daw_note("3.0", "0.25", 60, "1.0")) + """            </Notes>
          </Clip>
        </Clips>
      </Lanes>
    </Lanes>
  </Arrangement>
</Project>
"""
SAMPLER_LINE = ("kick: a sampler playing impulse.wav; DAWproject has no sampler, so its track "
                "has no instrument")

# ---------- 9. the transport a plugin is given ----------
# [signature, tick, quarters, bar start, bar number, numerator, denominator], by hand at 96 ppq:
#   5/4 at 480 (5 q): bar 480 ticks = 5 q; 480 // 480 = 1 -> bar 1, starts 5.0 (a bar runs
#     from its start up to, not including, the next: tick 480 is the second bar's first tick)
#   5/4 at 479: 479 // 480 = 0 -> bar 0, 0.0 (the first bar's last tick)
#   6/8 at 480: bar 288 ticks = 3 q; 480 // 288 = 1 -> bar 1, 3.0
#   3/4 at 480: bar 288 = 3 q -> bar 1, 3.0
#   5/4 at 672 (7 q): 672 // 480 = 1 -> bar 1, 5.0
#   7/8 at 480: bar 336 = 3.5 q; 480 // 336 = 1 -> 3.5
#   15/16 at 480: bar 360 = 3.75 q; 480 // 360 = 1 -> 3.75
#   15/16 at 1080: 1080 // 360 = 3 -> 3 * 3.75 = 11.25
#   4/4 at 480: bar 384 = 4 q -> bar 1, 4.0
TRANSPORT = [
    [[5, 4], 480, 5.0, 5.0, 1],
    [[5, 4], 479, 479 / 96, 0.0, 0],
    [[6, 8], 480, 5.0, 3.0, 1],
    [[3, 4], 480, 5.0, 3.0, 1],
    [[5, 4], 672, 7.0, 5.0, 1],
    [[7, 8], 480, 5.0, 3.5, 1],
    [[15, 16], 480, 5.0, 3.75, 1],
    [[15, 16], 1080, 11.25, 11.25, 3],
    [[4, 4], 480, 5.0, 4.0, 1],
]
for (b, u), tick, q, start, number in TRANSPORT:
    bar = b * PPQ * 4 // u
    assert number == tick // bar and start == number * bar / PPQ and q == tick / PPQ

# ---------- 10. the readout ----------
# [tick, signature, position, length], by hand (step 24 ticks):
#   0 in 4/4: bar 0 // 384 + 1 = 1, step 0 // 24 + 1 = 1, tick 0 -> "1:01:00"; length "0:00:00"
#   383 in 4/4: 383 = 15*24 + 23 -> "1:16:23"; "0:15:23"
#   300 in 3/4: 300 // 288 = 1 -> bar 2; 300 - 288 = 12 -> step 0 -> "01", tick 12 -> "2:01:12";
#               length "1:00:12"
#   300 in 6/8: the same bar of 288 ticks -> "2:01:12"; "1:00:12"
#   372 in 6/8: 372 - 288 = 84 = 3*24 + 12 -> step "04", tick 12 -> "2:04:12"; "1:03:12"
#   288 in 6/8: one whole bar -> "2:01:00"; "1:00:00"
#   2303 in 24/4: bar 2304; 2303 = 95*24 + 23 -> "1:96:23" (two digits hold 96); "0:95:23"
#   359 in 15/16: bar 360; 359 = 14*24 + 23 -> "1:15:23"; "0:14:23"
#   360 in 15/16: "2:01:00"; "1:00:00"
#   575 in 12/8: bar 576; 575 = 23*24 + 23 -> "1:24:23"; "0:23:23"
READOUTS = [
    [0, [4, 4], "1:01:00", "0:00:00"],
    [383, [4, 4], "1:16:23", "0:15:23"],
    [300, [3, 4], "2:01:12", "1:00:12"],
    [300, [6, 8], "2:01:12", "1:00:12"],
    [372, [6, 8], "2:04:12", "1:03:12"],
    [288, [6, 8], "2:01:00", "1:00:00"],
    [2303, [24, 4], "1:96:23", "0:95:23"],
    [359, [15, 16], "1:15:23", "0:14:23"],
    [360, [15, 16], "2:01:00", "1:00:00"],
    [575, [12, 8], "1:24:23", "0:23:23"],
]
for t, (b, u), pos, length in READOUTS:
    bar = b * PPQ * 4 // u
    assert pos == "%d:%02d:%02d" % (t // bar + 1, t % bar // STEP + 1, t % STEP)
    assert length == "%d:%02d:%02d" % (t // bar, t % bar // STEP, t % STEP)

# ---------- 11. the step grid ----------
# [signature, bars, steps, a note's tick, the step it lights, groups (steps each)], by hand:
#   6/8, 1 bar: 12 steps; tick 144 / 24 = step 6; groups of 2: 6 of them
#   3/4, 1 bar: 12 steps; tick 96 -> step 4 (the second beat's first); groups of 4: 3
#   7/8, 2 bars: 2*14 = 28 steps; tick 336 (bar 2) -> step 14; groups of 2: 14
#   15/16, 1 bar: 15 steps; tick 336 -> step 14; groups of 1: 15 (no gap between them)
#   4/4, 1 bar: 16 steps; tick 288 -> step 12; groups of 4: 4
STEP_GRIDS = [
    [[6, 8], 1, 12, 144, 6, [2] * 6],
    [[3, 4], 1, 12, 96, 4, [4] * 3],
    [[7, 8], 2, 28, 336, 14, [2] * 14],
    [[15, 16], 1, 15, 336, 14, [1] * 15],
    [[4, 4], 1, 16, 288, 12, [4] * 4],
]
for (b, u), bars, steps, tick, lit, groups in STEP_GRIDS:
    assert steps == bars * b * 16 // u and lit == tick // STEP and sum(groups) == steps
    assert set(groups) == {16 // u}


def expected():
    renders = {}
    for key, case in CLICKS.items():
        renders[key] = {"fixture": "p34_%s.json" % key, "frames": case["frames"],
                        "nonzero": clicks_render(case["frames"], case["beats"])}
    return {
        "message": MESSAGE,
        "table": [{"signature": sig, "beat": beat, "bar": bar, "steps": steps, "group": group,
                   "max_step_bars": most}
                  for sig, beat, bar, steps, group, most in TABLE],
        "typed": TYPED,
        "metronome": renders,
        "click_frames": CLICK_FRAMES,
        "count_in": COUNT_INS,
        "onsets": ONSETS,
        "file": {"common": "p34_file_44.json", "three_four": "p34_file_34.json"},
        "edit": {"before": "p34_edit.json", "after": "p34_edit_34.json", "signature": [3, 4],
                 "clip_length": 384},
        "midi": {key: {"project": "p34_midi_%s.json" % key,
                       "pattern": "p34_midi_%s_pattern.mid" % key,
                       "song": "p34_midi_%s_song.mid" % key,
                       "time_signature": TIME_SIGNATURE[key]} for key in MIDI_CASES},
        "dawproject": {"project": "p34_daw_68.json",
                       "entries": [["project.xml", "p34_daw_68.project.xml"],
                                   ["metadata.xml", None]],
                       "metadata": ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
                                    '<MetaData>\n  <Title>p34_daw_68</Title>\n</MetaData>\n'),
                       "report": [SAMPLER_LINE]},
        "transport": [{"signature": sig, "tick": tick, "quarters": q, "bar_start": start,
                       "bar_number": number} for sig, tick, q, start, number in TRANSPORT],
        "readouts": [{"tick": t, "signature": sig, "position": pos, "length": length}
                     for t, sig, pos, length in READOUTS],
        "step_grids": [{"signature": sig, "bars": bars, "steps": steps, "tick": tick,
                        "lit": lit, "groups": groups}
                       for sig, bars, steps, tick, lit, groups in STEP_GRIDS],
    }


def files():
    out = {name_: (J(d) + "\n").encode() for name_, d in CLICK_PROJECTS.items()}
    out["p34_file_44.json"] = (J(FILE_44) + "\n").encode()
    out["p34_file_34.json"] = (J(FILE_34) + "\n").encode()
    out["p34_edit.json"] = (J(edit_project([4, 4])) + "\n").encode()
    out["p34_edit_34.json"] = (J(edit_project([3, 4])) + "\n").encode()
    for key, (signature, bar) in MIDI_CASES.items():
        out["p34_midi_%s.json" % key] = (J(midi_project(signature, bar)) + "\n").encode()
        pattern, song = midi_files(key)
        out["p34_midi_%s_pattern.mid" % key] = pattern
        out["p34_midi_%s_song.mid" % key] = song
    out["p34_daw_68.json"] = (J(DAW_68) + "\n").encode()
    out["p34_daw_68.project.xml"] = DAW_68_XML.replace("{version}", version()).encode("utf-8")
    out["p34_expected.json"] = (J(expected()) + "\n").encode()
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
        if stale: sys.exit("not what gen_p34_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    for (b, u), beat, bar, steps, group, most in TABLE:
        print("%d/%d: beat %d, bar %d, %d steps in groups of %d, %d bars at most"
              % (b, u, beat, bar, steps, group, most))
    for key, case in CLICKS.items():
        print("%s: %d frames, clicks at %s" % (key, case["frames"], case["beats"]))
    for key, case in COUNT_INS.items():
        print("count-in %s: clicks at %s, starts at %d" % (key, case["clicks"], case["start"]))
    for key in MIDI_CASES:
        pattern, song = midi_files(key)
        print("midi %s: pattern %s\n         song %s" % (key, pattern.hex(), song.hex()))


if __name__ == "__main__":
    main()
