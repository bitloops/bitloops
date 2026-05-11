use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow, bail};

use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, InferenceConfig, InferenceProfileConfig, InferenceRuntimeConfig,
    InferenceTask, resolve_preferred_daemon_config_path_for_repo,
};

use super::embeddings::BitloopsEmbeddingsIpcService;
use super::text_generation::{
    BitloopsInferenceTextGenerationService, TextGenerationRequestDefaults,
};
use super::{
    BITLOOPS_EMBEDDINGS_IPC_DRIVER, BITLOOPS_PLATFORM_CHAT_DRIVER,
    BITLOOPS_PLATFORM_EMBEDDINGS_RUNTIME_ID, CLAUDE_CODE_PRINT_DRIVER, CODEX_EXEC_DRIVER,
    EmbeddingService, InferenceGateway, ResolvedInferenceSlot, StructuredGenerationService,
    TextGenerationService,
};

pub struct EmptyInferenceGateway;

impl InferenceGateway for EmptyInferenceGateway {
    fn embeddings(&self, slot_name: &str) -> Result<Arc<dyn EmbeddingService>> {
        bail!("embedding inference is not available for slot `{slot_name}`")
    }

    fn text_generation(&self, slot_name: &str) -> Result<Arc<dyn TextGenerationService>> {
        bail!("text-generation inference is not available for slot `{slot_name}`")
    }

    fn structured_generation(
        &self,
        slot_name: &str,
    ) -> Result<Arc<dyn StructuredGenerationService>> {
        bail!("structured-generation inference is not available for slot `{slot_name}`")
    }
}

#[derive(Clone)]
pub struct LocalInferenceGateway {
    repo_root: PathBuf,
    inference: InferenceConfig,
    slot_bindings: HashMap<String, BTreeMap<String, String>>,
    embedding_cache: Arc<Mutex<HashMap<String, Arc<dyn EmbeddingService>>>>,
    text_generation_cache: Arc<Mutex<HashMap<String, Arc<dyn TextGenerationService>>>>,
    structured_generation_cache: Arc<Mutex<HashMap<String, Arc<dyn StructuredGenerationService>>>>,
}

impl LocalInferenceGateway {
    pub fn new(
        repo_root: &Path,
        inference: InferenceConfig,
        slot_bindings: HashMap<String, BTreeMap<String, String>>,
    ) -> Self {
        Self {
            repo_root: repo_root.to_path_buf(),
            inference,
            slot_bindings,
            embedding_cache: Arc::new(Mutex::new(HashMap::new())),
            text_generation_cache: Arc::new(Mutex::new(HashMap::new())),
            structured_generation_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn scoped<'a>(&'a self, capability_id: Option<&'a str>) -> ScopedInferenceGateway<'a> {
        ScopedInferenceGateway {
            inner: self,
            capability_id,
        }
    }

    pub fn owned_scoped(&self, capability_id: Option<&str>) -> OwnedScopedInferenceGateway {
        OwnedScopedInferenceGateway {
            inner: self.clone(),
            capability_id: capability_id.map(ToOwned::to_owned),
        }
    }

    fn bound_profile_name(&self, capability_id: Option<&str>, slot_name: &str) -> Option<String> {
        let capability_id = capability_id?;
        self.slot_bindings
            .get(capability_id)
            .and_then(|slots| slots.get(slot_name))
            .cloned()
    }

    fn describe_slot(
        &self,
        capability_id: Option<&str>,
        slot_name: &str,
    ) -> Option<ResolvedInferenceSlot> {
        let capability_id = capability_id?;
        let profile_name = self.bound_profile_name(Some(capability_id), slot_name)?;
        let profile = self.inference.profiles.get(&profile_name);
        Some(ResolvedInferenceSlot {
            capability_id: capability_id.to_string(),
            slot_name: slot_name.to_string(),
            profile_name,
            task: profile.map(|profile| profile.task),
            driver: profile.map(|profile| profile.driver.clone()),
            runtime: profile.and_then(|profile| profile.runtime.clone()),
            model: profile.and_then(|profile| profile.model.clone()),
        })
    }

    fn resolve_profile_for_slot(
        &self,
        capability_id: Option<&str>,
        slot_name: &str,
        expected_task: InferenceTask,
    ) -> Result<(String, &InferenceProfileConfig)> {
        let profile_name = if let Some(capability_id) = capability_id {
            let Some(profile_name) = self.bound_profile_name(Some(capability_id), slot_name) else {
                bail!("capability `{capability_id}` does not bind inference slot `{slot_name}`");
            };
            profile_name
        } else if self.inference.profiles.contains_key(slot_name) {
            slot_name.to_string()
        } else {
            bail!("inference slot `{slot_name}` requires an active capability scope");
        };
        let profile = self
            .inference
            .profiles
            .get(&profile_name)
            .ok_or_else(|| anyhow!("inference profile `{profile_name}` is not defined"))?;
        if profile.task != expected_task {
            bail!(
                "inference profile `{profile_name}` is bound to slot `{slot_name}` but has task `{}` instead of `{}`",
                profile.task,
                expected_task
            );
        }
        Ok((profile_name, profile))
    }

    fn build_embedding_service(
        &self,
        profile_name: &str,
        profile: &InferenceProfileConfig,
    ) -> Result<Arc<dyn EmbeddingService>> {
        match profile.driver.as_str() {
            BITLOOPS_EMBEDDINGS_IPC_DRIVER => {
                let runtime_name = profile
                    .runtime
                    .as_deref()
                    .ok_or_else(|| anyhow!("profile `{profile_name}` requires a runtime"))?;
                let runtime = self.configured_runtime(profile_name, runtime_name)?;
                let model = profile
                    .model
                    .as_deref()
                    .ok_or_else(|| anyhow!("profile `{profile_name}` requires a model"))?;
                if profile.api_key.is_some() || profile.base_url.is_some() {
                    bail!(
                        "profile `{profile_name}` uses driver `{}` and cannot declare `api_key` or `base_url`",
                        BITLOOPS_EMBEDDINGS_IPC_DRIVER
                    );
                }

                let service = BitloopsEmbeddingsIpcService::new(
                    profile_name,
                    runtime,
                    model,
                    profile.cache_dir.as_deref(),
                    runtime_name == BITLOOPS_PLATFORM_EMBEDDINGS_RUNTIME_ID,
                )
                .with_context(|| {
                    format!(
                        "building `{BITLOOPS_EMBEDDINGS_IPC_DRIVER}` service for profile `{profile_name}`"
                    )
                })?;
                Ok(Arc::new(service))
            }
            other => bail!("unsupported embeddings driver `{other}`"),
        }
    }

    fn build_text_generation_service(
        &self,
        profile_name: &str,
        profile: &InferenceProfileConfig,
    ) -> Result<Arc<dyn TextGenerationService>> {
        let runtime_name = profile
            .runtime
            .as_deref()
            .ok_or_else(|| anyhow!("profile `{profile_name}` requires a runtime"))?;
        let runtime = self.configured_runtime(profile_name, runtime_name)?;
        let model = profile
            .model
            .as_deref()
            .ok_or_else(|| anyhow!("profile `{profile_name}` requires a model"))?;
        let request_defaults = request_defaults_for_profile(profile_name, profile)?;
        if profile.driver != BITLOOPS_PLATFORM_CHAT_DRIVER
            && profile
                .base_url
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            bail!("profile `{profile_name}` requires a base_url");
        }
        let config_path = self.resolve_runtime_config_path()?;
        let service = BitloopsInferenceTextGenerationService::new(
            profile_name,
            &profile.driver,
            runtime,
            &config_path,
            request_defaults,
        )
        .with_context(|| {
            format!("building text-generation service for profile `{profile_name}`")
        })?;
        let _ = model;
        Ok(Arc::new(service))
    }

    fn build_structured_generation_service(
        &self,
        profile_name: &str,
        profile: &InferenceProfileConfig,
    ) -> Result<Arc<dyn StructuredGenerationService>> {
        let runtime = self.structured_generation_runtime_config(profile_name, profile)?;
        let model = profile
            .model
            .as_deref()
            .ok_or_else(|| anyhow!("profile `{profile_name}` requires a model"))?;
        let request_defaults = request_defaults_for_profile(profile_name, profile)?;
        let config_path = self.resolve_runtime_config_path()?;
        let service = BitloopsInferenceTextGenerationService::new(
            profile_name,
            &profile.driver,
            &runtime,
            &config_path,
            request_defaults,
        )
        .with_context(|| {
            format!("building structured-generation service for profile `{profile_name}`")
        })?;
        let _ = model;
        Ok(Arc::new(service))
    }

    fn configured_runtime(
        &self,
        profile_name: &str,
        runtime_name: &str,
    ) -> Result<&InferenceRuntimeConfig> {
        let runtime = self
            .inference
            .runtimes
            .get(runtime_name)
            .ok_or_else(|| anyhow!("runtime `{runtime_name}` is not defined"))?;
        if runtime.command.trim().is_empty() {
            bail!(
                "runtime `{runtime_name}` for profile `{profile_name}` has no command configured"
            );
        }
        Ok(runtime)
    }

    fn bitloops_inference_launcher_runtime(
        &self,
        profile_name: &str,
    ) -> Result<&InferenceRuntimeConfig> {
        self.configured_runtime(profile_name, super::BITLOOPS_INFERENCE_RUNTIME_ID)
            .with_context(|| {
                format!(
                    "profile `{profile_name}` uses a CLI-agent structured-generation driver and requires runtime `{}` to launch `bitloops-inference`",
                    super::BITLOOPS_INFERENCE_RUNTIME_ID
                )
            })
    }

    fn validate_cli_agent_provider_runtime(
        &self,
        profile_name: &str,
        profile: &InferenceProfileConfig,
    ) -> Result<&InferenceRuntimeConfig> {
        if !is_cli_agent_structured_driver(&profile.driver) {
            bail!("profile `{profile_name}` does not use a CLI-agent structured-generation driver");
        }
        let provider_runtime_name = profile
            .runtime
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("profile `{profile_name}` requires a provider runtime"))?;
        let provider_runtime = self.configured_runtime(profile_name, provider_runtime_name)?;
        if provider_runtime.command.trim().is_empty() {
            bail!(
                "provider runtime `{provider_runtime_name}` for profile `{profile_name}` has no command configured"
            );
        }
        Ok(provider_runtime)
    }

    fn structured_generation_runtime_config(
        &self,
        profile_name: &str,
        profile: &InferenceProfileConfig,
    ) -> Result<InferenceRuntimeConfig> {
        let runtime_name = profile
            .runtime
            .as_deref()
            .ok_or_else(|| anyhow!("profile `{profile_name}` requires a runtime"))?;
        if !is_cli_agent_structured_driver(&profile.driver) {
            return Ok(self.configured_runtime(profile_name, runtime_name)?.clone());
        }

        let provider_runtime = self.validate_cli_agent_provider_runtime(profile_name, profile)?;
        let mut launcher_runtime = self
            .bitloops_inference_launcher_runtime(profile_name)?
            .clone();
        launcher_runtime.request_timeout_secs = launcher_runtime
            .request_timeout_secs
            .max(provider_runtime.request_timeout_secs);
        Ok(launcher_runtime)
    }

    fn resolve_runtime_config_path(&self) -> Result<PathBuf> {
        resolve_preferred_daemon_config_path_for_repo(&self.repo_root)
            .or_else(|_| Ok(self.repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH)))
    }
}

fn is_cli_agent_structured_driver(driver: &str) -> bool {
    matches!(driver.trim(), CODEX_EXEC_DRIVER | CLAUDE_CODE_PRINT_DRIVER)
}

fn request_defaults_for_profile(
    profile_name: &str,
    profile: &InferenceProfileConfig,
) -> Result<TextGenerationRequestDefaults> {
    let temperature_text = profile
        .temperature
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("profile `{profile_name}` requires a temperature"))?;
    let temperature = temperature_text.parse::<f32>().with_context(|| {
        format!("profile `{profile_name}` has invalid temperature `{temperature_text}`")
    })?;
    if !temperature.is_finite() {
        bail!("profile `{profile_name}` has non-finite temperature `{temperature_text}`");
    }
    let max_output_tokens = profile
        .max_output_tokens
        .filter(|value| *value > 0)
        .ok_or_else(|| anyhow!("profile `{profile_name}` requires max_output_tokens"))?;
    Ok(TextGenerationRequestDefaults {
        temperature,
        max_output_tokens,
    })
}

impl InferenceGateway for LocalInferenceGateway {
    fn embeddings(&self, slot_name: &str) -> Result<Arc<dyn EmbeddingService>> {
        self.scoped(None).embeddings(slot_name)
    }

    fn text_generation(&self, slot_name: &str) -> Result<Arc<dyn TextGenerationService>> {
        self.scoped(None).text_generation(slot_name)
    }

    fn structured_generation(
        &self,
        slot_name: &str,
    ) -> Result<Arc<dyn StructuredGenerationService>> {
        self.scoped(None).structured_generation(slot_name)
    }

    fn has_slot(&self, slot_name: &str) -> bool {
        self.inference.profiles.contains_key(slot_name)
    }
}

pub struct ScopedInferenceGateway<'a> {
    inner: &'a LocalInferenceGateway,
    capability_id: Option<&'a str>,
}

pub struct OwnedScopedInferenceGateway {
    inner: LocalInferenceGateway,
    capability_id: Option<String>,
}

impl InferenceGateway for OwnedScopedInferenceGateway {
    fn embeddings(&self, slot_name: &str) -> Result<Arc<dyn EmbeddingService>> {
        self.inner
            .scoped(self.capability_id.as_deref())
            .embeddings(slot_name)
    }

    fn text_generation(&self, slot_name: &str) -> Result<Arc<dyn TextGenerationService>> {
        self.inner
            .scoped(self.capability_id.as_deref())
            .text_generation(slot_name)
    }

    fn structured_generation(
        &self,
        slot_name: &str,
    ) -> Result<Arc<dyn StructuredGenerationService>> {
        self.inner
            .scoped(self.capability_id.as_deref())
            .structured_generation(slot_name)
    }

    fn has_slot(&self, slot_name: &str) -> bool {
        self.inner
            .scoped(self.capability_id.as_deref())
            .has_slot(slot_name)
    }

    fn describe(&self, slot_name: &str) -> Option<ResolvedInferenceSlot> {
        self.inner
            .scoped(self.capability_id.as_deref())
            .describe(slot_name)
    }
}

impl InferenceGateway for ScopedInferenceGateway<'_> {
    fn embeddings(&self, slot_name: &str) -> Result<Arc<dyn EmbeddingService>> {
        let (profile_name, profile) = self.inner.resolve_profile_for_slot(
            self.capability_id,
            slot_name,
            InferenceTask::Embeddings,
        )?;
        if let Some(service) = self
            .inner
            .embedding_cache
            .lock()
            .map_err(|_| anyhow!("embedding inference cache mutex was poisoned"))?
            .get(&profile_name)
            .cloned()
        {
            return Ok(service);
        }

        let service = self.inner.build_embedding_service(&profile_name, profile)?;
        let mut cache = self
            .inner
            .embedding_cache
            .lock()
            .map_err(|_| anyhow!("embedding inference cache mutex was poisoned"))?;
        Ok(cache
            .entry(profile_name)
            .or_insert_with(|| Arc::clone(&service))
            .clone())
    }

    fn text_generation(&self, slot_name: &str) -> Result<Arc<dyn TextGenerationService>> {
        let (profile_name, profile) = self.inner.resolve_profile_for_slot(
            self.capability_id,
            slot_name,
            InferenceTask::TextGeneration,
        )?;
        if let Some(service) = self
            .inner
            .text_generation_cache
            .lock()
            .map_err(|_| anyhow!("text-generation inference cache mutex was poisoned"))?
            .get(&profile_name)
            .cloned()
        {
            return Ok(service);
        }

        let service = self
            .inner
            .build_text_generation_service(&profile_name, profile)?;
        let mut cache = self
            .inner
            .text_generation_cache
            .lock()
            .map_err(|_| anyhow!("text-generation inference cache mutex was poisoned"))?;
        Ok(cache
            .entry(profile_name)
            .or_insert_with(|| Arc::clone(&service))
            .clone())
    }

    fn structured_generation(
        &self,
        slot_name: &str,
    ) -> Result<Arc<dyn StructuredGenerationService>> {
        let (profile_name, profile) = self.inner.resolve_profile_for_slot(
            self.capability_id,
            slot_name,
            InferenceTask::StructuredGeneration,
        )?;
        if let Some(service) = self
            .inner
            .structured_generation_cache
            .lock()
            .map_err(|_| anyhow!("structured-generation inference cache mutex was poisoned"))?
            .get(&profile_name)
            .cloned()
        {
            return Ok(service);
        }

        let service = self
            .inner
            .build_structured_generation_service(&profile_name, profile)?;
        let mut cache = self
            .inner
            .structured_generation_cache
            .lock()
            .map_err(|_| anyhow!("structured-generation inference cache mutex was poisoned"))?;
        Ok(cache
            .entry(profile_name)
            .or_insert_with(|| Arc::clone(&service))
            .clone())
    }

    fn has_slot(&self, slot_name: &str) -> bool {
        self.inner
            .bound_profile_name(self.capability_id, slot_name)
            .is_some()
    }

    fn describe(&self, slot_name: &str) -> Option<ResolvedInferenceSlot> {
        self.inner.describe_slot(self.capability_id, slot_name)
    }
}
