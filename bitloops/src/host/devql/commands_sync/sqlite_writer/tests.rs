use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;

use super::*;

fn writer_test_desired_file_state(
    path: &str,
    content_id: &str,
) -> crate::host::devql::sync::types::DesiredFileState {
    crate::host::devql::sync::types::DesiredFileState {
        path: path.to_string(),
        analysis_mode: crate::host::devql::AnalysisMode::Code,
        file_role: crate::host::devql::FileRole::SourceCode,
        text_index_mode: crate::host::devql::TextIndexMode::None,
        language: "python".to_string(),
        resolved_language: "python".to_string(),
        dialect: None,
        primary_context_id: None,
        secondary_context_ids: Vec::new(),
        frameworks: Vec::new(),
        runtime_profile: None,
        classification_reason: "sync_test".to_string(),
        context_fingerprint: None,
        extraction_fingerprint: format!("sync-test::{path}::{content_id}"),
        head_content_id: Some(content_id.to_string()),
        index_content_id: Some(content_id.to_string()),
        worktree_content_id: Some(content_id.to_string()),
        effective_content_id: content_id.to_string(),
        effective_source: crate::host::devql::sync::types::EffectiveSource::Worktree,
        exists_in_head: true,
        exists_in_index: true,
        exists_in_worktree: true,
    }
}

fn writer_test_prepared_item(index: usize, path: &str) -> PreparedSyncItem {
    let content_id = format!("content::{path}");
    PreparedSyncItem {
        index,
        desired: writer_test_desired_file_state(path, &content_id),
        extraction: crate::host::devql::sync::content_cache::CachedExtraction {
            content_id: content_id.clone(),
            language: "python".to_string(),
            extraction_fingerprint: format!("sync-test::{path}::{content_id}"),
            parser_version: "parser-version".to_string(),
            extractor_version: "extractor-version".to_string(),
            parse_status: crate::host::devql::sync::extraction::PARSE_STATUS_OK.to_string(),
            artefacts: Vec::new(),
            edges: Vec::new(),
        },
        prepared_rows: PreparedMaterialisationRows {
            materialized_artefacts: Vec::new(),
            materialized_edges: Vec::new(),
        },
        cache_store_retention_class: None,
        cache_touch_key: None,
        promote_cache_entry_to_git_backed: false,
    }
}

#[test]
fn sync_prepare_worker_count_is_bounded() {
    let count = sync_prepare_worker_count();
    assert!((2..=8).contains(&count), "worker count should be clamped");
}

#[test]
fn open_sync_sqlite_connection_reports_missing_database_file() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let path = dir.path().join("missing.sqlite");
    let err = open_sync_sqlite_connection(&path).expect_err("missing sqlite file should error");
    let message = format!("{err:#}");
    assert!(message.contains("SQLite database file not found"));
    assert!(message.contains("bitloops init"));
}

#[test]
fn sqlite_read_pool_checkout_exhausts_then_recovers_after_drop() {
    let pool = SqliteReadConnectionPool {
        connections: Arc::new(Mutex::new(vec![
            Connection::open_in_memory().expect("open in-memory sqlite"),
        ])),
    };

    let first = pool.checkout().expect("first checkout");
    assert!(
        pool.checkout().is_err(),
        "pool should be exhausted while connection is checked out"
    );
    first
        .connection()
        .execute("CREATE TABLE demo(id INTEGER PRIMARY KEY)", [])
        .expect("execute query on checked-out connection");
    drop(first);

    let second = pool
        .checkout()
        .expect("connection should be returned to pool on drop");
    let value: i64 = second
        .connection()
        .query_row("SELECT 1", [], |row| row.get(0))
        .expect("execute query after connection recycle");
    assert_eq!(value, 1);
}

#[test]
fn sync_batch_default_is_empty_and_has_no_deadline() {
    let batch = SyncBatch::default();
    assert!(batch.is_empty());
    assert!(!batch.should_flush());
    assert!(batch.flush_deadline().is_none());
}

#[tokio::test]
async fn sqlite_sync_writer_flush_waits_for_transient_write_lock() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let sqlite_path = dir.path().join("devql.sqlite");
    let repo_id = "repo-id";
    crate::host::devql::init_sqlite_schema(&sqlite_path)
        .await
        .expect("initialise sqlite schema");
    let seed_connection = open_sync_sqlite_connection(&sqlite_path).expect("open seed connection");
    seed_connection
        .execute(
            "INSERT INTO repositories (repo_id, provider, organization, name, default_branch) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![repo_id, "github", "bitloops", "sqlite-writer-test", "main"],
        )
        .expect("seed repository row");

    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let sqlite_path_for_blocker = sqlite_path.clone();
    let blocker = std::thread::spawn(move || {
        let mut connection =
            open_sync_sqlite_connection(&sqlite_path_for_blocker).expect("open blocker");
        connection
            .execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")
            .expect("configure blocker connection");
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .expect("start blocker transaction");
        ready_tx.send(()).expect("signal blocker ready");
        std::thread::sleep(Duration::from_millis(150));
        tx.commit().expect("commit blocker transaction");
    });
    ready_rx.recv().expect("wait for blocker readiness");

    let mut writer = SqliteSyncWriter::open(&sqlite_path)
        .await
        .expect("open sqlite sync writer");
    writer.push_item(writer_test_prepared_item(
        0,
        "crates/red_knot_vendored/vendor/typeshed/stdlib/shlex.pyi",
    ));

    writer
        .flush(repo_id, "parser-version", "extractor-version")
        .await
        .expect("flush should wait for transient write lock");

    blocker.join().expect("join blocker thread");
}

#[tokio::test]
async fn sqlite_sync_writer_finish_chunks_large_touch_sets() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let sqlite_path = dir.path().join("devql.sqlite");
    crate::host::devql::init_sqlite_schema(&sqlite_path)
        .await
        .expect("initialise sqlite schema");
    let seed_connection = open_sync_sqlite_connection(&sqlite_path).expect("open seed connection");
    for index in 0..1_001 {
        seed_connection
            .execute(
                "INSERT INTO content_cache (
                    content_id, language, extraction_fingerprint, parser_version,
                    extractor_version, retention_class, parse_status, parsed_at,
                    last_accessed_at
                ) VALUES (
                    ?1, 'python', ?2, 'parser-version', 'extractor-version',
                    'worktree_only', 'ok', datetime('now'), datetime('now')
                )",
                rusqlite::params![format!("content::{index}"), format!("fingerprint::{index}"),],
            )
            .expect("seed cache entry");
    }

    let mut writer = SqliteSyncWriter::open(&sqlite_path)
        .await
        .expect("open sqlite sync writer");
    for index in 0..1_001 {
        let mut item = writer_test_prepared_item(index, &format!("src/generated_{index}.py"));
        item.cache_touch_key = Some(CacheKey {
            content_id: format!("content::{index}"),
            language: "python".to_string(),
            extraction_fingerprint: format!("fingerprint::{index}"),
            parser_version: "parser-version".to_string(),
            extractor_version: "extractor-version".to_string(),
        });
        writer.push_item(item);
    }

    let outcome = writer.finish().await.expect("finish cache touches");
    assert_eq!(outcome.sqlite_rows_written, 1_001);
    assert!(
        outcome.sqlite_commits > 1,
        "large touch sets should be chunked across multiple transactions"
    );
    assert!(
        outcome.sqlite_phase_metrics.transaction_count > 1,
        "phase metrics should reflect chunked lock acquisitions"
    );
    assert_eq!(
        outcome.sqlite_phase_metrics.phase_name,
        Some(SYNC_FINALIZATION_PHASE_NAME)
    );
}
