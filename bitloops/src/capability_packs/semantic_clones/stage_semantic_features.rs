//! Stage 1: semantic feature rows (`symbol_semantics`, `symbol_features`) for the semantic_clones pipeline.

mod hydration;
mod persistence;
mod schema;
mod storage;
mod summary;

#[cfg(test)]
mod tests;

pub(crate) use self::hydration::{
    load_pre_stage_artefacts_for_blob, load_pre_stage_dependencies_for_blob,
    load_semantic_feature_inputs_for_artefacts, load_semantic_feature_inputs_for_current_artefacts,
    load_semantic_feature_inputs_for_current_repo,
};
pub(crate) use self::persistence::{
    clear_current_semantic_feature_rows_for_path, clear_current_semantic_feature_rows_for_paths,
    repair_current_semantic_feature_rows_from_historical, upsert_current_semantic_feature_rows,
    upsert_semantic_feature_rows,
};
pub(crate) use self::schema::{
    ensure_semantic_features_schema, init_postgres_semantic_features_schema,
    init_sqlite_current_projection_semantic_features_schema, init_sqlite_semantic_features_schema,
};
pub(crate) use self::storage::{
    build_conditional_current_semantic_persist_rows_sql,
    build_conditional_current_symbol_feature_persist_rows_sql,
    build_delete_current_symbol_semantics_for_artefact_sql,
    build_repair_current_semantic_projection_from_historical_sql,
    build_semantic_get_index_state_sql, build_semantic_persist_rows_sql,
    build_symbol_feature_persist_rows_sql, parse_semantic_index_state_rows,
    semantic_features_sqlite_schema_sql,
};
pub(crate) use self::summary::{
    ensure_required_llm_summary_output, load_semantic_summary_snapshot,
};
