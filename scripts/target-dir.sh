#!/usr/bin/env bash
# Prints the cargo target dir this workspace builds into: $CARGO_TARGET_DIR when set, else
# target/ in the workspace (or where a cargo config of the user's own puts it). Cargo is asked
# rather than the variable read, so every script names the folder its builds really use.
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/.."
dir="$(cargo metadata --format-version 1 --no-deps |
    scripts/python.sh -I -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"
# Cargo names it as Windows does (D:\a\...) under the Git Bash or MSYS shell; the scripts that
# use it want the shell's own form.
if command -v cygpath >/dev/null; then
    dir="$(cygpath -u "$dir")"
fi
printf '%s\n' "$dir"
