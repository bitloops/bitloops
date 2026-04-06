use super::*;

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn artefact_edges_constraints_and_dedup_work_in_postgres() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let cfg = test_cfg();
    init_postgres_schema(&cfg, &client).await.unwrap();

    let artefact_id = deterministic_uuid("test-art-a");
    let symbol_id = deterministic_uuid("test-symbol-a");
    let upsert_artefact_sql = format!(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash) \
VALUES ('{}', '{}', '{}', 'blob1', 'src/a.ts', 'typescript', 'function', 'function_declaration', 'src/a.ts::a', NULL, 1, 3, 0, 10, 'function a() {{', 'h1') \
ON CONFLICT (artefact_id) DO NOTHING",
        esc_pg(&artefact_id),
        esc_pg(&symbol_id),
        esc_pg(&cfg.repo.repo_id)
    );
    postgres_exec(&client, &upsert_artefact_sql).await.unwrap();

    let invalid_target_sql = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, edge_kind, language) \
VALUES ('{}', '{}', 'blob1', '{}', 'calls', 'typescript')",
        esc_pg(&deterministic_uuid("invalid-target")),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&artefact_id),
    );
    assert!(postgres_exec(&client, &invalid_target_sql).await.is_err());

    let invalid_range_sql = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line) \
VALUES ('{}', '{}', 'blob1', '{}', 'x', 'calls', 'typescript', 4, 3)",
        esc_pg(&deterministic_uuid("invalid-range")),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&artefact_id),
    );
    assert!(postgres_exec(&client, &invalid_range_sql).await.is_err());

    let edge_id_a = deterministic_uuid("dedup-a");
    let edge_insert_a = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line) \
VALUES ('{}', '{}', 'blob1', '{}', 'src/a.ts::x', 'calls', 'typescript', 2, 2)",
        esc_pg(&edge_id_a),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&artefact_id),
    );
    postgres_exec(&client, &edge_insert_a).await.unwrap();

    let edge_id_b = deterministic_uuid("dedup-b");
    let edge_insert_b = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line) \
VALUES ('{}', '{}', 'blob1', '{}', 'src/a.ts::x', 'calls', 'typescript', 2, 2)",
        esc_pg(&edge_id_b),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&artefact_id),
    );
    assert!(postgres_exec(&client, &edge_insert_b).await.is_err());
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn artefact_rows_preserve_symbol_continuity_across_blobs_in_postgres() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let cfg = test_cfg();
    init_postgres_schema(&cfg, &client).await.unwrap();

    let symbol_id = deterministic_uuid("stable-function");
    let artefact_a = deterministic_uuid("stable-function-blob-a");
    let artefact_b = deterministic_uuid("stable-function-blob-b");

    let insert_a = format!(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash) \
VALUES ('{}', '{}', '{}', 'blob-a', 'src/a.ts', 'typescript', 'function', 'function_declaration', 'src/a.ts::greet', NULL, 1, 3, 0, 10, 'function greet() {{', 'h-a')",
        esc_pg(&artefact_a),
        esc_pg(&symbol_id),
        esc_pg(&cfg.repo.repo_id)
    );
    let insert_b = format!(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash) \
VALUES ('{}', '{}', '{}', 'blob-b', 'src/a.ts', 'typescript', 'function', 'function_declaration', 'src/a.ts::greet', NULL, 4, 6, 11, 24, 'function greet() {{', 'h-b')",
        esc_pg(&artefact_b),
        esc_pg(&symbol_id),
        esc_pg(&cfg.repo.repo_id)
    );

    postgres_exec(&client, &insert_a).await.unwrap();
    postgres_exec(&client, &insert_b).await.unwrap();

    let row = client
        .query_one(
            "SELECT COUNT(*) FROM artefacts WHERE repo_id = $1 AND symbol_id = $2",
            &[&cfg.repo.repo_id, &symbol_id],
        )
        .await
        .unwrap();
    let count: i64 = row.get(0);
    assert_eq!(
        count, 2,
        "expected both revisions to share the same symbol_id"
    );
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn init_postgres_schema_creates_checkpoint_file_snapshots_projection() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let cfg = test_cfg();
    init_postgres_schema(&cfg, &client).await.unwrap();
    init_postgres_schema(&cfg, &client).await.unwrap();

    let table_name: Option<String> = client
        .query_one(
            "SELECT to_regclass('public.checkpoint_file_snapshots')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        table_name.as_deref(),
        Some("checkpoint_file_snapshots"),
        "expected init_postgres_schema to create checkpoint_file_snapshots"
    );

    let index_rows = client
        .query(
            "SELECT indexname
             FROM pg_indexes
             WHERE schemaname = 'public' AND tablename = 'checkpoint_file_snapshots'
             ORDER BY indexname",
            &[],
        )
        .await
        .unwrap();
    let index_names: Vec<String> = index_rows.into_iter().map(|row| row.get(0)).collect();
    assert_eq!(
        index_names,
        vec![
            "checkpoint_file_snapshots_agent_time_idx".to_string(),
            "checkpoint_file_snapshots_checkpoint_idx".to_string(),
            "checkpoint_file_snapshots_commit_idx".to_string(),
            "checkpoint_file_snapshots_event_time_idx".to_string(),
            "checkpoint_file_snapshots_lookup_idx".to_string(),
            "checkpoint_file_snapshots_pkey".to_string(),
        ],
        "expected init_postgres_schema to create the projection indexes"
    );
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn init_postgres_schema_preserves_existing_sync_rows_on_repeated_runs() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let mut cfg = test_cfg();
    cfg.repo.repo_id = deterministic_uuid("repo://postgres-preserve-sync-rows");
    init_postgres_schema(&cfg, &client).await.unwrap();

    let insert_repo_sql = format!(
        "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
         VALUES ('{}', 'github', 'bitloops', 'preserve-sync-rows', 'main')
         ON CONFLICT (repo_id) DO NOTHING",
        esc_pg(&cfg.repo.repo_id)
    );
    postgres_exec(&client, &insert_repo_sql).await.unwrap();

    let insert_sync_state_sql = format!(
        "INSERT INTO repo_sync_state (
            repo_id, repo_root, active_branch, head_commit_sha, head_tree_sha,
            parser_version, extractor_version, last_sync_started_at,
            last_sync_completed_at, last_sync_status, last_sync_reason
         ) VALUES (
            '{}', '/tmp/repo', 'main', 'commit-1', 'tree-1',
            'parser-v1', 'extractor-v1', '2026-04-03T10:00:00Z',
            '2026-04-03T10:01:00Z', 'completed', 'baseline'
         )",
        esc_pg(&cfg.repo.repo_id)
    );
    postgres_exec(&client, &insert_sync_state_sql)
        .await
        .unwrap();

    let insert_current_file_state_sql = format!(
        "INSERT INTO current_file_state (
            repo_id, path, language, head_content_id, index_content_id,
            worktree_content_id, effective_content_id, effective_source,
            parser_version, extractor_version, exists_in_head,
            exists_in_index, exists_in_worktree, last_synced_at
         ) VALUES (
            '{}', 'src/lib.rs', 'rust', 'head-1', 'index-1',
            'worktree-1', 'effective-1', 'worktree',
            'parser-v1', 'extractor-v1', 1, 1, 1, '2026-04-03T10:02:00Z'
         )",
        esc_pg(&cfg.repo.repo_id)
    );
    postgres_exec(&client, &insert_current_file_state_sql)
        .await
        .unwrap();

    let insert_artefacts_current_sql = format!(
        "INSERT INTO artefacts_current (
            repo_id, path, content_id, symbol_id, artefact_id, language,
            canonical_kind, language_kind, symbol_fqn, parent_symbol_id,
            parent_artefact_id, start_line, end_line, start_byte, end_byte,
            signature, modifiers, docstring, updated_at
         ) VALUES (
            '{}', 'src/lib.rs', 'content-1', '{}', '{}', 'rust',
            'function', 'function_item', 'src/lib.rs::answer', NULL,
            NULL, 1, 3, 0, 24, 'fn answer() -> i32', '[]'::jsonb, NULL,
            '2026-04-03T10:03:00Z'
         )",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&deterministic_uuid("postgres-preserve-symbol")),
        esc_pg(&deterministic_uuid("postgres-preserve-artefact")),
    );
    postgres_exec(&client, &insert_artefacts_current_sql)
        .await
        .unwrap();

    init_postgres_schema(&cfg, &client).await.unwrap();

    for table in ["repo_sync_state", "current_file_state", "artefacts_current"] {
        let row = client
            .query_one(
                &format!("SELECT COUNT(*) FROM {table} WHERE repo_id = $1"),
                &[&cfg.repo.repo_id],
            )
            .await
            .unwrap();
        let count: i64 = row.get(0);
        assert_eq!(count, 1, "expected `{table}` rows to be preserved");
    }
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn rebuilding_legacy_remote_sync_schema_upgrades_paths_sync_to_repair() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let repo = TempDir::new().expect("temp dir");
    init_test_repo(
        repo.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    fs::create_dir_all(repo.path().join("src")).expect("create src dir");
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn answer() -> i32 {\n    42\n}\n",
    )
    .expect("write lib.rs");
    fs::write(
        repo.path().join("src/extra.rs"),
        "pub fn bonus() -> i32 {\n    7\n}\n",
    )
    .expect("write extra.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(
        repo.path(),
        &["commit", "-m", "Seed legacy Postgres sync repo"],
    );

    let repo_identity = resolve_repo_identity(repo.path()).expect("resolve repo identity");
    let mut cfg = DevqlConfig::from_roots(
        repo.path().to_path_buf(),
        repo.path().to_path_buf(),
        repo_identity,
    )
    .expect("build DevQL config");
    cfg.pg_dsn = Some(dsn.clone());
    cfg.repo.repo_id = deterministic_uuid("repo://postgres-legacy-path-upgrade");

    let relational = postgres_relational_store(&cfg, &dsn).await;
    init_sqlite_schema(&relational.local.path)
        .await
        .expect("initialise local sqlite schema");

    postgres_exec(&client, postgres_schema_sql()).await.unwrap();
    postgres_exec(&client, crate::host::devql::sync::schema::sync_schema_sql())
        .await
        .unwrap();

    postgres_exec(
        &client,
        r#"
DROP TABLE IF EXISTS current_file_state;
CREATE TABLE current_file_state (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    committed_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (repo_id, path)
);
"#,
    )
    .await
    .unwrap();

    let insert_legacy_current_state_sql = format!(
        "INSERT INTO current_file_state (
            repo_id, path, commit_sha, blob_sha, committed_at, updated_at
         ) VALUES (
            '{}', 'src/legacy.rs', 'legacy-commit', 'legacy-blob',
            '2026-04-03T09:00:00Z', '2026-04-03T09:00:00Z'
         )",
        esc_pg(&cfg.repo.repo_id)
    );
    postgres_exec(&client, &insert_legacy_current_state_sql)
        .await
        .unwrap();

    let schema_outcome = init_postgres_schema_for_sync_execution(&cfg, &client)
        .await
        .expect("prepare sync execution schema");
    assert!(
        schema_outcome.rebuilt_current_state,
        "expected legacy remote sync schema to be rebuilt"
    );

    let effective_mode = effective_sync_mode_after_schema_preparation(
        SyncMode::Paths(vec!["src/lib.rs".to_string()]),
        SyncExecutionSchemaOutcome {
            remote_current_state_rebuilt: schema_outcome.rebuilt_current_state,
        },
    );
    assert!(
        matches!(effective_mode, SyncMode::Repair),
        "expected paths sync to be upgraded to repair, got {effective_mode:?}"
    );

    let summary = execute_sync(&cfg, &relational, effective_mode)
        .await
        .expect("execute upgraded repair sync");
    assert_eq!(summary.mode, "repair");

    let remote_paths = client
        .query(
            "SELECT path
             FROM current_file_state
             WHERE repo_id = $1
             ORDER BY path",
            &[&cfg.repo.repo_id],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get::<_, String>(0))
        .collect::<Vec<_>>();
    assert_eq!(
        remote_paths,
        vec!["src/extra.rs".to_string(), "src/lib.rs".to_string()],
        "expected repaired remote current-state tables to include the whole workspace"
    );
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn current_snapshot_updates_lines_and_bytes_for_moved_js_symbol_while_history_is_preserved() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let mut cfg = test_cfg();
    cfg.pg_dsn = Some(dsn.clone());
    cfg.repo.repo_id = deterministic_uuid("repo://devql-current-snapshot-move");
    init_postgres_schema(&cfg, &client).await.unwrap();
    let relational = postgres_relational_store(&cfg, &dsn).await;

    let path = "src/current_snapshot_move.ts";
    let commit_old = "commit-old";
    let commit_new = "commit-new";
    let blob_old = "blob-old";
    let blob_new = "blob-new";
    let file_symbol_id = file_symbol_id(path);
    let function_symbol_id = deterministic_uuid("stable-greet-symbol");

    let file_old = FileArtefactRow {
        artefact_id: revision_artefact_id(&cfg.repo.repo_id, blob_old, &file_symbol_id),
        symbol_id: file_symbol_id.clone(),
        language: "typescript".to_string(),
        end_line: 4,
        end_byte: 48,
    };
    let file_new = FileArtefactRow {
        artefact_id: revision_artefact_id(&cfg.repo.repo_id, blob_new, &file_symbol_id),
        symbol_id: file_symbol_id.clone(),
        language: "typescript".to_string(),
        end_line: 9,
        end_byte: 112,
    };

    let old_record = PersistedArtefactRecord {
        symbol_id: function_symbol_id.clone(),
        artefact_id: revision_artefact_id(&cfg.repo.repo_id, blob_old, &function_symbol_id),
        canonical_kind: Some("function".to_string()),
        language_kind: "function_declaration".to_string(),
        symbol_fqn: format!("{path}::greet"),
        parent_symbol_id: Some(file_symbol_id.clone()),
        parent_artefact_id: Some(file_old.artefact_id.clone()),
        start_line: 1,
        end_line: 3,
        start_byte: 0,
        end_byte: 35,
        signature: Some("export function greet(name: string) {".to_string()),
        modifiers: vec![],
        docstring: None,
        content_hash: "hash-old".to_string(),
    };
    let new_record = PersistedArtefactRecord {
        symbol_id: function_symbol_id.clone(),
        artefact_id: revision_artefact_id(&cfg.repo.repo_id, blob_new, &function_symbol_id),
        canonical_kind: Some("function".to_string()),
        language_kind: "function_declaration".to_string(),
        symbol_fqn: format!("{path}::greet"),
        parent_symbol_id: Some(file_symbol_id.clone()),
        parent_artefact_id: Some(file_new.artefact_id.clone()),
        start_line: 6,
        end_line: 9,
        start_byte: 58,
        end_byte: 111,
        signature: Some("export function greet(name: string) {".to_string()),
        modifiers: vec![],
        docstring: None,
        content_hash: "hash-new".to_string(),
    };

    upsert_file_state_row(&cfg.repo.repo_id, &relational, commit_old, path, blob_old)
        .await
        .unwrap();
    upsert_file_state_row(&cfg.repo.repo_id, &relational, commit_new, path, blob_new)
        .await
        .unwrap();
    persist_historical_artefact(
        &cfg,
        &relational,
        path,
        blob_old,
        &file_old.language,
        &old_record,
    )
    .await
    .unwrap();
    persist_historical_artefact(
        &cfg,
        &relational,
        path,
        blob_new,
        &file_new.language,
        &new_record,
    )
    .await
    .unwrap();

    refresh_current_state_for_path(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: commit_old,
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Commit,
                id: commit_old,
                temp_checkpoint_id: None,
            },
            commit_unix: 100,
            path,
            blob_sha: blob_old,
        },
        &file_old,
        None,
        std::slice::from_ref(&old_record),
        vec![],
    )
    .await
    .unwrap();
    refresh_current_state_for_path(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: commit_new,
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Commit,
                id: commit_new,
                temp_checkpoint_id: None,
            },
            commit_unix: 200,
            path,
            blob_sha: blob_new,
        },
        &file_new,
        None,
        std::slice::from_ref(&new_record),
        vec![],
    )
    .await
    .unwrap();

    let current_row = client
        .query_one(
            "SELECT artefact_id, start_line, end_line, start_byte, end_byte FROM artefacts_current WHERE repo_id = $1 AND symbol_id = $2",
            &[&cfg.repo.repo_id, &function_symbol_id],
        )
        .await
        .unwrap();
    let current_artefact_id: String = current_row.get(0);
    let current_start_line: i32 = current_row.get(1);
    let current_end_line: i32 = current_row.get(2);
    let current_start_byte: i32 = current_row.get(3);
    let current_end_byte: i32 = current_row.get(4);
    assert_eq!(current_artefact_id, new_record.artefact_id);
    assert_eq!(current_start_line, 6);
    assert_eq!(current_end_line, 9);
    assert_eq!(current_start_byte, 58);
    assert_eq!(current_end_byte, 111);

    let historical_count = client
        .query_one(
            "SELECT COUNT(*) FROM artefacts WHERE repo_id = $1 AND symbol_id = $2",
            &[&cfg.repo.repo_id, &function_symbol_id],
        )
        .await
        .unwrap();
    let historical_count: i64 = historical_count.get(0);
    assert_eq!(historical_count, 2);
    assert_ne!(old_record.artefact_id, new_record.artefact_id);

    let current_parsed = parse_devql_query(&format!(
        r#"repo("temp2")->file("{path}")->artefacts(kind:"function")->limit(10)"#
    ))
    .unwrap();
    let current_rows =
        execute_relational_pipeline(&cfg, &default_events_cfg(), &current_parsed, &relational)
            .await
            .unwrap();
    assert_eq!(current_rows.len(), 1);
    assert_eq!(
        current_rows[0]["artefact_id"],
        Value::String(new_record.artefact_id.clone())
    );
    assert_eq!(current_rows[0]["start_line"], Value::from(6));
    assert_eq!(current_rows[0]["end_line"], Value::from(9));
    assert_eq!(current_rows[0]["start_byte"], Value::from(58));
    assert_eq!(current_rows[0]["end_byte"], Value::from(111));

    let historical_parsed = parse_devql_query(&format!(
        r#"repo("temp2")->asOf(commit:"{commit_old}")->file("{path}")->artefacts(kind:"function")->limit(10)"#
    ))
    .unwrap();
    let historical_rows =
        execute_relational_pipeline(&cfg, &default_events_cfg(), &historical_parsed, &relational)
            .await
            .unwrap();
    assert_eq!(historical_rows.len(), 1);
    assert_eq!(
        historical_rows[0]["artefact_id"],
        Value::String(old_record.artefact_id.clone())
    );
    assert_eq!(historical_rows[0]["start_line"], Value::from(1));
    assert_eq!(historical_rows[0]["end_line"], Value::from(3));
    assert_eq!(historical_rows[0]["start_byte"], Value::from(0));
    assert_eq!(historical_rows[0]["end_byte"], Value::from(35));
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn older_current_refresh_does_not_clobber_newer_snapshot_for_the_same_path() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let cfg = test_cfg_with_repo_id("devql-current-snapshot-recency-guard", &dsn);
    init_postgres_schema(&cfg, &client).await.unwrap();
    let relational = postgres_relational_store(&cfg, &dsn).await;

    let path = "src/recency_guard.ts";
    let symbol_id = deterministic_uuid("recency-guard-symbol");
    let old_blob = "blob-old";
    let new_blob = "blob-new";
    let old_file = test_file_row(&cfg, path, old_blob, 4, 48);
    let new_file = test_file_row(&cfg, path, new_blob, 8, 96);
    let old_record = test_symbol_record(&cfg, path, old_blob, &symbol_id, "greet", 1, 3);
    let new_record = test_symbol_record(&cfg, path, new_blob, &symbol_id, "greet", 5, 8);

    refresh_current_state_for_path(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-new",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Commit,
                id: "commit-new",
                temp_checkpoint_id: None,
            },
            commit_unix: 200,
            path,
            blob_sha: new_blob,
        },
        &new_file,
        None,
        std::slice::from_ref(&new_record),
        vec![],
    )
    .await
    .unwrap();
    refresh_current_state_for_path(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-old",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Commit,
                id: "commit-old",
                temp_checkpoint_id: None,
            },
            commit_unix: 100,
            path,
            blob_sha: old_blob,
        },
        &old_file,
        None,
        &[old_record],
        vec![],
    )
    .await
    .unwrap();

    let row = client
        .query_one(
            "SELECT commit_sha, blob_sha, artefact_id, start_line, end_line FROM artefacts_current WHERE repo_id = $1 AND symbol_id = $2",
            &[&cfg.repo.repo_id, &symbol_id],
        )
        .await
        .unwrap();
    let commit_sha: String = row.get(0);
    let blob_sha: String = row.get(1);
    let artefact_id: String = row.get(2);
    let start_line: i32 = row.get(3);
    let end_line: i32 = row.get(4);

    assert_eq!(commit_sha, "commit-new");
    assert_eq!(blob_sha, new_blob);
    assert_eq!(artefact_id, new_record.artefact_id);
    assert_eq!(start_line, 5);
    assert_eq!(end_line, 8);
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn refreshing_a_path_rebuilds_current_outgoing_edges_instead_of_accumulating_stale_ones() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let cfg = test_cfg_with_repo_id("devql-current-outgoing-edge-refresh", &dsn);
    init_postgres_schema(&cfg, &client).await.unwrap();
    let relational = postgres_relational_store(&cfg, &dsn).await;

    let path = "src/caller.ts";
    let symbol_id = deterministic_uuid("caller-symbol");
    let old_blob = "blob-caller-old";
    let new_blob = "blob-caller-new";
    let old_file = test_file_row(&cfg, path, old_blob, 5, 60);
    let new_file = test_file_row(&cfg, path, new_blob, 5, 60);
    let old_record = test_symbol_record(&cfg, path, old_blob, &symbol_id, "caller", 1, 4);
    let new_record = test_symbol_record(&cfg, path, new_blob, &symbol_id, "caller", 1, 4);

    refresh_current_state_for_path(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-1",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Commit,
                id: "commit-1",
                temp_checkpoint_id: None,
            },
            commit_unix: 100,
            path,
            blob_sha: old_blob,
        },
        &old_file,
        None,
        std::slice::from_ref(&old_record),
        vec![test_unresolved_call_edge(
            &old_record.symbol_fqn,
            "src/lib.ts::old_target",
            2,
        )],
    )
    .await
    .unwrap();
    refresh_current_state_for_path(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-2",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Commit,
                id: "commit-2",
                temp_checkpoint_id: None,
            },
            commit_unix: 200,
            path,
            blob_sha: new_blob,
        },
        &new_file,
        None,
        std::slice::from_ref(&new_record),
        vec![test_unresolved_call_edge(
            &new_record.symbol_fqn,
            "src/lib.ts::new_target",
            3,
        )],
    )
    .await
    .unwrap();

    let rows = client
        .query(
            "SELECT to_symbol_ref, start_line FROM artefact_edges_current WHERE repo_id = $1 AND path = $2 ORDER BY start_line",
            &[&cfg.repo.repo_id, &path],
        )
        .await
        .unwrap();

    assert_eq!(rows.len(), 1);
    let to_symbol_ref: Option<String> = rows[0].get(0);
    let start_line: Option<i32> = rows[0].get(1);
    assert_eq!(to_symbol_ref.as_deref(), Some("src/lib.ts::new_target"));
    assert_eq!(start_line, Some(3));
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn deleting_a_current_symbol_removes_its_row_and_clears_inbound_edge_target_ids() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let cfg = test_cfg_with_repo_id("devql-current-delete-target", &dsn);
    init_postgres_schema(&cfg, &client).await.unwrap();
    let relational = postgres_relational_store(&cfg, &dsn).await;

    let target_path = "src/target.ts";
    let caller_path = "src/caller.ts";
    let target_symbol_id = deterministic_uuid("delete-target-symbol");
    let caller_symbol_id = deterministic_uuid("delete-caller-symbol");
    let target_blob = "blob-target-present";
    let target_deleted_blob = "blob-target-deleted";
    let caller_blob = "blob-caller";
    let target_file = test_file_row(&cfg, target_path, target_blob, 4, 48);
    let target_deleted_file = test_file_row(&cfg, target_path, target_deleted_blob, 1, 12);
    let caller_file = test_file_row(&cfg, caller_path, caller_blob, 5, 60);
    let target_record = test_symbol_record(
        &cfg,
        target_path,
        target_blob,
        &target_symbol_id,
        "target",
        1,
        3,
    );
    let caller_record = test_symbol_record(
        &cfg,
        caller_path,
        caller_blob,
        &caller_symbol_id,
        "caller",
        1,
        4,
    );

    refresh_current_state_for_path(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-target-1",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Commit,
                id: "commit-target-1",
                temp_checkpoint_id: None,
            },
            commit_unix: 100,
            path: target_path,
            blob_sha: target_blob,
        },
        &target_file,
        None,
        std::slice::from_ref(&target_record),
        vec![],
    )
    .await
    .unwrap();
    refresh_current_state_for_path(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-caller-1",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Commit,
                id: "commit-caller-1",
                temp_checkpoint_id: None,
            },
            commit_unix: 110,
            path: caller_path,
            blob_sha: caller_blob,
        },
        &caller_file,
        None,
        std::slice::from_ref(&caller_record),
        vec![test_call_edge(
            &caller_record.symbol_fqn,
            &target_record.symbol_fqn,
            2,
        )],
    )
    .await
    .unwrap();
    refresh_current_state_for_path(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-target-2",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Commit,
                id: "commit-target-2",
                temp_checkpoint_id: None,
            },
            commit_unix: 200,
            path: target_path,
            blob_sha: target_deleted_blob,
        },
        &target_deleted_file,
        None,
        &[],
        vec![],
    )
    .await
    .unwrap();

    let target_count = client
        .query_one(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = $1 AND symbol_id = $2",
            &[&cfg.repo.repo_id, &target_symbol_id],
        )
        .await
        .unwrap();
    let target_count: i64 = target_count.get(0);
    assert_eq!(target_count, 0);

    let edge = client
        .query_one(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref FROM artefact_edges_current WHERE repo_id = $1 AND path = $2",
            &[&cfg.repo.repo_id, &caller_path],
        )
        .await
        .unwrap();
    let to_symbol_id: Option<String> = edge.get(0);
    let to_artefact_id: Option<String> = edge.get(1);
    let to_symbol_ref: Option<String> = edge.get(2);
    assert!(to_symbol_id.is_none());
    assert!(to_artefact_id.is_none());
    assert_eq!(
        to_symbol_ref.as_deref(),
        Some(target_record.symbol_fqn.as_str())
    );
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn cross_file_current_edges_resolve_targets_and_retarget_after_target_refresh() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let cfg = test_cfg_with_repo_id("devql-current-cross-file-resolution", &dsn);
    init_postgres_schema(&cfg, &client).await.unwrap();
    let relational = postgres_relational_store(&cfg, &dsn).await;

    let target_path = "src/lib.ts";
    let caller_path = "src/app.ts";
    let target_symbol_id = deterministic_uuid("cross-file-target-symbol");
    let caller_symbol_id = deterministic_uuid("cross-file-caller-symbol");
    let target_blob_v1 = "blob-lib-v1";
    let target_blob_v2 = "blob-lib-v2";
    let caller_blob = "blob-app-v1";
    let target_file_v1 = test_file_row(&cfg, target_path, target_blob_v1, 4, 48);
    let target_file_v2 = test_file_row(&cfg, target_path, target_blob_v2, 6, 72);
    let caller_file = test_file_row(&cfg, caller_path, caller_blob, 5, 60);
    let target_record_v1 = test_symbol_record(
        &cfg,
        target_path,
        target_blob_v1,
        &target_symbol_id,
        "helper",
        1,
        3,
    );
    let target_record_v2 = test_symbol_record(
        &cfg,
        target_path,
        target_blob_v2,
        &target_symbol_id,
        "helper",
        3,
        6,
    );
    let caller_record = test_symbol_record(
        &cfg,
        caller_path,
        caller_blob,
        &caller_symbol_id,
        "caller",
        1,
        4,
    );

    refresh_current_state_for_path(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-lib-1",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Commit,
                id: "commit-lib-1",
                temp_checkpoint_id: None,
            },
            commit_unix: 100,
            path: target_path,
            blob_sha: target_blob_v1,
        },
        &target_file_v1,
        None,
        std::slice::from_ref(&target_record_v1),
        vec![],
    )
    .await
    .unwrap();
    refresh_current_state_for_path(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-app-1",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Commit,
                id: "commit-app-1",
                temp_checkpoint_id: None,
            },
            commit_unix: 110,
            path: caller_path,
            blob_sha: caller_blob,
        },
        &caller_file,
        None,
        std::slice::from_ref(&caller_record),
        vec![test_call_edge(
            &caller_record.symbol_fqn,
            &target_record_v1.symbol_fqn,
            2,
        )],
    )
    .await
    .unwrap();

    let initial_edge = client
        .query_one(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref FROM artefact_edges_current WHERE repo_id = $1 AND path = $2",
            &[&cfg.repo.repo_id, &caller_path],
        )
        .await
        .unwrap();
    let initial_to_symbol_id: Option<String> = initial_edge.get(0);
    let initial_to_artefact_id: Option<String> = initial_edge.get(1);
    let initial_to_symbol_ref: Option<String> = initial_edge.get(2);
    assert_eq!(
        initial_to_symbol_id.as_deref(),
        Some(target_symbol_id.as_str())
    );
    assert_eq!(
        initial_to_artefact_id.as_deref(),
        Some(target_record_v1.artefact_id.as_str())
    );
    assert_eq!(
        initial_to_symbol_ref.as_deref(),
        Some(target_record_v1.symbol_fqn.as_str())
    );

    refresh_current_state_for_path(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-lib-2",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Commit,
                id: "commit-lib-2",
                temp_checkpoint_id: None,
            },
            commit_unix: 200,
            path: target_path,
            blob_sha: target_blob_v2,
        },
        &target_file_v2,
        None,
        std::slice::from_ref(&target_record_v2),
        vec![],
    )
    .await
    .unwrap();

    let refreshed_edge = client
        .query_one(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref FROM artefact_edges_current WHERE repo_id = $1 AND path = $2",
            &[&cfg.repo.repo_id, &caller_path],
        )
        .await
        .unwrap();
    let refreshed_to_symbol_id: Option<String> = refreshed_edge.get(0);
    let refreshed_to_artefact_id: Option<String> = refreshed_edge.get(1);
    let refreshed_to_symbol_ref: Option<String> = refreshed_edge.get(2);
    assert_eq!(
        refreshed_to_symbol_id.as_deref(),
        Some(target_symbol_id.as_str())
    );
    assert_eq!(
        refreshed_to_artefact_id.as_deref(),
        Some(target_record_v2.artefact_id.as_str())
    );
    assert_eq!(
        refreshed_to_symbol_ref.as_deref(),
        Some(target_record_v2.symbol_fqn.as_str())
    );
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn export_edges_dedupe_same_alias_but_preserve_alias_distinct_in_postgres() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let cfg = test_cfg();
    init_postgres_schema(&cfg, &client).await.unwrap();

    let file_symbol_id = deterministic_uuid("file-symbol");
    let file_artefact_id = deterministic_uuid("file-artefact");
    let target_symbol_id = deterministic_uuid("target-symbol");
    let target_artefact_id = deterministic_uuid("target-artefact");

    let insert_file = format!(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash) \
VALUES ('{}', '{}', '{}', 'blob-exports', 'src/lib.ts', 'typescript', 'file', 'file', 'src/lib.ts', NULL, 1, 20, 0, 100, 'src/lib.ts', 'file-hash')",
        esc_pg(&file_artefact_id),
        esc_pg(&file_symbol_id),
        esc_pg(&cfg.repo.repo_id)
    );
    let insert_target = format!(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash) \
VALUES ('{}', '{}', '{}', 'blob-exports', 'src/lib.ts', 'typescript', 'function', 'function_declaration', 'src/lib.ts::helper', NULL, 2, 4, 10, 30, 'function helper() {{', 'target-hash')",
        esc_pg(&target_artefact_id),
        esc_pg(&target_symbol_id),
        esc_pg(&cfg.repo.repo_id)
    );

    postgres_exec(&client, &insert_file).await.unwrap();
    postgres_exec(&client, &insert_target).await.unwrap();

    let export_a = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_artefact_id, edge_kind, language, metadata) \
VALUES ('{}', '{}', 'blob-exports', '{}', '{}', 'exports', 'typescript', '{{\"export_name\":\"helper\",\"export_form\":\"named\",\"resolution\":\"local\"}}'::jsonb)",
        esc_pg(&deterministic_uuid("export-helper-a")),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&file_artefact_id),
        esc_pg(&target_artefact_id)
    );
    let export_dup = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_artefact_id, edge_kind, language, metadata) \
VALUES ('{}', '{}', 'blob-exports', '{}', '{}', 'exports', 'typescript', '{{\"export_name\":\"helper\",\"export_form\":\"named\",\"resolution\":\"local\"}}'::jsonb)",
        esc_pg(&deterministic_uuid("export-helper-b")),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&file_artefact_id),
        esc_pg(&target_artefact_id)
    );
    let export_alias = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_artefact_id, edge_kind, language, metadata) \
VALUES ('{}', '{}', 'blob-exports', '{}', '{}', 'exports', 'typescript', '{{\"export_name\":\"helperAlias\",\"export_form\":\"named\",\"resolution\":\"local\"}}'::jsonb)",
        esc_pg(&deterministic_uuid("export-helper-alias")),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&file_artefact_id),
        esc_pg(&target_artefact_id)
    );

    postgres_exec(&client, &export_a).await.unwrap();
    assert!(postgres_exec(&client, &export_dup).await.is_err());
    postgres_exec(&client, &export_alias).await.unwrap();

    let row = client
        .query_one(
            "SELECT COUNT(*) FROM artefact_edges WHERE repo_id = $1 AND edge_kind = 'exports'",
            &[&cfg.repo.repo_id],
        )
        .await
        .unwrap();
    let count: i64 = row.get(0);
    assert_eq!(
        count, 2,
        "expected alias-distinct export edges to survive dedup"
    );
}

#[test]
fn postgres_schema_sql_includes_checkpoint_migration_tables() {
    let schema = format!(
        "{}\n{}",
        postgres_schema_sql(),
        checkpoint_schema_sql_postgres()
    );
    for table in [
        "sessions",
        "temporary_checkpoints",
        "checkpoints",
        "checkpoint_sessions",
        "commit_checkpoints",
        "pre_prompt_states",
        "pre_task_markers",
        "checkpoint_blobs",
    ] {
        assert!(
            schema.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")),
            "expected checkpoint table `{table}` in postgres schema"
        );
    }
}
