use anyhow::{Context, Result, anyhow, bail};
use reqwest::Url;
use serde_json::{Value, json};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::time::{Duration, timeout};
use tokio_postgres::NoTls;

use crate::config::{StoreBackendConfig, resolve_store_backend_config};

const POSTGRES_POOL_SIZE: usize = 4;
/// Max time allowed per backend health ping so /api/db/health stays responsive.
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendHealthKind {
    Ok,
    Skip,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BackendHealth {
    pub(crate) kind: BackendHealthKind,
    pub(crate) detail: String,
}

impl BackendHealth {
    fn ok(detail: impl Into<String>) -> Self {
        Self {
            kind: BackendHealthKind::Ok,
            detail: detail.into(),
        }
    }

    fn skip(detail: impl Into<String>) -> Self {
        Self {
            kind: BackendHealthKind::Skip,
            detail: detail.into(),
        }
    }

    fn fail(detail: impl Into<String>) -> Self {
        Self {
            kind: BackendHealthKind::Fail,
            detail: detail.into(),
        }
    }

    pub(crate) fn status_label(&self) -> &'static str {
        match self.kind {
            BackendHealthKind::Ok => "OK",
            BackendHealthKind::Skip => "SKIP",
            BackendHealthKind::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DashboardDbHealth {
    pub(crate) relational: BackendHealth,
    pub(crate) events: BackendHealth,
    pub(crate) postgres: BackendHealth,
    pub(crate) clickhouse: BackendHealth,
}

impl DashboardDbHealth {
    fn with_compat_fields(
        relational: BackendHealth,
        events: BackendHealth,
        has_postgres: bool,
        has_clickhouse: bool,
    ) -> Self {
        let postgres = if has_postgres {
            relational.clone()
        } else {
            BackendHealth::skip("inactive compatibility key (relational: sqlite)")
        };
        let clickhouse = if has_clickhouse {
            events.clone()
        } else {
            BackendHealth::skip("inactive compatibility key (events: duckdb)")
        };

        Self {
            relational,
            events,
            postgres,
            clickhouse,
        }
    }

    pub(super) fn has_failures(&self) -> bool {
        self.relational.kind == BackendHealthKind::Fail
            || self.events.kind == BackendHealthKind::Fail
    }
}

#[derive(Debug, Clone)]
pub(super) struct DashboardDbInit {
    pub(super) pools: DashboardDbPools,
    pub(super) startup_health: DashboardDbHealth,
}

#[derive(Clone, Default)]
pub(crate) struct DashboardDbPools {
    pub(super) has_postgres: bool,
    pub(super) has_clickhouse: bool,
    pub(super) postgres: Option<PostgresPool>,
    pub(super) sqlite: Option<SqlitePool>,
    pub(super) clickhouse: Option<ClickHousePool>,
    pub(super) duckdb: Option<DuckDbPool>,
}

impl fmt::Debug for DashboardDbPools {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DashboardDbPools")
            .field("has_postgres", &self.has_postgres)
            .field("has_clickhouse", &self.has_clickhouse)
            .field("postgres_enabled", &self.postgres.is_some())
            .field("sqlite_enabled", &self.sqlite.is_some())
            .field("clickhouse_enabled", &self.clickhouse.is_some())
            .field("duckdb_enabled", &self.duckdb.is_some())
            .finish()
    }
}

impl DashboardDbPools {
    pub(crate) async fn health_check(&self) -> DashboardDbHealth {
        let postgres_pool = self.postgres.clone();
        let sqlite_pool = self.sqlite.clone();
        let clickhouse_pool = self.clickhouse.as_ref();
        let duckdb_pool = self.duckdb.as_ref();

        let relational_fut = async move {
            match (postgres_pool, sqlite_pool) {
                (Some(pool), _) => match timeout(HEALTH_CHECK_TIMEOUT, pool.ping()).await {
                    Ok(Ok(value)) => BackendHealth::ok(format!("SELECT 1 => {value}")),
                    Ok(Err(err)) => BackendHealth::fail(format!("{err:#}")),
                    Err(_) => BackendHealth::fail("health check timed out".to_string()),
                },
                (None, Some(pool)) => match timeout(HEALTH_CHECK_TIMEOUT, pool.ping()).await {
                    Ok(Ok(value)) => BackendHealth::ok(format!("SELECT 1 => {value}")),
                    Ok(Err(err)) => BackendHealth::fail(format!("{err:#}")),
                    Err(_) => BackendHealth::fail("health check timed out".to_string()),
                },
                (None, None) => BackendHealth::skip("not configured"),
            }
        };
        let clickhouse_fut = async move {
            match clickhouse_pool {
                Some(pool) => match timeout(HEALTH_CHECK_TIMEOUT, pool.ping()).await {
                    Ok(Ok(value)) => BackendHealth::ok(format!("SELECT 1 => {value}")),
                    Ok(Err(err)) => BackendHealth::fail(format!("{err:#}")),
                    Err(_) => BackendHealth::fail("health check timed out".to_string()),
                },
                None => BackendHealth::skip("not configured"),
            }
        };
        let duckdb_fut = async move {
            match duckdb_pool {
                Some(pool) => match timeout(HEALTH_CHECK_TIMEOUT, pool.ping()).await {
                    Ok(Ok(value)) => BackendHealth::ok(format!("SELECT 1 => {value}")),
                    Ok(Err(err)) => BackendHealth::fail(format!("{err:#}")),
                    Err(_) => BackendHealth::fail("health check timed out".to_string()),
                },
                None => BackendHealth::skip("not configured"),
            }
        };

        let (relational_backend, clickhouse, duckdb) =
            tokio::join!(relational_fut, clickhouse_fut, duckdb_fut);
        let relational = relational_backend;
        let events = if self.has_clickhouse {
            clickhouse.clone()
        } else {
            duckdb.clone()
        };

        DashboardDbHealth::with_compat_fields(
            relational,
            events,
            self.has_postgres,
            self.has_clickhouse,
        )
    }

    pub(crate) async fn query_sqlite_rows(&self, path: &Path, sql: &str) -> Result<Vec<Value>> {
        let started = Instant::now();
        let pooled = self.sqlite.as_ref().is_some_and(|pool| pool.path() == path);
        let result = if let Some(pool) = self.sqlite.as_ref()
            && pool.path() == path
        {
            pool.query_rows(sql).await
        } else {
            crate::host::devql::sqlite_query_rows_path(path, sql).await
        };
        record_backend_stage(
            "server.db.sqlite.query_rows",
            started.elapsed(),
            json!({
                "path": path.display().to_string(),
                "pooled": pooled,
                "rows": result.as_ref().map(|rows| rows.len()).ok(),
                "sqlBytes": sql.len(),
                "error": result.as_ref().err().map(|err| format!("{err:#}")),
            }),
        );
        result
    }

    pub(crate) async fn execute_sqlite_batch(&self, path: &Path, sql: &str) -> Result<()> {
        let started = Instant::now();
        let pooled = self.sqlite.as_ref().is_some_and(|pool| pool.path() == path);
        let result = if let Some(pool) = self.sqlite.as_ref()
            && pool.path() == path
        {
            pool.execute_batch(sql).await
        } else {
            sqlite_exec_path(path, sql).await
        };
        record_backend_stage(
            "server.db.sqlite.execute_batch",
            started.elapsed(),
            json!({
                "path": path.display().to_string(),
                "pooled": pooled,
                "sqlBytes": sql.len(),
                "error": result.as_ref().err().map(|err| format!("{err:#}")),
            }),
        );
        result
    }

    pub(crate) async fn query_duckdb_rows(&self, path: &Path, sql: &str) -> Result<Vec<Value>> {
        let started = Instant::now();
        let pooled = self.duckdb.as_ref().is_some_and(|pool| pool.path() == path);
        let result = if let Some(pool) = self.duckdb.as_ref()
            && pool.path() == path
        {
            pool.query_rows(sql).await
        } else {
            crate::host::devql::duckdb_query_rows_path(path, sql).await
        };
        record_backend_stage(
            "server.db.duckdb.query_rows",
            started.elapsed(),
            json!({
                "path": path.display().to_string(),
                "pooled": pooled,
                "rows": result.as_ref().map(|rows| rows.len()).ok(),
                "sqlBytes": sql.len(),
                "error": result.as_ref().err().map(|err| format!("{err:#}")),
            }),
        );
        result
    }

    pub(crate) async fn query_clickhouse_data(
        &self,
        cfg: &crate::host::devql::DevqlConfig,
        sql: &str,
    ) -> Result<Value> {
        let started = Instant::now();
        let pooled = self.clickhouse.is_some();
        let result = if let Some(pool) = self.clickhouse.as_ref() {
            pool.query_data(sql).await
        } else {
            crate::host::devql::clickhouse_query_data(cfg, sql).await
        };
        record_backend_stage(
            "server.db.clickhouse.query_data",
            started.elapsed(),
            json!({
                "pooled": pooled,
                "sqlBytes": sql.len(),
                "responseKind": result.as_ref().ok().map(json_value_kind),
                "rowCount": result.as_ref().ok().and_then(|value| value.as_array().map(|rows| rows.len())),
                "error": result.as_ref().err().map(|err| format!("{err:#}")),
            }),
        );
        result
    }
}

fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn record_backend_stage(stage: &str, elapsed: Duration, detail: Value) {
    crate::devql_timing::record_current_stage(stage, elapsed, detail);
}

#[derive(Clone)]
pub(super) struct PostgresPool {
    inner: Arc<PostgresPoolInner>,
}

struct PostgresPoolInner {
    clients: Vec<tokio_postgres::Client>,
    next: AtomicUsize,
}

impl fmt::Debug for PostgresPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgresPool")
            .field("size", &self.inner.clients.len())
            .finish()
    }
}

impl PostgresPool {
    async fn connect(dsn: &str, size: usize) -> Result<Self> {
        let size = size.max(1);
        let mut clients = Vec::with_capacity(size);

        for index in 0..size {
            let (client, connection) = tokio_postgres::connect(dsn, NoTls)
                .await
                .with_context(|| format!("connecting Postgres pool slot {}", index + 1))?;
            tokio::spawn(async move {
                if let Err(err) = connection.await {
                    log::warn!("dashboard Postgres connection task ended: {err:#}");
                }
            });
            clients.push(client);
        }

        Ok(Self {
            inner: Arc::new(PostgresPoolInner {
                clients,
                next: AtomicUsize::new(0),
            }),
        })
    }

    fn pick_client(&self) -> &tokio_postgres::Client {
        let len = self.inner.clients.len();
        let idx = self.inner.next.fetch_add(1, Ordering::Relaxed) % len;
        &self.inner.clients[idx]
    }

    async fn ping(&self) -> Result<i32> {
        let row = self
            .pick_client()
            .query_one("SELECT 1", &[])
            .await
            .context("running Postgres health query `SELECT 1`")?;
        let value: i32 = row
            .try_get(0)
            .context("reading Postgres health query result")?;
        Ok(value)
    }
}

#[derive(Clone)]
pub(super) struct SqlitePool {
    db_path: PathBuf,
    connection: Arc<Mutex<rusqlite::Connection>>,
}

impl fmt::Debug for SqlitePool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SqlitePool")
            .field("db_path", &self.db_path.display().to_string())
            .finish()
    }
}

impl SqlitePool {
    async fn connect(db_path: PathBuf) -> Result<Self> {
        ensure_sqlite_file_exists(&db_path)?;
        let connect_path = db_path.clone();
        let connection = tokio::task::spawn_blocking(move || open_sqlite_connection(&connect_path))
            .await
            .context("joining SQLite connect task")??;
        let pool = Self {
            db_path,
            connection: Arc::new(Mutex::new(connection)),
        };
        let _ = pool.ping().await?;
        Ok(pool)
    }

    fn path(&self) -> &Path {
        &self.db_path
    }

    async fn with_connection<T>(
        &self,
        operation: impl FnOnce(&rusqlite::Connection) -> Result<T> + Send + 'static,
    ) -> Result<T>
    where
        T: Send + 'static,
    {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || -> Result<T> {
            let conn = connection
                .lock()
                .map_err(|err| anyhow!("locking SQLite dashboard connection: {err}"))?;
            operation(&conn)
        })
        .await
        .context("joining SQLite connection task")?
    }

    async fn execute_batch(&self, sql: &str) -> Result<()> {
        let sql = sql.to_string();
        self.with_connection(move |conn| {
            conn.execute_batch(&sql)
                .context("executing SQLite statements")?;
            Ok(())
        })
        .await
    }

    async fn ping(&self) -> Result<i32> {
        self.with_connection(|conn| {
            let value: i32 = conn
                .query_row("SELECT 1", [], |row| row.get(0))
                .context("running SQLite health query `SELECT 1`")?;
            Ok(value)
        })
        .await
    }

    async fn query_rows(&self, sql: &str) -> Result<Vec<Value>> {
        let sql = sql.to_string();
        self.with_connection(move |conn| sqlite_query_rows_with_connection(conn, &sql))
            .await
    }
}

fn ensure_sqlite_file_exists(db_path: &Path) -> Result<()> {
    if db_path.is_file() {
        return Ok(());
    }

    bail!(
        "SQLite database file not found at {}. Run `bitloops init` to create and initialise stores.",
        db_path.display()
    );
}

fn ensure_duckdb_file_exists(path: &Path) -> Result<()> {
    if path.is_file() {
        return Ok(());
    }

    bail!(
        "DuckDB database file not found at {}. Run `bitloops init` to create and initialise stores.",
        path.display()
    );
}

fn open_duckdb_connection_existing(path: &Path) -> Result<duckdb::Connection> {
    ensure_duckdb_file_exists(path)?;
    duckdb::Connection::open(path)
        .with_context(|| format!("opening DuckDB events database at {}", path.display()))
}

fn open_sqlite_connection_existing(db_path: &Path) -> Result<rusqlite::Connection> {
    ensure_sqlite_file_exists(db_path)?;
    rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
        .with_context(|| format!("opening SQLite database at {}", db_path.display()))
}

fn open_sqlite_connection(db_path: &Path) -> Result<rusqlite::Connection> {
    let conn = open_sqlite_connection_existing(db_path)?;
    conn.busy_timeout(std::time::Duration::from_secs(30))
        .context("setting SQLite busy timeout")?;
    Ok(conn)
}

async fn sqlite_exec_path(path: &Path, sql: &str) -> Result<()> {
    let db_path = path.to_path_buf();
    let statement = sql.to_string();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = open_sqlite_connection(&db_path)?;
        conn.execute_batch(&statement)
            .context("executing SQLite statements")?;
        Ok(())
    })
    .await
    .context("joining SQLite execute task")?
}

fn sqlite_query_rows_with_connection(conn: &rusqlite::Connection, sql: &str) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(sql).context("preparing SQLite query")?;
    let column_names = stmt
        .column_names()
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    let mut rows = stmt.query([]).context("executing SQLite query")?;
    let mut out = Vec::new();

    while let Some(row) = rows.next().context("iterating SQLite query rows")? {
        let mut obj = serde_json::Map::new();
        for (idx, column_name) in column_names.iter().enumerate() {
            let value_ref = row.get_ref(idx).with_context(|| {
                format!("reading SQLite value for column index {idx} (`{column_name}`)")
            })?;
            obj.insert(
                column_name.clone(),
                crate::host::devql::sqlite_value_to_json(value_ref),
            );
        }
        out.push(Value::Object(obj));
    }

    Ok(out)
}

fn duckdb_query_rows_with_connection(conn: &duckdb::Connection, sql: &str) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(sql).context("preparing DuckDB query")?;
    let mut rows = stmt.query([]).context("executing DuckDB query")?;
    let column_names = rows
        .as_ref()
        .map(|statement| statement.column_names())
        .unwrap_or_default();
    let mut out = Vec::new();

    while let Some(row) = rows.next().context("iterating DuckDB query rows")? {
        let mut obj = serde_json::Map::new();
        for (idx, column_name) in column_names.iter().enumerate() {
            let value_ref = row.get_ref(idx).with_context(|| {
                format!("reading DuckDB value for column index {idx} (`{column_name}`)")
            })?;
            let owned: duckdb::types::Value = value_ref.to_owned();
            obj.insert(
                column_name.clone(),
                crate::host::devql::duckdb_value_to_json(owned),
            );
        }
        out.push(Value::Object(obj));
    }

    Ok(out)
}

#[derive(Clone)]
pub(super) struct ClickHousePool {
    client: reqwest::Client,
    endpoint: String,
    user: Option<String>,
    password: Option<String>,
}

impl fmt::Debug for ClickHousePool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClickHousePool")
            .field("endpoint", &self.endpoint)
            .field("auth_enabled", &self.user.is_some())
            .finish()
    }
}

impl ClickHousePool {
    fn build(cfg: &ClickHouseConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .pool_max_idle_per_host(16)
            .build()
            .context("building ClickHouse HTTP client")?;

        Ok(Self {
            client,
            endpoint: cfg.endpoint(),
            user: cfg.user.clone(),
            password: cfg.password.clone(),
        })
    }

    async fn run_sql(&self, sql: &str) -> Result<String> {
        let mut request = self.client.post(&self.endpoint).body(sql.to_string());
        if let Some(user) = &self.user {
            request = request.basic_auth(user, Some(self.password.clone().unwrap_or_default()));
        }

        let response = request.send().await.context("sending ClickHouse request")?;
        let status = response.status();
        let body = response
            .text()
            .await
            .context("reading ClickHouse response body")?;
        if !status.is_success() {
            let detail = body.trim();
            if detail.is_empty() {
                bail!("ClickHouse request failed with status {status}");
            }
            bail!("ClickHouse request failed with status {status}: {detail}");
        }

        Ok(body)
    }

    async fn ping(&self) -> Result<i32> {
        let raw = self.run_sql("SELECT 1 FORMAT TabSeparated").await?;
        let value_raw = raw
            .lines()
            .last()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .ok_or_else(|| anyhow!("ClickHouse health query returned an empty response"))?;
        let value = value_raw.parse::<i32>().with_context(|| {
            format!("parsing ClickHouse health query result as integer: {value_raw}")
        })?;
        Ok(value)
    }

    async fn query_data(&self, sql: &str) -> Result<Value> {
        let mut query = sql.trim().to_string();
        if !query.to_ascii_uppercase().contains("FORMAT JSON") {
            query.push_str(" FORMAT JSON");
        }

        let raw = self.run_sql(&query).await?;
        if raw.trim().is_empty() {
            return Ok(Value::Array(vec![]));
        }

        let parsed: Value = serde_json::from_str(&raw)
            .with_context(|| format!("parsing ClickHouse JSON response: {raw}"))?;
        Ok(parsed
            .get("data")
            .cloned()
            .unwrap_or_else(|| Value::Array(vec![])))
    }
}

#[derive(Clone)]
pub(super) struct DuckDbPool {
    path: PathBuf,
    connection: Arc<Mutex<duckdb::Connection>>,
}

impl fmt::Debug for DuckDbPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DuckDbPool")
            .field("path", &self.path.display().to_string())
            .finish()
    }
}

impl DuckDbPool {
    async fn connect(path: PathBuf) -> Result<Self> {
        let connect_path = path.clone();
        let connection =
            tokio::task::spawn_blocking(move || open_duckdb_connection_existing(&connect_path))
                .await
                .context("joining DuckDB connect task")??;
        let pool = Self {
            path,
            connection: Arc::new(Mutex::new(connection)),
        };
        let _ = pool.ping().await?;
        Ok(pool)
    }

    fn path(&self) -> &Path {
        &self.path
    }

    async fn with_connection<T>(
        &self,
        operation: impl FnOnce(&duckdb::Connection) -> Result<T> + Send + 'static,
    ) -> Result<T>
    where
        T: Send + 'static,
    {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || -> Result<T> {
            let conn = connection
                .lock()
                .map_err(|err| anyhow!("locking DuckDB dashboard connection: {err}"))?;
            operation(&conn)
        })
        .await
        .context("joining DuckDB connection task")?
    }

    async fn ping(&self) -> Result<i32> {
        self.with_connection(|conn| {
            let mut stmt = conn
                .prepare("SELECT 1")
                .context("preparing DuckDB health query")?;
            let mut rows = stmt.query([]).context("executing DuckDB health query")?;
            let row = rows
                .next()
                .context("iterating DuckDB health query rows")?
                .ok_or_else(|| anyhow!("DuckDB health query returned no rows"))?;
            let value: i32 = row.get(0).context("reading DuckDB health query result")?;
            Ok(value)
        })
        .await
    }

    async fn query_rows(&self, sql: &str) -> Result<Vec<Value>> {
        let sql = sql.to_string();
        self.with_connection(move |conn| duckdb_query_rows_with_connection(conn, &sql))
            .await
    }
}

#[derive(Debug, Clone)]
struct ClickHouseConfig {
    url: String,
    database: String,
    user: Option<String>,
    password: Option<String>,
}

impl ClickHouseConfig {
    fn endpoint(&self) -> String {
        let base = self.url.trim_end_matches('/');
        let Ok(mut url) = Url::parse(base) else {
            return format!("{base}/?database={}", self.database);
        };
        url.query_pairs_mut()
            .append_pair("database", &self.database);
        url.to_string()
    }
}

#[derive(Debug, Clone)]
struct DashboardDbConfig {
    backends: StoreBackendConfig,
}

impl DashboardDbConfig {
    fn from_env() -> Result<Self> {
        Ok(Self {
            backends: resolve_store_backend_config()?,
        })
    }

    fn clickhouse_config(&self) -> ClickHouseConfig {
        ClickHouseConfig {
            url: self
                .backends
                .events
                .clickhouse_url
                .clone()
                .unwrap_or_else(|| "http://localhost:8123".to_string()),
            database: self
                .backends
                .events
                .clickhouse_database
                .clone()
                .unwrap_or_else(|| "default".to_string()),
            user: self.backends.events.clickhouse_user.clone(),
            password: self.backends.events.clickhouse_password.clone(),
        }
    }

    fn duckdb_path(&self) -> PathBuf {
        self.backends.events.duckdb_path_or_default()
    }

    fn sqlite_db_path(&self) -> Result<PathBuf> {
        self.backends.relational.resolve_sqlite_db_path()
    }
}

pub(super) async fn init_dashboard_db() -> DashboardDbInit {
    let cfg = match DashboardDbConfig::from_env() {
        Ok(cfg) => cfg,
        Err(err) => {
            let failure = BackendHealth::fail(format!("{err:#}"));
            return DashboardDbInit {
                pools: DashboardDbPools::default(),
                startup_health: DashboardDbHealth::with_compat_fields(
                    failure.clone(),
                    failure,
                    false,
                    false,
                ),
            };
        }
    };

    let has_postgres = cfg.backends.relational.has_postgres();
    let has_clickhouse = cfg.backends.events.has_clickhouse();

    let mut pools = DashboardDbPools {
        has_postgres,
        has_clickhouse,
        postgres: None,
        sqlite: None,
        clickhouse: None,
        duckdb: None,
    };
    let relational_health: BackendHealth;
    let events_health: BackendHealth;

    if let Some(dsn) = cfg.backends.relational.postgres_dsn.clone() {
        match PostgresPool::connect(&dsn, POSTGRES_POOL_SIZE).await {
            Ok(pool) => match pool.ping().await {
                Ok(value) => {
                    pools.postgres = Some(pool);
                    relational_health = BackendHealth::ok(format!("SELECT 1 => {value}"));
                }
                Err(err) => {
                    relational_health = BackendHealth::fail(format!("{err:#}"));
                }
            },
            Err(err) => {
                relational_health = BackendHealth::fail(format!("{err:#}"));
            }
        }
    } else {
        match cfg.sqlite_db_path() {
            Ok(db_path) => {
                let db_label = db_path.display().to_string();
                match SqlitePool::connect(db_path).await {
                    Ok(pool) => match pool.ping().await {
                        Ok(value) => {
                            pools.sqlite = Some(pool);
                            relational_health =
                                BackendHealth::ok(format!("SELECT 1 => {value} ({db_label})"));
                        }
                        Err(err) => {
                            relational_health = BackendHealth::fail(format!("{err:#}"));
                        }
                    },
                    Err(err) => {
                        relational_health = BackendHealth::fail(format!("{err:#}"));
                    }
                }
            }
            Err(err) => {
                relational_health = BackendHealth::fail(format!("{err:#}"));
            }
        }
    }

    if has_clickhouse {
        let ch_cfg = cfg.clickhouse_config();
        match ClickHousePool::build(&ch_cfg) {
            Ok(pool) => match pool.ping().await {
                Ok(value) => {
                    pools.clickhouse = Some(pool);
                    events_health = BackendHealth::ok(format!("SELECT 1 => {value}"));
                }
                Err(err) => {
                    events_health = BackendHealth::fail(format!("{err:#}"));
                }
            },
            Err(err) => {
                events_health = BackendHealth::fail(format!("{err:#}"));
            }
        }
    } else {
        let duckdb_path = cfg.duckdb_path();
        match DuckDbPool::connect(duckdb_path).await {
            Ok(pool) => match pool.ping().await {
                Ok(value) => {
                    pools.duckdb = Some(pool);
                    events_health = BackendHealth::ok(format!("SELECT 1 => {value}"));
                }
                Err(err) => {
                    events_health = BackendHealth::fail(format!("{err:#}"));
                }
            },
            Err(err) => {
                events_health = BackendHealth::fail(format!("{err:#}"));
            }
        }
    }

    DashboardDbInit {
        startup_health: DashboardDbHealth::with_compat_fields(
            relational_health,
            events_health,
            has_postgres,
            has_clickhouse,
        ),
        pools,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use serde_json::json;
    use tempfile::tempdir;

    fn ok_health() -> BackendHealth {
        BackendHealth::ok("ok")
    }

    fn skip_health() -> BackendHealth {
        BackendHealth::skip("skip")
    }

    fn fail_health() -> BackendHealth {
        BackendHealth::fail("fail")
    }

    #[test]
    fn backend_health_status_label_ok_skip_fail() {
        assert_eq!(ok_health().status_label(), "OK");
        assert_eq!(skip_health().status_label(), "SKIP");
        assert_eq!(fail_health().status_label(), "FAIL");
    }

    #[test]
    fn dashboard_health_has_failures_true_when_relational_fail() {
        let health = DashboardDbHealth::with_compat_fields(fail_health(), ok_health(), true, true);
        assert!(health.has_failures());
    }

    #[test]
    fn dashboard_health_has_failures_true_when_events_fail() {
        let health = DashboardDbHealth::with_compat_fields(ok_health(), fail_health(), true, true);
        assert!(health.has_failures());
    }

    #[test]
    fn dashboard_health_has_failures_false_when_only_skip_or_ok() {
        let health = DashboardDbHealth::with_compat_fields(ok_health(), skip_health(), true, false);
        assert!(!health.has_failures());
    }

    #[test]
    fn ensure_sqlite_file_exists_missing_file_errors_with_guidance() {
        let dir = tempdir().expect("tempdir");
        let missing = dir.path().join("missing.sqlite");

        let err = ensure_sqlite_file_exists(&missing).expect_err("missing sqlite file must fail");
        let msg = format!("{err:#}");
        assert!(msg.contains("SQLite database file not found"));
        assert!(msg.contains("bitloops init"));
    }

    #[test]
    fn ensure_duckdb_file_exists_missing_file_errors_with_guidance() {
        let dir = tempdir().expect("tempdir");
        let missing = dir.path().join("missing.duckdb");

        let err = ensure_duckdb_file_exists(&missing).expect_err("missing duckdb file must fail");
        let msg = format!("{err:#}");
        assert!(msg.contains("DuckDB database file not found"));
        assert!(msg.contains("bitloops init"));
    }

    #[test]
    fn clickhouse_config_endpoint_handles_trailing_slash() {
        let cfg = ClickHouseConfig {
            url: "http://localhost:8123/".to_string(),
            database: "analytics".to_string(),
            user: None,
            password: None,
        };

        assert_eq!(cfg.endpoint(), "http://localhost:8123/?database=analytics");
    }

    #[test]
    fn clickhouse_config_endpoint_keeps_ipv6_and_appends_database_query() {
        let cfg = ClickHouseConfig {
            url: "http://[::1]:8123".to_string(),
            database: "events".to_string(),
            user: None,
            password: None,
        };

        let endpoint = cfg.endpoint();
        let parsed = Url::parse(&endpoint).expect("endpoint must be a valid URL");
        assert_eq!(parsed.host_str().expect("host must exist"), "[::1]");
        assert_eq!(parsed.port(), Some(8123));
        let params = parsed.query_pairs().collect::<Vec<_>>();
        assert!(params.iter().any(|(k, v)| k == "database" && v == "events"));
    }

    #[test]
    fn open_sqlite_connection_existing_missing_file_errors() {
        let dir = tempdir().expect("tempdir");
        let missing = dir.path().join("missing.sqlite");

        let err = open_sqlite_connection_existing(&missing).expect_err("missing sqlite must fail");
        let msg = format!("{err:#}");
        assert!(msg.contains("SQLite database file not found"));
    }

    #[test]
    fn open_duckdb_connection_existing_missing_file_errors() {
        let dir = tempdir().expect("tempdir");
        let missing = dir.path().join("missing.duckdb");

        let err = open_duckdb_connection_existing(&missing).expect_err("missing duckdb must fail");
        let msg = format!("{err:#}");
        assert!(msg.contains("DuckDB database file not found"));
    }

    #[tokio::test]
    async fn dashboard_db_pools_execute_and_query_sqlite_with_shared_pool() -> Result<()> {
        let dir = tempdir()?;
        let sqlite_path = dir.path().join("shared.sqlite");
        let _ = rusqlite::Connection::open(&sqlite_path)
            .context("creating sqlite file for shared pool test")?;
        let sqlite = SqlitePool::connect(sqlite_path.clone()).await?;
        let pools = DashboardDbPools {
            has_postgres: false,
            has_clickhouse: false,
            postgres: None,
            sqlite: Some(sqlite),
            clickhouse: None,
            duckdb: None,
        };

        pools
            .execute_sqlite_batch(
                &sqlite_path,
                "CREATE TABLE metrics(value INTEGER); INSERT INTO metrics(value) VALUES (1), (2);",
            )
            .await?;
        let rows = pools
            .query_sqlite_rows(&sqlite_path, "SELECT value FROM metrics ORDER BY value")
            .await?;

        assert_eq!(rows, vec![json!({ "value": 1 }), json!({ "value": 2 })]);
        Ok(())
    }

    #[tokio::test]
    async fn dashboard_db_pools_query_duckdb_with_shared_pool() -> Result<()> {
        let dir = tempdir()?;
        let duckdb_path = dir.path().join("events.duckdb");
        let conn =
            duckdb::Connection::open(&duckdb_path).context("creating duckdb file for test")?;
        conn.execute_batch(
            "CREATE TABLE checkpoint_events(value INTEGER); INSERT INTO checkpoint_events(value) VALUES (3), (4);",
        )
        .context("seeding duckdb rows for shared pool test")?;
        drop(conn);

        let duckdb = DuckDbPool::connect(duckdb_path.clone()).await?;
        let pools = DashboardDbPools {
            has_postgres: false,
            has_clickhouse: false,
            postgres: None,
            sqlite: None,
            clickhouse: None,
            duckdb: Some(duckdb),
        };

        let rows = pools
            .query_duckdb_rows(
                &duckdb_path,
                "SELECT value FROM checkpoint_events ORDER BY value",
            )
            .await?;

        assert_eq!(rows, vec![json!({ "value": 3 }), json!({ "value": 4 })]);
        Ok(())
    }
}
