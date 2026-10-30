#!/usr/bin/env bash
# Runs a command inside a memory-capped cgroup when systemd can provide one, so a runaway
# test or build can only kill itself. Why: a test binary that spun to 49 GB took the whole
# editor session down with it (2026-10-22); the cap confines the kernel's kill to the
# command. Without systemd (CI runners, Windows) the command runs as is. The scope's
# environment holds DAWSOME_CAPPED=1, and a run that finds it set (a capped script calling this
# again, as check.sh's cargo runs under a capped check) runs the command in place: a scope inside
# a scope would count against the outer cap twice and cost a call to systemd each time
# (tests/scripts/capped_test.py).
# Usage: scripts/capped.sh [--max SIZE] -- <command...>   (SIZE like 24G; default 24G)
set -euo pipefail
max=24G
if [ "${1:-}" = "--max" ]; then max=$2; shift 2; fi
[ "${1:-}" = "--" ] && shift
if [ "${DAWSOME_CAPPED:-}" = 1 ]; then
    exec "$@"
fi
# Passed on only when set: an empty CARGO_TARGET_DIR is an error to cargo (CI sets none).
target=()
if [ -n "${CARGO_TARGET_DIR:-}" ]; then
    target=(--setenv=CARGO_TARGET_DIR="$CARGO_TARGET_DIR")
fi
if command -v systemd-run >/dev/null 2>&1 && systemd-run --user --scope --quiet -- true >/dev/null 2>&1; then
    exec systemd-run --user --scope --quiet --same-dir \
        -p MemoryMax="$max" -p MemorySwapMax=2G \
        --setenv=DAWSOME_CAPPED=1 \
        --setenv=PATH="$PATH" --setenv=HOME="$HOME" \
        "${target[@]}" \
        --setenv=CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" \
        --setenv=RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \
        -- "$@"
fi
exec "$@"
