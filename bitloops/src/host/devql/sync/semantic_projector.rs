use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;

use crate::adapters::model_providers::embeddings::EmbeddingProvider;
use crate::capability_packs::semantic_clones::extension_descriptor as semantic_clones_pack;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::capability_packs::semantic_clones::{
    clear_current_semantic_feature_rows_for_path, clear_current_symbol_embedding_rows_for_path,
    upsert_current_semantic_feature_rows, upsert_current_symbol_embedding_rows,
};
use crate::host::devql::sync::content_cache::CachedExtraction;
use crate::host::devql::sync::materializer::{
    MaterializedArtefact, MaterializedEdge, derive_materialized_artefacts,
    derive_materialized_edges,
};
use crate::host::devql::sync::types::DesiredFileState;
use crate::host::devql::{DevqlConfig, RelationalStorage};

pub(crate) async fn project_path(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    desired: &DesiredFileState,
    extraction: &CachedExtraction,
    content: &str,
    summary_provider: Arc<dyn semantic::SemanticSummaryProvider>,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
) -> Result<()> {
    let materialized_artefacts = derive_materialized_artefacts(cfg, desired, extraction)?;
    let artefacts_by_key = materialized_artefacts
        .iter()
        .map(|artefact| (artefact.artifact_key.clone(), artefact.clone()))
        .collect::<HashMap<_, _>>();
    let materialized_edges =
        derive_materialized_edges(cfg, desired, extraction, &artefacts_by_key)?;

    let pre_stage_artefacts = materialized_artefacts
        .iter()
        .map(|artefact| pre_stage_artefact_row(cfg, desired, extraction, artefact))
        .collect::<Vec<_>>();
    let artefacts_by_artefact_id = materialized_artefacts
        .iter()
        .map(|artefact| (artefact.artefact_id.clone(), artefact))
        .collect::<HashMap<_, _>>();
    let pre_stage_dependencies = materialized_edges
        .iter()
        .filter_map(|edge| pre_stage_dependency_row(edge, &artefacts_by_artefact_id))
        .collect::<Vec<_>>();

    let inputs = semantic_clones_pack::build_semantic_feature_inputs(
        &pre_stage_artefacts,
        &pre_stage_dependencies,
        content,
    );
    if inputs.is_empty() {
        remove_path(cfg, relational, &desired.path).await?;
        return Ok(());
    }

    upsert_current_semantic_feature_rows(
        relational,
        &desired.path,
        &desired.effective_content_id,
        &inputs,
        summary_provider,
    )
    .await?;

    if let Some(embedding_provider) = embedding_provider {
        upsert_current_symbol_embedding_rows(
            relational,
            &desired.path,
            &desired.effective_content_id,
            &inputs,
            embedding_provider,
        )
        .await?;
    } else {
        clear_current_symbol_embedding_rows_for_path(relational, &cfg.repo.repo_id, &desired.path)
            .await?;
    }

    Ok(())
}

pub(crate) async fn remove_path(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    path: &str,
) -> Result<()> {
    clear_current_symbol_embedding_rows_for_path(relational, &cfg.repo.repo_id, path).await?;
    clear_current_semantic_feature_rows_for_path(relational, &cfg.repo.repo_id, path).await
}

fn pre_stage_artefact_row(
    cfg: &DevqlConfig,
    desired: &DesiredFileState,
    extraction: &CachedExtraction,
    artefact: &MaterializedArtefact,
) -> semantic::PreStageArtefactRow {
    semantic::PreStageArtefactRow {
        artefact_id: artefact.artefact_id.clone(),
        symbol_id: Some(artefact.symbol_id.clone()),
        repo_id: cfg.repo.repo_id.clone(),
        blob_sha: desired.effective_content_id.clone(),
        path: desired.path.clone(),
        language: extraction.language.clone(),
        canonical_kind: artefact
            .canonical_kind
            .clone()
            .unwrap_or_else(|| artefact.language_kind.clone()),
        language_kind: artefact.language_kind.clone(),
        symbol_fqn: artefact.symbol_fqn.clone(),
        parent_artefact_id: artefact.parent_artefact_id.clone(),
        start_line: Some(artefact.start_line),
        end_line: Some(artefact.end_line),
        start_byte: Some(artefact.start_byte),
        end_byte: Some(artefact.end_byte),
        signature: artefact.signature.clone(),
        modifiers: artefact.modifiers.clone(),
        docstring: artefact.docstring.clone(),
        content_hash: Some(desired.effective_content_id.clone()),
    }
}

fn pre_stage_dependency_row(
    edge: &MaterializedEdge,
    artefacts_by_artefact_id: &HashMap<String, &MaterializedArtefact>,
) -> Option<semantic::PreStageDependencyRow> {
    let target_ref = edge
        .to_artefact_id
        .as_ref()
        .and_then(|artefact_id| artefacts_by_artefact_id.get(artefact_id))
        .map(|artefact| artefact.symbol_fqn.clone())
        .or_else(|| edge.to_symbol_ref.clone())?;
    if target_ref.trim().is_empty() {
        return None;
    }

    Some(semantic::PreStageDependencyRow {
        from_artefact_id: edge.from_artefact_id.clone(),
        edge_kind: edge.edge_kind.clone(),
        target_ref,
    })
}
