#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

usage() {
  cat <<'EOF'
Usage: ./scripts/test-summary.sh [--full | --coverage]

Compatibility wrapper around the Cargo `dev-*` lanes.

  Default     Run `cargo dev-test-merge`
  --full      Run `cargo dev-test-full`
  --coverage  Run `cargo dev-coverage`

Use the Cargo aliases directly for day-to-day work.
EOF
}

mode="merge"
for arg in "$@"; do
  case "$arg" in
    --full)
      if [[ "$mode" != "merge" ]]; then
        echo "Options --full and --coverage are mutually exclusive." >&2
        exit 2
      fi
      mode="full"
      ;;
    --coverage)
      if [[ "$mode" != "merge" ]]; then
        echo "Options --full and --coverage are mutually exclusive." >&2
        exit 2
      fi
      mode="coverage"
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $arg" >&2
      usage >&2
      exit 2
      ;;
  esac
done

case "$mode" in
  merge)
    echo "==> cargo dev-test-merge"
    exec cargo dev-test-merge
    ;;
  full)
    echo "==> cargo dev-test-full"
    exec cargo dev-test-full
    ;;
  coverage)
    echo "==> cargo dev-coverage"
    exec cargo dev-coverage
    ;;
esac
