use std::io::{IsTerminal, Write};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use terminal_size::{Width, terminal_size};

use crate::devql_transport::SlimCliRepoScope;
use crate::runtime_presentation::{
    INIT_CODEBASE_LANE_LABEL, INIT_CODEBASE_SECTION_TITLE, INIT_EMBEDDINGS_LANE_LABEL,
    INIT_EMBEDDINGS_SECTION_TITLE, INIT_SUMMARIES_LANE_LABEL, INIT_SUMMARIES_SECTION_TITLE,
    RETRY_FAILED_ENRICHMENTS_COMMAND, embeddings_bootstrap_phase_label, ingest_phase_label,
    queue_state_summary, session_status_label, summary_bootstrap_phase_label, sync_phase_label,
    task_kind_label, waiting_reason_label,
};
use crate::utils::branding::{BITLOOPS_PURPLE_HEX, color_hex_if_enabled};

const INIT_PROGRESS_POLL_INTERVAL: Duration = Duration::from_secs(1);
const INIT_SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SUCCESS_GREEN_HEX: &str = "#22c55e";

pub(super) struct InitProgressOptions {
    pub(super) start_input: crate::cli::devql::graphql::RuntimeStartInitInput,
}

pub(super) async fn run_dual_init_progress(
    out: &mut dyn Write,
    scope: &SlimCliRepoScope,
    options: InitProgressOptions,
) -> Result<()> {
    let start =
        crate::cli::devql::graphql::start_init_via_runtime_graphql(scope, &options.start_input)
            .await?;
    let repo_id = options.start_input.repo_id.clone();
    let session_id = start.init_session_id;
    let mut renderer = RuntimeInitRenderer::new();
    let mut polling_only = false;

    writeln!(
        out,
        "{}",
        fit_line(
            "This may take a few minutes depending on your codebase size.",
            renderer.terminal_width,
        )
    )?;
    writeln!(out)?;
    out.flush()?;

    loop {
        let snapshot = crate::cli::devql::graphql::runtime_snapshot_via_graphql(scope, &repo_id)
            .await
            .with_context(|| format!("loading runtime snapshot for repo `{repo_id}`"))?;
        renderer.render(out, &snapshot, session_id.as_str())?;

        if let Some(session) = snapshot.current_init_session.as_ref()
            && session.init_session_id == session_id
        {
            match session.status.to_ascii_lowercase().as_str() {
                "completed" | "completed_with_warnings" => {
                    renderer.finish(out)?;
                    return Ok(());
                }
                "failed" => {
                    renderer.finish(out)?;
                    bail!(
                        "{}",
                        session
                            .terminal_error
                            .clone()
                            .unwrap_or_else(|| "init session failed".to_string())
                    );
                }
                _ => {}
            }
        }

        renderer.advance_spinner();
        if polling_only {
            tokio::time::sleep(INIT_PROGRESS_POLL_INTERVAL).await;
            continue;
        }

        match tokio::time::timeout(
            INIT_PROGRESS_POLL_INTERVAL,
            crate::cli::devql::graphql::next_runtime_event_via_subscription(
                scope,
                repo_id.as_str(),
                Some(session_id.as_str()),
            ),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(err)) => {
                log::debug!("runtime subscription unavailable; falling back to polling: {err:#}");
                polling_only = true;
            }
            Err(_) => {}
        }
    }
}

struct RuntimeInitRenderer {
    interactive: bool,
    terminal_width: Option<usize>,
    spinner_index: usize,
    last_frame: Option<String>,
    wrote_in_place: bool,
    rendered_lines: usize,
}

impl RuntimeInitRenderer {
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

    fn advance_spinner(&mut self) {
        self.spinner_index = (self.spinner_index + 1) % INIT_SPINNER_FRAMES.len();
    }

    fn render(
        &mut self,
        out: &mut dyn Write,
        snapshot: &crate::cli::devql::graphql::RuntimeSnapshotGraphqlRecord,
        session_id: &str,
    ) -> Result<()> {
        let frame = self.frame(snapshot, session_id);
        if self.last_frame.as_ref() == Some(&frame) {
            return Ok(());
        }

        if self.interactive && self.wrote_in_place {
            clear_rendered_lines(out, self.rendered_lines)?;
        }
        write!(out, "{frame}")?;
        out.flush()?;

        self.rendered_lines = rendered_terminal_line_count(frame.as_str(), self.terminal_width);
        self.last_frame = Some(frame);
        self.wrote_in_place = self.interactive;
        Ok(())
    }

    fn finish(&mut self, out: &mut dyn Write) -> Result<()> {
        if self.interactive && self.wrote_in_place {
            writeln!(out)?;
            out.flush()?;
            self.wrote_in_place = false;
        }
        Ok(())
    }

    fn frame(
        &self,
        snapshot: &crate::cli::devql::graphql::RuntimeSnapshotGraphqlRecord,
        session_id: &str,
    ) -> String {
        let session = snapshot
            .current_init_session
            .as_ref()
            .filter(|session| session.init_session_id == session_id);
        let spinner =
            color_hex_if_enabled(INIT_SPINNER_FRAMES[self.spinner_index], BITLOOPS_PURPLE_HEX);
        let tick = color_hex_if_enabled("✓", SUCCESS_GREEN_HEX);
        let render_context = LaneRenderContext {
            spinner: spinner.as_str(),
            tick: tick.as_str(),
            spinner_index: self.spinner_index,
            terminal_width: self.terminal_width,
        };
        let mut lines = Vec::new();

        let Some(session) = session else {
            lines.push(format!("{spinner} Waiting for init session state"));
            return lines.join("\n");
        };

        let selected_titles = compact_selected_section_titles(session);
        let label_width = selected_titles
            .iter()
            .map(|title| title.chars().count())
            .max()
            .unwrap_or(0)
            + 12;

        lines.extend(render_compact_lane(
            INIT_CODEBASE_SECTION_TITLE,
            &session.top_pipeline_lane,
            INIT_CODEBASE_LANE_LABEL,
            task_for_lane(snapshot, &session.top_pipeline_lane),
            None,
            label_width,
            &render_context,
        ));

        if session.embeddings_selected {
            lines.push(String::new());
            let bootstrap_task = session
                .embeddings_bootstrap_task_id
                .as_deref()
                .and_then(|task_id| task_by_id(snapshot, task_id));
            lines.extend(render_compact_lane(
                INIT_EMBEDDINGS_SECTION_TITLE,
                &session.embeddings_lane,
                INIT_EMBEDDINGS_LANE_LABEL,
                bootstrap_task,
                None,
                label_width,
                &render_context,
            ));
        }

        if session.summaries_selected {
            lines.push(String::new());
            let summary_run = snapshot
                .summaries_bootstrap
                .as_ref()
                .and_then(|run| (run.init_session_id == session.init_session_id).then_some(run));
            lines.extend(render_compact_lane(
                INIT_SUMMARIES_SECTION_TITLE,
                &session.summaries_lane,
                INIT_SUMMARIES_LANE_LABEL,
                None,
                summary_run,
                label_width,
                &render_context,
            ));
        }

        lines.push(String::new());
        lines.push(fit_line("Session Status", self.terminal_width));
        lines.push(format!(
            " {} {}",
            if matches!(
                session.status.to_ascii_lowercase().as_str(),
                "completed" | "completed_with_warnings"
            ) {
                tick.as_str()
            } else {
                spinner.as_str()
            },
            fit_line(
                &compact_session_status_line(snapshot, session),
                self.terminal_width.map(|width| width.saturating_sub(4)),
            )
        ));

        lines.join("\n")
    }
}

fn compact_selected_section_titles(
    session: &crate::cli::devql::graphql::RuntimeInitSessionGraphqlRecord,
) -> Vec<&'static str> {
    let mut titles = vec![INIT_CODEBASE_SECTION_TITLE];
    if session.embeddings_selected {
        titles.push(INIT_EMBEDDINGS_SECTION_TITLE);
    }
    if session.summaries_selected {
        titles.push(INIT_SUMMARIES_SECTION_TITLE);
    }
    titles
}

struct LaneRenderContext<'a> {
    spinner: &'a str,
    tick: &'a str,
    spinner_index: usize,
    terminal_width: Option<usize>,
}

fn render_compact_lane(
    title: &str,
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
    activity_label: &str,
    task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
    summary_run: Option<&crate::cli::devql::graphql::RuntimeSummaryBootstrapRunGraphqlRecord>,
    label_width: usize,
    render_context: &LaneRenderContext<'_>,
) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(compact_lane_heading(
        title,
        lane,
        task,
        summary_run,
        label_width,
        render_context,
    ));

    let mut status_parts = vec![activity_label.to_string()];
    if let Some(queue) = compact_queue_status_text(lane) {
        status_parts.push(queue);
    }
    if let Some(detail) = compact_lane_detail(title, lane) {
        status_parts.push(detail);
    }

    lines.push(format!(
        " {} {}",
        lane_status_icon(
            lane.status.as_str(),
            render_context.spinner,
            render_context.tick
        ),
        fit_line(
            &status_parts.join(" | "),
            render_context
                .terminal_width
                .map(|width| width.saturating_sub(2)),
        )
    ));
    lines
}

fn compact_lane_heading(
    title: &str,
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
    task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
    summary_run: Option<&crate::cli::devql::graphql::RuntimeSummaryBootstrapRunGraphqlRecord>,
    label_width: usize,
    render_context: &LaneRenderContext<'_>,
) -> String {
    let available_width = render_context.terminal_width.unwrap_or(80).max(24);
    let percent = compact_lane_percent(title, lane, task, summary_run)
        .map(|value| format!(" {:>3}%", value))
        .unwrap_or_else(|| "     ".to_string());
    let reserved = label_width + percent.chars().count() + 2;
    let bar_width = available_width.saturating_sub(reserved).max(8);
    let bar = if let Some(ratio) = compact_lane_ratio(title, lane, task, summary_run) {
        render_determinate_progress_bar(bar_width, ratio)
    } else {
        render_indeterminate_progress_bar(bar_width, render_context.spinner_index)
    };
    format!("{title:<label_width$}[{bar}]{percent}")
}

fn compact_lane_ratio(
    title: &str,
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
    task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
    summary_run: Option<&crate::cli::devql::graphql::RuntimeSummaryBootstrapRunGraphqlRecord>,
) -> Option<f64> {
    if let Some(task) = task.filter(|task| is_active_runtime_status(task.status.as_str())) {
        return task_progress(task).0;
    }
    if let Some(run) = summary_run.filter(|run| is_active_runtime_status(run.status.as_str())) {
        return summary_progress(run).0;
    }
    lane_progress(title, lane).0
}

fn compact_lane_percent(
    title: &str,
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
    task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
    summary_run: Option<&crate::cli::devql::graphql::RuntimeSummaryBootstrapRunGraphqlRecord>,
) -> Option<usize> {
    compact_lane_ratio(title, lane, task, summary_run)
        .map(|ratio| ((ratio * 100.0).round() as usize).min(100))
}

fn compact_queue_status_text(
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
) -> Option<String> {
    let queued = lane.queue.queued.max(0) as u64;
    let running = lane.queue.running.max(0) as u64;
    let failed = lane.queue.failed.max(0) as u64;
    if queued == 0 && running == 0 && failed == 0 {
        return None;
    }
    Some(format!(
        "Work items: {} waiting · {} in flight · {} failed",
        compact_count_column(queued, 3),
        compact_count_column(running, 3),
        compact_count_column(failed, 3)
    ))
}

fn compact_lane_detail(
    title: &str,
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
) -> Option<String> {
    if title == INIT_EMBEDDINGS_SECTION_TITLE {
        return compact_ready_summary(lane, false).or_else(|| compact_lane_waiting_detail(lane));
    }
    if title == INIT_SUMMARIES_SECTION_TITLE {
        return compact_ready_summary(lane, true).or_else(|| compact_lane_waiting_detail(lane));
    }

    lane.activity_label
        .clone()
        .or_else(|| lane.detail.clone())
        .or_else(|| compact_lane_waiting_detail(lane))
        .or_else(|| {
            if lane.status.eq_ignore_ascii_case("completed") {
                Some("Complete".to_string())
            } else if lane.status.eq_ignore_ascii_case("failed") {
                Some("Failed".to_string())
            } else {
                None
            }
        })
}

fn compact_ready_summary(
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
    include_percent: bool,
) -> Option<String> {
    let progress = lane.progress.as_ref()?;
    let total = progress.total.max(0);
    if total == 0 {
        return None;
    }
    let completed = progress.completed.max(0).min(total);
    if include_percent {
        let ratio = (completed as f64 / total as f64).clamp(0.0, 1.0);
        let total_width = total.to_string().len();
        return Some(format!(
            "{:>3}% · {} / {} ready",
            (ratio * 100.0).round() as usize,
            compact_count_column(completed as u64, total_width),
            total
        ));
    }
    Some(format!(
        "{} / {} ready",
        compact_count_column(completed as u64, total.to_string().len()),
        total
    ))
}

fn compact_lane_waiting_detail(
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
) -> Option<String> {
    lane.waiting_reason
        .as_ref()
        .map(|reason| match reason.as_str() {
            "waiting_for_follow_up_sync" => "Waiting for follow-up sync".to_string(),
            other => waiting_reason_label(other).to_string(),
        })
}

fn compact_session_status_line(
    snapshot: &crate::cli::devql::graphql::RuntimeSnapshotGraphqlRecord,
    session: &crate::cli::devql::graphql::RuntimeInitSessionGraphqlRecord,
) -> String {
    let mut parts = vec![compact_session_status_text(session)];
    if let Some(summary) = session.warning_summary.as_ref() {
        parts.push(summary.clone());
    } else if !snapshot.blocked_mailboxes.is_empty() {
        let blocked = snapshot
            .blocked_mailboxes
            .iter()
            .map(|blocked| blocked.display_name.as_str())
            .collect::<Vec<_>>();
        match blocked.as_slice() {
            [] => {}
            [label] => parts.push(format!("Blocked worker pool: {label}")),
            [label, ..] => parts.push(format!(
                "Blocked worker pools: {label} +{} more",
                blocked.len() - 1
            )),
        }
    }
    parts.join(" | ")
}

fn compact_session_status_text(
    session: &crate::cli::devql::graphql::RuntimeInitSessionGraphqlRecord,
) -> String {
    if let Some(reason) = session.waiting_reason.as_deref() {
        return match reason {
            "waiting_for_follow_up_sync" | "waiting_for_top_level_work" => {
                "Waiting for codebase processing to stabilise".to_string()
            }
            "waiting_on_blocked_mailbox" | "blocked_mailbox" => {
                "Waiting for blocked worker pools".to_string()
            }
            "waiting_for_embeddings_bootstrap" => "Waiting for embeddings to be ready".to_string(),
            "waiting_for_summary_bootstrap" => "Waiting for summaries to be ready".to_string(),
            other => waiting_reason_label(other).to_string(),
        };
    }

    match session.status.to_ascii_lowercase().as_str() {
        "completed" => "Setup tasks completed".to_string(),
        "completed_with_warnings" => "Setup tasks completed with warnings".to_string(),
        "failed" => "Setup failed".to_string(),
        "queued" => "Waiting to start background processing".to_string(),
        "running" => "Building your project's Intelligence Layer".to_string(),
        _ => "Background processing is running".to_string(),
    }
}

fn compact_count_column(value: u64, width: usize) -> String {
    format!("{value:>width$}")
}

#[allow(dead_code)]
fn render_lane(
    title: &str,
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
    task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
    summary_run: Option<&crate::cli::devql::graphql::RuntimeSummaryBootstrapRunGraphqlRecord>,
    render_context: &LaneRenderContext<'_>,
) -> String {
    let mut lines = Vec::new();
    lines.push(fit_line(title, render_context.terminal_width));
    lines.push(render_lane_progress_bar(
        title,
        lane,
        task,
        summary_run,
        render_context.spinner_index,
        render_context.terminal_width,
    ));
    lines.push(format!(
        "{} {}",
        lane_status_icon(
            lane.status.as_str(),
            render_context.spinner,
            render_context.tick,
        ),
        fit_line(
            &lane_status_text(lane, task, summary_run),
            render_context
                .terminal_width
                .map(|width| width.saturating_sub(2)),
        )
    ));
    let queue_line = queue_status_text(lane);
    if !queue_line.is_empty() {
        lines.push(fit_line(&queue_line, render_context.terminal_width));
    }
    if let Some(warning_line) = lane_warning_text(lane) {
        lines.push(fit_line(&warning_line, render_context.terminal_width));
    }
    lines.join("\n")
}

fn task_for_lane<'a>(
    snapshot: &'a crate::cli::devql::graphql::RuntimeSnapshotGraphqlRecord,
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
) -> Option<&'a crate::cli::devql::graphql::TaskGraphqlRecord> {
    lane.task_id
        .as_deref()
        .and_then(|task_id| task_by_id(snapshot, task_id))
}

fn task_by_id<'a>(
    snapshot: &'a crate::cli::devql::graphql::RuntimeSnapshotGraphqlRecord,
    task_id: &str,
) -> Option<&'a crate::cli::devql::graphql::TaskGraphqlRecord> {
    snapshot
        .task_queue
        .current_repo_tasks
        .iter()
        .find(|task| task.task_id == task_id)
}

#[allow(dead_code)]
fn session_status_line(
    snapshot: &crate::cli::devql::graphql::RuntimeSnapshotGraphqlRecord,
    session: &crate::cli::devql::graphql::RuntimeInitSessionGraphqlRecord,
) -> String {
    let mut line = format!(
        "Init session {}",
        session_status_label(session.status.as_str())
    );
    if let Some(reason) = session.waiting_reason.as_ref() {
        line.push_str(&format!(" · {}", waiting_reason_label(reason.as_str())));
    }
    if let Some(summary) = session.warning_summary.as_ref() {
        line.push_str(&format!(" · {summary}"));
    } else if !snapshot.blocked_mailboxes.is_empty() {
        let blocked = snapshot
            .blocked_mailboxes
            .iter()
            .map(|blocked| blocked.display_name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        line.push_str(&format!(" · blocked worker pools: {blocked}"));
    }
    line
}

#[allow(dead_code)]
fn lane_status_text(
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
    task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
    summary_run: Option<&crate::cli::devql::graphql::RuntimeSummaryBootstrapRunGraphqlRecord>,
) -> String {
    if let Some(task) = task.filter(|task| is_active_runtime_status(task.status.as_str())) {
        return task_status_text(task);
    }
    if let Some(run) = summary_run
        && is_active_runtime_status(run.status.as_str())
    {
        return summary_run_status_text(run, lane);
    }
    if lane.status.eq_ignore_ascii_case("skipped") {
        return "Not selected".to_string();
    }
    if lane.status.eq_ignore_ascii_case("completed") {
        return "Complete".to_string();
    }
    if lane.status.eq_ignore_ascii_case("warning") {
        return "Completed with warnings".to_string();
    }
    if lane.status.eq_ignore_ascii_case("failed") {
        return lane.detail.clone().unwrap_or_else(|| "Failed".to_string());
    }
    if let Some(reason) = lane.waiting_reason.as_ref() {
        return waiting_reason_label(reason).to_string();
    }
    lane.activity_label
        .clone()
        .or_else(|| lane.detail.clone())
        .unwrap_or_else(|| "Working".to_string())
}

#[allow(dead_code)]
fn task_status_text(task: &crate::cli::devql::graphql::TaskGraphqlRecord) -> String {
    if task.is_sync() {
        let mut line = format!(
            "{} · {}",
            task_kind_label("sync"),
            task.sync_progress
                .as_ref()
                .map(|progress| sync_phase_label(progress.phase.as_str()))
                .unwrap_or("Working"),
        );
        if let Some(progress) = task.sync_progress.as_ref() {
            if progress.paths_total > 0 {
                line.push_str(&format!(
                    " · {}/{} files",
                    progress.paths_completed, progress.paths_total
                ));
            }
            if let Some(path) = progress.current_path.as_ref() {
                line.push_str(&format!(" · {path}"));
            }
        }
        return line;
    }
    if task.is_ingest() {
        let mut line = format!(
            "{} · {}",
            task_kind_label("ingest"),
            task.ingest_progress
                .as_ref()
                .map(|progress| ingest_phase_label(progress.phase.as_str()))
                .unwrap_or("Working"),
        );
        if let Some(progress) = task.ingest_progress.as_ref() {
            if progress.commits_total > 0 {
                line.push_str(&format!(
                    " · {}/{} commits",
                    progress.commits_processed, progress.commits_total
                ));
            }
            if let Some(commit_sha) = progress.current_commit_sha.as_ref() {
                line.push_str(&format!(" · {commit_sha}"));
            }
        }
        return line;
    }
    if task.is_embeddings_bootstrap() {
        let mut line = task
            .embeddings_bootstrap_progress
            .as_ref()
            .map(|progress| embeddings_bootstrap_phase_label(progress.phase.as_str()).to_string())
            .unwrap_or_else(|| "Preparing the embeddings runtime".to_string());
        if let Some(progress) = task.embeddings_bootstrap_progress.as_ref() {
            if let Some(asset_name) = progress.asset_name.as_ref() {
                line.push_str(&format!(" · {asset_name}"));
            } else if let Some(message) = progress.message.as_ref() {
                line.push_str(&format!(" · {message}"));
            }
        }
        return line;
    }
    format!("Working on {}", task.repo_name)
}

#[allow(dead_code)]
fn summary_run_status_text(
    run: &crate::cli::devql::graphql::RuntimeSummaryBootstrapRunGraphqlRecord,
    _lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
) -> String {
    let mut line = summary_bootstrap_phase_label(run.progress.phase.as_str()).to_string();
    if let Some(message) = run.progress.message.as_ref() {
        line.push_str(&format!(" · {message}"));
    } else if let Some(asset_name) = run.progress.asset_name.as_ref() {
        line.push_str(&format!(" · {asset_name}"));
    }
    line
}

#[allow(dead_code)]
fn render_lane_progress_bar(
    title: &str,
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
    task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
    summary_run: Option<&crate::cli::devql::graphql::RuntimeSummaryBootstrapRunGraphqlRecord>,
    spinner_index: usize,
    terminal_width: Option<usize>,
) -> String {
    let available_width = terminal_width.unwrap_or(80).max(20);
    let (ratio, summary) =
        if let Some(task) = task.filter(|task| is_active_runtime_status(task.status.as_str())) {
            task_progress(task)
        } else if let Some(run) =
            summary_run.filter(|run| is_active_runtime_status(run.status.as_str()))
        {
            summary_progress(run)
        } else {
            lane_progress(title, lane)
        };
    let reserved = summary.chars().count() + 2;
    if available_width <= reserved + 1 {
        return fit_line(summary.trim(), Some(available_width));
    }
    let bar_width = available_width - reserved;
    let bar = if let Some(ratio) = ratio {
        render_determinate_progress_bar(bar_width, ratio)
    } else {
        render_indeterminate_progress_bar(bar_width, spinner_index)
    };
    format!("[{bar}]{summary}")
}

fn task_progress(task: &crate::cli::devql::graphql::TaskGraphqlRecord) -> (Option<f64>, String) {
    if task.is_sync() {
        if let Some(progress) = task.sync_progress.as_ref()
            && progress.paths_total > 0
        {
            let ratio =
                (progress.paths_completed as f64 / progress.paths_total as f64).clamp(0.0, 1.0);
            return (
                Some(ratio),
                format!(
                    " {:>3}% {}/{} paths",
                    (ratio * 100.0).round() as usize,
                    progress.paths_completed,
                    progress.paths_total
                ),
            );
        }
        return (
            None,
            format!(
                " {} ",
                task.sync_progress
                    .as_ref()
                    .map(|progress| sync_phase_label(progress.phase.as_str()))
                    .unwrap_or("Working")
            ),
        );
    }
    if task.is_ingest() {
        if let Some(progress) = task.ingest_progress.as_ref()
            && progress.commits_total > 0
        {
            let ratio =
                (progress.commits_processed as f64 / progress.commits_total as f64).clamp(0.0, 1.0);
            return (
                Some(ratio),
                format!(
                    " {:>3}% {}/{} commits",
                    (ratio * 100.0).round() as usize,
                    progress.commits_processed,
                    progress.commits_total
                ),
            );
        }
        return (
            None,
            format!(
                " {} ",
                task.ingest_progress
                    .as_ref()
                    .map(|progress| ingest_phase_label(progress.phase.as_str()))
                    .unwrap_or("Working")
            ),
        );
    }
    if task.is_embeddings_bootstrap() {
        if let Some(progress) = task.embeddings_bootstrap_progress.as_ref()
            && let Some(total) = progress.bytes_total
            && total > 0
        {
            let ratio = (progress.bytes_downloaded as f64 / total as f64).clamp(0.0, 1.0);
            return (
                Some(ratio),
                format!(
                    " {:>3}% {}",
                    (ratio * 100.0).round() as usize,
                    embeddings_bootstrap_phase_label(progress.phase.as_str())
                ),
            );
        }
        return (
            None,
            format!(
                " {} ",
                task.embeddings_bootstrap_progress
                    .as_ref()
                    .map(|progress| embeddings_bootstrap_phase_label(progress.phase.as_str()))
                    .unwrap_or("Preparing the embeddings runtime")
            ),
        );
    }
    (None, format!(" {} ", task.status.to_ascii_lowercase()))
}

fn summary_progress(
    run: &crate::cli::devql::graphql::RuntimeSummaryBootstrapRunGraphqlRecord,
) -> (Option<f64>, String) {
    if let Some(total) = run.progress.bytes_total
        && total > 0
    {
        let ratio = (run.progress.bytes_downloaded as f64 / total as f64).clamp(0.0, 1.0);
        return (
            Some(ratio),
            format!(
                " {:>3}% {}",
                (ratio * 100.0).round() as usize,
                summary_bootstrap_phase_label(run.progress.phase.as_str())
            ),
        );
    }
    (
        None,
        format!(
            " {} ",
            summary_bootstrap_phase_label(run.progress.phase.as_str())
        ),
    )
}

fn progress_summary(
    title: &str,
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
) -> Option<(f64, String)> {
    let progress = lane.progress.as_ref()?;
    if progress.total <= 0 {
        return None;
    }

    let completed = progress.completed.max(0).min(progress.total.max(0));
    let total = progress.total.max(0);
    let remaining = progress.remaining.max(0);
    let ratio = (completed as f64 / total as f64).clamp(0.0, 1.0);
    let noun = if title == INIT_SUMMARIES_LANE_LABEL {
        "summaries"
    } else if title == INIT_EMBEDDINGS_LANE_LABEL {
        "embeddings"
    } else {
        "items"
    };

    Some((
        ratio,
        format!(
            " {:>3}% {} of {} {} ready · {} left",
            (ratio * 100.0).round() as usize,
            completed,
            total,
            noun,
            remaining
        ),
    ))
}

fn lane_progress(
    title: &str,
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
) -> (Option<f64>, String) {
    if lane.status.eq_ignore_ascii_case("completed") || lane.status.eq_ignore_ascii_case("warning")
    {
        if let Some((ratio, summary)) = progress_summary(title, lane) {
            return (Some(ratio), summary);
        }
        return (Some(1.0), " 100% complete ".to_string());
    }
    if lane.status.eq_ignore_ascii_case("skipped") {
        return (Some(1.0), " skipped ".to_string());
    }
    if let Some((ratio, summary)) = progress_summary(title, lane) {
        return (Some(ratio), summary);
    }

    if lane.status.eq_ignore_ascii_case("waiting") && lane.waiting_reason.is_some() {
        return (
            None,
            format!(
                " {} ",
                waiting_reason_label(
                    lane.waiting_reason
                        .as_deref()
                        .unwrap_or(lane.status.as_str()),
                )
            ),
        );
    }

    let total = lane.queue.queued + lane.queue.running + lane.queue.failed;
    if total > 0 {
        return (
            None,
            format!(" {} ", lane.activity_label.as_deref().unwrap_or(title)),
        );
    }

    (
        None,
        format!(
            " {} ",
            waiting_reason_label(
                lane.waiting_reason
                    .as_deref()
                    .unwrap_or(lane.status.as_str()),
            )
        ),
    )
}

#[allow(dead_code)]
fn queue_status_text(lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord) -> String {
    let queued = lane.queue.queued.max(0) as u64;
    let running = lane.queue.running.max(0) as u64;
    let failed = lane.queue.failed.max(0) as u64;
    if queued == 0 && running == 0 && failed == 0 {
        return String::new();
    }
    queue_state_summary(queued, running, failed)
}

#[allow(dead_code)]
fn lane_warning_text(
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
) -> Option<String> {
    if lane.warnings.is_empty() {
        return None;
    }
    let retry_command = lane
        .warnings
        .first()
        .map(|warning| warning.retry_command.as_str())
        .unwrap_or(RETRY_FAILED_ENRICHMENTS_COMMAND);
    if lane.warnings.len() == 1 {
        let warning = &lane.warnings[0];
        return Some(format!(
            "Warning: {}. Retry with: {}",
            warning.message, retry_command
        ));
    }
    Some(format!(
        "Warnings: {} background tasks failed. Retry with: {}",
        lane.warnings.len(),
        retry_command
    ))
}

fn lane_status_icon<'a>(status: &str, spinner: &'a str, tick: &'a str) -> &'a str {
    match status.to_ascii_lowercase().as_str() {
        "completed" | "completed_with_warnings" | "warning" | "skipped" => tick,
        _ => spinner,
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
        .map(|line| visible_terminal_width(line).max(1).div_ceil(width))
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

fn render_determinate_progress_bar(width: usize, ratio: f64) -> String {
    let filled = ((width as f64) * ratio).round() as usize;
    let filled = filled.min(width);
    let fill = color_hex_if_enabled(&"█".repeat(filled), BITLOOPS_PURPLE_HEX);
    let empty = "░".repeat(width.saturating_sub(filled));
    format!("{fill}{empty}")
}

fn render_indeterminate_progress_bar(width: usize, spinner_index: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let position = spinner_index % width;
    let prefix = "░".repeat(position);
    let pulse = color_hex_if_enabled("█", BITLOOPS_PURPLE_HEX);
    let suffix = "░".repeat(width.saturating_sub(position + 1));
    format!("{prefix}{pulse}{suffix}")
}

fn fit_line(text: &str, available_width: Option<usize>) -> String {
    let Some(max_width) = available_width else {
        return text.to_string();
    };
    if max_width == 0 || text.chars().count() <= max_width {
        return text.to_string();
    }
    let prefix_len = (max_width.saturating_sub(1)) / 2;
    let suffix_len = max_width.saturating_sub(1).saturating_sub(prefix_len);
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

fn is_active_runtime_status(status: &str) -> bool {
    matches!(status.to_ascii_lowercase().as_str(), "queued" | "running")
}

#[cfg(test)]
mod tests {
    use super::{
        INIT_EMBEDDINGS_LANE_LABEL, INIT_EMBEDDINGS_SECTION_TITLE, INIT_SUMMARIES_LANE_LABEL,
        INIT_SUMMARIES_SECTION_TITLE, compact_lane_detail, compact_queue_status_text,
        is_active_runtime_status, lane_progress, queue_status_text,
    };
    use crate::cli::devql::graphql::{
        RuntimeInitLaneGraphqlRecord, RuntimeInitLaneProgressGraphqlRecord,
        RuntimeInitLaneQueueGraphqlRecord,
    };
    use crate::runtime_presentation::waiting_reason_label;

    #[test]
    fn waiting_reason_includes_embeddings_bootstrap_copy() {
        assert_eq!(
            waiting_reason_label("waiting_for_embeddings_bootstrap"),
            "Waiting for the embeddings runtime to warm up"
        );
    }

    #[test]
    fn waiting_reason_includes_summary_bootstrap_copy() {
        assert_eq!(
            waiting_reason_label("waiting_for_summary_bootstrap"),
            "Waiting for summary generation to be ready"
        );
    }

    #[test]
    fn active_runtime_status_only_includes_queued_and_running() {
        assert!(is_active_runtime_status("queued"));
        assert!(is_active_runtime_status("running"));
        assert!(!is_active_runtime_status("completed"));
        assert!(!is_active_runtime_status("failed"));
    }

    #[test]
    fn waiting_lane_progress_prefers_waiting_reason_over_queue_ratio() {
        let lane = RuntimeInitLaneGraphqlRecord {
            status: "waiting".to_string(),
            waiting_reason: Some("waiting_for_embeddings_bootstrap".to_string()),
            detail: Some("embeddings_bootstrap".to_string()),
            activity_label: Some("Preparing the embeddings runtime".to_string()),
            task_id: None,
            run_id: None,
            progress: None,
            queue: RuntimeInitLaneQueueGraphqlRecord {
                queued: 1,
                running: 0,
                failed: 0,
            },
            warnings: Vec::new(),
            pending_count: 1,
            running_count: 0,
            failed_count: 0,
            completed_count: 0,
        };

        let (ratio, summary) = lane_progress(INIT_EMBEDDINGS_LANE_LABEL, &lane);

        assert!(ratio.is_none());
        assert_eq!(summary, " Waiting for the embeddings runtime to warm up ");
    }

    #[test]
    fn queue_lane_progress_uses_runtime_progress_payload() {
        let lane = RuntimeInitLaneGraphqlRecord {
            status: "running".to_string(),
            waiting_reason: None,
            detail: Some("Building the semantic search index".to_string()),
            activity_label: Some("Indexing generated summaries".to_string()),
            task_id: None,
            run_id: None,
            progress: Some(RuntimeInitLaneProgressGraphqlRecord {
                completed: 262,
                total: 556,
                remaining: 294,
            }),
            queue: RuntimeInitLaneQueueGraphqlRecord {
                queued: 16,
                running: 2,
                failed: 1,
            },
            warnings: Vec::new(),
            pending_count: 2,
            running_count: 1,
            failed_count: 0,
            completed_count: 8,
        };

        let (ratio, summary) = lane_progress(INIT_EMBEDDINGS_LANE_LABEL, &lane);

        let ratio = ratio.expect("queue lanes with coverage should be determinate");
        assert!((ratio - (262.0 / 556.0)).abs() < f64::EPSILON);
        assert_eq!(summary, "  47% 262 of 556 embeddings ready · 294 left");
    }

    #[test]
    fn completed_lane_progress_uses_runtime_summary_payload() {
        let lane = RuntimeInitLaneGraphqlRecord {
            status: "completed".to_string(),
            waiting_reason: None,
            detail: None,
            activity_label: Some("Generating summaries".to_string()),
            task_id: None,
            run_id: None,
            progress: Some(RuntimeInitLaneProgressGraphqlRecord {
                completed: 225,
                total: 225,
                remaining: 0,
            }),
            queue: RuntimeInitLaneQueueGraphqlRecord {
                queued: 0,
                running: 0,
                failed: 0,
            },
            warnings: Vec::new(),
            pending_count: 0,
            running_count: 0,
            failed_count: 0,
            completed_count: 3,
        };

        let (ratio, summary) = lane_progress(INIT_SUMMARIES_LANE_LABEL, &lane);

        assert_eq!(ratio, Some(1.0));
        assert_eq!(summary, " 100% 225 of 225 summaries ready · 0 left");
    }

    #[test]
    fn queue_status_text_uses_waiting_running_and_failed_counts() {
        let lane = RuntimeInitLaneGraphqlRecord {
            status: "running".to_string(),
            waiting_reason: None,
            detail: None,
            activity_label: Some("Generating summaries".to_string()),
            task_id: None,
            run_id: None,
            progress: None,
            queue: RuntimeInitLaneQueueGraphqlRecord {
                queued: 66,
                running: 0,
                failed: 1,
            },
            warnings: Vec::new(),
            pending_count: 66,
            running_count: 0,
            failed_count: 1,
            completed_count: 8,
        };

        assert_eq!(
            queue_status_text(&lane),
            "Work items: 66 waiting · 0 in flight · 1 failed"
        );
    }

    #[test]
    fn compact_queue_status_text_keeps_all_queue_columns_visible() {
        let lane = RuntimeInitLaneGraphqlRecord {
            status: "running".to_string(),
            waiting_reason: None,
            detail: None,
            activity_label: Some("Indexing source code".to_string()),
            task_id: None,
            run_id: None,
            progress: None,
            queue: RuntimeInitLaneQueueGraphqlRecord {
                queued: 66,
                running: 0,
                failed: 1,
            },
            warnings: Vec::new(),
            pending_count: 66,
            running_count: 0,
            failed_count: 1,
            completed_count: 8,
        };

        assert_eq!(
            compact_queue_status_text(&lane),
            Some("Work items:  66 waiting ·   0 in flight ·   1 failed".to_string())
        );
    }

    #[test]
    fn compact_lane_detail_pads_ready_counts_to_total_width() {
        let lane = RuntimeInitLaneGraphqlRecord {
            status: "running".to_string(),
            waiting_reason: None,
            detail: None,
            activity_label: Some("Indexing source code".to_string()),
            task_id: None,
            run_id: None,
            progress: Some(RuntimeInitLaneProgressGraphqlRecord {
                completed: 7,
                total: 570,
                remaining: 563,
            }),
            queue: RuntimeInitLaneQueueGraphqlRecord::default(),
            warnings: Vec::new(),
            pending_count: 0,
            running_count: 0,
            failed_count: 0,
            completed_count: 0,
        };

        assert_eq!(
            compact_lane_detail(INIT_EMBEDDINGS_SECTION_TITLE, &lane),
            Some("  7 / 570 ready".to_string())
        );
        assert_eq!(
            compact_lane_detail(INIT_SUMMARIES_SECTION_TITLE, &lane),
            Some("  1% ·   7 / 570 ready".to_string())
        );
    }
}
