use std::io::{IsTerminal, Write};
use std::time::Duration;

use anyhow::{Result, bail};
use terminal_size::{Width, terminal_size};

use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
};
use crate::devql_transport::SlimCliRepoScope;
use crate::host::runtime_store::RepoSqliteRuntimeStore;
use crate::utils::branding::{BITLOOPS_PURPLE_HEX, color_hex_if_enabled};

use super::QueuedEmbeddingsBootstrapTask;

const INIT_PROGRESS_POLL_INTERVAL: Duration = Duration::from_secs(1);
const INIT_PROGRESS_TICK_INTERVAL: Duration = Duration::from_millis(120);
const INIT_SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SUCCESS_GREEN_HEX: &str = "#22c55e";

pub(super) struct InitProgressOptions {
    pub(super) show_sync: bool,
    pub(super) show_ingest: bool,
    pub(super) enqueue_ingest_after_sync: bool,
    pub(super) ingest_backfill: usize,
    pub(super) queued_embeddings_bootstrap: Option<QueuedEmbeddingsBootstrapTask>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EmbeddingQueueSnapshot {
    pending: u64,
    running: u64,
    failed: u64,
    completed: u64,
}

impl EmbeddingQueueSnapshot {
    fn remaining(self) -> u64 {
        self.pending + self.running
    }
}

async fn current_embedding_queue_snapshot(
    repo_root: &std::path::Path,
) -> Result<Option<EmbeddingQueueSnapshot>> {
    let daemon_status = crate::daemon::status().await?;
    let Some(enrichment) = daemon_status.enrichment else {
        return Ok(None);
    };

    let completed = RepoSqliteRuntimeStore::open(repo_root)
        .ok()
        .and_then(|store| {
            store
                .load_capability_workplane_mailbox_status(
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    [
                        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
                    ],
                )
                .ok()
        })
        .map(|status_by_mailbox| {
            status_by_mailbox
                .into_values()
                .map(|status| status.completed_recent_jobs)
                .sum()
        })
        .unwrap_or_default();

    Ok(Some(EmbeddingQueueSnapshot {
        pending: enrichment.state.pending_embedding_jobs,
        running: enrichment.state.running_embedding_jobs,
        failed: enrichment.state.failed_embedding_jobs,
        completed,
    }))
}

enum BottomProgressState {
    Bootstrap(crate::cli::devql::graphql::TaskGraphqlRecord),
    Queue {
        snapshot: EmbeddingQueueSnapshot,
        baseline_total: u64,
        completed_floor: u64,
    },
    QueueComplete {
        failed_jobs: u64,
        baseline_total: u64,
    },
    BootstrapFailed(crate::cli::devql::graphql::TaskGraphqlRecord),
    Hidden,
}

#[derive(Clone, Copy)]
struct InitChecklistState {
    show_sync: bool,
    show_ingest: bool,
    show_embeddings: bool,
    sync_complete: bool,
    ingest_complete: bool,
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
        checklist: InitChecklistState,
        top_task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
        bottom_state: &BottomProgressState,
    ) -> Result<()> {
        let frame = self.render_frame(checklist, top_task, bottom_state);
        self.write_frame(out, frame, false)
    }

    fn tick(
        &mut self,
        out: &mut dyn Write,
        checklist: InitChecklistState,
        top_task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
        bottom_state: &BottomProgressState,
    ) -> Result<()> {
        if !self.interactive {
            return Ok(());
        }
        self.spinner_index = (self.spinner_index + 1) % INIT_SPINNER_FRAMES.len();
        let frame = self.render_frame(checklist, top_task, bottom_state);
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
        checklist: InitChecklistState,
        top_task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
        bottom_state: &BottomProgressState,
    ) -> String {
        let mut lines = Vec::new();
        let spinner =
            color_hex_if_enabled(INIT_SPINNER_FRAMES[self.spinner_index], BITLOOPS_PURPLE_HEX);
        let tick = color_hex_if_enabled("✓", SUCCESS_GREEN_HEX);
        if checklist.show_sync {
            lines.push(render_init_checklist_item(
                if checklist.sync_complete {
                    InitChecklistItemState::Complete
                } else {
                    InitChecklistItemState::Active
                },
                spinner.as_str(),
                tick.as_str(),
                "Analysing your current branch to know what's what",
                self.terminal_width,
            ));
        }
        if checklist.show_ingest {
            lines.push(render_init_checklist_item(
                if checklist.ingest_complete {
                    InitChecklistItemState::Complete
                } else {
                    InitChecklistItemState::Active
                },
                spinner.as_str(),
                tick.as_str(),
                "Analysing your git history because you know... history is important",
                self.terminal_width,
            ));
        }
        if checklist.show_embeddings {
            lines.push(render_init_checklist_item(
                match bottom_state {
                    BottomProgressState::QueueComplete { failed_jobs, .. } if *failed_jobs == 0 => {
                        InitChecklistItemState::Complete
                    }
                    BottomProgressState::BootstrapFailed(_) => InitChecklistItemState::Failed,
                    _ => InitChecklistItemState::Active,
                },
                spinner.as_str(),
                tick.as_str(),
                "Creating code embeddings for fast search using our local embeddings provider",
                self.terminal_width,
            ));
        }
        lines.push(String::new());

        if let Some(task) = top_task {
            lines.push(fit_init_plain_line(
                init_task_description(task),
                self.terminal_width,
            ));
            lines.push(format_init_task_progress_bar_line(
                task,
                self.spinner_index,
                self.terminal_width,
            ));
            lines.push(format_init_task_status_line(
                task,
                spinner.as_str(),
                self.terminal_width,
            ));
        }
        match bottom_state {
            BottomProgressState::Bootstrap(task) | BottomProgressState::BootstrapFailed(task) => {
                if top_task.is_some() {
                    lines.push(String::new());
                }
                lines.push(fit_init_plain_line(
                    "Creating code embeddings for fast search using our local embeddings provider",
                    self.terminal_width,
                ));
                lines.push(format_init_task_progress_bar_line(
                    task,
                    self.spinner_index,
                    self.terminal_width,
                ));
                lines.push(format_init_task_status_line(
                    task,
                    spinner.as_str(),
                    self.terminal_width,
                ));
            }
            BottomProgressState::Queue {
                snapshot,
                baseline_total,
                completed_floor: _,
            } => {
                if top_task.is_some() {
                    lines.push(String::new());
                }
                lines.push(fit_init_plain_line(
                    "Creating code embeddings for fast search using our local embeddings provider",
                    self.terminal_width,
                ));
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
            BottomProgressState::QueueComplete {
                failed_jobs,
                baseline_total,
            } => {
                if top_task.is_some() {
                    lines.push(String::new());
                }
                lines.push(fit_init_plain_line(
                    "Creating code embeddings for fast search using our local embeddings provider",
                    self.terminal_width,
                ));
                lines.push(format_embedding_queue_complete_progress_bar_line(
                    *baseline_total,
                    self.terminal_width,
                ));
                if *failed_jobs > 0 {
                    lines.push(format!(
                        "✖ Embedding queue finished with {} failed job(s)",
                        format_count_u64(*failed_jobs)
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
                self.rendered_lines =
                    rendered_terminal_line_count(&frame, self.terminal_width).max(1);
                return Ok(());
            }
            write!(out, "{frame}")?;
            out.flush()?;
            self.last_frame = Some(frame.clone());
            self.wrote_in_place = true;
            self.rendered_lines = rendered_terminal_line_count(&frame, self.terminal_width).max(1);
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

fn rendered_terminal_line_count(frame: &str, terminal_width: Option<usize>) -> usize {
    let Some(width) = terminal_width.filter(|width| *width > 0) else {
        return frame.lines().count().max(1);
    };

    frame
        .split('\n')
        .map(|line| {
            let visible_width = visible_terminal_width(line);
            visible_width.max(1).div_ceil(width)
        })
        .sum::<usize>()
        .max(1)
}

fn visible_terminal_width(text: &str) -> usize {
    let mut width = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        width += 1;
    }
    width
}

pub(super) async fn run_dual_init_progress(
    out: &mut dyn Write,
    scope: &SlimCliRepoScope,
    mut top_task: Option<crate::cli::devql::graphql::TaskGraphqlRecord>,
    options: InitProgressOptions,
) -> Result<()> {
    let bootstrap_scope = options
        .queued_embeddings_bootstrap
        .as_ref()
        .map(|task| task.scope.clone());
    let mut enqueue_ingest_after_sync = options.enqueue_ingest_after_sync;
    let mut bottom_state = if let Some(bootstrap) = options.queued_embeddings_bootstrap.as_ref() {
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
    let mut checklist = InitChecklistState {
        show_sync: options.show_sync,
        show_ingest: options.show_ingest,
        show_embeddings: options.queued_embeddings_bootstrap.is_some(),
        sync_complete: false,
        ingest_complete: false,
    };
    let mut renderer = InitProgressRenderer::new();
    writeln!(
        out,
        "{}",
        fit_init_plain_line(
            "Bitloops is currently updating its local database with the following:",
            renderer.terminal_width,
        )
    )?;
    writeln!(out)?;
    out.flush()?;
    renderer.render(out, checklist, top_task.as_ref(), &bottom_state)?;

    let mut poll_interval = tokio::time::interval(INIT_PROGRESS_POLL_INTERVAL);
    let mut render_tick = tokio::time::interval(INIT_PROGRESS_TICK_INTERVAL);
    loop {
        tokio::select! {
            _ = render_tick.tick(), if renderer.is_interactive() => {
                renderer.tick(out, checklist, top_task.as_ref(), &bottom_state)?;
            }
            _ = poll_interval.tick() => {
                if let Some(current) = top_task.clone() {
                    match refresh_init_progress_task(scope, &current).await? {
                        Some(refreshed) if refreshed.is_terminal() => {
                            if refreshed.status.eq_ignore_ascii_case("completed") {
                                if refreshed.is_sync() {
                                    checklist.sync_complete = true;
                                } else if refreshed.is_ingest() {
                                    checklist.ingest_complete = true;
                                }
                                if refreshed.is_sync() && enqueue_ingest_after_sync {
                                    let (ingest_task, _merged) = crate::cli::devql::graphql::enqueue_ingest_task_via_graphql(
                                        scope,
                                        Some(options.ingest_backfill),
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
                        }
                        Some(refreshed) => top_task = Some(refreshed),
                        None if current.is_sync() && enqueue_ingest_after_sync => {
                            checklist.sync_complete = true;
                            let (ingest_task, _merged) = crate::cli::devql::graphql::enqueue_ingest_task_via_graphql(
                                scope,
                                Some(options.ingest_backfill),
                                false,
                            ).await?;
                            top_task = Some(ingest_task);
                            enqueue_ingest_after_sync = false;
                        }
                        None => {
                            if current.is_sync() {
                                checklist.sync_complete = true;
                            } else if current.is_ingest() {
                                checklist.ingest_complete = true;
                            }
                            top_task = None;
                        }
                    }
                }

                bottom_state = match bottom_state {
                    BottomProgressState::Bootstrap(current_task) => {
                        let bootstrap_scope = bootstrap_scope.as_ref().unwrap_or(scope);
                        match refresh_init_progress_task(bootstrap_scope, &current_task).await? {
                            Some(refreshed) if refreshed.is_terminal() => {
                                if refreshed.status.eq_ignore_ascii_case("completed") {
                                    if let Some(snapshot) =
                                        current_embedding_queue_snapshot(&scope.repo_root).await?
                                    {
                                        if snapshot.remaining() > 0 || snapshot.failed > 0 {
                                            let completed_floor = snapshot.completed;
                                            BottomProgressState::Queue {
                                                baseline_total: snapshot.remaining(),
                                                completed_floor,
                                                snapshot: EmbeddingQueueSnapshot {
                                                    completed: 0,
                                                    ..snapshot
                                                },
                                            }
                                        } else {
                                            BottomProgressState::QueueComplete {
                                                failed_jobs: 0,
                                                baseline_total: 0,
                                            }
                                        }
                                    } else {
                                        BottomProgressState::Hidden
                                    }
                                } else {
                                    BottomProgressState::BootstrapFailed(refreshed)
                                }
                            }
                            Some(refreshed) => BottomProgressState::Bootstrap(refreshed),
                            None => {
                                if let Some(snapshot) =
                                    current_embedding_queue_snapshot(&scope.repo_root).await?
                                {
                                    if snapshot.remaining() > 0 || snapshot.failed > 0 {
                                        let completed_floor = snapshot.completed;
                                        BottomProgressState::Queue {
                                            baseline_total: snapshot.remaining(),
                                            completed_floor,
                                            snapshot: EmbeddingQueueSnapshot {
                                                completed: 0,
                                                ..snapshot
                                            },
                                        }
                                    } else {
                                        BottomProgressState::QueueComplete {
                                            failed_jobs: 0,
                                            baseline_total: 0,
                                        }
                                    }
                                } else {
                                    BottomProgressState::Hidden
                                }
                            }
                        }
                    }
                    BottomProgressState::Queue {
                        snapshot: _,
                        baseline_total,
                        completed_floor,
                    } => {
                        if let Some(snapshot) =
                            current_embedding_queue_snapshot(&scope.repo_root).await?
                        {
                            let completed_since_start =
                                snapshot.completed.saturating_sub(completed_floor);
                            let baseline_total =
                                baseline_total.max(completed_since_start + snapshot.remaining());
                            let snapshot = EmbeddingQueueSnapshot {
                                completed: completed_since_start,
                                ..snapshot
                            };
                            if snapshot.remaining() == 0 {
                                BottomProgressState::QueueComplete {
                                    failed_jobs: snapshot.failed,
                                    baseline_total,
                                }
                            } else {
                                BottomProgressState::Queue {
                                    snapshot,
                                    baseline_total,
                                    completed_floor,
                                }
                            }
                        } else {
                            BottomProgressState::Hidden
                        }
                    }
                    other => other,
                };

                renderer.render(out, checklist, top_task.as_ref(), &bottom_state)?;
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

async fn refresh_init_progress_task(
    scope: &SlimCliRepoScope,
    current: &crate::cli::devql::graphql::TaskGraphqlRecord,
) -> Result<Option<crate::cli::devql::graphql::TaskGraphqlRecord>> {
    if let Some(task) =
        crate::cli::devql::graphql::query_task_via_graphql(scope, current.task_id.as_str()).await?
    {
        return Ok(Some(task));
    }

    let kind = if current.is_sync() {
        Some("sync")
    } else if current.is_ingest() {
        Some("ingest")
    } else if current.is_embeddings_bootstrap() {
        Some("embeddings_bootstrap")
    } else {
        None
    };
    let Some(kind) = kind else {
        return Ok(None);
    };

    let tasks =
        crate::cli::devql::graphql::list_tasks_via_graphql(scope, Some(kind), None, Some(16))
            .await?;
    Ok(tasks
        .into_iter()
        .find(|task| task.task_id == current.task_id))
}

#[derive(Clone, Copy)]
enum InitChecklistItemState {
    Active,
    Complete,
    Failed,
}

fn render_init_checklist_item(
    state: InitChecklistItemState,
    spinner: &str,
    tick: &str,
    text: &str,
    terminal_width: Option<usize>,
) -> String {
    let icon = match state {
        InitChecklistItemState::Active => spinner.to_string(),
        InitChecklistItemState::Complete => tick.to_string(),
        InitChecklistItemState::Failed => "✖".to_string(),
    };
    fit_init_plain_line(&format!("{icon} {text}"), terminal_width)
}

fn init_task_description(task: &crate::cli::devql::graphql::TaskGraphqlRecord) -> &'static str {
    if task.is_sync() {
        "Analysing your current branch to know what's what"
    } else if task.is_ingest() {
        "Analysing your git history because you know... history is important"
    } else if task.is_embeddings_bootstrap() {
        "Creating code embeddings for fast search using our local embeddings provider"
    } else {
        "Working"
    }
}

fn format_init_task_status_line(
    task: &crate::cli::devql::graphql::TaskGraphqlRecord,
    spinner: &str,
    terminal_width: Option<usize>,
) -> String {
    let status = if task.is_sync() {
        format_init_sync_status(task)
    } else if task.is_ingest() {
        format_init_ingest_status(task)
    } else if task.is_embeddings_bootstrap() {
        format_init_bootstrap_status(task)
    } else {
        format!("Working on {}", task.repo_name)
    };
    let fitted = fit_init_status_text(
        status.as_str(),
        terminal_width.map(|width| width.saturating_sub(2)),
    );
    format!("{spinner} {fitted}")
}

fn format_init_sync_status(task: &crate::cli::devql::graphql::TaskGraphqlRecord) -> String {
    let progress = task.sync_progress.as_ref();
    let mut line = format!(
        "Syncing {} · {}",
        task.repo_name,
        progress
            .map(|progress| humanise_init_sync_phase(progress.phase.as_str()))
            .unwrap_or("working"),
    );
    if let Some(progress) = progress {
        if progress.paths_total > 0 {
            line.push_str(&format!(
                " · {}/{} paths",
                format_count_i32(progress.paths_completed),
                format_count_i32(progress.paths_total),
            ));
        }
        if let Some(path) = progress.current_path.as_ref() {
            line.push_str(&format!(" · {path}"));
        }
    }
    line
}

fn format_init_ingest_status(task: &crate::cli::devql::graphql::TaskGraphqlRecord) -> String {
    let progress = task.ingest_progress.as_ref();
    let mut line = format!(
        "Ingesting {} · {}",
        task.repo_name,
        progress
            .map(|progress| humanise_init_ingest_phase(progress.phase.as_str()))
            .unwrap_or("working"),
    );
    if let Some(progress) = progress {
        if progress.commits_total > 0 {
            line.push_str(&format!(
                " · {}/{} commits",
                format_count_i32(progress.commits_processed),
                format_count_i32(progress.commits_total),
            ));
        }
        if let Some(commit_sha) = progress.current_commit_sha.as_ref() {
            line.push_str(&format!(" · {commit_sha}"));
        }
    }
    line
}

fn format_init_bootstrap_status(task: &crate::cli::devql::graphql::TaskGraphqlRecord) -> String {
    let progress = task.embeddings_bootstrap_progress.as_ref();
    let mut line = format!(
        "Bootstrapping embeddings for {} · {}",
        task.repo_name,
        progress
            .map(|progress| humanise_init_bootstrap_phase(progress.phase.as_str()))
            .unwrap_or("working"),
    );
    if let Some(progress) = progress {
        if let Some(total) = progress.bytes_total
            && total > 0
            && progress.bytes_downloaded > 0
        {
            line.push_str(&format!(
                " · {}/{} MB",
                format_megabytes(progress.bytes_downloaded),
                format_megabytes(total),
            ));
        }
        if let Some(asset_name) = progress.asset_name.as_ref() {
            line.push_str(&format!(" · {asset_name}"));
        } else if let Some(message) = progress.message.as_ref() {
            line.push_str(&format!(" · {message}"));
        }
    }
    line
}

fn format_init_task_progress_bar_line(
    task: &crate::cli::devql::graphql::TaskGraphqlRecord,
    spinner_index: usize,
    terminal_width: Option<usize>,
) -> String {
    let available_width = terminal_width.unwrap_or(80).max(16);
    let summary = if let Some((ratio, done, total, unit)) = init_task_progress_ratio(task) {
        format!(
            " {:>3}% {done}/{total} {unit}",
            (ratio * 100.0).round() as usize,
        )
    } else {
        format!(" {} ", init_task_progress_phase_summary(task))
    };
    let reserved = summary.chars().count() + 2;
    if available_width <= reserved + 1 {
        return fit_init_status_text(summary.trim(), Some(available_width));
    }

    let bar_width = available_width - reserved;
    let bar = if let Some((ratio, _, _, _)) = init_task_progress_ratio(task) {
        render_init_determinate_progress_bar(bar_width, ratio)
    } else {
        render_init_indeterminate_progress_bar(bar_width, spinner_index)
    };
    format!("[{bar}]{summary}")
}

fn init_task_progress_ratio(
    task: &crate::cli::devql::graphql::TaskGraphqlRecord,
) -> Option<(f64, String, String, &'static str)> {
    if task.is_sync() {
        return task.sync_progress.as_ref().and_then(|progress| {
            if progress.paths_total > 0 {
                Some((
                    (progress.paths_completed as f64 / progress.paths_total as f64).clamp(0.0, 1.0),
                    format_count_i32(progress.paths_completed),
                    format_count_i32(progress.paths_total),
                    "paths",
                ))
            } else {
                None
            }
        });
    }
    if task.is_ingest() {
        return task.ingest_progress.as_ref().and_then(|progress| {
            if progress.commits_total > 0 {
                Some((
                    (progress.commits_processed as f64 / progress.commits_total as f64)
                        .clamp(0.0, 1.0),
                    format_count_i32(progress.commits_processed),
                    format_count_i32(progress.commits_total),
                    "commits",
                ))
            } else {
                None
            }
        });
    }
    if task.is_embeddings_bootstrap() {
        return task
            .embeddings_bootstrap_progress
            .as_ref()
            .and_then(|progress| {
                progress.bytes_total.and_then(|total| {
                    (total > 0).then_some((
                        (progress.bytes_downloaded as f64 / total as f64).clamp(0.0, 1.0),
                        format_megabytes(progress.bytes_downloaded),
                        format_megabytes(total),
                        "MB",
                    ))
                })
            });
    }
    None
}

fn init_task_progress_phase_summary(
    task: &crate::cli::devql::graphql::TaskGraphqlRecord,
) -> &'static str {
    if task.is_sync() {
        task.sync_progress
            .as_ref()
            .map(|progress| humanise_init_sync_phase(progress.phase.as_str()))
            .unwrap_or("working")
    } else if task.is_ingest() {
        task.ingest_progress
            .as_ref()
            .map(|progress| humanise_init_ingest_phase(progress.phase.as_str()))
            .unwrap_or("working")
    } else if task.is_embeddings_bootstrap() {
        task.embeddings_bootstrap_progress
            .as_ref()
            .map(|progress| humanise_init_bootstrap_phase(progress.phase.as_str()))
            .unwrap_or("working")
    } else {
        "working"
    }
}

fn format_embedding_queue_status_line(snapshot: EmbeddingQueueSnapshot, spinner: &str) -> String {
    let mut line = format!(
        "{spinner} Embedding queue · {} remaining · {} running",
        format_count_u64(snapshot.remaining()),
        format_count_u64(snapshot.running),
    );
    if snapshot.failed > 0 {
        line.push_str(&format!(" · {} failed", format_count_u64(snapshot.failed)));
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
    let done = snapshot.completed.min(baseline_total);
    let summary = if baseline_total > 0 {
        let ratio = (done as f64 / baseline_total as f64).clamp(0.0, 1.0);
        format!(
            " {:>3}% {}/{} artefacts",
            (ratio * 100.0).round() as usize,
            format_count_u64(done),
            format_count_u64(baseline_total),
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

fn format_embedding_queue_complete_progress_bar_line(
    baseline_total: u64,
    terminal_width: Option<usize>,
) -> String {
    let available_width = terminal_width.unwrap_or(80).max(16);
    let summary = format!(
        " 100% {}/{} artefacts",
        format_count_u64(baseline_total),
        format_count_u64(baseline_total),
    );
    let reserved = summary.chars().count() + 2;
    if available_width <= reserved + 1 {
        return fit_init_status_text(summary.trim(), Some(available_width));
    }

    let bar_width = available_width - reserved;
    let bar = render_init_determinate_progress_bar(bar_width, 1.0);
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

fn format_count_i32(value: i32) -> String {
    format_count_u64(u64::try_from(value).unwrap_or_default())
}

fn format_count_u64(value: u64) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(ch);
    }
    formatted
}

fn format_megabytes(bytes: i64) -> String {
    let megabytes = bytes as f64 / (1024.0 * 1024.0);
    if megabytes >= 10.0 {
        format!("{megabytes:.0}")
    } else {
        format!("{megabytes:.1}")
    }
}

fn fit_init_plain_line(text: &str, terminal_width: Option<usize>) -> String {
    fit_init_status_text(text, terminal_width)
}

fn fit_init_status_text(text: &str, available_width: Option<usize>) -> String {
    let Some(max_width) = available_width else {
        return text.to_string();
    };
    if max_width == 0 {
        return String::new();
    }
    if text.chars().count() <= max_width {
        return text.to_string();
    }
    elide_init_middle(text, max_width)
}

fn elide_init_middle(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }
    if text.chars().count() <= max_width {
        return text.to_string();
    }
    let prefix_len = (max_width - 1) / 2;
    let suffix_len = max_width - 1 - prefix_len;
    let prefix = text.chars().take(prefix_len).collect::<String>();
    let suffix = text
        .chars()
        .rev()
        .take(suffix_len)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{prefix}…{suffix}")
}

fn humanise_init_sync_phase(phase: &str) -> &'static str {
    match phase {
        "queued" => "waiting in queue",
        "ensuring_schema" => "preparing schema",
        "inspecting_workspace" => "inspecting workspace",
        "building_manifest" => "building manifest",
        "loading_stored_state" => "loading stored state",
        "classifying_paths" => "classifying paths",
        "removing_paths" => "removing stale paths",
        "extracting_paths" => "extracting artefacts",
        "materialising_paths" => "materialising artefacts",
        "running_gc" => "cleaning caches",
        "complete" => "complete",
        "failed" => "failed",
        _ => "working",
    }
}

fn humanise_init_ingest_phase(phase: &str) -> &'static str {
    match phase.to_ascii_lowercase().as_str() {
        "initializing" => "initialising",
        "extracting" => "extracting checkpoints",
        "persisting" => "persisting state",
        "complete" => "complete",
        "failed" => "failed",
        _ => "working",
    }
}

fn humanise_init_bootstrap_phase(phase: &str) -> &'static str {
    match phase {
        "queued" => "waiting in queue",
        "preparing_config" => "preparing config",
        "resolving_release" => "resolving release",
        "downloading_runtime" => "downloading runtime",
        "extracting_runtime" => "extracting runtime",
        "rewriting_runtime" => "updating runtime config",
        "warming_profile" => "warming profile cache",
        "complete" => "complete",
        "failed" => "failed",
        _ => "working",
    }
}
