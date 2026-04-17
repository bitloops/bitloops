use super::super::{
    EmbeddingCompletionSource, EmbeddingQueueSnapshot, InitChecklistState, SummaryProgressState,
};
use super::terminal::{
    fit_init_status_text, format_count_u64, format_megabytes, humanise_summary_setup_phase,
    remaining_init_dependencies, render_init_determinate_progress_bar,
    render_init_indeterminate_progress_bar,
};

pub(super) fn format_embedding_queue_status_line(
    snapshot: EmbeddingQueueSnapshot,
    spinner: &str,
) -> String {
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

pub(super) fn format_embedding_queue_progress_bar_line(
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

pub(super) fn format_queue_waiting_progress_bar_line(
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

pub(super) fn format_embedding_queue_complete_progress_bar_line(
    baseline_total: u64,
    terminal_width: Option<usize>,
) -> String {
    if baseline_total == 0 {
        return super::task::format_init_complete_progress_bar_line(terminal_width);
    }

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

pub(super) fn format_embedding_queue_complete_status_line(
    completion_source: EmbeddingCompletionSource,
    baseline_total: u64,
    tick: &str,
    terminal_width: Option<usize>,
) -> String {
    let line = match completion_source {
        EmbeddingCompletionSource::Queue if baseline_total > 0 => {
            "Embedding queue complete".to_string()
        }
        EmbeddingCompletionSource::InlineSync => "Embeddings generated during sync".to_string(),
        EmbeddingCompletionSource::Queue | EmbeddingCompletionSource::NoneRequired => {
            "No queued embedding artefacts were needed".to_string()
        }
    };
    let fitted = fit_init_status_text(
        line.as_str(),
        terminal_width.map(|width| width.saturating_sub(2)),
    );
    format!("{tick} {fitted}")
}

pub(super) fn format_embedding_waiting_status_line(
    checklist: InitChecklistState,
    completed_jobs: u64,
    failed_jobs: u64,
    spinner: &str,
    terminal_width: Option<usize>,
) -> String {
    let prefix = if completed_jobs > 0 {
        format!(
            "bitloops-local-embeddings processed {} artefacts",
            format_count_u64(completed_jobs)
        )
    } else {
        "bitloops-local-embeddings is ready".to_string()
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

pub(super) fn format_summary_progress_bar_line(
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

pub(super) fn format_summary_status_line(
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
