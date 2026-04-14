use std::sync::Arc;

use anyhow::{Context, Result};

use super::schema::ensure_semantic_features_schema;
use super::storage::{
    build_conditional_current_semantic_persist_rows_sql, build_current_semantic_persist_rows_sql,
    build_delete_current_symbol_features_sql, build_delete_current_symbol_semantics_sql,
    build_semantic_get_index_state_sql, build_semantic_persist_rows_sql,
    parse_semantic_index_state_rows,
};
use super::summary::ensure_required_llm_summary_output;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::devql::RelationalStorage;

pub(crate) async fn upsert_semantic_feature_rows(
    relational: &RelationalStorage,
    inputs: &[semantic::SemanticFeatureInput],
    summary_provider: Arc<dyn semantic::SemanticSummaryProvider>,
) -> Result<semantic::SemanticFeatureIngestionStats> {
    let mut stats = semantic::SemanticFeatureIngestionStats::default();

    for input in inputs {
        let next_input_hash =
            semantic::build_semantic_feature_input_hash(input, summary_provider.as_ref());
        let state = load_semantic_index_state(relational, &input.artefact_id).await?;
        if !semantic::semantic_features_require_reindex(&state, &next_input_hash) {
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
        persist_semantic_feature_rows(relational, &rows).await?;
        persist_current_semantic_feature_rows_for_matching_input(relational, &input, &rows).await?;
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
    let Some(first) = inputs.first() else {
        return Ok(semantic::SemanticFeatureIngestionStats::default());
    };

    clear_current_semantic_feature_rows_for_path(relational, &first.repo_id, path).await?;

    let mut stats = semantic::SemanticFeatureIngestionStats::default();
    for input in inputs {
        let symbol_id = input.symbol_id.clone();
        let input = input.clone();
        let summary_provider_for_row = Arc::clone(&summary_provider);
        let rows = tokio::task::spawn_blocking(move || {
            semantic::build_semantic_feature_rows(&input, summary_provider_for_row.as_ref())
        })
        .await
        .context("building current semantic feature rows on blocking worker")?;
        ensure_required_llm_summary_output(&rows, summary_provider.as_ref())?;
        persist_current_semantic_feature_rows(
            relational,
            symbol_id.as_deref(),
            path,
            content_id,
            &rows,
        )
        .await?;
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
        .exec_batch_transactional(&[
            build_delete_current_symbol_features_sql(repo_id, path),
            build_delete_current_symbol_semantics_sql(repo_id, path),
        ])
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
        .exec(&build_semantic_persist_rows_sql(
            rows,
            relational.dialect(),
        )?)
        .await
}

pub(super) async fn persist_current_semantic_feature_rows_for_matching_input(
    relational: &RelationalStorage,
    input: &semantic::SemanticFeatureInput,
    rows: &semantic::SemanticFeatureRows,
) -> Result<()> {
    match relational
        .exec(&build_conditional_current_semantic_persist_rows_sql(
            rows,
            input,
            relational.dialect(),
        )?)
        .await
    {
        Ok(()) => Ok(()),
        Err(err) => {
            let message = err.to_string();
            if message.contains("no such table: artefacts_current")
                || message.contains("no such table: current_file_state")
            {
                return Ok(());
            }
            Err(err).context("persisting current semantic feature rows for matching input")
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
        .exec(&build_current_semantic_persist_rows_sql(
            rows,
            symbol_id,
            path,
            content_id,
            relational.dialect(),
        )?)
        .await
}
