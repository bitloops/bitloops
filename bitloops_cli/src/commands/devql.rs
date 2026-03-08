use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio_postgres::{NoTls, config::SslMode};

use crate::devql_config::DevqlFileConfig;
use crate::engine::db_status::{
    DatabaseConnectionStatus, DatabaseStatusRow, classify_connection_error,
};
use crate::engine::paths;
use crate::engine::strategy::manual_commit::{
    CommittedInfo, list_committed, read_committed, read_session_content, run_git,
};
use crate::engine::trailers::{CHECKPOINT_TRAILER_KEY, is_valid_checkpoint_id};
use crate::terminal::db_status_table::print_db_status_table;

#[derive(Args, Debug, Clone, Default)]
pub struct DevqlArgs {
    #[command(subcommand)]
    pub command: Option<DevqlCommand>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DevqlCommand {
    /// Create ClickHouse/Postgres schema required by DevQL MVP.
    Init(DevqlInitArgs),
    /// Ingest checkpoint/event data into ClickHouse and file artefacts into Postgres.
    Ingest(DevqlIngestArgs),
    /// Execute an MVP DevQL query.
    Query(DevqlQueryArgs),
    /// Check backend connectivity for Postgres and ClickHouse.
    ConnectionStatus(DevqlConnectionStatusArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct DevqlInitArgs {}

#[derive(Args, Debug, Clone)]
pub struct DevqlIngestArgs {
    /// Bootstrap tables before ingestion.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub init: bool,

    /// Limit checkpoints processed (newest-first).
    #[arg(long, default_value_t = 500)]
    pub max_checkpoints: usize,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlQueryArgs {
    /// DevQL pipeline query string.
    pub query: String,

    /// Print compact JSON.
    #[arg(long, default_value_t = false)]
    pub compact: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DevqlConnectionStatusArgs {}

pub async fn run(args: DevqlArgs) -> Result<()> {
    let Some(command) = args.command else {
        bail!(
            "missing subcommand. Use one of: `bitloops devql init`, `bitloops devql ingest`, `bitloops devql query`, `bitloops devql connection-status`"
        );
    };

    if matches!(&command, DevqlCommand::ConnectionStatus(_)) {
        return run_connection_status().await;
    }

    let repo_root = paths::repo_root()?;
    let repo = resolve_repo_identity(&repo_root)?;
    let cfg = DevqlConfig::from_env(repo_root, repo);

    match command {
        DevqlCommand::Init(_) => run_init(&cfg).await,
        DevqlCommand::Ingest(ingest_args) => run_ingest(&cfg, &ingest_args).await,
        DevqlCommand::Query(query_args) => run_query(&cfg, &query_args).await,
        DevqlCommand::ConnectionStatus(_) => unreachable!("handled before repo setup"),
    }
}

#[derive(Debug, Clone)]
struct DevqlConnectionConfig {
    pg_dsn: Option<String>,
    clickhouse_url: String,
    clickhouse_user: Option<String>,
    clickhouse_password: Option<String>,
    clickhouse_database: String,
}

impl DevqlConnectionConfig {
    fn from_env() -> Self {
        let file_cfg = DevqlFileConfig::load();
        Self {
            pg_dsn: env::var("BITLOOPS_DEVQL_PG_DSN")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .or(file_cfg.pg_dsn),
            clickhouse_url: env::var("BITLOOPS_DEVQL_CH_URL")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .or(file_cfg.clickhouse_url)
                .unwrap_or_else(|| "http://localhost:8123".to_string()),
            clickhouse_user: env::var("BITLOOPS_DEVQL_CH_USER")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .or(file_cfg.clickhouse_user),
            clickhouse_password: env::var("BITLOOPS_DEVQL_CH_PASSWORD")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .or(file_cfg.clickhouse_password),
            clickhouse_database: env::var("BITLOOPS_DEVQL_CH_DATABASE")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .or(file_cfg.clickhouse_database)
                .unwrap_or_else(|| "default".to_string()),
        }
    }

    fn clickhouse_endpoint(&self) -> String {
        let base = self.clickhouse_url.trim_end_matches('/');
        format!("{base}/?database={}", self.clickhouse_database)
    }
}

pub async fn run_connection_status() -> Result<()> {
    let cfg = DevqlConnectionConfig::from_env();
    let mut rows = Vec::new();

    let postgres_status = match cfg.pg_dsn.as_deref() {
        Some(dsn) => match check_postgres_connection(dsn).await {
            Ok(_) => DatabaseConnectionStatus::Connected,
            Err(err) => classify_connection_error(&err.to_string()),
        },
        None => DatabaseConnectionStatus::NotConfigured,
    };
    rows.push(DatabaseStatusRow {
        db: "Postgres",
        status: postgres_status,
    });

    let clickhouse_endpoint = cfg.clickhouse_endpoint();
    let clickhouse_status = match run_clickhouse_sql_http(
        &clickhouse_endpoint,
        cfg.clickhouse_user.as_deref(),
        cfg.clickhouse_password.as_deref(),
        "SELECT 1 FORMAT TabSeparated",
    )
    .await
    {
        Ok(_) => DatabaseConnectionStatus::Connected,
        Err(err) => classify_connection_error(&err.to_string()),
    };
    rows.push(DatabaseStatusRow {
        db: "ClickHouse",
        status: clickhouse_status,
    });

    print_db_status_table(&rows);

    let failures = rows.iter().filter(|row| row.status.is_failure()).count();
    if failures > 0 {
        bail!("{failures} backend connection check(s) failed");
    }

    Ok(())
}

async fn check_postgres_connection(dsn: &str) -> Result<()> {
    let client = connect_postgres_client(dsn).await?;

    let row = tokio::time::timeout(Duration::from_secs(10), client.query_one("SELECT 1", &[]))
        .await
        .context("Postgres health query timeout after 10s")?
        .context("running Postgres health query `SELECT 1`")?;
    let value: i32 = row
        .try_get(0)
        .context("reading Postgres health query result")?;
    if value != 1 {
        bail!("unexpected Postgres health query result: {value}");
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct RepoIdentity {
    provider: String,
    organization: String,
    name: String,
    identity: String,
    repo_id: String,
}

#[derive(Debug, Clone)]
struct DevqlConfig {
    repo_root: PathBuf,
    repo: RepoIdentity,
    pg_dsn: Option<String>,
    clickhouse_url: String,
    clickhouse_user: Option<String>,
    clickhouse_password: Option<String>,
    clickhouse_database: String,
}

impl DevqlConfig {
    fn from_env(repo_root: PathBuf, repo: RepoIdentity) -> Self {
        let file_cfg = DevqlFileConfig::load();
        Self {
            repo_root,
            repo,
            pg_dsn: env::var("BITLOOPS_DEVQL_PG_DSN")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .or(file_cfg.pg_dsn),
            clickhouse_url: env::var("BITLOOPS_DEVQL_CH_URL")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .or(file_cfg.clickhouse_url)
                .unwrap_or_else(|| "http://localhost:8123".to_string()),
            clickhouse_user: env::var("BITLOOPS_DEVQL_CH_USER")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .or(file_cfg.clickhouse_user),
            clickhouse_password: env::var("BITLOOPS_DEVQL_CH_PASSWORD")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .or(file_cfg.clickhouse_password),
            clickhouse_database: env::var("BITLOOPS_DEVQL_CH_DATABASE")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .or(file_cfg.clickhouse_database)
                .unwrap_or_else(|| "default".to_string()),
        }
    }

    fn require_pg_dsn(&self) -> Result<&str> {
        self.pg_dsn.as_deref().ok_or_else(|| {
            anyhow!(
                "BITLOOPS_DEVQL_PG_DSN is required for Postgres operations (example: postgres://user:pass@localhost:5432/bitloops)"
            )
        })
    }

    fn clickhouse_endpoint(&self) -> String {
        let base = self.clickhouse_url.trim_end_matches('/');
        format!("{base}/?database={}", self.clickhouse_database)
    }
}

async fn run_init(cfg: &DevqlConfig) -> Result<()> {
    let pg_client = connect_postgres_client(cfg.require_pg_dsn()?).await?;
    init_clickhouse_schema(cfg).await?;
    init_postgres_schema(cfg, &pg_client).await?;

    println!(
        "DevQL schema ready for repo {} ({})",
        cfg.repo.identity, cfg.repo.repo_id
    );
    Ok(())
}

async fn run_ingest(cfg: &DevqlConfig, args: &DevqlIngestArgs) -> Result<()> {
    let pg_client = connect_postgres_client(cfg.require_pg_dsn()?).await?;
    if args.init {
        init_clickhouse_schema(cfg).await?;
        init_postgres_schema(cfg, &pg_client).await?;
    }

    ensure_repository_row(cfg, &pg_client).await?;

    let mut checkpoints = list_committed(&cfg.repo_root)?;
    checkpoints.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    if args.max_checkpoints > 0 && checkpoints.len() > args.max_checkpoints {
        checkpoints.truncate(args.max_checkpoints);
    }

    let commit_map = collect_checkpoint_commit_map(&cfg.repo_root)?;
    let mut existing_event_ids = fetch_existing_checkpoint_event_ids(cfg).await?;

    let mut counters = IngestionCounters::default();

    for cp in checkpoints {
        let commit_info = commit_map.get(&cp.checkpoint_id);
        let commit_sha = commit_info
            .map(|info| info.commit_sha.clone())
            .unwrap_or_default();
        let event_id = deterministic_uuid(&format!(
            "{}|{}|{}|checkpoint_committed",
            cfg.repo.repo_id, cp.checkpoint_id, cp.session_id
        ));

        if !existing_event_ids.contains(&event_id) {
            insert_checkpoint_event(cfg, &cp, &event_id, commit_info).await?;
            existing_event_ids.insert(event_id);
            counters.events_inserted += 1;
        }

        if commit_sha.is_empty() {
            counters.checkpoints_without_commit += 1;
            continue;
        }

        upsert_commit_row(
            cfg,
            &pg_client,
            &cp,
            commit_info.expect("commit_info exists when sha exists"),
        )
        .await?;

        for path in &cp.files_touched {
            let normalized_path = normalize_repo_path(path);
            if normalized_path.is_empty() {
                continue;
            }

            let blob_sha = git_blob_sha_at_commit(&cfg.repo_root, &commit_sha, &normalized_path)
                .or_else(|| git_blob_sha_at_commit(&cfg.repo_root, &commit_sha, path));
            let Some(blob_sha) = blob_sha else {
                continue;
            };

            upsert_file_state_row(cfg, &pg_client, &commit_sha, &normalized_path, &blob_sha)
                .await?;
            let file_artefact =
                upsert_file_artefact_row(cfg, &pg_client, &normalized_path, &blob_sha).await?;
            upsert_language_artefacts(cfg, &pg_client, &normalized_path, &blob_sha, &file_artefact)
                .await?;
            counters.artefacts_upserted += 1;
        }

        counters.checkpoints_processed += 1;
    }

    println!(
        "DevQL ingest complete: checkpoints_processed={}, events_inserted={}, artefacts_upserted={}, checkpoints_without_commit={}",
        counters.checkpoints_processed,
        counters.events_inserted,
        counters.artefacts_upserted,
        counters.checkpoints_without_commit
    );
    Ok(())
}

async fn run_query(cfg: &DevqlConfig, args: &DevqlQueryArgs) -> Result<()> {
    let parsed = parse_devql_query(&args.query)?;
    let pg_client = if parsed.has_checkpoints_stage || parsed.has_telemetry_stage {
        None
    } else {
        Some(connect_postgres_client(cfg.require_pg_dsn()?).await?)
    };
    let mut rows = execute_devql_query(cfg, &parsed, pg_client.as_ref()).await?;

    if !parsed.select_fields.is_empty() {
        rows = project_rows(rows, &parsed.select_fields);
    }

    let output = Value::Array(rows);
    if args.compact {
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }

    Ok(())
}

include!("devql/support.rs");

#[cfg(test)]
#[path = "devql_tests.rs"]
mod tests;
