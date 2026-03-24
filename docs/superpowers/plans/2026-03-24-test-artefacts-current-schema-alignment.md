# Test Artefacts Current Schema Alignment — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate test harness storage from separate `test_suites`/`test_scenarios`/`test_links` tables to unified `test_artefacts_current` and `test_artefact_edges_current`, mirroring the production artefact live-layer pattern.

**Architecture:** The test harness is a capability pack (`capability_packs/test_harness`) that owns its own storage, persistence, identity logic, and query code. Changes flow bottom-up: schema → models → identity → storage traits → writes → materialize → ingest → queries → stage serving → tests.

**Tech Stack:** Rust, SQLite (rusqlite), Postgres (tokio-postgres), tree-sitter for test discovery

**Spec:** `docs/superpowers/specs/2026-03-24-test-artefacts-current-schema-alignment-design.md`

**JIRA:** [CLI-1486](https://bitloops.atlassian.net/browse/CLI-1486)

---

## File Structure

### Files to create

| File | Responsibility |
|------|---------------|
| `bitloops/src/capability_packs/test_harness/identity.rs` | Test artefact identity functions (symbol_id, artefact_id, edge_id) |

### Files to modify

| File | What changes |
|------|-------------|
| `bitloops/src/storage/init.rs` (lines 15–204) | Replace `TEST_DOMAIN_SCHEMA_SQL`: drop test_suites/test_scenarios/test_links, add test_artefacts_current/test_artefact_edges_current, update operational table FKs |
| `bitloops/src/storage/init/schema.rs` (lines 273–352) | Update FK references in shared schema definitions (references old test_scenarios, artefacts tables) |
| `bitloops/src/capability_packs/test_harness/storage/schema.rs` | Same schema changes for Postgres variant |
| `bitloops/src/models.rs` (lines 203–391) | Replace TestSuiteRecord/TestScenarioRecord/TestLinkRecord with TestArtefactCurrentRecord/TestArtefactEdgeCurrentRecord; update TestRunRecord, TestClassificationRecord, CoverageCaptureRecord, CoverageHitRecord, TestHarnessCommitCounts |
| `bitloops/src/capability_packs/test_harness/storage.rs` | Update storage repository trait: replace suite/scenario/link methods with artefact_current/edge_current methods; add cleanup/repair methods |
| `bitloops/src/capability_packs/test_harness/storage/sqlite/writes.rs` | Replace upsert_test_suite/upsert_test_scenario/upsert_test_link; add upsert_test_artefact_current, upsert_test_artefact_edge_current, delete_stale_test_artefacts_for_path, delete_stale_test_edges_for_path; update clear_existing_test_discovery_data |
| `bitloops/src/capability_packs/test_harness/storage/sqlite/stage_serving.rs` | Rewrite all 4 query functions to JOIN test_artefacts_current + test_artefact_edges_current |
| `bitloops/src/capability_packs/test_harness/storage/sqlite.rs` | Update trait impl to wire new methods |
| `bitloops/src/capability_packs/test_harness/storage/sqlite/lists.rs` | Update list queries for new table structure |
| `bitloops/src/capability_packs/test_harness/storage/postgres.rs` | Update Postgres trait impl for new methods |
| `bitloops/src/capability_packs/test_harness/storage/postgres/stage_serving.rs` | Rewrite all 4 async query functions |
| `bitloops/src/capability_packs/test_harness/storage/postgres/helpers.rs` | Update row mapping helpers for new record types |
| `bitloops/src/capability_packs/test_harness/storage/postgres/commit_counts.rs` | Update count queries for new table names |
| `bitloops/src/capability_packs/test_harness/storage/dispatch.rs` | Update dispatch routing for new trait methods |
| `bitloops/src/capability_packs/test_harness/mapping/model.rs` | Update StructuralMappingOutput fields (suites/scenarios/links → test_artefacts/test_edges); update StructuralMappingStats |
| `bitloops/src/capability_packs/test_harness/mapping/materialize.rs` | Emit TestArtefactCurrentRecord + TestArtefactEdgeCurrentRecord instead of 3 separate types; update MaterializationContext |
| `bitloops/src/capability_packs/test_harness/mapping/linker.rs` | Resolve production symbol_id from artefacts_current |
| `bitloops/src/capability_packs/test_harness/ingest/tests.rs` | Call new persistence path |
| `bitloops/src/capability_packs/test_harness/ingest/coverage.rs` | Use subject_test_symbol_id and production_symbol_id |
| `bitloops/src/capability_packs/test_harness/ingest/results.rs` | Match against test_artefacts_current.symbol_id |
| `bitloops/src/capability_packs/test_harness/ingest/parse_llvm_json.rs` | Update CoverageHitRecord construction to use production_symbol_id |
| `bitloops/src/capability_packs/test_harness/query.rs` | Resolve production symbol_id before querying; update trait method calls |
| `bitloops/src/capability_packs/test_harness/stages/tests.rs` | Pass symbol_id to stage queries |
| `bitloops/src/capability_packs/test_harness/stages/coverage.rs` | Pass symbol_id to stage queries |
| `bitloops/src/capability_packs/test_harness/stages/tests_summary.rs` | Update commit-level counts for new tables |
| `bitloops/src/capability_packs/test_harness/migrations/initial.rs` | Bump migration version |
| `bitloops/src/capability_packs/test_harness/storage/sqlite/tests.rs` | Update SQLite storage unit tests for new types/queries |
| `bitloops/src/capability_packs/test_harness/storage/postgres/tests.rs` | Update Postgres storage unit tests and fixtures (25+ type references) |
| `bitloops/tests/test_harness_support/mod.rs` | Update fixture builders for new record types |
| `bitloops/tests/test_harness_support/production_seed.rs` | Update seed data to include symbol_id references |
| `bitloops/src/host/devql/tests/cucumber_world.rs` | Update discovered_suites/scenarios/links fields to new types |
| `bitloops/src/host/devql/tests/cucumber_steps/core.rs` | Update BDD step definitions for new types and schema |
| `bitloops/src/host/devql/tests/devql_tests/query_pipeline.rs` | Update inline SQL DDL and TestLinkRecord construction |

---

## Task 1: SQLite Schema — Replace TEST_DOMAIN_SCHEMA_SQL

**Files:**
- Modify: `bitloops/src/storage/init.rs:15-204`
- Modify: `bitloops/src/storage/init/schema.rs:273-352` (shared schema with FK references to old tables)

- [ ] **Step 1: Read the current schema**

Read `bitloops/src/storage/init.rs` lines 15–204 to understand the full `TEST_DOMAIN_SCHEMA_SQL` constant.

- [ ] **Step 2: Write the new schema DDL**

Replace the `TEST_DOMAIN_SCHEMA_SQL` constant. The new schema must:

**Drop these table definitions:**
- `test_suites` + its indexes
- `test_scenarios` + its indexes
- `test_links` + its indexes

**Add these table definitions:**

```sql
CREATE TABLE IF NOT EXISTS test_artefacts_current (
    artefact_id       TEXT    NOT NULL,
    symbol_id         TEXT    NOT NULL,
    repo_id           TEXT    NOT NULL,
    commit_sha        TEXT    NOT NULL,
    blob_sha          TEXT    NOT NULL,
    path              TEXT    NOT NULL,
    language          TEXT    NOT NULL,
    canonical_kind    TEXT    NOT NULL,
    language_kind     TEXT,
    symbol_fqn        TEXT,
    name              TEXT    NOT NULL,
    parent_artefact_id TEXT,
    parent_symbol_id  TEXT,
    start_line        INTEGER NOT NULL,
    end_line          INTEGER NOT NULL,
    start_byte        INTEGER,
    end_byte          INTEGER,
    signature         TEXT,
    modifiers         TEXT    NOT NULL DEFAULT '[]',
    docstring         TEXT,
    content_hash      TEXT,
    discovery_source  TEXT    NOT NULL,
    revision_kind     TEXT    NOT NULL DEFAULT 'commit',
    revision_id       TEXT    NOT NULL DEFAULT '',
    updated_at        TEXT    DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, symbol_id)
);
CREATE INDEX IF NOT EXISTS idx_test_artefacts_current_path
    ON test_artefacts_current (repo_id, path);
CREATE INDEX IF NOT EXISTS idx_test_artefacts_current_kind
    ON test_artefacts_current (repo_id, canonical_kind);
CREATE INDEX IF NOT EXISTS idx_test_artefacts_current_parent
    ON test_artefacts_current (repo_id, parent_symbol_id);

CREATE TABLE IF NOT EXISTS test_artefact_edges_current (
    edge_id           TEXT    PRIMARY KEY,
    repo_id           TEXT    NOT NULL,
    commit_sha        TEXT    NOT NULL,
    blob_sha          TEXT    NOT NULL,
    path              TEXT    NOT NULL,
    from_artefact_id  TEXT    NOT NULL,
    from_symbol_id    TEXT    NOT NULL,
    to_artefact_id    TEXT,
    to_symbol_id      TEXT,
    to_symbol_ref     TEXT,
    edge_kind         TEXT    NOT NULL,
    language          TEXT    NOT NULL,
    start_line        INTEGER,
    end_line          INTEGER,
    metadata          TEXT    DEFAULT '{}',
    revision_kind     TEXT    NOT NULL DEFAULT 'commit',
    revision_id       TEXT    NOT NULL DEFAULT '',
    updated_at        TEXT    DEFAULT (datetime('now')),
    CHECK (to_symbol_id IS NOT NULL OR to_symbol_ref IS NOT NULL)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_test_artefact_edges_current_natural
    ON test_artefact_edges_current (repo_id, from_symbol_id, edge_kind, to_symbol_id, to_symbol_ref);
CREATE INDEX IF NOT EXISTS idx_test_artefact_edges_current_from
    ON test_artefact_edges_current (repo_id, from_symbol_id);
CREATE INDEX IF NOT EXISTS idx_test_artefact_edges_current_to
    ON test_artefact_edges_current (repo_id, to_symbol_id);
CREATE INDEX IF NOT EXISTS idx_test_artefact_edges_current_path
    ON test_artefact_edges_current (repo_id, path);
```

**Update these existing table definitions (FK column renames):**
- `test_runs`: rename `test_scenario_id` → `test_symbol_id`
- `test_classifications`: rename `test_scenario_id` → `test_symbol_id`
- `coverage_captures`: rename `subject_test_scenario_id` → `subject_test_symbol_id`
- `coverage_hits`: rename `production_artefact_id` → `production_symbol_id`, update PK to `(capture_id, production_symbol_id, line, branch_id)`

Keep all other tables unchanged: `test_runs`, `test_classifications`, `coverage_captures`, `coverage_hits`, `coverage_diagnostics`, `test_discovery_runs`, `test_discovery_diagnostics`.

- [ ] **Step 2b: Update storage/init/schema.rs**

Read `bitloops/src/storage/init/schema.rs` lines 273–352. This file contains a second set of schema definitions with FK references to old tables (`test_scenarios`, `artefacts`). Update all FK references to match the new schema:
- References to `test_scenarios(scenario_id)` → remove or update to `test_artefacts_current(symbol_id)`
- References to `artefacts(artefact_id)` → update to `artefacts_current(symbol_id)` where they concern test-domain tables
- Column renames matching the changes above

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p bitloops 2>&1 | head -50`

This will produce many compilation errors in downstream code — that's expected. The important thing is that the schema SQL itself is valid and the constant compiles.

- [ ] **Step 4: Commit**

```bash
git add bitloops/src/storage/init.rs
git commit -m "schema: replace test_suites/test_scenarios/test_links with test_artefacts_current and test_artefact_edges_current (CLI-1486)"
```

---

## Task 2: Postgres Schema — Update schema.rs

**Files:**
- Modify: `bitloops/src/capability_packs/test_harness/storage/schema.rs:1-216`

- [ ] **Step 1: Read the current Postgres schema**

Read `bitloops/src/capability_packs/test_harness/storage/schema.rs` to see the full Postgres DDL.

- [ ] **Step 2: Apply the same changes as Task 1 but with Postgres types**

Same structural changes as the SQLite schema. Type differences:
- `INTEGER` → `BIGINT`
- `TEXT DEFAULT (datetime('now'))` → `TIMESTAMPTZ DEFAULT now()`
- Postgres uses `TEXT` for UUIDs (same as SQLite)

Drop: `test_suites`, `test_scenarios`, `test_links`
Add: `test_artefacts_current`, `test_artefact_edges_current`
Update FK columns in: `test_runs`, `test_classifications`, `coverage_captures`, `coverage_hits`

- [ ] **Step 3: Commit**

```bash
git add bitloops/src/capability_packs/test_harness/storage/schema.rs
git commit -m "schema(postgres): align test harness Postgres schema with new test_artefacts_current tables (CLI-1486)"
```

---

## Task 3: Rust Models — Replace and Update Record Types

**Files:**
- Modify: `bitloops/src/models.rs:203-391`

- [ ] **Step 1: Read the current model definitions**

Read `bitloops/src/models.rs` lines 200–400 to see the full record types and their usages.

- [ ] **Step 2: Replace TestSuiteRecord, TestScenarioRecord, TestLinkRecord**

Delete `TestSuiteRecord` (lines 203–217), `TestScenarioRecord` (lines 220–235), and `TestLinkRecord` (lines 238–249).

Add in their place:

```rust
#[derive(Debug, Clone)]
pub struct TestArtefactCurrentRecord {
    pub artefact_id: String,
    pub symbol_id: String,
    pub repo_id: String,
    pub commit_sha: String,
    pub blob_sha: String,
    pub path: String,
    pub language: String,
    pub canonical_kind: String,
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

#[derive(Debug, Clone)]
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

- [ ] **Step 3: Update TestRunRecord**

Change `test_scenario_id: String` to `test_symbol_id: String`.

- [ ] **Step 4: Update TestClassificationRecord**

Change `test_scenario_id: String` to `test_symbol_id: String`.

- [ ] **Step 5: Update CoverageCaptureRecord**

Change `subject_test_scenario_id: Option<String>` to `subject_test_symbol_id: Option<String>`.

- [ ] **Step 6: Update CoverageHitRecord**

Change `production_artefact_id: String` to `production_symbol_id: String`.

- [ ] **Step 7: Update TestHarnessCommitCounts**

Replace `test_suites`, `test_scenarios`, `test_links` fields with `test_artefacts` and `test_artefact_edges`.

- [ ] **Step 8: Search for all usages of old type names**

Run: `rg 'TestSuiteRecord|TestScenarioRecord|TestLinkRecord|test_scenario_id|subject_test_scenario_id|production_artefact_id' bitloops/src/`

This will show all compilation errors we need to fix in later tasks. Don't fix them now — just verify the model changes are correct.

- [ ] **Step 9: Commit**

```bash
git add bitloops/src/models.rs
git commit -m "models: replace TestSuiteRecord/TestScenarioRecord/TestLinkRecord with TestArtefactCurrentRecord/TestArtefactEdgeCurrentRecord (CLI-1486)"
```

---

## Task 4: Identity Module — Create test_harness/identity.rs

**Files:**
- Create: `bitloops/src/capability_packs/test_harness/identity.rs`
- Modify: `bitloops/src/capability_packs/test_harness/mod.rs` (add `pub mod identity;`)

- [ ] **Step 1: Read the core identity module for reference**

Read `bitloops/src/host/devql/ingestion/artefact_identity.rs` to understand the production identity algorithm. Pay attention to `structural_symbol_id_for_artefact()` and `revision_artefact_id()`.

- [ ] **Step 2: Read the deterministic_uuid helper**

Find where `deterministic_uuid` is defined (likely in `bitloops/src/utils/` or similar). The test harness module needs to use the same UUID generation function.

- [ ] **Step 3: Write the identity module**

Create `bitloops/src/capability_packs/test_harness/identity.rs`:

```rust
//! Test artefact identity functions.
//!
//! These duplicate the core production identity algorithm intentionally —
//! the test harness capability pack does not import core internals.

use uuid::Uuid;

/// Deterministic UUID v5 from an input string, using a fixed namespace.
fn deterministic_uuid(input: &str) -> String {
    // Use the same namespace UUID as core's deterministic_uuid
    let namespace = Uuid::NAMESPACE_URL;
    Uuid::new_v5(&namespace, input.as_bytes()).to_string()
}

/// Stable logical identity for a test artefact (suite or scenario).
///
/// Survives: formatting changes, body edits, line shifts.
/// Changes on: rename, signature change, move to different file/parent.
pub fn test_structural_symbol_id(
    path: &str,
    canonical_kind: &str,
    language_kind: Option<&str>,
    parent_symbol_id: Option<&str>,
    name: &str,
    signature: Option<&str>,
) -> String {
    let normalized_sig = signature
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    deterministic_uuid(&format!(
        "{}|{}|{}|{}|{}|{}",
        path,
        canonical_kind,
        language_kind.unwrap_or("<null>"),
        parent_symbol_id.unwrap_or(""),
        name,
        normalized_sig,
    ))
}

/// Revision-specific identity for a test artefact.
///
/// Changes every time the file content changes (new blob SHA).
pub fn test_revision_artefact_id(repo_id: &str, blob_sha: &str, symbol_id: &str) -> String {
    deterministic_uuid(&format!("{}|{}|{}", repo_id, blob_sha, symbol_id))
}

/// Deterministic edge identity.
pub fn test_edge_id(
    repo_id: &str,
    from_symbol_id: &str,
    edge_kind: &str,
    to_symbol_id_or_ref: &str,
) -> String {
    deterministic_uuid(&format!(
        "{}|{}|{}|{}",
        repo_id, from_symbol_id, edge_kind, to_symbol_id_or_ref,
    ))
}
```

**MANDATORY:** Before writing this file, you MUST:
1. Run: `rg 'fn deterministic_uuid' bitloops/src/` to find core's implementation
2. Read the function to identify the exact namespace UUID used
3. Use the **exact same namespace UUID** in this module — if core uses a custom UUID instead of `Uuid::NAMESPACE_URL`, copy it exactly
4. If the core function is in a shared util module accessible to capability packs, consider re-exporting it instead of duplicating

- [ ] **Step 4: Register the module**

Add `pub mod identity;` to the test_harness module file (`bitloops/src/capability_packs/test_harness/mod.rs` or whichever file declares the test_harness submodules).

- [ ] **Step 5: Verify it compiles**

Run: `cargo check -p bitloops 2>&1 | head -20`

- [ ] **Step 6: Commit**

```bash
git add bitloops/src/capability_packs/test_harness/identity.rs bitloops/src/capability_packs/test_harness/mod.rs
git commit -m "feat: add test artefact identity module (symbol_id, artefact_id, edge_id) (CLI-1486)"
```

---

## Task 5: Storage Trait — Update TestHarnessRepository

**Files:**
- Modify: `bitloops/src/capability_packs/test_harness/storage.rs:1-131`

- [ ] **Step 1: Read the current storage trait**

Read `bitloops/src/capability_packs/test_harness/storage.rs` to see all trait methods.

- [ ] **Step 2: Replace test suite/scenario/link methods**

In the trait definition, replace:
- `upsert_test_suite(&self, record: &TestSuiteRecord) -> ...`
- `upsert_test_scenario(&self, record: &TestScenarioRecord) -> ...`
- `upsert_test_link(&self, record: &TestLinkRecord) -> ...`

With:
- `upsert_test_artefact_current(&self, record: &TestArtefactCurrentRecord) -> ...`
- `upsert_test_artefact_edge_current(&self, record: &TestArtefactEdgeCurrentRecord) -> ...`
- `delete_stale_test_artefacts_for_path(&self, repo_id: &str, path: &str, surviving_symbol_ids: &[String]) -> ...`
- `delete_stale_test_edges_for_path(&self, repo_id: &str, path: &str, surviving_from_symbol_ids: &[String]) -> ...`
- `repair_test_edges(&self, repo_id: &str, deleted_production_symbol_ids: &[String]) -> ...` — sets `to_artefact_id = NULL` and `to_symbol_id = NULL` for edges pointing to deleted production symbols
- `load_test_artefacts_for_path(&self, repo_id: &str, path: &str) -> ...`

- [ ] **Step 3: Update any query trait methods that reference old column names**

Look for methods that accept `artefact_id` parameters for production targets and update to `symbol_id` where needed.

- [ ] **Step 4: Update imports**

Replace `use crate::models::{TestSuiteRecord, TestScenarioRecord, TestLinkRecord}` with `use crate::models::{TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord}`.

- [ ] **Step 5: Commit**

```bash
git add bitloops/src/capability_packs/test_harness/storage.rs
git commit -m "trait: update TestHarnessRepository for test_artefacts_current/test_artefact_edges_current (CLI-1486)"
```

---

## Task 6: SQLite Writes — Implement New Persistence Functions

**Files:**
- Modify: `bitloops/src/capability_packs/test_harness/storage/sqlite/writes.rs:1-687`

- [ ] **Step 1: Read the current write functions**

Read `bitloops/src/capability_packs/test_harness/storage/sqlite/writes.rs` to understand the existing upsert patterns and SQL style.

- [ ] **Step 2: Replace upsert_test_suite (line 433)**

Delete `upsert_test_suite()`. Replace with `upsert_test_artefact_current()`:

```rust
pub(super) fn upsert_test_artefact_current(
    conn: &Connection,
    r: &TestArtefactCurrentRecord,
) -> Result<()> {
    conn.execute(
        "INSERT INTO test_artefacts_current (
            artefact_id, symbol_id, repo_id, commit_sha, blob_sha,
            path, language, canonical_kind, language_kind, symbol_fqn,
            name, parent_artefact_id, parent_symbol_id,
            start_line, end_line, start_byte, end_byte,
            signature, modifiers, docstring, content_hash,
            discovery_source, revision_kind, revision_id
        ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24)
        ON CONFLICT (repo_id, symbol_id) DO UPDATE SET
            artefact_id = excluded.artefact_id,
            commit_sha = excluded.commit_sha,
            blob_sha = excluded.blob_sha,
            path = excluded.path,
            language = excluded.language,
            canonical_kind = excluded.canonical_kind,
            language_kind = excluded.language_kind,
            symbol_fqn = excluded.symbol_fqn,
            name = excluded.name,
            parent_artefact_id = excluded.parent_artefact_id,
            parent_symbol_id = excluded.parent_symbol_id,
            start_line = excluded.start_line,
            end_line = excluded.end_line,
            start_byte = excluded.start_byte,
            end_byte = excluded.end_byte,
            signature = excluded.signature,
            modifiers = excluded.modifiers,
            docstring = excluded.docstring,
            content_hash = excluded.content_hash,
            discovery_source = excluded.discovery_source,
            revision_kind = excluded.revision_kind,
            revision_id = excluded.revision_id,
            updated_at = datetime('now')",
        params![
            r.artefact_id, r.symbol_id, r.repo_id, r.commit_sha, r.blob_sha,
            r.path, r.language, r.canonical_kind, r.language_kind, r.symbol_fqn,
            r.name, r.parent_artefact_id, r.parent_symbol_id,
            r.start_line, r.end_line, r.start_byte, r.end_byte,
            r.signature, r.modifiers, r.docstring, r.content_hash,
            r.discovery_source, r.revision_kind, r.revision_id,
        ],
    )?;
    Ok(())
}
```

- [ ] **Step 3: Delete upsert_test_scenario (line 474)**

Remove the function entirely — scenarios are now just rows in test_artefacts_current with `canonical_kind = 'test_scenario'`.

- [ ] **Step 4: Replace upsert_test_link (line 517)**

Delete `upsert_test_link()`. Replace with `upsert_test_artefact_edge_current()`:

```rust
pub(super) fn upsert_test_artefact_edge_current(
    conn: &Connection,
    r: &TestArtefactEdgeCurrentRecord,
) -> Result<()> {
    conn.execute(
        "INSERT INTO test_artefact_edges_current (
            edge_id, repo_id, commit_sha, blob_sha, path,
            from_artefact_id, from_symbol_id,
            to_artefact_id, to_symbol_id, to_symbol_ref,
            edge_kind, language, start_line, end_line,
            metadata, revision_kind, revision_id
        ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)
        ON CONFLICT (edge_id) DO UPDATE SET
            commit_sha = excluded.commit_sha,
            blob_sha = excluded.blob_sha,
            path = excluded.path,
            from_artefact_id = excluded.from_artefact_id,
            to_artefact_id = excluded.to_artefact_id,
            to_symbol_id = excluded.to_symbol_id,
            to_symbol_ref = excluded.to_symbol_ref,
            metadata = excluded.metadata,
            revision_kind = excluded.revision_kind,
            revision_id = excluded.revision_id,
            updated_at = datetime('now')",
        params![
            r.edge_id, r.repo_id, r.commit_sha, r.blob_sha, r.path,
            r.from_artefact_id, r.from_symbol_id,
            r.to_artefact_id, r.to_symbol_id, r.to_symbol_ref,
            r.edge_kind, r.language, r.start_line, r.end_line,
            r.metadata, r.revision_kind, r.revision_id,
        ],
    )?;
    Ok(())
}
```

- [ ] **Step 5: Add cleanup functions**

```rust
pub(super) fn delete_stale_test_artefacts_for_path(
    conn: &Connection,
    repo_id: &str,
    path: &str,
    surviving_symbol_ids: &[String],
) -> Result<u64> {
    if surviving_symbol_ids.is_empty() {
        let changed = conn.execute(
            "DELETE FROM test_artefacts_current WHERE repo_id = ?1 AND path = ?2",
            params![repo_id, path],
        )?;
        return Ok(changed as u64);
    }
    let placeholders: Vec<String> = surviving_symbol_ids.iter().enumerate()
        .map(|(i, _)| format!("?{}", i + 3))
        .collect();
    let sql = format!(
        "DELETE FROM test_artefacts_current WHERE repo_id = ?1 AND path = ?2 AND symbol_id NOT IN ({})",
        placeholders.join(",")
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(repo_id.to_string()),
        Box::new(path.to_string()),
    ];
    for id in surviving_symbol_ids {
        params.push(Box::new(id.clone()));
    }
    let changed = conn.execute(&sql, rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())))?;
    Ok(changed as u64)
}

pub(super) fn delete_stale_test_edges_for_path(
    conn: &Connection,
    repo_id: &str,
    path: &str,
    surviving_from_symbol_ids: &[String],
) -> Result<u64> {
    if surviving_from_symbol_ids.is_empty() {
        let changed = conn.execute(
            "DELETE FROM test_artefact_edges_current WHERE repo_id = ?1 AND path = ?2",
            params![repo_id, path],
        )?;
        return Ok(changed as u64);
    }
    let placeholders: Vec<String> = surviving_from_symbol_ids.iter().enumerate()
        .map(|(i, _)| format!("?{}", i + 3))
        .collect();
    let sql = format!(
        "DELETE FROM test_artefact_edges_current WHERE repo_id = ?1 AND path = ?2 AND from_symbol_id NOT IN ({})",
        placeholders.join(",")
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(repo_id.to_string()),
        Box::new(path.to_string()),
    ];
    for id in surviving_from_symbol_ids {
        params.push(Box::new(id.clone()));
    }
    let changed = conn.execute(&sql, rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())))?;
    Ok(changed as u64)
}
```

- [ ] **Step 5b: Add repair_test_edges function**

```rust
pub(super) fn repair_test_edges(
    conn: &Connection,
    repo_id: &str,
    deleted_production_symbol_ids: &[String],
) -> Result<u64> {
    if deleted_production_symbol_ids.is_empty() {
        return Ok(0);
    }
    let placeholders: Vec<String> = deleted_production_symbol_ids.iter().enumerate()
        .map(|(i, _)| format!("?{}", i + 2))
        .collect();
    let sql = format!(
        "UPDATE test_artefact_edges_current
         SET to_artefact_id = NULL, to_symbol_id = NULL, updated_at = datetime('now')
         WHERE repo_id = ?1 AND to_symbol_id IN ({})",
        placeholders.join(",")
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(repo_id.to_string()),
    ];
    for id in deleted_production_symbol_ids {
        params.push(Box::new(id.clone()));
    }
    let changed = conn.execute(&sql, rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())))?;
    Ok(changed as u64)
}
```

- [ ] **Step 6: Update clear_existing_test_discovery_data (line 80)**

This function currently deletes from `test_links`, `test_scenarios`, `test_suites`. Update to delete from `test_artefact_edges_current`, `test_artefacts_current`.

- [ ] **Step 7: Update upsert_test_run (line 552)**

Change the SQL column from `test_scenario_id` to `test_symbol_id`. Update the parameter binding to use `r.test_symbol_id`.

- [ ] **Step 8: Update upsert_test_classification (line 579)**

Same: `test_scenario_id` → `test_symbol_id`.

- [ ] **Step 9: Update imports and fix any compilation issues**

Replace old record type imports with new ones.

- [ ] **Step 10: Commit**

```bash
git add bitloops/src/capability_packs/test_harness/storage/sqlite/writes.rs
git commit -m "sqlite: implement test_artefacts_current/test_artefact_edges_current write operations (CLI-1486)"
```

---

## Task 7: SQLite Stage Serving — Rewrite Query Functions

**Files:**
- Modify: `bitloops/src/capability_packs/test_harness/storage/sqlite/stage_serving.rs:1-227`

- [ ] **Step 1: Read the current stage serving queries**

Read all 4 functions to understand the current JOIN patterns and return types.

- [ ] **Step 2: Rewrite load_stage_covering_tests**

Replace the current query that JOINs `test_links` + `test_scenarios` + `test_suites` with:

```sql
SELECT
    ta.symbol_id,
    ta.name,
    parent.name AS suite_name,
    ta.path,
    json_extract(te.metadata, '$.confidence') AS confidence,
    ta.discovery_source,
    json_extract(te.metadata, '$.link_source') AS link_source,
    json_extract(te.metadata, '$.linkage_status') AS linkage_status
FROM test_artefact_edges_current te
JOIN test_artefacts_current ta
    ON ta.symbol_id = te.from_symbol_id AND ta.repo_id = te.repo_id
LEFT JOIN test_artefacts_current parent
    ON parent.symbol_id = ta.parent_symbol_id AND parent.repo_id = ta.repo_id
WHERE te.repo_id = ?1 AND te.to_symbol_id = ?2
ORDER BY json_extract(te.metadata, '$.confidence') DESC
```

**Note:** The function parameter changes from `production_artefact_id` to `production_symbol_id`.

- [ ] **Step 3: Rewrite load_stage_line_coverage**

Update the WHERE clause to filter on `ch.production_symbol_id = ?` instead of `ch.production_artefact_id = ?`.

- [ ] **Step 4: Rewrite load_stage_branch_coverage**

Same column rename in WHERE clause.

- [ ] **Step 5: Rewrite load_stage_coverage_metadata**

Same column rename in WHERE clause.

- [ ] **Step 6: Commit**

```bash
git add bitloops/src/capability_packs/test_harness/storage/sqlite/stage_serving.rs
git commit -m "sqlite: rewrite stage-serving queries for test_artefacts_current/test_artefact_edges_current (CLI-1486)"
```

---

## Task 8: SQLite Main + Lists — Wire New Trait Methods

**Files:**
- Modify: `bitloops/src/capability_packs/test_harness/storage/sqlite.rs:1-839`
- Modify: `bitloops/src/capability_packs/test_harness/storage/sqlite/lists.rs:1-155`

- [ ] **Step 1: Read the current SQLite trait implementation**

Read `storage/sqlite.rs` to understand how trait methods are wired to the write/query functions.

- [ ] **Step 2: Replace trait method implementations**

Wire the new functions:
- `upsert_test_artefact_current()` → calls `writes::upsert_test_artefact_current()`
- `upsert_test_artefact_edge_current()` → calls `writes::upsert_test_artefact_edge_current()`
- `delete_stale_test_artefacts_for_path()` → calls `writes::delete_stale_test_artefacts_for_path()`
- `delete_stale_test_edges_for_path()` → calls `writes::delete_stale_test_edges_for_path()`
- `load_test_artefacts_for_path()` → calls new query in lists module

Remove old `upsert_test_suite()`, `upsert_test_scenario()`, `upsert_test_link()` implementations.

- [ ] **Step 3: Update lists.rs**

Rewrite list queries to read from `test_artefacts_current` instead of `test_suites`/`test_scenarios`. Update return types to use `TestArtefactCurrentRecord`.

- [ ] **Step 4: Update any remaining method signatures**

Search for any methods that pass `artefact_id` for production lookups and update to `symbol_id`.

- [ ] **Step 5: Commit**

```bash
git add bitloops/src/capability_packs/test_harness/storage/sqlite.rs bitloops/src/capability_packs/test_harness/storage/sqlite/lists.rs
git commit -m "sqlite: wire new trait methods for test_artefacts_current (CLI-1486)"
```

---

## Task 9: Storage Dispatch — Update Routing

**Files:**
- Modify: `bitloops/src/capability_packs/test_harness/storage/dispatch.rs:1-406`

- [ ] **Step 1: Read the dispatch module**

Read `storage/dispatch.rs` to understand how it routes to SQLite/Postgres implementations.

- [ ] **Step 2: Update dispatch routing**

Replace all references to old method names with new ones. The dispatcher delegates to the backend-specific implementation, so update the match arms / function calls to use the new trait methods.

- [ ] **Step 3: Commit**

```bash
git add bitloops/src/capability_packs/test_harness/storage/dispatch.rs
git commit -m "storage: update dispatch routing for new test artefact methods (CLI-1486)"
```

---

## Task 10: Postgres Storage — Update Implementation

**Files:**
- Modify: `bitloops/src/capability_packs/test_harness/storage/postgres.rs:1-959`
- Modify: `bitloops/src/capability_packs/test_harness/storage/postgres/stage_serving.rs:1-206`
- Modify: `bitloops/src/capability_packs/test_harness/storage/postgres/helpers.rs:1-493`
- Modify: `bitloops/src/capability_packs/test_harness/storage/postgres/commit_counts.rs:1-60`
- Modify: `bitloops/src/capability_packs/test_harness/storage/postgres/tests.rs:1-792`

- [ ] **Step 1: Read the Postgres trait implementation**

Read `storage/postgres.rs` to understand how it implements the trait.

- [ ] **Step 2: Implement new Postgres write methods**

Same logic as SQLite writes but using tokio-postgres parameter syntax (`$1`, `$2`, etc.) instead of rusqlite `?1`, `?2`.

- [ ] **Step 3: Rewrite Postgres stage serving queries**

Mirror the SQLite stage_serving changes using Postgres syntax.

- [ ] **Step 4: Update helpers.rs row mapping**

Update the row-to-struct mapping functions for `TestArtefactCurrentRecord` and `TestArtefactEdgeCurrentRecord`.

- [ ] **Step 5: Update commit_counts.rs**

Change table names in count queries: `test_suites` → `test_artefacts_current WHERE canonical_kind = 'test_suite'`, etc. Match the field name changes in `TestHarnessCommitCounts`.

- [ ] **Step 5b: Update postgres/tests.rs**

This file has 25+ references to old types (`TestSuiteRecord`, `TestScenarioRecord`, `TestLinkRecord`) and fixture builders (`stale_suite()`, `suite_record()`, `scenario_record()`, `link_record()`). Update all fixture builders to construct `TestArtefactCurrentRecord` and `TestArtefactEdgeCurrentRecord`. Update all field references (`test_scenario_id` → `test_symbol_id`, `production_artefact_id` → `production_symbol_id`).

Also add `repair_test_edges` to the Postgres trait implementation alongside the other new methods.

- [ ] **Step 6: Commit**

```bash
git add bitloops/src/capability_packs/test_harness/storage/postgres.rs \
       bitloops/src/capability_packs/test_harness/storage/postgres/stage_serving.rs \
       bitloops/src/capability_packs/test_harness/storage/postgres/helpers.rs \
       bitloops/src/capability_packs/test_harness/storage/postgres/commit_counts.rs \
       bitloops/src/capability_packs/test_harness/storage/postgres/tests.rs
git commit -m "postgres: implement test_artefacts_current storage and queries (CLI-1486)"
```

---

## Task 11: Materialize + Mapping Model — Emit New Record Types

**Files:**
- Modify: `bitloops/src/capability_packs/test_harness/mapping/model.rs:1-150`
- Modify: `bitloops/src/capability_packs/test_harness/mapping/materialize.rs:1-332`
- Modify: `bitloops/src/capability_packs/test_harness/mapping/linker.rs:1-324`

- [ ] **Step 0: Update mapping/model.rs**

Read `mapping/model.rs`. This file contains:
- `StructuralMappingOutput` with fields `suites: Vec<TestSuiteRecord>`, `scenarios: Vec<TestScenarioRecord>`, `links: Vec<TestLinkRecord>` (lines ~142-145)
- `StructuralMappingStats` with fields `suites`, `scenarios`, `links` (lines ~133-138)
- Import of `use crate::models::{TestLinkRecord, TestScenarioRecord, TestSuiteRecord};`

Update:
- Replace imports with `use crate::models::{TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord};`
- Change `StructuralMappingOutput` fields to `test_artefacts: Vec<TestArtefactCurrentRecord>` and `test_edges: Vec<TestArtefactEdgeCurrentRecord>`
- Change `StructuralMappingStats` fields from `suites`/`scenarios`/`links` to `test_artefacts: usize` and `test_edges: usize` (or keep granular counts as `test_suites`/`test_scenarios`/`test_edges` if downstream reporting needs them — check how they are used)

- [ ] **Step 1: Read materialize.rs**

Read the full file. Understand `MaterializationContext`, `build_test_suite_record()`, `build_test_scenario_record()`, `build_test_link_record()`.

- [ ] **Step 2: Update MaterializationContext**

Replace:
```rust
pub(crate) suites: &'a mut Vec<TestSuiteRecord>,
pub(crate) scenarios: &'a mut Vec<TestScenarioRecord>,
pub(crate) links: &'a mut Vec<TestLinkRecord>,
```
With:
```rust
pub(crate) test_artefacts: &'a mut Vec<TestArtefactCurrentRecord>,
pub(crate) test_edges: &'a mut Vec<TestArtefactEdgeCurrentRecord>,
```

- [ ] **Step 3: Replace build_test_suite_record**

Create a `build_test_artefact_current_record()` function that constructs `TestArtefactCurrentRecord` with `canonical_kind = "test_suite"` for suites and `canonical_kind = "test_scenario"` for scenarios. Use the identity module to compute `symbol_id` and `artefact_id`.

The function needs `blob_sha` as input — this must be computed from the test file content. Read the test file's content and compute a SHA (or use git's blob hash). In Phase 1, you can compute this from the file content hash.

- [ ] **Step 4: Replace build_test_link_record**

Create a `build_test_artefact_edge_current_record()` that constructs `TestArtefactEdgeCurrentRecord`. Use the identity module's `test_edge_id()`. Resolve the production target's `symbol_id` from `artefacts_current` via the linker.

Store confidence, link_source, and linkage_status in the `metadata` JSON field:
```json
{"confidence": 0.6, "link_source": "static_analysis", "linkage_status": "resolved"}
```

- [ ] **Step 5: Update materialize_source_discovery and materialize_enumerated_scenarios**

Update both functions to push `TestArtefactCurrentRecord` and `TestArtefactEdgeCurrentRecord` instead of separate suite/scenario/link records.

For suites: `canonical_kind = "test_suite"`, `parent_symbol_id = None`.
For scenarios: `canonical_kind = "test_scenario"`, `parent_symbol_id = Some(suite_symbol_id)`.

- [ ] **Step 6: Read and update linker.rs**

Read `mapping/linker.rs`. Update the linker to resolve production `symbol_id` from `artefacts_current` (not `artefact_id` from `artefacts`). The linker currently resolves production artefacts — it needs to return both `symbol_id` and `artefact_id` for the edge record.

- [ ] **Step 7: Commit**

```bash
git add bitloops/src/capability_packs/test_harness/mapping/materialize.rs \
       bitloops/src/capability_packs/test_harness/mapping/linker.rs
git commit -m "materialize: emit TestArtefactCurrentRecord and TestArtefactEdgeCurrentRecord (CLI-1486)"
```

---

## Task 12: Ingest Handlers — Update Persistence Calls

**Files:**
- Modify: `bitloops/src/capability_packs/test_harness/ingest/tests.rs:1-132`
- Modify: `bitloops/src/capability_packs/test_harness/ingest/coverage.rs:1-341`
- Modify: `bitloops/src/capability_packs/test_harness/ingest/parse_llvm_json.rs:1-181`
- Modify: `bitloops/src/capability_packs/test_harness/ingest/results.rs:1-167`

- [ ] **Step 1: Update ingest/tests.rs**

Read the file. Update the `execute()` function to:
- Call `upsert_test_artefact_current()` instead of `upsert_test_suite()` / `upsert_test_scenario()`
- Call `upsert_test_artefact_edge_current()` instead of `upsert_test_link()`
- After processing all files, call `delete_stale_test_artefacts_for_path()` and `delete_stale_test_edges_for_path()` per file to clean up removed tests
- Update the `MaterializationContext` construction to use new output vectors

- [ ] **Step 2: Update ingest/coverage.rs**

Read the file. Update:
- `subject_test_scenario_id` → `subject_test_symbol_id` in `CoverageCaptureRecord` construction
- `production_artefact_id` → `production_symbol_id` in `CoverageHitRecord` construction
- Any lookups that resolve test scenarios or production artefacts must now use `symbol_id`

- [ ] **Step 2b: Update ingest/parse_llvm_json.rs**

Read the file. Line 84 constructs `CoverageHitRecord` with `production_artefact_id`. This is a **semantic change**, not just a rename — the caller must now resolve the production artefact's `symbol_id` (from `artefacts_current`) before constructing `CoverageHitRecord`. Update the function to accept/resolve `production_symbol_id` instead.

- [ ] **Step 3: Update ingest/results.rs**

Read the file. Update:
- Test scenario matching must look up `test_artefacts_current` by `symbol_id` instead of `test_scenarios` by `scenario_id`
- `TestRunRecord` construction uses `test_symbol_id` instead of `test_scenario_id`

- [ ] **Step 4: Commit**

```bash
git add bitloops/src/capability_packs/test_harness/ingest/tests.rs \
       bitloops/src/capability_packs/test_harness/ingest/coverage.rs \
       bitloops/src/capability_packs/test_harness/ingest/results.rs
git commit -m "ingest: update all handlers for test_artefacts_current persistence (CLI-1486)"
```

---

## Task 13: Query Layer + Stages — Update for symbol_id

**Files:**
- Modify: `bitloops/src/capability_packs/test_harness/query.rs:1-636`
- Modify: `bitloops/src/capability_packs/test_harness/stages/tests.rs:1-124`
- Modify: `bitloops/src/capability_packs/test_harness/stages/coverage.rs:1-153`
- Modify: `bitloops/src/capability_packs/test_harness/stages/tests_summary.rs:1-208`

- [ ] **Step 1: Read query.rs**

Understand how it calls `find_artefact()`, `load_covering_tests()`, etc.

- [ ] **Step 2: Update query.rs**

The key change: when the query resolves a production artefact target, it must extract `symbol_id` (not just `artefact_id`) and pass that to test harness query methods like `load_covering_tests()` and `load_coverage_pair_stats()`.

Update method calls to pass `production_symbol_id` instead of `production_artefact_id`.

- [ ] **Step 3: Update stages/tests.rs**

Update the stage handler to pass `symbol_id` to `load_stage_covering_tests()`.

- [ ] **Step 4: Update stages/coverage.rs**

Update to pass `production_symbol_id` to coverage queries.

- [ ] **Step 5: Update stages/tests_summary.rs**

Update commit-level aggregation queries to count from `test_artefacts_current` and `test_artefact_edges_current` instead of `test_suites`, `test_scenarios`, `test_links`.

- [ ] **Step 6: Commit**

```bash
git add bitloops/src/capability_packs/test_harness/query.rs \
       bitloops/src/capability_packs/test_harness/stages/tests.rs \
       bitloops/src/capability_packs/test_harness/stages/coverage.rs \
       bitloops/src/capability_packs/test_harness/stages/tests_summary.rs
git commit -m "query: update query layer and stages for symbol_id-based lookups (CLI-1486)"
```

---

## Task 14: Ingesters (Capability Pack) — Update Linkage/Coverage/Classification

**Files:**
- Modify: `bitloops/src/capability_packs/test_harness/ingesters/linkage.rs:1-79`
- Modify: `bitloops/src/capability_packs/test_harness/ingesters/coverage.rs:1-115`
- Modify: `bitloops/src/capability_packs/test_harness/ingesters/classification.rs:1-70`

- [ ] **Step 1: Read all three ingester files**

These are the capability-pack registered ingesters (different from the `ingest/` command handlers). Understand how they call into storage.

- [ ] **Step 2: Update linkage ingester**

Update to use new record types and storage methods.

- [ ] **Step 3: Update coverage ingester**

Update `production_artefact_id` → `production_symbol_id` references.

- [ ] **Step 4: Update classification ingester**

Update `test_scenario_id` → `test_symbol_id` references.

- [ ] **Step 5: Commit**

```bash
git add bitloops/src/capability_packs/test_harness/ingesters/linkage.rs \
       bitloops/src/capability_packs/test_harness/ingesters/coverage.rs \
       bitloops/src/capability_packs/test_harness/ingesters/classification.rs
git commit -m "ingesters: update capability pack ingesters for new test artefact schema (CLI-1486)"
```

---

## Task 15: Migration Version Bump

**Files:**
- Modify: `bitloops/src/capability_packs/test_harness/migrations/initial.rs:1-16`

- [ ] **Step 1: Read the migration file**

- [ ] **Step 2: Bump the version**

Change version from `0.2.0` to `0.3.0` (or whatever the next version is). The migration function `run_test_harness_domain_schema` should now apply the updated schema with the new tables.

- [ ] **Step 3: Commit**

```bash
git add bitloops/src/capability_packs/test_harness/migrations/initial.rs
git commit -m "migration: bump test harness schema version for test_artefacts_current (CLI-1486)"
```

---

## Task 16: Full Compilation — Fix All Remaining Errors

**Files:**
- Various — whatever the compiler reports

- [ ] **Step 1: Run full cargo check**

Run: `cargo check -p bitloops 2>&1`

- [ ] **Step 2: Fix all compilation errors**

Work through each error systematically. Common patterns:
- Old type names (`TestSuiteRecord`, `TestScenarioRecord`, `TestLinkRecord`) → new types
- Old field names (`test_scenario_id`, `production_artefact_id`) → new names
- Old method calls on storage traits → new methods
- Import paths

- [ ] **Step 3: Run cargo check again until clean**

Run: `cargo check -p bitloops 2>&1`
Expected: `Finished dev profile`

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "fix: resolve all compilation errors from test artefact schema migration (CLI-1486)"
```

---

## Task 17: Update Test Support Fixtures + SQLite Unit Tests

**Files:**
- Modify: `bitloops/tests/test_harness_support/mod.rs:1-704`
- Modify: `bitloops/tests/test_harness_support/production_seed.rs:1-1026`
- Modify: `bitloops/src/capability_packs/test_harness/storage/sqlite/tests.rs:1-222`
- Modify: `bitloops/src/capability_packs/test_harness/mapping/tests.rs:1-628`

- [ ] **Step 1: Read test support files**

Read `bitloops/tests/test_harness_support/mod.rs` and `production_seed.rs`.

- [ ] **Step 2: Update fixture builders in mod.rs**

Replace any `TestSuiteRecord` / `TestScenarioRecord` / `TestLinkRecord` fixture builders with `TestArtefactCurrentRecord` / `TestArtefactEdgeCurrentRecord` builders. Include `symbol_id`, `artefact_id`, `blob_sha` in fixture data.

- [ ] **Step 3: Update production_seed.rs**

Update seed data to include `symbol_id` where needed, so test edge fixtures can reference valid production symbols.

- [ ] **Step 4: Update SQLite unit tests**

Read `storage/sqlite/tests.rs`. Update fixture construction and assertions for new types.

- [ ] **Step 5: Update mapping unit tests**

Read `mapping/tests.rs`. Update any assertions that expect `TestSuiteRecord`/`TestScenarioRecord`/`TestLinkRecord` output.

- [ ] **Step 6: Commit**

```bash
git add bitloops/tests/test_harness_support/ \
       bitloops/src/capability_packs/test_harness/storage/sqlite/tests.rs \
       bitloops/src/capability_packs/test_harness/mapping/tests.rs
git commit -m "test: update test harness fixtures and unit tests for test_artefacts_current (CLI-1486)"
```

---

## Task 17b: Update Cucumber/BDD Test Files

**Files:**
- Modify: `bitloops/src/host/devql/tests/cucumber_world.rs`
- Modify: `bitloops/src/host/devql/tests/cucumber_steps/core.rs`
- Modify: `bitloops/src/host/devql/tests/devql_tests/query_pipeline.rs`

- [ ] **Step 1: Read cucumber_world.rs**

This file has fields like `discovered_suites: Vec<TestSuiteRecord>`, `discovered_scenarios: Vec<TestScenarioRecord>`, `materialized_links: Vec<TestLinkRecord>`. Update to `test_artefacts: Vec<TestArtefactCurrentRecord>` and `test_edges: Vec<TestArtefactEdgeCurrentRecord>`.

- [ ] **Step 2: Read and update cucumber_steps/core.rs**

This file constructs `TestSuiteRecord`, `TestScenarioRecord`, `TestLinkRecord` directly and queries by `production_artefact_id` and `test_scenario_id`. Update all type constructions and field references.

- [ ] **Step 3: Read and update query_pipeline.rs**

This file contains inline SQL DDL that creates old-style tables and constructs `TestLinkRecord` values. Update the DDL to use new table definitions and update type construction.

- [ ] **Step 4: Run BDD tests**

Run: `cargo test -p bitloops -- cucumber 2>&1 | tail -30`

- [ ] **Step 5: Commit**

```bash
git add bitloops/src/host/devql/tests/
git commit -m "test: update cucumber/BDD tests for test_artefacts_current schema (CLI-1486)"
```

---

## Task 18: Run Full Test Suite and Fix Remaining Failures

**Files:**
- Various

- [ ] **Step 1: Run all tests**

Run: `cargo test -p bitloops 2>&1`

- [ ] **Step 2: Fix failures**

Address each test failure. Common causes:
- Queries expecting old table/column names
- Fixture data missing required fields
- Join paths changed

- [ ] **Step 3: Run tests again until green**

Run: `cargo test -p bitloops 2>&1`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "test: fix all test failures from test artefact schema migration (CLI-1486)"
```

---

## Task 19: Cleanup and Final Verification

- [ ] **Step 1: Search for any remaining references to old tables/types**

Run: `rg 'test_suites|test_scenarios|test_links|TestSuiteRecord|TestScenarioRecord|TestLinkRecord|test_scenario_id|subject_test_scenario_id' bitloops/src/ bitloops/tests/`

All matches should be in comments, docs, or the migration file — not in active code.

- [ ] **Step 2: Run cargo clippy**

Run: `cargo clippy -p bitloops 2>&1 | head -50`

Fix any warnings in changed code.

- [ ] **Step 3: Run full test suite one more time**

Run: `cargo test -p bitloops 2>&1`
Expected: All pass

- [ ] **Step 4: Final commit**

```bash
git add -A
git commit -m "cleanup: remove stale references to old test harness schema (CLI-1486)"
```
