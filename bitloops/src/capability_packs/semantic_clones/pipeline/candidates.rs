use anyhow::Result;
use serde_json::Value;

use crate::capability_packs::semantic_clones::{embeddings, scoring};
use crate::host::devql::RelationalStorage;

use super::parse::{parse_clone_json_string_array, parse_json_f32_array, value_as_usize};
use super::queries::{
    build_symbol_clone_candidate_lookup_sql, load_representation_embeddings_by_artefact_id,
    load_symbol_call_targets, load_symbol_churn_counts, load_symbol_dependency_targets,
};
use super::schema::CloneProjection;
use super::state::ActiveCloneEmbeddingStates;

pub(super) async fn load_symbol_clone_candidate_inputs(
    relational: &RelationalStorage,
    repo_id: &str,
    projection: CloneProjection,
    active_states: &ActiveCloneEmbeddingStates,
) -> Result<Vec<scoring::SymbolCloneCandidateInput>> {
    let churn_by_symbol_id = load_symbol_churn_counts(relational, repo_id, projection).await?;
    let call_targets_by_symbol_id =
        load_symbol_call_targets(relational, repo_id, projection).await?;
    let dependency_targets_by_symbol_id =
        load_symbol_dependency_targets(relational, repo_id, projection).await?;
    let rows = relational
        .query_rows(&build_symbol_clone_candidate_lookup_sql(
            repo_id,
            projection,
            &active_states.code,
        ))
        .await?;
    let artefact_ids = rows
        .iter()
        .filter_map(|row| {
            row.get("artefact_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    let summary_embeddings_by_artefact_id = load_representation_embeddings_by_artefact_id(
        relational,
        repo_id,
        projection,
        active_states.summary.as_ref(),
        &artefact_ids,
    )
    .await?;

    let mut candidates = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(symbol_id) = row.get("symbol_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(artefact_id) = row.get("artefact_id").and_then(Value::as_str) else {
            continue;
        };
        let embedding = parse_json_f32_array(row.get("embedding"));
        if embedding.is_empty() {
            continue;
        }

        candidates.push(scoring::SymbolCloneCandidateInput {
            repo_id: row
                .get("repo_id")
                .and_then(Value::as_str)
                .unwrap_or(repo_id)
                .to_string(),
            symbol_id: symbol_id.to_string(),
            artefact_id: artefact_id.to_string(),
            path: row
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            canonical_kind: row
                .get("canonical_kind")
                .and_then(Value::as_str)
                .unwrap_or("symbol")
                .to_string(),
            symbol_fqn: row
                .get("symbol_fqn")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            summary: row
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            normalized_name: row
                .get("normalized_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            normalized_signature: row
                .get("normalized_signature")
                .and_then(Value::as_str)
                .map(str::to_string),
            identifier_tokens: parse_clone_json_string_array(row.get("identifier_tokens")),
            normalized_body_tokens: parse_clone_json_string_array(
                row.get("normalized_body_tokens"),
            ),
            parent_kind: row
                .get("parent_kind")
                .and_then(Value::as_str)
                .map(str::to_string),
            context_tokens: parse_clone_json_string_array(row.get("context_tokens")),
            embedding_setup: embeddings::EmbeddingSetup::new(
                row.get("embedding_provider")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                row.get("embedding_model")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                row.get("embedding_dimension")
                    .and_then(value_as_usize)
                    .unwrap_or(active_states.code.setup.dimension),
            ),
            embedding,
            summary_embedding_setup: summary_embeddings_by_artefact_id
                .get(artefact_id)
                .map(|embedding| embedding.setup.clone()),
            summary_embedding: summary_embeddings_by_artefact_id
                .get(artefact_id)
                .map(|embedding| embedding.embedding.clone())
                .unwrap_or_default(),
            call_targets: call_targets_by_symbol_id
                .get(symbol_id)
                .cloned()
                .unwrap_or_default(),
            dependency_targets: dependency_targets_by_symbol_id
                .get(symbol_id)
                .cloned()
                .unwrap_or_default(),
            churn_count: churn_by_symbol_id.get(symbol_id).copied().unwrap_or(0),
        });
    }

    Ok(candidates)
}
