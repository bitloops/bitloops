#!/usr/bin/env bash
set -euo pipefail

# Checks Rust file sizes with a ratchet:
# - default max applies to non-test files
# - known legacy outliers can use a per-file budget
# - test files are ignored by default (set INCLUDE_TESTS=1 to include)
# - gitignored files are skipped

ROOT="${1:-bitloops}"
WARN_LINES="${RUST_FILE_WARN_LINES:-500}"
MAX_LINES="${RUST_FILE_MAX_LINES:-1000}"
INCLUDE_TESTS="${INCLUDE_TESTS:-0}"
GIT_TOPLEVEL="$(git -C "$ROOT" rev-parse --show-toplevel 2>/dev/null || true)"

declare -A LEGACY_MAX_LINES=()

if command -v rg >/dev/null 2>&1; then
  mapfile -t rust_files < <(rg --files "$ROOT" -g '*.rs')
else
  echo "warning: ripgrep (rg) not found; falling back to find"
  mapfile -t rust_files < <(find "$ROOT" -type f -name '*.rs' | sort)
fi
if [[ "${#rust_files[@]}" -eq 0 ]]; then
  echo "No Rust files found under $ROOT"
  exit 0
fi

declare -a warnings=()
declare -a failures=()
declare -a legacy_over_default=()
declare -a size_index=()

is_test_file() {
  local path="$1"
  [[ "$path" == */tests/* || "$path" == *_test.rs || "$path" == *tests.rs || "$path" == *_tests.rs ]]
}

is_gitignored_file() {
  local path="$1"
  [[ -z "$GIT_TOPLEVEL" ]] && return 1
  git -C "$GIT_TOPLEVEL" check-ignore -q -- "$path"
}

resolve_legacy_max_for_file() {
  local file="$1"
  local key
  local short_key
  for key in "${!LEGACY_MAX_LINES[@]}"; do
    short_key="${key#bitloops/}"
    if [[ "$file" == "$key" \
      || "$file" == "./$key" \
      || "$file" == "../$key" \
      || "$file" == "$short_key" \
      || "$file" == "./$short_key" \
      || "$file" == "../$short_key" \
      || "$file" == */"$key" \
      || "$file" == */"$short_key" ]]; then
      echo "${LEGACY_MAX_LINES[$key]}"
      return 0
    fi
  done
  return 1
}

for file in "${rust_files[@]}"; do
  if [[ "$INCLUDE_TESTS" != "1" ]] && is_test_file "$file"; then
    continue
  fi

  if is_gitignored_file "$file"; then
    continue
  fi

  lines="$(wc -l < "$file" | tr -d ' ')"
  size_index+=("${lines} ${file}")

  effective_max="$MAX_LINES"
  is_legacy=0
  if legacy_max="$(resolve_legacy_max_for_file "$file")"; then
    effective_max="$legacy_max"
    is_legacy=1
  fi

  if (( lines > effective_max )); then
    failures+=("${lines} ${file} (max ${effective_max})")
    continue
  fi

  if (( lines > WARN_LINES )); then
    warnings+=("${lines} ${file}")
  fi

  if (( is_legacy == 1 && lines > MAX_LINES )); then
    legacy_over_default+=("${lines} ${file} (legacy max ${effective_max})")
  fi
done

echo "Rust file-size check"
echo "- root: $ROOT"
echo "- warn: >${WARN_LINES} lines"
echo "- max:  >${MAX_LINES} lines (non-test, except legacy budgets)"
echo

if [[ "${#legacy_over_default[@]}" -gt 0 ]]; then
  echo "Legacy allowlist currently above default max:"
  printf '  %s\n' "${legacy_over_default[@]}" | sort -nr
  echo
fi

if [[ "${#warnings[@]}" -gt 0 ]]; then
  echo "Warnings:"
  printf '  %s\n' "${warnings[@]}" | sort -nr
  echo
fi

if [[ "${#size_index[@]}" -gt 0 ]]; then
  echo "Top non-test Rust files by line count:"
  printf '%s\n' "${size_index[@]}" | sort -nr | head -n 15 | sed 's/^/  /'
  echo
fi

if [[ "${#failures[@]}" -gt 0 ]]; then
  echo "Failures:"
  printf '  %s\n' "${failures[@]}" | sort -nr
  exit 1
fi

echo "OK: no non-test Rust file exceeded configured limits."
