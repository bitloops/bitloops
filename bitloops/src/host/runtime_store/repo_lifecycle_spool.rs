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
    boundary_snapshot TEXT,
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

#[cfg(test)]
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct LifecycleWorkspaceSnapshot {
    pub(crate) modified_files: Vec<String>,
    pub(crate) new_files: Vec<String>,
    pub(crate) deleted_files: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct LifecycleBoundarySnapshot {
    pub(crate) workspace: Option<LifecycleWorkspaceSnapshot>,
    pub(crate) pre_untracked_files: Vec<String>,
    pub(crate) transcript_offset: Option<i64>,
    pub(crate) branch_name: Option<String>,
    pub(crate) is_default_branch: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LifecycleJobInsert {
    pub(crate) repo_id: String,
    pub(crate) repo_root: PathBuf,
    pub(crate) config_root: PathBuf,
    pub(crate) agent_name: String,
    pub(crate) hook_name: String,
    pub(crate) raw_stdin: String,
    pub(crate) workspace_snapshot: Option<LifecycleWorkspaceSnapshot>,
    pub(crate) boundary_snapshot: Option<LifecycleBoundarySnapshot>,
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
    pub(crate) workspace_snapshot: Option<LifecycleWorkspaceSnapshot>,
    pub(crate) boundary_snapshot: Option<LifecycleBoundarySnapshot>,
    pub(crate) cwd: PathBuf,
    pub(crate) status: LifecycleJobStatus,
    pub(crate) attempts: u64,
    pub(crate) available_at_unix: u64,
    pub(crate) received_at_unix: u64,
    pub(crate) updated_at_unix: u64,
    pub(crate) last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct LifecycleSpoolEnqueueResult {
    pub(crate) inserted_jobs: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LifecycleHookEnqueueResult {
    pub(crate) inserted_jobs: u64,
}

pub(crate) fn initialise_lifecycle_spool_schema(sqlite: &SqliteConnectionPool) -> Result<()> {
    sqlite
        .execute_batch(LIFECYCLE_SPOOL_SCHEMA_SQLITE)
        .context("initialising lifecycle spool schema")?;
    sqlite.with_write_connection(|conn| {
        ensure_lifecycle_spool_columns(conn)?;
        migrate_legacy_lifecycle_stop_spool_jobs(conn)
    })
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

pub(crate) fn enqueue_lifecycle_job_hook_safe_at(
    db_path: &Path,
    insert: LifecycleJobInsert,
) -> Result<LifecycleHookEnqueueResult> {
    let conn = open_hook_safe_sqlite(db_path)?;
    insert_lifecycle_job(&conn, insert).context("hook-safe inserting lifecycle spool job")?;
    Ok(LifecycleHookEnqueueResult { inserted_jobs: 1 })
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

#[cfg(test)]
pub(crate) fn list_lifecycle_jobs_for_tests(
    sqlite: &SqliteConnectionPool,
) -> Result<Vec<LifecycleJobRecord>> {
    sqlite.with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT sequence, job_id, repo_id, repo_root, config_root, agent_name, hook_name,
                        raw_stdin, workspace_snapshot, boundary_snapshot, cwd, status, attempts,
                        available_at_unix, received_at_unix, updated_at_unix, last_error
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
    ensure_lifecycle_spool_columns(&conn)?;
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
    let boundary_snapshot = insert
        .boundary_snapshot
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .context("serialising lifecycle boundary snapshot")?;
    conn.execute(
        "INSERT INTO agent_lifecycle_spool_jobs (
            job_id, repo_id, repo_root, config_root, agent_name, hook_name,
            raw_stdin, workspace_snapshot, boundary_snapshot, cwd, status, attempts, available_at_unix,
            received_at_unix, updated_at_unix, last_error
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, ?10, ?11, 0, ?12,
            ?13, ?14, NULL
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
            boundary_snapshot,
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
                raw_stdin, workspace_snapshot, boundary_snapshot, cwd, status, attempts,
                available_at_unix, received_at_unix, updated_at_unix, last_error
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
                raw_stdin, workspace_snapshot, boundary_snapshot, cwd, status, attempts,
                available_at_unix, received_at_unix, updated_at_unix, last_error
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
    let boundary_snapshot = row
        .get::<_, Option<String>>("boundary_snapshot")?
        .map(parse_boundary_snapshot)
        .or_else(|| {
            workspace_snapshot
                .clone()
                .map(|workspace| LifecycleBoundarySnapshot {
                    workspace: Some(workspace),
                    ..LifecycleBoundarySnapshot::default()
                })
        });
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
        boundary_snapshot,
        cwd: PathBuf::from(row.get::<_, String>("cwd")?),
        status: LifecycleJobStatus::parse(&row.get::<_, String>("status")?),
        attempts: row_i64_as_u64(row, "attempts")?,
        available_at_unix: row_i64_as_u64(row, "available_at_unix")?,
        received_at_unix: row_i64_as_u64(row, "received_at_unix")?,
        updated_at_unix: row_i64_as_u64(row, "updated_at_unix")?,
        last_error: row.get("last_error")?,
    })
}

fn parse_workspace_snapshot(raw: String) -> LifecycleWorkspaceSnapshot {
    serde_json::from_str(&raw).unwrap_or_default()
}

fn parse_boundary_snapshot(raw: String) -> LifecycleBoundarySnapshot {
    serde_json::from_str(&raw).unwrap_or_default()
}

fn ensure_lifecycle_spool_columns(conn: &rusqlite::Connection) -> Result<()> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(agent_lifecycle_spool_jobs)")
        .context("preparing lifecycle spool table info query")?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .context("querying lifecycle spool table columns")?;
    let columns = rows
        .collect::<rusqlite::Result<HashSet<_>>>()
        .context("collecting lifecycle spool table columns")?;
    if !columns.contains("boundary_snapshot")
        && let Err(err) = conn.execute_batch(
            r#"ALTER TABLE agent_lifecycle_spool_jobs
               ADD COLUMN boundary_snapshot TEXT"#,
        )
        && !is_duplicate_column_error(&err, "boundary_snapshot")
    {
        return Err(err).context("adding lifecycle spool boundary_snapshot column");
    }
    Ok(())
}

fn is_duplicate_column_error(err: &rusqlite::Error, column_name: &str) -> bool {
    let message = err.to_string();
    message.contains("duplicate column name") && message.contains(column_name)
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
#[path = "repo_lifecycle_spool_tests.rs"]
mod tests;
