//! Root command helpers and metadata.

use anyhow::{Context, Result};
use clap::{Args, Command, CommandFactory, ValueEnum};
use clap_complete::generate;
use serde_json::Value;
use std::collections::HashMap;
use std::env;
#[cfg(test)]
use std::io::BufRead;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use crate::cli::{clean, doctor, enable, reset, resume, uninstall, versioncheck};
use crate::config::settings::{self, BitloopsSettings};
use crate::utils::branding::{BITLOOPS_PURPLE_HEX, bitloops_wordmark, color_hex_if_enabled};

pub const ROOT_NAME: &str = "bitloops";
pub const ROOT_SHORT_ABOUT: &str = "Bitloops CLI";
pub const ROOT_LONG_ABOUT: &str = r#"The command-line interface for Bitloops

Getting Started:
  To get started with Bitloops CLI, run 'bitloops start' to launch the
  daemon, then run 'bitloops init' inside a repository or subproject.
  For more information, visit:
  https://docs.bitloops.io/introduction

Environment Variables:
  ACCESSIBLE    Set to any value (e.g., ACCESSIBLE=1) to enable accessibility
                mode. This uses simpler text prompts instead of interactive
                TUI elements, which works better with screen readers.
"#;

#[derive(Args, Debug, Clone, Default)]
pub struct CleanArgs {
    /// Actually delete items (default: dry run).
    #[arg(long, short = 'f', default_value_t = false)]
    pub force: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DisableArgs {
    /// Deprecated: the nearest discovered project policy is edited automatically.
    #[arg(long, default_value_t = false)]
    pub project: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DoctorArgs {
    /// Fix all stuck sessions without prompting.
    #[arg(long, short = 'f', default_value_t = false)]
    pub force: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct HelpArgs {
    /// Show full command tree.
    #[arg(short = 't', long = "tree", hide = true, default_value_t = false)]
    pub tree: bool,

    /// Optional target command path.
    #[arg(value_name = "command")]
    pub command: Vec<String>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct ResetArgs {
    /// Skip confirmation prompt and active-session guard.
    #[arg(long, short = 'f', default_value_t = false)]
    pub force: bool,

    /// Reset a specific session by ID.
    #[arg(long)]
    pub session: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct ResumeArgs {
    /// Branch to switch to before resume logic.
    pub branch: String,

    /// Resume from older checkpoint without confirmation.
    #[arg(long, short = 'f', default_value_t = false)]
    pub force: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
}

#[derive(Args, Debug, Clone)]
pub struct CompletionArgs {
    #[arg(value_enum)]
    pub shell: CompletionShell,
}

#[derive(Args, Debug, Clone, Default)]
pub struct VersionArgs {
    /// Check for updates now.
    #[arg(long, default_value_t = false)]
    pub check: bool,
}

pub(crate) fn build_version() -> &'static str {
    option_env!("BITLOOPS_BUILD_VERSION").unwrap_or("dev")
}

pub(crate) fn build_commit() -> &'static str {
    option_env!("BITLOOPS_BUILD_COMMIT").unwrap_or("unknown")
}

pub(crate) fn build_target() -> &'static str {
    option_env!("BITLOOPS_BUILD_TARGET")
        .or(option_env!("TARGET"))
        .unwrap_or("unknown")
}

pub(crate) fn build_date() -> &'static str {
    option_env!("BITLOOPS_BUILD_DATE").unwrap_or("unknown")
}

/// Returns true when the executed command or any ancestor is hidden.
///
/// `hidden_chain` order must be leaf -> ... -> root.
#[cfg(test)]
pub(crate) fn has_hidden_in_chain(hidden_chain: &[bool]) -> bool {
    hidden_chain.iter().copied().any(|is_hidden| is_hidden)
}

/// Loads settings once for root post-run side effects.
///
/// Settings load failures are tolerated and
/// downstream telemetry/version logic simply proceeds with partial data.
pub(crate) fn load_settings_once(repo_root: &Path) -> Option<BitloopsSettings> {
    settings::load_settings(repo_root).ok()
}

pub fn run_clean_command(args: &CleanArgs) -> Result<()> {
    let mut out = io::stdout();
    clean::run_clean(&mut out, args.force)
}

pub fn run_disable_command(args: &DisableArgs) -> Result<()> {
    let cwd = env::current_dir().context("getting current directory")?;
    let mut out = io::stdout();
    enable::run_disable(&cwd, &mut out, args.project)
}

pub async fn run_uninstall_command(args: uninstall::UninstallArgs) -> Result<()> {
    uninstall::run(args).await
}

pub fn run_doctor_command(args: &DoctorArgs) -> Result<()> {
    doctor::run_doctor(args.force)
}

pub fn run_help_command(args: &HelpArgs) -> Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    write_help(&mut out, &args.command, args.tree)
}

pub fn run_root_default_help() -> Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    write_help(&mut out, &[], false)
}

pub fn run_reset_command(args: &ResetArgs) -> Result<()> {
    let cwd = env::current_dir().context("getting current directory")?;
    let repo_root = enable::find_repo_root(&cwd)?;
    let strategy_name = load_settings_once(&repo_root)
        .map(|settings| settings.strategy)
        .unwrap_or_else(|| settings::DEFAULT_STRATEGY.to_string());

    let config = reset::ResetConfig {
        repo_root,
        force: args.force,
        session_id: args.session.clone(),
        strategy_name,
    };
    reset::run_reset_cmd(&config)
}

pub fn run_resume_command(args: &ResumeArgs) -> Result<()> {
    let cwd = env::current_dir().context("getting current directory")?;
    let repo_root = enable::find_repo_root(&cwd)?;
    let outcome = resume::run_resume(&repo_root, &args.branch, args.force)?;
    if !outcome.message.is_empty() {
        println!("{}", outcome.message);
    }
    Ok(())
}

fn new_action(
    event: &str,
    properties: HashMap<String, Value>,
) -> crate::telemetry::analytics::ActionDescriptor {
    crate::telemetry::analytics::ActionDescriptor {
        event: event.to_string(),
        surface: "cli",
        properties,
    }
}

fn insert_flags(props: &mut HashMap<String, Value>, flags: Vec<&'static str>) {
    if flags.is_empty() {
        return;
    }

    props.insert(
        "flags".to_string(),
        Value::Array(
            flags
                .into_iter()
                .map(|flag| Value::String(flag.to_string()))
                .collect(),
        ),
    );
}

fn insert_bool_property(props: &mut HashMap<String, Value>, key: &str, value: bool) {
    props.insert(key.to_string(), Value::Bool(value));
}

fn insert_count_property(props: &mut HashMap<String, Value>, key: &str, value: usize) {
    props.insert(
        key.to_string(),
        Value::Number(serde_json::Number::from(
            u64::try_from(value).unwrap_or(u64::MAX),
        )),
    );
}

fn insert_optional_count_property(
    props: &mut HashMap<String, Value>,
    key: &str,
    value: Option<usize>,
) {
    if let Some(value) = value {
        insert_count_property(props, key, value);
    }
}

fn insert_string_property(props: &mut HashMap<String, Value>, key: &str, value: &str) {
    props.insert(key.to_string(), Value::String(value.to_string()));
}

fn stage_sequence_from_devql_query(query: &str) -> Vec<String> {
    query
        .split("->")
        .map(str::trim)
        .filter(|stage| !stage.is_empty())
        .filter_map(|stage| {
            let name = stage
                .split_once('(')
                .map(|(prefix, _)| prefix)
                .unwrap_or(stage)
                .trim();
            (!name.is_empty()).then(|| name.to_string())
        })
        .collect()
}

pub(crate) fn telemetry_action_for_version(
    check: bool,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if check {
        flags.push("check");
    }
    insert_flags(&mut props, flags);
    new_action("bitloops version", props)
}

pub(crate) fn telemetry_action_for_connection_status()
-> crate::telemetry::analytics::ActionDescriptor {
    new_action("bitloops connection status", HashMap::new())
}

pub(crate) fn telemetry_action_for_command(
    command: &crate::cli::Commands,
) -> Option<crate::telemetry::analytics::ActionDescriptor> {
    match command {
        crate::cli::Commands::Daemon(args) => match args.command.as_ref()? {
            crate::cli::daemon::DaemonCommand::Start(args) => Some(daemon_start_action(args)),
            crate::cli::daemon::DaemonCommand::Stop(args) => Some(daemon_stop_action(args)),
            crate::cli::daemon::DaemonCommand::Status(args) => Some(daemon_status_action(args)),
            crate::cli::daemon::DaemonCommand::Restart(args) => Some(daemon_restart_action(args)),
            crate::cli::daemon::DaemonCommand::Enrichments(args) => daemon_enrichments_action(args),
            crate::cli::daemon::DaemonCommand::Logs(args) => Some(daemon_logs_action(args)),
        },
        crate::cli::Commands::Start(args) => Some(daemon_start_action(args)),
        crate::cli::Commands::Stop(args) => Some(daemon_stop_action(args)),
        crate::cli::Commands::Status(args) => Some(daemon_status_action(args)),
        crate::cli::Commands::Restart(args) => Some(daemon_restart_action(args)),
        crate::cli::Commands::Checkpoints(args) => checkpoints_action(args),
        crate::cli::Commands::Rewind(args) => Some(rewind_action(args)),
        crate::cli::Commands::Resume(args) => Some(resume_action(args)),
        crate::cli::Commands::Clean(args) => Some(clean_action(args)),
        crate::cli::Commands::Reset(args) => Some(reset_action(args)),
        crate::cli::Commands::Init(args) => Some(init_action(args)),
        crate::cli::Commands::Enable(args) => Some(enable_action(args)),
        crate::cli::Commands::Disable(args) => Some(disable_action(args)),
        crate::cli::Commands::Uninstall(args) => Some(uninstall_action(args)),
        crate::cli::Commands::Dashboard(_) => {
            Some(new_action("bitloops dashboard", HashMap::new()))
        }
        crate::cli::Commands::Hooks(_) => None,
        crate::cli::Commands::Version(args) => Some(telemetry_action_for_version(args.check)),
        crate::cli::Commands::Explain(args) => Some(explain_action(args)),
        crate::cli::Commands::Debug(_) => None,
        crate::cli::Commands::Devql(args) => devql_action(args),
        crate::cli::Commands::Testlens(args) => testlens_action(args),
        crate::cli::Commands::Embeddings(args) => embeddings_action(args),
        crate::cli::Commands::EmbeddingsRuntime(_) => None,
        crate::cli::Commands::DevqlWatcher(_) => None,
        crate::cli::Commands::DaemonProcess(_) => None,
        crate::cli::Commands::DaemonSupervisor(_) => None,
        crate::cli::Commands::Doctor(args) => Some(doctor_action(args)),
        crate::cli::Commands::SendAnalytics(_) => None,
        crate::cli::Commands::Completion(_) => None,
        crate::cli::Commands::CurlBashPostInstall => None,
        crate::cli::Commands::Help(args) => Some(help_action(args)),
    }
}

pub(crate) fn should_attempt_watcher_autostart(command: &crate::cli::Commands) -> bool {
    matches!(
        command,
        crate::cli::Commands::Devql(_) | crate::cli::Commands::Testlens(_)
    )
}

pub(crate) fn run_persistent_post_run(
    action: Option<&crate::telemetry::analytics::ActionDescriptor>,
    duration: Duration,
    success: bool,
) {
    let Some(action) = action else {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        versioncheck::check_and_notify(&mut out, build_version());
        return;
    };

    let repo_root = env::current_dir()
        .ok()
        .and_then(|cwd| enable::find_repo_root(&cwd).ok());

    let dispatch_context = repo_root
        .as_ref()
        .and_then(|repo_root| {
            crate::telemetry::analytics::load_dispatch_context_for_repo(repo_root)
        })
        .or_else(crate::telemetry::analytics::load_global_dispatch_context);

    if let Some(ctx) = dispatch_context {
        crate::telemetry::analytics::track_action_detached(
            Some(action),
            &ctx,
            build_version(),
            repo_root.as_deref(),
            success,
            duration.as_millis(),
        );
    }

    let stdout = io::stdout();
    let mut out = stdout.lock();
    versioncheck::check_and_notify(&mut out, build_version());
}

pub fn run_send_analytics_command(
    args: &crate::telemetry::analytics::SendAnalyticsArgs,
) -> Result<()> {
    crate::telemetry::analytics::send_event(&args.payload);
    Ok(())
}

fn daemon_start_action(
    args: &crate::cli::daemon::DaemonStartArgs,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.create_default_config {
        flags.push("create_default_config");
    }
    if args.detached {
        flags.push("detached");
    }
    if args.until_stopped {
        flags.push("until_stopped");
    }
    if args.http {
        flags.push("http");
    }
    if args.recheck_local_dashboard_net {
        flags.push("recheck_local_dashboard_net");
    }
    if args.telemetry.is_some() {
        flags.push("telemetry");
    }
    if args.no_telemetry {
        flags.push("no_telemetry");
    }
    insert_flags(&mut props, flags);
    insert_bool_property(&mut props, "has_config", args.config.is_some());
    insert_bool_property(&mut props, "has_host", args.host.is_some());
    insert_bool_property(&mut props, "has_bundle_dir", args.bundle_dir.is_some());
    new_action("bitloops daemon start", props)
}

fn daemon_stop_action(
    args: &crate::cli::daemon::DaemonStopArgs,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    insert_bool_property(&mut props, "has_config", args.config.is_some());
    new_action("bitloops daemon stop", props)
}

fn daemon_status_action(
    args: &crate::cli::daemon::DaemonStatusArgs,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    insert_bool_property(&mut props, "has_config", args.config.is_some());
    new_action("bitloops daemon status", props)
}

fn daemon_restart_action(
    args: &crate::cli::daemon::DaemonRestartArgs,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    insert_bool_property(&mut props, "has_config", args.config.is_some());
    new_action("bitloops daemon restart", props)
}

fn daemon_logs_action(
    args: &crate::cli::daemon::DaemonLogsArgs,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.follow {
        flags.push("follow");
    }
    if args.path {
        flags.push("path");
    }
    insert_flags(&mut props, flags);
    insert_optional_count_property(&mut props, "tail_lines", args.tail);
    new_action("bitloops daemon logs", props)
}

fn daemon_enrichments_action(
    args: &crate::cli::daemon::EnrichmentArgs,
) -> Option<crate::telemetry::analytics::ActionDescriptor> {
    match args.command.as_ref()? {
        crate::cli::daemon::EnrichmentCommand::Status(_) => Some(new_action(
            "bitloops daemon enrichments status",
            HashMap::new(),
        )),
        crate::cli::daemon::EnrichmentCommand::Pause(args) => {
            let mut props = HashMap::new();
            insert_bool_property(&mut props, "has_reason", args.reason.is_some());
            Some(new_action("bitloops daemon enrichments pause", props))
        }
        crate::cli::daemon::EnrichmentCommand::Resume(_) => Some(new_action(
            "bitloops daemon enrichments resume",
            HashMap::new(),
        )),
        crate::cli::daemon::EnrichmentCommand::RetryFailed(_) => Some(new_action(
            "bitloops daemon enrichments retry-failed",
            HashMap::new(),
        )),
    }
}

fn checkpoints_action(
    args: &crate::cli::checkpoints::CheckpointsArgs,
) -> Option<crate::telemetry::analytics::ActionDescriptor> {
    match args.command.as_ref()? {
        crate::cli::checkpoints::CheckpointsCommand::Status(args) => {
            let mut props = HashMap::new();
            let mut flags = Vec::new();
            if args.detailed {
                flags.push("detailed");
            }
            insert_flags(&mut props, flags);
            Some(new_action("bitloops checkpoints status", props))
        }
    }
}

fn rewind_action(
    args: &crate::cli::rewind::RewindArgs,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.list {
        flags.push("list");
    }
    if args.logs_only {
        flags.push("logs_only");
    }
    if args.reset {
        flags.push("reset");
    }
    insert_flags(&mut props, flags);
    insert_bool_property(&mut props, "has_target", args.to.is_some());
    new_action("bitloops rewind", props)
}

fn resume_action(args: &ResumeArgs) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.force {
        flags.push("force");
    }
    insert_flags(&mut props, flags);
    new_action("bitloops resume", props)
}

fn clean_action(args: &CleanArgs) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.force {
        flags.push("force");
    }
    insert_flags(&mut props, flags);
    new_action("bitloops clean", props)
}

fn reset_action(args: &ResetArgs) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.force {
        flags.push("force");
    }
    insert_flags(&mut props, flags);
    insert_bool_property(&mut props, "has_session", args.session.is_some());
    new_action("bitloops reset", props)
}

fn init_action(args: &crate::cli::init::InitArgs) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.install_default_daemon {
        flags.push("install_default_daemon");
    }
    if args.force {
        flags.push("force");
    }
    if args.telemetry.is_some() {
        flags.push("telemetry");
    }
    if args.no_telemetry {
        flags.push("no_telemetry");
    }
    if args.skip_baseline {
        flags.push("skip_baseline");
    }
    insert_flags(&mut props, flags);
    insert_bool_property(&mut props, "has_agent", args.agent.is_some());
    insert_bool_property(&mut props, "has_sync_choice", args.sync.is_some());
    new_action("bitloops init", props)
}

fn enable_action(
    args: &crate::cli::enable::EnableArgs,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.local {
        flags.push("local");
    }
    if args.project {
        flags.push("project");
    }
    if args.force {
        flags.push("force");
    }
    if args.telemetry.is_some() {
        flags.push("telemetry");
    }
    if args.no_telemetry {
        flags.push("no_telemetry");
    }
    insert_flags(&mut props, flags);
    insert_bool_property(&mut props, "has_agent", args.agent.is_some());
    new_action("bitloops enable", props)
}

fn disable_action(args: &DisableArgs) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.project {
        flags.push("project");
    }
    insert_flags(&mut props, flags);
    new_action("bitloops disable", props)
}

fn uninstall_action(
    args: &crate::cli::uninstall::UninstallArgs,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.full {
        flags.push("full");
    }
    if args.binaries {
        flags.push("binaries");
    }
    if args.service {
        flags.push("service");
    }
    if args.data {
        flags.push("data");
    }
    if args.caching {
        flags.push("caching");
    }
    if args.config {
        flags.push("config");
    }
    if args.agent_hooks {
        flags.push("agent_hooks");
    }
    if args.git_hooks {
        flags.push("git_hooks");
    }
    if args.shell {
        flags.push("shell");
    }
    if args.only_current_project {
        flags.push("only_current_project");
    }
    if args.force {
        flags.push("force");
    }
    insert_flags(&mut props, flags);
    new_action("bitloops uninstall", props)
}

fn explain_action(
    args: &crate::cli::explain::ExplainArgs,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.no_pager {
        flags.push("no_pager");
    }
    if args.short {
        flags.push("short");
    }
    if args.full {
        flags.push("full");
    }
    if args.raw_transcript {
        flags.push("raw_transcript");
    }
    if args.generate {
        flags.push("generate");
    }
    if args.force {
        flags.push("force");
    }
    if args.search_all {
        flags.push("search_all");
    }
    insert_flags(&mut props, flags);
    insert_bool_property(&mut props, "has_session", args.session.is_some());
    insert_bool_property(&mut props, "has_commit", args.commit.is_some());
    insert_bool_property(&mut props, "has_checkpoint", args.checkpoint.is_some());
    new_action("bitloops explain", props)
}

fn doctor_action(args: &DoctorArgs) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.force {
        flags.push("force");
    }
    insert_flags(&mut props, flags);
    new_action("bitloops doctor", props)
}

fn help_action(args: &HelpArgs) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.tree {
        flags.push("tree");
    }
    insert_flags(&mut props, flags);
    insert_bool_property(&mut props, "has_command_target", !args.command.is_empty());
    new_action("bitloops help", props)
}

fn devql_action(
    args: &crate::cli::devql::DevqlArgs,
) -> Option<crate::telemetry::analytics::ActionDescriptor> {
    match args.command.as_ref()? {
        crate::cli::devql::DevqlCommand::Init(_) => {
            Some(new_action("bitloops devql init", HashMap::new()))
        }
        crate::cli::devql::DevqlCommand::Ingest(args) => {
            let mut props = HashMap::new();
            insert_count_property(&mut props, "max_checkpoints", args.max_checkpoints);
            Some(new_action("bitloops devql ingest", props))
        }
        crate::cli::devql::DevqlCommand::Sync(args) => {
            let mut props = HashMap::new();
            let mut flags = Vec::new();
            if args.status {
                flags.push("status");
            }
            insert_flags(&mut props, flags);
            let sync_mode = if args.full {
                "full"
            } else if args.paths.is_some() {
                "paths"
            } else if args.repair {
                "repair"
            } else if args.validate {
                "validate"
            } else {
                "incremental"
            };
            insert_string_property(&mut props, "sync_mode", sync_mode);
            insert_bool_property(&mut props, "status_follow", args.status);
            insert_optional_count_property(
                &mut props,
                "paths_count",
                args.paths.as_ref().map(Vec::len),
            );
            Some(new_action("bitloops devql sync", props))
        }
        crate::cli::devql::DevqlCommand::Projection(args) => match &args.command {
            crate::cli::devql::DevqlProjectionCommand::CheckpointFileSnapshots(args) => {
                let mut props = HashMap::new();
                let mut flags = Vec::new();
                if args.dry_run {
                    flags.push("dry_run");
                }
                insert_flags(&mut props, flags);
                insert_count_property(&mut props, "batch_size", args.batch_size);
                insert_optional_count_property(&mut props, "max_checkpoints", args.max_checkpoints);
                insert_bool_property(&mut props, "has_resume_after", args.resume_after.is_some());
                Some(new_action(
                    "bitloops devql projection checkpoint-file-snapshots",
                    props,
                ))
            }
        },
        crate::cli::devql::DevqlCommand::Query(args) => {
            let mut props = HashMap::new();
            let mut flags = Vec::new();
            if args.graphql {
                flags.push("graphql");
            }
            if args.compact {
                flags.push("compact");
            }
            insert_flags(&mut props, flags);
            let query_mode = if crate::host::devql::use_raw_graphql_mode(&args.query, args.graphql)
            {
                "raw_graphql"
            } else {
                "dsl"
            };
            insert_string_property(&mut props, "query_mode", query_mode);
            insert_string_property(
                &mut props,
                "output_mode",
                if args.compact { "compact" } else { "text" },
            );
            if query_mode == "dsl" {
                let stage_sequence = stage_sequence_from_devql_query(&args.query);
                insert_count_property(&mut props, "stage_count", stage_sequence.len());
                props.insert(
                    "stage_sequence".to_string(),
                    Value::Array(stage_sequence.into_iter().map(Value::String).collect()),
                );
            }
            Some(new_action("bitloops devql query", props))
        }
        crate::cli::devql::DevqlCommand::ConnectionStatus(_) => Some(new_action(
            "bitloops devql connection-status",
            HashMap::new(),
        )),
        crate::cli::devql::DevqlCommand::Packs(args) => {
            let mut props = HashMap::new();
            let mut flags = Vec::new();
            if args.with_health {
                flags.push("with_health");
            }
            if args.apply_migrations {
                flags.push("apply_migrations");
            }
            if args.with_extensions {
                flags.push("with_extensions");
            }
            insert_flags(&mut props, flags);
            insert_string_property(
                &mut props,
                "output_mode",
                if args.json { "json" } else { "text" },
            );
            Some(new_action("bitloops devql packs", props))
        }
        crate::cli::devql::DevqlCommand::Knowledge(args) => match &args.command {
            crate::cli::devql::DevqlKnowledgeCommand::Add(args) => {
                let mut props = HashMap::new();
                insert_bool_property(&mut props, "has_url", true);
                insert_bool_property(&mut props, "has_commit", args.commit.is_some());
                Some(new_action("bitloops devql knowledge add", props))
            }
            crate::cli::devql::DevqlKnowledgeCommand::Associate(args) => {
                let mut props = HashMap::new();
                insert_bool_property(&mut props, "has_source_ref", !args.source_ref.is_empty());
                insert_bool_property(&mut props, "has_target_ref", !args.target_ref.is_empty());
                Some(new_action("bitloops devql knowledge associate", props))
            }
            crate::cli::devql::DevqlKnowledgeCommand::Refresh(_) => Some(new_action(
                "bitloops devql knowledge refresh",
                HashMap::new(),
            )),
            crate::cli::devql::DevqlKnowledgeCommand::Versions(_) => Some(new_action(
                "bitloops devql knowledge versions",
                HashMap::new(),
            )),
        },
    }
}

fn testlens_action(
    args: &crate::cli::testlens::TestLensArgs,
) -> Option<crate::telemetry::analytics::ActionDescriptor> {
    match args.command.as_ref()? {
        crate::cli::testlens::TestLensCommand::Init(_) => {
            Some(new_action("bitloops testlens init", HashMap::new()))
        }
        crate::cli::testlens::TestLensCommand::IngestTests(_) => {
            Some(new_action("bitloops testlens ingest-tests", HashMap::new()))
        }
        crate::cli::testlens::TestLensCommand::IngestCoverage(args) => {
            let mut props = HashMap::new();
            insert_bool_property(&mut props, "has_lcov", args.lcov.is_some());
            insert_bool_property(&mut props, "has_input", args.input.is_some());
            insert_bool_property(
                &mut props,
                "has_test_artefact_id",
                args.test_artefact_id.is_some(),
            );
            insert_bool_property(&mut props, "has_format", args.format.is_some());
            Some(new_action("bitloops testlens ingest-coverage", props))
        }
        crate::cli::testlens::TestLensCommand::IngestCoverageBatch(_) => Some(new_action(
            "bitloops testlens ingest-coverage-batch",
            HashMap::new(),
        )),
        crate::cli::testlens::TestLensCommand::IngestResults(_) => Some(new_action(
            "bitloops testlens ingest-results",
            HashMap::new(),
        )),
    }
}

fn embeddings_action(
    args: &crate::cli::embeddings::EmbeddingsArgs,
) -> Option<crate::telemetry::analytics::ActionDescriptor> {
    match args.command.as_ref()? {
        crate::cli::embeddings::EmbeddingsCommand::Pull(_) => {
            Some(new_action("bitloops embeddings pull", HashMap::new()))
        }
        crate::cli::embeddings::EmbeddingsCommand::Doctor(args) => {
            let mut props = HashMap::new();
            insert_bool_property(&mut props, "has_profile", args.profile.is_some());
            Some(new_action("bitloops embeddings doctor", props))
        }
        crate::cli::embeddings::EmbeddingsCommand::ClearCache(_) => Some(new_action(
            "bitloops embeddings clear-cache",
            HashMap::new(),
        )),
    }
}

pub(crate) fn write_completion(w: &mut dyn Write, shell: CompletionShell) -> Result<()> {
    let mut cmd = crate::cli::Cli::command();
    // clap_complete splits subcommand paths using "__". Our hidden
    // "__send_analytics", "__devql-watcher", "__embeddings-runtime", and daemon internal commands
    // conflict with that separator and can panic during completion generation,
    // so we rename them only in this generated tree. Runtime parsing remains
    // unchanged.
    cmd = cmd.mut_subcommand("__embeddings-runtime", |sub| {
        sub.name("embeddings-runtime-internal")
            .bin_name(format!("{ROOT_NAME} embeddings-runtime-internal"))
    });
    cmd = cmd.mut_subcommand("__devql-watcher", |sub| {
        sub.name("devql-watcher-internal")
            .bin_name(format!("{ROOT_NAME} devql-watcher-internal"))
    });
    cmd = cmd.mut_subcommand("__daemon-process", |sub| {
        sub.name("daemon-process-internal")
            .bin_name(format!("{ROOT_NAME} daemon-process-internal"))
    });
    cmd = cmd.mut_subcommand("__daemon-supervisor", |sub| {
        sub.name("daemon-supervisor-internal")
            .bin_name(format!("{ROOT_NAME} daemon-supervisor-internal"))
    });
    cmd = cmd.mut_subcommand("__send_analytics", |sub| {
        sub.name("send-analytics-internal")
            .bin_name(format!("{ROOT_NAME} send-analytics-internal"))
    });
    cmd.build();
    match shell {
        CompletionShell::Bash => generate(clap_complete::Shell::Bash, &mut cmd, ROOT_NAME, w),
        CompletionShell::Zsh => generate(clap_complete::Shell::Zsh, &mut cmd, ROOT_NAME, w),
        CompletionShell::Fish => generate(clap_complete::Shell::Fish, &mut cmd, ROOT_NAME, w),
    }
    Ok(())
}

pub fn run_completion_command(args: &CompletionArgs) -> Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    write_completion(&mut out, args.shell)
}

pub fn run_version_command(check_for_updates: bool) -> Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    write_version(
        &mut out,
        build_version(),
        build_commit(),
        build_target(),
        build_date(),
    )?;

    if check_for_updates {
        versioncheck::check_now(&mut out, version_for_update_check());
    } else {
        writeln!(
            out,
            "Run `bitloops --version --check` to check for updates."
        )?;
    }

    Ok(())
}

#[cfg(test)]
pub(crate) fn run_curl_bash_post_install_command_with_io(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<()> {
    if let Err(err) = enable::run_post_install_shell_completion_with_io(out, input) {
        writeln!(out, "Note: Shell completion setup skipped: {err}")?;
    }
    Ok(())
}

pub fn run_curl_bash_post_install_command() -> Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if let Err(err) = enable::run_post_install_shell_completion(&mut out) {
        writeln!(out, "Note: Shell completion setup skipped: {err}")?;
    }
    Ok(())
}

pub(crate) fn write_help(
    w: &mut dyn Write,
    command_path: &[String],
    show_tree: bool,
) -> Result<()> {
    let root = crate::cli::Cli::command();
    if show_tree {
        return write_command_tree(w, &root);
    }

    let mut target = find_target_command(&root, command_path).clone();
    let mut rendered = Vec::new();
    target.write_long_help(&mut rendered)?;
    w.write_all(&rendered)?;
    writeln!(w)?;
    Ok(())
}

fn find_target_command<'a>(root: &'a Command, command_path: &[String]) -> &'a Command {
    if command_path.is_empty() {
        return root;
    }

    let mut current = root;
    for name in command_path.iter().filter(|value| !value.is_empty()) {
        let Some(next) = current.get_subcommands().find(|sub| sub.get_name() == name) else {
            return root;
        };
        current = next;
    }

    current
}

fn write_command_tree(w: &mut dyn Write, root: &Command) -> Result<()> {
    writeln!(w, "{}", root.get_name())?;
    write_children(w, root, "")
}

fn write_children(w: &mut dyn Write, cmd: &Command, indent: &str) -> Result<()> {
    let visible: Vec<&Command> = cmd
        .get_subcommands()
        .filter(|sub| !sub.is_hide_set() && sub.get_name() != "help")
        .collect();

    for (idx, sub) in visible.iter().enumerate() {
        let is_last = idx == visible.len().saturating_sub(1);
        write_node(w, sub, indent, is_last)?;
    }

    Ok(())
}

fn write_node(w: &mut dyn Write, cmd: &Command, indent: &str, is_last: bool) -> Result<()> {
    let (branch, child_indent) = if is_last {
        ("└── ", format!("{indent}    "))
    } else {
        ("├── ", format!("{indent}│   "))
    };

    write!(w, "{indent}{branch}{}", cmd.get_name())?;
    if let Some(short) = cmd.get_about().map(|about| about.to_string()) {
        let short = short.trim();
        if !short.is_empty() {
            write!(w, " - {short}")?;
        }
    }
    writeln!(w)?;

    write_children(w, cmd, &child_indent)
}

const VERSION_DIVIDER: &str = "───────────────────";

fn pretty_version(version: &str) -> String {
    let trimmed = version.trim();
    if trimmed.is_empty() {
        return "unknown".to_string();
    }
    if trimmed == "dev" {
        return format!("v{}", env!("CARGO_PKG_VERSION"));
    }
    if trimmed.starts_with('v') {
        return trimmed.to_string();
    }
    format!("v{trimmed}")
}

fn short_commit(commit: &str) -> String {
    let trimmed = commit.trim();
    if trimmed.is_empty() {
        return "unknown".to_string();
    }

    trimmed.chars().take(7).collect()
}

fn version_for_update_check() -> &'static str {
    let version = build_version();
    if version == "dev" {
        env!("CARGO_PKG_VERSION")
    } else {
        version
    }
}

pub(crate) fn write_version(
    w: &mut dyn Write,
    version: &str,
    commit: &str,
    target: &str,
    built: &str,
) -> Result<()> {
    writeln!(w)?;
    writeln!(
        w,
        "{}",
        color_hex_if_enabled(&bitloops_wordmark(), BITLOOPS_PURPLE_HEX)
    )?;
    writeln!(w, "Bitloops CLI {}", pretty_version(version))?;
    writeln!(w, "{VERSION_DIVIDER}")?;
    writeln!(w, "commit: {}", short_commit(commit))?;
    writeln!(w, "target: {}", target.trim())?;
    writeln!(w, "built: {}", built.trim())?;
    writeln!(w)?;
    Ok(())
}
