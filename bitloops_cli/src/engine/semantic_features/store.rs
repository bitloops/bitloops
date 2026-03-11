use anyhow::Result;
use serde_json::Value;

use crate::engine::infra::postgres::{esc_pg, pg_query_rows, postgres_exec};

use super::{
    PreStageArtefactRow, SemanticFeatureIndexState, SemanticFeatureRows,
};

pub(super) async fn get_artefacts(
    pg_client: &tokio_postgres::Client,
    repo_id: &str,
    blob_sha: &str,
    path: &str,
) -> Result<Vec<PreStageArtefactRow>> {
    let sql = format!(
        "SELECT artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash \
FROM artefacts \
WHERE repo_id = '{repo_id}' AND blob_sha = '{blob_sha}' AND path = '{path}' \
ORDER BY coalesce(start_byte, 0), coalesce(start_line, 0), artefact_id",
        repo_id = esc_pg(repo_id),
        blob_sha = esc_pg(blob_sha),
        path = esc_pg(path),
    );

    let rows = pg_query_rows(pg_client, &sql).await?;
    let mut artefacts = Vec::new();
    for row in rows {
        artefacts.push(serde_json::from_value::<PreStageArtefactRow>(row)?);
    }
    Ok(artefacts)
}

pub(super) async fn get_index_state(
    pg_client: &tokio_postgres::Client,
    artefact_id: &str,
) -> Result<SemanticFeatureIndexState> {
    let sql = format!(
        "SELECT \
            (SELECT semantic_features_input_hash FROM symbol_semantics WHERE artefact_id = '{artefact_id}') AS semantics_hash, \
            (SELECT prompt_version FROM symbol_semantics WHERE artefact_id = '{artefact_id}') AS semantics_prompt_version, \
            (SELECT semantic_features_input_hash FROM symbol_features WHERE artefact_id = '{artefact_id}') AS features_hash, \
            (SELECT prompt_version FROM symbol_features WHERE artefact_id = '{artefact_id}') AS features_prompt_version",
        artefact_id = esc_pg(artefact_id),
    );

    let rows = pg_query_rows(pg_client, &sql).await?;
    let Some(row) = rows.first() else {
        return Ok(SemanticFeatureIndexState::default());
    };

    Ok(SemanticFeatureIndexState {
        semantics_hash: row
            .get("semantics_hash")
            .and_then(Value::as_str)
            .map(str::to_string),
        semantics_prompt_version: row
            .get("semantics_prompt_version")
            .and_then(Value::as_str)
            .map(str::to_string),
        features_hash: row
            .get("features_hash")
            .and_then(Value::as_str)
            .map(str::to_string),
        features_prompt_version: row
            .get("features_prompt_version")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

pub(super) async fn persist_rows(
    pg_client: &tokio_postgres::Client,
    rows: &SemanticFeatureRows,
) -> Result<()> {
    let semantics = &rows.semantics;
    let features = &rows.features;

    let doc_comment_summary_expr = match semantics.doc_comment_summary.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let llm_summary_expr = match semantics.llm_summary.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let source_model_expr = match semantics.source_model.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let normalized_signature_expr = match features.normalized_signature.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let parent_kind_expr = match features.parent_kind.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let parent_symbol_expr = match features.parent_symbol.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let parameter_count_expr = match features.parameter_count {
        Some(value) => value.to_string(),
        None => "NULL".to_string(),
    };
    let return_shape_expr = match features.return_shape_hint.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let identifier_tokens = format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(&features.identifier_tokens)?)
    );
    let body_tokens = format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(&features.normalized_body_tokens)?)
    );
    let modifiers = format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(&features.modifiers)?)
    );
    let local_relationships = format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(&features.local_relationships)?)
    );
    let context_tokens = format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(&features.context_tokens)?)
    );

    let sql = format!(
        "INSERT INTO symbol_semantics (artefact_id, repo_id, blob_sha, semantic_features_input_hash, prompt_version, doc_comment_summary, llm_summary, template_summary, summary, confidence, summary_source, source_model) \
VALUES ('{artefact_id}', '{repo_id}', '{blob_sha}', '{input_hash}', '{prompt_version}', {doc_comment_summary}, {llm_summary}, '{template_summary}', '{summary}', {confidence:.4}, '{summary_source}', {source_model}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, prompt_version = EXCLUDED.prompt_version, doc_comment_summary = EXCLUDED.doc_comment_summary, llm_summary = EXCLUDED.llm_summary, template_summary = EXCLUDED.template_summary, summary = EXCLUDED.summary, confidence = EXCLUDED.confidence, summary_source = EXCLUDED.summary_source, source_model = EXCLUDED.source_model, generated_at = now(); \
INSERT INTO symbol_features (artefact_id, repo_id, blob_sha, semantic_features_input_hash, prompt_version, normalized_name, normalized_signature, identifier_tokens, normalized_body_tokens, parent_kind, parent_symbol, parameter_count, return_shape_hint, modifiers, local_relationships, context_tokens) \
VALUES ('{features_artefact_id}', '{features_repo_id}', '{features_blob_sha}', '{features_input_hash}', '{features_prompt_version}', '{normalized_name}', {normalized_signature}, {identifier_tokens}, {body_tokens}, {parent_kind}, {parent_symbol}, {parameter_count}, {return_shape_hint}, {modifiers}, {local_relationships}, {context_tokens}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, prompt_version = EXCLUDED.prompt_version, normalized_name = EXCLUDED.normalized_name, normalized_signature = EXCLUDED.normalized_signature, identifier_tokens = EXCLUDED.identifier_tokens, normalized_body_tokens = EXCLUDED.normalized_body_tokens, parent_kind = EXCLUDED.parent_kind, parent_symbol = EXCLUDED.parent_symbol, parameter_count = EXCLUDED.parameter_count, return_shape_hint = EXCLUDED.return_shape_hint, modifiers = EXCLUDED.modifiers, local_relationships = EXCLUDED.local_relationships, context_tokens = EXCLUDED.context_tokens, generated_at = now()",
        artefact_id = esc_pg(&semantics.artefact_id),
        repo_id = esc_pg(&semantics.repo_id),
        blob_sha = esc_pg(&semantics.blob_sha),
        input_hash = esc_pg(&rows.semantic_features_input_hash),
        prompt_version = esc_pg(&semantics.prompt_version),
        doc_comment_summary = doc_comment_summary_expr,
        llm_summary = llm_summary_expr,
        template_summary = esc_pg(&semantics.template_summary),
        summary = esc_pg(&semantics.summary),
        confidence = semantics.confidence,
        summary_source = semantics.summary_source.as_str(),
        source_model = source_model_expr,
        features_artefact_id = esc_pg(&features.artefact_id),
        features_repo_id = esc_pg(&features.repo_id),
        features_blob_sha = esc_pg(&features.blob_sha),
        features_input_hash = esc_pg(&rows.semantic_features_input_hash),
        features_prompt_version = esc_pg(&features.prompt_version),
        normalized_name = esc_pg(&features.normalized_name),
        normalized_signature = normalized_signature_expr,
        identifier_tokens = identifier_tokens,
        body_tokens = body_tokens,
        parent_kind = parent_kind_expr,
        parent_symbol = parent_symbol_expr,
        parameter_count = parameter_count_expr,
        return_shape_hint = return_shape_expr,
        modifiers = modifiers,
        local_relationships = local_relationships,
        context_tokens = context_tokens,
    );

    postgres_exec(pg_client, &sql).await
}
