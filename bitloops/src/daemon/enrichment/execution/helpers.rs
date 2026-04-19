use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;

use crate::capability_packs::semantic_clones::{
    clear_repo_active_embedding_setup, clear_repo_symbol_embedding_rows,
    load_semantic_feature_inputs_for_current_artefacts,
    load_semantic_feature_inputs_for_current_repo,
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

pub(crate) async fn load_current_semantic_inputs(
    relational: &RelationalStorage,
    repo_root: &Path,
    repo_id: &str,
    artefact_ids: Option<&[String]>,
) -> Result<Vec<SemanticFeatureInput>> {
    match artefact_ids {
        Some(artefact_ids) => {
            let mut seen = BTreeSet::new();
            let unique_ids = artefact_ids
                .iter()
                .filter(|artefact_id| seen.insert((*artefact_id).clone()))
                .cloned()
                .collect::<Vec<_>>();
            load_semantic_feature_inputs_for_current_artefacts(
                relational,
                repo_root,
                repo_id,
                &unique_ids,
            )
            .await
        }
        None => load_semantic_feature_inputs_for_current_repo(relational, repo_root, repo_id).await,
    }
}
