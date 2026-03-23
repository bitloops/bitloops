use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::config::{
    BlobStorageProvider, EventsProvider, RelationalProvider, resolve_blob_local_path_for_repo,
    resolve_duckdb_db_path_for_repo, resolve_sqlite_db_path_for_repo,
    resolve_store_backend_config_for_repo,
};
use crate::utils::paths;

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

pub(super) fn initialise_store_backends(repo_root: &Path) -> Result<()> {
    ensure_default_store_directories(repo_root)?;

    let cfg = resolve_store_backend_config_for_repo(repo_root)
        .context("resolving backend config for store initialisation")?;

    if cfg.relational.provider == RelationalProvider::Sqlite {
        let sqlite_path =
            resolve_sqlite_db_path_for_repo(repo_root, cfg.relational.sqlite_path.as_deref())
                .context("resolving SQLite path for `bitloops init`")?;
        let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path.clone())
            .with_context(|| format!("creating SQLite database at {}", sqlite_path.display()))?;
        sqlite
            .initialise_checkpoint_schema()
            .context("initialising SQLite checkpoint/session schema")?;
    }

    if cfg.events.provider == EventsProvider::DuckDb {
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

    if cfg.blobs.provider == BlobStorageProvider::Local {
        let blob_root =
            resolve_blob_local_path_for_repo(repo_root, cfg.blobs.local_path.as_deref())
                .context("resolving local blob store path for `bitloops init`")?;
        fs::create_dir_all(&blob_root)
            .with_context(|| format!("creating local blob store root {}", blob_root.display()))?;
    }

    Ok(())
}

fn ensure_default_store_directories(repo_root: &Path) -> Result<()> {
    for dir in [
        repo_root.join(paths::BITLOOPS_RELATIONAL_STORE_DIR),
        repo_root.join(paths::BITLOOPS_EVENT_STORE_DIR),
        repo_root.join(paths::BITLOOPS_BLOB_STORE_DIR),
    ] {
        fs::create_dir_all(&dir)
            .with_context(|| format!("creating store directory {}", dir.display()))?;
    }
    Ok(())
}
