"""Known answers for a Library sound, set or plugin dropped on an existing channel's header (its
light, name and knobs, left of its steps): the channel's sound is replaced.

Expected values come from the rules written out below, never from engine code:
  kept      the channel's id, place in the rack, notes, routing (insert), mute, volume, pan and
            MIDI input stay as they were
  sound     the kind, the file, the set and the plugin (with its state) become the dropped
            sound's: a sound file plays as a sampler (a starter-kit sound as its file in the
            kit's folder), a set as an instrument channel, a plugin instrument as a plugin
            channel with the plugin's default state (none stored)
  settings  the sound settings stay where the new kind takes them: all on a sampler, all but
            reverse and the start on a set, none on a plugin (the defaults, written as nothing);
            the gate stays as it was stored
  name      the dropped sound's name, unless the user renamed the channel: a channel is
            unrenamed while its name is what its sound names it when it is added (a sampler
            its file's name without its extension, "Sampler" without a file; a set the name
            its set.json gives; a plugin its name)
  version   format_version 5 while any channel holds sound settings, else 4 while one plays a
            set, else 2
  ties      when the plugin changes (another plugin, a set or a sampler in its place), the
            automations of the old plugin's parameters on that channel go with every clip that
            shows them, and so do the controller links to those parameters: a plugin's
            parameter numbers are its own, and would move the new plugin's parameter of the
            same number. The channel's volume and pan automations and links stay, and so do
            another channel's parameters'
  undo      one step, which gives the base document back byte for byte

  p15_base.json      four channels: 1 "impulse" (a sampler of impulse.wav, unrenamed, with
                     settings and a MIDI input, on insert 1), 2 "My pad" (the set Pad, renamed,
                     muted, with a release), 3 "beat" (a sampler of impulse.wav, renamed, with a
                     filter), 4 "Bell Keys" (the plugin, unrenamed, with a stored state, on
                     insert 2); each with notes
  p15_sample_to_set.json     the set "Pad" dropped on channel 1
  p15_set_to_sample.json     the starter kit's first drum dropped on channel 2
  p15_sampler_to_plugin.json the plugin "Bell Keys" dropped on channel 3
  p15_plugin_to_set.json     the set "Pad" dropped on channel 4
  p15_expected.json  each case: the channel, what was dropped, and its document
  p15_tied_base.json p15_base.json with a fifth channel, 5 "Analog Lead" (the plugin), and what
                     names the plugins' parameters: automation 1 of channel 4's parameter 7 (two
                     clips), 2 of channel 4's volume, 3 of channel 5's parameter 7; links CC 7 to
                     channel 4's pan, CC 21 on MIDI channel 1 to channel 4's parameter 0, CC 20
                     to its parameter 7, CC 22 to channel 5's parameter 7
  p15_tied_plugin_to_plugin.json  the plugin "Analog Lead" dropped on channel 4
  p15_tied_plugin_to_set.json     the set "Pad" dropped on channel 4
  p15_tied_expected.json  each tied case as p15_expected.json's, and the automations that go
  p15_long_song.json the playlist's and the rack's scroll (P6): one-bar clips of a two-bar
                     pattern at bars 1 and 19 on track 1, so the song ends with bar 19 (tick
                     7296): longer than a 1000 px playlist shows at 64 px a bar (13.75 bars),
                     it scrolls to 4 bars past its end; the pattern's 32 steps reach past a
                     1000 px rack

Run:
  python3 -I gen_p15_expected.py              print the known answers
  python3 -I gen_p15_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p15_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import copy, json, os, sys

PPQ = 96; SR = 44100; MBPM = 120000
PAD_SFZ = "sets/pad/pad.sfz"                         # the test set, as the project names it
PAD_NAME = "Pad"                                     # its set.json name
BELLS = {"id": "clap:org.example.bells", "name": "Bell Keys"}   # tests/fixtures/p7_catalogue.json
LEAD = {"id": "clap:org.example.lead", "name": "Analog Lead"}   # the same catalogue's
DEFAULT_SOUND = {"semitones": 0, "cents": 0, "velocity": 1.0, "attack": 0.0, "release": 0.0,
                 "reverse": False, "start": 0.0, "filter": False, "cutoff": 20000.0,
                 "resonance": 0.0}


def J(d): return json.dumps(d, separators=(",", ":"), ensure_ascii=False)


def kit_first_drum():
    """The starter kit's first drum, as assets/sounds/kit.json lists it, and its file."""
    here = os.path.dirname(os.path.abspath(__file__))
    with open(os.path.join(here, "..", "..", "assets", "sounds", "kit.json"), encoding="utf-8") as f:
        sound = [s for s in json.load(f)["sounds"] if s["note"] == "drum"][0]
    return sound["name"], "starter-kit/" + sound["file"]


def settings(**changed):
    """Sound settings: the defaults with `changed`, in the file's field order (the gate first,
    only when set)."""
    s = dict(DEFAULT_SOUND, **{k: v for k, v in changed.items() if k != "gate"})
    return ({"gate": changed["gate"]} if "gate" in changed else {}) | s


def channel(i, name, kind, source, volume=1.0, pan=0.0, mute=False, insert=None, port=None,
            sound=None):
    """A channel as the project file writes it: fields in order, defaults left out."""
    c = {"id": i, "name": name, "kind": kind}
    c.update(source)
    c.update({"volume": volume, "pan": pan, "mute": mute})
    if insert is not None: c["insert"] = insert
    if port is not None: c["midi_input"] = port
    if sound is not None and sound != DEFAULT_SOUND: c["sound"] = sound
    return c


def sampler_source(path): return {"sample": path}
def set_source(path): return {"instrument": path}
def plugin_source(processor): return {"processor": processor}


def project(channels):
    sounds = any("sound" in c for c in channels)
    sets = any(c["kind"] == "instrument" for c in channels)
    version = 5 if sounds else 4 if sets else 2
    notes = [{"channel": c["id"], "tick": t, "length": 0, "velocity": 1.0}
             for t in (0, 192) for c in channels]
    notes.sort(key=lambda n: (n["tick"], n["channel"]))
    return {"format_version": version, "sample_rate": SR, "tempo_mbpm": MBPM, "ppq": PPQ,
            "next_channel_id": max(c["id"] for c in channels) + 1, "channels": channels,
            "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": notes}]}


BASE = [
    channel(1, "impulse", "sampler", sampler_source("impulse.wav"), volume=0.5, pan=-0.25,
            insert=1, port="Port A",
            sound=settings(gate="hold", semitones=2, attack=0.125, reverse=True, start=0.25)),
    channel(2, "My pad", "instrument", set_source(PAD_SFZ), mute=True,
            sound=settings(release=0.5)),
    channel(3, "beat", "sampler", sampler_source("impulse.wav"),
            sound=settings(cents=-10, filter=True, cutoff=1000.0)),
    channel(4, "Bell Keys", "plugin", plugin_source(dict(BELLS, state="AAAA")), insert=2),
]


def stem(path): return os.path.splitext(os.path.basename(path))[0]


def default_name(c):
    """What a channel's sound names it when it is added."""
    if c["kind"] == "sampler": return stem(c["sample"]) if c["sample"] else "Sampler"
    if c["kind"] == "instrument": return PAD_NAME if c["instrument"] == PAD_SFZ else stem(c["instrument"])
    return c["processor"].get("name", "")


def kept_settings(sound, kind):
    if sound is None: return None
    if kind == "sampler": return sound
    if kind == "instrument": return dict(sound, reverse=False, start=0.0)
    return None


def replaced(c, kind, source, dropped_name):
    """Channel `c` given the dropped sound: the rules above."""
    name = c["name"] if c["name"] != default_name(c) else dropped_name
    return channel(c["id"], name, kind, source, volume=c["volume"], pan=c["pan"], mute=c["mute"],
                   insert=c.get("insert"), port=c.get("midi_input"),
                   sound=kept_settings(c.get("sound"), kind))


def cases():
    kit_name, kit_path = kit_first_drum()
    drops = {
        "sample_to_set": (1, "instrument", set_source(PAD_SFZ), PAD_NAME, "set"),
        "set_to_sample": (2, "sampler", sampler_source(kit_path), kit_name, "kit"),
        "sampler_to_plugin": (3, "plugin", plugin_source(dict(BELLS)), BELLS["name"], "plugin"),
        "plugin_to_set": (4, "instrument", set_source(PAD_SFZ), PAD_NAME, "set"),
    }
    out = {}
    for case, (cid, kind, source, name, dropped) in drops.items():
        channels = copy.deepcopy(BASE)
        channels[cid - 1] = replaced(channels[cid - 1], kind, source, name)
        out[case] = ({"channel": cid, "dropped": dropped, "name": name,
                      "document": "p15_%s.json" % case,
                      "channel_after": channels[cid - 1]}, project(channels))
    return out


def param(cid, number):
    """The target naming plugin channel `cid`'s parameter `number`, as the file writes it."""
    return {"channel_param": {"channel": cid, "param_id": number}}


def names_plugin_of(target, cid):
    """Whether `target` is a parameter of channel `cid`'s plugin."""
    return target.get("channel_param", {}).get("channel") == cid


# The links as the file writes them: sorted by their target, the target kinds in the engine's
# declared order (a channel's volume, its pan, ..., a plugin channel's parameter last, as
# src/project/automation.rs lists them), then by the numbers in them.
TARGET_KINDS = ["channel_volume", "channel_pan", "insert_fader", "insert_pan", "send", "effect",
                "channel_param"]


def link_order(link):
    (kind, value), = link["target"].items()
    numbers = (value["channel"], value["param_id"]) if kind == "channel_param" else (value,)
    return (TARGET_KINDS.index(kind), numbers)


TIED_CHANNELS = BASE + [channel(5, LEAD["name"], "plugin", plugin_source(dict(LEAD)))]
# (automation id, target, points as (tick, value), clips as (clip id, track, start tick))
TIED_AUTOMATIONS = [
    (1, param(4, 7), [(0, 0.25), (384, 0.75)], [(1, 1, 0), (2, 1, 768)]),
    (2, {"channel_volume": 4}, [(0, 1.0)], [(3, 2, 0)]),
    (3, param(5, 7), [(0, 0.5)], [(4, 3, 0)]),
]
TIED_LINKS = [
    {"cc": 7, "channel": None, "target": {"channel_pan": 4}},
    {"cc": 21, "channel": 0, "target": param(4, 0)},
    {"cc": 20, "channel": None, "target": param(4, 7)},
    {"cc": 22, "channel": None, "target": param(5, 7)},
]


def tied_project(channels, automations, links):
    """A project of `channels` whose automations (one-bar clips on 8 tracks) and links are
    `automations` and `links`; the ID counters stay above every ID given out, the ones gone
    too."""
    doc = project(channels)
    clips = [{"id": clip, "track": track, "start": start, "length": 4 * PPQ,
              "kind": "automation", "automation": aid}
             for aid, _, _, placed in automations for clip, track, start in placed]
    doc["playlist"] = {"next_track_id": 9,
                       "next_clip_id": 1 + max(c for *_, placed in TIED_AUTOMATIONS
                                               for c, _, _ in placed),
                       "tracks": [{"id": i, "name": "Track %d" % i, "mute": False}
                                  for i in range(1, 9)],
                       "clips": sorted(clips, key=lambda c: c["id"])}
    doc["automation"] = {"next_id": 1 + max(a[0] for a in TIED_AUTOMATIONS),
                         "list": [{"id": aid, "target": target,
                                   "points": [{"tick": t, "value": v} for t, v in points]}
                                  for aid, target, points, _ in automations]}
    doc["midi_links"] = sorted(links, key=link_order)
    return doc


def tied_cases():
    """A plugin channel holding automations and links of its plugin's parameters, given
    another plugin and a set: the rules above, the ties included."""
    drops = {
        "plugin_to_plugin": (4, "plugin", plugin_source(dict(LEAD)), LEAD["name"], "plugin"),
        "plugin_to_set": (4, "instrument", set_source(PAD_SFZ), PAD_NAME, "set"),
    }
    out = {}
    for case, (cid, kind, source, name, dropped) in drops.items():
        channels = copy.deepcopy(TIED_CHANNELS)
        channels[cid - 1] = replaced(channels[cid - 1], kind, source, name)
        kept = [a for a in TIED_AUTOMATIONS if not names_plugin_of(a[1], cid)]
        gone = [a[0] for a in TIED_AUTOMATIONS if names_plugin_of(a[1], cid)]
        links = [l for l in TIED_LINKS if not names_plugin_of(l["target"], cid)]
        out[case] = ({"channel": cid, "dropped": dropped, "name": name,
                      "document": "p15_tied_%s.json" % case,
                      "channel_after": channels[cid - 1], "automations_gone": gone},
                     tied_project(channels, kept, links))
    return out


def long_song():
    """Two one-bar clips of pattern 1 (two bars long) on track 1, at bars 1 and 19: the song
    ends with bar 19."""
    doc = project([channel(1, "beat", "sampler", sampler_source("impulse.wav"))])
    doc["patterns"][0]["length_bars"] = 2
    clips = [{"id": i + 1, "track": 1, "start": bar * 4 * PPQ, "length": 4 * PPQ,
              "kind": "pattern", "pattern": 1} for i, bar in enumerate((0, 18))]
    doc["playlist"] = {"next_track_id": 9, "next_clip_id": 3,
                       "tracks": [{"id": i, "name": "Track %d" % i, "mute": False}
                                  for i in range(1, 9)],
                       "clips": clips}
    return doc


def files():
    written = {"p15_base.json": J(project(BASE)) + "\n",
               "p15_long_song.json": J(long_song()) + "\n"}
    expected = {}
    for case, (answer, doc) in cases().items():
        written[answer["document"]] = J(doc) + "\n"
        expected[case] = answer
    written["p15_expected.json"] = J({"base": "p15_base.json", "cases": expected}) + "\n"
    written["p15_tied_base.json"] = J(tied_project(TIED_CHANNELS, TIED_AUTOMATIONS,
                                                   TIED_LINKS)) + "\n"
    tied = {}
    for case, (answer, doc) in tied_cases().items():
        written[answer["document"]] = J(doc) + "\n"
        tied[case] = answer
    written["p15_tied_expected.json"] = J({"base": "p15_tied_base.json", "cases": tied}) + "\n"
    return {name: text.encode("utf-8") for name, text in written.items()}


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
        if stale: sys.exit("not what gen_p15_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    for case, (answer, _) in cases().items():
        print("%s: channel %d takes the %s %r -> %s" % (case, answer["channel"], answer["dropped"],
              answer["name"], J(answer["channel_after"])))
    for case, (answer, doc) in tied_cases().items():
        print("tied %s: channel %d takes the %s %r; automations %s go; links left %s" % (
            case, answer["channel"], answer["dropped"], answer["name"],
            answer["automations_gone"], J(doc["midi_links"])))


if __name__ == "__main__":
    main()
