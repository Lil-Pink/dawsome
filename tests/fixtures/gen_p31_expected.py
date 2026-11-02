"""Known answers for swing: a pattern's `swing_tenths` plays every note on the second 16th of
an 8th-note pair late, at compile time, so playback hears it and the stored notes stay straight.

Expected values come from these rules, never from engine code; the MIDI bytes are built from
event lists written out by hand under the cases, checked against the rules in sanity():
  tick->sample  s(t) = floor(t*60*SR*1000 / (mbpm*96))            (the one integer rule)
  step, pair    U = 96/4 = 24 ticks (a 16th), P = 2U = 48 (an 8th)
  swing S       tenths of a per cent, 500..=750: the share of each pair its first 16th takes
                (500 straight, 667 a triplet feel, 750 dotted); 500 is written nowhere
  offset        o(U, S) = floor(2U*S / 1000): where the second 16th of a pair plays, from the
                pair's start (24 at 500, 32 at 667, 36 at 750)
  swung tick    a note whose stored tick t has t mod P = U plays at t - U + o(U, S); every other
                note (on-beats, notes off the step grid) plays at t
  length        a note keeps its length (its end moves with its start); a step hit (length 0)
                lasts one step, U, unless swing moved it: then it lasts 2U - o(U, S) (U - d for
                a delay d), so it still ends on the next step (16 at 667, 12 at 750)
  pattern mode  pass k (k from 0) plays the pattern's tick p at absolute tick k*L + p, L its
                length in ticks, so the second bar is floored from the absolute tick
  song mode     a pattern clip at song tick c, offset o, length n plays song tick T in c..c+n as
                the pattern's swung tick (T - c + o) mod L: swung tick p first at
                c + (p - o) mod L, then every L ticks while before c + n
  pattern MIDI  smf-export (gen_p18_expected.py's pattern export): format 0 at division 96,
                tick 0 the tempo (FF 51 07 A1 20 at 120 BPM), 4/4 (FF 58 04 04 02 18 08) and the
                pattern's name (FF 03), each note on MIDI channel 1 from its swung tick for the
                length it plays (note-on 90 3C 7F: velocity round half up(1.0 * 128) -> 127;
                note-off 80 3C 40), at one tick note-offs first, the track ending (FF 2F) on its
                last event; no same-key rule
  song MIDI     export-midi (gen_p18_expected.py's E1): format 1, track 0 the conductor (the
                tempo and 4/4), then a track per channel named after it, the notes the clips
                play as song mode plays them; per key, a note still sounding when the same key
                starts again ends there; every track ends on the song's last note-off
  sounds        p31_impulse.wav (one frame of 1.0, 48 kHz) at volume 1, centered (the pan law
                gives exactly 1 on both sides): 1.0 on the onset's frame, both sides;
                p31_dc.wav (48000 frames of 0.5) with the gate "hold" (gen_p13_expected.py's
                rule: cut at the note's end): 0.5 on frames s(T) to s(T + length) - 1

Cases (48 kHz, sampler "hat" on p31_impulse.wav, one-bar patterns; at 120 BPM s(t) = 250t):
  p31_plain.json       (a) the 16 steps, no swing written; two passes
  p31_straight.json    (a) the same with "swing_tenths":500 written: the same render, and saved
                       it is p31_plain.json's bytes (500 is the default, left out)
  p31_triplet_120.json (b) the 16 steps at 667, 120 BPM: ticks 48p and 48p + 32; two passes
  p31_dotted_130.json  (c) the 16 steps at 750, 130 BPM: ticks 48p and 48p + 36, each floored
                       from the absolute tick; two passes
  p31_offgrid.json     (d) 750, 120 BPM: hat hits at 24 (swung to 36) and 30 (off the grid,
                       kept), so the swung note passes the hand-placed one; channel 2 "pad" on
                       p31_dc.wav, held, one note at 120 (= 96 + 24, swung to 132) of length 48,
                       which ends at 180: the note-off moved with the start; one pass
  p31_song.json        (e) 667, 120 BPM, song mode: hat hits at 0, 24 and 72 (swung 0, 32, 80);
                       clip A at 0, offset 28, length 384; clip B at 384, offset 0, length 768
                       (the pattern twice); three bars
  p31_export.json      (f) 667: hat hits at 0 (a step), 24 (length 12), 96 and 168 (steps), no
                       playlist, through smf-export: p31_export.mid
  p31_export_song.json (f) the same pattern in one clip (start 0, length 384), through
                       export-midi: p31_export_song.mid
  p31_overlap_song.json (f) the 16 steps at 667 in one clip (start 0, length 384), through
                       export-midi (p31_overlap_song.mid) and smf-export (p31_overlap_pattern.mid):
                       the moved step hits last 16 ticks, so no two hits of the key overlap
  p31_expected.json    each render's frames, mode, bars and sounding runs [first frame, frames,
                       value on both sides]; the compiled events of (d) and the song notes of
                       (e) as (tick, length) and (start, end); each export's project, file and
                       (for export-midi) report lines

Run:
  python3 -I gen_p31_expected.py              print the known answers
  python3 -I gen_p31_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p31_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import json, os, struct, sys

PPQ = 96; SR = 48000; BAR = 4 * PPQ
STEP = PPQ // 4; PAIR = 2 * STEP
STRAIGHT = 500
DC_FRAMES = 48000


def s(t, mbpm): return t * 60 * SR * 1000 // (mbpm * PPQ)
def J(d): return json.dumps(d, separators=(",", ":"))


# ---------- the swing rule ----------
def offset(swing): return 2 * STEP * swing // 1000


def swung(t, swing): return t - STEP + offset(swing) if t % PAIR == STEP else t


def played(t, length, swing):
    """A stored note (tick t, length) as it plays: (tick, length heard), a step hit's length
    one step, or 2U - o(U, S) when swing moved it."""
    at = swung(t, swing)
    if length > 0: return at, length
    return at, (2 * STEP - offset(swing) if at != t else STEP)


# ---------- the sounds ----------
def wav_bytes(frames):
    """A mono 32-bit float WAV at SR."""
    data = struct.pack("<%df" % len(frames), *frames)
    fmt = struct.pack("<HHIIHH", 3, 1, SR, SR * 4, 4, 32)
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack("<I", len(data)) + data
    return b"RIFF" + struct.pack("<I", len(body)) + body


IMPULSE = [1.0]
DC = [0.5] * DC_FRAMES
HOLD = {"gate": "hold", "semitones": 0, "cents": 0, "velocity": 1.0, "attack": 0.0,
        "release": 0.0, "reverse": False, "start": 0.0, "filter": False, "cutoff": 20000.0,
        "resonance": 0.0}


# ---------- the cases ----------
HAT, PAD = 1, 2
SIXTEEN = [(HAT, STEP * k, 0) for k in range(16)]
# case: (tempo, swing, swing written when 500, notes (channel, tick, length), clips (start,
# offset, length) for song mode or None, passes or bars)
CASES = {
    "plain": (120000, STRAIGHT, False, SIXTEEN, None, 2),
    "straight": (120000, STRAIGHT, True, SIXTEEN, None, 2),
    "triplet_120": (120000, 667, False, SIXTEEN, None, 2),
    "dotted_130": (130000, 750, False, SIXTEEN, None, 2),
    "offgrid": (120000, 750, False, [(HAT, 24, 0), (HAT, 30, 0), (PAD, 120, 48)], None, 1),
    "song": (120000, 667, False, [(HAT, 0, 0), (HAT, 24, 0), (HAT, 72, 0)],
             [(0, 28, 384), (384, 0, 768)], 3),
}


# (f) the exports, at 667 and 120 BPM, one bar, a clip (start, offset, length) for export-midi
EXPORT_NOTES = [(HAT, 0, 0), (HAT, 24, 12), (HAT, 96, 0), (HAT, 168, 0)]
ONE_CLIP = [(0, 0, 384)]
EXPORTS = {
    "export": (120000, 667, False, EXPORT_NOTES, None),
    "export_song": (120000, 667, False, EXPORT_NOTES, ONE_CLIP),
    "overlap_song": (120000, 667, False, SIXTEEN, ONE_CLIP),
}


def channels(notes):
    out = [{"id": HAT, "name": "hat", "kind": "sampler", "sample": "p31_impulse.wav",
            "volume": 1.0, "pan": 0.0, "mute": False}]
    if any(channel == PAD for channel, _, _ in notes):
        out.append({"id": PAD, "name": "pad", "kind": "sampler", "sample": "p31_dc.wav",
                    "volume": 1.0, "pan": 0.0, "mute": False, "sound": HOLD})
    return out


def note_json(channel, tick, length):
    return {"channel": channel, "tick": tick, "length": length, "velocity": 1.0}


def pattern_json(swing, written, notes):
    p = {"id": 1, "name": "Pattern 1", "length_bars": 1}
    if swing != STRAIGHT or written: p["swing_tenths"] = swing
    p["notes"] = [note_json(*n) for n in sorted(notes, key=lambda n: (n[1], n[0]))]
    return p


def playlist_json(clips):
    tracks = [{"id": i, "name": "Track %d" % i, "mute": False} for i in range(1, 9)]
    out = []
    for i, (start, off, length) in enumerate(clips, 1):
        c = {"id": i, "track": 1, "start": start, "length": length, "kind": "pattern", "pattern": 1}
        if off: c["offset"] = off
        out.append(c)
    return {"next_track_id": 9, "next_clip_id": len(clips) + 1, "tracks": tracks, "clips": out}


def project(mbpm, swing, written, notes, clips):
    chans = channels(notes)
    # Version 5 holds the sound settings the held pad needs; every other case is version 2.
    d = {"format_version": 5 if len(chans) > 1 else 2, "sample_rate": SR, "tempo_mbpm": mbpm,
         "ppq": PPQ, "next_channel_id": len(chans) + 1, "channels": chans,
         "patterns": [pattern_json(swing, written, notes)]}
    if clips: d["playlist"] = playlist_json(clips)
    return d


# ---------- the renders ----------
def onsets(swing, notes, clips, count):
    """Every note the case plays as (channel, absolute tick, length heard, end tick or None):
    pattern mode `count` passes, song mode each clip's window."""
    out = []
    for channel, t, length in notes:
        at, heard = played(t, length, swing)
        if clips is None:
            out += [(channel, k * BAR + at, heard, None) for k in range(count)]
            continue
        for c, o, n in clips:
            first = c + (at - o) % BAR
            out += [(channel, T, heard, c + n) for T in range(first, c + n, BAR)]
    return out


def render(mbpm, swing, notes, clips, count):
    frames = s(count * BAR, mbpm)
    out = [0.0] * frames
    for channel, tick, heard, end in onsets(swing, notes, clips, count):
        first = s(tick, mbpm)
        if channel == HAT:
            out[first] += IMPULSE[0]
            continue
        off = tick + heard if end is None else min(tick + heard, end)
        for i in range(min(s(off, mbpm) - first, DC_FRAMES)):
            if first + i < frames: out[first + i] += DC[i]
    return frames, runs(out)


def runs(out):
    """The sounding frames as [first frame, frames, value]: each stretch of one value."""
    found = []
    for i, v in enumerate(out):
        if v == 0.0: continue
        if found and found[-1][0] + found[-1][1] == i and found[-1][2] == v: found[-1][1] += 1
        else: found.append([i, 1, v])
    return found


# ---------- the MIDI files ----------
def varlen(n):
    out = [n & 0x7F]; n >>= 7
    while n: out.append(0x80 | (n & 0x7F)); n >>= 7
    return bytes(reversed(out))


def mtrk(events):
    """A track of (absolute tick, message), with running status as the writer crate keeps it:
    a meta event cancels it, the same status byte is left out."""
    out = b""; last = 0; rs = None
    for t, msg in events:
        out += varlen(t - last); last = t
        if msg[0] == 0xFF: rs = None; out += bytes(msg)
        elif msg[0] == rs: out += bytes(msg[1:])
        else: rs = msg[0]; out += bytes(msg)
    return b"MTrk" + struct.pack(">I", len(out)) + out


def smf0(events): return b"MThd" + struct.pack(">IHHH", 6, 0, 1, PPQ) + mtrk(events)
def smf1(tracks): return b"MThd" + struct.pack(">IHHH", 6, 1, len(tracks), PPQ) + b"".join(mtrk(t) for t in tracks)
def meta(kind, data): return [0xFF, kind] + list(varlen(len(data))) + list(data)
def name(text): return meta(0x03, text.encode("utf-8"))
def on(key, vel): return [0x90, key, vel]
def off(key): return [0x80, key, 64]
TEMPO_120 = meta(0x51, [0x07, 0xA1, 0x20]); TS_44 = meta(0x58, [4, 2, 24, 8]); EOT = meta(0x2F, [])
MIDI_GENERIC = "A MIDI file holds notes and the tempo: the sounds, the mixer and automation stay in Dawsome"

# p31_export, by hand at 667 (o = 32; a moved step hit lasts 2*24 - 32 = 16):
#   0, a step on a pair's start: stays, 0 to 24
#   24, length 12: the second 16th of pair 0, plays at 24 - 24 + 32 = 32 for its 12: 32 to 44
#   96, a step on a pair's start: 96 to 120
#   168, a step: 168 mod 48 = 24, plays at 168 - 24 + 32 = 176 for 16: 176 to 192
# No two notes overlap; the last note-off is at 192. Deltas: 0 24 8 12 52 24 56 16.
EXPORT_MIDI = [(0, on(60, 127)), (24, off(60)), (32, on(60, 127)), (44, off(60)),
               (96, on(60, 127)), (120, off(60)), (176, on(60, 127)), (192, off(60))]
# p31_overlap_song, by hand at 667: pair p's first step plays 48p to 48p + 24, its second
# 48p + 32 for 16 to 48p + 48, where pair p + 1's first starts (its note-off first on that
# tick): nothing for the song export's same-key rule to cut. The last note-off is at 384.
# Deltas: 0 24 8 16, eight times.
OVERLAP_MIDI = [event for p in range(8) for event in (
    (48 * p, on(60, 127)), (48 * p + 24, off(60)), (48 * p + 32, on(60, 127)), (48 * p + 48, off(60)))]


def pattern_midi(notes, last):
    return smf0([(0, TEMPO_120), (0, TS_44), (0, name("Pattern 1"))] + notes + [(last, EOT)])


def song_midi(notes, last):
    return smf1([[(0, TEMPO_120), (0, TS_44), (last, EOT)],
                 [(0, name("hat"))] + notes + [(last, EOT)]])


MIDI_FILES = {
    "p31_export.mid": pattern_midi(EXPORT_MIDI, 192),
    "p31_export_song.mid": song_midi(EXPORT_MIDI, 192),
    "p31_overlap_song.mid": song_midi(OVERLAP_MIDI, 384),
    "p31_overlap_pattern.mid": pattern_midi(OVERLAP_MIDI, 384),
}


def rule_midi(swing, notes):
    """The (tick, message) list the rules above give a pattern's notes: each from its swung
    tick for the length it plays, note-offs first on a tick. For sanity() only."""
    timed = []
    for _, t, length in notes:
        at, heard = played(t, length, swing)
        timed += [(at, 1, on(60, 127)), (at + heard, 0, off(60))]
    return [(t, message) for t, _, message in sorted(timed)]


def expected():
    renders = {}
    for name, (mbpm, swing, _, notes, clips, count) in CASES.items():
        frames, sounding = render(mbpm, swing, notes, clips, count)
        renders[name] = {"fixture": "p31_%s.json" % name, "frames": frames,
                         "song": clips is not None, "bars": count, "runs": sounding}
    _, offgrid_swing, _, offgrid_notes, _, _ = CASES["offgrid"]
    compiled = sorted((played(t, length, offgrid_swing) for _, t, length in offgrid_notes),
                      key=lambda e: e[0])
    _, song_swing, _, song_notes, song_clips, _ = CASES["song"]
    song = sorted([T, min(T + heard, end)]
                  for _, T, heard, end in onsets(song_swing, song_notes, song_clips, 0))
    exports = {
        "pattern": {
            "export": {"project": "p31_export.json", "mid": "p31_export.mid"},
            "overlap": {"project": "p31_overlap_song.json", "mid": "p31_overlap_pattern.mid"},
        },
        "song": {
            "export": {"project": "p31_export_song.json", "mid": "p31_export_song.mid",
                       "report": [MIDI_GENERIC]},
            "overlap": {"project": "p31_overlap_song.json", "mid": "p31_overlap_song.mid",
                        "report": [MIDI_GENERIC]},
        },
    }
    return {"renders": renders, "offgrid_events": [list(e) for e in compiled],
            "song_notes": song, "exports": exports}


def files():
    out = {"p31_impulse.wav": wav_bytes(IMPULSE), "p31_dc.wav": wav_bytes(DC)}
    for name, (mbpm, swing, written, notes, clips, _) in CASES.items():
        out["p31_%s.json" % name] = (J(project(mbpm, swing, written, notes, clips)) + "\n").encode()
    for name, (mbpm, swing, written, notes, clips) in EXPORTS.items():
        out["p31_%s.json" % name] = (J(project(mbpm, swing, written, notes, clips)) + "\n").encode()
    out.update(MIDI_FILES)
    out["p31_expected.json"] = (J(expected()) + "\n").encode()
    return out


def sanity():
    """The design's tables, worked by hand, against the rules above."""
    assert [offset(v) for v in (500, 521, 520, 666, 667, 750)] == [24, 25, 24, 31, 32, 36]
    assert [2 * STEP - offset(v) for v in (500, 667, 750)] == [24, 16, 12]
    assert swung(72, 750) == 84 and swung(30, 667) == 30 and swung(48, 667) == 48
    b = expected()["renders"]["triplet_120"]["runs"]
    assert [r[0] for r in b[:6]] == [0, 8000, 12000, 20000, 24000, 32000]
    assert [r[0] for r in b[14:18]] == [84000, 92000, 96000, 104000]
    c = [r[0] for r in expected()["renders"]["dotted_130"]["runs"]]
    assert c[:5] == [0, 8307, 11076, 19384, 22153] and c[15] == 85846
    assert c[16:20] == [88615, 96923, 99692, 108000] and s(384, 130000) + s(36, 130000) == 96922
    d = expected()["renders"]["offgrid"]["runs"]
    assert d == [[7500, 1, 1.0], [9000, 1, 1.0], [33000, 12000, 0.5]]
    e = [r[0] for r in expected()["renders"]["song"]["runs"]]
    assert e == [1000, 13000, 89000, 96000, 104000, 116000, 192000, 200000, 212000]
    assert expected()["renders"]["song"]["frames"] == 288000
    for swing, notes, hand in ((667, EXPORT_NOTES, EXPORT_MIDI), (667, SIXTEEN, OVERLAP_MIDI)):
        assert rule_midi(swing, notes) == hand
    deltas = lambda events: [b[0] - a[0] for a, b in zip([(0, None)] + events, events)]
    assert deltas(EXPORT_MIDI) == [0, 24, 8, 12, 52, 24, 56, 16]
    assert deltas(OVERLAP_MIDI) == [0, 24, 8, 16] * 8
    # Ruling 1: a moved step hit ends where the next step starts, so no hit of the key overlaps
    # the next: every note-on comes after (or on the tick of) the note-off before it.
    for hand in (EXPORT_MIDI, OVERLAP_MIDI):
        assert all(a[1][0] == 0x90 and b[1][0] == 0x80 or a[1][0] == 0x80 and b[1][0] == 0x90
                   for a, b in zip(hand, hand[1:]))
    head = pattern_midi(EXPORT_MIDI, 192)
    assert head[:14] == bytes.fromhex("4d546864 00000006 0000 0001 0060")
    assert bytes.fromhex("ff 51 03 07 a1 20 00 ff 58 04 04 02 18 08") in head


def main():
    sanity()
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
        if stale: sys.exit("not what gen_p31_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    answers = expected()
    for name, r in answers["renders"].items():
        print("%-12s %6d frames, %d runs; onsets %s" % (
            name, r["frames"], len(r["runs"]), [run[0] for run in r["runs"]][:12]))
    print("offgrid events (tick, length):", answers["offgrid_events"])
    print("song notes (start, end):", answers["song_notes"])
    for path, content in sorted(MIDI_FILES.items()):
        print("%s: %d bytes\n  %s" % (path, len(content), content.hex()))


if __name__ == "__main__":
    main()
