use crate::runtime_presentation::{
    INIT_CODE_EMBEDDINGS_SECTION_TITLE, INIT_INGEST_SECTION_TITLE, INIT_SUMMARIES_SECTION_TITLE,
    INIT_SUMMARY_EMBEDDINGS_SECTION_TITLE, INIT_SYNC_SECTION_TITLE, waiting_reason_label,
};

use super::bars::{render_determinate_progress_bar, render_indeterminate_progress_bar};
use super::progress_calc::{
    is_active_runtime_status, lane_progress, lane_status_icon, summary_progress, task_progress,
};
use super::viewport::fit_line;

pub(crate) struct LaneRenderContext<'a> {
    pub(crate) spinner: &'a str,
    pub(crate) tick: &'a str,
    pub(crate) spinner_index: usize,
    pub(crate) terminal_width: Option<usize>,
}

pub(crate) fn compact_selected_section_titles(
    session: &crate::cli::devql::graphql::RuntimeInitSessionGraphqlRecord,
) -> Vec<&'static str> {
    let mut titles = Vec::new();
    if session.run_sync {
        titles.push(INIT_SYNC_SECTION_TITLE);
    }
    if session.run_ingest {
        titles.push(INIT_INGEST_SECTION_TITLE);
    }
    if session.embeddings_selected {
        titles.push(INIT_CODE_EMBEDDINGS_SECTION_TITLE);
    }
    if session.summaries_selected {
        titles.push(INIT_SUMMARIES_SECTION_TITLE);
    }
    if session.summary_embeddings_selected {
        titles.push(INIT_SUMMARY_EMBEDDINGS_SECTION_TITLE);
    }
    titles
}

pub(crate) fn render_compact_lane(
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
                .map(|width| width.saturating_sub(3)),
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
        render_determinate_progress_bar(
            bar_width,
            ratio,
            compact_lane_in_memory_ratio(lane, task, summary_run),
        )
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

pub(crate) fn compact_lane_in_memory_ratio(
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
    task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
    summary_run: Option<&crate::cli::devql::graphql::RuntimeSummaryBootstrapRunGraphqlRecord>,
) -> f64 {
    if task.is_some_and(|task| is_active_runtime_status(task.status.as_str()))
        || summary_run.is_some_and(|run| is_active_runtime_status(run.status.as_str()))
    {
        return 0.0;
    }
    lane_in_memory_ratio(lane)
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
    if title == INIT_CODE_EMBEDDINGS_SECTION_TITLE || title == INIT_SUMMARY_EMBEDDINGS_SECTION_TITLE
    {
        return preferred_compact_waiting_detail(lane)
            .or_else(|| compact_ready_summary(title, lane, false))
            .or_else(|| lane.activity_label.clone())
            .or_else(|| lane.detail.clone());
    }
    if title == INIT_SUMMARIES_SECTION_TITLE {
        return preferred_compact_waiting_detail(lane)
            .or_else(|| compact_ready_summary(title, lane, true))
            .or_else(|| lane.activity_label.clone())
            .or_else(|| lane.detail.clone());
    }

    lane.activity_label
        .clone()
        .or_else(|| compact_lane_waiting_detail(lane))
        .or_else(|| lane.detail.clone())
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LaneProgressCounts {
    pub(crate) completed: u64,
    pub(crate) in_memory_completed: u64,
    pub(crate) total: u64,
    pub(crate) visible_completed: u64,
    pub(crate) remaining: u64,
}

pub(crate) fn lane_progress_counts(
    progress: &crate::cli::devql::graphql::RuntimeInitLaneProgressGraphqlRecord,
) -> Option<LaneProgressCounts> {
    let total = progress.total.max(0) as u64;
    if total == 0 {
        return None;
    }

    let completed = (progress.completed.max(0) as u64).min(total);
    let in_memory_completed =
        (progress.in_memory_completed.max(0) as u64).min(total.saturating_sub(completed));
    let visible_completed = completed.saturating_add(in_memory_completed).min(total);
    Some(LaneProgressCounts {
        completed,
        in_memory_completed,
        total,
        visible_completed,
        remaining: total.saturating_sub(visible_completed),
    })
}

fn lane_in_memory_ratio(lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord) -> f64 {
    lane.progress
        .as_ref()
        .and_then(lane_progress_counts)
        .map(|counts| counts.in_memory_completed as f64 / counts.total as f64)
        .unwrap_or(0.0)
}

fn compact_ready_summary(
    title: &str,
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
    include_percent: bool,
) -> Option<String> {
    let counts = lane_progress_counts(lane.progress.as_ref()?)?;
    if include_percent {
        let ratio = (counts.visible_completed as f64 / counts.total as f64).clamp(0.0, 1.0);
        let total_width = counts.total.to_string().len();
        if counts.in_memory_completed > 0 {
            return Some(format!(
                "{:>3}% · {} / {} generated · {} persisted",
                (ratio * 100.0).round() as usize,
                compact_count_column(counts.visible_completed, total_width),
                counts.total,
                compact_count_column(counts.completed, total_width),
            ));
        }
        return Some(format!(
            "{:>3}% · {} / {} ready",
            (ratio * 100.0).round() as usize,
            compact_count_column(counts.completed, total_width),
            counts.total
        ));
    }
    Some(if title == INIT_CODE_EMBEDDINGS_SECTION_TITLE {
        format!(
            "{} / {} indexed",
            compact_count_column(counts.completed, counts.total.to_string().len()),
            counts.total
        )
    } else {
        format!(
            "{} / {} ready",
            compact_count_column(counts.completed, counts.total.to_string().len()),
            counts.total
        )
    })
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

fn preferred_compact_waiting_detail(
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
) -> Option<String> {
    lane.waiting_reason
        .as_deref()
        .filter(|reason| {
            matches!(
                *reason,
                "waiting_for_sync"
                    | "waiting_for_embeddings_bootstrap"
                    | "waiting_for_summary_bootstrap"
                    | "waiting_for_follow_up_sync"
                    | "waiting_for_summaries"
            )
        })
        .and_then(|_| compact_lane_waiting_detail(lane))
}

fn compact_count_column(value: u64, width: usize) -> String {
    format!("{value:>width$}")
}

#[cfg(test)]
mod tests {
    use super::{
        LaneRenderContext, compact_lane_detail, compact_queue_status_text, render_compact_lane,
    };
    use crate::cli::devql::graphql::{
        RuntimeInitLaneGraphqlRecord, RuntimeInitLaneProgressGraphqlRecord,
        RuntimeInitLaneQueueGraphqlRecord,
    };
    use crate::runtime_presentation::{
        INIT_CODE_EMBEDDINGS_SECTION_TITLE, INIT_SUMMARIES_SECTION_TITLE,
    };

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
                in_memory_completed: 0,
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
            compact_lane_detail(INIT_CODE_EMBEDDINGS_SECTION_TITLE, &lane),
            Some("  7 / 570 indexed".to_string())
        );
        assert_eq!(
            compact_lane_detail(INIT_SUMMARIES_SECTION_TITLE, &lane),
            Some("  1% ·   7 / 570 ready".to_string())
        );
    }

    #[test]
    fn compact_lane_detail_shows_generated_and_persisted_summary_counts() {
        let lane = RuntimeInitLaneGraphqlRecord {
            status: "running".to_string(),
            waiting_reason: None,
            detail: None,
            activity_label: Some("Generating summaries".to_string()),
            task_id: None,
            run_id: None,
            progress: Some(RuntimeInitLaneProgressGraphqlRecord {
                completed: 10,
                in_memory_completed: 15,
                total: 100,
                remaining: 90,
            }),
            queue: RuntimeInitLaneQueueGraphqlRecord::default(),
            warnings: Vec::new(),
            pending_count: 0,
            running_count: 0,
            failed_count: 0,
            completed_count: 0,
        };

        assert_eq!(
            compact_lane_detail(INIT_SUMMARIES_SECTION_TITLE, &lane),
            Some(" 25% ·  25 / 100 generated ·  10 persisted".to_string())
        );
    }

    #[test]
    fn compact_embeddings_waiting_detail_beats_ready_summary() {
        let lane = RuntimeInitLaneGraphqlRecord {
            status: "waiting".to_string(),
            waiting_reason: Some("waiting_for_follow_up_sync".to_string()),
            detail: None,
            activity_label: Some("Running a follow-up sync".to_string()),
            task_id: None,
            run_id: None,
            progress: Some(RuntimeInitLaneProgressGraphqlRecord {
                completed: 2193,
                in_memory_completed: 0,
                total: 2243,
                remaining: 50,
            }),
            queue: RuntimeInitLaneQueueGraphqlRecord::default(),
            warnings: Vec::new(),
            pending_count: 0,
            running_count: 0,
            failed_count: 0,
            completed_count: 0,
        };

        assert_eq!(
            compact_lane_detail(INIT_CODE_EMBEDDINGS_SECTION_TITLE, &lane),
            Some("Waiting for follow-up sync".to_string())
        );
    }

    #[test]
    fn compact_lane_status_line_respects_terminal_width_budget() {
        let lane = RuntimeInitLaneGraphqlRecord {
            status: "running".to_string(),
            waiting_reason: None,
            detail: None,
            activity_label: Some("Generating summaries".to_string()),
            task_id: None,
            run_id: None,
            progress: Some(RuntimeInitLaneProgressGraphqlRecord {
                completed: 132,
                in_memory_completed: 30,
                total: 285,
                remaining: 123,
            }),
            queue: RuntimeInitLaneQueueGraphqlRecord {
                queued: 0,
                running: 918,
                failed: 0,
            },
            warnings: Vec::new(),
            pending_count: 0,
            running_count: 918,
            failed_count: 0,
            completed_count: 132,
        };
        let render_context = LaneRenderContext {
            spinner: "⠏",
            tick: "✓",
            spinner_index: 0,
            terminal_width: Some(80),
        };

        let lines = render_compact_lane(
            INIT_SUMMARIES_SECTION_TITLE,
            &lane,
            "Generating summaries",
            None,
            None,
            22,
            &render_context,
        );

        assert_eq!(lines.len(), 2);
        assert!(
            lines[1].chars().count() <= 80,
            "status line exceeded terminal width: `{}`",
            lines[1]
        );
    }
}
