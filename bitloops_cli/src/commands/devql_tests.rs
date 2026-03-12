use super::*;
use crate::devql_config::DevqlFileConfig;
use crate::engine::semantic_features::{
    PreStageArtefactRow, SemanticFeatureIndexState, SemanticFeatureInput,
    build_semantic_feature_inputs_from_artefacts, build_semantic_feature_rows,
    semantic_features_require_reindex,
};
use clap::Parser;
use std::fs;
use std::path::Path;
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
        semantic_provider: None,
        semantic_model: None,
        semantic_api_key: None,
        semantic_base_url: None,
    }
}

fn sqlite_test_cfg(repo_root: PathBuf, sqlite_path: String) -> DevqlConfig {
    let duckdb_path = repo_root.join("events.duckdb");
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
                duckdb_path: Some(duckdb_path.to_string_lossy().to_string()),
                clickhouse_url: None,
                clickhouse_user: None,
                clickhouse_password: None,
                clickhouse_database: None,
            },
        },
        semantic_provider: None,
        semantic_model: None,
        semantic_api_key: None,
        semantic_base_url: None,
    }
}

fn init_temp_git_repo() -> TempDir {
    let temp = TempDir::new().expect("temp git dir");
    run_git(temp.path(), &["init"]).expect("init git repo");
    run_git(temp.path(), &["config", "user.email", "test@example.com"])
        .expect("set test git user email");
    run_git(temp.path(), &["config", "user.name", "Test User"]).expect("set test git user name");
    temp
}

fn write_repo_file(repo_root: &Path, relative_path: &str, content: &str) {
    let abs_path = repo_root.join(relative_path);
    if let Some(parent) = abs_path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(&abs_path, content).expect("write repo file");
}

fn commit_all(repo_root: &Path, subject: &str, body: Option<&str>) -> String {
    run_git(repo_root, &["add", "."]).expect("git add");
    let mut args = vec!["-c", "commit.gpgsign=false", "commit", "-m", subject];
    if let Some(body) = body {
        args.push("-m");
        args.push(body);
    }
    run_git(repo_root, &args).expect("git commit");
    run_git(repo_root, &["rev-parse", "HEAD"]).expect("read head sha")
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
fn semantic_summary_provider_resolves_default_openai_endpoint() {
    let endpoint =
        semantic::resolve_semantic_summary_endpoint("openai", None).expect("openai endpoint");
    assert_eq!(endpoint, "https://api.openai.com/v1/chat/completions");
}

#[test]
fn semantic_summary_provider_requires_model_when_enabled() {
    let mut cfg = test_cfg();
    cfg.semantic_provider = Some("openai".to_string());
    cfg.semantic_api_key = Some("test-key".to_string());

    let err = match semantic::build_semantic_summary_provider(&semantic_provider_config(&cfg)) {
        Ok(_) => panic!("provider should require model"),
        Err(err) => err,
    };
    assert!(
        err.to_string()
            .contains("BITLOOPS_DEVQL_SEMANTIC_MODEL is required")
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
fn parse_devql_query_rejects_empty_pipeline() {
    let err = parse_devql_query("   ").unwrap_err();
    assert!(err.to_string().contains("empty DevQL query"));
}

#[test]
fn parse_devql_query_rejects_unsupported_stage() {
    let err = parse_devql_query(r#"repo("x")->unknownStage()"#).unwrap_err();
    assert!(err.to_string().contains("unsupported DevQL stage"));
}

#[test]
fn parse_devql_query_rejects_invalid_limit() {
    let err = parse_devql_query(r#"repo("x")->artefacts()->limit(nope)"#).unwrap_err();
    assert!(err.to_string().contains("invalid limit value"));
}

#[test]
fn parse_devql_query_rejects_invalid_lines_range() {
    let err = parse_devql_query(r#"repo("x")->artefacts(lines:10..2)"#).unwrap_err();
    assert!(err.to_string().contains("invalid lines range"));
}

#[test]
fn parse_named_args_supports_quoted_commas() {
    let args = parse_named_args(r#"path:"src/a,b.ts",kind:"function",agent:"claude""#)
        .expect("parse args with commas");
    assert_eq!(args.get("path").map(String::as_str), Some("src/a,b.ts"));
    assert_eq!(args.get("kind").map(String::as_str), Some("function"));
    assert_eq!(args.get("agent").map(String::as_str), Some("claude"));
}

#[test]
fn parse_single_quoted_or_double_rejects_unquoted_values() {
    let err = parse_single_quoted_or_double("unquoted").unwrap_err();
    assert!(err.to_string().contains("expected quoted string"));
}

#[test]
fn parse_lines_range_rejects_non_positive_ranges() {
    let err = parse_lines_range("0..5").unwrap_err();
    assert!(err.to_string().contains("invalid lines range"));
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
fn extract_message_helpers_handle_fallback_shapes() {
    let role = extract_message_role(&serde_json::json!({
        "message": {"role": "assistant"},
        "type": "ignored"
    }));
    assert_eq!(role.as_deref(), Some("assistant"));

    let text = extract_message_text(&serde_json::json!({
        "content": [
            {"text": "hello"},
            {"content": ["world"]},
            {"input": "!"}
        ]
    }));
    assert_eq!(text.as_deref(), Some("hello\nworld\n!"));
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
fn parse_remote_owner_name_supports_multiple_remote_formats() {
    assert_eq!(
        parse_remote_owner_name("git@github.com:acme/api.git"),
        Some(("acme".to_string(), "api".to_string()))
    );
    assert_eq!(
        parse_remote_owner_name("https://gitlab.com/group/subgroup/repo.git"),
        Some(("subgroup".to_string(), "repo".to_string()))
    );
    assert_eq!(
        parse_remote_owner_name("ssh://git@server.local/myorg/service"),
        Some(("myorg".to_string(), "service".to_string()))
    );
}

#[test]
fn parse_owner_name_path_rejects_incomplete_paths() {
    assert_eq!(parse_owner_name_path("single"), None);
    assert_eq!(parse_owner_name_path("/"), None);
}

#[test]
fn resolve_repo_identity_falls_back_to_local_when_no_remote() {
    let temp = TempDir::new().expect("temp dir");
    let identity = resolve_repo_identity(temp.path()).expect("resolve local repo identity");

    assert_eq!(identity.provider, "local");
    assert_eq!(identity.organization, "local");
    assert_eq!(
        identity.name,
        temp.path().file_name().unwrap().to_string_lossy()
    );
    assert_eq!(
        identity.repo_id,
        deterministic_uuid(&format!(
            "local://local/{}",
            temp.path().file_name().unwrap().to_string_lossy()
        ))
    );
}

#[test]
fn resolve_repo_identity_uses_remote_origin_information() {
    let temp = init_temp_git_repo();
    run_git(
        temp.path(),
        &["remote", "add", "origin", "git@github.com:acme/widgets.git"],
    )
    .expect("add remote origin");

    let identity = resolve_repo_identity(temp.path()).expect("resolve remote identity");
    assert_eq!(identity.provider, "github");
    assert_eq!(identity.organization, "acme");
    assert_eq!(identity.name, "widgets");
    assert_eq!(identity.identity, "github://acme/widgets");
}

#[test]
fn default_branch_name_falls_back_to_main_when_git_fails() {
    let temp = TempDir::new().expect("temp dir");
    assert_eq!(default_branch_name(temp.path()), "main");
}

#[test]
fn default_branch_name_returns_active_branch() {
    let temp = init_temp_git_repo();
    write_repo_file(temp.path(), "README.md", "hello");
    commit_all(temp.path(), "init", None);
    run_git(temp.path(), &["checkout", "-b", "feature/devql"]).expect("create branch");
    assert_eq!(default_branch_name(temp.path()), "feature/devql");
}

#[test]
fn collect_checkpoint_commit_map_prefers_newest_commit_for_duplicate_checkpoint() {
    let temp = init_temp_git_repo();
    write_repo_file(temp.path(), "src/main.rs", "fn main() {}\n");
    commit_all(
        temp.path(),
        "first checkpoint",
        Some("Bitloops-Checkpoint: aabbccddeeff"),
    );

    write_repo_file(
        temp.path(),
        "src/main.rs",
        "fn main() { println!(\"x\"); }\n",
    );
    let latest_sha = commit_all(
        temp.path(),
        "second checkpoint",
        Some("Bitloops-Checkpoint: aabbccddeeff\nBitloops-Checkpoint: invalid"),
    );

    let map = collect_checkpoint_commit_map(temp.path()).expect("collect checkpoint map");
    let info = map.get("aabbccddeeff").expect("checkpoint info exists");
    assert_eq!(info.commit_sha, latest_sha);
    assert_eq!(info.subject, "second checkpoint");
}

#[test]
fn git_blob_helpers_return_expected_values() {
    let temp = init_temp_git_repo();
    write_repo_file(temp.path(), "src/newline.txt", "one\ntwo\n");
    write_repo_file(temp.path(), "src/no-newline.txt", "one\ntwo");
    let commit_sha = commit_all(temp.path(), "blob helpers", None);

    let with_newline_sha = run_git(temp.path(), &["rev-parse", "HEAD:src/newline.txt"])
        .expect("blob sha with newline");
    let without_newline_sha = run_git(temp.path(), &["rev-parse", "HEAD:src/no-newline.txt"])
        .expect("blob sha without newline");

    assert_eq!(
        git_blob_sha_at_commit(temp.path(), &commit_sha, "src/newline.txt").as_deref(),
        Some(with_newline_sha.as_str())
    );
    assert_eq!(
        git_blob_line_count(temp.path(), &with_newline_sha),
        Some(3),
        "run_git() trims stdout for this path, so newline-terminated blobs currently get +1"
    );
    assert_eq!(
        git_blob_line_count(temp.path(), &without_newline_sha),
        Some(3),
        "non-newline-terminated file should get +1 according to current heuristic"
    );
    assert_eq!(
        git_blob_sha_at_commit(temp.path(), "deadbeef", "src/newline.txt"),
        None
    );
}

#[test]
fn detect_language_covers_supported_extensions_and_fallback() {
    assert_eq!(detect_language("main.ts"), "typescript");
    assert_eq!(detect_language("lib/index.TSX"), "typescript");
    assert_eq!(detect_language("mod.rs"), "rust");
    assert_eq!(detect_language("app.jsx"), "javascript");
    assert_eq!(detect_language("script.py"), "python");
    assert_eq!(detect_language("server.go"), "go");
    assert_eq!(detect_language("Main.java"), "java");
    assert_eq!(detect_language("README"), "text");
}

#[test]
fn find_block_end_line_handles_missing_and_unbalanced_braces() {
    let missing = vec!["export const x = 1;"];
    assert_eq!(find_block_end_line(&missing, 0), None);

    let unbalanced = vec!["function a() {", "  return 1;"];
    assert_eq!(find_block_end_line(&unbalanced, 0), Some(2));
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
fn events_store_resolver_supports_duckdb_provider() {
    let backends = crate::devql_config::DevqlBackendConfig {
        relational: crate::devql_config::RelationalBackendConfig {
            provider: crate::devql_config::RelationalProvider::Postgres,
            sqlite_path: None,
            postgres_dsn: Some("postgres://user:pass@localhost:5432/bitloops".to_string()),
        },
        events: crate::devql_config::EventsBackendConfig {
            provider: crate::devql_config::EventsProvider::DuckDb,
            duckdb_path: Some("/tmp/devql-events.duckdb".to_string()),
            clickhouse_url: None,
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: None,
        },
    };

    let store = resolve_events_store_from_backends(&backends).expect("duckdb events store");
    assert_eq!(
        store.provider(),
        crate::devql_config::EventsProvider::DuckDb
    );
}

#[test]
fn events_store_resolver_supports_clickhouse_provider() {
    let backends = crate::devql_config::DevqlBackendConfig {
        relational: crate::devql_config::RelationalBackendConfig {
            provider: crate::devql_config::RelationalProvider::Postgres,
            sqlite_path: None,
            postgres_dsn: Some("postgres://user:pass@localhost:5432/bitloops".to_string()),
        },
        events: crate::devql_config::EventsBackendConfig {
            provider: crate::devql_config::EventsProvider::ClickHouse,
            duckdb_path: None,
            clickhouse_url: Some("http://localhost:8123".to_string()),
            clickhouse_user: Some("default".to_string()),
            clickhouse_password: Some("secret".to_string()),
            clickhouse_database: Some("default".to_string()),
        },
    };

    let store = resolve_events_store_from_backends(&backends).expect("clickhouse events store");
    assert_eq!(
        store.provider(),
        crate::devql_config::EventsProvider::ClickHouse
    );
}

#[tokio::test]
async fn check_events_connection_status_duckdb_reports_connected() {
    let cfg = DevqlConnectionConfig {
        backends: crate::devql_config::DevqlBackendConfig {
            relational: crate::devql_config::RelationalBackendConfig {
                provider: crate::devql_config::RelationalProvider::Sqlite,
                sqlite_path: Some("/tmp/devql-relational.sqlite".to_string()),
                postgres_dsn: None,
            },
            events: crate::devql_config::EventsBackendConfig {
                provider: crate::devql_config::EventsProvider::DuckDb,
                duckdb_path: Some("/tmp/devql-events.duckdb".to_string()),
                clickhouse_url: None,
                clickhouse_user: None,
                clickhouse_password: None,
                clickhouse_database: None,
            },
        },
    };

    let status = check_events_connection_status(&cfg).await;
    assert_eq!(status, DatabaseConnectionStatus::Connected);
}

#[tokio::test]
async fn duckdb_events_store_roundtrip_supports_core_queries() {
    let temp = tempfile::tempdir().expect("temp dir");
    let duckdb_path = temp.path().join("events.duckdb");

    let backends = crate::devql_config::DevqlBackendConfig {
        relational: crate::devql_config::RelationalBackendConfig {
            provider: crate::devql_config::RelationalProvider::Postgres,
            sqlite_path: None,
            postgres_dsn: Some("postgres://user:pass@localhost:5432/bitloops".to_string()),
        },
        events: crate::devql_config::EventsBackendConfig {
            provider: crate::devql_config::EventsProvider::DuckDb,
            duckdb_path: Some(duckdb_path.to_string_lossy().to_string()),
            clickhouse_url: None,
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: None,
        },
    };

    let store = resolve_events_store_from_backends(&backends).expect("duckdb events store");
    store.init_schema().await.expect("init duckdb schema");

    let event = store_contracts::CheckpointEventWrite {
        event_id: "evt-1".to_string(),
        repo_id: "repo-1".to_string(),
        checkpoint_id: "cp-1".to_string(),
        session_id: "session-1".to_string(),
        commit_sha: "commit-sha-1".to_string(),
        commit_unix: Some(1_741_211_200),
        branch: "main".to_string(),
        event_type: "checkpoint_committed".to_string(),
        agent: "claude-code".to_string(),
        strategy: "manual-commit".to_string(),
        files_touched: vec!["src/main.rs".to_string()],
        created_at: Some("2026-03-01T12:00:00Z".to_string()),
        payload: serde_json::json!({"checkpoints_count": 1}),
    };
    store
        .insert_checkpoint_event(event)
        .await
        .expect("insert duckdb event");

    let existing = store
        .existing_event_ids("repo-1".to_string())
        .await
        .expect("existing ids");
    assert!(existing.contains("evt-1"));

    let checkpoints = store
        .query_checkpoints(store_contracts::EventsCheckpointQuery {
            repo_id: "repo-1".to_string(),
            agent: Some("claude-code".to_string()),
            since: None,
            limit: 10,
        })
        .await
        .expect("query checkpoints");
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(checkpoints[0]["checkpoint_id"], "cp-1");
    assert_eq!(checkpoints[0]["files_touched"][0], "src/main.rs");

    let telemetry = store
        .query_telemetry(store_contracts::EventsTelemetryQuery {
            repo_id: "repo-1".to_string(),
            event_type: Some("checkpoint_committed".to_string()),
            agent: Some("claude-code".to_string()),
            since: None,
            limit: 10,
        })
        .await
        .expect("query telemetry");
    assert_eq!(telemetry.len(), 1);
    assert_eq!(telemetry[0]["event_type"], "checkpoint_committed");
    assert_eq!(telemetry[0]["payload"], "{\"checkpoints_count\":1}");

    let commit_shas = store
        .query_commit_shas(store_contracts::EventsCommitShaQuery {
            repo_id: "repo-1".to_string(),
            agent: Some("claude-code".to_string()),
            since: None,
            limit: 10,
        })
        .await
        .expect("query commit shas");
    assert_eq!(commit_shas, vec!["commit-sha-1".to_string()]);

    let history = store
        .query_checkpoint_events(store_contracts::EventsCheckpointHistoryQuery {
            repo_id: "repo-1".to_string(),
            commit_shas: vec!["commit-sha-1".to_string()],
            path_candidates: vec!["src/main.rs".to_string()],
            limit: 10,
        })
        .await
        .expect("query checkpoint events");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["checkpoint_id"], "cp-1");
}

#[tokio::test]
async fn duckdb_checkpoint_history_path_filter_treats_wildcards_as_literals() {
    let temp = tempfile::tempdir().expect("temp dir");
    let duckdb_path = temp.path().join("events.duckdb");

    let backends = crate::devql_config::DevqlBackendConfig {
        relational: crate::devql_config::RelationalBackendConfig {
            provider: crate::devql_config::RelationalProvider::Postgres,
            sqlite_path: None,
            postgres_dsn: Some("postgres://user:pass@localhost:5432/bitloops".to_string()),
        },
        events: crate::devql_config::EventsBackendConfig {
            provider: crate::devql_config::EventsProvider::DuckDb,
            duckdb_path: Some(duckdb_path.to_string_lossy().to_string()),
            clickhouse_url: None,
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: None,
        },
    };

    let store = resolve_events_store_from_backends(&backends).expect("duckdb events store");
    store.init_schema().await.expect("init duckdb schema");

    let expected_path = "src/100%_complete.rs";
    let lookalike_path = "src/100abcXcomplete.rs";

    let matching_event = store_contracts::CheckpointEventWrite {
        event_id: "evt-match".to_string(),
        repo_id: "repo-1".to_string(),
        checkpoint_id: "cp-match".to_string(),
        session_id: "session-1".to_string(),
        commit_sha: "commit-sha-1".to_string(),
        commit_unix: Some(1_741_211_200),
        branch: "main".to_string(),
        event_type: "checkpoint_committed".to_string(),
        agent: "claude-code".to_string(),
        strategy: "manual-commit".to_string(),
        files_touched: vec![expected_path.to_string()],
        created_at: Some("2026-03-01T12:00:00Z".to_string()),
        payload: serde_json::json!({"id": "match"}),
    };
    store
        .insert_checkpoint_event(matching_event)
        .await
        .expect("insert matching event");

    let non_matching_event = store_contracts::CheckpointEventWrite {
        event_id: "evt-lookalike".to_string(),
        repo_id: "repo-1".to_string(),
        checkpoint_id: "cp-lookalike".to_string(),
        session_id: "session-2".to_string(),
        commit_sha: "commit-sha-1".to_string(),
        commit_unix: Some(1_741_211_201),
        branch: "main".to_string(),
        event_type: "checkpoint_committed".to_string(),
        agent: "claude-code".to_string(),
        strategy: "manual-commit".to_string(),
        files_touched: vec![lookalike_path.to_string()],
        created_at: Some("2026-03-01T12:00:01Z".to_string()),
        payload: serde_json::json!({"id": "lookalike"}),
    };
    store
        .insert_checkpoint_event(non_matching_event)
        .await
        .expect("insert lookalike event");

    let history = store
        .query_checkpoint_events(store_contracts::EventsCheckpointHistoryQuery {
            repo_id: "repo-1".to_string(),
            commit_shas: vec!["commit-sha-1".to_string()],
            path_candidates: vec![expected_path.to_string()],
            limit: 10,
        })
        .await
        .expect("query checkpoint events");

    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["checkpoint_id"], "cp-match");
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
fn sql_path_candidates_clause_handles_empty_and_escaping() {
    assert_eq!(sql_path_candidates_clause("a.path", &[]), "1=0");

    let clause = sql_path_candidates_clause("a.path", &["src/a'b.rs".to_string()]);
    assert_eq!(clause, "a.path = 'src/a''b.rs'");
}

#[test]
fn glob_to_sql_like_converts_wildcards() {
    assert_eq!(glob_to_sql_like("src/*.rs"), "src/%.rs");
    assert_eq!(glob_to_sql_like("**/main.*"), "%/main.%");
}

#[test]
fn format_ch_array_escapes_values() {
    let encoded = format_ch_array(&["a'b".to_string(), "line\nbreak".to_string()]);
    assert_eq!(encoded, "['a\\'b','line\\nbreak']");
}

#[test]
fn parse_json_string_array_rejects_non_array_payloads() {
    assert_eq!(
        parse_json_string_array("{\"k\":1}".to_string()),
        Value::Array(vec![])
    );
    assert_eq!(
        parse_json_string_array("".to_string()),
        Value::Array(vec![])
    );
}

#[test]
fn bytes_to_hex_encodes_binary_data() {
    assert_eq!(bytes_to_hex(&[0x00, 0x0f, 0x10, 0xff]), "000f10ff");
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
fn resolve_commit_selector_supports_literal_commit_and_ref() {
    let temp = init_temp_git_repo();
    write_repo_file(temp.path(), "src/lib.rs", "pub fn x() {}\n");
    let head_sha = commit_all(temp.path(), "commit selector", None);
    let cfg = sqlite_test_cfg(
        temp.path().to_path_buf(),
        temp.path()
            .join("devql.sqlite")
            .to_string_lossy()
            .to_string(),
    );

    let parsed_commit =
        parse_devql_query(r#"asOf(commit:"abc123")->file("src/lib.rs")->artefacts()"#)
            .expect("parse commit selector query");
    assert_eq!(
        resolve_commit_selector(&cfg, &parsed_commit).expect("literal commit selector"),
        Some("abc123".to_string())
    );

    let parsed_ref = parse_devql_query(r#"asOf(ref:"HEAD")->file("src/lib.rs")->artefacts()"#)
        .expect("parse ref selector query");
    assert_eq!(
        resolve_commit_selector(&cfg, &parsed_ref).expect("resolve HEAD"),
        Some(head_sha)
    );
}

#[test]
fn project_rows_supports_count_and_nested_field_projection() {
    let rows = vec![
        serde_json::json!({"path":"src/a.rs","meta":{"lang":"rust"}}),
        serde_json::json!({"path":"src/b.ts","meta":{"lang":"typescript"}}),
    ];
    let count_only = project_rows(rows.clone(), &["count()".to_string()]);
    assert_eq!(count_only, vec![serde_json::json!({"count": 2})]);

    let projected = project_rows(
        rows,
        &[
            "path".to_string(),
            "meta.lang".to_string(),
            "meta.kind".to_string(),
        ],
    );
    assert_eq!(projected[0]["path"], "src/a.rs");
    assert_eq!(projected[0]["meta.lang"], "rust");
    assert_eq!(projected[0]["meta.kind"], Value::Null);
}

#[test]
fn sql_string_list_helpers_escape_values() {
    let values = vec!["a'b".to_string(), "x\\y".to_string()];
    assert_eq!(sql_string_list_pg(&values), "'a''b','x\\y'");
    assert_eq!(sql_string_list_ch(&values), "'a\\'b','x\\\\y'");
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

#[derive(Clone)]
struct MockSemanticSummaryProvider {
    candidate: Option<semantic::SemanticSummaryCandidate>,
}

impl semantic::SemanticSummaryProvider for MockSemanticSummaryProvider {
    fn generate(
        &self,
        _input: &SemanticFeatureInput,
    ) -> Option<semantic::SemanticSummaryCandidate> {
        self.candidate.clone()
    }

    fn prompt_version(&self) -> String {
        "semantic-summary-v5::provider=mock::model=test".to_string()
    }
}

fn mock_semantic_feature_blob_content() -> &'static str {
    r#"/* Service helpers for user operations. */
import { db } from './db';

export interface User {
  id: string;
  email: string;
}

export class UserService {
  // Fetch a user record by its id.
  async getById(id: string): Promise<User | null> {
    return db.users.findById(id);
  }
}

export function normalizeEmail(email: string): string {
  return email.trim().toLowerCase();
}

export const DEFAULT_PAGE_SIZE = 20;
export type UserId = string;
"#
}

fn mock_prestage_artefacts() -> Vec<PreStageArtefactRow> {
    serde_json::from_value(serde_json::json!([
        {
            "artefact_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
            "symbol_id": "1a2b3c4d-5e6f-7890-abcd-ef1234567890",
            "repo_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "blob_sha": "a3f1e2b7c4d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3",
            "path": "src/services/user.ts",
            "language": "typescript",
            "canonical_kind": "file",
            "language_kind": "module",
            "symbol_fqn": "src/services/user.ts",
            "parent_artefact_id": null,
            "start_line": 1,
            "end_line": 21,
            "start_byte": null,
            "end_byte": null,
            "signature": "src/services/user.ts",
            "content_hash": "c9d2e3f4-1a2b-3c4d-5e6f-7a8b9c0d1e2f"
        },
        {
            "artefact_id": "b2c3d4e5-f6a7-8901-bcde-f12345678901",
            "symbol_id": "2b3c4d5e-6f7a-8901-bcde-f12345678901",
            "repo_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "blob_sha": "a3f1e2b7c4d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3",
            "path": "src/services/user.ts",
            "language": "typescript",
            "canonical_kind": "import",
            "language_kind": "import",
            "symbol_fqn": "src/services/user.ts::import::./db",
            "parent_artefact_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
            "start_line": 2,
            "end_line": 2,
            "start_byte": null,
            "end_byte": null,
            "signature": "import { db } from './db';",
            "content_hash": null
        },
        {
            "artefact_id": "c3d4e5f6-a7b8-9012-cdef-123456789012",
            "symbol_id": "3c4d5e6f-7a8b-9012-cdef-123456789012",
            "repo_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "blob_sha": "a3f1e2b7c4d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3",
            "path": "src/services/user.ts",
            "language": "typescript",
            "canonical_kind": "interface",
            "language_kind": "interface",
            "symbol_fqn": "src/services/user.ts::User",
            "parent_artefact_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
            "start_line": 4,
            "end_line": 7,
            "start_byte": null,
            "end_byte": null,
            "signature": "export interface User {",
            "content_hash": "d4e5f6a7-2b3c-4d5e-6f7a-8b9c0d1e2f3a"
        },
        {
            "artefact_id": "d4e5f6a7-b8c9-0123-defa-234567890123",
            "symbol_id": "4d5e6f7a-8b9c-0123-defa-234567890123",
            "repo_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "blob_sha": "a3f1e2b7c4d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3",
            "path": "src/services/user.ts",
            "language": "typescript",
            "canonical_kind": "class",
            "language_kind": "class",
            "symbol_fqn": "src/services/user.ts::UserService",
            "parent_artefact_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
            "start_line": 9,
            "end_line": 14,
            "start_byte": null,
            "end_byte": null,
            "signature": "export class UserService {",
            "content_hash": "e5f6a7b8-3c4d-5e6f-7a8b-9c0d1e2f3a4b"
        },
        {
            "artefact_id": "e5f6a7b8-c9d0-1234-efab-345678901234",
            "symbol_id": "5e6f7a8b-9c0d-1234-efab-345678901234",
            "repo_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "blob_sha": "a3f1e2b7c4d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3",
            "path": "src/services/user.ts",
            "language": "typescript",
            "canonical_kind": "method",
            "language_kind": "method",
            "symbol_fqn": "src/services/user.ts::UserService::getById",
            "parent_artefact_id": "d4e5f6a7-b8c9-0123-defa-234567890123",
            "start_line": 11,
            "end_line": 13,
            "start_byte": null,
            "end_byte": null,
            "signature": "async getById(id: string): Promise<User | null> {",
            "doc_comment": "Fetch a user record by its id.",
            "content_hash": "f6a7b8c9-4d5e-6f7a-8b9c-0d1e2f3a4b5c"
        },
        {
            "artefact_id": "f6a7b8c9-d0e1-2345-fabc-456789012345",
            "symbol_id": "6f7a8b9c-0d1e-2345-fabc-456789012345",
            "repo_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "blob_sha": "a3f1e2b7c4d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3",
            "path": "src/services/user.ts",
            "language": "typescript",
            "canonical_kind": "function",
            "language_kind": "function",
            "symbol_fqn": "src/services/user.ts::normalizeEmail",
            "parent_artefact_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
            "start_line": 16,
            "end_line": 18,
            "start_byte": null,
            "end_byte": null,
            "signature": "export function normalizeEmail(email: string): string {",
            "content_hash": "a7b8c9d0-5e6f-7a8b-9c0d-1e2f3a4b5c6d"
        },
        {
            "artefact_id": "a8b9c0d1-e2f3-4567-abcd-567890123456",
            "symbol_id": "7a8b9c0d-1e2f-4567-abcd-567890123456",
            "repo_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "blob_sha": "a3f1e2b7c4d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3",
            "path": "src/services/user.ts",
            "language": "typescript",
            "canonical_kind": "variable",
            "language_kind": "const",
            "symbol_fqn": "src/services/user.ts::DEFAULT_PAGE_SIZE",
            "parent_artefact_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
            "start_line": 20,
            "end_line": 20,
            "start_byte": null,
            "end_byte": null,
            "signature": "export const DEFAULT_PAGE_SIZE = 20;",
            "content_hash": null
        },
        {
            "artefact_id": "b9c0d1e2-f3a4-5678-bcde-678901234567",
            "symbol_id": "8b9c0d1e-2f3a-5678-bcde-678901234567",
            "repo_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "blob_sha": "a3f1e2b7c4d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3",
            "path": "src/services/user.ts",
            "language": "typescript",
            "canonical_kind": "type",
            "language_kind": "type_alias",
            "symbol_fqn": "src/services/user.ts::UserId",
            "parent_artefact_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
            "start_line": 21,
            "end_line": 21,
            "start_byte": null,
            "end_byte": null,
            "signature": "export type UserId = string;",
            "content_hash": null
        }
    ]))
    .expect("mock artefacts should parse")
}

fn semantic_feature_input_named(name: &str) -> SemanticFeatureInput {
    build_semantic_feature_inputs_from_artefacts(
        &mock_prestage_artefacts(),
        mock_semantic_feature_blob_content(),
    )
    .into_iter()
    .find(|input| input.name == name)
    .expect("semantic feature input should exist")
}

#[test]
fn semantic_features_build_inputs_from_mock_prestage_rows() {
    let inputs = build_semantic_feature_inputs_from_artefacts(
        &mock_prestage_artefacts(),
        mock_semantic_feature_blob_content(),
    );

    assert_eq!(
        inputs.len(),
        7,
        "import rows should not be enriched in stage 1"
    );
    assert!(inputs.iter().all(|input| input.canonical_kind != "import"));

    let file = inputs
        .iter()
        .find(|input| input.canonical_kind == "file")
        .expect("file input");
    assert!(
        file.local_relationships
            .contains(&"contains:class".to_string()),
        "{:?}",
        file.local_relationships
    );

    let method = inputs
        .iter()
        .find(|input| input.name == "getById")
        .expect("method input");
    assert_eq!(method.parent_kind.as_deref(), Some("class"));
    assert_eq!(
        method.parent_symbol.as_deref(),
        Some("src/services/user.ts::UserService")
    );
    assert!(
        method.body.contains("findById"),
        "expected method body to be sliced from blob content"
    );
}

#[test]
fn semantic_features_synthesize_template_and_doc_comment_when_llm_missing() {
    let input = semantic_feature_input_named("getById");
    let output = build_semantic_feature_rows(&input, &semantic::NoopSemanticSummaryProvider);

    assert_eq!(
        output.semantics.summary,
        "Method get by id. Fetch a user record by its id."
    );
    assert_eq!(
        output.semantics.doc_comment_summary.as_deref(),
        Some("Fetch a user record by its id.")
    );
    assert_eq!(output.semantics.llm_summary, None);
    assert_eq!(output.semantics.template_summary, "Method get by id.");
    assert_eq!(output.features.normalized_name, "get_by_id");
    assert!(
        output
            .features
            .normalized_body_tokens
            .contains(&"find".to_string())
    );
}

#[test]
fn semantic_features_synthesize_template_and_llm_when_doc_comment_missing() {
    let input = semantic_feature_input_named("normalizeEmail");
    let output = build_semantic_feature_rows(
        &input,
        &MockSemanticSummaryProvider {
            candidate: Some(semantic::SemanticSummaryCandidate {
                summary: "Normalizes email addresses before storage".to_string(),
                confidence: 0.87,
                source_model: Some("mock-llm".to_string()),
            }),
        },
    );

    assert_eq!(
        output.semantics.summary,
        "Function normalize email. Normalizes email addresses before storage."
    );
    assert_eq!(output.semantics.doc_comment_summary, None);
    assert_eq!(
        output.semantics.llm_summary.as_deref(),
        Some("Normalizes email addresses before storage")
    );
    assert_eq!(
        output.semantics.template_summary,
        "Function normalize email."
    );
    assert_eq!(output.semantics.source_model.as_deref(), Some("mock-llm"));
}

#[test]
fn semantic_features_fall_back_to_template_when_no_detail_summary_is_usable() {
    let input = semantic_feature_input_named("normalizeEmail");
    let output = build_semantic_feature_rows(
        &input,
        &MockSemanticSummaryProvider {
            candidate: Some(semantic::SemanticSummaryCandidate {
                summary: "bad".to_string(),
                confidence: 0.8,
                source_model: Some("mock-llm".to_string()),
            }),
        },
    );

    assert_eq!(output.semantics.summary, "Function normalize email.");
    assert_eq!(output.semantics.doc_comment_summary, None);
    assert_eq!(output.semantics.llm_summary.as_deref(), Some("bad"));
    assert_eq!(
        output.semantics.template_summary,
        "Function normalize email."
    );
}

#[test]
fn semantic_features_synthesize_template_and_llm_while_persisting_all_candidates() {
    let input = semantic_feature_input_named("getById");
    let output = build_semantic_feature_rows(
        &input,
        &MockSemanticSummaryProvider {
            candidate: Some(semantic::SemanticSummaryCandidate {
                summary: "Loads a user entity by id from storage".to_string(),
                confidence: 0.82,
                source_model: Some("mock-llm".to_string()),
            }),
        },
    );

    assert_eq!(
        output.semantics.doc_comment_summary.as_deref(),
        Some("Fetch a user record by its id.")
    );
    assert_eq!(
        output.semantics.llm_summary.as_deref(),
        Some("Loads a user entity by id from storage")
    );
    assert_eq!(output.semantics.template_summary, "Method get by id.");
    assert_eq!(
        output.semantics.summary,
        "Method get by id. Loads a user entity by id from storage."
    );
    assert_eq!(output.semantics.source_model.as_deref(), Some("mock-llm"));
}

#[test]
fn semantic_features_use_doc_comment_detail_when_llm_summary_is_invalid() {
    let input = semantic_feature_input_named("getById");
    let output = build_semantic_feature_rows(
        &input,
        &MockSemanticSummaryProvider {
            candidate: Some(semantic::SemanticSummaryCandidate {
                summary: "bad".to_string(),
                confidence: 0.8,
                source_model: Some("mock-llm".to_string()),
            }),
        },
    );

    assert_eq!(
        output.semantics.summary,
        "Method get by id. Fetch a user record by its id."
    );
    assert_eq!(
        output.semantics.doc_comment_summary.as_deref(),
        Some("Fetch a user record by its id.")
    );
    assert_eq!(output.semantics.llm_summary.as_deref(), Some("bad"));
    assert_eq!(output.semantics.template_summary, "Method get by id.");
}

#[test]
fn semantic_features_reindex_when_hash_or_prompt_version_changes() {
    let input = semantic_feature_input_named("normalizeEmail");
    let output = build_semantic_feature_rows(&input, &semantic::NoopSemanticSummaryProvider);
    let hash = output.semantic_features_input_hash.clone();

    let unchanged = SemanticFeatureIndexState {
        semantics_hash: Some(hash.clone()),
        semantics_prompt_version: Some(output.semantics.prompt_version.clone()),
        features_hash: Some(hash.clone()),
        features_prompt_version: Some(output.features.prompt_version.clone()),
    };
    assert!(!semantic_features_require_reindex(
        &unchanged,
        &hash,
        &output.semantics.prompt_version,
        &output.features.prompt_version,
    ));

    let stale_prompt = SemanticFeatureIndexState {
        semantics_prompt_version: Some("semantic-summary-v0::provider=noop".to_string()),
        ..unchanged.clone()
    };
    assert!(semantic_features_require_reindex(
        &stale_prompt,
        &hash,
        &output.semantics.prompt_version,
        &output.features.prompt_version,
    ));

    let stale_hash = SemanticFeatureIndexState {
        features_hash: Some("different-hash".to_string()),
        ..unchanged
    };
    assert!(semantic_features_require_reindex(
        &stale_hash,
        &hash,
        &output.semantics.prompt_version,
        &output.features.prompt_version,
    ));
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

#[tokio::test]
async fn execute_events_pipeline_returns_empty_rows_for_fresh_events_store() {
    let temp = init_temp_git_repo();
    let cfg = DevqlConfig {
        repo_root: temp.path().to_path_buf(),
        repo: resolve_repo_identity(temp.path()).expect("repo identity"),
        backends: crate::devql_config::DevqlBackendConfig {
            relational: crate::devql_config::RelationalBackendConfig {
                provider: crate::devql_config::RelationalProvider::Sqlite,
                sqlite_path: Some(
                    temp.path()
                        .join("relational.sqlite")
                        .to_string_lossy()
                        .to_string(),
                ),
                postgres_dsn: None,
            },
            events: crate::devql_config::EventsBackendConfig {
                provider: crate::devql_config::EventsProvider::DuckDb,
                duckdb_path: Some(
                    temp.path()
                        .join("events.duckdb")
                        .to_string_lossy()
                        .to_string(),
                ),
                clickhouse_url: None,
                clickhouse_user: None,
                clickhouse_password: None,
                clickhouse_database: None,
            },
        },
        semantic_provider: None,
        semantic_model: None,
        semantic_api_key: None,
        semantic_base_url: None,
    };

    init_events_schema(&cfg).await.expect("init events schema");

    let telemetry_query =
        parse_devql_query(r#"telemetry(event_type:"x")->limit(5)"#).expect("parse telemetry query");
    let telemetry_rows = execute_events_pipeline(&cfg, &telemetry_query)
        .await
        .expect("execute telemetry pipeline");
    assert!(telemetry_rows.is_empty());

    let checkpoints_query =
        parse_devql_query(r#"checkpoints()->limit(5)"#).expect("parse checkpoints query");
    let checkpoints_rows = execute_events_pipeline(&cfg, &checkpoints_query)
        .await
        .expect("execute checkpoints pipeline");
    assert!(checkpoints_rows.is_empty());
}

#[tokio::test]
async fn execute_relational_pipeline_supports_files_glob_filter() {
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
    store
        .execute(&format!(
            "INSERT INTO artefacts (artefact_id, repo_id, blob_sha, path, language, canonical_kind, start_line, end_line, content_hash) VALUES ('{}', '{}', 'blobA', 'src/main.rs', 'rust', 'file', 1, 10, 'hashA')",
            esc_pg("glob-1"),
            esc_pg(&cfg.repo.repo_id),
        ))
        .await
        .expect("insert artefact");

    let parsed = parse_devql_query(r#"files(path:"src/*")->artefacts(kind:"file")->limit(10)"#)
        .expect("parse files glob query");
    let rows = execute_relational_pipeline(&cfg, &parsed, store.as_ref())
        .await
        .expect("execute relational pipeline");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["path"], "src/main.rs");
}

#[tokio::test]
async fn attach_chat_history_to_artefacts_handles_non_object_rows_and_missing_keys() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("relational.sqlite");
    let cfg = sqlite_test_cfg(
        temp.path().to_path_buf(),
        db_path.to_string_lossy().to_string(),
    );
    let store = connect_relational_store(&cfg)
        .await
        .expect("connect sqlite store");

    let input = vec![
        Value::String("raw".to_string()),
        serde_json::json!({"path":"","blob_sha":"","canonical_kind":"file"}),
    ];
    let output = attach_chat_history_to_artefacts(&cfg, store.as_ref(), &cfg.repo.repo_id, input)
        .await
        .expect("attach chat history");

    assert_eq!(output.len(), 2);
    assert_eq!(output[0], Value::String("raw".to_string()));
    assert_eq!(output[1]["chat_history"], Value::Array(vec![]));
}

#[tokio::test]
async fn commit_shas_for_artefact_blob_filters_blank_values() {
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

    store
        .execute(&format!(
            "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha) VALUES ('{}','sha1','src/main.rs','blob1'), ('{}',' ','./src/main.rs','blob1'), ('{}','sha2','src/main.rs','blob1')",
            esc_pg(&cfg.repo.repo_id),
            esc_pg(&cfg.repo.repo_id),
            esc_pg(&cfg.repo.repo_id),
        ))
        .await
        .expect("insert file_state rows");

    let commit_shas =
        commit_shas_for_artefact_blob(store.as_ref(), &cfg.repo.repo_id, "src/main.rs", "blob1")
            .await
            .expect("query commit shas");
    assert_eq!(commit_shas, vec!["sha1".to_string(), "sha2".to_string()]);
}

#[tokio::test]
async fn checkpoint_events_for_commits_short_circuits_on_empty_input() {
    let cfg = test_cfg();
    let rows = checkpoint_events_for_commits(&cfg, "repo-x", "src/main.rs", &[])
        .await
        .expect("empty commit list should short-circuit");
    assert!(rows.is_empty());
}

#[tokio::test]
async fn upsert_language_artefacts_inserts_function_rows_for_typescript() {
    let repo = init_temp_git_repo();
    write_repo_file(
        repo.path(),
        "src/main.ts",
        "export function one() {\n  return 1;\n}\n",
    );
    commit_all(repo.path(), "ts file", None);
    let blob_sha =
        run_git(repo.path(), &["rev-parse", "HEAD:src/main.ts"]).expect("resolve blob sha");

    let db_path = repo.path().join("relational.sqlite");
    let cfg = sqlite_test_cfg(
        repo.path().to_path_buf(),
        db_path.to_string_lossy().to_string(),
    );
    let store = connect_relational_store(&cfg)
        .await
        .expect("connect sqlite store");
    init_relational_schema(store.as_ref())
        .await
        .expect("init schema");

    let file_artefact = upsert_file_artefact_row(&cfg, store.as_ref(), "src/main.ts", &blob_sha)
        .await
        .expect("upsert file artefact");
    upsert_language_artefacts(
        &cfg,
        store.as_ref(),
        "src/main.ts",
        &blob_sha,
        &file_artefact,
    )
    .await
    .expect("upsert language artefacts");

    let rows = store
        .query_rows(&format!(
            "SELECT canonical_kind, symbol_fqn, parent_artefact_id FROM artefacts WHERE repo_id = '{}' AND canonical_kind = 'function'",
            esc_pg(&cfg.repo.repo_id),
        ))
        .await
        .expect("query function artefacts");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["canonical_kind"], "function");
    assert_eq!(rows[0]["symbol_fqn"], "src/main.ts::one");
    assert_eq!(rows[0]["parent_artefact_id"], file_artefact.artefact_id);
}

#[tokio::test]
async fn upsert_language_artefacts_skips_non_javascript_languages() {
    let temp = TempDir::new().expect("temp dir");
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

    let file_artefact = FileArtefactRow {
        artefact_id: "file-artefact-id".to_string(),
        language: "rust".to_string(),
    };
    upsert_language_artefacts(
        &cfg,
        store.as_ref(),
        "src/lib.rs",
        "blob-rust",
        &file_artefact,
    )
    .await
    .expect("non-js upsert should no-op");

    let rows = store
        .query_rows("SELECT canonical_kind FROM artefacts WHERE canonical_kind = 'function'")
        .await
        .expect("query function rows");
    assert!(rows.is_empty());
}
