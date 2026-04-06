use crate::host::checkpoints::strategy::manual_commit::CommittedInfo;

// Shared types used across ingestion modules.

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IngestionCounters {
    pub(crate) success: bool,
    pub(crate) checkpoints_processed: usize,
    pub(crate) events_inserted: usize,
    pub(crate) artefacts_upserted: usize,
    pub(crate) checkpoints_without_commit: usize,
    pub(crate) temporary_rows_promoted: usize,
    pub(crate) semantic_feature_rows_upserted: usize,
    pub(crate) semantic_feature_rows_skipped: usize,
    pub(crate) symbol_embedding_rows_upserted: usize,
    pub(crate) symbol_embedding_rows_skipped: usize,
    pub(crate) symbol_clone_edges_upserted: usize,
    pub(crate) symbol_clone_sources_scored: usize,
    #[serde(default)]
    pub(crate) interaction_events_attempted: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IngestionProgressPhase {
    Initializing,
    Extracting,
    Persisting,
    Complete,
    Failed,
}

#[derive(Debug, Clone)]
pub(crate) struct IngestionProgressUpdate {
    pub(crate) phase: IngestionProgressPhase,
    pub(crate) checkpoints_total: usize,
    pub(crate) checkpoints_processed: usize,
    pub(crate) current_checkpoint_id: Option<String>,
    pub(crate) current_commit_sha: Option<String>,
    pub(crate) counters: IngestionCounters,
}

#[derive(Debug, Clone)]
pub(crate) struct IngestedCheckpointNotification {
    pub(crate) checkpoint: CommittedInfo,
    pub(crate) commit_sha: Option<String>,
}

pub(crate) trait IngestionObserver: Send + Sync {
    fn on_progress(&self, update: IngestionProgressUpdate);

    fn on_checkpoint_ingested(&self, checkpoint: IngestedCheckpointNotification);
}

#[derive(Debug, Clone)]
pub(super) struct CheckpointCommitInfo {
    pub(super) commit_sha: String,
    pub(super) commit_unix: i64,
    pub(super) author_name: String,
    pub(super) author_email: String,
    pub(super) subject: String,
}

#[derive(Debug, Clone)]
pub(super) struct FileArtefactRow {
    pub(super) artefact_id: String,
    pub(super) symbol_id: String,
    pub(super) language: String,
    pub(super) end_line: i32,
    pub(super) end_byte: i32,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(super) struct FunctionArtefact {
    pub(super) name: String,
    pub(super) start_line: i32,
    pub(super) end_line: i32,
    pub(super) start_byte: i32,
    pub(super) end_byte: i32,
    pub(super) signature: String,
}
