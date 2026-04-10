#!/usr/bin/env bash
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
BL="$ROOT/bitloops"

usage() {
  cat <<'EOF'
Usage: bash scripts/check-dev.sh [--test] [--full]

  Default: Rust file-size check, cargo fmt, cargo clippy.
  --test   Also run cargo dev-test-merge (fast lane + curated slow smoke suites).
  --full   Run coverage baseline check only (llvm-cov runs the full test suite once; no duplicate plain test run).

  DuckDB: by default uses official prebuilt libduckdb (fast). Set DUCKDB_USE_BUNDLED=1 to compile from source instead.
EOF
}

# Fast path: download official libduckdb for this host (linux-gnu, macOS universal, win-msvc). Bundled C++ compile if unset.
if [[ "${DUCKDB_USE_BUNDLED:-}" == "1" ]]; then
  unset DUCKDB_DOWNLOAD_LIB || true
  DUCKDB_CARGO_FEATURES=()
else
  export DUCKDB_DOWNLOAD_LIB="${DUCKDB_DOWNLOAD_LIB:-1}"
  DUCKDB_CARGO_FEATURES=(--no-default-features)
fi

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

cargo fmt --all --manifest-path "$BL/Cargo.toml"
cargo clippy --manifest-path "$BL/Cargo.toml" --all-targets "${DUCKDB_CARGO_FEATURES[@]}" -- -D warnings

if [[ "$RUN_TEST" == 1 ]] && [[ "$RUN_FULL" == 0 ]]; then
  (cd "$ROOT" && cargo dev-test-merge)
fi

if [[ "$RUN_FULL" == 1 ]]; then
  sanitize_git_env_for_coverage
  (cd "$BL" && bash scripts/coverage-baseline-check.sh check)
fi
