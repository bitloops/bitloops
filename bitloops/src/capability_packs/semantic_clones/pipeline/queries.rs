use std::collections::{HashMap, HashSet};

use anyhow::Result;
use serde_json::Value;

use crate::capability_packs::semantic_clones::embeddings;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::devql::{
    EDGE_KIND_CALLS, EDGE_KIND_EXPORTS, RelationalStorage, esc_pg, sql_string_list_pg,
};

use super::parse::{parse_json_f32_array, value_as_usize};
use super::schema::CloneProjection;
use super::state::LoadedRepresentationEmbedding;

pub(super) async fn load_representation_embeddings_by_artefact_id(
    relational: &RelationalStorage,
    repo_id: &str,
    projection: CloneProjection,
    active_state: Option<&embeddings::ActiveEmbeddingRepresentationState>,
    artefact_ids: &[String],
) -> Result<HashMap<String, LoadedRepresentationEmbedding>> {
    let Some(active_state) = active_state else {
        return Ok(HashMap::new());
    };
    if artefact_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = relational
        .query_rows(&build_representation_embedding_lookup_sql(
            repo_id,
            projection,
            active_state,
            artefact_ids,
        ))
        .await?;
    let mut embeddings_by_artefact_id = HashMap::with_capacity(rows.len());
    for row in rows {
        let Some(artefact_id) = row.get("artefact_id").and_then(Value::as_str) else {
            continue;
        };
        let embedding = parse_json_f32_array(row.get("embedding"));
        if embedding.is_empty() {
            continue;
        }
        let setup = embeddings::EmbeddingSetup::new(
            row.get("embedding_provider")
                .and_then(Value::as_str)
                .unwrap_or(active_state.setup.provider.as_str()),
            row.get("embedding_model")
                .and_then(Value::as_str)
                .unwrap_or(active_state.setup.model.as_str()),
            row.get("embedding_dimension")
                .and_then(value_as_usize)
                .unwrap_or(active_state.setup.dimension),
        );
        embeddings_by_artefact_id.insert(
            artefact_id.to_string(),
            LoadedRepresentationEmbedding { setup, embedding },
        );
    }

    Ok(embeddings_by_artefact_id)
}

pub(super) async fn load_symbol_churn_counts(
    relational: &RelationalStorage,
    repo_id: &str,
    _projection: CloneProjection,
) -> Result<HashMap<String, usize>> {
    let sql = format!(
        "SELECT a.symbol_id, COUNT(DISTINCT s.blob_sha) AS churn_count \
FROM artefacts a \
JOIN artefact_snapshots s ON s.repo_id = a.repo_id AND s.artefact_id = a.artefact_id \
WHERE a.repo_id = '{}' AND a.symbol_id IS NOT NULL \
GROUP BY a.symbol_id",
        esc_pg(repo_id),
    );
    let rows = relational.query_rows(&sql).await?;
    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        let Some(symbol_id) = row.get("symbol_id").and_then(Value::as_str) else {
            continue;
        };
        let churn = row
            .get("churn_count")
            .and_then(value_as_usize)
            .unwrap_or_default();
        out.insert(symbol_id.to_string(), churn);
    }
    Ok(out)
}

pub(super) async fn load_symbol_call_targets(
    relational: &RelationalStorage,
    repo_id: &str,
    projection: CloneProjection,
) -> Result<HashMap<String, Vec<String>>> {
    let from_symbol_expr = projection.dependency_source_symbol_expr();
    let source_join = projection.dependency_source_join();
    let target_join = projection.dependency_target_join();
    let target_ref_expr = projection.dependency_target_ref_expr();
    let sql = format!(
        "SELECT {from_symbol_expr} AS from_symbol_id, {target_ref_expr} AS target_ref \
FROM {edges_table} e \
{source_join} \
{target_join} \
WHERE e.repo_id = '{}' AND e.edge_kind = '{}'",
        esc_pg(repo_id),
        esc_pg(EDGE_KIND_CALLS),
        edges_table = projection.dependency_edges_table(),
        from_symbol_expr = from_symbol_expr,
        source_join = source_join,
        target_join = target_join,
        target_ref_expr = target_ref_expr,
    );
    let rows = relational.query_rows(&sql).await?;
    let mut out = HashMap::<String, HashSet<String>>::new();
    for row in rows {
        let Some(from_symbol_id) = row.get("from_symbol_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(target_ref) = row.get("target_ref").and_then(Value::as_str) else {
            continue;
        };
        if target_ref.trim().is_empty() {
            continue;
        }
        out.entry(from_symbol_id.to_string())
            .or_default()
            .insert(target_ref.to_string());
    }

    Ok(out
        .into_iter()
        .map(|(symbol_id, targets)| {
            let mut targets = targets.into_iter().collect::<Vec<_>>();
            targets.sort();
            (symbol_id, targets)
        })
        .collect())
}

pub(super) async fn load_symbol_dependency_targets(
    relational: &RelationalStorage,
    repo_id: &str,
    projection: CloneProjection,
) -> Result<HashMap<String, Vec<String>>> {
    let from_symbol_expr = projection.dependency_source_symbol_expr();
    let source_join = projection.dependency_source_join();
    let target_join = projection.dependency_target_join();
    let target_ref_expr = projection.dependency_target_ref_expr();
    let sql = format!(
        "SELECT {from_symbol_expr} AS from_symbol_id, LOWER(e.edge_kind) AS edge_kind, \
{target_ref_expr} AS target_ref \
FROM {edges_table} e \
{source_join} \
{target_join} \
WHERE e.repo_id = '{}' AND e.edge_kind <> '{}' AND e.edge_kind <> '{}'",
        esc_pg(repo_id),
        esc_pg(EDGE_KIND_CALLS),
        esc_pg(EDGE_KIND_EXPORTS),
        edges_table = projection.dependency_edges_table(),
        from_symbol_expr = from_symbol_expr,
        source_join = source_join,
        target_join = target_join,
        target_ref_expr = target_ref_expr,
    );
    let rows = relational.query_rows(&sql).await?;
    let mut out = HashMap::<String, HashSet<String>>::new();
    for row in rows {
        let Some(from_symbol_id) = row.get("from_symbol_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(edge_kind) = row.get("edge_kind").and_then(Value::as_str) else {
            continue;
        };
        let Some(target_ref) = row.get("target_ref").and_then(Value::as_str) else {
            continue;
        };
        let Some(signal) = semantic::build_dependency_context_signal(edge_kind, target_ref) else {
            continue;
        };
        out.entry(from_symbol_id.to_string())
            .or_default()
            .insert(signal);
    }

    Ok(out
        .into_iter()
        .map(|(symbol_id, targets)| {
            let mut targets = targets.into_iter().collect::<Vec<_>>();
            targets.sort();
            (symbol_id, targets)
        })
        .collect())
}

pub(super) fn build_symbol_clone_candidate_lookup_sql(
    repo_id: &str,
    projection: CloneProjection,
    active_state: &embeddings::ActiveEmbeddingRepresentationState,
) -> String {
    let artefacts_table = projection.artefacts_table();
    let semantics_table = projection.semantics_table();
    let features_table = projection.features_table();
    let embeddings_table = projection.embeddings_table();
    let snapshot_column = projection.blob_column();
    let representation_predicate = representation_kind_sql_predicate(
        "e.representation_kind",
        active_state.representation_kind,
    );
    format!(
        "SELECT repo_id, symbol_id, artefact_id, path, canonical_kind, symbol_fqn, summary, \
normalized_name, normalized_signature, identifier_tokens, normalized_body_tokens, parent_kind, context_tokens, \
embedding_provider, embedding_model, embedding_dimension, embedding \
FROM ( \
SELECT a.repo_id, a.symbol_id, a.artefact_id, a.path, \
LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) AS canonical_kind, \
COALESCE(a.symbol_fqn, a.path) AS symbol_fqn, ss.summary, \
sf.normalized_name, sf.normalized_signature, sf.identifier_tokens, sf.normalized_body_tokens, sf.parent_kind, sf.context_tokens, \
e.provider AS embedding_provider, e.model AS embedding_model, e.dimension AS embedding_dimension, e.embedding, \
ROW_NUMBER() OVER ( \
    PARTITION BY e.artefact_id, e.setup_fingerprint \
    ORDER BY CASE e.representation_kind \
        WHEN 'code' THEN 0 \
        WHEN 'enriched' THEN 1 \
        WHEN 'baseline' THEN 2 \
        ELSE 3 \
    END \
) AS representation_rank \
FROM {embeddings_table} e \
JOIN {semantics_table} ss ON ss.repo_id = e.repo_id AND ss.artefact_id = e.artefact_id AND ss.{snapshot_column} = e.{snapshot_column} \
JOIN {features_table} sf ON sf.repo_id = e.repo_id AND sf.artefact_id = e.artefact_id AND sf.{snapshot_column} = e.{snapshot_column} \
JOIN {artefacts_table} a ON a.repo_id = e.repo_id AND a.artefact_id = e.artefact_id AND a.{snapshot_column} = e.{snapshot_column} \
WHERE e.repo_id = '{}' AND e.setup_fingerprint = '{}' AND {} \
) ranked \
WHERE representation_rank = 1 \
ORDER BY path, symbol_id",
        esc_pg(repo_id),
        esc_pg(&active_state.setup.setup_fingerprint),
        representation_predicate,
        artefacts_table = artefacts_table,
        semantics_table = semantics_table,
        features_table = features_table,
        embeddings_table = embeddings_table,
        snapshot_column = snapshot_column,
    )
}

pub(super) fn build_representation_embedding_lookup_sql(
    repo_id: &str,
    projection: CloneProjection,
    active_state: &embeddings::ActiveEmbeddingRepresentationState,
    artefact_ids: &[String],
) -> String {
    let embeddings_table = projection.embeddings_table();
    let representation_predicate = representation_kind_sql_predicate(
        "e.representation_kind",
        active_state.representation_kind,
    );
    format!(
        "SELECT e.artefact_id, e.provider AS embedding_provider, e.model AS embedding_model, \
e.dimension AS embedding_dimension, e.embedding \
FROM {embeddings_table} e \
WHERE e.repo_id = '{repo_id}' \
  AND e.setup_fingerprint = '{setup_fingerprint}' \
  AND {representation_predicate} \
  AND e.artefact_id IN ({artefact_ids}) \
ORDER BY e.artefact_id",
        embeddings_table = embeddings_table,
        repo_id = esc_pg(repo_id),
        setup_fingerprint = esc_pg(&active_state.setup.setup_fingerprint),
        representation_predicate = representation_predicate,
        artefact_ids = sql_string_list_pg(artefact_ids),
    )
}

fn representation_kind_sql_predicate(
    column: &str,
    kind: embeddings::EmbeddingRepresentationKind,
) -> String {
    let values = kind
        .storage_values()
        .iter()
        .map(|value| format!("'{}'", esc_pg(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{column} IN ({values})")
}
