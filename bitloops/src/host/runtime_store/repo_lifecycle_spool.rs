use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params, types::Type};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::storage::SqliteConnectionPool;

const REQUEUE_BACKOFF_SECS: u64 = 5;
const HOOK_SAFE_SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) const MAX_LIFECYCLE_JOB_ATTEMPTS: u64 = 5;

pub(crate) const LIFECYCLE_SPOOL_SCHEMA_SQLITE: &str = r#"
CREATE TABLE IF NOT EXISTS agent_lifecycle_spool_jobs (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL UNIQUE,
    repo_id TEXT NOT NULL,
    repo_root TEXT NOT NULL,
    config_root TEXT NOT NULL,
    agent_name TEXT NOT NULL,
    hook_name TEXT NOT NULL,
    raw_stdin TEXT NOT NULL,
    workspace_snapshot TEXT,
    cwd TEXT NOT NULL,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at_unix INTEGER NOT NULL,
    received_at_unix INTEGER NOT NULL,
    updated_at_unix INTEGER NOT NULL,
    last_error TEXT
);

CREATE INDEX IF NOT EXISTS idx_agent_lifecycle_spool_status_sequence
ON agent_lifecycle_spool_jobs (status, sequence);

CREATE INDEX IF NOT EXISTS idx_agent_lifecycle_spool_repo_status_sequence
ON agent_lifecycle_spool_jobs (repo_id, status, sequence);
"#;

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
pub(crate) enum LifecycleJobStatus {
    Pending,
    Running,
    Failed,
}

impl LifecycleJobStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

pub(crate) type LifecycleStopJobStatus = LifecycleJobStatus;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct LifecycleStopWorkspaceSnapshot {
    pub(crate) modified_files: Vec<String>,
    pub(crate) new_files: Vec<String>,
    pub(crate) deleted_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LifecycleJobInsert {
    pub(crate) repo_id: String,
    pub(crate) repo_root: PathBuf,
    pub(crate) config_root: PathBuf,
    pub(crate) agent_name: String,
    pub(crate) hook_name: String,
    pub(crate) raw_stdin: String,
    pub(crate) workspace_snapshot: Option<LifecycleStopWorkspaceSnapshot>,
    pub(crate) cwd: PathBuf,
    pub(crate) received_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LifecycleJobRecord {
    pub(crate) sequence: u64,
    pub(crate) job_id: String,
    pub(crate) repo_id: String,
    pub(crate) repo_root: PathBuf,
    pub(crate) config_root: PathBuf,
    pub(crate) agent_name: String,
    pub(crate) hook_name: String,
    pub(crate) raw_stdin: String,
    pub(crate) workspace_snapshot: Option<LifecycleStopWorkspaceSnapshot>,
    pub(crate) cwd: PathBuf,
    pub(crate) status: LifecycleJobStatus,
    pub(crate) attempts: u64,
    pub(crate) available_at_unix: u64,
    pub(crate) received_at_unix: u64,
    pub(crate) updated_at_unix: u64,
    pub(crate) last_error: Option<String>,
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

impl From<LifecycleStopJobInsert> for LifecycleJobInsert {
    fn from(insert: LifecycleStopJobInsert) -> Self {
        Self {
            repo_id: insert.repo_id,
            repo_root: insert.repo_root,
            config_root: insert.config_root,
            agent_name: insert.agent_name,
            hook_name: insert.hook_name,
            raw_stdin: insert.raw_stdin,
            workspace_snapshot: Some(insert.workspace_snapshot),
            cwd: insert.cwd,
            received_at_unix: insert.received_at_unix,
        }
    }
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

impl From<LifecycleJobRecord> for LifecycleStopJobRecord {
    fn from(job: LifecycleJobRecord) -> Self {
        Self {
            job_id: job.job_id,
            repo_id: job.repo_id,
            repo_root: job.repo_root,
            config_root: job.config_root,
            agent_name: job.agent_name,
            hook_name: job.hook_name,
            raw_stdin: job.raw_stdin,
            workspace_snapshot: job.workspace_snapshot.unwrap_or_default(),
            cwd: job.cwd,
            status: job.status,
            attempts: job.attempts,
            available_at_unix: job.available_at_unix,
            received_at_unix: job.received_at_unix,
            updated_at_unix: job.updated_at_unix,
            last_error: job.last_error,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct LifecycleSpoolEnqueueResult {
    pub(crate) inserted_jobs: u64,
}

#[cfg(test)]
pub(crate) type LifecycleStopSpoolEnqueueResult = LifecycleSpoolEnqueueResult;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LifecycleHookEnqueueResult {
    pub(crate) inserted_jobs: u64,
}

pub(crate) type LifecycleStopHookEnqueueResult = LifecycleHookEnqueueResult;

pub(crate) fn initialise_lifecycle_spool_schema(sqlite: &SqliteConnectionPool) -> Result<()> {
    sqlite
        .execute_batch(LIFECYCLE_SPOOL_SCHEMA_SQLITE)
        .context("initialising lifecycle spool schema")?;
    sqlite.with_write_connection(migrate_legacy_lifecycle_stop_spool_jobs)
}

pub(crate) fn initialise_lifecycle_stop_spool_schema(sqlite: &SqliteConnectionPool) -> Result<()> {
    initialise_lifecycle_spool_schema(sqlite)
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
pub(crate) fn enqueue_lifecycle_job_sqlite(
    sqlite: &SqliteConnectionPool,
    insert: LifecycleJobInsert,
) -> Result<LifecycleSpoolEnqueueResult> {
    sqlite.with_write_connection(|conn| {
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
            .context("starting lifecycle spool enqueue transaction")?;
        let result = insert_lifecycle_job(conn, insert).context("inserting lifecycle spool job");
        finish_transaction(conn, result)?;
        Ok(LifecycleSpoolEnqueueResult { inserted_jobs: 1 })
    })
}

#[cfg(test)]
pub(crate) fn enqueue_lifecycle_stop_job_sqlite(
    sqlite: &SqliteConnectionPool,
    insert: LifecycleStopJobInsert,
) -> Result<LifecycleStopSpoolEnqueueResult> {
    enqueue_lifecycle_job_sqlite(sqlite, insert.into())
}

pub(crate) fn enqueue_lifecycle_job_hook_safe_at(
    db_path: &Path,
    insert: LifecycleJobInsert,
) -> Result<LifecycleHookEnqueueResult> {
    let conn = open_hook_safe_sqlite(db_path)?;
    insert_lifecycle_job(&conn, insert).context("hook-safe inserting lifecycle spool job")?;
    Ok(LifecycleHookEnqueueResult { inserted_jobs: 1 })
}

pub(crate) fn enqueue_lifecycle_stop_job_hook_safe_at(
    db_path: &Path,
    insert: LifecycleStopJobInsert,
) -> Result<LifecycleStopHookEnqueueResult> {
    enqueue_lifecycle_job_hook_safe_at(db_path, insert.into())
}

pub(crate) fn claim_next_lifecycle_job(
    sqlite: &SqliteConnectionPool,
) -> Result<Option<LifecycleJobRecord>> {
    sqlite.with_write_connection(|conn| {
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
            .context("starting lifecycle spool claim transaction")?;
        let result = (|| {
            let Some(job) = select_oldest_active_lifecycle_job(conn)? else {
                return Ok(None);
            };
            if job.status == LifecycleJobStatus::Running {
                return Ok(None);
            }

            let now = unix_timestamp_now();
            if job.available_at_unix > now {
                return Ok(None);
            }

            conn.execute(
                "UPDATE agent_lifecycle_spool_jobs
                 SET status = ?1,
                     attempts = attempts + 1,
                     updated_at_unix = ?2,
                     last_error = NULL
                 WHERE job_id = ?3",
                params![
                    LifecycleJobStatus::Running.as_str(),
                    sql_i64(now)?,
                    job.job_id
                ],
            )
            .with_context(|| format!("marking lifecycle spool job `{}` running", job.job_id))?;

            Ok(Some(load_lifecycle_job(conn, &job.job_id)?))
        })();
        finish_transaction(conn, result)
    })
}

pub(crate) fn claim_next_lifecycle_stop_jobs(
    sqlite: &SqliteConnectionPool,
    _limit: usize,
) -> Result<Vec<LifecycleStopJobRecord>> {
    Ok(claim_next_lifecycle_job(sqlite)?
        .map(LifecycleStopJobRecord::from)
        .into_iter()
        .collect())
}

pub(crate) fn recover_running_lifecycle_jobs(sqlite: &SqliteConnectionPool) -> Result<u64> {
    sqlite.with_write_connection(|conn| {
        let now = sql_i64(unix_timestamp_now())?;
        let updated = conn
            .execute(
                "UPDATE agent_lifecycle_spool_jobs
                 SET status = ?1,
                     available_at_unix = ?2,
                     updated_at_unix = ?2
                 WHERE status = ?3",
                params![
                    LifecycleJobStatus::Pending.as_str(),
                    now,
                    LifecycleJobStatus::Running.as_str()
                ],
            )
            .context("recovering running lifecycle spool jobs")?;
        Ok(u64::try_from(updated).unwrap_or_default())
    })
}

pub(crate) fn recover_running_lifecycle_stop_jobs(sqlite: &SqliteConnectionPool) -> Result<u64> {
    recover_running_lifecycle_jobs(sqlite)
}

pub(crate) fn lifecycle_spool_repo_ids_with_work(
    sqlite: &SqliteConnectionPool,
) -> Result<HashSet<String>> {
    sqlite.with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT repo_id
                 FROM agent_lifecycle_spool_jobs
                 WHERE status IN (?1, ?2)",
            )
            .context("preparing lifecycle spool repo work query")?;
        let rows = stmt.query_map(
            params![
                LifecycleJobStatus::Pending.as_str(),
                LifecycleJobStatus::Running.as_str(),
            ],
            |row| row.get::<_, String>(0),
        )?;
        rows.collect::<rusqlite::Result<HashSet<_>>>()
            .context("collecting lifecycle spool repo ids with work")
    })
}

pub(crate) fn lifecycle_stop_spool_repo_ids_with_work(
    sqlite: &SqliteConnectionPool,
) -> Result<HashSet<String>> {
    lifecycle_spool_repo_ids_with_work(sqlite)
}

pub(crate) fn lifecycle_spool_has_repo_work(
    sqlite: &SqliteConnectionPool,
    repo_id: &str,
) -> Result<bool> {
    sqlite.with_connection(|conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM agent_lifecycle_spool_jobs
                 WHERE repo_id = ?1 AND status IN (?2, ?3)",
                params![
                    repo_id,
                    LifecycleJobStatus::Pending.as_str(),
                    LifecycleJobStatus::Running.as_str(),
                ],
                |row| row.get(0),
            )
            .context("checking lifecycle spool repo work")?;
        Ok(count > 0)
    })
}

pub(crate) fn lifecycle_stop_spool_has_repo_work(
    sqlite: &SqliteConnectionPool,
    repo_id: &str,
) -> Result<bool> {
    lifecycle_spool_has_repo_work(sqlite, repo_id)
}

pub(crate) fn requeue_lifecycle_job(
    sqlite: &SqliteConnectionPool,
    job_id: &str,
    last_error: &str,
) -> Result<()> {
    sqlite.with_write_connection(|conn| {
        let now = unix_timestamp_now();
        conn.execute(
            "UPDATE agent_lifecycle_spool_jobs
             SET status = ?1,
                 available_at_unix = ?2,
                 updated_at_unix = ?3,
                 last_error = ?4
             WHERE job_id = ?5",
            params![
                LifecycleJobStatus::Pending.as_str(),
                sql_i64(now.saturating_add(REQUEUE_BACKOFF_SECS))?,
                sql_i64(now)?,
                last_error,
                job_id
            ],
        )
        .with_context(|| format!("requeueing lifecycle spool job `{job_id}`"))?;
        Ok(())
    })
}

pub(crate) fn requeue_lifecycle_stop_job(
    sqlite: &SqliteConnectionPool,
    job_id: &str,
    last_error: &str,
) -> Result<()> {
    requeue_lifecycle_job(sqlite, job_id, last_error)
}

pub(crate) fn mark_lifecycle_job_failed(
    sqlite: &SqliteConnectionPool,
    job_id: &str,
    last_error: &str,
) -> Result<()> {
    sqlite.with_write_connection(|conn| {
        let now = unix_timestamp_now();
        conn.execute(
            "UPDATE agent_lifecycle_spool_jobs
             SET status = ?1,
                 updated_at_unix = ?2,
                 last_error = ?3
             WHERE job_id = ?4",
            params![
                LifecycleJobStatus::Failed.as_str(),
                sql_i64(now)?,
                last_error,
                job_id
            ],
        )
        .with_context(|| format!("marking lifecycle spool job `{job_id}` failed"))?;
        Ok(())
    })
}

pub(crate) fn fail_or_requeue_lifecycle_job(
    sqlite: &SqliteConnectionPool,
    job: &LifecycleJobRecord,
    last_error: &str,
) -> Result<()> {
    if job.attempts >= MAX_LIFECYCLE_JOB_ATTEMPTS {
        mark_lifecycle_job_failed(sqlite, &job.job_id, last_error)
    } else {
        requeue_lifecycle_job(sqlite, &job.job_id, last_error)
    }
}

pub(crate) fn delete_lifecycle_job(sqlite: &SqliteConnectionPool, job_id: &str) -> Result<()> {
    sqlite.with_write_connection(|conn| {
        conn.execute(
            "DELETE FROM agent_lifecycle_spool_jobs WHERE job_id = ?1",
            params![job_id],
        )
        .with_context(|| format!("deleting lifecycle spool job `{job_id}`"))?;
        Ok(())
    })
}

pub(crate) fn delete_lifecycle_stop_job(sqlite: &SqliteConnectionPool, job_id: &str) -> Result<()> {
    delete_lifecycle_job(sqlite, job_id)
}

#[cfg(test)]
pub(crate) fn list_lifecycle_jobs_for_tests(
    sqlite: &SqliteConnectionPool,
) -> Result<Vec<LifecycleJobRecord>> {
    sqlite.with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT sequence, job_id, repo_id, repo_root, config_root, agent_name, hook_name,
                        raw_stdin, workspace_snapshot, cwd, status, attempts, available_at_unix,
                        received_at_unix, updated_at_unix, last_error
                 FROM agent_lifecycle_spool_jobs
                 ORDER BY sequence ASC",
            )
            .context("preparing lifecycle spool test list")?;
        let rows = stmt
            .query_map([], map_lifecycle_job)
            .context("listing lifecycle spool jobs for tests")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("collecting lifecycle spool jobs for tests")
    })
}

#[cfg(test)]
pub(crate) fn list_lifecycle_stop_jobs_for_tests(
    sqlite: &SqliteConnectionPool,
) -> Result<Vec<LifecycleStopJobRecord>> {
    Ok(list_lifecycle_jobs_for_tests(sqlite)?
        .into_iter()
        .map(LifecycleStopJobRecord::from)
        .collect())
}

#[cfg(test)]
pub(crate) fn force_pending_job_available_for_tests(
    sqlite: &SqliteConnectionPool,
    job_id: &str,
) -> Result<()> {
    sqlite.with_write_connection(|conn| {
        let now = unix_timestamp_now();
        conn.execute(
            "UPDATE agent_lifecycle_spool_jobs
             SET available_at_unix = ?1,
                 updated_at_unix = ?1
             WHERE job_id = ?2 AND status = ?3",
            params![sql_i64(now)?, job_id, LifecycleJobStatus::Pending.as_str()],
        )
        .with_context(|| format!("forcing lifecycle spool job `{job_id}` available"))?;
        Ok(())
    })
}

fn open_hook_safe_sqlite(db_path: &Path) -> Result<rusqlite::Connection> {
    if let Some(parent) = db_path
        .parent()
        .filter(|candidate| !candidate.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "creating lifecycle spool SQLite directory {}",
                parent.display()
            )
        })?;
    }
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
    )
    .with_context(|| format!("opening lifecycle spool SQLite {}", db_path.display()))?;
    conn.busy_timeout(HOOK_SAFE_SQLITE_BUSY_TIMEOUT)
        .context("setting hook-safe lifecycle SQLite busy timeout")?;
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")
        .context("configuring hook-safe lifecycle SQLite pragmas")?;
    conn.execute_batch(LIFECYCLE_SPOOL_SCHEMA_SQLITE)
        .context("initialising hook-safe lifecycle spool schema")?;
    migrate_legacy_lifecycle_stop_spool_jobs(&conn)?;
    Ok(conn)
}

fn insert_lifecycle_job(conn: &rusqlite::Connection, insert: LifecycleJobInsert) -> Result<()> {
    let now = unix_timestamp_now();
    let received_at = if insert.received_at_unix == 0 {
        now
    } else {
        insert.received_at_unix
    };
    let workspace_snapshot = insert
        .workspace_snapshot
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .context("serialising lifecycle workspace snapshot")?;
    conn.execute(
        "INSERT INTO agent_lifecycle_spool_jobs (
            job_id, repo_id, repo_root, config_root, agent_name, hook_name,
            raw_stdin, workspace_snapshot, cwd, status, attempts, available_at_unix,
            received_at_unix, updated_at_unix, last_error
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, ?10, 0, ?11,
            ?12, ?13, NULL
         )",
        params![
            format!("lifecycle-job-{}", Uuid::new_v4()),
            insert.repo_id,
            insert.repo_root.to_string_lossy().to_string(),
            insert.config_root.to_string_lossy().to_string(),
            insert.agent_name,
            insert.hook_name,
            insert.raw_stdin,
            workspace_snapshot,
            insert.cwd.to_string_lossy().to_string(),
            LifecycleJobStatus::Pending.as_str(),
            sql_i64(now)?,
            sql_i64(received_at)?,
            sql_i64(now)?,
        ],
    )
    .context("inserting lifecycle spool job")?;
    Ok(())
}

fn select_oldest_active_lifecycle_job(
    conn: &rusqlite::Connection,
) -> Result<Option<LifecycleJobRecord>> {
    conn.query_row(
        "SELECT sequence, job_id, repo_id, repo_root, config_root, agent_name, hook_name,
                raw_stdin, workspace_snapshot, cwd, status, attempts, available_at_unix,
                received_at_unix, updated_at_unix, last_error
         FROM agent_lifecycle_spool_jobs
         WHERE status != ?1
         ORDER BY sequence ASC
         LIMIT 1",
        params![LifecycleJobStatus::Failed.as_str()],
        map_lifecycle_job,
    )
    .optional()
    .context("selecting oldest active lifecycle spool job")
}

fn load_lifecycle_job(conn: &rusqlite::Connection, job_id: &str) -> Result<LifecycleJobRecord> {
    conn.query_row(
        "SELECT sequence, job_id, repo_id, repo_root, config_root, agent_name, hook_name,
                raw_stdin, workspace_snapshot, cwd, status, attempts, available_at_unix,
                received_at_unix, updated_at_unix, last_error
         FROM agent_lifecycle_spool_jobs
         WHERE job_id = ?1",
        params![job_id],
        map_lifecycle_job,
    )
    .with_context(|| format!("loading lifecycle spool job `{job_id}`"))
}

fn map_lifecycle_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<LifecycleJobRecord> {
    let workspace_snapshot = row
        .get::<_, Option<String>>("workspace_snapshot")?
        .map(parse_workspace_snapshot);
    Ok(LifecycleJobRecord {
        sequence: row_i64_as_u64(row, "sequence")?,
        job_id: row.get("job_id")?,
        repo_id: row.get("repo_id")?,
        repo_root: PathBuf::from(row.get::<_, String>("repo_root")?),
        config_root: PathBuf::from(row.get::<_, String>("config_root")?),
        agent_name: row.get("agent_name")?,
        hook_name: row.get("hook_name")?,
        raw_stdin: row.get("raw_stdin")?,
        workspace_snapshot,
        cwd: PathBuf::from(row.get::<_, String>("cwd")?),
        status: LifecycleJobStatus::parse(&row.get::<_, String>("status")?),
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

fn migrate_legacy_lifecycle_stop_spool_jobs(conn: &rusqlite::Connection) -> Result<()> {
    if !sqlite_table_exists(conn, "agent_lifecycle_stop_spool_jobs")? {
        return Ok(());
    }
    ensure_lifecycle_stop_spool_columns(conn)?;
    conn.execute(
        "INSERT OR IGNORE INTO agent_lifecycle_spool_jobs (
            job_id, repo_id, repo_root, config_root, agent_name, hook_name,
            raw_stdin, workspace_snapshot, cwd, status, attempts, available_at_unix,
            received_at_unix, updated_at_unix, last_error
         )
         SELECT job_id, repo_id, repo_root, config_root, agent_name, hook_name,
                raw_stdin, workspace_snapshot, cwd, status, attempts, available_at_unix,
                received_at_unix, updated_at_unix, last_error
         FROM agent_lifecycle_stop_spool_jobs
         ORDER BY rowid ASC",
        [],
    )
    .context("copying legacy lifecycle stop spool jobs into generic spool")?;
    Ok(())
}

fn ensure_lifecycle_stop_spool_columns(conn: &rusqlite::Connection) -> Result<()> {
    if !sqlite_table_exists(conn, "agent_lifecycle_stop_spool_jobs")? {
        return Ok(());
    }
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

fn sqlite_table_exists(conn: &rusqlite::Connection, table_name: &str) -> Result<bool> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM sqlite_master
             WHERE type = 'table' AND name = ?1",
            params![table_name],
            |row| row.get(0),
        )
        .with_context(|| format!("checking SQLite table `{table_name}` existence"))?;
    Ok(count > 0)
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
                .context("committing lifecycle spool transaction")?;
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

    fn sample_lifecycle_insert(repo_root: &Path, hook_name: &str) -> LifecycleJobInsert {
        LifecycleJobInsert {
            repo_id: "repo-1".to_string(),
            repo_root: repo_root.to_path_buf(),
            config_root: repo_root.join(".bitloops-test-state/daemon"),
            agent_name: crate::adapters::agents::AGENT_NAME_CODEX.to_string(),
            hook_name: hook_name.to_string(),
            raw_stdin: r#"{"session_id":"session-1","transcript_path":"/tmp/session.jsonl"}"#
                .to_string(),
            workspace_snapshot: Some(LifecycleStopWorkspaceSnapshot::default()),
            cwd: repo_root.to_path_buf(),
            received_at_unix: 1_778_800_000,
        }
    }

    #[test]
    fn lifecycle_spool_claims_jobs_by_insertion_sequence_when_same_second() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let sqlite = sqlite_at(&dir);
        initialise_lifecycle_spool_schema(&sqlite)?;

        enqueue_lifecycle_job_sqlite(&sqlite, sample_lifecycle_insert(dir.path(), "first-hook"))?;
        enqueue_lifecycle_job_sqlite(&sqlite, sample_lifecycle_insert(dir.path(), "second-hook"))?;

        let first = claim_next_lifecycle_job(&sqlite)?.expect("first claimed job");
        assert_eq!(first.hook_name, "first-hook");
        delete_lifecycle_job(&sqlite, &first.job_id)?;

        let second = claim_next_lifecycle_job(&sqlite)?.expect("second claimed job");
        assert_eq!(second.hook_name, "second-hook");
        Ok(())
    }

    #[test]
    fn lifecycle_spool_blocks_behind_unavailable_head_job() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let sqlite = sqlite_at(&dir);
        initialise_lifecycle_spool_schema(&sqlite)?;

        enqueue_lifecycle_job_sqlite(&sqlite, sample_lifecycle_insert(dir.path(), "retry-head"))?;
        let first = claim_next_lifecycle_job(&sqlite)?.expect("first claimed job");
        requeue_lifecycle_job(&sqlite, &first.job_id, "transient failure")?;
        enqueue_lifecycle_job_sqlite(
            &sqlite,
            sample_lifecycle_insert(dir.path(), "blocked-behind-head"),
        )?;

        assert!(claim_next_lifecycle_job(&sqlite)?.is_none());
        Ok(())
    }

    #[test]
    fn lifecycle_spool_running_head_blocks_later_jobs() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let sqlite = sqlite_at(&dir);
        initialise_lifecycle_spool_schema(&sqlite)?;

        enqueue_lifecycle_job_sqlite(&sqlite, sample_lifecycle_insert(dir.path(), "running-head"))?;
        enqueue_lifecycle_job_sqlite(
            &sqlite,
            sample_lifecycle_insert(dir.path(), "blocked-behind-running"),
        )?;

        let first = claim_next_lifecycle_job(&sqlite)?.expect("first claimed job");
        assert_eq!(first.hook_name, "running-head");
        assert!(claim_next_lifecycle_job(&sqlite)?.is_none());
        Ok(())
    }

    #[test]
    fn lifecycle_spool_marks_job_failed_after_max_attempts() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let sqlite = sqlite_at(&dir);
        initialise_lifecycle_spool_schema(&sqlite)?;
        enqueue_lifecycle_job_sqlite(&sqlite, sample_lifecycle_insert(dir.path(), "stop"))?;

        let mut job_id = None;
        for expected_attempt in 1..=MAX_LIFECYCLE_JOB_ATTEMPTS {
            if let Some(job_id) = job_id.as_deref() {
                force_pending_job_available_for_tests(&sqlite, job_id)?;
            }
            let job = claim_next_lifecycle_job(&sqlite)?.expect("claimed retry job");
            assert_eq!(job.attempts, expected_attempt);
            job_id = Some(job.job_id.clone());
            fail_or_requeue_lifecycle_job(&sqlite, &job, "still failing")?;
        }

        let rows = list_lifecycle_jobs_for_tests(&sqlite)?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, LifecycleJobStatus::Failed);
        assert_eq!(rows[0].last_error.as_deref(), Some("still failing"));
        Ok(())
    }

    #[test]
    fn lifecycle_spool_initialise_copies_legacy_stop_jobs() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let sqlite = sqlite_at(&dir);
        sqlite.execute_batch(LIFECYCLE_STOP_SPOOL_SCHEMA_SQLITE)?;
        sqlite.with_write_connection(|conn| {
            conn.execute(
                "INSERT INTO agent_lifecycle_stop_spool_jobs (
                    job_id, repo_id, repo_root, config_root, agent_name, hook_name,
                    raw_stdin, workspace_snapshot, cwd, status, attempts, available_at_unix,
                    received_at_unix, updated_at_unix, last_error
                 ) VALUES (
                    'legacy-job-1', 'repo-legacy', ?1, ?2, ?3, 'Stop',
                    '{}', ?4, ?5, 'pending', 2, 10, 9, 11, 'old error'
                 )",
                params![
                    dir.path().to_string_lossy().to_string(),
                    dir.path()
                        .join(".bitloops-test-state/daemon")
                        .to_string_lossy()
                        .to_string(),
                    crate::adapters::agents::AGENT_NAME_CODEX,
                    serde_json::to_string(&LifecycleStopWorkspaceSnapshot::default())?,
                    dir.path().to_string_lossy().to_string(),
                ],
            )?;
            Ok(())
        })?;

        initialise_lifecycle_spool_schema(&sqlite)?;

        let rows = list_lifecycle_jobs_for_tests(&sqlite)?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].job_id, "legacy-job-1");
        assert_eq!(rows[0].repo_id, "repo-legacy");
        assert_eq!(rows[0].status, LifecycleJobStatus::Pending);
        assert_eq!(rows[0].attempts, 2);
        assert_eq!(rows[0].last_error.as_deref(), Some("old error"));
        Ok(())
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
    fn lifecycle_stop_spool_compat_claim_returns_at_most_one_job() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let sqlite = sqlite_at(&dir);
        initialise_lifecycle_stop_spool_schema(&sqlite)?;

        enqueue_lifecycle_stop_job_sqlite(&sqlite, sample_insert(dir.path()))?;
        enqueue_lifecycle_stop_job_sqlite(&sqlite, sample_insert(dir.path()))?;

        let claimed = claim_next_lifecycle_stop_jobs(&sqlite, 8)?;

        assert_eq!(claimed.len(), 1);
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
