//! Legacy renderers retained for compatibility while the compact renderer is
//! the only path exercised in production. The functions here are reachable
//! only from this module and so are gated with `#[allow(dead_code)]`.

use crate::runtime_presentation::{
    RETRY_FAILED_ENRICHMENTS_COMMAND, embeddings_bootstrap_phase_label, ingest_phase_label,
    queue_state_summary, session_status_label, summary_bootstrap_phase_label, sync_phase_label,
    task_kind_label, waiting_reason_label,
};

use super::bars::{render_determinate_progress_bar, render_indeterminate_progress_bar};
use super::compact::{LaneRenderContext, compact_lane_in_memory_ratio};
use super::progress_calc::{
    is_active_runtime_status, lane_progress, lane_status_icon, summary_progress, task_progress,
};
use super::viewport::fit_line;

#[allow(dead_code)]
pub(crate) fn render_lane(
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
        render_determinate_progress_bar(
            bar_width,
            ratio,
            compact_lane_in_memory_ratio(lane, task, summary_run),
        )
    } else {
        render_indeterminate_progress_bar(bar_width, spinner_index)
    };
    format!("[{bar}]{summary}")
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

#[cfg(test)]
mod tests {
    use super::queue_status_text;
    use crate::cli::devql::graphql::{
        RuntimeInitLaneGraphqlRecord, RuntimeInitLaneQueueGraphqlRecord,
    };

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
}
