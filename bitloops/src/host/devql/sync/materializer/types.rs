use serde_json::Value;

#[derive(Debug, Clone)]
pub(crate) struct MaterializedArtefact {
    pub(crate) artifact_key: String,
    pub(crate) symbol_id: String,
    pub(crate) artefact_id: String,
    pub(crate) canonical_kind: Option<String>,
    pub(crate) language_kind: String,
    pub(crate) symbol_fqn: String,
    pub(crate) parent_symbol_id: Option<String>,
    pub(crate) parent_artefact_id: Option<String>,
    pub(crate) start_line: i32,
    pub(crate) end_line: i32,
    pub(crate) start_byte: i32,
    pub(crate) end_byte: i32,
    pub(crate) signature: Option<String>,
    pub(crate) modifiers: Vec<String>,
    pub(crate) docstring: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct MaterializedEdge {
    pub(crate) edge_id: String,
    pub(crate) from_symbol_id: String,
    pub(crate) from_artefact_id: String,
    pub(crate) to_symbol_id: Option<String>,
    pub(crate) to_artefact_id: Option<String>,
    pub(crate) to_symbol_ref: Option<String>,
    pub(crate) edge_kind: String,
    pub(crate) language: String,
    pub(crate) start_line: Option<i32>,
    pub(crate) end_line: Option<i32>,
    pub(crate) metadata: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedMaterialisationRows {
    pub(crate) materialized_artefacts: Vec<MaterializedArtefact>,
    pub(crate) materialized_edges: Vec<MaterializedEdge>,
}

impl PreparedMaterialisationRows {
    pub(crate) fn row_operation_estimate(&self) -> usize {
        3 + self.materialized_artefacts.len() + self.materialized_edges.len()
    }
}
