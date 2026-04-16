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
    BITLOOPS_EMBEDDINGS_IPC_DRIVER, BITLOOPS_LOCAL_EMBEDDINGS_RUNTIME_ID,
    BITLOOPS_PLATFORM_EMBEDDINGS_RUNTIME_ID, EmbeddingInputType, InferenceGateway,
    LocalInferenceGateway,
};

use super::managed::{
    ensure_managed_embeddings_runtime, ensure_managed_embeddings_runtime_with_progress,
    managed_runtime_command_is_eligible, managed_runtime_version_for_command,
};

const LOCAL_PULL_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EmbeddingsInstallState {
    NotConfigured,
    ConfiguredLocal {
        profile_name: String,
    },
    ConfiguredPlatform {
        profile_name: String,
    },
    ConfiguredNonLocal {
        profile_name: String,
        kind: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PulledEmbeddingProfileOutcome {
    pub(crate) profile_name: String,
    pub(crate) cache_dir: PathBuf,
    pub(crate) runtime_name: String,
    pub(crate) model_name: String,
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
    let runtime = capability
        .inference
        .profiles
        .get(&profile_name)
        .and_then(|profile| profile.runtime.clone());
    if kind.as_deref() == Some(BITLOOPS_EMBEDDINGS_IPC_DRIVER)
        && runtime.as_deref() == Some(BITLOOPS_LOCAL_EMBEDDINGS_RUNTIME_ID)
    {
        EmbeddingsInstallState::ConfiguredLocal { profile_name }
    } else if kind.as_deref() == Some(BITLOOPS_EMBEDDINGS_IPC_DRIVER)
        && runtime.as_deref() == Some(BITLOOPS_PLATFORM_EMBEDDINGS_RUNTIME_ID)
    {
        EmbeddingsInstallState::ConfiguredPlatform { profile_name }
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

#[cfg_attr(not(test), allow(dead_code))]
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
    let outcome = warm_local_profile(repo_root, &capability, profile_name, |_| Ok(()))?;
    lines.extend([
        format!("Pulled embedding profile `{profile_name}`."),
        format!("Cache directory: {}", outcome.cache_dir.display()),
        format!("Runtime: {} {}", outcome.runtime_name, outcome.model_name),
    ]);
    Ok(lines)
}

pub(crate) fn pull_profile_with_config_path_and_progress<R>(
    repo_root: &Path,
    config_path: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: &str,
    mut report: R,
) -> Result<PulledEmbeddingProfileOutcome>
where
    R: FnMut(crate::daemon::EmbeddingsBootstrapProgress) -> Result<()>,
{
    let install_needed =
        should_install_managed_runtime_for_profile(config_path, capability, profile_name)?;
    if install_needed {
        ensure_managed_embeddings_runtime_with_progress(repo_root, Some(config_path), &mut report)?;
    }

    let capability = if install_needed {
        embedding_capability_for_config_path(config_path)?
    } else {
        capability.clone()
    };
    warm_local_profile(repo_root, &capability, profile_name, report)
}

fn warm_local_profile<R>(
    repo_root: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: &str,
    mut report: R,
) -> Result<PulledEmbeddingProfileOutcome>
where
    R: FnMut(crate::daemon::EmbeddingsBootstrapProgress) -> Result<()>,
{
    let capability = capability_with_local_warmup_timeouts(capability, profile_name);
    let profile = resolve_profile(&capability, profile_name)?;
    ensure_local_profile(profile, profile_name)?;

    let cache_dir = local_profile_cache_dir(profile)?;
    if let Some(parent) = cache_dir.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating cache parent {}", parent.display()))?;
    }

    report(crate::daemon::EmbeddingsBootstrapProgress {
        phase: crate::daemon::EmbeddingsBootstrapPhase::WarmingProfile,
        message: Some(format!("Warming profile `{profile_name}`")),
        ..Default::default()
    })?;

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

    Ok(PulledEmbeddingProfileOutcome {
        profile_name: profile_name.to_string(),
        cache_dir,
        runtime_name: provider.provider_name().to_string(),
        model_name: provider.model_name().to_string(),
    })
}

fn capability_with_local_warmup_timeouts(
    capability: &EmbeddingCapabilityConfig,
    profile_name: &str,
) -> EmbeddingCapabilityConfig {
    let mut adjusted = capability.clone();
    let Some(profile) = adjusted.inference.profiles.get(profile_name) else {
        return adjusted;
    };
    if profile.driver != BITLOOPS_EMBEDDINGS_IPC_DRIVER {
        return adjusted;
    }
    let Some(runtime_name) = profile.runtime.clone() else {
        return adjusted;
    };
    let Some(runtime) = adjusted.inference.runtimes.get_mut(&runtime_name) else {
        return adjusted;
    };
    runtime.startup_timeout_secs = runtime.startup_timeout_secs.max(LOCAL_PULL_TIMEOUT_SECS);
    runtime.request_timeout_secs = runtime.request_timeout_secs.max(LOCAL_PULL_TIMEOUT_SECS);
    adjusted
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
            if let Some(runtime_name) = profile.runtime.as_deref() {
                let runtime = capability.inference.runtimes.get(runtime_name);
                lines.push(format!("Runtime: {runtime_name}"));
                if runtime_name == BITLOOPS_LOCAL_EMBEDDINGS_RUNTIME_ID {
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
                } else {
                    lines.push("Cache directory: not applicable".to_string());
                }
                if let Some(runtime) = runtime {
                    lines.push(format!("Runtime command: {}", runtime.command));
                    if runtime_name == BITLOOPS_LOCAL_EMBEDDINGS_RUNTIME_ID {
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
    if profile.runtime.as_deref() != Some(BITLOOPS_LOCAL_EMBEDDINGS_RUNTIME_ID) {
        bail!(
            "embedding profile `{profile_name}` is not configured for the local embeddings runtime"
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
        .map(|dir| dir.join("bitloops-local-embeddings"))
        .context("resolving bitloops-local-embeddings cache directory")
}

pub(crate) fn embedding_capability_for_config_path(
    config_path: &Path,
) -> Result<EmbeddingCapabilityConfig> {
    let loaded = load_daemon_settings(Some(config_path))?;
    Ok(resolve_embedding_capability_from_unified(
        &loaded.settings,
        &loaded.root,
        |key| env::var(key).ok(),
    ))
}

pub(crate) fn selected_inference_profile_name(
    capability: &EmbeddingCapabilityConfig,
) -> Option<&str> {
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
        || profile.runtime.as_deref() != Some(BITLOOPS_LOCAL_EMBEDDINGS_RUNTIME_ID)
    {
        return Ok(false);
    }

    managed_runtime_command_is_eligible(config_path)
}
