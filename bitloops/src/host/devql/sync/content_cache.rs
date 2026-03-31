use anyhow::Result;
use serde_json::{Map, Value};

use crate::host::devql::RelationalStorage;
use crate::host::devql::db_utils::esc_pg;
use crate::host::devql::ingestion_artefact_persistence_sql::{
    parse_json_array_strings, parse_json_value_or_default, parse_nullable_i32, parse_required_i32,
    sql_json_value, sql_now,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CachedExtraction {
    pub(crate) content_id: String,
    pub(crate) language: String,
    pub(crate) parser_version: String,
    pub(crate) extractor_version: String,
    pub(crate) parse_status: String,
    pub(crate) artefacts: Vec<CachedArtefact>,
    pub(crate) edges: Vec<CachedEdge>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CachedArtefact {
    pub(crate) artifact_key: String,
    pub(crate) canonical_kind: Option<String>,
    pub(crate) language_kind: String,
    pub(crate) name: String,
    pub(crate) parent_artifact_key: Option<String>,
    pub(crate) start_line: i32,
    pub(crate) end_line: i32,
    pub(crate) start_byte: i32,
    pub(crate) end_byte: i32,
    pub(crate) signature: String,
    pub(crate) modifiers: Vec<String>,
    pub(crate) docstring: Option<String>,
    pub(crate) metadata: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CachedEdge {
    pub(crate) edge_key: String,
    pub(crate) from_artifact_key: String,
    pub(crate) to_artifact_key: Option<String>,
    pub(crate) to_symbol_ref: Option<String>,
    pub(crate) edge_kind: String,
    pub(crate) start_line: Option<i32>,
    pub(crate) end_line: Option<i32>,
    pub(crate) metadata: Value,
}

#[derive(Debug)]
struct CachedHeaderRow {
    content_id: String,
    language: String,
    parser_version: String,
    extractor_version: String,
    parse_status: String,
}

pub(crate) async fn lookup_cached_content(
    relational: &RelationalStorage,
    content_id: &str,
    language: &str,
    parser_version: &str,
    extractor_version: &str,
) -> Result<Option<CachedExtraction>> {
    let header_sql = format!(
        "SELECT content_id, language, parser_version, extractor_version, parse_status \
FROM content_cache \
WHERE content_id = '{}' AND language = '{}' AND parser_version = '{}' AND extractor_version = '{}' \
LIMIT 1",
        esc_pg(content_id),
        esc_pg(language),
        esc_pg(parser_version),
        esc_pg(extractor_version),
    );
    let header_rows = relational.query_rows(&header_sql).await?;
    let Some(header) = header_rows
        .first()
        .and_then(Value::as_object)
        .and_then(cached_header_from_row)
    else {
        return Ok(None);
    };

    let artefacts_sql = format!(
        "SELECT artifact_key, canonical_kind, language_kind, name, parent_artifact_key, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, metadata \
FROM content_cache_artefacts \
WHERE content_id = '{}' AND language = '{}' AND parser_version = '{}' AND extractor_version = '{}' \
ORDER BY artifact_key",
        esc_pg(content_id),
        esc_pg(language),
        esc_pg(parser_version),
        esc_pg(extractor_version),
    );
    let artefacts = relational
        .query_rows(&artefacts_sql)
        .await?
        .into_iter()
        .filter_map(|row| row.as_object().and_then(cached_artefact_from_row))
        .collect::<Vec<_>>();

    let edges_sql = format!(
        "SELECT edge_key, from_artifact_key, to_artifact_key, to_symbol_ref, edge_kind, start_line, end_line, metadata \
FROM content_cache_edges \
WHERE content_id = '{}' AND language = '{}' AND parser_version = '{}' AND extractor_version = '{}' \
ORDER BY edge_key",
        esc_pg(content_id),
        esc_pg(language),
        esc_pg(parser_version),
        esc_pg(extractor_version),
    );
    let edges = relational
        .query_rows(&edges_sql)
        .await?
        .into_iter()
        .filter_map(|row| row.as_object().and_then(cached_edge_from_row))
        .collect::<Vec<_>>();

    let update_last_accessed_sql = format!(
        "UPDATE content_cache \
SET last_accessed_at = {} \
WHERE content_id = '{}' AND language = '{}' AND parser_version = '{}' AND extractor_version = '{}'",
        sql_now(relational),
        esc_pg(content_id),
        esc_pg(language),
        esc_pg(parser_version),
        esc_pg(extractor_version),
    );
    relational.exec(&update_last_accessed_sql).await?;

    Ok(Some(CachedExtraction {
        content_id: header.content_id,
        language: header.language,
        parser_version: header.parser_version,
        extractor_version: header.extractor_version,
        parse_status: header.parse_status,
        artefacts,
        edges,
    }))
}

pub(crate) async fn store_cached_content(
    relational: &RelationalStorage,
    extraction: &CachedExtraction,
    retention_class: &str,
) -> Result<()> {
    let mut statements = vec![
        build_upsert_cached_header_sql(relational, extraction, retention_class),
        build_delete_cached_edges_sql(extraction),
        build_delete_cached_artefacts_sql(extraction),
    ];
    statements.extend(
        extraction
            .artefacts
            .iter()
            .map(|artefact| build_insert_cached_artefact_sql(relational, extraction, artefact)),
    );
    statements.extend(
        extraction
            .edges
            .iter()
            .map(|edge| build_insert_cached_edge_sql(relational, extraction, edge)),
    );

    relational.exec_batch_transactional(&statements).await
}

pub(crate) async fn promote_cached_content_to_git_backed(
    relational: &RelationalStorage,
    content_id: &str,
    language: &str,
    parser_version: &str,
    extractor_version: &str,
) -> Result<()> {
    let sql = format!(
        "UPDATE content_cache \
SET retention_class = 'git_backed' \
WHERE content_id = '{}' AND language = '{}' AND parser_version = '{}' AND extractor_version = '{}' \
AND retention_class = 'worktree_only'",
        esc_pg(content_id),
        esc_pg(language),
        esc_pg(parser_version),
        esc_pg(extractor_version),
    );
    relational.exec(&sql).await
}

fn cached_header_from_row(row: &Map<String, Value>) -> Option<CachedHeaderRow> {
    let content_id = row
        .get("content_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if content_id.is_empty() {
        return None;
    }

    Some(CachedHeaderRow {
        content_id,
        language: row
            .get("language")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        parser_version: row
            .get("parser_version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        extractor_version: row
            .get("extractor_version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        parse_status: row
            .get("parse_status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

fn cached_artefact_from_row(row: &Map<String, Value>) -> Option<CachedArtefact> {
    let artifact_key = row
        .get("artifact_key")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if artifact_key.is_empty() {
        return None;
    }

    Some(CachedArtefact {
        artifact_key,
        canonical_kind: row
            .get("canonical_kind")
            .and_then(Value::as_str)
            .map(str::to_string),
        language_kind: row
            .get("language_kind")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        name: row
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        parent_artifact_key: row
            .get("parent_artifact_key")
            .and_then(Value::as_str)
            .map(str::to_string),
        start_line: parse_required_i32(row.get("start_line")),
        end_line: parse_required_i32(row.get("end_line")),
        start_byte: parse_required_i32(row.get("start_byte")),
        end_byte: parse_required_i32(row.get("end_byte")),
        signature: row
            .get("signature")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        modifiers: parse_json_array_strings(row.get("modifiers")),
        docstring: row
            .get("docstring")
            .and_then(Value::as_str)
            .map(str::to_string),
        metadata: parse_json_value_or_default(row.get("metadata"), Value::Object(Map::new())),
    })
}

fn cached_edge_from_row(row: &Map<String, Value>) -> Option<CachedEdge> {
    let edge_key = row
        .get("edge_key")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if edge_key.is_empty() {
        return None;
    }

    Some(CachedEdge {
        edge_key,
        from_artifact_key: row
            .get("from_artifact_key")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        to_artifact_key: row
            .get("to_artifact_key")
            .and_then(Value::as_str)
            .map(str::to_string),
        to_symbol_ref: row
            .get("to_symbol_ref")
            .and_then(Value::as_str)
            .map(str::to_string),
        edge_kind: row
            .get("edge_kind")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        start_line: parse_nullable_i32(row.get("start_line")),
        end_line: parse_nullable_i32(row.get("end_line")),
        metadata: parse_json_value_or_default(row.get("metadata"), Value::Object(Map::new())),
    })
}

fn build_upsert_cached_header_sql(
    relational: &RelationalStorage,
    extraction: &CachedExtraction,
    retention_class: &str,
) -> String {
    format!(
        "INSERT INTO content_cache (content_id, language, parser_version, extractor_version, retention_class, parse_status, parsed_at, last_accessed_at) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', {}, {}) \
ON CONFLICT (content_id, language, parser_version, extractor_version) DO UPDATE SET retention_class = EXCLUDED.retention_class, parse_status = EXCLUDED.parse_status, parsed_at = EXCLUDED.parsed_at, last_accessed_at = EXCLUDED.last_accessed_at",
        esc_pg(&extraction.content_id),
        esc_pg(&extraction.language),
        esc_pg(&extraction.parser_version),
        esc_pg(&extraction.extractor_version),
        esc_pg(retention_class),
        esc_pg(&extraction.parse_status),
        sql_now(relational),
        sql_now(relational),
    )
}

fn build_delete_cached_artefacts_sql(extraction: &CachedExtraction) -> String {
    format!(
        "DELETE FROM content_cache_artefacts \
WHERE content_id = '{}' AND language = '{}' AND parser_version = '{}' AND extractor_version = '{}'",
        esc_pg(&extraction.content_id),
        esc_pg(&extraction.language),
        esc_pg(&extraction.parser_version),
        esc_pg(&extraction.extractor_version),
    )
}

fn build_delete_cached_edges_sql(extraction: &CachedExtraction) -> String {
    format!(
        "DELETE FROM content_cache_edges \
WHERE content_id = '{}' AND language = '{}' AND parser_version = '{}' AND extractor_version = '{}'",
        esc_pg(&extraction.content_id),
        esc_pg(&extraction.language),
        esc_pg(&extraction.parser_version),
        esc_pg(&extraction.extractor_version),
    )
}

fn build_insert_cached_artefact_sql(
    relational: &RelationalStorage,
    extraction: &CachedExtraction,
    artefact: &CachedArtefact,
) -> String {
    format!(
        "INSERT INTO content_cache_artefacts (content_id, language, parser_version, extractor_version, artifact_key, canonical_kind, language_kind, name, parent_artifact_key, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, metadata) \
VALUES ('{}', '{}', '{}', '{}', '{}', {}, '{}', '{}', {}, {}, {}, {}, {}, '{}', {}, {}, {})",
        esc_pg(&extraction.content_id),
        esc_pg(&extraction.language),
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

fn build_insert_cached_edge_sql(
    relational: &RelationalStorage,
    extraction: &CachedExtraction,
    edge: &CachedEdge,
) -> String {
    format!(
        "INSERT INTO content_cache_edges (content_id, language, parser_version, extractor_version, edge_key, from_artifact_key, to_artifact_key, to_symbol_ref, edge_kind, start_line, end_line, metadata) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', {}, {}, '{}', {}, {}, {})",
        esc_pg(&extraction.content_id),
        esc_pg(&extraction.language),
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
