use std::io::{self, BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::Args;
#[cfg(test)]
use std::{cell::RefCell, rc::Rc};

mod agent_hooks;
mod agent_selection;
use crate::adapters::agents::AgentAdapterRegistry;
use crate::adapters::agents::claude_code::git_hooks;
use crate::cli::embeddings::{
    EmbeddingsInstallState, inspect_embeddings_install_state, install_or_bootstrap_embeddings,
};
use crate::cli::telemetry_consent;
use crate::config::settings::{
    DEFAULT_STRATEGY, load_settings, write_project_bootstrap_settings_with_daemon_binding,
};
use crate::config::{
    REPO_POLICY_LOCAL_FILE_NAME, bootstrap_default_daemon_environment, default_daemon_config_exists,
};
use crate::devql_transport::discover_slim_cli_repo_scope;

pub use agent_selection::detect_or_select_agent;

pub type AgentSelector = dyn Fn(&[String]) -> std::result::Result<Vec<String>, String>;
const DEFAULT_INIT_INGEST_BACKFILL: usize = 50;

#[cfg(test)]
type InstallDefaultDaemonHook = dyn Fn(bool) -> Result<()> + 'static;

#[cfg(test)]
thread_local! {
    static INSTALL_DEFAULT_DAEMON_HOOK: RefCell<Option<Rc<InstallDefaultDaemonHook>>> =
        RefCell::new(None);
}

#[derive(Args)]
pub struct InitArgs {
    /// Bootstrap and start the default Bitloops daemon service if it is not already running.
    #[arg(long, default_value_t = false)]
    pub install_default_daemon: bool,

    /// Remove and reinstall existing hooks for selected agents.
    #[arg(long, short = 'f')]
    pub force: bool,

    /// Target a specific agent setup (claude-code|copilot|cursor|gemini|opencode).
    #[arg(long)]
    pub agent: Option<String>,

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

    /// Accepted for compatibility; `bitloops init` no longer runs the initial baseline sync.
    #[arg(long, default_value_t = false)]
    pub skip_baseline: bool,

    /// Queue an initial DevQL sync after hook setup.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub sync: Option<bool>,

    /// Run historical DevQL ingest after hook setup.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub ingest: Option<bool>,

    /// Bound init-triggered historical ingest to the latest N commits (bare flag = 50).
    #[arg(
        long,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "50",
        value_parser = parse_backfill_value
    )]
    pub backfill: Option<usize>,
}

pub async fn run(args: InitArgs) -> Result<()> {
    let mut out = io::stdout().lock();
    let stdin = io::stdin();
    let mut input = stdin.lock();
    run_with_io_async(args, &mut out, &mut input, None).await
}

#[cfg(test)]
fn run_with_writer_for_project_root(
    args: InitArgs,
    project_root: &Path,
    out: &mut dyn Write,
    select_fn: Option<&AgentSelector>,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("creating runtime for `bitloops init`")?;
    let mut input = io::Cursor::new(Vec::<u8>::new());
    runtime.block_on(run_with_io_async_for_project_root(
        args,
        project_root,
        out,
        &mut input,
        select_fn,
    ))
}

async fn run_with_io_async(
    args: InitArgs,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    select_fn: Option<&AgentSelector>,
) -> Result<()> {
    let project_root = std::env::current_dir().context("getting current directory")?;
    run_with_io_async_for_project_root(args, &project_root, out, input, select_fn).await
}

async fn run_with_io_async_for_project_root(
    args: InitArgs,
    project_root: &Path,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    select_fn: Option<&AgentSelector>,
) -> Result<()> {
    let git_root = crate::cli::enable::find_repo_root(project_root)?;
    let daemon_config_existed_at_entry = default_daemon_config_exists()?;
    let telemetry_choice =
        telemetry_consent::telemetry_flag_choice(args.telemetry, args.no_telemetry);
    if args.backfill.is_some() && args.ingest == Some(false) {
        bail!("`bitloops init --backfill` cannot be combined with `--ingest=false`.");
    }
    let effective_ingest = if args.backfill.is_some() {
        Some(true)
    } else {
        args.ingest
    };

    if (args.sync.is_none() || effective_ingest.is_none())
        && !telemetry_consent::can_prompt_interactively()
    {
        bail!(
            "`bitloops init` requires explicit `--sync=true|false` and `--ingest=true|false` choices when not running interactively."
        );
    }

    if !daemon_config_existed_at_entry
        && args.install_default_daemon
        && telemetry_choice.is_none()
        && !telemetry_consent::can_prompt_interactively()
    {
        bail!(telemetry_consent::NON_INTERACTIVE_TELEMETRY_ERROR);
    }

    maybe_install_default_daemon(args.install_default_daemon).await?;
    telemetry_consent::ensure_default_daemon_running().await?;
    let daemon_config_path = bound_running_daemon_config_path().await?;
    if daemon_config_existed_at_entry {
        telemetry_consent::ensure_existing_config_telemetry_consent(
            project_root,
            telemetry_choice,
            out,
            input,
        )
        .await?;
    } else if let Some(choice) = telemetry_choice {
        let persisted =
            telemetry_consent::update_cli_telemetry_consent_via_daemon(project_root, Some(choice))
                .await?;
        if persisted.needs_prompt {
            bail!("failed to persist telemetry consent");
        }
    }
    ensure_repo_local_policy_excluded(&git_root, project_root)?;

    let selected_agents = if let Some(agent) = args.agent.as_deref() {
        vec![AgentAdapterRegistry::builtin().normalise_agent_name(agent)?]
    } else {
        detect_or_select_agent(project_root, out, select_fn)?
    };
    let strategy = load_settings(project_root)
        .map(|settings| settings.strategy)
        .unwrap_or_else(|_| DEFAULT_STRATEGY.to_string());
    let local_policy_path = project_root.join(REPO_POLICY_LOCAL_FILE_NAME);
    write_project_bootstrap_settings_with_daemon_binding(
        &local_policy_path,
        &strategy,
        &selected_agents,
        Some(&daemon_config_path),
    )?;

    let settings = load_settings(project_root).unwrap_or_default();
    let git_count = git_hooks::install_git_hooks(&git_root, settings.local_dev)?;
    if git_count > 0 {
        writeln!(out, "Installed {git_count} git hook(s).")?;
    }

    reconcile_agent_hooks(
        project_root,
        &selected_agents,
        settings.local_dev,
        args.force,
        out,
    )?;

    let should_install_embeddings = should_install_embeddings_during_init(
        project_root,
        args.install_default_daemon,
        out,
        input,
    )?;
    let defer_embeddings_install_until_after_sync =
        should_install_embeddings && args.install_default_daemon;
    if should_install_embeddings && !defer_embeddings_install_until_after_sync {
        install_embeddings_during_init(project_root, out)?;
    }

    let should_sync = should_run_initial_sync(args.sync, out, input)?;
    let should_ingest = should_run_initial_ingest(effective_ingest, out, input)?;
    if should_sync || should_ingest {
        let scope = discover_slim_cli_repo_scope(Some(project_root))?;
        if should_sync {
            writeln!(out, "Starting initial DevQL sync...")?;
            out.flush()?;
            let (task, _merged) = crate::cli::devql::graphql::enqueue_sync_via_graphql(
                &scope, false, None, false, false, "init", false,
            )
            .await?;
            if let Some(summary) =
                crate::cli::devql::graphql::watch_sync_task_via_graphql(&scope, task.clone())
                    .await?
            {
                writeln!(
                    out,
                    "{}",
                    crate::cli::devql::format_sync_completion_summary(&summary)
                )?;
            }
        }
        if should_ingest {
            if should_sync {
                writeln!(out, "Starting initial DevQL ingest after sync...")?;
            } else {
                writeln!(out, "Starting initial DevQL ingest...")?;
            }
            out.flush()?;
            crate::cli::devql::graphql::run_ingest_via_graphql(
                &scope,
                Some(args.backfill.unwrap_or(DEFAULT_INIT_INGEST_BACKFILL)),
                false,
            )
            .await?;
        }
    }
    if defer_embeddings_install_until_after_sync {
        install_embeddings_during_init(project_root, out)?;
    }
    Ok(())
}

async fn bound_running_daemon_config_path() -> Result<std::path::PathBuf> {
    if let Some(runtime) = crate::daemon::status().await?.runtime {
        return Ok(runtime
            .config_path
            .canonicalize()
            .unwrap_or(runtime.config_path));
    }

    #[cfg(test)]
    if crate::cli::telemetry_consent::test_assume_daemon_running_override() == Some(true) {
        let config_path = crate::config::ensure_daemon_config_exists()?;
        return Ok(config_path.canonicalize().unwrap_or(config_path));
    }

    #[cfg(test)]
    if std::env::var("BITLOOPS_TEST_ASSUME_DAEMON_RUNNING")
        .ok()
        .is_some_and(|value| !value.trim().is_empty() && value.trim() != "0")
    {
        let config_path = crate::config::ensure_daemon_config_exists()?;
        return Ok(config_path.canonicalize().unwrap_or(config_path));
    }

    let runtime = crate::daemon::status()
        .await?
        .runtime
        .context("Bitloops daemon is not running")?;
    Ok(runtime
        .config_path
        .canonicalize()
        .unwrap_or(runtime.config_path))
}

fn install_embeddings_during_init(project_root: &Path, out: &mut dyn Write) -> Result<()> {
    writeln!(out, "Preparing local embeddings setup...")?;
    writeln!(
        out,
        "This can take a moment if the managed runtime needs to be downloaded."
    )?;
    out.flush()?;
    match install_or_bootstrap_embeddings(project_root) {
        Ok(lines) => {
            for line in lines {
                writeln!(out, "{line}")?;
            }
            Ok(())
        }
        Err(err) => {
            bail!("Bitloops init completed, but embeddings installation failed: {err:#}");
        }
    }
}

fn should_install_embeddings_during_init(
    repo_root: &Path,
    explicit_install: bool,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<bool> {
    if explicit_install {
        return Ok(true);
    }

    if !telemetry_consent::can_prompt_interactively() {
        return Ok(false);
    }

    if !matches!(
        inspect_embeddings_install_state(repo_root),
        EmbeddingsInstallState::NotConfigured
    ) {
        return Ok(false);
    }

    prompt_install_embeddings(out, input)
}

fn prompt_install_embeddings(out: &mut dyn Write, input: &mut dyn BufRead) -> Result<bool> {
    writeln!(out)?;
    writeln!(out, "Install local embeddings as well?")?;
    writeln!(
        out,
        "This is recommended and lets sync and ingest include them."
    )?;

    loop {
        writeln!(out, "Install embeddings now? (Y/n)")?;
        write!(out, "> ")?;
        out.flush()?;

        let mut line = String::new();
        input
            .read_line(&mut line)
            .context("reading init embeddings install prompt response")?;
        match line.trim().to_ascii_lowercase().as_str() {
            "" | "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => writeln!(out, "Please answer yes or no.")?,
        }
    }
}

fn should_run_initial_sync(
    sync: Option<bool>,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<bool> {
    if let Some(sync) = sync {
        return Ok(sync);
    }
    if !telemetry_consent::can_prompt_interactively() {
        bail!(
            "`bitloops init` requires explicit `--sync=true|false` and `--ingest=true|false` choices when not running interactively."
        );
    }

    writeln!(out, "Would you like to sync your codebase now (Y/n)?")?;
    write!(out, "> ")?;
    out.flush()?;
    let mut response = String::new();
    input
        .read_line(&mut response)
        .context("reading initial sync choice for `bitloops init`")?;
    let response = response.trim().to_ascii_lowercase();
    Ok(matches!(response.as_str(), "" | "y" | "yes"))
}

fn should_run_initial_ingest(
    ingest: Option<bool>,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<bool> {
    if let Some(ingest) = ingest {
        return Ok(ingest);
    }
    if !telemetry_consent::can_prompt_interactively() {
        bail!(
            "`bitloops init` requires explicit `--sync=true|false` and `--ingest=true|false` choices when not running interactively."
        );
    }

    writeln!(
        out,
        "Would you like to ingest your commit history now (Y/n)?"
    )?;
    write!(out, "> ")?;
    out.flush()?;
    let mut response = String::new();
    input
        .read_line(&mut response)
        .context("reading initial ingest choice for `bitloops init`")?;
    let response = response.trim().to_ascii_lowercase();
    Ok(matches!(response.as_str(), "" | "y" | "yes"))
}

fn parse_backfill_value(raw: &str) -> std::result::Result<usize, String> {
    let parsed = raw
        .parse::<usize>()
        .map_err(|_| format!("invalid value `{raw}` for `--backfill`"))?;
    if parsed == 0 {
        return Err("`--backfill` must be greater than zero".to_string());
    }
    Ok(parsed)
}

async fn maybe_install_default_daemon(install_default_daemon: bool) -> Result<()> {
    #[cfg(test)]
    if let Some(result) = maybe_run_install_default_daemon_hook(install_default_daemon) {
        return result;
    }

    if !install_default_daemon {
        return Ok(());
    }

    let status = crate::daemon::status().await?;
    if status.runtime.is_some() {
        return Ok(());
    }

    let config_path = bootstrap_default_daemon_environment()?;
    let daemon_config = crate::daemon::resolve_daemon_config(Some(config_path.as_path()))?;
    let config = crate::api::DashboardServerConfig {
        host: None,
        port: crate::api::DEFAULT_DASHBOARD_PORT,
        no_open: true,
        force_http: false,
        recheck_local_dashboard_net: false,
        bundle_dir: None,
    };
    let _ = crate::daemon::start_service(&daemon_config, config, None).await?;
    Ok(())
}

#[cfg(test)]
fn maybe_run_install_default_daemon_hook(install_default_daemon: bool) -> Option<Result<()>> {
    INSTALL_DEFAULT_DAEMON_HOOK.with(|cell: &RefCell<Option<Rc<InstallDefaultDaemonHook>>>| {
        cell.borrow()
            .as_ref()
            .map(|hook| hook(install_default_daemon))
    })
}

#[cfg(test)]
pub(super) fn with_install_default_daemon_hook<T>(
    hook: impl Fn(bool) -> Result<()> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    INSTALL_DEFAULT_DAEMON_HOOK.with(|cell: &RefCell<Option<Rc<InstallDefaultDaemonHook>>>| {
        assert!(
            cell.borrow().is_none(),
            "install default daemon hook already installed"
        );
        *cell.borrow_mut() = Some(Rc::new(hook));
    });
    let result = f();
    INSTALL_DEFAULT_DAEMON_HOOK.with(|cell: &RefCell<Option<Rc<InstallDefaultDaemonHook>>>| {
        *cell.borrow_mut() = None;
    });
    result
}

fn ensure_repo_local_policy_excluded(git_root: &Path, project_root: &Path) -> Result<()> {
    let exclude_path = git_root.join(".git").join("info").join("exclude");
    if let Some(parent) = exclude_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating git exclude directory {}", parent.display()))?;
    }

    let mut content = std::fs::read_to_string(&exclude_path).unwrap_or_default();
    let relative_local_policy = project_root
        .strip_prefix(git_root)
        .unwrap_or(project_root)
        .join(REPO_POLICY_LOCAL_FILE_NAME);
    let relative_local_policy = relative_local_policy.to_string_lossy().replace('\\', "/");

    let entry = relative_local_policy.as_str();
    if !content.lines().any(|line| line.trim() == entry) {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(entry);
        content.push('\n');
    }

    std::fs::write(&exclude_path, content)
        .with_context(|| format!("writing {}", exclude_path.display()))?;
    Ok(())
}

fn reconcile_agent_hooks(
    project_root: &Path,
    selected_agents: &[String],
    local_dev: bool,
    force: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let registry = AgentAdapterRegistry::builtin();
    let selected = selected_agents
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();

    for agent in registry.installed_agents(project_root) {
        if selected.contains(&agent) {
            continue;
        }
        let label = registry.uninstall_agent_hooks(project_root, &agent)?;
        writeln!(out, "Removed {label} hooks.")?;
    }

    for agent in selected_agents {
        let (label, installed) =
            registry.install_agent_hooks(project_root, agent, local_dev, force)?;
        if installed > 0 {
            writeln!(out, "Installed {installed} {label} hook(s).")?;
        } else {
            writeln!(out, "{label} hooks are already initialised.")?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
