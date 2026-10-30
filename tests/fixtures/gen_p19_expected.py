"""Known answers for MIDI out: an input channel with a MIDI out sends its notes (pattern steps,
piano-roll notes) to a MIDI port as note on and note off bytes, each stamped with the output
frame it is sent on, through the player's MIDI-out ring.

Expected values come from the rules written out below, never from engine code:
  time      s(t) = floor(t*60*sr*1000 / (mbpm*ppq)), the one tick-to-sample rule
            (engine/src/time.rs); play starts at output frame 0, tick 0
  pattern   pattern mode loops pattern 1 (length_bars*4*ppq ticks) from tick 0: pass n plays
            its note on tick t at song tick n*length + t
  song      song mode plays the playlist: song tick T in a clip's start..end plays the note on
            the pattern's tick (T - start + offset) mod length (notes at or past the pattern's
            end are silent); the song wraps from its end E (the latest clip end) to tick 0, so
            pass k of the song starts at output frame k*s(E)
  natural   a note's natural frame is the output frame its tick plays on; its end is the
            natural frame of its gate's end: tick + length (a step hit, length 0, one step:
            ppq/4 ticks), cut at its clip's end in song mode, and at least one frame after it
  who       only an input channel with a MIDI out sends notes; a muted one starts none (a
            plugin channel's rule); every other channel sends nothing
  latency   L = floor(latency_ms*sr/1000) frames: a note is sent L frames ahead of its natural
            frame, but never before play started (frame 0); its note off L frames ahead of its
            end, and always at least one frame after its own note on; a note whose note on had
            to wait for frame 0 keeps its full length: its note off at least its gate (end -
            natural) after its note on
  order     every message on a frame in this order: note offs before note ons, each kind in
            start order, the order of the notes' (on frame, natural frame, song order)
  again     a key a channel is still sounding when a note of that key starts there again is
            ended on that frame first (its note off moves there, before the new note on); a
            second note on of a key on the frame its first was sent on is left out
  stop      the render sends Stop at its last frame F: every note still sounding gets its note
            off on frame F; a note whose note on would be sent on F or later is never sent
  bytes     note on 9n kk vv, note off 8n kk 40 (hex), n the MIDI channel - 1, kk the key,
            vv = round(velocity*128) held to 1..127 (half away from zero)

  p19_pattern.json        pattern mode, 2 bars, MIDI channel 1, latency 0: a note held over the
                          bar line, a chord, a step hit, velocities 100/128, 0.5, 1 and 0.25
  p19_song_latency.json   song mode, 2 bars played, MIDI channel 2, latency 32 ms (1411
                          frames): one clip of the pattern, 640 ticks, repeats and is cut at its
                          end, and the song wraps at frame s(640) = 137812, the note after the
                          wrap sent ahead of it
  p19_rack.json           pattern mode, 1 bar, latency 0: two input channels with MIDI outs
                          (channels 1 and 3), a muted one, one with none and a sampler; one note
                          each on tick 0, and the first again on tick 96 while it still sounds
  p19_expected.json       each case: its project, bars, whether it plays the song, its frames,
                          and every message: frame, the rack channel, its bytes; and the echo
                          case below: its project, events and messages

Echo: notes played on MIDI ports (the app's --play-to --midi-events door) going out on the
MIDI out of the channel they play, or not:
  route     a port's note plays the channel the port is linked to (ports compared by their name
            without the ALSA numbers " <client>:<port>"), else the selected channel: the first
  echo      it goes out on that channel's MIDI out, unless echo is off there and the port is
            linked to the channel or is the MIDI out's own port; a channel without a MIDI out
            sends nothing
  guard     a note on of a key the port already holds is left out when it would go out (the
            instrument sending back what it was sent); a note on at velocity 0 is a note off
  off       a note off goes out when its note on went out, on the channel it went out on
  frame     a message comes before call k of 512 frames and plays on its first frame, k*512
  bytes     as above: 9n kk vv with vv the message's own velocity byte, 8n kk 40

  p19_echo.json           three input channels with MIDI outs: "Synth" (the selected one, linked
                          to no port, echo off), "Linked" (linked to its port, echo off) and "Echo"
                          (linked to its port, echo on)
  p19_echo_events.json    a keyboard and the Synth's own port playing the Synth, the linked
                          ports playing their channels, and the Echo's port sending back a note

Renders are 128 BPM at 44.1 kHz, ppq 96: s(t) = floor(t*215.33203125), a bar 82687.5 frames.

Run:
  python3 -I gen_p19_expected.py              print every case's messages as the plan's tables
  python3 -I gen_p19_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p19_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import json, os, sys
from collections import namedtuple

SR = 44100; PPQ = 96; MBPM = 128000
BAR = 4 * PPQ
STEP = PPQ // 4
PORT = "Synth:Synth MIDI 1"
DEVICE = "USB Audio Line Input"


def J(d): return json.dumps(d, separators=(",", ":"))
def s(t): return t * 60 * SR * 1000 // (MBPM * PPQ)
def latency_frames(ms): return ms * SR // 1000


def velocity_byte(v):
    """round(v*128) half away from zero, held to 1..127; v is a non-negative binary fraction."""
    x = v * 128
    whole = int(x)
    return max(1, min(127, whole + (1 if x - whole >= 0.5 else 0)))


# --- the projects (the landed format: fields in order, defaults left out) ---------------------

def midi_out(channel, latency_ms=0, port=PORT, echo=False):
    m = {"port": port, "channel": channel, "latency_ms": latency_ms}
    if echo: m["echo"] = True
    return m


def input_channel(i, name, out=None, mute=False, link=None):
    c = {"id": i, "name": name, "kind": "input",
         "input": {"device": DEVICE, "channels": {"stereo": [0, 1]}},
         "volume": 1.0, "pan": 0.0, "mute": mute}
    if link is not None: c["midi_input"] = link
    if out is not None: c["midi_out"] = out
    return c


def sampler_channel(i, name):
    return {"id": i, "name": name, "kind": "sampler", "sample": "impulse.wav",
            "volume": 1.0, "pan": 0.0, "mute": False}


def note(channel, tick, length, velocity, key):
    n = {"channel": channel, "tick": tick, "length": length, "velocity": velocity}
    if key != 60: n["key"] = key
    return n


def project(channels, notes, clips=None):
    notes = sorted(notes, key=lambda n: (n["tick"], n["channel"], n.get("key", 60)))
    d = {"format_version": 7, "sample_rate": SR, "tempo_mbpm": MBPM, "ppq": PPQ,
         "next_channel_id": max(c["id"] for c in channels) + 1, "channels": channels,
         "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": notes}]}
    if clips is not None:
        tracks = [{"id": i, "name": "Track %d" % i, "mute": False} for i in range(1, 9)]
        d["playlist"] = {"next_track_id": 9, "next_clip_id": max(c["id"] for c in clips) + 1,
                         "tracks": tracks, "clips": clips}
    return d


def pclip(i, track, start, length, pattern):
    return {"id": i, "track": track, "start": start, "length": length, "kind": "pattern",
            "pattern": pattern}


PROJECTS = {
    "p19_pattern.json": project(
        [input_channel(1, "Synth", midi_out(1))],
        [note(1, 0, 384, 100 / 128, 60), note(1, 96, 48, 0.5, 64), note(1, 96, 48, 1.0, 67),
         note(1, 288, 0, 0.25, 72)]),
    "p19_song_latency.json": project(
        [input_channel(1, "Synth", midi_out(2, 32))],
        [note(1, 0, 96, 0.75, 50), note(1, 192, 288, 0.5, 55), note(1, 336, 96, 0.5, 57)],
        [pclip(1, 1, 0, 640, 1)]),
    "p19_rack.json": project(
        [input_channel(1, "Synth A", midi_out(1)), input_channel(2, "Synth B", midi_out(3)),
         input_channel(3, "Synth muted", midi_out(4), mute=True), input_channel(4, "Line in"),
         sampler_channel(5, "hit")],
        [note(1, 0, 192, 0.5, 48), note(2, 0, 768, 1.0, 36), note(3, 0, 96, 1.0, 40),
         note(4, 0, 96, 1.0, 41), note(5, 0, 0, 1.0, 60), note(1, 96, 48, 0.75, 48)]),
}

LINKED_PORT = "Linked:Linked MIDI 1"
ECHO_PORT = "Echo:Echo MIDI 1"
ECHO_PROJECT = "p19_echo.json"
ECHO_EVENTS = "p19_echo_events.json"
PROJECTS[ECHO_PROJECT] = project(
    [input_channel(1, "Synth", midi_out(1)),
     input_channel(2, "Linked", midi_out(2, port=LINKED_PORT), link=LINKED_PORT),
     input_channel(3, "Echo", midi_out(3, port=ECHO_PORT, echo=True), link=ECHO_PORT)],
    [])

# The ports as ALSA lists them, numbers and all.
KEYS_IN = "Keys:Keys MIDI 1 20:0"
SYNTH_IN = PORT + " 24:0"
LINKED_IN = LINKED_PORT + " 28:0"
ECHO_IN = ECHO_PORT + " 32:0"
BUFFER = 512

# (call, port, bytes)
ECHO_MESSAGES = [
    (1, KEYS_IN, [0x90, 60, 100]),    # a keyboard linked to nothing plays the Synth: out
    (1, SYNTH_IN, [0x90, 62, 90]),    # the Synth's own port, linked to nothing: quiet
    (2, LINKED_IN, [0x90, 64, 80]),   # linked, echo off: quiet
    (2, ECHO_IN, [0x90, 67, 70]),     # linked, echo on: out
    (3, ECHO_IN, [0x90, 67, 70]),     # sent back while the port holds it: left out
    (4, KEYS_IN, [0x80, 60, 0]),
    (4, SYNTH_IN, [0x80, 62, 0]),
    (5, LINKED_IN, [0x90, 64, 0]),    # velocity 0: a note off
    (5, ECHO_IN, [0x80, 67, 64]),
    (6, ECHO_IN, [0x90, 67, 70]),     # held no more: a new note, out
    (7, ECHO_IN, [0x80, 67, 0]),
]

# case name: (project, bars, plays the song)
CASES = {
    "pattern": ("p19_pattern.json", 2, False),
    "song_latency": ("p19_song_latency.json", 2, True),
    "rack": ("p19_rack.json", 1, False),
}


# --- the rules -------------------------------------------------------------------------------

# One note the song plays: its natural frame and end, where it comes in song order, and who
# sends it how.
Played = namedtuple("Played", "natural end order channel status key velocity latency")


def senders(doc):
    """Each channel that sends notes, by ID: (MIDI channel - 1, latency in frames)."""
    out = {}
    for c in doc["channels"]:
        if c["kind"] == "input" and "midi_out" in c and not c["mute"]:
            m = c["midi_out"]
            out[c["id"]] = (m["channel"] - 1, latency_frames(m["latency_ms"]))
    return out


def gate_end(tick, length, clip_end=None):
    end = tick + (length if length > 0 else STEP)
    return end if clip_end is None else min(end, clip_end)


def played(doc, song, horizon):
    """Every note the song plays with a natural frame before `horizon`, in song order: by pass,
    tick, clip (by start, then ID) and the note's place in its pattern."""
    pattern = doc["patterns"][0]
    length = pattern["length_bars"] * BAR
    notes = [n for n in pattern["notes"] if n["tick"] < length]
    sends = senders(doc)
    out = []

    def add(base, tick, end, rank, n):
        if n["channel"] not in sends: return
        status, latency = sends[n["channel"]]
        natural = base + s(tick)
        out.append(Played(natural, max(base + s(end), natural + 1), (base, tick) + rank,
                          n["channel"], status, n.get("key", 60), n["velocity"], latency))

    if not song:
        passes = 0
        while s(passes * length) < horizon:
            for index, n in enumerate(notes):
                tick = passes * length + n["tick"]
                if s(tick) < horizon:
                    add(0, tick, gate_end(tick, n["length"]), (0, index), n)
            passes += 1
    else:
        clips = sorted(doc["playlist"]["clips"], key=lambda c: (c["start"], c["id"]))
        song_end = max(c["start"] + c["length"] for c in clips)
        k = 0
        while k * s(song_end) < horizon:
            base = k * s(song_end)
            for order, clip in enumerate(clips):
                start, end = clip["start"], clip["start"] + clip["length"]
                offset = clip.get("offset", 0)
                for tick in range(start, end):
                    data = (tick - start + offset) % length
                    for index, n in enumerate(notes):
                        if n["tick"] == data and base + s(tick) < horizon:
                            add(base, tick, gate_end(tick, n["length"], end), (order, index), n)
            k += 1
    return sorted(out, key=lambda p: p.order)


def messages(doc, song, frames):
    """Every message the render of `frames` frames sends, with Stop at its end, in order:
    (frame, rack channel, bytes)."""
    horizon = frames + max([latency for _, latency in senders(doc).values()] + [0])
    notes = []   # [on frame, off frame, natural, order, played]
    for p in played(doc, song, horizon):
        on = max(p.natural - p.latency, 0)
        if on >= frames: continue
        delayed = p.natural - p.latency < 0
        shortest = p.end - p.natural if delayed else 1
        notes.append([on, max(p.end - p.latency, on + shortest), p.natural, p.order, p])
    notes.sort(key=lambda n: (n[0], n[2], n[3]))
    kept, sounding = [], {}
    for n in notes:
        key = (n[4].channel, n[4].key)
        before = sounding.get(key)
        if before is not None and before[1] >= n[0]:
            if before[0] == n[0]: continue          # a second note on, on one frame: left out
            before[1] = n[0]                        # ended where the key starts again
        sounding[key] = n
        kept.append(n)
    events = []
    for rank, (on, off, _, _, p) in enumerate(kept):
        events.append((on, 1, rank, p.channel, [0x90 | p.status, p.key, velocity_byte(p.velocity)]))
        events.append((min(off, frames), 0, rank, p.channel, [0x80 | p.status, p.key, 0x40]))
    return [(frame, channel, data) for frame, _, _, channel, data in sorted(events)]


def hex_bytes(data): return " ".join("%02X" % b for b in data)


def stable(port):
    """The port's name without the ALSA numbers " <client>:<port>" at its end."""
    head, _, last = port.rpartition(" ")
    client, colon, number = last.partition(":")
    return head if head and colon and client.isdigit() and number.isdigit() else port


def echo_messages(doc, messages):
    """What the ports' notes send out: (frame, rack channel, bytes), in order."""
    channels = {c["id"]: c for c in doc["channels"]}
    links = {stable(c["midi_input"]): c["id"] for c in doc["channels"] if "midi_input" in c}
    selected = doc["channels"][0]["id"]
    held, sounding, out = {}, {}, []
    for call, port, (status, key, velocity) in messages:
        linked = links.get(stable(port))
        channel = selected if linked is None else linked
        m = channels[channel].get("midi_out")
        echoes = m is not None and (m.get("echo", False) or
                                    (linked != channel and stable(m["port"]) != stable(port)))
        keys = held.setdefault(port, set())
        if status & 0xF0 == 0x90 and velocity > 0:
            if echoes and key in keys: continue
            keys.add(key)
            if echoes:
                sounding[(port, key)] = channel
                out.append((call * BUFFER, channel,
                            [0x90 | (m["channel"] - 1), key, velocity_byte(velocity / 128)]))
        else:
            keys.discard(key)
            sent = sounding.pop((port, key), None)
            if sent is not None:
                status_channel = channels[sent]["midi_out"]["channel"] - 1
                out.append((call * BUFFER, sent, [0x80 | status_channel, key, 0x40]))
    return out


def echo_case():
    messages = [{"frame": f, "channel": c, "bytes": hex_bytes(d)}
                for f, c, d in echo_messages(PROJECTS[ECHO_PROJECT], ECHO_MESSAGES)]
    return {"fixture": ECHO_PROJECT, "events": ECHO_EVENTS, "bars": 1, "messages": messages}


def echo_events():
    return {"buffer": BUFFER,
            "events": [{"call": call, "bytes": data, "port": port}
                       for call, port, data in ECHO_MESSAGES]}


def expected():
    cases = {}
    for name, (fixture, bars, song) in CASES.items():
        frames = s(bars * BAR)
        events = [{"frame": f, "channel": c, "bytes": hex_bytes(d)}
                  for f, c, d in messages(PROJECTS[fixture], song, frames)]
        cases[name] = {"fixture": fixture, "bars": bars, "song": song, "frames": frames,
                       "events": events}
    return {"rate": SR, "port": PORT, "cases": cases, "echo": echo_case()}


def files():
    out = {name: (J(doc) + "\n").encode() for name, doc in PROJECTS.items()}
    out[ECHO_EVENTS] = (J(echo_events()) + "\n").encode()
    out["p19_expected.json"] = (J(expected()) + "\n").encode()
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
        if stale: sys.exit("not what gen_p19_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    for name, case in expected()["cases"].items():
        print("%s: %s, %d bar(s), %d frames" % (name, case["fixture"], case["bars"],
                                                case["frames"]))
        rows = {}
        for e in case["events"]:
            rows.setdefault(e["frame"], []).append("%s (ch %d)" % (e["bytes"], e["channel"]))
        for frame, row in rows.items():
            print("  %7d | %s" % (frame, " · ".join(row)))
    print("echo: %s, %s" % (ECHO_PROJECT, ECHO_EVENTS))
    for e in echo_case()["messages"]:
        print("  %7d | %s (ch %d)" % (e["frame"], e["bytes"], e["channel"]))


if __name__ == "__main__":
    main()
