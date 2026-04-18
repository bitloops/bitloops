use anyhow::{Context, Result};
use rusqlite::{Transaction, params, params_from_iter};

use super::super::super::content_cache::CachedExtraction;
use super::super::super::types::DesiredFileState;
use super::super::bool_sql;
use super::super::types::PreparedMaterialisationRows;

pub(crate) fn persist_prepared_materialisation_tx(
    tx: &Transaction<'_>,
    repo_id: &str,
    desired: &DesiredFileState,
    extraction: &CachedExtraction,
    prepared: &PreparedMaterialisationRows,
    parser_version: &str,
    extractor_version: &str,
) -> Result<usize> {
    let mut affected_rows = 0usize;

    affected_rows += tx
        .execute(
            "DELETE FROM artefact_edges_current WHERE repo_id = ?1 AND path = ?2",
            params![repo_id, desired.path],
        )
        .context("deleting materialised edge rows for path")?;
    affected_rows += tx
        .execute(
            "DELETE FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            params![repo_id, desired.path],
        )
        .context("deleting materialised artefact rows for path")?;

    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO artefacts_current \
                 (repo_id, path, content_id, symbol_id, artefact_id, language, extraction_fingerprint, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, datetime('now'))",
            )
            .context("preparing current artefact insert")?;
        for artefact in &prepared.materialized_artefacts {
            let modifiers =
                serde_json::to_string(&artefact.modifiers).unwrap_or_else(|_| "[]".to_string());
            affected_rows += stmt
                .execute(params![
                    repo_id,
                    desired.path,
                    desired.effective_content_id,
                    artefact.symbol_id,
                    artefact.artefact_id,
                    extraction.language,
                    desired.extraction_fingerprint,
                    artefact.canonical_kind,
                    artefact.language_kind,
                    artefact.symbol_fqn,
                    artefact.parent_symbol_id,
                    artefact.parent_artefact_id,
                    artefact.start_line,
                    artefact.end_line,
                    artefact.start_byte,
                    artefact.end_byte,
                    artefact.signature,
                    modifiers,
                    artefact.docstring,
                ])
                .context("inserting current artefact row")?;
        }
    }

    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO artefact_edges_current \
                 (repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, datetime('now'))",
            )
            .context("preparing current edge insert")?;
        for edge in &prepared.materialized_edges {
            let metadata =
                serde_json::to_string(&edge.metadata).unwrap_or_else(|_| "{}".to_string());
            affected_rows += stmt
                .execute(params![
                    repo_id,
                    edge.edge_id,
                    desired.path,
                    desired.effective_content_id,
                    edge.from_symbol_id,
                    edge.from_artefact_id,
                    edge.to_symbol_id,
                    edge.to_artefact_id,
                    edge.to_symbol_ref,
                    edge.edge_kind,
                    edge.language,
                    edge.start_line,
                    edge.end_line,
                    metadata,
                ])
                .context("inserting current edge row")?;
        }
    }

    affected_rows += tx
        .execute(
            "INSERT INTO current_file_state \
             (repo_id, path, analysis_mode, file_role, text_index_mode, language, resolved_language, dialect, primary_context_id, secondary_context_ids_json, frameworks_json, runtime_profile, classification_reason, context_fingerprint, extraction_fingerprint, head_content_id, index_content_id, worktree_content_id, effective_content_id, effective_source, parser_version, extractor_version, exists_in_head, exists_in_index, exists_in_worktree, last_synced_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, datetime('now')) \
             ON CONFLICT (repo_id, path) DO UPDATE SET \
                 analysis_mode = excluded.analysis_mode, \
                 file_role = excluded.file_role, \
                 text_index_mode = excluded.text_index_mode, \
                 language = excluded.language, \
                 resolved_language = excluded.resolved_language, \
                 dialect = excluded.dialect, \
                 primary_context_id = excluded.primary_context_id, \
                 secondary_context_ids_json = excluded.secondary_context_ids_json, \
                 frameworks_json = excluded.frameworks_json, \
                 runtime_profile = excluded.runtime_profile, \
                 classification_reason = excluded.classification_reason, \
                 context_fingerprint = excluded.context_fingerprint, \
                 extraction_fingerprint = excluded.extraction_fingerprint, \
                 head_content_id = excluded.head_content_id, \
                 index_content_id = excluded.index_content_id, \
                 worktree_content_id = excluded.worktree_content_id, \
                 effective_content_id = excluded.effective_content_id, \
                 effective_source = excluded.effective_source, \
                 parser_version = excluded.parser_version, \
                 extractor_version = excluded.extractor_version, \
                 exists_in_head = excluded.exists_in_head, \
                 exists_in_index = excluded.exists_in_index, \
                 exists_in_worktree = excluded.exists_in_worktree, \
                 last_synced_at = excluded.last_synced_at",
            params![
                repo_id,
                desired.path,
                desired.analysis_mode.as_str(),
                desired.file_role.as_str(),
                desired.text_index_mode.as_str(),
                desired.language,
                desired.resolved_language,
                desired.dialect,
                desired.primary_context_id,
                serde_json::to_string(&desired.secondary_context_ids)
                    .unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&desired.frameworks)
                    .unwrap_or_else(|_| "[]".to_string()),
                desired.runtime_profile,
                desired.classification_reason,
                desired.context_fingerprint,
                desired.extraction_fingerprint,
                desired.head_content_id,
                desired.index_content_id,
                desired.worktree_content_id,
                desired.effective_content_id,
                desired.effective_source.as_str(),
                parser_version,
                extractor_version,
                bool_sql(desired.exists_in_head),
                bool_sql(desired.exists_in_index),
                bool_sql(desired.exists_in_worktree),
            ],
        )
        .context("upserting current file state row")?;

    Ok(affected_rows)
}

pub(crate) fn remove_paths_tx(
    tx: &Transaction<'_>,
    repo_id: &str,
    paths: &[String],
) -> Result<usize> {
    if paths.is_empty() {
        return Ok(0);
    }

    let mut affected_rows = 0usize;
    affected_rows += delete_paths_tx(
        tx,
        "artefact_edges_current",
        repo_id,
        paths,
        "deleting current edge rows for removed paths",
    )?;
    affected_rows += delete_paths_tx(
        tx,
        "artefacts_current",
        repo_id,
        paths,
        "deleting current artefact rows for removed paths",
    )?;
    affected_rows += delete_paths_tx(
        tx,
        "current_file_state",
        repo_id,
        paths,
        "deleting current file state rows for removed paths",
    )?;
    Ok(affected_rows)
}

fn delete_paths_tx(
    tx: &Transaction<'_>,
    table: &str,
    repo_id: &str,
    paths: &[String],
    context: &str,
) -> Result<usize> {
    let placeholders = std::iter::repeat_n("?", paths.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("DELETE FROM {table} WHERE repo_id = ? AND path IN ({placeholders})");
    let params = std::iter::once(repo_id.to_string()).chain(paths.iter().cloned());
    tx.execute(&sql, params_from_iter(params))
        .with_context(|| context.to_string())
}
