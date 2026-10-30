# Test fixtures

Projects, sounds and known answers the tests read. A `pN_` file belongs to one feature, its
known answers written by `gen_pN_expected.py` (standard library only; `scripts/check.sh` runs
each `--check` mode that exists, so a fixture differing from its generator fails), and the tests
of that feature carry the same `pN_` prefix.

| Prefix | Feature |
|-|-|
| `p2` | the channel rack and the player: steps on the tick-to-sample rule, edits and undo |
| `p3` | the piano roll: note editing, pitch from the key, the device resampler |
| `p4` | the playlist: song mode, clips, the loop |
| `p5` | the mixer: inserts, sends, the built-in effects |
| `p7` | CLAP plugins (the test plugin's formulas) |
| `p8` | audio clips, recording, automation, the Library's samples |
| `p9` | MIDI input and controller links, MIDI file import and export, the ported effects' reference output (`ref/`) |
| `p10` | the Library: sounds and plugins dropped on the rack, device links |
| `p11` | instrument sets (SFZ) |
| `p12` | the electronic instrument sets |
| `p13` | sound settings |
| `p14` | the metronome |
| `p15` | a sound dropped on a channel's header |
| `p16` | the external input channel |
| `p17` | recording: note and audio takes, Capture |
| `p18` | export for other programs: MIDI, stems, DAWproject (`dawproject/` holds the schema) |
| `p19` | MIDI out |
| `p20` | clip link (`p20_five_clips.json`, worked by hand in the tests) |
| `p22` | the sustain pedal, pitch bend and the mod wheel |
| `p23` | input latency compensation |
| `p24` | the master limiter and loop wraps |
| `p25` | the sampler's interpolation and the export's dither |
| `p26` | One note at a time |
| `p27` | the VST3 host: scan, render, presets, latency |
| `p28` | VST3 in the app |
| `p29` | VST3 plugin windows |
| `p30` | a sound's edits (the sample editor's engine and media) |

The rest: `f1`..`f4` the first projects (`f1_four_on_floor.json` is the README's render
example); `a6_starter.json` the starter document; `k6_forty_tracks.json` a 40-track project for
the playlist's scroll probe; `hold2000.wav`, `impulse.wav`, `ramp4.wav`, `sine*.wav` short signals
with known samples; `resample_oracle.py` the resampler's oracle; `toolbar_narrow_layout.txt` the
toolbar laid out at a narrow width; `browser/` a sample tree for the Library (its `notes.txt` is the file
that is not audio); `instruments/` and `sound/` sets the tests install.
