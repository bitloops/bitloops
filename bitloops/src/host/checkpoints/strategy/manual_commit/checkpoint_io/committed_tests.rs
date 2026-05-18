use super::*;

fn sample_checkpoint_file_row()
-> crate::host::devql::checkpoint_provenance::CheckpointFileProvenanceRow {
    crate::host::devql::checkpoint_provenance::CheckpointFileProvenanceRow {
        relation_id: "relation-1".to_string(),
        repo_id: "repo-1".to_string(),
        checkpoint_id: "checkpoint-1".to_string(),
        session_id: "session-1".to_string(),
        event_time: "2026-05-18T10:00:00Z".to_string(),
        agent: "claude-code".to_string(),
        branch: "main".to_string(),
        strategy: "manual-commit".to_string(),
        commit_sha: "abc123".to_string(),
        change_kind: crate::host::devql::checkpoint_provenance::CheckpointFileChangeKind::Modify,
        path_before: Some("src/lib.rs".to_string()),
        path_after: Some("src/lib.rs".to_string()),
        blob_sha_before: Some("blob-before".to_string()),
        blob_sha_after: Some("blob-after".to_string()),
        copy_source_path: None,
        copy_source_blob_sha: None,
    }
}

fn sample_committed_metadata() -> CommittedMetadata {
    CommittedMetadata {
        checkpoint_id: "checkpoint-1".to_string(),
        session_id: "session-1".to_string(),
        checkpoints_count: 2,
        strategy: "manual-commit".to_string(),
        agent: "claude-code".to_string(),
        created_at: "2026-05-18T10:00:00Z".to_string(),
        cli_version: CLI_VERSION.to_string(),
        turn_id: "turn-1".to_string(),
        is_task: false,
        tool_use_id: String::new(),
        transcript_identifier_at_start: "msg-1".to_string(),
        checkpoint_transcript_start: 0,
        transcript_lines_at_start: 0,
        branch: "main".to_string(),
        summary: None,
        token_usage: None,
        initial_attribution: None,
        transcript_path: String::new(),
    }
}

#[test]
fn replace_checkpoint_provenance_rows_for_session_persists_locally_when_shared_role_is_sqlite() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let sqlite_path = tmp.path().join("checkpoint-relational.sqlite");
    let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path.clone())
        .expect("connect checkpoint sqlite");
    sqlite
        .initialise_relational_checkpoint_schema()
        .expect("initialise checkpoint schema");
    let relational = crate::host::relational_store::DefaultRelationalStore::from_inner(
        crate::host::devql::RelationalStorage::primary_backend_for_tests(
            sqlite_path,
            crate::host::devql::RelationalPrimaryBackend::Sqlite,
        ),
    );

    replace_checkpoint_provenance_rows_for_session(
        &relational,
        "repo-1",
        "checkpoint-1",
        "session-1",
        &[sample_checkpoint_file_row()],
        &crate::host::devql::checkpoint_provenance::CheckpointArtefactProvenanceBundle::default(),
    )
    .expect("local shared role should persist checkpoint provenance rows");

    let local_count = sqlite
        .with_connection(|conn| {
            conn.query_row("SELECT COUNT(*) FROM checkpoint_files", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(anyhow::Error::from)
        })
        .expect("count local checkpoint_files rows");
    assert_eq!(local_count, 1);
}

#[test]
fn replace_checkpoint_provenance_rows_for_session_routes_off_local_sqlite_when_shared_role_is_remote()
 {
    let tmp = tempfile::tempdir().expect("temp dir");
    let sqlite_path = tmp.path().join("checkpoint-relational.sqlite");
    let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path.clone())
        .expect("connect checkpoint sqlite");
    sqlite
        .initialise_relational_checkpoint_schema()
        .expect("initialise checkpoint schema");
    let relational = crate::host::relational_store::DefaultRelationalStore::from_inner(
        crate::host::devql::RelationalStorage::primary_backend_for_tests(
            sqlite_path,
            crate::host::devql::RelationalPrimaryBackend::Postgres,
        ),
    );

    let err = replace_checkpoint_provenance_rows_for_session(
        &relational,
        "repo-1",
        "checkpoint-1",
        "session-1",
        &[sample_checkpoint_file_row()],
        &crate::host::devql::checkpoint_provenance::CheckpointArtefactProvenanceBundle::default(),
    )
    .expect_err("remote shared role should not fall back to local checkpoint provenance");
    assert!(
        format!("{err:#}")
            .contains("remote Postgres shared relational backend is configured without a DSN"),
        "expected shared relational remote routing error, got: {err:#}"
    );

    let local_count = sqlite
        .with_connection(|conn| {
            conn.query_row("SELECT COUNT(*) FROM checkpoint_files", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(anyhow::Error::from)
        })
        .expect("count local checkpoint_files rows");
    assert_eq!(local_count, 0);
}

#[test]
fn checkpoint_session_metadata_writes_route_off_local_sqlite_when_shared_role_is_remote() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let sqlite_path = tmp.path().join("checkpoint-relational.sqlite");
    let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path.clone())
        .expect("connect checkpoint sqlite");
    sqlite
        .initialise_relational_checkpoint_schema()
        .expect("initialise checkpoint schema");
    let relational = crate::host::relational_store::DefaultRelationalStore::from_inner(
        crate::host::devql::RelationalStorage::primary_backend_for_tests(
            sqlite_path,
            crate::host::devql::RelationalPrimaryBackend::Postgres,
        ),
    );

    let sql = build_upsert_checkpoint_session_row_sql(
        0,
        &sample_committed_metadata(),
        "Test Author",
        "test@example.com",
        "sha256:abc",
        "",
        crate::host::devql::RelationalDialect::Postgres,
    )
    .expect("build checkpoint session upsert SQL");
    let err = exec_checkpoint_metadata_statements(&relational, &[sql])
        .expect_err("remote shared checkpoint metadata should not fall back to local sqlite");
    assert!(
        format!("{err:#}")
            .contains("remote Postgres shared relational backend is configured without a DSN"),
        "expected remote shared metadata routing error, got: {err:#}"
    );

    let local_count = sqlite
        .with_connection(|conn| {
            conn.query_row("SELECT COUNT(*) FROM checkpoint_sessions", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(anyhow::Error::from)
        })
        .expect("count local checkpoint_sessions rows");
    assert_eq!(local_count, 0);
}

#[test]
fn commit_checkpoint_mapping_writes_route_off_local_sqlite_when_shared_role_is_remote() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let sqlite_path = tmp.path().join("checkpoint-relational.sqlite");
    let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path.clone())
        .expect("connect checkpoint sqlite");
    sqlite
        .initialise_relational_checkpoint_schema()
        .expect("initialise checkpoint schema");
    let relational = crate::host::relational_store::DefaultRelationalStore::from_inner(
        crate::host::devql::RelationalStorage::primary_backend_for_tests(
            sqlite_path,
            crate::host::devql::RelationalPrimaryBackend::Postgres,
        ),
    );

    let err = exec_checkpoint_metadata_statements(
        &relational,
        &[build_insert_commit_checkpoint_mapping_sql(
            "repo-1",
            "commit-1",
            "checkpoint-1",
        )],
    )
    .expect_err("remote shared commit-checkpoint mappings should not fall back to local sqlite");
    assert!(
        format!("{err:#}")
            .contains("remote Postgres shared relational backend is configured without a DSN"),
        "expected remote shared commit-checkpoint mapping routing error, got: {err:#}"
    );

    let local_count = sqlite
        .with_connection(|conn| {
            conn.query_row("SELECT COUNT(*) FROM commit_checkpoints", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(anyhow::Error::from)
        })
        .expect("count local commit_checkpoints rows");
    assert_eq!(local_count, 0);
}
