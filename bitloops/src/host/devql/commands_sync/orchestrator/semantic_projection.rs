use crate::capability_packs::semantic_clones::features as semantic_features;
use crate::capability_packs::semantic_clones::ingesters::{
    SemanticFeaturesRefreshPayload, SemanticFeaturesRefreshScope, SemanticSummaryRefreshMode,
};
use crate::capability_packs::semantic_clones::runtime_config::{
    embeddings_enabled, resolve_semantic_clones_config,
};
use crate::host::capability_host::DevqlCapabilityHost;
use crate::host::devql::{DevqlConfig, RelationalStorage, build_capability_host};
use anyhow::{Context, Result};

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
        if !item.semantic_projection_allowed {
            crate::host::devql::sync::semantic_projector::remove_path(
                cfg,
                relational,
                &item.desired.path,
            )
            .await
            .with_context(|| {
                format!(
                    "clearing current semantic clone projection for `{}`",
                    item.desired.path
                )
            })?;
            continue;
        }

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
                serde_json::to_value(build_current_path_refresh_payload(
                    item.desired.path.clone(),
                    item.desired.effective_content_id.clone(),
                    inputs,
                ))?,
                Some(relational),
            )
            .await
            .with_context(|| {
                format!("refreshing current semantic features for `{}`", item.desired.path)
            })?;
    }

    Ok(())
}

fn build_current_path_refresh_payload(
    path: String,
    content_id: String,
    inputs: Vec<semantic_features::SemanticFeatureInput>,
) -> SemanticFeaturesRefreshPayload {
    SemanticFeaturesRefreshPayload {
        scope: SemanticFeaturesRefreshScope::CurrentPath,
        path: Some(path),
        content_id: Some(content_id),
        inputs,
        // Keep sync on the fast path: current-state reconciliation already queues
        // async summary refresh jobs for configured providers.
        mode: SemanticSummaryRefreshMode::DeterministicOnly,
    }
}

pub(super) async fn finalize_semantic_clone_projection_after_sync(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    capability_host: &DevqlCapabilityHost,
    current_projection_changed: bool,
) -> Result<()> {
    let _ = current_projection_changed;
    let semantic_clones = resolve_semantic_clones_config(
        &capability_host
            .config_view(crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID),
    );
    if !embeddings_enabled(&semantic_clones) {
        clear_semantic_clone_embedding_outputs(relational, &cfg.repo.repo_id).await?;
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_path_sync_projection_requests_deterministic_summaries() {
        let payload = build_current_path_refresh_payload(
            "src/lib.rs".to_string(),
            "blob-123".to_string(),
            Vec::new(),
        );

        assert_eq!(payload.scope, SemanticFeaturesRefreshScope::CurrentPath);
        assert_eq!(payload.path.as_deref(), Some("src/lib.rs"));
        assert_eq!(payload.content_id.as_deref(), Some("blob-123"));
        assert_eq!(payload.mode, SemanticSummaryRefreshMode::DeterministicOnly);
        assert!(payload.inputs.is_empty());
    }
}
