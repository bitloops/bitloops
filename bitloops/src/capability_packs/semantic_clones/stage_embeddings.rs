//! Stage 2: symbol embedding rows (`symbol_embeddings`) for the semantic_clones pipeline.

mod orchestrator;
mod persistence;
mod schema;
mod sql;
mod storage;

#[cfg(test)]
#[path = "stage_embeddings_tests.rs"]
mod semantic_embedding_persistence_tests;

pub(crate) use self::orchestrator::{
    refresh_current_repo_symbol_embeddings_and_clone_edges, upsert_current_symbol_embedding_rows,
    upsert_symbol_embedding_rows,
};
pub(crate) use self::persistence::{
    clear_current_symbol_embedding_rows_for_path, clear_current_symbol_embedding_rows_for_paths,
    clear_repo_active_embedding_setup, clear_repo_active_embedding_setup_for_representation,
    clear_repo_symbol_embedding_rows, clear_repo_symbol_embedding_rows_for_representation,
    persist_active_embedding_setup,
};
pub(crate) use self::schema::{
    init_postgres_semantic_embeddings_schema, init_sqlite_semantic_embeddings_schema,
    semantic_embeddings_sqlite_schema_sql,
};
pub(crate) use self::sql::{
    build_active_embedding_setup_persist_sql, build_current_symbol_embedding_persist_sql,
    build_delete_stale_current_symbol_embedding_rows_for_path_sql,
    build_embedding_setup_persist_sql, build_postgres_current_symbol_embedding_persist_sql,
    build_postgres_symbol_embedding_persist_sql, build_sqlite_symbol_embedding_persist_sql,
};
pub(crate) use self::storage::{
    determine_repo_embedding_sync_action, load_active_embedding_setup,
    load_current_repo_embedding_states, load_current_semantic_summary_map,
    load_current_symbol_embedding_index_state, load_semantic_summary_map,
    load_symbol_embedding_index_state, load_symbol_embedding_index_states,
};

#[cfg(test)]
use self::sql::{
    build_semantic_summary_lookup_sql, build_symbol_embedding_index_state_sql, sql_json_string,
    sql_vector_string,
};
use anyhow::Result;

use crate::capability_packs::semantic_clones::embeddings;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::capability_packs::semantic_clones::vector_backend::SemanticVectorBackend;
use crate::host::devql::RelationalStorage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepoEmbeddingSyncAction {
    Incremental,
    AdoptExisting,
    RefreshCurrentRepo,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CurrentRepoEmbeddingRefreshResult {
    pub semantic_feature_stats: semantic::SemanticFeatureIngestionStats,
    pub embedding_stats: embeddings::SymbolEmbeddingIngestionStats,
    pub clone_build: crate::capability_packs::semantic_clones::scoring::SymbolCloneBuildResult,
}

pub(crate) async fn ensure_semantic_embeddings_schema(
    relational: &RelationalStorage,
) -> Result<()> {
    crate::host::devql::ensure_sqlite_schema_once(
        relational.sqlite_path(),
        "semantic_embeddings_sqlite",
        |sqlite_path| async move { init_sqlite_semantic_embeddings_schema(&sqlite_path).await },
    )
    .await?;
    SemanticVectorBackend::resolve(relational)
        .ensure_schema()
        .await?;
    if let Some(remote_client) = relational.remote_client() {
        crate::host::devql::ensure_sqlite_schema_once(
            relational.sqlite_path(),
            "semantic_embeddings_postgres",
            |_| async move { init_postgres_semantic_embeddings_schema(remote_client).await },
        )
        .await?;
    }
    Ok(())
}
