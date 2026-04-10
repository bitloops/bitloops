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
pub(super) struct IngestMutationData {
    pub(super) ingest: IngestionCounters,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EnqueueSyncMutationData {
    pub(super) enqueue_sync: EnqueueSyncMutationResult,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EnqueueSyncMutationResult {
    pub(super) merged: bool,
    pub(super) task: SyncTaskGraphqlRecord,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SyncTaskQueryData {
    pub(super) sync_task: Option<SyncTaskGraphqlRecord>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SyncProgressSubscriptionData {
    pub(super) sync_progress: SyncTaskGraphqlRecord,
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

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub(crate) struct SyncTaskGraphqlRecord {
    pub task_id: String,
    pub repo_id: String,
    pub repo_name: String,
    pub repo_identity: String,
    pub source: String,
    pub mode: String,
    pub status: String,
    pub phase: String,
    pub submitted_at_unix: i64,
    pub started_at_unix: Option<i64>,
    pub updated_at_unix: i64,
    pub completed_at_unix: Option<i64>,
    pub queue_position: Option<i32>,
    pub tasks_ahead: Option<i32>,
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
    pub error: Option<String>,
    pub summary: Option<SyncMutationResult>,
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
