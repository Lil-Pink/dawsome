# Dawsome on Windows

Dawsome builds and runs on 64-bit Windows 10 and 11 (`x86_64-pc-windows-msvc`): the app, the command
line, CLAP and VST3 plugins (VST3 built, not yet tried on a Windows machine), audio through the
system's default output (WASAPI), MIDI input (WinMM), recording, WAV and MP3 export and
time-stretch. The programs carry the C and C++ runtime inside them, so no Visual C++ redistributable
is needed.

## Build on Windows

You need:

- Rust 1.93 or later with the MSVC toolchain (`rustup default stable-x86_64-pc-windows-msvc`).
- The Microsoft C++ build tools with a Windows SDK (the "Desktop development with C++"
  workload of the Visual Studio Build Tools). MP3 export builds the LAME encoder from its C
  source and time-stretch builds the Signalsmith Stretch C++ library; both are compiled with
  MSVC by their crates' build scripts.
- LLVM, for its `libclang.dll`: the time-stretch crate generates its Rust bindings with
  libclang. Install LLVM and point `LIBCLANG_PATH` at its `bin` folder, for example
  `C:\Program Files\LLVM\bin`.
- Python 3 (as `python`) to run the tests, and Git for Windows, whose Git Bash runs the
  scripts in `scripts/`.

Build output goes to `target\` in the repository, or to `CARGO_TARGET_DIR` when it is set (a
target dir grows to several GB). The static C runtime comes from the repository's
`.cargo/config.toml`; a `RUSTFLAGS` set in the environment replaces it, so leave that unset.
From the repository:

```bat
cargo build --release -p dawsome -p dawsome-app
target\release\dawsome-app.exe
```

Programs for a release zip are built from Git Bash with
`--config "$(scripts/remap-paths.sh)"` added, so they name none of your folders;
`scripts/package.sh` refuses one that names the home folder.

The zip holds the programs, `README.md` (the short introduction), `manual.md` (the reference),
`LICENSE`, this file, and a `licenses/` folder with the other license and source notices
(`NOTICE`, `NOTICES.txt`, `THIRD_PARTY.md`, `SOUNDS.md`, `LICENSE-SURGE`, `SOURCES.md`).

The tests, from Git Bash:

```sh
scripts/build-test-plugins.sh   # the plugin host's tests load these
cargo test --workspace
```

A few tests run on Linux only (they use a POSIX shell, `/proc`, `/dev/full`, file mode bits,
symbolic links or the Linux desktop entry); each says why where it is gated. The CI workflow
(`.github/workflows/ci.yml`) builds and tests on Windows on every push.

## Cross-build from Linux

`scripts/xwin-build.sh` builds the Windows programs on Linux with
[cargo-xwin](https://github.com/rust-cross/cargo-xwin), which downloads the Windows SDK and the
MSVC runtime libraries (about 1 GB) into `$XWIN_CACHE_DIR` (default `<target dir>/xwin`):

```sh
rustup target add x86_64-pc-windows-msvc
cargo install cargo-xwin
scripts/xwin-build.sh        # = cargo xwin build --release --target x86_64-pc-windows-msvc
                             #   -p dawsome -p dawsome-app -p dawsome-test-plugin
scripts/package.sh windows-x86_64 x86_64-pc-windows-msvc   # the zip, in <target dir>/dist
```

It needs `clang`, `clang-cl`, `lld-link`, `llvm-lib`, `llvm-rc` and libclang on the `PATH`, and
`make`. `llvm-rc` compiles the app's icon and version details (`crates/app/dawsome-app.rc`);
on Windows the SDK's `rc.exe` does.
Arguments, when given, go to `cargo xwin` in place of the default build, for example
`scripts/xwin-build.sh clippy --target x86_64-pc-windows-msvc --workspace --all-targets`. The
LAME crate picks its build by the machine it runs on, so from Linux it runs LAME's `configure`;
the script tells `configure` the host is a Windows one and gives its preprocessor check the
Windows headers. A build on Windows takes the crate's MSVC path and needs none of this.

## Where things are kept

- Settings, recent files and the Library's sample folders: `%APPDATA%\dawsome\`.
- The demo, the starter kit and the recordings of documents not yet saved:
  `%LOCALAPPDATA%\dawsome\`.
- The plugin catalogue and stretched or converted audio: `%LOCALAPPDATA%\dawsome\`.
- Autosaves of untitled documents: `%LOCALAPPDATA%\dawsome\autosave\`.

`DAWSOME_CONFIG_HOME`, `DAWSOME_DATA_HOME`, `DAWSOME_CACHE_HOME` and `DAWSOME_STATE_HOME`, set
to an absolute folder, move these on every platform (`dawsome\` is made inside); a relative
value is ignored. On Linux the `XDG_*` variables keep working. The tests never touch a user's
folders: they set `DAWSOME_CACHE_HOME` or pass `--cache` and `--cache-dir`, and the scripted
runs they and the scripts make (`--quit-after`, `--screenshot-after`) write no settings, recent
files or autosaves.

Projects are saved as `.dawsome` files (`.json` projects open too). The zip registers no file
type: to open `.dawsome` files with a double-click, right-click one, choose Open with → Choose
another app → More apps → Look for another app on this PC, pick `dawsome-app.exe` from the
unpacked folder and tick Always use this app. `dawsome-app.exe song.dawsome` opens one from a
console.

CLAP plugins are found in `%COMMONPROGRAMFILES%\CLAP`, `%LOCALAPPDATA%\Programs\Common\CLAP`
and the folders of `CLAP_PATH` (separated by `;`); VST3 plugins in `%COMMONPROGRAMFILES%\VST3`,
`%LOCALAPPDATA%\Programs\Common\VST3` and the folders of `VST3_PATH`. A project file writes relative sample paths
with `/` on every platform, so a project folder moves between Windows and Linux; absolute paths
name one machine's files.

## Limits on Windows

- The audio thread runs at normal priority: there is no request for realtime priority (on Linux
  it is asked for and the app plays on without it when refused). Audio uses the default output
  device in shared mode; JACK, PipeWire and ASIO are not built.
- A plugin's interface is embedded in a top-level window of the app's own (a Win32 window whose
  handle is the plugin's parent); a plugin that only floats gets its own floating window, and
  one with no interface the parameter window.
- The release build of `dawsome-app.exe` opens no console window, so what it prints is not
  shown; use `dawsome.exe` (the command line), or a debug build, to see messages.
- Renders match Linux renders byte for byte (measured under Wine with the cross-built release
  command line: f1_four_on_floor and the demo, as WAV and as MP3), except time-stretched clips:
  the time-stretch library's C++ maths comes out slightly differently from another compiler and
  C runtime (p9_stretch: at most 0.0012 apart on a 0.55 peak; p9_stretch_pitch, shifted up a
  fifth, at most 0.011 apart on a 0.52 peak). Each platform is consistent with itself, and its
  stretch cache rebuilds byte for byte.
- Each plugin scan child runs in a job object that is killed with everything in it at the
  timeout and when the scan ends. The child starts suspended and runs only once it is in the
  job, so every process it starts is in the job too.
