mod persistence_sql;
mod queries;
mod schema_sql;

#[cfg(test)]
mod tests;

pub(crate) use self::persistence_sql::{
    build_conditional_current_semantic_persist_existing_rows_sql,
    build_conditional_current_semantic_persist_rows_sql,
    build_conditional_current_symbol_feature_persist_rows_sql,
    build_current_semantic_persist_rows_sql, build_current_symbol_feature_persist_rows_sql,
    build_delete_current_symbol_features_for_paths_sql, build_delete_current_symbol_features_sql,
    build_delete_current_symbol_semantics_for_artefact_sql,
    build_delete_current_symbol_semantics_for_paths_sql, build_delete_current_symbol_semantics_sql,
    build_repair_current_semantic_projection_from_historical_sql,
    build_semantic_get_index_state_sql, build_semantic_persist_rows_sql,
    build_symbol_feature_persist_rows_sql, parse_semantic_index_state_rows,
};
pub(crate) use self::queries::{
    build_current_repo_artefacts_by_ids_sql, build_current_repo_artefacts_sql,
    build_semantic_get_artefacts_by_ids_sql, build_semantic_get_artefacts_sql,
    build_semantic_get_dependencies_sql, build_semantic_get_summary_sql,
    parse_semantic_artefact_rows, parse_semantic_dependency_rows,
};
pub(crate) use self::schema_sql::{
    semantic_features_postgres_schema_sql, semantic_features_postgres_upgrade_sql,
    semantic_features_sqlite_schema_sql, upgrade_sqlite_semantic_features_schema,
};
