# Dawsome

The reference for Dawsome 0.1.4: every panel, gesture and setting. The README is the short introduction.

Dawsome is a pattern-based digital audio workstation for Linux and Windows. Program beats in the
channel rack, write melodies in the piano roll, arrange patterns, audio and automation in the
playlist, and mix through inserts, sends and effects. It hosts CLAP and VST® 3 plugins, records
audio and MIDI, and renders the same audio from the command line that it plays in the window. It
is free software under the GNU General Public License, version 3 or later.

Dawsome is not affiliated with Dawesome, the plugin maker, whose name sounds the same.

## Features

- **Channel rack**: sampler channels with a step grid (a step is a 16th note, so a bar holds 16
  steps in 4/4, 12 in 3/4 and 6/8; see **Time signature**), volume and pan, patterns of 1 bar
  up to the most the rack's 512 steps hold (32 bars of 4/4, 42 of 3/4), each with its own swing
  (see **Swing**); plugin instruments (CLAP or VST3) as channels. A channel that plays notes
  starts at −5 dB, so a few notes together stay clear of the master; audio
  and input channels start at 0 dB. The unlit steps alternate two dark tones by beat, with a
  soft line between the beat groups; a muted channel's light (left of its name) is a ring, a
  heard one's a green dot, as a track's and a mixer strip's are. Its header reads, left to right,
  "Channel Rack", the pattern's **Swing** (its knob and readout) and its **Bars** (− the count
  +). The steps stand in groups a beat wide: four for a quarter-note beat (3/4, 4/4), two for
  an eighth (6/8, 7/8), one for a sixteenth (then with no gap between them); **+ Instrument…** stands under the last channel row, in line with the channels' names,
  as words alone, as **+ Track** stands under the tracks.
- **Piano roll**: draw, move, resize and delete notes, a velocity lane, selections, duplicate,
  snap and zoom, the other channels' notes shown faintly; a sample plays at the pitch of its key.
  Selection and editing: click, Shift+click and box selection, arrows to transpose and nudge,
  copy, cut and paste, and a notes menu (right-click, or the strip's Notes button) with Quantize
  (grid, strength, swing, starts or ends), Chop, Glue, Legato, Reverse, Invert and Humanize; velocity
  painted across stems or drawn as a ramp (Alt); a scale chooser that shades the keys outside it
  and folds the rows; the notes you play live (keyboard, MIDI) shown as they sound, and a
  take's light while recording them.
  - **The grid** reads as the playlist's does (**Reading the grid**, under **Playlist**): a
    ruler of bar numbers over it, a double line and a brighter number every fourth bar, every
    other bar shaded a little lighter, and finer lines (beats, steps, half steps) that fade in
    as you zoom in; the ruler ticks each beat too, fading with the beat lines. Past the
    pattern's end the rows, their shading and the lines are all dimmed, so the empty bars read
    quieter than the pattern. **Snap: Line**, the snap the roll starts with, snaps to the
    finest lines drawn at least half strength: half steps from 24 px a step, steps from 12,
    beats below that. A click draws a note one snap unit long (the strip's **Len**, or the snap
    picker's hint where the strip has folded Len away), so the
    smallest note a click draws covers the smallest cell you see, and it follows the zoom;
    once you click or resize a note, its length is the one drawn next, at any zoom. Under
    **None** (single ticks) a click draws a Line's length.
  - **The key column** at the left is drawn as a keyboard: each black key covers two thirds of
    the column, the white key showing beside it, and every C is named. Pressing a key's row
    plays that key. The grid shows whole rows only, from the top: the few pixels left under the
    last whole row are plain ground over the scroll bar, where a press does nothing, and the
    keys scroll a whole row at a time, so no key is ever cut to a sliver.
  - **Zoom**: anywhere from 4 to 64 pixels a step (16 to start); Ctrl+wheel over the grid or
    the ruler zooms a quarter of a doubling a notch, keeping what is under the pointer in place.
    The zoom is kept for the next start. Scrolling along time (Shift+wheel, a page or a notch on
    the scroll bar, letting go of its thumb, Follow's jumps) comes to rest on a whole beat. The
    scroll bar under the grid runs the panel's whole width, under the keys too: at the
    pattern's start its thumb begins at the panel's left edge.
  - **The velocity lane** under the grid is 32 pixels tall, always, on a dark ground between
    two light lines: a stem's height is its note's velocity, 128 at the lane's top and 0 at its
    foot. Every note in view has its stem: one scrolled half out of view on the left has it at
    the lane's left edge, and the notes of a chord share one column, the taller stems showing
    above the shorter. Alt+drag across the lane draws a ramp: each note it crosses takes the
    ramp's value where its stem stands, the stem pinned at the left edge included.
  - **The strip** over the roll: the channel's name, then the snap picker (Line, Step, Beat,
    Bar or None, no word before it) and Len, Preview and Follow, the scale (its chooser half as
    wide as its longest name: a long name shows cut short with "…" there, and whole in its menu)
    and Fold, the **Notes** menu, and the roll's **Play** and **Loop**, the two a little apart
    from the rest; the velocity a lane drag
    sets and the "Rec notes" light come and go after them, and the pop-out grip stands at the
    right end. Where the roll is too narrow for all of it, the strip folds, step by step until it
    fits: Len goes (the snap picker's hint then says the length, "Snap: Line · draws 0:01:00"),
    then Preview and Follow become 24 px buttons with a small speaker and a small arrow (lit as
    their words were), then the scale's chooser narrows to 48 px (the arrow alone; its menu and
    hint name the scale), and last the channel's name is cut short with "…" to the room left,
    never to fewer than its first three letters ("Bass 808 low" becomes "Bas…"; its hint says
    the name whole). Play and Loop, the snap picker, the scale and Fold, Notes and the grip
    always stay. Beside the Library (a 700 px roll) Len folds, and with a scale chosen Preview
    and Follow fold too; the roll in its own window at its least width, 608, folds Len, Preview
    and Follow, with a scale the scale's chooser too, and with a long name the name, every
    control still showing (a name whose first three letters are wider than "Bas", the velocity
    readout or the "Rec notes" light can still push Loop past the edge: widen the window).
  - **Playing from the piano roll**: the strip's **Play** plays the notes the roll holds, this
    channel's notes of this pattern alone (not the other channels', not the faint notes), from
    the pattern's start, through the channel's real sound: its volume, pan, mixer insert,
    effects and routing, so it sounds as it does in the song, while the other channels on the
    same insert stay silent. **Loop** (on to start) repeats the pattern; off, it plays once and
    stops at the pattern's end. Edit notes while it loops and the next pass plays the edit;
    choose another channel or pattern and it plays that. It is the one transport, not a second
    player: the toolbar shows neither Pat nor Song lit and its position runs, Play in the
    toolbar takes back over (from the pattern's start, or from the song marker in song mode),
    the roll's Play while the song plays takes over from the pattern's start, and Stop (or
    Space) stops either. While a take records, the roll's Play is off: recording is the
    toolbar's, so stop the take first. **Shift+Space** is the roll's Play / Stop while the roll
    has the keys. Loop is kept for the next start.
- **Playlist**: tracks holding pattern clips, audio clips (trim, gain, waveform, time-stretch or
  resample with pitch shift) and automation clips (linear, hold, smooth and exponential segments);
  a pattern clip draws each note as a bar from its start to its end, as the piano roll does, one
  row per pitch, a step hit as a short mark, a note running past the clip's end cut there;
  pattern and song mode; a loop range; the timeline's scroll bar under the tracks, across the
  whole panel (under the track names too, so at the song's start its thumb begins at the
  panel's left edge). The lanes fit the panel, squeezing down to 32 px as tracks are added, then
  the tracks scroll; while they fit they grow up to 106 px, the "+ Track" row under the last track taking only its own height.
  Alt+wheel fixes a height (32, 48, 64, 85 or 106 px) and the header's Fit fits them again. A
  sound file
  dropped from the desktop on a track is a new audio channel and its clip there, from the bar,
  beat or step the Snap chose (several files on the tracks under it, one undo step). Clips of one
  pattern are linked (an edit changes them all) and say so with "×N"; while such a clip is
  selected, a chain right of the header's Fit unlinks every selected clip (each its own copy,
  "Pattern 1 (copy)", "(copy 2)"…) and, crossed once they play copies, links them back, one
  undo step a click; a clip's menu (Ctrl+right-click) does the same for that clip.
  - **Reading the grid**: a bar is the time signature's beats (four quarters in 4/4, three in
    3/4, six eighths in 6/8), and a new pattern is one bar long, so its clip fills one bar. The lanes' lines mark the bars and, between them, the beats; every fourth bar
    (1, 5, 9, …) starts with a brighter double line and has its number written brighter in the
    ruler, so a song reads in phrases of four bars. Every other bar is shaded a little lighter
    (every other four bars when zoomed far out), so the bars read as blocks even where their
    lines are faint. Beat lines fade in as you zoom in and are gone when the beats are only a few
    pixels apart. Inside a loop range the lanes keep the loop's tint, unshaded.
  - **Zoom**: anywhere from 16 to 256 pixels a bar (64 to start). **−** and **+** halve and
    double it; Ctrl+wheel over the lanes or the ruler zooms a quarter of a doubling a notch,
    keeping what is under the pointer in place. The zoom is kept for the next start. Scrolling
    along time (Shift+wheel, a page or a notch on the scroll bar, letting go of its thumb, − and
    +, Follow's jumps) comes to rest on a whole beat, or on a whole bar while beat lines are not
    drawn, so the view never starts on a sliver of one; a drag of the thumb is smooth while you
    hold it.
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
  would make a loop is refused), up to ten effects per insert. Mixer insert… in a channel's
  right-click menu (and at the end of its instrument window's Out list) gives the channel an
  insert of its own, the first free one, and opens the mixer there. Built in: a three-band EQ, a delay
  (in milliseconds or synced to the tempo), a reverb, a limiter, the Pressure4 compressor and
  Density saturator ported from Airwindows, a filter, a distortion, a chorus, a flanger, a
  phaser, a gate and a stereo width.
  Plugin effects (CLAP or VST3) go in the same chain.
  - **The header's routing control** (− "Insert 3 → Master" +), centred in the mixer's header,
    names the selected insert and where it goes; the effects area has no title of its own.
  - **The effects area**: right of the strips, the selected insert's effects, in the room the
    strips leave (at least 332 px: in a narrower window the strips scroll under it). Each effect
    is a block, in chain order (the order the sound goes through them), left to right, a line
    under the one before when the room runs out; the wheel scrolls the lines when they are
    taller than the area. A block's head holds a green light like a strip's, the effect's name,
    ‹ and › and a remove ×; under it its settings as knobs, the value under each (drag right
    for more, Shift for fine, double-click for the default, right-click to automate or link a
    controller), five to a row and two rows at most. A choice (the Filter's type, the Phaser's
    stages) is a button: each click picks the next. A plugin effect shows its first ten
    settings; its menu's Parameters… has them all, and a double-click on its name opens its
    window.
  - **The light** switches an effect on or off, one undo step ("Effect on", "Effect off"): an
    effect that is off is passed by, the insert's sound going on unchanged, with its settings
    kept; switched on again it starts clear (a delay's old echoes are gone). **A new effect
    starts off**: "+ Add effect", the tile after the last block, puts the effect chosen at the
    end of the chain with its light off, so click the light to hear it. A project saved before
    effects had a light plays every effect on (the file writes `"on":false` only for one that
    is off).
  - **The order**: drag a block by its name past the middle of the block beside it to swap
    them (Esc while dragging puts it back), or click its ‹ or › to move it one place; each move
    is one undo step. Right-click a block for Remove (and a plugin's Parameters…), or click its
    ×.
  A new project starts with the Limiter as the master's first effect, on (ceiling −0.3 dBFS,
  release 100 ms, gain 0 dB), so chords and sounds played together never clip the output or an
  export; a project saved before keeps its master as it was (add the Limiter from the add tile).
  The Limiter's Gain (0 to +24 dB) raises the sound before it is limited: turn it up to make a
  mix louder and denser while no sample passes the ceiling. At 0 dB it changes nothing, so a
  project saved before Gain existed sounds as it did.
  - **Filter**: low-pass, high-pass, band-pass or notch (a state-variable filter), with a Slope
    of 12 dB per octave (one stage) or 24 dB (two in series, steeper: the classic synth filter
    sweep). Cutoff sweeps 20 Hz to 20 kHz evenly in octaves (632 Hz at the middle), and stops at
    45% of the sample rate (19.8 kHz at 44.1 kHz); Resonance sets Q from 0.5 to 20, also in
    octaves (the default 10% is Q 0.72, close to a flat Butterworth response). Both slopes ring
    the same amount at the cutoff. Above the default, the low- and high-pass turn themselves
    down as Resonance rises (by √(0.72/Q): nothing at the default or below, 14.4 dB at 100%), so
    a resonant sweep stays near the level of the sound it sweeps instead of jumping 13 dB over
    it; a sine right at the cutoff still comes out 11.6 dB louder at 100%. The band-pass stays at 0 dB at its cutoff and only
    narrows, and the notch never rises, so neither is turned down. No mix: the dry sound is what
    the type takes out.
  - **Distortion**: Drive (0 to 40 dB) into a soft clipper (tanh, which itself never goes past
    full scale), its aliasing lowered by its antiderivative (first-order ADAA, about as much as
    2× oversampling) with no added latency; then Auto gain, Tone, a gentle one-pole low-pass
    from 1 kHz to 20 kHz, Output (−24 to +6 dB) and Mix. Auto gain (on by default) turns the
    distorted sound down by about as much as the drive made it louder (d/(1 + d/40) dB for d dB
    of drive: 9.2 dB at the default 12, 20 dB at 40), so you hear the change in tone, not just
    a jump in level; how much louder a sound gets depends on the sound, so it keeps the level
    roughly (within a few dB), not exactly. Switch it off to drive the next effect harder. The
    anti-aliasing delays the distorted sound by half a sample, so at a partial mix the top of
    the band dips a little (−2.4 dB at 10 kHz, 44.1 kHz, mix 50%). Silence stays exactly silent.
  - **Chorus**: three voices of the sound, each 15 ms late, their delays swept by one slow sine
    (Rate, 0.1 to 5 Hz, 0.5 Hz by default) at 0°, 120° and 240° of it; Depth 100% sweeps each
    5 ms either way (10 to 20 ms; the default 30% sweeps 1.5 ms, a gentle wobble of about 8
    cents, as the classic Juno chorus). The right side's sweep runs 90° ahead of the left's, so
    even a mono sound comes out wide. The three are added at the level of one (their sum over
    √3) and cut below 150 Hz, so a bass or a kick keeps a clean low end, then mixed with the dry
    sound at equal power (Mix: at 50% each at −3 dB, so the chorus keeps the sound's level);
    no feedback. Below 150 Hz the sound is the dry part alone, so at 50% the deep bass is 3 dB
    lower. Between whole samples the delayed sound is read by the same 4-point Hermite
    interpolation as a pitched sample, which dulls the highs less than a straight line between
    samples would.
  - **Flanger**: the sound mixed with a copy of itself a few milliseconds late: Delay (0.1 to
    10 ms) is the longest the sweep reaches, and Depth how far below it the sweep goes, as a
    share of it (at 100% down to 2 samples, the shortest it reads, so the sweep stops just
    short of no delay at all). Rate 0.05 to 5 Hz; Feedback (−95% to +95%) sends part of the
    delayed copy back in, sharpening the comb (negative feedback moves its teeth by half a
    step). Spread (0° to 180°, 90° by default) runs the right side's sweep that far ahead of the
    left's, so the flange moves across the stereo field; at 0° both sides share one sweep (mono).
    Mix is equal-power (at 50% the dry and the delayed sound each at −3 dB), and 50% gives the
    deepest notches.
  - **Phaser**: the sound through a chain of 4, 6 or 8 allpass stages (Stages), each turning
    the phase by 90° at one frequency that a slow sine sweeps, mixed with the dry sound so the
    notches where the two meet out of phase move up and down (two notches for 4 stages, three
    for 6, four for 8). Center sets the middle of the sweep, 100 Hz to 4 kHz evenly in octaves
    (632 Hz at the middle), Depth its span (0 to 4 octaves, half either side), Rate 0.05 to
    5 Hz; the sweep stays between 20 Hz and 45% of the sample rate. Feedback (−95% to +95%) runs
    the chain's output back into it, for resonant peaks between the notches. Spread (0° to 180°,
    90° by default) runs the right side's sweep that far ahead of the left's, so the notches
    swirl across the stereo field; at 0° both sides share one sweep and the effect stays
    mono-compatible. Mix is equal-power (at 50% the dry and the phased sound each at −3 dB), and
    50% gives the deepest notches.
  - **Gate**: Threshold (−80 to 0 dBFS, −40 by default), Attack, Hold, Release, and Range (how
    far a closed gate turns the sound down, −80 to 0 dB; at 0 dB it leaves the sound
    untouched). Fixed: the level it listens to is the louder side's RMS over about 10 ms, as a
    meter reads it (so hiss at −50 dBFS reads −50, though its peaks reach about −40); it opens
    at the threshold and stays open down to 3 dB under it (hysteresis, so a level hovering at
    the threshold does not chatter), then for the hold, then closes by the release. Attack and
    release are the times the gain takes to cover 63% (1 − 1/e) of its way. No sidechain
    input.
  - **Stereo width**: 0% is mono (both sides the middle of the two), 100% leaves the sound as it
    is, bit for bit, and 200% doubles the difference between the sides. Width only changes what
    differs between the sides: a mono sound (a single sample, a mono instrument) passes as it
    is. Bass mono (Off, or 50 to 500 Hz) makes the sound mono below that frequency: the
    difference between the sides goes through a 12 dB per octave high-pass there, so an octave
    below it the sides are about 12 dB quieter, and the middle is untouched.
- **Sound**: a sample or an instrument set's sound played off its own pitch (another key, the
  channel's tuning, the bend, the mod wheel's vibrato, a resampled audio clip) is read between
  its recorded frames by 4-point cubic Hermite interpolation, a common realtime choice in
  samplers: less of its highs is dulled or folded back below than with a straight line between
  frames. At its own pitch it plays its frames exactly as recorded.
- **Library**: one panel down the left with one search over its sections: Instruments (the
  starter kit's drums, the instrument sets, your own SFZ sets and your plugin instruments by
  maker, a VST3 one marked so), Samples (your sample folders and the shipped one-shots and
  loops, with preview) and MIDI devices (what is plugged in). A click plays the sound at the
  level a new channel plays it (a step of it on a channel just added). Drag a sound or an
  instrument onto the rack or a playlist track, and a MIDI keyboard onto a rack channel to play
  that channel whatever is selected.
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
- **Dawsome Synth**: a synthesizer of Dawsome's own, a channel like any instrument: two
  oscillators with unison and noise, a resonant low-pass with its envelope, an amp envelope, an
  LFO, glide and velocity, 16 voices, its knobs in its instrument window and eight presets to
  start from (see [Dawsome Synth](#dawsome-synth)).
- **External input**: a channel that plays an audio input live (a synth or keyboard cabled into
  your interface): pick its device and channels in its window and it plays through its volume,
  pan and mixer insert like any channel, playing or stopped (Library → Instruments → External
  input).
- **Recording**: one Record button and a switch beside it saying what a take records, Notes or
  Audio, after a one-bar count-in, a click on each of the bar's beats (a key struck there and
  held into bar 1 starts on bar 1). Notes
  go into the piano roll the moment each ends, which scrolls up or down to show each key struck
  on the channel it shows: in pattern mode over the pattern as it loops, each
  pass adding (delete first to replace; a take with notes selected gets a new pattern, a blank
  take becomes a pattern); in song mode into the blank take selected, else a new take of the
  channel's, "Kick take 1", which starts one bar long at the song marker (Record plays from
  there) and grows by whole bars as you play; no clip is placed, so put the take on a track
  yourself. Patterns and takes: the pattern box's right arrow, past the last, makes a pattern in
  pattern mode and a take in song mode; switching modes turns a blank pattern into a blank take
  and back, and leaves one with notes as it is. While a take of notes runs, a click in the
  Library previews nothing until Stop.
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
  (Parameters…) or its mixer block's: its settings as a list of sliders in a window of its own,
  grouped as the plugin groups them, each one automatable by a right-click (a VST3 plugin's
  stepped parameters snap to their steps; the ones its MIDI mapping names are left out); a plugin
  that restarts is made again from its state; plugin state is saved in the project (a VST3
  plugin's as `.vstpreset` data), and kept when the plugin is missing. A VST3 plugin that delays
  its sound says so in its parameter window and its hint (CLAP plugins report no delay here).
- **Undo**: every edit, 500 steps back.

## Swing

Swing plays every second 16th note of a pattern late, for a shuffle. Each pattern has its own,
set with the **Swing** knob at the top of the Channel rack, right of its title; the
readout beside it shows the swing of the pattern in the pattern box.

- **The scale**: how much of each pair of 16ths (one 8th note) the first 16th takes. 50 % is
  straight, 66.7 % a triplet feel (2:1), 75 % dotted (3:1), the most it goes. A new pattern
  starts straight; a clip unlinked into a copy keeps its pattern's swing.
- **What moves**: only a note that starts exactly on the second 16th of a pair, the 2nd, 4th,
  6th… step of each bar. Notes on the beats and the 8ths, and notes placed between the steps in
  the piano roll, stay where they are. A moved note keeps its length, so it ends late too; a
  step's hit still ends where the next step starts.
- **Turning it**: drag left or right, 1 % per 4 pixels (0.1 % with Shift held); the mouse wheel
  moves it 1 % a notch (0.1 % with Shift). A double-click sets it straight again. You hear it
  change while you drag, and the whole drag is one undo step; each wheel notch is one.
- **What you hear**: with 96 ticks to a beat, the late note moves in whole ticks, so 13
  positions sound between 50 and 75 %: the second 16th plays from 0 to 12 ticks late, the first
  tick late at 52.1 %, 8 ticks (the exact triplet) at 66.7 %, 12 at 75 %. With the pointer on
  the knob, the hint bar shows how many ticks late it plays beside the per cent.
- **Drawn straight, played swung**: the step grid and the piano roll draw the notes where they
  are stored, on the grid; playback, plugins, MIDI out, renders and stems play them swung.
- **Exports carry it**: the song's MIDI file, a pattern's MIDI file and DAWproject put the late
  notes where you hear them, so another program plays the same groove.
- **Not swung**: notes you play live on the keyboard or a MIDI device, the metronome, and audio
  clips. A take of notes records them where you played them.
- **Older builds**: a Dawsome from before swing opens such a project and plays it straight, and
  saving it there drops the swing.

**Quantize with swing** (in the piano roll, Notes → Quantize…) swings the grid itself: every
second line of the grid sits late by the swing you choose there, and the notes move toward the
nearest line, as far as Strength says. While the dialog is open the piano roll previews each
choice on the notes as they were, with their old places drawn faint, and the dialog says how
many notes would land on another and merge; nothing changes until you press Quantize, and Cancel
leaves the notes as they were. Playback meanwhile plays the notes as they are, not the preview.

- **The Swing row**: under Strength, a slider from 50 % (straight) to 75 % (dotted) in steps of
  0.1 %, its value beside it, and a **Triplet** button that sets 66.7 %, the exact triplet (lit
  while set). It starts straight, and the dialog remembers it with the grid and the strength. On
  the 1/16 grid the late lines fall where the knob plays the second 16ths at the same per cent.
- **It moves the notes**: quantize writes the late ticks into the pattern, and the piano roll
  draws them there; one undo step puts them back. A note half way between two lines goes to the
  later one; with **Starts + ends**, each end moves toward its own nearest line.
- **It leaves the step grid**: the step grid lights only notes that start exactly on a step, so
  a note quantized late (8 ticks after its step, at 66.7 % on 1/16) no longer shows there; it
  lives in the piano roll.
- **Use one**: quantize with swing moves the notes; the knob moves only notes still on a
  straight second 16th. On one part, use one: the knob for step patterns, quantize for played
  parts you want to edit as you hear them.

## Dawsome Synth

Dawsome Synth is a small subtractive synthesizer built into Dawsome: two oscillators, mixed and
then filtered, shaped by two envelopes and an LFO. It is a channel of its own kind (its rack
row's hint and its window say Synth) and plays like any instrument: patterns, takes, the
computer keyboard, a MIDI keyboard with its sustain pedal, pitch bend and mod wheel.

- **Adding one**: in the Library, under Instruments → Plugins, Dawsome Synth is the first row
  (above the plugins found on the computer). Click it to hear a note, double-click it (or
  Enter) to add it to the end of the rack, right-click it to add it and open its window, or drag
  it onto the rack or a playlist track. It starts at its defaults (the Init preset).
- **Its window**: a double-click on its name in the rack. The synth's knobs in eight blocks,
  one per heading, laid out like a DJ's booth: a deck on each side (Osc 1 over Unison on the
  left, Osc 2 over the LFO on the right) and the mixer between them (Filter, Filter env, Amp env
  and Play, one under another). Under the left deck the LEVEL group (pan, volume, the output
  insert) as for any channel, one module as wide as the deck; under the right deck the Presets
  module (the Preset list and Save preset…); every column reaches down to one line, so no bare
  board shows. The window opens large enough to show the whole booth (924 × 516), and grows to
  that again each time it shows a synth channel; made smaller, the window scrolls and the
  blocks stay where they are.
- **The knobs** work as an effect's knobs in the mixer: drag left or right (Shift for fine), a
  double-click puts the default back, the value under each knob is in its unit, and a choice
  (the waves, the unison's voices, the LFO's shape) is a button that steps to the next value on
  each click. Each drag, reset or click is one undo step. Right-click a knob for Create
  automation clip (Show automation clip once it has one) and Link to controller… (the next knob
  moved on a MIDI controller takes it; Unlink controller once linked); an automated knob shows
  its automation's value and a dot, a linked one a small mark beside it. Every setting is saved in the project.

The settings, by heading (each knob's name in the window, then what it does):

- **Osc 1** and **Osc 2**, the two oscillators: **Wave** Sine, Triangle, Saw or Square
  (oscillator 1 starts as a saw, oscillator 2 as a square); **Width** the square's duty, 5 to
  95% (50% a plain square; only the square uses it, and it stays centred on zero at any
  width); **Octave** −2 to +2; **Semi** −12 to +12
  semitones; **Fine** −100 to +100 cents; **Level** 0 to 100%. The saw and the square are
  band-limited at their jumps (PolyBLEP), the triangle is not (its harmonics fall fast enough to
  keep its aliasing low). Under Osc 2, **Mix**: 0% is oscillator 1 alone (the default), 100%
  oscillator 2 alone, 50% both at half.
- **Unison**: **Voices** 1, 2 or 3 copies of each oscillator per note, their phases spread;
  **Detune** 0 to 50 cents, the distance between the lowest and the highest copy (10 by
  default); **Spread** 0 to 100%, how far the copies are panned apart (the outer ones hard left
  and right at 100%); the copies are added at the level of one. **Noise** 0 to 100%: white noise
  added to each voice, the same on both sides.
- **Filter**, a resonant low-pass per voice (the Filter effect's state-variable filter at 12 dB
  per octave): **Cutoff** 20 Hz to 20 kHz evenly in octaves, open at 100% (the default, where the
  filter passes the sound untouched while nothing moves it); **Resonance** Q 0.5 to 20, as the
  Filter effect's (10% is Q 0.72); **Env amt** −100 to +100%, how far the filter envelope moves
  the cutoff (8 octaves at full); **Key track** 0 to 100%: at 100% the cutoff follows the key
  note for note from C5 (key 60), so high notes stay as bright as low ones; **Drive** 0 to 100%:
  each note pushed into a soft clip after the filter (the Distortion effect's antialiased tanh,
  per note, so a chord's notes do not distort each other), warmer and grittier as it rises,
  the Distortion's Auto gain taking back what the clipping adds so it changes the sound more
  than its loudness (Saw Lead plays about 1.6 dB quieter at 50% and 2.5 dB at 100%); at 0% (the
  default) the sound passes untouched.
- **Filter env** and **Amp env**: **Attack**, **Decay** and **Release** each 1 ms to 10 s,
  evenly in decades (10 ms at a quarter of the knob, 100 ms at half, 1 s at three quarters), and
  **Sustain** 0 to 100%. The segments are exponential curves; the attack reaches full level in
  exactly its time, the release falls to silence in finite time and frees the voice. The amp
  envelope starts at 1 ms, 100 ms, 100% and 100 ms; the filter envelope at 1 ms, 100 ms, 0% and
  100 ms.
- **LFO**, one for the whole synth, running from Play: **Rate** 0.01 to 20 Hz evenly in
  octaves (2.99 Hz by default); **Shape** Sine, Triangle, Square or Saw; **→ Pitch** 0 to 12
  semitones of vibrato; **→ Cutoff** 0 to 100%, up to 4 octaves either way. The mod wheel adds
  up to a semitone of the LFO to the pitch, on top of **→ Pitch**.
- **Play**: **Glide** 0 to 1000 ms: a note started while another is held slides to its pitch
  from where the held note's pitch is, in a straight line in semitones (so every interval takes
  the glide's time); separate notes do not slide. **Volume** −48 to +6 dB (−6 by default).
  **Vel→level** 0 to 100%: at 100% a note's level follows its velocity all the way, at 0% every
  note plays full. **Vel→cutoff** 0 to 100%: softer notes are darker, by up to 4 octaves at the
  softest.

How it plays:

- **16 voices.** A note past them takes the voice that started first, without a click: the
  stolen voice keeps its filter and starts its envelopes from where they were.
- **Free-running oscillators.** A voice's waves run on from where its last note left them, as an
  analog synth's do, and Stop sets each voice to a point of the cycle of its own: the notes of a
  chord do not start in step (their first cycles pile up less), and a note played twice does
  not sound exactly the same twice.
- **Pitch bend** bends ±2 semitones (the window's Bend applies to samplers and sets, not to the
  synth). The **sustain pedal** holds the notes let go of until it lifts, as for every
  instrument.
- **Stop** silences it at once, as it does a plugin instrument.

**Presets**: the Preset list in the Presets module under the right deck lists the factory
presets, then yours.

- **Factory presets**: Init (every default: a saw, the filter open, attack 1 ms, decay 100 ms,
  sustain 100%, release 100 ms), Saw Lead, Square Bass, Sub, Pad, Pluck, Keys and Brass, built
  into the program.
- **Loading** one sets every setting at once, one undo step ("Load preset"): Undo gives back the
  sound before. While the synth sounds as a preset, the list shows that preset; once a knob
  moves it shows "Choose a preset".
- **Save preset…** opens a name box in its module: type a name and press Enter or Save. Names
  keep letters, digits, spaces, `-` and `_` (anything else is left out); a factory preset's name
  is refused, and a name you have used asks "Replace it?" first (Replace or Cancel). Esc closes
  the box.
- **Where**: each of your presets is a file, `~/.local/share/dawsome/synth-presets/<name>.json`
  (`%LOCALAPPDATA%\dawsome\synth-presets\` on Windows), listed in every synth channel of every
  project. The file holds `{"synth":"dawsome.synth","name":…,"params":{…}}` with only the
  settings that differ from their defaults, so a preset keeps sounding the same after a later
  version adds settings.

**Files**: a project with a synth channel is written as format version 9. Versions without the
synth refuse to open it, by its version, rather than open it without the synth's sound. In a
DAWproject export the synth is an instrument track with its notes and no device (the other
program plays them with an instrument of its own).

## What is included

- **Instrument sets**: two upright pianos (Upright Piano, and Upright Knight, re-voiced from
  VCSL's Knight recordings), four electric pianos (Electric Piano, and Dreamy EP, Turismo EP and
  Reed Piano, made for Dawsome by physical models, from no recording), a bass guitar, electric
  and synth basses, acoustic and electric guitars, a drawbar organ, a choir pad and a sweep pad,
  strings, a marimba, and acoustic and electronic drum kits, from the dawsome-kit repository
  (about 347 MB, at the commit `locks/kit.lock` pins); and seven electronic instruments made for
  Dawsome: two 808 basses, a synth bass, a lead, a pluck and two pads.
- **Starter kit**: twelve drums made for Dawsome (808 and 909 kicks, snare, clap, closed and
  open hi-hats, three toms, rim, cowbell, crash), built into the app.
- **One-shots and loops**: a folder of sounds the Samples section lists.
- **Dawsome Synth**: Dawsome's own synthesizer, with eight factory presets.
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

- **The toolbar's right end** holds the six panel buttons, then File, Edit and Help; what goes
  wrong (a missing sound, audio off) is said between the computer keyboard's switch and the
  panel buttons. The tempo reads one decimal, more only when set (120.0, 120.5, 120.125; type
  up to three), and it, the time signature and the position stand centred in their boxes.
- **Time signature**: the box right of the tempo, "4/4" to start: beats a bar over the beat's
  note. Drag it up or down, or scroll over it, for one beat more or less (1 to 24); hold Shift
  for the beat's note instead (4, 8 or 16: a quarter, an eighth or a sixteenth); click it to
  type one ("3/4", "6/8", "7/8", "12/8", "15/16"; Enter sets it, Esc leaves it). The tempo
  stays in quarter notes a minute whatever the signature, so 6/8 at 100 BPM clicks 200 eighths
  a minute, and a step stays a 16th. The signature sets where the bars and beats fall: the rack's
  steps and groups, the playlist's and the piano roll's grids and their Beat and Bar snaps, a
  pattern's length, the position (bar : step : tick with the bar's own steps), the metronome
  (a click every beat, a bar's first louder), the count-in (one bar), and what the exports and
  plugins are told. Changed, it changes no note and no clip: a pattern grows to the whole bars
  that hold every note it played, never shorter, and a clip keeps its length (one of a pattern
  now shorter plays it more times). It is one undo step, and the change is heard at once while
  playing. The box sets the signature the song starts in; while the song plays past a change
  (below), it shows the one there.
  - **A pattern's own**: right-click the pattern's name and choose Time signature… for its
    list: "Project's (4/4)" (it follows the project's), 3/4, 6/8, 5/4, 7/8, 12/8, 2/4, or
    Other… to type one. The pattern's steps, its piano roll's beats and its length are then its
    own, wherever its clips lie, and every clip of it follows (Unlink a clip first for a copy in
    another meter); it grows to the bars that hold its notes, never shorter. While it has its
    own, the rack's header shows it right of Bars ("3/4"; click it for the list). Played alone in
    pattern mode, its bars count the position, the metronome and the count-in; in the song the
    song's bars do. A clip whose pattern counts in another signature than the song where the
    clip starts says so after its name ("Verse · 3/4"), and its menu (Ctrl+right-click) offers
    Use the time signature here, which gives the pattern the song's signature there.
  - **Changes along the song**: right-click the ruler's numbers at a bar and choose a signature:
    from that bar on the song counts in it, and a flag with the signature stands in the ruler
    where the bar's number was. Right-click a flag to change it or remove it; drag it along the
    bars to move it (it lands on the bar line at or before the pointer). Bars after a change
    keep their numbers counting on; the playlist's lines, its Bar and Beat snaps, the position,
    the metronome, Record's count-in (one bar of the signature where it starts), the exports and
    the plugins all follow each change. A change sits on a bar line of the signature before it:
    when an earlier one moves, the changes after it settle onto their new bar lines (down to the
    one at or before them, or up when that would be the earlier change itself), and one undo
    puts every one back. At bar 1 the menu sets the signature the song starts in.

  A project with a signature other than 4/4, a pattern with its own or a change along the song
  is written as format version 10, which versions before 0.1.4 refuse to open rather than play
  in 4/4; a 4/4 project's file is as before.
- **Help** (at the right end of the toolbar, or F1) opens a guide for someone new to making music:
  what each part of the window is for and how to use it, a section a page, with pictures of the
  app.
- **The demo**: File → Open demo opens a short song made of the app's own synthesized sounds.
  A fresh install also lists it under the recent files.
- **A new document** starts with one pattern, an empty sampler channel, 8 playlist tracks and 16
  mixer inserts. "+ Instrument…" under the rack's rows adds a channel: an instrument set, a
  starter kit sound, a plugin or a sound file of yours ("Sample file…"); drop a file on a
  sampler row to change its sound, or anywhere else on the rack for a new channel.
- **Panels** open and close with the digits 1 to 5, or F5 to F9 (below), and share the window.
  Each keeps room for what it shows (the mixer every strip whole, 235 px); in a window too short
  for all of them the piano roll first shows fewer rows (down to five, its velocity lane always
  whole), then the panels scroll up and down. Drag the line between two stacked panels up or
  down to share the height between them (its grip lights orange while held); double-click it to
  put both back.
- **A panel in its own window**: hold the line over the channel rack, the playlist, the piano
  roll or the mixer and drag sideways instead (40 px, more than up or down): the line goes back
  where it was, lights orange along its length, an outline of a window follows the pointer and
  the hint bar says "release to pop the panel out". Let go there and the panel under the line
  leaves the stack for a window of its own, opened where you let go, as large as it was (or as
  you last left that window); the other panels share the main window as though it were closed.
  Drag back within the 40 px to resize instead, or press Esc to put the line back and do
  nothing. The window shows the panel exactly as in the stack, its menus inside it, and keeps
  above the main window as the other windows beside it do; drag it anywhere, to another screen
  too. While it has the keyboard, the keys act on that panel. Its close button puts the panel
  back in the stack, where it stacks and at the height it had. The panel's key or toolbar
  button brings its window to the front, or, pressed in that window, puts the panel back. The
  top panel of the stack has no line over it, so it stays docked; one panel per window, the
  Library never. A panel's own window is at least 480 px wide, the piano roll's at least 608, so
  its strip, folded all the way, still shows every control for most names. Its size and place
  are kept, and a panel out in its own window when Dawsome closes opens that way at the next
  start. Nothing dragged between windows: a clip or a file dropped on a panel's own window lands
  nowhere.
- **The pop-out button**: the small dim grip (two short lines, the dividers' grip in small) at
  the right end of the channel rack's, the playlist's, the piano roll's and the mixer's header
  (it lights as the pointer comes over it) does the same in one press:
  the panel opens in its own window where that window last was (the first time, where your
  system puts a new window, 1000 by 480). It works for a panel alone in the stack too, which has
  no line over it to drag. In the panel's own window the button is not there: close the window,
  or press the panel's key or toolbar button in it, to put the panel back.
- **Settings** (6 or F10, or the cog among the toolbar's panel buttons) opens a window of its
  own beside the main one, with three pages, chosen by the tabs along its top: **General** (MIDI
  input, the typing keyboard's octave, the hints, the zoom, the audio output and input, the
  input latency), **Keys** (the note layout and every shortcut) and **Theme** (the theme, the
  accent and the piano roll's notes' colour). It opens at a fixed size, about half the main
  window's width and two thirds of its height as the main window is at that moment, and cannot
  be resized; it opens again where it was last. In a wide window the General page uses the
  width: two columns from 800 pixels (the audio input and the input latency on the right),
  three from 1200 (the audio output in the middle). Its pages scroll, and Esc, its close button
  or the same key closes it from any page; it opens again on the page shown last.
- **Reset to defaults**, at the foot of Settings → General, puts the looks and the layout back as
  a first start has them: the zoom, the theme, the accent and the notes' colour, the open panels
  (the Library, the channel rack, the playlist and the mixer, none in a window of its own), the
  heights the
  lines between panels were dragged to, the playlist's and the piano roll's zoom, the playlist's
  lane height, the piano roll's Follow, the Library's folded groups and the hint bar. Your MIDI
  inputs, audio devices, input latency, keys, the piano roll's Loop and the other windows'
  places stay. It asks first:
  the first press turns it into "Press again to reset" for three seconds (or until the pointer
  leaves it), and only a second press resets.
- **Zoom** (Settings → General → Display → Zoom, or Ctrl+= and Ctrl+-, Ctrl+0 back to 100 %) makes the
  text and controls of every window larger or smaller, the main window and every window beside
  it alike, on top of the display scale your system sets: 50 % to 200 %, a step of 10 % at a
  time, with 75, 125 and 175 % between (125 and 175 % match the display scales Windows offers).
  The toolbar says "Zoom 125 %" for two seconds after each step; a double-click on the value in
  Settings sets 100 %. The zoom is kept for the next start, and each window keeps its size on
  the screen: a larger zoom shows less in it, larger. Plugins' own windows keep their size: the
  zoom does not reach them (they are told the desktop's scale as they open).
  Every window draws its text in the font Dawsome ships (Inter), the same on every system, in a
  few sizes: 12 px for labels, buttons and list rows, 13 for running text and menus, 14 for the
  panels' titles ("Channel Rack", "Playlist", "Mixer", "Library") and the piano roll's channel
  name, 16 for titles and the toolbar's readouts, 11 for numbers; each panel's first control
  stands 48 px after its title. Controls take a press 24 pixels across
  (Settings' rows are 24 px with 14 px boxes).
- **Theme** (Settings → Theme) sets the colours of every window as one set, one of five, their
  keys in one row, each with a swatch of its set (its panel and its accent, two small squares,
  edged in the theme's light edge colour so a dark swatch shows on a dark page) before its name:
  **Slate**, the default, darker and cooler than Graphite, its text brighter; **Graphite**, the
  look Dawsome had first; **Gilt**, gold leaf on deep chocolate (the accent, the lit edges, the
  knob rings and the fader caps in gold; an audio clip's body a shade lighter than elsewhere, so
  it reads apart from the brown lanes); **Garnet**, rose gold on deep plum; **Fjord**, ice blue on deep navy. A theme changes the panels, the text, the lines, the piano keys and the
  accent. Every theme writes labels of live information ("Bars", "Snap", group names, hints) in
  a grey that reads at 4.5:1 or more on its panels, and what cannot be used now (a button with
  nothing to do, an unplugged device) in a dimmer one; the channel colours, the clips,
  the notes, the lights, Record and the meters keep their colours in every theme, so what they
  tell you stays the same. Every window changes at once, the ones beside the main window and the
  Help window too, and the theme is kept for the next start. A theme this version does not know
  (one a newer version kept) shows Slate and says so in the toolbar.
- **Colours** (Settings → Theme, under the theme's keys): a menu over one colour wheel chooses
  what the wheel sets, **Accent** (as it opens) or **Piano roll notes**.
- **Accent** (Settings → Theme → Colours, the menu on Accent) makes the accent, the colour
  of what is lit and selected (lit buttons, ticked boxes, the playhead, selections, slider fills), any
  colour you like, in every theme. Drag round the ring of the colour wheel for the hue, and in
  the square inside it for how strong (to the right) and how bright (up) it is; every window
  follows as you drag, and the colour is kept when you let go. Or type a colour as #RRGGBB in the
  box beside the wheel and press Enter; anything else is refused, and the line under the row says
  how to write one. The swatch left of the wheel shows the accent drawn now. **Theme's own** puts
  each theme's accent back (its key is lit while it shows). A colour that is hard to see on the
  theme's panels, under 3:1 contrast, gets the note "Low contrast on this theme's panels" under
  the row, and is still yours to keep. The channel colours, the clips, the notes, the lights,
  Record, the loop range and the meters keep their colours whatever the accent.
- **Piano roll notes** (Settings → Theme → Colours, the menu on Piano roll notes) draws the piano
  roll's notes and their velocity stems in a colour of your own instead of each channel's: the
  wheel, the hex box and the swatch work as for the accent, the notes following as you drag.
  The other channels' notes shown faintly, the rack's steps and the playlist's clips keep their
  channel colours. **Channel's own** gives each channel its colour back. A colour under 3:1
  contrast on either of the roll's row tones gets the note "Low contrast on the piano roll's
  rows", and is still yours to keep. The colour is kept for the next start.

## Keys

| Key | Does |
|-|-|
| Space | play / stop |
| Shift+Space | the piano roll's Play / Stop (while the roll has the keys) |
| L (Shift+L while the computer keyboard plays notes) | pattern or song mode |
| 1, 2, 3, 4, 5, 6 | channel rack, playlist, piano roll, mixer, Library, the Settings window |
| F5, F6, F7, F8, F9, F10 | playlist, channel rack, piano roll, Library, mixer, the Settings window |
| F1 | the Help window |
| Ctrl+= (or Ctrl+Plus), Ctrl+-, Ctrl+0 | zoom every window a step larger, a step smaller, back to 100 % |
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
| R (Shift+R while the computer keyboard plays notes) | arm recording |
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
dawsome render song.json bass.wav --pattern Verse --channel Bass --once
                                                      # one channel's notes of a pattern, once
dawsome scan                                          # find CLAP and VST3 plugins
dawsome smf-import song.json melody.mid out.json --pattern 1 --channel 2
dawsome smf-export song.json melody.mid --pattern 1 --channel 2
dawsome export-midi song.json song.mid                # the whole song, a track per channel
dawsome export-stems song.json out/ --bits 24         # out/song stems/01 Track 1.wav ... master.wav
dawsome export-dawproject song.json song.dawproject   # for Bitwig Studio, Studio One, Cubase
```

`dawsome` with no arguments prints every option. A render plays the project through the same
engine path the app plays it through, so the file holds exactly what you hear. `--pattern NAME`
renders that pattern instead of the first; `--channel NAME` plays only that channel's notes of
it, as the piano roll's Play does (its sound through the mixer as ever), and `--once` plays it a
single time, so the file is one pass long. `--bars` counts bars of the meter that plays: the
song's, across its changes, with `--song`, a pattern's own otherwise (a 3/4 bar is 288 ticks, a
4/4 one 384).
`dawsome-app --help` lists the app's own options, among them those for scripted runs
(screenshots, `--window-size`): `--window-position <x>,<y>` opens the main window with its
top-left corner there, in the desktop's logical pixels (on another monitor, say), on X11 and
Windows only; on Wayland, where the compositor places every window, it is ignored with a line
saying so.

## Taking a song to another program

No music program opens another's own project files, so File has three exports that other
programs read, each followed by a report (shown, and written beside the export as
`<file>.report.txt`) of what did not carry over:

- **Export song as MIDI…**: one MIDI file (type 1) every program opens: the tempo, the time
  signature and each change of it along the song, and a track
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
  volume, pan and mute, the tempo, the time signature and its changes along the song, the pattern clips as note clips, the audio clips with their files packed
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
- Your Dawsome Synth presets: `~/.local/share/dawsome/synth-presets/`, a file each.
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
  `DAWSOME_WINDOW_SYSTEM=wayland`, where the pop-out windows, a panel's own window among them,
  can go under the main one: a Wayland desktop decides how windows stack). A
  plugin's own window, CLAP's or VST3's, opens through X11 in either case. As it opens, Dawsome
  tells the plugin the scale its window is drawn at: the desktop's, the one Dawsome's own windows
  get (under X11 one scale for every screen, about 1.7 for a 4K screen set to 170 %), so a plugin
  that takes it is drawn as large as the rest of the desktop; one that does not keeps its own
  scale. Dawsome's zoom (Settings → Zoom) does not apply to plugin windows: they are the plugin's
  own drawing, not Dawsome's. The typing keyboard keeps playing while a plugin's window has the
  keys: each key plays the note it plays in Dawsome's own windows and still reaches the plugin,
  and the plugin keeps its shortcuts (chords with Ctrl, Alt or the Windows key, the Super key on
  Linux) and Esc. A plugin without a window opens its parameter window. A
  plugin installed as both CLAP and VST3 is listed once, as CLAP (a project naming its VST3 build
  still plays it). VST3 on Windows, its windows included, is built but not yet tried on a Windows
  machine. No LV2 or LADSPA. Plugin latency is shown, not compensated: a VST3 plugin's parameter
  window, its rack or effect hint and a render name the delay it reports.
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
- One tempo per project. A MIDI file's own time signature (and its changes) is not applied when
  it is imported.
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
- The built-in effects and Dawsome Synth take each new setting at once, with no smoothing: fast
  automation of a filter's cutoff or a distortion's drive can be heard stepping. The synth's
  filter is not turned down as its resonance rises (the Filter effect's is), so a high
  resonance makes it louder at the cutoff. The delay's time is the
  exception: a change fades from the old echo to the new over 30 ms instead of jumping, so
  turning Time while it sounds does not click.
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
