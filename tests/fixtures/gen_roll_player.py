"""Known answers for the piano roll's player: `PlayMode::Channel { pattern, channel, once }`
plays one channel's notes of one pattern through the one transport, every other channel's notes
left out, the mixer (gain, pan, inserts, effects, routing) as ever.

Expected values come from these rules, never from engine code:
  tick->sample  s(t) = floor(t*60*SR*1000 / (mbpm*96)); at 48 kHz and 120 BPM s(t) = 250t
  pattern       L = bars*4*96 ticks; it plays s(L) frames a pass
  channel mode  the render of channel C of pattern P is the render of P in pattern mode with
                every channel but C muted (a muted channel starts no voice): byte for byte the
                same WAV, which `dawsome render --pattern P --channel C` and
                `dawsome render --pattern P` of the reference project (the others muted) write
  once          `--once` plays the pattern a single time: the render is s(L) frames, the first
                pass of the looped render
  loop          N passes of a pattern whose every sound ends inside its pass render N copies:
                the halves of a two-pass render are the same bytes
  sounds        rp_tone.wav: 100 frames of 0.25; rp_hit.wav: 40 frames of 0.5 (mono 32-bit
                float at SR). A sampler plays its whole sound from the note's frame at key 60
                (speed 1), velocity 1.0, volume 1.0, centred (the pan law gives exactly 1 on
                both sides), into the master at its default fader (unity): the sound's value
                on both sides from s(t) on

Cases (48 kHz, 120 BPM; channel 1 and 2 samplers):
  rp_keys_drums.json      "keys" (rp_tone) and "drums" (rp_hit); pattern 1 "Pattern 1" (one bar,
                          a decoy: the keys and the drums at tick 0) and pattern 2 "Verse" (two
                          bars): keys at 0, 96, 288, 672 and 767 (length 24), drums at 0, 48, 144,
                          384 and 600 (step hits). Channel "keys" of "Verse": the keys alone, the
                          drums silent; rp_keys_drums_ref.json is the same with the drums muted
  rp_shared_insert.json   "lead" (rp_tone) and "pad" (rp_hit) both on insert 1, which holds a
                          delay (3 steps, feedback 0.5, mix 1.0); "Pattern 1" (one bar): lead at
                          0 and 192 (length 24), pad at 96 and 288. Channel "lead": the lead
                          through the shared insert's delay, the pad silent;
                          rp_shared_insert_ref.json is the same with the pad muted
  rp_empty.json           "keys" (rp_tone, notes at 0 and 192) and "empty" (rp_hit, no note);
                          "Pattern 1" (one bar). Channel "empty": silence, s(L) frames;
                          rp_empty_ref.json is the same with the keys muted
  rp_input.json           "keys" (rp_tone, notes at 0 and 192) and "line", an input channel
                          ({"mono":0}, volume 1.0, centred: the channel pan law's sqrt(2)*sin(pi/4)
                          is exactly 1 in f32), which plays --input-feed rp_feed.wav (96000 frames
                          of 0.125) whatever the notes: format version 6. Channel "keys": the
                          keys alone, the line left out though it has no notes
                          (rp_input_line_muted.json mutes it); channel "line": the line alone,
                          0.125 on every frame (rp_input_keys_muted.json mutes the keys)
  rp_midi_out.json        "synth", an input channel with a MIDI out (MIDI channel 1, latency 10 ms:
                          L = floor(10*SR/1000) = 480 frames), "Pattern 1" (one bar): key 60 at
                          tick 0 for 96 ticks at velocity 1.0, key 62 at tick 288 for 48 at 0.5;
                          format version 7. Its notes go out as gen_p19_expected.py's rules send
                          them (L frames ahead of the natural frame, never before frame 0; a note
                          whose note on waited for frame 0 keeps its whole gate; the render's
                          Stop at its end, frame F, ends what still sounds; bytes 9n kk vv and 8n
                          kk 40, vv = round(velocity*128) held to 1..127). Played --once nothing
                          past the pattern's end is sent, though the next pass's first note's
                          sending frame (s(384) - L) lies inside the render; looped (--bars 1) it
                          is sent there and the Stop at F ends it. Changed late, in the call that
                          starts at frame W = 95744 (187 calls of 512, the pass's last L + one
                          call), with the transport playing on to frame 120320 (235 calls): Loop
                          switched on, the next pass's downbeat (natural 96000) goes out on W, the
                          earliest frame still possible (a note is never sent before the start of
                          the call that finds it), and, its note on having waited, keeps its whole
                          gate: off at W + 24000 = 119744 (not 120000 - L); the pattern grown to
                          two bars with key 67 at tick 384 (96 ticks, velocity 1.0, once kept):
                          bar 2's note goes out on W the same way, off at 119744
  rp_expected.json        each pair's project, reference, pattern, channel, bars, frames and (for
                          the two cases worked by hand) the sounding runs [first frame, frames,
                          value on both sides]; the frames where the plain pattern sounds a drum
                          the channel's render leaves silent; the once and loop renders' frames;
                          the player's figures (the pattern's frames a pass, the last keys note's
                          frame, which a longer sound rings on from past the pattern's end)

Run:
  python3 -I gen_roll_player.py              print the known answers
  python3 -I gen_roll_player.py --write DIR  also write the files above into DIR
  python3 -I gen_roll_player.py --check DIR  exit 1 unless the files in DIR are what --write
                                             writes
"""
import json, os, struct, sys

PPQ = 96; SR = 48000; MBPM = 120000; BAR = 4 * PPQ
TONE = [0.25] * 100
HIT = [0.5] * 40


def s(t): return t * 60 * SR * 1000 // (MBPM * PPQ)
def J(d): return json.dumps(d, separators=(",", ":"))


# ---------- the sounds ----------
def wav_bytes(frames):
    """A mono 32-bit float WAV at SR."""
    data = struct.pack("<%df" % len(frames), *frames)
    fmt = struct.pack("<HHIIHH", 3, 1, SR, SR * 4, 4, 32)
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack("<I", len(data)) + data
    return b"RIFF" + struct.pack("<I", len(body)) + body


FEED = [0.125] * (4 * PPQ * 250)
SOUNDS = {"rp_tone.wav": TONE, "rp_hit.wav": HIT, "rp_feed.wav": FEED}


# ---------- the projects ----------
def channel(cid, name, sample, muted, insert=0):
    c = {"id": cid, "name": name, "kind": "sampler", "sample": sample, "volume": 1.0, "pan": 0.0,
         "mute": muted}
    if insert: c["insert"] = insert
    return c


def note(cid, tick, length):
    return {"channel": cid, "tick": tick, "length": length, "velocity": 1.0}


def pattern(pid, name, bars, notes):
    """`notes` as (channel, tick, length), kept in the file's order: by tick, then channel."""
    return {"id": pid, "name": name, "length_bars": bars,
            "notes": [note(*n) for n in sorted(notes, key=lambda n: (n[1], n[0]))]}


def insert(i, effects=()):
    return {"id": i, "name": "Master" if i == 0 else "Insert %d" % i, "fader": 0.8, "pan": 0.0,
            "mute": False, "routes": [] if i == 0 else [{"to": 0, "amount": 1.0}],
            "effects": list(effects)}


DELAY = {"id": 1, "slot": 0, "processor": {"id": "dawsome.delay",
                                           "params": {"feedback": 0.5, "mix": 1.0,
                                                      "sync_steps": 3.0}}}


def project(channels, patterns, mixer=None):
    d = {"format_version": 2, "sample_rate": SR, "tempo_mbpm": MBPM, "ppq": PPQ,
         "next_channel_id": len(channels) + 1, "channels": channels, "patterns": patterns}
    if mixer: d["mixer"] = mixer
    return d


KEYS, DRUMS = 1, 2
VERSE_KEYS = [0, 96, 288, 672, 767]
VERSE_DRUMS = [0, 48, 144, 384, 600]


def keys_drums(drums_muted):
    verse = [(KEYS, t, 24) for t in VERSE_KEYS] + [(DRUMS, t, 0) for t in VERSE_DRUMS]
    return project([channel(KEYS, "keys", "rp_tone.wav", False),
                    channel(DRUMS, "drums", "rp_hit.wav", drums_muted)],
                   [pattern(1, "Pattern 1", 1, [(KEYS, 0, 24), (DRUMS, 0, 0)]),
                    pattern(2, "Verse", 2, verse)])


LEAD, PAD = 1, 2


def shared_insert(pad_muted):
    notes = [(LEAD, 0, 24), (LEAD, 192, 24), (PAD, 96, 0), (PAD, 288, 0)]
    mixer = {"next_effect_id": 2, "solo": [],
             "inserts": [insert(0), insert(1, [DELAY])] + [insert(i) for i in range(2, 17)]}
    return project([channel(LEAD, "lead", "rp_tone.wav", False, insert=1),
                    channel(PAD, "pad", "rp_hit.wav", pad_muted, insert=1)],
                   [pattern(1, "Pattern 1", 1, notes)], mixer)


EMPTY = 2


def empty(keys_muted):
    return project([channel(KEYS, "keys", "rp_tone.wav", keys_muted),
                    channel(EMPTY, "empty", "rp_hit.wav", False)],
                   [pattern(1, "Pattern 1", 1, [(KEYS, 0, 24), (KEYS, 192, 24)])])


LINE = 2


def with_input(keys_muted, line_muted):
    line = {"id": LINE, "name": "line", "kind": "input",
            "input": {"device": "", "channels": {"mono": 0}},
            "volume": 1.0, "pan": 0.0, "mute": line_muted}
    d = project([channel(KEYS, "keys", "rp_tone.wav", keys_muted), line],
                [pattern(1, "Pattern 1", 1, [(KEYS, 0, 24), (KEYS, 192, 24)])])
    d["format_version"] = 6
    return d


SYNTH = 1
LATENCY_MS = 10
MIDI_NOTES = [(0, 96, 1.0, 60), (288, 48, 0.5, 62)]


def midi_out_project():
    synth = {"id": SYNTH, "name": "synth", "kind": "input",
             "input": {"device": "", "channels": {"stereo": [0, 1]}},
             "volume": 1.0, "pan": 0.0, "mute": False,
             "midi_out": {"port": "Synth", "channel": 1, "latency_ms": LATENCY_MS}}
    notes = [{"channel": SYNTH, "tick": t, "length": n, "velocity": v, **({"key": k} if k != 60 else {})}
             for t, n, v, k in MIDI_NOTES]
    d = project([synth], [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": notes}])
    d["format_version"] = 7
    return d


LATE_SWAP = 187 * 512
LATE_UNTIL = 235 * 512
GROWN_NOTE = (384, 96, 1.0, 67)


def late_events(note):
    """The messages of the one-bar pass played once, then `note` (tick, length, velocity, key),
    found in the call starting at LATE_SWAP: sent there (its sending frame s(t) - L is before it),
    and ended its whole gate later, the note on having waited."""
    t, n, v, key = note
    L = LATENCY_MS * SR // 1000
    natural, gate = s(t), s(t + n) - s(t)
    on = max(natural - L, LATE_SWAP)
    off = max(natural + gate - L, on + gate if natural - L < LATE_SWAP else 0)
    vv = min(127, max(1, int(v * 128 + 0.5)))
    return midi_events(1) + [{"frame": on, "channel": SYNTH, "bytes": "90 %02X %02X" % (key, vv)},
                             {"frame": off, "channel": SYNTH, "bytes": "80 %02X 40" % key}]


def midi_events(passes_sent):
    """The messages a one-bar render sends: the notes of the passes whose note ons are sent
    (`passes_sent`: 1 played once; 2 looped, the second pass's first note sent L ahead of the
    bar's end), each [frame, bytes], the Stop at F = s(BAR) ending what still sounds."""
    L = LATENCY_MS * SR // 1000
    F = s(BAR)
    out = []
    for k in range(passes_sent):
        for t, n, v, key in MIDI_NOTES:
            natural = s(k * BAR + t)
            on = max(0, natural - L)
            if on >= F: continue
            gate = s(k * BAR + t + n) - natural
            off = max(natural + gate - L, on + 1, on + gate if natural - L < 0 else 0)
            vv = min(127, max(1, int(v * 128 + 0.5)))
            out.append((on, 1, "90 %02X %02X" % (key, vv)))
            out.append((min(off, F), 0, "80 %02X 40" % key))
    return [{"frame": frame, "channel": SYNTH, "bytes": b} for frame, _, b in sorted(out)]


PROJECTS = {
    "rp_keys_drums.json": keys_drums(False),
    "rp_keys_drums_ref.json": keys_drums(True),
    "rp_shared_insert.json": shared_insert(False),
    "rp_shared_insert_ref.json": shared_insert(True),
    "rp_empty.json": empty(False),
    "rp_empty_ref.json": empty(True),
    "rp_input.json": with_input(False, False),
    "rp_input_line_muted.json": with_input(False, True),
    "rp_input_keys_muted.json": with_input(True, False),
    "rp_midi_out.json": midi_out_project(),
}


# ---------- the renders ----------
def runs(hits, frames):
    """The frames `hits` (first frame, sound) sound in a render `frames` long, as [first frame,
    frames, value]: each stretch of one value."""
    out = [0.0] * frames
    for first, sound in hits:
        for i, v in enumerate(sound):
            if first + i < frames: out[first + i] += v
    found = []
    for i, v in enumerate(out):
        if v == 0.0: continue
        if found and found[-1][0] + found[-1][1] == i and found[-1][2] == v: found[-1][1] += 1
        else: found.append([i, 1, v])
    return found


VERSE_FRAMES = s(2 * BAR)
ONE_BAR = s(BAR)


def expected():
    keys_alone = runs([(s(t), TONE) for t in VERSE_KEYS], VERSE_FRAMES)
    # Where the plain pattern sounds a drum and no key: the channel's render is silent there.
    drum_alone = [s(t) for t in VERSE_DRUMS
                  if not any(s(k) <= s(t) < s(k) + len(TONE) for k in VERSE_KEYS)]
    renders = {
        "keys_drums": {"project": "rp_keys_drums.json", "reference": "rp_keys_drums_ref.json",
                       "pattern": "Verse", "channel": "keys", "bars": 2,
                       "frames": VERSE_FRAMES, "runs": keys_alone,
                       "silenced": [[frame, HIT[0]] for frame in drum_alone]},
        "shared_insert": {"project": "rp_shared_insert.json",
                          "reference": "rp_shared_insert_ref.json", "pattern": "Pattern 1",
                          "channel": "lead", "bars": 1, "frames": ONE_BAR},
        "empty": {"project": "rp_empty.json", "reference": "rp_empty_ref.json",
                  "pattern": "Pattern 1", "channel": "empty", "bars": 1, "frames": ONE_BAR,
                  "runs": []},
        "input_other": {"project": "rp_input.json", "reference": "rp_input_line_muted.json",
                        "feed": "rp_feed.wav", "pattern": "Pattern 1", "channel": "keys",
                        "bars": 1, "frames": ONE_BAR,
                        "runs": runs([(s(0), TONE), (s(192), TONE)], ONE_BAR)},
        "input_alone": {"project": "rp_input.json", "reference": "rp_input_keys_muted.json",
                        "feed": "rp_feed.wav", "pattern": "Pattern 1", "channel": "line",
                        "bars": 1, "frames": ONE_BAR, "runs": runs([(0, FEED)], ONE_BAR)},
    }
    return {
        "renders": renders,
        "once": {"project": "rp_keys_drums.json", "pattern": "Verse", "channel": "keys",
                 "frames": VERSE_FRAMES},
        "loop": {"project": "rp_keys_drums.json", "pattern": "Verse", "channel": "keys",
                 "bars": 4, "frames": 2 * VERSE_FRAMES, "pass_frames": VERSE_FRAMES},
        "player": {"project": "rp_keys_drums.json", "pattern_id": 2, "channel_id": KEYS,
                   "pattern_frames": VERSE_FRAMES, "last_note_frame": s(VERSE_KEYS[-1])},
        "midi_out": {"project": "rp_midi_out.json", "pattern": "Pattern 1", "channel": "synth",
                     "frames": ONE_BAR, "once": midi_events(1), "looped": midi_events(2),
                     "late": {"swap_frame": LATE_SWAP, "until_frame": LATE_UNTIL,
                              "loop_on": late_events((BAR,) + MIDI_NOTES[0][1:]),
                              "grown": late_events(GROWN_NOTE), "grown_note": list(GROWN_NOTE)}},
    }


def files():
    out = {name: wav_bytes(frames) for name, frames in SOUNDS.items()}
    for name, doc in PROJECTS.items():
        out[name] = (J(doc) + "\n").encode()
    out["rp_expected.json"] = (J(expected()) + "\n").encode()
    return out


def sanity():
    """The cases' figures, worked by hand, against the rules above."""
    assert s(1) == 250 and ONE_BAR == 96000 and VERSE_FRAMES == 192000
    want = expected()
    keys = want["renders"]["keys_drums"]
    assert keys["runs"] == [[0, 100, 0.25], [24000, 100, 0.25], [72000, 100, 0.25],
                            [168000, 100, 0.25], [191750, 100, 0.25]]
    # Every sound of the Verse ends inside its pass, so two passes are two copies: the last
    # key, at tick 767 (frame 191750), ends on frame 191850, before 192000.
    assert s(VERSE_KEYS[-1]) + len(TONE) <= VERSE_FRAMES
    assert all(s(t) + len(HIT) <= VERSE_FRAMES for t in VERSE_DRUMS)
    # The drum at tick 0 sounds with a key; the other four alone.
    assert keys["silenced"] == [[12000, 0.5], [36000, 0.5], [96000, 0.5], [150000, 0.5]]
    assert want["loop"]["frames"] == 384000 and want["player"]["last_note_frame"] == 191750
    assert want["renders"]["input_other"]["runs"] == [[0, 100, 0.25], [48000, 100, 0.25]]
    assert want["renders"]["input_alone"]["runs"] == [[0, 96000, 0.125]] and len(FEED) == ONE_BAR
    # By hand: L = 480; key 60 natural 0 waits for frame 0, keeps its gate (24000); key 62 at
    # 72000 - 480, off at 84000 - 480; looped, the next pass's key 60 at 96000 - 480, ended by
    # the Stop at 96000.
    once = [(e["frame"], e["bytes"]) for e in want["midi_out"]["once"]]
    assert once == [(0, "90 3C 7F"), (24000, "80 3C 40"), (71520, "90 3E 40"),
                    (83520, "80 3E 40")], once
    looped = [(e["frame"], e["bytes"]) for e in want["midi_out"]["looped"]]
    assert looped == once + [(95520, "90 3C 7F"), (96000, "80 3C 40")], looped
    # By hand, the late changes: W = 95744 is past both downbeats' sending frame 95520, so each
    # goes out on W and ends a whole gate (24000) later, 119744, inside the 120320 played.
    late = want["midi_out"]["late"]
    for case, key in (("loop_on", "3C"), ("grown", "43")):
        tail = [(e["frame"], e["bytes"]) for e in late[case]][4:]
        assert tail == [(95744, "90 %s 7F" % key), (119744, "80 %s 40" % key)], (case, tail)
        assert late[case][:4] == want["midi_out"]["once"]
    # The references differ from their projects in the one channel's mute alone.
    for name, ref in (("rp_keys_drums", "rp_keys_drums_ref"),
                      ("rp_shared_insert", "rp_shared_insert_ref"), ("rp_empty", "rp_empty_ref"),
                      ("rp_input", "rp_input_line_muted"), ("rp_input", "rp_input_keys_muted")):
        a, b = PROJECTS[name + ".json"], PROJECTS[ref + ".json"]
        diff = [(x["name"], x["mute"], y["mute"]) for x, y in zip(a["channels"], b["channels"])
                if x != y]
        assert len(diff) == 1 and diff[0][1] is False and diff[0][2] is True, diff


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
        if stale: sys.exit("not what gen_roll_player.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    answers = expected()
    for name, r in answers["renders"].items():
        print("%-14s %s/%s: %6d frames, runs %s" % (
            name, r["pattern"], r["channel"], r["frames"], r.get("runs", "(equivalence only)")))
    print("once:", answers["once"])
    print("loop:", answers["loop"])
    print("player:", answers["player"])


if __name__ == "__main__":
    main()
