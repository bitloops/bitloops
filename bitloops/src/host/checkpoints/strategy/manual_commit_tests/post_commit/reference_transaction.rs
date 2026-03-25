use super::*;

use super::helpers::init_devql_schema;

const SENTINEL_SYMBOL_ID: &str = "sentinel::symbol";
const SENTINEL_ARTEFACT_ID: &str = "sentinel::artefact";
const SENTINEL_EDGE_ID: &str = "sentinel::edge";
const SENTINEL_PATH: &str = "src/sentinel.ts";
const ZERO_SHA: &str = "0000000000000000000000000000000000000000";

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
pub(crate) fn reference_transaction_committed_local_branch_deletion_cleans_current_state() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let sqlite_path = init_devql_schema(dir.path());
    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .unwrap()
        .repo_id;

    let sqlite = rusqlite::Connection::open(&sqlite_path).unwrap();
    insert_sentinel_current_state(&sqlite, &repo_id, "main", &head);
    insert_sentinel_current_state(&sqlite, &repo_id, "feature/delete-me", &head);
    drop(sqlite);

    ManualCommitStrategy::new(dir.path())
        .reference_transaction(
            "committed",
            &[format!("{head} {ZERO_SHA} refs/heads/feature/delete-me")],
        )
        .unwrap();

    let sqlite = rusqlite::Connection::open(sqlite_path).unwrap();
    let deleted_artefact_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND branch = 'feature/delete-me'",
            rusqlite::params![repo_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(deleted_artefact_rows, 0);
    let deleted_edge_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND branch = 'feature/delete-me'",
            rusqlite::params![repo_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(deleted_edge_rows, 0);

    let main_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND branch = 'main'",
            rusqlite::params![repo_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(main_rows, 1, "cleanup must not affect other branches");
}

#[test]
pub(crate) fn reference_transaction_ignores_non_committed_states() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let sqlite_path = init_devql_schema(dir.path());
    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .unwrap()
        .repo_id;

    let sqlite = rusqlite::Connection::open(&sqlite_path).unwrap();
    insert_sentinel_current_state(&sqlite, &repo_id, "feature/keep-me", &head);
    drop(sqlite);

    ManualCommitStrategy::new(dir.path())
        .reference_transaction(
            "prepared",
            &[format!("{head} {ZERO_SHA} refs/heads/feature/keep-me")],
        )
        .unwrap();

    let sqlite = rusqlite::Connection::open(sqlite_path).unwrap();
    let remaining_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND branch = 'feature/keep-me'",
            rusqlite::params![repo_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining_rows, 1, "non-committed states must be ignored");
}

#[test]
pub(crate) fn reference_transaction_remote_deletion_is_noop_without_postgres() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let sqlite_path = init_devql_schema(dir.path());
    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .unwrap()
        .repo_id;

    let sqlite = rusqlite::Connection::open(&sqlite_path).unwrap();
    insert_sentinel_current_state(&sqlite, &repo_id, "origin/feature/remote-delete", &head);
    drop(sqlite);

    ManualCommitStrategy::new(dir.path())
        .reference_transaction(
            "committed",
            &[format!(
                "{head} {ZERO_SHA} refs/remotes/origin/feature/remote-delete"
            )],
        )
        .unwrap();

    let sqlite = rusqlite::Connection::open(sqlite_path).unwrap();
    let remaining_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND branch = 'origin/feature/remote-delete'",
            rusqlite::params![repo_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        remaining_rows, 1,
        "remote ref cleanup should not mutate SQLite when Postgres is not configured"
    );
}

#[test]
pub(crate) fn reference_transaction_deleting_parent_seed_branch_keeps_child_branch_rows() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let sqlite_path = init_devql_schema(dir.path());
    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .unwrap()
        .repo_id;

    let sqlite = rusqlite::Connection::open(&sqlite_path).unwrap();
    insert_sentinel_current_state(&sqlite, &repo_id, "feature/parent-a", &head);
    insert_sentinel_current_state(&sqlite, &repo_id, "feature/child-b", &head);
    drop(sqlite);

    ManualCommitStrategy::new(dir.path())
        .reference_transaction(
            "committed",
            &[format!("{head} {ZERO_SHA} refs/heads/feature/parent-a")],
        )
        .unwrap();

    let sqlite = rusqlite::Connection::open(sqlite_path).unwrap();
    let parent_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND branch = 'feature/parent-a'",
            rusqlite::params![repo_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(parent_rows, 0, "deleted branch rows should be removed");

    let child_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND branch = 'feature/child-b'",
            rusqlite::params![repo_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        child_rows, 1,
        "deleting parent seed branch must not remove child branch current-state rows"
    );
}
