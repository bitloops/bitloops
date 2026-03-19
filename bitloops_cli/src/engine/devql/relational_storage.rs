#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelationalDialect {
    Postgres,
    Sqlite,
}

#[derive(Debug)]
enum RelationalStorage {
    Postgres(tokio_postgres::Client),
    Sqlite { path: PathBuf },
}

impl RelationalStorage {
    async fn connect(
        cfg: &DevqlConfig,
        relational: &RelationalBackendConfig,
        command: &str,
    ) -> Result<Self> {
        match relational.provider {
            RelationalProvider::Postgres => {
                let pg_dsn = require_postgres_dsn(cfg, relational, command)?;
                let client = connect_postgres_client(pg_dsn).await?;
                Ok(Self::Postgres(client))
            }
            RelationalProvider::Sqlite => {
                let path = relational
                    .resolve_sqlite_db_path()
                    .with_context(|| format!("resolving SQLite path for `{command}`"))?;
                Ok(Self::Sqlite { path })
            }
        }
    }

    fn dialect(&self) -> RelationalDialect {
        match self {
            Self::Postgres(_) => RelationalDialect::Postgres,
            Self::Sqlite { .. } => RelationalDialect::Sqlite,
        }
    }

    async fn exec(&self, sql: &str) -> Result<()> {
        match self {
            Self::Postgres(client) => postgres_exec(client, sql).await,
            Self::Sqlite { path } => sqlite_exec_path(path, sql).await,
        }
    }

    async fn query_rows(&self, sql: &str) -> Result<Vec<Value>> {
        match self {
            Self::Postgres(client) => pg_query_rows(client, sql).await,
            Self::Sqlite { path } => sqlite_query_rows_path(path, sql).await,
        }
    }
}

async fn init_relational_schema(cfg: &DevqlConfig, relational: &RelationalStorage) -> Result<()> {
    match relational {
        RelationalStorage::Postgres(client) => init_postgres_schema(cfg, client).await,
        RelationalStorage::Sqlite { path } => init_sqlite_schema(path).await,
    }
}

fn require_postgres_dsn<'a>(
    cfg: &'a DevqlConfig,
    relational: &'a RelationalBackendConfig,
    command: &str,
) -> Result<&'a str> {
    relational
        .postgres_dsn
        .as_deref()
        .or(cfg.pg_dsn.as_deref())
        .ok_or_else(|| {
            anyhow!(
                "{DEVQL_POSTGRES_DSN_REQUIRED_PREFIX}: `{command}` requires `stores.relational.postgres_dsn` when `stores.relational.provider=postgres`"
            )
        })
}
