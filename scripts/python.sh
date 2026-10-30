#!/usr/bin/env bash
# Runs Python 3 with the given arguments: `python3` on Linux, `python` under the Git Bash or MSYS
# shell on Windows, where Python 3 installs as `python` and `python3` may be a store shortcut
# that runs nothing.
set -euo pipefail
case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) exec python "$@" ;;
    *) exec python3 "$@" ;;
esac
