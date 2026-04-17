use super::terminal::{
    fit_init_status_text, format_count_i32, format_megabytes, humanise_init_bootstrap_phase,
    humanise_init_ingest_phase, humanise_init_sync_phase, render_init_determinate_progress_bar,
    render_init_indeterminate_progress_bar, visible_init_progress_percent,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InitTaskLaneKind {
    Sync,
    Ingest,
}

impl InitTaskLaneKind {
    fn description(self) -> &'static str {
        match self {
            Self::Sync => "Analysing your current branch to know what's what",
            Self::Ingest => "Analysing your git history because you know... history is important",
        }
    }

    fn completion_label(self) -> &'static str {
        match self {
            Self::Sync => "Sync complete",
            Self::Ingest => "Ingest complete",
        }
    }
}

pub(super) fn init_task_description_for_kind(kind: InitTaskLaneKind) -> &'static str {
    kind.description()
}

pub(super) fn format_init_task_status_line(
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

pub(super) fn format_init_task_progress_bar_line(
    task: &crate::cli::devql::graphql::TaskGraphqlRecord,
    spinner_index: usize,
    terminal_width: Option<usize>,
) -> String {
    let available_width = terminal_width.unwrap_or(80).max(16);
    let summary = if let Some((ratio, done, total, unit)) = init_task_progress_ratio(task) {
        format!(
            " {:>3}% {done}/{total} {unit}",
            visible_init_progress_percent(ratio),
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

pub(super) fn format_init_waiting_progress_bar_line(
    spinner_index: usize,
    terminal_width: Option<usize>,
) -> String {
    let available_width = terminal_width.unwrap_or(80).max(16);
    let summary = " waiting ".to_string();
    let reserved = summary.chars().count() + 2;
    if available_width <= reserved + 1 {
        return fit_init_status_text(summary.trim(), Some(available_width));
    }

    let bar_width = available_width - reserved;
    let bar = render_init_indeterminate_progress_bar(bar_width, spinner_index);
    format!("[{bar}]{summary}")
}

pub(super) fn format_init_complete_progress_bar_line(terminal_width: Option<usize>) -> String {
    let available_width = terminal_width.unwrap_or(80).max(16);
    let summary = " 100% complete ".to_string();
    let reserved = summary.chars().count() + 2;
    if available_width <= reserved + 1 {
        return fit_init_status_text(summary.trim(), Some(available_width));
    }

    let bar_width = available_width - reserved;
    let bar = render_init_determinate_progress_bar(bar_width, 1.0);
    format!("[{bar}]{summary}")
}

pub(super) fn format_init_task_state_status_line(
    status: &str,
    icon: &str,
    terminal_width: Option<usize>,
) -> String {
    let fitted = fit_init_status_text(status, terminal_width.map(|width| width.saturating_sub(2)));
    format!("{icon} {fitted}")
}

pub(super) fn init_task_completion_label(kind: InitTaskLaneKind) -> &'static str {
    kind.completion_label()
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
