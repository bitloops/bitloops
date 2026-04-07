pub mod descriptor;
pub mod embeddings;
pub mod extension_descriptor;
pub mod features;
pub mod health;
pub mod ingesters;
pub mod migrations;
pub mod pack;
pub mod pipeline;
pub mod query_examples;
pub mod register;
pub mod schema;
pub mod schema_module;
pub mod scoring;
pub mod stages;
pub mod types;

mod stage_embeddings;
mod stage_semantic_features;

pub(crate) use stage_embeddings::{
    clear_repo_symbol_embedding_rows, ensure_semantic_embeddings_schema,
    init_postgres_semantic_embeddings_schema, init_sqlite_semantic_embeddings_schema,
    upsert_symbol_embedding_rows,
};
pub(crate) use stage_semantic_features::{
    init_postgres_semantic_features_schema, init_sqlite_semantic_features_schema,
    load_pre_stage_artefacts_for_blob, load_pre_stage_dependencies_for_blob,
    load_semantic_feature_inputs_for_artefacts, load_semantic_summary_snapshot,
    persist_semantic_summary_row, upsert_semantic_feature_rows,
};

pub use pack::SemanticClonesPack;
pub use types::{SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID};
