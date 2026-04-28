use anyhow::Result;
use serde_json::Value;

use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::devql::esc_pg;

pub(crate) fn build_semantic_get_artefacts_sql(
    repo_id: &str,
    blob_sha: &str,
    path: &str,
) -> String {
    format!(
        "SELECT artefact_id, symbol_id, repo_id, blob_sha, path, language, \
COALESCE(canonical_kind, COALESCE(language_kind, 'symbol')) AS canonical_kind, \
COALESCE(language_kind, COALESCE(canonical_kind, 'symbol')) AS language_kind, \
COALESCE(symbol_fqn, path) AS symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash \
FROM artefacts_historical \
WHERE repo_id = '{repo_id}' AND blob_sha = '{blob_sha}' AND path = '{path}' \
ORDER BY coalesce(start_byte, 0), coalesce(start_line, 0), artefact_id",
        repo_id = esc_pg(repo_id),
        blob_sha = esc_pg(blob_sha),
        path = esc_pg(path),
    )
}

pub(crate) fn build_current_repo_artefacts_sql(repo_id: &str) -> String {
    format!(
        "SELECT current.artefact_id, current.symbol_id, current.repo_id, current.content_id AS blob_sha, current.path, current.language, \
COALESCE(current.canonical_kind, COALESCE(current.language_kind, 'symbol')) AS canonical_kind, \
COALESCE(current.language_kind, COALESCE(current.canonical_kind, 'symbol')) AS language_kind, \
COALESCE(current.symbol_fqn, current.path) AS symbol_fqn, current.parent_artefact_id, current.start_line, current.end_line, current.start_byte, current.end_byte, current.signature, current.modifiers, current.docstring, a.content_hash \
FROM artefacts_current current \
JOIN current_file_state state ON state.repo_id = current.repo_id AND state.path = current.path \
LEFT JOIN artefacts a ON a.repo_id = current.repo_id AND a.artefact_id = current.artefact_id \
WHERE current.repo_id = '{repo_id}' AND state.analysis_mode = 'code' \
ORDER BY current.path, current.start_line, current.symbol_id, coalesce(current.start_byte, 0), current.artefact_id",
        repo_id = esc_pg(repo_id),
    )
}

pub(crate) fn build_current_repo_artefacts_by_ids_sql(
    repo_id: &str,
    artefact_ids: &[String],
) -> String {
    let artefact_ids = artefact_ids
        .iter()
        .map(|artefact_id| format!("'{}'", esc_pg(artefact_id)))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "SELECT current.artefact_id, current.symbol_id, current.repo_id, current.content_id AS blob_sha, current.path, current.language, \
COALESCE(current.canonical_kind, COALESCE(current.language_kind, 'symbol')) AS canonical_kind, \
COALESCE(current.language_kind, COALESCE(current.canonical_kind, 'symbol')) AS language_kind, \
COALESCE(current.symbol_fqn, current.path) AS symbol_fqn, current.parent_artefact_id, current.start_line, current.end_line, current.start_byte, current.end_byte, current.signature, current.modifiers, current.docstring, a.content_hash \
FROM artefacts_current current \
JOIN current_file_state state ON state.repo_id = current.repo_id AND state.path = current.path \
LEFT JOIN artefacts a ON a.repo_id = current.repo_id AND a.artefact_id = current.artefact_id \
WHERE current.repo_id = '{repo_id}' AND state.analysis_mode = 'code' AND current.artefact_id IN ({artefact_ids}) \
ORDER BY current.path, current.start_line, current.symbol_id, coalesce(current.start_byte, 0), current.artefact_id",
        repo_id = esc_pg(repo_id),
    )
}

pub(crate) fn build_semantic_get_dependencies_sql(
    repo_id: &str,
    blob_sha: &str,
    path: &str,
) -> String {
    format!(
        "SELECT e.from_artefact_id, LOWER(e.edge_kind) AS edge_kind, \
COALESCE(target.symbol_fqn, target.path, e.to_symbol_ref, e.to_artefact_id, '') AS target_ref \
FROM artefact_edges e \
JOIN artefacts_historical source ON source.repo_id = e.repo_id AND source.artefact_id = e.from_artefact_id AND source.blob_sha = e.blob_sha \
LEFT JOIN artefacts_historical target ON target.repo_id = e.repo_id AND target.artefact_id = e.to_artefact_id AND target.blob_sha = e.blob_sha \
WHERE e.repo_id = '{repo_id}' AND e.blob_sha = '{blob_sha}' AND source.path = '{path}' \
ORDER BY e.from_artefact_id, e.edge_kind, target_ref",
        repo_id = esc_pg(repo_id),
        blob_sha = esc_pg(blob_sha),
        path = esc_pg(path),
    )
}

pub(crate) fn parse_semantic_artefact_rows(
    rows: Vec<Value>,
) -> Result<Vec<semantic::PreStageArtefactRow>> {
    let mut artefacts = Vec::with_capacity(rows.len());
    for row in rows {
        let modifiers = parse_semantic_json_string_array(row.get("modifiers"));
        let mut normalized_row = row;
        if let Value::Object(ref mut object) = normalized_row {
            object.insert(
                "modifiers".to_string(),
                Value::Array(modifiers.iter().cloned().map(Value::String).collect()),
            );
        }
        let mut artefact = serde_json::from_value::<semantic::PreStageArtefactRow>(normalized_row)?;
        artefact.modifiers = modifiers;
        artefacts.push(artefact);
    }
    Ok(artefacts)
}

pub(crate) fn parse_semantic_dependency_rows(
    rows: Vec<Value>,
) -> Result<Vec<semantic::PreStageDependencyRow>> {
    let mut dependencies = Vec::with_capacity(rows.len());
    for row in rows {
        let dependency = serde_json::from_value::<semantic::PreStageDependencyRow>(row)?;
        if dependency.target_ref.trim().is_empty() {
            continue;
        }
        dependencies.push(dependency);
    }
    Ok(dependencies)
}

pub(crate) fn build_semantic_get_artefacts_by_ids_sql(artefact_ids: &[String]) -> String {
    let artefact_ids = artefact_ids
        .iter()
        .map(|artefact_id| format!("'{}'", esc_pg(artefact_id)))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "SELECT artefact_id, symbol_id, repo_id, blob_sha, path, language, \
COALESCE(canonical_kind, COALESCE(language_kind, 'symbol')) AS canonical_kind, \
COALESCE(language_kind, COALESCE(canonical_kind, 'symbol')) AS language_kind, \
COALESCE(symbol_fqn, path) AS symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash \
FROM artefacts_historical \
WHERE artefact_id IN ({artefact_ids}) \
ORDER BY repo_id, blob_sha, path, coalesce(start_byte, 0), coalesce(start_line, 0), artefact_id",
    )
}

pub(crate) fn build_semantic_get_summary_sql(artefact_id: &str) -> String {
    format!(
        "SELECT semantic_features_input_hash, summary, llm_summary, source_model \
FROM symbol_semantics \
WHERE artefact_id = '{artefact_id}'",
        artefact_id = esc_pg(artefact_id),
    )
}

fn parse_semantic_json_string_array(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::String(raw)) => serde_json::from_str::<Vec<String>>(raw).unwrap_or_default(),
        _ => Vec::new(),
    }
}
