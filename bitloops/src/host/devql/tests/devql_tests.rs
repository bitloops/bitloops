use super::*;
use crate::cli::devql::{DevqlArgs, DevqlCommand, DevqlInitArgs, run as run_devql_command};
use crate::cli::{Cli, Commands};
use crate::config::{BlobStorageConfig, StoreFileConfig};
use crate::test_support::git_fixtures::{git_ok, init_test_repo};
use crate::test_support::process_state::enter_process_state;
use clap::Parser;
use std::env;
use std::fs;
use std::path::Path;
use tempfile::{TempDir, tempdir};

fn test_cfg() -> DevqlConfig {
    DevqlConfig {
        config_root: PathBuf::from("/tmp/repo"),
        repo_root: PathBuf::from("/tmp/repo"),
        repo: RepoIdentity {
            provider: "github".to_string(),
            organization: "bitloops".to_string(),
            name: "temp2".to_string(),
            identity: "github/bitloops/temp2".to_string(),
            repo_id: deterministic_uuid("repo://github/bitloops/temp2"),
        },
        pg_dsn: None,
        clickhouse_url: "http://localhost:8123".to_string(),
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: "default".to_string(),
        semantic_provider: None,
        semantic_model: None,
        semantic_api_key: None,
        semantic_base_url: None,
    }
}

fn test_cfg_with_repo_id(repo_suffix: &str, dsn: &str) -> DevqlConfig {
    let mut cfg = test_cfg();
    cfg.pg_dsn = Some(dsn.to_string());
    cfg.repo.repo_id = deterministic_uuid(&format!("repo://{repo_suffix}"));
    cfg
}

fn backend_cfg(sqlite_path: Option<String>, duckdb_path: Option<String>) -> StoreBackendConfig {
    StoreBackendConfig {
        relational: RelationalBackendConfig {
            sqlite_path,
            postgres_dsn: None,
        },
        events: EventsBackendConfig {
            duckdb_path,
            clickhouse_url: None,
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: None,
        },
        blobs: BlobStorageConfig {
            local_path: None,
            s3_bucket: None,
            s3_region: None,
            s3_access_key_id: None,
            s3_secret_access_key: None,
            gcs_bucket: None,
            gcs_credentials_path: None,
        },
    }
}

fn default_events_cfg() -> EventsBackendConfig {
    backend_cfg(None, None).events
}

async fn postgres_relational_store(cfg: &DevqlConfig, dsn: &str) -> RelationalStorage {
    RelationalStorage::connect(
        cfg,
        &RelationalBackendConfig {
            sqlite_path: None,
            postgres_dsn: Some(dsn.to_string()),
        },
        "devql test",
    )
    .await
    .expect("connect postgres relational store")
}

fn create_sqlite_db(path: &Path) {
    let conn = rusqlite::Connection::open(path).expect("create sqlite db");
    conn.execute_batch("SELECT 1")
        .expect("validate sqlite db file");
}

fn create_duckdb_db(path: &Path) {
    let conn = duckdb::Connection::open(path).expect("create duckdb db");
    conn.execute_batch("SELECT 1")
        .expect("validate duckdb db file");
}

pub(super) fn write_repo_daemon_config(repo_root: &Path, body: impl AsRef<str>) {
    fs::create_dir_all(repo_root).expect("create test repo root");
    fs::write(
        repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH),
        body.as_ref(),
    )
    .expect("write test daemon config");
}

fn apply_symbol_clone_edges_sqlite_schema(path: &Path) {
    use crate::capability_packs::semantic_clones::schema::semantic_clones_sqlite_schema_sql;
    let conn = rusqlite::Connection::open(path).expect("open sqlite for clone DDL");
    conn.execute_batch(semantic_clones_sqlite_schema_sql())
        .expect("apply symbol_clone_edges DDL");
}

fn apply_legacy_current_state_compat_schema(_path: &Path) {
    // Legacy compat schema no longer needed — init schema and sync schema are aligned.
}

async fn seed_test_repository_catalog_row(relational: &RelationalStorage, cfg: &DevqlConfig) {
    relational
        .exec(&format!(
            "INSERT INTO repositories (repo_id, provider, organization, name, default_branch) \
             VALUES ('{}', '{}', '{}', '{}', 'main') \
             ON CONFLICT(repo_id) DO UPDATE SET \
               provider = excluded.provider, \
               organization = excluded.organization, \
               name = excluded.name, \
               default_branch = excluded.default_branch",
            crate::host::devql::db_utils::esc_pg(&cfg.repo.repo_id),
            crate::host::devql::db_utils::esc_pg(&cfg.repo.provider),
            crate::host::devql::db_utils::esc_pg(&cfg.repo.organization),
            crate::host::devql::db_utils::esc_pg(&cfg.repo.name),
        ))
        .await
        .expect("seed test repository catalog row");
}

async fn sqlite_relational_store_with_schema(path: &Path) -> RelationalStorage {
    init_sqlite_schema(path)
        .await
        .expect("initialise sqlite relational schema");
    let path_buf = path.to_path_buf();
    tokio::task::spawn_blocking({
        let path = path_buf.clone();
        move || apply_legacy_current_state_compat_schema(&path)
    })
    .await
    .expect("join blocking legacy current-state DDL");
    tokio::task::spawn_blocking({
        let path = path_buf.clone();
        move || apply_symbol_clone_edges_sqlite_schema(&path)
    })
    .await
    .expect("join blocking clone DDL");
    let relational = RelationalStorage::local_only(path_buf);
    seed_test_repository_catalog_row(&relational, &test_cfg()).await;
    relational
}

#[tokio::test]
async fn checkpoint_provenance_projection_is_idempotent_for_commit_diff_rows() {
    let repo = seed_git_repo();
    fs::create_dir_all(repo.path().join("src")).expect("create src dir");
    fs::write(
        repo.path().join("src/one.ts"),
        "export function one(): number {\n  return 1;\n}\n",
    )
    .expect("write src/one.ts");
    fs::write(
        repo.path().join("src/two.ts"),
        "export function two(): number {\n  return 2;\n}\n",
    )
    .expect("write src/two.ts");
    git_ok(repo.path(), &["add", "src/one.ts", "src/two.ts"]);
    git_ok(
        repo.path(),
        &["commit", "-m", "Add checkpoint provenance sources"],
    );
    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("relational.sqlite");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;

    let mut cfg = test_cfg();
    cfg.config_root = repo.path().to_path_buf();
    cfg.repo_root = repo.path().to_path_buf();
    cfg.repo = resolve_repo_identity(repo.path()).expect("resolve repo identity");

    let checkpoint = CommittedInfo {
        checkpoint_id: "checkpoint-1".to_string(),
        strategy: "manual-commit".to_string(),
        branch: "main".to_string(),
        files_touched: vec!["src/one.ts".to_string(), "src/two.ts".to_string()],
        session_id: "session-1".to_string(),
        agent: "claude-code".to_string(),
        created_at: "2026-03-27T10:15:00Z".to_string(),
        ..Default::default()
    };
    let commit_info = CheckpointCommitInfo {
        commit_sha: head_sha.clone(),
        commit_unix: 1_742_972_900,
        author_name: "Bitloops Test".to_string(),
        author_email: "bitloops-test@example.com".to_string(),
        subject: "checkpoint".to_string(),
    };

    let projected_first = upsert_checkpoint_file_snapshot_rows(
        &cfg,
        &relational,
        &checkpoint,
        &commit_info.commit_sha,
        Some(&commit_info),
    )
    .await
    .expect("project checkpoint snapshot rows");
    let projected_second = upsert_checkpoint_file_snapshot_rows(
        &cfg,
        &relational,
        &checkpoint,
        &commit_info.commit_sha,
        Some(&commit_info),
    )
    .await
    .expect("reproject checkpoint snapshot rows");
    assert_eq!(
        projected_first, 2,
        "expected two resolved file snapshot rows"
    );
    assert_eq!(
        projected_second, 2,
        "replaying the same checkpoint should target the same two rows"
    );

    let sqlite = rusqlite::Connection::open(&sqlite_path).expect("open sqlite db");
    let mut stmt = sqlite
        .prepare(
            "SELECT path_after, blob_sha_after, session_id, agent, branch, strategy, commit_sha, event_time, change_kind
             FROM checkpoint_files
             WHERE repo_id = ?1 AND checkpoint_id = ?2
             ORDER BY path_after ASC",
        )
        .expect("prepare checkpoint_files query");
    let rows = stmt
        .query_map(
            rusqlite::params![cfg.repo.repo_id.as_str(), checkpoint.checkpoint_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .expect("query checkpoint_files rows")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect checkpoint_files rows");

    assert_eq!(
        rows,
        vec![
            (
                "src/one.ts".to_string(),
                git_blob_sha_at_commit(repo.path(), &head_sha, "src/one.ts")
                    .expect("resolve blob sha for src/one.ts"),
                "session-1".to_string(),
                "claude-code".to_string(),
                "main".to_string(),
                "manual-commit".to_string(),
                head_sha.clone(),
                "2026-03-27T10:15:00Z".to_string(),
                "add".to_string(),
            ),
            (
                "src/two.ts".to_string(),
                git_blob_sha_at_commit(repo.path(), &head_sha, "src/two.ts")
                    .expect("resolve blob sha for src/two.ts"),
                "session-1".to_string(),
                "claude-code".to_string(),
                "main".to_string(),
                "manual-commit".to_string(),
                head_sha.clone(),
                "2026-03-27T10:15:00Z".to_string(),
                "add".to_string(),
            ),
        ],
        "projection should upsert one checkpoint_files row per changed file"
    );
}

#[tokio::test]
async fn init_duckdb_schema_creates_checkpoint_events_table() {
    let temp = tempdir().expect("temp dir");
    let path = temp.path().join("events.duckdb");
    let events_cfg = backend_cfg(None, Some(path.to_string_lossy().to_string())).events;

    init_duckdb_schema(temp.path(), &events_cfg)
        .await
        .expect("initialise duckdb schema");

    let conn = duckdb::Connection::open(path).expect("open duckdb");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = 'checkpoint_events'",
            [],
            |row| row.get(0),
        )
        .expect("query checkpoint_events table");
    assert_eq!(count, 1);
}

#[tokio::test]
async fn init_clickhouse_schema_returns_error_for_unreachable_endpoint() {
    let mut cfg = test_cfg();
    cfg.clickhouse_url = "http://127.0.0.1:9".to_string();

    let err = init_clickhouse_schema(&cfg)
        .await
        .expect_err("unreachable clickhouse endpoint must fail");

    assert!(
        err.to_string().contains("ClickHouse")
            || err.to_string().contains("sending ClickHouse request")
    );
}

fn seed_git_repo() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    init_test_repo(
        dir.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    git_ok(dir.path(), &["commit", "--allow-empty", "-m", "initial"]);
    dir
}

fn insert_commit_checkpoint_mapping(repo_root: &Path, commit_sha: &str, checkpoint_id: &str) {
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    let sqlite =
        crate::storage::SqliteConnectionPool::connect(sqlite_path).expect("connect sqlite");
    sqlite
        .initialise_checkpoint_schema()
        .expect("initialise checkpoint schema");
    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    sqlite
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO commit_checkpoints (commit_sha, checkpoint_id, repo_id)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![commit_sha, checkpoint_id, repo_id.as_str()],
            )?;
            Ok(())
        })
        .expect("insert commit-checkpoint mapping");
}

fn checkpoint_sqlite_path(repo_root: &Path) -> std::path::PathBuf {
    let cfg = crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config");
    if let Some(path) = cfg.relational.sqlite_path.as_deref() {
        crate::config::resolve_sqlite_db_path_for_repo(repo_root, Some(path))
            .expect("resolve configured sqlite path")
    } else {
        crate::utils::paths::default_relational_db_path(repo_root)
    }
}

fn status_for(rows: &[DatabaseStatusRow], label: &'static str) -> DatabaseConnectionStatus {
    rows.iter()
        .find(|row| row.db == label)
        .map(|row| row.status)
        .unwrap_or_else(|| panic!("missing status row for {label}"))
}

fn test_file_row(
    cfg: &DevqlConfig,
    path: &str,
    blob_sha: &str,
    end_line: i32,
    end_byte: i32,
) -> FileArtefactRow {
    let symbol_id = file_symbol_id(path);
    FileArtefactRow {
        artefact_id: revision_artefact_id(&cfg.repo.repo_id, blob_sha, &symbol_id),
        symbol_id,
        language: "typescript".to_string(),
        end_line,
        end_byte,
    }
}

fn test_symbol_record(
    cfg: &DevqlConfig,
    path: &str,
    blob_sha: &str,
    symbol_id: &str,
    name: &str,
    start_line: i32,
    end_line: i32,
) -> PersistedArtefactRecord {
    let file_symbol_id = file_symbol_id(path);
    let file_artefact_id = revision_artefact_id(&cfg.repo.repo_id, blob_sha, &file_symbol_id);
    PersistedArtefactRecord {
        symbol_id: symbol_id.to_string(),
        artefact_id: revision_artefact_id(&cfg.repo.repo_id, blob_sha, symbol_id),
        canonical_kind: Some("function".to_string()),
        language_kind: "function_declaration".to_string(),
        symbol_fqn: format!("{path}::{name}"),
        parent_symbol_id: Some(file_symbol_id),
        parent_artefact_id: Some(file_artefact_id),
        start_line,
        end_line,
        start_byte: (start_line - 1) * 10,
        end_byte: (end_line * 10) + 5,
        signature: Some(format!("export function {name}() {{")),
        modifiers: vec![],
        docstring: None,
        content_hash: format!("hash-{blob_sha}-{name}"),
    }
}

fn test_call_edge(from_symbol_fqn: &str, target_symbol_fqn: &str, line: i32) -> DependencyEdge {
    DependencyEdge {
        edge_kind: EdgeKind::Calls,
        from_symbol_fqn: from_symbol_fqn.to_string(),
        to_target_symbol_fqn: Some(target_symbol_fqn.to_string()),
        to_symbol_ref: Some(target_symbol_fqn.to_string()),
        start_line: Some(line),
        end_line: Some(line),
        metadata: EdgeMetadata::call(CallForm::Identifier, Resolution::Local),
    }
}

fn test_unresolved_call_edge(from_symbol_fqn: &str, symbol_ref: &str, line: i32) -> DependencyEdge {
    DependencyEdge {
        edge_kind: EdgeKind::Calls,
        from_symbol_fqn: from_symbol_fqn.to_string(),
        to_target_symbol_fqn: None,
        to_symbol_ref: Some(symbol_ref.to_string()),
        start_line: Some(line),
        end_line: Some(line),
        metadata: EdgeMetadata::call(CallForm::Identifier, Resolution::Unresolved),
    }
}

#[path = "devql_tests/baseline.rs"]
mod baseline;
#[path = "devql_tests/config_and_status.rs"]
mod config_and_status;
#[path = "devql_tests/core_and_ingestion.rs"]
mod core_and_ingestion;
#[path = "devql_tests/extraction_go.rs"]
mod extraction_go;
#[path = "devql_tests/extraction_js_ts.rs"]
mod extraction_js_ts;
#[path = "devql_tests/extraction_rust.rs"]
mod extraction_rust;
#[path = "devql_tests/identity_and_schema.rs"]
mod identity_and_schema;
#[path = "devql_tests/postgres_integration.rs"]
mod postgres_integration;
#[path = "devql_tests/projection_backfill.rs"]
mod projection_backfill;
#[path = "devql_tests/query_executor.rs"]
mod query_executor;
#[path = "devql_tests/query_pipeline.rs"]
mod query_pipeline;

// --- CLI arg parsing and run() dispatch (moved from commands/devql.rs) ---

#[test]
fn devql_cli_parses_ingest_defaults() {
    let parsed =
        Cli::try_parse_from(["bitloops", "devql", "ingest"]).expect("devql ingest should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Ingest(ingest)) = args.command else {
        panic!("expected devql ingest command");
    };

    assert_eq!(ingest.max_checkpoints, 500);
}

#[test]
fn devql_cli_parses_query_compact_flag() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "query",
        "repo(\"bitloops-cli\")",
        "--compact",
    ])
    .expect("devql query should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Query(query)) = args.command else {
        panic!("expected devql query command");
    };

    assert_eq!(query.query, "repo(\"bitloops-cli\")");
    assert!(query.compact);
}

#[tokio::test]
async fn devql_run_requires_subcommand() {
    let err = run_devql_command(DevqlArgs::default())
        .await
        .expect_err("missing subcommand should error");

    assert!(
        err.to_string()
            .contains(crate::cli::devql::MISSING_SUBCOMMAND_MESSAGE)
    );
}

#[tokio::test]
async fn devql_run_init_requires_running_daemon_after_repo_resolution() {
    let repo = seed_git_repo();
    let home = TempDir::new().expect("home dir");
    let home_path = home.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(repo.path()),
        &[
            ("HOME", Some(home_path.as_str())),
            ("USERPROFILE", Some(home_path.as_str())),
            ("BITLOOPS_DEVQL_PG_DSN", None),
            ("BITLOOPS_DEVQL_CH_URL", None),
            ("BITLOOPS_DEVQL_CH_USER", None),
            ("BITLOOPS_DEVQL_CH_PASSWORD", None),
            ("BITLOOPS_DEVQL_CH_DATABASE", None),
        ],
    );

    let err = run_devql_command(DevqlArgs {
        command: Some(DevqlCommand::Init(DevqlInitArgs::default())),
    })
    .await
    .expect_err("devql init should require a running daemon");

    assert!(
        err.to_string().contains("Bitloops daemon is not running"),
        "expected daemon-required error after repo resolution, got: {err:#}"
    );
}
#[path = "devql_tests/clones.rs"]
mod clones;
#[path = "devql_tests/semantic.rs"]
mod semantic;
