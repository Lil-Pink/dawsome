#!/usr/bin/env bash
# Builds the command-line image (Dockerfile) and renders projects inside it; each render must be
# the host's render of the same project, byte for byte: f1_four_on_floor (4 bars) and the demo
# (song mode, 8 bars).
# Uses docker when its daemon answers, else podman, rootless, with its image store, run state and
# temp files in the work dir, so neither the user's container storage nor any per-user folder is
# written. The host's release build goes to the target dir as usual.
# Usage: scripts/docker-check.sh [work-dir]   (default: <target dir>/docker-check)
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/.."
target_dir="$(scripts/target-dir.sh)"
work="$(realpath -m -- "${1:-$target_dir/docker-check}")"
out="$work/out"
rm -rf -- "$out"
mkdir -p -- "$out"
if command -v docker >/dev/null && docker info >/dev/null 2>&1; then
    engine=(docker)
    # The container writes into $out as the user running this script.
    as_user=(--user "$(id -u):$(id -g)")
elif command -v podman >/dev/null; then
    store="$work/podman"
    mkdir -p -- "$store/root" "$store/run" "$store/tmp" "$store/data" "$store/config" \
        "$store/cache"
    # The per-user folders too: podman keeps a layer cache under the data folder whatever
    # --root says.
    engine=(env XDG_DATA_HOME="$store/data" XDG_CONFIG_HOME="$store/config"
        XDG_CACHE_HOME="$store/cache"
        podman --root "$store/root" --runroot "$store/run" --tmpdir "$store/tmp"
        --cgroup-manager cgroupfs --events-backend none)
    # The image's user is the host user, so the container writes into $out as this user.
    as_user=(--userns keep-id)
else
    echo "docker-check: neither docker nor podman is installed" >&2
    exit 1
fi
image=localhost/dawsome-cli:check
"${engine[@]}" build --tag "$image" --file Dockerfile .
cargo build --release --locked -q -p dawsome
host="$target_dir/release/dawsome"
renders=(
    "f1 tests/fixtures/f1_four_on_floor.json --bars 4"
    "demo examples/demo.json --song --bars 8"
)
for render in "${renders[@]}"; do
    read -r name project options <<<"$render"
    read -r -a options <<<"$options"
    "$host" render "$project" "$out/$name-host.wav" "${options[@]}"
    "${engine[@]}" run --rm --network none "${as_user[@]}" \
        --volume "$PWD:/repo:ro" --volume "$out:/out" \
        "$image" render "/repo/$project" "/out/$name-image.wav" "${options[@]}"
    if ! cmp -- "$out/$name-host.wav" "$out/$name-image.wav"; then
        echo "docker-check: $name: the image's render differs from the host's" >&2
        exit 1
    fi
    echo "docker-check: $name: byte-identical ($(wc -c <"$out/$name-host.wav") bytes)"
done
