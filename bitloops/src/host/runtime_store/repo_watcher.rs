use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

use super::types::{RepoSqliteRuntimeStore, RepoWatcherRegistration};

impl RepoSqliteRuntimeStore {
    pub fn load_watcher_registration(&self) -> Result<Option<RepoWatcherRegistration>> {
        let sqlite = self.connect_repo_sqlite()?;
        sqlite
            .initialise_runtime_checkpoint_schema()
            .context("initialising runtime schema for watcher registration load")?;
        sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT repo_root, pid, restart_token
                 FROM repo_watcher_registrations
                 WHERE repo_id = ?1
                 LIMIT 1",
                rusqlite::params![self.repo_id.as_str()],
                |row| {
                    Ok(RepoWatcherRegistration {
                        repo_id: self.repo_id.clone(),
                        repo_root: std::path::PathBuf::from(row.get::<_, String>(0)?),
                        pid: row.get::<_, u32>(1)?,
                        restart_token: row.get::<_, String>(2)?,
                    })
                },
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
    }

    pub fn save_watcher_registration(
        &self,
        pid: u32,
        restart_token: &str,
        repo_root: &Path,
    ) -> Result<()> {
        let sqlite = self.connect_repo_sqlite()?;
        sqlite
            .initialise_runtime_checkpoint_schema()
            .context("initialising runtime schema for watcher registration save")?;
        sqlite.with_connection(|conn| {
            conn.execute(
                "INSERT INTO repo_watcher_registrations (
                    repo_id, repo_root, pid, restart_token, created_at, updated_at
                 ) VALUES (
                    ?1, ?2, ?3, ?4, datetime('now'), datetime('now')
                 )
                 ON CONFLICT(repo_id) DO UPDATE SET
                    repo_root = excluded.repo_root,
                    pid = excluded.pid,
                    restart_token = excluded.restart_token,
                    updated_at = datetime('now')",
                rusqlite::params![
                    self.repo_id.as_str(),
                    repo_root.to_string_lossy().to_string(),
                    pid,
                    restart_token,
                ],
            )
            .context("upserting repo watcher registration")?;
            Ok(())
        })
    }

    pub fn clear_watcher_registration(&self) -> Result<()> {
        let sqlite = self.connect_repo_sqlite()?;
        sqlite
            .initialise_runtime_checkpoint_schema()
            .context("initialising runtime schema for watcher registration delete")?;
        sqlite.with_connection(|conn| {
            conn.execute(
                "DELETE FROM repo_watcher_registrations WHERE repo_id = ?1",
                rusqlite::params![self.repo_id.as_str()],
            )
            .context("deleting repo watcher registration")?;
            Ok(())
        })
    }

    pub fn delete_watcher_registration_if_matches(
        &self,
        pid: u32,
        restart_token: &str,
    ) -> Result<()> {
        let sqlite = self.connect_repo_sqlite()?;
        sqlite
            .initialise_runtime_checkpoint_schema()
            .context("initialising runtime schema for watcher registration conditional delete")?;
        sqlite.with_connection(|conn| {
            conn.execute(
                "DELETE FROM repo_watcher_registrations
                 WHERE repo_id = ?1 AND pid = ?2 AND restart_token = ?3",
                rusqlite::params![self.repo_id.as_str(), pid, restart_token],
            )
            .context("conditionally deleting repo watcher registration")?;
            Ok(())
        })
    }
}
