#![allow(dead_code)]

mod current_edges;
mod current_state;
mod local_resolution;
mod reconcile;
#[cfg(test)]
mod tests;

use anyhow::Result;

use self::local_resolution::resolve_prepared_local_edges;
use super::super::content_cache::CachedExtraction;
use super::super::types::DesiredFileState;
use super::derive::prepare_materialization_rows;
use super::sql::{
    ArtefactInsertSqlInput, delete_artefacts_sql, delete_current_file_state_sql, delete_edges_sql,
    insert_artefact_sql, insert_edge_sql, upsert_current_file_state_sql,
};

pub(crate) use self::current_edges::reconcile_current_local_edges_for_paths;
#[cfg(test)]
pub(crate) use self::current_edges::{
    load_current_edges_for_local_reconciliation_with_connection,
    load_current_source_facts_for_paths_with_connection,
    load_current_targets_for_paths_for_local_resolution_with_connection,
    reconcile_current_local_edges_for_paths_with_write_lock,
};
pub(crate) use self::current_state::{persist_prepared_materialisation_tx, remove_paths_tx};
pub(crate) use self::local_resolution::resolve_prepared_local_edges_with_connection;

const SUPPORTED_LOCAL_RESOLUTION_LANGUAGES: &[&str] = &[
    "rust",
    "typescript",
    "javascript",
    "python",
    "go",
    "java",
    "csharp",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CurrentEdgeRecord {
    edge_id: String,
    path: String,
    content_id: String,
    from_symbol_id: String,
    from_artefact_id: String,
    to_symbol_id: Option<String>,
    to_artefact_id: Option<String>,
    to_symbol_ref: Option<String>,
    edge_kind: String,
    language: String,
    start_line: Option<i32>,
    end_line: Option<i32>,
    metadata_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurrentEdgeReplacement {
    old_edge_id: String,
    new_edges: Vec<CurrentEdgeRecord>,
}

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
    resolve_prepared_local_edges(cfg, relational, desired, &mut prepared).await?;

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

    relational.exec_batch_transactional(&statements).await?;
    reconcile_current_local_edges_for_paths(
        relational,
        &cfg.repo.repo_id,
        std::slice::from_ref(&desired.path),
    )
    .await?;
    Ok(())
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
        .await?;
    reconcile_current_local_edges_for_paths(relational, &cfg.repo.repo_id, &[path.to_string()])
        .await?;
    Ok(())
}
