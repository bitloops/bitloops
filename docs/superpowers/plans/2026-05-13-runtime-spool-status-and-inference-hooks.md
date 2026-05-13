# Runtime Spool Status And Inference Hooks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove runtime DB write-lock pressure from read-only architecture role status flows and prevent daemon-owned local inference agent subprocesses from re-entering Bitloops hooks.

**Architecture:** Split interaction spool schema creation from interaction search projection rebuilds, then make `architecture roles status` use explicit read-only status readers instead of a full current-state consumer context. Separately, define an internal hook-suppression environment contract that is set only on Bitloops-managed CLI-agent inference launcher processes and honored at the top of the agent hook dispatcher.

**Tech Stack:** Rust, rusqlite, SQLite WAL, Tokio, serde/serde_json, clap command routing, cargo-nextest-backed repo test lanes.

---

## Root Cause To Keep Fixed

The observed command failed here:

```text
Error: initialising interaction spool schema in runtime db: rebuilding interaction search projections: database is locked
```

That message is accurate but misleading for a status command. The lock comes from this chain:

- `bitloops/src/cli/devql/architecture.rs` builds `DevqlCapabilityHost` and `host.build_current_state_consumer_context("architecture_graph")` before it knows whether the user asked for read-only status.
- `build_current_state_consumer_context` constructs a capability workplane gateway.
- `LocalCapabilityWorkplaneGateway::new` opens `RepoSqliteRuntimeStore`.
- `RepoSqliteRuntimeStore::open_for_roots_with_repo_id` calls `initialise_repo_runtime_schema`.
- `initialise_repo_runtime_schema` opens `SqliteInteractionSpool::new(sqlite, "__runtime-bootstrap__")`.
- `SqliteInteractionSpool::new` currently creates schema and runs `projections::rebuild_all_projections`.
- `rebuild_all_projections` starts `BEGIN IMMEDIATE`, so it competes with active daemon workers and hook processes even though status only needs queue rows and relational review rows.

There is a second contributor during local `codex_exec` inference:

- The daemon launches `bitloops-inference` as an internal structured-generation runtime.
- `bitloops-inference` launches Codex as the provider process.
- The provider Codex process inherits the repo's Codex hook configuration.
- Codex invokes `bitloops hooks codex session-start` and `bitloops hooks codex user-prompt-submit`.
- Those hook handlers write interaction/runtime state while the daemon is also processing runtime workplane jobs.

This plan fixes both boundaries:

1. Runtime/status: schema bootstrap stays schema-only, projection rebuild becomes explicit, and `roles status` reads only what it needs.
2. Inference/hooks: internal inference subprocess trees carry an env guard, and agent hooks exit before repo/runtime access when that guard is present.

## File Map

- Modify `bitloops/src/host/interactions/db_store.rs`: expose schema-only bootstrap and explicit projection rebuild APIs; make `SqliteInteractionSpool::new` schema-only.
- Modify `bitloops/src/host/runtime_store/sqlite_migrate.rs`: call schema-only interaction spool bootstrap during runtime schema initialization.
- Modify `bitloops/src/host/interactions/db_store/tests.rs`: prove spool construction no longer rebuilds projections and explicit rebuild still works.
- Modify `bitloops/src/host/runtime_store/tests.rs`: prove runtime schema bootstrap no longer rebuilds interaction projections through the fake `__runtime-bootstrap__` repo ID.
- Modify `bitloops/src/storage/sqlite.rs`: add `ReadOnlySqliteConnectionPool`.
- Modify `bitloops/src/storage/sqlite/filesystem.rs`: implement read-only SQLite open without file creation, WAL switching, or writable transactions.
- Modify `bitloops/src/storage.rs`: export `ReadOnlySqliteConnectionPool`.
- Modify `bitloops/src/storage/sqlite/tests.rs`: test read-only pool semantics.
- Modify `bitloops/src/host/runtime_store/repo_workplane/jobs.rs`: factor the workplane job list SQL into a connection-level helper reusable by read-write and read-only readers.
- Create `bitloops/src/host/runtime_store/repo_workplane/read_only_status.rs`: read-only workplane status reader for queue/status commands.
- Modify `bitloops/src/host/runtime_store/repo_workplane.rs`: export the read-only status reader.
- Modify `bitloops/src/host/runtime_store.rs`: re-export the read-only status reader.
- Modify `bitloops/src/host/capability_host/host.rs`: add a relational-only storage accessor so read-only CLI status does not need `CurrentStateConsumerContext`.
- Modify `bitloops/src/cli/devql/architecture.rs`: route `roles status` before building current-state consumer context; pass `RelationalStorage` directly to status helpers.
- Modify `bitloops/src/cli/devql/architecture/deterministic_tests.rs`: add focused status-routing and read-only queue-loading tests.
- Modify `bitloops/src/host/hooks.rs`: define the internal hook suppression env var.
- Modify `bitloops/src/host/hooks/dispatcher.rs`: exit agent hook dispatch early when the env var is truthy; leave git hooks unchanged.
- Modify `bitloops/src/host/inference/text_generation.rs`: set the env var on standalone `bitloops-inference` launches for CLI-agent structured generation drivers.
- Modify `bitloops/src/host/inference/text_generation/tests.rs`: prove `codex_exec` launch inherits the hook-suppression env and non-agent runtimes do not.
- Optional follow-up, not part of this plan: fix the strict OpenAI schema failure in `bitloops/src/capability_packs/architecture_graph/roles/llm_executor.rs` where `assignments.items.required` does not include every property.

## Contract Decisions

- `SqliteInteractionSpool::new` must create and migrate schema only. It must not delete, rebuild, or refresh projections.
- Interaction projection rebuild remains available as an explicit method: `SqliteInteractionSpool::rebuild_search_projections`.
- Normal interaction writes continue to maintain projections incrementally through the existing `refresh_*` calls in `spool.rs`.
- Runtime store open paths that create or migrate schema may still execute DDL. They must not run full projection rebuilds.
- `roles status` must not call `build_current_state_consumer_context`, `build_workplane_gateway`, or `RepoSqliteRuntimeStore::open`.
- Missing runtime DB or missing workplane table should produce an empty queue in status, not create a runtime DB.
- Read-only SQLite status reads should open an existing DB only, set `query_only = ON`, and avoid `PRAGMA journal_mode = WAL`.
- Internal hook suppression is intentionally process-env based because Codex hook processes inherit the provider process environment.
- The hook suppression env var suppresses only agent hook dispatch under `bitloops hooks <agent> ...`; it does not suppress `bitloops hooks git ...`.
- Suppressed hooks must return success and emit no hook protocol stdout, because the provider agent expects hook commands to be non-disruptive.

---

### Task 1: Make Interaction Spool Open Schema-Only

**Files:**
- Modify: `bitloops/src/host/interactions/db_store.rs`
- Modify: `bitloops/src/host/runtime_store/sqlite_migrate.rs`
- Test: `bitloops/src/host/interactions/db_store/tests.rs`
- Test: `bitloops/src/host/runtime_store/tests.rs`

- [x] **Step 1: Add the failing spool projection test**

Add this test to `bitloops/src/host/interactions/db_store/tests.rs`:

```rust
#[test]
fn opening_spool_does_not_rebuild_search_projections() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let sqlite = SqliteConnectionPool::connect(dir.path().join("interaction-spool.sqlite"))?;
    initialise_interaction_spool_schema(&sqlite)?;

    sqlite.with_connection(|conn| {
        conn.execute(
            "INSERT INTO interaction_sessions (session_id, repo_id, first_prompt, started_at, updated_at)
             VALUES ('session-1', 'repo-test', 'hello from prompt', '2026-05-13T00:00:00Z', '2026-05-13T00:00:00Z')",
            [],
        )?;
        Ok(())
    })?;

    let _spool = SqliteInteractionSpool::new(sqlite.clone(), "repo-test".into())?;

    let docs_after_open: i64 = sqlite.with_connection(|conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM interaction_session_search_documents WHERE repo_id = 'repo-test'",
            [],
            |row| row.get(0),
        )
        .map_err(anyhow::Error::from)
    })?;
    assert_eq!(
        docs_after_open, 0,
        "constructing a spool must not rebuild interaction search projections"
    );

    Ok(())
}
```

- [x] **Step 2: Run the focused failing spool test**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features --lib opening_spool_does_not_rebuild_search_projections
```

Expected: FAIL because `SqliteInteractionSpool::new` currently calls `rebuild_all_projections`, which creates one search document.

- [x] **Step 3: Add schema-only and explicit rebuild functions**

In `bitloops/src/host/interactions/db_store.rs`, replace the current `SqliteInteractionSpool::new` body with explicit helpers:

```rust
pub fn initialise_interaction_spool_schema(sqlite: &SqliteConnectionPool) -> Result<()> {
    sqlite
        .with_connection(schema::initialise_schema)
        .context("initialising interaction spool schema")
}

pub fn rebuild_interaction_search_projections(
    sqlite: &SqliteConnectionPool,
    repo_id: &str,
) -> Result<()> {
    sqlite
        .with_connection(|conn| projections::rebuild_all_projections(conn, repo_id))
        .context("rebuilding interaction search projections")
}

impl SqliteInteractionSpool {
    pub fn new(sqlite: SqliteConnectionPool, repo_id: String) -> Result<Self> {
        initialise_interaction_spool_schema(&sqlite)?;
        Ok(Self { sqlite, repo_id })
    }

    pub fn rebuild_search_projections(&self) -> Result<()> {
        rebuild_interaction_search_projections(&self.sqlite, &self.repo_id)
    }

    pub fn repo_id(&self) -> &str {
        &self.repo_id
    }

    pub(crate) fn with_connection<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&rusqlite::Connection) -> Result<T>,
    {
        self.sqlite.with_connection(f)
    }
}
```

Keep the existing `ensure_repo_id` function unchanged.

- [x] **Step 4: Use schema-only bootstrap from runtime migrations**

In `bitloops/src/host/runtime_store/sqlite_migrate.rs`, replace the `SqliteInteractionSpool` import and construction:

```rust
use crate::host::interactions::db_store::initialise_interaction_spool_schema;
use crate::storage::SqliteConnectionPool;
```

The runtime schema function should call schema initialization directly:

```rust
pub(crate) fn initialise_repo_runtime_schema(sqlite: &SqliteConnectionPool) -> Result<()> {
    sqlite
        .execute_batch(crate::host::devql::checkpoint_runtime_schema_sql_sqlite())
        .context("initialising runtime checkpoint schema")?;
    initialise_interaction_spool_schema(sqlite)
        .context("initialising interaction spool schema in runtime db")?;
    sqlite
        .execute_batch(crate::host::devql::producer_spool_schema_sql_sqlite())
        .context("initialising DevQL producer spool schema in runtime db")?;
    sqlite
        .execute_batch(super::repo_workplane::REPO_WORKPLANE_SCHEMA)
        .context("initialising capability workplane schema in runtime db")?;
    super::repo_workplane::ensure_repo_workplane_schema_upgrades(sqlite)
        .context("upgrading capability workplane schema in runtime db")?;
    Ok(())
}
```

- [x] **Step 5: Add explicit rebuild coverage**

Extend the new test in `bitloops/src/host/interactions/db_store/tests.rs` after the `docs_after_open` assertion:

```rust
    _spool.rebuild_search_projections()?;
    let docs_after_rebuild: i64 = sqlite.with_connection(|conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM interaction_session_search_documents WHERE repo_id = 'repo-test'",
            [],
            |row| row.get(0),
        )
        .map_err(anyhow::Error::from)
    })?;
    assert_eq!(
        docs_after_rebuild, 1,
        "explicit projection rebuild should still repair search documents"
    );
```

- [x] **Step 6: Add runtime bootstrap regression test**

Add this test to `bitloops/src/host/runtime_store/tests.rs`:

```rust
#[test]
fn runtime_schema_initialisation_does_not_rebuild_interaction_projections() {
    let dir = TempDir::new().expect("tempdir");
    let sqlite_path = dir.path().join("runtime.sqlite");
    let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path)
        .expect("connect sqlite");
    super::sqlite_migrate::initialise_repo_runtime_schema(&sqlite)
        .expect("initialise runtime schema");

    sqlite
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO interaction_sessions (session_id, repo_id, first_prompt, started_at, updated_at)
                 VALUES ('session-1', '__runtime-bootstrap__', 'bootstrap prompt', '2026-05-13T00:00:00Z', '2026-05-13T00:00:00Z')",
                [],
            )?;
            Ok::<_, anyhow::Error>(())
        })
        .expect("seed bootstrap interaction row");

    super::sqlite_migrate::initialise_repo_runtime_schema(&sqlite)
        .expect("reinitialise runtime schema");

    let docs: i64 = sqlite
        .with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM interaction_session_search_documents WHERE repo_id = '__runtime-bootstrap__'",
                [],
                |row| row.get(0),
            )
            .map_err(anyhow::Error::from)
        })
        .expect("count search docs");
    assert_eq!(
        docs, 0,
        "runtime schema bootstrap must not rebuild interaction search projections"
    );
}
```

- [x] **Step 7: Run focused tests for this task**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features --lib opening_spool_does_not_rebuild_search_projections runtime_schema_initialisation_does_not_rebuild_interaction_projections
```

Expected: PASS for both tests.

- [x] **Step 8: Commit this task**

```bash
git add bitloops/src/host/interactions/db_store.rs bitloops/src/host/runtime_store/sqlite_migrate.rs bitloops/src/host/interactions/db_store/tests.rs bitloops/src/host/runtime_store/tests.rs
git commit -m "fix: keep runtime interaction spool bootstrap schema-only"
```

---

### Task 2: Add Read-Only SQLite And Workplane Status Reader

**Files:**
- Modify: `bitloops/src/storage/sqlite.rs`
- Modify: `bitloops/src/storage/sqlite/filesystem.rs`
- Modify: `bitloops/src/storage.rs`
- Modify: `bitloops/src/storage/sqlite/tests.rs`
- Modify: `bitloops/src/host/runtime_store/repo_workplane/jobs.rs`
- Create: `bitloops/src/host/runtime_store/repo_workplane/read_only_status.rs`
- Modify: `bitloops/src/host/runtime_store/repo_workplane.rs`
- Modify: `bitloops/src/host/runtime_store.rs`
- Test: `bitloops/src/host/runtime_store/tests.rs`

- [x] **Step 1: Add read-only SQLite pool tests**

Add these tests to `bitloops/src/storage/sqlite/tests.rs`:

```rust
use super::{ReadOnlySqliteConnectionPool, SqliteConnectionPool};

#[test]
fn read_only_sqlite_pool_reads_existing_database_without_writes() -> Result<()> {
    let temp = TempDir::new().context("creating temp dir")?;
    let sqlite_path = temp.path().join("runtime.sqlite");
    let writable = SqliteConnectionPool::connect(sqlite_path.clone())?;
    writable.execute_batch(
        "CREATE TABLE sample (id INTEGER PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO sample (value) VALUES ('stored');",
    )?;

    let readonly = ReadOnlySqliteConnectionPool::connect_existing(sqlite_path)?;
    let value: String = readonly.with_connection(|conn| {
        conn.query_row("SELECT value FROM sample WHERE id = 1", [], |row| row.get(0))
            .map_err(anyhow::Error::from)
    })?;
    assert_eq!(value, "stored");

    let write_err = readonly
        .with_connection(|conn| {
            conn.execute("INSERT INTO sample (value) VALUES ('blocked')", [])?;
            Ok(())
        })
        .expect_err("read-only pool should reject writes");
    let message = format!("{write_err:#}");
    assert!(
        message.contains("readonly") || message.contains("read-only"),
        "expected read-only error, got {message}"
    );

    Ok(())
}

#[test]
fn read_only_sqlite_pool_refuses_missing_database() -> Result<()> {
    let temp = TempDir::new().context("creating temp dir")?;
    let err = ReadOnlySqliteConnectionPool::connect_existing(temp.path().join("missing.sqlite"))
        .expect_err("missing DB should fail");
    assert!(
        err.to_string().contains("SQLite database file not found"),
        "unexpected error: {err:#}"
    );
    Ok(())
}
```

- [x] **Step 2: Run the focused failing storage tests**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features --lib read_only_sqlite_pool_reads_existing_database_without_writes read_only_sqlite_pool_refuses_missing_database
```

Expected: FAIL because `ReadOnlySqliteConnectionPool` does not exist.

- [x] **Step 3: Add the read-only pool type**

In `bitloops/src/storage/sqlite.rs`, add the new type next to `SqliteConnectionPool`:

```rust
#[derive(Debug, Clone)]
pub struct ReadOnlySqliteConnectionPool {
    db_path: PathBuf,
}

impl ReadOnlySqliteConnectionPool {
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}
```

- [x] **Step 4: Implement read-only SQLite opening**

In `bitloops/src/storage/sqlite/filesystem.rs`, update the import:

```rust
use super::{ReadOnlySqliteConnectionPool, SqliteConnectionPool};
```

Add this implementation below the existing `impl SqliteConnectionPool`:

```rust
impl ReadOnlySqliteConnectionPool {
    pub fn connect_existing(db_path: std::path::PathBuf) -> Result<Self> {
        ensure_sqlite_file_exists(&db_path)?;
        let pool = Self { db_path };
        pool.with_connection(|_| Ok(()))?;
        Ok(pool)
    }

    pub fn with_connection<T>(
        &self,
        operation: impl FnOnce(&rusqlite::Connection) -> Result<T>,
    ) -> Result<T> {
        let conn = open_read_only_sqlite_connection(&self.db_path)?;
        operation(&conn)
    }
}
```

Add the read-only connection helpers:

```rust
pub(super) fn open_read_only_sqlite_connection(db_path: &Path) -> Result<rusqlite::Connection> {
    ensure_sqlite_file_exists(db_path)?;
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .with_context(|| format!("opening SQLite database read-only at {}", db_path.display()))?;
    configure_read_only_sqlite_connection(&conn)?;
    Ok(conn)
}

fn configure_read_only_sqlite_connection(conn: &rusqlite::Connection) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(30))
        .context("setting SQLite busy timeout")?;
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA query_only = ON;")
        .context("configuring read-only SQLite pragmas")?;
    Ok(())
}
```

Do not call `create_sqlite_file_if_missing` and do not set `PRAGMA journal_mode = WAL` in the read-only path.

- [x] **Step 5: Export the read-only pool**

In `bitloops/src/storage.rs`, change the SQLite export to:

```rust
pub use sqlite::{ReadOnlySqliteConnectionPool, SqliteConnectionPool};
```

- [x] **Step 6: Factor workplane list query into a connection helper**

In `bitloops/src/host/runtime_store/repo_workplane/jobs.rs`, move the SQL body of `RepoSqliteRuntimeStore::list_capability_workplane_jobs` into a helper:

```rust
pub(super) fn list_capability_workplane_jobs_on_connection(
    conn: &rusqlite::Connection,
    repo_id: &str,
    query: WorkplaneJobQuery,
) -> Result<Vec<WorkplaneJobRecord>> {
    let mut sql = String::from(
        "SELECT job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                init_session_id, dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                lease_expires_at_unix, last_error
         FROM capability_workplane_jobs
         WHERE repo_id = ?1",
    );
    let mut params = vec![SqlValue::Text(repo_id.to_string())];
    let mut bind_index = 2usize;

    if let Some(capability_id) = query.capability_id {
        sql.push_str(&format!(" AND capability_id = ?{bind_index}"));
        params.push(SqlValue::Text(capability_id));
        bind_index += 1;
    }
    if let Some(mailbox_name) = query.mailbox_name {
        sql.push_str(&format!(" AND mailbox_name = ?{bind_index}"));
        params.push(SqlValue::Text(mailbox_name));
        bind_index += 1;
    }
    if !query.statuses.is_empty() {
        let placeholders = std::iter::repeat_n("?", query.statuses.len())
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" AND status IN ({placeholders})"));
        for status in &query.statuses {
            params.push(SqlValue::Text(status.as_str().to_string()));
            bind_index += 1;
        }
    }
    sql.push_str(" ORDER BY updated_at_unix DESC, submitted_at_unix DESC");
    if let Some(limit) = query.limit {
        sql.push_str(&format!(" LIMIT ?{bind_index}"));
        let limit_i64 =
            i64::try_from(limit).context("converting workplane job query limit to sqlite i64")?;
        params.push(SqlValue::Integer(limit_i64));
    }

    let mut stmt = conn
        .prepare(&sql)
        .context("preparing capability workplane jobs query")?;
    let rows = stmt
        .query_map(params_from_iter(params.iter()), |row| {
            let payload_raw = row.get::<_, String>(8)?;
            let payload = serde_json::from_str(&payload_raw).unwrap_or(serde_json::Value::Null);
            Ok(WorkplaneJobRecord {
                job_id: row.get(0)?,
                repo_id: row.get(1)?,
                repo_root: std::path::PathBuf::from(row.get::<_, String>(2)?),
                config_root: std::path::PathBuf::from(row.get::<_, String>(3)?),
                capability_id: row.get(4)?,
                mailbox_name: row.get(5)?,
                init_session_id: row.get(6)?,
                dedupe_key: row.get(7)?,
                payload,
                status: WorkplaneJobStatus::parse(&row.get::<_, String>(9)?),
                attempts: row.get(10)?,
                available_at_unix: u64::try_from(row.get::<_, i64>(11)?).unwrap_or_default(),
                submitted_at_unix: u64::try_from(row.get::<_, i64>(12)?).unwrap_or_default(),
                started_at_unix: row
                    .get::<_, Option<i64>>(13)?
                    .map(|value| u64::try_from(value).unwrap_or_default()),
                updated_at_unix: u64::try_from(row.get::<_, i64>(14)?).unwrap_or_default(),
                completed_at_unix: row
                    .get::<_, Option<i64>>(15)?
                    .map(|value| u64::try_from(value).unwrap_or_default()),
                lease_owner: row.get(16)?,
                lease_expires_at_unix: row
                    .get::<_, Option<i64>>(17)?
                    .map(|value| u64::try_from(value).unwrap_or_default()),
                last_error: row.get(18)?,
            })
        })
        .context("querying capability workplane jobs")?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}
```

Then keep the read-write method as a thin wrapper:

```rust
pub fn list_capability_workplane_jobs(
    &self,
    query: WorkplaneJobQuery,
) -> Result<Vec<WorkplaneJobRecord>> {
    let sqlite = self.connect_repo_sqlite()?;
    sqlite.with_connection(|conn| {
        list_capability_workplane_jobs_on_connection(conn, &self.repo_id, query)
    })
}
```

- [x] **Step 7: Create the read-only workplane status reader**

Create `bitloops/src/host/runtime_store/repo_workplane/read_only_status.rs`:

```rust
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::jobs::list_capability_workplane_jobs_on_connection;
use super::types::{WorkplaneJobQuery, WorkplaneJobRecord};
use crate::config::{
    resolve_bound_daemon_config_root_for_repo, resolve_repo_runtime_db_path_for_config_root,
};
use crate::storage::ReadOnlySqliteConnectionPool;

#[derive(Debug, Clone)]
pub struct RepoCapabilityWorkplaneStatusReader {
    repo_id: String,
    db_path: PathBuf,
    sqlite: ReadOnlySqliteConnectionPool,
}

impl RepoCapabilityWorkplaneStatusReader {
    pub fn open(repo_root: &Path, repo_id: &str) -> Result<Option<Self>> {
        let daemon_config_root = resolve_bound_daemon_config_root_for_repo(repo_root)
            .context("resolving daemon config root for read-only workplane status")?;
        Self::open_for_config_root(&daemon_config_root, repo_id)
    }

    pub fn open_for_config_root(config_root: &Path, repo_id: &str) -> Result<Option<Self>> {
        let db_path = resolve_repo_runtime_db_path_for_config_root(config_root);
        if !db_path.is_file() {
            return Ok(None);
        }
        let sqlite = ReadOnlySqliteConnectionPool::connect_existing(db_path.clone())
            .with_context(|| format!("opening repo runtime database read-only {}", db_path.display()))?;
        Ok(Some(Self {
            repo_id: repo_id.to_string(),
            db_path,
            sqlite,
        }))
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn list_capability_workplane_jobs(
        &self,
        query: WorkplaneJobQuery,
    ) -> Result<Vec<WorkplaneJobRecord>> {
        self.sqlite.with_connection(|conn| {
            if !table_exists(conn, "capability_workplane_jobs")? {
                return Ok(Vec::new());
            }
            list_capability_workplane_jobs_on_connection(conn, &self.repo_id, query)
        })
    }
}

fn table_exists(conn: &rusqlite::Connection, table_name: &str) -> Result<bool> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table_name],
            |row| row.get(0),
        )
        .context("checking runtime status table existence")?;
    Ok(count > 0)
}
```

- [x] **Step 8: Export the reader**

In `bitloops/src/host/runtime_store/repo_workplane.rs`, add the module and export:

```rust
mod read_only_status;
```

```rust
pub use read_only_status::RepoCapabilityWorkplaneStatusReader;
```

In `bitloops/src/host/runtime_store.rs`, add it to the `repo_workplane` export list:

```rust
RepoCapabilityWorkplaneStatusReader,
```

- [x] **Step 9: Add read-only reader regression tests**

Add this test to `bitloops/src/host/runtime_store/tests.rs` near existing workplane job tests:

```rust
#[test]
fn read_only_workplane_status_reader_lists_existing_jobs_without_runtime_open() {
    let dir = TempDir::new().expect("tempdir");
    let repo_root = dir.path().join("repo");
    fs::create_dir_all(&repo_root).expect("create repo root");
    init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops@example.com");

    let store = RepoSqliteRuntimeStore::open_for_roots(dir.path(), &repo_root)
        .expect("open repo runtime store");
    store
        .enqueue_capability_workplane_jobs(
            "architecture_graph",
            vec![CapabilityWorkplaneJobInsert::new(
                "architecture_graph.roles.adjudication",
                None,
                Some("queue-1".to_string()),
                serde_json::json!({"request": {"reason": "unknown_kind", "generation": 7}}),
            )],
        )
        .expect("enqueue workplane job");

    let reader = RepoCapabilityWorkplaneStatusReader::open_for_config_root(
        dir.path(),
        store.repo_id(),
    )
    .expect("open read-only status reader")
    .expect("runtime db should exist");
    let rows = reader
        .list_capability_workplane_jobs(WorkplaneJobQuery {
            capability_id: Some("architecture_graph".to_string()),
            mailbox_name: Some("architecture_graph.roles.adjudication".to_string()),
            statuses: vec![WorkplaneJobStatus::Pending],
            limit: Some(10),
        })
        .expect("list jobs read-only");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].dedupe_key.as_deref(), Some("queue-1"));
}

#[test]
fn read_only_workplane_status_reader_returns_none_for_missing_runtime_db() {
    let dir = TempDir::new().expect("tempdir");
    let reader = RepoCapabilityWorkplaneStatusReader::open_for_config_root(dir.path(), "repo-1")
        .expect("open status reader");
    assert!(reader.is_none());
}
```

- [x] **Step 10: Run focused tests for this task**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features --lib read_only_sqlite_pool_reads_existing_database_without_writes read_only_sqlite_pool_refuses_missing_database read_only_workplane_status_reader_lists_existing_jobs_without_runtime_open read_only_workplane_status_reader_returns_none_for_missing_runtime_db
```

Expected: PASS for all four tests.

- [x] **Step 11: Commit this task**

```bash
git add bitloops/src/storage.rs bitloops/src/storage/sqlite.rs bitloops/src/storage/sqlite/filesystem.rs bitloops/src/storage/sqlite/tests.rs bitloops/src/host/runtime_store.rs bitloops/src/host/runtime_store/repo_workplane.rs bitloops/src/host/runtime_store/repo_workplane/jobs.rs bitloops/src/host/runtime_store/repo_workplane/read_only_status.rs bitloops/src/host/runtime_store/tests.rs
git commit -m "feat: add read-only runtime workplane status reader"
```

---

### Task 3: Route `roles status` Without Current-State Context

**Files:**
- Modify: `bitloops/src/host/capability_host/host.rs`
- Modify: `bitloops/src/cli/devql/architecture.rs`
- Modify: `bitloops/src/cli/devql/architecture/deterministic_tests.rs`

- [x] **Step 1: Add routing predicate test**

Add this test to `bitloops/src/cli/devql/architecture/deterministic_tests.rs`:

```rust
#[test]
fn architecture_roles_status_does_not_require_current_state_context() {
    let args = DevqlArchitectureRolesArgs {
        command: DevqlArchitectureRolesCommand::Status(DevqlArchitectureRolesStatusArgs {
            json: true,
            limit: 10,
        }),
    };

    assert!(
        !architecture_roles_command_requires_current_state_context(&args),
        "roles status must be routed before current-state consumer context construction"
    );
}
```

- [x] **Step 2: Run the focused failing predicate test**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features --lib architecture_roles_status_does_not_require_current_state_context
```

Expected: FAIL because the predicate does not exist.

- [x] **Step 3: Add relational-only storage accessor**

In `bitloops/src/host/capability_host/host.rs`, add:

```rust
pub fn build_relational_storage(&self) -> Result<crate::host::devql::RelationalStorage> {
    let relational_store = DefaultRelationalStore::open_local_for_backend_config(
        self.repo_root(),
        &self.runtime.backends.relational,
    )?;
    Ok(relational_store.to_local_inner())
}
```

This intentionally mirrors the storage portion of `build_current_state_consumer_context` without constructing language services, inference, host services, test harness, or a workplane gateway.

- [x] **Step 4: Add context-routing helpers**

In `bitloops/src/cli/devql/architecture.rs`, add:

```rust
fn architecture_roles_command_requires_current_state_context(
    args: &DevqlArchitectureRolesArgs,
) -> bool {
    !matches!(&args.command, DevqlArchitectureRolesCommand::Status(_))
}
```

Change `run_architecture_command` so it only builds current-state context for commands that need it:

```rust
pub(super) async fn run_architecture_command(
    scope: &SlimCliRepoScope,
    args: DevqlArchitectureArgs,
) -> Result<()> {
    let host = DevqlCapabilityHost::builtin(scope.repo_root.clone(), scope.repo.clone())?;
    host.ensure_migrations_applied_sync()?;

    match args.command {
        DevqlArchitectureCommand::Roles(args) => {
            if !architecture_roles_command_requires_current_state_context(&args) {
                return run_architecture_roles_command_without_current_state(scope, &host, args)
                    .await;
            }
            let context = host.build_current_state_consumer_context("architecture_graph")?;
            run_architecture_roles_command(scope, &host, &context, args).await
        }
    }
}
```

Add the status-only command router:

```rust
async fn run_architecture_roles_command_without_current_state(
    scope: &SlimCliRepoScope,
    host: &DevqlCapabilityHost,
    args: DevqlArchitectureRolesArgs,
) -> Result<()> {
    match args.command {
        DevqlArchitectureRolesCommand::Status(args) => {
            let relational = host.build_relational_storage()?;
            run_architecture_roles_status(scope, &relational, args).await
        }
        _ => {
            let context = host.build_current_state_consumer_context("architecture_graph")?;
            run_architecture_roles_command(scope, host, &context, args).await
        }
    }
}
```

The fallback arm is defensive; the predicate should keep all non-status commands on the context path.

- [x] **Step 5: Change status to accept relational storage directly**

Change the signature in `bitloops/src/cli/devql/architecture.rs`:

```rust
async fn run_architecture_roles_status(
    scope: &SlimCliRepoScope,
    relational: &crate::host::devql::RelationalStorage,
    args: DevqlArchitectureRolesStatusArgs,
) -> Result<()> {
    let limit = usize::try_from(args.limit).context("converting --limit to usize")?;
    let queue_items = load_role_adjudication_queue_items(scope, limit)?;
    let review_items = load_role_review_items(relational, &scope.repo.repo_id, limit).await?;
    let adjudication_attempts =
        load_role_adjudication_attempt_items(relational, &scope.repo.repo_id, limit).await?;
    let summary = summarise_queue_items(&queue_items);
    let adjudication_attempt_summary = summarise_adjudication_attempts(&adjudication_attempts);
    let output = RolesStatusOutput {
        queue_summary: summary,
        queue_items,
        review_items,
        adjudication_attempt_summary,
        adjudication_attempts,
    };

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&output)
                .context("serialising roles status output as JSON")?
        );
        return Ok(());
    }

    print_roles_status_human(&output);
    Ok(())
}
```

Change the status arm inside `run_architecture_roles_command` for any direct internal caller that already has a context:

```rust
DevqlArchitectureRolesCommand::Status(args) => {
    run_architecture_roles_status(scope, context.storage.as_ref(), args).await
}
```

- [x] **Step 6: Load queue items through the read-only status reader**

In `bitloops/src/cli/devql/architecture.rs`, update imports:

```rust
use crate::host::runtime_store::{
    RepoCapabilityWorkplaneStatusReader, WorkplaneJobQuery, WorkplaneJobStatus,
};
```

Replace `load_role_adjudication_queue_items` with:

```rust
fn load_role_adjudication_queue_items(
    scope: &SlimCliRepoScope,
    limit: usize,
) -> Result<Vec<RoleAdjudicationQueueItem>> {
    let Some(reader) = RepoCapabilityWorkplaneStatusReader::open(
        &scope.repo_root,
        &scope.repo.repo_id,
    )
    .context("opening read-only repo runtime status reader for architecture roles status")?
    else {
        return Ok(Vec::new());
    };

    let jobs = reader
        .list_capability_workplane_jobs(WorkplaneJobQuery {
            capability_id: Some(ARCHITECTURE_GRAPH_CAPABILITY_ID.to_string()),
            mailbox_name: Some(ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX.to_string()),
            statuses: vec![
                WorkplaneJobStatus::Pending,
                WorkplaneJobStatus::Running,
                WorkplaneJobStatus::Failed,
            ],
            limit: Some(limit as u64),
        })
        .context("loading architecture role adjudication queue jobs read-only")?;

    Ok(jobs
        .into_iter()
        .map(role_adjudication_queue_item_from_job)
        .collect())
}
```

Remove the `RepoSqliteRuntimeStore` import from `architecture.rs`.

- [x] **Step 7: Add status queue reader unit coverage**

Add this test to `bitloops/src/cli/devql/architecture/deterministic_tests.rs`:

```rust
#[test]
fn role_adjudication_queue_item_maps_failed_job_payload_errors() {
    let job = WorkplaneJobRecord {
        job_id: "workplane-job-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: std::path::PathBuf::from("/tmp/repo"),
        config_root: std::path::PathBuf::from("/tmp/config"),
        capability_id: ARCHITECTURE_GRAPH_CAPABILITY_ID.to_string(),
        mailbox_name: ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX.to_string(),
        init_session_id: None,
        dedupe_key: Some("dedupe-1".to_string()),
        payload: serde_json::Value::Null,
        status: WorkplaneJobStatus::Failed,
        attempts: 2,
        available_at_unix: 1,
        submitted_at_unix: 1,
        started_at_unix: Some(2),
        updated_at_unix: 3,
        completed_at_unix: None,
        lease_owner: None,
        lease_expires_at_unix: None,
        last_error: Some("database is locked".to_string()),
    };

    let item = role_adjudication_queue_item_from_job(job);

    assert_eq!(item.status, "failed");
    assert_eq!(item.attempts, 2);
    assert_eq!(item.parse_error.as_deref(), Some("invalid type: null, expected struct RoleAdjudicationMailboxPayload"));
    assert_eq!(item.last_error.as_deref(), Some("database is locked"));
}
```

If the exact serde error string differs on this Rust/serde version, assert only that `item.parse_error.as_deref().unwrap_or_default().contains("expected struct RoleAdjudicationMailboxPayload")`.

- [x] **Step 8: Run focused status tests**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features --lib architecture_roles_status_does_not_require_current_state_context roles_status_reads_review_items_from_current_assignments roles_status_reads_recent_adjudication_attempts role_adjudication_queue_item_maps_failed_job_payload_errors
```

Expected: PASS.

- [x] **Step 9: Commit this task**

```bash
git add bitloops/src/host/capability_host/host.rs bitloops/src/cli/devql/architecture.rs bitloops/src/cli/devql/architecture/deterministic_tests.rs
git commit -m "fix: make architecture roles status read-only"
```

---

### Task 4: Suppress Bitloops Agent Hooks For Internal CLI-Agent Inference

**Files:**
- Modify: `bitloops/src/host/hooks.rs`
- Modify: `bitloops/src/host/hooks/dispatcher.rs`
- Modify: `bitloops/src/host/inference/text_generation.rs`
- Modify: `bitloops/src/host/inference/text_generation/tests.rs`

- [x] **Step 1: Add inference env propagation test**

Add this test to `bitloops/src/host/inference/text_generation/tests.rs` near existing `codex_exec` tests:

```rust
#[test]
fn codex_exec_runtime_launch_sets_agent_hook_suppression_env() {
    let _guard = test_lock();
    let repo = tempfile::TempDir::new().expect("tempdir");
    let script_path = repo.path().join("fake-codex-runtime.sh");
    let env_log = repo.path().join("env.log");
    fs::write(
        &script_path,
        format!(
            r#"printf '%s\n' "${{{env_name}:-}}" > {env_log:?}
while IFS= read -r line; do
  request_id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"describe"'*)
      printf '{{"type":"describe","request_id":"%s","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"architecture_fact_synthesis_codex","provider":{{"kind":"codex_exec","provider_name":"codex","model_name":"gpt-5.4-mini","endpoint":"codex","capabilities":{{"response_modes":["json_object"],"usage_reporting":false,"structured_output":["json_object","json_schema"]}}}}}}\n' "$request_id"
      ;;
    *'"type":"shutdown"'*)
      printf '{{"type":"shutdown","request_id":"%s"}}\n' "$request_id"
      exit 0
      ;;
  esac
done
"#,
            env_name = crate::host::hooks::BITLOOPS_SUPPRESS_AGENT_HOOKS_ENV,
            env_log = env_log.display(),
        ),
    )
    .expect("write fake runtime");

    let runtime = InferenceRuntimeConfig {
        command: "/bin/sh".to_string(),
        args: vec![script_path.to_string_lossy().into_owned()],
        startup_timeout_secs: 5,
        request_timeout_secs: 5,
    };
    let service = BitloopsInferenceTextGenerationService::new(
        "architecture_fact_synthesis_codex",
        CODEX_EXEC_DRIVER,
        &runtime,
        &repo.path().join(".bitloops/config.toml"),
        default_request_defaults(),
    )
    .expect("build codex_exec service");

    assert_eq!(service.descriptor(), "codex:gpt-5.4-mini");
    assert_eq!(
        fs::read_to_string(env_log).expect("read env log").trim(),
        "1",
        "internal codex_exec launcher must suppress Bitloops agent hooks"
    );
}
```

- [x] **Step 2: Add non-agent runtime guard test**

Add this test to `bitloops/src/host/inference/text_generation/tests.rs`:

```rust
#[test]
fn non_agent_runtime_launch_does_not_set_agent_hook_suppression_env() {
    let _guard = test_lock();
    let repo = tempfile::TempDir::new().expect("tempdir");
    let script_path = repo.path().join("fake-ollama-runtime.sh");
    let env_log = repo.path().join("env.log");
    fs::write(
        &script_path,
        format!(
            r#"printf '%s\n' "${{{env_name}:-}}" > {env_log:?}
while IFS= read -r line; do
  request_id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"describe"'*)
      printf '{{"type":"describe","request_id":"%s","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"summary_local","provider":{{"kind":"ollama_chat","provider_name":"ollama","model_name":"ministral-3:3b","endpoint":"http://127.0.0.1:11434","capabilities":{{"response_modes":["text"],"usage_reporting":true,"structured_output":[]}}}}}}\n' "$request_id"
      ;;
    *'"type":"shutdown"'*)
      printf '{{"type":"shutdown","request_id":"%s"}}\n' "$request_id"
      exit 0
      ;;
  esac
done
"#,
            env_name = crate::host::hooks::BITLOOPS_SUPPRESS_AGENT_HOOKS_ENV,
            env_log = env_log.display(),
        ),
    )
    .expect("write fake runtime");

    let runtime = InferenceRuntimeConfig {
        command: "/bin/sh".to_string(),
        args: vec![script_path.to_string_lossy().into_owned()],
        startup_timeout_secs: 5,
        request_timeout_secs: 5,
    };
    let service = BitloopsInferenceTextGenerationService::new(
        "summary_local",
        "ollama_chat",
        &runtime,
        &repo.path().join(".bitloops/config.toml"),
        default_request_defaults(),
    )
    .expect("build non-agent service");

    assert_eq!(service.descriptor(), "ollama:ministral-3:3b");
    assert_eq!(
        fs::read_to_string(env_log).expect("read env log").trim(),
        "",
        "non-agent inference runtimes should not receive hook suppression"
    );
}
```

- [x] **Step 3: Add hook env contract**

In `bitloops/src/host/hooks.rs`, add:

```rust
pub const BITLOOPS_SUPPRESS_AGENT_HOOKS_ENV: &str = "BITLOOPS_SUPPRESS_AGENT_HOOKS";

pub(crate) fn agent_hooks_suppressed_by_env() -> bool {
    std::env::var(BITLOOPS_SUPPRESS_AGENT_HOOKS_ENV)
        .map(|value| {
            let trimmed = value.trim();
            !trimmed.is_empty()
                && !matches!(
                    trimmed.to_ascii_lowercase().as_str(),
                    "0" | "false" | "no" | "off"
                )
        })
        .unwrap_or(false)
}
```

- [x] **Step 4: Add hook env helper tests**

Add these tests to `bitloops/src/host/hooks/dispatcher.rs` under the existing `#[cfg(test)]` test area or in a new `#[cfg(test)] mod tests` at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::hooks::{
        BITLOOPS_SUPPRESS_AGENT_HOOKS_ENV, agent_hooks_suppressed_by_env,
    };
    use crate::test_support::process_state::enter_process_state;

    #[test]
    fn agent_hooks_suppressed_env_accepts_truthy_values() {
        let _guard = enter_process_state(None, &[(BITLOOPS_SUPPRESS_AGENT_HOOKS_ENV, Some("1"))]);
        assert!(agent_hooks_suppressed_by_env());
    }

    #[test]
    fn agent_hooks_suppressed_env_rejects_false_values() {
        for value in ["", "0", "false", "no", "off"] {
            let _guard =
                enter_process_state(None, &[(BITLOOPS_SUPPRESS_AGENT_HOOKS_ENV, Some(value))]);
            assert!(
                !agent_hooks_suppressed_by_env(),
                "value `{value}` should not suppress hooks"
            );
        }
    }
}
```

If `dispatcher.rs` already has a test module by the time this task runs, add these two tests to that module instead of creating a second one.

- [x] **Step 5: Honor suppression in agent hook dispatch**

In `bitloops/src/host/hooks/dispatcher.rs`, add the early return after the git-hook split and before repo root resolution:

```rust
pub async fn run(args: HooksArgs, strategy_registry: &StrategyRegistry) -> Result<()> {
    let agent = match args.agent {
        HooksAgent::Git(git_args) => return git::run(git_args, strategy_registry).await,
        other => other,
    };

    if crate::host::hooks::agent_hooks_suppressed_by_env() {
        return Ok(());
    }

    let repo_root = match paths::repo_root() {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };
```

This ensures suppressed internal inference hooks do not resolve repo state, initialize logging, touch runtime DBs, read settings, or emit telemetry.

- [x] **Step 6: Set the env only for CLI-agent structured-generation runtimes**

In `bitloops/src/host/inference/text_generation.rs`, update the imports:

```rust
use super::{
    BITLOOPS_PLATFORM_CHAT_DRIVER, CLAUDE_CODE_PRINT_DRIVER, CODEX_EXEC_DRIVER,
    DEFAULT_REMOTE_TEXT_GENERATION_CONCURRENCY, OPENAI_CHAT_COMPLETIONS_DRIVER,
    StructuredGenerationRequest, StructuredGenerationService, TextGenerationOptions,
    TextGenerationService,
};
```

Add this helper near the other local helpers:

```rust
fn should_suppress_agent_hooks_for_driver(driver: &str) -> bool {
    matches!(driver.trim(), CODEX_EXEC_DRIVER | CLAUDE_CODE_PRINT_DRIVER)
}
```

In `BitloopsInferenceSession::start`, after the platform-auth env block and before stdio setup, add:

```rust
if should_suppress_agent_hooks_for_driver(&config.driver) {
    command.env(crate::host::hooks::BITLOOPS_SUPPRESS_AGENT_HOOKS_ENV, "1");
}
```

The env is placed on `bitloops-inference`; provider child processes inherit it, including Codex.

- [x] **Step 7: Run focused hook and inference tests**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features --lib codex_exec_runtime_launch_sets_agent_hook_suppression_env non_agent_runtime_launch_does_not_set_agent_hook_suppression_env agent_hooks_suppressed_env_accepts_truthy_values agent_hooks_suppressed_env_rejects_false_values codex_exec_runtime_does_not_request_platform_auth_environment
```

Expected: PASS.

- [x] **Step 8: Commit this task**

```bash
git add bitloops/src/host/hooks.rs bitloops/src/host/hooks/dispatcher.rs bitloops/src/host/inference/text_generation.rs bitloops/src/host/inference/text_generation/tests.rs
git commit -m "fix: suppress agent hooks for internal CLI inference"
```

---

### Task 5: End-To-End Regression Verification

**Files:**
- No new source files.
- Uses the files changed in Tasks 1 through 4.

- [ ] **Step 1: Run the focused library regression set**

Run:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --no-default-features --lib opening_spool_does_not_rebuild_search_projections runtime_schema_initialisation_does_not_rebuild_interaction_projections read_only_sqlite_pool_reads_existing_database_without_writes read_only_sqlite_pool_refuses_missing_database read_only_workplane_status_reader_lists_existing_jobs_without_runtime_open read_only_workplane_status_reader_returns_none_for_missing_runtime_db architecture_roles_status_does_not_require_current_state_context roles_status_reads_review_items_from_current_assignments roles_status_reads_recent_adjudication_attempts role_adjudication_queue_item_maps_failed_job_payload_errors codex_exec_runtime_launch_sets_agent_hook_suppression_env non_agent_runtime_launch_does_not_set_agent_hook_suppression_env agent_hooks_suppressed_env_accepts_truthy_values agent_hooks_suppressed_env_rejects_false_values codex_exec_runtime_does_not_request_platform_auth_environment
```

Expected: PASS.

- [ ] **Step 2: Run the relevant repo alias lane**

Run:

```bash
cargo dev-test-lib
```

Expected: PASS. This is the broad relevant lane because all changes are library-level behavior under CLI, runtime store, hooks, and inference modules. Do not run all tests unless explicitly requested.

- [ ] **Step 3: Run formatting and compile checks**

Run:

```bash
cargo dev-fmt-check
cargo dev-check
```

Expected: PASS for both commands.

- [ ] **Step 4: Optional manual reproduction check in a fixture repo**

Use a disposable or existing architecture roles fixture where daemon and local `codex_exec` are configured. Run the same shape as the original flow:

```bash
bitloops daemon restart
bitloops devql architecture roles seed
bitloops devql architecture roles classify --full
bitloops devql architecture roles status --json --limit 10
```

Expected:

- `roles status` returns JSON instead of `initialising interaction spool schema in runtime db: rebuilding interaction search projections: database is locked`.
- During the local `codex_exec` inference window, `ps` or daemon logs should not show nested `bitloops hooks codex session-start` or `bitloops hooks codex user-prompt-submit` processes spawned by daemon-owned inference.
- If role adjudication still reports `llm_error` about an invalid strict OpenAI schema, treat that as the separate `llm_executor.rs` schema issue called out in the File Map.

- [ ] **Step 5: Commit verification-only adjustments if any**

If the verification steps require small test-name or import fixes, commit them with the relevant prior task's commit message style. If no code changed during verification, do not create an empty commit.

---

## Self-Review Checklist

- Runtime spool open no longer rebuilds projections.
- Runtime schema bootstrap no longer constructs `SqliteInteractionSpool` for `__runtime-bootstrap__`.
- Explicit projection rebuild remains available and tested.
- `roles status` no longer builds a current-state consumer context.
- `roles status` queue reading no longer opens `RepoSqliteRuntimeStore`.
- Missing runtime DB is an empty queue, not a created DB.
- Read-only SQLite path does not create files and does not set `journal_mode`.
- Internal CLI-agent inference sets `BITLOOPS_SUPPRESS_AGENT_HOOKS=1`.
- Agent hook dispatcher returns success before repo/runtime access when suppression is active.
- Git hooks remain unaffected by suppression.
- Tests use focused `cargo nextest` commands and the relevant repo alias lane, not the full suite.
