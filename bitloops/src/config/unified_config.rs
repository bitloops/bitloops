//! Unified multi-scope configuration for Bitloops.
//!
//! One config schema, one merge pipeline: code defaults → global → project → project_local → env.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Which scope a configuration file belongs to. Must match the file's location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigScope {
    Global,
    Project,
    ProjectLocal,
}

impl std::fmt::Display for ConfigScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Global => write!(f, "global"),
            Self::Project => write!(f, "project"),
            Self::ProjectLocal => write!(f, "project_local"),
        }
    }
}

/// On-disk envelope wrapping every config file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigEnvelope {
    pub version: String,
    pub scope: ConfigScope,
    pub settings: UnifiedSettings,
}

/// The inner settings object shared by every scope.
///
/// All fields are `Option` so each layer only carries its deltas.
/// Subsystem blocks (`stores`, `knowledge`, …) are opaque `Value` here;
/// typed parsing happens downstream after the merge.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnifiedSettings {
    // Hooks / session behavior
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_dev: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_options: Option<HashMap<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<bool>,

    // Stores & providers
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stores: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_clones: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_guidance: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inference: Option<Value>,

    // UX / tooling
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dashboard: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch: Option<Value>,
}

/// Parse raw bytes into a validated [`ConfigEnvelope`].
///
/// Enforces strict schema (unknown keys rejected) and checks that the
/// `scope` field matches `expected_scope` (error on mismatch).
pub fn parse_config_envelope(data: &[u8], expected_scope: ConfigScope) -> Result<ConfigEnvelope> {
    let envelope: ConfigEnvelope =
        serde_json::from_slice(data).context("failed to parse config envelope")?;
    if envelope.scope != expected_scope {
        bail!(
            "scope mismatch: file declares scope \"{}\" but expected \"{}\"",
            envelope.scope,
            expected_scope,
        );
    }
    Ok(envelope)
}

/// Deep-merge two JSON values. Objects merge recursively; arrays and scalars
/// from `overlay` replace `base`. Explicit JSON `null` in overlay clears the key.
fn deep_merge_value(base: &Value, overlay: &Value) -> Value {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            let mut merged = base_map.clone();
            for (key, overlay_val) in overlay_map {
                if overlay_val.is_null() {
                    merged.remove(key);
                } else if let Some(base_val) = base_map.get(key) {
                    merged.insert(key.clone(), deep_merge_value(base_val, overlay_val));
                } else {
                    merged.insert(key.clone(), overlay_val.clone());
                }
            }
            Value::Object(merged)
        }
        // Arrays and scalars: overlay wins entirely
        _ => overlay.clone(),
    }
}

/// Deep-merge multiple layers of [`UnifiedSettings`] (lowest precedence first).
///
/// - **Objects:** deep-merge by key.
/// - **Arrays:** replace (higher layer wins entirely).
/// - **`null`:** clears the key (falls through to `None`).
pub fn merge_layers(layers: &[UnifiedSettings]) -> UnifiedSettings {
    // Convert each layer to a JSON Value so we can deep-merge uniformly,
    // then deserialize back to UnifiedSettings.
    let mut merged = Value::Object(serde_json::Map::new());

    for layer in layers {
        let layer_value = serde_json::to_value(layer).expect("UnifiedSettings must serialize");
        if let Value::Object(layer_map) = layer_value
            && let Value::Object(ref mut merged_map) = merged
        {
            for (key, val) in layer_map {
                if val.is_null() {
                    merged_map.remove(&key);
                } else if let Some(existing) = merged_map.get(&key) {
                    merged_map.insert(key, deep_merge_value(existing, &val));
                } else {
                    merged_map.insert(key, val);
                }
            }
        }
    }

    // Deserialize back; unknown keys cannot appear because we started from UnifiedSettings
    serde_json::from_value(merged).expect("merged value must deserialize to UnifiedSettings")
}

/// Deep-merge multiple raw JSON layers (lowest precedence first) into [`UnifiedSettings`].
///
/// Unlike [`merge_layers`], this operates on raw [`Value`] objects, so explicit
/// JSON `null` is preserved and correctly clears keys from lower layers.
/// Use this when layers come from parsed JSON files that may contain `null`.
pub fn merge_json_layers(layers: &[Value]) -> Result<UnifiedSettings> {
    let mut merged = Value::Object(serde_json::Map::new());

    for layer in layers {
        if let Value::Object(layer_map) = layer
            && let Value::Object(ref mut merged_map) = merged
        {
            for (key, val) in layer_map {
                if val.is_null() {
                    merged_map.remove(key);
                } else if let Some(existing) = merged_map.get(key) {
                    merged_map.insert(key.clone(), deep_merge_value(existing, val));
                } else {
                    merged_map.insert(key.clone(), val.clone());
                }
            }
        }
    }

    serde_json::from_value(merged).context("failed to deserialize merged config")
}

/// Load effective config from all file scopes and merge them.
///
/// Reads (when present):
/// - `<global_dir>/config.json`   (scope: global)
/// - `<project_root>/config.json`  (scope: project)
/// - `<project_root>/config.local.json` (scope: project_local)
///
/// Missing files are silently skipped. Returns the merged [`UnifiedSettings`].
pub fn load_effective_config(
    global_dir: Option<&Path>,
    project_root: &Path,
) -> Result<UnifiedSettings> {
    let mut candidates: Vec<(std::path::PathBuf, ConfigScope)> = Vec::with_capacity(3);
    if let Some(dir) = global_dir {
        candidates.push((dir.join("config.json"), ConfigScope::Global));
    }
    candidates.push((project_root.join("config.json"), ConfigScope::Project));
    candidates.push((
        project_root.join("config.local.json"),
        ConfigScope::ProjectLocal,
    ));

    let mut json_layers = Vec::new();
    for (path, expected_scope) in &candidates {
        if path.is_file() {
            let data = fs::read(path)
                .with_context(|| format!("failed to read config file {}", path.display()))?;
            // Validate envelope (scope check, unknown keys) but extract raw settings JSON
            // to preserve null semantics for the merge.
            let raw: Value =
                serde_json::from_slice(&data).context("failed to parse config JSON")?;
            // Validate via typed parse (strict schema + scope check)
            let _ = parse_config_envelope(&data, *expected_scope)
                .with_context(|| format!("in config file {}", path.display()))?;
            if let Some(settings) = raw.get("settings") {
                json_layers.push(settings.clone());
            }
        }
    }

    if json_layers.is_empty() {
        return Ok(UnifiedSettings::default());
    }

    merge_json_layers(&json_layers)
}

/// Deserialize a [`UnifiedSettings`] from a JSON [`Value`].
///
/// Useful for constructing layers from raw JSON (e.g. for null-handling tests).
pub fn settings_from_json(value: Value) -> Result<UnifiedSettings> {
    serde_json::from_value(value).context("failed to parse UnifiedSettings from JSON value")
}

// ---------------------------------------------------------------------------
// Consumer adapters: resolve subsystem configs from the merged unified tree.
// ---------------------------------------------------------------------------

use super::resolve::{
    resolve_architecture_from_unified_with, resolve_context_guidance_from_unified_with,
    resolve_inference_from_unified_with, resolve_provider_config_from_value_with,
    resolve_semantic_clones_from_unified_with, resolve_store_backend_config_with,
    resolve_watch_runtime_config_with,
};
use super::types::{
    ArchitectureConfig, ContextGuidanceConfig, DashboardFileConfig, InferenceCapabilityConfig,
    InferenceConfig, ProviderConfig, SemanticClonesConfig, StoreBackendConfig, WatchRuntimeConfig,
};

/// Resolve store backend configuration (relational, events, blob) from merged
/// [`UnifiedSettings`]. Applies defaults and resolves paths relative to `repo_root`.
pub fn resolve_store_backend_from_unified(
    settings: &UnifiedSettings,
    repo_root: &Path,
) -> Result<StoreBackendConfig> {
    let stores_value = settings
        .stores
        .clone()
        .unwrap_or(Value::Object(Default::default()));
    let file_cfg = super::types::StoreFileConfig::from_json_value(&stores_value);
    let mut config = resolve_store_backend_config_with(file_cfg.clone())?;

    config.relational.sqlite_path = Some(
        file_cfg
            .sqlite_path
            .as_deref()
            .map(|path| super::store_config_utils::resolve_configured_path(path, repo_root))
            .unwrap_or_else(|| crate::utils::paths::default_relational_db_path(repo_root))
            .to_string_lossy()
            .to_string(),
    );
    config.events.duckdb_path = Some(
        file_cfg
            .duckdb_path
            .as_deref()
            .map(|path| super::store_config_utils::resolve_configured_path(path, repo_root))
            .unwrap_or_else(|| crate::utils::paths::default_events_db_path(repo_root))
            .to_string_lossy()
            .to_string(),
    );
    config.blobs.local_path = Some(
        file_cfg
            .blob_local_path
            .as_deref()
            .map(|path| super::store_config_utils::resolve_configured_path(path, repo_root))
            .unwrap_or_else(|| crate::utils::paths::default_blob_store_path(repo_root))
            .to_string_lossy()
            .to_string(),
    );

    Ok(config)
}

pub fn resolve_semantic_clones_from_unified<F: Fn(&str) -> Option<String>>(
    settings: &UnifiedSettings,
    env_lookup: F,
) -> SemanticClonesConfig {
    resolve_semantic_clones_from_unified_with(settings, env_lookup)
}

pub fn resolve_context_guidance_from_unified<F: Fn(&str) -> Option<String>>(
    settings: &UnifiedSettings,
    env_lookup: F,
) -> ContextGuidanceConfig {
    resolve_context_guidance_from_unified_with(settings, env_lookup)
}

pub fn resolve_architecture_from_unified<F: Fn(&str) -> Option<String>>(
    settings: &UnifiedSettings,
    env_lookup: F,
) -> ArchitectureConfig {
    resolve_architecture_from_unified_with(settings, env_lookup)
}

pub fn resolve_inference_from_unified<F: Fn(&str) -> Option<String>>(
    settings: &UnifiedSettings,
    config_root: &Path,
    env_lookup: F,
) -> InferenceConfig {
    resolve_inference_from_unified_with(settings, config_root, env_lookup)
}

pub fn resolve_embeddings_from_unified<F: Fn(&str) -> Option<String>>(
    settings: &UnifiedSettings,
    config_root: &Path,
    env_lookup: F,
) -> InferenceConfig {
    resolve_inference_from_unified(settings, config_root, env_lookup)
}

pub fn resolve_inference_capability_from_unified<F: Fn(&str) -> Option<String>>(
    settings: &UnifiedSettings,
    config_root: &Path,
    env_lookup: F,
) -> InferenceCapabilityConfig {
    let semantic_clones = resolve_semantic_clones_from_unified_with(settings, &env_lookup);
    let context_guidance = resolve_context_guidance_from_unified_with(settings, &env_lookup);
    let architecture = resolve_architecture_from_unified_with(settings, &env_lookup);
    let inference = resolve_inference_from_unified_with(settings, config_root, env_lookup);

    InferenceCapabilityConfig {
        semantic_clones,
        context_guidance,
        architecture,
        inference,
    }
}

pub fn resolve_embedding_capability_from_unified<F: Fn(&str) -> Option<String>>(
    settings: &UnifiedSettings,
    config_root: &Path,
    env_lookup: F,
) -> InferenceCapabilityConfig {
    resolve_inference_capability_from_unified(settings, config_root, env_lookup)
}

/// Resolve watch runtime configuration from merged [`UnifiedSettings`] (JSON only,
/// no TOML). Environment variables take precedence where documented.
pub fn resolve_watch_from_unified<F: Fn(&str) -> Option<String>>(
    settings: &UnifiedSettings,
    env_lookup: F,
) -> WatchRuntimeConfig {
    // Build a synthetic value with watch at root level for WatchFileConfig::from_json_value.
    let mut map = serde_json::Map::new();
    if let Some(watch) = &settings.watch {
        map.insert("watch".into(), watch.clone());
    }
    let file_cfg = super::types::WatchFileConfig::from_json_value(&Value::Object(map));
    resolve_watch_runtime_config_with(file_cfg, env_lookup)
}

/// Resolve knowledge provider configuration from merged [`UnifiedSettings`],
/// supporting `${ENV_VAR}` indirection in JSON values.
pub fn resolve_provider_from_unified<F: Fn(&str) -> Option<String>>(
    settings: &UnifiedSettings,
    env_lookup: F,
) -> Result<ProviderConfig> {
    // Build a synthetic value with knowledge at root level for the provider resolver.
    let mut map = serde_json::Map::new();
    if let Some(knowledge) = &settings.knowledge {
        map.insert("knowledge".into(), knowledge.clone());
    }
    resolve_provider_config_from_value_with(&Value::Object(map), env_lookup)
}

/// Resolve dashboard configuration from merged [`UnifiedSettings`].
pub fn resolve_dashboard_from_unified(
    settings: &UnifiedSettings,
    config_root: &Path,
) -> DashboardFileConfig {
    let mut map = serde_json::Map::new();
    if let Some(dashboard) = &settings.dashboard {
        map.insert("dashboard".into(), dashboard.clone());
    }
    let mut config = DashboardFileConfig::from_json_value(&Value::Object(map));
    config.bundle_dir = config.bundle_dir.as_deref().map(|path| {
        super::store_config_utils::resolve_configured_path(&path.to_string_lossy(), config_root)
    });
    config
}
