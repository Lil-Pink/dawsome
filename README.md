# Dawsome
<img width="1600" height="847" alt="main-window" src="https://github.com/user-attachments/assets/319e4ecd-dcf2-41bd-bc3a-38e128f55859" />


Dawsome is a free, open-source music making software for Linux and Windows. It's pattern focused: you build short loops in a step grid and a piano roll, drop them on a playlist to make a song, mix it, and export it. It's written in Rust, it's small, and it comes with its own instruments and a bundled synth, so you can start making a beat without missing a BEAT (bud-um-tiss).

To be completely transparent, this was mainly agentically coded, human tested. I'd like to think the way I use agentic teams for building projects is different from most, but I'm sure that's what every agentic coder would like to think lol. But hey, it's free to use as you like.


Dawsome is not affiliated with Dawesome, the plugin company.

## What you can actually do with it

Open it, drag a drum from the Library onto the rack, click some steps, and you have a beat. Double-click an instrument to play it from your computer keyboard (Tab is the sustain pedal) or a MIDI keyboard, and draw or record notes into the piano roll. Make a few patterns, switch to Song mode, lay them out on the playlist, make a song or two.

Some of the features that I think are kinda nice:

- **Create set patterns while also allowing for unique adjustments.** Put the same pattern on the playlist five times, edit one copy, all five change. If you want to change a note on copy 4, one click unlinks it into its own, unique copy,
<img width="680" height="168" alt="chain" src="https://github.com/user-attachments/assets/7a00ddb0-5649-4aad-a7c9-bd4c2c646a18" />


- **A reactive piano roll.** Play any instrument, sample, or plug-in channel on the piano roll with real time input feedback. Keyboard is automatically set up as your MIDI with keys set to match a keyboard.
  <img width="680" height="496" alt="piano-roll-small" src="https://github.com/user-attachments/assets/d28d13a1-fc30-4e26-9bf2-1042c4a61041" />


- **Drag and drop audio clip imports and audio clip/sample editing.** Drag a sound file from your file manager straight onto a track. Trim it, stretch it to fit, pitch it, and open it in the sample editor to crop, fade, reverse or normalize it, without touching the original file. (Finer adjustments and sample tuning will be added in the future.)
  <img width="1800" height="880" alt="sample-editor" src="https://github.com/user-attachments/assets/0f19fbee-f225-4273-9cd7-4c1e078541b8" />
  
- **Recording in loops.** Hit record in Pattern mode and your notes land in the pattern as it loops. In Song mode a take grows as you play, if you feel like recording in one take. And if you were just noodling and played something good, Capture (Ctrl+Shift+R) turns the last 30 seconds into notes after the fact.
- **A mixer.** Right-click a channel, pick "Mixer insert…", and it gets its own strip with a three-band EQ, delay, reverb, a compressor and a saturator ready to go.
- **Plugins.** CLAP and VST3 instruments and effects both work, their windows open in Dawsome. Plugins without a window of their own get a generated one with every parameter automatable.
- **Virtual instruments in the box.** A piano, electric piano, three basses, acoustic and electric guitar, organ, two pads, strings, marimba and two drum kits, all made by me or public domain (CC0), plus Surge XT as a bundled synth. Bring your own SFZ instruments and sample folders.
- **Easy to use/grasp UI windows** - All UI windows are toggleable and adjustable. Creating an easy to use and easy to digest audio workstation. (Some of this UI is intended to cleaned up in the future, some small things bother me about it) 
- **Export anywhere.** WAV (16, 24 or 32-bit float), MP3, MIDI, stems, and DAWproject so a song can move to another program if needed.
- **A nice little Help guide** inside the app (F1) that walks through the entire app, with pictures :)
  

## How to Use/ Install

Grab the zip for your system from the Releases page, unpack it anywhere, and run `dawsome-app`.

On Linux there's a `scripts/install.sh` in the source if you'd rather have it in your menu.

The Windows build is cross-compiled from Linux and tested by hand on a mid-range Windows laptop (Intel Core Ultra 5 226V, Intel Arc 130V integrated graphics, 16 GB of memory). If anything is off on Windows builds, let me know and I will address it.

The full reference, every panel and setting, is in [docs/manual.md](docs/manual.md).

## What's missing (and what's next)

Compared to the paid DAWs, Dawsome still lacks: swing/groove, a big bundle of built-in effects (only six and one synth), audio warping or beat detection, a clip launcher, time signatures other than 4/4, take comping, VST2, or a Mac build. Most of that is coming, though macOS probably not and VST2 definitely not (Steinberg no longer licenses it to anyone).

Roughly in the order I plan to do them:

1. **Swing.** A knob for it on the step grid and in quantize.
2. **More built-in effects and a simple synth.** Chorus, phaser, flanger, distortion, a filter, a gate, stereo width.
3. **UI Clean-up** Because I built the app in Linux and on a 4k monitor, I realize the UI is not uniform across OSs/monitors. Plan to clean tidy this up and make the UI in general easier on the eyes.
4. **Sample chopping** and finer, precision sample editing.
5. **Meters and a spectrum view** on the master, with loudness so you can check a mix without a plugin.
6. **Piano roll helpers.** Chord, arpeggiate, strum, and a preset browser for plugin patches.
7. **Audio warping and tempo detection**, so a dropped loop follows the song on its own.
8. **Time signatures other than 4/4.**
9. **Take comping.**
10. **A clip launcher** for playing live.
11. **macOS**, if I get my hands on a Mac or receive outside help.

## If you'd like to play with the code

The engine is its own crate, and offline rendering and live playback go through the exact same function, so what the tests render is what you hear. The audio thread never allocates, and a test proves it. Expected values in the tests come from Python generators that compute the answer independently of the Rust code, so if you change the sound you find out straight away. There's a command-line renderer too, which is handy for scripts.

`CONTRIBUTING.md` has the build and the one check command that runs everything. Effects would be nice first contributions (an effect is one type implementing one trait, and there are two Airwindows ports to copy from) and anything on the list above. If you find a bug, one line with what you did, what you expected, and what happened is plenty.

## License

The programs are GPL-3.0-or-later. Every sound that ships is CC0. Surge XT keeps its own license and its source is attached to every release. `licenses/NOTICES.txt` has the license text of every library built in.

What you make with Dawsome is yours.
