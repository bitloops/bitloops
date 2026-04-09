use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow, bail};

use crate::adapters::model_providers::embeddings::{
    self as embedding_runtime, EmbeddingRuntimeClientConfig,
};
use crate::adapters::model_providers::llm::{self, LlmProvider};
use crate::config::{EmbeddingsConfig, StoreSemanticConfig};

pub const DEFAULT_TEXT_GENERATION_PROFILE_ID: &str = "default";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingInputType {
    Document,
    Query,
}

impl From<EmbeddingInputType> for embedding_runtime::EmbeddingInputType {
    fn from(value: EmbeddingInputType) -> Self {
        match value {
            EmbeddingInputType::Document => Self::Document,
            EmbeddingInputType::Query => Self::Query,
        }
    }
}

pub trait EmbeddingService: Send + Sync {
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    fn output_dimension(&self) -> Option<usize>;
    fn cache_key(&self) -> String;
    fn embed(&self, input: &str, input_type: EmbeddingInputType) -> Result<Vec<f32>>;
}

pub trait TextGenerationService: Send + Sync {
    fn descriptor(&self) -> String;
    fn cache_key(&self) -> String {
        self.descriptor()
    }
    fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String>;
}

pub trait InferenceGateway: Send + Sync {
    fn embeddings(&self, profile_id: &str) -> Result<Arc<dyn EmbeddingService>>;
    fn text_generation(&self, profile_id: &str) -> Result<Arc<dyn TextGenerationService>>;

    fn has_embedding_profile(&self, _profile_id: &str) -> bool {
        false
    }

    fn has_text_generation_profile(&self, _profile_id: &str) -> bool {
        false
    }
}

pub struct EmptyInferenceGateway;

impl InferenceGateway for EmptyInferenceGateway {
    fn embeddings(&self, profile_id: &str) -> Result<Arc<dyn EmbeddingService>> {
        bail!("embedding inference is not available for profile `{profile_id}`")
    }

    fn text_generation(&self, profile_id: &str) -> Result<Arc<dyn TextGenerationService>> {
        bail!("text-generation inference is not available for profile `{profile_id}`")
    }
}

pub struct LocalInferenceGateway {
    repo_root: PathBuf,
    daemon_config_path: PathBuf,
    embeddings: EmbeddingsConfig,
    legacy_semantic: StoreSemanticConfig,
    embedding_cache: Mutex<HashMap<String, Arc<dyn EmbeddingService>>>,
    text_generation_cache: Mutex<HashMap<String, Arc<dyn TextGenerationService>>>,
}

impl LocalInferenceGateway {
    pub fn new(
        repo_root: &Path,
        daemon_config_path: PathBuf,
        embeddings: EmbeddingsConfig,
        legacy_semantic: StoreSemanticConfig,
    ) -> Self {
        Self {
            repo_root: repo_root.to_path_buf(),
            daemon_config_path,
            embeddings,
            legacy_semantic,
            embedding_cache: Mutex::new(HashMap::new()),
            text_generation_cache: Mutex::new(HashMap::new()),
        }
    }

    fn build_embedding_service(&self, profile_id: &str) -> Result<Arc<dyn EmbeddingService>> {
        let Some(profile) = self.embeddings.profiles.get(profile_id) else {
            bail!("embedding profile `{profile_id}` is not defined");
        };

        let runtime = EmbeddingRuntimeClientConfig {
            command: self.embeddings.runtime.command.clone(),
            args: self.embeddings.runtime.args.clone(),
            startup_timeout_secs: self.embeddings.runtime.startup_timeout_secs,
            request_timeout_secs: self.embeddings.runtime.request_timeout_secs,
            config_path: self.daemon_config_path.clone(),
            profile_name: profile.name.clone(),
            repo_root: Some(self.repo_root.clone()),
        };
        let provider = embedding_runtime::build_embedding_provider(&runtime)
            .with_context(|| format!("building embeddings service for profile `{profile_id}`"))?;
        Ok(Arc::new(RuntimeEmbeddingService { inner: provider }))
    }

    fn build_text_generation_service(
        &self,
        profile_id: &str,
    ) -> Result<Arc<dyn TextGenerationService>> {
        let profile = self.resolve_text_generation_profile(profile_id)?;
        let provider = llm::build_llm_provider(
            &profile.provider,
            profile.model,
            profile.api_key,
            profile.base_url.as_deref(),
        )
        .with_context(|| format!("building text-generation service for profile `{profile_id}`"))?;
        Ok(Arc::new(LlmTextGenerationService { inner: provider }))
    }

    fn resolve_text_generation_profile(&self, profile_id: &str) -> Result<ResolvedSummaryProfile> {
        if profile_id != DEFAULT_TEXT_GENERATION_PROFILE_ID {
            bail!("text-generation profile `{profile_id}` is not defined");
        }

        let provider = self
            .legacy_semantic
            .semantic_provider
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if provider.is_empty() || matches!(provider.as_str(), "none" | "disabled") {
            bail!("text-generation profile `{profile_id}` is not configured");
        }

        let model = self
            .legacy_semantic
            .semantic_model
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("semantic model is required for profile `{profile_id}`"))?
            .trim()
            .to_string();
        let api_key = self
            .legacy_semantic
            .semantic_api_key
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("semantic API key is required for profile `{profile_id}`"))?
            .trim()
            .to_string();
        let base_url = self
            .legacy_semantic
            .semantic_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        Ok(ResolvedSummaryProfile {
            provider,
            model,
            api_key,
            base_url,
        })
    }
}

impl InferenceGateway for LocalInferenceGateway {
    fn embeddings(&self, profile_id: &str) -> Result<Arc<dyn EmbeddingService>> {
        if let Some(service) = self
            .embedding_cache
            .lock()
            .map_err(|_| anyhow!("embedding inference cache mutex was poisoned"))?
            .get(profile_id)
            .cloned()
        {
            return Ok(service);
        }

        let service = self.build_embedding_service(profile_id)?;
        let mut cache = self
            .embedding_cache
            .lock()
            .map_err(|_| anyhow!("embedding inference cache mutex was poisoned"))?;
        Ok(cache
            .entry(profile_id.to_string())
            .or_insert_with(|| Arc::clone(&service))
            .clone())
    }

    fn text_generation(&self, profile_id: &str) -> Result<Arc<dyn TextGenerationService>> {
        if let Some(service) = self
            .text_generation_cache
            .lock()
            .map_err(|_| anyhow!("text-generation inference cache mutex was poisoned"))?
            .get(profile_id)
            .cloned()
        {
            return Ok(service);
        }

        let service = self.build_text_generation_service(profile_id)?;
        let mut cache = self
            .text_generation_cache
            .lock()
            .map_err(|_| anyhow!("text-generation inference cache mutex was poisoned"))?;
        Ok(cache
            .entry(profile_id.to_string())
            .or_insert_with(|| Arc::clone(&service))
            .clone())
    }

    fn has_embedding_profile(&self, profile_id: &str) -> bool {
        self.embeddings.profiles.contains_key(profile_id)
    }

    fn has_text_generation_profile(&self, profile_id: &str) -> bool {
        self.resolve_text_generation_profile(profile_id).is_ok()
    }
}

struct RuntimeEmbeddingService {
    inner: Box<dyn embedding_runtime::EmbeddingProvider>,
}

impl EmbeddingService for RuntimeEmbeddingService {
    fn provider_name(&self) -> &str {
        self.inner.provider_name()
    }

    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn output_dimension(&self) -> Option<usize> {
        self.inner.output_dimension()
    }

    fn cache_key(&self) -> String {
        self.inner.cache_key()
    }

    fn embed(&self, input: &str, input_type: EmbeddingInputType) -> Result<Vec<f32>> {
        self.inner.embed(input, input_type.into())
    }
}

struct LlmTextGenerationService {
    inner: Box<dyn LlmProvider>,
}

impl TextGenerationService for LlmTextGenerationService {
    fn descriptor(&self) -> String {
        self.inner.descriptor()
    }

    fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        self.inner
            .complete(system_prompt, user_prompt)
            .ok_or_else(|| {
                anyhow!(
                    "text-generation provider `{}` returned no content",
                    self.descriptor()
                )
            })
    }
}

struct ResolvedSummaryProfile {
    provider: String,
    model: String,
    api_key: String,
    base_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EmbeddingProfileConfig, EmbeddingsRuntimeConfig};
    use std::collections::BTreeMap;

    #[test]
    fn local_inference_gateway_reports_available_profiles() {
        let mut profiles = BTreeMap::new();
        profiles.insert(
            "local".to_string(),
            EmbeddingProfileConfig {
                name: "local".to_string(),
                kind: "local_fastembed".to_string(),
                model: Some("jinaai/jina-embeddings-v2-base-code".to_string()),
                api_key: None,
                base_url: None,
                cache_dir: None,
            },
        );
        let gateway = LocalInferenceGateway::new(
            Path::new("/repo"),
            PathBuf::from("/config.toml"),
            EmbeddingsConfig {
                runtime: EmbeddingsRuntimeConfig::default(),
                profiles,
                warnings: Vec::new(),
            },
            StoreSemanticConfig {
                semantic_provider: Some("openai".to_string()),
                semantic_model: Some("gpt-test".to_string()),
                semantic_api_key: Some("secret".to_string()),
                semantic_base_url: None,
            },
        );

        assert!(gateway.has_embedding_profile("local"));
        assert!(!gateway.has_embedding_profile("missing"));
        assert!(gateway.has_text_generation_profile(DEFAULT_TEXT_GENERATION_PROFILE_ID));
        assert!(!gateway.has_text_generation_profile("missing"));
    }

    #[test]
    fn local_inference_gateway_rejects_unknown_profiles() {
        let gateway = LocalInferenceGateway::new(
            Path::new("/repo"),
            PathBuf::from("/config.toml"),
            EmbeddingsConfig::default(),
            StoreSemanticConfig::default(),
        );

        assert!(
            gateway
                .embeddings("missing")
                .err()
                .expect("missing embedding profile")
                .to_string()
                .contains("not defined")
        );
        assert!(
            gateway
                .text_generation("missing")
                .err()
                .expect("missing text-generation profile")
                .to_string()
                .contains("not defined")
        );
    }
}
