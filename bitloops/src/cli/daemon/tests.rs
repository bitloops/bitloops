use super::*;
use crate::cli::{Cli, Commands};
use crate::daemon::{
    DaemonServiceMetadata, DaemonStatusReport, EnrichmentQueueMode, EnrichmentQueueState,
    EnrichmentQueueStatus, ServiceManagerKind,
};
use crate::test_support::process_state::enter_process_state;
use clap::Parser;
use std::io::Cursor;
use tempfile::TempDir;

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
    };

    assert_eq!(
        status_lines(&report),
        vec![
            "Bitloops daemon: stopped".to_string(),
            "Mode: always-on service".to_string(),
            "Config: /tmp/bitloops/config.toml".to_string(),
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
