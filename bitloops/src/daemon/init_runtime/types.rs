use serde::{Deserialize, Serialize};

use crate::daemon::types::{
    BlockedMailboxStatus, EmbeddingsBootstrapGateStatus, InitSessionState,
    SummaryBootstrapRunRecord, SummaryBootstrapState,
};
use crate::daemon::{CapabilityEventQueueStatus, DevqlTaskQueueStatus};

pub(crate) type PersistedInitSessionState = InitSessionState;
pub(crate) type PersistedSummaryBootstrapState = SummaryBootstrapState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeEventRecord {
    pub domain: String,
    pub repo_id: String,
    pub init_session_id: Option<String>,
    pub updated_at_unix: u64,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub mailbox_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitSessionHandle {
    pub init_session_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeWorkplaneMailboxSnapshot {
    pub mailbox_name: String,
    pub display_name: String,
    pub pending_jobs: u64,
    pub running_jobs: u64,
    pub failed_jobs: u64,
    pub completed_recent_jobs: u64,
    pub pending_cursor_runs: u64,
    pub running_cursor_runs: u64,
    pub failed_cursor_runs: u64,
    pub completed_recent_cursor_runs: u64,
    pub intent_active: bool,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeWorkplaneSnapshot {
    pub pending_jobs: u64,
    pub running_jobs: u64,
    pub failed_jobs: u64,
    pub completed_recent_jobs: u64,
    pub pools: Vec<InitRuntimeWorkplanePoolSnapshot>,
    pub mailboxes: Vec<InitRuntimeWorkplaneMailboxSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeWorkplanePoolSnapshot {
    pub pool_name: String,
    pub display_name: String,
    pub worker_budget: u64,
    pub active_workers: u64,
    pub pending_jobs: u64,
    pub running_jobs: u64,
    pub failed_jobs: u64,
    pub completed_recent_jobs: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeLaneProgressView {
    pub completed: u64,
    pub in_memory_completed: u64,
    pub total: u64,
    pub remaining: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeLaneQueueView {
    pub queued: u64,
    pub running: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeLaneWarningView {
    pub component_label: String,
    pub message: String,
    pub retry_command: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeLaneView {
    pub status: String,
    pub waiting_reason: Option<String>,
    pub detail: Option<String>,
    pub activity_label: Option<String>,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub progress: Option<InitRuntimeLaneProgressView>,
    pub queue: InitRuntimeLaneQueueView,
    pub warnings: Vec<InitRuntimeLaneWarningView>,
    pub pending_count: u64,
    pub running_count: u64,
    pub failed_count: u64,
    pub completed_count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeSessionView {
    pub init_session_id: String,
    pub status: String,
    pub waiting_reason: Option<String>,
    pub warning_summary: Option<String>,
    pub follow_up_sync_required: bool,
    pub run_sync: bool,
    pub run_ingest: bool,
    pub embeddings_selected: bool,
    pub summaries_selected: bool,
    pub summary_embeddings_selected: bool,
    pub initial_sync_task_id: Option<String>,
    pub ingest_task_id: Option<String>,
    pub follow_up_sync_task_id: Option<String>,
    pub embeddings_bootstrap_task_id: Option<String>,
    pub summary_bootstrap_task_id: Option<String>,
    pub terminal_error: Option<String>,
    pub sync_lane: InitRuntimeLaneView,
    pub ingest_lane: InitRuntimeLaneView,
    pub code_embeddings_lane: InitRuntimeLaneView,
    pub summaries_lane: InitRuntimeLaneView,
    pub summary_embeddings_lane: InitRuntimeLaneView,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeOverviewSnapshot {
    pub repo_id: String,
    pub task_queue: DevqlTaskQueueStatus,
    pub current_state_consumer: CapabilityEventQueueStatus,
    pub workplane: InitRuntimeWorkplaneSnapshot,
    pub blocked_mailboxes: Vec<BlockedMailboxStatus>,
    pub embeddings_readiness_gate: Option<EmbeddingsBootstrapGateStatus>,
    pub summaries_bootstrap: Option<SummaryBootstrapRunRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeSnapshot {
    pub repo_id: String,
    pub task_queue: DevqlTaskQueueStatus,
    pub current_state_consumer: CapabilityEventQueueStatus,
    pub workplane: InitRuntimeWorkplaneSnapshot,
    pub blocked_mailboxes: Vec<BlockedMailboxStatus>,
    pub embeddings_readiness_gate: Option<EmbeddingsBootstrapGateStatus>,
    pub summaries_bootstrap: Option<SummaryBootstrapRunRecord>,
    pub current_init_session: Option<InitRuntimeSessionView>,
}

impl InitRuntimeLaneView {
    pub(crate) fn with_activity_label(mut self, activity_label: impl Into<String>) -> Self {
        let activity_label = activity_label.into();
        self.detail = Some(activity_label.clone());
        self.activity_label = Some(activity_label);
        self
    }

    pub(crate) fn with_activity_label_option(mut self, activity_label: Option<String>) -> Self {
        if let Some(activity_label) = activity_label {
            self = self.with_activity_label(activity_label);
        }
        self
    }

    pub(crate) fn with_waiting_reason(mut self, waiting_reason: impl Into<String>) -> Self {
        self.waiting_reason = Some(waiting_reason.into());
        self
    }

    pub(crate) fn with_task_id_option(mut self, task_id: Option<String>) -> Self {
        self.task_id = task_id;
        self
    }

    pub(crate) fn with_run_id_option(mut self, run_id: Option<String>) -> Self {
        self.run_id = run_id;
        self
    }

    pub(crate) fn with_detail(mut self, detail: String) -> Self {
        self.detail = Some(detail);
        self
    }
}
