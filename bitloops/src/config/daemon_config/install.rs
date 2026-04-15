use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use toml_edit::{Array, DocumentMut, Item, Value as TomlValue};

use crate::host::inference::{
    BITLOOPS_EMBEDDINGS_IPC_DRIVER, BITLOOPS_INFERENCE_RUNTIME_ID,
    BITLOOPS_LOCAL_EMBEDDINGS_RUNTIME_ID, BITLOOPS_PLATFORM_EMBEDDINGS_RUNTIME_ID,
};
use crate::utils::platform_dirs::ensure_parent_dir;

use super::plans::{
    DaemonEmbeddingsInstallMode, DaemonEmbeddingsInstallPlan, DaemonInferenceInstallPlan,
};
use super::toml::{
    ensure_child_table, ensure_table, inference_driver_for_profile, selected_inference_profile_name,
};

pub(crate) fn prepare_daemon_embeddings_install(
    config_path: &Path,
) -> Result<DaemonEmbeddingsInstallPlan> {
    const DEFAULT_LOCAL_PROFILE: &str = "local_code";
    const DEFAULT_LOCAL_MODEL: &str = "bge-m3";

    ensure_parent_dir(config_path)?;

    let original_contents = match fs::read_to_string(config_path) {
        Ok(contents) => Some(contents),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            return Err(err).with_context(|| {
                format!("reading Bitloops daemon config {}", config_path.display())
            });
        }
    };

    let mut doc = match original_contents.as_deref() {
        Some(existing) => existing
            .parse::<DocumentMut>()
            .with_context(|| format!("parsing Bitloops daemon config {}", config_path.display()))?,
        None => DocumentMut::new(),
    };

    if let Some(profile_name) = selected_inference_profile_name(&doc) {
        let profile_driver = inference_driver_for_profile(&doc, &profile_name);
        let mode = if profile_driver.as_deref() == Some(BITLOOPS_EMBEDDINGS_IPC_DRIVER) {
            DaemonEmbeddingsInstallMode::WarmExisting
        } else {
            DaemonEmbeddingsInstallMode::SkipHosted
        };
        return Ok(DaemonEmbeddingsInstallPlan {
            config_path: config_path.to_path_buf(),
            profile_name,
            runtime_name: BITLOOPS_LOCAL_EMBEDDINGS_RUNTIME_ID.to_string(),
            profile_driver,
            mode,
            config_modified: false,
            original_contents,
            prepared_contents: None,
        });
    }

    if let Some(kind) = inference_driver_for_profile(&doc, DEFAULT_LOCAL_PROFILE)
        && kind != BITLOOPS_EMBEDDINGS_IPC_DRIVER
    {
        bail!(
            "cannot install default local embeddings because profile `{DEFAULT_LOCAL_PROFILE}` already exists with driver `{kind}`"
        );
    }

    let mut modified = false;
    {
        let semantic_clones = ensure_table(&mut doc, "semantic_clones");
        let inference = ensure_child_table(semantic_clones, "inference");
        if inference
            .get("code_embeddings")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            != Some(DEFAULT_LOCAL_PROFILE)
        {
            inference["code_embeddings"] = Item::Value(DEFAULT_LOCAL_PROFILE.into());
            modified = true;
        }
        if inference
            .get("summary_embeddings")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            != Some(DEFAULT_LOCAL_PROFILE)
        {
            inference["summary_embeddings"] = Item::Value(DEFAULT_LOCAL_PROFILE.into());
            modified = true;
        }
    }

    {
        let inference = ensure_table(&mut doc, "inference");

        let runtimes = ensure_child_table(inference, "runtimes");
        let runtime = ensure_child_table(runtimes, BITLOOPS_LOCAL_EMBEDDINGS_RUNTIME_ID);
        let current_runtime_command = runtime
            .get("command")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .map(ToOwned::to_owned);
        if runtime
            .get("command")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .is_none()
        {
            runtime["command"] = Item::Value("bitloops-local-embeddings".into());
            modified = true;
        }

        let manages_default_args = current_runtime_command.is_none()
            || matches!(
                current_runtime_command.as_deref(),
                Some("")
                    | Some("bitloops-local-embeddings")
                    | Some("bitloops-local-embeddings.exe")
            );
        let runtime_args_are_empty = runtime
            .get("args")
            .and_then(Item::as_value)
            .and_then(|value| value.as_array())
            .is_some_and(|value| value.is_empty());
        if (manages_default_args && !runtime_args_are_empty)
            || !runtime.get("args").is_some_and(Item::is_value)
        {
            runtime["args"] = Item::Value(TomlValue::Array(Array::new()));
            modified = true;
        }
        if runtime
            .get("startup_timeout_secs")
            .and_then(Item::as_value)
            .and_then(|value| value.as_integer())
            .is_none()
        {
            runtime["startup_timeout_secs"] = Item::Value(60.into());
            modified = true;
        }
        if runtime
            .get("request_timeout_secs")
            .and_then(Item::as_value)
            .and_then(|value| value.as_integer())
            .is_none()
        {
            runtime["request_timeout_secs"] = Item::Value(300.into());
            modified = true;
        }

        let profiles = ensure_child_table(inference, "profiles");
        let local_profile = ensure_child_table(profiles, DEFAULT_LOCAL_PROFILE);
        if local_profile
            .get("task")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .is_none()
        {
            local_profile["task"] = Item::Value("embeddings".into());
            modified = true;
        }
        if local_profile
            .get("driver")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .is_none()
        {
            local_profile["driver"] = Item::Value(BITLOOPS_EMBEDDINGS_IPC_DRIVER.into());
            modified = true;
        }
        if local_profile
            .get("runtime")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .is_none()
        {
            local_profile["runtime"] = Item::Value(BITLOOPS_LOCAL_EMBEDDINGS_RUNTIME_ID.into());
            modified = true;
        }
        if local_profile
            .get("model")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .is_none()
        {
            local_profile["model"] = Item::Value(DEFAULT_LOCAL_MODEL.into());
            modified = true;
        }
    }

    let prepared_contents = modified.then(|| doc.to_string());

    Ok(DaemonEmbeddingsInstallPlan {
        config_path: config_path.to_path_buf(),
        profile_name: DEFAULT_LOCAL_PROFILE.to_string(),
        runtime_name: BITLOOPS_LOCAL_EMBEDDINGS_RUNTIME_ID.to_string(),
        profile_driver: Some(BITLOOPS_EMBEDDINGS_IPC_DRIVER.to_string()),
        mode: DaemonEmbeddingsInstallMode::Bootstrap,
        config_modified: modified,
        original_contents,
        prepared_contents,
    })
}

pub(crate) fn prepare_daemon_platform_embeddings_install(
    config_path: &Path,
    gateway_url: &str,
    api_key_env: &str,
) -> Result<DaemonEmbeddingsInstallPlan> {
    const DEFAULT_PLATFORM_PROFILE: &str = "platform_code";
    const DEFAULT_PLATFORM_MODEL: &str = "bge-m3";

    ensure_parent_dir(config_path)?;

    let original_contents = match fs::read_to_string(config_path) {
        Ok(contents) => Some(contents),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            return Err(err).with_context(|| {
                format!("reading Bitloops daemon config {}", config_path.display())
            });
        }
    };

    let mut doc = match original_contents.as_deref() {
        Some(existing) => existing
            .parse::<DocumentMut>()
            .with_context(|| format!("parsing Bitloops daemon config {}", config_path.display()))?,
        None => DocumentMut::new(),
    };

    {
        let semantic_clones = ensure_table(&mut doc, "semantic_clones");
        let inference = ensure_child_table(semantic_clones, "inference");
        if inference
            .get("code_embeddings")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            != Some(DEFAULT_PLATFORM_PROFILE)
        {
            inference["code_embeddings"] = Item::Value(DEFAULT_PLATFORM_PROFILE.into());
        }
        if inference
            .get("summary_embeddings")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            != Some(DEFAULT_PLATFORM_PROFILE)
        {
            inference["summary_embeddings"] = Item::Value(DEFAULT_PLATFORM_PROFILE.into());
        }
    }

    {
        let inference = ensure_table(&mut doc, "inference");
        let runtimes = ensure_child_table(inference, "runtimes");
        let runtime = ensure_child_table(runtimes, BITLOOPS_PLATFORM_EMBEDDINGS_RUNTIME_ID);
        runtime["command"] = Item::Value("bitloops-platform-embeddings".into());

        let mut args = Array::new();
        args.push("--gateway-url");
        args.push(gateway_url);
        args.push("--api-key-env");
        args.push(api_key_env);
        runtime["args"] = Item::Value(TomlValue::Array(args));
        runtime["startup_timeout_secs"] = Item::Value(60.into());
        runtime["request_timeout_secs"] = Item::Value(300.into());

        let profiles = ensure_child_table(inference, "profiles");
        let platform_profile = ensure_child_table(profiles, DEFAULT_PLATFORM_PROFILE);
        platform_profile["task"] = Item::Value("embeddings".into());
        platform_profile["driver"] = Item::Value(BITLOOPS_EMBEDDINGS_IPC_DRIVER.into());
        platform_profile["runtime"] = Item::Value(BITLOOPS_PLATFORM_EMBEDDINGS_RUNTIME_ID.into());
        platform_profile["model"] = Item::Value(DEFAULT_PLATFORM_MODEL.into());
    }

    let prepared_contents = doc.to_string();
    let config_modified = original_contents
        .as_deref()
        .is_none_or(|existing| existing != prepared_contents);

    Ok(DaemonEmbeddingsInstallPlan {
        config_path: config_path.to_path_buf(),
        profile_name: DEFAULT_PLATFORM_PROFILE.to_string(),
        runtime_name: BITLOOPS_PLATFORM_EMBEDDINGS_RUNTIME_ID.to_string(),
        profile_driver: Some(BITLOOPS_EMBEDDINGS_IPC_DRIVER.to_string()),
        mode: DaemonEmbeddingsInstallMode::Bootstrap,
        config_modified,
        original_contents,
        prepared_contents: config_modified.then_some(prepared_contents),
    })
}

pub(crate) fn prepare_daemon_inference_install(
    config_path: &Path,
) -> Result<DaemonInferenceInstallPlan> {
    ensure_parent_dir(config_path)?;

    let original_contents = match fs::read_to_string(config_path) {
        Ok(contents) => Some(contents),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            return Err(err).with_context(|| {
                format!("reading Bitloops daemon config {}", config_path.display())
            });
        }
    };

    let mut doc = match original_contents.as_deref() {
        Some(existing) => existing
            .parse::<DocumentMut>()
            .with_context(|| format!("parsing Bitloops daemon config {}", config_path.display()))?,
        None => DocumentMut::new(),
    };

    let mut modified = false;
    let inference = ensure_table(&mut doc, "inference");
    let runtimes = ensure_child_table(inference, "runtimes");
    let runtime = ensure_child_table(runtimes, BITLOOPS_INFERENCE_RUNTIME_ID);
    let current_runtime_command = runtime
        .get("command")
        .and_then(Item::as_value)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .map(ToOwned::to_owned);

    if runtime
        .get("command")
        .and_then(Item::as_value)
        .and_then(|value| value.as_str())
        .is_none()
    {
        runtime["command"] = Item::Value("bitloops-inference".into());
        modified = true;
    }

    let manages_default_args = current_runtime_command.is_none()
        || matches!(
            current_runtime_command.as_deref(),
            Some("") | Some("bitloops-inference") | Some("bitloops-inference.exe")
        );
    let runtime_args_are_empty = runtime
        .get("args")
        .and_then(Item::as_value)
        .and_then(|value| value.as_array())
        .is_some_and(|value| value.is_empty());
    if (manages_default_args && !runtime_args_are_empty)
        || !runtime.get("args").is_some_and(Item::is_value)
    {
        runtime["args"] = Item::Value(TomlValue::Array(Array::new()));
        modified = true;
    }
    if runtime
        .get("startup_timeout_secs")
        .and_then(Item::as_value)
        .and_then(|value| value.as_integer())
        .is_none()
    {
        runtime["startup_timeout_secs"] = Item::Value(60.into());
        modified = true;
    }
    if runtime
        .get("request_timeout_secs")
        .and_then(Item::as_value)
        .and_then(|value| value.as_integer())
        .is_none()
    {
        runtime["request_timeout_secs"] = Item::Value(300.into());
        modified = true;
    }

    let prepared_contents = modified.then(|| doc.to_string());

    Ok(DaemonInferenceInstallPlan {
        config_path: config_path.to_path_buf(),
        config_modified: modified,
        original_contents,
        prepared_contents,
    })
}
