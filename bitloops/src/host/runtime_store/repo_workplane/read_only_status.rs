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
        let sqlite =
            ReadOnlySqliteConnectionPool::connect_existing(db_path.clone()).with_context(|| {
                format!(
                    "opening repo runtime database read-only {}",
                    db_path.display()
                )
            })?;
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
