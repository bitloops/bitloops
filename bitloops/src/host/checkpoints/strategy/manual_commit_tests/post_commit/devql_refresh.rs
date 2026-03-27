use super::*;

use super::helpers::{commit_file, init_devql_schema};

#[test]
pub(crate) fn post_commit_projects_checkpoint_file_snapshots_for_committed_checkpoints() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let devql_sqlite_path = init_devql_schema(dir.path());

    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "projection-session".to_string(),
            phase: SessionPhase::Idle,
            base_commit: head,
            step_count: 1,
            agent_type: "claude-code".to_string(),
            files_touched: vec![
                "src/projection_a.ts".to_string(),
                "src/projection_b.ts".to_string(),
                "src/projection_missing.ts".to_string(),
            ],
            ..Default::default()
        })
        .unwrap();

    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("src/projection_a.ts"),
        "export const projectionA = () => 1;\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("src/projection_b.ts"),
        "export const projectionB = () => 2;\n",
    )
    .unwrap();
    git_ok(dir.path(), &["add", "."]);
    git_ok(dir.path(), &["commit", "-m", "project snapshot rows"]);
    let head_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    strategy.post_commit().unwrap();

    let checkpoint_id = read_commit_checkpoint_mappings(dir.path())
        .unwrap()
        .get(&head_sha)
        .cloned()
        .expect("post_commit should map the commit to a checkpoint");
    let blob_a = run_git(
        dir.path(),
        &["rev-parse", &format!("{head_sha}:src/projection_a.ts")],
    )
    .unwrap();
    let blob_b = run_git(
        dir.path(),
        &["rev-parse", &format!("{head_sha}:src/projection_b.ts")],
    )
    .unwrap();

    let sqlite = rusqlite::Connection::open(&devql_sqlite_path).unwrap();
    let projected_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM checkpoint_file_snapshots
             WHERE checkpoint_id = ?1 AND commit_sha = ?2",
            rusqlite::params![checkpoint_id.as_str(), head_sha.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        projected_rows, 2,
        "post_commit should project one snapshot row per resolved touched file"
    );

    let projection_a_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM checkpoint_file_snapshots
             WHERE checkpoint_id = ?1 AND path = ?2 AND blob_sha = ?3",
            rusqlite::params![
                checkpoint_id.as_str(),
                "src/projection_a.ts",
                blob_a.as_str()
            ],
            |row| row.get(0),
        )
        .unwrap();
    let projection_b_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM checkpoint_file_snapshots
             WHERE checkpoint_id = ?1 AND path = ?2 AND blob_sha = ?3",
            rusqlite::params![
                checkpoint_id.as_str(),
                "src/projection_b.ts",
                blob_b.as_str()
            ],
            |row| row.get(0),
        )
        .unwrap();
    let missing_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM checkpoint_file_snapshots
             WHERE checkpoint_id = ?1 AND path = ?2",
            rusqlite::params![checkpoint_id.as_str(), "src/projection_missing.ts"],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(
        projection_a_rows, 1,
        "expected projection row for first file"
    );
    assert_eq!(
        projection_b_rows, 1,
        "expected projection row for second file"
    );
    assert_eq!(
        missing_rows, 0,
        "unresolvable touched files should be skipped from the projection"
    );
    drop(sqlite);

    strategy.post_commit().unwrap();

    let sqlite = rusqlite::Connection::open(devql_sqlite_path).unwrap();
    let replayed_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM checkpoint_file_snapshots
             WHERE checkpoint_id = ?1 AND commit_sha = ?2",
            rusqlite::params![checkpoint_id.as_str(), head_sha.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        replayed_rows, 2,
        "replaying post_commit for the same mapped commit must stay idempotent"
    );
}

#[test]
pub(crate) fn post_commit_refreshes_devql_current_state_for_changed_files() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let devql_sqlite_path = init_devql_schema(dir.path());

    fs::create_dir_all(dir.path().join("src")).unwrap();
    commit_file(
        dir.path(),
        "src/post_commit.ts",
        "export function run(value: number) { return value + 1; }\n",
    );
    let head_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let sqlite = rusqlite::Connection::open(devql_sqlite_path).unwrap();
    let branch = run_git(dir.path(), &["branch", "--show-current"]).unwrap();
    let indexed_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE path = ?1 AND commit_sha = ?2 AND branch = ?3 AND revision_kind = 'commit'",
            rusqlite::params!["src/post_commit.ts", head_sha.as_str(), branch.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        indexed_rows > 0,
        "post_commit should index changed files into artefacts_current"
    );

    let commit_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commits WHERE commit_sha = ?1",
            rusqlite::params![head_sha.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        commit_rows, 1,
        "post_commit should upsert commit metadata in DevQL commits table"
    );
}

#[test]
pub(crate) fn post_commit_refresh_removes_devql_current_state_for_deleted_files() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let devql_sqlite_path = init_devql_schema(dir.path());

    fs::create_dir_all(dir.path().join("src")).unwrap();
    commit_file(
        dir.path(),
        "src/remove_me.ts",
        "export const removeMe = () => 'remove';\n",
    );
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let sqlite_path = devql_sqlite_path;
    let sqlite = rusqlite::Connection::open(&sqlite_path).unwrap();
    let branch = run_git(dir.path(), &["branch", "--show-current"]).unwrap();
    let before_delete: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE path = ?1 AND branch = ?2",
            rusqlite::params!["src/remove_me.ts", branch.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(before_delete > 0, "expected indexed file before deletion");
    drop(sqlite);

    fs::remove_file(dir.path().join("src/remove_me.ts")).unwrap();
    git_ok(dir.path(), &["add", "-A"]);
    git_ok(dir.path(), &["commit", "-m", "delete file"]);

    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let sqlite = rusqlite::Connection::open(&sqlite_path).unwrap();
    let after_delete: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE path = ?1 AND branch = ?2",
            rusqlite::params!["src/remove_me.ts", branch.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        after_delete, 0,
        "post_commit should remove deleted file rows from branch-scoped current state"
    );
}

#[test]
pub(crate) fn post_commit_on_feature_branch_preserves_main_branch_current_state() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let devql_sqlite_path = init_devql_schema(dir.path());

    fs::create_dir_all(dir.path().join("src")).unwrap();
    commit_file(
        dir.path(),
        "src/shared.ts",
        "export const shared = (value: number) => value + 1;\n",
    );
    let main_branch = run_git(dir.path(), &["branch", "--show-current"]).unwrap();
    let main_head = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    strategy.post_commit().unwrap();

    git_ok(dir.path(), &["checkout", "-b", "feature/branch-isolation"]);
    fs::write(
        dir.path().join("src/shared.ts"),
        "export const shared = (value: number) => value + 2;\n",
    )
    .unwrap();
    git_ok(dir.path(), &["add", "src/shared.ts"]);
    git_ok(dir.path(), &["commit", "-m", "feature change"]);
    let feature_branch = run_git(dir.path(), &["branch", "--show-current"]).unwrap();
    let feature_head = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    strategy.post_commit().unwrap();

    let sqlite = rusqlite::Connection::open(&devql_sqlite_path).unwrap();
    let feature_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE path = ?1 AND branch = ?2 AND commit_sha = ?3 AND revision_kind = 'commit'",
            rusqlite::params!["src/shared.ts", feature_branch.as_str(), feature_head.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        feature_rows > 0,
        "feature branch should be indexed at feature HEAD after post_commit"
    );

    let main_rows_at_main_head: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE path = ?1 AND branch = ?2 AND commit_sha = ?3 AND revision_kind = 'commit'",
            rusqlite::params!["src/shared.ts", main_branch.as_str(), main_head.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        main_rows_at_main_head > 0,
        "feature indexing should not overwrite main branch current-state rows"
    );

    let leaked_main_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE path = ?1 AND branch = ?2 AND commit_sha = ?3",
            rusqlite::params!["src/shared.ts", main_branch.as_str(), feature_head.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        leaked_main_rows, 0,
        "main branch should not receive feature commit rows"
    );
    drop(sqlite);

    git_ok(dir.path(), &["checkout", &main_branch]);
    strategy
        .post_checkout(&feature_head, &main_head, true)
        .unwrap();

    let sqlite = rusqlite::Connection::open(devql_sqlite_path).unwrap();
    let main_rows_after_switch: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE path = ?1 AND branch = ?2 AND commit_sha = ?3",
            rusqlite::params!["src/shared.ts", main_branch.as_str(), main_head.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        main_rows_after_switch > 0,
        "switching back to main should keep the existing indexed main rows"
    );
}
