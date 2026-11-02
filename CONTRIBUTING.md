# Contributing

Issues and pull requests are welcome. Contributions are licensed GPL-3.0-or-later, like the rest
of Dawsome.

- **Build.** The manual's (docs/manual.md) "Build and run" lists what to install on Linux (Rust, a C and C++
  compiler, clang, pkg-config, the ALSA, PipeWire, JACK and D-Bus development files, Python 3);
  `docs/windows.md` does so for Windows. Then `cargo build --workspace` builds everything and
  `cargo run -p dawsome-app` opens the app. The check below first fetches the instrument sets
  with `scripts/fetch-kit.sh`, from the url `locks/kit.lock` names or from `$DAWSOME_KIT_URL` when
  that is set.
- **Check before you send.** `scripts/check.sh` runs the license check, the format check, clippy
  with warnings as errors and every test, and `cargo deny check` when cargo-deny is installed; it
  must pass. A change to a panel also runs `scripts/ui-check.sh` in a graphical session.
- **Check the audio output list by hand.** No test asks the system for its output devices or
  opens one by name (headless runs list none), so a change to `crates/app/src/audio.rs` is also
  checked on the machine: `dawsome-app --list-outputs` prints the list Settings offers (it opens
  no device), "System default" and the system's output devices only, each once (no
  "default_sink", "default_output" or "unknown"); and in a run of the window a device chosen
  there plays at once, its name on the "Playing through:" line.
- **Build output** goes to `target/` in the repository, or to `$CARGO_TARGET_DIR` when it is set:
  a target dir grows to several GB, so on a small drive export one with room (for example
  `export CARGO_TARGET_DIR=/data/cargo-target/dawsome`). The scripts build into the same place.
- **Test first, against known answers.** Expected values come from the Python generators in
  `tests/fixtures/` (standard library only, run with `python3 -I`), never from running the engine.
  A change in behavior regenerates the fixtures from their generator; expected values are never
  edited by hand.
- **Keep the audio thread clean.** It never allocates, frees, locks or logs, and offline renders
  and live playback share `Player::process`, whose output must not depend on the block size.
- **Leave the machine as you found it.** Tests and scripts never write the user's settings, caches
  or data: pass `--cache`, `--cache-dir`, `--plugin-path` and temporary `XDG_*` folders.
- **Credit what you port.** Code from another project keeps its license and is listed in
  `licenses/THIRD_PARTY.md`. So is a dependency under a license other than MIT or Apache-2.0: a row in
  its crate table and its name in `licenses/NOTICE`, which `scripts/license-check.sh` checks. Any change
  to `Cargo.lock` needs `licenses/NOTICES.txt` written again: `python3 -I scripts/notices.py`.
