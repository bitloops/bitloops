use std::sync::Arc;

use anyhow::{Context, Result};

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::config::{SemanticCloneEmbeddingMode, SemanticClonesConfig, SemanticSummaryMode};
use crate::host::capability_host::CapabilityConfigView;
use crate::host::inference::{EmbeddingService, InferenceGateway};

use super::features::{self, SemanticSummaryProvider};
use super::types::{
    SEMANTIC_CLONES_CODE_EMBEDDINGS_SLOT, SEMANTIC_CLONES_SUMMARY_EMBEDDINGS_SLOT,
    SEMANTIC_CLONES_SUMMARY_GENERATION_SLOT,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryProviderMode {
    DeterministicOnly,
    ConfiguredDegrade,
    ConfiguredStrict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingProviderMode {
    ConfiguredDegrade,
    ConfiguredStrict,
}

pub struct SummaryProviderSelection {
    pub provider: Arc<dyn SemanticSummaryProvider>,
    pub degraded_reason: Option<String>,
    pub slot_name: Option<String>,
    pub profile_name: Option<String>,
}

pub struct EmbeddingProviderSelection {
    pub provider: Option<Arc<dyn EmbeddingService>>,
    pub degraded_reason: Option<String>,
    pub slot_name: Option<String>,
    pub profile_name: Option<String>,
}

pub fn resolve_semantic_clones_config(view: &CapabilityConfigView) -> SemanticClonesConfig {
    view.scoped()
        .cloned()
        .and_then(|value| serde_json::from_value::<SemanticClonesConfig>(value).ok())
        .unwrap_or_default()
}

pub fn embeddings_enabled(config: &SemanticClonesConfig) -> bool {
    config.embedding_mode != SemanticCloneEmbeddingMode::Off
        && (configured_slot_name(config.inference.code_embeddings.as_deref()).is_some()
            || configured_slot_name(config.inference.summary_embeddings.as_deref()).is_some())
}

fn configured_slot_name(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn resolved_profile_name(inference: &dyn InferenceGateway, slot_name: &str) -> Option<String> {
    inference.describe(slot_name).map(|slot| slot.profile_name)
}

pub fn resolve_selected_summary_slot(config: &SemanticClonesConfig) -> Option<String> {
    if config.summary_mode == SemanticSummaryMode::Off {
        return None;
    }

    configured_slot_name(config.inference.summary_generation.as_deref())
        .map(|_| SEMANTIC_CLONES_SUMMARY_GENERATION_SLOT.to_string())
}

pub fn embedding_slot_for_representation(
    config: &SemanticClonesConfig,
    representation_kind: EmbeddingRepresentationKind,
) -> Option<String> {
    if config.embedding_mode == SemanticCloneEmbeddingMode::Off {
        return None;
    }

    match representation_kind {
        EmbeddingRepresentationKind::Code => {
            configured_slot_name(config.inference.code_embeddings.as_deref())
                .map(|_| SEMANTIC_CLONES_CODE_EMBEDDINGS_SLOT.to_string())
        }
        EmbeddingRepresentationKind::Summary => {
            configured_slot_name(config.inference.summary_embeddings.as_deref())
                .map(|_| SEMANTIC_CLONES_SUMMARY_EMBEDDINGS_SLOT.to_string())
        }
    }
}

pub fn resolve_summary_provider(
    config: &SemanticClonesConfig,
    inference: &dyn InferenceGateway,
    mode: SummaryProviderMode,
) -> Result<SummaryProviderSelection> {
    if matches!(mode, SummaryProviderMode::DeterministicOnly)
        || config.summary_mode == SemanticSummaryMode::Off
    {
        return Ok(SummaryProviderSelection {
            provider: Arc::new(features::NoopSemanticSummaryProvider),
            degraded_reason: None,
            slot_name: None,
            profile_name: None,
        });
    }

    let Some(slot_name) = resolve_selected_summary_slot(config) else {
        return Ok(SummaryProviderSelection {
            provider: Arc::new(features::NoopSemanticSummaryProvider),
            degraded_reason: None,
            slot_name: None,
            profile_name: None,
        });
    };
    let profile_name = resolved_profile_name(inference, &slot_name);

    match inference.text_generation(&slot_name) {
        Ok(service) => Ok(SummaryProviderSelection {
            provider: features::summary_provider_from_service(service),
            degraded_reason: None,
            slot_name: Some(slot_name),
            profile_name,
        }),
        Err(err) if matches!(mode, SummaryProviderMode::ConfiguredDegrade) => {
            let message = format!("{err:#}");
            log::warn!(
                "semantic_clones semantic summaries degraded; using deterministic summaries only: {message}"
            );
            Ok(SummaryProviderSelection {
                provider: Arc::new(features::NoopSemanticSummaryProvider),
                degraded_reason: Some(message),
                slot_name: Some(slot_name),
                profile_name,
            })
        }
        Err(err) => Err(err).with_context(|| {
            format!(
                "resolving semantic summary provider for slot `{}`{}",
                slot_name,
                profile_name
                    .as_deref()
                    .map(|name| format!(" (profile `{name}`)"))
                    .unwrap_or_default()
            )
        }),
    }
}

pub fn resolve_embedding_provider(
    config: &SemanticClonesConfig,
    inference: &dyn InferenceGateway,
    representation_kind: EmbeddingRepresentationKind,
    mode: EmbeddingProviderMode,
) -> Result<EmbeddingProviderSelection> {
    let slot_name = embedding_slot_for_representation(config, representation_kind);
    if slot_name.is_none() {
        return Ok(EmbeddingProviderSelection {
            provider: None,
            degraded_reason: None,
            slot_name: None,
            profile_name: None,
        });
    }

    let slot_name = slot_name.expect("checked above");
    let profile_name = resolved_profile_name(inference, &slot_name);
    match inference.embeddings(&slot_name) {
        Ok(service) => Ok(EmbeddingProviderSelection {
            provider: Some(service),
            degraded_reason: None,
            slot_name: Some(slot_name),
            profile_name,
        }),
        Err(err) if matches!(mode, EmbeddingProviderMode::ConfiguredDegrade) => {
            Ok(EmbeddingProviderSelection {
                provider: None,
                degraded_reason: Some(format!("{err:#}")),
                slot_name: Some(slot_name),
                profile_name,
            })
        }
        Err(err) => Err(err).with_context(|| {
            format!(
                "resolving embedding provider for slot `{}`{}",
                slot_name,
                profile_name
                    .as_deref()
                    .map(|name| format!(" (profile `{name}`)"))
                    .unwrap_or_default()
            )
        }),
    }
}
