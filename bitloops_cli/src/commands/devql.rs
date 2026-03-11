use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio_postgres::{NoTls, config::SslMode};

use crate::devql_config::{
    DevqlBackendConfig, EventsProvider, RelationalProvider, resolve_devql_backend_config,
};
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
    /// Create schema for configured relational/events providers.
    Init(DevqlInitArgs),
    /// Ingest checkpoint/event data into the configured providers.
    Ingest(DevqlIngestArgs),
    /// Execute an MVP DevQL query.
    Query(DevqlQueryArgs),
    /// Check backend connectivity for configured relational/events providers.
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
    let cfg = DevqlConfig::from_env(repo_root, repo)?;

    match command {
        DevqlCommand::Init(_) => run_init(&cfg).await,
        DevqlCommand::Ingest(ingest_args) => run_ingest(&cfg, &ingest_args).await,
        DevqlCommand::Query(query_args) => run_query(&cfg, &query_args).await,
        DevqlCommand::ConnectionStatus(_) => unreachable!("handled before repo setup"),
    }
}

#[derive(Debug, Clone)]
struct DevqlConnectionConfig {
    backends: DevqlBackendConfig,
}

impl DevqlConnectionConfig {
    fn from_env() -> Result<Self> {
        Ok(Self {
            backends: resolve_devql_backend_config()?,
        })
    }
}

pub async fn run_connection_status() -> Result<()> {
    let cfg = DevqlConnectionConfig::from_env()?;
    let mut rows = Vec::new();

    let relational_status = check_relational_connection_status(&cfg).await;
    rows.push(DatabaseStatusRow {
        db: "Relational",
        status: relational_status,
    });

    let events_status = check_events_connection_status(&cfg).await;
    rows.push(DatabaseStatusRow {
        db: "Events",
        status: events_status,
    });

    print_db_status_table(&rows);

    let failures = rows.iter().filter(|row| row.status.is_failure()).count();
    if failures > 0 {
        bail!("{failures} backend connection check(s) failed");
    }

    Ok(())
}

async fn check_relational_connection_status(
    cfg: &DevqlConnectionConfig,
) -> DatabaseConnectionStatus {
    match cfg.backends.relational.provider {
        RelationalProvider::Sqlite => DatabaseConnectionStatus::NotConfigured,
        RelationalProvider::Postgres => match cfg.backends.relational.postgres_dsn.as_deref() {
            Some(dsn) => match check_postgres_connection(dsn).await {
                Ok(_) => DatabaseConnectionStatus::Connected,
                Err(err) => classify_connection_error(&err.to_string()),
            },
            None => DatabaseConnectionStatus::NotConfigured,
        },
    }
}

async fn check_events_connection_status(cfg: &DevqlConnectionConfig) -> DatabaseConnectionStatus {
    match cfg.backends.events.provider {
        EventsProvider::DuckDb => DatabaseConnectionStatus::NotConfigured,
        EventsProvider::ClickHouse => {
            let clickhouse_endpoint = cfg.backends.events.clickhouse_endpoint();
            match run_clickhouse_sql_http(
                &clickhouse_endpoint,
                cfg.backends.events.clickhouse_user.as_deref(),
                cfg.backends.events.clickhouse_password.as_deref(),
                "SELECT 1 FORMAT TabSeparated",
            )
            .await
            {
                Ok(_) => DatabaseConnectionStatus::Connected,
                Err(err) => classify_connection_error(&err.to_string()),
            }
        }
    }
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
    backends: DevqlBackendConfig,
}

impl DevqlConfig {
    fn from_env(repo_root: PathBuf, repo: RepoIdentity) -> Result<Self> {
        Ok(Self {
            repo_root,
            repo,
            backends: resolve_devql_backend_config()?,
        })
    }

    fn relational_provider(&self) -> RelationalProvider {
        self.backends.relational.provider
    }

    fn events_provider(&self) -> EventsProvider {
        self.backends.events.provider
    }

    fn ensure_postgres_relational_provider(&self) -> Result<()> {
        if self.relational_provider() != RelationalProvider::Postgres {
            bail!(
                "relational provider `{}` is not implemented yet in this build (tracked by CLI-1328); use `postgres` for now",
                self.relational_provider().as_str()
            );
        }
        Ok(())
    }

    fn ensure_clickhouse_events_provider(&self) -> Result<()> {
        if self.events_provider() != EventsProvider::ClickHouse {
            bail!(
                "events provider `{}` is not implemented yet in this build (tracked by CLI-1329); use `clickhouse` for now",
                self.events_provider().as_str()
            );
        }
        Ok(())
    }

    fn require_pg_dsn(&self) -> Result<&str> {
        self.ensure_postgres_relational_provider()?;
        self.backends.relational.postgres_dsn.as_deref().ok_or_else(|| {
            anyhow!(
                "postgres_dsn is required when `devql.relational.provider=postgres` (set `devql.relational.postgres_dsn` or `BITLOOPS_DEVQL_PG_DSN`)"
            )
        })
    }

    fn clickhouse_endpoint(&self) -> Result<String> {
        self.ensure_clickhouse_events_provider()?;
        Ok(self.backends.events.clickhouse_endpoint())
    }

    fn clickhouse_user(&self) -> Option<&str> {
        self.backends.events.clickhouse_user.as_deref()
    }

    fn clickhouse_password(&self) -> Option<&str> {
        self.backends.events.clickhouse_password.as_deref()
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
    let output = execute_query_json(cfg, &args.query).await?;
    if args.compact {
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }

    Ok(())
}

#[allow(dead_code)] // Compiled in both bin/lib crates; used by lib hook runtime path.
pub async fn execute_query_json_for_repo_root(repo_root: &Path, query: &str) -> Result<Value> {
    let repo = resolve_repo_identity(repo_root)?;
    let cfg = DevqlConfig::from_env(repo_root.to_path_buf(), repo)?;
    execute_query_json(&cfg, query).await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueryBackendUsage {
    uses_relational: bool,
    uses_events: bool,
}

fn resolve_query_backend_usage(parsed: &ParsedDevqlQuery) -> QueryBackendUsage {
    if parsed.has_checkpoints_stage || parsed.has_telemetry_stage {
        return QueryBackendUsage {
            uses_relational: false,
            uses_events: true,
        };
    }

    let uses_events = parsed.has_chat_history_stage
        || parsed.artefacts.agent.is_some()
        || parsed.artefacts.since.is_some();
    QueryBackendUsage {
        uses_relational: true,
        uses_events,
    }
}

async fn execute_query_json(cfg: &DevqlConfig, query: &str) -> Result<Value> {
    let parsed = parse_devql_query(query)?;
    let backend_usage = resolve_query_backend_usage(&parsed);

    if backend_usage.uses_relational {
        cfg.ensure_postgres_relational_provider()?;
    }
    if backend_usage.uses_events {
        cfg.ensure_clickhouse_events_provider()?;
    }

    let pg_client = if backend_usage.uses_relational {
        Some(connect_postgres_client(cfg.require_pg_dsn()?).await?)
    } else {
        None
    };
    let mut rows = execute_devql_query(cfg, &parsed, pg_client.as_ref()).await?;

    if !parsed.select_fields.is_empty() {
        rows = project_rows(rows, &parsed.select_fields);
    }

    Ok(Value::Array(rows))
}

#[path = "devql/store_contracts.rs"]
mod store_contracts;

include!("devql/support.rs");

#[cfg(test)]
#[path = "devql_tests.rs"]
mod tests;
