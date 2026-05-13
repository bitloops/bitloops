pub mod current_state;
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
pub(crate) mod vector_backend;
pub(crate) mod workplane;

pub(crate) mod stage_embeddings;
mod stage_search_documents;
mod stage_semantic_features;

#[allow(unused_imports)]
pub(crate) use stage_embeddings::{
    RepoEmbeddingSyncAction, build_active_embedding_setup_persist_sql,
    build_current_symbol_embedding_persist_sql,
    build_delete_stale_current_symbol_embedding_rows_for_path_sql,
    build_embedding_setup_persist_sql, build_postgres_current_symbol_embedding_persist_sql,
    build_postgres_symbol_embedding_persist_sql, build_sqlite_symbol_embedding_persist_sql,
    clear_current_symbol_embedding_rows_for_path, clear_current_symbol_embedding_rows_for_paths,
    clear_repo_active_embedding_setup, clear_repo_active_embedding_setup_for_representation,
    clear_repo_symbol_embedding_rows, clear_repo_symbol_embedding_rows_for_representation,
    determine_repo_embedding_sync_action, ensure_semantic_embeddings_schema,
    init_postgres_semantic_embeddings_schema, init_sqlite_semantic_embeddings_schema,
    load_active_embedding_setup, load_current_repo_embedding_states,
    load_current_semantic_summary_map, load_current_symbol_embedding_index_state,
    load_current_symbol_embedding_index_states, load_semantic_summary_map,
    load_symbol_embedding_index_state, load_symbol_embedding_index_states,
    persist_active_embedding_setup, refresh_current_repo_symbol_embeddings_and_clone_edges,
    semantic_embeddings_sqlite_schema_sql, upsert_current_symbol_embedding_rows,
    upsert_symbol_embedding_rows,
};
#[allow(unused_imports)]
pub(crate) use stage_search_documents::{
    SearchDocumentRow, build_current_search_document_persist_sql,
    build_delete_current_search_documents_for_artefact_sql,
    build_delete_current_search_documents_fts_for_artefact_sql,
    build_delete_current_search_documents_fts_sql, build_delete_current_search_documents_sql,
    build_search_document_from_semantic_rows, build_search_document_persist_sql,
    clear_current_search_document_rows_for_artefact, clear_current_search_document_rows_for_path,
    ensure_search_documents_schema, init_postgres_search_documents_schema,
    init_sqlite_search_documents_schema, persist_current_search_document_row,
    persist_search_document_row, search_documents_postgres_schema_sql,
    search_documents_sqlite_schema_sql,
};
#[allow(unused_imports)]
pub(crate) use stage_semantic_features::{
    build_conditional_current_semantic_persist_rows_sql,
    build_conditional_current_symbol_feature_persist_rows_sql,
    build_delete_current_symbol_semantics_for_artefact_sql,
    build_repair_current_semantic_projection_from_historical_sql,
    build_semantic_get_index_state_sql, build_semantic_persist_rows_sql,
    build_symbol_feature_persist_rows_sql, clear_current_semantic_feature_rows_for_path,
    clear_current_semantic_feature_rows_for_paths, ensure_required_llm_summary_output,
    ensure_semantic_features_schema, init_postgres_semantic_features_schema,
    init_sqlite_semantic_features_schema, load_pre_stage_artefacts_for_blob,
    load_pre_stage_dependencies_for_blob, load_semantic_feature_inputs_for_artefacts,
    load_semantic_feature_inputs_for_current_artefacts,
    load_semantic_feature_inputs_for_current_repo, load_semantic_summary_snapshot,
    parse_semantic_index_state_rows, repair_current_semantic_feature_rows_from_historical,
    semantic_features_sqlite_schema_sql, upsert_current_semantic_feature_rows,
    upsert_semantic_feature_rows,
};

pub use pack::SemanticClonesPack;
pub use types::{
    SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
    SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID,
    SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
};
