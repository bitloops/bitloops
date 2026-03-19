// Shared record types for artefact persistence.

struct FileRevision<'a> {
    commit_sha: &'a str,
    revision: RevisionRef<'a>,
    commit_unix: i64,
    path: &'a str,
    blob_sha: &'a str,
}

#[derive(Debug, Clone)]
struct RevisionRef<'a> {
    kind: &'a str,
    id: &'a str,
    temp_checkpoint_id: Option<i64>,
}

#[derive(Debug, Clone)]
struct CurrentFileRevisionRecord {
    revision_kind: String,
    revision_id: String,
    blob_sha: String,
    updated_at_unix: i64,
}

#[derive(Debug, Clone)]
struct PersistedArtefactRecord {
    symbol_id: String,
    artefact_id: String,
    canonical_kind: Option<String>,
    language_kind: String,
    symbol_fqn: String,
    parent_symbol_id: Option<String>,
    parent_artefact_id: Option<String>,
    start_line: i32,
    end_line: i32,
    start_byte: i32,
    end_byte: i32,
    signature: Option<String>,
    modifiers: Vec<String>,
    docstring: Option<String>,
    content_hash: String,
}

#[derive(Debug, Clone)]
struct PersistedEdgeRecord {
    edge_id: String,
    from_symbol_id: String,
    from_artefact_id: String,
    to_symbol_id: Option<String>,
    to_artefact_id: Option<String>,
    to_symbol_ref: Option<String>,
    edge_kind: String,
    language: String,
    start_line: Option<i32>,
    end_line: Option<i32>,
    metadata: Value,
}

#[derive(Debug, Clone)]
struct CurrentArtefactStateRecord {
    record: PersistedArtefactRecord,
    symbol_id: String,
    symbol_fqn: String,
}

#[derive(Debug, Clone)]
struct CurrentEdgeStateRecord {
    edge_id: String,
    record: PersistedEdgeRecord,
}
