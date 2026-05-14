use rusqlite::Connection;
use std::path::Path;

use super::commit::execute_summary_commit;
use super::runtime_store::open_semantic_writer_connection;
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

fn count_rows(path: &Path, table: &str) -> i64 {
    let conn = Connection::open(path).expect("open runtime sqlite");
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .expect("count rows")
}

fn test_repo_context(root: &Path) -> SemanticBatchRepoContext {
    SemanticBatchRepoContext {
        repo_id: "repo-1".to_string(),
        repo_root: root.join("repo"),
        config_root: root.join("config"),
    }
}
