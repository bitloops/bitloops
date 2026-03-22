use anyhow::{Result, bail};
use clap::{Parser, Subcommand};

pub mod clean;
pub mod dashboard;
pub mod debug;
pub mod devql;
pub mod doctor;
pub mod enable;
pub mod explain;
pub mod init;
pub mod reset;
pub mod resume;
pub mod rewind;
pub mod root;
pub mod status;
pub mod testlens;
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
    /// Browse checkpoints and rewind your session.
    Rewind(rewind::RewindArgs),
    /// Switch to a branch and resume its session.
    Resume(root::ResumeArgs),
    /// Clean up orphaned Bitloops data.
    Clean(root::CleanArgs),
    /// Reset shadow/session state for current HEAD.
    Reset(root::ResetArgs),
    /// Initialize agent integrations in the current project.
    Init(init::InitArgs),
    /// Enable Bitloops in the current project.
    Enable(enable::EnableArgs),
    /// Disable Bitloops in the current project.
    Disable(root::DisableArgs),
    /// Show current status.
    Status(status::StatusArgs),
    /// Serve the local Bitloops dashboard.
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
    /// Test harness ingestion and querying over DevQL production artefacts.
    Testlens(testlens::TestLensArgs),
    /// Hidden internal DevQL watcher process entry point.
    #[command(name = "__devql-watcher", hide = true)]
    DevqlWatcher(crate::host::devql::watch::WatcherProcessArgs),
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

pub async fn run(cli: Cli) -> Result<()> {
    let strategy_registry = crate::host::strategy::registry::StrategyRegistry::builtin();

    if cli.version {
        if cli.command.is_some() {
            bail!("`--version` cannot be combined with a subcommand");
        }
        if cli.connection_status {
            bail!("`--version` cannot be combined with `--connection-status`");
        }

        let result = root::run_version_command(cli.check);
        root::run_persistent_post_run(&[], "version");
        return result;
    }

    if cli.connection_status {
        if cli.command.is_some() {
            bail!("`--connection-status` cannot be combined with a subcommand");
        }
        let result = devql::run_connection_status().await;
        root::run_persistent_post_run(&[], "connection-status");
        return result;
    }

    let Some(command) = cli.command else {
        return root::run_root_default_help();
    };

    if root::should_attempt_watcher_autostart(&command)
        && let Ok(repo_root) = crate::utils::paths::repo_root()
        && let Err(err) = crate::host::devql::watch::ensure_watcher_running(&repo_root)
    {
        log::debug!("skipping DevQL watcher auto-start: {err:#}");
    }

    let command_name = root::command_name(&command);
    let hidden_chain = root::hidden_chain_for_command(&command);

    let result = match command {
        Commands::Rewind(args) => rewind::run(&args),
        Commands::Resume(args) => root::run_resume_command(&args),
        Commands::Clean(args) => root::run_clean_command(&args),
        Commands::Reset(args) => root::run_reset_command(&args),
        Commands::Init(args) => init::run(args).await,
        Commands::Enable(args) => enable::run(args).await,
        Commands::Disable(args) => root::run_disable_command(&args),
        Commands::Status(args) => status::run(args).await,
        Commands::Dashboard(args) => dashboard::run(args).await,
        Commands::Hooks(args) => {
            crate::host::hooks::dispatcher::run(args, &strategy_registry).await
        }
        Commands::Version(args) => root::run_version_command(args.check),
        Commands::Explain(args) => explain::run(args).await,
        Commands::Debug(args) => debug::run(&args),
        Commands::Devql(args) => devql::run(args).await,
        Commands::Testlens(args) => testlens::run(args).await,
        Commands::DevqlWatcher(args) => {
            crate::host::devql::watch::run_process_command(args).await
        }
        Commands::Doctor(args) => root::run_doctor_command(&args),
        Commands::SendAnalytics(args) => root::run_send_analytics_command(&args),
        Commands::Completion(args) => root::run_completion_command(&args),
        Commands::CurlBashPostInstall => root::run_curl_bash_post_install_command(),
        Commands::Help(args) => root::run_help_command(&args),
    };

    root::run_persistent_post_run(&hidden_chain, command_name);
    result
}

#[cfg(test)]
mod explain_test;

#[cfg(test)]
mod root_test;
