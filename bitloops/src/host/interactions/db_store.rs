use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::host::interactions::store::InteractionSpool;
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventFilter, InteractionSession, InteractionTurn,
};
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

pub(crate) fn initialise_interaction_spool_schema(sqlite: &SqliteConnectionPool) -> Result<()> {
    sqlite
        .with_write_connection(schema::initialise_schema)
        .context("initialising interaction spool schema")
}

pub(crate) fn rebuild_interaction_search_projections(
    sqlite: &SqliteConnectionPool,
    repo_id: &str,
) -> Result<()> {
    sqlite
        .with_write_connection(|conn| projections::rebuild_all_projections(conn, repo_id))
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

    pub fn assign_checkpoint_to_turns(
        &self,
        turn_ids: &[String],
        checkpoint_id: &str,
        assigned_at: &str,
    ) -> Result<()> {
        <Self as InteractionSpool>::assign_checkpoint_to_turns(
            self,
            turn_ids,
            checkpoint_id,
            assigned_at,
        )
    }

    pub fn list_sessions(
        &self,
        agent: Option<&str>,
        limit: usize,
    ) -> Result<Vec<InteractionSession>> {
        <Self as InteractionSpool>::list_sessions(self, agent, limit)
    }

    pub fn load_session(&self, session_id: &str) -> Result<Option<InteractionSession>> {
        <Self as InteractionSpool>::load_session(self, session_id)
    }

    pub fn list_turns_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<InteractionTurn>> {
        <Self as InteractionSpool>::list_turns_for_session(self, session_id, limit)
    }

    pub fn list_uncheckpointed_turns(&self) -> Result<Vec<InteractionTurn>> {
        <Self as InteractionSpool>::list_uncheckpointed_turns(self)
    }

    pub fn list_events(
        &self,
        filter: &InteractionEventFilter,
        limit: usize,
    ) -> Result<Vec<InteractionEvent>> {
        <Self as InteractionSpool>::list_events(self, filter, limit)
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
