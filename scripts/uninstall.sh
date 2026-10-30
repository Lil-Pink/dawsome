#!/usr/bin/env bash
# Removes what scripts/install.sh installed: its files, then each folder it made once that
# folder is empty, and nothing else (settings, caches, the demo and projects stay).
# Usage: scripts/uninstall.sh [--prefix <dir>]   (prefix default: ~/.local)
set -euo pipefail
usage="usage: scripts/uninstall.sh [--prefix <dir>]   (prefix default: ~/.local)"
# Only the prefix is taken: the install's other flags name what to install, which an
# uninstall has no use for, and its own usage line is the one shown.
case "$#:${1-}" in
    0: | 2:--prefix) ;;
    *) echo "$usage" >&2; exit 2 ;;
esac
exec "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/install.sh" --uninstall "$@"
