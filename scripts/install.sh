#!/usr/bin/env bash
# Installs Dawsome for the current user, built in release mode:
#   <prefix>/bin/dawsome, <prefix>/bin/dawsome-app
#   <prefix>/share/applications/dawsome.desktop (its Exec names the installed program; its
#     MimeType names the project file type, so a file manager opens *.dawsome files with it)
#   <prefix>/share/mime/packages/dawsome.xml: the type application/x-dawsome-project, files
#     named *.dawsome (assets/dawsome-mime.xml)
#   <prefix>/share/icons/hicolor/<N>x<N>/apps/dawsome.png and scalable/apps/dawsome.svg
#   <prefix>/share/dawsome/instruments/: the instrument sets the programs ship, which they find
#     there from <prefix>/bin (scripts/stage-instruments.sh: the kit at locks/kit.lock's commit and
#     assets/instruments/)
#   <prefix>/share/doc/dawsome/NOTICES.txt: the license texts of every library built into the
#     programs, where the container image keeps its license documents (Dockerfile)
#   <prefix>/share/dawsome/help/: the Help window's pictures (assets/help/*.png), which the app
#     finds there from <prefix>/bin
#   <prefix>/lib/dawsome/clap/: the bundled plugin, Surge XT.clap with its factory data in
#     SurgeXTData/ beside it, where Surge finds it; the programs look for plugins there from
#     <prefix>/bin. Installed when scripts/fetch-surge.sh has fetched it (locks/surge.lock); when it
#     is not fetched, the one installed before is kept, else it is left out with a note. A file
#     there that the fetch refuses fails the install. Plugins of the user's own put in that
#     folder are theirs: no install or uninstall touches them.
#   <prefix>/lib/dawsome/install-manifest: every file this install put, one per line, and
#   <prefix>/lib/dawsome/install-record: the folders it made
# Each file is copied beside its place and renamed into it, so a running copy keeps its file
# (scripts/install-files.py). A reinstall replaces the files the earlier install put and removes
# those it no longer puts; nothing else.
# With --uninstall (or scripts/uninstall.sh) the files the manifest names are removed, then each
# folder the record names once it is empty, and nothing else. Settings, caches, the demo, the
# sets in the data folder, projects and the user's own plugins are never touched.
# --programs <dir> installs the dawsome and dawsome-app programs already built in <dir> rather
# than building them in release mode; --surge <dir> installs the bundled plugin's folder <dir>
# (as scripts/fetch-surge.sh makes it) rather than the one fetched. Both are for checks of the
# install itself.
# Usage: scripts/install.sh [--prefix <dir>] [--programs <dir>] [--surge <dir>] [--uninstall]
#   (prefix default: ~/.local)
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/.."

usage="usage: scripts/install.sh [--prefix <dir>] [--programs <dir>] [--surge <dir>] [--uninstall]"
prefix="$HOME/.local"
programs=
surge=
mode=install
while [ $# -gt 0 ]; do
    case "$1" in
        --prefix | --programs | --surge)
            [ $# -ge 2 ] || { echo "$usage" >&2; exit 2; }
            case "$1" in
                --prefix) prefix="$2" ;;
                --programs) programs="$2" ;;
                --surge) surge="$2" ;;
            esac
            shift 2
            ;;
        --uninstall) mode=uninstall; shift ;;
        *) echo "$usage" >&2; exit 2 ;;
    esac
done
case "$prefix" in
    /*) ;;
    *) echo "install.sh: the prefix must be an absolute path: $prefix" >&2; exit 2 ;;
esac
# The desktop entry quotes the program's path, where these characters would need escaping.
case "$prefix" in
    *[\"\`\$\\%]*)
        echo "install.sh: the prefix holds a character a desktop entry cannot quote: $prefix" >&2
        exit 2
        ;;
esac

bin="$prefix/bin"
applications="$prefix/share/applications"
icons="$prefix/share/icons/hicolor"
mime="$prefix/share/mime"
sizes=(16 32 48 64 128 256 512)
sets_name=share/dawsome/instruments
clap_name=lib/dawsome/clap
# What an install put before it kept a manifest, the instrument sets and the bundled plugin's
# data folder each a folder put whole: without a manifest, these are the earlier install's.
legacy=(bin/dawsome bin/dawsome-app share/applications/dawsome.desktop
    share/icons/hicolor/scalable/apps/dawsome.svg lib/dawsome/install-record
    "$sets_name" "$clap_name/Surge XT.clap" "$clap_name/SurgeXTData")
for size in "${sizes[@]}"; do
    legacy+=("share/icons/hicolor/${size}x${size}/apps/dawsome.png")
done
# The bundled plugin's folder, which such an install made: theirs to remove once it is empty.
legacy_args=(--legacy-folder "$clap_name")
for place in "${legacy[@]}"; do
    legacy_args+=(--legacy "$place")
done

# The desktop rereads these folders by itself; refreshing an existing cache only makes it see
# the change sooner, and a cache is never created where there was none. The MIME database is
# not reread from its packages: the desktop knows the .dawsome type only once
# update-mime-database has run, which is done here where a database is already kept (it is on
# most desktops' ~/.local), else said, since making one would leave files the uninstall does not
# own. update-mime-database refuses a folder without packages/, which only an uninstall that
# removed the last package (and the packages/ its install made) leaves.
refresh_caches() {
    if [ -f "$mime/mime.cache" ] && [ -d "$mime/packages" ]; then
        if command -v update-mime-database >/dev/null; then
            update-mime-database "$mime" ||
                echo "install.sh: the MIME database was not refreshed" >&2
        else
            echo "install.sh: update-mime-database is not installed: the MIME database was not refreshed" >&2
        fi
    elif [ "$mode" = install ] && [ ! -f "$mime/mime.cache" ]; then
        echo "install.sh: $mime keeps no MIME database: the .dawsome file type is known to the desktop once 'update-mime-database $mime' has run" >&2
    fi
    if command -v update-desktop-database >/dev/null && [ -f "$applications/mimeinfo.cache" ]; then
        update-desktop-database -q "$applications" ||
            echo "install.sh: the desktop database was not refreshed" >&2
    fi
    if command -v gtk-update-icon-cache >/dev/null && [ -f "$icons/icon-theme.cache" ]; then
        gtk-update-icon-cache -q -f -t "$icons" ||
            echo "install.sh: the icon cache was not refreshed" >&2
    fi
}

if [ "$mode" = uninstall ]; then
    scripts/python.sh -I scripts/install-files.py uninstall --prefix "$prefix" "${legacy_args[@]}"
    refresh_caches
    exit 0
fi

# The bundled plugin first: only "not fetched" leaves it out (--if-fetched prints nothing), and
# any other failure of the fetch (a file refused, a lock that is wrong) stops the install here,
# before anything is written. Its download is large, so scripts/fetch-surge.sh is run on purpose.
# Then the sets: fetching them is the step that can fail for want of the network, and it writes
# only to the target dir.
if [ -z "$surge" ]; then
    surge="$(scripts/fetch-surge.sh --if-fetched linux-x86_64)"
elif [ ! -f "$surge/Surge XT.clap" ]; then
    echo "install.sh: $surge holds no Surge XT.clap" >&2
    exit 2
fi
scripts/fetch-kit.sh >/dev/null
if [ -z "$programs" ]; then
    cargo build --release -q --config "$(scripts/remap-paths.sh)" -p dawsome -p dawsome-app
    programs="$(scripts/target-dir.sh)/release"
fi
for program in dawsome dawsome-app; do
    [ -f "$programs/$program" ] || { echo "install.sh: $programs/$program is not built" >&2; exit 1; }
done

# The sets and the desktop entry are made in a folder of this install's own in the target dir.
work="$(scripts/target-dir.sh)/install.$$"
trap 'rm -rf -- "$work"' EXIT
rm -rf -- "$work"
mkdir -p -- "$work"
set_count="$(scripts/stage-instruments.sh "$work/instruments")"

# The entry starts the installed program by its full path: a desktop session's PATH often
# lacks ~/.local/bin.
exec_line=0
while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
        "Exec=dawsome-app "*)
            printf 'Exec="%s"%s\n' "$bin/dawsome-app" "${line#Exec=dawsome-app}"
            exec_line=$((exec_line + 1))
            ;;
        *) printf '%s\n' "$line" ;;
    esac
done < assets/dawsome.desktop > "$work/dawsome.desktop"
if [ "$exec_line" != 1 ]; then
    echo "install.sh: assets/dawsome.desktop has no single 'Exec=dawsome-app ...' line" >&2
    exit 1
fi

files=(--file 755 "$programs/dawsome" bin/dawsome
    --file 755 "$programs/dawsome-app" bin/dawsome-app
    --file 644 "$work/dawsome.desktop" share/applications/dawsome.desktop
    --file 644 assets/dawsome-mime.xml share/mime/packages/dawsome.xml
    --file 644 licenses/NOTICES.txt share/doc/dawsome/NOTICES.txt
    --file 644 assets/icon/dawsome.svg share/icons/hicolor/scalable/apps/dawsome.svg)
for size in "${sizes[@]}"; do
    files+=(--file 644 "assets/icon/dawsome-$size.png"
        "share/icons/hicolor/${size}x${size}/apps/dawsome.png")
done
files+=(--tree "$work/instruments" "$sets_name")
# The Help window's pictures alone: assets/help/source holds what they are made from.
mkdir -p -- "$work/help"
cp -- assets/help/*.png "$work/help/"
files+=(--tree "$work/help" share/dawsome/help)
if [ -n "$surge" ]; then
    files+=(--tree "$surge" "$clap_name")
    bundled=" and the bundled plugin, Surge XT"
elif [ -e "$prefix/$clap_name/Surge XT.clap" ]; then
    files+=(--keep "$clap_name")
    bundled=", keeping the bundled plugin installed before (scripts/fetch-surge.sh fetches it)"
else
    bundled=", without the bundled plugin (scripts/fetch-surge.sh fetches it)"
fi
count="$(scripts/python.sh -I scripts/install-files.py install --prefix "$prefix" \
    "${legacy_args[@]}" "${files[@]}")"
refresh_caches
echo "installed $count files, $set_count instrument sets$bundled into $prefix; start it from the menu or $bin/dawsome-app"
