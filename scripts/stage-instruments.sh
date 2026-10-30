#!/usr/bin/env bash
# Puts the instrument sets the programs ship into DEST, a folder of set folders the programs
# read as their installed sets: every set of the kit (scripts/fetch-kit.sh, at locks/kit.lock's
# commit) and each electronic set of assets/instruments/. DEST is made anew beside itself under
# a hidden name, then moved into place, so the programs never list a half-copied set. Two sets
# of one folder name are refused. Prints how many sets DEST holds.
# Usage: scripts/stage-instruments.sh DEST   (DEST is a folder named instruments)
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/.."
usage="usage: scripts/stage-instruments.sh DEST   (a folder named instruments)"
[ $# -eq 1 ] || { echo "$usage" >&2; exit 2; }
dest="$(realpath -m -- "$1")"
# Why only that name: DEST is replaced whole, so a mistyped argument must not name a folder of
# anything else.
[ "$(basename -- "$dest")" = instruments ] || { echo "$usage" >&2; exit 2; }

kit="$(scripts/fetch-kit.sh)"
parent="$(dirname -- "$dest")"
mkdir -p -- "$parent"
staged="$parent/.instruments.$$.tmp"
old="$parent/.instruments.$$.old"
trap 'rm -rf -- "$staged" "$old"' EXIT
rm -rf -- "$staged"
mkdir -- "$staged"
count=0
for set in "$kit"/sets/*/ assets/instruments/*/; do
    name="$(basename -- "$set")"
    if [ -e "$staged/$name" ]; then
        echo "stage-instruments: two sets are named $name" >&2
        exit 1
    fi
    cp -R -- "$set" "$staged/$name"
    count=$((count + 1))
done
if [ -e "$dest" ]; then
    mv -- "$dest" "$old"
fi
mv -- "$staged" "$dest"
echo "$count"
