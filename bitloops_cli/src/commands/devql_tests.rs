use super::*;
use crate::devql_config::DevqlFileConfig;
use clap::Parser;
use tempfile::TempDir;

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
        backends: crate::devql_config::DevqlBackendConfig {
            relational: crate::devql_config::RelationalBackendConfig {
                provider: crate::devql_config::RelationalProvider::Postgres,
                sqlite_path: None,
                postgres_dsn: Some("postgres://user:pass@localhost:5432/bitloops".to_string()),
            },
            events: crate::devql_config::EventsBackendConfig {
                provider: crate::devql_config::EventsProvider::ClickHouse,
                duckdb_path: None,
                clickhouse_url: Some("http://localhost:8123".to_string()),
                clickhouse_user: None,
                clickhouse_password: None,
                clickhouse_database: Some("default".to_string()),
            },
        },
    }
}

fn sqlite_test_cfg(repo_root: PathBuf, sqlite_path: String) -> DevqlConfig {
    DevqlConfig {
        repo_root,
        repo: RepoIdentity {
            provider: "local".to_string(),
            organization: "local".to_string(),
            name: "temp-sqlite".to_string(),
            identity: "local://local/temp-sqlite".to_string(),
            repo_id: deterministic_uuid("repo://local/temp-sqlite"),
        },
        backends: crate::devql_config::DevqlBackendConfig {
            relational: crate::devql_config::RelationalBackendConfig {
                provider: crate::devql_config::RelationalProvider::Sqlite,
                sqlite_path: Some(sqlite_path),
                postgres_dsn: None,
            },
            events: crate::devql_config::EventsBackendConfig {
                provider: crate::devql_config::EventsProvider::DuckDb,
                duckdb_path: None,
                clickhouse_url: None,
                clickhouse_user: None,
                clickhouse_password: None,
                clickhouse_database: None,
            },
        },
    }
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
fn query_backend_usage_for_checkpoints_is_events_only() {
    let parsed = parse_devql_query(r#"repo("bitloops-cli")->checkpoints()->limit(5)"#).unwrap();
    let usage = resolve_query_backend_usage(&parsed);

    assert!(!usage.uses_relational);
    assert!(usage.uses_events);
}

#[test]
fn query_backend_usage_for_simple_artefacts_is_relational_only() {
    let parsed =
        parse_devql_query(r#"repo("bitloops-cli")->file("src/main.rs")->artefacts()->limit(5)"#)
            .unwrap();
    let usage = resolve_query_backend_usage(&parsed);

    assert!(usage.uses_relational);
    assert!(!usage.uses_events);
}

#[test]
fn query_backend_usage_for_agent_filtered_artefacts_uses_both_backends() {
    let parsed =
        parse_devql_query(r#"repo("bitloops-cli")->artefacts(agent:"claude-code")->limit(5)"#)
            .unwrap();
    let usage = resolve_query_backend_usage(&parsed);

    assert!(usage.uses_relational);
    assert!(usage.uses_events);
}

#[test]
fn query_backend_usage_for_chat_history_uses_both_backends() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->file("index.ts")->artefacts(lines:1..10)->chatHistory()->limit(3)"#,
    )
    .unwrap();
    let usage = resolve_query_backend_usage(&parsed);

    assert!(usage.uses_relational);
    assert!(usage.uses_events);
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
async fn execute_devql_query_requires_relational_store_for_artefacts_stage() {
    let cfg = test_cfg();
    let parsed =
        parse_devql_query(r#"repo("temp2")->file("src/main.rs")->artefacts()->limit(1)"#).unwrap();
    let err = execute_devql_query(&cfg, &parsed, None).await.unwrap_err();
    assert!(err.to_string().contains("relational store is required"));
}

#[tokio::test]
async fn connection_status_reports_connected_for_sqlite_relational_provider() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("relational.sqlite");
    let cfg = sqlite_test_cfg(
        temp.path().to_path_buf(),
        db_path.to_string_lossy().to_string(),
    );

    let status = check_relational_connection_status(&DevqlConnectionConfig {
        backends: cfg.backends.clone(),
    })
    .await;
    assert_eq!(status, DatabaseConnectionStatus::Connected);
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
async fn sqlite_relational_store_supports_init_execute_and_query() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("relational.sqlite");
    let cfg = sqlite_test_cfg(
        temp.path().to_path_buf(),
        db_path.to_string_lossy().to_string(),
    );

    let store = connect_relational_store(&cfg)
        .await
        .expect("connect sqlite store");
    init_relational_schema(store.as_ref())
        .await
        .expect("init schema");

    let sql = format!(
        "INSERT INTO artefacts (artefact_id, repo_id, blob_sha, path, language, canonical_kind, start_line, end_line, content_hash) VALUES ('{}', '{}', 'blob1', 'src/main.rs', 'rust', 'file', 1, 10, 'blob1')",
        esc_pg("artifact-1"),
        esc_pg(&cfg.repo.repo_id),
    );
    store.execute(&sql).await.expect("insert artefact row");

    let rows = store
        .query_rows("SELECT path, canonical_kind, start_line FROM artefacts LIMIT 1")
        .await
        .expect("query artefact row");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["path"], "src/main.rs");
    assert_eq!(rows[0]["canonical_kind"], "file");
    assert_eq!(rows[0]["start_line"], 1);
}

#[tokio::test]
async fn run_init_and_ingest_work_with_sqlite_without_events_backend() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("relational.sqlite");
    let cfg = sqlite_test_cfg(
        temp.path().to_path_buf(),
        db_path.to_string_lossy().to_string(),
    );

    run_init(&cfg).await.expect("sqlite init should succeed");
    run_ingest(
        &cfg,
        &DevqlIngestArgs {
            init: true,
            max_checkpoints: 10,
        },
    )
    .await
    .expect("sqlite ingest should succeed without clickhouse");

    let store = connect_relational_store(&cfg)
        .await
        .expect("connect sqlite store");
    let rows = store
        .query_rows("SELECT repo_id FROM repositories LIMIT 1")
        .await
        .expect("query repository row");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["repo_id"], cfg.repo.repo_id);
}

#[tokio::test]
async fn execute_query_json_reads_from_sqlite_relational_store() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("relational.sqlite");
    let cfg = sqlite_test_cfg(
        temp.path().to_path_buf(),
        db_path.to_string_lossy().to_string(),
    );

    let store = connect_relational_store(&cfg)
        .await
        .expect("connect sqlite store");
    init_relational_schema(store.as_ref())
        .await
        .expect("init schema");

    let insert_sql = format!(
        "INSERT INTO artefacts (artefact_id, repo_id, blob_sha, path, language, canonical_kind, start_line, end_line, content_hash) VALUES ('{}', '{}', 'blob1', 'src/main.rs', 'rust', 'file', 1, 12, 'blob1')",
        esc_pg("artifact-query"),
        esc_pg(&cfg.repo.repo_id),
    );
    store
        .execute(&insert_sql)
        .await
        .expect("insert artefact row");

    let result = execute_query_json(&cfg, r#"file("src/main.rs")->artefacts()->limit(5)"#)
        .await
        .expect("query should succeed");
    let rows = result.as_array().cloned().expect("array result");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["path"], "src/main.rs");
}
