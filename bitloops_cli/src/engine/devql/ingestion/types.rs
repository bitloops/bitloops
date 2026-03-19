// Shared types used across ingestion modules.

#[derive(Debug, Clone, Default)]
struct IngestionCounters {
    checkpoints_processed: usize,
    events_inserted: usize,
    artefacts_upserted: usize,
    checkpoints_without_commit: usize,
    temporary_rows_promoted: usize,
    semantic_feature_rows_upserted: usize,
    semantic_feature_rows_skipped: usize,
    symbol_embedding_rows_upserted: usize,
    symbol_embedding_rows_skipped: usize,
    symbol_clone_edges_upserted: usize,
    symbol_clone_sources_scored: usize,
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
struct EdgeMetadata(Value);

impl EdgeMetadata {
    fn none() -> Self {
        Self(json!({}))
    }

    fn import(import_form: ImportForm) -> Self {
        Self(json!({
            "import_form": import_form.as_str(),
        }))
    }

    fn call(call_form: CallForm, resolution: Resolution) -> Self {
        Self(json!({
            "call_form": call_form.as_str(),
            "resolution": resolution.as_str(),
        }))
    }

    fn reference(ref_kind: RefKind, resolution: Resolution) -> Self {
        Self(json!({
            "ref_kind": ref_kind.as_str(),
            "resolution": resolution.as_str(),
        }))
    }

    fn export(export_name: String, export_form: ExportForm, resolution: Resolution) -> Self {
        Self(json!({
            "export_name": export_name,
            "export_form": export_form.as_str(),
            "resolution": resolution.as_str(),
        }))
    }

    fn to_value(&self) -> Value {
        self.0.clone()
    }
}

impl std::ops::Deref for EdgeMetadata {
    type Target = Value;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Clone)]
struct JsTsDependencyEdge {
    edge_kind: EdgeKind,
    from_symbol_fqn: String,
    to_target_symbol_fqn: Option<String>,
    to_symbol_ref: Option<String>,
    start_line: Option<i32>,
    end_line: Option<i32>,
    metadata: EdgeMetadata,
}

#[derive(Debug)]
struct RustUseExportEntry {
    path: String,
    export_name: String,
}
