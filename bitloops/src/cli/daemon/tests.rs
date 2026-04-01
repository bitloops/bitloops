use super::*;
use crate::cli::{Cli, Commands};
use crate::daemon::{DaemonServiceMetadata, DaemonStatusReport, ServiceManagerKind};
use crate::test_support::process_state::enter_process_state;
use clap::Parser;
use std::fs::{self, OpenOptions};
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

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

fn write_log_lines(path: &Path, lines: &[String]) {
    let parent = path.parent().expect("daemon log parent");
    fs::create_dir_all(parent).expect("create daemon log dir");
    let mut rendered = lines.join("\n");
    if !rendered.is_empty() {
        rendered.push('\n');
    }
    fs::write(path, rendered).expect("write daemon log");
}

#[test]
fn daemon_start_cli_parses_lifecycle_and_server_flags() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "daemon",
        "start",
        "--create-default-config",
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
    let config_root = TempDir::new().expect("temp dir");
    let data_root = TempDir::new().expect("temp dir");
    let cache_root = TempDir::new().expect("temp dir");
    let state_root = TempDir::new().expect("temp dir");
    let config_root_str = config_root.path().to_string_lossy().to_string();
    let data_root_str = data_root.path().to_string_lossy().to_string();
    let cache_root_str = cache_root.path().to_string_lossy().to_string();
    let state_root_str = state_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[
            (
                "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
                Some(config_root_str.as_str()),
            ),
            (
                "BITLOOPS_TEST_DATA_DIR_OVERRIDE",
                Some(data_root_str.as_str()),
            ),
            (
                "BITLOOPS_TEST_CACHE_DIR_OVERRIDE",
                Some(cache_root_str.as_str()),
            ),
            (
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(state_root_str.as_str()),
            ),
        ],
    );

    let err = run_start(DaemonStartArgs {
        config: None,
        create_default_config: false,
        detached: false,
        until_stopped: false,
        host: None,
        port: crate::api::DEFAULT_DASHBOARD_PORT,
        http: false,
        recheck_local_dashboard_net: false,
        bundle_dir: None,
        telemetry: None,
        no_telemetry: false,
    })
    .await
    .expect_err("plain start should require explicit bootstrap");

    assert_eq!(err.to_string(), missing_default_daemon_bootstrap_message());
}

#[test]
fn start_preflight_accepts_default_config_bootstrap_and_then_prompts_for_telemetry() {
    let config_root = TempDir::new().expect("temp dir");
    let data_root = TempDir::new().expect("temp dir");
    let cache_root = TempDir::new().expect("temp dir");
    let state_root = TempDir::new().expect("temp dir");
    let config_root_str = config_root.path().to_string_lossy().to_string();
    let data_root_str = data_root.path().to_string_lossy().to_string();
    let cache_root_str = cache_root.path().to_string_lossy().to_string();
    let state_root_str = state_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[
            (
                "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
                Some(config_root_str.as_str()),
            ),
            (
                "BITLOOPS_TEST_DATA_DIR_OVERRIDE",
                Some(data_root_str.as_str()),
            ),
            (
                "BITLOOPS_TEST_CACHE_DIR_OVERRIDE",
                Some(cache_root_str.as_str()),
            ),
            (
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(state_root_str.as_str()),
            ),
            ("BITLOOPS_TEST_TTY", Some("1")),
        ],
    );
    let mut out = Vec::new();
    let mut input = Cursor::new(b"\n\n".to_vec());

    let decision = resolve_start_preflight(
        &DaemonStartArgs {
            config: None,
            create_default_config: false,
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
    let config_root = TempDir::new().expect("temp dir");
    let data_root = TempDir::new().expect("temp dir");
    let cache_root = TempDir::new().expect("temp dir");
    let state_root = TempDir::new().expect("temp dir");
    let config_root_str = config_root.path().to_string_lossy().to_string();
    let data_root_str = data_root.path().to_string_lossy().to_string();
    let cache_root_str = cache_root.path().to_string_lossy().to_string();
    let state_root_str = state_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[
            (
                "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
                Some(config_root_str.as_str()),
            ),
            (
                "BITLOOPS_TEST_DATA_DIR_OVERRIDE",
                Some(data_root_str.as_str()),
            ),
            (
                "BITLOOPS_TEST_CACHE_DIR_OVERRIDE",
                Some(cache_root_str.as_str()),
            ),
            (
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(state_root_str.as_str()),
            ),
            ("BITLOOPS_TEST_TTY", Some("1")),
        ],
    );
    let mut out = Vec::new();
    let mut input = Cursor::new(b"n\n".to_vec());

    let err = resolve_start_preflight(
        &DaemonStartArgs {
            config: None,
            create_default_config: false,
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
    let config_root = TempDir::new().expect("temp dir");
    let data_root = TempDir::new().expect("temp dir");
    let cache_root = TempDir::new().expect("temp dir");
    let state_root = TempDir::new().expect("temp dir");
    let config_root_str = config_root.path().to_string_lossy().to_string();
    let data_root_str = data_root.path().to_string_lossy().to_string();
    let cache_root_str = cache_root.path().to_string_lossy().to_string();
    let state_root_str = state_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[
            (
                "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
                Some(config_root_str.as_str()),
            ),
            (
                "BITLOOPS_TEST_DATA_DIR_OVERRIDE",
                Some(data_root_str.as_str()),
            ),
            (
                "BITLOOPS_TEST_CACHE_DIR_OVERRIDE",
                Some(cache_root_str.as_str()),
            ),
            (
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(state_root_str.as_str()),
            ),
            ("BITLOOPS_TEST_TTY", Some("1")),
        ],
    );
    let mut out = Vec::new();
    let mut input = Cursor::new(b"".to_vec());

    let decision = resolve_start_preflight(
        &DaemonStartArgs {
            config: None,
            create_default_config: true,
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
    )
    .expect("explicit telemetry flag should suppress prompting");

    let rendered = String::from_utf8(out).expect("utf8 output");
    assert!(decision.create_default_config);
    assert_eq!(decision.startup_telemetry, Some(false));
    assert!(!rendered.contains("Help us improve Bitloops"));
    assert!(!rendered.contains("Enable anonymous telemetry? [Y/n]"));
}

#[test]
fn status_lines_show_global_supervisor_install_and_state() {
    let state_root = TempDir::new().expect("temp dir");
    let state_root_str = state_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[(
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(state_root_str.as_str()),
        )],
    );

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
    };
    let log_path = daemon::daemon_log_file_path();

    assert_eq!(
        status_lines(&report),
        vec![
            "Bitloops daemon: stopped".to_string(),
            "Mode: always-on service".to_string(),
            "Config: /tmp/bitloops/config.toml".to_string(),
            format!("Log file: {}", log_path.display()),
            "Supervisor service: com.bitloops.daemon (launchd, installed)".to_string(),
            "Supervisor state: stopped".to_string(),
            "Last URL: https://127.0.0.1:5173".to_string(),
        ]
    );
}

#[test]
fn status_lines_show_log_file_for_running_daemon() {
    let state_root = TempDir::new().expect("temp dir");
    let state_root_str = state_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[(
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(state_root_str.as_str()),
        )],
    );

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
    };

    let lines = status_lines(&report);
    assert!(lines.contains(&format!(
        "Log file: {}",
        daemon::daemon_log_file_path().display()
    )));
}

#[test]
fn status_lines_show_log_file_when_daemon_is_stopped() {
    let state_root = TempDir::new().expect("temp dir");
    let state_root_str = state_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[(
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(state_root_str.as_str()),
        )],
    );

    let report = DaemonStatusReport {
        runtime: None,
        service: None,
        service_running: false,
        health: None,
    };

    assert_eq!(
        status_lines(&report),
        vec![
            "Bitloops daemon: stopped".to_string(),
            "Mode: not running".to_string(),
            format!("Log file: {}", daemon::daemon_log_file_path().display()),
        ]
    );
}

#[test]
fn run_logs_prints_default_last_two_hundred_lines() {
    let state_root = TempDir::new().expect("temp dir");
    let state_root_str = state_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[(
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(state_root_str.as_str()),
        )],
    );
    let log_path = daemon::daemon_log_file_path();
    let lines = (1..=250)
        .map(|idx| format!("{{\"line\":{idx}}}"))
        .collect::<Vec<_>>();
    write_log_lines(&log_path, &lines);
    let mut out = Vec::new();

    run_logs_with_io(DaemonLogsArgs::default(), &mut out).expect("run daemon logs");

    let rendered = String::from_utf8(out).expect("utf8 output");
    let output_lines = rendered.lines().collect::<Vec<_>>();
    assert_eq!(output_lines.len(), 200);
    assert_eq!(output_lines.first().copied(), Some("{\"line\":51}"));
    assert_eq!(output_lines.last().copied(), Some("{\"line\":250}"));
}

#[test]
fn run_logs_honours_explicit_line_count() {
    let state_root = TempDir::new().expect("temp dir");
    let state_root_str = state_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[(
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(state_root_str.as_str()),
        )],
    );
    let log_path = daemon::daemon_log_file_path();
    let lines = (1..=5)
        .map(|idx| format!("{{\"line\":{idx}}}"))
        .collect::<Vec<_>>();
    write_log_lines(&log_path, &lines);
    let mut out = Vec::new();

    run_logs_with_io(
        DaemonLogsArgs {
            tail: Some(3),
            follow: false,
            path: false,
        },
        &mut out,
    )
    .expect("run daemon logs");

    assert_eq!(
        String::from_utf8(out).expect("utf8 output"),
        "{\"line\":3}\n{\"line\":4}\n{\"line\":5}\n"
    );
}

#[test]
fn run_logs_prints_log_path() {
    let state_root = TempDir::new().expect("temp dir");
    let state_root_str = state_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[(
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(state_root_str.as_str()),
        )],
    );
    let mut out = Vec::new();

    run_logs_with_io(
        DaemonLogsArgs {
            tail: None,
            follow: false,
            path: true,
        },
        &mut out,
    )
    .expect("print daemon log path");

    assert_eq!(
        String::from_utf8(out).expect("utf8 output"),
        format!("{}\n", daemon::daemon_log_file_path().display())
    );
}

#[test]
fn run_logs_reports_missing_file_with_expected_path() {
    let state_root = TempDir::new().expect("temp dir");
    let state_root_str = state_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[(
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(state_root_str.as_str()),
        )],
    );

    let err = run_logs_with_io(DaemonLogsArgs::default(), &mut Vec::new())
        .expect_err("daemon logs should fail when file is missing");

    assert!(
        err.to_string()
            .contains(&daemon::daemon_log_file_path().display().to_string())
    );
}

#[test]
fn follow_log_file_streams_appended_lines() {
    let state_root = TempDir::new().expect("temp dir");
    let state_root_str = state_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[(
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(state_root_str.as_str()),
        )],
    );
    let log_path = daemon::daemon_log_file_path();
    write_log_lines(&log_path, &["{\"line\":1}".to_string()]);

    let stop = Arc::new(AtomicBool::new(false));
    let stop_signal = stop.clone();
    let path_for_thread = log_path.clone();
    let mut writer = SharedBuffer::default();
    let shared = writer.clone();
    let handle = thread::spawn(move || {
        follow_log_file(
            &path_for_thread,
            &mut writer,
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
