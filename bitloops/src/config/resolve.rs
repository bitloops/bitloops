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
pub(crate) use super::inference_resolve::resolve_inference_from_unified_with;
use super::repo_policy::{REPO_POLICY_LOCAL_FILE_NAME, discover_repo_policy_optional};
use super::store_config_utils::{
    current_repo_root_or_cwd, current_repo_root_or_cwd_result, normalize_blob_path,
    normalize_sqlite_path, read_any_string, read_any_u64, read_non_empty_env,
    resolve_configured_path, resolve_required_provider_string,
};
use super::types::{
    AtlassianProviderConfig, BlobStorageConfig, ContextGuidanceConfig,
    ContextGuidanceInferenceBindings, DEFAULT_SEMANTIC_CLONES_ANN_NEIGHBORS, DashboardFileConfig,
    EmbeddingCapabilityConfig, EmbeddingsConfig, EventsBackendConfig, GithubProviderConfig,
    InferenceCapabilityConfig, InferenceConfig, MAX_SEMANTIC_CLONES_ANN_NEIGHBORS,
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SemanticClonesWorkerSettings {
    pub summary_workers: Option<usize>,
    pub embedding_workers: Option<usize>,
    pub clone_rebuild_workers: Option<usize>,
    pub legacy_enrichment_workers: Option<usize>,
}

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

pub(crate) fn resolve_preferred_daemon_config_path_for_repo(repo_root: &Path) -> Result<PathBuf> {
    if let Some(explicit_path) = env::var_os(ENV_DAEMON_CONFIG_PATH_OVERRIDE) {
        return Ok(PathBuf::from(explicit_path));
    }

    let policy = discover_repo_policy_optional(repo_root)?;
    if let Some(bound_path) = policy.daemon_config_path {
        return Ok(bound_path);
    }

    resolve_daemon_config_path_for_repo(repo_root)
}

fn preferred_daemon_settings_for_repo(repo_root: &Path) -> Result<(PathBuf, UnifiedSettings)> {
    if let Some(override_settings) = explicit_daemon_settings_override()? {
        return Ok(override_settings);
    }

    let policy = discover_repo_policy_optional(repo_root)?;
    if let Some(bound_path) = policy.daemon_config_path.as_deref() {
        let loaded = load_strict_daemon_settings(bound_path).with_context(|| {
            format!(
                "resolving preferred Bitloops daemon config from `{}`; rerun `bitloops init` to rebind this repo",
                REPO_POLICY_LOCAL_FILE_NAME
            )
        })?;
        return Ok((loaded.root, loaded.settings));
    }

    daemon_settings_for_repo(repo_root)
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
    let settings = preferred_daemon_settings_for_repo(repo_root)
        .map(|(_, settings)| settings)
        .unwrap_or_default();
    resolve_semantic_clones_from_unified(&settings, |key| env::var(key).ok())
}

pub(crate) fn resolve_semantic_clones_worker_settings_for_repo(
    repo_root: &Path,
) -> SemanticClonesWorkerSettings {
    let settings = preferred_daemon_settings_for_repo(repo_root)
        .map(|(_, settings)| settings)
        .unwrap_or_default();
    semantic_clones_worker_settings_from_unified(&settings)
}

pub fn resolve_inference_config_for_repo(repo_root: &Path) -> InferenceConfig {
    let (config_root, settings) = preferred_daemon_settings_for_repo(repo_root).unwrap_or_default();
    resolve_inference_from_unified(&settings, &config_root, |key| env::var(key).ok())
}

pub fn resolve_embeddings_config_for_repo(repo_root: &Path) -> EmbeddingsConfig {
    resolve_inference_config_for_repo(repo_root)
}

pub fn resolve_inference_capability_config_for_repo(repo_root: &Path) -> InferenceCapabilityConfig {
    let (config_root, settings) = preferred_daemon_settings_for_repo(repo_root).unwrap_or_default();
    resolve_inference_capability_from_unified(&settings, &config_root, |key| env::var(key).ok())
}

pub fn resolve_embedding_capability_config_for_repo(repo_root: &Path) -> EmbeddingCapabilityConfig {
    resolve_inference_capability_config_for_repo(repo_root)
}

fn semantic_clones_worker_settings_from_unified(
    settings: &UnifiedSettings,
) -> SemanticClonesWorkerSettings {
    let root = settings.semantic_clones.as_ref().and_then(Value::as_object);
    SemanticClonesWorkerSettings {
        summary_workers: root
            .and_then(|map| read_any_u64(map, &["summary_workers"]))
            .and_then(|value| usize::try_from(value).ok()),
        embedding_workers: root
            .and_then(|map| read_any_u64(map, &["embedding_workers"]))
            .and_then(|value| usize::try_from(value).ok()),
        clone_rebuild_workers: root
            .and_then(|map| read_any_u64(map, &["clone_rebuild_workers"]))
            .and_then(|value| usize::try_from(value).ok()),
        legacy_enrichment_workers: root
            .and_then(|map| read_any_u64(map, &["enrichment_workers"]))
            .and_then(|value| usize::try_from(value).ok()),
    }
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

/// Default relative paths for local daemon backends.
const DEFAULT_SQLITE_PATH: &str = "stores/relational/relational.db";
const DEFAULT_DUCKDB_PATH: &str = "stores/event/events.duckdb";
const DEFAULT_BLOB_LOCAL_PATH: &str = "stores/blob";

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
    let legacy_enrichment_workers = root
        .and_then(|map| read_any_u64(map, &["enrichment_workers"]))
        .or_else(|| {
            read_non_empty_env(&env_lookup, "BITLOOPS_SEMANTIC_CLONES_ENRICHMENT_WORKERS")
                .and_then(|value| value.trim().parse::<u64>().ok())
        })
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0);
    let summary_workers = root
        .and_then(|map| read_any_u64(map, &["summary_workers"]))
        .or_else(|| {
            read_non_empty_env(&env_lookup, "BITLOOPS_SEMANTIC_CLONES_SUMMARY_WORKERS")
                .and_then(|value| value.trim().parse::<u64>().ok())
        })
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| SemanticClonesConfig::default().summary_workers);
    let embedding_workers = root
        .and_then(|map| read_any_u64(map, &["embedding_workers"]))
        .or_else(|| {
            read_non_empty_env(&env_lookup, "BITLOOPS_SEMANTIC_CLONES_EMBEDDING_WORKERS")
                .and_then(|value| value.trim().parse::<u64>().ok())
        })
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .or(legacy_enrichment_workers)
        .unwrap_or_else(|| SemanticClonesConfig::default().embedding_workers);
    let clone_rebuild_workers = root
        .and_then(|map| read_any_u64(map, &["clone_rebuild_workers"]))
        .or_else(|| {
            read_non_empty_env(
                &env_lookup,
                "BITLOOPS_SEMANTIC_CLONES_CLONE_REBUILD_WORKERS",
            )
            .and_then(|value| value.trim().parse::<u64>().ok())
        })
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| SemanticClonesConfig::default().clone_rebuild_workers);
    let enrichment_workers = legacy_enrichment_workers
        .unwrap_or_else(|| SemanticClonesConfig::default().enrichment_workers);

    SemanticClonesConfig {
        summary_mode,
        embedding_mode,
        ann_neighbors,
        summary_workers,
        embedding_workers,
        clone_rebuild_workers,
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

pub(crate) fn resolve_context_guidance_from_unified_with<F>(
    settings: &UnifiedSettings,
    env_lookup: F,
) -> ContextGuidanceConfig
where
    F: Fn(&str) -> Option<String>,
{
    let root = settings
        .context_guidance
        .as_ref()
        .and_then(Value::as_object);
    let inference_root = root
        .and_then(|map| map.get("inference"))
        .and_then(Value::as_object);

    ContextGuidanceConfig {
        inference: ContextGuidanceInferenceBindings {
            guidance_generation: inference_root
                .and_then(|map| read_any_string(map, &["guidance_generation"]))
                .or_else(|| {
                    read_non_empty_env(&env_lookup, "BITLOOPS_CONTEXT_GUIDANCE_GUIDANCE_GENERATION")
                })
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .filter(|value| !matches!(value.to_ascii_lowercase().as_str(), "off" | "disabled")),
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
