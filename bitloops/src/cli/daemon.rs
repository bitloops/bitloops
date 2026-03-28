use anyhow::{Result, bail};
use clap::{Args, Subcommand};

use crate::api::DashboardServerConfig;
use crate::daemon::{self, DaemonMode};
use crate::utils::paths;

pub const MISSING_SUBCOMMAND_MESSAGE: &str = "missing subcommand. Use one of: `bitloops daemon start`, `bitloops daemon stop`, `bitloops daemon status`, `bitloops daemon restart`";

#[derive(Args, Debug, Clone, Default)]
pub struct DaemonArgs {
    #[command(subcommand)]
    pub command: Option<DaemonCommand>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DaemonCommand {
    /// Start the Bitloops daemon for the current repository.
    Start(DaemonStartArgs),
    /// Stop the Bitloops daemon for the current repository.
    Stop(DaemonStopArgs),
    /// Show Bitloops daemon status for the current repository.
    Status(DaemonStatusArgs),
    /// Restart the Bitloops daemon for the current repository.
    Restart(DaemonRestartArgs),
}

#[derive(Args, Debug, Clone)]
pub struct DaemonStartArgs {
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
}

#[derive(Args, Debug, Clone, Default)]
pub struct DaemonStopArgs {}

#[derive(Args, Debug, Clone, Default)]
pub struct DaemonStatusArgs {}

#[derive(Args, Debug, Clone, Default)]
pub struct DaemonRestartArgs {}

pub async fn run(args: DaemonArgs) -> Result<()> {
    let Some(command) = args.command else {
        bail!(MISSING_SUBCOMMAND_MESSAGE);
    };

    match command {
        DaemonCommand::Start(args) => run_start(args).await,
        DaemonCommand::Stop(args) => run_stop(args).await,
        DaemonCommand::Status(args) => run_status(args).await,
        DaemonCommand::Restart(args) => run_restart(args).await,
    }
}

pub async fn run_start(args: DaemonStartArgs) -> Result<()> {
    let repo_root = paths::repo_root()?;
    let config = build_server_config(&args);

    if args.until_stopped {
        let state = daemon::start_service(&repo_root, config).await?;
        println!(
            "Bitloops daemon started as an always-on service at {}",
            state.url
        );
        return Ok(());
    }

    let report = daemon::status(&repo_root).await?;
    if report.service.is_some() {
        let state = daemon::start_service(&repo_root, config).await?;
        println!(
            "Bitloops daemon started under the always-on service at {}",
            state.url
        );
        return Ok(());
    }

    if args.detached {
        let state = daemon::start_detached(&repo_root, config).await?;
        println!("Bitloops daemon started in detached mode at {}", state.url);
        return Ok(());
    }

    daemon::start_foreground(&repo_root, config, false, "Bitloops daemon").await
}

pub async fn run_stop(_args: DaemonStopArgs) -> Result<()> {
    let repo_root = paths::repo_root()?;
    daemon::stop(&repo_root).await?;
    println!("Bitloops daemon stopped.");
    Ok(())
}

pub async fn run_status(_args: DaemonStatusArgs) -> Result<()> {
    let repo_root = paths::repo_root()?;
    let report = daemon::status(&repo_root).await?;
    for line in status_lines(&report) {
        println!("{line}");
    }
    Ok(())
}

pub async fn run_restart(_args: DaemonRestartArgs) -> Result<()> {
    let repo_root = paths::repo_root()?;
    let report = daemon::status(&repo_root).await?;

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
        daemon::stop(&repo_root).await?;
        return daemon::start_foreground(&repo_root, config, false, "Bitloops daemon").await;
    }

    let state = daemon::restart(&repo_root).await?;
    println!("Bitloops daemon restarted at {}", state.url);
    Ok(())
}

pub async fn launch_dashboard() -> Result<()> {
    let repo_root = paths::repo_root()?;
    if let Some(url) = daemon::daemon_url(&repo_root)? {
        crate::api::open_in_default_browser(&url)?;
        println!("Opened Bitloops dashboard at {url}");
        return Ok(());
    }

    let report = daemon::status(&repo_root).await?;
    let config = DashboardServerConfig {
        host: None,
        port: crate::api::DEFAULT_DASHBOARD_PORT,
        no_open: false,
        force_http: false,
        recheck_local_dashboard_net: false,
        bundle_dir: None,
    };

    if report.service.is_some() {
        let state = daemon::start_service(&repo_root, config).await?;
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
            daemon::start_foreground(&repo_root, config, true, "Dashboard").await
        }
        DaemonMode::Detached => {
            let state = daemon::start_detached(&repo_root, config).await?;
            crate::api::open_in_default_browser(&state.url)?;
            println!("Opened Bitloops dashboard at {}", state.url);
            Ok(())
        }
        DaemonMode::Service => {
            let state = daemon::start_service(&repo_root, config).await?;
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

fn status_lines(report: &daemon::DaemonStatusReport) -> Vec<String> {
    let mut lines = Vec::new();

    if let Some(runtime) = report.runtime.as_ref() {
        lines.push("Bitloops daemon: running".to_string());
        lines.push(format!("Mode: {}", runtime.mode));
        lines.push(format!("URL: {}", runtime.url));
        lines.push(format!("PID: {}", runtime.pid));
        append_supervisor_lines(&mut lines, report);
        if let Some(health) = report.health.as_ref() {
            append_health_lines(&mut lines, health);
        }
        return lines;
    }

    if let Some(service) = report.service.as_ref() {
        lines.push("Bitloops daemon: stopped".to_string());
        lines.push("Mode: always-on service".to_string());
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
        return lines;
    }

    lines.push("Bitloops daemon: stopped".to_string());
    lines.push("Mode: not running".to_string());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use crate::daemon::{DaemonServiceMetadata, DaemonStatusReport, ServiceManagerKind};
    use clap::Parser;

    #[test]
    fn daemon_start_cli_parses_lifecycle_and_server_flags() {
        let parsed = Cli::try_parse_from([
            "bitloops",
            "daemon",
            "start",
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
    }

    #[test]
    fn status_lines_show_global_supervisor_install_and_state() {
        let report = DaemonStatusReport {
            runtime: None,
            service: Some(DaemonServiceMetadata {
                version: 1,
                repo_root: std::path::PathBuf::from("/tmp/repo"),
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

        assert_eq!(
            status_lines(&report),
            vec![
                "Bitloops daemon: stopped".to_string(),
                "Mode: always-on service".to_string(),
                "Supervisor service: com.bitloops.daemon (launchd, installed)".to_string(),
                "Supervisor state: stopped".to_string(),
                "Last URL: https://127.0.0.1:5173".to_string(),
            ]
        );
    }
}
