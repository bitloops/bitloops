use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};
use std::env;
use std::path::{Path, PathBuf};

use crate::utils::paths;

use super::constants::*;
use super::daemon_config::{
    LoadedDaemonSettings, default_daemon_config_exists, default_daemon_config_path,
    load_daemon_settings,
};
use super::repo_policy::{REPO_POLICY_LOCAL_FILE_NAME, discover_repo_policy_optional};
use super::store_config_utils::{
    current_repo_root_or_cwd, current_repo_root_or_cwd_result, normalize_blob_path,
    normalize_sqlite_path, read_any_string, read_any_u64, read_non_empty_env,
    resolve_configured_path, resolve_required_provider_string,
};
use super::types::{
    AtlassianProviderConfig, BlobStorageConfig, DEFAULT_SEMANTIC_CLONES_ANN_NEIGHBORS,
    DashboardFileConfig, EmbeddingCapabilityConfig, EmbeddingsConfig, EventsBackendConfig,
    GithubProviderConfig, InferenceCapabilityConfig, InferenceConfig, InferenceProfileConfig,
    InferenceRuntimeConfig, InferenceTask, MAX_SEMANTIC_CLONES_ANN_NEIGHBORS,
    MIN_SEMANTIC_CLONES_ANN_NEIGHBORS, ProviderConfig, RelationalBackendConfig,
    SemanticCloneEmbeddingMode, SemanticClonesConfig, SemanticSummaryMode, StoreBackendConfig,
    StoreFileConfig, WatchFileConfig, WatchRuntimeConfig,
};
use super::unified_config::{
    UnifiedSettings, resolve_dashboard_from_unified, resolve_inference_capability_from_unified,
    resolve_inference_from_unified, resolve_provider_from_unified,
    resolve_semantic_clones_from_unified, resolve_store_backend_from_unified,
    resolve_watch_from_unified,
};

fn explicit_daemon_settings_override() -> Result<Option<(PathBuf, UnifiedSettings)>> {
    let Some(explicit_path) = env::var_os(ENV_DAEMON_CONFIG_PATH_OVERRIDE) else {
        return Ok(None);
    };
    let loaded = load_daemon_settings(Some(Path::new(&explicit_path)))?;
    Ok(Some((loaded.root, loaded.settings)))
}

fn canonicalize_loaded_daemon_settings(mut loaded: LoadedDaemonSettings) -> LoadedDaemonSettings {
    if let Ok(path) = loaded.path.canonicalize() {
        loaded.root = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| loaded.root.clone());
        loaded.path = path;
    }
    loaded
}

fn load_strict_daemon_settings(path: &Path) -> Result<LoadedDaemonSettings> {
    load_daemon_settings(Some(path))
        .map(canonicalize_loaded_daemon_settings)
        .with_context(|| format!("loading Bitloops daemon config {}", path.display()))
}

fn discover_nearest_daemon_config(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|directory| directory.join(BITLOOPS_CONFIG_RELATIVE_PATH))
        .find(|candidate| candidate.is_file())
}

fn load_nearest_daemon_settings(repo_root: &Path) -> Result<Option<LoadedDaemonSettings>> {
    discover_nearest_daemon_config(repo_root)
        .map(|config_path| load_daemon_settings(Some(&config_path)))
        .transpose()
}

#[cfg(test)]
fn required_daemon_settings_for_repo(repo_root: &Path) -> Result<LoadedDaemonSettings> {
    // In tests, prefer repo-local config so concurrent tests using a process-wide
    // daemon-config override do not leak store paths across unrelated repos.
    if let Some(loaded) = load_nearest_daemon_settings(repo_root)? {
        return Ok(loaded);
    }

    if let Some(explicit_path) = env::var_os(ENV_DAEMON_CONFIG_PATH_OVERRIDE) {
        return load_daemon_settings(Some(Path::new(&explicit_path)));
    }

    bail!(
        "Bitloops daemon config is required to resolve the repo runtime store. Set `{}` to an explicit config path, add `{}` next to the repository, or create the default daemon config.",
        ENV_DAEMON_CONFIG_PATH_OVERRIDE,
        BITLOOPS_CONFIG_RELATIVE_PATH
    )
}

#[cfg(not(test))]
fn required_daemon_settings_for_repo(repo_root: &Path) -> Result<LoadedDaemonSettings> {
    if let Some(explicit_path) = env::var_os(ENV_DAEMON_CONFIG_PATH_OVERRIDE) {
        return load_daemon_settings(Some(Path::new(&explicit_path)));
    }

    if let Some(loaded) = load_nearest_daemon_settings(repo_root)? {
        return Ok(loaded);
    }

    if default_daemon_config_exists().unwrap_or(false) {
        return load_daemon_settings(None);
    }

    bail!(
        "Bitloops daemon config is required to resolve the repo runtime store. Set `{}` to an explicit config path, add `{}` next to the repository, or create the default daemon config.",
        ENV_DAEMON_CONFIG_PATH_OVERRIDE,
        BITLOOPS_CONFIG_RELATIVE_PATH
    )
}

pub fn resolve_daemon_config_root_for_repo(repo_root: &Path) -> Result<PathBuf> {
    required_daemon_settings_for_repo(repo_root).map(|loaded| loaded.root)
}

pub fn resolve_daemon_config_path_for_repo(repo_root: &Path) -> Result<PathBuf> {
    if let Some(explicit_path) = env::var_os(ENV_DAEMON_CONFIG_PATH_OVERRIDE) {
        return Ok(PathBuf::from(explicit_path));
    }

    if let Some(config_path) = discover_nearest_daemon_config(repo_root) {
        return Ok(config_path);
    }

    if default_daemon_config_exists().unwrap_or(false) {
        return default_daemon_config_path();
    }

    default_daemon_config_path()
}

#[cfg(test)]
fn daemon_settings_for_repo(repo_root: &Path) -> Result<(PathBuf, UnifiedSettings)> {
    if let Some(loaded) = load_nearest_daemon_settings(repo_root)? {
        return Ok((loaded.root, loaded.settings));
    }

    if let Some(override_settings) = explicit_daemon_settings_override()? {
        return Ok(override_settings);
    }

    Ok((repo_root.to_path_buf(), UnifiedSettings::default()))
}

fn repo_bound_daemon_settings_for_repo(repo_root: &Path) -> Result<LoadedDaemonSettings> {
    if let Some(explicit_path) = env::var_os(ENV_DAEMON_CONFIG_PATH_OVERRIDE) {
        return load_strict_daemon_settings(Path::new(&explicit_path)).with_context(|| {
            format!(
                "resolving repo-bound Bitloops daemon config from `{}`",
                ENV_DAEMON_CONFIG_PATH_OVERRIDE
            )
        });
    }

    let policy = discover_repo_policy_optional(repo_root)?;
    let Some(bound_path) = policy.daemon_config_path.as_deref() else {
        bail!(
            "Bitloops repo daemon binding is missing. Run `bitloops init` to bind this repo, or set `{}` to an explicit daemon config path.",
            ENV_DAEMON_CONFIG_PATH_OVERRIDE
        );
    };

    load_strict_daemon_settings(bound_path).with_context(|| {
        format!(
            "resolving repo-bound Bitloops daemon config from `{}`; rerun `bitloops init` to rebind this repo",
            REPO_POLICY_LOCAL_FILE_NAME
        )
    })
}

pub fn resolve_bound_daemon_config_root_for_repo(repo_root: &Path) -> Result<PathBuf> {
    repo_bound_daemon_settings_for_repo(repo_root).map(|loaded| loaded.root)
}

pub fn resolve_bound_daemon_config_path_for_repo(repo_root: &Path) -> Result<PathBuf> {
    repo_bound_daemon_settings_for_repo(repo_root).map(|loaded| loaded.path)
}

pub fn resolve_bound_store_backend_config_for_repo(repo_root: &Path) -> Result<StoreBackendConfig> {
    let loaded = repo_bound_daemon_settings_for_repo(repo_root)?;
    resolve_store_backend_from_unified(&loaded.settings, &loaded.root)
}

pub fn resolve_bound_repo_runtime_db_path_for_repo(repo_root: &Path) -> Result<PathBuf> {
    let config_root = resolve_bound_daemon_config_root_for_repo(repo_root)?;
    Ok(resolve_repo_runtime_db_path_for_config_root(&config_root))
}

#[cfg(not(test))]
fn daemon_settings_for_repo(repo_root: &Path) -> Result<(PathBuf, UnifiedSettings)> {
    if let Some(override_settings) = explicit_daemon_settings_override()? {
        return Ok(override_settings);
    }

    if let Some(loaded) = load_nearest_daemon_settings(repo_root)? {
        return Ok((loaded.root, loaded.settings));
    }

    if default_daemon_config_exists().unwrap_or(false) {
        let loaded = load_daemon_settings(None)?;
        return Ok((loaded.root, loaded.settings));
    }

    Ok((repo_root.to_path_buf(), UnifiedSettings::default()))
}

pub fn resolve_dashboard_config() -> DashboardFileConfig {
    let repo_root = current_repo_root_or_cwd();
    resolve_dashboard_config_for_repo(&repo_root)
}

pub fn resolve_dashboard_config_for_repo(repo_root: &Path) -> DashboardFileConfig {
    let (config_root, settings) = daemon_settings_for_repo(repo_root).unwrap_or_default();
    resolve_dashboard_from_unified(&settings, &config_root)
}

pub fn resolve_watch_runtime_config_for_repo(repo_root: &Path) -> WatchRuntimeConfig {
    let settings = discover_repo_policy_optional(repo_root)
        .map(|snapshot| UnifiedSettings {
            watch: Some(snapshot.watch),
            ..UnifiedSettings::default()
        })
        .unwrap_or_default();
    resolve_watch_from_unified(&settings, |key| env::var(key).ok())
}

#[cfg(test)]
pub fn resolve_store_backend_config() -> Result<StoreBackendConfig> {
    if let Some((config_root, settings)) = explicit_daemon_settings_override()? {
        return resolve_store_backend_from_unified(&settings, &config_root);
    }

    let repo_root = current_repo_root_or_cwd_result()?;
    resolve_store_backend_config_for_repo(&repo_root)
}

#[cfg(not(test))]
pub fn resolve_store_backend_config() -> Result<StoreBackendConfig> {
    let repo_root = current_repo_root_or_cwd_result()?;
    resolve_store_backend_config_for_repo(&repo_root)
}

pub fn resolve_store_backend_config_for_repo(repo_root: &Path) -> Result<StoreBackendConfig> {
    let (config_root, settings) = daemon_settings_for_repo(repo_root)?;
    resolve_store_backend_from_unified(&settings, &config_root)
}

pub fn resolve_repo_runtime_db_path_for_repo(repo_root: &Path) -> Result<PathBuf> {
    let config_root = resolve_daemon_config_root_for_repo(repo_root)?;
    Ok(resolve_repo_runtime_db_path_for_config_root(&config_root))
}

pub fn resolve_repo_runtime_db_path_for_config_root(config_root: &Path) -> PathBuf {
    config_root
        .join("stores")
        .join("runtime")
        .join("runtime.sqlite")
}

pub fn resolve_provider_config() -> Result<ProviderConfig> {
    let repo_root = current_repo_root_or_cwd_result()?;
    resolve_provider_config_for_repo(&repo_root)
}

pub fn resolve_provider_config_for_repo(repo_root: &Path) -> Result<ProviderConfig> {
    let settings = daemon_settings_for_repo(repo_root)
        .map(|(_, settings)| settings)
        .unwrap_or_default();
    resolve_provider_from_unified(&settings, |key| env::var(key).ok())
}
pub fn resolve_semantic_clones_config_for_repo(repo_root: &Path) -> SemanticClonesConfig {
    let settings = daemon_settings_for_repo(repo_root)
        .map(|(_, settings)| settings)
        .unwrap_or_default();
    resolve_semantic_clones_from_unified(&settings, |key| env::var(key).ok())
}

pub fn resolve_inference_config_for_repo(repo_root: &Path) -> InferenceConfig {
    let (config_root, settings) = daemon_settings_for_repo(repo_root).unwrap_or_default();
    resolve_inference_from_unified(&settings, &config_root, |key| env::var(key).ok())
}

pub fn resolve_embeddings_config_for_repo(repo_root: &Path) -> EmbeddingsConfig {
    resolve_inference_config_for_repo(repo_root)
}

pub fn resolve_inference_capability_config_for_repo(repo_root: &Path) -> InferenceCapabilityConfig {
    let (config_root, settings) = daemon_settings_for_repo(repo_root).unwrap_or_default();
    resolve_inference_capability_from_unified(&settings, &config_root, |key| env::var(key).ok())
}

pub fn resolve_embedding_capability_config_for_repo(repo_root: &Path) -> EmbeddingCapabilityConfig {
    resolve_inference_capability_config_for_repo(repo_root)
}

pub fn resolve_sqlite_db_path(raw_path: Option<&str>) -> Result<PathBuf> {
    let repo_root = current_repo_root_or_cwd_result()?;
    resolve_sqlite_db_path_for_repo(&repo_root, raw_path)
}

pub fn resolve_sqlite_db_path_for_repo(
    repo_root: &Path,
    raw_path: Option<&str>,
) -> Result<PathBuf> {
    match raw_path {
        Some(raw) if !raw.trim().is_empty() => normalize_sqlite_path(raw, repo_root),
        _ => Ok(paths::default_relational_db_path(repo_root)),
    }
}

pub fn resolve_duckdb_db_path_for_repo(repo_root: &Path, raw_path: Option<&str>) -> PathBuf {
    match raw_path {
        Some(raw) if !raw.trim().is_empty() => resolve_configured_path(raw, repo_root),
        _ => paths::default_events_db_path(repo_root),
    }
}

#[allow(dead_code)]
pub fn resolve_blob_local_path(raw_path: Option<&str>) -> Result<PathBuf> {
    let repo_root = current_repo_root_or_cwd_result()?;
    resolve_blob_local_path_for_repo(&repo_root, raw_path)
}

pub fn resolve_blob_local_path_for_repo(
    repo_root: &Path,
    raw_path: Option<&str>,
) -> Result<PathBuf> {
    match raw_path {
        Some(raw) if !raw.trim().is_empty() => normalize_blob_path(raw, repo_root),
        _ => Ok(paths::default_blob_store_path(repo_root)),
    }
}

/// Default relative paths for local backends (resolved against repo_root at use-time).
const DEFAULT_SQLITE_PATH: &str = ".bitloops/stores/relational/relational.db";
const DEFAULT_DUCKDB_PATH: &str = ".bitloops/stores/event/events.duckdb";
const DEFAULT_BLOB_LOCAL_PATH: &str = ".bitloops/stores/blob";

pub(crate) fn resolve_store_backend_config_with(
    file_cfg: StoreFileConfig,
) -> Result<StoreBackendConfig> {
    Ok(StoreBackendConfig {
        relational: RelationalBackendConfig {
            sqlite_path: file_cfg
                .sqlite_path
                .or_else(|| Some(DEFAULT_SQLITE_PATH.to_string())),
            postgres_dsn: file_cfg.pg_dsn,
        },
        events: EventsBackendConfig {
            duckdb_path: file_cfg
                .duckdb_path
                .or_else(|| Some(DEFAULT_DUCKDB_PATH.to_string())),
            clickhouse_url: file_cfg.clickhouse_url,
            clickhouse_user: file_cfg.clickhouse_user,
            clickhouse_password: file_cfg.clickhouse_password,
            clickhouse_database: file_cfg.clickhouse_database,
        },
        blobs: BlobStorageConfig {
            local_path: file_cfg
                .blob_local_path
                .or_else(|| Some(DEFAULT_BLOB_LOCAL_PATH.to_string())),
            s3_bucket: file_cfg.blob_s3_bucket,
            s3_region: file_cfg.blob_s3_region,
            s3_access_key_id: file_cfg.blob_s3_access_key_id,
            s3_secret_access_key: file_cfg.blob_s3_secret_access_key,
            gcs_bucket: file_cfg.blob_gcs_bucket,
            gcs_credentials_path: file_cfg.blob_gcs_credentials_path,
        },
    })
}

pub(crate) fn resolve_provider_config_from_value_with<F>(
    value: &Value,
    env_lookup: F,
) -> Result<ProviderConfig>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(root) = value.as_object() else {
        return Ok(ProviderConfig::default());
    };
    let Some(providers) = root
        .get(KNOWLEDGE_CONFIG_KEY)
        .and_then(Value::as_object)
        .and_then(|knowledge| knowledge.get(PROVIDERS_CONFIG_KEY))
        .and_then(Value::as_object)
    else {
        return Ok(ProviderConfig::default());
    };

    Ok(ProviderConfig {
        github: providers
            .get("github")
            .and_then(Value::as_object)
            .map(|github| parse_github_provider_config(github, &env_lookup))
            .transpose()?,
        atlassian: providers
            .get("atlassian")
            .and_then(Value::as_object)
            .map(|atlassian| {
                parse_atlassian_provider_config(
                    atlassian,
                    &env_lookup,
                    "knowledge.providers.atlassian",
                )
            })
            .transpose()?,
        jira: providers
            .get("jira")
            .and_then(Value::as_object)
            .map(|jira| {
                parse_atlassian_provider_config(jira, &env_lookup, "knowledge.providers.jira")
            })
            .transpose()?,
        confluence: providers
            .get("confluence")
            .and_then(Value::as_object)
            .map(|confluence| {
                parse_atlassian_provider_config(
                    confluence,
                    &env_lookup,
                    "knowledge.providers.confluence",
                )
            })
            .transpose()?,
    })
}

pub(crate) fn resolve_semantic_clones_from_unified_with<F>(
    settings: &UnifiedSettings,
    env_lookup: F,
) -> SemanticClonesConfig
where
    F: Fn(&str) -> Option<String>,
{
    let root = settings.semantic_clones.as_ref().and_then(Value::as_object);
    let summary_mode = root
        .and_then(|map| read_any_string(map, &["summary_mode"]))
        .or_else(|| read_non_empty_env(&env_lookup, "BITLOOPS_SEMANTIC_CLONES_SUMMARY_MODE"))
        .map(|value| parse_summary_mode(&value))
        .unwrap_or_default();
    let embedding_mode = root
        .and_then(|map| read_any_string(map, &["embedding_mode"]))
        .or_else(|| read_non_empty_env(&env_lookup, "BITLOOPS_SEMANTIC_CLONES_EMBEDDING_MODE"))
        .map(|value| parse_embedding_mode(&value))
        .unwrap_or_default();
    let inference_root = root
        .and_then(|map| map.get("inference"))
        .and_then(Value::as_object);
    let ann_neighbors = root
        .and_then(|map| {
            read_any_u64(map, &["ann_neighbors"])
                .map(|value| clamp_semantic_clones_ann_neighbors(value as i64))
                .or_else(|| {
                    read_any_string(map, &["ann_neighbors"])
                        .and_then(|value| value.trim().parse::<i64>().ok())
                        .map(clamp_semantic_clones_ann_neighbors)
                })
        })
        .or_else(|| {
            read_non_empty_env(&env_lookup, "BITLOOPS_SEMANTIC_CLONES_ANN_NEIGHBORS")
                .and_then(|value| value.trim().parse::<i64>().ok())
                .map(clamp_semantic_clones_ann_neighbors)
        })
        .unwrap_or(DEFAULT_SEMANTIC_CLONES_ANN_NEIGHBORS);
    let enrichment_workers = root
        .and_then(|map| read_any_u64(map, &["enrichment_workers"]))
        .or_else(|| {
            read_non_empty_env(&env_lookup, "BITLOOPS_SEMANTIC_CLONES_ENRICHMENT_WORKERS")
                .and_then(|value| value.trim().parse::<u64>().ok())
        })
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| SemanticClonesConfig::default().enrichment_workers);

    SemanticClonesConfig {
        summary_mode,
        embedding_mode,
        ann_neighbors,
        enrichment_workers,
        inference: crate::config::SemanticClonesInferenceBindings {
            summary_generation: inference_root
                .and_then(|map| read_any_string(map, &["summary_generation"]))
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .filter(|value| !matches!(value.to_ascii_lowercase().as_str(), "off" | "disabled")),
            code_embeddings: inference_root
                .and_then(|map| read_any_string(map, &["code_embeddings"]))
                .or_else(|| {
                    read_non_empty_env(&env_lookup, "BITLOOPS_SEMANTIC_CLONES_CODE_EMBEDDINGS")
                })
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            summary_embeddings: inference_root
                .and_then(|map| read_any_string(map, &["summary_embeddings"]))
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        },
    }
}

fn clamp_semantic_clones_ann_neighbors(value: i64) -> usize {
    if value < MIN_SEMANTIC_CLONES_ANN_NEIGHBORS as i64 {
        return MIN_SEMANTIC_CLONES_ANN_NEIGHBORS;
    }
    if value > MAX_SEMANTIC_CLONES_ANN_NEIGHBORS as i64 {
        return MAX_SEMANTIC_CLONES_ANN_NEIGHBORS;
    }
    value as usize
}

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
            if task == InferenceTask::TextGeneration
                && runtime
                    .as_deref()
                    .map(str::trim)
                    .is_none_or(|value| value.is_empty())
            {
                warnings.push(format!(
                    "inference.profiles.{name} uses task `text_generation` and should declare `runtime`"
                ));
            }
            if task == InferenceTask::TextGeneration
                && temperature
                    .as_deref()
                    .map(str::trim)
                    .is_none_or(|value| value.is_empty())
            {
                warnings.push(format!(
                    "inference.profiles.{name} uses task `text_generation` and should declare `temperature`"
                ));
            }
            if task == InferenceTask::TextGeneration && max_output_tokens.is_none() {
                warnings.push(format!(
                    "inference.profiles.{name} uses task `text_generation` and should declare `max_output_tokens`"
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

fn parse_github_provider_config<F>(
    map: &Map<String, Value>,
    env_lookup: &F,
) -> Result<GithubProviderConfig>
where
    F: Fn(&str) -> Option<String>,
{
    Ok(GithubProviderConfig {
        token: resolve_required_provider_string(
            map,
            "token",
            env_lookup,
            "knowledge.providers.github",
        )?,
    })
}

fn parse_atlassian_provider_config<F>(
    map: &Map<String, Value>,
    env_lookup: &F,
    section: &str,
) -> Result<AtlassianProviderConfig>
where
    F: Fn(&str) -> Option<String>,
{
    Ok(AtlassianProviderConfig {
        site_url: resolve_required_provider_string(map, "site_url", env_lookup, section)?,
        email: resolve_required_provider_string(map, "email", env_lookup, section)?,
        token: resolve_required_provider_string(map, "token", env_lookup, section)?,
    })
}

pub(crate) fn resolve_watch_runtime_config_with<F>(
    file_cfg: WatchFileConfig,
    env_lookup: F,
) -> WatchRuntimeConfig
where
    F: Fn(&str) -> Option<String>,
{
    let defaults = WatchRuntimeConfig::default();

    WatchRuntimeConfig {
        watch_debounce_ms: read_non_empty_env(&env_lookup, ENV_WATCH_DEBOUNCE_MS)
            .and_then(|value| value.parse::<u64>().ok())
            .or(file_cfg.watch_debounce_ms)
            .unwrap_or(defaults.watch_debounce_ms),
        watch_poll_fallback_ms: read_non_empty_env(&env_lookup, ENV_WATCH_POLL_FALLBACK_MS)
            .and_then(|value| value.parse::<u64>().ok())
            .or(file_cfg.watch_poll_fallback_ms)
            .unwrap_or(defaults.watch_poll_fallback_ms),
    }
}
#[cfg(test)]
pub(crate) fn resolve_store_backend_config_for_tests(
    file_cfg: StoreFileConfig,
) -> Result<StoreBackendConfig> {
    resolve_store_backend_config_with(file_cfg)
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

fn parse_summary_mode(raw: &str) -> SemanticSummaryMode {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "auto" => SemanticSummaryMode::Auto,
        "off" | "disabled" | "none" => SemanticSummaryMode::Off,
        _ => SemanticSummaryMode::Auto,
    }
}

fn parse_embedding_mode(raw: &str) -> SemanticCloneEmbeddingMode {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "semantic_aware_once" | "semantic-aware-once" => {
            SemanticCloneEmbeddingMode::SemanticAwareOnce
        }
        "off" | "disabled" | "none" => SemanticCloneEmbeddingMode::Off,
        "deterministic" => SemanticCloneEmbeddingMode::Deterministic,
        "refresh_on_upgrade" | "refresh-on-upgrade" => SemanticCloneEmbeddingMode::RefreshOnUpgrade,
        _ => SemanticCloneEmbeddingMode::SemanticAwareOnce,
    }
}

fn parse_inference_task(raw: &str) -> InferenceTask {
    match raw.trim().to_ascii_lowercase().as_str() {
        "text_generation" | "text-generation" => InferenceTask::TextGeneration,
        _ => InferenceTask::Embeddings,
    }
}

#[cfg(test)]
pub(crate) fn resolve_provider_config_for_tests(
    value: &Value,
    env: &[(&str, &str)],
) -> Result<ProviderConfig> {
    resolve_provider_config_from_value_with(value, |key| {
        env.iter().find_map(|(k, v)| {
            if *k == key {
                Some((*v).to_string())
            } else {
                None
            }
        })
    })
}

#[cfg(test)]
pub(crate) fn resolve_watch_runtime_config_for_tests(
    file_cfg: WatchFileConfig,
    env: &[(&str, &str)],
) -> WatchRuntimeConfig {
    resolve_watch_runtime_config_with(file_cfg, |key| {
        env.iter().find_map(|(k, v)| {
            if *k == key {
                Some((*v).to_string())
            } else {
                None
            }
        })
    })
}
