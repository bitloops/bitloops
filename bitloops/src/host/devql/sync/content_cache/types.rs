use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CachedExtraction {
    pub(crate) content_id: String,
    pub(crate) language: String,
    pub(crate) extraction_fingerprint: String,
    pub(crate) parser_version: String,
    pub(crate) extractor_version: String,
    pub(crate) parse_status: String,
    pub(crate) artefacts: Vec<CachedArtefact>,
    pub(crate) edges: Vec<CachedEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CacheKey {
    pub(crate) content_id: String,
    pub(crate) language: String,
    pub(crate) extraction_fingerprint: String,
    pub(crate) parser_version: String,
    pub(crate) extractor_version: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CachedArtefact {
    pub(crate) artifact_key: String,
    pub(crate) canonical_kind: Option<String>,
    pub(crate) language_kind: String,
    pub(crate) name: String,
    pub(crate) parent_artifact_key: Option<String>,
    pub(crate) start_line: i32,
    pub(crate) end_line: i32,
    pub(crate) start_byte: i32,
    pub(crate) end_byte: i32,
    pub(crate) signature: String,
    pub(crate) modifiers: Vec<String>,
    pub(crate) docstring: Option<String>,
    pub(crate) metadata: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CachedEdge {
    pub(crate) edge_key: String,
    pub(crate) from_artifact_key: String,
    pub(crate) to_artifact_key: Option<String>,
    pub(crate) to_symbol_ref: Option<String>,
    pub(crate) edge_kind: String,
    pub(crate) start_line: Option<i32>,
    pub(crate) end_line: Option<i32>,
    pub(crate) metadata: Value,
}
