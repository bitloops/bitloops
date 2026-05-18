use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;

#[cfg(test)]
use crate::capability_packs::semantic_clones::{
    clear_repo_active_embedding_setup, clear_repo_symbol_embedding_rows,
};
use crate::capability_packs::semantic_clones::{
    load_semantic_feature_inputs_for_current_artefacts,
    load_semantic_feature_inputs_for_current_repo,
};
use crate::host::devql::RelationalStorage;
use crate::host::runtime_store::{
    SemanticEmbeddingMailboxItemRecord, SemanticMailboxItemKind, SemanticSummaryMailboxItemRecord,
};

use super::SemanticFeatureInput;

#[cfg(test)]
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

pub(crate) fn requested_current_semantic_input_artefact_ids<T>(
    items: &[T],
    repo_backfill_batch_size: usize,
) -> Vec<String>
where
    T: CurrentSemanticInputSelectionItem,
{
    let mut artefact_ids = Vec::new();
    for item in items {
        match item.item_kind() {
            SemanticMailboxItemKind::Artefact => {
                if let Some(artefact_id) = item.artefact_id() {
                    artefact_ids.push(artefact_id.to_string());
                }
            }
            SemanticMailboxItemKind::RepoBackfill => {
                if let Some(payload_json) = item.payload_json() {
                    artefact_ids.extend(
                        payload_artefact_ids_from_value(payload_json)
                            .into_iter()
                            .take(repo_backfill_batch_size),
                    );
                }
            }
        }
    }
    artefact_ids
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

#[cfg(test)]
mod tests {
    use super::{CurrentSemanticInputSelectionItem, requested_current_semantic_input_artefact_ids};
    use crate::host::runtime_store::{
        SemanticEmbeddingMailboxItemRecord, SemanticMailboxItemKind, SemanticMailboxItemStatus,
        SemanticSummaryMailboxItemRecord,
    };
    use std::path::PathBuf;

    fn summary_item(
        item_kind: SemanticMailboxItemKind,
        artefact_id: Option<&str>,
        payload_ids: Option<&[&str]>,
    ) -> SemanticSummaryMailboxItemRecord {
        SemanticSummaryMailboxItemRecord {
            item_id: "summary-item".to_string(),
            repo_id: "repo-1".to_string(),
            repo_root: PathBuf::from("/tmp/repo"),
            config_root: PathBuf::from("/tmp/config"),
            init_session_id: None,
            item_kind,
            artefact_id: artefact_id.map(str::to_string),
            payload_json: payload_ids.map(|ids| {
                serde_json::Value::Array(
                    ids.iter()
                        .map(|id| serde_json::Value::String((*id).to_string()))
                        .collect(),
                )
            }),
            dedupe_key: None,
            status: SemanticMailboxItemStatus::Pending,
            attempts: 0,
            available_at_unix: 0,
            submitted_at_unix: 0,
            leased_at_unix: None,
            lease_expires_at_unix: None,
            lease_token: None,
            updated_at_unix: 0,
            last_error: None,
        }
    }

    fn embedding_item(
        item_kind: SemanticMailboxItemKind,
        artefact_id: Option<&str>,
        payload_ids: Option<&[&str]>,
    ) -> SemanticEmbeddingMailboxItemRecord {
        SemanticEmbeddingMailboxItemRecord {
            item_id: "embedding-item".to_string(),
            repo_id: "repo-1".to_string(),
            repo_root: PathBuf::from("/tmp/repo"),
            config_root: PathBuf::from("/tmp/config"),
            init_session_id: None,
            representation_kind: "code".to_string(),
            item_kind,
            artefact_id: artefact_id.map(str::to_string),
            payload_json: payload_ids.map(|ids| {
                serde_json::Value::Array(
                    ids.iter()
                        .map(|id| serde_json::Value::String((*id).to_string()))
                        .collect(),
                )
            }),
            dedupe_key: None,
            status: SemanticMailboxItemStatus::Pending,
            attempts: 0,
            available_at_unix: 0,
            submitted_at_unix: 0,
            leased_at_unix: None,
            lease_expires_at_unix: None,
            lease_token: None,
            updated_at_unix: 0,
            last_error: None,
        }
    }

    fn assert_limited_ids<T>(items: &[T], batch_size: usize)
    where
        T: CurrentSemanticInputSelectionItem,
    {
        assert_eq!(
            requested_current_semantic_input_artefact_ids(items, batch_size),
            vec![
                "explicit-1".to_string(),
                "backfill-1".to_string(),
                "backfill-2".to_string(),
                "explicit-2".to_string(),
            ]
        );
    }

    #[test]
    fn requested_current_semantic_input_ids_limit_summary_backfill_payloads_to_batch_size() {
        let items = vec![
            summary_item(SemanticMailboxItemKind::Artefact, Some("explicit-1"), None),
            summary_item(
                SemanticMailboxItemKind::RepoBackfill,
                None,
                Some(&["backfill-1", "backfill-2", "backfill-3", "backfill-4"]),
            ),
            summary_item(SemanticMailboxItemKind::Artefact, Some("explicit-2"), None),
            summary_item(SemanticMailboxItemKind::RepoBackfill, None, None),
        ];

        assert_limited_ids(&items, 2);
    }

    #[test]
    fn requested_current_semantic_input_ids_limit_embedding_backfill_payloads_to_batch_size() {
        let items = vec![
            embedding_item(SemanticMailboxItemKind::Artefact, Some("explicit-1"), None),
            embedding_item(
                SemanticMailboxItemKind::RepoBackfill,
                None,
                Some(&["backfill-1", "backfill-2", "backfill-3", "backfill-4"]),
            ),
            embedding_item(SemanticMailboxItemKind::Artefact, Some("explicit-2"), None),
            embedding_item(SemanticMailboxItemKind::RepoBackfill, None, None),
        ];

        assert_limited_ids(&items, 2);
    }
}
