#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

COVERAGE_FILE="$PROJECT_ROOT/target/llvm-cov.info"
MIN_LINE_COVERAGE_PCT="${BITLOOPS_MIN_LINE_COVERAGE_PCT:-80}"
MIN_FUNCTION_COVERAGE_PCT="${BITLOOPS_MIN_FUNCTION_COVERAGE_PCT:-75}"

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
  baseline  Run full workspace coverage, generate HTML + LCOV, and enforce minimum line/function coverage.
  report    Generate HTML + LCOV from existing collected coverage data.

Environment:
  BITLOOPS_MIN_LINE_COVERAGE_PCT       Minimum line coverage percentage for baseline mode (default: 80).
  BITLOOPS_MIN_FUNCTION_COVERAGE_PCT   Minimum function coverage percentage for baseline mode (default: 75).
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
  cargo llvm-cov report --lcov --output-path "$COVERAGE_FILE"
  echo "Coverage reports generated:"
  echo "  HTML: ${PROJECT_ROOT}/target/llvm-cov-html/index.html"
  echo "  LCOV: ${COVERAGE_FILE}"
}

read_coverage_totals() {
  awk -F: '
    BEGIN { lf=0; lh=0; fnf=0; fnh=0 }
    /^LF:/  { lf  += $2 + 0; next }
    /^LH:/  { lh  += $2 + 0; next }
    /^FNF:/ { fnf += $2 + 0; next }
    /^FNH:/ { fnh += $2 + 0; next }
    END { printf "%d %d %d %d\n", lh, lf, fnh, fnf }
  ' "$COVERAGE_FILE"
}

percent() {
  local covered="$1"
  local total="$2"
  awk -v c="$covered" -v t="$total" 'BEGIN {
    if (t == 0) {
      printf "0.00"
    } else {
      printf "%.2f", (c * 100.0 / t)
    }
  }'
}

validate_threshold() {
  local name="$1"
  local value="$2"
  if [[ ! "$value" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
    echo "Invalid ${name}: ${value}" >&2
    exit 2
  fi
}

is_below_minimum() {
  local current="$1"
  local minimum="$2"
  awk -v c="$current" -v m="$minimum" 'BEGIN { exit (c < m) ? 0 : 1 }'
}

check_minimums() {
  if [[ ! -f "$COVERAGE_FILE" ]]; then
    echo "Coverage file was not generated: $COVERAGE_FILE" >&2
    exit 1
  fi

  validate_threshold "BITLOOPS_MIN_LINE_COVERAGE_PCT" "$MIN_LINE_COVERAGE_PCT"
  validate_threshold "BITLOOPS_MIN_FUNCTION_COVERAGE_PCT" "$MIN_FUNCTION_COVERAGE_PCT"

  read -r lines_cov lines_total fn_cov fn_total <<<"$(read_coverage_totals)"

  local lines_pct functions_pct failures=0
  lines_pct="$(percent "$lines_cov" "$lines_total")"
  functions_pct="$(percent "$fn_cov" "$fn_total")"

  echo
  echo "Coverage minimums:"
  printf "  %-10s minimum: %7.2f%%  current: %7.2f%%\n" \
    "Lines" "$MIN_LINE_COVERAGE_PCT" "$lines_pct"
  printf "  %-10s minimum: %7.2f%%  current: %7.2f%%\n" \
    "Functions" "$MIN_FUNCTION_COVERAGE_PCT" "$functions_pct"

  if is_below_minimum "$lines_pct" "$MIN_LINE_COVERAGE_PCT"; then
    failures=$((failures + 1))
  fi
  if is_below_minimum "$functions_pct" "$MIN_FUNCTION_COVERAGE_PCT"; then
    failures=$((failures + 1))
  fi

  if [[ "$failures" -gt 0 ]]; then
    echo
    echo "Coverage minimum check failed."
    echo "Rule: lines >= ${MIN_LINE_COVERAGE_PCT}% and functions >= ${MIN_FUNCTION_COVERAGE_PCT}%."
    exit 1
  fi

  echo
  echo "Coverage minimum check passed."
}

run_baseline() {
  cargo llvm-cov clean --workspace
  cargo llvm-cov --workspace "${duckdb_no_bundle_flags[@]}" --all-targets --no-fail-fast --html --output-dir target/llvm-cov-html
  cargo llvm-cov report --lcov --output-path "$COVERAGE_FILE"
  echo "Coverage reports generated:"
  echo "  HTML: ${PROJECT_ROOT}/target/llvm-cov-html/index.html"
  echo "  LCOV: ${COVERAGE_FILE}"
  check_minimums
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
