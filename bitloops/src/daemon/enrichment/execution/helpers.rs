use std::collections::BTreeSet;

use anyhow::Result;

use crate::capability_packs::semantic_clones::{
    clear_repo_active_embedding_setup, clear_repo_symbol_embedding_rows,
};
use crate::host::devql::RelationalStorage;

use super::SemanticFeatureInput;

pub(crate) async fn clear_embedding_outputs(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<()> {
    clear_repo_symbol_embedding_rows(relational, repo_id).await?;
    clear_repo_active_embedding_setup(relational, repo_id).await?;
    crate::capability_packs::semantic_clones::pipeline::delete_repo_symbol_clone_edges(
        relational, repo_id,
    )
    .await
}

pub(crate) fn dedupe_inputs_by_artefact_id(inputs: &mut Vec<SemanticFeatureInput>) {
    let mut seen = BTreeSet::new();
    inputs.retain(|input| seen.insert(input.artefact_id.clone()));
}

pub(crate) fn payload_artefact_ids_from_value(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Array(values) => values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}
