#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE_DIR="$ROOT_DIR/testlens-fixture"
DB_PATH="${1:-$ROOT_DIR/testlens.db}"
COMMIT_SHA="${2:-fixture-dev}"
LCOV_PATH="$FIXTURE_DIR/coverage/lcov.info"
RESULTS_JSON_PATH="$ROOT_DIR/test-results.json"

echo "Building fixture outputs and ingesting into TestLens"
echo "  fixture: $FIXTURE_DIR"
echo "  db:      $DB_PATH"
echo "  commit:  $COMMIT_SHA"

if [[ ! -d "$FIXTURE_DIR/node_modules" ]]; then
  echo "Installing fixture dependencies..."
  (cd "$FIXTURE_DIR" && npm install)
fi

echo "Running fixture coverage + Jest JSON output (one test fails intentionally by design)..."
(
  cd "$FIXTURE_DIR"
  npx jest --coverage --json --outputFile="$RESULTS_JSON_PATH" --runInBand || true
)

if [[ ! -f "$LCOV_PATH" ]]; then
  echo "Expected LCOV file not found: $LCOV_PATH" >&2
  exit 1
fi

cargo run --manifest-path "$ROOT_DIR/Cargo.toml" -- \
  ingest-tests \
  --db "$DB_PATH" \
  --repo-dir "$FIXTURE_DIR" \
  --commit "$COMMIT_SHA"

cargo run --manifest-path "$ROOT_DIR/Cargo.toml" -- \
  ingest-coverage \
  --db "$DB_PATH" \
  --lcov "$LCOV_PATH" \
  --commit "$COMMIT_SHA"

cargo run --manifest-path "$ROOT_DIR/Cargo.toml" -- \
  ingest-results \
  --db "$DB_PATH" \
  --jest-json "$RESULTS_JSON_PATH" \
  --commit "$COMMIT_SHA"

if command -v sqlite3 >/dev/null 2>&1; then
  echo
  echo "Ingest verification"
  echo "test_scenarios: $(sqlite3 "$DB_PATH" "select count(*) from artefacts where commit_sha='$COMMIT_SHA' and canonical_kind='test_scenario';")"
  echo "test_links rows: $(sqlite3 "$DB_PATH" "select count(*) from test_links where commit_sha='$COMMIT_SHA';")"
  echo "test_coverage rows: $(sqlite3 "$DB_PATH" "select count(*) from test_coverage where commit_sha='$COMMIT_SHA';")"
  echo "test_runs rows: $(sqlite3 "$DB_PATH" "select count(*) from test_runs where commit_sha='$COMMIT_SHA';")"
  echo "test_classifications rows: $(sqlite3 "$DB_PATH" "select count(*) from test_classifications where commit_sha='$COMMIT_SHA';")"
fi
