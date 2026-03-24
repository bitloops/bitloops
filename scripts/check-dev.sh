#!/usr/bin/env bash
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
BL="$ROOT/bitloops"

usage() {
  cat <<'EOF'
Usage: bash scripts/check-dev.sh [--test] [--full]

  Default: Rust file-size check, cargo fmt --check, cargo clippy.
  --test   Also run the full suite via bitloops/scripts/test-summary.sh (cargo test --no-fail-fast + combined summaries).
  --full   Run coverage baseline check only (llvm-cov runs the full test suite once; no duplicate plain test run).
EOF
}

sanitize_git_env_for_coverage() {
  unset GIT_CONFIG_GLOBAL GIT_CONFIG_SYSTEM GIT_CONFIG_NOSYSTEM
  while IFS= read -r name; do
    unset "$name"
  done < <(git rev-parse --local-env-vars)
}

RUN_TEST=0
RUN_FULL=0
for arg in "$@"; do
  case "$arg" in
    --test) RUN_TEST=1 ;;
    --full) RUN_FULL=1 ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $arg" >&2
      usage >&2
      exit 1
      ;;
  esac
done

bash "$ROOT/scripts/check-rust-file-size.sh" "$BL"

cargo fmt --all --check --manifest-path "$BL/Cargo.toml"
cargo clippy --manifest-path "$BL/Cargo.toml" --all-targets --all-features -- -D warnings

if [[ "$RUN_TEST" == 1 ]] && [[ "$RUN_FULL" == 0 ]]; then
  (cd "$BL" && bash scripts/test-summary.sh)
fi

if [[ "$RUN_FULL" == 1 ]]; then
  sanitize_git_env_for_coverage
  (cd "$BL" && bash scripts/coverage-baseline-check.sh check)
fi
