use anyhow::{Context, Result, anyhow, bail};
use reqwest::Url;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::time::{Duration, timeout};
use tokio_postgres::NoTls;

use crate::config::{StoreBackendConfig, resolve_store_backend_config};

const POSTGRES_POOL_SIZE: usize = 4;
/// Max time allowed per backend health ping so /api/db/health stays responsive.
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
type RelationalHealthFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

pub(super) trait RelationalHealthStore: Send + Sync {
    fn ping<'a>(&'a self) -> RelationalHealthFuture<'a, i32>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BackendHealthKind {
    Ok,
    Skip,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BackendHealth {
    pub(super) kind: BackendHealthKind,
    pub(super) detail: String,
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

    pub(super) fn status_label(&self) -> &'static str {
        match self.kind {
            BackendHealthKind::Ok => "OK",
            BackendHealthKind::Skip => "SKIP",
            BackendHealthKind::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DashboardDbHealth {
    pub(super) relational: BackendHealth,
    pub(super) events: BackendHealth,
    pub(super) postgres: BackendHealth,
    pub(super) clickhouse: BackendHealth,
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
    pub(super) relational: Option<Arc<dyn RelationalHealthStore>>,
    pub(super) clickhouse: Option<ClickHousePool>,
    pub(super) duckdb: Option<DuckDbPool>,
}

impl fmt::Debug for DashboardDbPools {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DashboardDbPools")
            .field("has_postgres", &self.has_postgres)
            .field("has_clickhouse", &self.has_clickhouse)
            .field("relational_enabled", &self.relational.is_some())
            .field("clickhouse_enabled", &self.clickhouse.is_some())
            .field("duckdb_enabled", &self.duckdb.is_some())
            .finish()
    }
}

impl DashboardDbPools {
    pub(super) async fn health_check(&self) -> DashboardDbHealth {
        let relational_store = self.relational.clone();
        let clickhouse_pool = self.clickhouse.as_ref();
        let duckdb_pool = self.duckdb.as_ref();

        let relational_fut = async move {
            match relational_store {
                Some(store) => match timeout(HEALTH_CHECK_TIMEOUT, store.ping()).await {
                    Ok(Ok(value)) => BackendHealth::ok(format!("SELECT 1 => {value}")),
                    Ok(Err(err)) => BackendHealth::fail(format!("{err:#}")),
                    Err(_) => BackendHealth::fail("health check timed out".to_string()),
                },
                None => BackendHealth::skip("not configured"),
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

    async fn ping_inner(&self) -> Result<i32> {
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

impl RelationalHealthStore for PostgresPool {
    fn ping<'a>(&'a self) -> RelationalHealthFuture<'a, i32> {
        Box::pin(async move { self.ping_inner().await })
    }
}

#[derive(Clone)]
pub(super) struct SqlitePool {
    db_path: PathBuf,
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
        let pool = Self { db_path };
        let _ = pool.ping_inner().await?;
        Ok(pool)
    }

    async fn ping_inner(&self) -> Result<i32> {
        let db_path = self.db_path.clone();
        tokio::task::spawn_blocking(move || -> Result<i32> {
            let conn = open_sqlite_connection(&db_path)?;
            let value: i32 = conn
                .query_row("SELECT 1", [], |row| row.get(0))
                .context("running SQLite health query `SELECT 1`")?;
            Ok(value)
        })
        .await
        .context("joining SQLite health query task")?
    }
}

impl RelationalHealthStore for SqlitePool {
    fn ping<'a>(&'a self) -> RelationalHealthFuture<'a, i32> {
        Box::pin(async move { self.ping_inner().await })
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

fn open_duckdb_connection_for_health(path: &Path) -> Result<duckdb::Connection> {
    ensure_duckdb_file_exists(path)?;
    duckdb::Connection::open(path).with_context(|| {
        format!(
            "opening DuckDB events database for health check at {}",
            path.display()
        )
    })
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
}

#[derive(Clone)]
pub(super) struct DuckDbPool {
    path: PathBuf,
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
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = open_duckdb_connection_existing(&connect_path)?;
            conn.execute_batch("SELECT 1")
                .context("running initial DuckDB connectivity check")?;
            Ok(())
        })
        .await
        .context("joining DuckDB initial connectivity task")??;

        Ok(Self { path })
    }

    async fn ping(&self) -> Result<i32> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || -> Result<i32> {
            let conn = open_duckdb_connection_for_health(&path)?;
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
        .context("joining DuckDB health query task")?
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
        relational: None,
        clickhouse: None,
        duckdb: None,
    };
    let relational_health: BackendHealth;
    let events_health: BackendHealth;

    if let Some(dsn) = cfg.backends.relational.postgres_dsn.clone() {
        match PostgresPool::connect(&dsn, POSTGRES_POOL_SIZE).await {
            Ok(pool) => {
                let relational_store: Arc<dyn RelationalHealthStore> = Arc::new(pool);
                match relational_store.ping().await {
                    Ok(value) => {
                        pools.relational = Some(relational_store);
                        relational_health = BackendHealth::ok(format!("SELECT 1 => {value}"));
                    }
                    Err(err) => {
                        relational_health = BackendHealth::fail(format!("{err:#}"));
                    }
                }
            }
            Err(err) => {
                relational_health = BackendHealth::fail(format!("{err:#}"));
            }
        }
    } else {
        match cfg.sqlite_db_path() {
            Ok(db_path) => {
                let db_label = db_path.display().to_string();
                match SqlitePool::connect(db_path).await {
                    Ok(pool) => {
                        let relational_store: Arc<dyn RelationalHealthStore> = Arc::new(pool);
                        match relational_store.ping().await {
                            Ok(value) => {
                                pools.relational = Some(relational_store);
                                relational_health =
                                    BackendHealth::ok(format!("SELECT 1 => {value} ({db_label})"));
                            }
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
