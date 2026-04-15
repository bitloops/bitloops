#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use rusqlite::{Connection, Transaction, params, params_from_iter};

use super::super::content_cache::CachedExtraction;
use super::super::types::DesiredFileState;
use super::derive::prepare_materialization_rows;
use super::sql::{
    ArtefactInsertSqlInput, bool_sql, delete_artefacts_sql, delete_current_file_state_sql,
    delete_edges_sql, insert_artefact_sql, insert_edge_sql, upsert_current_file_state_sql,
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
    let mut prepared =
        prepare_materialization_rows(cfg, desired, extraction, parser_version, extractor_version)?;
    resolve_prepared_rust_local_edges(cfg, relational, desired, &mut prepared).await?;

    let now_sql = crate::host::devql::sql_now(relational);
    let mut statements = vec![
        delete_edges_sql(&cfg.repo.repo_id, &desired.path),
        delete_artefacts_sql(&cfg.repo.repo_id, &desired.path),
    ];
    statements.extend(prepared.materialized_artefacts.iter().map(|artefact| {
        insert_artefact_sql(
            relational,
            &ArtefactInsertSqlInput {
                repo_id: &cfg.repo.repo_id,
                path: &desired.path,
                content_id: &desired.effective_content_id,
                language: &extraction.language,
                extraction_fingerprint: &desired.extraction_fingerprint,
                artefact,
                now_sql,
            },
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

pub(crate) async fn resolve_prepared_rust_local_edges(
    cfg: &crate::host::devql::DevqlConfig,
    relational: &crate::host::devql::RelationalStorage,
    desired: &DesiredFileState,
    prepared: &mut PreparedMaterialisationRows,
) -> Result<()> {
    let pending =
        pending_rust_local_resolutions(desired.path.as_str(), &prepared.materialized_edges);
    if pending.is_empty() {
        return Ok(());
    }
    let candidate_fqns = pending
        .iter()
        .flat_map(|(_, candidates)| candidates.iter().cloned())
        .collect::<HashSet<_>>();
    let current_targets = load_current_targets_for_fqns(
        relational,
        &cfg.repo.repo_id,
        &desired.path,
        &candidate_fqns,
    )
    .await?;
    apply_rust_local_edge_resolutions(cfg, desired, prepared, &pending, &current_targets);
    Ok(())
}

pub(crate) fn resolve_prepared_rust_local_edges_with_connection(
    connection: &Connection,
    cfg: &crate::host::devql::DevqlConfig,
    desired: &DesiredFileState,
    prepared: &mut PreparedMaterialisationRows,
) -> Result<()> {
    let pending =
        pending_rust_local_resolutions(desired.path.as_str(), &prepared.materialized_edges);
    if pending.is_empty() {
        return Ok(());
    }
    let candidate_fqns = pending
        .iter()
        .flat_map(|(_, candidates)| candidates.iter().cloned())
        .collect::<HashSet<_>>();
    let current_targets = load_current_targets_for_fqns_with_connection(
        connection,
        &cfg.repo.repo_id,
        &desired.path,
        &candidate_fqns,
    )?;
    apply_rust_local_edge_resolutions(cfg, desired, prepared, &pending, &current_targets);
    Ok(())
}

fn pending_rust_local_resolutions(
    source_path: &str,
    edges: &[super::types::MaterializedEdge],
) -> Vec<(usize, Vec<String>)> {
    edges
        .iter()
        .enumerate()
        .filter_map(|(idx, edge)| {
            if edge.language != "rust" || edge.to_symbol_id.is_some() {
                return None;
            }
            let symbol_ref = edge.to_symbol_ref.as_deref()?;
            let candidates = crate::host::language_adapter::rust_local_symbol_fqn_candidates(
                source_path,
                symbol_ref,
            );
            (!candidates.is_empty()).then_some((idx, candidates))
        })
        .collect()
}

fn apply_rust_local_edge_resolutions(
    cfg: &crate::host::devql::DevqlConfig,
    desired: &DesiredFileState,
    prepared: &mut PreparedMaterialisationRows,
    pending: &[(usize, Vec<String>)],
    current_targets: &HashMap<String, (String, String)>,
) {
    let in_flight_targets = prepared
        .materialized_artefacts
        .iter()
        .map(|artefact| {
            (
                artefact.symbol_fqn.clone(),
                (artefact.symbol_id.clone(), artefact.artefact_id.clone()),
            )
        })
        .collect::<HashMap<_, _>>();

    for (edge_idx, candidates) in pending {
        let resolved = candidates
            .iter()
            .find_map(|candidate| {
                in_flight_targets
                    .get(candidate)
                    .cloned()
                    .map(|target| (candidate.clone(), target))
            })
            .or_else(|| {
                candidates.iter().find_map(|candidate| {
                    current_targets
                        .get(candidate)
                        .cloned()
                        .map(|target| (candidate.clone(), target))
                })
            });
        let Some((resolved_fqn, (symbol_id, artefact_id))) = resolved else {
            continue;
        };
        let edge = &mut prepared.materialized_edges[*edge_idx];
        edge.to_symbol_id = Some(symbol_id);
        edge.to_artefact_id = Some(artefact_id);
        edge.to_symbol_ref = Some(resolved_fqn);
        edge.edge_id = crate::host::devql::deterministic_uuid(&format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}",
            cfg.repo.repo_id,
            desired.path,
            edge.from_symbol_id,
            edge.edge_kind,
            edge.to_symbol_id.clone().unwrap_or_default(),
            edge.to_symbol_ref.clone().unwrap_or_default(),
            edge.start_line.unwrap_or(-1),
            edge.end_line.unwrap_or(-1),
            edge.metadata,
        ));
    }
}

async fn load_current_targets_for_fqns(
    relational: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    current_path: &str,
    candidate_fqns: &HashSet<String>,
) -> Result<HashMap<String, (String, String)>> {
    if candidate_fqns.is_empty() {
        return Ok(HashMap::new());
    }

    let in_list = candidate_fqns
        .iter()
        .map(|candidate| format!("'{}'", crate::host::devql::esc_pg(candidate)))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT symbol_fqn, symbol_id, artefact_id \
         FROM artefacts_current \
         WHERE repo_id = '{}' AND path != '{}' AND symbol_fqn IN ({in_list})",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(current_path),
    );
    let rows = relational.query_rows(&sql).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let obj = row.as_object()?;
            Some((
                obj.get("symbol_fqn")?.as_str()?.to_string(),
                (
                    obj.get("symbol_id")?.as_str()?.to_string(),
                    obj.get("artefact_id")?.as_str()?.to_string(),
                ),
            ))
        })
        .collect())
}

fn load_current_targets_for_fqns_with_connection(
    connection: &Connection,
    repo_id: &str,
    current_path: &str,
    candidate_fqns: &HashSet<String>,
) -> Result<HashMap<String, (String, String)>> {
    if candidate_fqns.is_empty() {
        return Ok(HashMap::new());
    }

    let in_list = candidate_fqns
        .iter()
        .map(|candidate| format!("'{}'", crate::host::devql::esc_pg(candidate)))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT symbol_fqn, symbol_id, artefact_id \
         FROM artefacts_current \
         WHERE repo_id = '{}' AND path != '{}' AND symbol_fqn IN ({in_list})",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(current_path),
    );
    let mut stmt = connection
        .prepare(&sql)
        .context("preparing current Rust target lookup query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (row.get::<_, String>(1)?, row.get::<_, String>(2)?),
            ))
        })
        .context("querying current Rust target lookup rows")?
        .collect::<Result<Vec<_>, _>>()
        .context("collecting current Rust target lookup rows")?;
    Ok(rows.into_iter().collect())
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
