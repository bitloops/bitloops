# Test Artefacts Current Schema Alignment

**JIRA:** [CLI-1486](https://bitloops.atlassian.net/browse/CLI-1486)
**Date:** 2026-03-24
**Status:** Approved design

## Problem

The core production artefact storage has two layers:

- **Live layer** (`artefacts_current`, `artefact_edges_current`) — current workspace state
- **Historical layer** (`artefacts`, `artefact_edges`) — committed history

Test artefacts do not follow this pattern. They use separate tables (`test_suites`, `test_scenarios`, `test_links`) with no `_current` equivalent, no `symbol_id`/`artefact_id` duality, and no blob-aware incremental change model.

Additionally, `test_links.production_artefact_id` and `coverage_hits.production_artefact_id` reference the **historical** `artefacts` table rather than the **live** `artefacts_current` table. This means test-to-production relationships can become stale or point to the wrong layer.

## Solution

Align test artefact storage with the production pattern by:

1. Consolidating `test_suites` and `test_scenarios` into a single `test_artefacts_current` table (suite/scenario becomes a `canonical_kind`)
2. Replacing `test_links` with `test_artefact_edges_current` mirroring `artefact_edges_current`
3. Using `symbol_id` (stable logical identity) for all cross-table references instead of `artefact_id` (revision-specific)
4. Updating operational tables (`test_runs`, `test_classifications`, `coverage_*`) to reference `symbol_id`

## Delivery phases

- **Phase 1:** Schema migration and full reparse — migrate to new tables/types/FKs, every ingest does a full reparse. `blob_sha` and `content_hash` are populated but not used for skip decisions.
- **Phase 2:** Incremental change model — use blob/content identity to skip unnecessary work on save, branch switch, and resume after downtime. To be revisited after Phase 1 is validated.

---

# Phase 1 — Schema Migration & Full Reparse

## Architectural context

The test harness is a **capability pack** (`capability_packs/test_harness`). It owns its own storage, persistence, identity logic, and query code. It follows the same algorithms and patterns as core but does not import core's internal modules directly. See [layered-extension-architecture-capability-packs.md](../../layered-extension-architecture-capability-packs.md).

## Schema

### `test_artefacts_current`

Mirrors `artefacts_current`. Primary key: `(repo_id, symbol_id)`.

| Column | Type | Notes |
|--------|------|-------|
| `artefact_id` | TEXT NOT NULL | Revision/blob-specific, changes on edit |
| `symbol_id` | TEXT NOT NULL | Stable logical identity |
| `repo_id` | TEXT NOT NULL | |
| `commit_sha` | TEXT NOT NULL | |
| `blob_sha` | TEXT NOT NULL | For incremental change detection (Phase 2) |
| `path` | TEXT NOT NULL | |
| `language` | TEXT NOT NULL | |
| `canonical_kind` | TEXT NOT NULL | `'test_suite'` or `'test_scenario'` |
| `language_kind` | TEXT | e.g. `'describe_block'`, `'test_fn'`, `'#[test]'` |
| `symbol_fqn` | TEXT | |
| `name` | TEXT NOT NULL | Human-readable test name (intentional divergence from production which uses only `symbol_fqn`) |
| `parent_artefact_id` | TEXT | Suite's artefact_id (for scenarios) |
| `parent_symbol_id` | TEXT | Suite's symbol_id (for scenarios) |
| `start_line` | INTEGER NOT NULL | |
| `end_line` | INTEGER NOT NULL | |
| `start_byte` | INTEGER | Nullable — enumerated/macro-generated scenarios may not have byte offsets |
| `end_byte` | INTEGER | Nullable — same reason as start_byte |
| `signature` | TEXT | |
| `modifiers` | TEXT NOT NULL DEFAULT '[]' | |
| `docstring` | TEXT | |
| `content_hash` | TEXT | For skip-if-unchanged optimization (Phase 2) |
| `discovery_source` | TEXT NOT NULL | `'source'`, `'macro_generated'`, `'doctest'`, `'enumeration'` |
| `revision_kind` | TEXT NOT NULL DEFAULT 'commit' | `'commit'` or `'temporary'` — tracks provenance |
| `revision_id` | TEXT NOT NULL DEFAULT '' | Specific revision identifier (e.g. commit SHA) |
| `updated_at` | TEXT DEFAULT (datetime('now')) | |

Indexes:
- `(repo_id, path)`
- `(repo_id, canonical_kind)`
- `(repo_id, parent_symbol_id)` — supports "find all scenarios for a suite" queries

### `test_artefact_edges_current`

Mirrors `artefact_edges_current`. Links test symbols to production symbols.

| Column | Type | Notes |
|--------|------|-------|
| `edge_id` | TEXT PRIMARY KEY | Deterministic — see identity section |
| `repo_id` | TEXT NOT NULL | |
| `commit_sha` | TEXT NOT NULL | |
| `blob_sha` | TEXT NOT NULL | |
| `path` | TEXT NOT NULL | Source test file |
| `from_artefact_id` | TEXT NOT NULL | Test artefact_id |
| `from_symbol_id` | TEXT NOT NULL | Test symbol_id |
| `to_artefact_id` | TEXT | Production artefact_id (nullable — NULL if target deleted) |
| `to_symbol_id` | TEXT | Production symbol_id |
| `to_symbol_ref` | TEXT | Unresolved reference string |
| `edge_kind` | TEXT NOT NULL | e.g. `'tests'`, `'calls'`, `'imports'` |
| `language` | TEXT NOT NULL | |
| `start_line` | INTEGER | |
| `end_line` | INTEGER | |
| `metadata` | TEXT DEFAULT '{}' | Evidence JSON, confidence, link_source, linkage_status |
| `revision_kind` | TEXT NOT NULL DEFAULT 'commit' | `'commit'` or `'temporary'` |
| `revision_id` | TEXT NOT NULL DEFAULT '' | |
| `updated_at` | TEXT DEFAULT (datetime('now')) | |

Constraints:
- `CHECK (to_symbol_id IS NOT NULL OR to_symbol_ref IS NOT NULL)` — at least one target reference must exist
- `UNIQUE (repo_id, from_symbol_id, edge_kind, to_symbol_id, to_symbol_ref)` — natural key prevents duplicate edges

Indexes:
- `(repo_id, from_symbol_id)`
- `(repo_id, to_symbol_id)`
- `(repo_id, path)` — supports per-file edge cleanup

### Operational table FK changes

These tables stay as separate tables. Only their foreign key columns change:

| Table | Old column | New column | References |
|-------|-----------|------------|------------|
| `test_runs` | `test_scenario_id` | `test_symbol_id` | `test_artefacts_current.symbol_id` |
| `test_classifications` | `test_scenario_id` | `test_symbol_id` | `test_artefacts_current.symbol_id` |
| `coverage_captures` | `subject_test_scenario_id` | `subject_test_symbol_id` | `test_artefacts_current.symbol_id` |
| `coverage_hits` | `production_artefact_id` | `production_symbol_id` | `artefacts_current.symbol_id` |

**`coverage_hits` primary key change:** The current PK is `(capture_id, production_artefact_id, line, branch_id)`. This becomes `(capture_id, production_symbol_id, line, branch_id)`. Since `capture_id` already scopes to a specific capture run, and `symbol_id` is unique per logical symbol, the deduplication semantics remain correct — you cannot have two coverage hits for the same symbol+line+branch within the same capture.

Tables unchanged: `test_discovery_runs`, `test_discovery_diagnostics`, `coverage_diagnostics`.

### Tables dropped

- `test_suites` — absorbed into `test_artefacts_current`
- `test_scenarios` — absorbed into `test_artefacts_current`
- `test_links` — replaced by `test_artefact_edges_current`

## Identity model

### Test symbol_id (stable logical identity)

Derived deterministically from semantic properties:
```
symbol_id = deterministic_uuid(
    path | canonical_kind | language_kind | parent_symbol_id | name | normalized_signature
)
```

- Survives: formatting changes, body edits, line shifts
- Changes on: rename, signature change, move to different file/parent

### Test artefact_id (revision-specific identity)

Derived from blob + symbol:
```
artefact_id = deterministic_uuid(
    repo_id | blob_sha | symbol_id
)
```

Changes every time the file content changes (new blob SHA).

### Suite-scenario relationship

Scenarios reference their parent suite via `parent_symbol_id` / `parent_artefact_id` on `test_artefacts_current`. This mirrors how production artefacts model nesting (e.g. method inside a class).

### Why symbol_id for cross-table references

Production `artefacts_current` uses UPSERT on PK `(repo_id, symbol_id)`. When a file is saved:
- `artefact_id` **changes** (blob-specific)
- `symbol_id` **stays the same** (semantic identity)
- Rows are updated in place, not deleted and reinserted
- No CASCADE constraints on `artefacts_current`

If test edges referenced `artefact_id`, they would become stale on every save. Referencing `symbol_id` means links survive code edits.

This is the same pattern `artefact_edges_current` already uses (`from_symbol_id` / `to_symbol_id`).

## Rust type changes

### Types added

```rust
pub struct TestArtefactCurrentRecord {
    pub artefact_id: String,
    pub symbol_id: String,
    pub repo_id: String,
    pub commit_sha: String,
    pub blob_sha: String,
    pub path: String,
    pub language: String,
    pub canonical_kind: String,      // "test_suite" or "test_scenario"
    pub language_kind: Option<String>,
    pub symbol_fqn: Option<String>,
    pub name: String,
    pub parent_artefact_id: Option<String>,
    pub parent_symbol_id: Option<String>,
    pub start_line: i64,
    pub end_line: i64,
    pub start_byte: Option<i64>,
    pub end_byte: Option<i64>,
    pub signature: Option<String>,
    pub modifiers: String,
    pub docstring: Option<String>,
    pub content_hash: Option<String>,
    pub discovery_source: String,
    pub revision_kind: String,
    pub revision_id: String,
}

pub struct TestArtefactEdgeCurrentRecord {
    pub edge_id: String,
    pub repo_id: String,
    pub commit_sha: String,
    pub blob_sha: String,
    pub path: String,
    pub from_artefact_id: String,
    pub from_symbol_id: String,
    pub to_artefact_id: Option<String>,
    pub to_symbol_id: Option<String>,
    pub to_symbol_ref: Option<String>,
    pub edge_kind: String,
    pub language: String,
    pub start_line: Option<i64>,
    pub end_line: Option<i64>,
    pub metadata: String,
    pub revision_kind: String,
    pub revision_id: String,
}
```

### Types dropped

- `TestSuiteRecord`
- `TestScenarioRecord`
- `TestLinkRecord`

### Types updated

- `TestRunRecord` — `test_scenario_id` becomes `test_symbol_id`
- `TestClassificationRecord` — `test_scenario_id` becomes `test_symbol_id`
- `CoverageCaptureRecord` — `subject_test_scenario_id` becomes `subject_test_symbol_id`
- `CoverageHitRecord` — `production_artefact_id` becomes `production_symbol_id`

## New module: `capability_packs/test_harness/identity.rs`

Pack-owned identity functions following the same algorithm as core:

- `test_structural_symbol_id(...)` — deterministic symbol identity from semantic properties
- `test_revision_artefact_id(repo_id, blob_sha, symbol_id)` — blob-specific artefact identity
- `test_edge_id(repo_id, from_symbol_id, edge_kind, to_symbol_id_or_ref)` — deterministic edge identity derived from: `repo_id | from_symbol_id | edge_kind | to_symbol_id OR to_symbol_ref`

These duplicate the core algorithm intentionally — the test harness capability pack does not import core internals.

## Persistence changes

### New persistence module

`capability_packs/test_harness/persistence.rs` (or within existing storage module):

- `upsert_test_artefact_current()` — INSERT ... ON CONFLICT (repo_id, symbol_id) DO UPDATE
- `upsert_test_artefact_edge_current()` — INSERT ... ON CONFLICT (edge_id) DO UPDATE
- `delete_stale_test_artefacts_for_path()` — remove symbols no longer present in file
- `delete_stale_test_edges_for_path()` — remove edges whose `from_symbol_id` no longer exists (edges are not FK-cascaded from artefacts, so explicit cleanup is required)
- `repair_test_edges()` — set `to_artefact_id = NULL` for edges pointing to deleted production symbols

In Phase 1, every `ingest-tests` does a full reparse — `blob_sha` and `content_hash` are populated but not compared.

### Cleanup strategy replacing `clear_existing_test_discovery_data`

The current `clear_existing_test_discovery_data()` function deletes from `test_links`, `test_scenarios`, `test_suites` by `commit_sha`. With `_current` tables (UPSERT on `symbol_id`, not commit-scoped rows), this changes fundamentally:

- **Before ingest:** no bulk delete by commit_sha
- **Per-file processing:** after reparsing a file, compute the set of surviving `symbol_id`s. Delete any `test_artefacts_current` rows for that `(repo_id, path)` whose `symbol_id` is not in the surviving set. Then delete orphaned edges via `delete_stale_test_edges_for_path()`.

### Edge resolution

When creating `test_artefact_edges_current` rows:
- Resolve `to_symbol_id` and `to_artefact_id` by looking up the production target in `artefacts_current`
- If production symbol not found, set both to NULL and store the unresolved reference in `to_symbol_ref`

### Edge case: production artefacts refreshed, test artefacts not yet re-ingested

- `to_artefact_id` may be stale (old revision's artefact_id)
- `to_symbol_id` remains valid (stable identity)
- Queries should JOIN on `to_symbol_id` as primary key
- Next test ingest corrects `to_artefact_id`

## Storage trait changes

Replace current methods on `TestHarnessRepository`:

| Old | New |
|-----|-----|
| `upsert_test_suite()` | `upsert_test_artefact_current()` |
| `upsert_test_scenario()` | `upsert_test_artefact_current()` |
| `upsert_test_link()` | `upsert_test_artefact_edge_current()` |

Add:
- `delete_stale_test_artefacts_for_path(repo_id, path, surviving_symbol_ids)`
- `delete_stale_test_edges_for_path(repo_id, path, surviving_from_symbol_ids)`
- `repair_test_edges(repo_id, deleted_production_symbol_ids)`
- `load_test_artefacts_for_path(repo_id, path)` — for future Phase 2 comparison

## Query layer changes

`stage_serving.rs` queries that currently JOIN `test_scenarios` + `test_suites` + `test_links` are rewritten to JOIN `test_artefacts_current` + `test_artefact_edges_current`.

Parent suite info is retrieved via self-join on `test_artefacts_current`:
```sql
SELECT
    ta.symbol_id, ta.name, parent.name AS suite_name, ta.path,
    te.metadata, ta.discovery_source
FROM test_artefact_edges_current te
JOIN test_artefacts_current ta ON ta.symbol_id = te.from_symbol_id AND ta.repo_id = te.repo_id
LEFT JOIN test_artefacts_current parent ON parent.symbol_id = ta.parent_symbol_id AND parent.repo_id = ta.repo_id
WHERE te.repo_id = ? AND te.to_symbol_id = ?
```

Confidence, link_source, and linkage_status move into `test_artefact_edges_current.metadata` (JSON).

### Stage handler caller changes

Current stage handlers pass a production `artefact_id` to query functions like `load_stage_covering_tests`. After migration, callers must resolve the production artefact's `symbol_id` (from the query context or by looking up `artefacts_current`) before calling into test harness queries that now filter on `to_symbol_id`.

### Coverage stage queries

The `coverage()` stage queries in `stage_serving.rs` that filter on `ch.production_artefact_id` must be rewritten to filter on `ch.production_symbol_id`. This includes:
- `load_stage_coverage_pair_stats` — coverage hit lookups by production target
- `load_stage_line_coverage` / `load_stage_branch_coverage` — per-line/branch queries
- Any summary queries that aggregate by production artefact

## Ingestion path changes

| File | Change |
|------|--------|
| `mapping/materialize.rs` | Emit `TestArtefactCurrentRecord` instead of separate suite/scenario records |
| `ingest/tests.rs` | Call new persistence path, write `test_artefacts_current` + `test_artefact_edges_current` |
| `ingest/coverage.rs` | Resolve against `test_artefacts_current.symbol_id` and `artefacts_current.symbol_id` |
| `ingest/results.rs` | Resolve against `test_artefacts_current.symbol_id` |

## Schema applies to both SQLite and Postgres

The test harness currently has two schema definitions:
- SQLite: `storage/init.rs` (`TEST_DOMAIN_SCHEMA_SQL`)
- Postgres: `capability_packs/test_harness/storage/schema.rs`

Both must be updated. Type differences follow existing conventions: `INTEGER` (SQLite) vs `BIGINT` (Postgres), `TEXT DEFAULT (datetime('now'))` (SQLite) vs `TIMESTAMPTZ DEFAULT now()` (Postgres).

## Migration policy

No backward-compatible DB migration. Bump schema version. Old DBs must be recreated. Fail fast if old tables are detected without new ones.

## Acceptance criteria

### Schema
- `test_artefacts_current` and `test_artefact_edges_current` exist and are populated
- `test_suites`, `test_scenarios`, `test_links` no longer exist
- Operational tables use `symbol_id` references

### Ingestion
- `ingest-tests` writes `test_artefacts_current` and `test_artefact_edges_current`
- `ingest-coverage` writes `coverage_hits` with `production_symbol_id`
- `ingest-results` writes `test_runs` with `test_symbol_id`

### Query
- `tests()` stage returns covering tests with suite names, confidence, discovery source
- `coverage()` stage returns coverage data
- All existing query behaviors preserved

### Integrity
- `test_artefact_edges_current.to_symbol_id` points to valid production symbols in `artefacts_current` (or NULL if unresolved)
- Deleting test artefacts does not affect production artefacts
- Deleting production artefacts does not delete test data (edges get NULL targets)

### Cleanup
- After removing a test from a file and re-ingesting, the old `test_artefacts_current` row and its edges are deleted
- After deleting a production artefact and re-ingesting tests, edges targeting it have `to_symbol_id`/`to_artefact_id` set to NULL

### Migration
- Schema version is bumped; opening an old DB with new code fails fast with a clear error
- Both SQLite and Postgres schemas are updated

---

# Phase 2 — Incremental Change Model (Future)

> Phase 2 will be revisited after Phase 1 is migrated and validated. This section documents the intended design at a high level.

## Goal

Use `blob_sha` / `content_hash` to skip unnecessary parsing work. Three triggers:

### On local file save

1. Compute content hash / blob-equivalent for saved file
2. Compare against existing `test_artefacts_current` rows for that `(repo_id, path)`
3. **Blob unchanged** → skip entirely
4. **Blob changed** → reparse single file, UPSERT test artefacts, delete removed symbols, re-resolve edges

### On branch switch

1. Capture old file→blob mapping from `test_artefacts_current`
2. After checkout, compute new file→blob mapping
3. Diff:
   - **Same blob** → skip
   - **Changed blob** → reparse file
   - **New file** → parse fresh
   - **Deleted file** → delete test artefacts and edges for that path
4. Edge repair pass: re-resolve production targets, NULL out stale references, fill in newly available matches

### On re-enable after downtime

1. Scan workspace, enumerate test files, compute blob SHAs
2. Compare against existing `test_artefacts_current` state
3. Same logic as branch switch but treats current DB state as "old" and filesystem as "new"
4. Edge repair pass

### Edge cases to address in Phase 2

- **Production artefacts refreshed but test artefacts not re-ingested:** `to_artefact_id` may be stale, `to_symbol_id` remains valid. Queries JOIN on `to_symbol_id`. Next test ingest corrects `to_artefact_id`.
- **Test file renamed (same content, different path):** Old path rows deleted, new path rows inserted. `symbol_id` changes because path is part of identity. Correct behavior.
- **Test file deleted:** All test artefact rows for path deleted. Edges cleaned up. Operational tables retain historical data.
- **Concurrent rapid saves:** Last-write-wins via UPSERT. SQLite write serialization handles concurrency.
- **New test added to existing file:** New `symbol_id` → INSERT with no conflict.
