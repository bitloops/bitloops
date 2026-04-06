#![cfg_attr(not(test), allow(dead_code))]

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CacheKey {
    pub(crate) content_id: String,
    pub(crate) language: String,
    pub(crate) parser_version: String,
    pub(crate) extractor_version: String,
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

pub(crate) fn lookup_cached_content_with_connection(
    conn: &Connection,
    content_id: &str,
    language: &str,
    parser_version: &str,
    extractor_version: &str,
) -> Result<Option<CachedExtraction>> {
    let header = {
        let mut stmt = conn
            .prepare(
                "SELECT content_id, language, parser_version, extractor_version, parse_status \
                 FROM content_cache \
                 WHERE content_id = ?1 AND language = ?2 AND parser_version = ?3 AND extractor_version = ?4 \
                 LIMIT 1",
            )
            .context("preparing content cache header lookup")?;
        stmt.query_row(
            params![content_id, language, parser_version, extractor_version],
            |row| {
                Ok(CachedHeaderRow {
                    content_id: row.get(0)?,
                    language: row.get(1)?,
                    parser_version: row.get(2)?,
                    extractor_version: row.get(3)?,
                    parse_status: row.get(4)?,
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
                 WHERE content_id = ?1 AND language = ?2 AND parser_version = ?3 AND extractor_version = ?4 \
                 ORDER BY artifact_key",
            )
            .context("preparing content cache artefact lookup")?;
        stmt.query_map(
            params![content_id, language, parser_version, extractor_version],
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
                 WHERE content_id = ?1 AND language = ?2 AND parser_version = ?3 AND extractor_version = ?4 \
                 ORDER BY edge_key",
            )
            .context("preparing content cache edge lookup")?;
        stmt.query_map(
            params![content_id, language, parser_version, extractor_version],
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
    let artefacts = dedupe_last_wins(&extraction.artefacts, |artefact| {
        artefact.artifact_key.as_str()
    });
    let edges = dedupe_last_wins(&extraction.edges, |edge| edge.edge_key.as_str());
    let mut statements = vec![
        build_upsert_cached_header_sql(relational, extraction, retention_class),
        build_delete_cached_edges_sql(extraction),
        build_delete_cached_artefacts_sql(extraction),
    ];
    statements.extend(
        artefacts
            .iter()
            .map(|artefact| build_insert_cached_artefact_sql(relational, extraction, artefact)),
    );
    statements.extend(
        edges
            .iter()
            .map(|edge| build_insert_cached_edge_sql(relational, extraction, edge)),
    );

    relational.exec_batch_transactional(&statements).await
}

pub(crate) fn persist_cached_content_tx(
    tx: &Transaction<'_>,
    extraction: &CachedExtraction,
    retention_class: &str,
) -> Result<usize> {
    let (artefacts, edges) = deduped_cached_content_parts(extraction);
    let mut affected_rows = 0usize;

    affected_rows += tx
        .execute(
            "INSERT INTO content_cache (content_id, language, parser_version, extractor_version, retention_class, parse_status, parsed_at, last_accessed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now')) \
             ON CONFLICT (content_id, language, parser_version, extractor_version) DO UPDATE SET \
                 retention_class = excluded.retention_class, \
                 parse_status = excluded.parse_status, \
                 parsed_at = excluded.parsed_at, \
                 last_accessed_at = excluded.last_accessed_at",
            params![
                extraction.content_id,
                extraction.language,
                extraction.parser_version,
                extraction.extractor_version,
                retention_class,
                extraction.parse_status,
            ],
        )
        .context("upserting content cache header")?;

    affected_rows += tx
        .execute(
            "DELETE FROM content_cache_edges WHERE content_id = ?1 AND language = ?2 AND parser_version = ?3 AND extractor_version = ?4",
            params![
                extraction.content_id,
                extraction.language,
                extraction.parser_version,
                extraction.extractor_version,
            ],
        )
        .context("deleting cached content edges before rewrite")?;
    affected_rows += tx
        .execute(
            "DELETE FROM content_cache_artefacts WHERE content_id = ?1 AND language = ?2 AND parser_version = ?3 AND extractor_version = ?4",
            params![
                extraction.content_id,
                extraction.language,
                extraction.parser_version,
                extraction.extractor_version,
            ],
        )
        .context("deleting cached content artefacts before rewrite")?;

    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO content_cache_artefacts \
                 (content_id, language, parser_version, extractor_version, artifact_key, canonical_kind, language_kind, name, parent_artifact_key, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, metadata) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            )
            .context("preparing content cache artefact insert")?;
        for artefact in &artefacts {
            let modifiers =
                serde_json::to_string(&artefact.modifiers).unwrap_or_else(|_| "[]".to_string());
            let metadata =
                serde_json::to_string(&artefact.metadata).unwrap_or_else(|_| "{}".to_string());
            affected_rows += stmt
                .execute(params![
                    extraction.content_id,
                    extraction.language,
                    extraction.parser_version,
                    extraction.extractor_version,
                    artefact.artifact_key,
                    artefact.canonical_kind,
                    artefact.language_kind,
                    artefact.name,
                    artefact.parent_artifact_key,
                    artefact.start_line,
                    artefact.end_line,
                    artefact.start_byte,
                    artefact.end_byte,
                    artefact.signature,
                    modifiers,
                    artefact.docstring,
                    metadata,
                ])
                .context("inserting cached artefact row")?;
        }
    }

    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO content_cache_edges \
                 (content_id, language, parser_version, extractor_version, edge_key, from_artifact_key, to_artifact_key, to_symbol_ref, edge_kind, start_line, end_line, metadata) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )
            .context("preparing content cache edge insert")?;
        for edge in &edges {
            let metadata =
                serde_json::to_string(&edge.metadata).unwrap_or_else(|_| "{}".to_string());
            affected_rows += stmt
                .execute(params![
                    extraction.content_id,
                    extraction.language,
                    extraction.parser_version,
                    extraction.extractor_version,
                    edge.edge_key,
                    edge.from_artifact_key,
                    edge.to_artifact_key,
                    edge.to_symbol_ref,
                    edge.edge_kind,
                    edge.start_line,
                    edge.end_line,
                    metadata,
                ])
                .context("inserting cached edge row")?;
        }
    }

    Ok(affected_rows)
}

pub(crate) fn touch_cache_entries_tx(
    tx: &Transaction<'_>,
    touches: &HashMap<CacheKey, bool>,
) -> Result<usize> {
    if touches.is_empty() {
        return Ok(0);
    }

    let mut affected_rows = 0usize;
    let mut stmt = tx
        .prepare(
            "UPDATE content_cache \
             SET retention_class = CASE WHEN ?5 <> 0 THEN 'git_backed' ELSE retention_class END, \
                 last_accessed_at = datetime('now') \
             WHERE content_id = ?1 AND language = ?2 AND parser_version = ?3 AND extractor_version = ?4",
        )
        .context("preparing content cache touch update")?;
    for (key, promote_to_git_backed) in touches {
        affected_rows += stmt
            .execute(params![
                key.content_id,
                key.language,
                key.parser_version,
                key.extractor_version,
                if *promote_to_git_backed { 1 } else { 0 },
            ])
            .context("touching content cache entry")?;
    }
    Ok(affected_rows)
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

pub(crate) async fn promote_to_git_backed(
    relational: &RelationalStorage,
    content_id: &str,
    language: &str,
    parser_version: &str,
    extractor_version: &str,
) -> Result<()> {
    promote_cached_content_to_git_backed(
        relational,
        content_id,
        language,
        parser_version,
        extractor_version,
    )
    .await
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

fn dedupe_last_wins<T: Clone, F>(items: &[T], key_fn: F) -> Vec<T>
where
    F: Fn(&T) -> &str,
{
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();

    for item in items.iter().rev() {
        let key = key_fn(item);
        if seen.insert(key.to_string()) {
            deduped.push(item.clone());
        }
    }

    deduped.reverse();
    deduped
}

pub(crate) fn deduped_cached_content_parts(
    extraction: &CachedExtraction,
) -> (Vec<CachedArtefact>, Vec<CachedEdge>) {
    (
        dedupe_last_wins(&extraction.artefacts, |artefact| {
            artefact.artifact_key.as_str()
        }),
        dedupe_last_wins(&extraction.edges, |edge| edge.edge_key.as_str()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tempfile::tempdir;

    async fn create_test_relational() -> RelationalStorage {
        let temp = tempdir().expect("temp dir");
        let sqlite_path = temp.path().join("devql.sqlite");
        crate::host::devql::init_sqlite_schema(&sqlite_path)
            .await
            .expect("initialise sqlite relational schema");
        let sqlite_path = temp.keep().join("devql.sqlite");
        RelationalStorage::local_only(sqlite_path)
    }

    async fn retention_class_for(
        relational: &RelationalStorage,
        content_id: &str,
    ) -> Option<String> {
        let sql = format!(
            "SELECT retention_class FROM content_cache \
WHERE content_id = '{}' AND language = 'rust' AND parser_version = 'parser-v1' AND extractor_version = 'extractor-v1'",
            esc_pg(content_id),
        );
        relational
            .query_rows(&sql)
            .await
            .expect("query content_cache")
            .first()
            .and_then(Value::as_object)
            .and_then(|row| row.get("retention_class"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    async fn count_rows(relational: &RelationalStorage, table: &str, content_id: &str) -> usize {
        let sql = format!(
            "SELECT COUNT(*) AS count FROM {} \
WHERE content_id = '{}' AND language = 'rust' AND parser_version = 'parser-v1' AND extractor_version = 'extractor-v1'",
            table,
            esc_pg(content_id),
        );
        relational
            .query_rows(&sql)
            .await
            .expect("query row count")
            .first()
            .and_then(Value::as_object)
            .and_then(|row| row.get("count"))
            .and_then(Value::as_i64)
            .unwrap_or_default() as usize
    }

    #[tokio::test]
    async fn worktree_only_promoted_to_git_backed_when_seen_as_head_blob() {
        let relational = create_test_relational().await;
        let extraction = CachedExtraction {
            content_id: "abc123".to_string(),
            language: "rust".to_string(),
            parser_version: "parser-v1".to_string(),
            extractor_version: "extractor-v1".to_string(),
            parse_status: "ok".to_string(),
            artefacts: vec![],
            edges: vec![],
        };

        store_cached_content(&relational, &extraction, "worktree_only")
            .await
            .expect("store worktree-only cache entry");
        assert_eq!(
            retention_class_for(&relational, &extraction.content_id)
                .await
                .as_deref(),
            Some("worktree_only")
        );

        promote_to_git_backed(
            &relational,
            &extraction.content_id,
            &extraction.language,
            &extraction.parser_version,
            &extraction.extractor_version,
        )
        .await
        .expect("promote cache entry");

        assert_eq!(
            retention_class_for(&relational, &extraction.content_id)
                .await
                .as_deref(),
            Some("git_backed")
        );
    }

    #[tokio::test]
    async fn store_cached_content_deduplicates_duplicate_keys() {
        let relational = create_test_relational().await;
        let extraction = CachedExtraction {
            content_id: "abc123".to_string(),
            language: "rust".to_string(),
            parser_version: "parser-v1".to_string(),
            extractor_version: "extractor-v1".to_string(),
            parse_status: "ok".to_string(),
            artefacts: vec![
                CachedArtefact {
                    artifact_key: "file::src/lib.rs".to_string(),
                    canonical_kind: Some("file".to_string()),
                    language_kind: "file".to_string(),
                    name: "src/lib.rs".to_string(),
                    parent_artifact_key: None,
                    start_line: 1,
                    end_line: 2,
                    start_byte: 0,
                    end_byte: 10,
                    signature: "fn old()".to_string(),
                    modifiers: vec!["pub".to_string()],
                    docstring: Some("old".to_string()),
                    metadata: Value::String("old".to_string()),
                },
                CachedArtefact {
                    artifact_key: "file::src/lib.rs".to_string(),
                    canonical_kind: Some("file".to_string()),
                    language_kind: "file".to_string(),
                    name: "src/lib.rs".to_string(),
                    parent_artifact_key: None,
                    start_line: 3,
                    end_line: 4,
                    start_byte: 11,
                    end_byte: 20,
                    signature: "fn new()".to_string(),
                    modifiers: vec!["pub".to_string(), "async".to_string()],
                    docstring: Some("new".to_string()),
                    metadata: Value::String("new".to_string()),
                },
            ],
            edges: vec![
                CachedEdge {
                    edge_key: "edge::call".to_string(),
                    from_artifact_key: "file::src/lib.rs".to_string(),
                    to_artifact_key: None,
                    to_symbol_ref: Some("old::target".to_string()),
                    edge_kind: "calls".to_string(),
                    start_line: Some(1),
                    end_line: Some(1),
                    metadata: Value::String("old".to_string()),
                },
                CachedEdge {
                    edge_key: "edge::call".to_string(),
                    from_artifact_key: "file::src/lib.rs".to_string(),
                    to_artifact_key: Some("file::target".to_string()),
                    to_symbol_ref: Some("new::target".to_string()),
                    edge_kind: "calls".to_string(),
                    start_line: Some(2),
                    end_line: Some(2),
                    metadata: Value::String("new".to_string()),
                },
            ],
        };

        store_cached_content(&relational, &extraction, "git_backed")
            .await
            .expect("store deduplicated cache entry");

        assert_eq!(
            count_rows(
                &relational,
                "content_cache_artefacts",
                &extraction.content_id
            )
            .await,
            1
        );
        assert_eq!(
            count_rows(&relational, "content_cache_edges", &extraction.content_id).await,
            1
        );

        let cached = lookup_cached_content(
            &relational,
            &extraction.content_id,
            &extraction.language,
            &extraction.parser_version,
            &extraction.extractor_version,
        )
        .await
        .expect("lookup stored cache entry")
        .expect("cache entry should exist");

        assert_eq!(cached.artefacts.len(), 1);
        assert_eq!(cached.edges.len(), 1);
        assert_eq!(cached.artefacts[0].start_line, 3);
        assert_eq!(cached.artefacts[0].end_line, 4);
        assert_eq!(cached.artefacts[0].signature, "fn new()");
        assert_eq!(cached.artefacts[0].docstring.as_deref(), Some("new"));
        assert_eq!(
            cached.artefacts[0].metadata,
            Value::String("new".to_string())
        );
        assert_eq!(
            cached.edges[0].to_artifact_key.as_deref(),
            Some("file::target")
        );
        assert_eq!(
            cached.edges[0].to_symbol_ref.as_deref(),
            Some("new::target")
        );
        assert_eq!(cached.edges[0].start_line, Some(2));
        assert_eq!(cached.edges[0].metadata, Value::String("new".to_string()));
    }
}
