use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;

use crate::capability_packs::semantic_clones::embeddings;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::devql::{RelationalStorage, esc_pg, sql_string_list_pg};
use crate::host::inference::EmbeddingService;

use super::ensure_semantic_embeddings_schema;
use super::sql::{
    build_active_embedding_setup_persist_sql, build_current_symbol_embedding_persist_sql,
    build_embedding_setup_persist_sql, build_sqlite_symbol_embedding_persist_sql,
    representation_kind_sql_predicate,
};

#[allow(dead_code)]
pub(crate) async fn clear_repo_symbol_embedding_rows(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<()> {
    ensure_semantic_embeddings_schema(relational).await?;
    relational
        .exec_batch_transactional(&[
            format!(
                "DELETE FROM symbol_embeddings WHERE repo_id = '{}'",
                esc_pg(repo_id),
            ),
            format!(
                "DELETE FROM symbol_embeddings_current WHERE repo_id = '{}'",
                esc_pg(repo_id),
            ),
        ])
        .await
}

pub(crate) async fn clear_repo_symbol_embedding_rows_for_representation(
    relational: &RelationalStorage,
    repo_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> Result<()> {
    ensure_semantic_embeddings_schema(relational).await?;
    let predicate = representation_kind_sql_predicate("representation_kind", representation_kind);
    relational
        .exec_batch_transactional(&[
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
        ])
        .await
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
    relational.exec(&sql).await
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
    relational.exec(&sql).await
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
    relational.exec(&sql).await
}

pub(crate) async fn persist_active_embedding_setup(
    relational: &RelationalStorage,
    repo_id: &str,
    active_state: &embeddings::ActiveEmbeddingRepresentationState,
) -> Result<()> {
    ensure_semantic_embeddings_schema(relational).await?;
    persist_embedding_setup(relational, &active_state.setup).await?;
    let sql = build_active_embedding_setup_persist_sql(repo_id, active_state);
    relational.exec(&sql).await
}

pub(super) async fn persist_symbol_embedding_row(
    relational: &RelationalStorage,
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<()> {
    persist_embedding_setup(
        relational,
        &embeddings::EmbeddingSetup {
            provider: row.provider.clone(),
            model: row.model.clone(),
            dimension: row.dimension,
            setup_fingerprint: row.setup_fingerprint.clone(),
        },
    )
    .await?;
    let sql = build_sqlite_symbol_embedding_persist_sql(row)?;
    relational.exec(&sql).await
}

#[allow(dead_code)]
pub(super) async fn persist_current_symbol_embedding_row(
    relational: &RelationalStorage,
    input: &semantic::SemanticFeatureInput,
    path: &str,
    content_id: &str,
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<()> {
    persist_embedding_setup(
        relational,
        &embeddings::EmbeddingSetup {
            provider: row.provider.clone(),
            model: row.model.clone(),
            dimension: row.dimension,
            setup_fingerprint: row.setup_fingerprint.clone(),
        },
    )
    .await?;
    let sql = build_current_symbol_embedding_persist_sql(input, path, content_id, row)?;
    relational.exec(&sql).await
}

pub(super) async fn delete_stale_current_symbol_embedding_rows_for_path(
    relational: &RelationalStorage,
    repo_id: &str,
    path: &str,
    content_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    keep_artefact_ids: &[String],
) -> Result<()> {
    let extra_delete_clause = if keep_artefact_ids.is_empty() {
        " OR 1 = 1".to_string()
    } else {
        format!(
            " OR artefact_id NOT IN ({})",
            sql_string_list_pg(keep_artefact_ids)
        )
    };
    let sql = format!(
        "DELETE FROM symbol_embeddings_current \
WHERE repo_id = '{repo_id}' AND path = '{path}' AND {representation_predicate} \
  AND (content_id <> '{content_id}'{extra_delete_clause})",
        repo_id = esc_pg(repo_id),
        path = esc_pg(path),
        content_id = esc_pg(content_id),
        representation_predicate =
            representation_kind_sql_predicate("representation_kind", representation_kind),
        extra_delete_clause = extra_delete_clause,
    );
    relational.exec(&sql).await
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

async fn persist_embedding_setup(
    relational: &RelationalStorage,
    setup: &embeddings::EmbeddingSetup,
) -> Result<()> {
    relational
        .exec(&build_embedding_setup_persist_sql(setup))
        .await
}
