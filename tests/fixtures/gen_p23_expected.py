"""Known answers for input latency compensation: Settings measures how late the line in records
what the output plays (a click burst played while a take of the input runs, found in it by
cross-correlation), keeps the figure per (output device, input device), and an audio take of the
line in is then placed that many frames earlier.

Expected values come from the rules written out below, never from engine code:
  burst     what the measurement plays, mono at the project's rate: LEAD = rate/10 frames of
            silence, then CLICKS = 3 clicks of CLICK = 256 frames each, SPACING = rate/5 frames
            apart (click k from LEAD + k*SPACING), silence between; each click frame takes the
            next value of one LCG run from SEED, x' = (1664525*x + 1013904223) mod 2^32, as
            AMP*((x' >> 8)/2^23 - 1) with AMP = 0.25 (every value exact in f32); the clicks take
            the run in order, click 0 its first 256 values
  record    the burst plays from the take's first frame (the master's recording holds its first
            click from frame LEAD), or from the next buffer's; the measurement records RECORDING = LEAD + 2*SPACING + CLICK + rate/2 +
            rate/4 frames of the master and of the input
  measure   the input's frame j is the master's frame j + L: the measured latency is the L at
            which the burst sits in the input, searched from -rate/20 (50 ms early) to +rate/2
            (500 ms late); a burst outside those lags, or none, is "no click heard" and nothing
            is stored
  feed      the input as the stand-in device delivers it, from the take's first frame, 16-bit
            (q/32768), stereo; after the file's last frame every frame is silence
  store     the figure is kept per (output, input) device names ("" for the system's default)
            in the settings as {"output", "input", "input_latency_frames", "rate", "measured"};
            shown as ms with one decimal, frames*1000/rate rounded half up ("25.7 ms (measured)")
  place     an audio take of the line in that starts at tick T with latency L frames at the
            project's rate (a figure kept at another rate is L = round half up of
            frames*project_rate/rate) lands where it was played: p = s(T) - L; at p <= 0 the
            clip starts at tick 0 with frame_offset -p; else at the first tick t with s(t) >= p,
            frame_offset s(t) - p; its length the fewest ticks (at least 1) that hold the frames
            after the offset; a take whose frames all lie inside the offset is placed as before
            (from T, no offset, "not shifted"); the take's file is never changed; a take of the
            master, and one with no figure kept, are placed as before
  time      s(t) = floor(t*60*sr*1000 / (mbpm*96)); at 120 BPM and 48 kHz s(t) = 250*t
  report    the toolbar after a shifted take: "Take N placed M ms early (measured latency)" (or
            "late" for a negative L; "(latency set by hand)" for a figure set by hand), M =
            |L|*1000/rate rounded half up; one that could not be shifted: "Take N not shifted:
            shorter than the input latency"
  undo      a take is one undo step, which gives the document back byte for byte

  p23_feed_clean.wav   the burst on both sides, LATE_CLEAN = 1234 frames late
  p23_feed_messy.wav   the right side: the burst at -0.2 (polarity inverted), 2205 frames late,
                       an echo of it at -0.1 1536 frames after that, an LCG hum at 0.02; the
                       left side: an LCG hum at 0.05 on a DC offset of 0.02 (no click)
  p23_feed_early.wav   the burst on both sides, 960 frames early (-20 ms)
  p23_feed_late.wav    the burst on both sides 30000 frames late: past the lags searched
  p23_feed_noise.wav   the hum of the messy feed's left side on both sides, no click
  p23_feed_opposite.wav  the burst 1234 frames late, its sides in opposite polarity (a balanced
                       output into a stereo line in): a sum of the sides would be silence
  loop      a loop cable from the output into the input, played through the window:
            the stand-in input's block k (512 frames) is the output's block k - 3 times a gain of
            1.25 or 2.0 (clipped to +-1) plus an LCG hum at 0.00316 (-50 dBFS, seed 3, the
            burst's rule), so the latency is 1536 frames whatever the gain; the measurement plays
            no input channel back out (else the loop feeds back: its echo outscores the click
            and the output runs past full scale): the output is the burst, then silence
  p23_line.json        48 kHz, 120 BPM: input channel 1 "Oriole" on "USB Audio Line Input"
                       (stereo 1+2), pattern 1 of one bar, no clip
  p23_*_after.json     the document after each placement case
  p23_expected.json    every case: its inputs and the values the tests compare

Run:
  python3 -I gen_p23_expected.py              print the known answers
  python3 -I gen_p23_expected.py --write DIR  also write the files above into DIR
  python3 -I gen_p23_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes
"""
import copy, hashlib, json, os, struct, sys

SR = 48000; PPQ = 96; MBPM = 120000; BAR = 4 * PPQ
LEAD = SR // 10; SPACING = SR // 5; CLICK = 256; CLICKS = 3; AMP = 0.25; SEED = 0x5EED1234
EARLIEST = -(SR // 20); LATEST = SR // 2
BURST = LEAD + (CLICKS - 1) * SPACING + CLICK
RECORDING = BURST + SR // 2 + SR // 4
DEVICE = "USB Audio Line Input"
TAKE_FRAMES = 24000


def J(d): return json.dumps(d, separators=(",", ":"), ensure_ascii=False)
def s(t): return t * 60 * SR * 1000 // (MBPM * PPQ)
def first_tick_at_or_after(sample): return -(-sample * MBPM * PPQ // (60 * SR * 1000))
def ticks_holding(start, frames): return max(1, first_tick_at_or_after(s(start) + frames) - start)
def half_up(n, d): return (abs(n) * 2 + d) // (2 * d) * (1 if n >= 0 else -1)


def lcg(x): return (1664525 * x + 1013904223) % 2 ** 32


def burst():
    out = [0.0] * BURST
    x = SEED
    for k in range(CLICKS):
        for t in range(CLICK):
            x = lcg(x)
            out[LEAD + k * SPACING + t] = AMP * ((x >> 8) / 2 ** 23 - 1)
    return out


def hum(frames, level, seed):
    x, out = seed, []
    for _ in range(frames):
        x = lcg(x)
        out.append(level * ((x >> 8) / 2 ** 23 - 1))
    return out


def q16(v): return max(-32768, min(32767, round(v * 32768)))


def wav_s16_stereo(left, right):
    data = b"".join(struct.pack("<hh", q16(l), q16(r)) for l, r in zip(left, right))
    fmt = struct.pack("<HHIIHH", 1, 2, SR, SR * 4, 4, 16)
    return (b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVEfmt " + struct.pack("<I", 16) + fmt
            + b"data" + struct.pack("<I", len(data)) + data)


def placed(signal, at, gain, frames):
    """`signal` from frame `at` (cut where it would start before frame 0), times `gain`."""
    out = [0.0] * frames
    for n, v in enumerate(signal):
        if 0 <= at + n < frames:
            out[at + n] += gain * v
    return out


def mixed(*parts): return [sum(values) for values in zip(*parts)]


def feeds():
    b = burst()
    def both(late): return placed(b, late, 1.0, BURST + max(late, 0))
    messy_len = BURST + 2205 + 1536
    left_hum = [v + 0.02 for v in hum(messy_len, 0.05, 7)]
    right = mixed(placed(b, 2205, -0.2, messy_len), placed(b, 2205 + 1536, -0.1, messy_len),
                  hum(messy_len, 0.02, 11))
    clean, early, late = both(1234), both(-960), both(30000)
    noise = [v + 0.02 for v in hum(BURST, 0.05, 7)]
    opposite = both(1234)
    return {"p23_feed_opposite.wav": (opposite, [-v for v in opposite], 1234),
            "p23_feed_clean.wav": (clean, clean, 1234),
            "p23_feed_messy.wav": (left_hum, right, 2205),
            "p23_feed_early.wav": (early, early, -960),
            "p23_feed_late.wav": (late, late, None),
            "p23_feed_noise.wav": (noise, noise, None)}


def ms1(frames, rate):
    tenths = half_up(frames * 10000, rate)
    sign = "-" if tenths < 0 else ""
    return "%s%d.%d ms" % (sign, abs(tenths) // 10, abs(tenths) % 10)


# ---------- the placement cases, through the window ----------
def tracks(): return [{"id": i, "name": "Track %d" % i, "mute": False} for i in range(1, 9)]


def line_project():
    return {"format_version": 6, "sample_rate": SR, "tempo_mbpm": MBPM, "ppq": PPQ,
            "next_channel_id": 2,
            "channels": [{"id": 1, "name": "Oriole", "kind": "input",
                          "input": {"device": DEVICE, "channels": {"stereo": [0, 1]}},
                          "volume": 1.0, "pan": 0.0, "mute": False}],
            "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": []}]}


def placement(marker, latency):
    """(start, frame_offset, length, shifted) of a take from `marker`, `latency` frames late."""
    if latency is None:
        return marker, 0, ticks_holding(marker, TAKE_FRAMES), False
    p = s(marker) - latency
    if p <= 0:
        start, offset = 0, -p
    else:
        start = first_tick_at_or_after(p)
        offset = s(start) - p
    if offset >= TAKE_FRAMES:
        return marker, 0, ticks_holding(marker, TAKE_FRAMES), False
    return start, offset, ticks_holding(start, TAKE_FRAMES - offset), True


def after_take(start, offset, length):
    doc = line_project()
    doc["channels"].append({"id": 2, "name": "Take 1", "kind": "audio",
                            "sample": "samples/p23_line take 1.wav", "volume": 1.0, "pan": 0.0,
                            "mute": False})
    doc["next_channel_id"] = 3
    clip = {"id": 1, "track": 1, "start": start, "length": length, "kind": "audio", "channel": 2}
    if offset: clip["frame_offset"] = offset
    doc["playlist"] = {"next_track_id": 9, "next_clip_id": 2, "tracks": tracks(), "clips": [clip]}
    return doc


def entry(frames, rate, measured=True):
    return {"output": "", "input": DEVICE, "input_latency_frames": frames, "rate": rate,
            "measured": measured}


# name: (song marker, the figure kept (None: none), what the take records)
PLACEMENTS = {
    "shifted": (192, entry(1134, 44100), "input"),       # 1134 at 44.1 kHz is 1234 at 48 kHz
    "at_zero": (0, entry(1234, SR), "input"),
    "saturated": (0, entry(30000, SR), "input"),
    "late": (192, entry(-960, SR, measured=False), "input"),
    "unset": (192, None, "input"),
    "master": (192, entry(1234, SR), "master"),
}


def placements():
    cases, docs = {}, {}
    for name, (marker, kept, source) in PLACEMENTS.items():
        latency = None
        if kept is not None and source == "input":
            latency = half_up(kept["input_latency_frames"] * SR, kept["rate"])
        start, offset, length, shifted = placement(marker, latency)
        doc = after_take(start, offset, length)
        document = "p23_%s_after.json" % name
        docs[document] = doc
        if shifted:
            word = "early" if latency > 0 else "late"
            by = "measured latency" if kept["measured"] else "latency set by hand"
            report = "Take 1 placed %d ms %s (%s)" % (half_up(abs(latency) * 1000, SR), word, by)
        elif latency is not None:
            report = "Take 1 not shifted: shorter than the input latency"
        else:
            report = None
        cases[name] = {"marker": marker, "source": source, "frames": TAKE_FRAMES,
                       "settings": {"input_latency": [kept]} if kept else {},
                       "latency": latency, "start": start, "frame_offset": offset,
                       "length": length, "report": report, "document": document}
    return cases, docs


def files():
    written = {}
    measures = {}
    for name, (left, right, late) in feeds().items():
        written[name] = wav_s16_stereo(left, right)
        measures[name[len("p23_"):-len(".wav")]] = {
            "file": name, "frames": len(left), "latency": late,
            "shown": (ms1(late, SR) + " (measured)") if late is not None else None}
    cases, docs = placements()
    for name, doc in docs.items():
        written[name] = (J(doc) + "\n").encode()
    written["p23_line.json"] = (J(line_project()) + "\n").encode()
    data = struct.pack("<%df" % BURST, *burst())
    expected = {"rate": SR, "device": DEVICE, "lead": LEAD, "spacing": SPACING, "click": CLICK,
                "clicks": CLICKS, "seed": SEED, "burst_frames": BURST,
                "burst_sha256": hashlib.sha256(data).hexdigest(),
                "burst_probes": [[k, burst()[k]] for k in (LEAD, LEAD + 1, LEAD + SPACING,
                                                            LEAD + 2 * SPACING + CLICK - 1)],
                "recording_frames": RECORDING, "earliest": EARLIEST, "latest": LATEST,
                "measure": measures, "place": cases,
                "loop": {"block": 512, "blocks_late": 3, "gains": [1.25, 2.0],
                         "hum": 0.00316, "hum_seed": 3,
                         "latency": 3 * 512, "shown": ms1(3 * 512, SR) + " (measured)"}}
    written["p23_expected.json"] = (J(expected) + "\n").encode()
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
        if stale: sys.exit("not what gen_p23_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check": return
    elif args: sys.exit(__doc__)
    expected = json.loads(files()["p23_expected.json"])
    print("burst: %d frames, sha256 %s" % (expected["burst_frames"], expected["burst_sha256"][:16]))
    for name, m in expected["measure"].items():
        print("measure %s: %s" % (name, m["shown"] or "no click heard"))
    for name, c in expected["place"].items():
        print("place %s: start %d, offset %d, length %d; %s" % (
            name, c["start"], c["frame_offset"], c["length"], c["report"]))


if __name__ == "__main__":
    main()
