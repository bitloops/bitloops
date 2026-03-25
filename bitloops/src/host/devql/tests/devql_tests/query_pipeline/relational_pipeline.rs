use super::super::*;

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
        rusqlite::params![
            cfg.repo.repo_id.as_str(),
            "commit-old",
            "src/caller.ts",
            "blob-old"
        ],
    )
    .expect("insert file_state for old commit");
    conn.execute(
        "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            cfg.repo.repo_id.as_str(),
            "commit-new",
            "src/caller.ts",
            "blob-new"
        ],
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
    let shared_blob = git_ok(
        dir.path(),
        &["rev-parse", &format!("{commit_sha}:src/shared-a.ts")],
    );
    assert_eq!(
        shared_blob,
        git_ok(
            dir.path(),
            &["rev-parse", &format!("{commit_sha}:src/shared-b.ts")]
        )
    );

    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
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
    let shared_blob = git_ok(
        dir.path(),
        &["rev-parse", &format!("{commit_sha}:src/shared-a.ts")],
    );
    assert_eq!(
        shared_blob,
        git_ok(
            dir.path(),
            &["rev-parse", &format!("{commit_sha}:src/shared-b.ts")]
        )
    );

    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
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
