use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio_postgres::{NoTls, config::SslMode};

use crate::engine::db_status::{
    DatabaseConnectionStatus, DatabaseStatusRow, classify_connection_error,
};
use crate::engine::semantic_features as semantic;
use crate::engine::strategy::manual_commit::{
    CommittedInfo, list_committed, read_commit_checkpoint_mappings, read_committed,
    read_session_content, run_git,
};
use crate::store_config::{
    EventsBackendConfig, EventsProvider, RelationalBackendConfig, RelationalProvider,
    StoreBackendConfig, resolve_store_backend_config, resolve_store_backend_config_for_repo,
    resolve_store_semantic_config,
};
use crate::terminal::db_status_table::print_db_status_table;

pub mod watch;

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
}

impl DevqlConfig {
    pub fn from_env(repo_root: PathBuf, repo: RepoIdentity) -> Result<Self> {
        let backend_cfg = resolve_store_backend_config_for_repo(&repo_root)
            .context("resolving backend config for DevQL runtime")?;
        let semantic_cfg = resolve_store_semantic_config();
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
        })
    }

    fn clickhouse_endpoint(&self) -> String {
        let base = self.clickhouse_url.trim_end_matches('/');
        format!("{base}/?database={}", self.clickhouse_database)
    }
}

const RELATIONAL_SQLITE_LABEL: &str = "Relational (SQLite)";
const RELATIONAL_POSTGRES_LABEL: &str = "Relational (Postgres)";
const EVENTS_DUCKDB_LABEL: &str = "Events (DuckDB)";
const EVENTS_CLICKHOUSE_LABEL: &str = "Events (ClickHouse)";
pub(crate) const DEVQL_POSTGRES_DSN_REQUIRED_PREFIX: &str = "DevQL Postgres DSN is required";

pub async fn run_connection_status() -> Result<()> {
    let cfg = resolve_store_backend_config()?;
    let rows = collect_connection_status_rows(&cfg).await;

    print_db_status_table(&rows);

    let failures = rows.iter().filter(|row| row.status.is_failure()).count();
    if failures > 0 {
        bail!("{failures} backend connection check(s) failed");
    }

    Ok(())
}

async fn collect_connection_status_rows(cfg: &StoreBackendConfig) -> Vec<DatabaseStatusRow> {
    vec![
        DatabaseStatusRow {
            db: relational_status_label(&cfg.relational),
            status: relational_connection_status(&cfg.relational).await,
        },
        DatabaseStatusRow {
            db: events_status_label(&cfg.events),
            status: events_connection_status(&cfg.events).await,
        },
    ]
}

fn relational_status_label(cfg: &RelationalBackendConfig) -> &'static str {
    match cfg.provider {
        RelationalProvider::Sqlite => RELATIONAL_SQLITE_LABEL,
        RelationalProvider::Postgres => RELATIONAL_POSTGRES_LABEL,
    }
}

fn events_status_label(cfg: &EventsBackendConfig) -> &'static str {
    match cfg.provider {
        EventsProvider::DuckDb => EVENTS_DUCKDB_LABEL,
        EventsProvider::ClickHouse => EVENTS_CLICKHOUSE_LABEL,
    }
}

async fn relational_connection_status(cfg: &RelationalBackendConfig) -> DatabaseConnectionStatus {
    match cfg.provider {
        RelationalProvider::Sqlite => match cfg.resolve_sqlite_db_path() {
            Ok(path) => match check_sqlite_connection(&path).await {
                Ok(_) => DatabaseConnectionStatus::Connected,
                Err(err) => classify_connection_error(&err.to_string()),
            },
            Err(err) => classify_connection_error(&err.to_string()),
        },
        RelationalProvider::Postgres => match cfg.postgres_dsn.as_deref() {
            Some(dsn) => match check_postgres_connection(dsn).await {
                Ok(_) => DatabaseConnectionStatus::Connected,
                Err(err) => classify_connection_error(&err.to_string()),
            },
            None => DatabaseConnectionStatus::NotConfigured,
        },
    }
}

async fn events_connection_status(cfg: &EventsBackendConfig) -> DatabaseConnectionStatus {
    match cfg.provider {
        EventsProvider::DuckDb => {
            let duckdb_path = cfg.duckdb_path_or_default();
            match check_duckdb_connection(&duckdb_path).await {
                Ok(_) => DatabaseConnectionStatus::Connected,
                Err(err) => classify_connection_error(&err.to_string()),
            }
        }
        EventsProvider::ClickHouse => {
            let clickhouse_url = cfg
                .clickhouse_url
                .clone()
                .unwrap_or_else(|| "http://localhost:8123".to_string());
            let clickhouse_database = cfg
                .clickhouse_database
                .clone()
                .unwrap_or_else(|| "default".to_string());
            let clickhouse_endpoint = clickhouse_endpoint(&clickhouse_url, &clickhouse_database);
            match run_clickhouse_sql_http(
                &clickhouse_endpoint,
                cfg.clickhouse_user.as_deref(),
                cfg.clickhouse_password.as_deref(),
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

async fn check_sqlite_connection(path: &Path) -> Result<()> {
    let db_path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .with_context(|| format!("opening SQLite database at {}", db_path.display()))?;

        let value: i32 = conn
            .query_row("SELECT 1", [], |row| row.get(0))
            .context("running SQLite health query `SELECT 1`")?;
        if value != 1 {
            bail!("unexpected SQLite health query result: {value}");
        }

        Ok(())
    })
    .await
    .context("joining SQLite health query task")?
}

async fn check_duckdb_connection(path: &Path) -> Result<()> {
    let db_path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        if !db_path.is_file() {
            bail!(
                "DuckDB database file not found at {}. Run `bitloops init` to create and initialise stores.",
                db_path.display()
            );
        }
        let conn = duckdb::Connection::open(&db_path)
            .with_context(|| format!("opening DuckDB events database at {}", db_path.display()))?;
        conn.execute_batch("SELECT 1")
            .context("running DuckDB health query `SELECT 1`")?;
        Ok(())
    })
    .await
    .context("joining DuckDB health query task")?
}

fn clickhouse_endpoint(url: &str, database: &str) -> String {
    let base = url.trim_end_matches('/');
    format!("{base}/?database={database}")
}

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

fn semantic_provider_config(cfg: &DevqlConfig) -> semantic::SemanticSummaryProviderConfig {
    semantic::SemanticSummaryProviderConfig {
        semantic_provider: cfg.semantic_provider.clone(),
        semantic_model: cfg.semantic_model.clone(),
        semantic_api_key: cfg.semantic_api_key.clone(),
        semantic_base_url: cfg.semantic_base_url.clone(),
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

pub async fn run_init(cfg: &DevqlConfig) -> Result<()> {
    let backends = resolve_store_backend_config_for_repo(&cfg.repo_root)
        .context("resolving DevQL backend config for `devql init`")?;
    let relational = RelationalStorage::connect(cfg, &backends.relational, "devql init").await?;

    match backends.events.provider {
        EventsProvider::ClickHouse => init_clickhouse_schema(cfg).await?,
        EventsProvider::DuckDb => init_duckdb_schema(&backends.events).await?,
    }

    init_relational_schema(cfg, &relational).await?;

    println!(
        "DevQL schema ready for repo {} ({})",
        cfg.repo.identity, cfg.repo.repo_id
    );
    Ok(())
}

pub async fn run_ingest(cfg: &DevqlConfig, init: bool, max_checkpoints: usize) -> Result<()> {
    let backends = resolve_store_backend_config_for_repo(&cfg.repo_root)
        .context("resolving DevQL backend config for `devql ingest`")?;
    let relational = RelationalStorage::connect(cfg, &backends.relational, "devql ingest").await?;
    let summary_provider: Arc<dyn semantic::SemanticSummaryProvider> =
        semantic::build_semantic_summary_provider(&semantic_provider_config(cfg))?.into();

    if init {
        match backends.events.provider {
            EventsProvider::ClickHouse => init_clickhouse_schema(cfg).await?,
            EventsProvider::DuckDb => init_duckdb_schema(&backends.events).await?,
        }
        init_relational_schema(cfg, &relational).await?;
    }

    ensure_repository_row(cfg, &relational).await?;

    let mut checkpoints = list_committed(&cfg.repo_root)?;
    checkpoints.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    if max_checkpoints > 0 && checkpoints.len() > max_checkpoints {
        checkpoints.truncate(max_checkpoints);
    }

    let commit_map = collect_checkpoint_commit_map(&cfg.repo_root)?;
    let mut existing_event_ids = fetch_existing_checkpoint_event_ids(cfg, &backends.events).await?;

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
            insert_checkpoint_event(cfg, &backends.events, &cp, &event_id, commit_info).await?;
            existing_event_ids.insert(event_id);
            counters.events_inserted += 1;
        }

        if commit_sha.is_empty() {
            counters.checkpoints_without_commit += 1;
            continue;
        }

        upsert_commit_row(
            cfg,
            &relational,
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
            let content = git_blob_content(&cfg.repo_root, &blob_sha).unwrap_or_default();

            upsert_file_state_row(
                &cfg.repo.repo_id,
                &relational,
                &commit_sha,
                &normalized_path,
                &blob_sha,
            )
            .await?;
            let file_artefact = upsert_file_artefact_row(
                &cfg.repo.repo_id,
                &cfg.repo_root,
                &relational,
                &normalized_path,
                &blob_sha,
            )
            .await?;
            upsert_language_artefacts(
                cfg,
                &relational,
                &FileRevision {
                    commit_sha: &commit_sha,
                    commit_unix: commit_info
                        .expect("commit_info exists when sha exists")
                        .commit_unix,
                    path: &normalized_path,
                    blob_sha: &blob_sha,
                },
                &file_artefact,
            )
            .await?;
            counters.artefacts_upserted += 1;

            let pre_stage_artefacts = load_pre_stage_artefacts_for_blob(
                &relational,
                &cfg.repo.repo_id,
                &blob_sha,
                &normalized_path,
            )
            .await?;
            let semantic_feature_inputs = semantic::build_semantic_feature_inputs_from_artefacts(
                &pre_stage_artefacts,
                &content,
            );
            let semantic_feature_stats = upsert_semantic_feature_rows(
                &relational,
                &semantic_feature_inputs,
                Arc::clone(&summary_provider),
            )
            .await?;
            counters.semantic_feature_rows_upserted += semantic_feature_stats.upserted;
            counters.semantic_feature_rows_skipped += semantic_feature_stats.skipped;
        }

        counters.checkpoints_processed += 1;
    }

    println!(
        "DevQL ingest complete: checkpoints_processed={}, events_inserted={}, artefacts_upserted={}, checkpoints_without_commit={}, semantic_feature_rows_upserted={}, semantic_feature_rows_skipped={}",
        counters.checkpoints_processed,
        counters.events_inserted,
        counters.artefacts_upserted,
        counters.checkpoints_without_commit,
        counters.semantic_feature_rows_upserted,
        counters.semantic_feature_rows_skipped
    );
    Ok(())
}

pub async fn run_query(cfg: &DevqlConfig, query: &str, compact: bool) -> Result<()> {
    let output = execute_query_json(cfg, query).await?;
    if compact {
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

async fn execute_query_json(cfg: &DevqlConfig, query: &str) -> Result<Value> {
    let parsed = parse_devql_query(query)?;
    let backends = resolve_store_backend_config_for_repo(&cfg.repo_root)
        .context("resolving DevQL backend config for `devql query`")?;
    let relational = if parsed.has_checkpoints_stage || parsed.has_telemetry_stage {
        None
    } else {
        Some(RelationalStorage::connect(cfg, &backends.relational, "devql query").await?)
    };
    let mut rows = execute_devql_query(cfg, &parsed, &backends.events, relational.as_ref()).await?;

    if !parsed.select_fields.is_empty() {
        rows = project_rows(rows, &parsed.select_fields);
    }

    Ok(Value::Array(rows))
}

include!("canonical_mapping.rs");
include!("vocab.rs");
// ingestion: shared types
include!("ingestion/types.rs");
// ingestion: repo identity & git remote parsing
include!("ingestion/repo_identity.rs");
// ingestion: database schema DDL
include!("ingestion/schema.rs");
// ingestion: language detection & git blob utilities
include!("ingestion/language.rs");
// ingestion: artefact symbol identity helpers
include!("ingestion/artefact_identity.rs");
// ingestion: checkpoint / commit / event persistence
include!("ingestion/checkpoint.rs");
// ingestion: file & language artefact DB upserts
include!("ingestion/artefact_persistence.rs");
// ingestion: Stage 1 semantic persistence
include!("ingestion/semantic_features_persistence.rs");
// ingestion: JS/TS artefact extraction (tree-sitter)
include!("ingestion/extraction_js_ts.rs");
// ingestion: Rust artefact extraction (tree-sitter)
include!("ingestion/extraction_rust.rs");
// ingestion: shared edge-building utilities
include!("ingestion/edges_shared.rs");
// ingestion: export edges (JS/TS + Rust)
include!("ingestion/edges_export.rs");
// ingestion: inheritance edges (JS/TS + Rust)
include!("ingestion/edges_inherits.rs");
// ingestion: reference edges (JS/TS + Rust)
include!("ingestion/edges_reference.rs");
// ingestion: JS/TS dependency edge orchestration
include!("ingestion/edges_js_ts.rs");
// ingestion: Rust dependency edge orchestration
include!("ingestion/edges_rust.rs");
include!("query/parser.rs");
include!("query/executor.rs");
include!("query/utils.rs");
include!("deps_query.rs");
include!("db_utils.rs");

#[cfg(test)]
fn symbol_id_for_artefact(item: &JsTsArtefact) -> String {
    structural_symbol_id_for_artefact(item, None)
}

#[cfg(test)]
#[path = "tests/devql_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/identity_tests.rs"]
mod identity_tests;

#[cfg(test)]
#[path = "tests/mapping_tests.rs"]
mod mapping_tests;

#[cfg(test)]
#[path = "tests/cucumber_world.rs"]
mod cucumber_world;

#[cfg(test)]
#[path = "tests/cucumber_steps.rs"]
mod cucumber_steps;

#[cfg(test)]
#[path = "tests/cucumber_bdd.rs"]
mod cucumber_bdd;
