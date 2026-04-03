use anyhow::{Context, Result};
use rusqlite::{Transaction, params, params_from_iter};

use super::super::content_cache::CachedExtraction;
use super::super::types::DesiredFileState;
use super::derive::prepare_materialization_rows;
use super::sql::{
    bool_sql, delete_artefacts_sql, delete_current_file_state_sql, delete_edges_sql,
    insert_artefact_sql, insert_edge_sql, upsert_current_file_state_sql,
};
use super::types::PreparedMaterialisationRows;

pub(crate) async fn materialize_path(
    cfg: &crate::host::devql::DevqlConfig,
    relational: &crate::host::devql::RelationalStorage,
    desired: &DesiredFileState,
    extraction: &CachedExtraction,
    parser_version: &str,
    extractor_version: &str,
) -> Result<()> {
    let prepared =
        prepare_materialization_rows(cfg, desired, extraction, parser_version, extractor_version)?;

    let now_sql = crate::host::devql::sql_now(relational);
    let mut statements = vec![
        delete_edges_sql(&cfg.repo.repo_id, &desired.path),
        delete_artefacts_sql(&cfg.repo.repo_id, &desired.path),
    ];
    statements.extend(prepared.materialized_artefacts.iter().map(|artefact| {
        insert_artefact_sql(
            relational,
            &cfg.repo.repo_id,
            &desired.path,
            &desired.effective_content_id,
            &extraction.language,
            artefact,
            now_sql,
        )
    }));
    statements.extend(prepared.materialized_edges.iter().map(|edge| {
        insert_edge_sql(
            relational,
            &cfg.repo.repo_id,
            &desired.path,
            &desired.effective_content_id,
            edge,
            now_sql,
        )
    }));
    statements.push(upsert_current_file_state_sql(
        &cfg.repo.repo_id,
        desired,
        parser_version,
        extractor_version,
        now_sql,
    ));

    relational.exec_batch_transactional(&statements).await
}

pub(crate) async fn remove_path(
    cfg: &crate::host::devql::DevqlConfig,
    relational: &crate::host::devql::RelationalStorage,
    path: &str,
) -> Result<()> {
    relational
        .exec_batch_transactional(&[
            delete_edges_sql(&cfg.repo.repo_id, path),
            delete_artefacts_sql(&cfg.repo.repo_id, path),
            delete_current_file_state_sql(&cfg.repo.repo_id, path),
        ])
        .await
}

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
                 (repo_id, path, content_id, symbol_id, artefact_id, language, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, datetime('now'))",
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
             (repo_id, path, language, head_content_id, index_content_id, worktree_content_id, effective_content_id, effective_source, parser_version, extractor_version, exists_in_head, exists_in_index, exists_in_worktree, last_synced_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, datetime('now')) \
             ON CONFLICT (repo_id, path) DO UPDATE SET \
                 language = excluded.language, \
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
                desired.language,
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
