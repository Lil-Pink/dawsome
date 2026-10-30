"""Known answers for the Library panel: what dropping a starter-kit sound or a plugin instrument
onto the playlist writes into the document, and which channel a linked MIDI device plays.

Expected values come from the rules written out below, never from engine code. Every project
here is authored from the dicts below in the project file's canonical form (fields in order,
defaults left out), so a test compares the saved document byte for byte.

  p10_base.json            one sampler "beat" (impulse.wav) and a one-bar Pattern 1
  p10_after_kit.json       p10_base after the starter kit's first sound (kit.json's first drum,
                           "Kick 808") is dropped on track 2 at bar 3: a new audio channel named
                           after the sound and one clip of the whole file at the project's rate,
                           from the bar's first tick
  p10_after_plugin.json    p10_base after the plugin "Bell Keys" is dropped on track 1 at bar 2:
                           a new plugin channel named after the plugin, at the volume a channel
                           that plays notes is added at (NEW_CHANNEL_VOLUME), and one clip of the
                           pattern the rack shows, a bar long, from the bar's first tick
  p10_link.json            "kick" (impulse.wav, the first channel, so the one selected) and
                           "keys" (ramp4.wav), which plays the notes of LINKED_PORT
  p10_link_on_keys.json    p10_link with one note on "keys" at tick 0 (the render a note from
  p10_link_on_kick.json    LINKED_PORT must equal), and the same note on "kick" (the render a
                           note from any other port must equal)
  p10_link_events.json     note-on C4 at velocity 100 before call 0, from LINKED_PORT, and
  p10_unlinked_events.json the same from another port: a live note sounds from the first frame
                           of the call it comes before, which for call 0 is tick 0's sample
  p10_link_alsa.json       p10_link with the link saved under the port's Linux name, which ends
                           in the ALSA client and port numbers (SAVED_ALSA_PORT)
  p10_alsa_events.json     note-on C4 at velocity 100 before call 0 from the same device numbered
                           anew (RENUMBERED_ALSA_PORT): it is the linked port, so it plays "keys"
                           and renders as p10_link_on_keys.json does
  p10_two_ports_events.json LINKED_PORT and another port both hold C4 (on "keys" and on the
                           selected "kick"), then each lets it go at a call of its own: the notes
                           a take records (--record notes, no count-in, no quantize)
  p10_library.json         the Library screenshot's project: "kick" linked to Port A (listed in
                           the shot), "keys" to Port C (not listed: unplugged)
  p10_expected.json        every figure the tests read

Run:
  python3 -I gen_p10_expected.py              print the known answers
  python3 -I gen_p10_expected.py --write DIR  also write the files above into DIR
"""
import json, os, sys

PPQ = 96; BAR = 4 * PPQ; SR = 44100; MBPM = 120000; BUF = 512
LINKED_PORT = "USB Keyboard MIDI 1"
OTHER_PORT = "Another Port"
# midir's names on Linux: "<client>:<port> <client number>:<port number>"; the numbers are the
# ALSA sequencer's, handed out in the order devices are found.
SAVED_ALSA_PORT = "USB Keyboard:USB Keyboard MIDI 1 32:0"
RENUMBERED_ALSA_PORT = "USB Keyboard:USB Keyboard MIDI 1 36:0"
VELOCITY = 100                                       # the MIDI byte; stored as 100/128

def t2s(t): return t * 60 * SR * 1000 // (MBPM * PPQ)    # the one tick-to-sample rule
def tick_at(s):
    """Playhead tick of absolute sample s: one before the first tick at or after s + 1."""
    return -(-(s + 1) * MBPM * PPQ // (60 * SR * 1000)) - 1
def ticks_holding(start, frames):
    """The fewest ticks (at least 1) from `start` whose samples hold `frames` frames."""
    length = 1
    while t2s(start + length) - t2s(start) < frames: length += 1
    return length

def starter_kit():
    """The starter kit's sounds as assets/sounds/kit.json states them, in its order: its drums
    (the pitched sounds are instrument sets, not the kit's)."""
    here = os.path.dirname(os.path.abspath(__file__))
    with open(os.path.join(here, "..", "..", "assets", "sounds", "kit.json"), encoding="utf-8") as f:
        return [sound for sound in json.load(f)["sounds"] if sound["note"] == "drum"]
def frames_at(frames, rate):
    """A file's frames once resampled to the project's rate on load: ceil(frames * SR / rate)."""
    return -(-frames * SR // rate)

# ---------- the projects (the landed format: canonical field order, defaults left out) ----------
def J(d): return json.dumps(d, separators=(",", ":"), ensure_ascii=False)
def tracks(n=8): return [{"id": i, "name": "Track %d" % i, "mute": False} for i in range(1, n + 1)]
def sampler(i, name, sample, port=None):
    c = {"id": i, "name": name, "kind": "sampler", "sample": sample, "volume": 1.0, "pan": 0.0, "mute": False}
    if port: c["midi_input"] = port
    return c
def note(channel, tick, velocity=1.0):
    return {"channel": channel, "tick": tick, "length": 0, "velocity": velocity}
def project(channels, notes, playlist=None):
    d = {"format_version": 2, "sample_rate": SR, "tempo_mbpm": MBPM, "ppq": PPQ,
         "next_channel_id": max(c["id"] for c in channels) + 1, "channels": channels,
         "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": notes}]}
    if playlist: d["playlist"] = playlist
    return d

BEAT = [note(1, 0), note(1, 2 * PPQ)]
BASE = project([sampler(1, "beat", "impulse.wav")], BEAT)

def after_kit(kit):
    """The kit's first sound on track 2 at bar 3: AddAudioClip with the next channel and clip
    ids; the clip is as long as the file at the project's rate (the kit's 48 kHz files load
    resampled to the project's 44.1 kHz)."""
    track, bar = 2, 3
    start = (bar - 1) * BAR
    sound = kit[0]
    frames = frames_at(sound["frames"], sound["rate"])
    clip = {"id": 1, "track": track, "start": start, "length": ticks_holding(start, frames),
            "kind": "audio", "channel": 2}
    channel = {"id": 2, "name": sound["name"], "kind": "audio",
               "sample": "starter-kit/" + sound["file"], "volume": 1.0, "pan": 0.0, "mute": False}
    doc = project(BASE["channels"] + [channel], BEAT,
                  {"next_track_id": 9, "next_clip_id": 2, "tracks": tracks(), "clips": [clip]})
    return doc, {"sound": sound["name"], "file": sound["file"], "track": track, "bar": bar,
                 "tick": start, "file_frames": sound["frames"], "file_rate": sound["rate"],
                 "frames": frames, "length": clip["length"], "document": "p10_after_kit.json"}

# The volume a channel that plays notes (a sampler, a plugin, a set) is added at: 9/16, -5.0 dB, the
# level the imported format gives a new channel (an audio channel and an input channel are added at
# 1).
NEW_CHANNEL_VOLUME = 0.5625

def after_plugin():
    """Bell Keys on track 1 at bar 2: AddPluginChannel then PlaceClips of pattern 1 (a bar long)
    with the next clip id; the new channel is the one selected."""
    track, bar = 1, 2
    start = (bar - 1) * BAR
    channel = {"id": 2, "name": "Bell Keys", "kind": "plugin",
               "processor": {"id": "clap:org.example.bells", "name": "Bell Keys"},
               "volume": NEW_CHANNEL_VOLUME, "pan": 0.0, "mute": False}
    clip = {"id": 1, "track": track, "start": start, "length": BAR, "kind": "pattern", "pattern": 1}
    doc = project(BASE["channels"] + [channel], BEAT,
                  {"next_track_id": 9, "next_clip_id": 2, "tracks": tracks(), "clips": [clip]})
    return doc, {"plugin": "Bell Keys", "id": "clap:org.example.bells", "track": track, "bar": bar,
                 "tick": start, "selected_channel": 2, "document": "p10_after_plugin.json"}

LINK_CHANNELS = [sampler(1, "kick", "impulse.wav"), sampler(2, "keys", "ramp4.wav", LINKED_PORT)]
FIX = {
    "p10_base": BASE,
    "p10_link": project(LINK_CHANNELS, []),
    "p10_link_on_keys": project(LINK_CHANNELS, [note(2, 0, VELOCITY / 128)]),
    "p10_link_on_kick": project(LINK_CHANNELS, [note(1, 0, VELOCITY / 128)]),
    "p10_library": project([sampler(1, "kick", "impulse.wav", "Port A"),
                            sampler(2, "keys", "ramp4.wav", "Port C")], []),
    "p10_link_alsa": project([sampler(1, "kick", "impulse.wav"),
                              sampler(2, "keys", "ramp4.wav", SAVED_ALSA_PORT)], []),
}

def events(port):
    return {"buffer": BUF, "events": [{"call": 0, "bytes": [0x90, 60, VELOCITY], "port": port}]}

# Two devices holding one key: (call, port, message). Each port's note-off ends that port's own
# note of the key, whichever channel the other port's note of it plays.
TWO_PORTS = [(8, LINKED_PORT, [0x90, 60, VELOCITY]), (8, OTHER_PORT, [0x90, 60, VELOCITY]),
             (40, LINKED_PORT, [0x80, 60, 0]), (70, OTHER_PORT, [0x80, 60, 0])]
CHANNEL_OF = {LINKED_PORT: 2, OTHER_PORT: 1}          # keys is linked; kick, the first, is selected

def two_ports():
    """The take's notes: each heard from the playhead tick of its call's first sample, wrapped to
    the one-bar pattern, as long as the ticks to where its own port's note-off was heard; in the
    pattern's order (tick, channel, key)."""
    held, notes = {}, []
    for call, port, m in TWO_PORTS:
        T = tick_at(call * BUF)
        if m[0] == 0x90 and m[2] > 0: held[(port, m[1])] = T
        else:
            T0 = held.pop((port, m[1]))
            notes.append({"channel": CHANNEL_OF[port], "tick": T0 % BAR, "length": T - T0,
                          "velocity": VELOCITY / 128})
    assert not held
    return sorted(notes, key=lambda n: (n["tick"], n["channel"]))

def two_ports_events():
    return {"buffer": BUF, "events": [{"call": c, "bytes": m, "port": p} for c, p, m in TWO_PORTS]}

def expected(kit):
    _, kit_drop = after_kit(kit)
    _, plugin_drop = after_plugin()
    link = {"port": LINKED_PORT, "other_port": OTHER_PORT, "call": 0, "tick": 0, "sample": t2s(0),
            "frames": t2s(BAR), "velocity": VELOCITY / 128,
            "linked": "p10_link_on_keys.json", "unlinked": "p10_link_on_kick.json",
            "alsa": {"project": "p10_link_alsa.json", "events": "p10_alsa_events.json",
                     "saved": SAVED_ALSA_PORT, "now": RENUMBERED_ALSA_PORT,
                     "renders_as": "p10_link_on_keys.json"}}
    two = {"events": "p10_two_ports_events.json", "bars": 1, "notes": two_ports()}
    return {"kit_drop": kit_drop, "plugin_drop": plugin_drop, "link": link, "two_ports": two}

def write(out, kit):
    docs = dict(FIX)
    docs["p10_after_kit"] = after_kit(kit)[0]
    docs["p10_after_plugin"] = after_plugin()[0]
    for name, doc in docs.items():
        with open(os.path.join(out, name + ".json"), "w") as f: f.write(J(doc) + "\n")
    for name, port in (("p10_link_events", LINKED_PORT), ("p10_unlinked_events", OTHER_PORT),
                       ("p10_alsa_events", RENUMBERED_ALSA_PORT)):
        with open(os.path.join(out, name + ".json"), "w") as f: f.write(J(events(port)) + "\n")
    with open(os.path.join(out, "p10_two_ports_events.json"), "w") as f:
        f.write(J(two_ports_events()) + "\n")
    with open(os.path.join(out, "p10_expected.json"), "w") as f: f.write(J(expected(kit)) + "\n")

def main():
    kit = starter_kit()
    if sys.argv[1:2] == ["--write"] and len(sys.argv) == 3: write(sys.argv[2], kit)
    elif len(sys.argv) > 1: sys.exit(__doc__)
    answers = expected(kit)
    k, p, l, t = answers["kit_drop"], answers["plugin_drop"], answers["link"], answers["two_ports"]
    print("(1a) %s dropped on track %d at bar %d: tick %d; %d frames at %d Hz are %d at %d Hz and need %d ticks from there"
          % (k["sound"], k["track"], k["bar"], k["tick"], k["file_frames"], k["file_rate"], k["frames"], SR, k["length"]))
    print("     (s(%d) = %d, s(%d) = %d, s(%d) = %d)" % (k["tick"], t2s(k["tick"]), k["tick"] + k["length"] - 1,
          t2s(k["tick"] + k["length"] - 1), k["tick"] + k["length"], t2s(k["tick"] + k["length"])))
    print("(1b) %s dropped on track %d at bar %d: plugin channel 2 (%s), pattern 1 clip at tick %d, a bar (%d ticks) long; channel %d selected"
          % (p["plugin"], p["track"], p["bar"], p["id"], p["tick"], BAR, p["selected_channel"]))
    print("(2)  note-on C4 velocity %d from '%s' before call 0 plays 'keys' at sample %d (= %s); from '%s' it plays the selected 'kick' (= %s); a bar is %d frames"
          % (VELOCITY, l["port"], l["sample"], l["linked"], l["other_port"], l["unlinked"], l["frames"]))
    print("     a link saved as '%s' plays the notes of '%s' (= %s)" % (SAVED_ALSA_PORT, RENUMBERED_ALSA_PORT, l["alsa"]["renders_as"]))
    print("(3)  two ports hold C4, released at calls 40 and 70: the take records %s" % J(t["notes"]))

if __name__ == "__main__":
    main()
