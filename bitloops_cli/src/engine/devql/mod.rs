use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use serde_json::{Map, Value, json};
use tokio_postgres::{NoTls, config::SslMode};

use crate::engine::capability_packs::builtin::semantic_clones as semantic_clones_pack;
use crate::engine::db_status::{
    DatabaseConnectionStatus, DatabaseStatusRow, classify_connection_error,
};
use crate::engine::extensions::{
    CapabilityExecutionContext, CapabilityIngestContext, CoreExtensionHost, LanguagePackContext,
    LanguagePackResolutionInput,
};
use crate::engine::providers::embeddings::EmbeddingProvider;
use crate::engine::semantic_clones;
use crate::engine::semantic_embeddings;
use crate::engine::semantic_features as semantic;
use crate::engine::strategy::manual_commit::{
    CommittedInfo, list_committed, read_commit_checkpoint_mappings, read_committed,
    read_session_content, run_git,
};
use crate::store_config::{
    EventsBackendConfig, EventsProvider, RelationalBackendConfig, RelationalProvider,
    StoreBackendConfig, resolve_store_backend_config, resolve_store_backend_config_for_repo,
    resolve_store_embedding_config, resolve_store_semantic_config,
};
use crate::terminal::db_status_table::print_db_status_table;

pub mod capabilities;
pub mod capability_host;
pub(crate) mod identity;

pub(crate) use identity::deterministic_uuid;
pub mod watch;

pub fn build_capability_host(
    repo_root: &Path,
    repo: RepoIdentity,
) -> anyhow::Result<capability_host::DevqlCapabilityHost> {
    capability_host::DevqlCapabilityHost::builtin(repo_root.to_path_buf(), repo)
}

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
    pub(crate) embedding_provider: Option<String>,
    pub(crate) embedding_model: Option<String>,
    pub(crate) embedding_api_key: Option<String>,
}

impl DevqlConfig {
    pub fn from_env(repo_root: PathBuf, repo: RepoIdentity) -> Result<Self> {
        let backend_cfg = resolve_store_backend_config_for_repo(&repo_root)
            .context("resolving backend config for DevQL runtime")?;
        let semantic_cfg = resolve_store_semantic_config();
        let embedding_cfg = resolve_store_embedding_config();
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
            embedding_provider: embedding_cfg.embedding_provider,
            embedding_model: embedding_cfg.embedding_model,
            embedding_api_key: embedding_cfg.embedding_api_key,
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
const RUST_LANGUAGE_PACK_ID: &str = "rust-language-pack";
const TS_JS_LANGUAGE_PACK_ID: &str = "ts-js-language-pack";
const SEMANTIC_CLONES_CAPABILITY_STAGE_ID: &str = semantic_clones_pack::SEMANTIC_CLONES_STAGE_ID;
const KNOWLEDGE_CAPABILITY_INGESTER_ID: &str = "knowledge-ingester";
const TEST_HARNESS_CAPABILITY_INGESTER_ID: &str = "test-harness-ingester";
pub(crate) const DEVQL_POSTGRES_DSN_REQUIRED_PREFIX: &str = "DevQL Postgres DSN is required";

fn core_extension_host() -> Result<&'static CoreExtensionHost> {
    static CORE_EXTENSION_HOST: OnceLock<Result<CoreExtensionHost, String>> = OnceLock::new();
    let host_result = CORE_EXTENSION_HOST.get_or_init(|| {
        CoreExtensionHost::with_builtins()
            .map_err(|err| format!("bootstrapping built-in extension packs: {err}"))
    });
    match host_result {
        Ok(host) => Ok(host),
        Err(error_message) => Err(anyhow!(
            "initialising Core extension host for DevQL runtime: {error_message}"
        )),
    }
}

fn normalise_optional_commit_sha(commit_sha: Option<&str>) -> Option<String> {
    commit_sha
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn resolve_language_pack_owner_for_input(
    language: &str,
    file_path: Option<&str>,
) -> Option<&'static str> {
    core_extension_host().ok().and_then(|host| {
        if let Some(path) = file_path
            && let Ok(resolved) = host
                .language_packs()
                .resolve(LanguagePackResolutionInput::for_file_path(path))
        {
            return Some(resolved.pack.id);
        }

        let input = file_path
            .map(|path| LanguagePackResolutionInput::for_language(language).with_file_path(path))
            .unwrap_or_else(|| LanguagePackResolutionInput::for_language(language));

        host.language_packs()
            .resolve(input)
            .ok()
            .map(|resolved| resolved.pack.id)
            .or_else(|| {
                host.language_packs()
                    .owner_for_language(language)
                    .and_then(|pack_key| host.language_packs().resolve_pack(pack_key))
                    .map(|descriptor| descriptor.id)
            })
    })
}

fn resolve_language_pack_owner(language: &str) -> Option<&'static str> {
    resolve_language_pack_owner_for_input(language, None)
}

fn resolve_language_id_for_file_path(file_path: &str) -> Option<&'static str> {
    core_extension_host().ok().and_then(|host| {
        host.language_packs()
            .resolve(LanguagePackResolutionInput::for_file_path(file_path))
            .ok()
            .map(|resolved| resolved.profile.language_id)
    })
}

fn language_pack_context_for_language(
    cfg: &DevqlConfig,
    commit_sha: Option<&str>,
    language: &str,
    file_path: Option<&str>,
) -> Result<Option<(LanguagePackContext, &'static str)>> {
    let Some(pack_id) = resolve_language_pack_owner_for_input(language, file_path) else {
        return Ok(None);
    };
    Ok(Some((
        LanguagePackContext::new(
            cfg.repo_root.clone(),
            cfg.repo.repo_id.clone(),
            normalise_optional_commit_sha(commit_sha),
        ),
        pack_id,
    )))
}

fn capability_execution_context_for_stage(
    cfg: &DevqlConfig,
    commit_sha: Option<&str>,
    stage_id: &str,
) -> Result<CapabilityExecutionContext> {
    let host = core_extension_host()?;
    let capability_pack_id = host.resolve_stage_owner_for_execution(stage_id)?;
    Ok(CapabilityExecutionContext::new(
        cfg.repo_root.clone(),
        cfg.repo.repo_id.clone(),
        normalise_optional_commit_sha(commit_sha),
        capability_pack_id,
        stage_id,
    ))
}

fn capability_ingest_context_for_ingester(
    cfg: &DevqlConfig,
    commit_sha: Option<&str>,
    ingester_id: &str,
) -> Result<CapabilityIngestContext> {
    let host = core_extension_host()?;
    let capability_pack_id = host.resolve_ingester_owner_for_ingest(ingester_id)?;
    Ok(CapabilityIngestContext::new(
        cfg.repo_root.clone(),
        cfg.repo.repo_id.clone(),
        normalise_optional_commit_sha(commit_sha),
        capability_pack_id,
        ingester_id,
    ))
}

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
    }?;

    let test_harness_context =
        capability_ingest_context_for_ingester(cfg, None, TEST_HARNESS_CAPABILITY_INGESTER_ID)
            .context("resolving test-harness capability ingester owner")?;
    crate::engine::test_harness::init_schema_for_repo(&cfg.repo_root).with_context(|| {
        format!(
            "initialising test-harness schema for capability pack `{}`",
            test_harness_context.capability_pack_id
        )
    })?;
    Ok(())
}

fn semantic_provider_config(cfg: &DevqlConfig) -> semantic::SemanticSummaryProviderConfig {
    semantic::SemanticSummaryProviderConfig {
        semantic_provider: cfg.semantic_provider.clone(),
        semantic_model: cfg.semantic_model.clone(),
        semantic_api_key: cfg.semantic_api_key.clone(),
        semantic_base_url: cfg.semantic_base_url.clone(),
    }
}

fn embedding_provider_config(cfg: &DevqlConfig) -> semantic_embeddings::EmbeddingProviderConfig {
    semantic_embeddings::EmbeddingProviderConfig {
        embedding_provider: cfg.embedding_provider.clone(),
        embedding_model: cfg.embedding_model.clone(),
        embedding_api_key: cfg.embedding_api_key.clone(),
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
    let _ = core_extension_host().context("loading Core extension host for `devql ingest`")?;
    let backends = resolve_store_backend_config_for_repo(&cfg.repo_root)
        .context("resolving DevQL backend config for `devql ingest`")?;
    let relational = RelationalStorage::connect(cfg, &backends.relational, "devql ingest").await?;
    let knowledge_context =
        capability_ingest_context_for_ingester(cfg, None, KNOWLEDGE_CAPABILITY_INGESTER_ID)
            .context("resolving knowledge capability ingester owner")?;
    let summary_provider =
        semantic_clones_pack::build_semantic_summary_provider(&semantic_provider_config(cfg))?;
    let embedding_provider = semantic_clones_pack::build_symbol_embedding_provider(
        &embedding_provider_config(cfg),
        Some(&cfg.repo_root),
    )?;
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
                delete_current_state_for_path(cfg, &relational, &normalized_path).await?;
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
                    revision: TemporalRevisionRef {
                        kind: TemporalRevisionKind::Commit,
                        id: &commit_sha,
                        temp_checkpoint_id: None,
                    },
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
            let pre_stage_dependencies = load_pre_stage_dependencies_for_blob(
                &relational,
                &cfg.repo.repo_id,
                &blob_sha,
                &normalized_path,
            )
            .await?;
            let semantic_feature_inputs = semantic_clones_pack::build_semantic_feature_inputs(
                &pre_stage_artefacts,
                &pre_stage_dependencies,
                &content,
            );
            let semantic_feature_stats = upsert_semantic_feature_rows(
                &relational,
                &semantic_feature_inputs,
                Arc::clone(&summary_provider),
            )
            .await
            .with_context(|| {
                format!(
                    "running capability ingester `{}` owned by `{}`",
                    knowledge_context.ingester_id, knowledge_context.capability_pack_id
                )
            })?;
            if let Some(embedding_provider) = embedding_provider.as_ref() {
                let embedding_stats = upsert_symbol_embedding_rows(
                    &relational,
                    &semantic_feature_inputs,
                    Arc::clone(embedding_provider),
                )
                .await?;
                counters.symbol_embedding_rows_upserted += embedding_stats.upserted;
                counters.symbol_embedding_rows_skipped += embedding_stats.skipped;
            }
            counters.artefacts_upserted += 1;
            counters.semantic_feature_rows_upserted += semantic_feature_stats.upserted;
            counters.semantic_feature_rows_skipped += semantic_feature_stats.skipped;
        }

        counters.checkpoints_processed += 1;
    }

    counters.temporary_rows_promoted =
        promote_temporary_current_rows_for_head_commit(cfg, &relational).await?;

    let semantic_clones_context =
        capability_execution_context_for_stage(cfg, None, SEMANTIC_CLONES_CAPABILITY_STAGE_ID)
            .context("resolving semantic-clones capability stage owner")?;
    let clone_result = rebuild_symbol_clone_edges(&relational, &cfg.repo.repo_id)
        .await
        .with_context(|| {
            format!(
                "running capability stage `{}` owned by `{}`",
                semantic_clones_context.stage_id, semantic_clones_context.capability_pack_id
            )
        })?;
    counters.symbol_clone_edges_upserted += clone_result.edges.len();
    counters.symbol_clone_sources_scored += clone_result.sources_considered;

    println!(
        "DevQL ingest complete: checkpoints_processed={}, events_inserted={}, artefacts_upserted={}, checkpoints_without_commit={}, temporary_rows_promoted={}, semantic_feature_rows_upserted={}, semantic_feature_rows_skipped={}, symbol_embedding_rows_upserted={}, symbol_embedding_rows_skipped={}, symbol_clone_edges_upserted={}, symbol_clone_sources_scored={}",
        counters.checkpoints_processed,
        counters.events_inserted,
        counters.artefacts_upserted,
        counters.checkpoints_without_commit,
        counters.temporary_rows_promoted,
        counters.semantic_feature_rows_upserted,
        counters.semantic_feature_rows_skipped,
        counters.symbol_embedding_rows_upserted,
        counters.symbol_embedding_rows_skipped,
        counters.symbol_clone_edges_upserted,
        counters.symbol_clone_sources_scored
    );
    Ok(())
}

async fn promote_temporary_current_rows_for_head_commit(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
) -> Result<usize> {
    let head_sha = run_git(&cfg.repo_root, &["rev-parse", "HEAD"]).unwrap_or_default();
    if head_sha.is_empty() {
        return Ok(0);
    }
    let head_unix = run_git(&cfg.repo_root, &["show", "-s", "--format=%ct", &head_sha])
        .ok()
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .unwrap_or_default();
    let updated_at_sql = revision_timestamp_sql(relational, head_unix);

    let sql = format!(
        "SELECT path, blob_sha FROM artefacts_current \
	WHERE repo_id = '{}' AND canonical_kind = 'file' AND (revision_kind = 'temporary' OR revision_id LIKE 'temp:%')",
        esc_pg(&cfg.repo.repo_id),
    );
    let rows = relational.query_rows(&sql).await?;
    let mut promoted = 0usize;

    for row in rows {
        let Some(path) = row.get("path").and_then(Value::as_str) else {
            continue;
        };
        let Some(blob_sha) = row.get("blob_sha").and_then(Value::as_str) else {
            continue;
        };
        let Some(head_blob_sha) = git_blob_sha_at_commit(&cfg.repo_root, &head_sha, path) else {
            continue;
        };
        if head_blob_sha != blob_sha {
            continue;
        }

        upsert_file_state_row(
            &cfg.repo.repo_id,
            relational,
            &head_sha,
            path,
            &head_blob_sha,
        )
        .await?;

        let sql_artefacts = format!(
            "UPDATE artefacts_current \
	SET commit_sha = '{}', revision_kind = 'commit', revision_id = '{}', temp_checkpoint_id = NULL, blob_sha = '{}', updated_at = {} \
	WHERE repo_id = '{}' AND path = '{}' AND (revision_kind = 'temporary' OR revision_id LIKE 'temp:%')",
            esc_pg(&head_sha),
            esc_pg(&head_sha),
            esc_pg(&head_blob_sha),
            updated_at_sql,
            esc_pg(&cfg.repo.repo_id),
            esc_pg(path),
        );
        relational.exec(&sql_artefacts).await?;

        let sql_edges = format!(
            "UPDATE artefact_edges_current \
	SET commit_sha = '{}', revision_kind = 'commit', revision_id = '{}', temp_checkpoint_id = NULL, blob_sha = '{}', updated_at = {} \
	WHERE repo_id = '{}' AND path = '{}' AND (revision_kind = 'temporary' OR revision_id LIKE 'temp:%')",
            esc_pg(&head_sha),
            esc_pg(&head_sha),
            esc_pg(&head_blob_sha),
            updated_at_sql,
            esc_pg(&cfg.repo.repo_id),
            esc_pg(path),
        );
        relational.exec(&sql_edges).await?;

        promoted += 1;
    }

    Ok(promoted)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegisteredStageCompositionContext {
    pub(crate) caller_capability_id: String,
    pub(crate) depth: usize,
    pub(crate) max_depth: usize,
}

async fn execute_query_json(cfg: &DevqlConfig, query: &str) -> Result<Value> {
    execute_query_json_with_composition(cfg, query, None).await
}

pub(crate) async fn execute_query_json_with_composition(
    cfg: &DevqlConfig,
    query: &str,
    composition: Option<RegisteredStageCompositionContext>,
) -> Result<Value> {
    let parsed = parse_devql_query(query)?;
    let backends = resolve_store_backend_config_for_repo(&cfg.repo_root)
        .context("resolving DevQL backend config for `devql query`")?;
    let relational = if parsed.has_checkpoints_stage || parsed.has_telemetry_stage {
        None
    } else {
        Some(RelationalStorage::connect(cfg, &backends.relational, "devql query").await?)
    };
    let mut rows = execute_devql_query(cfg, &parsed, &backends.events, relational.as_ref()).await?;
    rows = execute_registered_stages_with_composition(cfg, &parsed, rows, composition.as_ref())
        .await?;

    if !parsed.select_fields.is_empty() {
        rows = project_rows(rows, &parsed.select_fields);
    }

    Ok(Value::Array(rows))
}

include!("core_contracts.rs");
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
// ingestion: shared record types for artefact persistence
include!("ingestion/artefact_persistence_types.rs");
// ingestion: SQL dialect helpers, JSON utilities, timestamp expressions
include!("ingestion/artefact_persistence_sql.rs");
// ingestion: file state row, file artefact upsert, revision management
include!("ingestion/artefact_persistence_file.rs");
// ingestion: symbol record building, content hashing, artefact DB upserts
include!("ingestion/artefact_persistence_symbols.rs");
// ingestion: edge records, current state queries/mutations, row deserialization
include!("ingestion/artefact_persistence_edges.rs");
// ingestion: top-level orchestration (refresh/upsert/delete current state)
include!("ingestion/artefact_persistence.rs");
// ingestion: Stage 1 semantic persistence
include!("ingestion/semantic_features_persistence.rs");
// ingestion: Stage 2 embedding persistence
include!("ingestion/semantic_embeddings_persistence.rs");
// ingestion: Stage 3 clone persistence
include!("ingestion/semantic_clones_persistence.rs");
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
#[path = "tests/core_contract_tests.rs"]
mod core_contract_tests;

#[cfg(test)]
#[path = "tests/cucumber_world.rs"]
mod cucumber_world;

#[cfg(test)]
#[path = "tests/cucumber_steps.rs"]
mod cucumber_steps;

#[cfg(test)]
#[path = "tests/cucumber_bdd.rs"]
mod cucumber_bdd;
