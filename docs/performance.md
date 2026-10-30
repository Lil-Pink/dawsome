# Performance

What Dawsome costs while it sits open on a desktop, idle or playing: how it is measured, the
figures before and after the work to lower them, what moved and why, and what is left. The figures
come from one host and were taken on 2026-10-21, on that day's build: it had none of MIDI out,
VST3 hosting, the master limiter or the parameter window, so a newer build's figures may differ.
`scripts/measure.sh` takes them again on any build and any host.

## Host

| | |
|-|-|
| CPU | AMD Ryzen 9 7900X, 12 cores / 24 threads (integrated Radeon graphics also present) |
| GPU | NVIDIA GeForce RTX 4080 SUPER, driver 615.78.08 |
| System | Linux 7.2 (CachyOS), KDE Plasma 6.8 on Wayland, display scale 1.7 |
| Audio | PipeWire 1.6.9: 48 kHz default, 44.1 kHz allowed, default quantum 1024 frames |
| Build | Rust 1.93.1, release profile |

The app asks the device for 512-frame buffers. A 44.1 kHz project ran the device at 44.1 kHz
(buffers of up to 498 frames), a 48 kHz project at 48 kHz (512 frames).

## Method

`scripts/measure.sh [out-dir] [scenario..]` builds the release binaries, launches the app once
so the graphics drivers' shader caches are filled, then runs each scenario in a real window with
the scenario's flags plus `--midi-ports none --stats --quit-after`. After 4 s it samples the
process for 20 s (`scripts/measure.py sample`):

- **CPU %**: time on a CPU per thread (`/proc/<pid>/task/<tid>/schedstat`) as a percentage of
  one core, for the whole process and split into the UI thread (the main thread), the audio
  thread (PipeWire's `pw_out` loop thread, where the output callback runs) and the other
  threads (toolkit, graphics driver and worker threads).
- **RSS**: the peak resident set (`VmRSS`), and its anonymous part (`RssAnon`): the process's
  own memory without the mapped libraries and driver files.
- **Context switches/s**: voluntary plus involuntary, all threads.
- **Frames/s**: the frames the window drew over the run (`--stats`). Every message the app
  takes rebuilds the view and draws a frame, so this is how often the window wakes, and the
  most reliable measure of the GPU work the app causes.
- **Worst callback**: the longest single audio callback over the run, render time only
  (`--stats`).
- **GPU**: this process's SM utilization and frame-buffer memory from
  `nvidia-smi pmon -s um -d 1`. Both are per-process figures; whole-GPU utilization is not
  used. They are coarse (whole percent, one sample a second, no sample in many seconds).
- **Engine time per block**: `cargo run --release -p dawsome --example block_time --
  <project>` plays a project through `Player::process` in 512-frame blocks back to back, times
  each block with `Instant` (the CPU's time-stamp counter through the vDSO), and prints the
  mean, the 99.9th percentile and the worst against 10.67 ms, the time 512 frames play at
  48 kHz.

The output went to an unmonitored PipeWire null sink (`MEASURE_SINK`, passed to the app as
`PIPEWIRE_NODE`): the same graph and the same pacing as a speaker, with nothing audible. Every
run is scripted, so it writes no settings, recent files or autosaves, lists and opens no MIDI
port, and its per-user folders point into the out dir. Its window's application id is
`dawsome-test` (`DAWSOME_APP_ID`, which `measure.sh` sets), so a window rule of the desktop can
open it on a screen of its own without taking focus, where input meant for other windows does
not reach it. The plugin scenarios load the release
build of the test plugin, scanned into the out dir.

Each figure below is the mean of two runs, taken alternately: before, after, before, after.
BEFORE is the build with the measuring tools and none of the three changes under "What moved
and why", built in a copy of its own; AFTER is the build with all three.

**Conditions.** From 17:25 local time on 2026-10-21 another GPU-heavy process held 11.4 GB
of the GPU's 16 GB and used the GPU and about two CPU cores on and off. Every figure in the tables was taken under that shared condition, BEFORE and AFTER alike,
between 17:54 and 18:18. A first BEFORE run at 17:17, with the GPU to itself, agreed with the
shared-condition BEFORE figures within 0.25 percentage points of CPU in eight scenarios; in
(i) and (j) the run with the GPU to itself was 0.5 to 0.6 points higher, so the shared
condition did not inflate the CPU figures. Memory and context switches agreed, except the
first scenario of that run, which carried a cold shader cache (see below). The per-process GPU
samples did not repeat even between identical runs (see GPU below).

The app windows open on the desktop in use, so a run can be disturbed from outside the script.
Each run's updates and frames per second were checked against what its scenario makes (2 a
second idle, 32 playing, plus 62 for a loaded plugin before the change). Four of the 46 runs
did not match, most likely from keyboard or pointer input or another window covering the app's:
a playing run that stopped, two idle runs that played, and an idle run that drew a third of its
frames. Those four were set aside and their scenarios run again, so every figure is the mean of
two runs that matched. `measure.sh` now does the first part itself: a run whose window updated
more than 3 times above or below its scenario's rate is run once more, and when it is off again
it stays in `results.json` marked `off_shape`, its row in `results.md` naming its rate
(`measure.py collect`).

The tables below are what `scripts/measure.py compare` prints for these runs: each build's
`results.json` files in the order they were taken, and the BEFORE build's plugin scenarios
expected at their higher rate (the window timer that build ran for a loaded plugin):

```
python3 scripts/measure.py compare \
    --before before-1/results.json before-2/results.json before-3/results.json \
    --after after-1/results.json after-2/results.json after-3/results.json \
    --expect before:e_p7_clap_playing=88-100 --expect before:i_p7_clap_idle=60-70
```

It keeps the first two runs per scenario whose window ran at its scenario's rate, prints the
three tables with each figure's change, then names the runs it set aside and how far the two
runs kept differed in process CPU. The third run of each build holds only the scenarios run
again.

## Scenarios

| | Project | Panels and window | Transport |
|-|-|-|-|
| (a) | empty document | rack, 1000×560 | stopped |
| (b) | `tests/fixtures/f1_four_on_floor.json` | rack, 1000×560 | stopped |
| (c) | F1 | rack, 1000×560 | playing |
| (d) | `tests/fixtures/p5_delay_sync.json` (16 inserts, a tempo-synced delay) | rack and mixer, 1000×560 | playing |
| (e) | `tests/fixtures/p7_sine.json` (a CLAP instrument, the test sine) | rack, 1000×560 | playing |
| (f) | `tests/fixtures/p8_ui.json` (an audio clip and an automation clip), song mode | playlist, 1000×560 | playing |
| (g) | generated: 500 tracks (the cap) holding 50 pattern clips | playlist, 1000×560 | stopped |
| (h) | generated session: 8 channels on 8 inserts, a delay, EQ, reverb, Pressure4 and Density, a 4-bar pattern of 64 notes, 16 tracks with 32 clips | playlist, piano roll and mixer, 1700×952 | stopped |
| (i) | as (e) | rack, 1000×560 | stopped |
| (j) | as (h), song mode | as (h) | playing |

Sizes are logical pixels; at scale 1.7 a 1000×560 window is 1700×952 device pixels. The two
generated projects come from `scripts/measure.py fixtures`.

## Results: BEFORE → AFTER (change)

Two runs of one scenario on one build differed by up to 0.38 percentage points of process CPU
(0.2 or less in most), so changes smaller than that, such as every idle row without a plugin,
are noise. The changes that hold are the plugin rows (e, i), the frame rates, the thread
count, resident memory, and the UI thread while playing (d, f).

### CPU and frames

| Scenario | CPU % (process) | UI thread % | audio thread % | other threads % | frames/s |
|-|-|-|-|-|-|
| (a) empty document, idle | 0.46 → 0.38 (-0.08, -17 %) | 0.10 → 0.09 (-0.01, -8 %) | 0.20 → 0.17 (-0.03) | 0.15 → 0.12 (-0.04) | 2.0 → 2.0 (+0.0) |
| (b) F1, idle | 0.43 → 0.46 (+0.04, +8 %) | 0.10 → 0.11 (+0.00, +3 %) | 0.19 → 0.21 (+0.02) | 0.14 → 0.15 (+0.01) | 2.1 → 2.2 (+0.1) |
| (c) F1, playing | 1.88 → 1.70 (-0.18, -10 %) | 1.21 → 1.09 (-0.12, -10 %) | 0.12 → 0.14 (+0.03) | 0.56 → 0.47 (-0.08) | 32.0 → 32.2 (+0.2) |
| (d) p5_delay_sync, mixer open, playing | 2.17 → 1.92 (-0.25, -11 %) | 1.47 → 1.26 (-0.21, -14 %) | 0.17 → 0.17 (+0.01) | 0.53 → 0.48 (-0.04) | 32.5 → 32.1 (-0.3) |
| (e) p7_sine (CLAP), playing | 3.53 → 1.79 (-1.74, -49 %) | 2.61 → 1.12 (-1.49, -57 %) | 0.13 → 0.14 (+0.01) | 0.79 → 0.54 (-0.25) | 90.1 → 32.0 (-58.0) |
| (f) p8 audio clip + automation, song, playing | 2.05 → 1.88 (-0.17, -8 %) | 1.34 → 1.20 (-0.14, -11 %) | 0.18 → 0.14 (-0.04) | 0.53 → 0.55 (+0.02) | 31.9 → 32.0 (+0.1) |
| (g) 500 tracks, 50 clips, playlist, idle | 0.39 → 0.35 (-0.03, -9 %) | 0.12 → 0.13 (+0.01, +5 %) | 0.13 → 0.09 (-0.04) | 0.14 → 0.13 (-0.01) | 2.2 → 2.6 (+0.3) |
| (h) mixer + playlist + piano roll, 1700×952, idle | 0.39 → 0.35 (-0.04, -11 %) | 0.13 → 0.12 (-0.01, -5 %) | 0.13 → 0.10 (-0.03) | 0.14 → 0.13 (-0.01) | 2.0 → 2.0 (+0.0) |
| (i) p7_sine (CLAP), idle | 2.87 → 0.34 (-2.53, -88 %) | 2.07 → 0.09 (-1.98, -96 %) | 0.13 → 0.09 (-0.03) | 0.67 → 0.16 (-0.51) | 63.4 → 2.0 (-61.4) |
| (j) as (h), song, playing | 2.47 → 2.58 (+0.11, +5 %) | 1.39 → 1.40 (+0.01, +1 %) | 0.60 → 0.66 (+0.06) | 0.47 → 0.52 (+0.05) | 31.9 → 32.1 (+0.2) |

### Memory, threads, wake-ups and audio callbacks

| Scenario | RSS MiB | anonymous MiB | threads | context switches/s | worst callback µs |
|-|-|-|-|-|-|
| (a) empty document, idle | 201 → 196 (-5) | 31 → 30 (-1) | 40 → 14 (-26) | 375 → 368 (-7, -2 %) | 53 → 56 (+3) |
| (b) F1, idle | 202 → 197 (-5) | 32 → 30 (-1) | 41 → 15 (-26) | 371 → 389 (+18, +5 %) | 56 → 51 (-5) |
| (c) F1, playing | 202 → 198 (-5) | 32 → 31 (-1) | 41 → 15 (-26) | 856 → 855 (-1, -0 %) | 10 → 13 (+3) |
| (d) p5_delay_sync, mixer open, playing | 206 → 200 (-5) | 35 → 34 (-1) | 41 → 15 (-26) | 868 → 851 (-16, -2 %) | 33 → 25 (-8) |
| (e) p7_sine (CLAP), playing | 202 → 198 (-5) | 32 → 31 (-1) | 40 → 14 (-26) | 1455 → 908 (-547, -38 %) | 20 → 26 (+6) |
| (f) p8 audio clip + automation, song, playing | 203 → 198 (-5) | 33 → 32 (-1) | 41 → 15 (-26) | 869 → 848 (-20, -2 %) | 31 → 18 (-13) |
| (g) 500 tracks, 50 clips, playlist, idle | 203 → 198 (-5) | 32 → 31 (-1) | 41 → 15 (-26) | 379 → 402 (+23, +6 %) | 53 → 33 (-20) |
| (h) mixer + playlist + piano roll, 1700×952, idle | 204 → 200 (-5) | 33 → 32 (-1) | 41 → 15 (-26) | 374 → 365 (-8, -2 %) | 56 → 18 (-38) |
| (i) p7_sine (CLAP), idle | 202 → 197 (-5) | 32 → 30 (-1) | 40 → 14 (-26) | 1211 → 446 (-765, -63 %) | 55 → 22 (-33) |
| (j) as (h), song, playing | 206 → 202 (-5) | 36 → 35 (-1) | 41 → 15 (-26) | 878 → 844 (-34, -4 %) | 178 → 175 (-3) |

### GPU, per process

| Scenario | GPU SM % | VRAM MiB |
|-|-|-|
| (a) empty document, idle | 0.0 → 0.4 (+0.4) | 77 → 61 (-16) |
| (b) F1, idle | 0.0 → 0.0 (+0.0) | 61 → 61 (+0) |
| (c) F1, playing | 0.3 → 1.8 (+1.5) | 77 → 77 (+0) |
| (d) p5_delay_sync, mixer open, playing | 1.3 → 1.8 (+0.5) | 61 → 77 (+16) |
| (e) p7_sine (CLAP), playing | 9.6 → 0.6 (-9.0) | 61 → 77 (+16) |
| (f) p8 audio clip + automation, song, playing | 2.5 → 2.9 (+0.4) | 61 → 77 (+16) |
| (g) 500 tracks, 50 clips, playlist, idle | 1.7 → 0.4 (-1.3) | 61 → 77 (+16) |
| (h) mixer + playlist + piano roll, 1700×952, idle | 0.0 → 0.0 (+0.0) | 160 → 160 (+0) |
| (i) p7_sine (CLAP), idle | 2.5 → 0.0 (-2.5) | 77 → 61 (-16) |
| (j) as (h), song, playing | 3.0 → 7.8 (+4.8) | 118 → 118 (+0) |

The per-process GPU samples did not repeat between identical runs (the same build and
scenario read 0 % in one run and 10 % in another before the GPU was shared), so single cells
are not evidence of a change. The one consistent signal is the plugin scenarios, where the GPU
share follows the frame rate. VRAM moved between 45, 61 and 77 MiB for the same scenario from
run to run, at both window sizes, independently of the build: the driver allocates the
window's buffers in steps. The larger window (h, j: 2890×1618 device pixels) holds 118–160
MiB. Frames/s is the figure to follow for the GPU work the app causes.

### Engine time per 512-frame block

| Project | Build | Mean µs | p99.9 µs | Worst µs | Headroom (budget ÷ worst) |
|-|-|-|-|-|-|
| `p5_delay_sync` (d) | BEFORE | 2.1 | 6.0–6.4 | 67.6 | ×158 |
| | AFTER | 2.0–2.1 | 5.6–5.7 | 16.7 | ×638 |
| `p7_sine` (e) | BEFORE | 2.4 | 7.6–7.9 | 22.3 | ×479 |
| | AFTER | 2.2 | 7.5–7.8 | 21.6 | ×494 |
| generated session (h, j), song | BEFORE | 46.4–46.7 | 60.2–66.6 | 82.0 | ×130 |
| | AFTER | 46.0–46.1 | 68.4–71.7 | 269.9 | ×40 |

Two runs of 60 s of audio per project and build; ranges are over the two runs, the worst is the
worse of the two. The goal for (d) and (e), the 512-frame budget with three times headroom
(a worst block under 3.56 ms), is met by more than two orders of magnitude. The engine's code
did not change in that work: the differences between the builds are within the run-to-run
spread, and the single slow blocks (67.6 µs, 269.9 µs) are preemptions of the timing thread,
which runs at normal priority.

The worst block in a back-to-back run includes the times the operating system preempted the
thread; the live callback runs at real-time priority and its worst, in the tables above, stayed
under 0.24 ms (236 µs in (j), 2.2 % of its buffer's time), before and after alike.

## What moved and why

1. **A loaded plugin no longer wakes the window 62 times a second** (scenarios e and i). The
   plugins' main-thread work ran on a 16 ms window timer for as long as any plugin instance
   lived, and every timer message rebuilds the view and draws a frame. Plugins rarely need
   that: their requests (a callback, a restart, a changed state, a failure, a window closing or
   resizing) are flags they set, some from the audio thread. Each request now also rings one
   doorbell per host, an atomic flag; the window watches it off its own thread every 16 ms and
   takes a plugin tick only when it rang. The 16 ms window timer still runs while work comes
   due by the clock (a plugin timer or file descriptor, an open plugin window, a notice
   waiting, an instance on its way out, a state capture or a scan), and while a request rang
   but has not been served: the watch clears the bell as it looks, so a tick the window could
   not take (a dialog was up) leaves the request to the timer, served by the first tick after
   the dialog closes. Idle with a plugin loaded: 63 → 2 frames/s, CPU 2.87 % → 0.34 %,
   context switches −63 %. Playing it: 90 → 32 frames/s, CPU −49 %.
2. **Fewer threads.** The toolkit ran its tasks on a runtime with one worker per core (24
   here), and its desktop theme detection kept a D-Bus connection with four threads of its own
   for a theme the app never uses (it is always dark). The app now runs the toolkit on a
   two-worker runtime and leaves theme detection out: 41 → 15 threads, about 5 MiB less
   resident memory, nothing drawn differently. The work per timer tick did not shrink with the
   pool (each tick still wakes a worker and each window event wakes each event listener), so
   context switches stayed within noise outside the plugin scenarios.
3. **Release profile: one optimization unit (fat LTO, one codegen unit).** Most of the UI
   thread's time per frame is inside the toolkit and renderer crates, which inlining across
   crates reaches. In the paired runs above it saved about 10 % of UI-thread CPU in (d) and
   (f), within noise in (c) and (j); those runs also carry the change in 2. An interleaved A/B
   of the profile alone (two runs per profile, the model server busy in two of them) varied
   more between its runs than the effect, so it gives no figure of its own. The binary went
   from 29 MB to 21 MB. Release builds take about two minutes instead of
   forty seconds; debug builds and the checks are unchanged. Unwinding is kept: the plugin host
   catches a panic at the plugin boundary rather than aborting the process.

None of these touch what is rendered: all 128 fixture renders (every project fixture, pattern
and song mode, 4 bars, through `dawsome render`) are byte-identical between the BEFORE and the
AFTER build; the counting-allocator tests stay green, and the playing scenarios draw the same
32 frames a second (the 30 Hz playhead and meter timer).

## Measured and left alone

- **Redraw rate.** Idle, the window wakes only for the output-health check (every 500 ms:
  2 frames/s); nothing else ticks, no animation runs, and the MIDI controller queue wakes the
  window only when a controller moves. Playing, the 30 Hz playhead timer draws 32 frames/s; it
  carries the playhead and the meters and stays.
- **Per-frame cost.** The app's own update and view take about 10 µs a frame in (j)
  (instrumented in a separate build); the rest of the 0.35–0.45 ms the UI thread spends per
  frame is the toolkit's layout, drawing and presentation. The static layers are cached and
  cleared only on a change (rack rows, playlist ruler and tracks, piano-roll grid and lane,
  mixer strips); each frame draws only the overlays (playhead, meters, outlines).
- **The audio engine.** Headroom ×40 to ×640 against the 512-frame budget at 48 kHz (table
  above), mean 46 µs a block for the 8-channel session with five effects. No change needed.
- **The idle callback.** 2–3 µs per callback while stopped. Most of the audio thread's
  0.1–0.2 % idle is the sound server's own cycle, about 94 times a second at 512 frames.
- **Input device.** Open only while Record is armed with Input as its source (its meter shows
  the level); closed otherwise.
- **MIDI port threads.** Not measured: measured runs never list or open the system's MIDI
  ports.
- **Plugin scan at start.** A fresh catalogue is read, not scanned; a stale one is scanned in
  the background at start (not measured here; scripted runs never scan).
- **Worker threads.** The preview, stretch, conversion and waveform workers run only while
  they have work; the sound loader thread sleeps on its queue.
- **Memory held by sounds.** Files up to 30 s are held decoded (32-bit float, stereo kept);
  longer files stream from disk (at most 64 streams). Waveform peaks are one minimum/maximum
  pair per 256 frames per channel, 1/64 of the decoded size. The process's own (anonymous)
  memory is 30–35 MiB in every scenario here; the rest of the ~200 MiB resident set is mapped
  libraries, most of it the graphics drivers.
- **Cold shader cache.** The first launch with empty driver shader caches holds about 46 MiB
  more resident memory (16 MiB anonymous) while the drivers compile; later launches do not.
  `measure.sh` launches once before measuring for that reason.
- **Graphics backends probed.** The renderer enumerates OpenGL as well as Vulkan adapters,
  which loads both vendors' OpenGL drivers and the integrated GPU's Vulkan driver.
  `WGPU_BACKEND=vulkan` measured 26 MiB less resident memory (203 → 177 MiB) here. The app
  does not set it: on a machine without Vulkan the toolkit would fall back to software
  rendering instead of OpenGL, and the variable would also bind plugin interfaces in the
  process that render the same way.
- **Threads outside the app.** The graphics driver's presentation thread wakes about 140 times
  a second whatever the app does (0.05 % CPU); the toolkit's Wayland clipboard thread wakes
  about five times per frame drawn.

## Known remaining costs

1. **Idle frames from the health check (2/s).** Watch the output's health and the
   dropped-note counters off the window's thread, as the plugin doorbell does, and wake the
   window only on a change: idle frames 2 → 0 a second; likely up to 0.1 % CPU.
2. **Frames that redraw nothing new while playing.** The 30 Hz timer draws 32 frames a second
   whatever is visible; with only the rack open the step playhead moves 8 times a second at
   120 BPM. A watch that wakes the window only when the visible playhead or a visible meter
   changes would cut those frames.
3. **One task wake per event listener.** Every window event, the frame event included, wakes
   each event subscription's task (keys, rescale, close); one listener for keys and window
   events would wake once.
4. **The idle call while stopped** walks all 5,010 processor slots per callback (about 2 µs);
   a list of the occupied slots would make it proportional to the processors loaded.
5. **Vulkan-only renderer selection**, 26 MiB resident, if machines without Vulkan are out of
   scope (see above).

## A sound's edits on ten minutes

Taken 2026-10-27 on the host above, release build, by `cargo run --release -p dawsome-media
--example edit_time -- <scratch folder>`: ten minutes of stereo at 48 kHz (28,800,000 frames,
230 MB as f32) through 200 ops of every kind over most of the sound (the sample editor's 10x
load). Not pass/fail; the memory while the editor is open is below.

| What | Time |
|-|-|
| The 200 ops from scratch (an undo, a redo or a project's opening: media's worker) | 3.0–3.2 s, 15–16 ms an op |
| One op appended, a new buffer each (media's path for an op past a sound it holds) | 240 ms an op, 300 ms at most |
| One op appended in place on frames held for editing (`dsp::edits::apply_in_place`) | 16 ms an op, 57 ms at most (a normalize: a pass to find the peak, one to scale) |
| The edit job: decode, 200 ops, the 32-bit float WAV written into the edited cache | 3.4–3.8 s, 230,376,068 bytes |
| The same job again: found in the cache | under 0.1 ms, nothing written |
| Of which the cache name's digest of the file's two ends (`edited::ends_digest`: 64 KiB read at each end of the 230 MB file and hashed; five runs, the file in the page cache) | 63–78 µs, 144 µs the first |

Why a new buffer costs fifteen times the op: each copies every frame twice (the frames into a
`Vec`, the `Vec` into the shared buffer), 460 MB of copying for an op that may touch 480 frames.
Media only takes that path on the UI thread for a short sound (30 s at most, 11.5 MB); a long
one goes to the worker. The editor's view buffer applies an op in place. The command line's
ten-minute test (`crates/cli/tests/sound_edits.rs`, a debug build, two ops) renders through the
edited cache in about 5 s.

The sample editor holding the same ten minutes (gen_p30's sound and its two ops, a debug build,
by the app's test `the_editor_holds_one_buffer_and_its_peaks_of_ten_minutes`, the counting
allocator's tally of what the window's opening left on the heap, 2026-10-27):

| What | Bytes |
|-|-|
| Held while it is open: the frames (230,400,000) and their peaks (1,800,000), and 108 more | 232,200,108 |
| The most while it opened: the decode, the decode's shared copy and the editor's own, briefly | 532,390,186 |
| One op more, in place (a gain over the whole sound) | under 64 KiB, the same held |

Making the sound again (an undo, a revert) holds the same transient on the worker while the
sound shown stays, so about three buffers at the most. Playback holds its own copy besides: a
sampler's sound in memory, a clip's streamed from the edited cache.

Save as new file… and Play of the whole ten minutes hand their work to a worker (the app's test
`saving_and_playing_ten_minutes_leave_the_window_at_once`, debug build, 2026-10-27): the
window's thread spends 151 µs and at most 1,982 bytes of its heap on the save's message, and
58 µs and 1,360 bytes on Play's, where a copy of the frames on it would take 230 MB (twice for
Play) and a write seconds. Meanwhile the worker holds Play's copy (230 MB, twice briefly); the
save writes from the frames the editor shows.
