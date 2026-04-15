use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value};

use crate::host::devql::RelationalStorage;
use crate::host::devql::db_utils::esc_pg;
use crate::host::devql::ingestion_artefact_persistence_sql::{
    parse_json_array_strings, parse_json_value_or_default, parse_nullable_i32, parse_required_i32,
    sql_now,
};

use super::types::{CachedArtefact, CachedEdge, CachedExtraction};

#[derive(Debug)]
struct CachedHeaderRow {
    content_id: String,
    language: String,
    extraction_fingerprint: String,
    parser_version: String,
    extractor_version: String,
    parse_status: String,
}

pub(crate) async fn lookup_cached_content(
    relational: &RelationalStorage,
    content_id: &str,
    language: &str,
    extraction_fingerprint: &str,
    parser_version: &str,
    extractor_version: &str,
) -> Result<Option<CachedExtraction>> {
    let header_sql = format!(
        "SELECT content_id, language, extraction_fingerprint, parser_version, extractor_version, parse_status \
FROM content_cache \
WHERE content_id = '{}' AND language = '{}' AND extraction_fingerprint = '{}' AND parser_version = '{}' AND extractor_version = '{}' \
LIMIT 1",
        esc_pg(content_id),
        esc_pg(language),
        esc_pg(extraction_fingerprint),
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
WHERE content_id = '{}' AND language = '{}' AND extraction_fingerprint = '{}' AND parser_version = '{}' AND extractor_version = '{}' \
ORDER BY artifact_key",
        esc_pg(content_id),
        esc_pg(language),
        esc_pg(extraction_fingerprint),
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
WHERE content_id = '{}' AND language = '{}' AND extraction_fingerprint = '{}' AND parser_version = '{}' AND extractor_version = '{}' \
ORDER BY edge_key",
        esc_pg(content_id),
        esc_pg(language),
        esc_pg(extraction_fingerprint),
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
WHERE content_id = '{}' AND language = '{}' AND extraction_fingerprint = '{}' AND parser_version = '{}' AND extractor_version = '{}'",
        sql_now(relational),
        esc_pg(content_id),
        esc_pg(language),
        esc_pg(extraction_fingerprint),
        esc_pg(parser_version),
        esc_pg(extractor_version),
    );
    relational.exec(&update_last_accessed_sql).await?;

    Ok(Some(CachedExtraction {
        content_id: header.content_id,
        language: header.language,
        extraction_fingerprint: header.extraction_fingerprint,
        parser_version: header.parser_version,
        extractor_version: header.extractor_version,
        parse_status: header.parse_status,
        artefacts,
        edges,
    }))
}

pub(crate) fn lookup_cached_content_with_connection(
    conn: &Connection,
    content_id: &str,
    language: &str,
    extraction_fingerprint: &str,
    parser_version: &str,
    extractor_version: &str,
) -> Result<Option<CachedExtraction>> {
    let header = {
        let mut stmt = conn
            .prepare(
                "SELECT content_id, language, extraction_fingerprint, parser_version, extractor_version, parse_status \
                 FROM content_cache \
                 WHERE content_id = ?1 AND language = ?2 AND extraction_fingerprint = ?3 AND parser_version = ?4 AND extractor_version = ?5 \
                 LIMIT 1",
            )
            .context("preparing content cache header lookup")?;
        stmt.query_row(
            params![
                content_id,
                language,
                extraction_fingerprint,
                parser_version,
                extractor_version
            ],
            |row| {
                Ok(CachedHeaderRow {
                    content_id: row.get(0)?,
                    language: row.get(1)?,
                    extraction_fingerprint: row.get(2)?,
                    parser_version: row.get(3)?,
                    extractor_version: row.get(4)?,
                    parse_status: row.get(5)?,
                })
            },
        )
        .optional()
        .context("querying content cache header")?
    };
    let Some(header) = header else {
        return Ok(None);
    };

    let artefacts = {
        let mut stmt = conn
            .prepare(
                "SELECT artifact_key, canonical_kind, language_kind, name, parent_artifact_key, \
                        start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, metadata \
                 FROM content_cache_artefacts \
                 WHERE content_id = ?1 AND language = ?2 AND extraction_fingerprint = ?3 AND parser_version = ?4 AND extractor_version = ?5 \
                 ORDER BY artifact_key",
            )
            .context("preparing content cache artefact lookup")?;
        stmt.query_map(
            params![
                content_id,
                language,
                extraction_fingerprint,
                parser_version,
                extractor_version
            ],
            |row| {
                Ok(CachedArtefact {
                    artifact_key: row.get(0)?,
                    canonical_kind: row.get(1)?,
                    language_kind: row.get(2)?,
                    name: row.get(3)?,
                    parent_artifact_key: row.get(4)?,
                    start_line: row.get(5)?,
                    end_line: row.get(6)?,
                    start_byte: row.get(7)?,
                    end_byte: row.get(8)?,
                    signature: row.get(9)?,
                    modifiers: row
                        .get::<_, String>(10)
                        .ok()
                        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
                        .unwrap_or_default(),
                    docstring: row.get(11)?,
                    metadata: row
                        .get::<_, String>(12)
                        .ok()
                        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                        .unwrap_or_else(|| Value::Object(Map::new())),
                })
            },
        )
        .context("querying content cache artefacts")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("collecting content cache artefacts")?
    };

    let edges = {
        let mut stmt = conn
            .prepare(
                "SELECT edge_key, from_artifact_key, to_artifact_key, to_symbol_ref, edge_kind, start_line, end_line, metadata \
                 FROM content_cache_edges \
                 WHERE content_id = ?1 AND language = ?2 AND extraction_fingerprint = ?3 AND parser_version = ?4 AND extractor_version = ?5 \
                 ORDER BY edge_key",
            )
            .context("preparing content cache edge lookup")?;
        stmt.query_map(
            params![
                content_id,
                language,
                extraction_fingerprint,
                parser_version,
                extractor_version
            ],
            |row| {
                Ok(CachedEdge {
                    edge_key: row.get(0)?,
                    from_artifact_key: row.get(1)?,
                    to_artifact_key: row.get(2)?,
                    to_symbol_ref: row.get(3)?,
                    edge_kind: row.get(4)?,
                    start_line: row.get(5)?,
                    end_line: row.get(6)?,
                    metadata: row
                        .get::<_, String>(7)
                        .ok()
                        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                        .unwrap_or_else(|| Value::Object(Map::new())),
                })
            },
        )
        .context("querying content cache edges")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("collecting content cache edges")?
    };

    Ok(Some(CachedExtraction {
        content_id: header.content_id,
        language: header.language,
        extraction_fingerprint: header.extraction_fingerprint,
        parser_version: header.parser_version,
        extractor_version: header.extractor_version,
        parse_status: header.parse_status,
        artefacts,
        edges,
    }))
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
        extraction_fingerprint: row
            .get("extraction_fingerprint")
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
