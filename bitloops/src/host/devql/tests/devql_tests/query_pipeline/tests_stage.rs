use super::super::*;

#[test]
fn parse_devql_tests_stage_basic() {
    let parsed =
        parse_devql_query(r#"repo("r")->file("src/lib.rs")->artefacts(kind:"function")->tests()"#)
            .unwrap();
    assert!(parsed.has_artefacts_stage);
    assert_eq!(parsed.registered_stages.len(), 1);
    assert_eq!(parsed.registered_stages[0].stage_name, "tests");
    assert!(parsed.registered_stages[0].args.is_empty());
}

#[test]
fn parse_devql_tests_stage_with_filters() {
    let parsed = parse_devql_query(
        r#"repo("r")->artefacts()->tests(min_confidence:0.5,linkage_source:"static_analysis")"#,
    )
    .unwrap();
    assert_eq!(parsed.registered_stages.len(), 1);
    assert_eq!(parsed.registered_stages[0].stage_name, "tests");
    assert_eq!(
        parsed.registered_stages[0]
            .args
            .get("min_confidence")
            .map(String::as_str),
        Some("0.5")
    );
    assert_eq!(
        parsed.registered_stages[0]
            .args
            .get("linkage_source")
            .map(String::as_str),
        Some("static_analysis")
    );
}

#[test]
fn parse_devql_rejects_legacy_internal_core_stages() {
    for q in [
        r#"repo("r")->__core_test_links(artefact_id:"artefact::a_1",min_confidence:0.5,linkage_source:"static_analysis")->limit(7)"#,
        r#"repo("r")->__core_line_coverage()"#,
        r#"repo("r")->__core_branch_coverage(artefact_id:"a")"#,
        r#"repo("r")->__core_coverage_metadata()"#,
    ] {
        let err = parse_devql_query(q).unwrap_err();
        assert!(
            err.to_string().contains("unsupported DevQL stage"),
            "expected unsupported stage for {q:?}, got {err}"
        );
    }
}

#[tokio::test]
async fn execute_devql_query_rejects_tests_without_artefacts() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(r#"repo("temp2")->tests()->limit(1)"#).unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("tests() requires an artefacts() stage")
    );
}

#[tokio::test]
async fn execute_devql_query_rejects_tests_with_deps() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->dependencies(kind:"calls")->tests()->limit(1)"#,
    )
    .unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("tests() cannot be combined with dependencies()")
    );
}

#[tokio::test]
async fn execute_devql_query_rejects_tests_with_clones() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->clones()->tests()->limit(1)"#,
    )
    .unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("tests() cannot be combined with clones()")
    );
}

#[tokio::test]
async fn execute_devql_query_rejects_tests_with_chat_history() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->chatHistory()->tests()->limit(1)"#,
    )
    .unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("tests() cannot be combined with chatHistory()")
    );
}

#[tokio::test]
async fn execute_devql_query_rejects_tests_with_non_test_harness_registered_stages() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->tests()->knowledge()->limit(1)"#,
    )
    .unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains(
        "tests() cannot currently be combined with additional registered capability-pack stages"
    ));
}

#[tokio::test]
async fn execute_registered_tests_stage_returns_covering_tests() {
    let temp = tempdir().expect("tempdir");
    let home = TempDir::new().expect("home dir");
    let home_path = home.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
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
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let mut cfg = test_cfg();
    cfg.daemon_config_root = repo_root.clone();
    cfg.repo_root = repo_root;
    let events_cfg = default_events_cfg();
    let sqlite_path = temp.path().join("relational.sqlite");
    if let Some(parent) = sqlite_path.parent() {
        std::fs::create_dir_all(parent).expect("create relational parent dir");
    }
    write_repo_daemon_config(
        &cfg.repo_root,
        format!(
            "[stores.relational]\nsqlite_path = {path:?}\n",
            path = sqlite_path.to_string_lossy()
        ),
    );
    // Use _for_repo to avoid cwd dependency under parallel test execution.
    let backends = crate::config::resolve_store_backend_config_for_repo(&cfg.repo_root)
        .expect("resolve backend config");
    let host_sqlite_path = crate::config::resolve_sqlite_db_path_for_repo(
        &cfg.repo_root,
        backends.relational.sqlite_path.as_deref(),
    )
    .expect("resolve host sqlite path");
    assert_eq!(host_sqlite_path, sqlite_path);
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");

    // Create test harness tables (not part of the DevQL relational schema)
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS test_artefacts_current (
            artefact_id TEXT NOT NULL,
            symbol_id TEXT NOT NULL,
            repo_id TEXT NOT NULL,
            commit_sha TEXT NOT NULL,
            blob_sha TEXT NOT NULL,
            path TEXT NOT NULL,
            language TEXT NOT NULL,
            canonical_kind TEXT NOT NULL,
            language_kind TEXT,
            symbol_fqn TEXT,
            name TEXT NOT NULL,
            parent_artefact_id TEXT,
            parent_symbol_id TEXT,
            start_line BIGINT NOT NULL,
            end_line BIGINT NOT NULL,
            start_byte BIGINT,
            end_byte BIGINT,
            signature TEXT,
            modifiers TEXT NOT NULL DEFAULT '[]',
            docstring TEXT,
            content_hash TEXT,
            discovery_source TEXT NOT NULL,
            revision_kind TEXT NOT NULL DEFAULT 'commit',
            revision_id TEXT NOT NULL,
            PRIMARY KEY (repo_id, symbol_id)
        );
        CREATE TABLE IF NOT EXISTS test_artefact_edges_current (
            edge_id TEXT PRIMARY KEY,
            repo_id TEXT NOT NULL,
            commit_sha TEXT NOT NULL,
            blob_sha TEXT NOT NULL,
            path TEXT NOT NULL,
            from_artefact_id TEXT NOT NULL,
            from_symbol_id TEXT NOT NULL,
            to_artefact_id TEXT,
            to_symbol_id TEXT,
            to_symbol_ref TEXT,
            edge_kind TEXT NOT NULL,
            language TEXT NOT NULL,
            start_line BIGINT,
            end_line BIGINT,
            metadata TEXT NOT NULL DEFAULT '{}',
            revision_kind TEXT NOT NULL DEFAULT 'commit',
            revision_id TEXT NOT NULL
        );
        "#,
    )
    .expect("create test harness tables");

    // Insert a production artefact
    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, path, content_id, symbol_id, artefact_id, language,
            canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id,
            start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, NULL, ?10, ?11, ?12, ?13, NULL, ?14, NULL,
            '2026-01-01T00:00:00Z'
        )",
        rusqlite::params![
            cfg.repo.repo_id.as_str(),
            "src/user/service.rs",
            "blob-1",
            "sym::create_user",
            "artefact::create_user",
            "rust",
            "function",
            "function_item",
            "src/user/service.rs::create_user",
            1,
            3,
            0,
            42,
            "[]",
        ],
    )
    .expect("insert production artefact");

    // Insert test harness data
    conn.execute(
        "INSERT INTO test_artefacts_current (
            artefact_id, symbol_id, repo_id, commit_sha, blob_sha, path, language,
            canonical_kind, symbol_fqn, name, start_line, end_line, modifiers,
            discovery_source, revision_kind, revision_id
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        rusqlite::params![
            "test-artefact::suite::tests",
            "suite::tests",
            cfg.repo.repo_id.as_str(),
            "commit-1",
            "blob-test-1",
            "src/user/service_tests.rs",
            "rust",
            "test_suite",
            "tests",
            "tests",
            1i64,
            10i64,
            "[]",
            "source",
            "commit",
            "commit-1",
        ],
    )
    .expect("insert test suite");

    conn.execute(
        "INSERT INTO test_artefacts_current (
            artefact_id, symbol_id, repo_id, commit_sha, blob_sha, path, language,
            canonical_kind, symbol_fqn, name, parent_artefact_id, parent_symbol_id,
            start_line, end_line, modifiers, discovery_source, revision_kind, revision_id
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        rusqlite::params![
            "test-artefact::scenario::test_create_user",
            "scenario::test_create_user",
            cfg.repo.repo_id.as_str(),
            "commit-1",
            "blob-test-1",
            "src/user/service_tests.rs",
            "rust",
            "test_scenario",
            "tests.test_create_user",
            "test_create_user",
            "test-artefact::suite::tests",
            "suite::tests",
            5i64,
            8i64,
            "[]",
            "source",
            "commit",
            "commit-1",
        ],
    )
    .expect("insert test scenario");

    conn.execute(
        "INSERT INTO test_artefact_edges_current (
            edge_id, repo_id, commit_sha, blob_sha, path, from_artefact_id, from_symbol_id,
            to_artefact_id, to_symbol_id, edge_kind, language, start_line, end_line,
            metadata, revision_kind, revision_id
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        rusqlite::params![
            "link::1",
            cfg.repo.repo_id.as_str(),
            "commit-1",
            "blob-test-1",
            "src/user/service_tests.rs",
            "test-artefact::scenario::test_create_user",
            "scenario::test_create_user",
            "artefact::create_user",
            "sym::create_user",
            "tests",
            "rust",
            5i64,
            8i64,
            "{\"confidence\":0.6,\"link_source\":\"static_analysis\",\"linkage_status\":\"resolved\"}",
            "commit",
            "commit-1",
        ],
    )
    .expect("insert test link");
    let covering_rows_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM test_artefact_edges_current WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count test links");
    assert_eq!(covering_rows_count, 1);

    let parsed = parse_devql_query(
        r#"repo("temp2")->file("src/user/service.rs")->artefacts(kind:"function")->tests()->limit(10)"#,
    )
    .expect("parse query");
    let base_rows = execute_devql_query(&cfg, &parsed, &events_cfg, Some(&relational))
        .await
        .expect("execute base pipeline");
    let rows = execute_registered_stages(&cfg, &parsed, base_rows)
        .await
        .expect("execute tests pipeline");

    assert_eq!(rows.len(), 1);
    let artefact = rows[0].get("artefact").expect("should have artefact");
    assert_eq!(
        artefact.get("artefact_id").and_then(Value::as_str),
        Some("artefact::create_user")
    );

    let covering_tests = rows[0]
        .get("covering_tests")
        .and_then(Value::as_array)
        .expect("should have covering_tests");
    assert_eq!(covering_tests.len(), 1);
    assert_eq!(
        covering_tests[0].get("test_name").and_then(Value::as_str),
        Some("test_create_user")
    );

    let summary = rows[0].get("summary").expect("should have summary");
    assert_eq!(
        summary.get("total_covering_tests").and_then(Value::as_i64),
        Some(1)
    );
}

#[tokio::test]
async fn execute_registered_tests_stage_scopes_asof_commit_links_by_commit() {
    let temp = tempdir().expect("tempdir");
    let home = TempDir::new().expect("home dir");
    let home_path = home.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
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
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let mut cfg = test_cfg();
    cfg.daemon_config_root = repo_root.clone();
    cfg.repo_root = repo_root;
    let sqlite_path = temp.path().join("relational.sqlite");
    if let Some(parent) = sqlite_path.parent() {
        std::fs::create_dir_all(parent).expect("create relational parent dir");
    }
    write_repo_daemon_config(
        &cfg.repo_root,
        format!(
            "[stores.relational]\nsqlite_path = {path:?}\n",
            path = sqlite_path.to_string_lossy()
        ),
    );
    crate::capability_packs::test_harness::storage::init_schema_for_repo(&cfg.repo_root)
        .expect("initialise test harness schema");

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let repo_id = cfg.repo.repo_id.as_str();

    for (artefact_id, symbol_id, name, parent_artefact_id, parent_symbol_id, commit_sha) in [
        (
            "test-artefact::suite::tests_old",
            "suite::tests_old",
            "tests_old",
            None,
            None,
            "commit-old",
        ),
        (
            "test-artefact::suite::tests_new",
            "suite::tests_new",
            "tests_new",
            None,
            None,
            "commit-new",
        ),
        (
            "test-artefact::scenario::test_create_user_old",
            "scenario::test_create_user_old",
            "test_create_user_old",
            Some("test-artefact::suite::tests_old"),
            Some("suite::tests_old"),
            "commit-old",
        ),
        (
            "test-artefact::scenario::test_create_user_new",
            "scenario::test_create_user_new",
            "test_create_user_new",
            Some("test-artefact::suite::tests_new"),
            Some("suite::tests_new"),
            "commit-new",
        ),
    ] {
        let canonical_kind = if symbol_id.starts_with("suite::") {
            "test_suite"
        } else {
            "test_scenario"
        };
        let symbol_fqn = if symbol_id.starts_with("suite::") {
            name.to_string()
        } else {
            format!(
                "{}.{name}",
                parent_symbol_id.expect("scenario parent symbol")
            )
        };
        conn.execute(
            "INSERT INTO test_artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, symbol_fqn, name, parent_artefact_id, parent_symbol_id,
                start_line, end_line, modifiers, discovery_source
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            rusqlite::params![
                repo_id,
                "src/user/service_tests.rs",
                format!("blob:{commit_sha}"),
                symbol_id,
                artefact_id,
                "rust",
                canonical_kind,
                symbol_fqn,
                name,
                parent_artefact_id,
                parent_symbol_id,
                1i64,
                10i64,
                "[]",
                "source",
            ],
        )
        .expect("insert test artefact");
    }

    for (edge_id, from_artefact_id, from_symbol_id, commit_sha, confidence) in [
        (
            "link::old",
            "test-artefact::scenario::test_create_user_old",
            "scenario::test_create_user_old",
            "commit-old",
            0.6,
        ),
        (
            "link::new",
            "test-artefact::scenario::test_create_user_new",
            "scenario::test_create_user_new",
            "commit-new",
            0.9,
        ),
    ] {
        conn.execute(
            "INSERT INTO test_artefact_edges_current (
                repo_id, path, content_id, edge_id, from_artefact_id, from_symbol_id,
                to_artefact_id, to_symbol_id, edge_kind, language, start_line, end_line, metadata
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                repo_id,
                "src/user/service_tests.rs",
                format!("blob:{commit_sha}"),
                edge_id,
                from_artefact_id,
                from_symbol_id,
                "artefact::create_user",
                "sym::create_user",
                "tests",
                "rust",
                5i64,
                8i64,
                format!("{{\"confidence\":{confidence},\"link_source\":\"static_analysis\",\"linkage_status\":\"resolved\"}}"),
            ],
        )
        .expect("insert test link");
    }

    let parsed = parse_devql_query(
        r#"repo("temp2")->asOf(commit:"commit-old")->file("src/user/service.rs")->artefacts(kind:"function")->tests()->limit(10)"#,
    )
    .expect("parse query");
    let rows = execute_registered_stages(
        &cfg,
        &parsed,
        vec![json!({
            "artefact_id": "historical:src/user/service.rs:create_user",
            "symbol_id": "sym::create_user",
            "symbol_fqn": "src/user/service.rs::create_user",
            "canonical_kind": "function",
            "path": "src/user/service.rs",
            "start_line": 1,
            "end_line": 3,
        })],
    )
    .await
    .expect("execute historical tests stage");

    let covering_tests = rows[0]
        .get("covering_tests")
        .and_then(Value::as_array)
        .expect("should have covering_tests");
    assert_eq!(covering_tests.len(), 2);
    assert_eq!(
        covering_tests[0].get("test_name").and_then(Value::as_str),
        Some("test_create_user_new")
    );
}
