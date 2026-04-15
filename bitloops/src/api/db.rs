mod clickhouse;
mod config;
mod duckdb;
mod health;
mod init;
mod pools;
mod postgres;
mod sqlite;

#[cfg(test)]
mod tests;

pub(crate) use self::health::{BackendHealth, BackendHealthKind};
pub(crate) use self::pools::DashboardDbPools;

#[derive(Debug, Clone)]
pub(super) struct DashboardDbInit {
    pub(super) pools: DashboardDbPools,
    pub(super) startup_health: health::DashboardDbHealth,
}

pub(super) async fn init_dashboard_db() -> DashboardDbInit {
    self::init::init_dashboard_db().await
}

const POSTGRES_POOL_SIZE: usize = 4;
/// Max time allowed per backend health ping so dashboard health queries stay responsive.
const HEALTH_CHECK_TIMEOUT: tokio::time::Duration = tokio::time::Duration::from_secs(10);
