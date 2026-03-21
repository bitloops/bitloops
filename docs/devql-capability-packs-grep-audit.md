# Grep-driven audit: capability pack boundaries and ingestion table access

**Purpose:** Quick, repeatable check for “wrong module” use of Knowledge gateways/SQLite and for where **semantic clones** vs **test harness** data is touched under `devql/ingestion` and related paths.

**Date:** 2026-03-20 (repo snapshot).

---

## 1. Methodology (commands to re-run)

From repo root (`bitloops/`):

```bash
# Knowledge gateways: should NOT appear under test_harness
rg 'knowledge_relational|knowledge_documents|KnowledgeRelational|KnowledgeDocument' \
  bitloops_cli/src/engine/devql/capabilities/test_harness

# Direct DB drivers in capability pack trees
rg 'rusqlite|SqliteConnectionPool|sqlx|duckdb::|Connection::open' \
  bitloops_cli/src/engine/devql/capabilities/test_harness
rg 'rusqlite|SqliteConnectionPool|sqlx|duckdb::' \
  bitloops_cli/src/engine/devql/capabilities/knowledge

# Ingestion: semantic clones vs test harness surfaces
rg -i 'semantic_clone|symbol_clone|test_harness|test_links|coverage|linkage' \
  bitloops_cli/src/engine/devql/ingestion --glob '*.rs'

# Broader: who touches test_links / symbol_clone_edges
rg 'test_links' bitloops_cli --glob '*.rs'
rg 'symbol_clone_edges' bitloops_cli --glob '*.rs'
```

---

## 2. `capabilities/test_harness` — Knowledge gateways & raw SQL

| Check | Result |
|--------|--------|
| `knowledge_relational` / `knowledge_documents` / `KnowledgeRelational*` / `KnowledgeDocument*` | **No matches** in `capabilities/test_harness/**`. |
| `rusqlite`, `SqliteConnectionPool`, `sqlx`, `duckdb::`, `Connection::open` | **No matches** in `capabilities/test_harness/**`. |

**Interpretation:** The Test Harness **capability pack module** does not call Knowledge gateways or open SQLite/DuckDB directly. Query-time behaviour is delegated to core DevQL via **`__core_test_links`** (see `stages/tests.rs` → `build_core_test_links_query` + `execute_devql_subquery`).

**Remaining coupling (outside this folder):** Core executor and relational pipeline implement `__core_test_links` against **`test_links`** (`engine/devql/query/executor/relational.rs`). That is **core/test-harness engine** code, not `capabilities/test_harness`, but it means verification data is still **centralised** rather than behind a pack-only gateway inside the capability tree.

---

## 3. `capabilities/knowledge` — `knowledge_*` context usage

| Location | `knowledge_relational` / `knowledge_documents` |
|----------|--------------------------------------------------|
| `knowledge/migrations/initial.rs` | Yes — migration uses gateways (expected). |
| `knowledge/refs.rs` | Yes — resolution helpers (expected). |
| `knowledge/services.rs` | Yes — main pack services (expected). |
| `knowledge/services.rs` (tests) | Test doubles implementing trait (expected). |
| Other knowledge files (ingesters, stages, health, …) | Use context/services; gateway names concentrated above. |

**Direct SQLite / DuckDB / pool usage:**

| Path | Notes |
|------|--------|
| `knowledge/storage/sqlite_relational.rs` | **rusqlite** + `SqliteConnectionPool` — **gateway implementation** (expected location). |
| `knowledge/storage/duckdb_documents.rs` | **duckdb::Connection** — **gateway implementation** (expected). |
| `knowledge/plugin.rs` | Constructs `SqliteKnowledgeRelationalStore` with pool — **host/plugin wiring** (borderline: still knowledge-adjacent). |
| `knowledge/services.rs` (`#[cfg(test)]`) | Constructs `SqliteKnowledgeRelationalStore` for tests. |
| `knowledge/tests.rs` | **Heavy** `rusqlite` / `duckdb` / `SqliteConnectionPool` for assertions — **tests only**; acceptable if kept out of production call paths. |

**Interpretation:** No evidence of **Knowledge pack production code** opening databases outside `storage/` and host wiring. **Leaks to watch:** `plugin.rs` and test-only construction paths should stay the only non-storage constructors of concrete stores.

---

## 4. `devql/ingestion` — semantic clones vs test harness

### 4.1 Semantic clones (pack-owned pipeline)

| File | Role |
|------|------|
| `capabilities/semantic_clones/stage_semantic_features.rs` | Stage 1: **`symbol_semantics`** / **`symbol_features`** DDL init, pre-stage loaders, upsert orchestration. |
| `capabilities/semantic_clones/stage_embeddings.rs` | Stage 2: **`symbol_embeddings`** DDL init, upsert orchestration, **`ensure_semantic_embeddings_schema`**. |
| `capabilities/semantic_clones/pipeline.rs` | Stage 3: **`rebuild_symbol_clone_edges`**, candidate load, delete/insert **`symbol_clone_edges`**, SQLite/Postgres DDL for clone tables (`ensure_semantic_clones_schema`). |
| `capabilities/semantic_clones/schema.rs` | Canonical **`symbol_clone_edges`** DDL strings (shared with migrations + pipeline). |
| `ingestion/schema/relational_initialisation.rs` | Relational bootstrap calls **`init_*_schema`** on stages 1–2 and **`pipeline::init_postgres_semantic_clones_schema`** (Postgres + SQLite base paths). |
| `ingestion/types.rs` | Counters: `symbol_clone_edges_upserted`, `symbol_clone_sources_scored`. |

**Cross-callers:**

- `engine/devql/mod.rs` — `devql ingest` ends with **`invoke_ingester_with_relational`** for `semantic_clones.rebuild`; **`#[cfg(test)] pub(crate) use`** re-exports `rebuild_symbol_clone_edges` at `crate::engine::devql` for **`devql::tests`**.

**Interpretation:** Semantic clone **stages 1–3** persistence and rebuild orchestration live under **`capabilities/semantic_clones`** (not `ingestion/`). Ingestion triggers the pack ingester; pure scoring remains in **`engine/capability_packs/builtin/semantic_clones`**. Postgres bootstrap ensures semantic + clone tables early; pack migrations cover versioned SQLite host paths (see [core ↔ pack boundaries](./devql-core-pack-boundaries.md#relational-ddl-postgres-bootstrap-vs-sqlite-pack-migrations-semantic-stack)).

### 4.2 Test harness / `test_links` (thin in `ingestion/`)

| File | Role |
|------|------|
| `ingestion/schema/relational_initialisation.rs` | Postgres path runs **`test_links_upgrade_sql()`** (adds `confidence`, `linkage_status`). |
| `ingestion/schema/relational_postgres_migrations.rs` | **`test_links_upgrade_sql`** definition. |

**No** `test_links` SELECT/INSERT/DELETE inside `devql/ingestion/**` beyond these migrations.

**Where `test_links` is actually read/written:**

- `engine/devql/query/executor/relational.rs` — **`__core_test_links`** pipeline (read).
- `engine/test_harness/postgres/*.rs`, `repository/sqlite/*.rs`, `db/mod.rs`, `db/schema.rs` — writes and queries.
- `app/commands/ingest_coverage.rs` — documents **`coverage_hits`** path; comment states **no fan-out through `test_links`**.

**Interpretation:** Under `devql/ingestion`, **test harness** shows up almost only as **schema migration** for `test_links`. Runtime access is **split** between **DevQL relational executor**, **`engine/test_harness`**, and **repository** layers — **not** the Test Harness capability pack’s ingesters.

### 4.3 Cross-feature bleed

| Concern | In `devql/ingestion`? |
|---------|------------------------|
| Semantic clones SQL mixed into test_links DDL | **No** — separate modules/strings. |
| test_links DDL mixed into semantic clones pipeline | **No** (clone DDL in **`capabilities/semantic_clones/schema.rs`** / **`pipeline.rs`**). |
| Single function touching both tables | **Not found** in `ingestion/`; `relational_initialisation.rs` orchestrates **both** init paths sequentially (shared bootstrap file only). |

---

## 5. Summary: leaks and non-leaks

### Clean for stated scope

- **`capabilities/test_harness`:** No Knowledge gateway usage; no raw SQLite/DuckDB in tree.
- **`capabilities/knowledge`:** Gateway usage confined to expected modules; raw SQL drivers isolated under **`storage/`** (+ tests/plugin wiring).

### Architectural “leaks” (by design today, not by wrong import)

1. **Semantic clones:** **`symbol_clone_edges`** rebuild orchestration is in **`capabilities/semantic_clones/pipeline.rs`**; **`devql ingest`** triggers **`semantic_clones.rebuild`** via **`invoke_ingester_with_relational`** (scoped relational on `CapabilityIngestContext`). **Residual:** stage 1–2 tables still written from **`devql/ingestion`**.
2. **Test harness data:** **`test_links`** migrated in ingestion schema init, but **read/write** lives in **core executor + `engine/test_harness` + repository**, while **`capabilities/test_harness`** stages **compose** into `__core_test_links` instead of owning storage gateways.
3. **Shared bootstrap:** `relational_initialisation.rs` is a **choke point** that knows about **both** semantic clones schema and test_links upgrades — acceptable operationally, but it **couples** “relational init” to multiple capability domains in one file.

---

## 6. Suggested follow-up greps (regression watch)

```bash
# Fail if test_harness pack starts touching knowledge stores
rg 'knowledge_relational|knowledge_documents' bitloops_cli/src/engine/devql/capabilities/test_harness && exit 1

# Fail if test_harness pack opens DB drivers directly
rg 'rusqlite|SqliteConnectionPool|duckdb::' bitloops_cli/src/engine/devql/capabilities/test_harness && exit 1

# Optional: flag new ingestion writers for symbol_clone_edges outside persistence module
rg 'symbol_clone_edges' bitloops_cli/src/engine/devql/ingestion --glob '*.rs'
```

---

*Companion to [devql-capability-packs-implementation-gaps.md](./devql-capability-packs-implementation-gaps.md).*
