#!/usr/bin/env bash
# Fetches the instrument sets the programs ship: the dawsome-kit repository at the commit
# locks/kit.lock pins, into <target dir>/kit, its tree exactly that commit's; then runs the kit's own
# tools/check.py --manifest (every file under sets/ with its SHA-256, a CC0-1.0 license and its
# source). Prints the kit's folder. A clone already there is fetched into only when it lacks the
# commit. locks/kit.lock's url is cloned from, or $DAWSOME_KIT_URL when set (it overrides the url, so a
# clone builds from the published kit, or a mirror, whatever locks/kit.lock's url names).
# Usage: scripts/fetch-kit.sh
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/.."
[ $# -eq 0 ] || { echo "usage: scripts/fetch-kit.sh" >&2; exit 2; }

lock() { sed -n "s/^$1=//p" locks/kit.lock; }
url="${DAWSOME_KIT_URL:-$(lock url)}"
commit="$(lock commit)"
if [ -z "$url" ] || ! [[ "$commit" =~ ^[0-9a-f]{40}$ ]]; then
    echo "fetch-kit: locks/kit.lock needs a url and a 40-digit commit" >&2
    exit 2
fi
kit="$(scripts/target-dir.sh)/kit"
# Only the kit's own settings: no hooks or templates of this machine's run in the clone. And the
# kit's own repository alone: named outright, a missing or broken $kit/.git is an error, never
# the repository around it. Why: found by looking upward, git in a kit folder whose .git a cache
# restored in part took this checkout, and the forced checkout and clean below then put the
# kit's files over it and deleted the target dir (CI, 2026-10-30; tests/scripts/fetch_kit_test.py).
kit_git() {
    GIT_CONFIG_NOSYSTEM=1 git -C "$kit" --git-dir=.git --work-tree=. \
        -c advice.detachedHead=false -c core.hooksPath=/dev/null "$@"
}

if ! kit_git rev-parse --git-dir >/dev/null 2>&1; then
    rm -rf -- "$kit"
    mkdir -p -- "$(dirname -- "$kit")"
    GIT_CONFIG_NOSYSTEM=1 git clone --quiet --no-checkout --template= -- "$url" "$kit"
fi
if ! kit_git cat-file -e "$commit^{commit}" 2>/dev/null; then
    kit_git fetch --quiet -- "$url" '+refs/heads/*:refs/remotes/origin/*' '+refs/tags/*:refs/tags/*'
fi
kit_git checkout --quiet --force --detach "$commit"
kit_git clean --quiet -d -x -f
if [ "$(kit_git rev-parse HEAD)" != "$commit" ]; then
    echo "fetch-kit: $kit is not at $commit" >&2
    exit 1
fi
# -B: the check writes no bytecode into the clone, which stays the commit's files alone.
scripts/python.sh -I -B "$kit/tools/check.py" --manifest >&2
printf '%s\n' "$kit"
