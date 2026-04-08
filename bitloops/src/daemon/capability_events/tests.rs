use super::*;
use crate::daemon::CapabilityEventRunStatus;
use crate::host::runtime_store::DaemonSqliteRuntimeStore;
use crate::test_support::process_state::with_env_var;
use tempfile::TempDir;

fn queued_run(
    run_id: &str,
    repo_id: &str,
    capability_id: &str,
    handler_id: &str,
    submitted_at_unix: u64,
    status: CapabilityEventRunStatus,
) -> CapabilityEventRunRecord {
    CapabilityEventRunRecord {
        run_id: run_id.to_string(),
        repo_id: repo_id.to_string(),
        capability_id: capability_id.to_string(),
        handler_id: handler_id.to_string(),
        event_kind: "sync_completed".to_string(),
        lane_key: build_lane_key(repo_id, capability_id, handler_id),
        event_payload_json: serde_json::json!({
            "repo_id": repo_id,
            "repo_root": "/tmp/repo",
            "active_branch": "main",
            "head_commit_sha": "abc123",
            "sync_mode": "full",
            "sync_completed_at": "2026-04-06T00:00:00Z",
            "files": {"added": [], "changed": [], "removed": []},
            "artefacts": {"added": [], "changed": [], "removed": []},
        })
        .to_string(),
        status,
        attempts: 0,
        submitted_at_unix,
        started_at_unix: None,
        updated_at_unix: submitted_at_unix,
        completed_at_unix: None,
        error: None,
    }
}

#[test]
fn next_pending_run_skips_running_lane_and_preserves_fifo() {
    let state = PersistedCapabilityEventQueueState {
        version: 1,
        runs: vec![
            queued_run(
                "run-a-1",
                "repo-1",
                "test_harness",
                "test_harness#0",
                1,
                CapabilityEventRunStatus::Running,
            ),
            queued_run(
                "run-a-2",
                "repo-1",
                "test_harness",
                "test_harness#0",
                2,
                CapabilityEventRunStatus::Queued,
            ),
            queued_run(
                "run-b-1",
                "repo-1",
                "semantic_clones",
                "semantic_clones#0",
                3,
                CapabilityEventRunStatus::Queued,
            ),
        ],
        last_action: Some("running".to_string()),
        updated_at_unix: 3,
    };

    let index = next_pending_run_index(&state).expect("expected runnable run");
    assert_eq!(state.runs[index].run_id, "run-b-1");
}

#[test]
fn next_pending_run_preserves_insertion_order_with_same_timestamp() {
    let state = PersistedCapabilityEventQueueState {
        version: 1,
        runs: vec![
            queued_run(
                "run-z",
                "repo-1",
                "test_harness",
                "test_harness#0",
                1,
                CapabilityEventRunStatus::Queued,
            ),
            queued_run(
                "run-a",
                "repo-1",
                "test_harness",
                "test_harness#0",
                1,
                CapabilityEventRunStatus::Queued,
            ),
        ],
        last_action: Some("enqueue".to_string()),
        updated_at_unix: 1,
    };

    let index = next_pending_run_index(&state).expect("expected runnable run");
    assert_eq!(state.runs[index].run_id, "run-z");
}

#[test]
fn different_lanes_remain_runnable_when_one_lane_is_running() {
    let state = PersistedCapabilityEventQueueState {
        version: 1,
        runs: vec![
            queued_run(
                "run-a-1",
                "repo-1",
                "test_harness",
                "test_harness#0",
                1,
                CapabilityEventRunStatus::Running,
            ),
            queued_run(
                "run-b-1",
                "repo-1",
                "knowledge",
                "knowledge#0",
                2,
                CapabilityEventRunStatus::Queued,
            ),
        ],
        last_action: Some("running".to_string()),
        updated_at_unix: 2,
    };

    let index = next_pending_run_index(&state).expect("expected runnable run");
    assert_eq!(state.runs[index].run_id, "run-b-1");
}

#[test]
fn failed_lane_does_not_block_other_lanes() {
    let state = PersistedCapabilityEventQueueState {
        version: 1,
        runs: vec![
            queued_run(
                "run-a-1",
                "repo-1",
                "test_harness",
                "test_harness#0",
                1,
                CapabilityEventRunStatus::Failed,
            ),
            queued_run(
                "run-b-1",
                "repo-1",
                "semantic_clones",
                "semantic_clones#0",
                2,
                CapabilityEventRunStatus::Queued,
            ),
        ],
        last_action: Some("failed".to_string()),
        updated_at_unix: 2,
    };

    let index = next_pending_run_index(&state).expect("expected runnable run");
    assert_eq!(state.runs[index].run_id, "run-b-1");
}

#[test]
fn project_status_counts_runs_and_selects_current_repo_run() {
    let state = PersistedCapabilityEventQueueState {
        version: 1,
        runs: vec![
            queued_run(
                "run-a-1",
                "repo-1",
                "test_harness",
                "test_harness#0",
                1,
                CapabilityEventRunStatus::Completed,
            ),
            queued_run(
                "run-a-2",
                "repo-1",
                "test_harness",
                "test_harness#0",
                2,
                CapabilityEventRunStatus::Failed,
            ),
            queued_run(
                "run-a-3",
                "repo-1",
                "test_harness",
                "test_harness#0",
                3,
                CapabilityEventRunStatus::Queued,
            ),
            queued_run(
                "run-b-1",
                "repo-2",
                "knowledge",
                "knowledge#0",
                4,
                CapabilityEventRunStatus::Running,
            ),
        ],
        last_action: Some("running".to_string()),
        updated_at_unix: 4,
    };

    let projected = project_status(&state, Some("repo-1"), true);
    assert_eq!(projected.state.pending_runs, 1);
    assert_eq!(projected.state.running_runs, 1);
    assert_eq!(projected.state.failed_runs, 1);
    assert_eq!(projected.state.completed_recent_runs, 1);
    assert!(projected.persisted);
    assert_eq!(
        projected
            .current_repo_run
            .as_ref()
            .map(|run| run.run_id.as_str()),
        Some("run-a-3")
    );
}

#[test]
fn project_status_selects_first_queued_run_for_same_timestamp() {
    let state = PersistedCapabilityEventQueueState {
        version: 1,
        runs: vec![
            queued_run(
                "run-z",
                "repo-1",
                "test_harness",
                "test_harness#0",
                1,
                CapabilityEventRunStatus::Queued,
            ),
            queued_run(
                "run-a",
                "repo-1",
                "test_harness",
                "test_harness#0",
                1,
                CapabilityEventRunStatus::Queued,
            ),
        ],
        last_action: Some("enqueue".to_string()),
        updated_at_unix: 1,
    };

    let projected = project_status(&state, Some("repo-1"), true);
    assert_eq!(
        projected
            .current_repo_run
            .as_ref()
            .map(|run| run.run_id.as_str()),
        Some("run-z")
    );
}

#[test]
fn capability_event_queue_state_persists_in_sqlite() {
    let state_dir = TempDir::new().expect("tempdir");
    crate::test_support::process_state::with_env_var(
        "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
        Some(state_dir.path().to_string_lossy().as_ref()),
        || {
            let store = DaemonSqliteRuntimeStore::open().expect("open daemon runtime store");
            store
                .mutate_capability_event_queue_state(|state| {
                    state.version = 9;
                    state.runs.push(queued_run(
                        "run-1",
                        "repo-1",
                        "test_harness",
                        "test_harness#0",
                        1,
                        CapabilityEventRunStatus::Queued,
                    ));
                    Ok(())
                })
                .expect("save capability event queue state");
            let loaded = store
                .load_capability_event_queue_state()
                .expect("load capability event queue state")
                .expect("state exists");
            assert_eq!(loaded.version, 9);
            assert_eq!(loaded.runs.len(), 1);
        },
    );
}

#[test]
fn coordinator_enqueue_run_persists_and_snapshots_state() {
    let state_dir = TempDir::new().expect("tempdir");
    with_env_var(
        "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
        Some(state_dir.path().to_string_lossy().as_ref()),
        || {
            let coordinator = CapabilityEventCoordinator::shared();
            let run = queued_run(
                "run-1",
                "repo-1",
                "test_harness",
                "test_harness#0",
                1,
                CapabilityEventRunStatus::Queued,
            );

            let outcome = coordinator.enqueue_run(run).expect("enqueue run");
            assert_eq!(outcome.runs.len(), 1);
            let run_id = outcome.runs[0].run_id.clone();
            let loaded = coordinator
                .run(&run_id)
                .expect("load run by id")
                .expect("run must exist");
            assert_eq!(loaded.run_id, run_id);
            assert_eq!(loaded.status, CapabilityEventRunStatus::Queued);

            let snapshot = coordinator
                .snapshot(Some("repo-1"))
                .expect("snapshot capability event status");
            assert_eq!(snapshot.state.pending_runs, 1);
            assert_eq!(
                snapshot
                    .current_repo_run
                    .as_ref()
                    .map(|run| run.run_id.as_str()),
                Some("run-1")
            );
        },
    );
}

#[test]
fn snapshot_does_not_recover_running_runs_without_activation() {
    let state_dir = TempDir::new().expect("tempdir");
    with_env_var(
        "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
        Some(state_dir.path().to_string_lossy().as_ref()),
        || {
            let store = DaemonSqliteRuntimeStore::open().expect("open daemon runtime store");
            let started_at_unix = 42_u64;
            let updated_at_unix = 99_u64;
            store
                .mutate_capability_event_queue_state(|state| {
                    state.version = 1;
                    state.last_action = Some("running".to_string());
                    state.updated_at_unix = updated_at_unix;
                    let mut run = queued_run(
                        "run-running",
                        "repo-1",
                        "test_harness",
                        "test_harness#0",
                        1,
                        CapabilityEventRunStatus::Running,
                    );
                    run.started_at_unix = Some(started_at_unix);
                    run.updated_at_unix = updated_at_unix;
                    state.runs = vec![run];
                    Ok(())
                })
                .expect("seed running capability event run");

            let coordinator = CapabilityEventCoordinator::shared();
            let snapshot = coordinator
                .snapshot(Some("repo-1"))
                .expect("snapshot capability event status");
            assert_eq!(snapshot.state.running_runs, 1);

            let loaded = store
                .load_capability_event_queue_state()
                .expect("load capability event queue state")
                .expect("state exists");
            let run = loaded
                .runs
                .iter()
                .find(|run| run.run_id == "run-running")
                .expect("running run exists");
            assert_eq!(run.status, CapabilityEventRunStatus::Running);
            assert_eq!(run.started_at_unix, Some(started_at_unix));
            assert_eq!(loaded.last_action.as_deref(), Some("running"));
            assert_eq!(loaded.updated_at_unix, updated_at_unix);
        },
    );
}

#[test]
fn activate_worker_recovers_running_runs() {
    let state_dir = TempDir::new().expect("tempdir");
    with_env_var(
        "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
        Some(state_dir.path().to_string_lossy().as_ref()),
        || {
            let store = DaemonSqliteRuntimeStore::open().expect("open daemon runtime store");
            store
                .mutate_capability_event_queue_state(|state| {
                    state.version = 1;
                    state.last_action = Some("running".to_string());
                    let mut run = queued_run(
                        "run-running",
                        "repo-1",
                        "test_harness",
                        "test_harness#0",
                        1,
                        CapabilityEventRunStatus::Running,
                    );
                    run.started_at_unix = Some(12);
                    run.error = Some("previous failure".to_string());
                    state.runs = vec![run];
                    Ok(())
                })
                .expect("seed running capability event run");

            let coordinator = CapabilityEventCoordinator::shared();
            coordinator.activate_worker();
            let snapshot = coordinator
                .snapshot(Some("repo-1"))
                .expect("snapshot capability event status");
            assert_eq!(snapshot.state.pending_runs, 1);
            assert_eq!(snapshot.state.running_runs, 0);

            let loaded = store
                .load_capability_event_queue_state()
                .expect("load capability event queue state")
                .expect("state exists");
            let run = loaded
                .runs
                .iter()
                .find(|run| run.run_id == "run-running")
                .expect("running run exists");
            assert_eq!(run.status, CapabilityEventRunStatus::Queued);
            assert_eq!(run.started_at_unix, None);
            assert_eq!(run.completed_at_unix, None);
            assert_eq!(run.error, None);
            assert_eq!(
                loaded.last_action.as_deref(),
                Some("recovered_running_runs")
            );
        },
    );
}
