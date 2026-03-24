use super::*;

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
}

impl DevqlConfig {
    pub fn from_env(repo_root: PathBuf, repo: RepoIdentity) -> Result<Self> {
        let backend_cfg = resolve_store_backend_config_for_repo(&repo_root)
            .context("resolving backend config for DevQL runtime")?;
        let semantic_cfg = resolve_store_semantic_config();
        let embedding_cfg = resolve_store_embedding_config();
        Ok(Self {
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
pub enum RelationalStorage {
    Postgres(tokio_postgres::Client),
    Sqlite { path: PathBuf },
}

impl RelationalStorage {
    pub(super) async fn connect(
        cfg: &DevqlConfig,
        relational: &RelationalBackendConfig,
        command: &str,
    ) -> Result<Self> {
        if relational.has_postgres() {
            let pg_dsn = require_postgres_dsn(cfg, relational, command)?;
            let client = connect_postgres_client(pg_dsn).await?;
            Ok(Self::Postgres(client))
        } else {
            let path = relational
                .resolve_sqlite_db_path()
                .with_context(|| format!("resolving SQLite path for `{command}`"))?;
            Ok(Self::Sqlite { path })
        }
    }

    pub fn dialect(&self) -> RelationalDialect {
        match self {
            Self::Postgres(_) => RelationalDialect::Postgres,
            Self::Sqlite { .. } => RelationalDialect::Sqlite,
        }
    }

    pub async fn exec(&self, sql: &str) -> Result<()> {
        match self {
            Self::Postgres(client) => postgres_exec(client, sql).await,
            Self::Sqlite { path } => sqlite_exec_path(path, sql).await,
        }
    }

    pub async fn exec_batch_transactional(&self, statements: &[String]) -> Result<()> {
        match self {
            Self::Postgres(client) => postgres_exec_batch_transactional(client, statements).await,
            Self::Sqlite { path } => sqlite_exec_batch_transactional_path(path, statements).await,
        }
    }

    pub async fn query_rows(&self, sql: &str) -> Result<Vec<Value>> {
        match self {
            Self::Postgres(client) => pg_query_rows(client, sql).await,
            Self::Sqlite { path } => sqlite_query_rows_path(path, sql).await,
        }
    }
}
