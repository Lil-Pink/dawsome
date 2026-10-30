"""Playlist known answers (song mode, clips, loop). Expected values come from the playlist's
rules, never from engine code. Usage: python3 -I gen_p4_expected.py [fixture_dir]
Writes the p4_*.json fixtures into fixture_dir (if given) and prints the expected onsets.
A sound file loads resampled to the project rate (resample_oracle.py beside this
script), so p4_messy (impulse.wav, 44.1 kHz, in a 48 kHz project) plays a two-frame click."""
import importlib.util, json, sys, os

def resampled_click(rate):
    """impulse.wav (a single 1.0 at 44.1 kHz) as it loads in a project at `rate`."""
    here = os.path.dirname(os.path.abspath(__file__))
    spec = importlib.util.spec_from_file_location("resample_oracle", os.path.join(here, "resample_oracle.py"))
    oracle = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(oracle)
    return oracle.resample([1.0], 44100, rate)
PPQ = 96; BAR = 4 * PPQ
def t2s(tick, sr, mbpm): return tick * 60 * sr * 1000 // (mbpm * PPQ)   # the one integer rule
def J(d): return json.dumps(d, separators=(',', ':'))
def ch(i, n, vol=1.0): return {"id": i, "name": n, "kind": "sampler", "sample": "impulse.wav", "volume": vol, "pan": 0.0, "mute": False}
def note(c, tick, vel=1.0): return {"channel": c, "tick": tick, "length": 0, "velocity": vel}
PATTERNS = {1: [note(1, t) for t in (0, 96, 192, 288)],          # steps 1,5,9,13 (as p2_three_patterns)
            2: [note(1, t) for t in (48, 144)],                  # steps 3,7
            3: [note(2, 0, 0.5), note(2, 192, 0.5)]}             # channel 2, steps 1,9, velocity 0.5
def project(clips, loop=None, sr=44100, mbpm=120000, tracks=8):
    pl = {"next_track_id": tracks + 1, "next_clip_id": len(clips) + 1,
          "tracks": [{"id": i, "name": f"Track {i}", "mute": False} for i in range(1, tracks + 1)],
          "clips": [dict(id=i, track=c[0], start=c[1], length=c[2], kind="pattern", pattern=c[3]) for i, c in enumerate(clips, 1)]}
    if loop: pl["loop"] = {"start": loop[0], "end": loop[1], "on": loop[2]}
    return {"format_version": 2, "sample_rate": sr, "tempo_mbpm": mbpm, "ppq": PPQ, "next_channel_id": 3,
            "channels": [ch(1, "kick"), ch(2, "hat")],
            "patterns": [{"id": p, "name": f"Pattern {p}", "length_bars": 1, "notes": n} for p, n in PATTERNS.items()],
            "playlist": pl}

def song_events(d):
    """(song_tick, clip_id, event_index, gain) for every note the clips play (rule: window over looped data)."""
    vol = {c["id"]: c["volume"] for c in d["channels"]}
    pats = {p["id"]: (p["length_bars"] * BAR, p["notes"]) for p in d["patterns"]}
    out = []
    for c in d["playlist"]["clips"]:
        L, notes = pats[c["pattern"]]; off = c.get("offset", 0)
        for q in range(off, off + c["length"]):              # q = position in the pattern data
            for k, n in enumerate(notes):
                if n["tick"] < L and q % L == n["tick"]:
                    out.append((c["start"] + q - off, c["id"], k, n["velocity"] * vol[n["channel"]]))
    return out

def render(d, bars):
    """Output frame -> summed amplitude for `dawsome render --song --bars N` (play starts at tick 0)."""
    sr, m = d["sample_rate"], d["tempo_mbpm"]; N = t2s(bars * BAR, sr, m)
    ev = song_events(d); pl = d["playlist"]
    end = max(c["start"] + c["length"] for c in pl["clips"])
    lp = pl.get("loop")
    W, R = (t2s(lp["end"], sr, m), t2s(lp["start"], sr, m)) if lp and lp["on"] else (t2s(end, sr, m), 0)
    click = resampled_click(sr)
    frames = {}; f = 0; p = 0
    while f < N:
        if p == W and W > R: p = R                            # wrap: the frame at song sample W plays song sample R
        seg_end = W if p < W else float("inf")
        for tick, cid, k, g in ev:
            s = t2s(tick, sr, m)
            # A voice plays the loaded sound frame by frame on the output clock, across a wrap.
            for k, value in enumerate(click):
                at = f + (s - p) + k
                if p <= s < seg_end and at < N:
                    frames[at] = frames.get(at, 0.0) + g * value
        if seg_end == float("inf"): break
        f += seg_end - p; p = seg_end
    return N, dict(sorted(frames.items())), end, (W, R)

FIX = {
 "p4_two_clips":  (project([(1, 0, 384, 1), (1, 768, 384, 1)]), [3, 4]),
 "p4_resize":     (project([(1, 0, 192, 1), (2, 0, 768, 2)]), [2]),
 "p4_loop":       (project([(1, 0, 384, 1), (1, 384, 384, 2), (1, 768, 384, 1)], loop=(384, 768, True)), [3]),
 "p4_loop_off":   (project([(1, 0, 384, 1), (1, 384, 384, 2), (1, 768, 384, 1)], loop=(384, 768, False)), [3]),
 "p4_two_tracks": (project([(1, 0, 384, 1), (2, 0, 384, 3)]), [1]),
 "p4_overlap":    (project([(1, 0, 384, 1), (1, 0, 384, 1), (1, 192, 384, 1)]), [2]),
 "p4_messy":      (project([(1, 37, 250, 1), (2, 401, 384, 2)], loop=(133, 500, True), sr=48000, mbpm=140000), [2]),
}
outdir = sys.argv[1] if len(sys.argv) > 1 else None
for name, (d, bars_list) in FIX.items():
    if outdir:
        with open(os.path.join(outdir, name + ".json"), "w") as fh: fh.write(J(d) + "\n")
    for bars in bars_list:
        N, frames, end, (W, R) = render(d, bars)
        print(f"{name} --song --bars {bars}: frames={N} song_end_tick={end} wrap_at_song_sample={W} to={R}")
        print("   onsets", list(frames.keys()))
        print("   values", [round(v, 6) for v in frames.values()] if d["sample_rate"] == 44100 else list(frames.values()))
# Pattern mode is unchanged: the same file without --song renders pattern 1 (F1's onsets).
d = FIX["p4_two_clips"][0]
print("p4_two_clips (no --song) --bars 1: frames", t2s(BAR, 44100, 120000), "onsets", [t2s(t, 44100, 120000) for t in (0, 96, 192, 288)])
# Block-size invariance: every run above with --block 1, 7, 64, 1000, 4096 is byte-identical.

# Headless probe points (logical px; window 1000x560, playlist panel only, origin (0,64), zoom 64 px/bar).
# Probes sit at bar x + 24: clear of beat lines (+16, +32, +48) and of note marks (multiples of 16 for these patterns).
# (The toolbar is 64 px, and the lanes fit the 8 tracks and the "+ Track" row into the
# track area. The timeline's 10 px scroll bar moved from the header to under the tracks, so the
# area is 560 - 64 - 24 - 28 - 10 = 434 px: floor(434 / 9) - 1 = 47 px lanes, a row every 48 px; it was 436,
# 47 px lanes a row every 48, then 426 under the 72 px toolbar, 46 px lanes a row every 47.)
TOOLBAR = 64
TIME_BAR = 10
TRACKS_AREA = 560 - TOOLBAR - 24 - 28 - TIME_BAR
LANE = max(32, min(96, TRACKS_AREA // 9 - 1))
X0, PX_BAR, RULER_Y, LOOP_Y, TRACK_Y, ROW = 120, 64, TOOLBAR + 24, TOOLBAR + 24 + 16, TOOLBAR + 24 + 28, LANE + 1
lane8 = TRACK_Y + 7 * ROW + LANE // 2
print("probe bar lines (lane color #484D55) x", [X0 + b * PX_BAR for b in range(1, 14)], "y", lane8, "; lane between them at x+24")
print("probe bar-number boxes (text pixel present) x", [(X0 + b * PX_BAR + 3, X0 + b * PX_BAR + 15) for b in range(13)], "y", (RULER_Y + 2, RULER_Y + 14),
      "; mid-bar boxes (no text) at x+35..x+47")
print("probe p4_two_clips clip body #5B7FA6 at", [(X0 + b * PX_BAR + 24, TRACK_Y + 22) for b in (0, 2)],
      "lane #34383F at", [(X0 + PX_BAR + 24, TRACK_Y + 22), (X0 + 24, TRACK_Y + ROW + 22)])
print("probe p4_loop strip #C47A2C at", (X0 + PX_BAR + 24, LOOP_Y + 6), "ruler #1F2125 at", [(X0 + 24, LOOP_Y + 6), (X0 + 2 * PX_BAR + 24, LOOP_Y + 6)],
      "; p4_loop_off strip #5E5143 at", (X0 + PX_BAR + 24, LOOP_Y + 6))
