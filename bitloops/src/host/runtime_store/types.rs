use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::host::checkpoints::transcript::metadata::SessionMetadataBundle;

pub trait RuntimeStore: Send + Sync {
    type RepoStore;
    type DaemonStore;

    fn repo_store(&self, repo_root: &std::path::Path) -> Result<Self::RepoStore>;
    fn daemon_store(&self) -> Result<Self::DaemonStore>;
}

#[derive(Debug, Clone, Default)]
pub struct SqliteRuntimeStore;

#[derive(Debug, Clone)]
pub struct RepoSqliteRuntimeStore {
    pub(crate) config_root: PathBuf,
    pub(crate) repo_root: PathBuf,
    pub(crate) repo_id: String,
    pub(crate) db_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoWatcherRegistration {
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub pid: u32,
    pub restart_token: String,
    pub state: RepoWatcherRegistrationState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepoWatcherRegistrationState {
    Pending,
    Ready,
}

impl RepoWatcherRegistrationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "ready" => Some(Self::Ready),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DaemonSqliteRuntimeStore {
    pub(crate) db_path: PathBuf,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PersistedSyncQueueState {
    pub version: u8,
    pub tasks: Vec<LegacySyncTaskRecord>,
    pub last_action: Option<String>,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacySyncTaskRecord {
    pub task_id: String,
    pub repo_id: String,
    pub repo_name: String,
    pub repo_provider: String,
    pub repo_organisation: String,
    pub repo_identity: String,
    #[serde(alias = "config_root", default)]
    pub daemon_config_root: PathBuf,
    pub repo_root: PathBuf,
    pub source: crate::daemon::DevqlTaskSource,
    pub mode: crate::daemon::SyncTaskMode,
    pub status: crate::daemon::DevqlTaskStatus,
    pub submitted_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
    pub queue_position: Option<u64>,
    pub tasks_ahead: Option<u64>,
    pub progress: crate::host::devql::SyncProgressUpdate,
    pub error: Option<String>,
    pub summary: Option<crate::host::devql::SyncSummary>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PersistedDevqlTaskQueueState {
    pub version: u8,
    pub tasks: Vec<crate::daemon::DevqlTaskRecord>,
    pub repo_controls: BTreeMap<String, crate::daemon::RepoTaskControlState>,
    pub last_action: Option<String>,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PersistedCapabilityEventQueueState {
    pub version: u8,
    pub runs: Vec<crate::daemon::CapabilityEventRunRecord>,
    pub last_action: Option<String>,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeMetadataBlobType {
    Transcript,
    Prompts,
    Summary,
    Context,
    TaskCheckpoint,
    SubagentTranscript,
    IncrementalCheckpoint,
    Prompt,
}

impl RuntimeMetadataBlobType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Transcript => "transcript",
            Self::Prompts => "prompts",
            Self::Summary => "summary",
            Self::Context => "context",
            Self::TaskCheckpoint => "task_checkpoint",
            Self::SubagentTranscript => "subagent_transcript",
            Self::IncrementalCheckpoint => "incremental_checkpoint",
            Self::Prompt => "prompt",
        }
    }

    pub const fn default_file_name(self) -> &'static str {
        match self {
            Self::Transcript => "full.jsonl",
            Self::Prompts => "prompt.txt",
            Self::Summary => "summary.txt",
            Self::Context => "context.md",
            Self::TaskCheckpoint => "checkpoint.json",
            Self::SubagentTranscript => "agent.jsonl",
            Self::IncrementalCheckpoint => "incremental-checkpoint.json",
            Self::Prompt => "prompt.txt",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        match value {
            "transcript" => Some(Self::Transcript),
            "prompts" => Some(Self::Prompts),
            "summary" => Some(Self::Summary),
            "context" => Some(Self::Context),
            "task_checkpoint" => Some(Self::TaskCheckpoint),
            "subagent_transcript" => Some(Self::SubagentTranscript),
            "incremental_checkpoint" => Some(Self::IncrementalCheckpoint),
            "prompt" => Some(Self::Prompt),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMetadataSnapshot {
    pub snapshot_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub transcript_identifier: String,
    pub transcript_path: String,
    pub bundle: SessionMetadataBundle,
}

impl SessionMetadataSnapshot {
    pub fn new(session_id: impl Into<String>, bundle: SessionMetadataBundle) -> Self {
        Self {
            snapshot_id: Uuid::new_v4().simple().to_string(),
            session_id: session_id.into(),
            turn_id: String::new(),
            transcript_identifier: String::new(),
            transcript_path: String::new(),
            bundle,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskCheckpointArtefact {
    pub artefact_id: String,
    pub session_id: String,
    pub tool_use_id: String,
    pub agent_id: String,
    pub checkpoint_uuid: String,
    pub kind: RuntimeMetadataBlobType,
    pub incremental_sequence: Option<u32>,
    pub incremental_type: String,
    pub is_incremental: bool,
    pub payload: Vec<u8>,
}

impl TaskCheckpointArtefact {
    pub fn new(
        session_id: impl Into<String>,
        tool_use_id: impl Into<String>,
        kind: RuntimeMetadataBlobType,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            artefact_id: Uuid::new_v4().simple().to_string(),
            session_id: session_id.into(),
            tool_use_id: tool_use_id.into(),
            agent_id: String::new(),
            checkpoint_uuid: String::new(),
            kind,
            incremental_sequence: None,
            incremental_type: String::new(),
            is_incremental: matches!(kind, RuntimeMetadataBlobType::IncrementalCheckpoint),
            payload,
        }
    }
}

impl Default for PersistedSyncQueueState {
    fn default() -> Self {
        Self {
            version: 1,
            tasks: Vec::new(),
            last_action: Some("initialized".to_string()),
            updated_at_unix: 0,
        }
    }
}

impl Default for PersistedDevqlTaskQueueState {
    fn default() -> Self {
        Self {
            version: 1,
            tasks: Vec::new(),
            repo_controls: BTreeMap::new(),
            last_action: Some("initialized".to_string()),
            updated_at_unix: 0,
        }
    }
}

impl Default for PersistedCapabilityEventQueueState {
    fn default() -> Self {
        Self {
            version: 1,
            runs: Vec::new(),
            last_action: Some("initialized".to_string()),
            updated_at_unix: 0,
        }
    }
}

impl PersistedSyncQueueState {
    pub(crate) fn normalise_legacy_values(&mut self) {
        for task in &mut self.tasks {
            task.normalise_legacy_values();
        }
    }
}

impl LegacySyncTaskRecord {
    pub(crate) fn normalise_legacy_values(&mut self) {
        if self.daemon_config_root.as_os_str().is_empty() {
            self.daemon_config_root = self.repo_root.clone();
        }
    }
}

impl PersistedDevqlTaskQueueState {
    pub(crate) fn normalise_legacy_values(&mut self) {
        for task in &mut self.tasks {
            task.normalise_legacy_values();
        }
    }
}

impl SqliteRuntimeStore {
    pub fn new() -> Self {
        Self
    }
}

impl RuntimeStore for SqliteRuntimeStore {
    type RepoStore = RepoSqliteRuntimeStore;
    type DaemonStore = DaemonSqliteRuntimeStore;

    fn repo_store(&self, repo_root: &Path) -> Result<Self::RepoStore> {
        RepoSqliteRuntimeStore::open(repo_root)
    }

    fn daemon_store(&self) -> Result<Self::DaemonStore> {
        DaemonSqliteRuntimeStore::open()
    }
}
