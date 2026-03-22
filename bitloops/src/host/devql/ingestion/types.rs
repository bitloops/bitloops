use super::*;

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

#[derive(Debug, Clone)]
pub(super) struct JsTsArtefact {
    pub(super) canonical_kind: Option<String>,
    pub(super) language_kind: String,
    pub(super) name: String,
    pub(super) symbol_fqn: String,
    pub(super) parent_symbol_fqn: Option<String>,
    pub(super) start_line: i32,
    pub(super) end_line: i32,
    pub(super) start_byte: i32,
    pub(super) end_byte: i32,
    pub(super) signature: String,
    pub(super) modifiers: Vec<String>,
    pub(super) docstring: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct EdgeMetadata(pub(super) Value);

impl EdgeMetadata {
    pub(super) fn none() -> Self {
        Self(json!({}))
    }

    pub(super) fn import(import_form: ImportForm) -> Self {
        Self(json!({
            "import_form": import_form.as_str(),
        }))
    }

    pub(super) fn call(call_form: CallForm, resolution: Resolution) -> Self {
        Self(json!({
            "call_form": call_form.as_str(),
            "resolution": resolution.as_str(),
        }))
    }

    pub(super) fn reference(ref_kind: RefKind, resolution: Resolution) -> Self {
        Self(json!({
            "ref_kind": ref_kind.as_str(),
            "resolution": resolution.as_str(),
        }))
    }

    pub(super) fn export(
        export_name: String,
        export_form: ExportForm,
        resolution: Resolution,
    ) -> Self {
        Self(json!({
            "export_name": export_name,
            "export_form": export_form.as_str(),
            "resolution": resolution.as_str(),
        }))
    }

    pub(super) fn to_value(&self) -> Value {
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
pub(super) struct JsTsDependencyEdge {
    pub(super) edge_kind: EdgeKind,
    pub(super) from_symbol_fqn: String,
    pub(super) to_target_symbol_fqn: Option<String>,
    pub(super) to_symbol_ref: Option<String>,
    pub(super) start_line: Option<i32>,
    pub(super) end_line: Option<i32>,
    pub(super) metadata: EdgeMetadata,
}

#[derive(Debug)]
pub(super) struct RustUseExportEntry {
    pub(super) path: String,
    pub(super) export_name: String,
}
