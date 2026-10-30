#!/usr/bin/env bash
# Fetches Surge XT, the CLAP plugin the programs bundle, as locks/surge.lock pins it (release 1.3.4):
# for each PLATFORM (linux-x86_64, windows-x86_64; default: this machine's) <target dir>/surge/
# <PLATFORM> holds `Surge XT.clap` and its factory data as `SurgeXTData/`, and its path is
# printed. --source fetches the release's complete source too and prints its path. Each asset
# is downloaded into a folder of its own under <target dir>/surge/downloads and refused unless
# its SHA-256 is the one pinned; nothing downloaded is run (scripts/surge.py says how).
# --vst3 makes <target dir>/surge/<PLATFORM>-vst3 instead: both VST3 bundles of the release with
# the factory data, for the VST3 host's tests only (never installed or packaged).
# --offline downloads nothing: it prints what is fetched, or exits 3 when it is not.
# --if-fetched downloads nothing either and prints what is fetched, nothing (exit 0) for what is
# not; any other failure (a file refused, a lock that is wrong) still fails with its message.
# Usage: scripts/fetch-surge.sh [--offline | --if-fetched] [--source] [--vst3] [PLATFORM...]
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/.."
args=()
platforms=()
for arg in "$@"; do
    case "$arg" in
        --offline | --if-fetched | --source | --vst3) args+=("$arg") ;;
        -*)
            echo "usage: scripts/fetch-surge.sh [--offline | --if-fetched] [--source] [--vst3] [PLATFORM...]" >&2
            exit 2
            ;;
        *) platforms+=("$arg") ;;
    esac
done
if [ ${#platforms[@]} -eq 0 ]; then
    case "$(uname -s)" in
        MINGW* | MSYS* | CYGWIN*) platforms=(windows-x86_64) ;;
        *) platforms=(linux-x86_64) ;;
    esac
fi
dir="$(scripts/target-dir.sh)/surge"
found="$(scripts/python.sh -I scripts/surge.py fetch --dir "$dir" "${args[@]}" "${platforms[@]}")"
[ -n "$found" ] || exit 0
# Python names the folders as Windows does (D:\a\...) under the Git Bash or MSYS shell; the
# scripts that use them want the shell's own form.
while IFS= read -r path; do
    if command -v cygpath >/dev/null; then
        path="$(cygpath -u -- "$path")"
    fi
    printf '%s\n' "$path"
done <<<"$found"
