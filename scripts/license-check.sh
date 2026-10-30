#!/usr/bin/env bash
# Fails when licenses/THIRD_PARTY.md or licenses/NOTICE misses or misstates a crate that the two programs are built
# from under a license other than MIT or Apache-2.0, on any release target; scripts/license-check.py
# says how it decides. Usage: scripts/license-check.sh [--third-party <file>] [--notice <file>]
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/.."
exec scripts/python.sh -I scripts/license-check.py "$@"
