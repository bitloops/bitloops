use super::*;

use super::helpers::{commit_file, init_devql_schema};

#[test]
pub(crate) fn post_checkout_same_head_replays_sync_without_duplicate_current_state() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let devql_sqlite_path = init_devql_schema(dir.path());

    fs::create_dir_all(dir.path().join("src")).unwrap();
    commit_file(
        dir.path(),
        "src/shared.ts",
        "export const shared = (value: number) => value + 1;\n",
    );
    let main_head = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .unwrap()
        .repo_id;
    let sqlite = rusqlite::Connection::open(&devql_sqlite_path).unwrap();
    let before_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), "src/shared.ts"],
            |row| row.get(0),
        )
        .unwrap();
    let before_state_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), "src/shared.ts"],
            |row| row.get(0),
        )
        .unwrap();
    drop(sqlite);

    git_ok(dir.path(), &["checkout", "-b", "feature/seed-copy"]);
    ManualCommitStrategy::new(dir.path())
        .post_checkout(&main_head, &main_head, true)
        .unwrap();

    let sqlite = rusqlite::Connection::open(devql_sqlite_path).unwrap();
    let after_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), "src/shared.ts"],
            |row| row.get(0),
        )
        .unwrap();
    let after_state_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), "src/shared.ts"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        after_rows, before_rows,
        "same-head post-checkout should leave sync-owned current-state rows stable"
    );
    assert_eq!(
        after_state_rows, before_state_rows,
        "same-head post-checkout should remain idempotent for current_file_state"
    );
}

#[test]
pub(crate) fn post_checkout_repeated_sync_keeps_existing_current_state_stable() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let devql_sqlite_path = init_devql_schema(dir.path());

    fs::create_dir_all(dir.path().join("src")).unwrap();
    commit_file(
        dir.path(),
        "src/shared.ts",
        "export const shared = (value: number) => value + 1;\n",
    );
    let main_head = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    git_ok(dir.path(), &["checkout", "-b", "feature/already-indexed"]);
    let strategy = ManualCommitStrategy::new(dir.path());
    strategy
        .post_checkout(&main_head, &main_head, true)
        .unwrap();

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .unwrap()
        .repo_id;
    let sqlite = rusqlite::Connection::open(&devql_sqlite_path).unwrap();
    let before_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), "src/shared.ts"],
            |row| row.get(0),
        )
        .unwrap();
    let before_state_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), "src/shared.ts"],
            |row| row.get(0),
        )
        .unwrap();
    drop(sqlite);

    strategy
        .post_checkout(&main_head, &main_head, true)
        .unwrap();

    let sqlite = rusqlite::Connection::open(devql_sqlite_path).unwrap();
    let after_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), "src/shared.ts"],
            |row| row.get(0),
        )
        .unwrap();
    let after_state_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), "src/shared.ts"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        after_rows, before_rows,
        "re-running post-checkout sync should not duplicate current artefact rows"
    );
    assert_eq!(
        after_state_rows, before_state_rows,
        "re-running post-checkout sync should not duplicate current_file_state rows"
    );
}

#[test]
pub(crate) fn post_checkout_first_visit_to_diverged_branch_indexes_head_tree() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let devql_sqlite_path = init_devql_schema(dir.path());

    fs::create_dir_all(dir.path().join("src")).unwrap();
    commit_file(
        dir.path(),
        "src/base.ts",
        "export const base = (value: number) => value;\n",
    );
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();
    let main_head = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    git_ok(dir.path(), &["checkout", "-b", "feature/diverged-seed"]);
    commit_file(
        dir.path(),
        "src/feature_only.ts",
        "export const featureOnly = (value: number) => value * 2;\n",
    );
    let feature_head = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
    let feature_blob = run_git(dir.path(), &["rev-parse", "HEAD:src/feature_only.ts"]).unwrap();
    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .unwrap()
        .repo_id;

    let sqlite = rusqlite::Connection::open(&devql_sqlite_path).unwrap();
    let before_feature_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), "src/feature_only.ts"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        before_feature_rows, 0,
        "test precondition: feature-only path should not be materialized before post-checkout sync"
    );
    drop(sqlite);

    ManualCommitStrategy::new(dir.path())
        .post_checkout(&main_head, &feature_head, true)
        .unwrap();

    let sqlite = rusqlite::Connection::open(devql_sqlite_path).unwrap();
    let indexed_feature_file_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), "src/feature_only.ts"],
            |row| row.get(0),
        )
        .unwrap();
    let current_state: (String, String) = sqlite
        .query_row(
            "SELECT effective_content_id, effective_source FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), "src/feature_only.ts"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(
        indexed_feature_file_rows > 0,
        "post-checkout should sync the diverged branch HEAD into current-state tables"
    );
    assert_eq!(current_state.0, feature_blob);
    assert_eq!(current_state.1, "head");
}
