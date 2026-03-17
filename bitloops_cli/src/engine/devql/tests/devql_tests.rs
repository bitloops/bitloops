use super::*;
use crate::commands::devql::DevqlCommand;
use crate::devql_config::{BlobStorageConfig, BlobStorageProvider};
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
        embedding_base_url: None,
        embedding_output_dimension: None,
    }
}

fn test_cfg_with_repo_id(repo_suffix: &str, dsn: &str) -> DevqlConfig {
    let mut cfg = test_cfg();
    cfg.pg_dsn = Some(dsn.to_string());
    cfg.repo.repo_id = deterministic_uuid(&format!("repo://{repo_suffix}"));
    cfg
}

fn backend_cfg(sqlite_path: Option<String>, duckdb_path: Option<String>) -> DevqlBackendConfig {
    DevqlBackendConfig {
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
    let cfg =
        crate::devql_config::resolve_devql_backend_config().expect("resolve devql backend config");
    if let Some(path) = cfg.relational.sqlite_path.as_deref() {
        crate::devql_config::resolve_sqlite_db_path(Some(path))
            .expect("resolve configured sqlite path")
    } else {
        repo_root
            .join(crate::engine::paths::BITLOOPS_DIR)
            .join("devql")
            .join("relational.db")
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

#[test]
fn sql_helpers_escape_nullable_and_json_values() {
    assert_eq!(sql_nullable_text(None), "NULL");
    assert_eq!(sql_nullable_text(Some("O'Reilly")), "'O''Reilly'");
    assert_eq!(
        sql_jsonb_text_array(&["O'Reilly".to_string(), "plain".to_string()]),
        r#"'["O''Reilly","plain"]'::jsonb"#
    );
}

#[test]
fn supported_symbol_languages_are_whitelisted() {
    for language in ["typescript", "javascript", "rust"] {
        assert!(
            is_supported_symbol_language(language),
            "{language} should be supported"
        );
    }

    for language in ["python", "go", ""] {
        assert!(
            !is_supported_symbol_language(language),
            "{language} should not be supported"
        );
    }
}

#[test]
fn build_file_current_record_preserves_file_metadata() {
    let cfg = test_cfg();
    let file = test_file_row(&cfg, "src/main.rs", "blob-1", 42, 420);
    let record = build_file_current_record(
        "src/main.rs",
        "blob-1",
        &file,
        Some("Top-level docs".to_string()),
    );

    assert_eq!(record.symbol_id, file.symbol_id);
    assert_eq!(record.artefact_id, file.artefact_id);
    assert_eq!(record.canonical_kind.as_deref(), Some("file"));
    assert_eq!(record.language_kind, "file");
    assert_eq!(record.symbol_fqn, "src/main.rs");
    assert_eq!(record.end_line, 42);
    assert_eq!(record.end_byte, 420);
    assert_eq!(record.docstring.as_deref(), Some("Top-level docs"));
    assert_eq!(record.content_hash, "blob-1");
}

#[test]
fn build_symbol_records_chain_file_and_nested_parent_links() {
    let cfg = test_cfg();
    let path = "src/ui.ts";
    let blob_sha = "blob-ui";
    let file = test_file_row(&cfg, path, blob_sha, 30, 300);
    let items = vec![
        JsTsArtefact {
            canonical_kind: Some("class".to_string()),
            language_kind: "class_declaration".to_string(),
            name: "Widget".to_string(),
            symbol_fqn: format!("{path}::Widget"),
            parent_symbol_fqn: None,
            start_line: 1,
            end_line: 20,
            start_byte: 0,
            end_byte: 200,
            signature: "export class Widget {}".to_string(),
            modifiers: vec!["export".to_string()],
            docstring: Some("Widget docs".to_string()),
        },
        JsTsArtefact {
            canonical_kind: Some("method".to_string()),
            language_kind: "method_definition".to_string(),
            name: "render".to_string(),
            symbol_fqn: format!("{path}::Widget::render"),
            parent_symbol_fqn: Some(format!("{path}::Widget")),
            start_line: 5,
            end_line: 10,
            start_byte: 40,
            end_byte: 120,
            signature: "render(): void {}".to_string(),
            modifiers: vec![],
            docstring: None,
        },
    ];

    let records = build_symbol_records(&cfg, path, blob_sha, &file, &items);
    assert_eq!(records.len(), 2);

    let class_record = &records[0];
    assert_eq!(class_record.parent_symbol_id, Some(file.symbol_id.clone()));
    assert_eq!(
        class_record.parent_artefact_id,
        Some(file.artefact_id.clone())
    );
    assert_eq!(class_record.docstring.as_deref(), Some("Widget docs"));

    let method_record = &records[1];
    assert_eq!(
        method_record.parent_symbol_id,
        Some(class_record.symbol_id.clone())
    );
    assert_eq!(
        method_record.parent_artefact_id,
        Some(class_record.artefact_id.clone())
    );
    assert_eq!(
        method_record.signature.as_deref(),
        Some("render(): void {}")
    );
}

#[test]
fn build_historical_edge_records_keep_resolved_and_unresolved_targets() {
    let cfg = test_cfg();
    let path = "src/main.ts";
    let blob_sha = "blob-2";
    let from = test_symbol_record(&cfg, path, blob_sha, "from-symbol", "source", 1, 2);
    let to = test_symbol_record(&cfg, path, blob_sha, "to-symbol", "target", 4, 5);
    let current_by_fqn = [
        (from.symbol_fqn.clone(), from.clone()),
        (to.symbol_fqn.clone(), to.clone()),
    ]
    .into_iter()
    .collect::<HashMap<_, _>>();

    let records = build_historical_edge_records(
        &cfg,
        blob_sha,
        "typescript",
        vec![
            test_call_edge(&from.symbol_fqn, &to.symbol_fqn, 7),
            test_unresolved_call_edge(&from.symbol_fqn, "remote::symbol", 9),
            test_call_edge("missing::from", &to.symbol_fqn, 11),
        ],
        &current_by_fqn,
    );

    assert_eq!(records.len(), 2);
    assert_eq!(
        records[0].to_symbol_id.as_deref(),
        Some(to.symbol_id.as_str())
    );
    assert_eq!(
        records[0].to_artefact_id.as_deref(),
        Some(to.artefact_id.as_str())
    );
    assert!(records[0].to_symbol_ref.is_none());
    assert!(records[1].to_symbol_id.is_none());
    assert!(records[1].to_artefact_id.is_none());
    assert_eq!(records[1].to_symbol_ref.as_deref(), Some("remote::symbol"));
}

#[test]
fn build_current_edge_records_resolve_local_and_external_targets() {
    let cfg = test_cfg();
    let path = "src/main.ts";
    let blob_sha = "blob-3";
    let from = test_symbol_record(&cfg, path, blob_sha, "from-symbol", "source", 1, 2);
    let to = test_symbol_record(&cfg, path, blob_sha, "to-symbol", "target", 4, 5);
    let current_by_fqn = [
        (from.symbol_fqn.clone(), from.clone()),
        (to.symbol_fqn.clone(), to.clone()),
    ]
    .into_iter()
    .collect::<HashMap<_, _>>();
    let external_targets = [(
        "pkg::remote".to_string(),
        (
            "external-symbol".to_string(),
            "external-artefact".to_string(),
        ),
    )]
    .into_iter()
    .collect::<HashMap<_, _>>();

    let records = build_current_edge_records(
        &cfg,
        "commit-3",
        blob_sha,
        "typescript",
        vec![
            test_call_edge(&from.symbol_fqn, &to.symbol_fqn, 7),
            test_unresolved_call_edge(&from.symbol_fqn, "pkg::remote", 8),
        ],
        &current_by_fqn,
        &external_targets,
    );

    assert_eq!(records.len(), 2);
    assert_eq!(
        records[0].to_symbol_id.as_deref(),
        Some(to.symbol_id.as_str())
    );
    assert_eq!(
        records[0].to_artefact_id.as_deref(),
        Some(to.artefact_id.as_str())
    );
    assert_eq!(records[1].to_symbol_id.as_deref(), Some("external-symbol"));
    assert_eq!(
        records[1].to_artefact_id.as_deref(),
        Some("external-artefact")
    );
    assert_eq!(records[1].to_symbol_ref.as_deref(), Some("pkg::remote"));
}

#[test]
fn incoming_revision_is_newer_prefers_newer_timestamp_then_sha() {
    assert!(incoming_revision_is_newer(None, "bbb", 10));
    assert!(incoming_revision_is_newer(
        Some(("aaa".to_string(), 9)),
        "bbb",
        10
    ));
    assert!(!incoming_revision_is_newer(
        Some(("zzz".to_string(), 11)),
        "bbb",
        10
    ));
    assert!(incoming_revision_is_newer(
        Some(("aaa".to_string(), 10)),
        "bbb",
        10
    ));
    assert!(!incoming_revision_is_newer(
        Some(("ccc".to_string(), 10)),
        "bbb",
        10
    ));
}

#[test]
fn default_branch_name_uses_current_branch_and_falls_back_to_main() {
    let repo = seed_git_repo();
    git_ok(repo.path(), &["checkout", "-B", "feature/test-branch"]);

    assert_eq!(default_branch_name(repo.path()), "feature/test-branch");
    assert_eq!(
        default_branch_name(tempdir().expect("temp dir").path()),
        "main"
    );
}

#[test]
fn collect_checkpoint_commit_map_prefers_newest_db_mapped_checkpoint_commit() {
    let repo = seed_git_repo();
    let checkpoint_id = "aabbccddeeff";

    git_ok(
        repo.path(),
        &["commit", "--allow-empty", "-m", "older checkpoint"],
    );
    let older_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    git_ok(
        repo.path(),
        &["commit", "--allow-empty", "-m", "newest checkpoint"],
    );
    let newest_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    insert_commit_checkpoint_mapping(repo.path(), &older_sha, checkpoint_id);
    insert_commit_checkpoint_mapping(repo.path(), &newest_sha, checkpoint_id);

    let checkpoint_map =
        collect_checkpoint_commit_map(repo.path()).expect("checkpoint commit map should build");

    assert_eq!(checkpoint_map.len(), 1);
    let info = checkpoint_map
        .get(checkpoint_id)
        .expect("checkpoint should be present");
    assert_eq!(info.subject, "newest checkpoint");
    assert!(!info.commit_sha.is_empty());
    assert!(info.commit_unix > 0);
}

#[test]
fn collect_checkpoint_commit_map_reads_commit_checkpoints_table() {
    let repo = seed_git_repo();
    let checkpoint_id = "b0b1b2b3b4b5";

    git_ok(
        repo.path(),
        &[
            "commit",
            "--allow-empty",
            "-m",
            "checkpoint without trailer",
        ],
    );
    let commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    insert_commit_checkpoint_mapping(repo.path(), &commit_sha, checkpoint_id);

    let checkpoint_map =
        collect_checkpoint_commit_map(repo.path()).expect("checkpoint commit map should build");

    assert_eq!(checkpoint_map.len(), 1);
    let info = checkpoint_map
        .get(checkpoint_id)
        .expect("checkpoint should be present");
    assert_eq!(info.commit_sha, commit_sha);
    assert_eq!(info.subject, "checkpoint without trailer");
    assert!(info.commit_unix > 0);
}

#[test]
fn parse_devql_pipeline_basic() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->asOf(ref:"main")->file("src/main.rs")->artefacts(lines:1..50,kind:"file",agent:"claude-code",since:"2026-03-01")->select(path,canonical_kind)->limit(10)"#,
    )
    .unwrap();

    assert_eq!(parsed.repo.as_deref(), Some("bitloops-cli"));
    assert!(matches!(parsed.as_of, Some(AsOfSelector::Ref(ref v)) if v == "main"));
    assert_eq!(parsed.file.as_deref(), Some("src/main.rs"));
    assert_eq!(parsed.artefacts.kind.as_deref(), Some("file"));
    assert_eq!(parsed.artefacts.lines, Some((1, 50)));
    assert_eq!(parsed.artefacts.agent.as_deref(), Some("claude-code"));
    assert_eq!(parsed.artefacts.since.as_deref(), Some("2026-03-01"));
    assert_eq!(parsed.limit, 10);
    assert_eq!(parsed.select_fields, vec!["path", "canonical_kind"]);
}

#[test]
fn parse_devql_artefacts_symbol_fqn_filter() {
    let parsed = parse_devql_query(
        r#"repo("rust-example")->artefacts(kind:"method",symbol_fqn:"hello_rust/src/main.rs::impl@1::handle_factorial")->limit(5)"#,
    )
    .unwrap();

    assert_eq!(parsed.artefacts.kind.as_deref(), Some("method"));
    assert_eq!(
        parsed.artefacts.symbol_fqn.as_deref(),
        Some("hello_rust/src/main.rs::impl@1::handle_factorial")
    );
}

#[test]
fn parse_devql_checkpoints_basic() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->checkpoints(agent:"claude-code",since:"2026-03-01")->select(checkpoint_id,created_at)->limit(5)"#,
    )
    .unwrap();

    assert!(parsed.has_checkpoints_stage);
    assert_eq!(parsed.checkpoints.agent.as_deref(), Some("claude-code"));
    assert_eq!(parsed.checkpoints.since.as_deref(), Some("2026-03-01"));
    assert_eq!(parsed.limit, 5);
}

#[test]
fn parse_devql_chat_history_stage_basic() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->file("index.ts")->artefacts(lines:1..10)->chatHistory()->limit(3)"#,
    )
    .unwrap();

    assert!(parsed.has_artefacts_stage);
    assert!(parsed.has_chat_history_stage);
    assert_eq!(parsed.limit, 3);
}

#[test]
fn parse_devql_deps_stage_basic() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->file("src/main.ts")->artefacts(kind:"function")->deps(kind:"calls",direction:"both",include_unresolved:false)->limit(25)"#,
    )
    .unwrap();

    assert!(parsed.has_deps_stage);
    assert_eq!(parsed.deps.kind.as_deref(), Some("calls"));
    assert_eq!(parsed.deps.direction, "both");
    assert!(!parsed.deps.include_unresolved);
    assert_eq!(parsed.limit, 25);
}

#[test]
fn parse_devql_semantic_neighbors_stage_basic() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts(kind:"function")->semanticNeighbors()->limit(7)"#,
    )
    .unwrap();

    assert!(parsed.has_artefacts_stage);
    assert!(parsed.has_semantic_neighbors_stage);
    assert_eq!(parsed.limit, 7);
}

#[tokio::test]
async fn execute_devql_query_rejects_combining_deps_and_checkpoints_stage() {
    let cfg = test_cfg();
    let parsed = parse_devql_query(r#"repo("temp2")->checkpoints()->artefacts()->deps()"#).unwrap();
    let err = execute_devql_query(&cfg, &parsed, None).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("MVP limitation: telemetry/checkpoints stages cannot be combined")
    );
}

#[test]
fn parse_devql_deps_stage_accepts_all_v1_edge_kinds() {
    for kind in [
        "imports",
        "calls",
        "references",
        "inherits",
        "implements",
        "exports",
    ] {
        let parsed = parse_devql_query(&format!(
            r#"repo("bitloops-cli")->artefacts(kind:"function")->deps(kind:"{kind}")->limit(5)"#
        ))
        .unwrap();

        assert_eq!(parsed.deps.kind.as_deref(), Some(kind));
    }
}

#[test]
fn build_postgres_deps_query_respects_direction_and_unresolved_filters() {
    let cfg = test_cfg();
    let out = parse_devql_query(
        r#"repo("bitloops-cli")->file("src/main.ts")->artefacts(kind:"function")->deps(kind:"calls",direction:"out",include_unresolved:false)->limit(5)"#,
    )
    .unwrap();
    let in_query = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts(kind:"interface")->deps(kind:"references",direction:"in")->limit(5)"#,
    )
    .unwrap();
    let both = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts()->deps(kind:"exports",direction:"both")->limit(5)"#,
    )
    .unwrap();

    let out_sql = build_postgres_deps_query(&cfg, &out, &cfg.repo.repo_id).unwrap();
    let in_sql = build_postgres_deps_query(&cfg, &in_query, &cfg.repo.repo_id).unwrap();
    let both_sql = build_postgres_deps_query(&cfg, &both, &cfg.repo.repo_id).unwrap();

    assert!(out_sql.contains("e.edge_kind = 'calls'"));
    assert!(out_sql.contains("e.to_artefact_id IS NOT NULL"));
    assert!(
        out_sql.contains("LEFT JOIN artefacts_current at ON at.artefact_id = e.to_artefact_id")
    );
    assert!(!out_sql.contains(" a."));

    assert!(in_sql.contains("e.edge_kind = 'references'"));
    assert!(in_sql.contains("JOIN artefacts_current at ON at.artefact_id = e.to_artefact_id"));
    assert!(!in_sql.contains("WITH out_edges AS"));

    assert!(both_sql.contains("e.edge_kind = 'exports'"));
    assert!(both_sql.contains("FROM artefact_edges_current e JOIN artefacts_current a"));
    assert!(both_sql.contains("WITH out_edges AS"));
    assert!(both_sql.contains("UNION ALL"));
    assert!(both_sql.contains("SELECT DISTINCT"));
}

#[test]
fn build_postgres_deps_query_uses_historical_tables_for_asof_queries() {
    let cfg = test_cfg();
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->asOf(commit:"abc123")->file("src/main.ts")->artefacts(kind:"function")->deps(kind:"calls")->limit(5)"#,
    )
    .unwrap();

    let sql = build_postgres_deps_query(&cfg, &parsed, &cfg.repo.repo_id).unwrap();

    assert!(sql.contains("FROM artefact_edges e"));
    assert!(sql.contains("JOIN artefacts af ON af.artefact_id = e.from_artefact_id"));
    assert!(sql.contains("LEFT JOIN artefacts at ON at.artefact_id = e.to_artefact_id"));
    assert!(!sql.contains("artefact_edges_current"));
    assert!(!sql.contains("artefacts_current"));
}

#[tokio::test]
async fn build_postgres_artefacts_query_includes_language_kind_and_symbol_fqn_filter() {
    let cfg = test_cfg();
    let parsed = parse_devql_query(
        r#"repo("temp2")->file("hello_rust/src/main.rs")->artefacts(kind:"method",symbol_fqn:"hello_rust/src/main.rs::impl@1::handle_factorial")->limit(10)"#,
    )
    .unwrap();

    let sql = build_postgres_artefacts_query(&cfg, &parsed, None, &cfg.repo.repo_id)
        .await
        .unwrap();

    assert!(sql.contains("a.language_kind"));
    assert!(sql.contains("a.modifiers"));
    assert!(sql.contains("a.docstring"));
    assert!(sql.contains("a.symbol_fqn = 'hello_rust/src/main.rs::impl@1::handle_factorial'"));
    assert!(sql.contains("FROM artefacts_current a"));
}

#[test]
fn build_postgres_deps_query_supports_symbol_fqn_filter() {
    let cfg = test_cfg();
    let parsed = parse_devql_query(
        r#"repo("rust-example")->artefacts(kind:"method",symbol_fqn:"hello_rust/src/main.rs::impl@1::handle_factorial")->deps(kind:"calls",direction:"out")->limit(20)"#,
    )
    .unwrap();

    let sql = build_postgres_deps_query(&cfg, &parsed, &cfg.repo.repo_id).unwrap();

    assert!(sql.contains("af.symbol_fqn = 'hello_rust/src/main.rs::impl@1::handle_factorial'"));
}

#[test]
fn build_postgres_deps_query_rejects_invalid_direction() {
    let cfg = test_cfg();
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts()->deps(kind:"calls",direction:"sideways")->limit(5)"#,
    )
    .unwrap();

    let err = build_postgres_deps_query(&cfg, &parsed, &cfg.repo.repo_id).unwrap_err();
    assert!(
        err.to_string()
            .contains("deps(direction:...) must be one of: out, in, both")
    );
}

#[test]
fn build_postgres_deps_query_rejects_invalid_kind() {
    let cfg = test_cfg();
    let parsed =
        parse_devql_query(r#"repo("bitloops-cli")->artefacts()->deps(kind:"surprise")->limit(5)"#)
            .unwrap();

    let err = build_postgres_deps_query(&cfg, &parsed, &cfg.repo.repo_id).unwrap_err();
    assert!(err.to_string().contains(
        "deps(kind:...) must be one of: imports, calls, references, inherits, implements, exports"
    ));
}

#[tokio::test]
async fn execute_devql_query_rejects_chat_history_without_artefacts_stage() {
    let cfg = test_cfg();
    let parsed = parse_devql_query(r#"repo("temp2")->chatHistory()->limit(1)"#).unwrap();
    let err = execute_devql_query(&cfg, &parsed, None).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("chatHistory() requires an artefacts() stage")
    );
}

#[tokio::test]
async fn execute_devql_query_rejects_combining_checkpoints_and_artefacts_stage() {
    let cfg = test_cfg();
    let parsed =
        parse_devql_query(r#"repo("temp2")->checkpoints()->artefacts(agent:"claude-code")"#)
            .unwrap();
    let err = execute_devql_query(&cfg, &parsed, None).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("MVP limitation: telemetry/checkpoints stages cannot be combined")
    );
}

#[tokio::test]
async fn execute_devql_query_rejects_combining_deps_and_chat_history_stage() {
    let cfg = test_cfg();
    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->deps(kind:"calls")->chatHistory()->limit(1)"#,
    )
    .unwrap();
    let err = execute_devql_query(&cfg, &parsed, None).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("deps() cannot be combined with chatHistory()")
    );
}

#[tokio::test]
async fn execute_devql_query_rejects_semantic_neighbors_without_artefacts_stage() {
    let cfg = test_cfg();
    let parsed = parse_devql_query(r#"repo("temp2")->semanticNeighbors()->limit(1)"#).unwrap();
    let err = execute_devql_query(&cfg, &parsed, None).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("semanticNeighbors() requires an artefacts() stage")
    );
}

#[tokio::test]
async fn execute_devql_query_rejects_combining_semantic_neighbors_and_deps_stage() {
    let cfg = test_cfg();
    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->deps(kind:"calls")->semanticNeighbors()->limit(1)"#,
    )
    .unwrap();
    let err = execute_devql_query(&cfg, &parsed, None).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("semanticNeighbors() cannot be combined with deps()")
    );
}

#[test]
fn build_postgres_semantic_neighbors_sql_uses_embedding_similarity() {
    let sql = build_postgres_semantic_neighbors_sql("repo-1", "artefact-1", 8);
    assert!(sql.contains("FROM symbol_embeddings src"));
    assert!(sql.contains("JOIN symbol_embeddings emb"));
    assert!(sql.contains("semantic_score"));
    assert!(sql.contains("LIMIT 8"));
}

#[test]
fn extract_chat_messages_from_transcript_parses_jsonl() {
    let transcript = r#"{"type":"user","message":{"content":"Fix index.ts"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Done"},{"type":"tool_use","name":"Edit"}]}}
{"type":"assistant","content":"Added tests"}"#;

    let messages = extract_chat_messages_from_transcript(transcript);
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["text"], "Fix index.ts");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[1]["text"], "Done");
    assert_eq!(messages[2]["text"], "Added tests");
}

#[test]
fn deterministic_uuid_is_stable() {
    let a = deterministic_uuid("same-input");
    let b = deterministic_uuid("same-input");
    let c = deterministic_uuid("different-input");

    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_eq!(a.len(), 36);
}

#[test]
fn devql_file_config_parses_nested_block() {
    let value = serde_json::json!({
        "devql": {
            "postgres_dsn": "postgres://user:pass@localhost:5432/bitloops",
            "clickhouse_url": "http://localhost:8123",
            "clickhouse_database": "default"
        }
    });

    let cfg = DevqlFileConfig::from_json_value(&value);
    assert_eq!(
        cfg.pg_dsn.as_deref(),
        Some("postgres://user:pass@localhost:5432/bitloops")
    );
    assert_eq!(cfg.clickhouse_url.as_deref(), Some("http://localhost:8123"));
    assert_eq!(cfg.clickhouse_database.as_deref(), Some("default"));
}

#[test]
fn devql_file_config_parses_top_level_env_keys() {
    let value = serde_json::json!({
        "BITLOOPS_DEVQL_PG_DSN": "postgres://x/y",
        "BITLOOPS_DEVQL_CH_URL": "http://ch:8123",
        "BITLOOPS_DEVQL_CH_DATABASE": "analytics"
    });

    let cfg = DevqlFileConfig::from_json_value(&value);
    assert_eq!(cfg.pg_dsn.as_deref(), Some("postgres://x/y"));
    assert_eq!(cfg.clickhouse_url.as_deref(), Some("http://ch:8123"));
    assert_eq!(cfg.clickhouse_database.as_deref(), Some("analytics"));
}

#[tokio::test]
async fn connection_status_rows_report_connected_for_sqlite_and_duckdb_files() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("relational.sqlite");
    let duckdb_path = temp.path().join("events.duckdb");

    create_sqlite_db(&sqlite_path);
    create_duckdb_db(&duckdb_path);

    let cfg = backend_cfg(
        Some(sqlite_path.display().to_string()),
        Some(duckdb_path.display().to_string()),
    );

    let rows = collect_connection_status_rows(&cfg).await;

    assert_eq!(
        status_for(&rows, RELATIONAL_SQLITE_LABEL),
        DatabaseConnectionStatus::Connected
    );
    assert_eq!(
        status_for(&rows, EVENTS_DUCKDB_LABEL),
        DatabaseConnectionStatus::Connected
    );
}

#[tokio::test]
async fn connection_status_rows_report_error_for_invalid_sqlite_and_duckdb_paths() {
    let temp = tempdir().expect("temp dir");
    let invalid_path = temp.path().display().to_string();
    let cfg = backend_cfg(Some(invalid_path.clone()), Some(invalid_path));

    let rows = collect_connection_status_rows(&cfg).await;

    assert_eq!(
        status_for(&rows, RELATIONAL_SQLITE_LABEL),
        DatabaseConnectionStatus::Error
    );
    assert_eq!(
        status_for(&rows, EVENTS_DUCKDB_LABEL),
        DatabaseConnectionStatus::Error
    );
}

#[test]
fn classify_connection_error_authentication() {
    let status = classify_connection_error("psql failed: FATAL: password authentication failed");
    assert_eq!(status, DatabaseConnectionStatus::CouldNotAuthenticate);
}

#[test]
fn classify_connection_error_reachability() {
    let status =
        classify_connection_error("ClickHouse request failed: curl: (7) Failed to connect");
    assert_eq!(status, DatabaseConnectionStatus::CouldNotReachDb);
}

#[test]
fn classify_connection_error_unknown() {
    let status = classify_connection_error("unexpected database failure");
    assert_eq!(status, DatabaseConnectionStatus::Error);
}

#[test]
fn normalize_repo_path_removes_dot_prefix() {
    assert_eq!(normalize_repo_path("./index.ts"), "index.ts");
    assert_eq!(normalize_repo_path("index.ts"), "index.ts");
    assert_eq!(normalize_repo_path(".\\src\\index.ts"), "src/index.ts");
}

#[test]
fn build_path_candidates_includes_variants() {
    let candidates = build_path_candidates("./index.ts");
    assert!(candidates.contains(&"./index.ts".to_string()));
    assert!(candidates.contains(&"index.ts".to_string()));
}

#[test]
fn extract_js_ts_functions_detects_basic_function() {
    let content = r#"export function hello() {
  return "Hello World";
}
"#;
    let functions = extract_js_ts_functions(content).unwrap();
    assert_eq!(functions.len(), 1);
    assert_eq!(functions[0].name, "hello");
    assert_eq!(functions[0].start_line, 1);
    assert_eq!(functions[0].end_line, 3);
    assert_eq!(functions[0].start_byte, 0);
    assert_eq!(functions[0].end_byte as usize, content.len());
    assert_eq!(functions[0].signature, "export function hello() {");
}

#[test]
fn extract_js_ts_functions_detects_arrow_function_assignment() {
    let content = r#"export const hello = () => {
  return "Hello World";
}
"#;
    let functions = extract_js_ts_functions(content).unwrap();
    assert_eq!(functions.len(), 1);
    assert_eq!(functions[0].name, "hello");
    assert_eq!(functions[0].start_line, 1);
    assert_eq!(functions[0].end_line, 3);
    assert_eq!(functions[0].start_byte, 0);
    assert_eq!(functions[0].end_byte as usize, content.len());
    assert_eq!(functions[0].signature, "export const hello = () => {");
}

#[test]
fn extract_js_ts_artefacts_covers_phase1_kinds() {
    let content = r#"import { helper } from "./helper";
export interface User {
  id: string;
}
export type UserId = string;
export class Service {
  run(input: string) {
    return input;
  }
}
export const answer = 42;
export function greet(name: string) {
  return helper(name);
}
"#;

    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let kinds = artefacts
        .iter()
        .map(|a| a.canonical_kind.as_deref())
        .collect::<Vec<_>>();

    assert!(kinds.contains(&Some("import")));
    assert!(kinds.contains(&Some("interface")));
    assert!(kinds.contains(&Some("type")));
    assert!(kinds.contains(&None));
    assert!(kinds.contains(&Some("method")));
    assert!(kinds.contains(&Some("variable")));
    assert!(kinds.contains(&Some("function")));

    let class = artefacts
        .iter()
        .find(|a| a.language_kind == "class_declaration" && a.name == "Service")
        .expect("expected class artefact");
    assert_eq!(class.canonical_kind, None);

    let method = artefacts
        .iter()
        .find(|a| a.canonical_kind.as_deref() == Some("method") && a.name == "run")
        .expect("expected class method artefact");
    assert_eq!(
        method.parent_symbol_fqn.as_deref(),
        Some("src/sample.ts::Service")
    );
    assert_eq!(method.symbol_fqn, "src/sample.ts::Service::run");
}

#[test]
fn extract_js_ts_artefacts_emits_constructor_and_only_top_level_variables() {
    let content = r#"import { helper } from "./helper";
const cacheKey = "demo";
export const API_URL = "/v1";
interface User {
  id: string;
}
type UserId = string;
class Service {
  constructor(private readonly value: string) {}

  run() {
    const localOnly = this.value;
    return helper(localOnly);
  }
}
function boot() {
  const nestedOnly = 1;
  return nestedOnly;
}
"#;

    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();

    let constructor = artefacts
        .iter()
        .find(|a| a.language_kind == "constructor" && a.name == "constructor")
        .expect("expected constructor artefact");
    assert_eq!(constructor.canonical_kind, None);
    assert_eq!(
        constructor.parent_symbol_fqn.as_deref(),
        Some("src/sample.ts::Service")
    );

    assert!(
        artefacts
            .iter()
            .any(|a| a.language_kind == "variable_declarator" && a.name == "cacheKey")
    );
    assert!(
        artefacts
            .iter()
            .any(|a| a.language_kind == "variable_declarator" && a.name == "API_URL")
    );
    assert!(
        !artefacts
            .iter()
            .any(|a| a.language_kind == "variable_declarator" && a.name == "localOnly")
    );
    assert!(
        !artefacts
            .iter()
            .any(|a| a.language_kind == "variable_declarator" && a.name == "nestedOnly")
    );
}

#[test]
fn extract_js_ts_artefacts_returns_no_symbols_when_treesitter_parse_fails() {
    let content = "export function broken( {";

    let artefacts = extract_js_ts_artefacts(content, "src/broken.ts").unwrap();

    assert!(artefacts.is_empty());
}

#[test]
fn extract_js_ts_artefacts_collect_modifiers_for_methods_fields_and_variables() {
    let content = r#"/* class summary */
class Service {
  // field summary
  public static readonly value: string = "ok";

  // method summary
  public static async run() {
    return Promise.resolve();
  }
}

// variable summary
export const FLAG = "demo";
"#;

    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();

    let class = artefacts
        .iter()
        .find(|artefact| {
            artefact.language_kind == "class_declaration" && artefact.name == "Service"
        })
        .expect("expected class artefact");
    assert!(class.modifiers.is_empty());
    assert_eq!(class.docstring.as_deref(), Some("class summary"));

    let field = artefacts
        .iter()
        .find(|artefact| {
            artefact.language_kind == "public_field_definition" && artefact.name == "value"
        })
        .expect("expected field artefact");
    assert_eq!(
        field.modifiers,
        vec![
            "public".to_string(),
            "static".to_string(),
            "readonly".to_string()
        ]
    );
    assert_eq!(field.docstring.as_deref(), Some("field summary"));

    let method = artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "method_definition" && artefact.name == "run")
        .expect("expected method artefact");
    assert_eq!(
        method.modifiers,
        vec![
            "public".to_string(),
            "static".to_string(),
            "async".to_string()
        ]
    );
    assert_eq!(method.docstring.as_deref(), Some("method summary"));

    let variable = artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "variable_declarator" && artefact.name == "FLAG")
        .expect("expected variable artefact");
    assert_eq!(variable.modifiers, vec!["export".to_string()]);
    assert_eq!(variable.docstring.as_deref(), Some("variable summary"));
}

#[test]
fn extract_js_ts_artefacts_merge_mixed_docstring_comment_blocks() {
    let content = r#"// first line
// second line
/* block detail */
/** final detail */
export async function greet(name: string) {
  return name;
}
"#;

    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let function = artefacts
        .iter()
        .find(|artefact| {
            artefact.language_kind == "function_declaration" && artefact.name == "greet"
        })
        .expect("expected function artefact");

    assert_eq!(
        function.modifiers,
        vec!["export".to_string(), "async".to_string()]
    );
    assert_eq!(
        function.docstring.as_deref(),
        Some("first line\nsecond line\n\nblock detail\n\nfinal detail")
    );
}

#[test]
fn extract_js_ts_dependency_edges_resolves_imports_and_calls() {
    let content = r#"import { helper as extHelper } from "./utils";
function local() {
  return 1;
}
function caller() {
  local();
  extHelper();
}
"#;
    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let edges = extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap();

    assert!(edges.iter().any(|e| {
        e.edge_kind == "imports"
            && e.from_symbol_fqn == "src/sample.ts"
            && e.to_symbol_ref.as_deref() == Some("./utils")
    }));

    assert!(edges.iter().any(|e| {
        e.edge_kind == "calls"
            && e.from_symbol_fqn == "src/sample.ts::caller"
            && e.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::local")
    }));

    assert!(edges.iter().any(|e| {
        e.edge_kind == "calls"
            && e.from_symbol_fqn == "src/sample.ts::caller"
            && e.to_symbol_ref.as_deref() == Some("./utils::helper")
    }));
}

#[test]
fn extract_js_ts_dependency_edges_emits_unresolved_call_fallback() {
    let content = r#"function caller() {
  mystery();
}
"#;
    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let edges = extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap();

    assert!(edges.iter().any(|e| {
        e.edge_kind == "calls"
            && e.from_symbol_fqn == "src/sample.ts::caller"
            && e.to_symbol_ref.as_deref() == Some("src/sample.ts::mystery")
    }));
}

#[test]
fn extract_js_ts_dependency_edges_are_ordered_and_resolve_local_before_imports() {
    let content = r#"import defaultHelper, { helper as extHelper } from "./utils";
import "./setup";
function extHelper() {
  return 1;
}
function caller() {
  extHelper();
  defaultHelper();
  mystery();
}
"#;
    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let snapshot = |edges: &[JsTsDependencyEdge]| {
        edges
            .iter()
            .map(|edge| {
                let metadata = |field: &str| {
                    edge.metadata
                        .get(field)
                        .and_then(|value| value.as_str())
                        .unwrap_or("-")
                };
                format!(
                    "{}|{}|{}|{}|{}|{}|{}",
                    edge.edge_kind,
                    edge.from_symbol_fqn,
                    edge.to_target_symbol_fqn.as_deref().unwrap_or("-"),
                    edge.to_symbol_ref.as_deref().unwrap_or("-"),
                    edge.start_line.unwrap_or_default(),
                    metadata("import_form"),
                    metadata("resolution"),
                )
            })
            .collect::<Vec<_>>()
    };

    let edges = extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap();
    let edge_snapshot = snapshot(&edges);
    let repeated_snapshot =
        snapshot(&extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap());

    assert_eq!(
        edge_snapshot,
        vec![
            "imports|src/sample.ts|-|./utils|1|module|-".to_string(),
            "imports|src/sample.ts|-|./setup|2|side_effect|-".to_string(),
            "calls|src/sample.ts::caller|src/sample.ts::extHelper|-|7|-|local".to_string(),
            "calls|src/sample.ts::caller|-|./utils::default|8|-|import".to_string(),
            "calls|src/sample.ts::caller|-|src/sample.ts::mystery|9|-|unresolved".to_string(),
        ]
    );
    assert_eq!(edge_snapshot, repeated_snapshot);
}

#[test]
fn extract_js_ts_dependency_edges_emit_type_and_value_references_with_ref_kind() {
    let content = r#"interface User {
  id: string;
}
const DEFAULT_USER: User = { id: "1" };
function project(user: User): User {
  const current: User = DEFAULT_USER;
  return current;
}
"#;
    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let edges = extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap();

    let type_reference = edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "references"
                && edge.from_symbol_fqn == "src/sample.ts::project"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::User")
        })
        .expect("expected local type reference edge for User");
    assert_eq!(
        type_reference
            .metadata
            .get("ref_kind")
            .and_then(|value| value.as_str()),
        Some("type")
    );

    let value_reference = edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "references"
                && edge.from_symbol_fqn == "src/sample.ts::project"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::DEFAULT_USER")
        })
        .expect("expected local value reference edge for DEFAULT_USER");
    assert_eq!(
        value_reference
            .metadata
            .get("ref_kind")
            .and_then(|value| value.as_str()),
        Some("value")
    );
}

#[test]
fn extract_js_ts_dependency_edges_emit_inherits_for_extends_clauses() {
    let content = r#"class BaseService {}
class UserService extends BaseService {}

interface UserShape {
  id: string;
}

interface AdminShape extends UserShape {
  role: string;
}
"#;
    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let edges = extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap();

    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "inherits"
            && edge.from_symbol_fqn == "src/sample.ts::UserService"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::BaseService")
    }));
    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "inherits"
            && edge.from_symbol_fqn == "src/sample.ts::AdminShape"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::UserShape")
    }));
}

#[test]
fn extract_js_ts_dependency_edges_emit_exports_with_alias_distinct_dedup() {
    let content = r#"function helper() {
  return 1;
}

export { helper };
export { helper };
export { helper as helperAlias };
export { remoteFoo } from "./remote";
export { remoteFoo } from "./remote";
export { remoteFoo as remoteAlias } from "./remote";
"#;
    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let edges = extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap();
    let export_edges = edges
        .iter()
        .filter(|edge| edge.edge_kind == "exports")
        .collect::<Vec<_>>();

    assert_eq!(
        export_edges.len(),
        4,
        "expected duplicate export/re-export edges to collapse while alias-distinct exports stay separate"
    );

    assert_eq!(
        export_edges
            .iter()
            .filter(|edge| {
                edge.from_symbol_fqn == "src/sample.ts"
                    && edge.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::helper")
                    && edge
                        .metadata
                        .get("export_name")
                        .and_then(|value| value.as_str())
                        == Some("helper")
            })
            .count(),
        1,
        "duplicate local exports for the same alias should dedupe"
    );

    let local_alias = export_edges
        .iter()
        .find(|edge| {
            edge.from_symbol_fqn == "src/sample.ts"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::helper")
                && edge
                    .metadata
                    .get("export_name")
                    .and_then(|value| value.as_str())
                    == Some("helperAlias")
        })
        .expect("expected aliased local export edge for helperAlias");
    assert_eq!(
        local_alias
            .metadata
            .get("export_form")
            .and_then(|value| value.as_str()),
        Some("named")
    );

    assert_eq!(
        export_edges
            .iter()
            .filter(|edge| {
                edge.from_symbol_fqn == "src/sample.ts"
                    && edge.to_symbol_ref.as_deref() == Some("./remote::remoteFoo")
                    && edge
                        .metadata
                        .get("export_name")
                        .and_then(|value| value.as_str())
                        == Some("remoteFoo")
            })
            .count(),
        1,
        "duplicate re-exports for the same alias should dedupe"
    );

    let re_export_alias = export_edges
        .iter()
        .find(|edge| {
            edge.from_symbol_fqn == "src/sample.ts"
                && edge.to_symbol_ref.as_deref() == Some("./remote::remoteFoo")
                && edge
                    .metadata
                    .get("export_name")
                    .and_then(|value| value.as_str())
                    == Some("remoteAlias")
        })
        .expect("expected aliased re-export edge for remoteAlias");
    assert_eq!(
        re_export_alias
            .metadata
            .get("export_form")
            .and_then(|value| value.as_str()),
        Some("re_export")
    );
}

#[test]
fn extract_rust_artefacts_covers_phase1_kinds() {
    let content = r#"use std::fmt::Debug;

struct User {
    id: u64,
}

trait DoThing {
    fn do_it(&self);
}

impl DoThing for User {
    fn do_it(&self) {}
}

fn run() {
    println!("ok");
}
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let kinds = artefacts
        .iter()
        .map(|a| a.canonical_kind.as_deref())
        .collect::<Vec<_>>();
    assert!(kinds.contains(&Some("import")));
    assert!(kinds.contains(&None));
    assert!(kinds.contains(&Some("interface")));
    assert!(kinds.contains(&Some("method")));
    assert!(kinds.contains(&Some("function")));

    let trait_item = artefacts
        .iter()
        .find(|a| a.language_kind == "trait_item" && a.name == "DoThing")
        .expect("expected trait artefact");
    assert_eq!(trait_item.canonical_kind.as_deref(), Some("interface"));

    let struct_item = artefacts
        .iter()
        .find(|a| a.language_kind == "struct_item" && a.name == "User")
        .expect("expected struct artefact");
    assert_eq!(struct_item.canonical_kind, None);

    let impl_item = artefacts
        .iter()
        .find(|a| a.language_kind == "impl_item")
        .expect("expected impl artefact");
    assert_eq!(impl_item.canonical_kind, None);
}

#[test]
fn extract_rust_dependency_edges_emits_import_calls_and_implements() {
    let content = r#"use crate::math::sum;

trait DoThing { fn do_it(&self); }
struct User;

impl DoThing for User {
    fn do_it(&self) {
        sum(1, 2);
    }
}
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let edges = extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap();

    assert!(edges.iter().any(|e| {
        e.edge_kind == "imports" && e.to_symbol_ref.as_deref() == Some("crate::math::sum")
    }));
    assert!(
        edges
            .iter()
            .any(|e| e.edge_kind == "implements" && e.to_symbol_ref.as_deref() == Some("DoThing"))
    );
    assert!(edges.iter().any(|e| e.edge_kind == "calls"));
}

#[test]
fn extract_rust_dependency_edges_are_ordered_and_keep_local_resolution_stable() {
    let content = r#"use crate::math::sum;
fn sum() {}
trait DoThing { fn do_it(&self); }
struct User;
impl DoThing for User {
    fn do_it(&self) {
        sum();
        missing();
    }
}
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let snapshot = |edges: &[JsTsDependencyEdge]| {
        edges
            .iter()
            .map(|edge| {
                let metadata = |field: &str| {
                    edge.metadata
                        .get(field)
                        .and_then(|value| value.as_str())
                        .unwrap_or("-")
                };
                format!(
                    "{}|{}|{}|{}|{}|{}|{}",
                    edge.edge_kind,
                    edge.from_symbol_fqn,
                    edge.to_target_symbol_fqn.as_deref().unwrap_or("-"),
                    edge.to_symbol_ref.as_deref().unwrap_or("-"),
                    edge.start_line.unwrap_or_default(),
                    metadata("import_form"),
                    metadata("resolution"),
                )
            })
            .collect::<Vec<_>>()
    };

    let edges = extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap();
    let edge_snapshot = snapshot(&edges);
    let repeated_snapshot =
        snapshot(&extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap());

    assert_eq!(
        edge_snapshot,
        vec![
            "imports|src/lib.rs|-|crate::math::sum|1|use|-".to_string(),
            "implements|src/lib.rs::impl@5|-|DoThing|5|-|-".to_string(),
            "calls|src/lib.rs::impl@5::do_it|src/lib.rs::sum|-|7|-|local".to_string(),
        ]
    );
    assert_eq!(edge_snapshot, repeated_snapshot);
}

#[test]
fn extract_rust_dependency_edges_emit_type_and_value_references_with_ref_kind() {
    let content = r#"struct User;
const DEFAULT_USER: User = User;

fn project(user: User) -> User {
    let current: User = DEFAULT_USER;
    current
}
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let edges = extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap();

    let type_reference = edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "references"
                && edge.from_symbol_fqn == "src/lib.rs::project"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/lib.rs::User")
        })
        .expect("expected local type reference edge for User");
    assert_eq!(
        type_reference
            .metadata
            .get("ref_kind")
            .and_then(|value| value.as_str()),
        Some("type")
    );

    let value_reference = edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "references"
                && edge.from_symbol_fqn == "src/lib.rs::project"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/lib.rs::DEFAULT_USER")
        })
        .expect("expected local value reference edge for DEFAULT_USER");
    assert_eq!(
        value_reference
            .metadata
            .get("ref_kind")
            .and_then(|value| value.as_str()),
        Some("value")
    );
}

#[test]
fn extract_rust_artefacts_preserve_expected_method_and_function_line_ranges() {
    let content = r##"impl AppServer {
    fn handle_factorial(&self, input: &str) -> Response<std::io::Cursor<Vec<u8>>> {
        match input.parse::<u64>() {
            Ok(n) if n <= 20 => {
                let result = factorial(n);
                Response::from_string(format!("{}! = {}\n", n, result))
            }
            Ok(_) => Response::from_string("Error: n must be <= 20\n")
                .with_status_code(400),
            Err(_) => Response::from_string("Error: invalid number\n")
                .with_status_code(400),
        }
    }
}

fn factorial(n: u64) -> u64 {
    (1..=n).product()
}
"##;

    let artefacts = extract_rust_artefacts(content, "src/main.rs").unwrap();
    let method = artefacts
        .iter()
        .find(|artefact| artefact.symbol_fqn == "src/main.rs::impl@1::handle_factorial")
        .expect("expected handle_factorial method artefact");
    assert_eq!(method.start_line, 2);
    assert_eq!(method.end_line, 13);

    let function = artefacts
        .iter()
        .find(|artefact| artefact.symbol_fqn == "src/main.rs::factorial")
        .expect("expected factorial function artefact");
    assert_eq!(function.start_line, 16);
    assert_eq!(function.end_line, 18);
}

#[test]
fn extract_rust_artefacts_collect_modifiers_and_outer_docstrings() {
    let content = r#"/// repository contract
pub(crate) trait Repository {
    fn save(&self);
}

/** stores the cache */
pub(crate) static CACHE: &str = "demo";

/// runs the worker
pub async unsafe fn run() {}
"#;

    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();

    let trait_item = artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "trait_item" && artefact.name == "Repository")
        .expect("expected trait artefact");
    assert_eq!(trait_item.modifiers, vec!["pub(crate)".to_string()]);
    assert_eq!(trait_item.docstring.as_deref(), Some("repository contract"));

    let static_item = artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "static_item" && artefact.name == "CACHE")
        .expect("expected static artefact");
    assert_eq!(
        static_item.modifiers,
        vec!["pub(crate)".to_string(), "static".to_string()]
    );
    assert_eq!(static_item.docstring.as_deref(), Some("stores the cache"));

    let function = artefacts
        .iter()
        .find(|artefact| {
            artefact.language_kind == "function_item"
                && artefact.name == "run"
                && artefact.parent_symbol_fqn.is_none()
        })
        .expect("expected free function artefact");
    assert_eq!(
        function.modifiers,
        vec!["pub".to_string(), "async".to_string(), "unsafe".to_string()]
    );
    assert_eq!(function.docstring.as_deref(), Some("runs the worker"));
}

#[test]
fn extract_rust_inner_doc_comments_attach_to_file_and_module() {
    let content = r#"//! crate level docs
/*! more crate docs */
mod api {
    //! module docs
    /*! more module docs */
    pub fn call() {}
}
"#;

    assert_eq!(
        extract_rust_file_docstring(content).as_deref(),
        Some("crate level docs\n\nmore crate docs")
    );

    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let module = artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "mod_item" && artefact.name == "api")
        .expect("expected module artefact");

    assert_eq!(
        module.docstring.as_deref(),
        Some("module docs\n\nmore module docs")
    );
}

#[test]
fn extract_rust_dependency_edges_emit_inherits_for_supertraits() {
    let content = r#"trait Reader {}
trait Writer {}

trait Repository: Reader + Writer {
    fn load(&self);
}
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let edges = extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap();

    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "inherits"
            && edge.from_symbol_fqn == "src/lib.rs::Repository"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/lib.rs::Reader")
    }));
    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "inherits"
            && edge.from_symbol_fqn == "src/lib.rs::Repository"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/lib.rs::Writer")
    }));
}

#[test]
fn extract_rust_dependency_edges_emit_pub_use_exports_with_alias_distinct_dedup() {
    let content = r#"pub fn helper() {}

pub use self::helper;
pub use self::helper;
pub use self::helper as helper_alias;
pub use crate::support::Thing;
pub use crate::support::Thing;
pub use crate::support::Thing as RenamedThing;
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let edges = extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap();
    let export_edges = edges
        .iter()
        .filter(|edge| edge.edge_kind == "exports")
        .collect::<Vec<_>>();

    assert_eq!(
        export_edges.len(),
        4,
        "expected duplicate pub use exports to collapse while alias-distinct exports stay separate"
    );

    assert_eq!(
        export_edges
            .iter()
            .filter(|edge| {
                edge.from_symbol_fqn == "src/lib.rs"
                    && edge.to_target_symbol_fqn.as_deref() == Some("src/lib.rs::helper")
                    && edge
                        .metadata
                        .get("export_name")
                        .and_then(|value| value.as_str())
                        == Some("helper")
            })
            .count(),
        1,
        "duplicate local pub use edges for the same alias should dedupe"
    );

    let local_alias = export_edges
        .iter()
        .find(|edge| {
            edge.from_symbol_fqn == "src/lib.rs"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/lib.rs::helper")
                && edge
                    .metadata
                    .get("export_name")
                    .and_then(|value| value.as_str())
                    == Some("helper_alias")
        })
        .expect("expected aliased local pub use edge for helper_alias");
    assert_eq!(
        local_alias
            .metadata
            .get("export_form")
            .and_then(|value| value.as_str()),
        Some("pub_use")
    );

    assert_eq!(
        export_edges
            .iter()
            .filter(|edge| {
                edge.from_symbol_fqn == "src/lib.rs"
                    && edge.to_symbol_ref.as_deref() == Some("crate::support::Thing")
                    && edge
                        .metadata
                        .get("export_name")
                        .and_then(|value| value.as_str())
                        == Some("Thing")
            })
            .count(),
        1,
        "duplicate external pub use edges for the same alias should dedupe"
    );

    let external_alias = export_edges
        .iter()
        .find(|edge| {
            edge.from_symbol_fqn == "src/lib.rs"
                && edge.to_symbol_ref.as_deref() == Some("crate::support::Thing")
                && edge
                    .metadata
                    .get("export_name")
                    .and_then(|value| value.as_str())
                    == Some("RenamedThing")
        })
        .expect("expected aliased external pub use edge for RenamedThing");
    assert_eq!(
        external_alias
            .metadata
            .get("export_form")
            .and_then(|value| value.as_str()),
        Some("pub_use")
    );
}

#[test]
fn extract_rust_dependency_edges_drop_unresolved_macro_calls_under_import_local_policy() {
    let content = r#"fn project() {
    println!("hi");
}
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let edges = extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap();

    assert!(
        !edges.iter().any(|edge| {
            edge.edge_kind == "calls" && edge.from_symbol_fqn == "src/lib.rs::project"
        }),
        "unresolved macro calls should be dropped under the import+local policy"
    );
}

#[test]
fn extract_rust_dependency_edges_keep_imported_helper_calls() {
    let content = r#"use crate::utils::slugify;

fn project(value: &str) {
    slugify(value);
}
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let edges = extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap();

    let imported_call = edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "calls"
                && edge.from_symbol_fqn == "src/lib.rs::project"
                && edge.to_symbol_ref.as_deref() == Some("crate::utils::slugify")
        })
        .expect("expected imported helper call edge");

    assert_eq!(
        imported_call
            .metadata
            .get("call_form")
            .and_then(|value| value.as_str()),
        Some("function")
    );
    assert_eq!(
        imported_call
            .metadata
            .get("resolution")
            .and_then(|value| value.as_str()),
        Some("import")
    );
    assert_eq!(imported_call.start_line, Some(4));
    assert_eq!(imported_call.end_line, Some(4));
}

#[test]
fn extract_rust_dependency_edges_keep_local_call_and_drop_external_noise_in_method() {
    let content = r##"impl AppServer {
    fn handle_factorial(&self, input: &str) -> Response<std::io::Cursor<Vec<u8>>> {
        match input.parse::<u64>() {
            Ok(n) if n <= 20 => {
                let result = factorial(n);
                Response::from_string(format!("{}! = {}\n", n, result))
            }
            Ok(_) => Response::from_string("Error: n must be <= 20\n")
                .with_status_code(400),
            Err(_) => Response::from_string("Error: invalid number\n")
                .with_status_code(400),
        }
    }
}

fn factorial(n: u64) -> u64 {
    (1..=n).product()
}
"##;
    let artefacts = extract_rust_artefacts(content, "src/main.rs").unwrap();
    let edges = extract_rust_dependency_edges(content, "src/main.rs", &artefacts).unwrap();

    let local_factorial_call = edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "calls"
                && edge.from_symbol_fqn == "src/main.rs::impl@1::handle_factorial"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/main.rs::factorial")
        })
        .expect("expected local factorial call edge");

    assert_eq!(
        local_factorial_call
            .metadata
            .get("call_form")
            .and_then(|value| value.as_str()),
        Some("function")
    );
    assert_eq!(
        local_factorial_call
            .metadata
            .get("resolution")
            .and_then(|value| value.as_str()),
        Some("local")
    );
    assert_eq!(local_factorial_call.start_line, Some(5));
    assert_eq!(local_factorial_call.end_line, Some(5));

    assert!(
        !edges.iter().any(|edge| {
            edge.edge_kind == "calls"
                && edge.to_symbol_ref.as_deref().is_some_and(|target| {
                    target.contains("::<u64>")
                        || target.contains("from_string(\"")
                        || target.contains(".with_status_code")
                })
        }),
        "rust call edges should not contain generic fragments or chained receiver text"
    );

    assert!(
        !edges.iter().any(|edge| {
            edge.edge_kind == "calls"
                && edge
                    .metadata
                    .get("resolution")
                    .and_then(|value| value.as_str())
                    == Some("unresolved")
        }),
        "unresolved rust call edges should be filtered out under the import+local policy"
    );
}

#[test]
fn symbol_id_is_stable_when_impl_block_moves_lines() {
    let original = JsTsArtefact {
        canonical_kind: None,
        language_kind: "impl_item".to_string(),
        name: "impl@12".to_string(),
        symbol_fqn: "src/lib.rs::impl@12".to_string(),
        parent_symbol_fqn: None,
        start_line: 12,
        end_line: 18,
        start_byte: 0,
        end_byte: 0,
        signature: "impl Repo for PgRepo {".to_string(),
        modifiers: vec![],
        docstring: None,
    };
    let moved = JsTsArtefact {
        name: "impl@30".to_string(),
        symbol_fqn: "src/lib.rs::impl@30".to_string(),
        start_line: 30,
        end_line: 36,
        ..original.clone()
    };

    assert_eq!(
        structural_symbol_id_for_artefact(&original, None),
        structural_symbol_id_for_artefact(&moved, None)
    );
}

#[test]
fn revision_artefact_id_changes_per_blob_for_same_symbol() {
    let symbol_id = deterministic_uuid("stable-symbol");

    assert_eq!(
        revision_artefact_id("repo-1", "blob-a", &symbol_id),
        revision_artefact_id("repo-1", "blob-a", &symbol_id)
    );
    assert_ne!(
        revision_artefact_id("repo-1", "blob-a", &symbol_id),
        revision_artefact_id("repo-1", "blob-b", &symbol_id)
    );
}

#[test]
fn reingestion_is_idempotent_for_unchanged_js_symbols() {
    let content = r#"export function greet(name: string) {
  return name.trim();
}
"#;
    let first = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let second = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();

    let first_fn = first
        .iter()
        .find(|artefact| artefact.symbol_fqn == "src/sample.ts::greet")
        .expect("expected greet artefact in first ingest");
    let second_fn = second
        .iter()
        .find(|artefact| artefact.symbol_fqn == "src/sample.ts::greet")
        .expect("expected greet artefact in second ingest");

    let first_symbol_id = symbol_id_for_artefact(first_fn);
    let second_symbol_id = symbol_id_for_artefact(second_fn);
    assert_eq!(first_symbol_id, second_symbol_id);
    assert_eq!(
        revision_artefact_id("repo-1", "blob-a", &first_symbol_id),
        revision_artefact_id("repo-1", "blob-a", &second_symbol_id)
    );
}

#[test]
fn reingestion_preserves_symbol_continuity_across_rust_line_moves() {
    let original = r#"struct Repo;
trait Service {
    fn run(&self);
}
impl Service for Repo {
    fn run(&self) {}
}
"#;
    let moved = r#"

struct Repo;

trait Service {
    fn run(&self);
}

impl Service for Repo {
    fn run(&self) {}
}
"#;

    let original_artefacts = extract_rust_artefacts(original, "src/lib.rs").unwrap();
    let moved_artefacts = extract_rust_artefacts(moved, "src/lib.rs").unwrap();

    let original_impl = original_artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "impl_item")
        .expect("expected impl artefact in original ingest");
    let moved_impl = moved_artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "impl_item")
        .expect("expected impl artefact in moved ingest");
    let original_impl_symbol_id = structural_symbol_id_for_artefact(original_impl, None);
    let moved_impl_symbol_id = structural_symbol_id_for_artefact(moved_impl, None);
    assert_eq!(original_impl_symbol_id, moved_impl_symbol_id);

    let original_method = original_artefacts
        .iter()
        .find(|artefact| {
            artefact.canonical_kind.as_deref() == Some("method") && artefact.name == "run"
        })
        .expect("expected run method in original ingest");
    let moved_method = moved_artefacts
        .iter()
        .find(|artefact| {
            artefact.canonical_kind.as_deref() == Some("method") && artefact.name == "run"
        })
        .expect("expected run method in moved ingest");

    let original_method_symbol_id =
        structural_symbol_id_for_artefact(original_method, Some(&original_impl_symbol_id));
    let moved_method_symbol_id =
        structural_symbol_id_for_artefact(moved_method, Some(&moved_impl_symbol_id));

    assert_eq!(original_method_symbol_id, moved_method_symbol_id);
    assert_ne!(
        revision_artefact_id("repo-1", "blob-a", &original_method_symbol_id),
        revision_artefact_id("repo-1", "blob-b", &moved_method_symbol_id)
    );
}

#[test]
fn postgres_schema_sql_includes_artefact_edges_hardening() {
    let sql = postgres_schema_sql();
    assert!(sql.contains("symbol_id TEXT"));
    assert!(sql.contains("modifiers JSONB NOT NULL DEFAULT '[]'::jsonb"));
    assert!(sql.contains("docstring TEXT"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefacts_symbol_idx"));
    assert!(sql.contains("CREATE TABLE IF NOT EXISTS current_file_state"));
    assert!(sql.contains("CREATE TABLE IF NOT EXISTS artefacts_current"));
    assert!(sql.contains("CREATE TABLE IF NOT EXISTS artefact_edges_current"));
    assert!(sql.contains("PRIMARY KEY (repo_id, symbol_id)"));
    assert!(sql.contains("CREATE TABLE IF NOT EXISTS artefact_edges"));
    assert!(sql.contains("CONSTRAINT artefact_edges_target_chk"));
    assert!(sql.contains("CONSTRAINT artefact_edges_line_range_chk"));
    assert!(sql.contains("metadata JSONB DEFAULT '{}'::jsonb"));
    assert!(sql.contains("CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_natural_uq"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefact_edges_symbol_ref_idx"));
    assert!(sql.contains("CONSTRAINT artefact_edges_current_target_chk"));
    assert!(sql.contains("CONSTRAINT artefact_edges_current_line_range_chk"));
    assert!(sql.contains("CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_current_natural_uq"));
}

#[test]
fn artefact_edges_hardening_sql_includes_constraints_and_indexes() {
    let sql = artefact_edges_hardening_sql();
    assert!(sql.contains("ADD CONSTRAINT artefact_edges_target_chk"));
    assert!(sql.contains("ADD CONSTRAINT artefact_edges_line_range_chk"));
    assert!(sql.contains("CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_natural_uq"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefact_edges_symbol_ref_idx"));
}

#[test]
fn current_state_hardening_sql_includes_current_state_constraints_and_indexes() {
    let sql = current_state_hardening_sql();
    assert!(sql.contains("CREATE TABLE IF NOT EXISTS current_file_state"));
    assert!(sql.contains("ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS commit_sha TEXT"));
    assert!(sql.contains("ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS modifiers JSONB"));
    assert!(sql.contains("ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS docstring TEXT"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefacts_current_symbol_fqn_idx"));
    assert!(sql.contains("ADD CONSTRAINT artefact_edges_current_target_chk"));
    assert!(sql.contains("ADD CONSTRAINT artefact_edges_current_line_range_chk"));
    assert!(sql.contains("CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_current_natural_uq"));
}

#[test]
fn artefacts_upgrade_sql_adds_modifiers_and_docstring() {
    let sql = artefacts_upgrade_sql();
    assert!(sql.contains("ADD COLUMN IF NOT EXISTS modifiers JSONB"));
    assert!(sql.contains("ADD COLUMN IF NOT EXISTS docstring TEXT"));
    assert!(sql.contains("SET modifiers = '[]'::jsonb"));
    assert!(sql.contains("ALTER COLUMN modifiers SET NOT NULL"));
}

#[test]
fn incoming_revision_is_newer_rejects_older_commits_and_uses_commit_sha_as_tiebreaker() {
    assert!(incoming_revision_is_newer(None, "commit-b", 200));
    assert!(incoming_revision_is_newer(
        Some(("commit-a".to_string(), 100)),
        "commit-b",
        200
    ));
    assert!(incoming_revision_is_newer(
        Some(("commit-a".to_string(), 100)),
        "commit-b",
        100
    ));
    assert!(!incoming_revision_is_newer(
        Some(("commit-b".to_string(), 200)),
        "commit-a",
        100
    ));
    assert!(!incoming_revision_is_newer(
        Some(("commit-z".to_string(), 200)),
        "commit-a",
        200
    ));
}

#[test]
fn devql_ingest_accepts_explicit_false_for_init() {
    let parsed =
        crate::commands::Cli::try_parse_from(["bitloops", "devql", "ingest", "--init=false"])
            .expect("devql ingest should parse with explicit boolean value");

    let Some(crate::commands::Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Ingest(ingest)) = args.command else {
        panic!("expected devql ingest command");
    };

    assert!(!ingest.init);
}

#[test]
fn resolve_repo_id_for_query_is_strict_for_unknown_repo_names() {
    let cfg = test_cfg();

    let local = resolve_repo_id_for_query(&cfg, Some("temp2"));
    let unknown = resolve_repo_id_for_query(&cfg, Some("test2"));

    assert_eq!(local, cfg.repo.repo_id);
    assert_ne!(unknown, cfg.repo.repo_id);
}

#[test]
fn postgres_sslmode_validation_allows_default_dsn_without_sslmode() {
    let dsn = "postgres://user:pass@localhost:5432/bitloops";
    let pg_cfg: tokio_postgres::Config = dsn.parse().expect("valid dsn");
    assert!(matches!(pg_cfg.get_ssl_mode(), SslMode::Prefer));
    validate_postgres_sslmode_for_notls(dsn, pg_cfg.get_ssl_mode()).expect("prefer is allowed");
}

#[test]
fn postgres_sslmode_validation_rejects_require() {
    let dsn = "postgres://user:pass@localhost:5432/bitloops?sslmode=require";
    let pg_cfg: tokio_postgres::Config = dsn.parse().expect("valid dsn");
    let err = validate_postgres_sslmode_for_notls(dsn, pg_cfg.get_ssl_mode()).unwrap_err();
    assert!(
        err.to_string()
            .contains("Postgres DSN requires TLS (sslmode=Require)")
    );
}

#[test]
fn postgres_sslmode_validation_rejects_verify_full_dsn() {
    let dsn = "postgres://user:pass@localhost:5432/bitloops?sslmode=verify-full";
    let err = validate_postgres_sslmode_for_notls(dsn, SslMode::Prefer).unwrap_err();
    assert!(err.to_string().contains("sslmode=verify-ca/verify-full"));
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn artefact_edges_constraints_and_dedup_work_in_postgres() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let cfg = test_cfg();
    init_postgres_schema(&cfg, &client).await.unwrap();

    let artefact_id = deterministic_uuid("test-art-a");
    let symbol_id = deterministic_uuid("test-symbol-a");
    let upsert_artefact_sql = format!(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash) \
VALUES ('{}', '{}', '{}', 'blob1', 'src/a.ts', 'typescript', 'function', 'function_declaration', 'src/a.ts::a', NULL, 1, 3, 0, 10, 'function a() {{', 'h1') \
ON CONFLICT (artefact_id) DO NOTHING",
        esc_pg(&artefact_id),
        esc_pg(&symbol_id),
        esc_pg(&cfg.repo.repo_id)
    );
    postgres_exec(&client, &upsert_artefact_sql).await.unwrap();

    let invalid_target_sql = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, edge_kind, language) \
VALUES ('{}', '{}', 'blob1', '{}', 'calls', 'typescript')",
        esc_pg(&deterministic_uuid("invalid-target")),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&artefact_id),
    );
    assert!(postgres_exec(&client, &invalid_target_sql).await.is_err());

    let invalid_range_sql = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line) \
VALUES ('{}', '{}', 'blob1', '{}', 'x', 'calls', 'typescript', 4, 3)",
        esc_pg(&deterministic_uuid("invalid-range")),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&artefact_id),
    );
    assert!(postgres_exec(&client, &invalid_range_sql).await.is_err());

    let edge_id_a = deterministic_uuid("dedup-a");
    let edge_insert_a = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line) \
VALUES ('{}', '{}', 'blob1', '{}', 'src/a.ts::x', 'calls', 'typescript', 2, 2)",
        esc_pg(&edge_id_a),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&artefact_id),
    );
    postgres_exec(&client, &edge_insert_a).await.unwrap();

    let edge_id_b = deterministic_uuid("dedup-b");
    let edge_insert_b = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line) \
VALUES ('{}', '{}', 'blob1', '{}', 'src/a.ts::x', 'calls', 'typescript', 2, 2)",
        esc_pg(&edge_id_b),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&artefact_id),
    );
    assert!(postgres_exec(&client, &edge_insert_b).await.is_err());
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn artefact_rows_preserve_symbol_continuity_across_blobs_in_postgres() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let cfg = test_cfg();
    init_postgres_schema(&cfg, &client).await.unwrap();

    let symbol_id = deterministic_uuid("stable-function");
    let artefact_a = deterministic_uuid("stable-function-blob-a");
    let artefact_b = deterministic_uuid("stable-function-blob-b");

    let insert_a = format!(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash) \
VALUES ('{}', '{}', '{}', 'blob-a', 'src/a.ts', 'typescript', 'function', 'function_declaration', 'src/a.ts::greet', NULL, 1, 3, 0, 10, 'function greet() {{', 'h-a')",
        esc_pg(&artefact_a),
        esc_pg(&symbol_id),
        esc_pg(&cfg.repo.repo_id)
    );
    let insert_b = format!(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash) \
VALUES ('{}', '{}', '{}', 'blob-b', 'src/a.ts', 'typescript', 'function', 'function_declaration', 'src/a.ts::greet', NULL, 4, 6, 11, 24, 'function greet() {{', 'h-b')",
        esc_pg(&artefact_b),
        esc_pg(&symbol_id),
        esc_pg(&cfg.repo.repo_id)
    );

    postgres_exec(&client, &insert_a).await.unwrap();
    postgres_exec(&client, &insert_b).await.unwrap();

    let row = client
        .query_one(
            "SELECT COUNT(*) FROM artefacts WHERE repo_id = $1 AND symbol_id = $2",
            &[&cfg.repo.repo_id, &symbol_id],
        )
        .await
        .unwrap();
    let count: i64 = row.get(0);
    assert_eq!(
        count, 2,
        "expected both revisions to share the same symbol_id"
    );
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn current_snapshot_updates_lines_and_bytes_for_moved_js_symbol_while_history_is_preserved() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let mut cfg = test_cfg();
    cfg.pg_dsn = Some(dsn.clone());
    cfg.repo.repo_id = deterministic_uuid("repo://devql-current-snapshot-move");
    init_postgres_schema(&cfg, &client).await.unwrap();

    let path = "src/current_snapshot_move.ts";
    let commit_old = "commit-old";
    let commit_new = "commit-new";
    let blob_old = "blob-old";
    let blob_new = "blob-new";
    let file_symbol_id = file_symbol_id(path);
    let function_symbol_id = deterministic_uuid("stable-greet-symbol");

    let file_old = FileArtefactRow {
        artefact_id: revision_artefact_id(&cfg.repo.repo_id, blob_old, &file_symbol_id),
        symbol_id: file_symbol_id.clone(),
        language: "typescript".to_string(),
        end_line: 4,
        end_byte: 48,
    };
    let file_new = FileArtefactRow {
        artefact_id: revision_artefact_id(&cfg.repo.repo_id, blob_new, &file_symbol_id),
        symbol_id: file_symbol_id.clone(),
        language: "typescript".to_string(),
        end_line: 9,
        end_byte: 112,
    };

    let old_record = PersistedArtefactRecord {
        symbol_id: function_symbol_id.clone(),
        artefact_id: revision_artefact_id(&cfg.repo.repo_id, blob_old, &function_symbol_id),
        canonical_kind: Some("function".to_string()),
        language_kind: "function_declaration".to_string(),
        symbol_fqn: format!("{path}::greet"),
        parent_symbol_id: Some(file_symbol_id.clone()),
        parent_artefact_id: Some(file_old.artefact_id.clone()),
        start_line: 1,
        end_line: 3,
        start_byte: 0,
        end_byte: 35,
        signature: Some("export function greet(name: string) {".to_string()),
        modifiers: vec![],
        docstring: None,
        content_hash: "hash-old".to_string(),
    };
    let new_record = PersistedArtefactRecord {
        symbol_id: function_symbol_id.clone(),
        artefact_id: revision_artefact_id(&cfg.repo.repo_id, blob_new, &function_symbol_id),
        canonical_kind: Some("function".to_string()),
        language_kind: "function_declaration".to_string(),
        symbol_fqn: format!("{path}::greet"),
        parent_symbol_id: Some(file_symbol_id.clone()),
        parent_artefact_id: Some(file_new.artefact_id.clone()),
        start_line: 6,
        end_line: 9,
        start_byte: 58,
        end_byte: 111,
        signature: Some("export function greet(name: string) {".to_string()),
        modifiers: vec![],
        docstring: None,
        content_hash: "hash-new".to_string(),
    };

    upsert_file_state_row(&cfg, &client, commit_old, path, blob_old)
        .await
        .unwrap();
    upsert_file_state_row(&cfg, &client, commit_new, path, blob_new)
        .await
        .unwrap();
    persist_historical_artefact(
        &cfg,
        &client,
        path,
        blob_old,
        &file_old.language,
        &old_record,
    )
    .await
    .unwrap();
    persist_historical_artefact(
        &cfg,
        &client,
        path,
        blob_new,
        &file_new.language,
        &new_record,
    )
    .await
    .unwrap();

    refresh_current_state_for_path(
        &cfg,
        &client,
        &FileRevision {
            commit_sha: commit_old,
            commit_unix: 100,
            path,
            blob_sha: blob_old,
        },
        &file_old,
        None,
        std::slice::from_ref(&old_record),
        vec![],
    )
    .await
    .unwrap();
    refresh_current_state_for_path(
        &cfg,
        &client,
        &FileRevision {
            commit_sha: commit_new,
            commit_unix: 200,
            path,
            blob_sha: blob_new,
        },
        &file_new,
        None,
        std::slice::from_ref(&new_record),
        vec![],
    )
    .await
    .unwrap();

    let current_row = client
        .query_one(
            "SELECT artefact_id, start_line, end_line, start_byte, end_byte FROM artefacts_current WHERE repo_id = $1 AND symbol_id = $2",
            &[&cfg.repo.repo_id, &function_symbol_id],
        )
        .await
        .unwrap();
    let current_artefact_id: String = current_row.get(0);
    let current_start_line: i32 = current_row.get(1);
    let current_end_line: i32 = current_row.get(2);
    let current_start_byte: i32 = current_row.get(3);
    let current_end_byte: i32 = current_row.get(4);
    assert_eq!(current_artefact_id, new_record.artefact_id);
    assert_eq!(current_start_line, 6);
    assert_eq!(current_end_line, 9);
    assert_eq!(current_start_byte, 58);
    assert_eq!(current_end_byte, 111);

    let historical_count = client
        .query_one(
            "SELECT COUNT(*) FROM artefacts WHERE repo_id = $1 AND symbol_id = $2",
            &[&cfg.repo.repo_id, &function_symbol_id],
        )
        .await
        .unwrap();
    let historical_count: i64 = historical_count.get(0);
    assert_eq!(historical_count, 2);
    assert_ne!(old_record.artefact_id, new_record.artefact_id);

    let current_parsed = parse_devql_query(&format!(
        r#"repo("temp2")->file("{path}")->artefacts(kind:"function")->limit(10)"#
    ))
    .unwrap();
    let current_rows = execute_postgres_pipeline(&cfg, &current_parsed, &client)
        .await
        .unwrap();
    assert_eq!(current_rows.len(), 1);
    assert_eq!(
        current_rows[0]["artefact_id"],
        Value::String(new_record.artefact_id.clone())
    );
    assert_eq!(current_rows[0]["start_line"], Value::from(6));
    assert_eq!(current_rows[0]["end_line"], Value::from(9));
    assert_eq!(current_rows[0]["start_byte"], Value::from(58));
    assert_eq!(current_rows[0]["end_byte"], Value::from(111));

    let historical_parsed = parse_devql_query(&format!(
        r#"repo("temp2")->asOf(commit:"{commit_old}")->file("{path}")->artefacts(kind:"function")->limit(10)"#
    ))
    .unwrap();
    let historical_rows = execute_postgres_pipeline(&cfg, &historical_parsed, &client)
        .await
        .unwrap();
    assert_eq!(historical_rows.len(), 1);
    assert_eq!(
        historical_rows[0]["artefact_id"],
        Value::String(old_record.artefact_id.clone())
    );
    assert_eq!(historical_rows[0]["start_line"], Value::from(1));
    assert_eq!(historical_rows[0]["end_line"], Value::from(3));
    assert_eq!(historical_rows[0]["start_byte"], Value::from(0));
    assert_eq!(historical_rows[0]["end_byte"], Value::from(35));
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn older_current_refresh_does_not_clobber_newer_snapshot_for_the_same_path() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let cfg = test_cfg_with_repo_id("devql-current-snapshot-recency-guard", &dsn);
    init_postgres_schema(&cfg, &client).await.unwrap();

    let path = "src/recency_guard.ts";
    let symbol_id = deterministic_uuid("recency-guard-symbol");
    let old_blob = "blob-old";
    let new_blob = "blob-new";
    let old_file = test_file_row(&cfg, path, old_blob, 4, 48);
    let new_file = test_file_row(&cfg, path, new_blob, 8, 96);
    let old_record = test_symbol_record(&cfg, path, old_blob, &symbol_id, "greet", 1, 3);
    let new_record = test_symbol_record(&cfg, path, new_blob, &symbol_id, "greet", 5, 8);

    refresh_current_state_for_path(
        &cfg,
        &client,
        &FileRevision {
            commit_sha: "commit-new",
            commit_unix: 200,
            path,
            blob_sha: new_blob,
        },
        &new_file,
        None,
        std::slice::from_ref(&new_record),
        vec![],
    )
    .await
    .unwrap();
    refresh_current_state_for_path(
        &cfg,
        &client,
        &FileRevision {
            commit_sha: "commit-old",
            commit_unix: 100,
            path,
            blob_sha: old_blob,
        },
        &old_file,
        None,
        &[old_record],
        vec![],
    )
    .await
    .unwrap();

    let row = client
        .query_one(
            "SELECT commit_sha, blob_sha, artefact_id, start_line, end_line FROM artefacts_current WHERE repo_id = $1 AND symbol_id = $2",
            &[&cfg.repo.repo_id, &symbol_id],
        )
        .await
        .unwrap();
    let commit_sha: String = row.get(0);
    let blob_sha: String = row.get(1);
    let artefact_id: String = row.get(2);
    let start_line: i32 = row.get(3);
    let end_line: i32 = row.get(4);

    assert_eq!(commit_sha, "commit-new");
    assert_eq!(blob_sha, new_blob);
    assert_eq!(artefact_id, new_record.artefact_id);
    assert_eq!(start_line, 5);
    assert_eq!(end_line, 8);
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn refreshing_a_path_rebuilds_current_outgoing_edges_instead_of_accumulating_stale_ones() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let cfg = test_cfg_with_repo_id("devql-current-outgoing-edge-refresh", &dsn);
    init_postgres_schema(&cfg, &client).await.unwrap();

    let path = "src/caller.ts";
    let symbol_id = deterministic_uuid("caller-symbol");
    let old_blob = "blob-caller-old";
    let new_blob = "blob-caller-new";
    let old_file = test_file_row(&cfg, path, old_blob, 5, 60);
    let new_file = test_file_row(&cfg, path, new_blob, 5, 60);
    let old_record = test_symbol_record(&cfg, path, old_blob, &symbol_id, "caller", 1, 4);
    let new_record = test_symbol_record(&cfg, path, new_blob, &symbol_id, "caller", 1, 4);

    refresh_current_state_for_path(
        &cfg,
        &client,
        &FileRevision {
            commit_sha: "commit-1",
            commit_unix: 100,
            path,
            blob_sha: old_blob,
        },
        &old_file,
        None,
        std::slice::from_ref(&old_record),
        vec![test_unresolved_call_edge(
            &old_record.symbol_fqn,
            "src/lib.ts::old_target",
            2,
        )],
    )
    .await
    .unwrap();
    refresh_current_state_for_path(
        &cfg,
        &client,
        &FileRevision {
            commit_sha: "commit-2",
            commit_unix: 200,
            path,
            blob_sha: new_blob,
        },
        &new_file,
        None,
        std::slice::from_ref(&new_record),
        vec![test_unresolved_call_edge(
            &new_record.symbol_fqn,
            "src/lib.ts::new_target",
            3,
        )],
    )
    .await
    .unwrap();

    let rows = client
        .query(
            "SELECT to_symbol_ref, start_line FROM artefact_edges_current WHERE repo_id = $1 AND path = $2 ORDER BY start_line",
            &[&cfg.repo.repo_id, &path],
        )
        .await
        .unwrap();

    assert_eq!(rows.len(), 1);
    let to_symbol_ref: Option<String> = rows[0].get(0);
    let start_line: Option<i32> = rows[0].get(1);
    assert_eq!(to_symbol_ref.as_deref(), Some("src/lib.ts::new_target"));
    assert_eq!(start_line, Some(3));
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn deleting_a_current_symbol_removes_its_row_and_clears_inbound_edge_target_ids() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let cfg = test_cfg_with_repo_id("devql-current-delete-target", &dsn);
    init_postgres_schema(&cfg, &client).await.unwrap();

    let target_path = "src/target.ts";
    let caller_path = "src/caller.ts";
    let target_symbol_id = deterministic_uuid("delete-target-symbol");
    let caller_symbol_id = deterministic_uuid("delete-caller-symbol");
    let target_blob = "blob-target-present";
    let target_deleted_blob = "blob-target-deleted";
    let caller_blob = "blob-caller";
    let target_file = test_file_row(&cfg, target_path, target_blob, 4, 48);
    let target_deleted_file = test_file_row(&cfg, target_path, target_deleted_blob, 1, 12);
    let caller_file = test_file_row(&cfg, caller_path, caller_blob, 5, 60);
    let target_record = test_symbol_record(
        &cfg,
        target_path,
        target_blob,
        &target_symbol_id,
        "target",
        1,
        3,
    );
    let caller_record = test_symbol_record(
        &cfg,
        caller_path,
        caller_blob,
        &caller_symbol_id,
        "caller",
        1,
        4,
    );

    refresh_current_state_for_path(
        &cfg,
        &client,
        &FileRevision {
            commit_sha: "commit-target-1",
            commit_unix: 100,
            path: target_path,
            blob_sha: target_blob,
        },
        &target_file,
        None,
        std::slice::from_ref(&target_record),
        vec![],
    )
    .await
    .unwrap();
    refresh_current_state_for_path(
        &cfg,
        &client,
        &FileRevision {
            commit_sha: "commit-caller-1",
            commit_unix: 110,
            path: caller_path,
            blob_sha: caller_blob,
        },
        &caller_file,
        None,
        std::slice::from_ref(&caller_record),
        vec![test_call_edge(
            &caller_record.symbol_fqn,
            &target_record.symbol_fqn,
            2,
        )],
    )
    .await
    .unwrap();
    refresh_current_state_for_path(
        &cfg,
        &client,
        &FileRevision {
            commit_sha: "commit-target-2",
            commit_unix: 200,
            path: target_path,
            blob_sha: target_deleted_blob,
        },
        &target_deleted_file,
        None,
        &[],
        vec![],
    )
    .await
    .unwrap();

    let target_count = client
        .query_one(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = $1 AND symbol_id = $2",
            &[&cfg.repo.repo_id, &target_symbol_id],
        )
        .await
        .unwrap();
    let target_count: i64 = target_count.get(0);
    assert_eq!(target_count, 0);

    let edge = client
        .query_one(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref FROM artefact_edges_current WHERE repo_id = $1 AND path = $2",
            &[&cfg.repo.repo_id, &caller_path],
        )
        .await
        .unwrap();
    let to_symbol_id: Option<String> = edge.get(0);
    let to_artefact_id: Option<String> = edge.get(1);
    let to_symbol_ref: Option<String> = edge.get(2);
    assert!(to_symbol_id.is_none());
    assert!(to_artefact_id.is_none());
    assert_eq!(
        to_symbol_ref.as_deref(),
        Some(target_record.symbol_fqn.as_str())
    );
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn cross_file_current_edges_resolve_targets_and_retarget_after_target_refresh() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let cfg = test_cfg_with_repo_id("devql-current-cross-file-resolution", &dsn);
    init_postgres_schema(&cfg, &client).await.unwrap();

    let target_path = "src/lib.ts";
    let caller_path = "src/app.ts";
    let target_symbol_id = deterministic_uuid("cross-file-target-symbol");
    let caller_symbol_id = deterministic_uuid("cross-file-caller-symbol");
    let target_blob_v1 = "blob-lib-v1";
    let target_blob_v2 = "blob-lib-v2";
    let caller_blob = "blob-app-v1";
    let target_file_v1 = test_file_row(&cfg, target_path, target_blob_v1, 4, 48);
    let target_file_v2 = test_file_row(&cfg, target_path, target_blob_v2, 6, 72);
    let caller_file = test_file_row(&cfg, caller_path, caller_blob, 5, 60);
    let target_record_v1 = test_symbol_record(
        &cfg,
        target_path,
        target_blob_v1,
        &target_symbol_id,
        "helper",
        1,
        3,
    );
    let target_record_v2 = test_symbol_record(
        &cfg,
        target_path,
        target_blob_v2,
        &target_symbol_id,
        "helper",
        3,
        6,
    );
    let caller_record = test_symbol_record(
        &cfg,
        caller_path,
        caller_blob,
        &caller_symbol_id,
        "caller",
        1,
        4,
    );

    refresh_current_state_for_path(
        &cfg,
        &client,
        &FileRevision {
            commit_sha: "commit-lib-1",
            commit_unix: 100,
            path: target_path,
            blob_sha: target_blob_v1,
        },
        &target_file_v1,
        None,
        std::slice::from_ref(&target_record_v1),
        vec![],
    )
    .await
    .unwrap();
    refresh_current_state_for_path(
        &cfg,
        &client,
        &FileRevision {
            commit_sha: "commit-app-1",
            commit_unix: 110,
            path: caller_path,
            blob_sha: caller_blob,
        },
        &caller_file,
        None,
        std::slice::from_ref(&caller_record),
        vec![test_call_edge(
            &caller_record.symbol_fqn,
            &target_record_v1.symbol_fqn,
            2,
        )],
    )
    .await
    .unwrap();

    let initial_edge = client
        .query_one(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref FROM artefact_edges_current WHERE repo_id = $1 AND path = $2",
            &[&cfg.repo.repo_id, &caller_path],
        )
        .await
        .unwrap();
    let initial_to_symbol_id: Option<String> = initial_edge.get(0);
    let initial_to_artefact_id: Option<String> = initial_edge.get(1);
    let initial_to_symbol_ref: Option<String> = initial_edge.get(2);
    assert_eq!(
        initial_to_symbol_id.as_deref(),
        Some(target_symbol_id.as_str())
    );
    assert_eq!(
        initial_to_artefact_id.as_deref(),
        Some(target_record_v1.artefact_id.as_str())
    );
    assert_eq!(
        initial_to_symbol_ref.as_deref(),
        Some(target_record_v1.symbol_fqn.as_str())
    );

    refresh_current_state_for_path(
        &cfg,
        &client,
        &FileRevision {
            commit_sha: "commit-lib-2",
            commit_unix: 200,
            path: target_path,
            blob_sha: target_blob_v2,
        },
        &target_file_v2,
        None,
        std::slice::from_ref(&target_record_v2),
        vec![],
    )
    .await
    .unwrap();

    let refreshed_edge = client
        .query_one(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref FROM artefact_edges_current WHERE repo_id = $1 AND path = $2",
            &[&cfg.repo.repo_id, &caller_path],
        )
        .await
        .unwrap();
    let refreshed_to_symbol_id: Option<String> = refreshed_edge.get(0);
    let refreshed_to_artefact_id: Option<String> = refreshed_edge.get(1);
    let refreshed_to_symbol_ref: Option<String> = refreshed_edge.get(2);
    assert_eq!(
        refreshed_to_symbol_id.as_deref(),
        Some(target_symbol_id.as_str())
    );
    assert_eq!(
        refreshed_to_artefact_id.as_deref(),
        Some(target_record_v2.artefact_id.as_str())
    );
    assert_eq!(
        refreshed_to_symbol_ref.as_deref(),
        Some(target_record_v2.symbol_fqn.as_str())
    );
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn export_edges_dedupe_same_alias_but_preserve_alias_distinct_in_postgres() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let cfg = test_cfg();
    init_postgres_schema(&cfg, &client).await.unwrap();

    let file_symbol_id = deterministic_uuid("file-symbol");
    let file_artefact_id = deterministic_uuid("file-artefact");
    let target_symbol_id = deterministic_uuid("target-symbol");
    let target_artefact_id = deterministic_uuid("target-artefact");

    let insert_file = format!(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash) \
VALUES ('{}', '{}', '{}', 'blob-exports', 'src/lib.ts', 'typescript', 'file', 'file', 'src/lib.ts', NULL, 1, 20, 0, 100, 'src/lib.ts', 'file-hash')",
        esc_pg(&file_artefact_id),
        esc_pg(&file_symbol_id),
        esc_pg(&cfg.repo.repo_id)
    );
    let insert_target = format!(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash) \
VALUES ('{}', '{}', '{}', 'blob-exports', 'src/lib.ts', 'typescript', 'function', 'function_declaration', 'src/lib.ts::helper', NULL, 2, 4, 10, 30, 'function helper() {{', 'target-hash')",
        esc_pg(&target_artefact_id),
        esc_pg(&target_symbol_id),
        esc_pg(&cfg.repo.repo_id)
    );

    postgres_exec(&client, &insert_file).await.unwrap();
    postgres_exec(&client, &insert_target).await.unwrap();

    let export_a = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_artefact_id, edge_kind, language, metadata) \
VALUES ('{}', '{}', 'blob-exports', '{}', '{}', 'exports', 'typescript', '{{\"export_name\":\"helper\",\"export_form\":\"named\",\"resolution\":\"local\"}}'::jsonb)",
        esc_pg(&deterministic_uuid("export-helper-a")),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&file_artefact_id),
        esc_pg(&target_artefact_id)
    );
    let export_dup = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_artefact_id, edge_kind, language, metadata) \
VALUES ('{}', '{}', 'blob-exports', '{}', '{}', 'exports', 'typescript', '{{\"export_name\":\"helper\",\"export_form\":\"named\",\"resolution\":\"local\"}}'::jsonb)",
        esc_pg(&deterministic_uuid("export-helper-b")),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&file_artefact_id),
        esc_pg(&target_artefact_id)
    );
    let export_alias = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_artefact_id, edge_kind, language, metadata) \
VALUES ('{}', '{}', 'blob-exports', '{}', '{}', 'exports', 'typescript', '{{\"export_name\":\"helperAlias\",\"export_form\":\"named\",\"resolution\":\"local\"}}'::jsonb)",
        esc_pg(&deterministic_uuid("export-helper-alias")),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&file_artefact_id),
        esc_pg(&target_artefact_id)
    );

    postgres_exec(&client, &export_a).await.unwrap();
    assert!(postgres_exec(&client, &export_dup).await.is_err());
    postgres_exec(&client, &export_alias).await.unwrap();

    let row = client
        .query_one(
            "SELECT COUNT(*) FROM artefact_edges WHERE repo_id = $1 AND edge_kind = 'exports'",
            &[&cfg.repo.repo_id],
        )
        .await
        .unwrap();
    let count: i64 = row.get(0);
    assert_eq!(
        count, 2,
        "expected alias-distinct export edges to survive dedup"
    );
}

#[test]
fn postgres_schema_sql_includes_checkpoint_migration_tables() {
    let schema = format!(
        "{}\n{}",
        postgres_schema_sql(),
        checkpoint_schema_sql_postgres()
    );
    for table in [
        "sessions",
        "temporary_checkpoints",
        "checkpoints",
        "checkpoint_sessions",
        "commit_checkpoints",
        "pre_prompt_states",
        "pre_task_markers",
        "checkpoint_blobs",
    ] {
        assert!(
            schema.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")),
            "expected checkpoint table `{table}` in postgres schema"
        );
    }
}
