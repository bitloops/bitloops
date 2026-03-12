use self::store_contracts::{
    CheckpointEventWrite, EventsCheckpointHistoryQuery, EventsCheckpointQuery,
    EventsCommitShaQuery, EventsStore, EventsTelemetryQuery, StoreFuture,
};
use rusqlite::types::ValueRef;

#[derive(Debug)]
struct PostgresRelationalStore {
    client: tokio_postgres::Client,
}

impl PostgresRelationalStore {
    async fn connect(dsn: &str) -> Result<Self> {
        Ok(Self {
            client: connect_postgres_client(dsn).await?,
        })
    }
}

impl store_contracts::RelationalStore for PostgresRelationalStore {
    fn provider(&self) -> RelationalProvider {
        RelationalProvider::Postgres
    }

    fn ping<'a>(&'a self) -> store_contracts::StoreFuture<'a, i32> {
        Box::pin(async move { run_postgres_ping(&self.client).await })
    }

    fn init_schema<'a>(&'a self) -> store_contracts::StoreFuture<'a, ()> {
        Box::pin(async move {
            postgres_exec(&self.client, RELATIONAL_SCHEMA_SQL)
                .await
                .context("creating Postgres DevQL tables")
        })
    }

    fn execute<'a>(&'a self, sql: &'a str) -> store_contracts::StoreFuture<'a, ()> {
        Box::pin(async move { postgres_exec(&self.client, sql).await })
    }

    fn query_rows<'a>(&'a self, sql: &'a str) -> store_contracts::StoreFuture<'a, Vec<Value>> {
        Box::pin(async move { pg_query_rows(&self.client, sql).await })
    }
}

#[derive(Debug, Clone)]
struct SqliteRelationalStore {
    db_path: PathBuf,
}

impl SqliteRelationalStore {
    async fn connect(db_path: PathBuf) -> Result<Self> {
        ensure_sqlite_parent_dir(&db_path)?;
        run_sqlite_ping(db_path.clone()).await?;
        Ok(Self { db_path })
    }
}

impl store_contracts::RelationalStore for SqliteRelationalStore {
    fn provider(&self) -> RelationalProvider {
        RelationalProvider::Sqlite
    }

    fn ping<'a>(&'a self) -> store_contracts::StoreFuture<'a, i32> {
        let db_path = self.db_path.clone();
        Box::pin(async move { run_sqlite_ping(db_path).await })
    }

    fn init_schema<'a>(&'a self) -> store_contracts::StoreFuture<'a, ()> {
        let db_path = self.db_path.clone();
        Box::pin(async move {
            sqlite_exec(db_path, RELATIONAL_SCHEMA_SQL.to_string())
                .await
                .context("creating SQLite DevQL tables")
        })
    }

    fn execute<'a>(&'a self, sql: &'a str) -> store_contracts::StoreFuture<'a, ()> {
        let db_path = self.db_path.clone();
        let statement = sql.to_string();
        Box::pin(async move { sqlite_exec(db_path, statement).await })
    }

    fn query_rows<'a>(&'a self, sql: &'a str) -> store_contracts::StoreFuture<'a, Vec<Value>> {
        let db_path = self.db_path.clone();
        let statement = sql.to_string();
        Box::pin(async move { sqlite_query_rows(db_path, statement).await })
    }
}

async fn postgres_exec(pg_client: &tokio_postgres::Client, sql: &str) -> Result<()> {
    run_postgres_exec(pg_client, sql).await
}

async fn pg_query_rows(pg_client: &tokio_postgres::Client, sql: &str) -> Result<Vec<Value>> {
    let wrapped = format!(
        "SELECT coalesce(json_agg(t), '[]'::json)::text FROM ({}) t",
        sql.trim().trim_end_matches(';')
    );
    let raw = run_postgres_query_scalar_text(pg_client, &wrapped).await?;
    let parsed: Value = serde_json::from_str(raw.trim()).with_context(|| {
        format!(
            "parsing Postgres JSON payload failed: {}",
            truncate_for_error(&raw)
        )
    })?;
    match parsed {
        Value::Array(rows) => Ok(rows),
        Value::Object(_) => Ok(vec![parsed]),
        Value::Null => Ok(vec![]),
        other => bail!("unexpected Postgres JSON payload type: {other}"),
    }
}

async fn run_postgres_exec(pg_client: &tokio_postgres::Client, sql: &str) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(30), pg_client.batch_execute(sql))
        .await
        .context("Postgres statement timeout after 30s")?
        .context("executing Postgres statements")?;
    Ok(())
}

async fn run_postgres_query_scalar_text(
    pg_client: &tokio_postgres::Client,
    sql: &str,
) -> Result<String> {
    let row = tokio::time::timeout(Duration::from_secs(30), pg_client.query_one(sql, &[]))
        .await
        .context("Postgres query timeout after 30s")?
        .context("executing Postgres query")?;
    let value: String = row
        .try_get(0)
        .context("reading Postgres scalar text result")?;
    Ok(value)
}

async fn run_postgres_ping(pg_client: &tokio_postgres::Client) -> Result<i32> {
    let row = tokio::time::timeout(Duration::from_secs(10), pg_client.query_one("SELECT 1", &[]))
        .await
        .context("Postgres health query timeout after 10s")?
        .context("running Postgres health query `SELECT 1`")?;
    let value: i32 = row
        .try_get(0)
        .context("reading Postgres health query result")?;
    Ok(value)
}

async fn run_sqlite_ping(db_path: PathBuf) -> Result<i32> {
    tokio::task::spawn_blocking(move || -> Result<i32> {
        let conn = open_sqlite_connection(&db_path)?;
        let value: i32 = conn
            .query_row("SELECT 1", [], |row| row.get(0))
            .context("running SQLite health query `SELECT 1`")?;
        Ok(value)
    })
    .await
    .context("joining SQLite health check task")?
}

async fn sqlite_exec(db_path: PathBuf, sql: String) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = open_sqlite_connection(&db_path)?;
        conn.execute_batch(&sql)
            .with_context(|| format!("executing SQLite statements: {}", truncate_for_error(&sql)))
    })
    .await
    .context("joining SQLite execution task")?
}

async fn sqlite_query_rows(db_path: PathBuf, sql: String) -> Result<Vec<Value>> {
    tokio::task::spawn_blocking(move || -> Result<Vec<Value>> {
        let conn = open_sqlite_connection(&db_path)?;
        let mut stmt = conn
            .prepare(&sql)
            .with_context(|| format!("preparing SQLite query: {}", truncate_for_error(&sql)))?;
        let column_names = stmt
            .column_names()
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>();

        let mut rows = stmt.query([]).context("executing SQLite query")?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().context("reading SQLite row")? {
            let mut object = Map::new();
            for (idx, column_name) in column_names.iter().enumerate() {
                let value = row
                    .get_ref(idx)
                    .with_context(|| format!("reading SQLite column `{column_name}`"))?;
                object.insert(column_name.clone(), sqlite_value_to_json(value));
            }
            out.push(Value::Object(object));
        }

        Ok(out)
    })
    .await
    .context("joining SQLite query task")?
}

fn ensure_sqlite_parent_dir(db_path: &Path) -> Result<()> {
    let parent = db_path
        .parent()
        .filter(|candidate| !candidate.as_os_str().is_empty());
    if let Some(parent) = parent {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating SQLite parent directory {}", parent.display()))?;
    }
    Ok(())
}

fn open_sqlite_connection(db_path: &Path) -> Result<rusqlite::Connection> {
    ensure_sqlite_parent_dir(db_path)?;
    let conn = rusqlite::Connection::open(db_path)
        .with_context(|| format!("opening SQLite database at {}", db_path.display()))?;
    conn.busy_timeout(Duration::from_secs(30))
        .context("setting SQLite busy timeout")?;
    Ok(conn)
}

fn sqlite_value_to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(v) => Value::Number(v.into()),
        ValueRef::Real(v) => serde_json::Number::from_f64(v)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        ValueRef::Text(v) => Value::String(String::from_utf8_lossy(v).to_string()),
        ValueRef::Blob(v) => Value::String(bytes_to_hex(v)),
    }
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn validate_postgres_sslmode_for_notls(dsn: &str, ssl_mode: SslMode) -> Result<()> {
    let dsn_lower = dsn.to_ascii_lowercase();
    if dsn_lower.contains("sslmode=verify-ca") || dsn_lower.contains("sslmode=verify-full") {
        bail!(
            "Postgres DSN requires certificate-based TLS (sslmode=verify-ca/verify-full), \
but this client is configured to use an unencrypted connection (NoTls). \
Use sslmode=disable if plaintext is acceptable, or configure a TLS-enabled Postgres client."
        );
    }

    match ssl_mode {
        SslMode::Disable | SslMode::Prefer => {}
        _ => {
            bail!(
                "Postgres DSN requires TLS (sslmode={:?}), \
but this client is configured to use an unencrypted connection (NoTls). \
Either adjust the DSN to use sslmode=disable if plaintext is acceptable, \
or configure a TLS-enabled Postgres client.",
                ssl_mode
            );
        }
    }
    Ok(())
}

async fn connect_postgres_client(dsn: &str) -> Result<tokio_postgres::Client> {
    let mut pg_cfg: tokio_postgres::Config = dsn.parse().context("parsing Postgres DSN")?;
    validate_postgres_sslmode_for_notls(dsn, pg_cfg.get_ssl_mode())?;

    pg_cfg.connect_timeout(Duration::from_secs(10));

    let (client, connection) = tokio::time::timeout(Duration::from_secs(10), pg_cfg.connect(NoTls))
        .await
        .context("Postgres connect timeout after 10s")?
        .context("connecting to Postgres")?;

    tokio::spawn(async move {
        if let Err(err) = connection.await {
            log::warn!("Postgres connection task ended: {err:#}");
        }
    });

    Ok(client)
}

fn truncate_for_error(input: &str) -> String {
    const MAX: usize = 500;
    let mut out = input.to_string();
    if out.len() > MAX {
        out.truncate(MAX);
        out.push_str("...");
    }
    out
}

