use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{params, types::Type};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

use crate::storage::SqliteConnectionPool;

const REQUEUE_BACKOFF_SECS: u64 = 5;
const HOOK_SAFE_SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) const LIFECYCLE_STOP_SPOOL_SCHEMA_SQLITE: &str = r#"
CREATE TABLE IF NOT EXISTS agent_lifecycle_stop_spool_jobs (
    job_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    repo_root TEXT NOT NULL,
    config_root TEXT NOT NULL,
    agent_name TEXT NOT NULL,
    hook_name TEXT NOT NULL,
    raw_stdin TEXT NOT NULL,
    workspace_snapshot TEXT NOT NULL DEFAULT '{"modified_files":[],"new_files":[],"deleted_files":[]}',
    cwd TEXT NOT NULL,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at_unix INTEGER NOT NULL,
    received_at_unix INTEGER NOT NULL,
    updated_at_unix INTEGER NOT NULL,
    last_error TEXT
);

CREATE INDEX IF NOT EXISTS idx_agent_lifecycle_stop_spool_status_available
ON agent_lifecycle_stop_spool_jobs (status, available_at_unix, received_at_unix);

CREATE INDEX IF NOT EXISTS idx_agent_lifecycle_stop_spool_repo_status
ON agent_lifecycle_stop_spool_jobs (repo_id, status, received_at_unix);
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LifecycleStopJobStatus {
    Pending,
    Running,
}

impl LifecycleStopJobStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct LifecycleStopWorkspaceSnapshot {
    pub(crate) modified_files: Vec<String>,
    pub(crate) new_files: Vec<String>,
    pub(crate) deleted_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LifecycleStopJobInsert {
    pub(crate) repo_id: String,
    pub(crate) repo_root: PathBuf,
    pub(crate) config_root: PathBuf,
    pub(crate) agent_name: String,
    pub(crate) hook_name: String,
    pub(crate) raw_stdin: String,
    pub(crate) workspace_snapshot: LifecycleStopWorkspaceSnapshot,
    pub(crate) cwd: PathBuf,
    pub(crate) received_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LifecycleStopJobRecord {
    pub(crate) job_id: String,
    pub(crate) repo_id: String,
    pub(crate) repo_root: PathBuf,
    pub(crate) config_root: PathBuf,
    pub(crate) agent_name: String,
    pub(crate) hook_name: String,
    pub(crate) raw_stdin: String,
    pub(crate) workspace_snapshot: LifecycleStopWorkspaceSnapshot,
    pub(crate) cwd: PathBuf,
    pub(crate) status: LifecycleStopJobStatus,
    pub(crate) attempts: u64,
    pub(crate) available_at_unix: u64,
    pub(crate) received_at_unix: u64,
    pub(crate) updated_at_unix: u64,
    pub(crate) last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct LifecycleStopSpoolEnqueueResult {
    pub(crate) inserted_jobs: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LifecycleStopHookEnqueueResult {
    pub(crate) inserted_jobs: u64,
}

pub(crate) fn initialise_lifecycle_stop_spool_schema(sqlite: &SqliteConnectionPool) -> Result<()> {
    sqlite
        .execute_batch(LIFECYCLE_STOP_SPOOL_SCHEMA_SQLITE)
        .context("initialising lifecycle stop spool schema")?;
    sqlite.with_write_connection(ensure_lifecycle_stop_spool_columns)
}

pub(crate) fn unix_timestamp_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn sql_i64(value: u64) -> Result<i64> {
    i64::try_from(value).context("converting unix timestamp to SQLite integer")
}

#[cfg(test)]
pub(crate) fn enqueue_lifecycle_stop_job_sqlite(
    sqlite: &SqliteConnectionPool,
    insert: LifecycleStopJobInsert,
) -> Result<LifecycleStopSpoolEnqueueResult> {
    sqlite.with_write_connection(|conn| {
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
            .context("starting lifecycle stop spool enqueue transaction")?;
        let result =
            insert_lifecycle_stop_job(conn, insert).context("inserting lifecycle stop spool job");
        finish_transaction(conn, result)?;
        Ok(LifecycleStopSpoolEnqueueResult { inserted_jobs: 1 })
    })
}

pub(crate) fn enqueue_lifecycle_stop_job_hook_safe_at(
    db_path: &Path,
    insert: LifecycleStopJobInsert,
) -> Result<LifecycleStopHookEnqueueResult> {
    let conn = open_hook_safe_sqlite(db_path)?;
    insert_lifecycle_stop_job(&conn, insert)
        .context("hook-safe inserting lifecycle stop spool job")?;
    Ok(LifecycleStopHookEnqueueResult { inserted_jobs: 1 })
}

pub(crate) fn claim_next_lifecycle_stop_jobs(
    sqlite: &SqliteConnectionPool,
    limit: usize,
) -> Result<Vec<LifecycleStopJobRecord>> {
    let limit = limit.max(1);
    sqlite.with_write_connection(|conn| {
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
            .context("starting lifecycle stop spool claim transaction")?;
        let result = (|| {
            let now = sql_i64(unix_timestamp_now())?;
            let job_ids = {
                let mut stmt = conn
                    .prepare(
                        "SELECT job_id
                         FROM agent_lifecycle_stop_spool_jobs
                         WHERE status = ?1 AND available_at_unix <= ?2
                         ORDER BY received_at_unix ASC, job_id ASC
                         LIMIT ?3",
                    )
                    .context("preparing lifecycle stop spool claim selection")?;
                let rows = stmt
                    .query_map(
                        params![
                            LifecycleStopJobStatus::Pending.as_str(),
                            now,
                            i64::try_from(limit).context("converting lifecycle claim limit")?,
                        ],
                        |row| row.get::<_, String>(0),
                    )
                    .context("querying lifecycle stop spool claim candidates")?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
                    .context("collecting lifecycle stop spool claim candidates")?
            };

            for job_id in &job_ids {
                conn.execute(
                    "UPDATE agent_lifecycle_stop_spool_jobs
                     SET status = ?1,
                         attempts = attempts + 1,
                         updated_at_unix = ?2,
                         last_error = NULL
                     WHERE job_id = ?3",
                    params![LifecycleStopJobStatus::Running.as_str(), now, job_id],
                )
                .with_context(|| format!("marking lifecycle stop spool job `{job_id}` running"))?;
            }

            let mut records = Vec::with_capacity(job_ids.len());
            for job_id in &job_ids {
                records.push(load_lifecycle_stop_job(conn, job_id)?);
            }
            Ok(records)
        })();
        finish_transaction(conn, result)
    })
}

pub(crate) fn recover_running_lifecycle_stop_jobs(sqlite: &SqliteConnectionPool) -> Result<u64> {
    sqlite.with_write_connection(|conn| {
        let now = sql_i64(unix_timestamp_now())?;
        let updated = conn
            .execute(
                "UPDATE agent_lifecycle_stop_spool_jobs
                 SET status = ?1,
                     available_at_unix = ?2,
                     updated_at_unix = ?2
                 WHERE status = ?3",
                params![
                    LifecycleStopJobStatus::Pending.as_str(),
                    now,
                    LifecycleStopJobStatus::Running.as_str()
                ],
            )
            .context("recovering running lifecycle stop spool jobs")?;
        Ok(u64::try_from(updated).unwrap_or_default())
    })
}

pub(crate) fn lifecycle_stop_spool_repo_ids_with_work(
    sqlite: &SqliteConnectionPool,
) -> Result<HashSet<String>> {
    sqlite.with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT repo_id
                 FROM agent_lifecycle_stop_spool_jobs
                 WHERE status IN (?1, ?2)",
            )
            .context("preparing lifecycle stop spool repo work query")?;
        let rows = stmt.query_map(
            params![
                LifecycleStopJobStatus::Pending.as_str(),
                LifecycleStopJobStatus::Running.as_str(),
            ],
            |row| row.get::<_, String>(0),
        )?;
        rows.collect::<rusqlite::Result<HashSet<_>>>()
            .context("collecting lifecycle stop spool repo ids with work")
    })
}

pub(crate) fn lifecycle_stop_spool_has_repo_work(
    sqlite: &SqliteConnectionPool,
    repo_id: &str,
) -> Result<bool> {
    sqlite.with_connection(|conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM agent_lifecycle_stop_spool_jobs
                 WHERE repo_id = ?1 AND status IN (?2, ?3)",
                params![
                    repo_id,
                    LifecycleStopJobStatus::Pending.as_str(),
                    LifecycleStopJobStatus::Running.as_str(),
                ],
                |row| row.get(0),
            )
            .context("checking lifecycle stop spool repo work")?;
        Ok(count > 0)
    })
}

pub(crate) fn requeue_lifecycle_stop_job(
    sqlite: &SqliteConnectionPool,
    job_id: &str,
    last_error: &str,
) -> Result<()> {
    sqlite.with_write_connection(|conn| {
        let now = unix_timestamp_now();
        conn.execute(
            "UPDATE agent_lifecycle_stop_spool_jobs
             SET status = ?1,
                 available_at_unix = ?2,
                 updated_at_unix = ?3,
                 last_error = ?4
             WHERE job_id = ?5",
            params![
                LifecycleStopJobStatus::Pending.as_str(),
                sql_i64(now.saturating_add(REQUEUE_BACKOFF_SECS))?,
                sql_i64(now)?,
                last_error,
                job_id
            ],
        )
        .with_context(|| format!("requeueing lifecycle stop spool job `{job_id}`"))?;
        Ok(())
    })
}

pub(crate) fn delete_lifecycle_stop_job(sqlite: &SqliteConnectionPool, job_id: &str) -> Result<()> {
    sqlite.with_write_connection(|conn| {
        conn.execute(
            "DELETE FROM agent_lifecycle_stop_spool_jobs WHERE job_id = ?1",
            params![job_id],
        )
        .with_context(|| format!("deleting lifecycle stop spool job `{job_id}`"))?;
        Ok(())
    })
}

#[cfg(test)]
pub(crate) fn list_lifecycle_stop_jobs_for_tests(
    sqlite: &SqliteConnectionPool,
) -> Result<Vec<LifecycleStopJobRecord>> {
    sqlite.with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT job_id, repo_id, repo_root, config_root, agent_name, hook_name,
                        raw_stdin, workspace_snapshot, cwd, status, attempts, available_at_unix,
                        received_at_unix, updated_at_unix, last_error
                 FROM agent_lifecycle_stop_spool_jobs
                 ORDER BY received_at_unix ASC, job_id ASC",
            )
            .context("preparing lifecycle stop spool test list")?;
        let rows = stmt
            .query_map([], map_lifecycle_stop_job)
            .context("listing lifecycle stop spool jobs for tests")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("collecting lifecycle stop spool jobs for tests")
    })
}

fn open_hook_safe_sqlite(db_path: &Path) -> Result<rusqlite::Connection> {
    if let Some(parent) = db_path
        .parent()
        .filter(|candidate| !candidate.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "creating lifecycle stop spool SQLite directory {}",
                parent.display()
            )
        })?;
    }
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
    )
    .with_context(|| format!("opening lifecycle stop spool SQLite {}", db_path.display()))?;
    conn.busy_timeout(HOOK_SAFE_SQLITE_BUSY_TIMEOUT)
        .context("setting hook-safe lifecycle stop SQLite busy timeout")?;
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")
        .context("configuring hook-safe lifecycle stop SQLite pragmas")?;
    conn.execute_batch(LIFECYCLE_STOP_SPOOL_SCHEMA_SQLITE)
        .context("initialising hook-safe lifecycle stop spool schema")?;
    ensure_lifecycle_stop_spool_columns(&conn)?;
    Ok(conn)
}

fn insert_lifecycle_stop_job(
    conn: &rusqlite::Connection,
    insert: LifecycleStopJobInsert,
) -> Result<()> {
    let now = unix_timestamp_now();
    let received_at = if insert.received_at_unix == 0 {
        now
    } else {
        insert.received_at_unix
    };
    let workspace_snapshot = serde_json::to_string(&insert.workspace_snapshot)
        .context("serialising lifecycle stop workspace snapshot")?;
    conn.execute(
        "INSERT INTO agent_lifecycle_stop_spool_jobs (
            job_id, repo_id, repo_root, config_root, agent_name, hook_name,
            raw_stdin, workspace_snapshot, cwd, status, attempts, available_at_unix,
            received_at_unix, updated_at_unix, last_error
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, ?10, 0, ?11,
            ?12, ?13, NULL
         )",
        params![
            format!("lifecycle-stop-job-{}", Uuid::new_v4()),
            insert.repo_id,
            insert.repo_root.to_string_lossy().to_string(),
            insert.config_root.to_string_lossy().to_string(),
            insert.agent_name,
            insert.hook_name,
            insert.raw_stdin,
            workspace_snapshot,
            insert.cwd.to_string_lossy().to_string(),
            LifecycleStopJobStatus::Pending.as_str(),
            sql_i64(now)?,
            sql_i64(received_at)?,
            sql_i64(now)?,
        ],
    )
    .context("inserting lifecycle stop spool job")?;
    Ok(())
}

fn load_lifecycle_stop_job(
    conn: &rusqlite::Connection,
    job_id: &str,
) -> Result<LifecycleStopJobRecord> {
    conn.query_row(
        "SELECT job_id, repo_id, repo_root, config_root, agent_name, hook_name,
                raw_stdin, workspace_snapshot, cwd, status, attempts, available_at_unix,
                received_at_unix, updated_at_unix, last_error
         FROM agent_lifecycle_stop_spool_jobs
         WHERE job_id = ?1",
        params![job_id],
        map_lifecycle_stop_job,
    )
    .with_context(|| format!("loading lifecycle stop spool job `{job_id}`"))
}

fn map_lifecycle_stop_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<LifecycleStopJobRecord> {
    Ok(LifecycleStopJobRecord {
        job_id: row.get("job_id")?,
        repo_id: row.get("repo_id")?,
        repo_root: PathBuf::from(row.get::<_, String>("repo_root")?),
        config_root: PathBuf::from(row.get::<_, String>("config_root")?),
        agent_name: row.get("agent_name")?,
        hook_name: row.get("hook_name")?,
        raw_stdin: row.get("raw_stdin")?,
        workspace_snapshot: parse_workspace_snapshot(row.get("workspace_snapshot")?),
        cwd: PathBuf::from(row.get::<_, String>("cwd")?),
        status: LifecycleStopJobStatus::parse(&row.get::<_, String>("status")?),
        attempts: row_i64_as_u64(row, "attempts")?,
        available_at_unix: row_i64_as_u64(row, "available_at_unix")?,
        received_at_unix: row_i64_as_u64(row, "received_at_unix")?,
        updated_at_unix: row_i64_as_u64(row, "updated_at_unix")?,
        last_error: row.get("last_error")?,
    })
}

fn parse_workspace_snapshot(raw: String) -> LifecycleStopWorkspaceSnapshot {
    serde_json::from_str(&raw).unwrap_or_default()
}

fn ensure_lifecycle_stop_spool_columns(conn: &rusqlite::Connection) -> Result<()> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(agent_lifecycle_stop_spool_jobs)")
        .context("preparing lifecycle stop spool table info query")?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .context("querying lifecycle stop spool table columns")?;
    let columns = rows
        .collect::<rusqlite::Result<HashSet<_>>>()
        .context("collecting lifecycle stop spool table columns")?;
    if !columns.contains("workspace_snapshot") {
        conn.execute_batch(
            r#"ALTER TABLE agent_lifecycle_stop_spool_jobs
               ADD COLUMN workspace_snapshot TEXT NOT NULL DEFAULT '{"modified_files":[],"new_files":[],"deleted_files":[]}'"#,
        )
        .context("adding lifecycle stop workspace snapshot column")?;
    }
    Ok(())
}

fn row_i64_as_u64(row: &rusqlite::Row<'_>, column: &str) -> rusqlite::Result<u64> {
    let value: i64 = row.get(column)?;
    u64::try_from(value)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(0, Type::Integer, Box::new(err)))
}

fn finish_transaction<T>(conn: &rusqlite::Connection, result: Result<T>) -> Result<T> {
    match result {
        Ok(value) => {
            conn.execute_batch("COMMIT")
                .context("committing lifecycle stop spool transaction")?;
            Ok(value)
        }
        Err(err) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sqlite_at(dir: &tempfile::TempDir) -> SqliteConnectionPool {
        SqliteConnectionPool::connect(dir.path().join("runtime.sqlite")).expect("open sqlite")
    }

    fn sample_insert(repo_root: &Path) -> LifecycleStopJobInsert {
        LifecycleStopJobInsert {
            repo_id: "repo-1".to_string(),
            repo_root: repo_root.to_path_buf(),
            config_root: repo_root.join(".bitloops-test-state/daemon"),
            agent_name: crate::adapters::agents::AGENT_NAME_CODEX.to_string(),
            hook_name: crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_STOP.to_string(),
            raw_stdin: r#"{"session_id":"session-1","transcript_path":"/tmp/session.jsonl"}"#
                .to_string(),
            workspace_snapshot: LifecycleStopWorkspaceSnapshot::default(),
            cwd: repo_root.to_path_buf(),
            received_at_unix: 1_778_800_000,
        }
    }

    #[test]
    fn lifecycle_stop_spool_inserts_and_claims_pending_jobs() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let sqlite = sqlite_at(&dir);
        initialise_lifecycle_stop_spool_schema(&sqlite)?;

        let inserted = enqueue_lifecycle_stop_job_sqlite(&sqlite, sample_insert(dir.path()))?;
        assert_eq!(inserted.inserted_jobs, 1);

        let claimed = claim_next_lifecycle_stop_jobs(&sqlite, 8)?;
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].repo_id, "repo-1");
        assert_eq!(
            claimed[0].agent_name,
            crate::adapters::agents::AGENT_NAME_CODEX
        );
        assert_eq!(claimed[0].status, LifecycleStopJobStatus::Running);

        let claimed_again = claim_next_lifecycle_stop_jobs(&sqlite, 8)?;
        assert!(claimed_again.is_empty());
        Ok(())
    }

    #[test]
    fn lifecycle_stop_spool_recovers_running_jobs() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let sqlite = sqlite_at(&dir);
        initialise_lifecycle_stop_spool_schema(&sqlite)?;
        enqueue_lifecycle_stop_job_sqlite(&sqlite, sample_insert(dir.path()))?;

        let first_claim = claim_next_lifecycle_stop_jobs(&sqlite, 8)?;
        assert_eq!(first_claim.len(), 1);

        let recovered = recover_running_lifecycle_stop_jobs(&sqlite)?;
        assert_eq!(recovered, 1);

        let second_claim = claim_next_lifecycle_stop_jobs(&sqlite, 8)?;
        assert_eq!(second_claim.len(), 1);
        assert_eq!(second_claim[0].attempts, 2);
        Ok(())
    }

    #[test]
    fn lifecycle_stop_spool_requeues_failed_jobs_with_error() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let sqlite = sqlite_at(&dir);
        initialise_lifecycle_stop_spool_schema(&sqlite)?;
        enqueue_lifecycle_stop_job_sqlite(&sqlite, sample_insert(dir.path()))?;

        let claimed = claim_next_lifecycle_stop_jobs(&sqlite, 8)?;
        requeue_lifecycle_stop_job(&sqlite, &claimed[0].job_id, "parse failed")?;

        let rows = list_lifecycle_stop_jobs_for_tests(&sqlite)?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, LifecycleStopJobStatus::Pending);
        assert_eq!(rows[0].last_error.as_deref(), Some("parse failed"));
        Ok(())
    }

    #[test]
    fn lifecycle_stop_spool_deletes_completed_jobs() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let sqlite = sqlite_at(&dir);
        initialise_lifecycle_stop_spool_schema(&sqlite)?;
        enqueue_lifecycle_stop_job_sqlite(&sqlite, sample_insert(dir.path()))?;

        let claimed = claim_next_lifecycle_stop_jobs(&sqlite, 8)?;
        delete_lifecycle_stop_job(&sqlite, &claimed[0].job_id)?;

        assert!(list_lifecycle_stop_jobs_for_tests(&sqlite)?.is_empty());
        Ok(())
    }

    #[test]
    fn lifecycle_stop_hook_enqueue_writes_sqlite_row_with_bounded_connection() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let db_path = dir.path().join("stores/runtime/runtime.sqlite");

        let result = enqueue_lifecycle_stop_job_hook_safe_at(&db_path, sample_insert(dir.path()))?;

        assert_eq!(result.inserted_jobs, 1);
        let sqlite = SqliteConnectionPool::connect(db_path)?;
        let rows = list_lifecycle_stop_jobs_for_tests(&sqlite)?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].raw_stdin, sample_insert(dir.path()).raw_stdin);
        Ok(())
    }

    #[test]
    fn lifecycle_stop_spool_round_trips_workspace_snapshot() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let sqlite = sqlite_at(&dir);
        initialise_lifecycle_stop_spool_schema(&sqlite)?;
        let mut insert = sample_insert(dir.path());
        insert.workspace_snapshot = LifecycleStopWorkspaceSnapshot {
            modified_files: vec!["src/main.rs".to_string()],
            new_files: vec!["src/new.rs".to_string()],
            deleted_files: vec!["old.rs".to_string()],
        };

        enqueue_lifecycle_stop_job_sqlite(&sqlite, insert)?;

        let rows = list_lifecycle_stop_jobs_for_tests(&sqlite)?;
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].workspace_snapshot.modified_files,
            vec!["src/main.rs"]
        );
        assert_eq!(rows[0].workspace_snapshot.new_files, vec!["src/new.rs"]);
        assert_eq!(rows[0].workspace_snapshot.deleted_files, vec!["old.rs"]);
        Ok(())
    }

    #[test]
    fn lifecycle_stop_hook_enqueue_fails_quickly_when_runtime_sqlite_write_lock_is_held()
    -> Result<()> {
        let dir = tempfile::tempdir()?;
        let db_path = dir.path().join("runtime.sqlite");
        let locker = rusqlite::Connection::open(&db_path)?;
        locker.execute_batch(LIFECYCLE_STOP_SPOOL_SCHEMA_SQLITE)?;
        locker.busy_timeout(Duration::from_secs(30))?;
        locker.execute_batch("BEGIN IMMEDIATE TRANSACTION;")?;

        let started = std::time::Instant::now();
        let result = enqueue_lifecycle_stop_job_hook_safe_at(&db_path, sample_insert(dir.path()));

        locker.execute_batch("ROLLBACK")?;
        assert!(result.is_err());
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "hook-safe enqueue waited too long: {:?}",
            started.elapsed()
        );
        Ok(())
    }
}
