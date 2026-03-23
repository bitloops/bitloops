//! Unified multi-scope configuration for Bitloops 0.1.0.
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
    pub semantic: Option<Value>,

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
/// - `<global_dir>/.bitloops/config.json`   (scope: global)
/// - `<project_root>/.bitloops/config.json`  (scope: project)
/// - `<project_root>/.bitloops/config.local.json` (scope: project_local)
///
/// Missing files are silently skipped. Returns the merged [`UnifiedSettings`].
pub fn load_effective_config(global_dir: &Path, project_root: &Path) -> Result<UnifiedSettings> {
    let candidates: Vec<(std::path::PathBuf, ConfigScope)> = vec![
        (
            global_dir.join(".bitloops/config.json"),
            ConfigScope::Global,
        ),
        (
            project_root.join(".bitloops/config.json"),
            ConfigScope::Project,
        ),
        (
            project_root.join(".bitloops/config.local.json"),
            ConfigScope::ProjectLocal,
        ),
    ];

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

use super::types::{
    DashboardFileConfig, ProviderConfig, StoreBackendConfig, StoreEmbeddingConfig,
    StoreSemanticConfig, WatchRuntimeConfig,
};

/// Resolve store backend configuration (relational, events, blob) from merged
/// [`UnifiedSettings`]. Applies defaults and resolves paths relative to `repo_root`.
pub fn resolve_store_backend_from_unified(
    _settings: &UnifiedSettings,
    _repo_root: &Path,
) -> Result<StoreBackendConfig> {
    todo!()
}

/// Resolve semantic search configuration from merged [`UnifiedSettings`],
/// with environment variables taking precedence where documented.
pub fn resolve_semantic_from_unified<F: Fn(&str) -> Option<String>>(
    _settings: &UnifiedSettings,
    _env_lookup: F,
) -> StoreSemanticConfig {
    todo!()
}

/// Resolve embedding configuration from merged [`UnifiedSettings`],
/// with environment variables taking precedence where documented.
pub fn resolve_embedding_from_unified<F: Fn(&str) -> Option<String>>(
    _settings: &UnifiedSettings,
    _env_lookup: F,
) -> StoreEmbeddingConfig {
    todo!()
}

/// Resolve watch runtime configuration from merged [`UnifiedSettings`] (JSON only,
/// no TOML). Environment variables take precedence where documented.
pub fn resolve_watch_from_unified<F: Fn(&str) -> Option<String>>(
    _settings: &UnifiedSettings,
    _env_lookup: F,
) -> WatchRuntimeConfig {
    todo!()
}

/// Resolve knowledge provider configuration from merged [`UnifiedSettings`],
/// supporting `${ENV_VAR}` indirection in JSON values.
pub fn resolve_provider_from_unified<F: Fn(&str) -> Option<String>>(
    _settings: &UnifiedSettings,
    _env_lookup: F,
) -> Result<ProviderConfig> {
    todo!()
}

/// Resolve dashboard configuration from merged [`UnifiedSettings`].
pub fn resolve_dashboard_from_unified(_settings: &UnifiedSettings) -> DashboardFileConfig {
    todo!()
}
