#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

BASELINE_FILE="$PROJECT_ROOT/.coverage-baseline.json"
COVERAGE_FILE="$PROJECT_ROOT/target/llvm-cov.info"
CANONICAL_CMD="cargo llvm-cov --workspace --all-features --all-targets --lcov --output-path target/llvm-cov.info"

usage() {
  cat <<'EOF'
Usage: ./scripts/coverage-baseline-check.sh [check|update]

Modes:
  check   Run coverage and fail if current coverage is lower than baseline.
  update  Run coverage and rewrite .coverage-baseline.json with current metrics.
EOF
}

ensure_llvm_cov() {
  if ! cargo llvm-cov --version >/dev/null 2>&1; then
    echo "cargo-llvm-cov is not installed."
    echo "Install it once with: cargo install cargo-llvm-cov"
    exit 1
  fi
}

run_coverage() {
  rm -f "$COVERAGE_FILE"
  cargo llvm-cov --workspace --all-features --all-targets --lcov --output-path "$COVERAGE_FILE"
  if [[ ! -f "$COVERAGE_FILE" ]]; then
    echo "Coverage file was not generated: $COVERAGE_FILE"
    exit 1
  fi
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

read_baseline_metric() {
  local key="$1"
  local value
  value="$(awk -v key="$key" '
    $0 ~ "\"" key "\"" {
      if (match($0, /-?[0-9]+(\.[0-9]+)?/)) {
        print substr($0, RSTART, RLENGTH)
        exit
      }
    }
  ' "$BASELINE_FILE")"

  if [[ -z "$value" ]]; then
    echo "Failed to read baseline metric \"$key\" from $BASELINE_FILE" >&2
    exit 1
  fi

  printf "%s" "$value"
}

is_less() {
  local lhs="$1"
  local rhs="$2"
  awk -v l="$lhs" -v r="$rhs" 'BEGIN { exit (l < r) ? 0 : 1 }'
}

delta() {
  local current="$1"
  local baseline="$2"
  awk -v c="$current" -v b="$baseline" 'BEGIN { printf "%+.2f", (c - b) }'
}

write_baseline() {
  local lines_pct="$1"
  local functions_pct="$2"
  local lines_cov="$3"
  local lines_total="$4"
  local fn_cov="$5"
  local fn_total="$6"
  local now_utc

  now_utc="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

  cat > "$BASELINE_FILE" <<EOF
{
  "version": 1,
  "generated_at_utc": "$now_utc",
  "command": "$CANONICAL_CMD",
  "metrics": {
    "lines_pct": $lines_pct,
    "functions_pct": $functions_pct
  },
  "raw": {
    "lines": { "covered": $lines_cov, "total": $lines_total },
    "functions": { "covered": $fn_cov, "total": $fn_total }
  }
}
EOF
}

check_mode() {
  if [[ ! -f "$BASELINE_FILE" ]]; then
    echo "Coverage baseline file not found: $BASELINE_FILE"
    echo "Create it first with: ./scripts/coverage-baseline-check.sh update"
    exit 1
  fi

  run_coverage

  read -r lines_cov lines_total fn_cov fn_total <<<"$(read_coverage_totals)"
  local lines_pct functions_pct
  lines_pct="$(percent "$lines_cov" "$lines_total")"
  functions_pct="$(percent "$fn_cov" "$fn_total")"

  local base_lines base_functions
  base_lines="$(read_baseline_metric "lines_pct")"
  base_functions="$(read_baseline_metric "functions_pct")"

  echo "Coverage comparison (current vs baseline):"
  printf "  %-10s baseline: %7.2f%%  current: %7.2f%%  delta: %s\n" \
    "Lines" "$base_lines" "$lines_pct" "$(delta "$lines_pct" "$base_lines")"
  printf "  %-10s baseline: %7.2f%%  current: %7.2f%%  delta: %s\n" \
    "Functions" "$base_functions" "$functions_pct" "$(delta "$functions_pct" "$base_functions")"

  local regressions=0
  if is_less "$lines_pct" "$base_lines"; then
    regressions=$((regressions + 1))
  fi
  if is_less "$functions_pct" "$base_functions"; then
    regressions=$((regressions + 1))
  fi

  if [[ "$regressions" -gt 0 ]]; then
    echo
    echo "Coverage regression detected. Push blocked."
    echo "Add tests or intentionally update baseline."
    exit 1
  fi

  echo
  echo "Coverage check passed (no metric decreased)."
}

update_mode() {
  run_coverage

  read -r lines_cov lines_total fn_cov fn_total <<<"$(read_coverage_totals)"
  local lines_pct functions_pct
  lines_pct="$(percent "$lines_cov" "$lines_total")"
  functions_pct="$(percent "$fn_cov" "$fn_total")"

  write_baseline \
    "$lines_pct" "$functions_pct" \
    "$lines_cov" "$lines_total" \
    "$fn_cov" "$fn_total"

  echo "Coverage baseline updated: $BASELINE_FILE"
  printf "  %-10s %7.2f%% (%d/%d)\n" "Lines" "$lines_pct" "$lines_cov" "$lines_total"
  printf "  %-10s %7.2f%% (%d/%d)\n" "Functions" "$functions_pct" "$fn_cov" "$fn_total"
}

main() {
  local mode="${1:-check}"
  case "$mode" in
    -h|--help)
      usage
      ;;
    check)
      ensure_llvm_cov
      check_mode
      ;;
    update)
      ensure_llvm_cov
      update_mode
      ;;
    *)
      usage
      exit 1
      ;;
  esac
}

main "$@"
