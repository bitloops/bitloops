use super::*;
use crate::commands::devql::DevqlCommand;
use crate::store_config::{BlobStorageConfig, BlobStorageProvider, StoreFileConfig};
use crate::test_support::git_fixtures::{git_ok, init_test_repo};
use clap::Parser;
use serde_json::json;
use std::env;
use std::path::Path;
use tempfile::{TempDir, tempdir};

fn test_cfg() -> DevqlConfig {
    DevqlConfig {
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
        embedding_provider: None,
        embedding_model: None,
        embedding_api_key: None,
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
            provider: RelationalProvider::Sqlite,
            sqlite_path,
            postgres_dsn: None,
        },
        events: EventsBackendConfig {
            provider: EventsProvider::DuckDb,
            duckdb_path,
            clickhouse_url: None,
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: None,
        },
        blobs: BlobStorageConfig {
            provider: BlobStorageProvider::Local,
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
            provider: RelationalProvider::Postgres,
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

async fn sqlite_relational_store_with_schema(path: &Path) -> RelationalStorage {
    init_sqlite_schema(path)
        .await
        .expect("initialise sqlite relational schema");
    RelationalStorage::Sqlite {
        path: path.to_path_buf(),
    }
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
        crate::engine::db::SqliteConnectionPool::connect(sqlite_path).expect("connect sqlite");
    sqlite
        .initialise_checkpoint_schema()
        .expect("initialise checkpoint schema");
    let repo_id = crate::engine::devql::resolve_repo_id(repo_root).expect("resolve repo id");
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
    let cfg = crate::store_config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config");
    if let Some(path) = cfg.relational.sqlite_path.as_deref() {
        crate::store_config::resolve_sqlite_db_path_for_repo(repo_root, Some(path))
            .expect("resolve configured sqlite path")
    } else {
        crate::engine::paths::default_relational_db_path(repo_root)
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

fn test_call_edge(from_symbol_fqn: &str, target_symbol_fqn: &str, line: i32) -> JsTsDependencyEdge {
    JsTsDependencyEdge {
        edge_kind: "calls".to_string(),
        from_symbol_fqn: from_symbol_fqn.to_string(),
        to_target_symbol_fqn: Some(target_symbol_fqn.to_string()),
        to_symbol_ref: Some(target_symbol_fqn.to_string()),
        start_line: Some(line),
        end_line: Some(line),
        metadata: json!({ "resolution": "local" }),
    }
}

fn test_unresolved_call_edge(
    from_symbol_fqn: &str,
    symbol_ref: &str,
    line: i32,
) -> JsTsDependencyEdge {
    JsTsDependencyEdge {
        edge_kind: "calls".to_string(),
        from_symbol_fqn: from_symbol_fqn.to_string(),
        to_target_symbol_fqn: None,
        to_symbol_ref: Some(symbol_ref.to_string()),
        start_line: Some(line),
        end_line: Some(line),
        metadata: json!({ "resolution": "unresolved" }),
    }
}

include!("devql_tests/core_and_ingestion.rs");
include!("devql_tests/query_pipeline.rs");
include!("devql_tests/query_executor.rs");
include!("devql_tests/config_and_status.rs");
include!("devql_tests/extraction_js_ts.rs");
include!("devql_tests/extraction_rust.rs");
include!("devql_tests/identity_and_schema.rs");
include!("devql_tests/postgres_integration.rs");
include!("devql_tests/semantic.rs");
