pub mod descriptor;
pub mod embeddings;
pub mod features;
pub mod health;
pub mod ingesters;
pub mod migrations;
pub mod pack;
pub mod pipeline;
pub mod query_examples;
pub mod register;
pub(crate) mod runtime_config;
pub mod schema;
pub mod schema_module;
pub mod scoring;
pub mod stages;
pub mod types;

mod stage_embeddings;
mod stage_semantic_features;

#[allow(unused_imports)]
pub(crate) use stage_embeddings::{
    RepoEmbeddingSyncAction, clear_current_symbol_embedding_rows_for_path,
    clear_repo_active_embedding_setup, clear_repo_active_embedding_setup_for_representation,
    clear_repo_symbol_embedding_rows, clear_repo_symbol_embedding_rows_for_representation,
    determine_repo_embedding_sync_action, ensure_semantic_embeddings_schema,
    init_postgres_semantic_embeddings_schema, init_sqlite_semantic_embeddings_schema,
    load_active_embedding_setup, load_current_repo_embedding_states,
    persist_active_embedding_setup, refresh_current_repo_symbol_embeddings_and_clone_edges,
    upsert_current_symbol_embedding_rows, upsert_symbol_embedding_rows,
};
pub(crate) use stage_semantic_features::{
    clear_current_semantic_feature_rows_for_path, ensure_semantic_features_schema,
    init_postgres_semantic_features_schema, init_sqlite_semantic_features_schema,
    load_pre_stage_artefacts_for_blob, load_pre_stage_dependencies_for_blob,
    load_semantic_feature_inputs_for_artefacts, load_semantic_feature_inputs_for_current_repo,
    load_semantic_summary_snapshot, upsert_current_semantic_feature_rows,
    upsert_semantic_feature_rows,
};

pub use pack::SemanticClonesPack;
pub use types::{
    SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
    SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID,
    SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
};
