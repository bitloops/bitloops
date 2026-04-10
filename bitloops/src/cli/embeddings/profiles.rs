use anyhow::{Context, Result, bail};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(test)]
use crate::config::BITLOOPS_CONFIG_RELATIVE_PATH;
use crate::config::unified_config::resolve_embedding_capability_from_unified;
use crate::config::{
    EmbeddingCapabilityConfig, EmbeddingProfileConfig, InferenceTask, load_daemon_settings,
    resolve_daemon_config_path_for_repo,
};
use crate::host::inference::{
    BITLOOPS_EMBEDDINGS_IPC_DRIVER, BITLOOPS_EMBEDDINGS_RUNTIME_ID, EmbeddingInputType,
    InferenceGateway, LocalInferenceGateway,
};

use super::managed::{
    ensure_managed_embeddings_runtime, managed_runtime_command_is_eligible,
    managed_runtime_version_for_command,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EmbeddingsInstallState {
    NotConfigured,
    ConfiguredLocal {
        profile_name: String,
    },
    ConfiguredNonLocal {
        profile_name: String,
        kind: Option<String>,
    },
}

pub(crate) fn inspect_embeddings_install_state(repo_root: &Path) -> EmbeddingsInstallState {
    let Ok(config_path) = resolve_daemon_config_path_for_repo(repo_root) else {
        return EmbeddingsInstallState::NotConfigured;
    };
    if !config_path.is_file() {
        return EmbeddingsInstallState::NotConfigured;
    }
    let Ok(capability) = embedding_capability_for_config_path(&config_path) else {
        return EmbeddingsInstallState::NotConfigured;
    };
    let Some(profile_name) = selected_inference_profile_name(&capability).map(ToOwned::to_owned)
    else {
        return EmbeddingsInstallState::NotConfigured;
    };
    let kind = capability
        .inference
        .profiles
        .get(&profile_name)
        .map(|profile| profile.driver.clone());
    if kind.as_deref() == Some(BITLOOPS_EMBEDDINGS_IPC_DRIVER) {
        EmbeddingsInstallState::ConfiguredLocal { profile_name }
    } else {
        EmbeddingsInstallState::ConfiguredNonLocal { profile_name, kind }
    }
}

#[cfg(test)]
pub(crate) fn pull_profile(
    repo_root: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: &str,
) -> Result<Vec<String>> {
    let config_path = resolve_daemon_config_path_for_repo(repo_root)
        .unwrap_or_else(|_| repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH));
    pull_profile_with_config_path(repo_root, &config_path, capability, profile_name)
}

pub(super) fn pull_profile_with_config_path(
    repo_root: &Path,
    config_path: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: &str,
) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    let install_needed =
        should_install_managed_runtime_for_profile(config_path, capability, profile_name)?;
    if install_needed {
        lines.extend(ensure_managed_embeddings_runtime(
            repo_root,
            Some(config_path),
        )?);
    }

    let capability = if install_needed {
        embedding_capability_for_config_path(config_path)?
    } else {
        capability.clone()
    };
    let profile = resolve_profile(&capability, profile_name)?;
    ensure_local_profile(profile, profile_name)?;

    let cache_dir = local_profile_cache_dir(profile)?;
    if let Some(parent) = cache_dir.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating cache parent {}", parent.display()))?;
    }

    let gateway = LocalInferenceGateway::new(
        repo_root,
        capability.inference.clone(),
        std::collections::HashMap::new(),
    );
    let provider = gateway.embeddings(profile_name)?;
    let _ = provider
        .embed(
            "bitloops embeddings cache warmup",
            EmbeddingInputType::Document,
        )
        .context("warming local embedding cache")?;

    lines.extend([
        format!("Pulled embedding profile `{profile_name}`."),
        format!("Cache directory: {}", cache_dir.display()),
        format!(
            "Runtime: {} {}",
            provider.provider_name(),
            provider.model_name()
        ),
    ]);

    Ok(lines)
}

pub(crate) fn doctor_profile(
    _repo_root: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: Option<&str>,
) -> Result<Vec<String>> {
    let Some((profile_name, profile)) = resolve_doctor_target(capability, profile_name)? else {
        return Ok(vec![
            "Embeddings: disabled".to_string(),
            "No embedding inference profile is bound in [semantic_clones.inference].".to_string(),
        ]);
    };

    let mut lines = vec![
        format!("Profile: {profile_name}"),
        format!("Task: {}", profile.task),
        format!("Driver: {}", profile.driver),
        format!("Kind: {}", profile.driver),
    ];

    if let Some(model) = profile.model.as_deref() {
        lines.push(format!("Model: {model}"));
    }
    if let Some(base_url) = profile.base_url.as_deref() {
        lines.push(format!("Base URL: {base_url}"));
    }

    match profile.driver.as_str() {
        BITLOOPS_EMBEDDINGS_IPC_DRIVER => {
            let cache_dir = local_profile_cache_dir(profile)?;
            lines.push(format!("Cache directory: {}", cache_dir.display()));
            lines.push(format!(
                "Cache status: {}",
                if cache_dir.exists() {
                    "present"
                } else {
                    "missing"
                }
            ));
            if let Some(runtime_name) = profile.runtime.as_deref() {
                let runtime = capability.inference.runtimes.get(runtime_name);
                lines.push(format!("Runtime: {runtime_name}"));
                if let Some(runtime) = runtime {
                    lines.push(format!("Runtime command: {}", runtime.command));
                    match managed_runtime_version_for_command(&runtime.command) {
                        Ok(Some(version)) => {
                            lines.push(format!("Managed runtime version: {version}"));
                        }
                        Ok(None) => {}
                        Err(err) => {
                            lines.push(format!("Managed runtime metadata warning: {err}"));
                        }
                    }
                }
            }
        }
        "openai" | "voyage" => {
            lines.push("Cache directory: not applicable".to_string());
            lines.push("Runtime: hosted profile".to_string());
        }
        other => {
            lines.push(format!("Cache directory: unsupported for driver `{other}`"));
        }
    }

    Ok(lines)
}

pub(crate) fn clear_cache_for_profile(
    _repo_root: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: &str,
) -> Result<Vec<String>> {
    let profile = resolve_profile(capability, profile_name)?;
    ensure_local_profile(profile, profile_name)?;
    let cache_dir = local_profile_cache_dir(profile)?;

    if cache_dir.exists() {
        fs::remove_dir_all(&cache_dir)
            .with_context(|| format!("removing cache directory {}", cache_dir.display()))?;
        return Ok(vec![
            format!("Cleared cache for profile `{profile_name}`."),
            format!("Cache directory: {}", cache_dir.display()),
        ]);
    }

    Ok(vec![
        format!("Cache already empty for profile `{profile_name}`."),
        format!("Cache directory: {}", cache_dir.display()),
    ])
}

fn resolve_doctor_target<'a>(
    capability: &'a EmbeddingCapabilityConfig,
    profile_name: Option<&'a str>,
) -> Result<Option<(&'a str, &'a EmbeddingProfileConfig)>> {
    if !capability
        .inference
        .profiles
        .values()
        .any(|profile| profile.task == InferenceTask::Embeddings)
    {
        return Ok(None);
    }

    if let Some(profile_name) = profile_name {
        let profile = resolve_profile(capability, profile_name)?;
        return Ok(Some((profile_name, profile)));
    }

    if let Some(active_profile) = selected_inference_profile_name(capability) {
        let profile = resolve_profile(capability, active_profile)?;
        return Ok(Some((active_profile, profile)));
    }

    if capability
        .inference
        .profiles
        .values()
        .filter(|profile| profile.task == InferenceTask::Embeddings)
        .count()
        == 1
    {
        let (name, profile) = capability
            .inference
            .profiles
            .iter()
            .find(|(_, profile)| profile.task == InferenceTask::Embeddings)
            .expect("at least one profile exists");
        return Ok(Some((name.as_str(), profile)));
    }

    Err(anyhow::anyhow!(
        "multiple embedding profiles are configured; pass one explicitly"
    ))
}

fn resolve_profile<'a>(
    capability: &'a EmbeddingCapabilityConfig,
    profile_name: &str,
) -> Result<&'a EmbeddingProfileConfig> {
    capability
        .inference
        .profiles
        .get(profile_name)
        .ok_or_else(|| anyhow::anyhow!("embedding profile `{profile_name}` was not found"))
}

fn ensure_local_profile(profile: &EmbeddingProfileConfig, profile_name: &str) -> Result<()> {
    if profile.task != InferenceTask::Embeddings {
        bail!("embedding profile `{profile_name}` is not an embeddings profile");
    }
    if profile.driver != BITLOOPS_EMBEDDINGS_IPC_DRIVER {
        bail!(
            "embedding profile `{profile_name}` is not a `{BITLOOPS_EMBEDDINGS_IPC_DRIVER}` profile"
        );
    }
    Ok(())
}

fn local_profile_cache_dir(profile: &EmbeddingProfileConfig) -> Result<PathBuf> {
    if let Some(cache_dir) = profile.cache_dir.clone() {
        return Ok(cache_dir);
    }

    dirs::cache_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join(".cache")))
        .map(|dir| dir.join("bitloops-embeddings"))
        .context("resolving bitloops-embeddings cache directory")
}

pub(super) fn embedding_capability_for_config_path(
    config_path: &Path,
) -> Result<EmbeddingCapabilityConfig> {
    let loaded = load_daemon_settings(Some(config_path))?;
    Ok(resolve_embedding_capability_from_unified(
        &loaded.settings,
        &loaded.root,
        |key| env::var(key).ok(),
    ))
}

fn selected_inference_profile_name(capability: &EmbeddingCapabilityConfig) -> Option<&str> {
    capability
        .semantic_clones
        .inference
        .code_embeddings
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            capability
                .semantic_clones
                .inference
                .summary_embeddings
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
}

fn should_install_managed_runtime_for_profile(
    config_path: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: &str,
) -> Result<bool> {
    let profile = resolve_profile(capability, profile_name)?;
    if profile.task != InferenceTask::Embeddings
        || profile.driver != BITLOOPS_EMBEDDINGS_IPC_DRIVER
        || profile.runtime.as_deref() != Some(BITLOOPS_EMBEDDINGS_RUNTIME_ID)
    {
        return Ok(false);
    }

    managed_runtime_command_is_eligible(config_path)
}
