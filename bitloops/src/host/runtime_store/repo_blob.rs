use anyhow::{Context, Result};

use crate::config::resolve_store_backend_config_for_repo;
use crate::storage::SqliteConnectionPool;

use super::types::RepoSqliteRuntimeStore;
use super::util::sha256_hex;

impl RepoSqliteRuntimeStore {
    pub(crate) fn open_repo_blob_store(&self) -> Result<crate::storage::blob::ResolvedBlobStore> {
        let cfg = resolve_store_backend_config_for_repo(&self.repo_root)
            .context("resolving backend config for repo runtime metadata")?;
        crate::storage::blob::create_blob_store_with_backend_for_repo(&cfg.blobs, &self.repo_root)
            .context("initialising blob storage for repo runtime metadata")
    }

    pub(crate) fn write_runtime_blob(
        &self,
        key: &str,
        payload: &[u8],
    ) -> Result<(String, String, String, i64)> {
        let resolved = self.open_repo_blob_store()?;
        resolved
            .store
            .write(key, payload)
            .with_context(|| format!("writing runtime metadata blob `{key}`"))?;
        Ok((
            resolved.backend.to_string(),
            key.to_string(),
            format!("sha256:{}", sha256_hex(payload)),
            payload.len() as i64,
        ))
    }

    /// Opens a pooled connection to the repo runtime database (used by several call sites).
    pub(crate) fn connect_repo_sqlite(&self) -> Result<SqliteConnectionPool> {
        SqliteConnectionPool::connect(self.db_path.clone())
            .with_context(|| format!("opening repo runtime database {}", self.db_path.display()))
    }
}
