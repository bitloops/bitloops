use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::embeddings_bootstrap::{
    EmbeddingsBootstrapProgress, EmbeddingsBootstrapResult, EmbeddingsBootstrapTaskSpec,
};
use super::summary_bootstrap::{
    SummaryBootstrapProgress, SummaryBootstrapRequest, SummaryBootstrapResultRecord,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DevqlTaskKind {
    Sync,
    Ingest,
    EmbeddingsBootstrap,
    SummaryBootstrap,
}

impl fmt::Display for DevqlTaskKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sync => write!(f, "sync"),
            Self::Ingest => write!(f, "ingest"),
            Self::EmbeddingsBootstrap => write!(f, "embeddings_bootstrap"),
            Self::SummaryBootstrap => write!(f, "summary_bootstrap"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DevqlTaskSource {
    Init,
    ManualCli,
    Watcher,
    PostCommit,
    PostMerge,
    PostCheckout,
    RepoPolicyChange,
}

impl fmt::Display for DevqlTaskSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Init => write!(f, "init"),
            Self::ManualCli => write!(f, "manual_cli"),
            Self::Watcher => write!(f, "watcher"),
            Self::PostCommit => write!(f, "post_commit"),
            Self::PostMerge => write!(f, "post_merge"),
            Self::PostCheckout => write!(f, "post_checkout"),
            Self::RepoPolicyChange => write!(f, "repo_policy_change"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncTaskMode {
    Auto,
    Full,
    Paths { paths: Vec<String> },
    Repair,
    Validate,
}

impl fmt::Display for SyncTaskMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::Full => write!(f, "full"),
            Self::Paths { .. } => write!(f, "paths"),
            Self::Repair => write!(f, "repair"),
            Self::Validate => write!(f, "validate"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DevqlTaskStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl fmt::Display for DevqlTaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Queued => write!(f, "queued"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostCommitSnapshotSpec {
    pub commit_sha: String,
    pub changed_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncTaskSpec {
    pub mode: SyncTaskMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_commit_snapshot: Option<PostCommitSnapshotSpec>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IngestTaskSpec {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commits: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backfill: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum DevqlTaskSpec {
    Sync(SyncTaskSpec),
    Ingest(IngestTaskSpec),
    EmbeddingsBootstrap(EmbeddingsBootstrapTaskSpec),
    SummaryBootstrap(SummaryBootstrapRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum DevqlTaskProgress {
    Sync(crate::host::devql::SyncProgressUpdate),
    Ingest(crate::host::devql::IngestionProgressUpdate),
    EmbeddingsBootstrap(EmbeddingsBootstrapProgress),
    SummaryBootstrap(SummaryBootstrapProgress),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum DevqlTaskResult {
    Sync(Box<crate::host::devql::SyncSummary>),
    Ingest(crate::host::devql::IngestionCounters),
    EmbeddingsBootstrap(EmbeddingsBootstrapResult),
    SummaryBootstrap(SummaryBootstrapResultRecord),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DevqlTaskRecord {
    pub task_id: String,
    pub repo_id: String,
    pub repo_name: String,
    pub repo_provider: String,
    pub repo_organisation: String,
    pub repo_identity: String,
    #[serde(alias = "config_root", default)]
    pub daemon_config_root: PathBuf,
    pub repo_root: PathBuf,
    #[serde(default)]
    pub init_session_id: Option<String>,
    pub kind: DevqlTaskKind,
    pub source: DevqlTaskSource,
    pub spec: DevqlTaskSpec,
    pub status: DevqlTaskStatus,
    pub submitted_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
    pub queue_position: Option<u64>,
    pub tasks_ahead: Option<u64>,
    pub progress: DevqlTaskProgress,
    pub error: Option<String>,
    pub result: Option<DevqlTaskResult>,
}

impl DevqlTaskRecord {
    pub fn normalise_legacy_values(&mut self) {
        if self.daemon_config_root.as_os_str().is_empty() {
            self.daemon_config_root = self.repo_root.clone();
        }
    }

    pub fn sync_spec(&self) -> Option<&SyncTaskSpec> {
        match &self.spec {
            DevqlTaskSpec::Sync(spec) => Some(spec),
            DevqlTaskSpec::Ingest(_)
            | DevqlTaskSpec::EmbeddingsBootstrap(_)
            | DevqlTaskSpec::SummaryBootstrap(_) => None,
        }
    }

    pub fn ingest_spec(&self) -> Option<&IngestTaskSpec> {
        match &self.spec {
            DevqlTaskSpec::Sync(_) => None,
            DevqlTaskSpec::Ingest(spec) => Some(spec),
            DevqlTaskSpec::EmbeddingsBootstrap(_) | DevqlTaskSpec::SummaryBootstrap(_) => None,
        }
    }

    pub fn embeddings_bootstrap_spec(&self) -> Option<&EmbeddingsBootstrapTaskSpec> {
        match &self.spec {
            DevqlTaskSpec::EmbeddingsBootstrap(spec) => Some(spec),
            DevqlTaskSpec::Sync(_)
            | DevqlTaskSpec::Ingest(_)
            | DevqlTaskSpec::SummaryBootstrap(_) => None,
        }
    }

    pub fn summary_bootstrap_spec(&self) -> Option<&SummaryBootstrapRequest> {
        match &self.spec {
            DevqlTaskSpec::SummaryBootstrap(spec) => Some(spec),
            DevqlTaskSpec::Sync(_)
            | DevqlTaskSpec::Ingest(_)
            | DevqlTaskSpec::EmbeddingsBootstrap(_) => None,
        }
    }

    pub fn sync_progress(&self) -> Option<&crate::host::devql::SyncProgressUpdate> {
        match &self.progress {
            DevqlTaskProgress::Sync(progress) => Some(progress),
            DevqlTaskProgress::Ingest(_)
            | DevqlTaskProgress::EmbeddingsBootstrap(_)
            | DevqlTaskProgress::SummaryBootstrap(_) => None,
        }
    }

    pub fn ingest_progress(&self) -> Option<&crate::host::devql::IngestionProgressUpdate> {
        match &self.progress {
            DevqlTaskProgress::Sync(_)
            | DevqlTaskProgress::EmbeddingsBootstrap(_)
            | DevqlTaskProgress::SummaryBootstrap(_) => None,
            DevqlTaskProgress::Ingest(progress) => Some(progress),
        }
    }

    pub fn embeddings_bootstrap_progress(&self) -> Option<&EmbeddingsBootstrapProgress> {
        match &self.progress {
            DevqlTaskProgress::EmbeddingsBootstrap(progress) => Some(progress),
            DevqlTaskProgress::Sync(_)
            | DevqlTaskProgress::Ingest(_)
            | DevqlTaskProgress::SummaryBootstrap(_) => None,
        }
    }

    pub fn summary_bootstrap_progress(&self) -> Option<&SummaryBootstrapProgress> {
        match &self.progress {
            DevqlTaskProgress::SummaryBootstrap(progress) => Some(progress),
            DevqlTaskProgress::Sync(_)
            | DevqlTaskProgress::Ingest(_)
            | DevqlTaskProgress::EmbeddingsBootstrap(_) => None,
        }
    }

    pub fn sync_result(&self) -> Option<&crate::host::devql::SyncSummary> {
        match &self.result {
            Some(DevqlTaskResult::Sync(result)) => Some(result.as_ref()),
            _ => None,
        }
    }

    pub fn ingest_result(&self) -> Option<&crate::host::devql::IngestionCounters> {
        match &self.result {
            Some(DevqlTaskResult::Ingest(result)) => Some(result),
            _ => None,
        }
    }

    pub fn embeddings_bootstrap_result(&self) -> Option<&EmbeddingsBootstrapResult> {
        match &self.result {
            Some(DevqlTaskResult::EmbeddingsBootstrap(result)) => Some(result),
            _ => None,
        }
    }

    pub fn summary_bootstrap_result(&self) -> Option<&SummaryBootstrapResultRecord> {
        match &self.result {
            Some(DevqlTaskResult::SummaryBootstrap(result)) => Some(result),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoTaskControlState {
    pub repo_id: String,
    pub paused: bool,
    pub paused_reason: Option<String>,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevqlTaskKindCounts {
    pub kind: DevqlTaskKind,
    pub queued_tasks: u64,
    pub running_tasks: u64,
    pub failed_tasks: u64,
    pub completed_recent_tasks: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevqlTaskQueueState {
    pub version: u8,
    pub queued_tasks: u64,
    pub running_tasks: u64,
    pub failed_tasks: u64,
    pub completed_recent_tasks: u64,
    pub by_kind: Vec<DevqlTaskKindCounts>,
    pub last_action: Option<String>,
    pub last_updated_unix: u64,
}

impl Default for DevqlTaskQueueState {
    fn default() -> Self {
        Self {
            version: 1,
            queued_tasks: 0,
            running_tasks: 0,
            failed_tasks: 0,
            completed_recent_tasks: 0,
            by_kind: default_devql_task_kind_counts(),
            last_action: Some("initialized".to_string()),
            last_updated_unix: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DevqlTaskQueueStatus {
    pub state: DevqlTaskQueueState,
    pub persisted: bool,
    pub current_repo_tasks: Vec<DevqlTaskRecord>,
    pub current_repo_control: Option<RepoTaskControlState>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DevqlTaskControlResult {
    pub message: String,
    pub control: RepoTaskControlState,
}

fn default_devql_task_kind_counts() -> Vec<DevqlTaskKindCounts> {
    vec![
        DevqlTaskKindCounts {
            kind: DevqlTaskKind::Sync,
            queued_tasks: 0,
            running_tasks: 0,
            failed_tasks: 0,
            completed_recent_tasks: 0,
        },
        DevqlTaskKindCounts {
            kind: DevqlTaskKind::Ingest,
            queued_tasks: 0,
            running_tasks: 0,
            failed_tasks: 0,
            completed_recent_tasks: 0,
        },
    ]
}
