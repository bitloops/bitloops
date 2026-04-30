use anyhow::Result;
use serde_json::Value;

use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::devql::{RelationalDialect, esc_pg, sql_string_list_pg};

pub(crate) fn build_semantic_get_index_state_sql(artefact_id: &str) -> String {
    format!(
        "SELECT \
            (SELECT semantic_features_input_hash FROM symbol_semantics WHERE artefact_id = '{artefact_id}') AS semantics_hash, \
            (SELECT semantic_features_input_hash FROM symbol_features WHERE artefact_id = '{artefact_id}') AS features_hash, \
            CASE \
                WHEN EXISTS ( \
                    SELECT 1 \
                    FROM symbol_semantics \
                    WHERE artefact_id = '{artefact_id}' \
                      AND (TRIM(COALESCE(llm_summary, '')) <> '' OR TRIM(COALESCE(source_model, '')) <> '') \
                ) THEN 1 \
                ELSE 0 \
            END AS semantics_llm_enriched",
        artefact_id = esc_pg(artefact_id),
    )
}

pub(crate) fn build_delete_current_symbol_semantics_sql(repo_id: &str, path: &str) -> String {
    format!(
        "DELETE FROM symbol_semantics_current WHERE repo_id = '{}' AND path = '{}'",
        esc_pg(repo_id),
        esc_pg(path),
    )
}

pub(crate) fn build_delete_current_symbol_semantics_for_artefact_sql(
    repo_id: &str,
    artefact_id: &str,
) -> String {
    format!(
        "DELETE FROM symbol_semantics_current WHERE repo_id = '{}' AND artefact_id = '{}'",
        esc_pg(repo_id),
        esc_pg(artefact_id),
    )
}

#[allow(dead_code)]
pub(crate) fn build_delete_current_symbol_features_sql(repo_id: &str, path: &str) -> String {
    format!(
        "DELETE FROM symbol_features_current WHERE repo_id = '{}' AND path = '{}'",
        esc_pg(repo_id),
        esc_pg(path),
    )
}

pub(crate) fn build_delete_current_symbol_semantics_for_paths_sql(
    repo_id: &str,
    paths: &[String],
) -> Option<String> {
    if paths.is_empty() {
        return None;
    }
    Some(format!(
        "DELETE FROM symbol_semantics_current WHERE repo_id = '{}' AND path IN ({})",
        esc_pg(repo_id),
        sql_string_list_pg(paths),
    ))
}

pub(crate) fn build_delete_current_symbol_features_for_paths_sql(
    repo_id: &str,
    paths: &[String],
) -> Option<String> {
    if paths.is_empty() {
        return None;
    }
    Some(format!(
        "DELETE FROM symbol_features_current WHERE repo_id = '{}' AND path IN ({})",
        esc_pg(repo_id),
        sql_string_list_pg(paths),
    ))
}

pub(crate) fn parse_semantic_index_state_rows(
    rows: &[Value],
) -> semantic::SemanticFeatureIndexState {
    let Some(row) = rows.first() else {
        return semantic::SemanticFeatureIndexState::default();
    };

    semantic::SemanticFeatureIndexState {
        semantics_hash: row
            .get("semantics_hash")
            .and_then(Value::as_str)
            .map(str::to_string),
        features_hash: row
            .get("features_hash")
            .and_then(Value::as_str)
            .map(str::to_string),
        semantics_llm_enriched: row
            .get("semantics_llm_enriched")
            .map(value_as_boolish)
            .unwrap_or(false),
    }
}

fn value_as_boolish(value: &Value) -> bool {
    value.as_bool().unwrap_or_else(|| {
        value
            .as_i64()
            .map(|value| value != 0)
            .or_else(|| {
                value.as_str().map(|value| {
                    value.eq_ignore_ascii_case("true")
                        || value.eq_ignore_ascii_case("t")
                        || value == "1"
                })
            })
            .unwrap_or(false)
    })
}

fn semantic_generated_at_now_sql(dialect: RelationalDialect) -> &'static str {
    match dialect {
        RelationalDialect::Postgres => "now()",
        RelationalDialect::Sqlite => "datetime('now')",
    }
}

pub(crate) fn build_symbol_feature_persist_rows_sql(
    rows: &semantic::SemanticFeatureRows,
    dialect: RelationalDialect,
) -> Result<String> {
    let features = &rows.features;
    let normalized_signature_expr = sql_optional_string(features.normalized_signature.as_deref());
    let parent_kind_expr = sql_optional_string(features.parent_kind.as_deref());
    let modifiers_expr = sql_json_string_for_dialect(&features.modifiers, dialect)?;
    let identifier_tokens_expr = sql_json_string_for_dialect(&features.identifier_tokens, dialect)?;
    let body_tokens_expr = sql_json_string_for_dialect(&features.normalized_body_tokens, dialect)?;
    let context_tokens_expr = sql_json_string_for_dialect(&features.context_tokens, dialect)?;
    let generated_at_sql = semantic_generated_at_now_sql(dialect);

    Ok(format!(
        "INSERT INTO symbol_features (artefact_id, repo_id, blob_sha, semantic_features_input_hash, normalized_name, normalized_signature, modifiers, identifier_tokens, normalized_body_tokens, parent_kind, context_tokens) \
VALUES ('{features_artefact_id}', '{features_repo_id}', '{features_blob_sha}', '{features_input_hash}', '{normalized_name}', {normalized_signature}, {modifiers}, {identifier_tokens}, {body_tokens}, {parent_kind}, {context_tokens}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, normalized_name = EXCLUDED.normalized_name, normalized_signature = EXCLUDED.normalized_signature, modifiers = EXCLUDED.modifiers, identifier_tokens = EXCLUDED.identifier_tokens, normalized_body_tokens = EXCLUDED.normalized_body_tokens, parent_kind = EXCLUDED.parent_kind, context_tokens = EXCLUDED.context_tokens, generated_at = {generated_at}",
        features_artefact_id = esc_pg(&features.artefact_id),
        features_repo_id = esc_pg(&features.repo_id),
        features_blob_sha = esc_pg(&features.blob_sha),
        features_input_hash = esc_pg(&rows.semantic_features_input_hash),
        normalized_name = esc_pg(&features.normalized_name),
        normalized_signature = normalized_signature_expr,
        modifiers = modifiers_expr,
        identifier_tokens = identifier_tokens_expr,
        body_tokens = body_tokens_expr,
        parent_kind = parent_kind_expr,
        context_tokens = context_tokens_expr,
        generated_at = generated_at_sql,
    ))
}

pub(crate) fn build_current_symbol_feature_persist_rows_sql(
    rows: &semantic::SemanticFeatureRows,
    symbol_id: Option<&str>,
    path: &str,
    content_id: &str,
    dialect: RelationalDialect,
) -> Result<String> {
    let features = &rows.features;
    let symbol_id_expr = sql_optional_string(symbol_id);
    let normalized_signature_expr = sql_optional_string(features.normalized_signature.as_deref());
    let parent_kind_expr = sql_optional_string(features.parent_kind.as_deref());
    let modifiers_expr = sql_json_string_for_dialect(&features.modifiers, dialect)?;
    let identifier_tokens_expr = sql_json_string_for_dialect(&features.identifier_tokens, dialect)?;
    let body_tokens_expr = sql_json_string_for_dialect(&features.normalized_body_tokens, dialect)?;
    let context_tokens_expr = sql_json_string_for_dialect(&features.context_tokens, dialect)?;
    let generated_at_sql = semantic_generated_at_now_sql(dialect);

    Ok(format!(
        "INSERT INTO symbol_features_current (artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash, normalized_name, normalized_signature, modifiers, identifier_tokens, normalized_body_tokens, parent_kind, context_tokens) \
VALUES ('{features_artefact_id}', '{features_repo_id}', '{path}', '{content_id}', {symbol_id}, '{features_input_hash}', '{normalized_name}', {normalized_signature}, {modifiers}, {identifier_tokens}, {body_tokens}, {parent_kind}, {context_tokens}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, path = EXCLUDED.path, content_id = EXCLUDED.content_id, symbol_id = EXCLUDED.symbol_id, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, normalized_name = EXCLUDED.normalized_name, normalized_signature = EXCLUDED.normalized_signature, modifiers = EXCLUDED.modifiers, identifier_tokens = EXCLUDED.identifier_tokens, normalized_body_tokens = EXCLUDED.normalized_body_tokens, parent_kind = EXCLUDED.parent_kind, context_tokens = EXCLUDED.context_tokens, generated_at = {generated_at}",
        features_artefact_id = esc_pg(&features.artefact_id),
        features_repo_id = esc_pg(&features.repo_id),
        path = esc_pg(path),
        content_id = esc_pg(content_id),
        symbol_id = symbol_id_expr,
        features_input_hash = esc_pg(&rows.semantic_features_input_hash),
        normalized_name = esc_pg(&features.normalized_name),
        normalized_signature = normalized_signature_expr,
        modifiers = modifiers_expr,
        identifier_tokens = identifier_tokens_expr,
        body_tokens = body_tokens_expr,
        parent_kind = parent_kind_expr,
        context_tokens = context_tokens_expr,
        generated_at = generated_at_sql,
    ))
}

pub(crate) fn build_conditional_current_symbol_feature_persist_rows_sql(
    rows: &semantic::SemanticFeatureRows,
    input: &semantic::SemanticFeatureInput,
    dialect: RelationalDialect,
) -> Result<String> {
    let features = &rows.features;
    let normalized_signature_expr = sql_optional_string(features.normalized_signature.as_deref());
    let parent_kind_expr = sql_optional_string(features.parent_kind.as_deref());
    let modifiers_expr = sql_json_string_for_dialect(&features.modifiers, dialect)?;
    let identifier_tokens_expr = sql_json_string_for_dialect(&features.identifier_tokens, dialect)?;
    let body_tokens_expr = sql_json_string_for_dialect(&features.normalized_body_tokens, dialect)?;
    let context_tokens_expr = sql_json_string_for_dialect(&features.context_tokens, dialect)?;
    let generated_at_sql = semantic_generated_at_now_sql(dialect);
    let target_select = build_current_semantic_target_select_sql(input);

    Ok(format!(
        "INSERT INTO symbol_features_current (artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash, normalized_name, normalized_signature, modifiers, identifier_tokens, normalized_body_tokens, parent_kind, context_tokens) \
SELECT target.artefact_id, target.repo_id, target.path, target.content_id, target.symbol_id, '{features_input_hash}', '{normalized_name}', {normalized_signature}, {modifiers}, {identifier_tokens}, {body_tokens}, {parent_kind}, {context_tokens} \
FROM ({target_select}) target \
WHERE 1 = 1 \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, path = EXCLUDED.path, content_id = EXCLUDED.content_id, symbol_id = EXCLUDED.symbol_id, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, normalized_name = EXCLUDED.normalized_name, normalized_signature = EXCLUDED.normalized_signature, modifiers = EXCLUDED.modifiers, identifier_tokens = EXCLUDED.identifier_tokens, normalized_body_tokens = EXCLUDED.normalized_body_tokens, parent_kind = EXCLUDED.parent_kind, context_tokens = EXCLUDED.context_tokens, generated_at = {generated_at}",
        target_select = target_select,
        features_input_hash = esc_pg(&rows.semantic_features_input_hash),
        normalized_name = esc_pg(&features.normalized_name),
        normalized_signature = normalized_signature_expr,
        modifiers = modifiers_expr,
        identifier_tokens = identifier_tokens_expr,
        body_tokens = body_tokens_expr,
        parent_kind = parent_kind_expr,
        context_tokens = context_tokens_expr,
        generated_at = generated_at_sql,
    ))
}

pub(crate) fn build_semantic_persist_rows_sql(
    rows: &semantic::SemanticFeatureRows,
    dialect: RelationalDialect,
) -> Result<String> {
    let semantics = &rows.semantics;
    let features = &rows.features;

    let normalized_signature_expr = sql_optional_string(features.normalized_signature.as_deref());
    let parent_kind_expr = sql_optional_string(features.parent_kind.as_deref());
    let modifiers_expr = sql_json_string_for_dialect(&features.modifiers, dialect)?;
    let identifier_tokens_expr = sql_json_string_for_dialect(&features.identifier_tokens, dialect)?;
    let body_tokens_expr = sql_json_string_for_dialect(&features.normalized_body_tokens, dialect)?;
    let context_tokens_expr = sql_json_string_for_dialect(&features.context_tokens, dialect)?;
    let generated_at_sql = semantic_generated_at_now_sql(dialect);

    Ok(format!(
        "{persist_summary_sql}; \
INSERT INTO symbol_features (artefact_id, repo_id, blob_sha, semantic_features_input_hash, normalized_name, normalized_signature, modifiers, identifier_tokens, normalized_body_tokens, parent_kind, context_tokens) \
VALUES ('{features_artefact_id}', '{features_repo_id}', '{features_blob_sha}', '{features_input_hash}', '{normalized_name}', {normalized_signature}, {modifiers}, {identifier_tokens}, {body_tokens}, {parent_kind}, {context_tokens}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, normalized_name = EXCLUDED.normalized_name, normalized_signature = EXCLUDED.normalized_signature, modifiers = EXCLUDED.modifiers, identifier_tokens = EXCLUDED.identifier_tokens, normalized_body_tokens = EXCLUDED.normalized_body_tokens, parent_kind = EXCLUDED.parent_kind, context_tokens = EXCLUDED.context_tokens, generated_at = {generated_at}",
        persist_summary_sql = build_semantic_persist_summary_sql(
            semantics,
            &rows.semantic_features_input_hash,
            dialect,
        )?,
        features_artefact_id = esc_pg(&features.artefact_id),
        features_repo_id = esc_pg(&features.repo_id),
        features_blob_sha = esc_pg(&features.blob_sha),
        features_input_hash = esc_pg(&rows.semantic_features_input_hash),
        normalized_name = esc_pg(&features.normalized_name),
        normalized_signature = normalized_signature_expr,
        modifiers = modifiers_expr,
        identifier_tokens = identifier_tokens_expr,
        body_tokens = body_tokens_expr,
        parent_kind = parent_kind_expr,
        context_tokens = context_tokens_expr,
        generated_at = generated_at_sql,
    ))
}

#[allow(dead_code)]
pub(crate) fn build_current_semantic_persist_rows_sql(
    rows: &semantic::SemanticFeatureRows,
    symbol_id: Option<&str>,
    path: &str,
    content_id: &str,
    dialect: RelationalDialect,
) -> Result<String> {
    let semantics = &rows.semantics;
    let features = &rows.features;
    let symbol_id_expr = sql_optional_string(symbol_id);
    let normalized_signature_expr = sql_optional_string(features.normalized_signature.as_deref());
    let parent_kind_expr = sql_optional_string(features.parent_kind.as_deref());
    let modifiers_expr = sql_json_string_for_dialect(&features.modifiers, dialect)?;
    let identifier_tokens_expr = sql_json_string_for_dialect(&features.identifier_tokens, dialect)?;
    let body_tokens_expr = sql_json_string_for_dialect(&features.normalized_body_tokens, dialect)?;
    let context_tokens_expr = sql_json_string_for_dialect(&features.context_tokens, dialect)?;
    let generated_at_sql = semantic_generated_at_now_sql(dialect);

    Ok(format!(
        "{persist_summary_sql}; \
INSERT INTO symbol_features_current (artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash, normalized_name, normalized_signature, modifiers, identifier_tokens, normalized_body_tokens, parent_kind, context_tokens) \
VALUES ('{features_artefact_id}', '{features_repo_id}', '{path}', '{content_id}', {symbol_id}, '{features_input_hash}', '{normalized_name}', {normalized_signature}, {modifiers}, {identifier_tokens}, {body_tokens}, {parent_kind}, {context_tokens}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, path = EXCLUDED.path, content_id = EXCLUDED.content_id, symbol_id = EXCLUDED.symbol_id, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, normalized_name = EXCLUDED.normalized_name, normalized_signature = EXCLUDED.normalized_signature, modifiers = EXCLUDED.modifiers, identifier_tokens = EXCLUDED.identifier_tokens, normalized_body_tokens = EXCLUDED.normalized_body_tokens, parent_kind = EXCLUDED.parent_kind, context_tokens = EXCLUDED.context_tokens, generated_at = {generated_at}",
        persist_summary_sql = build_current_semantic_persist_summary_sql(
            semantics,
            &rows.semantic_features_input_hash,
            symbol_id,
            path,
            content_id,
            dialect,
        )?,
        features_artefact_id = esc_pg(&features.artefact_id),
        features_repo_id = esc_pg(&features.repo_id),
        path = esc_pg(path),
        content_id = esc_pg(content_id),
        symbol_id = symbol_id_expr,
        features_input_hash = esc_pg(&rows.semantic_features_input_hash),
        normalized_name = esc_pg(&features.normalized_name),
        normalized_signature = normalized_signature_expr,
        modifiers = modifiers_expr,
        identifier_tokens = identifier_tokens_expr,
        body_tokens = body_tokens_expr,
        parent_kind = parent_kind_expr,
        context_tokens = context_tokens_expr,
        generated_at = generated_at_sql,
    ))
}

pub(crate) fn build_conditional_current_semantic_persist_rows_sql(
    rows: &semantic::SemanticFeatureRows,
    input: &semantic::SemanticFeatureInput,
    dialect: RelationalDialect,
) -> Result<String> {
    let features = &rows.features;
    let normalized_signature_expr = sql_optional_string(features.normalized_signature.as_deref());
    let parent_kind_expr = sql_optional_string(features.parent_kind.as_deref());
    let modifiers_expr = sql_json_string_for_dialect(&features.modifiers, dialect)?;
    let identifier_tokens_expr = sql_json_string_for_dialect(&features.identifier_tokens, dialect)?;
    let body_tokens_expr = sql_json_string_for_dialect(&features.normalized_body_tokens, dialect)?;
    let context_tokens_expr = sql_json_string_for_dialect(&features.context_tokens, dialect)?;
    let generated_at_sql = semantic_generated_at_now_sql(dialect);
    let target_select = build_current_semantic_target_select_sql(input);

    Ok(format!(
        "{persist_summary_sql}; \
INSERT INTO symbol_features_current (artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash, normalized_name, normalized_signature, modifiers, identifier_tokens, normalized_body_tokens, parent_kind, context_tokens) \
SELECT target.artefact_id, target.repo_id, target.path, target.content_id, target.symbol_id, '{features_input_hash}', '{normalized_name}', {normalized_signature}, {modifiers}, {identifier_tokens}, {body_tokens}, {parent_kind}, {context_tokens} \
FROM ({target_select}) target \
WHERE 1 = 1 \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, path = EXCLUDED.path, content_id = EXCLUDED.content_id, symbol_id = EXCLUDED.symbol_id, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, normalized_name = EXCLUDED.normalized_name, normalized_signature = EXCLUDED.normalized_signature, modifiers = EXCLUDED.modifiers, identifier_tokens = EXCLUDED.identifier_tokens, normalized_body_tokens = EXCLUDED.normalized_body_tokens, parent_kind = EXCLUDED.parent_kind, context_tokens = EXCLUDED.context_tokens, generated_at = {generated_at}",
        persist_summary_sql = build_conditional_current_semantic_persist_summary_sql(
            &rows.semantics,
            &rows.semantic_features_input_hash,
            input,
            dialect,
        )?,
        target_select = target_select,
        features_input_hash = esc_pg(&rows.semantic_features_input_hash),
        normalized_name = esc_pg(&features.normalized_name),
        normalized_signature = normalized_signature_expr,
        modifiers = modifiers_expr,
        identifier_tokens = identifier_tokens_expr,
        body_tokens = body_tokens_expr,
        parent_kind = parent_kind_expr,
        context_tokens = context_tokens_expr,
        generated_at = generated_at_sql,
    ))
}

pub(crate) fn build_repair_current_semantic_projection_from_historical_sql(
    repo_id: &str,
    artefact_ids: &[String],
    dialect: RelationalDialect,
) -> String {
    let repo_filter = format!("a.repo_id = '{}'", esc_pg(repo_id));
    let artefact_filter = if artefact_ids.is_empty() {
        String::new()
    } else {
        format!(
            " AND a.artefact_id IN ({})",
            artefact_ids
                .iter()
                .map(|artefact_id| format!("'{}'", esc_pg(artefact_id)))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    build_repair_current_semantic_projection_from_historical_sql_with_filter(
        &repo_filter,
        &artefact_filter,
        dialect,
    )
}

pub(crate) fn build_conditional_current_semantic_persist_existing_rows_sql(
    input: &semantic::SemanticFeatureInput,
    semantic_features_input_hash: &str,
    dialect: RelationalDialect,
) -> Result<String> {
    let generated_at_sql = semantic_generated_at_now_sql(dialect);
    let target_select_for_semantics = build_current_semantic_target_select_sql(input);
    let target_select_for_features = build_current_semantic_target_select_sql(input);

    Ok(format!(
        "INSERT INTO symbol_semantics_current (artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash, docstring_summary, llm_summary, template_summary, summary, confidence, source_model) \
SELECT target.artefact_id, target.repo_id, target.path, target.content_id, target.symbol_id, s.semantic_features_input_hash, s.docstring_summary, s.llm_summary, s.template_summary, s.summary, s.confidence, s.source_model \
FROM ({target_select_for_semantics}) target \
JOIN symbol_semantics s \
  ON s.repo_id = target.repo_id \
 AND s.artefact_id = '{artefact_id}' \
 AND s.blob_sha = target.content_id \
 AND s.semantic_features_input_hash = '{input_hash}' \
WHERE 1 = 1 \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, path = EXCLUDED.path, content_id = EXCLUDED.content_id, symbol_id = EXCLUDED.symbol_id, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, docstring_summary = EXCLUDED.docstring_summary, llm_summary = EXCLUDED.llm_summary, template_summary = EXCLUDED.template_summary, summary = EXCLUDED.summary, confidence = EXCLUDED.confidence, source_model = EXCLUDED.source_model, generated_at = {generated_at}; \
INSERT INTO symbol_features_current (artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash, normalized_name, normalized_signature, modifiers, identifier_tokens, normalized_body_tokens, parent_kind, context_tokens) \
SELECT target.artefact_id, target.repo_id, target.path, target.content_id, target.symbol_id, f.semantic_features_input_hash, f.normalized_name, f.normalized_signature, f.modifiers, f.identifier_tokens, f.normalized_body_tokens, f.parent_kind, f.context_tokens \
FROM ({target_select_for_features}) target \
JOIN symbol_features f \
  ON f.repo_id = target.repo_id \
 AND f.artefact_id = '{artefact_id}' \
 AND f.blob_sha = target.content_id \
 AND f.semantic_features_input_hash = '{input_hash}' \
WHERE 1 = 1 \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, path = EXCLUDED.path, content_id = EXCLUDED.content_id, symbol_id = EXCLUDED.symbol_id, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, normalized_name = EXCLUDED.normalized_name, normalized_signature = EXCLUDED.normalized_signature, modifiers = EXCLUDED.modifiers, identifier_tokens = EXCLUDED.identifier_tokens, normalized_body_tokens = EXCLUDED.normalized_body_tokens, parent_kind = EXCLUDED.parent_kind, context_tokens = EXCLUDED.context_tokens, generated_at = {generated_at}",
        target_select_for_semantics = target_select_for_semantics,
        target_select_for_features = target_select_for_features,
        artefact_id = esc_pg(&input.artefact_id),
        input_hash = esc_pg(semantic_features_input_hash),
        generated_at = generated_at_sql,
    ))
}

pub(crate) fn build_repair_all_current_semantic_projection_from_historical_sql(
    dialect: RelationalDialect,
) -> String {
    build_repair_current_semantic_projection_from_historical_sql_with_filter("1 = 1", "", dialect)
}

fn build_repair_current_semantic_projection_from_historical_sql_with_filter(
    repo_filter: &str,
    artefact_filter: &str,
    dialect: RelationalDialect,
) -> String {
    let generated_at_sql = semantic_generated_at_now_sql(dialect);
    format!(
        "INSERT INTO symbol_features_current (artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash, normalized_name, normalized_signature, modifiers, identifier_tokens, normalized_body_tokens, parent_kind, context_tokens) \
SELECT a.artefact_id, a.repo_id, a.path, a.content_id, a.symbol_id, f.semantic_features_input_hash, f.normalized_name, f.normalized_signature, f.modifiers, f.identifier_tokens, f.normalized_body_tokens, f.parent_kind, f.context_tokens \
FROM artefacts_current a \
JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path AND cfs.effective_content_id = a.content_id \
JOIN symbol_features f \
  ON f.repo_id = a.repo_id \
 AND f.artefact_id = a.artefact_id \
 AND f.blob_sha = a.content_id \
WHERE {repo_filter} \
  AND cfs.analysis_mode = 'code'{artefact_filter} \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, path = EXCLUDED.path, content_id = EXCLUDED.content_id, symbol_id = EXCLUDED.symbol_id, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, normalized_name = EXCLUDED.normalized_name, normalized_signature = EXCLUDED.normalized_signature, modifiers = EXCLUDED.modifiers, identifier_tokens = EXCLUDED.identifier_tokens, normalized_body_tokens = EXCLUDED.normalized_body_tokens, parent_kind = EXCLUDED.parent_kind, context_tokens = EXCLUDED.context_tokens, generated_at = {generated_at}; \
INSERT INTO symbol_semantics_current (artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash, docstring_summary, llm_summary, template_summary, summary, confidence, source_model) \
SELECT a.artefact_id, a.repo_id, a.path, a.content_id, a.symbol_id, s.semantic_features_input_hash, s.docstring_summary, s.llm_summary, s.template_summary, s.summary, s.confidence, s.source_model \
FROM artefacts_current a \
JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path AND cfs.effective_content_id = a.content_id \
JOIN symbol_features f \
  ON f.repo_id = a.repo_id \
 AND f.artefact_id = a.artefact_id \
 AND f.blob_sha = a.content_id \
JOIN symbol_semantics s \
  ON s.repo_id = f.repo_id \
 AND s.artefact_id = f.artefact_id \
 AND s.blob_sha = f.blob_sha \
 AND s.semantic_features_input_hash = f.semantic_features_input_hash \
WHERE {repo_filter} \
  AND cfs.analysis_mode = 'code'{artefact_filter} \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, path = EXCLUDED.path, content_id = EXCLUDED.content_id, symbol_id = EXCLUDED.symbol_id, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, docstring_summary = EXCLUDED.docstring_summary, llm_summary = EXCLUDED.llm_summary, template_summary = EXCLUDED.template_summary, summary = EXCLUDED.summary, confidence = EXCLUDED.confidence, source_model = EXCLUDED.source_model, generated_at = {generated_at}",
        repo_filter = repo_filter,
        artefact_filter = artefact_filter,
        generated_at = generated_at_sql,
    )
}

pub(crate) fn build_semantic_persist_summary_sql(
    semantics: &semantic::SymbolSemanticsRow,
    semantic_features_input_hash: &str,
    dialect: RelationalDialect,
) -> Result<String> {
    let docstring_summary_expr = sql_optional_string(semantics.docstring_summary.as_deref());
    let llm_summary_expr = sql_optional_string(semantics.llm_summary.as_deref());
    let source_model_expr = sql_optional_string(semantics.source_model.as_deref());
    let generated_at_sql = semantic_generated_at_now_sql(dialect);

    Ok(format!(
        "INSERT INTO symbol_semantics (artefact_id, repo_id, blob_sha, semantic_features_input_hash, docstring_summary, llm_summary, template_summary, summary, confidence, source_model) \
VALUES ('{artefact_id}', '{repo_id}', '{blob_sha}', '{input_hash}', {docstring_summary}, {llm_summary}, '{template_summary}', '{summary}', {confidence:.4}, {source_model}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, docstring_summary = EXCLUDED.docstring_summary, llm_summary = EXCLUDED.llm_summary, template_summary = EXCLUDED.template_summary, summary = EXCLUDED.summary, confidence = EXCLUDED.confidence, source_model = EXCLUDED.source_model, generated_at = {generated_at}",
        artefact_id = esc_pg(&semantics.artefact_id),
        repo_id = esc_pg(&semantics.repo_id),
        blob_sha = esc_pg(&semantics.blob_sha),
        input_hash = esc_pg(semantic_features_input_hash),
        docstring_summary = docstring_summary_expr,
        llm_summary = llm_summary_expr,
        template_summary = esc_pg(&semantics.template_summary),
        summary = esc_pg(&semantics.summary),
        confidence = semantics.confidence,
        source_model = source_model_expr,
        generated_at = generated_at_sql,
    ))
}

fn build_current_semantic_target_select_sql(input: &semantic::SemanticFeatureInput) -> String {
    format!(
        "SELECT current.artefact_id, current.repo_id, current.path, current.content_id, current.symbol_id \
FROM artefacts_current current \
JOIN current_file_state state ON state.repo_id = current.repo_id AND state.path = current.path \
WHERE current.repo_id = '{repo_id}' \
  AND current.path = '{path}' \
  AND current.content_id = '{content_id}' \
  AND LOWER(COALESCE(current.canonical_kind, COALESCE(current.language_kind, 'symbol'))) = '{canonical_kind}' \
  AND COALESCE(current.symbol_fqn, current.path) = '{symbol_fqn}' \
  AND state.analysis_mode = 'code' \
  AND state.effective_content_id = current.content_id \
ORDER BY coalesce(current.start_line, 0), current.symbol_id, coalesce(current.start_byte, 0), current.artefact_id \
LIMIT 1",
        repo_id = esc_pg(&input.repo_id),
        path = esc_pg(&input.path),
        content_id = esc_pg(&input.blob_sha),
        canonical_kind = esc_pg(&input.canonical_kind.to_ascii_lowercase()),
        symbol_fqn = esc_pg(&input.symbol_fqn),
    )
}

#[allow(dead_code)]
fn build_current_semantic_persist_summary_sql(
    semantics: &semantic::SymbolSemanticsRow,
    semantic_features_input_hash: &str,
    symbol_id: Option<&str>,
    path: &str,
    content_id: &str,
    dialect: RelationalDialect,
) -> Result<String> {
    let symbol_id_expr = sql_optional_string(symbol_id);
    let docstring_summary_expr = sql_optional_string(semantics.docstring_summary.as_deref());
    let llm_summary_expr = sql_optional_string(semantics.llm_summary.as_deref());
    let source_model_expr = sql_optional_string(semantics.source_model.as_deref());
    let generated_at_sql = semantic_generated_at_now_sql(dialect);

    Ok(format!(
        "INSERT INTO symbol_semantics_current (artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash, docstring_summary, llm_summary, template_summary, summary, confidence, source_model) \
VALUES ('{artefact_id}', '{repo_id}', '{path}', '{content_id}', {symbol_id}, '{input_hash}', {docstring_summary}, {llm_summary}, '{template_summary}', '{summary}', {confidence:.4}, {source_model}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, path = EXCLUDED.path, content_id = EXCLUDED.content_id, symbol_id = EXCLUDED.symbol_id, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, docstring_summary = EXCLUDED.docstring_summary, llm_summary = EXCLUDED.llm_summary, template_summary = EXCLUDED.template_summary, summary = EXCLUDED.summary, confidence = EXCLUDED.confidence, source_model = EXCLUDED.source_model, generated_at = {generated_at}",
        artefact_id = esc_pg(&semantics.artefact_id),
        repo_id = esc_pg(&semantics.repo_id),
        path = esc_pg(path),
        content_id = esc_pg(content_id),
        symbol_id = symbol_id_expr,
        input_hash = esc_pg(semantic_features_input_hash),
        docstring_summary = docstring_summary_expr,
        llm_summary = llm_summary_expr,
        template_summary = esc_pg(&semantics.template_summary),
        summary = esc_pg(&semantics.summary),
        confidence = semantics.confidence,
        source_model = source_model_expr,
        generated_at = generated_at_sql,
    ))
}

fn build_conditional_current_semantic_persist_summary_sql(
    semantics: &semantic::SymbolSemanticsRow,
    semantic_features_input_hash: &str,
    input: &semantic::SemanticFeatureInput,
    dialect: RelationalDialect,
) -> Result<String> {
    let docstring_summary_expr = sql_optional_string(semantics.docstring_summary.as_deref());
    let llm_summary_expr = sql_optional_string(semantics.llm_summary.as_deref());
    let source_model_expr = sql_optional_string(semantics.source_model.as_deref());
    let generated_at_sql = semantic_generated_at_now_sql(dialect);
    let target_select = build_current_semantic_target_select_sql(input);

    Ok(format!(
        "INSERT INTO symbol_semantics_current (artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash, docstring_summary, llm_summary, template_summary, summary, confidence, source_model) \
SELECT target.artefact_id, target.repo_id, target.path, target.content_id, target.symbol_id, '{input_hash}', {docstring_summary}, {llm_summary}, '{template_summary}', '{summary}', {confidence:.4}, {source_model} \
FROM ({target_select}) target \
WHERE 1 = 1 \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, path = EXCLUDED.path, content_id = EXCLUDED.content_id, symbol_id = EXCLUDED.symbol_id, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, docstring_summary = EXCLUDED.docstring_summary, llm_summary = EXCLUDED.llm_summary, template_summary = EXCLUDED.template_summary, summary = EXCLUDED.summary, confidence = EXCLUDED.confidence, source_model = EXCLUDED.source_model, generated_at = {generated_at}",
        input_hash = esc_pg(semantic_features_input_hash),
        docstring_summary = docstring_summary_expr,
        llm_summary = llm_summary_expr,
        template_summary = esc_pg(&semantics.template_summary),
        summary = esc_pg(&semantics.summary),
        confidence = semantics.confidence,
        source_model = source_model_expr,
        target_select = target_select,
        generated_at = generated_at_sql,
    ))
}

fn sql_string(value: &str) -> String {
    format!("'{}'", esc_pg(value))
}

fn sql_optional_string(value: Option<&str>) -> String {
    value.map(sql_string).unwrap_or_else(|| "NULL".to_string())
}

fn sql_json_string_for_dialect<T: serde::Serialize>(
    value: &T,
    dialect: RelationalDialect,
) -> Result<String> {
    let json = esc_pg(&serde_json::to_string(value)?);
    Ok(match dialect {
        RelationalDialect::Postgres => format!("'{json}'::jsonb"),
        RelationalDialect::Sqlite => format!("'{json}'"),
    })
}
