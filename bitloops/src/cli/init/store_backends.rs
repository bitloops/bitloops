use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::config::{
    resolve_blob_local_path_for_repo, resolve_duckdb_db_path_for_repo,
    resolve_store_backend_config_for_repo,
};

const DUCKDB_CHECKPOINT_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS checkpoint_events (
    event_id VARCHAR PRIMARY KEY,
    event_time VARCHAR,
    repo_id VARCHAR,
    checkpoint_id VARCHAR,
    session_id VARCHAR,
    commit_sha VARCHAR,
    branch VARCHAR,
    event_type VARCHAR,
    agent VARCHAR,
    strategy VARCHAR,
    files_touched VARCHAR,
    payload VARCHAR
);

CREATE INDEX IF NOT EXISTS checkpoint_events_repo_time_idx
ON checkpoint_events (repo_id, event_time);

CREATE INDEX IF NOT EXISTS checkpoint_events_repo_commit_idx
ON checkpoint_events (repo_id, commit_sha);
"#;

pub(crate) fn initialise_store_backends(repo_root: &Path) -> Result<()> {
    let cfg = resolve_store_backend_config_for_repo(repo_root)
        .context("resolving backend config for store initialisation")?;

    crate::host::runtime_store::RepoSqliteRuntimeStore::open(repo_root)
        .context("initialising repo runtime store")?;

    if !cfg.relational.has_postgres() {
        let relational =
            crate::host::relational_store::DefaultRelationalStore::open_local_for_repo_root(
                repo_root,
            )
            .context("opening relational store for `bitloops init`")?;
        relational
            .initialise_local_relational_checkpoint_schema()
            .context("initialising SQLite relational checkpoint schema")?;
        let sqlite = crate::host::relational_store::RelationalStore::local_sqlite_pool(&relational)
            .context("opening SQLite database for `bitloops init`")?;
        sqlite
            .initialise_devql_schema()
            .context("initialising SQLite DevQL schema")?;
    }

    if !cfg.events.has_clickhouse() {
        let duckdb_path =
            resolve_duckdb_db_path_for_repo(repo_root, cfg.events.duckdb_path.as_deref());
        if let Some(parent) = duckdb_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating DuckDB directory {}", parent.display()))?;
        }

        let conn = duckdb::Connection::open(&duckdb_path)
            .with_context(|| format!("creating DuckDB database at {}", duckdb_path.display()))?;
        conn.execute_batch(DUCKDB_CHECKPOINT_SCHEMA_SQL)
            .context("initialising DuckDB checkpoint events schema")?;
    }

    if !cfg.blobs.has_remote() {
        let blob_root =
            resolve_blob_local_path_for_repo(repo_root, cfg.blobs.local_path.as_deref())
                .context("resolving local blob store path for `bitloops init`")?;
        fs::create_dir_all(&blob_root)
            .with_context(|| format!("creating local blob store root {}", blob_root.display()))?;
    }

    Ok(())
}
