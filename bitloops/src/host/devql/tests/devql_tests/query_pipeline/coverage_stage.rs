use super::super::*;

#[test]
fn parse_devql_coverage_stage_basic() {
    let parsed = parse_devql_query(
        r#"repo("r")->file("src/lib.rs")->artefacts(kind:"function")->coverage()"#,
    )
    .unwrap();
    assert!(parsed.has_artefacts_stage);
    assert_eq!(parsed.registered_stages.len(), 1);
    assert_eq!(parsed.registered_stages[0].stage_name, "coverage");
}

#[tokio::test]
async fn execute_devql_query_rejects_coverage_without_artefacts() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(r#"repo("temp2")->coverage()->limit(1)"#).unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("coverage() requires an artefacts() stage")
    );
}

#[tokio::test]
async fn execute_devql_query_rejects_coverage_with_deps() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->dependencies(kind:"calls")->coverage()->limit(1)"#,
    )
    .unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("coverage() cannot be combined with dependencies()")
    );
}

#[tokio::test]
async fn execute_devql_query_rejects_coverage_with_tests() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->tests()->coverage()->limit(1)"#,
    )
    .unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("coverage() cannot be combined with tests()")
    );
}

#[tokio::test]
async fn execute_devql_query_rejects_coverage_with_non_test_harness_registered_stages() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->coverage()->knowledge()->limit(1)"#,
    )
    .unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains(
        "coverage() cannot currently be combined with additional registered capability-pack stages"
    ));
}

#[tokio::test]
async fn execute_registered_coverage_stage_returns_coverage_data() {
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

    // Create coverage tables (not part of the DevQL relational schema)
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS coverage_captures (
            capture_id TEXT PRIMARY KEY,
            repo_id TEXT NOT NULL,
            commit_sha TEXT NOT NULL,
            tool TEXT NOT NULL DEFAULT 'unknown',
            format TEXT NOT NULL DEFAULT 'lcov',
            scope_kind TEXT NOT NULL DEFAULT 'workspace',
            subject_test_symbol_id TEXT,
            line_truth INTEGER NOT NULL DEFAULT 1,
            branch_truth INTEGER NOT NULL DEFAULT 0,
            captured_at TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'complete',
            metadata_json TEXT
        );
        CREATE TABLE IF NOT EXISTS coverage_hits (
            capture_id TEXT NOT NULL REFERENCES coverage_captures(capture_id) ON DELETE CASCADE,
            production_symbol_id TEXT NOT NULL,
            file_path TEXT NOT NULL,
            line INTEGER NOT NULL,
            branch_id INTEGER NOT NULL DEFAULT -1,
            covered INTEGER NOT NULL,
            hit_count INTEGER DEFAULT 0,
            PRIMARY KEY (capture_id, production_symbol_id, line, branch_id)
        );
        "#,
    )
    .expect("create coverage tables");

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
            42,
            89,
            0,
            500,
            "[]",
        ],
    )
    .expect("insert production artefact");

    // Insert a coverage capture
    conn.execute(
        "INSERT INTO coverage_captures (
            capture_id, repo_id, commit_sha, tool, format,
            line_truth, branch_truth, captured_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            "capture-1",
            cfg.repo.repo_id.as_str(),
            "commit-1",
            "lcov",
            "lcov",
            1,
            1,
            "2026-01-01T00:00:00Z",
        ],
    )
    .expect("insert coverage capture");

    // Insert line coverage hits (lines 42-46: 42,43,44 covered; 45,46 uncovered)
    for (line, covered) in [(42, 1), (43, 1), (44, 1), (45, 0), (46, 0)] {
        conn.execute(
            "INSERT INTO coverage_hits (
                capture_id, production_symbol_id, file_path, line, branch_id, covered, hit_count
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                "capture-1",
                "sym::create_user",
                "src/user/service.rs",
                line,
                -1,
                covered,
                if covered == 1 { 3 } else { 0 },
            ],
        )
        .expect("insert line coverage hit");
    }

    // Insert branch coverage hits
    for (line, branch_id, covered, hit_count) in [(48, 0, 1, 3), (48, 1, 0, 0)] {
        conn.execute(
            "INSERT INTO coverage_hits (
                capture_id, production_symbol_id, file_path, line, branch_id, covered, hit_count
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                "capture-1",
                "sym::create_user",
                "src/user/service.rs",
                line,
                branch_id,
                covered,
                hit_count,
            ],
        )
        .expect("insert branch coverage hit");
    }
    let line_rows_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM coverage_hits WHERE production_symbol_id = ?1 AND branch_id = -1",
            rusqlite::params!["sym::create_user"],
            |row| row.get(0),
        )
        .expect("count line coverage rows");
    assert_eq!(line_rows_count, 5);

    let parsed = parse_devql_query(
        r#"repo("temp2")->file("src/user/service.rs")->artefacts(kind:"function")->coverage()->limit(10)"#,
    )
    .expect("parse query");
    let base_rows = execute_devql_query(&cfg, &parsed, &events_cfg, Some(&relational))
        .await
        .expect("execute base pipeline");
    let rows = execute_registered_stages(&cfg, &parsed, base_rows)
        .await
        .expect("execute coverage pipeline");

    assert_eq!(rows.len(), 1);

    // Verify artefact
    let artefact = rows[0].get("artefact").expect("should have artefact");
    assert_eq!(
        artefact.get("artefact_id").and_then(Value::as_str),
        Some("artefact::create_user")
    );
    assert_eq!(artefact.get("start_line").and_then(Value::as_i64), Some(42));
    assert_eq!(artefact.get("end_line").and_then(Value::as_i64), Some(89));

    // Verify coverage
    let coverage = rows[0].get("coverage").expect("should have coverage");
    assert_eq!(
        coverage.get("coverage_source").and_then(Value::as_str),
        Some("lcov")
    );
    assert!(
        coverage
            .get("line_data_available")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    );
    assert!(
        coverage
            .get("branch_data_available")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    );

    let line_pct = coverage
        .get("line_coverage_pct")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    assert!(
        (line_pct - 60.0).abs() < 0.1,
        "expected ~60% line coverage, got {line_pct}"
    );

    let branch_pct = coverage
        .get("branch_coverage_pct")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    assert!(
        (branch_pct - 50.0).abs() < 0.1,
        "expected ~50% branch coverage, got {branch_pct}"
    );

    let uncovered = coverage
        .get("uncovered_lines")
        .and_then(Value::as_array)
        .expect("should have uncovered_lines");
    assert_eq!(uncovered.len(), 2);
    assert_eq!(uncovered[0].as_i64(), Some(45));
    assert_eq!(uncovered[1].as_i64(), Some(46));

    let branches = coverage
        .get("branches")
        .and_then(Value::as_array)
        .expect("should have branches");
    assert_eq!(branches.len(), 2);
    assert!(
        branches[0]
            .get("covered")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    );
    assert!(
        !branches[1]
            .get("covered")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    );

    // Verify summary
    let summary = rows[0].get("summary").expect("should have summary");
    assert_eq!(
        summary.get("uncovered_line_count").and_then(Value::as_i64),
        Some(2)
    );
    assert_eq!(
        summary
            .get("uncovered_branch_count")
            .and_then(Value::as_i64),
        Some(1)
    );
    assert_eq!(
        summary.get("diagnostic_count").and_then(Value::as_i64),
        Some(0)
    );
}
