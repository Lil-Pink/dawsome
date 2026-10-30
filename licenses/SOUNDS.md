# The sounds Dawsome ships

Every sound the programs ship is dedicated to the public domain under CC0 1.0 Universal
(CC0-1.0) at its source, or made for Dawsome and dedicated so: use, change, sell and
share it, in songs or in software, with no credit due. The bundled plugin's own factory
data is the one exception, under the plugin's license (the last section). This file says
where each comes from. `scripts/license-check.py --sounds` writes it from the manifests,
and `scripts/check.sh` fails when it is out of date.

## The starter kit and the electronic instrument sets

Made for Dawsome, CC0-1.0. `assets/sounds/kit.json`
states each file's SHA-256 and how it was made; the drums are the starter kit
(`assets/sounds/`), each pitched sound an instrument set of its own
(`assets/instruments/`).

| Sound | Where | Made after |
|-|-|-|
| Kick 808 | starter kit | Mutable Instruments Plaits, plaits/dsp/drums (MIT, Emilie Gillet) AnalogBassDrum; Werner, Abel & Smith, TR-808 bass drum (DAFx-14) and cymbal (ICMC-SMC 2014) models |
| Kick 909 | starter kit | Mutable Instruments Plaits, plaits/dsp/drums (MIT, Emilie Gillet) SyntheticBassDrum |
| Snare | starter kit | Mutable Instruments Plaits, plaits/dsp/drums (MIT, Emilie Gillet) SyntheticSnareDrum |
| Clap | starter kit | Clark, Nord Modular book (percussion); Noise Engineering, percussion synthesis part 3 |
| Hat closed | starter kit | Werner, Abel & Smith, TR-808 bass drum (DAFx-14) and cymbal (ICMC-SMC 2014) models; Mutable Instruments Plaits, plaits/dsp/drums (MIT, Emilie Gillet) HiHat |
| Hat open | starter kit | Werner, Abel & Smith, TR-808 bass drum (DAFx-14) and cymbal (ICMC-SMC 2014) models; Mutable Instruments Plaits, plaits/dsp/drums (MIT, Emilie Gillet) HiHat |
| Tom low | starter kit | Rossing, membrane modes; 808-style pitch sweep |
| Tom mid | starter kit | Rossing, membrane modes; 808-style pitch sweep |
| Tom high | starter kit | Rossing, membrane modes; 808-style pitch sweep |
| Rim | starter kit | modal synthesis (decaying sinusoids) |
| Cowbell | starter kit | Reid, Synthesizing Cowbells (and Claves), Sound On Sound Synth Secrets (2002) |
| Crash | starter kit | Werner, Abel & Smith, TR-808 bass drum (DAFx-14) and cymbal (ICMC-SMC 2014) models |
| Bass 808 C1 | set `bass-808-c1` | Werner, Abel & Smith, TR-808 bass drum (DAFx-14) and cymbal (ICMC-SMC 2014) models; 808 bass practice (Attack Magazine) |
| Bass 808 C2 | set `bass-808-c2` | Werner, Abel & Smith, TR-808 bass drum (DAFx-14) and cymbal (ICMC-SMC 2014) models; 808 bass practice (Attack Magazine) |
| Bass synth C2 | set `bass-synth-c2` | Surge XT 1.3.4 (GPL-3) rendered headless with a patch written for this kit |
| Lead C4 | set `lead-c4` | Surge XT 1.3.4 (GPL-3) rendered headless with a patch written for this kit |
| Pluck C4 | set `pluck-c4` | Karplus-Strong, extended (Jaffe & Smith, CMJ 1983) |
| Pad warm C3 | set `pad-warm-c3` | PADsynth (Nasca Octavian Paul, public domain) |
| Pad bright C3 | set `pad-bright-c3` | Surge XT 1.3.4 (GPL-3) rendered headless with a patch written for this kit |

## The instrument sets of dawsome-kit

The dawsome-kit repository, installed with the programs, at the commit `locks/kit.lock` pins:
`081b109502308476dc09bbf942bf0f37f25bdf48`. Its `MANIFEST.json` names the source, the license page
and the SHA-256 of every one of its files, and its `tools/check.py --manifest` holds the files
to it. Files counts every file of a set: its sounds, its .sfz and its `set.json`.

| Set | Name | Maker | Source | Files | License | License page |
|-|-|-|-|-|-|-|
| `bass-electric` | Electric Bass | Andrea Biasior, for FreePats | FreePats Bass Guitar YR, finger variation, version 2019-09-30 (SFZ + FLAC) | 14 | CC0-1.0 | https://freepats.zenvoid.org/ElectricGuitar/clean-electric-bass.html |
| `bass-guitar` | Bass Guitar | Karoryfer Samples: a Killer KB-Simmony bass guitar played and mapped by D. Smolken | Karoryfer Fashionbass, its notes and clean mapping (fingers, neck pickup), at a pinned commit | 288 | CC0-1.0 | https://github.com/sfzinstruments/karoryfer.fashionbass/blob/0703973b300a6bacb837602be9b588ab64732a72/LICENSE |
| `bass-synth` | Synth Bass | FreePats, recorded from the Hexter synthesizer | FreePats Synth Bass #2, version 2021-04-05 (SFZ + FLAC) | 11 | CC0-1.0 | https://freepats.zenvoid.org/Synthesizer/synth-bass.html |
| `drums-acoustic` | Acoustic Drums | Karoryfer Samples | Karoryfer Big Rusty Drums, stick hits, close and overhead microphones, at a pinned commit | 86 | CC0-1.0 | https://github.com/sfzinstruments/karoryfer.big-rusty-drums/blob/f07ce00df34a46b6b08375be56fe116cf15782bc/LICENSE |
| `drums-electronic` | Electronic Drums | Sonic Pi contributors; each sound's Freesound uploader (listed in etc/samples/README.md) | Sonic Pi sample set (etc/samples), at a pinned commit of the dev branch | 39 | CC0-1.0 | https://github.com/sonic-pi-net/sonic-pi/blob/df6f17e4625b0451fa2af8accf2e1e641c7ad458/LICENSE.md |
| `ep` | Electric Piano | Roberto, for FreePats, recorded from the Hexter synthesizer (the Yamaha DX7's 'E. Piano 1' patch) | FreePats FM Synthesized Piano #1, version 2019-09-16 (SFZ + FLAC) | 39 | CC0-1.0 | https://freepats.zenvoid.org/ElectricPiano/synthesized-piano.html |
| `guitar-acoustic` | Acoustic Guitar | dawsome-kit, from Karoryfer Shinyguitar's recorded archtop (CC0, played by D. Smolken) and the kit's modelled steel string and body | dawsome-kit's acoustic guitar: Karoryfer Shinyguitar's recorded pluck driving a modelled steel string and body | 105 | CC0-1.0 | https://creativecommons.org/publicdomain/zero/1.0/ |
| `guitar-electric` | Electric Guitar | FreePats: a Fender electric guitar sampled direct, processed through an amplifier and effects rack | FreePats FSBS Electric Guitar Clean #1, version 2026-08-07, bridge pickup (SFZ + FLAC) | 33 | CC0-1.0 | https://freepats.zenvoid.org/ElectricGuitar/clean-electric-guitar.html |
| `mallets` | Marimba | Versilian Studios LLC | Versilian Community Sample Library (VCSL), at a pinned commit | 32 | CC0-1.0 | https://github.com/sgossner/VCSL |
| `one-shots` | One-shots and Loops | Sonic Pi contributors; each sound's Freesound uploader (listed in etc/samples/README.md) | Sonic Pi sample set (etc/samples), at a pinned commit of the dev branch | 35 | CC0-1.0 | https://github.com/sonic-pi-net/sonic-pi/blob/df6f17e4625b0451fa2af8accf2e1e641c7ad458/LICENSE.md |
| `organ` | Drawbar Organ | Roberto, for FreePats, recorded from the setBfree synthesizer | FreePats Drawbar organ emulation, version 2019-07-12 (SFZ + WAV) | 18 | CC0-1.0 | https://freepats.zenvoid.org/Organ/electric-organ.html |
| `pad-choir` | Choir Pad | 快乐爱的小精灵 (Elf of Happy and Love), for FreePats, made with ZynAddSubFX | FreePats Synth Pad Choir, version 2020-05-16 (SFZ + FLAC) | 14 | CC0-1.0 | https://freepats.zenvoid.org/Synthesizer/synth-pad.html |
| `pad-sweep` | Sweep Pad | FreePats, made with ZynAddSubFX / Yoshimi | FreePats Sweep Pad, version 2019-08-13 (SFZ + FLAC) | 15 | CC0-1.0 | https://freepats.zenvoid.org/Synthesizer/synth-pad.html |
| `piano` | Upright Piano | FreePats: recorded by Gonzalo and Roberto, edited by Roberto | FreePats Upright Piano KW, version 2022-02-21 (SFZ + FLAC) | 68 | CC0-1.0 | https://freepats.zenvoid.org/Piano/acoustic-grand-piano.html |
| `strings` | Strings | Versilian Studios LLC (Sam Gossner) and Simon Dalzell | VS Chamber Orchestra 2: Community Edition (raw WAV), at a pinned commit | 36 | CC0-1.0 | https://versilian-studios.com/vsco-community/ |

## The bundled plugin: Surge XT

Surge XT 1.3.4, the Surge Synth Team's synthesizer, is bundled as a CLAP plugin
(`plugins/` in a release zip, `lib/dawsome/clap/` installed) with its factory data in
`SurgeXTData/`: patches, wavetables, skins, tunings and presets, under the plugin's
license, GPL-3.0-or-later (`LICENSE-SURGE`); the third-party patches and wavetables of its
data, and its tutorial skins, are left out. The sounds you play and record with it are
yours, as Surge's own FAQ says: "The output of Surge XT is yours to do with as you
want!" Its data comes unchanged from the release asset
`surge-xt-portable-content-1.3.4.tar.gz` (252763750 bytes, SHA-256
`7ed24822eab69f73101d8558222979b1bceb4882a780292bb7087309b7b101bd`), which `locks/surge.lock` pins.
