#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

with_coverage=0
cargo_args=()
for arg in "$@"; do
  case "$arg" in
    --coverage)
      with_coverage=1
      ;;
    -h|--help)
      cat <<'EOF'
Usage: ./scripts/test-summary.sh [--coverage] [cargo test args...]

Options:
  --coverage       Run tests through cargo-llvm-cov (single test run),
                   then print a coverage summary.

Examples:
  ./scripts/test-summary.sh
  ./scripts/test-summary.sh --lib
  ./scripts/test-summary.sh --coverage
  ./scripts/test-summary.sh --coverage --lib
EOF
      exit 0
      ;;
    *)
      cargo_args+=("$arg")
      ;;
  esac
done

log_file="$(mktemp -t bitloops-test-summary.XXXXXX.log)"
cleanup() {
  rm -f "$log_file"
}
trap cleanup EXIT

coverage_file="$PROJECT_ROOT/target/llvm-cov.info"

set +e
if [[ "$with_coverage" -eq 1 ]]; then
  if ! cargo llvm-cov --version >/dev/null 2>&1; then
    echo "cargo-llvm-cov is not installed. Run: cargo install cargo-llvm-cov"
    exit 2
  fi

  rm -f "$coverage_file"
  if [[ ${#cargo_args[@]} -gt 0 ]]; then
    cargo llvm-cov --lcov --output-path "$coverage_file" "${cargo_args[@]}" 2>&1 | tee "$log_file"
  else
    cargo llvm-cov --workspace --all-features --all-targets --lcov --output-path "$coverage_file" 2>&1 | tee "$log_file"
  fi
else
  cargo test --no-fail-fast "${cargo_args[@]}" 2>&1 | tee "$log_file"
fi
status=${PIPESTATUS[0]}
set -e

echo
echo "=== Combined test summaries ==="
if command -v rg >/dev/null 2>&1; then
  if ! rg '^test result:' "$log_file"; then
    echo "No test summary lines found."
  fi
else
  if ! grep -E '^test result:' "$log_file"; then
    echo "No test summary lines found."
  fi
fi

if [[ "$with_coverage" -eq 1 ]]; then
  echo
  echo "=== Coverage summary ==="
  if [[ -f "$coverage_file" ]]; then
    echo "Overall:"
    awk -F: '
      BEGIN{lf=0;lh=0;brf=0;brh=0;fnf=0;fnh=0}
      /^LF:/ {lf += $2}
      /^LH:/ {lh += $2}
      /^BRF:/ {brf += $2}
      /^BRH:/ {brh += $2}
      /^FNF:/ {fnf += $2}
      /^FNH:/ {fnh += $2}
      function pct(c, t) {
        if (t == 0) return "n/a";
        return sprintf("%.2f%%", (c * 100.0 / t));
      }
      function row(metric, covered, total) {
        printf("  %-10s %10d %10d %10s\n", metric, covered, total, pct(covered, total));
      }
      END{
        printf("  %-10s %10s %10s %10s\n", "Metric", "Covered", "Total", "Percent");
        row("Lines", lh, lf);
        row("Functions", fnh, fnf);
        row("Branches", brh, brf);
      }
    ' "$coverage_file"

    per_file_tmp="$(mktemp -t bitloops-cov-files.XXXXXX)"
    cleanup_cov_tmp() { rm -f "$per_file_tmp"; }
    trap 'cleanup; cleanup_cov_tmp' EXIT

    PROJECT_ROOT="$PROJECT_ROOT" awk -F: '
      function flush(    line_pct, fn_pct, br_pct, p) {
        if (sf == "") return;
        p = sf;
        gsub("^" ENVIRON["PROJECT_ROOT"] "/", "", p);
        line_pct = (lf > 0) ? (lh * 100.0 / lf) : -1;
        fn_pct = (fnf > 0) ? (fnh * 100.0 / fnf) : -1;
        br_pct = (brf > 0) ? (brh * 100.0 / brf) : -1;
        printf("%f\t%s\t%d\t%d\t%f\t%d\t%d\t%f\t%d\t%d\t%f\n",
          (line_pct < 0 ? 101.0 : line_pct),
          p,
          lh, lf, line_pct,
          fnh, fnf, fn_pct,
          brh, brf, br_pct);
      }
      /^SF:/ {
        flush();
        sf = substr($0, 4);
        lf = lh = fnf = fnh = brf = brh = 0;
        next;
      }
      /^LF:/ { lf = $2 + 0; next; }
      /^LH:/ { lh = $2 + 0; next; }
      /^FNF:/ { fnf = $2 + 0; next; }
      /^FNH:/ { fnh = $2 + 0; next; }
      /^BRF:/ { brf = $2 + 0; next; }
      /^BRH:/ { brh = $2 + 0; next; }
      /^end_of_record$/ {
        flush();
        sf = "";
        next;
      }
      END { flush(); }
    ' "$coverage_file" > "$per_file_tmp"

    echo
    echo "Lowest line-coverage files (top 15):"
    sort -n -t $'\t' -k1,1 "$per_file_tmp" | head -n 15 | awk -F'\t' '
      function ratio(c, t, p) {
        if (t == 0) return "n/a";
        return sprintf("%d/%d (%.1f%%)", c, t, p);
      }
      function trim_path(s, n) {
        if (length(s) <= n) return s;
        return "..." substr(s, length(s) - n + 4);
      }
      BEGIN {
        printf("  %-62s | %-20s | %-20s | %-20s\n", "File", "Lines", "Functions", "Branches");
      }
      {
        printf("  %-62s | %-20s | %-20s | %-20s\n",
          trim_path($2, 62),
          ratio($3, $4, $5),
          ratio($6, $7, $8),
          ratio($9, $10, $11));
      }
    '

    echo "Coverage source: $coverage_file"
  else
    echo "No coverage data found at $coverage_file."
  fi
fi

exit "$status"
