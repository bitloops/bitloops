use super::*;
use crate::cli::{Cli, Commands};
use crate::daemon::{
    CapabilityEventQueueState, CapabilityEventQueueStatus, CapabilityEventRunRecord,
    CapabilityEventRunStatus, DaemonServiceMetadata, DaemonStatusReport, EnrichmentQueueMode,
    EnrichmentQueueState, EnrichmentQueueStatus, ServiceManagerKind, DevqlTaskKind,
    DevqlTaskKindCounts, DevqlTaskProgress, DevqlTaskQueueState, DevqlTaskQueueStatus,
    DevqlTaskRecord, DevqlTaskSource, DevqlTaskSpec, DevqlTaskStatus, RepoTaskControlState,
    SyncTaskMode, SyncTaskSpec,
};
use crate::host::devql::{SyncProgressPhase, SyncProgressUpdate};
use clap::Parser;
use std::io::Cursor;
use tempfile::TempDir;

use std::fs::{self, OpenOptions};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Clone, Default)]
struct SharedBuffer {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl SharedBuffer {
    fn contents(&self) -> String {
        String::from_utf8(self.bytes.lock().expect("shared buffer lock").clone())
            .expect("utf8 log output")
    }
}

impl Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes
            .lock()
            .expect("shared buffer lock")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn sample_capability_event_run(status: CapabilityEventRunStatus) -> CapabilityEventRunRecord {
    CapabilityEventRunRecord {
        run_id: "capability-event-run-1".to_string(),
        repo_id: "repo-1".to_string(),
        capability_id: "test_harness".to_string(),
        handler_id: "sync_completed".to_string(),
        event_kind: "sync_completed".to_string(),
        lane_key: "repo-1:test_harness:sync_completed".to_string(),
        event_payload_json: serde_json::json!({
            "repo_id": "repo-1",
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
        attempts: 1,
        submitted_at_unix: 1,
        started_at_unix: Some(2),
        updated_at_unix: 3,
        completed_at_unix: Some(4),
        error: Some("handler failed".to_string()),
    }
}

fn sample_capability_event_status() -> CapabilityEventQueueStatus {
    CapabilityEventQueueStatus {
        state: CapabilityEventQueueState {
            version: 1,
            pending_runs: 2,
            running_runs: 1,
            failed_runs: 3,
            completed_recent_runs: 4,
            last_action: Some("running".to_string()),
            last_updated_unix: 5,
        },
        persisted: true,
        current_repo_run: Some(sample_capability_event_run(
            CapabilityEventRunStatus::Running,
        )),
    }
}

fn write_log_lines(path: &Path, lines: &[String]) {
    let parent = path.parent().expect("daemon log parent");
    fs::create_dir_all(parent).expect("create daemon log dir");
    let mut rendered = lines.join("\n");
    if !rendered.is_empty() {
        rendered.push('\n');
    }
    fs::write(path, rendered).expect("write daemon log");
}

fn write_log_content(path: &Path, content: &str) {
    let parent = path.parent().expect("daemon log parent");
    fs::create_dir_all(parent).expect("create daemon log dir");
    fs::write(path, content).expect("write daemon log");
}

fn temp_log_path() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("logs").join("daemon.log");
    (dir, path)
}

fn test_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

#[test]
fn daemon_start_cli_parses_lifecycle_and_server_flags() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "daemon",
        "start",
        "--create-default-config",
        "--bootstrap-local-stores",
        "-d",
        "--host",
        "127.0.0.1",
        "--port",
        "6100",
        "--http",
        "--recheck-local-dashboard-net",
        "--bundle-dir",
        "/tmp/bundle",
    ])
    .expect("daemon start should parse");

    let Some(Commands::Daemon(daemon)) = parsed.command else {
        panic!("expected daemon command");
    };
    let Some(DaemonCommand::Start(start)) = daemon.command else {
        panic!("expected daemon start command");
    };

    assert!(start.create_default_config);
    assert!(start.bootstrap_local_stores);
    assert!(start.detached);
    assert!(!start.until_stopped);
    assert_eq!(start.host.as_deref(), Some("127.0.0.1"));
    assert_eq!(start.port, 6100);
    assert!(start.http);
    assert!(start.recheck_local_dashboard_net);
    assert_eq!(
        start.bundle_dir,
        Some(std::path::PathBuf::from("/tmp/bundle"))
    );
    assert_eq!(start.telemetry, None);
    assert!(!start.no_telemetry);
}

#[test]
fn daemon_enable_cli_parses_install_embeddings_flag() {
    let parsed = Cli::try_parse_from(["bitloops", "daemon", "enable", "--install-embeddings"])
        .expect("daemon enable should parse");

    let Some(Commands::Daemon(daemon)) = parsed.command else {
        panic!("expected daemon command");
    };
    let Some(DaemonCommand::Enable(enable)) = daemon.command else {
        panic!("expected daemon enable command");
    };

    assert!(enable.install_embeddings);
}

#[test]
fn daemon_start_cli_parses_telemetry_flags() {
    let parsed = Cli::try_parse_from(["bitloops", "daemon", "start", "--telemetry=false"])
        .expect("daemon start should parse telemetry=false");

    let Some(Commands::Daemon(daemon)) = parsed.command else {
        panic!("expected daemon command");
    };
    let Some(DaemonCommand::Start(start)) = daemon.command else {
        panic!("expected daemon start command");
    };

    assert_eq!(start.telemetry, Some(false));
    assert!(!start.no_telemetry);

    let parsed = Cli::try_parse_from(["bitloops", "daemon", "start", "--no-telemetry"])
        .expect("daemon start should parse no-telemetry");

    let Some(Commands::Daemon(daemon)) = parsed.command else {
        panic!("expected daemon command");
    };
    let Some(DaemonCommand::Start(start)) = daemon.command else {
        panic!("expected daemon start command");
    };

    assert_eq!(start.telemetry, None);
    assert!(start.no_telemetry);
}

#[test]
fn daemon_start_rejects_create_default_config_with_explicit_config() {
    let err = Cli::try_parse_from([
        "bitloops",
        "daemon",
        "start",
        "--config",
        "/tmp/bitloops.toml",
        "--create-default-config",
    ])
    .err()
    .expect("daemon start should reject conflicting config bootstrap flags");

    assert!(err.to_string().contains("--create-default-config"));
}

#[test]
fn daemon_start_accepts_bootstrap_local_stores_with_explicit_config() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "daemon",
        "start",
        "--config",
        "/tmp/bitloops.toml",
        "--bootstrap-local-stores",
    ])
    .expect("daemon start should accept explicit config store bootstrap");

    let Some(Commands::Daemon(daemon)) = parsed.command else {
        panic!("expected daemon command");
    };
    let Some(DaemonCommand::Start(start)) = daemon.command else {
        panic!("expected daemon start command");
    };

    assert_eq!(
        start.config,
        Some(std::path::PathBuf::from("/tmp/bitloops.toml"))
    );
    assert!(start.bootstrap_local_stores);
    assert!(!start.create_default_config);
}

#[test]
fn daemon_logs_cli_parses_tail_follow_and_path_flags() {
    let parsed = Cli::try_parse_from(["bitloops", "daemon", "logs", "--tail", "25", "--follow"])
        .expect("daemon logs should parse");

    let Some(Commands::Daemon(daemon)) = parsed.command else {
        panic!("expected daemon command");
    };
    let Some(DaemonCommand::Logs(args)) = daemon.command else {
        panic!("expected daemon logs command");
    };

    assert_eq!(args.tail, Some(25));
    assert!(args.follow);
    assert!(!args.path);
}

#[test]
fn daemon_logs_cli_rejects_conflicting_path_flags() {
    let err = Cli::try_parse_from(["bitloops", "daemon", "logs", "--path", "--follow"])
        .err()
        .expect("daemon logs should reject --path with --follow");
    assert!(err.to_string().contains("--path"));

    let err = Cli::try_parse_from(["bitloops", "daemon", "logs", "--path", "--tail", "5"])
        .err()
        .expect("daemon logs should reject --path with --tail");
    assert!(err.to_string().contains("--path"));
}

#[test]
fn daemon_logs_cli_rejects_lines_flag() {
    let err = Cli::try_parse_from(["bitloops", "daemon", "logs", "--lines", "5"])
        .err()
        .expect("daemon logs should reject removed --lines flag");
    assert!(err.to_string().contains("--lines"));
}

#[tokio::test]
async fn run_start_requires_explicit_bootstrap_when_default_config_is_missing() {
    let mut out = Vec::new();
    let mut input = Cursor::new(Vec::<u8>::new());

    let err = resolve_start_preflight_for_tests(
        &DaemonStartArgs {
            config: None,
            create_default_config: false,
            bootstrap_local_stores: false,
            detached: false,
            until_stopped: false,
            host: None,
            port: crate::api::DEFAULT_DASHBOARD_PORT,
            http: false,
            recheck_local_dashboard_net: false,
            bundle_dir: None,
            telemetry: None,
            no_telemetry: false,
        },
        &mut out,
        &mut input,
        true,
        false,
    )
    .expect_err("plain start should require explicit bootstrap");

    assert_eq!(err.to_string(), missing_default_daemon_bootstrap_message());
}

#[test]
fn start_preflight_accepts_default_config_bootstrap_and_then_prompts_for_telemetry() {
    let mut out = Vec::new();
    let mut input = Cursor::new(b"\n\n".to_vec());

    let decision = resolve_start_preflight_for_tests(
        &DaemonStartArgs {
            config: None,
            create_default_config: false,
            bootstrap_local_stores: false,
            detached: false,
            until_stopped: false,
            host: None,
            port: crate::api::DEFAULT_DASHBOARD_PORT,
            http: false,
            recheck_local_dashboard_net: false,
            bundle_dir: None,
            telemetry: None,
            no_telemetry: false,
        },
        &mut out,
        &mut input,
        true,
        true,
    )
    .expect("start preflight should prompt for bootstrap and telemetry");

    let rendered = String::from_utf8(out).expect("utf8 output");
    assert!(decision.create_default_config);
    assert_eq!(decision.startup_telemetry, Some(true));
    assert!(rendered.contains(
        "No global Bitloops daemon config was found. Set up the default configuration? [Y/n]"
    ));
    assert!(rendered.contains("Help us improve Bitloops"));
    assert!(rendered.contains("Enable anonymous telemetry? [Y/n]"));
}

#[test]
fn start_preflight_reuses_missing_config_error_when_user_declines_bootstrap() {
    let mut out = Vec::new();
    let mut input = Cursor::new(b"n\n".to_vec());

    let err = resolve_start_preflight_for_tests(
        &DaemonStartArgs {
            config: None,
            create_default_config: false,
            bootstrap_local_stores: false,
            detached: false,
            until_stopped: false,
            host: None,
            port: crate::api::DEFAULT_DASHBOARD_PORT,
            http: false,
            recheck_local_dashboard_net: false,
            bundle_dir: None,
            telemetry: None,
            no_telemetry: false,
        },
        &mut out,
        &mut input,
        true,
        true,
    )
    .expect_err("declining bootstrap should fail with the missing-config error");

    let rendered = String::from_utf8(out).expect("utf8 output");
    assert_eq!(err.to_string(), missing_default_daemon_bootstrap_message());
    assert!(rendered.contains(
        "No global Bitloops daemon config was found. Set up the default configuration? [Y/n]"
    ));
    assert!(!rendered.contains("Enable anonymous telemetry? [Y/n]"));
}

#[test]
fn start_preflight_uses_explicit_telemetry_choice_without_prompting() {
    let mut out = Vec::new();
    let mut input = Cursor::new(b"".to_vec());

    let decision = resolve_start_preflight_for_tests(
        &DaemonStartArgs {
            config: None,
            create_default_config: true,
            bootstrap_local_stores: false,
            detached: false,
            until_stopped: false,
            host: None,
            port: crate::api::DEFAULT_DASHBOARD_PORT,
            http: false,
            recheck_local_dashboard_net: false,
            bundle_dir: None,
            telemetry: Some(false),
            no_telemetry: false,
        },
        &mut out,
        &mut input,
        true,
        true,
    )
    .expect("explicit telemetry flag should suppress prompting");

    let rendered = String::from_utf8(out).expect("utf8 output");
    assert!(decision.create_default_config);
    assert_eq!(decision.startup_telemetry, Some(false));
    assert!(!rendered.contains("Help us improve Bitloops"));
    assert!(!rendered.contains("Enable anonymous telemetry? [Y/n]"));
}

#[test]
fn daemon_enrichments_cli_parses_controls() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "daemon",
        "enrichments",
        "pause",
        "--reason",
        "maintenance",
    ])
    .expect("daemon enrichments should parse");

    let Some(Commands::Daemon(daemon)) = parsed.command else {
        panic!("expected daemon command");
    };
    let Some(DaemonCommand::Enrichments(enrichments)) = daemon.command else {
        panic!("expected daemon enrichments command");
    };
    let Some(EnrichmentCommand::Pause(pause)) = enrichments.command else {
        panic!("expected pause command");
    };

    assert_eq!(pause.reason.as_deref(), Some("maintenance"));
}

#[test]
fn status_lines_show_global_supervisor_install_and_state() {
    let (_log_dir, log_path) = temp_log_path();

    let report = DaemonStatusReport {
        runtime: None,
        service: Some(DaemonServiceMetadata {
            version: 1,
            config_path: std::path::PathBuf::from("/tmp/bitloops/config.toml"),
            config_root: std::path::PathBuf::from("/tmp"),
            manager: ServiceManagerKind::Launchd,
            service_name: "com.bitloops.daemon".to_string(),
            service_file: None,
            config: DashboardServerConfig {
                host: None,
                port: crate::api::DEFAULT_DASHBOARD_PORT,
                no_open: true,
                force_http: false,
                recheck_local_dashboard_net: false,
                bundle_dir: None,
            },
            last_url: Some("https://127.0.0.1:5173".to_string()),
            last_pid: None,
        }),
        service_running: false,
        health: None,
        capability_events: None,
        enrichment: Some(EnrichmentQueueStatus {
            state: EnrichmentQueueState {
                version: 1,
                mode: EnrichmentQueueMode::Paused,
                pending_jobs: 2,
                pending_semantic_jobs: 1,
                pending_embedding_jobs: 1,
                pending_clone_edges_rebuild_jobs: 0,
                running_jobs: 1,
                running_semantic_jobs: 1,
                running_embedding_jobs: 0,
                running_clone_edges_rebuild_jobs: 0,
                failed_jobs: 3,
                failed_semantic_jobs: 1,
                failed_embedding_jobs: 1,
                failed_clone_edges_rebuild_jobs: 1,
                retried_failed_jobs: 4,
                last_action: Some("paused".to_string()),
                last_updated_unix: 0,
                paused_reason: Some("maintenance".to_string()),
            },
            persisted: true,
        }),
        devql_tasks: None,
    };

    assert_eq!(
        super::display::status_lines_with_log_path(&report, &log_path),
        vec![
            "Bitloops daemon: stopped".to_string(),
            "Mode: always-on service".to_string(),
            "Config: /tmp/bitloops/config.toml".to_string(),
            format!("Log file: {}", log_path.display()),
            "Supervisor service: com.bitloops.daemon (launchd, installed)".to_string(),
            "Supervisor state: stopped".to_string(),
            "Last URL: https://127.0.0.1:5173".to_string(),
            "Enrichment mode: paused".to_string(),
            "Enrichment pending jobs: 2".to_string(),
            "Enrichment pending semantic jobs: 1".to_string(),
            "Enrichment pending embedding jobs: 1".to_string(),
            "Enrichment pending clone-edge rebuild jobs: 0".to_string(),
            "Enrichment running jobs: 1".to_string(),
            "Enrichment running semantic jobs: 1".to_string(),
            "Enrichment running embedding jobs: 0".to_string(),
            "Enrichment running clone-edge rebuild jobs: 0".to_string(),
            "Enrichment failed jobs: 3".to_string(),
            "Enrichment failed semantic jobs: 1".to_string(),
            "Enrichment failed embedding jobs: 1".to_string(),
            "Enrichment failed clone-edge rebuild jobs: 1".to_string(),
            "Enrichment retried failed jobs: 4".to_string(),
            "Enrichment last action: paused".to_string(),
            "Enrichment pause reason: maintenance".to_string(),
            "Enrichment persisted: yes".to_string(),
        ]
    );
}

#[test]
fn status_lines_show_log_file_for_running_daemon() {
    let (_log_dir, log_path) = temp_log_path();

    let report = DaemonStatusReport {
        runtime: Some(crate::daemon::DaemonRuntimeState {
            version: 1,
            config_path: std::path::PathBuf::from("/tmp/bitloops/config.toml"),
            config_root: std::path::PathBuf::from("/tmp"),
            pid: 42,
            mode: DaemonMode::Foreground,
            service_name: None,
            url: "http://127.0.0.1:5667".to_string(),
            host: "127.0.0.1".to_string(),
            port: 5667,
            bundle_dir: std::path::PathBuf::from("/tmp/bundle"),
            relational_db_path: std::path::PathBuf::from("/tmp/relational.db"),
            events_db_path: std::path::PathBuf::from("/tmp/events.duckdb"),
            blob_store_path: std::path::PathBuf::from("/tmp/blob"),
            repo_registry_path: std::path::PathBuf::from("/tmp/repo-registry.json"),
            binary_fingerprint: "abc".to_string(),
            updated_at_unix: 1,
        }),
        service: None,
        service_running: false,
        health: None,
        capability_events: None,
        enrichment: None,
        devql_tasks: None,
    };

    let lines = super::display::status_lines_with_log_path(&report, &log_path);
    assert!(lines.contains(&format!("Log file: {}", log_path.display())));
}

#[test]
fn status_lines_show_log_file_when_daemon_is_stopped() {
    let (_log_dir, log_path) = temp_log_path();

    let report = DaemonStatusReport {
        runtime: None,
        service: None,
        service_running: false,
        health: None,
        capability_events: None,
        enrichment: None,
        devql_tasks: None,
    };

    assert_eq!(
        super::display::status_lines_with_log_path(&report, &log_path),
        vec![
            "Bitloops daemon: stopped".to_string(),
            "Mode: not running".to_string(),
            format!("Log file: {}", log_path.display()),
        ]
    );
}

#[test]
fn send_session_end_for_all_sessions_clears_store_with_valid_json() {
    let state_root = TempDir::new().expect("temp dir");
    let repo_root = TempDir::new().expect("temp dir");
    let state_dir = state_root.path().join("bitloops");

    let mut store = crate::telemetry::sessions::SessionStore::default();
    let _ = store.get_or_create_session(repo_root.path());
    store
        .save(&state_dir)
        .expect("save session store before stop");

    send_session_end_for_all_sessions_in(&state_dir);

    let cleared_path = state_dir.join("telemetry_sessions.json");
    let cleared_content = fs::read_to_string(&cleared_path).expect("read cleared session store");
    let cleared_store: crate::telemetry::sessions::SessionStore =
        serde_json::from_str(&cleared_content).expect("parse cleared session store");
    assert_eq!(cleared_store.sessions().count(), 0);
}

#[test]
fn status_lines_include_sync_queue_and_current_repo_task() {
    let report = DaemonStatusReport {
        runtime: None,
        service: None,
        service_running: false,
        health: None,
        capability_events: None,
        enrichment: None,
        devql_tasks: Some(DevqlTaskQueueStatus {
            state: DevqlTaskQueueState {
                version: 1,
                queued_tasks: 2,
                running_tasks: 1,
                failed_tasks: 3,
                completed_recent_tasks: 4,
                by_kind: vec![
                    DevqlTaskKindCounts {
                        kind: DevqlTaskKind::Sync,
                        queued_tasks: 2,
                        running_tasks: 1,
                        failed_tasks: 3,
                        completed_recent_tasks: 4,
                    },
                    DevqlTaskKindCounts {
                        kind: DevqlTaskKind::Ingest,
                        queued_tasks: 0,
                        running_tasks: 0,
                        failed_tasks: 0,
                        completed_recent_tasks: 0,
                    },
                ],
                last_action: Some("running".to_string()),
                last_updated_unix: 0,
            },
            persisted: true,
            current_repo_tasks: vec![DevqlTaskRecord {
                task_id: "sync-task-1".to_string(),
                repo_id: "repo-1".to_string(),
                repo_name: "demo".to_string(),
                repo_provider: "local".to_string(),
                repo_organisation: "local".to_string(),
                repo_identity: "local/demo".to_string(),
                daemon_config_root: std::path::PathBuf::from("/tmp/repo"),
                repo_root: std::path::PathBuf::from("/tmp/repo"),
                kind: DevqlTaskKind::Sync,
                source: DevqlTaskSource::ManualCli,
                spec: DevqlTaskSpec::Sync(SyncTaskSpec {
                    mode: SyncTaskMode::Full,
                }),
                status: DevqlTaskStatus::Running,
                submitted_at_unix: 1,
                started_at_unix: Some(2),
                updated_at_unix: 3,
                completed_at_unix: None,
                queue_position: Some(1),
                tasks_ahead: Some(0),
                progress: DevqlTaskProgress::Sync(SyncProgressUpdate {
                    phase: SyncProgressPhase::ExtractingPaths,
                    current_path: Some("src/lib.rs".to_string()),
                    paths_total: 10,
                    paths_completed: 4,
                    paths_remaining: 6,
                    paths_unchanged: 1,
                    paths_added: 2,
                    paths_changed: 6,
                    paths_removed: 1,
                    cache_hits: 3,
                    cache_misses: 2,
                    parse_errors: 0,
                }),
                error: None,
                result: None,
            }],
            current_repo_control: Some(RepoTaskControlState {
                repo_id: "repo-1".to_string(),
                paused: false,
                paused_reason: None,
                updated_at_unix: 3,
            }),
        }),
    };

    let lines = status_lines(&report);
    assert!(lines.contains(&"DevQL queued tasks: 2".to_string()));
    assert!(lines.contains(&"DevQL running tasks: 1".to_string()));
    assert!(lines.contains(&"DevQL failed tasks: 3".to_string()));
    assert!(lines.contains(&"DevQL completed recent tasks: 4".to_string()));
    assert!(lines.contains(&"DevQL last action: running".to_string()));
    assert!(lines.contains(
        &"Current repo task: sync-task-1 (running, kind=sync, source=manual_cli)".to_string()
    ));
    assert!(lines.contains(&"Current repo sync phase: extracting_paths".to_string()));
    assert!(
        lines
            .contains(&"Current repo sync progress: 4/10 paths complete (6 remaining)".to_string())
    );
    assert!(lines.contains(&"Current repo sync path: src/lib.rs".to_string()));
    assert!(lines.contains(&"DevQL persisted: yes".to_string()));
}

#[test]
fn status_lines_include_capability_event_queue_and_current_repo_run() {
    let report = DaemonStatusReport {
        runtime: None,
        service: None,
        service_running: false,
        health: None,
        capability_events: Some({
            let mut status = sample_capability_event_status();
            status.current_repo_run = Some(sample_capability_event_run(
                CapabilityEventRunStatus::Failed,
            ));
            status
        }),
        enrichment: None,
        devql_tasks: None,
    };

    let lines = status_lines(&report);
    assert!(lines.contains(&"Capability event queue: available".to_string()));
    assert!(lines.contains(&"Capability event pending runs: 2".to_string()));
    assert!(lines.contains(&"Capability event running runs: 1".to_string()));
    assert!(lines.contains(&"Capability event failed runs: 3".to_string()));
    assert!(lines.contains(&"Capability event completed recent runs: 4".to_string()));
    assert!(lines.contains(&"Capability event last action: running".to_string()));
    assert!(lines.contains(
        &"Current repo capability event run: capability-event-run-1 (failed, capability=test_harness, handler=sync_completed, event_kind=sync_completed)".to_string()
    ));
    assert!(lines.contains(&"Current repo capability event error: handler failed".to_string()));
    assert!(lines.contains(&"Capability event persisted: yes".to_string()));
}

#[test]
fn run_status_writes_json_when_requested() {
    let report = DaemonStatusReport {
        runtime: None,
        service: None,
        service_running: false,
        health: None,
        capability_events: Some(sample_capability_event_status()),
        enrichment: None,
        devql_tasks: None,
    };
    let mut out = SharedBuffer::default();

    write_status_output(&report, true, &mut out).expect("write daemon status json");

    let rendered = out.contents();
    let value: serde_json::Value =
        serde_json::from_str(rendered.trim()).expect("parse daemon status json");
    assert_eq!(
        value["capability_events"]["state"]["pending_runs"],
        serde_json::json!(2)
    );
    assert_eq!(
        value["capability_events"]["state"]["running_runs"],
        serde_json::json!(1)
    );
    assert_eq!(
        value["capability_events"]["current_repo_run"]["capability_id"],
        serde_json::json!("test_harness")
    );
    assert_eq!(
        value["capability_events"]["current_repo_run"]["handler_id"],
        serde_json::json!("sync_completed")
    );
    assert_eq!(
        value["capability_events"]["current_repo_run"]["error"],
        serde_json::json!("handler failed")
    );
}

#[test]
fn run_logs_prints_default_last_two_hundred_lines() {
    let (_log_dir, log_path) = temp_log_path();
    let lines = (1..=250)
        .map(|idx| format!("{{\"line\":{idx}}}"))
        .collect::<Vec<_>>();
    write_log_lines(&log_path, &lines);
    let mut out = Vec::new();

    test_runtime()
        .block_on(run_logs_with_io_at_path(
            DaemonLogsArgs::default(),
            &mut out,
            log_path,
        ))
        .expect("run daemon logs");

    let rendered = String::from_utf8(out).expect("utf8 output");
    let output_lines = rendered.lines().collect::<Vec<_>>();
    assert_eq!(output_lines.len(), 200);
    assert_eq!(output_lines.first().copied(), Some("{\"line\":51}"));
    assert_eq!(output_lines.last().copied(), Some("{\"line\":250}"));
}

#[test]
fn run_logs_honours_explicit_line_count() {
    let (_log_dir, log_path) = temp_log_path();
    let lines = (1..=5)
        .map(|idx| format!("{{\"line\":{idx}}}"))
        .collect::<Vec<_>>();
    write_log_lines(&log_path, &lines);
    let mut out = Vec::new();

    test_runtime()
        .block_on(run_logs_with_io_at_path(
            DaemonLogsArgs {
                tail: Some(3),
                follow: false,
                path: false,
            },
            &mut out,
            log_path,
        ))
        .expect("run daemon logs");

    assert_eq!(
        String::from_utf8(out).expect("utf8 output"),
        "{\"line\":3}\n{\"line\":4}\n{\"line\":5}\n"
    );
}

#[test]
fn run_logs_prints_log_path() {
    let (_log_dir, log_path) = temp_log_path();
    let mut out = Vec::new();

    test_runtime()
        .block_on(run_logs_with_io_at_path(
            DaemonLogsArgs {
                tail: None,
                follow: false,
                path: true,
            },
            &mut out,
            log_path.clone(),
        ))
        .expect("print daemon log path");

    assert_eq!(
        String::from_utf8(out).expect("utf8 output"),
        format!("{}\n", log_path.display())
    );
}

#[test]
fn run_logs_reports_missing_file_with_expected_path() {
    let (_log_dir, log_path) = temp_log_path();

    let err = test_runtime()
        .block_on(run_logs_with_io_at_path(
            DaemonLogsArgs::default(),
            &mut Vec::new(),
            log_path.clone(),
        ))
        .expect_err("daemon logs should fail when file is missing");

    assert!(err.to_string().contains(&log_path.display().to_string()));
}

#[test]
fn follow_log_file_streams_appended_lines() {
    let (_log_dir, log_path) = temp_log_path();
    write_log_lines(&log_path, &["{\"line\":1}".to_string()]);

    let stop = Arc::new(AtomicBool::new(false));
    let stop_signal = stop.clone();
    let path_for_thread = log_path.clone();
    let mut writer = SharedBuffer::default();
    let shared = writer.clone();
    let handle = thread::spawn(move || {
        follow_log_file(
            &path_for_thread,
            |chunk| {
                write!(writer, "{chunk}")?;
                writer.flush()?;
                Ok(())
            },
            &|| stop_signal.load(Ordering::SeqCst),
            Duration::from_millis(10),
        )
    });

    thread::sleep(Duration::from_millis(30));
    let mut file = OpenOptions::new()
        .append(true)
        .open(&log_path)
        .expect("open daemon log for append");
    writeln!(file, "{{\"line\":2}}").expect("append followed line");
    writeln!(file, "{{\"line\":3}}").expect("append followed line");
    file.flush().expect("flush appended daemon log");

    thread::sleep(Duration::from_millis(60));
    stop.store(true, Ordering::SeqCst);
    handle
        .join()
        .expect("join follow thread")
        .expect("follow daemon log");

    assert_eq!(shared.contents(), "{\"line\":2}\n{\"line\":3}\n");
}

#[test]
fn tail_log_file_handles_file_without_trailing_newline() {
    let dir = TempDir::new().expect("temp dir");
    let log_path = dir.path().join("daemon.log");
    write_log_content(&log_path, "{\"line\":1}\n{\"line\":2}\n{\"line\":3}");

    let lines = tail_log_file(&log_path, 2).expect("tail daemon log");

    assert_eq!(lines, vec!["{\"line\":2}", "{\"line\":3}"]);
}

#[test]
fn tail_log_file_reads_across_reverse_scan_blocks() {
    let dir = TempDir::new().expect("temp dir");
    let log_path = dir.path().join("daemon.log");
    let long_prefix = "x".repeat(9000);
    write_log_content(
        &log_path,
        &format!("{long_prefix}\n{{\"line\":2}}\n{{\"line\":3}}\n"),
    );

    let lines = tail_log_file(&log_path, 2).expect("tail daemon log");

    assert_eq!(lines, vec!["{\"line\":2}", "{\"line\":3}"]);
}

#[test]
fn tail_log_file_returns_all_lines_when_tail_exceeds_file_length() {
    let dir = TempDir::new().expect("temp dir");
    let log_path = dir.path().join("daemon.log");
    write_log_content(&log_path, "{\"line\":1}\n{\"line\":2}\n");

    let lines = tail_log_file(&log_path, 10).expect("tail daemon log");

    assert_eq!(lines, vec!["{\"line\":1}", "{\"line\":2}"]);
}

#[tokio::test(flavor = "current_thread")]
async fn run_logs_follow_stops_when_async_shutdown_resolves() {
    let (_log_dir, log_path) = temp_log_path();
    write_log_lines(&log_path, &["{\"line\":1}".to_string()]);
    let mut out = SharedBuffer::default();

    tokio::time::timeout(
        Duration::from_millis(250),
        run_logs_with_io_and_shutdown_at_path(
            DaemonLogsArgs {
                tail: Some(1),
                follow: true,
                path: false,
            },
            &mut out,
            async {
                tokio::time::sleep(Duration::from_millis(30)).await;
            },
            Duration::from_millis(10),
            log_path,
        ),
    )
    .await
    .expect("follow should not block the runtime")
    .expect("follow should stop cleanly");

    assert_eq!(out.contents(), "{\"line\":1}\n");
}
