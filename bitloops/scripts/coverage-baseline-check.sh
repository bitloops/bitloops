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

BASELINE_FILE_JSONL="$PROJECT_ROOT/.coverage-baseline.jsonl"
COVERAGE_FILE="$PROJECT_ROOT/target/llvm-cov.info"
CANONICAL_CMD="cargo llvm-cov --workspace --no-default-features --all-targets --no-fail-fast --lcov --output-path target/llvm-cov.info"
EPSILON="0.05"

sanitize_git_env() {
  # Not included in local-env-vars but can poison git config resolution.
  unset GIT_CONFIG_GLOBAL GIT_CONFIG_SYSTEM GIT_CONFIG_NOSYSTEM

  while IFS= read -r name; do
    unset "$name"
  done < <(git rev-parse --local-env-vars)
}

usage() {
  cat <<'EOF'
Usage: ./scripts/coverage-baseline-check.sh [check|update]

Modes:
  check   Run coverage and fail if current coverage is lower than baseline.
  update  Run coverage and append a new JSON entry to .coverage-baseline.jsonl.
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
  cargo llvm-cov --workspace "${duckdb_no_bundle_flags[@]}" --all-targets --no-fail-fast --lcov --output-path "$COVERAGE_FILE"
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

json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

current_git_branch() {
  local branch
  branch="$(git -C "$PROJECT_ROOT" rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
  if [[ -z "$branch" || "$branch" == "HEAD" ]]; then
    printf "DETACHED_HEAD"
    return
  fi
  printf "%s" "$branch"
}

read_latest_baseline_line() {
  if [[ ! -f "$BASELINE_FILE_JSONL" ]]; then
    echo "Coverage baseline history file not found: $BASELINE_FILE_JSONL"
    echo "Create it first with: ./scripts/coverage-baseline-check.sh update"
    exit 1
  fi

  local line
  line="$(tail -n 1 "$BASELINE_FILE_JSONL")"
  if [[ -z "${line//[[:space:]]/}" ]]; then
    echo "Baseline history file is malformed: last line is empty."
    echo "Fix $BASELINE_FILE_JSONL or regenerate with: ./scripts/coverage-baseline-check.sh update"
    exit 1
  fi

  printf '%s' "$line"
}

read_baseline_metric_from_line() {
  local line="$1"
  local key="$2"
  local value
  value="$(printf '%s\n' "$line" | sed -nE "s/.*\"${key}\"[[:space:]]*:[[:space:]]*(-?[0-9]+([.][0-9]+)?).*/\\1/p")"

  if [[ -z "$value" ]]; then
    echo "Failed to read baseline metric \"$key\" from latest JSONL entry in $BASELINE_FILE_JSONL" >&2
    exit 1
  fi

  printf "%s" "$value"
}

is_regression() {
  local current="$1"
  local baseline="$2"
  local epsilon="$3"
  awk -v c="$current" -v b="$baseline" -v e="$epsilon" 'BEGIN { exit (c < (b - e)) ? 0 : 1 }'
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
  local now_utc git_branch escaped_branch escaped_cmd

  now_utc="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
  git_branch="$(current_git_branch)"
  escaped_branch="$(json_escape "$git_branch")"
  escaped_cmd="$(json_escape "$CANONICAL_CMD")"

  printf '{"version":1,"generated_at_utc":"%s","git_branch":"%s","command":"%s","epsilon":%s,"metrics":{"lines_pct":%s,"functions_pct":%s},"raw":{"lines":{"covered":%s,"total":%s},"functions":{"covered":%s,"total":%s}}}\n' \
    "$now_utc" \
    "$escaped_branch" \
    "$escaped_cmd" \
    "$EPSILON" \
    "$lines_pct" \
    "$functions_pct" \
    "$lines_cov" \
    "$lines_total" \
    "$fn_cov" \
    "$fn_total" \
    >> "$BASELINE_FILE_JSONL"
}

check_mode() {
  run_coverage

  read -r lines_cov lines_total fn_cov fn_total <<<"$(read_coverage_totals)"
  local lines_pct functions_pct
  lines_pct="$(percent "$lines_cov" "$lines_total")"
  functions_pct="$(percent "$fn_cov" "$fn_total")"

  local latest_baseline_line base_lines base_functions
  latest_baseline_line="$(read_latest_baseline_line)"
  base_lines="$(read_baseline_metric_from_line "$latest_baseline_line" "lines_pct")"
  base_functions="$(read_baseline_metric_from_line "$latest_baseline_line" "functions_pct")"

  echo "Coverage comparison (current vs baseline):"
  echo "  Baseline source: $(basename "$BASELINE_FILE_JSONL") (last entry via tail -n 1)"
  echo "  Tolerance: -${EPSILON} percentage points"
  printf "  %-10s baseline: %7.2f%%  current: %7.2f%%  delta: %s\n" \
    "Lines" "$base_lines" "$lines_pct" "$(delta "$lines_pct" "$base_lines")"
  printf "  %-10s baseline: %7.2f%%  current: %7.2f%%  delta: %s\n" \
    "Functions" "$base_functions" "$functions_pct" "$(delta "$functions_pct" "$base_functions")"

  local regressions=0
  if is_regression "$lines_pct" "$base_lines" "$EPSILON"; then
    regressions=$((regressions + 1))
  fi
  if is_regression "$functions_pct" "$base_functions" "$EPSILON"; then
    regressions=$((regressions + 1))
  fi

  if [[ "$regressions" -gt 0 ]]; then
    echo
    echo "Coverage regression detected. Push blocked."
    echo "Rule: current must be at least baseline - ${EPSILON}."
    echo "Add tests or intentionally update baseline."
    exit 1
  fi

  echo
  echo "Coverage check passed (within tolerance)."
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

  echo "Coverage baseline entry appended: $BASELINE_FILE_JSONL"
  printf "  %-10s %7.2f%% (%d/%d)\n" "Lines" "$lines_pct" "$lines_cov" "$lines_total"
  printf "  %-10s %7.2f%% (%d/%d)\n" "Functions" "$functions_pct" "$fn_cov" "$fn_total"
}

main() {
  sanitize_git_env

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
