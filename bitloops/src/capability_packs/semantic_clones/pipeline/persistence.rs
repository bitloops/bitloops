use anyhow::Result;

use crate::capability_packs::semantic_clones::scoring;
use crate::host::devql::{RelationalStorage, esc_pg, sql_json_value, sql_now};

use super::schema::{CloneProjection, ensure_semantic_clones_schema};

const SYMBOL_CLONE_EDGE_UPSERT_CHUNK_SIZE: usize = 250;

pub(crate) async fn delete_repo_symbol_clone_edges(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<()> {
    delete_repo_symbol_clone_edges_for_projection(relational, repo_id, CloneProjection::Historical)
        .await
}

#[allow(dead_code)]
pub(crate) async fn delete_repo_current_symbol_clone_edges(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<()> {
    delete_repo_symbol_clone_edges_for_projection(relational, repo_id, CloneProjection::Current)
        .await
}

pub(super) async fn delete_repo_symbol_clone_edges_for_projection(
    relational: &RelationalStorage,
    repo_id: &str,
    projection: CloneProjection,
) -> Result<()> {
    ensure_semantic_clones_schema(relational).await?;
    let sql = build_delete_repo_symbol_clone_edges_sql(repo_id, projection);
    relational.exec(&sql).await
}

pub(super) async fn replace_repo_symbol_clone_edges_for_projection(
    relational: &RelationalStorage,
    repo_id: &str,
    projection: CloneProjection,
    rows: &[scoring::SymbolCloneEdgeRow],
) -> Result<()> {
    ensure_semantic_clones_schema(relational).await?;
    let mut statements = vec![build_delete_repo_symbol_clone_edges_sql(
        repo_id, projection,
    )];
    statements.extend(build_persist_symbol_clone_edge_statements(
        relational, projection, rows,
    ));
    relational.exec_batch_transactional(&statements).await
}

fn build_delete_repo_symbol_clone_edges_sql(repo_id: &str, projection: CloneProjection) -> String {
    format!(
        "DELETE FROM {} WHERE repo_id = '{}'",
        projection.clone_edges_table(),
        esc_pg(repo_id),
    )
}

fn build_persist_symbol_clone_edge_statements(
    relational: &RelationalStorage,
    projection: CloneProjection,
    rows: &[scoring::SymbolCloneEdgeRow],
) -> Vec<String> {
    rows.chunks(SYMBOL_CLONE_EDGE_UPSERT_CHUNK_SIZE)
        .map(|chunk| build_persist_symbol_clone_edge_statement(relational, projection, chunk))
        .collect()
}

fn build_persist_symbol_clone_edge_statement(
    relational: &RelationalStorage,
    projection: CloneProjection,
    rows: &[scoring::SymbolCloneEdgeRow],
) -> String {
    let generated_at = sql_now(relational);
    let values = rows
        .iter()
        .map(|row| build_symbol_clone_edge_values_sql(relational, row))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO {table} (repo_id, source_symbol_id, source_artefact_id, target_symbol_id, target_artefact_id, relation_kind, score, semantic_score, lexical_score, structural_score, clone_input_hash, explanation_json) \
VALUES {values} \
ON CONFLICT (repo_id, source_symbol_id, target_symbol_id) DO UPDATE SET source_artefact_id = EXCLUDED.source_artefact_id, target_artefact_id = EXCLUDED.target_artefact_id, relation_kind = EXCLUDED.relation_kind, score = EXCLUDED.score, semantic_score = EXCLUDED.semantic_score, lexical_score = EXCLUDED.lexical_score, structural_score = EXCLUDED.structural_score, clone_input_hash = EXCLUDED.clone_input_hash, explanation_json = EXCLUDED.explanation_json, generated_at = {generated_at}",
        table = projection.clone_edges_table(),
        values = values,
        generated_at = generated_at,
    )
}

fn build_symbol_clone_edge_values_sql(
    relational: &RelationalStorage,
    row: &scoring::SymbolCloneEdgeRow,
) -> String {
    let explanation_expr = sql_json_value(relational, &row.explanation_json);
    format!(
        "('{repo_id}', '{source_symbol_id}', '{source_artefact_id}', '{target_symbol_id}', '{target_artefact_id}', '{relation_kind}', {score}, {semantic_score}, {lexical_score}, {structural_score}, '{clone_input_hash}', {explanation_json})",
        repo_id = esc_pg(&row.repo_id),
        source_symbol_id = esc_pg(&row.source_symbol_id),
        source_artefact_id = esc_pg(&row.source_artefact_id),
        target_symbol_id = esc_pg(&row.target_symbol_id),
        target_artefact_id = esc_pg(&row.target_artefact_id),
        relation_kind = esc_pg(&row.relation_kind),
        score = row.score,
        semantic_score = row.semantic_score,
        lexical_score = row.lexical_score,
        structural_score = row.structural_score,
        clone_input_hash = esc_pg(&row.clone_input_hash),
        explanation_json = explanation_expr,
    )
}
