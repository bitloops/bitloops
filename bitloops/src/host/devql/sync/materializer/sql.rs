use serde_json::Value;

use super::super::types::DesiredFileState;
use super::types::{MaterializedArtefact, MaterializedEdge};

pub(super) fn insert_artefact_sql(
    relational: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    path: &str,
    content_id: &str,
    language: &str,
    artefact: &MaterializedArtefact,
    now_sql: &str,
) -> String {
    let canonical_kind_sql = nullable_text_sql(artefact.canonical_kind.as_deref());
    let parent_symbol_id_sql = nullable_text_sql(artefact.parent_symbol_id.as_deref());
    let parent_artefact_id_sql = nullable_text_sql(artefact.parent_artefact_id.as_deref());
    let signature_sql = nullable_text_sql(artefact.signature.as_deref());
    let modifiers_sql = crate::host::devql::sql_json_value(
        relational,
        &serde_json::to_value(&artefact.modifiers).unwrap_or(Value::Array(Vec::new())),
    );
    let docstring_sql = nullable_text_sql(artefact.docstring.as_deref());

    format!(
        "INSERT INTO artefacts_current (repo_id, path, content_id, symbol_id, artefact_id, language, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, updated_at) \\
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', {}, '{}', '{}', {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(path),
        crate::host::devql::esc_pg(content_id),
        crate::host::devql::esc_pg(&artefact.symbol_id),
        crate::host::devql::esc_pg(&artefact.artefact_id),
        crate::host::devql::esc_pg(language),
        canonical_kind_sql,
        crate::host::devql::esc_pg(&artefact.language_kind),
        crate::host::devql::esc_pg(&artefact.symbol_fqn),
        parent_symbol_id_sql,
        parent_artefact_id_sql,
        artefact.start_line,
        artefact.end_line,
        artefact.start_byte,
        artefact.end_byte,
        signature_sql,
        modifiers_sql,
        docstring_sql,
        now_sql,
    )
}

pub(super) fn insert_edge_sql(
    relational: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    path: &str,
    content_id: &str,
    edge: &MaterializedEdge,
    now_sql: &str,
) -> String {
    let to_symbol_id_sql = nullable_text_sql(edge.to_symbol_id.as_deref());
    let to_artefact_id_sql = nullable_text_sql(edge.to_artefact_id.as_deref());
    let to_symbol_ref_sql = nullable_text_sql(edge.to_symbol_ref.as_deref());
    let start_line_sql = nullable_i32_sql(edge.start_line);
    let end_line_sql = nullable_i32_sql(edge.end_line);
    let metadata_sql = crate::host::devql::sql_json_value(relational, &edge.metadata);

    format!(
        "INSERT INTO artefact_edges_current (repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata, updated_at) \\
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', {}, {}, {}, '{}', '{}', {}, {}, {}, {})",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(&edge.edge_id),
        crate::host::devql::esc_pg(path),
        crate::host::devql::esc_pg(content_id),
        crate::host::devql::esc_pg(&edge.from_symbol_id),
        crate::host::devql::esc_pg(&edge.from_artefact_id),
        to_symbol_id_sql,
        to_artefact_id_sql,
        to_symbol_ref_sql,
        crate::host::devql::esc_pg(&edge.edge_kind),
        crate::host::devql::esc_pg(&edge.language),
        start_line_sql,
        end_line_sql,
        metadata_sql,
        now_sql,
    )
}

pub(super) fn upsert_current_file_state_sql(
    repo_id: &str,
    desired: &DesiredFileState,
    parser_version: &str,
    extractor_version: &str,
    now_sql: &str,
) -> String {
    let head_content_id_sql = nullable_text_sql(desired.head_content_id.as_deref());
    let index_content_id_sql = nullable_text_sql(desired.index_content_id.as_deref());
    let worktree_content_id_sql = nullable_text_sql(desired.worktree_content_id.as_deref());
    format!(
        "INSERT INTO current_file_state (repo_id, path, language, head_content_id, index_content_id, worktree_content_id, effective_content_id, effective_source, parser_version, extractor_version, exists_in_head, exists_in_index, exists_in_worktree, last_synced_at) \\
VALUES ('{}', '{}', '{}', {}, {}, {}, '{}', '{}', '{}', '{}', {}, {}, {}, {}) \\
ON CONFLICT (repo_id, path) DO UPDATE SET language = EXCLUDED.language, head_content_id = EXCLUDED.head_content_id, index_content_id = EXCLUDED.index_content_id, worktree_content_id = EXCLUDED.worktree_content_id, effective_content_id = EXCLUDED.effective_content_id, effective_source = EXCLUDED.effective_source, parser_version = EXCLUDED.parser_version, extractor_version = EXCLUDED.extractor_version, exists_in_head = EXCLUDED.exists_in_head, exists_in_index = EXCLUDED.exists_in_index, exists_in_worktree = EXCLUDED.exists_in_worktree, last_synced_at = EXCLUDED.last_synced_at",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(&desired.path),
        crate::host::devql::esc_pg(&desired.language),
        head_content_id_sql,
        index_content_id_sql,
        worktree_content_id_sql,
        crate::host::devql::esc_pg(&desired.effective_content_id),
        crate::host::devql::esc_pg(desired.effective_source.as_str()),
        crate::host::devql::esc_pg(parser_version),
        crate::host::devql::esc_pg(extractor_version),
        bool_sql(desired.exists_in_head),
        bool_sql(desired.exists_in_index),
        bool_sql(desired.exists_in_worktree),
        now_sql,
    )
}

pub(super) fn delete_edges_sql(repo_id: &str, path: &str) -> String {
    format!(
        "DELETE FROM artefact_edges_current WHERE repo_id = '{}' AND path = '{}'",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(path),
    )
}

pub(super) fn delete_artefacts_sql(repo_id: &str, path: &str) -> String {
    format!(
        "DELETE FROM artefacts_current WHERE repo_id = '{}' AND path = '{}'",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(path),
    )
}

pub(super) fn delete_current_file_state_sql(repo_id: &str, path: &str) -> String {
    format!(
        "DELETE FROM current_file_state WHERE repo_id = '{}' AND path = '{}'",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(path),
    )
}

fn nullable_text_sql(value: Option<&str>) -> String {
    value
        .map(|value| format!("'{}'", crate::host::devql::esc_pg(value)))
        .unwrap_or_else(|| "NULL".to_string())
}

fn nullable_i32_sql(value: Option<i32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

pub(super) fn bool_sql(value: bool) -> i32 {
    if value { 1 } else { 0 }
}

pub(super) fn non_empty_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
