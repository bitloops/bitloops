use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::capability_packs::semantic_clones::embeddings::{
    ActiveEmbeddingRepresentationState, EmbeddingRepresentationKind, resolve_embedding_setup,
};
use crate::capability_packs::semantic_clones::features::{
    SemanticFeatureInput, build_semantic_feature_input_hash,
};
use crate::capability_packs::semantic_clones::runtime_config::{
    EmbeddingProviderMode, SummaryProviderMode, resolve_embedding_provider,
    resolve_semantic_clones_config, resolve_summary_provider,
};
use crate::host::capability_host::registrar::{
    BoxFuture, IngestRequest, IngestResult, IngesterHandler, IngesterRegistration,
};
use crate::host::devql::RelationalStorage;

use super::super::types::{
    SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID,
    SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
};
use crate::capability_packs::semantic_clones::{
    RepoEmbeddingSyncAction, clear_current_symbol_embedding_rows_for_path,
    clear_repo_active_embedding_setup, clear_repo_active_embedding_setup_for_representation,
    clear_repo_symbol_embedding_rows, clear_repo_symbol_embedding_rows_for_representation,
    determine_repo_embedding_sync_action, load_semantic_summary_snapshot,
    persist_active_embedding_setup, refresh_current_repo_symbol_embeddings_and_clone_edges,
    upsert_current_semantic_feature_rows, upsert_current_symbol_embedding_rows,
    upsert_semantic_feature_rows, upsert_symbol_embedding_rows,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticFeaturesRefreshScope {
    #[default]
    Historical,
    CurrentPath,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticSummaryRefreshMode {
    DeterministicOnly,
    #[default]
    ConfiguredDegrade,
    ConfiguredStrict,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticFeaturesRefreshPayload {
    #[serde(default)]
    pub scope: SemanticFeaturesRefreshScope,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub content_id: Option<String>,
    #[serde(default)]
    pub inputs: Vec<SemanticFeatureInput>,
    #[serde(default)]
    pub mode: SemanticSummaryRefreshMode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolEmbeddingsRefreshScope {
    #[default]
    Historical,
    CurrentPath,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingRefreshMode {
    #[default]
    ConfiguredDegrade,
    ConfiguredStrict,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolEmbeddingsRefreshPayload {
    #[serde(default)]
    pub scope: SymbolEmbeddingsRefreshScope,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub content_id: Option<String>,
    #[serde(default)]
    pub inputs: Vec<SemanticFeatureInput>,
    #[serde(default)]
    pub expected_input_hashes: BTreeMap<String, String>,
    #[serde(default)]
    pub representation_kind: EmbeddingRepresentationKind,
    #[serde(default)]
    pub mode: EmbeddingRefreshMode,
    #[serde(default)]
    pub manage_active_state: bool,
}

struct SemanticFeaturesRefreshIngester;

impl IngesterHandler for SemanticFeaturesRefreshIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        ctx: &'a mut dyn crate::host::capability_host::CapabilityIngestContext,
    ) -> BoxFuture<'a, Result<IngestResult>> {
        Box::pin(async move {
            let payload: SemanticFeaturesRefreshPayload = request.parse_json()?;
            let config = resolve_semantic_clones_config(
                &ctx.config_view(SEMANTIC_CLONES_CAPABILITY_ID)
                    .context("loading semantic_clones config view")?,
            );
            let summary_provider = resolve_summary_provider(
                &config,
                ctx.inference(),
                match payload.mode {
                    SemanticSummaryRefreshMode::DeterministicOnly => {
                        SummaryProviderMode::DeterministicOnly
                    }
                    SemanticSummaryRefreshMode::ConfiguredDegrade => {
                        SummaryProviderMode::ConfiguredDegrade
                    }
                    SemanticSummaryRefreshMode::ConfiguredStrict => {
                        SummaryProviderMode::ConfiguredStrict
                    }
                },
            )?;
            let input_hashes = payload
                .inputs
                .iter()
                .map(|input| {
                    (
                        input.artefact_id.clone(),
                        build_semantic_feature_input_hash(
                            input,
                            summary_provider.provider.as_ref(),
                        ),
                    )
                })
                .collect::<BTreeMap<_, _>>();

            let stats = match payload.scope {
                SemanticFeaturesRefreshScope::Historical => {
                    upsert_semantic_feature_rows(
                        current_relational(ctx)?,
                        &payload.inputs,
                        Arc::clone(&summary_provider.provider),
                    )
                    .await?
                }
                SemanticFeaturesRefreshScope::CurrentPath => {
                    let path = required_field(payload.path.as_deref(), "path")?;
                    let content_id = required_field(payload.content_id.as_deref(), "content_id")?;
                    upsert_current_semantic_feature_rows(
                        current_relational(ctx)?,
                        path,
                        content_id,
                        &payload.inputs,
                        Arc::clone(&summary_provider.provider),
                    )
                    .await?
                }
            };
            let produced_enriched_semantics =
                any_llm_enriched_rows(current_relational(ctx)?, &payload.inputs).await?;

            Ok(IngestResult::new(
                json!({
                    "semantic_feature_rows_upserted": stats.upserted,
                    "semantic_feature_rows_skipped": stats.skipped,
                    "produced_enriched_semantics": produced_enriched_semantics,
                    "input_hashes": input_hashes,
                    "degraded_summary_provider": summary_provider.degraded_reason.is_some(),
                    "summary_slot": summary_provider.slot_name,
                    "summary_inference_profile": summary_provider.profile_name,
                    "degraded_reason": summary_provider.degraded_reason,
                }),
                format!(
                    "refreshed semantic feature rows: upserted={}, skipped={}",
                    stats.upserted, stats.skipped
                ),
            ))
        })
    }
}

struct SymbolEmbeddingsRefreshIngester;

impl IngesterHandler for SymbolEmbeddingsRefreshIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        ctx: &'a mut dyn crate::host::capability_host::CapabilityIngestContext,
    ) -> BoxFuture<'a, Result<IngestResult>> {
        Box::pin(async move {
            let payload: SymbolEmbeddingsRefreshPayload = request.parse_json()?;
            let config = resolve_semantic_clones_config(
                &ctx.config_view(SEMANTIC_CLONES_CAPABILITY_ID)
                    .context("loading semantic_clones config view")?,
            );
            let provider = resolve_embedding_provider(
                &config,
                ctx.inference(),
                payload.representation_kind,
                match payload.mode {
                    EmbeddingRefreshMode::ConfiguredDegrade => {
                        EmbeddingProviderMode::ConfiguredDegrade
                    }
                    EmbeddingRefreshMode::ConfiguredStrict => {
                        EmbeddingProviderMode::ConfiguredStrict
                    }
                },
            )?;
            let provider_profile_name = provider.profile_name.clone();
            let provider_slot_name = provider.slot_name.clone();
            let relational = current_relational(ctx)?;

            match payload.scope {
                SymbolEmbeddingsRefreshScope::CurrentPath => {
                    return handle_current_path_embedding_refresh(
                        relational,
                        ctx.repo().repo_id.as_str(),
                        &payload,
                        provider,
                    )
                    .await;
                }
                SymbolEmbeddingsRefreshScope::Historical => {}
            }

            if provider.provider.is_none() {
                if payload.manage_active_state {
                    clear_embedding_outputs(relational, ctx.repo().repo_id.as_str()).await?;
                }
                return Ok(IngestResult::new(
                    json!({
                        "symbol_embedding_rows_upserted": 0,
                        "symbol_embedding_rows_skipped": 0,
                        "symbol_embedding_rows_eligible": 0,
                        "provider_available": false,
                        "embedding_slot": provider.slot_name,
                        "degraded_reason": provider.degraded_reason,
                        "embedding_inference_profile": provider.profile_name,
                        "sync_action": serde_json::Value::Null,
                        "clone_rebuild_performed": false,
                        "clone_rebuild_recommended": false,
                        "symbol_clone_edges_upserted": 0,
                        "symbol_clone_sources_scored": 0,
                    }),
                    "symbol embeddings unavailable",
                ));
            }

            let provider = provider.provider.expect("checked above");
            let setup = resolve_embedding_setup(provider.as_ref())?;
            let sync_action = if payload.manage_active_state {
                determine_repo_embedding_sync_action(
                    relational,
                    &ctx.repo().repo_id,
                    payload.representation_kind,
                    &setup,
                )
                .await?
            } else {
                RepoEmbeddingSyncAction::Incremental
            };

            if payload.manage_active_state
                && sync_action == RepoEmbeddingSyncAction::RefreshCurrentRepo
            {
                clear_repo_symbol_embedding_rows_for_representation(
                    relational,
                    &ctx.repo().repo_id,
                    payload.representation_kind,
                )
                .await?;
                clear_repo_active_embedding_setup_for_representation(
                    relational,
                    &ctx.repo().repo_id,
                    payload.representation_kind,
                )
                .await?;
                let summary_provider = resolve_summary_provider(
                    &config,
                    ctx.inference(),
                    match payload.mode {
                        EmbeddingRefreshMode::ConfiguredDegrade => {
                            SummaryProviderMode::ConfiguredDegrade
                        }
                        EmbeddingRefreshMode::ConfiguredStrict => {
                            SummaryProviderMode::ConfiguredStrict
                        }
                    },
                )?;
                let refresh = refresh_current_repo_symbol_embeddings_and_clone_edges(
                    relational,
                    ctx.repo_root(),
                    &ctx.repo().repo_id,
                    summary_provider.provider,
                    payload.representation_kind,
                    provider,
                )
                .await?;
                return Ok(IngestResult::new(
                    json!({
                        "semantic_feature_rows_upserted": refresh.semantic_feature_stats.upserted,
                        "semantic_feature_rows_skipped": refresh.semantic_feature_stats.skipped,
                        "symbol_embedding_rows_upserted": refresh.embedding_stats.upserted,
                        "symbol_embedding_rows_skipped": refresh.embedding_stats.skipped,
                        "symbol_embedding_rows_eligible": refresh.embedding_stats.eligible,
                        "provider_available": true,
                        "embedding_slot": provider_slot_name.as_deref(),
                        "embedding_inference_profile": provider_profile_name.as_deref(),
                        "sync_action": sync_action.to_string(),
                        "clone_rebuild_performed": payload.representation_kind == EmbeddingRepresentationKind::Code,
                        "clone_rebuild_recommended": false,
                        "symbol_clone_edges_upserted": refresh.clone_build.edges.len(),
                        "symbol_clone_sources_scored": refresh.clone_build.sources_considered,
                    }),
                    "refreshed symbol embeddings from current repo",
                ));
            }

            let inputs = if payload.expected_input_hashes.is_empty() {
                payload.inputs.clone()
            } else {
                filter_current_inputs(relational, &payload.inputs, &payload.expected_input_hashes)
                    .await?
            };
            let stats = upsert_symbol_embedding_rows(
                relational,
                &inputs,
                payload.representation_kind,
                Arc::clone(&provider),
            )
            .await?;

            if payload.manage_active_state && stats.eligible > 0 {
                persist_active_embedding_setup(
                    relational,
                    &ctx.repo().repo_id,
                    &ActiveEmbeddingRepresentationState::new(payload.representation_kind, setup),
                )
                .await?;
            }

            let clone_rebuild_recommended = payload.manage_active_state
                && payload.representation_kind == EmbeddingRepresentationKind::Code
                && (stats.eligible > 0 || sync_action == RepoEmbeddingSyncAction::AdoptExisting);
            Ok(IngestResult::new(
                json!({
                    "semantic_feature_rows_upserted": 0,
                    "semantic_feature_rows_skipped": 0,
                    "symbol_embedding_rows_upserted": stats.upserted,
                    "symbol_embedding_rows_skipped": stats.skipped,
                    "symbol_embedding_rows_eligible": stats.eligible,
                    "provider_available": true,
                    "embedding_slot": provider_slot_name.as_deref(),
                    "degraded_reason": serde_json::Value::Null,
                    "embedding_inference_profile": provider_profile_name.as_deref(),
                    "sync_action": if payload.manage_active_state { json!(sync_action.to_string()) } else { serde_json::Value::Null },
                    "clone_rebuild_performed": false,
                    "clone_rebuild_recommended": clone_rebuild_recommended,
                    "symbol_clone_edges_upserted": 0,
                    "symbol_clone_sources_scored": 0,
                }),
                format!(
                    "refreshed symbol embeddings: eligible={}, upserted={}, skipped={}",
                    stats.eligible, stats.upserted, stats.skipped
                ),
            ))
        })
    }
}

async fn handle_current_path_embedding_refresh(
    relational: &RelationalStorage,
    repo_id: &str,
    payload: &SymbolEmbeddingsRefreshPayload,
    provider: crate::capability_packs::semantic_clones::runtime_config::EmbeddingProviderSelection,
) -> Result<IngestResult> {
    let path = required_field(payload.path.as_deref(), "path")?;
    let content_id = required_field(payload.content_id.as_deref(), "content_id")?;
    let profile_name = provider.profile_name.clone();
    let slot_name = provider.slot_name.clone();
    let degraded_reason = provider.degraded_reason.clone();

    let Some(provider) = provider.provider else {
        clear_current_symbol_embedding_rows_for_path(relational, repo_id, path).await?;
        return Ok(IngestResult::new(
            json!({
                "semantic_feature_rows_upserted": 0,
                "semantic_feature_rows_skipped": 0,
                "symbol_embedding_rows_upserted": 0,
                "symbol_embedding_rows_skipped": 0,
                "symbol_embedding_rows_eligible": 0,
                "provider_available": false,
                "embedding_slot": slot_name.as_deref(),
                "degraded_reason": degraded_reason,
                "embedding_inference_profile": profile_name,
                "sync_action": serde_json::Value::Null,
                "clone_rebuild_performed": false,
                "clone_rebuild_recommended": false,
                "symbol_clone_edges_upserted": 0,
                "symbol_clone_sources_scored": 0,
            }),
            "current-path symbol embeddings unavailable",
        ));
    };

    let stats = upsert_current_symbol_embedding_rows(
        relational,
        path,
        content_id,
        &payload.inputs,
        payload.representation_kind,
        provider,
    )
    .await?;
    Ok(IngestResult::new(
        json!({
            "semantic_feature_rows_upserted": 0,
            "semantic_feature_rows_skipped": 0,
            "symbol_embedding_rows_upserted": stats.upserted,
            "symbol_embedding_rows_skipped": stats.skipped,
            "symbol_embedding_rows_eligible": stats.eligible,
            "provider_available": true,
            "embedding_slot": slot_name.as_deref(),
            "degraded_reason": serde_json::Value::Null,
            "embedding_inference_profile": profile_name,
            "sync_action": serde_json::Value::Null,
            "clone_rebuild_performed": false,
            "clone_rebuild_recommended": false,
            "symbol_clone_edges_upserted": 0,
            "symbol_clone_sources_scored": 0,
        }),
        format!(
            "refreshed current-path symbol embeddings: eligible={}, upserted={}, skipped={}",
            stats.eligible, stats.upserted, stats.skipped
        ),
    ))
}

fn current_relational(
    ctx: &dyn crate::host::capability_host::CapabilityIngestContext,
) -> Result<&RelationalStorage> {
    ctx.devql_relational()
        .ok_or_else(|| anyhow!("DevQL relational store is not attached to this ingest"))
}

fn required_field<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("`{field}` is required"))
}

async fn clear_embedding_outputs(relational: &RelationalStorage, repo_id: &str) -> Result<()> {
    clear_repo_symbol_embedding_rows(relational, repo_id).await?;
    clear_repo_active_embedding_setup(relational, repo_id).await?;
    crate::capability_packs::semantic_clones::pipeline::delete_repo_symbol_clone_edges(
        relational, repo_id,
    )
    .await
}

async fn any_llm_enriched_rows(
    relational: &RelationalStorage,
    inputs: &[SemanticFeatureInput],
) -> Result<bool> {
    for input in inputs {
        if load_semantic_summary_snapshot(relational, &input.artefact_id)
            .await?
            .is_some_and(|snapshot| snapshot.is_llm_enriched())
        {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn filter_current_inputs(
    relational: &RelationalStorage,
    inputs: &[SemanticFeatureInput],
    expected_input_hashes: &BTreeMap<String, String>,
) -> Result<Vec<SemanticFeatureInput>> {
    let mut filtered = Vec::with_capacity(inputs.len());
    for input in inputs {
        let Some(expected_hash) = expected_input_hashes.get(&input.artefact_id) else {
            continue;
        };
        let Some(snapshot) = load_semantic_summary_snapshot(relational, &input.artefact_id).await?
        else {
            continue;
        };
        if snapshot.semantic_features_input_hash == *expected_hash {
            filtered.push(input.clone());
        }
    }
    Ok(filtered)
}

pub fn build_semantic_features_refresh_ingester() -> IngesterRegistration {
    IngesterRegistration::new(
        SEMANTIC_CLONES_CAPABILITY_ID,
        SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID,
        Arc::new(SemanticFeaturesRefreshIngester),
    )
}

pub fn build_symbol_embeddings_refresh_ingester() -> IngesterRegistration {
    IngesterRegistration::new(
        SEMANTIC_CLONES_CAPABILITY_ID,
        SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
        Arc::new(SymbolEmbeddingsRefreshIngester),
    )
}

trait RepoEmbeddingSyncActionExt {
    fn to_string(self) -> &'static str;
}

impl RepoEmbeddingSyncActionExt for RepoEmbeddingSyncAction {
    fn to_string(self) -> &'static str {
        match self {
            RepoEmbeddingSyncAction::Incremental => "incremental",
            RepoEmbeddingSyncAction::AdoptExisting => "adopt_existing",
            RepoEmbeddingSyncAction::RefreshCurrentRepo => "refresh_current_repo",
        }
    }
}
