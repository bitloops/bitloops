use anyhow::{Context, Result};

use crate::capability_packs::semantic_clones::RepoEmbeddingSyncAction;
use crate::capability_packs::semantic_clones::embeddings::{
    ActiveEmbeddingRepresentationState, EmbeddingRepresentationKind, EmbeddingSetup,
};
use crate::capability_packs::semantic_clones::features as semantic_features;
use crate::capability_packs::semantic_clones::ingesters::{
    EmbeddingRefreshMode, SemanticFeaturesRefreshPayload, SemanticFeaturesRefreshScope,
    SemanticSummaryRefreshMode, SymbolEmbeddingsRefreshPayload, SymbolEmbeddingsRefreshScope,
};
use crate::capability_packs::semantic_clones::runtime_config::{
    EmbeddingProviderMode, embeddings_enabled, resolve_embedding_provider,
    resolve_semantic_clones_config,
};
use crate::host::capability_host::DevqlCapabilityHost;
use crate::host::devql::{DevqlConfig, RelationalStorage, build_capability_host};

use super::super::sqlite_writer::PreparedSyncItem;

pub(super) fn build_current_projection_context(cfg: &DevqlConfig) -> Result<DevqlCapabilityHost> {
    build_capability_host(&cfg.repo_root, cfg.repo.clone())
}

pub(super) async fn project_materialized_items(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    current_projection: &DevqlCapabilityHost,
    items: &[PreparedSyncItem],
) -> Result<()> {
    for item in items {
        let inputs =
            semantic_features::build_semantic_feature_inputs_from_artefacts_with_dependencies(
                &crate::host::devql::sync::semantic_projector::pre_stage_artefacts_for_projection(
                    cfg,
                    &item.desired,
                    &item.extraction,
                )?,
                &crate::host::devql::sync::semantic_projector::pre_stage_dependencies_for_projection(
                    cfg,
                    &item.desired,
                    &item.extraction,
                )?,
                &item.effective_content,
            );
        current_projection
            .invoke_ingester_with_relational(
                crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID,
                crate::capability_packs::semantic_clones::SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID,
                serde_json::to_value(SemanticFeaturesRefreshPayload {
                    scope: SemanticFeaturesRefreshScope::CurrentPath,
                    path: Some(item.desired.path.clone()),
                    content_id: Some(item.desired.effective_content_id.clone()),
                    inputs: inputs.clone(),
                    mode: SemanticSummaryRefreshMode::ConfiguredDegrade,
                })?,
                Some(relational),
            )
            .await
            .with_context(|| {
                format!("refreshing current semantic features for `{}`", item.desired.path)
            })?;
        refresh_current_path_embeddings(
            current_projection,
            relational,
            &item.desired.path,
            &item.desired.effective_content_id,
            &inputs,
            EmbeddingRepresentationKind::Code,
        )
        .await
        .with_context(|| {
            format!(
                "refreshing current code embeddings for `{}`",
                item.desired.path
            )
        })?;
        refresh_current_path_embeddings(
            current_projection,
            relational,
            &item.desired.path,
            &item.desired.effective_content_id,
            &inputs,
            EmbeddingRepresentationKind::Summary,
        )
        .await
        .with_context(|| {
            format!(
                "refreshing current summary embeddings for `{}`",
                item.desired.path
            )
        })?;
    }

    Ok(())
}

pub(super) async fn finalize_semantic_clone_projection_after_sync(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    capability_host: &DevqlCapabilityHost,
    current_projection_changed: bool,
) -> Result<()> {
    let semantic_clones = resolve_semantic_clones_config(
        &capability_host
            .config_view(crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID),
    );
    if !embeddings_enabled(&semantic_clones) {
        clear_semantic_clone_embedding_outputs(relational, &cfg.repo.repo_id).await?;
        return Ok(());
    }

    let semantic_inference = capability_host.inference_for_capability(
        crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID,
    );
    let mut current_inputs: Option<Vec<semantic_features::SemanticFeatureInput>> = None;
    let mut code_setup = None;
    let mut rebuilt_current_projection = false;
    let mut rebuilt_historical_projection = false;

    for representation_kind in [
        EmbeddingRepresentationKind::Code,
        EmbeddingRepresentationKind::Summary,
    ] {
        let selection = resolve_embedding_provider(
            &semantic_clones,
            &semantic_inference,
            representation_kind,
            EmbeddingProviderMode::ConfiguredDegrade,
        )?;
        if let Some(reason) = selection.degraded_reason.as_deref() {
            log::warn!(
                "semantic_clones {} embeddings degraded during sync finalization for repo `{}`: {}",
                representation_kind,
                cfg.repo.repo_id,
                reason
            );
        }
        let Some(provider) = selection.provider.as_ref() else {
            clear_semantic_clone_embedding_outputs_for_representation(
                relational,
                &cfg.repo.repo_id,
                representation_kind,
            )
            .await?;
            if representation_kind == EmbeddingRepresentationKind::Code {
                crate::capability_packs::semantic_clones::pipeline::delete_repo_symbol_clone_edges(
                    relational,
                    &cfg.repo.repo_id,
                )
                .await?;
                crate::capability_packs::semantic_clones::pipeline::delete_repo_current_symbol_clone_edges(
                    relational,
                    &cfg.repo.repo_id,
                )
                .await?;
            }
            continue;
        };

        let setup = crate::capability_packs::semantic_clones::embeddings::resolve_embedding_setup(
            provider.as_ref(),
        )?;
        if representation_kind == EmbeddingRepresentationKind::Code {
            code_setup = Some(setup.clone());
        }
        let sync_action =
            crate::capability_packs::semantic_clones::determine_repo_embedding_sync_action(
                relational,
                &cfg.repo.repo_id,
                representation_kind,
                &setup,
            )
            .await?;
        let should_refresh_repo_embeddings =
            current_projection_changed || sync_action != RepoEmbeddingSyncAction::Incremental;
        if !should_refresh_repo_embeddings {
            continue;
        }

        let inputs = if let Some(inputs) = current_inputs.as_ref() {
            inputs.clone()
        } else {
            let loaded =
                crate::capability_packs::semantic_clones::load_semantic_feature_inputs_for_current_repo(
                    relational,
                    &cfg.repo_root,
                    &cfg.repo.repo_id,
                )
                .await?;
            current_inputs = Some(loaded.clone());
            loaded
        };
        if inputs.is_empty() {
            continue;
        }

        let outcome = run_repo_symbol_embeddings_refresh(
            capability_host,
            relational,
            representation_kind,
            inputs,
        )
        .await
        .with_context(|| {
            format!(
                "refreshing repo-wide {} embeddings after sync",
                representation_kind
            )
        })?;
        rebuilt_current_projection |= outcome.symbol_clone_edges_upserted > 0
            || sync_action == RepoEmbeddingSyncAction::RefreshCurrentRepo;
        rebuilt_historical_projection |= outcome.clone_rebuild_recommended
            || outcome.symbol_clone_edges_upserted > 0
            || sync_action != RepoEmbeddingSyncAction::Incremental;
    }

    if let Some(setup) = code_setup.as_ref()
        && rebuilt_historical_projection
    {
        if let Some(active_state) =
            select_active_code_embedding_state_for_repo(relational, &cfg.repo.repo_id, setup)
                .await?
        {
            crate::capability_packs::semantic_clones::persist_active_embedding_setup(
                relational,
                &cfg.repo.repo_id,
                &active_state,
            )
            .await?;
            rebuild_active_clone_edges(capability_host, relational).await?;
            rebuilt_current_projection = true;
        } else {
            crate::capability_packs::semantic_clones::pipeline::delete_repo_symbol_clone_edges(
                relational,
                &cfg.repo.repo_id,
            )
            .await?;
        }
    }

    if current_projection_changed || rebuilt_current_projection {
        crate::capability_packs::semantic_clones::pipeline::rebuild_current_symbol_clone_edges(
            relational,
            &cfg.repo.repo_id,
        )
        .await
        .context("rebuilding current semantic clone edges after sync projection changes")?;
    }

    Ok(())
}

async fn refresh_current_path_embeddings(
    current_projection: &DevqlCapabilityHost,
    relational: &RelationalStorage,
    path: &str,
    content_id: &str,
    inputs: &[semantic_features::SemanticFeatureInput],
    representation_kind: EmbeddingRepresentationKind,
) -> Result<()> {
    current_projection
        .invoke_ingester_with_relational(
            crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID,
            crate::capability_packs::semantic_clones::SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
            serde_json::to_value(SymbolEmbeddingsRefreshPayload {
                scope: SymbolEmbeddingsRefreshScope::CurrentPath,
                path: Some(path.to_string()),
                content_id: Some(content_id.to_string()),
                inputs: inputs.to_vec(),
                expected_input_hashes: Default::default(),
                representation_kind,
                mode: EmbeddingRefreshMode::ConfiguredDegrade,
                manage_active_state: false,
            })?,
            Some(relational),
        )
        .await?;
    Ok(())
}

async fn clear_semantic_clone_embedding_outputs(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<()> {
    crate::capability_packs::semantic_clones::clear_repo_symbol_embedding_rows(relational, repo_id)
        .await?;
    crate::capability_packs::semantic_clones::clear_repo_active_embedding_setup(
        relational, repo_id,
    )
    .await?;
    crate::capability_packs::semantic_clones::pipeline::delete_repo_symbol_clone_edges(
        relational, repo_id,
    )
    .await?;
    crate::capability_packs::semantic_clones::pipeline::delete_repo_current_symbol_clone_edges(
        relational, repo_id,
    )
    .await
}

async fn clear_semantic_clone_embedding_outputs_for_representation(
    relational: &RelationalStorage,
    repo_id: &str,
    representation_kind: EmbeddingRepresentationKind,
) -> Result<()> {
    crate::capability_packs::semantic_clones::clear_repo_symbol_embedding_rows_for_representation(
        relational,
        repo_id,
        representation_kind,
    )
    .await?;
    crate::capability_packs::semantic_clones::clear_repo_active_embedding_setup_for_representation(
        relational,
        repo_id,
        representation_kind,
    )
    .await
}

#[derive(Debug, Clone, Default)]
struct SyncSymbolEmbeddingsRefreshOutcome {
    clone_rebuild_recommended: bool,
    symbol_clone_edges_upserted: usize,
}

async fn run_repo_symbol_embeddings_refresh(
    capability_host: &DevqlCapabilityHost,
    relational: &RelationalStorage,
    representation_kind: EmbeddingRepresentationKind,
    inputs: Vec<semantic_features::SemanticFeatureInput>,
) -> Result<SyncSymbolEmbeddingsRefreshOutcome> {
    let result = capability_host
        .invoke_ingester_with_relational(
            crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID,
            crate::capability_packs::semantic_clones::SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
            serde_json::to_value(SymbolEmbeddingsRefreshPayload {
                scope: SymbolEmbeddingsRefreshScope::Historical,
                path: None,
                content_id: None,
                inputs,
                expected_input_hashes: Default::default(),
                representation_kind,
                mode: EmbeddingRefreshMode::ConfiguredDegrade,
                manage_active_state: true,
            })?,
            Some(relational),
        )
        .await?;
    Ok(SyncSymbolEmbeddingsRefreshOutcome {
        clone_rebuild_recommended: result.payload["clone_rebuild_recommended"]
            .as_bool()
            .unwrap_or(false),
        symbol_clone_edges_upserted: result.payload["symbol_clone_edges_upserted"]
            .as_u64()
            .unwrap_or_default() as usize,
    })
}

async fn select_active_code_embedding_state_for_repo(
    relational: &RelationalStorage,
    repo_id: &str,
    setup: &EmbeddingSetup,
) -> Result<Option<ActiveEmbeddingRepresentationState>> {
    let states = crate::capability_packs::semantic_clones::load_current_repo_embedding_states(
        relational,
        repo_id,
        Some(EmbeddingRepresentationKind::Code),
    )
    .await?;
    Ok(states.into_iter().find(|state| state.setup == *setup))
}

async fn rebuild_active_clone_edges(
    capability_host: &DevqlCapabilityHost,
    relational: &RelationalStorage,
) -> Result<()> {
    capability_host
        .invoke_ingester_with_relational(
            crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID,
            crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
            serde_json::json!({}),
            Some(relational),
        )
        .await
        .with_context(|| {
            format!(
                "running capability ingester `{}` for `{}`",
                crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
                crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID
            )
        })?;
    Ok(())
}
