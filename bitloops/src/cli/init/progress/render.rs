use std::io::{IsTerminal, Write};

use anyhow::Result;
use terminal_size::{Width, terminal_size};

use crate::utils::branding::{BITLOOPS_PURPLE_HEX, color_hex_if_enabled};

use super::{
    BottomProgressState, EmbeddingQueueSnapshot, INIT_SPINNER_FRAMES, InitChecklistState,
    SUCCESS_GREEN_HEX, SummaryProgressState,
};

pub(super) struct InitProgressRenderer {
    interactive: bool,
    terminal_width: Option<usize>,
    spinner_index: usize,
    last_frame: Option<String>,
    wrote_in_place: bool,
    rendered_lines: usize,
}

impl InitProgressRenderer {
    pub(super) fn new() -> Self {
        Self {
            interactive: std::io::stdout().is_terminal() && std::env::var("ACCESSIBLE").is_err(),
            terminal_width: terminal_size().map(|(Width(width), _)| width as usize),
            spinner_index: 0,
            last_frame: None,
            wrote_in_place: false,
            rendered_lines: 0,
        }
    }

    pub(super) fn terminal_width(&self) -> Option<usize> {
        self.terminal_width
    }

    pub(super) fn is_interactive(&self) -> bool {
        self.interactive
    }

    pub(super) fn render(
        &mut self,
        out: &mut dyn Write,
        checklist: InitChecklistState,
        top_task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
        bottom_state: &BottomProgressState,
        summary_state: &SummaryProgressState,
    ) -> Result<()> {
        let frame = self.render_frame(checklist, top_task, bottom_state, summary_state);
        self.write_frame(out, frame, false)
    }

    pub(super) fn tick(
        &mut self,
        out: &mut dyn Write,
        checklist: InitChecklistState,
        top_task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
        bottom_state: &BottomProgressState,
        summary_state: &SummaryProgressState,
    ) -> Result<()> {
        if !self.interactive {
            return Ok(());
        }
        self.spinner_index = (self.spinner_index + 1) % INIT_SPINNER_FRAMES.len();
        let frame = self.render_frame(checklist, top_task, bottom_state, summary_state);
        self.write_frame(out, frame, true)
    }

    pub(super) fn finish(&mut self, out: &mut dyn Write) -> Result<()> {
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
        summary_state: &SummaryProgressState,
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
                    BottomProgressState::QueueComplete { .. }
                    | BottomProgressState::BootstrapFailed(_) => InitChecklistItemState::Failed,
                    _ => InitChecklistItemState::Active,
                },
                spinner.as_str(),
                tick.as_str(),
                "Creating code embeddings for fast search using our local embeddings provider",
                self.terminal_width,
            ));
        }
        if checklist.show_summaries {
            lines.push(render_init_checklist_item(
                match summary_state {
                    SummaryProgressState::Complete { failed_jobs, .. } if *failed_jobs == 0 => {
                        InitChecklistItemState::Complete
                    }
                    SummaryProgressState::Complete { .. } | SummaryProgressState::Failed { .. } => {
                        InitChecklistItemState::Failed
                    }
                    _ => InitChecklistItemState::Active,
                },
                spinner.as_str(),
                tick.as_str(),
                "Configuring local semantic summaries with bitloops-inference",
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
            BottomProgressState::WaitingForQueue {
                baseline_total: _,
                completed_floor: _,
                completed_jobs,
                failed_jobs,
            } => {
                if top_task.is_some() {
                    lines.push(String::new());
                }
                lines.push(fit_init_plain_line(
                    "Creating code embeddings for fast search using our local embeddings provider",
                    self.terminal_width,
                ));
                lines.push(format_queue_waiting_progress_bar_line(
                    checklist,
                    self.spinner_index,
                    self.terminal_width,
                ));
                lines.push(format_embedding_waiting_status_line(
                    checklist,
                    *completed_jobs,
                    *failed_jobs,
                    spinner.as_str(),
                    self.terminal_width,
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
        if !matches!(summary_state, SummaryProgressState::Hidden) {
            if top_task.is_some() || !matches!(bottom_state, BottomProgressState::Hidden) {
                lines.push(String::new());
            }
            lines.push(fit_init_plain_line(
                "Configuring local semantic summaries with bitloops-inference",
                self.terminal_width,
            ));
            lines.push(format_summary_progress_bar_line(
                summary_state,
                self.spinner_index,
                self.terminal_width,
            ));
            lines.push(format_summary_status_line(
                summary_state,
                checklist,
                spinner.as_str(),
                tick.as_str(),
                self.terminal_width,
            ));
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

fn format_queue_waiting_progress_bar_line(
    checklist: InitChecklistState,
    spinner_index: usize,
    terminal_width: Option<usize>,
) -> String {
    let available_width = terminal_width.unwrap_or(80).max(16);
    let summary = match remaining_init_dependencies(checklist) {
        Some(dependencies) => format!(" waiting for {dependencies} "),
        None => " waiting for queued work ".to_string(),
    };
    let reserved = summary.chars().count() + 2;
    if available_width <= reserved + 1 {
        return fit_init_status_text(summary.trim(), Some(available_width));
    }

    let bar_width = available_width - reserved;
    let bar = render_init_indeterminate_progress_bar(bar_width, spinner_index);
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

fn format_embedding_waiting_status_line(
    checklist: InitChecklistState,
    completed_jobs: u64,
    failed_jobs: u64,
    spinner: &str,
    terminal_width: Option<usize>,
) -> String {
    let prefix = if completed_jobs > 0 {
        format!(
            "bitloops-embeddings processed {} artefacts",
            format_count_u64(completed_jobs)
        )
    } else {
        "bitloops-embeddings is ready".to_string()
    };
    let waiting = match remaining_init_dependencies(checklist) {
        Some(dependencies) => format!("waiting for {dependencies} to queue embeddings"),
        None => "waiting for queued embedding work to appear".to_string(),
    };
    let mut line = format!("{prefix} · {waiting}");
    if failed_jobs > 0 {
        line.push_str(&format!(" · {} failed", format_count_u64(failed_jobs)));
    }
    let fitted = fit_init_status_text(
        line.as_str(),
        terminal_width.map(|width| width.saturating_sub(2)),
    );
    format!("{spinner} {fitted}")
}

fn format_summary_progress_bar_line(
    state: &SummaryProgressState,
    spinner_index: usize,
    terminal_width: Option<usize>,
) -> String {
    let available_width = terminal_width.unwrap_or(80).max(16);
    let summary = match state {
        SummaryProgressState::Queue {
            snapshot,
            baseline_total,
            ..
        } => {
            let done = snapshot.completed.min(*baseline_total);
            if *baseline_total > 0 {
                let ratio = (done as f64 / *baseline_total as f64).clamp(0.0, 1.0);
                format!(
                    " {:>3}% {}/{} summaries",
                    (ratio * 100.0).round() as usize,
                    format_count_u64(done),
                    format_count_u64(*baseline_total),
                )
            } else {
                " waiting ".to_string()
            }
        }
        SummaryProgressState::WaitingForQueue { .. } => " waiting ".to_string(),
        SummaryProgressState::Complete { .. } => " 100% complete ".to_string(),
        SummaryProgressState::Failed { .. } => " failed ".to_string(),
        SummaryProgressState::Running(progress) => {
            if let Some((ratio, done, total, unit)) = summary_progress_ratio(progress) {
                format!(
                    " {:>3}% {done}/{total} {unit}",
                    (ratio * 100.0).round() as usize,
                )
            } else {
                format!(" {} ", humanise_summary_setup_phase(progress.phase))
            }
        }
        SummaryProgressState::Hidden => String::new(),
    };
    let reserved = summary.chars().count() + 2;
    if available_width <= reserved + 1 {
        return fit_init_status_text(summary.trim(), Some(available_width));
    }

    let bar_width = available_width - reserved;
    let bar = match state {
        SummaryProgressState::Queue {
            snapshot,
            baseline_total,
            ..
        } => {
            if *baseline_total > 0 {
                let done = snapshot.completed.min(*baseline_total);
                let ratio = (done as f64 / *baseline_total as f64).clamp(0.0, 1.0);
                render_init_determinate_progress_bar(bar_width, ratio)
            } else {
                render_init_indeterminate_progress_bar(bar_width, spinner_index)
            }
        }
        SummaryProgressState::WaitingForQueue { .. } => {
            render_init_indeterminate_progress_bar(bar_width, spinner_index)
        }
        SummaryProgressState::Complete { .. } => {
            render_init_determinate_progress_bar(bar_width, 1.0)
        }
        SummaryProgressState::Running(progress) => {
            if let Some((ratio, _, _, _)) = summary_progress_ratio(progress) {
                render_init_determinate_progress_bar(bar_width, ratio)
            } else {
                render_init_indeterminate_progress_bar(bar_width, spinner_index)
            }
        }
        SummaryProgressState::Failed { .. } => {
            render_init_indeterminate_progress_bar(bar_width, spinner_index)
        }
        SummaryProgressState::Hidden => String::new(),
    };
    format!("[{bar}]{summary}")
}

fn summary_progress_ratio(
    progress: &crate::cli::inference::SummarySetupProgress,
) -> Option<(f64, String, String, &'static str)> {
    progress.bytes_total.and_then(|total| {
        (total > 0).then_some((
            (progress.bytes_downloaded as f64 / total as f64).clamp(0.0, 1.0),
            format_megabytes(progress.bytes_downloaded as i64),
            format_megabytes(total as i64),
            "MB",
        ))
    })
}

fn format_summary_status_line(
    state: &SummaryProgressState,
    checklist: InitChecklistState,
    spinner: &str,
    tick: &str,
    terminal_width: Option<usize>,
) -> String {
    match state {
        SummaryProgressState::Running(progress) => {
            let mut line = format!(
                "Semantic summaries · {}",
                humanise_summary_setup_phase(progress.phase)
            );
            if let Some(total) = progress.bytes_total
                && total > 0
                && progress.bytes_downloaded > 0
            {
                line.push_str(&format!(
                    " · {}/{} MB",
                    format_megabytes(progress.bytes_downloaded as i64),
                    format_megabytes(total as i64),
                ));
            }
            if let Some(asset_name) = progress.asset_name.as_ref() {
                line.push_str(&format!(" · {asset_name}"));
            } else if let Some(message) = progress.message.as_ref() {
                line.push_str(&format!(" · {message}"));
            }
            let fitted = fit_init_status_text(
                line.as_str(),
                terminal_width.map(|width| width.saturating_sub(2)),
            );
            format!("{spinner} {fitted}")
        }
        SummaryProgressState::Queue { snapshot, .. } => {
            let mut line = format!(
                "Semantic summary queue · {} remaining · {} running",
                format_count_u64(snapshot.remaining()),
                format_count_u64(snapshot.running),
            );
            if snapshot.failed > 0 {
                line.push_str(&format!(" · {} failed", format_count_u64(snapshot.failed)));
            }
            let fitted = fit_init_status_text(
                line.as_str(),
                terminal_width.map(|width| width.saturating_sub(2)),
            );
            format!("{spinner} {fitted}")
        }
        SummaryProgressState::WaitingForQueue {
            completed_jobs,
            failed_jobs,
            ..
        } => {
            let prefix = if *completed_jobs > 0 {
                format!(
                    "bitloops-inference processed {} summaries",
                    format_count_u64(*completed_jobs)
                )
            } else {
                "bitloops-inference is ready".to_string()
            };
            let waiting = match remaining_init_dependencies(checklist) {
                Some(dependencies) => {
                    format!("waiting for {dependencies} to queue semantic summaries")
                }
                None => "waiting for queued semantic summaries to appear".to_string(),
            };
            let mut line = format!("{prefix} · {waiting}");
            if *failed_jobs > 0 {
                line.push_str(&format!(" · {} failed", format_count_u64(*failed_jobs)));
            }
            let fitted = fit_init_status_text(
                line.as_str(),
                terminal_width.map(|width| width.saturating_sub(2)),
            );
            format!("{spinner} {fitted}")
        }
        SummaryProgressState::Complete {
            result,
            failed_jobs,
            baseline_total,
        } => {
            let line = if *failed_jobs > 0 {
                format!(
                    "Semantic summary queue finished with {} failed job(s)",
                    format_count_u64(*failed_jobs)
                )
            } else if *baseline_total > 0 {
                "Semantic summary queue complete".to_string()
            } else {
                result.message.clone()
            };
            let fitted = fit_init_status_text(
                line.as_str(),
                terminal_width.map(|width| width.saturating_sub(2)),
            );
            if *failed_jobs > 0 {
                format!("✖ {fitted}")
            } else {
                format!("{tick} {fitted}")
            }
        }
        SummaryProgressState::Failed { error, .. } => {
            let fitted = fit_init_status_text(
                format!("Semantic summary setup failed · {error}").as_str(),
                terminal_width.map(|width| width.saturating_sub(2)),
            );
            format!("✖ {fitted}")
        }
        SummaryProgressState::Hidden => String::new(),
    }
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

fn remaining_init_dependencies(checklist: InitChecklistState) -> Option<&'static str> {
    match (
        checklist.show_sync && !checklist.sync_complete,
        checklist.show_ingest && !checklist.ingest_complete,
    ) {
        (true, true) => Some("sync and ingest"),
        (true, false) => Some("sync"),
        (false, true) => Some("ingest"),
        (false, false) => None,
    }
}

pub(super) fn fit_init_plain_line(text: &str, terminal_width: Option<usize>) -> String {
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

fn humanise_summary_setup_phase(phase: crate::cli::inference::SummarySetupPhase) -> &'static str {
    match phase {
        crate::cli::inference::SummarySetupPhase::Queued => "queued",
        crate::cli::inference::SummarySetupPhase::ResolvingRelease => "resolving release",
        crate::cli::inference::SummarySetupPhase::DownloadingRuntime => "downloading runtime",
        crate::cli::inference::SummarySetupPhase::ExtractingRuntime => "extracting runtime",
        crate::cli::inference::SummarySetupPhase::RewritingRuntime => "updating runtime config",
        crate::cli::inference::SummarySetupPhase::WritingProfile => "writing summary profile",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_waiting_lanes_do_not_report_completion_early() {
        let renderer = InitProgressRenderer {
            interactive: false,
            terminal_width: None,
            spinner_index: 0,
            last_frame: None,
            wrote_in_place: false,
            rendered_lines: 0,
        };
        let frame = renderer.render_frame(
            InitChecklistState {
                show_sync: true,
                show_ingest: true,
                show_embeddings: true,
                show_summaries: true,
                sync_complete: false,
                ingest_complete: false,
            },
            None,
            &BottomProgressState::WaitingForQueue {
                baseline_total: 0,
                completed_floor: 0,
                completed_jobs: 0,
                failed_jobs: 0,
            },
            &SummaryProgressState::WaitingForQueue {
                result: crate::cli::inference::SummarySetupExecutionResult {
                    outcome: crate::cli::inference::SummarySetupOutcome::Configured {
                        model_name: "ministral-3:3b".to_string(),
                    },
                    message: "Configured semantic summaries to use Ollama model `ministral-3:3b`."
                        .to_string(),
                },
                baseline_total: 0,
                completed_floor: 0,
                completed_jobs: 0,
                failed_jobs: 0,
            },
        );

        assert!(frame.contains("waiting for sync and ingest"));
        assert!(!frame.contains("Embedding queue complete"));
        assert!(!frame.contains("Semantic summary queue complete"));
    }
}
