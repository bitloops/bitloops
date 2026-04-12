use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Args;
#[cfg(test)]
use std::{cell::RefCell, rc::Rc};
use terminal_size::{Width, terminal_size};

mod agent_hooks;
mod agent_selection;
use crate::adapters::agents::AgentAdapterRegistry;
use crate::adapters::agents::claude_code::git_hooks;
use crate::cli::embeddings::{
    EmbeddingsInstallState, enqueue_embeddings_bootstrap_task, inspect_embeddings_install_state,
    install_or_bootstrap_embeddings,
};
use crate::cli::telemetry_consent;
use crate::config::settings::{
    DEFAULT_STRATEGY, load_settings, write_project_bootstrap_settings_with_daemon_binding,
};
use crate::config::{
    REPO_POLICY_LOCAL_FILE_NAME, bootstrap_default_daemon_environment, default_daemon_config_exists,
};
use crate::devql_transport::{SlimCliRepoScope, discover_slim_cli_repo_scope};
use crate::utils::branding::{BITLOOPS_PURPLE_HEX, bitloops_wordmark, color_hex_if_enabled};

pub use agent_selection::detect_or_select_agent;

pub type AgentSelector = dyn Fn(&[String]) -> std::result::Result<Vec<String>, String>;
const DEFAULT_INIT_INGEST_BACKFILL: usize = 50;
const INIT_PROGRESS_POLL_INTERVAL: Duration = Duration::from_secs(1);
const INIT_PROGRESS_TICK_INTERVAL: Duration = Duration::from_millis(120);
const INIT_SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

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

    let mut queued_embeddings_bootstrap = None;
    let should_install_embeddings = should_install_embeddings_during_init(
        project_root,
        args.install_default_daemon,
        out,
        input,
    )?;
    if should_install_embeddings {
        if args.install_default_daemon {
            queued_embeddings_bootstrap =
                Some(enqueue_embeddings_bootstrap_during_init(project_root, out).await?);
        } else {
            install_embeddings_during_init(project_root, out)?;
        }
    }
    let should_sync = should_run_initial_sync(args.sync, out, input)?;
    let should_ingest = should_run_initial_ingest(effective_ingest, out, input)?;
    if args.install_default_daemon {
        write_init_setup_handoff(out).await?;
    }
    if should_sync || should_ingest {
        let scope = discover_slim_cli_repo_scope(Some(project_root))?;
        let run_concurrent_init_progress =
            args.install_default_daemon && queued_embeddings_bootstrap.is_some();
        if run_concurrent_init_progress {
            let initial_top_task = if should_sync {
                writeln!(out, "Starting initial DevQL sync...")?;
                out.flush()?;
                let (task, _merged) = crate::cli::devql::graphql::enqueue_sync_task_via_graphql(
                    &scope, false, None, false, false, "init", false,
                )
                .await?;
                Some(task)
            } else if should_ingest {
                writeln!(out, "Starting initial DevQL ingest...")?;
                out.flush()?;
                let (task, _merged) = crate::cli::devql::graphql::enqueue_ingest_task_via_graphql(
                    &scope,
                    Some(args.backfill.unwrap_or(DEFAULT_INIT_INGEST_BACKFILL)),
                    false,
                )
                .await?;
                Some(task)
            } else {
                None
            };
            run_dual_init_progress(
                out,
                &scope,
                initial_top_task,
                should_sync && should_ingest,
                args.backfill.unwrap_or(DEFAULT_INIT_INGEST_BACKFILL),
                queued_embeddings_bootstrap.as_ref(),
            )
            .await?;
        } else if should_sync {
            writeln!(out, "Starting initial DevQL sync...")?;
            out.flush()?;
            let (task, _merged) = crate::cli::devql::graphql::enqueue_sync_task_via_graphql(
                &scope, false, None, false, false, "init", false,
            )
            .await?;
            if let Some(task) =
                crate::cli::devql::graphql::watch_task_via_graphql(&scope, task.clone()).await?
            {
                writeln!(
                    out,
                    "{}",
                    crate::cli::devql::format_task_completion_summary(&task)
                )?;
            }
            if should_ingest {
                writeln!(out, "Starting initial DevQL ingest after sync...")?;
                out.flush()?;
                let (task, _merged) = crate::cli::devql::graphql::enqueue_ingest_task_via_graphql(
                    &scope,
                    Some(args.backfill.unwrap_or(DEFAULT_INIT_INGEST_BACKFILL)),
                    false,
                )
                .await?;
                if let Some(task) =
                    crate::cli::devql::graphql::watch_task_via_graphql(&scope, task).await?
                {
                    writeln!(
                        out,
                        "{}",
                        crate::cli::devql::format_task_completion_summary(&task)
                    )?;
                }
            }
        } else if should_ingest {
            if should_sync {
                writeln!(out, "Starting initial DevQL ingest after sync...")?;
            } else {
                writeln!(out, "Starting initial DevQL ingest...")?;
            }
            out.flush()?;
            let (task, _merged) = crate::cli::devql::graphql::enqueue_ingest_task_via_graphql(
                &scope,
                Some(args.backfill.unwrap_or(DEFAULT_INIT_INGEST_BACKFILL)),
                false,
            )
            .await?;
            if let Some(task) =
                crate::cli::devql::graphql::watch_task_via_graphql(&scope, task).await?
            {
                writeln!(
                    out,
                    "{}",
                    crate::cli::devql::format_task_completion_summary(&task)
                )?;
            }
        }
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

async fn enqueue_embeddings_bootstrap_during_init(
    project_root: &Path,
    out: &mut dyn Write,
) -> Result<QueuedEmbeddingsBootstrapTask> {
    writeln!(out, "Queueing embeddings bootstrap in the daemon...")?;
    let (scope, queued) =
        enqueue_embeddings_bootstrap_task(project_root, None, crate::daemon::DevqlTaskSource::Init)
            .await?;
    let phase = queued
        .task
        .embeddings_bootstrap_progress()
        .map(|progress| progress.phase.as_str())
        .unwrap_or("queued");
    writeln!(out, "Embeddings bootstrap task: {}", queued.task.task_id)?;
    writeln!(out, "Embeddings bootstrap phase: {phase}")?;
    out.flush()?;
    Ok(QueuedEmbeddingsBootstrapTask {
        scope,
        task_id: queued.task.task_id,
    })
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

async fn write_init_setup_handoff(out: &mut dyn Write) -> Result<()> {
    writeln!(out)?;
    writeln!(
        out,
        "{}",
        color_hex_if_enabled(&bitloops_wordmark(), BITLOOPS_PURPLE_HEX)
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "The setup is complete! You can continue on with your work and Bitloops will continue enriching your codebase's Intelligence Layer in the background. You can continue viewing the progress here or you can close this terminal if you prefer and you can always run `bitloops status` or visit the dashboard for more information."
    )?;
    if let Some(url) = current_dashboard_url().await? {
        writeln!(out, "Dashboard URL: {url}")?;
    }
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

async fn current_dashboard_url() -> Result<Option<String>> {
    Ok(crate::daemon::status()
        .await
        .ok()
        .and_then(|status| status.runtime.map(|runtime| runtime.url)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EmbeddingQueueSnapshot {
    pending: u64,
    running: u64,
    failed: u64,
}

impl EmbeddingQueueSnapshot {
    fn remaining(self) -> u64 {
        self.pending + self.running
    }
}

async fn current_embedding_queue_snapshot() -> Result<Option<EmbeddingQueueSnapshot>> {
    Ok(crate::daemon::status()
        .await?
        .enrichment
        .map(|status| EmbeddingQueueSnapshot {
            pending: status.state.pending_embedding_jobs,
            running: status.state.running_embedding_jobs,
            failed: status.state.failed_embedding_jobs,
        }))
}

enum BottomProgressState {
    Bootstrap(crate::cli::devql::graphql::TaskGraphqlRecord),
    Queue {
        snapshot: EmbeddingQueueSnapshot,
        baseline_total: u64,
    },
    QueueComplete {
        failed_jobs: u64,
    },
    BootstrapFailed(crate::cli::devql::graphql::TaskGraphqlRecord),
    Hidden,
}

struct InitProgressRenderer {
    interactive: bool,
    terminal_width: Option<usize>,
    spinner_index: usize,
    last_frame: Option<String>,
    wrote_in_place: bool,
    rendered_lines: usize,
}

impl InitProgressRenderer {
    fn new() -> Self {
        Self {
            interactive: std::io::stdout().is_terminal() && std::env::var("ACCESSIBLE").is_err(),
            terminal_width: terminal_size().map(|(Width(width), _)| width as usize),
            spinner_index: 0,
            last_frame: None,
            wrote_in_place: false,
            rendered_lines: 0,
        }
    }

    fn is_interactive(&self) -> bool {
        self.interactive
    }

    fn render(
        &mut self,
        out: &mut dyn Write,
        top_task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
        bottom_state: &BottomProgressState,
    ) -> Result<()> {
        let frame = self.render_frame(top_task, bottom_state);
        self.write_frame(out, frame, false)
    }

    fn tick(
        &mut self,
        out: &mut dyn Write,
        top_task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
        bottom_state: &BottomProgressState,
    ) -> Result<()> {
        if !self.interactive {
            return Ok(());
        }
        self.spinner_index = (self.spinner_index + 1) % INIT_SPINNER_FRAMES.len();
        let frame = self.render_frame(top_task, bottom_state);
        self.write_frame(out, frame, true)
    }

    fn finish(&mut self, out: &mut dyn Write) -> Result<()> {
        if self.interactive && self.wrote_in_place {
            writeln!(out)?;
            out.flush()?;
            self.wrote_in_place = false;
        }
        Ok(())
    }

    fn render_frame(
        &self,
        top_task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
        bottom_state: &BottomProgressState,
    ) -> String {
        let mut lines = Vec::new();
        let spinner =
            color_hex_if_enabled(INIT_SPINNER_FRAMES[self.spinner_index], BITLOOPS_PURPLE_HEX);
        if let Some(task) = top_task {
            lines.push(
                crate::cli::devql::graphql::format_live_task_progress_bar_line(
                    task,
                    self.spinner_index,
                    self.terminal_width,
                ),
            );
            lines.push(crate::cli::devql::graphql::format_live_task_status_line(
                task,
                spinner.as_str(),
                self.terminal_width,
            ));
        }
        match bottom_state {
            BottomProgressState::Bootstrap(task) | BottomProgressState::BootstrapFailed(task) => {
                lines.push(
                    crate::cli::devql::graphql::format_live_task_progress_bar_line(
                        task,
                        self.spinner_index,
                        self.terminal_width,
                    ),
                );
                lines.push(crate::cli::devql::graphql::format_live_task_status_line(
                    task,
                    spinner.as_str(),
                    self.terminal_width,
                ));
            }
            BottomProgressState::Queue {
                snapshot,
                baseline_total,
            } => {
                lines.push(format_embedding_queue_progress_bar_line(
                    *snapshot,
                    *baseline_total,
                    self.spinner_index,
                    self.terminal_width,
                ));
                lines.push(format_embedding_queue_status_line(
                    *snapshot,
                    spinner.as_str(),
                ));
            }
            BottomProgressState::QueueComplete { failed_jobs } => {
                lines.push(
                    "[████████████████████████████████████████████████████████████] 100% 1/1"
                        .to_string(),
                );
                if *failed_jobs > 0 {
                    lines.push(format!(
                        "✖ Embedding queue finished with {} failed job(s)",
                        failed_jobs
                    ));
                } else {
                    lines.push("✓ Embedding queue complete".to_string());
                }
            }
            BottomProgressState::Hidden => {}
        }
        lines.join("\n")
    }

    fn write_frame(&mut self, out: &mut dyn Write, frame: String, force: bool) -> Result<()> {
        if self.interactive {
            if !force && self.last_frame.as_deref() == Some(frame.as_str()) {
                return Ok(());
            }
            if self.wrote_in_place {
                clear_rendered_lines(out, self.rendered_lines)?;
            } else {
                write!(out, "{frame}")?;
                out.flush()?;
                self.last_frame = Some(frame.clone());
                self.wrote_in_place = true;
                self.rendered_lines = frame.lines().count().max(1);
                return Ok(());
            }
            write!(out, "{frame}")?;
            out.flush()?;
            self.last_frame = Some(frame.clone());
            self.wrote_in_place = true;
            self.rendered_lines = frame.lines().count().max(1);
            return Ok(());
        }

        if self.last_frame.as_deref() != Some(frame.as_str()) {
            writeln!(out, "{frame}")?;
            out.flush()?;
            self.last_frame = Some(frame);
        }
        Ok(())
    }
}

fn clear_rendered_lines(out: &mut dyn Write, line_count: usize) -> Result<()> {
    if line_count == 0 {
        return Ok(());
    }
    write!(out, "\r\x1b[2K")?;
    for _ in 1..line_count {
        write!(out, "\x1b[1A\r\x1b[2K")?;
    }
    Ok(())
}

async fn run_dual_init_progress(
    out: &mut dyn Write,
    scope: &SlimCliRepoScope,
    mut top_task: Option<crate::cli::devql::graphql::TaskGraphqlRecord>,
    mut enqueue_ingest_after_sync: bool,
    ingest_backfill: usize,
    queued_embeddings_bootstrap: Option<&QueuedEmbeddingsBootstrapTask>,
) -> Result<()> {
    let mut bottom_state = if let Some(bootstrap) = queued_embeddings_bootstrap {
        match crate::cli::devql::graphql::query_task_via_graphql(
            &bootstrap.scope,
            bootstrap.task_id.as_str(),
        )
        .await?
        {
            Some(task) => BottomProgressState::Bootstrap(task),
            None => BottomProgressState::Hidden,
        }
    } else {
        BottomProgressState::Hidden
    };
    let mut renderer = InitProgressRenderer::new();
    renderer.render(out, top_task.as_ref(), &bottom_state)?;

    let mut poll_interval = tokio::time::interval(INIT_PROGRESS_POLL_INTERVAL);
    let mut render_tick = tokio::time::interval(INIT_PROGRESS_TICK_INTERVAL);
    loop {
        tokio::select! {
            _ = render_tick.tick(), if renderer.is_interactive() => {
                renderer.tick(out, top_task.as_ref(), &bottom_state)?;
            }
            _ = poll_interval.tick() => {
                if let Some(current) = top_task.clone() {
                    let task_id = current.task_id.clone();
                    let refreshed = crate::cli::devql::graphql::query_task_via_graphql(scope, task_id.as_str())
                        .await?
                        .unwrap_or(current);
                    if refreshed.is_terminal() {
                        if refreshed.status.eq_ignore_ascii_case("completed") {
                            if refreshed.is_sync() && enqueue_ingest_after_sync {
                                let (ingest_task, _merged) = crate::cli::devql::graphql::enqueue_ingest_task_via_graphql(
                                    scope,
                                    Some(ingest_backfill),
                                    false,
                                ).await?;
                                top_task = Some(ingest_task);
                                enqueue_ingest_after_sync = false;
                            } else {
                                top_task = None;
                            }
                        } else if let Some(error) = refreshed.error.as_ref() {
                            renderer.finish(out)?;
                            bail!("task {} failed: {error}", refreshed.task_id);
                        } else {
                            renderer.finish(out)?;
                            bail!(
                                "task {} ended with status {}",
                                refreshed.task_id,
                                refreshed.status
                            );
                        }
                    } else {
                        top_task = Some(refreshed);
                    }
                }

                bottom_state = match bottom_state {
                    BottomProgressState::Bootstrap(current_task) => {
                        let refreshed = crate::cli::devql::graphql::query_task_via_graphql(
                            scope,
                            current_task.task_id.as_str(),
                        )
                        .await?
                        .unwrap_or(current_task);
                        if refreshed.is_terminal() {
                            if refreshed.status.eq_ignore_ascii_case("completed") {
                                if let Some(snapshot) = current_embedding_queue_snapshot().await? {
                                    if snapshot.remaining() > 0 || snapshot.failed > 0 {
                                        BottomProgressState::Queue {
                                            baseline_total: snapshot.remaining(),
                                            snapshot,
                                        }
                                    } else {
                                        BottomProgressState::QueueComplete { failed_jobs: 0 }
                                    }
                                } else {
                                    BottomProgressState::Hidden
                                }
                            } else {
                                BottomProgressState::BootstrapFailed(refreshed)
                            }
                        } else {
                            BottomProgressState::Bootstrap(refreshed)
                        }
                    }
                    BottomProgressState::Queue {
                        snapshot: _,
                        baseline_total,
                    } => {
                        if let Some(snapshot) = current_embedding_queue_snapshot().await? {
                            let baseline_total = baseline_total.max(snapshot.remaining());
                            if snapshot.remaining() == 0 {
                                BottomProgressState::QueueComplete {
                                    failed_jobs: snapshot.failed,
                                }
                            } else {
                                BottomProgressState::Queue {
                                    snapshot,
                                    baseline_total,
                                }
                            }
                        } else {
                            BottomProgressState::Hidden
                        }
                    }
                    other => other,
                };

                renderer.render(out, top_task.as_ref(), &bottom_state)?;
                if top_task.is_none()
                    && matches!(
                        bottom_state,
                        BottomProgressState::Hidden
                            | BottomProgressState::QueueComplete { .. }
                            | BottomProgressState::BootstrapFailed(_)
                    )
                {
                    renderer.finish(out)?;
                    return Ok(());
                }
            }
        }
    }
}

fn format_embedding_queue_status_line(snapshot: EmbeddingQueueSnapshot, spinner: &str) -> String {
    let mut line = format!(
        "{spinner} Embedding queue · {} remaining · {} running",
        snapshot.remaining(),
        snapshot.running
    );
    if snapshot.failed > 0 {
        line.push_str(&format!(" · {} failed", snapshot.failed));
    }
    line
}

fn format_embedding_queue_progress_bar_line(
    snapshot: EmbeddingQueueSnapshot,
    baseline_total: u64,
    spinner_index: usize,
    terminal_width: Option<usize>,
) -> String {
    let available_width = terminal_width.unwrap_or(80).max(16);
    let done = baseline_total.saturating_sub(snapshot.remaining());
    let summary = if baseline_total > 0 {
        let ratio = (done as f64 / baseline_total as f64).clamp(0.0, 1.0);
        format!(
            " {:>3}% {done}/{}",
            (ratio * 100.0).round() as usize,
            baseline_total
        )
    } else {
        " waiting ".to_string()
    };
    let reserved = summary.chars().count() + 2;
    if available_width <= reserved + 1 {
        return summary.trim().to_string();
    }

    let bar_width = available_width - reserved;
    let bar = if baseline_total > 0 {
        let ratio = (done as f64 / baseline_total as f64).clamp(0.0, 1.0);
        render_init_determinate_progress_bar(bar_width, ratio)
    } else {
        render_init_indeterminate_progress_bar(bar_width, spinner_index)
    };
    format!("[{bar}]{summary}")
}

fn render_init_determinate_progress_bar(width: usize, ratio: f64) -> String {
    let filled = ((width as f64) * ratio).round() as usize;
    let filled = filled.min(width);
    let fill = color_hex_if_enabled(&"█".repeat(filled), BITLOOPS_PURPLE_HEX);
    let empty = "░".repeat(width.saturating_sub(filled));
    format!("{fill}{empty}")
}

fn render_init_indeterminate_progress_bar(width: usize, spinner_index: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let position = spinner_index % width;
    let prefix = "░".repeat(position);
    let pulse = color_hex_if_enabled("█", BITLOOPS_PURPLE_HEX);
    let suffix = "░".repeat(width.saturating_sub(position + 1));
    format!("{prefix}{pulse}{suffix}")
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
