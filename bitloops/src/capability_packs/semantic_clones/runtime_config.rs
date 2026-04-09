use std::sync::Arc;

use anyhow::{Context, Result};

use crate::adapters::model_providers::embeddings::EmbeddingProvider;
use crate::config::{SemanticCloneEmbeddingMode, SemanticClonesConfig, SemanticSummaryMode};
use crate::host::capability_host::CapabilityConfigView;
use crate::host::inference::{DEFAULT_TEXT_GENERATION_PROFILE_ID, InferenceGateway};

use super::embeddings;
use super::features::{self, SemanticSummaryProvider};

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
    pub profile_name: Option<String>,
}

pub struct EmbeddingProviderSelection {
    pub provider: Option<Arc<dyn EmbeddingProvider>>,
    pub degraded_reason: Option<String>,
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
        && config
            .embedding_profile
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

pub fn resolve_selected_summary_profile(
    config: &SemanticClonesConfig,
    inference: &dyn InferenceGateway,
) -> Option<String> {
    if config.summary_mode == SemanticSummaryMode::Off {
        return None;
    }
    if let Some(profile_name) = config
        .summary_profile
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(profile_name.to_string());
    }
    inference
        .has_text_generation_profile(DEFAULT_TEXT_GENERATION_PROFILE_ID)
        .then_some(DEFAULT_TEXT_GENERATION_PROFILE_ID.to_string())
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
            profile_name: None,
        });
    }

    let Some(profile_name) = resolve_selected_summary_profile(config, inference) else {
        return Ok(SummaryProviderSelection {
            provider: Arc::new(features::NoopSemanticSummaryProvider),
            degraded_reason: None,
            profile_name: None,
        });
    };

    match inference.text_generation(&profile_name) {
        Ok(service) => Ok(SummaryProviderSelection {
            provider: features::summary_provider_from_service(service),
            degraded_reason: None,
            profile_name: Some(profile_name),
        }),
        Err(err) if matches!(mode, SummaryProviderMode::ConfiguredDegrade) => {
            let message = format!("{err:#}");
            log::warn!(
                "semantic_clones semantic summaries degraded; using deterministic summaries only: {message}"
            );
            Ok(SummaryProviderSelection {
                provider: Arc::new(features::NoopSemanticSummaryProvider),
                degraded_reason: Some(message),
                profile_name: Some(profile_name),
            })
        }
        Err(err) => Err(err).with_context(|| {
            format!("resolving semantic summary provider for profile `{profile_name}`")
        }),
    }
}

pub fn resolve_embedding_provider(
    config: &SemanticClonesConfig,
    inference: &dyn InferenceGateway,
    mode: EmbeddingProviderMode,
) -> Result<EmbeddingProviderSelection> {
    let profile_name = config
        .embedding_profile
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if config.embedding_mode == SemanticCloneEmbeddingMode::Off || profile_name.is_none() {
        return Ok(EmbeddingProviderSelection {
            provider: None,
            degraded_reason: None,
            profile_name,
        });
    }

    let profile_name = profile_name.expect("checked above");
    match inference.embeddings(&profile_name) {
        Ok(service) => Ok(EmbeddingProviderSelection {
            provider: Some(embeddings::provider_from_service(service)),
            degraded_reason: None,
            profile_name: Some(profile_name),
        }),
        Err(err) if matches!(mode, EmbeddingProviderMode::ConfiguredDegrade) => {
            Ok(EmbeddingProviderSelection {
                provider: None,
                degraded_reason: Some(format!("{err:#}")),
                profile_name: Some(profile_name),
            })
        }
        Err(err) => Err(err)
            .with_context(|| format!("resolving embedding provider for profile `{profile_name}`")),
    }
}
