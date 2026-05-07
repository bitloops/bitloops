use std::sync::Arc;

use anyhow::{Context, Result};

use super::schema::ensure_semantic_features_schema;
use super::storage::{
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
use super::summary::ensure_required_llm_summary_output;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::capability_packs::semantic_clones::{
    clear_current_search_document_rows_for_artefact, clear_current_search_document_rows_for_path,
    ensure_search_documents_schema, persist_current_search_document_row,
    persist_search_document_row,
};
use crate::host::devql::RelationalStorage;

pub(crate) async fn upsert_semantic_feature_rows(
    relational: &RelationalStorage,
    inputs: &[semantic::SemanticFeatureInput],
    summary_provider: Arc<dyn semantic::SemanticSummaryProvider>,
) -> Result<semantic::SemanticFeatureIngestionStats> {
    let mut stats = semantic::SemanticFeatureIngestionStats::default();
    ensure_semantic_features_schema(relational).await?;
    ensure_search_documents_schema(relational).await?;

    for input in inputs {
        let persist_summaries = summary_provider.persists_summaries_for(input);
        let next_input_hash =
            semantic::build_semantic_feature_input_hash(input, summary_provider.as_ref());
        let state = load_semantic_index_state(relational, &input.artefact_id).await?;
        if !semantic::semantic_features_require_reindex(
            &state,
            &next_input_hash,
            summary_provider.requires_model_output(),
            persist_summaries,
        ) {
            repair_current_semantic_feature_rows_from_historical(
                relational,
                &input.repo_id,
                std::slice::from_ref(&input.artefact_id),
            )
            .await?;
            if persist_summaries {
                persist_existing_semantic_feature_rows_to_current_for_matching_input(
                    relational,
                    input,
                    &next_input_hash,
                )
                .await?;
            } else {
                clear_current_summary_rows_for_artefact(relational, input).await?;
            }
            stats.skipped += 1;
            continue;
        }

        let input = input.clone();
        let summary_provider_for_row = Arc::clone(&summary_provider);
        let input_for_row = input.clone();
        let rows = tokio::task::spawn_blocking(move || {
            semantic::build_semantic_feature_rows(&input_for_row, summary_provider_for_row.as_ref())
        })
        .await
        .context("building semantic feature rows on blocking worker")?;
        ensure_required_llm_summary_output(&rows, summary_provider.as_ref())?;
        if persist_summaries {
            persist_semantic_feature_rows(relational, &rows).await?;
            persist_current_semantic_feature_rows_for_matching_input(relational, &input, &rows)
                .await?;
            persist_search_document_row(relational, &input, &rows).await?;
        } else {
            persist_symbol_feature_rows(relational, &rows).await?;
            persist_current_symbol_feature_rows_for_matching_input(relational, &input, &rows)
                .await?;
            clear_current_summary_rows_for_artefact(relational, &input).await?;
        }
        stats.upserted += 1;
    }

    Ok(stats)
}

#[allow(dead_code)]
pub(crate) async fn upsert_current_semantic_feature_rows(
    relational: &RelationalStorage,
    path: &str,
    content_id: &str,
    inputs: &[semantic::SemanticFeatureInput],
    summary_provider: Arc<dyn semantic::SemanticSummaryProvider>,
) -> Result<semantic::SemanticFeatureIngestionStats> {
    ensure_semantic_features_schema(relational).await?;
    ensure_search_documents_schema(relational).await?;
    let Some(first) = inputs.first() else {
        return Ok(semantic::SemanticFeatureIngestionStats::default());
    };

    clear_current_semantic_feature_rows_for_path(relational, &first.repo_id, path).await?;
    clear_current_search_document_rows_for_path(relational, &first.repo_id, path).await?;

    let mut stats = semantic::SemanticFeatureIngestionStats::default();
    for input in inputs {
        let persist_summaries = summary_provider.persists_summaries_for(input);
        let symbol_id = input.symbol_id.clone();
        let input = input.clone();
        let summary_provider_for_row = Arc::clone(&summary_provider);
        let input_for_row = input.clone();
        let rows = tokio::task::spawn_blocking(move || {
            semantic::build_semantic_feature_rows(&input_for_row, summary_provider_for_row.as_ref())
        })
        .await
        .context("building current semantic feature rows on blocking worker")?;
        ensure_required_llm_summary_output(&rows, summary_provider.as_ref())?;
        if persist_summaries {
            persist_current_semantic_feature_rows(
                relational,
                symbol_id.as_deref(),
                path,
                content_id,
                &rows,
            )
            .await?;
            persist_current_search_document_row(relational, &input, &rows).await?;
        } else {
            persist_current_symbol_feature_rows(
                relational,
                symbol_id.as_deref(),
                path,
                content_id,
                &rows,
            )
            .await?;
            clear_current_summary_rows_for_artefact(relational, &input).await?;
        }
        stats.upserted += 1;
    }

    Ok(stats)
}

#[allow(dead_code)]
pub(crate) async fn clear_current_semantic_feature_rows_for_path(
    relational: &RelationalStorage,
    repo_id: &str,
    path: &str,
) -> Result<()> {
    ensure_semantic_features_schema(relational).await?;
    relational
        .exec_serialized_batch_transactional(&[
            build_delete_current_symbol_features_sql(repo_id, path),
            build_delete_current_symbol_semantics_sql(repo_id, path),
        ])
        .await
}

pub(crate) async fn clear_current_semantic_feature_rows_for_paths(
    relational: &RelationalStorage,
    repo_id: &str,
    paths: &[String],
) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    ensure_semantic_features_schema(relational).await?;
    let mut statements = Vec::new();
    if let Some(sql) = build_delete_current_symbol_features_for_paths_sql(repo_id, paths) {
        statements.push(sql);
    }
    if let Some(sql) = build_delete_current_symbol_semantics_for_paths_sql(repo_id, paths) {
        statements.push(sql);
    }
    relational
        .exec_serialized_batch_transactional(&statements)
        .await
}

async fn load_semantic_index_state(
    relational: &RelationalStorage,
    artefact_id: &str,
) -> Result<semantic::SemanticFeatureIndexState> {
    let rows = relational
        .query_rows(&build_semantic_get_index_state_sql(artefact_id))
        .await?;
    Ok(parse_semantic_index_state_rows(&rows))
}

async fn persist_semantic_feature_rows(
    relational: &RelationalStorage,
    rows: &semantic::SemanticFeatureRows,
) -> Result<()> {
    relational
        .exec_serialized(&build_semantic_persist_rows_sql(
            rows,
            relational.dialect(),
        )?)
        .await
}

async fn persist_symbol_feature_rows(
    relational: &RelationalStorage,
    rows: &semantic::SemanticFeatureRows,
) -> Result<()> {
    relational
        .exec_serialized(&build_symbol_feature_persist_rows_sql(
            rows,
            relational.dialect(),
        )?)
        .await
}

async fn clear_current_summary_rows_for_artefact(
    relational: &RelationalStorage,
    input: &semantic::SemanticFeatureInput,
) -> Result<()> {
    relational
        .exec_serialized(&build_delete_current_symbol_semantics_for_artefact_sql(
            &input.repo_id,
            &input.artefact_id,
        ))
        .await?;
    clear_current_search_document_rows_for_artefact(relational, &input.repo_id, &input.artefact_id)
        .await
}

pub(crate) async fn repair_current_semantic_feature_rows_from_historical(
    relational: &RelationalStorage,
    repo_id: &str,
    artefact_ids: &[String],
) -> Result<()> {
    ensure_semantic_features_schema(relational).await?;
    match relational
        .exec_serialized(
            &build_repair_current_semantic_projection_from_historical_sql(
                repo_id,
                artefact_ids,
                relational.dialect(),
            ),
        )
        .await
    {
        Ok(()) => Ok(()),
        Err(err) => {
            let message = format!("{err:#}");
            if missing_current_projection_schema_error(&message) {
                return Ok(());
            }
            Err(err).context("repairing current semantic projection from historical rows")
        }
    }
}

pub(super) async fn persist_existing_semantic_feature_rows_to_current_for_matching_input(
    relational: &RelationalStorage,
    input: &semantic::SemanticFeatureInput,
    semantic_features_input_hash: &str,
) -> Result<()> {
    match relational
        .exec_serialized(
            &build_conditional_current_semantic_persist_existing_rows_sql(
                input,
                semantic_features_input_hash,
                relational.dialect(),
            )?,
        )
        .await
    {
        Ok(()) => Ok(()),
        Err(err) => {
            let message = format!("{err:#}");
            if missing_current_projection_schema_error(&message) {
                return Ok(());
            }
            Err(err).context("repairing remapped current semantic projection from historical rows")
        }
    }
}

pub(super) async fn persist_current_semantic_feature_rows_for_matching_input(
    relational: &RelationalStorage,
    input: &semantic::SemanticFeatureInput,
    rows: &semantic::SemanticFeatureRows,
) -> Result<()> {
    match relational
        .exec_serialized(&build_conditional_current_semantic_persist_rows_sql(
            rows,
            input,
            relational.dialect(),
        )?)
        .await
    {
        Ok(()) => Ok(()),
        Err(err) => {
            let message = format!("{err:#}");
            if missing_current_projection_schema_error(&message) {
                return Ok(());
            }
            Err(err).context("persisting current semantic feature rows for matching input")
        }
    }
}

pub(super) async fn persist_current_symbol_feature_rows_for_matching_input(
    relational: &RelationalStorage,
    input: &semantic::SemanticFeatureInput,
    rows: &impl semantic::HashedFeatureRows,
) -> Result<()> {
    match relational
        .exec_serialized(&build_conditional_current_symbol_feature_persist_rows_sql(
            rows,
            input,
            relational.dialect(),
        )?)
        .await
    {
        Ok(()) => Ok(()),
        Err(err) => {
            let message = format!("{err:#}");
            if missing_current_projection_schema_error(&message) {
                return Ok(());
            }
            Err(err).context("persisting current symbol feature rows for matching input")
        }
    }
}

#[allow(dead_code)]
async fn persist_current_semantic_feature_rows(
    relational: &RelationalStorage,
    symbol_id: Option<&str>,
    path: &str,
    content_id: &str,
    rows: &semantic::SemanticFeatureRows,
) -> Result<()> {
    relational
        .exec_serialized(&build_current_semantic_persist_rows_sql(
            rows,
            symbol_id,
            path,
            content_id,
            relational.dialect(),
        )?)
        .await
}

async fn persist_current_symbol_feature_rows(
    relational: &RelationalStorage,
    symbol_id: Option<&str>,
    path: &str,
    content_id: &str,
    rows: &impl semantic::HashedFeatureRows,
) -> Result<()> {
    relational
        .exec_serialized(&build_current_symbol_feature_persist_rows_sql(
            rows,
            symbol_id,
            path,
            content_id,
            relational.dialect(),
        )?)
        .await
}

fn missing_current_projection_schema_error(message: &str) -> bool {
    missing_relation_error(message, "artefacts_current")
        || missing_relation_error(message, "current_file_state")
        || missing_column_error(message, "cfs.effective_content_id")
        || missing_column_error(message, "state.effective_content_id")
        || missing_column_error(message, "effective_content_id")
}

fn missing_relation_error(message: &str, relation: &str) -> bool {
    message.contains(&format!("no such table: {relation}"))
        || message.contains(&format!("relation \"{relation}\" does not exist"))
        || message.contains(&format!("relation '{relation}' does not exist"))
        || message.contains(&format!("relation {relation} does not exist"))
}

fn missing_column_error(message: &str, column: &str) -> bool {
    message.contains(&format!("no such column: {column}"))
        || message.contains(&format!("column \"{column}\" does not exist"))
        || message.contains(&format!("column '{column}' does not exist"))
        || message.contains(&format!("column {column} does not exist"))
}

#[cfg(test)]
mod tests {
    use super::missing_current_projection_schema_error;

    #[test]
    fn missing_current_projection_schema_error_recognizes_postgres_missing_relation() {
        assert!(missing_current_projection_schema_error(
            "error returned from database: relation \"current_file_state\" does not exist",
        ));
    }

    #[test]
    fn missing_current_projection_schema_error_recognizes_postgres_missing_aliased_column() {
        assert!(missing_current_projection_schema_error(
            "error returned from database: column state.effective_content_id does not exist",
        ));
    }

    #[test]
    fn missing_current_projection_schema_error_ignores_unrelated_errors() {
        assert!(!missing_current_projection_schema_error(
            "error returned from database: syntax error near FROM",
        ));
    }
}
