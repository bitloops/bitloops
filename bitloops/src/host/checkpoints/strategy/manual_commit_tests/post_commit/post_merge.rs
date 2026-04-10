use super::*;

use super::helpers::{commit_file, init_devql_schema};

#[test]
pub(crate) fn post_merge_refreshes_devql_current_state_for_merged_files() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let devql_sqlite_path = init_devql_schema(dir.path());
    let primary_branch = run_git(dir.path(), &["branch", "--show-current"]).unwrap();

    fs::create_dir_all(dir.path().join("src")).unwrap();
    commit_file(
        dir.path(),
        "src/base.ts",
        "export const base = (value: number) => value;\n",
    );
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    git_ok(
        dir.path(),
        &["checkout", "-b", "feature/post-merge-refresh"],
    );
    commit_file(
        dir.path(),
        "src/post_merge.ts",
        "export const mergedValue = (value: number) => value + 1;\n",
    );

    git_ok(dir.path(), &["checkout", &primary_branch]);
    git_ok(
        dir.path(),
        &["merge", "--ff-only", "feature/post-merge-refresh"],
    );
    let _merged_head = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
    let merged_blob = run_git(dir.path(), &["rev-parse", "HEAD:src/post_merge.ts"]).unwrap();
    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .unwrap()
        .repo_id;

    ManualCommitStrategy::new(dir.path())
        .post_merge(false)
        .unwrap();

    let sqlite = rusqlite::Connection::open(devql_sqlite_path).unwrap();
    let indexed_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), "src/post_merge.ts"],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        indexed_rows > 0,
        "post_merge should refresh sync-owned current-state artefacts for merged files"
    );

    let current_state: (String, String) = sqlite
        .query_row(
            "SELECT effective_content_id, effective_source FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), "src/post_merge.ts"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(current_state.0, merged_blob);
    assert_eq!(current_state.1, "head");

    let current_branch = run_git(dir.path(), &["branch", "--show-current"]).unwrap();
    assert_eq!(current_branch, primary_branch);
}

#[test]
pub(crate) fn post_merge_refresh_removes_devql_current_state_for_deleted_files() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let devql_sqlite_path = init_devql_schema(dir.path());
    let primary_branch = run_git(dir.path(), &["branch", "--show-current"]).unwrap();

    fs::create_dir_all(dir.path().join("src")).unwrap();
    commit_file(
        dir.path(),
        "src/remove_on_merge.ts",
        "export const removeOnMerge = () => 'remove';\n",
    );
    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .unwrap()
        .repo_id;
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let sqlite = rusqlite::Connection::open(&devql_sqlite_path).unwrap();
    let before_delete: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), "src/remove_on_merge.ts"],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        before_delete > 0,
        "expected indexed file before merge deletion"
    );
    drop(sqlite);

    git_ok(dir.path(), &["checkout", "-b", "feature/post-merge-delete"]);
    fs::remove_file(dir.path().join("src/remove_on_merge.ts")).unwrap();
    git_ok(dir.path(), &["add", "-A"]);
    git_ok(dir.path(), &["commit", "-m", "delete file on feature"]);

    git_ok(dir.path(), &["checkout", &primary_branch]);
    git_ok(
        dir.path(),
        &["merge", "--ff-only", "feature/post-merge-delete"],
    );

    ManualCommitStrategy::new(dir.path())
        .post_merge(false)
        .unwrap();

    let sqlite = rusqlite::Connection::open(devql_sqlite_path).unwrap();
    let after_delete: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), "src/remove_on_merge.ts"],
            |row| row.get(0),
        )
        .unwrap();
    let file_state_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), "src/remove_on_merge.ts"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        after_delete, 0,
        "post_merge should remove deleted file rows from sync-owned current state"
    );
    assert_eq!(
        file_state_rows, 0,
        "deleted merge paths should be removed from current_file_state"
    );
}
