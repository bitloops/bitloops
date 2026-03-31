use std::io::{self, BufRead, Write};

use anyhow::{Result, bail};
use clap::{Args, Subcommand};

use crate::api::DashboardServerConfig;
use crate::cli::telemetry_consent;
use crate::config::{bootstrap_default_daemon_environment, default_daemon_config_exists};
use crate::daemon::{self, DaemonMode};
pub const MISSING_SUBCOMMAND_MESSAGE: &str = "missing subcommand. Use one of: `bitloops daemon start`, `bitloops daemon stop`, `bitloops daemon status`, `bitloops daemon restart`, `bitloops daemon enrichments`";

#[derive(Args, Debug, Clone, Default)]
pub struct DaemonArgs {
    #[command(subcommand)]
    pub command: Option<DaemonCommand>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DaemonCommand {
    /// Start the global Bitloops daemon.
    Start(DaemonStartArgs),
    /// Stop the global Bitloops daemon.
    Stop(DaemonStopArgs),
    /// Show global Bitloops daemon status.
    Status(DaemonStatusArgs),
    /// Restart the global Bitloops daemon.
    Restart(DaemonRestartArgs),
    /// Inspect or control the enrichment coordinator.
    Enrichments(EnrichmentArgs),
}

#[derive(Args, Debug, Clone)]
pub struct DaemonStartArgs {
    /// Path to the Bitloops daemon config file.
    #[arg(long, value_name = "PATH")]
    pub config: Option<std::path::PathBuf>,

    /// Create the default global daemon config and local default stores if missing.
    #[arg(long, default_value_t = false, conflicts_with = "config")]
    pub create_default_config: bool,

    /// Start detached instead of holding the current terminal open.
    #[arg(
        short = 'd',
        long,
        default_value_t = false,
        conflicts_with = "until_stopped"
    )]
    pub detached: bool,

    /// Install or refresh an always-on user-scoped service, then start it.
    #[arg(long, default_value_t = false, conflicts_with = "detached")]
    pub until_stopped: bool,

    /// Hostname to bind the daemon server to.
    #[arg(long)]
    pub host: Option<String>,

    /// Port to bind the daemon server to.
    #[arg(long, default_value_t = crate::api::DEFAULT_DASHBOARD_PORT)]
    pub port: u16,

    /// Force fast local HTTP mode. Requires `--host 127.0.0.1`.
    #[arg(long, default_value_t = false)]
    pub http: bool,

    /// Force a full local dashboard network recheck and refresh discovery hints.
    #[arg(long = "recheck-local-dashboard-net", default_value_t = false)]
    pub recheck_local_dashboard_net: bool,

    /// Path to the dashboard bundle directory (contains index.html).
    #[arg(long = "bundle-dir", alias = "bundle", value_name = "PATH")]
    pub bundle_dir: Option<std::path::PathBuf>,

    /// Enable anonymous telemetry for this CLI version.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub telemetry: Option<bool>,

    /// Disable anonymous telemetry for this CLI version.
    #[arg(
        long = "no-telemetry",
        conflicts_with = "telemetry",
        default_value_t = false
    )]
    pub no_telemetry: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DaemonStopArgs {
    /// Path to the Bitloops daemon config file.
    #[arg(long, value_name = "PATH")]
    pub config: Option<std::path::PathBuf>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DaemonStatusArgs {
    /// Path to the Bitloops daemon config file.
    #[arg(long, value_name = "PATH")]
    pub config: Option<std::path::PathBuf>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DaemonRestartArgs {
    /// Path to the Bitloops daemon config file.
    #[arg(long, value_name = "PATH")]
    pub config: Option<std::path::PathBuf>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct EnrichmentArgs {
    #[command(subcommand)]
    pub command: Option<EnrichmentCommand>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum EnrichmentCommand {
    /// Show enrichment queue status.
    Status(EnrichmentStatusArgs),
    /// Pause background enrichment work.
    Pause(EnrichmentPauseArgs),
    /// Resume background enrichment work.
    Resume(EnrichmentResumeArgs),
    /// Requeue failed enrichment work.
    RetryFailed(EnrichmentRetryFailedArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct EnrichmentStatusArgs {}

#[derive(Args, Debug, Clone, Default)]
pub struct EnrichmentResumeArgs {}

#[derive(Args, Debug, Clone, Default)]
pub struct EnrichmentRetryFailedArgs {}

#[derive(Args, Debug, Clone, Default)]
pub struct EnrichmentPauseArgs {
    /// Optional reason for pausing the queue.
    #[arg(long)]
    pub reason: Option<String>,
}

pub async fn run(args: DaemonArgs) -> Result<()> {
    let Some(command) = args.command else {
        bail!(MISSING_SUBCOMMAND_MESSAGE);
    };

    match command {
        DaemonCommand::Start(args) => run_start(args).await,
        DaemonCommand::Stop(args) => run_stop(args).await,
        DaemonCommand::Status(args) => run_status(args).await,
        DaemonCommand::Restart(args) => run_restart(args).await,
        DaemonCommand::Enrichments(args) => run_enrichments(args).await,
    }
}

pub async fn run_start(args: DaemonStartArgs) -> Result<()> {
    let mut out = io::stdout().lock();
    let stdin = io::stdin();
    let mut input = stdin.lock();
    run_start_with_io(args, &mut out, &mut input).await
}

async fn run_start_with_io(
    args: DaemonStartArgs,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<()> {
    print_legacy_repo_data_warnings();
    let preflight = resolve_start_preflight(&args, out, input)?;

    if preflight.create_default_config {
        let _ = bootstrap_default_daemon_environment()?;
    }
    let daemon_config = daemon::resolve_daemon_config(args.config.as_deref())?;
    let config = build_server_config(&args);

    if args.until_stopped {
        let state =
            daemon::start_service(&daemon_config, config, preflight.startup_telemetry).await?;
        println!(
            "Bitloops daemon started as an always-on service at {}",
            state.url
        );
        return Ok(());
    }

    let report = daemon::status().await?;
    if report.service.is_some() {
        let state =
            daemon::start_service(&daemon_config, config, preflight.startup_telemetry).await?;
        println!(
            "Bitloops daemon started under the always-on service at {}",
            state.url
        );
        return Ok(());
    }

    if args.detached {
        let state =
            daemon::start_detached(&daemon_config, config, preflight.startup_telemetry).await?;
        println!("Bitloops daemon started in detached mode at {}", state.url);
        return Ok(());
    }

    daemon::start_foreground(
        &daemon_config,
        config,
        false,
        "Bitloops daemon",
        preflight.startup_telemetry,
    )
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StartPreflightDecision {
    create_default_config: bool,
    startup_telemetry: Option<bool>,
}

fn resolve_start_preflight(
    args: &DaemonStartArgs,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<StartPreflightDecision> {
    let default_config_missing = args.config.is_none() && !default_daemon_config_exists()?;
    let mut create_default_config = args.create_default_config;

    if default_config_missing && !create_default_config {
        if !telemetry_consent::can_prompt_interactively() {
            bail!(missing_default_daemon_bootstrap_message());
        }
        create_default_config = telemetry_consent::prompt_default_config_setup(out, input)?;
        if !create_default_config {
            bail!(missing_default_daemon_bootstrap_message());
        }
    }

    let startup_telemetry = collect_startup_telemetry_choice(
        default_config_missing && create_default_config,
        args,
        out,
        input,
    )?;

    Ok(StartPreflightDecision {
        create_default_config,
        startup_telemetry,
    })
}

fn collect_startup_telemetry_choice(
    created_default_config: bool,
    args: &DaemonStartArgs,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<Option<bool>> {
    let telemetry_choice =
        telemetry_consent::telemetry_flag_choice(args.telemetry, args.no_telemetry);
    if !created_default_config {
        return Ok(telemetry_choice);
    }

    if let Some(choice) = telemetry_choice {
        return Ok(Some(choice));
    }

    if !telemetry_consent::can_prompt_interactively() {
        bail!(telemetry_consent::NON_INTERACTIVE_TELEMETRY_ERROR);
    }

    Ok(Some(telemetry_consent::prompt_telemetry_consent(
        out, input,
    )?))
}

pub async fn run_stop(args: DaemonStopArgs) -> Result<()> {
    if let Some(config_path) = args.config.as_deref() {
        let _ = daemon::resolve_daemon_config(Some(config_path))?;
    }
    daemon::stop().await?;
    println!("Bitloops daemon stopped.");
    Ok(())
}

pub async fn run_status(args: DaemonStatusArgs) -> Result<()> {
    if let Some(config_path) = args.config.as_deref() {
        let _ = daemon::resolve_daemon_config(Some(config_path))?;
    }
    let report = daemon::status().await?;
    for line in status_lines(&report) {
        println!("{line}");
    }
    for line in legacy_repo_data_warnings() {
        println!("{line}");
    }
    Ok(())
}

pub async fn run_restart(args: DaemonRestartArgs) -> Result<()> {
    let requested_config: Option<daemon::ResolvedDaemonConfig> = args
        .config
        .as_deref()
        .map(|config_path| daemon::resolve_daemon_config(Some(config_path)))
        .transpose()?;
    let report = daemon::status().await?;

    if report.service.is_none()
        && let Some(runtime) = report.runtime
        && matches!(runtime.mode, DaemonMode::Foreground)
    {
        let config = DashboardServerConfig {
            host: Some(runtime.host),
            port: runtime.port,
            no_open: true,
            force_http: runtime.url.starts_with("http://"),
            recheck_local_dashboard_net: false,
            bundle_dir: Some(runtime.bundle_dir),
        };
        daemon::stop().await?;
        let daemon_config = match requested_config {
            Some(ref daemon_config) => daemon_config.clone(),
            None => daemon::resolve_daemon_config(Some(runtime.config_path.as_path()))?,
        };
        return daemon::start_foreground(&daemon_config, config, false, "Bitloops daemon", None)
            .await;
    }

    let state = daemon::restart(requested_config.as_ref()).await?;
    println!("Bitloops daemon restarted at {}", state.url);
    Ok(())
}

pub async fn run_enrichments(args: EnrichmentArgs) -> Result<()> {
    let Some(command) = args.command else {
        bail!("missing subcommand. Use one of: `bitloops daemon enrichments status`, `bitloops daemon enrichments pause`, `bitloops daemon enrichments resume`, `bitloops daemon enrichments retry-failed`");
    };

    match command {
        EnrichmentCommand::Status(_) => {
            let status = daemon::enrichment_status()?;
            for line in enrichment_status_lines(&status) {
                println!("{line}");
            }
            Ok(())
        }
        EnrichmentCommand::Pause(args) => {
            let result = daemon::pause_enrichments(args.reason)?;
            println!("{}", result.message);
            Ok(())
        }
        EnrichmentCommand::Resume(_) => {
            let result = daemon::resume_enrichments()?;
            println!("{}", result.message);
            Ok(())
        }
        EnrichmentCommand::RetryFailed(_) => {
            let result = daemon::retry_failed_enrichments()?;
            println!("{}", result.message);
            Ok(())
        }
    }
}

pub async fn launch_dashboard() -> Result<()> {
    print_legacy_repo_data_warnings();
    if let Some(url) = daemon::daemon_url()? {
        crate::api::open_in_default_browser(&url)?;
        println!("Opened Bitloops dashboard at {url}");
        return Ok(());
    }

    if !default_daemon_config_exists()? {
        bail!(missing_default_daemon_bootstrap_message());
    }

    let daemon_config = daemon::resolve_daemon_config(None)?;
    let report = daemon::status().await?;
    let config = DashboardServerConfig {
        host: None,
        port: crate::api::DEFAULT_DASHBOARD_PORT,
        no_open: false,
        force_http: false,
        recheck_local_dashboard_net: false,
        bundle_dir: None,
    };

    if report.service.is_some() {
        let state = daemon::start_service(&daemon_config, config, None).await?;
        crate::api::open_in_default_browser(&state.url)?;
        println!("Opened Bitloops dashboard at {}", state.url);
        return Ok(());
    }

    let Some(choice) = daemon::choose_dashboard_launch_mode()? else {
        bail!(
            "Bitloops daemon is not running. Start it with `bitloops daemon start`, `bitloops daemon start -d`, or `bitloops daemon start --until-stopped`."
        );
    };

    match choice {
        DaemonMode::Foreground => {
            daemon::start_foreground(&daemon_config, config, true, "Dashboard", None).await
        }
        DaemonMode::Detached => {
            let state = daemon::start_detached(&daemon_config, config, None).await?;
            crate::api::open_in_default_browser(&state.url)?;
            println!("Opened Bitloops dashboard at {}", state.url);
            Ok(())
        }
        DaemonMode::Service => {
            let state = daemon::start_service(&daemon_config, config, None).await?;
            crate::api::open_in_default_browser(&state.url)?;
            println!("Opened Bitloops dashboard at {}", state.url);
            Ok(())
        }
    }
}

fn build_server_config(args: &DaemonStartArgs) -> DashboardServerConfig {
    DashboardServerConfig {
        host: args.host.clone(),
        port: args.port,
        no_open: true,
        force_http: args.http,
        recheck_local_dashboard_net: args.recheck_local_dashboard_net,
        bundle_dir: args.bundle_dir.clone(),
    }
}

fn missing_default_daemon_bootstrap_message() -> &'static str {
    "Bitloops daemon has not been bootstrapped yet. Run `bitloops start --create-default-config` or `bitloops init --install-default-daemon`."
}

fn status_lines(report: &daemon::DaemonStatusReport) -> Vec<String> {
    let mut lines = Vec::new();

    if let Some(runtime) = report.runtime.as_ref() {
        lines.push("Bitloops daemon: running".to_string());
        lines.push(format!("Mode: {}", runtime.mode));
        lines.push(format!("URL: {}", runtime.url));
        lines.push(format!("Config: {}", runtime.config_path.display()));
        lines.push(format!("PID: {}", runtime.pid));
        append_supervisor_lines(&mut lines, report);
        if let Some(health) = report.health.as_ref() {
            append_health_lines(&mut lines, health);
        }
        if let Some(enrichment) = report.enrichment.as_ref() {
            append_enrichment_lines(&mut lines, enrichment);
        }
        return lines;
    }

    if let Some(service) = report.service.as_ref() {
        lines.push("Bitloops daemon: stopped".to_string());
        lines.push("Mode: always-on service".to_string());
        lines.push(format!("Config: {}", service.config_path.display()));
        lines.push(format!(
            "Supervisor service: {} ({}, installed)",
            service.service_name, service.manager
        ));
        lines.push(format!(
            "Supervisor state: {}",
            if report.service_running {
                "running"
            } else {
                "stopped"
            }
        ));
        if let Some(url) = service.last_url.as_ref() {
            lines.push(format!("Last URL: {url}"));
        }
        if let Some(enrichment) = report.enrichment.as_ref() {
            append_enrichment_lines(&mut lines, enrichment);
        }
        return lines;
    }

    lines.push("Bitloops daemon: stopped".to_string());
    lines.push("Mode: not running".to_string());
    if let Some(enrichment) = report.enrichment.as_ref() {
        append_enrichment_lines(&mut lines, enrichment);
    }
    lines
}

fn append_supervisor_lines(lines: &mut Vec<String>, report: &daemon::DaemonStatusReport) {
    if let Some(service) = report.service.as_ref() {
        lines.push(format!(
            "Supervisor service: {} ({}, installed)",
            service.service_name, service.manager
        ));
        lines.push(format!(
            "Supervisor state: {}",
            if report.service_running {
                "running"
            } else {
                "stopped"
            }
        ));
    }
}

fn append_health_lines(lines: &mut Vec<String>, health: &daemon::DaemonHealthSummary) {
    if let (Some(backend), Some(connected)) =
        (&health.relational_backend, health.relational_connected)
    {
        lines.push(format!(
            "Relational: {} ({})",
            backend,
            if connected {
                "connected"
            } else {
                "disconnected"
            }
        ));
    }
    if let (Some(backend), Some(connected)) = (&health.events_backend, health.events_connected) {
        lines.push(format!(
            "Events: {} ({})",
            backend,
            if connected {
                "connected"
            } else {
                "disconnected"
            }
        ));
    }
    if let (Some(backend), Some(connected)) = (&health.blob_backend, health.blob_connected) {
        lines.push(format!(
            "Blob: {} ({})",
            backend,
            if connected {
                "available"
            } else {
                "unavailable"
            }
        ));
    }
}

fn enrichment_status_lines(status: &daemon::EnrichmentQueueStatus) -> Vec<String> {
    let mut lines = vec!["Enrichment queue: available".to_string()];
    append_enrichment_lines(&mut lines, status);
    lines
}

fn append_enrichment_lines(lines: &mut Vec<String>, status: &daemon::EnrichmentQueueStatus) {
    lines.push(format!("Enrichment mode: {}", status.state.mode));
    lines.push(format!("Enrichment pending jobs: {}", status.state.pending_jobs));
    lines.push(format!(
        "Enrichment pending semantic jobs: {}",
        status.state.pending_semantic_jobs
    ));
    lines.push(format!(
        "Enrichment pending embedding jobs: {}",
        status.state.pending_embedding_jobs
    ));
    lines.push(format!(
        "Enrichment pending clone-edge rebuild jobs: {}",
        status.state.pending_clone_edges_rebuild_jobs
    ));
    lines.push(format!("Enrichment running jobs: {}", status.state.running_jobs));
    lines.push(format!(
        "Enrichment running semantic jobs: {}",
        status.state.running_semantic_jobs
    ));
    lines.push(format!(
        "Enrichment running embedding jobs: {}",
        status.state.running_embedding_jobs
    ));
    lines.push(format!(
        "Enrichment running clone-edge rebuild jobs: {}",
        status.state.running_clone_edges_rebuild_jobs
    ));
    lines.push(format!("Enrichment failed jobs: {}", status.state.failed_jobs));
    lines.push(format!(
        "Enrichment failed semantic jobs: {}",
        status.state.failed_semantic_jobs
    ));
    lines.push(format!(
        "Enrichment failed embedding jobs: {}",
        status.state.failed_embedding_jobs
    ));
    lines.push(format!(
        "Enrichment failed clone-edge rebuild jobs: {}",
        status.state.failed_clone_edges_rebuild_jobs
    ));
    lines.push(format!(
        "Enrichment retried failed jobs: {}",
        status.state.retried_failed_jobs
    ));
    if let Some(action) = status.state.last_action.as_ref() {
        lines.push(format!("Enrichment last action: {action}"));
    }
    if let Some(reason) = status.state.paused_reason.as_ref() {
        lines.push(format!("Enrichment pause reason: {reason}"));
    }
    lines.push(format!(
        "Enrichment persisted: {}",
        if status.persisted { "yes" } else { "no" }
    ));
}

fn print_legacy_repo_data_warnings() {
    for line in legacy_repo_data_warnings() {
        eprintln!("{line}");
    }
}

fn legacy_repo_data_warnings() -> Vec<String> {
    let Some(repo_root) = crate::utils::paths::repo_root().ok() else {
        return Vec::new();
    };

    let legacy_paths = [
        repo_root.join(".bitloops").join("stores"),
        repo_root.join(".bitloops").join("embeddings"),
        repo_root.join(".bitloops").join("tmp"),
        repo_root.join(".bitloops").join("metadata"),
    ];
    let found: Vec<_> = legacy_paths
        .into_iter()
        .filter(|path| path.exists())
        .collect();
    if found.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::with_capacity(found.len() + 1);
    lines.push(
        "Warning: legacy repo-local Bitloops data was found and is ignored unless you configure those paths explicitly in the daemon config.".to_string(),
    );
    lines.extend(
        found
            .into_iter()
            .map(|path| format!("Legacy path: {}", path.display())),
    );
    lines
}

#[cfg(test)]
mod tests {
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
}
