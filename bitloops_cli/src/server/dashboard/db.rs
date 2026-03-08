use anyhow::{Context, Result, anyhow, bail};
use reqwest::Url;
use std::env;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::time::{Duration, timeout};
use tokio_postgres::NoTls;

use crate::devql_config::DevqlFileConfig;

const POSTGRES_POOL_SIZE: usize = 4;
/// Max time allowed per backend health ping so /api/db/health stays responsive.
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

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
    pub(super) postgres: BackendHealth,
    pub(super) clickhouse: BackendHealth,
}

impl DashboardDbHealth {
    pub(super) fn has_failures(&self) -> bool {
        self.postgres.kind == BackendHealthKind::Fail
            || self.clickhouse.kind == BackendHealthKind::Fail
    }
}

#[derive(Debug, Clone)]
pub(super) struct DashboardDbInit {
    pub(super) pools: DashboardDbPools,
    pub(super) startup_health: DashboardDbHealth,
}

#[derive(Clone, Default)]
pub(in crate::server) struct DashboardDbPools {
    pub(super) postgres: Option<PostgresPool>,
    pub(super) clickhouse: Option<ClickHousePool>,
}

impl fmt::Debug for DashboardDbPools {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DashboardDbPools")
            .field("postgres_enabled", &self.postgres.is_some())
            .field("clickhouse_enabled", &self.clickhouse.is_some())
            .finish()
    }
}

impl DashboardDbPools {
    pub(super) async fn health_check(&self) -> DashboardDbHealth {
        let postgres_pool = self.postgres.as_ref();
        let clickhouse_pool = self.clickhouse.as_ref();

        let postgres_fut = async move {
            match postgres_pool {
                Some(pool) => match timeout(HEALTH_CHECK_TIMEOUT, pool.ping()).await {
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

        let (postgres, clickhouse) = tokio::join!(postgres_fut, clickhouse_fut);

        DashboardDbHealth {
            postgres,
            clickhouse,
        }
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
    postgres_dsn: Option<String>,
    clickhouse: Option<ClickHouseConfig>,
}

impl DashboardDbConfig {
    fn from_env() -> Self {
        let file_cfg = DevqlFileConfig::load();

        let env_pg = env::var("BITLOOPS_DEVQL_PG_DSN")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let postgres_dsn = env_pg.or(file_cfg.pg_dsn);

        let env_ch_url = env::var("BITLOOPS_DEVQL_CH_URL")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let env_ch_db = env::var("BITLOOPS_DEVQL_CH_DATABASE")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let env_ch_user = env::var("BITLOOPS_DEVQL_CH_USER")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let env_ch_password = env::var("BITLOOPS_DEVQL_CH_PASSWORD")
            .ok()
            .filter(|s| !s.trim().is_empty());

        let clickhouse_explicit = env_ch_url.is_some()
            || env_ch_db.is_some()
            || env_ch_user.is_some()
            || env_ch_password.is_some()
            || file_cfg.clickhouse_url.is_some()
            || file_cfg.clickhouse_database.is_some()
            || file_cfg.clickhouse_user.is_some()
            || file_cfg.clickhouse_password.is_some();

        let clickhouse = if clickhouse_explicit {
            Some(ClickHouseConfig {
                url: env_ch_url
                    .or(file_cfg.clickhouse_url)
                    .unwrap_or_else(|| "http://localhost:8123".to_string()),
                database: env_ch_db
                    .or(file_cfg.clickhouse_database)
                    .unwrap_or_else(|| "default".to_string()),
                user: env_ch_user.or(file_cfg.clickhouse_user),
                password: env_ch_password.or(file_cfg.clickhouse_password),
            })
        } else {
            None
        };

        Self {
            postgres_dsn,
            clickhouse,
        }
    }
}

pub(super) async fn init_dashboard_db() -> DashboardDbInit {
    let cfg = DashboardDbConfig::from_env();
    let mut pools = DashboardDbPools::default();
    let mut postgres_health = BackendHealth::skip("not configured");
    let mut clickhouse_health = BackendHealth::skip("not configured");

    if let Some(dsn) = cfg.postgres_dsn {
        match PostgresPool::connect(&dsn, POSTGRES_POOL_SIZE).await {
            Ok(pool) => match pool.ping().await {
                Ok(value) => {
                    pools.postgres = Some(pool);
                    postgres_health = BackendHealth::ok(format!("SELECT 1 => {value}"));
                }
                Err(err) => {
                    postgres_health = BackendHealth::fail(format!("{err:#}"));
                }
            },
            Err(err) => {
                postgres_health = BackendHealth::fail(format!("{err:#}"));
            }
        }
    }

    if let Some(ch_cfg) = cfg.clickhouse {
        match ClickHousePool::build(&ch_cfg) {
            Ok(pool) => match pool.ping().await {
                Ok(value) => {
                    pools.clickhouse = Some(pool);
                    clickhouse_health = BackendHealth::ok(format!("SELECT 1 => {value}"));
                }
                Err(err) => {
                    clickhouse_health = BackendHealth::fail(format!("{err:#}"));
                }
            },
            Err(err) => {
                clickhouse_health = BackendHealth::fail(format!("{err:#}"));
            }
        }
    }

    DashboardDbInit {
        pools,
        startup_health: DashboardDbHealth {
            postgres: postgres_health,
            clickhouse: clickhouse_health,
        },
    }
}
