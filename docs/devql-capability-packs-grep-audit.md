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

### 4.1 Semantic clones (strong concentration)

| File | Role |
|------|------|
| `ingestion/semantic_clones_persistence.rs` | Defines **`symbol_clone_edges`** DDL (SQLite + Postgres), `ensure_semantic_clones_schema`, `rebuild_symbol_clone_edges`, load candidates, delete/insert edges. **Primary leak surface** for clone persistence. |
| `ingestion/schema/relational_initialisation.rs` | Calls `init_sqlite_semantic_clones_schema` / `init_postgres_semantic_clones_schema` alongside other relational setup. |
| `ingestion/types.rs` | Counters: `symbol_clone_edges_upserted`, `symbol_clone_sources_scored`. |

**Cross-callers (outside `ingestion/` but part of ingest pipeline):**

- `engine/devql/mod.rs`, `engine/devql/ingest.rs` — invoke `rebuild_symbol_clone_edges` and bump counters.

**Interpretation (superseded in part):** Clone-edge persistence still lives in `ingestion/semantic_clones_persistence.rs`, but **`devql ingest` rebuilds edges via the `semantic_clones` pack ingester** (`invoke_ingester_with_relational` + `CapabilityIngestContext::devql_relational`). SQLite `symbol_clone_edges` DDL is also applied through **pack migrations** on the DevQL capability host (Postgres DDL remains in relational bootstrap).

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
| test_links DDL mixed into `semantic_clones_persistence.rs` | **No**. |
| Single function touching both tables | **Not found** in `ingestion/`; `relational_initialisation.rs` orchestrates **both** init paths sequentially (shared bootstrap file only). |

---

## 5. Summary: leaks and non-leaks

### Clean for stated scope

- **`capabilities/test_harness`:** No Knowledge gateway usage; no raw SQLite/DuckDB in tree.
- **`capabilities/knowledge`:** Gateway usage confined to expected modules; raw SQL drivers isolated under **`storage/`** (+ tests/plugin wiring).

### Architectural “leaks” (by design today, not by wrong import)

1. **Semantic clones:** All **`symbol_clone_edges`** lifecycle in **`semantic_clones_persistence.rs`** + ingest entrypoints — **not** behind `CapabilityIngestContext` / pack ingester in `capability_host`.
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
