//! Shared DevQL config file parsing and path.
//! Used by both the CLI (devql commands) and the dashboard server so supported keys and defaults stay in sync.

use anyhow::{Result, bail};
use serde_json::{Map, Value};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Relative path from user home to the DevQL config file (e.g. `~/.bitloops/config.json`).
pub const DEVQL_CONFIG_RELATIVE_PATH: &str = ".bitloops/config.json";
pub const DEVQL_DUCKDB_DEFAULT_PATH: &str = "~/.bitloops/devql/events.duckdb";
/// Default relative path from user home to the local SQLite relational DB file.
pub const DEVQL_SQLITE_RELATIVE_PATH: &str = ".bitloops/devql/relational.db";
/// Default relative path from user home to local blob storage.
#[allow(dead_code)]
pub const DEVQL_BLOB_LOCAL_RELATIVE_PATH: &str = ".bitloops/blobs";

const ENV_RELATIONAL_PROVIDER: &str = "BITLOOPS_DEVQL_RELATIONAL_PROVIDER";
const ENV_EVENTS_PROVIDER: &str = "BITLOOPS_DEVQL_EVENTS_PROVIDER";
const ENV_SQLITE_PATH: &str = "BITLOOPS_DEVQL_SQLITE_PATH";
const ENV_DUCKDB_PATH: &str = "BITLOOPS_DEVQL_DUCKDB_PATH";
const ENV_POSTGRES_DSN: &str = "BITLOOPS_DEVQL_PG_DSN";
const ENV_CLICKHOUSE_URL: &str = "BITLOOPS_DEVQL_CH_URL";
const ENV_CLICKHOUSE_USER: &str = "BITLOOPS_DEVQL_CH_USER";
const ENV_CLICKHOUSE_PASSWORD: &str = "BITLOOPS_DEVQL_CH_PASSWORD";
const ENV_CLICKHOUSE_DATABASE: &str = "BITLOOPS_DEVQL_CH_DATABASE";
const ENV_SEMANTIC_PROVIDER: &str = "BITLOOPS_DEVQL_SEMANTIC_PROVIDER";
const ENV_SEMANTIC_MODEL: &str = "BITLOOPS_DEVQL_SEMANTIC_MODEL";
const ENV_SEMANTIC_API_KEY: &str = "BITLOOPS_DEVQL_SEMANTIC_API_KEY";
const ENV_SEMANTIC_BASE_URL: &str = "BITLOOPS_DEVQL_SEMANTIC_BASE_URL";
const ENV_EMBEDDING_PROVIDER: &str = "BITLOOPS_DEVQL_EMBEDDING_PROVIDER";
const ENV_EMBEDDING_MODEL: &str = "BITLOOPS_DEVQL_EMBEDDING_MODEL";
const ENV_EMBEDDING_API_KEY: &str = "BITLOOPS_DEVQL_EMBEDDING_API_KEY";
const DEFAULT_EMBEDDING_PROVIDER: &str = "local";
const ENV_BLOB_STORAGE_PROVIDER: &str = "BITLOOPS_DEVQL_BLOB_PROVIDER";
const ENV_BLOB_LOCAL_PATH: &str = "BITLOOPS_DEVQL_BLOB_LOCAL_PATH";
const ENV_BLOB_S3_BUCKET: &str = "BITLOOPS_DEVQL_BLOB_S3_BUCKET";
const ENV_BLOB_S3_REGION: &str = "BITLOOPS_DEVQL_BLOB_S3_REGION";
const ENV_BLOB_S3_ACCESS_KEY_ID: &str = "BITLOOPS_DEVQL_BLOB_S3_ACCESS_KEY_ID";
const ENV_BLOB_S3_SECRET_ACCESS_KEY: &str = "BITLOOPS_DEVQL_BLOB_S3_SECRET_ACCESS_KEY";
const ENV_BLOB_GCS_BUCKET: &str = "BITLOOPS_DEVQL_BLOB_GCS_BUCKET";
const ENV_BLOB_GCS_CREDENTIALS_PATH: &str = "BITLOOPS_DEVQL_BLOB_GCS_CREDENTIALS_PATH";
const DASHBOARD_CONFIG_KEY: &str = "dashboard";
const DASHBOARD_USE_BITLOOPS_LOCAL_KEY: &str = "use_bitloops_local";

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
pub struct DevqlBackendConfig {
    pub relational: RelationalBackendConfig,
    pub events: EventsBackendConfig,
    pub blobs: BlobStorageConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DevqlSemanticConfig {
    pub semantic_provider: Option<String>,
    pub semantic_model: Option<String>,
    pub semantic_api_key: Option<String>,
    pub semantic_base_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DevqlEmbeddingConfig {
    pub embedding_provider: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_api_key: Option<String>,
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
        match self.duckdb_path.as_deref() {
            // For an explicitly configured path, preserve existing behavior:
            // expand a leading '~' if possible, otherwise return it as-is.
            Some(path) => expand_tilde_path(path),
            // For the default path, avoid creating a literal '~' directory when
            // HOME/USERPROFILE is not set by resolving the home directory
            // explicitly and falling back to a relative path without '~'.
            None => {
                // DEVQL_DUCKDB_DEFAULT_PATH is "~/.bitloops/devql/events.duckdb"
                let relative = DEVQL_DUCKDB_DEFAULT_PATH
                    .strip_prefix("~/")
                    .unwrap_or(DEVQL_DUCKDB_DEFAULT_PATH);

                if let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")) {
                    PathBuf::from(home).join(relative)
                } else {
                    // Fall back to a relative path without '~' to avoid creating
                    // a directory literally named "~" in the current working directory.
                    PathBuf::from(relative)
                }
            }
        }
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
pub struct DevqlFileConfig {
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
    pub(crate) embedding_provider: Option<String>,
    pub(crate) embedding_model: Option<String>,
    pub(crate) embedding_api_key: Option<String>,
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

impl DevqlFileConfig {
    /// Load config from `$HOME/.bitloops/config.json` (or `$USERPROFILE` on Windows).
    /// Returns default if the file is missing or invalid.
    pub fn load() -> Self {
        let Some(path) = user_home_config_path() else {
            return Self::default();
        };

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
    /// Looks for a `"devql"` object first, then falls back to the root object.
    pub fn from_json_value(value: &Value) -> Self {
        let root_opt = value
            .get("devql")
            .and_then(Value::as_object)
            .or_else(|| value.as_object());
        let Some(root) = root_opt else {
            return Self::default();
        };

        let relational = root.get("relational").and_then(Value::as_object);
        let events = root.get("events").and_then(Value::as_object);
        let blobs = root.get("blobs").and_then(Value::as_object);

        Self {
            relational_provider: read_any_string_opt(
                relational,
                &["provider", "relational_provider", ENV_RELATIONAL_PROVIDER],
            )
            .or_else(|| read_any_string(root, &["relational_provider", ENV_RELATIONAL_PROVIDER])),
            sqlite_path: read_any_string_opt(relational, &["sqlite_path", "path"])
                .or_else(|| read_any_string(root, &["sqlite_path", ENV_SQLITE_PATH])),
            pg_dsn: read_any_string_opt(relational, &["postgres_dsn", "pg_dsn", ENV_POSTGRES_DSN])
                .or_else(|| read_any_string(root, &["postgres_dsn", "pg_dsn", ENV_POSTGRES_DSN])),
            events_provider: read_any_string_opt(events, &["provider", "events_provider"])
                .or_else(|| read_any_string(root, &["events_provider", ENV_EVENTS_PROVIDER])),
            duckdb_path: read_any_string_opt(events, &["duckdb_path", "path"])
                .or_else(|| read_any_string(root, &["duckdb_path", ENV_DUCKDB_PATH])),
            clickhouse_url: read_any_string_opt(events, &["clickhouse_url", "ch_url"]).or_else(
                || read_any_string(root, &["clickhouse_url", "ch_url", ENV_CLICKHOUSE_URL]),
            ),
            clickhouse_user: read_any_string_opt(events, &["clickhouse_user", "ch_user"]).or_else(
                || read_any_string(root, &["clickhouse_user", "ch_user", ENV_CLICKHOUSE_USER]),
            ),
            clickhouse_password: read_any_string_opt(
                events,
                &["clickhouse_password", "ch_password"],
            )
            .or_else(|| {
                read_any_string(
                    root,
                    &[
                        "clickhouse_password",
                        "ch_password",
                        ENV_CLICKHOUSE_PASSWORD,
                    ],
                )
            }),
            clickhouse_database: read_any_string_opt(
                events,
                &["clickhouse_database", "ch_database"],
            )
            .or_else(|| {
                read_any_string(
                    root,
                    &[
                        "clickhouse_database",
                        "ch_database",
                        ENV_CLICKHOUSE_DATABASE,
                    ],
                )
            }),
            semantic_provider: read_any_string(root, &["semantic_provider", ENV_SEMANTIC_PROVIDER]),
            semantic_model: read_any_string(root, &["semantic_model", ENV_SEMANTIC_MODEL]),
            semantic_api_key: read_any_string(root, &["semantic_api_key", ENV_SEMANTIC_API_KEY]),
            semantic_base_url: read_any_string(root, &["semantic_base_url", ENV_SEMANTIC_BASE_URL]),
            embedding_provider: read_any_string(
                root,
                &["embedding_provider", ENV_EMBEDDING_PROVIDER],
            ),
            embedding_model: read_any_string(root, &["embedding_model", ENV_EMBEDDING_MODEL]),
            embedding_api_key: read_any_string(root, &["embedding_api_key", ENV_EMBEDDING_API_KEY]),
            blob_provider: read_any_string_opt(
                blobs,
                &["provider", "blob_provider", ENV_BLOB_STORAGE_PROVIDER],
            )
            .or_else(|| read_any_string(root, &["blob_provider", ENV_BLOB_STORAGE_PROVIDER])),
            blob_local_path: read_any_string_opt(blobs, &["local_path", ENV_BLOB_LOCAL_PATH])
                .or_else(|| read_any_string(root, &["blob_local_path", ENV_BLOB_LOCAL_PATH])),
            blob_s3_bucket: read_any_string_opt(blobs, &["s3_bucket", ENV_BLOB_S3_BUCKET])
                .or_else(|| read_any_string(root, &["blob_s3_bucket", ENV_BLOB_S3_BUCKET])),
            blob_s3_region: read_any_string_opt(blobs, &["s3_region", ENV_BLOB_S3_REGION])
                .or_else(|| read_any_string(root, &["blob_s3_region", ENV_BLOB_S3_REGION])),
            blob_s3_access_key_id: read_any_string_opt(
                blobs,
                &["s3_access_key_id", ENV_BLOB_S3_ACCESS_KEY_ID],
            )
            .or_else(|| {
                read_any_string(root, &["blob_s3_access_key_id", ENV_BLOB_S3_ACCESS_KEY_ID])
            }),
            blob_s3_secret_access_key: read_any_string_opt(
                blobs,
                &["s3_secret_access_key", ENV_BLOB_S3_SECRET_ACCESS_KEY],
            )
            .or_else(|| {
                read_any_string(
                    root,
                    &["blob_s3_secret_access_key", ENV_BLOB_S3_SECRET_ACCESS_KEY],
                )
            }),
            blob_gcs_bucket: read_any_string_opt(blobs, &["gcs_bucket", ENV_BLOB_GCS_BUCKET])
                .or_else(|| read_any_string(root, &["blob_gcs_bucket", ENV_BLOB_GCS_BUCKET])),
            blob_gcs_credentials_path: read_any_string_opt(
                blobs,
                &["gcs_credentials_path", ENV_BLOB_GCS_CREDENTIALS_PATH],
            )
            .or_else(|| {
                read_any_string(
                    root,
                    &["blob_gcs_credentials_path", ENV_BLOB_GCS_CREDENTIALS_PATH],
                )
            }),
        }
    }
}

impl DashboardFileConfig {
    /// Load dashboard config from `$HOME/.bitloops/config.json` (or `$USERPROFILE` on Windows).
    /// Returns default if the file is missing or invalid.
    pub fn load() -> Self {
        let Some(path) = user_home_config_path() else {
            return Self::default();
        };

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
    /// Reads the nested `"dashboard"` object.
    pub fn from_json_value(value: &Value) -> Self {
        let Some(root) = value.get(DASHBOARD_CONFIG_KEY).and_then(Value::as_object) else {
            return Self::default();
        };

        Self {
            use_bitloops_local: read_any_bool(root, &[DASHBOARD_USE_BITLOOPS_LOCAL_KEY]),
        }
    }
}

pub fn dashboard_use_bitloops_local() -> bool {
    DashboardFileConfig::load()
        .use_bitloops_local
        .unwrap_or(false)
}

pub fn resolve_devql_backend_config() -> Result<DevqlBackendConfig> {
    let file_cfg = DevqlFileConfig::load();
    resolve_devql_backend_config_with(file_cfg, |key| env::var(key).ok())
}

pub fn resolve_devql_semantic_config() -> DevqlSemanticConfig {
    let file_cfg = DevqlFileConfig::load();
    resolve_devql_semantic_config_with(file_cfg, |key| env::var(key).ok())
}

pub fn resolve_devql_embedding_config() -> DevqlEmbeddingConfig {
    let file_cfg = DevqlFileConfig::load();
    resolve_devql_embedding_config_with(file_cfg, |key| env::var(key).ok())
}

pub fn resolve_sqlite_db_path(raw_path: Option<&str>) -> Result<PathBuf> {
    match raw_path {
        Some(raw) if !raw.trim().is_empty() => normalize_sqlite_path(raw),
        _ => {
            let Some(home) = user_home_dir() else {
                bail!(
                    "unable to resolve home directory for default SQLite path; configure `devql.relational.sqlite_path` or `BITLOOPS_DEVQL_SQLITE_PATH`"
                );
            };
            Ok(home.join(DEVQL_SQLITE_RELATIVE_PATH))
        }
    }
}

#[allow(dead_code)]
pub fn resolve_blob_local_path(raw_path: Option<&str>) -> Result<PathBuf> {
    match raw_path {
        Some(raw) if !raw.trim().is_empty() => {
            let expanded = expand_home_prefix(raw.trim())?;
            Ok(PathBuf::from(expanded))
        }
        _ => {
            let Some(home) = user_home_dir() else {
                bail!(
                    "unable to resolve home directory for default blob path; configure `devql.blobs.local_path` or `BITLOOPS_DEVQL_BLOB_LOCAL_PATH`"
                );
            };
            Ok(home.join(DEVQL_BLOB_LOCAL_RELATIVE_PATH))
        }
    }
}

fn resolve_devql_backend_config_with<F>(
    file_cfg: DevqlFileConfig,
    env_lookup: F,
) -> Result<DevqlBackendConfig>
where
    F: Fn(&str) -> Option<String>,
{
    let env_rel_provider = read_non_empty_env(&env_lookup, ENV_RELATIONAL_PROVIDER);
    let env_events_provider = read_non_empty_env(&env_lookup, ENV_EVENTS_PROVIDER);

    let sqlite_path = read_non_empty_env(&env_lookup, ENV_SQLITE_PATH).or(file_cfg.sqlite_path);
    let postgres_dsn = read_non_empty_env(&env_lookup, ENV_POSTGRES_DSN).or(file_cfg.pg_dsn);

    let duckdb_path = read_non_empty_env(&env_lookup, ENV_DUCKDB_PATH).or(file_cfg.duckdb_path);
    let clickhouse_url =
        read_non_empty_env(&env_lookup, ENV_CLICKHOUSE_URL).or(file_cfg.clickhouse_url);
    let clickhouse_user =
        read_non_empty_env(&env_lookup, ENV_CLICKHOUSE_USER).or(file_cfg.clickhouse_user);
    let clickhouse_password =
        read_non_empty_env(&env_lookup, ENV_CLICKHOUSE_PASSWORD).or(file_cfg.clickhouse_password);
    let clickhouse_database =
        read_non_empty_env(&env_lookup, ENV_CLICKHOUSE_DATABASE).or(file_cfg.clickhouse_database);
    let blob_provider_raw =
        read_non_empty_env(&env_lookup, ENV_BLOB_STORAGE_PROVIDER).or(file_cfg.blob_provider);
    let blob_local_path =
        read_non_empty_env(&env_lookup, ENV_BLOB_LOCAL_PATH).or(file_cfg.blob_local_path);
    let blob_s3_bucket =
        read_non_empty_env(&env_lookup, ENV_BLOB_S3_BUCKET).or(file_cfg.blob_s3_bucket);
    let blob_s3_region =
        read_non_empty_env(&env_lookup, ENV_BLOB_S3_REGION).or(file_cfg.blob_s3_region);
    let blob_s3_access_key_id = read_non_empty_env(&env_lookup, ENV_BLOB_S3_ACCESS_KEY_ID)
        .or(file_cfg.blob_s3_access_key_id);
    let blob_s3_secret_access_key = read_non_empty_env(&env_lookup, ENV_BLOB_S3_SECRET_ACCESS_KEY)
        .or(file_cfg.blob_s3_secret_access_key);
    let blob_gcs_bucket =
        read_non_empty_env(&env_lookup, ENV_BLOB_GCS_BUCKET).or(file_cfg.blob_gcs_bucket);
    let blob_gcs_credentials_path = read_non_empty_env(&env_lookup, ENV_BLOB_GCS_CREDENTIALS_PATH)
        .or(file_cfg.blob_gcs_credentials_path);

    let relational_provider = if let Some(raw) = env_rel_provider.or(file_cfg.relational_provider) {
        parse_relational_provider(&raw)?
    } else if postgres_dsn.is_some() {
        RelationalProvider::Postgres
    } else {
        RelationalProvider::Sqlite
    };

    let clickhouse_legacy_detected = clickhouse_url.is_some()
        || clickhouse_user.is_some()
        || clickhouse_password.is_some()
        || clickhouse_database.is_some();
    let events_provider = if let Some(raw) = env_events_provider.or(file_cfg.events_provider) {
        parse_events_provider(&raw)?
    } else if clickhouse_legacy_detected {
        EventsProvider::ClickHouse
    } else {
        EventsProvider::DuckDb
    };
    let blob_provider = if let Some(raw) = blob_provider_raw {
        parse_blob_storage_provider(&raw)?
    } else if blob_s3_bucket.is_some() {
        BlobStorageProvider::S3
    } else if blob_gcs_bucket.is_some() {
        BlobStorageProvider::Gcs
    } else {
        BlobStorageProvider::Local
    };

    Ok(DevqlBackendConfig {
        relational: RelationalBackendConfig {
            provider: relational_provider,
            sqlite_path,
            postgres_dsn,
        },
        events: EventsBackendConfig {
            provider: events_provider,
            duckdb_path,
            clickhouse_url,
            clickhouse_user,
            clickhouse_password,
            clickhouse_database,
        },
        blobs: BlobStorageConfig {
            provider: blob_provider,
            local_path: blob_local_path,
            s3_bucket: blob_s3_bucket,
            s3_region: blob_s3_region,
            s3_access_key_id: blob_s3_access_key_id,
            s3_secret_access_key: blob_s3_secret_access_key,
            gcs_bucket: blob_gcs_bucket,
            gcs_credentials_path: blob_gcs_credentials_path,
        },
    })
}

fn resolve_devql_semantic_config_with<F>(
    file_cfg: DevqlFileConfig,
    env_lookup: F,
) -> DevqlSemanticConfig
where
    F: Fn(&str) -> Option<String>,
{
    DevqlSemanticConfig {
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

fn resolve_devql_embedding_config_with<F>(
    file_cfg: DevqlFileConfig,
    env_lookup: F,
) -> DevqlEmbeddingConfig
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

    DevqlEmbeddingConfig {
        embedding_provider,
        embedding_model,
        embedding_api_key,
    }
}

fn user_home_config_path() -> Option<PathBuf> {
    user_home_dir().map(|home| home.join(DEVQL_CONFIG_RELATIVE_PATH))
}

fn user_home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn normalize_sqlite_path(raw_path: &str) -> Result<PathBuf> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        bail!(
            "sqlite path is empty; set `devql.relational.sqlite_path` or `BITLOOPS_DEVQL_SQLITE_PATH`"
        );
    }

    let expanded = expand_home_prefix(trimmed)?;
    Ok(Path::new(&expanded).to_path_buf())
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
        other => {
            bail!("unsupported devql relational provider `{other}` (supported: sqlite, postgres)")
        }
    }
}

fn parse_events_provider(raw: &str) -> Result<EventsProvider> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "duckdb" => Ok(EventsProvider::DuckDb),
        "clickhouse" => Ok(EventsProvider::ClickHouse),
        other => {
            bail!("unsupported devql events provider `{other}` (supported: duckdb, clickhouse)")
        }
    }
}

fn parse_blob_storage_provider(raw: &str) -> Result<BlobStorageProvider> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "local" => Ok(BlobStorageProvider::Local),
        "s3" => Ok(BlobStorageProvider::S3),
        "gcs" => Ok(BlobStorageProvider::Gcs),
        other => {
            bail!("unsupported devql blob storage provider `{other}` (supported: local, s3, gcs)")
        }
    }
}

#[cfg(test)]
pub(crate) fn resolve_devql_backend_config_for_tests(
    file_cfg: DevqlFileConfig,
    env: &[(&str, &str)],
) -> Result<DevqlBackendConfig> {
    resolve_devql_backend_config_with(file_cfg, |key| {
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
pub(crate) fn resolve_devql_semantic_config_for_tests(
    file_cfg: DevqlFileConfig,
    env: &[(&str, &str)],
) -> DevqlSemanticConfig {
    resolve_devql_semantic_config_with(file_cfg, |key| {
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
pub(crate) fn resolve_devql_embedding_config_for_tests(
    file_cfg: DevqlFileConfig,
    env: &[(&str, &str)],
) -> DevqlEmbeddingConfig {
    resolve_devql_embedding_config_with(file_cfg, |key| {
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
