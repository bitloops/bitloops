use crate::runtime_presentation::{
    INIT_CODE_EMBEDDINGS_LANE_LABEL, INIT_SUMMARIES_LANE_LABEL, INIT_SUMMARY_EMBEDDINGS_LANE_LABEL,
    embeddings_bootstrap_phase_label, ingest_phase_label, summary_bootstrap_phase_label,
    sync_phase_label, waiting_reason_label,
};

use super::compact::lane_progress_counts;

pub(crate) fn task_progress(
    task: &crate::cli::devql::graphql::TaskGraphqlRecord,
) -> (Option<f64>, String) {
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

pub(crate) fn summary_progress(
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
    let counts = lane_progress_counts(lane.progress.as_ref()?)?;
    let ratio = (counts.visible_completed as f64 / counts.total as f64).clamp(0.0, 1.0);
    let noun = if title == INIT_SUMMARIES_LANE_LABEL {
        "summaries"
    } else if title == INIT_CODE_EMBEDDINGS_LANE_LABEL {
        "code embeddings"
    } else if title == INIT_SUMMARY_EMBEDDINGS_LANE_LABEL {
        "summary embeddings"
    } else {
        "items"
    };

    if title == INIT_SUMMARIES_LANE_LABEL && counts.in_memory_completed > 0 {
        return Some((
            ratio,
            format!(
                " {:>3}% {} of {} summaries generated · {} persisted · {} left",
                (ratio * 100.0).round() as usize,
                counts.visible_completed,
                counts.total,
                counts.completed,
                counts.remaining
            ),
        ));
    }

    Some((
        ratio,
        format!(
            " {:>3}% {} of {} {} ready · {} left",
            (ratio * 100.0).round() as usize,
            counts.completed,
            counts.total,
            noun,
            counts.remaining
        ),
    ))
}

pub(crate) fn lane_progress(
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

pub(crate) fn lane_status_icon<'a>(status: &str, spinner: &'a str, tick: &'a str) -> &'a str {
    match status.to_ascii_lowercase().as_str() {
        "completed" | "completed_with_warnings" | "skipped" => tick,
        _ => spinner,
    }
}

pub(crate) fn is_active_runtime_status(status: &str) -> bool {
    matches!(status.to_ascii_lowercase().as_str(), "queued" | "running")
}

#[cfg(test)]
mod tests {
    use super::{is_active_runtime_status, lane_progress, lane_status_icon};
    use crate::cli::devql::graphql::{
        RuntimeInitLaneGraphqlRecord, RuntimeInitLaneProgressGraphqlRecord,
        RuntimeInitLaneQueueGraphqlRecord,
    };
    use crate::runtime_presentation::{
        INIT_CODE_EMBEDDINGS_LANE_LABEL, INIT_SUMMARIES_LANE_LABEL,
        INIT_SUMMARY_EMBEDDINGS_LANE_LABEL, waiting_reason_label,
    };

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
    fn warning_lane_status_icon_uses_spinner() {
        assert_eq!(lane_status_icon("warning", "spin", "tick"), "spin");
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

        let (ratio, summary) = lane_progress(INIT_CODE_EMBEDDINGS_LANE_LABEL, &lane);

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
                in_memory_completed: 0,
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

        let (ratio, summary) = lane_progress(INIT_CODE_EMBEDDINGS_LANE_LABEL, &lane);

        let ratio = ratio.expect("queue lanes with coverage should be determinate");
        assert!((ratio - (262.0 / 556.0)).abs() < f64::EPSILON);
        assert_eq!(summary, "  47% 262 of 556 code embeddings ready · 294 left");
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
                in_memory_completed: 0,
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
    fn summary_embedding_lane_progress_uses_specific_noun() {
        let lane = RuntimeInitLaneGraphqlRecord {
            status: "running".to_string(),
            waiting_reason: None,
            detail: None,
            activity_label: Some("Creating summary embeddings".to_string()),
            task_id: None,
            run_id: None,
            progress: Some(RuntimeInitLaneProgressGraphqlRecord {
                completed: 12,
                in_memory_completed: 0,
                total: 40,
                remaining: 28,
            }),
            queue: RuntimeInitLaneQueueGraphqlRecord {
                queued: 4,
                running: 1,
                failed: 0,
            },
            warnings: Vec::new(),
            pending_count: 4,
            running_count: 1,
            failed_count: 0,
            completed_count: 12,
        };

        let (ratio, summary) = lane_progress(INIT_SUMMARY_EMBEDDINGS_LANE_LABEL, &lane);

        let ratio = ratio.expect("summary embedding lane should render a determinate ratio");
        assert!((ratio - 0.3).abs() < f64::EPSILON);
        assert_eq!(summary, "  30% 12 of 40 summary embeddings ready · 28 left");
    }

    #[test]
    fn summary_lane_progress_counts_in_memory_completions() {
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
            queue: RuntimeInitLaneQueueGraphqlRecord {
                queued: 20,
                running: 3,
                failed: 0,
            },
            warnings: Vec::new(),
            pending_count: 20,
            running_count: 3,
            failed_count: 0,
            completed_count: 10,
        };

        let (ratio, summary) = lane_progress(INIT_SUMMARIES_LANE_LABEL, &lane);

        let ratio = ratio.expect("summary lane should render a determinate ratio");
        assert!((ratio - 0.25).abs() < f64::EPSILON);
        assert_eq!(
            summary,
            "  25% 25 of 100 summaries generated · 10 persisted · 75 left"
        );
    }
}
