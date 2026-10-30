"""Known answers for the metronome: the clicks the player adds to what it plays, beat by beat,
while the transport plays and the metronome is on.

Expected values come from these rules, never from engine code:
  tick->sample  s(t) = floor(t*60*sr*1000 / (mbpm*96))            (the one integer rule)
  beats         every tick that is a multiple of 96 the song position plays, from its first
                sample; in pattern mode the song position runs on while the pattern repeats (its
                notes too), so beat k from the start plays at output frame s(96k)
  song loop     in song mode a loop range wraps: at the end tick's first sample the position
                jumps back to the start tick's, so a pass from tick a to the end plays tick t at
                output frame o + s(t) - s(a), o where the pass began
  click         the count-in's burst: 64 frames from the beat's first sample, frame n of it
                A*(1 - n/64)*(+1 if (n//8) even else -1), A = 0.25 (-12 dBFS) on a bar's first
                beat (a tick that is a multiple of 384) and 0.125 (-18 dBFS) on the others;
                every value a multiple of 1/512, exact in f32; the same on both sides, added
                after the master (so the record tap and an export hold none of it), each side
                of the sum held to +-max(1, |master|) (never reached here: these masters are
                silent)

  p14_click_120.json  120 BPM, 44.1 kHz, an empty one-bar pattern: two bars of clicks, every
                      beat on a whole sample (22050 apart)
  p14_click_130.json  130 BPM: beats on fractional samples (20353.85 apart), each floored from
                      the start (the second bar's second beat at s(480) = 101769, not
                      s(384) + s(96) = 101768)
  p14_click_loop.json 130 BPM in song mode, a loop range from tick 96 to tick 480 (a bar and a
                      beat): the bar's clicks, then each pass of the loop from beat 2
  p14_expected.json   each render's frames and its sounding frames

Run:
  python3 -I gen_p14_expected.py              print the known answers
  python3 -I gen_p14_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p14_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import json, os, sys

PPQ = 96; BEAT = 96; BAR = 4 * BEAT; SR = 44100
CLICK_FRAMES = 64


def s(t, mbpm): return t * 60 * SR * 1000 // (mbpm * PPQ)
def J(d): return json.dumps(d, separators=(",", ":"))


def click(n, accented):
    amplitude = 0.25 if accented else 0.125
    sign = 1.0 if (n // 8) % 2 == 0 else -1.0
    return amplitude * (1.0 - n / CLICK_FRAMES) * sign


# case: (tempo, loop range (start, end) played in song mode, or none for pattern mode)
CASES = {"click_120": (120000, None), "click_130": (130000, None),
         "click_loop": (130000, (96, 480))}
LOOPS = 2


def project(mbpm, loop):
    d = {"format_version": 2, "sample_rate": SR, "tempo_mbpm": mbpm, "ppq": PPQ,
         "next_channel_id": 2,
         "channels": [{"id": 1, "name": "kick", "kind": "sampler", "sample": "impulse.wav",
                       "volume": 1.0, "pan": 0.0, "mute": False}],
         "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": []}]}
    if loop:
        d["playlist"] = {"next_track_id": 2, "next_clip_id": 1,
                         "tracks": [{"id": 1, "name": "Track 1", "mute": False}], "clips": [],
                         "loop": {"start": loop[0], "end": loop[1], "on": True}}
    return d


def passes(mbpm, loop):
    """The passes the song position makes, as (output frame, first tick, end tick): pattern
    mode one long pass of LOOPS bars; song mode with a loop from tick 0 to its end, then LOOPS
    passes of the loop."""
    if not loop:
        return [(0, 0, LOOPS * BAR)]
    start, end = loop
    out, at = [(0, 0, end)], s(end, mbpm)
    for _ in range(LOOPS):
        out.append((at, start, end))
        at += s(end, mbpm) - s(start, mbpm)
    return out


def render(mbpm, loop):
    runs = passes(mbpm, loop)
    last = runs[-1]
    frames = last[0] + s(last[2], mbpm) - s(last[1], mbpm)
    out = [0.0] * frames
    for o, a, end in runs:
        for tick in range(-(-a // BEAT) * BEAT, end, BEAT):
            at = o + s(tick, mbpm) - s(a, mbpm)
            for k in range(CLICK_FRAMES):
                if at + k < frames:
                    out[at + k] += click(k, tick % BAR == 0)
    return frames, [[i, v] for i, v in enumerate(out) if v != 0.0]


def expected():
    renders = {}
    for name, (mbpm, loop) in CASES.items():
        frames, nonzero = render(mbpm, loop)
        renders[name] = {"fixture": "p14_%s.json" % name, "frames": frames,
                         "song": loop is not None, "nonzero": nonzero}
    return {"renders": renders, "click_frames": CLICK_FRAMES}


def files():
    out = {"p14_%s.json" % name: (J(project(mbpm, loop)) + "\n").encode()
           for name, (mbpm, loop) in CASES.items()}
    out["p14_expected.json"] = (J(expected()) + "\n").encode()
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
        if stale: sys.exit("not what gen_p14_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    for name, r in expected()["renders"].items():
        print("%s: %d frames, %d sounding; beats at %s" % (
            name, r["frames"], len(r["nonzero"]),
            [r["nonzero"][i][0] for i in range(0, len(r["nonzero"]), CLICK_FRAMES)]))


if __name__ == "__main__":
    main()
