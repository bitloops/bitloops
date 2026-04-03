use crate::test_support::process_state::with_env_vars;

use super::progress::{format_live_sync_progress_bar_line, format_live_sync_task_status_line};
use super::subscription::should_accept_invalid_daemon_websocket_certs;
use super::types::SyncTaskGraphqlRecord;

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
fn live_sync_status_line_is_compact_and_single_line() {
    let task = SyncTaskGraphqlRecord {
        task_id: "sync-task-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_name: "bitloops".to_string(),
        repo_identity: "local/bitloops".to_string(),
        source: "init".to_string(),
        mode: "auto".to_string(),
        status: "running".to_string(),
        phase: "extracting_paths".to_string(),
        submitted_at_unix: 1,
        started_at_unix: Some(2),
        updated_at_unix: 3,
        completed_at_unix: None,
        queue_position: Some(1),
        tasks_ahead: Some(0),
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
        error: None,
        summary: None,
    };

    let rendered = format_live_sync_task_status_line(&task, "*", None);
    assert_eq!(
        rendered,
        "* Syncing bitloops · extracting artefacts · 4/12 · src/lib.rs"
    );
    assert!(!rendered.contains('\n'));
}

#[test]
fn live_sync_status_line_elides_to_terminal_width() {
    let task = SyncTaskGraphqlRecord {
        task_id: "sync-task-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_name: "bitloops".to_string(),
        repo_identity: "local/bitloops".to_string(),
        source: "init".to_string(),
        mode: "auto".to_string(),
        status: "running".to_string(),
        phase: "extracting_paths".to_string(),
        submitted_at_unix: 1,
        started_at_unix: Some(2),
        updated_at_unix: 3,
        completed_at_unix: None,
        queue_position: Some(1),
        tasks_ahead: Some(0),
        current_path: Some("bitloops/src/host/devql/commands_sync/orchestrator.rs".to_string()),
        paths_total: 764,
        paths_completed: 472,
        paths_remaining: 292,
        paths_unchanged: 0,
        paths_added: 0,
        paths_changed: 0,
        paths_removed: 0,
        cache_hits: 0,
        cache_misses: 0,
        parse_errors: 0,
        error: None,
        summary: None,
    };

    let rendered = format_live_sync_task_status_line(&task, "*", Some(48));
    assert!(rendered.chars().count() <= 48);
    assert!(rendered.contains('…'));
    assert!(!rendered.contains('\n'));
}

#[test]
fn live_sync_progress_bar_line_fits_requested_width() {
    with_env_vars(&[("NO_COLOR", Some("1"))], || {
        let task = SyncTaskGraphqlRecord {
            task_id: "sync-task-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_name: "bitloops".to_string(),
            repo_identity: "local/bitloops".to_string(),
            source: "init".to_string(),
            mode: "auto".to_string(),
            status: "running".to_string(),
            phase: "materialising_paths".to_string(),
            submitted_at_unix: 1,
            started_at_unix: Some(2),
            updated_at_unix: 3,
            completed_at_unix: None,
            queue_position: Some(1),
            tasks_ahead: Some(0),
            current_path: None,
            paths_total: 764,
            paths_completed: 472,
            paths_remaining: 292,
            paths_unchanged: 0,
            paths_added: 0,
            paths_changed: 0,
            paths_removed: 0,
            cache_hits: 0,
            cache_misses: 0,
            parse_errors: 0,
            error: None,
            summary: None,
        };

        let rendered = format_live_sync_progress_bar_line(&task, 0, Some(48));
        assert!(rendered.chars().count() <= 48);
        assert!(rendered.contains("472/764"));
        assert!(!rendered.contains('\n'));
    });
}
