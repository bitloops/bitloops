#[test]
fn sql_helpers_escape_nullable_and_json_values() {
    assert_eq!(sql_nullable_text(None), "NULL");
    assert_eq!(sql_nullable_text(Some("O'Reilly")), "'O''Reilly'");
    assert_eq!(
        sql_jsonb_text_array(&["O'Reilly".to_string(), "plain".to_string()]),
        r#"'["O''Reilly","plain"]'::jsonb"#
    );
}

#[test]
fn supported_symbol_languages_are_whitelisted() {
    for language in ["typescript", "javascript", "rust"] {
        assert!(
            is_supported_symbol_language(language),
            "{language} should be supported"
        );
    }

    for language in ["python", "go", ""] {
        assert!(
            !is_supported_symbol_language(language),
            "{language} should not be supported"
        );
    }
}

#[test]
fn build_file_current_record_preserves_file_metadata() {
    let cfg = test_cfg();
    let file = test_file_row(&cfg, "src/main.rs", "blob-1", 42, 420);
    let record = build_file_current_record(
        "src/main.rs",
        "blob-1",
        &file,
        Some("Top-level docs".to_string()),
    );

    assert_eq!(record.symbol_id, file.symbol_id);
    assert_eq!(record.artefact_id, file.artefact_id);
    assert_eq!(record.canonical_kind.as_deref(), Some("file"));
    assert_eq!(record.language_kind, "file");
    assert_eq!(record.symbol_fqn, "src/main.rs");
    assert_eq!(record.end_line, 42);
    assert_eq!(record.end_byte, 420);
    assert_eq!(record.docstring.as_deref(), Some("Top-level docs"));
    assert_eq!(record.content_hash, "blob-1");
}

#[test]
fn build_symbol_records_chain_file_and_nested_parent_links() {
    let cfg = test_cfg();
    let path = "src/ui.ts";
    let blob_sha = "blob-ui";
    let file = test_file_row(&cfg, path, blob_sha, 30, 300);
    let items = vec![
        JsTsArtefact {
            canonical_kind: Some("class".to_string()),
            language_kind: "class_declaration".to_string(),
            name: "Widget".to_string(),
            symbol_fqn: format!("{path}::Widget"),
            parent_symbol_fqn: None,
            start_line: 1,
            end_line: 20,
            start_byte: 0,
            end_byte: 200,
            signature: "export class Widget {}".to_string(),
            modifiers: vec!["export".to_string()],
            docstring: Some("Widget docs".to_string()),
        },
        JsTsArtefact {
            canonical_kind: Some("method".to_string()),
            language_kind: "method_definition".to_string(),
            name: "render".to_string(),
            symbol_fqn: format!("{path}::Widget::render"),
            parent_symbol_fqn: Some(format!("{path}::Widget")),
            start_line: 5,
            end_line: 10,
            start_byte: 40,
            end_byte: 120,
            signature: "render(): void {}".to_string(),
            modifiers: vec![],
            docstring: None,
        },
    ];

    let records = build_symbol_records(&cfg, path, blob_sha, &file, &items);
    assert_eq!(records.len(), 2);

    let class_record = &records[0];
    assert_eq!(class_record.parent_symbol_id, Some(file.symbol_id.clone()));
    assert_eq!(
        class_record.parent_artefact_id,
        Some(file.artefact_id.clone())
    );
    assert_eq!(class_record.docstring.as_deref(), Some("Widget docs"));

    let method_record = &records[1];
    assert_eq!(
        method_record.parent_symbol_id,
        Some(class_record.symbol_id.clone())
    );
    assert_eq!(
        method_record.parent_artefact_id,
        Some(class_record.artefact_id.clone())
    );
    assert_eq!(
        method_record.signature.as_deref(),
        Some("render(): void {}")
    );
}

#[test]
fn build_historical_edge_records_keep_resolved_and_unresolved_targets() {
    let cfg = test_cfg();
    let path = "src/main.ts";
    let blob_sha = "blob-2";
    let from = test_symbol_record(&cfg, path, blob_sha, "from-symbol", "source", 1, 2);
    let to = test_symbol_record(&cfg, path, blob_sha, "to-symbol", "target", 4, 5);
    let current_by_fqn = [
        (from.symbol_fqn.clone(), from.clone()),
        (to.symbol_fqn.clone(), to.clone()),
    ]
    .into_iter()
    .collect::<HashMap<_, _>>();

    let records = build_historical_edge_records(
        &cfg,
        blob_sha,
        "typescript",
        vec![
            test_call_edge(&from.symbol_fqn, &to.symbol_fqn, 7),
            test_unresolved_call_edge(&from.symbol_fqn, "remote::symbol", 9),
            test_call_edge("missing::from", &to.symbol_fqn, 11),
        ],
        &current_by_fqn,
    );

    assert_eq!(records.len(), 2);
    assert_eq!(
        records[0].to_symbol_id.as_deref(),
        Some(to.symbol_id.as_str())
    );
    assert_eq!(
        records[0].to_artefact_id.as_deref(),
        Some(to.artefact_id.as_str())
    );
    assert!(records[0].to_symbol_ref.is_none());
    assert!(records[1].to_symbol_id.is_none());
    assert!(records[1].to_artefact_id.is_none());
    assert_eq!(records[1].to_symbol_ref.as_deref(), Some("remote::symbol"));
}

#[test]
fn build_current_edge_records_resolve_local_and_external_targets() {
    let cfg = test_cfg();
    let path = "src/main.ts";
    let blob_sha = "blob-3";
    let from = test_symbol_record(&cfg, path, blob_sha, "from-symbol", "source", 1, 2);
    let to = test_symbol_record(&cfg, path, blob_sha, "to-symbol", "target", 4, 5);
    let current_by_fqn = [
        (from.symbol_fqn.clone(), from.clone()),
        (to.symbol_fqn.clone(), to.clone()),
    ]
    .into_iter()
    .collect::<HashMap<_, _>>();
    let external_targets = [(
        "pkg::remote".to_string(),
        (
            "external-symbol".to_string(),
            "external-artefact".to_string(),
        ),
    )]
    .into_iter()
    .collect::<HashMap<_, _>>();

    let records = build_current_edge_records(
        &cfg,
        path,
        "typescript",
        vec![
            test_call_edge(&from.symbol_fqn, &to.symbol_fqn, 7),
            test_unresolved_call_edge(&from.symbol_fqn, "pkg::remote", 8),
        ],
        &current_by_fqn,
        &external_targets,
    );

    assert_eq!(records.len(), 2);
    assert_eq!(
        records[0].to_symbol_id.as_deref(),
        Some(to.symbol_id.as_str())
    );
    assert_eq!(
        records[0].to_artefact_id.as_deref(),
        Some(to.artefact_id.as_str())
    );
    assert_eq!(records[1].to_symbol_id.as_deref(), Some("external-symbol"));
    assert_eq!(
        records[1].to_artefact_id.as_deref(),
        Some("external-artefact")
    );
    assert_eq!(records[1].to_symbol_ref.as_deref(), Some("pkg::remote"));
}

#[test]
fn incoming_revision_is_newer_prefers_revision_kind_then_timestamp_then_sha() {
    let state = |commit_sha: &str, revision_kind: &str, revision_id: &str, updated_at_unix: i64| {
        CurrentFileRevisionRecord {
            commit_sha: commit_sha.to_string(),
            revision_kind: revision_kind.to_string(),
            revision_id: revision_id.to_string(),
            temp_checkpoint_id: None,
            blob_sha: "blob".to_string(),
            updated_at_unix,
        }
    };
    assert!(incoming_revision_is_newer(None, "commit", "bbb", 10));
    let existing_1 = state("aaa", "commit", "aaa", 9);
    assert!(incoming_revision_is_newer(
        Some(&existing_1),
        "commit",
        "bbb",
        10
    ));
    let existing_2 = state("zzz", "commit", "zzz", 11);
    assert!(!incoming_revision_is_newer(
        Some(&existing_2),
        "commit",
        "bbb",
        10
    ));
    let existing_3 = state("aaa", "commit", "aaa", 10);
    assert!(incoming_revision_is_newer(
        Some(&existing_3),
        "commit",
        "bbb",
        10
    ));
    let existing_4 = state("ccc", "commit", "ccc", 10);
    assert!(!incoming_revision_is_newer(
        Some(&existing_4),
        "commit",
        "bbb",
        10
    ));
    let existing_5 = state("temp:9", "temporary", "temp:9", 10);
    assert!(incoming_revision_is_newer(
        Some(&existing_5),
        "temporary",
        "temp:10",
        10
    ));
    let existing_6 = state("temp:10", "temporary", "temp:10", 10);
    assert!(!incoming_revision_is_newer(
        Some(&existing_6),
        "temporary",
        "temp:9",
        10
    ));
    let existing_7 = state("commit-a", "commit", "commit-a", 100);
    assert!(incoming_revision_is_newer(
        Some(&existing_7),
        "temporary",
        "temp:200",
        200
    ));
    let existing_7b = state("commit-a", "commit", "commit-a", 100);
    assert!(incoming_revision_is_newer(
        Some(&existing_7b),
        "temporary",
        "temp:201",
        100
    ));
    let existing_8 = state("commit-a", "temporary", "temp:88", 100);
    assert!(incoming_revision_is_newer(
        Some(&existing_8),
        "commit",
        "commit-b",
        100
    ));
}

#[tokio::test]
async fn commit_revision_replaces_temporary_current_metadata_for_unchanged_content() {
    let cfg = test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("relational.db");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    let path = "src/train.txt";
    let blob_sha = "blob-stable";

    upsert_current_state_for_content(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-old",
            revision: RevisionRef {
                kind: "temporary",
                id: "temp:7",
                temp_checkpoint_id: Some(7),
            },
            commit_unix: 100,
            path,
            blob_sha,
        },
        "hello world\n",
    )
    .await
    .expect("write temporary current state");

    upsert_current_state_for_content(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-new",
            revision: RevisionRef {
                kind: "commit",
                id: "commit-new",
                temp_checkpoint_id: None,
            },
            commit_unix: 100,
            path,
            blob_sha,
        },
        "hello world\n",
    )
    .await
    .expect("write committed current state");

    let conn = rusqlite::Connection::open(sqlite_path).expect("open sqlite");
    let artefact_row: (String, String, String, Option<i64>) = conn
        .query_row(
            "SELECT commit_sha, revision_kind, revision_id, temp_checkpoint_id \
             FROM artefacts_current WHERE repo_id = ?1 AND symbol_id = ?2",
            rusqlite::params![cfg.repo.repo_id, file_symbol_id(path)],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("fetch current file artefact row");
    assert_eq!(artefact_row.0, "commit-new");
    assert_eq!(artefact_row.1, "commit");
    assert_eq!(artefact_row.2, "commit-new");
    assert!(artefact_row.3.is_none());
}

#[tokio::test]
async fn refresh_current_state_deletes_stale_edge_ids_before_upserting_new_natural_edge() {
    let cfg = test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("relational.db");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;

    let path = "src/main.ts";
    let blob_sha = "blob-123";
    let file_artefact = test_file_row(&cfg, path, blob_sha, 20, 200);
    let source = test_symbol_record(&cfg, path, blob_sha, "source-symbol", "source", 1, 5);
    let edge = test_unresolved_call_edge(&source.symbol_fqn, "pkg::remote", 4);
    let edge_metadata = edge.metadata.to_value().to_string();

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    conn.execute(
        "INSERT INTO artefact_edges_current (
            edge_id, repo_id, commit_sha, revision_kind, revision_id, temp_checkpoint_id,
            blob_sha, path, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id,
            to_symbol_ref, edge_kind, language, start_line, end_line, metadata
        ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, NULL, NULL, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            "legacy-edge-id",
            cfg.repo.repo_id,
            "commit-old",
            "commit",
            "commit-old",
            "blob-old",
            path,
            source.symbol_id,
            source.artefact_id,
            "pkg::remote",
            "calls",
            "typescript",
            4_i64,
            4_i64,
            edge_metadata,
        ],
    )
    .expect("insert stale edge row with legacy edge_id");

    refresh_current_state_for_path(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-new",
            revision: RevisionRef {
                kind: "commit",
                id: "commit-new",
                temp_checkpoint_id: None,
            },
            commit_unix: 200,
            path,
            blob_sha,
        },
        &file_artefact,
        None,
        &[source],
        vec![edge],
    )
    .await
    .expect("refresh current state should replace stale edge ids");

    let edge_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![cfg.repo.repo_id, path],
            |row| row.get(0),
        )
        .expect("count current edges");
    assert_eq!(edge_rows, 1);
    let has_legacy: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND edge_id = 'legacy-edge-id'",
            rusqlite::params![cfg.repo.repo_id],
            |row| row.get(0),
        )
        .expect("count legacy edge ids");
    assert_eq!(has_legacy, 0);
}

#[tokio::test]
async fn promote_temporary_rows_for_head_commit_updates_file_row_to_commit() {
    let repo_dir = tempdir().expect("temp dir");
    init_test_repo(
        repo_dir.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    std::fs::create_dir_all(repo_dir.path().join("src")).expect("create src dir");
    std::fs::write(
        repo_dir.path().join("src/lib.rs"),
        "pub fn one() -> i32 {\n    1\n}\n",
    )
    .expect("write initial source");
    git_ok(repo_dir.path(), &["add", "."]);
    git_ok(repo_dir.path(), &["commit", "-m", "initial"]);
    let old_head = git_ok(repo_dir.path(), &["rev-parse", "HEAD"]);

    let repo = resolve_repo_identity(repo_dir.path()).expect("resolve repo identity");
    let cfg = DevqlConfig::from_env(repo_dir.path().to_path_buf(), repo).expect("build config");
    let sqlite_path = repo_dir.path().join("devql-relational.db");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;

    let path = "src/lib.rs";
    let updated_content = "pub fn two() -> i32 {\n    2\n}\n";
    std::fs::write(repo_dir.path().join(path), updated_content).expect("update source");
    let blob_sha = git_ok(repo_dir.path(), &["hash-object", path]);

    upsert_current_state_for_content(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: &old_head,
            revision: RevisionRef {
                kind: "temporary",
                id: "temp:1",
                temp_checkpoint_id: Some(1),
            },
            commit_unix: 100,
            path,
            blob_sha: &blob_sha,
        },
        updated_content,
    )
    .await
    .expect("insert temporary current state");

    git_ok(repo_dir.path(), &["add", path]);
    git_ok(repo_dir.path(), &["commit", "-m", "update"]);
    let new_head = git_ok(repo_dir.path(), &["rev-parse", "HEAD"]);

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let promoted = promote_temporary_current_rows_for_head_commit(&cfg, &relational)
        .await
        .expect("promote temporary rows");
    assert_eq!(promoted, 1);

    let row: (String, String, String, Option<i64>) = conn
        .query_row(
            "SELECT commit_sha, revision_kind, revision_id, temp_checkpoint_id \
             FROM artefacts_current WHERE repo_id = ?1 AND symbol_id = ?2",
            rusqlite::params![cfg.repo.repo_id, file_symbol_id(path)],
            |record| {
                Ok((
                    record.get(0)?,
                    record.get(1)?,
                    record.get(2)?,
                    record.get(3)?,
                ))
            },
        )
        .expect("read current file row");
    assert_eq!(row.0, new_head);
    assert_eq!(row.1, "commit");
    assert_eq!(row.2, new_head);
    assert!(row.3.is_none());

    let historical_blob: String = conn
        .query_row(
            "SELECT blob_sha FROM file_state WHERE repo_id = ?1 AND commit_sha = ?2 AND path = ?3",
            rusqlite::params![cfg.repo.repo_id, new_head, path],
            |record| record.get(0),
        )
        .expect("read committed file_state row");
    assert_eq!(historical_blob, blob_sha);

    let temp_artefacts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2 AND revision_kind = 'temporary'",
            rusqlite::params![cfg.repo.repo_id, path],
            |record| record.get(0),
        )
        .expect("count temporary artefacts");
    assert_eq!(temp_artefacts, 0);
}

#[test]
fn default_branch_name_uses_current_branch_and_falls_back_to_main() {
    let repo = seed_git_repo();
    git_ok(repo.path(), &["checkout", "-B", "feature/test-branch"]);

    assert_eq!(default_branch_name(repo.path()), "feature/test-branch");
    assert_eq!(
        default_branch_name(tempdir().expect("temp dir").path()),
        "main"
    );
}

#[test]
fn collect_checkpoint_commit_map_prefers_newest_db_mapped_checkpoint_commit() {
    let repo = seed_git_repo();
    let checkpoint_id = "aabbccddeeff";

    git_ok(
        repo.path(),
        &["commit", "--allow-empty", "-m", "older checkpoint"],
    );
    let older_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    git_ok(
        repo.path(),
        &["commit", "--allow-empty", "-m", "newest checkpoint"],
    );
    let newest_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    insert_commit_checkpoint_mapping(repo.path(), &older_sha, checkpoint_id);
    insert_commit_checkpoint_mapping(repo.path(), &newest_sha, checkpoint_id);

    let checkpoint_map =
        collect_checkpoint_commit_map(repo.path()).expect("checkpoint commit map should build");

    assert_eq!(checkpoint_map.len(), 1);
    let info = checkpoint_map
        .get(checkpoint_id)
        .expect("checkpoint should be present");
    assert_eq!(info.subject, "newest checkpoint");
    assert!(!info.commit_sha.is_empty());
    assert!(info.commit_unix > 0);
}

#[test]
fn collect_checkpoint_commit_map_reads_commit_checkpoints_table() {
    let repo = seed_git_repo();
    let checkpoint_id = "b0b1b2b3b4b5";

    git_ok(
        repo.path(),
        &[
            "commit",
            "--allow-empty",
            "-m",
            "checkpoint without trailer",
        ],
    );
    let commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    insert_commit_checkpoint_mapping(repo.path(), &commit_sha, checkpoint_id);

    let checkpoint_map =
        collect_checkpoint_commit_map(repo.path()).expect("checkpoint commit map should build");

    assert_eq!(checkpoint_map.len(), 1);
    let info = checkpoint_map
        .get(checkpoint_id)
        .expect("checkpoint should be present");
    assert_eq!(info.commit_sha, commit_sha);
    assert_eq!(info.subject, "checkpoint without trailer");
    assert!(info.commit_unix > 0);
}
