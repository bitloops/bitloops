use std::fs;

use rusqlite::params;
use tempfile::TempDir;

use crate::capability_packs::architecture_graph::types::{
    ARCHITECTURE_GRAPH_CONSUMER_ID, ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID,
};
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_INBOUND_CURRENT_STATE_MAILBOX,
};
use crate::daemon::capability_events::queue::sql_i64;
use crate::daemon::types::{CapabilityEventRunRecord, CapabilityEventRunStatus};
use crate::host::capability_host::{DevqlCapabilityHost, SyncArtefactDiff, SyncFileDiff};
use crate::host::devql::{DevqlConfig, SyncSummary, resolve_repo_identity};
use crate::host::runtime_store::DaemonSqliteRuntimeStore;
use crate::test_support::git_fixtures::{init_test_repo, write_test_daemon_config};

use super::types::{CapabilityEventCoordinator, RunCompletion, SyncGenerationInput};
use super::worker::load_claimable_runs;

fn test_runtime_store(dir: &TempDir) -> DaemonSqliteRuntimeStore {
    DaemonSqliteRuntimeStore::open_at(dir.path().join("runtime.sqlite"))
        .expect("open test runtime store")
}

fn sample_run(status: CapabilityEventRunStatus) -> CapabilityEventRunRecord {
    CapabilityEventRunRecord {
        run_id: "run-1".to_string(),
        repo_id: "repo-1".to_string(),
        capability_id: "test_harness".to_string(),
        consumer_id: "test_harness.current_state".to_string(),
        handler_id: "test_harness.current_state".to_string(),
        from_generation_seq: 2,
        to_generation_seq: 5,
        reconcile_mode: "merged_delta".to_string(),
        event_kind: "current_state_consumer".to_string(),
        lane_key: "repo-1:test_harness.current_state".to_string(),
        event_payload_json: String::new(),
        init_session_id: None,
        status,
        attempts: 1,
        submitted_at_unix: 10,
        started_at_unix: Some(20),
        updated_at_unix: 30,
        completed_at_unix: None,
        error: Some("stale error".to_string()),
    }
}

fn insert_consumer_row(
    store: &DaemonSqliteRuntimeStore,
    repo_id: &str,
    capability_id: &str,
    consumer_id: &str,
    last_applied_generation_seq: Option<u64>,
    last_error: Option<&str>,
    updated_at_unix: u64,
) {
    store
        .with_write_connection(|conn| {
            conn.execute(
                "INSERT INTO capability_workplane_cursor_mailboxes (repo_id, capability_id, mailbox_name, last_applied_generation_seq, last_error, updated_at_unix) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    repo_id,
                    capability_id,
                    consumer_id,
                    last_applied_generation_seq.map(sql_i64).transpose()?,
                    last_error,
                    sql_i64(updated_at_unix)?,
                ],
            )?;
            Ok(())
        })
        .expect("insert consumer row");
}

fn insert_run_row(store: &DaemonSqliteRuntimeStore, run: &CapabilityEventRunRecord) {
    store
        .with_write_connection(|conn| {
            conn.execute(
                "INSERT INTO capability_workplane_cursor_runs (run_id, repo_id, repo_root, mailbox_name, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    run.run_id,
                    run.repo_id,
                    "/tmp/repo",
                    run.consumer_id,
                    run.capability_id,
                    sql_i64(run.from_generation_seq)?,
                    sql_i64(run.to_generation_seq)?,
                    run.reconcile_mode,
                    run.status.to_string(),
                    run.attempts,
                    sql_i64(run.submitted_at_unix)?,
                    run.started_at_unix.map(sql_i64).transpose()?,
                    sql_i64(run.updated_at_unix)?,
                    run.completed_at_unix.map(sql_i64).transpose()?,
                    run.error,
                ],
            )?;
            Ok(())
        })
        .expect("insert run row");
}

fn test_cfg(repo_root: &std::path::Path) -> DevqlConfig {
    let repo = resolve_repo_identity(repo_root).expect("resolve repo identity");
    DevqlConfig::from_env(repo_root.to_path_buf(), repo).expect("build devql config")
}

#[test]
fn record_sync_generation_schedules_consumers_for_successful_empty_sync() {
    let temp = TempDir::new().expect("tempdir");
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(&repo_root).expect("create repo root");
    init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops@example.com");
    write_test_daemon_config(&repo_root);

    let cfg = test_cfg(&repo_root);
    let host = DevqlCapabilityHost::builtin(repo_root.clone(), cfg.repo.clone())
        .expect("build capability host");
    let coordinator = CapabilityEventCoordinator::new_shared_instance(test_runtime_store(&temp));
    let cursor_mailbox_count = host
        .workplane_mailboxes()
        .iter()
        .filter(|registration| {
            registration.policy == crate::host::capability_host::CapabilityMailboxPolicy::Cursor
        })
        .count();

    let outcome = coordinator
        .record_sync_generation(
            &host,
            &cfg,
            &SyncSummary {
                success: true,
                mode: "auto".to_string(),
                active_branch: Some("main".to_string()),
                head_commit_sha: Some("abc123".to_string()),
                ..SyncSummary::default()
            },
            SyncGenerationInput {
                file_diff: SyncFileDiff::default(),
                artefact_diff: SyncArtefactDiff::default(),
                source_task_id: None,
                init_session_id: None,
            },
        )
        .expect("record empty sync generation");

    assert!(
        cursor_mailbox_count > 0,
        "expected built-in cursor mailboxes to be registered"
    );
    assert_eq!(outcome.runs.len(), cursor_mailbox_count);

    let snapshot = coordinator
        .snapshot(Some(&cfg.repo.repo_id))
        .expect("snapshot capability event queue");
    assert_eq!(snapshot.state.pending_runs as usize, cursor_mailbox_count);
}

#[test]
fn record_sync_generation_attaches_init_session_only_to_blocking_cursor_mailboxes() {
    let temp = TempDir::new().expect("tempdir");
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(&repo_root).expect("create repo root");
    init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops@example.com");
    write_test_daemon_config(&repo_root);

    let cfg = test_cfg(&repo_root);
    let host = DevqlCapabilityHost::builtin(repo_root.clone(), cfg.repo.clone())
        .expect("build capability host");
    let store = test_runtime_store(&temp);
    let coordinator = CapabilityEventCoordinator::new_shared_instance(store.clone());

    coordinator
        .record_sync_generation(
            &host,
            &cfg,
            &SyncSummary {
                success: true,
                mode: "auto".to_string(),
                active_branch: Some("main".to_string()),
                head_commit_sha: Some("abc123".to_string()),
                ..SyncSummary::default()
            },
            SyncGenerationInput {
                file_diff: SyncFileDiff::default(),
                artefact_diff: SyncArtefactDiff::default(),
                source_task_id: None,
                init_session_id: Some("init-session-1"),
            },
        )
        .expect("record sync generation");

    let rows = store
        .with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT capability_id, mailbox_name, init_session_id
                 FROM capability_workplane_cursor_runs
                 WHERE repo_id = ?1
                 ORDER BY capability_id ASC, mailbox_name ASC",
            )?;
            let rows = stmt
                .query_map(params![cfg.repo.repo_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .expect("load cursor runs");

    let semantic = rows
        .iter()
        .find(|(capability_id, mailbox_name, _)| {
            capability_id == SEMANTIC_CLONES_CAPABILITY_ID
                && mailbox_name == SEMANTIC_CLONES_INBOUND_CURRENT_STATE_MAILBOX
        })
        .expect("semantic current-state run");
    assert_eq!(semantic.2.as_deref(), Some("init-session-1"));

    let architecture = rows
        .iter()
        .find(|(_, mailbox_name, _)| mailbox_name == ARCHITECTURE_GRAPH_CONSUMER_ID)
        .expect("architecture graph snapshot run");
    assert_eq!(architecture.2, None);
    assert!(
        rows.iter().any(|(_, mailbox_name, init_session_id)| {
            mailbox_name == ARCHITECTURE_GRAPH_CONSUMER_ID && init_session_id.is_none()
        }),
        "architecture graph snapshot must be scheduled as background current-state work"
    );
    assert!(
        rows.iter().any(|(_, mailbox_name, init_session_id)| {
            mailbox_name == ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID
                && init_session_id.is_none()
        }),
        "architecture role current-state work must be scheduled as background work"
    );
}

#[test]
fn apply_completion_noop_keeps_existing_cursor_and_clears_error() {
    let temp = TempDir::new().expect("tempdir");
    let store = test_runtime_store(&temp);
    let coordinator = CapabilityEventCoordinator::new_shared_instance(store.clone());
    let run = sample_run(CapabilityEventRunStatus::Running);
    insert_consumer_row(
        &store,
        &run.repo_id,
        &run.capability_id,
        &run.consumer_id,
        Some(7),
        Some("previous failure"),
        29,
    );
    insert_run_row(&store, &run);

    coordinator
        .apply_completion(RunCompletion::NoopCompleted { run: run.clone() })
        .expect("apply noop completion");

    store
        .with_connection(|conn| {
            let cursor = conn.query_row(
                "SELECT last_applied_generation_seq, last_error
                 FROM capability_workplane_cursor_mailboxes
                 WHERE repo_id = ?1 AND mailbox_name = ?2",
                params![&run.repo_id, &run.consumer_id],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )?;
            assert_eq!(cursor.0, Some(7));
            assert_eq!(cursor.1, None);
            Ok(())
        })
        .expect("load consumer cursor");

    let persisted = coordinator
        .run(&run.run_id)
        .expect("load run")
        .expect("persisted run");
    assert_eq!(persisted.status, CapabilityEventRunStatus::Completed);
    assert_eq!(persisted.error, None);
}

#[test]
fn apply_completion_retryable_failure_clears_started_at_when_requeued() {
    let temp = TempDir::new().expect("tempdir");
    let store = test_runtime_store(&temp);
    let coordinator = CapabilityEventCoordinator::new_shared_instance(store.clone());
    let run = sample_run(CapabilityEventRunStatus::Running);
    insert_consumer_row(
        &store,
        &run.repo_id,
        &run.capability_id,
        &run.consumer_id,
        Some(2),
        None,
        29,
    );
    insert_run_row(&store, &run);

    coordinator
        .apply_completion(RunCompletion::RetryableFailure {
            run: run.clone(),
            error: "temporary failure".to_string(),
        })
        .expect("apply retryable failure");

    let persisted = coordinator
        .run(&run.run_id)
        .expect("load run")
        .expect("persisted run");
    assert_eq!(persisted.status, CapabilityEventRunStatus::Queued);
    assert_eq!(persisted.started_at_unix, None);
    assert_eq!(persisted.completed_at_unix, None);
    assert_eq!(persisted.error.as_deref(), Some("temporary failure"));
}

#[test]
fn snapshot_uses_latest_run_status_and_timestamp() {
    let temp = TempDir::new().expect("tempdir");
    let store = test_runtime_store(&temp);
    let coordinator = CapabilityEventCoordinator::new_shared_instance(store.clone());
    let mut older = sample_run(CapabilityEventRunStatus::Running);
    older.run_id = "run-older".to_string();
    older.updated_at_unix = 11;
    older.submitted_at_unix = 9;
    let mut newer = sample_run(CapabilityEventRunStatus::Failed);
    newer.run_id = "run-newer".to_string();
    newer.updated_at_unix = 42;
    newer.submitted_at_unix = 41;
    insert_run_row(&store, &older);
    insert_run_row(&store, &newer);

    let snapshot = coordinator.snapshot(None).expect("snapshot queue");
    assert_eq!(snapshot.state.last_action.as_deref(), Some("failed"));
    assert_eq!(snapshot.state.last_updated_unix, 42);
}

#[test]
fn load_claimable_runs_claims_one_current_state_run_at_a_time() {
    let temp = TempDir::new().expect("tempdir");
    let store = test_runtime_store(&temp);
    let mut first = sample_run(CapabilityEventRunStatus::Queued);
    first.run_id = "run-first".to_string();
    first.consumer_id = "test_harness.current_state".to_string();
    first.lane_key = "repo-1:test_harness.current_state".to_string();
    first.submitted_at_unix = 10;
    let mut second = sample_run(CapabilityEventRunStatus::Queued);
    second.run_id = "run-second".to_string();
    second.consumer_id = "architecture_graph.snapshot".to_string();
    second.lane_key = "repo-1:architecture_graph.snapshot".to_string();
    second.submitted_at_unix = 11;
    insert_run_row(&store, &first);
    insert_run_row(&store, &second);

    let claimed = store
        .with_connection(load_claimable_runs)
        .expect("load claimable runs");

    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].record.run_id, "run-first");
}

#[test]
fn load_claimable_runs_waits_when_current_state_run_is_active() {
    let temp = TempDir::new().expect("tempdir");
    let store = test_runtime_store(&temp);
    let active = sample_run(CapabilityEventRunStatus::Running);
    let mut queued = sample_run(CapabilityEventRunStatus::Queued);
    queued.run_id = "run-queued".to_string();
    queued.consumer_id = "architecture_graph.snapshot".to_string();
    queued.lane_key = "repo-1:architecture_graph.snapshot".to_string();
    queued.submitted_at_unix = 11;
    insert_run_row(&store, &active);
    insert_run_row(&store, &queued);

    let claimed = store
        .with_connection(load_claimable_runs)
        .expect("load claimable runs");

    assert!(claimed.is_empty());
}
