#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

if [[ "${DUCKDB_USE_BUNDLED:-}" == "1" ]]; then
  unset DUCKDB_DOWNLOAD_LIB || true
  duckdb_no_bundle_flags=()
else
  export DUCKDB_DOWNLOAD_LIB="${DUCKDB_DOWNLOAD_LIB:-1}"
  duckdb_no_bundle_flags=(--no-default-features)
fi

usage() {
  cat <<'EOF'
Usage: ./scripts/test-coverage.sh [baseline|report]

Modes:
  baseline  Run full workspace coverage with default cargo threading and generate HTML + LCOV.
  report    Generate HTML + LCOV from existing collected coverage data.
EOF
}

ensure_llvm_cov() {
  if ! cargo llvm-cov --version >/dev/null 2>&1; then
    echo "cargo-llvm-cov is not installed."
    echo "Install it once with: cargo install cargo-llvm-cov"
    exit 1
  fi
}

generate_reports() {
  cargo llvm-cov report --html --output-dir target/llvm-cov-html
  cargo llvm-cov report --lcov --output-path target/llvm-cov.info
  echo "Coverage reports generated:"
  echo "  HTML: ${PROJECT_ROOT}/target/llvm-cov-html/index.html"
  echo "  LCOV: ${PROJECT_ROOT}/target/llvm-cov.info"
}

run_baseline() {
  cargo llvm-cov clean --workspace
  cargo llvm-cov --workspace "${duckdb_no_bundle_flags[@]}" --all-targets --no-fail-fast --html --output-dir target/llvm-cov-html
  cargo llvm-cov report --lcov --output-path target/llvm-cov.info
  echo "Coverage reports generated:"
  echo "  HTML: ${PROJECT_ROOT}/target/llvm-cov-html/index.html"
  echo "  LCOV: ${PROJECT_ROOT}/target/llvm-cov.info"
}

main() {
  local mode="${1:-baseline}"
  if [[ "${mode}" == "-h" || "${mode}" == "--help" ]]; then
    usage
    exit 0
  fi

  ensure_llvm_cov

  case "${mode}" in
    baseline)
      run_baseline
      ;;
    report)
      generate_reports
      ;;
    *)
      usage
      exit 1
      ;;
  esac
}

main "$@"
