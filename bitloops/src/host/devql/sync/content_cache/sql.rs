use serde_json::Value;

use crate::host::devql::RelationalStorage;
use crate::host::devql::db_utils::esc_pg;
use crate::host::devql::ingestion_artefact_persistence_sql::{sql_json_value, sql_now};

use super::types::{CachedArtefact, CachedEdge, CachedExtraction};

pub(super) fn build_upsert_cached_header_sql(
    relational: &RelationalStorage,
    extraction: &CachedExtraction,
    retention_class: &str,
) -> String {
    format!(
        "INSERT INTO content_cache (content_id, language, extraction_fingerprint, parser_version, extractor_version, retention_class, parse_status, parsed_at, last_accessed_at) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', '{}', {}, {}) \
ON CONFLICT (content_id, language, extraction_fingerprint, parser_version, extractor_version) DO UPDATE SET retention_class = EXCLUDED.retention_class, parse_status = EXCLUDED.parse_status, parsed_at = EXCLUDED.parsed_at, last_accessed_at = EXCLUDED.last_accessed_at",
        esc_pg(&extraction.content_id),
        esc_pg(&extraction.language),
        esc_pg(&extraction.extraction_fingerprint),
        esc_pg(&extraction.parser_version),
        esc_pg(&extraction.extractor_version),
        esc_pg(retention_class),
        esc_pg(&extraction.parse_status),
        sql_now(relational),
        sql_now(relational),
    )
}

pub(super) fn build_delete_cached_artefacts_sql(extraction: &CachedExtraction) -> String {
    format!(
        "DELETE FROM content_cache_artefacts \
WHERE content_id = '{}' AND language = '{}' AND extraction_fingerprint = '{}' AND parser_version = '{}' AND extractor_version = '{}'",
        esc_pg(&extraction.content_id),
        esc_pg(&extraction.language),
        esc_pg(&extraction.extraction_fingerprint),
        esc_pg(&extraction.parser_version),
        esc_pg(&extraction.extractor_version),
    )
}

pub(super) fn build_delete_cached_edges_sql(extraction: &CachedExtraction) -> String {
    format!(
        "DELETE FROM content_cache_edges \
WHERE content_id = '{}' AND language = '{}' AND extraction_fingerprint = '{}' AND parser_version = '{}' AND extractor_version = '{}'",
        esc_pg(&extraction.content_id),
        esc_pg(&extraction.language),
        esc_pg(&extraction.extraction_fingerprint),
        esc_pg(&extraction.parser_version),
        esc_pg(&extraction.extractor_version),
    )
}

pub(super) fn build_insert_cached_artefact_sql(
    relational: &RelationalStorage,
    extraction: &CachedExtraction,
    artefact: &CachedArtefact,
) -> String {
    format!(
        "INSERT INTO content_cache_artefacts (content_id, language, extraction_fingerprint, parser_version, extractor_version, artifact_key, canonical_kind, language_kind, name, parent_artifact_key, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, metadata) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', {}, '{}', '{}', {}, {}, {}, {}, {}, '{}', {}, {}, {})",
        esc_pg(&extraction.content_id),
        esc_pg(&extraction.language),
        esc_pg(&extraction.extraction_fingerprint),
        esc_pg(&extraction.parser_version),
        esc_pg(&extraction.extractor_version),
        esc_pg(&artefact.artifact_key),
        nullable_text_sql(artefact.canonical_kind.as_deref()),
        esc_pg(&artefact.language_kind),
        esc_pg(&artefact.name),
        nullable_text_sql(artefact.parent_artifact_key.as_deref()),
        artefact.start_line,
        artefact.end_line,
        artefact.start_byte,
        artefact.end_byte,
        esc_pg(&artefact.signature),
        sql_json_value(
            relational,
            &Value::Array(
                artefact
                    .modifiers
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect::<Vec<_>>(),
            ),
        ),
        nullable_text_sql(artefact.docstring.as_deref()),
        sql_json_value(relational, &artefact.metadata),
    )
}

pub(super) fn build_insert_cached_edge_sql(
    relational: &RelationalStorage,
    extraction: &CachedExtraction,
    edge: &CachedEdge,
) -> String {
    format!(
        "INSERT INTO content_cache_edges (content_id, language, extraction_fingerprint, parser_version, extractor_version, edge_key, from_artifact_key, to_artifact_key, to_symbol_ref, edge_kind, start_line, end_line, metadata) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', '{}', {}, {}, '{}', {}, {}, {})",
        esc_pg(&extraction.content_id),
        esc_pg(&extraction.language),
        esc_pg(&extraction.extraction_fingerprint),
        esc_pg(&extraction.parser_version),
        esc_pg(&extraction.extractor_version),
        esc_pg(&edge.edge_key),
        esc_pg(&edge.from_artifact_key),
        nullable_text_sql(edge.to_artifact_key.as_deref()),
        nullable_text_sql(edge.to_symbol_ref.as_deref()),
        esc_pg(&edge.edge_kind),
        nullable_i32_sql(edge.start_line),
        nullable_i32_sql(edge.end_line),
        sql_json_value(relational, &edge.metadata),
    )
}

fn nullable_text_sql(value: Option<&str>) -> String {
    value
        .map(|value| format!("'{}'", esc_pg(value)))
        .unwrap_or_else(|| "NULL".to_string())
}

fn nullable_i32_sql(value: Option<i32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}
