use super::*;

// Shared record types for artefact persistence.

pub(super) struct FileRevision<'a> {
    pub(super) commit_sha: &'a str,
    pub(super) revision: TemporalRevisionRef<'a>,
    pub(super) commit_unix: i64,
    pub(super) path: &'a str,
    pub(super) blob_sha: &'a str,
}

#[derive(Debug, Clone)]
pub(super) struct CurrentFileRevisionRecord {
    pub(super) revision_kind: TemporalRevisionKind,
    pub(super) revision_id: String,
    pub(super) blob_sha: String,
    pub(super) updated_at_unix: i64,
}

#[derive(Debug, Clone)]
pub(super) struct PersistedArtefactRecord {
    pub(super) symbol_id: String,
    pub(super) artefact_id: String,
    pub(super) canonical_kind: Option<String>,
    pub(super) language_kind: String,
    pub(super) symbol_fqn: String,
    pub(super) parent_symbol_id: Option<String>,
    pub(super) parent_artefact_id: Option<String>,
    pub(super) start_line: i32,
    pub(super) end_line: i32,
    pub(super) start_byte: i32,
    pub(super) end_byte: i32,
    pub(super) signature: Option<String>,
    pub(super) modifiers: Vec<String>,
    pub(super) docstring: Option<String>,
    pub(super) content_hash: String,
}

#[derive(Debug, Clone)]
pub(super) struct PersistedEdgeRecord {
    pub(super) edge_id: String,
    pub(super) from_symbol_id: String,
    pub(super) from_artefact_id: String,
    pub(super) to_symbol_id: Option<String>,
    pub(super) to_artefact_id: Option<String>,
    pub(super) to_symbol_ref: Option<String>,
    pub(super) edge_kind: String,
    pub(super) language: String,
    pub(super) start_line: Option<i32>,
    pub(super) end_line: Option<i32>,
    pub(super) metadata: Value,
}

#[derive(Debug, Clone)]
pub(super) struct CurrentArtefactStateRecord {
    pub(super) record: PersistedArtefactRecord,
    pub(super) symbol_id: String,
    pub(super) symbol_fqn: String,
}

#[derive(Debug, Clone)]
pub(super) struct CurrentEdgeStateRecord {
    pub(super) edge_id: String,
    pub(super) record: PersistedEdgeRecord,
}
