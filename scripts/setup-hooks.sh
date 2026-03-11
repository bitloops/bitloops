#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ ! -d "$ROOT/.git" ]]; then
  echo "Error: $ROOT is not a git repository root"
  exit 1
fi

git -C "$ROOT" config core.hooksPath .githooks
chmod +x "$ROOT/.githooks/pre-commit" "$ROOT/.githooks/pre-push"

echo "Hooks configured."
echo "core.hooksPath=$(git -C "$ROOT" config --get core.hooksPath)"
echo "Installed hooks:"
echo "  - $ROOT/.githooks/pre-commit"
echo "  - $ROOT/.githooks/pre-push"
