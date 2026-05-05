use std::path::Path;

use anyhow::Result;
use serde_json::{Map, Value};

use super::store_config_utils::{read_any_u64, resolve_configured_path};
use super::types::{
    InferenceConfig, InferenceProfileConfig, InferenceRuntimeConfig, InferenceTask,
};
use super::unified_config::UnifiedSettings;

pub(crate) fn resolve_inference_from_unified_with<F>(
    settings: &UnifiedSettings,
    config_root: &Path,
    env_lookup: F,
) -> InferenceConfig
where
    F: Fn(&str) -> Option<String>,
{
    let root = settings.inference.as_ref().and_then(Value::as_object);
    let runtimes_root = root
        .and_then(|map| map.get("runtimes"))
        .and_then(Value::as_object);
    let profiles_root = root
        .and_then(|map| map.get("profiles"))
        .and_then(Value::as_object);

    let defaults = InferenceRuntimeConfig::default();
    let mut warnings = Vec::new();
    let mut runtimes = std::collections::BTreeMap::new();
    if let Some(runtimes_root) = runtimes_root {
        for (name, value) in runtimes_root {
            let Some(runtime_root) = value.as_object() else {
                warnings.push(format!(
                    "inference.runtimes.{name} must be a table and was ignored"
                ));
                continue;
            };

            let command = resolve_runtime_string_opt(
                Some(runtime_root),
                "command",
                &env_lookup,
                &mut warnings,
                &format!("inference.runtimes.{name}.command"),
            )
            .unwrap_or_default();
            if command.trim().is_empty() {
                warnings.push(format!(
                    "inference.runtimes.{name}.command is empty; the runtime will fail if a profile references it"
                ));
            }

            runtimes.insert(
                name.to_string(),
                InferenceRuntimeConfig {
                    command,
                    args: resolve_runtime_args(
                        Some(runtime_root),
                        &env_lookup,
                        &mut warnings,
                        &format!("inference.runtimes.{name}.args"),
                    )
                    .unwrap_or_default(),
                    startup_timeout_secs: read_any_u64(runtime_root, &["startup_timeout_secs"])
                        .unwrap_or(defaults.startup_timeout_secs),
                    request_timeout_secs: read_any_u64(runtime_root, &["request_timeout_secs"])
                        .unwrap_or(defaults.request_timeout_secs),
                },
            );
        }
    }

    let mut profiles = std::collections::BTreeMap::new();
    if let Some(profiles_root) = profiles_root {
        for (name, value) in profiles_root {
            let Some(profile_root) = value.as_object() else {
                warnings.push(format!(
                    "inference.profiles.{name} must be a table and was ignored"
                ));
                continue;
            };

            let task = resolve_runtime_string_opt(
                Some(profile_root),
                "task",
                &env_lookup,
                &mut warnings,
                &format!("inference.profiles.{name}.task"),
            )
            .unwrap_or_default();
            let driver = resolve_runtime_string_opt(
                Some(profile_root),
                "driver",
                &env_lookup,
                &mut warnings,
                &format!("inference.profiles.{name}.driver"),
            )
            .unwrap_or_default();
            if task.is_empty() || driver.is_empty() {
                warnings.push(format!(
                    "inference.profiles.{name} must declare both `task` and `driver` and was ignored"
                ));
                continue;
            }
            let task = parse_inference_task(&task);

            let runtime = resolve_runtime_string_opt(
                Some(profile_root),
                "runtime",
                &env_lookup,
                &mut warnings,
                &format!("inference.profiles.{name}.runtime"),
            );
            let temperature = resolve_runtime_string_opt(
                Some(profile_root),
                "temperature",
                &env_lookup,
                &mut warnings,
                &format!("inference.profiles.{name}.temperature"),
            );
            let max_output_tokens = read_any_u64(profile_root, &["max_output_tokens"])
                .map(|value| value.min(u32::MAX as u64) as u32);
            if matches!(
                task,
                InferenceTask::TextGeneration | InferenceTask::StructuredGeneration
            ) && runtime
                .as_deref()
                .map(str::trim)
                .is_none_or(|value| value.is_empty())
            {
                warnings.push(format!(
                    "inference.profiles.{name} uses task `{task}` and should declare `runtime`"
                ));
            }
            if matches!(
                task,
                InferenceTask::TextGeneration | InferenceTask::StructuredGeneration
            ) && temperature
                .as_deref()
                .map(str::trim)
                .is_none_or(|value| value.is_empty())
            {
                warnings.push(format!(
                    "inference.profiles.{name} uses task `{task}` and should declare `temperature`"
                ));
            }
            if matches!(
                task,
                InferenceTask::TextGeneration | InferenceTask::StructuredGeneration
            ) && max_output_tokens.is_none()
            {
                warnings.push(format!(
                    "inference.profiles.{name} uses task `{task}` and should declare `max_output_tokens`"
                ));
            }

            profiles.insert(
                name.to_string(),
                InferenceProfileConfig {
                    name: name.to_string(),
                    task,
                    driver,
                    runtime,
                    model: resolve_runtime_string_opt(
                        Some(profile_root),
                        "model",
                        &env_lookup,
                        &mut warnings,
                        &format!("inference.profiles.{name}.model"),
                    ),
                    api_key: resolve_runtime_string_opt(
                        Some(profile_root),
                        "api_key",
                        &env_lookup,
                        &mut warnings,
                        &format!("inference.profiles.{name}.api_key"),
                    ),
                    base_url: resolve_runtime_string_opt(
                        Some(profile_root),
                        "base_url",
                        &env_lookup,
                        &mut warnings,
                        &format!("inference.profiles.{name}.base_url"),
                    ),
                    temperature,
                    max_output_tokens,
                    cache_dir: resolve_runtime_string_opt(
                        Some(profile_root),
                        "cache_dir",
                        &env_lookup,
                        &mut warnings,
                        &format!("inference.profiles.{name}.cache_dir"),
                    )
                    .map(|path| resolve_configured_path(&path, config_root)),
                },
            );
        }
    }

    InferenceConfig {
        runtimes,
        profiles,
        warnings,
    }
}
fn resolve_runtime_string_opt<F>(
    root: Option<&Map<String, Value>>,
    key: &str,
    env_lookup: &F,
    warnings: &mut Vec<String>,
    field_name: &str,
) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    let raw = root.and_then(|map| map.get(key)).and_then(Value::as_str)?;
    match resolve_runtime_string(raw, env_lookup) {
        Ok(Some(value)) => Some(value),
        Ok(None) => None,
        Err(err) => {
            warnings.push(format!("{field_name}: {err}"));
            None
        }
    }
}

fn resolve_runtime_args<F>(
    root: Option<&Map<String, Value>>,
    env_lookup: &F,
    warnings: &mut Vec<String>,
    field_name: &str,
) -> Option<Vec<String>>
where
    F: Fn(&str) -> Option<String>,
{
    let raw = root.and_then(|map| map.get("args"))?;
    let Some(items) = raw.as_array() else {
        warnings.push(format!("{field_name}: expected an array of strings"));
        return None;
    };
    let mut resolved = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Some(item) = item.as_str() else {
            warnings.push(format!("{field_name}[{index}]: expected a string"));
            continue;
        };
        match resolve_runtime_string(item, env_lookup) {
            Ok(Some(value)) => resolved.push(value),
            Ok(None) => {}
            Err(err) => warnings.push(format!("{field_name}[{index}]: {err}")),
        }
    }
    Some(resolved)
}

fn resolve_runtime_string<F>(raw: &str, env_lookup: &F) -> Result<Option<String>>
where
    F: Fn(&str) -> Option<String>,
{
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    if let Some(key) = trimmed
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
    {
        let Some(env_value) = env_lookup(key) else {
            anyhow::bail!("environment variable `{key}` is not set");
        };
        let env_trimmed = env_value.trim();
        if env_trimmed.is_empty() {
            anyhow::bail!("environment variable `{key}` is empty");
        }
        return Ok(Some(env_trimmed.to_string()));
    }

    Ok(Some(trimmed.to_string()))
}
fn parse_inference_task(raw: &str) -> InferenceTask {
    match raw.trim().to_ascii_lowercase().as_str() {
        "text_generation" | "text-generation" => InferenceTask::TextGeneration,
        "structured_generation" | "structured-generation" => InferenceTask::StructuredGeneration,
        _ => InferenceTask::Embeddings,
    }
}
