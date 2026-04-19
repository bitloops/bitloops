//! Public data types and status enums for the repo workplane store.

use serde_json::Value;
use std::path::PathBuf;

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
