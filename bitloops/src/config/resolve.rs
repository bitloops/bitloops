use anyhow::Result;
use serde_json::{Map, Value};
use std::env;
use std::path::{Path, PathBuf};

use crate::utils::paths;

use super::constants::*;
use super::store_config_utils::{
    current_repo_root_or_cwd, current_repo_root_or_cwd_result, normalize_blob_path,
    normalize_sqlite_path, parse_blob_storage_provider, parse_events_provider,
    parse_relational_provider, read_non_empty_env, resolve_configured_path,
    resolve_required_provider_string, user_home_dir,
};
use super::types::{
    AtlassianProviderConfig, BlobStorageConfig, BlobStorageProvider, EventsBackendConfig,
    EventsProvider, GithubProviderConfig, ProviderConfig, RelationalBackendConfig,
    RelationalProvider, StoreBackendConfig, StoreEmbeddingConfig, StoreFileConfig,
    StoreSemanticConfig, WatchFileConfig, WatchRuntimeConfig,
};
use super::unified_config::{
    UnifiedSettings, load_effective_config, resolve_dashboard_from_unified,
    resolve_embedding_from_unified, resolve_provider_from_unified, resolve_semantic_from_unified,
    resolve_store_backend_from_unified, resolve_watch_from_unified,
};

fn effective_settings_for_repo(repo_root: &Path) -> Result<UnifiedSettings> {
    let global_dir = user_home_dir().unwrap_or_else(|| PathBuf::from("."));
    load_effective_config(&global_dir, repo_root)
}

pub fn dashboard_use_bitloops_local() -> bool {
    let repo_root = current_repo_root_or_cwd();
    let settings = effective_settings_for_repo(&repo_root).unwrap_or_default();
    resolve_dashboard_from_unified(&settings)
        .use_bitloops_local
        .unwrap_or(false)
}

pub fn resolve_watch_runtime_config_for_repo(repo_root: &Path) -> WatchRuntimeConfig {
    let settings = effective_settings_for_repo(repo_root).unwrap_or_default();
    resolve_watch_from_unified(&settings, |key| env::var(key).ok())
}

pub fn resolve_store_backend_config() -> Result<StoreBackendConfig> {
    let repo_root = current_repo_root_or_cwd_result()?;
    resolve_store_backend_config_for_repo(&repo_root)
}

pub fn resolve_store_backend_config_for_repo(repo_root: &Path) -> Result<StoreBackendConfig> {
    let settings = effective_settings_for_repo(repo_root)?;
    resolve_store_backend_from_unified(&settings, repo_root)
}

pub fn resolve_store_semantic_config() -> StoreSemanticConfig {
    let repo_root = current_repo_root_or_cwd();
    let settings = effective_settings_for_repo(&repo_root).unwrap_or_default();
    resolve_semantic_from_unified(&settings, |key| env::var(key).ok())
}

pub fn resolve_provider_config() -> Result<ProviderConfig> {
    let repo_root = current_repo_root_or_cwd_result()?;
    resolve_provider_config_for_repo(&repo_root)
}

pub fn resolve_provider_config_for_repo(repo_root: &Path) -> Result<ProviderConfig> {
    let settings = effective_settings_for_repo(repo_root)?;
    resolve_provider_from_unified(&settings, |key| env::var(key).ok())
}

pub fn resolve_store_embedding_config() -> StoreEmbeddingConfig {
    let repo_root = current_repo_root_or_cwd();
    let settings = effective_settings_for_repo(&repo_root).unwrap_or_default();
    resolve_embedding_from_unified(&settings, |key| env::var(key).ok())
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

pub(crate) fn resolve_store_backend_config_with(
    file_cfg: StoreFileConfig,
) -> Result<StoreBackendConfig> {
    let relational_provider = if let Some(raw) = file_cfg.relational_provider {
        parse_relational_provider(&raw)?
    } else {
        RelationalProvider::Sqlite
    };

    let events_provider = if let Some(raw) = file_cfg.events_provider {
        parse_events_provider(&raw)?
    } else {
        EventsProvider::DuckDb
    };

    let blob_provider = if let Some(raw) = file_cfg.blob_provider {
        parse_blob_storage_provider(&raw)?
    } else {
        BlobStorageProvider::Local
    };

    Ok(StoreBackendConfig {
        relational: RelationalBackendConfig {
            provider: relational_provider,
            sqlite_path: file_cfg.sqlite_path,
            postgres_dsn: file_cfg.pg_dsn,
        },
        events: EventsBackendConfig {
            provider: events_provider,
            duckdb_path: file_cfg.duckdb_path,
            clickhouse_url: file_cfg.clickhouse_url,
            clickhouse_user: file_cfg.clickhouse_user,
            clickhouse_password: file_cfg.clickhouse_password,
            clickhouse_database: file_cfg.clickhouse_database,
        },
        blobs: BlobStorageConfig {
            provider: blob_provider,
            local_path: file_cfg.blob_local_path,
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
            .or(file_cfg.semantic_provider),
        semantic_model: read_non_empty_env(&env_lookup, ENV_SEMANTIC_MODEL)
            .or(file_cfg.semantic_model),
        semantic_api_key: read_non_empty_env(&env_lookup, ENV_SEMANTIC_API_KEY)
            .or(file_cfg.semantic_api_key),
        semantic_base_url: read_non_empty_env(&env_lookup, ENV_SEMANTIC_BASE_URL)
            .or(file_cfg.semantic_base_url),
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

pub(crate) fn resolve_store_embedding_config_with<F>(
    file_cfg: StoreFileConfig,
    env_lookup: F,
) -> StoreEmbeddingConfig
where
    F: Fn(&str) -> Option<String>,
{
    let embedding_model =
        read_non_empty_env(&env_lookup, ENV_EMBEDDING_MODEL).or(file_cfg.embedding_model);
    let embedding_api_key =
        read_non_empty_env(&env_lookup, ENV_EMBEDDING_API_KEY).or(file_cfg.embedding_api_key);
    let embedding_provider = read_non_empty_env(&env_lookup, ENV_EMBEDDING_PROVIDER)
        .or(file_cfg.embedding_provider)
        .or_else(|| Some(DEFAULT_EMBEDDING_PROVIDER.to_string()));

    StoreEmbeddingConfig {
        embedding_provider,
        embedding_model,
        embedding_api_key,
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

#[cfg(test)]
pub(crate) fn resolve_store_embedding_config_for_tests(
    file_cfg: StoreFileConfig,
    env: &[(&str, &str)],
) -> StoreEmbeddingConfig {
    resolve_store_embedding_config_with(file_cfg, |key| {
        env.iter().find_map(|(k, v)| {
            if *k == key {
                Some((*v).to_string())
            } else {
                None
            }
        })
    })
}
