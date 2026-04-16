use std::path::PathBuf;

use clap::{Args, Subcommand};

pub const MISSING_SUBCOMMAND_MESSAGE: &str = "missing subcommand. Use one of: `bitloops daemon start`, `bitloops daemon stop`, `bitloops daemon status`, `bitloops daemon restart`, `bitloops daemon enable`, `bitloops daemon enrichments`, `bitloops daemon logs`";

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
    /// Enable capture in the current Bitloops project.
    Enable(crate::cli::enable::EnableArgs),
    /// Inspect or control the enrichment coordinator.
    Enrichments(EnrichmentArgs),
    /// Show daemon log output.
    Logs(DaemonLogsArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonLogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl DaemonLogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Args, Debug, Clone)]
pub struct DaemonStartArgs {
    /// Path to the Bitloops daemon config file.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Create the default global daemon config and local default stores if missing.
    #[arg(long, default_value_t = false, conflicts_with = "config")]
    pub create_default_config: bool,

    /// Create local store artefacts for the selected config before starting.
    #[arg(long, default_value_t = false)]
    pub bootstrap_local_stores: bool,

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
    pub bundle_dir: Option<PathBuf>,

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
    pub config: Option<PathBuf>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DaemonStatusArgs {
    /// Path to the Bitloops daemon config file.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Emit daemon status as JSON.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DaemonRestartArgs {
    /// Path to the Bitloops daemon config file.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DaemonLogsArgs {
    /// Print the last N lines from the daemon log.
    #[arg(long, value_name = "N", value_parser = parse_log_lines)]
    pub tail: Option<usize>,

    /// Only include log lines at the selected levels.
    #[arg(long = "level", value_name = "LEVEL", value_parser = parse_daemon_log_level)]
    pub levels: Vec<DaemonLogLevel>,

    /// Keep streaming appended daemon log lines.
    #[arg(long, default_value_t = false, conflicts_with = "path")]
    pub follow: bool,

    /// Print the daemon log file path and exit.
    #[arg(long, default_value_t = false, conflicts_with_all = ["follow", "tail"])]
    pub path: bool,
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

fn parse_log_lines(value: &str) -> std::result::Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("invalid value `{value}` for --tail"))?;
    if parsed == 0 {
        return Err("--tail must be greater than 0".to_string());
    }
    Ok(parsed)
}

fn parse_daemon_log_level(value: &str) -> std::result::Result<DaemonLogLevel, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "debug" => Ok(DaemonLogLevel::Debug),
        "info" => Ok(DaemonLogLevel::Info),
        "warn" | "warning" => Ok(DaemonLogLevel::Warn),
        "error" => Ok(DaemonLogLevel::Error),
        _ => Err(format!("invalid value `{value}` for --level")),
    }
}
