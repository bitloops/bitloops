use super::*;
use crate::config::{
    resolve_store_embedding_config_for_repo, resolve_store_semantic_config_for_repo,
};

#[derive(Debug, Clone)]
pub struct RepoIdentity {
    pub(crate) provider: String,
    pub(crate) organization: String,
    pub(crate) name: String,
    pub(crate) identity: String,
    pub(crate) repo_id: String,
}

#[derive(Debug, Clone)]
pub struct DevqlConfig {
    pub(crate) config_root: PathBuf,
    pub(crate) repo_root: PathBuf,
    pub(crate) repo: RepoIdentity,
    pub(crate) pg_dsn: Option<String>,
    pub(crate) clickhouse_url: String,
    pub(crate) clickhouse_user: Option<String>,
    pub(crate) clickhouse_password: Option<String>,
    pub(crate) clickhouse_database: String,
    pub(crate) semantic_provider: Option<String>,
    pub(crate) semantic_model: Option<String>,
    pub(crate) semantic_api_key: Option<String>,
    pub(crate) semantic_base_url: Option<String>,
    pub(crate) embedding_provider: Option<String>,
    pub(crate) embedding_model: Option<String>,
    pub(crate) embedding_api_key: Option<String>,
    pub(crate) embedding_cache_dir: Option<PathBuf>,
}

impl DevqlConfig {
    pub fn from_env(repo_root: PathBuf, repo: RepoIdentity) -> Result<Self> {
        Self::from_roots(repo_root.clone(), repo_root, repo)
    }

    pub fn from_roots(
        config_root: PathBuf,
        repo_root: PathBuf,
        repo: RepoIdentity,
    ) -> Result<Self> {
        let backend_cfg = resolve_store_backend_config_for_repo(&config_root)
            .context("resolving backend config for DevQL runtime")?;
        let semantic_cfg = resolve_store_semantic_config_for_repo(&config_root);
        let embedding_cfg = resolve_store_embedding_config_for_repo(&config_root);
        Ok(Self {
            config_root,
            repo_root,
            repo,
            pg_dsn: backend_cfg.relational.postgres_dsn,
            clickhouse_url: backend_cfg
                .events
                .clickhouse_url
                .unwrap_or_else(|| "http://localhost:8123".to_string()),
            clickhouse_user: backend_cfg.events.clickhouse_user,
            clickhouse_password: backend_cfg.events.clickhouse_password,
            clickhouse_database: backend_cfg
                .events
                .clickhouse_database
                .unwrap_or_else(|| "default".to_string()),
            semantic_provider: semantic_cfg.semantic_provider,
            semantic_model: semantic_cfg.semantic_model,
            semantic_api_key: semantic_cfg.semantic_api_key,
            semantic_base_url: semantic_cfg.semantic_base_url,
            embedding_provider: embedding_cfg.embedding_provider,
            embedding_model: embedding_cfg.embedding_model,
            embedding_api_key: embedding_cfg.embedding_api_key,
            embedding_cache_dir: embedding_cfg.embedding_cache_dir,
        })
    }

    pub(super) fn clickhouse_endpoint(&self) -> String {
        let base = self.clickhouse_url.trim_end_matches('/');
        format!("{base}/?database={}", self.clickhouse_database)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationalDialect {
    Postgres,
    Sqlite,
}

#[derive(Debug)]
pub struct SqliteStorage {
    pub path: PathBuf,
}

#[derive(Debug)]
pub struct PostgresStorage {
    pub client: tokio_postgres::Client,
}

#[derive(Debug)]
pub struct RelationalStorage {
    pub local: SqliteStorage,
    pub remote: Option<PostgresStorage>,
}

impl RelationalStorage {
    pub(super) async fn connect(
        cfg: &DevqlConfig,
        relational: &RelationalBackendConfig,
        command: &str,
    ) -> Result<Self> {
        let sqlite_path = relational
            .resolve_sqlite_db_path_for_repo(&cfg.repo_root)
            .with_context(|| format!("resolving SQLite path for `{command}`"))?;
        let remote_dsn = relational
            .postgres_dsn
            .as_deref()
            .or(cfg.pg_dsn.as_deref())
            .map(str::trim)
            .filter(|dsn| !dsn.is_empty());
        let remote = if let Some(dsn) = remote_dsn {
            let client = connect_postgres_client(dsn).await?;
            Some(PostgresStorage { client })
        } else {
            None
        };

        Ok(Self {
            local: SqliteStorage { path: sqlite_path },
            remote,
        })
    }

    pub fn local_only(path: PathBuf) -> Self {
        Self {
            local: SqliteStorage { path },
            remote: None,
        }
    }

    pub fn with_remote_client(path: PathBuf, client: tokio_postgres::Client) -> Self {
        Self {
            local: SqliteStorage { path },
            remote: Some(PostgresStorage { client }),
        }
    }

    pub fn dialect(&self) -> RelationalDialect {
        RelationalDialect::Sqlite
    }

    pub async fn exec(&self, sql: &str) -> Result<()> {
        sqlite_exec_path(&self.local.path, sql).await
    }

    pub async fn exec_batch_transactional(&self, statements: &[String]) -> Result<()> {
        sqlite_exec_batch_transactional_path(&self.local.path, statements).await
    }

    pub async fn exec_remote_batch_transactional(&self, statements: &[String]) -> Result<()> {
        if let Some(remote) = self.remote.as_ref() {
            return postgres_exec_batch_transactional(&remote.client, statements).await;
        }
        bail!("remote Postgres storage is not configured")
    }

    pub async fn query_rows(&self, sql: &str) -> Result<Vec<Value>> {
        sqlite_query_rows_path(&self.local.path, sql).await
    }

    pub async fn query_rows_remote(&self, sql: &str) -> Result<Vec<Value>> {
        if let Some(remote) = self.remote.as_ref() {
            return pg_query_rows(&remote.client, sql).await;
        }
        bail!("remote Postgres storage is not configured")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cfg(repo_root: PathBuf) -> DevqlConfig {
        DevqlConfig {
            config_root: repo_root.clone(),
            repo_root,
            repo: RepoIdentity {
                provider: "git".to_string(),
                organization: "bitloops".to_string(),
                name: "bitloops".to_string(),
                identity: "git/bitloops/bitloops".to_string(),
                repo_id: "repo-1".to_string(),
            },
            pg_dsn: None,
            clickhouse_url: "http://localhost:8123".to_string(),
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: "default".to_string(),
            semantic_provider: None,
            semantic_model: None,
            semantic_api_key: None,
            semantic_base_url: None,
            embedding_provider: None,
            embedding_model: None,
            embedding_api_key: None,
            embedding_cache_dir: None,
        }
    }

    #[tokio::test]
    async fn connect_always_builds_local_sqlite_storage() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let sqlite_path = temp.path().join("stores").join("relational.sqlite");
        let cfg = sample_cfg(temp.path().to_path_buf());
        let backends = RelationalBackendConfig {
            sqlite_path: Some(sqlite_path.to_string_lossy().to_string()),
            postgres_dsn: None,
        };

        let relational = RelationalStorage::connect(&cfg, &backends, "devql test")
            .await
            .expect("connect relational storage");

        assert_eq!(relational.local.path, sqlite_path);
        assert!(relational.remote.is_none());
        assert_eq!(relational.dialect(), RelationalDialect::Sqlite);
    }

    #[tokio::test]
    async fn connect_fails_fast_when_postgres_dsn_is_invalid() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let sqlite_path = temp.path().join("stores").join("relational.sqlite");
        let cfg = sample_cfg(temp.path().to_path_buf());
        let backends = RelationalBackendConfig {
            sqlite_path: Some(sqlite_path.to_string_lossy().to_string()),
            postgres_dsn: Some("postgres://not a valid dsn".to_string()),
        };

        let err = RelationalStorage::connect(&cfg, &backends, "devql test")
            .await
            .expect_err("invalid DSN should fail");
        assert!(
            err.to_string().contains("parsing Postgres DSN")
                || err.to_string().contains("connecting to Postgres"),
            "expected DSN connection setup to fail, got: {err:#}"
        );
    }
}
