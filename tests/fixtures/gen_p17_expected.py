"""Known answers for Recording v2: one Record button with a visible target (Notes or Audio), notes
that are written into the pattern the moment they end, song takes that grow by whole bars, audio
takes named after the project, and Capture.

Expected values come from the rules written out below, never from engine code:
  time      tick -> sample s(t) = floor(t*60*sr*1000 / (mbpm*ppq)); the playhead's tick at sample
            s is the last tick that starts at or before it; a bar is 384 ticks (4/4, PPQ 96);
            the player is called with 512 frames, and a MIDI message timed for call k sounds
            from that call's first frame
  pattern   a take in pattern mode writes each note into the looping pattern the moment its
            end is heard: start = its tick modulo the pattern, moved to the nearest step (24
            ticks, half up, an end snapped onto the pattern's end wrapping to 0), length as
            played; a note still held when the take stops ends at the stop's tick; nothing is
            written again at the stop, so the notes heard mid-take are there unchanged
  wrap      (found in use) a pattern take's note that runs past the pattern's end is split there:
            one note from its start to the end, one from tick 0 for what is left, no longer than
            its own start, so a key held a whole pattern or more covers the pattern once; a note
            within the pattern stays one
  song      a take in song mode makes pattern "<selected channel> take N" (the first N no
            pattern is named with) and one clip of it from the song marker, on the armed track,
            else the first track with no clip over the take's first bar, else a new track; while
            it plays the pattern and its clip are as many whole bars as hold the bar the playhead
            is in (floor((T - start)/384) + 1); at the stop, as many as hold start..stop
            (ceil((stop - start)/384), at least 1, and at least the bar of the last note's
            start); notes at T - start, quantized to the step as above (no wrap); the song's end
            moves with the clip; a take with no note leaves the document as it was
  audio     a take of what is heard goes to the project's recordings folder (samples/ beside it)
            as "<project> take N.wav", N the first number with no such file; in song mode it runs
            from the song marker until the stop, in pattern mode for one pass of the pattern
            (s(bars*384) frames) and stops by itself; it becomes audio channel "Take N" and one
            clip from the song marker, L ticks long with L the fewest ticks (at least 1) whose
            samples hold the file, L = min{L >= 1 : s(start+L) - s(start) >= frames}, on the
            armed track, else the first track free over the clip, else a new track
  capture   the notes played while no take runs, within 30 s of the newest one's start, laid out
            at the current tempo from the output clock's frames: the first note's start is tick
            0 and a note at frame f starts at tick_at(f - f0) and ends at tick_at(end - f0);
            quantized to the step as above; in pattern mode they go into the selected pattern
            from its first bar's start, the pattern growing by whole bars to hold every note's
            end; in song mode a new pattern and clip as a song take's, from the bar start
            nearest the song marker (half up)
  undo      every take and every Capture is one undo step, which gives the document back byte
            for byte; its redo gives the document after it byte for byte

  p17_pattern.json          44.1 kHz, 120 BPM: channel 1 "Kick" (a sampler of impulse.wav),
                            pattern 1 of one bar, no notes
  p17_pattern_events.json   a take of two passes over the pattern (`--midi-events`), one note
                            held to the end
  p17_pattern_after.json    the document after it (`--quantize on`, no count-in, 2 bars)
  p17_wrap_events.json      two keys from tick 300 of the one-bar pattern: one let go of in the
                            next pass, one held a pass and a half (3 bars, either quantize)
  p17_song.json             44.1 kHz, 120 BPM: channel 1 "Kick", pattern 1 (a kick on the
                            first step) and its clip on track 1 over bars 1-4
  p17_song_events.json      three notes over 5.5 bars from tick 384, and one held over the stop
  p17_song_after.json       the document after it
  p17_song_past_end_events.json  one note in a take from tick 3072, past the song's end
  p17_song_past_end_after.json   the document after it
  p17_audio48.json          48 kHz, 120 BPM: channel 1 "Kick", pattern 1 and its clip on track 1
                            over bars 1-4
  p17_audio_song_after.json      two song-mode takes of the master from tick 192, 1000 frames each
  p17_audio_pattern_after.json   one pattern-mode take of the master from tick 0
  p17_capture.json          44.1 kHz, 120 BPM: channel 1 "Keys", pattern 1 of one bar
  p17_capture_events.json   notes played while stopped: one 31 s before the phrase, the phrase
  p17_capture_after.json    the document after Capture in pattern mode
  p17_capture_song_after.json    the document after Capture in song mode, the marker at 400
  p17_expected.json         every case: its inputs and the values the tests compare

Run:
  python3 -I gen_p17_expected.py              print the known answers
  python3 -I gen_p17_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p17_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import copy, json, os, sys

PPQ = 96; BAR = 4 * PPQ; STEP = PPQ // 4; BUF = 512; MBPM = 120000
CAPTURE_SECONDS = 30


def J(d): return json.dumps(d, separators=(",", ":"), ensure_ascii=False)


def t2s(t, sr): return t * 60 * sr * 1000 // (MBPM * PPQ)                 # the one tick-to-sample rule
def first_tick_at_or_after(s, sr): return -(-s * MBPM * PPQ // (60 * sr * 1000))
def tick_at(s, sr): return first_tick_at_or_after(s + 1, sr) - 1         # the playhead's tick
def snap_near(t, u): return (t + u // 2) // u * u                       # half up
def bars_holding(span): return max(1, -(-span // BAR))                  # whole bars, at least 1
def ticks_holding(start, frames, sr):
    return max(1, first_tick_at_or_after(t2s(start, sr) + frames, sr) - start)
def vel(b): return b / 128                                              # MIDI byte -> stored velocity


def note(channel, tick, length, velocity, key):
    n = {"channel": channel, "tick": tick, "length": length, "velocity": velocity}
    if key != 60: n["key"] = key
    return n


def sorted_notes(ns): return sorted(ns, key=lambda n: (n["tick"], n["channel"], n.get("key", 60)))


def channel(cid, name, kind="sampler", sample="impulse.wav"):
    return {"id": cid, "name": name, "kind": kind, "sample": sample, "volume": 1.0, "pan": 0.0,
            "mute": False}


def tracks(n=8): return [{"id": i, "name": "Track %d" % i, "mute": False} for i in range(1, n + 1)]


def project(sr, channels, patterns, playlist=None):
    doc = {"format_version": 2, "sample_rate": sr, "tempo_mbpm": MBPM, "ppq": PPQ,
           "next_channel_id": max(c["id"] for c in channels) + 1, "channels": channels,
           "patterns": patterns}
    if playlist is not None: doc["playlist"] = playlist
    return doc


def events(script): return {"buffer": BUF, "events": [{"call": c, "bytes": b} for c, b in script]}


# ---------- (1) a take in pattern mode, through --play-to (and the window) ----------
P_SR = 44100
P_BARS = 2                                   # the render: two passes over the one-bar pattern
P_DUMP = 210                                 # notes dumped once 210 calls have played
P_EVENTS = [(20, [0x90, 60, 100]), (40, [0x80, 60, 0]),
            (60, [0x90, 62, 64]), (70, [0x90, 62, 0]),          # a note-on of velocity 0 ends it
            (150, [0x90, 64, 127]), (200, [0x80, 64, 64]),      # across the loop's end
            (250, [0x90, 65, 80]), (260, [0x80, 65, 0]),        # the second pass: overdub
            (300, [0x90, 67, 90])]                              # held to the end


def pattern_base():
    return project(P_SR, [channel(1, "Kick")],
                   [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": []}])


def split(start, length, pattern):
    """A pattern take's note from `start` (within the pattern), `length` as played, as it lands
    in the looping pattern: cut at the pattern's end, the rest from tick 0, no longer than its
    own start (so held a whole pattern or more it covers the pattern once)."""
    first = min(length, pattern - start)
    notes = [(start, first)]
    rest = min(length - first, start)
    if rest > 0: notes.append((0, rest))
    return notes


WRAP_BARS = 3
WRAP_EVENTS = [(135, [0x90, 60, 100]), (135, [0x90, 62, 64]),     # both at tick 300
               (218, [0x80, 60, 0]),                             # tick 485: 101 into the next pass
               (393, [0x80, 62, 0])]                             # tick 876: a pass and a half


def pattern_wrap(quantize):
    on, notes = {}, []
    for call, m in WRAP_EVENTS:
        T = tick_at(call * BUF, P_SR)
        if m[0] == 0x90 and m[2] > 0: on[m[1]] = (T, m[2])
        else:
            T0, v = on.pop(m[1])
            start = T0 % BAR
            if quantize: start = snap_near(start, STEP) % BAR
            for tick, length in split(start, T - T0, BAR):
                notes.append(note(1, tick, length, vel(v), m[1]))
    return sorted_notes(notes)


def pattern_take():
    """Each note as its end is heard (the call after its end's call has played), and the held
    one at the render's end: (dump notes, final notes)."""
    frames = t2s(P_BARS * BAR, P_SR)
    length = BAR
    on, written = {}, []                          # written: (call its end was heard after, note)
    for call, m in P_EVENTS:
        T = tick_at(call * BUF, P_SR)
        if m[0] == 0x90 and m[2] > 0:
            on[m[1]] = (T, m[2])
        else:
            T0, v = on.pop(m[1])
            start = snap_near(T0 % length, STEP) % length
            for tick, played in split(start, T - T0, length):
                written.append((call, note(1, tick, played, vel(v), m[1])))
    stop = tick_at(frames, P_SR)                  # the playhead after the last frame
    for key, (T0, v) in on.items():
        start = snap_near(T0 % length, STEP) % length
        for tick, played in split(start, stop - T0, length):
            written.append((None, note(1, tick, played, vel(v), key)))
    dump = sorted_notes([n for call, n in written if call is not None and call < P_DUMP])
    final = sorted_notes([n for _, n in written])
    return frames, stop, dump, final


# ---------- (2) and (5) song takes, through the window ----------
S_SR = 44100
S_MARKER = 384
S_CALLS = 948                                # 5.5 bars and a little: the stop
S_EVENTS = [(50, [0x90, 60, 100]), (120, [0x80, 60, 0]),
            (400, [0x90, 64, 64]), (450, [0x80, 64, 0]),
            (800, [0x90, 67, 127]), (900, [0x80, 67, 0]),
            (940, [0x90, 69, 100])]                             # held over the stop
S_CHECKPOINTS = [0, 171, 172, 400, 600, 947]  # after these calls: the take's bars so far
PAST_MARKER = 3072                           # bar 9: past the song's end (bar 4's end, 1536)
PAST_CALLS = 300
PAST_EVENTS = [(100, [0x90, 62, 100]), (150, [0x80, 62, 0])]
EMPTY_CALLS = 400


def song_playlist(sr_clips):
    return {"next_track_id": 9, "next_clip_id": len(sr_clips) + 1, "tracks": tracks(),
            "clips": sr_clips}


def song_base(sr=S_SR):
    clip = {"id": 1, "track": 1, "start": 0, "length": 4 * BAR, "kind": "pattern", "pattern": 1}
    return project(sr, [channel(1, "Kick")],
                   [{"id": 1, "name": "Pattern 1", "length_bars": 1,
                     "notes": [note(1, 0, 0, 1.0, 60)]}],
                   song_playlist([clip]))


def free_track(doc, start, end):
    for t in doc["playlist"]["tracks"]:
        if not any(c["track"] == t["id"] and c["start"] < end and c["start"] + c["length"] > start
                   for c in doc["playlist"]["clips"]):
            return t["id"]
    return None


def song_take(marker, script, calls, sr=S_SR):
    """The document after a take in song mode from `marker`, `calls` calls long, and the bars
    the take holds after each checkpoint call."""
    base = song_base(sr)
    start_s = t2s(marker, sr)
    position = lambda calls_done: tick_at(start_s + calls_done * BUF, sr)
    on, notes = {}, []
    for call, m in script:
        T = position(call)
        if m[0] == 0x90 and m[2] > 0:
            on[m[1]] = (T, m[2])
        else:
            T0, v = on.pop(m[1])
            notes.append(note(1, snap_near(T0 - marker, STEP), T - T0, vel(v), m[1]))
    stop = position(calls)
    for key, (T0, v) in on.items():
        notes.append(note(1, snap_near(T0 - marker, STEP), stop - T0, vel(v), key))
    if not notes:
        return base, None, stop
    bars = bars_holding(stop - marker)
    bars = max(bars, max(n["tick"] for n in notes) // BAR + 1)
    doc = copy.deepcopy(base)
    track = free_track(doc, marker, marker + BAR)
    doc["patterns"].append({"id": 2, "name": "Kick take 1", "length_bars": bars,
                            "notes": sorted_notes(notes)})
    pl = doc["playlist"]
    pl["clips"].append({"id": 2, "track": track, "start": marker, "length": bars * BAR,
                        "kind": "pattern", "pattern": 2})
    pl["next_clip_id"] = 3
    checkpoints = [[c, (position(c + 1) - marker) // BAR + 1] for c in S_CHECKPOINTS if c < calls]
    return doc, {"bars": bars, "track": track, "stop": stop, "checkpoints": checkpoints,
                 "song_end": max(c["start"] + c["length"] for c in pl["clips"])}, stop


# ---------- (3) audio takes, through the window ----------
A_SR = 48000
A_SONG_MARKER = 192
A_SONG_FRAMES = 1000
A_TAKES = 2


def audio_take_doc(doc, number, start, frames, sr, project_name):
    doc = copy.deepcopy(doc)
    cid = doc["next_channel_id"]
    length = ticks_holding(start, frames, sr)
    track = free_track(doc, start, start + length)
    sample = "samples/%s take %d.wav" % (project_name, number)
    doc["channels"].append(channel(cid, "Take %d" % number, "audio", sample))
    doc["next_channel_id"] = cid + 1
    pl = doc["playlist"]
    clip_id = pl["next_clip_id"]
    pl["clips"].append({"id": clip_id, "track": track, "start": start, "length": length,
                        "kind": "audio", "channel": cid})
    pl["next_clip_id"] = clip_id + 1
    return doc, {"file": sample, "frames": frames, "channel": cid, "clip": clip_id,
                 "track": track, "start": start, "length": length}


def audio_song():
    doc = song_base(A_SR)
    takes = []
    for number in range(1, A_TAKES + 1):
        doc, take = audio_take_doc(doc, number, A_SONG_MARKER, A_SONG_FRAMES, A_SR, "p17_audio48")
        takes.append(take)
    return doc, takes


def audio_pattern():
    frames = t2s(1 * BAR, A_SR)                  # one pass of the one-bar pattern
    return audio_take_doc(song_base(A_SR), 1, 0, frames, A_SR, "p17_audio48")


# ---------- (4) Capture, through the window ----------
C_SR = 44100
C_OLD = (10, 30)                             # a note played and let go of long before the phrase
C_PHRASE = [(3000, [0x90, 60, 100]), (3040, [0x80, 60, 0]),
            (3100, [0x90, 64, 64]), (3150, [0x80, 64, 0]),
            (3300, [0x90, 67, 127]), (3320, [0x80, 67, 0])]
C_CALLS = 3330                               # stopped all the while; Capture after this call
C_SONG_MARKER = 400


def capture_events():
    script = [(C_OLD[0], [0x90, 72, 100]), (C_OLD[1], [0x80, 72, 0])] + C_PHRASE
    return script


def capture_base():
    return project(C_SR, [channel(1, "Keys")],
                   [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": []}])


def captured():
    """The notes Capture lays out: those starting within 30 s of the newest start, from the
    first one's frame."""
    on, played = {}, []
    for call, m in capture_events():
        frame = call * BUF
        if m[0] == 0x90 and m[2] > 0: on[m[1]] = (frame, m[2])
        else:
            f0, v = on.pop(m[1]); played.append((f0, frame, v, m[1]))
    newest = max(f for f, _, _, _ in played)
    window = CAPTURE_SECONDS * C_SR
    kept = [p for p in played if p[0] >= newest - window]
    first = min(f for f, _, _, _ in kept)
    notes = []
    for f0, f1, v, key in kept:
        start = tick_at(f0 - first, C_SR)
        notes.append(note(1, snap_near(start, STEP), tick_at(f1 - first, C_SR) - start, vel(v), key))
    left_out = len(played) - len(kept)
    return sorted_notes(notes), left_out


def capture_docs():
    notes, left_out = captured()
    reach = max(n["tick"] + n["length"] for n in notes)
    bars = max(1, bars_holding(reach))
    pattern = copy.deepcopy(capture_base())
    pattern["patterns"][0]["length_bars"] = max(1, bars)
    pattern["patterns"][0]["notes"] = notes
    song = copy.deepcopy(capture_base())
    start = (C_SONG_MARKER + BAR // 2) // BAR * BAR
    song["patterns"].append({"id": 2, "name": "Keys take 1", "length_bars": bars, "notes": notes})
    pl = {"next_track_id": 9, "next_clip_id": 2, "tracks": tracks(),
          "clips": [{"id": 1, "track": 1, "start": start, "length": bars * BAR,
                     "kind": "pattern", "pattern": 2}]}
    song["playlist"] = pl
    return pattern, song, {"bars": bars, "notes": notes, "left_out": left_out,
                           "song_start": start}


def files():
    written = {}
    frames, stop, dump, final = pattern_take()
    base = pattern_base()
    after = copy.deepcopy(base); after["patterns"][0]["notes"] = final
    written["p17_pattern.json"] = J(base)
    written["p17_pattern_events.json"] = J(events(P_EVENTS))
    written["p17_pattern_after.json"] = J(after)
    written["p17_wrap_events.json"] = J(events(WRAP_EVENTS))
    song_doc, song, _ = song_take(S_MARKER, S_EVENTS, S_CALLS)
    past_doc, past, _ = song_take(PAST_MARKER, PAST_EVENTS, PAST_CALLS)
    empty_doc, empty, _ = song_take(S_MARKER, [], EMPTY_CALLS)
    assert empty is None and empty_doc == song_base()
    written["p17_song.json"] = J(song_base())
    written["p17_song_events.json"] = J(events(S_EVENTS))
    written["p17_song_after.json"] = J(song_doc)
    written["p17_song_past_end_events.json"] = J(events(PAST_EVENTS))
    written["p17_song_past_end_after.json"] = J(past_doc)
    a_song_doc, a_song = audio_song()
    a_pat_doc, a_pat = audio_pattern()
    written["p17_audio48.json"] = J(song_base(A_SR))
    written["p17_audio_song_after.json"] = J(a_song_doc)
    written["p17_audio_pattern_after.json"] = J(a_pat_doc)
    c_pattern, c_song, capture = capture_docs()
    written["p17_capture.json"] = J(capture_base())
    written["p17_capture_events.json"] = J(events(capture_events()))
    written["p17_capture_after.json"] = J(c_pattern)
    written["p17_capture_song_after.json"] = J(c_song)
    expected = {
        "pattern": {"project": "p17_pattern.json", "events": "p17_pattern_events.json",
                    "bars": P_BARS, "frames": frames, "stop": stop, "dump_call": P_DUMP,
                    "dump_notes": dump, "notes": final, "document": "p17_pattern_after.json"},
        "pattern_wrap": {"project": "p17_pattern.json", "events": "p17_wrap_events.json",
                         "bars": WRAP_BARS, "notes_off": pattern_wrap(False),
                         "notes_on": pattern_wrap(True)},
        "song": {"project": "p17_song.json", "events": "p17_song_events.json",
                 "marker": S_MARKER, "calls": S_CALLS, "pattern": 2, "name": "Kick take 1",
                 "clip": 2, "document": "p17_song_after.json", **song},
        "song_past_end": {"project": "p17_song.json", "events": "p17_song_past_end_events.json",
                          "marker": PAST_MARKER, "calls": PAST_CALLS, "pattern": 2, "clip": 2,
                          "document": "p17_song_past_end_after.json", **past},
        "song_empty": {"project": "p17_song.json", "marker": S_MARKER, "calls": EMPTY_CALLS},
        "audio_song": {"project": "p17_audio48.json", "marker": A_SONG_MARKER,
                       "frames": A_SONG_FRAMES, "takes": a_song,
                       "document": "p17_audio_song_after.json"},
        "audio_pattern": {"project": "p17_audio48.json", "marker": 0, "take": a_pat,
                          "document": "p17_audio_pattern_after.json"},
        "capture": {"project": "p17_capture.json", "events": "p17_capture_events.json",
                    "calls": C_CALLS, "song_marker": C_SONG_MARKER,
                    "document": "p17_capture_after.json",
                    "song_document": "p17_capture_song_after.json", **capture},
    }
    written["p17_expected.json"] = J(expected)
    return {name: (text + "\n").encode("utf-8") for name, text in written.items()}


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
        if stale: sys.exit("not what gen_p17_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    expected = json.loads(files()["p17_expected.json"])
    p = expected["pattern"]
    print("pattern take: dump after call %d %s" % (p["dump_call"], J(p["dump_notes"])))
    print("  at the stop (tick %d) %s" % (p["stop"], J(p["notes"])))
    w = expected["pattern_wrap"]
    print("pattern take across the loop's end: quantize off %s" % J(w["notes_off"]))
    print("  quantize on %s" % J(w["notes_on"]))
    for case in ("song", "song_past_end"):
        s = expected[case]
        print("%s: from %d, %d bars on track %d, stop %d, song end %d, checkpoints %s" % (
            case, s["marker"], s["bars"], s["track"], s["stop"], s["song_end"],
            s["checkpoints"]))
    for take in expected["audio_song"]["takes"]:
        print("audio (song): %s" % J(take))
    print("audio (pattern): %s" % J(expected["audio_pattern"]["take"]))
    c = expected["capture"]
    print("capture: %d bars, %d left out, song start %d, %s" % (
        c["bars"], c["left_out"], c["song_start"], J(c["notes"])))


if __name__ == "__main__":
    main()
