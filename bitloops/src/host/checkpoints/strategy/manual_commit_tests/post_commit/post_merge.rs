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
    let merged_head = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    ManualCommitStrategy::new(dir.path())
        .post_merge(false)
        .unwrap();

    let sqlite = rusqlite::Connection::open(devql_sqlite_path).unwrap();
    let current_branch = run_git(dir.path(), &["branch", "--show-current"]).unwrap();
    let indexed_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE path = ?1 AND commit_sha = ?2 AND branch = ?3 AND revision_kind = 'commit'",
            rusqlite::params!["src/post_merge.ts", merged_head.as_str(), current_branch.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        indexed_rows > 0,
        "post_merge should index files changed by pull/merge into artefacts_current"
    );

    let commit_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commits WHERE commit_sha = ?1",
            rusqlite::params![merged_head.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        commit_rows, 1,
        "post_merge should upsert commit metadata in DevQL commits table"
    );
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
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let sqlite = rusqlite::Connection::open(&devql_sqlite_path).unwrap();
    let before_delete: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE path = ?1 AND branch = ?2",
            rusqlite::params!["src/remove_on_merge.ts", primary_branch.as_str()],
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
    let current_branch = run_git(dir.path(), &["branch", "--show-current"]).unwrap();
    let after_delete: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE path = ?1 AND branch = ?2",
            rusqlite::params!["src/remove_on_merge.ts", current_branch.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        after_delete, 0,
        "post_merge should remove deleted file rows from branch-scoped current state"
    );
}
