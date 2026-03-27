use super::DevqlGraphqlContext;
use crate::config::resolve_store_backend_config_for_repo;
use crate::graphql::types::Checkpoint;
use crate::host::checkpoints::strategy::manual_commit::read_committed_info;
use crate::host::devql::resolve_repo_identity;
use crate::storage::SqliteConnectionPool;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::Path;
use tokio::task;

impl DevqlGraphqlContext {
    pub(crate) async fn list_commit_checkpoints(
        &self,
        commit_sha: &str,
    ) -> Result<Vec<Checkpoint>> {
        let repo_root = self.repo_root.clone();
        let repo_id = self.repo_identity.repo_id.clone();
        let commit_sha = commit_sha.to_string();
        let sqlite_path = self
            .backend_config
            .as_ref()
            .context("store backend configuration unavailable")?
            .relational
            .resolve_sqlite_db_path_for_repo(&self.repo_root)
            .context("resolving SQLite path for commit checkpoints")?;

        task::spawn_blocking(move || -> Result<Vec<Checkpoint>> {
            let sqlite = match SqliteConnectionPool::connect_existing(sqlite_path) {
                Ok(sqlite) => sqlite,
                Err(err) if is_missing_sqlite_store_error(&err) => return Ok(Vec::new()),
                Err(err) => return Err(err).context("opening checkpoint SQLite store"),
            };
            sqlite
                .initialise_checkpoint_schema()
                .context("initialising checkpoint schema for GraphQL commit checkpoints")?;
            let checkpoint_ids = sqlite.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT checkpoint_id
                     FROM commit_checkpoints
                     WHERE repo_id = ?1 AND commit_sha = ?2
                     ORDER BY created_at DESC, checkpoint_id DESC",
                )?;
                let mut rows =
                    stmt.query(rusqlite::params![repo_id.as_str(), commit_sha.as_str()])?;
                let mut ids = Vec::new();
                while let Some(row) = rows.next()? {
                    ids.push(row.get::<_, String>(0)?);
                }
                Ok(ids)
            })?;

            let mut checkpoints = Vec::new();
            for checkpoint_id in checkpoint_ids {
                if let Some(info) = read_committed_info(repo_root.as_path(), &checkpoint_id)? {
                    checkpoints.push(Checkpoint::from_committed(&commit_sha, &info));
                }
            }
            Ok(checkpoints)
        })
        .await
        .context("joining commit checkpoint query task")?
    }
}

pub(super) fn read_commit_checkpoint_mappings_all(
    repo_root: &Path,
) -> Result<BTreeMap<String, Vec<String>>> {
    let cfg = resolve_store_backend_config_for_repo(repo_root)?;
    let sqlite_path = cfg.relational.resolve_sqlite_db_path_for_repo(repo_root)?;
    let sqlite = SqliteConnectionPool::connect_existing(sqlite_path)
        .context("opening SQLite database for commit-checkpoint mappings")?;
    sqlite
        .initialise_checkpoint_schema()
        .context("initialising checkpoint schema for commit-checkpoint mappings")?;
    let repo_id = resolve_repo_identity(repo_root)?.repo_id;

    sqlite.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT commit_sha, checkpoint_id
             FROM commit_checkpoints
             WHERE repo_id = ?1
             ORDER BY created_at DESC, checkpoint_id DESC",
        )?;
        let mut rows = stmt.query(rusqlite::params![repo_id.as_str()])?;
        let mut out = BTreeMap::<String, Vec<String>>::new();
        while let Some(row) = rows.next()? {
            let commit_sha = row.get::<_, String>(0)?.trim().to_string();
            let checkpoint_id = row.get::<_, String>(1)?.trim().to_string();
            if commit_sha.is_empty() || checkpoint_id.is_empty() {
                continue;
            }
            out.entry(commit_sha).or_default().push(checkpoint_id);
        }
        Ok(out)
    })
}

pub(super) fn is_missing_sqlite_store_error(err: &anyhow::Error) -> bool {
    format!("{err:#}").contains("SQLite database file not found")
}
