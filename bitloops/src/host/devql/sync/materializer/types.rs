use serde_json::Value;

#[derive(Debug, Clone)]
pub(super) struct MaterializedArtefact {
    pub(super) artifact_key: String,
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
}

#[derive(Debug, Clone)]
pub(super) struct MaterializedEdge {
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
pub(crate) struct PreparedMaterialisationRows {
    pub(super) materialized_artefacts: Vec<MaterializedArtefact>,
    pub(super) materialized_edges: Vec<MaterializedEdge>,
}

impl PreparedMaterialisationRows {
    pub(crate) fn row_operation_estimate(&self) -> usize {
        3 + self.materialized_artefacts.len() + self.materialized_edges.len()
    }
}
