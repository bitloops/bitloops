use anyhow::Result;
use std::path::PathBuf;

use super::resolve::{
    resolve_blob_local_path, resolve_duckdb_db_path_for_repo, resolve_sqlite_db_path,
};
use super::store_config_utils::current_repo_root_or_cwd;

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
pub struct ProviderConfig {
    pub github: Option<GithubProviderConfig>,
    pub atlassian: Option<AtlassianProviderConfig>,
    pub jira: Option<AtlassianProviderConfig>,
    pub confluence: Option<AtlassianProviderConfig>,
}

impl ProviderConfig {
    pub fn jira_config(&self) -> Option<&AtlassianProviderConfig> {
        self.jira.as_ref().or(self.atlassian.as_ref())
    }

    pub fn confluence_config(&self) -> Option<&AtlassianProviderConfig> {
        self.confluence.as_ref().or(self.atlassian.as_ref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubProviderConfig {
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtlassianProviderConfig {
    pub site_url: String,
    pub email: String,
    pub token: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreSemanticConfig {
    pub semantic_provider: Option<String>,
    pub semantic_model: Option<String>,
    pub semantic_api_key: Option<String>,
    pub semantic_base_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreEmbeddingConfig {
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
