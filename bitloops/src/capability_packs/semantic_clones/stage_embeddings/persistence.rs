use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;

use crate::capability_packs::semantic_clones::embeddings;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::capability_packs::semantic_clones::vector_backend::{
    clear_sqlite_current_rows_for_paths, clear_sqlite_repo_rows,
    clear_sqlite_repo_rows_for_representation, delete_sqlite_stale_current_rows_for_path,
    ensure_postgres_pgvector_indexes_for_dimension, sync_sqlite_current_symbol_embedding_row,
};
use crate::host::devql::{RelationalPrimaryBackend, RelationalStorage, esc_pg, sql_string_list_pg};
use crate::host::inference::EmbeddingService;

use super::ensure_semantic_embeddings_schema;
use super::sql::{
    build_active_embedding_setup_persist_sql, build_current_symbol_embedding_persist_sql,
    build_delete_stale_current_symbol_embedding_rows_for_path_sql,
    build_embedding_setup_persist_sql, build_postgres_current_symbol_embedding_persist_sql,
    build_postgres_symbol_embedding_persist_sql, build_sqlite_symbol_embedding_persist_sql,
    representation_kind_sql_predicate,
};

async fn execute_remote_primary_batch_if_needed(
    relational: &RelationalStorage,
    statements: &[String],
) -> Result<()> {
    if !matches!(
        relational.primary_backend(),
        RelationalPrimaryBackend::Postgres
    ) || statements.is_empty()
    {
        return Ok(());
    }
    relational.exec_remote_batch_transactional(statements).await
}

#[allow(dead_code)]
pub(crate) async fn clear_repo_symbol_embedding_rows(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<()> {
    ensure_semantic_embeddings_schema(relational).await?;
    let statements = vec![
        format!(
            "DELETE FROM symbol_embeddings WHERE repo_id = '{}'",
            esc_pg(repo_id),
        ),
        format!(
            "DELETE FROM symbol_embeddings_current WHERE repo_id = '{}'",
            esc_pg(repo_id),
        ),
    ];
    relational
        .exec_serialized_batch_transactional(&statements)
        .await?;
    execute_remote_primary_batch_if_needed(relational, &statements).await?;
    clear_sqlite_repo_rows(relational, repo_id).await
}

pub(crate) async fn clear_repo_symbol_embedding_rows_for_representation(
    relational: &RelationalStorage,
    repo_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> Result<()> {
    ensure_semantic_embeddings_schema(relational).await?;
    let predicate = representation_kind_sql_predicate("representation_kind", representation_kind);
    let statements = vec![
        format!(
            "DELETE FROM symbol_embeddings WHERE repo_id = '{repo_id}' AND {predicate}",
            repo_id = esc_pg(repo_id),
            predicate = predicate,
        ),
        format!(
            "DELETE FROM symbol_embeddings_current WHERE repo_id = '{repo_id}' AND {predicate}",
            repo_id = esc_pg(repo_id),
            predicate = predicate,
        ),
    ];
    relational
        .exec_serialized_batch_transactional(&statements)
        .await?;
    execute_remote_primary_batch_if_needed(relational, &statements).await?;
    clear_sqlite_repo_rows_for_representation(relational, repo_id, representation_kind).await
}

#[allow(dead_code)]
pub(crate) async fn clear_current_symbol_embedding_rows_for_path(
    relational: &RelationalStorage,
    repo_id: &str,
    path: &str,
) -> Result<()> {
    ensure_semantic_embeddings_schema(relational).await?;
    let sql = format!(
        "DELETE FROM symbol_embeddings_current WHERE repo_id = '{}' AND path = '{}'",
        esc_pg(repo_id),
        esc_pg(path),
    );
    relational.exec_serialized(&sql).await?;
    execute_remote_primary_batch_if_needed(relational, &[sql]).await?;
    clear_sqlite_current_rows_for_paths(relational, repo_id, &[path.to_string()]).await
}

pub(crate) async fn clear_current_symbol_embedding_rows_for_paths(
    relational: &RelationalStorage,
    repo_id: &str,
    paths: &[String],
) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    ensure_semantic_embeddings_schema(relational).await?;
    let sql = format!(
        "DELETE FROM symbol_embeddings_current WHERE repo_id = '{}' AND path IN ({})",
        esc_pg(repo_id),
        sql_string_list_pg(paths),
    );
    relational.exec_serialized(&sql).await?;
    execute_remote_primary_batch_if_needed(relational, &[sql]).await?;
    clear_sqlite_current_rows_for_paths(relational, repo_id, paths).await
}

pub(crate) async fn clear_repo_active_embedding_setup(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<()> {
    ensure_semantic_embeddings_schema(relational).await?;
    let sql = format!(
        "DELETE FROM semantic_clone_embedding_setup_state WHERE repo_id = '{}'",
        esc_pg(repo_id),
    );
    relational.exec_serialized(&sql).await?;
    execute_remote_primary_batch_if_needed(relational, &[sql]).await
}

pub(crate) async fn clear_repo_active_embedding_setup_for_representation(
    relational: &RelationalStorage,
    repo_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> Result<()> {
    ensure_semantic_embeddings_schema(relational).await?;
    let sql = format!(
        "DELETE FROM semantic_clone_embedding_setup_state WHERE repo_id = '{repo_id}' AND {representation_predicate}",
        repo_id = esc_pg(repo_id),
        representation_predicate =
            representation_kind_sql_predicate("representation_kind", representation_kind),
    );
    relational.exec_serialized(&sql).await?;
    execute_remote_primary_batch_if_needed(relational, &[sql]).await
}

pub(crate) async fn persist_active_embedding_setup(
    relational: &RelationalStorage,
    repo_id: &str,
    active_state: &embeddings::ActiveEmbeddingRepresentationState,
) -> Result<()> {
    ensure_semantic_embeddings_schema(relational).await?;
    let statements = vec![
        build_embedding_setup_persist_sql(&active_state.setup),
        build_active_embedding_setup_persist_sql(repo_id, active_state),
    ];
    relational
        .exec_serialized_batch_transactional(&statements)
        .await?;
    execute_remote_primary_batch_if_needed(relational, &statements).await
}

pub(super) async fn persist_symbol_embedding_row(
    relational: &RelationalStorage,
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<()> {
    let setup = embeddings::EmbeddingSetup {
        provider: row.provider.clone(),
        model: row.model.clone(),
        dimension: row.dimension,
        setup_fingerprint: row.setup_fingerprint.clone(),
    };
    let local_statements = vec![
        build_embedding_setup_persist_sql(&setup),
        build_sqlite_symbol_embedding_persist_sql(row)?,
    ];
    relational
        .exec_serialized_batch_transactional(&local_statements)
        .await?;
    let remote_statements = vec![
        build_embedding_setup_persist_sql(&setup),
        build_postgres_symbol_embedding_persist_sql(row)?,
    ];
    execute_remote_primary_batch_if_needed(relational, &remote_statements).await?;
    ensure_postgres_pgvector_indexes_for_dimension(relational, row.dimension).await
}

#[allow(dead_code)]
pub(super) async fn persist_current_symbol_embedding_row(
    relational: &RelationalStorage,
    input: &semantic::SemanticFeatureInput,
    path: &str,
    content_id: &str,
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<()> {
    let setup = embeddings::EmbeddingSetup {
        provider: row.provider.clone(),
        model: row.model.clone(),
        dimension: row.dimension,
        setup_fingerprint: row.setup_fingerprint.clone(),
    };
    let local_statements = vec![
        build_embedding_setup_persist_sql(&setup),
        build_current_symbol_embedding_persist_sql(input, path, content_id, row)?,
    ];
    relational
        .exec_serialized_batch_transactional(&local_statements)
        .await?;
    let remote_statements = vec![
        build_embedding_setup_persist_sql(&setup),
        build_postgres_current_symbol_embedding_persist_sql(input, path, content_id, row)?,
    ];
    execute_remote_primary_batch_if_needed(relational, &remote_statements).await?;
    sync_sqlite_current_symbol_embedding_row(relational, path, row).await?;
    ensure_postgres_pgvector_indexes_for_dimension(relational, row.dimension).await
}

pub(super) async fn delete_stale_current_symbol_embedding_rows_for_path(
    relational: &RelationalStorage,
    repo_id: &str,
    path: &str,
    content_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    keep_artefact_ids: &[String],
) -> Result<()> {
    let sql = build_delete_stale_current_symbol_embedding_rows_for_path_sql(
        repo_id,
        path,
        content_id,
        representation_kind,
        keep_artefact_ids,
    );
    relational.exec_serialized(&sql).await?;
    execute_remote_primary_batch_if_needed(relational, &[sql]).await?;
    delete_sqlite_stale_current_rows_for_path(
        relational,
        repo_id,
        path,
        representation_kind,
        keep_artefact_ids,
    )
    .await
}

pub(super) async fn upsert_current_repo_symbol_embedding_rows(
    relational: &RelationalStorage,
    inputs: &[semantic::SemanticFeatureInput],
    representation_kind: embeddings::EmbeddingRepresentationKind,
    embedding_provider: Arc<dyn EmbeddingService>,
) -> Result<embeddings::SymbolEmbeddingIngestionStats> {
    let mut grouped = BTreeMap::<(String, String), Vec<semantic::SemanticFeatureInput>>::new();
    for input in inputs {
        grouped
            .entry((input.path.clone(), input.blob_sha.clone()))
            .or_default()
            .push(input.clone());
    }

    let mut stats = embeddings::SymbolEmbeddingIngestionStats::default();
    for ((path, content_id), path_inputs) in grouped {
        let path_stats =
            crate::capability_packs::semantic_clones::upsert_current_symbol_embedding_rows(
                relational,
                &path,
                &content_id,
                &path_inputs,
                representation_kind,
                Arc::clone(&embedding_provider),
            )
            .await?;
        stats.eligible += path_stats.eligible;
        stats.upserted += path_stats.upserted;
        stats.skipped += path_stats.skipped;
    }
    Ok(stats)
}
