use super::*;

use super::helpers::init_devql_schema;

const SENTINEL_SYMBOL_ID: &str = "sentinel::symbol";
const SENTINEL_ARTEFACT_ID: &str = "sentinel::artefact";
const SENTINEL_EDGE_ID: &str = "sentinel::edge";
const SENTINEL_PATH: &str = "src/sentinel.ts";
const SENTINEL_CONTENT_ID: &str = "sentinel-content";
const ZERO_SHA: &str = "0000000000000000000000000000000000000000";

fn insert_sentinel_current_state(sqlite: &rusqlite::Connection, repo_id: &str) {
    sqlite
        .execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language, canonical_kind,
                language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id,
                start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, 'typescript', 'function', 'function', ?6, NULL, NULL,
                1, 1, 0, 1, 'sentinel()', '[]', 'sentinel row', datetime('now')
            )",
            rusqlite::params![
                repo_id,
                SENTINEL_PATH,
                SENTINEL_CONTENT_ID,
                SENTINEL_SYMBOL_ID,
                SENTINEL_ARTEFACT_ID,
                SENTINEL_SYMBOL_ID,
            ],
        )
        .expect("insert sentinel artefact row");

    sqlite
        .execute(
            "INSERT INTO artefact_edges_current (
                repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id,
                to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
                start_line, end_line, metadata, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, 'sentinel::target', 'references', 'typescript',
                1, 1, '{}', datetime('now')
            )",
            rusqlite::params![
                repo_id,
                SENTINEL_EDGE_ID,
                SENTINEL_PATH,
                SENTINEL_CONTENT_ID,
                SENTINEL_SYMBOL_ID,
                SENTINEL_ARTEFACT_ID,
            ],
        )
        .expect("insert sentinel edge row");
}

#[test]
pub(crate) fn reference_transaction_committed_local_branch_deletion_leaves_sync_current_state_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let sqlite_path = init_devql_schema(dir.path());
    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .unwrap()
        .repo_id;

    let sqlite = rusqlite::Connection::open(&sqlite_path).unwrap();
    insert_sentinel_current_state(&sqlite, &repo_id);
    drop(sqlite);

    ManualCommitStrategy::new(dir.path())
        .reference_transaction(
            "committed",
            &[format!("{head} {ZERO_SHA} refs/heads/feature/delete-me")],
        )
        .unwrap();

    let sqlite = rusqlite::Connection::open(sqlite_path).unwrap();
    let remaining_artefact_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), SENTINEL_PATH],
            |row| row.get(0),
        )
        .unwrap();
    let remaining_edge_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND edge_id = ?2",
            rusqlite::params![repo_id.as_str(), SENTINEL_EDGE_ID],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        remaining_artefact_rows, 1,
        "branch deletion should not remove sync-owned current-state artefacts"
    );
    assert_eq!(
        remaining_edge_rows, 1,
        "branch deletion should not remove sync-owned current-state edges"
    );
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
    insert_sentinel_current_state(&sqlite, &repo_id);
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
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), SENTINEL_PATH],
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
    insert_sentinel_current_state(&sqlite, &repo_id);
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
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), SENTINEL_PATH],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        remaining_rows, 1,
        "remote ref cleanup should not mutate sync-owned SQLite current state"
    );
}

#[test]
pub(crate) fn reference_transaction_deleting_parent_seed_branch_keeps_sync_current_state_rows() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let sqlite_path = init_devql_schema(dir.path());
    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .unwrap()
        .repo_id;

    let sqlite = rusqlite::Connection::open(&sqlite_path).unwrap();
    insert_sentinel_current_state(&sqlite, &repo_id);
    drop(sqlite);

    ManualCommitStrategy::new(dir.path())
        .reference_transaction(
            "committed",
            &[format!("{head} {ZERO_SHA} refs/heads/feature/parent-a")],
        )
        .unwrap();

    let sqlite = rusqlite::Connection::open(sqlite_path).unwrap();
    let current_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), SENTINEL_PATH],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        current_rows, 1,
        "reference-transaction cleanup should leave shared current-state rows untouched"
    );
}
