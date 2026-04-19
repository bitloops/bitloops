use std::path::Path;

use anyhow::{Context, Result, bail};
use rusqlite::OptionalExtension;

use super::types::{RepoSqliteRuntimeStore, RepoWatcherRegistration, RepoWatcherRegistrationState};

impl RepoSqliteRuntimeStore {
    pub fn load_watcher_registration(&self) -> Result<Option<RepoWatcherRegistration>> {
        let sqlite = self.connect_repo_sqlite()?;
        sqlite
            .initialise_runtime_checkpoint_schema()
            .context("initialising runtime schema for watcher registration load")?;
        sqlite.with_connection(|conn| {
            load_watcher_registration_record(conn, self.repo_id.as_str())
                .context("loading watcher registration row")?
                .map(|record| map_watcher_registration_record(self.repo_id.clone(), record))
                .transpose()
        })
    }

    pub fn save_watcher_registration(
        &self,
        pid: u32,
        restart_token: &str,
        repo_root: &Path,
        state: RepoWatcherRegistrationState,
    ) -> Result<()> {
        let sqlite = self.connect_repo_sqlite()?;
        sqlite
            .initialise_runtime_checkpoint_schema()
            .context("initialising runtime schema for watcher registration save")?;
        sqlite.with_connection(|conn| {
            upsert_watcher_registration(
                conn,
                self.repo_id.as_str(),
                repo_root,
                pid,
                restart_token,
                state,
            )?;
            Ok(())
        })
    }

    pub fn claim_pending_watcher_registration(
        &self,
        pid: u32,
        restart_token: &str,
        repo_root: &Path,
    ) -> Result<Option<RepoWatcherRegistration>> {
        let sqlite = self.connect_repo_sqlite()?;
        sqlite
            .initialise_runtime_checkpoint_schema()
            .context("initialising runtime schema for watcher registration pending claim")?;
        sqlite.with_connection(|conn| {
            with_immediate_transaction(conn, || {
                let existing = load_watcher_registration_record(conn, self.repo_id.as_str())?;
                match existing {
                    Some(record) if record.pid != pid || record.restart_token != restart_token => {
                        Ok(Some(map_watcher_registration_record(
                            self.repo_id.clone(),
                            record,
                        )?))
                    }
                    Some(record) => {
                        if record.state != RepoWatcherRegistrationState::Ready {
                            upsert_watcher_registration(
                                conn,
                                self.repo_id.as_str(),
                                repo_root,
                                pid,
                                restart_token,
                                RepoWatcherRegistrationState::Pending,
                            )?;
                        }
                        Ok(None)
                    }
                    None => {
                        upsert_watcher_registration(
                            conn,
                            self.repo_id.as_str(),
                            repo_root,
                            pid,
                            restart_token,
                            RepoWatcherRegistrationState::Pending,
                        )?;
                        Ok(None)
                    }
                }
            })
        })
    }

    pub fn promote_watcher_registration_to_ready(
        &self,
        pid: u32,
        restart_token: &str,
        repo_root: &Path,
    ) -> Result<()> {
        let sqlite = self.connect_repo_sqlite()?;
        sqlite
            .initialise_runtime_checkpoint_schema()
            .context("initialising runtime schema for watcher registration ready promote")?;
        sqlite.with_connection(|conn| {
            with_immediate_transaction(conn, || {
                match load_watcher_registration_record(conn, self.repo_id.as_str())? {
                    Some(record)
                        if record.pid != pid || record.restart_token != restart_token =>
                    {
                        let existing =
                            map_watcher_registration_record(self.repo_id.clone(), record)?;
                        bail!(
                            "watcher registration for repo {} is already owned by pid {} in {} state",
                            self.repo_id,
                            existing.pid,
                            existing.state.as_str()
                        );
                    }
                    _ => {
                        upsert_watcher_registration(
                            conn,
                            self.repo_id.as_str(),
                            repo_root,
                            pid,
                            restart_token,
                            RepoWatcherRegistrationState::Ready,
                        )?;
                    }
                }
                Ok(())
            })
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

    pub fn delete_pending_watcher_registration_if_matches(
        &self,
        pid: u32,
        restart_token: &str,
    ) -> Result<bool> {
        let sqlite = self.connect_repo_sqlite()?;
        sqlite
            .initialise_runtime_checkpoint_schema()
            .context("initialising runtime schema for pending watcher registration delete")?;
        sqlite.with_connection(|conn| {
            let deleted = conn
                .execute(
                    "DELETE FROM repo_watcher_registrations
                     WHERE repo_id = ?1 AND pid = ?2 AND restart_token = ?3 AND state = ?4",
                    rusqlite::params![
                        self.repo_id.as_str(),
                        pid,
                        restart_token,
                        RepoWatcherRegistrationState::Pending.as_str(),
                    ],
                )
                .context("conditionally deleting pending repo watcher registration")?;
            Ok(deleted > 0)
        })
    }
}

#[derive(Debug, Clone)]
struct WatcherRegistrationRecord {
    repo_root: String,
    pid: u32,
    restart_token: String,
    state: RepoWatcherRegistrationState,
}

fn load_watcher_registration_record(
    conn: &rusqlite::Connection,
    repo_id: &str,
) -> Result<Option<WatcherRegistrationRecord>> {
    conn.query_row(
        "SELECT repo_root, pid, restart_token, state
         FROM repo_watcher_registrations
         WHERE repo_id = ?1
         LIMIT 1",
        rusqlite::params![repo_id],
        |row| {
            let state = row.get::<_, String>(3)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, String>(2)?,
                state,
            ))
        },
    )
    .optional()
    .map_err(anyhow::Error::from)?
    .map(|(repo_root, pid, restart_token, state)| {
        let state = RepoWatcherRegistrationState::from_str(&state)
            .with_context(|| format!("unknown repo watcher registration state `{state}`"))?;
        Ok(WatcherRegistrationRecord {
            repo_root,
            pid,
            restart_token,
            state,
        })
    })
    .transpose()
}

fn map_watcher_registration_record(
    repo_id: String,
    record: WatcherRegistrationRecord,
) -> Result<RepoWatcherRegistration> {
    Ok(RepoWatcherRegistration {
        repo_id,
        repo_root: std::path::PathBuf::from(record.repo_root),
        pid: record.pid,
        restart_token: record.restart_token,
        state: record.state,
    })
}

fn upsert_watcher_registration(
    conn: &rusqlite::Connection,
    repo_id: &str,
    repo_root: &Path,
    pid: u32,
    restart_token: &str,
    state: RepoWatcherRegistrationState,
) -> Result<()> {
    conn.execute(
        "INSERT INTO repo_watcher_registrations (
            repo_id, repo_root, pid, restart_token, state, created_at, updated_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, datetime('now'), datetime('now')
         )
         ON CONFLICT(repo_id) DO UPDATE SET
            repo_root = excluded.repo_root,
            pid = excluded.pid,
            restart_token = excluded.restart_token,
            state = excluded.state,
            updated_at = datetime('now')",
        rusqlite::params![
            repo_id,
            repo_root.to_string_lossy().to_string(),
            pid,
            restart_token,
            state.as_str(),
        ],
    )
    .context("upserting repo watcher registration")?;
    Ok(())
}

fn with_immediate_transaction<T>(
    conn: &rusqlite::Connection,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
        .context("starting SQLite immediate transaction")?;
    match operation() {
        Ok(value) => {
            conn.execute_batch("COMMIT;")
                .context("committing SQLite immediate transaction")?;
            Ok(value)
        }
        Err(err) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(err)
        }
    }
}
