# Changelog

What each release of Dawsome does for you. The manual (docs/manual.md) has the details.

## 0.1.0 — unreleased

The first release: a pattern-based digital audio workstation for Linux and Windows.

- **Channel rack**: sampler channels with a step grid, volume and pan, patterns of 1 to 32 bars,
  and plugin instruments as channels. A new channel that plays notes starts at −5 dB, so a few
  notes together stay clear of the master.
- **Piano roll**: draw, move, resize and delete notes; a velocity lane; selections, copy, cut,
  paste and duplicate; Quantize, Chop, Glue, Legato, Reverse, Invert and Humanize; a scale
  chooser; the notes you play live shown as they sound.
- **Playlist**: pattern clips, audio clips (trim by dragging an edge, Alt+drag the right edge to
  stretch the sound to fit, gain, time-stretch or resample with pitch shift) and automation clips; pattern and song mode; a loop range. Clips of one pattern are
  linked, and one click gives a clip its own copy or links it back. Drag a clip, its edge or a
  selection box past the left or right edge and the playlist scrolls along, faster the further
  you go.
- **Stop ends every note**: a key you hold on the typing keyboard, a MIDI device or an
  instrument window stops with the song and plays again when you press it next; Backspace on
  the typing keyboard ends every note without stopping the song. Play and a click on the ruler
  keep a held key sounding.
- **Sample editor**: crop, delete, silence, fade, gain, normalize and reverse a sampler's or an
  audio clip's sound in a window of its own, opened by a double-click on an audio clip or Edit
  sample… in the channel's menu, each change one step of Undo. Your file is never changed: the
  project keeps the changes, every clip of the channel plays them, and Save as new file… writes
  the result as a WAV.
- **Mixer**: the master and 16 inserts (up to 500), faders, pan, mute, solo, sends and ten
  effect slots per insert; Mixer insert… in a channel's menu gives it a strip of its own. Built in: an EQ, a delay, a reverb, a limiter (on the master of every
  new project, so an export never clips), and the Pressure4 compressor and Density saturator
  from Airwindows.
- **Sounds**: a starter kit of twelve drums, seven electronic instruments, multisampled
  instruments (piano, electric piano, basses, guitars, organ, pads, strings, marimba, drum
  kits) and a folder of one-shots and loops, all free to use under CC0 1.0. Your own SFZ
  instruments play too.
- **Sound settings**: how a channel's notes end, its tuning, bend range, attack, release, filter
  and velocity, and One note at a time for basses that should not stack.
- **Plugins**: CLAP and VST 3 instruments and effects, scanned apart so a crashing plugin never
  takes the app down; a plugin's own window, which keeps to the size the plugin can take as you
  drag it, or a list of its parameters, each automatable; its state saved in the project. Surge
  XT comes with it.
- **Recording**: notes or audio after a one-bar count-in, in pattern or song mode, each take one
  undo step; Capture turns the notes you played in the last 30 seconds into notes. External
  input channels play an audio input live, and a take of the line in can be placed by the
  input's measured latency.
- **MIDI**: input ports with controller links and learn, the sustain pedal, pitch bend and the
  mod wheel, the computer keyboard as a piano, and MIDI out to hardware instruments.
- **Files**: `.dawsome` projects with autosave and recovery after a crash; import of MIDI files
  and audio; export to WAV (16-bit dithered, 24-bit, 32-bit float) or MP3,
  and for other programs the song as one MIDI file, stems or a DAWproject file.
- **Command line**: `dawsome` renders a project to the same audio the window plays, scans
  plugins, imports and exports.
- **Help**: a Help window with a page for each part of the app, and a hint bar that names what
  is under the pointer.
