use anyhow::Result;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use super::resolve::{
    resolve_blob_local_path, resolve_duckdb_db_path_for_repo, resolve_sqlite_db_path,
    resolve_sqlite_db_path_for_repo,
};
use super::store_config_utils::current_repo_root_or_cwd;

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

<<<<<<< Updated upstream
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreEmbeddingConfig {
    pub embedding_provider: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_api_key: Option<String>,
    pub embedding_cache_dir: Option<PathBuf>,
=======
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticSummaryMode {
    #[default]
    Auto,
    Off,
}

impl fmt::Display for SemanticSummaryMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::Off => write!(f, "off"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticCloneEmbeddingMode {
    Off,
    Deterministic,
    #[default]
    SemanticAwareOnce,
    RefreshOnUpgrade,
}

impl fmt::Display for SemanticCloneEmbeddingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Off => write!(f, "off"),
            Self::Deterministic => write!(f, "deterministic"),
            Self::SemanticAwareOnce => write!(f, "semantic_aware_once"),
            Self::RefreshOnUpgrade => write!(f, "refresh_on_upgrade"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticClonesConfig {
    pub summary_mode: SemanticSummaryMode,
    pub embedding_mode: SemanticCloneEmbeddingMode,
    pub embedding_profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingsRuntimeConfig {
    pub command: String,
    pub args: Vec<String>,
    pub startup_timeout_secs: u64,
    pub request_timeout_secs: u64,
}

impl Default for EmbeddingsRuntimeConfig {
    fn default() -> Self {
        Self {
            command: "bitloops-embeddings".to_string(),
            args: Vec::new(),
            startup_timeout_secs: 10,
            request_timeout_secs: 60,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmbeddingProfileConfig {
    pub name: String,
    pub kind: String,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub cache_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmbeddingsConfig {
    pub runtime: EmbeddingsRuntimeConfig,
    pub profiles: BTreeMap<String, EmbeddingProfileConfig>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmbeddingCapabilityConfig {
    pub semantic_clones: SemanticClonesConfig,
    pub embeddings: EmbeddingsConfig,
>>>>>>> Stashed changes
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticSummaryMode {
    #[default]
    Auto,
    Off,
}

impl fmt::Display for SemanticSummaryMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::Off => write!(f, "off"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticCloneEmbeddingMode {
    Off,
    Deterministic,
    #[default]
    SemanticAwareOnce,
    RefreshOnUpgrade,
}

impl fmt::Display for SemanticCloneEmbeddingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Off => write!(f, "off"),
            Self::Deterministic => write!(f, "deterministic"),
            Self::SemanticAwareOnce => write!(f, "semantic_aware_once"),
            Self::RefreshOnUpgrade => write!(f, "refresh_on_upgrade"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticClonesConfig {
    pub summary_mode: SemanticSummaryMode,
    pub embedding_mode: SemanticCloneEmbeddingMode,
    pub embedding_profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingsRuntimeConfig {
    pub command: String,
    pub args: Vec<String>,
    pub startup_timeout_secs: u64,
    pub request_timeout_secs: u64,
}

impl Default for EmbeddingsRuntimeConfig {
    fn default() -> Self {
        Self {
            command: "bitloops-embeddings".to_string(),
            args: Vec::new(),
            startup_timeout_secs: 10,
            request_timeout_secs: 60,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmbeddingProfileConfig {
    pub name: String,
    pub kind: String,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub cache_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmbeddingsConfig {
    pub runtime: EmbeddingsRuntimeConfig,
    pub profiles: BTreeMap<String, EmbeddingProfileConfig>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmbeddingCapabilityConfig {
    pub semantic_clones: SemanticClonesConfig,
    pub embeddings: EmbeddingsConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationalBackendConfig {
    pub sqlite_path: Option<String>,
    pub postgres_dsn: Option<String>,
}

impl RelationalBackendConfig {
    /// Returns `true` when a Postgres DSN is configured.
    pub fn has_postgres(&self) -> bool {
        self.postgres_dsn.is_some()
    }

    pub fn resolve_sqlite_db_path(&self) -> Result<PathBuf> {
        resolve_sqlite_db_path(self.sqlite_path.as_deref())
    }

    /// Resolve the SQLite path relative to an explicit repo root (avoids cwd dependency).
    pub fn resolve_sqlite_db_path_for_repo(&self, repo_root: &Path) -> Result<PathBuf> {
        resolve_sqlite_db_path_for_repo(repo_root, self.sqlite_path.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventsBackendConfig {
    pub duckdb_path: Option<String>,
    pub clickhouse_url: Option<String>,
    pub clickhouse_user: Option<String>,
    pub clickhouse_password: Option<String>,
    pub clickhouse_database: Option<String>,
}

impl EventsBackendConfig {
    /// Returns `true` when a ClickHouse URL is configured.
    pub fn has_clickhouse(&self) -> bool {
        self.clickhouse_url.is_some()
    }

    pub fn duckdb_path_or_default(&self) -> PathBuf {
        let repo_root = current_repo_root_or_cwd();
        resolve_duckdb_db_path_for_repo(&repo_root, self.duckdb_path.as_deref())
    }

    /// Resolve the DuckDB path relative to an explicit repo root (avoids cwd dependency).
    pub fn resolve_duckdb_db_path_for_repo(&self, repo_root: &Path) -> PathBuf {
        resolve_duckdb_db_path_for_repo(repo_root, self.duckdb_path.as_deref())
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
    pub local_path: Option<String>,
    pub s3_bucket: Option<String>,
    pub s3_region: Option<String>,
    pub s3_access_key_id: Option<String>,
    pub s3_secret_access_key: Option<String>,
    pub gcs_bucket: Option<String>,
    pub gcs_credentials_path: Option<String>,
}

impl BlobStorageConfig {
    /// Returns `true` when any remote blob backend (S3 or GCS) is configured.
    pub fn has_remote(&self) -> bool {
        self.s3_bucket.is_some() || self.gcs_bucket.is_some()
    }

    #[allow(dead_code)]
    pub fn local_path_or_default(&self) -> Result<PathBuf> {
        resolve_blob_local_path(self.local_path.as_deref())
    }
}

#[derive(Debug, Clone, Default)]
pub struct StoreFileConfig {
    pub(crate) sqlite_path: Option<String>,
    pub(crate) pg_dsn: Option<String>,
    pub(crate) duckdb_path: Option<String>,
    pub(crate) clickhouse_url: Option<String>,
    pub(crate) clickhouse_user: Option<String>,
    pub(crate) clickhouse_password: Option<String>,
    pub(crate) clickhouse_database: Option<String>,
    pub(crate) semantic_provider: Option<String>,
    pub(crate) semantic_model: Option<String>,
    pub(crate) semantic_api_key: Option<String>,
    pub(crate) semantic_base_url: Option<String>,
<<<<<<< Updated upstream
    #[allow(dead_code)]
=======
<<<<<<< Updated upstream
>>>>>>> Stashed changes
    pub(crate) embedding_provider: Option<String>,
    #[allow(dead_code)]
    pub(crate) embedding_model: Option<String>,
    #[allow(dead_code)]
    pub(crate) embedding_api_key: Option<String>,
    #[allow(dead_code)]
    pub(crate) embedding_cache_dir: Option<String>,
=======
>>>>>>> Stashed changes
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
    pub local_dashboard: Option<DashboardLocalDashboardConfig>,
    pub bundle_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashboardLocalDashboardConfig {
    pub tls: Option<bool>,
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
