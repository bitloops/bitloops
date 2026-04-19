use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;

use crate::capability_packs::semantic_clones::{
    clear_repo_active_embedding_setup, clear_repo_symbol_embedding_rows,
    load_semantic_feature_inputs_for_current_artefacts,
    load_semantic_feature_inputs_for_current_repo,
};
use crate::host::devql::RelationalStorage;
use crate::host::runtime_store::{
    SemanticEmbeddingMailboxItemRecord, SemanticMailboxItemKind, SemanticSummaryMailboxItemRecord,
};

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

pub(crate) struct CurrentSemanticInputSelection {
    pub explicit_current_artefact_ids: Vec<String>,
    pub requires_full_current_inputs: bool,
}

impl CurrentSemanticInputSelection {
    pub(crate) fn requested_artefact_ids(&self) -> Option<&[String]> {
        if self.requires_full_current_inputs {
            None
        } else {
            Some(self.explicit_current_artefact_ids.as_slice())
        }
    }
}

pub(crate) trait CurrentSemanticInputSelectionItem {
    fn item_kind(&self) -> SemanticMailboxItemKind;
    fn artefact_id(&self) -> Option<&str>;
    fn payload_json(&self) -> Option<&serde_json::Value>;
}

impl CurrentSemanticInputSelectionItem for SemanticSummaryMailboxItemRecord {
    fn item_kind(&self) -> SemanticMailboxItemKind {
        self.item_kind
    }

    fn artefact_id(&self) -> Option<&str> {
        self.artefact_id.as_deref()
    }

    fn payload_json(&self) -> Option<&serde_json::Value> {
        self.payload_json.as_ref()
    }
}

impl CurrentSemanticInputSelectionItem for SemanticEmbeddingMailboxItemRecord {
    fn item_kind(&self) -> SemanticMailboxItemKind {
        self.item_kind
    }

    fn artefact_id(&self) -> Option<&str> {
        self.artefact_id.as_deref()
    }

    fn payload_json(&self) -> Option<&serde_json::Value> {
        self.payload_json.as_ref()
    }
}

pub(crate) fn select_current_semantic_input_scope<T>(items: &[T]) -> CurrentSemanticInputSelection
where
    T: CurrentSemanticInputSelectionItem,
{
    let mut explicit_current_artefact_ids = Vec::new();
    let mut requires_full_current_inputs = false;
    for item in items {
        match item.item_kind() {
            SemanticMailboxItemKind::Artefact => {
                if let Some(artefact_id) = item.artefact_id() {
                    explicit_current_artefact_ids.push(artefact_id.to_string());
                }
            }
            SemanticMailboxItemKind::RepoBackfill => {
                let requested_ids = item
                    .payload_json()
                    .map(payload_artefact_ids_from_value)
                    .unwrap_or_default();
                if requested_ids.is_empty() {
                    requires_full_current_inputs = true;
                } else {
                    explicit_current_artefact_ids.extend(requested_ids);
                }
            }
        }
    }
    CurrentSemanticInputSelection {
        explicit_current_artefact_ids,
        requires_full_current_inputs,
    }
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
