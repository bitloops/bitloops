use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::capability_packs::semantic_clones::embeddings;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::devql::RelationalStorage;
use crate::host::inference::EmbeddingService;

use super::CurrentRepoEmbeddingRefreshResult;
use super::ensure_semantic_embeddings_schema;
use super::persistence::{
    delete_stale_current_symbol_embedding_rows_for_path, persist_active_embedding_setup,
    persist_current_symbol_embedding_row, persist_symbol_embedding_row,
    upsert_current_repo_symbol_embedding_rows,
};
use super::storage::{
    load_current_semantic_summary_map, load_current_symbol_embedding_index_state,
    load_semantic_summary_map, load_symbol_embedding_index_state,
};

pub(crate) async fn upsert_symbol_embedding_rows(
    relational: &RelationalStorage,
    inputs: &[semantic::SemanticFeatureInput],
    representation_kind: embeddings::EmbeddingRepresentationKind,
    embedding_provider: Arc<dyn EmbeddingService>,
) -> Result<embeddings::SymbolEmbeddingIngestionStats> {
    let mut stats = embeddings::SymbolEmbeddingIngestionStats::default();
    if inputs.is_empty() {
        return Ok(stats);
    }

    ensure_semantic_embeddings_schema(relational).await?;
    let setup = embeddings::resolve_embedding_setup(embedding_provider.as_ref())?;

    let artefact_ids = inputs
        .iter()
        .map(|input| input.artefact_id.clone())
        .collect::<Vec<_>>();
    let summary_by_artefact_id =
        load_semantic_summary_map(relational, &artefact_ids, representation_kind).await?;
    let embedding_inputs = embeddings::build_symbol_embedding_inputs(
        inputs,
        representation_kind,
        &summary_by_artefact_id,
    );
    stats.eligible = embedding_inputs.len();

    let mut reindex_inputs = Vec::new();
    for input in embedding_inputs {
        let next_input_hash =
            embeddings::build_symbol_embedding_input_hash(&input, embedding_provider.as_ref());
        let state = load_symbol_embedding_index_state(
            relational,
            &input.artefact_id,
            input.representation_kind,
            &setup.setup_fingerprint,
        )
        .await?;
        if !embeddings::symbol_embeddings_require_reindex(&state, &next_input_hash) {
            stats.skipped += 1;
            continue;
        }
        reindex_inputs.push(input);
    }

    if !reindex_inputs.is_empty() {
        let embedding_provider_for_rows = Arc::clone(&embedding_provider);
        let rows = tokio::task::spawn_blocking(move || {
            embeddings::build_symbol_embedding_rows(
                &reindex_inputs,
                embedding_provider_for_rows.as_ref(),
            )
        })
        .await
        .context("building semantic embedding rows on blocking worker")??;
        for row in rows {
            persist_symbol_embedding_row(relational, &row).await?;
            stats.upserted += 1;
        }
    }

    Ok(stats)
}

#[allow(dead_code)]
pub(crate) async fn upsert_current_symbol_embedding_rows(
    relational: &RelationalStorage,
    path: &str,
    content_id: &str,
    inputs: &[semantic::SemanticFeatureInput],
    representation_kind: embeddings::EmbeddingRepresentationKind,
    embedding_provider: Arc<dyn EmbeddingService>,
) -> Result<embeddings::SymbolEmbeddingIngestionStats> {
    let mut stats = embeddings::SymbolEmbeddingIngestionStats::default();
    let Some(first) = inputs.first() else {
        return Ok(stats);
    };

    ensure_semantic_embeddings_schema(relational).await?;
    let setup = embeddings::resolve_embedding_setup(embedding_provider.as_ref())?;

    let artefact_ids = inputs
        .iter()
        .map(|input| input.artefact_id.clone())
        .collect::<Vec<_>>();
    let summary_by_artefact_id =
        load_current_semantic_summary_map(relational, &artefact_ids, representation_kind).await?;
    let input_by_artefact_id = inputs
        .iter()
        .map(|input| (input.artefact_id.clone(), input))
        .collect::<HashMap<_, _>>();
    let embedding_inputs = embeddings::build_symbol_embedding_inputs(
        inputs,
        representation_kind,
        &summary_by_artefact_id,
    );
    stats.eligible = embedding_inputs.len();
    delete_stale_current_symbol_embedding_rows_for_path(
        relational,
        &first.repo_id,
        path,
        content_id,
        representation_kind,
        &embedding_inputs
            .iter()
            .map(|input| input.artefact_id.clone())
            .collect::<Vec<_>>(),
    )
    .await?;

    let mut reindex_inputs = Vec::new();
    for input in embedding_inputs {
        let next_input_hash =
            embeddings::build_symbol_embedding_input_hash(&input, embedding_provider.as_ref());
        let state = load_current_symbol_embedding_index_state(
            relational,
            &input.artefact_id,
            input.representation_kind,
            &setup.setup_fingerprint,
        )
        .await?;
        if !embeddings::symbol_embeddings_require_reindex(&state, &next_input_hash) {
            stats.skipped += 1;
            continue;
        }
        reindex_inputs.push(input);
    }

    if !reindex_inputs.is_empty() {
        let embedding_provider_for_rows = Arc::clone(&embedding_provider);
        let rows = tokio::task::spawn_blocking(move || {
            embeddings::build_symbol_embedding_rows(
                &reindex_inputs,
                embedding_provider_for_rows.as_ref(),
            )
        })
        .await
        .context("building current semantic embedding rows on blocking worker")??;
        for row in rows {
            let input_metadata = input_by_artefact_id
                .get(&row.artefact_id)
                .copied()
                .ok_or_else(|| {
                    anyhow::anyhow!("missing current semantic input for `{}`", row.artefact_id)
                })?;
            persist_current_symbol_embedding_row(
                relational,
                input_metadata,
                path,
                content_id,
                &row,
            )
            .await?;
            stats.upserted += 1;
        }
    }

    Ok(stats)
}

pub(crate) async fn refresh_current_repo_symbol_embeddings_and_clone_edges(
    relational: &RelationalStorage,
    repo_root: &Path,
    repo_id: &str,
    summary_provider: Arc<dyn semantic::SemanticSummaryProvider>,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    embedding_provider: Arc<dyn EmbeddingService>,
    perform_clone_rebuild_inline: bool,
) -> Result<CurrentRepoEmbeddingRefreshResult> {
    ensure_semantic_embeddings_schema(relational).await?;
    let setup = embeddings::resolve_embedding_setup(embedding_provider.as_ref())?;
    let current_inputs =
        crate::capability_packs::semantic_clones::load_semantic_feature_inputs_for_current_repo(
            relational, repo_root, repo_id,
        )
        .await?;
    let semantic_feature_stats =
        crate::capability_packs::semantic_clones::upsert_semantic_feature_rows(
            relational,
            &current_inputs,
            summary_provider,
        )
        .await?;
    upsert_symbol_embedding_rows(
        relational,
        &current_inputs,
        representation_kind,
        Arc::clone(&embedding_provider),
    )
    .await?;
    let embedding_stats = upsert_current_repo_symbol_embedding_rows(
        relational,
        &current_inputs,
        representation_kind,
        embedding_provider,
    )
    .await?;
    if embedding_stats.eligible == 0 {
        return Ok(CurrentRepoEmbeddingRefreshResult {
            semantic_feature_stats,
            embedding_stats,
            clone_build: Default::default(),
        });
    }

    persist_active_embedding_setup(
        relational,
        repo_id,
        &embeddings::ActiveEmbeddingRepresentationState::new(representation_kind, setup),
    )
    .await?;
    let clone_build = if perform_clone_rebuild_inline
        && representation_updates_clone_scoring(representation_kind)
    {
        crate::capability_packs::semantic_clones::pipeline::rebuild_symbol_clone_edges(
            relational, repo_id,
        )
        .await?;
        crate::capability_packs::semantic_clones::pipeline::rebuild_current_symbol_clone_edges(
            relational, repo_id,
        )
        .await?
    } else {
        Default::default()
    };

    Ok(CurrentRepoEmbeddingRefreshResult {
        semantic_feature_stats,
        embedding_stats,
        clone_build,
    })
}

fn representation_updates_clone_scoring(
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> bool {
    matches!(
        representation_kind,
        embeddings::EmbeddingRepresentationKind::Code
            | embeddings::EmbeddingRepresentationKind::Summary
    )
}
