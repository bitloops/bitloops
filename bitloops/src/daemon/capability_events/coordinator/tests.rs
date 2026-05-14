use std::collections::VecDeque;
use std::fs;
use std::sync::Arc;
use std::sync::Mutex;

use rusqlite::params;
use tempfile::TempDir;

use crate::daemon::capability_events::queue::sql_i64;
use crate::daemon::current_state_worker::{
    CurrentStateWorkerHandle, CurrentStateWorkerInvocation, CurrentStateWorkerRunner,
};
use crate::daemon::types::{CapabilityEventRunRecord, CapabilityEventRunStatus};
use crate::host::capability_host::{
    CurrentStateConsumerResult, DevqlCapabilityHost, SyncArtefactDiff, SyncFileDiff,
};
use crate::host::devql::{DevqlConfig, SyncSummary, resolve_repo_identity};
use crate::host::runtime_store::DaemonSqliteRuntimeStore;
use crate::test_support::git_fixtures::{init_test_repo, write_test_daemon_config};
use crate::test_support::log_capture::capture_logs_async;

use super::super::queue::StoredRunRecord;
use super::types::{CapabilityEventCoordinator, RunCompletion, SyncGenerationInput};

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
        .with_connection(|conn| {
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
        .with_connection(|conn| {
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

fn insert_generation_row(
    store: &DaemonSqliteRuntimeStore,
    repo_id: &str,
    generation_seq: u64,
    active_branch: &str,
    head_commit_sha: &str,
) {
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO capability_workplane_cursor_generations (repo_id, generation_seq, source_task_id, sync_mode, active_branch, head_commit_sha, requires_full_reconcile, created_at_unix) VALUES (?1, ?2, NULL, 'merged_delta', ?3, ?4, 0, ?5)",
                params![
                    repo_id,
                    sql_i64(generation_seq)?,
                    active_branch,
                    head_commit_sha,
                    sql_i64(generation_seq + 100)?,
                ],
            )?;
            Ok(())
        })
        .expect("insert generation row");
}

fn insert_file_change_row(
    store: &DaemonSqliteRuntimeStore,
    repo_id: &str,
    generation_seq: u64,
    path: &str,
) {
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO capability_workplane_cursor_file_changes (repo_id, generation_seq, path, change_kind, language, content_id) VALUES (?1, ?2, ?3, 'changed', 'rust', 'content-id')",
                params![repo_id, sql_i64(generation_seq)?, path],
            )?;
            Ok(())
        })
        .expect("insert file change row");
}

fn test_cfg(repo_root: &std::path::Path) -> DevqlConfig {
    let repo = resolve_repo_identity(repo_root).expect("resolve repo identity");
    DevqlConfig::from_env(repo_root.to_path_buf(), repo).expect("build devql config")
}

#[derive(Debug, Clone)]
struct StubWorkerRunner {
    pid: u32,
    invocations: Arc<Mutex<Vec<CurrentStateWorkerInvocation>>>,
    results: Arc<Mutex<VecDeque<anyhow::Result<CurrentStateConsumerResult>>>>,
}

impl StubWorkerRunner {
    fn success(pid: u32, result: CurrentStateConsumerResult) -> Self {
        Self {
            pid,
            invocations: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(VecDeque::from(vec![Ok(result)]))),
        }
    }

    fn failure(pid: u32, error: &str) -> Self {
        Self {
            pid,
            invocations: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(VecDeque::from(vec![Err(anyhow::anyhow!(
                error.to_string()
            ))]))),
        }
    }

    fn invocations(&self) -> Vec<CurrentStateWorkerInvocation> {
        self.invocations.lock().expect("lock invocations").clone()
    }
}

impl CurrentStateWorkerRunner for StubWorkerRunner {
    fn spawn(
        &self,
        invocation: CurrentStateWorkerInvocation,
    ) -> anyhow::Result<Box<dyn CurrentStateWorkerHandle>> {
        self.invocations
            .lock()
            .expect("lock invocations")
            .push(invocation);
        let result = self
            .results
            .lock()
            .expect("lock results")
            .pop_front()
            .unwrap_or_else(|| Err(anyhow::anyhow!("stub worker runner exhausted")));
        Ok(Box::new(StubWorkerHandle {
            pid: self.pid,
            result: Some(result),
        }))
    }
}

struct StubWorkerHandle {
    pid: u32,
    result: Option<anyhow::Result<CurrentStateConsumerResult>>,
}

impl CurrentStateWorkerHandle for StubWorkerHandle {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn wait<'a>(
        mut self: Box<Self>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = anyhow::Result<CurrentStateConsumerResult>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            self.result
                .take()
                .expect("stub worker handle should only be awaited once")
        })
    }
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

fn architecture_graph_worker_run(status: CapabilityEventRunStatus) -> CapabilityEventRunRecord {
    CapabilityEventRunRecord {
        run_id: "run-architecture-graph".to_string(),
        repo_id: "repo-architecture".to_string(),
        capability_id: "architecture_graph".to_string(),
        consumer_id: "architecture_graph.snapshot".to_string(),
        handler_id: "architecture_graph.snapshot".to_string(),
        from_generation_seq: 0,
        to_generation_seq: 1,
        reconcile_mode: "full_reconcile".to_string(),
        event_kind: "current_state_consumer".to_string(),
        lane_key: "repo-architecture:architecture_graph.snapshot".to_string(),
        event_payload_json: String::new(),
        init_session_id: Some("init-session-1".to_string()),
        status,
        attempts: 1,
        submitted_at_unix: 10,
        started_at_unix: Some(20),
        updated_at_unix: 30,
        completed_at_unix: None,
        error: None,
    }
}

fn prepare_architecture_graph_worker_run(
    runner: Arc<dyn CurrentStateWorkerRunner>,
) -> (
    TempDir,
    Arc<CapabilityEventCoordinator>,
    CapabilityEventRunRecord,
    StoredRunRecord,
) {
    let temp = TempDir::new().expect("tempdir");
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(&repo_root).expect("create repo root");
    init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops@example.com");
    write_test_daemon_config(&repo_root);

    let store = test_runtime_store(&temp);
    let coordinator =
        CapabilityEventCoordinator::new_shared_instance_with_runner(store.clone(), runner);
    let run = architecture_graph_worker_run(CapabilityEventRunStatus::Running);
    insert_consumer_row(
        &store,
        &run.repo_id,
        &run.capability_id,
        &run.consumer_id,
        Some(0),
        None,
        1,
    );
    insert_generation_row(&store, &run.repo_id, 1, "main", "abc123");
    insert_file_change_row(&store, &run.repo_id, 1, "src/lib.rs");
    insert_run_row(&store, &run);

    let stored_run = StoredRunRecord {
        record: run.clone(),
        repo_root,
    };

    (temp, coordinator, run, stored_run)
}

#[tokio::test]
async fn execute_run_uses_worker_for_architecture_graph_full_reconcile_and_applies_completion() {
    let runner = Arc::new(StubWorkerRunner::success(
        4242,
        CurrentStateConsumerResult::applied(1),
    ));
    let (_temp, coordinator, run, stored_run) =
        prepare_architecture_graph_worker_run(runner.clone());

    let (completion, records) =
        capture_logs_async(Arc::clone(&coordinator).execute_run(stored_run)).await;
    let RunCompletion::Completed {
        applied_to_generation_seq,
        ..
    } = completion.clone()
    else {
        panic!("expected completed worker run, got {completion:?}");
    };
    assert_eq!(applied_to_generation_seq, 1);

    let invocations = runner.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].capability_id, "architecture_graph");
    assert_eq!(invocations[0].consumer_id, "architecture_graph.snapshot");
    assert_eq!(
        invocations[0].request.reconcile_mode,
        crate::host::capability_host::ReconcileMode::FullReconcile
    );
    assert_eq!(invocations[0].parent_pid, Some(std::process::id()));
    assert_eq!(
        invocations[0].init_session_id.as_deref(),
        run.init_session_id.as_deref()
    );
    assert!(records.iter().any(|record| {
        record.message.contains("current-state worker spawned")
            && record.message.contains("pid=4242")
            && record.message.contains("run_id=run-architecture-graph")
            && record.message.contains("capability_id=architecture_graph")
            && record
                .message
                .contains("consumer_id=architecture_graph.snapshot")
            && record
                .message
                .contains("route_reason=architecture_graph_full_reconcile")
    }));
    assert!(records.iter().any(|record| {
        record
            .message
            .contains("current-state worker exited successfully")
            && record.message.contains("pid=4242")
            && record.message.contains("run_id=run-architecture-graph")
            && record
                .message
                .contains("route_reason=architecture_graph_full_reconcile")
    }));
    assert!(
        coordinator
            .active_worker_children
            .lock()
            .expect("lock active worker map")
            .is_empty(),
        "tracked worker map should be empty after worker exit"
    );

    coordinator
        .apply_completion(completion)
        .expect("apply completion should succeed");

    let persisted = coordinator
        .run(&run.run_id)
        .expect("load run")
        .expect("persisted run");
    assert_eq!(persisted.status, CapabilityEventRunStatus::Completed);

    coordinator
        .runtime_store
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
            assert_eq!(cursor.0, Some(1));
            assert_eq!(cursor.1, None);
            Ok(())
        })
        .expect("load updated cursor");
}

#[tokio::test]
async fn execute_run_worker_failures_follow_retry_budget() {
    for error in [
        "current-state worker exited unsuccessfully (exit code 7)",
        "current-state worker produced empty stdout",
        "parsing current-state worker stdout as JSON",
        "unsupported current-state worker target",
    ] {
        let runner = Arc::new(StubWorkerRunner::failure(4242, error));
        let (_temp, coordinator, _run, stored_run) = prepare_architecture_graph_worker_run(runner);

        let (completion, records) =
            capture_logs_async(Arc::clone(&coordinator).execute_run(stored_run)).await;
        let RunCompletion::RetryableFailure { error: actual, .. } = completion else {
            panic!("expected retryable failure for `{error}`, got {completion:?}");
        };
        assert!(
            actual.contains(error),
            "expected retryable failure to include `{error}`, got `{actual}`"
        );
        assert!(records.iter().any(|record| {
            record.message.contains("current-state worker spawned")
                && record.message.contains("pid=4242")
        }));
        assert!(records.iter().any(|record| {
            record
                .message
                .contains("current-state worker exited with failure")
                && record.message.contains("pid=4242")
                && record.message.contains("run_id=run-architecture-graph")
                && record.message.contains(error)
        }));
        assert!(
            coordinator
                .active_worker_children
                .lock()
                .expect("lock active worker map")
                .is_empty(),
            "tracked worker map should be empty after worker failure"
        );
    }
}

#[cfg(unix)]
#[test]
fn terminate_active_worker_children_kills_tracked_workers_and_clears_map() {
    let temp = TempDir::new().expect("tempdir");
    let coordinator = CapabilityEventCoordinator::new_shared_instance(test_runtime_store(&temp));
    let child = std::process::Command::new("sh")
        .args(["-c", "sleep 30"])
        .spawn()
        .expect("spawn sleeper");
    let pid = child.id();
    // The coordinator terminates and reaps this worker by PID, so the handle can be dropped.
    drop(child);

    coordinator
        .active_worker_children
        .lock()
        .expect("lock active worker map")
        .insert(
            "run-sleep".to_string(),
            super::types::ActiveWorkerChild { pid },
        );

    coordinator
        .terminate_active_worker_children()
        .expect("terminate active worker children");

    assert!(
        coordinator
            .active_worker_children
            .lock()
            .expect("lock active worker map")
            .is_empty(),
        "tracked worker map should be cleared"
    );
    assert!(
        !crate::daemon::process_is_running(pid).expect("check worker process liveness"),
        "sleep child should be terminated"
    );
}
