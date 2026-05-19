use rusqlite::Connection;
use serde_json::json;
use std::path::Path;
use std::time::Duration;

use super::commit::{
    execute_embedding_commit, execute_embedding_relational_commit,
    execute_embedding_runtime_finalization, execute_summary_commit,
};
use super::runtime_store::{
    open_semantic_writer_connection, open_semantic_writer_relational_connection,
    open_semantic_writer_runtime_connection,
};
use super::*;
use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::host::runtime_store::{SemanticEmbeddingMailboxItemInsert, SemanticMailboxItemKind};
use tempfile::TempDir;

#[test]
fn execute_summary_commit_reports_substage_timings_on_success() {
    let temp = TempDir::new().expect("temp dir");
    let runtime_db_path = temp.path().join("runtime.sqlite");
    let relational_db_path = temp.path().join("relational.sqlite");
    create_relational_db(&relational_db_path);
    create_runtime_db(&runtime_db_path, true, true);
    seed_summary_mailbox_row(&runtime_db_path, "lease-1", "summary-item-1");

    let mut connection = open_semantic_writer_connection(&runtime_db_path, &relational_db_path)
        .expect("open semantic writer connection");
    let report = execute_summary_commit(
        &mut connection,
        &CommitSummaryBatchRequest {
            repo: test_repo_context(temp.path()),
            lease_token: "lease-1".to_string(),
            semantic_statements: Vec::new(),
            remote_semantic_statements: Vec::new(),
            embedding_follow_ups: vec![SemanticEmbeddingMailboxItemInsert::new(
                None,
                EmbeddingRepresentationKind::Summary.to_string(),
                SemanticMailboxItemKind::Artefact,
                Some("artefact-1".to_string()),
                None,
                Some("summary:artefact-1".to_string()),
            )],
            replacement_backfill_item: None,
            acked_item_ids: vec!["summary-item-1".to_string()],
        },
    )
    .expect("commit summary batch");

    assert_eq!(
        count_rows(&runtime_db_path, "semantic_embedding_mailbox_items"),
        1
    );
    assert_eq!(
        count_rows(&runtime_db_path, "semantic_summary_mailbox_items"),
        0
    );
    assert_eq!(report.timings.summary_sql_ms, 0);
    assert_eq!(report.timings.replacement_summary_backfill_insert_ms, 0);
}

#[test]
fn execute_summary_commit_reports_embedding_upsert_failure_before_runtime_store_write() {
    let temp = TempDir::new().expect("temp dir");
    let runtime_db_path = temp.path().join("runtime.sqlite");
    let relational_db_path = temp.path().join("relational.sqlite");
    create_relational_db(&relational_db_path);
    create_runtime_db(&runtime_db_path, true, false);

    let mut connection = open_semantic_writer_connection(&runtime_db_path, &relational_db_path)
        .expect("open semantic writer connection");
    let failure = execute_summary_commit(
        &mut connection,
        &CommitSummaryBatchRequest {
            repo: test_repo_context(temp.path()),
            lease_token: "lease-1".to_string(),
            semantic_statements: Vec::new(),
            remote_semantic_statements: Vec::new(),
            embedding_follow_ups: vec![SemanticEmbeddingMailboxItemInsert::new(
                None,
                EmbeddingRepresentationKind::Summary.to_string(),
                SemanticMailboxItemKind::Artefact,
                Some("artefact-1".to_string()),
                None,
                Some("summary:artefact-1".to_string()),
            )],
            replacement_backfill_item: None,
            acked_item_ids: Vec::new(),
        },
    )
    .expect_err("summary commit should fail");

    assert_eq!(
        failure.phase(),
        SummaryCommitPhase::RuntimeEmbeddingMailboxUpsert
    );
    assert!(!failure.runtime_store_writes_succeeded_in_tx());
    assert!(format!("{:#}", failure).contains("failure_substage=runtime_embedding_mailbox_upsert"));
}

#[test]
fn execute_summary_commit_reports_delete_failure_after_runtime_store_write() {
    let temp = TempDir::new().expect("temp dir");
    let runtime_db_path = temp.path().join("runtime.sqlite");
    let relational_db_path = temp.path().join("relational.sqlite");
    create_relational_db(&relational_db_path);
    create_runtime_db(&runtime_db_path, false, true);

    let mut connection = open_semantic_writer_connection(&runtime_db_path, &relational_db_path)
        .expect("open semantic writer connection");
    let failure = execute_summary_commit(
        &mut connection,
        &CommitSummaryBatchRequest {
            repo: test_repo_context(temp.path()),
            lease_token: "lease-1".to_string(),
            semantic_statements: Vec::new(),
            remote_semantic_statements: Vec::new(),
            embedding_follow_ups: vec![SemanticEmbeddingMailboxItemInsert::new(
                None,
                EmbeddingRepresentationKind::Summary.to_string(),
                SemanticMailboxItemKind::Artefact,
                Some("artefact-1".to_string()),
                None,
                Some("summary:artefact-1".to_string()),
            )],
            replacement_backfill_item: None,
            acked_item_ids: vec!["summary-item-1".to_string()],
        },
    )
    .expect_err("summary commit should fail");

    assert_eq!(
        failure.phase(),
        SummaryCommitPhase::RuntimeSummaryMailboxDelete
    );
    assert!(failure.runtime_store_writes_succeeded_in_tx());
    assert!(
        format!("{:#}", failure).contains("deleting acknowledged semantic summary mailbox items")
    );
}

#[test]
fn semantic_writer_sqlite_locks_wait_for_runtime_db_lock() {
    let temp = TempDir::new().expect("temp dir");
    let runtime_db_path = temp.path().join("runtime.sqlite");
    let relational_db_path = temp.path().join("relational.sqlite");
    create_relational_db(&relational_db_path);
    create_runtime_db(&runtime_db_path, false, true);

    let held_lock =
        crate::storage::sqlite::hold_sqlite_write_lock_until_release(runtime_db_path.clone())
            .expect("hold runtime DB write lock");
    let runtime_db_path_for_writer = runtime_db_path.clone();
    let relational_db_path_for_writer = relational_db_path.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        started_tx
            .send(())
            .expect("signal semantic writer lock started");
        done_tx
            .send(super::actor::with_semantic_writer_sqlite_locks(
                &runtime_db_path_for_writer,
                &relational_db_path_for_writer,
                || Ok(()),
            ))
            .expect("send semantic writer lock result");
    });
    started_rx.recv().expect("wait for semantic writer start");
    assert!(
        done_rx.recv_timeout(Duration::from_millis(50)).is_err(),
        "semantic writer should not acquire locks while the runtime DB write lock is held"
    );
    held_lock.release().expect("release runtime DB write lock");
    done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("wait for semantic writer lock result")
        .expect("acquire semantic writer locks");
    worker.join().expect("join semantic writer lock worker");
}

#[test]
fn execute_embedding_commit_runtime_finalization_is_idempotent() {
    let temp = TempDir::new().expect("temp dir");
    let runtime_db_path = temp.path().join("runtime.sqlite");
    let relational_db_path = temp.path().join("relational.sqlite");
    create_relational_db(&relational_db_path);
    create_runtime_db(&runtime_db_path, false, true);
    seed_embedding_mailbox_row(
        &runtime_db_path,
        "lease-1",
        "embedding-item-1",
        "code",
        SemanticMailboxItemKind::RepoBackfill,
        Some(json!(["artefact-a", "artefact-b"])),
        Some("code:repo-backfill"),
    );

    let mut relational_connection = open_semantic_writer_relational_connection(&relational_db_path)
        .expect("open semantic embedding relational connection");
    let mut runtime_connection = open_semantic_writer_runtime_connection(&runtime_db_path)
        .expect("open semantic embedding runtime connection");
    let request = CommitEmbeddingBatchRequest {
        repo: test_repo_context(temp.path()),
        lease_token: "lease-1".to_string(),
        embedding_statements: Vec::new(),
        setup_statements: Vec::new(),
        remote_embedding_statements: Vec::new(),
        remote_setup_statements: Vec::new(),
        clone_rebuild_signal: None,
        replacement_backfill_item: Some(SemanticEmbeddingMailboxItemInsert::new(
            None,
            "code",
            SemanticMailboxItemKind::RepoBackfill,
            None,
            Some(json!(["artefact-b"])),
            Some("code:repo-backfill".to_string()),
        )),
        acked_item_ids: vec!["embedding-item-1".to_string()],
    };

    execute_embedding_commit(
        &mut relational_connection,
        &mut runtime_connection,
        &request,
    )
    .expect("first embedding commit");
    execute_embedding_commit(
        &mut relational_connection,
        &mut runtime_connection,
        &request,
    )
    .expect("second embedding commit");

    assert_eq!(
        count_rows(&runtime_db_path, "semantic_embedding_mailbox_items"),
        1
    );
    assert_eq!(
        count_matching_rows(
            &runtime_db_path,
            "semantic_embedding_mailbox_items",
            "status = 'pending' AND lease_token IS NULL",
        ),
        1
    );
    assert_eq!(
        load_embedding_payload(&runtime_db_path, "code:repo-backfill"),
        Some("[\"artefact-b\"]".to_string())
    );
}

#[test]
fn embedding_relational_commit_completes_before_runtime_finalization_lock_is_released() {
    let temp = TempDir::new().expect("temp dir");
    let runtime_db_path = temp.path().join("runtime.sqlite");
    let relational_db_path = temp.path().join("relational.sqlite");
    create_relational_db(&relational_db_path);
    Connection::open(&relational_db_path)
        .expect("open relational sqlite")
        .execute_batch("CREATE TABLE embedding_events (event_id TEXT PRIMARY KEY);")
        .expect("create embedding events table");
    create_runtime_db(&runtime_db_path, false, true);
    seed_embedding_mailbox_row(
        &runtime_db_path,
        "lease-1",
        "embedding-item-1",
        "code",
        SemanticMailboxItemKind::Artefact,
        None,
        Some("code:artefact-a"),
    );

    let held_runtime_lock =
        crate::storage::sqlite::hold_sqlite_write_lock_until_release(runtime_db_path.clone())
            .expect("hold runtime DB write lock");
    let runtime_db_path_for_worker = runtime_db_path.clone();
    let relational_db_path_for_worker = relational_db_path.clone();
    let request = CommitEmbeddingBatchRequest {
        repo: test_repo_context(temp.path()),
        lease_token: "lease-1".to_string(),
        embedding_statements: vec![
            "INSERT INTO embedding_events (event_id) VALUES ('embedding-1');".to_string(),
        ],
        setup_statements: Vec::new(),
        remote_embedding_statements: Vec::new(),
        remote_setup_statements: Vec::new(),
        clone_rebuild_signal: None,
        replacement_backfill_item: None,
        acked_item_ids: vec!["embedding-item-1".to_string()],
    };
    let (relational_committed_tx, relational_committed_rx) = std::sync::mpsc::channel();
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut relational_connection =
            open_semantic_writer_relational_connection(&relational_db_path_for_worker)
                .expect("open semantic embedding relational connection");
        let mut runtime_connection =
            open_semantic_writer_runtime_connection(&runtime_db_path_for_worker)
                .expect("open semantic embedding runtime connection");
        let result =
            crate::storage::sqlite::with_sqlite_write_lock(&relational_db_path_for_worker, || {
                execute_embedding_relational_commit(&mut relational_connection, &request)
            })
            .and_then(|()| {
                relational_committed_tx
                    .send(())
                    .expect("signal relational commit");
                crate::storage::sqlite::with_sqlite_write_lock(&runtime_db_path_for_worker, || {
                    execute_embedding_runtime_finalization(&mut runtime_connection, &request)
                })
            });
        finished_tx.send(result).expect("send worker result");
    });

    relational_committed_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("wait for relational commit");
    assert_eq!(count_rows(&relational_db_path, "embedding_events"), 1);
    assert!(
        finished_rx.recv_timeout(Duration::from_millis(50)).is_err(),
        "runtime finalization should still be waiting on the runtime lock"
    );

    held_runtime_lock
        .release()
        .expect("release runtime DB write lock");
    finished_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("wait for worker result")
        .expect("complete runtime finalization");
    worker.join().expect("join embedding commit worker");
}

fn create_relational_db(path: &Path) {
    Connection::open(path).expect("create relational sqlite");
}

fn create_runtime_db(
    path: &Path,
    include_summary_mailbox_table: bool,
    include_embedding_mailbox_table: bool,
) {
    let conn = Connection::open(path).expect("create runtime sqlite");
    if include_summary_mailbox_table {
        conn.execute_batch(
            "CREATE TABLE semantic_summary_mailbox_items (
                item_id TEXT PRIMARY KEY,
                repo_id TEXT NOT NULL,
                repo_root TEXT NOT NULL,
                config_root TEXT NOT NULL,
                init_session_id TEXT,
                item_kind TEXT NOT NULL,
                artefact_id TEXT,
                payload_json TEXT,
                dedupe_key TEXT,
                status TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                available_at_unix INTEGER NOT NULL,
                submitted_at_unix INTEGER NOT NULL,
                leased_at_unix INTEGER,
                lease_expires_at_unix INTEGER,
                lease_token TEXT,
                updated_at_unix INTEGER NOT NULL,
                last_error TEXT
            );",
        )
        .expect("create summary mailbox table");
    }
    if include_embedding_mailbox_table {
        conn.execute_batch(
            "CREATE TABLE semantic_embedding_mailbox_items (
                item_id TEXT PRIMARY KEY,
                repo_id TEXT NOT NULL,
                repo_root TEXT NOT NULL,
                config_root TEXT NOT NULL,
                init_session_id TEXT,
                representation_kind TEXT NOT NULL,
                item_kind TEXT NOT NULL,
                artefact_id TEXT,
                payload_json TEXT,
                dedupe_key TEXT,
                status TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                available_at_unix INTEGER NOT NULL,
                submitted_at_unix INTEGER NOT NULL,
                leased_at_unix INTEGER,
                lease_expires_at_unix INTEGER,
                lease_token TEXT,
                updated_at_unix INTEGER NOT NULL,
                last_error TEXT
            );",
        )
        .expect("create embedding mailbox table");
    }
}

fn seed_summary_mailbox_row(path: &Path, lease_token: &str, item_id: &str) {
    let conn = Connection::open(path).expect("open runtime sqlite");
    conn.execute(
        "INSERT INTO semantic_summary_mailbox_items (
            item_id, repo_id, repo_root, config_root, init_session_id, item_kind,
            artefact_id, payload_json, dedupe_key, status, attempts, available_at_unix,
            submitted_at_unix, leased_at_unix, lease_expires_at_unix, lease_token,
            updated_at_unix, last_error
         ) VALUES (
            ?1, ?2, ?3, ?4, NULL, ?5,
            ?6, NULL, ?7, 'leased', 1, ?8,
            ?9, ?10, ?11, ?12,
            ?13, NULL
         )",
        rusqlite::params![
            item_id,
            "repo-1",
            "/tmp/repo",
            "/tmp/config",
            SemanticMailboxItemKind::Artefact.as_str(),
            "artefact-1",
            "summary:artefact-1",
            1_i64,
            1_i64,
            1_i64,
            2_i64,
            lease_token,
            1_i64,
        ],
    )
    .expect("seed summary mailbox row");
}

fn seed_embedding_mailbox_row(
    path: &Path,
    lease_token: &str,
    item_id: &str,
    representation_kind: &str,
    item_kind: SemanticMailboxItemKind,
    payload_json: Option<serde_json::Value>,
    dedupe_key: Option<&str>,
) {
    let conn = Connection::open(path).expect("open runtime sqlite");
    conn.execute(
        "INSERT INTO semantic_embedding_mailbox_items (
            item_id, repo_id, repo_root, config_root, init_session_id, representation_kind,
            item_kind, artefact_id, payload_json, dedupe_key, status, attempts,
            available_at_unix, submitted_at_unix, leased_at_unix, lease_expires_at_unix,
            lease_token, updated_at_unix, last_error
         ) VALUES (
            ?1, ?2, ?3, ?4, NULL, ?5,
            ?6, NULL, ?7, ?8, 'leased', 1,
            ?9, ?10, ?11, ?12,
            ?13, ?14, NULL
         )",
        rusqlite::params![
            item_id,
            "repo-1",
            "/tmp/repo",
            "/tmp/config",
            representation_kind,
            item_kind.as_str(),
            payload_json.map(|value| value.to_string()),
            dedupe_key,
            1_i64,
            1_i64,
            1_i64,
            2_i64,
            lease_token,
            1_i64,
        ],
    )
    .expect("seed embedding mailbox row");
}

fn count_rows(path: &Path, table: &str) -> i64 {
    let conn = Connection::open(path).expect("open runtime sqlite");
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .expect("count rows")
}

fn count_matching_rows(path: &Path, table: &str, predicate: &str) -> i64 {
    let conn = Connection::open(path).expect("open sqlite");
    conn.query_row(
        &format!("SELECT COUNT(*) FROM {table} WHERE {predicate}"),
        [],
        |row| row.get(0),
    )
    .expect("count matching rows")
}

fn load_embedding_payload(path: &Path, dedupe_key: &str) -> Option<String> {
    let conn = Connection::open(path).expect("open runtime sqlite");
    conn.query_row(
        "SELECT payload_json
         FROM semantic_embedding_mailbox_items
         WHERE dedupe_key = ?1
         ORDER BY submitted_at_unix ASC
         LIMIT 1",
        [dedupe_key],
        |row| row.get(0),
    )
    .ok()
}

fn test_repo_context(root: &Path) -> SemanticBatchRepoContext {
    SemanticBatchRepoContext {
        repo_id: "repo-1".to_string(),
        repo_root: root.join("repo"),
        config_root: root.join("config"),
    }
}
