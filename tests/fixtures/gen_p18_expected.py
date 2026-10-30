"""Known answers for interchange export: the song as one MIDI file (E1), stems (E2), DAWproject
(E3), and the report each export writes beside its file (E4).

Expected values come from the rules written out below, never from engine code. The MIDI bytes
are built from event lists written out by hand under each project, the project.xml files are
written out by hand as text, and the stems' frames and levels are worked out in the comments.

  E1 song MIDI  type 1 at division 96, track 0 the conductor (tempo FF 51 = round half up of
                6e10 / milli-BPM, 4/4 as FF 58 04 04 02 18 08), then one track per channel in
                rack order named after it (FF 03), every note on MIDI channel 1. A pattern clip
                that plays (not muted, its track not muted) plays song tick T as the pattern's
                tick (T - start + offset) mod L for T in start..end, so a clip longer than its
                pattern repeats it; a note sounds its length (a step hit one step, 24 ticks) and
                is cut at its clip's end. Notes at or past the pattern's end are silent and left
                out; so are an audio or input channel's notes (its track stays, empty). Per
                channel and key, in (tick, clip order) order: a second note on the same tick is
                left out (the first stays), and a note still sounding when the same key starts
                again ends there. A note is at least one file tick long: at a project PPQ finer
                than the file's, both its ends can round to one tick. Velocity byte = round half
                up(v * 128), 1 to 127. At one tick
                note-offs (velocity 64) come first, then note-ons, each by key. Every track ends
                (FF 2F) on the song's last note-off. Running status as the writer crate keeps it:
                a meta event cancels it, the same status byte is left out.
  E2 stems      one file per playlist track, "NN name.wav" (NN from 01, the track's place; a
                name's / \\ : * ? " < > | and control characters become _), then master.wav,
                in "<project> stems/". Every file is the song's whole bars (the song end rounded
                up to a bar) at 32-bit float unless --bits says otherwise. Track k's file is the
                song with every pattern and audio clip of the other tracks muted; automation
                clips play in every file, a track's own mute is kept (its file is silence), the
                project's loop range is left out. master.wav is the song itself.
  E3 DAWproject a zip of project.xml, metadata.xml, the audio clips' files under audio/ and each
                plugin channel's saved state under plugins/. One Track per rack channel, each
                with a Channel (volume linear, pan normalized (p + 1) / 2, mute) into the master
                track; Lanes per track in beats; a pattern clip is a Notes clip of that channel's
                notes unrolled as in E1 (no overlap rule: the notes as they play), times from
                the clip's start; an audio clip is a Warps clip: the file in seconds, warped to
                beats at the tempo (its stretched length when stretched), playStart its offset
                in beats; a muted clip (or one on a muted track) has enable="false". Numbers are
                the shortest decimal that reads back as the double, with ".0" when whole.
  E4 report     "Exported <path>", then "Not carried over:" and each line, two spaces in.

Run:
  python3 -I gen_p18_expected.py              print the known answers
  python3 -I gen_p18_expected.py --write DIR  also write the files below into DIR
  python3 -I gen_p18_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes

Files: p18_song_repeat.json + .mid, p18_song_tempo.json + .mid, p18_song_fine.json + .mid and
p18_pattern_fine.mid (E1; a project at PPQ 960, for the song's and the pattern's export); p18_stems.json,
p18_stems_noloop.json, p18_stems_solo1.json, p18_stems_solo2.json (E2); p18_daw_pattern.json,
p18_daw_audio.json, p18_daw_mix.json, each with its .project.xml, and p18_loop.wav (E3);
p18_expected.json (the stems' frames and levels, every report's lines, the zips' entries).
"""
import json, os, re, struct, sys

PPQ = 96; SR = 44100; MBPM = 120000; BAR = 384; STEP = 24


def J(d): return json.dumps(d, separators=(",", ":"), ensure_ascii=False)


def version():
    """The workspace's version, as Cargo.toml states it: what Application names."""
    here = os.path.dirname(os.path.abspath(__file__))
    with open(os.path.join(here, "..", "..", "Cargo.toml"), encoding="utf-8") as f:
        text = f.read()
    section = text.split("[workspace.package]", 1)[1]
    return re.search(r'^version = "([^"]+)"', section, re.M).group(1)


# ---------- the projects (the landed format: fields in order, defaults left out) ----------
def tracks(names=None, muted=()):
    names = names or {}
    return [{"id": i, "name": names.get(i, "Track %d" % i), "mute": i in muted} for i in range(1, 9)]


def sampler(i, name, volume=1.0, pan=0.0, mute=False):
    return {"id": i, "name": name, "kind": "sampler", "sample": "impulse.wav", "volume": volume,
            "pan": pan, "mute": mute}


def audio(i, name, sample, volume=1.0):
    return {"id": i, "name": name, "kind": "audio", "sample": sample, "volume": volume, "pan": 0.0,
            "mute": False}


def plugin(i, name, processor, volume=1.0, pan=0.0):
    return {"id": i, "name": name, "kind": "plugin", "processor": processor, "volume": volume,
            "pan": pan, "mute": False}


def note(channel, tick, length, velocity, key=60):
    n = {"channel": channel, "tick": tick, "length": length, "velocity": velocity}
    if key != 60: n["key"] = key
    return n


def pattern(i, name, bars, notes):
    """A pattern; `bars` long at whatever PPQ its project has."""
    notes = sorted(notes, key=lambda n: (n["tick"], n["channel"], n.get("key", 60)))
    return {"id": i, "name": name, "length_bars": bars, "notes": notes}


def pclip(i, track, start, length, pat, offset=0, muted=False):
    c = {"id": i, "track": track, "start": start, "length": length, "kind": "pattern", "pattern": pat}
    if offset: c["offset"] = offset
    if muted: c["muted"] = True
    return c


def aclip(i, track, start, length, channel, frame_offset=0, gain=1.0, stretch=None):
    c = {"id": i, "track": track, "start": start, "length": length, "kind": "audio", "channel": channel}
    if frame_offset: c["frame_offset"] = frame_offset
    if gain != 1.0: c["gain"] = gain
    if stretch: c["stretch"] = stretch
    return c


def mclip(i, track, start, length, automation):
    return {"id": i, "track": track, "start": start, "length": length, "kind": "automation",
            "automation": automation}


def project(channels, patterns, clips, mbpm=MBPM, names=None, muted=(), loop=None,
            automation=None, ppq=PPQ):
    d = {"format_version": 2, "sample_rate": SR, "tempo_mbpm": mbpm, "ppq": ppq,
         "next_channel_id": max(c["id"] for c in channels) + 1, "channels": channels,
         "patterns": patterns}
    playlist = {"next_track_id": 9, "next_clip_id": max(c["id"] for c in clips) + 1,
                "tracks": tracks(names, muted), "clips": clips}
    if loop: playlist["loop"] = loop
    d["playlist"] = playlist
    if automation: d["automation"] = automation
    return d


# ---------- E1: the song as MIDI ----------
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


def smf1(tracks_):
    return b"MThd" + struct.pack(">IHHH", 6, 1, len(tracks_), PPQ) + b"".join(mtrk(t) for t in tracks_)


def smf0(events):
    """A format 0 file, as the pattern export writes one."""
    return b"MThd" + struct.pack(">IHHH", 6, 0, 1, PPQ) + mtrk(events)


def meta(kind, data): return [0xFF, kind] + list(varlen(len(data))) + list(data)
def name(text): return meta(0x03, text.encode("utf-8"))
def on(key, vel): return [0x90, key, vel]
def off(key): return [0x80, key, 64]
TS_44 = meta(0x58, [4, 2, 24, 8]); EOT = meta(0x2F, [])
def tempo(per_quarter): return meta(0x51, list(per_quarter.to_bytes(3, "big")))


# Project A: one sampler "kick" and one audio channel "loop".
#   Pattern 1 "Beat" (1 bar): kick 0 step v1.0 key 60; 192 step v0.5 key 60; 288 len 144 v0.75 key 64.
#   Pattern 2 "Hold" (1 bar): kick 0 len 48 v0.25 key 60; 96 len 192 v0.25 key 60.
#   Clip 1 track 1: Beat from 0 for 768 ticks (it repeats once). Clip 2 track 2: Beat from 768 for
#   192 with offset 192 (its second half). Clip 3 track 3: Beat from 960, muted. Clip 4 track 4:
#   Hold from 0 for 384. Clip 5 track 5: the loop's audio from 0 for 384.
SONG_REPEAT = project(
    [sampler(1, "kick"), audio(2, "loop", "p18_loop.wav")],
    [pattern(1, "Beat", 1, [note(1, 0, 0, 1.0), note(1, 192, 0, 0.5), note(1, 288, 144, 0.75, 64)]),
     pattern(2, "Hold", 1, [note(1, 0, 48, 0.25), note(1, 96, 192, 0.25)])],
    [pclip(1, 1, 0, 768, 1), pclip(2, 2, 768, 192, 1, offset=192), pclip(3, 3, 960, 384, 1, muted=True),
     pclip(4, 4, 0, 384, 2), aclip(5, 5, 0, 384, 2)])
# By hand, kick, in (tick, clip order):
#   clip 1: 0 (v1.0, to 24), 192 (v0.5, to 216), 288 (k64 v0.75, to 432), and again one bar
#           later: 384 (to 408), 576 (to 600), 672 (k64, to 816 cut at the clip's end 768)
#   clip 4: 0 (v0.25, to 48): the same tick and key as clip 1's, which came first: left out;
#           96 (v0.25, to 288): key 60 starts again at 192 (clip 1), so it ends at 192
#   clip 2: song 768 plays tick 192 (v0.5, to 792); 864 plays 288 (k64, to 1008 cut at 960)
#   clip 3: muted, left out. The last note-off is at 960.
# Velocity bytes: 1.0 -> 128 -> 127, 0.75 -> 96, 0.5 -> 64, 0.25 -> 32.
SONG_REPEAT_TRACKS = [
    [(0, tempo(500000)), (0, TS_44), (960, EOT)],
    [(0, name("kick")),
     (0, on(60, 127)), (24, off(60)), (96, on(60, 32)), (192, off(60)), (192, on(60, 64)),
     (216, off(60)), (288, on(64, 96)), (384, on(60, 127)), (408, off(60)), (432, off(64)),
     (576, on(60, 64)), (600, off(60)), (672, on(64, 96)), (768, off(64)), (768, on(60, 64)),
     (792, off(60)), (864, on(64, 96)), (960, off(64)), (960, EOT)],
    [(0, name("loop")), (960, EOT)],
]
# The same notes read back through smf-import into pattern 1, channel 1 (velocity = byte / 128):
# (tick, length, velocity, key); the pattern grows to hold tick 960: 3 bars.
SONG_REPEAT_REIMPORT = [(0, 24, 127 / 128, 60), (96, 96, 0.25, 60), (192, 24, 0.5, 60),
                        (288, 144, 0.75, 64), (384, 24, 127 / 128, 60), (576, 24, 0.5, 60),
                        (672, 96, 0.75, 64), (768, 24, 0.5, 60), (864, 96, 0.75, 64)]

# Project B: 140 BPM; "lead" (a plugin), "pad" (muted), "silent" (no notes).
#   Pattern 1 "Lead" (2 bars): lead 0 len 96 v0.3 key 72; 96 len 96 v0.0 key 74; 480 len 384 v1.0
#   key 76. Pattern 2 "Pad" (1 bar): pad 0 len 384 v0.5 keys 48 and 55.
#   Clip 1 track 1: Lead from 384 for 768. Clip 2 track 2: Pad from 0 for 768 (it repeats).
LEAD = {"id": "clap:org.example.lead", "name": "Analog Lead"}
SONG_TEMPO = project(
    [plugin(1, "lead", LEAD), sampler(2, "pad", mute=True), sampler(3, "silent")],
    [pattern(1, "Lead", 2, [note(1, 0, 96, 0.3, 72), note(1, 96, 96, 0.0, 74), note(1, 480, 384, 1.0, 76)]),
     pattern(2, "Pad", 1, [note(2, 0, 384, 0.5, 48), note(2, 0, 384, 0.5, 55)])],
    [pclip(1, 1, 384, 768, 1), pclip(2, 2, 0, 768, 2)], mbpm=140000)
# Tempo: (6e10 + 70000) // 140000 = 428571 (06 8A 1B). By hand:
#   lead: 384 (k72, to 480), 480 (k74, to 576), 864 (k76, to 1248 cut at the clip's end 1152)
#   pad: 0 and 384, keys 48 and 55 each, to 384 and 768: the second pair starts as the first
#        ends, so nothing is cut. The last note-off is at 1152.
# Velocity bytes: f32(0.3) * 128 = 38.4000015 -> 38; 0.0 -> 0 -> 1; 1.0 -> 127; 0.5 -> 64.
SONG_TEMPO_TRACKS = [
    [(0, tempo(428571)), (0, TS_44), (1152, EOT)],
    [(0, name("lead")), (384, on(72, 38)), (480, off(72)), (480, on(74, 1)), (576, off(74)),
     (864, on(76, 127)), (1152, off(76)), (1152, EOT)],
    [(0, name("pad")), (0, on(48, 64)), (0, on(55, 64)), (384, off(48)), (384, off(55)),
     (384, on(48, 64)), (384, on(55, 64)), (768, off(48)), (768, off(55)), (1152, EOT)],
    [(0, name("silent")), (1152, EOT)],
]

# Project C: a project at PPQ 960, ten of its ticks to a file tick. Pattern 1 (1 bar = 3840
# ticks), channel "fine": key 60 at tick 5 for 1 tick, v1.0; key 60 at 960 for 480, v0.5; key 64
# at 1000, a step (240 ticks at PPQ 960), v0.75. Clip 1 track 1: pattern 1 from 0 for 3840.
SONG_FINE = project(
    [sampler(1, "fine")],
    [pattern(1, "Pattern 1", 1, [note(1, 5, 1, 1.0), note(1, 960, 480, 0.5),
                                 note(1, 1000, 0, 0.75, 64)])],
    [pclip(1, 1, 0, 3840, 1)], ppq=960)
# By hand, a file tick is round half up(t * 96 / 960): tick 5 -> 0.5 -> 1, its end 6 -> 0.6 -> 1,
# the same tick, so the note is made one file tick long, 1 to 2 (else its note-off sorts before
# its note-on and a reader pairs the note-on with the next note-off); 960 -> 96 to 1440 -> 144;
# 1000 -> 100 to 1240 -> 124. The last note-off is at 144. Both exports, the song's and the
# pattern's, write the notes so.
FINE_NOTES = [(1, on(60, 127)), (2, off(60)), (96, on(60, 64)), (100, on(64, 96)), (124, off(64)),
              (144, off(60))]
SONG_FINE_TRACKS = [
    [(0, tempo(500000)), (0, TS_44), (144, EOT)],
    [(0, name("fine"))] + FINE_NOTES + [(144, EOT)],
]
# The pattern export (pattern 1, channel 1): format 0, the tempo, 4/4 and the pattern's name on
# tick 0, the track ending on its last event.
PATTERN_FINE = [(0, tempo(500000)), (0, TS_44), (0, name("Pattern 1"))] + FINE_NOTES + [(144, EOT)]

MIDI_GENERIC = "A MIDI file holds notes and the tempo: the sounds, the mixer and automation stay in Dawsome"

# ---------- E2: stems ----------
# Two samplers of impulse.wav (one frame of 1.0): "kick" and "hat". Pattern 1: kick at 0 and 192;
# pattern 2: hat at 96 and 288 (steps, v1.0). Track 1 "Drums": pattern 1 from 0 for 768 (twice).
# Track 2 "Hats/Top": pattern 2 from 384 for 384. Track 3: an automation clip of kick's volume from
# 0 for 768, one point 0.125 (0 + 0.125 * 4 = volume 0.5). Track 4 (muted): pattern 2 from 0. The
# loop range 0..384 is on.
STEM_AUTOMATION = {"next_id": 2, "list": [{"id": 1, "target": {"channel_volume": 1},
                                           "points": [{"tick": 0, "value": 0.125}]}]}
STEM_CLIPS = [pclip(1, 1, 0, 768, 1), pclip(2, 2, 384, 384, 2), mclip(3, 3, 0, 768, 1),
              pclip(4, 4, 0, 384, 2)]


def stems_project(clips, loop):
    return project([sampler(1, "kick"), sampler(2, "hat")],
                   [pattern(1, "Kick", 1, [note(1, 0, 0, 1.0), note(1, 192, 0, 1.0)]),
                    pattern(2, "Hat", 1, [note(2, 96, 0, 1.0), note(2, 288, 0, 1.0)])],
                   clips, names={1: "Drums", 2: "Hats/Top"}, muted=(4,), loop=loop,
                   automation=STEM_AUTOMATION)


def solo(track):
    """Track `track`'s file as a project: the other tracks' pattern and audio clips muted."""
    clips = [dict(c, muted=True) if c["track"] != track and c["kind"] != "automation" else c
             for c in STEM_CLIPS]
    return stems_project(clips, None)


STEMS = stems_project(STEM_CLIPS, {"start": 0, "end": 384, "on": True})
# The song ends at 768: 2 bars, s(768) = 768 * 60 * 44100 * 1000 // (120000 * 96) = 176400 frames.
# A beat (96 ticks) is 22050 frames. Both sides of every frame are equal (pan 0).
#   01 Drums: kick at song 0, 192, 384, 576 = frames 0, 44100, 88200, 132300, level
#             1.0 * velocity 1.0 * volume 0.5 (the automation on track 3) = 0.5
#   02 Hats_Top: hat at song 384 + 96 = 480 and 384 + 288 = 672 = frames 110250, 154350, level 1.0
#   03 Track 3: automation alone: silence. 04 Track 4: muted: silence. 05..08: empty: silence.
#   master: all six.
DRUMS = [[0, 0.5], [44100, 0.5], [88200, 0.5], [132300, 0.5]]
HATS = [[110250, 1.0], [154350, 1.0]]
STEM_FILES = [["01 Drums.wav", DRUMS], ["02 Hats_Top.wav", HATS]] + \
    [["%02d Track %d.wav" % (i, i), []] for i in range(3, 9)] + \
    [["master.wav", sorted(DRUMS + HATS)]]

# ---------- E3: DAWproject ----------
BELLS = {"id": "clap:org.example.bells", "name": "Bell Keys", "state": "AAAA"}
DAW_PATTERN = project(
    [sampler(1, "kick")],
    [pattern(1, "Beat", 1, [note(1, 0, 0, 1.0), note(1, 192, 96, 0.5, 64)])],
    [pclip(1, 1, 384, 768, 1), pclip(2, 2, 1152, 384, 1, muted=True)])
DAW_AUDIO = project(
    [audio(1, "loop", "p18_loop.wav", volume=0.8)],
    [pattern(1, "Pattern 1", 1, [])],
    [aclip(1, 1, 384, 192, 1, frame_offset=4410),
     aclip(2, 2, 768, 384, 1, gain=0.5, stretch={"mode": "resample", "frames": 44100})])
DAW_MIX = project(
    [sampler(1, "kick", volume=0.5, pan=-0.25, mute=True),
     plugin(2, "Bell Keys", BELLS, pan=0.5)],
    [pattern(1, "Pattern 1", 1, [note(1, 0, 0, 1.0), note(2, 96, 192, 0.75, 67)])],
    [pclip(1, 1, 0, 384, 1)])

LOOP_FRAMES = 22050                                  # 0.5 s at 44100 Hz, mono


def loop_wav():
    """p18_loop.wav: mono 32-bit float at 44100 Hz, 22050 frames of (i % 100) / 100."""
    data = struct.pack("<%df" % LOOP_FRAMES, *[(i % 100) / 100 for i in range(LOOP_FRAMES)])
    fmt = struct.pack("<HHIIHH", 3, 1, SR, SR * 4, 4, 32)
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack("<I", len(data)) + data
    return b"RIFF" + struct.pack("<I", len(body)) + body


HEAD = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0">
  <Application name="Dawsome" version="{version}"/>
  <Transport>
    <Tempo unit="bpm" value="120.0"/>
    <TimeSignature numerator="4" denominator="4"/>
  </Transport>
  <Structure>
"""
MASTER = """    <Track contentType="audio notes" id="master-track" name="Master">
      <Channel audioChannels="2" role="master" solo="false" id="master">
        <Mute value="false"/>
        <Pan unit="normalized" value="0.5" min="0.0" max="1.0"/>
        <Volume unit="linear" value="1.0" min="0.0" max="4.0"/>
      </Channel>
    </Track>
  </Structure>
"""

# Beat: a step at 0 (0.25 beats long), key 64 at tick 192 = beat 2.0 for 96 ticks = 1.0 beat.
# Clip 1 from tick 384 = beat 4.0 for 768 ticks = 8.0 beats: the pattern twice, the second time
# 4.0 beats on. Clip 2 from 1152 = beat 12.0 for 4.0 beats, muted: enable="false".
DAW_PATTERN_XML = HEAD + """    <Track contentType="notes" id="track1" name="kick">
      <Channel audioChannels="2" destination="master" role="regular" solo="false" id="channel1">
        <Mute value="false"/>
        <Pan unit="normalized" value="0.5" min="0.0" max="1.0"/>
        <Volume unit="linear" value="1.0" min="0.0" max="4.0"/>
      </Channel>
    </Track>
""" + MASTER + """  <Arrangement>
    <Lanes timeUnit="beats">
      <Lanes track="track1">
        <Clips>
          <Clip time="4.0" duration="8.0" name="Beat">
            <Notes>
              <Note time="0.0" duration="0.25" channel="0" key="60" vel="1.0"/>
              <Note time="2.0" duration="1.0" channel="0" key="64" vel="0.5"/>
              <Note time="4.0" duration="0.25" channel="0" key="60" vel="1.0"/>
              <Note time="6.0" duration="1.0" channel="0" key="64" vel="0.5"/>
            </Notes>
          </Clip>
          <Clip time="12.0" duration="4.0" enable="false" name="Beat">
            <Notes>
              <Note time="0.0" duration="0.25" channel="0" key="60" vel="1.0"/>
              <Note time="2.0" duration="1.0" channel="0" key="64" vel="0.5"/>
            </Notes>
          </Clip>
        </Clips>
      </Lanes>
    </Lanes>
  </Arrangement>
</Project>
"""

# The file: 22050 frames at 44100 Hz = 0.5 s, mono; at 120 BPM (2 beats a second) 1.0 beat.
# Clip 1 from beat 4.0 for 192 ticks = 2.0 beats, from frame 4410 = 0.1 s = 0.2 beats in.
# Clip 2 from beat 8.0 for 4.0 beats: the file resampled to 44100 frames = 1.0 s = 2.0 beats, so
# its warp ends at 2.0 beats for the file's 0.5 s; its gain and resample go to the report.
DAW_AUDIO_XML = HEAD + """    <Track contentType="audio" id="track1" name="loop">
      <Channel audioChannels="2" destination="master" role="regular" solo="false" id="channel1">
        <Mute value="false"/>
        <Pan unit="normalized" value="0.5" min="0.0" max="1.0"/>
        <Volume unit="linear" value="0.8" min="0.0" max="4.0"/>
      </Channel>
    </Track>
""" + MASTER + """  <Arrangement>
    <Lanes timeUnit="beats">
      <Lanes track="track1">
        <Clips>
          <Clip time="4.0" duration="2.0" playStart="0.2" name="loop">
            <Warps contentTimeUnit="seconds" timeUnit="beats">
              <Audio channels="1" duration="0.5" sampleRate="44100" timeUnit="seconds">
                <File path="audio/p18_loop.wav"/>
              </Audio>
              <Warp time="0.0" contentTime="0.0"/>
              <Warp time="1.0" contentTime="0.5"/>
            </Warps>
          </Clip>
          <Clip time="8.0" duration="4.0" playStart="0.0" name="loop">
            <Warps contentTimeUnit="seconds" timeUnit="beats">
              <Audio channels="1" duration="0.5" sampleRate="44100" timeUnit="seconds">
                <File path="audio/p18_loop.wav"/>
              </Audio>
              <Warp time="0.0" contentTime="0.0"/>
              <Warp time="2.0" contentTime="0.5"/>
            </Warps>
          </Clip>
        </Clips>
      </Lanes>
    </Lanes>
  </Arrangement>
</Project>
"""

# kick: volume 0.5, pan -0.25 -> (-0.25 + 1) / 2 = 0.375, muted. Bell Keys: pan 0.5 -> 0.75, its
# state "AAAA" (three zero bytes) in plugins/channel2.clap-preset. One clip from 0 for 4.0 beats:
# kick's step at 0 (0.25 beats); Bell Keys' key 67 at tick 96 = beat 1.0 for 192 ticks = 2.0 beats.
DAW_MIX_XML = HEAD + """    <Track contentType="notes" id="track1" name="kick">
      <Channel audioChannels="2" destination="master" role="regular" solo="false" id="channel1">
        <Mute value="true"/>
        <Pan unit="normalized" value="0.375" min="0.0" max="1.0"/>
        <Volume unit="linear" value="0.5" min="0.0" max="4.0"/>
      </Channel>
    </Track>
    <Track contentType="notes" id="track2" name="Bell Keys">
      <Channel audioChannels="2" destination="master" role="regular" solo="false" id="channel2">
        <Devices>
          <ClapPlugin deviceID="org.example.bells" deviceName="Bell Keys" deviceRole="instrument">
            <State path="plugins/channel2.clap-preset"/>
          </ClapPlugin>
        </Devices>
        <Mute value="false"/>
        <Pan unit="normalized" value="0.75" min="0.0" max="1.0"/>
        <Volume unit="linear" value="1.0" min="0.0" max="4.0"/>
      </Channel>
    </Track>
""" + MASTER + """  <Arrangement>
    <Lanes timeUnit="beats">
      <Lanes track="track1">
        <Clips>
          <Clip time="0.0" duration="4.0" name="Pattern 1">
            <Notes>
              <Note time="0.0" duration="0.25" channel="0" key="60" vel="1.0"/>
            </Notes>
          </Clip>
        </Clips>
      </Lanes>
      <Lanes track="track2">
        <Clips>
          <Clip time="0.0" duration="4.0" name="Pattern 1">
            <Notes>
              <Note time="1.0" duration="2.0" channel="0" key="67" vel="0.75"/>
            </Notes>
          </Clip>
        </Clips>
      </Lanes>
    </Lanes>
  </Arrangement>
</Project>
"""


def metadata(title):
    return ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<MetaData>\n'
            '  <Title>%s</Title>\n</MetaData>\n' % title)


SAMPLER_LINE = "kick: a sampler playing impulse.wav; DAWproject has no sampler, so its track has no instrument"


def files():
    """Every file --write writes, by name."""
    v = version()
    written = {
        "p18_song_repeat.json": (J(SONG_REPEAT) + "\n").encode("utf-8"),
        "p18_song_repeat.mid": smf1(SONG_REPEAT_TRACKS),
        "p18_song_tempo.json": (J(SONG_TEMPO) + "\n").encode("utf-8"),
        "p18_song_tempo.mid": smf1(SONG_TEMPO_TRACKS),
        "p18_song_fine.json": (J(SONG_FINE) + "\n").encode("utf-8"),
        "p18_song_fine.mid": smf1(SONG_FINE_TRACKS),
        "p18_pattern_fine.mid": smf0(PATTERN_FINE),
        "p18_stems.json": (J(STEMS) + "\n").encode("utf-8"),
        "p18_stems_noloop.json": (J(stems_project(STEM_CLIPS, None)) + "\n").encode("utf-8"),
        "p18_stems_solo1.json": (J(solo(1)) + "\n").encode("utf-8"),
        "p18_stems_solo2.json": (J(solo(2)) + "\n").encode("utf-8"),
        "p18_daw_pattern.json": (J(DAW_PATTERN) + "\n").encode("utf-8"),
        "p18_daw_audio.json": (J(DAW_AUDIO) + "\n").encode("utf-8"),
        "p18_daw_mix.json": (J(DAW_MIX) + "\n").encode("utf-8"),
        "p18_daw_pattern.project.xml": DAW_PATTERN_XML.replace("{version}", v).encode("utf-8"),
        "p18_daw_audio.project.xml": DAW_AUDIO_XML.replace("{version}", v).encode("utf-8"),
        "p18_daw_mix.project.xml": DAW_MIX_XML.replace("{version}", v).encode("utf-8"),
        "p18_loop.wav": loop_wav(),
    }
    expected = {
        "midi": {
            "repeat": {"project": "p18_song_repeat.json", "mid": "p18_song_repeat.mid",
                       "report": [MIDI_GENERIC,
                                  "loop: its audio clips are not in a MIDI file; its track is empty",
                                  "1 muted clip left out",
                                  "1 overlapping note shortened where the same key starts again",
                                  "1 note left out where another starts on the same tick and key"],
                       "reimport": {"length_bars": 3,
                                    "notes": [list(n) for n in SONG_REPEAT_REIMPORT]}},
            "tempo": {"project": "p18_song_tempo.json", "mid": "p18_song_tempo.mid",
                      "report": [MIDI_GENERIC, "pad is muted here; its notes are in the file"]},
            "fine": {"project": "p18_song_fine.json", "mid": "p18_song_fine.mid",
                     "report": [MIDI_GENERIC]},
        },
        "pattern_export": {"project": "p18_song_fine.json", "mid": "p18_pattern_fine.mid"},
        "stems": {"project": "p18_stems.json", "folder": "p18_stems stems", "bars": 2,
                  "frames": 176400, "files": STEM_FILES,
                  "solo": {"01 Drums.wav": "p18_stems_solo1.json",
                           "02 Hats_Top.wav": "p18_stems_solo2.json"},
                  "master": "p18_stems_noloop.json",
                  "report": ["The loop range is left out: every file plays the song once from its start",
                             "Track 4 is muted, so its file is silence"]},
        "dawproject": {
            "pattern": {"project": "p18_daw_pattern.json",
                        "entries": [["project.xml", "p18_daw_pattern.project.xml"],
                                    ["metadata.xml", None]],
                        "metadata": metadata("p18_daw_pattern"),
                        "report": [SAMPLER_LINE]},
            "audio": {"project": "p18_daw_audio.json",
                      "entries": [["project.xml", "p18_daw_audio.project.xml"],
                                  ["metadata.xml", None],
                                  ["audio/p18_loop.wav", "p18_loop.wav"]],
                      "metadata": metadata("p18_daw_audio"),
                      "report": ["loop clip at bar 3: gain 0.5 is not carried",
                                 "loop clip at bar 3: its resample goes as a warp to its length; "
                                 "how it stretches is up to the other program"]},
            "mix": {"project": "p18_daw_mix.json",
                    "entries": [["project.xml", "p18_daw_mix.project.xml"],
                                ["metadata.xml", None],
                                ["plugins/channel2.clap-preset", None]],
                    "metadata": metadata("p18_daw_mix"),
                    "state_hex": "000000",
                    "report": [SAMPLER_LINE,
                               "Bell Keys: Bell Keys goes as a CLAP plugin with its saved state; "
                               "loading that state is up to the other program"]},
        },
    }
    written["p18_expected.json"] = (J(expected) + "\n").encode("utf-8")
    return written


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
        if stale: sys.exit("not what gen_p18_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    written = files()
    for mid in ("p18_song_repeat.mid", "p18_song_tempo.mid"):
        print("%s: %d bytes\n  %s" % (mid, len(written[mid]), written[mid].hex()))
    expected = json.loads(written["p18_expected.json"])
    s = expected["stems"]
    print("stems: %d bars, %d frames a file" % (s["bars"], s["frames"]))
    for name_, levels in s["files"]:
        print("  %-18s %s" % (name_, " ".join("%d:%s" % (f, v) for f, v in levels) or "silence"))
    for case, d in expected["dawproject"].items():
        print("dawproject %s: %s" % (case, ", ".join(e for e, _ in d["entries"])))


if __name__ == "__main__":
    main()
