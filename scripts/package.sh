#!/usr/bin/env bash
# Packs the release programs into <target dir>/dist/dawsome-<version>-<platform>.zip, with the
# instrument sets in instruments/ beside them (where the programs look for them; the kit at
# locks/kit.lock's commit and assets/instruments, scripts/stage-instruments.sh), the bundled plugin in
# plugins/ beside them (Surge XT.clap of the platform and its factory data, SurgeXTData/, as
# scripts/fetch-surge.sh fetched them; the programs look for plugins there), the icon, the demo
# project, the starter kit's sounds, the Help window's pictures in help/ beside the programs
# (where the app looks for them), the README (short), the manual (docs/manual.md, as manual.md)
# and LICENSE at the top (with docs/windows.md for Windows), and in licenses/ the rest of the
# paperwork: NOTICE, THIRD_PARTY.md, SOUNDS.md, NOTICES.txt (the texts of every library built
# in), LICENSE-SURGE and SOURCES.md (where the plugin comes from and its complete source; the
# release offers that source beside the zips). Build and fetch first (cargo build
# --release --config "$(scripts/remap-paths.sh)" -p dawsome -p dawsome-app, or
# scripts/xwin-build.sh for Windows from Linux; scripts/fetch-surge.sh <platform>). A program
# that names the home folder (built without scripts/remap-paths.sh) is refused.
# Usage: scripts/package.sh <platform> [<target triple>]
#   e.g. scripts/package.sh linux-x86_64
#        scripts/package.sh windows-x86_64 x86_64-pc-windows-msvc   (the cross-built programs)
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/.."
usage="usage: scripts/package.sh <platform> [<target triple>]"
[ $# -ge 1 ] && [ $# -le 2 ] || { echo "$usage" >&2; exit 2; }
platform="$1"
triple="${2:-}"
target_dir="$(scripts/target-dir.sh)"
release="$target_dir${triple:+/$triple}/release"
# The programs are .exe files when built for Windows, by name or on a Windows shell; the
# Windows zip also holds the notes on Windows.
case "$triple:$(uname -s)" in
    *windows*:* | :MINGW* | :MSYS* | :CYGWIN*) exe=.exe; notes=(docs/windows.md) ;;
    *) exe=; notes=() ;;
esac
# A Windows zip holds Windows programs: on another system the platform alone names no
# build, and the Linux programs went out under a Windows name once (2026-10-28).
case "$platform:$exe" in
    *windows*:) echo "package: $platform needs the Windows programs: scripts/package.sh $platform x86_64-pc-windows-msvc (after scripts/xwin-build.sh)" >&2; exit 2 ;;
esac
surge="$(scripts/fetch-surge.sh --offline "$platform")" || {
    echo "package: Surge XT is not fetched for $platform: scripts/fetch-surge.sh $platform" >&2
    exit 1
}
version="$(cargo metadata --format-version 1 --no-deps | scripts/python.sh -I -c \
    'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "dawsome"))')"
name="dawsome-$version-$platform"
dist="$target_dir/dist"
stage="$dist/$name"
rm -rf -- "$stage" "$dist/$name.zip"
mkdir -p -- "$stage/licenses" "$stage/icon" "$stage/demo/sounds" "$stage/starter-kit" "$stage/help"
# The home folder as a program built here would name it: panic locations in every crate's
# source carry their paths unless scripts/remap-paths.sh mapped them out.
homes=("$HOME/")
if command -v cygpath >/dev/null; then
    homes+=("$(cygpath -w -- "$HOME")\\")
fi
for program in dawsome dawsome-app; do
    [ -f "$release/$program$exe" ] || { echo "package: $release/$program$exe is not built" >&2; exit 1; }
    for home in "${homes[@]}"; do
        if grep -q -a -F -- "$home" "$release/$program$exe"; then
            echo "package: $release/$program$exe names $home; build it with" \
                "--config \"\$(scripts/remap-paths.sh)\"" >&2
            exit 1
        fi
    done
    cp -- "$release/$program$exe" "$stage/"
done
cp -- assets/icon/dawsome.svg assets/icon/dawsome-*.png "$stage/icon/"
cp -- examples/demo.json "$stage/demo/"
cp -- examples/sounds/*.wav "$stage/demo/sounds/"
cp -- assets/sounds/*.wav "$stage/starter-kit/"
cp -- assets/help/*.png "$stage/help/"
scripts/stage-instruments.sh "$stage/instruments" >/dev/null
cp -R -- "$surge" "$stage/plugins"
scripts/python.sh -I scripts/surge.py sources "$platform" > "$stage/licenses/SOURCES.md"
cp -- README.md LICENSE "${notes[@]}" "$stage/"
cp -- docs/manual.md "$stage/manual.md"
cp -- licenses/LICENSE-SURGE licenses/NOTICE licenses/NOTICES.txt licenses/THIRD_PARTY.md licenses/SOUNDS.md "$stage/licenses/"
(cd -- "$dist" && "$OLDPWD/scripts/python.sh" -I -m zipfile -c "$name.zip" "$name")
echo "$dist/$name.zip"
