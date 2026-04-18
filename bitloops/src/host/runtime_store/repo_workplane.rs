use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;

use super::types::RepoSqliteRuntimeStore;
use crate::storage::SqliteConnectionPool;

pub(crate) const REPO_WORKPLANE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS capability_workplane_cursor_generations (
    repo_id TEXT NOT NULL,
    generation_seq INTEGER NOT NULL,
    source_task_id TEXT,
    sync_mode TEXT NOT NULL,
    active_branch TEXT,
    head_commit_sha TEXT,
    requires_full_reconcile INTEGER NOT NULL DEFAULT 0,
    created_at_unix INTEGER NOT NULL,
    PRIMARY KEY (repo_id, generation_seq)
);

CREATE INDEX IF NOT EXISTS idx_capability_workplane_cursor_generations_repo_created
ON capability_workplane_cursor_generations (repo_id, created_at_unix DESC);

CREATE TABLE IF NOT EXISTS capability_workplane_cursor_file_changes (
    repo_id TEXT NOT NULL,
    generation_seq INTEGER NOT NULL,
    path TEXT NOT NULL,
    change_kind TEXT NOT NULL,
    language TEXT,
    content_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_capability_workplane_cursor_file_changes_repo_generation
ON capability_workplane_cursor_file_changes (repo_id, generation_seq, path);

CREATE TABLE IF NOT EXISTS capability_workplane_cursor_artefact_changes (
    repo_id TEXT NOT NULL,
    generation_seq INTEGER NOT NULL,
    symbol_id TEXT NOT NULL,
    change_kind TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    path TEXT NOT NULL,
    canonical_kind TEXT,
    name TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_capability_workplane_cursor_artefact_changes_repo_generation
ON capability_workplane_cursor_artefact_changes (repo_id, generation_seq, symbol_id);

CREATE TABLE IF NOT EXISTS capability_workplane_cursor_mailboxes (
    repo_id TEXT NOT NULL,
    capability_id TEXT NOT NULL,
    mailbox_name TEXT NOT NULL,
    last_applied_generation_seq INTEGER,
    last_error TEXT,
    updated_at_unix INTEGER NOT NULL,
    PRIMARY KEY (repo_id, capability_id, mailbox_name)
);

CREATE INDEX IF NOT EXISTS idx_capability_workplane_cursor_mailboxes_repo_capability
ON capability_workplane_cursor_mailboxes (repo_id, capability_id, mailbox_name);

CREATE TABLE IF NOT EXISTS capability_workplane_mailbox_intents (
    repo_id TEXT NOT NULL,
    capability_id TEXT NOT NULL,
    mailbox_name TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 1,
    source TEXT,
    updated_at_unix INTEGER NOT NULL,
    PRIMARY KEY (repo_id, capability_id, mailbox_name)
);

CREATE INDEX IF NOT EXISTS idx_capability_workplane_mailbox_intents_repo_capability
ON capability_workplane_mailbox_intents (repo_id, capability_id, mailbox_name, active);

CREATE TABLE IF NOT EXISTS capability_workplane_cursor_runs (
    run_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    repo_root TEXT NOT NULL,
    capability_id TEXT NOT NULL,
    mailbox_name TEXT NOT NULL,
    init_session_id TEXT,
    from_generation_seq INTEGER NOT NULL,
    to_generation_seq INTEGER NOT NULL,
    reconcile_mode TEXT NOT NULL,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL,
    submitted_at_unix INTEGER NOT NULL,
    started_at_unix INTEGER,
    updated_at_unix INTEGER NOT NULL,
    completed_at_unix INTEGER,
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_capability_workplane_cursor_runs_repo_mailbox_status
ON capability_workplane_cursor_runs (repo_id, capability_id, mailbox_name, status, submitted_at_unix);

CREATE TABLE IF NOT EXISTS capability_workplane_jobs (
    job_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    repo_root TEXT NOT NULL,
    config_root TEXT NOT NULL,
    capability_id TEXT NOT NULL,
    mailbox_name TEXT NOT NULL,
    init_session_id TEXT,
    dedupe_key TEXT,
    payload TEXT NOT NULL,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at_unix INTEGER NOT NULL,
    submitted_at_unix INTEGER NOT NULL,
    started_at_unix INTEGER,
    updated_at_unix INTEGER NOT NULL,
    completed_at_unix INTEGER,
    lease_owner TEXT,
    lease_expires_at_unix INTEGER,
    last_error TEXT
);

CREATE INDEX IF NOT EXISTS idx_capability_workplane_jobs_repo_mailbox_status
ON capability_workplane_jobs (repo_id, capability_id, mailbox_name, status, available_at_unix, submitted_at_unix);

CREATE INDEX IF NOT EXISTS idx_capability_workplane_jobs_status_mailbox_available_submitted
ON capability_workplane_jobs (status, mailbox_name, available_at_unix, submitted_at_unix);

CREATE INDEX IF NOT EXISTS idx_capability_workplane_jobs_dedupe
ON capability_workplane_jobs (repo_id, capability_id, mailbox_name, dedupe_key);
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityWorkplaneJobInsert {
    pub mailbox_name: String,
    pub init_session_id: Option<String>,
    pub dedupe_key: Option<String>,
    pub payload: Value,
}

impl CapabilityWorkplaneJobInsert {
    pub fn new(
        mailbox_name: impl Into<String>,
        init_session_id: Option<String>,
        dedupe_key: Option<String>,
        payload: Value,
    ) -> Self {
        Self {
            mailbox_name: mailbox_name.into(),
            init_session_id,
            dedupe_key,
            payload,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkplaneCursorRunStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl WorkplaneCursorRunStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Queued,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkplaneJobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl WorkplaneJobStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkplaneCursorRunRecord {
    pub run_id: String,
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub capability_id: String,
    pub mailbox_name: String,
    pub init_session_id: Option<String>,
    pub from_generation_seq: u64,
    pub to_generation_seq: u64,
    pub reconcile_mode: String,
    pub status: WorkplaneCursorRunStatus,
    pub attempts: u32,
    pub submitted_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkplaneJobRecord {
    pub job_id: String,
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub config_root: PathBuf,
    pub capability_id: String,
    pub mailbox_name: String,
    pub init_session_id: Option<String>,
    pub dedupe_key: Option<String>,
    pub payload: Value,
    pub status: WorkplaneJobStatus,
    pub attempts: u32,
    pub available_at_unix: u64,
    pub submitted_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
    pub lease_owner: Option<String>,
    pub lease_expires_at_unix: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityWorkplaneMailboxStatus {
    pub pending_jobs: u64,
    pub running_jobs: u64,
    pub failed_jobs: u64,
    pub completed_recent_jobs: u64,
    pub pending_cursor_runs: u64,
    pub running_cursor_runs: u64,
    pub failed_cursor_runs: u64,
    pub completed_recent_cursor_runs: u64,
    pub intent_active: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityWorkplaneEnqueueResult {
    pub inserted_jobs: u64,
    pub updated_jobs: u64,
}

impl RepoSqliteRuntimeStore {
    pub fn set_capability_workplane_mailbox_intents<'a>(
        &self,
        capability_id: &str,
        mailbox_names: impl IntoIterator<Item = &'a str>,
        active: bool,
        source: Option<&str>,
    ) -> Result<()> {
        let sqlite = self.connect_repo_sqlite()?;
        sqlite.with_connection(|conn| {
            let now = unix_timestamp_now();
            let mut stmt = conn.prepare(
                "INSERT INTO capability_workplane_mailbox_intents (
                    repo_id, capability_id, mailbox_name, active, source, updated_at_unix
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (repo_id, capability_id, mailbox_name)
                 DO UPDATE SET
                    active = excluded.active,
                    source = excluded.source,
                    updated_at_unix = excluded.updated_at_unix",
            )?;
            for mailbox_name in mailbox_names {
                stmt.execute(params![
                    &self.repo_id,
                    capability_id,
                    mailbox_name,
                    if active { 1 } else { 0 },
                    source,
                    sql_i64(now)?,
                ])
                .with_context(|| {
                    format!(
                        "upserting capability workplane mailbox intent `{mailbox_name}` for repo `{}`",
                        self.repo_id
                    )
                })?;
            }
            Ok(())
        })
    }

    pub fn enqueue_capability_workplane_jobs(
        &self,
        capability_id: &str,
        jobs: Vec<CapabilityWorkplaneJobInsert>,
    ) -> Result<CapabilityWorkplaneEnqueueResult> {
        if jobs.is_empty() {
            return Ok(CapabilityWorkplaneEnqueueResult::default());
        }

        let sqlite = self.connect_repo_sqlite()?;
        sqlite.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
                .context("starting capability workplane enqueue transaction")?;
            let result = (|| {
                let now = unix_timestamp_now();
                let mut inserted_jobs = 0u64;
                let mut updated_jobs = 0u64;
                for job in jobs {
                    if let Some(existing) = load_deduped_job(
                        conn,
                        &self.repo_id,
                        capability_id,
                        &job.mailbox_name,
                        job.init_session_id.as_deref(),
                        job.dedupe_key.as_deref(),
                    )? {
                        if existing.status == WorkplaneJobStatus::Pending {
                            conn.execute(
                                "UPDATE capability_workplane_jobs
                                 SET payload = ?1, updated_at_unix = ?2, available_at_unix = ?3, last_error = NULL
                                 WHERE job_id = ?4",
                                params![
                                    job.payload.to_string(),
                                    sql_i64(now)?,
                                    sql_i64(now)?,
                                    existing.job_id,
                                ],
                            )
                            .with_context(|| {
                                format!(
                                    "refreshing pending capability workplane job `{}`",
                                    existing.job_id
                                )
                            })?;
                        }
                        updated_jobs += 1;
                        continue;
                    }

                    let job_id = format!("workplane-job-{}", Uuid::new_v4());
                    conn.execute(
                        "INSERT INTO capability_workplane_jobs (
                            job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                            init_session_id, dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                            started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                            lease_expires_at_unix, last_error
                         ) VALUES (
                            ?1, ?2, ?3, ?4, ?5, ?6,
                            ?7, ?8, ?9, ?10, 0, ?11, ?12,
                            NULL, ?13, NULL, NULL,
                            NULL, NULL
                         )",
                        params![
                            &job_id,
                            &self.repo_id,
                            self.repo_root.to_string_lossy().to_string(),
                            self.config_root.to_string_lossy().to_string(),
                            capability_id,
                            &job.mailbox_name,
                            job.init_session_id.as_deref(),
                            job.dedupe_key.as_deref(),
                            job.payload.to_string(),
                            WorkplaneJobStatus::Pending.as_str(),
                            sql_i64(now)?,
                            sql_i64(now)?,
                            sql_i64(now)?,
                        ],
                    )
                    .with_context(|| {
                        format!(
                            "inserting capability workplane job `{job_id}` for mailbox `{}`",
                            job.mailbox_name
                        )
                    })?;
                    inserted_jobs += 1;
                }
                Ok(CapabilityWorkplaneEnqueueResult {
                    inserted_jobs,
                    updated_jobs,
                })
            })();

            match result {
                Ok(result) => {
                    conn.execute_batch("COMMIT;")
                        .context("committing capability workplane enqueue transaction")?;
                    Ok(result)
                }
                Err(err) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(err)
                }
            }
        })
    }

    pub fn load_capability_workplane_mailbox_status<'a>(
        &self,
        capability_id: &str,
        mailbox_names: impl IntoIterator<Item = &'a str>,
    ) -> Result<BTreeMap<String, crate::host::capability_host::gateways::CapabilityMailboxStatus>>
    {
        let sqlite = self.connect_repo_sqlite()?;
        sqlite.with_connection(|conn| {
            let mut status_by_mailbox = mailbox_names
                .into_iter()
                .map(|mailbox_name| {
                    (
                        mailbox_name.to_string(),
                        crate::host::capability_host::gateways::CapabilityMailboxStatus::default(),
                    )
                })
                .collect::<BTreeMap<_, _>>();

            {
                let mut stmt = conn.prepare(
                    "SELECT mailbox_name, active
                     FROM capability_workplane_mailbox_intents
                     WHERE repo_id = ?1 AND capability_id = ?2",
                )?;
                let rows = stmt.query_map(params![&self.repo_id, capability_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })?;
                for row in rows {
                    let (mailbox_name, active) = row?;
                    let Some(entry) = status_by_mailbox.get_mut(&mailbox_name) else {
                        continue;
                    };
                    entry.intent_active = active != 0;
                }
            }

            {
                let mut stmt = conn.prepare(
                    "SELECT mailbox_name, status, COUNT(*)
                     FROM capability_workplane_jobs
                     WHERE repo_id = ?1 AND capability_id = ?2
                     GROUP BY mailbox_name, status",
                )?;
                let rows = stmt.query_map(params![&self.repo_id, capability_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })?;
                for row in rows {
                    let (mailbox_name, status, count) = row?;
                    let Some(entry) = status_by_mailbox.get_mut(&mailbox_name) else {
                        continue;
                    };
                    let count = u64::try_from(count).unwrap_or_default();
                    match WorkplaneJobStatus::parse(&status) {
                        WorkplaneJobStatus::Pending => entry.pending_jobs += count,
                        WorkplaneJobStatus::Running => entry.running_jobs += count,
                        WorkplaneJobStatus::Completed => entry.completed_recent_jobs += count,
                        WorkplaneJobStatus::Failed => entry.failed_jobs += count,
                    }
                }
            }

            {
                let mut stmt = conn.prepare(
                    "SELECT mailbox_name, status, COUNT(*)
                     FROM capability_workplane_cursor_runs
                     WHERE repo_id = ?1 AND capability_id = ?2
                     GROUP BY mailbox_name, status",
                )?;
                let rows = stmt.query_map(params![&self.repo_id, capability_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })?;
                for row in rows {
                    let (mailbox_name, status, count) = row?;
                    let Some(entry) = status_by_mailbox.get_mut(&mailbox_name) else {
                        continue;
                    };
                    let count = u64::try_from(count).unwrap_or_default();
                    match WorkplaneCursorRunStatus::parse(&status) {
                        WorkplaneCursorRunStatus::Queued => entry.pending_cursor_runs += count,
                        WorkplaneCursorRunStatus::Running => entry.running_cursor_runs += count,
                        WorkplaneCursorRunStatus::Completed => {
                            entry.completed_recent_cursor_runs += count
                        }
                        WorkplaneCursorRunStatus::Failed => entry.failed_cursor_runs += count,
                        WorkplaneCursorRunStatus::Cancelled => {}
                    }
                }
            }

            Ok(status_by_mailbox)
        })
    }
}

fn load_deduped_job(
    conn: &rusqlite::Connection,
    repo_id: &str,
    capability_id: &str,
    mailbox_name: &str,
    init_session_id: Option<&str>,
    dedupe_key: Option<&str>,
) -> Result<Option<WorkplaneJobRecord>> {
    let Some(dedupe_key) = dedupe_key else {
        return Ok(None);
    };
    if let Some(init_session_id) = init_session_id {
        return conn
            .query_row(
                "SELECT job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                        init_session_id, dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                        started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                        lease_expires_at_unix, last_error
                 FROM capability_workplane_jobs
                 WHERE repo_id = ?1
                   AND capability_id = ?2
                   AND mailbox_name = ?3
                   AND init_session_id = ?4
                   AND dedupe_key = ?5
                   AND status IN (?6, ?7)
                 ORDER BY CASE status WHEN 'running' THEN 0 ELSE 1 END, submitted_at_unix ASC
                 LIMIT 1",
                params![
                    repo_id,
                    capability_id,
                    mailbox_name,
                    init_session_id,
                    dedupe_key,
                    WorkplaneJobStatus::Running.as_str(),
                    WorkplaneJobStatus::Pending.as_str(),
                ],
                map_workplane_job_record_row,
            )
            .optional()
            .map_err(anyhow::Error::from);
    }

    conn.query_row(
        "SELECT job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                init_session_id, dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                lease_expires_at_unix, last_error
         FROM capability_workplane_jobs
         WHERE repo_id = ?1
           AND capability_id = ?2
           AND mailbox_name = ?3
           AND init_session_id IS NULL
           AND dedupe_key = ?4
           AND status IN (?5, ?6)
         ORDER BY CASE status WHEN 'running' THEN 0 ELSE 1 END, submitted_at_unix ASC
         LIMIT 1",
        params![
            repo_id,
            capability_id,
            mailbox_name,
            dedupe_key,
            WorkplaneJobStatus::Running.as_str(),
            WorkplaneJobStatus::Pending.as_str(),
        ],
        map_workplane_job_record_row,
    )
    .optional()
    .map_err(anyhow::Error::from)
}

fn map_workplane_job_record_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkplaneJobRecord> {
    let payload_raw = row.get::<_, String>(8)?;
    let payload = serde_json::from_str(&payload_raw).unwrap_or(Value::Null);
    Ok(WorkplaneJobRecord {
        job_id: row.get(0)?,
        repo_id: row.get(1)?,
        repo_root: PathBuf::from(row.get::<_, String>(2)?),
        config_root: PathBuf::from(row.get::<_, String>(3)?),
        capability_id: row.get(4)?,
        mailbox_name: row.get(5)?,
        init_session_id: row.get(6)?,
        dedupe_key: row.get(7)?,
        payload,
        status: WorkplaneJobStatus::parse(&row.get::<_, String>(9)?),
        attempts: row.get(10)?,
        available_at_unix: parse_u64(row.get::<_, i64>(11)?),
        submitted_at_unix: parse_u64(row.get::<_, i64>(12)?),
        started_at_unix: row.get::<_, Option<i64>>(13)?.map(parse_u64),
        updated_at_unix: parse_u64(row.get::<_, i64>(14)?),
        completed_at_unix: row.get::<_, Option<i64>>(15)?.map(parse_u64),
        lease_owner: row.get(16)?,
        lease_expires_at_unix: row.get::<_, Option<i64>>(17)?.map(parse_u64),
        last_error: row.get(18)?,
    })
}

fn parse_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

fn sql_i64(value: u64) -> Result<i64> {
    i64::try_from(value).context("converting runtime workplane integer to sqlite i64")
}

fn unix_timestamp_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub(crate) fn ensure_repo_workplane_schema_upgrades(sqlite: &SqliteConnectionPool) -> Result<()> {
    sqlite.with_connection(|conn| {
        ensure_table_has_column(
            conn,
            "capability_workplane_cursor_runs",
            "init_session_id",
            "ALTER TABLE capability_workplane_cursor_runs ADD COLUMN init_session_id TEXT",
        )?;
        ensure_table_has_column(
            conn,
            "capability_workplane_jobs",
            "init_session_id",
            "ALTER TABLE capability_workplane_jobs ADD COLUMN init_session_id TEXT",
        )?;
        Ok(())
    })
}

fn ensure_table_has_column(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
    alter_sql: &str,
) -> Result<()> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .with_context(|| format!("preparing PRAGMA table_info for `{table}`"))?;
    let mut rows = stmt
        .query([])
        .with_context(|| format!("querying PRAGMA table_info for `{table}`"))?;
    while let Some(row) = rows.next().context("reading PRAGMA row")? {
        let name: String = row
            .get(1)
            .with_context(|| format!("reading column name from `{table}`"))?;
        if name == column {
            return Ok(());
        }
    }
    conn.execute_batch(alter_sql)
        .with_context(|| format!("adding `{column}` column to `{table}`"))?;
    Ok(())
}
