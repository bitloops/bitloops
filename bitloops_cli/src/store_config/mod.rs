//! Shared backend config parsing and path resolution.
//! Used by both the CLI and the dashboard server so supported keys and defaults stay in sync.

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::engine::paths;

pub const BITLOOPS_CONFIG_RELATIVE_PATH: &str = ".bitloops/config.json";
pub const BITLOOPS_CONFIG_TOML_RELATIVE_PATH: &str = ".bitloops/config.toml";

const STORES_CONFIG_KEY: &str = "stores";
const RELATIONAL_CONFIG_KEY: &str = "relational";
const EVENT_CONFIG_KEY: &str = "event";
const EVENTS_CONFIG_KEY: &str = "events";
const BLOB_CONFIG_KEY: &str = "blob";
const BLOBS_CONFIG_KEY: &str = "blobs";
const SEMANTIC_CONFIG_KEY: &str = "semantic";
const DASHBOARD_CONFIG_KEY: &str = "dashboard";
const DASHBOARD_USE_BITLOOPS_LOCAL_KEY: &str = "use_bitloops_local";
const WATCH_CONFIG_KEY: &str = "watch";
const DEVQL_CONFIG_KEY: &str = "devql";
const WATCH_DEBOUNCE_MS_KEY: &str = "watch_debounce_ms";
const WATCH_POLL_FALLBACK_MS_KEY: &str = "watch_poll_fallback_ms";

const ENV_SEMANTIC_PROVIDER: &str = "BITLOOPS_DEVQL_SEMANTIC_PROVIDER";
const ENV_SEMANTIC_MODEL: &str = "BITLOOPS_DEVQL_SEMANTIC_MODEL";
const ENV_SEMANTIC_API_KEY: &str = "BITLOOPS_DEVQL_SEMANTIC_API_KEY";
const ENV_SEMANTIC_BASE_URL: &str = "BITLOOPS_DEVQL_SEMANTIC_BASE_URL";
const ENV_WATCH_DEBOUNCE_MS: &str = "BITLOOPS_DEVQL_WATCH_DEBOUNCE_MS";
const ENV_WATCH_POLL_FALLBACK_MS: &str = "BITLOOPS_DEVQL_WATCH_POLL_FALLBACK_MS";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationalProvider {
    Sqlite,
    Postgres,
}

impl RelationalProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventsProvider {
    DuckDb,
    ClickHouse,
}

impl EventsProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DuckDb => "duckdb",
            Self::ClickHouse => "clickhouse",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobStorageProvider {
    Local,
    S3,
    Gcs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreBackendConfig {
    pub relational: RelationalBackendConfig,
    pub events: EventsBackendConfig,
    pub blobs: BlobStorageConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreSemanticConfig {
    pub semantic_provider: Option<String>,
    pub semantic_model: Option<String>,
    pub semantic_api_key: Option<String>,
    pub semantic_base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationalBackendConfig {
    pub provider: RelationalProvider,
    pub sqlite_path: Option<String>,
    pub postgres_dsn: Option<String>,
}

impl RelationalBackendConfig {
    pub fn resolve_sqlite_db_path(&self) -> Result<PathBuf> {
        resolve_sqlite_db_path(self.sqlite_path.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventsBackendConfig {
    pub provider: EventsProvider,
    pub duckdb_path: Option<String>,
    pub clickhouse_url: Option<String>,
    pub clickhouse_user: Option<String>,
    pub clickhouse_password: Option<String>,
    pub clickhouse_database: Option<String>,
}

impl EventsBackendConfig {
    pub fn duckdb_path_or_default(&self) -> PathBuf {
        let repo_root = current_repo_root_or_cwd();
        resolve_duckdb_db_path_for_repo(&repo_root, self.duckdb_path.as_deref())
    }

    pub fn clickhouse_endpoint(&self) -> String {
        let base = self
            .clickhouse_url
            .clone()
            .unwrap_or_else(|| "http://localhost:8123".to_string());
        let database = self
            .clickhouse_database
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let base = base.trim_end_matches('/');
        format!("{base}/?database={database}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobStorageConfig {
    pub provider: BlobStorageProvider,
    pub local_path: Option<String>,
    pub s3_bucket: Option<String>,
    pub s3_region: Option<String>,
    pub s3_access_key_id: Option<String>,
    pub s3_secret_access_key: Option<String>,
    pub gcs_bucket: Option<String>,
    pub gcs_credentials_path: Option<String>,
}

impl BlobStorageConfig {
    #[allow(dead_code)]
    pub fn local_path_or_default(&self) -> Result<PathBuf> {
        resolve_blob_local_path(self.local_path.as_deref())
    }
}

#[derive(Debug, Clone, Default)]
pub struct StoreFileConfig {
    pub(crate) relational_provider: Option<String>,
    pub(crate) sqlite_path: Option<String>,
    pub(crate) pg_dsn: Option<String>,
    pub(crate) events_provider: Option<String>,
    pub(crate) duckdb_path: Option<String>,
    pub(crate) clickhouse_url: Option<String>,
    pub(crate) clickhouse_user: Option<String>,
    pub(crate) clickhouse_password: Option<String>,
    pub(crate) clickhouse_database: Option<String>,
    pub(crate) semantic_provider: Option<String>,
    pub(crate) semantic_model: Option<String>,
    pub(crate) semantic_api_key: Option<String>,
    pub(crate) semantic_base_url: Option<String>,
    pub(crate) blob_provider: Option<String>,
    pub(crate) blob_local_path: Option<String>,
    pub(crate) blob_s3_bucket: Option<String>,
    pub(crate) blob_s3_region: Option<String>,
    pub(crate) blob_s3_access_key_id: Option<String>,
    pub(crate) blob_s3_secret_access_key: Option<String>,
    pub(crate) blob_gcs_bucket: Option<String>,
    pub(crate) blob_gcs_credentials_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashboardFileConfig {
    pub use_bitloops_local: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchFileConfig {
    pub watch_debounce_ms: Option<u64>,
    pub watch_poll_fallback_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchRuntimeConfig {
    pub watch_debounce_ms: u64,
    pub watch_poll_fallback_ms: u64,
}

impl Default for WatchRuntimeConfig {
    fn default() -> Self {
        Self {
            watch_debounce_ms: 500,
            watch_poll_fallback_ms: 2_000,
        }
    }
}

impl StoreFileConfig {
    /// Load config from `<repo>/.bitloops/config.json`.
    /// Returns default if the file is missing or invalid.
    pub fn load() -> Self {
        let repo_root = current_repo_root_or_cwd();
        Self::load_for_repo(&repo_root)
    }

    /// Load config from `<repo_root>/.bitloops/config.json`.
    pub fn load_for_repo(repo_root: &Path) -> Self {
        let path = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);

        let data = match fs::read(&path) {
            Ok(data) => data,
            Err(_) => return Self::default(),
        };
        let value: Value = match serde_json::from_slice(&data) {
            Ok(value) => value,
            Err(_) => return Self::default(),
        };
        Self::from_json_value(&value)
    }

    /// Parse config from a JSON value (e.g. from file or tests).
    /// Reads the `stores` object when present, otherwise falls back to root.
    pub fn from_json_value(value: &Value) -> Self {
        let root_opt = value
            .get(STORES_CONFIG_KEY)
            .and_then(Value::as_object)
            .or_else(|| value.as_object());
        let Some(root) = root_opt else {
            return Self::default();
        };

        let relational = root.get(RELATIONAL_CONFIG_KEY).and_then(Value::as_object);
        let events = root
            .get(EVENT_CONFIG_KEY)
            .or_else(|| root.get(EVENTS_CONFIG_KEY))
            .and_then(Value::as_object);
        let blobs = root
            .get(BLOB_CONFIG_KEY)
            .or_else(|| root.get(BLOBS_CONFIG_KEY))
            .and_then(Value::as_object);
        let semantic = root.get(SEMANTIC_CONFIG_KEY).and_then(Value::as_object);

        Self {
            relational_provider: read_any_string_opt(relational, &["provider"])
                .or_else(|| read_any_string(root, &["relational_provider"])),
            sqlite_path: read_any_string_opt(relational, &["sqlite_path", "path"])
                .or_else(|| read_any_string(root, &["sqlite_path"])),
            pg_dsn: read_any_string_opt(relational, &["postgres_dsn", "pg_dsn"])
                .or_else(|| read_any_string(root, &["postgres_dsn", "pg_dsn"])),
            events_provider: read_any_string_opt(events, &["provider"])
                .or_else(|| read_any_string(root, &["events_provider", "event_provider"])),
            duckdb_path: read_any_string_opt(events, &["duckdb_path", "path"])
                .or_else(|| read_any_string(root, &["duckdb_path"])),
            clickhouse_url: read_any_string_opt(events, &["clickhouse_url"])
                .or_else(|| read_any_string(root, &["clickhouse_url"])),
            clickhouse_user: read_any_string_opt(events, &["clickhouse_user"])
                .or_else(|| read_any_string(root, &["clickhouse_user"])),
            clickhouse_password: read_any_string_opt(events, &["clickhouse_password"])
                .or_else(|| read_any_string(root, &["clickhouse_password"])),
            clickhouse_database: read_any_string_opt(events, &["clickhouse_database"])
                .or_else(|| read_any_string(root, &["clickhouse_database"])),
            semantic_provider: read_any_string_opt(semantic, &["provider", "semantic_provider"])
                .or_else(|| read_any_string(root, &["semantic_provider"])),
            semantic_model: read_any_string_opt(semantic, &["model", "semantic_model"])
                .or_else(|| read_any_string(root, &["semantic_model"])),
            semantic_api_key: read_any_string_opt(semantic, &["api_key", "semantic_api_key"])
                .or_else(|| read_any_string(root, &["semantic_api_key"])),
            semantic_base_url: read_any_string_opt(semantic, &["base_url", "semantic_base_url"])
                .or_else(|| read_any_string(root, &["semantic_base_url"])),
            blob_provider: read_any_string_opt(blobs, &["provider"])
                .or_else(|| read_any_string(root, &["blob_provider"])),
            blob_local_path: read_any_string_opt(blobs, &["local_path"])
                .or_else(|| read_any_string(root, &["blob_local_path"])),
            blob_s3_bucket: read_any_string_opt(blobs, &["s3_bucket"])
                .or_else(|| read_any_string(root, &["blob_s3_bucket"])),
            blob_s3_region: read_any_string_opt(blobs, &["s3_region"])
                .or_else(|| read_any_string(root, &["blob_s3_region"])),
            blob_s3_access_key_id: read_any_string_opt(blobs, &["s3_access_key_id"])
                .or_else(|| read_any_string(root, &["blob_s3_access_key_id"])),
            blob_s3_secret_access_key: read_any_string_opt(blobs, &["s3_secret_access_key"])
                .or_else(|| read_any_string(root, &["blob_s3_secret_access_key"])),
            blob_gcs_bucket: read_any_string_opt(blobs, &["gcs_bucket"])
                .or_else(|| read_any_string(root, &["blob_gcs_bucket"])),
            blob_gcs_credentials_path: read_any_string_opt(blobs, &["gcs_credentials_path"])
                .or_else(|| read_any_string(root, &["blob_gcs_credentials_path"])),
        }
    }
}

impl DashboardFileConfig {
    /// Load dashboard config from `<repo>/.bitloops/config.json`.
    /// Returns default if the file is missing or invalid.
    pub fn load() -> Self {
        let repo_root = current_repo_root_or_cwd();
        let path = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);

        let data = match fs::read(&path) {
            Ok(data) => data,
            Err(_) => return Self::default(),
        };
        let value: Value = match serde_json::from_slice(&data) {
            Ok(value) => value,
            Err(_) => return Self::default(),
        };
        Self::from_json_value(&value)
    }

    /// Parse dashboard config from a JSON value.
    /// Reads the nested `dashboard` object.
    pub fn from_json_value(value: &Value) -> Self {
        let Some(root) = value.get(DASHBOARD_CONFIG_KEY).and_then(Value::as_object) else {
            return Self::default();
        };

        Self {
            use_bitloops_local: read_any_bool(root, &[DASHBOARD_USE_BITLOOPS_LOCAL_KEY]),
        }
    }
}

impl WatchFileConfig {
    pub fn load_for_repo(repo_root: &Path) -> Self {
        let mut merged = Self::default();

        let json_path = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
        if let Ok(data) = fs::read(&json_path)
            && let Ok(value) = serde_json::from_slice::<Value>(&data)
        {
            merged = Self::from_json_value(&value);
        }

        let toml_path = repo_root.join(BITLOOPS_CONFIG_TOML_RELATIVE_PATH);
        if let Ok(data) = fs::read_to_string(&toml_path) {
            merged.merge(Self::from_toml_str(&data));
        }

        merged
    }

    pub fn from_json_value(value: &Value) -> Self {
        let root = value.as_object();
        let watch = root.and_then(|map| map.get(WATCH_CONFIG_KEY)).and_then(Value::as_object);
        let devql = root.and_then(|map| map.get(DEVQL_CONFIG_KEY)).and_then(Value::as_object);
        let devql_watch =
            devql.and_then(|map| map.get(WATCH_CONFIG_KEY)).and_then(Value::as_object);

        Self {
            watch_debounce_ms: read_any_u64_opt(watch, &[WATCH_DEBOUNCE_MS_KEY])
                .or_else(|| read_any_u64_opt(devql_watch, &[WATCH_DEBOUNCE_MS_KEY]))
                .or_else(|| read_any_u64_opt(devql, &[WATCH_DEBOUNCE_MS_KEY]))
                .or_else(|| root.and_then(|map| read_any_u64(map, &[WATCH_DEBOUNCE_MS_KEY]))),
            watch_poll_fallback_ms: read_any_u64_opt(watch, &[WATCH_POLL_FALLBACK_MS_KEY])
                .or_else(|| read_any_u64_opt(devql_watch, &[WATCH_POLL_FALLBACK_MS_KEY]))
                .or_else(|| read_any_u64_opt(devql, &[WATCH_POLL_FALLBACK_MS_KEY]))
                .or_else(|| {
                    root.and_then(|map| read_any_u64(map, &[WATCH_POLL_FALLBACK_MS_KEY]))
                }),
        }
    }

    pub fn from_toml_str(input: &str) -> Self {
        let mut cfg = Self::default();
        let mut section: Vec<String> = Vec::new();

        for raw_line in input.lines() {
            let line = raw_line
                .split('#')
                .next()
                .unwrap_or_default()
                .trim();
            if line.is_empty() {
                continue;
            }

            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1]
                    .split('.')
                    .map(|part| part.trim().trim_matches('"').to_string())
                    .filter(|part| !part.is_empty())
                    .collect();
                continue;
            }

            let Some((raw_key, raw_value)) = line.split_once('=') else {
                continue;
            };

            let key = raw_key.trim().trim_matches('"');
            let value = raw_value.trim().trim_matches('"').trim_matches('\'');
            let in_watch_scope = section.is_empty()
                || section.as_slice() == [WATCH_CONFIG_KEY]
                || section.as_slice() == [DEVQL_CONFIG_KEY]
                || section.as_slice() == [DEVQL_CONFIG_KEY, WATCH_CONFIG_KEY];
            if !in_watch_scope {
                continue;
            }

            match key {
                WATCH_DEBOUNCE_MS_KEY => cfg.watch_debounce_ms = value.parse::<u64>().ok(),
                WATCH_POLL_FALLBACK_MS_KEY => {
                    cfg.watch_poll_fallback_ms = value.parse::<u64>().ok()
                }
                _ => {}
            }
        }

        cfg
    }

    fn merge(&mut self, other: Self) {
        if other.watch_debounce_ms.is_some() {
            self.watch_debounce_ms = other.watch_debounce_ms;
        }
        if other.watch_poll_fallback_ms.is_some() {
            self.watch_poll_fallback_ms = other.watch_poll_fallback_ms;
        }
    }
}

pub fn dashboard_use_bitloops_local() -> bool {
    DashboardFileConfig::load()
        .use_bitloops_local
        .unwrap_or(false)
}

pub fn resolve_watch_runtime_config_for_repo(repo_root: &Path) -> WatchRuntimeConfig {
    let file_cfg = WatchFileConfig::load_for_repo(repo_root);
    resolve_watch_runtime_config_with(file_cfg, |key| env::var(key).ok())
}

pub fn resolve_store_backend_config() -> Result<StoreBackendConfig> {
    let repo_root = current_repo_root_or_cwd_result()?;
    resolve_store_backend_config_for_repo(&repo_root)
}

pub fn resolve_store_backend_config_for_repo(repo_root: &Path) -> Result<StoreBackendConfig> {
    let file_cfg = StoreFileConfig::load_for_repo(repo_root);
    resolve_store_backend_config_with(file_cfg)
}

pub fn resolve_store_semantic_config() -> StoreSemanticConfig {
    let file_cfg = StoreFileConfig::load();
    resolve_store_semantic_config_with(file_cfg, |key| env::var(key).ok())
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

fn resolve_store_backend_config_with(file_cfg: StoreFileConfig) -> Result<StoreBackendConfig> {
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

fn resolve_store_semantic_config_with<F>(
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

fn resolve_watch_runtime_config_with<F>(file_cfg: WatchFileConfig, env_lookup: F) -> WatchRuntimeConfig
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

fn current_repo_root_or_cwd_result() -> Result<PathBuf> {
    paths::repo_root()
        .or_else(|_| env::current_dir().context("resolving current directory for repo config"))
}

fn current_repo_root_or_cwd() -> PathBuf {
    current_repo_root_or_cwd_result().unwrap_or_else(|_| PathBuf::from("."))
}

fn user_home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn normalize_sqlite_path(raw_path: &str, repo_root: &Path) -> Result<PathBuf> {
    normalize_repo_scoped_path(
        raw_path,
        repo_root,
        "sqlite path is empty; set `stores.relational.sqlite_path`",
    )
}

fn normalize_blob_path(raw_path: &str, repo_root: &Path) -> Result<PathBuf> {
    normalize_repo_scoped_path(
        raw_path,
        repo_root,
        "blob local path is empty; set `stores.blob.local_path`",
    )
}

fn normalize_repo_scoped_path(
    raw_path: &str,
    repo_root: &Path,
    empty_err: &str,
) -> Result<PathBuf> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        bail!("{empty_err}");
    }

    let expanded = expand_home_prefix(trimmed)?;
    let candidate = Path::new(&expanded).to_path_buf();
    if candidate.is_absolute() {
        Ok(candidate)
    } else {
        Ok(repo_root.join(candidate))
    }
}

fn resolve_configured_path(raw_path: &str, repo_root: &Path) -> PathBuf {
    let expanded = expand_tilde_path(raw_path);
    if expanded.is_absolute() {
        expanded
    } else {
        repo_root.join(expanded)
    }
}

fn expand_home_prefix(path: &str) -> Result<String> {
    let home = user_home_dir();
    expand_home_prefix_with(path, home.as_deref())
}

fn expand_home_prefix_with(path: &str, home: Option<&Path>) -> Result<String> {
    if path == "~" {
        let Some(home) = home else {
            bail!("unable to resolve home directory for `~` path");
        };
        return Ok(home.to_string_lossy().to_string());
    }

    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        let Some(home) = home else {
            bail!("unable to resolve home directory for `~` path");
        };
        return Ok(home.join(rest).to_string_lossy().to_string());
    }

    Ok(path.to_string())
}

fn read_any_string(root: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = root.get(*key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn read_any_string_opt(root: Option<&Map<String, Value>>, keys: &[&str]) -> Option<String> {
    root.and_then(|map| read_any_string(map, keys))
}

fn read_any_bool(root: &Map<String, Value>, keys: &[&str]) -> Option<bool> {
    for key in keys {
        let Some(value) = root.get(*key) else {
            continue;
        };
        if let Some(boolean) = value.as_bool() {
            return Some(boolean);
        }
        if let Some(raw) = value.as_str() {
            match raw.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => return Some(true),
                "false" | "0" | "no" | "off" => return Some(false),
                _ => {}
            }
        }
    }
    None
}

fn read_any_u64(root: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    for key in keys {
        let Some(value) = root.get(*key) else {
            continue;
        };

        if let Some(number) = value.as_u64() {
            return Some(number);
        }
        if let Some(number) = value.as_i64().filter(|number| *number >= 0) {
            return Some(number as u64);
        }
        if let Some(raw) = value.as_str()
            && let Ok(number) = raw.trim().parse::<u64>()
        {
            return Some(number);
        }
    }
    None
}

fn read_any_u64_opt(root: Option<&Map<String, Value>>, keys: &[&str]) -> Option<u64> {
    root.and_then(|map| read_any_u64(map, keys))
}

fn read_non_empty_env<F>(env_lookup: &F, key: &str) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    env_lookup(key).and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn expand_tilde_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed == "~" {
        return env::var_os("HOME")
            .or_else(|| env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(trimmed));
    }

    if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
        && let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE"))
    {
        return PathBuf::from(home).join(rest);
    }

    PathBuf::from(trimmed)
}

fn parse_relational_provider(raw: &str) -> Result<RelationalProvider> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "sqlite" => Ok(RelationalProvider::Sqlite),
        "postgres" | "postgresql" => Ok(RelationalProvider::Postgres),
        other => bail!("unsupported relational provider `{other}` (supported: sqlite, postgres)"),
    }
}

fn parse_events_provider(raw: &str) -> Result<EventsProvider> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "duckdb" => Ok(EventsProvider::DuckDb),
        "clickhouse" => Ok(EventsProvider::ClickHouse),
        other => {
            bail!("unsupported events provider `{other}` (supported: duckdb, clickhouse)")
        }
    }
}

fn parse_blob_storage_provider(raw: &str) -> Result<BlobStorageProvider> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "local" => Ok(BlobStorageProvider::Local),
        "s3" => Ok(BlobStorageProvider::S3),
        "gcs" => Ok(BlobStorageProvider::Gcs),
        other => bail!("unsupported blob provider `{other}` (supported: local, s3, gcs)"),
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
mod tests;
