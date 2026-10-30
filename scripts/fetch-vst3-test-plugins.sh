#!/usr/bin/env bash
# Fetches the real VST3 plugins the VST3 host's integration tests load, as locks/vst3-test.lock pins
# them (Dexed 1.0.1, JS80P 4.1.1, DPF-Plugins 1.7; test inputs only, never shipped, installed or
# packaged): for each NAME (dexed, js80p, dpf; default: all) <target dir>/vst3-test/<NAME> holds
# what the tests load of it, and its path is printed. Each asset is downloaded into a folder of
# its own under <target dir>/vst3-test/downloads and refused unless its size and SHA-256 are the
# ones pinned; nothing downloaded is run (scripts/vst3_test_plugins.py says how).
# --offline downloads nothing: it prints what is fetched, or exits 3 when it is not.
# --if-fetched downloads nothing either and prints what is fetched, nothing (exit 0) for what is
# not; any other failure (a file refused, a lock that is wrong) still fails with its message.
# Usage: scripts/fetch-vst3-test-plugins.sh [--offline | --if-fetched] [NAME...]
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/.."
args=()
for arg in "$@"; do
    case "$arg" in
        --offline | --if-fetched) args+=("$arg") ;;
        -*)
            echo "usage: scripts/fetch-vst3-test-plugins.sh [--offline | --if-fetched] [NAME...]" >&2
            exit 2
            ;;
        *) args+=("$arg") ;;
    esac
done
dir="$(scripts/target-dir.sh)/vst3-test"
exec scripts/python.sh -I scripts/vst3_test_plugins.py fetch --dir "$dir" "${args[@]}"
