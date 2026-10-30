#!/usr/bin/env python3
"""The helper scripts/measure.sh runs, and the tables of its results; five commands.

    measure.py fixtures OUT_DIR
        Writes the generated projects into OUT_DIR:
        - g_500_tracks.json: the playlist's track cap (500 tracks) holding 50 one-bar pattern
          clips spread over every tenth track and 16 bars, one sampler channel.
        - h_session.json: a working session: 8 sampler channels on inserts 1-8; a delay, an
          EQ, a reverb, Pressure4 and Density on inserts 1-5; a 4-bar pattern of 64 notes over
          the 8 channels; 16 tracks holding 32 clips.
        Both play the repository's impulse.wav, named by absolute path.

    measure.py sample PID WARMUP_S DURATION_S
        Samples one running process and prints one JSON object. After WARMUP_S, over
        DURATION_S:
        - CPU: nanoseconds on a CPU per thread (/proc/PID/task/TID/schedstat) as % of one core,
          grouped as `ui` (the main thread, TID == PID), `audio` (the thread the output callback
          runs on: PipeWire's loop thread `pw_out`, or `dawsome-parked` without a device) and
          `other` (toolkit, driver and worker threads).
        - Context switches per second, voluntary plus involuntary (/proc/PID/task/TID/status).
        - RSS (/proc/PID/status) once a second: the peak, and the peak of its anonymous part
          (the process's own memory, without mapped files such as the graphics driver).
        - GPU (`nvidia-smi pmon -s um -d 1`): this process's SM % averaged over the seconds it
          was listed ('-', no sample that second, counts as 0) and its frame-buffer memory
          (VRAM), the peak.
        - The same CPU and switch figures by thread name (`by_thread`), for finding who wakes.
        Threads that start during the window count from zero; threads that end in it are missed.

    measure.py collect RESULTS_JSON SCENARIO LABEL SAMPLE_JSON APP_OUTPUT
        Adds one scenario to RESULTS_JSON (made when missing; an earlier run of the scenario is
        replaced): the sample, and what the app printed under --stats: the audio callback
        timings and the window's updates and frames per second (over the whole run, warm-up
        included). Each scenario's window makes 2 updates a second idle and 32 playing (by its
        name); a run more than 3 times above or below that (input from outside the script
        started or stopped the transport, or the window was covered) is marked `off_shape` and
        collect exits 3, for scripts/measure.sh to run it once more.

    measure.py report RESULTS_JSON [BEFORE_JSON]
        Prints the results as a Markdown table; with BEFORE_JSON, a before/after table with the
        change of each figure.

    measure.py compare --before RESULTS_JSON.. --after RESULTS_JSON.. [--expect SIDE:SCENARIO=LOW-HIGH]..
        Prints the before/after tables from several runs of each build
        (the results.json of each, in the order they were taken): every figure the mean of the
        first two runs per scenario that ran undisturbed, with its change (and, for CPU and
        context switches, the change in per cent); then the runs set aside, and how far the two
        runs kept differed in process CPU. A run is set aside when its window's updates per
        second are not what its scenario makes (2 a second idle, 32 playing; --expect gives
        another range for one scenario of one side, `before` or `after`) or it drew fewer than
        nine tenths of that many frames: input from outside the script started or stopped the
        transport, or the window was covered.
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

PPQ = 96
BAR = 4 * PPQ
STEP = PPQ // 4
IMPULSE = str(Path(__file__).resolve().parent.parent / "tests/fixtures/impulse.wav")
AUDIO_THREADS = {"pw_out", "dawsome-parked"}


# fixtures

def channel(cid, name, insert=None):
    entry = {"id": cid, "name": name, "kind": "sampler", "sample": IMPULSE,
             "volume": 1.0, "pan": 0.0, "mute": False}
    if insert is not None:
        entry["insert"] = insert
    return entry


def tracks(count):
    return [{"id": i, "name": f"Track {i}", "mute": False} for i in range(1, count + 1)]


def project(channels, patterns, playlist, mixer=None):
    doc = {"format_version": 2, "sample_rate": 48000, "tempo_mbpm": 120000, "ppq": PPQ,
           "next_channel_id": len(channels) + 1, "channels": channels, "patterns": patterns,
           "playlist": playlist}
    if mixer is not None:
        doc["mixer"] = mixer
    return doc


def five_hundred_tracks():
    notes = [{"channel": 1, "tick": beat * PPQ, "length": 0, "velocity": 1.0}
             for beat in range(4)]
    clips = [{"id": i + 1, "track": 10 * i + 1, "start": (i % 16) * BAR, "length": BAR,
              "kind": "pattern", "pattern": 1} for i in range(50)]
    playlist = {"next_track_id": 501, "next_clip_id": 51, "tracks": tracks(500),
                "clips": clips}
    pattern = {"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": notes}
    return project([channel(1, "kick")], [pattern], playlist)


def session():
    channels = [channel(i, f"Channel {i}", insert=i) for i in range(1, 9)]
    # 64 steps over 4 bars, each on the channel its step number picks; channel 1's notes climb
    # so the piano roll has a line to draw.
    notes = [{"channel": step % 8 + 1, "tick": step * STEP, "length": STEP, "velocity": 0.8,
              "key": 60 + (step * 5) % 24 if step % 8 == 0 else 60}
             for step in range(64)]
    effects = {1: ("dawsome.delay", {"feedback": 0.4, "mix": 0.5, "sync_steps": 3.0}),
               2: ("dawsome.eq3", {}),
               3: ("dawsome.reverb", {"mix": 0.3}),
               4: ("dawsome.pressure4", {}),
               5: ("dawsome.density", {})}
    inserts = [{"id": 0, "name": "Master", "fader": 0.8, "pan": 0.0, "mute": False,
                "routes": [], "effects": []}]
    for i in range(1, 9):
        chain = []
        if i in effects:
            kind, params = effects[i]
            chain.append({"id": i, "slot": 0, "processor": {"id": kind, "params": params}})
        inserts.append({"id": i, "name": f"Insert {i}", "fader": 0.8, "pan": 0.0,
                        "mute": False, "routes": [{"to": 0, "amount": 1.0}],
                        "effects": chain})
    mixer = {"next_effect_id": len(effects) + 1, "solo": [], "inserts": inserts}
    clips = [{"id": i + 1, "track": i % 16 + 1, "start": (i // 16) * 4 * BAR,
              "length": 4 * BAR, "kind": "pattern", "pattern": 1} for i in range(32)]
    playlist = {"next_track_id": 17, "next_clip_id": 33, "tracks": tracks(16), "clips": clips}
    notes.sort(key=lambda note: (note["tick"], note["channel"], note["key"]))
    pattern = {"id": 1, "name": "Pattern 1", "length_bars": 4, "notes": notes}
    return project(channels, [pattern], playlist, mixer)


def write_fixtures(out):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    for name, doc in (("g_500_tracks.json", five_hundred_tracks()),
                      ("h_session.json", session())):
        (out / name).write_text(json.dumps(doc) + "\n", encoding="utf-8")


# sample

def threads(pid):
    """{tid: (comm, cpu_ns, context_switches)} for the live threads of pid."""
    found = {}
    base = f"/proc/{pid}/task"
    try:
        tids = os.listdir(base)
    except FileNotFoundError:
        return found
    for tid in tids:
        try:
            with open(f"{base}/{tid}/comm", encoding="utf-8") as f:
                comm = f.read().strip()
            with open(f"{base}/{tid}/schedstat", encoding="utf-8") as f:
                cpu_ns = int(f.read().split()[0])
            switches = 0
            with open(f"{base}/{tid}/status", encoding="utf-8") as f:
                for line in f:
                    if line.startswith(("voluntary_ctxt_switches", "nonvoluntary_ctxt_switches")):
                        switches += int(line.split()[1])
        except (FileNotFoundError, ProcessLookupError, ValueError):
            continue
        found[int(tid)] = (comm, cpu_ns, switches)
    return found


def rss_mib(pid):
    """(RSS, its anonymous part) in MiB, (0, 0) once the process is gone."""
    found = {}
    try:
        with open(f"/proc/{pid}/status", encoding="utf-8") as f:
            for line in f:
                key, _, value = line.partition(":")
                if key in ("VmRSS", "RssAnon"):
                    found[key] = int(value.split()[0]) / 1024
    except FileNotFoundError:
        pass
    return found.get("VmRSS", 0.0), found.get("RssAnon", 0.0)


def thread_group(pid, tid, comm):
    if tid == pid:
        return "ui"
    return "audio" if comm in AUDIO_THREADS else "other"


def gpu(pid, lines):
    """(mean SM %, samples, peak VRAM MiB) for pid from `nvidia-smi pmon -s um` output."""
    sm, fb = [], []
    for line in lines:
        cols = line.split()
        # gpu pid type sm mem enc dec jpg ofa fb ccpm command
        if len(cols) < 10 or cols[0].startswith("#") or cols[1] != str(pid):
            continue
        sm.append(0 if cols[3] == "-" else int(cols[3]))
        if cols[9] != "-":
            fb.append(int(cols[9]))
    mean = sum(sm) / len(sm) if sm else None
    return mean, len(sm), (max(fb) if fb else None)


def sample(pid, warmup, duration):
    time.sleep(warmup)
    pmon = None
    if shutil.which("nvidia-smi"):
        pmon = subprocess.Popen(
            ["nvidia-smi", "pmon", "-s", "um", "-d", "1", "-c", str(duration)],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    start, began = threads(pid), time.monotonic_ns()
    peak_rss, peak_anon = rss_mib(pid)
    for _ in range(duration):
        time.sleep(1)
        rss, anon = rss_mib(pid)
        peak_rss, peak_anon = max(peak_rss, rss), max(peak_anon, anon)
    end, ended = threads(pid), time.monotonic_ns()
    wall = ended - began
    cpu = {"ui": 0, "audio": 0, "other": 0}
    switches = {"ui": 0, "audio": 0, "other": 0}
    by_thread = {}
    for tid, (comm, cpu_ns, count) in end.items():
        _, cpu_was, count_was = start.get(tid, (comm, 0, 0))
        name = thread_group(pid, tid, comm)
        cpu[name] += cpu_ns - cpu_was
        switches[name] += count - count_was
        named = by_thread.setdefault(comm, [0, 0, 0])
        named[0] += 1
        named[1] += cpu_ns - cpu_was
        named[2] += count - count_was
    gpu_lines = []
    if pmon is not None:
        try:
            out, _ = pmon.communicate(timeout=10)
            gpu_lines = out.splitlines()
        except subprocess.TimeoutExpired:
            pmon.kill()
            pmon.communicate()
    sm, samples, vram = gpu(pid, gpu_lines)
    seconds = wall / 1e9
    cpu_pct = {name: 100 * ns / wall for name, ns in cpu.items()}
    cpu_pct["total"] = sum(cpu_pct.values())
    per_s = {name: count / seconds for name, count in switches.items()}
    per_s["total"] = sum(per_s.values())
    threads_shown = {
        comm: {"threads": count, "cpu_pct": round(100 * ns / wall, 3),
               "switches_per_s": round(switched / seconds, 1)}
        for comm, (count, ns, switched) in sorted(by_thread.items())}
    return {"alive": bool(end), "threads": len(end), "cpu_pct": cpu_pct,
            "switches_per_s": per_s, "rss_mib": peak_rss, "rss_anon_mib": peak_anon,
            "gpu_sm_pct": sm,
            "gpu_samples": samples, "vram_mib": vram, "by_thread": threads_shown}


# collect

# Updates per second a scenario's window makes undisturbed: the 500 ms health check while idle,
# the 30 Hz playhead timer and the health check while playing; and how many times above or
# below that a run may be before it is taken for a disturbed one. Why: an idle run whose
# window took input meant for another was measured as playing (2026-10-22).
IDLE_RATE = 2.0
PLAYING_RATE = 32.0
OFF_BY = 3.0


def expected_updates(scenario):
    """The window updates a second `scenario` makes undisturbed: a playing one's, by its name,
    else an idle one's."""
    return PLAYING_RATE if "playing" in scenario else IDLE_RATE


def off_shape(scenario, window):
    """Whether a run's window (its --stats line, None when it printed none) updated more than
    OFF_BY times above or below what `scenario` makes."""
    if window is None:
        return True
    ratio = window["updates_per_s"] / expected_updates(scenario)
    return not 1 / OFF_BY <= ratio <= OFF_BY


CALLBACKS = re.compile(
    r"audio callbacks: (\d+), mean ([\d.]+) µs, worst ([\d.]+) µs \(([\d.]+) % of its "
    r"buffer's time\), buffers up to (\d+) frames at (\d+) Hz")


def callbacks(output):
    """The --stats line of the app's output, or None when no stream played."""
    found = CALLBACKS.search(output)
    if found is None:
        return None
    count, mean, worst, load, frames, rate = found.groups()
    return {"count": int(count), "mean_us": float(mean), "worst_us": float(worst),
            "worst_load_pct": float(load), "frames": int(frames), "rate": int(rate)}


WINDOW = re.compile(
    r"window: (\d+) updates \(([\d.]+)/s\), (\d+) frames drawn \(([\d.]+)/s\) in ([\d.]+) s")


def window(output):
    """The window line of the app's --stats output."""
    found = WINDOW.search(output)
    if found is None:
        return None
    updates, updates_per_s, frames, frames_per_s, seconds = found.groups()
    return {"updates": int(updates), "updates_per_s": float(updates_per_s),
            "frames": int(frames), "frames_per_s": float(frames_per_s),
            "seconds": float(seconds)}


def collect(results_path, scenario, label, sample_path, output_path):
    """Adds the run to the results; returns whether it is off its scenario's shape."""
    results_path = Path(results_path)
    results = (json.loads(results_path.read_text(encoding="utf-8"))
               if results_path.exists() else [])
    entry = json.loads(Path(sample_path).read_text(encoding="utf-8"))
    output = Path(output_path).read_text(encoding="utf-8")
    shown = window(output)
    off = off_shape(scenario, shown)
    entry.update(scenario=scenario, label=label, callbacks=callbacks(output), window=shown,
                 expected_updates_per_s=expected_updates(scenario), off_shape=off)
    results = [old for old in results if old["scenario"] != scenario] + [entry]
    results_path.write_text(json.dumps(results, indent=1) + "\n", encoding="utf-8")
    return off


# report

COLUMNS = [
    ("CPU %", lambda r: r["cpu_pct"]["total"], 2),
    ("UI %", lambda r: r["cpu_pct"]["ui"], 2),
    ("audio %", lambda r: r["cpu_pct"]["audio"], 2),
    ("other %", lambda r: r["cpu_pct"]["other"], 2),
    ("RSS MiB", lambda r: r["rss_mib"], 0),
    ("anon MiB", lambda r: r.get("rss_anon_mib"), 0),
    ("GPU SM %", lambda r: r["gpu_sm_pct"], 1),
    ("VRAM MiB", lambda r: r["vram_mib"], 0),
    ("switches/s", lambda r: r["switches_per_s"]["total"], 0),
    ("frames/s", lambda r: (r.get("window") or {}).get("frames_per_s"), 1),
    ("worst callback µs", lambda r: (r.get("callbacks") or {}).get("worst_us"), 0),
]


def shown(value, places):
    return "–" if value is None else f"{value:.{places}f}"


def row_label(entry):
    """The scenario's label, and, for a run off its shape, its rate and the scenario's."""
    if not entry.get("off_shape"):
        return entry["label"]
    rate = (entry.get("window") or {}).get("updates_per_s")
    return (f"{entry['label']} (off: {shown(rate, 1)} updates/s, "
            f"{entry['expected_updates_per_s']:g} expected)")


def report(results, before=None):
    rows = ["| Scenario | " + " | ".join(name for name, _, _ in COLUMNS) + " |",
            "|-|" + "-|" * len(COLUMNS)]
    was = {entry["scenario"]: entry for entry in before or []}
    for entry in results:
        cells = []
        for _, read, places in COLUMNS:
            now = read(entry)
            if entry["scenario"] not in was:
                cells.append(shown(now, places))
                continue
            then = read(was[entry["scenario"]])
            if now is None or then is None:
                cells.append(f"{shown(then, places)} → {shown(now, places)}")
            else:
                cells.append(f"{shown(then, places)} → {shown(now, places)} "
                             f"({now - then:+.{places}f})")
        rows.append(f"| {row_label(entry)} | " + " | ".join(cells) + " |")
    return "\n".join(rows)


# compare

# Updates per second a scenario's window makes undisturbed: the 500 ms health check while idle,
# the 30 Hz playhead timer and the health check while playing.
IDLE_UPDATES = (1.5, 3.5)
PLAYING_UPDATES = (30.0, 35.0)

CPU_COLUMNS = [
    ("CPU % (process)", lambda r: r["cpu_pct"]["total"], 2, True),
    ("UI thread %", lambda r: r["cpu_pct"]["ui"], 2, True),
    ("audio thread %", lambda r: r["cpu_pct"]["audio"], 2, False),
    ("other threads %", lambda r: r["cpu_pct"]["other"], 2, False),
    ("frames/s", lambda r: (r.get("window") or {}).get("frames_per_s"), 1, False),
]
MEMORY_COLUMNS = [
    ("RSS MiB", lambda r: r["rss_mib"], 0, False),
    ("anonymous MiB", lambda r: r.get("rss_anon_mib"), 0, False),
    ("threads", lambda r: r["threads"], 0, False),
    ("context switches/s", lambda r: r["switches_per_s"]["total"], 0, True),
    ("worst callback µs", lambda r: (r.get("callbacks") or {}).get("worst_us"), 0, False),
]
GPU_COLUMNS = [
    ("GPU SM %", lambda r: r["gpu_sm_pct"], 1, False),
    ("VRAM MiB", lambda r: r["vram_mib"], 0, False),
]
TABLES = [("CPU and frames", CPU_COLUMNS),
          ("Memory, threads, wake-ups and audio callbacks", MEMORY_COLUMNS),
          ("GPU, per process", GPU_COLUMNS)]


def expectation(text):
    """`before:e_p7_clap_playing=88-100` as (("before", "e_p7_clap_playing"), (88.0, 100.0))."""
    try:
        key, _, span = text.partition("=")
        side, _, scenario = key.partition(":")
        low, _, high = span.partition("-")
        if side not in ("before", "after") or not scenario:
            raise ValueError(text)
        return (side, scenario), (float(low), float(high))
    except ValueError:
        raise argparse.ArgumentTypeError(
            f"{text!r} is not SIDE:SCENARIO=LOW-HIGH, SIDE before or after")


def undisturbed(side, runs, expected):
    """The first two entries per scenario, over `runs` in order, whose window updated and drew
    at the rate the scenario makes; and the (run, scenario, updates/s, frames/s) set aside."""
    kept, set_aside = {}, []
    for number, run in enumerate(runs, start=1):
        for entry in run:
            scenario = entry["scenario"]
            default = PLAYING_UPDATES if "playing" in scenario else IDLE_UPDATES
            low, high = expected.get((side, scenario), default)
            window = entry.get("window") or {}
            updates = window.get("updates_per_s", 0.0)
            frames = window.get("frames_per_s", 0.0)
            if low <= updates <= high and frames >= 0.9 * low:
                chosen = kept.setdefault(scenario, [])
                if len(chosen) < 2:
                    chosen.append(entry)
            else:
                set_aside.append((number, scenario, updates, frames))
    return kept, set_aside


def mean_of(entries, read):
    values = [value for value in map(read, entries) if value is not None]
    return sum(values) / len(values) if values else None


def changed(before, after, read, places, percent):
    then, now = mean_of(before, read), mean_of(after, read)
    if then is None or now is None:
        return "–"
    delta = now - then
    share = f", {100 * delta / then:+.0f} %" if percent and then else ""
    return f"{then:.{places}f} → {now:.{places}f} ({delta:+.{places}f}{share})"


def compare(before_runs, after_runs, expected):
    before, set_aside_before = undisturbed("before", before_runs, expected)
    after, set_aside_after = undisturbed("after", after_runs, expected)
    first = before_runs[0]
    order = [entry["scenario"] for entry in first]
    labels = {entry["scenario"]: entry["label"] for entry in first}
    lines = []
    for title, columns in TABLES:
        lines += [f"### {title}", "",
                  "| Scenario | " + " | ".join(name for name, *_ in columns) + " |",
                  "|-|" + "-|" * len(columns)]
        for scenario in order:
            cells = [changed(before.get(scenario, []), after.get(scenario, []), read, places,
                             percent)
                     for _, read, places, percent in columns]
            lines.append(f"| {labels[scenario]} | " + " | ".join(cells) + " |")
        lines.append("")
    lines.append("Runs set aside (side, run, scenario, updates/s, frames/s):")
    for side, set_aside in (("before", set_aside_before), ("after", set_aside_after)):
        lines += [f"- {side} {number} {scenario}: {updates:.1f}, {frames:.1f}"
                  for number, scenario, updates, frames in set_aside]
    lines += ["", "Spread of process CPU % between the two runs kept (before, after):"]
    total = CPU_COLUMNS[0][1]
    for scenario in order:
        spreads = []
        for kept in (before, after):
            values = [total(entry) for entry in kept.get(scenario, [])]
            spreads.append(f"{max(values) - min(values):.2f}" if len(values) == 2 else "–")
        lines.append(f"- {scenario}: {spreads[0]}, {spreads[1]}")
    return "\n".join(lines)


def compare_main(argv):
    parser = argparse.ArgumentParser(prog="measure.py compare")
    parser.add_argument("--before", nargs="+", required=True, metavar="RESULTS_JSON")
    parser.add_argument("--after", nargs="+", required=True, metavar="RESULTS_JSON")
    parser.add_argument("--expect", type=expectation, action="append", default=[],
                        metavar="SIDE:SCENARIO=LOW-HIGH")
    args = parser.parse_args(argv)
    read = lambda path: json.loads(Path(path).read_text(encoding="utf-8"))
    print(compare([read(path) for path in args.before], [read(path) for path in args.after],
                  dict(args.expect)))


def main():
    command = sys.argv[1] if len(sys.argv) > 1 else ""
    if command == "fixtures" and len(sys.argv) == 3:
        write_fixtures(sys.argv[2])
    elif command == "sample" and len(sys.argv) == 5:
        print(json.dumps(sample(int(sys.argv[2]), float(sys.argv[3]), int(sys.argv[4]))))
    elif command == "collect" and len(sys.argv) == 7:
        if collect(*sys.argv[2:]):
            sys.exit(3)
    elif command == "report" and len(sys.argv) in (3, 4):
        results = json.loads(Path(sys.argv[2]).read_text(encoding="utf-8"))
        before = (json.loads(Path(sys.argv[3]).read_text(encoding="utf-8"))
                  if len(sys.argv) == 4 else None)
        print(report(results, before))
    elif command == "compare":
        compare_main(sys.argv[2:])
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
