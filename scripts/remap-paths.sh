#!/usr/bin/env bash
# Writes a cargo config file that maps this machine's folders out of a build, and prints its path
# for --config: the home folder, this repository, the cargo home (the crates' sources), the rustup
# home (the standard library's) and the target dir (generated code) become ~, ., /cargo, /rustup
# and /target in the panic locations and debug info a program carries, and the C and C++ code the
# crates build (LAME's asserts name their source files) gets the same maps for the cargo home and
# the target dir. A release then names no user's folders. Every release build passes it
# (scripts/install.sh, scripts/xwin-build.sh, the release workflow):
#   cargo build --release --config "$(scripts/remap-paths.sh)" -p dawsome -p dawsome-app
# Why --config and not RUSTFLAGS: RUSTFLAGS from the environment replaces the Windows target's
# flags in .cargo/config.toml (its static C runtime), while a --config rustflags list is added to
# them. Why not in .cargo/config.toml: the folders differ on every machine. The C maps are for a
# Linux host's compiler only: on Windows the crates' C code is built from relative paths, and a
# cross-build's clang-cl takes other flags.
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/.."
home="${HOME:?remap-paths: HOME is not set}"
cargo_home="${CARGO_HOME:-$home/.cargo}"
rustup_home="${RUSTUP_HOME:-$home/.rustup}"
target="$(scripts/target-dir.sh)"
repo="$(pwd -P)"

rust=() c=()
# remap FROM TO [c]: the folder as given and its real path, in the form the compilers see it
# (Windows paths under the Git Bash or MSYS shell); with c, for the C compiler too.
remap() {
    local from real forms=("$1")
    if [ -e "$1" ] && real="$(realpath -- "$1")" && [ "$real" != "$1" ]; then
        forms+=("$real")
    fi
    for from in "${forms[@]}"; do
        if command -v cygpath >/dev/null; then
            from="$(cygpath -w -- "$from")"
        fi
        case "$from" in
            *"'"* | *'"'* | *[[:space:]]*)
                echo "remap-paths: $from holds a quote or a space, which the maps cannot carry" >&2
                exit 1
                ;;
        esac
        rust+=("'--remap-path-prefix=$from=$2'")
        [ "${3-}" != c ] || c+=("-ffile-prefix-map=$from=$2")
    done
}
# rustc applies the last prefix that matches, so the wider folders come first; the C maps name
# folders that do not hold one another.
remap "$home" "~"
remap "$repo" "."
remap "$cargo_home" /cargo c
remap "$rustup_home" /rustup
remap "$target" /target c

config="$target/remap-paths.toml"
mkdir -p -- "$target"
{
    echo "# Written by scripts/remap-paths.sh for --config; made again on every run."
    echo "[target.'cfg(all())']"
    echo "rustflags = [$(IFS=,; printf '%s' "${rust[*]}")]"
    if ! command -v cygpath >/dev/null; then
        host="$(rustc -vV | sed -n 's/^host: //p' | tr '-' '_')"
        echo "[env]"
        echo "CFLAGS_$host = '${c[*]}'"
        echo "CXXFLAGS_$host = '${c[*]}'"
    fi
} >"$config"
printf '%s\n' "$config"
