use super::*;

#[tokio::test]
#[ignore = "promote_temporary_current_rows_for_head_commit is not yet aligned with sync-shaped artefacts_current"]
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
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Temporary,
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

    let row: (String,) = conn
        .query_row(
            "SELECT content_id \
             FROM artefacts_current WHERE repo_id = ?1 AND symbol_id = ?2",
            rusqlite::params![cfg.repo.repo_id, file_symbol_id(path)],
            |record| Ok((record.get(0)?,)),
        )
        .expect("read current file row");
    assert_eq!(row.0, blob_sha);

    let historical_blob: String = conn
        .query_row(
            "SELECT blob_sha FROM file_state WHERE repo_id = ?1 AND commit_sha = ?2 AND path = ?3",
            rusqlite::params![cfg.repo.repo_id, new_head, path],
            |record| record.get(0),
        )
        .expect("read committed file_state row");
    assert_eq!(historical_blob, blob_sha);

    let artefact_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![cfg.repo.repo_id, path],
            |record| record.get(0),
        )
        .expect("count current artefacts");
    assert!(artefact_count > 0);
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
        &["commit", "--allow-empty", "-m", "checkpoint via DB"],
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
    assert_eq!(info.subject, "checkpoint via DB");
    assert!(info.commit_unix > 0);
}
