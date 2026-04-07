use anyhow::{Context, Result};
use reqwest::Url;
use serde_json::json;
use std::fs;
use tempfile::tempdir;

use super::config::ClickHouseConfig;
use super::duckdb::{DuckDbPool, ensure_duckdb_file_exists, open_duckdb_connection_existing};
use super::health::{BackendHealth, DashboardDbHealth};
use super::pools::DashboardDbPools;
use super::sqlite::{SqlitePool, ensure_sqlite_file_exists, open_sqlite_connection_existing};

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
    let pools = DashboardDbPools::for_test_backends(Some(sqlite), None);

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
    let conn = duckdb::Connection::open(&duckdb_path).context("creating duckdb file for test")?;
    conn.execute_batch(
        "CREATE TABLE checkpoint_events(value INTEGER); INSERT INTO checkpoint_events(value) VALUES (3), (4);",
    )
    .context("seeding duckdb rows for shared pool test")?;
    drop(conn);

    let duckdb = DuckDbPool::connect(duckdb_path.clone()).await?;
    let pools = DashboardDbPools::for_test_backends(None, Some(duckdb));

    let rows = pools
        .query_duckdb_rows(
            &duckdb_path,
            "SELECT value FROM checkpoint_events ORDER BY value",
        )
        .await?;

    assert_eq!(rows, vec![json!({ "value": 3 }), json!({ "value": 4 })]);
    Ok(())
}

#[tokio::test]
async fn bootstrapped_daemon_store_artifacts_are_openable_by_dashboard_pools() -> Result<()> {
    let dir = tempdir()?;
    let config_path = dir.path().join("config.toml");
    fs::write(
        &config_path,
        r#"
[runtime]
local_dev = false
cli_version = "0.0.12"

[stores.relational]
sqlite_path = "stores/relational/relational.db"

[stores.events]
duckdb_path = "stores/event/events.duckdb"
"#,
    )
    .context("write daemon config")?;

    crate::config::ensure_daemon_store_artifacts(Some(config_path.as_path()))?;

    let sqlite_path = dir.path().join("stores/relational/relational.db");
    let duckdb_path = dir.path().join("stores/event/events.duckdb");

    let sqlite = SqlitePool::connect(sqlite_path).await?;
    let duckdb = DuckDbPool::connect(duckdb_path).await?;

    assert_eq!(sqlite.ping().await?, 1);
    assert_eq!(duckdb.ping().await?, 1);
    Ok(())
}
