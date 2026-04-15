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
    load_semantic_feature_inputs_for_artefacts, load_semantic_feature_inputs_for_current_repo,
};
pub(crate) use self::persistence::{
    clear_current_semantic_feature_rows_for_path, upsert_current_semantic_feature_rows,
    upsert_semantic_feature_rows,
};
pub(crate) use self::schema::{
    ensure_semantic_features_schema, init_postgres_semantic_features_schema,
    init_sqlite_semantic_features_schema,
};
pub(crate) use self::summary::load_semantic_summary_snapshot;
