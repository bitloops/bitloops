//! Root command helpers and metadata.

use anyhow::{Context, Result};
use clap::{Args, Command, CommandFactory, ValueEnum};
use clap_complete::generate;
use serde::{Deserialize, Serialize};
use std::env;
#[cfg(test)]
use std::io::BufRead;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command as ProcessCommand;

use crate::commands::{clean, doctor, enable, reset, resume, versioncheck};
use crate::engine::settings::{self, BitloopsSettings};

pub const ROOT_NAME: &str = "bitloops";
pub const ROOT_SHORT_ABOUT: &str = "Bitloops CLI";
pub const ROOT_LONG_ABOUT: &str = r#"The command-line interface for Bitloops

Getting Started:
  To get started with Bitloops CLI, run 'bitloops enable' to configure
  project settings and git hooks, then run 'bitloops init' to initialize
  agent integrations. For more information, visit:
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
    /// Update project settings file instead of local.
    #[arg(long, default_value_t = false)]
    pub project: bool,

    /// Completely remove Bitloops from repository.
    #[arg(long, default_value_t = false)]
    pub uninstall: bool,

    /// Skip confirmation prompt for uninstall behavior.
    #[arg(long, default_value_t = false)]
    pub force: bool,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelemetryEvent {
    pub command: String,
    pub strategy: String,
    pub agent: String,
    pub is_enabled: bool,
    pub cli_version: String,
}

pub(crate) fn build_version() -> &'static str {
    option_env!("BITLOOPS_BUILD_VERSION").unwrap_or("dev")
}

pub(crate) fn build_commit() -> &'static str {
    option_env!("BITLOOPS_BUILD_COMMIT").unwrap_or("unknown")
}

/// Returns true when the executed command or any ancestor is hidden.
///
/// `hidden_chain` order must be leaf -> ... -> root.
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
    if args.uninstall {
        let cwd = env::current_dir().context("getting current directory")?;
        let repo_root = enable::find_repo_root(&cwd).unwrap_or(cwd);
        let mut out = io::stdout();
        let mut err = io::stderr();
        return enable::run_uninstall(&repo_root, &mut out, &mut err, args.force);
    }

    let cwd = env::current_dir().context("getting current directory")?;
    let repo_root = enable::find_repo_root(&cwd)?;
    let mut out = io::stdout();
    enable::run_disable(&repo_root, &mut out, args.project)
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

pub(crate) fn command_name(command: &crate::commands::Commands) -> &'static str {
    match command {
        crate::commands::Commands::Rewind(_) => "rewind",
        crate::commands::Commands::Resume(_) => "resume",
        crate::commands::Commands::Clean(_) => "clean",
        crate::commands::Commands::Reset(_) => "reset",
        crate::commands::Commands::Init(_) => "init",
        crate::commands::Commands::Enable(_) => "enable",
        crate::commands::Commands::Disable(_) => "disable",
        crate::commands::Commands::Status(_) => "status",
        crate::commands::Commands::Dashboard(_) => "dashboard",
        crate::commands::Commands::Hooks(_) => "hooks",
        crate::commands::Commands::Version => "version",
        crate::commands::Commands::Explain(_) => "explain",
        crate::commands::Commands::Debug(_) => "debug",
        crate::commands::Commands::Devql(_) => "devql",
        crate::commands::Commands::Doctor(_) => "doctor",
        crate::commands::Commands::SendAnalytics(_) => "__send_analytics",
        crate::commands::Commands::Completion(_) => "completion",
        crate::commands::Commands::CurlBashPostInstall => "curl-bash-post-install",
        crate::commands::Commands::Help(_) => "help",
    }
}

pub(crate) fn hidden_chain_for_command(command: &crate::commands::Commands) -> Vec<bool> {
    vec![matches!(
        command,
        crate::commands::Commands::Hooks(_)
            | crate::commands::Commands::Debug(_)
            | crate::commands::Commands::SendAnalytics(_)
            | crate::commands::Commands::Completion(_)
            | crate::commands::Commands::CurlBashPostInstall
    )]
}

pub(crate) fn run_persistent_post_run(hidden_chain: &[bool], command_name: &str) {
    let is_hidden = has_hidden_in_chain(hidden_chain);
    if is_hidden {
        return;
    }

    let argv = env::args().collect::<Vec<_>>();
    let command_info = crate::engine::telemetry::CommandInfo {
        command_path: command_name.to_string(),
        hidden: is_hidden,
        flag_names: crate::engine::telemetry::collect_flag_names_from_argv(&argv),
    };

    let dispatch_context = crate::engine::telemetry::load_dispatch_context().or_else(|| {
        env::current_dir()
            .ok()
            .and_then(|cwd| enable::find_repo_root(&cwd).ok())
            .and_then(|repo_root| {
                build_telemetry_event(hidden_chain, &repo_root, command_name, build_version())
            })
            .map(|event| crate::engine::telemetry::TelemetryDispatchContext {
                strategy: event.strategy,
                agent: event.agent,
                is_bitloops_enabled: event.is_enabled,
                version: event.cli_version,
            })
    });

    if let Some(ctx) = dispatch_context {
        crate::engine::telemetry::track_command_detached(
            Some(&command_info),
            &ctx.strategy,
            &ctx.agent,
            ctx.is_bitloops_enabled,
            &ctx.version,
        );
    }

    let stdout = io::stdout();
    let mut out = stdout.lock();
    versioncheck::check_and_notify(&mut out, build_version());
}

pub(crate) fn build_telemetry_event(
    hidden_chain: &[bool],
    repo_root: &Path,
    command_name: &str,
    version: &str,
) -> Option<TelemetryEvent> {
    if has_hidden_in_chain(hidden_chain) {
        return None;
    }

    let settings = load_settings_once(repo_root)?;
    if settings.telemetry != Some(true) {
        return None;
    }

    let agents = join_agent_names(&agents_with_hooks_installed(repo_root));
    Some(TelemetryEvent {
        command: command_name.to_string(),
        strategy: settings.strategy,
        agent: agents,
        is_enabled: settings.enabled,
        cli_version: version.to_string(),
    })
}

pub(crate) fn agents_with_hooks_installed(repo_root: &Path) -> Vec<String> {
    let mut agents = enable::initialized_agents(repo_root);
    if (agents.iter().any(|agent| agent == "cursor") || cursor_hooks_installed(repo_root))
        && !agents.iter().any(|agent| agent == "cursor")
    {
        agents.push("cursor".to_string());
    }
    agents.sort();
    agents
}

fn cursor_hooks_installed(repo_root: &Path) -> bool {
    let hooks_path = repo_root.join(".cursor").join("hooks.json");
    let Ok(data) = std::fs::read(&hooks_path) else {
        return false;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&data) else {
        return false;
    };
    let Some(hooks) = value.get("hooks").and_then(serde_json::Value::as_object) else {
        return false;
    };
    hooks.values().any(|entries| {
        entries.as_array().is_some_and(|items| {
            items.iter().any(|entry| {
                entry
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|command| command.starts_with("bitloops "))
            })
        })
    })
}

pub(crate) fn join_agent_names(agents: &[String]) -> String {
    agents.join(",")
}

pub fn run_send_analytics_command(
    args: &crate::engine::telemetry::SendAnalyticsArgs,
) -> Result<()> {
    crate::engine::telemetry::send_event(&args.payload);
    Ok(())
}

pub(crate) fn write_completion(w: &mut dyn Write, shell: CompletionShell) -> Result<()> {
    let mut cmd = crate::commands::Cli::command();
    // clap_complete splits subcommand paths using "__". Our hidden
    // "__send_analytics" command conflicts with that separator and causes a
    // panic during completion generation, so we rename only in this generated
    // tree. Runtime parsing remains unchanged.
    cmd = cmd.mut_subcommand("__send_analytics", |sub| {
        sub.name("send-analytics-internal")
    });
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

pub fn run_version_command() -> Result<()> {
    let runtime = runtime_version_string();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    write_version(
        &mut out,
        build_version(),
        build_commit(),
        &runtime,
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
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
    let root = crate::commands::Cli::command();
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

fn runtime_version_string() -> String {
    let output = ProcessCommand::new("rustc").arg("--version").output();
    let Ok(output) = output else {
        return "unknown".to_string();
    };
    if !output.status.success() {
        return "unknown".to_string();
    }

    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

pub(crate) fn write_version(
    w: &mut dyn Write,
    version: &str,
    commit: &str,
    runtime: &str,
    os: &str,
    arch: &str,
) -> Result<()> {
    writeln!(w, "Bitloops CLI {version} ({commit})")?;
    writeln!(w, "Rust version: {runtime}")?;
    writeln!(w, "OS/Arch: {os}/{arch}")?;
    Ok(())
}
