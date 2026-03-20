fn executor_test_cfg() -> DevqlConfig {
    DevqlConfig {
        repo_root: PathBuf::from("."),
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
        embedding_provider: None,
        embedding_model: None,
        embedding_api_key: None,
    }
}

fn executor_events_cfg(provider: EventsProvider) -> EventsBackendConfig {
    EventsBackendConfig {
        provider,
        duckdb_path: None,
        clickhouse_url: None,
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: None,
    }
}

fn executor_test_cfg_for_repo_root(repo_root: PathBuf) -> DevqlConfig {
    let mut cfg = executor_test_cfg();
    cfg.repo_root = repo_root;
    cfg
}

async fn sqlite_relational_with_sql(sql: &str) -> RelationalStorage {
    let temp = tempdir().expect("temp dir");
    let db_path = temp.path().join("relational.sqlite");
    sqlite_exec_path_allow_create(&db_path, sql)
        .await
        .expect("create sqlite database");
    std::mem::forget(temp);
    RelationalStorage::Sqlite { path: db_path }
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

#[test]
fn normalise_duckdb_event_row_parses_json_payload_columns() {
    let row = json!({
        "event_type": "checkpoint_committed",
        "files_touched": "[\"src/lib.rs\"]",
        "payload": "{\"agent\":\"codex\"}"
    });

    let normalized = normalise_duckdb_event_row(row);

    assert_eq!(normalized.get("files_touched"), Some(&json!(["src/lib.rs"])));
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
        provider: EventsProvider::DuckDb,
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
async fn commit_shas_for_artefact_blob_reads_sqlite_rows() {
    let relational = sqlite_relational_with_sql(
        "CREATE TABLE file_state (
            repo_id TEXT,
            path TEXT,
            blob_sha TEXT,
            commit_sha TEXT
        );
        INSERT INTO file_state VALUES
            ('repo-1', 'src/lib.rs', 'blob-1', 'commit-1'),
            ('repo-1', 'src/lib.rs', 'blob-1', 'commit-2');",
    )
    .await;

    let commit_shas = commit_shas_for_artefact_blob(&relational, "repo-1", "src/lib.rs", "blob-1")
        .await
        .expect("load commit shas");

    assert_eq!(commit_shas, vec!["commit-1".to_string(), "commit-2".to_string()]);
}

#[tokio::test]
async fn checkpoint_events_for_commits_reads_duckdb_rows() {
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
            '{\"step\":1}'
        );",
    )
    .await;
    let cfg = executor_test_cfg();
    let events_cfg = EventsBackendConfig {
        provider: EventsProvider::DuckDb,
        duckdb_path: Some(duckdb_path.to_string_lossy().to_string()),
        clickhouse_url: None,
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: None,
    };

    let rows = checkpoint_events_for_commits(
        &cfg,
        &events_cfg,
        "repo-1",
        "src/lib.rs",
        &["commit-1".to_string()],
    )
    .await
    .expect("checkpoint events");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("checkpoint_id"), Some(&json!("checkpoint-1")));
    assert_eq!(rows[0].get("payload"), Some(&json!({"step": 1})));
}

#[tokio::test]
async fn blob_shas_changed_in_events_reads_duckdb_and_sqlite_stores() {
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
            '{}'
        );",
    )
    .await;
    let relational = sqlite_relational_with_sql(
        "CREATE TABLE file_state (
            repo_id TEXT,
            commit_sha TEXT,
            blob_sha TEXT
        );
        INSERT INTO file_state VALUES ('repo-1', 'commit-1', 'blob-1');",
    )
    .await;
    let cfg = executor_test_cfg();
    let events_cfg = EventsBackendConfig {
        provider: EventsProvider::DuckDb,
        duckdb_path: Some(duckdb_path.to_string_lossy().to_string()),
        clickhouse_url: None,
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: None,
    };

    let blob_shas =
        blob_shas_changed_in_events(&cfg, &events_cfg, &relational, "repo-1", None, None)
            .await
            .expect("blob shas");

    assert_eq!(blob_shas, vec!["blob-1".to_string()]);
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
        &executor_events_cfg(EventsProvider::DuckDb),
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
    let cfg = executor_test_cfg_for_repo_root(repo_root);
    let parsed =
        parse_devql_query(r#"repo("bitloops")->test_harness_tests()->limit(3)"#).expect("parse");

    let rows = execute_registered_stages(&cfg, &parsed, vec![json!({ "artefact_id": "a-1" })])
        .await
        .expect("execute registered stages");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("capability"), Some(&json!("test_harness")));
    assert_eq!(rows[0].get("stage"), Some(&json!("test_harness_tests")));
    assert_eq!(rows[0].get("status"), Some(&json!("dependency_gated")));
}
