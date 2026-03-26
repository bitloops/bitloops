use super::*;

use super::helpers::{commit_file, init_devql_schema};

const SENTINEL_SYMBOL_ID: &str = "sentinel::symbol";
const SENTINEL_ARTEFACT_ID: &str = "sentinel::artefact";
const SENTINEL_EDGE_ID: &str = "sentinel::edge";
const SENTINEL_PATH: &str = "src/sentinel.ts";

fn insert_sentinel_current_state(
    sqlite: &rusqlite::Connection,
    repo_id: &str,
    branch: &str,
    commit_sha: &str,
) {
    sqlite
        .execute(
            "INSERT INTO artefacts_current (
                repo_id, branch, symbol_id, artefact_id, commit_sha, revision_kind, revision_id, temp_checkpoint_id,
                blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id,
                start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, 'temporary', 'temp:sentinel', NULL,
                'blob-sentinel', ?6, 'typescript', 'function', 'function', ?7, NULL, NULL,
                1, 1, 0, 1, 'sentinel()', '[]', 'sentinel row', 'hash-sentinel', datetime('now')
            )",
            rusqlite::params![
                repo_id,
                branch,
                SENTINEL_SYMBOL_ID,
                SENTINEL_ARTEFACT_ID,
                commit_sha,
                SENTINEL_PATH,
                SENTINEL_SYMBOL_ID
            ],
        )
        .expect("insert sentinel artefact row");

    sqlite
        .execute(
            "INSERT INTO artefact_edges_current (
                edge_id, repo_id, branch, commit_sha, revision_kind, revision_id, temp_checkpoint_id, blob_sha, path,
                from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
                start_line, end_line, metadata, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, 'temporary', 'temp:sentinel', NULL, 'blob-sentinel', ?5,
                ?6, ?7, NULL, NULL, 'sentinel::target', 'references', 'typescript',
                1, 1, '{}', datetime('now')
            )",
            rusqlite::params![
                SENTINEL_EDGE_ID,
                repo_id,
                branch,
                commit_sha,
                SENTINEL_PATH,
                SENTINEL_SYMBOL_ID,
                SENTINEL_ARTEFACT_ID
            ],
        )
        .expect("insert sentinel edge row");
}

#[test]
pub(crate) fn post_checkout_same_head_seeds_branch_by_copying_current_state_rows() {
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
    let main_branch = run_git(dir.path(), &["branch", "--show-current"]).unwrap();
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .unwrap()
        .repo_id;
    let sqlite = rusqlite::Connection::open(&devql_sqlite_path).unwrap();
    insert_sentinel_current_state(&sqlite, &repo_id, &main_branch, &main_head);
    drop(sqlite);

    git_ok(dir.path(), &["checkout", "-b", "feature/seed-copy"]);
    let feature_branch = run_git(dir.path(), &["branch", "--show-current"]).unwrap();
    ManualCommitStrategy::new(dir.path())
        .post_checkout(&main_head, &main_head, true)
        .unwrap();

    let sqlite = rusqlite::Connection::open(devql_sqlite_path).unwrap();
    let copied_symbol_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND branch = ?2 AND symbol_id = ?3",
            rusqlite::params![repo_id.as_str(), feature_branch.as_str(), SENTINEL_SYMBOL_ID],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        copied_symbol_rows, 1,
        "same-head post-checkout should copy current-state artefacts into the new branch"
    );

    let copied_edge_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND branch = ?2 AND edge_id = ?3",
            rusqlite::params![repo_id.as_str(), feature_branch.as_str(), SENTINEL_EDGE_ID],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        copied_edge_rows, 1,
        "same-head post-checkout should copy current-state edges into the new branch"
    );
}

#[test]
pub(crate) fn post_checkout_skips_when_target_branch_already_has_current_state() {
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
    let main_branch = run_git(dir.path(), &["branch", "--show-current"]).unwrap();
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    git_ok(dir.path(), &["checkout", "-b", "feature/already-indexed"]);
    let feature_branch = run_git(dir.path(), &["branch", "--show-current"]).unwrap();
    let strategy = ManualCommitStrategy::new(dir.path());
    strategy
        .post_checkout(&main_head, &main_head, true)
        .unwrap();

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .unwrap()
        .repo_id;
    git_ok(dir.path(), &["checkout", &main_branch]);
    let sqlite = rusqlite::Connection::open(&devql_sqlite_path).unwrap();
    insert_sentinel_current_state(&sqlite, &repo_id, &main_branch, &main_head);
    drop(sqlite);

    git_ok(dir.path(), &["checkout", &feature_branch]);
    strategy
        .post_checkout(&main_head, &main_head, true)
        .unwrap();

    let sqlite = rusqlite::Connection::open(devql_sqlite_path).unwrap();
    let leaked_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND branch = ?2 AND symbol_id = ?3",
            rusqlite::params![repo_id.as_str(), feature_branch.as_str(), SENTINEL_SYMBOL_ID],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        leaked_rows, 0,
        "post-checkout should do no work for already-indexed branches"
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
    let feature_branch = run_git(dir.path(), &["branch", "--show-current"]).unwrap();

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .unwrap()
        .repo_id;
    let sqlite = rusqlite::Connection::open(&devql_sqlite_path).unwrap();
    let before_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND branch = ?2",
            rusqlite::params![repo_id.as_str(), feature_branch.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        before_rows, 0,
        "test precondition: diverged branch should start without indexed current-state rows"
    );
    drop(sqlite);

    ManualCommitStrategy::new(dir.path())
        .post_checkout(&main_head, &feature_head, true)
        .unwrap();

    let sqlite = rusqlite::Connection::open(devql_sqlite_path).unwrap();
    let indexed_feature_file_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND branch = ?2 AND path = ?3 AND commit_sha = ?4 AND revision_kind = 'commit'",
            rusqlite::params![
                repo_id.as_str(),
                feature_branch.as_str(),
                "src/feature_only.ts",
                feature_head.as_str()
            ],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        indexed_feature_file_rows > 0,
        "post-checkout should index the diverged branch HEAD when no current-state rows exist"
    );
}
