use anyhow::Result;

use crate::capability_packs::semantic_clones::scoring;
use crate::host::devql::{RelationalStorage, esc_pg, sql_json_value, sql_now};

use super::schema::{CloneProjection, ensure_semantic_clones_schema};

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
    let sql = format!(
        "DELETE FROM {} WHERE repo_id = '{}'",
        projection.clone_edges_table(),
        esc_pg(repo_id),
    );
    relational.exec(&sql).await
}

pub(super) async fn persist_symbol_clone_edges_for_projection(
    relational: &RelationalStorage,
    projection: CloneProjection,
    rows: &[scoring::SymbolCloneEdgeRow],
) -> Result<()> {
    for row in rows {
        let explanation_expr = sql_json_value(relational, &row.explanation_json);
        let generated_at = sql_now(relational);
        let sql = format!(
            "INSERT INTO {table} (repo_id, source_symbol_id, source_artefact_id, target_symbol_id, target_artefact_id, relation_kind, score, semantic_score, lexical_score, structural_score, clone_input_hash, explanation_json) \
VALUES ('{repo_id}', '{source_symbol_id}', '{source_artefact_id}', '{target_symbol_id}', '{target_artefact_id}', '{relation_kind}', {score}, {semantic_score}, {lexical_score}, {structural_score}, '{clone_input_hash}', {explanation_json}) \
ON CONFLICT (repo_id, source_symbol_id, target_symbol_id) DO UPDATE SET source_artefact_id = EXCLUDED.source_artefact_id, target_artefact_id = EXCLUDED.target_artefact_id, relation_kind = EXCLUDED.relation_kind, score = EXCLUDED.score, semantic_score = EXCLUDED.semantic_score, lexical_score = EXCLUDED.lexical_score, structural_score = EXCLUDED.structural_score, clone_input_hash = EXCLUDED.clone_input_hash, explanation_json = EXCLUDED.explanation_json, generated_at = {generated_at}",
            table = projection.clone_edges_table(),
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
            generated_at = generated_at,
        );
        relational.exec(&sql).await?;
    }
    Ok(())
}
