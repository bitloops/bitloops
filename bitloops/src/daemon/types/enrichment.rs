use std::fmt;

use serde::{Deserialize, Serialize};

use super::embeddings_bootstrap::EmbeddingsBootstrapGateStatus;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnrichmentQueueMode {
    Running,
    Paused,
}

impl fmt::Display for EnrichmentQueueMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Paused => write!(f, "paused"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnrichmentWorkerPoolKind {
    SummaryRefresh,
    Embeddings,
    CloneRebuild,
}

impl fmt::Display for EnrichmentWorkerPoolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SummaryRefresh => write!(f, "summary_refresh"),
            Self::Embeddings => write!(f, "embeddings"),
            Self::CloneRebuild => write!(f, "clone_rebuild"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentWorkerPoolStatus {
    pub kind: EnrichmentWorkerPoolKind,
    pub worker_budget: u64,
    pub active_workers: u64,
    pub pending_jobs: u64,
    pub running_jobs: u64,
    pub failed_jobs: u64,
    pub completed_recent_jobs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentQueueState {
    pub version: u8,
    pub mode: EnrichmentQueueMode,
    #[serde(default)]
    pub worker_pools: Vec<EnrichmentWorkerPoolStatus>,
    pub pending_jobs: u64,
    pub pending_work_items: u64,
    pub pending_semantic_jobs: u64,
    pub pending_semantic_work_items: u64,
    pub pending_embedding_jobs: u64,
    pub pending_embedding_work_items: u64,
    pub pending_clone_edges_rebuild_jobs: u64,
    pub pending_clone_edges_rebuild_work_items: u64,
    #[serde(default)]
    pub completed_recent_jobs: u64,
    pub running_jobs: u64,
    pub running_work_items: u64,
    pub running_semantic_jobs: u64,
    pub running_semantic_work_items: u64,
    pub running_embedding_jobs: u64,
    pub running_embedding_work_items: u64,
    pub running_clone_edges_rebuild_jobs: u64,
    pub running_clone_edges_rebuild_work_items: u64,
    pub failed_jobs: u64,
    pub failed_work_items: u64,
    pub failed_semantic_jobs: u64,
    pub failed_semantic_work_items: u64,
    pub failed_embedding_jobs: u64,
    pub failed_embedding_work_items: u64,
    pub failed_clone_edges_rebuild_jobs: u64,
    pub failed_clone_edges_rebuild_work_items: u64,
    pub retried_failed_jobs: u64,
    pub last_action: Option<String>,
    pub last_updated_unix: u64,
    pub paused_reason: Option<String>,
}

impl Default for EnrichmentQueueState {
    fn default() -> Self {
        Self {
            version: 1,
            mode: EnrichmentQueueMode::Running,
            worker_pools: Vec::new(),
            pending_jobs: 0,
            pending_work_items: 0,
            pending_semantic_jobs: 0,
            pending_semantic_work_items: 0,
            pending_embedding_jobs: 0,
            pending_embedding_work_items: 0,
            pending_clone_edges_rebuild_jobs: 0,
            pending_clone_edges_rebuild_work_items: 0,
            completed_recent_jobs: 0,
            running_jobs: 0,
            running_work_items: 0,
            running_semantic_jobs: 0,
            running_semantic_work_items: 0,
            running_embedding_jobs: 0,
            running_embedding_work_items: 0,
            running_clone_edges_rebuild_jobs: 0,
            running_clone_edges_rebuild_work_items: 0,
            failed_jobs: 0,
            failed_work_items: 0,
            failed_semantic_jobs: 0,
            failed_semantic_work_items: 0,
            failed_embedding_jobs: 0,
            failed_embedding_work_items: 0,
            failed_clone_edges_rebuild_jobs: 0,
            failed_clone_edges_rebuild_work_items: 0,
            retried_failed_jobs: 0,
            last_action: Some("initialized".to_string()),
            last_updated_unix: 0,
            paused_reason: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FailedEmbeddingJobSummary {
    pub job_id: String,
    pub repo_id: String,
    pub branch: String,
    pub representation_kind: String,
    pub artefact_count: u64,
    pub attempts: u32,
    pub error: Option<String>,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlockedMailboxStatus {
    pub mailbox_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnrichmentQueueStatus {
    pub state: EnrichmentQueueState,
    pub persisted: bool,
    pub embeddings_gate: Option<EmbeddingsBootstrapGateStatus>,
    pub blocked_mailboxes: Vec<BlockedMailboxStatus>,
    pub last_failed_embedding: Option<FailedEmbeddingJobSummary>,
}
