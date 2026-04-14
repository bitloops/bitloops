use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use rusqlite::{Transaction, params};

use crate::host::devql::RelationalStorage;
use crate::host::devql::db_utils::esc_pg;

use super::sql::{
    build_delete_cached_artefacts_sql, build_delete_cached_edges_sql,
    build_insert_cached_artefact_sql, build_insert_cached_edge_sql, build_upsert_cached_header_sql,
};
use super::types::{CacheKey, CachedArtefact, CachedEdge, CachedExtraction};

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
            "INSERT INTO content_cache (content_id, language, extraction_fingerprint, parser_version, extractor_version, retention_class, parse_status, parsed_at, last_accessed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'), datetime('now')) \
             ON CONFLICT (content_id, language, extraction_fingerprint, parser_version, extractor_version) DO UPDATE SET \
                 retention_class = excluded.retention_class, \
                 parse_status = excluded.parse_status, \
                 parsed_at = excluded.parsed_at, \
                 last_accessed_at = excluded.last_accessed_at",
            params![
                extraction.content_id,
                extraction.language,
                extraction.extraction_fingerprint,
                extraction.parser_version,
                extraction.extractor_version,
                retention_class,
                extraction.parse_status,
            ],
        )
        .context("upserting content cache header")?;

    affected_rows += tx
        .execute(
            "DELETE FROM content_cache_edges WHERE content_id = ?1 AND language = ?2 AND extraction_fingerprint = ?3 AND parser_version = ?4 AND extractor_version = ?5",
            params![
                extraction.content_id,
                extraction.language,
                extraction.extraction_fingerprint,
                extraction.parser_version,
                extraction.extractor_version,
            ],
        )
        .context("deleting cached content edges before rewrite")?;
    affected_rows += tx
        .execute(
            "DELETE FROM content_cache_artefacts WHERE content_id = ?1 AND language = ?2 AND extraction_fingerprint = ?3 AND parser_version = ?4 AND extractor_version = ?5",
            params![
                extraction.content_id,
                extraction.language,
                extraction.extraction_fingerprint,
                extraction.parser_version,
                extraction.extractor_version,
            ],
        )
        .context("deleting cached content artefacts before rewrite")?;

    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO content_cache_artefacts \
                 (content_id, language, extraction_fingerprint, parser_version, extractor_version, artifact_key, canonical_kind, language_kind, name, parent_artifact_key, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, metadata) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
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
                    extraction.extraction_fingerprint,
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
                 (content_id, language, extraction_fingerprint, parser_version, extractor_version, edge_key, from_artifact_key, to_artifact_key, to_symbol_ref, edge_kind, start_line, end_line, metadata) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            )
            .context("preparing content cache edge insert")?;
        for edge in &edges {
            let metadata =
                serde_json::to_string(&edge.metadata).unwrap_or_else(|_| "{}".to_string());
            affected_rows += stmt
                .execute(params![
                    extraction.content_id,
                    extraction.language,
                    extraction.extraction_fingerprint,
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
             SET retention_class = CASE WHEN ?6 <> 0 THEN 'git_backed' ELSE retention_class END, \
                 last_accessed_at = datetime('now') \
             WHERE content_id = ?1 AND language = ?2 AND extraction_fingerprint = ?3 AND parser_version = ?4 AND extractor_version = ?5",
        )
        .context("preparing content cache touch update")?;
    for (key, promote_to_git_backed) in touches {
        affected_rows += stmt
            .execute(params![
                key.content_id,
                key.language,
                key.extraction_fingerprint,
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
    extraction_fingerprint: &str,
    parser_version: &str,
    extractor_version: &str,
) -> Result<()> {
    let sql = format!(
        "UPDATE content_cache \
SET retention_class = 'git_backed' \
WHERE content_id = '{}' AND language = '{}' AND extraction_fingerprint = '{}' AND parser_version = '{}' AND extractor_version = '{}' \
AND retention_class = 'worktree_only'",
        esc_pg(content_id),
        esc_pg(language),
        esc_pg(extraction_fingerprint),
        esc_pg(parser_version),
        esc_pg(extractor_version),
    );
    relational.exec(&sql).await
}

pub(crate) async fn promote_to_git_backed(
    relational: &RelationalStorage,
    content_id: &str,
    language: &str,
    extraction_fingerprint: &str,
    parser_version: &str,
    extractor_version: &str,
) -> Result<()> {
    promote_cached_content_to_git_backed(
        relational,
        content_id,
        language,
        extraction_fingerprint,
        parser_version,
        extractor_version,
    )
    .await
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
