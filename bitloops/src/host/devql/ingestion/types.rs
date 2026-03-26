// Shared types used across ingestion modules.

#[derive(Debug, Clone, Default)]
pub(super) struct IngestionCounters {
    pub(super) checkpoints_processed: usize,
    pub(super) events_inserted: usize,
    pub(super) artefacts_upserted: usize,
    pub(super) checkpoints_without_commit: usize,
    pub(super) temporary_rows_promoted: usize,
    pub(super) semantic_feature_rows_upserted: usize,
    pub(super) semantic_feature_rows_skipped: usize,
    pub(super) symbol_embedding_rows_upserted: usize,
    pub(super) symbol_embedding_rows_skipped: usize,
    pub(super) symbol_clone_edges_upserted: usize,
    pub(super) symbol_clone_sources_scored: usize,
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
