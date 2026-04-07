#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ ! -d "$ROOT/.git" ]]; then
  echo "Error: $ROOT is not a git repository root"
  exit 1
fi

git -C "$ROOT" config --unset-all core.hooksPath 2>/dev/null || true

echo "Repo-local git hooks are cleared (core.hooksPath unset if it pointed here)."
echo "Run checks manually when needed:"
echo "  bash $ROOT/scripts/check-dev.sh"
echo "  bash $ROOT/scripts/check-dev.sh --test   # add the merge smoke lane"
echo "  bash $ROOT/scripts/check-dev.sh --full"
