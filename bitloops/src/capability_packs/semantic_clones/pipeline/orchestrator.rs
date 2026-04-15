use anyhow::{Context, Result};

use crate::capability_packs::semantic_clones::{
    ensure_semantic_embeddings_schema, ensure_semantic_features_schema, scoring,
};
use crate::host::devql::RelationalStorage;

use super::candidates::load_symbol_clone_candidate_inputs;
use super::persistence::replace_repo_symbol_clone_edges_for_projection;
use super::schema::{CloneProjection, ensure_semantic_clones_schema};
use super::state::resolve_active_embedding_states_for_clone_rebuild;

/// Rebuilds the historical clone-edge projection.
pub(crate) async fn rebuild_symbol_clone_edges(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<scoring::SymbolCloneBuildResult> {
    rebuild_symbol_clone_edges_with_options(
        relational,
        repo_id,
        scoring::CloneScoringOptions::default(),
    )
    .await
}

/// Rebuilds the historical clone-edge projection.
pub(crate) async fn rebuild_symbol_clone_edges_with_options(
    relational: &RelationalStorage,
    repo_id: &str,
    options: scoring::CloneScoringOptions,
) -> Result<scoring::SymbolCloneBuildResult> {
    rebuild_symbol_clone_edges_for_projection(
        relational,
        repo_id,
        CloneProjection::Historical,
        options,
    )
    .await
}

#[allow(dead_code)]
pub(crate) async fn rebuild_current_symbol_clone_edges(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<scoring::SymbolCloneBuildResult> {
    rebuild_symbol_clone_edges_for_projection(
        relational,
        repo_id,
        CloneProjection::Current,
        scoring::CloneScoringOptions::default(),
    )
    .await
}

async fn rebuild_symbol_clone_edges_for_projection(
    relational: &RelationalStorage,
    repo_id: &str,
    projection: CloneProjection,
    options: scoring::CloneScoringOptions,
) -> Result<scoring::SymbolCloneBuildResult> {
    ensure_semantic_clones_schema(relational).await?;
    ensure_semantic_features_schema(relational).await?;
    ensure_semantic_embeddings_schema(relational).await?;
    let active_states =
        resolve_active_embedding_states_for_clone_rebuild(relational, repo_id, projection).await?;
    let candidates = match active_states.as_ref() {
        Some(active_states) => {
            load_symbol_clone_candidate_inputs(relational, repo_id, projection, active_states)
                .await?
        }
        None => Vec::new(),
    };
    let build_result = tokio::task::spawn_blocking(move || {
        scoring::build_symbol_clone_edges_with_options(&candidates, options)
    })
    .await
    .context("building semantic clone edges on blocking worker")?;

    replace_repo_symbol_clone_edges_for_projection(
        relational,
        repo_id,
        projection,
        &build_result.edges,
    )
    .await?;
    Ok(build_result)
}

pub(crate) async fn score_symbol_clone_edges_for_source_with_options(
    relational: &RelationalStorage,
    repo_id: &str,
    source_symbol_id: &str,
    options: scoring::CloneScoringOptions,
) -> Result<scoring::SymbolCloneBuildResult> {
    ensure_semantic_clones_schema(relational).await?;
    ensure_semantic_features_schema(relational).await?;
    ensure_semantic_embeddings_schema(relational).await?;
    let projection = CloneProjection::Current;
    let active_states =
        resolve_active_embedding_states_for_clone_rebuild(relational, repo_id, projection).await?;
    let candidates = match active_states.as_ref() {
        Some(active_states) => {
            load_symbol_clone_candidate_inputs(relational, repo_id, projection, active_states)
                .await?
        }
        None => Vec::new(),
    };
    let source_symbol_id = source_symbol_id.to_string();
    tokio::task::spawn_blocking(move || {
        scoring::build_symbol_clone_edges_for_source_with_options(
            &candidates,
            &source_symbol_id,
            options,
        )
    })
    .await
    .context("building source symbol clone edges on blocking worker")
}
