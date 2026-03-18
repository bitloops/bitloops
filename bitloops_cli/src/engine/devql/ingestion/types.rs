// Shared types used across ingestion modules.

#[derive(Debug, Clone, Default)]
struct IngestionCounters {
    checkpoints_processed: usize,
    events_inserted: usize,
    artefacts_upserted: usize,
    checkpoints_without_commit: usize,
    semantic_feature_rows_upserted: usize,
    semantic_feature_rows_skipped: usize,
    symbol_embedding_rows_upserted: usize,
    symbol_embedding_rows_skipped: usize,
}

#[derive(Debug, Clone)]
struct CheckpointCommitInfo {
    commit_sha: String,
    commit_unix: i64,
    author_name: String,
    author_email: String,
    subject: String,
}

#[derive(Debug, Clone)]
struct FileArtefactRow {
    artefact_id: String,
    symbol_id: String,
    language: String,
    end_line: i32,
    end_byte: i32,
}

#[cfg(test)]
#[derive(Debug, Clone)]
struct FunctionArtefact {
    name: String,
    start_line: i32,
    end_line: i32,
    start_byte: i32,
    end_byte: i32,
    signature: String,
}

#[derive(Debug, Clone)]
struct JsTsArtefact {
    canonical_kind: Option<String>,
    language_kind: String,
    name: String,
    symbol_fqn: String,
    parent_symbol_fqn: Option<String>,
    start_line: i32,
    end_line: i32,
    start_byte: i32,
    end_byte: i32,
    signature: String,
    modifiers: Vec<String>,
    docstring: Option<String>,
}

#[derive(Debug, Clone)]
struct JsTsDependencyEdge {
    edge_kind: String,
    from_symbol_fqn: String,
    to_target_symbol_fqn: Option<String>,
    to_symbol_ref: Option<String>,
    start_line: Option<i32>,
    end_line: Option<i32>,
    metadata: Value,
}

#[derive(Debug)]
struct RustUseExportEntry {
    path: String,
    export_name: String,
}
