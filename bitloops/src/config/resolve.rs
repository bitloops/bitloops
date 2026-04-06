use anyhow::{Result, bail};
use serde_json::{Map, Value};
use std::env;
use std::path::{Path, PathBuf};

use crate::utils::paths;

use super::constants::*;
#[cfg(not(test))]
use super::daemon_config::default_daemon_config_exists;
use super::daemon_config::{LoadedDaemonSettings, load_daemon_settings};
use super::repo_policy::discover_repo_policy_optional;
use super::store_config_utils::{
    current_repo_root_or_cwd, current_repo_root_or_cwd_result, normalize_blob_path,
    normalize_sqlite_path, read_any_string, read_any_u64, read_non_empty_env,
    resolve_configured_path, resolve_optional_env_indirection, resolve_required_provider_string,
};
use super::types::{
    AtlassianProviderConfig, BlobStorageConfig, DashboardFileConfig, EmbeddingCapabilityConfig,
    EmbeddingProfileConfig, EmbeddingsConfig, EmbeddingsRuntimeConfig, EventsBackendConfig,
    GithubProviderConfig, ProviderConfig, RelationalBackendConfig, SemanticCloneEmbeddingMode,
    SemanticClonesConfig, SemanticSummaryMode, StoreBackendConfig, StoreFileConfig,
    StoreSemanticConfig, WatchFileConfig, WatchRuntimeConfig,
};
use super::unified_config::{
    UnifiedSettings, resolve_dashboard_from_unified, resolve_embedding_capability_from_unified,
    resolve_embeddings_from_unified, resolve_provider_from_unified,
    resolve_semantic_clones_from_unified, resolve_semantic_from_unified,
    resolve_store_backend_from_unified, resolve_watch_from_unified,
};

fn explicit_daemon_settings_override() -> Result<Option<(PathBuf, UnifiedSettings)>> {
    let Some(explicit_path) = env::var_os(ENV_DAEMON_CONFIG_PATH_OVERRIDE) else {
        return Ok(None);
    };
    let loaded = load_daemon_settings(Some(Path::new(&explicit_path)))?;
    Ok(Some((loaded.root, loaded.settings)))
}

fn required_daemon_settings_for_repo(repo_root: &Path) -> Result<LoadedDaemonSettings> {
    if let Some(explicit_path) = env::var_os(ENV_DAEMON_CONFIG_PATH_OVERRIDE) {
        return load_daemon_settings(Some(Path::new(&explicit_path)));
    }

    let repo_toml = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    if repo_toml.is_file() {
        return load_daemon_settings(Some(&repo_toml));
    }

    #[cfg(not(test))]
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

#[cfg(test)]
fn daemon_settings_for_repo(repo_root: &Path) -> Result<(PathBuf, UnifiedSettings)> {
    if let Some(override_settings) = explicit_daemon_settings_override()? {
        return Ok(override_settings);
    }

    let repo_toml = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    if repo_toml.is_file() {
        let loaded = load_daemon_settings(Some(&repo_toml))?;
        return Ok((loaded.root, loaded.settings));
    }

    Ok((repo_root.to_path_buf(), UnifiedSettings::default()))
}

#[cfg(not(test))]
fn daemon_settings_for_repo(repo_root: &Path) -> Result<(PathBuf, UnifiedSettings)> {
    if let Some(override_settings) = explicit_daemon_settings_override()? {
        return Ok(override_settings);
    }

    let repo_toml = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    if repo_toml.is_file() {
        let loaded = load_daemon_settings(Some(&repo_toml))?;
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

pub fn resolve_store_semantic_config() -> StoreSemanticConfig {
    let repo_root = current_repo_root_or_cwd();
    resolve_store_semantic_config_for_repo(&repo_root)
}

pub fn resolve_store_semantic_config_for_repo(repo_root: &Path) -> StoreSemanticConfig {
    let settings = daemon_settings_for_repo(repo_root)
        .map(|(_, settings)| settings)
        .unwrap_or_default();
    resolve_semantic_from_unified(&settings, |key| env::var(key).ok())
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

pub fn resolve_embeddings_config_for_repo(repo_root: &Path) -> EmbeddingsConfig {
    let (config_root, settings) = daemon_settings_for_repo(repo_root).unwrap_or_default();
    resolve_embeddings_from_unified(&settings, &config_root, |key| env::var(key).ok())
}

pub fn resolve_embedding_capability_config_for_repo(repo_root: &Path) -> EmbeddingCapabilityConfig {
    let (config_root, settings) = daemon_settings_for_repo(repo_root).unwrap_or_default();
    resolve_embedding_capability_from_unified(&settings, &config_root, |key| env::var(key).ok())
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

pub(crate) fn resolve_store_semantic_config_with<F>(
    file_cfg: StoreFileConfig,
    env_lookup: F,
) -> StoreSemanticConfig
where
    F: Fn(&str) -> Option<String>,
{
    StoreSemanticConfig {
        semantic_provider: read_non_empty_env(&env_lookup, ENV_SEMANTIC_PROVIDER)
            .or_else(|| resolve_optional_env_indirection(file_cfg.semantic_provider, &env_lookup)),
        semantic_model: read_non_empty_env(&env_lookup, ENV_SEMANTIC_MODEL)
            .or_else(|| resolve_optional_env_indirection(file_cfg.semantic_model, &env_lookup)),
        semantic_api_key: read_non_empty_env(&env_lookup, ENV_SEMANTIC_API_KEY)
            .or_else(|| resolve_optional_env_indirection(file_cfg.semantic_api_key, &env_lookup)),
        semantic_base_url: read_non_empty_env(&env_lookup, ENV_SEMANTIC_BASE_URL)
            .or_else(|| resolve_optional_env_indirection(file_cfg.semantic_base_url, &env_lookup)),
    }
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
    let embedding_profile = root
        .and_then(|map| read_any_string(map, &["embedding_profile"]))
        .or_else(|| read_non_empty_env(&env_lookup, "BITLOOPS_SEMANTIC_CLONES_EMBEDDING_PROFILE"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .filter(|value| {
            !matches!(
                value.to_ascii_lowercase().as_str(),
                "none" | "disabled" | "off"
            )
        });

    SemanticClonesConfig {
        summary_mode,
        embedding_mode,
        embedding_profile,
    }
}

pub(crate) fn resolve_embeddings_from_unified_with<F>(
    settings: &UnifiedSettings,
    config_root: &Path,
    env_lookup: F,
) -> EmbeddingsConfig
where
    F: Fn(&str) -> Option<String>,
{
    let root = settings.embeddings.as_ref().and_then(Value::as_object);
    let runtime_root = root
        .and_then(|map| map.get(EMBEDDINGS_RUNTIME_CONFIG_KEY))
        .and_then(Value::as_object);
    let profiles_root = root
        .and_then(|map| map.get(EMBEDDINGS_PROFILES_CONFIG_KEY))
        .and_then(Value::as_object);

    let defaults = EmbeddingsRuntimeConfig::default();
    let mut warnings = Vec::new();
    let runtime = EmbeddingsRuntimeConfig {
        command: resolve_runtime_string_opt(
            runtime_root,
            "command",
            &env_lookup,
            &mut warnings,
            "embeddings.runtime.command",
        )
        .unwrap_or(defaults.command),
        args: resolve_runtime_args(
            runtime_root,
            &env_lookup,
            &mut warnings,
            "embeddings.runtime.args",
        )
        .unwrap_or(defaults.args),
        startup_timeout_secs: runtime_root
            .and_then(|map| read_any_u64(map, &["startup_timeout_secs"]))
            .unwrap_or(defaults.startup_timeout_secs),
        request_timeout_secs: runtime_root
            .and_then(|map| read_any_u64(map, &["request_timeout_secs"]))
            .unwrap_or(defaults.request_timeout_secs),
    };

    let mut profiles = std::collections::BTreeMap::new();
    if let Some(profiles_root) = profiles_root {
        for (name, value) in profiles_root {
            let Some(profile_root) = value.as_object() else {
                warnings.push(format!(
                    "embeddings.profiles.{name} must be a table and was ignored"
                ));
                continue;
            };

            let kind = resolve_runtime_string_opt(
                Some(profile_root),
                "kind",
                &env_lookup,
                &mut warnings,
                &format!("embeddings.profiles.{name}.kind"),
            )
            .unwrap_or_default();
            if kind.is_empty() {
                warnings.push(format!(
                    "embeddings.profiles.{name} is missing `kind` and was ignored"
                ));
                continue;
            }

            profiles.insert(
                name.to_string(),
                EmbeddingProfileConfig {
                    name: name.to_string(),
                    kind,
                    model: resolve_runtime_string_opt(
                        Some(profile_root),
                        "model",
                        &env_lookup,
                        &mut warnings,
                        &format!("embeddings.profiles.{name}.model"),
                    ),
                    api_key: resolve_runtime_string_opt(
                        Some(profile_root),
                        "api_key",
                        &env_lookup,
                        &mut warnings,
                        &format!("embeddings.profiles.{name}.api_key"),
                    ),
                    base_url: resolve_runtime_string_opt(
                        Some(profile_root),
                        "base_url",
                        &env_lookup,
                        &mut warnings,
                        &format!("embeddings.profiles.{name}.base_url"),
                    ),
                    cache_dir: resolve_runtime_string_opt(
                        Some(profile_root),
                        "cache_dir",
                        &env_lookup,
                        &mut warnings,
                        &format!("embeddings.profiles.{name}.cache_dir"),
                    )
                    .map(|path| resolve_configured_path(&path, config_root)),
                },
            );
        }
    }

    EmbeddingsConfig {
        runtime,
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

#[cfg(test)]
pub(crate) fn resolve_store_semantic_config_for_tests(
    file_cfg: StoreFileConfig,
    env: &[(&str, &str)],
) -> StoreSemanticConfig {
    resolve_store_semantic_config_with(file_cfg, |key| {
        env.iter().find_map(|(k, v)| {
            if *k == key {
                Some((*v).to_string())
            } else {
                None
            }
        })
    })
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
