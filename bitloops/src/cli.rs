use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub mod checkpoints;
pub mod clean;
pub mod daemon;
pub mod dashboard;
pub mod debug;
pub mod devql;
pub mod doctor;
pub mod embeddings;
pub mod enable;
pub mod explain;
pub mod init;
pub mod reset;
pub mod resume;
pub mod rewind;
pub mod root;
pub(crate) mod telemetry_consent;
pub mod uninstall;
pub mod versioncheck;

/// Bitloops CLI
#[derive(Parser)]
#[command(
    name = root::ROOT_NAME,
    version,
    about = root::ROOT_SHORT_ABOUT,
    long_about = root::ROOT_LONG_ABOUT,
    disable_version_flag = true,
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Show build information.
    #[arg(long = "version", short = 'V', default_value_t = false)]
    pub version: bool,

    /// Check for updates (only valid with --version).
    #[arg(long = "check", requires = "version", default_value_t = false)]
    pub check: bool,

    /// Check backend connectivity for configured relational/events providers.
    #[arg(long = "connection-status", global = true, default_value_t = false)]
    pub connection_status: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Manage the Bitloops daemon lifecycle.
    Daemon(daemon::DaemonArgs),
    /// Start the global Bitloops daemon.
    Start(daemon::DaemonStartArgs),
    /// Stop the global Bitloops daemon.
    Stop(daemon::DaemonStopArgs),
    /// Show global Bitloops daemon status.
    Status(daemon::DaemonStatusArgs),
    /// Restart the global Bitloops daemon.
    Restart(daemon::DaemonRestartArgs),
    /// Repository/session checkpoint status and related views.
    Checkpoints(checkpoints::CheckpointsArgs),
    /// Browse checkpoints and rewind your session.
    Rewind(rewind::RewindArgs),
    /// Switch to a branch and resume its session.
    Resume(root::ResumeArgs),
    /// Clean up orphaned Bitloops data.
    Clean(root::CleanArgs),
    /// Reset shadow/session state for current HEAD.
    Reset(root::ResetArgs),
    /// Initialise Bitloops for the current project.
    Init(init::InitArgs),
    /// Enable capture in the current Bitloops project.
    Enable(enable::EnableArgs),
    /// Disable capture in the current Bitloops project.
    Disable(root::DisableArgs),
    /// Uninstall Bitloops artefacts from your system or known repositories.
    Uninstall(uninstall::UninstallArgs),
    /// Open the local Bitloops dashboard in your browser.
    Dashboard(dashboard::DashboardArgs),
    /// Internal: agent hook handlers (called by supported agents, not users).
    #[command(hide = true)]
    Hooks(crate::host::hooks::dispatcher::HooksArgs),
    /// Show build information.
    Version(root::VersionArgs),
    /// Explain a session, commit, or checkpoint
    Explain(explain::ExplainArgs),
    /// Hidden debug commands for troubleshooting.
    #[command(hide = true)]
    Debug(debug::DebugArgs),
    /// DevQL ingestion and querying.
    Devql(devql::DevqlArgs),
    /// Manage embedding profiles and caches.
    Embeddings(embeddings::EmbeddingsArgs),
    /// Hidden internal DevQL watcher process entry point.
    #[command(name = "__devql-watcher", hide = true)]
    DevqlWatcher(crate::host::devql::watch::WatcherProcessArgs),
    /// Hidden internal daemon process entry point.
    #[command(name = "__daemon-process", hide = true)]
    DaemonProcess(crate::daemon::InternalDaemonProcessArgs),
    /// Hidden internal daemon supervisor entry point.
    #[command(name = "__daemon-supervisor", hide = true)]
    DaemonSupervisor(crate::daemon::InternalDaemonSupervisorArgs),
    /// Diagnose and fix stuck sessions.
    Doctor(root::DoctorArgs),
    /// Hidden internal analytics dispatch command.
    #[command(name = "__send_analytics", hide = true)]
    SendAnalytics(crate::telemetry::analytics::SendAnalyticsArgs),
    /// Hidden shell completion generator command.
    #[command(name = "completion", hide = true)]
    Completion(root::CompletionArgs),
    /// Hidden post-install command used by curl|bash install flow.
    #[command(name = "curl-bash-post-install", hide = true)]
    CurlBashPostInstall,
    /// Help about any command.
    Help(root::HelpArgs),
}

/// Marker error: the command already printed a user-facing message.
/// main.rs checks for this type to avoid printing a duplicate error line.
#[derive(Debug)]
pub struct SilentError;

impl std::fmt::Display for SilentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "command failed")
    }
}

impl std::error::Error for SilentError {}

fn resolve_watcher_autostart_config_root(repo_root: &Path, policy_start: &Path) -> Option<PathBuf> {
    if !crate::config::settings::is_enabled_for_hooks(policy_start) {
        return None;
    }

    crate::config::resolve_daemon_config_root_for_repo(repo_root).ok()
}

pub async fn run(cli: Cli) -> Result<()> {
    let strategy_registry =
        crate::host::checkpoints::strategy::registry::StrategyRegistry::builtin();

    if cli.version {
        if cli.command.is_some() {
            bail!("`--version` cannot be combined with a subcommand");
        }
        if cli.connection_status {
            bail!("`--version` cannot be combined with `--connection-status`");
        }

        let started = Instant::now();
        let telemetry_action = root::telemetry_action_for_version(cli.check);
        let result = root::run_version_command(cli.check);
        root::run_persistent_post_run(Some(&telemetry_action), started.elapsed(), result.is_ok());
        return result;
    }

    if cli.connection_status {
        if cli.command.is_some() {
            bail!("`--connection-status` cannot be combined with a subcommand");
        }
        let started = Instant::now();
        let telemetry_action = root::telemetry_action_for_connection_status();
        let result = devql::run_connection_status().await;
        root::run_persistent_post_run(Some(&telemetry_action), started.elapsed(), result.is_ok());
        return result;
    }

    let Some(command) = cli.command else {
        return root::run_root_default_help();
    };

    if root::should_attempt_watcher_autostart(&command)
        && let Ok(repo_root) = crate::utils::paths::repo_root()
        && let Ok(policy_start) = std::env::current_dir()
        && let Some(config_root) = resolve_watcher_autostart_config_root(&repo_root, &policy_start)
        && let Err(err) =
            crate::host::devql::watch::ensure_watcher_running(&repo_root, &config_root)
    {
        log::debug!("skipping DevQL watcher auto-start: {err:#}");
    }

    let telemetry_action = root::telemetry_action_for_command(&command);
    let started = Instant::now();

    let result = match command {
        Commands::Daemon(args) => daemon::run(args).await,
        Commands::Start(args) => daemon::run_start(args).await,
        Commands::Stop(args) => daemon::run_stop(args).await,
        Commands::Status(args) => daemon::run_status(args).await,
        Commands::Restart(args) => daemon::run_restart(args).await,
        Commands::Checkpoints(args) => checkpoints::run(args).await,
        Commands::Rewind(args) => rewind::run(&args),
        Commands::Resume(args) => root::run_resume_command(&args),
        Commands::Clean(args) => root::run_clean_command(&args),
        Commands::Reset(args) => root::run_reset_command(&args),
        Commands::Init(args) => init::run(args).await,
        Commands::Enable(args) => enable::run(args).await,
        Commands::Disable(args) => root::run_disable_command(&args),
        Commands::Uninstall(args) => uninstall::run(args).await,
        Commands::Dashboard(args) => dashboard::run(args).await,
        Commands::Hooks(args) => {
            crate::host::hooks::dispatcher::run(args, &strategy_registry).await
        }
        Commands::Version(args) => root::run_version_command(args.check),
        Commands::Explain(args) => explain::run(args).await,
        Commands::Debug(args) => debug::run(&args),
        Commands::Devql(args) => devql::run(args).await,
        Commands::Embeddings(args) => embeddings::run(args),
        Commands::DevqlWatcher(args) => crate::host::devql::watch::run_process_command(args).await,
        Commands::DaemonProcess(args) => crate::daemon::run_internal_process(args).await,
        Commands::DaemonSupervisor(args) => crate::daemon::run_internal_supervisor(args).await,
        Commands::Doctor(args) => root::run_doctor_command(&args),
        Commands::SendAnalytics(args) => root::run_send_analytics_command(&args),
        Commands::Completion(args) => root::run_completion_command(&args),
        Commands::CurlBashPostInstall => root::run_curl_bash_post_install_command(),
        Commands::Help(args) => root::run_help_command(&args),
    };

    root::run_persistent_post_run(telemetry_action.as_ref(), started.elapsed(), result.is_ok());
    result
}

#[cfg(test)]
mod explain_test;

#[cfg(test)]
mod root_test;
