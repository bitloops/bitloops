use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::storage::sqlite::SqliteConnectionPool;

mod projections;
mod row_mapping;
mod schema;
mod spool;

#[cfg(test)]
mod tests;

const LEGACY_INTERACTION_SPOOL_FILE_NAME: &str = "interaction_spool.sqlite";

pub fn interaction_spool_db_path(repo_root: &Path) -> Result<PathBuf> {
    crate::config::resolve_bound_repo_runtime_db_path_for_repo(repo_root)
}

pub fn legacy_interaction_spool_db_path(repo_root: &Path) -> Result<PathBuf> {
    let backends = crate::config::resolve_bound_store_backend_config_for_repo(repo_root)
        .context("resolving backend config for interaction spool")?;
    let events_db_path = backends.events.resolve_duckdb_db_path_for_repo(repo_root);
    let parent = events_db_path.parent().with_context(|| {
        format!(
            "resolving interaction spool directory from event db path {}",
            events_db_path.display()
        )
    })?;
    Ok(parent.join(LEGACY_INTERACTION_SPOOL_FILE_NAME))
}

pub struct SqliteInteractionSpool {
    pub(super) sqlite: SqliteConnectionPool,
    pub(super) repo_id: String,
}

impl SqliteInteractionSpool {
    pub fn new(sqlite: SqliteConnectionPool, repo_id: String) -> Result<Self> {
        sqlite
            .execute_batch(schema::SCHEMA)
            .context("initialising interaction spool schema")?;
        sqlite
            .with_connection(schema::ensure_additive_columns)
            .context("ensuring additive interaction spool columns")?;
        sqlite
            .with_connection(|conn| projections::rebuild_all_projections(conn, &repo_id))
            .context("rebuilding interaction search projections")?;
        Ok(Self { sqlite, repo_id })
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

fn ensure_repo_id(expected: &str, actual: &str, entity: &str) -> Result<()> {
    if expected == actual {
        return Ok(());
    }
    anyhow::bail!("repo_id mismatch for {entity}: expected '{expected}', got '{actual}'");
}
