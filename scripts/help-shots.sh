#!/usr/bin/env bash
# The Help window's pictures (help guide H4), made again: builds the app, then
# scripts/help_shots.py takes each screenshot assets/help/source/shots.json lists, headless and
# one window at a time, in per-user folders of their own under <target dir>/help-shots (never
# the user's settings; no audio or MIDI device opened), with the sets the programs ship staged
# there as an install has them (scripts/stage-instruments.sh: the kit at locks/kit.lock's commit and
# assets/instruments), cuts each picture out, draws its rings and arrows (Pillow) and writes
# assets/help/*.png and assets/help/source/shots.lock, which scripts/check.sh holds the
# pictures to. Run it after changing a shot, a source project, the
# script, or a part of the app a picture shows; then look at the pictures.
# Needs a graphical session (Wayland or X11) and Python 3 with Pillow.
# Usage: scripts/help-shots.sh
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/.."
target_dir="$(scripts/target-dir.sh)"
work="$target_dir/help-shots"
cargo build -q -p dawsome-app
rm -rf -- "$work"
mkdir -p -- "$work"
scripts/stage-instruments.sh "$work/instruments" >/dev/null
scripts/python.sh -I scripts/help_shots.py make --app "$target_dir/debug/dawsome-app" \
    --work "$work" --instruments "$work/instruments"
