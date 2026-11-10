#!/usr/bin/env bash
# The project's check, run by CI and before every commit: licenses, dependencies, format, lint,
# tests. Exits non-zero on any failure.
set -euo pipefail

# Test folders go under the target dir, never /tmp, which is often memory (once some 2900
# stale dawsome-* test folders held 14 GB of it), so runs with separate target dirs keep
# apart; folders a day old are removed first. An absolute path: rustc runs in each dependency's
# own folder, where a relative one (CI's, with no CARGO_TARGET_DIR) names no folder at all.
cd -- "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/.."
export TMPDIR="${TMPDIR:-$(scripts/target-dir.sh)/tmp}"
mkdir -p "$TMPDIR"
find "$TMPDIR" -mindepth 1 -maxdepth 1 -name 'dawsome-*' -mmin +1440 -exec rm -rf {} + 2>/dev/null || true
scripts/lint-names.sh
# licenses/THIRD_PARTY.md and licenses/NOTICE against the crates the programs are built from, on every target.
scripts/license-check.sh
# licenses/NOTICES.txt, the license and notice texts of every crate and C library built in, against what
# scripts/notices.py writes from Cargo.lock and the crates' own files (offline): a changed lock
# needs the file written again.
scripts/python.sh -I scripts/notices.py --check
# The instrument sets the programs ship, at locks/kit.lock's commit (each file against the kit's
# manifest), and licenses/SOUNDS.md against every shipped sound's manifest: all CC0-1.0, each source
# listed.
kit="$(scripts/fetch-kit.sh)"
scripts/license-check.sh --sounds "$kit"
# The Windows app's icon is made from the icon's PNGs; a changed PNG needs a new .ico.
scripts/python.sh -I scripts/make-ico.py --check
# The starter kit's drums and the electronic instrument sets are what assets/sounds/kit.json
# states: SHA-256, WAV header, frames and peak, and each set's .sfz and set.json as check.py
# writes them. The sounds are committed with their hashes, and check.py checks them.
scripts/python.sh -I assets/sounds/check.py --check
# The instrument test sets, their projects and their known answers are what their generator
# writes, byte for byte: the tests read the answers from those files.
scripts/python.sh -I tests/fixtures/gen_p11_expected.py --check tests/fixtures
# The electronic instrument sets' known answers, from the sets as committed.
scripts/python.sh -I tests/fixtures/gen_p12_expected.py --check tests/fixtures
# The sound settings' known answers, their projects and their sounds are what their generator
# writes, byte for byte.
scripts/python.sh -I tests/fixtures/gen_p13_expected.py --check tests/fixtures
# The metronome's known answers and their projects are what their generator writes.
scripts/python.sh -I tests/fixtures/gen_p14_expected.py --check tests/fixtures
# A sound dropped on a channel's header: the documents each drop gives, as their generator
# writes them.
scripts/python.sh -I tests/fixtures/gen_p15_expected.py --check tests/fixtures
# The external input channel's known answers, its projects and its feed file are what their
# generator writes, byte for byte.
scripts/python.sh -I tests/fixtures/gen_p16_expected.py --check tests/fixtures
# Recording v2's known answers (takes of notes in pattern and song mode, audio takes, Capture):
# the projects, MIDI events and the documents each case leaves are what their generator writes.
scripts/python.sh -I tests/fixtures/gen_p17_expected.py --check tests/fixtures
# Interchange export's known answers (the song as MIDI, stems, DAWproject, their reports): the
# projects, the hand-written MIDI bytes and project.xml files are what their generator writes.
scripts/python.sh -I tests/fixtures/gen_p18_expected.py --check tests/fixtures
# MIDI out's known answers (the note on and off bytes an input channel with a MIDI out sends, and
# the output frame of each): the projects and the messages are what their generator writes.
scripts/python.sh -I tests/fixtures/gen_p19_expected.py --check tests/fixtures
# The sustain pedal's, pitch bend's and the mod wheel's known answers: the sine set, the projects,
# the MIDI messages and the renders, takes and MIDI-out messages they give.
scripts/python.sh -I tests/fixtures/gen_p22_expected.py --check tests/fixtures
# Input latency compensation's known answers: the click burst, the input feeds the measurement
# finds it in, and the documents a take of the line in leaves once a latency is kept.
scripts/python.sh -I tests/fixtures/gen_p23_expected.py --check tests/fixtures
# The master limiter's known answers (and the two loop fixes found beside it): the sounds, sets and
# projects, and the renders the Limiter gives, frame by frame, as its generator writes them.
scripts/python.sh -I tests/fixtures/gen_p24_expected.py --check tests/fixtures
# The sampler's interpolation and the export's dither: the sounds, sets and projects, the frames
# the 4-point Hermite read gives, and the 16-bit values the dither writes, as their generator
# writes them.
scripts/python.sh -I tests/fixtures/gen_p25_expected.py --check tests/fixtures
# Cut itself: the sine and its sets, the projects and MIDI messages, and the renders a note's cut
# of the notes before it gives (pattern notes, live notes, under the pedal), as their generator
# writes them.
scripts/python.sh -I tests/fixtures/gen_p26_expected.py --check tests/fixtures
# The VST3 host's known answers: the test plugins' projects, a catalogue of both formats, and the
# renders, presets, scan records and latency the VST3 test plugins' formulas give.
scripts/python.sh -I tests/fixtures/gen_p27_expected.py --check tests/fixtures
# VST3 in the app: the one list over both formats and its twin rule, the dumps and Library rows
# of the test plugins, the scan child's output, pitch bend and the mod wheel through a plugin's
# MIDI mapping, stepped and mapped parameters, restarts and the latency texts.
scripts/python.sh -I tests/fixtures/gen_p28_expected.py --check tests/fixtures
# VST3 plugin windows: the editor's calls the host makes in each case (opening, a plugin's and a
# user's resize, the frame's run loop, focus, the ways a window closes), the XEmbed messages and
# the X11 requests every plugin window opens with, and the View class's project and render.
scripts/python.sh -I tests/fixtures/gen_p29_expected.py --check tests/fixtures
# A sound's edit list (the sample editor's crop, delete, silence, fades, gain, normalize and
# reverse): the sounds and projects, the frames each list gives, the renders, the clips' moved
# offsets, every op's undo step and the zero-crossing search, as their generator writes them.
scripts/python.sh -I tests/fixtures/gen_p30_expected.py --check tests/fixtures
# Swing: the sounds and projects, the renders a pattern's swing gives (the second 16th of each
# pair late, the note's end with it, a swung note passing a hand-placed one, a song clip's
# window on the swung ticks), as their generator writes them.
scripts/python.sh -I tests/fixtures/gen_p31_expected.py --check tests/fixtures
# The native effects (filter, distortion, gate, stereo width, chorus, flanger, phaser): the input
# sounds and projects, each effect's parameters, and the renders their formulas give.
scripts/python.sh -I tests/fixtures/gen_p32_expected.py --check tests/fixtures
# Dawsome Synth: its projects and MIDI messages, its 37 parameters, and the renders its
# formulas give (the voice pool, oscillator 1's sine, the amp envelope, the pedal and the bend).
scripts/python.sh -I tests/fixtures/gen_p33_expected.py --check tests/fixtures
# Time signatures other than 4/4: the beat and the bar each makes, the metronome's and the
# count-in's clicks, the render lengths, the project file, the edit's growth, the MIDI and
# DAWproject files, a plugin's bar, the position readout and the step grid, as their generator
# writes them.
scripts/python.sh -I tests/fixtures/gen_p34_expected.py --check tests/fixtures
# Time signatures per pattern and along the song: a pattern's own signature (its renders, MIDI
# file, edit, step grid, and the metronome, count-in, readout and plugin bar while it plays
# alone) and the song's meter map (its bars, readouts, snaps, metronome, count-ins, plugin bars,
# MIDI and DAWproject files, --bars, the stems' end, the change edits and their settle rule), as
# their generator writes them.
scripts/python.sh -I tests/fixtures/gen_p35_expected.py --check tests/fixtures
# The piano roll's player: the sounds, projects and their references (every other channel muted)
# a channel's render alone must equal byte for byte, the frames worked out by hand, and the
# lengths of a pass played once and of two passes looped.
scripts/python.sh -I tests/fixtures/gen_roll_player.py --check tests/fixtures
# The UI's bundled font (crates/app/assets/fonts): each file against fonts.lock's size, SHA-256
# and version string, and the widths the app's tests hold it to ("Notes" at 12 px, the readouts')
# as worked out from the font's own tables.
scripts/python.sh -I tests/fixtures/gen_u1_expected.py --check tests/fixtures
# The type scale: every text size comes from crates/app/src/ui/fonts.rs's `size` (TINY, DENSE,
# LABEL, BODY, PANEL_TITLE, TITLE, and OCTAVE_NAME, the typing keyboard's octave alone). A literal size (`.size(12)`, `.text_size(12)`, a canvas text's
# `size: 12.0.into()`) or a text-size constant of a file's own (`const NAME_SIZE: f32 = ...`)
# anywhere else fails; the *_SIZE constants named here size a control, not text. Why: about ten
# sizes a pixel apart read as noise, and a size of a file's own drifts off the scale unseen.
type_scale="$(grep -rnE '\.(text_)?size\([0-9]|size: *\(?[0-9.]+\)?\.into\(\)|const [A-Z_]*_SIZE: f32 = ' \
    crates/app/src --include='*.rs' |
    grep -v '^crates/app/src/ui/fonts\.rs:' |
    grep -vE 'const (CONTROL|KNOB|PAN|SEND|REMOVE|TICK)_SIZE: f32 = ' || true)"
if [ -n "$type_scale" ]; then
    echo "check: text sizes outside the type scale (crates/app/src/ui/fonts.rs's size):" >&2
    echo "$type_scale" >&2
    exit 1
fi
# The colour themes: each theme's contrast, rule by rule, from the design's colours and the WCAG
# formula; the app's palettes are held to its colours and ratios.
scripts/python.sh -I tests/fixtures/gen_ui_contrast.py --check tests/fixtures
# The accent of the user's own: HSV and RGB, the colour wheel's geometry, the contrast note's
# threshold and the slider fill's law, from their formulas; the app's wheel and palette are held
# to them.
scripts/python.sh -I tests/fixtures/gen_accent_expected.py --check tests/fixtures
# The Help window's pictures against what they are made from (scripts/help-shots.sh remakes them).
scripts/python.sh -I scripts/help_shots.py check
# The DAWproject schema checker finds the faults planted in its own documents, and the known
# answers' project.xml files follow the published schema (tests/fixtures/dawproject/); xmllint,
# when installed, validates them too, as a second, independent reader of the schema.
scripts/python.sh -I -B tests/scripts/dawproject_check.py --self-test
scripts/python.sh -I -B tests/scripts/dawproject_check.py tests/fixtures/p18_daw_*.project.xml tests/fixtures/p34_daw_*.project.xml tests/fixtures/p35_meter.project.xml
if command -v xmllint >/dev/null; then
    xmllint --noout --schema tests/fixtures/dawproject/Project.xsd tests/fixtures/p18_daw_*.project.xml tests/fixtures/p34_daw_*.project.xml tests/fixtures/p35_meter.project.xml
else
    echo "xmllint not installed: second schema validation skipped"
fi
# The layout bounds check (scripts/ui-check.sh runs it on the app's dumps at four zooms) finds the
# overlap and the cut label planted in a dump of its own, each named, and passes the clean one.
scripts/python.sh -I -B tests/scripts/layout_bounds.py --self-test
# The check that the toolbar's time signature stands right of the tempo in a layout dump
# (scripts/ui-check.sh runs it on the zoom's dumps and on F1 in 6/8) finds its planted faults.
scripts/python.sh -I -B tests/scripts/signature_readout.py --self-test
# The fetch of the instrument sets lands the kit at its commit, and fetches a kit folder whose .git
# is no repository again without touching the repository around it (repositories of its own).
scripts/python.sh -I -B tests/scripts/fetch_kit_test.py
# The fetch of the bundled plugin keeps only the files locks/surge.lock pins and takes only the plugin
# and its factory data, from archives of the tests' own (no network).
scripts/python.sh -I -B tests/scripts/fetch_surge_test.py
# The fetch of the real VST3 plugins the VST3 host's tests load (test inputs, never shipped) keeps
# only what locks/vst3-test.lock pins and takes only the plugins named, from archives of its own.
scripts/python.sh -I -B tests/scripts/fetch_vst3_test_plugins_test.py
# The install touches only the files its manifest names, and a refused plugin fails it: stand-in
# programs and plugin installed into temporary prefixes under the target dir.
scripts/python.sh -I -B tests/scripts/install_test.py
# The memory cap starts one scope, never a scope inside a scope (a stand-in systemd-run).
scripts/python.sh -I -B tests/scripts/capped_test.py
# The measurement marks a run whose window updated far off its scenario's rate, for
# scripts/measure.sh to run it again (made-up runs, no app started).
scripts/python.sh -I -B tests/scripts/measure_test.py
# The plugin host's tests load Dawsome's own test plugins from the target dir.
scripts/build-test-plugins.sh
# The test plugins are the host's oracles, so they must be valid CLAP plugins themselves.
if command -v clap-validator >/dev/null; then
    target_dir="$(scripts/target-dir.sh)"
    clap-validator validate --only-failed "$target_dir/test-plugins/dawsome_test_plugin.clap"
else
    echo "clap-validator not installed: test plugin validation skipped"
fi
# The dependency graph of both release targets (deny.toml): licenses, advisories, sources.
# Offline, the advisory database last fetched is used. Errors only: the warnings (duplicate
# versions inside the dependencies' own trees, the LAME crates' deprecated license identifier)
# fail nothing, and `cargo deny check` lists them.
if cargo deny --version >/dev/null 2>&1; then
    cargo deny fetch db >/dev/null 2>&1 ||
        echo "cargo-deny: the advisory database was not refreshed; checking against the last copy"
    cargo deny --offline --log-level error check
else
    echo "cargo-deny not installed: dependency audit skipped"
fi
cargo fmt --all -- --check
# Both run inside a memory-capped cgroup when systemd offers one (scripts/capped.sh): a runaway
# test can then only kill itself, not the editor session that started the check.
scripts/capped.sh -- cargo clippy --workspace --all-targets -q -- -D warnings
scripts/capped.sh -- cargo test --workspace -q
# The install, headless: the programs the tests just built and the instrument sets installed into
# a temporary prefix, the installed app's listing against what ships, the bundled plugin found
# and scanned when scripts/fetch-surge.sh has fetched it, then the uninstall.
scripts/install-check.sh
