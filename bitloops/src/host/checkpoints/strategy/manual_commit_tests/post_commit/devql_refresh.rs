use super::*;

use super::helpers::{commit_file, init_devql_schema};

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
