use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;

use super::types::RepoSqliteRuntimeStore;
use crate::storage::SqliteConnectionPool;

const SEMANTIC_SUMMARY_REFRESH_MAILBOX_NAME: &str = "semantic_clones.summary_refresh";
const SEMANTIC_CODE_EMBEDDING_MAILBOX_NAME: &str = "semantic_clones.embedding.code";
const SEMANTIC_SUMMARY_EMBEDDING_MAILBOX_NAME: &str = "semantic_clones.embedding.summary";

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

CREATE TABLE IF NOT EXISTS semantic_summary_mailbox_items (
    item_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    repo_root TEXT NOT NULL,
    config_root TEXT NOT NULL,
    init_session_id TEXT,
    item_kind TEXT NOT NULL,
    artefact_id TEXT,
    payload_json TEXT,
    dedupe_key TEXT,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at_unix INTEGER NOT NULL,
    submitted_at_unix INTEGER NOT NULL,
    leased_at_unix INTEGER,
    lease_expires_at_unix INTEGER,
    lease_token TEXT,
    updated_at_unix INTEGER NOT NULL,
    last_error TEXT
);

CREATE INDEX IF NOT EXISTS idx_semantic_summary_mailbox_items_repo_status
ON semantic_summary_mailbox_items (repo_id, status, available_at_unix, submitted_at_unix);

CREATE INDEX IF NOT EXISTS idx_semantic_summary_mailbox_items_status_available
ON semantic_summary_mailbox_items (status, available_at_unix, submitted_at_unix);

CREATE INDEX IF NOT EXISTS idx_semantic_summary_mailbox_items_dedupe
ON semantic_summary_mailbox_items (repo_id, dedupe_key);

CREATE TABLE IF NOT EXISTS semantic_embedding_mailbox_items (
    item_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    repo_root TEXT NOT NULL,
    config_root TEXT NOT NULL,
    init_session_id TEXT,
    representation_kind TEXT NOT NULL,
    item_kind TEXT NOT NULL,
    artefact_id TEXT,
    payload_json TEXT,
    dedupe_key TEXT,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at_unix INTEGER NOT NULL,
    submitted_at_unix INTEGER NOT NULL,
    leased_at_unix INTEGER,
    lease_expires_at_unix INTEGER,
    lease_token TEXT,
    updated_at_unix INTEGER NOT NULL,
    last_error TEXT
);

CREATE INDEX IF NOT EXISTS idx_semantic_embedding_mailbox_items_repo_status
ON semantic_embedding_mailbox_items (repo_id, representation_kind, status, available_at_unix, submitted_at_unix);

CREATE INDEX IF NOT EXISTS idx_semantic_embedding_mailbox_items_status_available
ON semantic_embedding_mailbox_items (status, representation_kind, available_at_unix, submitted_at_unix);

CREATE INDEX IF NOT EXISTS idx_semantic_embedding_mailbox_items_dedupe
ON semantic_embedding_mailbox_items (repo_id, representation_kind, dedupe_key);
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityWorkplaneJobInsert {
    pub mailbox_name: String,
    pub init_session_id: Option<String>,
    pub dedupe_key: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticMailboxItemKind {
    Artefact,
    RepoBackfill,
}

impl SemanticMailboxItemKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Artefact => "artefact",
            Self::RepoBackfill => "repo_backfill",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "repo_backfill" => Self::RepoBackfill,
            _ => Self::Artefact,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticMailboxItemStatus {
    Pending,
    Leased,
    Failed,
}

impl SemanticMailboxItemStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Leased => "leased",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "leased" => Self::Leased,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticSummaryMailboxItemInsert {
    pub init_session_id: Option<String>,
    pub item_kind: SemanticMailboxItemKind,
    pub artefact_id: Option<String>,
    pub payload_json: Option<Value>,
    pub dedupe_key: Option<String>,
}

impl SemanticSummaryMailboxItemInsert {
    pub fn new(
        init_session_id: Option<String>,
        item_kind: SemanticMailboxItemKind,
        artefact_id: Option<String>,
        payload_json: Option<Value>,
        dedupe_key: Option<String>,
    ) -> Self {
        Self {
            init_session_id,
            item_kind,
            artefact_id,
            payload_json,
            dedupe_key,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticEmbeddingMailboxItemInsert {
    pub init_session_id: Option<String>,
    pub representation_kind: String,
    pub item_kind: SemanticMailboxItemKind,
    pub artefact_id: Option<String>,
    pub payload_json: Option<Value>,
    pub dedupe_key: Option<String>,
}

impl SemanticEmbeddingMailboxItemInsert {
    pub fn new(
        init_session_id: Option<String>,
        representation_kind: impl Into<String>,
        item_kind: SemanticMailboxItemKind,
        artefact_id: Option<String>,
        payload_json: Option<Value>,
        dedupe_key: Option<String>,
    ) -> Self {
        Self {
            init_session_id,
            representation_kind: representation_kind.into(),
            item_kind,
            artefact_id,
            payload_json,
            dedupe_key,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticSummaryMailboxItemRecord {
    pub item_id: String,
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub config_root: PathBuf,
    pub init_session_id: Option<String>,
    pub item_kind: SemanticMailboxItemKind,
    pub artefact_id: Option<String>,
    pub payload_json: Option<Value>,
    pub dedupe_key: Option<String>,
    pub status: SemanticMailboxItemStatus,
    pub attempts: u32,
    pub available_at_unix: u64,
    pub submitted_at_unix: u64,
    pub leased_at_unix: Option<u64>,
    pub lease_expires_at_unix: Option<u64>,
    pub lease_token: Option<String>,
    pub updated_at_unix: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticEmbeddingMailboxItemRecord {
    pub item_id: String,
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub config_root: PathBuf,
    pub init_session_id: Option<String>,
    pub representation_kind: String,
    pub item_kind: SemanticMailboxItemKind,
    pub artefact_id: Option<String>,
    pub payload_json: Option<Value>,
    pub dedupe_key: Option<String>,
    pub status: SemanticMailboxItemStatus,
    pub attempts: u32,
    pub available_at_unix: u64,
    pub submitted_at_unix: u64,
    pub leased_at_unix: Option<u64>,
    pub lease_expires_at_unix: Option<u64>,
    pub lease_token: Option<String>,
    pub updated_at_unix: u64,
    pub last_error: Option<String>,
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

    pub fn enqueue_semantic_summary_mailbox_items(
        &self,
        items: Vec<SemanticSummaryMailboxItemInsert>,
    ) -> Result<CapabilityWorkplaneEnqueueResult> {
        if items.is_empty() {
            return Ok(CapabilityWorkplaneEnqueueResult::default());
        }

        let sqlite = self.connect_repo_sqlite()?;
        sqlite.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
                .context("starting semantic summary mailbox enqueue transaction")?;
            let result = (|| {
                let now = unix_timestamp_now();
                let mut inserted_jobs = 0u64;
                let mut updated_jobs = 0u64;
                for item in items {
                    if let Some(existing) = load_deduped_summary_mailbox_item(
                        conn,
                        &self.repo_id,
                        item.dedupe_key.as_deref(),
                    )? {
                        if existing.status == SemanticMailboxItemStatus::Pending {
                            conn.execute(
                                "UPDATE semantic_summary_mailbox_items
                                 SET init_session_id = COALESCE(init_session_id, ?1),
                                     payload_json = ?2,
                                     updated_at_unix = ?3,
                                     available_at_unix = ?4,
                                     last_error = NULL
                                 WHERE item_id = ?5",
                                params![
                                    item.init_session_id.as_deref(),
                                    item.payload_json.as_ref().map(Value::to_string),
                                    sql_i64(now)?,
                                    sql_i64(now)?,
                                    existing.item_id,
                                ],
                            )
                            .with_context(|| {
                                format!(
                                    "refreshing pending semantic summary mailbox item `{}`",
                                    existing.item_id
                                )
                            })?;
                        }
                        updated_jobs += 1;
                        continue;
                    }

                    let item_id = format!("semantic-summary-mailbox-item-{}", Uuid::new_v4());
                    conn.execute(
                        "INSERT INTO semantic_summary_mailbox_items (
                            item_id, repo_id, repo_root, config_root, init_session_id, item_kind,
                            artefact_id, payload_json, dedupe_key, status, attempts,
                            available_at_unix, submitted_at_unix, leased_at_unix,
                            lease_expires_at_unix, lease_token, updated_at_unix, last_error
                         ) VALUES (
                            ?1, ?2, ?3, ?4, ?5, ?6,
                            ?7, ?8, ?9, ?10, 0,
                            ?11, ?12, NULL,
                            NULL, NULL, ?13, NULL
                         )",
                        params![
                            &item_id,
                            &self.repo_id,
                            self.repo_root.to_string_lossy().to_string(),
                            self.config_root.to_string_lossy().to_string(),
                            item.init_session_id.as_deref(),
                            item.item_kind.as_str(),
                            item.artefact_id.as_deref(),
                            item.payload_json.as_ref().map(Value::to_string),
                            item.dedupe_key.as_deref(),
                            SemanticMailboxItemStatus::Pending.as_str(),
                            sql_i64(now)?,
                            sql_i64(now)?,
                            sql_i64(now)?,
                        ],
                    )
                    .with_context(|| {
                        format!("inserting semantic summary mailbox item `{item_id}`")
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
                        .context("committing semantic summary mailbox enqueue transaction")?;
                    Ok(result)
                }
                Err(err) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(err)
                }
            }
        })
    }

    pub fn enqueue_semantic_embedding_mailbox_items(
        &self,
        items: Vec<SemanticEmbeddingMailboxItemInsert>,
    ) -> Result<CapabilityWorkplaneEnqueueResult> {
        if items.is_empty() {
            return Ok(CapabilityWorkplaneEnqueueResult::default());
        }

        let sqlite = self.connect_repo_sqlite()?;
        sqlite.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
                .context("starting semantic embedding mailbox enqueue transaction")?;
            let result = (|| {
                let now = unix_timestamp_now();
                let mut inserted_jobs = 0u64;
                let mut updated_jobs = 0u64;
                for item in items {
                    if let Some(existing) = load_deduped_embedding_mailbox_item(
                        conn,
                        &self.repo_id,
                        &item.representation_kind,
                        item.dedupe_key.as_deref(),
                    )? {
                        if existing.status == SemanticMailboxItemStatus::Pending {
                            conn.execute(
                                "UPDATE semantic_embedding_mailbox_items
                                 SET init_session_id = COALESCE(init_session_id, ?1),
                                     payload_json = ?2,
                                     updated_at_unix = ?3,
                                     available_at_unix = ?4,
                                     last_error = NULL
                                 WHERE item_id = ?5",
                                params![
                                    item.init_session_id.as_deref(),
                                    item.payload_json.as_ref().map(Value::to_string),
                                    sql_i64(now)?,
                                    sql_i64(now)?,
                                    existing.item_id,
                                ],
                            )
                            .with_context(|| {
                                format!(
                                    "refreshing pending semantic embedding mailbox item `{}`",
                                    existing.item_id
                                )
                            })?;
                        }
                        updated_jobs += 1;
                        continue;
                    }

                    let item_id = format!("semantic-embedding-mailbox-item-{}", Uuid::new_v4());
                    conn.execute(
                        "INSERT INTO semantic_embedding_mailbox_items (
                            item_id, repo_id, repo_root, config_root, init_session_id,
                            representation_kind, item_kind, artefact_id, payload_json,
                            dedupe_key, status, attempts, available_at_unix,
                            submitted_at_unix, leased_at_unix, lease_expires_at_unix,
                            lease_token, updated_at_unix, last_error
                         ) VALUES (
                            ?1, ?2, ?3, ?4, ?5,
                            ?6, ?7, ?8, ?9,
                            ?10, ?11, 0, ?12,
                            ?13, NULL, NULL,
                            NULL, ?14, NULL
                         )",
                        params![
                            &item_id,
                            &self.repo_id,
                            self.repo_root.to_string_lossy().to_string(),
                            self.config_root.to_string_lossy().to_string(),
                            item.init_session_id.as_deref(),
                            &item.representation_kind,
                            item.item_kind.as_str(),
                            item.artefact_id.as_deref(),
                            item.payload_json.as_ref().map(Value::to_string),
                            item.dedupe_key.as_deref(),
                            SemanticMailboxItemStatus::Pending.as_str(),
                            sql_i64(now)?,
                            sql_i64(now)?,
                            sql_i64(now)?,
                        ],
                    )
                    .with_context(|| {
                        format!("inserting semantic embedding mailbox item `{item_id}`")
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
                        .context("committing semantic embedding mailbox enqueue transaction")?;
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
                    "SELECT status, COUNT(*)
                     FROM semantic_summary_mailbox_items
                     WHERE repo_id = ?1
                     GROUP BY status",
                )?;
                let rows = stmt.query_map(params![&self.repo_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })?;
                for row in rows {
                    let (status, count) = row?;
                    let Some(entry) =
                        status_by_mailbox.get_mut(SEMANTIC_SUMMARY_REFRESH_MAILBOX_NAME)
                    else {
                        continue;
                    };
                    let count = u64::try_from(count).unwrap_or_default();
                    match SemanticMailboxItemStatus::parse(&status) {
                        SemanticMailboxItemStatus::Pending => entry.pending_jobs += count,
                        SemanticMailboxItemStatus::Leased => entry.running_jobs += count,
                        SemanticMailboxItemStatus::Failed => entry.failed_jobs += count,
                    }
                }
            }

            {
                let mut stmt = conn.prepare(
                    "SELECT representation_kind, status, COUNT(*)
                     FROM semantic_embedding_mailbox_items
                     WHERE repo_id = ?1
                     GROUP BY representation_kind, status",
                )?;
                let rows = stmt.query_map(params![&self.repo_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })?;
                for row in rows {
                    let (representation_kind, status, count) = row?;
                    let mailbox_name = match representation_kind.as_str() {
                        "summary" => SEMANTIC_SUMMARY_EMBEDDING_MAILBOX_NAME,
                        _ => SEMANTIC_CODE_EMBEDDING_MAILBOX_NAME,
                    };
                    let Some(entry) = status_by_mailbox.get_mut(mailbox_name) else {
                        continue;
                    };
                    let count = u64::try_from(count).unwrap_or_default();
                    match SemanticMailboxItemStatus::parse(&status) {
                        SemanticMailboxItemStatus::Pending => entry.pending_jobs += count,
                        SemanticMailboxItemStatus::Leased => entry.running_jobs += count,
                        SemanticMailboxItemStatus::Failed => entry.failed_jobs += count,
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

fn load_deduped_summary_mailbox_item(
    conn: &rusqlite::Connection,
    repo_id: &str,
    dedupe_key: Option<&str>,
) -> Result<Option<SemanticSummaryMailboxItemRecord>> {
    let Some(dedupe_key) = dedupe_key else {
        return Ok(None);
    };
    conn.query_row(
        "SELECT item_id, repo_id, repo_root, config_root, init_session_id, item_kind,
                artefact_id, payload_json, dedupe_key, status, attempts, available_at_unix,
                submitted_at_unix, leased_at_unix, lease_expires_at_unix, lease_token,
                updated_at_unix, last_error
         FROM semantic_summary_mailbox_items
         WHERE repo_id = ?1
           AND dedupe_key = ?2
           AND status IN (?3, ?4, ?5)
         ORDER BY CASE status
                      WHEN 'leased' THEN 0
                      WHEN 'pending' THEN 1
                      ELSE 2
                  END,
                  submitted_at_unix ASC
         LIMIT 1",
        params![
            repo_id,
            dedupe_key,
            SemanticMailboxItemStatus::Leased.as_str(),
            SemanticMailboxItemStatus::Pending.as_str(),
            SemanticMailboxItemStatus::Failed.as_str(),
        ],
        map_summary_mailbox_item_record_row,
    )
    .optional()
    .map_err(anyhow::Error::from)
}

fn load_deduped_embedding_mailbox_item(
    conn: &rusqlite::Connection,
    repo_id: &str,
    representation_kind: &str,
    dedupe_key: Option<&str>,
) -> Result<Option<SemanticEmbeddingMailboxItemRecord>> {
    let Some(dedupe_key) = dedupe_key else {
        return Ok(None);
    };
    conn.query_row(
        "SELECT item_id, repo_id, repo_root, config_root, init_session_id, representation_kind,
                item_kind, artefact_id, payload_json, dedupe_key, status, attempts,
                available_at_unix, submitted_at_unix, leased_at_unix, lease_expires_at_unix,
                lease_token, updated_at_unix, last_error
         FROM semantic_embedding_mailbox_items
         WHERE repo_id = ?1
           AND representation_kind = ?2
           AND dedupe_key = ?3
           AND status IN (?4, ?5, ?6)
         ORDER BY CASE status
                      WHEN 'leased' THEN 0
                      WHEN 'pending' THEN 1
                      ELSE 2
                  END,
                  submitted_at_unix ASC
         LIMIT 1",
        params![
            repo_id,
            representation_kind,
            dedupe_key,
            SemanticMailboxItemStatus::Leased.as_str(),
            SemanticMailboxItemStatus::Pending.as_str(),
            SemanticMailboxItemStatus::Failed.as_str(),
        ],
        map_embedding_mailbox_item_record_row,
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

fn map_summary_mailbox_item_record_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<SemanticSummaryMailboxItemRecord> {
    let payload_json = row
        .get::<_, Option<String>>(7)?
        .and_then(|raw| serde_json::from_str(&raw).ok());
    Ok(SemanticSummaryMailboxItemRecord {
        item_id: row.get(0)?,
        repo_id: row.get(1)?,
        repo_root: PathBuf::from(row.get::<_, String>(2)?),
        config_root: PathBuf::from(row.get::<_, String>(3)?),
        init_session_id: row.get(4)?,
        item_kind: SemanticMailboxItemKind::parse(&row.get::<_, String>(5)?),
        artefact_id: row.get(6)?,
        payload_json,
        dedupe_key: row.get(8)?,
        status: SemanticMailboxItemStatus::parse(&row.get::<_, String>(9)?),
        attempts: row.get(10)?,
        available_at_unix: parse_u64(row.get::<_, i64>(11)?),
        submitted_at_unix: parse_u64(row.get::<_, i64>(12)?),
        leased_at_unix: row.get::<_, Option<i64>>(13)?.map(parse_u64),
        lease_expires_at_unix: row.get::<_, Option<i64>>(14)?.map(parse_u64),
        lease_token: row.get(15)?,
        updated_at_unix: parse_u64(row.get::<_, i64>(16)?),
        last_error: row.get(17)?,
    })
}

fn map_embedding_mailbox_item_record_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<SemanticEmbeddingMailboxItemRecord> {
    let payload_json = row
        .get::<_, Option<String>>(8)?
        .and_then(|raw| serde_json::from_str(&raw).ok());
    Ok(SemanticEmbeddingMailboxItemRecord {
        item_id: row.get(0)?,
        repo_id: row.get(1)?,
        repo_root: PathBuf::from(row.get::<_, String>(2)?),
        config_root: PathBuf::from(row.get::<_, String>(3)?),
        init_session_id: row.get(4)?,
        representation_kind: row.get(5)?,
        item_kind: SemanticMailboxItemKind::parse(&row.get::<_, String>(6)?),
        artefact_id: row.get(7)?,
        payload_json,
        dedupe_key: row.get(9)?,
        status: SemanticMailboxItemStatus::parse(&row.get::<_, String>(10)?),
        attempts: row.get(11)?,
        available_at_unix: parse_u64(row.get::<_, i64>(12)?),
        submitted_at_unix: parse_u64(row.get::<_, i64>(13)?),
        leased_at_unix: row.get::<_, Option<i64>>(14)?.map(parse_u64),
        lease_expires_at_unix: row.get::<_, Option<i64>>(15)?.map(parse_u64),
        lease_token: row.get(16)?,
        updated_at_unix: parse_u64(row.get::<_, i64>(17)?),
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
