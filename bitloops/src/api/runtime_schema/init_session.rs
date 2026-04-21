use async_graphql::{ID, SimpleObject};

use super::util::to_graphql_i32;
use crate::daemon::{
    InitRuntimeLaneProgressView, InitRuntimeLaneQueueView, InitRuntimeLaneView,
    InitRuntimeLaneWarningView, InitRuntimeSessionView,
};

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeInitSessionObject {
    #[graphql(name = "initSessionId")]
    pub init_session_id: ID,
    pub status: String,
    #[graphql(name = "waitingReason")]
    pub waiting_reason: Option<String>,
    #[graphql(name = "warningSummary")]
    pub warning_summary: Option<String>,
    #[graphql(name = "followUpSyncRequired")]
    pub follow_up_sync_required: bool,
    #[graphql(name = "runSync")]
    pub run_sync: bool,
    #[graphql(name = "runIngest")]
    pub run_ingest: bool,
    #[graphql(name = "embeddingsSelected")]
    pub embeddings_selected: bool,
    #[graphql(name = "summariesSelected")]
    pub summaries_selected: bool,
    #[graphql(name = "summaryEmbeddingsSelected")]
    pub summary_embeddings_selected: bool,
    #[graphql(name = "initialSyncTaskId")]
    pub initial_sync_task_id: Option<String>,
    #[graphql(name = "ingestTaskId")]
    pub ingest_task_id: Option<String>,
    #[graphql(name = "followUpSyncTaskId")]
    pub follow_up_sync_task_id: Option<String>,
    #[graphql(name = "embeddingsBootstrapTaskId")]
    pub embeddings_bootstrap_task_id: Option<String>,
    #[graphql(name = "summaryBootstrapTaskId")]
    pub summary_bootstrap_task_id: Option<ID>,
    #[graphql(name = "terminalError")]
    pub terminal_error: Option<String>,
    #[graphql(name = "syncLane")]
    pub sync_lane: RuntimeInitLaneObject,
    #[graphql(name = "ingestLane")]
    pub ingest_lane: RuntimeInitLaneObject,
    #[graphql(name = "codeEmbeddingsLane")]
    pub code_embeddings_lane: RuntimeInitLaneObject,
    #[graphql(name = "summariesLane")]
    pub summaries_lane: RuntimeInitLaneObject,
    #[graphql(name = "summaryEmbeddingsLane")]
    pub summary_embeddings_lane: RuntimeInitLaneObject,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeInitLaneObject {
    pub status: String,
    #[graphql(name = "waitingReason")]
    pub waiting_reason: Option<String>,
    pub detail: Option<String>,
    #[graphql(name = "activityLabel")]
    pub activity_label: Option<String>,
    #[graphql(name = "taskId")]
    pub task_id: Option<String>,
    #[graphql(name = "runId")]
    pub run_id: Option<ID>,
    pub progress: Option<RuntimeInitLaneProgressObject>,
    pub queue: RuntimeInitLaneQueueObject,
    pub warnings: Vec<RuntimeInitLaneWarningObject>,
    #[graphql(name = "pendingCount")]
    pub pending_count: i32,
    #[graphql(name = "runningCount")]
    pub running_count: i32,
    #[graphql(name = "failedCount")]
    pub failed_count: i32,
    #[graphql(name = "completedCount")]
    pub completed_count: i32,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeInitLaneProgressObject {
    pub completed: i32,
    #[graphql(name = "inMemoryCompleted")]
    pub in_memory_completed: i32,
    pub total: i32,
    pub remaining: i32,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeInitLaneQueueObject {
    pub queued: i32,
    pub running: i32,
    pub failed: i32,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeInitLaneWarningObject {
    #[graphql(name = "componentLabel")]
    pub component_label: String,
    pub message: String,
    #[graphql(name = "retryCommand")]
    pub retry_command: String,
}

impl From<InitRuntimeSessionView> for RuntimeInitSessionObject {
    fn from(value: InitRuntimeSessionView) -> Self {
        Self {
            init_session_id: ID::from(value.init_session_id),
            status: value.status,
            waiting_reason: value.waiting_reason,
            warning_summary: value.warning_summary,
            follow_up_sync_required: value.follow_up_sync_required,
            run_sync: value.run_sync,
            run_ingest: value.run_ingest,
            embeddings_selected: value.embeddings_selected,
            summaries_selected: value.summaries_selected,
            summary_embeddings_selected: value.summary_embeddings_selected,
            initial_sync_task_id: value.initial_sync_task_id,
            ingest_task_id: value.ingest_task_id,
            follow_up_sync_task_id: value.follow_up_sync_task_id,
            embeddings_bootstrap_task_id: value.embeddings_bootstrap_task_id,
            summary_bootstrap_task_id: value.summary_bootstrap_task_id.map(ID::from),
            terminal_error: value.terminal_error,
            sync_lane: value.sync_lane.into(),
            ingest_lane: value.ingest_lane.into(),
            code_embeddings_lane: value.code_embeddings_lane.into(),
            summaries_lane: value.summaries_lane.into(),
            summary_embeddings_lane: value.summary_embeddings_lane.into(),
        }
    }
}

impl From<InitRuntimeLaneView> for RuntimeInitLaneObject {
    fn from(value: InitRuntimeLaneView) -> Self {
        Self {
            status: value.status,
            waiting_reason: value.waiting_reason,
            detail: value.detail,
            activity_label: value.activity_label,
            task_id: value.task_id,
            run_id: value.run_id.map(ID::from),
            progress: value.progress.map(Into::into),
            queue: value.queue.into(),
            warnings: value.warnings.into_iter().map(Into::into).collect(),
            pending_count: to_graphql_i32(value.pending_count),
            running_count: to_graphql_i32(value.running_count),
            failed_count: to_graphql_i32(value.failed_count),
            completed_count: to_graphql_i32(value.completed_count),
        }
    }
}

impl From<InitRuntimeLaneProgressView> for RuntimeInitLaneProgressObject {
    fn from(value: InitRuntimeLaneProgressView) -> Self {
        Self {
            completed: to_graphql_i32(value.completed),
            in_memory_completed: to_graphql_i32(value.in_memory_completed),
            total: to_graphql_i32(value.total),
            remaining: to_graphql_i32(value.remaining),
        }
    }
}

impl From<InitRuntimeLaneQueueView> for RuntimeInitLaneQueueObject {
    fn from(value: InitRuntimeLaneQueueView) -> Self {
        Self {
            queued: to_graphql_i32(value.queued),
            running: to_graphql_i32(value.running),
            failed: to_graphql_i32(value.failed),
        }
    }
}

impl From<InitRuntimeLaneWarningView> for RuntimeInitLaneWarningObject {
    fn from(value: InitRuntimeLaneWarningView) -> Self {
        Self {
            component_label: value.component_label,
            message: value.message,
            retry_command: value.retry_command,
        }
    }
}
