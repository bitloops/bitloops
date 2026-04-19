use crate::daemon::types::{
    DevqlTaskRecord, DevqlTaskStatus, InitSessionRecord, SummaryBootstrapRunRecord,
    SummaryBootstrapStatus,
};
use crate::runtime_presentation::{lane_activity_label, mailbox_label, task_kind_label};

use super::orchestration::{
    embeddings_bootstrap_outstanding_after_initial_sync, embeddings_follow_up_pending,
    summaries_follow_up_pending, summary_bootstrap_outstanding_after_initial_sync, task_failed,
};
use super::stats::{SessionWorkplaneStats, StatusCounts};
use super::types::{
    InitRuntimeLaneProgressView, InitRuntimeLaneQueueView, InitRuntimeLaneView,
    InitRuntimeLaneWarningView,
};

pub(crate) fn derive_top_pipeline_lane(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    ingest_task: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    current_state: StatusCounts,
) -> InitRuntimeLaneView {
    if !session.selections.run_sync && !session.selections.run_ingest {
        return skipped_lane();
    }
    if let Some(task) = active_task(follow_up_sync) {
        return lane_from_task(
            task,
            Some("follow_up_sync".to_string()),
            current_state,
            None,
            Vec::new(),
        );
    }
    if let Some(task) = active_task(ingest_task) {
        return lane_from_task(
            task,
            Some("ingest".to_string()),
            current_state,
            None,
            Vec::new(),
        );
    }
    if let Some(task) = active_task(initial_sync) {
        return lane_from_task(
            task,
            Some("sync".to_string()),
            current_state,
            None,
            Vec::new(),
        );
    }
    if let Some(task) = follow_up_sync
        && task_failed(Some(task))
    {
        return failed_lane(
            Some("Running a follow-up sync failed".to_string()),
            current_state,
            Some(task.task_id.clone()),
            None,
            None,
            Vec::new(),
        );
    }
    if let Some(task) = ingest_task
        && task_failed(Some(task))
    {
        return failed_lane(
            Some("Ingesting commit history failed".to_string()),
            current_state,
            Some(task.task_id.clone()),
            None,
            None,
            Vec::new(),
        );
    }
    if let Some(task) = initial_sync
        && task_failed(Some(task))
    {
        return failed_lane(
            Some("Syncing repository failed".to_string()),
            current_state,
            Some(task.task_id.clone()),
            None,
            None,
            Vec::new(),
        );
    }
    if current_state.failed > 0 {
        return failed_lane(
            Some("Applying codebase updates failed".to_string()),
            current_state,
            None,
            None,
            None,
            Vec::new(),
        );
    }
    if current_state.pending > 0 || current_state.running > 0 {
        return runtime_lane("waiting", None, current_state, Vec::new())
            .with_waiting_reason("waiting_for_current_state_consumer")
            .with_activity_label("Applying codebase updates");
    }
    completed_lane()
}

pub(crate) fn derive_session_status(
    has_failure: bool,
    has_remaining_work: bool,
    completed: bool,
    waiting_reason: Option<&str>,
    has_warnings: bool,
) -> &'static str {
    if has_failure && has_remaining_work {
        "failing"
    } else if has_failure {
        "failed"
    } else if completed && has_warnings {
        "completed_with_warnings"
    } else if completed {
        "completed"
    } else if waiting_reason.is_some_and(|reason| reason.starts_with("waiting")) {
        "waiting"
    } else {
        "running"
    }
}

pub(crate) fn derive_embeddings_lane(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    embeddings_task: Option<&DevqlTaskRecord>,
    stats: &SessionWorkplaneStats,
    progress: Option<InitRuntimeLaneProgressView>,
) -> InitRuntimeLaneView {
    if session.selections.embeddings_bootstrap.is_none() {
        return skipped_lane();
    }
    let warnings = stats.embedding_warnings();
    if let Some(task) = active_task(embeddings_task) {
        return lane_from_task(
            task,
            Some("embeddings_bootstrap".to_string()),
            stats.embedding_jobs,
            progress,
            warnings,
        );
    }
    if let Some(task) = embeddings_task
        && task_failed(Some(task))
    {
        return failed_lane(
            Some("Preparing the embeddings runtime failed".to_string()),
            stats.embedding_jobs,
            Some(task.task_id.clone()),
            None,
            progress,
            warnings,
        );
    }
    if let Some(reason) = stats.blocked_embedding_reason.clone() {
        return runtime_lane("waiting", progress, stats.embedding_jobs, warnings)
            .with_waiting_reason("blocked_mailbox")
            .with_activity_label(
                stats
                    .active_embedding_mailbox()
                    .map(mailbox_label)
                    .unwrap_or("Building the semantic search index")
                    .to_string(),
            )
            .with_detail(reason);
    }
    if stats.embedding_jobs.pending > 0 || stats.embedding_jobs.running > 0 {
        return runtime_lane(
            if stats.embedding_jobs.running > 0 {
                "running"
            } else {
                "queued"
            },
            progress,
            stats.embedding_jobs,
            warnings,
        )
        .with_activity_label(
            stats
                .active_embedding_mailbox()
                .map(mailbox_label)
                .unwrap_or("Building the semantic search index")
                .to_string(),
        );
    }
    if embeddings_bootstrap_outstanding_after_initial_sync(session, initial_sync, embeddings_task) {
        return runtime_lane("waiting", progress, stats.embedding_jobs, warnings)
            .with_waiting_reason("waiting_for_embeddings_bootstrap")
            .with_activity_label("Preparing the embeddings runtime");
    }
    if embeddings_follow_up_pending(session, initial_sync, follow_up_sync, embeddings_task) {
        return runtime_lane("waiting", progress, stats.embedding_jobs, warnings)
            .with_waiting_reason("waiting_for_follow_up_sync")
            .with_activity_label("Running a follow-up sync");
    }
    if !warnings.is_empty() {
        return runtime_lane("warning", progress, stats.embedding_jobs, warnings)
            .with_activity_label("Building the semantic search index");
    }
    if progress_has_remaining(progress.as_ref()) {
        return runtime_lane("waiting", progress, stats.embedding_jobs, warnings)
            .with_waiting_reason("waiting_for_workplane")
            .with_activity_label("Building the semantic search index");
    }
    completed_lane_with_progress(progress)
}

pub(crate) fn derive_summaries_lane(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
    stats: &SessionWorkplaneStats,
    progress: Option<InitRuntimeLaneProgressView>,
) -> InitRuntimeLaneView {
    if session.selections.summaries_bootstrap.is_none() {
        return skipped_lane();
    }
    let warnings = stats.summary_warnings();
    if let Some(run) = summary_run {
        if run.status == SummaryBootstrapStatus::Running
            || run.status == SummaryBootstrapStatus::Queued
        {
            return runtime_lane(
                if run.status == SummaryBootstrapStatus::Queued {
                    "queued"
                } else {
                    "running"
                },
                progress,
                stats.summary_jobs,
                warnings,
            )
            .with_activity_label("Preparing summary generation")
            .with_run_id_option(Some(run.run_id.clone()));
        }
        if run.status == SummaryBootstrapStatus::Failed {
            return failed_lane(
                Some("Preparing summary generation failed".to_string()),
                stats.summary_jobs,
                None,
                Some(run.run_id.clone()),
                progress,
                warnings,
            );
        }
    }
    if let Some(reason) = stats.blocked_summary_reason.clone() {
        return runtime_lane("waiting", progress, stats.summary_jobs, warnings)
            .with_waiting_reason("blocked_mailbox")
            .with_activity_label("Generating summaries")
            .with_run_id_option(summary_run.map(|run| run.run_id.clone()))
            .with_detail(reason);
    }
    if stats.summary_jobs.pending > 0 || stats.summary_jobs.running > 0 {
        return runtime_lane(
            if stats.summary_jobs.running > 0 {
                "running"
            } else {
                "queued"
            },
            progress,
            stats.summary_jobs,
            warnings,
        )
        .with_activity_label("Generating summaries")
        .with_run_id_option(summary_run.map(|run| run.run_id.clone()));
    }
    if summary_bootstrap_outstanding_after_initial_sync(session, initial_sync, summary_run) {
        return runtime_lane("waiting", progress, stats.summary_jobs, warnings)
            .with_waiting_reason("waiting_for_summary_bootstrap")
            .with_activity_label("Preparing summary generation")
            .with_run_id_option(summary_run.map(|run| run.run_id.clone()));
    }
    if summaries_follow_up_pending(session, initial_sync, follow_up_sync, summary_run) {
        return runtime_lane("waiting", progress, stats.summary_jobs, warnings)
            .with_waiting_reason("waiting_for_follow_up_sync")
            .with_activity_label("Running a follow-up sync")
            .with_run_id_option(summary_run.map(|run| run.run_id.clone()));
    }
    if !warnings.is_empty() {
        return runtime_lane("warning", progress, stats.summary_jobs, warnings)
            .with_activity_label("Generating summaries")
            .with_run_id_option(summary_run.map(|run| run.run_id.clone()));
    }
    if progress_has_remaining(progress.as_ref()) {
        return runtime_lane("warning", progress, stats.summary_jobs, warnings)
            .with_activity_label("Generating summaries")
            .with_detail(
                "Summary generation finished without producing current summaries for every eligible artefact"
                    .to_string(),
            )
            .with_run_id_option(summary_run.map(|run| run.run_id.clone()));
    }
    completed_lane_with_progress(progress)
}

fn lane_from_task(
    task: &DevqlTaskRecord,
    detail: Option<String>,
    counts: StatusCounts,
    progress: Option<InitRuntimeLaneProgressView>,
    warnings: Vec<InitRuntimeLaneWarningView>,
) -> InitRuntimeLaneView {
    let activity_label = detail
        .as_deref()
        .map(lane_activity_label)
        .map(str::to_string)
        .or_else(|| Some(task_kind_label(&task.kind.to_string()).to_string()));
    let status = match task.status {
        DevqlTaskStatus::Queued => "queued",
        DevqlTaskStatus::Running => "running",
        DevqlTaskStatus::Completed => "completed",
        DevqlTaskStatus::Failed | DevqlTaskStatus::Cancelled => "failed",
    };
    let lane = runtime_lane(status, progress, counts, warnings)
        .with_activity_label_option(activity_label)
        .with_task_id_option(Some(task.task_id.clone()));
    if let Some(detail) = detail {
        lane.with_detail(lane_activity_label(&detail).to_string())
    } else {
        lane
    }
}

fn failed_lane(
    detail: Option<String>,
    counts: StatusCounts,
    task_id: Option<String>,
    run_id: Option<String>,
    progress: Option<InitRuntimeLaneProgressView>,
    warnings: Vec<InitRuntimeLaneWarningView>,
) -> InitRuntimeLaneView {
    let lane = runtime_lane("failed", progress, counts, warnings)
        .with_waiting_reason("failed")
        .with_activity_label_option(detail.clone())
        .with_task_id_option(task_id)
        .with_run_id_option(run_id);
    if let Some(detail) = detail {
        lane.with_detail(detail)
    } else {
        lane
    }
}

fn completed_lane() -> InitRuntimeLaneView {
    completed_lane_with_progress(None)
}

fn completed_lane_with_progress(
    progress: Option<InitRuntimeLaneProgressView>,
) -> InitRuntimeLaneView {
    runtime_lane("completed", progress, StatusCounts::default(), Vec::new())
}

fn skipped_lane() -> InitRuntimeLaneView {
    runtime_lane("skipped", None, StatusCounts::default(), Vec::new())
}

fn runtime_lane(
    status: &str,
    progress: Option<InitRuntimeLaneProgressView>,
    counts: StatusCounts,
    warnings: Vec<InitRuntimeLaneWarningView>,
) -> InitRuntimeLaneView {
    InitRuntimeLaneView {
        status: status.to_string(),
        waiting_reason: None,
        detail: None,
        activity_label: None,
        task_id: None,
        run_id: None,
        progress,
        queue: InitRuntimeLaneQueueView {
            queued: counts.queued(),
            running: counts.running,
            failed: counts.failed,
        },
        warnings,
        pending_count: counts.pending,
        running_count: counts.running,
        failed_count: counts.failed,
        completed_count: counts.completed,
    }
}

fn progress_has_remaining(progress: Option<&InitRuntimeLaneProgressView>) -> bool {
    progress.is_some_and(|progress| progress.remaining > 0)
}

pub(crate) fn derive_embeddings_completed_count(
    code_total: u64,
    code_completed_current: u64,
    code_queue: StatusCounts,
    summary_total: u64,
    summary_completed_current: u64,
    summaries_completed_current: u64,
    summary_queue: StatusCounts,
) -> u64 {
    let code_completed = code_total
        .saturating_sub(code_queue.pending + code_queue.running + code_queue.failed)
        .max(code_completed_current.min(code_total));
    let summary_completed = summaries_completed_current
        .saturating_sub(summary_queue.pending + summary_queue.running + summary_queue.failed)
        .max(summary_completed_current.min(summary_total))
        .min(summary_total)
        .min(summaries_completed_current);

    code_completed + summary_completed
}

pub(crate) fn active_task(task: Option<&DevqlTaskRecord>) -> Option<&DevqlTaskRecord> {
    task.filter(|task| {
        matches!(
            task.status,
            DevqlTaskStatus::Queued | DevqlTaskStatus::Running
        )
    })
}

pub(crate) fn running_task(task: Option<&DevqlTaskRecord>) -> Option<&DevqlTaskRecord> {
    task.filter(|task| task.status == DevqlTaskStatus::Running)
}
