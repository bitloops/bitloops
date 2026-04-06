use super::*;
use crate::host::checkpoints::strategy::manual_commit::{WriteCommittedOptions, write_committed};

fn isolated_executor_repo_root() -> PathBuf {
    let temp = tempdir().expect("temp dir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create isolated executor repo root");
    std::mem::forget(temp);
    repo_root
}

fn executor_test_cfg() -> DevqlConfig {
    let repo_root = isolated_executor_repo_root();
    DevqlConfig {
        config_root: repo_root.clone(),
        repo_root,
        repo: RepoIdentity {
            provider: "local".to_string(),
            organization: "bitloops".to_string(),
            name: "bitloops".to_string(),
            identity: "bitloops/bitloops".to_string(),
            repo_id: "repo-1".to_string(),
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

fn executor_events_cfg() -> EventsBackendConfig {
    EventsBackendConfig {
        duckdb_path: None,
        clickhouse_url: None,
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: None,
    }
}

fn executor_test_cfg_for_repo_root(repo_root: PathBuf) -> DevqlConfig {
    let mut cfg = executor_test_cfg();
    cfg.config_root = repo_root.clone();
    cfg.repo_root = repo_root;
    cfg
}

fn configure_executor_sqlite_backend(repo_root: &std::path::Path) {
    let sqlite_path = repo_root.join(".bitloops/stores/relational/relational.db");
    if let Some(parent) = sqlite_path.parent() {
        std::fs::create_dir_all(parent).expect("create sqlite parent");
    }
    rusqlite::Connection::open(&sqlite_path).expect("create sqlite file");
    write_repo_daemon_config(
        repo_root,
        format!(
            "[stores.relational]\nsqlite_path = {path:?}\n",
            path = sqlite_path.to_string_lossy()
        ),
    );
}

async fn sqlite_relational_with_sql(sql: &str) -> RelationalStorage {
    let temp = tempdir().expect("temp dir");
    let db_path = temp.path().join("relational.sqlite");
    sqlite_exec_path_allow_create(&db_path, sql)
        .await
        .expect("create sqlite database");
    std::mem::forget(temp);
    RelationalStorage::local_only(db_path)
}

async fn duckdb_path_with_sql(sql: &str) -> PathBuf {
    let temp = tempdir().expect("temp dir");
    let db_path = temp.path().join("events.duckdb");
    duckdb_exec_path_allow_create(&db_path, sql)
        .await
        .expect("create duckdb database");
    std::mem::forget(temp);
    db_path
}

fn checkpoint_file_snapshot_projection_table_sql() -> &'static str {
    "CREATE TABLE checkpoint_files (
        relation_id TEXT NOT NULL,
        repo_id TEXT NOT NULL,
        checkpoint_id TEXT NOT NULL,
        session_id TEXT NOT NULL,
        event_time TEXT NOT NULL,
        agent TEXT NOT NULL,
        branch TEXT NOT NULL,
        strategy TEXT NOT NULL,
        commit_sha TEXT NOT NULL,
        change_kind TEXT NOT NULL,
        path_before TEXT,
        path_after TEXT,
        blob_sha_before TEXT,
        blob_sha_after TEXT,
        copy_source_path TEXT,
        copy_source_blob_sha TEXT
    );"
}

fn test_write_committed_options(
    checkpoint_id: &str,
    session_id: &str,
    agent: &str,
    files_touched: &[&str],
    transcript: &str,
) -> WriteCommittedOptions {
    WriteCommittedOptions {
        checkpoint_id: checkpoint_id.to_string(),
        session_id: session_id.to_string(),
        strategy: "manual-commit".to_string(),
        agent: agent.to_string(),
        transcript: transcript.as_bytes().to_vec(),
        prompts: None,
        context: None,
        checkpoints_count: 1,
        files_touched: files_touched
            .iter()
            .map(|path| (*path).to_string())
            .collect(),
        token_usage_input: None,
        token_usage_output: None,
        token_usage_api_call_count: None,
        turn_id: String::new(),
        transcript_identifier_at_start: String::new(),
        checkpoint_transcript_start: 0,
        token_usage: None,
        initial_attribution: None,
        author_name: "Bitloops Test".to_string(),
        author_email: "bitloops-test@example.com".to_string(),
        summary: None,
        is_task: false,
        tool_use_id: String::new(),
        agent_id: String::new(),
        transcript_path: String::new(),
        subagent_transcript_path: String::new(),
    }
}

#[test]
fn normalise_duckdb_event_row_parses_json_payload_columns() {
    let row = json!({
        "event_type": "checkpoint_committed",
        "files_touched": "[\"src/lib.rs\"]",
        "payload": "{\"agent\":\"codex\"}"
    });

    let normalized = normalise_duckdb_event_row(row);

    assert_eq!(
        normalized.get("files_touched"),
        Some(&json!(["src/lib.rs"]))
    );
    assert_eq!(normalized.get("payload"), Some(&json!({"agent": "codex"})));
}

#[test]
fn normalise_relational_result_row_parses_json_fields() {
    let row = json!({
        "artefact_id": "a-1",
        "modifiers": "[\"pub\"]",
        "metadata": "{\"score\": 0.9}"
    });

    let normalized = normalise_relational_result_row(row);

    assert_eq!(normalized.get("modifiers"), Some(&json!(["pub"])));
    assert_eq!(normalized.get("metadata"), Some(&json!({"score": 0.9})));
}

#[tokio::test]
async fn execute_duckdb_pipeline_reads_telemetry_rows() {
    let duckdb_path = duckdb_path_with_sql(
        "CREATE TABLE checkpoint_events (
            repo_id TEXT,
            event_type TEXT,
            event_time TEXT,
            checkpoint_id TEXT,
            session_id TEXT,
            agent TEXT,
            commit_sha TEXT,
            branch TEXT,
            strategy TEXT,
            files_touched TEXT,
            payload TEXT
        );
        INSERT INTO checkpoint_events VALUES (
            'repo-1',
            'checkpoint_committed',
            '2026-03-17T12:00:00Z',
            'checkpoint-1',
            'session-1',
            'codex',
            'commit-1',
            'main',
            'manual',
            '[\"src/lib.rs\"]',
            '{\"ok\":true}'
        );",
    )
    .await;
    let cfg = executor_test_cfg();
    let parsed = parse_devql_query("telemetry()").expect("parsed devql query");
    let events_cfg = EventsBackendConfig {
        duckdb_path: Some(duckdb_path.to_string_lossy().to_string()),
        clickhouse_url: None,
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: None,
    };

    let rows = execute_duckdb_pipeline(&cfg, &events_cfg, &parsed)
        .await
        .expect("execute duckdb telemetry pipeline");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("commit_sha"), Some(&json!("commit-1")));
    assert_eq!(rows[0].get("files_touched"), Some(&json!(["src/lib.rs"])));
}

#[tokio::test]
async fn checkpoint_matches_for_artefact_snapshot_reads_projection_rows() {
    let relational = sqlite_relational_with_sql(
        &format!(
            "{}\
             INSERT INTO checkpoint_files VALUES \
                ('relation-1', 'repo-1', 'checkpoint-1', 'session-1', '2026-03-17T12:00:00Z', 'codex', 'main', 'manual', 'commit-1', 'modify', 'src/lib.rs', 'src/lib.rs', 'blob-1', 'blob-1', NULL, NULL), \
                ('relation-2', 'repo-1', 'checkpoint-2', 'session-2', '2026-03-18T12:00:00Z', 'codex', 'main', 'manual', 'commit-2', 'modify', 'src/lib.rs', 'src/lib.rs', 'blob-1', 'blob-1', NULL, NULL), \
                ('relation-3', 'repo-1', 'checkpoint-3', 'session-3', '2026-03-19T12:00:00Z', 'codex', 'main', 'manual', 'commit-3', 'modify', 'src/other.rs', 'src/other.rs', 'blob-1', 'blob-1', NULL, NULL);",
            checkpoint_file_snapshot_projection_table_sql()
        ),
    )
    .await;

    let matches =
        checkpoint_matches_for_artefact_snapshot(&relational, "repo-1", "./src/lib.rs", "blob-1")
            .await
            .expect("load checkpoint matches");

    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].checkpoint_id, "checkpoint-2");
    assert_eq!(matches[1].checkpoint_id, "checkpoint-1");
    assert!(matches.iter().all(|row| row.path == "src/lib.rs"));
}

#[tokio::test]
async fn attach_chat_history_to_artefacts_uses_empty_history_when_blob_is_missing() {
    let relational = sqlite_relational_with_sql(
        "CREATE TABLE file_state (
            repo_id TEXT,
            path TEXT,
            blob_sha TEXT,
            commit_sha TEXT
        );",
    )
    .await;
    let cfg = executor_test_cfg();

    let rows = attach_chat_history_to_artefacts(
        &cfg,
        &executor_events_cfg(),
        &relational,
        "repo-1",
        vec![json!({
            "artefact_id": "artefact-1",
            "path": "src/lib.rs",
            "blob_sha": ""
        })],
    )
    .await
    .expect("attach empty chat history");

    assert_eq!(rows[0].get("chat_history"), Some(&json!([])));
}

#[tokio::test]
async fn attach_chat_history_to_artefacts_uses_projection_without_file_state() {
    let repo = seed_git_repo();
    configure_executor_sqlite_backend(repo.path());
    let cfg = executor_test_cfg_for_repo_root(repo.path().to_path_buf());

    write_committed(
        repo.path(),
        test_write_committed_options(
            "a1b2c3d4e5f6",
            "session-older",
            "codex",
            &["src/lib.rs"],
            r#"{"role":"user","content":"Explain older()"}
{"role":"assistant","content":"older() was superseded."}"#,
        ),
    )
    .expect("write older checkpoint");
    write_committed(
        repo.path(),
        test_write_committed_options(
            "b1c2d3e4f5a6",
            "session-newer",
            "codex",
            &["src/lib.rs"],
            r#"{"role":"user","content":"Explain newer()"}
{"role":"assistant","content":"newer() is current."}"#,
        ),
    )
    .expect("write newer checkpoint");
    write_committed(
        repo.path(),
        test_write_committed_options(
            "c1d2e3f4a5b6",
            "session-other",
            "codex",
            &["src/other.rs"],
            r#"{"role":"user","content":"Explain other()"}
{"role":"assistant","content":"other() reuses the blob."}"#,
        ),
    )
    .expect("write other-path checkpoint");

    let relational = sqlite_relational_with_sql(
        &format!(
            "{}\
             INSERT INTO checkpoint_files VALUES \
                ('relation-1', 'repo-1', 'a1b2c3d4e5f6', 'session-older', '2026-03-17T12:00:00Z', 'codex', 'main', 'manual-commit', 'commit-1', 'modify', 'src/lib.rs', 'src/lib.rs', 'blob-1', 'blob-1', NULL, NULL), \
                ('relation-2', 'repo-1', 'b1c2d3e4f5a6', 'session-newer', '2026-03-18T12:00:00Z', 'codex', 'main', 'manual-commit', 'commit-2', 'modify', 'src/lib.rs', 'src/lib.rs', 'blob-1', 'blob-1', NULL, NULL), \
                ('relation-3', 'repo-1', 'c1d2e3f4a5b6', 'session-other', '2026-03-19T12:00:00Z', 'codex', 'main', 'manual-commit', 'commit-3', 'modify', 'src/other.rs', 'src/other.rs', 'blob-1', 'blob-1', NULL, NULL);",
            checkpoint_file_snapshot_projection_table_sql()
        ),
    )
    .await;

    let rows = attach_chat_history_to_artefacts(
        &cfg,
        &executor_events_cfg(),
        &relational,
        "repo-1",
        vec![json!({
            "artefact_id": "artefact-1",
            "path": "src/lib.rs",
            "blob_sha": "blob-1"
        })],
    )
    .await
    .expect("attach projection-backed chat history");

    let history = rows[0]
        .get("chat_history")
        .and_then(Value::as_array)
        .expect("chat history array");
    assert_eq!(history.len(), 2);
    assert_eq!(
        history[0].get("checkpoint_id"),
        Some(&json!("b1c2d3e4f5a6"))
    );
    assert_eq!(
        history[1].get("checkpoint_id"),
        Some(&json!("a1b2c3d4e5f6"))
    );
    assert_eq!(
        history[0]
            .get("chat")
            .and_then(|value| value.get("messages"))
            .and_then(Value::as_array)
            .and_then(|messages| messages[0].get("text"))
            .and_then(Value::as_str),
        Some("Explain newer()")
    );
    assert_eq!(
        history[1]
            .get("chat")
            .and_then(|value| value.get("messages"))
            .and_then(Value::as_array)
            .and_then(|messages| messages[1].get("text"))
            .and_then(Value::as_str),
        Some("older() was superseded.")
    );
}

#[tokio::test]
async fn session_chat_payload_returns_none_when_checkpoint_is_missing() {
    let temp = tempdir().expect("temp dir");
    let cfg = executor_test_cfg_for_repo_root(temp.path().to_path_buf());
    let mut cache = HashMap::new();

    let payload = session_chat_payload(&cfg, "checkpoint-1", "session-1", &mut cache);

    assert!(payload.is_none());
}

#[tokio::test]
async fn execute_registered_stages_routes_to_test_harness_pack() {
    let temp = tempdir().expect("temp dir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    configure_executor_sqlite_backend(&repo_root);
    let cfg = executor_test_cfg_for_repo_root(repo_root);
    let parsed = parse_devql_query(r#"repo("bitloops")->tests()->limit(3)"#).expect("parse");

    let rows = execute_registered_stages(
        &cfg,
        &parsed,
        vec![json!({
            "artefact_id": "a-1",
            "path": "src/lib.rs",
            "canonical_kind": "function",
            "symbol_fqn": "src/lib.rs::a_1",
            "start_line": 1,
            "end_line": 3
        })],
    )
    .await
    .expect("execute registered stages");

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]
            .get("artefact")
            .and_then(|value| value.get("artefact_id"))
            .and_then(Value::as_str),
        Some("a-1")
    );
    assert_eq!(
        rows[0]
            .get("summary")
            .and_then(|value| value.get("total_covering_tests"))
            .and_then(Value::as_i64),
        Some(0)
    );
}

#[tokio::test]
async fn execute_registered_stages_with_composition_rejects_undeclared_cross_pack_dependency() {
    let temp = tempdir().expect("temp dir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    configure_executor_sqlite_backend(&repo_root);
    let cfg = executor_test_cfg_for_repo_root(repo_root);
    let parsed = parse_devql_query(r#"repo("bitloops")->knowledge()->limit(1)"#).expect("parse");
    let composition = RegisteredStageCompositionContext {
        caller_capability_id: "test_harness".to_string(),
        depth: 1,
        max_depth: 3,
    };

    let err = execute_registered_stages_with_composition(
        &cfg,
        &parsed,
        vec![json!({ "artefact_id": "a-1" })],
        Some(&composition),
    )
    .await
    .expect_err("undeclared cross-pack invocation must fail");

    assert!(
        err.to_string().contains("no descriptor dependency")
            && err.to_string().contains("cross_pack_access"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn execute_registered_stages_with_composition_allows_declared_cross_pack_dependency() {
    let temp = tempdir().expect("temp dir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    configure_executor_sqlite_backend(&repo_root);
    let cfg = executor_test_cfg_for_repo_root(repo_root);
    let parsed = parse_devql_query(r#"repo("bitloops")->tests()->limit(3)"#).expect("parse");
    let composition = RegisteredStageCompositionContext {
        caller_capability_id: "knowledge".to_string(),
        depth: 1,
        max_depth: 3,
    };

    let rows = execute_registered_stages_with_composition(
        &cfg,
        &parsed,
        vec![json!({
            "artefact_id": "a-1",
            "path": "src/lib.rs",
            "canonical_kind": "function",
            "symbol_fqn": "src/lib.rs::a_1",
            "start_line": 1,
            "end_line": 3
        })],
        Some(&composition),
    )
    .await
    .expect("declared dependency should allow cross-pack invocation");

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]
            .get("summary")
            .and_then(|value| value.get("total_covering_tests"))
            .and_then(Value::as_i64),
        Some(0)
    );
}

#[tokio::test]
async fn execute_registered_stages_with_composition_rejects_depth_overflow() {
    let temp = tempdir().expect("temp dir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    configure_executor_sqlite_backend(&repo_root);
    let cfg = executor_test_cfg_for_repo_root(repo_root);
    let parsed = parse_devql_query(r#"repo("bitloops")->tests()->limit(3)"#).expect("parse");
    let composition = RegisteredStageCompositionContext {
        caller_capability_id: "knowledge".to_string(),
        depth: 4,
        max_depth: 3,
    };

    let err = execute_registered_stages_with_composition(
        &cfg,
        &parsed,
        vec![json!({
            "artefact_id": "a-1",
            "path": "src/lib.rs",
            "canonical_kind": "function",
            "symbol_fqn": "src/lib.rs::a_1",
            "start_line": 1,
            "end_line": 3
        })],
        Some(&composition),
    )
    .await
    .expect_err("composition depth overflow must fail");

    assert!(
        err.to_string()
            .contains("DevQL composition depth 4 exceeds configured max depth 3"),
        "unexpected error: {err}"
    );
}
