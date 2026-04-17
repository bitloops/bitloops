use anyhow::{Context, Result};
use serde_json::Value;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::features as semantic_features;
use crate::capability_packs::semantic_clones::ingesters::{
    EmbeddingRefreshMode, SemanticFeaturesRefreshPayload, SemanticFeaturesRefreshScope,
    SemanticSummaryRefreshMode, SymbolEmbeddingsRefreshPayload, SymbolEmbeddingsRefreshScope,
};
use crate::capability_packs::semantic_clones::runtime_config::{
    embeddings_enabled, resolve_semantic_clones_config,
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
                serde_json::to_value(SemanticFeaturesRefreshPayload {
                    scope: SemanticFeaturesRefreshScope::CurrentPath,
                    path: Some(item.desired.path.clone()),
                    content_id: Some(item.desired.effective_content_id.clone()),
                    inputs: inputs.clone(),
                    mode: SemanticSummaryRefreshMode::ConfiguredStrict,
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

    if !current_projection_changed {
        rehydrate_current_projection_if_needed(cfg, relational, capability_host)
            .await
            .context("rehydrating current semantic clone projection after sync")?;
    }

    crate::capability_packs::semantic_clones::pipeline::rebuild_current_symbol_clone_edges(
        relational,
        &cfg.repo.repo_id,
    )
    .await
    .context("rebuilding current semantic clone edges after sync")?;

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

async fn rehydrate_current_projection_if_needed(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    current_projection: &DevqlCapabilityHost,
) -> Result<()> {
    if !current_projection_needs_rehydrate(relational, &cfg.repo.repo_id).await? {
        return Ok(());
    }

    let path_content_ids =
        load_current_projection_content_ids(relational, &cfg.repo.repo_id).await?;
    if path_content_ids.is_empty() {
        return Ok(());
    }

    let current_inputs =
        crate::capability_packs::semantic_clones::load_semantic_feature_inputs_for_current_repo(
            relational,
            &cfg.repo_root,
            &cfg.repo.repo_id,
        )
        .await?;
    if current_inputs.is_empty() {
        return Ok(());
    }

    let mut inputs_by_path = std::collections::BTreeMap::<String, Vec<_>>::new();
    for input in current_inputs {
        if path_content_ids.contains_key(&input.path) {
            inputs_by_path
                .entry(input.path.clone())
                .or_default()
                .push(input);
        }
    }

    for (path, inputs) in inputs_by_path {
        let Some(content_id) = path_content_ids.get(&path) else {
            continue;
        };
        current_projection
            .invoke_ingester_with_relational(
                crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID,
                crate::capability_packs::semantic_clones::SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID,
                serde_json::to_value(SemanticFeaturesRefreshPayload {
                    scope: SemanticFeaturesRefreshScope::CurrentPath,
                    path: Some(path.clone()),
                    content_id: Some(content_id.clone()),
                    inputs: inputs.clone(),
                    mode: SemanticSummaryRefreshMode::ConfiguredStrict,
                })?,
                Some(relational),
            )
            .await
            .with_context(|| format!("rehydrating current semantic features for `{path}`"))?;
        refresh_current_path_embeddings(
            current_projection,
            relational,
            &path,
            content_id,
            &inputs,
            EmbeddingRepresentationKind::Code,
        )
        .await
        .with_context(|| format!("rehydrating current code embeddings for `{path}`"))?;
        refresh_current_path_embeddings(
            current_projection,
            relational,
            &path,
            content_id,
            &inputs,
            EmbeddingRepresentationKind::Summary,
        )
        .await
        .with_context(|| format!("rehydrating current summary embeddings for `{path}`"))?;
    }

    Ok(())
}

async fn current_projection_needs_rehydrate(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<bool> {
    let counts = relational
        .query_rows(&format!(
            "SELECT
                (SELECT COUNT(*) FROM current_file_state WHERE repo_id = '{repo_id}' AND analysis_mode = 'code' AND effective_content_id IS NOT NULL) AS current_paths,
                (SELECT COUNT(*) FROM symbol_semantics_current WHERE repo_id = '{repo_id}') AS semantic_rows,
                (SELECT COUNT(*) FROM symbol_features_current WHERE repo_id = '{repo_id}') AS feature_rows,
                (SELECT COUNT(*) FROM symbol_embeddings_current WHERE repo_id = '{repo_id}' AND representation_kind = 'code') AS code_embedding_rows,
                (SELECT COUNT(*) FROM symbol_embeddings_current WHERE repo_id = '{repo_id}' AND representation_kind = 'summary') AS summary_embedding_rows",
            repo_id = crate::host::devql::esc_pg(repo_id),
        ))
        .await?;
    let Some(row) = counts.first() else {
        return Ok(false);
    };
    let current_paths = row
        .get("current_paths")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    if current_paths == 0 {
        return Ok(false);
    }

    Ok(row
        .get("semantic_rows")
        .and_then(Value::as_i64)
        .unwrap_or_default()
        == 0
        || row
            .get("feature_rows")
            .and_then(Value::as_i64)
            .unwrap_or_default()
            == 0
        || row
            .get("code_embedding_rows")
            .and_then(Value::as_i64)
            .unwrap_or_default()
            == 0
        || row
            .get("summary_embedding_rows")
            .and_then(Value::as_i64)
            .unwrap_or_default()
            == 0)
}

async fn load_current_projection_content_ids(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<std::collections::BTreeMap<String, String>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT path, effective_content_id
             FROM current_file_state
             WHERE repo_id = '{repo_id}'
               AND analysis_mode = 'code'
               AND effective_content_id IS NOT NULL
             ORDER BY path",
            repo_id = crate::host::devql::esc_pg(repo_id),
        ))
        .await?;

    let mut content_ids = std::collections::BTreeMap::new();
    for row in rows {
        let Some(path) = row.get("path").and_then(Value::as_str) else {
            continue;
        };
        let Some(content_id) = row.get("effective_content_id").and_then(Value::as_str) else {
            continue;
        };
        if path.trim().is_empty() || content_id.trim().is_empty() {
            continue;
        }
        content_ids.insert(path.to_string(), content_id.to_string());
    }
    Ok(content_ids)
}
