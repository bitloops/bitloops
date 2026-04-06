use std::path::Path;

use anyhow::{Context, Result};

use crate::config::resolve_repo_runtime_db_path_for_repo;
use crate::host::checkpoints::session::DbSessionBackend;
use crate::host::interactions::db_store::SqliteInteractionSpool;
use crate::storage::SqliteConnectionPool;

use super::sqlite_migrate::initialise_repo_runtime_schema;
use super::types::RepoSqliteRuntimeStore;

impl RepoSqliteRuntimeStore {
    pub fn open(repo_root: &Path) -> Result<Self> {
        let repo = crate::host::devql::resolve_repo_identity(repo_root)
            .context("resolving repo identity for runtime store")?;
        let db_path = resolve_repo_runtime_db_path_for_repo(repo_root)
            .context("resolving configured repo runtime database path")?;
        let sqlite = SqliteConnectionPool::connect(db_path.clone())
            .with_context(|| format!("opening repo runtime database {}", db_path.display()))?;
        initialise_repo_runtime_schema(&sqlite)?;
        let store = Self {
            repo_root: repo_root.to_path_buf(),
            repo_id: repo.repo_id,
            db_path,
        };
        store.import_legacy_repo_local_runtime_if_needed()?;
        store.import_legacy_checkpoint_runtime_if_needed()?;
        store.import_legacy_interaction_spool_if_needed()?;
        store.import_legacy_checkpoint_metadata_if_needed()?;
        Ok(store)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn repo_id(&self) -> &str {
        &self.repo_id
    }

    pub fn session_backend(&self) -> Result<DbSessionBackend> {
        DbSessionBackend::from_sqlite_path(self.repo_id.clone(), self.db_path.clone())
    }

    pub fn interaction_spool(&self) -> Result<SqliteInteractionSpool> {
        let sqlite = self.connect_repo_sqlite()?;
        SqliteInteractionSpool::new(sqlite, self.repo_id.clone())
    }
}
