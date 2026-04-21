#![allow(dead_code)]

use crate::host::devql::{
    IngestionCounters, InitSchemaSummary, SyncSummary, SyncValidationFileDrift,
    SyncValidationSummary,
};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InitSchemaMutationData {
    pub(super) init_schema: InitSchemaSummary,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EnqueueTaskMutationData {
    pub(super) enqueue_task: EnqueueTaskMutationResult,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EnqueueTaskMutationResult {
    pub(super) merged: bool,
    pub(super) task: TaskGraphqlRecord,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TaskQueryData {
    pub(super) task: Option<TaskGraphqlRecord>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TasksQueryData {
    pub(super) tasks: Vec<TaskGraphqlRecord>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TaskQueueQueryData {
    pub(super) task_queue: TaskQueueGraphqlRecord,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PauseTaskQueueMutationData {
    pub(super) pause_task_queue: TaskQueueControlGraphqlRecord,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ResumeTaskQueueMutationData {
    pub(super) resume_task_queue: TaskQueueControlGraphqlRecord,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CancelTaskMutationData {
    pub(super) cancel_task: TaskGraphqlRecord,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TaskProgressSubscriptionData {
    pub(super) task_progress: TaskProgressEventGraphqlRecord,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TaskProgressEventGraphqlRecord {
    pub(super) task: TaskGraphqlRecord,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskGraphqlRecord {
    pub task_id: String,
    pub repo_id: String,
    pub repo_name: String,
    pub repo_identity: String,
    pub kind: String,
    pub source: String,
    pub status: String,
    pub submitted_at_unix: i64,
    pub started_at_unix: Option<i64>,
    pub updated_at_unix: i64,
    pub completed_at_unix: Option<i64>,
    pub queue_position: Option<i32>,
    pub tasks_ahead: Option<i32>,
    pub error: Option<String>,
    pub sync_spec: Option<SyncTaskSpecGraphqlRecord>,
    pub ingest_spec: Option<IngestTaskSpecGraphqlRecord>,
    pub embeddings_bootstrap_spec: Option<EmbeddingsBootstrapTaskSpecGraphqlRecord>,
    pub summary_bootstrap_spec: Option<SummaryBootstrapTaskSpecGraphqlRecord>,
    pub sync_progress: Option<SyncTaskProgressGraphqlRecord>,
    pub ingest_progress: Option<IngestTaskProgressGraphqlRecord>,
    pub embeddings_bootstrap_progress: Option<EmbeddingsBootstrapProgressGraphqlRecord>,
    pub summary_bootstrap_progress: Option<SummaryBootstrapProgressGraphqlRecord>,
    pub sync_result: Option<SyncMutationResult>,
    pub ingest_result: Option<IngestionCounters>,
    pub embeddings_bootstrap_result: Option<EmbeddingsBootstrapResultGraphqlRecord>,
    pub summary_bootstrap_result: Option<SummaryBootstrapResultGraphqlRecord>,
}

impl TaskGraphqlRecord {
    pub(crate) fn is_sync(&self) -> bool {
        self.kind.eq_ignore_ascii_case("sync")
    }

    pub(crate) fn is_ingest(&self) -> bool {
        self.kind.eq_ignore_ascii_case("ingest")
    }

    pub(crate) fn is_embeddings_bootstrap(&self) -> bool {
        self.kind.eq_ignore_ascii_case("embeddings_bootstrap")
    }

    pub(crate) fn is_summary_bootstrap(&self) -> bool {
        self.kind.eq_ignore_ascii_case("summary_bootstrap")
    }

    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self.status.to_ascii_lowercase().as_str(),
            "completed" | "failed" | "cancelled"
        )
    }

    pub(crate) fn sync_summary(&self) -> Option<SyncSummary> {
        self.sync_result.clone().map(Into::into)
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncTaskSpecGraphqlRecord {
    pub mode: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IngestTaskSpecGraphqlRecord {
    pub backfill: Option<i32>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmbeddingsBootstrapTaskSpecGraphqlRecord {
    pub config_path: String,
    pub profile_name: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SummaryBootstrapTaskSpecGraphqlRecord {
    pub action: String,
    pub message: Option<String>,
    pub model_name: Option<String>,
    pub gateway_url_override: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncTaskProgressGraphqlRecord {
    pub phase: String,
    pub current_path: Option<String>,
    pub paths_total: i32,
    pub paths_completed: i32,
    pub paths_remaining: i32,
    pub paths_unchanged: i32,
    pub paths_added: i32,
    pub paths_changed: i32,
    pub paths_removed: i32,
    pub cache_hits: i32,
    pub cache_misses: i32,
    pub parse_errors: i32,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IngestTaskProgressGraphqlRecord {
    pub phase: String,
    pub commits_total: i32,
    pub commits_processed: i32,
    pub checkpoint_companions_processed: i32,
    pub current_checkpoint_id: Option<String>,
    pub current_commit_sha: Option<String>,
    pub events_inserted: i32,
    pub artefacts_upserted: i32,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmbeddingsBootstrapProgressGraphqlRecord {
    pub phase: String,
    pub asset_name: Option<String>,
    pub bytes_downloaded: i64,
    pub bytes_total: Option<i64>,
    pub version: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SummaryBootstrapProgressGraphqlRecord {
    pub phase: String,
    pub asset_name: Option<String>,
    pub bytes_downloaded: i64,
    pub bytes_total: Option<i64>,
    pub version: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmbeddingsBootstrapResultGraphqlRecord {
    pub version: Option<String>,
    pub binary_path: Option<String>,
    pub cache_dir: Option<String>,
    pub runtime_name: Option<String>,
    pub model_name: Option<String>,
    pub freshly_installed: bool,
    pub message: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SummaryBootstrapResultGraphqlRecord {
    pub outcome_kind: String,
    pub model_name: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskQueueGraphqlRecord {
    pub persisted: bool,
    pub queued_tasks: i32,
    pub running_tasks: i32,
    pub failed_tasks: i32,
    pub completed_recent_tasks: i32,
    pub by_kind: Vec<TaskKindCountsGraphqlRecord>,
    pub paused: bool,
    pub paused_reason: Option<String>,
    pub last_action: Option<String>,
    pub last_updated_unix: i64,
    pub current_repo_tasks: Vec<TaskGraphqlRecord>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskKindCountsGraphqlRecord {
    pub kind: String,
    pub queued_tasks: i32,
    pub running_tasks: i32,
    pub failed_tasks: i32,
    pub completed_recent_tasks: i32,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskQueueControlGraphqlRecord {
    pub message: String,
    pub repo_id: String,
    pub paused: bool,
    pub paused_reason: Option<String>,
    pub updated_at_unix: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeStartInitInput {
    pub repo_id: String,
    pub run_sync: bool,
    pub run_ingest: bool,
    pub run_code_embeddings: bool,
    pub run_summaries: bool,
    pub run_summary_embeddings: bool,
    pub ingest_backfill: Option<usize>,
    pub embeddings_bootstrap: Option<RuntimeEmbeddingsBootstrapRequestInput>,
    pub summaries_bootstrap: Option<RuntimeSummaryBootstrapRequestInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeEmbeddingsBootstrapRequestInput {
    pub config_path: String,
    pub profile_name: String,
    pub mode: String,
    pub gateway_url_override: Option<String>,
    pub api_key_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeSummaryBootstrapRequestInput {
    pub action: String,
    pub message: Option<String>,
    pub model_name: Option<String>,
    pub gateway_url_override: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StartInitMutationData {
    pub(super) start_init: StartInitResultGraphqlRecord,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartInitResultGraphqlRecord {
    pub init_session_id: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RuntimeSnapshotQueryData {
    pub(super) runtime_snapshot: RuntimeSnapshotGraphqlRecord,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RuntimeEventsSubscriptionData {
    pub(super) runtime_events: RuntimeEventGraphqlRecord,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeSnapshotGraphqlRecord {
    pub repo_id: String,
    pub task_queue: TaskQueueGraphqlRecord,
    pub current_state_consumer: RuntimeCurrentStateConsumerGraphqlRecord,
    pub workplane: RuntimeWorkplaneGraphqlRecord,
    pub blocked_mailboxes: Vec<RuntimeBlockedMailboxGraphqlRecord>,
    pub embeddings_readiness_gate: Option<RuntimeEmbeddingsReadinessGateGraphqlRecord>,
    pub summaries_bootstrap: Option<RuntimeSummaryBootstrapRunGraphqlRecord>,
    pub current_init_session: Option<RuntimeInitSessionGraphqlRecord>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeCurrentStateConsumerGraphqlRecord {
    pub persisted: bool,
    pub pending_runs: i32,
    pub running_runs: i32,
    pub failed_runs: i32,
    pub completed_recent_runs: i32,
    pub last_action: Option<String>,
    pub last_updated_unix: i64,
    pub current_repo_run: Option<RuntimeCurrentStateRunGraphqlRecord>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeCurrentStateRunGraphqlRecord {
    pub run_id: String,
    pub repo_id: String,
    pub capability_id: String,
    pub init_session_id: Option<String>,
    pub consumer_id: String,
    pub handler_id: String,
    pub from_generation_seq: i64,
    pub to_generation_seq: i64,
    pub reconcile_mode: String,
    pub status: String,
    pub attempts: i32,
    pub submitted_at_unix: i64,
    pub started_at_unix: Option<i64>,
    pub updated_at_unix: i64,
    pub completed_at_unix: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeWorkplaneGraphqlRecord {
    pub pending_jobs: i32,
    pub running_jobs: i32,
    pub failed_jobs: i32,
    pub completed_recent_jobs: i32,
    #[serde(default)]
    pub pools: Vec<RuntimeWorkplanePoolGraphqlRecord>,
    pub mailboxes: Vec<RuntimeWorkplaneMailboxGraphqlRecord>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeWorkplanePoolGraphqlRecord {
    pub pool_name: String,
    pub display_name: String,
    pub worker_budget: i32,
    pub active_workers: i32,
    pub pending_jobs: i32,
    pub running_jobs: i32,
    pub failed_jobs: i32,
    pub completed_recent_jobs: i32,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeWorkplaneMailboxGraphqlRecord {
    pub mailbox_name: String,
    pub display_name: String,
    pub pending_jobs: i32,
    pub running_jobs: i32,
    pub failed_jobs: i32,
    pub completed_recent_jobs: i32,
    pub pending_cursor_runs: i32,
    pub running_cursor_runs: i32,
    pub failed_cursor_runs: i32,
    pub completed_recent_cursor_runs: i32,
    pub intent_active: bool,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeBlockedMailboxGraphqlRecord {
    pub mailbox_name: String,
    pub display_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeEmbeddingsReadinessGateGraphqlRecord {
    pub blocked: bool,
    pub readiness: Option<String>,
    pub reason: Option<String>,
    pub active_task_id: Option<String>,
    pub profile_name: Option<String>,
    pub config_path: Option<String>,
    pub last_error: Option<String>,
    pub last_updated_unix: i64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeSummaryBootstrapRunGraphqlRecord {
    pub run_id: String,
    pub repo_id: String,
    pub init_session_id: String,
    pub status: String,
    pub request: RuntimeSummaryBootstrapRequestGraphqlRecord,
    pub progress: RuntimeSummaryBootstrapProgressGraphqlRecord,
    pub result: Option<RuntimeSummaryBootstrapResultGraphqlRecord>,
    pub error: Option<String>,
    pub submitted_at_unix: i64,
    pub started_at_unix: Option<i64>,
    pub updated_at_unix: i64,
    pub completed_at_unix: Option<i64>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeSummaryBootstrapRequestGraphqlRecord {
    pub action: String,
    pub message: Option<String>,
    pub model_name: Option<String>,
    pub gateway_url_override: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeSummaryBootstrapProgressGraphqlRecord {
    pub phase: String,
    pub asset_name: Option<String>,
    pub bytes_downloaded: i64,
    pub bytes_total: Option<i64>,
    pub version: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeSummaryBootstrapResultGraphqlRecord {
    pub outcome_kind: String,
    pub model_name: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeInitSessionGraphqlRecord {
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
    pub sync_lane: RuntimeInitLaneGraphqlRecord,
    pub ingest_lane: RuntimeInitLaneGraphqlRecord,
    pub code_embeddings_lane: RuntimeInitLaneGraphqlRecord,
    pub summaries_lane: RuntimeInitLaneGraphqlRecord,
    pub summary_embeddings_lane: RuntimeInitLaneGraphqlRecord,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeInitLaneGraphqlRecord {
    pub status: String,
    pub waiting_reason: Option<String>,
    pub detail: Option<String>,
    pub activity_label: Option<String>,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub progress: Option<RuntimeInitLaneProgressGraphqlRecord>,
    #[serde(default)]
    pub queue: RuntimeInitLaneQueueGraphqlRecord,
    #[serde(default)]
    pub warnings: Vec<RuntimeInitLaneWarningGraphqlRecord>,
    #[serde(default)]
    pub pending_count: i32,
    #[serde(default)]
    pub running_count: i32,
    #[serde(default)]
    pub failed_count: i32,
    #[serde(default)]
    pub completed_count: i32,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeInitLaneProgressGraphqlRecord {
    pub completed: i32,
    #[serde(default)]
    pub in_memory_completed: i32,
    pub total: i32,
    pub remaining: i32,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeInitLaneQueueGraphqlRecord {
    pub queued: i32,
    pub running: i32,
    pub failed: i32,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeInitLaneWarningGraphqlRecord {
    pub component_label: String,
    pub message: String,
    pub retry_command: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeEventGraphqlRecord {
    pub domain: String,
    pub repo_id: String,
    pub init_session_id: Option<String>,
    pub updated_at_unix: i64,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub mailbox_name: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncMutationResult {
    pub(crate) success: bool,
    pub(crate) mode: String,
    pub(crate) parser_version: String,
    pub(crate) extractor_version: String,
    pub(crate) active_branch: Option<String>,
    pub(crate) head_commit_sha: Option<String>,
    pub(crate) head_tree_sha: Option<String>,
    pub(crate) paths_unchanged: usize,
    pub(crate) paths_added: usize,
    pub(crate) paths_changed: usize,
    pub(crate) paths_removed: usize,
    pub(crate) cache_hits: usize,
    pub(crate) cache_misses: usize,
    pub(crate) parse_errors: usize,
    pub(crate) validation: Option<SyncValidationMutationResult>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncValidationMutationResult {
    pub(crate) valid: bool,
    pub(crate) expected_artefacts: usize,
    pub(crate) actual_artefacts: usize,
    pub(crate) expected_edges: usize,
    pub(crate) actual_edges: usize,
    pub(crate) missing_artefacts: usize,
    pub(crate) stale_artefacts: usize,
    pub(crate) mismatched_artefacts: usize,
    pub(crate) missing_edges: usize,
    pub(crate) stale_edges: usize,
    pub(crate) mismatched_edges: usize,
    pub(crate) files_with_drift: Vec<SyncValidationFileDriftMutationResult>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncValidationFileDriftMutationResult {
    pub(crate) path: String,
    pub(crate) missing_artefacts: usize,
    pub(crate) stale_artefacts: usize,
    pub(crate) mismatched_artefacts: usize,
    pub(crate) missing_edges: usize,
    pub(crate) stale_edges: usize,
    pub(crate) mismatched_edges: usize,
}

impl From<SyncMutationResult> for SyncSummary {
    fn from(value: SyncMutationResult) -> Self {
        Self {
            success: value.success,
            mode: value.mode,
            parser_version: value.parser_version,
            extractor_version: value.extractor_version,
            active_branch: value.active_branch,
            head_commit_sha: value.head_commit_sha,
            head_tree_sha: value.head_tree_sha,
            paths_unchanged: value.paths_unchanged,
            paths_added: value.paths_added,
            paths_changed: value.paths_changed,
            paths_removed: value.paths_removed,
            cache_hits: value.cache_hits,
            cache_misses: value.cache_misses,
            parse_errors: value.parse_errors,
            validation: value.validation.map(|validation| SyncValidationSummary {
                valid: validation.valid,
                expected_artefacts: validation.expected_artefacts,
                actual_artefacts: validation.actual_artefacts,
                expected_edges: validation.expected_edges,
                actual_edges: validation.actual_edges,
                missing_artefacts: validation.missing_artefacts,
                stale_artefacts: validation.stale_artefacts,
                mismatched_artefacts: validation.mismatched_artefacts,
                missing_edges: validation.missing_edges,
                stale_edges: validation.stale_edges,
                mismatched_edges: validation.mismatched_edges,
                files_with_drift: validation
                    .files_with_drift
                    .into_iter()
                    .map(|file| SyncValidationFileDrift {
                        path: file.path,
                        missing_artefacts: file.missing_artefacts,
                        stale_artefacts: file.stale_artefacts,
                        mismatched_artefacts: file.mismatched_artefacts,
                        missing_edges: file.missing_edges,
                        stale_edges: file.stale_edges,
                        mismatched_edges: file.mismatched_edges,
                    })
                    .collect(),
            }),
        }
    }
}
