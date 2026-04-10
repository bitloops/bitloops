use crate::test_support::process_state::with_env_vars;

use serde_json::json;
use std::path::PathBuf;

use super::progress::{format_live_task_progress_bar_line, format_live_task_status_line};
use super::subscription::should_accept_invalid_daemon_websocket_certs;
use super::types::{
    SyncMutationResult, SyncTaskProgressGraphqlRecord, SyncTaskSpecGraphqlRecord,
    SyncValidationFileDriftMutationResult, SyncValidationMutationResult, TaskGraphqlRecord,
};

fn sample_task(status: &str) -> TaskGraphqlRecord {
    TaskGraphqlRecord {
        task_id: "sync-task-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_name: "bitloops".to_string(),
        repo_identity: "local/bitloops".to_string(),
        kind: "SYNC".to_string(),
        source: "init".to_string(),
        status: status.to_ascii_uppercase(),
        submitted_at_unix: 1,
        started_at_unix: Some(2),
        updated_at_unix: 3,
        completed_at_unix: None,
        queue_position: Some(1),
        tasks_ahead: Some(0),
        error: None,
        sync_spec: Some(SyncTaskSpecGraphqlRecord {
            mode: "auto".to_string(),
            paths: Vec::new(),
        }),
        ingest_spec: None,
        sync_progress: Some(SyncTaskProgressGraphqlRecord {
            phase: "extracting_paths".to_string(),
            current_path: Some("src/lib.rs".to_string()),
            paths_total: 12,
            paths_completed: 4,
            paths_remaining: 8,
            paths_unchanged: 1,
            paths_added: 1,
            paths_changed: 2,
            paths_removed: 0,
            cache_hits: 1,
            cache_misses: 2,
            parse_errors: 0,
        }),
        ingest_progress: None,
        sync_result: None,
        ingest_result: None,
    }
}

fn sample_task_json(status: &str) -> serde_json::Value {
    json!({
        "taskId": "sync-task-1",
        "repoId": "repo-1",
        "repoName": "bitloops",
        "repoIdentity": "local/bitloops",
        "kind": "SYNC",
        "source": "init",
        "status": status.to_ascii_uppercase(),
        "submittedAtUnix": 1,
        "startedAtUnix": 2,
        "updatedAtUnix": 3,
        "completedAtUnix": if status.eq_ignore_ascii_case("completed") { json!(4) } else { serde_json::Value::Null },
        "queuePosition": 1,
        "tasksAhead": 0,
        "error": serde_json::Value::Null,
        "syncSpec": {
            "mode": "auto",
            "paths": []
        },
        "ingestSpec": serde_json::Value::Null,
        "syncProgress": {
            "phase": "complete",
            "currentPath": serde_json::Value::Null,
            "pathsTotal": 12,
            "pathsCompleted": 12,
            "pathsRemaining": 0,
            "pathsUnchanged": 1,
            "pathsAdded": 1,
            "pathsChanged": 2,
            "pathsRemoved": 0,
            "cacheHits": 1,
            "cacheMisses": 2,
            "parseErrors": 0
        },
        "ingestProgress": serde_json::Value::Null
    })
}

fn test_scope() -> crate::devql_transport::SlimCliRepoScope {
    crate::devql_transport::SlimCliRepoScope {
        repo: crate::host::devql::RepoIdentity {
            provider: "local".to_string(),
            organization: "local".to_string(),
            name: "bitloops".to_string(),
            identity: "local://local/bitloops".to_string(),
            repo_id: "repo-1".to_string(),
        },
        repo_root: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        branch_name: "main".to_string(),
        project_path: None,
        git_dir_relative_path: ".git".to_string(),
        config_fingerprint: "test-config-fingerprint".to_string(),
    }
}

#[test]
fn websocket_client_only_relaxes_loopback_wss_urls() {
    assert!(should_accept_invalid_daemon_websocket_certs(
        "wss://localhost:5667/devql/global"
    ));
    assert!(should_accept_invalid_daemon_websocket_certs(
        "wss://127.0.0.1:5667/devql/global"
    ));
    assert!(should_accept_invalid_daemon_websocket_certs(
        "wss://[::1]:5667/devql/global"
    ));
    assert!(!should_accept_invalid_daemon_websocket_certs(
        "ws://127.0.0.1:5667/devql/global"
    ));
    assert!(!should_accept_invalid_daemon_websocket_certs(
        "wss://dev.internal:5667/devql/global"
    ));
    assert!(!should_accept_invalid_daemon_websocket_certs("not-a-url"));
}

#[test]
fn live_task_status_line_is_compact_and_single_line() {
    let task = sample_task("running");

    let rendered = format_live_task_status_line(&task, "*", None);
    assert_eq!(
        rendered,
        "* Syncing bitloops · extracting artefacts · 4/12 · src/lib.rs"
    );
    assert!(!rendered.contains('\n'));
}

#[test]
fn live_task_status_line_elides_to_terminal_width() {
    let mut task = sample_task("running");
    task.sync_progress
        .as_mut()
        .expect("sync progress")
        .current_path = Some("bitloops/src/host/devql/commands_sync/orchestrator.rs".to_string());
    task.sync_progress
        .as_mut()
        .expect("sync progress")
        .paths_total = 764;
    task.sync_progress
        .as_mut()
        .expect("sync progress")
        .paths_completed = 472;
    task.sync_progress
        .as_mut()
        .expect("sync progress")
        .paths_remaining = 292;

    let rendered = format_live_task_status_line(&task, "*", Some(48));
    assert!(rendered.chars().count() <= 48);
    assert!(rendered.contains('…'));
    assert!(!rendered.contains('\n'));
}

#[test]
fn live_task_progress_bar_line_fits_requested_width() {
    with_env_vars(&[("NO_COLOR", Some("1"))], || {
        let mut task = sample_task("running");
        task.sync_progress.as_mut().expect("sync progress").phase =
            "materialising_paths".to_string();
        task.sync_progress
            .as_mut()
            .expect("sync progress")
            .current_path = None;
        task.sync_progress
            .as_mut()
            .expect("sync progress")
            .paths_total = 764;
        task.sync_progress
            .as_mut()
            .expect("sync progress")
            .paths_completed = 472;
        task.sync_progress
            .as_mut()
            .expect("sync progress")
            .paths_remaining = 292;

        let rendered = format_live_task_progress_bar_line(&task, 0, Some(48));
        assert!(rendered.chars().count() <= 48);
        assert!(rendered.contains("472/764"));
        assert!(!rendered.contains('\n'));
    });
}

#[test]
fn live_task_status_line_covers_terminal_states() {
    let queued = format_live_task_status_line(&sample_task("queued"), "*", None);
    assert!(queued.contains("Sync queued for bitloops"));
    assert!(queued.contains("mode=auto"));

    let completed = format_live_task_status_line(&sample_task("completed"), "*", None);
    assert!(completed.contains("Sync complete"));

    let failed = format_live_task_status_line(&sample_task("failed"), "*", None);
    assert!(failed.contains("Sync failed"));

    let cancelled = format_live_task_status_line(&sample_task("cancelled"), "*", None);
    assert!(cancelled.contains("Sync cancelled"));

    let unknown = format_live_task_status_line(&sample_task("paused"), "*", None);
    assert!(unknown.contains("Sync paused for bitloops"));
}

#[test]
fn live_task_progress_bar_line_handles_non_progress_states() {
    with_env_vars(&[("NO_COLOR", Some("1"))], || {
        let mut queued = sample_task("queued");
        queued
            .sync_progress
            .as_mut()
            .expect("sync progress")
            .paths_total = 0;
        queued
            .sync_progress
            .as_mut()
            .expect("sync progress")
            .paths_completed = 0;
        queued
            .sync_progress
            .as_mut()
            .expect("sync progress")
            .paths_remaining = 0;
        queued.sync_progress.as_mut().expect("sync progress").phase =
            "building_manifest".to_string();

        let rendered = format_live_task_progress_bar_line(&queued, 2, Some(40));
        assert!(rendered.contains("building manifest"));

        let mut failed = sample_task("failed");
        failed
            .sync_progress
            .as_mut()
            .expect("sync progress")
            .paths_total = 10;
        failed
            .sync_progress
            .as_mut()
            .expect("sync progress")
            .paths_completed = 3;
        failed
            .sync_progress
            .as_mut()
            .expect("sync progress")
            .paths_remaining = 7;
        let failed_line = format_live_task_progress_bar_line(&failed, 0, Some(40));
        assert!(failed_line.contains(" 30% 3/10"));
    });
}

#[test]
fn live_task_progress_bar_line_elides_when_too_narrow() {
    with_env_vars(&[("NO_COLOR", Some("1"))], || {
        let mut task = sample_task("running");
        task.sync_progress
            .as_mut()
            .expect("sync progress")
            .paths_total = 100;
        task.sync_progress
            .as_mut()
            .expect("sync progress")
            .paths_completed = 20;
        task.sync_progress
            .as_mut()
            .expect("sync progress")
            .paths_remaining = 80;

        let rendered = format_live_task_progress_bar_line(&task, 0, Some(8));
        assert!(rendered.chars().count() <= 16);
    });
}

#[test]
fn sync_mutation_result_converts_to_sync_summary_with_validation_details() {
    let result = SyncMutationResult {
        success: true,
        mode: "validate".to_string(),
        parser_version: "parser@1".to_string(),
        extractor_version: "extractor@1".to_string(),
        active_branch: Some("main".to_string()),
        head_commit_sha: Some("abc".to_string()),
        head_tree_sha: Some("def".to_string()),
        paths_unchanged: 1,
        paths_added: 2,
        paths_changed: 3,
        paths_removed: 4,
        cache_hits: 5,
        cache_misses: 6,
        parse_errors: 7,
        validation: Some(SyncValidationMutationResult {
            valid: false,
            expected_artefacts: 10,
            actual_artefacts: 9,
            expected_edges: 8,
            actual_edges: 7,
            missing_artefacts: 1,
            stale_artefacts: 2,
            mismatched_artefacts: 3,
            missing_edges: 4,
            stale_edges: 5,
            mismatched_edges: 6,
            files_with_drift: vec![SyncValidationFileDriftMutationResult {
                path: "src/lib.rs".to_string(),
                missing_artefacts: 1,
                stale_artefacts: 2,
                mismatched_artefacts: 3,
                missing_edges: 4,
                stale_edges: 5,
                mismatched_edges: 6,
            }],
        }),
    };

    let summary: crate::host::devql::SyncSummary = result.into();
    assert!(summary.success);
    assert_eq!(summary.mode, "validate");
    assert_eq!(summary.paths_added, 2);
    assert_eq!(summary.parse_errors, 7);
    let validation = summary.validation.expect("validation payload");
    assert!(!validation.valid);
    assert_eq!(validation.files_with_drift.len(), 1);
    assert_eq!(validation.files_with_drift[0].path, "src/lib.rs");
}

#[test]
fn tasks_query_omits_result_payloads_and_deserializes_completed_tasks() {
    let scope = test_scope();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");

    super::with_graphql_executor_hook(
        |_, query, _| {
            assert_eq!(query, super::documents::TASKS_QUERY);
            assert!(!query.contains("syncResult"));
            assert!(!query.contains("ingestResult"));
            Ok(json!({
                "tasks": [sample_task_json("completed")]
            }))
        },
        || {
            let tasks = runtime
                .block_on(super::list_tasks_via_graphql(
                    &scope,
                    Some("sync"),
                    Some("completed"),
                    Some(1),
                ))
                .expect("list tasks via graphql");
            assert_eq!(tasks.len(), 1);
            assert!(tasks[0].is_terminal());
            assert!(tasks[0].sync_result.is_none());
            assert!(tasks[0].ingest_result.is_none());
        },
    );
}

#[test]
fn task_queue_query_omits_result_payloads_and_deserializes_current_repo_tasks() {
    let scope = test_scope();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");

    super::with_graphql_executor_hook(
        |_, query, _| {
            assert_eq!(query, super::documents::TASK_QUEUE_QUERY);
            assert!(!query.contains("syncResult"));
            assert!(!query.contains("ingestResult"));
            Ok(json!({
                "taskQueue": {
                    "persisted": true,
                    "queuedTasks": 0,
                    "runningTasks": 0,
                    "failedTasks": 0,
                    "completedRecentTasks": 1,
                    "byKind": [
                        {
                            "kind": "SYNC",
                            "queuedTasks": 0,
                            "runningTasks": 0,
                            "failedTasks": 0,
                            "completedRecentTasks": 1
                        },
                        {
                            "kind": "INGEST",
                            "queuedTasks": 0,
                            "runningTasks": 0,
                            "failedTasks": 0,
                            "completedRecentTasks": 0
                        }
                    ],
                    "paused": false,
                    "pausedReason": serde_json::Value::Null,
                    "lastAction": "completed",
                    "lastUpdatedUnix": 3,
                    "currentRepoTasks": [sample_task_json("completed")]
                }
            }))
        },
        || {
            let status = runtime
                .block_on(super::task_queue_status_via_graphql(&scope))
                .expect("load task queue via graphql");
            assert_eq!(status.current_repo_tasks.len(), 1);
            assert!(status.current_repo_tasks[0].is_terminal());
            assert!(status.current_repo_tasks[0].sync_result.is_none());
            assert!(status.current_repo_tasks[0].ingest_result.is_none());
        },
    );
}
