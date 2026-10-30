#!/usr/bin/env bash
# The install, checked headless: scripts/install.sh puts the programs the tests built
# (<target dir>/debug) into a temporary prefix with the instrument sets, the installed app lists
# what it finds (dawsome-app --list-instruments, which opens no window), and that must be every
# set the kit at locks/kit.lock's commit and assets/instruments ship, as each set.json names it, found
# under <prefix>/share/dawsome/instruments: the instrument sets, and the kit's folder of sounds
# under Samples. When scripts/fetch-surge.sh has fetched the bundled plugin, it must be installed
# as fetched in <prefix>/lib/dawsome/clap, the installed app must look for plugins there
# (dawsome-app --list-plugin-folders), and the installed dawsome's scan of that folder must
# list Surge XT as Surge's own source names it; without it, that part is skipped and says so (a
# file the fetch refuses fails the check). The project file type must be installed:
# share/mime/packages/dawsome.xml as assets/dawsome-mime.xml has it, declaring the type the
# installed desktop entry's MimeType names, for files named *.dawsome. The Help window's
# pictures must be installed in <prefix>/share/dawsome/help as assets/help has them, and the
# installed app must read them there (dawsome-app --list-help): every picture its guide names
# found, and no picture installed that the guide does not name. Then the uninstall must
# leave nothing of the prefix. Everything lies under
# <target dir>/install-check, the run's per-user folders and home included: loading a plugin
# runs its code, and Surge makes its user folder in the home it is given.
# Usage: scripts/install-check.sh
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/.."
[ $# -eq 0 ] || { echo "usage: scripts/install-check.sh" >&2; exit 2; }
target_dir="$(scripts/target-dir.sh)"
root="$target_dir/install-check"
rm -rf -- "$root"
mkdir -p -- "$root"
prefix="$root/prefix"
# Only "not fetched" skips the plugin's part (--if-fetched prints nothing); a refused file or a
# wrong lock fails the check here, with the fetch's message.
surge="$(scripts/fetch-surge.sh --if-fetched linux-x86_64)"
kit="$(scripts/fetch-kit.sh 2>/dev/null)"
with_surge=()
if [ -n "$surge" ]; then
    with_surge=(--surge "$surge")
fi
scripts/install.sh --prefix "$prefix" --programs "$target_dir/debug" "${with_surge[@]}" \
    >"$root/install.txt" 2>&1 || {
    echo "install-check: scripts/install.sh --prefix $prefix failed:" >&2
    cat "$root/install.txt" >&2
    exit 1
}
# The project file type: the package installed as shipped, declaring for *.dawsome the type the
# installed desktop entry opens.
scripts/python.sh -I - "$prefix" assets/dawsome-mime.xml <<'PY'
import os, sys
import xml.etree.ElementTree as ET

prefix, shipped = sys.argv[1:]
package = os.path.join(prefix, "share/mime/packages/dawsome.xml")
entry = os.path.join(prefix, "share/applications/dawsome.desktop")
if not os.path.isfile(package):
    sys.exit("install-check: the project file type is not installed: no %s" % package)
with open(package, "rb") as f, open(shipped, "rb") as g:
    if f.read() != g.read():
        sys.exit("install-check: %s is not %s as shipped" % (package, shipped))
ns = {"m": "http://www.freedesktop.org/standards/shared-mime-info"}
types = {t.get("type"): [glob.get("pattern") for glob in t.findall("m:glob", ns)]
         for t in ET.parse(package).getroot().findall("m:mime-type", ns)}
with open(entry, encoding="utf-8") as f:
    listed = [line.split("=", 1)[1] for line in f.read().splitlines()
              if line.startswith("MimeType=")]
opened = [kind for line in listed for kind in line.split(";") if kind]
if types != {"application/x-dawsome-project": ["*.dawsome"]} or opened != list(types):
    sys.exit("install-check: the package declares %r, the desktop entry opens %r" % (types, opened))
print("install-check: share/mime/packages/dawsome.xml declares application/x-dawsome-project "
      "(*.dawsome), which the desktop entry opens")
PY
# The installed programs, run with every per-user folder and the home under the root, with no
# display to open a window on and no plugin folder of the system's added.
mkdir -p -- "$root/home"
installed() {
    env -u DAWSOME_SHIPPED_INSTRUMENTS -u DAWSOME_HELP_DIR -u CLAP_PATH -u DISPLAY -u WAYLAND_DISPLAY \
        HOME="$root/home" \
        DAWSOME_DATA_HOME="$root/data" DAWSOME_CONFIG_HOME="$root/config" \
        DAWSOME_CACHE_HOME="$root/cache" DAWSOME_STATE_HOME="$root/state" \
        XDG_DATA_HOME="$root/xdg/data" XDG_CONFIG_HOME="$root/xdg/config" \
        XDG_CACHE_HOME="$root/xdg/cache" XDG_STATE_HOME="$root/xdg/state" \
        "$prefix/bin/$1" "${@:2}"
}
installed dawsome-app --list-instruments > "$root/listed.txt"
scripts/python.sh -I - "$root/listed.txt" "$prefix/share/dawsome/instruments" "$kit" \
    assets/instruments <<'PY'
import json, os, sys

listed, installed, kit, electronic = sys.argv[1:]
want = []
counts = {}
for source, root in (("kit", os.path.join(kit, "sets")), ("electronic", electronic)):
    for folder in sorted(os.listdir(root)):
        with open(os.path.join(root, folder, "set.json"), encoding="utf-8") as f:
            about = json.load(f)
        place = os.path.join(installed, folder)
        if about["kind"] == "samples":
            want.append(("samples", about["name"], os.path.join(place, "samples")))
        else:
            want.append(("set", about["name"], about["maker"], os.path.join(place, about["sfz"])))
        counts[source] = counts.get(source, 0) + 1
with open(listed, encoding="utf-8") as f:
    got = [tuple(line.split("\t")) for line in f.read().splitlines()]
problems = []
if counts != {"kit": 15, "electronic": 7}:
    problems.append("the sets shipped are %r, not 15 of the kit and 7 electronic" % counts)
problems += ["not listed: %s" % "\t".join(line) for line in want if line not in got]
problems += ["listed, not shipped: %s" % "\t".join(line) for line in got if line not in want]
if problems:
    sys.exit("install-check: the installed app lists other than what ships:\n  "
             + "\n  ".join(problems))
print("install-check: the installed app lists %d instrument sets and %d folder of sounds"
      % (sum(line[0] == "set" for line in got), sum(line[0] == "samples" for line in got)))
PY
installed dawsome-app --list-help > "$root/help.txt"
scripts/python.sh -I - "$root/help.txt" "$prefix/share/dawsome/help" assets/help <<'PY'
import filecmp, os, sys

listed, installed, shipped = sys.argv[1:]
with open(listed, encoding="utf-8") as f:
    lines = [line.split("\t") for line in f.read().splitlines()]
problems = []
if lines[:1] != [["folder", installed]]:
    problems.append("the app reads its pictures from %r, not %s" % (lines[:1], installed))
named = [line[1] for line in lines[1:] if line[0] == "picture"]
problems += ["%s is %s" % (line[1], line[2]) for line in lines[1:] if line[2] != "found"]
put = sorted(os.listdir(installed)) if os.path.isdir(installed) else []
problems += ["%s is installed but not in the guide" % name for name in put if name not in named]
problems += ["%s is not assets/help's" % name for name in named if name in put
             and not filecmp.cmp(os.path.join(installed, name), os.path.join(shipped, name), False)]
if not named:
    problems.append("the guide names no picture")
if problems:
    sys.exit("install-check: the Help pictures are not installed as the guide names them:\n  "
             + "\n  ".join(problems))
print("install-check: the installed app reads the guide's %d pictures from share/dawsome/help: %s"
      % (len(named), ", ".join(named)))
PY
clap="$prefix/lib/dawsome/clap"
if [ -z "$surge" ]; then
    echo "install-check: Surge XT is not fetched (scripts/fetch-surge.sh): its install is not checked"
else
    if ! diff -r -q -- "$surge" "$clap" >&2; then
        echo "install-check: $clap is not Surge XT as fetched into $surge" >&2
        exit 1
    fi
    installed dawsome-app --list-plugin-folders > "$root/plugin-folders.txt"
    if ! grep -q -x -F -- "$clap" "$root/plugin-folders.txt"; then
        echo "install-check: the installed app does not look for plugins in $clap; it looks in:" >&2
        cat "$root/plugin-folders.txt" >&2
        exit 1
    fi
    if ! installed dawsome scan --plugin-path "$clap" --cache "$root/catalogue.json" \
        > "$root/scan.txt" 2>&1; then
        echo "install-check: the installed dawsome's scan of $clap failed:" >&2
        cat "$root/scan.txt" >&2
        exit 1
    fi
    # The known answer, from Surge's own source at release_xt_1.3.4: src/surge-xt/CMakeLists.txt
    # gives the CLAP id (CLAP_ID, line 65), the name (PRODUCT_NAME, lines 21 and 25), the maker
    # (COMPANY_NAME, line 26) and the "instrument" feature (CLAP_FEATURES, line 68); the version
    # is the release's, locks/surge.lock's.
    version="$(sed -n 's/^version=//p' locks/surge.lock)"
    scripts/python.sh -I - "$root/catalogue.json" "$clap/Surge XT.clap" "$version" <<'PY'
import json, sys

catalogue, plugin, version = sys.argv[1:]
with open(catalogue, encoding="utf-8") as f:
    files = json.load(f)["files"]
want = [{"id": "org.surge-synth-team.surge-xt", "name": "Surge XT", "vendor": "Surge Synth Team",
         "version": version, "kind": "instrument"}]
found = [f for f in files if f["file"] == plugin]
if len(found) != 1 or found[0]["status"] != "ok":
    sys.exit("install-check: the scan did not load %s: %r" % (plugin, found))
got = [{key: p[key] for key in want[0]} for p in found[0]["plugins"]]
if got != want:
    sys.exit("install-check: the scan lists %r in %s, not %r" % (got, plugin, want))
print("install-check: the installed app looks for plugins in lib/dawsome/clap, where the scan "
      "finds Surge XT (%s) by Surge Synth Team" % want[0]["id"])
PY
fi
scripts/install.sh --prefix "$prefix" --uninstall >/dev/null
if [ -e "$prefix" ]; then
    echo "install-check: the uninstall left:" >&2
    find "$prefix" >&2
    exit 1
fi
rm -rf -- "$root"
