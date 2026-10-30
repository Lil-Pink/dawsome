# Dawsome

The reference for Dawsome 0.1.0: every panel, gesture and setting. The README is the short introduction.

Dawsome is a pattern-based digital audio workstation for Linux and Windows. Program beats in the
channel rack, write melodies in the piano roll, arrange patterns, audio and automation in the
playlist, and mix through inserts, sends and effects. It hosts CLAP and VST® 3 plugins, records
audio and MIDI, and renders the same audio from the command line that it plays in the window. It
is free software under the GNU General Public License, version 3 or later.

Dawsome is not affiliated with Dawesome, the plugin maker, whose name sounds the same.

## Features

- **Channel rack**: sampler channels with a step grid (16 steps a bar), volume and pan, patterns
  of 1 to 32 bars; plugin instruments (CLAP or VST3) as channels. A channel that plays notes
  starts at −5 dB, so a few notes together stay clear of the master; audio
  and input channels start at 0 dB.
- **Piano roll**: draw, move, resize and delete notes, a velocity lane, selections, duplicate,
  snap and zoom, the other channels' notes shown faintly; a sample plays at the pitch of its key.
  Selection and editing: click, Shift+click and box selection, arrows to transpose and nudge,
  copy, cut and paste, and a notes menu (right-click, or the strip's Notes button) with Quantize
  (grid, strength, starts or ends), Chop, Glue, Legato, Reverse, Invert and Humanize; velocity
  painted across stems or drawn as a ramp (Alt); a scale chooser that shades the keys outside it
  and folds the rows; the notes you play live (keyboard, MIDI) shown as they sound, and a
  take's light while recording them.
- **Playlist**: tracks holding pattern clips, audio clips (trim, gain, waveform, time-stretch or
  resample with pitch shift) and automation clips (linear, hold, smooth and exponential segments);
  pattern and song mode; a loop range; the timeline's scroll bar under the tracks. A sound file
  dropped from the desktop on a track is a new audio channel and its clip there, from the bar,
  beat or step the Snap chose (several files on the tracks under it, one undo step). Clips of one
  pattern are linked (an edit changes them all) and say so with "×N"; while such a clip is
  selected, a chain right of the header's Fit unlinks every selected clip (each its own copy,
  "Pattern 1 (copy)", "(copy 2)"…) and, crossed once they play copies, links them back, one
  undo step a click; a clip's menu (Ctrl+right-click) does the same for that clip.
- **Sample editor**: a sampler's or an audio channel's sound in a window of its own (a
  double-click on an audio clip, or Edit sample… in the channel's right-click menu or at the
  foot of its instrument window): its waveform a lane a side, zoomed with Ctrl+wheel from the
  whole sound to the single frames; a selection dragged across it, its edges snapped to the zero
  crossings (Snap to zero, on until switched off), played on its own; Crop, Delete, Silence, Fade
  in, Fade out, Gain, Normalize and Reverse, over the selection or the whole sound, each one undo
  step, and Revert. The file is never changed: the project keeps the edits and every clip of the
  channel plays the file through them (a crop or a delete moves the clips after it so each plays
  what it played); Save as new file… writes the sound as it plays to a 32-bit float WAV beside
  the project's takes. Opened from a clip, the clip's part of the sound is marked on the ruler;
  a file changed on disk since its edits (another length) is said in the heading.
- **Mixer**: the master and 16 inserts (up to 500), faders, pan, mute, solo, sends (a send that
  would make a loop is refused), ten effect slots per insert. Mixer insert… in a channel's
  right-click menu (and at the end of its instrument window's Out list) gives the channel an
  insert of its own, the first free one, and opens the mixer there. Built in: a three-band EQ, a delay
  (in milliseconds or synced to the tempo), a reverb, a limiter, and the Pressure4 compressor and
  Density saturator ported from Airwindows. Plugin effects (CLAP or VST3) go in the same slots.
  A new project starts with the Limiter in the master's first slot (ceiling −0.3 dBFS, release
  100 ms), so chords and sounds played together never clip the output or an export; a project
  saved before keeps its master as it was (add the Limiter from an empty slot's menu).
- **Sound**: a sample or an instrument set's sound played off its own pitch (another key, the
  channel's tuning, the bend, the mod wheel's vibrato, a resampled audio clip) is read between
  its recorded frames by 4-point cubic Hermite interpolation, a common realtime choice in
  samplers: less of its highs is dulled or folded back below than with a straight line between
  frames. At its own pitch it plays its frames exactly as recorded.
- **Library**: one panel down the left with one search over its sections: Instruments (the
  starter kit's drums, the instrument sets, your own SFZ sets and your plugin instruments by
  maker, a VST3 one marked so), Samples (your sample folders and the shipped one-shots and
  loops, with preview) and MIDI devices (what is plugged in). Drag a sound or an instrument onto
  the rack or a playlist track, and a MIDI keyboard onto a rack channel to play that channel
  whatever is selected.
- **Instrument sets**: multisampled instruments read from SFZ files (key and velocity zones,
  loops, release, round robin), played from the piano roll, a MIDI keyboard or the computer
  keyboard like any instrument.
- **Sound settings**: a sampler's or a set's instrument window (a double-click on its name in the rack)
  sets how its notes end (One-shot or Hold), its tuning and bend range, attack and release, a
  low-pass filter, how much velocity counts and, for a sample, its direction and start. **One
  note at a time**, in the channel's right-click menu in the rack (ticked while on): a new note
  ends the one before it on that channel, like plucking a bass string, falling over the release
  (over 2 ms without one), so a fast line does not stack; the 808 basses are added with it on,
  every other sound with it off, and a channel's hint in the rack says when it is on.
- **External input**: a channel that plays an audio input live (a synth or keyboard cabled into
  your interface): pick its device and channels in its window and it plays through its volume,
  pan and mixer insert like any channel, playing or stopped (Library → Instruments → External
  input).
- **Recording**: one Record button and a switch beside it saying what a take records, Notes or
  Audio, after a one-bar count-in. Notes go into the piano roll the moment each ends: in pattern
  mode over the pattern as it loops, each pass adding (delete first to replace); in song mode
  into a new pattern of the channel's, "Kick take 1", whose clip grows by whole bars as you play.
  Audio records what you hear (the master) or the line in (the selected input channel's device,
  else the default input) into "<project> take N.wav": one pass of the pattern in pattern mode,
  until Stop in song mode, a clip from the song marker; the takes' channels fold under one
  Recordings row in the rack and their files are in the Library under Recordings. Capture
  (Ctrl+Shift+R) turns the notes you played in the last 30 s, without recording, into notes.
  Every take is one undo step (Undo keeps an audio take's file).
- **MIDI**: input ports (the ALSA sequencer on Linux, WinMM on Windows), controller links with
  learn, and the computer keyboard as a piano (Tab is its sustain pedal). A keyboard's sustain
  pedal holds the notes you let go of until it lifts (a take records them as they sounded); its
  pitch bend wheel bends the notes you play, ±2 semitones unless the instrument window's Bend
  says more (up to ±24); its mod wheel opens a channel's low-pass filter when it is on, else adds
  a vibrato. A controller link on the mod wheel (CC 1) or the pedal (CC 64) takes it instead. A
  channel with a MIDI out sends the pedal, the bend and the wheel on to its instrument; a CLAP
  plugin gets them too (the bend and the wheel as MIDI, when it takes MIDI), and a VST3 plugin
  as the parameters its MIDI mapping names for them (none, when it maps them to nothing).
- **Files**: projects saved as `.dawsome` files (JSON inside; `.json` projects open too) with
  autosave and recovery after a crash, recent files, MIDI file import and export per pattern,
  export to WAV (16-bit, 24-bit or 32-bit float) or MP3
  (320 kbps), a 16-bit file dithered (TPDF noise of one step at most, so a quiet fade ends in a
  soft hiss rather than in steps; the export dialog's Dither set Off rounds plainly), and export
  for other programs: the song as one MIDI file, stems, DAWproject (see
  [Taking a song to another program](#taking-a-song-to-another-program)). A project opens on its
  music: the first clip on Track 1 (else on the topmost track holding one) chooses the pattern and
  channel shown, in song mode.
- **Plugins**: CLAP and VST® 3 instruments and effects, scanned in separate processes so a
  crashing plugin never takes the app down, listed in one list with their format (a plugin
  installed in both formats is one row, its CLAP build); a plugin's own window (CLAP or VST3), or
  a parameter window for one without, and for any plugin from its channel's right-click menu
  (Parameters…) or its mixer slot's: its settings as a list of sliders in a window of its own,
  grouped as the plugin groups them, each one automatable by a right-click (a VST3 plugin's
  stepped parameters snap to their steps; the ones its MIDI mapping names are left out); a plugin
  that restarts is made again from its state; plugin state is saved in the project (a VST3
  plugin's as `.vstpreset` data), and kept when the plugin is missing. A VST3 plugin that delays
  its sound says so in its parameter window and its hint (CLAP plugins report no delay here).
- **Undo**: every edit, 500 steps back.

## What is included

- **Instrument sets**: an upright piano, an electric piano, a bass guitar, electric and synth
  basses, acoustic and electric guitars, a drawbar organ, a choir pad and a sweep pad, strings, a
  marimba, and acoustic and electronic drum kits, from the dawsome-kit repository (about 142 MB,
  at the commit `locks/kit.lock` pins); and seven electronic instruments made for Dawsome: two 808
  basses, a synth bass, a lead, a pluck and two pads.
- **Starter kit**: twelve drums made for Dawsome (808 and 909 kicks, snare, clap, closed and
  open hi-hats, three toms, rim, cowbell, crash), built into the app.
- **One-shots and loops**: a folder of sounds the Samples section lists.
- **Surge XT**: the Surge Synth Team's synthesizer, release 1.3.4, as a CLAP plugin with its
  factory patches and wavetables; the Library lists it under Plugins like any other.
- **Licenses**: every sound is dedicated to the public domain under CC0 1.0: use it in songs or
  in software, change it, sell it, with no credit due. `licenses/SOUNDS.md` lists where each comes from.
  The programs themselves are GPL-3.0-or-later (below), and so is Surge XT with its factory data
  (`LICENSE-SURGE`; `SOURCES.md` says where its source is). In a release zip, those two and
  `NOTICES.txt`, `NOTICE`, `THIRD_PARTY.md` and `SOUNDS.md` are in `licenses/`. `NOTICES.txt` holds
  the license and copyright notice of every library built into the programs, word for word. What
  you make with it is yours.

## Build and run

Dawsome builds with stable Rust (edition 2021; tested with 1.93). On Windows, follow
`docs/windows.md`. On Linux it also needs a C and C++ compiler, clang with libclang, pkg-config, the
development files of ALSA, PipeWire, JACK and D-Bus, and Python 3 for the tests. For example:

```sh
# Debian, Ubuntu
sudo apt install build-essential clang libclang-dev pkg-config libasound2-dev \
    libpipewire-0.3-dev libjack-jackd2-dev libdbus-1-dev python3
# Arch (pipewire-jack provides JACK; on a host running JACK itself, jack2 does)
sudo pacman -S --needed base-devel clang pkgconf alsa-lib pipewire pipewire-jack dbus python
```

Then, from the repository:

```sh
cargo run --release -p dawsome-app          # the app
cargo run --release -p dawsome-app -- song.dawsome
scripts/install.sh                          # Linux: install for your user (see below)
```

On Linux, `scripts/install.sh` builds the release binaries and installs `dawsome-app` (the
window) and `dawsome` (the command line) into `~/.local/bin`, a menu entry and the icons into
`~/.local/share`, the project file type (`application/x-dawsome-project`, `*.dawsome`, in
`~/.local/share/mime/packages`; the menu entry opens it, so a double-click on a project in a file
manager opens it in the app; the desktop's MIME database is refreshed when `~/.local/share/mime`
keeps one, else the install says how), the instrument sets into
`~/.local/share/dawsome/instruments`, and keeps a
list of the files it put and the folders it made in `~/.local/lib/dawsome/`. It fetches the sets
first (`scripts/fetch-kit.sh`: the dawsome-kit repository at the commit `locks/kit.lock` pins, checked
file by file). Surge XT goes into `~/.local/lib/dawsome/clap/`, where the programs look for plugins,
when `scripts/fetch-surge.sh` has fetched it first (about 350 MB of downloads, each held to the
SHA-256 `locks/surge.lock` pins). `--prefix <dir>` installs elsewhere; `scripts/uninstall.sh` removes
exactly the files the install put, then each folder it made once nothing else is in it. Neither
a reinstall nor the uninstall touches your settings, your own sets or projects, or a plugin of
your own put in `~/.local/lib/dawsome/clap/`. On Windows, the two
programs run from the folder they are built or unpacked in, the sets in `instruments/` and
Surge XT in `plugins/` beside them; to open `.dawsome` files with a double-click, choose
`dawsome-app.exe` once under Open with (docs/windows.md).

The app draws with the GPU when it can, and in software otherwise. On Linux it plays through
PipeWire when it runs and through ALSA otherwise; on Windows through WASAPI. Settings → Audio
output chooses the device (PipeWire's outputs by the names the desktop gives them), and a
choice plays at once.

On a Wayland desktop the app's windows open through XWayland (X11) when the session has it, as
its plugins' windows always do: a Wayland compositor lets no program keep a window above
another, so the instrument, Settings, Help and parameter windows would go under the main window
at a click on it. On X11 they stay above it while the app has the keyboard, and stack as any
window once another program has it. `DAWSOME_WINDOW_SYSTEM=wayland` keeps the windows on Wayland
(they may then go under the main window); `DAWSOME_WINDOW_SYSTEM=x11` asks for X11 in a run
with script flags too, which otherwise keeps the toolkit's choice. Text is sharp where the
desktop lets X11 programs scale themselves (KDE Plasma: Display Configuration → Legacy
applications (X11) → Apply scaling themselves); a screen at another scale than the one X11
programs are given shows the windows a little softer.

## First steps

- **Help** (at the right end of the toolbar, or F1) opens a guide for someone new to making music:
  what each part of the window is for and how to use it, a section a page, with pictures of the
  app.
- **The demo**: File → Open demo opens a short song made of the app's own synthesized sounds.
  A fresh install also lists it under the recent files.
- **A new document** starts with one pattern, an empty sampler channel, 8 playlist tracks and 16
  mixer inserts. "+ Instrument…" in the rack adds a channel: an instrument set, a starter kit
  sound, a plugin or a sound file of yours ("Sample file…"); drop a file on a sampler row to
  change its sound.
- **Panels** open and close with the digits 1 to 5, or F5 to F9 (below), and share the window.
  **Settings** (6 or F10, or the cog among the toolbar's panel buttons) opens a window of its
  own beside the main one: its pages scroll, and Esc, its close button or the same key closes
  it. It keeps its size and place for the next time.

## Keys

| Key | Does |
|-|-|
| Space | play / stop |
| L | pattern or song mode |
| 1, 2, 3, 4, 5, 6 | channel rack, playlist, piano roll, mixer, Library, the Settings window |
| F5, F6, F7, F8, F9, F10 | playlist, channel rack, piano roll, Library, mixer, the Settings window |
| F1 | the Help window |
| Ctrl+Z | undo |
| Ctrl+Shift+Z, Ctrl+Y | redo |
| Ctrl+A | select all notes in the piano roll |
| Ctrl+B, Ctrl+D | duplicate the selection one selection length later |
| Delete, Esc | delete the selection, clear the selection |
| Up, Down (Shift: an octave) | piano roll: move the selected notes a semitone |
| Left, Right | piano roll: move the selected notes by the snap |
| Ctrl+C, Ctrl+X, Ctrl+V | piano roll: copy, cut and paste notes |
| Enter | open the selected plugin channel's window |
| Ctrl+L | play the selected channel into the mixer's selected insert |
| R | arm recording |
| Ctrl+P | count-in on or off |
| Ctrl+Shift+R | Capture: the notes played in the last 30 s into notes |
| Ctrl+T | computer keyboard as a piano (A is C5, middle C; Z and X an octave down and up; Tab sustains; Backspace ends every note) |
| Arrow keys | move through the Library's sample folders (while the Library has the keys) |
| Ctrl+N, Ctrl+O, Ctrl+S, Ctrl+Shift+S | new, open, save, save as |

With the mouse: Alt+drag an audio clip's right edge stretches its sound to fill the new length
(a plain drag trims, adding silence when longer); Alt+wheel changes the playlist's track
height; a double click puts a fader back to unity and a pan back to the center; a double click
on a note deletes it and a right click opens the notes menu; on a playlist clip a right click
deletes it, Shift+right-click mutes it and Ctrl+right-click opens its menu (unlink, link back).
The piano roll's keys (the arrows,
Delete, Ctrl+D, C, X, V and A) act on it after a click in it, or while the cursor is over it with
notes selected, even while the computer keyboard plays notes. In the sample editor's window,
Space plays the selection, Delete deletes it, Ctrl+A selects the whole sound and Esc clears the
selection, then closes the window; a double-click on an audio clip opens it.

## Command line

```sh
dawsome render song.json song.wav --song --bars 8     # or song.mp3; --bits 16|24|32f
dawsome render song.json cd.wav --bits 16 --no-dither # 16 bits rounded plainly, no dither
dawsome render live.json live.wav --input-feed take.wav   # input channels play take.wav
dawsome scan                                          # find CLAP and VST3 plugins
dawsome smf-import song.json melody.mid out.json --pattern 1 --channel 2
dawsome smf-export song.json melody.mid --pattern 1 --channel 2
dawsome export-midi song.json song.mid                # the whole song, a track per channel
dawsome export-stems song.json out/ --bits 24         # out/song stems/01 Track 1.wav ... master.wav
dawsome export-dawproject song.json song.dawproject   # for Bitwig Studio, Studio One, Cubase
```

`dawsome` with no arguments prints every option. A render plays the project through the same
engine path the app plays it through, so the file holds exactly what you hear.

## Taking a song to another program

No music program opens another's own project files, so File has three exports that other
programs read, each followed by a report (shown, and written beside the export as
`<file>.report.txt`) of what did not carry over:

- **Export song as MIDI…**: one MIDI file (type 1) every program opens: the tempo, and a track
  per rack channel named after it holding its notes as the playlist plays them (repeats, clip
  offsets, notes cut at a clip's end). A MIDI file holds notes only: the sounds, the mixer,
  automation and audio clips stay behind (an audio channel's track is empty).
- **Export stems…**: a WAV file per playlist track (`01 Track 1.wav` …) and `master.wav`, all the
  song's length, in `<project> stems/`; each is the song with the other tracks' clips muted, so
  they line up from the start (and add up to the master, unless an effect such as a compressor
  hears several tracks at once). Every program reads them; nothing is editable as notes any
  more.
- **Export DAWproject…**: a `.dawproject` file, the open format Bitwig Studio (5.0.9 and later),
  Studio One (6.5 and later) and Cubase (14 and later) open: a track per rack channel with its
  volume, pan and mute, the pattern clips as note clips, the audio clips with their files packed
  inside, muted clips kept switched off, and a CLAP instrument (Surge XT) as that plugin with its
  saved settings. Samplers and instrument sets have no counterpart in the format (their tracks
  come without an instrument; the report names each sound file), and the mixer's inserts and
  effects, automation, clip gains and stretch modes stay behind. A program that does not read
  DAWproject takes MIDI and stems.

## Where things are kept

- Projects are single JSON files; sample paths are stored relative to the project's folder when
  the sound lies under it.
- Settings, recent files and the Library's sample folders: `~/.config/dawsome/`.
- The demo, the starter kit (written there the first time one of its sounds is played or
  dragged) and the recordings of documents not yet saved: `~/.local/share/dawsome/`.
- Instrument sets of your own: `~/.local/share/dawsome/instruments/`, a folder each (an .sfz,
  its sounds, and a `set.json` naming the set and its maker). A folder there of the same name as
  an installed set plays in its place. SFZ files in your sample folders list under My
  instruments.
- The plugin catalogue, stretched or converted audio and the edited sounds of long files:
  `~/.cache/dawsome/`.
- Autosaves of untitled documents: `~/.local/state/dawsome/`; a saved project's autosave sits
  beside it (`song.autosave.dawsome` for `song.dawsome`, `song.autosave.json` for `song.json`).

The `XDG_*` variables move these folders as usual; `DAWSOME_CONFIG_HOME`, `DAWSOME_DATA_HOME`,
`DAWSOME_CACHE_HOME` and `DAWSOME_STATE_HOME` move them on every platform. Windows keeps them in
`%APPDATA%\dawsome\` and `%LOCALAPPDATA%\dawsome\` (see `docs/windows.md`).

## Limits

- Linux and Windows. On Windows the audio thread runs at normal priority, and building needs
  the MSVC tools and LLVM; `docs/windows.md` has the build steps, the cross-build from Linux and
  the details.
- On Linux the windows open through X11 (XWayland on a Wayland desktop, unless
  `DAWSOME_WINDOW_SYSTEM=wayland`, where the pop-out windows can go under the main one). A
  VST3 plugin's own window opens through X11 in either case, at the
  plugin's own scale (Dawsome gives it none); one without a window opens its parameter window. A
  plugin installed as both CLAP and VST3 is listed once, as CLAP (a project naming its VST3 build
  still plays it). VST3 on Windows, its windows included, is built but not yet tried on a Windows
  machine. No LV2 or LADSPA. Plugin latency is shown, not compensated: a VST3 plugin's parameter
  window, its rack or slot hint and a render name the delay it reports.
- One note at a time acts within one channel: there are no cut groups across channels (an
  open hi-hat cut by a closed one), and a plugin's or a MIDI out's notes are its instrument's to
  end.
- Files dropped from another program (a file manager): a window on Wayland itself gets none, as
  the toolkit (winit 0.30) takes no drops there; add their folder to the Library and drag them
  from there, or use File → Import audio…. On X11 (XWayland too) and Windows a drop carries no
  position of its own, so the app asks the window system where the pointer is over its window and
  lands the file there (on Windows built, not yet tried on a Windows machine); when it cannot say, it lands where the window last saw the
  pointer, and when it saw none, at the song position on a free track. Under XWayland a drop from
  another X11 program has been checked; one from a Wayland program, which the desktop passes on
  to X11, not yet.
- One tempo per project, and four beats to the bar.
- The sample editor's edits belong to a channel's sound: every clip of the channel plays them,
  and one clip cannot be edited alone yet. It has no markers, slicing or loop points, no copy and
  paste of audio, and no pitch or time change of a part (planned for later). A project holding
  edits is written as format version 8, which older builds refuse to open rather than play the
  files unedited. An edited sound kept in the cache is told from a replaced file by the file's
  size, its time and its first and last 64 KiB: a file changed only in its middle, at the same
  size and time, plays as cached (as does a stretched clip of an unedited file changed at the
  same size and time) until the cache (`~/.cache/dawsome/`) is cleared.
- A take of the line in is placed earlier by the input's latency once Settings → Input latency
  has measured it (loop a cable from the output into the input and press Measure) or it is set
  there by hand; until then such takes land late by it, typically 10 to 40 ms. Plugin latency is
  still not compensated. An input channel is heard a few buffers behind (its window says how
  far). An input device must offer
  the project's sample rate: inputs are not resampled.
- The Limiter catches each peak on its own sample, with no lookahead (so no latency): it holds
  the samples under its ceiling, not the true peak between them, which an MP3 encoder or a
  converter can raise a little past it. Projects made before it keep their masters without one.
  When more sound at once than the player holds (128 instrument zones from patterns and 32
  played live, 64 sampler notes and 16 played live, over all channels), the oldest is cut off
  at once, not faded out.
- Pitched sounds are read by 4-point Hermite interpolation; there is no windowed-sinc read, so a
  note far above its sound's root still folds a little of its highs back below. The Density
  saturator runs without oversampling.
- The 16-bit dither is plain TPDF, with no noise shaping, and silence is dithered too: a silent
  stretch of a 16-bit file holds noise one step high (−90 dBFS peak, −96 dBFS RMS; `--no-dither`,
  or Dither Off in the export dialog, for digital silence). Every file's dither starts from the
  same seed, so the same project gives the same bytes each time, and 16-bit stems summed back add their dither alike.
  24-bit and 32-bit float files are never dithered.

## Contributing and license

See `CONTRIBUTING.md` for how to build, test and send changes.

Dawsome is free software: you can redistribute it and/or modify it under the terms of the GNU
General Public License as published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version. It is distributed without any warranty; see
`LICENSE` for the full text and `licenses/NOTICE` for the copyright notice. Code ported from Airwindows,
the libraries built in and the bundled Surge XT keep their own licenses, listed in
`licenses/THIRD_PARTY.md`. The real plugins some tests load when they are fetched (Surge XT's VST3
bundles, Dexed, JS80P and DPF-Plugins' 3-Band EQ, under the GPL-3.0, LGPL-3.0 and ISC licenses:
`scripts/fetch-surge.sh --vst3`, `scripts/fetch-vst3-test-plugins.sh`) are test inputs only and
never part of a release.

VST is a registered trademark of Steinberg Media Technologies GmbH.
