#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE_DIR="$ROOT_DIR/testlens-fixture"
DB_PATH="${1:-$ROOT_DIR/testlens.db}"
COMMIT_SHA="${2:-fixture-dev}"

echo "Initializing TestLens DB"
echo "  root:   $ROOT_DIR"
echo "  db:     $DB_PATH"
echo "  commit: $COMMIT_SHA"

rm -f "$DB_PATH"

cargo run --manifest-path "$ROOT_DIR/Cargo.toml" -- \
  init \
  --db "$DB_PATH"

cargo run --manifest-path "$ROOT_DIR/Cargo.toml" -- \
  ingest-production-artefacts \
  --db "$DB_PATH" \
  --repo-dir "$FIXTURE_DIR" \
  --commit "$COMMIT_SHA"

if command -v sqlite3 >/dev/null 2>&1; then
  echo
  echo "Seed verification"
  sqlite3 "$DB_PATH" ".tables"
  echo "artefacts: $(sqlite3 "$DB_PATH" "select count(*) from artefacts where commit_sha='$COMMIT_SHA';")"
  echo "production files: $(sqlite3 "$DB_PATH" "select count(*) from artefacts where commit_sha='$COMMIT_SHA' and canonical_kind='file' and path like 'src/%';")"
  echo "test_links (expected 0): $(sqlite3 "$DB_PATH" "select count(*) from test_links where commit_sha='$COMMIT_SHA';")"
  echo "test_runs  (expected 0): $(sqlite3 "$DB_PATH" "select count(*) from test_runs where commit_sha='$COMMIT_SHA';")"
fi
