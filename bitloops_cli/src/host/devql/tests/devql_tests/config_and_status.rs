use super::*;

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
        "stores": {
            "relational": {
                "provider": "postgres",
                "postgres_dsn": "postgres://user:pass@localhost:5432/bitloops"
            },
            "event": {
                "provider": "clickhouse",
                "clickhouse_url": "http://localhost:8123",
                "clickhouse_database": "default"
            }
        }
    });

    let cfg = StoreFileConfig::from_json_value(&value);
    assert_eq!(
        cfg.pg_dsn.as_deref(),
        Some("postgres://user:pass@localhost:5432/bitloops")
    );
    assert_eq!(cfg.clickhouse_url.as_deref(), Some("http://localhost:8123"));
    assert_eq!(cfg.clickhouse_database.as_deref(), Some("default"));
}

#[test]
fn devql_file_config_parses_root_store_keys_without_wrapper() {
    let value = serde_json::json!({
        "relational": {
            "provider": "postgres",
            "postgres_dsn": "postgres://x/y"
        },
        "event": {
            "provider": "clickhouse",
            "clickhouse_url": "http://ch:8123",
            "clickhouse_database": "analytics"
        }
    });

    let cfg = StoreFileConfig::from_json_value(&value);
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
