# Third-party code in Dawsome

Dawsome is GPL-3.0-or-later. These parts come from others, under their own licenses, which
travel with them. `NOTICES.txt` carries the license and notice texts of every crate and C library
built into the programs (`scripts/notices.py` writes it).

## Ported source

### Airwindows: Pressure4 and Density

`crates/engine/src/dsp/airwindows/pressure4.rs` and `density.rs` are Rust ports of the Airwindows
plugins Pressure4 and Density, from github.com/airwindows/airwindows at commit
`e68f75154ed0520fc5757fec9e7792dd1747e5ee`. `tests/fixtures/ref/` holds renders of the
originals made from that commit (`tests/fixtures/ref/provenance.json` says how), which the
ports are tested against.

```
MIT License

Copyright (c) 2018 Chris Johnson

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Files from others in the tests

### DAWproject schemas

`tests/fixtures/dawproject/Project.xsd` and `MetaData.xsd` are the DAWproject 1.0 XML schemas
from github.com/bitwig/dawproject at commit `ee4dcdde75940f30e14e55401a26955a58b8322b`, unchanged
(SHA-256 `58e2fd9864772850aac3eab1f3de8693857dc5384df6d29fbc320ae1de2347cc` and
`fb3ba378271770dddbcced8990aba537de3d36ff2d58573523460a699221c99f`, which
`tests/scripts/dawproject_check.py` holds them to); `LICENSE` beside them is the repository's
license. They are never built into the programs: the checker holds the DAWproject files the
export writes to them.

```
MIT License

Copyright (c) 2020 Bitwig

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## C and C++ libraries built in

Their crates' build scripts compile them from the source the crates bundle, and both programs
link them statically.

| What | Used for | License | Copyright |
|-|-|-|-|
| Signalsmith Stretch (C++ header library, bundled by the `signalsmith-stretch` crate) | time-stretch and pitch shift of audio clips | MIT | Geraint Luff / Signalsmith Audio Ltd. (2022) |
| signalsmith-linear (bundled with it) | its FFT and vector maths | MIT | Signalsmith Audio (2025) |
| LAME 3.100 (C library, bundled by the `mp3lame-sys` crate) | MP3 export | LGPL-2.0-or-later | Mark Taylor, Takehiro Tominaga, Robert Hegemann and the LAME developers; lame.sourceforge.io |

## A plugin bundled with the programs: Surge XT

Surge XT 1.3.4, the Surge Synth Team's synthesizer, ships beside the programs: in the release
zips as `plugins/Surge XT.clap` with its factory data in `plugins/SurgeXTData/`, and installed by
`scripts/install.sh` into `<prefix>/lib/dawsome/clap/`, where the programs look for plugins. It
is a separate program, which Dawsome loads through the CLAP plugin interface as it loads any
plugin; nothing of it is built into Dawsome, and its files are the release's own, unchanged.

| What | License | Copyright |
|-|-|-|
| Surge XT 1.3.4, the CLAP plugin alone (no VST3, VST2, standalone or effects plugin), and its factory patches, wavetables, skins, tunings and presets (the third-party patches and wavetables of its data, and its tutorial skins, which carry fonts under the OFL, left out) | GPL-3.0-or-later | various authors, as described in Surge's GitHub transaction log (2018-2024); Claes Johanson at Vember Audio (2004-2018) |

`LICENSE-SURGE` holds its notice and the license as its source holds them. `locks/surge.lock` pins
the release assets the files come from, each by its SHA-256, and the complete source of the
release, `surge-src-1.3.4.tar.gz`; `scripts/fetch-surge.sh` downloads them and refuses any other
file. Each release zip names that source in its `SOURCES.md`, and a release publishes it beside
the zips. Why 1.3.4: it is built on JUCE 7 under the GPL; later Surge builds use JUCE 8, whose
open license is the AGPL.

## Rust crates

Most crates in the two programs are under MIT or Apache-2.0, or offer one of them, and are not
listed. The table lists every crate under another license, or under one in addition, with the
license its manifest declares, and a few MIT crates worth naming. `scripts/license-check.sh`
(run by `scripts/check.sh`) compares it with the crates both programs are built from on Linux
and on Windows, and fails when a crate is missing, its license or its platforms differ, or a
listed crate is no longer used.

| Crate | License | Platforms | Used for | Copyright |
|-|-|-|-|-|
| `mp3lame-sys`, `mp3lame-encoder` | LGPL-3.0 | Linux, Windows | building LAME and the Rust interface to it, for MP3 export | Douman (github.com/DoumanAsh) |
| `symphonia`, `symphonia-core`, `symphonia-bundle-flac`, `symphonia-bundle-mp3`, `symphonia-codec-pcm`, `symphonia-codec-vorbis`, `symphonia-format-ogg`, `symphonia-format-riff`, `symphonia-metadata`, `symphonia-utils-xiph` | MPL-2.0 | Linux, Windows | decoding WAV, AIFF, FLAC, Ogg Vorbis and MP3 files | Philip Deljanov and the Symphonia contributors |
| `audio_thread_priority` | MPL-2.0 | Linux | asking for realtime priority for the audio thread (through `cpal`) | Paul Adenot |
| `option-ext` | MPL-2.0 | Linux, Windows | finding the per-user folders (through `dirs`) | Simon Ochsenreither |
| `encoding_rs` | (Apache-2.0 OR MIT) AND BSD-3-Clause | Linux, Windows | the text encodings of tags in sound files (through Symphonia) | Mozilla Foundation; WHATWG (Apple, Google, Mozilla, Microsoft) for the encoding data |
| `tiny-skia`, `tiny-skia-path` | BSD-3-Clause | Linux, Windows | drawing the window without a GPU; on Linux also the window decorations under Wayland | Google Inc. (2011), Yevhenii Reizner (2020) |
| `arrayref` | BSD-2-Clause | Linux, Windows | array access inside `tiny-skia` | David Roundy (2015) |
| `kamadak-exif`, `mutate_once` | BSD-2-Clause | Linux, Windows | reading a picture's orientation in the UI toolkit's image support, which draws the Help window's pictures | KAMADA Ken'ichi (2016-2023; `mutate_once` 2019) |
| `libloading` | ISC | Linux, Windows | loading CLAP and VST3 plugins, and the graphics, windowing and JACK libraries at run time | Simonas Kazlauskas (2015) |
| `foldhash` | Zlib | Linux, Windows | hashing inside the GPU renderer | Orson Peters (2024) |
| `slotmap` | Zlib | Linux, Windows | the font database of the UI toolkit | Orson Peters (2021) |
| `unicode-ident` | (MIT OR Apache-2.0) AND Unicode-3.0 | Linux, Windows | telling identifier characters apart in the GPU renderer's shader compiler | David Tolnay; Unicode, Inc. (1991-2023) for the Unicode data |
| `hexf-parse` | CC0-1.0 | Linux, Windows | reading hexadecimal float numbers in the shader compiler | Kang Seonghoon (public domain dedication) |
| `clipboard-win`, `error-code` | BSL-1.0 | Windows | the clipboard | Douman (github.com/DoumanAsh) |
| `midly` | Unlicense | Linux, Windows | reading and writing Standard MIDI Files | negamartin (public domain) |
| `midir` | MIT | Linux, Windows | MIDI input ports (the ALSA sequencer, WinMM) | Patrick Reisert and the RtMidi contributors (2015) |
| `wmidi` | MIT | Linux, Windows | parsing live MIDI messages | wmedrano |
| `signalsmith-stretch` | MIT | Linux, Windows | the Rust interface to Signalsmith Stretch | Colin Marc (2024) |
| `vst3` | MIT OR Apache-2.0 | Linux, Windows | the VST3 plugin interfaces the programs' VST3 host calls: Rust bindings generated from the VST 3 SDK's `pluginterfaces` 3.8.0, which Steinberg publishes under the MIT license (its text below) | Micah Johnston (the bindings, 2025); Steinberg Media Technologies GmbH (the interfaces, 2025) |

`mp3lame-sys` and `mp3lame-encoder` declare `LGPL-3.0`, the deprecated SPDX name of
LGPL-3.0-only; their license files hold the GNU Lesser General Public License version 3. The
MPL-2.0 crates are used unchanged; their source is on crates.io under each crate's name, and the
license text at mozilla.org/MPL/2.0. The MIT license of each MIT entry has the same terms as the
Airwindows text above, with its own copyright line. Every crate carries its full license text in
its own source.

## License texts that travel with the programs

The BSD, ISC and Unicode licenses ask for their text to come with copies of the programs.

### `arrayref` (BSD-2-Clause)

```
Copyright (c) 2015 David Roundy <roundyd@physics.oregonstate.edu>
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

1. Redistributions of source code must retain the above copyright
   notice, this list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright
   notice, this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the
   distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### `kamadak-exif` and `mutate_once` (BSD-2-Clause)

`mutate_once` carries the same text with its own copyright line, "Copyright (c) 2019 KAMADA
Ken'ichi."

```
Copyright (c) 2016-2023 KAMADA Ken'ichi.
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:
1. Redistributions of source code must retain the above copyright
   notice, this list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright
   notice, this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE AUTHOR AND CONTRIBUTORS ``AS IS'' AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
ARE DISCLAIMED.  IN NO EVENT SHALL THE AUTHOR OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS
OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION)
HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT
LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY
OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF
SUCH DAMAGE.
```

### `tiny-skia` and `tiny-skia-path` (BSD-3-Clause)

```
Copyright (c) 2011 Google Inc. All rights reserved.
Copyright (c) 2020 Yevhenii Reizner All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

  * Redistributions of source code must retain the above copyright
    notice, this list of conditions and the following disclaimer.

  * Redistributions in binary form must reproduce the above copyright
    notice, this list of conditions and the following disclaimer in
    the documentation and/or other materials provided with the
    distribution.

  * Neither the name of the copyright holder nor the names of its
    contributors may be used to endorse or promote products derived
    from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### `encoding_rs`: the encoding data (BSD-3-Clause)

```
Copyright © WHATWG (Apple, Google, Mozilla, Microsoft).

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### `libloading` (ISC)

```
Copyright © 2015, Simonas Kazlauskas

Permission to use, copy, modify, and/or distribute this software for any purpose with or without
fee is hereby granted, provided that the above copyright notice and this permission notice appear
in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS
SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE
AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT,
NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF
THIS SOFTWARE.
```

### The VST 3 SDK's interfaces (MIT)

The `vst3` crate's bindings are generated from the interfaces of the VST 3 SDK, release 3.8.0
(`vst3_pluginterfaces` at `v3.8.0_build_66`), whose license asks for this notice to come with
them. VST is a registered trademark of Steinberg Media Technologies GmbH.

```
MIT License

Copyright (c) 2025, Steinberg Media Technologies GmbH

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### `unicode-ident`: the Unicode data (Unicode-3.0)

```
UNICODE LICENSE V3

COPYRIGHT AND PERMISSION NOTICE

Copyright © 1991-2023 Unicode, Inc.

NOTICE TO USER: Carefully read the following legal agreement. BY
DOWNLOADING, INSTALLING, COPYING OR OTHERWISE USING DATA FILES, AND/OR
SOFTWARE, YOU UNEQUIVOCALLY ACCEPT, AND AGREE TO BE BOUND BY, ALL OF THE
TERMS AND CONDITIONS OF THIS AGREEMENT. IF YOU DO NOT AGREE, DO NOT
DOWNLOAD, INSTALL, COPY, DISTRIBUTE OR USE THE DATA FILES OR SOFTWARE.

Permission is hereby granted, free of charge, to any person obtaining a
copy of data files and any associated documentation (the "Data Files") or
software and any associated documentation (the "Software") to deal in the
Data Files or Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, and/or sell
copies of the Data Files or Software, and to permit persons to whom the
Data Files or Software are furnished to do so, provided that either (a)
this copyright and permission notice appear with all copies of the Data
Files or Software, or (b) this copyright and permission notice appear in
associated Documentation.

THE DATA FILES AND SOFTWARE ARE PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
THIRD PARTY RIGHTS.

IN NO EVENT SHALL THE COPYRIGHT HOLDER OR HOLDERS INCLUDED IN THIS NOTICE
BE LIABLE FOR ANY CLAIM, OR ANY SPECIAL INDIRECT OR CONSEQUENTIAL DAMAGES,
OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THE DATA
FILES OR SOFTWARE.

Except as contained in this notice, the name of a copyright holder shall
not be used in advertising or otherwise to promote the sale, use or other
dealings in these Data Files or Software without prior written
authorization of the copyright holder.
```
