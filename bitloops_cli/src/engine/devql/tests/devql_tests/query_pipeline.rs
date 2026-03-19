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
    assert_eq!(parsed.deps.kind, Some(DepsKind::Calls));
    assert_eq!(parsed.deps.direction, DepsDirection::Both);
    assert!(!parsed.deps.include_unresolved);
    assert_eq!(parsed.limit, 25);
}

#[tokio::test]
async fn execute_devql_query_rejects_combining_deps_and_checkpoints_stage() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(r#"repo("temp2")->checkpoints()->artefacts()->deps()"#).unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();
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
        "extends",
        "implements",
        "exports",
    ] {
        let parsed = parse_devql_query(&format!(
            r#"repo("bitloops-cli")->artefacts(kind:"function")->deps(kind:"{kind}")->limit(5)"#
        ))
        .unwrap();

        assert_eq!(parsed.deps.kind, DepsKind::from_str(kind));
    }

    let legacy = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts(kind:"function")->deps(kind:"inherits")->limit(5)"#,
    )
    .unwrap();
    assert_eq!(legacy.deps.kind, Some(DepsKind::Extends));
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
    assert!(!sql.contains("a.revision_kind"));
    assert!(!sql.contains("a.revision_id"));
    assert!(!sql.contains("e.revision_kind"));
    assert!(!sql.contains("e.revision_id"));
    assert!(!sql.contains("current_scope"));
}

#[test]
fn build_postgres_deps_query_filters_temporary_revision_for_save_revision() {
    let cfg = test_cfg();
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->asOf(saveRevision:"temp:42")->artefacts(kind:"function")->deps(kind:"calls",direction:"both")->limit(10)"#,
    )
    .unwrap();

    let sql = build_postgres_deps_query(&cfg, &parsed, &cfg.repo.repo_id).unwrap();

    assert!(sql.contains("FROM artefact_edges_current e"));
    assert!(sql.contains("JOIN artefacts_current a ON a.artefact_id = e.from_artefact_id"));
    assert!(sql.contains("e.revision_kind = 'temporary'"));
    assert!(sql.contains("e.revision_id = 'temp:42'"));
    assert!(sql.contains("a.revision_kind = 'temporary'"));
    assert!(sql.contains("a.revision_id = 'temp:42'"));
    assert!(!sql.contains("FROM artefact_edges e"));
    assert!(!sql.contains("JOIN artefacts a ON a.artefact_id = e.from_artefact_id"));
}

#[tokio::test]
async fn build_relational_artefacts_query_includes_language_kind_and_symbol_fqn_filter() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(
        r#"repo("temp2")->file("hello_rust/src/main.rs")->artefacts(kind:"method",symbol_fqn:"hello_rust/src/main.rs::impl@1::handle_factorial")->limit(10)"#,
    )
    .unwrap();

    let sql = build_relational_artefacts_query(&cfg, &events_cfg, &parsed, None, &cfg.repo.repo_id)
        .await
        .unwrap();

    assert!(sql.contains("a.language_kind"));
    assert!(sql.contains("a.modifiers"));
    assert!(sql.contains("a.docstring"));
    assert!(sql.contains("a.symbol_fqn = 'hello_rust/src/main.rs::impl@1::handle_factorial'"));
    assert!(sql.contains("FROM artefacts_current a"));
}

#[tokio::test]
async fn execute_relational_pipeline_reads_artefacts_from_sqlite_relational_store() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let temp = tempdir().expect("tempdir");
    let sqlite_path = temp.path().join("relational.sqlite");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, symbol_id, artefact_id, commit_sha, blob_sha, path, language,
            canonical_kind, language_kind, symbol_fqn, start_line, end_line, start_byte,
            end_byte, modifiers, docstring, content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        rusqlite::params![
            cfg.repo.repo_id.as_str(),
            "sym::greet",
            "artefact::greet",
            "commit-1",
            "blob-1",
            "src/main.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/main.ts::greet",
            1,
            5,
            0,
            42,
            "[\"export\"]",
            "docs",
            "hash-1",
        ],
    )
    .expect("insert artefact row");

    let parsed = parse_devql_query(
        r#"repo("temp2")->file("src/main.ts")->artefacts(kind:"function")->limit(10)"#,
    )
    .expect("parse query");
    let rows = execute_relational_pipeline(&cfg, &events_cfg, &parsed, &relational)
        .await
        .expect("execute sqlite relational artefacts query");

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]["artefact_id"],
        Value::String("artefact::greet".to_string())
    );
    assert_eq!(rows[0]["path"], Value::String("src/main.ts".to_string()));
    assert!(rows[0]["modifiers"].is_array());
}

#[tokio::test]
async fn execute_relational_pipeline_reads_deps_from_sqlite_relational_store() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let temp = tempdir().expect("tempdir");
    let sqlite_path = temp.path().join("relational.sqlite");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, symbol_id, artefact_id, commit_sha, blob_sha, path, language,
            canonical_kind, language_kind, symbol_fqn, start_line, end_line, start_byte,
            end_byte, modifiers, content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        rusqlite::params![
            cfg.repo.repo_id.as_str(),
            "sym::caller",
            "artefact::caller",
            "commit-1",
            "blob-1",
            "src/caller.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/caller.ts::caller",
            1,
            5,
            0,
            42,
            "[]",
            "hash-caller",
        ],
    )
    .expect("insert caller artefact");
    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, symbol_id, artefact_id, commit_sha, blob_sha, path, language,
            canonical_kind, language_kind, symbol_fqn, start_line, end_line, start_byte,
            end_byte, modifiers, content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        rusqlite::params![
            cfg.repo.repo_id.as_str(),
            "sym::target",
            "artefact::target",
            "commit-1",
            "blob-2",
            "src/target.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/target.ts::target",
            1,
            3,
            0,
            24,
            "[]",
            "hash-target",
        ],
    )
    .expect("insert target artefact");
    conn.execute(
        "INSERT INTO artefact_edges_current (
            edge_id, repo_id, commit_sha, blob_sha, path, from_symbol_id, from_artefact_id,
            to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line,
            end_line, metadata
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            "edge-1",
            cfg.repo.repo_id.as_str(),
            "commit-1",
            "blob-1",
            "src/caller.ts",
            "sym::caller",
            "artefact::caller",
            "sym::target",
            "artefact::target",
            "src/target.ts::target",
            "calls",
            "typescript",
            2,
            2,
            "{\"resolution\":\"local\"}",
        ],
    )
    .expect("insert deps edge");

    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->deps(kind:"calls",direction:"out")->limit(10)"#,
    )
    .expect("parse query");
    let rows = execute_relational_pipeline(&cfg, &events_cfg, &parsed, &relational)
        .await
        .expect("execute sqlite relational deps query");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["edge_id"], Value::String("edge-1".to_string()));
    assert_eq!(rows[0]["edge_kind"], Value::String("calls".to_string()));
    assert_eq!(
        rows[0]["from_path"],
        Value::String("src/caller.ts".to_string())
    );
    assert_eq!(
        rows[0]["to_path"],
        Value::String("src/target.ts".to_string())
    );
    assert!(rows[0]["metadata"].is_object());
}

#[tokio::test]
async fn execute_relational_pipeline_reads_commit_asof_deps_from_historical_tables() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let temp = tempdir().expect("tempdir");
    let sqlite_path = temp.path().join("relational.sqlite");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    conn.execute(
        "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![cfg.repo.repo_id.as_str(), "commit-old", "src/caller.ts", "blob-old"],
    )
    .expect("insert file_state for old commit");
    conn.execute(
        "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![cfg.repo.repo_id.as_str(), "commit-new", "src/caller.ts", "blob-new"],
    )
    .expect("insert file_state for new commit");

    conn.execute(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
            language_kind, symbol_fqn, start_line, end_line, start_byte, end_byte, modifiers,
            content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            "artefact::caller-old",
            "sym::caller-old",
            cfg.repo.repo_id.as_str(),
            "blob-old",
            "src/caller.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/caller.ts::callerOld",
            1,
            5,
            0,
            50,
            "[]",
            "hash-caller-old",
        ],
    )
    .expect("insert historical caller old");
    conn.execute(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
            language_kind, symbol_fqn, start_line, end_line, start_byte, end_byte, modifiers,
            content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            "artefact::target-old",
            "sym::target-old",
            cfg.repo.repo_id.as_str(),
            "blob-target-old",
            "src/target-old.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/target-old.ts::targetOld",
            1,
            3,
            0,
            30,
            "[]",
            "hash-target-old",
        ],
    )
    .expect("insert historical target old");
    conn.execute(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
            language_kind, symbol_fqn, start_line, end_line, start_byte, end_byte, modifiers,
            content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            "artefact::caller-new",
            "sym::caller-new",
            cfg.repo.repo_id.as_str(),
            "blob-new",
            "src/caller.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/caller.ts::callerNew",
            1,
            5,
            0,
            50,
            "[]",
            "hash-caller-new",
        ],
    )
    .expect("insert historical caller new");
    conn.execute(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
            language_kind, symbol_fqn, start_line, end_line, start_byte, end_byte, modifiers,
            content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            "artefact::target-new",
            "sym::target-new",
            cfg.repo.repo_id.as_str(),
            "blob-target-new",
            "src/target-new.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/target-new.ts::targetNew",
            1,
            3,
            0,
            30,
            "[]",
            "hash-target-new",
        ],
    )
    .expect("insert historical target new");

    conn.execute(
        "INSERT INTO artefact_edges (
            edge_id, repo_id, blob_sha, from_artefact_id, to_artefact_id, to_symbol_ref,
            edge_kind, language, start_line, end_line, metadata
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            "edge-old",
            cfg.repo.repo_id.as_str(),
            "blob-old",
            "artefact::caller-old",
            "artefact::target-old",
            "src/target-old.ts::targetOld",
            "calls",
            "typescript",
            2,
            2,
            "{\"resolution\":\"local\"}",
        ],
    )
    .expect("insert historical old edge");
    conn.execute(
        "INSERT INTO artefact_edges (
            edge_id, repo_id, blob_sha, from_artefact_id, to_artefact_id, to_symbol_ref,
            edge_kind, language, start_line, end_line, metadata
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            "edge-new",
            cfg.repo.repo_id.as_str(),
            "blob-new",
            "artefact::caller-new",
            "artefact::target-new",
            "src/target-new.ts::targetNew",
            "calls",
            "typescript",
            3,
            3,
            "{\"resolution\":\"local\"}",
        ],
    )
    .expect("insert historical new edge");

    let parsed = parse_devql_query(
        r#"repo("temp2")->asOf(commit:"commit-old")->file("src/caller.ts")->artefacts(kind:"function")->deps(kind:"calls",direction:"out")->limit(10)"#,
    )
    .expect("parse query");
    let rows = execute_relational_pipeline(&cfg, &events_cfg, &parsed, &relational)
        .await
        .expect("execute commit asOf deps query");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["edge_id"], Value::String("edge-old".to_string()));
    assert_eq!(
        rows[0]["from_path"],
        Value::String("src/caller.ts".to_string())
    );
    assert_eq!(
        rows[0]["to_path"],
        Value::String("src/target-old.ts".to_string())
    );
}

#[tokio::test]
async fn execute_relational_pipeline_reads_save_revision_asof_deps_from_current_tables() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let temp = tempdir().expect("tempdir");
    let sqlite_path = temp.path().join("relational.sqlite");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, symbol_id, artefact_id, commit_sha, revision_kind, revision_id, blob_sha,
            path, language, canonical_kind, language_kind, symbol_fqn, start_line, end_line,
            start_byte, end_byte, modifiers, content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        rusqlite::params![
            cfg.repo.repo_id.as_str(),
            "sym::caller-temp",
            "artefact::caller-temp",
            "temp:42",
            "temporary",
            "temp:42",
            "blob-temp",
            "src/caller.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/caller.ts::callerTemp",
            1,
            5,
            0,
            42,
            "[]",
            "hash-caller-temp",
        ],
    )
    .expect("insert temporary caller");
    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, symbol_id, artefact_id, commit_sha, revision_kind, revision_id, blob_sha,
            path, language, canonical_kind, language_kind, symbol_fqn, start_line, end_line,
            start_byte, end_byte, modifiers, content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        rusqlite::params![
            cfg.repo.repo_id.as_str(),
            "sym::target-temp",
            "artefact::target-temp",
            "temp:42",
            "temporary",
            "temp:42",
            "blob-target-temp",
            "src/target.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/target.ts::targetTemp",
            1,
            3,
            0,
            24,
            "[]",
            "hash-target-temp",
        ],
    )
    .expect("insert temporary target");
    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, symbol_id, artefact_id, commit_sha, revision_kind, revision_id, blob_sha,
            path, language, canonical_kind, language_kind, symbol_fqn, start_line, end_line,
            start_byte, end_byte, modifiers, content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        rusqlite::params![
            cfg.repo.repo_id.as_str(),
            "sym::caller-commit",
            "artefact::caller-commit",
            "commit-1",
            "commit",
            "commit-1",
            "blob-commit",
            "src/caller.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/caller.ts::callerCommit",
            1,
            5,
            0,
            42,
            "[]",
            "hash-caller-commit",
        ],
    )
    .expect("insert committed caller");
    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, symbol_id, artefact_id, commit_sha, revision_kind, revision_id, blob_sha,
            path, language, canonical_kind, language_kind, symbol_fqn, start_line, end_line,
            start_byte, end_byte, modifiers, content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        rusqlite::params![
            cfg.repo.repo_id.as_str(),
            "sym::target-commit",
            "artefact::target-commit",
            "commit-1",
            "commit",
            "commit-1",
            "blob-target-commit",
            "src/target.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/target.ts::targetCommit",
            1,
            3,
            0,
            24,
            "[]",
            "hash-target-commit",
        ],
    )
    .expect("insert committed target");

    conn.execute(
        "INSERT INTO artefact_edges_current (
            edge_id, repo_id, commit_sha, revision_kind, revision_id, blob_sha, path,
            from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref,
            edge_kind, language, start_line, end_line, metadata
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        rusqlite::params![
            "edge-temp",
            cfg.repo.repo_id.as_str(),
            "temp:42",
            "temporary",
            "temp:42",
            "blob-temp",
            "src/caller.ts",
            "sym::caller-temp",
            "artefact::caller-temp",
            "sym::target-temp",
            "artefact::target-temp",
            "src/target.ts::targetTemp",
            "calls",
            "typescript",
            2,
            2,
            "{\"resolution\":\"local\"}",
        ],
    )
    .expect("insert temporary edge");
    conn.execute(
        "INSERT INTO artefact_edges_current (
            edge_id, repo_id, commit_sha, revision_kind, revision_id, blob_sha, path,
            from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref,
            edge_kind, language, start_line, end_line, metadata
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        rusqlite::params![
            "edge-commit",
            cfg.repo.repo_id.as_str(),
            "commit-1",
            "commit",
            "commit-1",
            "blob-commit",
            "src/caller.ts",
            "sym::caller-commit",
            "artefact::caller-commit",
            "sym::target-commit",
            "artefact::target-commit",
            "src/target.ts::targetCommit",
            "calls",
            "typescript",
            4,
            4,
            "{\"resolution\":\"local\"}",
        ],
    )
    .expect("insert committed edge");

    let parsed = parse_devql_query(
        r#"repo("temp2")->asOf(saveRevision:"temp:42")->artefacts(kind:"function")->deps(kind:"calls",direction:"out")->limit(10)"#,
    )
    .expect("parse query");
    let rows = execute_relational_pipeline(&cfg, &events_cfg, &parsed, &relational)
        .await
        .expect("execute saveRevision asOf deps query");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["edge_id"], Value::String("edge-temp".to_string()));
    assert_eq!(
        rows[0]["from_path"],
        Value::String("src/caller.ts".to_string())
    );
    assert_eq!(
        rows[0]["to_path"],
        Value::String("src/target.ts".to_string())
    );
}

#[tokio::test]
async fn execute_relational_pipeline_reads_inbound_deps_for_blast_radius_queries() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let temp = tempdir().expect("tempdir");
    let sqlite_path = temp.path().join("relational.sqlite");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, symbol_id, artefact_id, commit_sha, blob_sha, path, language,
            canonical_kind, language_kind, symbol_fqn, start_line, end_line, start_byte,
            end_byte, modifiers, content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        rusqlite::params![
            cfg.repo.repo_id.as_str(),
            "sym::target",
            "artefact::target",
            "commit-1",
            "blob-target",
            "src/target.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/target.ts::target",
            1,
            3,
            0,
            30,
            "[]",
            "hash-target",
        ],
    )
    .expect("insert target artefact");
    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, symbol_id, artefact_id, commit_sha, blob_sha, path, language,
            canonical_kind, language_kind, symbol_fqn, start_line, end_line, start_byte,
            end_byte, modifiers, content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        rusqlite::params![
            cfg.repo.repo_id.as_str(),
            "sym::caller_a",
            "artefact::caller_a",
            "commit-1",
            "blob-caller-a",
            "src/caller-a.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/caller-a.ts::callerA",
            1,
            4,
            0,
            40,
            "[]",
            "hash-caller-a",
        ],
    )
    .expect("insert caller A artefact");
    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, symbol_id, artefact_id, commit_sha, blob_sha, path, language,
            canonical_kind, language_kind, symbol_fqn, start_line, end_line, start_byte,
            end_byte, modifiers, content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        rusqlite::params![
            cfg.repo.repo_id.as_str(),
            "sym::caller_b",
            "artefact::caller_b",
            "commit-1",
            "blob-caller-b",
            "src/caller-b.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/caller-b.ts::callerB",
            1,
            4,
            0,
            40,
            "[]",
            "hash-caller-b",
        ],
    )
    .expect("insert caller B artefact");

    conn.execute(
        "INSERT INTO artefact_edges_current (
            edge_id, repo_id, commit_sha, blob_sha, path, from_symbol_id, from_artefact_id,
            to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line,
            end_line, metadata
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            "edge-call-a",
            cfg.repo.repo_id.as_str(),
            "commit-1",
            "blob-caller-a",
            "src/caller-a.ts",
            "sym::caller_a",
            "artefact::caller_a",
            "sym::target",
            "artefact::target",
            "src/target.ts::target",
            "calls",
            "typescript",
            2,
            2,
            "{\"resolution\":\"local\"}",
        ],
    )
    .expect("insert edge caller A -> target");
    conn.execute(
        "INSERT INTO artefact_edges_current (
            edge_id, repo_id, commit_sha, blob_sha, path, from_symbol_id, from_artefact_id,
            to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line,
            end_line, metadata
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            "edge-call-b",
            cfg.repo.repo_id.as_str(),
            "commit-1",
            "blob-caller-b",
            "src/caller-b.ts",
            "sym::caller_b",
            "artefact::caller_b",
            "sym::target",
            "artefact::target",
            "src/target.ts::target",
            "calls",
            "typescript",
            3,
            3,
            "{\"resolution\":\"local\"}",
        ],
    )
    .expect("insert edge caller B -> target");

    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(symbol_fqn:"src/target.ts::target")->deps(kind:"calls",direction:"in")->limit(10)"#,
    )
    .expect("parse query");
    let rows = execute_relational_pipeline(&cfg, &events_cfg, &parsed, &relational)
        .await
        .expect("execute inbound deps query");

    assert_eq!(rows.len(), 2);
    let mut edge_ids = rows
        .iter()
        .filter_map(|row| row["edge_id"].as_str())
        .map(str::to_string)
        .collect::<Vec<_>>();
    edge_ids.sort();
    assert_eq!(
        edge_ids,
        vec!["edge-call-a".to_string(), "edge-call-b".to_string()]
    );
    for row in rows {
        assert_eq!(
            row["to_symbol_fqn"],
            Value::String("src/target.ts::target".to_string())
        );
    }
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
    let err = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts()->deps(kind:"calls",direction:"sideways")->limit(5)"#,
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("deps(direction:...) must be one of: out, in, both")
    );
}

#[test]
fn build_postgres_deps_query_rejects_invalid_kind() {
    let err =
        parse_devql_query(r#"repo("bitloops-cli")->artefacts()->deps(kind:"surprise")->limit(5)"#)
            .unwrap_err();
    assert!(err.to_string().contains(
        "deps(kind:...) must be one of: imports, calls, references, extends, implements, exports"
    ));
}

#[tokio::test]
async fn execute_devql_query_rejects_chat_history_without_artefacts_stage() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(r#"repo("temp2")->chatHistory()->limit(1)"#).unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("chatHistory() requires an artefacts() stage")
    );
}

#[tokio::test]
async fn execute_devql_query_rejects_combining_checkpoints_and_artefacts_stage() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed =
        parse_devql_query(r#"repo("temp2")->checkpoints()->artefacts(agent:"claude-code")"#)
            .unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("MVP limitation: telemetry/checkpoints stages cannot be combined")
    );
}

#[tokio::test]
async fn execute_devql_query_rejects_combining_deps_and_chat_history_stage() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->deps(kind:"calls")->chatHistory()->limit(1)"#,
    )
    .unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("deps() cannot be combined with chatHistory()")
    );
}
