use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
#[cfg(test)]
use std::{cell::RefCell, rc::Rc};

use crate::cli::embeddings::{
    EmbeddingsInstallState, EmbeddingsRuntime, inspect_embeddings_install_state,
};
use crate::cli::inference::{
    SummarySetupSelection, prompt_summary_setup_selection, summary_generation_configured,
};
use crate::cli::telemetry_consent;
use crate::config::{REPO_POLICY_LOCAL_FILE_NAME, bootstrap_default_daemon_environment};
use crate::devql_transport::SlimCliRepoScope;

#[path = "init/agent_hooks.rs"]
mod agent_hooks;
#[path = "init/agent_selection.rs"]
mod agent_selection;
#[path = "init/progress.rs"]
mod progress;
#[path = "init/workflow.rs"]
mod workflow;

pub use agent_selection::detect_or_select_agent;

pub type AgentSelector = dyn Fn(&[String]) -> std::result::Result<Vec<String>, String>;
const DEFAULT_INIT_INGEST_BACKFILL: usize = 50;

#[cfg(test)]
type InstallDefaultDaemonHook = dyn Fn(bool) -> Result<()> + 'static;

#[derive(Clone)]
struct QueuedEmbeddingsBootstrapTask {
    scope: SlimCliRepoScope,
    task_id: String,
}

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

    /// Target specific agent setups (repeatable).
    #[arg(long = "agent", value_name = "AGENT")]
    pub agent: Vec<String>,

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

    /// Exclude repo-relative paths/globs from DevQL indexing (repeatable).
    #[arg(long = "exclude")]
    pub exclude: Vec<String>,

    /// Load additional exclusion globs from files under the repo-policy root (repeatable).
    #[arg(long = "exclude-from")]
    pub exclude_from: Vec<String>,

    /// Select which embeddings runtime to configure when embeddings are installed during init.
    #[arg(long, value_enum, default_value_t = EmbeddingsRuntime::Local)]
    pub embeddings_runtime: EmbeddingsRuntime,

    /// Public platform embeddings endpoint used when `--embeddings-runtime platform` is selected.
    #[arg(long)]
    pub embeddings_gateway_url: Option<String>,

    /// Environment variable that contains the platform gateway bearer token.
    #[arg(long, default_value = "BITLOOPS_PLATFORM_GATEWAY_TOKEN")]
    pub embeddings_api_key_env: String,
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
    workflow::run_for_project_root(args, project_root, out, input, select_fn).await
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

pub(super) async fn choose_summary_setup_during_init(
    repo_root: &Path,
    install_default_daemon: bool,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<SummarySetupSelection> {
    if summary_generation_configured(repo_root) {
        return Ok(SummarySetupSelection::Skip);
    }

    let cloud_logged_in = crate::daemon::resolve_workos_session_status()
        .await?
        .is_some();

    prompt_summary_setup_selection(
        out,
        input,
        telemetry_consent::can_prompt_interactively(),
        install_default_daemon,
        cloud_logged_in,
    )
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

    writeln!(out)?;
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

fn normalize_cli_exclusions(values: &[String]) -> Vec<String> {
    let mut normalized = values
        .iter()
        .map(|value| value.trim().replace('\\', "/"))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn normalize_exclude_from_paths(policy_root: &Path, values: &[String]) -> Result<Vec<String>> {
    let policy_root = policy_root
        .canonicalize()
        .unwrap_or_else(|_| policy_root.to_path_buf());
    let mut normalized = Vec::new();

    for raw_value in values {
        let raw_value = raw_value.trim();
        if raw_value.is_empty() {
            continue;
        }
        let candidate = PathBuf::from(raw_value);
        let absolute = if candidate.is_absolute() {
            candidate
        } else {
            policy_root.join(candidate)
        };
        let absolute = normalize_lexical_path(&absolute);
        if !absolute.starts_with(&policy_root) {
            bail!(
                "`--exclude-from` path `{}` must be under repo-policy root {}",
                raw_value,
                policy_root.display()
            );
        }
        let relative = absolute
            .strip_prefix(&policy_root)
            .unwrap_or(absolute.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        if !relative.is_empty() {
            normalized.push(relative);
        }
    }

    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
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

#[cfg(test)]
mod tests;
