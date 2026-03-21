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
async fn execute_relational_pipeline_scopes_commit_asof_artefacts_by_path_when_blob_is_shared() {
    let dir = TempDir::new().expect("temp dir");
    init_test_repo(
        dir.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    std::fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    std::fs::write(
        dir.path().join("src/shared-a.ts"),
        "export function shared() {\n  return 1;\n}\n",
    )
    .expect("write shared-a");
    std::fs::write(
        dir.path().join("src/shared-b.ts"),
        "export function shared() {\n  return 1;\n}\n",
    )
    .expect("write shared-b");
    git_ok(dir.path(), &["add", "."]);
    git_ok(dir.path(), &["commit", "-m", "add shared files"]);

    let commit_sha = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let shared_blob = git_ok(dir.path(), &["rev-parse", &format!("{commit_sha}:src/shared-a.ts")]);
    assert_eq!(
        shared_blob,
        git_ok(dir.path(), &["rev-parse", &format!("{commit_sha}:src/shared-b.ts")])
    );

    let repo = crate::engine::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let mut cfg = test_cfg();
    cfg.repo_root = dir.path().to_path_buf();
    cfg.repo = repo;

    let events_cfg = default_events_cfg();
    let sqlite_path = dir.path().join("relational.sqlite");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");

    conn.execute(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
            language_kind, symbol_fqn, start_line, end_line, start_byte, end_byte, modifiers,
            content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            "artefact::shared-a",
            "sym::shared-a",
            cfg.repo.repo_id.as_str(),
            shared_blob.as_str(),
            "src/shared-a.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/shared-a.ts::shared",
            1,
            3,
            0,
            40,
            "[]",
            "hash-shared-a",
        ],
    )
    .expect("insert shared-a artefact");
    conn.execute(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
            language_kind, symbol_fqn, start_line, end_line, start_byte, end_byte, modifiers,
            content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            "artefact::shared-b",
            "sym::shared-b",
            cfg.repo.repo_id.as_str(),
            shared_blob.as_str(),
            "src/shared-b.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/shared-b.ts::shared",
            1,
            3,
            0,
            40,
            "[]",
            "hash-shared-b",
        ],
    )
    .expect("insert shared-b artefact");

    let parsed = parse_devql_query(&format!(
        r#"asOf(commit:"{commit_sha}")->file("src/shared-a.ts")->artefacts(kind:"function")->limit(10)"#
    ))
    .expect("parse query");
    let rows = execute_relational_pipeline(&cfg, &events_cfg, &parsed, &relational)
        .await
        .expect("execute commit asOf artefacts query");

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]["path"],
        Value::String("src/shared-a.ts".to_string())
    );
}

#[tokio::test]
async fn execute_relational_pipeline_scopes_commit_asof_deps_by_path_when_blob_is_shared() {
    let dir = TempDir::new().expect("temp dir");
    init_test_repo(
        dir.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    std::fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    std::fs::write(
        dir.path().join("src/shared-a.ts"),
        "export function shared() {\n  return 1;\n}\n",
    )
    .expect("write shared-a");
    std::fs::write(
        dir.path().join("src/shared-b.ts"),
        "export function shared() {\n  return 1;\n}\n",
    )
    .expect("write shared-b");
    git_ok(dir.path(), &["add", "."]);
    git_ok(dir.path(), &["commit", "-m", "add shared files"]);

    let commit_sha = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let shared_blob = git_ok(dir.path(), &["rev-parse", &format!("{commit_sha}:src/shared-a.ts")]);
    assert_eq!(
        shared_blob,
        git_ok(dir.path(), &["rev-parse", &format!("{commit_sha}:src/shared-b.ts")])
    );

    let repo = crate::engine::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let mut cfg = test_cfg();
    cfg.repo_root = dir.path().to_path_buf();
    cfg.repo = repo;

    let events_cfg = default_events_cfg();
    let sqlite_path = dir.path().join("relational.sqlite");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");

    conn.execute(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
            language_kind, symbol_fqn, start_line, end_line, start_byte, end_byte, modifiers,
            content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            "artefact::shared-a",
            "sym::shared-a",
            cfg.repo.repo_id.as_str(),
            shared_blob.as_str(),
            "src/shared-a.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/shared-a.ts::shared",
            1,
            3,
            0,
            40,
            "[]",
            "hash-shared-a",
        ],
    )
    .expect("insert shared-a artefact");
    conn.execute(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
            language_kind, symbol_fqn, start_line, end_line, start_byte, end_byte, modifiers,
            content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            "artefact::shared-b",
            "sym::shared-b",
            cfg.repo.repo_id.as_str(),
            shared_blob.as_str(),
            "src/shared-b.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/shared-b.ts::shared",
            1,
            3,
            0,
            40,
            "[]",
            "hash-shared-b",
        ],
    )
    .expect("insert shared-b artefact");
    conn.execute(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
            language_kind, symbol_fqn, start_line, end_line, start_byte, end_byte, modifiers,
            content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            "artefact::target-a",
            "sym::target-a",
            cfg.repo.repo_id.as_str(),
            "blob-target-a",
            "src/target-a.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/target-a.ts::target",
            1,
            3,
            0,
            30,
            "[]",
            "hash-target-a",
        ],
    )
    .expect("insert target-a artefact");
    conn.execute(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
            language_kind, symbol_fqn, start_line, end_line, start_byte, end_byte, modifiers,
            content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            "artefact::target-b",
            "sym::target-b",
            cfg.repo.repo_id.as_str(),
            "blob-target-b",
            "src/target-b.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/target-b.ts::target",
            1,
            3,
            0,
            30,
            "[]",
            "hash-target-b",
        ],
    )
    .expect("insert target-b artefact");

    conn.execute(
        "INSERT INTO artefact_edges (
            edge_id, repo_id, blob_sha, from_artefact_id, to_artefact_id, to_symbol_ref,
            edge_kind, language, start_line, end_line, metadata
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            "edge-a",
            cfg.repo.repo_id.as_str(),
            shared_blob.as_str(),
            "artefact::shared-a",
            "artefact::target-a",
            "src/target-a.ts::target",
            "calls",
            "typescript",
            2,
            2,
            "{\"resolution\":\"local\"}",
        ],
    )
    .expect("insert edge-a");
    conn.execute(
        "INSERT INTO artefact_edges (
            edge_id, repo_id, blob_sha, from_artefact_id, to_artefact_id, to_symbol_ref,
            edge_kind, language, start_line, end_line, metadata
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            "edge-b",
            cfg.repo.repo_id.as_str(),
            shared_blob.as_str(),
            "artefact::shared-b",
            "artefact::target-b",
            "src/target-b.ts::target",
            "calls",
            "typescript",
            2,
            2,
            "{\"resolution\":\"local\"}",
        ],
    )
    .expect("insert edge-b");

    let parsed = parse_devql_query(&format!(
        r#"asOf(commit:"{commit_sha}")->file("src/shared-a.ts")->artefacts(kind:"function")->deps(kind:"calls",direction:"out")->limit(10)"#
    ))
    .expect("parse query");
    let rows = execute_relational_pipeline(&cfg, &events_cfg, &parsed, &relational)
        .await
        .expect("execute commit asOf deps query");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["edge_id"], Value::String("edge-a".to_string()));
    assert_eq!(
        rows[0]["from_path"],
        Value::String("src/shared-a.ts".to_string())
    );
    assert_eq!(
        rows[0]["to_path"],
        Value::String("src/target-a.ts".to_string())
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

#[test]
fn parse_devql_tests_stage_basic() {
    let parsed = parse_devql_query(
        r#"repo("r")->file("src/lib.rs")->artefacts(kind:"function")->tests()"#,
    )
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
fn parse_devql_internal_core_test_links_stage_with_args() {
    let parsed = parse_devql_query(
        r#"repo("r")->__core_test_links(artefact_id:"artefact::a_1",min_confidence:0.5,linkage_source:"static_analysis")->limit(7)"#,
    )
    .unwrap();

    assert!(parsed.has_test_harness_core_test_links_stage);
    assert_eq!(
        parsed.test_harness_core_test_links.artefact_id.as_deref(),
        Some("artefact::a_1")
    );
    assert_eq!(parsed.test_harness_core_test_links.min_confidence, Some(0.5));
    assert_eq!(
        parsed.test_harness_core_test_links.linkage_source.as_deref(),
        Some("static_analysis")
    );
    assert_eq!(parsed.limit, 7);
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
async fn execute_devql_query_rejects_internal_core_stage_without_artefact_id() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(r#"repo("temp2")->__core_line_coverage()->limit(1)"#).unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("__core_line_coverage() requires artefact_id")
    );
}

#[tokio::test]
async fn execute_devql_query_rejects_tests_with_deps() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->deps(kind:"calls")->tests()->limit(1)"#,
    )
    .unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("tests() cannot be combined with deps()")
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
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let mut cfg = test_cfg();
    cfg.repo_root = repo_root;
    let events_cfg = default_events_cfg();
    let sqlite_path = temp.path().join("relational.sqlite");
    if let Some(parent) = sqlite_path.parent() {
        std::fs::create_dir_all(parent).expect("create relational parent dir");
    }
    let config_dir = cfg.repo_root.join(".bitloops");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("config.json"),
        serde_json::to_vec_pretty(&json!({
            "stores": {
                "relational": {
                    "provider": "sqlite",
                    "sqlite_path": sqlite_path.to_string_lossy()
                }
            }
        }))
        .expect("serialise config"),
    )
    .expect("write config");
    let host_sqlite_path = crate::config::resolve_store_backend_config_for_repo(&cfg.repo_root)
        .expect("resolve backend config")
        .relational
        .resolve_sqlite_db_path()
        .expect("resolve host sqlite path");
    assert_eq!(host_sqlite_path, sqlite_path);
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");

    // Create test harness tables (not part of the DevQL relational schema)
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS test_suites (
            suite_id TEXT PRIMARY KEY,
            repo_id TEXT NOT NULL,
            commit_sha TEXT NOT NULL,
            language TEXT NOT NULL,
            path TEXT NOT NULL,
            name TEXT NOT NULL,
            symbol_fqn TEXT,
            start_line BIGINT NOT NULL,
            end_line BIGINT NOT NULL,
            start_byte BIGINT,
            end_byte BIGINT,
            signature TEXT,
            discovery_source TEXT NOT NULL,
            created_at TEXT DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS test_scenarios (
            scenario_id TEXT PRIMARY KEY,
            suite_id TEXT REFERENCES test_suites(suite_id) ON DELETE CASCADE,
            repo_id TEXT NOT NULL,
            commit_sha TEXT NOT NULL,
            language TEXT NOT NULL,
            path TEXT NOT NULL,
            name TEXT NOT NULL,
            symbol_fqn TEXT,
            start_line BIGINT NOT NULL,
            end_line BIGINT NOT NULL,
            start_byte BIGINT,
            end_byte BIGINT,
            signature TEXT,
            discovery_source TEXT NOT NULL,
            created_at TEXT DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS test_links (
            test_link_id TEXT PRIMARY KEY,
            repo_id TEXT NOT NULL,
            commit_sha TEXT NOT NULL,
            test_scenario_id TEXT NOT NULL REFERENCES test_scenarios(scenario_id) ON DELETE CASCADE,
            production_artefact_id TEXT NOT NULL,
            production_symbol_id TEXT,
            link_source TEXT NOT NULL DEFAULT 'static_analysis',
            evidence_json TEXT DEFAULT '{}',
            confidence REAL NOT NULL DEFAULT 0.6,
            linkage_status TEXT NOT NULL DEFAULT 'resolved',
            created_at TEXT DEFAULT (datetime('now'))
        );
        "#,
    )
    .expect("create test harness tables");

    // Insert a production artefact
    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, symbol_id, artefact_id, commit_sha, blob_sha, path, language,
            canonical_kind, language_kind, symbol_fqn, start_line, end_line, start_byte,
            end_byte, modifiers, content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        rusqlite::params![
            cfg.repo.repo_id.as_str(),
            "sym::create_user",
            "artefact::create_user",
            "commit-1",
            "blob-1",
            "src/user/service.rs",
            "rust",
            "function",
            "function_item",
            "src/user/service.rs::create_user",
            1,
            3,
            0,
            42,
            "[]",
            "hash-1",
        ],
    )
    .expect("insert production artefact");

    // Insert test harness data
    conn.execute(
        "INSERT INTO test_suites (
            suite_id, repo_id, commit_sha, language, path, name,
            start_line, end_line, discovery_source
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            "suite::tests",
            cfg.repo.repo_id.as_str(),
            "commit-1",
            "rust",
            "src/user/service_tests.rs",
            "tests",
            1,
            10,
            "source",
        ],
    )
    .expect("insert test suite");

    conn.execute(
        "INSERT INTO test_scenarios (
            scenario_id, suite_id, repo_id, commit_sha, language, path, name,
            start_line, end_line, discovery_source
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            "scenario::test_create_user",
            "suite::tests",
            cfg.repo.repo_id.as_str(),
            "commit-1",
            "rust",
            "src/user/service_tests.rs",
            "test_create_user",
            5,
            8,
            "source",
        ],
    )
    .expect("insert test scenario");

    conn.execute(
        "INSERT INTO test_links (
            test_link_id, repo_id, commit_sha, test_scenario_id, production_artefact_id,
            link_source, confidence, linkage_status
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            "link::1",
            cfg.repo.repo_id.as_str(),
            "commit-1",
            "scenario::test_create_user",
            "artefact::create_user",
            "static_analysis",
            0.6,
            "resolved",
        ],
    )
    .expect("insert test link");
    let covering_rows_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM test_links WHERE repo_id = ?1",
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
        covering_tests[0]
            .get("test_name")
            .and_then(Value::as_str),
        Some("test_create_user")
    );

    let summary = rows[0].get("summary").expect("should have summary");
    assert_eq!(
        summary
            .get("total_covering_tests")
            .and_then(Value::as_i64),
        Some(1)
    );
}

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
        r#"repo("temp2")->artefacts(kind:"function")->deps(kind:"calls")->coverage()->limit(1)"#,
    )
    .unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("coverage() cannot be combined with deps()")
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
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let mut cfg = test_cfg();
    cfg.repo_root = repo_root;
    let events_cfg = default_events_cfg();
    let sqlite_path = temp.path().join("relational.sqlite");
    if let Some(parent) = sqlite_path.parent() {
        std::fs::create_dir_all(parent).expect("create relational parent dir");
    }
    let config_dir = cfg.repo_root.join(".bitloops");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("config.json"),
        serde_json::to_vec_pretty(&json!({
            "stores": {
                "relational": {
                    "provider": "sqlite",
                    "sqlite_path": sqlite_path.to_string_lossy()
                }
            }
        }))
        .expect("serialise config"),
    )
    .expect("write config");
    let host_sqlite_path = crate::config::resolve_store_backend_config_for_repo(&cfg.repo_root)
        .expect("resolve backend config")
        .relational
        .resolve_sqlite_db_path()
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
            subject_test_scenario_id TEXT,
            line_truth INTEGER NOT NULL DEFAULT 1,
            branch_truth INTEGER NOT NULL DEFAULT 0,
            captured_at TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'complete',
            metadata_json TEXT
        );
        CREATE TABLE IF NOT EXISTS coverage_hits (
            capture_id TEXT NOT NULL REFERENCES coverage_captures(capture_id) ON DELETE CASCADE,
            production_artefact_id TEXT NOT NULL,
            file_path TEXT NOT NULL,
            line INTEGER NOT NULL,
            branch_id INTEGER NOT NULL DEFAULT -1,
            covered INTEGER NOT NULL,
            hit_count INTEGER DEFAULT 0,
            PRIMARY KEY (capture_id, production_artefact_id, line, branch_id)
        );
        "#,
    )
    .expect("create coverage tables");

    // Insert a production artefact
    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, symbol_id, artefact_id, commit_sha, blob_sha, path, language,
            canonical_kind, language_kind, symbol_fqn, start_line, end_line, start_byte,
            end_byte, modifiers, content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        rusqlite::params![
            cfg.repo.repo_id.as_str(),
            "sym::create_user",
            "artefact::create_user",
            "commit-1",
            "blob-1",
            "src/user/service.rs",
            "rust",
            "function",
            "function_item",
            "src/user/service.rs::create_user",
            42,
            89,
            0,
            500,
            "[]",
            "hash-1",
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
                capture_id, production_artefact_id, file_path, line, branch_id, covered, hit_count
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                "capture-1",
                "artefact::create_user",
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
    for (line, branch_id, covered, hit_count) in
        [(48, 0, 1, 3), (48, 1, 0, 0)]
    {
        conn.execute(
            "INSERT INTO coverage_hits (
                capture_id, production_artefact_id, file_path, line, branch_id, covered, hit_count
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                "capture-1",
                "artefact::create_user",
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
            "SELECT COUNT(*) FROM coverage_hits WHERE production_artefact_id = ?1 AND branch_id = -1",
            rusqlite::params!["artefact::create_user"],
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
    assert_eq!(
        artefact.get("start_line").and_then(Value::as_i64),
        Some(42)
    );
    assert_eq!(
        artefact.get("end_line").and_then(Value::as_i64),
        Some(89)
    );

    // Verify coverage
    let coverage = rows[0].get("coverage").expect("should have coverage");
    assert_eq!(
        coverage.get("coverage_source").and_then(Value::as_str),
        Some("lcov")
    );
    assert!(coverage.get("line_data_available").and_then(Value::as_bool).unwrap_or(false));
    assert!(coverage.get("branch_data_available").and_then(Value::as_bool).unwrap_or(false));

    let line_pct = coverage
        .get("line_coverage_pct")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    assert!((line_pct - 60.0).abs() < 0.1, "expected ~60% line coverage, got {line_pct}");

    let branch_pct = coverage
        .get("branch_coverage_pct")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    assert!((branch_pct - 50.0).abs() < 0.1, "expected ~50% branch coverage, got {branch_pct}");

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
    assert!(branches[0].get("covered").and_then(Value::as_bool).unwrap_or(false));
    assert!(!branches[1].get("covered").and_then(Value::as_bool).unwrap_or(true));

    // Verify summary
    let summary = rows[0].get("summary").expect("should have summary");
    assert_eq!(
        summary.get("uncovered_line_count").and_then(Value::as_i64),
        Some(2)
    );
    assert_eq!(
        summary.get("uncovered_branch_count").and_then(Value::as_i64),
        Some(1)
    );
    assert_eq!(
        summary.get("diagnostic_count").and_then(Value::as_i64),
        Some(0)
    );
}
