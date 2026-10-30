#!/usr/bin/env bash
# Rejects auto-generated identifier patterns in tracked files: tool-made labels and placeholder
# names (FUN_1a2b, DAT_..., param_1, uVar3, local_10, unaff_..., in_EAX, calling-convention
# keywords). Nobody writes these by hand, so a hit is generated text that does not belong here.
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/.."
pattern='\b(FUN_[0-9a-fA-F]{4,}|DAT_[0-9a-fA-F]{4,}|LAB_[0-9a-fA-F]{4,}|param_[0-9]+|[uilpc]Var[0-9]+|local_[0-9a-fA-F]+|unaff_[A-Za-z0-9]+|in_(E|R)?[A-D]X|uStack[0-9_]+|__(this|fast|std)call)\b'
hits=$(git ls-files -- '*.md' '*.rs' '*.toml' '*.json' | grep -v '^scripts/lint-names.sh$' | xargs grep -nE "$pattern" 2>/dev/null || true)
if [ -n "$hits" ]; then
  echo "lint-names: auto-generated identifiers found in tracked files:" >&2
  echo "$hits" >&2
  exit 1
fi
echo "lint-names: clean"
